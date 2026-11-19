//! Tests for W5: realms and their definitions, the cosmos, strata and
//! ambient conditions, and the read API with its unloaded error.

mod common;

use std::sync::Arc;

use clash_core::{Hash128, IVec3, Real};
use clash_time::{ClockId, Timeline};
use clash_world::defs::{EdgeRule, RealmDef};
use clash_world::dims::{REGION_CELLS, TRACT_XY};
use clash_world::test_support::{flat_column, load_column, load_flat, unload_column};
use clash_world::{
    BlockState, CellFlags, CellPos, ColumnBuilder, Cosmos, Edge, Extent, FieldId, FixtureState,
    GridId, Location, RealmId, RealmState, TractPos, WorldDefs, WorldError,
};

struct World {
    defs: Arc<WorldDefs>,
    timeline: Timeline,
    cosmos: Cosmos,
    realm: RealmId,
    grid: GridId,
}

/// A cosmos with one `testland` realm and nothing loaded.
fn world() -> World {
    let defs = common::test_defs();
    let mut timeline = Timeline::new();
    let mut cosmos = Cosmos::new(Arc::clone(&defs));
    let def = defs.realms().lookup("core:testland").unwrap();
    let realm = cosmos.create_realm(&mut timeline, def, 42).unwrap();
    let grid = cosmos.realm(realm).unwrap().base_grid;
    World {
        defs,
        timeline,
        cosmos,
        realm,
        grid,
    }
}

/// Loads flat rock with grass on storey 0 into a tract.
fn load_ground(world: &mut World, tract: TractPos) {
    let rock = world.defs.block_state("solid", None).unwrap();
    let grass = world.defs.floor_state("grass", None).unwrap();
    load_flat(&mut world.cosmos, world.grid, [tract], rock, grass, 0).unwrap();
}

#[test]
fn a_realm_definition_reads_as_written() {
    let defs = common::test_defs();
    let id = defs.realms().lookup("core:testland").unwrap();
    let kind = defs.realms().get(id).unwrap();
    assert_eq!(kind.topology.x, Extent::Wrapped(2 * REGION_CELLS));
    assert_eq!(kind.topology.y, Extent::Bounded(REGION_CELLS));
    assert_eq!((kind.topology.z_min, kind.topology.z_max), (-16, 200));
    assert_eq!(
        kind.below_fill,
        defs.block_state("solid@granite", None).unwrap()
    );
    assert_eq!(kind.above_fill, BlockState::NONE);
    assert_eq!(kind.strata.len(), 3);
    assert_eq!(kind.time_rate, Real::ONE);
    assert_eq!(*kind.edge(Edge::East), EdgeRule::Wrapped);
    assert_eq!(*kind.edge(Edge::West), EdgeRule::Wrapped);
    assert_eq!(*kind.edge(Edge::North), EdgeRule::Barrier);
    assert_eq!(*kind.edge(Edge::South), EdgeRule::Void);
    assert_eq!(*kind.edge(Edge::Top), EdgeRule::Void);
    assert_eq!(*kind.edge(Edge::Bottom), EdgeRule::Barrier);

    // The larger example of 02 §4.1 reads too, with its boundary link.
    let earth: RealmDef = serde_json::from_str(
        r#"{
          "size_regions": [512, 256],
          "wrap": ["x"],
          "z_range": [-256, 140000],
          "generator": "earthlike",
          "fill": { "below": "solid@granite", "above": null },
          "strata": [
            { "from_z": -256,  "gravity": 9.81, "atmosphere": "air",      "sky": "day_night" },
            { "from_z": 4000,  "gravity": 9.81, "atmosphere": "thin_air", "sky": "high_altitude", "temp_offset": -40 },
            { "from_z": 33000, "gravity": 0.0,  "atmosphere": null,       "sky": "orbit", "temp": "-120 C" }
          ],
          "time_rate": 1.0,
          "edges": {
            "top": { "exit": "chart", "chart": "sol", "node": "earth" },
            "bottom": "barrier",
            "north": "barrier",
            "south": "barrier"
          }
        }"#,
    )
    .unwrap();
    let mut raw = common::test_raw();
    raw.realms.push(("test:earth".to_owned(), earth));
    let defs = WorldDefs::build(raw).unwrap();
    let earth = defs
        .realms()
        .get(defs.realms().lookup("test:earth").unwrap())
        .unwrap();
    assert!(
        matches!(earth.edge(Edge::Top), EdgeRule::Link(link) if link.node.as_deref() == Some("earth"))
    );
    assert_eq!(earth.topology.x, Extent::Wrapped(512 * REGION_CELLS));
    assert_eq!(earth.strata[2].temp, Some(Real::of(-120.0 + 273.15)));
    assert_eq!(earth.strata[1].temp_offset, Some(Real::of(-40.0)));
}

