//! The [`Timeline`]: the queue of wake-ups and the tree of clocks.

use std::collections::{BTreeMap, BTreeSet};

use clash_core::{Dur, LookupMap, Real, StateHash, StateHasher, Tick, define_id};
use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::error::TimeError;

define_id! {
    /// A clock in a [`Timeline`]'s tree of clocks.
    ///
    /// Ids are never reused within one timeline, so an id held after its
    /// clock was destroyed is simply unknown.
    pub struct ClockId;

    /// A stable handle to a scheduled wake-up.
    ///
    /// The number is the wake-up's sequence number: it is assigned when
    /// the wake-up is scheduled and never changes, whatever happens to the
    /// clock it is on (03 §3.2). A later wake-up always has a larger id.
    pub struct WakeId(u64);
}

impl ClockId {
    /// The root of the clock tree: universal time. Its rate is always 1.
    pub const UNIVERSAL: ClockId = ClockId::new(0);
}

/// Who handles a wake-up when it fires.
///
/// The timeline does not know what any of these mean. The simulation
/// kernel reads the domain of a popped wake-up to pick a handler (03 §4.3).
/// When a new kind of handler is introduced, add its domain at the end.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WakeDomain {
    /// A world process: a blast, a collapse, a body in flight, a field step.
    World,
    /// A phase boundary of an entity's action.
    Action,
    /// A timer: a status wearing off, a cooldown, a delayed effect.
    Timer,
    /// A recurring or delayed rule.
    Rule,
    /// A stepper that advances things in bulk at lower detail.
    Stepper,
    /// Perception: a sense's sweep, or a delayed signal arriving
    /// (15 §5.2, §3.1).
    Perception,
}

impl StateHash for WakeDomain {
    fn state_hash(&self, hasher: &mut StateHasher) {
        hasher.write_u8(*self as u8);
    }
}

/// What a wake-up carries: who handles it and two numbers for the handler.
///
/// The payload is opaque to the timeline. By convention `id` names the
/// thing being woken (a process id, an entity id packed with
/// `EntityId::to_bits`) and `arg` is whatever else the handler needs.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Serialize, Deserialize)]
pub struct Wake {
    /// Who handles it.
    pub domain: WakeDomain,
    /// What is being woken, in the handler's own terms.
    pub id: u64,
    /// Anything else the handler needs.
    pub arg: u64,
}

impl Wake {
    /// A wake-up payload.
    pub const fn new(domain: WakeDomain, id: u64, arg: u64) -> Wake {
        Wake { domain, id, arg }
    }
}

impl StateHash for Wake {
    fn state_hash(&self, hasher: &mut StateHasher) {
        self.domain.state_hash(hasher);
        hasher.write_u64(self.id);
        hasher.write_u64(self.arg);
    }
}

/// One clock of the tree.
#[derive(Clone, PartialEq, Eq, Debug)]
struct Clock {
    /// `None` only for the universal clock.
    parent: Option<ClockId>,
    /// Rate relative to the parent. Zero or more, finite.
    rate: Real,
    /// Whole microseconds of local time at `synced`.
    local: Dur,
    /// The fraction of a microsecond of local time at `synced`, from zero
    /// up to but not including one. Kept so that bringing a clock up to
    /// date never loses local time.
    frac: Real,
    /// The universal tick at which `local` and `frac` were current.
    synced: Tick,

    // Derived from the above and from the wake-ups; rebuilt on load.
    /// The product of rates from the root down to this clock.
    effective: Real,
    /// Child clocks, ascending.
    children: Vec<ClockId>,
    /// The wake-ups on this clock, in scheduling order.
    wakes: BTreeSet<WakeId>,
}

impl Clock {
    /// Local time at `now`: whole microseconds and the fraction left over.
    fn local_at(&self, now: Tick) -> (Dur, Real) {
        let span = now.since(self.synced);
        if self.effective == Real::ONE {
            // Exact however large the numbers get.
            return (self.local + span, self.frac);
        }
        let total = self.frac + local_elapsed(span, self.effective);
        let whole = total.floor();
        (self.local + Dur(whole.to_i64()), total - whole)
    }
}

