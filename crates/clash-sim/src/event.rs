//! [`SimEvent`]: what the simulation reports having happened.
//!
//! Every state change of interest produces an event (01 §8). Events go
//! first to the handlers that listen for them (05 §8) and then to the
//! output that presentation drains with
//! [`Sim::drain_events`](crate::Sim::drain_events).
//!
//! Every event carries the universal tick, the clock it happened on and
//! that clock's local time, its cause, and its subjects (03 §12).
//!
//! # Adding an event
//!
//! Add a variant to [`EventBody`], give it a name in
//! [`EventBody::name`], its subjects in [`EventBody::subjects`], its
//! fields in [`EventBody::field`], its hashing in the `StateHash`
//! implementation and an entry in [`KERNEL_EVENTS`]. A test fails if a
//! name is missing from the table.

use clash_core::{Dur, EntityId, Real, StateHash, StateHasher, Tick, Value};
use clash_time::ClockId;
use clash_world::{AreaId, Cause, Location, WorldEvent};
use serde::{Deserialize, Serialize};

use crate::bus::{EventPayload, FieldValue};
use crate::decision::DecisionKind;
use crate::entity::{Control, Place};
use crate::reason::Reason;

/// Something an event is about (05 §5.1).
///
/// The kernel names the kinds it can refer to itself. Package K4 turns
/// these into the expression language's handles and adds the rest.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum Subject {
    /// An entity.
    Entity(EntityId),
    /// A cell.
    Loc(Location),
    /// An area.
    Area(AreaId),
}

impl Subject {
    /// The name of the subject's kind as rules write it: `entity`, `loc`,
    /// `area`.
    pub const fn kind(&self) -> &'static str {
        match self {
            Subject::Entity(_) => "entity",
            Subject::Loc(_) => "loc",
            Subject::Area(_) => "area",
        }
    }

    /// The entity, if the subject is one.
    pub const fn entity(&self) -> Option<EntityId> {
        match self {
            Subject::Entity(id) => Some(*id),
            _ => None,
        }
    }
}

impl StateHash for Subject {
    fn state_hash(&self, hasher: &mut StateHasher) {
        match self {
            Subject::Entity(id) => {
                hasher.write_u8(0);
                id.state_hash(hasher);
            }
            Subject::Loc(loc) => {
                hasher.write_u8(1);
                loc.state_hash(hasher);
            }
            Subject::Area(area) => {
                hasher.write_u8(2);
                area.state_hash(hasher);
            }
        }
    }
}

/// Where an event happened, which decides the clock whose local time it
/// carries.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Where {
    /// To an entity: the entity's clock.
    Entity(EntityId),
    /// At a cell: the clock of that location.
    Loc(Location),
    /// On a clock.
    Clock(ClockId),
    /// Nowhere in particular: universal time.
    Global,
}

/// Something that happened in the simulation.
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub struct SimEvent {
    /// When, in universal time.
    pub tick: Tick,
    /// The clock it happened on.
    pub clock: ClockId,
    /// That clock's local time when it happened.
    pub local: Dur,
    /// Who and what is behind it. Truth: what witnesses believe happened
    /// is a separate matter (01 R11).
    pub cause: Cause,
    /// What happened.
    pub body: EventBody,
}

impl SimEvent {
    /// The name of the event as rules write it: `entity_relocated`.
    pub fn name(&self) -> &str {
        self.body.name()
    }

    /// The event's subjects by name, in the order handlers are found by
    /// (05 §8.3): those of what happened, then `actor` and `instigator`
    /// from the cause where it names them.
    pub fn subjects(&self) -> Vec<(&str, Subject)> {
        let mut subjects = self.body.subjects();
        if let Some(actor) = self.cause.actor {
            subjects.push(("actor", Subject::Entity(actor)));
        }
        if let Some(instigator) = self.cause.instigator {
            subjects.push(("instigator", Subject::Entity(instigator)));
        }
        subjects
    }

    /// One of the event's subjects, by name.
    pub fn subject(&self, name: &str) -> Option<Subject> {
        self.subjects()
            .into_iter()
            .find(|(have, _)| *have == name)
            .map(|(_, subject)| subject)
    }
}

