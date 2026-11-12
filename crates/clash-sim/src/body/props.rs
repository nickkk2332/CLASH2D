//! What the expression language knows of bodies: the kinds, properties
//! and events this module registers (05 §5.3; 06 §13), and the function
//! that answers the properties from the stores.
//!
//! # How a body is written
//!
//! | Written | Type | Is |
//! |---------|------|----|
//! | `self.fn.walking` | `real` | A function's capacity |
//! | `self.fn.value(id)` | `real` | The same, for a function chosen at run time |
//! | `self.scale` | `real?` | The body's overall size factor |
//! | `self.reach` | `real?` | How far it can reach to strike, in metres |
//! | `self.part('l_arm')` | `part?` | A part by its name in the body |
//! | `self.parts` | `list<part>` | Every part that is there |
//! | `self.wounds` | `list<wound>` | Every wound |
//! | `part.condition` | `real` | From 0 to 1 |
//! | `part.state` | `string` | `whole`, `disabled`, `severed` or `destroyed` |
//! | `wound.depth` | `int` | The layers it goes through |
//!
//! `fn` is a property of an entity that gives a subject of the kind
//! `functions`: the same entity, seen as what its body can do. Each
//! `body_function` definition is a property of that kind, under its
//! qualified id and, where nothing else has the name, its bare id. A
//! misspelt function is therefore a load error with a suggestion.
//!
//! A part and a wound are subjects of their own. Their handles hold the
//! owner's slot, the low sixteen bits of its generation and the part's
//! key or the wound's id ([`part_handle`], [`wound_handle`]).
//!
//! None of these is safe for belief: what a body can do, what state its
//! parts are in and what wounds it carries are its owner's to know, and
//! anyone else's to perceive (15). A character reads its own. None is
//! settable: bodies change through hits and the body verbs, so that
//! hooks run and what follows is reported. Anything read of an entity
//! with no body, of a part or wound that is not there, or by an id that
//! names nothing is null.

use clash_core::{Dur, EntityId, LookupMap, Real};
use clash_data::{Def, Diagnostic, RegistryCx, code};
use clash_expr::{
    DefRef, DefTypeId, EventSpec, Handle, KindId, KindSpec, PropertyId, PropertySpec, Registry,
    RegistryError, Type, Value,
};
use clash_time::{ClockId, Timeline};
use clash_world::defs::{DamageTypeDef, MaterialDef};

use super::defs::{BodyDefs, BodyFunctionDef, BodyFunctionId, BodyKinds, BodyPartDef, BodyPlanDef};
use super::{Body, BodyStore, PartInst, PartKey, Relation, Wound, WoundId};

/// The kind of subject that has a body.
pub const ENTITY_KIND: &str = "entity";
/// The kind `entity.fn` gives: what an entity's body can do.
pub const FUNCTIONS_KIND: &str = "functions";
/// The kind of a part of a body.
pub const PART_KIND: &str = "part";
/// The kind of a wound on a part.
pub const WOUND_KIND: &str = "wound";

/// The hook that runs before a hit is applied to a body.
pub const HIT_INCOMING: &str = "hit_incoming";
/// The event raised for each part a hit did something to.
pub const HIT_LANDED: &str = "hit_landed";
/// The event raised when a hit breaches a part.
pub const WOUND_INFLICTED: &str = "wound_inflicted";
/// The event raised when a wound stops leaking.
pub const WOUND_CLOSED: &str = "wound_closed";
/// The event raised when a closed wound is opened again.
pub const WOUND_REOPENED: &str = "wound_reopened";
/// The event raised when a wound is gone.
pub const WOUND_HEALED: &str = "wound_healed";
/// The event raised when a part stops working.
pub const PART_DISABLED: &str = "part_disabled";
/// The event raised when a part works again.
pub const PART_RESTORED: &str = "part_restored";
/// The event raised when a part comes off.
pub const PART_SEVERED: &str = "part_severed";
/// The event raised when a part is destroyed.
pub const PART_DESTROYED: &str = "part_destroyed";
/// The event raised when a function's capacity moves.
pub const FUNCTION_CHANGED: &str = "function_changed";

/// One of this module's registered properties.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum Prop {
    /// `entity.fn`.
    Functions,
    /// `functions.<function>`.
    Function(BodyFunctionId),
    /// `functions.value(id)`.
    FunctionValue,
    /// `entity.scale`.
    Scale,
    /// `entity.reach`.
    Reach,
    /// `entity.part(name)`.
    Part,
    /// `entity.parts`.
    Parts,
    /// `entity.wounds`.
    Wounds,
    /// A property of a part.
    OfPart(PartProp),
    /// A property of a wound.
    OfWound(WoundProp),
}

