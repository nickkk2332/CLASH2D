//! Property tests that drive a `Timeline` with random scripts of
//! operations and compare everything an observer can see with two
//! independent references:
//!
//! - **The naive model** follows the rules of
//!   `docs/03-time-and-actions.md` section 3.3 with the simplest possible
//!   bookkeeping: plain lists, linear scans, no queue, no cached rates, no
//!   indexes. It checks the timeline's bookkeeping under every kind of
//!   rate.
//! - **The tick oracle** shares no arithmetic with the timeline at all. It
//!   steps one tick at a time in integers, takes each clock's rate off
//!   every wake-up on it, and fires a wake-up when its time runs out. With
//!   rates that are powers of two the timeline's arithmetic is exact, so
//!   the two must agree to the tick. It checks that the timeline's
//!   arithmetic means what the specification says.
//!
//! A third test saves and loads the timeline in the middle of a script
//! and requires the rest of the run to be identical.

use clash_core::{Dur, Hash128, Real, Tick};
use clash_time::{ClockId, Timeline, Wake, WakeDomain, WakeId};
use proptest::prelude::*;

// ---- Scripts -------------------------------------------------------------

/// One step of a script. Indices choose among whatever clocks and
/// wake-ups exist at the time, so most steps are valid; the rest exercise
/// refusals.
#[derive(Clone, Debug)]
enum Op {
    /// Run everything due in the next `0` ticks, then move time on.
    Advance(i64),
    Schedule {
        clock: usize,
        after: i64,
    },
    Cancel {
        wake: usize,
    },
    Rehome {
        wake: usize,
        clock: usize,
    },
    CreateClock {
        parent: usize,
        rate: usize,
    },
    SetRate {
        clock: usize,
        rate: usize,
    },
    Reparent {
        clock: usize,
        parent: usize,
    },
    DestroyClock {
        clock: usize,
    },
}

fn op() -> impl Strategy<Value = Op> {
    let index = || 0..64_usize;
    prop_oneof![
        5 => (0..200_i64).prop_map(Op::Advance),
        6 => (index(), -3..400_i64).prop_map(|(clock, after)| Op::Schedule { clock, after }),
        1 => index().prop_map(|wake| Op::Cancel { wake }),
        2 => (index(), index()).prop_map(|(wake, clock)| Op::Rehome { wake, clock }),
        2 => (index(), index()).prop_map(|(parent, rate)| Op::CreateClock { parent, rate }),
        4 => (index(), index()).prop_map(|(clock, rate)| Op::SetRate { clock, rate }),
        2 => (index(), index()).prop_map(|(clock, parent)| Op::Reparent { clock, parent }),
        1 => index().prop_map(|clock| Op::DestroyClock { clock }),
    ]
}

fn script() -> impl Strategy<Value = Vec<Op>> {
    prop::collection::vec(op(), 1..60)
}

/// Rates of every kind: stopped, slow, fast, and not representable
/// exactly in binary.
const ANY_RATES: [f64; 8] = [0.0, 0.3, 0.5, 1.0, 1.7, 365.0, 1e-3, 2.5];

/// Rates that are powers of two (or zero), for which the timeline's
/// arithmetic is exact. Each is a whole number of quarters.
const DYADIC_RATES: [f64; 6] = [0.0, 0.25, 0.5, 1.0, 2.0, 4.0];

// ---- The common interface ---------------------------------------------------

/// What the script runner needs from an implementation.
trait Subject {
    fn now(&self) -> i64;
    fn clocks(&self) -> Vec<u32>;
    fn wakes(&self) -> Vec<u64>;
    fn parent(&self, clock: u32) -> Option<u32>;
    fn schedule(&mut self, clock: u32, after: i64, payload: u64) -> Option<u64>;
    fn cancel(&mut self, wake: u64) -> bool;
    fn rehome(&mut self, wake: u64, to: u32) -> bool;
    fn create_clock(&mut self, parent: u32, rate: f64) -> Option<u32>;
    fn set_rate(&mut self, clock: u32, rate: f64) -> bool;
    fn reparent(&mut self, clock: u32, parent: u32) -> bool;
    fn destroy_clock(&mut self, clock: u32) -> bool;
    /// The next wake-up due by `limit`: its id and payload.
    fn pop(&mut self, limit: i64) -> Option<(u64, u64)>;
    fn advance_to(&mut self, tick: i64) -> i64;
    fn remaining(&self, wake: u64) -> Option<i64>;
    fn local_now(&self, clock: u32) -> Option<i64>;
    /// Details only some implementations can report: each wake-up's due
    /// tick and each clock's effective rate, as text.
    fn details(&self) -> Option<String>;
}

