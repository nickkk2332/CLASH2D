//! Detection (15 §5): awareness for each observer, sense and target,
//! built by what is received and faded by what is not, and the sweeps
//! that read standing signatures.
//!
//! ```text
//! ratio = received / threshold
//! if ratio ≥ 1:   level += gain × (ratio − 1 + ε) × dt
//! else:           level −= decay × dt
//! ```
//!
//! For a sweep, `dt` is the time since the sense last swept; for a
//! signal that was given off, which has no length of its own, it is the
//! sense's interval: a signal counts as standing for one sweep.

use std::sync::Arc;

use clash_core::{Dur, EntityId, Real, StateHash, StateHasher, impl_state_hash};
use clash_world::{Activity, Location};
use serde::{Deserialize, Serialize};

use super::defs::{SenseId, SignalModel};
use super::signal::{self, Spot};
use super::{AWARENESS_EPSILON, MAX_AWARENESS_SETTING, belief, events, party, senses};
use crate::reason::Reason;
use crate::{Place, Sim};

/// The most targets an observer is aware of through all its senses when
/// the setting `perception.max_awareness` is not given.
pub const DEFAULT_MAX_AWARENESS: i64 = 64;

/// The least a threshold is taken to be, so that a sense whose threshold
/// comes to zero still divides by something.
const MIN_THRESHOLD: Real = Real::of(1e-9);

/// One observer's awareness of one target through one sense.
#[derive(Clone, PartialEq, Debug, Serialize, Deserialize)]
pub struct AwareEntry {
    target: EntityId,
    sense: SenseId,
    level: Real,
    /// The observer's local time when it was last brought up to date.
    at: Dur,
    suspected: bool,
    detected: bool,
}

impl_state_hash!(AwareEntry {
    target,
    sense,
    level,
    at,
    suspected,
    detected
});

impl AwareEntry {
    /// Of whom or what.
    pub fn target(&self) -> EntityId {
        self.target
    }

    /// Through which sense.
    pub fn sense(&self) -> SenseId {
        self.sense
    }

    /// The level, from 0 to 1.
    pub fn level(&self) -> Real {
        self.level
    }

    /// Whether the level has crossed the sense's `suspect` and not fallen
    /// back below it.
    pub fn is_suspected(&self) -> bool {
        self.suspected
    }

    /// Whether the level has reached 1 and not yet fallen to 0.
    pub fn is_detected(&self) -> bool {
        self.detected
    }
}

/// What an observer is aware of (15 §5.1): the `Awareness` component.
/// Pairs with nothing received and no awareness are not kept.
#[derive(Clone, PartialEq, Debug, Default, Serialize, Deserialize)]
pub struct Awareness {
    /// In order of target and then sense.
    entries: Vec<AwareEntry>,
}

impl StateHash for Awareness {
    fn state_hash(&self, hasher: &mut StateHasher) {
        self.entries.state_hash(hasher);
    }
}

impl Awareness {
    /// Every entry, in order of target and then sense.
    pub fn entries(&self) -> &[AwareEntry] {
        &self.entries
    }

    /// The level of awareness of a target through one sense.
    pub fn level(&self, target: EntityId, sense: SenseId) -> Option<Real> {
        self.find(target, sense)
            .ok()
            .map(|at| self.entries[at].level)
    }

    /// The highest level of awareness of a target through any sense.
    pub fn of(&self, target: EntityId) -> Option<Real> {
        self.of_target(target)
            .map(|entry| entry.level)
            .reduce(Real::max)
    }

    /// Whether a target is detected through any sense.
    pub fn detects(&self, target: EntityId) -> bool {
        self.of_target(target).any(|entry| entry.detected)
    }

    fn of_target(&self, target: EntityId) -> impl Iterator<Item = &AwareEntry> + '_ {
        let start = self.entries.partition_point(|entry| entry.target < target);
        self.entries[start..]
            .iter()
            .take_while(move |entry| entry.target == target)
    }

    fn find(&self, target: EntityId, sense: SenseId) -> Result<usize, usize> {
        self.entries
            .binary_search_by(|entry| (entry.target, entry.sense).cmp(&(target, sense)))
    }

    /// The targets kept for one sense, in order.
    fn targets_of(&self, sense: SenseId) -> Vec<EntityId> {
        self.entries
            .iter()
            .filter(|entry| entry.sense == sense)
            .map(|entry| entry.target)
            .collect()
    }

    /// Forgets everything known through one sense.
    pub(super) fn forget_sense(&mut self, sense: SenseId) {
        self.entries.retain(|entry| entry.sense != sense);
    }
}

