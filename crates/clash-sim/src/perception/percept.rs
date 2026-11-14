//! Percepts (15 §6): what an observer makes of a signal that reached it,
//! the `perceived` event, the two functions rules ask perception by, and
//! the seam window kinds use.

use std::sync::Arc;

use clash_core::{EntityId, Real};
use clash_world::Location;

use super::defs::{SenseId, SignalKindId, SignalModel};
use super::detect::{self, Reading};
use super::signal::{self, Signal, Spot};
use super::{WARM_FACTOR_SETTING, belief, events, party, senses};
use crate::{Place, Sim};

/// The multiple of its threshold a signal must reach to be received by a
/// warm observer when `perception.warm_factor` is not set (15 §14).
pub const DEFAULT_WARM_FACTOR: Real = Real::of(4.0);

/// The least a threshold is taken to be.
const MIN_THRESHOLD: Real = Real::of(1e-9);

/// The warm factor in force.
fn warm_factor(sim: &Sim) -> Real {
    super::setting_real(sim, WARM_FACTOR_SETTING)
        .filter(|factor| factor.is_finite() && *factor >= Real::ONE)
        .unwrap_or(DEFAULT_WARM_FACTOR)
}

/// What one sense of an observer receives of a signal now, as a multiple
/// of its threshold, and where the signal seemed to come from; `None` if
/// nothing of it arrives: the sense does not work, the signal is outside
/// its arc or on another plane, or nothing reaches the eye.
pub fn reading_of(sim: &Sim, signal: &Signal, observer: EntityId, sense: SenseId) -> Option<Real> {
    let defs = &sim.perception_defs;
    let def = defs.sense(sense)?;
    let kind = defs.kind(signal.kind)?;
    if defs.sense_kind(sense) != Some(signal.kind) {
        return None;
    }
    let eye = signal::eye_of(sim, observer)?;
    if !senses::sense_on(sim, observer, def) {
        return None;
    }
    let slot = sim.components.senses.get(observer)?.slot(sense)?;
    let threshold = senses::threshold_now(sim, observer, sense, def, slot)?;
    if !def.planes
        && let Some(about) = signal.about
    {
        let mine = sim.entities.get(observer)?.presence().plane;
        let top = sim.entities.outermost(about)?;
        let theirs = sim.entities.get(top)?.presence().plane;
        if !mine.shares(theirs) {
            return None;
        }
    }
    let facing = sim.components.senses.get(observer)?.facing();
    if !senses::within_arc(facing, def.arc, senses::bearing(sim, eye.loc, signal.at)) {
        return None;
    }
    let source = Spot::point(signal.at, signal.alt_mm);
    let mut received = signal::strength_at(sim, kind, signal.strength, source, eye);
    if matches!(sim.entities.get(observer)?.place(), Place::In(_)) {
        received *= kind.contained;
    }
    (received > Real::ZERO).then(|| received / threshold.max(MIN_THRESHOLD))
}

/// Whether a signal of a kind and a strength from a source could reach
/// one sense of an observer whose eye is at `eye` at or over its
/// threshold (over the warm factor times it, for a warm observer), with
/// nothing in its way: [`signal::bound_at`], the most of it that could
/// arrive, against the threshold [`reading_of`] would use; for a
/// message that `involved` the listener, the ordinary threshold however
/// warm it is (D-934). Never false for a signal [`receive`] would take
/// from there (D-894).
#[allow(
    clippy::too_many_arguments,
    reason = "one listener weighed against a signal, with what was worked out once for all"
)]
pub(super) fn could_receive(
    sim: &Sim,
    kind: &super::defs::KindInfo,
    strength: Real,
    source: Spot,
    observer: EntityId,
    sense: SenseId,
    eye: Spot,
    involved: bool,
) -> bool {
    let Some(slot) = sim
        .components
        .senses
        .get(observer)
        .and_then(|senses| senses.slot(sense))
    else {
        return false;
    };
    // One whose state stops the sense (D-930) is not waited for.
    if sim
        .perception_defs
        .sense(sense)
        .is_some_and(|def| senses::stopped(sim, observer, &def.stopped_by))
    {
        return false;
    }
    let Some(threshold) = sim
        .perception_defs
        .sense(sense)
        .and_then(|def| senses::threshold_now(sim, observer, sense, def, slot))
    else {
        return false;
    };
    let mut most = signal::bound_at(sim, kind, strength, source, eye);
    if kind.contained > Real::ONE
        && sim
            .entities
            .get(observer)
            .is_some_and(|record| matches!(record.place(), Place::In(_)))
    {
        most *= kind.contained;
    }
    let clarity = most / threshold.max(MIN_THRESHOLD);
    let warm = crate::ai::warm::is_warm(sim, observer) || detect::is_warm(sim, eye.loc);
    clarity
        >= if warm && !involved {
            warm_factor(sim)
        } else {
            Real::ONE
        }
}

/// Whether a signal carries a message that involves an observer
/// (D-934): heard at the ordinary threshold however warm it is.
pub(super) fn involving(sim: &Sim, signal: &Signal, observer: EntityId) -> bool {
    signal
        .message
        .as_ref()
        .is_some_and(|message| crate::social::speech::involves(sim, message, observer))
}

