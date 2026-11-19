//! The world's verbs (02 §23; 05 §10.7): what an effect calls to change
//! cells, deliver damage, let energy go, put fields down, throw what
//! stands in the world and make areas.
//!
//! Every verb here is a thin call of one function of the world, made
//! inside [`Sim::world`] with the cause the verb is handed, so that the
//! world's own events follow and the kernel keeps up with them. None
//! writes a cell any other way (01 R3). A refusal is a [`Reason`]: the
//! world's own (`world.not_loaded`, `world.outside_realm`, …, through
//! [`Reason::from_world`]) or one of [`WORLD_VERB_REASONS`].
//!
//! | Verb | Arguments | Returns | Calls |
//! |------|-----------|---------|-------|
//! | `set_floor`, `set_block`, `set_fixture` | `at: loc`, `to: string?` | null | `WorldEdit::set_floor` and its siblings |
//! | `set_material` | `of: element`, `material: id<material>` | null | `WorldEdit::set_material` |
//! | `set_state` | `of: element`, `state: string` | null | `WorldEdit::set_state_bits` |
//! | `damage` | `at: any`, `layer: string?`, `kind: id<damage_type>`, `energy: real`, `intensity: real`, `dir: any?` | `string` | `WorldEdit::damage`, `damage_along` |
//! | `destroy` | `of: element`, `kind: id<damage_type>` | `bool` | `WorldEdit::destroy` |
//! | `check_stability` | `at: loc` | null | `WorldCtx::check_stability` |
//! | `release` | `at: any`, `kind: id<damage_type>`, `energy: real`, `shape: string?`, `dir: any?`, `half_angle: real?`, `radius: real?`, `range: real?`, `burst: bool?`, `speed: real?`, `height: real?` | null | `WorldCtx::release` |
//! | `add_field`, `set_field` | `at: loc`, `field: id<field>`, `amount: int` or `level: int` | `int` | `WorldEdit::add_field`, `set_field` |
//! | `launch` | `what: entity`, `vel: any` | null | `WorldCtx::launch` |
//! | `impulse` | `what: entity`, `j: any` | null | `WorldCtx::impulse` |
//! | `arrest` | `what: entity` | null | `WorldCtx::arrest` |
//! | `create_area` | `at: loc`, `to: loc?`, `radius: real?`, `kind: id<area_kind>?`, `tags: list<string>?`, `clock_rate: real?`, `priority: int?`, `anchor: entity?`, `gravity: any?`, `light: real?`, `light_radius: real?`, `bound: bool?` and the terms of a binding (`ability`, `for`, `ends_on`, `on_end`, `as`; 08 §7.3) | `area` | `WorldCtx::create_area` |
//! | `remove_area` | `area: area` | null | `WorldCtx::remove_area` |
//! | `enclose` | `from: loc`, `kind: id<area_kind>?`, `limit: int?` | `area` | `WorldCtx::enclose` |
//!
//! What 02 §23 names and is not bound here, and why, is listed in that
//! section.

use clash_core::Real;
use clash_data::Def;
use clash_expr::{Registry, Value, VerbSpec};
use clash_world::dims::{CELL_M, STOREY_M};
use clash_world::field::FieldDef;
use clash_world::{
    AREA_MAX_CELLS, AmbientOverride, AreaAnchor, AreaKindId, AreaShape, AreaSpec, Body, BodyOwner,
    Cause, CellBox, DamagePacket, DamageResult, DamageTypeId, ElementState, EnergyRelease, FieldId,
    GravityOverride, Layer, LightSource, Location, MaterialId, RELEASE_DEFAULT_ALT_MM,
    ReleaseShape, StateRefError,
};

use crate::Sim;
use crate::component::Physical;
use crate::entity::Place;
use crate::event::Subject;
use crate::reason::{Reason, ReasonInfo};
use crate::rule::VerbCall;
use crate::verb::VerbDef;

/// Millimetres in a metre.
const MM_PER_M: Real = Real::of(1000.0);

/// Half of one: the middle of a cell.
const HALF: Real = Real::of(0.5);

/// How many cells `enclose` fills before it gives up, when the effect
/// gives no `limit`: the most an area may list.
pub const ENCLOSE_DEFAULT_LIMIT: u32 = AREA_MAX_CELLS;

/// The stable codes of the reasons the world's verbs refuse for, beyond
/// the world's own. Each is listed in [`WORLD_VERB_REASONS`].
pub mod codes {
    /// An argument is missing or of the wrong kind. Values: `verb`,
    /// `arg`.
    pub const BAD_ARGUMENT: &str = "world.bad_argument";
    /// A state as written names no definition, material or state.
    /// Values: `state`.
    pub const UNKNOWN_STATE: &str = "world.unknown_state";
    /// An id names no definition the world has: a damage type, a
    /// material, a field, an area kind. Values: `def`.
    pub const UNKNOWN_DEF: &str = "world.unknown_def";
    /// There is no element where one is wanted. Values: `layer`.
    pub const NO_ELEMENT: &str = "world.no_element";
    /// An entity that is to be thrown, pushed or caught is not standing
    /// in the world, or is not in flight. Values: `entity`.
    pub const NOT_IN_WORLD: &str = "world.not_in_world";
}

/// The reasons the world's verbs refuse for, for the generated
/// reference. The world's own refusals (`world.not_loaded`,
/// `world.outside_realm`, `world.refused`, …) are the kernel's.
pub const WORLD_VERB_REASONS: &[ReasonInfo] = &[
    ReasonInfo {
        code: codes::BAD_ARGUMENT,
        doc: "A world verb was given an argument that is missing or of the wrong kind. Values: `verb`, `arg`.",
    },
    ReasonInfo {
        code: codes::UNKNOWN_STATE,
        doc: "A state as written (`def`, `def@material`, `def#state`) names no definition of the layer, no material or no state of the definition. Values: `state`.",
    },
    ReasonInfo {
        code: codes::UNKNOWN_DEF,
        doc: "An id names no damage type, material, field or area kind the world has. Values: `def`.",
    },
    ReasonInfo {
        code: codes::NO_ELEMENT,
        doc: "The cell has no element on the layer the verb acts on. Values: `layer`.",
    },
    ReasonInfo {
        code: codes::NOT_IN_WORLD,
        doc: "An entity to be thrown is not standing in the world, or one to be pushed or caught is not in flight. Values: `entity`.",
    },
];

