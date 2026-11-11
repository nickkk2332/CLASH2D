//! Resolving a map for one placement: [`MapDefs::resolve`] and
//! [`ResolvedMap`] (10 §3.3 to §3.5).
//!
//! # Randomness
//!
//! Every choice a map makes is drawn from a generator seeded by
//! [`pos_hash`] of the placement's seed, a named domain, the cell the
//! choice is for (in the map's own coordinates, before any turn) and a
//! number that tells the choices at one cell apart. Nothing is drawn
//! from a stream, and nothing a choice draws from depends on another
//! choice having been made. So a resolved map depends on the seed and
//! on nothing else: not on the order its cells are visited in, and not
//! on when it is resolved (01 §4, invariant X6 of 10 §14).
//!
//! The random functions of the expression language (`rand`, `rand_int`,
//! `chance`, `pick`) draw from the generator the host hands them. The
//! host here is private to this module and has no generator but the
//! positional one, so an expression in a map cannot reach a stream of
//! the simulation: there is none within its reach.

use std::collections::{BTreeMap, BTreeSet};

use clash_core::{Dir8, HashDomain, IVec2, IVec3, Real, Rng, Turn, VarMap, pos_hash};
use clash_data::expr::{Any, Int, Number};
use clash_data::{Cond, Expr, Runner};
use clash_expr::{FunctionId, Handle, Host, PropertyId, RuntimeWarning, Value};
use clash_world::{
    AreaKindId, BlockState, ElementState, FixtureState, FloorState, Layer, StateRefError,
};
use thiserror::Error;

use super::compile::{
    ClearedLayer, CompiledCount, CompiledEntry, CompiledLevel, CompiledMap, CompiledPlace,
    CompiledRule, CompiledSpawn, ItemSource, JointDir, MapDefs, MapDevice, NO_ENTRY, PatternMatch,
    PlaceAt, PortalTo, SpawnAt, SpawnWho, StateSlot, TemplatePart, allowed_turns, fits_at,
    nested_origin, turned_cell, turned_size,
};
use super::def::MapId;

/// The domain of the draws that give parameters their values.
const PARAM_DOMAIN: HashDomain = HashDomain::new("gen.map.param");
/// The domain of the generator the random functions of an expression
/// draw from.
const EXPR_DOMAIN: HashDomain = HashDomain::new("gen.map.expr");
/// The domain of the choice between weighted alternatives.
const ALTERNATIVE_DOMAIN: HashDomain = HashDomain::new("gen.map.alternative");
/// The domain of chances and of counts given as a range.
const ROLL_DOMAIN: HashDomain = HashDomain::new("gen.map.roll");
/// The domain of the seeds nested maps are given.
const NESTED_DOMAIN: HashDomain = HashDomain::new("gen.map.nested");
/// The domain of the choice of which nested map, and turned how.
const PICK_DOMAIN: HashDomain = HashDomain::new("gen.map.pick");
/// The domain of the order cells of an area are tried in when maps are
/// scattered through it.
const SCATTER_DOMAIN: HashDomain = HashDomain::new("gen.map.scatter");
/// The domain of the order a test visits cells in. It decides nothing
/// about the result.
const ORDER_DOMAIN: HashDomain = HashDomain::new("gen.map.order");

/// How far apart the salts of two kinds of choice are, so that the
/// index of a choice within its kind never reaches the next kind.
const SALT_STRIDE: u64 = 1 << 24;

/// What a choice is for. With an index, it tells apart the choices
/// made at one cell.
#[derive(Clone, Copy)]
enum Salt {
    Condition = 1,
    Weight,
    ItemsChance,
    ItemsCount,
    SpawnChance,
    SpawnCount,
    PlaceChance,
    PlaceCount,
    PlaceParam,
    MapSpawnChance,
    MapSpawnCount,
}

impl Salt {
    const fn with(self, index: usize) -> u64 {
        self as u64 * SALT_STRIDE + index as u64
    }
}

/// The cell of a choice made once for the whole placement.
const PLACEMENT: IVec3 = IVec3::new(0, 0, 0);

/// Why a map could not be resolved. Each is a refusal with its reason;
/// none is a panic.
#[derive(Clone, PartialEq, Eq, Debug, Error)]
pub enum ResolveError {
    /// The id names no map of these definitions.
    #[error("there is no map with the id {0:?}")]
    UnknownMap(MapId),
    /// The placement gives a parameter the map does not declare.
    #[error("the map `{map}` has no parameter `{name}`")]
    UnknownParam {
        /// The map's qualified id.
        map: String,
        /// The parameter given.
        name: String,
    },
    /// The map needs a parameter that nothing gave.
    #[error("the map `{map}` needs the parameter `{name}`, and the placement gives none")]
    MissingParam {
        /// The map's qualified id.
        map: String,
        /// The parameter.
        name: String,
    },
    /// A parameter was given, or came to, a value of the wrong type.
    #[error("the parameter `{name}` of the map `{map}` is a {expected}, and was given {given}")]
    ParamType {
        /// The map's qualified id.
        map: String,
        /// The parameter.
        name: String,
        /// The type it is declared with.
        expected: &'static str,
        /// What it was given, as text.
        given: String,
    },
    /// A state written with parameters names nothing once they are
    /// filled in.
    #[error("in the map `{map}`, the state `{text}` cannot be used: {error}")]
    State {
        /// The map's qualified id.
        map: String,
        /// The state with its parameters filled in.
        text: String,
        /// What is wrong with it.
        error: StateRefError,
    },
    /// The map says `"turn": false` and was asked for turned.
    #[error("the map `{map}` may not be turned")]
    TurnForbidden {
        /// The map's qualified id.
        map: String,
    },
    /// Maps nest deeper than the limit. Building the maps refuses this,
    /// so it is not reached through [`MapDefs::from_defs`].
    #[error("maps nest deeper than {limit} below `{map}`")]
    TooDeep {
        /// The map's qualified id.
        map: String,
        /// The limit, `gen.max_nesting`.
        limit: u32,
    },
}

/// The order the cells of a level are visited in while a map is
/// resolved. The result is the same for every order (invariant X6);
/// this exists so that a test can show it.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum EvalOrder {
    /// Storey by storey upward, row by row, west to east.
    #[default]
    Forward,
    /// The other way round.
    Reverse,
    /// An order shuffled by this number.
    Shuffled(u64),
}

/// What a resolved map puts in one cell.
///
/// Each layer is `None`, "leave what is there", or a state to put
/// there, which may be the state of nothing: `Some(BlockState::NONE)`
/// empties the layer.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct ResolvedCell {
    /// The floor.
    pub floor: Option<FloorState>,
    /// The block.
    pub block: Option<BlockState>,
    /// The fixture.
    pub fixture: Option<FixtureState>,
}

impl ResolvedCell {
    /// A cell the map does not touch.
    pub const LEAVE: ResolvedCell = ResolvedCell {
        floor: None,
        block: None,
        fixture: None,
    };

    /// A cell a character drew, before its spec has said anything:
    /// every layer emptied.
    const DRAWN: ResolvedCell = ResolvedCell {
        floor: Some(FloorState::NONE),
        block: Some(BlockState::NONE),
        fixture: Some(FixtureState::NONE),
    };

    /// Whether the map leaves every layer of the cell as it is.
    pub const fn is_leave(&self) -> bool {
        self.floor.is_none() && self.block.is_none() && self.fixture.is_none()
    }

