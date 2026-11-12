//! A session: one game being played (13 §4, 03 §5.1).
//!
//! [`new_game`] makes the simulation a game runs in: a cosmos with the
//! start's realm, the generators the loaded definitions give and a
//! tract store, then the player's character from its templates
//! ([`spawn_player`]), its session at the start position and direct
//! control. Everything after the simulation exists is done to it by
//! command (`Sim::submit`).
//!
//! # Seams
//!
//! | Seam | Stands in for | Replaced by |
//! |------|---------------|-------------|
//! | [`StartDef`] | A game scenario (13 §3) | The `scenario` definition: its `worlds` and `start` |
//! | [`Session::percepts`] | Each character's percepts and memory (15) | Perception ([`crate::perception_shim`]) |

use std::sync::Arc;

use clash_core::EntityId;
use clash_scene::{AppEntry, GlyphDef, StartDef};
use clash_sim::mind::VitalState;
use clash_sim::template::CharacterSpec;
use clash_sim::{Command, Control, DebugCommand, Place, Sim, Subject};
use clash_time::Timeline;
use clash_world::defs::RealmDef;
use clash_world::{CellPos, Cosmos, GridId, Location, MemoryStore, MoveProfile, MoveRules};

use crate::boot::Loaded;
use crate::perception_shim::Percepts;
use crate::saves::SavedGame;

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
    /// The start the game began from: `app.scenario`.
    pub start: AppEntry,
    /// How the character is drawn: the glyph of its templates, or else
    /// the start's. `None` if neither gives one: it is then drawn as a
    /// presence.
    pub look: Option<GlyphDef>,
    /// What the character perceives and remembers
    /// ([`crate::perception_shim`]).
    pub percepts: Percepts,
    /// Whether the simulation has moved since the character last looked.
    pub stale_view: bool,
    /// Whether the play screen has been on show for this game. Once it
    /// has, the game is over when it no longer is.
    pub shown: bool,
    /// Whether the last feed of the view was the omniscient one.
    pub omniscient: bool,
}

impl Session {
    /// The grid and cell the player's character is in, if it is in the
    /// world.
    pub fn player_loc(&self) -> Option<Location> {
        self.sim.truth().location(self.player)
    }

    /// Submits a debug command, turning a refusal into its reason.
    pub(crate) fn debug(&mut self, command: DebugCommand) -> Result<(), String> {
        self.sim
            .submit(Command::Debug(command))
            .map_err(|refusal| refusal.to_string())
    }