/// What a sense made of a target: how many times over its threshold, and
/// where it seemed to be.
#[derive(Clone, Copy, PartialEq, Debug)]
pub(super) struct Reading {
    pub(super) ratio: Real,
    pub(super) at: Location,
}

/// The local time of an observer's clock, or zero for one on no clock.
pub(super) fn local_now(sim: &Sim, entity: EntityId) -> Dur {
    sim.clock_of(entity)
        .and_then(|clock| sim.timeline.local_now(clock).ok())
        .unwrap_or(Dur::ZERO)
}

/// Whether a cell is in a warm tract: loaded, with the simulation
/// throttled (15 §14). A tract with no activity recorded is hot.
pub(super) fn is_warm(sim: &Sim, loc: Location) -> bool {
    sim.cosmos
        .realm_of(loc.grid)
        .is_some_and(|realm| sim.cosmos.activity(realm, loc.cell.tract()) == Some(Activity::Warm))
}

/// Brings one pair up to date: builds awareness by what was received, or
/// fades it, over `dt` seconds, and raises what crossing a level raises.
fn update(
    sim: &mut Sim,
    observer: EntityId,
    sense: SenseId,
    target: EntityId,
    reading: Option<Reading>,
    dt: Real,
    now: Dur,
) {
    let defs = Arc::clone(&sim.perception_defs);
    let Some(def) = defs.sense(sense) else {
        return;
    };
    let delta = match reading {
        Some(reading) if reading.ratio >= Real::ONE => {
            def.gain * (reading.ratio - Real::ONE + AWARENESS_EPSILON) * dt
        }
        _ => -(def.decay * dt),
    };
    let received = reading.is_some_and(|reading| reading.ratio >= Real::ONE);
    let max = sim
        .setting_int(MAX_AWARENESS_SETTING, DEFAULT_MAX_AWARENESS)
        .max(1) as usize;
    let awareness = sim
        .components
        .awareness
        .get_or_insert_with(observer, Awareness::default);
    let at = match awareness.find(target, sense) {
        Ok(at) => at,
        Err(at) => {
            if !received {
                return;
            }
            awareness.entries.insert(
                at,
                AwareEntry {
                    target,
                    sense,
                    level: Real::ZERO,
                    at: now,
                    suspected: false,
                    detected: false,
                },
            );
            at
        }
    };
    let entry = &mut awareness.entries[at];
    let before = entry.clone();
    entry.level = (entry.level + delta).clamp(Real::ZERO, Real::ONE);
    entry.at = now;
    let level = entry.level;
    let mut raise_suspicion = false;
    let mut raise_detected = false;
    let mut raise_lost = false;
    if level >= def.suspect && !before.suspected {
        entry.suspected = true;
        raise_suspicion = true;
    } else if level < def.suspect {
        entry.suspected = false;
    }
    if level >= Real::ONE && !before.detected {
        entry.detected = true;
        raise_detected = true;
    }
    let gone = level <= Real::ZERO;
    if gone {
        raise_lost = before.detected;
        awareness.entries.remove(at);
    }
    let detected_now = !gone && (before.detected || raise_detected);
    trim(sim, observer, max);

    let sense_name = defs.sense_name(sense).to_owned();
    let kind = defs.sense_kind(sense);
    if raise_suspicion {
        events::suspicion_raised(sim, observer, target, &sense_name, level);
    }
    if detected_now
        && received
        && let (Some(reading), Some(kind)) = (reading, kind)
    {
        belief::perceived_at(sim, observer, target, reading.at, kind);
    }
    if raise_detected {
        let at = reading.map(|reading| reading.at);
        events::detected(sim, observer, target, &sense_name, at);
        let clarity = reading.map_or(Real::ONE, |reading| reading.ratio);
        events::perceived(
            sim,
            events::Percept {
                observer,
                about: Some(target),
                percept: events::PERCEPT_DETECTION,
                sense: sense_name.clone(),
                signal: kind
                    .map(|kind| defs.kind_name(kind).to_owned())
                    .unwrap_or_default(),
                event: None,
                about_actor: None,
                at,
                clarity,
            },
        );
    }
    if raise_lost {
        let last = belief::believed_loc(sim, observer, target);
        events::lost_track(sim, observer, target, &sense_name, last);
    }
}

