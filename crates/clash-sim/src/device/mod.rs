//! Devices (02 §5.5; 16 §4.1 to §4.3; package E12): an element that
//! needs state of its own becomes, for as long as it does, an entity
//! bound to it.
//!
//! A locker, a door or a lamp is a fixture or a block in the grid, and
//! costs no more than a desk while nobody uses it: a street of ten
//! thousand lockers is ten thousand cells and no entity. An element
//! whose definition names a `device` is **promoted** to a device entity
//! when it needs more than its sixteen bits: when its state leaves the
//! default, when something is put inside it, when a rule asks
//! ([`promote`], the verb `promote`). The device stands in the element's
//! cell as an occupant of the kind `device`, carries a [`Device`]
//! component that says which element it gives behaviour to, and holds
//! whatever is inside it: items stowed in its `holds`, and anyone hiding
//! in it enclosed (`Hold::Inside`, 06 §9.2), out of the world's
//! occupant index. When it is back in its default state with nothing
//! inside, no variables and no rules of its own, it is **demoted**: the
//! entity is removed and the element is a plain element again
//! ([`settle`]).
//!
//! The element stays the physical object. When it is destroyed, or
//! replaced by something else, the device's contents spill into the
//! cell (or where something dropped there comes to rest) and whoever
//! hid there is out in the world again; then the device goes. The
//! kernel calls [`on_event`] for every event it delivers, and this is
//! where the world's `element_changed` and `element_destroyed` are
//! answered.
//!
//! | Part | Is |
//! |------|----|
//! | [`DeviceDef`] | The `device` definition type: its name, what it holds, its tags, the interactions it offers |
//! | [`Device`] | The component: the definition, and the element it is bound to |
//! | [`device_at`], [`device_of`] | The device bound to an element, if it has been promoted |
//! | [`promote`], [`settle`], [`unbind`] | Making, dropping and spilling a device |
//! | [`holds_of`] | What a device holds, for `put_in` and `take_out` |
//! | [`properties`], [`verbs()`], [`event_types`], [`DEVICE_REASONS`] | What rules read, call, listen for and are refused with |
//!
//! # Not built here
//!
//! The rest of 16 §4 is package G2's: modes, ports, stores, config,
//! processes, slots, passive loads (`load`, `when_supplied`) and the
//! generic configuration screen. A device's definition holds only what
//! E12 needs, and G2 adds to it.

mod defs;

use clash_core::{EntityId, impl_state_hash};
use clash_data::{Def, DefTypes};
use clash_expr::{EventSpec, Registry, Type, Value};
use clash_world::{
    Cause, ElementState, Layer, Location, OccupantFilter, OccupantKind, OccupantKinds, WorldEvent,
    distinct_ids,
};
use serde::{Deserialize, Serialize};

use crate::Sim;
use crate::bus::{EventPayload, EventType, FieldSpec, FieldType};
use crate::component::{DefKey, Identity};
use crate::entity::{EntitySpec, Place, Presence};
use crate::event::{CAUSE_SUBJECTS, EventBody, SimEvent, Subject, Where};
use crate::item::HoldsDef;
use crate::reason::{Reason, ReasonInfo};
use crate::registry::PropertyDef;
use crate::verb::VerbDef;
use crate::world_verbs::{ArgRow, element_arg, spec};

pub use defs::{DeviceDef, DeviceId, check_element_devices};

/// Registers the `device` definition type and the check that every
/// element's `device` names one. Called from the crate's
/// `register_defs`.
pub fn register_defs(types: &mut DefTypes) {
    types
        .register::<DeviceDef>()
        .add_cross_check(check_element_devices);
}

/// Enters the `device` definition type into an expression registry if
/// it lacks it. Called when the engine's registry is built, before its
/// properties.
pub fn register_types(registry: &mut Registry) {
    if registry.def_type_id(DeviceDef::TYPE).is_none() {
        let _ = registry.add_def_type(DeviceDef::TYPE, DeviceDef::DOC);
    }
}

