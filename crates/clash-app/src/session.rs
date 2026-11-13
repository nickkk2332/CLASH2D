//! A session: one game being played (13 §4, 03 §5.1).
//!
//! [`new_game`] makes the simulation a game runs in, from a game
//! scenario (13 §3): the scenario's world, made by
//! `clash_sim::scenario::make_world` with the seed its `seed` chooses
//! and its clock at the time it begins; the generators the loaded
//! definitions give and a tract store; the player's character from its
//! templates ([`spawn_player`]); and the command `BeginScenario`, which
//! gives the character senses, places it, gives it direct control and
//! sets the game up (13 §12). Everything after the simulation exists is
//! done to it by command (`Sim::submit`).
//!
//! What the party perceives is the simulation's (15 §13): the session
//! keeps only what the application has taken of it for the view model
//! ([`Session::percepts`], [`crate::perception`]).

use std::sync::Arc;

use clash_core::EntityId;
use clash_scene::{AppEntry, GlyphDef};
use clash_sim::mind::VitalState;
use clash_sim::scenario::{BeginScenario, ScenarioDef, make_world};
use clash_sim::template::CharacterSpec;
use clash_sim::{Command, Control, DebugCommand, Place, Sim, Subject};
use clash_world::{Location, MemoryStore};

use crate::boot::Loaded;
use crate::perception::{self, Percepts};
use crate::saves::{SaveMeta, SavedGame};

/// One game being played: the simulation, and whose eyes it is seen
/// through.
#[derive(Debug)]
pub struct Session {
    /// The simulation. Advanced only by the frame loop, changed only by
    /// command.
    pub sim: Sim,
    /// The player's character.
    pub player: EntityId,
    /// What the player called the character: kept beside the save, and
    /// what `name()` of the character gives while it has no name of its
    /// own (a template with no choice that sets one).
    pub name: String,
    /// The scenario the game began from: `app.scenario`.
    pub scenario: AppEntry,
    /// How the character is drawn: the glyph of its templates. `None` if
    /// they give none: it is then drawn as a presence.
    pub look: Option<GlyphDef>,
    /// What the application has taken of the party's perception for the
    /// view model ([`crate::perception`]).
    pub percepts: Percepts,
    /// The saving policy of the game (13 §9.2): `free`, `single`,
    /// `checkpoints` or `none`, as the scenario says.
    pub saving: String,
    /// The seed the game's world was made with: kept in its saves.
    pub seed: u64,
    /// Whether the simulation has moved since the character last looked.
    pub stale_view: bool,
    /// The direction of the last step the simulation refused because the
    /// way is closed (`move.closed`): what the frame loop offers to open.
    pub bumped: Option<String>,
    /// Whether the play screen has been on show for this game. Once it
    /// has, the game is over when it no longer is.
    pub shown: bool,
    /// Whether the last feed of the view was the omniscient one.
    pub omniscient: bool,
    /// The outcome, once the game has ended (13 §9.4): the simulation is
    /// no longer advanced, and leaving it saves nothing.
    pub ended: Option<String>,
}

impl Session {
    /// The grid and cell the player's character is in, if it is in the
    /// world.
    pub fn player_loc(&self) -> Option<Location> {
        self.sim.truth().location(self.player)
    }

    /// Whether a wake-up is due at the present tick: what the
    /// simulation runs before it asks for a decision (03 §5.2), such as
    /// the sweep of a sense whose owner has just moved.
    pub fn due_now(&self) -> bool {
        self.ended.is_none()
            && self
                .sim
                .timeline()
                .peek()
                .is_some_and(|due| due <= self.sim.now())
    }

    /// Whether the player is waiting on the simulation: no decision is
    /// asked of it, and it is busy (an action, a wait, a plan) or
    /// unconscious, so that time must pass before it is asked. Not while
    /// it is free, dead or out of the world: nothing would ever ask it.
    /// Never once the game has ended.
    pub fn waits_on_simulation(&self) -> bool {
        let sim = &self.sim;
        if self.ended.is_some() || sim.pending_decision().is_some() || self.player_loc().is_none() {
            return false;
        }
        let driven = sim.driver(self.player).is_some();
        match sim.vital(self.player) {
            Some(VitalState::Dead) if !driven => false,
            Some(VitalState::Unconscious) if !driven => true,
            _ => !sim.is_free(self.player),
        }
    }
}

