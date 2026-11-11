//! Pools (06 §4.2): a bounded quantity with a net rate, whose threshold
//! crossings are scheduled and never polled.
//!
//! # The level is a straight line
//!
//! A pool keeps its level as of a moment of its owner's local time, and
//! a **net rate**: its definition's `regen` and every [`PoolRate`]
//! applied, summed in order of source. Between changes
//!
//! ```text
//! level(t) = clamp(level + net × (t − synced), 0, max)
//! ```
//!
//! so reading it costs one multiplication and nothing runs while it
//! moves. Every function that changes a pool first brings the level up
//! to date, then makes its change, then schedules again. `max` and
//! `regen` are expressions; they are worked out when the pool is given,
//! when an attribute they read changes, and on
//! [`refresh`](super::refresh).
//!
//! # Crossings
//!
//! A threshold is a fraction of `max`. A pool remembers how many of its
//! thresholds it is at or above (its **band**), and reports a crossing
//! whenever that changes, whatever moved it: a rate over time, an amount
//! added or taken, or a change of `max`. A level that rises to a
//! threshold exactly has crossed it upward; one that falls to it exactly
//! has crossed it downward. A pool that holds nothing (`max` of zero) is
//! below every threshold.
//!
//! A pool whose net rate carries it toward a threshold schedules **one**
//! wake-up, for the local time at which the line meets it:
//!
//! ```text
//! after = ceil(|level − threshold × max| / |net|)   in microseconds
//! ```
//!
//! [`on_wake`] then puts the level on the threshold, reports the
//! crossing and schedules the next. A pool with no net rate, or with no
//! threshold ahead of it, schedules nothing: the bounds themselves are
//! kept by the clamp above and need no wake-up. The wake-up is on the
//! owner's clock, so when that clock's rate changes the timeline moves
//! it and keeps its remaining local time (03 §3.3); nothing here is
//! called.
//!
//! # Leaving and returning
//!
//! [`suspend`] brings an entity's pools up to date and cancels their
//! wake-ups; while suspended they stand still. [`resume`] takes the
//! local time that passed: the pools move by it in one step, the
//! thresholds passed are reported once each in the order they were
//! passed, and the next crossings are scheduled (03 §9).

use clash_core::{Dur, EntityId, MICROS_PER_SECOND, Real, impl_state_hash};
use clash_data::Def;
use clash_expr::{Host, Value};
use clash_time::{ClockId, Timeline, Wake, WakeDomain, WakeId};
use serde::{Deserialize, Serialize};

use super::attr::Source;
use super::defs::{PoolDef, PoolId, StatDefs};
use super::props::Scoped;
use super::{Change, Changes, StatsCx, StatsError, stats_mut, stats_of};

/// The bit of a wake-up's `arg` that marks it as a pool's crossing. The
/// rest of `arg` is the pool's dense id, and `id` is the owner's
/// [`EntityId::to_bits`].
pub const POOL_WAKE_TAG: u64 = 1 << 63;

/// The longest a crossing is scheduled ahead, in microseconds of local
/// time. One further off than this (about 146,000 years) is not
/// scheduled at all: the timeline could not hold its due tick.
const MAX_WAIT_MICROS: i64 = i64::MAX / 2;

/// The wake-up payload of a pool's next crossing.
pub fn pool_wake(entity: EntityId, pool: PoolId) -> Wake {
    Wake::new(
        WakeDomain::Timer,
        entity.to_bits(),
        POOL_WAKE_TAG | pool.index() as u64,
    )
}

/// Whether a popped wake-up is a pool's crossing, to hand to
/// [`on_wake`].
pub fn is_pool_wake(wake: &Wake) -> bool {
    wake.domain == WakeDomain::Timer && wake.arg & POOL_WAKE_TAG != 0
}

/// A rate applied to a pool for as long as its source lasts: a leak, an
/// upkeep, a feed.
#[derive(Clone, Copy, PartialEq, Debug, Serialize, Deserialize)]
pub struct PoolRate {
    /// Who applied it.
    pub source: Source,
    /// What the pool gains in a second of its owner's time; negative to
    /// drain.
    pub rate: Real,
}

impl_state_hash!(PoolRate { source, rate });

/// Which way a pool's level moved against its thresholds.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Dir {
    /// Toward `max`.
    Rising,
    /// Toward zero.
    Falling,
    /// Neither.
    Level,
}

impl Dir {
    fn of(old: Real, new: Real) -> Dir {
        if new > old {
            Dir::Rising
        } else if new < old {
            Dir::Falling
        } else {
            Dir::Level
        }
    }
}