/// The element a device gives behaviour to (16 §4.1): a cell and one of
/// its layers.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub struct Bound {
    /// The element's cell, normalised.
    pub loc: Location,
    /// Which of the cell's elements.
    pub layer: Layer,
}

impl_state_hash!(Bound { loc, layer });

/// What makes an entity a device (16 §4.1): its definition and, for one
/// bound to an element, which.
///
/// Package G2 adds the mode, ports, stores, process, config and slots.
#[derive(Clone, PartialEq, Debug, Serialize, Deserialize)]
pub struct Device {
    def: DeviceId,
    bound: Option<Bound>,
}

impl_state_hash!(Device { def, bound });

impl Device {
    /// A device of a definition, bound to an element or standing alone.
    pub fn new(def: DeviceId, bound: Option<Bound>) -> Device {
        Device { def, bound }
    }

    /// Its definition.
    pub fn def(&self) -> DeviceId {
        self.def
    }

    /// The element it gives behaviour to, if it is bound to one.
    pub fn bound(&self) -> Option<Bound> {
        self.bound
    }

    /// A device of no particular definition: for the tests of the
    /// component store.
    #[cfg(test)]
    pub(crate) fn sample(seed: u32) -> Device {
        Device {
            def: DeviceId::new(u16::try_from(seed % 7).unwrap_or(0)),
            bound: None,
        }
    }
}

// ---- Reasons -----------------------------------------------------------------

/// The stable codes of the reasons devices refuse for. Each is listed in
/// [`DEVICE_REASONS`].
pub mod codes {
    /// The element's definition names no `device`. Values: `element`.
    pub const NONE: &str = "device.none";
    /// The `device` an element names is not a definition. Values:
    /// `device`.
    pub const UNKNOWN_DEF: &str = "device.unknown_def";
    /// There is no element there. Values: `layer`.
    pub const NO_ELEMENT: &str = "device.no_element";
    /// The definitions were loaded without the `device` type. Values:
    /// none.
    pub const NOT_REGISTERED: &str = "device.not_registered";
}

/// The reasons devices refuse for, for the generated reference.
pub const DEVICE_REASONS: &[ReasonInfo] = &[
    ReasonInfo {
        code: codes::NONE,
        doc: "An element was to be promoted whose definition names no `device`. Values: `element`, the element's state as data writes it.",
    },
    ReasonInfo {
        code: codes::UNKNOWN_DEF,
        doc: "The `device` an element's definition names is not a `device` definition. Values: `device`.",
    },
    ReasonInfo {
        code: codes::NO_ELEMENT,
        doc: "There is no element on that layer of the cell. Values: `layer`.",
    },
    ReasonInfo {
        code: codes::NOT_REGISTERED,
        doc: "The definitions were loaded without the `device` definition type. Values: none.",
    },
];

// ---- Finding devices -----------------------------------------------------------

/// The device entity bound to an element, if the element has been
/// promoted. Reads truth.
pub fn device_at(sim: &Sim, loc: Location, layer: Layer) -> Option<EntityId> {
    let loc = sim.cosmos.normalize(loc).ok()?;
    let filter = OccupantFilter::ANY.of_kinds(OccupantKinds::of(&[OccupantKind::Device]));
    let found = sim.cosmos.occupants_at(loc, &filter).ok()?;
    distinct_ids(&found).into_iter().find(|id| {
        sim.components
            .get::<Device>(*id)
            .and_then(Device::bound)
            .is_some_and(|bound| bound.loc == loc && bound.layer == layer)
    })
}

/// The device bound to an element, as a subject: [`device_at`].
pub fn device_of(sim: &Sim, subject: Subject) -> Option<EntityId> {
    match subject {
        Subject::Element(loc, layer) => device_at(sim, loc, layer),
        _ => None,
    }
}

/// The definition of a device entity.
pub fn def_of(sim: &Sim, device: EntityId) -> Option<&DeviceDef> {
    let id = sim.components.get::<Device>(device)?.def;
    sim.defs.try_registry::<DeviceDef>()?.try_get(id)
}

