//! The kernel's properties and functions (05 §5.2, §4.7; 02 §23;
//! 03 §13): what each reads, and what the settable ones write.
//!
//! Every getter reads ground truth through the simulation and gives
//! null for whatever it cannot answer: a subject that no longer exists,
//! a cell that is not loaded, an argument of the wrong sort. Every
//! setter goes through the function the engine itself uses, so setting
//! a property emits what that function emits (01 R3).
//!
//! The tables here are what [`engine_registry`](crate::engine_registry)
//! enters and what the generated reference lists. The documents state
//! the same entries: 05 §5.3 the core ones, 02 §23 the world's, 03 §13
//! those of time and control, 06 §13 those of components.

use clash_core::{Dur, EntityId, Real};
use clash_data::Mass;
use clash_expr::{DefRef, Value};
use clash_time::ClockId;
use clash_world::defs::{EngineFlag, WorldDefs};
use clash_world::{
    Activity, AreaKindId, BlockId, Cause, CellFlags, CellPos, ElementState, FixtureId, FloorId,
    Layer, Location, OccupantFilter, OccupantKind, SkyTime, distance_m, distinct_ids,
};

use crate::Sim;
use crate::clock::ClockTarget;
use crate::component::{ComponentKind, Identity, Physical};
use crate::entity::{Control, KernelEntity, Place};
use crate::event::{Subject, control_name, layer_name};
use crate::reason::{Reason, codes};
use crate::registry::{EngineDefTypes, FunctionDef, PropertyDef};

/// Millimetres in a metre.
const MM_PER_M: f64 = 1000.0;

/// The heading the functions of space are listed under.
const GROUP_SPACE: &str = "Space";
/// The heading the functions of time are listed under.
const GROUP_TIME: &str = "Time";
/// The heading the functions of data are listed under.
const GROUP_DATA: &str = "Data";

// ---- helpers -------------------------------------------------------------

fn entity(sim: &Sim, subject: Subject) -> Option<&KernelEntity> {
    match subject {
        Subject::Entity(id) => sim.entities.get(id),
        _ => None,
    }
}

fn entity_id(sim: &Sim, subject: Subject) -> Option<EntityId> {
    entity(sim, subject).map(KernelEntity::id)
}

fn opt<T: Into<Value>>(value: Option<T>) -> Value {
    value.map_or(Value::Null, Into::into)
}

fn seconds(span: Dur) -> Value {
    Value::Real(span.as_secs_real())
}

fn texts<'a>(items: impl IntoIterator<Item = &'a str>) -> Value {
    Value::list(items.into_iter().map(Value::from))
}

fn subjects(sim: &Sim, items: impl IntoIterator<Item = Subject>) -> Value {
    Value::list(items.into_iter().map(|subject| sim.subject_value(subject)))
}

fn text_arg(args: &[Value]) -> Option<&str> {
    args.first().and_then(Value::as_str)
}

/// The effective rate of a clock: the product of rates up to universal
/// time.
fn clock_rate(sim: &Sim, clock: Option<ClockId>) -> Value {
    opt(clock.and_then(|clock| sim.timeline.effective_rate(clock)))
}

/// The local time of a clock, in seconds.
fn clock_time(sim: &Sim, clock: Option<ClockId>) -> Value {
    clock
        .and_then(|clock| sim.timeline.local_now(clock).ok())
        .map_or(Value::Null, seconds)
}

/// The cell a subject is a cell of, normalised, if it is one and lies
/// inside its grid.
fn cell(sim: &Sim, subject: Subject) -> Option<Location> {
    sim.cosmos.normalize(subject.loc()?).ok()
}

/// The state of an element and the tables it is read against, if the
/// subject is an element of a loaded cell that holds one.
fn element(sim: &Sim, subject: Subject) -> Option<(Location, Layer, ElementState, &WorldDefs)> {
    let Subject::Element(loc, layer) = subject else {
        return None;
    };
    let state = sim
        .cosmos
        .grid(loc.grid)
        .ok()?
        .element(loc.cell, layer)
        .ok()?;
    (!state.is_none()).then(|| (loc, layer, state, &**sim.cosmos.defs()))
}

/// The qualified id of an element's definition.
fn element_def_name(defs: &WorldDefs, layer: Layer, state: ElementState) -> &str {
    match layer {
        Layer::Floor => defs.floors().name(FloorId::new(state.def)),
        Layer::Block => defs.blocks().name(BlockId::new(state.def)),
        Layer::Fixture => defs.fixtures().name(FixtureId::new(state.def)),
    }
}

/// The tags of an element's definition.
fn element_tags(defs: &WorldDefs, layer: Layer, state: ElementState) -> &[String] {
    defs.element(layer, state)
        .map_or(&[], |element| element.tags.as_slice())
}

/// Where a value is, for distance and sight: an entity where it stands
/// with its height, a cell, an element's cell, an area's middle, a
/// site's anchor.
fn position(sim: &Sim, value: &Value) -> Option<(Location, u16)> {
    match sim.subject_of(value)? {
        Subject::Entity(id) => {
            let top = sim.entities.outermost(id)?;
            match sim.entities.get(top)?.place() {
                Place::At { loc, alt_mm } => Some((loc, alt_mm)),
                Place::In(_) | Place::Stored => None,
            }
        }
        Subject::Loc(loc) | Subject::Element(loc, _) => Some((loc, 0)),
        Subject::Area(id) => {
            let area = sim.cosmos.area(id)?;
            Some((Location::new(area.grid, area.shape.centre()?), 0))
        }
        Subject::Site(id) => {
            let site = sim.cosmos.site(id)?;
            let grid = sim.cosmos.realm(site.realm)?.base_grid;
            Some((Location::new(grid, site.anchor), 0))
        }
        Subject::Tract(..) | Subject::Realm(_) | Subject::Global => None,
    }
}

/// Where a thing is as a character believes it (05 §7.3): the character
/// itself, and anything it holds or is inside, where it is; another
/// entity where the character believes it to be, `None` if it has no
/// idea or the viewer is not an entity; any other place where it is.
/// `dist(a, b)` as it is.
fn dist_now(sim: &Sim, args: &[Value]) -> Value {
    let ends = args
        .first()
        .and_then(|a| position(sim, a))
        .zip(args.get(1).and_then(|b| position(sim, b)));
    opt(ends
        .and_then(|((a, _), (b, _))| sim.cosmos.delta(a, b))
        .map(distance_m))
}

/// `dir(from, to)` as it is.
fn dir_now(sim: &Sim, args: &[Value]) -> Value {
    let ends = args
        .first()
        .and_then(|a| position(sim, a))
        .zip(args.get(1).and_then(|b| position(sim, b)));
    opt(ends
        .and_then(|((a, _), (b, _))| sim.cosmos.delta(a, b))
        .map(|by| {
            Value::list(
                [by.x, by.y, by.z]
                    .into_iter()
                    .map(|part| Value::Int(i64::from(part.signum()))),
            )
        }))
}

fn believed_position(sim: &Sim, viewer: Option<&Value>, value: &Value) -> Option<Location> {
    match sim.subject_of(value)? {
        Subject::Entity(id) => {
            let Some(Subject::Entity(viewer)) = viewer.and_then(|v| sim.subject_of(v)) else {
                return None;
            };
            let top = sim.entities.outermost(id)?;
            if top == viewer || sim.entities.outermost(viewer) == Some(top) {
                return position(sim, value).map(|(loc, _)| loc);
            }
            crate::perception::belief::believed_loc(sim, viewer, id)
        }
        _ => position(sim, value).map(|(loc, _)| loc),
    }
}