#[test]
fn mistakes_in_a_realm_definition_are_reported() {
    let mut raw = common::test_raw();
    common::add_json(
        &mut raw,
        "bad",
        r#"[
          { "type": "realm", "id": "nowhere", "size_regions": [0, 3000000], "wrap": ["x"],
            "z_range": [10, -10], "fill": { "below": "bedrock" },
            "strata": [ { "from_z": 0, "gravity": 1, "atmosphere": "ether" },
                        { "from_z": 0, "gravity": 2 } ],
            "time_rate": -1, "edges": { "east": "void" } },
          { "type": "realm", "id": "bare", "size_regions": [1, 1], "z_range": [0, 1], "strata": [] }
        ]"#,
        "inline",
    );
    let errors: Vec<String> = WorldDefs::build(raw)
        .unwrap_err()
        .iter()
        .map(ToString::to_string)
        .collect();
    let has = |needle: &str| errors.iter().any(|error| error.contains(needle));
    assert!(
        has("realm `bad:nowhere`, size_regions.0: must be from 1 to"),
        "{errors:#?}"
    );
    assert!(
        has("realm `bad:nowhere`, size_regions.1: must be from 1 to"),
        "{errors:#?}"
    );
    assert!(
        has("realm `bad:nowhere`, z_range: the lowest storey comes first"),
        "{errors:#?}"
    );
    assert!(
        has("realm `bad:nowhere`, fill.below: there is no block `bedrock`"),
        "{errors:#?}"
    );
    assert!(
        has("realm `bad:nowhere`, strata.0.atmosphere: there is no atmosphere `ether`"),
        "{errors:#?}"
    );
    assert!(
        has("realm `bad:nowhere`, strata: two strata start at the same storey"),
        "{errors:#?}"
    );
    assert!(
        has("realm `bad:nowhere`, time_rate: must be zero or more"),
        "{errors:#?}"
    );
    assert!(
        has("realm `bad:nowhere`, edges.east: this axis wraps, so it has no edge"),
        "{errors:#?}"
    );
    assert!(
        has("realm `bad:bare`, strata: a realm needs at least one stratum"),
        "{errors:#?}"
    );
}