// ---- Arguments ---------------------------------------------------------------

/// The refusal for an argument that is missing or of the wrong kind.
pub(crate) fn bad_argument(verb: &str, name: &str, wanted: &str) -> Reason {
    Reason::new(
        codes::BAD_ARGUMENT,
        format!("`{verb}`: the argument `{name}` must be {wanted}"),
    )
    .with("verb", verb)
    .with("arg", name)
}

/// The value of an argument, by its position in the specification.
pub(crate) fn arg<'a>(call: &'a VerbCall<'_>, index: usize) -> &'a Value {
    call.args.get(index).unwrap_or(&Value::Null)
}

/// What an argument names, if it is one of the simulation's subjects.
pub(crate) fn subject_arg(sim: &Sim, call: &VerbCall<'_>, index: usize) -> Option<Subject> {
    sim.subject_of(arg(call, index))
}

/// An argument that is a number, or `None` for null.
fn number(
    verb: &str,
    call: &VerbCall<'_>,
    index: usize,
    name: &str,
) -> Result<Option<Real>, Reason> {
    match arg(call, index) {
        Value::Null => Ok(None),
        value => value
            .as_number()
            .filter(|number| number.is_finite())
            .map(Some)
            .ok_or_else(|| bad_argument(verb, name, "a finite number")),
    }
}

/// An argument that must be a number.
fn required_number(
    verb: &str,
    call: &VerbCall<'_>,
    index: usize,
    name: &str,
) -> Result<Real, Reason> {
    number(verb, call, index, name)?.ok_or_else(|| bad_argument(verb, name, "a finite number"))
}

/// An argument that is a cell: a `loc`, or the cell of an element.
pub(crate) fn cell_arg(
    sim: &Sim,
    verb: &str,
    call: &VerbCall<'_>,
    index: usize,
    name: &str,
) -> Result<Location, Reason> {
    match subject_arg(sim, call, index) {
        Some(Subject::Loc(loc) | Subject::Element(loc, _)) => Ok(loc),
        _ => Err(bad_argument(verb, name, "a cell")),
    }
}

/// An argument that is an element.
pub(crate) fn element_arg(
    sim: &Sim,
    verb: &str,
    call: &VerbCall<'_>,
    index: usize,
    name: &str,
) -> Result<(Location, Layer), Reason> {
    match subject_arg(sim, call, index) {
        Some(Subject::Element(loc, layer)) => Ok((loc, layer)),
        _ => Err(bad_argument(
            verb,
            name,
            "an element: a cell's floor, block or fixture",
        )),
    }
}

/// An argument that is an entity.
pub(crate) fn entity_arg(
    sim: &Sim,
    verb: &str,
    call: &VerbCall<'_>,
    index: usize,
    name: &str,
) -> Result<clash_core::EntityId, Reason> {
    match subject_arg(sim, call, index) {
        Some(Subject::Entity(id)) => Ok(id),
        _ => Err(bad_argument(verb, name, "an entity")),
    }
}

/// Where something is, as a cell and a height above its base in
/// millimetres: a cell at its middle, an element's cell, an entity where
/// it (or what holds it) stands.
pub(crate) fn position_of(sim: &Sim, subject: Subject) -> Option<(Location, u16)> {
    match subject {
        Subject::Loc(loc) | Subject::Element(loc, _) => Some((loc, RELEASE_DEFAULT_ALT_MM)),
        Subject::Entity(id) => {
            let top = sim.entities.outermost(id)?;
            match sim.entities.get(top)?.place() {
                Place::At { loc, alt_mm } => Some((loc, alt_mm)),
                Place::In(_) | Place::Stored => None,
            }
        }
        _ => None,
    }
}

/// A point in metres in its grid: the middle of a cell, at a height
/// above its base.
pub(crate) fn point_of(loc: Location, alt_mm: u16) -> [Real; 3] {
    let middle = |at: i32| (Real::from_i64(i64::from(at)) + HALF) * CELL_M;
    [
        middle(loc.cell.x),
        middle(loc.cell.y),
        Real::from_i64(i64::from(loc.cell.z)) * STOREY_M
            + Real::from_i64(i64::from(alt_mm)) / MM_PER_M,
    ]
}

/// A vector east, south and up: a list of three numbers as written, or
/// the way from `from` to where a subject is.
fn vector(sim: &Sim, value: &Value, from: Option<[Real; 3]>) -> Option<[Real; 3]> {
    if let Some(items) = value.as_list() {
        let parts: Vec<Real> = items.iter().filter_map(Value::as_number).collect();
        return match parts.as_slice() {
            [x, y, z] if parts.iter().all(|part| part.is_finite()) => Some([*x, *y, *z]),
            _ => None,
        };
    }
    let (loc, alt) = position_of(sim, sim.subject_of(value)?)?;
    let to = point_of(loc, alt);
    let from = from?;
    Some([to[0] - from[0], to[1] - from[1], to[2] - from[2]])
}

/// The world's damage type an argument names: an id, or its qualified
/// id as a text.
pub(crate) fn damage_type_arg(
    sim: &Sim,
    verb: &str,
    call: &VerbCall<'_>,
    index: usize,
    name: &str,
) -> Result<DamageTypeId, Reason> {
    let written = def_name_of(sim, arg(call, index))
        .ok_or_else(|| bad_argument(verb, name, "a damage type"))?;
    let defs = sim.cosmos.defs();
    defs.damage_types()
        .resolve(&written, None)
        .ok_or_else(|| unknown_def(&written))
}

/// The qualified id of what a value names: an id, or a text as written.
pub(crate) fn def_name_of(sim: &Sim, value: &Value) -> Option<String> {
    match value {
        Value::Id(id) => sim.exprs.registry().def_name(*id).map(str::to_owned),
        Value::Str(text) => Some(text.to_string()),
        _ => None,
    }
}

