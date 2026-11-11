//! Wounds (06 §7.3): what a breach leaves on a part, what it drains,
//! and how it closes and heals.
//!
//! # A wound's life
//!
//! | State | Means |
//! |-------|-------|
//! | Open | Its breached layers leak: each one whose tissue has a `leak` drains its pool by `rate × area`, as a rate on the pool under the wound as source. The layers it goes through do not heal. |
//! | Closed | Nothing leaks. Its layers recover at their tissues' `heal` rates. |
//! | Healed | Every layer it went through is whole again. The wound is gone from the body and is reported once. |
//!
//! An open wound closes by itself after the longest `closes` time among
//! its leaking tissues, if each of them gives one; with a tissue that
//! gives none it stays open until [`close_wound`]. A wound through
//! nothing that leaks is closed from the start.
//!
//! # Healing
//!
//! A damaged layer whose tissue has a `heal` rate recovers one 255th at
//! a time, each after `1 / (255 × heal)` seconds of its owner's time,
//! unless an open wound goes through it. Damage that breached nothing
//! (a bruise) heals the same way with no wound to show for it.
//!
//! # Time
//!
//! Nothing runs per tick. The body keeps the local time, on its owner's
//! clock, up to which closing and healing have been worked out, and one
//! wake-up for the earliest thing ahead: a wound closing, or a layer's
//! next 255th. Every function that changes a body first brings it up to
//! that moment, and schedules again when it is done. Times are kept as
//! what is left, not as moments, so a move to another clock or a spell
//! out of the simulation needs no conversion.

use clash_core::{Dur, EntityId, Real, StateHash, StateHasher, impl_state_hash};
use clash_expr::Host;
use clash_time::{ClockId, Wake, WakeDomain};
use clash_world::Cause;
use clash_world::defs::DamageTypeId;
use serde::{Deserialize, Serialize};

use super::defs::DAMAGE_PARTS;
use super::{
    Body, BodyCx, BodyDefs, BodyError, Change, Changes, PartKey, Work, WoundId, body_mut, body_of,
    change, finish, present_part,
};
use crate::stats::{self, PoolId, Source, SourceKind, StatsError};

/// The `arg` of a body's wake-up: what tells it from a pool's crossing
/// (whose `arg` has the top bit set) and from the kernel's own timers,
/// which leave this bit clear or ask [`is_body_wake`] first. Its `id` is
/// the owner's [`EntityId::to_bits`].
pub const BODY_WAKE_ARG: u64 = 1 << 62;

/// The wake-up payload of a body's next closing or healing.
pub fn body_wake(entity: EntityId) -> Wake {
    Wake::new(WakeDomain::Timer, entity.to_bits(), BODY_WAKE_ARG)
}

/// Whether a popped wake-up is a body's, to hand to [`on_wake`].
pub fn is_body_wake(wake: &Wake) -> bool {
    wake.domain == WakeDomain::Timer && wake.arg == BODY_WAKE_ARG
}

/// Whether a wound still leaks (06 §7.3).
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[repr(u8)]
pub enum WoundState {
    /// It leaks, and what it goes through does not heal.
    Open,
    /// It no longer leaks, and is healing.
    Closed,
    /// It is gone. A body never lists a wound in this state: it is what
    /// a wound is reported as when it leaves.
    Healed,
}

impl WoundState {
    /// The word rules read: `open`, `closed`, `healed`.
    pub const fn name(self) -> &'static str {
        match self {
            WoundState::Open => "open",
            WoundState::Closed => "closed",
            WoundState::Healed => "healed",
        }
    }
}

impl StateHash for WoundState {
    fn state_hash(&self, hasher: &mut StateHasher) {
        hasher.write_u8(*self as u8);
    }
}

/// A rate a wound has applied to a pool, as the stats hold it.
#[derive(Clone, Copy, PartialEq, Debug, Serialize, Deserialize)]
pub struct WoundLeak {
    /// The pool drained.
    pub pool: PoolId,
    /// What it loses each second of its owner's time.
    pub rate: Real,
}

impl_state_hash!(WoundLeak { pool, rate });

