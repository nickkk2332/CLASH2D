//! Timers and continuations (05 §9.4, §10.4, §13.4; 03 §9): what runs
//! later, kept as plain state.
//!
//! A **timer** belongs to an owner (an entity, a cell, an area, a site,
//! a realm or the simulation) and runs on that owner's clock: an area
//! that slows time slows the timers of whoever is inside it, and a
//! personal clock speeds its owner's. It runs one of two things:
//!
//! - a **continuation** (05 §13.4): the effects inside an `after` or an
//!   `every`, found again by where the effect is written (definition
//!   type, qualified id and JSON path), with the subjects, locals and
//!   event fields that were bound there when the timer was set;
//! - a **recurring rule** (05 §9.4): a rule's own `every`, for one
//!   owner. A global one has the simulation as its owner and is started
//!   with the simulation; an attached one is started when it is
//!   attached and stopped when it is detached.
//!
//! Every timer is a wake-up of the [`WakeDomain::Rule`] domain whose
//! `id` is the timer's number. An entity's timers are its wake-ups
//! ([`Sim::schedule_for`]), so they follow it from clock to clock.
//!
//! # Away from the simulation
//!
//! When an entity leaves the world (its tract unloads, or it is stored)
//! its timers are taken off the timeline with the local time each had
//! left, and the clock and the local time it was on when it left. When
//! it returns, each is put back by its `while_unloaded`:
//!
//! | Mode | On return, if `E` local time passed and `R` was left |
//! |------|-------------------------------------------------------|
//! | `pause` | `R` is left: the timer stood still |
//! | `elapse` | `R - E` is left; if that is not above zero, it fires once at once, and an `every` goes on from where its schedule now is |
//! | `all` (`every`) | As `elapse`, but it fires once for each interval missed, at most [`MAX_MISSED_RUNS`] |
//! | `skip` (a rule's) | As `elapse`, but what was missed does not run |
//! | `once` (a rule's) | As `elapse`: one run if at least one was missed |
//!
//! `E` is the local time that passed on the clock the timer was on, or
//! universal time if that clock has gone. What fires at once fires in
//! the order the timers fell due.
//!
//! # Staggering
//!
//! The first run of a recurring rule for an owner comes after a share of
//! its interval given by a positional hash of the owner and the rule's
//! qualified id ([`stagger`]), never by the order owners were made in: a
//! thousand owners of one rule fire across the interval, not at one
//! tick.

use std::collections::{BTreeMap, BTreeSet};

use clash_core::{
    Dur, EntityId, HashDomain, StateHash, StateHasher, Tick, impl_state_hash, pos_hash,
};
use clash_expr::{EventId, Handle, Value};
use clash_time::{ClockId, Wake, WakeDomain, WakeId};
use clash_world::{Cause, CauseKind};
use serde::{Deserialize, Deserializer, Serialize, Serializer};

use super::host::RunView;
use super::state::{EffectSite, SubjectKey};
use super::tree::{AskOptions, Block, EffectKind, Timer};
use super::{EocId, reason};
use crate::Sim;
use crate::event::Subject;
use crate::reason::Reason;

/// The most runs a timer that fires for every interval missed (`all`)
/// makes on its owner's return.
pub const MAX_MISSED_RUNS: u32 = 100;

/// What a timer does about the time its owner spends away from the
/// simulation (03 §9; 05 §9.4, §10.4).
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub enum WhileAway {
    /// The timer stands still.
    Pause,
    /// Time passes; one that fell due meanwhile fires once on return.
    Elapse,
    /// Time passes; it fires once for each interval missed, capped.
    All,
    /// Time passes; what fell due meanwhile does not run.
    Skip,
    /// Time passes; one run on return if at least one was missed.
    Once,
}

impl WhileAway {
    /// The mode written in data: `pause`, `elapse`, `all`, `skip`,
    /// `once`.
    pub fn parse(written: &str) -> Option<WhileAway> {
        Some(match written {
            "pause" => WhileAway::Pause,
            "elapse" => WhileAway::Elapse,
            "all" => WhileAway::All,
            "skip" => WhileAway::Skip,
            "once" => WhileAway::Once,
            _ => return None,
        })
    }
}

impl StateHash for WhileAway {
    fn state_hash(&self, hasher: &mut StateHasher) {
        hasher.write_u8(*self as u8);
    }
}