/// Makes the player's character from its templates (06 §10.3), out of
/// the world: the debug command `spawn`, so that it is in the input log
/// as everything else done to the simulation is. Refused with the
/// template's reason: an unknown template, a selection a choice does not
/// take.
pub fn spawn_player(sim: &mut Sim, spec: CharacterSpec) -> Result<EntityId, String> {
    let id = sim
        .truth()
        .entities()
        .next_id()
        .ok_or("every entity slot has been used")?;
    sim.submit(Command::Debug(DebugCommand::Spawn {
        spec,
        place: Place::Stored,
    }))
    .map_err(|refusal| format!("the character could not be made: {refusal}"))?;
    Ok(id)
}

/// A scenario by its qualified id, and its entry as `app.scenarios`
/// lists it.
pub fn scenario_of<'a>(
    loaded: &'a Loaded,
    id: &str,
) -> Result<(&'a ScenarioDef, AppEntry), String> {
    let scenarios = loaded
        .defs
        .try_registry::<ScenarioDef>()
        .ok_or("the loaded mods define no game scenario to start")?;
    let found = scenarios
        .lookup(id)
        .ok_or_else(|| format!("there is no game scenario `{id}`"))?;
    let def = scenarios.get(found);
    Ok((
        def,
        AppEntry {
            id: id.to_owned(),
            name: def.shown_name(id),
            description: def.doc.clone(),
            state: "current".to_owned(),
        },
    ))
}

/// The qualified id of the template a scenario makes its character from
/// by default, if it names one.
pub fn scenario_template(loaded: &Loaded, scenario: &str) -> Result<Option<String>, String> {
    let (def, _) = scenario_of(loaded, scenario)?;
    let namespace = scenario
        .split_once(':')
        .map_or("", |(namespace, _)| namespace);
    Ok(def.template_id(namespace))
}

/// A session round a simulation that has its player.
fn session_of(
    sim: Sim,
    player: EntityId,
    name: &str,
    def: &ScenarioDef,
    entry: AppEntry,
) -> Session {
    let look = perception::look_of(&sim, player);
    Session {
        sim,
        player,
        name: name.to_owned(),
        saving: def.saving_policy().to_owned(),
        scenario: entry,
        look,
        percepts: Percepts::afresh(),
        stale_view: true,
        bumped: None,
        seed: 0,
        shown: false,
        omniscient: false,
        ended: None,
    }
}

/// What the player chose for a new game beside the character: the seed
/// given, one drawn at random for a scenario whose seed is `random`,
/// the place to begin at and the answers to the scenario's options.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Answers {
    /// The seed the player gave.
    pub seed: u64,
    /// A seed drawn for a scenario that asks for a random one.
    pub drawn: u64,
    /// Which of the scenario's places, from 0.
    pub place: u32,
    /// Settings by key, with the values chosen.
    pub options: Vec<(String, clash_core::Value)>,
}