fn depth(subject: &impl Subject, clock: u32) -> usize {
    let mut steps = 0;
    let mut at = clock;
    while let Some(parent) = subject.parent(at) {
        at = parent;
        steps += 1;
    }
    steps
}

/// How far the deepest clock under `clock` is below it.
fn height(subject: &impl Subject, clock: u32) -> usize {
    let base = depth(subject, clock);
    subject
        .clocks()
        .into_iter()
        .filter(|other| {
            let mut at = *other;
            loop {
                if at == clock {
                    return true;
                }
                match subject.parent(at) {
                    Some(parent) => at = parent,
                    None => return false,
                }
            }
        })
        .map(|other| depth(subject, other) - base)
        .max()
        .unwrap_or(0)
}

/// Runs a script and returns a trace of everything observable: what each
/// step did, and after each step the time, every clock's local time and
/// every wake-up's remaining time.
///
/// `max_depth` skips steps that would make the clock tree deeper than
/// that. `before` is called ahead of each step with its index.
fn run<S: Subject>(
    subject: &mut S,
    script: &[Op],
    rates: &[f64],
    max_depth: Option<usize>,
    mut before: impl FnMut(usize, &mut S),
) -> Vec<String> {
    let mut trace = Vec::new();
    let mut payload = 0_u64;
    for (index, op) in script.iter().enumerate() {
        before(index, subject);
        let clocks = subject.clocks();
        let wakes = subject.wakes();
        let pick_clock = |n: usize| clocks[n % clocks.len()];
        let pick_wake = |n: usize| (!wakes.is_empty()).then(|| wakes[n % wakes.len()]);

        let line = match *op {
            Op::Advance(ticks) => {
                let limit = subject.now() + ticks;
                let mut fired = Vec::new();
                while let Some((id, payload)) = subject.pop(limit) {
                    fired.push(format!("{id}/{payload}@{}", subject.now()));
                }
                let now = subject.advance_to(limit);
                format!("advance to {now}: fired [{}]", fired.join(", "))
            }
            Op::Schedule { clock, after } => {
                payload += 1;
                let clock = pick_clock(clock);
                let id = subject.schedule(clock, after, payload);
                format!("schedule on {clock} after {after}: {id:?}")
            }
            Op::Cancel { wake } => match pick_wake(wake) {
                Some(wake) => format!("cancel {wake}: {}", subject.cancel(wake)),
                // Nothing to cancel: try an id that does not exist.
                None => format!("cancel nothing: {}", subject.cancel(u64::MAX)),
            },
            Op::Rehome { wake, clock } => match pick_wake(wake) {
                Some(wake) => {
                    let to = pick_clock(clock);
                    format!("rehome {wake} to {to}: {}", subject.rehome(wake, to))
                }
                None => format!("rehome nothing: {}", subject.rehome(u64::MAX, 0)),
            },
            Op::CreateClock { parent, rate } => {
                let parent = pick_clock(parent);
                let rate = rates[rate % rates.len()];
                if max_depth.is_some_and(|max| depth(subject, parent) + 1 > max) {
                    "create clock: skipped".to_owned()
                } else {
                    let id = subject.create_clock(parent, rate);
                    format!("create clock under {parent} at {rate}: {id:?}")
                }
            }
            Op::SetRate { clock, rate } => {
                let clock = pick_clock(clock);
                let rate = rates[rate % rates.len()];
                format!(
                    "set rate of {clock} to {rate}: {}",
                    subject.set_rate(clock, rate)
                )
            }
            Op::Reparent { clock, parent } => {
                let clock = pick_clock(clock);
                let parent = pick_clock(parent);
                let too_deep = max_depth
                    .is_some_and(|max| depth(subject, parent) + 1 + height(subject, clock) > max);
                if too_deep {
                    "reparent: skipped".to_owned()
                } else {
                    format!(
                        "reparent {clock} under {parent}: {}",
                        subject.reparent(clock, parent)
                    )
                }
            }
            Op::DestroyClock { clock } => {
                let clock = pick_clock(clock);
                format!("destroy clock {clock}: {}", subject.destroy_clock(clock))
            }
        };
        trace.push(line);

        let locals: Vec<String> = subject
            .clocks()
            .into_iter()
            .map(|clock| {
                format!(
                    "{clock}^{:?}={:?}",
                    subject.parent(clock),
                    subject.local_now(clock)
                )
            })
            .collect();
        let remaining: Vec<String> = subject
            .wakes()
            .into_iter()
            .map(|wake| format!("{wake}:{:?}", subject.remaining(wake)))
            .collect();
        trace.push(format!(
            "  now {} clocks [{}] wakes [{}]",
            subject.now(),
            locals.join(" "),
            remaining.join(" ")
        ));
        if let Some(details) = subject.details() {
            trace.push(format!("  {details}"));
        }
    }
    trace
}