/// The properties of a part.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum PartProp {
    Name,
    Def,
    Owner,
    Parent,
    Side,
    Condition,
    State,
    Working,
    Exposed,
    Size,
    Length,
    Girth,
    Mass,
    Reach,
    Layers,
    LayerDamage,
    LayerMaterial,
    LayerThickness,
    HasTag,
    Wounds,
    Inside,
    Attached,
}

/// The properties of a wound.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum WoundProp {
    Part,
    Owner,
    Kind,
    Depth,
    Area,
    Energy,
    Age,
    State,
    Rate,
    Stump,
}

/// The names of the properties of a part, with which each is.
const PART_PROPS: &[(&str, PartProp)] = &[
    ("name", PartProp::Name),
    ("def", PartProp::Def),
    ("owner", PartProp::Owner),
    ("parent", PartProp::Parent),
    ("side", PartProp::Side),
    ("condition", PartProp::Condition),
    ("state", PartProp::State),
    ("working", PartProp::Working),
    ("exposed", PartProp::Exposed),
    ("size", PartProp::Size),
    ("length", PartProp::Length),
    ("girth", PartProp::Girth),
    ("mass", PartProp::Mass),
    ("reach", PartProp::Reach),
    ("layers", PartProp::Layers),
    ("layer_damage", PartProp::LayerDamage),
    ("layer_material", PartProp::LayerMaterial),
    ("layer_thickness", PartProp::LayerThickness),
    ("has_tag", PartProp::HasTag),
    ("wounds", PartProp::Wounds),
    ("inside", PartProp::Inside),
    ("attached", PartProp::Attached),
];

/// The names of the properties of a wound, with which each is.
const WOUND_PROPS: &[(&str, WoundProp)] = &[
    ("part", WoundProp::Part),
    ("owner", WoundProp::Owner),
    ("kind", WoundProp::Kind),
    ("depth", WoundProp::Depth),
    ("area", WoundProp::Area),
    ("energy", WoundProp::Energy),
    ("age", WoundProp::Age),
    ("state", WoundProp::State),
    ("rate", WoundProp::Rate),
    ("stump", WoundProp::Stump),
];

/// The handle of a part of an entity's body: the entity's slot, the low
/// sixteen bits of its generation, and the part's key.
pub fn part_handle(kinds: BodyKinds, entity: EntityId, part: PartKey) -> Handle {
    Handle::new(kinds.part, pack(entity, part.raw()))
}

/// The handle of a wound of an entity's body: the entity's slot, the low
/// sixteen bits of its generation, and the wound's id.
pub fn wound_handle(kinds: BodyKinds, entity: EntityId, wound: WoundId) -> Handle {
    Handle::new(kinds.wound, pack(entity, wound.raw()))
}

/// The part a handle of the kind `part` stands for, if its owner still
/// has a body: the owner and the part's key. The part may have gone.
pub fn part_of_handle(bodies: &BodyStore, handle: Handle) -> Option<(EntityId, PartKey)> {
    let (entity, index) = unpack(bodies, handle.payload)?;
    Some((entity, PartKey::new(index)))
}

/// The wound a handle of the kind `wound` stands for, if its owner
/// still has a body: the owner and the wound's id. The wound may have
/// healed.
pub fn wound_of_handle(bodies: &BodyStore, handle: Handle) -> Option<(EntityId, WoundId)> {
    let (entity, index) = unpack(bodies, handle.payload)?;
    Some((entity, WoundId::new(index)))
}

/// The bits of a generation a handle keeps.
const GENERATION_BITS: u32 = 16;

fn pack(entity: EntityId, index: u16) -> u64 {
    let generation = u64::from(entity.generation()) & ((1 << GENERATION_BITS) - 1);
    u64::from(entity.slot()) | (u64::from(index) << 32) | (generation << 48)
}

/// The owner a packed payload names, if the body in that slot is still
/// the same entity's as far as the handle can tell, and the index.
fn unpack(bodies: &BodyStore, payload: u64) -> Option<(EntityId, u16)> {
    let slot = payload as u32;
    let index = (payload >> 32) as u16;
    let generation = (payload >> 48) as u16;
    let owner = bodies.owner_in_slot(slot)?;
    (owner.generation() as u16 == generation).then_some((owner, index))
}