/// What a breach left on a part (06 §7.3).
#[derive(Clone, PartialEq, Debug, Serialize, Deserialize)]
pub struct Wound {
    pub(super) id: WoundId,
    pub(super) part: PartKey,
    pub(super) kind: DamageTypeId,
    pub(super) depth: u8,
    pub(super) area: Real,
    pub(super) energy: Real,
    pub(super) age: Dur,
    pub(super) state: WoundState,
    /// The local time left before it closes by itself. `None` for one
    /// that will not, and for one that is closed.
    pub(super) closes_in: Option<Dur>,
    /// The part whose loss left it, for a stump.
    pub(super) lost: Option<PartKey>,
    pub(super) cause: Cause,
    /// The rates it has applied, in order of pool.
    pub(super) leaks: Vec<WoundLeak>,
}

impl_state_hash!(Wound {
    id,
    part,
    kind,
    depth,
    area,
    energy,
    age,
    state,
    closes_in,
    lost,
    cause,
    leaks
});

impl Wound {
    /// Its id in its body.
    pub fn id(&self) -> WoundId {
        self.id
    }

    /// The part it is on.
    pub fn part(&self) -> PartKey {
        self.part
    }

    /// The kind of damage that made it.
    pub fn kind(&self) -> DamageTypeId {
        self.kind
    }

    /// How many of the part's layers it goes through, from the outside.
    pub fn depth(&self) -> u8 {
        self.depth
    }

    /// Its area, in square metres.
    pub fn area(&self) -> Real {
        self.area
    }

    /// The joules the part absorbed in making it.
    pub fn energy(&self) -> Real {
        self.energy
    }

    /// How old it was when the body was last brought up to date. Its
    /// age now is [`Body::wound_age`].
    pub fn age(&self) -> Dur {
        self.age
    }

    /// Whether it still leaks.
    pub fn state(&self) -> WoundState {
        self.state
    }

    /// The local time left before it closes by itself, as of when the
    /// body was last brought up to date.
    pub fn closes_in(&self) -> Option<Dur> {
        self.closes_in
    }

    /// The part whose loss left this wound, if it is a stump.
    pub fn lost(&self) -> Option<PartKey> {
        self.lost
    }

    /// Who and what made it.
    pub fn cause(&self) -> Cause {
        self.cause
    }

    /// The rates it has applied to pools, in order of pool.
    pub fn leaks(&self) -> &[WoundLeak] {
        &self.leaks
    }

    /// The source its rates are applied under.
    pub fn source(&self) -> Source {
        wound_source(self.id)
    }
}

/// The source a wound's rates are applied to pools under.
pub fn wound_source(wound: WoundId) -> Source {
    Source::new(SourceKind::Wound, u64::from(wound.raw()))
}

impl Body {
    /// How old a wound is at a local time on the body's clock.
    pub fn wound_age(&self, wound: &Wound, now: Dur) -> Dur {
        if self.suspended {
            return wound.age;
        }
        Dur(wound
            .age
            .0
            .saturating_add((now - self.synced).max(Dur::ZERO).0))
    }

    /// An id no wound of the body has, or `None` when every id is taken.
    pub(super) fn free_wound_id(&mut self) -> Option<WoundId> {
        for _ in 0..=u16::MAX {
            let id = WoundId::new(self.next_wound);
            self.next_wound = self.next_wound.wrapping_add(1);
            if self.wound(id).is_none() {
                return Some(id);
            }
        }
        None
    }

    /// Whether an open wound goes through a layer of a part, which keeps
    /// the layer from healing.
    fn held_open(&self, part: PartKey, layer: usize) -> bool {
        self.wounds.iter().any(|wound| {
            wound.part == part
                && wound.state == WoundState::Open
                && usize::from(wound.depth) > layer
        })
    }

    /// How long a breach of a part to a depth leaks before it closes by
    /// itself: the longest of its leaking tissues' times, nothing if
    /// none of them leaks, and `None` if one of them never closes
    /// unaided.
    pub(super) fn closing_time(&self, defs: &BodyDefs, part: PartKey, depth: u8) -> Option<Dur> {
        let mut longest = Dur::ZERO;
        let Some(part) = self.part(part) else {
            return Some(longest);
        };
        for layer in part.layers().iter().take(usize::from(depth)) {
            let Some(leak) = defs.tissue(layer.material()).and_then(|tissue| tissue.leak) else {
                continue;
            };
            longest = longest.max(leak.closes?);
        }
        Some(longest)
    }

