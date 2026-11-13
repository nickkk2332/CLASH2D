//! What minds keep (09 §3, §4): the `MindState` component of each
//! character that thinks, and the [`AiState`] the simulation holds for
//! all of them. Both are plain data that saves and hashes; the module
//! that changes them is [`think`](super::think).

use std::collections::BTreeSet;

use clash_core::{Dur, EntityId, Real, Rng, StateHash, StateHasher, Tick, impl_state_hash};
use clash_time::WakeId;
use clash_world::Location;
use serde::{Deserialize, Serialize};

use super::defs::{GoalId, StepKind};
use crate::later::ActionId;
use crate::perception::PersonaId;
use crate::reason::Reason;

/// How a mind stands toward someone, until relations are built (S7).
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Stance {
    /// Held hostile: an enemy.
    Hostile,
    /// Feared: someone to get away from.
    Feared,
}

impl Stance {
    /// The word rules read.
    pub const fn name(self) -> &'static str {
        match self {
            Stance::Hostile => "hostile",
            Stance::Feared => "feared",
        }
    }
}

impl StateHash for Stance {
    fn state_hash(&self, hasher: &mut StateHasher) {
        hasher.write_u8(*self as u8);
    }
}

/// What a held goal is about, as the mind keeps it between thoughts.
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AboutHeld {
    /// Nothing.
    None,
    /// The persona of a track.
    Track(PersonaId),
    /// A memory, by what it is, where and when it happened.
    Memory(MemoryKey),
    /// A cell.
    Place(Location),
}

impl StateHash for AboutHeld {
    fn state_hash(&self, hasher: &mut StateHasher) {
        match self {
            AboutHeld::None => hasher.write_u8(0),
            AboutHeld::Track(persona) => {
                hasher.write_u8(1);
                persona.state_hash(hasher);
            }
            AboutHeld::Memory(key) => {
                hasher.write_u8(2);
                key.state_hash(hasher);
            }
            AboutHeld::Place(loc) => {
                hasher.write_u8(3);
                loc.state_hash(hasher);
            }
        }
    }
}

/// Which memory: its event, where and the mind's local time it happened.
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Debug, Serialize, Deserialize)]
pub struct MemoryKey {
    /// The event type.
    pub event: String,
    /// Where.
    pub loc: Option<Location>,
    /// When, in the mind's local time.
    pub at: Dur,
}

impl_state_hash!(MemoryKey { event, loc, at });

/// What came of a step that takes time.
#[derive(Clone, PartialEq, Eq, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Outcome {
    /// Nothing is known yet.
    #[default]
    Running,
    /// It ended as it should.
    Done,
    /// It ended short, for this reason.
    Failed(Reason),
}

impl StateHash for Outcome {
    fn state_hash(&self, hasher: &mut StateHasher) {
        match self {
            Outcome::Running => hasher.write_u8(0),
            Outcome::Done => hasher.write_u8(1),
            Outcome::Failed(reason) => {
                hasher.write_u8(2);
                reason.state_hash(hasher);
            }
        }
    }
}

/// A step that takes time and is under way.
#[derive(Clone, PartialEq, Debug, Serialize, Deserialize)]
pub struct Pending {
    /// What kind of step.
    pub kind: u8,
    /// The action it started, if it started one.
    pub action: Option<ActionId>,
    /// For a step that moves: where it is going, and how close is close
    /// enough, in metres.
    pub target: Option<Location>,
    /// How close is close enough, in metres.
    pub within: Real,
    /// For a step that follows someone: whom.
    pub whom: Option<EntityId>,
    /// What came of it so far.
    pub outcome: Outcome,
    /// For a step that moves: a closed way it met, and opening it
    /// ([`doors`](super::doors)).
    #[serde(default)]
    pub door: Door,
}