/// One scheduled wake-up.
#[derive(Clone, PartialEq, Eq, Debug)]
struct Entry {
    clock: ClockId,
    /// Local time left before it fires, in microseconds, as of `as_of`.
    /// Never negative.
    remaining: Real,
    /// The universal tick at which `remaining` was current. Never earlier
    /// than the last change to the effective rate of `clock`.
    as_of: Tick,
    /// The universal tick at which it fires, or `None` while it is parked
    /// on a clock whose effective rate is zero.
    due: Option<Tick>,
    wake: Wake,
}

/// Local time, in microseconds, that passes on a clock of the given
/// effective rate while `span` of universal time passes.
fn local_elapsed(span: Dur, rate: Real) -> Real {
    Real::from_i64(span.as_micros()) * rate
}

/// The universal time until `remaining` microseconds of local time have
/// passed at the given effective rate, rounded up to a whole tick, or
/// `None` if they never will (03 §3.3).
fn universal_wait(remaining: Real, rate: Real) -> Option<Dur> {
    if remaining <= Real::ZERO {
        Some(Dur::ZERO)
    } else if rate <= Real::ZERO {
        None
    } else {
        Some(Dur((remaining / rate).ceil().to_i64()))
    }
}

/// The timeline: every wake-up that will happen, ordered by time, and the
/// tree of clocks those wake-ups run on (01 §5, 03 §3).
///
/// Nothing in the simulation runs "every turn". Whatever happens later was
/// scheduled here, and the simulation proceeds by popping the earliest
/// wake-up and running it.
///
/// # Wake-ups
///
/// [`schedule`](Timeline::schedule) asks for a wake-up after some *local*
/// time has passed on a clock. Wake-ups pop in order of
/// `(universal due tick, sequence number)`, so two that are due at the
/// same tick run in the order they were scheduled, whatever has happened
/// to their clocks since.
///
/// ```
/// use clash_core::{Dur, Tick};
/// use clash_time::{ClockId, Timeline, Wake, WakeDomain};
///
/// let mut timeline = Timeline::new();
/// let ring = Wake::new(WakeDomain::Timer, 1, 0);
/// let id = timeline.schedule(ClockId::UNIVERSAL, Dur::from_secs(3), ring).unwrap();
///
/// assert_eq!(timeline.peek(), Some(Tick(3_000_000)));
/// assert_eq!(timeline.pop(Tick(2_000_000)), None);
/// assert_eq!(timeline.pop(Tick::MAX), Some((id, ring)));
/// assert_eq!(timeline.now(), Tick(3_000_000));
/// ```
///
/// # Clocks
///
/// Every wake-up belongs to a clock. A clock has a parent and a rate
/// relative to it, and its *effective rate* is the product of rates up to
/// the root, [`ClockId::UNIVERSAL`]. Slow a clock and everything on it and
/// under it slows. Stop it (rate zero) and its wake-ups are *parked*: they
/// never fire, and they resume with the same local time left when the rate
/// rises again.
///
/// ```
/// use clash_core::{Dur, Real, Tick};
/// use clash_time::{ClockId, Timeline, Wake, WakeDomain};
///
/// let mut timeline = Timeline::new();
/// let field = timeline.create_clock(ClockId::UNIVERSAL, Real::ONE).unwrap();
/// let wake = Wake::new(WakeDomain::Timer, 1, 0);
/// let id = timeline.schedule(field, Dur::from_secs(10), wake).unwrap();
///
/// // Four seconds in, the field slows to half speed.
/// timeline.advance_to(Tick(4_000_000));
/// timeline.set_rate(field, Real::of(0.5)).unwrap();
/// assert_eq!(timeline.remaining(id), Some(Dur::from_secs(6)));
/// assert_eq!(timeline.peek(), Some(Tick(16_000_000)));
/// ```
///
/// # Rounding
///
/// A wake-up's remaining local time is tracked exactly, as a real number
/// of microseconds. Its universal due tick is that time divided by the
/// effective rate and rounded **up** to a whole tick, worked out afresh
/// whenever a rate changes. So however many times its clocks change, a
/// wake-up fires within one tick of the exact moment (03 §14, T2).
///
/// # Saving
///
/// A timeline serialises to its essential state and rebuilds its queue and
/// indexes on load. A loaded timeline pops exactly what the original would
/// have, in the same order, and has the same [`StateHash`].
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Timeline {
    now: Tick,
    next_wake: u64,
    next_clock: u32,
    clocks: BTreeMap<ClockId, Clock>,
    /// Every scheduled wake-up. Only ever looked up by id, except when
    /// saving and hashing, which take the entries sorted by id.
    wakes: LookupMap<WakeId, Entry>,
    /// Derived: the wake-ups that are not parked, in the order they pop.
    queue: BTreeSet<(Tick, WakeId)>,
}

