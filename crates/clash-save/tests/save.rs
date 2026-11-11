//! The whole-game save (01 §11; 02 §11.4): a game saved in the middle
//! of a collapse, with tracts unloaded behind a walker and an entity
//! shelved with one of them, loads to the same state hash and runs on
//! to the same end as a game that was never saved, through a real file
//! and through bytes; a save made with a mod that is gone loads with
//! placeholders and says so; a damaged save is an error.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use clash_core::{EntityId, Hash128, Tick};
use clash_data::Defs;
use clash_save::{
    SaveError, SaveMeta, generators_for, list_saves, load_game, load_game_bytes, load_warnings,
    read_meta, save_game, save_game_bytes, verify,
};
use clash_sim::{
    Advance, AdvanceLimit, Command, DebugCommand, EntitySpec, Place, Presence, Sim, SimEvent,
};
use clash_time::Timeline;
use clash_world::dims::TRACT_XY;
use clash_world::{
    BlockState, CellPos, Cosmos, GridId, Layer, Location, MemoryStore, OccupantFlags, OccupantKind,
    RealmId, RedbStore, TractPos, TractStore, WorldDefs,
};

/// The repository's `mods` directory.
fn mods_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../mods")
}

/// The default profile, with one more mod in memory if asked for.
fn defs(extra: bool) -> Arc<Defs> {
    let types = clash_scene::engine_types();
    let mut loader = clash_scene::engine_loader(&types).search_path(mods_dir());
    let outcome = if extra {
        loader = loader.memory_mod(
            "extra",
            r#"{ "id": "extra", "version": "1.2.0", "requires": ["core"] }"#,
            &[(
                "data/crate.json",
                r#"[ { "type": "block", "id": "crate", "material": "core:wood",
                       "height": 1.0, "fill": 0.5, "flags": ["opaque"],
                       "glyph": { "ch": "x", "fg": "material" } } ]"#,
            )],
        );
        loader.load_mods(&["core", "ui", "extra"])
    } else {
        loader.load_profile("default")
    };
    outcome
        .into_result()
        .unwrap_or_else(|diagnostics| panic!("{}", diagnostics.render()))
}