// ---- The timeline, behind the common interface --------------------------------

fn payload_wake(payload: u64) -> Wake {
    Wake::new(WakeDomain::Rule, payload, payload.wrapping_mul(3))
}

impl Subject for Timeline {
    fn now(&self) -> i64 {
        Timeline::now(self).as_micros()
    }

    fn clocks(&self) -> Vec<u32> {
        Timeline::clocks(self).map(ClockId::raw).collect()
    }

    fn wakes(&self) -> Vec<u64> {
        let mut wakes: Vec<u64> = Timeline::clocks(self)
            .flat_map(|clock| self.wakes_on(clock))
            .map(WakeId::raw)
            .collect();
        wakes.sort_unstable();
        assert_eq!(wakes.len(), self.wake_count());
        wakes
    }

    fn parent(&self, clock: u32) -> Option<u32> {
        Timeline::parent(self, ClockId::new(clock)).map(ClockId::raw)
    }

    fn schedule(&mut self, clock: u32, after: i64, payload: u64) -> Option<u64> {
        Timeline::schedule(self, ClockId::new(clock), Dur(after), payload_wake(payload))
            .ok()
            .map(WakeId::raw)
    }

    fn cancel(&mut self, wake: u64) -> bool {
        Timeline::cancel(self, WakeId::new(wake))
    }

    fn rehome(&mut self, wake: u64, to: u32) -> bool {
        Timeline::rehome(self, WakeId::new(wake), ClockId::new(to)).is_ok()
    }

    fn create_clock(&mut self, parent: u32, rate: f64) -> Option<u32> {
        Timeline::create_clock(self, ClockId::new(parent), Real::of(rate))
            .ok()
            .map(ClockId::raw)
    }

    fn set_rate(&mut self, clock: u32, rate: f64) -> bool {
        Timeline::set_rate(self, ClockId::new(clock), Real::of(rate)).is_ok()
    }

    fn reparent(&mut self, clock: u32, parent: u32) -> bool {
        Timeline::reparent(self, ClockId::new(clock), ClockId::new(parent)).is_ok()
    }

    fn destroy_clock(&mut self, clock: u32) -> bool {
        Timeline::destroy_clock(self, ClockId::new(clock)).is_ok()
    }

    fn pop(&mut self, limit: i64) -> Option<(u64, u64)> {
        let before = Timeline::now(self);
        let due = self.peek();
        let (id, wake) = Timeline::pop(self, Tick(limit))?;
        // What pop promises: the payload is intact, time moved to the due
        // tick that peek announced, and never backwards.
        assert_eq!(wake.arg, wake.id.wrapping_mul(3));
        assert_eq!(wake.domain, WakeDomain::Rule);
        assert_eq!(Some(Timeline::now(self)), due);
        assert!(Timeline::now(self) >= before);
        Some((id.raw(), wake.id))
    }

    fn advance_to(&mut self, tick: i64) -> i64 {
        Timeline::advance_to(self, Tick(tick)).as_micros()
    }

    fn remaining(&self, wake: u64) -> Option<i64> {
        Timeline::remaining(self, WakeId::new(wake)).map(Dur::as_micros)
    }

    fn local_now(&self, clock: u32) -> Option<i64> {
        Timeline::local_now(self, ClockId::new(clock))
            .ok()
            .map(Dur::as_micros)
    }

    fn details(&self) -> Option<String> {
        let rates: Vec<String> = Timeline::clocks(self)
            .map(|clock| {
                let rate = self.effective_rate(clock).map(|rate| rate.get().to_bits());
                format!("{clock}x{rate:?}")
            })
            .collect();
        let dues: Vec<String> = Subject::wakes(self)
            .into_iter()
            .map(|wake| {
                let id = WakeId::new(wake);
                let due = self.due(id).map(Tick::as_micros);
                assert_eq!(self.is_parked(id), due.is_none());
                format!("{wake}@{due:?}on{:?}", self.clock_of(id).map(ClockId::raw))
            })
            .collect();
        Some(format!(
            "rates [{}] dues [{}] next {:?}",
            rates.join(" "),
            dues.join(" "),
            self.peek().map(Tick::as_micros)
        ))
    }
}

