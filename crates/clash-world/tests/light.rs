//! Tests for S1: light (15 §3.3). The sky by time of day, its spread
//! under cover, fixed and moving sources, the cache against a fresh
//! working-out, and the batch fill against cell-by-cell reads.

mod common;

use std::collections::BTreeMap;
use std::sync::Arc;

use clash_core::{Dur, Real};
use clash_time::Timeline;
use clash_world::dims::{CHUNK_XY, REGION_CELLS, TRACT_XY};
use clash_world::light::{
    LIGHT_EYE_MM, LIGHT_FILL_MAX_CELLS, SKY_DARK, STEP_DIAGONAL, STEP_SIDE, STEP_STOREY,
};
use clash_world::test_support::{load_column, unload_column};
use clash_world::{
    BlockState, Cause, CellBox, CellPos, ColumnBuilder, Cosmos, EventSink, FixtureState,
    FloorState, GridId, LightSource, Location, MovingLight, SkyTime, TractPos, WorldCtx, WorldDefs,
    WorldError, WorldEvent,
};
use proptest::prelude::*;

/// Test content beside the core mod's.
const EXTRA: &str = r#"[
  { "type": "fixture", "id": "gloom", "material": "steel", "height": 1.0, "fill": 0.02,
    "light": { "intensity": -0.6, "radius": "4 m" } },
  { "type": "block", "id": "hedge", "material": "wood", "height": 1.0, "fill": 0.3,
    "flags": ["opaque"] },
  { "type": "floor", "id": "glow_tile", "material": "glass", "thickness": 0.05,
    "light": { "intensity": 0.25, "radius": "3 m" } }
]"#;

struct Kit {
    defs: Arc<WorldDefs>,
    rock: BlockState,
    wall: BlockState,
    window: BlockState,
    door: BlockState,
    door_open: BlockState,
    hedge: BlockState,
    glow_block: BlockState,
    slab: FloorState,
    glow_tile: FloorState,
    desk: FixtureState,
    lamp: FixtureState,
    lamp_off: FixtureState,
    gloom: FixtureState,
}

fn kit() -> Kit {
    let mut raw = common::test_raw();
    common::add_json(&mut raw, "core", EXTRA, "EXTRA");
    let defs = Arc::new(WorldDefs::build(raw).expect("the test definitions build"));
    let block = |text: &str| defs.block_state(text, None).unwrap();
    let fixture = |text: &str| defs.fixture_state(text, None).unwrap();
    Kit {
        rock: block("solid"),
        wall: block("wall"),
        window: block("window"),
        door: block("door"),
        door_open: block("door#open"),
        hedge: block("hedge"),
        glow_block: block("glow_block"),
        slab: defs.floor_state("slab", None).unwrap(),
        glow_tile: defs.floor_state("glow_tile", None).unwrap(),
        desk: fixture("desk"),
        lamp: fixture("lamp"),
        lamp_off: fixture("lamp#off"),
        gloom: fixture("gloom"),
        defs,
    }
}

/// Sets cells of tracts that are being built, whichever tract each
/// falls in. A cell outside those tracts is left alone.
struct Painter<'a> {
    builders: BTreeMap<TractPos, ColumnBuilder<'a>>,
}

impl Painter<'_> {
    fn block(&mut self, cell: CellPos, state: BlockState) {
        if let Some(builder) = self.builders.get_mut(&cell.tract()) {
            builder.set_block(cell, state);
        }
    }

    fn floor(&mut self, cell: CellPos, state: FloorState) {
        if let Some(builder) = self.builders.get_mut(&cell.tract()) {
            builder.set_floor(cell, state);
        }
    }

    fn fixture(&mut self, cell: CellPos, state: FixtureState) {
        if let Some(builder) = self.builders.get_mut(&cell.tract()) {
            builder.set_fixture(cell, state);
        }
    }

    /// Lays a floor over a rectangle of one storey.
    fn floor_rect(&mut self, min: (i32, i32), max: (i32, i32), z: i32, state: FloorState) {
        for y in min.1..=max.1 {
            for x in min.0..=max.0 {
                self.floor(CellPos::new(x, y, z), state);
            }
        }
    }

    /// Builds the outline of a rectangle of one storey out of a block.
    fn outline(&mut self, min: (i32, i32), max: (i32, i32), z: i32, state: BlockState) {
        for x in min.0..=max.0 {
            self.block(CellPos::new(x, min.1, z), state);
            self.block(CellPos::new(x, max.1, z), state);
        }
        for y in min.1..=max.1 {
            self.block(CellPos::new(min.0, y, z), state);
            self.block(CellPos::new(max.0, y, z), state);
        }
    }

    /// A room of one storey: walls round a rectangle, a floor in it and
    /// a roof over the whole of it.
    fn room(&mut self, kit: &Kit, min: (i32, i32), max: (i32, i32), z: i32) {
        self.outline(min, max, z, kit.wall);
        self.floor_rect(min, max, z, kit.slab);
        self.floor_rect(min, max, z + 1, kit.slab);
    }
}

struct Scene {
    kit: Kit,
    cosmos: Cosmos,
    timeline: Timeline,
    events: EventSink<WorldEvent>,
    grid: GridId,
}

/// Noon by the clock of `core:testland`, whose sky starts the day at
/// eight in the morning.
fn noon() -> SkyTime {
    SkyTime::at(Dur::from_secs(4 * 3600))
}

/// Midnight by the same clock.
fn midnight() -> SkyTime {
    SkyTime::at(Dur::from_secs(16 * 3600))
}

/// The light the test sky gives at night.
const NIGHT: f64 = 0.03;

/// What is left of sky light after spreading over whole cells.
fn spread(cells: i32) -> Real {
    (0..cells).fold(Real::ONE, |share, _| share * Real::of(0.75))
}

impl Scene {
    fn loc(&self, x: i32, y: i32, z: i32) -> Location {
        Location::new(self.grid, CellPos::new(x, y, z))
    }

    fn light(&self, x: i32, y: i32, z: i32, time: SkyTime) -> Real {
        self.cosmos.light(self.loc(x, y, z), time).unwrap()
    }

    fn steps(&self, x: i32, y: i32, z: i32) -> u8 {
        self.cosmos
            .static_light(self.loc(x, y, z))
            .unwrap()
            .sky_steps
    }

    fn fixed(&self, x: i32, y: i32, z: i32) -> Real {
        self.cosmos.static_light(self.loc(x, y, z)).unwrap().fixed
    }

    fn fill(&self, min: (i32, i32, i32), max: (i32, i32, i32), time: SkyTime) -> Vec<Real> {
        let mut out = Vec::new();
        let cells = CellBox::new(
            CellPos::new(min.0, min.1, min.2),
            CellPos::new(max.0, max.1, max.2),
        );
        self.cosmos
            .fill_light(self.grid, cells, time, &[], &mut out)
            .unwrap();
        out
    }

    fn set_block(&mut self, x: i32, y: i32, z: i32, state: BlockState) {
        let loc = self.loc(x, y, z);
        let mut ctx = WorldCtx::new(&mut self.cosmos, &mut self.timeline, &mut self.events);
        ctx.edit(Cause::DEBUG).set_block(loc, state).unwrap();
    }

