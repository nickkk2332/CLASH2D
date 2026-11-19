//! [`GameHost`]: the console's host over the real simulation (14 §3.3).
//!
//! The console, the test-script runner and their language are built in
//! `clash_sim::console` against one trait, `ConsoleHost`. This is that
//! trait for a real [`Sim`]: it starts a simulation from a profile,
//! builds a small world, carries commands out, evaluates expressions,
//! lets time pass, saves and loads the whole game through this crate,
//! hands over the events as records, and presents a frame. The
//! test-script runner (`clash-tools scenario run`), the terminal viewer
//! (`clash-tools worldview`) and the game's console overlay all drive
//! one.
//!
//! It lives here, and not in the tools, because the application needs
//! it too and may not depend on the tools (01 §3). It is called
//! `GameHost` because `clash_sim::SimHost` is already the expression
//! language's host.
//!
//! ```no_run
//! use std::sync::Arc;
//! use clash_save::host::{GameHost, HostEnv};
//! use clash_sim::console::{HostKind, Session};
//!
//! let env = Arc::new(HostEnv::new("mods"));
//! let table = GameHost::table();
//! let mut host = GameHost::new(env);
//! let mut session = Session::new(HostKind::Viewer);
//! for line in ["use default seed=7", "gen flat size=1", "spawn creature as=a", "frame"] {
//!     let output = session.run_line(&table, &mut host, line).unwrap();
//!     println!("{}", output.text);
//! }
//! ```
//!
//! # The commands the host adds
//!
//! Beside the built-in commands of 14 §3.3, of which it carries out
//! `step`, `check`, `inspect`, `hash <subsystem>` and `size`, the host
//! registers ([`host_commands`]):
//!
//! | Command | Does | Through |
//! |---------|------|---------|
//! | `spawn <what> [at] [mass] [tag] [control]` | Makes an entity of a kind of occupant and puts it in a cell or inside an entity | `DebugCommand::CreateEntity` |
//! | `move <who> <to>` | Moves an entity to a cell or into an entity | `DebugCommand::MoveEntity` |
//! | `remove <who>` | Removes an entity for good | `DebugCommand::RemoveEntity` |
//! | `set <at> <layer> <state>` | Sets the floor, block or fixture of a cell; `none` clears it | `DebugCommand::SetElement` |
//! | `control <who> <mode>` | Changes who decides for an entity; one under direct control becomes `@self` | `DebugCommand::SetControl` |
//! | `session <who> [at]` | Starts a session: loads the tracts round a place, puts the entity there and holds its ticket | `DebugCommand::StartSession` |
//! | `damage <at> <energy> [kind] [intensity] [layer]` | Sends a packet of damage to one element of a cell: the one named, else the first there is of its block, its fixture and its floor | `WorldEdit::damage` |
//! | `field <at> <kind> [amount]` | Adds to a field in a cell, or takes from it | `WorldEdit::add_field` |
//! | `look [at]` | Moves the cursor (`@here`) and says what is in its cell | Nothing: the cursor is the console's |
//! | `begin <scenario> [place]` | Begins a game from a game scenario (13 §12): a new simulation over its world, made with the seed `use` gave where the scenario asks for one, its character made from its template with every choice at its default, begun at its place `place` (default 0). The character becomes `@self` and the cursor goes to it | `clash_sim::scenario::make_world`, `DebugCommand::Spawn`, `Command::BeginScenario` |
//! | `frame` | Prints the frame as plain text | Nothing |
//!
//! `damage` and `field` change the world through `Sim::world` with the
//! debug cause. The kernel has no `Debug` command for either yet, so
//! they are not in the input log: the session's transcript is what
//! replays them.
//!
//! # What waits for other packages
//!
//! - **Verbs and rules.** A command that is a verb, a rule used as
//!   one, `do` and `run` all arrive at one function,
//!   `GameHost::run_verb`, which carries them out through the rule
//!   runtime. What they change is not in the input log (D-687): the
//!   transcript replays it, as it does `damage` and `field`.
//! - **What is registered on a simulation.** Whatever a package sets up
//!   on a new simulation beyond what `Sim` does itself (wake-up
//!   handlers, event handlers, event types) is set up in one place,
//!   `GameHost::set_up`, which runs after `use` and after `load`.
//! - **Spawning from a template.** `spawn` takes a kind of occupant
//!   until templates are built (06).
//! - **Decisions.** When a directly controlled entity is free to act
//!   and nobody is at the keys, it waits until time has passed as far
//!   as asked, by a `Wait` command (03 §5.2). A choice a rule puts
//!   (`ask`) is not answered: time refuses to pass with
//!   [`reasons::DECISION_PENDING`].
//! - The commands of 14 §3.3 that have nothing behind them yet are
//!   refused with [`reasons::NOT_BUILT`].

mod env;
mod view;

use std::fmt::Write as _;
use std::sync::Arc;

use clash_core::codec;
use clash_core::{Dur, EntityId, Hash128, Tick};
use clash_expr::{Compiled, Context, Type, Value, compile};
use clash_gen::{CITY_GENERATOR, FLAT_GENERATOR, STANDARD_GENERATOR};
use clash_scene::frame_to_text;
use clash_sim::component::physical;
use clash_sim::console::{
    ArgValue, BoundCommand, CommandSource, CommandSpec, CommandTable, ConsoleHost, Expression,
    HostError, HostResult, Output, ParamSpec, ParamType, Record, Reference, Refusal,
};
use clash_sim::{
    Advance, AdvanceLimit, Bound, Command, Control, DebugCommand, DecisionKind, DecisionRequest,
    EntitySpec, Place, Presence, Reason, Sim, SimEvent, Subject, debug_ticket,
};
use clash_time::Timeline;
use clash_world::defs::GeneratorSpec;
use clash_world::dims::TRACT_XY;
use clash_world::{
    BlockState, Cause, CellPos, Cosmos, DamagePacket, DamageResult, FixtureState, FloorState,
    Layer, Location, MemoryStore, OccupantFilter, OccupantFlags, OccupantKind, RealmDefId,
    TractPos, WorldDefs, WorldError,
};
use serde::{Deserialize, Serialize};

use crate::game::{load_game_from, read_meta_from};
use crate::meta::SaveMeta;
use crate::pack::{save_game_bytes, unpack_store};

pub use env::HostEnv;
use env::Profile;
use view::Painter;

/// The codes of the reasons the host refuses with (14 §5), beside the
/// kernel's and the world's own, which it passes on.
pub mod reasons {
    /// No simulation is running: `use` starts one.
    pub const NO_GAME: &str = "console.no_game";
    /// The simulation has no realm yet: `gen` makes one.
    pub const NO_WORLD: &str = "console.no_world";
    /// There is no cursor: `gen` or `look` puts it somewhere.
    pub const NO_CURSOR: &str = "console.no_cursor";
    /// No character is active: `control <who> direct` makes one.
    pub const NO_ACTIVE: &str = "console.no_active";
    /// Nothing is selected.
    pub const NOTHING_SELECTED: &str = "console.nothing_selected";
    /// A name that was not given with `as=`.
    pub const UNKNOWN_NAME: &str = "console.unknown_name";
    /// A reference of a kind the host does not have, or to something of
    /// a kind that is not there.
    pub const UNKNOWN_REFERENCE: &str = "console.unknown_reference";
    /// A value that should name a place does not.
    pub const NOT_A_PLACE: &str = "console.not_a_place";
    /// A value that should name an entity does not.
    pub const NOT_AN_ENTITY: &str = "console.not_an_entity";
    /// A word that should name a kind of world, a kind of occupant, a
    /// damage type, a field or an element state does not.
    pub const UNKNOWN_KIND: &str = "console.unknown_kind";
    /// A world larger than `gen` builds.
    pub const TOO_LARGE: &str = "console.too_large";
    /// A command of 14 §3.3 that has nothing behind it yet.
    pub const NOT_BUILT: &str = "console.not_built";
    /// A verb, a rule or an effect was asked for of a host whose rule
    /// runtime is not bound to its simulation. The host over the real
    /// simulation no longer gives it.
    pub const RULES_UNBOUND: &str = "console.rules_unbound";
    /// A rule has put a choice to a directly controlled entity, and
    /// nobody is there to answer it.
    pub const DECISION_PENDING: &str = "console.decision_pending";
    /// A subsystem `hash` does not know.
    pub const UNKNOWN_SUBSYSTEM: &str = "console.unknown_subsystem";