/// A fresh directory of this test's own under the system's temporary
/// directory.
fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("clash-save-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

const GRID: GridId = GridId::new(0);
/// Where the walk starts: a tract well inside the city.
const START: TractPos = TractPos::new(20, 20);
/// How many tracts east the walker goes: far enough for the start to be
/// unloaded behind it.
const FAR: i32 = 11;

fn cell(tract: TractPos, dx: i32, dy: i32, z: i32) -> Location {
    Location::new(
        GRID,
        CellPos::new(tract.x * TRACT_XY + dx, tract.y * TRACT_XY + dy, z),
    )
}

fn debug(sim: &mut Sim, command: DebugCommand) {
    sim.submit(Command::Debug(command))
        .unwrap_or_else(|error| panic!("{error}"));
}

/// A simulation over an empty city realm, with every bundled generator.
fn city(defs: &Arc<Defs>) -> (Sim, RealmId) {
    let world = WorldDefs::from_defs(defs).unwrap_or_else(|d| panic!("{}", d.render()));
    let mut timeline = Timeline::new();
    let mut cosmos = Cosmos::new(Arc::clone(&world));
    let def = world.realms().lookup("core:test_city").unwrap();
    let realm = cosmos.create_realm(&mut timeline, def, 11).unwrap();
    let sim = Sim::from_parts(Arc::clone(defs), cosmos, timeline, 5)
        .with_io(generators_for(defs).unwrap(), Box::new(MemoryStore::new()));
    (sim, realm)
}

fn step(sim: &mut Sim, walker: EntityId, by: i32) {
    let Some(Place::At { loc, .. }) = sim.truth().place(walker) else {
        panic!("the walker is not in the world");
    };
    let to = Location::new(
        loc.grid,
        CellPos::new(loc.cell.x + by * TRACT_XY, loc.cell.y, loc.cell.z),
    );
    debug(
        sim,
        DebugCommand::MoveEntity {
            entity: walker,
            to: Place::at(to),
        },
    );
}

/// The first half of the game: a session starts, something is left
/// behind beside a wall that was not there, the walker goes east until
/// the start is unloaded, the ground storey of the buildings round it
/// is taken out, and time passes until masses are falling.
fn first_half(defs: &Arc<Defs>) -> (Sim, RealmId, EntityId, EntityId) {
    let (mut sim, realm) = city(defs);
    debug(
        &mut sim,
        DebugCommand::CreateEntity {
            spec: EntitySpec::new(OccupantKind::Creature).with_presence(
                Presence::new(OccupantKind::Creature).with_flags(OccupantFlags::BLOCKS_MOVE),
            ),
            place: Place::Stored,
        },
    );
    let walker = sim.truth().entities().ids().next().unwrap();
    let start = cell(START, 1, 1, 0);
    debug(
        &mut sim,
        DebugCommand::StartSession {
            entity: walker,
            at: start,
        },
    );
    debug(
        &mut sim,
        DebugCommand::CreateEntity {
            spec: EntitySpec::new(OccupantKind::Item).with_tag("left_behind"),
            place: Place::at(cell(START, 2, 1, 0)),
        },
    );
    let idle = sim.truth().entities().ids().nth(1).unwrap();
    let wall = sim
        .truth()
        .cosmos()
        .defs()
        .block_state("core:wall", None)
        .unwrap();
    debug(
        &mut sim,
        DebugCommand::SetElement {
            loc: cell(START, 1, 2, 0),
            layer: Layer::Block,
            state: wall.raw(),
        },
    );
    for _ in 0..FAR {
        step(&mut sim, walker, 1);
    }
    assert!(!sim.truth().cosmos().is_tract_loaded(realm, START));
    assert!(sim.shelf().of(idle).is_some(), "the idle one is shelved");
    assert!(sim.store().get(realm, START).unwrap().is_some());

    // Out with the ground storey of whatever stands in the walker's
    // tract and the one east of it.
    let here = TractPos::new(START.x + FAR, START.y);
    let mut removed = 0;
    for tract in [here, TractPos::new(here.x + 1, here.y)] {
        for dy in 0..TRACT_XY {
            for dx in 0..TRACT_XY {
                let loc = cell(tract, dx, dy, 0);
                let has_block = sim
                    .truth()
                    .cosmos()
                    .cell(loc)
                    .is_ok_and(|found| !found.block.is_none());
                if has_block {
                    debug(
                        &mut sim,
                        DebugCommand::SetElement {
                            loc,
                            layer: Layer::Block,
                            state: BlockState::NONE.raw(),
                        },
                    );
                    removed += 1;
                }
            }
        }
    }
    assert!(removed > 20, "there were buildings: {removed}");
    // Into the middle of the collapse.
    let mut falling = false;
    for _ in 0..100 {
        let until = Tick(sim.now().as_micros() + 50_000);
        assert_eq!(
            sim.advance(AdvanceLimit::Until(until)),
            Advance::Reached(until)
        );
        if sim.truth().cosmos().falling_masses().count() > 0 {
            falling = true;
            break;
        }
    }
    assert!(falling, "nothing came down");
    assert!(sim.truth().cosmos().collapses().count() > 0);
    (sim, realm, walker, idle)
}

/// The second half: the walker goes back, and time runs until nothing
/// is left to happen.
fn second_half(sim: &mut Sim, walker: EntityId) -> Vec<String> {
    for _ in 0..FAR {
        step(sim, walker, -1);
    }
    assert!(matches!(
        sim.advance(AdvanceLimit::UntilDecision),
        Advance::Reached(_)
    ));
    assert!(sim.truth().cosmos().processes().is_idle());
    sim.drain_events()
        .iter()
        .map(|event: &SimEvent| event.name().to_owned())
        .collect()
}

#[test]
fn a_game_saved_in_the_middle_of_a_collapse_and_a_walk_goes_on_the_same() {
    let defs = defs(false);

    // Never saved.
    let (mut straight, realm, walker, idle) = first_half(&defs);
    let mid = straight.hashes();
    let told = second_half(&mut straight, walker);
    assert!(told.iter().any(|name| name == "collapse_settled"));
    assert!(!told.iter().any(|name| name == "fault"), "{told:?}");
    assert!(straight.shelf().is_empty());
    assert!(matches!(
        straight.truth().place(idle),
        Some(Place::At { .. })
    ));

    // Saved to a file in the middle, and loaded from it.
    let dir = scratch("midway");
    let path = dir.join("midway.c2save");
    let (mut first, _, _, _) = first_half(&defs);
    assert_eq!(first.hashes(), mid);
    let meta = SaveMeta::new("Midway")
        .summary("While the block comes down")
        .seed(5)
        .app(vec![1, 2, 3]);
    let report = save_game(&mut first, &path, &meta).unwrap();
    assert!(report.tracts > 0);
    assert!(report.world.tract_blobs > 0);
    assert_eq!(first.hashes(), mid, "saving changes nothing");

    let read = read_meta(&path).unwrap();
    assert_eq!(read.name, "Midway");
    assert_eq!(read.summary, "While the block comes down");
    assert_eq!(read.seed, 5);
    assert_eq!(read.app, [1, 2, 3]);
    assert_eq!(read.profile, "default");
    let mods: Vec<&str> = read.mods.iter().map(|stamp| stamp.id.as_str()).collect();
    assert_eq!(mods, ["core", "ui"]);
    assert_eq!(read.fingerprint, defs.fingerprint());
    assert_eq!(read.tick, first.now());
    assert_eq!(read.state_hash, mid.total);

    let generators = generators_for(&defs).unwrap();
    let (mut second, loaded_meta) =
        load_game(&path, Arc::clone(&defs), generators.clone()).unwrap();
    assert_eq!(loaded_meta, read);
    assert_eq!(second.hashes(), mid);
    assert!(load_warnings(&mut second, &loaded_meta).is_empty());
    assert!(second.shelf().of(idle).is_some());
    assert!(second.store().get(realm, START).unwrap().is_some());
    let told_again = second_half(&mut second, walker);
    assert_eq!(second.hashes(), straight.hashes());
    assert_eq!(told_again, told);

    // The one that was saved goes on the same too.
    let told_first = second_half(&mut first, walker);
    assert_eq!(first.hashes(), straight.hashes());
    assert_eq!(told_first, told);

    // The same through bytes, in memory.
    let (mut third, _, _, _) = first_half(&defs);
    let bytes = save_game_bytes(&mut third, &meta).unwrap();
    let (mut fourth, _) = load_game_bytes(&bytes, Arc::clone(&defs), generators.clone()).unwrap();
    assert_eq!(fourth.hashes(), mid);
    second_half(&mut fourth, walker);
    assert_eq!(fourth.hashes(), straight.hashes());

    // Saved again over the same file at the end: the file is the later
    // game, and it checks out.
    save_game(&mut second, &path, &SaveMeta::new("The end").seed(5)).unwrap();
    let verified = verify(&path, Arc::clone(&defs), generators.clone()).unwrap();
    assert_eq!(verified.meta.name, "The end");
    assert_eq!(verified.state_hash, straight.state_hash());
    assert!(verified.tracts > 0);
    assert!(verified.warnings.is_empty());

    // The directory lists it, and a file that is not a save beside it.
    std::fs::write(dir.join("junk.c2save"), b"not a save at all").unwrap();
    std::fs::write(dir.join("notes.txt"), b"ignored").unwrap();
    let listed = list_saves(&dir);
    assert_eq!(listed.len(), 2);
    assert!(listed[0].path.ends_with("junk.c2save"));
    assert!(listed[0].meta.is_err());
    assert_eq!(listed[1].meta.as_ref().unwrap().name, "The end");
    let _ = std::fs::remove_dir_all(&dir);
}

/// A small flat world with a few things changed in it.
fn flat(defs: &Arc<Defs>) -> Sim {
    let world = WorldDefs::from_defs(defs).unwrap_or_else(|d| panic!("{}", d.render()));
    let mut timeline = Timeline::new();
    let mut cosmos = Cosmos::new(Arc::clone(&world));
    let def = world.realms().lookup("core:testland").unwrap();
    let realm = cosmos.create_realm(&mut timeline, def, 3).unwrap();
    let mut sim = Sim::from_parts(Arc::clone(defs), cosmos, timeline, 9)
        .with_io(generators_for(defs).unwrap(), Box::new(MemoryStore::new()));
    sim.hold_ticket(clash_sim::debug_ticket(realm, TractPos::new(0, 0), 1))
        .unwrap();
    sim.reconcile_activity();
    sim.pump();
    sim
}

#[test]
fn a_save_made_with_a_mod_that_is_gone_loads_with_placeholders_and_says_so() {
    let with_extra = defs(true);
    let mut sim = flat(&with_extra);
    let crate_state = sim
        .truth()
        .cosmos()
        .defs()
        .block_state("extra:crate", None)
        .unwrap();
    let at = Location::new(GRID, CellPos::new(5, 5, 0));
    debug(
        &mut sim,
        DebugCommand::SetElement {
            loc: at,
            layer: Layer::Block,
            state: crate_state.raw(),
        },
    );
    let bytes = save_game_bytes(&mut sim, &SaveMeta::new("With a crate")).unwrap();

    // Without the mod.
    let plain = defs(false);
    let (mut loaded, meta) =
        load_game_bytes(&bytes, Arc::clone(&plain), generators_for(&plain).unwrap()).unwrap();
    let warnings = load_warnings(&mut loaded, &meta);
    assert!(
        warnings
            .iter()
            .any(|warning| warning.contains("`extra` 1.2.0, which is not loaded")),
        "{warnings:?}"
    );
    assert!(
        warnings
            .iter()
            .any(|warning| warning.contains("extra:crate") && warning.contains("placeholder")),
        "{warnings:?}"
    );
    let cosmos = loaded.truth().cosmos();
    let found = cosmos.cell(at).unwrap().block;
    assert!(!found.is_none(), "a placeholder stands where the crate was");
    assert!(
        cosmos
            .defs()
            .state_text(Layer::Block, found.raw())
            .starts_with("extra:crate")
    );

    // Saved without the mod and loaded with it again, the crate is back.
    let again = save_game_bytes(&mut loaded, &SaveMeta::new("Without")).unwrap();
    let (mut back, meta) = load_game_bytes(
        &again,
        Arc::clone(&with_extra),
        generators_for(&with_extra).unwrap(),
    )
    .unwrap();
    assert_eq!(back.truth().cosmos().cell(at).unwrap().block, crate_state);
    assert_eq!(back.state_hash(), sim.state_hash());
    let warnings = load_warnings(&mut back, &meta);
    assert_eq!(warnings.len(), 1, "{warnings:?}");
    assert!(warnings[0].contains("`extra` 1.2.0 is loaded and the save was made without it"));
}

#[test]
fn a_damaged_save_is_an_error_and_never_a_panic() {
    let defs = defs(false);
    let generators = generators_for(&defs).unwrap();
    let dir = scratch("damaged");

    // No file, and a file that is not a save.
    let missing = dir.join("missing.c2save");
    assert_eq!(
        read_meta(&missing).unwrap_err(),
        SaveError::NoSuchFile(missing.clone())
    );
    assert!(matches!(
        load_game(&missing, Arc::clone(&defs), generators.clone()),
        Err(SaveError::NoSuchFile(_))
    ));
    assert!(!missing.exists(), "looking for a save does not make one");
    let junk = dir.join("junk.c2save");
    std::fs::write(&junk, vec![7u8; 4096]).unwrap();
    assert!(matches!(read_meta(&junk), Err(SaveError::File { .. })));

    // A real save, with a changed tract in it.
    let mut sim = flat(&defs);
    let wall = sim
        .truth()
        .cosmos()
        .defs()
        .block_state("core:wall", None)
        .unwrap();
    debug(
        &mut sim,
        DebugCommand::SetElement {
            loc: Location::new(GRID, CellPos::new(3, 3, 0)),
            layer: Layer::Block,
            state: wall.raw(),
        },
    );
    let path = dir.join("game.c2save");
    save_game(&mut sim, &path, &SaveMeta::new("Whole")).unwrap();
    assert!(verify(&path, Arc::clone(&defs), generators.clone()).is_ok());

    // Cut short: an error, whatever it is.
    let whole = std::fs::read(&path).unwrap();
    let cut = dir.join("cut.c2save");
    std::fs::write(&cut, &whole[..whole.len() / 3]).unwrap();
    assert!(load_game(&cut, Arc::clone(&defs), generators.clone()).is_err());

    // A tract written over inside the file: named.
    let realm = RealmId::new(0);
    let tract = TractPos::new(0, 0);
    {
        let mut store = RedbStore::open(&path).unwrap();
        let mut blob = store.get(realm, tract).unwrap().expect("the changed tract");
        let last = blob.len() - 1;
        blob[last] ^= 0xff;
        store.put(realm, tract, &blob);
        store.commit().unwrap();
    }
    match verify(&path, Arc::clone(&defs), generators.clone()) {
        Err(SaveError::Tract {
            realm: found_realm,
            tract: found_tract,
            ..
        }) => assert_eq!((found_realm, found_tract), (realm, tract)),
        other => panic!("the damaged tract was not named: {other:?}"),
    }
    assert!(load_game(&path, Arc::clone(&defs), generators.clone()).is_err());

    // A section missing, and one written over.
    {
        let mut store = RedbStore::open(&path).unwrap();
        store.delete_section(clash_save::SIM_SECTION);
        store.commit().unwrap();
    }
    assert!(read_meta(&path).is_ok(), "the metadata still reads");
    assert!(load_game(&path, Arc::clone(&defs), generators.clone()).is_err());
    {
        let mut store = RedbStore::open(&path).unwrap();
        store.put_section(clash_save::META_SECTION, b"C2DBnonsense");
        store.commit().unwrap();
    }
    assert!(matches!(
        read_meta(&path),
        Err(SaveError::Section { name, .. }) if name == clash_save::META_SECTION
    ));

    // Bytes that are not a packed save.
    assert!(load_game_bytes(b"nope", Arc::clone(&defs), generators).is_err());
    assert_ne!(Hash128::default(), sim.state_hash());
    let _ = std::fs::remove_dir_all(&dir);
}