impl StateHash for SimEvent {
    fn state_hash(&self, hasher: &mut StateHasher) {
        self.tick.state_hash(hasher);
        self.clock.state_hash(hasher);
        self.local.state_hash(hasher);
        self.cause.state_hash(hasher);
        self.body.state_hash(hasher);
    }
}

/// What happened. Later packages add variants for what they build: match
/// with a wildcard arm.
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum EventBody {
    /// Something happened in the world (02 §7.4). The world's event is
    /// passed on as it was reported.
    World(WorldEvent),
    /// An entity came to exist. It is not yet anywhere.
    EntityCreated {
        /// The new entity.
        entity: EntityId,
    },
    /// An entity ceased to exist. Its id will never name another.
    EntityRemoved {
        /// The entity that is gone.
        entity: EntityId,
    },
    /// An entity's place changed: it moved, was put inside something or
    /// taken out, entered the world or left it.
    EntityRelocated {
        /// Who.
        entity: EntityId,
        /// Where it was.
        from: Place,
        /// Where it is.
        to: Place,
    },
    /// Who decides for an entity changed.
    ControlChanged {
        /// Whose.
        entity: EntityId,
        /// The mode before.
        old: Control,
        /// The mode after.
        new: Control,
    },
    /// A clock's rate changed, or a clock was given or taken away.
    ClockRateChanged {
        /// The clock. For a clock that was taken away, the one it was.
        clock: ClockId,
        /// The entity whose personal clock it is, if it is one.
        entity: Option<EntityId>,
        /// The area whose clock it is, if it is one.
        area: Option<AreaId>,
        /// Its rate relative to its parent before. `None`: there was no
        /// such clock.
        old: Option<Real>,
        /// Its rate after. `None`: the clock is gone.
        new: Option<Real>,
    },
    /// A directly controlled entity must choose (03 §5.2).
    DecisionRequested {
        /// Who must choose.
        entity: EntityId,
        /// What is being asked.
        kind: DecisionKind,
    },
    /// The engine contained a fault: a limit was exceeded, or something
    /// was scheduled that nothing handles. The simulation carries on.
    Fault {
        /// What went wrong.
        reason: Reason,
    },
    /// A hook was cancelled: what it preceded did not happen (05 §8.2).
    EventCancelled {
        /// The name of the hook.
        event: String,
        /// Why, as the handler that cancelled it said.
        reason: Reason,
    },
    /// An event of a type registered at run time: one a mod defined with
    /// an `event` definition, or one of a later package (05 §8.1).
    Custom(EventPayload),
}

impl EventBody {
    /// The name of the event as rules write it.
    pub fn name(&self) -> &str {
        match self {
            EventBody::World(event) => event.name(),
            EventBody::EntityCreated { .. } => "entity_created",
            EventBody::EntityRemoved { .. } => "entity_removed",
            EventBody::EntityRelocated { .. } => "entity_relocated",
            EventBody::ControlChanged { .. } => "control_changed",
            EventBody::ClockRateChanged { .. } => "clock_rate_changed",
            EventBody::DecisionRequested { .. } => "decision_requested",
            EventBody::Fault { .. } => "fault",
            EventBody::EventCancelled { .. } => "event_cancelled",
            EventBody::Custom(payload) => &payload.name,
        }
    }

    /// The subjects of what happened, by name, in declared order.
    pub fn subjects(&self) -> Vec<(&str, Subject)> {
        match self {
            EventBody::World(event) => world_subjects(event),
            EventBody::EntityCreated { entity }
            | EventBody::EntityRemoved { entity }
            | EventBody::ControlChanged { entity, .. }
            | EventBody::DecisionRequested { entity, .. } => {
                vec![("self", Subject::Entity(*entity))]
            }
            EventBody::EntityRelocated { entity, to, .. } => {
                let mut subjects = vec![("self", Subject::Entity(*entity))];
                if let Some(loc) = to.loc() {
                    subjects.push(("here", Subject::Loc(loc)));
                }
                subjects
            }
            EventBody::ClockRateChanged { entity, area, .. } => {
                let mut subjects = Vec::new();
                if let Some(entity) = entity {
                    subjects.push(("self", Subject::Entity(*entity)));
                }
                if let Some(area) = area {
                    subjects.push(("area", Subject::Area(*area)));
                }
                subjects
            }
            EventBody::Fault { .. } | EventBody::EventCancelled { .. } => Vec::new(),
            EventBody::Custom(payload) => payload
                .subjects
                .iter()
                .map(|(name, subject)| (name.as_str(), *subject))
                .collect(),
        }
    }