impl StateHash for Pending {
    fn state_hash(&self, hasher: &mut StateHasher) {
        self.kind.state_hash(hasher);
        self.action.state_hash(hasher);
        self.target.state_hash(hasher);
        self.within.state_hash(hasher);
        self.whom.state_hash(hasher);
        self.outcome.state_hash(hasher);
        // A step that met no door hashes as before doors were opened.
        if self.door != Door::None {
            self.door.state_hash(hasher);
        }
    }
}

/// A closed way a step that moves met, and what is being done about it
/// (09 §5.3; [`doors`](super::doors)).
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Door {
    /// None met.
    #[default]
    None,
    /// A step was refused because the way is closed, in this direction
    /// (the step's `dir` as data writes it, by its index in
    /// `clash_world::StepKind::ALL`), after `tries` openings.
    Met {
        /// The direction, by its index.
        dir: u8,
        /// How many times a door on this step has been opened already.
        tries: u8,
    },
    /// The interaction that opens it is under way, as this action.
    Opening {
        /// The action doing the interaction.
        action: ActionId,
        /// How many times a door on this step has been opened, this
        /// one included.
        tries: u8,
    },
    /// It was opened (or the opening ended): the step is begun again.
    Opened {
        /// How many times a door on this step has been opened.
        tries: u8,
    },
    /// Begun again after opening doors: walking on.
    Walking {
        /// How many times a door on this step has been opened.
        tries: u8,
    },
}

impl StateHash for Door {
    fn state_hash(&self, hasher: &mut StateHasher) {
        match *self {
            Door::None => hasher.write_u8(0),
            Door::Met { dir, tries } => {
                hasher.write_u8(1);
                hasher.write_u8(dir);
                hasher.write_u8(tries);
            }
            Door::Opening { action, tries } => {
                hasher.write_u8(2);
                action.state_hash(hasher);
                hasher.write_u8(tries);
            }
            Door::Opened { tries } => {
                hasher.write_u8(3);
                hasher.write_u8(tries);
            }
            Door::Walking { tries } => {
                hasher.write_u8(4);
                hasher.write_u8(tries);
            }
        }
    }
}

impl Door {
    /// How many times a door on the step has been opened.
    pub const fn tries(self) -> u8 {
        match self {
            Door::None => 0,
            Door::Met { tries, .. }
            | Door::Opening { tries, .. }
            | Door::Opened { tries }
            | Door::Walking { tries } => tries,
        }
    }
}

impl Pending {
    /// A pending step of a kind.
    pub fn new(kind: StepKind) -> Pending {
        Pending {
            kind: step_code(kind),
            action: None,
            target: None,
            within: Real::ZERO,
            whom: None,
            outcome: Outcome::Running,
            door: Door::None,
        }
    }

    /// The kind of step.
    pub fn kind(&self) -> StepKind {
        step_of(self.kind)
    }
}

/// The number a step kind is kept as.
const fn step_code(kind: StepKind) -> u8 {
    match kind {
        StepKind::Go => 0,
        StepKind::Do => 1,
        StepKind::Choose => 2,
        StepKind::Wait => 3,
        StepKind::Look => 4,
        StepKind::Say => 5,
        StepKind::Set => 6,
        StepKind::Settle => 7,
        StepKind::Stance => 8,
        StepKind::KeepNear => 9,
        StepKind::KeepFrom => 10,
    }
}

const fn step_of(code: u8) -> StepKind {
    match code {
        0 => StepKind::Go,
        1 => StepKind::Do,
        2 => StepKind::Choose,
        4 => StepKind::Look,
        5 => StepKind::Say,
        6 => StepKind::Set,
        7 => StepKind::Settle,
        8 => StepKind::Stance,
        9 => StepKind::KeepNear,
        10 => StepKind::KeepFrom,
        _ => StepKind::Wait,
    }
}