    fn set(&mut self, layer: Layer, state: Option<ElementState>) {
        match layer {
            Layer::Floor => self.floor = state.map(FloorState::from_raw),
            Layer::Block => self.block = state.map(BlockState::from_raw),
            Layer::Fixture => self.fixture = state.map(FixtureState::from_raw),
        }
    }

    fn get(&self, layer: Layer) -> Option<ElementState> {
        match layer {
            Layer::Floor => self.floor.map(FloorState::raw),
            Layer::Block => self.block.map(BlockState::raw),
            Layer::Fixture => self.fixture.map(FixtureState::raw),
        }
    }
}

/// One storey of a resolved map.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct ResolvedLevel {
    /// The storey, relative to the map's own ground level.
    pub z: i32,
    /// The cells, row by row from the north, each row west to east:
    /// the cell at `(x, y)` is `cells[y * width + x]`.
    pub cells: Vec<ResolvedCell>,
}

/// The cells an area of a resolved map covers, in map coordinates.
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub enum MapShape {
    /// A box, both corners included.
    Box {
        /// The lowest corner.
        min: IVec3,
        /// The highest corner.
        max: IVec3,
    },
    /// Listed cells, sorted by storey, then row, then column.
    Cells(Vec<IVec3>),
}

/// An area a resolved map asks for (02 §8.3).
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub struct ResolvedArea {
    /// The name the map knows it by: its `name`, or its kind as written.
    pub name: String,
    /// The `area_kind` definition.
    pub kind: Option<AreaKindId>,
    /// Tags it carries besides its kind's, sorted.
    pub tags: Vec<String>,
    /// The cells it covers.
    pub shape: MapShape,
}

/// Where the creatures of a spawn go.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum SpawnPlace {
    /// This cell.
    Cell(IVec3),
    /// Anywhere in the areas of this name.
    Area(String),
    /// Anywhere in the map.
    Anywhere,
}

/// Creatures a resolved map asks for. The world cannot take them yet:
/// they are carried for the package that makes entities.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct ResolvedSpawn {
    /// Who.
    pub who: SpawnWho,
    /// How many, already drawn.
    pub count: u32,
    /// Where.
    pub place: SpawnPlace,
}

/// Items a resolved map lays in a cell.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct ResolvedItems {
    /// The cell.
    pub at: IVec3,
    /// An item or a content table.
    pub source: ItemSource,
    /// How many, or how many draws from the table.
    pub count: u32,
}

/// A cell of a berth, or of the line into it (10 §3.2).
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct BerthCell {
    /// The cell.
    pub at: IVec3,
    /// The berth's name within the map.
    pub name: String,
    /// The kind of berth, as written. Empty for a cell of the approach.
    pub kind: String,
    /// Whether the cell is on the approach and not in the berth.
    pub approach: bool,
}

/// A joint of a resolved map (10 §7.2).
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct ResolvedJoint {
    /// Its label within the map.
    pub id: Option<String>,
    /// The cell.
    pub at: IVec3,
    /// Which way it opens.
    pub dir: JointDir,
    /// What kind of opening it is.
    pub kind: String,
}

/// A link between two named devices of a resolved map (10 §3.6).
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct MapLink {
    /// The name of the device it runs from.
    pub from: String,
    /// The name of the device it runs to.
    pub to: String,
    /// The resource it carries.
    pub resource: Option<String>,
}

/// A portal a resolved map asks for (02 §19).
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct MapPortal {
    /// The mark one end stands at.
    pub from: String,
    /// The other end.
    pub to: PortalTo,
}

/// A vehicle standing in a resolved map (11 §12.1).
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct MapVehicle {
    /// The vehicle definition, as written.
    pub vehicle: String,
    /// The cell.
    pub at: IVec3,
    /// Which way it points.
    pub facing: Option<Dir8>,
}

/// A map resolved for one placement: every choice made, every state
/// typed, in map coordinates after turning.
///
/// Coordinates are `[x, y, z]` with `x` east and `y` south from the
/// north-west corner of the turned map, and `z` the storey relative to
/// the map's own ground level. Every list is sorted by cell: storey,
/// then row, then column.
///
/// # What placing a map reads
///
/// - [`size`](ResolvedMap::size) and the storeys of
///   [`levels`](ResolvedMap::levels): the box the placement fills.
/// - [`levels`](ResolvedMap::levels): for each cell and each of floor,
///   block and fixture, `None` to leave the layer or a state to write
///   through `WorldEdit`.
/// - [`anchor_cell`](ResolvedMap::anchor_cell): the cell that goes
///   where the placement says. The north-west corner at level 0 unless
///   the map names an anchor.
/// - [`areas`](ResolvedMap::areas), [`marks`](ResolvedMap::marks):
///   areas to create and named cells to record with the site.
/// - Everything else is carried for later packages: spawns and items
///   (entities), conduits, fields, devices and links (devices),
///   berths, watched cells, variables, decals, patterns, joints,
///   portals and vehicles.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct ResolvedMap {
    /// The map.
    pub map: MapId,
    /// Width and depth in cells, after turning.
    pub size: IVec2,
    /// The storeys, lowest first, with none left out.
    pub levels: Vec<ResolvedLevel>,
    /// The cell a placement names when it says where the map goes, if
    /// the map names one. `None` means the north-west corner at level
    /// 0 of the map as it is here, after turning:
    /// [`anchor_cell`](ResolvedMap::anchor_cell) gives the cell either
    /// way.
    pub anchor: Option<IVec3>,
    /// Which side faces a street, after turning.
    pub front: Option<Dir8>,
    /// Areas, in a fixed order: by name, kind, tags and shape.
    pub areas: Vec<ResolvedArea>,
    /// Named cells.
    pub marks: BTreeMap<String, Vec<IVec3>>,
    /// Creatures and population members.
    pub spawns: Vec<ResolvedSpawn>,
    /// Items.
    pub items: Vec<ResolvedItems>,
    /// Conduits, by cell and conduit id as written.
    pub conduits: Vec<(IVec3, String)>,
    /// Field amounts, by cell and field id as written.
    pub fields: Vec<(IVec3, String, Real)>,
    /// Device names and settings, by cell.
    pub devices: Vec<(IVec3, MapDevice)>,
    /// Links between named devices.
    pub links: Vec<MapLink>,
    /// Cells of berths and their approaches.
    pub berths: Vec<BerthCell>,
    /// Cells whose contents the site's people keep an eye on.
    pub watched: Vec<IVec3>,
    /// Cell variables.
    pub vars: Vec<(IVec3, String, clash_core::Value)>,
    /// Decals, by cell and decal id as written.
    pub decals: Vec<(IVec3, String)>,
    /// What cells of a pattern ask for in place of a state.
    pub patterns: Vec<(IVec3, PatternMatch)>,
    /// Layers besides the three elements that cells empty.
    pub cleared: Vec<(IVec3, ClearedLayer)>,
    /// Where the map can connect to others.
    pub joints: Vec<ResolvedJoint>,
    /// Portals between marks.
    pub portals: Vec<MapPortal>,
    /// Vehicles.
    pub vehicles: Vec<MapVehicle>,
}

/// Cells in the order positions are listed in.
type CellSet = BTreeSet<(i32, i32, i32)>;

