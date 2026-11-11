//! The [`ProcessTable`]: scheduled activity that belongs to the world
//! rather than to an entity.

use std::collections::BTreeMap;

use clash_core::{Dur, StateHash, StateHasher, define_id};
use serde::{Deserialize, Serialize};

use crate::error::TimeError;
use crate::timeline::{ClockId, Timeline, Wake, WakeDomain, WakeId};

define_id! {
    /// A process in a [`ProcessTable`]. Ids are never reused within one
    /// table.
    pub struct ProcessId;
}

/// What a process asks for when it has done a piece of work.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Next {
    /// Wake me again after this much local time on my clock.
    Sleep(Dur),
    /// I have finished. Forget me.
    Done,
}

/// One process and its place on the timeline.
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
struct Slot<P> {
    /// The clock the process runs on.
    clock: ClockId,
    /// Its pending wake-up, if it has one.
    wake: Option<WakeId>,
    /// Its state. `None` while it is taken out to be run.
    state: Option<P>,
}

/// A table of processes: plain state that is woken by the timeline, does
/// some work, and says when to wake it next (01 §5.1).
///
/// A pressure wave expanding, a tower collapsing, a body in flight and a
/// fire spreading are processes. The crate that owns them defines one type
/// `P` for all of its kinds, usually an enum, and keeps a
/// `ProcessTable<P>`. The table gives each process an id, keeps exactly
/// one wake-up scheduled for it, and is saved and hashed with the rest of
/// the state.
///
/// # Running a process
///
/// When the timeline pops a wake-up for this table's domain,
/// [`process_of`](ProcessTable::process_of) says which process it is for.
/// There are two ways to run it.
///
/// If the work needs nothing that holds the table, run it in place with
/// [`wake`](ProcessTable::wake):
///
/// ```
/// use clash_core::{Dur, Tick};
/// use clash_time::{ClockId, Next, ProcessTable, Timeline, WakeDomain};
///
/// let mut timeline = Timeline::new();
/// let mut fuses: ProcessTable<u32> = ProcessTable::new(WakeDomain::World);
/// let fuse = fuses
///     .spawn(&mut timeline, ClockId::UNIVERSAL, Dur::SECOND, 2)
///     .unwrap();
///
/// // Each wake burns one unit; the fuse is done when none are left.
/// let mut burn = |left: &mut u32| {
///     *left -= 1;
///     if *left == 0 { Next::Done } else { Next::Sleep(Dur::SECOND) }
/// };
///
/// let (_, wake) = timeline.pop(Tick::MAX).unwrap();
/// assert_eq!(fuses.process_of(&wake), Some(fuse));
/// fuses.wake(&mut timeline, fuse, &mut burn).unwrap();
/// assert_eq!(fuses.get(fuse), Some(&1));
///
/// timeline.pop(Tick::MAX).unwrap();
/// fuses.wake(&mut timeline, fuse, &mut burn).unwrap();
/// assert!(fuses.is_empty());
/// assert_eq!(timeline.now(), Tick(2_000_000));
/// ```
///
/// If the table lives inside the very state the work must change, take
/// the process out with [`take`](ProcessTable::take), run it against that
/// state, and hand it back with [`settle`](ProcessTable::settle):
///
/// ```
/// # use clash_core::Dur;
/// # use clash_time::{ClockId, Next, ProcessId, ProcessTable, Timeline, WakeDomain};
/// struct World {
///     heat: u32,
///     fires: ProcessTable<u32>,
/// }
///
/// fn wake_fire(world: &mut World, timeline: &mut Timeline, id: ProcessId) {
///     let Some(mut strength) = world.fires.take(timeline, id) else { return };
///     world.heat += strength; // free use of the whole world
///     strength -= 1;
///     let next = if strength == 0 { Next::Done } else { Next::Sleep(Dur::SECOND) };
///     world.fires.settle(timeline, id, strength, next).unwrap();
/// }
/// # let mut timeline = Timeline::new();
/// # let mut world = World { heat: 0, fires: ProcessTable::new(WakeDomain::World) };
/// # let id = world.fires.spawn(&mut timeline, ClockId::UNIVERSAL, Dur::SECOND, 2).unwrap();
/// # wake_fire(&mut world, &mut timeline, id);
/// # wake_fire(&mut world, &mut timeline, id);
/// # assert_eq!(world.heat, 3);
/// # assert!(world.fires.is_empty());
/// ```
///
/// # Force-resolving
///
/// A process that must finish at once, because its tract is being
/// unloaded or the loaded-tract budget is spent (02 §10.3), is taken out
/// for good with [`remove`](ProcessTable::remove). That cancels its
/// wake-up and returns its state, for the owner to resolve instantly.
///
/// # Clocks
///
/// A process runs on the clock it was spawned on. When it moves somewhere
/// with a different clock, call [`rehome`](ProcessTable::rehome). Rehome
/// the processes of a clock before destroying the clock.
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub struct ProcessTable<P> {
    domain: WakeDomain,
    next_id: u32,
    slots: BTreeMap<ProcessId, Slot<P>>,
}