    /// Every reason of the host, with what it means: for the generated
    /// reference.
    pub const ALL: &[(&str, &str)] = &[
        (
            NO_GAME,
            "No simulation is running. `use <profile>` starts one.",
        ),
        (
            NO_WORLD,
            "The simulation has no realm. `gen <kind>` makes one.",
        ),
        (
            NO_CURSOR,
            "`@here` was asked for and there is no cursor. `gen` and `look` put it somewhere.",
        ),
        (
            NO_ACTIVE,
            "`@self` was asked for and no character is active. `control <who> direct` makes one active.",
        ),
        (
            NOTHING_SELECTED,
            "`@sel` was asked for and nothing is selected.",
        ),
        (
            UNKNOWN_NAME,
            "A reference `@name` to a name that no command gave with `as=`. Values: `name`.",
        ),
        (
            UNKNOWN_REFERENCE,
            "A reference `@kind:name` of a kind the host does not have, or to something that is not there. Values: `kind`, `name`.",
        ),
        (
            NOT_A_PLACE,
            "An argument that should be a cell, or something that is in one, is not.",
        ),
        (
            NOT_AN_ENTITY,
            "An argument that should be an entity is not.",
        ),
        (
            UNKNOWN_KIND,
            "A word that should name a kind of world, a kind of occupant, a damage type, a field or an element state names none. Values: `what`, `name`.",
        ),
        (
            TOO_LARGE,
            "`gen` was asked for a world of more tracts a side than it builds. Values: `size`, `max`.",
        ),
        (
            NOT_BUILT,
            "The command is registered and nothing carries it out yet. Values: `command`.",
        ),
        (
            RULES_UNBOUND,
            "A verb, a rule or an effect was asked for of a host whose rule runtime is not bound to its simulation. The host over the real simulation carries them out and no longer gives this. Values: `command`.",
        ),
        (
            DECISION_PENDING,
            "Time cannot pass: a rule has put a choice to a directly controlled entity and nobody is there to answer it. An entity that is only free to act waits instead. Values: `entity`.",
        ),
        (
            UNKNOWN_SUBSYSTEM,
            "`hash` was asked for a subsystem it does not know. Values: `subsystem`.",
        ),
    ];
}

/// The most tracts along a side that `gen` loads.
pub const GEN_MAX_SIZE: u32 = 16;

/// The size of the frame until `size` says otherwise: columns and rows.
pub const DEFAULT_FRAME_SIZE: (u16, u16) = (64, 24);

/// How often `advance` goes round again when the kernel yields before
/// it gives up: content that never lets time move is contained, not
/// waited for.
const MAX_ADVANCE_ROUNDS: u32 = 10_000;

/// What `save` gives when no simulation is running.
const NO_GAME_BYTES: &[u8] = b"C2-NO-GAME";

/// The kinds of world `gen` knows by a short name, each the name of a
/// generator: the first realm definition that names the generator is
/// the one made. A realm definition's own id is always accepted.
const GEN_KINDS: &[(&str, &str)] = &[
    ("flat", FLAT_GENERATOR),
    ("city", CITY_GENERATOR),
    ("country", STANDARD_GENERATOR),
];

/// The kinds of occupant `spawn` makes, by the word that names each.
const SPAWN_KINDS: &[(&str, OccupantKind)] = &[
    ("creature", OccupantKind::Creature),
    ("item", OccupantKind::Item),
    ("device", OccupantKind::Device),
    ("other", OccupantKind::Other),
];

/// The layers of a cell, by the word that names each.
const LAYERS: &[(&str, Layer)] = &[
    ("floor", Layer::Floor),
    ("block", Layer::Block),
    ("fixture", Layer::Fixture),
];

/// The control modes, by the word that names each.
const CONTROLS: &[(&str, Control)] = &[
    ("none", Control::None),
    ("direct", Control::Direct),
    ("delegated", Control::Delegated),
];

/// The word that stands for no element in `set`.
const NO_ELEMENT: &str = "none";

/// What the console keeps beside the simulation: saved with the game,
/// not hashed.
#[derive(Clone, Debug, Serialize, Deserialize)]
struct Marks {
    /// The cursor's cell: `@here`.
    here: Option<Location>,
    /// The active character: `@self`.
    active: Option<EntityId>,
    /// The last selection: `@sel`.
    selection: Option<clash_core::Value>,
    /// The size of the frame.
    cols: u16,
    rows: u16,
}

impl Default for Marks {
    fn default() -> Marks {
        Marks {
            here: None,
            active: None,
            selection: None,
            cols: DEFAULT_FRAME_SIZE.0,
            rows: DEFAULT_FRAME_SIZE.1,
        }
    }
}

/// What the host writes into a save's metadata as its own bytes.
#[derive(Serialize, Deserialize)]
struct HostState {
    profile: String,
    seed: u64,
    marks: Marks,
}

/// A simulation the host is running.
#[derive(Debug)]
struct Game {
    profile: Profile,
    seed: u64,
    sim: Sim,
    marks: Marks,
}

/// The console's host over a real simulation. See the
/// [module documentation](self).
#[derive(Debug)]
pub struct GameHost {
    env: Arc<HostEnv>,
    threads: Option<u32>,
    game: Option<Game>,
    last_refusal: Option<Refusal>,
    painter: Option<Painter>,
}

fn refuse(code: &str, message: impl Into<String>) -> HostError {
    HostError::Refused(Refusal::new(code, message))
}

/// A reason of the simulation's as the console reports it.
fn refusal_of(reason: &Reason) -> HostError {
    let mut refusal = Refusal::new(reason.code.clone(), reason.message.clone());
    for (name, value) in &reason.values {
        refusal = refusal.with(name.clone(), Value::from(value));
    }
    HostError::Refused(refusal)
}

fn world_refusal(error: &WorldError) -> HostError {
    refusal_of(&Reason::from_world(error))
}

/// The type a value has, for declaring what holds it.
fn type_of(value: &Value) -> Type {
    match value {
        Value::Subject(handle) => Type::Subject(handle.kind),
        Value::Int(_) => Type::Int,
        Value::Real(_) => Type::Real,
        Value::Bool(_) => Type::Bool,
        Value::Str(_) => Type::Str,
        _ => Type::Any,
    }
}

fn word_of<T: Copy>(table: &[(&str, T)], word: &str) -> Option<T> {
    table
        .iter()
        .find(|(name, _)| *name == word)
        .map(|(_, found)| *found)
}

fn words<T>(table: &'static [(&'static str, T)]) -> Vec<&'static str> {
    table.iter().map(|(name, _)| *name).collect()
}

/// The commands the host registers beside the built-in ones: see the
/// [module documentation](self).
pub fn host_commands() -> Vec<CommandSpec> {
    use ParamSpec as P;
    use ParamType as T;
    let cmd = CommandSpec::new;
    let word = |text: &str| ArgValue::Word(text.to_owned());
    vec![
        cmd("spawn")
            .doc("Makes an entity and puts it somewhere. What it gives back, for `as=`, is the entity.")
            .param(P::required("what", T::Word, "The kind of occupant: creature, item, device or other. A template's id once templates are built (06)."))
            .param(P::optional("at", T::Subject, "Where: a cell, or an entity to put it inside. The cursor's cell if left out."))
            .param(P::optional("mass", T::Real, "Its mass in kilograms."))
            .param(P::optional("tag", T::Word, "A tag to give it."))
            .param(P::optional("control", T::choice(words(CONTROLS)), "Who decides for it: none, direct or delegated.").default(word("none")))
            .changes(),
        cmd("move")
            .doc("Moves an entity to a cell, or into another entity.")
            .param(P::required("who", T::Subject, "The entity."))
            .param(P::required("to", T::Subject, "Where to: a cell, or an entity to put it inside."))
            .changes(),
        cmd("remove")
            .doc("Removes an entity for good. What it held is put where it was.")
            .param(P::required("who", T::Subject, "The entity."))
            .changes(),
        cmd("set")
            .doc("Sets the floor, block or fixture of a cell.")
            .param(P::required("at", T::Subject, "The cell."))
            .param(P::required("layer", T::choice(words(LAYERS)), "Which of its elements: floor, block or fixture."))
            .param(P::required("state", T::Word, "What to put there, as data writes an element state (`core:wall`, `wall@brick`, `door#open`), or `none` for nothing."))
            .changes(),
        cmd("control")
            .doc("Changes who decides for an entity. One put under direct control becomes the active character, `@self`.")
            .param(P::required("who", T::Subject, "The entity."))
            .param(P::required("mode", T::choice(words(CONTROLS)), "none, direct or delegated."))
            .changes(),
        cmd("session")
            .doc("Starts a session for an entity at a place that need not be loaded: loads the tracts round it, puts the entity there and holds the ticket of a controlled character for it.")
            .param(P::required("who", T::Subject, "The entity."))
            .param(P::optional("at", T::Subject, "The cell. The cursor's cell if left out."))
            .changes(),
        cmd("damage")
            .doc("Sends a packet of damage to one element of a cell (02 §12): the one named, else what a blow meets first, the first there is of its block, its fixture and its floor.")
            .param(P::required("at", T::Subject, "The cell."))
            .param(P::required("energy", T::Real, "Joules delivered: `5kJ`, `1e6`."))
            .param(P::optional("kind", T::Word, "The damage type's id. The kinetic damage type if left out."))
            .param(P::optional("intensity", T::Real, "How concentrated the delivery is, in pascals. As many pascals as there are joules if left out: the energy in one cubic metre."))
            .param(P::optional("layer", T::choice(words(LAYERS)), "The one element struck: floor, block or fixture."))
            .changes(),
        cmd("field")
            .doc("Adds to a field in a cell (02 §16), or takes from it with a negative amount.")
            .param(P::required("at", T::Subject, "The cell."))
            .param(P::required("kind", T::Word, "The field's id."))
            .param(P::optional("amount", T::Int, "How many levels to add, of 255.").default(ArgValue::Value(Value::Int(255))))
            .changes(),
        cmd("look")
            .doc("Moves the cursor, `@here`, to a cell or to where an entity is, and says what is in the cursor's cell.")
            .param(P::optional("at", T::Subject, "Where to put the cursor. Left out, it stays.")),
        cmd("frame").doc("Prints the frame: the storey of the cursor from above, as plain text."),
        cmd("begin")
            .doc("Begins a game from a game scenario (13 §12): a new simulation over its world, made with the seed `use` gave where the scenario asks for one (and for one drawn at random), its character made from its template with every choice at its default, begun at one of its places. The character becomes `@self` and the cursor goes to it. What gives back, for `as=`, is the character.")
            .param(P::required("scenario", T::Word, "The scenario: its qualified id, or a bare name."))
            .param(P::optional("place", T::Int, "Which of its places, from 0.").default(ArgValue::Value(Value::Int(0))))
            .changes(),
    ]
}

