//! [`WorldEvent`]: what the world reports having happened, and
//! [`EventSink`], where it reports it (02 §7.4).
//!
//! The world never calls upward (01 R4). Whatever changes it emits an
//! event, and the simulation above forwards events to rules and to the
//! presentation stream.

use clash_core::{EntityId, Real, StateHash, StateHasher, Tick, Value, VarKey};
use clash_time::ClockId;
use serde::{Deserialize, Serialize};

use crate::area::AreaId;
use crate::body::{BodyId, BodyOwner};
use crate::cause::{Cause, CauseKind};
use crate::chunk::{Decal, DecalId};
use crate::collapse::{CollapseId, CollapseTally};
use crate::cosmos::RealmId;
use crate::damage::{DamagePacket, DamageResult, Destruction};
use crate::defs::{Contact, DamageTypeId, ElementState, Layer};
use crate::pos::{CellBox, Location, TractPos};
use crate::ticket::{TicketId, TicketReason};

/// Where events go: a list, in the order they were emitted.
///
/// Whoever owns the sink drains it when it is ready to deal with what
/// happened. The world only ever adds to it.
///
/// ```
/// use clash_world::EventSink;
///
/// let mut sink = EventSink::new();
/// sink.emit("first");
/// sink.emit("second");
/// assert_eq!(sink.len(), 2);
/// assert_eq!(sink.drain(), ["first", "second"]);
/// assert!(sink.is_empty());
/// ```
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub struct EventSink<E> {
    events: Vec<E>,
}

impl<E> EventSink<E> {
    /// An empty sink.
    pub const fn new() -> EventSink<E> {
        EventSink { events: Vec::new() }
    }

    /// Adds an event after those already there.
    pub fn emit(&mut self, event: E) {
        self.events.push(event);
    }

    /// How many events are waiting.
    pub fn len(&self) -> usize {
        self.events.len()
    }

    /// Whether no event is waiting.
    pub fn is_empty(&self) -> bool {
        self.events.is_empty()
    }

    /// The events waiting, oldest first.
    pub fn events(&self) -> &[E] {
        &self.events
    }

    /// Takes every event out, oldest first, and leaves the sink empty.
    pub fn drain(&mut self) -> Vec<E> {
        core::mem::take(&mut self.events)
    }

    /// Throws away every event.
    pub fn clear(&mut self) {
        self.events.clear();
    }
}

impl<E> Default for EventSink<E> {
    fn default() -> EventSink<E> {
        EventSink::new()
    }
}

impl<E: StateHash> StateHash for EventSink<E> {
    fn state_hash(&self, hasher: &mut StateHasher) {
        self.events.state_hash(hasher);
    }
}