/// What a device holds (16 §4.2 `holds`, 06 §9.2): what `put_in` checks
/// a thing against when the container is a device. `None` for an entity
/// that is not a device, or a device that holds nothing.
pub fn holds_of(sim: &Sim, container: EntityId) -> Option<&HoldsDef> {
    def_of(sim, container)?.holds.as_ref()
}

/// The element of a cell on a layer, if the cell is loaded and holds
/// one.
fn element_state(sim: &Sim, loc: Location, layer: Layer) -> Option<ElementState> {
    let state = sim
        .cosmos
        .grid(loc.grid)
        .ok()?
        .element(loc.cell, layer)
        .ok()?;
    (!state.is_none()).then_some(state)
}

/// The namespace of a qualified id: `core` of `core:locker`.
fn namespace_of(qualified: &str) -> Option<&str> {
    qualified.split_once(':').map(|(namespace, _)| namespace)
}

/// The `device` definition an element names, resolved as a reference
/// written in the element's own namespace. `Ok(None)` for an element
/// that names none.
pub fn element_device(
    sim: &Sim,
    layer: Layer,
    state: ElementState,
) -> Result<Option<DeviceId>, Reason> {
    let world = sim.cosmos.defs();
    let Some(element) = world.element(layer, state) else {
        return Ok(None);
    };
    let Some(written) = element.device.as_deref() else {
        return Ok(None);
    };
    let registry = sim.defs.try_registry::<DeviceDef>().ok_or_else(|| {
        Reason::new(
            codes::NOT_REGISTERED,
            "the definitions were loaded without the `device` type",
        )
    })?;
    let own = world.state_text(layer, state);
    let qualified = match (written.contains(':'), namespace_of(&own)) {
        (false, Some(namespace)) => format!("{namespace}:{written}"),
        _ => written.to_owned(),
    };
    registry
        .lookup(&qualified)
        .or_else(|| registry.lookup(written))
        .map(Some)
        .ok_or_else(|| {
            Reason::new(
                codes::UNKNOWN_DEF,
                format!("there is no device definition `{written}`"),
            )
            .with("device", written)
        })
}

// ---- Promotion and demotion ----------------------------------------------------

/// Promotes an element to its device (02 §5.5): makes a device entity
/// in the element's cell, bound to it, and raises `device_promoted`.
/// Returns the device; one already promoted is returned as it is.
///
/// Refused for a cell that is not loaded, a layer with no element
/// (`device.no_element`), an element whose definition names no device
/// (`device.none`) or one that does not exist (`device.unknown_def`).
pub fn promote(
    sim: &mut Sim,
    loc: Location,
    layer: Layer,
    cause: Cause,
) -> Result<EntityId, Reason> {
    let loc = sim
        .cosmos
        .normalize(loc)
        .map_err(|error| Reason::from_world(&error))?;
    if let Some(device) = device_at(sim, loc, layer) {
        return Ok(device);
    }
    let state = sim
        .cosmos
        .grid(loc.grid)
        .and_then(|grid| grid.element(loc.cell, layer))
        .map_err(|error| Reason::from_world(&error))?;
    if state.is_none() {
        return Err(Reason::new(
            codes::NO_ELEMENT,
            format!("there is no {} there", layer.name()),
        )
        .with("layer", layer.name()));
    }
    let Some(def) = element_device(sim, layer, state)? else {
        let element = sim.cosmos.defs().state_text(layer, state);
        return Err(Reason::new(
            codes::NONE,
            format!("{element} has no device to give it state of its own"),
        )
        .with("element", element));
    };
    let mut spec =
        EntitySpec::new(OccupantKind::Device).with_presence(Presence::new(OccupantKind::Device));
    if let Some(device_def) = sim
        .defs
        .try_registry::<DeviceDef>()
        .and_then(|registry| registry.try_get(def))
    {
        for tag in &device_def.tags {
            spec = spec.with_tag(tag.clone());
        }
    }
    let id = sim.spawn(spec, Place::at(loc), cause)?;
    let key = sim
        .exprs
        .registry()
        .def_type_id(DeviceDef::TYPE)
        .map(|ty| DefKey::new(ty, u32::from(def.raw())));
    sim.components.insert(id, Identity::new(key));
    sim.components
        .insert(id, Device::new(def, Some(Bound { loc, layer })));
    raise(
        sim,
        id,
        EventPayload::new(DEVICE_PROMOTED)
            .subject("self", Subject::Entity(id))
            .subject("element", Subject::Element(loc, layer)),
        cause,
    );
    Ok(id)
}

