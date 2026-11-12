//! Test support shared by this crate's tests: a small world to stand in,
//! made from the world content of `mods/core` loaded through the mod
//! loader, with a few definitions the tests add as a file of the same
//! mod.

// Each test binary uses its own part of this module.
#![allow(dead_code)]

use std::path::{Path, PathBuf};
use std::sync::Arc;

use clash_core::{Dur, EntityId};
use clash_data::{DefTypes, Defs, Loader};
use clash_sim::{
    Command, DebugCommand, EntitySpec, Place, Presence, Sim, SimEvent, Subject, Unplaced,
};
use clash_time::{Timeline, Wake, WakeDomain, WakeId};
use clash_world::dims::TRACT_XY;
use clash_world::test_support::{load_column, load_flat, unload_column};
use clash_world::{
    AreaId, AreaShape, AreaSpec, BlockState, Cause, CellBox, CellPos, Cosmos, GridId, Location,
    OccupantFlags, OccupantKind, RealmId, TractPos, WorldDefs,
};

/// The base grid of the one realm these tests make.
pub const GRID: GridId = GridId::new(0);

/// How many cells the loaded ground measures along each side: two
/// tracts.
pub const SIDE: i32 = 2 * TRACT_XY;

/// Definitions the tests add to `mods/core`.
const EXTRA_DEFS: &str = r#"[
  { "type": "area_kind", "id": "stopped_time", "clock_rate": 0, "priority": 100 },
  { "type": "area_kind", "id": "slow_time", "clock_rate": 0.5 },
  { "type": "event", "id": "ritual_completed",
    "subjects": { "caster": "entity", "here": "loc" },
    "fields": { "power": "real", "school": "string" },
    "doc": "A ritual circle finished charging." },
  { "type": "event", "id": "ritual_completing",
    "subjects": { "caster": "entity" },
    "fields": { "power": "real" },
    "hook": true, "mutable": ["power"],
    "doc": "A ritual circle is about to finish charging." },
  { "type": "event", "id": "echo",
    "fields": { "depth": "int" },
    "doc": "Raised by a test handler in answer to itself." }
]"#;

/// The repository's `mods` directory.
pub fn mods_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../mods")
}

/// The definition types of the expression language, the world and the
/// simulation, registered.
pub fn types() -> DefTypes {
    let mut types = DefTypes::new();
    clash_data::expr::register_defs(&mut types);
    clash_world::register_defs(&mut types);
    clash_sim::register_defs(&mut types);
    types
}

/// The world definition files of `mods/core`, as paths within the mod
/// and their text, in file-name order.
fn core_files() -> Vec<(String, String)> {
    core_dir_files("world")
}

/// The definition files of one directory under `mods/core/data`, as
/// paths within the mod and their text, in file-name order.
fn core_dir_files(sub: &str) -> Vec<(String, String)> {
    let dir = mods_dir().join("core/data").join(sub);
    let mut files: Vec<PathBuf> = std::fs::read_dir(&dir)
        .unwrap_or_else(|error| panic!("{}: {error}", dir.display()))
        .filter_map(|entry| entry.ok().map(|entry| entry.path()))
        .filter(|path| {
            path.extension()
                .is_some_and(|extension| extension == "json")
        })
        .collect();
    files.sort();
    files
        .iter()
        .map(|file| {
            let name = file.file_name().unwrap_or_default().to_string_lossy();
            let text = std::fs::read_to_string(file)
                .unwrap_or_else(|error| panic!("{}: {error}", file.display()));
            (format!("data/{sub}/{name}"), text)
        })
        .collect()
}