/// What a timer runs when it fires, as plain data (05 §13.4).
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub struct Continuation {
    /// Where the `after` or `every` is written.
    pub site: EffectSite,
    /// The qualified id of the rule whose effects these are, if they are
    /// a rule's.
    pub rule: Option<String>,
    /// What that rule is attached to, if it is attached.
    pub owner: Option<SubjectKey>,
    /// The names of the subjects bound where the effect is written, by
    /// slot.
    pub names: Vec<String>,
    /// Their values when the timer was set.
    pub subjects: Vec<clash_core::Value>,
    /// The locals of the list the effect is in, by slot, when the timer
    /// was set.
    pub locals: Vec<clash_core::Value>,
    /// The event that was being handled, by name, and its fields.
    pub event: Option<(String, Vec<clash_core::Value>)>,
    /// What the effects are caused by.
    pub cause: Cause,
}

impl_state_hash!(Continuation {
    site,
    rule,
    owner,
    names,
    subjects,
    locals,
    event,
    cause
});

/// What a timer runs.
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub enum TimerBody {
    /// The effects of an `after` or an `every`.
    Effects(Box<Continuation>),
    /// A recurring rule, by qualified id, for the timer's owner.
    Rule(String),
}

impl StateHash for TimerBody {
    fn state_hash(&self, hasher: &mut StateHasher) {
        match self {
            TimerBody::Effects(continuation) => {
                hasher.write_u8(0);
                continuation.state_hash(hasher);
            }
            TimerBody::Rule(name) => {
                hasher.write_u8(1);
                hasher.write_str(name);
            }
        }
    }
}

/// Where a timer is.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub enum TimerState {
    /// On the timeline.
    Waiting {
        /// Its wake-up.
        wake: WakeId,
    },
    /// Off it, while its owner is away (03 §9).
    Away {
        /// The local time it had left.
        remaining: Dur,
        /// The clock it was on, if any.
        clock: Option<ClockId>,
        /// That clock's local time when it left.
        mark: Dur,
        /// Universal time when it left.
        since: Tick,
    },
}

impl StateHash for TimerState {
    fn state_hash(&self, hasher: &mut StateHasher) {
        match self {
            TimerState::Waiting { wake } => {
                hasher.write_u8(0);
                wake.state_hash(hasher);
            }
            TimerState::Away {
                remaining,
                clock,
                mark,
                since,
            } => {
                hasher.write_u8(1);
                remaining.state_hash(hasher);
                clock.state_hash(hasher);
                mark.state_hash(hasher);
                since.state_hash(hasher);
            }
        }
    }
}

/// One timer.
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub struct TimerRecord {
    /// Whose clock it runs on.
    pub owner: SubjectKey,
    /// Its name, unique to its owner, if it has one.
    pub name: Option<String>,
    /// For a timer that repeats, how often, in local time.
    pub interval: Option<Dur>,
    /// For one that repeats a number of times, how many runs are left.
    pub left: Option<u64>,
    /// What it does while its owner is away.
    pub away: WhileAway,
    /// What it runs.
    pub body: TimerBody,
    /// Where it is.
    pub state: TimerState,
    /// Runs still to make at once, after the one now due, on a return.
    pub catch_up: u32,
    /// After those, the local time to the next run of its schedule.
    pub resume: Option<Dur>,
}

impl_state_hash!(TimerRecord {
    owner,
    name,
    interval,
    left,
    away,
    body,
    state,
    catch_up,
    resume
});

/// Every timer, by number (05 invariant E6). Saved and hashed with the
/// rule state; which timers each owner has is worked out again on load.
#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct Timers {
    records: BTreeMap<u64, TimerRecord>,
    next: u64,
    /// Derived: each owner's timers.
    by_owner: BTreeMap<SubjectKey, BTreeSet<u64>>,
}

impl StateHash for Timers {
    fn state_hash(&self, hasher: &mut StateHasher) {
        self.records.state_hash(hasher);
        hasher.write_u64(self.next);
    }
}

