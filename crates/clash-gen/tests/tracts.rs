//! Tests for W8: generation behind the interfaces of 02 §9.3, the two
//! test generators, and loading and unloading tracts against an
//! in-memory store (02 §9.4, §9.5).

mod common;

use std::sync::Arc;

use clash_core::{EntityId, Hash128, Rng, Tick};
use clash_gen::{GenError, RayonRunner, SerialRunner, realize_tracts};
use clash_time::Timeline;
use clash_world::dims::TRACT_XY;
use clash_world::{
    BlockState, Cause, CellBox, CellEdit, CellPos, Cosmos, EngineFlag, EventSink, Footprint,
    GridId, Layer, LoadError, Location, MemoryStore, MoveProfile, Occupant, OccupantKind,
    PathLimits, PendingEdit, RealmId, RevParts, RevStamp, SpawnRequest, TractIo, TractPos,
    TractState, WorldCtx, WorldDefs, WorldError, WorldEvent,
};

/// A world with one realm and nothing loaded.
struct World {
    defs: Arc<WorldDefs>,
    timeline: Timeline,
    cosmos: Cosmos,
    events: EventSink<WorldEvent>,
    realm: RealmId,
    grid: GridId,
}

fn world(realm_def: &str, seed: u64) -> World {
    let defs = common::core_defs();
    let mut timeline = Timeline::new();
    let mut cosmos = Cosmos::new(Arc::clone(&defs));
    let def = common::realm_def(&defs, realm_def);
    let realm = cosmos.create_realm(&mut timeline, def, seed).unwrap();
    let grid = cosmos.realm(realm).unwrap().base_grid;
    World {
        defs,
        timeline,
        cosmos,
        events: EventSink::new(),
        realm,
        grid,
    }
}

impl World {
    fn ctx(&mut self) -> WorldCtx<'_> {
        WorldCtx::new(&mut self.cosmos, &mut self.timeline, &mut self.events)
    }

    fn at(&self, x: i32, y: i32, z: i32) -> Location {
        Location::new(self.grid, CellPos::new(x, y, z))
    }

    fn column_hash(&self, tract: TractPos) -> Hash128 {
        let view = self.cosmos.grid(self.grid).unwrap();
        Hash128::of(view.column(tract).expect("the tract is loaded"))
    }
}

/// The generators with a pool of this many threads, or on the calling
/// thread for zero.
fn generators(defs: &Arc<WorldDefs>, threads: usize) -> clash_gen::Generators {
    let mut registry = clash_gen::registry();
    if threads == 0 {
        registry.set_runner(Arc::new(SerialRunner));
    } else {
        registry.set_runner(Arc::new(RayonRunner::with_threads(threads).unwrap()));
    }
    registry.bind(defs)
}

const SEED: u64 = 0xC17;

#[test]
fn realising_gives_the_same_tracts_in_any_order_and_on_any_thread_count() {
    let defs = common::core_defs();
    let def = common::realm_def(&defs, common::CITY_REALM);
    let tracts = common::block(4);
    let hashes = |threads: usize, order: &[TractPos]| -> Vec<(TractPos, Hash128)> {
        let generators = generators(&defs, threads);
        let realised = realize_tracts(&generators, def, SEED, order).unwrap();
        let mut hashes: Vec<(TractPos, Hash128)> = order
            .iter()
            .copied()
            .zip(realised.iter().map(|tract| Hash128::of(&tract.column)))
            .collect();
        hashes.sort_by_key(|(tract, _)| *tract);
        hashes
    };
    let straight = hashes(0, &tracts);
    // The tracts differ from one another: the test would pass on a
    // generator that ignored where it was.
    assert!(straight.windows(2).any(|pair| pair[0].1 != pair[1].1));

    let mut shuffled = tracts.clone();
    Rng::from_seed(5).shuffle(&mut shuffled);
    assert_ne!(shuffled, tracts);
    assert_eq!(hashes(0, &shuffled), straight);
    for threads in [1, 2, 7] {
        assert_eq!(hashes(threads, &tracts), straight, "{threads} threads");
        assert_eq!(hashes(threads, &shuffled), straight, "{threads} threads");
    }
    // One tract on its own is the tract it is in a batch.
    for (tract, hash) in &straight {
        assert_eq!(hashes(0, &[*tract]), [(*tract, *hash)]);
    }
    // Another seed is another city.
    let generators = generators(&defs, 0);
    let other = realize_tracts(&generators, def, SEED + 1, &tracts).unwrap();
    assert_ne!(Hash128::of(&other[0].column), straight[0].1);
}

