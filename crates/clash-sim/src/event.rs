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
use clash_world::{
    AreaId, Cause, Layer, Location, RealmId, SiteId, SpawnRequest, TractPos, WorldEvent,
};
use serde::{Deserialize, Serialize};

use crate::bus::{EventPayload, FieldValue};
use crate::decision::DecisionKind;
use crate::entity::{Control, Place};
use crate::reason::Reason;

/// Something the simulation can refer to: what an event is about, what
/// a property is read of, what a variable lives on (05 §5.1).
///
/// These are the kinds the simulation serves so far. In an expression a
/// subject is a handle: [`Kinds`](crate::subject::Kinds) converts
/// between the two. Later packages add their kinds: match with a
/// wildcard arm.
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
    /// The floor, block or fixture of a cell.
    Element(Location, Layer),
    /// A site.
    Site(SiteId),
    /// A tract of a realm, loaded or not.
    Tract(RealmId, TractPos),
    /// A realm.
    Realm(RealmId),
    /// The whole simulation.
    Global,
}

impl Subject {
    /// The name of the subject's kind as rules write it: `entity`, `loc`,
    /// `area`, `element`, `site`, `tract`, `realm`, `global`.
    pub const fn kind(&self) -> &'static str {
        match self {
            Subject::Entity(_) => "entity",
            Subject::Loc(_) => "loc",
            Subject::Area(_) => "area",
            Subject::Element(..) => "element",
            Subject::Site(_) => "site",
            Subject::Tract(..) => "tract",
            Subject::Realm(_) => "realm",
            Subject::Global => "global",
        }
    }

    /// The cell, if the subject is one or an element of one.
    pub const fn loc(&self) -> Option<Location> {
        match self {
            Subject::Loc(loc) | Subject::Element(loc, _) => Some(*loc),
            _ => None,
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
            Subject::Element(loc, layer) => {
                hasher.write_u8(3);
                loc.state_hash(hasher);
                layer.state_hash(hasher);
            }
            Subject::Site(site) => {
                hasher.write_u8(4);
                site.state_hash(hasher);
            }
            Subject::Tract(realm, tract) => {
                hasher.write_u8(5);
                realm.state_hash(hasher);
                tract.state_hash(hasher);
            }
            Subject::Realm(realm) => {
                hasher.write_u8(6);
                realm.state_hash(hasher);
            }
            Subject::Global => hasher.write_u8(7),
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
    /// What held an entity up has gone (02 §14.3). The entity is where
    /// it was: what falling does to it is for the movement package.
    EntityUnsupported {
        /// Who has nothing under them.
        entity: EntityId,
        /// Where it is.
        loc: Location,
    },
    /// Something struck the cell an entity is in: a mass that landed
    /// there (02 §14.3). What that does to the entity is for the body
    /// model.
    EntityExposed {
        /// Who was there.
        entity: EntityId,
        /// The cell that was struck.
        loc: Location,
        /// The kind of damage: the qualified id of a `damage_type`.
        kind: String,
        /// Joules delivered to the cell.
        energy: Real,
        /// How concentrated the delivery was, in pascals.
        intensity: Real,
    },
    /// A variable declared with `"watch": true` changed (05 §6).
    VarChanged {
        /// What the variable lives on.
        owner: Subject,
        /// The variable's qualified name.
        name: String,
        /// What it read as before.
        old: FieldValue,
        /// What it reads as now.
        new: FieldValue,
    },
    /// A tract that has just been loaded asks for something to be put
    /// into the world (02 §9.3, §9.4): its generator's request, or one
    /// that was left for it. Whoever makes entities from definitions
    /// answers.
    SpawnRequested {
        /// The realm of the tract.
        realm: RealmId,
        /// Where, in the realm's base grid.
        loc: Location,
        /// What to make and what else the request says.
        request: SpawnRequest,
    },
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
            EventBody::EntityUnsupported { .. } => "entity_unsupported",
            EventBody::EntityExposed { .. } => "entity_exposed",
            EventBody::SpawnRequested { .. } => "spawn_requested",
            EventBody::VarChanged { .. } => "var_changed",
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
            EventBody::EntityUnsupported { entity, loc }
            | EventBody::EntityExposed { entity, loc, .. } => vec![
                ("self", Subject::Entity(*entity)),
                ("here", Subject::Loc(*loc)),
            ],
            EventBody::SpawnRequested { loc, .. } => vec![("here", Subject::Loc(*loc))],
            EventBody::VarChanged { owner, .. } => match var_owner_name(owner) {
                Some(name) => vec![(name, *owner)],
                None => Vec::new(),
            },
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
            EventBody::EntityExposed {
                kind,
                energy,
                intensity,
                ..
            } => match name {
                "kind" => Some(FieldValue::text(kind)),
                "energy" => Some(FieldValue::from(*energy)),
                "intensity" => Some(FieldValue::from(*intensity)),
                _ => None,
            },
            EventBody::SpawnRequested { realm, request, .. } => match name {
                "what" => Some(FieldValue::text(&request.what)),
                "realm" => Some(FieldValue::from(i64::from(realm.raw()))),
                _ => None,
            },
            EventBody::VarChanged {
                owner,
                name: variable,
                old,
                new,
            } => match name {
                "name" => Some(FieldValue::text(variable)),
                "scope" => Some(FieldValue::text(owner.kind())),
                "old" => Some(old.clone()),
                "new" => Some(new.clone()),
                _ => None,
            },
            EventBody::EntityCreated { .. }
            | EventBody::EntityRemoved { .. }
            | EventBody::EntityRelocated { .. }
            | EventBody::DecisionRequested { .. }
            | EventBody::EntityUnsupported { .. } => None,
        }
    }
}

