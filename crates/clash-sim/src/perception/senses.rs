//! Senses, facing and signatures (15 §3.2, §4): what an entity perceives
//! with, which way it looks, and what it gives off.

use clash_core::HashDomain;
use clash_core::{Dur, EntityId, Real, StateHash, StateHasher, impl_state_hash, pos_hash};
use clash_data::Ref;
use clash_time::{Wake, WakeDomain, WakeId};
use clash_world::{Location, OccupantKind};
use serde::{Deserialize, Serialize};

use super::SWEEP_ARG_LIMIT;
use super::defs::{SenseId, SignalKindId};
use super::reason as why;
use crate::body::BodyFunctionDef;
use crate::event::Subject;
use crate::host::Bound;
use crate::reason::Reason;
use crate::sim::unknown_entity;
use crate::{Place, Sim};

/// Where an observer's first sweep falls within its interval: drawn from
/// the observer's id and the sense, never from the order senses were
/// given (01 §4).
const STAGGER: HashDomain = HashDomain::new("perception.stagger");

/// Degrees in a turn.
const TURN_DEG: Real = Real::of(360.0);

/// One sense an entity has, and what is kept about it between sweeps.
#[derive(Clone, PartialEq, Debug, Serialize, Deserialize)]
pub struct SenseSlot {
    /// Which sense.
    pub(super) sense: SenseId,
    /// Its threshold as last worked out.
    pub(super) threshold: Option<Real>,
    /// The owner's local time at its last sweep.
    pub(super) last_sweep: Option<Dur>,
    /// The wake-up of its next sweep.
    pub(super) wake: Option<WakeId>,
    /// For a sense of the `field` model: whether the last reading
    /// reached the threshold.
    pub(super) above: bool,
}

impl_state_hash!(SenseSlot {
    sense,
    threshold,
    last_sweep,
    wake,
    above
});

impl SenseSlot {
    /// Which sense.
    pub fn sense(&self) -> SenseId {
        self.sense
    }

    /// The threshold as last worked out, if it could be.
    pub fn threshold(&self) -> Option<Real> {
        self.threshold
    }

    /// The owner's local time at the last sweep.
    pub fn last_sweep(&self) -> Option<Dur> {
        self.last_sweep
    }
}

/// The senses an entity has and which way it faces (15 §4): the `Senses`
/// component.
///
/// Facing is in degrees clockwise from north (the direction of falling
/// `y`), from 0 up to 360.
#[derive(Clone, PartialEq, Debug, Default, Serialize, Deserialize)]
pub struct Senses {
    facing: Real,
    /// In order of sense id.
    slots: Vec<SenseSlot>,
}

impl StateHash for Senses {
    fn state_hash(&self, hasher: &mut StateHasher) {
        self.facing.state_hash(hasher);
        self.slots.state_hash(hasher);
    }
}

impl Senses {
    /// Which way the entity faces, in degrees clockwise from north.
    pub fn facing(&self) -> Real {
        self.facing
    }

    /// Its senses, in order of id.
    pub fn slots(&self) -> &[SenseSlot] {
        &self.slots
    }

    /// One of its senses.
    pub fn slot(&self, sense: SenseId) -> Option<&SenseSlot> {
        self.slots
            .binary_search_by_key(&sense, |slot| slot.sense)
            .ok()
            .map(|at| &self.slots[at])
    }

    pub(super) fn slot_mut(&mut self, sense: SenseId) -> Option<&mut SenseSlot> {
        self.slots
            .binary_search_by_key(&sense, |slot| slot.sense)
            .ok()
            .map(|at| &mut self.slots[at])
    }

    /// Whether it has a sense.
    pub fn has(&self, sense: SenseId) -> bool {
        self.slot(sense).is_some()
    }
}

/// What an entity gives off standing still, for each kind it has been
/// given (15 §3.2): the `Signature` component. A kind it has not been
/// given is the kind's `standing`, for a creature; anything else gives
/// off nothing of it.
#[derive(Clone, PartialEq, Debug, Default, Serialize, Deserialize)]
pub struct Signature {
    /// In order of kind.
    entries: Vec<(SignalKindId, Real)>,
}