/// Keeps an observer's entries within `max`: the faintest go first, then
/// the longest unchanged, then by target.
fn trim(sim: &mut Sim, observer: EntityId, max: usize) {
    let Some(awareness) = sim.components.awareness.get_mut(observer) else {
        return;
    };
    while awareness.entries.len() > max {
        let Some(weakest) = awareness
            .entries
            .iter()
            .enumerate()
            .min_by(|(_, a), (_, b)| {
                (a.detected, a.level, a.at, a.target, a.sense)
                    .cmp(&(b.detected, b.level, b.at, b.target, b.sense))
            })
            .map(|(at, _)| at)
        else {
            return;
        };
        awareness.entries.remove(weakest);
    }
}

/// A signal of a sense's kind about `target` reached `observer`: its
/// awareness builds as if the signal had stood for one interval.
pub(super) fn emitted(
    sim: &mut Sim,
    observer: EntityId,
    sense: SenseId,
    target: EntityId,
    reading: Reading,
) {
    let Some(interval) = sim
        .perception_defs
        .sense(sense)
        .map(|def| def.interval.dur().as_secs_real())
    else {
        return;
    };
    let now = local_now(sim, observer);
    update(sim, observer, sense, target, Some(reading), interval, now);
}

/// What one sense of an observer at `eye` makes of one target's standing
/// signature now, or `None` if nothing of it arrives.
fn read(
    sim: &Sim,
    observer: EntityId,
    sense: SenseId,
    eye: Spot,
    threshold: Real,
    target: EntityId,
) -> Option<Reading> {
    let defs = &sim.perception_defs;
    let def = defs.sense(sense)?;
    let kind_id = defs.sense_kind(sense)?;
    let kind = defs.kind(kind_id)?;
    let strength = senses::standing_signature(sim, target, kind_id);
    if strength <= Real::ZERO {
        return None;
    }
    if !def.planes {
        let mine = sim.entities.get(observer)?.presence().plane;
        let top = sim.entities.outermost(target)?;
        let theirs = sim.entities.get(top)?.presence().plane;
        if !mine.shares(theirs) {
            return None;
        }
    }
    let source = signal::source_of(sim, target)?;
    let facing = sim.components.senses.get(observer)?.facing();
    if !senses::within_arc(facing, def.arc, senses::bearing(sim, eye.loc, source.loc)) {
        return None;
    }
    let mut received = signal::strength_at(sim, kind, strength, source, eye);
    if matches!(sim.entities.get(observer)?.place(), Place::In(_)) {
        received *= kind.contained;
    }
    if received <= Real::ZERO {
        return None;
    }
    Some(Reading {
        ratio: received / threshold.max(MIN_THRESHOLD),
        at: source.loc,
    })
}

/// One sweep of one sense (15 §5.2): the next is scheduled, then every
/// target within range, and every target already in the observer's
/// awareness through this sense, is brought up to date. A warm observer
/// does not sweep (15 §14); a sense its body cannot work receives
/// nothing.
pub(super) fn sweep(sim: &mut Sim, observer: EntityId, sense: SenseId, bonus: Real) {
    let defs = Arc::clone(&sim.perception_defs);
    let Some(def) = defs.sense(sense) else {
        return;
    };
    let interval = def.interval.dur();
    senses::schedule_sweep(sim, observer, sense, interval);
    let Some(eye) = signal::eye_of(sim, observer) else {
        return;
    };
    if is_warm(sim, eye.loc) {
        return;
    }
    let Some(kind_id) = defs.sense_kind(sense) else {
        return;
    };
    let Some(kind) = defs.kind(kind_id) else {
        return;
    };
    let now = local_now(sim, observer);
    let works = senses::sense_works(sim, observer, def.function);
    let threshold = if works {
        senses::threshold_of(sim, observer, sense)
    } else {
        None
    }
    .map(|threshold| threshold / bonus.max(MIN_THRESHOLD));
    let Some(slot) = sim
        .components
        .senses
        .get_mut(observer)
        .and_then(|senses| senses.slot_mut(sense))
    else {
        return;
    };
    let dt = slot
        .last_sweep
        .map_or(interval, |last| now - last)
        .as_secs_real()
        .max(Real::ZERO);
    slot.last_sweep = Some(now);
    slot.threshold = threshold;

    if kind.model == SignalModel::Field {
        field_sweep(sim, observer, sense, eye, threshold);
        return;
    }
    let mut targets = match threshold {
        Some(_) => signal::candidates(sim, observer, eye.loc, def.range.get()),
        None => Vec::new(),
    };
    let tracked = sim
        .components
        .awareness
        .get(observer)
        .map(|awareness| awareness.targets_of(sense))
        .unwrap_or_default();
    let in_range = targets.clone();
    targets.extend(tracked);
    targets.sort_unstable();
    targets.dedup();
    for target in targets {
        let reading = match threshold {
            Some(threshold) if in_range.binary_search(&target).is_ok() => {
                read(sim, observer, sense, eye, threshold, target)
            }
            _ => None,
        };
        update(sim, observer, sense, target, reading, dt, now);
    }
    if kind.uses_light {
        party::after_sweep(sim, observer, sense);
    }
}