    /// Whether the player is waiting on the simulation: no decision is
    /// asked of it, and it is busy (an action, a wait, a plan) or
    /// unconscious, so that time must pass before it is asked. Not while
    /// it is free, dead or out of the world: nothing would ever ask it.
    pub fn waits_on_simulation(&self) -> bool {
        let sim = &self.sim;
        if sim.pending_decision().is_some() || self.player_loc().is_none() {
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

/// The highest cell of a column a walker can stand in, looking down
/// from the top of the grid. `None` if there is none.
fn standing_cell(cosmos: &Cosmos, grid: GridId, x: i32, y: i32) -> Option<Location> {
    let topology = cosmos.topology(grid).ok()?;
    let walker = MoveProfile::walker();
    (topology.z_min..=topology.z_max).rev().find_map(|z| {
        let loc = Location::new(grid, CellPos::new(x, y, z));
        let here = cosmos.neighbours(loc).ok()?.here;
        walker.can_occupy(&here).then_some(loc)
    })
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

/// The glyph a character is drawn with: its templates' (06 §10.4), in
/// the interface's form, if they give one that reads as one.
fn template_look(sim: &Sim, entity: EntityId) -> Option<GlyphDef> {
    let glyph = sim.glyph(entity)?;
    let mut written = serde_json::Map::new();
    written.insert("ch".to_owned(), serde_json::Value::from(glyph.ch.clone()));
    if let Some(fg) = &glyph.fg {
        written.insert("fg".to_owned(), serde_json::Value::from(fg.clone()));
    }
    if let Some(bg) = &glyph.bg {
        written.insert("bg".to_owned(), serde_json::Value::from(bg.clone()));
    }
    serde_json::from_value(serde_json::Value::Object(written)).ok()
}

/// What a start says, for a session.
struct StartOf<'a> {
    def: &'a StartDef,
    entry: AppEntry,
}

/// Finds a start by its qualified id.
fn start_of<'a>(loaded: &'a Loaded, start: &str) -> Result<StartOf<'a>, String> {
    let starts = loaded
        .defs
        .try_registry::<StartDef>()
        .ok_or("the loaded mods define no way to start a game")?;
    let id = starts
        .lookup(start)
        .ok_or_else(|| format!("there is no start `{start}`"))?;
    let def = starts.get(id);
    Ok(StartOf {
        def,
        entry: AppEntry {
            id: start.to_owned(),
            name: if def.name.is_empty() {
                start.to_owned()
            } else {
                def.name.clone()
            },
            description: def.doc.clone(),
            state: "current".to_owned(),
        },
    })
}

/// The qualified id of the template a start makes its character from by
/// default, if it names one.
pub fn start_template(loaded: &Loaded, start: &str) -> Result<Option<String>, String> {
    let found = start_of(loaded, start)?;
    let namespace = start.split_once(':').map_or("", |(namespace, _)| namespace);
    Ok(found.def.template_id(namespace))
}

/// A session round a simulation that has its player.
fn session_of(sim: Sim, player: EntityId, name: &str, start: StartOf<'_>) -> Session {
    let look = template_look(&sim, player).or_else(|| start.def.glyph.clone());
    Session {
        sim,
        player,
        name: name.to_owned(),
        start: start.entry,
        look,
        percepts: Percepts::default(),
        stale_view: true,
        shown: false,
        omniscient: false,
    }
}

/// Starts a game from a start (the stand-in for a game scenario): makes
/// the world, the player's character from `spec` and its session.
///
/// `start` is the qualified id of a `start` definition. The realm is
/// made with `seed`. `name` is what the player called the character.
/// Whatever cannot be done is returned as a sentence to show the
/// player.
pub fn new_game(
    loaded: &Loaded,
    start: &str,
    seed: u64,
    spec: CharacterSpec,
    name: &str,
) -> Result<Session, String> {
    let defs = &loaded.defs;
    let tables = loaded
        .tables
        .as_ref()
        .map_err(|problems| format!("the world cannot be made:\n{problems}"))?;
    let found = start_of(loaded, start)?;
    let def = found.def;
    let namespace = start.split_once(':').map_or("", |(namespace, _)| namespace);
    let realm_name = def.realm_id(namespace);
    let realm_def = defs.lookup::<RealmDef>(&realm_name).ok_or_else(|| {
        format!("the start `{start}` begins in a realm `{realm_name}` that no mod defines")
    })?;

    // The world: one realm, made from its definition with the seed.
    let mut timeline = Timeline::new();
    let mut cosmos = Cosmos::new(Arc::clone(&tables.world));
    let realm = cosmos
        .create_realm(&mut timeline, realm_def, seed)
        .map_err(|error| format!("the realm `{realm_name}` could not be made: {error}"))?;
    let grid = cosmos
        .realm(realm)
        .map(|realm| realm.base_grid)
        .ok_or("the realm was not made")?;
    let generators = clash_gen::generators_with(&tables.generation);
    let mut sim = Sim::from_parts(Arc::clone(defs), cosmos, timeline, seed)
        .with_io(generators, Box::new(MemoryStore::new()));

    let (x, y) = match def.at.as_slice() {
        [x, y] | [x, y, _] => (*x, *y),
        _ => return Err(format!("the start `{start}` does not say where it begins")),
    };
    let player = spawn_player(&mut sim, spec)?;
    let mut session = session_of(sim, player, name, found);

    // The session: the tracts round the start are loaded and the
    // character is put there. With no storey given it begins at the top
    // of the grid and is then set down where a walker can stand.
    let top = session
        .sim
        .truth()
        .cosmos()
        .topology(grid)
        .map(|topology| topology.z_max)
        .map_err(|error| error.to_string())?;
    let z = def.at.get(2).copied().unwrap_or(top);
    let begin = Location::new(grid, CellPos::new(x, y, z));
    session
        .debug(DebugCommand::StartSession {
            entity: player,
            at: begin,
        })
        .map_err(|reason| format!("the game could not begin at {begin}: {reason}"))?;
    if session.player_loc().is_none() {
        // The session's failure was reported as a fault, not a refusal.
        let fault = session
            .sim
            .undrained_events()
            .iter()
            .rev()
            .find_map(|event| match &event.body {
                clash_sim::EventBody::Fault { reason } => Some(reason.to_string()),
                _ => None,
            })
            .unwrap_or_else(|| "its tract could not be loaded".to_owned());
        return Err(format!("the game could not begin at {begin}: {fault}"));
    }
    if def.at.len() == 2 {
        let stand = standing_cell(session.sim.truth().cosmos(), grid, x, y).ok_or_else(|| {
            format!("the start `{start}` begins at {x}, {y}, where there is nothing to stand on")
        })?;
        session.debug(DebugCommand::MoveEntity {
            entity: player,
            to: Place::at(stand),
        })?;
    }
    session.debug(DebugCommand::SetControl {
        entity: player,
        control: Control::Direct,
    })?;
    Ok(session)
}

/// Carries on a game read back from a save: the player's character is
/// the first directly controlled entity of the simulation.
pub fn resume(loaded: &Loaded, saved: SavedGame) -> Result<Session, String> {
    let found = start_of(loaded, &saved.meta.start)?;
    let player = saved
        .sim
        .truth()
        .entities()
        .iter()
        .find(|entity| entity.control() == Control::Direct)
        .map(clash_sim::KernelEntity::id)
        .ok_or("the save holds no character that is the player's to play")?;
    Ok(session_of(saved.sim, player, &saved.meta.name, found))
}

/// The value that stands for the player's character in expressions.
pub fn player_value(session: &Session) -> clash_expr::Value {
    session.sim.subject_value(Subject::Entity(session.player))
}