impl Timeline {
    /// A timeline at tick zero, with the universal clock and nothing
    /// scheduled.
    pub fn new() -> Timeline {
        let mut clocks = BTreeMap::new();
        clocks.insert(
            ClockId::UNIVERSAL,
            Clock {
                parent: None,
                rate: Real::ONE,
                local: Dur::ZERO,
                frac: Real::ZERO,
                synced: Tick::ZERO,
                effective: Real::ONE,
                children: Vec::new(),
                wakes: BTreeSet::new(),
            },
        );
        Timeline {
            now: Tick::ZERO,
            next_wake: 0,
            next_clock: 1,
            clocks,
            wakes: LookupMap::new(),
            queue: BTreeSet::new(),
        }
    }

    // ---- Time ------------------------------------------------------------

    /// The current universal time.
    pub fn now(&self) -> Tick {
        self.now
    }

    /// The current local time on a clock: how much time has passed for
    /// things on it since it was created, in whole microseconds. On the
    /// universal clock this is the tick count.
    pub fn local_now(&self, clock: ClockId) -> Result<Dur, TimeError> {
        let clock = self.clock(clock)?;
        Ok(clock.local_at(self.now).0)
    }

    /// The universal tick at which the next wake-up is due, if any is
    /// queued. Parked wake-ups are not queued.
    pub fn peek(&self) -> Option<Tick> {
        self.queue.first().map(|(due, _)| *due)
    }

    /// Takes the next wake-up, if it is due at or before `limit`, and moves
    /// the current time forward to its due tick.
    ///
    /// Returns `None`, leaving the current time where it is, when nothing
    /// is due by `limit`. To then move time on to the limit, call
    /// [`advance_to`](Timeline::advance_to).
    pub fn pop(&mut self, limit: Tick) -> Option<(WakeId, Wake)> {
        let (due, id) = *self.queue.first()?;
        if due > limit {
            return None;
        }
        self.queue.remove(&(due, id));
        let entry = self.wakes.remove(&id)?;
        if let Some(clock) = self.clocks.get_mut(&entry.clock) {
            clock.wakes.remove(&id);
        }
        self.now = self.now.max(due);
        Some((id, entry.wake))
    }

    /// Moves the current time forward to `tick` without running anything,
    /// and returns the new current time.
    ///
    /// Time never passes a queued wake-up and never goes backwards: if a
    /// wake-up is due before `tick`, time stops at its due tick instead,
    /// and a `tick` in the past changes nothing.
    pub fn advance_to(&mut self, tick: Tick) -> Tick {
        let target = match self.peek() {
            Some(due) => tick.min(due),
            None => tick,
        };
        self.now = self.now.max(target);
        self.now
    }

    // ---- Wake-ups --------------------------------------------------------

    /// Schedules a wake-up for when `after` more local time has passed on
    /// `clock`, and returns its handle.
    ///
    /// Its universal due tick is `now + ceil(after / effective rate)`. A
    /// negative `after` counts as zero: the wake-up is due now, after any
    /// already scheduled for this tick. On a clock whose effective rate is
    /// zero it is parked until the rate rises.
    pub fn schedule(
        &mut self,
        clock: ClockId,
        after: Dur,
        wake: Wake,
    ) -> Result<WakeId, TimeError> {
        let rate = self.clock(clock)?.effective;
        let id = WakeId::new(self.next_wake);
        self.next_wake += 1;

        let remaining = Real::from_i64(after.as_micros().max(0));
        let due = universal_wait(remaining, rate).map(|wait| self.now + wait);
        if let Some(due) = due {
            self.queue.insert((due, id));
        }
        if let Some(clock) = self.clocks.get_mut(&clock) {
            clock.wakes.insert(id);
        }
        self.wakes.insert(
            id,
            Entry {
                clock,
                remaining,
                as_of: self.now,
                due,
                wake,
            },
        );
        Ok(id)
    }

