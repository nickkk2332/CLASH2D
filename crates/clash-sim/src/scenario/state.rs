//! A scenario's state in the simulation (13 §3): what game is being
//! played, its objectives and missions, the checkpoints made and how it
//! ended. Saved with the simulation and hashed with it (invariant Z8).

use std::collections::BTreeSet;

use clash_core::{EntityId, StateHash, StateHasher, Tick, Value};
use clash_world::{Location, RealmId, SiteId};
use serde::{Deserialize, Serialize};

/// Where an objective stands (13 §5.1).
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ObjectiveState {
    /// Not shown yet: waiting for those it comes after, or for a rule.
    Hidden,
    /// Shown, and not settled.
    Active,
    /// Done. It does not change again.
    Done,
    /// Failed. It does not change again.
    Failed,
}

impl ObjectiveState {
    /// The word rules and screens read.
    pub const fn name(self) -> &'static str {
        match self {
            ObjectiveState::Hidden => "hidden",
            ObjectiveState::Active => "active",
            ObjectiveState::Done => "done",
            ObjectiveState::Failed => "failed",
        }
    }

    /// Whether it is done or failed, after which it does not change
    /// (invariant Z4).
    pub const fn settled(self) -> bool {
        matches!(self, ObjectiveState::Done | ObjectiveState::Failed)
    }
}

impl StateHash for ObjectiveState {
    fn state_hash(&self, hasher: &mut StateHasher) {
        hasher.write_u8(*self as u8);
    }
}

/// One objective of a game, as it stands.
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub struct Objective {
    /// Its id, as its scenario or mission writes it.
    pub id: String,
    /// Where it stands.
    pub state: ObjectiveState,
    /// When it fails if it is still active, in universal time.
    pub expires: Option<Tick>,
}

clash_core::impl_state_hash!(Objective { id, state, expires });

/// Where a mission stands (13 §5.2).
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MissionState {
    /// Being done.
    Active,
    /// Every objective done.
    Done,
    /// An objective failed, or its `failed` came to hold.
    Failed,
    /// Its time ran out.
    Expired,
}

impl MissionState {
    /// The word rules and screens read.
    pub const fn name(self) -> &'static str {
        match self {
            MissionState::Active => "active",
            MissionState::Done => "done",
            MissionState::Failed => "failed",
            MissionState::Expired => "expired",
        }
    }
}

impl StateHash for MissionState {
    fn state_hash(&self, hasher: &mut StateHasher) {
        hasher.write_u8(*self as u8);
    }
}

/// A mission someone took on (13 §5.2).
#[derive(Clone, PartialEq, Debug, Serialize, Deserialize)]
pub struct Mission {
    /// Its number in this game, from 1: what `mission_state` and the
    /// mission's events name it by.
    pub serial: u32,
    /// The qualified id of its definition.
    pub def: String,
    /// Who took it on.
    pub taker: EntityId,
    /// The values it was started with, in the order of its parameters.
    pub params: Vec<Value>,
    /// Where it stands.
    pub state: MissionState,
    /// When it was started, in universal time.
    pub started: Tick,
    /// When it expires if it is still active.
    pub expires: Option<Tick>,
    /// Its objectives, in the order its definition writes them.
    pub objectives: Vec<Objective>,
}

clash_core::impl_state_hash!(Mission {
    serial,
    def,
    taker,
    params,
    state,
    started,
    expires,
    objectives
});

/// The game being played (13 §3).
#[derive(Clone, PartialEq, Debug, Serialize, Deserialize)]
pub struct Game {
    /// The qualified id of the scenario.
    pub scenario: String,
    /// When it began, in universal time: the world's own time may have
    /// been set before (13 §4.3).
    pub began: Tick,
    /// The characters the player plays, directly controlled, in the
    /// order they were taken up.
    pub party: Vec<EntityId>,
    /// Those the player played who are gone: dead, or no longer theirs.
    pub fallen: Vec<EntityId>,
    /// Which of the scenario's places the party began at.
    pub place: u32,
    /// The realm the party began in.
    pub realm: Option<RealmId>,
    /// The site the party began at, if it began at one.
    pub site: Option<SiteId>,
    /// The cell the party began at.
    pub at: Option<Location>,
    /// The scenario's own objectives, in the order it writes them.
    pub objectives: Vec<Objective>,
    /// Every mission started, in order.
    pub missions: Vec<Mission>,
    /// The checkpoints asked for, oldest first.
    pub checkpoints: Vec<String>,
    /// Whether the party is gone and what follows has been done.
    pub gone: bool,
    /// The outcome, once the game has ended.
    pub ended: Option<String>,
}

clash_core::impl_state_hash!(Game {
    scenario,
    began,
    party,
    fallen,
    place,
    realm,
    site,
    at,
    objectives,
    missions,
    checkpoints,
    gone,
    ended
});

impl Game {
    /// A mission by its number.
    pub fn mission(&self, serial: u32) -> Option<&Mission> {
        self.missions
            .iter()
            .find(|mission| mission.serial == serial)
    }
}

/// What the simulation keeps of the scenario: the game, if one is being
/// played, and what is waiting to be looked at again at the next safe
/// point. All of it is saved, and all of it is hashed while there is a
/// game: what waits decides what happens next.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct ScenarioState {
    /// The game being played, if one is. A simulation that was never
    /// given one (a test, a tool) has none, and nothing of this is
    /// hashed.
    pub game: Option<Game>,
    /// The events delivered since the objectives were last looked at
    /// that one of them watches.
    pub(crate) stale: BTreeSet<String>,
    /// Whether everything must be looked at again: a wake-up, a change
    /// by a verb.
    pub(crate) dirty: bool,
    /// Endings waiting to take effect, by the `on` of an ending.
    pub(crate) pending_ends: Vec<String>,
    /// Whether the objectives are being looked at now. Not state: false
    /// between any two calls.
    #[serde(skip)]
    pub(crate) settling: bool,
}

impl ScenarioState {
    /// Whether a game is being played and has not ended.
    pub fn playing(&self) -> bool {
        self.game.as_ref().is_some_and(|game| game.ended.is_none())
    }
}

impl StateHash for ScenarioState {
    fn state_hash(&self, hasher: &mut StateHasher) {
        // A simulation with no game hashes as it did before scenarios
        // existed.
        if let Some(game) = &self.game {
            game.state_hash(hasher);
            hasher.write_u64(self.stale.len() as u64);
            for name in &self.stale {
                hasher.write_str(name);
            }
            self.dirty.state_hash(hasher);
            self.pending_ends.state_hash(hasher);
        }
    }
}