#[test]
fn creating_a_realm_gives_it_a_clock_and_a_grid() {
    let mut world = world();
    let realm = world.cosmos.realm(world.realm).unwrap().clone();
    assert_eq!(realm.seed, 42);
    assert_eq!(realm.state, RealmState::Active);
    assert_eq!(world.cosmos.realm_of(realm.base_grid), Some(realm.id));
    assert_eq!(world.timeline.parent(realm.clock), Some(ClockId::UNIVERSAL));
    assert_eq!(world.timeline.rate(realm.clock), Some(Real::ONE));

    // A second realm runs on its own clock at its own rate.
    let def = world.defs.realms().lookup("core:training_room").unwrap();
    let room = world
        .cosmos
        .create_realm(&mut world.timeline, def, 7)
        .unwrap();
    let room = world.cosmos.realm(room).unwrap().clone();
    assert_ne!(room.id, realm.id);
    assert_ne!(room.base_grid, realm.base_grid);
    assert_ne!(room.clock, realm.clock);
    assert_eq!(world.timeline.rate(room.clock), Some(Real::of(365.0)));
    assert_eq!(
        world
            .cosmos
            .realms()
            .map(|realm| realm.id)
            .collect::<Vec<_>>(),
        [realm.id, room.id]
    );
    let ambient = world
        .cosmos
        .ambient(Location::new(room.base_grid, CellPos::new(1, 1, 1)))
        .unwrap();
    assert_eq!(ambient.gravity, Real::of(98.1));
    assert_eq!(ambient.clock, room.clock);

    // A definition that does not exist is refused and changes nothing.
    let before = Hash128::of(&world.cosmos);
    let missing = clash_world::RealmDefId::new(999);
    assert_eq!(
        world.cosmos.create_realm(&mut world.timeline, missing, 0),
        Err(WorldError::UnknownRealmDef(missing))
    );
    assert_eq!(Hash128::of(&world.cosmos), before);
}

#[test]
fn positions_wrap_or_name_the_edge_they_cross() {
    let world = world();
    let at = |x, y, z| Location::new(world.grid, CellPos::new(x, y, z));
    let width = 2 * REGION_CELLS;
    // East and west meet.
    assert_eq!(
        world.cosmos.normalize(at(-1, 5, 0)),
        Ok(at(width - 1, 5, 0))
    );
    assert_eq!(world.cosmos.normalize(at(width, 5, 0)), Ok(at(0, 5, 0)));
    assert_eq!(
        world.cosmos.delta(at(width - 2, 0, 0), at(3, 0, 0)),
        Some(IVec3::new(5, 0, 0))
    );
    // North, south, top and bottom do not.
    let edge = |loc: Location| match world.cosmos.normalize(loc) {
        Err(WorldError::OutsideRealm { edge, loc: given }) => {
            assert_eq!(given, loc);
            Some(edge)
        }
        _ => None,
    };
    assert_eq!(edge(at(0, -1, 0)), Some(Edge::North));
    assert_eq!(edge(at(0, REGION_CELLS, 0)), Some(Edge::South));
    assert_eq!(edge(at(0, 0, -17)), Some(Edge::Bottom));
    assert_eq!(edge(at(0, 0, 201)), Some(Edge::Top));
    assert_eq!(edge(at(0, 0, 200)), None);
    // Every read normalises first: beyond an edge is an error value.
    assert!(matches!(
        world.cosmos.flags(at(0, -1, 0)),
        Err(WorldError::OutsideRealm {
            edge: Edge::North,
            ..
        })
    ));
    let message = world.cosmos.cell(at(0, 0, 201)).unwrap_err().to_string();
    assert_eq!(
        message,
        format!(
            "grid {} (0, 0, 201) is outside the realm, beyond its top edge",
            world.grid
        )
    );
    // Locations in different grids have no offset between them.
    assert_eq!(
        world.cosmos.delta(
            at(0, 0, 0),
            Location::new(GridId::new(77), CellPos::new(0, 0, 0))
        ),
        None
    );
}