#[test]
fn a_world_loaded_in_any_order_on_any_thread_count_hashes_the_same() {
    let tracts = common::block(3);
    let loaded = |threads: usize, order: &[TractPos]| -> Hash128 {
        let mut w = world(common::CITY_REALM, SEED);
        let generators = generators(&w.defs, threads);
        let mut store = MemoryStore::new();
        let mut io = TractIo::new(&generators, &mut store);
        let realm = w.realm;
        let order: Vec<(RealmId, TractPos)> = order.iter().map(|tract| (realm, *tract)).collect();
        for outcome in w.ctx().load_tracts(&mut io, &order) {
            assert!(outcome.unwrap().first);
        }
        Hash128::of(&w.cosmos)
    };
    let straight = loaded(0, &tracts);
    let mut shuffled = tracts.clone();
    Rng::from_seed(11).shuffle(&mut shuffled);
    assert_eq!(loaded(0, &shuffled), straight);
    assert_eq!(loaded(3, &tracts), straight);
    assert_eq!(loaded(5, &shuffled), straight);
    // Loading one at a time is the same again.
    let mut w = world(common::CITY_REALM, SEED);
    let generators = generators(&w.defs, 2);
    let mut store = MemoryStore::new();
    let mut io = TractIo::new(&generators, &mut store);
    let realm = w.realm;
    for tract in shuffled {
        w.ctx().load_tract(&mut io, realm, tract).unwrap();
    }
    assert_eq!(Hash128::of(&w.cosmos), straight);
}

#[test]
fn loading_unloading_and_loading_again_gives_the_same_cells() {
    let mut w = world(common::CITY_REALM, SEED);
    let generators = clash_gen::generators(&w.defs);
    let mut store = MemoryStore::new();
    let mut io = TractIo::new(&generators, &mut store);
    let (realm, tract) = (w.realm, TractPos::new(1, 2));

    let first = w.ctx().load_tract(&mut io, realm, tract).unwrap();
    assert!(first.first && !first.from_store);
    assert!(w.cosmos.is_tract_loaded(realm, tract));
    assert_eq!(
        w.cosmos.tract_record(realm, tract).unwrap().state,
        TractState::Realized { pristine: true }
    );
    let cells = w.column_hash(tract);
    assert_eq!(
        w.ctx().load_tract(&mut io, realm, tract),
        Err(LoadError::AlreadyLoaded { realm, tract })
    );

    // Unchanged: nothing is stored, and it is generated again.
    w.ctx().unload_tract(&mut io, realm, tract).unwrap();
    assert!(!w.cosmos.is_tract_loaded(realm, tract));
    assert_eq!(
        w.ctx().unload_tract(&mut io, realm, tract),
        Err(LoadError::NotLoaded { realm, tract })
    );
    assert_eq!(io.store.get(realm, tract), Ok(None));
    let again = w.ctx().load_tract(&mut io, realm, tract).unwrap();
    assert!(!again.first && !again.from_store);
    assert_eq!(w.column_hash(tract), cells);

    // Changed: it is stored whole and read back, change and all.
    let origin = tract.origin(0);
    let spot = w.at(origin.x + 1, origin.y + 1, 0);
    let before = w.cosmos.cell(spot).unwrap();
    assert!(before.block.is_none());
    let wall = w.defs.block_state("wall", None).unwrap();
    w.ctx().edit(Cause::DEBUG).set_block(spot, wall).unwrap();
    let corner = w.at(origin.x, origin.y, 0);
    w.ctx()
        .edit(Cause::DEBUG)
        .set_damage(corner, Layer::Floor, 9)
        .unwrap();
    let changed = w.column_hash(tract);
    assert_ne!(changed, cells);
    w.ctx().unload_tract(&mut io, realm, tract).unwrap();
    assert!(io.store.get(realm, tract).unwrap().is_some());
    let record = w.cosmos.tract_record(realm, tract).unwrap();
    assert_eq!(record.state, TractState::Realized { pristine: false });
    let stored = w.ctx().load_tract(&mut io, realm, tract).unwrap();
    assert!(stored.from_store && !stored.first);
    assert_eq!(w.column_hash(tract), changed);
    assert_eq!(w.cosmos.cell(spot).unwrap().floor, before.floor);
    assert_eq!(w.cosmos.cell(spot).unwrap().block, wall);
    assert_eq!(w.cosmos.damage(corner).unwrap()[0], 9);

    // Loaded from the store and not touched, it is not written again.
    let blob = io.store.get(realm, tract).unwrap();
    w.ctx().unload_tract(&mut io, realm, tract).unwrap();
    assert_eq!(io.store.get(realm, tract).unwrap(), blob);

    // A blob that has gone missing or been damaged is an error, and the
    // tract stays unloaded.
    io.store.put(realm, tract, b"not a tract");
    assert!(matches!(
        w.ctx().load_tract(&mut io, realm, tract),
        Err(LoadError::Store(_))
    ));
    io.store.delete(realm, tract);
    assert_eq!(
        w.ctx().load_tract(&mut io, realm, tract),
        Err(LoadError::NotStored { realm, tract })
    );
    assert!(!w.cosmos.is_tract_loaded(realm, tract));
}

