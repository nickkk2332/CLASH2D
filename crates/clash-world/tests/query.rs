//! Tests for W18: rays, lines of sight, fields of view, the geometry
//! round a cell and paths (02 §22).

mod common;

use std::cmp::Reverse;
use std::collections::{BTreeMap, BinaryHeap};
use std::sync::Arc;

use clash_core::{Dir8, IVec3, Real};
use clash_time::Timeline;
use clash_world::dims::{REGION_CELLS, TRACT_XY};
use clash_world::test_support::load_column;
use clash_world::{
    BlockState, CellFlags, CellPos, ColumnBuilder, Connection, Cosmos, Edge, ElementState,
    EngineFlag, FixtureState, FloorState, Footing, FovSet, Freshness, GridId, Layer, Location,
    MoveProfile, MoveRules, PathFail, PathFinder, PathLimits, PointM, RayBlocks, RayEnd, RayFilter,
    RayHit, RayHits, RayObstacles, RaySpan, RayTarget, StepKind, TractPos, WorldDefs, WorldError,
};
use proptest::prelude::*;

/// Test content beside the core mod's: things of other heights, things
/// that close part of a cell, and ways between storeys.
const EXTRA: &str = r#"[
  { "type": "block", "id": "hedge", "material": "wood", "height": 1.0, "fill": 0.3,
    "flags": ["opaque"] },
  { "type": "block", "id": "fence", "material": "wood", "height": 1.0, "fill": 0.05 },
  { "type": "block", "id": "bars", "material": "steel", "height": 3.0, "fill": 0.05,
    "coverage": 0.3 },
  { "type": "fixture", "id": "stairs", "material": "wood", "height": 3.0, "fill": 0.3,
    "flags": ["stairs"] },
  { "type": "fixture", "id": "ladder", "material": "wood", "height": 3.0, "fill": 0.02,
    "flags": ["ladder", "passable"] },
  { "type": "fixture", "id": "locker", "material": "steel", "height": 2.0, "fill": 0.1,
    "flags": ["opaque"] }
]"#;

struct Kit {
    defs: Arc<WorldDefs>,
    rock: BlockState,
    wall: BlockState,
    window: BlockState,
    door: BlockState,
    door_open: BlockState,
    pillar: BlockState,
    ramp: BlockState,
    hedge: BlockState,
    fence: BlockState,
    bars: BlockState,
    slab: FloorState,
    desk: FixtureState,
    rubble: FixtureState,
    stairs: FixtureState,
    ladder: FixtureState,
    locker: FixtureState,
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
        pillar: block("pillar"),
        ramp: block("ramp"),
        hedge: block("hedge"),
        fence: block("fence"),
        bars: block("bars"),
        slab: defs.floor_state("slab", None).unwrap(),
        desk: fixture("desk"),
        rubble: fixture("rubble"),
        stairs: fixture("stairs"),
        ladder: fixture("ladder"),
        locker: fixture("locker"),
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
}

struct Scene {
    kit: Kit,
    cosmos: Cosmos,
    grid: GridId,
}

impl Scene {
    fn loc(&self, x: i32, y: i32, z: i32) -> Location {
        Location::new(self.grid, CellPos::new(x, y, z))
    }

    fn point(&self, x: f64, y: f64, z: f64) -> PointM {
        PointM::new(self.grid, [Real::of(x), Real::of(y), Real::of(z)])
    }

    fn fov(&self, origin: Location, eye_alt_mm: u16, radius: f64) -> FovSet {
        let mut set = FovSet::new();
        self.cosmos
            .field_of_view(origin, eye_alt_mm, Real::of(radius), &mut set)
            .unwrap();
        set
    }
}

/// A world of the `testland` realm with the given tracts loaded: rock
/// below storey 0, nothing from there up, and whatever `paint` adds.
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
                    let builder = ColumnBuilder::new(&kit.defs, kit.rock, BlockState::NONE, 0);
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
    Scene { kit, cosmos, grid }
}

/// The four tracts that meet at cell (32, 32).
fn four_tracts() -> [TractPos; 4] {
    [
        TractPos::new(0, 0),
        TractPos::new(1, 0),
        TractPos::new(0, 1),
        TractPos::new(1, 1),
    ]
}

fn element_of(hit: &RayHit) -> (Layer, ElementState) {
    match hit.target {
        RayTarget::Element { layer, state } => (layer, state),
        RayTarget::Occupant(id) => panic!("the terrain holds no occupant {id:?}"),
    }
}

const EAST: [Real; 3] = [Real::ONE, Real::ZERO, Real::ZERO];

// ---------------------------------------------------------------- rays

#[test]
fn a_ray_at_eye_height_passes_a_fence_and_stops_at_a_wall() {
    // W18: a ray at 1.5 m passes a 1 m fence and stops at a wall.
    let scene = scene(&[TractPos::new(0, 0)], |kit, paint| {
        paint.block(CellPos::new(5, 5, 0), kit.hedge);
        paint.block(CellPos::new(7, 5, 0), kit.fence);
        paint.block(CellPos::new(9, 5, 0), kit.wall);
    });
    let high = scene.point(2.5, 5.5, 1.5);
    let low = scene.point(2.5, 5.5, 0.5);
    let far = Real::of(20.0);

    for filter in [RayFilter::SIGHT, RayFilter::MOVEMENT, RayFilter::PROJECTILE] {
        let hits = scene.cosmos.raycast(high, EAST, far, filter);
        assert_eq!(*hits.end(), RayEnd::Stopped, "{filter:?}");
        let stop = hits.stop().unwrap();
        assert_eq!(stop.loc, scene.loc(9, 5, 0));
        assert_eq!(element_of(stop), (Layer::Block, scene.kit.wall.raw()));
        assert_eq!(stop.dist_m, Real::of(6.5));
        assert_eq!(stop.point, [Real::of(9.0), Real::of(5.5), Real::of(1.5)]);
        assert_eq!(hits.length_m(), Real::of(6.5));
        assert_eq!(hits.hits().len(), 1, "nothing lower is met on the way");
    }

    // Below the top of the hedge, sight stops at it. The fence beyond is
    // not opaque and would not have stopped it.
    let hits = scene.cosmos.raycast(low, EAST, far, RayFilter::SIGHT);
    let stop = hits.stop().unwrap();
    assert_eq!(stop.loc, scene.loc(5, 5, 0));
    assert_eq!(stop.dist_m, Real::of(2.5));
    assert_eq!(element_of(stop).1, scene.kit.hedge.raw());

    // The same of lines of sight, both ways.
    let beyond = |alt: f64| scene.point(8.5, 5.5, alt);
    assert_eq!(scene.cosmos.line_of_sight(high, beyond(1.5)), Ok(true));
    assert_eq!(scene.cosmos.line_of_sight(beyond(1.5), high), Ok(true));
    assert_eq!(scene.cosmos.line_of_sight(low, beyond(0.5)), Ok(false));
    assert_eq!(scene.cosmos.line_of_sight(beyond(0.5), low), Ok(false));
    // A line that comes down behind the hedge is hidden by it.
    assert_eq!(
        scene.cosmos.line_of_sight(high, scene.point(6.5, 5.5, 0.1)),
        Ok(false)
    );
    // Nothing is seen through the wall, and the wall itself is.
    assert_eq!(
        scene
            .cosmos
            .line_of_sight(high, scene.point(11.5, 5.5, 1.5)),
        Ok(false)
    );
    assert_eq!(
        scene.cosmos.line_of_sight(high, scene.point(9.5, 5.5, 1.5)),
        Ok(true)
    );
    assert_eq!(
        scene
            .cosmos
            .line_of_sight_between(scene.loc(2, 5, 0), 1500, scene.loc(8, 5, 0), 1500),
        Ok(true)
    );
}

#[test]
fn a_ray_that_comes_down_meets_a_thing_where_it_dips_below_its_top() {
    let scene = scene(&[TractPos::new(0, 0)], |kit, paint| {
        paint.block(CellPos::new(6, 5, 0), kit.fence);
    });
    // From 2 m up, falling one metre in four: it is at 1.125 m where it
    // enters the fence's cell and reaches 1 m half a metre further on.
    let from = scene.point(2.5, 5.5, 2.0);
    let dir = [Real::of(4.0), Real::ZERO, Real::of(-1.0)];
    let hits = scene
        .cosmos
        .raycast(from, dir, Real::of(30.0), RayFilter::MOVEMENT);
    let stop = hits.stop().unwrap();
    assert_eq!(stop.loc, scene.loc(6, 5, 0));
    assert!(
        (stop.point[0].get() - 6.5).abs() < 1e-12,
        "{:?}",
        stop.point
    );
    assert!((stop.point[2].get() - 1.0).abs() < 1e-12);
    // A ray exactly level with the top passes over.
    let level = scene.point(2.5, 5.5, 1.0);
    let hits = scene
        .cosmos
        .raycast(level, EAST, Real::of(8.0), RayFilter::MOVEMENT);
    assert!(hits.reached());
    assert!(hits.hits().is_empty());
    assert_eq!(hits.length_m(), Real::of(8.0));
}