impl<P> ProcessTable<P> {
    /// An empty table whose wake-ups carry the given domain.
    pub const fn new(domain: WakeDomain) -> ProcessTable<P> {
        ProcessTable {
            domain,
            next_id: 0,
            slots: BTreeMap::new(),
        }
    }

    /// The domain this table's wake-ups carry.
    pub const fn domain(&self) -> WakeDomain {
        self.domain
    }

    /// The number of processes, counting any that are taken out to be run.
    pub fn len(&self) -> usize {
        self.slots.len()
    }

    /// Whether there are no processes.
    pub fn is_empty(&self) -> bool {
        self.slots.is_empty()
    }

    /// Whether a process with this id exists.
    pub fn contains(&self, id: ProcessId) -> bool {
        self.slots.contains_key(&id)
    }

    /// A process's state. `None` if there is no such process or it is
    /// taken out to be run.
    pub fn get(&self, id: ProcessId) -> Option<&P> {
        self.slots.get(&id).and_then(|slot| slot.state.as_ref())
    }

    /// A process's state, to change in place. `None` if there is no such
    /// process or it is taken out to be run.
    pub fn get_mut(&mut self, id: ProcessId) -> Option<&mut P> {
        self.slots.get_mut(&id).and_then(|slot| slot.state.as_mut())
    }

    /// The clock a process runs on.
    pub fn clock_of(&self, id: ProcessId) -> Option<ClockId> {
        self.slots.get(&id).map(|slot| slot.clock)
    }

    /// A process's pending wake-up. `None` if there is no such process, or
    /// it is being run, or it could not be rescheduled.
    pub fn wake_of(&self, id: ProcessId) -> Option<WakeId> {
        self.slots.get(&id).and_then(|slot| slot.wake)
    }