    /// Cancels a wake-up. Returns whether there was one to cancel: `false`
    /// if it has already fired or been cancelled.
    pub fn cancel(&mut self, id: WakeId) -> bool {
        let Some(entry) = self.wakes.remove(&id) else {
            return false;
        };
        if let Some(due) = entry.due {
            self.queue.remove(&(due, id));
        }
        if let Some(clock) = self.clocks.get_mut(&entry.clock) {
            clock.wakes.remove(&id);
        }
        true
    }

    /// The local time left before a wake-up fires, on its own clock,
    /// rounded up to a whole microsecond. `None` if there is no such
    /// wake-up.
    pub fn remaining(&self, id: WakeId) -> Option<Dur> {
        let entry = self.wakes.get(&id)?;
        Some(Dur(self.remaining_exact(entry).ceil().to_i64()))
    }

    /// Moves a wake-up to another clock, keeping the local time it has
    /// left. It will take that much of the new clock's local time.
    pub fn rehome(&mut self, id: WakeId, to: ClockId) -> Result<(), TimeError> {
        let rate = self.clock(to)?.effective;
        let entry = self.wakes.get(&id).ok_or(TimeError::UnknownWake(id))?;
        let from = entry.clock;
        if from == to {
            return Ok(());
        }
        let remaining = self.remaining_exact(entry);
        let old_due = entry.due;
        let due = universal_wait(remaining, rate).map(|wait| self.now + wait);

        if let Some(old_due) = old_due {
            self.queue.remove(&(old_due, id));
        }
        if let Some(due) = due {
            self.queue.insert((due, id));
        }
        if let Some(clock) = self.clocks.get_mut(&from) {
            clock.wakes.remove(&id);
        }
        if let Some(clock) = self.clocks.get_mut(&to) {
            clock.wakes.insert(id);
        }
        if let Some(entry) = self.wakes.get_mut(&id) {
            entry.clock = to;
            entry.remaining = remaining;
            entry.as_of = self.now;
            entry.due = due;
        }
        Ok(())
    }

    /// The payload of a scheduled wake-up, if there is one with this id.
    pub fn wake(&self, id: WakeId) -> Option<Wake> {
        self.wakes.get(&id).map(|entry| entry.wake)
    }

    /// The clock a scheduled wake-up is on.
    pub fn clock_of(&self, id: WakeId) -> Option<ClockId> {
        self.wakes.get(&id).map(|entry| entry.clock)
    }

    /// The universal tick at which a wake-up will fire if no clock changes
    /// first. `None` if it is parked or there is no such wake-up.
    pub fn due(&self, id: WakeId) -> Option<Tick> {
        self.wakes.get(&id).and_then(|entry| entry.due)
    }

    /// Whether a wake-up exists but cannot fire because its clock's
    /// effective rate is zero.
    pub fn is_parked(&self, id: WakeId) -> bool {
        self.wakes.get(&id).is_some_and(|entry| entry.due.is_none())
    }

    /// The number of scheduled wake-ups, parked ones included.
    pub fn wake_count(&self) -> usize {
        self.wakes.len()
    }