/// Something that happened in the world (02 §7.4).
///
/// Every event carries the tick it happened at, in universal time, and
/// the cause behind it. Later packages add variants for what they build:
/// match with a wildcard arm.
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum WorldEvent {
    /// A floor, block or fixture was set, replaced, transmuted or
    /// removed. A replaced element's accumulated damage is gone with it.
    ElementChanged {
        /// When.
        tick: Tick,
        /// The cell.
        loc: Location,
        /// Which of the cell's elements.
        layer: Layer,
        /// The state before.
        old: ElementState,
        /// The state after.
        new: ElementState,
        /// Who and what did it.
        cause: Cause,
    },
    /// The accumulated damage of an element changed without the element
    /// itself changing.
    ElementDamaged {
        /// When.
        tick: Tick,
        /// The cell.
        loc: Location,
        /// Which of the cell's elements.
        layer: Layer,
        /// The element.
        state: ElementState,
        /// Its damage before, in 255ths of its capacity.
        old: u8,
        /// Its damage after.
        new: u8,
        /// Who and what did it.
        cause: Cause,
    },
    /// A mark was left in a cell. The mark holds its own cause.
    DecalAdded {
        /// When.
        tick: Tick,
        /// The cell.
        loc: Location,
        /// The mark.
        decal: Decal,
    },
    /// Every mark of one kind was removed from a cell.
    DecalsRemoved {
        /// When.
        tick: Tick,
        /// The cell.
        loc: Location,
        /// The kind of mark.
        kind: DecalId,
        /// How many there were.
        count: u32,
        /// Who and what did it.
        cause: Cause,
    },
    /// A mod variable on a cell was set, changed or cleared.
    CellVarChanged {
        /// When.
        tick: Tick,
        /// The cell.
        loc: Location,
        /// The variable.
        key: VarKey,
        /// Its value before, if it had one.
        old: Option<Value>,
        /// Its value after, if it has one.
        new: Option<Value>,
        /// Who and what did it.
        cause: Cause,
    },
    /// An occupant came to be inside an area: it moved or was placed
    /// there, or the area was made, moved or reshaped over it.
    AreaEntered {
        /// When.
        tick: Tick,
        /// The area.
        area: AreaId,
        /// Who entered.
        occupant: EntityId,
        /// Where the occupant is.
        loc: Location,
        /// The area's clock, if it has one. Whatever is inside runs on it.
        clock: Option<ClockId>,
        /// Who and what did it.
        cause: Cause,
    },
    /// An occupant stopped being inside an area: it moved or was removed,
    /// or the area was removed, moved or reshaped away from it.
    AreaExited {
        /// When.
        tick: Tick,
        /// The area.
        area: AreaId,
        /// Who left.
        occupant: EntityId,
        /// Where the occupant is now, or was when it was removed.
        loc: Location,
        /// The area's clock, if it has one.
        clock: Option<ClockId>,
        /// Who and what did it.
        cause: Cause,
    },
    /// An area made by enclosing a space no longer closes.
    AreaBreached {
        /// When.
        tick: Tick,
        /// The area.
        area: AreaId,
        /// A cell through which it leaks.
        leak: Location,
        /// Who and what opened it.
        cause: Cause,
    },
    /// A breached area closes again.
    AreaClosed {
        /// When.
        tick: Tick,
        /// The area.
        area: AreaId,
        /// Where the space was enclosed from.
        loc: Location,
        /// Who and what closed it.
        cause: Cause,
    },
    /// An edit put something in the way of an occupant, which was moved
    /// to the nearest cell it could occupy (02 §23).
    OccupantDisplaced {
        /// When.
        tick: Tick,
        /// Who was moved.
        occupant: EntityId,
        /// Where it was.
        from: Location,
        /// Where it is now.
        to: Location,
        /// Who and what made the edit.
        cause: Cause,
    },
    /// An edit put something in the way of an occupant and there was
    /// nowhere near for it to go. It stays where it is.
    OccupantEntombed {
        /// When.
        tick: Tick,
        /// Who is walled in.
        occupant: EntityId,
        /// Where it is.
        loc: Location,
        /// Who and what made the edit.
        cause: Cause,
    },
    /// A tract has been loaded (02 §9.4): its cells can be read and
    /// changed, and occupants can be placed in it. The simulation puts
    /// its entities there.
    TractLoaded {
        /// When.
        tick: Tick,
        /// The realm.
        realm: RealmId,
        /// The tract's cell with the lowest horizontal coordinates, at
        /// storey 0, in the realm's base grid.
        loc: Location,
        /// The tract.
        tract: TractPos,
        /// Whether this is the first time the tract has ever been
        /// loaded.
        first: bool,
    },
    /// A tract is about to be unloaded (02 §9.5). The simulation takes
    /// its entities out; a tract that still holds occupants is unloaded
    /// by the first reconciliation that finds it empty.
    TractUnloading {
        /// When.
        tick: Tick,
        /// The realm.
        realm: RealmId,
        /// The tract's cell with the lowest horizontal coordinates, at
        /// storey 0, in the realm's base grid.
        loc: Location,
        /// The tract.
        tract: TractPos,
    },
    /// A ticket was refused because the tickets together wanted more
    /// tracts than may be loaded (02 §10.3). It is no longer held.
    TicketRefused {
        /// When.
        tick: Tick,
        /// The id the ticket had.
        ticket: TicketId,
        /// The realm whose tracts it kept.
        realm: RealmId,
        /// The tract it was centred on, as for `TractLoaded`.
        loc: Location,
        /// Who held it.
        reason: TicketReason,
    },
    /// A floor, block or fixture was destroyed (02 §12.3): by damage, by
    /// the load on it, or by falling. It follows the `ElementChanged`
    /// events of the change itself and of the rubble left.
    ElementDestroyed {
        /// When.
        tick: Tick,
        /// The cell.
        loc: Location,
        /// Which of the cell's elements.
        layer: Layer,
        /// The element that was destroyed.
        old: ElementState,
        /// What it became: its `on_destroy` state, or nothing.
        new: ElementState,
        /// How it was destroyed.
        how: Destruction,
        /// The joules it took from the packet that destroyed it. Zero
        /// for one that was crushed or fell.
        absorbed: Real,
        /// The joules of that packet that carried on beyond it.
        passed: Real,
        /// Who and what destroyed it.
        cause: Cause,
    },
    /// A stability check found something giving way, where no collapse
    /// was in progress (02 §14.3).
    CollapseStarted {
        /// When.
        tick: Tick,
        /// The collapse.
        collapse: CollapseId,
        /// The cell whose change started it.
        loc: Location,
        /// Who and what started it: the instigator of that change.
        cause: Cause,
    },
    /// A collapse has ended: nothing of it is in flight and no check is
    /// waiting near it.
    CollapseSettled {
        /// When.
        tick: Tick,
        /// The collapse.
        collapse: CollapseId,
        /// The cell whose change started it.
        loc: Location,
        /// The box of cells it touched.
        bounds: CellBox,
        /// What it came to, by instigator in order: elements destroyed,
        /// mass brought down, occupants exposed.
        tallies: Vec<CollapseTally>,
        /// Who and what started it.
        cause: Cause,
    },
    /// What held an occupant up has gone: the floor it stood on, the
    /// solid top below it or the stairs it was at the top of. It is
    /// where it was; what falling does to it is for the simulation.
    OccupantUnsupported {
        /// When.
        tick: Tick,
        /// Who has nothing under them.
        occupant: EntityId,
        /// Where it is.
        loc: Location,
        /// Who and what took its footing away.
        cause: Cause,
    },
    /// Something struck the cell an occupant is in: a mass that landed
    /// there (02 §14.3). The occupant receives the same packet the cell
    /// did. What that does to it is for the simulation.
    OccupantExposed {
        /// When.
        tick: Tick,
        /// Who was there.
        occupant: EntityId,
        /// The cell that was struck.
        loc: Location,
        /// The kind of damage.
        kind: DamageTypeId,
        /// Joules delivered to the cell.
        energy: Real,
        /// How concentrated the delivery was, in pascals.
        intensity: Real,
        /// Who and what is behind it.
        cause: Cause,
    },
    /// Something has been set flying (02 §15). From now on the world
    /// moves it.
    BodyLaunched {
        /// When.
        tick: Tick,
        /// The body.
        body: BodyId,
        /// What it is.
        owner: BodyOwner,
        /// The cell it starts in.
        loc: Location,
        /// Where it starts, in metres in its grid's frame.
        pos: [Real; 3],
        /// Its velocity in m/s.
        vel: [Real; 3],
        /// Its mass in kilograms.
        mass: Real,
        /// Who and what set it going.
        cause: Cause,
    },
    /// A body in flight struck a floor, block or fixture (02 §15.2).
    /// What the blow did to the element is reported by the element's
    /// own events. If the body is an entity, what the blow does to it
    /// is for the simulation: it took `energy` at `intensity`, on the
    /// element's `contact` surface if it has one (07 §6.5).
    BodyImpact {
        /// When.
        tick: Tick,
        /// The body.
        body: BodyId,
        /// What it is.
        owner: BodyOwner,
        /// The cell of the element it struck.
        loc: Location,
        /// Which of the cell's elements.
        layer: Layer,
        /// The element as it was when it was struck.
        state: ElementState,
        /// Where it struck, in metres in its grid's frame.
        pos: [Real; 3],
        /// The joules of its motion the body lost.
        energy: Real,
        /// How concentrated the blow was, in pascals.
        intensity: Real,
        /// What the blow did to the element.
        result: DamageResult,
        /// The element's striking surface, if it has one.
        contact: Option<Contact>,
        /// The body's doing: it as the actor, for whoever set it going.
        cause: Cause,
    },
    /// A body in flight struck an occupant (02 §15.2). The world has
    /// shared their momentum along the body's path and changed nothing
    /// else: what the blow does to the occupant, and whether the
    /// occupant flies in turn, is for the simulation.
    BodyHitOccupant {
        /// When.
        tick: Tick,
        /// The body.
        body: BodyId,
        /// What it is.
        owner: BodyOwner,
        /// Who was struck.
        occupant: EntityId,
        /// The cell it was struck in.
        loc: Location,
        /// Where, in metres in the grid's frame: the height says which
        /// part of it.
        pos: [Real; 3],
        /// The blow: the whole energy of the body's motion, as
        /// concentrated as its surface makes it.
        packet: DamagePacket,
        /// The body's velocity as it struck, in m/s.
        vel: [Real; 3],
        /// The velocity, in m/s, that the blow gives the occupant if it
        /// is free to move, by conservation of momentum. Zero where the
        /// world was told no mass for it.
        push: [Real; 3],
        /// The body's doing.
        cause: Cause,
    },
    /// A body has come to rest and is no longer a body (02 §15.3). A
    /// fragment has been left as rubble. An entity is in the cell
    /// named, back in the simulation's hands.
    BodyLanded {
        /// When.
        tick: Tick,
        /// The body it was.
        body: BodyId,
        /// What it is.
        owner: BodyOwner,
        /// The cell it came to rest in.
        loc: Location,
        /// Where, in metres in its grid's frame.
        pos: [Real; 3],
        /// The body's doing.
        cause: Cause,
    },
    /// A body's flight has taken it out of loaded space, and has been
    /// finished by arithmetic (02 §15.4).
    BodyLeftLoaded {
        /// When.
        tick: Tick,
        /// The body.
        body: BodyId,
        /// What it is.
        owner: BodyOwner,
        /// The last loaded cell it was in.
        loc: Location,
        /// Where it comes down, or comes back into loaded space. `None`
        /// if it never does: it left the realm, or its flight has no end.
        lands: Option<Location>,
        /// When it does, in universal time.
        at: Option<Tick>,
        /// Whether it comes back into loaded space and flies on as a
        /// body. If not it is a body no longer: its impact waits with
        /// the tract it came down in.
        returns: bool,
        /// The joules of its motion as it arrives.
        energy: Real,
        /// The body's doing.
        cause: Cause,
    },
}