#[test]
fn loading_and_unloading_say_so_and_note_the_time() {
    let mut w = world(common::FLAT_REALM, 3);
    let generators = clash_gen::generators(&w.defs);
    let mut store = MemoryStore::new();
    let mut io = TractIo::new(&generators, &mut store);
    let (realm, tract) = (w.realm, TractPos::new(4, 5));
    w.timeline.advance_to(Tick(1_000));
    w.ctx().load_tract(&mut io, realm, tract).unwrap();
    w.timeline.advance_to(Tick(5_000));
    w.ctx().unload_tract(&mut io, realm, tract).unwrap();
    let loc = Location::new(w.grid, tract.origin(0));
    assert_eq!(
        w.events.drain(),
        [
            WorldEvent::TractLoaded {
                tick: Tick(1_000),
                realm,
                loc,
                tract,
                first: true
            },
            WorldEvent::TractUnloading {
                tick: Tick(5_000),
                realm,
                loc,
                tract
            },
        ]
    );
    let extra = w.cosmos.tract_record(realm, tract).unwrap().extra.clone();
    assert_eq!(extra.unwrap().last_loaded, Tick(5_000));
    // The flat generator's ground is what the realm's fill says.
    w.ctx().load_tract(&mut io, realm, tract).unwrap();
    let ground = w
        .cosmos
        .cell(Location::new(w.grid, tract.origin(-1)))
        .unwrap();
    assert_eq!(
        ground.block,
        w.defs.block_state("solid@granite", None).unwrap()
    );
    assert!(w.cosmos.cell(loc).unwrap().is_empty());
    assert!(w.cosmos.standable(loc).unwrap());
    // A tract beyond the realm's edge is refused, and one round the
    // wrapped axis is the tract it wraps to.
    assert!(matches!(
        w.ctx().load_tract(&mut io, realm, TractPos::new(0, -1)),
        Err(LoadError::World(WorldError::OutsideRealm { .. }))
    ));
    let round = w
        .ctx()
        .load_tract(&mut io, realm, TractPos::new(-1, 0))
        .unwrap();
    assert_eq!(round.tract.y, 0);
    assert!(round.tract.x > 0);
}