/// The order positions are listed in: storey, then row, then column.
fn cell_key(cell: IVec3) -> (i32, i32, i32) {
    (cell.z, cell.y, cell.x)
}

impl ResolvedMap {
    fn empty(map: &CompiledMap) -> ResolvedMap {
        let cells = (map.size.x * map.size.y) as usize;
        ResolvedMap {
            map: map.id,
            size: map.size,
            levels: (0..map.levels.len())
                .map(|index| ResolvedLevel {
                    z: map.lowest + index as i32,
                    cells: vec![ResolvedCell::LEAVE; cells],
                })
                .collect(),
            anchor: map.anchor,
            front: map.front,
            areas: Vec::new(),
            marks: BTreeMap::new(),
            spawns: Vec::new(),
            items: Vec::new(),
            conduits: Vec::new(),
            fields: Vec::new(),
            devices: Vec::new(),
            links: Vec::new(),
            berths: Vec::new(),
            watched: Vec::new(),
            vars: Vec::new(),
            decals: Vec::new(),
            patterns: Vec::new(),
            cleared: Vec::new(),
            joints: Vec::new(),
            portals: Vec::new(),
            vehicles: Vec::new(),
        }
    }

    /// The cell that goes where a placement says the map goes: the
    /// map's anchor, or the north-west corner at level 0.
    pub fn anchor_cell(&self) -> IVec3 {
        self.anchor.unwrap_or(IVec3::new(0, 0, 0))
    }

    /// The lowest and the highest storey.
    pub fn level_range(&self) -> (i32, i32) {
        let lowest = self.levels.first().map_or(0, |level| level.z);
        (lowest, lowest + self.levels.len() as i32 - 1)
    }

    /// The storey at `z`, if the map has it.
    pub fn level(&self, z: i32) -> Option<&ResolvedLevel> {
        let (lowest, _) = self.level_range();
        usize::try_from(z - lowest)
            .ok()
            .and_then(|index| self.levels.get(index))
    }

    /// What the map puts in a cell. A cell outside the map is left as
    /// it is.
    pub fn cell(&self, x: i32, y: i32, z: i32) -> ResolvedCell {
        if x < 0 || y < 0 || x >= self.size.x || y >= self.size.y {
            return ResolvedCell::LEAVE;
        }
        self.level(z)
            .and_then(|level| level.cells.get((y * self.size.x + x) as usize))
            .copied()
            .unwrap_or(ResolvedCell::LEAVE)
    }

    /// Puts every list in its fixed order.
    fn settle(&mut self) {
        for cells in self.marks.values_mut() {
            cells.sort_by_key(|cell| cell_key(*cell));
            cells.dedup();
        }
        for area in &mut self.areas {
            if let MapShape::Cells(cells) = &mut area.shape {
                cells.sort_by_key(|cell| cell_key(*cell));
                cells.dedup();
            }
        }
        self.areas.sort();
        self.spawns.sort_by_key(|spawn| match &spawn.place {
            SpawnPlace::Cell(cell) => (0, cell_key(*cell)),
            SpawnPlace::Area(_) => (1, (0, 0, 0)),
            SpawnPlace::Anywhere => (2, (0, 0, 0)),
        });
        self.items.sort_by_key(|items| cell_key(items.at));
        self.conduits.sort_by_key(|(cell, _)| cell_key(*cell));
        self.fields.sort_by_key(|(cell, _, _)| cell_key(*cell));
        self.devices.sort_by_key(|(cell, _)| cell_key(*cell));
        self.berths.sort_by_key(|berth| cell_key(berth.at));
        self.watched.sort_by_key(|cell| cell_key(*cell));
        self.watched.dedup();
        self.vars.sort_by_key(|(cell, _, _)| cell_key(*cell));
        self.decals.sort_by_key(|(cell, _)| cell_key(*cell));
        self.patterns.sort_by_key(|(cell, _)| cell_key(*cell));
        self.cleared.sort_by_key(|(cell, _)| cell_key(*cell));
        self.joints.sort_by_key(|joint| cell_key(joint.at));
        self.vehicles.sort_by_key(|vehicle| cell_key(vehicle.at));
    }

    /// This map turned: mirrored if the turn says so, then rotated, and
    /// moved so that its north-west corner is at the origin again.
    /// States that have a facing are turned with it (10 §3.3).
    ///
    /// Turning composes: `map.turned(a, defs).turned(b, defs)` is
    /// `map.turned(a.then(b), defs)`. Four quarter-turns, or two
    /// mirrors, give the map back.
    #[must_use]
    pub fn turned(&self, turn: Turn, defs: &MapDefs) -> ResolvedMap {
        let size = self.size;
        let cell = |cell: IVec3| turned_cell(cell.xy(), size, turn).with_z(cell.z);
        let new_size = turned_size(size, turn);
        let levels = self
            .levels
            .iter()
            .map(|level| {
                let mut cells = turn_grid(&level.cells, size, turn, ResolvedCell::LEAVE);
                for cell in &mut cells {
                    for layer in Layer::ALL {
                        if let Some(state) = cell.get(layer) {
                            cell.set(layer, Some(defs.facing.turned(layer, state, turn)));
                        }
                    }
                }
                ResolvedLevel { z: level.z, cells }
            })
            .collect();
        let mut turned = ResolvedMap {
            map: self.map,
            size: new_size,
            levels,
            anchor: self.anchor.map(cell),
            front: self.front.map(|front| front.turned(turn)),
            areas: self
                .areas
                .iter()
                .map(|area| ResolvedArea {
                    shape: match &area.shape {
                        MapShape::Box { min, max } => {
                            let (a, b) = (cell(*min), cell(*max));
                            MapShape::Box {
                                min: a.component_min(b),
                                max: a.component_max(b),
                            }
                        }
                        MapShape::Cells(cells) => {
                            MapShape::Cells(cells.iter().map(|at| cell(*at)).collect())
                        }
                    },
                    ..area.clone()
                })
                .collect(),
            marks: self
                .marks
                .iter()
                .map(|(name, cells)| (name.clone(), cells.iter().map(|at| cell(*at)).collect()))
                .collect(),
            spawns: self
                .spawns
                .iter()
                .map(|spawn| ResolvedSpawn {
                    place: match &spawn.place {
                        SpawnPlace::Cell(at) => SpawnPlace::Cell(cell(*at)),
                        other => other.clone(),
                    },
                    ..spawn.clone()
                })
                .collect(),
            items: self
                .items
                .iter()
                .map(|items| ResolvedItems {
                    at: cell(items.at),
                    ..items.clone()
                })
                .collect(),
            conduits: self
                .conduits
                .iter()
                .map(|(at, id)| (cell(*at), id.clone()))
                .collect(),
            fields: self
                .fields
                .iter()
                .map(|(at, id, amount)| (cell(*at), id.clone(), *amount))
                .collect(),
            devices: self
                .devices
                .iter()
                .map(|(at, device)| (cell(*at), device.clone()))
                .collect(),
            links: self.links.clone(),
            berths: self
                .berths
                .iter()
                .map(|berth| BerthCell {
                    at: cell(berth.at),
                    ..berth.clone()
                })
                .collect(),
            watched: self.watched.iter().map(|at| cell(*at)).collect(),
            vars: self
                .vars
                .iter()
                .map(|(at, name, value)| (cell(*at), name.clone(), value.clone()))
                .collect(),
            decals: self
                .decals
                .iter()
                .map(|(at, id)| (cell(*at), id.clone()))
                .collect(),
            patterns: self
                .patterns
                .iter()
                .map(|(at, pattern)| (cell(*at), pattern.clone()))
                .collect(),
            cleared: self
                .cleared
                .iter()
                .map(|(at, layer)| (cell(*at), *layer))
                .collect(),
            joints: self
                .joints
                .iter()
                .map(|joint| ResolvedJoint {
                    at: cell(joint.at),
                    dir: match joint.dir {
                        JointDir::Side(dir) => JointDir::Side(dir.turned(turn)),
                        vertical => vertical,
                    },
                    ..joint.clone()
                })
                .collect(),
            portals: self.portals.clone(),
            vehicles: self
                .vehicles
                .iter()
                .map(|vehicle| MapVehicle {
                    at: cell(vehicle.at),
                    facing: vehicle.facing.map(|facing| facing.turned(turn)),
                    ..vehicle.clone()
                })
                .collect(),
        };
        turned.settle();
        turned
    }
}

