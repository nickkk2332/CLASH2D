//! The `test_city` test generator: a grid of streets with sidewalks, and
//! on every block a building of several storeys (02 §9.3). Every
//! destruction test stands on it.
//!
//! # The layout
//!
//! Streets run north to south and east to west, every `street_spacing`
//! cells. Along each axis one pitch of the grid is: the street, a
//! sidewalk, the lot, a sidewalk.
//!
//! ```text
//!   |street|sw|          lot           |sw|street|sw| ...
//!   0      street_width                   street_spacing
//! ```
//!
//! Each lot holds one building that fills it:
//!
//! - outer walls, with a window in every second cell and, on the ground
//!   storey, one or two doors onto the sidewalk;
//! - a floor slab on every storey and a roof over the top one;
//! - two partitions that cross, making four rooms, with a door in each
//!   of the four stretches of partition;
//! - pillars wherever both lot coordinates are multiples of
//!   `column_spacing`;
//! - stairs from every storey but the top one, alternating between two
//!   cells so that each flight starts beside where the last one ended,
//!   with an opening in the slab above each flight;
//! - a few desks, away from the doors and the stairs.
//!
//! # Why it needs no neighbour
//!
//! Everything about a cell follows from its position: which block of
//! the street grid it is in, and where in that block. What a building
//! is like (how tall, where its doors and partitions are) is drawn from
//! the positional hash of its block, so the two halves of a building
//! that straddles a tract boundary agree without either tract seeing
//! the other (invariant W2).
//!
//! # No content is named here
//!
//! Every floor, block and fixture the city is built from is named by a
//! parameter of the realm definition (01 R2).

use std::collections::BTreeMap;
use std::sync::Arc;

use clash_core::{HashDomain, Rng};
use clash_world::dims::TRACT_XY;
use clash_world::{
    BlockState, CellPos, ColumnBuilder, FixtureState, FloorState, GenCtx, GenError,
    GeneratorRequest, RealizedTract, RealmInfo, RegionPos, RegionRecord, RegionSurvey,
    RegionSurveyResult, RegionSurveyor, TractPos, TractRealizer, TractRecord, TractRecords3x3,
    WorldGenerator,
};

/// The name realm definitions give this generator by.
pub const CITY_GENERATOR: &str = "test_city";

/// What a building is like: drawn once for each block of the street
/// grid.
const BUILDING: HashDomain = HashDomain::new("gen.test_city.building");

/// Whether a cell of a room holds a desk.
const DESK: HashDomain = HashDomain::new("gen.test_city.desk");

/// The narrowest lot a building fits on, in cells: its two outer walls,
/// a partition, and a room two cells deep on each side of it.
const MIN_LOT: i64 = 7;

/// The nearest a partition runs to an outer wall, in lot coordinates:
/// it leaves a room two cells deep.
const FIRST_SPLIT: i32 = 3;

/// The most storeys a building may be given.
const MAX_STOREYS: i64 = 40;

/// The widest pitch of the street grid, in cells.
const MAX_STREET_SPACING: i64 = 200;

/// The widest a street or a sidewalk may be, in cells.
const MAX_WIDTH: i64 = 16;

/// The widest spacing of pillars, in cells.
const MAX_COLUMN_SPACING: i64 = 16;

/// Every this many cells along an outer wall is a window.
const WINDOW_PITCH: i32 = 2;

/// One free cell of a room in this many holds a desk.
const DESK_ODDS: u64 = 16;

const SURFACE_Z: &str = "surface_z";
const STREET_SPACING: &str = "street_spacing";
const STREET_WIDTH: &str = "street_width";
const SIDEWALK_WIDTH: &str = "sidewalk_width";
const STOREYS: &str = "storeys";
const COLUMN_SPACING: &str = "column_spacing";
const GROUND: &str = "ground";
const STREET: &str = "street";
const SIDEWALK: &str = "sidewalk";
const SLAB: &str = "slab";
const ROOF: &str = "roof";
const WALL: &str = "wall";
const WINDOW: &str = "window";
const DOOR: &str = "door";
const PARTITION: &str = "partition";
const PILLAR: &str = "pillar";
const STAIRS: &str = "stairs";
const DESK_PARAM: &str = "desk";

