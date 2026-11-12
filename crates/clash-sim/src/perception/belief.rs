//! Beliefs (15 §7): what a mind holds about others and the world, all
//! of it from what it perceived, was told or worked out.
//!
//! | Kind | Holds |
//! |------|-------|
//! | [`Track`] | Where and when a persona was last perceived, how, with what confidence, and what it was seen to do |
//! | [`Memory`] | Something that happened: the event type, where, when, who, how it is known, confidence and salience |
//! | [`Noted`] | Something noticed about the world itself: an element that is there |
//!
//! Every list is bounded by a setting, and what goes first when one is
//! full is fixed: the weakest and oldest.

use clash_core::{Dur, EntityId, Real, StateHash, StateHasher, impl_state_hash};
use clash_world::{ElementState, Layer, Location};
use serde::{Deserialize, Serialize};

use super::defs::SignalKindId;
use super::detect::local_now;
use super::reason as why;
use super::{
    CONFIDENCE_HALFLIFE_SETTING, MAX_MEMORIES_SETTING, MAX_NOTED_SETTING, MAX_TRACKS_SETTING,
    SALIENCE_HALFLIFE_SETTING, TRACK_CAPABILITIES_SETTING, events,
};
use crate::Sim;
use crate::reason::Reason;

/// The most tracks a mind keeps when `mind.max_tracks` is not set.
pub const DEFAULT_MAX_TRACKS: i64 = 64;
/// The most memories a mind keeps when `mind.max_memories` is not set.
pub const DEFAULT_MAX_MEMORIES: i64 = 32;
/// The most noted things a mind keeps when `mind.max_noted` is not set.
pub const DEFAULT_MAX_NOTED: i64 = 32;
/// The most observed capabilities a track keeps when
/// `track_capabilities` is not set (15 §7.1).
pub const DEFAULT_TRACK_CAPABILITIES: i64 = 8;
/// How long a track's confidence takes to halve when
/// `mind.confidence_halflife` is not set, in seconds of the mind's time.
pub const DEFAULT_CONFIDENCE_HALFLIFE_S: f64 = 60.0;
/// How long a memory's salience takes to halve when
/// `mind.salience_halflife` is not set, in seconds of the mind's time.
pub const DEFAULT_SALIENCE_HALFLIFE_S: f64 = 600.0;

/// Who an observer takes something to be (15 §8.1).
///
/// Personas are the next package's (S6). Until then every entity presents
/// one persona, its own, and this is that persona: the entity's id under
/// another name. Beliefs are kept by persona already, so that a disguise
/// changes what is believed without changing how it is kept (15
/// invariant P2).
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug, Serialize, Deserialize)]
pub struct PersonaId(EntityId);

impl PersonaId {
    /// The persona an entity presents.
    pub const fn presented_by(entity: EntityId) -> PersonaId {
        PersonaId(entity)
    }

    /// The entity whose true persona this is.
    pub const fn entity(self) -> EntityId {
        self.0
    }
}

impl StateHash for PersonaId {
    fn state_hash(&self, hasher: &mut StateHasher) {
        self.0.state_hash(hasher);
    }
}

/// How something is known (15 §7.1).
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Knowing {
    /// Perceived through a sense of this kind of signal.
    Perceived(SignalKindId),
    /// Told by someone.
    Told,
    /// Worked out.
    Inferred,
    /// Carried back from a time that was undone (13 §9.3).
    Carried,
}

impl StateHash for Knowing {
    fn state_hash(&self, hasher: &mut StateHasher) {
        match self {
            Knowing::Perceived(kind) => {
                hasher.write_u8(0);
                kind.state_hash(hasher);
            }
            Knowing::Told => hasher.write_u8(1),
            Knowing::Inferred => hasher.write_u8(2),
            Knowing::Carried => hasher.write_u8(3),
        }
    }
}

/// Something a persona was seen, or said, to do (15 §7.1).
#[derive(Clone, PartialEq, Debug, Serialize, Deserialize)]
pub struct Capability {
    /// The action's or ability's qualified id.
    pub what: String,
    /// The strongest result seen.
    pub best: Real,
    /// The mind's local time it was last seen or told of.
    pub at: Dur,
    /// Whether it was told of rather than seen.
    pub told: bool,
}