impl StateHash for Signature {
    fn state_hash(&self, hasher: &mut StateHasher) {
        self.entries.state_hash(hasher);
    }
}

impl Signature {
    /// The strength given for a kind, if one was given.
    pub fn get(&self, kind: SignalKindId) -> Option<Real> {
        self.entries
            .binary_search_by_key(&kind, |(have, _)| *have)
            .ok()
            .map(|at| self.entries[at].1)
    }

    /// Every kind given, in order.
    pub fn entries(&self) -> &[(SignalKindId, Real)] {
        &self.entries
    }
}

/// A wake-up for a sense's sweep.
pub(super) fn sweep_wake(observer: EntityId, sense: SenseId) -> Wake {
    Wake::new(
        WakeDomain::Perception,
        observer.to_bits(),
        sense.index() as u64,
    )
}

/// Gives an entity a sense (15 §4). Its sweeps begin at a point of its
/// first interval drawn from its id, so that observers given senses at
/// the same moment do not all sweep together.
///
/// Refused for an entity that does not exist, a sense the definitions do
/// not have, and a sense the entity has already.
pub fn attach_sense(sim: &mut Sim, entity: EntityId, sense: SenseId) -> Result<(), Reason> {
    sim.known(entity)?;
    let defs = std::sync::Arc::clone(&sim.perception_defs);
    let def = defs.sense(sense).ok_or_else(|| {
        Reason::new(why::UNKNOWN_SENSE, "there is no such sense")
            .with("sense", sense.index() as i64)
    })?;
    let interval = def.interval.dur();
    let senses = sim
        .components
        .senses
        .get_or_insert_with(entity, Senses::default);
    let at = match senses.slots.binary_search_by_key(&sense, |slot| slot.sense) {
        Ok(_) => {
            return Err(Reason::new(
                why::HAS_SENSE,
                format!(
                    "{entity} already has the sense `{}`",
                    defs.sense_name(sense)
                ),
            )
            .with("entity", entity.to_string())
            .with("sense", defs.sense_name(sense)));
        }
        Err(at) => at,
    };
    senses.slots.insert(
        at,
        SenseSlot {
            sense,
            threshold: None,
            last_sweep: None,
            wake: None,
            above: false,
        },
    );
    let threshold = threshold_of(sim, entity, sense);
    if let Some(slot) = sim
        .components
        .senses
        .get_mut(entity)
        .and_then(|senses| senses.slot_mut(sense))
    {
        slot.threshold = threshold;
    }
    let micros = interval.as_micros().max(1) as u64;
    let (slot_bits, generation) = (entity.to_bits() as i32, (entity.to_bits() >> 32) as i32);
    let first = pos_hash(
        sim.rng.seed(),
        STAGGER,
        slot_bits,
        generation,
        0,
        sense.index() as u64,
    ) % micros;
    schedule_sweep(sim, entity, sense, Dur::from_micros(first as i64));
    Ok(())
}

/// Takes a sense away, with what the entity was aware of through it.
/// Refused for an entity that does not have it.
pub fn detach_sense(sim: &mut Sim, entity: EntityId, sense: SenseId) -> Result<(), Reason> {
    sim.known(entity)?;
    let wake = sim
        .components
        .senses
        .get_mut(entity)
        .and_then(|senses| {
            let at = senses
                .slots
                .binary_search_by_key(&sense, |slot| slot.sense)
                .ok()?;
            Some(senses.slots.remove(at).wake)
        })
        .ok_or_else(|| {
            Reason::new(why::NO_SENSE, format!("{entity} does not have that sense"))
                .with("entity", entity.to_string())
                .with("sense", sim.perception_defs.sense_name(sense))
        })?;
    if let Some(wake) = wake {
        sim.cancel_wake(wake);
    }
    if let Some(awareness) = sim.components.awareness.get_mut(entity) {
        awareness.forget_sense(sense);
    }
    Ok(())
}