/// Whether a device bound to an element has nothing of its own left:
/// its element is in its default state (the first its definition
/// declares), nothing is inside it, and it carries no variables, no
/// rules attached to it and no tags beyond its definition's.
pub fn is_idle(sim: &Sim, device: EntityId) -> bool {
    let (Some(component), Some(entity)) = (
        sim.components.get::<Device>(device),
        sim.entities.get(device),
    ) else {
        return false;
    };
    let Some(bound) = component.bound else {
        return false;
    };
    let default_state =
        element_state(sim, bound.loc, bound.layer).is_some_and(|state| state.bits == 0);
    let own_tags = def_of(sim, device).map_or(0, |def| def.tags.len());
    default_state
        && sim.entities.contents(device).is_empty()
        && entity.vars.is_empty()
        && entity.attached.is_empty()
        && entity.tags.len() <= own_tags
}

/// Demotes a device that is idle ([`is_idle`]): raises
/// `device_demoted` and removes the entity, leaving the element plain.
/// Returns whether it was demoted.
pub fn settle(sim: &mut Sim, device: EntityId, cause: Cause) -> bool {
    if !is_idle(sim, device) {
        return false;
    }
    let Some(bound) = sim.components.get::<Device>(device).and_then(Device::bound) else {
        return false;
    };
    raise(
        sim,
        device,
        EventPayload::new(DEVICE_DEMOTED)
            .subject("self", Subject::Entity(device))
            .subject("element", Subject::Element(bound.loc, bound.layer)),
        cause,
    );
    sim.remove_entity(device, cause).is_ok()
}

/// What a device does when the element it is bound to has gone,
/// destroyed or replaced (02 §5.5): everything inside it spills where
/// something dropped into the cell comes to rest, each with
/// `device_spilled`, so that whoever hid there is out in the world
/// again; then the device is demoted whatever it holds. Returns what
/// spilled, in id order.
pub fn unbind(sim: &mut Sim, device: EntityId, cause: Cause) -> Vec<EntityId> {
    let Some(bound) = sim.components.get::<Device>(device).and_then(Device::bound) else {
        return Vec::new();
    };
    let rest = sim.cosmos.resting_cell(bound.loc).unwrap_or(bound.loc);
    let mut spilled = Vec::new();
    for inside in sim.entities.contents(device).to_vec() {
        let out = sim
            .relocate(inside, Place::at(rest), cause)
            .or_else(|_| sim.relocate(inside, Place::at(bound.loc), cause));
        if out.is_ok() {
            spilled.push(inside);
            raise(
                sim,
                inside,
                EventPayload::new(DEVICE_SPILLED)
                    .subject("self", Subject::Entity(inside))
                    .subject("device", Subject::Entity(device))
                    .subject("here", Subject::Loc(rest))
                    .with("layer", bound.layer.name()),
                cause,
            );
        }
    }
    raise(
        sim,
        device,
        EventPayload::new(DEVICE_DEMOTED)
            .subject("self", Subject::Entity(device))
            .subject("element", Subject::Element(bound.loc, bound.layer)),
        cause,
    );
    let _ = sim.remove_entity(device, cause);
    spilled
}