    /// What a wound drains while it is open: each layer it goes through
    /// has its share of the wound's area, by thickness, and one whose
    /// tissue leaks drains `rate × that share`. Summed by pool, in order
    /// of pool.
    fn wanted_leaks(&self, defs: &BodyDefs, wound: &Wound) -> Vec<WoundLeak> {
        let mut leaks: Vec<WoundLeak> = Vec::new();
        if wound.state != WoundState::Open {
            return leaks;
        }
        let Some(part) = self.part(wound.part) else {
            return leaks;
        };
        let depth = usize::from(wound.depth);
        let mut through = Real::ZERO;
        for layer in part.layers().iter().take(depth) {
            through += layer.thickness();
        }
        for layer in part.layers().iter().take(depth) {
            let Some(leak) = defs.tissue(layer.material()).and_then(|tissue| tissue.leak) else {
                continue;
            };
            let rate = leak.rate * wound.area * layer.thickness() / through;
            if !(rate.is_finite() && rate > Real::ZERO) {
                continue;
            }
            match leaks.binary_search_by_key(&leak.pool, |leak| leak.pool) {
                Ok(at) => leaks[at].rate += rate,
                Err(at) => leaks.insert(
                    at,
                    WoundLeak {
                        pool: leak.pool,
                        rate,
                    },
                ),
            }
        }
        leaks
    }

    /// The local time until the next wound closes or the next layer
    /// recovers a 255th, if anything is ahead.
    pub(super) fn next_event(&self, defs: &BodyDefs) -> Option<Dur> {
        let mut next: Option<Dur> = None;
        let mut sooner = |after: Dur| {
            next = Some(next.map_or(after, |next: Dur| next.min(after)));
        };
        for wound in &self.wounds {
            if wound.state == WoundState::Open
                && let Some(closes_in) = wound.closes_in
            {
                sooner(closes_in);
            }
        }
        for part in &self.parts {
            if !part.is_present() {
                continue;
            }
            for (index, layer) in part.layers().iter().enumerate() {
                if layer.damage == 0 || self.held_open(part.key(), index) {
                    continue;
                }
                if let Some(step) = defs
                    .tissue(layer.material())
                    .and_then(|tissue| tissue.heal_step)
                {
                    sooner(step - layer.progress);
                }
            }
        }
        next
    }

    /// Lets a span of local time pass for every wound and every healing
    /// layer. Nothing is closed or healed here: [`settle_due`] does that.
    fn pass(&mut self, defs: &BodyDefs, span: Dur) {
        let healing: Vec<(usize, usize)> = self
            .parts
            .iter()
            .enumerate()
            .filter(|(_, part)| part.is_present())
            .flat_map(|(at, part)| (0..part.layers().len()).map(move |layer| (at, layer)))
            .filter(|(at, layer)| {
                let part = &self.parts[*at];
                let inst = &part.layers()[*layer];
                inst.damage > 0
                    && !self.held_open(part.key(), *layer)
                    && defs
                        .tissue(inst.material())
                        .is_some_and(|tissue| tissue.heal_step.is_some())
            })
            .collect();
        for (at, layer) in healing {
            let inst = &mut self.parts[at].layers[layer];
            inst.progress = Dur(inst.progress.0.saturating_add(span.0));
        }
        for wound in &mut self.wounds {
            wound.age = Dur(wound.age.0.saturating_add(span.0));
            if wound.state == WoundState::Open
                && let Some(closes_in) = &mut wound.closes_in
            {
                *closes_in -= span;
            }
        }
    }

    /// Closes the wounds whose time is up, recovers the 255ths that are
    /// due, and takes away the wounds whose layers are whole again.
    fn settle_due(&mut self, defs: &BodyDefs, entity: EntityId, work: &mut Work) {
        for wound in &mut self.wounds {
            if wound.state == WoundState::Open
                && wound
                    .closes_in
                    .is_some_and(|closes_in| closes_in <= Dur::ZERO)
            {
                wound.state = WoundState::Closed;
                wound.closes_in = None;
                work.changes.push(Change::WoundClosed {
                    entity,
                    wound: wound.id,
                    part: wound.part,
                });
            }
        }
        for at in 0..self.parts.len() {
            if !self.parts[at].is_present() {
                continue;
            }
            let key = self.parts[at].key();
            for index in 0..self.parts[at].layers.len() {
                let layer = self.parts[at].layers[index];
                if layer.damage == 0 {
                    continue;
                }
                let Some(step) = defs
                    .tissue(layer.material)
                    .and_then(|tissue| tissue.heal_step)
                else {
                    continue;
                };
                if self.held_open(key, index) || layer.progress < step {
                    continue;
                }
                let due = (layer.progress.0 / step.0).min(i64::from(layer.damage));
                let layer = &mut self.parts[at].layers[index];
                // `due` is at most the damage, which is a `u8`.
                layer.damage -= u8::try_from(due).unwrap_or(layer.damage);
                layer.progress = if layer.damage == 0 {
                    Dur::ZERO
                } else {
                    Dur(layer.progress.0 - due * step.0)
                };
            }
        }
        let mut healed = Vec::new();
        self.wounds.retain(|wound| {
            let whole = self
                .parts
                .iter()
                .find(|part| part.key() == wound.part)
                .is_none_or(|part| {
                    part.layers()
                        .iter()
                        .take(usize::from(wound.depth))
                        .all(|layer| layer.damage == 0)
                });
            // A stump has no damaged layers to wait for: it is healed
            // once it has closed.
            let done = whole && (wound.lost.is_none() || wound.state == WoundState::Closed);
            if done {
                healed.push((wound.id, wound.part, wound.kind));
            }
            !done
        });
        for (wound, part, kind) in healed {
            work.gone.push(wound);
            work.changes.push(Change::WoundHealed {
                entity,
                wound,
                part,
                kind,
            });
        }
    }
}