impl GameHost {
    /// A host with no simulation yet: `use` starts one and `load` puts
    /// one back.
    pub fn new(env: Arc<HostEnv>) -> GameHost {
        GameHost {
            env,
            threads: None,
            game: None,
            last_refusal: None,
            painter: None,
        }
    }

    /// The commands a console over this host has: the built-in commands
    /// of 14 §3.3, the host's own ([`host_commands`]) and one for each
    /// verb of the engine's registry. A built-in command keeps its name
    /// from a verb of the same name, which is still reached with `do`.
    pub fn table() -> CommandTable {
        GameHost::table_with(&clash_sim::engine_registry())
    }

    /// As [`table`](GameHost::table), with the verbs of another
    /// registry: that of a loaded profile, which also holds the rules
    /// its mods offer as verbs.
    pub fn table_with(registry: &clash_expr::Registry) -> CommandTable {
        let mut table = CommandTable::builtin();
        for spec in host_commands() {
            // The host's registrations are checked by a test.
            let _ = table.add(spec);
        }
        table.add_verbs(registry);
        table
    }

    /// The simulation, if one is running: for a tool that reads it
    /// beside the console.
    pub fn sim(&self) -> Option<&Sim> {
        self.game.as_ref().map(|game| &game.sim)
    }

    /// The cursor's cell, `@here`, if there is a cursor.
    pub fn here(&self) -> Option<Location> {
        self.game.as_ref().and_then(|game| game.marks.here)
    }

    /// Moves the cursor: for a viewer's own keys.
    pub fn set_here(&mut self, at: Location) {
        if let Some(game) = &mut self.game {
            game.marks.here = Some(at);
        }
    }

    /// The seed the simulation was started with.
    pub fn seed(&self) -> Option<u64> {
        self.game.as_ref().map(|game| game.seed)
    }

    /// Saves the game into a file ([`crate::save_game`]), with a name
    /// and a summary for a load screen and the console's own marks (the
    /// cursor, the active character, the selection) beside it, so that
    /// [`load_from`](GameHost::load_from) puts the console back as it
    /// was.
    pub fn save_to(
        &mut self,
        path: impl AsRef<std::path::Path>,
        name: &str,
        summary: &str,
    ) -> HostResult<crate::SaveReport> {
        let game = self.game_mut()?;
        let meta = game.meta(name, summary)?;
        crate::save_game(&mut game.sim, path, &meta)
            .map_err(|error| HostError::failed(format!("could not save: {error}")))
    }

    /// Loads a game from a file that [`save_to`](GameHost::save_to)
    /// wrote, with the profile it names, and gives back its metadata.
    /// A save made by something other than the console loads too, with
    /// the profile its metadata names and no marks.
    pub fn load_from(&mut self, path: impl AsRef<std::path::Path>) -> HostResult<SaveMeta> {
        let failed = |error: &dyn std::fmt::Display| {
            HostError::failed(format!("the save does not load: {error}"))
        };
        let mut store =
            crate::game::open_existing(path.as_ref()).map_err(|error| failed(&error))?;
        self.load_store(&mut store)
    }

    /// Loads a game out of a store, with the profile its metadata names.
    fn load_store(&mut self, store: &mut dyn clash_world::TractStore) -> HostResult<SaveMeta> {
        let failed = |error: &dyn std::fmt::Display| {
            HostError::failed(format!("the save does not load: {error}"))
        };
        let meta = read_meta_from(store).map_err(|error| failed(&error))?;
        let state: HostState = codec::from_bytes(&meta.app).unwrap_or_else(|_| HostState {
            profile: meta.profile.clone(),
            seed: meta.seed,
            marks: Marks::default(),
        });
        let profile = self
            .env
            .profile(&state.profile)
            .map_err(|error| failed(&error))?;
        let generators = profile
            .generators(self.threads)
            .map_err(|error| failed(&error))?;
        let (sim, _) = load_game_from(store, Arc::clone(&profile.defs), generators)
            .map_err(|error| failed(&error))?;
        self.game = Some(Game {
            profile,
            seed: state.seed,
            sim: GameHost::set_up(sim),
            marks: state.marks,
        });
        Ok(meta)
    }

    /// Whatever a package sets up on a simulation beyond what `Sim`
    /// does itself: wake-up handlers, event handlers and event types.
    /// It runs on every simulation the host makes, new or loaded, so
    /// that both are set up alike (03 §4.1).
    ///
    /// Nothing needs it yet: every handler built so far is among the
    /// defaults of `Sim`.
    fn set_up(sim: Sim) -> Sim {
        sim
    }

    /// Carries out a verb (05 §10), a rule used as one, `do` and `run`:
    /// everything that reaches the rule runtime (14 §3.3).
    ///
    /// A verb of the engine is called with its arguments by name and
    /// `self` bound to the `as` subject, else the active character, and
    /// gives back what it gives, for `as=`. A rule used as a verb, `do`
    /// and `run` are compiled as a list of one effect against the
    /// loaded definitions (`Sim::compile_effects`), in a scope that
    /// binds `self`, `here` (the cursor) and `global`, and run through
    /// `Sim::run_effects`. What they change is caused by the debug
    /// cause, and is not in the input log: the transcript replays it
    /// (D-585, D-687).
    fn run_verb(&mut self, command: &BoundCommand) -> HostResult<Output> {
        self.game_mut()?.run_rules(command)
    }

    fn game(&self) -> HostResult<&Game> {
        self.game.as_ref().ok_or_else(|| {
            refuse(
                reasons::NO_GAME,
                "no simulation is running; `use <profile>` starts one",
            )
        })
    }

    fn game_mut(&mut self) -> HostResult<&mut Game> {
        self.game.as_mut().ok_or_else(|| {
            refuse(
                reasons::NO_GAME,
                "no simulation is running; `use <profile>` starts one",
            )
        })
    }

    /// Notes whether a request was refused, for `last_refusal`.
    fn noted<T>(&mut self, result: HostResult<T>) -> HostResult<T> {
        self.last_refusal = match &result {
            Err(HostError::Refused(refusal)) => Some(refusal.clone()),
            _ => None,
        };
        result
    }

    /// The frame as rows of plain characters, under a line that says
    /// where the cursor is and what time it is: what `frame` prints.
    pub fn frame_plain(&mut self) -> HostResult<String> {
        let frame = self.paint()?;
        let game = self.game()?;
        let mut out = String::new();
        if let Some(here) = game.marks.here {
            let _ = writeln!(
                out,
                "@c:{},{},{}  t={}",
                here.cell.x,
                here.cell.y,
                here.cell.z,
                game.sim.now().since(Tick::ZERO)
            );
        }
        out.push_str(&frame.text());
        Ok(out)
    }

    /// The painter for the running game's definitions, made the first
    /// time it is wanted.
    fn painter(&mut self) -> HostResult<(&Game, &Painter)> {
        let game = self.game.as_ref().ok_or_else(|| {
            refuse(
                reasons::NO_GAME,
                "no simulation is running; `use <profile>` starts one",
            )
        })?;
        let world = game.sim.truth().cosmos().defs();
        if !self
            .painter
            .as_ref()
            .is_some_and(|painter| painter.is_for(world))
        {
            self.painter =
                Some(Painter::new(&game.profile.defs, world).map_err(HostError::Failed)?);
        }
        let Some(painter) = &self.painter else {
            return Err(HostError::failed("the frame could not be set up"));
        };
        Ok((game, painter))
    }

    fn paint(&mut self) -> HostResult<clash_scene::Frame> {
        let (game, painter) = self.painter()?;
        let here = game.here()?;
        painter
            .paint(&game.sim, here, game.marks.cols, game.marks.rows)
            .map_err(HostError::Failed)
    }

    /// Draws the storey of the cursor from above into an area of a
    /// canvas, the cursor in the middle, as `frame` does: for a viewer
    /// that draws its own lines round the map.
    pub fn draw_map(
        &mut self,
        canvas: &mut clash_scene::Canvas<'_>,
        area: clash_scene::Rect,
    ) -> HostResult<()> {
        let (game, painter) = self.painter()?;
        let here = game.here()?;
        painter
            .paint_into(&game.sim, here, canvas, area)
            .map_err(HostError::Failed)
    }

