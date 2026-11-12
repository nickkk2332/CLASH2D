//! Perception and belief (`docs/15-perception-and-social.md` sections 3
//! to 7, 13 and 14): what each character can sense, what it detects,
//! what it believes, and what the party saw.
//!
//! This package is rule R11 (characters act on belief, not truth) made
//! into data. Things give off **signals**; characters have **senses**;
//! a signal that reaches a sense at or over its threshold builds
//! **awareness** and raises `perceived`; what is perceived goes into the
//! character's **beliefs**, which is all its decisions may read.
//!
//! | Part | Is | Module |
//! |------|----|--------|
//! | [`SignalKindDef`], [`SenseDef`], [`EventSignalDef`] | The definition types `signal_kind`, `sense` and `event_signal` | [`defs`] |
//! | [`Signal`], [`emit_signal`] | A signal and how it travels: the `line`, `through` and `field` models, delay | [`signal`] |
//! | [`Senses`], [`Signature`] | The components: what an entity senses with and which way it faces; what it gives off | [`senses`] |
//! | [`Awareness`] | The component: what an observer is aware of, by target and sense | [`detect`] |
//! | [`Beliefs`], [`Track`], [`Memory`], [`Noted`] | The component: what a mind believes | [`belief`] |
//! | [`perceives`], [`could_perceive`], [`notices_source`] | What rules and windows ask perception | [`percept`] |
//! | [`MapMemory`], [`PartyFrame`], [`PartyPerception`] | What the player perceives, and the frames the view model is fed | [`party`] |
//! | [`believed_move_rules`] | Paths over what a mind believes | [`paths`] |
//!
//! # What runs when
//!
//! Nothing is evaluated every tick (15 §5.2, decision D-072).
//!
//! | When | What |
//! |------|------|
//! | A sense's `interval` elapses | That sense sweeps: every entity within its range, and every one already in its owner's awareness through it, is brought up to date. Sweeps of different observers are staggered by a hash of the observer. |
//! | An observer moves or turns | Each of its senses that has not swept for a quarter of its interval sweeps at the next wake-up. It turns to face the way it moved. |
//! | A signal is given off ([`emit_signal`], the verb `emit_signal`, an `event_signal`) | Every observer within reach with a sense of its kind receives it, at once or when the front of a delayed signal reaches it. |
//! | An explicit search or listen | [`search`]: every sense of the observer sweeps now, with a bonus. |
//! | An event is delivered | Its `event_signal`s are given off; the party's tag of it is recorded. |
//!
//! A target that moves is caught by the next sweep of each observer in
//! range: at most one evaluation of a pair for each interval. Only hot
//! observers sweep; a warm one receives only signals over
//! `perception.warm_factor` times its threshold (15 §14).
//!
//! # What the kernel holds and calls
//!
//! The simulation holds a [`PerceptionState`] (signals in flight, the
//! party's view, frames and tags) beside its components, saved and
//! hashed with the kernel, and the [`PerceptionDefs`] made from its
//! definitions. The kernel's pump calls [`observe_event`] for every
//! event it delivers; the wake-ups of the domain `Perception` go to
//! [`on_wake`].

pub mod belief;
pub mod defs;
pub mod detect;
pub mod events;
mod lint;
pub mod party;
pub mod paths;
pub mod percept;
pub(crate) mod props;
pub mod senses;
pub mod signal;

use std::sync::Arc;

use clash_core::{EntityId, Real, StateHash, StateHasher};
use clash_data::DefTypes;
use clash_time::{Wake, WakeId};
use serde::{Deserialize, Serialize};

use crate::entity::Control;
use crate::event::{EventBody, SimEvent, Subject};
use crate::host::Bound;
use crate::reason::ReasonInfo;
use crate::{Place, Sim};