impl Serialize for Timers {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        (&self.records, self.next).serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for Timers {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Timers, D::Error> {
        let (records, next) = <(BTreeMap<u64, TimerRecord>, u64)>::deserialize(deserializer)?;
        let mut by_owner: BTreeMap<SubjectKey, BTreeSet<u64>> = BTreeMap::new();
        for (id, record) in &records {
            by_owner.entry(record.owner).or_default().insert(*id);
        }
        Ok(Timers {
            records,
            next,
            by_owner,
        })
    }
}

impl Timers {
    /// How many timers there are.
    pub fn len(&self) -> usize {
        self.records.len()
    }

    /// Whether there are none.
    pub fn is_empty(&self) -> bool {
        self.records.is_empty()
    }

    /// A timer, by number.
    pub fn get(&self, id: u64) -> Option<&TimerRecord> {
        self.records.get(&id)
    }

    /// Every timer, by number.
    pub fn iter(&self) -> impl Iterator<Item = (u64, &TimerRecord)> {
        self.records.iter().map(|(id, record)| (*id, record))
    }

    /// The numbers of an owner's timers, in order.
    pub fn of_owner(&self, owner: SubjectKey) -> Vec<u64> {
        self.by_owner
            .get(&owner)
            .map(|ids| ids.iter().copied().collect())
            .unwrap_or_default()
    }

    /// The timer of an owner with a name.
    pub fn named(&self, owner: SubjectKey, name: &str) -> Option<u64> {
        self.by_owner.get(&owner)?.iter().copied().find(|id| {
            self.records
                .get(id)
                .is_some_and(|record| record.name.as_deref() == Some(name))
        })
    }

    /// The timer that runs a recurring rule for an owner.
    pub fn recurring(&self, owner: SubjectKey, rule: &str) -> Option<u64> {
        self.by_owner.get(&owner)?.iter().copied().find(|id| {
            self.records
                .get(id)
                .is_some_and(|record| matches!(&record.body, TimerBody::Rule(name) if name == rule))
        })
    }

    fn insert(&mut self, record: TimerRecord) -> u64 {
        let id = self.next;
        self.next = self.next.saturating_add(1);
        self.by_owner.entry(record.owner).or_default().insert(id);
        self.records.insert(id, record);
        id
    }

    fn get_mut(&mut self, id: u64) -> Option<&mut TimerRecord> {
        self.records.get_mut(&id)
    }

    fn remove(&mut self, id: u64) -> Option<TimerRecord> {
        let record = self.records.remove(&id)?;
        if let Some(ids) = self.by_owner.get_mut(&record.owner) {
            ids.remove(&id);
            if ids.is_empty() {
                self.by_owner.remove(&record.owner);
            }
        }
        Some(record)
    }

    /// Forgets every timer of an owner that no longer exists. Its
    /// wake-ups are the kernel's to cancel.
    pub(crate) fn forget(&mut self, owner: SubjectKey) {
        for id in self.of_owner(owner) {
            self.remove(id);
        }
    }
}

/// How long a timer waits, and how often it repeats.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum TimerPlan {
    /// `after`: once, after this much local time.
    After(Dur),
    /// `every`: after each interval, `times` times or for ever.
    Every {
        /// The interval, in local time.
        interval: Dur,
        /// How many times; for ever if `None`.
        times: Option<u64>,
    },
}

/// A timer an `after` or an `every` asks the host to set (05 §10.4).
#[derive(Clone, Copy, Debug)]
pub struct TimerSet<'a> {
    /// When it fires.
    pub plan: TimerPlan,
    /// Its name, unique to its owner: one of the same name is replaced.
    pub name: Option<&'a str>,
    /// Its owner, as written or as `self` gives it; `None` for the
    /// host to choose (the realm of `here`, or the simulation).
    pub owner: Option<Handle>,
    /// What is bound as `here`, for that choice.
    pub here: Option<Handle>,
    /// What it does while its owner is away.
    pub away: WhileAway,
    /// The origin number of the effect: where it is written.
    pub origin: u64,
    /// The locals of the list the effect is in, by slot.
    pub locals: &'a [Value],
    /// The event being handled where the effect is written, and its
    /// fields.
    pub event: Option<(EventId, &'a [Value])>,
    /// What is running: the rule, its owner and its subjects.
    pub by: RunView<'a>,
}