#[test]
fn unloading_a_tract_that_still_has_an_occupant_is_refused() {
    let mut w = world(common::FLAT_REALM, 3);
    let generators = clash_gen::generators(&w.defs);
    let mut store = MemoryStore::new();
    let mut io = TractIo::new(&generators, &mut store);
    let (realm, tract) = (w.realm, TractPos::new(2, 2));
    w.ctx().load_tract(&mut io, realm, tract).unwrap();
    let someone = EntityId::new(7, 0);
    let origin = tract.origin(0);
    // High above the stored chunks, where the index is kept beside them.
    let up = w.at(origin.x + 3, origin.y + 3, 90);
    w.ctx()
        .place(
            Occupant::new(someone, OccupantKind::Creature),
            up,
            Footprint::SINGLE,
            Cause::DEBUG,
        )
        .unwrap();
    w.events.clear();
    let before = Hash128::of(&w.cosmos);
    assert_eq!(
        w.ctx().unload_tract(&mut io, realm, tract),
        Err(LoadError::Occupied {
            realm,
            tract,
            occupant: someone
        })
    );
    // Refused means nothing changed and nothing was said.
    assert!(w.cosmos.is_tract_loaded(realm, tract));
    assert!(w.cosmos.occupant(someone).is_some());
    assert_eq!(Hash128::of(&w.cosmos), before);
    assert!(w.events.is_empty());
    assert!(io.store.get(realm, tract).unwrap().is_none());
    // With the occupant gone it unloads.
    w.ctx().remove(someone, Cause::DEBUG).unwrap();
    w.ctx().unload_tract(&mut io, realm, tract).unwrap();
    assert!(!w.cosmos.is_tract_loaded(realm, tract));
}

#[test]
fn a_tract_loaded_again_does_not_look_unchanged_to_a_cache() {
    let mut w = world(common::CITY_REALM, SEED);
    let generators = clash_gen::generators(&w.defs);
    let mut store = MemoryStore::new();
    let mut io = TractIo::new(&generators, &mut store);
    let (realm, tract) = (w.realm, TractPos::new(0, 0));
    w.ctx().load_tract(&mut io, realm, tract).unwrap();

    // A cache that read the tract's cells, stored chunks and the empty
    // air above them alike.
    let cells = CellBox::new(
        CellPos::new(0, 0, 0),
        CellPos::new(TRACT_XY - 1, TRACT_XY - 1, 40),
    );
    let grid = w.grid;
    let stamp_of = |cosmos: &Cosmos| {
        let mut stamp = RevStamp::new(RevParts::STRUCTURE);
        stamp.record_box(&cosmos.grid(grid).unwrap(), cells);
        stamp
    };
    let stamp = stamp_of(&w.cosmos);
    assert!(stamp.is_current(&w.cosmos));

    // Unloaded and loaded again without the cache looking in between:
    // the cells are the same, and the cache must still not trust itself.
    let cells_before = w.column_hash(tract);
    w.ctx().unload_tract(&mut io, realm, tract).unwrap();
    w.ctx().load_tract(&mut io, realm, tract).unwrap();
    assert_eq!(w.column_hash(tract), cells_before);
    assert!(!stamp.is_current(&w.cosmos));

    // And again: each loading starts where no earlier one stood, even
    // one that was itself never written to.
    let second = stamp_of(&w.cosmos);
    assert!(second.is_current(&w.cosmos));
    w.ctx().unload_tract(&mut io, realm, tract).unwrap();
    // A change made while nobody was looking.
    let wall = w.cosmos.defs().block_state("wall", None).unwrap();
    w.cosmos
        .add_pending_edit(
            realm,
            tract,
            PendingEdit::Cells(vec![CellEdit {
                cell: CellPos::new(1, 1, 0),
                layer: Layer::Block,
                state: wall.raw(),
            }]),
        )
        .unwrap();
    w.ctx().load_tract(&mut io, realm, tract).unwrap();
    assert_ne!(w.column_hash(tract), cells_before);
    assert!(!stamp.is_current(&w.cosmos));
    assert!(!second.is_current(&w.cosmos));

    // A stamp taken now is current until the next edit, as ever.
    let third = stamp_of(&w.cosmos);
    assert!(third.is_current(&w.cosmos));
    let spot = w.at(1, 1, 0);
    w.ctx()
        .edit(Cause::DEBUG)
        .set_block(spot, BlockState::NONE)
        .unwrap();
    assert!(!third.is_current(&w.cosmos));
}