/// A grid of cells turned with its map.
fn turn_grid<T: Copy>(cells: &[T], size: IVec2, turn: Turn, fill: T) -> Vec<T> {
    let new_size = turned_size(size, turn);
    let mut out = vec![fill; cells.len()];
    for y in 0..size.y {
        for x in 0..size.x {
            let to = turned_cell(IVec2::new(x, y), size, turn);
            let from = (y * size.x + x) as usize;
            let into = (to.y * new_size.x + to.x) as usize;
            if let (Some(value), Some(slot)) = (cells.get(from), out.get_mut(into)) {
                *slot = *value;
            }
        }
    }
    out
}

/// The floor of a cell has been stated.
const STATED_FLOOR: u8 = 1;
/// The block of a cell has been stated.
const STATED_BLOCK: u8 = 2;
/// The fixture of a cell has been stated.
const STATED_FIXTURE: u8 = 4;
/// A character drew the cell.
const DRAWN: u8 = 8;

const fn stated_bit(layer: Layer) -> u8 {
    match layer {
        Layer::Floor => STATED_FLOOR,
        Layer::Block => STATED_BLOCK,
        Layer::Fixture => STATED_FIXTURE,
    }
}

/// A map being resolved: the result so far, with what a map laid over
/// it needs to know. A layer of a drawn cell that nothing has stated is
/// empty, and gives way to whatever is laid under or over it.
struct Draft {
    map: ResolvedMap,
    /// For each level, for each cell: which layers a spec has stated,
    /// and whether the cell is drawn.
    stated: Vec<Vec<u8>>,
    /// Cells that specs put in areas, with the area's name and kind.
    /// Touching cells of one name become one area when the map is done.
    area_cells: Vec<(IVec3, String, Option<AreaKindId>)>,
}

impl Draft {
    fn new(map: &CompiledMap) -> Draft {
        let resolved = ResolvedMap::empty(map);
        let cells = (map.size.x * map.size.y) as usize;
        Draft {
            stated: vec![vec![0; cells]; resolved.levels.len()],
            map: resolved,
            area_cells: Vec::new(),
        }
    }

    fn index(&self, at: IVec3) -> Option<(usize, usize)> {
        let (lowest, _) = self.map.level_range();
        let size = self.map.size;
        if at.x < 0 || at.y < 0 || at.x >= size.x || at.y >= size.y {
            return None;
        }
        let level = usize::try_from(at.z - lowest).ok()?;
        (level < self.map.levels.len()).then_some((level, (at.y * size.x + at.x) as usize))
    }

    /// Marks a cell as drawn, emptying it if it was not.
    fn draw(&mut self, at: IVec3) {
        if let Some((level, cell)) = self.index(at)
            && self.stated[level][cell] & DRAWN == 0
        {
            self.stated[level][cell] = DRAWN;
            self.map.levels[level].cells[cell] = ResolvedCell::DRAWN;
        }
    }

    /// States one layer of a cell: a state, or `None` to leave it.
    fn state(&mut self, at: IVec3, layer: Layer, state: Option<ElementState>) {
        if let Some((level, cell)) = self.index(at) {
            self.stated[level][cell] |= stated_bit(layer);
            self.map.levels[level].cells[cell].set(layer, state);
        }
    }

    fn turned(&self, turn: Turn, defs: &MapDefs) -> Draft {
        let size = self.map.size;
        Draft {
            map: self.map.turned(turn, defs),
            stated: self
                .stated
                .iter()
                .map(|level| turn_grid(level, size, turn, 0))
                .collect(),
            area_cells: self
                .area_cells
                .iter()
                .map(|(at, name, kind)| {
                    (
                        turned_cell(at.xy(), size, turn).with_z(at.z),
                        name.clone(),
                        *kind,
                    )
                })
                .collect(),
        }
    }

    /// Lays a nested map over this one with its north-west corner at
    /// `origin`. Where the nested map drew a cell, the layers it stated
    /// replace this map's; a layer it did not state keeps what this map
    /// has there, as a map's `default` shows through its characters.
    fn lay(&mut self, child: &Draft, origin: IVec3) {
        let moved = |at: IVec3| IVec3::new(at.x + origin.x, at.y + origin.y, at.z + origin.z);
        for (level_index, level) in child.map.levels.iter().enumerate() {
            for y in 0..child.map.size.y {
                for x in 0..child.map.size.x {
                    let from = (y * child.map.size.x + x) as usize;
                    let stated = child.stated[level_index][from];
                    if stated & DRAWN == 0 {
                        continue;
                    }
                    let at = moved(IVec3::new(x, y, level.z));
                    self.draw(at);
                    for layer in Layer::ALL {
                        if stated & stated_bit(layer) != 0 {
                            self.state(at, layer, level.cells[from].get(layer));
                        }
                    }
                }
            }
        }
        let from = &child.map;
        let into = &mut self.map;
        for (name, cells) in &from.marks {
            into.marks
                .entry(name.clone())
                .or_default()
                .extend(cells.iter().map(|at| moved(*at)));
        }
        into.areas
            .extend(from.areas.iter().map(|area| ResolvedArea {
                shape: match &area.shape {
                    MapShape::Box { min, max } => MapShape::Box {
                        min: moved(*min),
                        max: moved(*max),
                    },
                    MapShape::Cells(cells) => {
                        MapShape::Cells(cells.iter().map(|at| moved(*at)).collect())
                    }
                },
                ..area.clone()
            }));
        into.spawns
            .extend(from.spawns.iter().map(|spawn| ResolvedSpawn {
                place: match &spawn.place {
                    SpawnPlace::Cell(at) => SpawnPlace::Cell(moved(*at)),
                    other => other.clone(),
                },
                ..spawn.clone()
            }));
        into.items
            .extend(from.items.iter().map(|items| ResolvedItems {
                at: moved(items.at),
                ..items.clone()
            }));
        into.conduits.extend(
            from.conduits
                .iter()
                .map(|(at, id)| (moved(*at), id.clone())),
        );
        into.fields.extend(
            from.fields
                .iter()
                .map(|(at, id, amount)| (moved(*at), id.clone(), *amount)),
        );
        into.devices.extend(
            from.devices
                .iter()
                .map(|(at, device)| (moved(*at), device.clone())),
        );
        into.links.extend(from.links.iter().cloned());
        into.berths
            .extend(from.berths.iter().map(|berth| BerthCell {
                at: moved(berth.at),
                ..berth.clone()
            }));
        into.watched
            .extend(from.watched.iter().map(|at| moved(*at)));
        into.vars.extend(
            from.vars
                .iter()
                .map(|(at, name, value)| (moved(*at), name.clone(), value.clone())),
        );
        into.decals
            .extend(from.decals.iter().map(|(at, id)| (moved(*at), id.clone())));
        into.patterns.extend(
            from.patterns
                .iter()
                .map(|(at, pattern)| (moved(*at), pattern.clone())),
        );
        into.cleared
            .extend(from.cleared.iter().map(|(at, layer)| (moved(*at), *layer)));
        into.portals.extend(from.portals.iter().cloned());
        into.vehicles
            .extend(from.vehicles.iter().map(|vehicle| MapVehicle {
                at: moved(vehicle.at),
                ..vehicle.clone()
            }));
        self.area_cells.extend(
            child
                .area_cells
                .iter()
                .map(|(at, name, kind)| (moved(*at), name.clone(), *kind)),
        );
    }