/// The share of an interval after which a recurring rule first runs for
/// an owner: a positional hash of the owner and the rule's qualified id,
/// from a microsecond up to the whole interval.
pub fn stagger(owner: SubjectKey, rule: &str, interval: Dur) -> Dur {
    const DOMAIN: HashDomain = HashDomain::new("rules.stagger");
    let span = u64::try_from(interval.as_micros().max(1)).unwrap_or(1);
    let low = i32::from_ne_bytes((owner.payload as u32).to_ne_bytes());
    let high = i32::from_ne_bytes(((owner.payload >> 32) as u32).to_ne_bytes());
    let hash = pos_hash(
        0,
        DOMAIN,
        low,
        high,
        i32::from(owner.kind),
        HashDomain::new(rule).raw(),
    );
    Dur::from_micros(i64::try_from(1 + hash % span).unwrap_or(1))
}

/// What a return does to a timer: how many runs to make at once, and
/// the local time from then to the next run of its schedule (`None` if
/// it ends with those).
fn on_return(record: &TimerRecord, remaining: Dur, elapsed: Dur) -> (u32, Option<Dur>) {
    if record.away == WhileAway::Pause || elapsed < remaining {
        let left = if record.away == WhileAway::Pause {
            remaining
        } else {
            remaining - elapsed
        };
        return (0, Some(left));
    }
    let overdue = elapsed - remaining;
    let Some(interval) = record.interval.filter(|interval| *interval > Dur::ZERO) else {
        // `after`: once, now.
        return (1, None);
    };
    let over = overdue.as_micros();
    let every = interval.as_micros();
    let missed = 1 + over / every;
    let next = Dur::from_micros(every - over % every);
    let runs = match record.away {
        WhileAway::Skip => 0,
        WhileAway::All => u32::try_from(missed)
            .unwrap_or(u32::MAX)
            .min(MAX_MISSED_RUNS),
        _ => 1,
    };
    let runs = match record.left {
        Some(left) => runs.min(u32::try_from(left).unwrap_or(u32::MAX)),
        None => runs,
    };
    (runs, Some(next))
}

/// The wake-up of a timer.
fn timer_wake_of(id: u64) -> Wake {
    Wake::new(WakeDomain::Rule, id, 0)
}

/// Looks for the `after` or `every` written at a path in a block and
/// the blocks inside it.
pub(crate) fn timer_at<'a>(block: &'a Block, path: &str) -> Option<&'a Timer> {
    for effect in block.effects() {
        // An effect inside this one has a path that starts with its.
        if !path.starts_with(&*effect.path) {
            continue;
        }
        let inner: Vec<&Block> = match &effect.kind {
            EffectKind::Timer(timer) => {
                if &*effect.path == path {
                    return Some(timer);
                }
                vec![&timer.body]
            }
            EffectKind::If {
                then, otherwise, ..
            } => vec![then, otherwise],
            EffectKind::First { entries } => entries.iter().map(|(_, body)| body).collect(),
            EffectKind::Switch { cases, default, .. } => cases
                .iter()
                .map(|(_, body)| body)
                .chain([default])
                .collect(),
            EffectKind::ForEach { body, .. }
            | EffectKind::Repeat { body, .. }
            | EffectKind::While { body, .. }
            | EffectKind::Rebind { body, .. } => vec![body],
            EffectKind::Weighted { entries } => entries.iter().map(|(_, body)| body).collect(),
            EffectKind::Ask(ask) => match &ask.options {
                AskOptions::Listed(options) => options.iter().map(|option| &option.body).collect(),
                AskOptions::Computed(_) => Vec::new(),
            },
            _ => Vec::new(),
        };
        if let Some(found) = inner.into_iter().find_map(|block| timer_at(block, path)) {
            return Some(found);
        }
    }
    None
}

impl Sim {
    /// Every timer (05 §10.4).
    pub fn timers(&self) -> &Timers {
        &self.rules.timers
    }

    /// The local time a timer has left, wherever it is.
    pub fn timer_left(&self, id: u64) -> Option<Dur> {
        match self.rules.timers.get(id)?.state {
            TimerState::Waiting { wake } => self.timeline.remaining(wake),
            TimerState::Away { remaining, .. } => Some(remaining),
        }
    }