#[test]
fn changes_waiting_for_a_tract_are_made_when_it_loads() {
    let mut w = world(common::FLAT_REALM, 3);
    let generators = clash_gen::generators(&w.defs);
    let mut store = MemoryStore::new();
    let mut io = TractIo::new(&generators, &mut store);
    let (realm, tract) = (w.realm, TractPos::new(1, 1));
    let origin = tract.origin(0);
    let wall = w.defs.block_state("wall", None).unwrap();
    let change = |dx: i32| CellEdit {
        cell: CellPos::new(origin.x + dx, origin.y, 0),
        layer: Layer::Block,
        state: wall.raw(),
    };

    // No record yet: the region has not been surveyed.
    assert_eq!(
        w.cosmos
            .add_pending_edit(realm, tract, PendingEdit::Cells(vec![change(0)])),
        Err(WorldError::NotSurveyed { realm, tract })
    );
    assert_eq!(
        w.cosmos.survey_region(&generators, realm, tract.region()),
        Ok(true)
    );
    assert_eq!(
        w.cosmos.survey_region(&generators, realm, tract.region()),
        Ok(false)
    );
    // A cell of another tract is refused.
    assert!(matches!(
        w.cosmos
            .add_pending_edit(realm, tract, PendingEdit::Cells(vec![change(-1)])),
        Err(WorldError::BadValue { .. })
    ));
    w.cosmos
        .add_pending_edit(realm, tract, PendingEdit::Cells(vec![change(0), change(1)]))
        .unwrap();
    let spawn = SpawnRequest {
        what: "core:someone".to_owned(),
        at: origin,
        params: Default::default(),
    };
    w.cosmos
        .add_pending_edit(realm, tract, PendingEdit::Spawn(spawn.clone()))
        .unwrap();
    assert_eq!(
        w.cosmos.tract_record(realm, tract).unwrap().pending().len(),
        2
    );

    let loaded = w.ctx().load_tract(&mut io, realm, tract).unwrap();
    assert_eq!(loaded.spawns, [spawn]);
    assert!(
        w.cosmos
            .tract_record(realm, tract)
            .unwrap()
            .pending()
            .is_empty()
    );
    assert_eq!(
        w.cosmos.cell(w.at(origin.x, origin.y, 0)).unwrap().block,
        wall
    );
    assert_eq!(
        w.cosmos
            .cell(w.at(origin.x + 1, origin.y, 0))
            .unwrap()
            .block,
        wall
    );
    // Made through the one door: two element events with the coarse
    // cause, then the tract is announced.
    let events = w.events.drain();
    let names: Vec<&str> = events.iter().map(WorldEvent::name).collect();
    assert_eq!(
        names,
        ["element_changed", "element_changed", "tract_loaded"]
    );
    assert_eq!(events[0].cause().kind, clash_world::CauseKind::Coarse);
    // A loaded tract is edited, not sent changes for later.
    assert!(matches!(
        w.cosmos
            .add_pending_edit(realm, tract, PendingEdit::Cells(vec![change(2)])),
        Err(WorldError::BadValue { .. })
    ));
    // The change was made once: it is part of the stored tract now.
    w.ctx().unload_tract(&mut io, realm, tract).unwrap();
    assert!(io.store.get(realm, tract).unwrap().is_some());
    let again = w.ctx().load_tract(&mut io, realm, tract).unwrap();
    assert!(again.from_store && again.spawns.is_empty());
    assert_eq!(
        w.cosmos.cell(w.at(origin.x, origin.y, 0)).unwrap().block,
        wall
    );
}