/// The cause of what the world does of itself when it loads and unloads
/// tracts.
const ACTIVITY_CAUSE: Cause = Cause::new(CauseKind::Coarse);

impl WorldEvent {
    /// When it happened, in universal time.
    pub fn tick(&self) -> Tick {
        match self {
            WorldEvent::ElementChanged { tick, .. }
            | WorldEvent::ElementDamaged { tick, .. }
            | WorldEvent::DecalAdded { tick, .. }
            | WorldEvent::DecalsRemoved { tick, .. }
            | WorldEvent::CellVarChanged { tick, .. }
            | WorldEvent::AreaEntered { tick, .. }
            | WorldEvent::AreaExited { tick, .. }
            | WorldEvent::AreaBreached { tick, .. }
            | WorldEvent::AreaClosed { tick, .. }
            | WorldEvent::OccupantDisplaced { tick, .. }
            | WorldEvent::OccupantEntombed { tick, .. }
            | WorldEvent::TractLoaded { tick, .. }
            | WorldEvent::TractUnloading { tick, .. }
            | WorldEvent::TicketRefused { tick, .. }
            | WorldEvent::ElementDestroyed { tick, .. }
            | WorldEvent::CollapseStarted { tick, .. }
            | WorldEvent::CollapseSettled { tick, .. }
            | WorldEvent::OccupantUnsupported { tick, .. }
            | WorldEvent::OccupantExposed { tick, .. }
            | WorldEvent::BodyLaunched { tick, .. }
            | WorldEvent::BodyImpact { tick, .. }
            | WorldEvent::BodyHitOccupant { tick, .. }
            | WorldEvent::BodyLanded { tick, .. }
            | WorldEvent::BodyLeftLoaded { tick, .. } => *tick,
        }
    }