/// What devices do about an event, once its handlers have run: the
/// kernel calls this for every event it delivers.
///
/// - `element_changed`: what is in the cell now decides, since the
///   event may be delivered after later changes. An element that names
///   a device and is out of its default state is promoted; a device
///   whose element is still the one it was made for settles; a device
///   whose element has gone, or names another device now, is unbound
///   ([`unbind`]).
/// - `element_destroyed`: the device of the element destroyed is
///   unbound: its contents spill and whoever hid there is exposed.
/// - `entity_relocated` out of a device: the device settles.
pub fn on_event(sim: &mut Sim, event: &SimEvent) {
    match &event.body {
        EventBody::World(WorldEvent::ElementChanged { loc, layer, .. }) => {
            // The event may be delivered after later changes: what is
            // there now decides, not what the event says changed.
            let now = element_state(sim, *loc, *layer);
            let names = now.and_then(|state| element_device(sim, *layer, state).ok().flatten());
            match device_at(sim, *loc, *layer) {
                Some(device) => {
                    let def = sim.components.get::<Device>(device).map(Device::def);
                    if names.is_none() || names != def {
                        unbind(sim, device, event.cause);
                    } else {
                        settle(sim, device, event.cause);
                    }
                }
                None => {
                    if names.is_some() && now.is_some_and(|state| state.bits != 0) {
                        let _ = promote(sim, *loc, *layer, event.cause);
                    }
                }
            }
        }
        EventBody::World(WorldEvent::ElementDestroyed { loc, layer, .. }) => {
            if let Some(device) = device_at(sim, *loc, *layer) {
                unbind(sim, device, event.cause);
            }
        }
        EventBody::EntityRelocated {
            from: Place::In(container),
            ..
        } if sim.components.get::<Device>(*container).is_some() => {
            settle(sim, *container, event.cause);
        }
        _ => {}
    }
}

// ---- Events --------------------------------------------------------------------

/// An element became a device entity.
pub const DEVICE_PROMOTED: &str = "device_promoted";
/// A device entity was dropped and its element is plain again.
pub const DEVICE_DEMOTED: &str = "device_demoted";
/// Something came out of a device whose element was destroyed or
/// replaced.
pub const DEVICE_SPILLED: &str = "device_spilled";

/// One event of this module, as both registries are told of it.
struct EventRow {
    name: &'static str,
    doc: &'static str,
    subjects: &'static [(&'static str, &'static str)],
    fields: &'static [(&'static str, &'static str, &'static str)],
}

const EVENTS: &[EventRow] = &[
    EventRow {
        name: DEVICE_PROMOTED,
        doc: "An element became a device entity bound to it (02 §5.5): its state left the default, something was put inside it, or a rule asked. `self` is the device, `element` what it gives behaviour to.",
        subjects: &[("self", "entity"), ("element", "element")],
        fields: &[],
    },
    EventRow {
        name: DEVICE_DEMOTED,
        doc: "A device entity was dropped: its element is back in its default state with nothing inside it, or the element is gone. When it is delivered the device no longer exists; `element` is what it was bound to.",
        subjects: &[("self", "entity"), ("element", "element")],
        fields: &[],
    },
    EventRow {
        name: DEVICE_SPILLED,
        doc: "Something came out of a device whose element was destroyed or replaced (02 §5.5): an item lies in the cell, someone who hid there stands in it. `here` is where it came to rest.",
        subjects: &[("self", "entity"), ("device", "entity"), ("here", "loc")],
        fields: &[(
            "layer",
            "string",
            "Which of the cell's elements the device was bound to: `floor`, `block` or `fixture`.",
        )],
    },
];

/// Raises a notification of this module.
fn raise(sim: &mut Sim, about: EntityId, payload: EventPayload, cause: Cause) {
    // The payload is built to the registered type: a refusal means the
    // type is not registered with this simulation's bus.
    let _ = sim.raise(Where::Entity(about), cause, payload);
}

/// The event types of this module, for the simulation's bus (05 §8.1).
pub fn event_types() -> Vec<EventType> {
    EVENTS
        .iter()
        .map(|row| {
            let mut ty = EventType::new(row.name).doc(row.doc);
            for (name, kind) in row.subjects {
                ty = ty.subject(*name, *kind);
            }
            ty.fields = row
                .fields
                .iter()
                .map(|(name, written, doc)| FieldSpec {
                    name: (*name).to_owned(),
                    ty: (*written).to_owned(),
                    check: Some(FieldType::parse(written).unwrap_or(FieldType::Any)),
                    doc: (*doc).to_owned(),
                })
                .collect();
            ty
        })
        .collect()
}