/// Lets `span` of local time pass for a body, closing and healing
/// everything that falls due on the way, each at its own moment and in
/// order. With no time to pass it settles what is already due.
pub(super) fn advance(
    body: &mut Body,
    defs: &BodyDefs,
    span: Dur,
    entity: EntityId,
    work: &mut Work,
) {
    let mut left = span.max(Dur::ZERO);
    loop {
        body.settle_due(defs, entity, work);
        if left <= Dur::ZERO {
            return;
        }
        // Whatever is due has just been settled, so what is ahead is
        // ahead by at least a microsecond.
        let step = body
            .next_event(defs)
            .map_or(left, |next| next.clamp(Dur::MICROSECOND, left));
        body.pass(defs, step);
        left -= step;
    }
}

/// Brings a body's closing and healing up to the present moment of its
/// clock. A body out of the simulation stands still.
pub(super) fn sync<H: Host + ?Sized>(
    cx: &mut BodyCx<'_, H>,
    entity: EntityId,
    work: &mut Work,
) -> Result<(), BodyError> {
    let defs = cx.defs;
    let body = body_mut(cx.bodies, entity)?;
    if body.suspended {
        return Ok(());
    }
    let now = cx.stats.timeline.local_now(body.clock)?;
    let span = now - body.synced;
    body.synced = body.synced.max(now);
    advance(body, defs, span, entity, work);
    Ok(())
}

/// Cancels a body's wake-up and schedules the one for whatever closes
/// or heals next, if anything is ahead and the owner is in the
/// simulation.
pub(super) fn reschedule<H: Host + ?Sized>(
    cx: &mut BodyCx<'_, H>,
    entity: EntityId,
) -> Result<(), BodyError> {
    let defs = cx.defs;
    let body = body_mut(cx.bodies, entity)?;
    if let Some(wake) = body.wake.take() {
        cx.stats.timeline.cancel(wake);
    }
    if body.suspended {
        return Ok(());
    }
    if let Some(after) = body.next_event(defs) {
        let after = after.max(Dur::MICROSECOND);
        body.wake = Some(
            cx.stats
                .timeline
                .schedule(body.clock, after, body_wake(entity))?,
        );
    }
    Ok(())
}

/// Whether the stats refused only because there is nothing to leak
/// into: the entity has no stats, or not that pool.
fn nothing_to_leak_into(error: &StatsError) -> bool {
    matches!(
        error,
        StatsError::NoStats { .. } | StatsError::NoSuchPool { .. } | StatsError::UnknownStat { .. }
    )
}

/// Passes on what the stats did, leaving out a refusal that only says
/// there was no pool.
fn stats_did(
    result: Result<stats::Changes, StatsError>,
    out: &mut Changes,
) -> Result<bool, BodyError> {
    match result {
        Ok(changes) => {
            out.extend(changes.into_iter().map(Change::Stats));
            Ok(true)
        }
        Err(error) if nothing_to_leak_into(&error) => Ok(false),
        Err(error) => Err(BodyError::Stats(error)),
    }
}