#[test]
fn strata_change_with_height_and_nothing_else() {
    let world = world();
    let air = world.defs.atmospheres().lookup("core:air");
    let thin = world.defs.atmospheres().lookup("core:thin_air");
    let clock = world.cosmos.realm(world.realm).unwrap().clock;
    let ambient = |z| {
        world
            .cosmos
            .ambient(Location::new(world.grid, CellPos::new(100, 100, z)))
            .unwrap()
    };
    // Nothing is loaded: ambient conditions need no cells.
    assert_eq!(ambient(-16).atmosphere, air);
    assert_eq!(ambient(0).gravity, Real::of(9.81));
    assert_eq!(ambient(99).atmosphere, air);
    assert_eq!(ambient(100).atmosphere, thin);
    assert_eq!(ambient(149).atmosphere, thin);
    assert_eq!(ambient(149).gravity, Real::of(9.81));
    assert_eq!(ambient(150).atmosphere, None);
    assert_eq!(ambient(150).gravity, Real::ZERO);
    assert_eq!(ambient(200).atmosphere, None);
    assert_eq!(ambient(0).clock, clock);
    // The same height elsewhere, also round the wrap, is the same.
    let far = world
        .cosmos
        .ambient(Location::new(world.grid, CellPos::new(-5, 2000, 99)))
        .unwrap();
    assert_eq!(far, ambient(99));
    assert!(
        world
            .cosmos
            .ambient(Location::new(world.grid, CellPos::new(0, 0, 500)))
            .is_err()
    );
}

#[test]
fn reading_an_unloaded_cell_is_an_error_value() {
    let mut world = world();
    let near = Location::new(world.grid, CellPos::new(5, 5, 0));
    let far = Location::new(world.grid, CellPos::new(5 + TRACT_XY, 5, 0));
    assert!(!world.cosmos.is_loaded(near));
    assert_eq!(world.cosmos.cell(near), Err(WorldError::NotLoaded(near)));
    assert_eq!(world.cosmos.flags(near), Err(WorldError::NotLoaded(near)));
    assert_eq!(
        world.cosmos.standable(near),
        Err(WorldError::NotLoaded(near))
    );
    assert_eq!(
        world.cosmos.obstacle_height(near),
        Err(WorldError::NotLoaded(near))
    );
    assert_eq!(
        world.cosmos.field(near, FieldId::new(0)),
        Err(WorldError::NotLoaded(near))
    );
    assert_eq!(world.cosmos.height(near), Err(WorldError::NotLoaded(near)));
    assert_eq!(world.cosmos.damage(near), Err(WorldError::NotLoaded(near)));

    load_ground(&mut world, TractPos::new(0, 0));
    assert!(world.cosmos.is_loaded(near));
    assert!(world.cosmos.cell(near).is_ok());
    // The tract next door is still not loaded, and reading it loads
    // nothing.
    assert_eq!(world.cosmos.flags(far), Err(WorldError::NotLoaded(far)));
    assert!(!world.cosmos.is_loaded(far));
    // A position that wraps is reported where it really is.
    let wrapped = Location::new(world.grid, CellPos::new(-1, 5, 0));
    let real = world.cosmos.normalize(wrapped).unwrap();
    assert_eq!(
        world.cosmos.flags(wrapped),
        Err(WorldError::NotLoaded(real))
    );
    // A grid that does not exist is its own error.
    let nowhere = Location::new(GridId::new(999), CellPos::new(0, 0, 0));
    assert_eq!(
        world.cosmos.cell(nowhere),
        Err(WorldError::UnknownGrid(GridId::new(999)))
    );
    assert!(!world.cosmos.is_loaded(nowhere));
    // Unloading makes it unreadable again.
    assert!(unload_column(&mut world.cosmos, world.grid, TractPos::new(0, 0)).is_some());
    assert_eq!(world.cosmos.cell(near), Err(WorldError::NotLoaded(near)));
}