    fn carry_out(&mut self, command: &BoundCommand) -> HostResult<Output> {
        match &command.source {
            CommandSource::Verb | CommandSource::Rule(_) => return self.run_verb(command),
            CommandSource::Builtin => {}
        }
        match command.name.as_str() {
            "do" | "run" => self.run_verb(command),
            "frame" => Ok(Output::text(self.frame_plain()?)),
            "spawn" => self.game_mut()?.spawn(command),
            "move" => self.game_mut()?.move_entity(command),
            "remove" => self.game_mut()?.remove(command),
            "set" => self.game_mut()?.set_element(command),
            "control" => self.game_mut()?.control(command),
            "session" => self.game_mut()?.session(command),
            "damage" => self.game_mut()?.damage(command),
            "field" => self.game_mut()?.field(command),
            "look" => self.game_mut()?.look(command),
            "begin" => {
                let threads = self.threads;
                self.game_mut()?.begin(command, threads)
            }
            "step" => self.game_mut()?.step(command),
            "check" => self.game()?.check(),
            "inspect" => self.game_mut()?.inspect(command),
            "hash" => self.game()?.hash(command),
            "size" => self.game_mut()?.size(command),
            other => Err(HostError::Refused(
                Refusal::new(
                    reasons::NOT_BUILT,
                    format!("`{other}` is registered and nothing carries it out yet"),
                )
                .with("command", other),
            )),
        }
    }
}

/// The realm definition a kind of world names: a definition's own id,
/// qualified or bare, or one of [`GEN_KINDS`].
fn realm_for(world: &WorldDefs, kind: &str) -> HostResult<RealmDefId> {
    let realms = world.realms();
    if let Some(found) = realms.lookup(kind).or_else(|| realms.resolve(kind, None)) {
        return Ok(found);
    }
    let by_generator = word_of(GEN_KINDS, kind).and_then(|generator| {
        realms
            .iter()
            .find(|(_, realm)| {
                matches!(&realm.generator, Some(GeneratorSpec::Named(name)) if name.0 == generator)
            })
            .map(|(id, _)| id)
    });
    by_generator.ok_or_else(|| {
        let known: Vec<&str> = GEN_KINDS.iter().map(|(name, _)| *name).collect();
        let ids: Vec<&str> = realms.iter().map(|(id, _)| realms.name(id)).collect();
        HostError::Refused(
            Refusal::new(
                reasons::UNKNOWN_KIND,
                format!(
                    "there is no kind of world `{kind}`: it is one of {}, or a realm definition's id ({})",
                    known.join(", "),
                    ids.join(", ")
                ),
            )
            .with("what", "world")
            .with("name", kind),
        )
    })
}

/// The highest cell of a column that can be stood in, or the cell at
/// storey zero if none can.
fn standing_cell(cosmos: &Cosmos, grid: clash_world::GridId, x: i32, y: i32) -> Location {
    let (low, high) = cosmos
        .topology(grid)
        .map_or((0, 0), |topology| (topology.z_min, topology.z_max));
    (low..=high)
        .rev()
        .map(|z| Location::new(grid, CellPos::new(x, y, z)))
        .find(|loc| cosmos.standable(*loc).unwrap_or(false))
        .unwrap_or_else(|| Location::new(grid, CellPos::new(x, y, 0)))
}

/// An event as the console's expectations read it: its name, its tick,
/// its subjects and its fields by name.
fn record_of(sim: &Sim, event: &SimEvent) -> Record {
    let mut record = Record::event(event.name(), event.tick);
    for (name, subject) in event.subjects() {
        record = record.with(name, sim.subject_value(subject));
    }
    if let Some(ty) = sim.event_type(event.name()) {
        for field in &ty.fields {
            if let Some(value) = event.body.field(&field.name) {
                record = record.with(field.name.as_str(), sim.exprs().field_value(&value));
            }
        }
    }
    record
}

impl Game {
    /// A new simulation over a profile, with an empty world.
    fn start(profile: Profile, seed: u64, threads: Option<u32>) -> HostResult<Game> {
        let generators = profile.generators(threads).map_err(HostError::Failed)?;
        let sim = Sim::from_parts(
            Arc::clone(&profile.defs),
            Cosmos::new(Arc::clone(&profile.world)),
            Timeline::new(),
            seed,
        )
        .with_io(generators, Box::new(MemoryStore::new()));
        Ok(Game {
            profile,
            seed,
            sim: GameHost::set_up(sim),
            marks: Marks::default(),
        })
    }

    /// The metadata of a save of this game: a name and a summary, the
    /// seed, and the console's own state as the saver's bytes.
    fn meta(&self, name: &str, summary: &str) -> HostResult<SaveMeta> {
        let state = HostState {
            profile: self.profile.name.clone(),
            seed: self.seed,
            marks: self.marks.clone(),
        };
        let own = codec::to_bytes(&state)
            .map_err(|error| HostError::failed(format!("could not save: {error}")))?;
        Ok(SaveMeta::new(name)
            .summary(summary)
            .seed(self.seed)
            .app(own))
    }

    fn here(&self) -> HostResult<Location> {
        self.marks.here.ok_or_else(|| {
            refuse(
                reasons::NO_CURSOR,
                "there is no cursor; `gen` builds a world and puts it there, and `look <at>` moves it",
            )
        })
    }

    fn value(&self, subject: Subject) -> HostResult<Value> {
        match self.sim.subject_value(subject) {
            Value::Null => Err(refuse(
                reasons::NOT_A_PLACE,
                "that is beyond what a reference can name",
            )),
            value => Ok(value),
        }
    }

    /// The cell a value names: a cell, an element's cell, or where an
    /// entity is.
    fn loc_of(&self, value: &Value) -> HostResult<Location> {
        match self.sim.subject_of(value) {
            Some(Subject::Loc(loc) | Subject::Element(loc, _)) => Ok(loc),
            Some(Subject::Entity(id)) => self.sim.truth().location(id).ok_or_else(|| {
                refuse(
                    reasons::NOT_A_PLACE,
                    format!("entity {id} is not in the world"),
                )
            }),
            _ => Err(refuse(
                reasons::NOT_A_PLACE,
                format!("`{value}` is not a cell, nor something that is in one"),
            )),
        }
    }

    fn entity_of(&self, value: &Value) -> HostResult<EntityId> {
        match self.sim.subject_of(value) {
            Some(Subject::Entity(id)) => Ok(id),
            _ => Err(refuse(
                reasons::NOT_AN_ENTITY,
                format!("`{value}` is not an entity"),
            )),
        }
    }

    /// Where a value says to put something: at a cell, or inside an
    /// entity.
    fn place_of(&self, value: &Value) -> HostResult<Place> {
        match self.sim.subject_of(value) {
            Some(Subject::Entity(id)) => Ok(Place::In(id)),
            _ => self.loc_of(value).map(Place::at),
        }
    }

    /// The value of an argument, evaluating it if it is an expression.
    fn arg(&self, command: &BoundCommand, name: &str) -> HostResult<Option<Value>> {
        let own = match &command.subject {
            Some(ArgValue::Value(value)) => Some(value),
            _ => None,
        };
        Ok(match command.arg(name) {
            None => None,
            Some(ArgValue::Value(value)) => Some(value.clone()),
            Some(ArgValue::Expr(expression)) => Some(self.eval(expression, own)?),
            Some(ArgValue::Word(word)) => Some(Value::text(word)),
            Some(ArgValue::Dur(time)) => Some(Value::Real(time.as_secs_real())),
            Some(other) => {
                return Err(HostError::failed(format!(
                    "`{name}` cannot be `{}`",
                    other.shown()
                )));
            }
        })
    }

    fn required(&self, command: &BoundCommand, name: &str) -> HostResult<Value> {
        self.arg(command, name)?
            .ok_or_else(|| HostError::failed(format!("`{name}` has no value")))
    }

    /// The cell an argument names, or the cursor's if it was left out.
    fn loc_arg(&self, command: &BoundCommand, name: &str) -> HostResult<Location> {
        match self.arg(command, name)? {
            Some(value) => self.loc_of(&value),
            None => self.here(),
        }
    }

    fn entity_arg(&self, command: &BoundCommand, name: &str) -> HostResult<EntityId> {
        self.entity_of(&self.required(command, name)?)
    }

    fn word_arg<T: Copy>(
        &self,
        command: &BoundCommand,
        name: &str,
        table: &[(&str, T)],
        what: &str,
    ) -> HostResult<Option<T>> {
        let Some(word) = command.word(name) else {
            return Ok(None);
        };
        word_of(table, word).map(Some).ok_or_else(|| {
            let known: Vec<&str> = table.iter().map(|(name, _)| *name).collect();
            HostError::Refused(
                Refusal::new(
                    reasons::UNKNOWN_KIND,
                    format!(
                        "there is no {what} `{word}`: it is one of {}",
                        known.join(", ")
                    ),
                )
                .with("what", what)
                .with("name", word),
            )
        })
    }

    /// Submits a debug command (03 §4.4): validated, logged and
    /// replayed like any other.
    fn submit(&mut self, command: DebugCommand) -> HostResult<()> {
        self.sim
            .submit(Command::Debug(command))
            .map_err(|error| refusal_of(&error.reason))
    }

    /// Brings the simulation to a safe point after the host changed
    /// the world through `Sim::world`: what the world reported is
    /// delivered and tickets are reconciled, as after a command.
    fn settle(&mut self) {
        self.sim.pump();
        self.sim.reconcile_activity();
        self.sim.pump();
    }

    /// What `self` is for a command that reaches the rule runtime: the
    /// `as` subject, else the active character, else nothing.
    fn rule_self(&self, command: &BoundCommand) -> Value {
        match &command.subject {
            Some(ArgValue::Value(value)) => value.clone(),
            _ => self
                .marks
                .active
                .filter(|id| self.sim.truth().entities().contains(*id))
                .map_or(Value::Null, |id| {
                    self.sim.subject_value(Subject::Entity(id))
                }),
        }
    }