fn bad_value(wanted: &str, value: &Value) -> Reason {
    Reason::new(
        codes::WRITE_BAD_VALUE,
        format!("this takes {wanted}, not a {}", value.type_name()),
    )
    .with("found", value.type_name())
}

fn rate_arg(value: &Value) -> Result<Option<Real>, Reason> {
    match value {
        Value::Null => Ok(None),
        other => other
            .as_number()
            .map(Some)
            .ok_or_else(|| bad_value("a rate, or null for none", other)),
    }
}

fn gone(subject: Subject) -> Reason {
    Reason::new(
        codes::SUBJECT_GONE,
        format!("the {} no longer exists", subject.kind()),
    )
    .with("kind", subject.kind())
}

// ---- setters -------------------------------------------------------------

/// `entity.loc = loc`: relocates the entity to stand in the cell; null
/// takes it out of the world.
fn set_entity_loc(
    sim: &mut Sim,
    subject: Subject,
    value: &Value,
    cause: Cause,
) -> Result<(), Reason> {
    let id = entity_id(sim, subject).ok_or_else(|| gone(subject))?;
    let to = match (value, sim.subject_of(value)) {
        (Value::Null, _) => Place::Stored,
        (_, Some(Subject::Loc(loc))) => Place::at(loc),
        (other, _) => return Err(bad_value("a cell, or null to leave the world", other)),
    };
    sim.relocate(id, to, cause).map(|_| ())
}

/// `entity.container = entity`: puts the entity inside another; null
/// takes it out, to where its container is.
fn set_entity_container(
    sim: &mut Sim,
    subject: Subject,
    value: &Value,
    cause: Cause,
) -> Result<(), Reason> {
    let id = entity_id(sim, subject).ok_or_else(|| gone(subject))?;
    let to = match (value, sim.subject_of(value)) {
        (Value::Null, _) => match sim.entities.get(id).map(KernelEntity::place) {
            Some(Place::In(_)) => sim.entities.location(id).map_or(Place::Stored, Place::at),
            // It is in nothing: there is nothing to take it out of.
            _ => return Ok(()),
        },
        (_, Some(Subject::Entity(container))) => Place::In(container),
        (other, _) => return Err(bad_value("an entity, or null to take it out", other)),
    };
    sim.relocate(id, to, cause).map(|_| ())
}

/// `entity.control = 'direct'`.
fn set_entity_control(
    sim: &mut Sim,
    subject: Subject,
    value: &Value,
    cause: Cause,
) -> Result<(), Reason> {
    let id = entity_id(sim, subject).ok_or_else(|| gone(subject))?;
    let control = match value.as_str() {
        Some("none") => Control::None,
        Some("direct") => Control::Direct,
        Some("delegated") => Control::Delegated,
        _ => return Err(bad_value("`none`, `direct` or `delegated`", value)),
    };
    sim.set_control(id, control, cause).map(|_| ())
}

/// `entity.time_rate = 2`: gives the entity a personal clock at that
/// rate; null takes it away.
fn set_entity_time_rate(
    sim: &mut Sim,
    subject: Subject,
    value: &Value,
    cause: Cause,
) -> Result<(), Reason> {
    let id = entity_id(sim, subject).ok_or_else(|| gone(subject))?;
    sim.set_time_rate(id, rate_arg(value)?, cause)
}

/// `entity.own_mass = 80`.
fn set_entity_own_mass(
    sim: &mut Sim,
    subject: Subject,
    value: &Value,
    cause: Cause,
) -> Result<(), Reason> {
    let id = entity_id(sim, subject).ok_or_else(|| gone(subject))?;
    let mass = value
        .as_number()
        .ok_or_else(|| bad_value("a mass in kilograms", value))?;
    sim.set_own_mass(id, Mass::new(mass), cause)
}

/// `area.clock_rate = 0` and `realm.clock_rate = 365`.
fn set_clock_rate(
    sim: &mut Sim,
    subject: Subject,
    value: &Value,
    cause: Cause,
) -> Result<(), Reason> {
    let target = match subject {
        Subject::Area(id) => ClockTarget::Area(id),
        Subject::Realm(id) => ClockTarget::Realm(id),
        other => return Err(gone(other)),
    };
    sim.set_clock_rate(target, rate_arg(value)?, cause)
}

/// `site.name = 'The Vault'`.
fn set_site_name(
    sim: &mut Sim,
    subject: Subject,
    value: &Value,
    _cause: Cause,
) -> Result<(), Reason> {
    let Subject::Site(id) = subject else {
        return Err(gone(subject));
    };
    let name = value
        .as_str()
        .ok_or_else(|| bad_value("a name", value))?
        .to_owned();
    sim.cosmos
        .set_site_name(id, name)
        .map_err(|error| Reason::from_world(&error))
}

// ---- properties ----------------------------------------------------------

/// The tags of whatever carries tags: an entity's own, an area's, a
/// site's, those of an element's definition, and for a cell those of
/// the definitions of its three elements together. `None` for a subject
/// that does not exist or a cell that is not loaded.
fn tags_of(sim: &Sim, subject: Subject) -> Option<Vec<&str>> {
    fn texts(tags: &[String]) -> Vec<&str> {
        tags.iter().map(String::as_str).collect()
    }
    match subject {
        Subject::Entity(id) => Some(sim.entities.get(id)?.tags.iter().collect()),
        Subject::Area(id) => Some(texts(&sim.cosmos.area(id)?.tags)),
        Subject::Site(id) => Some(texts(&sim.cosmos.site(id)?.tags)),
        Subject::Element(..) => {
            let (_, layer, state, defs) = element(sim, subject)?;
            Some(texts(element_tags(defs, layer, state)))
        }
        Subject::Loc(loc) => {
            let defs = sim.cosmos.defs();
            let view = sim.cosmos.grid(loc.grid).ok()?;
            let mut found: Vec<&str> = Vec::new();
            for layer in Layer::ALL {
                let state = view.element(loc.cell, layer).ok()?;
                found.extend(element_tags(defs, layer, state).iter().map(String::as_str));
            }
            found.sort_unstable();
            found.dedup();
            Some(found)
        }
        _ => None,
    }
}

/// The properties every kind has (05 §17): its kind's name, and, where
/// the thing carries tags, `tags` and `has_tag`.
fn core_properties(table: &mut Vec<PropertyDef>) {
    for (kind, _) in crate::subject::SUBJECT_KIND_DOCS {
        table.push(
            PropertyDef::new(
                kind,
                "kind",
                "string",
                "The name of the kind of subject this is: `entity`, `loc`, `element`, `area`, `site`, `tract`, `realm` or `global`.",
                |_, subject, _| Value::from(subject.kind()),
            )
            .safe(),
        );
    }
    let tagged: [(&'static str, &'static str); 5] = [
        ("entity", "The entity's tags, in order."),
        (
            "loc",
            "The tags of the definitions of the cell's floor, block and fixture, in order. Null for a cell that is not loaded.",
        ),
        ("element", "The tags of the element's definition, in order."),
        (
            "area",
            "The area's tags, with those its kind gave it, in order.",
        ),
        ("site", "The site's tags, in order."),
    ];
    for (kind, doc) in tagged {
        table.push(PropertyDef::new(
            kind,
            "tags",
            "list<string>?",
            doc,
            |sim, subject, _| tags_of(sim, subject).map_or(Value::Null, texts),
        ));
        table.push(
            PropertyDef::new(
                kind,
                "has_tag",
                "bool",
                "Whether it carries a tag.",
                |sim, subject, args| {
                    let carried = tags_of(sim, subject)
                        .zip(text_arg(args))
                        .is_some_and(|(tags, tag)| tags.contains(&tag));
                    Value::Bool(carried)
                },
            )
            .arg("tag", "string", "The tag to look for."),
        );
    }
}