    /// Every process id, ascending: the order processes were spawned in.
    pub fn ids(&self) -> impl Iterator<Item = ProcessId> + '_ {
        self.slots.keys().copied()
    }

    /// Every process that is not taken out, with its id, in ascending
    /// order of id.
    pub fn iter(&self) -> impl Iterator<Item = (ProcessId, &P)> + '_ {
        self.slots
            .iter()
            .filter_map(|(id, slot)| slot.state.as_ref().map(|state| (*id, state)))
    }

    /// The process a popped wake-up is for: `Some` if the wake-up carries
    /// this table's domain and names a process in it.
    pub fn process_of(&self, wake: &Wake) -> Option<ProcessId> {
        if wake.domain != self.domain {
            return None;
        }
        let id = ProcessId::new(u32::try_from(wake.id).ok()?);
        self.contains(id).then_some(id)
    }

    /// Adds a process on `clock` and schedules its first wake-up for when
    /// `first_wake` of that clock's local time has passed.
    pub fn spawn(
        &mut self,
        timeline: &mut Timeline,
        clock: ClockId,
        first_wake: Dur,
        process: P,
    ) -> Result<ProcessId, TimeError> {
        let next_id = self
            .next_id
            .checked_add(1)
            .ok_or(TimeError::ProcessIdsExhausted)?;
        let id = ProcessId::new(self.next_id);
        let wake = timeline.schedule(clock, first_wake, self.payload(id))?;
        self.next_id = next_id;
        self.slots.insert(
            id,
            Slot {
                clock,
                wake: Some(wake),
                state: Some(process),
            },
        );
        Ok(id)
    }

    /// Runs a process in place: calls `step` on its state, then schedules
    /// its next wake-up or, if it is done, removes it.
    ///
    /// Returns what the process asked for, or `None` without calling
    /// `step` if there is no such process (a stale wake-up) or it is taken
    /// out. Called before the process's wake-up has fired, it runs the
    /// process early and cancels that wake-up.
    ///
    /// If the next wake-up cannot be scheduled because the process's clock
    /// no longer exists, the error is returned and the process stays in
    /// the table with no wake-up: [`remove`](ProcessTable::remove) it.
    pub fn wake(
        &mut self,
        timeline: &mut Timeline,
        id: ProcessId,
        step: impl FnOnce(&mut P) -> Next,
    ) -> Result<Option<Next>, TimeError> {
        let Some(mut process) = self.take(timeline, id) else {
            return Ok(None);
        };
        let next = step(&mut process);
        self.settle(timeline, id, process, next)?;
        Ok(Some(next))
    }

    /// Takes a process's state out of the table so that it can be run
    /// against state that holds the table. Hand it back with
    /// [`settle`](ProcessTable::settle).
    ///
    /// The process keeps its id and its place while it is out. Any
    /// wake-up it still has pending is cancelled. Returns `None` if there
    /// is no such process or it is already out.
    pub fn take(&mut self, timeline: &mut Timeline, id: ProcessId) -> Option<P> {
        let slot = self.slots.get_mut(&id)?;
        let state = slot.state.take()?;
        if let Some(wake) = slot.wake.take() {
            timeline.cancel(wake);
        }
        Some(state)
    }

    /// Hands back a process taken with [`take`](ProcessTable::take), with
    /// what it asked for: its next wake-up is scheduled, or, if it is
    /// done, it is removed.
    ///
    /// Fails with [`TimeError::UnknownProcess`] if the process is not out
    /// (it was removed meanwhile, or never taken); the state given is then
    /// dropped. If the next wake-up cannot be scheduled because the
    /// process's clock no longer exists, the error is returned and the
    /// process stays in the table with no wake-up:
    /// [`remove`](ProcessTable::remove) it.
    pub fn settle(
        &mut self,
        timeline: &mut Timeline,
        id: ProcessId,
        process: P,
        next: Next,
    ) -> Result<(), TimeError> {
        let payload = self.payload(id);
        let Some(slot) = self.slots.get_mut(&id).filter(|slot| slot.state.is_none()) else {
            return Err(TimeError::UnknownProcess(id));
        };
        match next {
            Next::Done => {
                self.slots.remove(&id);
                Ok(())
            }
            Next::Sleep(after) => {
                slot.state = Some(process);
                slot.wake = Some(timeline.schedule(slot.clock, after, payload)?);
                Ok(())
            }
        }
    }

    /// Removes a process for good and returns its state, cancelling its
    /// wake-up. This is the entry point for force-resolving a process: the
    /// owner takes the state and finishes it instantly.
    ///
    /// Returns `None` if there is no such process. Removing a process that
    /// is taken out to be run also returns `None`, and the later
    /// [`settle`](ProcessTable::settle) for it fails.
    #[doc(alias = "force_resolve")]
    pub fn remove(&mut self, timeline: &mut Timeline, id: ProcessId) -> Option<P> {
        let slot = self.slots.remove(&id)?;
        if let Some(wake) = slot.wake {
            timeline.cancel(wake);
        }
        slot.state
    }

    /// Moves a process to another clock. Its pending wake-up moves with
    /// it, keeping the local time it has left.
    pub fn rehome(
        &mut self,
        timeline: &mut Timeline,
        id: ProcessId,
        to: ClockId,
    ) -> Result<(), TimeError> {
        let slot = self
            .slots
            .get_mut(&id)
            .ok_or(TimeError::UnknownProcess(id))?;
        if !timeline.has_clock(to) {
            return Err(TimeError::UnknownClock(to));
        }
        if let Some(wake) = slot.wake {
            timeline.rehome(wake, to)?;
        }
        slot.clock = to;
        Ok(())
    }

    /// The payload of the wake-up that wakes a process.
    fn payload(&self, id: ProcessId) -> Wake {
        Wake::new(self.domain, u64::from(id.raw()), 0)
    }
}

impl<P: StateHash> StateHash for Slot<P> {
    fn state_hash(&self, hasher: &mut StateHasher) {
        self.clock.state_hash(hasher);
        self.wake.state_hash(hasher);
        self.state.state_hash(hasher);
    }
}

impl<P: StateHash> StateHash for ProcessTable<P> {
    fn state_hash(&self, hasher: &mut StateHasher) {
        self.domain.state_hash(hasher);
        hasher.write_u32(self.next_id);
        self.slots.state_hash(hasher);
    }
}