/// The part of a qualified id after its namespace.
fn bare(qualified: &str) -> &str {
    qualified
        .rsplit_once(':')
        .map_or(qualified, |(_, bare)| bare)
}

/// The dense number of the definition a per-definition property name
/// stands for: a qualified id exactly, or a bare one if exactly one
/// definition of the type has it.
fn def_by_property_name(registry: &Registry, ty: DefTypeId, name: &str) -> Option<u32> {
    if name.contains(':') {
        return registry.def_ref(ty, name).map(|found| found.index);
    }
    let mut matches = registry
        .def_type(ty)?
        .ids()
        .filter(|(qualified, _)| bare(qualified) == name);
    let (_, index) = matches.next()?;
    matches.next().is_none().then_some(index)
}

/// Which registered property is which, for [`BodyDefs`].
pub(super) fn table(registry: &Registry, kinds: BodyKinds) -> LookupMap<PropertyId, Prop> {
    let mut table = LookupMap::new();
    let mut fixed = vec![
        (kinds.entity, "fn", Prop::Functions),
        (kinds.entity, "scale", Prop::Scale),
        (kinds.entity, "reach", Prop::Reach),
        (kinds.entity, "part", Prop::Part),
        (kinds.entity, "parts", Prop::Parts),
        (kinds.entity, "wounds", Prop::Wounds),
        (kinds.functions, "value", Prop::FunctionValue),
    ];
    fixed.extend(
        PART_PROPS
            .iter()
            .map(|(name, prop)| (kinds.part, *name, Prop::OfPart(*prop))),
    );
    fixed.extend(
        WOUND_PROPS
            .iter()
            .map(|(name, prop)| (kinds.wound, *name, Prop::OfWound(*prop))),
    );
    for (kind, name, prop) in fixed {
        if let Some(id) = registry.property_id(kind, name) {
            table.insert(id, prop);
        }
    }
    if let Some(ty) = registry.def_type_id(BodyFunctionDef::TYPE) {
        for property in registry.properties_of(kinds.functions) {
            if table.contains_key(&property.id) || !property.spec.params.is_empty() {
                continue;
            }
            let function = def_by_property_name(registry, ty, &property.spec.name)
                .and_then(|index| BodyFunctionId::from_index(index as usize));
            if let Some(function) = function {
                table.insert(property.id, Prop::Function(function));
            }
        }
    }
    table
}

/// The kind with this name, registered if it is not there.
fn kind(registry: &mut Registry, name: &str, doc: &str) -> Result<KindId, RegistryError> {
    match registry.kind_id(name) {
        Some(kind) => Ok(kind),
        None => registry.add_kind(KindSpec::new(name).doc(doc)),
    }
}

/// The definition type with this name, registered if it is not there.
fn def_type(registry: &mut Registry, name: &str, doc: &str) -> Result<DefTypeId, RegistryError> {
    match registry.def_type_id(name) {
        Some(ty) => Ok(ty),
        None => registry.add_def_type(name, doc),
    }
}