fn entity_properties(table: &mut Vec<PropertyDef>) {
    let p = PropertyDef::new;
    table.extend([
        p(
            "entity",
            "loc",
            "loc?",
            "The cell the entity is in: its own, or that of whatever it is inside. Null if it is not in the world. Setting it relocates the entity to stand in that cell (`entity_relocated`); setting null takes it out of the world.",
            |sim, subject, _| {
                opt(entity_id(sim, subject)
                    .and_then(|id| sim.entities.location(id))
                    .map(|loc| sim.subject_value(Subject::Loc(loc))))
            },
        )
        .settable(set_entity_loc)
        .changed_by(&["entity_relocated"]),
        p(
            "entity",
            "container",
            "entity?",
            "What the entity is directly inside, or null. Setting it puts the entity inside another (`entity_relocated`); setting null takes it out, to where its container is.",
            |sim, subject, _| {
                opt(entity(sim, subject)
                    .and_then(|entity| entity.place().container())
                    .map(|id| sim.subject_value(Subject::Entity(id))))
            },
        )
        .settable(set_entity_container)
        .changed_by(&["entity_relocated"]),
        p(
            "entity",
            "outermost",
            "entity?",
            "The entity that is in the world on its behalf: itself, or the container that everything it is nested in is inside.",
            |sim, subject, _| {
                opt(entity_id(sim, subject)
                    .and_then(|id| sim.entities.outermost(id))
                    .map(|id| sim.subject_value(Subject::Entity(id))))
            },
        )
        .changed_by(&["entity_relocated"]),
        p(
            "entity",
            "contents",
            "list<entity>",
            "What is directly inside the entity, in id order.",
            |sim, subject, _| match entity_id(sim, subject) {
                Some(id) => subjects(
                    sim,
                    sim.entities.contents(id).iter().map(|id| Subject::Entity(*id)),
                ),
                None => Value::Null,
            },
        )
        .changed_by(&["entity_relocated"]),
        p(
            "entity",
            "in_world",
            "bool",
            "Whether the entity is in the world: at a cell, or inside something that is.",
            |sim, subject, _| {
                Value::Bool(
                    entity_id(sim, subject).is_some_and(|id| sim.entities.location(id).is_some()),
                )
            },
        )
        .changed_by(&["entity_relocated"]),
        p(
            "entity",
            "altitude",
            "real?",
            "How high the entity stands above the base of its cell, in metres. Null if it is not at a cell itself.",
            |sim, subject, _| match entity(sim, subject).map(KernelEntity::place) {
                Some(Place::At { alt_mm, .. }) => {
                    Value::Real(Real::from(alt_mm) / Real::of(MM_PER_M))
                }
                _ => Value::Null,
            },
        )
        .changed_by(&["entity_relocated"]),
        p(
            "entity",
            "occupant_kind",
            "string",
            "What the world is told the entity is, for queries: `creature`, `item`, `device` or `other`.",
            |sim, subject, _| {
                opt(entity(sim, subject).map(|entity| match entity.presence().kind {
                    OccupantKind::Creature => "creature",
                    OccupantKind::Item => "item",
                    OccupantKind::Device => "device",
                    OccupantKind::Other => "other",
                }))
            },
        ),
        p(
            "entity",
            "def",
            "any",
            "The definition the entity was made from, as an id of whatever type that is. Null for an entity made from none.",
            |sim, subject, _| {
                opt(entity_id(sim, subject)
                    .and_then(|id| sim.components.get::<Identity>(id)?.def())
                    .map(|key| Value::Id(DefRef::from(key))))
            },
        ),
        p(
            "entity",
            "name",
            "string?",
            "The entity's own name, if it has been given one. Null for one that goes by its definition's.",
            |sim, subject, _| {
                opt(entity_id(sim, subject)
                    .and_then(|id| sim.components.get::<Identity>(id)?.name()))
            },
        ),
        p(
            "entity",
            "description",
            "string?",
            "The entity's own description, if it has been given one.",
            |sim, subject, _| {
                opt(entity_id(sim, subject)
                    .and_then(|id| sim.components.get::<Identity>(id)?.description()))
            },
        ),
        p(
            "entity",
            "has_component",
            "bool",
            "Whether the entity has a component of a kind (06 §3.1).",
            |sim, subject, args| {
                let kind = text_arg(args).and_then(ComponentKind::from_name);
                Value::Bool(match (entity_id(sim, subject), kind) {
                    (Some(id), Some(kind)) => sim.components.has(kind, id),
                    _ => false,
                })
            },
        )
        .arg(
            "component",
            "string",
            "The component's name: `identity`, `physical`.",
        ),
        p(
            "entity",
            "mass",
            "real?",
            "The mass of the entity and everything inside it, in kilograms.",
            |sim, subject, _| {
                opt(entity(sim, subject).map(|entity| {
                    match sim.components.get::<Physical>(entity.id()) {
                        Some(physical) => physical.mass().get(),
                        None => entity.presence().mass,
                    }
                }))
            },
        )
        .changed_by(&["entity_relocated"]),
        p(
            "entity",
            "own_mass",
            "real?",
            "The mass of the entity alone, in kilograms. Null for one with no physical component. Setting it brings every container round the entity and the world's record of each up to date.",
            |sim, subject, _| {
                opt(entity_id(sim, subject)
                    .and_then(|id| sim.components.get::<Physical>(id))
                    .map(|physical| physical.own_mass().get()))
            },
        )
        .settable(set_entity_own_mass),
        p(
            "entity",
            "carried_mass",
            "real?",
            "The mass of everything inside the entity, to any depth, in kilograms. Null for one with no physical component.",
            |sim, subject, _| {
                opt(entity_id(sim, subject)
                    .and_then(|id| sim.components.get::<Physical>(id))
                    .map(|physical| physical.carried_mass().get()))
            },
        )
        .changed_by(&["entity_relocated"]),
        p(
            "entity",
            "volume",
            "real?",
            "The space the entity takes up, in cubic metres. Null for one with no physical component.",
            |sim, subject, _| {
                opt(entity_id(sim, subject)
                    .and_then(|id| sim.components.get::<Physical>(id))
                    .map(|physical| physical.volume().get()))
            },
        ),
        p(
            "entity",
            "height",
            "real?",
            "How tall the entity stands, in metres. Null for one with no physical component.",
            |sim, subject, _| {
                opt(entity_id(sim, subject)
                    .and_then(|id| sim.components.get::<Physical>(id))
                    .map(|physical| physical.height().get()))
            },
        ),
        p(
            "entity",
            "control",
            "string",
            "Who decides for the entity (03 §5.1): `none`, `direct` or `delegated`. Setting it emits `control_changed`.",
            |sim, subject, _| {
                opt(entity(sim, subject).map(|entity| control_name(entity.control())))
            },
        )
        .settable(set_entity_control)
        .changed_by(&["control_changed"]),
        p(
            "entity",
            "time_rate",
            "real?",
            "The rate of the entity's personal clock relative to the clock of where it is, or null if it has none (03 §3.5). Setting a rate gives it a personal clock or changes its rate; setting null takes the clock away. Either emits `clock_rate_changed`.",
            |sim, subject, _| {
                opt(entity(sim, subject)
                    .and_then(KernelEntity::clock)
                    .and_then(|clock| sim.timeline.rate(clock)))
            },
        )
        .settable(set_entity_time_rate)
        .changed_by(&["clock_rate_changed"]),
        p(
            "entity",
            "clock_rate",
            "real?",
            "How fast time passes for the entity against universal time: the rate of the clock it is on, with every clock above it. Null if it is on none.",
            |sim, subject, _| clock_rate(sim, entity_id(sim, subject).and_then(|id| sim.clock_of(id))),
        ),
        p(
            "entity",
            "local_time",
            "real?",
            "The local time of the clock the entity is on, in seconds. Null if it is on none.",
            |sim, subject, _| clock_time(sim, entity_id(sim, subject).and_then(|id| sim.clock_of(id))),
        ),
        p(
            "entity",
            "own_time",
            "real?",
            "How much local time has passed for the entity, on whatever clocks it has been on while it was in the world, in seconds.",
            |sim, subject, _| opt(entity_id(sim, subject).map(|id| seconds(sim.own_time(id)))),
        ),
    ]);
}