/// Every parameter the generator takes.
const PARAMS: [&str; 18] = [
    SURFACE_Z,
    STREET_SPACING,
    STREET_WIDTH,
    SIDEWALK_WIDTH,
    STOREYS,
    COLUMN_SPACING,
    GROUND,
    STREET,
    SIDEWALK,
    SLAB,
    ROOF,
    WALL,
    WINDOW,
    DOOR,
    PARTITION,
    PILLAR,
    STAIRS,
    DESK_PARAM,
];

/// What a realm definition tells the `test_city` generator.
///
/// | Parameter | Meaning | Default |
/// |-----------|---------|---------|
/// | `surface_z` | The storey of the streets and the ground storeys | 0 |
/// | `street_spacing` | Cells from one street to the next | 16 |
/// | `street_width` | Cells across a street | 3 |
/// | `sidewalk_width` | Cells across a sidewalk | 1 |
/// | `storeys` | The fewest and the most storeys of a building | `[1, 3]` |
/// | `column_spacing` | Cells between pillars | 4 |
/// | `ground` | The block state under the surface | The realm's `fill.below` |
/// | `street`, `sidewalk`, `slab`, `roof` | Floor states | required |
/// | `wall`, `window`, `door`, `partition`, `pillar`, `stairs` | Block states. `stairs` must be flagged as a vertical connection to be climbed | required |
/// | `desk` | A fixture state | required |
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct CityParams {
    /// The storey of the streets and the ground storeys.
    pub surface_z: i32,
    /// Cells from one street to the next.
    pub street_spacing: i32,
    /// Cells across a street.
    pub street_width: i32,
    /// Cells across a sidewalk.
    pub sidewalk_width: i32,
    /// The fewest and the most storeys of a building.
    pub storeys: (i32, i32),
    /// Cells between pillars.
    pub column_spacing: i32,
    /// What lies under the surface.
    pub ground: BlockState,
    /// The floor of a street.
    pub street: FloorState,
    /// The floor of a sidewalk.
    pub sidewalk: FloorState,
    /// The floor of every storey of a building.
    pub slab: FloorState,
    /// The floor laid over the top storey.
    pub roof: FloorState,
    /// An outer wall.
    pub wall: BlockState,
    /// A window in an outer wall.
    pub window: BlockState,
    /// A door, in an outer wall or a partition.
    pub door: BlockState,
    /// A wall between rooms.
    pub partition: BlockState,
    /// A pillar.
    pub pillar: BlockState,
    /// A flight of stairs to the storey above.
    pub stairs: BlockState,
    /// A desk.
    pub desk: FixtureState,
}

impl CityParams {
    /// Reads the parameters of a realm definition, or says what is
    /// wrong with them.
    pub fn read(request: &GeneratorRequest<'_>) -> Result<CityParams, GenError> {
        request.only(&PARAMS)?;
        let topology = request.kind.topology;
        let (z_min, z_max) = (i64::from(topology.z_min), i64::from(topology.z_max));
        let surface_z = request.int(SURFACE_Z, 0, z_min..=z_max)?;
        let street_spacing = request.int(STREET_SPACING, 16, MIN_LOT..=MAX_STREET_SPACING)?;
        let street_width = request.int(STREET_WIDTH, 3, 1..=MAX_WIDTH)?;
        let sidewalk_width = request.int(SIDEWALK_WIDTH, 1, 0..=MAX_WIDTH)?;
        let storeys = request.int_pair(STOREYS, (1, 3), 1..=MAX_STOREYS)?;
        let column_spacing = request.int(COLUMN_SPACING, 4, 2..=MAX_COLUMN_SPACING)?;
        let lot = street_spacing - street_width - 2 * sidewalk_width;
        if lot < MIN_LOT {
            return Err(request.error(
                STREET_SPACING,
                format!(
                    "leaves lots {lot} cells wide between the streets and their sidewalks, \
                     and a building needs {MIN_LOT}"
                ),
            ));
        }
        if surface_z + storeys.1 > z_max {
            return Err(request.error(
                STOREYS,
                format!(
                    "the roof of the tallest building would be at storey {}, above the \
                     realm's highest, {z_max}",
                    surface_z + storeys.1
                ),
            ));
        }
        let ground = if request.kind.generator_params.contains_key(GROUND) {
            request.block(GROUND)?
        } else {
            request.kind.below_fill
        };
        // Every number was checked against a range that fits an `i32`.
        let cells = |value: i64| i32::try_from(value).unwrap_or(i32::MAX);
        Ok(CityParams {
            surface_z: cells(surface_z),
            street_spacing: cells(street_spacing),
            street_width: cells(street_width),
            sidewalk_width: cells(sidewalk_width),
            storeys: (cells(storeys.0), cells(storeys.1)),
            column_spacing: cells(column_spacing),
            ground,
            street: request.floor(STREET)?,
            sidewalk: request.floor(SIDEWALK)?,
            slab: request.floor(SLAB)?,
            roof: request.floor(ROOF)?,
            wall: request.block(WALL)?,
            window: request.block(WINDOW)?,
            door: request.block(DOOR)?,
            partition: request.block(PARTITION)?,
            pillar: request.block(PILLAR)?,
            stairs: request.block(STAIRS)?,
            desk: request.fixture(DESK_PARAM)?,
        })
    }