// ---- The naive model ----------------------------------------------------------------

#[derive(Clone, Debug)]
struct ModelClock {
    id: u32,
    parent: Option<u32>,
    rate: Real,
    local: i64,
    frac: Real,
    synced: i64,
}

#[derive(Clone, Debug)]
struct ModelWake {
    id: u64,
    clock: u32,
    remaining: Real,
    as_of: i64,
    due: Option<i64>,
    payload: u64,
}

/// The rules of 03 §3.3 with no cleverness: every question is answered by
/// scanning the lists.
#[derive(Clone, Debug)]
struct Model {
    now: i64,
    next_wake: u64,
    next_clock: u32,
    clocks: Vec<ModelClock>,
    wakes: Vec<ModelWake>,
}

impl Model {
    fn new() -> Model {
        Model {
            now: 0,
            next_wake: 0,
            next_clock: 1,
            clocks: vec![ModelClock {
                id: 0,
                parent: None,
                rate: Real::ONE,
                local: 0,
                frac: Real::ZERO,
                synced: 0,
            }],
            wakes: Vec::new(),
        }
    }

    fn find(&self, clock: u32) -> Option<&ModelClock> {
        self.clocks.iter().find(|candidate| candidate.id == clock)
    }

    /// The product of rates from the root down, worked out afresh.
    fn effective(&self, clock: u32) -> Real {
        let Some(found) = self.find(clock) else {
            return Real::ZERO;
        };
        match found.parent {
            None => Real::ONE,
            Some(parent) => self.effective(parent) * found.rate,
        }
    }

    fn is_under(&self, clock: u32, ancestor: u32) -> bool {
        let mut at = clock;
        loop {
            if at == ancestor {
                return true;
            }
            match self.find(at).and_then(|found| found.parent) {
                Some(parent) => at = parent,
                None => return false,
            }
        }
    }

    fn local_at_now(&self, clock: &ModelClock) -> (i64, Real) {
        let span = self.now - clock.synced;
        let rate = self.effective(clock.id);
        if rate == Real::ONE {
            return (clock.local + span, clock.frac);
        }
        let total = clock.frac + Real::from_i64(span) * rate;
        let whole = total.floor();
        (clock.local + whole.to_i64(), total - whole)
    }

    fn left(&self, wake: &ModelWake) -> Real {
        let passed = Real::from_i64(self.now - wake.as_of) * self.effective(wake.clock);
        (wake.remaining - passed).max(Real::ZERO)
    }

    fn wait(&self, remaining: Real, clock: u32) -> Option<i64> {
        let rate = self.effective(clock);
        if remaining <= Real::ZERO {
            Some(self.now)
        } else if rate <= Real::ZERO {
            None
        } else {
            Some(self.now.saturating_add((remaining / rate).ceil().to_i64()))
        }
    }

    /// "The clock and all its descendants first bring their local time up
    /// to date at the old rate."
    fn bring_up_to_date(&mut self, root: u32) -> Vec<u32> {
        let affected: Vec<u32> = self
            .clocks
            .iter()
            .map(|clock| clock.id)
            .filter(|id| self.is_under(*id, root))
            .collect();
        let locals: Vec<(u32, (i64, Real))> = affected
            .iter()
            .filter_map(|id| self.find(*id))
            .map(|clock| (clock.id, self.local_at_now(clock)))
            .collect();
        let lefts: Vec<(u64, Real)> = self
            .wakes
            .iter()
            .filter(|wake| affected.contains(&wake.clock))
            .map(|wake| (wake.id, self.left(wake)))
            .collect();
        for (id, (local, frac)) in locals {
            if let Some(clock) = self.clocks.iter_mut().find(|clock| clock.id == id) {
                clock.local = local;
                clock.frac = frac;
                clock.synced = self.now;
            }
        }
        for (id, left) in lefts {
            if let Some(wake) = self.wakes.iter_mut().find(|wake| wake.id == id) {
                wake.remaining = left;
                wake.as_of = self.now;
            }
        }
        affected
    }