fn unknown_def(name: &str) -> Reason {
    Reason::new(
        codes::UNKNOWN_DEF,
        format!("the world has no definition `{name}`"),
    )
    .with("def", name)
}

fn unknown_state(text: &str, error: &StateRefError) -> Reason {
    Reason::new(codes::UNKNOWN_STATE, error.to_string()).with("state", text)
}

/// The refusal of the world, as a reason.
pub(crate) fn world(error: &clash_world::WorldError) -> Reason {
    Reason::from_world(error)
}

/// The element of a cell on a layer, or the reason it cannot be read.
pub(crate) fn element_at(sim: &Sim, loc: Location, layer: Layer) -> Result<ElementState, Reason> {
    sim.cosmos
        .grid(loc.grid)
        .and_then(|grid| grid.element(loc.cell, layer))
        .map_err(|error| world(&error))
}

fn no_element(layer: Layer) -> Reason {
    Reason::new(
        codes::NO_ELEMENT,
        format!("there is no {} there", layer.name()),
    )
    .with("layer", layer.name())
}

/// A layer as data writes it.
pub(crate) fn layer_named(name: &str) -> Option<Layer> {
    Layer::ALL.into_iter().find(|layer| layer.name() == name)
}

// ---- Specifications ------------------------------------------------------------