/// The name of a control mode as data writes it.
pub(crate) const fn control_name(control: Control) -> &'static str {
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
        WorldEvent::OccupantEntombed { occupant, loc, .. }
        | WorldEvent::OccupantUnsupported { occupant, loc, .. }
        | WorldEvent::OccupantExposed { occupant, loc, .. } => vec![
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
            WorldEvent::ElementChanged { layer, .. }
            | WorldEvent::ElementDamaged { layer, .. }
            | WorldEvent::ElementDestroyed { layer, .. },
            "layer",
        ) => Some(FieldValue::text(layer_name(*layer))),
        (WorldEvent::ElementDestroyed { how, .. }, "how") => Some(FieldValue::text(match how {
            clash_world::Destruction::Damage(_) => "damage",
            clash_world::Destruction::Crushed => "crushed",
            clash_world::Destruction::Fell => "fell",
        })),
        (WorldEvent::ElementDestroyed { absorbed, .. }, "absorbed") => {
            Some(FieldValue::from(*absorbed))
        }
        (WorldEvent::ElementDestroyed { passed, .. }, "passed") => Some(FieldValue::from(*passed)),
        (WorldEvent::CollapseStarted { collapse, .. }, "collapse")
        | (WorldEvent::CollapseSettled { collapse, .. }, "collapse") => {
            Some(FieldValue::from(i64::from(collapse.raw())))
        }
        (WorldEvent::OccupantExposed { energy, .. }, "energy") => Some(FieldValue::from(*energy)),
        (WorldEvent::OccupantExposed { intensity, .. }, "intensity") => {
            Some(FieldValue::from(*intensity))
        }
        (WorldEvent::TractLoaded { first, .. }, "first") => Some(FieldValue::from(*first)),
        (
            WorldEvent::TractLoaded { realm, .. }
            | WorldEvent::TractUnloading { realm, .. }
            | WorldEvent::TicketRefused { realm, .. },
            "realm",
        ) => Some(FieldValue::from(i64::from(realm.raw()))),
        (WorldEvent::TicketRefused { ticket, .. }, "ticket") => {
            Some(FieldValue::from(i64::from(ticket.raw())))
        }
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

/// The name `var_changed` binds the owner of the variable under, by its
/// kind: an entity is `self`, a cell `here`, and an area, a site, a
/// tract and a realm go by the name of their kind. A global variable
/// has no owner to bind.
pub(crate) const fn var_owner_name(owner: &Subject) -> Option<&'static str> {
    match owner {
        Subject::Entity(_) => Some("self"),
        Subject::Loc(_) | Subject::Element(..) => Some("here"),
        Subject::Area(_) => Some("area"),
        Subject::Site(_) => Some("site"),
        Subject::Tract(..) => Some("tract"),
        Subject::Realm(_) => Some("realm"),
        Subject::Global => None,
    }
}