/// Registers everything the expression language knows of bodies: the
/// kinds `functions`, `part` and `wound`, the properties of the
/// [module](self) table and of the tables in 06 §6.7, and the events and
/// the hook of 06 §7.6.
///
/// The registry must already hold the ids of the `body_function`
/// definitions (the loader puts them there before any registry step
/// runs). The kind `entity` is the kernel's; it is registered here only
/// if nobody has, so that a tool without a kernel can still load bodies.
pub fn register(registry: &mut Registry) -> Result<(), RegistryError> {
    let entity = kind(
        registry,
        ENTITY_KIND,
        "Any entity: a creature, an item, a device.",
    )?;
    let functions = registry.add_kind(
        KindSpec::new(FUNCTIONS_KIND)
            .doc("What one entity's body can do, reached as `entity.fn` (06 §6.3)."),
    )?;
    let part = kind(registry, PART_KIND, "A part of a body (06 §6).")?;
    let wound = kind(
        registry,
        WOUND_KIND,
        "A wound on a part of a body (06 §7.3).",
    )?;
    let body_part = def_type(registry, BodyPartDef::TYPE, BodyPartDef::DOC)?;
    def_type(registry, BodyPlanDef::TYPE, BodyPlanDef::DOC)?;
    let body_function = def_type(registry, BodyFunctionDef::TYPE, BodyFunctionDef::DOC)?;
    let damage_type = def_type(registry, DamageTypeDef::TYPE, DamageTypeDef::DOC)?;
    let material = def_type(registry, MaterialDef::TYPE, MaterialDef::DOC)?;
    let part_ty = Type::Subject(part);
    let wound_ty = Type::Subject(wound);
    let kind_ty = Type::Id(damage_type);

    // The events first: the properties name them.
    registry.add_event(
        EventSpec::new(HIT_INCOMING)
            .subject("self", entity)
            .subject("actor", entity)
            .subject("instigator", entity)
            .field("part", part_ty.clone().nullable(), "The part the hit is aimed at. Null for a hit at a height, anywhere, or spread over the body.")
            .field("height", Type::Real.nullable(), "The height above the target's footing it arrives at, in metres, for a hit aimed at a height.")
            .field("spread", Type::Bool, "Whether it is spread over the whole body.")
            .field("kind", kind_ty.clone(), "The kind of damage.")
            .field("energy", Type::Real, "Joules delivered.")
            .field("intensity", Type::Real, "How concentrated the delivery is, in pascals.")
            .field("area", Type::Real, "The area of contact, in square metres.")
            .field("span", Type::Real, "The length of contact across the part, in metres.")
            .field("dir", Type::list(Type::Real).nullable(), "The direction it travels in: east, south and up.")
            .hook(["energy", "intensity", "part", "dir"])
            .doc("Runs before a hit is applied to a body (06 §7.1). A handler may change its energy, its intensity, the part it lands on and its direction, or cancel it. `actor` and `instigator` are whoever is behind the hit."),
    )?;
    registry.add_event(
        EventSpec::new(HIT_LANDED)
            .subject("self", entity)
            .subject("actor", entity)
            .subject("instigator", entity)
            .field("part", part_ty.clone(), "The part struck.")
            .field("kind", kind_ty.clone(), "The kind of damage.")
            .field("absorbed", Type::Real, "Joules the part's own layers took.")
            .field("passed", Type::Real, "Joules that went on through them.")
            .field("depth", Type::Int, "How many of the part's layers the hit got through.")
            .doc("A hit did something to a part. Raised once for each part reached, outermost first: the part struck, then a part inside it."),
    )?;
    registry.add_event(
        EventSpec::new(WOUND_INFLICTED)
            .subject("self", entity)
            .subject("actor", entity)
            .subject("instigator", entity)
            .field("wound", wound_ty.clone(), "The wound.")
            .field("part", part_ty.clone(), "The part it is on.")
            .field("kind", kind_ty.clone(), "The kind of damage.")
            .field("depth", Type::Int, "How many layers it goes through.")
            .field("area", Type::Real, "Its area, in square metres.")
            .doc("A hit breached a part, or made a wound of the same kind that was open there worse. For a part that came off, the wound is the stump, on the part it was joined to."),
    )?;
    let of_wound = |name: &str, doc: &str| {
        EventSpec::new(name)
            .subject("self", entity)
            .subject("actor", entity)
            .subject("instigator", entity)
            .field("wound", wound_ty.clone(), "The wound.")
            .field("part", part_ty.clone(), "The part it is on.")
            .doc(doc)
    };
    registry.add_event(of_wound(
        WOUND_CLOSED,
        "A wound stopped leaking: its time came, or `close_wound` closed it. What it goes through begins to heal.",
    ))?;
    registry.add_event(of_wound(
        WOUND_REOPENED,
        "A closed wound was opened again by `reopen_wound`: it leaks as it did.",
    ))?;
    registry.add_event(
        EventSpec::new(WOUND_HEALED)
            .subject("self", entity)
            .subject("actor", entity)
            .subject("instigator", entity)
            .field("part", part_ty.clone(), "The part it was on.")
            .field("kind", kind_ty.clone(), "The kind of damage it was.")
            .doc("A wound is gone: every layer it went through is whole again."),
    )?;
    let of_part = |name: &str, doc: &str| {
        EventSpec::new(name)
            .subject("self", entity)
            .subject("actor", entity)
            .subject("instigator", entity)
            .field("part", part_ty.clone(), "The part.")
            .doc(doc)
    };
    registry.add_event(of_part(
        PART_DISABLED,
        "A part stopped working: its condition fell below its `disable_below`, or something disabled it. It contributes nothing and its channel slots are gone.",
    ))?;
    registry.add_event(of_part(
        PART_RESTORED,
        "A part works again: its condition is back, or what disabled it let go.",
    ))?;
    registry.add_event(
        of_part(
            PART_SEVERED,
            "A part came off, with everything on it and in it. Raised for the part cut, not for what went with it.",
        )
        .field("parent", part_ty.clone().nullable(), "The part it was joined to."),
    )?;
    registry.add_event(of_part(
        PART_DESTROYED,
        "A part was destroyed: every layer of it is gone. What was on it came off with it. For the root, the body is in pieces.",
    ))?;
    registry.add_event(
        EventSpec::new(FUNCTION_CHANGED)
            .subject("self", entity)
            .subject("actor", entity)
            .subject("instigator", entity)
            .field("function", Type::Id(body_function), "The function.")
            .field("old", Type::Real, "Its capacity before.")
            .field("new", Type::Real, "Its capacity now.")
            .doc("What the body can do of something changed: a part that provides it was damaged, healed, disabled, restored or lost."),
    )?;

    let state_events = [PART_DISABLED, PART_RESTORED, PART_SEVERED, PART_DESTROYED];
    let wound_events = [WOUND_INFLICTED, WOUND_HEALED];

    // Of an entity.
    registry.add_property(
        PropertySpec::new(entity, "fn", Type::Subject(functions))
            .doc(
                "What the entity's body can do: follow with a function's id, as in `self.fn.<id>`.",
            )
            .yields_own(true),
    )?;
    registry.add_property(
        PropertySpec::new(entity, "scale", Type::Real.nullable())
            .doc("The body's overall size factor. Null for something with no body."),
    )?;
    registry.add_property(
        PropertySpec::new(entity, "reach", Type::Real.nullable())
            .doc("How far the body can reach to strike, in metres: the greatest length, from the trunk out, among its working parts that have a striking surface. Null for something with no body.")
            .changed_by(state_events),
    )?;
    registry.add_property(
        PropertySpec::new(entity, "part", part_ty.clone().nullable())
            .param("name", Type::Str, "The part's name in the body: its `id` in the plan.")
            .doc("A part of the entity's body by name, one that is gone included. Null if the body has no part of that name.")
            .yields_own(true),
    )?;
    registry.add_property(
        PropertySpec::new(entity, "parts", Type::list(part_ty.clone()))
            .doc("Every part of the entity's body that is there, in the order they were made. Empty for something with no body.")
            .yields_own(true)
            .changed_by([PART_SEVERED, PART_DESTROYED]),
    )?;
    registry.add_property(
        PropertySpec::new(entity, "wounds", Type::list(wound_ty.clone()))
            .doc("Every wound on the entity's body, oldest id first. Empty for something with no body.")
            .yields_own(true)
            .changed_by(wound_events),
    )?;

    // Of its functions.
    registry.add_property(
        PropertySpec::new(functions, "value", Type::Real)
            .param("function", Type::Id(body_function), "Which function.")
            .doc("The capacity of a function chosen by id. Null for something with no body.")
            .changed_by([FUNCTION_CHANGED]),
    )?;
    let ids: Vec<String> = registry
        .def_type(body_function)
        .map(|def_type| def_type.ids().map(|(name, _)| name.to_owned()).collect())
        .unwrap_or_default();
    let of_function = |name: &str, qualified: &str| {
        PropertySpec::new(functions, name, Type::Real)
            .doc(format!(
                "The capacity of the function `{qualified}`: what the working parts that provide it contribute. Null for something with no body."
            ))
            .changed_by([FUNCTION_CHANGED])
    };
    for qualified in &ids {
        registry.add_property(of_function(qualified, qualified))?;
    }
    for qualified in &ids {
        let short = bare(qualified);
        let unique = ids.iter().filter(|other| bare(other) == short).count() == 1;
        if unique && registry.property_id(functions, short).is_none() {
            // A bare id that is a reserved word cannot be written after
            // a dot; the qualified one can.
            let _ = registry.add_property(of_function(short, qualified));
        }
    }

    // Of a part.
    let of = |name: &str, ty: Type, doc: &str| PropertySpec::new(part, name, ty).doc(doc);
    let layer = |name: &str, ty: Type, doc: &str| {
        PropertySpec::new(part, name, ty.nullable())
            .param("layer", Type::Int, "Which layer, from 0 for the outermost.")
            .doc(doc)
    };
    let part_props = [
        of("name", Type::Str, "The part's name in its body."),
        of("def", Type::Id(body_part), "What kind of part it is."),
        of(
            "owner",
            Type::Subject(entity),
            "Whose body it is a part of.",
        )
        .yields_own(true),
        of(
            "parent",
            part_ty.clone().nullable(),
            "The part it is joined to. Null for the root.",
        )
        .yields_own(true),
        of(
            "side",
            Type::Str.nullable(),
            "Its side label, if the plan gave it one.",
        ),
        of(
            "condition",
            Type::Real,
            "Its condition, from 0 to 1: 1 less each layer's weight times its damage. 0 for a part that is gone.",
        ),
        of(
            "state",
            Type::Str,
            "`whole`, `disabled`, `severed` or `destroyed`.",
        )
        .changed_by(state_events),
        of(
            "working",
            Type::Bool,
            "Whether it is there and not disabled.",
        )
        .changed_by(state_events),
        of(
            "exposed",
            Type::Bool,
            "Whether it is there and on the outside: it sits inside no other part.",
        )
        .changed_by([PART_SEVERED, PART_DESTROYED]),
        of(
            "size",
            Type::Real,
            "The area it presents, in square metres: its length times its girth.",
        ),
        of("length", Type::Real, "How long it is, in metres."),
        of("girth", Type::Real, "How far across it is, in metres."),
        of(
            "mass",
            Type::Real,
            "Its mass, in kilograms. 0 for a part that is gone.",
        ),
        of(
            "reach",
            Type::Real,
            "How far it can reach from the trunk, in metres: its own length and that of every part between it and the root.",
        ),
        of(
            "layers",
            Type::Int,
            "How many layers it is built of. 0 for a part that is gone.",
        ),
        layer(
            "layer_damage",
            Type::Real,
            "How damaged a layer is, from 0 to 1. Null if the part has no such layer.",
        ),
        layer(
            "layer_material",
            Type::Id(material),
            "What a layer is made of. Null if the part has no such layer.",
        ),
        layer(
            "layer_thickness",
            Type::Real,
            "How thick a layer is, in metres. Null if the part has no such layer.",
        ),
        PropertySpec::new(part, "has_tag", Type::Bool)
            .param("tag", Type::Str, "The tag.")
            .doc("Whether the part's definition carries a tag."),
        of("wounds", Type::list(wound_ty.clone()), "The wounds on it.")
            .yields_own(true)
            .changed_by(wound_events),
        of(
            "inside",
            Type::list(part_ty.clone()),
            "The parts that are there and sit inside it.",
        )
        .yields_own(true),
        of(
            "attached",
            Type::list(part_ty.clone()),
            "The parts that are there and are attached on it.",
        )
        .yields_own(true),
    ];
    for spec in part_props {
        registry.add_property(spec)?;
    }

    // Of a wound.
    let of = |name: &str, ty: Type, doc: &str| PropertySpec::new(wound, name, ty).doc(doc);
    let wound_props = [
        of("part", part_ty.clone(), "The part it is on.").yields_own(true),
        of("owner", Type::Subject(entity), "Whose wound it is.").yields_own(true),
        of("kind", kind_ty, "The kind of damage that made it."),
        of(
            "depth",
            Type::Int,
            "How many of the part's layers it goes through, from the outside.",
        ),
        of("area", Type::Real, "Its area, in square metres."),
        of(
            "energy",
            Type::Real,
            "The joules the part absorbed in making it.",
        ),
        of(
            "age",
            Type::Real,
            "How long ago it was made, in seconds of its owner's time.",
        ),
        of("state", Type::Str, "`open` or `closed`.").changed_by([WOUND_CLOSED, WOUND_REOPENED]),
        of(
            "rate",
            Type::Real,
            "What it drains from pools each second, every pool summed. 0 once it is closed.",
        )
        .changed_by([WOUND_CLOSED, WOUND_REOPENED]),
        of("stump", Type::Bool, "Whether it is where a part came off."),
    ];
    for spec in wound_props {
        registry.add_property(spec)?;
    }
    Ok(())
}