    /// Makes areas of the cells specs put in them: touching cells of
    /// one name are one area.
    fn gather_areas(&mut self) {
        let mut by_name: BTreeMap<(String, Option<AreaKindId>), CellSet> = BTreeMap::new();
        for (at, name, kind) in self.area_cells.drain(..) {
            by_name
                .entry((name, kind))
                .or_default()
                .insert(cell_key(at));
        }
        for ((name, kind), mut cells) in by_name {
            while let Some(first) = cells.pop_first() {
                let mut area = vec![first];
                let mut next = 0;
                while next < area.len() {
                    let (z, y, x) = area[next];
                    next += 1;
                    for touching in [
                        (z, y, x - 1),
                        (z, y, x + 1),
                        (z, y - 1, x),
                        (z, y + 1, x),
                        (z - 1, y, x),
                        (z + 1, y, x),
                    ] {
                        if cells.remove(&touching) {
                            area.push(touching);
                        }
                    }
                }
                area.sort_unstable();
                self.map.areas.push(ResolvedArea {
                    name: name.clone(),
                    kind,
                    tags: Vec::new(),
                    shape: MapShape::Cells(
                        area.into_iter()
                            .map(|(z, y, x)| IVec3::new(x, y, z))
                            .collect(),
                    ),
                });
            }
        }
    }
}

/// What an expression in a map is evaluated against: the parameters of
/// one placement, and a generator seeded by the placement and the cell.
/// It has no world to answer from and no stream to draw from.
struct GenHost {
    /// The parameters, in the order the map declares them.
    params: Vec<Value>,
    /// The generator the random functions draw from, set anew from a
    /// positional hash before every evaluation.
    rng: Rng,
    /// How many times an expression went wrong while this map was
    /// resolved. A faulting expression gives null, which every use here
    /// treats as "not given".
    warnings: u32,
}

impl Host for GenHost {
    fn subject(&mut self, _slot: u16) -> Value {
        Value::Null
    }

    fn local(&mut self, slot: u16) -> Value {
        self.params
            .get(usize::from(slot))
            .cloned()
            .unwrap_or_default()
    }

    fn event_field(&mut self, _slot: u16) -> Value {
        Value::Null
    }

    fn property(&mut self, _property: PropertyId, _subject: Handle, _args: &[Value]) -> Value {
        Value::Null
    }

    fn variable(&mut self, _subject: Handle, _name: &str) -> Value {
        Value::Null
    }

    fn call(&mut self, _function: FunctionId, _args: &[Value]) -> Value {
        Value::Null
    }

    fn rng(&mut self) -> Option<&mut Rng> {
        Some(&mut self.rng)
    }

    fn warn(&mut self, _warning: RuntimeWarning) {
        self.warnings = self.warnings.saturating_add(1);
    }
}

/// What one layer of an alternative says for this placement.
#[derive(Clone, Copy)]
enum Stated {
    Nothing,
    Keep,
    State(ElementState),
}

/// A palette entry with its parameters filled in for one placement.
struct ReadyEntry {
    /// For each alternative: floor, block, fixture.
    layers: Vec<[Stated; 3]>,
    weights: Vec<Real>,
    total: Real,
    /// The alternative chosen for the whole placement, for `once`.
    once: Option<usize>,
}

/// One map being resolved for one placement.
struct Instance<'a> {
    defs: &'a MapDefs,
    map: &'a CompiledMap,
    seed: u64,
    host: GenHost,
    entries: Vec<ReadyEntry>,
}

/// Resolving, across a map and the maps nested in it.
struct Resolver<'a> {
    defs: &'a MapDefs,
    runner: Runner,
    order: EvalOrder,
}

fn core_value(value: Value) -> Option<clash_core::Value> {
    match value {
        Value::Bool(value) => Some(clash_core::Value::Bool(value)),
        Value::Int(value) => Some(clash_core::Value::Int(value)),
        Value::Real(value) => Some(clash_core::Value::Real(value)),
        Value::Str(value) => Some(clash_core::Value::Str(value.to_string())),
        _ => None,
    }
}

impl<'a> Resolver<'a> {
    fn number(
        &mut self,
        inst: &mut Instance<'a>,
        expr: &Expr<Number>,
        at: IVec3,
        salt: u64,
    ) -> Option<Real> {
        inst.host.rng = Rng::from_seed(pos_hash(inst.seed, EXPR_DOMAIN, at.x, at.y, at.z, salt));
        expr.eval(&mut self.runner.over(&self.defs.exprs, &mut inst.host))
    }

    fn int(
        &mut self,
        inst: &mut Instance<'a>,
        expr: &Expr<Int>,
        at: IVec3,
        salt: u64,
    ) -> Option<i64> {
        inst.host.rng = Rng::from_seed(pos_hash(inst.seed, EXPR_DOMAIN, at.x, at.y, at.z, salt));
        expr.eval(&mut self.runner.over(&self.defs.exprs, &mut inst.host))
    }

    fn any(&mut self, inst: &mut Instance<'a>, expr: &Expr<Any>, at: IVec3, salt: u64) -> Value {
        inst.host.rng = Rng::from_seed(pos_hash(inst.seed, EXPR_DOMAIN, at.x, at.y, at.z, salt));
        expr.value(&mut self.runner.over(&self.defs.exprs, &mut inst.host))
    }

    fn holds(&mut self, inst: &mut Instance<'a>, cond: &Cond, at: IVec3, salt: u64) -> bool {
        inst.host.rng = Rng::from_seed(pos_hash(inst.seed, EXPR_DOMAIN, at.x, at.y, at.z, salt));
        cond.holds(&mut self.runner.over(&self.defs.exprs, &mut inst.host))
    }

    /// Whether a chance comes up. One that is not given is certain.
    fn chance(
        &mut self,
        inst: &mut Instance<'a>,
        chance: &Option<Expr<Number>>,
        at: IVec3,
        salt: u64,
    ) -> bool {
        let Some(chance) = chance else { return true };
        let probability = self.number(inst, chance, at, salt).unwrap_or(Real::ZERO);
        Rng::from_seed(pos_hash(inst.seed, ROLL_DOMAIN, at.x, at.y, at.z, salt)).chance(probability)
    }