/// One pool of one entity.
#[derive(Clone, PartialEq, Debug, Serialize, Deserialize)]
pub(super) struct PoolInst {
    /// Which pool.
    pub(super) pool: PoolId,
    /// The level as of `synced`.
    level: Real,
    /// The local time on the owner's clock at which `level` was current.
    synced: Dur,
    /// The most the pool holds: the definition's `max`, as last worked
    /// out.
    max: Real,
    /// The definition's `regen`, as last worked out.
    regen: Real,
    /// The rates applied, in order of source.
    rates: Vec<PoolRate>,
    /// How many thresholds the level counts as at or above.
    band: u16,
    /// The wake-up for the next crossing, if one is ahead.
    wake: Option<WakeId>,
}

impl_state_hash!(PoolInst {
    pool,
    level,
    synced,
    max,
    regen,
    rates,
    band,
    wake
});

impl PoolInst {
    pub(super) fn max(&self) -> Real {
        self.max
    }

    pub(super) fn rates(&self) -> &[PoolRate] {
        &self.rates
    }

    pub(super) fn band(&self) -> usize {
        usize::from(self.band)
    }

    pub(super) fn wake(&self) -> Option<WakeId> {
        self.wake
    }

    /// The net rate: regeneration and every rate applied.
    pub(super) fn net(&self) -> Real {
        let mut net = self.regen;
        for rate in &self.rates {
            net += rate.rate;
        }
        net
    }

    /// The level as it was last brought up to date.
    pub(super) fn level_as_synced(&self) -> Real {
        self.level
    }

    /// The level at a local time on the owner's clock.
    pub(super) fn level_at(&self, now: Dur) -> Real {
        let elapsed = (now - self.synced).max(Dur::ZERO).as_secs_real();
        (self.level + self.net() * elapsed).clamp(Real::ZERO, self.max)
    }

    /// The level a threshold stands for.
    fn mark(&self, def: &PoolDef, index: usize) -> Real {
        def.thresholds[index] * self.max
    }

    /// The threshold the net rate is carrying the level toward, and
    /// which way.
    fn aim(&self, def: &PoolDef) -> Option<(usize, Dir)> {
        if self.max <= Real::ZERO {
            return None;
        }
        let net = self.net();
        // A save made under other definitions may count more thresholds
        // than there are now.
        let band = self.band().min(def.thresholds.len());
        if net < Real::ZERO && band > 0 {
            Some((band - 1, Dir::Falling))
        } else if net > Real::ZERO && band < def.thresholds.len() {
            Some((band, Dir::Rising))
        } else {
            None
        }
    }

    /// The local time until the level meets the threshold it is carried
    /// toward, if there is one within the timeline's reach.
    fn next_crossing(&self, def: &PoolDef) -> Option<Dur> {
        let (index, _) = self.aim(def)?;
        let gap = (self.mark(def, index) - self.level).abs();
        let seconds = gap / self.net().abs();
        let micros = (seconds * Real::from_i64(MICROS_PER_SECOND)).ceil();
        if !micros.is_finite() || micros > Real::from_i64(MAX_WAIT_MICROS) {
            return None;
        }
        Some(Dur(micros.to_i64()))
    }

    /// Brings the band into line with the level after it moved in `dir`,
    /// reporting each threshold crossed.
    fn recross(&mut self, def: &PoolDef, dir: Dir, entity: EntityId, out: &mut Changes) {
        let cross = |inst: &PoolInst, index: usize, rising: bool| Change::PoolCrossed {
            entity,
            pool: inst.pool,
            threshold: def.thresholds[index],
            rising,
            level: inst.level,
        };
        if self.band() > def.thresholds.len() {
            // A save made under other definitions: there are fewer
            // thresholds now than the pool had counted.
            self.band = u16::try_from(def.thresholds.len()).unwrap_or(u16::MAX);
        }
        if self.max <= Real::ZERO {
            // A pool that holds nothing is below every threshold.
            while self.band > 0 {
                self.band -= 1;
                out.push(cross(self, self.band(), false));
            }
            return;
        }
        while self.band() < def.thresholds.len() {
            let mark = self.mark(def, self.band());
            if self.level > mark || (dir == Dir::Rising && self.level >= mark) {
                out.push(cross(self, self.band(), true));
                self.band += 1;
            } else {
                break;
            }
        }
        while self.band > 0 {
            let mark = self.mark(def, self.band() - 1);
            if self.level < mark || (dir == Dir::Falling && self.level <= mark) {
                self.band -= 1;
                out.push(cross(self, self.band(), false));
            } else {
                break;
            }
        }
    }