    /// The wake-ups on a clock, in the order they were scheduled. Empty
    /// for an unknown clock.
    pub fn wakes_on(&self, clock: ClockId) -> impl Iterator<Item = WakeId> + '_ {
        self.clocks
            .get(&clock)
            .into_iter()
            .flat_map(|clock| clock.wakes.iter().copied())
    }

    // ---- Clocks ----------------------------------------------------------

    /// Creates a clock under `parent`, running at `rate` relative to it.
    /// Its local time starts at zero.
    pub fn create_clock(&mut self, parent: ClockId, rate: Real) -> Result<ClockId, TimeError> {
        let rate = checked_rate(rate)?;
        let parent_rate = self.clock(parent)?.effective;
        let id = ClockId::new(self.next_clock);
        self.next_clock = self
            .next_clock
            .checked_add(1)
            .ok_or(TimeError::ClockIdsExhausted)?;

        self.clocks.insert(
            id,
            Clock {
                parent: Some(parent),
                rate,
                local: Dur::ZERO,
                frac: Real::ZERO,
                synced: self.now,
                effective: parent_rate * rate,
                children: Vec::new(),
                wakes: BTreeSet::new(),
            },
        );
        if let Some(parent) = self.clocks.get_mut(&parent) {
            // A new id is larger than every existing one.
            parent.children.push(id);
        }
        Ok(id)
    }

    /// Sets a clock's rate relative to its parent.
    ///
    /// The clock and every clock under it first bring their local time up
    /// to date at the old rate. Then every wake-up on them gets a new
    /// universal due tick from the local time it has left. A rate of zero
    /// parks them. Order among the wake-ups of one clock is unchanged.
    ///
    /// Setting the rate a clock already has changes nothing at all.
    pub fn set_rate(&mut self, clock: ClockId, rate: Real) -> Result<(), TimeError> {
        if clock == ClockId::UNIVERSAL {
            return Err(TimeError::UniversalClock);
        }
        let rate = checked_rate(rate)?;
        if self.clock(clock)?.rate == rate {
            return Ok(());
        }

        let affected = self.subtree(clock);
        let moved = self.bring_up_to_date(&affected);
        if let Some(clock) = self.clocks.get_mut(&clock) {
            clock.rate = rate;
        }
        self.recompute_rates(&affected);
        self.requeue(&moved);
        Ok(())
    }

    /// Moves a clock, with everything under it, to a new parent. Its own
    /// rate and local time are kept; its effective rate becomes the new
    /// parent's times its own.
    ///
    /// Wake-ups on the clock and under it keep the local time they have
    /// left and get new universal due ticks, as for
    /// [`set_rate`](Timeline::set_rate).
    pub fn reparent(&mut self, clock: ClockId, parent: ClockId) -> Result<(), TimeError> {
        if clock == ClockId::UNIVERSAL {
            return Err(TimeError::UniversalClock);
        }
        let old_parent = self.clock(clock)?.parent;
        self.clock(parent)?;
        if old_parent == Some(parent) {
            return Ok(());
        }
        let affected = self.subtree(clock);
        if affected.contains(&parent) {
            return Err(TimeError::ClockLoop { clock, parent });
        }

        let moved = self.bring_up_to_date(&affected);
        if let Some(old_parent) = old_parent.and_then(|id| self.clocks.get_mut(&id)) {
            clash_core::sorted::remove(&mut old_parent.children, &clock);
        }
        if let Some(new_parent) = self.clocks.get_mut(&parent) {
            clash_core::sorted::insert(&mut new_parent.children, clock);
        }
        if let Some(clock) = self.clocks.get_mut(&clock) {
            clock.parent = Some(parent);
        }
        self.recompute_rates(&affected);
        self.requeue(&moved);
        Ok(())
    }

    /// Destroys a clock. Its wake-ups move to its parent, keeping the
    /// local time they have left, and its child clocks become children of
    /// its parent, keeping their own rates.
    pub fn destroy_clock(&mut self, clock: ClockId) -> Result<(), TimeError> {
        if clock == ClockId::UNIVERSAL {
            return Err(TimeError::UniversalClock);
        }
        let Some(parent) = self.clock(clock)?.parent else {
            return Err(TimeError::UniversalClock);
        };

        let affected = self.subtree(clock);
        let moved = self.bring_up_to_date(&affected);

        let Some(destroyed) = self.clocks.remove(&clock) else {
            return Err(TimeError::UnknownClock(clock));
        };
        for id in &destroyed.wakes {
            if let Some(entry) = self.wakes.get_mut(id) {
                entry.clock = parent;
            }
        }
        for child in &destroyed.children {
            if let Some(child) = self.clocks.get_mut(child) {
                child.parent = Some(parent);
            }
        }
        if let Some(heir) = self.clocks.get_mut(&parent) {
            clash_core::sorted::remove(&mut heir.children, &clock);
            for child in &destroyed.children {
                clash_core::sorted::insert(&mut heir.children, *child);
            }
            heir.wakes.extend(destroyed.wakes.iter().copied());
        }

        // The destroyed clock is first in `affected`; the rest survive.
        self.recompute_rates(&affected[1..]);
        self.requeue(&moved);
        Ok(())
    }

    /// Whether a clock with this id exists.
    pub fn has_clock(&self, clock: ClockId) -> bool {
        self.clocks.contains_key(&clock)
    }

    /// A clock's parent. `None` for the universal clock and for an unknown
    /// clock.
    pub fn parent(&self, clock: ClockId) -> Option<ClockId> {
        self.clocks.get(&clock).and_then(|clock| clock.parent)
    }

    /// A clock's rate relative to its parent.
    pub fn rate(&self, clock: ClockId) -> Option<Real> {
        self.clocks.get(&clock).map(|clock| clock.rate)
    }

    /// A clock's effective rate: the product of rates from the root down
    /// to it. This is how fast its local time runs against universal time.
    pub fn effective_rate(&self, clock: ClockId) -> Option<Real> {
        self.clocks.get(&clock).map(|clock| clock.effective)
    }

    /// Every clock, in ascending order of id. The universal clock is
    /// first.
    pub fn clocks(&self) -> impl Iterator<Item = ClockId> + '_ {
        self.clocks.keys().copied()
    }

    // ---- Internals -------------------------------------------------------

    fn clock(&self, id: ClockId) -> Result<&Clock, TimeError> {
        self.clocks.get(&id).ok_or(TimeError::UnknownClock(id))
    }

    /// The local time a wake-up has left right now. Never negative.
    fn remaining_exact(&self, entry: &Entry) -> Real {
        let rate = self
            .clocks
            .get(&entry.clock)
            .map_or(Real::ZERO, |clock| clock.effective);
        let passed = local_elapsed(self.now.since(entry.as_of), rate);
        (entry.remaining - passed).max(Real::ZERO)
    }

    /// A clock and every clock under it, each parent before its children.
    fn subtree(&self, root: ClockId) -> Vec<ClockId> {
        let mut found = vec![root];
        let mut at = 0;
        while at < found.len() {
            if let Some(clock) = self.clocks.get(&found[at]) {
                found.extend(clock.children.iter().copied());
            }
            at += 1;
        }
        found
    }

    /// The first half of any change to effective rates. Brings the local
    /// time of the given clocks, and the remaining time of every wake-up
    /// on them, up to date at the rates in force until now, and takes
    /// those wake-ups out of the queue. Returns them, for
    /// [`requeue`](Timeline::requeue).
    fn bring_up_to_date(&mut self, clocks: &[ClockId]) -> Vec<WakeId> {
        let now = self.now;
        let mut moved = Vec::new();
        for id in clocks {
            let Some(clock) = self.clocks.get_mut(id) else {
                continue;
            };
            let (local, frac) = clock.local_at(now);
            clock.local = local;
            clock.frac = frac;
            clock.synced = now;

            for wake in &clock.wakes {
                let Some(entry) = self.wakes.get_mut(wake) else {
                    continue;
                };
                let passed = local_elapsed(now.since(entry.as_of), clock.effective);
                entry.remaining = (entry.remaining - passed).max(Real::ZERO);
                entry.as_of = now;
                if let Some(due) = entry.due.take() {
                    self.queue.remove(&(due, *wake));
                }
                moved.push(*wake);
            }
        }
        moved
    }

    /// Recomputes the effective rates of the given clocks, which must list
    /// each parent before its children.
    fn recompute_rates(&mut self, clocks: &[ClockId]) {
        for id in clocks {
            let parent_rate = self
                .clocks
                .get(id)
                .and_then(|clock| clock.parent)
                .and_then(|parent| self.clocks.get(&parent))
                .map_or(Real::ONE, |parent| parent.effective);
            if let Some(clock) = self.clocks.get_mut(id) {
                clock.effective = parent_rate * clock.rate;
            }
        }
    }

    /// The second half of any change to effective rates: gives each of the
    /// wake-ups a universal due tick from the local time it has left, and
    /// queues it unless its clock is stopped.
    fn requeue(&mut self, wakes: &[WakeId]) {
        for id in wakes {
            let Some(entry) = self.wakes.get_mut(id) else {
                continue;
            };
            let rate = self
                .clocks
                .get(&entry.clock)
                .map_or(Real::ZERO, |clock| clock.effective);
            entry.due = universal_wait(entry.remaining, rate).map(|wait| self.now + wait);
            if let Some(due) = entry.due {
                self.queue.insert((due, *id));
            }
        }
    }
}