    /// One of the event's discrete fields, by name: what a rule's
    /// `filter` tests (05 §8.4). `None` for a field the event does not
    /// have, or one that is not a plain value.
    pub fn field(&self, name: &str) -> Option<FieldValue> {
        match self {
            EventBody::World(event) => world_field(event, name),
            EventBody::ControlChanged { old, new, .. } => match name {
                "old" => Some(FieldValue::text(control_name(*old))),
                "new" => Some(FieldValue::text(control_name(*new))),
                _ => None,
            },
            EventBody::ClockRateChanged {
                clock, old, new, ..
            } => match name {
                "clock" => Some(FieldValue::from(i64::from(clock.raw()))),
                "old" => old.map(FieldValue::from),
                "new" => new.map(FieldValue::from),
                _ => None,
            },
            EventBody::Fault { reason } => match name {
                "code" => Some(FieldValue::text(&reason.code)),
                "message" => Some(FieldValue::text(&reason.message)),
                _ => None,
            },
            EventBody::EventCancelled { event, reason } => match name {
                "event" => Some(FieldValue::text(event)),
                "code" => Some(FieldValue::text(&reason.code)),
                "message" => Some(FieldValue::text(&reason.message)),
                _ => None,
            },
            EventBody::Custom(payload) => payload.field(name).cloned(),
            EventBody::EntityCreated { .. }
            | EventBody::EntityRemoved { .. }
            | EventBody::EntityRelocated { .. }
            | EventBody::DecisionRequested { .. } => None,
        }
    }
}

/// The name of a control mode as data writes it.
const fn control_name(control: Control) -> &'static str {
    match control {
        Control::None => "none",
        Control::Direct => "direct",
        Control::Delegated => "delegated",
    }
}

/// The subjects of a world event. The occupant of an area or occupant
/// event is `target`, as rules attached to an area kind expect
/// (05 §9.3); the cell is `here`.
fn world_subjects(event: &WorldEvent) -> Vec<(&'static str, Subject)> {
    match event {
        WorldEvent::AreaEntered {
            area,
            occupant,
            loc,
            ..
        }
        | WorldEvent::AreaExited {
            area,
            occupant,
            loc,
            ..
        } => vec![
            ("area", Subject::Area(*area)),
            ("target", Subject::Entity(*occupant)),
            ("here", Subject::Loc(*loc)),
        ],
        WorldEvent::AreaBreached { area, leak, .. } => vec![
            ("area", Subject::Area(*area)),
            ("here", Subject::Loc(*leak)),
        ],
        WorldEvent::AreaClosed { area, loc, .. } => {
            vec![("area", Subject::Area(*area)), ("here", Subject::Loc(*loc))]
        }
        WorldEvent::OccupantDisplaced { occupant, to, .. } => vec![
            ("target", Subject::Entity(*occupant)),
            ("here", Subject::Loc(*to)),
        ],
        WorldEvent::OccupantEntombed { occupant, loc, .. } => vec![
            ("target", Subject::Entity(*occupant)),
            ("here", Subject::Loc(*loc)),
        ],
        other => vec![("here", Subject::Loc(other.location()))],
    }
}