#[test]
fn a_generator_that_cannot_be_had_is_an_error_that_says_where() {
    let defs = common::core_defs();
    let generators = clash_gen::generators(&defs);
    // `mods/core` builds: nothing is wrong with its parameters.
    assert_eq!(generators.param_errors().count(), 0);
    // A realm whose generator nobody registered is refused when used.
    let room = common::realm_def(&defs, "core:training_room");
    assert_eq!(
        generators.of(room).err(),
        Some(GenError::UnknownGenerator {
            realm: "core:training_room".to_owned(),
            name: "white_void".to_owned(),
        })
    );
    let mut timeline = Timeline::new();
    let mut cosmos = Cosmos::new(Arc::clone(&defs));
    let realm = cosmos.create_realm(&mut timeline, room, 1).unwrap();
    let mut events = EventSink::new();
    let mut store = MemoryStore::new();
    let mut io = TractIo::new(&generators, &mut store);
    let refused = WorldCtx::new(&mut cosmos, &mut timeline, &mut events).load_tract(
        &mut io,
        realm,
        TractPos::new(0, 0),
    );
    assert!(matches!(
        refused,
        Err(LoadError::Gen(GenError::UnknownGenerator { .. }))
    ));
    assert!(events.is_empty());
    assert!(cosmos.region(realm, TractPos::new(0, 0).region()).is_none());

    // Mistakes in a realm's parameters, each named by its path. The
    // town is the test city with one parameter changed or added.
    let city = common::realm_def(&defs, common::CITY_REALM);
    let good = defs.realms().get(city).unwrap().generator_params.clone();
    let bad = |param: &str, value: serde_json::Value| -> GenError {
        let mut params = good.clone();
        params.insert(param.to_owned(), value);
        let params = serde_json::to_string(&params).unwrap();
        let text = format!(
            r#"[ {{ "type": "realm", "id": "town", "copy_from": "core:test_city",
                   "generator_params": {params} }} ]"#
        );
        let defs = common::try_defs_with(&[("data/town.json", &text)]).unwrap();
        let generators = clash_gen::generators(&defs);
        let town = common::realm_def(&defs, "extra:town");
        let error = generators.of(town).expect_err("the parameters are wrong");
        assert_eq!(generators.param_errors().next(), Some(&error));
        error
    };
    let expect = |error: GenError, path: &str, says: &str| match error {
        GenError::BadParam {
            realm,
            path: found,
            message,
        } => {
            assert_eq!(realm, "extra:town");
            assert_eq!(found, path);
            assert!(message.contains(says), "{message}");
        }
        other => panic!("not a parameter error: {other}"),
    };
    use serde_json::json;
    expect(
        bad("wall", json!("no_such_block")),
        "generator_params.wall",
        "there is no block",
    );
    expect(
        bad("desk", json!(4)),
        "generator_params.desk",
        "state of a fixture",
    );
    expect(
        bad("storeys", json!([3, 2])),
        "generator_params.storeys",
        "must not be above",
    );
    expect(
        bad("storeys", json!([2, 90])),
        "generator_params.storeys",
        "must be from 1 to 40",
    );
    expect(
        bad("street_spacing", json!(9)),
        "generator_params.street_spacing",
        "a building needs 7",
    );
    expect(
        bad("colum_spacing", json!(4)),
        "generator_params.colum_spacing",
        "no such parameter",
    );
}

/// How many cells of a storey of a block of the city satisfy a test.
fn count(
    w: &World,
    side: i32,
    z: i32,
    mut test: impl FnMut(clash_world::CellView) -> bool,
) -> usize {
    let mut found = 0;
    for y in 0..side * TRACT_XY {
        for x in 0..side * TRACT_XY {
            if test(w.cosmos.cell(w.at(x, y, z)).unwrap()) {
                found += 1;
            }
        }
    }
    found
}