#[test]
fn the_kind_of_ray_decides_what_stops_it() {
    let scene = scene(&[TractPos::new(0, 0)], |kit, paint| {
        paint.block(CellPos::new(4, 5, 0), kit.door_open);
        paint.block(CellPos::new(6, 5, 0), kit.pillar);
        paint.fixture(CellPos::new(7, 5, 0), kit.desk);
        paint.block(CellPos::new(8, 5, 0), kit.bars);
        paint.block(CellPos::new(10, 5, 0), kit.window);
        paint.block(CellPos::new(12, 5, 0), kit.door);
    });
    let from = scene.point(2.5, 5.5, 0.6);
    let cast = |filter| scene.cosmos.raycast(from, EAST, Real::of(30.0), filter);

    // Sight passes everything that is not opaque: it stops at the door.
    let sight = cast(RayFilter::SIGHT);
    assert_eq!(sight.hits().len(), 1);
    assert_eq!(sight.stop().unwrap().loc, scene.loc(12, 5, 0));

    // Movement passes the open door, which is passable, and stops at
    // the pillar.
    let movement = cast(RayFilter::MOVEMENT);
    assert_eq!(movement.hits().len(), 1);
    assert_eq!(movement.stop().unwrap().loc, scene.loc(6, 5, 0));

    // A projectile is not met by the open door (coverage 0); it meets
    // the pillar, which closes half the cell, the desk, which closes all
    // of it below its top, and stops there.
    let shot = cast(RayFilter::PROJECTILE);
    let met: Vec<(i32, Real, bool)> = shot
        .hits()
        .iter()
        .map(|hit| (hit.loc.cell.x, hit.coverage, hit.stops))
        .collect();
    assert_eq!(met, [(6, Real::of(0.5), false), (7, Real::ONE, true)]);

    // Above the desk it goes on through the bars, which close three
    // tenths, to the window, which closes everything.
    let higher = scene.point(2.5, 5.5, 1.2);
    let shot = scene
        .cosmos
        .raycast(higher, EAST, Real::of(30.0), RayFilter::PROJECTILE);
    let met: Vec<(i32, bool)> = shot
        .hits()
        .iter()
        .map(|hit| (hit.loc.cell.x, hit.stops))
        .collect();
    assert_eq!(met, [(6, false), (8, false), (10, true)]);
    assert!((shot.hits()[1].coverage.get() - 0.3).abs() < 1e-12);
    // Distances come in order.
    assert!(
        shot.hits()
            .windows(2)
            .all(|pair| pair[0].dist_m <= pair[1].dist_m)
    );
}

#[test]
fn floors_stop_rays_between_storeys_and_holes_do_not() {
    let scene = scene(&[TractPos::new(0, 0)], |kit, paint| {
        paint.floor_rect((0, 0), (20, 20), 1, kit.slab);
        paint.floor(CellPos::new(10, 10, 1), FloorState::NONE);
    });
    let down = [Real::ZERO, Real::ZERO, Real::of(-1.0)];
    let up = [Real::ZERO, Real::ZERO, Real::ONE];
    for filter in [RayFilter::SIGHT, RayFilter::MOVEMENT, RayFilter::PROJECTILE] {
        // Down from the upper storey: stopped by its own floor.
        let hits = scene
            .cosmos
            .raycast(scene.point(5.5, 5.5, 4.5), down, Real::of(9.0), filter);
        let stop = hits.stop().unwrap();
        assert_eq!(stop.loc, scene.loc(5, 5, 1));
        assert_eq!(element_of(stop), (Layer::Floor, scene.kit.slab.raw()));
        assert_eq!(stop.dist_m, Real::of(1.5));
        // Up from below: stopped by the floor of the cell above.
        let hits = scene
            .cosmos
            .raycast(scene.point(5.5, 5.5, 1.0), up, Real::of(9.0), filter);
        let stop = hits.stop().unwrap();
        assert_eq!(stop.loc, scene.loc(5, 5, 1));
        assert_eq!(stop.dist_m, Real::of(2.0));
        // Through the hole: down to the rock, which is the block of the
        // cell below storey 0.
        let hits = scene
            .cosmos
            .raycast(scene.point(10.5, 10.5, 4.5), down, Real::of(9.0), filter);
        let stop = hits.stop().unwrap();
        assert_eq!(stop.loc, scene.loc(10, 10, -1));
        assert_eq!(element_of(stop), (Layer::Block, scene.kit.rock.raw()));
        assert_eq!(stop.dist_m, Real::of(4.5));
    }
    let above = scene.point(10.5, 10.5, 4.0);
    assert_eq!(
        scene
            .cosmos
            .line_of_sight(above, scene.point(10.5, 10.5, 1.0)),
        Ok(true)
    );
    assert_eq!(
        scene
            .cosmos
            .line_of_sight(scene.point(5.5, 5.5, 4.0), scene.point(5.5, 5.5, 1.0)),
        Ok(false)
    );
}

#[test]
fn a_ray_ends_where_loaded_space_ends_and_says_why() {
    let scene = scene(&[TractPos::new(0, 0)], |_, _| {});
    let from = scene.point(30.5, 5.5, 1.5);
    // East into a tract that is not loaded.
    let hits = scene
        .cosmos
        .raycast(from, EAST, Real::of(10.0), RayFilter::SIGHT);
    let unloaded = WorldError::NotLoaded(scene.loc(TRACT_XY, 5, 0));
    assert_eq!(*hits.end(), RayEnd::Refused(unloaded.clone()));
    assert_eq!(hits.length_m(), Real::of(1.5));
    assert!(hits.hits().is_empty() && hits.stop().is_none());
    assert_eq!(
        scene
            .cosmos
            .line_of_sight(from, scene.point(40.5, 5.5, 1.5)),
        Err(unloaded)
    );
    // North over the edge of the realm.
    let north = [Real::ZERO, Real::of(-1.0), Real::ZERO];
    let hits = scene
        .cosmos
        .raycast(from, north, Real::of(10.0), RayFilter::MOVEMENT);
    assert_eq!(
        *hits.end(),
        RayEnd::Refused(WorldError::OutsideRealm {
            loc: scene.loc(30, -1, 0),
            edge: Edge::North
        })
    );
    assert_eq!(hits.length_m(), Real::of(5.5));
    // A grid that does not exist, a ray with no direction, a ray with
    // no length: none of them panics.
    let nowhere = PointM::new(GridId::new(99), from.pos);
    let hits = scene
        .cosmos
        .raycast(nowhere, EAST, Real::of(5.0), RayFilter::SIGHT);
    assert_eq!(
        *hits.end(),
        RayEnd::Refused(WorldError::UnknownGrid(GridId::new(99)))
    );
    assert!(scene.cosmos.line_of_sight(nowhere, nowhere).is_err());
    let still = scene
        .cosmos
        .raycast(from, [Real::ZERO; 3], Real::of(5.0), RayFilter::SIGHT);
    assert!(still.reached() && still.length_m() == Real::ZERO);
    let short = scene
        .cosmos
        .raycast(from, EAST, Real::of(-1.0), RayFilter::SIGHT);
    assert!(short.reached() && short.hits().is_empty());
    // A line of sight from a point to itself is clear.
    assert_eq!(scene.cosmos.line_of_sight(from, from), Ok(true));
}

#[test]
fn rays_carry_on_round_a_wrapped_axis() {
    // The realm is two regions round along x.
    let width = 2 * REGION_CELLS;
    let last = TractPos::new(width / TRACT_XY - 1, 0);
    let scene = scene(&[TractPos::new(0, 0), last], |kit, paint| {
        paint.block(CellPos::new(width - 6, 5, 0), kit.wall);
    });
    let from = scene.point(2.5, 5.5, 1.5);
    let west = [Real::of(-1.0), Real::ZERO, Real::ZERO];
    let hits = scene
        .cosmos
        .raycast(from, west, Real::of(20.0), RayFilter::SIGHT);
    let stop = hits.stop().unwrap();
    assert_eq!(stop.loc, scene.loc(width - 6, 5, 0));
    assert_eq!(stop.dist_m, Real::of(7.5));
    // The point carries on past the edge in the ray's own frame.
    assert_eq!(stop.point[0], Real::of(-5.0));
    // A line of sight takes the shorter way round, whichever end is
    // given first.
    let near = scene.point(1.5, 5.5, 1.5);
    let across = scene.point(f64::from(width) - 2.5, 5.5, 1.5);
    let hidden = scene.point(f64::from(width) - 8.5, 5.5, 1.5);
    assert_eq!(scene.cosmos.line_of_sight(near, across), Ok(true));
    assert_eq!(scene.cosmos.line_of_sight(across, near), Ok(true));
    assert_eq!(scene.cosmos.line_of_sight(near, hidden), Ok(false));
    assert_eq!(scene.cosmos.line_of_sight(hidden, near), Ok(false));
}