    /// A count. One that is not given is one.
    fn count(
        &mut self,
        inst: &mut Instance<'a>,
        count: &Option<CompiledCount>,
        at: IVec3,
        salt: u64,
    ) -> u32 {
        let count = match count {
            None => 1,
            Some(CompiledCount::Range(low, high)) => {
                Rng::from_seed(pos_hash(inst.seed, ROLL_DOMAIN, at.x, at.y, at.z, salt))
                    .range(*low, *high)
            }
            Some(CompiledCount::Expr(expr)) => self.int(inst, expr, at, salt).unwrap_or(0),
        };
        u32::try_from(count.max(0)).unwrap_or(u32::MAX)
    }

    /// The values of a map's parameters for one placement: what the
    /// placement gave, and for the rest what the map's own rule draws.
    fn params(&mut self, inst: &mut Instance<'a>, given: &VarMap) -> Result<(), ResolveError> {
        let map = inst.map;
        for (name, _) in given {
            if !map.params.iter().any(|param| param.name == name.as_str()) {
                return Err(ResolveError::UnknownParam {
                    map: map.name.clone(),
                    name: name.as_str().to_owned(),
                });
            }
        }
        for param in &map.params {
            let wrong = |given: String| ResolveError::ParamType {
                map: map.name.clone(),
                name: param.name.clone(),
                expected: param.ty.name(),
                given,
            };
            let salt = HashDomain::new(&param.name).raw();
            let hash = pos_hash(inst.seed, PARAM_DOMAIN, 0, 0, 0, salt);
            let value = match (given.get(&param.name), &param.rule) {
                (Some(value), _) => param
                    .ty
                    .accept(value)
                    .ok_or_else(|| wrong(format!("the {} `{value}`", value.type_name())))?,
                (None, CompiledRule::Pick(values)) => {
                    let index = Rng::from_seed(hash).below(values.len() as u64) as usize;
                    values
                        .get(index)
                        .cloned()
                        .ok_or_else(|| wrong("nothing".to_owned()))?
                }
                (None, CompiledRule::RangeInt(low, high)) => {
                    clash_core::Value::Int(Rng::from_seed(hash).range(*low, *high))
                }
                (None, CompiledRule::RangeReal(low, high)) => {
                    clash_core::Value::Real(*low + (*high - *low) * Rng::from_seed(hash).unit())
                }
                (None, CompiledRule::Default(value)) => value.clone(),
                (None, CompiledRule::Expr(expr)) => {
                    let value = self.any(inst, expr, PLACEMENT, salt);
                    let shown = value.to_string();
                    core_value(value)
                        .and_then(|value| param.ty.accept(&value))
                        .ok_or_else(|| wrong(format!("`{shown}` by its expression")))?
                }
                (None, CompiledRule::Required) => {
                    return Err(ResolveError::MissingParam {
                        map: map.name.clone(),
                        name: param.name.clone(),
                    });
                }
            };
            inst.host.params.push(Value::from(value));
        }
        Ok(())
    }

    /// Fills in the parameters of every palette entry for one
    /// placement.
    fn ready(&mut self, inst: &mut Instance<'a>) -> Result<(), ResolveError> {
        let map = inst.map;
        for (entry_index, entry) in map.entries.iter().enumerate() {
            let entry: &CompiledEntry = entry;
            let mut layers = Vec::with_capacity(entry.alternatives.len());
            let mut weights = Vec::with_capacity(entry.alternatives.len());
            for (alt_index, alt) in entry.alternatives.iter().enumerate() {
                let mut stated = [Stated::Nothing; 3];
                for (layer, slot) in [
                    (Layer::Floor, &alt.spec.floor),
                    (Layer::Block, &alt.spec.block),
                    (Layer::Fixture, &alt.spec.fixture),
                ] {
                    stated[layer.index()] = match slot {
                        StateSlot::Unset => Stated::Nothing,
                        StateSlot::Keep => Stated::Keep,
                        StateSlot::Fixed(state) => Stated::State(*state),
                        StateSlot::Template(parts) => {
                            let mut text = String::new();
                            for part in parts {
                                match part {
                                    TemplatePart::Text(piece) => text.push_str(piece),
                                    TemplatePart::Param(index) => {
                                        if let Some(Value::Str(value)) =
                                            inst.host.params.get(*index)
                                        {
                                            text.push_str(value);
                                        }
                                    }
                                }
                            }
                            let namespace =
                                Some(map.namespace.as_str()).filter(|name| !name.is_empty());
                            let world = &self.defs.world;
                            let found = match layer {
                                Layer::Floor => {
                                    world.floor_state(&text, namespace).map(FloorState::raw)
                                }
                                Layer::Block => {
                                    world.block_state(&text, namespace).map(BlockState::raw)
                                }
                                Layer::Fixture => {
                                    world.fixture_state(&text, namespace).map(FixtureState::raw)
                                }
                            };
                            Stated::State(found.map_err(|error| ResolveError::State {
                                map: map.name.clone(),
                                text,
                                error,
                            })?)
                        }
                    };
                }
                layers.push(stated);
                let weight = match &alt.weight {
                    None => Real::ONE,
                    Some(weight) => {
                        // A weight is for the placement, not for a cell.
                        let salt =
                            Salt::Weight.with(entry_index * entry.alternatives.len() + alt_index);
                        self.number(inst, weight, PLACEMENT, salt)
                            .unwrap_or(Real::ZERO)
                            .max(Real::ZERO)
                    }
                };
                weights.push(weight);
            }
            let total = weights.iter().fold(Real::ZERO, |sum, weight| sum + *weight);
            let mut ready = ReadyEntry {
                layers,
                weights,
                total,
                once: None,
            };
            if entry.once {
                ready.once = Some(choose(&ready, inst.seed, PLACEMENT, entry_index));
            }
            inst.entries.push(ready);
        }
        Ok(())
    }

    /// Lays one palette entry in a cell: the layers its chosen
    /// alternative states, and everything else it says.
    fn apply(&mut self, inst: &mut Instance<'a>, draft: &mut Draft, entry: u16, at: IVec3) {
        let entry_index = usize::from(entry);
        let Some(ready) = inst.entries.get(entry_index) else {
            return;
        };
        let alt = ready
            .once
            .unwrap_or_else(|| choose(ready, inst.seed, at, entry_index));
        let Some(stated) = ready.layers.get(alt).copied() else {
            return;
        };
        draft.draw(at);
        for layer in Layer::ALL {
            match stated[layer.index()] {
                Stated::Nothing => {}
                Stated::Keep => draft.state(at, layer, None),
                Stated::State(state) => draft.state(at, layer, Some(state)),
            }
        }
        let map = inst.map;
        let Some(extras) = map
            .entries
            .get(entry_index)
            .and_then(|entry| entry.alternatives.get(alt))
            .and_then(|alt| alt.spec.extras.as_deref())
        else {
            return;
        };
        let out = &mut draft.map;
        out.conduits
            .extend(extras.conduits.iter().map(|id| (at, id.clone())));
        out.fields.extend(
            extras
                .fields
                .iter()
                .map(|(id, amount)| (at, id.clone(), *amount)),
        );
        if let Some(items) = &extras.items
            && self.chance(inst, &items.chance, at, Salt::ItemsChance.with(entry_index))
        {
            let count = self.count(inst, &items.count, at, Salt::ItemsCount.with(entry_index));
            if count > 0 {
                out.items.push(ResolvedItems {
                    at,
                    source: items.source.clone(),
                    count,
                });
            }
        }
        if let Some(spawn) = &extras.spawn
            && self.chance(inst, &spawn.chance, at, Salt::SpawnChance.with(entry_index))
        {
            let count = self.count(inst, &spawn.count, at, Salt::SpawnCount.with(entry_index));
            if count > 0 {
                out.spawns.push(ResolvedSpawn {
                    who: spawn.who.clone(),
                    count,
                    place: SpawnPlace::Cell(at),
                });
            }
        }
        if let Some(device) = &extras.device {
            out.devices.push((at, device.clone()));
        }
        for mark in &extras.marks {
            out.marks.entry(mark.clone()).or_default().push(at);
        }
        if let Some((name, kind)) = &extras.berth {
            out.berths.push(BerthCell {
                at,
                name: name.clone(),
                kind: kind.clone(),
                approach: false,
            });
        }
        if let Some(name) = &extras.approach {
            out.berths.push(BerthCell {
                at,
                name: name.clone(),
                kind: String::new(),
                approach: true,
            });
        }
        if extras.watched {
            out.watched.push(at);
        }
        if let Some((name, kind)) = &extras.area {
            draft.area_cells.push((at, name.clone(), *kind));
        }
        out.vars.extend(
            extras
                .vars
                .iter()
                .map(|(name, value)| (at, name.clone(), value.clone())),
        );
        out.decals
            .extend(extras.decals.iter().map(|id| (at, id.clone())));
        if let Some(pattern) = &extras.pattern {
            out.patterns.push((at, pattern.clone()));
        }
        out.cleared
            .extend(extras.cleared.iter().map(|layer| (at, *layer)));
    }

