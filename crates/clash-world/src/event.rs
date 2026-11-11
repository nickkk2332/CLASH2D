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
use crate::cause::Cause;
use crate::chunk::{Decal, DecalId};
use crate::collapse::{CollapseId, CollapseTally};
use crate::damage::Destruction;
use crate::defs::{DamageTypeId, ElementState, Layer};
use crate::pos::{CellBox, Location};

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
}

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
            | WorldEvent::ElementDestroyed { tick, .. }
            | WorldEvent::CollapseStarted { tick, .. }
            | WorldEvent::CollapseSettled { tick, .. }
            | WorldEvent::OccupantUnsupported { tick, .. }
            | WorldEvent::OccupantExposed { tick, .. } => *tick,
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
            | WorldEvent::OccupantExposed { cause, .. } => *cause,
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
            | WorldEvent::ElementDestroyed { loc, .. }
            | WorldEvent::CollapseStarted { loc, .. }
            | WorldEvent::CollapseSettled { loc, .. }
            | WorldEvent::OccupantUnsupported { loc, .. }
            | WorldEvent::OccupantExposed { loc, .. } => *loc,
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
            WorldEvent::ElementDestroyed { .. } => "element_destroyed",
            WorldEvent::CollapseStarted { .. } => "collapse_started",
            WorldEvent::CollapseSettled { .. } => "collapse_settled",
            WorldEvent::OccupantUnsupported { .. } => "occupant_unsupported",
            WorldEvent::OccupantExposed { .. } => "occupant_exposed",
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
        }
    }
}