/// Something standing in a cell, reported the way the occupant index
/// will report it.
struct Bystander {
    at: Location,
    stops: bool,
}

impl RayObstacles for Bystander {
    fn in_cell(&mut self, loc: Location, span: &RaySpan, out: &mut Vec<RayHit>) {
        if loc == self.at {
            let middle = (span.enter_m + span.exit_m) / Real::of(2.0);
            let id = clash_core::EntityId::new(4, 0);
            out.push(RayHit::occupant(loc, id, middle, Real::ONE, self.stops));
        }
    }
}

#[test]
fn what_else_stands_in_the_way_joins_the_ray_in_order() {
    let scene = scene(&[TractPos::new(0, 0)], |kit, paint| {
        paint.block(CellPos::new(6, 5, 0), kit.pillar);
        paint.block(CellPos::new(9, 5, 0), kit.wall);
    });
    let from = scene.point(2.5, 5.5, 1.5);
    let mut hits = RayHits::new();
    let mut passer = Bystander {
        at: scene.loc(7, 5, 0),
        stops: false,
    };
    scene.cosmos.raycast_into(
        from,
        EAST,
        Real::of(20.0),
        RayFilter::PROJECTILE,
        &mut passer,
        &mut hits,
    );
    let cells: Vec<i32> = hits.hits().iter().map(|hit| hit.loc.cell.x).collect();
    assert_eq!(cells, [6, 7, 9]);
    assert_eq!(
        hits.hits()[1].target,
        RayTarget::Occupant(clash_core::EntityId::new(4, 0))
    );
    assert_eq!(hits.hits()[1].point[0], Real::of(7.5));

    // One that stops the ray ends it there. The buffer is the same one.
    let mut blocker = Bystander {
        at: scene.loc(4, 5, 0),
        stops: true,
    };
    scene.cosmos.raycast_into(
        from,
        EAST,
        Real::of(20.0),
        RayFilter::PROJECTILE,
        &mut blocker,
        &mut hits,
    );
    assert_eq!(hits.hits().len(), 1);
    assert_eq!(hits.stop().unwrap().dist_m, Real::of(2.0));
    assert_eq!(*hits.end(), RayEnd::Stopped);
}

// ------------------------------------------------------- random maps

/// The size of a random map, and where its low corner is: placed so
/// that it lies across the corner where four tracts meet.
const MAP: i32 = 14;
const MAP_LOW: i32 = TRACT_XY - MAP / 2;
const MAP_CELLS: usize = (MAP * MAP) as usize;

/// What one cell of a random map holds.
#[derive(Clone, Copy, Debug)]
struct MapCell {
    block: u8,
    fixture: u8,
    floor: bool,
}

fn map_cell() -> impl Strategy<Value = MapCell> {
    // Mostly empty, so that there is something to see and somewhere to
    // walk.
    (0_u8..24, 0_u8..12, any::<bool>()).prop_map(|(block, fixture, floor)| MapCell {
        block,
        fixture,
        floor,
    })
}

/// Two storeys of random cells.
fn random_map() -> impl Strategy<Value = Vec<MapCell>> {
    prop::collection::vec(map_cell(), 2 * MAP_CELLS)
}

fn map_pos(index: usize) -> CellPos {
    let index = index as i32;
    let area = MAP * MAP;
    CellPos::new(
        MAP_LOW + index % MAP,
        MAP_LOW + (index % area) / MAP,
        index / area,
    )
}

/// Builds a random map of two storeys. The lower stands on rock, with
/// a floor laid in some cells; the upper has a floor in some cells and
/// is open air in the rest. `ways_up` adds stairs and ladders.
fn map_scene(map: &[MapCell], ways_up: bool) -> Scene {
    scene(&four_tracts(), |kit, paint| {
        for (index, cell) in map.iter().enumerate() {
            let pos = map_pos(index);
            let block = match cell.block {
                0..=2 => kit.wall,
                3 => kit.hedge,
                4 => kit.fence,
                5 => kit.pillar,
                6 => kit.bars,
                7 => kit.window,
                8 => kit.door_open,
                9 => kit.door,
                _ => BlockState::NONE,
            };
            let fixture = match cell.fixture {
                0 => kit.desk,
                1 => kit.locker,
                2 => kit.rubble,
                3 if ways_up && pos.z == 0 => kit.stairs,
                4 if ways_up && pos.z == 0 => kit.ladder,
                _ => FixtureState::NONE,
            };
            paint.block(pos, block);
            paint.fixture(pos, fixture);
            if cell.floor {
                paint.floor(pos, kit.slab);
            }
        }
    })
}

/// A ray's meeting with one element, worked out the slow way.
#[derive(Clone, Copy, Debug)]
struct Met {
    cell: CellPos,
    layer: Layer,
    dist: f64,
    stops: bool,
}

/// Where a ray enters and leaves a box, if it runs inside it for some
/// length.
fn box_span(
    origin: [f64; 3],
    dir: [f64; 3],
    max: f64,
    low: [f64; 3],
    high: [f64; 3],
) -> Option<(f64, f64)> {
    let mut enter = 0.0_f64;
    let mut exit = max;
    for axis in 0..3 {
        if dir[axis] == 0.0 {
            if origin[axis] < low[axis] || origin[axis] >= high[axis] {
                return None;
            }
            continue;
        }
        let a = (low[axis] - origin[axis]) / dir[axis];
        let b = (high[axis] - origin[axis]) / dir[axis];
        enter = enter.max(a.min(b));
        exit = exit.min(a.max(b));
    }
    (exit - enter > 1e-9).then_some((enter, exit))
}