/// Schedules a sense's next sweep `after` its owner's local time from
/// now, replacing one already waiting. Nothing is scheduled for an
/// entity that is not in the world: its sweeps begin again when it
/// returns.
pub(super) fn schedule_sweep(sim: &mut Sim, entity: EntityId, sense: SenseId, after: Dur) {
    if sense.index() as u64 >= SWEEP_ARG_LIMIT {
        return;
    }
    let old = sim
        .components
        .senses
        .get(entity)
        .and_then(|senses| senses.slot(sense))
        .and_then(|slot| slot.wake);
    if let Some(old) = old
        && sim.wake_owner(old) == Some(entity)
    {
        sim.cancel_wake(old);
    }
    let wake = sim
        .schedule_for(entity, after, sweep_wake(entity, sense))
        .ok();
    if let Some(slot) = sim
        .components
        .senses
        .get_mut(entity)
        .and_then(|senses| senses.slot_mut(sense))
    {
        slot.wake = wake;
    }
}

/// Makes sure every sense of an entity has its sweep waiting: after it
/// came back into the world, its wake-ups were cancelled while it was
/// away (invariant T9).
pub(super) fn ensure_sweeps(sim: &mut Sim, entity: EntityId) {
    let Some(senses) = sim.components.senses.get(entity) else {
        return;
    };
    let missing: Vec<(SenseId, Dur)> = senses
        .slots
        .iter()
        .filter(|slot| {
            slot.wake
                .is_none_or(|wake| sim.wake_owner(wake) != Some(entity))
        })
        .map(|slot| {
            let interval = sim
                .perception_defs
                .sense(slot.sense)
                .map_or(Dur::ZERO, |def| def.interval.dur());
            (slot.sense, interval)
        })
        .collect();
    for (sense, interval) in missing {
        schedule_sweep(sim, entity, sense, interval);
    }
}

/// Works a sense's threshold out for its owner: its expression, with the
/// owner as `self`. `None` where it comes to null or is not a number.
pub(super) fn threshold_of(sim: &Sim, entity: EntityId, sense: SenseId) -> Option<Real> {
    let def = sim.perception_defs.sense(sense)?;
    let value = match def.threshold.constant() {
        Some(value) => value.clone(),
        None => {
            let code = def.threshold.compiled()?;
            let me = sim.subject_value(Subject::Entity(entity));
            sim.eval(code, Bound::new().subject(me))
        }
    };
    value.as_number().filter(|threshold| threshold.is_finite())
}

/// Whether a sense works for its owner: it names no body function, or its
/// owner has no body, or the body's capacity of that function is above
/// zero.
pub(super) fn sense_works(
    sim: &Sim,
    entity: EntityId,
    function: Option<Ref<BodyFunctionDef>>,
) -> bool {
    let Some(function) = function else {
        return true;
    };
    match sim.components.body.get(entity) {
        Some(body) => body.function(function.id()) > Real::ZERO,
        None => true,
    }
}

/// Turns an entity to face a direction, in degrees clockwise from north.
/// Refused for an entity with no senses, and for a direction that is not
/// a finite number.
pub fn set_facing(sim: &mut Sim, entity: EntityId, degrees: Real) -> Result<(), Reason> {
    sim.known(entity)?;
    if !degrees.is_finite() {
        return Err(
            Reason::new(why::BAD_VALUE, "a facing is a finite number of degrees")
                .with("value", degrees),
        );
    }
    let senses = sim
        .components
        .senses
        .get_mut(entity)
        .ok_or_else(|| no_senses(entity))?;
    senses.facing = normalize_deg(degrees);
    Ok(())
}