    /// The clock a timer of an owner runs on: an entity's own, the
    /// clock of a cell, an area's own or else its realm's, a site's
    /// realm's, a realm's, and universal time for the simulation.
    fn owner_clock(&self, owner: Subject) -> Option<ClockId> {
        match owner {
            Subject::Entity(id) => self.entities.location(id).and_then(|_| self.clock_of(id)),
            Subject::Loc(loc) | Subject::Element(loc, _) => Some(self.location_clock(loc)),
            Subject::Area(id) => {
                let area = self.cosmos.area(id)?;
                area.clock
                    .or_else(|| self.cosmos.grid(area.grid).ok().map(|grid| grid.clock()))
            }
            Subject::Site(id) => {
                let site = self.cosmos.site(id)?;
                self.cosmos.realm(site.realm).map(|realm| realm.clock)
            }
            Subject::Tract(realm, _) | Subject::Realm(realm) => {
                self.cosmos.realm(realm).map(|realm| realm.clock)
            }
            Subject::Global => Some(ClockId::UNIVERSAL),
        }
    }

    /// Puts a timer on the timeline for its owner, or keeps it away if
    /// its owner is an entity out of the world.
    fn place_timer(&mut self, id: u64, delay: Dur) -> Result<(), Reason> {
        let Some(owner) = self.rules.timers.get(id).map(|record| record.owner) else {
            return Ok(());
        };
        let subject = self.exprs.subject(Handle::from(owner)).ok_or_else(|| {
            Reason::new(
                reason::TIMER_REFUSED,
                "a timer's owner is something the simulation keeps",
            )
        })?;
        let state = match subject {
            Subject::Entity(entity) if self.entities.location(entity).is_none() => {
                self.known(entity)?;
                TimerState::Away {
                    remaining: delay,
                    clock: None,
                    mark: Dur::ZERO,
                    since: self.now(),
                }
            }
            Subject::Entity(entity) => TimerState::Waiting {
                wake: self.schedule_for(entity, delay, timer_wake_of(id))?,
            },
            other => {
                let clock = self.owner_clock(other).ok_or_else(|| {
                    Reason::new(
                        reason::TIMER_REFUSED,
                        "the timer's owner is on no clock: it no longer exists",
                    )
                })?;
                TimerState::Waiting {
                    wake: self.schedule_on(clock, delay, timer_wake_of(id))?,
                }
            }
        };
        if let Some(record) = self.rules.timers.get_mut(id) {
            record.state = state;
        }
        Ok(())
    }

    /// Takes a timer off the timeline and forgets it.
    fn drop_timer(&mut self, id: u64) {
        if let Some(record) = self.rules.timers.remove(id)
            && let TimerState::Waiting { wake } = record.state
        {
            self.cancel_wake(wake);
        }
    }

    /// Sets a timer and gives its number. One of the same name and owner
    /// is replaced.
    pub(crate) fn add_timer(&mut self, record: TimerRecord, delay: Dur) -> Result<u64, Reason> {
        if let Some(name) = &record.name
            && let Some(old) = self.rules.timers.named(record.owner, name)
        {
            self.drop_timer(old);
        }
        let id = self.rules.timers.insert(record);
        if let Err(reason) = self.place_timer(id, delay) {
            self.rules.timers.remove(id);
            return Err(reason);
        }
        Ok(id)
    }

    /// The owner of a timer the effect named none for, and no `self`
    /// gives (05 §10.4): the realm of `here`, else the simulation.
    pub(crate) fn default_timer_owner(&self, here: Option<Handle>) -> Option<Handle> {
        let realm = here
            .and_then(|here| self.exprs.subject(here))
            .and_then(|here| match here {
                Subject::Loc(loc) | Subject::Element(loc, _) => self.cosmos.realm_of(loc.grid),
                _ => None,
            });
        match realm {
            Some(realm) => self.exprs.kinds().handle(Subject::Realm(realm)),
            None => self.exprs.kinds().handle(Subject::Global),
        }
    }

    /// Cancels an owner's timer by name. Gives whether there was one.
    pub(crate) fn cancel_named_timer(&mut self, owner: Handle, name: &str) -> bool {
        match self.rules.timers.named(SubjectKey::from(owner), name) {
            Some(id) => {
                self.drop_timer(id);
                true
            }
            None => false,
        }
    }