/// One argument as a verb's specification writes it: name, type, whether
/// it may be left out, what it means.
pub(crate) type ArgRow = (&'static str, &'static str, bool, &'static str);

/// The specification of a verb, if the kinds and types it names are
/// registered.
pub(crate) fn spec(
    registry: &Registry,
    name: &str,
    doc: &str,
    args: &[ArgRow],
) -> Option<VerbSpec> {
    let mut spec = VerbSpec::new(name).doc(doc);
    for (arg, written, optional, arg_doc) in args {
        let ty = registry.parse_type(written)?;
        spec = if *optional {
            spec.optional_arg(*arg, ty.nullable(), *arg_doc)
        } else {
            spec.arg(*arg, ty, *arg_doc)
        };
    }
    Some(spec)
}

/// Enters the definition types the world's verbs name (`field`) into an
/// expression registry, those it lacks. Called when the engine's
/// registry is built, before its verbs.
pub fn register_types(registry: &mut Registry) {
    if registry.def_type_id(FieldDef::TYPE).is_none() {
        let _ = registry.add_def_type(FieldDef::TYPE, FieldDef::DOC);
    }
}

// ---- The verbs -------------------------------------------------------------------

/// Sets one layer of a cell to a state written as data writes it, or
/// clears it.
fn set_layer(
    sim: &mut Sim,
    verb: &str,
    layer: Layer,
    call: &VerbCall<'_>,
    cause: Cause,
) -> Result<Value, Reason> {
    let at = cell_arg(sim, verb, call, 0, "at")?;
    let state = match arg(call, 1) {
        Value::Null => ElementState::NONE,
        Value::Str(text) => {
            let defs = sim.cosmos.defs();
            let found = match layer {
                Layer::Floor => defs.floor_state(text, None).map(|state| state.raw()),
                Layer::Block => defs.block_state(text, None).map(|state| state.raw()),
                Layer::Fixture => defs.fixture_state(text, None).map(|state| state.raw()),
            };
            found.map_err(|error| unknown_state(text, &error))?
        }
        _ => {
            return Err(bad_argument(
                verb,
                "to",
                "a state as data writes it, or null",
            ));
        }
    };
    sim.world(|ctx| ctx.edit(cause).set_element(at, layer, state))
        .map_err(|error| world(&error))?;
    Ok(Value::Null)
}

const SET_LAYER_ARGS: [ArgRow; 2] = [
    ("at", "loc", false, "The cell."),
    (
        "to",
        "string",
        true,
        "What to put there, as data writes a state: `def`, `def@material`, `def#state` or `def@material#state`. Null takes what is there away.",
    ),
];

fn set_state(sim: &mut Sim, call: &VerbCall<'_>, cause: Cause) -> Result<Value, Reason> {
    let (loc, layer) = element_arg(sim, "set_state", call, 0, "of")?;
    let state = element_at(sim, loc, layer)?;
    if state.is_none() {
        return Err(no_element(layer));
    }
    let written = arg(call, 1)
        .as_str()
        .ok_or_else(|| bad_argument("set_state", "state", "the name of a state"))?;
    let defs = sim.cosmos.defs();
    let bits = defs
        .element(layer, state)
        .and_then(|element| element.state_index(written))
        .or_else(|| written.parse::<u16>().ok());
    let Some(bits) = bits else {
        let def = defs.state_text(layer, state);
        return Err(Reason::new(
            codes::UNKNOWN_STATE,
            format!("{def} has no state `{written}`"),
        )
        .with("state", written));
    };
    sim.world(|ctx| ctx.edit(cause).set_state_bits(loc, layer, bits))
        .map_err(|error| world(&error))?;
    Ok(Value::Null)
}

fn set_material(sim: &mut Sim, call: &VerbCall<'_>, cause: Cause) -> Result<Value, Reason> {
    let (loc, layer) = element_arg(sim, "set_material", call, 0, "of")?;
    let written = def_name_of(sim, arg(call, 1))
        .ok_or_else(|| bad_argument("set_material", "material", "a material"))?;
    let material: MaterialId = sim
        .cosmos
        .defs()
        .materials()
        .resolve(&written, None)
        .ok_or_else(|| unknown_def(&written))?;
    if element_at(sim, loc, layer)?.is_none() {
        return Err(no_element(layer));
    }
    sim.world(|ctx| ctx.edit(cause).set_material(loc, layer, material))
        .map_err(|error| world(&error))?;
    Ok(Value::Null)
}

fn damage(sim: &mut Sim, call: &VerbCall<'_>, cause: Cause) -> Result<Value, Reason> {
    const VERB: &str = "damage";
    let (loc, own_layer) = match subject_arg(sim, call, 0) {
        Some(Subject::Element(loc, layer)) => (loc, Some(layer)),
        Some(Subject::Loc(loc)) => (loc, None),
        _ => return Err(bad_argument(VERB, "at", "a cell or an element")),
    };
    let layer = match (own_layer, arg(call, 1)) {
        (Some(layer), _) => layer,
        (None, Value::Null) => Layer::Block,
        (None, Value::Str(name)) => layer_named(name)
            .ok_or_else(|| bad_argument(VERB, "layer", "`floor`, `block` or `fixture`"))?,
        _ => return Err(bad_argument(VERB, "layer", "`floor`, `block` or `fixture`")),
    };
    let kind = damage_type_arg(sim, VERB, call, 2, "kind")?;
    let energy = required_number(VERB, call, 3, "energy")?;
    let intensity = required_number(VERB, call, 4, "intensity")?;
    if energy < Real::ZERO || intensity < Real::ZERO {
        return Err(
            bad_argument(VERB, "energy", "a number of zero or more").with("intensity", intensity)
        );
    }
    let dir = match arg(call, 5) {
        Value::Null => None,
        value => Some(
            vector(sim, value, Some(point_of(loc, RELEASE_DEFAULT_ALT_MM))).ok_or_else(|| {
                bad_argument(VERB, "dir", "a list of three numbers, or something to face")
            })?,
        ),
    };
    let packet = DamagePacket::new(kind, energy, intensity, cause);
    let outcome = sim
        .world(|ctx| ctx.edit(cause).damage_along(loc, layer, packet, dir))
        .map_err(|error| world(&error))?;
    Ok(Value::from(match outcome.result {
        DamageResult::Unaffected => "unaffected",
        DamageResult::Damaged => "damaged",
        DamageResult::Destroyed => "destroyed",
    }))
}

fn destroy(sim: &mut Sim, call: &VerbCall<'_>, cause: Cause) -> Result<Value, Reason> {
    let (loc, layer) = element_arg(sim, "destroy", call, 0, "of")?;
    let kind = damage_type_arg(sim, "destroy", call, 1, "kind")?;
    let destroyed = sim
        .world(|ctx| ctx.edit(cause).destroy(loc, layer, kind))
        .map_err(|error| world(&error))?;
    Ok(Value::Bool(destroyed))
}

fn release(sim: &mut Sim, call: &VerbCall<'_>, cause: Cause) -> Result<Value, Reason> {
    const VERB: &str = "release";
    let subject = subject_arg(sim, call, 0)
        .ok_or_else(|| bad_argument(VERB, "at", "a cell, an element or an entity"))?;
    let (origin, mut alt_mm) = position_of(sim, subject)
        .ok_or_else(|| bad_argument(VERB, "at", "a cell, an element or an entity in the world"))?;
    let kind = damage_type_arg(sim, VERB, call, 1, "kind")?;
    let energy = required_number(VERB, call, 2, "energy")?;
    if let Some(height) = number(VERB, call, 9, "height")? {
        let mm = (height * MM_PER_M)
            .round()
            .to_i64()
            .clamp(0, i64::from(u16::MAX));
        alt_mm = u16::try_from(mm).unwrap_or(u16::MAX);
    }
    let from = point_of(origin, alt_mm);
    let dir = || {
        vector(sim, arg(call, 4), Some(from)).ok_or_else(|| {
            bad_argument(
                VERB,
                "dir",
                "a list of three numbers, or something to aim at",
            )
        })
    };
    let shape = match arg(call, 3) {
        Value::Null => ReleaseShape::Sphere,
        Value::Str(name) if &**name == "sphere" => ReleaseShape::Sphere,
        Value::Str(name) if &**name == "cone" => ReleaseShape::Cone {
            dir: dir()?,
            half_angle: required_number(VERB, call, 5, "half_angle")?,
        },
        Value::Str(name) if &**name == "beam" => ReleaseShape::Beam {
            dir: dir()?,
            radius: required_number(VERB, call, 6, "radius")?,
            range: required_number(VERB, call, 7, "range")?,
            burst: arg(call, 8).as_bool().unwrap_or(false),
        },
        _ => return Err(bad_argument(VERB, "shape", "`sphere`, `cone` or `beam`")),
    };
    let spec = EnergyRelease {
        origin,
        origin_alt_mm: alt_mm,
        energy,
        kind,
        shape,
        speed: number(VERB, call, 10, "speed")?,
        cause,
    };
    sim.world(|ctx| ctx.release(spec))
        .map_err(|error| world(&error))?;
    Ok(Value::Null)
}

/// The world's field an argument names.
fn field_arg(sim: &Sim, verb: &str, call: &VerbCall<'_>, index: usize) -> Result<FieldId, Reason> {
    let written =
        def_name_of(sim, arg(call, index)).ok_or_else(|| bad_argument(verb, "field", "a field"))?;
    sim.cosmos
        .defs()
        .fields()
        .resolve(&written, None)
        .ok_or_else(|| unknown_def(&written))
}

fn add_field(sim: &mut Sim, call: &VerbCall<'_>, cause: Cause) -> Result<Value, Reason> {
    let at = cell_arg(sim, "add_field", call, 0, "at")?;
    let field = field_arg(sim, "add_field", call, 1)?;
    let amount = arg(call, 2)
        .as_int()
        .map(|amount| amount.clamp(i64::from(i16::MIN), i64::from(i16::MAX)))
        .and_then(|amount| i16::try_from(amount).ok())
        .ok_or_else(|| bad_argument("add_field", "amount", "a whole number"))?;
    let old = sim
        .world(|ctx| ctx.edit(cause).add_field(at, field, amount))
        .map_err(|error| world(&error))?;
    Ok(Value::Int(i64::from(old)))
}

fn set_field(sim: &mut Sim, call: &VerbCall<'_>, cause: Cause) -> Result<Value, Reason> {
    let at = cell_arg(sim, "set_field", call, 0, "at")?;
    let field = field_arg(sim, "set_field", call, 1)?;
    let level = arg(call, 2)
        .as_int()
        .and_then(|level| u8::try_from(level).ok())
        .ok_or_else(|| bad_argument("set_field", "level", "a whole number from 0 to 255"))?;
    let old = sim
        .world(|ctx| ctx.edit(cause).set_field(at, field, level))
        .map_err(|error| world(&error))?;
    Ok(Value::Int(i64::from(old)))
}

fn not_in_world(entity: clash_core::EntityId, what: &str) -> Reason {
    Reason::new(codes::NOT_IN_WORLD, format!("{entity} {what}")).with("entity", entity.to_string())
}

fn launch(sim: &mut Sim, call: &VerbCall<'_>, cause: Cause) -> Result<Value, Reason> {
    const VERB: &str = "launch";
    let entity = entity_arg(sim, VERB, call, 0, "what")?;
    let presence = *sim.known(entity)?.presence();
    let Some(Place::At { loc, alt_mm }) = sim.entities.get(entity).map(|known| known.place())
    else {
        return Err(not_in_world(entity, "is not standing in the world"));
    };
    let pos = point_of(loc, alt_mm);
    let vel = vector(sim, arg(call, 1), Some(pos))
        .ok_or_else(|| bad_argument(VERB, "vel", "a list of three numbers in m/s"))?;
    let physical = sim.components.get::<Physical>(entity);
    let mass = physical.map_or(presence.mass, |physical| physical.mass().get());
    let mut body = Body::of_mass(BodyOwner::Entity(entity), loc.grid, pos, vel, mass, cause);
    if let Some(physical) = physical {
        if physical.cross_section() > Real::ZERO {
            body.area = physical.cross_section();
        }
        if physical.volume().get() > Real::ZERO {
            body.volume = physical.volume().get();
        }
    }
    body.plane = presence.plane;
    sim.world(|ctx| ctx.launch(body))
        .map_err(|error| world(&error))?;
    Ok(Value::Null)
}

/// The body in flight that an entity is.
fn body_of(sim: &Sim, entity: clash_core::EntityId) -> Result<clash_world::BodyId, Reason> {
    sim.cosmos
        .body_of(entity)
        .ok_or_else(|| not_in_world(entity, "is not in flight"))
}

fn impulse(sim: &mut Sim, call: &VerbCall<'_>, _cause: Cause) -> Result<Value, Reason> {
    let entity = entity_arg(sim, "impulse", call, 0, "what")?;
    let body = body_of(sim, entity)?;
    let j = vector(sim, arg(call, 1), None)
        .ok_or_else(|| bad_argument("impulse", "j", "a list of three numbers in kg·m/s"))?;
    sim.world(|ctx| ctx.impulse(body, j))
        .map_err(|error| world(&error))?;
    Ok(Value::Null)
}

fn arrest(sim: &mut Sim, call: &VerbCall<'_>, _cause: Cause) -> Result<Value, Reason> {
    let entity = entity_arg(sim, "arrest", call, 0, "what")?;
    let body = body_of(sim, entity)?;
    sim.world(|ctx| ctx.arrest(body))
        .map_err(|error| world(&error))?;
    Ok(Value::Null)
}

/// An optional area kind.
fn area_kind_arg(
    sim: &Sim,
    verb: &str,
    call: &VerbCall<'_>,
    index: usize,
) -> Result<Option<AreaKindId>, Reason> {
    match arg(call, index) {
        Value::Null => Ok(None),
        value => {
            let written = def_name_of(sim, value)
                .ok_or_else(|| bad_argument(verb, "kind", "an area kind"))?;
            sim.cosmos
                .defs()
                .area_kinds()
                .resolve(&written, None)
                .map(Some)
                .ok_or_else(|| unknown_def(&written))
        }
    }
}

fn area_value(sim: &Sim, id: clash_world::AreaId) -> Value {
    sim.subject_value(Subject::Area(id))
}

fn create_area(sim: &mut Sim, call: &VerbCall<'_>, cause: Cause) -> Result<Value, Reason> {
    const VERB: &str = "create_area";
    // Left out, `at` is where the anchor is.
    let at = match (arg(call, 0), subject_arg(sim, call, 7)) {
        (Value::Null, Some(Subject::Entity(anchor))) => sim
            .entities
            .location(anchor)
            .ok_or_else(|| bad_argument(VERB, "at", "a cell, or an anchor that is in the world"))?,
        _ => cell_arg(sim, VERB, call, 0, "at")?,
    };
    let to = match arg(call, 1) {
        Value::Null => None,
        _ => Some(cell_arg(sim, VERB, call, 1, "to")?),
    };
    let radius = number(VERB, call, 2, "radius")?;
    let shape = match (to, radius) {
        (Some(_), Some(_)) => {
            return Err(bad_argument(VERB, "radius", "left out when `to` is given"));
        }
        (Some(to), None) => {
            if to.grid != at.grid {
                return Err(bad_argument(VERB, "to", "a cell of the same grid as `at`"));
            }
            AreaShape::Box(CellBox::new(at.cell, to.cell))
        }
        (None, Some(radius)) => AreaShape::Radius {
            centre: at.cell,
            radius,
        },
        (None, None) => AreaShape::Box(CellBox::new(at.cell, at.cell)),
    };
    let mut spec = AreaSpec::new(at.grid, shape);
    spec.kind = area_kind_arg(sim, VERB, call, 3)?;
    if let Some(tags) = arg(call, 4).as_list() {
        spec.tags = tags
            .iter()
            .filter_map(|tag| tag.as_str().map(str::to_owned))
            .collect();
        spec.tags.sort();
        spec.tags.dedup();
    }
    spec.clock_rate = number(VERB, call, 5, "clock_rate")?;
    spec.priority = match arg(call, 6) {
        Value::Null => None,
        value => Some(
            value
                .as_int()
                .and_then(|priority| i32::try_from(priority).ok())
                .ok_or_else(|| bad_argument(VERB, "priority", "a whole number"))?,
        ),
    };
    // What moves it, what it does to gravity and light (02 §8.3).
    spec.anchor = match arg(call, 7) {
        Value::Null => None,
        _ => match subject_arg(sim, call, 7) {
            Some(Subject::Entity(entity)) if sim.entities.contains(entity) => {
                Some(AreaAnchor::Entity(entity))
            }
            _ => return Err(bad_argument(VERB, "anchor", "an entity that exists")),
        },
    };
    let gravity = match arg(call, 8) {
        Value::Null => None,
        value => match value.as_number() {
            Some(scale) if scale.is_finite() => Some(GravityOverride::Scale(scale)),
            Some(_) => {
                return Err(bad_argument(VERB, "gravity", "a finite scale"));
            }
            None => Some(GravityOverride::Vector(
                vector(sim, value, None).ok_or_else(|| {
                    bad_argument(
                        VERB,
                        "gravity",
                        "a scale, or a list of three accelerations in m/s²",
                    )
                })?,
            )),
        },
    };
    if let Some(gravity) = gravity {
        spec.ambient = Some(AmbientOverride {
            gravity: Some(gravity),
            drift: None,
        });
    }
    if let Some(intensity) = number(VERB, call, 9, "light")? {
        let radius = number(VERB, call, 10, "light_radius")?;
        spec.light = LightSource::shining(intensity, radius)
            .map_err(|why| bad_argument(VERB, "light_radius", &why))?;
    }
    // Bound to an ability (08 §7.3): checked before anything is made.
    let terms = crate::ability::bind::area_terms(sim, call, 11)?;
    if let Some(terms) = &terms {
        crate::ability::bind::check_room(sim, terms.number)?;
    }
    let id = sim
        .world(|ctx| ctx.create_area(spec, cause))
        .map_err(|error| world(&error))?;
    if let Some(terms) = &terms {
        crate::ability::bind::bind_area(sim, terms, id, cause);
    }
    Ok(area_value(sim, id))
}

fn remove_area(sim: &mut Sim, call: &VerbCall<'_>, cause: Cause) -> Result<Value, Reason> {
    let Some(Subject::Area(id)) = subject_arg(sim, call, 0) else {
        return Err(bad_argument("remove_area", "area", "an area"));
    };
    sim.world(|ctx| ctx.remove_area(id, cause))
        .map_err(|error| world(&error))?;
    Ok(Value::Null)
}

fn enclose(sim: &mut Sim, call: &VerbCall<'_>, cause: Cause) -> Result<Value, Reason> {
    const VERB: &str = "enclose";
    let from = cell_arg(sim, VERB, call, 0, "from")?;
    let kind = area_kind_arg(sim, VERB, call, 1)?;
    let limit = match arg(call, 2) {
        Value::Null => ENCLOSE_DEFAULT_LIMIT,
        value => value
            .as_int()
            .and_then(|limit| u32::try_from(limit).ok())
            .ok_or_else(|| bad_argument(VERB, "limit", "a whole number of cells"))?,
    };
    let id = sim
        .world(|ctx| ctx.enclose(from, kind, limit, cause))
        .map_err(|error| world(&error))?;
    Ok(area_value(sim, id))
}

fn check_stability(sim: &mut Sim, call: &VerbCall<'_>, cause: Cause) -> Result<Value, Reason> {
    let at = cell_arg(sim, "check_stability", call, 0, "at")?;
    sim.world(|ctx| ctx.check_stability(at, cause))
        .map_err(|error| world(&error))?;
    Ok(Value::Null)
}

const DAMAGE_KIND: ArgRow = ("kind", "id<damage_type>", false, "The kind of damage.");

/// The world's verbs, for [`crate::verb::engine_verbs`].
pub fn world_verbs() -> Vec<VerbDef> {
    vec![
        VerbDef {
            name: "set_floor",
            build: |registry| {
                spec(
                    registry,
                    "set_floor",
                    "Puts a floor in a cell, or takes it away (02 §7.2). A floor put there starts undamaged. Raises `element_changed`, and asks for a stability check if a floor was removed or weakened. Refused for a cell that is not loaded or outside the realm, and for a state that names nothing (`world.unknown_state`).",
                    &SET_LAYER_ARGS,
                )
            },
            run: |sim, call, cause| set_layer(sim, "set_floor", Layer::Floor, call, cause),
        },
        VerbDef {
            name: "set_block",
            build: |registry| {
                spec(
                    registry,
                    "set_block",
                    "Puts a block in a cell, or takes it away (02 §7.2). A block put there starts undamaged; whoever it leaves no room is moved aside (`occupant_displaced`) or walled in (`occupant_entombed`). Raises `element_changed`, and asks for a stability check if a block was removed or weakened. Refused as `set_floor` is.",
                    &SET_LAYER_ARGS,
                )
            },
            run: |sim, call, cause| set_layer(sim, "set_block", Layer::Block, call, cause),
        },
        VerbDef {
            name: "set_fixture",
            build: |registry| {
                spec(
                    registry,
                    "set_fixture",
                    "Puts a fixture in a cell, or takes it away (02 §7.2). As `set_block`, with no stability check.",
                    &SET_LAYER_ARGS,
                )
            },
            run: |sim, call, cause| set_layer(sim, "set_fixture", Layer::Fixture, call, cause),
        },
        VerbDef {
            name: "set_material",
            build: |registry| {
                spec(
                    registry,
                    "set_material",
                    "Makes an element of another material (02 §7.2): it keeps its definition, its state and its damage. Raises `element_changed`; a block or floor asks for a stability check. Refused where there is no element (`world.no_element`), for a material that does not exist, and for a cell not loaded.",
                    &[
                        ("of", "element", false, "The element."),
                        (
                            "material",
                            "id<material>",
                            false,
                            "What it is to be made of.",
                        ),
                    ],
                )
            },
            run: set_material,
        },
        VerbDef {
            name: "set_state",
            build: |registry| {
                spec(
                    registry,
                    "set_state",
                    "Puts an element into another of its states (16 §3.5): a door open or shut, a lamp on or off. It keeps its definition, its material and its damage. Raises `element_changed`; whoever a state that blocks leaves no room is moved aside. Refused where there is no element, for a state its definition does not declare (`world.unknown_state`), and for a cell not loaded.",
                    &[
                        ("of", "element", false, "The element."),
                        (
                            "state",
                            "string",
                            false,
                            "The name of one of its definition's states, or its bits as a number written as text.",
                        ),
                    ],
                )
            },
            run: set_state,
        },
        VerbDef {
            name: "damage",
            build: |registry| {
                spec(
                    registry,
                    "damage",
                    "Delivers a packet of damage to one element of a cell, by the rule of 02 §12.2, with the cause it is handed: below the threshold nothing happens; otherwise the element is damaged (`element_damaged`) or destroyed (`element_destroyed`, rubble, and fragments along `dir` if one is given). Returns `unaffected`, `damaged` or `destroyed`. Refused for a cell not loaded or outside the realm, and for a damage type that does not exist.",
                    &[
                        (
                            "at",
                            "any",
                            false,
                            "The cell, or the element: an element names its own layer.",
                        ),
                        (
                            "layer",
                            "string",
                            true,
                            "For a cell: `floor`, `block` or `fixture`. Default `block`.",
                        ),
                        DAMAGE_KIND,
                        ("energy", "real", false, "Joules delivered."),
                        (
                            "intensity",
                            "real",
                            false,
                            "How concentrated, in pascals (J/m³).",
                        ),
                        (
                            "dir",
                            "any",
                            true,
                            "The direction of the blow, east, south and up, as a list of three numbers, or something to strike toward. With one, what is destroyed throws fragments.",
                        ),
                    ],
                )
            },
            run: damage,
        },
        VerbDef {
            name: "destroy",
            build: |registry| {
                spec(
                    registry,
                    "destroy",
                    "Destroys an element outright, as though damage of a kind had been more than it could take (02 §12.3): it becomes its `on_destroy` state or is removed, its matter is left as rubble, and `element_destroyed` is raised with nothing absorbed. Returns whether there was an element. Refused for a damage type that does not exist and a cell not loaded.",
                    &[
                        ("of", "element", false, "The element."),
                        (
                            "kind",
                            "id<damage_type>",
                            false,
                            "The kind of damage it is destroyed by, as `element_destroyed` reports it.",
                        ),
                    ],
                )
            },
            run: destroy,
        },
        VerbDef {
            name: "check_stability",
            build: |registry| {
                spec(
                    registry,
                    "check_stability",
                    "Asks for a stability check of a cell and what it holds up (02 §14.2), after the settle delay. What no longer stands falls (`collapse_started`). Refused for a cell not loaded or outside the realm.",
                    &[("at", "loc", false, "The cell.")],
                )
            },
            run: check_stability,
        },
        VerbDef {
            name: "release",
            build: |registry| {
                spec(
                    registry,
                    "release",
                    "Lets energy go at a place (02 §13): an explosion, a shockwave, a cone of fire, a beam. Raises `release_started` at once and `energy_released` when the front has gone as far as it goes; what it does on the way carries the cause it is handed. Refused for energy not above zero, a damage type that does not exist, a place not loaded, a cone or beam with no direction, a cone's angle not above zero and at most π, a beam with no radius or range.",
                    &[
                        (
                            "at",
                            "any",
                            false,
                            "Where: a cell, an element's cell, or an entity where it stands.",
                        ),
                        DAMAGE_KIND,
                        ("energy", "real", false, "Joules let go."),
                        (
                            "shape",
                            "string",
                            true,
                            "`sphere` (the default), `cone` or `beam`.",
                        ),
                        (
                            "dir",
                            "any",
                            true,
                            "For a cone or a beam: its axis, east, south and up, as a list of three numbers, or something to aim at.",
                        ),
                        (
                            "half_angle",
                            "real",
                            true,
                            "For a cone: the angle from its axis to its edge, in radians.",
                        ),
                        (
                            "radius",
                            "real",
                            true,
                            "For a beam: how far from its axis it reaches, in metres.",
                        ),
                        (
                            "range",
                            "real",
                            true,
                            "For a beam: how far it goes, in metres.",
                        ),
                        (
                            "burst",
                            "bool",
                            true,
                            "For a beam: whether what is left where it ends is let go there as a sphere. Default false.",
                        ),
                        (
                            "speed",
                            "real",
                            true,
                            "The speed of the front in m/s. Default the speed of sound where it starts.",
                        ),
                        (
                            "height",
                            "real",
                            true,
                            "Metres above the base of the cell. Default the middle of the storey, or where an entity stands.",
                        ),
                    ],
                )
            },
            run: release,
        },
        VerbDef {
            name: "add_field",
            build: |registry| {
                spec(
                    registry,
                    "add_field",
                    "Adds to the level of a field in a cell, or takes from it (02 §16), between 0 and 255. Raises `field_appeared` or `field_vanished` when the field comes or goes. Returns the level it had. Refused for a cell not loaded and a field that does not exist.",
                    &[
                        ("at", "loc", false, "The cell."),
                        ("field", "id<field>", false, "The field."),
                        (
                            "amount",
                            "int",
                            false,
                            "How much to add; less than zero takes away.",
                        ),
                    ],
                )
            },
            run: add_field,
        },
        VerbDef {
            name: "set_field",
            build: |registry| {
                spec(
                    registry,
                    "set_field",
                    "Sets the level of a field in a cell outright (02 §16); zero takes it away. Returns the level it had. Refused as `add_field` is.",
                    &[
                        ("at", "loc", false, "The cell."),
                        ("field", "id<field>", false, "The field."),
                        ("level", "int", false, "The level, 0 to 255."),
                    ],
                )
            },
            run: set_field,
        },
        VerbDef {
            name: "launch",
            build: |registry| {
                spec(
                    registry,
                    "launch",
                    "Throws an entity that stands in the world (02 §15): it becomes a body in flight with its own mass, volume, cross-section and planes, from the middle of its cell at the height it stands, and the world carries it until it lands. Raises `body_launched`. Refused for an entity not standing in the world (`world.not_in_world`), one already in flight, and one with no mass.",
                    &[
                        ("what", "entity", false, "Who or what is thrown."),
                        (
                            "vel",
                            "any",
                            false,
                            "Its velocity in m/s, east, south and up, as a list of three numbers.",
                        ),
                    ],
                )
            },
            run: launch,
        },
        VerbDef {
            name: "impulse",
            build: |registry| {
                spec(
                    registry,
                    "impulse",
                    "Pushes an entity that is in flight (02 §15): adds an impulse to its body. Refused for an entity not in flight (`world.not_in_world`) or beyond loaded space.",
                    &[
                        ("what", "entity", false, "Who or what is pushed."),
                        (
                            "j",
                            "any",
                            false,
                            "The impulse in kg·m/s, east, south and up, as a list of three numbers.",
                        ),
                    ],
                )
            },
            run: impulse,
        },
        VerbDef {
            name: "arrest",
            build: |registry| {
                spec(
                    registry,
                    "arrest",
                    "Catches an entity in flight (02 §15): its body is taken out where it is now and the entity stands there. Raises nothing. Refused for an entity not in flight.",
                    &[("what", "entity", false, "Who or what is caught.")],
                )
            },
            run: arrest,
        },
        VerbDef {
            name: "create_area",
            build: |registry| {
                spec(
                    registry,
                    "create_area",
                    "Makes an area (02 §8.3): a box from `at` to `to`, every cell within `radius` metres of `at`, or the one cell `at`; anchored to an entity, it moves with it; it may set gravity inside it and carry a light. Whoever it comes over is told (`area_entered`). Returns the area. Refused for a box in two grids, a kind that does not exist, an anchor that does not exist, gravity that is neither a scale nor three numbers, a light whose radius is out of range, and a shape larger than the engine allows.",
                    &[
                        (
                            "at",
                            "loc",
                            true,
                            "A corner of the box, or the middle. Left out, the cell its `anchor` is in.",
                        ),
                        ("to", "loc", true, "The other corner of a box."),
                        (
                            "radius",
                            "real",
                            true,
                            "For a round area: the distance in metres, a storey counting three.",
                        ),
                        (
                            "kind",
                            "id<area_kind>",
                            true,
                            "Its kind, which gives default tags, variables, clock rate, priority and rules.",
                        ),
                        ("tags", "list<string>", true, "Tags, added to its kind's."),
                        (
                            "clock_rate",
                            "real",
                            true,
                            "The rate of a clock of its own, relative to its realm's.",
                        ),
                        (
                            "priority",
                            "int",
                            true,
                            "Which area applies where several overlap.",
                        ),
                        (
                            "anchor",
                            "entity",
                            true,
                            "An entity it moves with (02 §8.3): when the entity moves, the area moves by as much, and who it comes over or leaves is told. An aura, a bubble round its maker.",
                        ),
                        (
                            "gravity",
                            "any",
                            true,
                            "Gravity inside it in place of the stratum's: a number scales the stratum's (0 is weightlessness), a list of three numbers replaces it (m/s² east, south and up). Left out, its kind's.",
                        ),
                        (
                            "light",
                            "real",
                            true,
                            "The intensity of a source of light it carries at its middle (15 §3.3), on the scale where full daylight is 1. Negative darkens; the light of a cell never goes below zero. Left out, its kind's.",
                        ),
                        (
                            "light_radius",
                            "real",
                            true,
                            "How far that light shines, in metres, at most 16. Left out, as far as it stays noticeable.",
                        ),
                        (
                            "bound",
                            "bool",
                            true,
                            "Whether it is bound to an ability (08 §7.3) and removed when the binding ends. Giving any of the binding's terms below binds it too.",
                        ),
                        crate::ability::bind::TERM_ARGS[0],
                        crate::ability::bind::TERM_ARGS[1],
                        crate::ability::bind::TERM_ARGS[2],
                        crate::ability::bind::TERM_ARGS[3],
                        crate::ability::bind::TERM_ARGS[4],
                    ],
                )
            },
            run: create_area,
        },
        VerbDef {
            name: "remove_area",
            build: |registry| {
                spec(
                    registry,
                    "remove_area",
                    "Removes an area (02 §8.3). Whoever was inside it is told (`area_exited`), and its clock goes. Refused for an area that does not exist.",
                    &[("area", "area", false, "The area.")],
                )
            },
            run: remove_area,
        },
        VerbDef {
            name: "enclose",
            build: |registry| {
                spec(
                    registry,
                    "enclose",
                    "Makes an area of the closed space round a cell (02 §8.3): it fills outward through open cells and, if the space closes within `limit` cells, makes an area over it. Returns the area. Refused, naming a cell it leaks through, for a space that does not close; for a covered start, a fill that reaches unloaded cells, and a limit above 65,536.",
                    &[
                        ("from", "loc", false, "A cell inside the space."),
                        ("kind", "id<area_kind>", true, "The kind of area to make."),
                        (
                            "limit",
                            "int",
                            true,
                            "The most cells it may hold. Default 65,536.",
                        ),
                    ],
                )
            },
            run: enclose,
        },
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_world_verb_registers_with_every_argument_documented() {
        let registry = crate::engine_registry();
        for verb in world_verbs() {
            let id = registry
                .verb_id(verb.name)
                .unwrap_or_else(|| panic!("`{}` is not in the registry", verb.name));
            let spec = &registry.verb(id).unwrap().spec;
            assert!(!spec.doc.is_empty(), "{}", verb.name);
            for arg in &spec.args {
                assert!(!arg.doc.is_empty(), "{}.{}", verb.name, arg.name);
            }
        }
    }

    #[test]
    fn every_reason_is_listed_once() {
        let mut codes: Vec<&str> = WORLD_VERB_REASONS.iter().map(|info| info.code).collect();
        let count = codes.len();
        codes.sort_unstable();
        codes.dedup();
        assert_eq!(codes.len(), count);
    }
}