    /// "Then every wake-up on them gets a new universal due tick from its
    /// remaining local time."
    fn give_new_due_ticks(&mut self, wakes: &[u64]) {
        let dues: Vec<(u64, Option<i64>)> = self
            .wakes
            .iter()
            .filter(|wake| wakes.contains(&wake.id))
            .map(|wake| (wake.id, self.wait(wake.remaining, wake.clock)))
            .collect();
        for (id, due) in dues {
            if let Some(wake) = self.wakes.iter_mut().find(|wake| wake.id == id) {
                wake.due = due;
            }
        }
    }

    fn wakes_on(&self, clocks: &[u32]) -> Vec<u64> {
        self.wakes
            .iter()
            .filter(|wake| clocks.contains(&wake.clock))
            .map(|wake| wake.id)
            .collect()
    }

    fn rate_allowed(rate: f64) -> Option<Real> {
        if rate < 0.0 || !rate.is_finite() {
            None
        } else if rate == 0.0 {
            Some(Real::ZERO)
        } else {
            Some(Real::of(rate))
        }
    }
}

impl Subject for Model {
    fn now(&self) -> i64 {
        self.now
    }

    fn clocks(&self) -> Vec<u32> {
        let mut ids: Vec<u32> = self.clocks.iter().map(|clock| clock.id).collect();
        ids.sort_unstable();
        ids
    }

    fn wakes(&self) -> Vec<u64> {
        let mut ids: Vec<u64> = self.wakes.iter().map(|wake| wake.id).collect();
        ids.sort_unstable();
        ids
    }

    fn parent(&self, clock: u32) -> Option<u32> {
        self.find(clock).and_then(|found| found.parent)
    }

    fn schedule(&mut self, clock: u32, after: i64, payload: u64) -> Option<u64> {
        self.find(clock)?;
        let id = self.next_wake;
        self.next_wake += 1;
        let remaining = Real::from_i64(after.max(0));
        let due = self.wait(remaining, clock);
        self.wakes.push(ModelWake {
            id,
            clock,
            remaining,
            as_of: self.now,
            due,
            payload,
        });
        Some(id)
    }

    fn cancel(&mut self, wake: u64) -> bool {
        let before = self.wakes.len();
        self.wakes.retain(|candidate| candidate.id != wake);
        self.wakes.len() != before
    }

    fn rehome(&mut self, wake: u64, to: u32) -> bool {
        if self.find(to).is_none() {
            return false;
        }
        let Some(found) = self.wakes.iter().find(|candidate| candidate.id == wake) else {
            return false;
        };
        if found.clock == to {
            return true;
        }
        let left = self.left(found);
        let due = self.wait(left, to);
        let now = self.now;
        if let Some(found) = self.wakes.iter_mut().find(|candidate| candidate.id == wake) {
            found.clock = to;
            found.remaining = left;
            found.as_of = now;
            found.due = due;
        }
        true
    }

    fn create_clock(&mut self, parent: u32, rate: f64) -> Option<u32> {
        let rate = Model::rate_allowed(rate)?;
        self.find(parent)?;
        let id = self.next_clock;
        self.next_clock += 1;
        self.clocks.push(ModelClock {
            id,
            parent: Some(parent),
            rate,
            local: 0,
            frac: Real::ZERO,
            synced: self.now,
        });
        Some(id)
    }

    fn set_rate(&mut self, clock: u32, rate: f64) -> bool {
        if clock == 0 {
            return false;
        }
        let Some(rate) = Model::rate_allowed(rate) else {
            return false;
        };
        let Some(found) = self.find(clock) else {
            return false;
        };
        if found.rate == rate {
            return true;
        }
        let affected = self.bring_up_to_date(clock);
        if let Some(found) = self.clocks.iter_mut().find(|found| found.id == clock) {
            found.rate = rate;
        }
        let wakes = self.wakes_on(&affected);
        self.give_new_due_ticks(&wakes);
        true
    }

    fn reparent(&mut self, clock: u32, parent: u32) -> bool {
        if clock == 0 {
            return false;
        }
        let Some(found) = self.find(clock) else {
            return false;
        };
        if self.find(parent).is_none() {
            return false;
        }
        if found.parent == Some(parent) {
            return true;
        }
        if self.is_under(parent, clock) {
            return false;
        }
        let affected = self.bring_up_to_date(clock);
        if let Some(found) = self.clocks.iter_mut().find(|found| found.id == clock) {
            found.parent = Some(parent);
        }
        let wakes = self.wakes_on(&affected);
        self.give_new_due_ticks(&wakes);
        true
    }