    /// Who and what is behind it.
    pub fn cause(&self) -> Cause {
        match self {
            WorldEvent::DecalAdded { decal, .. } => decal.cause,
            WorldEvent::ElementChanged { cause, .. }
            | WorldEvent::ElementDamaged { cause, .. }
            | WorldEvent::DecalsRemoved { cause, .. }
            | WorldEvent::CellVarChanged { cause, .. }
            | WorldEvent::AreaEntered { cause, .. }
            | WorldEvent::AreaExited { cause, .. }
            | WorldEvent::AreaBreached { cause, .. }
            | WorldEvent::AreaClosed { cause, .. }
            | WorldEvent::OccupantDisplaced { cause, .. }
            | WorldEvent::OccupantEntombed { cause, .. }
            | WorldEvent::ElementDestroyed { cause, .. }
            | WorldEvent::CollapseStarted { cause, .. }
            | WorldEvent::CollapseSettled { cause, .. }
            | WorldEvent::OccupantUnsupported { cause, .. }
            | WorldEvent::OccupantExposed { cause, .. }
            | WorldEvent::BodyLaunched { cause, .. }
            | WorldEvent::BodyImpact { cause, .. }
            | WorldEvent::BodyHitOccupant { cause, .. }
            | WorldEvent::BodyLanded { cause, .. }
            | WorldEvent::BodyLeftLoaded { cause, .. } => *cause,
            WorldEvent::TractLoaded { .. }
            | WorldEvent::TractUnloading { .. }
            | WorldEvent::TicketRefused { .. } => ACTIVITY_CAUSE,
        }
    }