#[test]
fn the_read_api_reports_cells_flags_and_geometry() {
    let mut world = world();
    let defs = Arc::clone(&world.defs);
    let rock = defs.block_state("solid", None).unwrap();
    let wall = defs.block_state("wall@brick", None).unwrap();
    let door = defs.block_state("door#open", None).unwrap();
    let slab = defs.floor_state("slab", None).unwrap();
    let desk = defs.fixture_state("desk", None).unwrap();

    let mut builder = ColumnBuilder::new(&defs, rock, BlockState::NONE, 0);
    builder
        .fill_floor(0, slab)
        .set_block(CellPos::new(2, 2, 0), wall)
        .set_block(CellPos::new(3, 2, 0), door)
        .set_fixture(CellPos::new(4, 2, 0), desk)
        .set_floor(CellPos::new(2, 2, 1), slab);
    load_column(
        &mut world.cosmos,
        world.grid,
        TractPos::new(0, 0),
        builder.build(),
    )
    .unwrap();
    let at = |x, y, z| Location::new(world.grid, CellPos::new(x, y, z));

    let cell = world.cosmos.cell(at(2, 2, 0)).unwrap();
    assert_eq!(
        (cell.floor, cell.block, cell.fixture),
        (slab, wall, FixtureState::NONE)
    );
    assert_eq!(
        world.cosmos.flags(at(2, 2, 0)).unwrap(),
        CellFlags::HAS_FLOOR | CellFlags::OPAQUE | CellFlags::WALK_BLOCKED
    );
    assert_eq!(
        world.cosmos.flags(at(3, 2, 0)).unwrap(),
        CellFlags::HAS_FLOOR
    );
    assert_eq!(
        world.cosmos.flags(at(4, 2, 0)).unwrap(),
        CellFlags::HAS_FLOOR | CellFlags::WALK_BLOCKED
    );
    assert_eq!(world.cosmos.flags(at(9, 9, 5)).unwrap(), CellFlags::NONE);
    assert_eq!(
        world.cosmos.flags(at(9, 9, -1)).unwrap(),
        CellFlags::OPAQUE | CellFlags::WALK_BLOCKED | CellFlags::SOLID_TOP
    );

    assert!(world.cosmos.standable(at(9, 9, 0)).unwrap());
    assert!(!world.cosmos.standable(at(9, 9, 1)).unwrap());
    assert!(world.cosmos.standable(at(2, 2, 1)).unwrap());
    assert!(world.cosmos.is_open(at(9, 9, 1)).unwrap());
    // The lowest storey of the realm stands on the fill below it.
    assert!(world.cosmos.standable(at(9, 9, -16)).unwrap());

    assert_eq!(
        world.cosmos.obstacle_height(at(2, 2, 0)).unwrap(),
        Real::of(3.0)
    );
    assert_eq!(
        world.cosmos.obstacle_height(at(4, 2, 0)).unwrap(),
        Real::of(0.8)
    );
    // An open door is passable: it is no obstacle whatever its height.
    assert_eq!(
        world.cosmos.obstacle_height(at(3, 2, 0)).unwrap(),
        Real::ZERO
    );
    assert_eq!(
        world.cosmos.obstacle_height(at(9, 9, 0)).unwrap(),
        Real::ZERO
    );

    assert_eq!(world.cosmos.height(at(9, 9, 0)).unwrap(), 0);
    assert_eq!(world.cosmos.height(at(2, 2, 0)).unwrap(), 1);
    assert!(world.cosmos.under_open_sky(at(9, 9, 0)).unwrap());
    assert!(!world.cosmos.under_open_sky(at(2, 2, 0)).unwrap());
    assert!(world.cosmos.under_open_sky(at(2, 2, 1)).unwrap());

    assert_eq!(world.cosmos.field(at(2, 2, 0), FieldId::new(0)).unwrap(), 0);
    assert_eq!(world.cosmos.damage(at(2, 2, 0)).unwrap(), [0; 3]);
    assert!(world.cosmos.decals(at(2, 2, 0)).unwrap().is_empty());
    assert_eq!(world.cosmos.var(at(2, 2, 0), "core:mark").unwrap(), None);

    // The grid view answers the same questions without the grid lookup.
    let view = world.cosmos.grid(world.grid).unwrap();
    assert_eq!(
        view.flags(CellPos::new(2, 2, 0)),
        world.cosmos.flags(at(2, 2, 0))
    );
    assert_eq!(view.loaded_tracts(), [TractPos::new(0, 0)]);
    assert_eq!(view.realm(), world.realm);
}