/// Brings the rates the stats hold into line with the body's wounds:
/// each open wound drains what its breached tissues leak, a closed one
/// nothing, and one that is gone has its source removed.
pub(super) fn reconcile_leaks<H: Host + ?Sized>(
    cx: &mut BodyCx<'_, H>,
    entity: EntityId,
    work: &mut Work,
) -> Result<(), BodyError> {
    for wound in core::mem::take(&mut work.gone) {
        let result = stats::remove_source(&mut cx.stats, entity, wound_source(wound));
        stats_did(result, &mut work.changes)?;
    }
    let defs = cx.defs;
    let count = body_of(cx.bodies, entity)?.wounds.len();
    for at in 0..count {
        let body = body_of(cx.bodies, entity)?;
        let wound = &body.wounds[at];
        let wanted = body.wanted_leaks(defs, wound);
        if wanted == wound.leaks {
            continue;
        }
        let source = wound.source();
        let held = wound.leaks.clone();
        let mut applied = Vec::with_capacity(wanted.len());
        for old in &held {
            if !wanted.iter().any(|leak| leak.pool == old.pool) {
                let result = stats::remove_rate(&mut cx.stats, entity, old.pool, source);
                stats_did(result, &mut work.changes)?;
            }
        }
        for leak in wanted {
            let result = stats::set_rate(
                &mut cx.stats,
                entity,
                leak.pool,
                source,
                Real::ZERO - leak.rate,
            );
            if stats_did(result, &mut work.changes)? {
                applied.push(leak);
            }
        }
        body_mut(cx.bodies, entity)?.wounds[at].leaks = applied;
    }
    Ok(())
}

/// Handles a body's wake-up: a wound's time to close has come, or a
/// layer has recovered a 255th. Reports what followed and schedules the
/// next.
///
/// A wake-up that is not a body's ([`is_body_wake`]), or is for a body
/// that has since gone or been scheduled afresh, changes nothing.
pub fn on_wake<H: Host + ?Sized>(cx: &mut BodyCx<'_, H>, wake: Wake) -> Result<Changes, BodyError> {
    if !is_body_wake(&wake) {
        return Ok(Changes::new());
    }
    let entity = EntityId::from_bits(wake.id);
    // The body's own wake-up has fired exactly when the body still
    // names one and the timeline no longer holds it.
    let fired = cx
        .bodies
        .get(entity)
        .and_then(|body| body.wake)
        .is_some_and(|id| cx.stats.timeline.wake(id).is_none());
    if !fired {
        return Ok(Changes::new());
    }
    let defs = cx.defs;
    let mut work = Work::default();
    let body = body_mut(cx.bodies, entity)?;
    body.wake = None;
    let now = cx.stats.timeline.local_now(body.clock)?;
    // The wake-up is rounded to a whole tick and local time is rounded
    // down, so the clock may read a hair short of the moment the
    // wake-up was for. What it was for has come.
    let due = body.next_event(defs).unwrap_or(Dur::ZERO);
    let span = (now - body.synced).max(due);
    body.synced = body.synced.max(now);
    advance(body, defs, span, entity, &mut work);
    finish(cx, entity, &mut work)?;
    Ok(work.changes)
}

/// Takes a body out of time: its wounds and healing are brought up to
/// date and its wake-up cancelled, and they stand still until
/// [`resume`]. Call it before the entity is stored, unloaded or
/// destroyed.
pub fn suspend<H: Host + ?Sized>(
    cx: &mut BodyCx<'_, H>,
    entity: EntityId,
) -> Result<Changes, BodyError> {
    let mut work = Work::default();
    // A clock that is gone cannot say what time it is: the body is then
    // left as it was last brought up to date. Leaving must not fail.
    let synced = sync(cx, entity, &mut work);
    let body = body_mut(cx.bodies, entity)?;
    body.suspended = true;
    if let Some(wake) = body.wake.take() {
        cx.stats.timeline.cancel(wake);
    }
    if synced.is_ok() {
        finish(cx, entity, &mut work)?;
    }
    Ok(work.changes)
}

/// Puts a body back in time, on `clock`, after `elapsed` of local time
/// passed for it: zero if it was paused, the time away if it is to
/// count (03 §9). Wounds close and layers heal by that much in one
/// step, each reported once in order, and the next wake-up is
/// scheduled.
pub fn resume<H: Host + ?Sized>(
    cx: &mut BodyCx<'_, H>,
    entity: EntityId,
    clock: ClockId,
    elapsed: Dur,
) -> Result<Changes, BodyError> {
    let now = cx.stats.timeline.local_now(clock)?;
    // A body still in time is brought up to date where it is first.
    let mut changes = if body_of(cx.bodies, entity)?.suspended {
        Changes::new()
    } else {
        suspend(cx, entity)?
    };
    let defs = cx.defs;
    let mut work = Work::default();
    let body = body_mut(cx.bodies, entity)?;
    body.clock = clock;
    body.suspended = false;
    body.synced = now;
    advance(body, defs, elapsed, entity, &mut work);
    finish(cx, entity, &mut work)?;
    changes.append(&mut work.changes);
    Ok(changes)
}