fn loc_properties(table: &mut Vec<PropertyDef>) {
    let p = PropertyDef::new;
    table.extend([
        p("loc", "x", "int", "The cell's coordinate east.", |_, subject, _| {
            opt(subject.loc().map(|loc| i64::from(loc.cell.x)))
        })
        .safe(),
        p("loc", "y", "int", "The cell's coordinate south.", |_, subject, _| {
            opt(subject.loc().map(|loc| i64::from(loc.cell.y)))
        })
        .safe(),
        p("loc", "z", "int", "The cell's storey.", |_, subject, _| {
            opt(subject.loc().map(|loc| i64::from(loc.cell.z)))
        })
        .safe(),
        p("loc", "grid", "int", "The id of the grid the cell is in.", |_, subject, _| {
            opt(subject.loc().map(|loc| i64::from(loc.grid.raw())))
        })
        .safe(),
        p(
            "loc",
            "realm",
            "realm?",
            "The realm the cell's grid belongs to.",
            |sim, subject, _| {
                opt(subject
                    .loc()
                    .and_then(|loc| sim.cosmos.realm_of(loc.grid))
                    .map(|realm| sim.subject_value(Subject::Realm(realm))))
            },
        )
        .safe(),
        p(
            "loc",
            "tract",
            "tract?",
            "The tract the cell is in. Null for a cell outside its realm, or of a grid that is not a realm's base grid.",
            |sim, subject, _| {
                opt(cell(sim, subject).and_then(|loc| {
                    let realm = sim.cosmos.realm_of(loc.grid)?;
                    (sim.cosmos.realm(realm)?.base_grid == loc.grid)
                        .then(|| sim.subject_value(Subject::Tract(realm, loc.cell.tract())))
                }))
            },
        )
        .safe(),
        p(
            "loc",
            "site",
            "site?",
            "The site the cell's tract belongs to, or null.",
            |sim, subject, _| {
                opt(cell(sim, subject)
                    .and_then(|loc| sim.cosmos.site_of(loc).ok().flatten())
                    .map(|site| sim.subject_value(Subject::Site(site))))
            },
        ),
        p(
            "loc",
            "areas",
            "list<area>",
            "The areas that cover the cell, in id order.",
            |sim, subject, _| match cell(sim, subject).and_then(|loc| sim.cosmos.areas_at(loc).ok()) {
                Some(areas) => subjects(sim, areas.into_iter().map(Subject::Area)),
                None => Value::Null,
            },
        )
        .changed_by(&["area_entered", "area_exited"]),
        p(
            "loc",
            "in_area",
            "bool",
            "Whether an area of a kind covers the cell.",
            |sim, subject, args| {
                let wanted = match args.first() {
                    Some(Value::Id(id)) => AreaKindId::from_index(id.index as usize),
                    Some(Value::Str(name)) => sim.cosmos.defs().area_kinds().lookup(name),
                    _ => None,
                };
                Value::Bool(match (cell(sim, subject), wanted) {
                    (Some(loc), Some(wanted)) => sim.cosmos.areas_at(loc).is_ok_and(|areas| {
                        areas.iter().any(|id| {
                            sim.cosmos.area(*id).is_some_and(|area| area.kind == Some(wanted))
                        })
                    }),
                    _ => false,
                })
            },
        )
        .arg("kind", "id<area_kind>", "The kind of area."),
        p(
            "loc",
            "in_area_tagged",
            "bool",
            "Whether an area that carries a tag covers the cell.",
            |sim, subject, args| {
                Value::Bool(match (cell(sim, subject), text_arg(args)) {
                    (Some(loc), Some(tag)) => sim.cosmos.areas_at(loc).is_ok_and(|areas| {
                        areas
                            .iter()
                            .any(|id| sim.cosmos.area(*id).is_some_and(|area| area.has_tag(tag)))
                    }),
                    _ => false,
                })
            },
        )
        .arg("tag", "string", "The tag to look for."),
        p(
            "loc",
            "floor",
            "element?",
            "The cell's floor, or null if it has none or is not loaded.",
            |sim, subject, _| layer_of(sim, subject, Layer::Floor),
        )
        .changed_by(&["element_changed"]),
        p(
            "loc",
            "block",
            "element?",
            "The cell's block, or null if it has none or is not loaded.",
            |sim, subject, _| layer_of(sim, subject, Layer::Block),
        )
        .changed_by(&["element_changed"]),
        p(
            "loc",
            "fixture",
            "element?",
            "The cell's fixture, or null if it has none or is not loaded.",
            |sim, subject, _| layer_of(sim, subject, Layer::Fixture),
        )
        .changed_by(&["element_changed"]),
        p(
            "loc",
            "is_loaded",
            "bool",
            "Whether the cell is inside its grid and its tract is loaded.",
            |sim, subject, _| Value::Bool(subject.loc().is_some_and(|loc| sim.cosmos.is_loaded(loc))),
        )
        .changed_by(&["tract_loaded", "tract_unloading"]),
        p(
            "loc",
            "standable",
            "bool?",
            "Whether the cell can be stood in: it has a floor, or the cell below has a block with a solid top (02 §5.2). Null for a cell that is not loaded.",
            |sim, subject, _| opt(subject.loc().and_then(|loc| sim.cosmos.standable(loc).ok())),
        )
        .changed_by(&["element_changed"]),
        p(
            "loc",
            "is_open",
            "bool?",
            "Whether the cell is open: it holds nothing and cannot be stood in.",
            |sim, subject, _| opt(subject.loc().and_then(|loc| sim.cosmos.is_open(loc).ok())),
        )
        .changed_by(&["element_changed"]),
        p(
            "loc",
            "obstacle_height",
            "real?",
            "The height in metres of the tallest block or fixture in the cell that is in the way.",
            |sim, subject, _| {
                opt(subject.loc().and_then(|loc| sim.cosmos.obstacle_height(loc).ok()))
            },
        )
        .changed_by(&["element_changed"]),
        p(
            "loc",
            "under_open_sky",
            "bool?",
            "Whether nothing stands between the cell and the sky above it.",
            |sim, subject, _| {
                opt(subject.loc().and_then(|loc| sim.cosmos.under_open_sky(loc).ok()))
            },
        )
        .changed_by(&["element_changed"]),
        p(
            "loc",
            "has_flag",
            "bool?",
            "Whether the cell has one of the flags derived from its elements (02 §6.2): `walk_blocked`, `opaque`, `has_floor`, `solid_top` or `fragile`. Null for a cell that is not loaded or a name that is not a flag.",
            |sim, subject, args| {
                let flag = text_arg(args).and_then(|name| {
                    CellFlags::NAMED
                        .iter()
                        .find(|(_, have)| *have == name)
                        .map(|(flag, _)| *flag)
                });
                opt(subject
                    .loc()
                    .zip(flag)
                    .and_then(|(loc, flag)| Some(sim.cosmos.flags(loc).ok()?.contains(flag))))
            },
        )
        .arg("flag", "string", "The flag's name.")
        .changed_by(&["element_changed"]),
        p(
            "loc",
            "light",
            "real?",
            "How bright the cell is now (15 §3.3): what the sky and the fixed sources give it. Null for a cell that is not loaded.",
            |sim, subject, _| light_at(sim, subject.loc()),
        ),
        p(
            "loc",
            "gravity",
            "real?",
            "Ambient gravity at the cell in m/s², downward: its stratum's, as the areas that cover it change it.",
            |sim, subject, _| {
                opt(subject
                    .loc()
                    .and_then(|loc| sim.cosmos.ambient(loc).ok())
                    .map(|ambient| ambient.gravity))
            },
        ),
        p(
            "loc",
            "atmosphere",
            "id<atmosphere>?",
            "The atmosphere at the cell. Null is vacuum.",
            |sim, subject, _| {
                opt(subject
                    .loc()
                    .and_then(|loc| sim.cosmos.ambient(loc).ok()?.atmosphere)
                    .map(|id| EngineDefTypes::id(sim.exprs.def_types().atmosphere, id.index())))
            },
        ),
        p(
            "loc",
            "time_of_day",
            "real?",
            "The time of day at the cell, in seconds since midnight, by the sky its stratum names. Null where there is no sky. Safe for belief: anyone may know the hour, as a screen's clock shows it (decision D-872).",
            |sim, subject, _| {
                opt(subject.loc().and_then(|loc| {
                    let sky = sim.cosmos.sky(loc).ok()??;
                    let clock = sim.cosmos.grid(loc.grid).ok()?.clock();
                    let now = sim.timeline.local_now(clock).ok()?;
                    let day = sky.day.as_micros().max(1);
                    let of_day = (sky.starts_at.as_micros() + now.as_micros()).rem_euclid(day);
                    Some(seconds(Dur::from_micros(of_day)))
                }))
            },
        )
        .safe(),
        p(
            "loc",
            "occupants",
            "list<entity>",
            "The entities that have a cell of their footprint here, in id order.",
            |sim, subject, _| {
                match subject
                    .loc()
                    .and_then(|loc| sim.cosmos.occupants_at(loc, &OccupantFilter::ANY).ok())
                {
                    Some(found) => {
                        subjects(sim, distinct_ids(&found).into_iter().map(Subject::Entity))
                    }
                    None => Value::Null,
                }
            },
        )
        .changed_by(&["entity_relocated"]),
        p(
            "loc",
            "clock_rate",
            "real?",
            "How fast time passes at the cell against universal time: the rate of the clock things there run on, with every clock above it.",
            |sim, subject, _| clock_rate(sim, subject.loc().map(|loc| sim.location_clock(loc))),
        )
        .changed_by(&["clock_rate_changed"]),
        p(
            "loc",
            "local_time",
            "real?",
            "The local time of the clock things at the cell run on, in seconds.",
            |sim, subject, _| clock_time(sim, subject.loc().map(|loc| sim.location_clock(loc))),
        ),
    ]);
}