/// The registry step of this module (04 §7.3): [`register`], run by the
/// loader once the ids of the mods being loaded are known and before any
/// expression is compiled.
pub fn registry_step(cx: &mut RegistryCx<'_>) {
    if let Err(error) = register(cx.registry()) {
        cx.report(Diagnostic::new(
            code::VALIDATION,
            format!("bodies could not be registered with the expression language: {error}"),
        ));
    }
}

fn real(value: Option<Real>) -> Value {
    value.map_or(Value::Null, Value::Real)
}

fn int(value: usize) -> Value {
    Value::Int(i64::try_from(value).unwrap_or(i64::MAX))
}

fn id(ty: Option<DefTypeId>, index: usize) -> Value {
    match (ty, u32::try_from(index)) {
        (Some(ty), Ok(index)) => Value::Id(DefRef::new(ty, index)),
        _ => Value::Null,
    }
}

fn parts<'a>(
    kinds: BodyKinds,
    entity: EntityId,
    parts: impl Iterator<Item = &'a PartInst>,
) -> Value {
    Value::list(parts.map(|part| Value::Subject(part_handle(kinds, entity, part.key()))))
}

fn wounds<'a>(
    kinds: BodyKinds,
    entity: EntityId,
    wounds: impl Iterator<Item = &'a Wound>,
) -> Value {
    Value::list(wounds.map(|wound| Value::Subject(wound_handle(kinds, entity, wound.id()))))
}

