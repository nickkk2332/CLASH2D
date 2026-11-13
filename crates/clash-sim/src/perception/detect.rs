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
use clash_world::{Activity, Location, OccupantFilter};
use serde::{Deserialize, Serialize};

use super::defs::{SenseId, SignalModel};
use super::signal::{self, Spot};
use super::{
    AWARENESS_EPSILON, MAX_AWARENESS_SETTING, MAX_CANDIDATES_SETTING, belief, events, party, senses,
};
use crate::reason::Reason;
use crate::{Place, Sim};

/// The most pairs an observer keeps awareness of through all its senses
/// when the setting `perception.max_awareness` is not given.
pub const DEFAULT_MAX_AWARENESS: i64 = 128;

/// The most entities one sweep of one sense considers, the nearest
/// first, when the setting `perception.max_candidates` is not given;
/// those already in the observer's awareness are considered as well.
pub const DEFAULT_MAX_CANDIDATES: i64 = 32;

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
#[allow(
    clippy::too_many_arguments,
    reason = "one pair's update, with what the sweep worked out once for all of them"
)]
fn update(
    sim: &mut Sim,
    observer: EntityId,
    sense: SenseId,
    target: EntityId,
    reading: Option<Reading>,
    dt: Real,
    now: Dur,
    max: usize,
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

    // The sense's name is looked up only for an event that needs it.
    let sense_name = || defs.sense_name(sense);
    let kind = defs.sense_kind(sense);
    if raise_suspicion {
        events::suspicion_raised(sim, observer, target, sense_name(), level);
    }
    if raise_detected {
        // Who the observer takes it for is settled before anything is
        // believed of it (15 §8.2).
        crate::social::identify(sim, observer, target, sense);
    }
    if detected_now
        && received
        && let (Some(reading), Some(kind)) = (reading, kind)
    {
        belief::perceived_at(sim, observer, target, reading.at, kind);
    }
    if raise_detected {
        let at = reading.map(|reading| reading.at);
        events::detected(sim, observer, target, sense_name(), at);
        let clarity = reading.map_or(Real::ONE, |reading| reading.ratio);
        events::perceived(
            sim,
            events::Percept {
                observer,
                about: Some(target),
                percept: events::PERCEPT_DETECTION,
                sense: sense_name().to_owned(),
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
        events::lost_track(sim, observer, target, sense_name(), last);
        // Out of every sense, it is taken for whatever it presents when
        // next it is detected (15 §8.2).
        if !perceives(sim, observer, target) {
            crate::social::lost_sight(sim, observer, target);
        }
    }
}

/// The most pairs an observer keeps: the setting
/// `perception.max_awareness`, at least one.
fn max_awareness(sim: &Sim) -> usize {
    sim.setting_int(MAX_AWARENESS_SETTING, DEFAULT_MAX_AWARENESS)
        .max(1) as usize
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
    let max = max_awareness(sim);
    update(
        sim,
        observer,
        sense,
        target,
        Some(reading),
        interval,
        now,
        max,
    );
}

/// What a sweep of one sense works out once for every target: the
/// sense, its kind, the observer's eye, facing and plane, and whether it
/// is inside something.
struct Sweeper<'a> {
    def: &'a super::defs::SenseDef,
    kind_id: super::defs::SignalKindId,
    kind: &'a super::defs::KindInfo,
    eye: Spot,
    threshold: Real,
    facing: Real,
    plane: clash_world::PlaneMask,
    contained: bool,
}

/// What one sense of an observer at `eye` makes of one target's standing
/// signature now, or `None` if nothing of it arrives.
fn read(sim: &Sim, sweeper: &Sweeper<'_>, target: EntityId) -> Option<Reading> {
    let (def, kind) = (sweeper.def, sweeper.kind);
    let strength = senses::standing_signature(sim, target, sweeper.kind_id);
    if strength <= Real::ZERO {
        return None;
    }
    if !def.planes {
        let top = sim.entities.outermost(target)?;
        let theirs = sim.entities.get(top)?.presence().plane;
        if !sweeper.plane.shares(theirs) {
            return None;
        }
    }
    let source = signal::source_of(sim, target)?;
    let eye = sweeper.eye;
    if !senses::within_arc(
        sweeper.facing,
        def.arc,
        senses::bearing(sim, eye.loc, source.loc),
    ) {
        return None;
    }
    // What could arrive with nothing in the way: below the threshold,
    // the line need not be followed.
    let light = signal::source_light(sim, kind, source);
    if signal::bound_lit(sim, kind, strength, source, eye, light) < sweeper.threshold {
        return None;
    }
    let mut received = signal::strength_lit(sim, kind, strength, source, eye, || light);
    if sweeper.contained {
        received *= kind.contained;
    }
    if received <= Real::ZERO {
        return None;
    }
    Some(Reading {
        ratio: received / sweeper.threshold.max(MIN_THRESHOLD),
        at: source.loc,
    })
}

/// [`read`] as it was first written: what the faster one is tested
/// against.
#[cfg(test)]
fn read_reference(
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
    if signal::bound_at(sim, kind, strength, source, eye) < threshold {
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
/// does not sweep (15 §14): one in a warm tract keeps its sweeps
/// waiting, a character stepped warm has none until it is promoted. A
/// sense its body cannot work receives nothing.
pub(super) fn sweep(sim: &mut Sim, observer: EntityId, sense: SenseId, bonus: Real) {
    let defs = Arc::clone(&sim.perception_defs);
    let Some(def) = defs.sense(sense) else {
        return;
    };
    // A character stepped warm (09 §9.3) does not sweep, and its sweeps
    // are not kept waiting: they begin again when it is promoted.
    if crate::ai::warm::is_warm(sim, observer) {
        if let Some(slot) = sim
            .components
            .senses
            .get_mut(observer)
            .and_then(|senses| senses.slot_mut(sense))
        {
            slot.wake = None;
        }
        return;
    }
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
    // What the sense notices (D-923): every kind, for a party member.
    let kinds = match &def.notices {
        Some(kinds) if !party::is_member(sim, observer) => clash_world::OccupantKinds::of(kinds),
        _ => clash_world::OccupantKinds::ALL,
    };
    let mut targets = match threshold {
        Some(_) => nearest_in_range(sim, observer, eye, def.range.get(), kinds),
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
    let sweeper = match threshold {
        Some(threshold) => sweeper(sim, observer, def, kind_id, kind, eye, threshold),
        None => None,
    };
    let max = max_awareness(sim);
    for target in targets {
        let reading = match &sweeper {
            Some(sweeper) if in_range.binary_search(&target).is_ok() => read(sim, sweeper, target),
            _ => None,
        };
        update(sim, observer, sense, target, reading, dt, now, max);
    }
    if kind.uses_light {
        party::after_sweep(sim, observer, sense);
    }
}

/// What a sweep of a sense works out once: `None` where [`read`] would
/// find nothing for any target, the observer having no senses or not
/// being in the world.
fn sweeper<'a>(
    sim: &Sim,
    observer: EntityId,
    def: &'a super::defs::SenseDef,
    kind_id: super::defs::SignalKindId,
    kind: &'a super::defs::KindInfo,
    eye: Spot,
    threshold: Real,
) -> Option<Sweeper<'a>> {
    let record = sim.entities.get(observer)?;
    Some(Sweeper {
        def,
        kind_id,
        kind,
        eye,
        threshold,
        facing: sim.components.senses.get(observer)?.facing(),
        plane: record.presence().plane,
        contained: matches!(record.place(), Place::In(_)),
    })
}

/// The nearest `perception.max_candidates` of everyone within a sense's
/// range (and what they hold), by distance and then id, in order of id
/// (D-897, D-920): who stands in reach is looked at a block of the
/// occupant index at a time, the nearest blocks first, each once with
/// what is inside it, keeping the nearest so far; the search stops at
/// the first block whose nearest cell is farther than the farthest kept.
/// The same as gathering everyone in reach and keeping the nearest
/// ([`nearest_in_range_reference`], performance 2), without looking at
/// the furniture of the rest of the range.
fn nearest_in_range(
    sim: &Sim,
    observer: EntityId,
    eye: Spot,
    range: Real,
    kinds: clash_world::OccupantKinds,
) -> Vec<EntityId> {
    let max = sim
        .setting_int(MAX_CANDIDATES_SETTING, DEFAULT_MAX_CANDIDATES)
        .max(1) as usize;
    let distance = |id: EntityId| {
        sim.entities
            .location(id)
            .and_then(|loc| sim.cosmos.delta(eye.loc, loc))
            .map_or(i64::MAX, clash_world::distance_sq_cells)
    };
    // Each once: who has been met, in order. A search meets few, so a
    // sorted list costs less than a mark for every slot there is.
    let mut met: Vec<EntityId> = Vec::new();
    let mut mark = |id: EntityId| match met.binary_search(&id) {
        Ok(_) => false,
        Err(at) => {
            met.insert(at, id);
            true
        }
    };
    // The nearest so far, the farthest of them on top.
    let mut kept: std::collections::BinaryHeap<(i64, EntityId)> =
        std::collections::BinaryHeap::with_capacity(max + 1);
    let keep = |kept: &mut std::collections::BinaryHeap<(i64, EntityId)>, id: EntityId| {
        let key = (distance(id), id);
        if kept.len() < max {
            kept.push(key);
        } else if kept.peek().is_some_and(|farthest| key < *farthest) {
            kept.pop();
            kept.push(key);
        }
    };
    let mut held: Vec<EntityId> = Vec::new();
    // What is inside something is noticed by its own kind.
    let noticed = |id: EntityId| {
        kinds == clash_world::OccupantKinds::ALL
            || sim
                .entities
                .get(id)
                .is_some_and(|entity| kinds.has(entity.presence().kind))
    };
    let searched = sim.cosmos.occupant_ids_within_nearest_first(
        eye.loc,
        range,
        &OccupantFilter::ANY.of_kinds(kinds).except(observer),
        |least, ids| {
            // Nothing in this block, nor in any after it, is nearer than
            // the farthest kept.
            if kept.len() >= max && kept.peek().is_some_and(|farthest| least > farthest.0) {
                return false;
            }
            for &(cell, id) in ids {
                // Farther than the farthest kept: it can be kept only if
                // a cell of it nearer is in reach, met in its own turn.
                if kept.len() >= max && kept.peek().is_some_and(|farthest| cell > farthest.0) {
                    continue;
                }
                if id == observer || !mark(id) {
                    continue;
                }
                keep(&mut kept, id);
                // What it holds stands where it does.
                held.clear();
                held.extend(sim.entities.contents_deep(id));
                let mut at = 0;
                while at < held.len() {
                    let inside = held[at];
                    if inside != observer && mark(inside) {
                        if noticed(inside) {
                            keep(&mut kept, inside);
                        }
                        held.extend(sim.entities.contents_deep(inside));
                    }
                    at += 1;
                }
            }
            true
        },
    );
    if searched.is_err() {
        return Vec::new();
    }
    let mut ids: Vec<EntityId> = kept.into_iter().map(|(_, id)| id).collect();
    ids.sort_unstable();
    ids
}

/// [`nearest_in_range`] as performance 2 left it: everyone in reach
/// gathered in one unordered pass, each once, with what they hold, and
/// the nearest picked without sorting the rest. What the search nearest
/// first is tested against.
#[cfg(test)]
fn nearest_in_range_reference(
    sim: &Sim,
    observer: EntityId,
    eye: Spot,
    range: Real,
) -> Vec<EntityId> {
    let max = sim
        .setting_int(MAX_CANDIDATES_SETTING, DEFAULT_MAX_CANDIDATES)
        .max(1) as usize;
    let mut found: Vec<EntityId> = Vec::new();
    if sim
        .cosmos
        .occupant_ids_within_unordered(
            eye.loc,
            range,
            &OccupantFilter::ANY.except(observer),
            &mut found,
        )
        .is_err()
    {
        return Vec::new();
    }
    // Each once: an entity's slot is marked as it is first met.
    let mut marked: Vec<u64> = Vec::new();
    let mut mark = |id: EntityId| {
        let slot = id.index();
        let (word, bit) = (slot / 64, 1_u64 << (slot % 64));
        if marked.len() <= word {
            marked.resize(word + 1, 0);
        }
        let new = marked[word] & bit == 0;
        marked[word] |= bit;
        new
    };
    let mut ids: Vec<EntityId> = Vec::with_capacity(found.len());
    for id in found {
        if id != observer && mark(id) {
            ids.push(id);
        }
    }
    let mut at = 0;
    while at < ids.len() {
        for inside in sim.entities.contents_deep(ids[at]) {
            if inside != observer && mark(inside) {
                ids.push(inside);
            }
        }
        at += 1;
    }
    if ids.len() > max {
        let distance = |id: EntityId| {
            sim.entities
                .location(id)
                .and_then(|loc| sim.cosmos.delta(eye.loc, loc))
                .map_or(i64::MAX, clash_world::distance_sq_cells)
        };
        let mut keyed: Vec<(i64, EntityId)> = ids.iter().map(|id| (distance(*id), *id)).collect();
        keyed.select_nth_unstable(max);
        keyed.truncate(max);
        ids.clear();
        ids.extend(keyed.into_iter().map(|(_, id)| id));
    }
    ids.sort_unstable();
    ids
}

/// At most `perception.max_candidates` of the candidates of a sweep: the
/// nearest, then by id; in order of id. What `nearest_in_range` is
/// tested against.
#[cfg(test)]
fn nearest(sim: &Sim, eye: Spot, mut candidates: Vec<EntityId>) -> Vec<EntityId> {
    let max = sim
        .setting_int(MAX_CANDIDATES_SETTING, DEFAULT_MAX_CANDIDATES)
        .max(1) as usize;
    if candidates.len() <= max {
        return candidates;
    }
    let distance = |id: EntityId| {
        sim.entities
            .location(id)
            .and_then(|loc| sim.cosmos.delta(eye.loc, loc))
            .map_or(i64::MAX, clash_world::distance_sq_cells)
    };
    // Each distance worked out once; no two keys are equal, so the
    // nearest `max` are the same whichever way they are picked.
    let mut keyed: Vec<(i64, EntityId)> =
        candidates.iter().map(|id| (distance(*id), *id)).collect();
    keyed.select_nth_unstable(max);
    keyed.truncate(max);
    candidates.clear();
    candidates.extend(keyed.into_iter().map(|(_, id)| id));
    candidates.sort_unstable();
    candidates
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
    // A warm character does not sweep (15 §14).
    if crate::ai::warm::is_warm(sim, observer) {
        return;
    }
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
        update(
            sim,
            observer,
            sense,
            target,
            reading,
            dt,
            now,
            max_awareness(sim),
        );
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
        update(
            sim,
            observer,
            sense,
            target,
            None,
            dt,
            now,
            max_awareness(sim),
        );
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

#[cfg(test)]
mod tests {
    //! The faster sight against the first (D-732): a sweep's reading of
    //! each target, and the exposure of a body, on random crowds in a
    //! street of the test city with things of several heights put up in
    //! it, against the references kept beside them.

    use std::path::Path;

    use clash_core::{Real, StateHasher};
    use clash_data::{DefTypes, Loader};
    use clash_time::Timeline;
    use clash_world::dims::TRACT_XY;
    use clash_world::{CellPos, Cosmos, GridId, Layer, Location, OccupantKind, WorldDefs};

    use super::super::signal::{exposure, exposure_reference};
    use super::*;
    use crate::{Command, DebugCommand, EntitySpec, Presence};

    /// A tract well inside the city.
    const START: (i32, i32) = (20, 20);

    fn street(dx: i32, dy: i32) -> Location {
        Location::new(
            GridId::new(0),
            CellPos::new(START.0 * TRACT_XY + dx, START.1 * TRACT_XY + 1 + dy, 0),
        )
    }

    fn city() -> Sim {
        let mut types = DefTypes::new();
        clash_data::expr::register_defs(&mut types);
        clash_world::register_defs(&mut types);
        crate::register_defs(&mut types);
        // The world's and perception's content of `mods/core`.
        let data = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../mods/core/data");
        let mut files: Vec<(String, String)> = Vec::new();
        for dir in ["world", "perception"] {
            let mut paths: Vec<_> = std::fs::read_dir(data.join(dir))
                .unwrap()
                .filter_map(|entry| entry.ok().map(|entry| entry.path()))
                .filter(|path| path.extension().is_some_and(|ext| ext == "json"))
                .collect();
            paths.sort();
            for path in paths {
                let name = path.file_name().unwrap().to_string_lossy().into_owned();
                files.push((
                    format!("data/{dir}/{name}"),
                    std::fs::read_to_string(&path).unwrap(),
                ));
            }
        }
        let files: Vec<(&str, &str)> = files
            .iter()
            .map(|(path, text)| (path.as_str(), text.as_str()))
            .collect();
        let defs = Loader::new(&types)
            .expr_registry(&crate::engine_registry())
            .memory_mod("core", r#"{ "id": "core" }"#, &files)
            .load_mods(&["core"])
            .into_result()
            .unwrap_or_else(|diagnostics| panic!("{}", diagnostics.render()));
        let world = WorldDefs::from_defs(&defs).unwrap_or_else(|d| panic!("{}", d.render()));
        let mut timeline = Timeline::new();
        let mut cosmos = Cosmos::new(Arc::clone(&world));
        let def = world.realms().lookup("core:test_city").unwrap();
        cosmos.create_realm(&mut timeline, def, 11).unwrap();
        let mut sim = Sim::from_parts(defs, cosmos, timeline, 5);
        let anchor = person(&mut sim, Place::Stored);
        let far = street(1, 3 * TRACT_XY);
        sim.submit(Command::Debug(DebugCommand::StartSession {
            entity: anchor,
            at: far,
        }))
        .unwrap();
        sim
    }

    fn person(sim: &mut Sim, place: Place) -> EntityId {
        let id = sim.truth().entities().next_id().unwrap();
        let spec = EntitySpec::new(OccupantKind::Creature)
            .with_presence(Presence::new(OccupantKind::Creature));
        sim.submit(Command::Debug(DebugCommand::CreateEntity { spec, place }))
            .unwrap();
        id
    }

    /// A small generator of numbers, so that the cases are the same on
    /// every run.
    struct Draw(u64);

    impl Draw {
        fn next(&mut self, below: u64) -> u64 {
            let mut hasher = StateHasher::default();
            hasher.write_u64(self.0);
            self.0 += 1;
            (hasher.finish().0 as u64) % below
        }
    }

    #[test]
    fn readings_and_exposure_are_what_the_first_ones_were() {
        let mut sim = city();
        let mut draw = Draw(7);
        let sight = sim
            .perception_defs()
            .sense_named("core:human_sight")
            .unwrap();
        let hearing = sim
            .perception_defs()
            .sense_named("core:human_hearing")
            .unwrap();
        let blocks: Vec<clash_world::ElementState> = [
            "core:wall",
            "core:pillar",
            "core:window",
            "core:door",
            "core:ramp",
        ]
        .iter()
        .filter_map(|name| sim.truth().cosmos().defs().block_state(name, None).ok())
        .map(|state| state.raw())
        .collect();
        assert!(blocks.len() >= 3);
        let mut compared = 0;
        let mut seen = 0;
        for round in 0..6_u16 {
            // Things of several kinds put up about the street.
            for _ in 0..12 {
                let at = street(draw.next(48) as i32, draw.next(3) as i32 - 1);
                let state = blocks[draw.next(blocks.len() as u64) as usize];
                let _ = sim.submit(Command::Debug(DebugCommand::SetElement {
                    loc: at,
                    layer: Layer::Block,
                    state,
                }));
            }
            // A crowd, facing every way.
            let mut crowd = Vec::new();
            for _ in 0..10 {
                let at = street(draw.next(48) as i32, draw.next(3) as i32 - 1);
                let one = person(&mut sim, Place::at(at));
                for sense in [sight, hearing] {
                    super::super::attach_sense(&mut sim, one, sense).unwrap();
                }
                let facing = Real::from(draw.next(360) as u16);
                super::super::set_facing(&mut sim, one, facing).unwrap();
                crowd.push(one);
            }
            let defs = Arc::clone(&sim.perception_defs);
            for &observer in &crowd {
                let Some(eye) = signal::eye_of(&sim, observer) else {
                    continue;
                };
                for sense in [sight, hearing] {
                    let def = defs.sense(sense).unwrap();
                    let kind_id = defs.sense_kind(sense).unwrap();
                    let kind = defs.kind(kind_id).unwrap();
                    let threshold = Real::of(0.02) / Real::from(1 + round);
                    let sweeper =
                        sweeper(&sim, observer, def, kind_id, kind, eye, threshold).unwrap();
                    for &target in &crowd {
                        let fast = read(&sim, &sweeper, target);
                        let first = read_reference(&sim, observer, sense, eye, threshold, target);
                        assert_eq!(fast, first, "{observer} of {target}");
                        compared += 1;
                        seen += usize::from(fast.is_some());
                        let Some(source) = signal::source_of(&sim, target) else {
                            continue;
                        };
                        let eyes = [eye, Spot::point(eye.loc, draw.next(2900) as u16)];
                        for eye in eyes {
                            let tall = Spot {
                                height_mm: draw.next(2500) as u16,
                                ..source
                            };
                            for body in [source, tall, Spot::point(source.loc, 900)] {
                                assert_eq!(
                                    exposure(sim.truth().cosmos(), eye, body),
                                    exposure_reference(sim.truth().cosmos(), eye, body),
                                );
                            }
                        }
                    }
                }
            }
        }
        assert!(
            compared > 1000 && seen > 50,
            "{compared} compared, {seen} seen"
        );
    }

    #[test]
    fn candidates_and_the_nearest_are_what_the_first_ones_were() {
        // Performance 2: the candidates of a sweep found by id alone, and
        // the nearest of them picked without sorting them all.
        let mut sim = city();
        let mut draw = Draw(11);
        let mut crowd = Vec::new();
        for _ in 0..60 {
            let at = street(draw.next(64) as i32 - 8, draw.next(5) as i32 - 2);
            crowd.push(person(&mut sim, Place::at(at)));
        }
        // Some inside others, as things are carried.
        for _ in 0..8 {
            let one = person(&mut sim, Place::Stored);
            let holder = crowd[draw.next(crowd.len() as u64) as usize];
            let _ = sim.relocate(one, Place::In(holder), clash_world::Cause::DEBUG);
        }
        // Goods lying about and carried (D-923: what a sense notices).
        let item = |sim: &mut Sim, place: Place| {
            let id = sim.truth().entities().next_id().unwrap();
            let spec = EntitySpec::new(OccupantKind::Item)
                .with_presence(Presence::new(OccupantKind::Item));
            sim.submit(Command::Debug(DebugCommand::CreateEntity { spec, place }))
                .unwrap();
            id
        };
        for _ in 0..40 {
            let at = street(draw.next(64) as i32 - 8, draw.next(5) as i32 - 2);
            item(&mut sim, Place::at(at));
        }
        for _ in 0..8 {
            let holder = crowd[draw.next(crowd.len() as u64) as usize];
            item(&mut sim, Place::In(holder));
        }
        for (max, &observer) in [12_usize, 40].iter().cycle().zip(&crowd) {
            sim.set_setting(MAX_CANDIDATES_SETTING, *max as i64);
            let at = sim.entities.location(observer).unwrap();
            for range in [4.0, 15.0, 40.0] {
                let range = Real::of(range);
                let fast = signal::candidates(&sim, observer, at, range);
                let first = signal::candidates_reference(&sim, observer, at, range);
                assert_eq!(fast, first, "{observer} at {range}");
                let eye = signal::eye_of(&sim, observer).unwrap();
                let mut all = fast.clone();
                let near = nearest(&sim, eye, fast);
                // The reference: every distance, all sorted.
                let distance = |id: EntityId| {
                    sim.entities
                        .location(id)
                        .and_then(|loc| sim.cosmos.delta(eye.loc, loc))
                        .map_or(i64::MAX, clash_world::distance_sq_cells)
                };
                all.sort_by_key(|id| (distance(*id), *id));
                all.truncate(*max);
                all.sort_unstable();
                assert_eq!(near, all, "{observer} at {range}");
                // Gathered in one pass (performance 2), and looked for a
                // block at a time nearest first (performance 3): the same.
                assert_eq!(
                    nearest_in_range_reference(&sim, observer, eye, range),
                    near,
                    "{observer} at {range}"
                );
                assert_eq!(
                    nearest_in_range(&sim, observer, eye, range, clash_world::OccupantKinds::ALL),
                    near,
                    "{observer} at {range}"
                );
                // A sense that notices creatures alone: the nearest of
                // them, the goods passed over.
                let mut creatures: Vec<EntityId> = signal::candidates_reference(&sim, observer, at, range)
                    .into_iter()
                    .filter(|id| {
                        sim.entities
                            .get(*id)
                            .is_some_and(|entity| entity.presence().kind == OccupantKind::Creature)
                    })
                    .collect();
                creatures.sort_by_key(|id| (distance(*id), *id));
                creatures.truncate(*max);
                creatures.sort_unstable();
                assert_eq!(
                    nearest_in_range(
                        &sim,
                        observer,
                        eye,
                        range,
                        clash_world::OccupantKinds::of(&[OccupantKind::Creature])
                    ),
                    creatures,
                    "{observer} at {range}, creatures"
                );
            }
        }
    }

    #[test]
    fn a_view_kept_is_the_view_worked_out_afresh() {
        // Performance 3 (D-921): a member's view kept from its last sweep,
        // used while the eye, the cells and the light are as they were,
        // against the view worked out afresh, as the member stands, turns,
        // moves and the street is built on about it.
        let mut sim = city();
        let mut draw = Draw(13);
        let sight = sim
            .perception_defs()
            .sense_named("core:human_sight")
            .unwrap();
        let wall = sim
            .truth()
            .cosmos()
            .defs()
            .block_state("core:wall", None)
            .unwrap()
            .raw();
        let me = person(&mut sim, Place::at(street(20, 0)));
        super::super::attach_sense(&mut sim, me, sight).unwrap();
        let mut cache = super::super::party::ViewCache::default();
        let mut kept = 0;
        for round in 0..40_u32 {
            match draw.next(6) {
                // Turn.
                0 => {
                    let facing = Real::from(draw.next(360) as u16);
                    super::super::set_facing(&mut sim, me, facing).unwrap();
                }
                // A wall put up somewhere along the street.
                1 => {
                    let at = street(draw.next(48) as i32, draw.next(3) as i32 - 1);
                    let _ = sim.submit(Command::Debug(DebugCommand::SetElement {
                        loc: at,
                        layer: Layer::Block,
                        state: wall,
                    }));
                }
                // A step along the street.
                2 => {
                    let at = street(draw.next(48) as i32, 0);
                    let _ = sim.relocate(me, Place::at(at), clash_world::Cause::DEBUG);
                }
                // Time passes, nothing else.
                _ => {
                    let by = Dur::from_millis(250 * (draw.next(4) as i64 + 1));
                    let until = sim.now() + by;
                    sim.advance(crate::AdvanceLimit::Until(until));
                }
            }
            let before = sim.truth().cosmos().cell_epoch();
            let fresh = super::super::party::view_of_reference(&sim, me, sight);
            let cached = super::super::party::view_of(&sim, me, sight, &mut cache);
            assert_eq!(cached, fresh, "round {round}");
            // Asked again with nothing changed, the kept view.
            let again = super::super::party::view_of(&sim, me, sight, &mut cache);
            assert_eq!(again, fresh, "round {round}, again");
            assert_eq!(sim.truth().cosmos().cell_epoch(), before);
            kept += 1;
        }
        assert_eq!(kept, 40);
    }
}