    /// The value of a verb's argument, of the type it declares: a word
    /// where an id is wanted names the definition.
    fn verb_value(&self, command: &BoundCommand, name: &str, ty: &Type) -> HostResult<Value> {
        if let (Type::Id(def), Some(ArgValue::Word(word))) = (ty.non_null(), command.arg(name)) {
            let registry = self.sim.defs().expr_registry();
            return registry
                .def_ref(*def, word)
                .or_else(|| registry.def_ref(*def, &format!("core:{word}")))
                .map(Value::Id)
                .ok_or_else(|| {
                    HostError::Refused(
                        Refusal::new(reasons::UNKNOWN_NAME, format!("there is no `{word}`"))
                            .with("name", word.as_str()),
                    )
                });
        }
        Ok(self.arg(command, name)?.unwrap_or(Value::Null))
    }

    /// Carries out what reaches the rule runtime: see
    /// [`GameHost::run_verb`].
    fn run_rules(&mut self, command: &BoundCommand) -> HostResult<Output> {
        let own = self.rule_self(command);
        let here = self
            .marks
            .here
            .map_or(Value::Null, |loc| self.sim.subject_value(Subject::Loc(loc)));
        let global = self.sim.subject_value(Subject::Global);
        let registry = Arc::clone(self.sim.defs().expr_registry());
        let refused = |reason: Reason| refusal_of(&reason);
        // A verb of the engine: called directly, for what it gives. One
        // the simulation does not carry out is refused there, with
        // `rule.verb_unbound`.
        if command.source == CommandSource::Verb
            && self.sim.rule_set().lookup(&command.name).is_none()
        {
            let params = registry
                .verb_id(&command.name)
                .and_then(|id| registry.verb(id))
                .map(|verb| verb.spec.args.clone())
                .unwrap_or_default();
            let mut args: Vec<(String, Value)> = Vec::new();
            for param in &params {
                let value = self.verb_value(command, &param.name, &param.ty)?;
                args.push((param.name.clone(), value));
            }
            let args: Vec<(&str, Value)> = args
                .iter()
                .map(|(name, value)| (name.as_str(), value.clone()))
                .collect();
            let given = self
                .sim
                .call_verb(&command.name, &args, &[("self", own)], Cause::DEBUG)
                .map_err(refused);
            self.settle();
            let given = given?;
            let mut output = Output::text(if given.is_null() {
                format!("`{}` done", command.name)
            } else {
                format!("`{}` gave {given}", command.name)
            });
            if let Value::Subject(_) = given {
                output = output.with_subject(given);
            }
            return Ok(output);
        }
        // Everything else is a list of one effect.
        let mut locals: Vec<(String, Value)> = Vec::new();
        let effect: serde_json::Value = match command.name.as_str() {
            "do" => match command.arg("effect") {
                Some(ArgValue::Json(effect)) => effect.clone(),
                _ => return Err(HostError::failed("`do` takes an effect, written as JSON")),
            },
            "run" => {
                let rule = command
                    .word("rule")
                    .ok_or_else(|| HostError::failed("`run` takes the id of a rule"))?;
                match command.arg("with") {
                    Some(ArgValue::Json(with)) => {
                        serde_json::json!({ "run": { "eoc": rule, "with": with } })
                    }
                    _ => serde_json::json!({ "run": rule }),
                }
            }
            name => {
                // A rule used as a verb: its arguments as locals.
                let spec = registry
                    .verb_id(name)
                    .and_then(|id| registry.verb(id))
                    .map(|verb| verb.spec.clone());
                let mut with = serde_json::Map::new();
                for param in spec.iter().flat_map(|spec| spec.args.iter()) {
                    let value = self.verb_value(command, &param.name, &param.ty)?;
                    if value.is_null() {
                        continue;
                    }
                    let local = format!("a{}", locals.len());
                    with.insert(
                        param.name.clone(),
                        serde_json::Value::String(format!("${local}")),
                    );
                    locals.push((local, value));
                }
                let mut object = serde_json::Map::new();
                object.insert(name.to_owned(), serde_json::Value::Object(with));
                serde_json::Value::Object(object)
            }
        };
        let json = serde_json::Value::Array(vec![effect]).to_string();
        let own_type = match &own {
            Value::Null => Type::Any,
            other => type_of(other),
        };
        let subjects = [
            ("self", own_type),
            (
                "here",
                if here.is_null() {
                    Type::Any
                } else {
                    type_of(&here)
                },
            ),
            ("global", type_of(&global)),
        ];
        let typed: Vec<(&str, Type)> = locals
            .iter()
            .map(|(name, value)| (name.as_str(), type_of(value)))
            .collect();
        let effects = self
            .sim
            .compile_effects(&json, &subjects, &typed)
            .map_err(|errors| {
                HostError::failed(format!("the effect does not compile: {errors}"))
            })?;
        let mut values: Vec<Value> = locals.into_iter().map(|(_, value)| value).collect();
        let outcome = self.sim.run_effects(
            &effects,
            &[own, here, global],
            &mut values,
            None,
            Cause::DEBUG,
        );
        self.settle();
        Ok(Output::text(if outcome.aborted {
            "the run was ended by a limit".to_owned()
        } else if outcome.value.is_null() {
            String::new()
        } else {
            format!("gave {}", outcome.value)
        }))
    }

    fn select(&mut self, value: &Value) {
        self.marks.selection = Some(value.to_stored());
    }

    // ---- Setting up ------------------------------------------------------

    fn generate(&mut self, kind: &str, size: u32) -> HostResult<Output> {
        if size > GEN_MAX_SIZE {
            return Err(HostError::Refused(
                Refusal::new(
                    reasons::TOO_LARGE,
                    format!(
                        "`gen` builds at most {GEN_MAX_SIZE} tracts a side, and {size} is more"
                    ),
                )
                .with("size", i64::from(size))
                .with("max", i64::from(GEN_MAX_SIZE)),
            ));
        }
        let def = realm_for(&self.profile.world, kind)?;
        let name = self.profile.world.realms().name(def).to_owned();
        // Each realm of a simulation draws from a seed of its own.
        let made = self.sim.truth().cosmos().realms().count() as u64;
        let seed = self.seed.wrapping_add(made);
        let realm = self
            .sim
            .world(|ctx| ctx.cosmos.create_realm(ctx.timeline, def, seed))
            .map_err(|error| world_refusal(&error))?;
        let side = i32::try_from(size).unwrap_or(1);
        for y in 0..side {
            for x in 0..side {
                self.sim
                    .hold_ticket(debug_ticket(realm, TractPos::new(x, y), 0))
                    .map_err(|reason| refusal_of(&reason))?;
            }
        }
        self.settle();
        let cosmos = self.sim.truth().cosmos();
        let loaded = cosmos.loaded_tracts(realm).len();
        let wanted = (side * side) as usize;
        if loaded < wanted {
            let faults: Vec<String> = self
                .sim
                .undrained_events()
                .iter()
                .filter(|event| event.name() == "fault")
                .filter_map(|event| event.body.field("message"))
                .map(|message| self.sim.exprs().field_value(&message).to_string())
                .collect();
            return Err(HostError::failed(format!(
                "only {loaded} of {wanted} tracts of `{name}` could be loaded{}{}",
                if faults.is_empty() { "" } else { ": " },
                faults.join("; ")
            )));
        }
        let grid = cosmos
            .realm(realm)
            .map(|realm| realm.base_grid)
            .ok_or_else(|| HostError::failed("the realm was not made"))?;
        let middle = side * TRACT_XY / 2;
        let here = standing_cell(cosmos, grid, middle, middle);
        self.marks.here = Some(here);
        let subject = self.value(Subject::Realm(realm))?;
        Ok(Output::text(format!(
            "made a realm of `{name}` and loaded {loaded} tracts; the cursor is at @c:{},{},{}",
            here.cell.x, here.cell.y, here.cell.z
        ))
        .with_subject(subject))
    }