/// The goal a mind holds and how far it has got with it (09 §3.3, §4).
#[derive(Clone, PartialEq, Debug, Serialize, Deserialize)]
pub struct Held {
    /// The goal.
    pub goal: GoalId,
    /// What it is about.
    pub about: AboutHeld,
    /// Its score when last scored.
    pub score: Real,
    /// Whether a rule set it (`set_goal`): it is not displaced by
    /// scoring, only by being done, failing or `clear_goal`.
    pub forced: bool,
    /// The method in use: its position in the goal's `methods`.
    pub method: Option<u16>,
    /// The methods that failed in this pursuit, which are not used again
    /// until the goal is adopted anew.
    pub failed: Vec<u16>,
    /// The step of the method in hand.
    pub step: u16,
    /// The step under way, if one takes time.
    pub pending: Option<Pending>,
}

impl_state_hash!(Held {
    goal,
    about,
    score,
    forced,
    method,
    failed,
    step,
    pending
});

/// What a character that thinks keeps (09 §3, §4): the `mind_state`
/// component. Made the first time its mind thinks.
#[derive(Clone, PartialEq, Debug, Serialize, Deserialize)]
pub struct MindState {
    /// The `mind` definition `set_mind_def` gave it, in place of its
    /// mind's profile, as written.
    pub(super) def: Option<String>,
    /// The goal held, if any.
    pub(super) held: Option<Held>,
    /// Its own stream: whatever chance is in its choices is drawn from
    /// here and nowhere else (09 §1).
    pub(super) rng: Rng,
    /// Where it belongs: where it first thought, or what `set_home`
    /// said.
    pub(super) home: Option<Location>,
    /// How it stands toward others, by persona, in order of persona.
    pub(super) stances: Vec<(PersonaId, Stance)>,
    /// Tracks it has settled, with the time each was last perceived
    /// when it was settled: a track perceived since is not settled.
    pub(super) settled_tracks: Vec<(PersonaId, Dur)>,
    /// Memories it has settled, oldest first, at most
    /// [`MAX_SETTLED`](super::MAX_SETTLED).
    pub(super) settled_memories: Vec<MemoryKey>,
    /// Goals set aside after failing, until a tick.
    pub(super) penalties: Vec<(GoalId, Tick)>,
    /// How long it now waits between thoughts while nothing happens: its
    /// `think`, stretched while nothing changes.
    pub(super) interval: Dur,
    /// When it next thinks with nothing to prompt it.
    pub(super) next_think: Option<Tick>,
    /// The tick of its last thought, and how many it has had then.
    pub(super) thought_at: Tick,
    pub(super) thoughts: u32,
    /// Why its last step failed, for the inspector (09 §16).
    pub(super) last_failure: Option<Reason>,
    /// The wake-up of the wait it took to rest, with nothing to do: a
    /// goal taken up ends it.
    pub(super) rest: Option<WakeId>,
    /// Closed ways it found it could not open, most recent last, at most
    /// [`MAX_SHUT`](super::doors::MAX_SHUT): its paths go round them
    /// ([`doors`](super::doors)).
    #[serde(default)]
    pub(super) shut: Vec<Location>,
}

impl StateHash for MindState {
    fn state_hash(&self, hasher: &mut StateHasher) {
        self.def.state_hash(hasher);
        self.held.state_hash(hasher);
        self.rng.state_hash(hasher);
        self.home.state_hash(hasher);
        self.stances.state_hash(hasher);
        self.settled_tracks.state_hash(hasher);
        self.settled_memories.state_hash(hasher);
        self.penalties.state_hash(hasher);
        self.interval.state_hash(hasher);
        self.next_think.state_hash(hasher);
        self.thought_at.state_hash(hasher);
        self.thoughts.state_hash(hasher);
        self.last_failure.state_hash(hasher);
        self.rest.state_hash(hasher);
        // A mind that has met no shut door hashes as before.
        if !self.shut.is_empty() {
            self.shut.state_hash(hasher);
        }
    }
}

impl MindState {
    /// A new state, drawing from its own stream.
    pub(super) fn new(rng: Rng, home: Option<Location>, interval: Dur) -> MindState {
        MindState {
            def: None,
            held: None,
            rng,
            home,
            stances: Vec::new(),
            settled_tracks: Vec::new(),
            settled_memories: Vec::new(),
            penalties: Vec::new(),
            interval,
            next_think: None,
            thought_at: Tick::ZERO,
            thoughts: 0,
            last_failure: None,
            rest: None,
            shut: Vec::new(),
        }
    }