    /// Where it happened. For an occupant that was moved, where it is
    /// now; for a breach, the cell it leaks through; for a collapse, the
    /// cell whose change started it.
    pub fn location(&self) -> Location {
        match self {
            WorldEvent::ElementChanged { loc, .. }
            | WorldEvent::ElementDamaged { loc, .. }
            | WorldEvent::DecalAdded { loc, .. }
            | WorldEvent::DecalsRemoved { loc, .. }
            | WorldEvent::CellVarChanged { loc, .. }
            | WorldEvent::AreaEntered { loc, .. }
            | WorldEvent::AreaExited { loc, .. }
            | WorldEvent::AreaClosed { loc, .. }
            | WorldEvent::OccupantEntombed { loc, .. }
            | WorldEvent::TractLoaded { loc, .. }
            | WorldEvent::TractUnloading { loc, .. }
            | WorldEvent::TicketRefused { loc, .. }
            | WorldEvent::ElementDestroyed { loc, .. }
            | WorldEvent::CollapseStarted { loc, .. }
            | WorldEvent::CollapseSettled { loc, .. }
            | WorldEvent::OccupantUnsupported { loc, .. }
            | WorldEvent::OccupantExposed { loc, .. }
            | WorldEvent::BodyLaunched { loc, .. }
            | WorldEvent::BodyImpact { loc, .. }
            | WorldEvent::BodyHitOccupant { loc, .. }
            | WorldEvent::BodyLanded { loc, .. }
            | WorldEvent::BodyLeftLoaded { loc, .. } => *loc,
            WorldEvent::AreaBreached { leak, .. } => *leak,
            WorldEvent::OccupantDisplaced { to, .. } => *to,
        }
    }

    /// The name of the event as rules write it: `element_changed`.
    pub fn name(&self) -> &'static str {
        match self {
            WorldEvent::ElementChanged { .. } => "element_changed",
            WorldEvent::ElementDamaged { .. } => "element_damaged",
            WorldEvent::DecalAdded { .. } => "decal_added",
            WorldEvent::DecalsRemoved { .. } => "decals_removed",
            WorldEvent::CellVarChanged { .. } => "cell_var_changed",
            WorldEvent::AreaEntered { .. } => "area_entered",
            WorldEvent::AreaExited { .. } => "area_exited",
            WorldEvent::AreaBreached { .. } => "area_breached",
            WorldEvent::AreaClosed { .. } => "area_closed",
            WorldEvent::OccupantDisplaced { .. } => "occupant_displaced",
            WorldEvent::OccupantEntombed { .. } => "occupant_entombed",
            WorldEvent::TractLoaded { .. } => "tract_loaded",
            WorldEvent::TractUnloading { .. } => "tract_unloading",
            WorldEvent::TicketRefused { .. } => "ticket_refused",
            WorldEvent::ElementDestroyed { .. } => "element_destroyed",
            WorldEvent::CollapseStarted { .. } => "collapse_started",
            WorldEvent::CollapseSettled { .. } => "collapse_settled",
            WorldEvent::OccupantUnsupported { .. } => "occupant_unsupported",
            WorldEvent::OccupantExposed { .. } => "occupant_exposed",
            WorldEvent::BodyLaunched { .. } => "body_launched",
            WorldEvent::BodyImpact { .. } => "body_impact",
            WorldEvent::BodyHitOccupant { .. } => "body_hit_occupant",
            WorldEvent::BodyLanded { .. } => "body_landed",
            WorldEvent::BodyLeftLoaded { .. } => "body_left_loaded",
        }
    }
}