    /// Lets time pass up to `now`. If `fired`, the pool's wake-up is what
    /// brought it here, and the level is put on the threshold it was
    /// aimed at: the wake-up is rounded to a whole tick, the line is not.
    fn advance(
        &mut self,
        def: &PoolDef,
        now: Dur,
        fired: bool,
        entity: EntityId,
        out: &mut Changes,
    ) {
        let old = self.level;
        let mut level = self.level_at(now);
        let mut dir = Dir::of(old, level);
        if fired && let Some((index, aimed)) = self.aim(def) {
            let mark = self.mark(def, index);
            level = match aimed {
                Dir::Falling => level.min(mark),
                Dir::Rising => level.max(mark),
                Dir::Level => level,
            };
            dir = aimed;
        }
        self.level = level;
        self.synced = self.synced.max(now);
        self.recross(def, dir, entity, out);
    }

    /// Adds an amount, negative to take away, keeping the level within
    /// the pool's bounds. Returns what was in fact added.
    fn shift(&mut self, def: &PoolDef, amount: Real, entity: EntityId, out: &mut Changes) -> Real {
        let old = self.level;
        let wanted = old + amount;
        let (level, applied) = if wanted > self.max {
            (self.max, self.max - old)
        } else if wanted < Real::ZERO {
            (Real::ZERO, Real::ZERO - old)
        } else {
            (wanted, amount)
        };
        self.level = level;
        self.recross(def, Dir::of(old, level), entity, out);
        applied
    }

    /// Takes a new `max` and `regen`. A level above the new `max` comes
    /// down to it.
    fn rebound(
        &mut self,
        def: &PoolDef,
        max: Real,
        regen: Real,
        entity: EntityId,
        out: &mut Changes,
    ) {
        let old_max = self.max;
        self.max = max;
        self.regen = regen;
        self.level = self.level.min(max);
        // A smaller `max` raises the fraction the level stands at.
        self.recross(def, Dir::of(max, old_max), entity, out);
    }

    /// The band of a pool just made: the thresholds at or below its
    /// level.
    fn first_band(&self, def: &PoolDef) -> u16 {
        if self.max <= Real::ZERO {
            return 0;
        }
        let at_or_below = def
            .thresholds
            .iter()
            .filter(|threshold| **threshold * self.max <= self.level)
            .count();
        u16::try_from(at_or_below).unwrap_or(u16::MAX)
    }
}

/// A number an expression gave, as an amount a pool can use: null, a
/// negative number where one may not be, and anything not finite count
/// as zero.
fn amount(value: Option<Real>, may_be_negative: bool) -> Real {
    match value {
        Some(value) if value.is_finite() && (may_be_negative || value >= Real::ZERO) => value,
        _ => Real::ZERO,
    }
}

/// Works out a pool definition's `max` and `regen` for an entity, and
/// its `start` if asked.
fn evaluate<H: Host + ?Sized>(
    cx: &mut StatsCx<'_, H>,
    entity: EntityId,
    def: &PoolDef,
    with_start: bool,
) -> (Real, Real, Real) {
    let mut host = Scoped {
        stats: &*cx.stats,
        defs: cx.defs,
        timeline: &*cx.timeline,
        inner: &mut *cx.host,
        owner: entity,
        second: Value::Null,
        local: Value::Null,
        allowed: Some(def.reads()),
    };
    let exprs = cx.defs.defs().exprs();
    let max = amount(def.max.eval(&mut cx.runner.over(exprs, &mut host)), false);
    host.second = Value::Real(max);
    let regen = amount(def.regen.eval(&mut cx.runner.over(exprs, &mut host)), true);
    let start = if with_start {
        // What a pool starts at is read once and depends on nothing
        // afterwards, so it may read any attribute.
        host.allowed = None;
        amount(def.start.eval(&mut cx.runner.over(exprs, &mut host)), false)
    } else {
        Real::ZERO
    };
    (max, regen, start)
}

/// The definition of a pool and its qualified id.
fn pool_def(defs: &StatDefs, pool: PoolId) -> Result<&PoolDef, StatsError> {
    defs.pool(pool).ok_or(StatsError::UnknownStat {
        what: PoolDef::TYPE,
    })
}