/// A cell's element on a layer as a subject, or null.
fn layer_of(sim: &Sim, subject: Subject, layer: Layer) -> Value {
    let Some(loc) = cell(sim, subject) else {
        return Value::Null;
    };
    let wanted = Subject::Element(loc, layer);
    match element(sim, wanted) {
        Some(_) => sim.subject_value(wanted),
        None => Value::Null,
    }
}

/// The light at a cell now.
fn light_at(sim: &Sim, loc: Option<Location>) -> Value {
    opt(loc.and_then(|loc| {
        let time = SkyTime::now(&sim.cosmos, &sim.timeline, loc.grid).ok()?;
        sim.cosmos.light(loc, time).ok()
    }))
}

fn element_properties(table: &mut Vec<PropertyDef>) {
    let p = PropertyDef::new;
    table.extend([
        p(
            "element",
            "layer",
            "string",
            "Which of its cell's elements it is: `floor`, `block` or `fixture`.",
            |_, subject, _| match subject {
                Subject::Element(_, layer) => Value::from(layer_name(layer)),
                _ => Value::Null,
            },
        )
        .safe(),
        p("element", "loc", "loc", "The cell it is in.", |sim, subject, _| {
            opt(subject.loc().map(|loc| sim.subject_value(Subject::Loc(loc))))
        })
        .safe(),
        p(
            "element",
            "def",
            "any",
            "Its definition, as an id of `floor`, `block` or `fixture` according to its layer. Null if the cell holds no such element or is not loaded.",
            |sim, subject, _| {
                opt(element(sim, subject).map(|(_, layer, state, _)| {
                    let types = sim.exprs.def_types();
                    let ty = match layer {
                        Layer::Floor => types.floor,
                        Layer::Block => types.block,
                        Layer::Fixture => types.fixture,
                    };
                    EngineDefTypes::id(ty, usize::from(state.def))
                }))
            },
        )
        .changed_by(&["element_changed"]),
        p(
            "element",
            "def_id",
            "string?",
            "The qualified id of its definition, as a text.",
            |sim, subject, _| {
                opt(element(sim, subject)
                    .map(|(_, layer, state, defs)| element_def_name(defs, layer, state).to_owned()))
            },
        )
        .changed_by(&["element_changed"]),
        p(
            "element",
            "material",
            "id<material>?",
            "What it is made of.",
            |sim, subject, _| {
                opt(element(sim, subject).map(|(_, _, state, _)| {
                    EngineDefTypes::id(sim.exprs.def_types().material, state.material.index())
                }))
            },
        )
        .changed_by(&["element_changed"]),
        p(
            "element",
            "state",
            "int?",
            "Its state: the index of the named state its definition declares, or the definition's own number (an amount, a facing).",
            |sim, subject, _| {
                opt(element(sim, subject).map(|(_, _, state, _)| i64::from(state.bits)))
            },
        )
        .changed_by(&["element_changed"]),
        p(
            "element",
            "state_name",
            "string?",
            "The name of its state, where its definition declares named states.",
            |sim, subject, _| {
                opt(element(sim, subject).and_then(|(_, layer, state, defs)| {
                    defs.element(layer, state)?
                        .state_names
                        .get(usize::from(state.bits))
                        .cloned()
                }))
            },
        )
        .changed_by(&["element_changed"]),
        p(
            "element",
            "damage",
            "int?",
            "Its accumulated damage, in 255ths of what it can take (02 §12).",
            |sim, subject, _| {
                opt(element(sim, subject).and_then(|(loc, layer, _, _)| {
                    let damage = sim.cosmos.damage(loc).ok()?;
                    damage.get(layer.index()).map(|parts| i64::from(*parts))
                }))
            },
        )
        .changed_by(&["element_damaged", "element_changed"]),
        p(
            "element",
            "height",
            "real?",
            "How high it stands from the base of its cell, in metres. Zero for a floor.",
            |sim, subject, _| {
                opt(element(sim, subject)
                    .map(|(_, layer, state, defs)| defs.info(layer, state).height))
            },
        )
        .changed_by(&["element_changed"]),
        p(
            "element",
            "has_flag",
            "bool?",
            "Whether its state has one of the engine's flags (02 §5.4): `opaque`, `passable`, `load_bearing`, `anchor`, `ramp`, `stairs`, `ladder`, `liquid`, `grate`, `rough` or `fragile`. Null for a name that is not a flag.",
            |sim, subject, args| {
                let flag = text_arg(args)
                    .and_then(|name| EngineFlag::ALL.into_iter().find(|flag| flag.name() == name));
                opt(element(sim, subject)
                    .zip(flag)
                    .map(|((_, layer, state, defs), flag)| defs.flags(layer, state).has(flag)))
            },
        )
        .arg("flag", "string", "The flag's name.")
        .changed_by(&["element_changed"]),
    ]);
}