impl_state_hash!(Capability {
    what,
    best,
    at,
    told
});

/// What a mind knows about one persona (15 §7.1).
#[derive(Clone, PartialEq, Debug, Serialize, Deserialize)]
pub struct Track {
    persona: PersonaId,
    loc: Option<Location>,
    /// The mind's local time it was last perceived or heard of.
    at: Dur,
    how: Knowing,
    /// The confidence at `at`; it halves every halflife since.
    confidence: Real,
    capabilities: Vec<Capability>,
}

impl_state_hash!(Track {
    persona,
    loc,
    at,
    how,
    confidence,
    capabilities
});

impl Track {
    /// Whom it is about.
    pub fn persona(&self) -> PersonaId {
        self.persona
    }

    /// The last known place.
    pub fn loc(&self) -> Option<Location> {
        self.loc
    }

    /// The mind's local time when it was last perceived or heard of.
    pub fn at(&self) -> Dur {
        self.at
    }

    /// How it was last known.
    pub fn how(&self) -> Knowing {
        self.how
    }

    /// The confidence when it was last perceived, 0 to 1.
    pub fn confidence_then(&self) -> Real {
        self.confidence
    }

    /// What it was seen or said to do, strongest first.
    pub fn capabilities(&self) -> &[Capability] {
        &self.capabilities
    }
}

/// Something that happened, as a mind remembers it (15 §7.1).
#[derive(Clone, PartialEq, Debug, Serialize, Deserialize)]
pub struct Memory {
    /// The event type.
    pub event: String,
    /// Where.
    pub loc: Option<Location>,
    /// The mind's local time when it happened.
    pub at: Dur,
    /// Who played which part, as the mind made them out: role and
    /// persona, or none where it did not.
    pub roles: Vec<(String, Option<PersonaId>)>,
    /// How it is known.
    pub how: Knowing,
    /// How sure the mind is, 0 to 1.
    pub confidence: Real,
    /// How much it matters at `at`, 0 to 1; it halves every halflife
    /// since.
    pub salience: Real,
}

impl_state_hash!(Memory {
    event,
    loc,
    at,
    roles,
    how,
    confidence,
    salience
});

/// Something a mind noticed about the world itself (15 §7.1): an element
/// and the state it was in.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub struct Noted {
    /// The cell.
    pub loc: Location,
    /// Which of its elements.
    pub layer: Layer,
    /// The state it was in.
    pub state: ElementState,
    /// The mind's local time it was noticed.
    pub at: Dur,
}

impl_state_hash!(Noted {
    loc,
    layer,
    state,
    at
});

/// Everything a mind believes (15 §7): the `Beliefs` component.
#[derive(Clone, PartialEq, Debug, Default, Serialize, Deserialize)]
pub struct Beliefs {
    /// In order of persona.
    tracks: Vec<Track>,
    /// In the order they were made.
    memories: Vec<Memory>,
    /// In the order they were noticed.
    noted: Vec<Noted>,
}

impl StateHash for Beliefs {
    fn state_hash(&self, hasher: &mut StateHasher) {
        self.tracks.state_hash(hasher);
        self.memories.state_hash(hasher);
        self.noted.state_hash(hasher);
    }
}

impl Beliefs {
    /// Every track, in order of persona.
    pub fn tracks(&self) -> &[Track] {
        &self.tracks
    }

    /// The track of a persona.
    pub fn track(&self, persona: PersonaId) -> Option<&Track> {
        self.tracks
            .binary_search_by_key(&persona, |track| track.persona)
            .ok()
            .map(|at| &self.tracks[at])
    }

    /// Every memory, oldest first.
    pub fn memories(&self) -> &[Memory] {
        &self.memories
    }

    /// Every noted thing, oldest first.
    pub fn noted(&self) -> &[Noted] {
        &self.noted
    }
}

/// A setting that is a number of seconds, or its default.
fn seconds_setting(sim: &Sim, key: &str, default: f64) -> Real {
    super::setting_real(sim, key)
        .filter(|value| value.is_finite() && *value > Real::ZERO)
        .unwrap_or(Real::of(default))
}