fn pool_name(defs: &StatDefs, pool: PoolId) -> String {
    defs.defs().registry::<PoolDef>().name(pool).to_owned()
}

/// Cancels a pool's wake-up and schedules the one for its next crossing,
/// if there is one ahead and the owner is in the simulation.
fn reschedule(
    timeline: &mut Timeline,
    clock: ClockId,
    suspended: bool,
    entity: EntityId,
    inst: &mut PoolInst,
    def: &PoolDef,
) -> Result<(), StatsError> {
    if let Some(wake) = inst.wake.take() {
        timeline.cancel(wake);
    }
    if suspended {
        return Ok(());
    }
    if let Some(after) = inst.next_crossing(def) {
        inst.wake = Some(timeline.schedule(clock, after, pool_wake(entity, inst.pool))?);
    }
    Ok(())
}

/// Brings one pool of an entity up to date, makes a change to it, and
/// schedules its next crossing. The one way a pool that exists is
/// changed.
fn with_pool<H: Host + ?Sized, T>(
    cx: &mut StatsCx<'_, H>,
    entity: EntityId,
    pool: PoolId,
    fired: bool,
    out: &mut Changes,
    change: impl FnOnce(&mut PoolInst, &PoolDef, &mut Changes) -> Result<T, StatsError>,
) -> Result<T, StatsError> {
    let def = pool_def(cx.defs, pool)?;
    let stats = stats_mut(cx.stats, entity)?;
    let now = stats.local_now(cx.timeline);
    if now.is_none() && !stats.suspended {
        // The clock is gone: say so rather than let the pool stand still.
        cx.timeline.local_now(stats.clock)?;
    }
    let (clock, suspended) = (stats.clock, stats.suspended);
    let Ok(at) = stats.pool_at(pool) else {
        return Err(StatsError::NoSuchPool {
            entity,
            pool,
            name: pool_name(cx.defs, pool),
        });
    };
    let inst = &mut stats.pools[at];
    if let Some(now) = now {
        inst.advance(def, now, fired, entity, out);
    }
    let result = change(inst, def, out)?;
    reschedule(cx.timeline, clock, suspended, entity, inst, def)?;
    Ok(result)
}

/// Gives an entity a pool, at its definition's `start`. Nothing is
/// reported: a pool is not said to cross the thresholds it starts
/// beyond.
///
/// Refused if the entity already has the pool.
pub fn add_pool<H: Host + ?Sized>(
    cx: &mut StatsCx<'_, H>,
    entity: EntityId,
    pool: PoolId,
) -> Result<(), StatsError> {
    let def = pool_def(cx.defs, pool)?;
    let stats = stats_of(cx.stats, entity)?;
    if stats.has_pool(pool) {
        return Err(StatsError::HasPool {
            entity,
            pool,
            name: pool_name(cx.defs, pool),
        });
    }
    let (clock, suspended) = (stats.clock, stats.suspended);
    let synced = if suspended {
        Dur::ZERO
    } else {
        cx.timeline.local_now(clock)?
    };
    let (max, regen, start) = evaluate(cx, entity, def, true);
    let mut inst = PoolInst {
        pool,
        level: start.min(max),
        synced,
        max,
        regen,
        rates: Vec::new(),
        band: 0,
        wake: None,
    };
    inst.band = inst.first_band(def);
    reschedule(cx.timeline, clock, suspended, entity, &mut inst, def)?;
    let stats = stats_mut(cx.stats, entity)?;
    let at = stats.pool_at(pool).unwrap_or_else(|at| at);
    stats.pools.insert(at, inst);
    Ok(())
}

/// Takes a pool away from an entity, with its rates and its wake-up.
/// Nothing is reported.
pub fn remove_pool<H: Host + ?Sized>(
    cx: &mut StatsCx<'_, H>,
    entity: EntityId,
    pool: PoolId,
) -> Result<(), StatsError> {
    let stats = stats_mut(cx.stats, entity)?;
    let Ok(at) = stats.pool_at(pool) else {
        return Err(StatsError::NoSuchPool {
            entity,
            pool,
            name: pool_name(cx.defs, pool),
        });
    };
    if let Some(wake) = stats.pools.remove(at).wake {
        cx.timeline.cancel(wake);
    }
    Ok(())
}

/// What [`pool_add`] did.
#[derive(Clone, PartialEq, Debug)]
pub struct PoolAdded {
    /// What was in fact added: the amount asked for, or less where the
    /// pool's bounds stopped it. Negative when taking away.
    pub applied: Real,
    /// The thresholds crossed.
    pub changes: Changes,
}