/// The discrete fields of a world event that a filter can test.
fn world_field(event: &WorldEvent, name: &str) -> Option<FieldValue> {
    match (event, name) {
        (
            WorldEvent::ElementChanged { layer, .. } | WorldEvent::ElementDamaged { layer, .. },
            "layer",
        ) => Some(FieldValue::text(match layer {
            clash_world::Layer::Floor => "floor",
            clash_world::Layer::Block => "block",
            clash_world::Layer::Fixture => "fixture",
        })),
        (WorldEvent::ElementDamaged { old, .. }, "old") => Some(FieldValue::from(i64::from(*old))),
        (WorldEvent::ElementDamaged { new, .. }, "new") => Some(FieldValue::from(i64::from(*new))),
        (WorldEvent::DecalsRemoved { count, .. }, "count") => {
            Some(FieldValue::from(i64::from(*count)))
        }
        (WorldEvent::CellVarChanged { key, .. }, "key") => Some(FieldValue::text(key.as_str())),
        (WorldEvent::CellVarChanged { old, .. }, "old") => old.clone().map(plain_field),
        (WorldEvent::CellVarChanged { new, .. }, "new") => new.clone().map(plain_field),
        _ => None,
    }
}

/// A stored value as an event field.
fn plain_field(value: Value) -> FieldValue {
    FieldValue::Plain(value)
}

/// Hashes the event's name and then its fields in the order they are
/// declared, so that two runs can compare what they reported.
impl StateHash for EventBody {
    fn state_hash(&self, hasher: &mut StateHasher) {
        match self {
            // The world's event hashes its own name first.
            EventBody::World(event) => event.state_hash(hasher),
            EventBody::EntityCreated { entity } | EventBody::EntityRemoved { entity } => {
                hasher.write_str(self.name());
                entity.state_hash(hasher);
            }
            EventBody::EntityRelocated { entity, from, to } => {
                hasher.write_str(self.name());
                entity.state_hash(hasher);
                from.state_hash(hasher);
                to.state_hash(hasher);
            }
            EventBody::ControlChanged { entity, old, new } => {
                hasher.write_str(self.name());
                entity.state_hash(hasher);
                old.state_hash(hasher);
                new.state_hash(hasher);
            }
            EventBody::ClockRateChanged {
                clock,
                entity,
                area,
                old,
                new,
            } => {
                hasher.write_str(self.name());
                clock.state_hash(hasher);
                entity.state_hash(hasher);
                area.state_hash(hasher);
                old.state_hash(hasher);
                new.state_hash(hasher);
            }
            EventBody::DecisionRequested { entity, kind } => {
                hasher.write_str(self.name());
                entity.state_hash(hasher);
                kind.state_hash(hasher);
            }
            EventBody::Fault { reason } => {
                hasher.write_str(self.name());
                reason.state_hash(hasher);
            }
            EventBody::EventCancelled { event, reason } => {
                hasher.write_str(self.name());
                hasher.write_str(event);
                reason.state_hash(hasher);
            }
            EventBody::Custom(payload) => payload.state_hash(hasher),
        }
    }
}

/// One entry of a table of events: what the generated reference says
/// about an event type (05 §8.1).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct EventInfo {
    /// The name as rules write it.
    pub name: &'static str,
    /// The subjects it binds, in delivery order: name and kind. A name
    /// ending in `?` is bound only when there is one.
    pub subjects: &'static [(&'static str, &'static str)],
    /// Its fields: name, type and what it holds.
    pub fields: &'static [(&'static str, &'static str, &'static str)],
    /// What happened.
    pub doc: &'static str,
}

/// The subjects every event may also bind, from its cause.
pub const CAUSE_SUBJECTS: &[(&str, &str)] = &[("actor?", "entity"), ("instigator?", "entity")];