/// Starts a game from a scenario (13 §12): makes its world, the player's
/// character from `spec` and the session, and begins it.
///
/// `scenario` is the qualified id of a `scenario` definition; `name` is
/// what the player called the character. Whatever cannot be done is
/// returned as a sentence to show the player.
pub fn new_game(
    loaded: &Loaded,
    scenario: &str,
    answers: &Answers,
    spec: CharacterSpec,
    name: &str,
) -> Result<Session, String> {
    let defs = &loaded.defs;
    let tables = loaded
        .tables
        .as_ref()
        .map_err(|problems| format!("the world cannot be made:\n{problems}"))?;
    let (def, entry) = scenario_of(loaded, scenario)?;
    let seed = def.world().map_or(answers.seed, |world| {
        world.seed.choose(answers.seed, answers.drawn)
    });
    let made = make_world(defs, Arc::clone(&tables.world), scenario, seed)
        .map_err(|reason| format!("the world of `{scenario}` could not be made: {reason}"))?;
    let generators = clash_gen::generators_with(&tables.generation);
    let mut sim = Sim::from_parts(Arc::clone(defs), made.cosmos, made.timeline, seed)
        .with_io(generators, Box::new(MemoryStore::new()));
    let player = spawn_player(&mut sim, spec)?;
    sim.submit(Command::BeginScenario(BeginScenario {
        scenario: scenario.to_owned(),
        party: vec![player],
        place: answers.place,
        options: answers.options.clone(),
    }))
    .map_err(|refusal| format!("the game could not begin: {refusal}"))?;
    if sim.truth().location(player).is_none() {
        return Err("the game could not begin: the character was not put in the world".to_owned());
    }
    let mut session = session_of(sim, player, name, def, entry);
    session.seed = seed;
    Ok(session)
}

/// Carries on a game read back from a save: the player's character is
/// the one the game's party was last led by, or the one the save names,
/// or else the first directly controlled entity of the simulation. The
/// view model is built afresh from what the party remembers and sees,
/// and nothing is played back.
pub fn resume(loaded: &Loaded, saved: SavedGame) -> Result<Session, String> {
    if saved.meta.scenario.is_empty() {
        return Err(
            "the save does not say how its game began: it was not made by this application"
                .to_owned(),
        );
    }
    let (def, entry) = scenario_of(loaded, &saved.meta.scenario).map_err(|reason| {
        format!("the save's game began from a scenario the loaded mods do not define: {reason}")
    })?;
    let game = saved.sim.scenario().game.as_ref();
    let lead = game.and_then(|game| game.party.last().copied());
    let ended = game.and_then(|game| game.ended.clone());
    let direct = |player: &EntityId| {
        saved
            .sim
            .truth()
            .entity(*player)
            .is_some_and(|entity| entity.control() == Control::Direct)
    };
    let player = lead
        .filter(direct)
        .or_else(|| saved.meta.player.filter(direct))
        .or_else(|| {
            saved
                .sim
                .truth()
                .entities()
                .iter()
                .find(|entity| entity.control() == Control::Direct)
                .map(clash_sim::KernelEntity::id)
        })
        .ok_or("the save holds no character that is the player's to play")?;
    let mut session = session_of(saved.sim, player, &saved.meta.name, def, entry);
    session.seed = saved.meta.seed;
    session.ended = ended;
    // Frames the simulation held when it was saved are not shown: the
    // picture is built from the map memory.
    let _ = clash_sim::perception::drain_frames(&mut session.sim);
    Ok(session)
}

/// How long a game has run, as a load screen says it: `2:05:09`, hours,
/// minutes and seconds.
pub fn game_time(played: clash_core::Dur) -> String {
    let seconds = played.as_micros().max(0) / 1_000_000;
    format!(
        "{}:{:02}:{:02}",
        seconds / 3_600,
        seconds % 3_600 / 60,
        seconds % 60
    )
}

impl Session {
    /// The name a save is given when the player gives none: the
    /// character's, or else the scenario's.
    pub fn default_save_name(&self) -> String {
        if self.name.trim().is_empty() {
            self.scenario.name.clone()
        } else {
            self.name.clone()
        }
    }

    /// What the application writes beside the simulation when it saves
    /// this game.
    pub fn save_meta(&self) -> SaveMeta {
        let who = if self.name.trim().is_empty() {
            "Someone"
        } else {
            self.name.trim()
        };
        SaveMeta {
            scenario: self.scenario.id.clone(),
            name: self.name.clone(),
            player: Some(self.player),
            seed: self.seed,
            summary: format!(
                "{who}, {}, {} in",
                self.scenario.name,
                game_time(clash_sim::scenario::played(&self.sim))
            ),
        }
    }
}

/// The value that stands for the player's character in expressions.
pub fn player_value(session: &Session) -> clash_expr::Value {
    session.sim.subject_value(Subject::Entity(session.player))
}