/// A sweep of a sense of the `field` model: the observer reads the field
/// in its cell, and perceives it when the reading first reaches the
/// threshold, with where it seems to come from: the neighbour on its
/// storey with the most, if any has more than its own cell.
fn field_sweep(
    sim: &mut Sim,
    observer: EntityId,
    sense: SenseId,
    eye: Spot,
    threshold: Option<Real>,
) {
    let defs = Arc::clone(&sim.perception_defs);
    let Some(kind_id) = defs.sense_kind(sense) else {
        return;
    };
    let Some(field) = defs.kind(kind_id).and_then(|kind| kind.field) else {
        return;
    };
    let level_at = |sim: &Sim, loc: Location| -> u8 {
        sim.cosmos
            .field_cell(loc, field)
            .ok()
            .flatten()
            .map_or(0, |cell| cell.level)
    };
    let own = level_at(sim, eye.loc);
    let reading = Real::from(own) / Real::from(u8::MAX);
    let clarity = threshold.map_or(Real::ZERO, |threshold| {
        reading / threshold.max(MIN_THRESHOLD)
    });
    let above = clarity >= Real::ONE;
    let was = sim
        .components
        .senses
        .get_mut(observer)
        .and_then(|senses| senses.slot_mut(sense))
        .map(|slot| core::mem::replace(&mut slot.above, above))
        .unwrap_or(false);
    if !above || was {
        return;
    }
    let mut from = eye.loc;
    let mut most = own;
    for dir in clash_core::Dir8::ALL {
        let delta = dir.delta();
        let Ok(next) = sim.cosmos.normalize(Location::new(
            eye.loc.grid,
            clash_world::CellPos::new(
                eye.loc.cell.x + delta.x,
                eye.loc.cell.y + delta.y,
                eye.loc.cell.z,
            ),
        )) else {
            continue;
        };
        let level = level_at(sim, next);
        if level > most {
            most = level;
            from = next;
        }
    }
    events::perceived(
        sim,
        events::Percept {
            observer,
            about: None,
            percept: events::PERCEPT_SIGNAL,
            sense: defs.sense_name(sense).to_owned(),
            signal: defs.kind_name(kind_id).to_owned(),
            event: None,
            about_actor: None,
            at: Some(from),
            clarity,
        },
    );
}

/// An observer moved or turned (15 §5.2): each of its senses that has
/// not swept for a quarter of its interval sweeps at once, at the next
/// wake-up.
pub(super) fn observer_moved(sim: &mut Sim, observer: EntityId) {
    let Some(senses) = sim.components.senses.get(observer) else {
        return;
    };
    let now = local_now(sim, observer);
    let due: Vec<SenseId> = senses
        .slots()
        .iter()
        .filter(|slot| {
            let quarter = sim
                .perception_defs
                .sense(slot.sense())
                .map_or(Dur::ZERO, |def| {
                    Dur::from_micros(def.interval.dur().as_micros() / 4)
                });
            slot.last_sweep().is_none_or(|last| now - last >= quarter)
        })
        .map(|slot| slot.sense())
        .collect();
    for sense in due {
        senses::schedule_sweep(sim, observer, sense, Dur::ZERO);
    }
}