    /// `begin`: see [`host_commands`].
    fn begin(&mut self, command: &BoundCommand, threads: Option<u32>) -> HostResult<Output> {
        use clash_sim::scenario::{BeginScenario, ScenarioDef, make_world};
        let written = command.word("scenario").unwrap_or_default().to_owned();
        let defs = Arc::clone(&self.profile.defs);
        let scenarios = defs.try_registry::<ScenarioDef>().ok_or_else(|| {
            refuse(
                clash_sim::scenario::why::UNKNOWN,
                "the profile defines no game scenario",
            )
        })?;
        let id = scenarios
            .lookup(&written)
            .or_else(|| {
                scenarios
                    .names()
                    .iter()
                    .filter(|name| name.rsplit(':').next() == Some(written.as_str()))
                    .min()
                    .and_then(|name| scenarios.lookup(name))
            })
            .ok_or_else(|| {
                HostError::Refused(
                    Refusal::new(
                        clash_sim::scenario::why::UNKNOWN,
                        format!("there is no game scenario `{written}`"),
                    )
                    .with("scenario", written.as_str()),
                )
            })?;
        let scenario = scenarios.name(id).to_owned();
        let def = scenarios.get(id);
        let place = command
            .int("place")
            .and_then(|place| u32::try_from(place).ok())
            .unwrap_or(0);
        let seed = def
            .world()
            .map_or(self.seed, |world| world.seed.choose(self.seed, self.seed));
        let made = make_world(&defs, Arc::clone(&self.profile.world), &scenario, seed)
            .map_err(|reason| refusal_of(&reason))?;
        let generators = self
            .profile
            .generators(threads)
            .map_err(HostError::Failed)?;
        let mut sim = Sim::from_parts(Arc::clone(&defs), made.cosmos, made.timeline, self.seed)
            .with_io(generators, Box::new(MemoryStore::new()));
        let namespace = scenario
            .split_once(':')
            .map_or("", |(namespace, _)| namespace);
        let template = def.template_id(namespace).ok_or_else(|| {
            HostError::failed(format!(
                "`{scenario}` names no template to make its character from"
            ))
        })?;
        let player = sim
            .truth()
            .entities()
            .next_id()
            .ok_or_else(|| HostError::failed("every entity slot has been used"))?;
        sim.submit(Command::Debug(DebugCommand::Spawn {
            spec: clash_sim::template::CharacterSpec::of([template]),
            place: Place::Stored,
        }))
        .map_err(|error| refusal_of(&error.reason))?;
        sim.submit(Command::BeginScenario(BeginScenario {
            scenario: scenario.clone(),
            party: vec![player],
            place,
            options: Vec::new(),
        }))
        .map_err(|error| refusal_of(&error.reason))?;
        self.sim = GameHost::set_up(sim);
        self.marks.active = Some(player);
        self.marks.here = self.sim.truth().location(player);
        let subject = self.value(Subject::Entity(player))?;
        let at = self
            .marks
            .here
            .map(|here| format!(" at @c:{},{},{}", here.cell.x, here.cell.y, here.cell.z))
            .unwrap_or_default();
        Ok(
            Output::text(format!("began `{scenario}`{at}; the character is @self"))
                .with_subject(subject),
        )
    }

    // ---- Commands --------------------------------------------------------

    fn spawn(&mut self, command: &BoundCommand) -> HostResult<Output> {
        let what = command.word("what").unwrap_or_default();
        let kind = self
            .word_arg(command, "what", SPAWN_KINDS, "kind of occupant")?
            .ok_or_else(|| HostError::failed("`what` has no value"))?;
        let place = match self.arg(command, "at")? {
            Some(value) => self.place_of(&value)?,
            None => Place::at(self.here()?),
        };
        let control = self
            .word_arg(command, "control", CONTROLS, "control mode")?
            .unwrap_or_default();
        let mut presence = Presence::new(kind);
        if kind == OccupantKind::Creature {
            presence = presence.with_flags(OccupantFlags::BLOCKS_MOVE);
        }
        if let Some(mass) = self.arg(command, "mass")?.and_then(|mass| mass.as_number()) {
            presence = presence.with_mass(mass);
        }
        let mut spec = EntitySpec::new(kind)
            .with_presence(presence)
            .with_control(control);
        if let Some(tag) = command.word("tag") {
            spec = spec.with_tag(tag);
        }
        let id = self.sim.truth().entities().next_id().ok_or_else(|| {
            refusal_of(&Reason::new(
                clash_sim::codes::ENTITY_IDS_EXHAUSTED,
                "every entity slot has been used",
            ))
        })?;
        self.submit(DebugCommand::CreateEntity { spec, place })?;
        if control == Control::Direct {
            self.marks.active = Some(id);
        }
        let subject = self.value(Subject::Entity(id))?;
        self.select(&subject);
        Ok(Output::text(format!("made the {what} @e:{id}")).with_subject(subject))
    }

    fn move_entity(&mut self, command: &BoundCommand) -> HostResult<Output> {
        let entity = self.entity_arg(command, "who")?;
        let to = self.place_of(&self.required(command, "to")?)?;
        self.submit(DebugCommand::MoveEntity { entity, to })?;
        let subject = self.value(Subject::Entity(entity))?;
        self.select(&subject);
        Ok(Output::none().with_subject(subject))
    }

    fn remove(&mut self, command: &BoundCommand) -> HostResult<Output> {
        let entity = self.entity_arg(command, "who")?;
        self.submit(DebugCommand::RemoveEntity { entity })?;
        if self.marks.active == Some(entity) {
            self.marks.active = None;
        }
        Ok(Output::none())
    }

    fn set_element(&mut self, command: &BoundCommand) -> HostResult<Output> {
        let loc = self.loc_arg(command, "at")?;
        let layer = self
            .word_arg(command, "layer", LAYERS, "layer")?
            .ok_or_else(|| HostError::failed("`layer` has no value"))?;
        let text = command.word("state").unwrap_or_default();
        let defs = Arc::clone(self.sim.truth().cosmos().defs());
        let state = if text == NO_ELEMENT {
            match layer {
                Layer::Floor => Ok(FloorState::NONE.raw()),
                Layer::Block => Ok(BlockState::NONE.raw()),
                Layer::Fixture => Ok(FixtureState::NONE.raw()),
            }
        } else {
            match layer {
                Layer::Floor => defs.floor_state(text, None).map(|state| state.raw()),
                Layer::Block => defs.block_state(text, None).map(|state| state.raw()),
                Layer::Fixture => defs.fixture_state(text, None).map(|state| state.raw()),
            }
        }
        .map_err(|error| {
            HostError::Refused(
                Refusal::new(reasons::UNKNOWN_KIND, error.to_string())
                    .with("what", "element state")
                    .with("name", text),
            )
        })?;
        self.submit(DebugCommand::SetElement { loc, layer, state })?;
        let subject = self.value(Subject::Loc(loc))?;
        self.select(&subject);
        Ok(Output::none().with_subject(subject))
    }

    fn control(&mut self, command: &BoundCommand) -> HostResult<Output> {
        let entity = self.entity_arg(command, "who")?;
        let control = self
            .word_arg(command, "mode", CONTROLS, "control mode")?
            .ok_or_else(|| HostError::failed("`mode` has no value"))?;
        self.submit(DebugCommand::SetControl { entity, control })?;
        if control == Control::Direct {
            self.marks.active = Some(entity);
        } else if self.marks.active == Some(entity) {
            self.marks.active = None;
        }
        Ok(Output::none())
    }

    fn session(&mut self, command: &BoundCommand) -> HostResult<Output> {
        let entity = self.entity_arg(command, "who")?;
        let at = self.loc_arg(command, "at")?;
        self.submit(DebugCommand::StartSession { entity, at })?;
        self.marks.here = Some(at);
        let loaded: usize = {
            let cosmos = self.sim.truth().cosmos();
            cosmos
                .realms()
                .map(|realm| cosmos.loaded_tracts(realm.id).len())
                .sum()
        };
        Ok(Output::text(format!("{loaded} tracts are loaded")))
    }

    fn damage(&mut self, command: &BoundCommand) -> HostResult<Output> {
        let loc = self.loc_arg(command, "at")?;
        let energy = self
            .required(command, "energy")?
            .as_number()
            .ok_or_else(|| HostError::failed("`energy` is not a number"))?;
        let intensity = self
            .arg(command, "intensity")?
            .and_then(|value| value.as_number())
            .unwrap_or(energy);
        let defs = Arc::clone(self.sim.truth().cosmos().defs());
        let kind = match command.word("kind") {
            Some(name) => {
                let types = defs.damage_types();
                types.lookup(name).or_else(|| types.resolve(name, None))
            }
            None => defs.kinetic_damage_type(),
        }
        .ok_or_else(|| {
            let name = command.word("kind").unwrap_or("the kinetic damage type");
            HostError::Refused(
                Refusal::new(
                    reasons::UNKNOWN_KIND,
                    format!("there is no damage type `{name}`"),
                )
                .with("what", "damage type")
                .with("name", name),
            )
        })?;
        // What a blow to the cell meets first: its block, else its
        // fixture, else its floor.
        let layers: Vec<Layer> = match self.word_arg(command, "layer", LAYERS, "layer")? {
            Some(layer) => vec![layer],
            None => {
                let cell = self.sim.truth().cosmos().cell(loc).ok();
                let first = [Layer::Block, Layer::Fixture, Layer::Floor]
                    .into_iter()
                    .find(|layer| {
                        cell.is_some_and(|cell| match layer {
                            Layer::Block => !cell.block.is_none(),
                            Layer::Fixture => !cell.fixture.is_none(),
                            Layer::Floor => !cell.floor.is_none(),
                        })
                    })
                    .unwrap_or(Layer::Block);
                vec![first]
            }
        };
        let packet = DamagePacket::new(kind, energy, intensity, Cause::DEBUG);
        let struck = self.sim.world(|ctx| {
            let mut edit = ctx.edit(Cause::DEBUG);
            let mut struck = Vec::new();
            for &layer in &layers {
                struck.push((layer, edit.damage(loc, layer, packet)?));
            }
            // Whoever delivers a packet to a cell tells its fields.
            edit.conduct(loc, packet)?;
            Ok::<_, WorldError>(struck)
        });
        self.settle();
        let struck = struck.map_err(|error| world_refusal(&error))?;
        let mut text = String::new();
        for (layer, outcome) in struck {
            let name = LAYERS
                .iter()
                .find(|(_, have)| *have == layer)
                .map_or("", |(name, _)| *name);
            let result = match outcome.result {
                DamageResult::Unaffected => "unaffected",
                DamageResult::Damaged => "damaged",
                DamageResult::Destroyed => "destroyed",
            };
            let _ = writeln!(
                text,
                "{name}: {result}, absorbed {} J, passed {} J",
                outcome.absorbed, outcome.passed
            );
        }
        let subject = self.value(Subject::Loc(loc))?;
        self.select(&subject);
        Ok(Output::text(text.trim_end()).with_subject(subject))
    }