impl Default for Timeline {
    fn default() -> Timeline {
        Timeline::new()
    }
}

/// A rate a clock may have: zero or more, and finite. Negative zero is
/// stored as zero.
fn checked_rate(rate: Real) -> Result<Real, TimeError> {
    if rate < Real::ZERO || !rate.is_finite() {
        Err(TimeError::BadRate(rate))
    } else if rate == Real::ZERO {
        Ok(Real::ZERO)
    } else {
        Ok(rate)
    }
}

/// Hashes the essential state: the time, the counters, every clock and
/// every wake-up. The queue and the indexes follow from those.
impl StateHash for Timeline {
    fn state_hash(&self, hasher: &mut StateHasher) {
        self.now.state_hash(hasher);
        hasher.write_u64(self.next_wake);
        hasher.write_u32(self.next_clock);

        hasher.write_len(self.clocks.len());
        for (id, clock) in &self.clocks {
            id.state_hash(hasher);
            clock.parent.state_hash(hasher);
            clock.rate.state_hash(hasher);
            clock.local.state_hash(hasher);
            clock.frac.state_hash(hasher);
            clock.synced.state_hash(hasher);
        }

        hasher.write_len(self.wakes.len());
        for (id, entry) in self.wakes.sorted_entries() {
            id.state_hash(hasher);
            entry.clock.state_hash(hasher);
            entry.remaining.state_hash(hasher);
            entry.as_of.state_hash(hasher);
            entry.due.state_hash(hasher);
            entry.wake.state_hash(hasher);
        }
    }
}