/// Turns an entity to face a cell. Facing its own cell changes nothing.
pub fn face_toward(sim: &mut Sim, entity: EntityId, at: Location) -> Result<(), Reason> {
    let from = sim
        .entities
        .location(entity)
        .ok_or_else(|| unknown_entity(entity))?;
    match bearing(sim, from, at) {
        Some(degrees) => set_facing(sim, entity, degrees),
        None => {
            sim.components
                .senses
                .get(entity)
                .ok_or_else(|| no_senses(entity))?;
            Ok(())
        }
    }
}

/// The reason for an entity with no senses.
pub(super) fn no_senses(entity: EntityId) -> Reason {
    Reason::new(why::NO_SENSES, format!("{entity} has no senses"))
        .with("entity", entity.to_string())
}

/// An angle brought into 0 to 360 degrees.
pub(super) fn normalize_deg(degrees: Real) -> Real {
    let turns = (degrees / TURN_DEG).floor();
    let wrapped = degrees - turns * TURN_DEG;
    if wrapped >= TURN_DEG {
        Real::ZERO
    } else {
        wrapped
    }
}

/// The direction from one cell to another, in degrees clockwise from
/// north, the shorter way round on wrapped axes. `None` for a cell
/// straight above or below, or in another grid.
pub(super) fn bearing(sim: &Sim, from: Location, to: Location) -> Option<Real> {
    let delta = sim.cosmos.delta(from, to)?;
    if delta.x == 0 && delta.y == 0 {
        return None;
    }
    let radians = Real::from(delta.x).atan2(-Real::from(delta.y));
    Some(normalize_deg(
        radians * Real::of(180.0) / Real::of(core::f64::consts::PI),
    ))
}

/// Whether a direction lies within an arc centred on a facing.
pub(super) fn within_arc(facing: Real, arc: Real, direction: Option<Real>) -> bool {
    if arc >= TURN_DEG {
        return true;
    }
    let Some(direction) = direction else {
        return true;
    };
    let off = normalize_deg(direction - facing + Real::of(180.0)) - Real::of(180.0);
    off.abs() <= arc / Real::of(2.0)
}

/// Gives an entity a standing signature of a kind (15 §3.2): what it
/// gives off standing still. A strength of zero gives off nothing.
/// Refused for an entity that does not exist, a kind the definitions do
/// not have and a strength that is negative or not finite.
pub fn set_signature(
    sim: &mut Sim,
    entity: EntityId,
    kind: SignalKindId,
    strength: Real,
) -> Result<(), Reason> {
    sim.known(entity)?;
    if sim.perception_defs.kind(kind).is_none() {
        return Err(
            Reason::new(why::UNKNOWN_KIND, "there is no such signal kind")
                .with("kind", kind.index() as i64),
        );
    }
    if !strength.is_finite() || strength < Real::ZERO {
        return Err(Reason::new(
            why::BAD_VALUE,
            "a signature is a finite strength of zero or more",
        )
        .with("value", strength));
    }
    let signature = sim
        .components
        .signature
        .get_or_insert_with(entity, Signature::default);
    match signature
        .entries
        .binary_search_by_key(&kind, |(have, _)| *have)
    {
        Ok(at) => signature.entries[at].1 = strength,
        Err(at) => signature.entries.insert(at, (kind, strength)),
    }
    Ok(())
}

/// What an entity gives off of a kind standing still, where it is now:
/// the strength it was given, or for a creature the kind's `standing`,
/// times the kind's `contained` share if it is inside something. Zero for
/// an entity that is not in the world.
pub fn standing_signature(sim: &Sim, entity: EntityId, kind: SignalKindId) -> Real {
    let Some(info) = sim.perception_defs.kind(kind) else {
        return Real::ZERO;
    };
    let Some(record) = sim.entities.get(entity) else {
        return Real::ZERO;
    };
    let base = match sim
        .components
        .signature
        .get(entity)
        .and_then(|signature| signature.get(kind))
    {
        Some(strength) => strength,
        None if record.presence().kind == OccupantKind::Creature => info.standing,
        None => Real::ZERO,
    };
    match record.place() {
        Place::At { .. } => base,
        Place::In(_) => base * info.contained,
        Place::Stored => Real::ZERO,
    }
}