/// A setting that is a count, at least 1.
fn count_setting(sim: &Sim, key: &str, default: i64) -> usize {
    sim.setting_int(key, default).max(1) as usize
}

/// A value that halves every `halflife` seconds, `age` seconds on.
fn faded(value: Real, age: Real, halflife: Real) -> Real {
    if age <= Real::ZERO {
        return value;
    }
    value * Real::of(0.5).powf(age / halflife)
}

/// The persona a mind perceives an entity as. Until personas, the
/// entity's own (see [`PersonaId`]).
pub fn persona_of(_sim: &Sim, _mind: EntityId, entity: EntityId) -> PersonaId {
    PersonaId::presented_by(entity)
}

/// A mind's track of what it perceives an entity as, if it has one.
pub fn track_of(sim: &Sim, mind: EntityId, entity: EntityId) -> Option<&Track> {
    let persona = persona_of(sim, mind, entity);
    sim.components.beliefs.get(mind)?.track(persona)
}

/// Where a mind last perceived an entity, if it has a track of it.
pub fn believed_loc(sim: &Sim, mind: EntityId, entity: EntityId) -> Option<Location> {
    track_of(sim, mind, entity)?.loc
}

/// How sure a mind is of its track of an entity now: the confidence it
/// had, halved for every `mind.confidence_halflife` since.
pub fn confidence(sim: &Sim, mind: EntityId, entity: EntityId) -> Option<Real> {
    let track = track_of(sim, mind, entity)?;
    let age = (local_now(sim, mind) - track.at).as_secs_real();
    let halflife = seconds_setting(
        sim,
        CONFIDENCE_HALFLIFE_SETTING,
        DEFAULT_CONFIDENCE_HALFLIFE_S,
    );
    Some(faded(track.confidence, age, halflife))
}

/// Seconds of the mind's time since it last perceived or heard of an
/// entity.
pub fn last_seen(sim: &Sim, mind: EntityId, entity: EntityId) -> Option<Real> {
    let track = track_of(sim, mind, entity)?;
    Some(
        (local_now(sim, mind) - track.at)
            .as_secs_real()
            .max(Real::ZERO),
    )
}

/// Brings a mind's track of an entity up to date or makes one, keeping
/// the list within `mind.max_tracks`: the least confident, then the
/// oldest, go first.
fn put_track(
    sim: &mut Sim,
    mind: EntityId,
    entity: EntityId,
    loc: Option<Location>,
    how: Knowing,
    confidence: Real,
) {
    let persona = persona_of(sim, mind, entity);
    let now = local_now(sim, mind);
    let max = count_setting(sim, MAX_TRACKS_SETTING, DEFAULT_MAX_TRACKS);
    let beliefs = sim
        .components
        .beliefs
        .get_or_insert_with(mind, Beliefs::default);
    match beliefs
        .tracks
        .binary_search_by_key(&persona, |track| track.persona)
    {
        Ok(at) => {
            let track = &mut beliefs.tracks[at];
            if loc.is_some() {
                track.loc = loc;
            }
            track.at = now;
            track.how = how;
            track.confidence = confidence;
        }
        Err(at) => beliefs.tracks.insert(
            at,
            Track {
                persona,
                loc,
                at: now,
                how,
                confidence,
                capabilities: Vec::new(),
            },
        ),
    }
    while beliefs.tracks.len() > max {
        let Some(weakest) = beliefs
            .tracks
            .iter()
            .enumerate()
            .filter(|(_, track)| track.persona != persona)
            .min_by(|(_, a), (_, b)| {
                (a.confidence, a.at, a.persona).cmp(&(b.confidence, b.at, b.persona))
            })
            .map(|(at, _)| at)
        else {
            break;
        };
        beliefs.tracks.remove(weakest);
    }
}

/// A mind perceived an entity at a place through a kind of signal: its
/// track is brought up to date there, with full confidence.
pub(super) fn perceived_at(
    sim: &mut Sim,
    mind: EntityId,
    entity: EntityId,
    at: Location,
    kind: SignalKindId,
) {
    put_track(
        sim,
        mind,
        entity,
        Some(at),
        Knowing::Perceived(kind),
        Real::ONE,
    );
}