    /// How many cells across a lot is.
    pub fn lot(&self) -> i32 {
        self.street_spacing - self.street_width - 2 * self.sidewalk_width
    }

    /// What a cell of the street grid is, from its position within its
    /// block along each axis.
    fn zone(&self, lx: i32, ly: i32) -> Zone {
        if lx < self.street_width || ly < self.street_width {
            return Zone::Street;
        }
        let first = self.street_width + self.sidewalk_width;
        let end = self.street_spacing - self.sidewalk_width;
        if lx < first || ly < first || lx >= end || ly >= end {
            return Zone::Sidewalk;
        }
        Zone::Lot(lx - first, ly - first)
    }
}

/// What a cell of the street grid is.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Zone {
    Street,
    Sidewalk,
    /// Part of a lot, at these coordinates from its north-west corner.
    Lot(i32, i32),
}

/// A side of a building.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Side {
    North,
    East,
    South,
    West,
}

impl Side {
    const ALL: [Side; 4] = [Side::North, Side::East, Side::South, Side::West];
}

/// What one cell of one storey of a building is.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Part {
    /// Not part of the building.
    Outside,
    Wall,
    Window,
    /// A door in an outer wall or in a partition.
    Door,
    Partition,
    Pillar,
    Stairs,
    /// The opening in the slab over the flight of stairs below.
    Well,
    /// Free floor.
    Room,
}

/// The cell of a storey, in lot coordinates, that holds its flight of
/// stairs. Flights alternate between two cells in the north-west room,
/// so each starts beside the opening the last one came up through.
fn stair_cell(storey: i32) -> (i32, i32) {
    (1 + storey.rem_euclid(2), 1)
}

/// What a building is like. Drawn from the positional hash of its
/// block.
#[derive(Clone, Copy, Debug)]
struct Building {
    storeys: i32,
    /// The doors of the ground storey: a side and a position along it.
    doors: [Option<(Side, i32)>; 2],
    /// The lot coordinate of the partition that runs north to south.
    split_u: i32,
    /// The lot coordinate of the partition that runs east to west.
    split_v: i32,
    /// Where the door is in each stretch of partition: along the
    /// north-south one, north and then south of the crossing; along the
    /// east-west one, west and then east of it.
    gaps: [i32; 4],
}

/// A whole number from `low` to `high`, both included.
fn draw(rng: &mut Rng, low: i32, high: i32) -> i32 {
    i32::try_from(rng.range(i64::from(low), i64::from(high))).unwrap_or(low)
}

