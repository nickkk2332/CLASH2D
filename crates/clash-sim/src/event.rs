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
    /// Something happened to an action, a plan or a free decision, a
    /// rule told a player something, or a tag changed (03 §13,
    /// 05 §10.6): see [`ActEvent`](crate::action::ActEvent).
    Act(crate::action::ActEvent),
    /// A part came off a body, with everything on it and in it: cut
    /// off, destroyed, or taken off by an edit (06 §7.5). The event
    /// carries the part as it was, for whoever makes an item of it.
    PartDetached {
        /// Whose body it came off.
        entity: EntityId,
        /// Where that body stands, if it is in the world.
        loc: Option<Location>,
        /// Whether the part's definition says it leaves an item, and
        /// not nothing.
        leaves_item: bool,
        /// The part and what went with it: keys, names, layers and
        /// damage, the wounds that were on them, their mass, how it
        /// came off and the cause.
        detached: Box<crate::body::Detached>,
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
            EventBody::Act(event) => event.name(),
            EventBody::PartDetached { .. } => "part_detached",
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
            EventBody::Act(event) => event.subjects(),
            EventBody::PartDetached { entity, loc, .. } => {
                let mut subjects = vec![("self", Subject::Entity(*entity))];
                if let Some(loc) = loc {
                    subjects.push(("here", Subject::Loc(*loc)));
                }
                subjects
            }
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
            EventBody::PartDetached {
                leaves_item,
                detached,
                ..
            } => match name {
                "part" => detached
                    .parts
                    .first()
                    .map(|part| FieldValue::text(part.name())),
                "how" => Some(FieldValue::text(match detached.how {
                    crate::body::Detach::Severed => "severed",
                    crate::body::Detach::Destroyed => "destroyed",
                    crate::body::Detach::Removed => "removed",
                })),
                "mass" => Some(FieldValue::from(detached.mass)),
                "parts" => Some(FieldValue::from(detached.parts.len() as i64)),
                "leaves_item" => Some(FieldValue::from(*leaves_item)),
                _ => None,
            },
            EventBody::EntityCreated { .. }
            | EventBody::EntityRemoved { .. }
            | EventBody::EntityRelocated { .. }
            | EventBody::DecisionRequested { .. }
            | EventBody::EntityUnsupported { .. } => None,
            EventBody::Act(event) => event.field(name),
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
        // The occupant a field touched or dosed, or a body struck.
        WorldEvent::FieldContact { occupant, loc, .. }
        | WorldEvent::OccupantDosed { occupant, loc, .. }
        | WorldEvent::BodyHitOccupant { occupant, loc, .. } => vec![
            ("target", Subject::Entity(*occupant)),
            ("here", Subject::Loc(*loc)),
        ],
        // A body that is an entity is `target`: it is what flies,
        // strikes and comes to rest.
        WorldEvent::BodyLaunched { owner, loc, .. }
        | WorldEvent::BodyImpact { owner, loc, .. }
        | WorldEvent::BodyLanded { owner, loc, .. }
        | WorldEvent::BodyLeftLoaded { owner, loc, .. } => match owner {
            clash_world::BodyOwner::Entity(entity) => vec![
                ("target", Subject::Entity(*entity)),
                ("here", Subject::Loc(*loc)),
            ],
            clash_world::BodyOwner::Debris(_) | clash_world::BodyOwner::Projectile(_) => {
                vec![("here", Subject::Loc(*loc))]
            }
        },
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
        // Fields (02 §16).
        (
            WorldEvent::FieldAppeared { level, .. }
            | WorldEvent::FieldContact { level, .. }
            | WorldEvent::OccupantDosed { level, .. },
            "level",
        ) => Some(FieldValue::from(i64::from(*level))),
        (WorldEvent::OccupantDosed { pool, .. }, "pool") => Some(FieldValue::text(pool)),
        (WorldEvent::OccupantDosed { amount, .. }, "amount") => Some(FieldValue::from(*amount)),
        (WorldEvent::OccupantDosed { route, .. }, "route") => Some(FieldValue::text(route.name())),
        (WorldEvent::FieldBudgetExceeded { active, .. }, "active") => Some(count_field(*active)),
        (WorldEvent::FieldBudgetExceeded { max, .. }, "max") => Some(count_field(*max)),
        // Bodies (02 §15).
        (
            WorldEvent::BodyLaunched { body, .. }
            | WorldEvent::BodyImpact { body, .. }
            | WorldEvent::BodyHitOccupant { body, .. }
            | WorldEvent::BodyLanded { body, .. }
            | WorldEvent::BodyLeftLoaded { body, .. },
            "body",
        ) => Some(FieldValue::from(i64::from(body.raw()))),
        (
            WorldEvent::BodyLaunched { owner, .. }
            | WorldEvent::BodyImpact { owner, .. }
            | WorldEvent::BodyHitOccupant { owner, .. }
            | WorldEvent::BodyLanded { owner, .. }
            | WorldEvent::BodyLeftLoaded { owner, .. },
            "owner",
        ) => Some(FieldValue::text(body_owner_name(owner))),
        (
            WorldEvent::BodyLaunched { owner, .. }
            | WorldEvent::BodyImpact { owner, .. }
            | WorldEvent::BodyHitOccupant { owner, .. }
            | WorldEvent::BodyLanded { owner, .. }
            | WorldEvent::BodyLeftLoaded { owner, .. },
            "tag",
        ) => match owner {
            clash_world::BodyOwner::Projectile(projectile) => Some(count_field(projectile.tag)),
            clash_world::BodyOwner::Entity(_) | clash_world::BodyOwner::Debris(_) => None,
        },
        (WorldEvent::BodyLaunched { mass, .. }, "mass") => Some(FieldValue::from(*mass)),
        (WorldEvent::BodyLaunched { vel, .. }, "speed") => {
            let [east, south, up] = *vel;
            Some(FieldValue::from(
                (east * east + south * south + up * up).sqrt(),
            ))
        }
        (WorldEvent::BodyImpact { layer, .. }, "layer") => {
            Some(FieldValue::text(layer_name(*layer)))
        }
        (WorldEvent::BodyImpact { result, .. }, "result") => Some(FieldValue::text(match result {
            clash_world::DamageResult::Unaffected => "unaffected",
            clash_world::DamageResult::Damaged => "damaged",
            clash_world::DamageResult::Destroyed => "destroyed",
        })),
        (
            WorldEvent::BodyImpact { energy, .. }
            | WorldEvent::BodyLeftLoaded { energy, .. }
            | WorldEvent::ReleaseStarted { energy, .. }
            | WorldEvent::EnergyReleased { energy, .. },
            "energy",
        ) => Some(FieldValue::from(*energy)),
        (WorldEvent::BodyImpact { intensity, .. }, "intensity") => {
            Some(FieldValue::from(*intensity))
        }
        (WorldEvent::BodyHitOccupant { packet, .. }, "energy") => {
            Some(FieldValue::from(packet.energy))
        }
        (WorldEvent::BodyHitOccupant { packet, .. }, "intensity") => {
            Some(FieldValue::from(packet.intensity))
        }
        (WorldEvent::BodyLeftLoaded { returns, .. }, "returns") => Some(FieldValue::from(*returns)),
        // Releases (02 §13).
        (
            WorldEvent::ReleaseStarted { release, .. } | WorldEvent::EnergyReleased { release, .. },
            "release",
        ) => Some(FieldValue::from(i64::from(release.raw()))),
        (
            WorldEvent::ReleaseStarted { reach, .. } | WorldEvent::EnergyReleased { reach, .. },
            "reach",
        ) => Some(FieldValue::from(*reach)),
        (WorldEvent::EnergyReleased { absorbed, .. }, "absorbed") => {
            Some(FieldValue::from(*absorbed))
        }
        (WorldEvent::EnergyReleased { cells, .. }, "cells") => {
            Some(FieldValue::from(i64::from(*cells)))
        }
        (WorldEvent::EnergyReleased { cut_off, .. }, "cut_off") => Some(FieldValue::from(*cut_off)),
        (WorldEvent::EnergyReleased { tallies, .. }, "elements") => Some(FieldValue::from(
            tallies
                .iter()
                .map(|tally| i64::from(tally.elements))
                .sum::<i64>(),
        )),
        (WorldEvent::EnergyReleased { tallies, .. }, "exposed") => Some(FieldValue::from(
            tallies
                .iter()
                .map(|tally| i64::from(tally.exposed))
                .sum::<i64>(),
        )),
        (WorldEvent::EnergyReleased { tallies, .. }, "thrown") => Some(FieldValue::from(
            tallies
                .iter()
                .map(|tally| i64::from(tally.thrown))
                .sum::<i64>(),
        )),
        (WorldEvent::EnergyReleased { tallies, .. }, "mass") => Some(FieldValue::from(
            tallies
                .iter()
                .fold(Real::ZERO, |sum, tally| sum + tally.mass),
        )),
        _ => None,
    }
}