// ---- Saving and loading ------------------------------------------------

/// The saved form of a [`Timeline`]: its essential state, without the
/// queue and the indexes.
#[derive(Serialize, Deserialize)]
struct TimelineData {
    now: Tick,
    next_wake: u64,
    next_clock: u32,
    clocks: Vec<ClockData>,
    wakes: Vec<WakeData>,
}

#[derive(Serialize, Deserialize)]
struct ClockData {
    id: ClockId,
    parent: Option<ClockId>,
    rate: Real,
    local: Dur,
    frac: Real,
    synced: Tick,
}

#[derive(Serialize, Deserialize)]
struct WakeData {
    id: WakeId,
    clock: ClockId,
    remaining: Real,
    as_of: Tick,
    due: Option<Tick>,
    wake: Wake,
}

impl From<&Timeline> for TimelineData {
    fn from(timeline: &Timeline) -> TimelineData {
        TimelineData {
            now: timeline.now,
            next_wake: timeline.next_wake,
            next_clock: timeline.next_clock,
            clocks: timeline
                .clocks
                .iter()
                .map(|(id, clock)| ClockData {
                    id: *id,
                    parent: clock.parent,
                    rate: clock.rate,
                    local: clock.local,
                    frac: clock.frac,
                    synced: clock.synced,
                })
                .collect(),
            wakes: timeline
                .wakes
                .sorted_entries()
                .into_iter()
                .map(|(id, entry)| WakeData {
                    id: *id,
                    clock: entry.clock,
                    remaining: entry.remaining,
                    as_of: entry.as_of,
                    due: entry.due,
                    wake: entry.wake,
                })
                .collect(),
        }
    }
}

/// Rebuilds a timeline from saved state, checking that the state is one a
/// timeline could have been in. The error is a message for a person.
impl TryFrom<TimelineData> for Timeline {
    type Error = String;