/// Answers a property of a part.
fn answer_part(
    body: &Body,
    defs: &BodyDefs,
    entity: EntityId,
    part: &PartInst,
    prop: PartProp,
    args: &[Value],
) -> Value {
    let kinds = defs.kinds();
    let layer = || {
        let index = usize::try_from(args.first()?.as_int()?).ok()?;
        part.layers().get(index)
    };
    match prop {
        PartProp::Name => Value::text(part.name()),
        PartProp::Def => id(Some(defs.def_types().body_part), part.def().index()),
        PartProp::Owner => Value::Subject(Handle::new(kinds.entity, entity.to_bits())),
        PartProp::Parent => part.parent().map_or(Value::Null, |parent| {
            Value::Subject(part_handle(kinds, entity, parent))
        }),
        PartProp::Side => part.side().map_or(Value::Null, Value::text),
        PartProp::Condition => Value::Real(part.condition()),
        PartProp::State => Value::text(part.state().name()),
        PartProp::Working => Value::Bool(part.is_working()),
        PartProp::Exposed => Value::Bool(body.is_exposed(part.key())),
        PartProp::Size => Value::Real(body.part_area(defs, part)),
        PartProp::Length => Value::Real(body.part_length(defs, part)),
        PartProp::Girth => Value::Real(body.part_girth(defs, part)),
        PartProp::Mass => Value::Real(body.part_mass(defs, part)),
        PartProp::Reach => Value::Real(body.part_reach(defs, part.key())),
        PartProp::Layers => int(part.layers().len()),
        PartProp::LayerDamage => real(layer().map(|layer| layer.damage_fraction())),
        PartProp::LayerMaterial => layer().map_or(Value::Null, |layer| {
            id(defs.def_types().material, layer.material().index())
        }),
        PartProp::LayerThickness => real(layer().map(|layer| layer.thickness())),
        PartProp::HasTag => {
            let tag = match args.first() {
                Some(Value::Str(name)) => defs.defs().tags().lookup(name),
                _ => None,
            };
            let has = tag.is_some_and(|tag| {
                defs.part(part.def())
                    .is_some_and(|def| def.tags.contains(tag))
            });
            Value::Bool(has)
        }
        PartProp::Wounds => wounds(kinds, entity, body.wounds_on(part.key())),
        PartProp::Inside => parts(kinds, entity, body.inside(part.key())),
        PartProp::Attached => parts(
            kinds,
            entity,
            body.children(part.key())
                .filter(|child| child.is_present() && child.relation() == Relation::On),
        ),
    }
}