    fn set_floor(&mut self, x: i32, y: i32, z: i32, state: FloorState) {
        let loc = self.loc(x, y, z);
        let mut ctx = WorldCtx::new(&mut self.cosmos, &mut self.timeline, &mut self.events);
        ctx.edit(Cause::DEBUG).set_floor(loc, state).unwrap();
    }

    fn set_fixture(&mut self, x: i32, y: i32, z: i32, state: FixtureState) {
        let loc = self.loc(x, y, z);
        let mut ctx = WorldCtx::new(&mut self.cosmos, &mut self.timeline, &mut self.events);
        ctx.edit(Cause::DEBUG).set_fixture(loc, state).unwrap();
    }

    /// How many chunks have had their light worked out so far.
    fn lit(&self) -> u64 {
        self.cosmos.light_stats().chunks_lit
    }
}

/// Rock up to storey 0, with a slab over the whole of every tract, and
/// whatever `paint` adds.
fn scene(tracts: &[TractPos], paint: impl FnOnce(&Kit, &mut Painter<'_>)) -> Scene {
    let kit = kit();
    let mut timeline = Timeline::new();
    let mut cosmos = Cosmos::new(Arc::clone(&kit.defs));
    let def = kit.defs.realms().lookup("core:testland").unwrap();
    let realm = cosmos.create_realm(&mut timeline, def, 7).unwrap();
    let grid = cosmos.realm(realm).unwrap().base_grid;
    let columns: Vec<_> = {
        let mut painter = Painter {
            builders: tracts
                .iter()
                .map(|tract| {
                    let mut builder = ColumnBuilder::new(&kit.defs, kit.rock, BlockState::NONE, 0);
                    builder.fill_floor(0, kit.slab);
                    (*tract, builder)
                })
                .collect(),
        };
        paint(&kit, &mut painter);
        painter
            .builders
            .into_iter()
            .map(|(tract, builder)| (tract, builder.build()))
            .collect()
    };
    for (tract, column) in columns {
        load_column(&mut cosmos, grid, tract, column).unwrap();
    }
    Scene {
        kit,
        cosmos,
        timeline,
        events: EventSink::new(),
        grid,
    }
}

fn nine_tracts() -> Vec<TractPos> {
    (0..3)
        .flat_map(|y| (0..3).map(move |x| TractPos::new(x, y)))
        .collect()
}

fn four_tracts() -> Vec<TractPos> {
    (0..2)
        .flat_map(|y| (0..2).map(move |x| TractPos::new(x, y)))
        .collect()
}

// ---- The sky ---------------------------------------------------------

#[test]
fn noon_is_bright_and_midnight_dark_under_the_open_sky() {
    let scene = scene(&[TractPos::new(0, 0)], |_, _| {});
    assert_eq!(scene.light(10, 10, 0, noon()), Real::ONE);
    assert_eq!(scene.light(10, 10, 0, midnight()), Real::of(NIGHT));
    // Half way through dusk, half way between the two.
    let dusk = SkyTime::at(Dur::from_secs(10 * 3600));
    assert_eq!(
        scene.light(10, 10, 0, dusk),
        Real::of(1.0) + Real::of(NIGHT - 1.0) * Real::of(0.5)
    );
    // The air above is as bright as the ground, and the weather takes
    // its share.
    assert_eq!(scene.light(10, 10, 5, noon()), Real::ONE);
    assert_eq!(
        scene.light(10, 10, 0, noon().dimmed(Real::of(0.5))),
        Real::of(0.5)
    );
    assert_eq!(scene.steps(10, 10, 0), 0);
    // A day later it is noon again.
    let tomorrow = SkyTime::at(Dur::from_secs((24 + 4) * 3600));
    assert_eq!(scene.light(10, 10, 0, tomorrow), Real::ONE);
}

#[test]
fn a_stratum_that_names_no_sky_is_dark() {
    let scene = scene(&[TractPos::new(0, 0)], |kit, paint| {
        // A roof high up, where the sky of `core:testland` gives way to
        // a stratum that names none.
        paint.floor_rect((10, 10), (14, 14), 121, kit.slab);
    });
    // Storey 120 is in the stratum from 100, which has no sky.
    assert_eq!(scene.light(5, 5, 120, noon()), Real::ZERO);
    assert_eq!(scene.steps(5, 5, 120), 0);
    // Under the roof the cells are a few steps from the open sky all
    // the same: the distance does not depend on which sky it is.
    assert_eq!(scene.steps(12, 12, 120), 3 * STEP_SIDE);
}

#[test]
fn a_closed_room_is_dark_at_noon() {
    let scene = scene(&[TractPos::new(0, 0)], |kit, paint| {
        paint.room(kit, (8, 8), (20, 16), 0);
    });
    for y in 9..=15 {
        for x in 9..=19 {
            assert_eq!(scene.light(x, y, 0, noon()), Real::ZERO, "({x}, {y})");
            assert_eq!(scene.steps(x, y, 0), SKY_DARK);
        }
    }
    // Its walls are lit from outside, a cell from the open sky, and the
    // roof and the ground beside it are in full light.
    assert_eq!(scene.light(8, 12, 0, noon()), spread(1));
    assert_eq!(scene.light(8, 8, 0, noon()), spread(1));
    assert_eq!(scene.light(12, 12, 1, noon()), Real::ONE);
    assert_eq!(scene.light(7, 12, 0, noon()), Real::ONE);
}

#[test]
fn a_room_is_lit_near_its_windows_and_dark_beyond_the_cap() {
    let scene = scene(&[TractPos::new(0, 0)], |kit, paint| {
        paint.room(kit, (4, 4), (28, 16), 0);
        paint.block(CellPos::new(4, 10, 0), kit.window);
    });
    // The window is one cell from the open sky and each cell further in
    // is one more: a quarter lost for each.
    assert_eq!(scene.steps(3, 10, 0), 0);
    assert_eq!(scene.steps(4, 10, 0), STEP_SIDE);
    for depth in 1..=7 {
        let x = 4 + depth;
        assert_eq!(scene.steps(x, 10, 0), STEP_SIDE * (1 + depth as u8));
        assert_eq!(scene.light(x, 10, 0, noon()), spread(1 + depth), "{x}");
    }
    // Eight cells is the cap of the test sky: past it, nothing.
    assert_eq!(scene.light(11, 10, 0, noon()), spread(8));
    assert!(scene.light(11, 10, 0, noon()) > Real::of(0.1));
    assert_eq!(scene.light(12, 10, 0, noon()), Real::ZERO);
    assert_eq!(scene.steps(12, 10, 0), SKY_DARK);
    assert_eq!(scene.light(27, 15, 0, noon()), Real::ZERO);
    // To the side of the window the light goes diagonally at a cell and
    // a half for each step.
    assert_eq!(scene.steps(6, 11, 0), 2 * STEP_SIDE + STEP_DIAGONAL);
    assert_eq!(scene.steps(7, 12, 0), 2 * STEP_SIDE + 2 * STEP_DIAGONAL);
    assert!(scene.light(6, 11, 0, noon()) < scene.light(6, 10, 0, noon()));
    assert!(scene.light(6, 11, 0, noon()) > scene.light(7, 10, 0, noon()));
    // At night the same room has the same shape of light, fainter.
    assert_eq!(
        scene.light(5, 10, 0, midnight()),
        Real::of(NIGHT) * spread(2)
    );
    // The inside of the wall beside the window is lit from the room.
    assert!(scene.light(4, 9, 0, noon()) > Real::ZERO);
}

#[test]
fn light_passes_an_open_door_and_stops_at_a_shut_one() {
    let mut scene = scene(&[TractPos::new(0, 0)], |kit, paint| {
        paint.room(kit, (8, 8), (16, 14), 0);
        paint.block(CellPos::new(12, 8, 0), kit.door);
    });
    assert_eq!(scene.light(12, 9, 0, noon()), Real::ZERO);
    // The shut door is lit from outside like the wall beside it.
    assert_eq!(scene.light(12, 8, 0, noon()), spread(1));
    scene.set_block(12, 8, 0, scene.kit.door_open);
    assert_eq!(scene.light(12, 9, 0, noon()), spread(2));
    scene.set_block(12, 8, 0, scene.kit.door);
    assert_eq!(scene.light(12, 9, 0, noon()), Real::ZERO);
}

#[test]
fn light_does_not_slip_between_walls_that_touch_at_their_corners() {
    let scene = scene(&[TractPos::new(0, 0)], |kit, paint| {
        paint.room(kit, (8, 8), (16, 14), 0);
        // A gap in the corner of the wall, closed diagonally: the cell
        // at the corner is open, and the two walls beside it stand.
        paint.block(CellPos::new(8, 8, 0), BlockState::NONE);
    });
    // The corner cell itself is lit from outside.
    assert_eq!(scene.steps(8, 8, 0), STEP_SIDE);
    // The cell diagonally inside it is not reached: both cells beside
    // the corner are walls.
    assert_eq!(scene.steps(9, 9, 0), SKY_DARK);
    assert_eq!(scene.light(9, 9, 0, noon()), Real::ZERO);
}

#[test]
fn light_goes_between_storeys_where_there_is_no_floor() {
    let scene = scene(&[TractPos::new(0, 0)], |kit, paint| {
        // Two storeys, the upper with a window, and a hole in the floor
        // between them.
        paint.room(kit, (8, 8), (18, 14), 0);
        paint.room(kit, (8, 8), (18, 14), 1);
        paint.block(CellPos::new(8, 11, 1), kit.window);
        paint.floor(CellPos::new(10, 11, 1), FloorState::NONE);
    });
    // Upstairs: the window, then in.
    assert_eq!(scene.steps(9, 11, 1), 2 * STEP_SIDE);
    assert_eq!(scene.steps(10, 11, 1), 3 * STEP_SIDE);
    // Down the hole costs a storey, and the room below has no other
    // light.
    assert_eq!(scene.steps(10, 11, 0), 3 * STEP_SIDE + STEP_STOREY);
    assert_eq!(scene.steps(11, 11, 0), 4 * STEP_SIDE + STEP_STOREY);
    assert_eq!(scene.steps(9, 9, 0), SKY_DARK);
    // Where the floor is whole nothing comes down.
    assert_eq!(scene.steps(15, 13, 0), SKY_DARK);
}

#[test]
fn something_opaque_and_low_does_not_stop_light() {
    let scene = scene(&[TractPos::new(0, 0)], |kit, paint| {
        paint.room(kit, (8, 8), (18, 14), 0);
        paint.block(CellPos::new(8, 11, 0), kit.window);
        // A hedge a metre tall right behind the window.
        paint.block(CellPos::new(9, 11, 0), kit.hedge);
        paint.fixture(CellPos::new(12, 11, 0), kit.lamp);
    });
    assert_eq!(scene.steps(10, 11, 0), 3 * STEP_SIDE);
    // Nor does it cast a shadow from a lamp: light travels above it.
    const { assert!(LIGHT_EYE_MM > 1000) };
    assert_eq!(scene.fixed(8, 11, 0), Real::ONE / Real::of(17.0));
}

// ---- Edits and the cache ----------------------------------------------

#[test]
fn a_hole_in_the_wall_lets_light_in_and_relights_only_what_it_reaches() {
    let mut scene = scene(&nine_tracts(), |kit, paint| {
        // A closed room well inside the middle tract, and another hard
        // against that tract's western side.
        paint.room(kit, (44, 44), (52, 52), 0);
        paint.room(kit, (33, 42), (39, 50), 0);
    });
    let whole = |scene: &Scene| scene.fill((0, 0, 0), (95, 95, 0), noon());
    let before = whole(&scene);
    assert_eq!(scene.lit(), 9, "one working-out for each chunk asked about");
    assert_eq!(scene.light(45, 48, 0, noon()), Real::ZERO);
    // Asking again works nothing out.
    assert_eq!(whole(&scene), before);
    assert_eq!(scene.lit(), 9);

    // Knock a hole in the wall of the first room. Light gets in, and no
    // chunk but the room's own is within its reach.
    scene.set_block(44, 48, 0, BlockState::NONE);
    assert_eq!(scene.light(44, 48, 0, noon()), spread(1));
    assert_eq!(scene.light(45, 48, 0, noon()), spread(2));
    assert_eq!(scene.light(51, 48, 0, noon()), spread(8));
    let after = whole(&scene);
    assert_eq!(scene.lit(), 10, "only the chunk the light reaches");
    let width = 96;
    let changed: Vec<(usize, usize)> = (0..before.len())
        .filter(|at| before[*at] != after[*at])
        .map(|at| (at % width, at / width))
        .collect();
    assert!(!changed.is_empty());
    assert!(
        changed
            .iter()
            .all(|(x, y)| (44..=52).contains(x) && (44..=52).contains(y)),
        "{changed:?}"
    );

    // A hole in the western wall of the second room is two cells from
    // the next chunk: that chunk is within reach and is worked out too.
    scene.set_block(33, 46, 0, BlockState::NONE);
    assert_eq!(scene.light(34, 46, 0, noon()), spread(2));
    whole(&scene);
    assert_eq!(scene.lit(), 12, "the room's chunk and its neighbour");

    // An edit that light does not care about works nothing out: a desk
    // casts no shadow and gives no light.
    scene.set_fixture(60, 60, 0, scene.kit.desk);
    scene.set_fixture(46, 46, 0, scene.kit.desk);
    whole(&scene);
    assert_eq!(scene.lit(), 12);

    // Mending both holes puts the rooms back in the dark, and the whole
    // view back as it was.
    scene.set_block(44, 48, 0, scene.kit.wall);
    assert_eq!(scene.light(45, 48, 0, noon()), Real::ZERO);
    scene.set_block(33, 46, 0, scene.kit.wall);
    scene.set_fixture(60, 60, 0, FixtureState::NONE);
    scene.set_fixture(46, 46, 0, FixtureState::NONE);
    assert_eq!(whole(&scene), before);
    assert_eq!(scene.lit(), 15);
}

#[test]
fn taking_the_roof_off_lights_the_room_below() {
    let mut scene = scene(&[TractPos::new(0, 0)], |kit, paint| {
        paint.room(kit, (10, 10), (20, 20), 0);
    });
    assert_eq!(scene.light(15, 15, 0, noon()), Real::ZERO);
    // One slab of the roof gone: the cell below is open to the sky, and
    // light spreads from it.
    scene.set_floor(15, 15, 1, FloorState::NONE);
    assert_eq!(scene.light(15, 15, 0, noon()), Real::ONE);
    assert_eq!(scene.light(16, 15, 0, noon()), spread(1));
    assert_eq!(scene.light(18, 15, 0, noon()), spread(3));
    scene.set_floor(15, 15, 1, scene.kit.slab);
    assert_eq!(scene.light(15, 15, 0, noon()), Real::ZERO);
    // A roof far above does the same: what is open to the sky is read
    // from the whole column.
    scene.set_floor(5, 5, 40, scene.kit.slab);
    assert_eq!(scene.steps(5, 5, 0), STEP_SIDE);
    assert_eq!(scene.light(5, 5, 0, noon()), spread(1));
    assert_eq!(scene.light(5, 5, 40, noon()), Real::ONE);
}

#[test]
fn loading_and_unloading_a_tract_is_seen() {
    let mut scene = scene(&[TractPos::new(0, 0), TractPos::new(1, 0)], |kit, paint| {
        // A room across the line between the two tracts, open to the
        // east only.
        paint.room(kit, (26, 8), (38, 14), 0);
        paint.block(CellPos::new(38, 11, 0), kit.window);
    });
    assert_eq!(scene.light(37, 11, 0, noon()), spread(2));
    assert_eq!(scene.light(31, 11, 0, noon()), spread(8));
    let lit = scene.lit();
    // With the eastern tract gone its cells are an error, and no light
    // comes out of it.
    let column = unload_column(&mut scene.cosmos, scene.grid, TractPos::new(1, 0)).unwrap();
    let gone = scene.loc(37, 11, 0);
    assert_eq!(
        scene.cosmos.light(gone, noon()),
        Err(WorldError::NotLoaded(gone))
    );
    assert_eq!(scene.light(31, 11, 0, noon()), Real::ZERO);
    assert_eq!(scene.lit(), lit + 1);
    // A fill writes zero for what is not loaded.
    let row = scene.fill((30, 11, 0), (33, 11, 0), noon());
    assert_eq!(row, [Real::ZERO; 4]);
    // And back.
    load_column(&mut scene.cosmos, scene.grid, TractPos::new(1, 0), column).unwrap();
    assert_eq!(scene.light(31, 11, 0, noon()), spread(8));
    assert_eq!(scene.light(37, 11, 0, noon()), spread(2));
}

#[test]
fn a_tract_put_in_place_of_another_is_read_again() {
    let mut scene = scene(&[TractPos::new(0, 0)], |kit, paint| {
        paint.room(kit, (8, 8), (20, 16), 0);
    });
    assert_eq!(scene.light(12, 12, 0, noon()), Real::ZERO);
    // The same tract with other cells, built the same way, so that the
    // revisions of its chunks are what they were.
    let other = {
        let mut builder = ColumnBuilder::new(&scene.kit.defs, scene.kit.rock, BlockState::NONE, 0);
        builder.fill_floor(0, scene.kit.slab);
        builder.build()
    };
    load_column(&mut scene.cosmos, scene.grid, TractPos::new(0, 0), other).unwrap();
    assert_eq!(scene.light(12, 12, 0, noon()), Real::ONE);
    assert_eq!(scene.steps(12, 12, 0), 0);
}

#[test]
fn reads_refuse_what_is_not_there() {
    let scene = scene(&[TractPos::new(0, 0)], |_, _| {});
    let nowhere = Location::new(GridId::new(77), CellPos::new(0, 0, 0));
    assert_eq!(
        scene.cosmos.light(nowhere, noon()),
        Err(WorldError::UnknownGrid(GridId::new(77)))
    );
    let mut out = vec![Real::ONE];
    assert_eq!(
        scene.cosmos.fill_light(
            GridId::new(77),
            CellBox::single(CellPos::new(0, 0, 0)),
            noon(),
            &[],
            &mut out
        ),
        Err(WorldError::UnknownGrid(GridId::new(77)))
    );
    assert!(out.is_empty());
    // North of the realm there is nothing.
    let beyond = scene.loc(3, -1, 0);
    assert!(matches!(
        scene.cosmos.light(beyond, noon()),
        Err(WorldError::OutsideRealm { .. })
    ));
    assert!(matches!(
        scene.cosmos.static_light(beyond),
        Err(WorldError::OutsideRealm { .. })
    ));
    assert!(matches!(
        scene.cosmos.add_moving_light(beyond, Real::ZERO, &[]),
        Err(WorldError::OutsideRealm { .. })
    ));
    // A box too large to fill is refused.
    let huge = CellBox::new(CellPos::new(0, 0, 0), CellPos::new(2000, 2000, 0));
    assert_eq!(
        scene
            .cosmos
            .fill_light(scene.grid, huge, noon(), &[], &mut out),
        Err(WorldError::TooLarge {
            what: "a box of cells to fill with light",
            limit: LIGHT_FILL_MAX_CELLS
        })
    );
}

// ---- Fixed sources ----------------------------------------------------

#[test]
fn a_lamp_lights_a_dark_room_and_a_wall_stops_it() {
    let mut scene = scene(&[TractPos::new(0, 0)], |kit, paint| {
        paint.room(kit, (4, 4), (28, 14), 0);
        // A partition across the room.
        for y in 4..=14 {
            paint.block(CellPos::new(14, y, 0), kit.wall);
        }
        paint.fixture(CellPos::new(8, 8, 0), kit.lamp);
    });
    let inverse = |d_sq: f64| Real::ONE / Real::of(1.0 + d_sq);
    // intensity / (1 + d²), in the lamp's own cell and out from it.
    assert_eq!(scene.fixed(8, 8, 0), Real::ONE);
    assert_eq!(scene.fixed(9, 8, 0), inverse(1.0));
    assert_eq!(scene.fixed(10, 8, 0), inverse(4.0));
    assert_eq!(scene.fixed(9, 9, 0), inverse(2.0));
    assert_eq!(scene.fixed(11, 12, 0), inverse(9.0 + 16.0));
    assert_eq!(scene.fixed(13, 8, 0), inverse(25.0));
    // At noon the closed room has that and nothing else.
    assert_eq!(scene.light(10, 8, 0, noon()), inverse(4.0));
    // The partition is lit on the lamp's side, and stops the light.
    assert_eq!(scene.fixed(14, 8, 0), inverse(36.0));
    assert_eq!(scene.fixed(14, 6, 0), inverse(36.0 + 4.0));
    assert_eq!(scene.fixed(15, 8, 0), Real::ZERO);
    assert_eq!(scene.fixed(16, 6, 0), Real::ZERO);
    assert_eq!(scene.light(15, 8, 0, noon()), Real::ZERO);
    // Nothing gets out through the outer wall, which is lit too.
    assert_eq!(scene.fixed(8, 3, 0), Real::ZERO);
    assert_eq!(scene.fixed(8, 4, 0), inverse(16.0));
    assert_eq!(scene.fixed(4, 8, 0), inverse(16.0));
    // The roof is a floor: nothing comes up through it.
    assert_eq!(scene.fixed(8, 8, 1), Real::ZERO);

    // A doorway in the partition lets the light through in a beam, as
    // far as the lamp's radius of eight metres and no further.
    scene.set_block(14, 8, 0, BlockState::NONE);
    assert_eq!(scene.fixed(14, 8, 0), inverse(36.0));
    assert_eq!(scene.fixed(15, 8, 0), inverse(49.0));
    assert_eq!(scene.fixed(16, 8, 0), inverse(64.0));
    assert_eq!(scene.fixed(17, 8, 0), Real::ZERO);
    assert_eq!(scene.fixed(15, 6, 0), Real::ZERO);
    // A shut door stops it and an open one does not.
    scene.set_block(14, 8, 0, scene.kit.door);
    assert_eq!(scene.fixed(15, 8, 0), Real::ZERO);
    scene.set_block(14, 8, 0, scene.kit.door_open);
    assert_eq!(scene.fixed(15, 8, 0), inverse(49.0));

    // Out in the open the sky and a lamp add up.
    scene.set_fixture(20, 25, 0, scene.kit.lamp);
    assert_eq!(scene.light(21, 25, 0, noon()), Real::ONE + inverse(1.0));
    assert_eq!(
        scene.light(21, 25, 0, midnight()),
        Real::of(NIGHT) + inverse(1.0)
    );

    // Switched off, the lamp gives nothing; on again, what it gave;
    // taken down, nothing.
    scene.set_fixture(8, 8, 0, scene.kit.lamp_off);
    assert_eq!(scene.fixed(8, 8, 0), Real::ZERO);
    assert_eq!(scene.light(10, 8, 0, noon()), Real::ZERO);
    scene.set_fixture(8, 8, 0, scene.kit.lamp);
    assert_eq!(scene.fixed(10, 8, 0), inverse(4.0));
    scene.set_fixture(8, 8, 0, FixtureState::NONE);
    assert_eq!(scene.fixed(9, 8, 0), Real::ZERO);
}

#[test]
fn lamps_add_up_and_a_source_of_darkness_takes_away() {
    let scene = scene(&[TractPos::new(0, 0)], |kit, paint| {
        paint.room(kit, (4, 4), (28, 14), 0);
        paint.fixture(CellPos::new(8, 8, 0), kit.lamp);
        paint.fixture(CellPos::new(12, 8, 0), kit.lamp);
        paint.fixture(CellPos::new(20, 20, 0), kit.gloom);
        paint.block(CellPos::new(26, 26, 0), kit.glow_block);
        paint.floor(CellPos::new(22, 8, 0), kit.glow_tile);
    });
    // Between two lamps: both, in the order of their cells.
    assert_eq!(
        scene.fixed(10, 8, 0),
        Real::ONE / Real::of(5.0) + Real::ONE / Real::of(5.0)
    );
    assert_eq!(
        scene.fixed(9, 8, 0),
        Real::ONE / Real::of(2.0) + Real::ONE / Real::of(10.0)
    );
    // Darkness out in the open at noon: less light, never less than
    // none.
    let gloom = Real::of(-0.6);
    assert_eq!(scene.fixed(20, 20, 0), gloom);
    assert_eq!(scene.light(20, 20, 0, noon()), Real::ONE + gloom);
    assert_eq!(
        scene.light(21, 20, 0, noon()),
        Real::ONE + gloom / Real::of(2.0)
    );
    assert_eq!(scene.light(20, 20, 0, midnight()), Real::ZERO);
    // A block that glows lights what is round it though it is opaque,
    // and a floor that glows lights its room.
    assert_eq!(scene.fixed(26, 26, 0), Real::of(0.5));
    assert_eq!(scene.fixed(27, 26, 0), Real::of(0.25));
    assert_eq!(scene.fixed(22, 8, 0), Real::of(0.25));
    assert_eq!(scene.fixed(22, 10, 0), Real::of(0.05));
    assert_eq!(scene.fixed(22, 12, 0), Real::ZERO);
}

#[test]
fn a_lamp_lights_across_the_line_between_chunks() {
    let scene = scene(&four_tracts(), |kit, paint| {
        paint.room(kit, (24, 24), (40, 40), 0);
        paint.fixture(CellPos::new(31, 31, 0), kit.lamp);
    });
    let inverse = |d_sq: f64| Real::ONE / Real::of(1.0 + d_sq);
    assert_eq!(scene.fixed(31, 31, 0), Real::ONE);
    assert_eq!(scene.fixed(32, 31, 0), inverse(1.0));
    assert_eq!(scene.fixed(34, 35, 0), inverse(9.0 + 16.0));
    assert_eq!(scene.fixed(28, 34, 0), inverse(9.0 + 9.0));
    assert_eq!(scene.fixed(39, 31, 0), inverse(64.0));
}

#[test]
fn mistakes_in_light_definitions_are_reported() {
    let mut raw = common::test_raw();
    common::add_json(
        &mut raw,
        "bad",
        r#"[
          { "type": "fixture", "id": "beacon", "material": "steel", "height": 1.0, "fill": 0.1,
            "light": { "intensity": 3.0, "radius": "40 m" },
            "states": { "on": {}, "dim": { "light": { "intensity": 1.0, "radius": -2 } } } },
          { "type": "sky", "id": "wrong", "day": "0 s", "spread": { "loss": 2.0, "reach": 40 },
            "light": [ { "at": "30 h", "level": 1.0 }, { "at": "2 h", "level": -1.0 } ] },
          { "type": "realm", "id": "lost", "size_regions": [1, 1], "z_range": [0, 8],
            "strata": [ { "from_z": 0, "gravity": 9.81, "sky": "missing" } ] }
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
        has("fixture `bad:beacon`, light: the radius must be from 0 to 16 m"),
        "{errors:#?}"
    );
    assert!(
        has("fixture `bad:beacon`, states.dim.light: the radius must be"),
        "{errors:#?}"
    );
    assert!(
        has("sky `bad:wrong`, day: must be more than no time"),
        "{errors:#?}"
    );
    assert!(has("sky `bad:wrong`, spread.loss"), "{errors:#?}");
    assert!(has("sky `bad:wrong`, spread.reach"), "{errors:#?}");
    assert!(has("sky `bad:wrong`, light.0.at"), "{errors:#?}");
    assert!(has("sky `bad:wrong`, light.1.level"), "{errors:#?}");
    assert!(
        has("realm `bad:lost`, strata.0.sky: there is no sky `missing`"),
        "{errors:#?}"
    );
}

#[test]
fn the_core_mod_has_a_sky_and_a_lamp() {
    let kit = kit();
    let sky = kit.defs.skies().lookup("core:day_night").unwrap();
    let sky = kit.defs.skies().get(sky).unwrap();
    assert_eq!(sky.day, Dur::DAY);
    assert_eq!(sky.reach, 8);
    assert_eq!(sky.level_at(Dur::ZERO), Real::ONE);
    let land = kit.defs.realms().lookup("core:testland").unwrap();
    let land = kit.defs.realms().get(land).unwrap();
    assert!(land.stratum_at(0).unwrap().sky_id.is_some());
    assert!(land.stratum_at(120).unwrap().sky_id.is_none());
    assert_eq!(
        kit.defs
            .light_of(clash_world::Layer::Fixture, kit.lamp.raw()),
        Some(LightSource {
            intensity: Real::ONE,
            radius: Real::of(8.0)
        })
    );
    assert_eq!(
        kit.defs
            .light_of(clash_world::Layer::Fixture, kit.lamp_off.raw()),
        None
    );
    assert_eq!(
        kit.defs
            .light_of(clash_world::Layer::Fixture, kit.desk.raw()),
        None
    );
    assert!(kit.defs.has_light_sources());
}

// ---- Moving sources ---------------------------------------------------

#[test]
fn a_moving_source_lights_what_it_sees_and_not_what_is_behind_a_wall() {
    let scene = scene(&[TractPos::new(0, 0)], |kit, paint| {
        paint.room(kit, (4, 4), (28, 14), 0);
        for y in 4..=12 {
            paint.block(CellPos::new(14, y, 0), kit.wall);
        }
    });
    let torch = MovingLight::new(scene.loc(8, 8, 0), Real::of(2.0));
    let lights = [torch];
    let with = |x: i32, y: i32| {
        scene
            .cosmos
            .light_with(scene.loc(x, y, 0), noon(), &lights)
            .unwrap()
    };
    let two = Real::of(2.0);
    assert_eq!(with(8, 8), two);
    assert_eq!(with(10, 8), two / Real::of(5.0));
    assert_eq!(with(9, 9), two / Real::of(3.0));
    // The partition is lit, and what is behind it is not.
    assert_eq!(with(14, 8), two / Real::of(37.0));
    assert_eq!(with(15, 8), Real::ZERO);
    // Nor what is outside the room.
    assert_eq!(with(8, 2), Real::ONE);
    // The cache has not been touched by any of it.
    assert_eq!(scene.fixed(10, 8, 0), Real::ZERO);
    assert_eq!(scene.light(10, 8, 0, noon()), Real::ZERO);

    // The same, added to a level read before.
    let level = scene.light(10, 8, 0, noon());
    assert_eq!(
        scene
            .cosmos
            .add_moving_light(scene.loc(10, 8, 0), level, &lights),
        Ok(two / Real::of(5.0))
    );
    // A source far off, one in another grid and one of no intensity
    // give nothing; darkness takes away and stops at none.
    let others = [
        MovingLight::new(scene.loc(27, 13, 0), Real::of(0.5)),
        MovingLight::new(
            Location::new(GridId::new(9), CellPos::new(8, 8, 0)),
            Real::ONE,
        ),
        MovingLight::new(scene.loc(8, 8, 0), Real::ZERO),
    ];
    assert_eq!(
        scene
            .cosmos
            .add_moving_light(scene.loc(8, 8, 0), Real::of(0.25), &others),
        Ok(Real::of(0.25))
    );
    let dark = [MovingLight::new(scene.loc(8, 8, 0), Real::of(-5.0))];
    assert_eq!(
        scene
            .cosmos
            .add_moving_light(scene.loc(8, 8, 0), Real::ONE, &dark),
        Ok(Real::ZERO)
    );
    // A radius of its own is kept to.
    let narrow = [MovingLight {
        at: scene.loc(8, 8, 0),
        source: LightSource {
            intensity: Real::ONE,
            radius: Real::of(2.0),
        },
    }];
    let reach = |x: i32| {
        scene
            .cosmos
            .add_moving_light(scene.loc(x, 8, 0), Real::ZERO, &narrow)
            .unwrap()
    };
    assert_eq!(reach(10), Real::of(0.2));
    assert_eq!(reach(11), Real::ZERO);
}

// ---- The batch fill ---------------------------------------------------

/// A town of rooms with windows, doors, lamps and darkness over four
/// tracts.
fn town() -> Scene {
    scene(&four_tracts(), |kit, paint| {
        for (index, (x, y)) in [(4, 4), (20, 6), (36, 3), (6, 24), (26, 28), (44, 40)]
            .into_iter()
            .enumerate()
        {
            let (w, h) = (9 + (index as i32 % 3) * 3, 7 + (index as i32 % 2) * 4);
            paint.room(kit, (x, y), (x + w, y + h), 0);
            paint.block(CellPos::new(x, y + 3, 0), kit.window);
            paint.block(CellPos::new(x + 4, y + h, 0), kit.door_open);
            if index % 2 == 0 {
                paint.fixture(CellPos::new(x + w - 2, y + 2, 0), kit.lamp);
            }
        }
        // One room of two storeys with a hole between them.
        paint.room(kit, (26, 28), (38, 39), 1);
        paint.block(CellPos::new(26, 31, 1), kit.window);
        paint.floor(CellPos::new(30, 33, 1), FloorState::NONE);
        paint.fixture(CellPos::new(50, 20, 0), kit.gloom);
        paint.block(CellPos::new(31, 31, 0), kit.glow_block);
        paint.fixture(CellPos::new(33, 20, 0), kit.lamp);
    })
}

/// Checks a fill of a box against reading each of its cells.
fn check_fill(scene: &Scene, cells: CellBox, time: SkyTime, moving: &[MovingLight]) {
    let mut out = vec![Real::of(9.0); 3];
    scene
        .cosmos
        .fill_light(scene.grid, cells, time, moving, &mut out)
        .unwrap();
    assert_eq!(out.len() as u64, cells.volume());
    for (at, cell) in cells.cells().enumerate() {
        let loc = Location::new(scene.grid, cell);
        let single = scene
            .cosmos
            .light_with(loc, time, moving)
            .unwrap_or(Real::ZERO);
        assert_eq!(out[at], single, "{cell}");
    }
}

#[test]
fn a_fill_gives_what_asking_cell_by_cell_gives() {
    let scene = town();
    let moving = [
        MovingLight::new(scene.loc(8, 8, 0), Real::of(1.5)),
        MovingLight::new(scene.loc(31, 20, 0), Real::of(0.8)),
        MovingLight::new(scene.loc(40, 44, 0), Real::of(-0.7)),
        MovingLight::new(scene.loc(30, 34, 1), Real::of(1.0)),
        MovingLight::new(scene.loc(200, 8, 0), Real::of(1.0)),
    ];
    let boxed = |min: (i32, i32, i32), max: (i32, i32, i32)| {
        CellBox::new(
            CellPos::new(min.0, min.1, min.2),
            CellPos::new(max.0, max.1, max.2),
        )
    };
    for time in [noon(), midnight(), noon().dimmed(Real::of(0.4))] {
        // A screen across the four tracts; one that hangs over the edge
        // of what is loaded and of the realm; one cell; three storeys.
        check_fill(&scene, boxed((0, 0, 0), (63, 49, 0)), time, &moving);
        check_fill(&scene, boxed((-9, -6, 0), (70, 43, 0)), time, &moving);
        check_fill(&scene, boxed((31, 31, 0), (31, 31, 0)), time, &moving);
        check_fill(&scene, boxed((20, 22, 0), (45, 45, 2)), time, &moving);
        check_fill(&scene, boxed((20, 22, 1), (45, 45, 1)), time, &[]);
    }
}

#[test]
fn a_fill_carries_on_round_a_wrapped_axis() {
    let width = 2 * REGION_CELLS;
    let last = TractPos::new(width / TRACT_XY - 1, 0);
    let scene = scene(&[TractPos::new(0, 0), last], |kit, paint| {
        // A room across the seam, with a window on each side.
        paint.room(kit, (width - 6, 6), (width - 1, 14), 0);
        paint.room(kit, (0, 6), (6, 14), 0);
        for y in 7..=13 {
            paint.block(CellPos::new(width - 1, y, 0), BlockState::NONE);
            paint.block(CellPos::new(0, y, 0), BlockState::NONE);
        }
        paint.block(CellPos::new(6, 10, 0), kit.window);
        paint.fixture(CellPos::new(width - 3, 10, 0), kit.lamp);
    });
    // The lamp lights across the seam, and the window's light crosses
    // it the other way.
    assert_eq!(scene.fixed(1, 10, 0), Real::ONE / Real::of(17.0));
    assert_eq!(scene.steps(width - 1, 10, 0), 8 * STEP_SIDE);
    let moving = [MovingLight::new(scene.loc(2, 8, 0), Real::ONE)];
    let across = CellBox::new(CellPos::new(-20, 0, 0), CellPos::new(20, 20, 0));
    check_fill(&scene, across, noon(), &moving);
    check_fill(&scene, across, midnight(), &moving);
    let far_side = CellBox::new(
        CellPos::new(width - 20, 0, 0),
        CellPos::new(width + 20, 20, 0),
    );
    check_fill(&scene, far_side, noon(), &moving);
}

#[test]
fn light_is_the_same_whatever_order_cells_are_asked_in() {
    let forwards = town();
    let backwards = town();
    let cells = CellBox::new(CellPos::new(0, 0, 0), CellPos::new(63, 63, 1));
    let read = |scene: &Scene, cell: CellPos| {
        let loc = Location::new(scene.grid, cell);
        (
            scene.cosmos.static_light(loc).unwrap(),
            scene.cosmos.light(loc, noon()).unwrap(),
        )
    };
    let all: Vec<CellPos> = cells.cells().collect();
    let first: Vec<_> = all.iter().map(|cell| read(&forwards, *cell)).collect();
    let mut second: Vec<_> = all
        .iter()
        .rev()
        .map(|cell| read(&backwards, *cell))
        .collect();
    second.reverse();
    assert_eq!(first, second);
    // And a fill on a cosmos that has worked nothing out agrees.
    let fresh = town();
    let filled = fresh.fill((0, 0, 0), (63, 63, 1), noon());
    let levels: Vec<Real> = first.iter().map(|(_, level)| *level).collect();
    assert_eq!(filled, levels);
}

// ---- The cache against working it out afresh --------------------------

/// One random change to the town.
#[derive(Clone, Copy, Debug)]
enum Change {
    Wall,
    Window,
    Clear,
    DoorShut,
    DoorOpen,
    Lamp,
    LampOff,
    Gloom,
    NoFixture,
    Desk,
    Roof,
    NoRoof,
    Glow,
    HighRoof,
}

fn change() -> impl Strategy<Value = Change> {
    prop::sample::select(vec![
        Change::Wall,
        Change::Wall,
        Change::Window,
        Change::Clear,
        Change::Clear,
        Change::Clear,
        Change::DoorShut,
        Change::DoorOpen,
        Change::Lamp,
        Change::LampOff,
        Change::Gloom,
        Change::NoFixture,
        Change::Desk,
        Change::Roof,
        Change::NoRoof,
        Change::NoRoof,
        Change::Glow,
        Change::HighRoof,
    ])
}

fn apply(scene: &mut Scene, change: Change, x: i32, y: i32, z: i32) {
    let kit_wall = scene.kit.wall;
    match change {
        Change::Wall => scene.set_block(x, y, z, kit_wall),
        Change::Window => scene.set_block(x, y, z, scene.kit.window),
        Change::Clear => scene.set_block(x, y, z, BlockState::NONE),
        Change::DoorShut => scene.set_block(x, y, z, scene.kit.door),
        Change::DoorOpen => scene.set_block(x, y, z, scene.kit.door_open),
        Change::Lamp => scene.set_fixture(x, y, z, scene.kit.lamp),
        Change::LampOff => scene.set_fixture(x, y, z, scene.kit.lamp_off),
        Change::Gloom => scene.set_fixture(x, y, z, scene.kit.gloom),
        Change::NoFixture => scene.set_fixture(x, y, z, FixtureState::NONE),
        Change::Desk => scene.set_fixture(x, y, z, scene.kit.desk),
        Change::Roof => scene.set_floor(x, y, z + 1, scene.kit.slab),
        Change::NoRoof => scene.set_floor(x, y, z + 1, FloorState::NONE),
        Change::Glow => scene.set_block(x, y, z, scene.kit.glow_block),
        Change::HighRoof => scene.set_floor(x, y, 12, scene.kit.slab),
    }
}

/// The light of the whole town, three storeys of it, at dusk: the kept
/// cache against a copy of the cosmos that has worked nothing out.
fn assert_cache_is_fresh(scene: &Scene) {
    let dusk = SkyTime::at(Dur::from_secs(10 * 3600));
    let cells = CellBox::new(CellPos::new(0, 0, 0), CellPos::new(63, 63, 2));
    let mut kept = Vec::new();
    scene
        .cosmos
        .fill_light(scene.grid, cells, dusk, &[], &mut kept)
        .unwrap();
    let fresh = scene.cosmos.clone();
    assert_eq!(fresh.light_stats().chunks_lit, 0);
    let mut afresh = Vec::new();
    fresh
        .fill_light(scene.grid, cells, dusk, &[], &mut afresh)
        .unwrap();
    for (at, cell) in cells.cells().enumerate() {
        assert_eq!(kept[at], afresh[at], "{cell}");
    }
    // The parts kept, too, at the cells of one storey.
    for cell in cells
        .cells()
        .filter(|cell| cell.z == 0 && (cell.x + cell.y) % 3 == 0)
    {
        let loc = Location::new(scene.grid, cell);
        assert_eq!(
            scene.cosmos.static_light(loc),
            fresh.static_light(loc),
            "{cell}"
        );
    }
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(24))]

    /// Invariant P5: after any edits, what the cache holds is what
    /// working the light out from nothing gives.
    #[test]
    fn the_cache_equals_working_it_out_afresh_after_edits(
        changes in prop::collection::vec(
            (change(), 0..64_i32, 0..64_i32, 0..2_i32, any::<bool>()),
            1..14,
        ),
    ) {
        let mut scene = town();
        assert_cache_is_fresh(&scene);
        for (change, x, y, z, look) in changes {
            apply(&mut scene, change, x, y, z);
            // Sometimes several edits pile up before anyone looks.
            if look {
                assert_cache_is_fresh(&scene);
            }
        }
        assert_cache_is_fresh(&scene);
    }

    /// The same with edits crowded round one spot near where four chunks
    /// meet, so that they undo and overlap one another, and with a tract
    /// unloaded and loaded in between.
    #[test]
    fn the_cache_stays_fresh_through_crowded_edits_and_reloads(
        changes in prop::collection::vec(
            (change(), 26..38_i32, 26..38_i32, 0..2_i32, 0..12_u8),
            1..20,
        ),
    ) {
        let mut scene = town();
        assert_cache_is_fresh(&scene);
        let mut held = None;
        for (change, x, y, z, also) in changes {
            if held.is_none() || x < CHUNK_XY || y < CHUNK_XY {
                apply(&mut scene, change, x, y, z);
            }
            match (also, held.take()) {
                (0, None) => {
                    held = unload_column(&mut scene.cosmos, scene.grid, TractPos::new(1, 1));
                }
                (0..=3, Some(column)) => {
                    load_column(&mut scene.cosmos, scene.grid, TractPos::new(1, 1), column)
                        .unwrap();
                }
                (_, column) => held = column,
            }
            if also % 2 == 0 {
                assert_cache_is_fresh(&scene);
            }
        }
        assert_cache_is_fresh(&scene);
    }
}