    fn try_from(data: TimelineData) -> Result<Timeline, String> {
        let mut clocks: BTreeMap<ClockId, Clock> = BTreeMap::new();
        for saved in &data.clocks {
            if saved.id.raw() >= data.next_clock {
                return Err(format!("clock `{}` is beyond the next clock id", saved.id));
            }
            if saved.synced > data.now {
                return Err(format!("clock `{}` is ahead of the current time", saved.id));
            }
            if saved.frac < Real::ZERO || saved.frac >= Real::ONE {
                return Err(format!("clock `{}` has a bad fraction", saved.id));
            }
            let is_root = saved.id == ClockId::UNIVERSAL;
            if is_root != saved.parent.is_none() {
                return Err(format!("clock `{}` has the wrong kind of parent", saved.id));
            }
            if is_root && saved.rate != Real::ONE {
                return Err("the universal clock's rate is not 1".to_owned());
            }
            let rate = checked_rate(saved.rate).map_err(|error| error.to_string())?;
            let clock = Clock {
                parent: saved.parent,
                rate,
                local: saved.local,
                frac: saved.frac,
                synced: saved.synced,
                effective: Real::ZERO,
                children: Vec::new(),
                wakes: BTreeSet::new(),
            };
            if clocks.insert(saved.id, clock).is_some() {
                return Err(format!("clock `{}` appears twice", saved.id));
            }
        }
        if !clocks.contains_key(&ClockId::UNIVERSAL) {
            return Err("the universal clock is missing".to_owned());
        }

        // Children, then effective rates from the root down. A clock that
        // the walk does not reach hangs from a loop.
        for saved in &data.clocks {
            let Some(parent) = saved.parent else { continue };
            let Some(parent_clock) = clocks.get_mut(&parent) else {
                return Err(format!(
                    "clock `{}` has an unknown parent `{parent}`",
                    saved.id
                ));
            };
            clash_core::sorted::insert(&mut parent_clock.children, saved.id);
        }
        let mut order = vec![ClockId::UNIVERSAL];
        let mut at = 0;
        while at < order.len() {
            if let Some(clock) = clocks.get(&order[at]) {
                order.extend(clock.children.iter().copied());
            }
            at += 1;
        }
        if order.len() != clocks.len() {
            return Err("the clocks do not form a tree".to_owned());
        }

        let mut wakes: LookupMap<WakeId, Entry> = LookupMap::new();
        let mut queue = BTreeSet::new();
        for saved in &data.wakes {
            if saved.id.raw() >= data.next_wake {
                return Err(format!(
                    "wake-up `{}` is beyond the next wake-up id",
                    saved.id
                ));
            }
            if saved.as_of > data.now || saved.due.is_some_and(|due| due < data.now) {
                return Err(format!(
                    "wake-up `{}` is out of step with the current time",
                    saved.id
                ));
            }
            if saved.remaining < Real::ZERO {
                return Err(format!("wake-up `{}` has negative time left", saved.id));
            }
            let Some(clock) = clocks.get_mut(&saved.clock) else {
                return Err(format!(
                    "wake-up `{}` is on an unknown clock `{}`",
                    saved.id, saved.clock
                ));
            };
            clock.wakes.insert(saved.id);
            if let Some(due) = saved.due {
                queue.insert((due, saved.id));
            }
            let entry = Entry {
                clock: saved.clock,
                remaining: saved.remaining,
                as_of: saved.as_of,
                due: saved.due,
                wake: saved.wake,
            };
            if wakes.insert(saved.id, entry).is_some() {
                return Err(format!("wake-up `{}` appears twice", saved.id));
            }
        }

        let mut timeline = Timeline {
            now: data.now,
            next_wake: data.next_wake,
            next_clock: data.next_clock,
            clocks,
            wakes,
            queue,
        };
        timeline.recompute_rates(&order);
        Ok(timeline)
    }
}

impl Serialize for Timeline {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        TimelineData::from(self).serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for Timeline {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Timeline, D::Error> {
        let data = TimelineData::deserialize(deserializer)?;
        Timeline::try_from(data).map_err(serde::de::Error::custom)
    }
}