pub use belief::{
    Beliefs, Capability, Knowing, Memory, Noted, PersonaId, Track, add_track, believed_loc,
    clear_track, note_element, observe_capability, remember, track_of,
};
pub use defs::{
    EventSignalDef, EventSignalId, KindInfo, PerceptionDefs, SenseDef, SenseId, SignalKindDef,
    SignalKindId, SignalModel,
};
pub use detect::{AwareEntry, Awareness, awareness_of, search, set_awareness};
pub use party::{
    MapMemory, PartyFrame, PartyPerception, Remembered, SeenCell, SeenEntity, drain_frames,
    set_omniscient, snapshot,
};
pub use paths::{BelievedMoveRules, believed_move_rules, believed_move_rules_with};
pub use percept::{could_perceive, notices_source, perceives};
pub use senses::{
    Senses, Signature, attach_sense, detach_sense, face_toward, set_facing, set_signature,
    standing_signature,
};
pub use signal::{Emitted, Signal, SignalEvent, emit_signal};

/// How much faster awareness builds than the plain excess over the
/// threshold says (15 §5.1's ε): a signal exactly at its threshold still
/// builds, slowly.
pub const AWARENESS_EPSILON: Real = Real::of(0.1);

/// A sweep's wake-up carries its sense's index, which must stay below
/// this.
pub const SWEEP_ARG_LIMIT: u64 = 1 << 16;

/// The `arg` of the wake-up of a delayed signal's arrival.
pub const FLIGHT_ARG: u64 = u64::MAX;

/// The multiple of its threshold a signal must reach to be received by a
/// warm observer (15 §14).
pub const WARM_FACTOR_SETTING: &str = "perception.warm_factor";
/// The most pairs an observer keeps awareness of.
pub const MAX_AWARENESS_SETTING: &str = "perception.max_awareness";
/// The most entities one sweep of one sense considers, the nearest first.
pub const MAX_CANDIDATES_SETTING: &str = "perception.max_candidates";
/// Whether every event counts as seen by the party (15 §13).
pub const OMNISCIENT_SETTING: &str = "perception.omniscient";
/// The most tracks a mind keeps (15 §15).
pub const MAX_TRACKS_SETTING: &str = "mind.max_tracks";
/// The most memories a mind keeps (15 §15).
pub const MAX_MEMORIES_SETTING: &str = "mind.max_memories";
/// The most noted things a mind keeps (15 §15).
pub const MAX_NOTED_SETTING: &str = "mind.max_noted";
/// The most observed capabilities a track keeps (15 §15).
pub const TRACK_CAPABILITIES_SETTING: &str = "track_capabilities";
/// How long a track's confidence takes to halve, in seconds.
pub const CONFIDENCE_HALFLIFE_SETTING: &str = "mind.confidence_halflife";
/// How long a memory's salience takes to halve, in seconds.
pub const SALIENCE_HALFLIFE_SETTING: &str = "mind.salience_halflife";

/// The stable codes of the reasons perception refuses for.
pub mod reason {
    /// The sense is not one of the loaded definitions'. Values: `sense`.
    pub const UNKNOWN_SENSE: &str = "perception.unknown_sense";
    /// The signal kind is not one of the loaded definitions'. Values:
    /// `kind`.
    pub const UNKNOWN_KIND: &str = "perception.unknown_kind";
    /// The entity has that sense already. Values: `entity`, `sense`.
    pub const HAS_SENSE: &str = "perception.has_sense";
    /// The entity does not have that sense. Values: `entity`, `sense`.
    pub const NO_SENSE: &str = "perception.no_sense";
    /// The entity has no senses at all. Values: `entity`.
    pub const NO_SENSES: &str = "perception.no_senses";
    /// A number or an argument out of its range. Values: `value` or
    /// `argument`.
    pub const BAD_VALUE: &str = "perception.bad_value";
    /// A signal of the `field` model was to be given off. Values: `kind`.
    pub const FIELD_NOT_SENT: &str = "perception.field_not_sent";
    /// A mind was to keep a track of itself. Values: `entity`.
    pub const SELF_TRACK: &str = "perception.self_track";
}

