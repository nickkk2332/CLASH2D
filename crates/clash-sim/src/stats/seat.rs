//! Seats (06 §8.2): what of an entity's stats goes with its mind when
//! the two part company, and what stays with the body.
//!
//! An attribute, a pool and a track each state a `seat`, `body` (the
//! default) or `mind`. [`take_seated`] takes out of an entity's stats
//! everything of one seat that is the entity's own: the bases it was
//! given, the modifiers on those attributes, the pools with their
//! levels and rates, and the tracks. [`give_seated`] puts such a bundle
//! into another entity's stats. Between the two the bundle is plain
//! data.
//!
//! An attribute's value is not moved: every entity with stats has every
//! attribute, and the value follows from the base and the modifiers
//! that were.

use clash_core::{EntityId, Real};
use clash_expr::Host;

use super::defs::{AttributeId, PoolId, Seat, TrackId};
use super::pool::PoolRate;
use super::{
    AttrInst, Changes, Modifier, StatsCx, StatsError, TrackInst, attr, pool, stats_mut, stats_of,
};

/// One pool on its way from one entity to another.
#[derive(Clone, PartialEq, Debug)]
pub struct SeatedPool {
    /// Which pool.
    pub pool: PoolId,
    /// What it held when it was taken.
    pub level: Real,
    /// The rates applied to it, in order of source.
    pub rates: Vec<PoolRate>,
}

/// What of an entity's stats is seated in one place: what
/// [`take_seated`] took and [`give_seated`] gives.
#[derive(Clone, PartialEq, Debug, Default)]
pub struct Seated {
    /// The bases of its own the entity had, by attribute.
    pub bases: Vec<(AttributeId, Real)>,
    /// The modifiers on attributes of the seat.
    pub modifiers: Vec<Modifier>,
    /// The pools of the seat.
    pub pools: Vec<SeatedPool>,
    /// The tracks of the seat: what each had accumulated and its level.
    pub tracks: Vec<(TrackId, Real, u32)>,
}

impl Seated {
    /// Whether there is nothing in the bundle.
    pub fn is_empty(&self) -> bool {
        self.bases.is_empty()
            && self.modifiers.is_empty()
            && self.pools.is_empty()
            && self.tracks.is_empty()
    }
}

/// Takes out of an entity's stats everything seated in `seat`, and works
/// the rest out again. Returns what was taken and what changed.
pub fn take_seated<H: Host + ?Sized>(
    cx: &mut StatsCx<'_, H>,
    entity: EntityId,
    seat: Seat,
) -> Result<(Seated, Changes), StatsError> {
    let defs = cx.defs;
    let attr_seated = |attr: AttributeId| defs.attribute(attr).is_some_and(|def| def.seat == seat);
    let stats = stats_mut(cx.stats, entity)?;
    let now = stats.local_now(cx.timeline);
    let mut taken = Seated::default();
    for inst in &mut stats.attrs {
        if attr_seated(inst.attr)
            && let Some(base) = inst.base.take()
        {
            taken.bases.push((inst.attr, base));
        }
    }
    stats.modifiers.retain(|modifier| {
        let goes = attr_seated(modifier.attr);
        if goes {
            taken.modifiers.push(*modifier);
        }
        !goes
    });
    let mut kept = Vec::with_capacity(stats.pools.len());
    for inst in core::mem::take(&mut stats.pools) {
        if defs.pool(inst.pool).is_some_and(|def| def.seat == seat) {
            if let Some(wake) = inst.wake() {
                cx.timeline.cancel(wake);
            }
            taken.pools.push(SeatedPool {
                pool: inst.pool,
                level: now.map_or_else(|| inst.level_as_synced(), |now| inst.level_at(now)),
                rates: inst.rates().to_vec(),
            });
        } else {
            kept.push(inst);
        }
    }
    stats.pools = kept;
    stats.tracks.retain(|inst| {
        let goes = defs.track(inst.track).is_some_and(|def| def.seat == seat);
        if goes {
            taken.tracks.push((inst.track, inst.amount, inst.level));
        }
        !goes
    });
    let changes = attr::refresh(cx, entity)?;
    Ok((taken, changes))
}

/// Puts a bundle of seated stats into an entity's stats, in place of
/// whatever of the same it had, and works everything out again. A pool
/// arrives at the level it was taken at, or at its new owner's `max` if
/// that is less, with the rates it had.
pub fn give_seated<H: Host + ?Sized>(
    cx: &mut StatsCx<'_, H>,
    entity: EntityId,
    seated: Seated,
) -> Result<Changes, StatsError> {
    let stats = stats_mut(cx.stats, entity)?;
    for (attr, base) in seated.bases {
        match stats.attr_at(attr) {
            Ok(at) => stats.attrs[at].base = Some(base),
            Err(at) => stats.attrs.insert(
                at,
                AttrInst {
                    attr,
                    base: Some(base),
                    value: Real::ZERO,
                },
            ),
        }
    }
    for modifier in seated.modifiers {
        let key = (modifier.attr, modifier.source);
        let at = stats
            .modifiers
            .partition_point(|other| (other.attr, other.source) <= key);
        stats.modifiers.insert(at, modifier);
    }
    for (track, amount, level) in seated.tracks {
        let inst = TrackInst {
            track,
            amount,
            level,
        };
        match stats.track_at(track) {
            Ok(at) => stats.tracks[at] = inst,
            Err(at) => stats.tracks.insert(at, inst),
        }
    }
    let mut changes = attr::refresh(cx, entity)?;
    for seated in seated.pools {
        if stats_of(cx.stats, entity)?.has_pool(seated.pool) {
            pool::remove_pool(cx, entity, seated.pool)?;
        }
        pool::add_pool_at(
            cx,
            entity,
            seated.pool,
            seated.level,
            seated.rates,
            &mut changes,
        )?;
    }
    Ok(changes)
}