/// Gives a mind a track of an entity, or brings the one it has up to
/// date: what the `add_track` effect does (15 §15). `at` is where it is
/// believed to be, if anywhere is; `how` is how it is known. Refused for
/// a mind or an entity that does not exist, for itself, and for a
/// confidence outside 0 to 1.
pub fn add_track(
    sim: &mut Sim,
    mind: EntityId,
    entity: EntityId,
    at: Option<Location>,
    how: Knowing,
    confidence: Real,
) -> Result<(), Reason> {
    sim.known(mind)?;
    sim.known(entity)?;
    if mind == entity {
        return Err(
            Reason::new(why::SELF_TRACK, "a mind keeps no track of itself")
                .with("entity", mind.to_string()),
        );
    }
    if !confidence.is_finite() || confidence < Real::ZERO || confidence > Real::ONE {
        return Err(
            Reason::new(why::BAD_VALUE, "a confidence is from 0 to 1").with("value", confidence)
        );
    }
    put_track(sim, mind, entity, at, how, confidence);
    Ok(())
}

/// Takes a mind's track of an entity away: the `clear_track` effect.
/// Returns whether there was one.
pub fn clear_track(sim: &mut Sim, mind: EntityId, entity: EntityId) -> Result<bool, Reason> {
    sim.known(mind)?;
    let persona = persona_of(sim, mind, entity);
    let Some(beliefs) = sim.components.beliefs.get_mut(mind) else {
        return Ok(false);
    };
    match beliefs
        .tracks
        .binary_search_by_key(&persona, |track| track.persona)
    {
        Ok(at) => {
            beliefs.tracks.remove(at);
            Ok(true)
        }
        Err(_) => Ok(false),
    }
}

/// Records on a mind's track of an entity that it was seen, or said, to
/// use an action or ability (15 §7.1): its id, the strongest result and
/// when. The list keeps `track_capabilities` entries, the weakest and
/// oldest going first. A mind with no track of the entity gains one,
/// with no place. Refused for a result that is not finite.
pub fn observe_capability(
    sim: &mut Sim,
    mind: EntityId,
    entity: EntityId,
    what: &str,
    result: Real,
    told: bool,
) -> Result<(), Reason> {
    sim.known(mind)?;
    sim.known(entity)?;
    if !result.is_finite() {
        return Err(
            Reason::new(why::BAD_VALUE, "a capability's result is a finite number")
                .with("value", result),
        );
    }
    if track_of(sim, mind, entity).is_none() {
        let how = if told {
            Knowing::Told
        } else {
            Knowing::Inferred
        };
        put_track(sim, mind, entity, None, how, Real::ONE);
    }
    let persona = persona_of(sim, mind, entity);
    let now = local_now(sim, mind);
    let max = count_setting(sim, TRACK_CAPABILITIES_SETTING, DEFAULT_TRACK_CAPABILITIES);
    let Some(track) = sim.components.beliefs.get_mut(mind).and_then(|beliefs| {
        let at = beliefs
            .tracks
            .binary_search_by_key(&persona, |track| track.persona)
            .ok()?;
        beliefs.tracks.get_mut(at)
    }) else {
        return Ok(());
    };
    match track.capabilities.iter_mut().find(|have| have.what == what) {
        Some(have) => {
            have.best = have.best.max(result);
            have.at = now;
            // Seen once is seen: being told again does not undo it.
            have.told &= told;
        }
        None => track.capabilities.push(Capability {
            what: what.to_owned(),
            best: result,
            at: now,
            told,
        }),
    }
    track
        .capabilities
        .sort_by(|a, b| (b.best, b.at, &a.what).cmp(&(a.best, a.at, &b.what)));
    track.capabilities.truncate(max);
    Ok(())
}