    /// The cells of a level in the order they are to be visited.
    fn visits(&self, map: &CompiledMap, seed: u64, z: i32) -> Vec<(i32, i32)> {
        let mut cells: Vec<(i32, i32)> = (0..map.size.y)
            .flat_map(|y| (0..map.size.x).map(move |x| (x, y)))
            .collect();
        match self.order {
            EvalOrder::Forward => {}
            EvalOrder::Reverse => cells.reverse(),
            EvalOrder::Shuffled(by) => {
                Rng::from_seed(pos_hash(by, ORDER_DOMAIN, 0, 0, z, seed)).shuffle(&mut cells);
            }
        }
        cells
    }

    fn resolve(
        &mut self,
        id: MapId,
        given: &VarMap,
        seed: u64,
        depth: u32,
    ) -> Result<Option<Draft>, ResolveError> {
        let defs = self.defs;
        let map = defs.get(id).ok_or(ResolveError::UnknownMap(id))?;
        if depth > defs.limits.max_nesting {
            return Err(ResolveError::TooDeep {
                map: map.name.clone(),
                limit: defs.limits.max_nesting,
            });
        }
        let mut inst = Instance {
            defs,
            map,
            seed,
            host: GenHost {
                params: Vec::with_capacity(map.params.len()),
                rng: Rng::from_seed(seed),
                warnings: 0,
            },
            entries: Vec::with_capacity(map.entries.len()),
        };
        self.params(&mut inst, given)?;
        if let Some(condition) = &map.condition
            && !self.holds(&mut inst, condition, PLACEMENT, Salt::Condition.with(0))
        {
            return Ok(None);
        }
        self.ready(&mut inst)?;
        let mut draft = Draft::new(map);

        // The characters. Each cell is decided by the seed and its own
        // position, so the order of this loop decides nothing.
        let mut storeys: Vec<i32> = (0..map.levels.len() as i32)
            .map(|i| map.lowest + i)
            .collect();
        if self.order != EvalOrder::Forward {
            storeys.reverse();
        }
        for z in storeys {
            let level = map.level(z);
            if matches!(level, CompiledLevel::Empty) {
                continue;
            }
            for (x, y) in self.visits(map, seed, z) {
                let at = IVec3::new(x, y, z);
                match level {
                    CompiledLevel::Empty => {}
                    CompiledLevel::Rows(cells) => {
                        let entry = cells.get(map.cell_index(x, y)).copied().unwrap_or(NO_ENTRY);
                        if entry == NO_ENTRY {
                            continue;
                        }
                        if let Some(default) = map.default {
                            self.apply(&mut inst, &mut draft, default, at);
                        }
                        self.apply(&mut inst, &mut draft, entry, at);
                    }
                    CompiledLevel::Roof => {
                        if map.drawn(x, y, z - 1)
                            && let Some(roof) = map.roof.or(map.default)
                        {
                            self.apply(&mut inst, &mut draft, roof, at);
                        }
                    }
                }
            }
        }
        // From here on the order is the one the map is written in.
        draft.map.settle();
        draft.area_cells.sort_by_key(|(at, _, _)| cell_key(*at));

        for set in &map.set {
            for z in set.min.z..=set.max.z {
                for y in set.min.y..=set.max.y {
                    for x in set.min.x..=set.max.x {
                        self.apply(&mut inst, &mut draft, set.entry, IVec3::new(x, y, z));
                    }
                }
            }
        }

        for (name, cells) in &map.marks {
            draft
                .map
                .marks
                .entry(name.clone())
                .or_default()
                .extend(cells.iter().copied());
        }
        draft
            .map
            .areas
            .extend(map.areas.iter().map(|area| ResolvedArea {
                name: area.name.clone(),
                kind: area.kind,
                tags: area.tags.clone(),
                shape: MapShape::Box {
                    min: area.min,
                    max: area.max,
                },
            }));
        draft.map.conduits.extend(map.wiring.iter().cloned());
        draft
            .map
            .links
            .extend(map.links.iter().map(|(from, to, resource)| MapLink {
                from: from.clone(),
                to: to.clone(),
                resource: resource.clone(),
            }));
        draft
            .map
            .joints
            .extend(map.joints.iter().map(|joint| ResolvedJoint {
                id: joint.id.clone(),
                at: joint.at,
                dir: joint.dir,
                kind: joint.kind.clone(),
            }));
        draft
            .map
            .portals
            .extend(map.portals.iter().map(|(from, to)| MapPortal {
                from: from.clone(),
                to: to.clone(),
            }));
        draft
            .map
            .vehicles
            .extend(map.vehicles.iter().map(|(vehicle, at, facing)| MapVehicle {
                vehicle: vehicle.clone(),
                at: *at,
                facing: *facing,
            }));
        for (index, spawn) in map.spawns.iter().enumerate() {
            let spawn_def: &CompiledSpawn = &spawn.spawn;
            if !self.chance(
                &mut inst,
                &spawn_def.chance,
                PLACEMENT,
                Salt::MapSpawnChance.with(index),
            ) {
                continue;
            }
            let count = self.count(
                &mut inst,
                &spawn_def.count,
                PLACEMENT,
                Salt::MapSpawnCount.with(index),
            );
            if count == 0 {
                continue;
            }
            draft.map.spawns.push(ResolvedSpawn {
                who: spawn_def.who.clone(),
                count,
                place: match &spawn.at {
                    SpawnAt::Cell(at) => SpawnPlace::Cell(*at),
                    SpawnAt::Area(name) => SpawnPlace::Area(name.clone()),
                    SpawnAt::Anywhere => SpawnPlace::Anywhere,
                },
            });
        }

        for (index, place) in map.place.iter().enumerate() {
            self.place(&mut inst, &mut draft, index, place, depth)?;
        }
        draft.map.settle();
        Ok(Some(draft))
    }