/// Hashes the event's name and then its fields in the order they are
/// declared, so that two runs can compare what they reported.
impl StateHash for WorldEvent {
    fn state_hash(&self, hasher: &mut StateHasher) {
        hasher.write_str(self.name());
        match self {
            WorldEvent::ElementChanged {
                tick,
                loc,
                layer,
                old,
                new,
                cause,
            } => {
                tick.state_hash(hasher);
                loc.state_hash(hasher);
                layer.state_hash(hasher);
                old.state_hash(hasher);
                new.state_hash(hasher);
                cause.state_hash(hasher);
            }
            WorldEvent::ElementDamaged {
                tick,
                loc,
                layer,
                state,
                old,
                new,
                cause,
            } => {
                tick.state_hash(hasher);
                loc.state_hash(hasher);
                layer.state_hash(hasher);
                state.state_hash(hasher);
                hasher.write_u8(*old);
                hasher.write_u8(*new);
                cause.state_hash(hasher);
            }
            WorldEvent::DecalAdded { tick, loc, decal } => {
                tick.state_hash(hasher);
                loc.state_hash(hasher);
                decal.state_hash(hasher);
            }
            WorldEvent::DecalsRemoved {
                tick,
                loc,
                kind,
                count,
                cause,
            } => {
                tick.state_hash(hasher);
                loc.state_hash(hasher);
                kind.state_hash(hasher);
                hasher.write_u32(*count);
                cause.state_hash(hasher);
            }
            WorldEvent::CellVarChanged {
                tick,
                loc,
                key,
                old,
                new,
                cause,
            } => {
                tick.state_hash(hasher);
                loc.state_hash(hasher);
                key.state_hash(hasher);
                old.state_hash(hasher);
                new.state_hash(hasher);
                cause.state_hash(hasher);
            }
            WorldEvent::AreaEntered {
                tick,
                area,
                occupant,
                loc,
                clock,
                cause,
            }
            | WorldEvent::AreaExited {
                tick,
                area,
                occupant,
                loc,
                clock,
                cause,
            } => {
                tick.state_hash(hasher);
                area.state_hash(hasher);
                occupant.state_hash(hasher);
                loc.state_hash(hasher);
                clock.state_hash(hasher);
                cause.state_hash(hasher);
            }
            WorldEvent::AreaBreached {
                tick,
                area,
                leak: loc,
                cause,
            }
            | WorldEvent::AreaClosed {
                tick,
                area,
                loc,
                cause,
            } => {
                tick.state_hash(hasher);
                area.state_hash(hasher);
                loc.state_hash(hasher);
                cause.state_hash(hasher);
            }
            WorldEvent::OccupantDisplaced {
                tick,
                occupant,
                from,
                to,
                cause,
            } => {
                tick.state_hash(hasher);
                occupant.state_hash(hasher);
                from.state_hash(hasher);
                to.state_hash(hasher);
                cause.state_hash(hasher);
            }
            WorldEvent::OccupantEntombed {
                tick,
                occupant,
                loc,
                cause,
            }
            | WorldEvent::OccupantUnsupported {
                tick,
                occupant,
                loc,
                cause,
            } => {
                tick.state_hash(hasher);
                occupant.state_hash(hasher);
                loc.state_hash(hasher);
                cause.state_hash(hasher);
            }
            WorldEvent::TractLoaded {
                tick,
                realm,
                loc,
                tract,
                first,
            } => {
                tick.state_hash(hasher);
                realm.state_hash(hasher);
                loc.state_hash(hasher);
                tract.state_hash(hasher);
                first.state_hash(hasher);
            }
            WorldEvent::TractUnloading {
                tick,
                realm,
                loc,
                tract,
            } => {
                tick.state_hash(hasher);
                realm.state_hash(hasher);
                loc.state_hash(hasher);
                tract.state_hash(hasher);
            }
            WorldEvent::TicketRefused {
                tick,
                ticket,
                realm,
                loc,
                reason,
            } => {
                tick.state_hash(hasher);
                ticket.state_hash(hasher);
                realm.state_hash(hasher);
                loc.state_hash(hasher);
                reason.state_hash(hasher);
            }
            WorldEvent::ElementDestroyed {
                tick,
                loc,
                layer,
                old,
                new,
                how,
                absorbed,
                passed,
                cause,
            } => {
                tick.state_hash(hasher);
                loc.state_hash(hasher);
                layer.state_hash(hasher);
                old.state_hash(hasher);
                new.state_hash(hasher);
                how.state_hash(hasher);
                absorbed.state_hash(hasher);
                passed.state_hash(hasher);
                cause.state_hash(hasher);
            }
            WorldEvent::CollapseStarted {
                tick,
                collapse,
                loc,
                cause,
            } => {
                tick.state_hash(hasher);
                collapse.state_hash(hasher);
                loc.state_hash(hasher);
                cause.state_hash(hasher);
            }
            WorldEvent::CollapseSettled {
                tick,
                collapse,
                loc,
                bounds,
                tallies,
                cause,
            } => {
                tick.state_hash(hasher);
                collapse.state_hash(hasher);
                loc.state_hash(hasher);
                bounds.state_hash(hasher);
                tallies.state_hash(hasher);
                cause.state_hash(hasher);
            }
            WorldEvent::OccupantExposed {
                tick,
                occupant,
                loc,
                kind,
                energy,
                intensity,
                cause,
            } => {
                tick.state_hash(hasher);
                occupant.state_hash(hasher);
                loc.state_hash(hasher);
                kind.state_hash(hasher);
                energy.state_hash(hasher);
                intensity.state_hash(hasher);
                cause.state_hash(hasher);
            }
            WorldEvent::BodyLaunched {
                tick,
                body,
                owner,
                loc,
                pos,
                vel,
                mass,
                cause,
            } => {
                tick.state_hash(hasher);
                body.state_hash(hasher);
                owner.state_hash(hasher);
                loc.state_hash(hasher);
                pos.state_hash(hasher);
                vel.state_hash(hasher);
                mass.state_hash(hasher);
                cause.state_hash(hasher);
            }
            WorldEvent::BodyImpact {
                tick,
                body,
                owner,
                loc,
                layer,
                state,
                pos,
                energy,
                intensity,
                result,
                contact,
                cause,
            } => {
                tick.state_hash(hasher);
                body.state_hash(hasher);
                owner.state_hash(hasher);
                loc.state_hash(hasher);
                layer.state_hash(hasher);
                state.state_hash(hasher);
                pos.state_hash(hasher);
                energy.state_hash(hasher);
                intensity.state_hash(hasher);
                result.state_hash(hasher);
                contact.state_hash(hasher);
                cause.state_hash(hasher);
            }
            WorldEvent::BodyHitOccupant {
                tick,
                body,
                owner,
                occupant,
                loc,
                pos,
                packet,
                vel,
                push,
                cause,
            } => {
                tick.state_hash(hasher);
                body.state_hash(hasher);
                owner.state_hash(hasher);
                occupant.state_hash(hasher);
                loc.state_hash(hasher);
                pos.state_hash(hasher);
                packet.state_hash(hasher);
                vel.state_hash(hasher);
                push.state_hash(hasher);
                cause.state_hash(hasher);
            }
            WorldEvent::BodyLanded {
                tick,
                body,
                owner,
                loc,
                pos,
                cause,
            } => {
                tick.state_hash(hasher);
                body.state_hash(hasher);
                owner.state_hash(hasher);
                loc.state_hash(hasher);
                pos.state_hash(hasher);
                cause.state_hash(hasher);
            }
            WorldEvent::BodyLeftLoaded {
                tick,
                body,
                owner,
                loc,
                lands,
                at,
                returns,
                energy,
                cause,
            } => {
                tick.state_hash(hasher);
                body.state_hash(hasher);
                owner.state_hash(hasher);
                loc.state_hash(hasher);
                lands.state_hash(hasher);
                at.state_hash(hasher);
                returns.state_hash(hasher);
                energy.state_hash(hasher);
                cause.state_hash(hasher);
            }
        }
    }
}