/// What a ray meets, by testing it against the box of every element
/// anywhere near it: no walk from cell to cell, so nothing in common
/// with how `raycast` finds its way.
fn reference_ray(
    scene: &Scene,
    origin: [f64; 3],
    dir: [f64; 3],
    max: f64,
    blocks: RayBlocks,
) -> Vec<Met> {
    let defs = &scene.kit.defs;
    let end = [
        origin[0] + dir[0] * max,
        origin[1] + dir[1] * max,
        origin[2] + dir[2] * max,
    ];
    let cell_of = |p: [f64; 3]| {
        CellPos::new(
            p[0].floor() as i32,
            p[1].floor() as i32,
            (p[2] / 3.0).floor() as i32,
        )
    };
    let (a, b) = (cell_of(origin), cell_of(end));
    let start = a;
    let mut met = Vec::new();
    for z in a.z.min(b.z) - 1..=a.z.max(b.z) + 1 {
        for y in a.y.min(b.y) - 1..=a.y.max(b.y) + 1 {
            for x in a.x.min(b.x) - 1..=a.x.max(b.x) + 1 {
                let cell = CellPos::new(x, y, z);
                let view = scene.cosmos.cell(Location::new(scene.grid, cell)).unwrap();
                let base = [f64::from(x), f64::from(y), f64::from(z) * 3.0];
                let elements = [
                    (Layer::Block, view.block.raw()),
                    (Layer::Fixture, view.fixture.raw()),
                ];
                for (layer, state) in elements {
                    if state.is_none() || cell == start {
                        continue;
                    }
                    let info = defs.info(layer, state);
                    let height = info.height.get();
                    let top = [base[0] + 1.0, base[1] + 1.0, base[2] + height];
                    let Some((enter, _)) = box_span(origin, dir, max, base, top) else {
                        continue;
                    };
                    let stops = match blocks {
                        RayBlocks::Sight if info.flags.has(EngineFlag::Opaque) => true,
                        RayBlocks::Movement if !info.flags.has(EngineFlag::Passable) => true,
                        RayBlocks::Projectile => {
                            let share = info.coverage.get() * 3.0 / height;
                            if share <= 0.0 {
                                continue;
                            }
                            share >= 1.0
                        }
                        _ => continue,
                    };
                    met.push(Met {
                        cell,
                        layer,
                        dist: enter,
                        stops,
                    });
                }
                // The floor is the base of the cell: met where the ray
                // passes through that plane inside the cell's footprint.
                if !view.floor.is_none() && dir[2] != 0.0 {
                    let dist = (base[2] - origin[2]) / dir[2];
                    let px = origin[0] + dir[0] * dist;
                    let py = origin[1] + dir[1] * dist;
                    let inside =
                        px > base[0] && px < base[0] + 1.0 && py > base[1] && py < base[1] + 1.0;
                    if dist > 1e-9 && dist <= max && inside {
                        met.push(Met {
                            cell,
                            layer: Layer::Floor,
                            dist,
                            stops: true,
                        });
                    }
                }
            }
        }
    }
    // Where a floor and what lies beyond it are met at the same moment,
    // the floor comes first.
    let rank = |hit: &Met| hit.layer.index();
    met.sort_by(|a, b| a.dist.total_cmp(&b.dist).then(rank(a).cmp(&rank(b))));
    if let Some(at) = met.iter().position(|hit| hit.stops) {
        met.truncate(at + 1);
    }
    met
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(96))]

    /// A ray meets what a test of it against every element's box says it
    /// meets, at the same distances, for each kind of ray.
    #[test]
    fn rays_match_a_brute_force_reference(
        map in random_map(),
        rays in prop::collection::vec(
            ((0.05_f64..0.95, 0.05_f64..0.95, 0.02_f64..0.98), (0..MAP, 0..MAP, 0..2_i32),
             (-1.0_f64..1.0, -1.0_f64..1.0, -0.6_f64..0.6), 1.0_f64..12.0),
            12,
        ),
    ) {
        let scene = map_scene(&map, false);
        for ((fx, fy, fz), (cx, cy, cz), (dx, dy, dz), max) in rays {
            let length = (dx * dx + dy * dy + dz * dz).sqrt();
            prop_assume!(length > 1e-3);
            let dir = [dx / length, dy / length, dz / length];
            let origin = [
                f64::from(MAP_LOW + cx) + fx,
                f64::from(MAP_LOW + cy) + fy,
                (f64::from(cz) + fz) * 3.0,
            ];
            let from = scene.point(origin[0], origin[1], origin[2]);
            let cast_dir = dir.map(Real::of);
            for blocks in [RayBlocks::Sight, RayBlocks::Movement, RayBlocks::Projectile] {
                let expected = reference_ray(&scene, origin, dir, max, blocks);
                let hits = scene.cosmos.raycast(from, cast_dir, Real::of(max), RayFilter::new(blocks));
                let got: Vec<Met> = hits.hits().iter().map(|hit| {
                    let (layer, _) = element_of(hit);
                    Met { cell: hit.loc.cell, layer, dist: hit.dist_m.get(), stops: hit.stops }
                }).collect();
                prop_assert_eq!(got.len(), expected.len(), "{:?}: {:?} against {:?}", blocks, got, expected);
                for (got, expected) in got.iter().zip(&expected) {
                    prop_assert_eq!((got.cell, got.layer, got.stops), (expected.cell, expected.layer, expected.stops));
                    prop_assert!((got.dist - expected.dist).abs() < 1e-9, "{:?} against {:?}", got, expected);
                }
                let stopped = expected.last().is_some_and(|hit| hit.stops);
                prop_assert_eq!(hits.end(), if stopped { &RayEnd::Stopped } else { &RayEnd::Reached });
                let length = if stopped { expected.last().unwrap().dist } else { max };
                prop_assert!((hits.length_m().get() - length).abs() < 1e-9);
                // Where it was met is on or in the cell that holds it.
                for hit in hits.hits() {
                    let cell = hit.loc.cell;
                    let low = [f64::from(cell.x), f64::from(cell.y), f64::from(cell.z) * 3.0];
                    let size = [1.0, 1.0, 3.0];
                    for axis in 0..3 {
                        let at = hit.point[axis].get();
                        prop_assert!(at > low[axis] - 1e-9 && at < low[axis] + size[axis] + 1e-9);
                    }
                }
            }
        }
    }

    /// A line of sight is the same from either end, and is what a sight
    /// ray from one end to the other says.
    #[test]
    fn lines_of_sight_go_both_ways(
        map in random_map(),
        pairs in prop::collection::vec(
            ((0..MAP, 0..MAP, 0..2_i32, 100_u16..2900), (0..MAP, 0..MAP, 0..2_i32, 100_u16..2900)),
            24,
        ),
    ) {
        let scene = map_scene(&map, false);
        for ((ax, ay, az, a_alt), (bx, by, bz, b_alt)) in pairs {
            let a = scene.loc(MAP_LOW + ax, MAP_LOW + ay, az);
            let b = scene.loc(MAP_LOW + bx, MAP_LOW + by, bz);
            let forward = scene.cosmos.line_of_sight_between(a, a_alt, b, b_alt);
            let backward = scene.cosmos.line_of_sight_between(b, b_alt, a, a_alt);
            prop_assert_eq!(&forward, &backward);
            if a.cell == b.cell {
                prop_assert_eq!(forward, Ok(true));
                continue;
            }
            // A sight ray from one end stops short of the other end's
            // cell exactly when there is no line of sight.
            let (from, to) = (a.point_at(a_alt), b.point_at(b_alt));
            let offset = [to.pos[0] - from.pos[0], to.pos[1] - from.pos[1], to.pos[2] - from.pos[2]];
            let length = (offset[0] * offset[0] + offset[1] * offset[1] + offset[2] * offset[2]).sqrt();
            let hits = scene.cosmos.raycast(from, offset, length, RayFilter::SIGHT);
            let clear = match hits.stop() {
                None => true,
                Some(stop) => stop.loc == b && element_of(stop).0 != Layer::Floor,
            };
            prop_assert_eq!(forward, Ok(clear), "{:?} to {:?}: {:?}", a, b, hits);
        }
    }
}

// ------------------------------------------------------ field of view

/// Whether a cell blocks sight at eye level, read from the definitions.
fn blocks_sight(scene: &Scene, loc: Location, eye_mm: u16) -> bool {
    let view = scene.cosmos.cell(loc).unwrap();
    let eye = f64::from(eye_mm) / 1000.0;
    let defs = &scene.kit.defs;
    [defs.block_info(view.block), defs.fixture_info(view.fixture)]
        .iter()
        .any(|info| info.flags.has(EngineFlag::Opaque) && info.height.get() > eye)
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(48))]

    /// W18: field of view is symmetric on random maps. Between any two
    /// cells that do not block sight, on one storey or two, each sees
    /// the other or neither does.
    #[test]
    fn field_of_view_is_symmetric(
        map in random_map(),
        eye_mm in prop::sample::select(vec![300_u16, 1500, 2500]),
        radius in prop::sample::select(vec![4.0_f64, 7.5, 11.0]),
    ) {
        let scene = map_scene(&map, false);
        // Every cell of the map, and some of the air over it, where a
        // storey holds nothing and is seen whole.
        let in_the_air = (2 * MAP_CELLS..4 * MAP_CELLS).filter(|index| index % 5 == 0);
        let viewers: Vec<Location> = (0..2 * MAP_CELLS)
            .chain(in_the_air)
            .map(|index| Location::new(scene.grid, map_pos(index)))
            .filter(|loc| !blocks_sight(&scene, *loc, eye_mm))
            .collect();
        let views: Vec<FovSet> = viewers.iter().map(|loc| scene.fov(*loc, eye_mm, radius)).collect();
        for (a, a_view) in viewers.iter().zip(&views) {
            prop_assert!(a_view.contains(*a), "a viewer sees its own cell");
            for (b, b_view) in viewers.iter().zip(&views) {
                prop_assert_eq!(
                    a_view.contains(*b), b_view.contains(*a),
                    "{:?} and {:?} at eye {} radius {}", a.cell, b.cell, eye_mm, radius
                );
            }
        }
        // What a view lists is what it contains, in order, once each,
        // and all within the radius.
        for (viewer, view) in viewers.iter().zip(&views).take(8) {
            let offsets: Vec<IVec3> = view.offsets().collect();
            prop_assert_eq!(offsets.len(), view.len());
            let in_order = offsets.windows(2).all(|pair| {
                (pair[0].z, pair[0].y, pair[0].x) < (pair[1].z, pair[1].y, pair[1].x)
            });
            prop_assert!(in_order);
            for (offset, loc) in offsets.iter().zip(view.cells()) {
                prop_assert!(view.contains_offset(*offset));
                prop_assert_eq!(loc.cell, viewer.cell.offset(*offset));
                let (x, y, up) = (offset.x, offset.y, 3 * offset.z);
                let reach = f64::from(x * x + y * y + up * up);
                prop_assert!(reach <= radius * radius);
            }
        }
    }
}

#[test]
fn a_view_in_the_open_is_a_disc_and_walls_hide_what_is_behind_them() {
    let scene = scene(&four_tracts(), |kit, paint| {
        paint.floor_rect((0, 0), (63, 63), 0, kit.slab);
        // A long wall four cells east of the viewer.
        for y in 0..64 {
            paint.block(CellPos::new(36, y, 0), kit.wall);
        }
        // A roof over everything, so that there is no looking up.
        paint.floor_rect((0, 0), (63, 63), 1, kit.slab);
    });
    let origin = scene.loc(32, 32, 0);
    let view = scene.fov(origin, 1500, 6.0);
    assert_eq!(view.origin(), Some(origin));
    assert_eq!((view.eye_alt_mm(), view.radius_m()), (1500, Real::of(6.0)));
    assert_eq!(view.storeys(), Some((0, 0)));
    for dy in -7..=7 {
        for dx in -7..=7 {
            let inside = dx * dx + dy * dy <= 36;
            // West of the wall everything within the radius is seen;
            // the wall is seen; nothing east of it is.
            let expected = inside && dx <= 4;
            assert_eq!(
                view.contains(scene.loc(32 + dx, 32 + dy, 0)),
                expected,
                "offset ({dx}, {dy})"
            );
        }
    }
    // A cell exactly at the radius is seen.
    assert!(view.contains(scene.loc(26, 32, 0)));
    assert!(!view.contains(scene.loc(25, 32, 0)));
    // Not another grid's cells, whatever their coordinates.
    assert!(!view.contains(Location::new(GridId::new(9), origin.cell)));
}