    /// One entry of a map's `place`: the nested maps it puts in.
    fn place(
        &mut self,
        inst: &mut Instance<'a>,
        draft: &mut Draft,
        index: usize,
        place: &CompiledPlace,
        depth: u32,
    ) -> Result<(), ResolveError> {
        if !self.chance(
            inst,
            &place.chance,
            PLACEMENT,
            Salt::PlaceChance.with(index),
        ) {
            return Ok(());
        }
        let spots: Vec<IVec3> = match &place.at {
            PlaceAt::Cell(at) => vec![*at],
            PlaceAt::Area(name) => {
                let wanted =
                    self.count(inst, &place.count, PLACEMENT, Salt::PlaceCount.with(index));
                let mut cells: BTreeSet<(i32, i32, i32)> = BTreeSet::new();
                for area in inst.map.areas.iter().filter(|area| area.name == *name) {
                    for z in area.min.z..=area.max.z {
                        for y in area.min.y..=area.max.y {
                            for x in area.min.x..=area.max.x {
                                cells.insert((z, y, x));
                            }
                        }
                    }
                }
                cells.extend(
                    draft
                        .area_cells
                        .iter()
                        .filter(|(_, have, _)| have == name)
                        .map(|(at, _, _)| cell_key(*at)),
                );
                // Try the cells in an order the seed decides, keeping
                // those where a map fits and that are far enough from
                // the ones kept before.
                let mut ranked: Vec<(u64, IVec3)> = cells
                    .into_iter()
                    .map(|(z, y, x)| {
                        (
                            pos_hash(inst.seed, SCATTER_DOMAIN, x, y, z, index as u64),
                            IVec3::new(x, y, z),
                        )
                    })
                    .collect();
                ranked.sort_by_key(|(rank, at)| (*rank, cell_key(*at)));
                let mut kept: Vec<IVec3> = Vec::new();
                for (_, at) in ranked {
                    if kept.len() as u64 >= u64::from(wanted) {
                        break;
                    }
                    let crowded = kept.iter().any(|have| {
                        have.z == at.z
                            && IVec2::new(have.x - at.x, have.y - at.y).chebyshev()
                                <= i64::from(place.apart)
                    });
                    if !crowded && self.choice(inst, index, place, at).is_some() {
                        kept.push(at);
                    }
                }
                kept
            }
        };
        for at in spots {
            let Some((child_id, turn)) = self.choice(inst, index, place, at) else {
                continue;
            };
            let mut given = VarMap::default();
            for (param_index, (name, value)) in place.params.iter().enumerate() {
                let salt = Salt::PlaceParam.with(index * place.params.len() + param_index);
                let value = self.any(inst, value, at, salt);
                let shown = value.to_string();
                let Some(value) = core_value(value) else {
                    let child = self.defs.name(child_id).to_owned();
                    let expected = self
                        .defs
                        .get(child_id)
                        .and_then(|child| child.params.iter().find(|param| param.name == *name))
                        .map_or("value", |param| param.ty.name());
                    return Err(ResolveError::ParamType {
                        map: child,
                        name: name.clone(),
                        expected,
                        given: format!("`{shown}` by the map `{}`", inst.map.name),
                    });
                };
                given.set(name.as_str(), value);
            }
            let child_seed = pos_hash(inst.seed, NESTED_DOMAIN, at.x, at.y, at.z, index as u64);
            let Some(child) = self.resolve(child_id, &given, child_seed, depth + 1)? else {
                continue;
            };
            let Some(child_map) = self.defs.get(child_id) else {
                continue;
            };
            let origin = nested_origin(child_map.size, child_map.anchor, turn, at);
            let child = if turn == Turn::IDENTITY {
                child
            } else {
                child.turned(turn, self.defs)
            };
            draft.lay(&child, origin);
        }
        Ok(())
    }

    /// Which map, turned how, a `place` entry puts at a cell: one of
    /// those that fit there, chosen by the seed and the cell.
    fn choice(
        &self,
        inst: &Instance<'a>,
        index: usize,
        place: &CompiledPlace,
        at: IVec3,
    ) -> Option<(MapId, Turn)> {
        let mut options: Vec<(MapId, Turn)> = Vec::new();
        for &candidate in &place.candidates {
            let Some(child) = inst.defs.get(candidate) else {
                continue;
            };
            for turn in allowed_turns(place.turn, child.may_turn) {
                if fits_at(
                    inst.map,
                    child.size,
                    child.level_range(),
                    child.anchor,
                    turn,
                    at,
                ) {
                    options.push((candidate, turn));
                }
            }
        }
        let hash = pos_hash(inst.seed, PICK_DOMAIN, at.x, at.y, at.z, index as u64);
        let chosen = Rng::from_seed(hash).below(options.len() as u64) as usize;
        options.get(chosen).copied()
    }
}

/// Which alternative of an entry a cell gets: by weight, from the seed
/// and the cell.
fn choose(entry: &ReadyEntry, seed: u64, at: IVec3, entry_index: usize) -> usize {
    if entry.weights.len() < 2 || entry.total <= Real::ZERO {
        return 0;
    }
    let hash = pos_hash(
        seed,
        ALTERNATIVE_DOMAIN,
        at.x,
        at.y,
        at.z,
        entry_index as u64,
    );
    let mut left = Rng::from_seed(hash).unit() * entry.total;
    for (index, weight) in entry.weights.iter().enumerate() {
        if left < *weight {
            return index;
        }
        left -= *weight;
    }
    entry.weights.len() - 1
}

impl MapDefs {
    /// Resolves a map for one placement: its parameters chosen, every
    /// weighted alternative decided, its nested maps laid in, and the
    /// whole turned.
    ///
    /// `params` are the parameters the placement gives; the map's own
    /// rules supply the rest. `Ok(None)` means the map's `if` is false
    /// for these parameters: it is not placed.
    ///
    /// The result is a pure function of the arguments. The same seed
    /// gives the same map in every order of evaluation, on every
    /// thread, at any time (10 §14, X6): nothing here reads a clock, a
    /// stream of the simulation or the world.
    pub fn resolve(
        &self,
        map: MapId,
        params: &VarMap,
        seed: u64,
        turn: Turn,
    ) -> Result<Option<ResolvedMap>, ResolveError> {
        self.resolve_in_order(map, params, seed, turn, EvalOrder::Forward)
    }

    /// [`resolve`](MapDefs::resolve), visiting the cells of each level
    /// in a stated order. Every order gives the same map; this is here
    /// so that a test can show it.
    pub fn resolve_in_order(
        &self,
        map: MapId,
        params: &VarMap,
        seed: u64,
        turn: Turn,
        order: EvalOrder,
    ) -> Result<Option<ResolvedMap>, ResolveError> {
        let compiled = self.get(map).ok_or(ResolveError::UnknownMap(map))?;
        if turn != Turn::IDENTITY && !compiled.may_turn {
            return Err(ResolveError::TurnForbidden {
                map: compiled.name.clone(),
            });
        }
        let mut resolver = Resolver {
            defs: self,
            runner: Runner::new(),
            order,
        };
        let Some(mut draft) = resolver.resolve(map, params, seed, 0)? else {
            return Ok(None);
        };
        draft.gather_areas();
        draft.map.settle();
        Ok(Some(if turn == Turn::IDENTITY {
            draft.map
        } else {
            draft.map.turned(turn, self)
        }))
    }
}