#[test]
fn the_city_has_what_a_city_needs_and_can_be_walked() {
    let mut w = world(common::CITY_REALM, 1);
    let generators = clash_gen::generators(&w.defs);
    let mut store = MemoryStore::new();
    let mut io = TractIo::new(&generators, &mut store);
    let realm = w.realm;
    let order: Vec<(RealmId, TractPos)> = common::block(2)
        .into_iter()
        .map(|tract| (realm, tract))
        .collect();
    for outcome in w.ctx().load_tracts(&mut io, &order) {
        outcome.unwrap();
    }
    let defs = Arc::clone(&w.defs);
    let floor = |name: &str| defs.floor_state(name, None).unwrap();
    let block = |name: &str| defs.block_state(name, None).unwrap();
    let desk = defs.fixture_state("desk", None).unwrap();

    // Sixteen blocks of the street grid, each with a building.
    assert!(count(&w, 2, 0, |cell| cell.floor == floor("asphalt")) > 1000);
    assert!(count(&w, 2, 0, |cell| cell.floor == floor("pavement")) > 300);
    assert_eq!(count(&w, 2, 0, |cell| cell.block == block("stairs")), 16);
    let doors = count(&w, 2, 0, |cell| cell.block == block("door"));
    assert!(
        (16 + 4 * 16..=2 * 16 + 4 * 16).contains(&doors),
        "{doors} doors"
    );
    assert!(count(&w, 2, 0, |cell| cell.block == block("window")) > 16 * 12);
    assert!(count(&w, 2, 0, |cell| cell.block == block("pillar")) > 16 * 2);
    assert!(count(&w, 2, 0, |cell| cell.block == block("wall@brick")) > 16 * 8);
    let desks = count(&w, 2, 0, |cell| cell.fixture == desk);
    assert!((16..16 * 8).contains(&desks), "{desks} desks");
    // Every building has at least two storeys and a roof no higher than
    // the fifth, and nothing stands above that.
    assert!(count(&w, 2, 1, |cell| cell.block == block("wall")) >= 16 * 4);
    assert!(count(&w, 2, 2, |cell| cell.floor == floor("roof")) > 0);
    assert_eq!(count(&w, 2, 6, |cell| !cell.is_empty()), 0);
    // The stairs are stairs to the engine, and the streets stay on
    // the ground.
    assert!(
        defs.block_info(block("stairs"))
            .flags
            .has(EngineFlag::Stairs)
    );
    assert_eq!(count(&w, 2, 1, |cell| cell.floor == floor("asphalt")), 0);

    // Someone who opens doors can walk from the street corner to the
    // top storey of the first building: in at a door, up the stairs.
    let walker = MoveProfile {
        opens: true,
        ..MoveProfile::walker()
    };
    let roof = (1..6)
        .rev()
        .find(|z| !w.cosmos.cell(w.at(9, 9, *z)).unwrap().is_empty())
        .expect("the first building has a roof");
    let from = w.at(0, 0, 0);
    // A free cell of the top storey: floor, and nothing standing on it.
    let to = (5..14)
        .flat_map(|y| (5..14).map(move |x| (x, y)))
        .map(|(x, y)| w.at(x, y, roof - 1))
        .find(|loc| {
            let cell = w.cosmos.cell(*loc).unwrap();
            !cell.floor.is_none() && cell.block.is_none() && cell.fixture.is_none()
        })
        .expect("the top storey has free floor");
    let path = w
        .cosmos
        .find_path(from, to, &walker, PathLimits::DEFAULT)
        .expect("the top storey can be reached");
    assert!(path.cells().iter().any(|cell| {
        w.cosmos.cell(Location::new(w.grid, *cell)).unwrap().block == block("door")
    }));
    assert!(path.cells().iter().any(|cell| {
        w.cosmos.cell(Location::new(w.grid, *cell)).unwrap().block == block("stairs")
    }));
    // Without opening doors there is no way in.
    assert!(
        w.cosmos
            .find_path(from, to, &MoveProfile::walker(), PathLimits::DEFAULT)
            .is_err()
    );
}