/// An explicit search or listen (15 §5.2): every sense of the observer
/// sweeps now, with its threshold divided by `bonus` (1 for none). The
/// next sweeps follow a whole interval later. Refused for an entity
/// with no senses and a bonus that is not above zero.
pub fn search(sim: &mut Sim, observer: EntityId, bonus: Real) -> Result<(), Reason> {
    sim.known(observer)?;
    if !bonus.is_finite() || bonus <= Real::ZERO {
        return Err(
            Reason::new(super::reason::BAD_VALUE, "a search's bonus is above zero")
                .with("value", bonus),
        );
    }
    let senses: Vec<SenseId> = sim
        .components
        .senses
        .get(observer)
        .ok_or_else(|| senses::no_senses(observer))?
        .slots()
        .iter()
        .map(|slot| slot.sense())
        .collect();
    for sense in senses {
        sweep(sim, observer, sense, bonus);
    }
    Ok(())
}

/// Whether `observer` currently has `target` detected (15 §6): its
/// awareness of it reached 1 through some sense and has not since fallen
/// to 0.
pub fn perceives(sim: &Sim, observer: EntityId, target: EntityId) -> bool {
    sim.components
        .awareness
        .get(observer)
        .is_some_and(|awareness| awareness.detects(target))
}

/// The highest awareness `observer` has of `target` through any sense,
/// if it has any.
pub fn awareness_of(sim: &Sim, observer: EntityId, target: EntityId) -> Option<Real> {
    sim.components
        .awareness
        .get(observer)
        .and_then(|awareness| awareness.of(target))
}

/// Sets an observer's awareness of a target through one of its senses
/// (the `set_awareness` effect, 15 §15), raising what crossing a level
/// raises. A level of 0 forgets the pair. Refused for an observer
/// without the sense and a level outside 0 to 1.
pub fn set_awareness(
    sim: &mut Sim,
    observer: EntityId,
    target: EntityId,
    sense: SenseId,
    level: Real,
) -> Result<(), Reason> {
    sim.known(observer)?;
    sim.known(target)?;
    if !level.is_finite() || level < Real::ZERO || level > Real::ONE {
        return Err(Reason::new(
            super::reason::BAD_VALUE,
            "an awareness level is from 0 to 1",
        )
        .with("value", level));
    }
    let has = sim
        .components
        .senses
        .get(observer)
        .is_some_and(|senses| senses.has(sense));
    if !has {
        return Err(Reason::new(
            super::reason::NO_SENSE,
            format!("{observer} does not have that sense"),
        )
        .with("entity", observer.to_string())
        .with("sense", sim.perception_defs.sense_name(sense)));
    }
    let now = local_now(sim, observer);
    let current = sim
        .components
        .awareness
        .get(observer)
        .and_then(|awareness| awareness.level(target, sense))
        .unwrap_or(Real::ZERO);
    let at = signal::source_of(sim, target).map(|spot| spot.loc);
    let reading = at.map(|at| Reading {
        ratio: Real::of(2.0),
        at,
    });
    if level > current {
        // Built as if received, by exactly the difference.
        let gain = sim
            .perception_defs
            .sense(sense)
            .map_or(Real::ONE, |def| def.gain)
            .max(MIN_THRESHOLD);
        let dt = (level - current) / (gain * (Real::ONE + AWARENESS_EPSILON));
        update(sim, observer, sense, target, reading, dt, now);
        if let Some(entry) = sim
            .components
            .awareness
            .get_mut(observer)
            .and_then(|awareness| {
                let at = awareness.find(target, sense).ok()?;
                awareness.entries.get_mut(at)
            })
        {
            entry.level = level;
        }
    } else if level < current {
        let decay = sim
            .perception_defs
            .sense(sense)
            .map_or(Real::ONE, |def| def.decay)
            .max(MIN_THRESHOLD);
        let dt = (current - level) / decay;
        update(sim, observer, sense, target, None, dt, now);
        if let Some(entry) = sim
            .components
            .awareness
            .get_mut(observer)
            .and_then(|awareness| {
                let at = awareness.find(target, sense).ok()?;
                awareness.entries.get_mut(at)
            })
        {
            entry.level = level;
        }
    }
    Ok(())
}