/// A signal reached an observer's sense (15 §6). If it is received at or
/// over the threshold (over the warm factor times it, for a warm
/// observer: one in a warm tract or stepped warm), the observer's awareness of what it is about builds, it
/// perceives it, remembers the event it carries if that is salient, and
/// a controlled character's frame notes it as heard. Returns whether it
/// was perceived.
pub(super) fn receive(sim: &mut Sim, signal: &Signal, observer: EntityId, sense: SenseId) -> bool {
    let Some(clarity) = reading_of(sim, signal, observer, sense) else {
        return false;
    };
    if clarity < Real::ONE {
        return false;
    }
    let stepped_warm = crate::ai::warm::is_warm(sim, observer);
    let warm = stepped_warm
        || signal::eye_of(sim, observer).is_some_and(|eye| detect::is_warm(sim, eye.loc));
    // A message that involves the listener is heard at its ordinary
    // threshold, and promotes one stepped warm (D-934).
    let involved = warm && involving(sim, signal, observer);
    if warm && !involved && clarity < warm_factor(sim) {
        return false;
    }
    if involved && stepped_warm {
        crate::ai::warm::promote(sim, observer, crate::ai::warm::Why::Involved);
    }
    let defs = Arc::clone(&sim.perception_defs);
    let identifies = defs.kind(signal.kind).is_some_and(|kind| kind.identifies);
    if let Some(about) = signal.about
        && about != observer
    {
        detect::emitted(
            sim,
            observer,
            sense,
            about,
            Reading {
                ratio: clarity,
                at: signal.at,
            },
        );
    }
    let made_out = |sim: &Sim, who: Option<EntityId>| {
        who.filter(|who| {
            *who != observer
                && (detect::perceives(sim, observer, *who)
                    || (identifies
                        && signal::source_of(sim, *who).is_some_and(|spot| spot.loc == signal.at)))
        })
    };
    let about = made_out(sim, signal.about);
    let about_actor = made_out(sim, signal.event.as_ref().and_then(|event| event.actor));
    events::perceived(
        sim,
        events::Percept {
            observer,
            about,
            percept: events::PERCEPT_SIGNAL,
            sense: defs.sense_name(sense).to_owned(),
            signal: defs.kind_name(signal.kind).to_owned(),
            event: signal.event.as_ref().map(|event| event.name.clone()),
            about_actor,
            at: Some(signal.at),
            clarity,
        },
    );
    if let Some(event) = &signal.event
        && event.salience > Real::ZERO
    {
        belief::remember_percept(
            sim,
            observer,
            &event.name,
            signal.at,
            about_actor,
            signal.kind,
            event.salience,
        );
    }
    party::heard(sim, observer, signal, clarity);
    if let Some(message) = &signal.message {
        // Speech (15 §11): what was said reaches whoever perceived it.
        crate::social::speech::heard(sim, observer, sense, message, about, signal.at, clarity);
    }
    true
}

/// Whether `observer` currently has `target` detected (15 §6): the
/// function `perceives(a, b)`.
pub fn perceives(sim: &Sim, observer: EntityId, target: EntityId) -> bool {
    detect::perceives(sim, observer, target)
}

/// Whether a signal of a kind at a place would reach `observer` now
/// through one of its senses, at or over the threshold (15 §6): the
/// function `could_perceive(a, loc, kind)`. The signal is taken to be of
/// the kind's standing strength, or 1 for a kind whose standing is 0,
/// from the height given in the cell. A `field` kind is read where the
/// observer is, so a place other than its own cell is never reached.
pub fn could_perceive(
    sim: &Sim,
    observer: EntityId,
    at: Location,
    alt_mm: u16,
    kind: SignalKindId,
) -> bool {
    let Some(info) = sim.perception_defs.kind(kind) else {
        return false;
    };
    let Some(senses) = sim.components.senses.get(observer) else {
        return false;
    };
    if info.model == SignalModel::Field {
        return false;
    }
    let strength = if info.standing > Real::ZERO {
        info.standing
    } else {
        Real::ONE
    };
    let Ok(at) = sim.cosmos.normalize(at) else {
        return false;
    };
    let probe = Signal::new(
        kind,
        at,
        strength,
        clash_world::Cause::new(clash_world::CauseKind::Process),
    )
    .at_height(alt_mm);
    senses.slots().iter().any(|slot| {
        reading_of(sim, &probe, observer, slot.sense()).is_some_and(|clarity| clarity >= Real::ONE)
    })
}

/// The seam for window kinds (03 §7.2): whether a candidate notices a
/// window opened by `source`. A window with no source is noticed; a
/// candidate notices its own; otherwise only a candidate that currently
/// perceives the source does. Whoever builds windows asks this before a
/// candidate's `noticed_if`.
pub fn notices_source(sim: &Sim, candidate: EntityId, source: Option<EntityId>) -> bool {
    match source {
        None => true,
        Some(source) if source == candidate => true,
        Some(source) => detect::perceives(sim, candidate, source),
    }
}