/// Answers a property of a wound.
fn answer_wound(
    body: &Body,
    defs: &BodyDefs,
    now: &dyn Fn(ClockId) -> Option<Dur>,
    entity: EntityId,
    wound: &Wound,
    prop: WoundProp,
) -> Value {
    let kinds = defs.kinds();
    match prop {
        WoundProp::Part => Value::Subject(part_handle(kinds, entity, wound.part())),
        WoundProp::Owner => Value::Subject(Handle::new(kinds.entity, entity.to_bits())),
        WoundProp::Kind => id(defs.def_types().damage_type, wound.kind().index()),
        WoundProp::Depth => Value::Int(i64::from(wound.depth())),
        WoundProp::Area => Value::Real(wound.area()),
        WoundProp::Energy => Value::Real(wound.energy()),
        WoundProp::Age => {
            let age = match now(body.clock()) {
                Some(now) => body.wound_age(wound, now),
                None => wound.age(),
            };
            Value::Real(age.as_secs_real())
        }
        WoundProp::State => Value::text(wound.state().name()),
        WoundProp::Rate => {
            let mut rate = Real::ZERO;
            for leak in wound.leaks() {
                rate += leak.rate;
            }
            Value::Real(rate)
        }
        WoundProp::Stump => Value::Bool(wound.lost().is_some()),
    }
}