    fn destroy_clock(&mut self, clock: u32) -> bool {
        if clock == 0 {
            return false;
        }
        let Some(heir) = self.find(clock).and_then(|found| found.parent) else {
            return false;
        };
        let affected = self.bring_up_to_date(clock);
        let wakes = self.wakes_on(&affected);
        for wake in self.wakes.iter_mut().filter(|wake| wake.clock == clock) {
            wake.clock = heir;
        }
        for child in self
            .clocks
            .iter_mut()
            .filter(|child| child.parent == Some(clock))
        {
            child.parent = Some(heir);
        }
        self.clocks.retain(|candidate| candidate.id != clock);
        self.give_new_due_ticks(&wakes);
        true
    }

    fn pop(&mut self, limit: i64) -> Option<(u64, u64)> {
        let (due, id) = self
            .wakes
            .iter()
            .filter_map(|wake| wake.due.map(|due| (due, wake.id)))
            .min()?;
        if due > limit {
            return None;
        }
        let at = self.wakes.iter().position(|wake| wake.id == id)?;
        let wake = self.wakes.remove(at);
        self.now = self.now.max(due);
        Some((id, wake.payload))
    }

    fn advance_to(&mut self, tick: i64) -> i64 {
        let next = self.wakes.iter().filter_map(|wake| wake.due).min();
        let target = next.map_or(tick, |due| tick.min(due));
        self.now = self.now.max(target);
        self.now
    }

    fn remaining(&self, wake: u64) -> Option<i64> {
        let found = self.wakes.iter().find(|candidate| candidate.id == wake)?;
        Some(self.left(found).ceil().to_i64())
    }

    fn local_now(&self, clock: u32) -> Option<i64> {
        self.find(clock).map(|found| self.local_at_now(found).0)
    }

    fn details(&self) -> Option<String> {
        let rates: Vec<String> = Subject::clocks(self)
            .into_iter()
            .map(|clock| {
                let rate = Some(self.effective(clock).get().to_bits());
                format!("{clock}x{rate:?}")
            })
            .collect();
        let dues: Vec<String> = Subject::wakes(self)
            .into_iter()
            .filter_map(|id| self.wakes.iter().find(|wake| wake.id == id))
            .map(|wake| format!("{}@{:?}on{:?}", wake.id, wake.due, Some(wake.clock)))
            .collect();
        let next = self.wakes.iter().filter_map(|wake| wake.due).min();
        Some(format!(
            "rates [{}] dues [{}] next {next:?}",
            rates.join(" "),
            dues.join(" ")
        ))
    }
}

// ---- The tick oracle ------------------------------------------------------------------

/// Local time is counted in 64ths of a microsecond, so that every product
/// of up to three dyadic rates is a whole number of units per tick.
const UNITS: i64 = 64;
/// The deepest clock tree the oracle's units can express.
const ORACLE_DEPTH: usize = 3;

#[derive(Clone, Debug)]
struct OracleClock {
    parent: Option<u32>,
    /// The rate in quarters: 0, 1, 2, 4, 8 or 16.
    quarters: i64,
    /// Local time since creation, in units.
    local: i64,
}

#[derive(Clone, Debug)]
struct OracleWake {
    clock: u32,
    /// Local time left, in units.
    left: i64,
    payload: u64,
}

/// Time, one tick at a time, in integers.
#[derive(Clone, Debug)]
struct Oracle {
    now: i64,
    next_wake: u64,
    next_clock: u32,
    clocks: std::collections::BTreeMap<u32, OracleClock>,
    wakes: std::collections::BTreeMap<u64, OracleWake>,
}

impl Oracle {
    fn new() -> Oracle {
        let mut clocks = std::collections::BTreeMap::new();
        clocks.insert(
            0,
            OracleClock {
                parent: None,
                quarters: 4,
                local: 0,
            },
        );
        Oracle {
            now: 0,
            next_wake: 0,
            next_clock: 1,
            clocks,
            wakes: std::collections::BTreeMap::new(),
        }
    }

    /// Units of local time per tick on a clock.
    fn speed(&self, clock: u32) -> i64 {
        let mut product = UNITS;
        let mut at = clock;
        while let Some(found) = self.clocks.get(&at) {
            let Some(parent) = found.parent else { break };
            product = product * found.quarters / 4;
            at = parent;
        }
        product
    }