#[test]
fn eye_height_decides_what_is_seen_over() {
    let scene = scene(&[TractPos::new(0, 0)], |kit, paint| {
        paint.floor_rect((0, 0), (31, 31), 0, kit.slab);
        paint.floor_rect((0, 0), (31, 31), 1, kit.slab);
        for y in 0..TRACT_XY {
            paint.block(CellPos::new(12, y, 0), kit.hedge);
            paint.fixture(CellPos::new(16, y, 0), kit.locker);
        }
        paint.block(CellPos::new(8, 10, 0), kit.door);
        paint.block(CellPos::new(8, 12, 0), kit.door_open);
        paint.block(CellPos::new(8, 14, 0), kit.window);
    });
    let origin = scene.loc(10, 10, 0);
    let standing = scene.fov(origin, 1500, 12.0);
    let prone = scene.fov(origin, 300, 12.0);
    // Standing sees over a hedge of one metre and not over a locker of
    // two; lying down, the hedge is a wall.
    assert!(standing.contains(scene.loc(12, 10, 0)));
    assert!(standing.contains(scene.loc(14, 10, 0)));
    assert!(standing.contains(scene.loc(16, 10, 0)));
    assert!(!standing.contains(scene.loc(18, 10, 0)));
    assert!(prone.contains(scene.loc(12, 10, 0)));
    assert!(!prone.contains(scene.loc(14, 10, 0)));
    // A closed door hides what is behind it; an open one and a window
    // do not.
    assert!(standing.contains(scene.loc(8, 10, 0)));
    assert!(!standing.contains(scene.loc(6, 10, 0)));
    let by_open_door = scene.fov(scene.loc(10, 12, 0), 1500, 12.0);
    assert!(by_open_door.contains(scene.loc(6, 12, 0)));
    let by_window = scene.fov(scene.loc(10, 14, 0), 1500, 12.0);
    assert!(by_window.contains(scene.loc(6, 14, 0)));
}

#[test]
fn a_view_goes_down_through_openings_and_up_through_open_sky() {
    // A flat-roofed building beside a street. The roof is the floor of
    // storey 1; the street is storey 0 under open sky.
    let scene = scene(&four_tracts(), |kit, paint| {
        paint.floor_rect((0, 0), (63, 63), 0, kit.slab);
        paint.outline((20, 20), (30, 30), 0, kit.wall);
        paint.floor_rect((20, 20), (30, 30), 1, kit.slab);
        // A hole in the roof, over the middle of the room.
        paint.floor(CellPos::new(25, 25, 1), FloorState::NONE);
    });
    let roof = scene.loc(22, 22, 1);
    let street = scene.loc(34, 22, 0);
    let room = scene.loc(25, 25, 0);
    let corner = scene.loc(22, 28, 0);

    let from_roof = scene.fov(roof, 1500, 20.0);
    // Down to the street beyond the edge of the roof, and down through
    // the hole into the room, but not into the rest of the room.
    assert!(from_roof.contains(street));
    assert!(from_roof.contains(room));
    assert!(!from_roof.contains(corner));
    assert!(
        !from_roof.contains(scene.loc(22, 22, 0)),
        "not through the roof underfoot"
    );
    assert_eq!(from_roof.storeys().map(|(low, _)| low), Some(-1));

    // And back: from the street, up through the open sky; from under the
    // hole, up through it; from the corner of the room, nothing above.
    let from_street = scene.fov(street, 1500, 20.0);
    assert!(from_street.contains(roof));
    assert!(
        from_street.contains(scene.loc(34, 22, 1)),
        "the air overhead"
    );
    let from_room = scene.fov(room, 1500, 20.0);
    assert!(from_room.contains(roof));
    let from_corner = scene.fov(corner, 1500, 20.0);
    assert!(!from_corner.contains(roof));
    assert_eq!(from_corner.storeys(), Some((0, 0)));

    // The storeys of nothing but air over the street are seen whole:
    // everything within the radius, and nothing beyond it.
    for up in 2..=6 {
        for dy in -21..=21 {
            for dx in -21..=21 {
                let inside = dx * dx + dy * dy + 9 * up * up <= 400;
                assert_eq!(
                    from_street.contains_offset(IVec3::new(dx, dy, up)),
                    inside,
                    "offset ({dx}, {dy}, {up})"
                );
            }
        }
    }
    assert_eq!(from_street.storeys(), Some((0, 6)));

    // Standing on bare rock with no floor laid, the ground itself is
    // what is seen below.
    let bare = scene_bare();
    let view = bare.fov(bare.loc(5, 5, 0), 1500, 4.0);
    assert!(view.contains(bare.loc(5, 5, -1)));
    assert!(view.contains(bare.loc(6, 5, -1)));
    assert!(!view.contains(bare.loc(5, 5, -2)));
}

fn scene_bare() -> Scene {
    scene(&[TractPos::new(0, 0)], |_, _| {})
}

#[test]
fn a_view_stops_at_unloaded_space_and_carries_on_round_a_wrap() {
    let width = 2 * REGION_CELLS;
    let last = TractPos::new(width / TRACT_XY - 1, 0);
    let scene = scene(&[TractPos::new(0, 0), last], |kit, paint| {
        paint.floor_rect((0, 0), (31, 31), 0, kit.slab);
        paint.floor_rect((width - 32, 0), (width - 1, 31), 0, kit.slab);
        paint.floor_rect((0, 0), (31, 31), 1, kit.slab);
        paint.floor_rect((width - 32, 0), (width - 1, 31), 1, kit.slab);
    });
    let origin = scene.loc(2, 30, 0);
    let view = scene.fov(origin, 1500, 8.0);
    // West across the seam.
    assert!(view.contains(scene.loc(width - 3, 30, 0)));
    assert!(view.contains_offset(IVec3::new(-5, 0, 0)));
    // South is a tract that is not loaded: not seen, and no panic.
    assert!(!view.contains(scene.loc(2, 33, 0)));
    assert!(view.cells().all(|loc| scene.cosmos.is_loaded(loc)));
    assert!(view.cells().any(|loc| loc.cell.x > width / 2));

    // A viewer that is nowhere gets an error and an empty view.
    let mut set = view.clone();
    let nowhere = scene.loc(2, 40, 0);
    assert_eq!(
        scene
            .cosmos
            .field_of_view(nowhere, 1500, Real::of(8.0), &mut set),
        Err(WorldError::NotLoaded(nowhere))
    );
    assert!(set.is_empty() && set.origin().is_none() && !set.contains(origin));
    assert_eq!(set.offsets().count(), 0);
    let off_grid = Location::new(GridId::new(77), origin.cell);
    assert!(
        scene
            .cosmos
            .field_of_view(off_grid, 1500, Real::of(8.0), &mut set)
            .is_err()
    );
    // The same buffer serves again.
    assert_eq!(
        scene
            .cosmos
            .field_of_view(origin, 1500, Real::of(8.0), &mut set),
        Ok(Freshness::Computed)
    );
    assert_eq!(set.len(), view.len());
    assert!(set.offsets().eq(view.offsets()));
    assert_eq!(
        scene
            .cosmos
            .field_of_view(origin, 1500, Real::of(8.0), &mut set),
        Ok(Freshness::Reused)
    );
    assert!(set.is_current(&scene.cosmos));
    // A radius of nothing sees the viewer's own cell, and a huge one
    // does not panic.
    assert_eq!(scene.fov(origin, 1500, 0.0).len(), 1);
    assert!(scene.fov(origin, 1500, 1.0e9).len() > view.len());
}

// --------------------------------------------------- geometry and paths