/// Every reason perception refuses for, for the generated reference.
pub const REASONS: &[ReasonInfo] = &[
    ReasonInfo {
        code: reason::UNKNOWN_SENSE,
        doc: "The sense named is not one of the loaded definitions'. Values: `sense`.",
    },
    ReasonInfo {
        code: reason::UNKNOWN_KIND,
        doc: "The signal kind named is not one of the loaded definitions'. Values: `kind`.",
    },
    ReasonInfo {
        code: reason::HAS_SENSE,
        doc: "The entity has that sense already. Values: `entity`, `sense`.",
    },
    ReasonInfo {
        code: reason::NO_SENSE,
        doc: "The entity does not have that sense. Values: `entity`, `sense`.",
    },
    ReasonInfo {
        code: reason::NO_SENSES,
        doc: "The entity has no senses: it cannot face or search. Values: `entity`.",
    },
    ReasonInfo {
        code: reason::BAD_VALUE,
        doc: "A strength, level, confidence, salience, facing or bonus out of its range, or an argument that names nothing. Values: `value` or `argument`.",
    },
    ReasonInfo {
        code: reason::FIELD_NOT_SENT,
        doc: "A signal of the `field` model is read from a field where the observer stands and cannot be given off. Values: `kind`.",
    },
    ReasonInfo {
        code: reason::SELF_TRACK,
        doc: "A mind keeps no track of itself. Values: `entity`.",
    },
];

/// What perception keeps that belongs to no one entity: signals in
/// flight and the party's view, frames and tags. Saved; hashed but for
/// the frames and tags, which are records of output.
#[derive(Clone, PartialEq, Debug, Default, Serialize, Deserialize)]
pub struct PerceptionState {
    /// Delayed signals on their way, in order of id.
    flights: Vec<signal::Flight>,
    next_flight: u64,
    party: party::PartyState,
}

impl StateHash for PerceptionState {
    fn state_hash(&self, hasher: &mut StateHasher) {
        self.flights.state_hash(hasher);
        hasher.write_u64(self.next_flight);
        self.party.state_hash(hasher);
    }
}

impl PerceptionState {
    /// How many delayed signals are on their way.
    pub fn in_flight(&self) -> usize {
        self.flights.len()
    }
}

/// Registers this package's definition types and its checks. Called
/// from the crate's `register_defs`.
pub fn register_defs(types: &mut DefTypes) {
    types
        .register::<SignalKindDef>()
        .register::<SenseDef>()
        .register::<EventSignalDef>()
        .add_cross_check(defs::check_one_noise)
        .add_cross_check(lint::check_belief_reads);
}

/// Handles a wake-up of the `Perception` domain: a delayed signal's
/// arrival, or a sense's sweep. A sweep whose sense has since been
/// rescheduled or taken away is passed over.
pub fn on_wake(sim: &mut Sim, id: WakeId, wake: Wake) {
    if signal::is_flight(&wake) {
        signal::arrive(sim, wake.id);
        return;
    }
    let observer = EntityId::from_bits(wake.id);
    let Some(sense) = usize::try_from(wake.arg).ok().and_then(SenseId::from_index) else {
        return;
    };
    let current = sim
        .components
        .senses
        .get(observer)
        .and_then(|senses| senses.slot(sense))
        .is_some_and(|slot| slot.wake == Some(id));
    if current {
        if let Some(slot) = sim
            .components
            .senses
            .get_mut(observer)
            .and_then(|senses| senses.slot_mut(sense))
        {
            slot.wake = None;
        }
        detect::sweep(sim, observer, sense, Real::ONE);
    }
}

/// What the pump calls for every event it delivers (03 §4.3): the event
/// gives off its `event_signal`s, an observer that moved turns and looks
/// again, and the party's tag of the event is recorded.
pub fn observe_event(sim: &mut Sim, event: &SimEvent) {
    let name = event.name();
    if events::NAMES.contains(&name) {
        party::tag(sim, event, &[]);
        return;
    }
    match &event.body {
        EventBody::EntityRelocated { entity, from, to } => relocated(sim, *entity, *from, *to),
        EventBody::EntityRemoved { entity } => party::left(sim, *entity),
        EventBody::ControlChanged { entity, new, .. } if *new != Control::Direct => {
            party::left(sim, *entity);
        }
        _ => {}
    }
    let signals = event_signals(sim, event);
    for signal in &signals {
        let _ = signal::emit_signal(sim, signal.clone());
    }
    party::tag(sim, event, &signals);
}