/// The function an id argument names: an id value, or a text holding a
/// qualified id.
fn function_arg(defs: &BodyDefs, args: &[Value]) -> Option<BodyFunctionId> {
    let ty = defs.def_types().body_function;
    let index = match args.first()? {
        Value::Id(id) if id.ty == ty => id.index as usize,
        Value::Str(text) => defs.defs().expr_registry().def_ref(ty, text)?.index as usize,
        _ => return None,
    };
    BodyFunctionId::from_index(index)
}

/// Answers a property this module knows, given which it is.
fn answer(
    bodies: &BodyStore,
    defs: &BodyDefs,
    now: &dyn Fn(ClockId) -> Option<Dur>,
    prop: Prop,
    subject: Handle,
    args: &[Value],
) -> Value {
    let kinds = defs.kinds();
    let wanted = match prop {
        Prop::Functions | Prop::Scale | Prop::Reach | Prop::Part | Prop::Parts | Prop::Wounds => {
            kinds.entity
        }
        Prop::Function(_) | Prop::FunctionValue => kinds.functions,
        Prop::OfPart(_) => kinds.part,
        Prop::OfWound(_) => kinds.wound,
    };
    if subject.kind != wanted {
        return Value::Null;
    }
    match prop {
        Prop::OfPart(prop) => {
            let found = part_of_handle(bodies, subject).and_then(|(entity, key)| {
                let body = bodies.get(entity)?;
                Some((entity, body, body.part(key)?))
            });
            match found {
                Some((entity, body, part)) => answer_part(body, defs, entity, part, prop, args),
                None => Value::Null,
            }
        }
        Prop::OfWound(prop) => {
            let found = wound_of_handle(bodies, subject).and_then(|(entity, id)| {
                let body = bodies.get(entity)?;
                Some((entity, body, body.wound(id)?))
            });
            match found {
                Some((entity, body, wound)) => answer_wound(body, defs, now, entity, wound, prop),
                None => Value::Null,
            }
        }
        Prop::Functions => Value::Subject(Handle::new(kinds.functions, subject.payload)),
        _ => {
            let entity = EntityId::from_bits(subject.payload);
            let body = bodies.get(entity);
            match prop {
                Prop::Function(function) => real(body.map(|body| body.function(function))),
                Prop::FunctionValue => real(
                    body.zip(function_arg(defs, args))
                        .map(|(body, function)| body.function(function)),
                ),
                Prop::Scale => real(body.map(Body::scale)),
                Prop::Reach => real(body.map(|body| body.reach(defs))),
                Prop::Part => {
                    let part = match args.first() {
                        Some(Value::Str(name)) => body.and_then(|body| body.part_named(name)),
                        _ => None,
                    };
                    part.map_or(Value::Null, |part| {
                        Value::Subject(part_handle(kinds, entity, part.key()))
                    })
                }
                Prop::Parts => parts(
                    kinds,
                    entity,
                    body.into_iter()
                        .flat_map(|body| body.parts().iter())
                        .filter(|part| part.is_present()),
                ),
                Prop::Wounds => wounds(
                    kinds,
                    entity,
                    body.into_iter().flat_map(|body| body.wounds().iter()),
                ),
                Prop::Functions | Prop::OfPart(_) | Prop::OfWound(_) => Value::Null,
            }
        }
    }
}

/// Answers a property of this module from the stores: what the
/// simulation's `Host::property` forwards.
///
/// `None` means the property is not one of this module's, and the caller
/// should look elsewhere. `Some(Value::Null)` means it is, and there is
/// nothing to read: the entity has no body, the part or the wound is not
/// there, or the id names nothing.
///
/// A wound's age is its age at the timeline's present moment, which is
/// why the timeline is needed. A host that cannot lend the timeline,
/// because something else is changing it, uses [`property_at`].
pub fn property(
    bodies: &BodyStore,
    defs: &BodyDefs,
    timeline: &Timeline,
    property: PropertyId,
    subject: Handle,
    args: &[Value],
) -> Option<Value> {
    property_at(
        bodies,
        defs,
        &|clock| timeline.local_now(clock).ok(),
        property,
        subject,
        args,
    )
}

/// As [`property`], with the present moment of a clock given by a
/// function in place of the timeline. Where it gives `None` a wound's
/// age is what it was when its body was last brought up to date.
pub fn property_at(
    bodies: &BodyStore,
    defs: &BodyDefs,
    now: &dyn Fn(ClockId) -> Option<Dur>,
    property: PropertyId,
    subject: Handle,
    args: &[Value],
) -> Option<Value> {
    let prop = defs.prop(property)?;
    Some(answer(bodies, defs, now, prop, subject, args))
}