/// The events the kernel emits, for the generated reference. Every event
/// also binds [`CAUSE_SUBJECTS`] and carries the universal tick, its
/// clock, that clock's local time and its cause.
pub const KERNEL_EVENTS: &[EventInfo] = &[
    EventInfo {
        name: "entity_created",
        subjects: &[("self", "entity")],
        fields: &[],
        doc: "An entity came to exist. It is not yet anywhere: `entity_relocated` follows when it is put somewhere.",
    },
    EventInfo {
        name: "entity_removed",
        subjects: &[("self", "entity")],
        fields: &[],
        doc: "An entity ceased to exist. What it held was put where it was first.",
    },
    EventInfo {
        name: "entity_relocated",
        subjects: &[("self", "entity"), ("here?", "loc")],
        fields: &[
            (
                "from",
                "place",
                "Where it was: a cell and a height, a container, or not in the world.",
            ),
            (
                "to",
                "place",
                "Where it is. `here` is bound when that is a cell.",
            ),
        ],
        doc: "An entity's place changed: it moved, was put inside something or taken out, entered the world or left it. What it holds moves with it and is not reported.",
    },
    EventInfo {
        name: "control_changed",
        subjects: &[("self", "entity")],
        fields: &[
            (
                "old",
                "string",
                "The mode before: `none`, `direct` or `delegated`.",
            ),
            ("new", "string", "The mode after."),
        ],
        doc: "Who decides for an entity changed (03 §5.1).",
    },
    EventInfo {
        name: "clock_rate_changed",
        subjects: &[("self?", "entity"), ("area?", "area")],
        fields: &[
            ("clock", "int", "The clock's id."),
            (
                "old",
                "real?",
                "Its rate relative to its parent before; null if there was no such clock.",
            ),
            ("new", "real?", "Its rate after; null if the clock is gone."),
        ],
        doc: "The rate of an entity's personal clock, an area's clock or a realm's clock changed, or a clock was given or taken away (03 §3.4).",
    },
    EventInfo {
        name: "decision_requested",
        subjects: &[("self", "entity")],
        fields: &[("kind", "decision", "What is being asked (03 §5.2).")],
        doc: "A directly controlled entity must choose. Time does not move until it has.",
    },
    EventInfo {
        name: "fault",
        subjects: &[],
        fields: &[
            ("code", "string", "The stable code of the reason."),
            ("message", "string", "What went wrong, in a sentence."),
        ],
        doc: "The engine contained a fault: a limit was exceeded, or something was scheduled that nothing handles. Not delivered to handlers; it goes straight to the output.",
    },
    EventInfo {
        name: "event_cancelled",
        subjects: &[],
        fields: &[
            (
                "event",
                "string",
                "The name of the hook that was cancelled.",
            ),
            ("code", "string", "The stable code of the reason."),
            (
                "message",
                "string",
                "Why, as the handler that cancelled it said.",
            ),
        ],
        doc: "A hook was cancelled: what it preceded did not happen. The hook's own subjects are not bound; its cause is carried.",
    },
];

/// The world's events as the kernel passes them on (02 §7.4): the names
/// the subjects are bound under and the fields a filter can test.
pub const WORLD_EVENTS: &[EventInfo] = &[
    EventInfo {
        name: "element_changed",
        subjects: &[("here", "loc")],
        fields: &[("layer", "string", "`floor`, `block` or `fixture`.")],
        doc: "A floor, block or fixture was set, replaced, transmuted or removed.",
    },
    EventInfo {
        name: "element_damaged",
        subjects: &[("here", "loc")],
        fields: &[
            ("layer", "string", "`floor`, `block` or `fixture`."),
            (
                "old",
                "int",
                "Damage before, in 255ths of the element's capacity.",
            ),
            ("new", "int", "Damage after."),
        ],
        doc: "The accumulated damage of an element changed.",
    },
    EventInfo {
        name: "decal_added",
        subjects: &[("here", "loc")],
        fields: &[],
        doc: "A mark was left in a cell.",
    },
    EventInfo {
        name: "decals_removed",
        subjects: &[("here", "loc")],
        fields: &[("count", "int", "How many marks there were.")],
        doc: "Every mark of one kind was removed from a cell.",
    },
    EventInfo {
        name: "cell_var_changed",
        subjects: &[("here", "loc")],
        fields: &[
            ("key", "string", "The variable's qualified name."),
            ("old", "any", "Its value before; null if it had none."),
            ("new", "any", "Its value after; null if it has none."),
        ],
        doc: "A variable on a cell was set, changed or cleared.",
    },
    EventInfo {
        name: "area_entered",
        subjects: &[("area", "area"), ("target", "entity"), ("here", "loc")],
        fields: &[],
        doc: "An entity came to be inside an area. Its wake-ups have moved to the clock that now applies.",
    },
    EventInfo {
        name: "area_exited",
        subjects: &[("area", "area"), ("target", "entity"), ("here", "loc")],
        fields: &[],
        doc: "An entity stopped being inside an area.",
    },
    EventInfo {
        name: "area_breached",
        subjects: &[("area", "area"), ("here", "loc")],
        fields: &[],
        doc: "An enclosed area no longer closes; `here` is a cell it leaks through.",
    },
    EventInfo {
        name: "area_closed",
        subjects: &[("area", "area"), ("here", "loc")],
        fields: &[],
        doc: "A breached area closes again.",
    },
    EventInfo {
        name: "occupant_displaced",
        subjects: &[("target", "entity"), ("here", "loc")],
        fields: &[],
        doc: "An edit left an entity no room and it was moved to the nearest cell with room. `entity_relocated` follows.",
    },
    EventInfo {
        name: "occupant_entombed",
        subjects: &[("target", "entity"), ("here", "loc")],
        fields: &[],
        doc: "An edit left an entity no room and there was nowhere near to go. It stays where it is.",
    },
];