    fn field(&mut self, command: &BoundCommand) -> HostResult<Output> {
        let loc = self.loc_arg(command, "at")?;
        let name = command.word("kind").unwrap_or_default();
        let defs = Arc::clone(self.sim.truth().cosmos().defs());
        let fields = defs.fields();
        let field = fields
            .lookup(name)
            .or_else(|| fields.resolve(name, None))
            .ok_or_else(|| {
                HostError::Refused(
                    Refusal::new(reasons::UNKNOWN_KIND, format!("there is no field `{name}`"))
                        .with("what", "field")
                        .with("name", name),
                )
            })?;
        let amount = command.int("amount").unwrap_or(255);
        let amount = i16::try_from(amount.clamp(-255, 255)).unwrap_or(0);
        let old = self
            .sim
            .world(|ctx| ctx.edit(Cause::DEBUG).add_field(loc, field, amount));
        self.settle();
        let old = old.map_err(|error| world_refusal(&error))?;
        let now = self.sim.truth().cosmos().field(loc, field).unwrap_or(0);
        let subject = self.value(Subject::Loc(loc))?;
        self.select(&subject);
        Ok(Output::text(format!("{}: {old} to {now}", fields.name(field))).with_subject(subject))
    }

    fn look(&mut self, command: &BoundCommand) -> HostResult<Output> {
        if let Some(value) = self.arg(command, "at")? {
            let loc = self.loc_of(&value)?;
            self.marks.here = Some(loc);
        }
        let here = self.here()?;
        let subject = self.value(Subject::Loc(here))?;
        self.select(&subject);
        Ok(Output::text(self.describe_cell(here)).with_subject(subject))
    }

    /// What is in a cell: its elements with their damage, its fields
    /// and whoever stands in it.
    fn describe_cell(&self, loc: Location) -> String {
        let cosmos = self.sim.truth().cosmos();
        let defs = cosmos.defs();
        let mut out = format!("@c:{},{},{}", loc.cell.x, loc.cell.y, loc.cell.z);
        let Ok(cell) = cosmos.cell(loc) else {
            out.push_str("  not loaded");
            return out;
        };
        let damage = cosmos.damage(loc).unwrap_or_default();
        let states = [
            ("floor", Layer::Floor, cell.floor.raw(), damage[0]),
            ("block", Layer::Block, cell.block.raw(), damage[1]),
            ("fixture", Layer::Fixture, cell.fixture.raw(), damage[2]),
        ];
        for (name, layer, state, damage) in states {
            if state.is_none() {
                continue;
            }
            let _ = write!(out, "\n  {name}: {}", defs.state_text(layer, state));
            if damage > 0 {
                let _ = write!(out, " (damage {damage})");
            }
        }
        for (field, level) in cosmos.fields_at(loc).unwrap_or_default() {
            let _ = write!(out, "\n  field: {} {level}", defs.fields().name(field));
        }
        for occupied in cosmos
            .occupants_at(loc, &OccupantFilter::ANY)
            .unwrap_or_default()
        {
            let kind = SPAWN_KINDS
                .iter()
                .find(|(_, kind)| *kind == occupied.kind)
                .map_or("occupant", |(name, _)| *name);
            let _ = write!(out, "\n  {kind}: @e:{}", occupied.id);
        }
        out
    }

    fn step(&mut self, command: &BoundCommand) -> HostResult<Output> {
        let count = u32::try_from(command.int("count").unwrap_or(1)).unwrap_or(0);
        let before = self.sim.steps();
        if let Advance::NeedInput(request) = self.sim.advance(AdvanceLimit::Work(count)) {
            let now = self.sim.now();
            self.wait_out(&request, now)?;
        }
        Ok(Output::text(format!(
            "handled {} wake-ups; t={}",
            self.sim.steps() - before,
            self.sim.now().since(Tick::ZERO)
        )))
    }

    /// The invariants that can be checked today (14 §7.3): every
    /// component has an owner, and every mass agrees with what it
    /// holds. The registry of checkers is tooling package TL7's.
    fn check(&self) -> HostResult<Output> {
        let sim = &self.sim;
        let mut violations: Vec<String> = Vec::new();
        for (kind, owner) in sim
            .components()
            .orphans(|id| sim.truth().entities().contains(id))
        {
            violations.push(format!(
                "a `{}` component belongs to entity {owner}, which does not exist",
                kind.name()
            ));
        }
        for mismatch in physical::mismatches(&sim.components().physical, sim.truth().entities()) {
            violations.push(format!("a mass does not add up: {mismatch:?}"));
        }
        if violations.is_empty() {
            Ok(Output::text("2 checks, no violations"))
        } else {
            Err(HostError::failed(format!(
                "{} violations:\n{}",
                violations.len(),
                violations.join("\n")
            )))
        }
    }

    fn inspect(&mut self, command: &BoundCommand) -> HostResult<Output> {
        let value = match self.arg(command, "subject")? {
            Some(value) => value,
            None => match &self.marks.selection {
                Some(selection) => Value::from(selection),
                None => self.value(Subject::Loc(self.here()?))?,
            },
        };
        let Some(subject) = self.sim.subject_of(&value) else {
            return Err(refuse(
                reasons::NOT_A_PLACE,
                format!("`{value}` is not a subject"),
            ));
        };
        let mut out = format!("{} {value}", subject.kind());
        for property in clash_sim::engine_properties() {
            if property.kind != subject.kind() || !property.args.is_empty() {
                continue;
            }
            let found = self.sim.property_named(subject, property.name, &[]);
            let _ = write!(out, "\n  {} = {found}", property.name);
        }
        Ok(Output::text(out).with_subject(value))
    }

    fn hash(&self, command: &BoundCommand) -> HostResult<Output> {
        let parts = crate::subsystem_hashes(&self.sim);
        let wanted = command.word("subsystem").unwrap_or("all");
        if wanted == "all" {
            let lines: Vec<String> = parts
                .iter()
                .map(|(name, hash)| format!("{name:<10} {hash}"))
                .collect();
            return Ok(Output::text(lines.join("\n")));
        }
        parts
            .iter()
            .find(|(name, _)| *name == wanted)
            .map(|(_, hash)| Output::text(hash.to_string()))
            .ok_or_else(|| {
                let known: Vec<&str> = parts.iter().map(|(name, _)| *name).collect();
                HostError::Refused(
                    Refusal::new(
                        reasons::UNKNOWN_SUBSYSTEM,
                        format!(
                            "there is no subsystem `{wanted}`: it is one of {}, or all",
                            known.join(", ")
                        ),
                    )
                    .with("subsystem", wanted),
                )
            })
    }

    fn size(&mut self, command: &BoundCommand) -> HostResult<Output> {
        let side = |name: &str| {
            command
                .int(name)
                .and_then(|value| u16::try_from(value).ok())
                .filter(|value| *value >= 1)
                .ok_or_else(|| HostError::failed(format!("`{name}` is from 1 to 65535")))
        };
        self.marks.cols = side("cols")?;
        self.marks.rows = side("rows")?;
        Ok(Output::none())
    }

    // ---- References and expressions ----------------------------------------

    /// The grid a bare cell is in: the cursor's, or the first realm's.
    fn grid(&self) -> HostResult<clash_world::GridId> {
        if let Some(here) = self.marks.here {
            return Ok(here.grid);
        }
        self.sim
            .truth()
            .cosmos()
            .realms()
            .next()
            .map(|realm| realm.base_grid)
            .ok_or_else(|| {
                refuse(
                    reasons::NO_WORLD,
                    "the simulation has no realm; `gen <kind>` makes one",
                )
            })
    }

    fn resolve(&self, reference: &Reference) -> HostResult<Value> {
        match reference {
            Reference::Here => self.value(Subject::Loc(self.here()?)),
            Reference::Active => {
                let active = self
                    .marks
                    .active
                    .filter(|id| self.sim.truth().entities().contains(*id))
                    .ok_or_else(|| {
                        refuse(
                            reasons::NO_ACTIVE,
                            "no character is active; `control <who> direct` makes one active",
                        )
                    })?;
                self.value(Subject::Entity(active))
            }
            Reference::Selection => self
                .marks
                .selection
                .as_ref()
                .map(Value::from)
                .ok_or_else(|| refuse(reasons::NOTHING_SELECTED, "nothing is selected")),
            Reference::Cell { x, y, z } => self.value(Subject::Loc(Location::new(
                self.grid()?,
                CellPos::new(*x, *y, *z),
            ))),
            Reference::Entity { slot, generation } => {
                let found = self
                    .sim
                    .truth()
                    .entities()
                    .ids()
                    .find(|id| id.slot() == *slot)
                    .filter(|id| generation.is_none_or(|wanted| id.generation() == wanted));
                match found {
                    Some(id) => self.value(Subject::Entity(id)),
                    None => Err(refusal_of(
                        &Reason::new(
                            clash_sim::codes::ENTITY_UNKNOWN,
                            format!("there is no entity {reference}"),
                        )
                        .with("entity", reference.to_string()),
                    )),
                }
            }
            Reference::Of { kind, name } => {
                let unknown = |message: String| {
                    HostError::Refused(
                        Refusal::new(reasons::UNKNOWN_REFERENCE, message)
                            .with("kind", kind.as_str())
                            .with("name", name.as_str()),
                    )
                };
                match kind.as_str() {
                    "site" => {
                        let site = self
                            .sim
                            .truth()
                            .cosmos()
                            .sites()
                            .find(|site| site.name == *name)
                            .ok_or_else(|| unknown(format!("there is no site named `{name}`")))?;
                        self.value(Subject::Site(site.id))
                    }
                    other => Err(unknown(format!(
                        "the console has no reference of kind `{other}`; it has `site`"
                    ))),
                }
            }
            Reference::Named(name) => Err(HostError::Refused(
                Refusal::new(
                    reasons::UNKNOWN_NAME,
                    format!(
                        "nothing is named `{name}`; `as={name}` on a command names what it gives"
                    ),
                )
                .with("name", name.as_str()),
            )),
        }
    }