    fn quarters(rate: f64) -> i64 {
        (rate * 4.0) as i64
    }

    /// One tick passes everywhere.
    fn step(&mut self) {
        self.now += 1;
        let speeds: Vec<(u32, i64)> = self
            .clocks
            .keys()
            .map(|clock| (*clock, self.speed(*clock)))
            .collect();
        for (clock, speed) in &speeds {
            if let Some(found) = self.clocks.get_mut(clock) {
                found.local += speed;
            }
        }
        for wake in self.wakes.values_mut() {
            let speed = speeds
                .iter()
                .find(|(clock, _)| *clock == wake.clock)
                .map_or(0, |(_, speed)| *speed);
            wake.left -= speed;
        }
    }

    /// The earliest-scheduled wake-up whose time has run out.
    fn take_due(&mut self) -> Option<(u64, u64)> {
        let id = self
            .wakes
            .iter()
            .find(|(_, wake)| wake.left <= 0)
            .map(|(id, _)| *id)?;
        let wake = self.wakes.remove(&id)?;
        Some((id, wake.payload))
    }
}

impl Subject for Oracle {
    fn now(&self) -> i64 {
        self.now
    }

    fn clocks(&self) -> Vec<u32> {
        self.clocks.keys().copied().collect()
    }

    fn wakes(&self) -> Vec<u64> {
        self.wakes.keys().copied().collect()
    }

    fn parent(&self, clock: u32) -> Option<u32> {
        self.clocks.get(&clock).and_then(|found| found.parent)
    }

    fn schedule(&mut self, clock: u32, after: i64, payload: u64) -> Option<u64> {
        self.clocks.get(&clock)?;
        let id = self.next_wake;
        self.next_wake += 1;
        self.wakes.insert(
            id,
            OracleWake {
                clock,
                left: after.max(0) * UNITS,
                payload,
            },
        );
        Some(id)
    }

    fn cancel(&mut self, wake: u64) -> bool {
        self.wakes.remove(&wake).is_some()
    }

    fn rehome(&mut self, wake: u64, to: u32) -> bool {
        if !self.clocks.contains_key(&to) {
            return false;
        }
        match self.wakes.get_mut(&wake) {
            Some(found) => {
                found.clock = to;
                true
            }
            None => false,
        }
    }

    fn create_clock(&mut self, parent: u32, rate: f64) -> Option<u32> {
        self.clocks.get(&parent)?;
        let id = self.next_clock;
        self.next_clock += 1;
        self.clocks.insert(
            id,
            OracleClock {
                parent: Some(parent),
                quarters: Oracle::quarters(rate),
                local: 0,
            },
        );
        Some(id)
    }

    fn set_rate(&mut self, clock: u32, rate: f64) -> bool {
        if clock == 0 {
            return false;
        }
        match self.clocks.get_mut(&clock) {
            Some(found) => {
                found.quarters = Oracle::quarters(rate);
                true
            }
            None => false,
        }
    }

    fn reparent(&mut self, clock: u32, parent: u32) -> bool {
        if clock == 0 || !self.clocks.contains_key(&clock) || !self.clocks.contains_key(&parent) {
            return false;
        }
        // Refuse a loop: the new parent may not be the clock or under it.
        let mut at = Some(parent);
        while let Some(id) = at {
            if id == clock {
                return false;
            }
            at = self.parent(id);
        }
        if let Some(found) = self.clocks.get_mut(&clock) {
            found.parent = Some(parent);
        }
        true
    }

    fn destroy_clock(&mut self, clock: u32) -> bool {
        if clock == 0 {
            return false;
        }
        let Some(heir) = self.parent(clock) else {
            return false;
        };
        self.clocks.remove(&clock);
        for child in self.clocks.values_mut() {
            if child.parent == Some(clock) {
                child.parent = Some(heir);
            }
        }
        for wake in self.wakes.values_mut() {
            if wake.clock == clock {
                wake.clock = heir;
            }
        }
        true
    }

    fn pop(&mut self, limit: i64) -> Option<(u64, u64)> {
        loop {
            if let Some(fired) = self.take_due() {
                return Some(fired);
            }
            if self.now >= limit {
                return None;
            }
            self.step();
        }
    }

    fn advance_to(&mut self, tick: i64) -> i64 {
        while self.now < tick && !self.wakes.values().any(|wake| wake.left <= 0) {
            self.step();
        }
        self.now
    }

