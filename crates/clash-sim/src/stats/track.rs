//! Tracks (06 §4.3): an accumulating counter with levels.
//!
//! A track holds what has accumulated and the level reached. Its
//! definition's `levels` is asked, for each level in turn, the total the
//! counter must have reached to be at that level; the track is at the
//! highest level whose total it has. How a track gains is rules
//! (decision D-038): they call [`advance_track`].
//!
//! Reaching a level is reported once for that level. An advance that
//! passes several reports each, lowest first. A level, once reached, is
//! kept: the curve is asked only about the levels above it.

use clash_core::{EntityId, Real};
use clash_data::Def;
use clash_expr::{Host, Value};

use super::defs::{TrackDef, TrackId};
use super::props::Scoped;
use super::{Change, Changes, StatsCx, StatsError, TrackInst, stats_mut, stats_of};

/// Adds to a track's counter and raises its level as far as the counter
/// now reaches. Returns one [`Change::TrackAdvanced`] for each level
/// reached, lowest first.
///
/// The amount must be finite and not negative: a track only
/// accumulates.
pub fn advance_track<H: Host + ?Sized>(
    cx: &mut StatsCx<'_, H>,
    entity: EntityId,
    track: TrackId,
    amount: Real,
) -> Result<Changes, StatsError> {
    let def = cx.defs.track(track).ok_or(StatsError::UnknownStat {
        what: TrackDef::TYPE,
    })?;
    if !amount.is_finite() || amount < Real::ZERO {
        return Err(StatsError::BadAmount { amount });
    }
    let stats = stats_of(cx.stats, entity)?;
    let total = stats.track_amount(track) + amount;
    let mut level = stats.track_level(track);
    let limit = def.level_limit();

    let mut changes = Changes::new();
    let mut host = Scoped {
        stats: &*cx.stats,
        defs: cx.defs,
        timeline: &*cx.timeline,
        inner: &mut *cx.host,
        owner: entity,
        second: Value::Null,
        local: Value::Null,
        allowed: None,
    };
    let exprs = cx.defs.defs().exprs();
    while level < limit {
        host.local = Value::Int(i64::from(level) + 1);
        // A curve that gives nothing for a level has no such level.
        let Some(needed) = def.levels.eval(&mut cx.runner.over(exprs, &mut host)) else {
            break;
        };
        if total < needed {
            break;
        }
        level += 1;
        changes.push(Change::TrackAdvanced {
            entity,
            track,
            level,
        });
    }

    let stats = stats_mut(cx.stats, entity)?;
    match stats.track_at(track) {
        Ok(at) => {
            stats.tracks[at].amount = total;
            stats.tracks[at].level = level;
        }
        Err(at) => stats.tracks.insert(
            at,
            TrackInst {
                track,
                amount: total,
                level,
            },
        ),
    }
    Ok(changes)
}