    /// The closed ways it found it could not open, most recent last.
    pub fn shut_doors(&self) -> &[Location] {
        &self.shut
    }

    /// A state for tests of the component store, differing by `seed`.
    #[cfg(test)]
    pub(crate) fn sample(seed: u32) -> MindState {
        MindState::new(
            Rng::from_seed(u64::from(seed)),
            None,
            Dur::from_micros(i64::from(seed) + 1),
        )
    }

    /// The goal held, if any.
    pub fn held(&self) -> Option<&Held> {
        self.held.as_ref()
    }

    /// Where it belongs.
    pub fn home(&self) -> Option<Location> {
        self.home
    }

    /// How it stands toward a persona, if it holds a stance.
    pub fn stance(&self, persona: PersonaId) -> Option<Stance> {
        self.stances
            .binary_search_by_key(&persona, |(have, _)| *have)
            .ok()
            .map(|at| self.stances[at].1)
    }

    /// Every stance it holds, by persona, in order of persona.
    pub fn stances(&self) -> &[(PersonaId, Stance)] {
        &self.stances
    }

    /// Sets or clears its stance toward a persona.
    pub(super) fn set_stance(&mut self, persona: PersonaId, stance: Option<Stance>) {
        match (
            self.stances
                .binary_search_by_key(&persona, |(have, _)| *have),
            stance,
        ) {
            (Ok(at), Some(stance)) => self.stances[at].1 = stance,
            (Ok(at), None) => {
                self.stances.remove(at);
            }
            (Err(at), Some(stance)) => self.stances.insert(at, (persona, stance)),
            (Err(_), None) => {}
        }
    }

    /// Why its last step failed, if one has.
    pub fn last_failure(&self) -> Option<&Reason> {
        self.last_failure.as_ref()
    }

    /// The mind definition set in place of its profile, if one was.
    pub fn def_override(&self) -> Option<&str> {
        self.def.as_deref()
    }

    /// When it next thinks with nothing to prompt it.
    pub fn next_think(&self) -> Option<Tick> {
        self.next_think
    }
}

/// What the simulation keeps for every mind at once: who is due to
/// think, and when those with nothing to prompt them think next.
/// Saved with the simulation and hashed with the kernel.
#[derive(Clone, PartialEq, Eq, Debug, Default, Serialize, Deserialize)]
pub struct AiState {
    /// Minds prompted to think at the next batch.
    pub(super) due: BTreeSet<EntityId>,
    /// Minds that think at a tick with nothing to prompt them, in order
    /// of tick and then entity.
    pub(super) schedule: BTreeSet<(Tick, EntityId)>,
    /// Cells whose geometry changed since routes in use were last
    /// checked.
    pub(super) changed: BTreeSet<Location>,
    /// Whether more changed than `changed` keeps: every route is
    /// checked.
    pub(super) all_changed: bool,
    /// Who is hot and who warm, and the warm stepper (09 §9;
    /// [`warm`](super::warm)).
    #[serde(default)]
    pub(super) levels: super::warm::Levels,
    /// What a mind's thought reads again and again, kept while it weighs
    /// its goals (D-922). Derived: not saved, hashed or compared.
    #[serde(skip)]
    pub(crate) stances: StanceMemo,
}

/// What a mind's goals and steps read again and again, kept while its
/// goals are weighed and its steps' conditions read (D-922): how minds
/// stand toward others ([`stance_toward`](super::stance_toward)), and
/// which part of its day a routine has a character in. While it is kept
/// the simulation is only read, so an answer once worked out is the
/// answer again. Empty and kept by no one otherwise. Derived data: a
/// copy starts empty, it is neither saved nor hashed, and two are
/// always equal.
#[derive(Default)]
pub(crate) struct StanceMemo(std::sync::Mutex<Option<Kept>>);