impl Building {
    fn plan(cx: &GenCtx<'_>, realm: &RealmInfo, params: &CityParams, bx: i32, by: i32) -> Building {
        let lot = params.lot();
        let inner_last = lot - 2;
        let mut rng = cx.rng(realm, BUILDING, CellPos::new(bx, by, 0), 0);
        let storeys = draw(&mut rng, params.storeys.0, params.storeys.1);
        let last_split = lot - 1 - FIRST_SPLIT;
        let split_u = draw(&mut rng, FIRST_SPLIT, last_split);
        let split_v = draw(&mut rng, FIRST_SPLIT, last_split);
        let gaps = [
            draw(&mut rng, 1, split_v - 1),
            draw(&mut rng, split_v + 1, inner_last),
            draw(&mut rng, 1, split_u - 1),
            draw(&mut rng, split_u + 1, inner_last),
        ];
        // A door must open onto floor: not onto the end of a partition,
        // and not onto the stairs.
        let stairs = [stair_cell(0), stair_cell(1)];
        let door = |rng: &mut Rng, side: Side| -> Option<(Side, i32)> {
            let places: Vec<i32> = (1..=inner_last)
                .filter(|&t| match side {
                    Side::North => t != split_u && stairs.iter().all(|cell| *cell != (t, 1)),
                    Side::South => t != split_u,
                    Side::West => t != split_v && stairs.iter().all(|cell| *cell != (1, t)),
                    Side::East => t != split_v,
                })
                .collect();
            rng.pick(&places).map(|place| (side, *place))
        };
        let first_side = usize::try_from(draw(&mut rng, 0, 3)).unwrap_or(0);
        let first = door(&mut rng, Side::ALL[first_side % Side::ALL.len()]);
        // Half the buildings have a second door, on another side.
        let second = if draw(&mut rng, 0, 1) == 1 {
            let step = usize::try_from(draw(&mut rng, 1, 3)).unwrap_or(1);
            door(&mut rng, Side::ALL[(first_side + step) % Side::ALL.len()])
        } else {
            None
        };
        Building {
            storeys,
            doors: [first, second],
            split_u,
            split_v,
            gaps,
        }
    }

    /// What the cell at lot coordinates `(u, v)` of a storey is.
    fn part(&self, params: &CityParams, storey: i32, u: i32, v: i32) -> Part {
        let lot = params.lot();
        let edge = lot - 1;
        if u < 0 || v < 0 || u > edge || v > edge || storey < 0 || storey >= self.storeys {
            return Part::Outside;
        }
        let on_west_east = u == 0 || u == edge;
        let on_north_south = v == 0 || v == edge;
        if on_west_east && on_north_south {
            return Part::Wall;
        }
        if on_west_east || on_north_south {
            let (side, along) = if v == 0 {
                (Side::North, u)
            } else if v == edge {
                (Side::South, u)
            } else if u == 0 {
                (Side::West, v)
            } else {
                (Side::East, v)
            };
            if storey == 0
                && self
                    .doors
                    .iter()
                    .flatten()
                    .any(|door| *door == (side, along))
            {
                return Part::Door;
            }
            return if along % WINDOW_PITCH == 0 {
                Part::Window
            } else {
                Part::Wall
            };
        }
        if storey + 1 < self.storeys && (u, v) == stair_cell(storey) {
            return Part::Stairs;
        }
        if storey >= 1 && (u, v) == stair_cell(storey - 1) {
            return Part::Well;
        }
        let pillar = u % params.column_spacing == 0 && v % params.column_spacing == 0;
        let (on_u, on_v) = (u == self.split_u, v == self.split_v);
        if on_u || on_v {
            let gap = match (on_u, on_v) {
                (true, true) => false,
                (true, false) => v == self.gaps[usize::from(v > self.split_v)],
                _ => u == self.gaps[2 + usize::from(u > self.split_u)],
            };
            return if gap {
                Part::Door
            } else if pillar {
                Part::Pillar
            } else {
                Part::Partition
            };
        }
        if pillar { Part::Pillar } else { Part::Room }
    }

    /// Whether a free cell of a room stands clear of the doors and the
    /// stairs, so that a desk there is in nobody's way.
    fn clear(&self, params: &CityParams, storey: i32, u: i32, v: i32) -> bool {
        [(0, -1), (1, 0), (0, 1), (-1, 0)]
            .into_iter()
            .all(|(du, dv)| {
                !matches!(
                    self.part(params, storey, u + du, v + dv),
                    Part::Door | Part::Stairs | Part::Well
                )
            })
    }
}