#[test]
fn the_printed_city_is_the_stored_one() {
    let text = common::city_text(1, 0).unwrap();
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/golden/city_seed1_storey0.txt");
    if std::env::var_os("BLESS").is_some_and(|value| value == "1") {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, &text).unwrap();
        return;
    }
    let stored = std::fs::read_to_string(&path).unwrap_or_else(|error| {
        panic!(
            "no stored map at {}: {error}\nrun the test with BLESS=1 to write it, then check \
             it by eye",
            path.display()
        )
    });
    for (line, (stored, now)) in stored.lines().zip(text.lines()).enumerate() {
        assert_eq!(
            stored,
            now,
            "the map differs from {} at line {}\nrun with BLESS=1 to accept the change, and \
             say why in the change's description",
            path.display(),
            line + 1
        );
    }
    assert_eq!(stored.lines().count(), text.lines().count());
    // The other storeys and seeds print too, and differ.
    assert_ne!(common::city_text(1, 1).unwrap(), text);
    assert_ne!(common::city_text(2, 0).unwrap(), text);
}

/// The best of a few passes of a piece of work, in microseconds.
#[allow(
    clippy::disallowed_types,
    clippy::disallowed_methods,
    reason = "a timing test reads the wall clock; nothing in the simulation does"
)]
fn best_micros(mut work: impl FnMut()) -> f64 {
    let mut best = f64::INFINITY;
    for _ in 0..7 {
        let started = std::time::Instant::now();
        work();
        best = best.min(started.elapsed().as_nanos() as f64 / 1000.0);
    }
    best
}

/// Not a benchmark with a budget: rough figures for the budgets of
/// 02 §25, printed. Run with
/// `cargo test --release -p clash-gen --test tracts -- --ignored --nocapture --test-threads=1`.
#[test]
#[ignore = "timing; run by hand in release mode"]
fn timing_of_realising_and_loading() {
    let defs = common::core_defs();
    let def = common::realm_def(&defs, common::CITY_REALM);
    let tracts = common::block(8);
    for threads in [0, 1, 4] {
        let generators = generators(&defs, threads);
        let micros = best_micros(|| {
            let realised = realize_tracts(&generators, def, SEED, &tracts).unwrap();
            std::hint::black_box(realised);
        });
        println!(
            "realise a test_city tract, {threads} threads: {:.0} µs a tract ({} tracts)",
            micros / tracts.len() as f64,
            tracts.len()
        );
    }
    // Loading a stored tract: every tract of the block changed, unloaded
    // and read back.
    let mut w = world(common::CITY_REALM, SEED);
    let generators = generators(&w.defs, 0);
    let mut store = MemoryStore::new();
    let mut io = TractIo::new(&generators, &mut store);
    let realm = w.realm;
    let order: Vec<(RealmId, TractPos)> = tracts.iter().map(|tract| (realm, *tract)).collect();
    w.ctx().load_tracts(&mut io, &order);
    let wall = w.defs.block_state("wall", None).unwrap();
    for tract in &tracts {
        let spot = Location::new(w.grid, tract.origin(0));
        w.ctx().edit(Cause::DEBUG).set_block(spot, wall).unwrap();
    }
    let unload = best_micros(|| {
        for tract in &tracts {
            w.ctx().unload_tract(&mut io, realm, *tract).unwrap();
        }
        let loaded = w.ctx().load_tracts(&mut io, &order);
        assert!(
            loaded
                .iter()
                .all(|outcome| outcome.as_ref().is_ok_and(|l| l.from_store))
        );
        for tract in &tracts {
            let spot = Location::new(w.grid, tract.origin(1));
            w.ctx().edit(Cause::DEBUG).set_block(spot, wall).unwrap();
            w.ctx()
                .edit(Cause::DEBUG)
                .set_block(spot, BlockState::NONE)
                .unwrap();
        }
    });
    println!(
        "store and load a changed test_city tract: {:.0} µs a tract",
        unload / tracts.len() as f64
    );
    let bytes: usize = tracts
        .iter()
        .map(|tract| {
            io.store
                .get(realm, *tract)
                .unwrap()
                .map_or(0, |blob| blob.len())
        })
        .sum();
    println!("a stored test_city tract: {} bytes", bytes / tracts.len());
}