#[test]
fn neighbours_report_the_geometry_round_a_cell() {
    let scene = scene(&[TractPos::new(0, 0)], |kit, paint| {
        paint.floor(CellPos::new(10, 10, 0), kit.slab);
        paint.block(CellPos::new(10, 9, 0), kit.wall); // north
        paint.block(CellPos::new(11, 10, 0), kit.door); // east
        paint.fixture(CellPos::new(10, 11, 0), kit.rubble); // south
        paint.fixture(CellPos::new(9, 10, 0), kit.desk); // west
        paint.block(CellPos::new(11, 11, 0), kit.ramp); // south-east
        paint.fixture(CellPos::new(9, 11, 0), kit.stairs); // south-west
        paint.fixture(CellPos::new(9, 9, 0), kit.ladder); // north-west
        paint.block(CellPos::new(11, 9, 0), kit.door_open); // north-east
    });
    let around = scene.cosmos.neighbours(scene.loc(10, 10, 0)).unwrap();
    assert_eq!(around.here.cell, CellPos::new(10, 10, 0));
    assert_eq!(around.here.footing, Footing::Floor);
    assert_eq!(around.here.obstacle_m, Real::ZERO);

    let at = |dir: Dir8| around.around[dir.index()].unwrap();
    // A wall: in the way for good.
    let wall = at(Dir8::North);
    assert_eq!(wall.cell, CellPos::new(10, 9, 0));
    assert!(wall.walk_blocked() && !wall.opens);
    assert_eq!(wall.obstacle_m, Real::of(3.0));
    assert_eq!(wall.footing, Footing::Ground);
    // A closed door: in the way, but it could stand aside.
    let door = at(Dir8::East);
    assert!(door.walk_blocked() && door.opens);
    // An open one is not in the way at all.
    let open = at(Dir8::NorthEast);
    assert!(!open.walk_blocked() && !open.opens);
    assert_eq!(open.obstacle_m, Real::ZERO);
    // Rubble is rough and low enough to step over.
    let rubble = at(Dir8::South);
    assert!(rubble.rough && !rubble.walk_blocked());
    assert_eq!(rubble.obstacle_m, Real::of(0.5));
    // A desk is in the way of walking, 0.8 m of it.
    let desk = at(Dir8::West);
    assert!(desk.walk_blocked() && !desk.opens && !desk.rough);
    assert_eq!(desk.obstacle_m, Real::of(0.8));
    // The three ways up.
    assert_eq!(at(Dir8::SouthEast).connection, Some(Connection::Ramp));
    assert_eq!(at(Dir8::SouthWest).connection, Some(Connection::Stairs));
    assert_eq!(at(Dir8::NorthWest).connection, Some(Connection::Ladder));
    assert!(!at(Dir8::SouthEast).walk_blocked());
    assert_eq!(at(Dir8::SouthEast).obstacle_m, Real::ZERO);

    // Above is open air: nothing to stand on. Below is the rock.
    let above = around.above.unwrap();
    assert_eq!(above.footing, Footing::None);
    let below = around.below.unwrap();
    assert!(below.flags.contains(CellFlags::SOLID_TOP));
    // The cell over a ramp, stairs or a ladder is stood in by standing
    // at the top of it.
    for (x, y) in [(11, 11), (9, 11), (9, 9)] {
        let over = scene.cosmos.neighbours(scene.loc(x, y, 1)).unwrap();
        assert_eq!(over.here.footing, Footing::Connection, "over ({x}, {y})");
        assert_eq!(
            scene.cosmos.footing(scene.loc(x, y, 1)),
            Ok(Footing::Connection)
        );
    }
    // The same asked of one cell at a time.
    assert_eq!(
        scene.cosmos.footing(scene.loc(10, 10, 0)),
        Ok(Footing::Floor)
    );
    assert_eq!(
        scene.cosmos.footing(scene.loc(12, 12, 0)),
        Ok(Footing::Ground)
    );
    assert_eq!(
        scene.cosmos.footing(scene.loc(10, 10, 1)),
        Ok(Footing::None)
    );
    assert!(scene.cosmos.footing(scene.loc(40, 0, 0)).is_err());

    // At the edge of what is loaded, and of the realm, there is no
    // neighbour; asking about a cell that is not loaded is an error.
    let corner = scene.cosmos.neighbours(scene.loc(31, 0, 0)).unwrap();
    assert!(corner.around[Dir8::East.index()].is_none());
    assert!(corner.around[Dir8::North.index()].is_none());
    assert!(corner.around[Dir8::West.index()].is_some());
    assert_eq!(
        scene.cosmos.neighbours(scene.loc(40, 0, 0)).err(),
        Some(WorldError::NotLoaded(scene.loc(40, 0, 0)))
    );
}

#[test]
fn a_walker_does_not_cut_corners_or_walk_on_air() {
    let scene = scene(&[TractPos::new(0, 0)], |kit, paint| {
        paint.block(CellPos::new(11, 10, 0), kit.wall); // east of (10, 10)
        paint.fixture(CellPos::new(5, 5, 0), kit.stairs);
        paint.fixture(CellPos::new(7, 5, 0), kit.ladder);
        paint.floor_rect((4, 6), (8, 8), 1, kit.slab);
    });
    let walker = MoveProfile::walker();
    let steps_from = |x: i32, y: i32, z: i32, rules: &MoveProfile| -> Vec<(StepKind, Real)> {
        let around = scene.cosmos.neighbours(scene.loc(x, y, z)).unwrap();
        around
            .steps(rules)
            .map(|(step, cost)| (step.kind, cost))
            .collect()
    };
    let kinds =
        |steps: &[(StepKind, Real)]| -> Vec<StepKind> { steps.iter().map(|step| step.0).collect() };

    // Beside a wall to the east: not into it, and not diagonally past
    // either end of it.
    let steps = steps_from(10, 10, 0, &walker);
    assert_eq!(
        kinds(&steps),
        [
            StepKind::Level(Dir8::North),
            StepKind::Level(Dir8::South),
            StepKind::Level(Dir8::SouthWest),
            StepKind::Level(Dir8::West),
            StepKind::Level(Dir8::NorthWest),
        ]
    );
    assert_eq!(steps[0].1, Real::ONE);
    assert_eq!(steps[2].1, Real::of(core::f64::consts::SQRT_2));

    // On the stairs: the eight round about and up.
    let steps = steps_from(5, 5, 0, &walker);
    assert_eq!(steps.len(), 9);
    assert_eq!(steps[8], (StepKind::Up, Real::of(3.0)));
    // At the top: down again, and onto the floor to the south, but not
    // out over the air to the north.
    let steps = steps_from(5, 5, 1, &walker);
    assert_eq!(
        kinds(&steps),
        [
            StepKind::Level(Dir8::SouthEast),
            StepKind::Level(Dir8::South),
            StepKind::Level(Dir8::SouthWest),
            StepKind::Down,
        ]
    );
    // A ladder is climbed only by one who climbs ladders.
    assert!(kinds(&steps_from(7, 5, 0, &walker)).contains(&StepKind::Up));
    let no_hands = MoveProfile {
        ladders: false,
        ..walker
    };
    assert!(!kinds(&steps_from(7, 5, 0, &no_hands)).contains(&StepKind::Up));
    assert!(!kinds(&steps_from(7, 5, 1, &no_hands)).contains(&StepKind::Down));
}

/// A house of two rooms with a door between and stairs to the roof.
fn house() -> Scene {
    scene(&four_tracts(), |kit, paint| {
        paint.floor_rect((20, 20), (40, 30), 0, kit.slab);
        paint.outline((20, 20), (40, 30), 0, kit.wall);
        for y in 20..=30 {
            paint.block(CellPos::new(30, y, 0), kit.wall);
        }
        paint.block(CellPos::new(30, 25, 0), kit.door);
        paint.block(CellPos::new(30, 28, 0), BlockState::NONE);
        paint.fixture(CellPos::new(22, 22, 0), kit.rubble);
        paint.fixture(CellPos::new(38, 22, 0), kit.stairs);
        paint.floor_rect((20, 20), (40, 30), 1, kit.slab);
    })
}