/// A routine's part of the day as a [`StanceMemo`] keeps it: by the
/// routine's definition (its address, for as long as the definitions are
/// read), the realm, the character's seed and the reckoned time.
pub(crate) type PartKey = (usize, clash_world::RealmId, u64, Dur);

/// The answers a [`StanceMemo`] keeps.
#[derive(Default)]
struct Kept {
    /// A mind, whom it stands toward, how.
    stances: Vec<(EntityId, EntityId, Option<Stance>)>,
    /// A routine's part of the day, and when it began.
    parts: Vec<(PartKey, Option<(usize, Dur)>)>,
}

impl StanceMemo {
    /// Begins keeping answers, from none.
    pub(super) fn begin(&self) {
        *self.lock() = Some(Kept::default());
    }

    /// Stops keeping answers and forgets them.
    pub(super) fn end(&self) {
        *self.lock() = None;
    }

    /// The answer kept for a mind toward another, if answers are being
    /// kept and this one has been worked out.
    pub(super) fn get(&self, mind: EntityId, other: EntityId) -> Option<Option<Stance>> {
        self.lock().as_ref().and_then(|kept| {
            kept.stances
                .iter()
                .find(|(have, toward, _)| *have == mind && *toward == other)
                .map(|(_, _, stance)| *stance)
        })
    }

    /// Keeps an answer, if answers are being kept.
    pub(super) fn put(&self, mind: EntityId, other: EntityId, stance: Option<Stance>) {
        if let Some(kept) = self.lock().as_mut() {
            kept.stances.push((mind, other, stance));
        }
    }

    /// The part of the day kept for a routine, a seed and a reckoned
    /// time, if answers are being kept and this one has been worked out.
    pub(crate) fn part(&self, key: PartKey) -> Option<Option<(usize, Dur)>> {
        self.lock().as_ref().and_then(|kept| {
            kept.parts
                .iter()
                .find(|(have, _)| *have == key)
                .map(|(_, part)| *part)
        })
    }

    /// Keeps a part of the day, if answers are being kept.
    pub(crate) fn put_part(&self, key: PartKey, part: Option<(usize, Dur)>) {
        if let Some(kept) = self.lock().as_mut() {
            kept.parts.push((key, part));
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Option<Kept>> {
        // A lock left poisoned is taken as it is: what it holds is only
        // ever whole answers.
        match self.0.lock() {
            Ok(kept) => kept,
            Err(poisoned) => poisoned.into_inner(),
        }
    }
}

impl Clone for StanceMemo {
    fn clone(&self) -> StanceMemo {
        StanceMemo::default()
    }
}

impl PartialEq for StanceMemo {
    fn eq(&self, _: &StanceMemo) -> bool {
        true
    }
}

impl Eq for StanceMemo {}

impl core::fmt::Debug for StanceMemo {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str("StanceMemo")
    }
}

impl StateHash for AiState {
    fn state_hash(&self, hasher: &mut StateHasher) {
        self.due.state_hash(hasher);
        self.schedule.state_hash(hasher);
        self.changed.state_hash(hasher);
        self.all_changed.state_hash(hasher);
        // Levels hash only while they hold something, so that a
        // simulation in which nobody was ever warm hashes as before.
        if !self.levels.is_empty() {
            self.levels.state_hash(hasher);
        }
    }
}

impl AiState {
    /// Whether nothing is due or scheduled and nothing changed.
    pub fn is_empty(&self) -> bool {
        self.due.is_empty()
            && self.schedule.is_empty()
            && self.changed.is_empty()
            && self.levels.is_empty()
    }

    /// How many minds are prompted to think at the next batch.
    pub fn due_count(&self) -> usize {
        self.due.len()
    }

    /// How many minds have a thought scheduled.
    pub fn scheduled_count(&self) -> usize {
        self.schedule.len()
    }

    /// Who is hot and who warm (09 §9).
    pub fn levels(&self) -> &super::warm::Levels {
        &self.levels
    }
}