    /// Starts or stops a recurring rule for an owner (05 §9.4): what
    /// attaching and detaching a rule with an `every` does.
    pub(crate) fn set_recurring(&mut self, owner: Handle, rule: EocId, on: bool) {
        let rules = std::sync::Arc::clone(&self.rt.rules);
        let (Some(def), Some(name)) = (rules.rule(rule), rules.name(rule)) else {
            return;
        };
        let Some(interval) = def.every() else {
            return;
        };
        let key = SubjectKey::from(owner);
        let existing = self.rules.timers.recurring(key, name);
        match (on, existing) {
            (true, None) => {
                let record = TimerRecord {
                    owner: key,
                    name: None,
                    interval: Some(interval),
                    left: None,
                    away: recurring_away(def.while_unloaded),
                    body: TimerBody::Rule(name.to_owned()),
                    state: TimerState::Away {
                        remaining: Dur::ZERO,
                        clock: None,
                        mark: Dur::ZERO,
                        since: self.now(),
                    },
                    catch_up: 0,
                    resume: None,
                };
                if let Err(reason) = self.add_timer(record, stagger(key, name, interval)) {
                    self.fault(reason);
                }
            }
            (false, Some(id)) => self.drop_timer(id),
            _ => {}
        }
    }

    /// Starts every global recurring rule that has no timer yet: when
    /// the simulation is set up, and after a load for rules that came
    /// with the definitions since.
    pub(crate) fn start_recurring_rules(&mut self) {
        let rules = std::sync::Arc::clone(&self.rt.rules);
        let Some(global) = self.exprs.kinds().handle(Subject::Global) else {
            return;
        };
        let Some(registry) = rules.defs().try_registry::<super::EocDef>() else {
            return;
        };
        for (id, def) in registry.iter() {
            if def.every().is_some() && !def.is_attached() {
                self.set_recurring(global, id, true);
            }
        }
    }

    /// Takes an entity's timers off the timeline as it leaves the world,
    /// and puts them back as it returns, by each one's `while_unloaded`
    /// (03 §9). The kernel calls this whenever it puts an entity on the
    /// clock that now applies.
    pub(crate) fn timers_follow(&mut self, entity: EntityId) {
        let Some(handle) = self.exprs.kinds().handle(Subject::Entity(entity)) else {
            return;
        };
        let ids = self.rules.timers.of_owner(SubjectKey::from(handle));
        if ids.is_empty() {
            return;
        }
        let in_world = self.entities.location(entity).is_some();
        // What fell due while it was away, most overdue first.
        let mut due_now: Vec<(i64, u64, u32, Option<Dur>)> = Vec::new();
        for id in ids {
            let Some(record) = self.rules.timers.get(id).cloned() else {
                continue;
            };
            match (record.state, in_world) {
                (TimerState::Waiting { wake }, false) => {
                    let remaining = self.timeline.remaining(wake).unwrap_or(Dur::ZERO);
                    let clock = self.timeline.clock_of(wake);
                    let mark = clock
                        .and_then(|clock| self.timeline.local_now(clock).ok())
                        .unwrap_or(Dur::ZERO);
                    self.cancel_wake(wake);
                    if let Some(record) = self.rules.timers.get_mut(id) {
                        record.state = TimerState::Away {
                            remaining,
                            clock,
                            mark,
                            since: self.timeline.now(),
                        };
                    }
                }
                (
                    TimerState::Away {
                        remaining,
                        clock,
                        mark,
                        since,
                    },
                    true,
                ) => {
                    let elapsed = match clock.and_then(|clock| self.timeline.local_now(clock).ok())
                    {
                        Some(now) => now - mark,
                        None => self.timeline.now().since(since),
                    };
                    let (runs, next) = on_return(&record, remaining, elapsed);
                    if runs == 0 {
                        if let Some(next) = next {
                            if let Err(reason) = self.place_timer(id, next) {
                                self.fault(reason);
                            }
                        } else {
                            self.rules.timers.remove(id);
                        }
                    } else {
                        let overdue = (elapsed - remaining).as_micros();
                        due_now.push((overdue, id, runs, next));
                    }
                }
                _ => {}
            }
        }
        due_now.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
        for (_, id, runs, next) in due_now {
            if let Some(record) = self.rules.timers.get_mut(id) {
                record.catch_up = runs.saturating_sub(1);
                record.resume = next;
                if next.is_none() && record.interval.is_some() {
                    // Its runs are used up with these.
                    record.left = Some(u64::from(runs));
                }
            }
            if let Err(reason) = self.place_timer(id, Dur::ZERO) {
                self.fault(reason);
            }
        }
    }
}