#[test]
fn realms_are_isolated_from_each_other() {
    let mut world = world();
    let def = world.defs.realms().lookup("core:training_room").unwrap();
    let room = world
        .cosmos
        .create_realm(&mut world.timeline, def, 1)
        .unwrap();
    let room_grid = world.cosmos.realm(room).unwrap().base_grid;
    let wall = world.defs.block_state("wall", None).unwrap();

    load_ground(&mut world, TractPos::new(0, 0));
    let mut builder = ColumnBuilder::new(&world.defs, BlockState::NONE, BlockState::NONE, 0);
    builder.set_block(CellPos::new(5, 5, 0), wall);
    load_column(
        &mut world.cosmos,
        room_grid,
        TractPos::new(0, 0),
        builder.build(),
    )
    .unwrap();

    // The same coordinates are different places in different grids.
    let cell = CellPos::new(5, 5, 0);
    let here = world.cosmos.cell(Location::new(world.grid, cell)).unwrap();
    let there = world.cosmos.cell(Location::new(room_grid, cell)).unwrap();
    assert!(here.block.is_none());
    assert!(!here.floor.is_none());
    assert_eq!(there.block, wall);
    assert!(there.floor.is_none());
    // Each realm has its own shape: the room does not wrap.
    assert!(
        world
            .cosmos
            .normalize(Location::new(world.grid, CellPos::new(-1, 0, 0)))
            .is_ok()
    );
    assert!(
        world
            .cosmos
            .normalize(Location::new(room_grid, CellPos::new(-1, 0, 0)))
            .is_err()
    );
    // Loading in one changes nothing in the other.
    assert!(
        !world
            .cosmos
            .is_loaded(Location::new(room_grid, CellPos::new(40, 0, 0)))
    );
    load_ground(&mut world, TractPos::new(1, 0));
    assert!(
        world
            .cosmos
            .is_loaded(Location::new(world.grid, CellPos::new(40, 0, 0)))
    );
    assert!(
        !world
            .cosmos
            .is_loaded(Location::new(room_grid, CellPos::new(40, 0, 0)))
    );
}

#[test]
fn the_cosmos_hashes_by_content_not_by_load_order() {
    let build = |order: &[TractPos]| {
        let mut world = world();
        for tract in order {
            load_ground(&mut world, *tract);
        }
        world
    };
    let a = TractPos::new(0, 0);
    let b = TractPos::new(1, 0);
    let c = TractPos::new(0, 1);
    let one = build(&[a, b, c]);
    let two = build(&[c, a, b]);
    assert_eq!(Hash128::of(&one.cosmos), Hash128::of(&two.cosmos));
    let fewer = build(&[a, b]);
    assert_ne!(Hash128::of(&one.cosmos), Hash128::of(&fewer.cosmos));

    // A different cell is a different hash.
    let mut changed = build(&[a, b]);
    let wall = changed.defs.block_state("wall", None).unwrap();
    let rock = changed.defs.block_state("solid", None).unwrap();
    let grass = changed.defs.floor_state("grass", None).unwrap();
    let mut builder = ColumnBuilder::new(&changed.defs, rock, BlockState::NONE, 0);
    builder.fill_floor(0, grass);
    assert_eq!(
        Hash128::of(builder.column()),
        Hash128::of(&flat_column(&changed.defs, rock, grass, 0))
    );
    builder.set_block(CellPos::new(1, 1, 0), wall);
    load_column(&mut changed.cosmos, changed.grid, b, builder.build()).unwrap();
    assert_ne!(Hash128::of(&changed.cosmos), Hash128::of(&fewer.cosmos));
}