fn area_properties(table: &mut Vec<PropertyDef>) {
    let p = PropertyDef::new;
    table.extend([
        p(
            "area",
            "def",
            "id<area_kind>?",
            "The kind that gave the area its defaults and carries its rules, if it has one.",
            |sim, subject, _| {
                let Subject::Area(id) = subject else {
                    return Value::Null;
                };
                opt(sim
                    .cosmos
                    .area(id)
                    .and_then(|area| area.kind)
                    .map(|kind| EngineDefTypes::id(sim.exprs.def_types().area_kind, kind.index())))
            },
        ),
        p("area", "site", "site?", "The site the area belongs to, if any.", |sim, subject, _| {
            let Subject::Area(id) = subject else {
                return Value::Null;
            };
            opt(sim
                .cosmos
                .area(id)
                .and_then(|area| area.site)
                .map(|site| sim.subject_value(Subject::Site(site))))
        }),
        p(
            "area",
            "priority",
            "int?",
            "Which area applies where several overlap: the highest.",
            |sim, subject, _| {
                let Subject::Area(id) = subject else {
                    return Value::Null;
                };
                opt(sim.cosmos.area(id).map(|area| i64::from(area.priority)))
            },
        ),
        p("area", "grid", "int?", "The id of the grid the area is in.", |sim, subject, _| {
            let Subject::Area(id) = subject else {
                return Value::Null;
            };
            opt(sim.cosmos.area(id).map(|area| i64::from(area.grid.raw())))
        }),
        p(
            "area",
            "loc",
            "loc?",
            "The cell at the middle of the area.",
            |sim, subject, _| {
                let Subject::Area(id) = subject else {
                    return Value::Null;
                };
                opt(sim.cosmos.area(id).and_then(|area| {
                    let centre = area.shape.centre()?;
                    Some(sim.subject_value(Subject::Loc(Location::new(area.grid, centre))))
                }))
            },
        ),
        p(
            "area",
            "contains",
            "bool?",
            "Whether the area covers a cell.",
            |sim, subject, args| {
                let Subject::Area(id) = subject else {
                    return Value::Null;
                };
                let at = args
                    .first()
                    .and_then(|value| position(sim, value))
                    .and_then(|(loc, _)| sim.cosmos.normalize(loc).ok());
                opt(sim.cosmos.area(id).zip(at).and_then(|(area, at)| {
                    let topology = sim.cosmos.topology(area.grid).ok()?;
                    Some(area.grid == at.grid && area.shape.contains(topology, at.cell))
                }))
            },
        )
        .arg("at", "any", "A cell, or something that is somewhere."),
        p(
            "area",
            "occupants",
            "list<entity>",
            "The entities inside the area, each once, in the order their cells come.",
            |sim, subject, _| {
                let Subject::Area(id) = subject else {
                    return Value::Null;
                };
                match sim.cosmos.occupants_in_area(id, &OccupantFilter::ANY) {
                    Ok(found) => {
                        subjects(sim, distinct_ids(&found).into_iter().map(Subject::Entity))
                    }
                    Err(_) => Value::Null,
                }
            },
        )
        .changed_by(&["area_entered", "area_exited"]),
        p(
            "area",
            "breached",
            "bool?",
            "Whether the area was made by enclosing a space that no longer closes.",
            |sim, subject, _| {
                let Subject::Area(id) = subject else {
                    return Value::Null;
                };
                opt(sim.cosmos.area(id).map(clash_world::Area::is_breached))
            },
        )
        .changed_by(&["area_breached", "area_closed"]),
        p(
            "area",
            "clock_rate",
            "real?",
            "The rate of the area's own clock relative to the clock above it, or null if it has none (03 §3.4). Setting a rate gives the area a clock or changes its rate; setting null takes the clock away. Whoever is inside is put on the clock that now applies, and `clock_rate_changed` is emitted.",
            |sim, subject, _| {
                let Subject::Area(id) = subject else {
                    return Value::Null;
                };
                opt(sim
                    .cosmos
                    .area(id)
                    .and_then(|area| area.clock)
                    .and_then(|clock| sim.timeline.rate(clock)))
            },
        )
        .settable(set_clock_rate)
        .changed_by(&["clock_rate_changed"]),
        p(
            "area",
            "local_time",
            "real?",
            "The local time of the area's own clock, in seconds, or null if it has none.",
            |sim, subject, _| {
                let Subject::Area(id) = subject else {
                    return Value::Null;
                };
                clock_time(sim, sim.cosmos.area(id).and_then(|area| area.clock))
            },
        ),
    ]);
}

fn site_properties(table: &mut Vec<PropertyDef>) {
    let p = PropertyDef::new;
    table.extend([
        p(
            "site",
            "name",
            "string?",
            "The site's name, as shown. It can be set.",
            |sim, subject, _| {
                let Subject::Site(id) = subject else {
                    return Value::Null;
                };
                opt(sim.cosmos.site(id).map(|site| site.name.clone()))
            },
        )
        .settable(set_site_name),
        p(
            "site",
            "realm",
            "realm?",
            "The realm the site is in.",
            |sim, subject, _| {
                let Subject::Site(id) = subject else {
                    return Value::Null;
                };
                opt(sim
                    .cosmos
                    .site(id)
                    .map(|site| sim.subject_value(Subject::Realm(site.realm))))
            },
        ),
        p(
            "site",
            "loc",
            "loc?",
            "The cell that stands for the place: its door, its heart.",
            |sim, subject, _| {
                let Subject::Site(id) = subject else {
                    return Value::Null;
                };
                opt(sim.cosmos.site(id).and_then(|site| {
                    let grid = sim.cosmos.realm(site.realm)?.base_grid;
                    Some(sim.subject_value(Subject::Loc(Location::new(grid, site.anchor))))
                }))
            },
        ),
        p(
            "site",
            "tracts",
            "list<tract>",
            "The tracts the site spans, in order.",
            |sim, subject, _| {
                let Subject::Site(id) = subject else {
                    return Value::Null;
                };
                match sim.cosmos.site(id) {
                    Some(site) => subjects(
                        sim,
                        site.tracts
                            .iter()
                            .map(|tract| Subject::Tract(site.realm, *tract)),
                    ),
                    None => Value::Null,
                }
            },
        ),
    ]);
}