    fn remaining(&self, wake: u64) -> Option<i64> {
        let left = self.wakes.get(&wake)?.left.max(0);
        Some((left + UNITS - 1) / UNITS)
    }

    fn local_now(&self, clock: u32) -> Option<i64> {
        self.clocks.get(&clock).map(|found| found.local / UNITS)
    }

    fn details(&self) -> Option<String> {
        None
    }
}

/// The trace without the lines only some implementations produce.
fn without_details(trace: Vec<String>) -> Vec<String> {
    trace
        .into_iter()
        .filter(|line| !line.starts_with("  rates"))
        .collect()
}

fn reload(timeline: &Timeline) -> Timeline {
    let json = serde_json::to_string(timeline).expect("a timeline serialises");
    serde_json::from_str(&json).expect("a saved timeline loads")
}

// ---- The properties ---------------------------------------------------------------------

proptest! {
    #![proptest_config(ProptestConfig::with_cases(400))]

    /// Random schedule, cancel, rehome, rate, reparent and destroy
    /// operations give the same pops, times, remaining times, due ticks
    /// and rates as the naive model.
    #[test]
    fn the_timeline_matches_the_naive_model(script in script()) {
        let mut timeline = Timeline::new();
        let mut model = Model::new();
        let from_timeline = run(&mut timeline, &script, &ANY_RATES, None, |_, _| {});
        let from_model = run(&mut model, &script, &ANY_RATES, None, |_, _| {});
        prop_assert_eq!(from_timeline, from_model);
    }

    /// With exact rates, every wake-up fires at exactly the tick at which
    /// its local time runs out, counted one tick at a time, and remaining
    /// and local times agree throughout (invariants T1, T2 and T3).
    #[test]
    fn the_timeline_matches_tick_by_tick_counting(script in script()) {
        let mut timeline = Timeline::new();
        let mut oracle = Oracle::new();
        let depth = Some(ORACLE_DEPTH);
        let from_timeline = run(&mut timeline, &script, &DYADIC_RATES, depth, |_, _| {});
        let from_oracle = run(&mut oracle, &script, &DYADIC_RATES, depth, |_, _| {});
        prop_assert_eq!(without_details(from_timeline), from_oracle);
    }

    /// Saving and loading at any point in a run changes nothing: the rest
    /// of the run pops the same wake-ups at the same ticks and ends in the
    /// same state (01 R7).
    #[test]
    fn saving_and_loading_mid_run_is_invisible(script in script(), split in any::<prop::sample::Index>()) {
        let split = split.index(script.len());

        let mut straight = Timeline::new();
        let expected = run(&mut straight, &script, &ANY_RATES, None, |_, _| {});

        let mut reloaded = Timeline::new();
        let got = run(&mut reloaded, &script, &ANY_RATES, None, |index, timeline| {
            if index == split {
                let loaded = reload(timeline);
                assert_eq!(&loaded, &*timeline);
                assert_eq!(Hash128::of(&loaded), Hash128::of(&*timeline));
                *timeline = loaded;
            }
        });

        prop_assert_eq!(got, expected);
        prop_assert_eq!(&reloaded, &straight);
        prop_assert_eq!(Hash128::of(&reloaded), Hash128::of(&straight));
    }

    /// Wake-ups pop in order of due tick and then of scheduling, and time
    /// never runs backwards, whatever is done to the clocks in between.
    #[test]
    fn pops_are_ordered_by_tick_then_sequence(script in script()) {
        let mut timeline = Timeline::new();
        let trace = run(&mut timeline, &script, &ANY_RATES, None, |_, _| {});
        // Each "advance" line lists what fired as id/payload@tick, in pop order.
        for line in trace.iter().filter(|line| line.starts_with("advance")) {
            let Some(list) = line.split('[').nth(1).map(|rest| rest.trim_end_matches(']')) else {
                continue;
            };
            let fired: Vec<(i64, u64)> = list
                .split(", ")
                .filter(|item| !item.is_empty())
                .map(|item| {
                    let (id, rest) = item.split_once('/').expect("id/payload@tick");
                    let (_, tick) = rest.split_once('@').expect("id/payload@tick");
                    (tick.parse().expect("a tick"), id.parse().expect("an id"))
                })
                .collect();
            prop_assert!(
                fired.windows(2).all(|pair| pair[0] < pair[1]),
                "out of order: {line}"
            );
        }
    }
}