#[test]
fn paths_go_round_walls_through_gaps_and_up_stairs() {
    let scene = house();
    let walker = MoveProfile::walker();
    let limits = PathLimits::DEFAULT;
    let from = scene.loc(25, 25, 0);
    let to = scene.loc(35, 25, 0);

    // The door is shut and the walker opens nothing: round by the gap.
    let path = scene.cosmos.find_path(from, to, &walker, limits).unwrap();
    assert_eq!(path.start(), Some(from));
    assert_eq!(path.goal(), Some(to));
    assert_eq!(path.grid(), scene.grid);
    assert!(path.cells().contains(&CellPos::new(30, 28, 0)));
    assert!(!path.cells().contains(&CellPos::new(30, 25, 0)));
    assert_eq!(path.steps(), path.cells().len() - 1);
    assert!(path.is_current(&scene.cosmos));
    // The same question has the same answer, from fresh storage or old.
    let again = scene.cosmos.find_path(from, to, &walker, limits).unwrap();
    assert_eq!(again, path);
    let mut finder = PathFinder::new();
    for _ in 0..3 {
        assert_eq!(
            finder
                .find(&scene.cosmos, from, to, &walker, limits)
                .unwrap(),
            path
        );
    }

    // One who opens doors goes straight through, and pays for it.
    let opener = MoveProfile {
        opens: true,
        ..walker
    };
    let through = scene.cosmos.find_path(from, to, &opener, limits).unwrap();
    assert_eq!(through.steps(), 10);
    assert_eq!(through.cost_m(), Real::of(10.0) + opener.open_cost_m);
    assert!(through.cost_m() < path.cost_m());

    // Up the stairs to the roof.
    let roof = scene.loc(35, 25, 1);
    let up = scene.cosmos.find_path(to, roof, &walker, limits).unwrap();
    assert!(up.cells().contains(&CellPos::new(38, 22, 0)));
    assert!(up.cells().contains(&CellPos::new(38, 22, 1)));
    // And a path to where one already is.
    let stay = scene.cosmos.find_path(from, from, &walker, limits).unwrap();
    assert_eq!(
        (stay.cells(), stay.cost_m()),
        (&[from.cell][..], Real::ZERO)
    );

    // Rough ground costs double, so the path goes round the rubble.
    let near = scene
        .cosmos
        .find_path(scene.loc(21, 22, 0), scene.loc(23, 22, 0), &walker, limits)
        .unwrap();
    assert!(!near.cells().contains(&CellPos::new(22, 22, 0)));
    assert_eq!(near.cost_m(), Real::of(2.0 * core::f64::consts::SQRT_2));
}

#[test]
fn a_search_that_fails_says_why() {
    let scene = house();
    let walker = MoveProfile::walker();
    let limits = PathLimits::DEFAULT;
    let inside = scene.loc(25, 25, 0);
    let outside = scene.loc(10, 10, 0);
    // The house has no way out for one who opens nothing.
    assert_eq!(
        scene.cosmos.find_path(inside, outside, &walker, limits),
        Err(PathFail::NoRoute {
            from: inside,
            to: outside
        })
    );
    // Into a wall.
    let wall = scene.loc(20, 25, 0);
    assert_eq!(
        scene.cosmos.find_path(inside, wall, &walker, limits),
        Err(PathFail::GoalBlocked(wall))
    );
    // To a place that is not loaded, or from one.
    let nowhere = scene.loc(100, 100, 0);
    assert_eq!(
        scene.cosmos.find_path(inside, nowhere, &walker, limits),
        Err(PathFail::World(WorldError::NotLoaded(nowhere)))
    );
    assert!(
        scene
            .cosmos
            .find_path(nowhere, inside, &walker, limits)
            .is_err()
    );
    let elsewhere = Location::new(GridId::new(5), inside.cell);
    assert_eq!(
        scene.cosmos.find_path(inside, elsewhere, &walker, limits),
        Err(PathFail::OtherGrid {
            from: inside,
            to: elsewhere
        })
    );
    // Too few cells allowed, or too little cost.
    let far = scene.loc(35, 25, 0);
    let few = PathLimits {
        max_nodes: 5,
        ..limits
    };
    assert_eq!(
        scene.cosmos.find_path(inside, far, &walker, few),
        Err(PathFail::BudgetSpent {
            to: far,
            max_nodes: 5
        })
    );
    let cheap = PathLimits {
        max_cost_m: Real::of(11.0),
        ..limits
    };
    assert_eq!(
        scene.cosmos.find_path(inside, far, &walker, cheap),
        Err(PathFail::TooFar {
            to: far,
            max_cost_m: Real::of(11.0)
        })
    );
    // Every refusal has words.
    let message = scene
        .cosmos
        .find_path(inside, wall, &walker, limits)
        .unwrap_err()
        .to_string();
    assert!(message.contains("cannot be at"), "{message}");
}

#[test]
fn paths_cross_tract_borders_and_wrapped_edges_and_stay_in_loaded_space() {
    let width = 2 * REGION_CELLS;
    let last = TractPos::new(width / TRACT_XY - 1, 0);
    let scene = scene(&[TractPos::new(0, 0), TractPos::new(1, 0), last], |_, _| {});
    let walker = MoveProfile::walker();
    // Across the border between two tracts.
    let path = scene
        .cosmos
        .find_path(
            scene.loc(28, 5, 0),
            scene.loc(36, 5, 0),
            &walker,
            PathLimits::DEFAULT,
        )
        .unwrap();
    assert_eq!(path.cost_m(), Real::of(8.0));
    // Across the seam of the wrapped axis, the short way.
    let path = scene
        .cosmos
        .find_path(
            scene.loc(3, 5, 0),
            scene.loc(width - 4, 5, 0),
            &walker,
            PathLimits::DEFAULT,
        )
        .unwrap();
    assert_eq!(path.cost_m(), Real::of(7.0));
    assert_eq!(path.cells()[4], CellPos::new(width - 1, 5, 0));
    // Never through a tract that is not loaded.
    for cell in path.cells() {
        assert!(scene.cosmos.is_loaded(Location::new(scene.grid, *cell)));
    }
}

/// The rules of a plain walker, written again from the cells
/// themselves: what `find_path` is checked against.
struct Reference<'a> {
    scene: &'a Scene,
}

impl Reference<'_> {
    fn flags(&self, cell: CellPos) -> Option<CellFlags> {
        self.scene
            .cosmos
            .flags(Location::new(self.scene.grid, cell))
            .ok()
    }

    fn element_flags(&self, cell: CellPos) -> Vec<clash_world::ElementFlags> {
        let defs = &self.scene.kit.defs;
        match self.scene.cosmos.cell(Location::new(self.scene.grid, cell)) {
            Ok(view) => vec![
                defs.floor_info(view.floor).flags,
                defs.block_info(view.block).flags,
                defs.fixture_info(view.fixture).flags,
            ],
            Err(_) => Vec::new(),
        }
    }

    fn connects_up(&self, cell: CellPos) -> bool {
        self.element_flags(cell).iter().any(|flags| {
            flags.has(EngineFlag::Ramp)
                || flags.has(EngineFlag::Stairs)
                || flags.has(EngineFlag::Ladder)
        })
    }

    fn in_way(&self, cell: CellPos) -> bool {
        self.flags(cell)
            .is_none_or(|flags| flags.contains(CellFlags::WALK_BLOCKED))
    }

    fn can_stand(&self, cell: CellPos) -> bool {
        let Some(flags) = self.flags(cell) else {
            return false;
        };
        let held = flags.contains(CellFlags::HAS_FLOOR)
            || self
                .flags(cell.below())
                .is_some_and(|below| below.contains(CellFlags::SOLID_TOP))
            || self.connects_up(cell.below());
        held && !flags.contains(CellFlags::WALK_BLOCKED)
    }

    fn rough(&self, cell: CellPos) -> bool {
        self.element_flags(cell)
            .iter()
            .any(|flags| flags.has(EngineFlag::Rough))
    }

    /// Every step a walker can make from a cell, with its cost.
    fn steps(&self, cell: CellPos) -> Vec<(CellPos, Real)> {
        let mut steps = Vec::new();
        let mut add = |to: CellPos, length: f64| {
            let factor = if self.rough(to) { 2.0 } else { 1.0 };
            steps.push((to, Real::of(length * factor)));
        };
        for dy in -1..=1 {
            for dx in -1..=1 {
                let to = cell.offset(IVec3::new(dx, dy, 0));
                if (dx, dy) == (0, 0) || !self.can_stand(to) {
                    continue;
                }
                let diagonal = dx != 0 && dy != 0;
                if diagonal
                    && (self.in_way(cell.offset(IVec3::new(dx, 0, 0)))
                        || self.in_way(cell.offset(IVec3::new(0, dy, 0))))
                {
                    continue;
                }
                add(
                    to,
                    if diagonal {
                        core::f64::consts::SQRT_2
                    } else {
                        1.0
                    },
                );
            }
        }
        if self.connects_up(cell) && self.can_stand(cell.above()) {
            add(cell.above(), 3.0);
        }
        if self.connects_up(cell.below()) && self.can_stand(cell.below()) {
            add(cell.below(), 3.0);
        }
        steps
    }

    /// The cost of the cheapest way to every cell that can be reached,
    /// by trying everything.
    fn costs_from(&self, from: CellPos) -> BTreeMap<CellPos, Real> {
        let mut best: BTreeMap<CellPos, Real> = BTreeMap::new();
        let mut open = BinaryHeap::new();
        best.insert(from, Real::ZERO);
        open.push(Reverse((Real::ZERO, from)));
        while let Some(Reverse((cost, cell))) = open.pop() {
            if best.get(&cell).is_some_and(|known| *known < cost) {
                continue;
            }
            for (to, step) in self.steps(cell) {
                let reached = cost + step;
                if best.get(&to).is_none_or(|known| reached < *known) {
                    best.insert(to, reached);
                    open.push(Reverse((reached, to)));
                }
            }
        }
        best
    }
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(64))]

    /// On random maps of two storeys with stairs and ladders, a path
    /// never passes through a cell that is in the way, never cuts a
    /// corner, and costs what the cheapest way costs by brute force.
    #[test]
    fn paths_are_legal_and_cheapest(
        map in random_map(),
        trips in prop::collection::vec((0..2 * MAP_CELLS, 0..2 * MAP_CELLS), 6),
    ) {
        let scene = map_scene(&map, true);
        let reference = Reference { scene: &scene };
        let walker = MoveProfile::walker();
        let mut finder = PathFinder::new();
        for (from, to) in trips {
            let (from, to) = (map_pos(from), map_pos(to));
            let costs = reference.costs_from(from);
            let found = finder.find(
                &scene.cosmos,
                Location::new(scene.grid, from),
                Location::new(scene.grid, to),
                &walker,
                PathLimits::DEFAULT,
            );
            match (costs.get(&to), found) {
                (Some(best), Ok(path)) => {
                    prop_assert_eq!(path.cells().first(), Some(&from));
                    prop_assert_eq!(path.cells().last(), Some(&to));
                    let mut total = Real::ZERO;
                    for pair in path.cells().windows(2) {
                        let step = reference.steps(pair[0]).into_iter().find(|(cell, _)| *cell == pair[1]);
                        prop_assert!(step.is_some(), "{:?} to {:?} is not a step a walker can make", pair[0], pair[1]);
                        total += step.unwrap().1;
                    }
                    prop_assert!((total.get() - path.cost_m().get()).abs() < 1e-9);
                    prop_assert!(
                        (path.cost_m().get() - best.get()).abs() < 1e-9,
                        "{:?} to {:?}: found {} and the cheapest is {}", from, to, path.cost_m(), best
                    );
                }
                (None, Err(fail)) => {
                    let expected = matches!(fail, PathFail::NoRoute { .. } | PathFail::GoalBlocked(_));
                    prop_assert!(expected, "{:?}", fail);
                }
                (best, found) => {
                    prop_assert!(false, "{:?} to {:?}: brute force {:?}, search {:?}", from, to, best, found);
                }
            }
        }
    }
}