/// What `owner.timer(name)` reads: the local time, in seconds, a named
/// timer of the owner has left, or null if it has none of that name.
fn timer_left_of(sim: &Sim, owner: Subject, args: &[Value]) -> Value {
    let (Some(name), Some(handle)) = (
        args.first().and_then(Value::as_str),
        sim.exprs.kinds().handle(owner),
    ) else {
        return Value::Null;
    };
    sim.rules
        .timers
        .named(SubjectKey::from(handle), name)
        .and_then(|id| sim.timer_left(id))
        .map_or(Value::Null, |left| Value::Real(left.as_secs_real()))
}

/// The property `timer(name)` on every kind of subject a timer may
/// belong to (05 §10.4).
pub(crate) fn properties() -> Vec<crate::registry::PropertyDef> {
    ["entity", "loc", "area", "site", "realm", "global"]
        .into_iter()
        .map(|kind| {
            crate::registry::PropertyDef::new(
                kind,
                "timer",
                "real?",
                "The local time, in seconds, that the timer of this name belonging to it has left (05 §10.4); null if it has none of that name. A timer away with its owner keeps what it had left.",
                timer_left_of,
            )
            .arg("name", "string", "The timer's name.")
        })
        .collect()
}

/// A recurring rule's `while_unloaded` as a timer's.
fn recurring_away(mode: super::WhileUnloaded) -> WhileAway {
    match mode {
        super::WhileUnloaded::Skip => WhileAway::Skip,
        super::WhileUnloaded::Once => WhileAway::Once,
        super::WhileUnloaded::All => WhileAway::All,
    }
}

/// The cause of a recurring rule's run: the rule's, with its owner as
/// the actor if that is an entity.
fn recurring_cause(sim: &Sim, owner: SubjectKey) -> Cause {
    let actor = match sim.exprs.subject(Handle::from(owner)) {
        Some(Subject::Entity(id)) => Some(id),
        _ => None,
    };
    Cause {
        actor,
        instigator: actor,
        kind: CauseKind::Eoc,
    }
}

/// Handles a wake-up of the `Rule` domain: a timer that is due. What
/// comes next is settled before what it runs runs, so that its effects
/// may cancel or replace it.
pub(crate) fn timer_wake(sim: &mut Sim, wake_id: WakeId, wake: Wake) {
    let id = wake.id;
    let Some(record) = sim.rules.timers.get(id).cloned() else {
        // Cancelled since.
        return;
    };
    if record.state != (TimerState::Waiting { wake: wake_id }) {
        return;
    }
    // Settle what comes next.
    let mut catch_up = record.catch_up;
    let mut resume = record.resume;
    let mut left = record.left;
    if let Some(count) = &mut left {
        *count = count.saturating_sub(1);
    }
    let next = if left == Some(0) {
        None
    } else if catch_up > 0 {
        catch_up -= 1;
        Some(Dur::ZERO)
    } else if let Some(after) = resume.take() {
        Some(after)
    } else {
        record.interval
    };
    match next {
        Some(delay) => {
            if let Some(stored) = sim.rules.timers.get_mut(id) {
                stored.catch_up = catch_up;
                stored.resume = resume;
                stored.left = left;
            }
            if let Err(reason) = sim.place_timer(id, delay) {
                sim.fault(reason);
                sim.rules.timers.remove(id);
            }
        }
        None => {
            sim.rules.timers.remove(id);
        }
    }
    // Run what it runs.
    let rules = std::sync::Arc::clone(&sim.rt.rules);
    match &record.body {
        TimerBody::Effects(continuation) => {
            let result = rules.resume_timer(&mut sim.rule_host(continuation.cause), continuation);
            if let Err(reason) = result {
                sim.fault(reason);
                sim.rules.timers.remove(id);
            }
        }
        TimerBody::Rule(name) => {
            let Some(rule) = rules.lookup(name) else {
                sim.fault(
                    Reason::new(
                        reason::TIMER_LOST,
                        format!("the recurring rule `{name}` is no longer among the definitions"),
                    )
                    .with("rule", name.as_str()),
                );
                sim.drop_timer(id);
                return;
            };
            let cause = recurring_cause(sim, record.owner);
            rules.run_recurring(&mut sim.rule_host(cause), rule, Handle::from(record.owner));
        }
    }
}