/// An entity's place changed: an observer that came back into the world
/// has its sweeps scheduled again, one that moved on its grid turns to
/// face the way it went and looks again.
fn relocated(sim: &mut Sim, entity: EntityId, from: Place, to: Place) {
    if !sim.components.senses.contains(entity) {
        return;
    }
    if let Place::At { .. } = to {
        senses::ensure_sweeps(sim, entity);
    }
    if let (Place::At { loc: a, .. }, Place::At { loc: b, .. }) = (from, to)
        && let Some(degrees) = senses::bearing(sim, a, b)
    {
        let _ = senses::set_facing(sim, entity, degrees);
    }
    if to.loc().is_some() || matches!(to, Place::In(_)) {
        detect::observer_moved(sim, entity);
    }
}

/// The signals an event gives off by the `event_signal`s of its type
/// whose condition holds, each from where the event happened.
fn event_signals(sim: &Sim, event: &SimEvent) -> Vec<signal::Signal> {
    let defs = Arc::clone(&sim.perception_defs);
    let ids = defs.signals_of(event.name());
    if ids.is_empty() {
        return Vec::new();
    }
    let Some(bound) = sim.exprs().bind_event(event) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for id in ids {
        let Some(def) = defs.event_signal(*id) else {
            continue;
        };
        if !holds(sim, &def.condition, &bound) {
            continue;
        }
        let strength = match def.strength.constant() {
            Some(value) => value.as_number(),
            None => def
                .strength
                .compiled()
                .and_then(|code| sim.eval(code, bound.clone()).as_number()),
        };
        let Some(strength) =
            strength.filter(|strength| strength.is_finite() && *strength > Real::ZERO)
        else {
            continue;
        };
        let about = def
            .about
            .as_ref()
            .and_then(|name| event.subject(&name.value))
            .and_then(|subject| subject.entity());
        let Some((at, alt)) = where_of(sim, event, about) else {
            continue;
        };
        let mut signal = signal::Signal::new(def.signal.id(), at, strength, event.cause)
            .at_height(alt)
            .carrying(signal::SignalEvent {
                name: event.name().to_owned(),
                actor: event.cause.actor,
                salience: def.salience,
            });
        if let Some(about) = about {
            signal = signal.about(about);
        }
        out.push(signal);
    }
    out
}

/// Whether a condition holds for an event: a constant, or its compiled
/// form evaluated with what the event binds. Null counts as false.
fn holds(sim: &Sim, condition: &clash_data::Cond, bound: &Bound) -> bool {
    if let Some(constant) = condition.constant_value() {
        return constant;
    }
    match condition.compiled() {
        Some(code) => matches!(sim.eval(code, bound.clone()), clash_expr::Value::Bool(true)),
        None => true,
    }
}

/// Where an event happened, for the signal it gives off: its `here`, the
/// cell of what the signal is about, or the cell of the event's first
/// subject that is somewhere.
fn where_of(
    sim: &Sim,
    event: &SimEvent,
    about: Option<EntityId>,
) -> Option<(clash_world::Location, u16)> {
    if let EventBody::World(world) = &event.body {
        return Some((world.location(), 0));
    }
    if let Some(Subject::Loc(loc)) = event.subject("here") {
        return Some((loc, 0));
    }
    if let Some(spot) = about.and_then(|about| signal::source_of(sim, about)) {
        return Some((spot.loc, spot.alt_mm));
    }
    event
        .subjects()
        .into_iter()
        .find_map(|(_, subject)| match subject {
            Subject::Entity(id) => signal::source_of(sim, id).map(|spot| (spot.loc, spot.alt_mm)),
            other => other.loc().map(|loc| (loc, 0)),
        })
}

impl Sim {
    /// How the party perceived an event this simulation delivered
    /// (15 §13): what playback asks so that it shows only what was
    /// perceived. See [`party::perceived`].
    pub fn party_perceived(&self, event: &SimEvent) -> PartyPerception {
        party::perceived(self, event)
    }

    /// What perception keeps that belongs to no one entity.
    pub fn perception(&self) -> &PerceptionState {
        &self.perception
    }

    /// The tables of perception made from the definitions.
    pub fn perception_defs(&self) -> &Arc<PerceptionDefs> {
        &self.perception_defs
    }
}

/// A setting that is a number, whole or not.
pub(crate) fn setting_real(sim: &Sim, key: &str) -> Option<Real> {
    match sim.settings().get(key)? {
        clash_core::Value::Real(value) => Some(*value),
        clash_core::Value::Int(value) => Some(Real::from_i64(*value)),
        _ => None,
    }
}
