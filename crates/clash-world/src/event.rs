//! [`WorldEvent`]: what the world reports having happened, and
//! [`EventSink`], where it reports it (02 §7.4).
//!
//! The world never calls upward (01 R4). Whatever changes it emits an
//! event, and the simulation above forwards events to rules and to the
//! presentation stream.

use clash_core::{EntityId, StateHash, StateHasher, Tick, Value, VarKey};
use clash_time::ClockId;
use serde::{Deserialize, Serialize};

use crate::area::AreaId;
use crate::cause::{Cause, CauseKind};
use crate::chunk::{Decal, DecalId};
use crate::cosmos::RealmId;
use crate::defs::{ElementState, Layer};
use crate::pos::{Location, TractPos};
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
            | WorldEvent::TicketRefused { tick, .. } => *tick,
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
            | WorldEvent::OccupantEntombed { cause, .. } => *cause,
            WorldEvent::TractLoaded { .. }
            | WorldEvent::TractUnloading { .. }
            | WorldEvent::TicketRefused { .. } => ACTIVITY_CAUSE,
        }
    }

    /// Where it happened. For an occupant that was moved, where it is
    /// now; for a breach, the cell it leaks through.
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
            | WorldEvent::TicketRefused { loc, .. } => *loc,
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
        }
    }
}