// ---- Timing -----------------------------------------------------------

/// The time of one round of `work` in microseconds, the best of five
/// runs, and a checksum to keep the work from being optimised away.
#[allow(
    clippy::disallowed_types,
    clippy::disallowed_methods,
    reason = "a timing test reads the wall clock; nothing in the simulation does"
)]
fn time_micros(rounds: u32, mut work: impl FnMut() -> u64) -> (f64, u64) {
    let mut best = f64::INFINITY;
    let mut checksum = 0;
    for _ in 0..5 {
        let started = std::time::Instant::now();
        let mut seen = 0_u64;
        for _ in 0..rounds {
            seen = seen.wrapping_add(std::hint::black_box(work()));
        }
        best = best.min(started.elapsed().as_secs_f64() * 1.0e6 / f64::from(rounds));
        checksum = seen;
    }
    (best, checksum)
}

/// Nine tracts of open ground.
fn open_field() -> Scene {
    scene(&nine_tracts(), |_, _| {})
}

/// Nine tracts filled with one building: rooms six cells square with a
/// doorway in each wall, a window here and there in the outer wall, a
/// lamp in every third room, and a roof.
fn building() -> Scene {
    scene(&nine_tracts(), |kit, paint| {
        paint.floor_rect((0, 0), (95, 95), 1, kit.slab);
        for y in 0..96 {
            for x in 0..96 {
                let on_wall = x % 7 == 0 || y % 7 == 0;
                let doorway = (x % 7 == 3) != (y % 7 == 3);
                let outer = x == 0 || y == 0;
                if on_wall && outer && (x + y) % 5 == 3 {
                    paint.block(CellPos::new(x, y, 0), kit.window);
                } else if on_wall && !(doorway && !outer) {
                    paint.block(CellPos::new(x, y, 0), kit.wall);
                } else if x % 21 == 4 && y % 7 == 4 {
                    paint.fixture(CellPos::new(x, y, 0), kit.lamp);
                }
            }
        }
    })
}