fn tract_properties(table: &mut Vec<PropertyDef>) {
    let p = PropertyDef::new;
    table.extend([
        p("tract", "realm", "realm", "The realm the tract is in.", |sim, subject, _| {
            match subject {
                Subject::Tract(realm, _) => sim.subject_value(Subject::Realm(realm)),
                _ => Value::Null,
            }
        })
        .safe(),
        p("tract", "x", "int", "The tract's coordinate east, in tracts.", |_, subject, _| {
            match subject {
                Subject::Tract(_, tract) => Value::Int(i64::from(tract.x)),
                _ => Value::Null,
            }
        })
        .safe(),
        p("tract", "y", "int", "The tract's coordinate south, in tracts.", |_, subject, _| {
            match subject {
                Subject::Tract(_, tract) => Value::Int(i64::from(tract.y)),
                _ => Value::Null,
            }
        })
        .safe(),
        p(
            "tract",
            "loc",
            "loc?",
            "The tract's cell with the lowest horizontal coordinates, at storey 0, in its realm's base grid.",
            |sim, subject, _| {
                let Subject::Tract(realm, tract) = subject else {
                    return Value::Null;
                };
                opt(sim.cosmos.realm(realm).map(|realm| {
                    let origin: CellPos = tract.origin(0);
                    sim.subject_value(Subject::Loc(Location::new(realm.base_grid, origin)))
                }))
            },
        )
        .safe(),
        p(
            "tract",
            "loaded",
            "bool",
            "Whether the tract is loaded.",
            |sim, subject, _| match subject {
                Subject::Tract(realm, tract) => {
                    Value::Bool(sim.cosmos.is_tract_loaded(realm, tract))
                }
                _ => Value::Null,
            },
        )
        .changed_by(&["tract_loaded", "tract_unloading"]),
        p(
            "tract",
            "activity",
            "string?",
            "How active the tract is, as of the last safe point (02 §10.1): `hot`, `warm`, or null for one the tickets do not keep.",
            |sim, subject, _| {
                let Subject::Tract(realm, tract) = subject else {
                    return Value::Null;
                };
                opt(sim.cosmos.activity(realm, tract).map(|level| match level {
                    Activity::Hot => "hot",
                    Activity::Warm => "warm",
                }))
            },
        ),
        p(
            "tract",
            "surveyed",
            "bool",
            "Whether the tract's region has been surveyed, so that the tract has a record.",
            |sim, subject, _| match subject {
                Subject::Tract(realm, tract) => {
                    Value::Bool(sim.cosmos.tract_record(realm, tract).is_some())
                }
                _ => Value::Null,
            },
        ),
        p(
            "tract",
            "devastation",
            "int?",
            "How devastated the tract is, 0 to 255 (02 §17.2). Null for a tract with no record.",
            |sim, subject, _| {
                let Subject::Tract(realm, tract) = subject else {
                    return Value::Null;
                };
                opt(sim
                    .cosmos
                    .tract_record(realm, tract)
                    .map(|record| i64::from(record.devastation)))
            },
        ),
        p(
            "tract",
            "site",
            "site?",
            "The site the tract belongs to, or null.",
            |sim, subject, _| {
                let Subject::Tract(realm, tract) = subject else {
                    return Value::Null;
                };
                opt(sim
                    .cosmos
                    .site_at(realm, tract)
                    .map(|site| sim.subject_value(Subject::Site(site))))
            },
        ),
    ]);
}

fn realm_properties(table: &mut Vec<PropertyDef>) {
    let p = PropertyDef::new;
    table.extend([
        p("realm", "def", "id<realm>?", "The realm's definition.", |sim, subject, _| {
            let Subject::Realm(id) = subject else {
                return Value::Null;
            };
            opt(sim
                .cosmos
                .realm(id)
                .map(|realm| EngineDefTypes::id(sim.exprs.def_types().realm, realm.def.index())))
        }),
        p(
            "realm",
            "grid",
            "int?",
            "The id of the realm's base grid.",
            |sim, subject, _| {
                let Subject::Realm(id) = subject else {
                    return Value::Null;
                };
                opt(sim
                    .cosmos
                    .realm(id)
                    .map(|realm| i64::from(realm.base_grid.raw())))
            },
        ),
        p(
            "realm",
            "clock_rate",
            "real?",
            "The rate of the realm's clock against universal time (03 §3.4). Setting it changes the rate and emits `clock_rate_changed`; a realm always has a clock, so null is refused.",
            |sim, subject, _| {
                let Subject::Realm(id) = subject else {
                    return Value::Null;
                };
                opt(sim
                    .cosmos
                    .realm(id)
                    .and_then(|realm| sim.timeline.rate(realm.clock)))
            },
        )
        .settable(set_clock_rate)
        .changed_by(&["clock_rate_changed"]),
        p(
            "realm",
            "local_time",
            "real?",
            "The local time of the realm's clock, in seconds.",
            |sim, subject, _| {
                let Subject::Realm(id) = subject else {
                    return Value::Null;
                };
                clock_time(sim, sim.cosmos.realm(id).map(|realm| realm.clock))
            },
        ),
    ]);
}

fn global_properties(table: &mut Vec<PropertyDef>) {
    table.push(
        PropertyDef::new(
            "global",
            "now",
            "real",
            "Universal time, in seconds since the simulation began.",
            |sim, _, _| seconds(sim.now().since(clash_core::Tick::ZERO)),
        )
        .safe(),
    );
}

/// Every property the kernel offers.
pub(crate) fn properties() -> Vec<PropertyDef> {
    let mut table = Vec::new();
    core_properties(&mut table);
    entity_properties(&mut table);
    loc_properties(&mut table);
    element_properties(&mut table);
    area_properties(&mut table);
    site_properties(&mut table);
    tract_properties(&mut table);
    realm_properties(&mut table);
    global_properties(&mut table);
    table
}

// ---- functions -----------------------------------------------------------