/// Moves a body to another clock, when its owner moves to a place with
/// one (03 §3.4). It is brought up to date on the old clock and
/// scheduled on the new, with as much local time left to everything as
/// it had.
pub fn set_clock<H: Host + ?Sized>(
    cx: &mut BodyCx<'_, H>,
    entity: EntityId,
    clock: ClockId,
) -> Result<Changes, BodyError> {
    cx.stats.timeline.local_now(clock)?;
    if body_of(cx.bodies, entity)?.suspended {
        // Out of time: it will be on the new clock when it returns.
        body_mut(cx.bodies, entity)?.clock = clock;
        return Ok(Changes::new());
    }
    resume(cx, entity, clock, Dur::ZERO)
}

/// Closes an open wound (06 §7.3, `close_wound`): it stops leaking and
/// what it goes through begins to heal.
///
/// Refused if the body has no such wound, or the wound is not open.
pub fn close_wound<H: Host + ?Sized>(
    cx: &mut BodyCx<'_, H>,
    entity: EntityId,
    wound: WoundId,
) -> Result<Changes, BodyError> {
    let found = body_of(cx.bodies, entity)?
        .wound(wound)
        .ok_or(BodyError::NoSuchWound { entity, wound })?;
    if found.state != WoundState::Open {
        return Err(BodyError::WoundNotOpen { entity, wound });
    }
    change(cx, entity, |body, _, work| {
        // Time has been brought up to date since it was looked at: it
        // may have closed, or healed, by itself.
        if let Some(found) = body.wounds.iter_mut().find(|found| found.id == wound)
            && found.state == WoundState::Open
        {
            found.state = WoundState::Closed;
            found.closes_in = None;
            work.changes.push(Change::WoundClosed {
                entity,
                wound,
                part: found.part,
            });
        }
        Ok(())
    })
}

/// Opens a closed wound again (06 §7.3, `reopen_wound`): it leaks as it
/// did, what it goes through stops healing, and its time to close by
/// itself starts over.
///
/// Refused if the body has no such wound, or the wound is not closed.
pub fn reopen_wound<H: Host + ?Sized>(
    cx: &mut BodyCx<'_, H>,
    entity: EntityId,
    wound: WoundId,
) -> Result<Changes, BodyError> {
    let found = body_of(cx.bodies, entity)?
        .wound(wound)
        .ok_or(BodyError::NoSuchWound { entity, wound })?;
    if found.state != WoundState::Closed {
        return Err(BodyError::WoundNotClosed { entity, wound });
    }
    change(cx, entity, |body, defs, work| {
        let Some(at) = body.wounds.iter().position(|found| found.id == wound) else {
            return Ok(());
        };
        let (part, depth) = (body.wounds[at].part, body.wounds[at].depth);
        let closes_in = body.closing_time(defs, part, depth);
        let found = &mut body.wounds[at];
        found.state = WoundState::Open;
        found.closes_in = closes_in;
        work.changes.push(Change::WoundReopened {
            entity,
            wound,
            part,
        });
        Ok(())
    })
}

/// Takes damage off every layer of a part (06 §6.5, `heal_part`): a
/// share of a whole layer, from each. A wound whose layers are whole
/// again is healed.
///
/// Refused for a part that is not there, and for a share that is not
/// above zero.
pub fn heal_part<H: Host + ?Sized>(
    cx: &mut BodyCx<'_, H>,
    entity: EntityId,
    part: PartKey,
    share: Real,
) -> Result<Changes, BodyError> {
    if !(share.is_finite() && share > Real::ZERO) {
        return Err(BodyError::BadAmount { amount: share });
    }
    present_part(body_of(cx.bodies, entity)?, entity, part)?;
    let whole = Real::from_i64(i64::from(DAMAGE_PARTS));
    let taken = (share.min(Real::ONE) * whole).ceil().to_i64();
    let taken = u8::try_from(taken).unwrap_or(DAMAGE_PARTS);
    change(cx, entity, |body, _, _| {
        if let Some(found) = body.part_mut(part) {
            for layer in &mut found.layers {
                layer.damage = layer.damage.saturating_sub(taken);
                if layer.damage == 0 {
                    layer.progress = Dur::ZERO;
                }
            }
        }
        Ok(())
    })
}