    /// Compiles a console expression: the code, and what it is bound
    /// to. `self` is the subject given, else the active character, else
    /// nothing.
    fn compile(
        &self,
        expression: &Expression,
        subject: Option<&Value>,
    ) -> HostResult<(Compiled, Bound)> {
        let own = subject.cloned().unwrap_or_else(|| {
            self.marks
                .active
                .filter(|id| self.sim.truth().entities().contains(*id))
                .map_or(Value::Null, |id| {
                    self.sim.subject_value(Subject::Entity(id))
                })
        });
        let own_type = match &own {
            Value::Null => Type::Any,
            other => type_of(other),
        };
        let mut context = Context::new().subject("self", own_type);
        let mut bound = Bound::new().subject(own);
        for local in &expression.locals {
            context.add_local(local.name.as_str(), type_of(&local.value));
            bound = bound.local(local.value.clone());
        }
        let registry = self.sim.defs().expr_registry();
        compile(&expression.source, registry, &context, &Type::Any)
            .into_result()
            .map(|code| (code, bound))
            .map_err(|errors| {
                HostError::failed(format!(
                    "`{}` does not compile: {}",
                    expression.written,
                    errors
                        .iter()
                        .map(ToString::to_string)
                        .collect::<Vec<_>>()
                        .join("; ")
                ))
            })
    }

    /// Evaluates a console expression against truth, reading only: an
    /// evaluation never changes the simulation, so a function that
    /// draws at random gives null here.
    fn eval(&self, expression: &Expression, subject: Option<&Value>) -> HostResult<Value> {
        let (code, bound) = self.compile(expression, subject)?;
        Ok(self.sim.eval(&code, bound))
    }

    // ---- Time ------------------------------------------------------------

    /// Answers a decision the simulation stopped for, as the console
    /// does with nobody at the keys: an entity free to act waits until
    /// `until` (03 §5.2), by a `Wait` command, logged like any other. A
    /// choice put by a rule is not answered: time cannot pass.
    fn wait_out(&mut self, request: &DecisionRequest, until: Tick) -> HostResult<()> {
        if !matches!(request.kind, DecisionKind::Free { .. }) {
            return Err(decision_pending(request.entity));
        }
        let dur = if until > self.sim.now() {
            until - self.sim.now()
        } else {
            Dur::ZERO
        };
        self.sim
            .submit(Command::Wait {
                entity: request.entity,
                dur,
            })
            .map_err(|error| refusal_of(&error.reason))
    }

    /// Runs the simulation up to a tick.
    fn run_until(&mut self, target: Tick) -> HostResult<()> {
        for _ in 0..MAX_ADVANCE_ROUNDS {
            match self.sim.advance(AdvanceLimit::Until(target)) {
                Advance::Reached(_) => return Ok(()),
                // More is due at this tick, or a rule asked the
                // application for something the console has no
                // application to do: go on.
                Advance::Yielded | Advance::Request(_) => {}
                Advance::NeedInput(request) => self.wait_out(&request, target)?,
            }
        }
        Err(HostError::failed(format!(
            "time did not reach {} after {MAX_ADVANCE_ROUNDS} rounds of work: something keeps the simulation at tick {}",
            target.since(Tick::ZERO),
            self.sim.now().as_micros()
        )))
    }
}

fn decision_pending(entity: EntityId) -> HostError {
    HostError::Refused(
        Refusal::new(
            reasons::DECISION_PENDING,
            format!(
                "entity {entity} must choose before time can pass, and nobody is there to choose"
            ),
        )
        .with("entity", entity.to_string()),
    )
}

impl ConsoleHost for GameHost {
    fn start(&mut self, profile: &str, seed: u64) -> HostResult<Output> {
        let result = (|| {
            let profile = self.env.profile(profile).map_err(|diagnostics| {
                HostError::failed(format!(
                    "the profile `{profile}` does not load:\n{diagnostics}"
                ))
            })?;
            let summary = format!(
                "started `{}` with seed {seed}: {} mods, fingerprint {}",
                profile.name,
                profile.defs.mods().len(),
                profile.defs.fingerprint()
            );
            self.game = Some(Game::start(profile, seed, self.threads)?);
            Ok(Output::text(summary))
        })();
        self.noted(result)
    }

    fn generate(&mut self, kind: &str, size: u32) -> HostResult<Output> {
        let result = self.game_mut().and_then(|game| game.generate(kind, size));
        self.noted(result)
    }

    fn execute(&mut self, command: &BoundCommand) -> HostResult<Output> {
        let result = self.carry_out(command);
        self.noted(result)
    }

    fn resolve(&mut self, reference: &Reference) -> HostResult<Value> {
        self.game()?.resolve(reference)
    }

    fn eval(&mut self, expression: &Expression, subject: Option<&Value>) -> HostResult<Value> {
        self.game()?.eval(expression, subject)
    }

    fn advance(&mut self, time: Dur) -> HostResult<()> {
        let result = self.game_mut().and_then(|game| {
            let target = game.sim.now() + time;
            game.run_until(target)
        });
        self.noted(result)
    }

    fn advance_until(
        &mut self,
        condition: &Expression,
        subject: Option<&Value>,
        max: Dur,
    ) -> HostResult<bool> {
        let result = self.game_mut().and_then(|game| {
            let (code, bound) = game.compile(condition, subject)?;
            let holds = |sim: &Sim| sim.eval(&code, bound.clone()) == Value::Bool(true);
            let deadline = game.sim.now() + max;
            for _ in 0..MAX_ADVANCE_ROUNDS.saturating_mul(100) {
                if holds(&game.sim) {
                    return Ok(true);
                }
                let due = game
                    .sim
                    .timeline()
                    .peek()
                    .is_some_and(|due| due <= deadline);
                if !due {
                    game.run_until(deadline)?;
                    return Ok(holds(&game.sim));
                }
                // One wake-up, then look again: the condition is
                // checked between events.
                if let Advance::NeedInput(request) = game.sim.advance(AdvanceLimit::Work(1)) {
                    let next = game.sim.timeline().peek().unwrap_or(deadline).min(deadline);
                    game.wait_out(&request, next)?;
                }
            }
            Err(HostError::failed(
                "the condition was checked more often than a console waits for",
            ))
        });
        self.noted(result)
    }

    fn now(&self) -> Tick {
        self.game.as_ref().map_or(Tick::ZERO, |game| game.sim.now())
    }

    fn state_hash(&self) -> Hash128 {
        self.game
            .as_ref()
            .map_or_else(Hash128::default, |game| game.sim.state_hash())
    }

    fn save(&mut self) -> HostResult<Vec<u8>> {
        let Some(game) = &mut self.game else {
            return Ok(NO_GAME_BYTES.to_vec());
        };
        let summary = format!("t={}", game.sim.now().since(Tick::ZERO));
        let meta = game.meta("console", &summary)?;
        save_game_bytes(&mut game.sim, &meta)
            .map_err(|error| HostError::failed(format!("could not save: {error}")))
    }

    fn load(&mut self, bytes: &[u8]) -> HostResult<()> {
        self.last_refusal = None;
        if bytes == NO_GAME_BYTES {
            self.game = None;
            return Ok(());
        }
        let mut store = unpack_store(bytes)
            .map_err(|error| HostError::failed(format!("the save does not load: {error}")))?;
        self.load_store(&mut store).map(|_| ())
    }

    fn drain_records(&mut self) -> Vec<Record> {
        let Some(game) = &mut self.game else {
            return Vec::new();
        };
        let events = game.sim.drain_events();
        events
            .iter()
            .map(|event| record_of(&game.sim, event))
            .collect()
    }

    fn last_refusal(&self) -> Option<Refusal> {
        self.last_refusal.clone()
    }

    fn frame(&mut self) -> HostResult<String> {
        Ok(frame_to_text(&self.paint()?))
    }

    fn set_threads(&mut self, threads: u32) -> HostResult<()> {
        if threads == 0 {
            return Err(HostError::failed("a host needs at least one thread"));
        }
        self.threads = Some(threads);
        // A simulation that is already running is given generators on
        // the new pool; nothing that is state changes.
        if let Some(game) = self.game.take() {
            let generators = game
                .profile
                .generators(self.threads)
                .map_err(HostError::Failed)?;
            let store = game.sim.store().clone_store();
            self.game = Some(Game {
                sim: game.sim.with_io(generators, store),
                ..game
            });
        }
        Ok(())
    }
}