/// Adds an amount to a pool, negative to take away. The level stays
/// within the pool's bounds: what would pass `max` or zero is not
/// applied, and the result says how much was.
///
/// The hook `pool_changing` is the caller's to raise first.
pub fn pool_add<H: Host + ?Sized>(
    cx: &mut StatsCx<'_, H>,
    entity: EntityId,
    pool: PoolId,
    amount: Real,
) -> Result<PoolAdded, StatsError> {
    if !amount.is_finite() {
        return Err(StatsError::BadAmount { amount });
    }
    let mut changes = Changes::new();
    let applied = with_pool(cx, entity, pool, false, &mut changes, |inst, def, out| {
        Ok(inst.shift(def, amount, entity, out))
    })?;
    Ok(PoolAdded { applied, changes })
}

/// Takes an amount from a pool, in full or not at all (08 §5.2). Refused
/// with [`StatsError::PoolTooLow`] if the pool holds less.
pub fn pool_spend<H: Host + ?Sized>(
    cx: &mut StatsCx<'_, H>,
    entity: EntityId,
    pool: PoolId,
    amount: Real,
) -> Result<Changes, StatsError> {
    if !amount.is_finite() || amount < Real::ZERO {
        return Err(StatsError::BadAmount { amount });
    }
    let have = stats_of(cx.stats, entity)?
        .pool_level(cx.timeline, pool)
        .ok_or_else(|| StatsError::NoSuchPool {
            entity,
            pool,
            name: pool_name(cx.defs, pool),
        })?;
    if have < amount {
        return Err(StatsError::PoolTooLow {
            entity,
            pool,
            name: pool_name(cx.defs, pool),
            wanted: amount,
            have,
        });
    }
    let mut changes = Changes::new();
    with_pool(cx, entity, pool, false, &mut changes, |inst, def, out| {
        inst.shift(def, Real::ZERO - amount, entity, out);
        Ok(())
    })?;
    Ok(changes)
}

/// Applies a rate to a pool under a source, replacing the one that
/// source had applied, if any. The pool's next crossing is scheduled
/// again.
pub fn set_rate<H: Host + ?Sized>(
    cx: &mut StatsCx<'_, H>,
    entity: EntityId,
    pool: PoolId,
    source: Source,
    rate: Real,
) -> Result<Changes, StatsError> {
    if !rate.is_finite() {
        return Err(StatsError::BadAmount { amount: rate });
    }
    let mut changes = Changes::new();
    with_pool(cx, entity, pool, false, &mut changes, |inst, _, _| {
        match inst.rates.binary_search_by_key(&source, |rate| rate.source) {
            Ok(at) => inst.rates[at].rate = rate,
            Err(at) => inst.rates.insert(at, PoolRate { source, rate }),
        }
        Ok(())
    })?;
    Ok(changes)
}

/// Takes away the rate a source applied to a pool, if it applied one.
pub fn remove_rate<H: Host + ?Sized>(
    cx: &mut StatsCx<'_, H>,
    entity: EntityId,
    pool: PoolId,
    source: Source,
) -> Result<Changes, StatsError> {
    let mut changes = Changes::new();
    with_pool(cx, entity, pool, false, &mut changes, |inst, _, _| {
        inst.rates.retain(|rate| rate.source != source);
        Ok(())
    })?;
    Ok(changes)
}

/// Takes away every rate a source applied, to any pool of an entity.
pub(super) fn drop_rates<H: Host + ?Sized>(
    cx: &mut StatsCx<'_, H>,
    entity: EntityId,
    source: Source,
    out: &mut Changes,
) -> Result<(), StatsError> {
    let touched: Vec<PoolId> = stats_of(cx.stats, entity)?
        .pools
        .iter()
        .filter(|inst| inst.rates.iter().any(|rate| rate.source == source))
        .map(|inst| inst.pool)
        .collect();
    for pool in touched {
        with_pool(cx, entity, pool, false, out, |inst, _, _| {
            inst.rates.retain(|rate| rate.source != source);
            Ok(())
        })?;
    }
    Ok(())
}

/// Works a pool's `max` and `regen` out again, after something they
/// read changed.
pub(super) fn rework<H: Host + ?Sized>(
    cx: &mut StatsCx<'_, H>,
    entity: EntityId,
    pool: PoolId,
    out: &mut Changes,
) -> Result<(), StatsError> {
    let def = pool_def(cx.defs, pool)?;
    let (max, regen, _) = evaluate(cx, entity, def, false);
    with_pool(cx, entity, pool, false, out, |inst, def, out| {
        inst.rebound(def, max, regen, entity, out);
        Ok(())
    })
}