/// The name of a layer as data writes it.
pub(crate) const fn layer_name(layer: clash_world::Layer) -> &'static str {
    match layer {
        clash_world::Layer::Floor => "floor",
        clash_world::Layer::Block => "block",
        clash_world::Layer::Fixture => "fixture",
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
            EventBody::EntityUnsupported { entity, loc } => {
                hasher.write_str(self.name());
                entity.state_hash(hasher);
                loc.state_hash(hasher);
            }
            EventBody::EntityExposed {
                entity,
                loc,
                kind,
                energy,
                intensity,
            } => {
                hasher.write_str(self.name());
                entity.state_hash(hasher);
                loc.state_hash(hasher);
                hasher.write_str(kind);
                energy.state_hash(hasher);
                intensity.state_hash(hasher);
            }
            EventBody::SpawnRequested {
                realm,
                loc,
                request,
            } => {
                hasher.write_str(self.name());
                realm.state_hash(hasher);
                loc.state_hash(hasher);
                request.state_hash(hasher);
            }
            EventBody::VarChanged {
                owner,
                name,
                old,
                new,
            } => {
                hasher.write_str(self.name());
                owner.state_hash(hasher);
                hasher.write_str(name);
                old.state_hash(hasher);
                new.state_hash(hasher);
            }
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
        name: "entity_unsupported",
        subjects: &[("self", "entity"), ("here", "loc")],
        fields: &[],
        doc: "What held an entity up has gone: its floor, the solid top below it or the stairs it stood on (02 §14.3). It is where it was. Follows the world's `occupant_unsupported`.",
    },
    EventInfo {
        name: "entity_exposed",
        subjects: &[("self", "entity"), ("here", "loc")],
        fields: &[
            (
                "kind",
                "string",
                "The kind of damage: the qualified id of a `damage_type`.",
            ),
            ("energy", "real", "Joules delivered to the cell."),
            (
                "intensity",
                "real",
                "How concentrated the delivery was, in pascals.",
            ),
        ],
        doc: "Something struck the cell an entity is in: a mass that landed there (02 §14.3). The entity receives the packet the cell did. Follows the world's `occupant_exposed`.",
    },
    EventInfo {
        name: "spawn_requested",
        subjects: &[("here", "loc")],
        fields: &[
            (
                "what",
                "string",
                "What to make: the id of a definition, as the request wrote it.",
            ),
            ("realm", "int", "The realm's id."),
        ],
        doc: "A tract that has just been loaded asks for something to be put into the world at `here`: its generator's request, or one left for it (02 §9.4).",
    },
    EventInfo {
        name: "var_changed",
        subjects: &[
            ("self?", "entity"),
            ("here?", "loc"),
            ("area?", "area"),
            ("site?", "site"),
            ("tract?", "tract"),
            ("realm?", "realm"),
        ],
        fields: &[
            ("name", "string", "The variable's qualified name."),
            (
                "scope",
                "string",
                "The kind of what the variable lives on: `entity`, `loc`, `area`, `site`, `tract`, `realm` or `global`.",
            ),
            (
                "old",
                "any",
                "What it read as before: its value, or its default if it was not set.",
            ),
            ("new", "any", "What it reads as now."),
        ],
        doc: "A variable declared with `\"watch\": true` changed (05 §6). What it lives on is bound under the name of its kind: an entity as `self`, a cell as `here`, an area, a site, a tract or a realm under those names; a global variable binds nothing.",
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
    EventInfo {
        name: "occupant_unsupported",
        subjects: &[("target", "entity"), ("here", "loc")],
        fields: &[],
        doc: "What held an entity up has gone. `entity_unsupported` follows.",
    },
    EventInfo {
        name: "occupant_exposed",
        subjects: &[("target", "entity"), ("here", "loc")],
        fields: &[
            ("energy", "real", "Joules delivered to the cell."),
            (
                "intensity",
                "real",
                "How concentrated the delivery was, in pascals.",
            ),
        ],
        doc: "Something struck the cell an entity is in. `entity_exposed` follows.",
    },
    EventInfo {
        name: "element_destroyed",
        subjects: &[("here", "loc")],
        fields: &[
            ("layer", "string", "`floor`, `block` or `fixture`."),
            ("how", "string", "`damage`, `crushed` or `fell`."),
            (
                "absorbed",
                "real",
                "The joules it took from the packet that destroyed it.",
            ),
            (
                "passed",
                "real",
                "The joules of that packet that carried on beyond it.",
            ),
        ],
        doc: "A floor, block or fixture was destroyed: by damage, by the load on it, or by falling (02 §12.3).",
    },
    EventInfo {
        name: "collapse_started",
        subjects: &[("here", "loc")],
        fields: &[("collapse", "int", "The collapse's id.")],
        doc: "A stability check found something giving way where no collapse was in progress. `here` is the cell whose change started it (02 §14.3).",
    },
    EventInfo {
        name: "collapse_settled",
        subjects: &[("here", "loc")],
        fields: &[("collapse", "int", "The collapse's id.")],
        doc: "A collapse has ended: nothing of it is in flight and no check is waiting near it.",
    },
    EventInfo {
        name: "tract_loaded",
        subjects: &[("here", "loc")],
        fields: &[
            ("realm", "int", "The realm's id."),
            (
                "first",
                "bool",
                "Whether the tract has never been loaded before.",
            ),
        ],
        doc: "A tract has been loaded. `here` is its cell with the lowest horizontal coordinates, at storey 0. The entities that were waiting for it are put back (02 §9.4).",
    },
    EventInfo {
        name: "tract_unloading",
        subjects: &[("here", "loc")],
        fields: &[("realm", "int", "The realm's id.")],
        doc: "A tract is about to be unloaded. The entities standing in it leave the world and wait for it to be loaded again (02 §9.5).",
    },
    EventInfo {
        name: "ticket_refused",
        subjects: &[("here", "loc")],
        fields: &[
            ("realm", "int", "The realm's id."),
            ("ticket", "int", "The id the ticket had."),
        ],
        doc: "A ticket was refused because the tickets together wanted more tracts than may be loaded. It is no longer held (02 §10.3).",
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