/// Rules of the caller's own: something that flies, for which footing
/// does not matter and height costs nothing extra.
struct Flier;

impl MoveRules for Flier {
    fn can_occupy(&self, cell: &clash_world::CellGeometry) -> bool {
        !cell.walk_blocked()
    }

    fn step_cost(&self, step: &clash_world::Step) -> Option<Real> {
        // A floor closes the way between storeys.
        let closed = match step.kind {
            StepKind::Up => step.to.flags.contains(CellFlags::HAS_FLOOR),
            StepKind::Down => step.from.flags.contains(CellFlags::HAS_FLOOR),
            StepKind::Level(_) => false,
        };
        // Cheaper than the distance: raised to it by the search.
        (!closed).then_some(Real::of(0.1))
    }
}

#[test]
fn the_caller_may_bring_its_own_rules() {
    // A wall from one side of loaded space to the other.
    let scene = scene(&four_tracts(), |kit, paint| {
        for y in 0..2 * TRACT_XY {
            paint.block(CellPos::new(30, y, 0), kit.wall);
        }
    });
    let from = scene.loc(25, 25, 0);
    let to = scene.loc(35, 25, 0);
    // A walker has no way past. A flier goes over: four along, up, two
    // across the top, down, four along.
    assert_eq!(
        scene
            .cosmos
            .find_path(from, to, &MoveProfile::walker(), PathLimits::DEFAULT),
        Err(PathFail::NoRoute { from, to })
    );
    let path = scene
        .cosmos
        .find_path(from, to, &Flier, PathLimits::DEFAULT)
        .unwrap();
    assert!(path.cells().iter().any(|cell| cell.z == 1));
    // No step was priced below its own length, whatever the rules said.
    assert_eq!(path.cost_m(), Real::of(16.0));
}

// ------------------------------------------------------------- timing

/// The best time of a few rounds of `work`, in microseconds per call.
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

/// Nine tracts of open ground with a floor laid, under open sky.
fn open_field() -> Scene {
    let tracts: Vec<TractPos> = (0..3)
        .flat_map(|y| (0..3).map(move |x| TractPos::new(x, y)))
        .collect();
    scene(&tracts, |kit, paint| {
        paint.floor_rect((0, 0), (95, 95), 0, kit.slab)
    })
}

/// Nine tracts filled with one building: rooms six cells square with a
/// doorway in each wall, desks here and there, and a ceiling.
fn building() -> Scene {
    let tracts: Vec<TractPos> = (0..3)
        .flat_map(|y| (0..3).map(move |x| TractPos::new(x, y)))
        .collect();
    scene(&tracts, |kit, paint| {
        paint.floor_rect((0, 0), (95, 95), 0, kit.slab);
        paint.floor_rect((0, 0), (95, 95), 1, kit.slab);
        for y in 0..96 {
            for x in 0..96 {
                let on_wall = x % 7 == 0 || y % 7 == 0;
                let doorway = (x % 7 == 3) != (y % 7 == 3);
                if on_wall && !doorway {
                    paint.block(CellPos::new(x, y, 0), kit.wall);
                } else if x % 7 == 5 && y % 7 == 2 {
                    paint.fixture(CellPos::new(x, y, 0), kit.desk);
                }
            }
        }
    })
}

/// Not a benchmark with a budget: rough figures, printed. Run with
/// `cargo test --release -p clash-world --test query -- --ignored --nocapture --test-threads=1`.
#[test]
#[ignore = "timing; run by hand in release mode"]
fn timing_of_fields_of_view() {
    let radius = Real::of(30.0);
    for (label, scene) in [
        ("open field, under open sky", open_field()),
        ("building interior", building()),
    ] {
        let origin = scene.loc(46, 46, 0);
        let mut view = FovSet::new();
        let (micros, cells) = time_micros(200, || {
            view.clear();
            scene
                .cosmos
                .field_of_view(origin, 1500, radius, &mut view)
                .unwrap();
            view.len() as u64
        });
        println!(
            "field of view, radius 30, {label}: {micros:.1} µs ({} cells seen)",
            cells / 200
        );
        let (micros, _) = time_micros(2000, || {
            scene
                .cosmos
                .field_of_view(origin, 1500, radius, &mut view)
                .unwrap();
            view.len() as u64
        });
        println!("  the same asked again of an unchanged world: {micros:.3} µs");
    }
    // The open field again with a ceiling, to show one storey alone.
    let roofed = {
        let tracts: Vec<TractPos> = (0..3)
            .flat_map(|y| (0..3).map(move |x| TractPos::new(x, y)))
            .collect();
        scene(&tracts, |kit, paint| {
            paint.floor_rect((0, 0), (95, 95), 0, kit.slab);
            paint.floor_rect((0, 0), (95, 95), 1, kit.slab);
        })
    };
    let mut view = FovSet::new();
    let (micros, cells) = time_micros(200, || {
        view.clear();
        roofed
            .cosmos
            .field_of_view(roofed.loc(46, 46, 0), 1500, radius, &mut view)
            .unwrap();
        view.len() as u64
    });
    println!(
        "field of view, radius 30, open floor under a ceiling: {micros:.1} µs ({} cells seen)",
        cells / 200
    );
}

/// Not a benchmark with a budget: rough figures, printed. Run with
/// `cargo test --release -p clash-world --test query -- --ignored --nocapture --test-threads=1`.
#[test]
#[ignore = "timing; run by hand in release mode"]
fn timing_of_paths_and_lines_of_sight() {
    let walker = MoveProfile::walker();
    for (label, scene, to) in [
        ("open field", open_field(), (70, 10)),
        ("building interior", building(), (67, 11)),
    ] {
        let from = scene.loc(10, 10, 0);
        let to = scene.loc(to.0, to.1, 0);
        let mut finder = PathFinder::new();
        let mut steps = 0;
        let mut settled = 0;
        let (micros, _) = time_micros(200, || {
            let path = finder
                .find(&scene.cosmos, from, to, &walker, PathLimits::DEFAULT)
                .unwrap();
            steps = path.steps();
            settled = path.settled();
            path.steps() as u64
        });
        println!("path of {steps} steps, {label}: {micros:.1} µs ({settled} cells settled)");
        let a = scene.loc(16, 46, 0).point_at(1500);
        let b = scene.loc(46, 46, 0).point_at(1500);
        let (micros, _) = time_micros(20_000, || {
            u64::from(scene.cosmos.line_of_sight(a, b) == Ok(true))
        });
        println!("line of sight over 30 m, {label}: {micros:.3} µs");
        let (micros, _) = time_micros(20_000, || {
            scene
                .cosmos
                .raycast(a, EAST, Real::of(30.0), RayFilter::PROJECTILE)
                .hits()
                .len() as u64
        });
        println!("projectile ray of 30 m, {label}: {micros:.3} µs");
    }
}