/// Not a benchmark with a budget: rough figures, printed. Run with
/// `cargo test --release -p clash-world --test light -- --ignored --nocapture --test-threads=1`.
#[test]
#[ignore = "timing; run by hand in release mode"]
fn timing_of_light() {
    // An 80 × 50 view, a little off the lines between chunks.
    let view = CellBox::new(CellPos::new(9, 21, 0), CellPos::new(88, 70, 0));
    let total = |out: &[Real]| {
        out.iter()
            .map(|level| level.get().to_bits() & 1)
            .sum::<u64>()
    };
    for (label, mut scene) in [
        ("in the open", open_field()),
        ("inside a building", building()),
    ] {
        let mut out = Vec::new();
        let (micros, _) = time_micros(20, || {
            scene.cosmos.clear_light();
            scene
                .cosmos
                .fill_light(scene.grid, view, noon(), &[], &mut out)
                .unwrap();
            total(&out)
        });
        println!("light of an 80 × 50 view {label}, nothing kept: {micros:.1} µs");
        let (micros, _) = time_micros(2000, || {
            scene
                .cosmos
                .fill_light(scene.grid, view, noon(), &[], &mut out)
                .unwrap();
            total(&out)
        });
        println!(
            "  the same from the cache: {micros:.2} µs ({:.2} ns a cell)",
            micros * 1000.0 / 4000.0
        );
        // One edit that light cares about, then the view again.
        let mut open = false;
        let wall = scene.kit.wall;
        let (micros, _) = time_micros(200, || {
            open = !open;
            let state = if open { BlockState::NONE } else { wall };
            scene.set_block(49, 45, 0, state);
            scene
                .cosmos
                .fill_light(scene.grid, view, noon(), &[], &mut out)
                .unwrap();
            total(&out)
        });
        println!("  after one edit to a wall: {micros:.1} µs");
        // One edit that light does not care about.
        let desk = scene.kit.desk;
        let (micros, _) = time_micros(200, || {
            open = !open;
            let state = if open { FixtureState::NONE } else { desk };
            scene.set_fixture(51, 46, 0, state);
            scene
                .cosmos
                .fill_light(scene.grid, view, noon(), &[], &mut out)
                .unwrap();
            total(&out)
        });
        println!("  after one edit that moves nothing light cares about: {micros:.1} µs");
        let torch = [MovingLight::new(scene.loc(46, 46, 0), Real::ONE)];
        let (micros, _) = time_micros(2000, || {
            scene
                .cosmos
                .fill_light(scene.grid, view, noon(), &torch, &mut out)
                .unwrap();
            total(&out)
        });
        println!("  from the cache with one moving source: {micros:.2} µs");
        let at = scene.loc(46, 46, 0);
        let (micros, _) = time_micros(20_000, || {
            scene.cosmos.light(at, noon()).unwrap().get().to_bits() & 1
        });
        println!("  one cell with `light`: {:.1} ns", micros * 1000.0);
        let (micros, _) = time_micros(2000, || {
            scene
                .cosmos
                .light_with(at, noon(), &torch)
                .unwrap()
                .get()
                .to_bits()
                & 1
        });
        println!("  one cell with one moving source: {micros:.2} µs");
    }
}