/// Loads `mods/core`'s world content with one more file of definitions,
/// or renders why not.
pub fn try_defs(extra: &str) -> Result<Arc<Defs>, String> {
    let types = types();
    let mut files = core_files();
    files.push(("data/test/extra.json".to_owned(), extra.to_owned()));
    let files: Vec<(&str, &str)> = files
        .iter()
        .map(|(path, text)| (path.as_str(), text.as_str()))
        .collect();
    Loader::new(&types)
        .expr_registry(&clash_sim::engine_registry())
        .memory_mod("core", r#"{ "id": "core" }"#, &files)
        .load_mods(&["core"])
        .into_result()
        .map_err(|diagnostics| diagnostics.render())
}

/// Loads `mods/core`'s world content with more files of definitions,
/// each a path within the mod and its text, or renders why not.
pub fn try_defs_with(extra: &[(&str, &str)]) -> Result<Arc<Defs>, String> {
    let types = types();
    let core = core_files();
    let files: Vec<(&str, &str)> = core
        .iter()
        .map(|(path, text)| (path.as_str(), text.as_str()))
        .chain(extra.iter().copied())
        .collect();
    Loader::new(&types)
        .expr_registry(&clash_sim::engine_registry())
        .memory_mod("core", r#"{ "id": "core" }"#, &files)
        .load_mods(&["core"])
        .into_result()
        .map_err(|diagnostics| diagnostics.render())
}

/// Loads `mods/core`'s world content and its living things (bodies,
/// stats, templates), with one more file of definitions.
pub fn life_defs(extra: &str) -> Arc<Defs> {
    let life = core_dir_files("life");
    let mut files: Vec<(&str, &str)> = life
        .iter()
        .map(|(path, text)| (path.as_str(), text.as_str()))
        .collect();
    // The channel arms supply is defined with `mods/core`'s items,
    // which need the interface's colours: here it is on its own.
    files.push((
        "data/test/channels.json",
        r#"[{ "type": "channel", "id": "arm", "name": "arm" }]"#,
    ));
    files.push(("data/test/extra.json", extra));
    try_defs_with(&files)
        .unwrap_or_else(|error| panic!("the test definitions do not load:\n{error}"))
}

/// The definitions every test here runs on.
pub fn defs() -> Arc<Defs> {
    try_defs(EXTRA_DEFS)
        .unwrap_or_else(|error| panic!("the test definitions do not load:\n{error}"))
}

/// A cell of the test grid at ground level.
pub fn at(x: i32, y: i32) -> Location {
    Location::new(GRID, CellPos::new(x, y, 0))
}

/// The place of something standing in a cell at ground level.
pub fn on(x: i32, y: i32) -> Place {
    Place::at(at(x, y))
}

/// A simulation over one `testland` realm with flat ground loaded in
/// tracts (0, 0) to (1, 1): cells 0 to `SIDE - 1` each way, standing
/// room on storey 0.
pub fn sim(seed: u64) -> Sim {
    sim_over(defs(), seed)
}

/// [`sim`] over definitions a test loaded itself.
pub fn sim_over(defs: Arc<Defs>, seed: u64) -> Sim {
    let world = WorldDefs::from_defs(&defs)
        .unwrap_or_else(|diagnostics| panic!("{}", diagnostics.render()));
    let mut timeline = Timeline::new();
    let mut cosmos = Cosmos::new(Arc::clone(&world));
    let realm_def = world.realms().lookup("core:testland").unwrap();
    let realm = cosmos.create_realm(&mut timeline, realm_def, 7).unwrap();
    assert_eq!(cosmos.realm(realm).unwrap().base_grid, GRID);
    let rock = world.block_state("solid", None).unwrap();
    let grass = world.floor_state("grass", None).unwrap();
    let tracts = [0, 1]
        .into_iter()
        .flat_map(|y| [0, 1].map(|x| TractPos::new(x, y)));
    load_flat(&mut cosmos, GRID, tracts, rock, grass, 0).unwrap();
    Sim::from_parts(defs, cosmos, timeline, seed)
}

/// The one realm of [`sim`].
pub fn realm(sim: &Sim) -> RealmId {
    sim.truth().cosmos().realm_of(GRID).unwrap()
}

/// The `wall` block of `mods/core`.
pub fn wall(sim: &Sim) -> BlockState {
    sim.truth()
        .cosmos()
        .defs()
        .block_state("wall", None)
        .unwrap()
}

/// What a creature is made from in these tests: it blocks movement.
pub fn creature() -> EntitySpec {
    EntitySpec::new(OccupantKind::Creature)
        .with_presence(Presence::new(OccupantKind::Creature).with_flags(OccupantFlags::BLOCKS_MOVE))
}

/// What an item is made from in these tests.
pub fn item() -> EntitySpec {
    EntitySpec::new(OccupantKind::Item)
}

/// Makes an entity through the simulation's own functions.
pub fn spawn(sim: &mut Sim, spec: EntitySpec, place: Place) -> EntityId {
    sim.spawn(spec, place, Cause::DEBUG).unwrap()
}

/// Submits a debug command.
pub fn debug(sim: &mut Sim, command: DebugCommand) -> Result<(), clash_sim::CommandError> {
    sim.submit(Command::Debug(command))
}

/// Makes a box-shaped area of the test grid, with a clock at `rate` if
/// one is given, at a priority.
pub fn box_area(
    sim: &mut Sim,
    min: (i32, i32),
    max: (i32, i32),
    rate: Option<f64>,
    priority: i32,
) -> AreaId {
    let shape = AreaShape::Box(CellBox::new(
        CellPos::new(min.0, min.1, 0),
        CellPos::new(max.0, max.1, 0),
    ));
    let mut spec = AreaSpec::new(GRID, shape);
    spec.clock_rate = rate.map(clash_core::Real::of);
    spec.priority = Some(priority);
    sim.world(|ctx| ctx.create_area(spec, Cause::DEBUG))
        .unwrap()
}

/// A wake-up for the test handler [`walker`]: it moves `entity` and
/// comes back `hops` more times.
pub fn walk_wake(entity: EntityId, hops: u64) -> Wake {
    Wake::new(WakeDomain::Stepper, entity.to_bits(), hops)
}

/// A wake-up handler for tests: moves the entity the wake-up names one
/// cell in a direction drawn from the `test.walk` stream, and schedules
/// itself again, after a drawn delay, as many more times as the
/// wake-up's argument says.
pub fn walker(sim: &mut Sim, _id: WakeId, wake: Wake) {
    let entity = EntityId::from_bits(wake.id);
    let Some(loc) = sim.truth().place(entity).and_then(Place::loc) else {
        return;
    };
    let stream = sim.rng.stream("test.walk");
    let (dx, dy) = (stream.range(-1, 2) as i32, stream.range(-1, 2) as i32);
    let delay = Dur::from_millis(50 + stream.below(400) as i64);
    let to = at(
        (loc.cell.x + dx).clamp(0, SIDE - 1),
        (loc.cell.y + dy).clamp(0, SIDE - 1),
    );
    let _ = sim.relocate(entity, Place::at(to), Cause::DEBUG.by(entity));
    if wake.arg > 0 {
        let _ = sim.schedule_for(entity, delay, walk_wake(entity, wake.arg - 1));
    }
}

/// A simulation as [`sim`] makes it, with the test handler registered.
pub fn walking_sim(seed: u64) -> Sim {
    let mut sim = sim(seed);
    sim.set_wake_handler(WakeDomain::Stepper, walker);
    sim
}

/// Saves a simulation and loads it again as a fresh process would: the
/// simulation's own state through its blob, the world as its store
/// would hand it back, with nobody in its occupant index, and then
/// `reindex`. Returns the loaded simulation and whoever could not be
/// placed again.
pub fn save_and_load(sim: &Sim) -> (Sim, Vec<Unplaced>) {
    let blob = sim.save().unwrap();
    let mut cosmos = sim.truth().cosmos().clone();
    // The world does not store who occupies it: a loaded column holds
    // no one.
    let tracts = cosmos.grid(GRID).unwrap().loaded_tracts();
    for tract in tracts {
        let column = unload_column(&mut cosmos, GRID, tract).unwrap();
        load_column(&mut cosmos, GRID, tract, column).unwrap();
    }
    assert_eq!(cosmos.occupant_count(), 0);
    let mut loaded = Sim::load(Arc::clone(sim.defs()), cosmos, &blob).unwrap();
    let unplaced = loaded.reindex();
    (loaded, unplaced)
}

/// The names of events, in order.
pub fn names(events: &[SimEvent]) -> Vec<&str> {
    events.iter().map(SimEvent::name).collect()
}

/// The entity an event is about, under a subject name.
pub fn entity_of(event: &SimEvent, name: &str) -> Option<EntityId> {
    event.subject(name).and_then(|subject| match subject {
        Subject::Entity(id) => Some(id),
        _ => None,
    })
}

/// Loads `mods/core`'s world content and, beside it, fixture mods found
/// under `search`, by id: the definitions a test of rules or actions
/// runs on, loaded through the loader as a game's are.
pub fn defs_with(search: &Path, mods: &[&str]) -> Arc<Defs> {
    let types = types();
    let files = core_files();
    let files: Vec<(&str, &str)> = files
        .iter()
        .map(|(path, text)| (path.as_str(), text.as_str()))
        .collect();
    let mut listed = vec!["core"];
    listed.extend_from_slice(mods);
    Loader::new(&types)
        .expr_registry(&clash_sim::engine_registry())
        .memory_mod("core", r#"{ "id": "core" }"#, &files)
        .search_path(search)
        .load_mods(&listed)
        .into_result()
        .unwrap_or_else(|diagnostics| panic!("the fixtures do not load:\n{}", diagnostics.render()))
}

/// The fixture mods of the tests of rules and actions bound to the
/// simulation.
pub fn act_fixtures() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/act/mods")
}