/// A mind remembers something (the `remember` effect, 15 §7.1). The
/// list keeps `mind.max_memories`; the least salient now, then the
/// oldest, is forgotten to make room, with `memory_forgotten`. Raises
/// `memory_added`. Refused for a salience or a confidence outside 0 to
/// 1.
pub fn remember(sim: &mut Sim, mind: EntityId, memory: Memory) -> Result<(), Reason> {
    sim.known(mind)?;
    for (name, value) in [
        ("salience", memory.salience),
        ("confidence", memory.confidence),
    ] {
        if !value.is_finite() || value < Real::ZERO || value > Real::ONE {
            return Err(
                Reason::new(why::BAD_VALUE, format!("a memory's {name} is from 0 to 1"))
                    .with("value", value),
            );
        }
    }
    let max = count_setting(sim, MAX_MEMORIES_SETTING, DEFAULT_MAX_MEMORIES);
    let halflife = seconds_setting(sim, SALIENCE_HALFLIFE_SETTING, DEFAULT_SALIENCE_HALFLIFE_S);
    let now = local_now(sim, mind);
    let (event, loc, salience) = (memory.event.clone(), memory.loc, memory.salience);
    let beliefs = sim
        .components
        .beliefs
        .get_or_insert_with(mind, Beliefs::default);
    beliefs.memories.push(memory);
    let mut forgotten = Vec::new();
    while beliefs.memories.len() > max {
        let Some(weakest) = beliefs
            .memories
            .iter()
            .enumerate()
            .min_by(|(i, a), (j, b)| {
                let fade = |memory: &Memory| {
                    faded(memory.salience, (now - memory.at).as_secs_real(), halflife)
                };
                (fade(a), a.at, *i).cmp(&(fade(b), b.at, *j))
            })
            .map(|(at, _)| at)
        else {
            break;
        };
        forgotten.push(beliefs.memories.remove(weakest));
    }
    events::memory_added(sim, mind, &event, loc, salience);
    for memory in forgotten {
        events::memory_forgotten(sim, mind, &memory.event, memory.loc);
    }
    Ok(())
}

/// A mind perceived an event that its signal says is worth remembering.
pub(super) fn remember_percept(
    sim: &mut Sim,
    mind: EntityId,
    event: &str,
    at: Location,
    actor: Option<EntityId>,
    kind: SignalKindId,
    salience: Real,
) {
    let actor = actor.map(|actor| persona_of(sim, mind, actor));
    let memory = Memory {
        event: event.to_owned(),
        loc: Some(at),
        at: local_now(sim, mind),
        roles: vec![("actor".to_owned(), actor)],
        how: Knowing::Perceived(kind),
        confidence: Real::ONE,
        salience,
    };
    let _ = remember(sim, mind, memory);
}

/// A mind notices an element of a cell as it is now (15 §7.1): it is
/// noted with its state, which is how a mind comes to know an element
/// tagged `hidden` (15 §7.3). The list keeps `mind.max_noted`, the oldest
/// going first. Refused for a cell that cannot be read.
pub fn note_element(
    sim: &mut Sim,
    mind: EntityId,
    loc: Location,
    layer: Layer,
) -> Result<(), Reason> {
    sim.known(mind)?;
    let loc = sim
        .cosmos
        .normalize(loc)
        .map_err(|error| Reason::from_world(&error))?;
    let cell = sim
        .cosmos
        .cell(loc)
        .map_err(|error| Reason::from_world(&error))?;
    let state = match layer {
        Layer::Floor => cell.floor.raw(),
        Layer::Block => cell.block.raw(),
        Layer::Fixture => cell.fixture.raw(),
    };
    let now = local_now(sim, mind);
    let max = count_setting(sim, MAX_NOTED_SETTING, DEFAULT_MAX_NOTED);
    let beliefs = sim
        .components
        .beliefs
        .get_or_insert_with(mind, Beliefs::default);
    beliefs
        .noted
        .retain(|noted| !(noted.loc == loc && noted.layer == layer));
    beliefs.noted.push(Noted {
        loc,
        layer,
        state,
        at: now,
    });
    while beliefs.noted.len() > max {
        beliefs.noted.remove(0);
    }
    Ok(())
}

/// Whether a mind has noted the element of a cell on a layer, in the
/// state it is in now.
pub fn has_noted(
    sim: &Sim,
    mind: EntityId,
    loc: Location,
    layer: Layer,
    state: ElementState,
) -> bool {
    sim.components.beliefs.get(mind).is_some_and(|beliefs| {
        beliefs
            .noted
            .iter()
            .any(|noted| noted.loc == loc && noted.layer == layer && noted.state.def == state.def)
    })
}