/// Loads a block of tracts of varied ground for the timing tests and
/// returns the cells to read.
fn timing_scene() -> (World, Vec<Location>) {
    let mut world = world();
    let defs = Arc::clone(&world.defs);
    let rock = defs.block_state("solid", None).unwrap();
    let wall = defs.block_state("wall", None).unwrap();
    let slab = defs.floor_state("slab", None).unwrap();
    let desk = defs.fixture_state("desk", None).unwrap();
    let side = 8;
    for ty in 0..side {
        for tx in 0..side {
            let mut builder = ColumnBuilder::new(&defs, rock, BlockState::NONE, 0);
            builder.fill_floor(0, slab);
            for step in 0..TRACT_XY {
                builder.set_block(CellPos::new(step, (step * 7 + tx) % TRACT_XY, 0), wall);
                builder.set_fixture(CellPos::new((step * 5 + ty) % TRACT_XY, step, 0), desk);
                builder.set_floor(CellPos::new(step, step, 1), slab);
            }
            load_column(
                &mut world.cosmos,
                world.grid,
                TractPos::new(tx, ty),
                builder.build(),
            )
            .unwrap();
        }
    }
    // A fixed scatter of cells over the loaded tracts and three storeys.
    let span = side * TRACT_XY;
    let mut cells = Vec::new();
    let mut state = 0x9e37_79b9_u32;
    for _ in 0..4096 {
        state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
        let x = (state >> 8) as i32 % span;
        state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
        let y = (state >> 8) as i32 % span;
        let z = (state >> 28) as i32 % 3 - 1;
        cells.push(Location::new(world.grid, CellPos::new(x, y, z)));
    }
    (world, cells)
}

/// Times a read over the scattered cells: the best of a few passes, in
/// nanoseconds per read, printed.
#[allow(
    clippy::disallowed_types,
    clippy::disallowed_methods,
    reason = "a timing test reads the wall clock; nothing in the simulation does"
)]
fn time_reads(label: &str, cells: &[Location], mut read: impl FnMut(Location) -> u64) {
    let rounds = 500;
    let mut best = f64::INFINITY;
    let mut checksum = 0_u64;
    for _ in 0..5 {
        let started = std::time::Instant::now();
        let mut seen = 0_u64;
        for _ in 0..rounds {
            for loc in cells {
                seen = seen.wrapping_add(read(std::hint::black_box(*loc)));
            }
        }
        let nanos = started.elapsed().as_nanos() as f64 / (rounds * cells.len()) as f64;
        best = best.min(nanos);
        checksum = seen;
    }
    println!("{label}: {best:.1} ns per read (checksum {checksum})");
}

/// Not a benchmark with a budget: rough figures, printed. Run with
/// `cargo test --release -p clash-world --test cosmos -- --ignored --nocapture --test-threads=1`.
#[test]
#[ignore = "timing; run by hand in release mode"]
fn timing_of_flag_reads() {
    let (world, cells) = timing_scene();
    let view = world.cosmos.grid(world.grid).unwrap();
    for _ in 0..2 {
        time_reads("flag read through Cosmos", &cells, |loc| {
            world
                .cosmos
                .flags(loc)
                .map_or(0, |flags| u64::from(flags.bits()))
        });
        time_reads("flag read through a GridView", &cells, |loc| {
            view.flags(loc.cell)
                .map_or(0, |flags| u64::from(flags.bits()))
        });
    }
}

/// Not a benchmark with a budget: rough figures, printed. Run with
/// `cargo test --release -p clash-world --test cosmos -- --ignored --nocapture --test-threads=1`.
#[test]
#[ignore = "timing; run by hand in release mode"]
fn timing_of_state_reads() {
    let (world, cells) = timing_scene();
    let view = world.cosmos.grid(world.grid).unwrap();
    let sum = |cell: clash_world::CellView| {
        u64::from(cell.block.raw().def) + u64::from(cell.floor.raw().def)
    };
    for _ in 0..2 {
        time_reads(
            "state read (three elements) through Cosmos",
            &cells,
            |loc| world.cosmos.cell(loc).map_or(0, sum),
        );
        time_reads(
            "state read (three elements) through a GridView",
            &cells,
            |loc| view.cell(loc.cell).map_or(0, sum),
        );
    }
}