#[cfg(test)]
mod tests {
    use super::*;
    use clash_world::{CellPos, GridId};

    fn listed(name: &str) -> Option<&'static EventInfo> {
        KERNEL_EVENTS
            .iter()
            .chain(WORLD_EVENTS)
            .find(|info| info.name == name)
    }

    #[test]
    fn every_kernel_event_is_in_the_table_with_its_subjects() {
        let entity = EntityId::new(1, 0);
        let loc = Location::new(GridId::new(0), CellPos::new(1, 2, 0));
        let bodies = [
            EventBody::EntityCreated { entity },
            EventBody::EntityRemoved { entity },
            EventBody::EntityRelocated {
                entity,
                from: Place::Stored,
                to: Place::at(loc),
            },
            EventBody::ControlChanged {
                entity,
                old: Control::None,
                new: Control::Direct,
            },
            EventBody::ClockRateChanged {
                clock: ClockId::new(3),
                entity: Some(entity),
                area: None,
                old: None,
                new: Some(Real::ONE),
            },
            EventBody::DecisionRequested {
                entity,
                kind: DecisionKind::Free {
                    why: crate::decision::FreeReason::Started,
                },
            },
            EventBody::Fault {
                reason: Reason::new("x", "y"),
            },
            EventBody::EventCancelled {
                event: "h".to_owned(),
                reason: Reason::new("x", "y"),
            },
        ];
        for body in bodies {
            let info = listed(body.name())
                .unwrap_or_else(|| panic!("`{}` is not in KERNEL_EVENTS", body.name()));
            assert!(!info.doc.is_empty());
            // Every plain field the table lists can be read by name.
            for (field, ty, _) in info.fields {
                if matches!(*ty, "string" | "int") {
                    assert!(
                        body.field(field).is_some(),
                        "`{}` lists `{field}` but it cannot be read",
                        info.name
                    );
                }
            }
            for (name, subject) in body.subjects() {
                let declared = info
                    .subjects
                    .iter()
                    .find(|(have, _)| have.trim_end_matches('?') == name)
                    .unwrap_or_else(|| panic!("`{}` binds undeclared `{name}`", info.name));
                assert_eq!(declared.1, subject.kind());
            }
        }
    }

    #[test]
    fn world_events_bind_the_occupant_as_target() {
        let event = WorldEvent::AreaEntered {
            tick: Tick(5),
            area: AreaId::new(2),
            occupant: EntityId::new(4, 0),
            loc: Location::new(GridId::new(0), CellPos::new(0, 0, 0)),
            clock: None,
            cause: Cause::DEBUG.by(EntityId::new(9, 0)),
        };
        let event = SimEvent {
            tick: Tick(5),
            clock: ClockId::UNIVERSAL,
            local: Dur(5),
            cause: event.cause(),
            body: EventBody::World(event),
        };
        assert_eq!(event.name(), "area_entered");
        assert!(listed(event.name()).is_some());
        let names: Vec<&str> = event.subjects().iter().map(|(name, _)| *name).collect();
        assert_eq!(names, ["area", "target", "here", "actor", "instigator"]);
        assert_eq!(
            event.subject("target"),
            Some(Subject::Entity(EntityId::new(4, 0)))
        );
    }
}