/// Handles a pool's wake-up: the level has met the threshold it was
/// carried toward. Reports the crossing and schedules the next.
///
/// A wake-up that is not a pool's ([`is_pool_wake`]), or is for a pool
/// that has since gone or been scheduled afresh, changes nothing.
pub fn on_wake<H: Host + ?Sized>(
    cx: &mut StatsCx<'_, H>,
    wake: Wake,
) -> Result<Changes, StatsError> {
    let mut changes = Changes::new();
    if !is_pool_wake(&wake) {
        return Ok(changes);
    }
    let entity = EntityId::from_bits(wake.id);
    let Some(pool) = PoolId::from_index((wake.arg & !POOL_WAKE_TAG) as usize) else {
        return Ok(changes);
    };
    // The pool's own wake-up has fired exactly when the pool still names
    // one and the timeline no longer holds it.
    let fired = cx
        .stats
        .get(entity)
        .and_then(|stats| stats.pool_wake_id(pool))
        .is_some_and(|id| cx.timeline.wake(id).is_none());
    if !fired {
        return Ok(changes);
    }
    with_pool(cx, entity, pool, true, &mut changes, |_, _, _| Ok(()))?;
    Ok(changes)
}

/// Takes an entity's pools out of time: each is brought up to date and
/// its wake-up cancelled, and they stand still until [`resume`]. Call
/// it before the entity is stored, unloaded or destroyed.
pub fn suspend<H: Host + ?Sized>(
    cx: &mut StatsCx<'_, H>,
    entity: EntityId,
) -> Result<Changes, StatsError> {
    let mut changes = Changes::new();
    let defs = cx.defs;
    let stats = stats_mut(cx.stats, entity)?;
    // A clock that is gone cannot say what time it is: the pools are
    // then left as they were last brought up to date. Leaving must not
    // fail, or nothing could be taken out of the simulation.
    let now = stats.local_now(cx.timeline);
    stats.suspended = true;
    for inst in &mut stats.pools {
        if let (Some(now), Some(def)) = (now, defs.pool(inst.pool)) {
            inst.advance(def, now, false, entity, &mut changes);
        }
        if let Some(wake) = inst.wake.take() {
            cx.timeline.cancel(wake);
        }
    }
    Ok(changes)
}

/// Puts an entity's pools back in time, on `clock`, after `elapsed` of
/// local time passed for them: zero if they were paused, the time away
/// if it is to count (03 §9). The pools move by it in one step, the
/// thresholds passed are reported once each in order, and the next
/// crossings are scheduled.
pub fn resume<H: Host + ?Sized>(
    cx: &mut StatsCx<'_, H>,
    entity: EntityId,
    clock: ClockId,
    elapsed: Dur,
) -> Result<Changes, StatsError> {
    let now = cx.timeline.local_now(clock)?;
    // Pools still in time are brought up to date where they are first.
    let mut changes = if stats_of(cx.stats, entity)?.suspended {
        Changes::new()
    } else {
        suspend(cx, entity)?
    };
    let stats = stats_mut(cx.stats, entity)?;
    stats.clock = clock;
    stats.suspended = false;
    let since = now - elapsed.max(Dur::ZERO);
    for inst in &mut stats.pools {
        inst.synced = since;
    }
    let pools: Vec<PoolId> = stats.pools().collect();
    for pool in pools {
        // A pool whose definition is gone stays as it is.
        if cx.defs.pool(pool).is_some() {
            with_pool(cx, entity, pool, false, &mut changes, |_, _, _| Ok(()))?;
        }
    }
    Ok(changes)
}

/// Moves an entity's pools to another clock, when the entity moves to a
/// place with one (03 §3.4). Each pool is brought up to date on the old
/// clock and scheduled on the new, where it has as much local time left
/// to its crossing as it had.
pub fn set_clock<H: Host + ?Sized>(
    cx: &mut StatsCx<'_, H>,
    entity: EntityId,
    clock: ClockId,
) -> Result<Changes, StatsError> {
    cx.timeline.local_now(clock)?;
    if stats_of(cx.stats, entity)?.suspended {
        // Out of time: it will be on the new clock when it returns.
        stats_mut(cx.stats, entity)?.clock = clock;
        return Ok(Changes::new());
    }
    resume(cx, entity, clock, Dur::ZERO)
}