/// The `test_city` generator. See the module documentation.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct CityGenerator {
    params: CityParams,
}

impl CityGenerator {
    /// A city with these parameters.
    pub fn new(params: CityParams) -> CityGenerator {
        CityGenerator { params }
    }

    /// Makes the generator a realm definition asks for.
    pub fn factory(request: &GeneratorRequest<'_>) -> Result<Arc<dyn WorldGenerator>, GenError> {
        Ok(Arc::new(CityGenerator::new(CityParams::read(request)?)))
    }

    /// The city's parameters.
    pub fn params(&self) -> &CityParams {
        &self.params
    }

    /// Builds one cell column of a building, storey by storey, and its
    /// roof.
    fn raise(
        &self,
        cx: &GenCtx<'_>,
        realm: &RealmInfo,
        column: &mut ColumnBuilder<'_>,
        building: &Building,
        at: (i32, i32),
        lot: (i32, i32),
    ) {
        let params = &self.params;
        let (u, v) = lot;
        for storey in 0..building.storeys {
            let cell = CellPos::new(at.0, at.1, params.surface_z + storey);
            let part = building.part(params, storey, u, v);
            if part != Part::Well {
                column.set_floor(cell, params.slab);
            }
            let block = match part {
                Part::Wall => params.wall,
                Part::Window => params.window,
                Part::Door => params.door,
                Part::Partition => params.partition,
                Part::Pillar => params.pillar,
                Part::Stairs => params.stairs,
                Part::Outside | Part::Well | Part::Room => BlockState::NONE,
            };
            if !block.is_none() {
                column.set_block(cell, block);
            }
            if part == Part::Room
                && building.clear(params, storey, u, v)
                && cx.hash(realm, DESK, cell, 0).is_multiple_of(DESK_ODDS)
            {
                column.set_fixture(cell, params.desk);
            }
        }
        let top = CellPos::new(at.0, at.1, params.surface_z + building.storeys);
        column.set_floor(top, params.roof);
    }
}

impl RegionSurveyor for CityGenerator {
    fn survey(&self, _: &GenCtx<'_>, _: &RealmInfo, _: RegionPos) -> RegionSurveyResult {
        // Every tract is the same kind of place: the street grid is a
        // matter of position, and needs nothing written down.
        RegionSurveyResult::new(RegionRecord::new(RegionSurvey::default()))
    }
}

impl TractRealizer for CityGenerator {
    fn realize(
        &self,
        cx: &GenCtx<'_>,
        realm: &RealmInfo,
        pos: TractPos,
        _: &TractRecord,
        _: &TractRecords3x3,
    ) -> RealizedTract {
        let params = &self.params;
        let mut column =
            ColumnBuilder::new(cx.defs(), params.ground, realm.above_fill, params.surface_z);
        let origin = pos.origin(params.surface_z);
        // The buildings this tract holds a part of, by block.
        let mut buildings: BTreeMap<(i32, i32), Building> = BTreeMap::new();
        for dy in 0..TRACT_XY {
            for dx in 0..TRACT_XY {
                let (x, y) = (origin.x.wrapping_add(dx), origin.y.wrapping_add(dy));
                let block = (
                    x.div_euclid(params.street_spacing),
                    y.div_euclid(params.street_spacing),
                );
                let within = (
                    x.rem_euclid(params.street_spacing),
                    y.rem_euclid(params.street_spacing),
                );
                let ground = CellPos::new(x, y, params.surface_z);
                match params.zone(within.0, within.1) {
                    Zone::Street => {
                        column.set_floor(ground, params.street);
                    }
                    Zone::Sidewalk => {
                        column.set_floor(ground, params.sidewalk);
                    }
                    Zone::Lot(u, v) => {
                        let building = *buildings
                            .entry(block)
                            .or_insert_with(|| Building::plan(cx, realm, params, block.0, block.1));
                        self.raise(cx, realm, &mut column, &building, (x, y), (u, v));
                    }
                }
            }
        }
        RealizedTract::new(column.build())
    }
}