/// Enters this module's events into an expression registry, leaving
/// alone those it already holds.
pub fn register_events(registry: &mut Registry) {
    for row in EVENTS {
        if registry.event_id(row.name).is_some() {
            continue;
        }
        let mut spec = EventSpec::new(row.name).doc(row.doc);
        for (name, kind) in row.subjects.iter().chain(CAUSE_SUBJECTS) {
            if let Some(kind) = registry.kind_id(kind) {
                spec = spec.subject(name.trim_end_matches('?'), kind);
            }
        }
        for (name, written, doc) in row.fields {
            let ty = registry.parse_type(written).unwrap_or(Type::Any);
            spec = spec.field(*name, ty, *doc);
        }
        let _ = registry.add_event(spec);
    }
}

// ---- Properties and verbs ------------------------------------------------------

/// The properties of devices (16 §3.5, §14): `element.device` and
/// `entity.element`. Both read truth and neither is safe for belief.
pub fn properties() -> Vec<PropertyDef> {
    vec![
        PropertyDef::new(
            "element",
            "device",
            "entity?",
            "The device entity bound to the element, if it has been promoted (02 §5.5). Null for an element that is plain now, whether or not its definition names a device.",
            |sim, subject, _| {
                device_of(sim, subject).map_or(Value::Null, |id| sim.subject_value(Subject::Entity(id)))
            },
        )
        .changed_by(&[DEVICE_PROMOTED, DEVICE_DEMOTED]),
        PropertyDef::new(
            "entity",
            "element",
            "element?",
            "For a device bound to an element, the element: what it gives behaviour to (16 §4.1). Null for anything else.",
            |sim, subject, _| match subject {
                Subject::Entity(id) => sim
                    .components
                    .get::<Device>(id)
                    .and_then(Device::bound)
                    .map_or(Value::Null, |bound| {
                        sim.subject_value(Subject::Element(bound.loc, bound.layer))
                    }),
                _ => Value::Null,
            },
        ),
        PropertyDef::new(
            "entity",
            "is_device",
            "bool",
            "Whether the entity is a device (16 §4.1).",
            |sim, subject, _| {
                Value::Bool(matches!(subject, Subject::Entity(id) if sim.components.get::<Device>(id).is_some()))
            },
        ),
    ]
}

const OF_ELEMENT: ArgRow = ("of", "element", false, "The element.");

/// The verbs of devices, for [`crate::verb::engine_verbs`].
pub fn verbs() -> Vec<VerbDef> {
    vec![VerbDef {
        name: "promote",
        build: |registry| {
            spec(
                registry,
                "promote",
                "Promotes an element to its device entity (02 §5.5) and returns it: the one there already, or a new one bound to the element, with `device_promoted`. Afterwards `target.device` names it, so that something can be put inside it. A device left in its default state with nothing inside is dropped again when something next leaves it or its state is next set, or when the interaction that promoted it ends. Refused for an element whose definition names no device (`device.none`), no element (`device.no_element`), and a cell not loaded.",
                &[OF_ELEMENT],
            )
            .map(|spec| spec.direct("of"))
        },
        run: |sim, call, cause| {
            let (loc, layer) = element_arg(sim, "promote", call, 0, "of")?;
            let id = promote(sim, loc, layer, cause)?;
            Ok(sim.subject_value(Subject::Entity(id)))
        },
    }]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_reason_is_listed_once() {
        let mut codes: Vec<&str> = DEVICE_REASONS.iter().map(|info| info.code).collect();
        let count = codes.len();
        codes.sort_unstable();
        codes.dedup();
        assert_eq!(codes.len(), count);
    }

    #[test]
    fn every_event_registers_in_both_registries() {
        let registry = crate::engine_registry();
        for ty in event_types() {
            assert!(registry.event_id(&ty.name).is_some(), "{}", ty.name);
            assert!(!ty.doc.is_empty());
        }
    }
}