/// A count or a number the world keeps unsigned, as a field: the most
/// an integer field holds if it is more than that.
fn count_field(count: u64) -> FieldValue {
    FieldValue::from(i64::try_from(count).unwrap_or(i64::MAX))
}

/// What a body is, as data writes it.
const fn body_owner_name(owner: &clash_world::BodyOwner) -> &'static str {
    match owner {
        clash_world::BodyOwner::Entity(_) => "entity",
        clash_world::BodyOwner::Debris(_) => "debris",
        clash_world::BodyOwner::Projectile(_) => "projectile",
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
            // The event hashes its own name first.
            EventBody::Act(event) => event.state_hash(hasher),
            EventBody::PartDetached {
                entity,
                loc,
                leaves_item,
                detached,
            } => {
                hasher.write_str(self.name());
                entity.state_hash(hasher);
                loc.state_hash(hasher);
                hasher.write_bool(*leaves_item);
                (**detached).state_hash(hasher);
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
    EventInfo {
        name: "perceived",
        subjects: &[("self", "entity"), ("about?", "entity")],
        fields: &[
            (
                "percept",
                "string",
                "`detection`: awareness of `about` reached 1; `signal`: a signal that was given off was received.",
            ),
            ("sense", "string", "The qualified id of the sense."),
            (
                "signal",
                "string",
                "The qualified id of the kind of signal that carried it.",
            ),
            (
                "event",
                "string?",
                "The type of the event the signal carried, if it carried one.",
            ),
            (
                "about_actor",
                "entity?",
                "Whom the observer took the event's actor for: someone it had detected, or seen where the signal came from. Null if it made out nobody. The true actor is not given.",
            ),
            ("where", "loc?", "Where it seemed to come from."),
            (
                "clarity",
                "real",
                "How far over the threshold it was received: received over threshold.",
            ),
        ],
        doc: "`self` perceived something (15 §6). `about` is who or what it appeared to be about, if the observer made that out: until personas, the entity, which a mind may read only through its belief properties. Its cause names no actor.",
    },
    EventInfo {
        name: "suspicion_raised",
        subjects: &[("self", "entity"), ("about", "entity")],
        fields: &[
            ("sense", "string", "The qualified id of the sense."),
            ("level", "real", "The awareness now, 0 to 1."),
        ],
        doc: "`self`'s awareness of `about` through a sense crossed that sense's `suspect`: something is there (15 §5.1).",
    },
    EventInfo {
        name: "detected",
        subjects: &[("self", "entity"), ("about", "entity")],
        fields: &[
            ("sense", "string", "The qualified id of the sense."),
            ("where", "loc?", "Where it was perceived."),
        ],
        doc: "`self`'s awareness of `about` through a sense reached 1: it knows what and where, and its track is brought up to date (15 §5.1).",
    },
    EventInfo {
        name: "lost_track",
        subjects: &[("self", "entity"), ("about", "entity")],
        fields: &[
            ("sense", "string", "The qualified id of the sense."),
            (
                "where",
                "loc?",
                "The last known place: where the track ends.",
            ),
        ],
        doc: "`self`'s awareness of something it had detected fell to 0 through a sense: it no longer perceives it there (15 §5.1).",
    },
    EventInfo {
        name: "memory_added",
        subjects: &[("self", "entity")],
        fields: &[
            (
                "event",
                "string",
                "What is remembered: an event type or any name.",
            ),
            ("where", "loc?", "Where it happened."),
            ("salience", "real", "How much it matters, 0 to 1."),
        ],
        doc: "`self` remembered something (15 §7.1).",
    },
    EventInfo {
        name: "memory_forgotten",
        subjects: &[("self", "entity")],
        fields: &[
            ("event", "string", "What was forgotten."),
            ("where", "loc?", "Where it happened."),
        ],
        doc: "`self` forgot its least salient memory to make room for another (15 §7.1).",
    },
    EventInfo {
        name: "part_detached",
        subjects: &[("self", "entity"), ("here?", "loc")],
        fields: &[
            (
                "part",
                "string",
                "The name, in the body it came off, of the part that came off.",
            ),
            ("how", "string", "`severed`, `destroyed` or `removed`."),
            (
                "mass",
                "real",
                "The mass of the part and everything that went with it, in kilograms.",
            ),
            (
                "parts",
                "int",
                "How many parts came off: the part and what was on it and in it.",
            ),
            (
                "leaves_item",
                "bool",
                "Whether the part's definition says it leaves an item, and not nothing.",
            ),
        ],
        doc: "A part came off a body with everything on it and in it: cut off or destroyed by a hit, or taken off by an edit (06 §7.5). `self` is whose body it was and `here` is where that body stands. In Rust the event carries the part itself, for whoever makes an item of it.",
    },
    // ---- Minds (09 §17) -------------------------------------------------
    EventInfo {
        name: "goal_adopted",
        subjects: &[("self", "entity")],
        fields: &[
            ("goal", "string", "The qualified id of the goal taken up."),
            (
                "about",
                "any",
                "What it is about: an entity, a memory as a map, a cell, or null.",
            ),
            ("score", "real", "Its score when it was taken up."),
            (
                "previous",
                "string?",
                "The qualified id of the goal it displaced, if any.",
            ),
        ],
        doc: "`self`'s mind took up a goal (09 §3.3): it scored best by more than the held goal's `commit`, or a rule set it.",
    },
    EventInfo {
        name: "goal_done",
        subjects: &[("self", "entity")],
        fields: &[("goal", "string", "The qualified id of the goal.")],
        doc: "The goal `self`'s mind held is done: its `done` held (09 §3.2).",
    },
    EventInfo {
        name: "goal_failed",
        subjects: &[("self", "entity")],
        fields: &[
            ("goal", "string", "The qualified id of the goal."),
            ("code", "string", "The stable code of the reason."),
            ("message", "string", "Why, in words."),
        ],
        doc: "The goal `self`'s mind held failed: its `fail` held, or no method of it could be used. It is set aside for its `retry_after` (09 §4).",
    },
    EventInfo {
        name: "step_failed",
        subjects: &[("self", "entity")],
        fields: &[
            ("goal", "string", "The qualified id of the goal."),
            ("step", "string?", "The kind of step: `go`, `do`, …"),
            ("code", "string", "The stable code of the reason."),
            ("message", "string", "Why, in words."),
        ],
        doc: "A step of the method `self`'s mind was following failed: its action was refused or interrupted, or it could not find its target (09 §4). The method is given up and the next is tried.",
    },
    EventInfo {
        name: "method_chosen",
        subjects: &[("self", "entity")],
        fields: &[
            ("goal", "string", "The qualified id of the goal."),
            (
                "method",
                "int",
                "The method's position in the goal's `methods`.",
            ),
        ],
        doc: "`self`'s mind began to follow a method of its goal (09 §17). Raised only while something listens for it.",
    },
    EventInfo {
        name: "step_started",
        subjects: &[("self", "entity")],
        fields: &[
            ("goal", "string", "The qualified id of the goal."),
            ("step", "string", "The kind of step."),
        ],
        doc: "`self`'s mind began a step (09 §17). Raised only while something listens for it.",
    },
    EventInfo {
        name: "option_chosen",
        subjects: &[("self", "entity")],
        fields: &[
            ("purpose", "string", "The purpose the `choose` step named."),
            (
                "action",
                "string",
                "The qualified id of the action started.",
            ),
        ],
        doc: "A `choose` step of `self`'s mind started an action (09 §17). Raised only while something listens for it.",
    },
    EventInfo {
        name: "said",
        subjects: &[("self", "entity")],
        fields: &[
            ("text", "string", "What was said."),
            (
                "volume",
                "real",
                "How loud, as a factor content gives its signal; 1 for an ordinary voice.",
            ),
        ],
        doc: "`self` said something (a `say` step, 09 §4). Content gives it a signal with an `event_signal`, so that others hear it. Speech (15 §11) will raise its own.",
    },
    EventInfo {
        name: "route_blocked",
        subjects: &[("self", "entity"), ("here", "loc")],
        fields: &[],
        doc: "A change in the world blocked the path `self` was following at `here`: the path is planned again before its next step (09 §5.2).",
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
    EventInfo {
        name: "field_appeared",
        subjects: &[("here", "loc")],
        fields: &[("level", "int", "The level it has there, 1 to 255.")],
        doc: "A field came to be in a cell that held none of it (02 §16). Other changes of a field's level are not reported. Which field it is cannot be tested by a filter.",
    },
    EventInfo {
        name: "field_vanished",
        subjects: &[("here", "loc")],
        fields: &[],
        doc: "The last of a field left a cell (02 §16).",
    },
    EventInfo {
        name: "field_contact",
        subjects: &[("target", "entity"), ("here", "loc")],
        fields: &[("level", "int", "The level the field has there.")],
        doc: "A field and an entity came to share a cell: the field appeared where the entity is, or the entity moved to where the field is. Reported once, not at every step (02 §16).",
    },
    EventInfo {
        name: "occupant_dosed",
        subjects: &[("target", "entity"), ("here", "loc")],
        fields: &[
            ("level", "int", "The level the field has there."),
            (
                "pool",
                "string",
                "The pool the field's `expose` rule adds to, as its definition writes it.",
            ),
            (
                "amount",
                "real",
                "The dose: the level times the rule's `per_level`.",
            ),
            (
                "route",
                "string",
                "The route the dose takes: `breath`, `skin` or `any`.",
            ),
        ],
        doc: "An entity stands in a field whose `expose` rule adds to a pool. Reported once for each step of the field, for the strongest cell of it the entity fills. The world applies no route and no guard: what reaches the pool is for the simulation (02 §16).",
    },
    EventInfo {
        name: "field_budget_exceeded",
        subjects: &[("here", "loc")],
        fields: &[
            ("active", "int", "How many field cells are active."),
            ("max", "int", "The most there may be."),
        ],
        doc: "There are more active field cells than the budget allows: until there are fewer, the weakest decay faster. Reported when a step first finds it so; `here` is the first cell of that step (02 §16).",
    },
    EventInfo {
        name: "body_launched",
        subjects: &[("target?", "entity"), ("here", "loc")],
        fields: &[
            ("body", "int", "The body's id."),
            (
                "owner",
                "string",
                "What the body is: `entity`, `debris` or `projectile`.",
            ),
            (
                "tag",
                "int?",
                "For a projectile, the number it was launched with; otherwise null.",
            ),
            ("mass", "real", "Its mass in kilograms."),
            ("speed", "real", "How fast it starts, in m/s."),
        ],
        doc: "Something has been set flying: from now on the world moves it (02 §15). `target` is the entity the body is, if it is one; `here` is the cell it starts in.",
    },
    EventInfo {
        name: "body_impact",
        subjects: &[("target?", "entity"), ("here", "loc")],
        fields: &[
            ("body", "int", "The body's id."),
            (
                "owner",
                "string",
                "What the body is: `entity`, `debris` or `projectile`.",
            ),
            (
                "tag",
                "int?",
                "For a projectile, the number it was launched with; otherwise null.",
            ),
            ("layer", "string", "`floor`, `block` or `fixture`."),
            ("energy", "real", "The joules of its motion the body lost."),
            (
                "intensity",
                "real",
                "How concentrated the blow was, in pascals.",
            ),
            (
                "result",
                "string",
                "What the blow did to the element: `unaffected`, `damaged` or `destroyed`.",
            ),
        ],
        doc: "A body in flight struck a floor, block or fixture; `here` is the element's cell (02 §15.2). What the blow did to the element is reported by the element's own events. `target` is the entity the body is, if it is one: what the blow does to it is for the simulation.",
    },
    EventInfo {
        name: "body_hit_occupant",
        subjects: &[("target", "entity"), ("here", "loc")],
        fields: &[
            ("body", "int", "The body's id."),
            (
                "owner",
                "string",
                "What the body is: `entity`, `debris` or `projectile`.",
            ),
            (
                "tag",
                "int?",
                "For a projectile, the number it was launched with; otherwise null.",
            ),
            (
                "energy",
                "real",
                "The joules of the blow: the whole energy of the body's motion.",
            ),
            (
                "intensity",
                "real",
                "How concentrated the blow was, in pascals.",
            ),
        ],
        doc: "A body in flight struck an entity, `target`, in the cell `here` (02 §15.2). The world has shared their momentum and changed nothing else: what the blow does is for the simulation. A body that is an entity is the event's `actor`.",
    },
    EventInfo {
        name: "body_landed",
        subjects: &[("target?", "entity"), ("here", "loc")],
        fields: &[
            ("body", "int", "The body's id."),
            (
                "owner",
                "string",
                "What the body is: `entity`, `debris` or `projectile`.",
            ),
            (
                "tag",
                "int?",
                "For a projectile, the number it was launched with; otherwise null.",
            ),
        ],
        doc: "A body has come to rest and is a body no longer (02 §15.3). A fragment has been left as rubble. An entity, `target`, is in the cell `here`, back in the simulation's hands.",
    },
    EventInfo {
        name: "body_left_loaded",
        subjects: &[("target?", "entity"), ("here", "loc")],
        fields: &[
            ("body", "int", "The body's id."),
            (
                "owner",
                "string",
                "What the body is: `entity`, `debris` or `projectile`.",
            ),
            (
                "tag",
                "int?",
                "For a projectile, the number it was launched with; otherwise null.",
            ),
            (
                "returns",
                "bool",
                "Whether it comes back into loaded space and flies on as a body.",
            ),
            ("energy", "real", "The joules of its motion as it arrives."),
        ],
        doc: "A body's flight has taken it out of loaded space and has been finished by arithmetic; `here` is the last loaded cell it was in (02 §15.4). If it does not return it is a body no longer: its impact waits with the tract it came down in.",
    },
    EventInfo {
        name: "release_started",
        subjects: &[("here", "loc")],
        fields: &[
            ("release", "int", "The release's id."),
            ("energy", "real", "How much was let go, in joules."),
            (
                "reach",
                "real",
                "How far it will reach with nothing in its way, in metres.",
            ),
        ],
        doc: "Energy has been let go at a point: its front starts to spread from `here` now (02 §13).",
    },
    EventInfo {
        name: "energy_released",
        subjects: &[("here", "loc")],
        fields: &[
            ("release", "int", "The release's id."),
            ("energy", "real", "How much was let go, in joules."),
            (
                "absorbed",
                "real",
                "The joules that elements took from it. Never more than was let go.",
            ),
            ("reach", "real", "How far its front got, in metres."),
            ("cells", "int", "How many cells its front visited."),
            (
                "cut_off",
                "bool",
                "Whether it was cut off before its reach for the budget of cells.",
            ),
            ("elements", "int", "How many elements it destroyed."),
            (
                "mass",
                "real",
                "The mass of what it destroyed, in kilograms.",
            ),
            (
                "exposed",
                "int",
                "How many times an entity was exposed to it.",
            ),
            ("thrown", "int", "How many entities it threw."),
        ],
        doc: "A release has gone as far as it goes; `here` is where it was let go (02 §13). It sums what the release did: rules that keep account of what someone has broken listen to this, not to every element.",
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

    #[test]
    fn the_events_of_fields_bodies_and_releases_are_in_the_table() {
        use clash_world::{
            BodyId, BodyOwner, DamagePacket, DamageResult, DamageTypeId, ElementState,
            ExposureRoute, FieldId, Projectile, ReleaseId, ReleaseTally,
        };

        let tick = Tick(5);
        let loc = Location::new(GridId::new(0), CellPos::new(1, 2, 0));
        let cause = Cause::DEBUG.by(EntityId::new(9, 0));
        let occupant = EntityId::new(4, 0);
        let field = FieldId::new(1);
        let kind = DamageTypeId::new(0);
        let body = BodyId::new(7);
        let release = ReleaseId::new(3);
        let still = [Real::ZERO; 3];
        let moving = [Real::of(3.0), Real::of(4.0), Real::ZERO];
        let thrown = BodyOwner::Entity(occupant);
        let shot = BodyOwner::Projectile(Projectile {
            kind,
            area: Real::ONE,
            give: Real::ONE,
            tag: 12,
        });
        let tally = |elements: u32| ReleaseTally {
            instigator: None,
            elements,
            mass: Real::of(10.0),
            exposed: 1,
            thrown: 2,
            absorbed: Real::ONE,
        };
        // Each with the subjects it must bind besides those of its
        // cause.
        let events: Vec<(WorldEvent, &[&str])> = vec![
            (
                WorldEvent::FieldAppeared {
                    tick,
                    loc,
                    field,
                    level: 9,
                    cause,
                },
                &["here"],
            ),
            (
                WorldEvent::FieldVanished {
                    tick,
                    loc,
                    field,
                    cause,
                },
                &["here"],
            ),
            (
                WorldEvent::FieldContact {
                    tick,
                    occupant,
                    loc,
                    field,
                    level: 9,
                    cause,
                },
                &["target", "here"],
            ),
            (
                WorldEvent::OccupantDosed {
                    tick,
                    occupant,
                    loc,
                    field,
                    level: 9,
                    pool: "x:smoke".to_owned(),
                    amount: Real::ONE,
                    route: ExposureRoute::Breath,
                    cause,
                },
                &["target", "here"],
            ),
            (
                WorldEvent::FieldBudgetExceeded {
                    tick,
                    loc,
                    active: 9,
                    max: 8,
                    cause,
                },
                &["here"],
            ),
            (
                WorldEvent::BodyLaunched {
                    tick,
                    body,
                    owner: thrown,
                    loc,
                    pos: still,
                    vel: moving,
                    mass: Real::ONE,
                    cause,
                },
                &["target", "here"],
            ),
            (
                WorldEvent::BodyLaunched {
                    tick,
                    body,
                    owner: shot,
                    loc,
                    pos: still,
                    vel: moving,
                    mass: Real::ONE,
                    cause,
                },
                &["here"],
            ),
            (
                WorldEvent::BodyImpact {
                    tick,
                    body,
                    owner: thrown,
                    loc,
                    layer: Layer::Block,
                    state: ElementState::NONE,
                    pos: still,
                    energy: Real::ONE,
                    intensity: Real::ONE,
                    result: DamageResult::Damaged,
                    contact: None,
                    cause,
                },
                &["target", "here"],
            ),
            (
                WorldEvent::BodyHitOccupant {
                    tick,
                    body,
                    owner: shot,
                    occupant,
                    loc,
                    pos: still,
                    packet: Box::new(DamagePacket::new(kind, Real::of(6.0), Real::of(7.0), cause)),
                    vel: moving,
                    push: still,
                },
                &["target", "here"],
            ),
            (
                WorldEvent::BodyLanded {
                    tick,
                    body,
                    owner: thrown,
                    loc,
                    pos: still,
                    cause,
                },
                &["target", "here"],
            ),
            (
                WorldEvent::BodyLeftLoaded {
                    tick,
                    body,
                    owner: shot,
                    loc,
                    lands: None,
                    at: None,
                    returns: false,
                    energy: Real::ONE,
                    cause,
                },
                &["here"],
            ),
            (
                WorldEvent::ReleaseStarted {
                    tick,
                    release,
                    loc,
                    kind,
                    energy: Real::ONE,
                    reach: Real::ONE,
                    cause,
                },
                &["here"],
            ),
            (
                WorldEvent::EnergyReleased {
                    tick,
                    release,
                    loc,
                    kind,
                    energy: Real::ONE,
                    absorbed: Real::ONE,
                    reach: Real::ONE,
                    cells: 30,
                    cut_off: true,
                    tallies: vec![tally(2), tally(3)],
                    cause,
                },
                &["here"],
            ),
        ];
        for (event, binds) in events {
            let body = EventBody::World(event);
            let info = WORLD_EVENTS
                .iter()
                .find(|info| info.name == body.name())
                .unwrap_or_else(|| panic!("`{}` is not in WORLD_EVENTS", body.name()));
            assert!(!info.doc.is_empty());
            // Every field the table lists can be read by name, unless it
            // is one that may be missing.
            for (field, ty, doc) in info.fields {
                assert!(!doc.is_empty());
                assert!(
                    ty.ends_with('?') || body.field(field).is_some(),
                    "`{}` lists `{field}` but it cannot be read",
                    info.name
                );
            }
            let subjects = body.subjects();
            let names: Vec<&str> = subjects.iter().map(|(name, _)| *name).collect();
            assert_eq!(names, binds, "{}", info.name);
            for (name, subject) in subjects {
                let declared = info
                    .subjects
                    .iter()
                    .find(|(have, _)| have.trim_end_matches('?') == name)
                    .unwrap_or_else(|| panic!("`{}` binds undeclared `{name}`", info.name));
                assert_eq!(declared.1, subject.kind());
            }
            // And every subject that is not optional is bound.
            for (name, _) in info.subjects {
                assert!(name.ends_with('?') || binds.contains(name), "{}", info.name);
            }
        }

        // What a filter reads of them.
        let read = |event: WorldEvent, name: &str| EventBody::World(event).field(name);
        let launched = WorldEvent::BodyLaunched {
            tick,
            body,
            owner: shot,
            loc,
            pos: still,
            vel: moving,
            mass: Real::ONE,
            cause,
        };
        assert_eq!(
            read(launched.clone(), "speed"),
            Some(FieldValue::from(Real::of(5.0)))
        );
        assert_eq!(
            read(launched.clone(), "owner"),
            Some(FieldValue::text("projectile"))
        );
        assert_eq!(
            read(launched.clone(), "tag"),
            Some(FieldValue::from(12_i64))
        );
        assert_eq!(read(launched, "body"), Some(FieldValue::from(7_i64)));
        let landed = WorldEvent::BodyLanded {
            tick,
            body,
            owner: thrown,
            loc,
            pos: still,
            cause,
        };
        assert_eq!(
            read(landed.clone(), "owner"),
            Some(FieldValue::text("entity"))
        );
        assert_eq!(read(landed, "tag"), None);
        let summed = WorldEvent::EnergyReleased {
            tick,
            release,
            loc,
            kind,
            energy: Real::ONE,
            absorbed: Real::ONE,
            reach: Real::ONE,
            cells: 30,
            cut_off: true,
            tallies: vec![tally(2), tally(3)],
            cause,
        };
        assert_eq!(
            read(summed.clone(), "elements"),
            Some(FieldValue::from(5_i64))
        );
        assert_eq!(
            read(summed.clone(), "thrown"),
            Some(FieldValue::from(4_i64))
        );
        assert_eq!(
            read(summed.clone(), "mass"),
            Some(FieldValue::from(Real::of(20.0)))
        );
        assert_eq!(read(summed, "cut_off"), Some(FieldValue::from(true)));
    }

    #[test]
    fn every_world_event_name_is_in_the_table_once() {
        let mut names: Vec<&str> = WORLD_EVENTS.iter().map(|info| info.name).collect();
        let listed = names.len();
        names.sort_unstable();
        names.dedup();
        assert_eq!(names.len(), listed);
        for name in [
            "field_appeared",
            "field_vanished",
            "field_contact",
            "occupant_dosed",
            "field_budget_exceeded",
            "body_launched",
            "body_impact",
            "body_hit_occupant",
            "body_landed",
            "body_left_loaded",
            "release_started",
            "energy_released",
        ] {
            assert!(names.contains(&name), "{name}");
        }
    }
}