/// Every function the kernel offers beyond those the expression
/// language computes itself (05 §4.7: space, time, data).
pub(crate) fn functions() -> Vec<FunctionDef> {
    let f = FunctionDef::new;
    vec![
        f(
            "dist",
            "real?",
            GROUP_SPACE,
            "The distance between two things in metres, a storey counting three, the shorter way round on wrapped axes (02 §3.3). Each is a cell, an element, or something that is somewhere: an entity, an area (its middle), a site (its anchor). Null if either is nowhere or they are in different grids. In a mind context it is `believed_dist` with the character first: an entity is where the character believes it is.",
            dist_now,
        )
        .arg("a", "any", "One end.")
        .arg("b", "any", "The other end.")
        .safe()
        .believed_as("believed_dist")
        .changed_by(&["entity_relocated"]),
        f(
            "believed_dist",
            "real?",
            GROUP_SPACE,
            "`dist` as a character believes it: what `dist(a, b)` is compiled as in that character's mind context (05 §7.3). The character itself, and whatever it holds or is inside, is where it is; anyone else is where it believes them to be (`believed_loc`), null if it has no idea; cells, elements, areas and sites are where they are. Reads that character's beliefs, so it is not safe for belief itself. Where an expression compiled in a mind context is held to truth, as an action's `requires` is when it starts (03 §6.6), it is `dist(a, b)`.",
            |sim, args| {
                let viewer = args.first();
                let ends = args
                    .get(1)
                    .and_then(|a| believed_position(sim, viewer, a))
                    .zip(args.get(2).and_then(|b| believed_position(sim, viewer, b)));
                opt(ends
                    .and_then(|(a, b)| sim.cosmos.delta(a, b))
                    .map(distance_m))
            },
        )
        .arg("viewer", "any", "The character whose belief is read.")
        .arg("a", "any", "One end.")
        .arg("b", "any", "The other end.")
        .in_truth(|sim, args| dist_now(sim, args.get(1..).unwrap_or_default()))
        .changed_by(&["entity_relocated"]),
        f(
            "believed_dir",
            "list<int>?",
            GROUP_SPACE,
            "`dir` as a character believes it, as `believed_dist` is `dist`: what `dir(from, to)` is compiled as in that character's mind context. Where that is held to truth, as an action's `requires` is when it starts, it is `dir(from, to)`.",
            |sim, args| {
                let viewer = args.first();
                let ends = args
                    .get(1)
                    .and_then(|a| believed_position(sim, viewer, a))
                    .zip(args.get(2).and_then(|b| believed_position(sim, viewer, b)));
                opt(ends.and_then(|(a, b)| sim.cosmos.delta(a, b)).map(|by| {
                    Value::list(
                        [by.x, by.y, by.z]
                            .into_iter()
                            .map(|part| Value::Int(i64::from(part.signum()))),
                    )
                }))
            },
        )
        .arg("viewer", "any", "The character whose belief is read.")
        .arg("from", "any", "Where the step starts.")
        .arg("to", "any", "What it is toward.")
        .in_truth(|sim, args| dir_now(sim, args.get(1..).unwrap_or_default()))
        .changed_by(&["entity_relocated"]),
        f(
            "dir",
            "list<int>?",
            GROUP_SPACE,
            "The step from one thing toward another: the signs of the shortest offset east, south and up, each -1, 0 or 1. Null where `dist` is null. In a mind context it is `believed_dir` with the character first.",
            dir_now,
        )
        .arg("from", "any", "Where the step starts.")
        .arg("to", "any", "What it is toward.")
        .safe()
        .believed_as("believed_dir")
        .changed_by(&["entity_relocated"]),
        f(
            "offset",
            "loc?",
            GROUP_SPACE,
            "The cell so many cells east, south and up of a cell, wrapped on wrapped axes. Null beyond an edge of the realm.",
            |sim, args| {
                let Some(Subject::Loc(loc)) = args.first().and_then(|at| sim.subject_of(at))
                else {
                    return Value::Null;
                };
                let part = |at: usize| {
                    args.get(at)
                        .map_or(Some(0), Value::as_int)
                        .and_then(|part| i32::try_from(part).ok())
                };
                let (Some(dx), Some(dy), Some(dz)) = (part(1), part(2), part(3)) else {
                    return Value::Null;
                };
                let (Some(x), Some(y), Some(z)) = (
                    loc.cell.x.checked_add(dx),
                    loc.cell.y.checked_add(dy),
                    loc.cell.z.checked_add(dz),
                ) else {
                    return Value::Null;
                };
                opt(sim
                    .cosmos
                    .normalize(Location::new(loc.grid, CellPos::new(x, y, z)))
                    .ok()
                    .map(|to| sim.subject_value(Subject::Loc(to))))
            },
        )
        .arg("at", "loc", "The cell to start from.")
        .arg("dx", "int", "Cells east.")
        .arg("dy", "int", "Cells south.")
        .optional("dz", "int", "Storeys up. Zero if left out.")
        .safe(),
        f(
            "places",
            "list<element>",
            GROUP_SPACE,
            "The elements, blocks and fixtures, within `radius` metres of a cell (as `dist` measures, a storey counting three; at most 16) whose definition lists an interaction, nearest the cell first and then by cell (storey, row, column), leaving out those whose cell is in `except`: where a searcher goes to look (09 §4.1). Which elements stand where and what their definitions offer is static geometry (09 §3.4), known to every mind, so it is safe for belief; an element tagged `hidden` is not listed, nor is a cell that is not loaded. Not what state anything is in or what it holds. Empty for an interaction that is not loaded.",
            |sim, args| {
                let Some(Subject::Loc(at) | Subject::Element(at, _)) =
                    args.first().and_then(|at| sim.subject_of(at))
                else {
                    return Value::list(std::iter::empty());
                };
                let radius = args.get(1).and_then(Value::as_number).unwrap_or(Real::ZERO);
                let Some(interaction) = args
                    .get(2)
                    .and_then(Value::as_str)
                    .and_then(|name| crate::interaction::lookup(sim, name))
                else {
                    return Value::list(std::iter::empty());
                };
                let except: Vec<Location> = args
                    .get(3)
                    .and_then(Value::as_list)
                    .map(|list| {
                        list.iter()
                            .filter_map(|value| match sim.subject_of(value) {
                                Some(Subject::Loc(loc) | Subject::Element(loc, _)) => Some(loc),
                                _ => None,
                            })
                            .collect()
                    })
                    .unwrap_or_default();
                Value::list(
                    crate::interaction::places_offering(sim, at, radius, interaction, &except)
                        .into_iter()
                        .map(|(loc, layer)| sim.subject_value(Subject::Element(loc, layer))),
                )
            },
        )
        .arg("at", "any", "The cell to look about, or an element (its cell). Null gives an empty list.")
        .arg("radius", "real", "How far from it, in metres.")
        .arg("interaction", "string", "The qualified id of the interaction (`core:search`).")
        .optional("except", "list<any>", "Cells, or elements by their cell, to leave out: those already searched.")
        .safe(),
        f(
            "los",
            "bool?",
            GROUP_SPACE,
            "Whether terrain leaves a clear line between two things, each as for `dist`, an entity seen from and at the height it stands at (02 §22). Null if either is nowhere, or the line comes to space that is not loaded before anything blocks it.",
            |sim, args| {
                let ends = args
                    .first()
                    .and_then(|a| position(sim, a))
                    .zip(args.get(1).and_then(|b| position(sim, b)));
                opt(ends.and_then(|((a, a_alt), (b, b_alt))| {
                    sim.cosmos.line_of_sight_between(a, a_alt, b, b_alt).ok()
                }))
            },
        )
        .arg("a", "any", "One end.")
        .arg("b", "any", "The other end.")
        .changed_by(&["entity_relocated", "element_changed"]),
        f(
            "light",
            "real?",
            GROUP_SPACE,
            "How bright a cell is now (15 §3.3). The same as its `light` property.",
            |sim, args| {
                light_at(
                    sim,
                    args.first()
                        .and_then(|at| position(sim, at))
                        .map(|(loc, _)| loc),
                )
            },
        )
        .arg("at", "any", "A cell, or something that is somewhere."),
        f(
            "now",
            "real",
            GROUP_TIME,
            "Universal time, in seconds since the simulation began.",
            |sim, _| seconds(sim.now().since(clash_core::Tick::ZERO)),
        )
        .safe(),
        f(
            "local_now",
            "real?",
            GROUP_TIME,
            "The local time of the clock a subject is on, in seconds (03 §3.4): an entity's clock, the clock of a cell, an area's or a realm's own. Null for a subject on no clock.",
            |sim, args| {
                let clock = match args.first().and_then(|subject| sim.subject_of(subject)) {
                    Some(Subject::Entity(id)) => sim.clock_of(id),
                    Some(Subject::Loc(loc) | Subject::Element(loc, _)) => {
                        Some(sim.location_clock(loc))
                    }
                    Some(Subject::Area(id)) => sim.cosmos.area(id).and_then(|area| area.clock),
                    Some(Subject::Realm(id)) => sim.cosmos.realm(id).map(|realm| realm.clock),
                    Some(Subject::Global) => Some(ClockId::UNIVERSAL),
                    _ => None,
                };
                clock_time(sim, clock)
            },
        )
        .arg("subject", "any", "What to read the clock of."),
        f(
            "setting",
            "any",
            GROUP_DATA,
            "The value of a setting (04 §10), or null if it has none.",
            |sim, args| {
                opt(text_arg(args)
                    .and_then(|key| sim.settings.get(key))
                    .map(Value::from))
            },
        )
        .arg("id", "string", "The setting's qualified id.")
        .safe(),
        f(
            "has_def",
            "bool",
            GROUP_DATA,
            "Whether a definition of a type with a qualified id is loaded.",
            |sim, args| {
                let registry = sim.exprs.registry();
                let found = args
                    .first()
                    .and_then(Value::as_str)
                    .and_then(|ty| registry.def_type_id(ty))
                    .zip(args.get(1).and_then(Value::as_str))
                    .and_then(|(ty, id)| registry.def_ref(ty, id));
                Value::Bool(found.is_some())
            },
        )
        .arg("type", "string", "The definition type: `material`, `block`.")
        .arg("id", "string", "The qualified id.")
        .safe(),
    ]
}
