//! What movement needs of the world: [`Cosmos::neighbours`], the raw
//! geometry round a cell (02 §22), and [`Cosmos::find_path`], a search
//! over cells built on it.
//!
//! The world reports geometry. Whether a particular mover can make a
//! step, and what it costs, is for a [`MoveRules`]: the simulation's
//! movement rules, which know the mover. [`MoveProfile`] is a plain set
//! of rules for something that walks, for callers that have no better.

use core::cmp::Reverse;
use std::collections::BinaryHeap;

use clash_core::{Dir8, IVec3, LookupMap, Real};

use crate::cosmos::{Cosmos, GridView};
use crate::defs::{CellFlags, ElementFlags, EngineFlag, Layer, WALK_STEP_M, WorldDefs};
use crate::dims::{CELL_M, STOREY_M};
use crate::error::WorldError;
use crate::grid::Column;
use crate::iter::step_cost;
use crate::pos::{CellBox, CellPos, GridId, Location};
use crate::topology::Topology;

use super::locate;
use super::stamp::{RevParts, RevStamp};

/// How many cells a search may settle before it gives up, unless the
/// caller says otherwise.
pub const PATH_DEFAULT_MAX_NODES: u32 = 20_000;

/// How much dearer a step into a `rough` cell is for a [`MoveProfile`]
/// that does not say.
pub const PROFILE_ROUGH_FACTOR: Real = Real::of(2.0);

/// What opening something in the way costs a [`MoveProfile`] that does
/// not say, as metres of walking.
pub const PROFILE_OPEN_COST_M: Real = Real::of(2.0);

/// What holds up someone standing in a cell.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Footing {
    /// Nothing: the cell is open, and what is in it falls.
    None,
    /// The cell's own floor.
    Floor,
    /// The solid top of the block in the cell below.
    Ground,
    /// A ramp, stairs or ladder in the cell below: whoever is at its top
    /// stands on it.
    Connection,
}

/// A vertical connection in a cell, to the cell above it (02 §5.2).
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub enum Connection {
    /// Walked up.
    Ramp,
    /// Climbed on foot.
    Stairs,
    /// Climbed by hand.
    Ladder,
}

/// What movement rules need to know about one cell.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct CellGeometry {
    /// The cell, inside its grid.
    pub cell: CellPos,
    /// Its derived flags.
    pub flags: CellFlags,
    /// What holds up someone standing in it.
    pub footing: Footing,
    /// The height in metres a mover must step over to enter: that of the
    /// tallest block or fixture that stands in the way of walking, being
    /// neither `passable` nor a `ramp`, `stairs` or `liquid`. Zero when
    /// nothing does.
    pub obstacle_m: Real,
    /// The vertical connection it holds, if any: where a block and a
    /// fixture differ, the first of ramp, stairs, ladder.
    pub connection: Option<Connection>,
    /// Whether it is filled with a still liquid.
    pub liquid: bool,
    /// Whether any of its elements is `rough`.
    pub rough: bool,
    /// Whether what stands in the way of walking here could stand aside:
    /// every such element has a declared state in which it does not. A
    /// closed door has; a wall has not. `false` when nothing is in the
    /// way.
    pub opens: bool,
}

impl CellGeometry {
    /// Whether something stands in the way of ordinary walking here:
    /// the obstacle is taller than a step ([`WALK_STEP_M`]).
    pub fn walk_blocked(&self) -> bool {
        self.flags.contains(CellFlags::WALK_BLOCKED)
    }
}

/// The connection an element state offers, if any.
fn connection_of(flags: ElementFlags) -> Option<Connection> {
    if flags.has(EngineFlag::Ramp) {
        Some(Connection::Ramp)
    } else if flags.has(EngineFlag::Stairs) {
        Some(Connection::Stairs)
    } else if flags.has(EngineFlag::Ladder) {
        Some(Connection::Ladder)
    } else {
        None
    }
}

/// What holds up someone in a cell of a column, given the cell's flags.
fn footing_in(defs: &WorldDefs, column: &Column, cell: CellPos, flags: CellFlags) -> Footing {
    if flags.contains(CellFlags::HAS_FLOOR) {
        return Footing::Floor;
    }
    if column.flags(cell.below()).contains(CellFlags::SOLID_TOP) {
        return Footing::Ground;
    }
    let below = column.cell(cell.below());
    let connects = connection_of(defs.block_info(below.block).flags).is_some()
        || connection_of(defs.fixture_info(below.fixture).flags).is_some();
    if connects {
        Footing::Connection
    } else {
        Footing::None
    }
}

/// The geometry of one cell, or why it cannot be read.
fn cell_geometry(
    view: &GridView<'_>,
    defs: &WorldDefs,
    cell: CellPos,
) -> Result<CellGeometry, WorldError> {
    let (column, cell) = locate(view, cell)?;
    let flags = column.flags(cell);
    let states = column.cell(cell);
    let block = defs.block_info(states.block);
    let fixture = defs.fixture_info(states.fixture);
    let floor = defs.floor_info(states.floor);
    let in_way = |height: Real, flags: ElementFlags| {
        let steps_aside = flags.has(EngineFlag::Passable)
            || flags.has(EngineFlag::Ramp)
            || flags.has(EngineFlag::Stairs)
            || flags.has(EngineFlag::Liquid);
        if steps_aside { Real::ZERO } else { height }
    };
    let footing = footing_in(defs, column, cell, flags);
    // What is in the way could stand aside if each element that is in
    // the way has some state in which it is not.
    let opens = flags.contains(CellFlags::WALK_BLOCKED)
        && [
            (Layer::Block, states.block.raw()),
            (Layer::Fixture, states.fixture.raw()),
        ]
        .into_iter()
        .all(|(layer, state)| {
            let blocks = defs
                .info(layer, state)
                .cell
                .contains(CellFlags::WALK_BLOCKED);
            !blocks
                || defs.element(layer, state).is_some_and(|element| {
                    element
                        .states
                        .iter()
                        .any(|other| !other.cell.contains(CellFlags::WALK_BLOCKED))
                })
        });
    Ok(CellGeometry {
        cell,
        flags,
        footing,
        obstacle_m: in_way(block.height, block.flags).max(in_way(fixture.height, fixture.flags)),
        connection: connection_of(block.flags)
            .into_iter()
            .chain(connection_of(fixture.flags))
            .min(),
        liquid: block.flags.has(EngineFlag::Liquid) || fixture.flags.has(EngineFlag::Liquid),
        rough: block.flags.has(EngineFlag::Rough)
            || fixture.flags.has(EngineFlag::Rough)
            || floor.flags.has(EngineFlag::Rough),
        opens,
    })
}

/// Which way a step goes.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum StepKind {
    /// To one of the eight cells round about, on the same storey.
    Level(Dir8),
    /// To the cell directly above.
    Up,
    /// To the cell directly below.
    Down,
}

impl StepKind {
    /// The ten steps in the order they are always listed: the eight
    /// round about clockwise from north, then up, then down (02 §3.3).
    pub const ALL: [StepKind; 10] = [
        StepKind::Level(Dir8::North),
        StepKind::Level(Dir8::NorthEast),
        StepKind::Level(Dir8::East),
        StepKind::Level(Dir8::SouthEast),
        StepKind::Level(Dir8::South),
        StepKind::Level(Dir8::SouthWest),
        StepKind::Level(Dir8::West),
        StepKind::Level(Dir8::NorthWest),
        StepKind::Up,
        StepKind::Down,
    ];

    /// The offset the step moves by.
    pub fn delta(self) -> IVec3 {
        match self {
            StepKind::Level(dir) => dir.delta().with_z(0),
            StepKind::Up => IVec3::new(0, 0, 1),
            StepKind::Down => IVec3::new(0, 0, -1),
        }
    }

    /// The length of the step in metres between cell centres: a cell
    /// width, √2 of one for a diagonal, a storey up or down (02 §3.3).
    pub fn length_m(self) -> Real {
        match self {
            StepKind::Level(dir) => step_cost(dir),
            StepKind::Up | StepKind::Down => STOREY_M,
        }
    }

    /// The offsets of the two cells a diagonal step passes between.
    /// `None` for any other step.
    fn sides(self) -> Option<[IVec3; 2]> {
        match self {
            StepKind::Level(dir) if dir.is_diagonal() => {
                let delta = dir.delta();
                Some([IVec3::new(delta.x, 0, 0), IVec3::new(0, delta.y, 0)])
            }
            _ => None,
        }
    }
}

/// One step from a cell to a cell next to it, with the geometry a rule
/// needs to judge it.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Step {
    /// Which way it goes.
    pub kind: StepKind,
    /// The cell stepped from.
    pub from: CellGeometry,
    /// The cell stepped into.
    pub to: CellGeometry,
    /// For a diagonal step, the two cells it passes between: the one
    /// beside `from` along `x`, then the one along `y`. `None` for a
    /// cell that cannot be read. Both `None` for a step that is not
    /// diagonal.
    pub sides: [Option<CellGeometry>; 2],
    /// Its length in metres between cell centres.
    pub length_m: Real,
}

/// The raw geometry round a cell, for movement rules: the answer of
/// [`Cosmos::neighbours`].
///
/// A neighbour is `None` when it cannot be read: its tract is not
/// loaded, or it lies beyond an edge of the realm. There is nothing
/// there to step into.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct NeighbourGeometry {
    /// The grid.
    pub grid: GridId,
    /// The cell itself.
    pub here: CellGeometry,
    /// The eight cells round about on the same storey, clockwise from
    /// north.
    pub around: [Option<CellGeometry>; 8],
    /// The cell directly above.
    pub above: Option<CellGeometry>,
    /// The cell directly below.
    pub below: Option<CellGeometry>,
}

impl NeighbourGeometry {
    /// The neighbour a step leads to.
    pub fn neighbour(&self, kind: StepKind) -> Option<CellGeometry> {
        match kind {
            StepKind::Level(dir) => self.around[dir.index()],
            StepKind::Up => self.above,
            StepKind::Down => self.below,
        }
    }

    /// The step of a kind from the cell, or `None` if there is no cell
    /// there to step into.
    pub fn step(&self, kind: StepKind) -> Option<Step> {
        let to = self.neighbour(kind)?;
        let sides = match kind {
            StepKind::Level(dir) if dir.is_diagonal() => {
                let delta = dir.delta();
                let along_x = Dir8::from_delta(clash_core::IVec2::new(delta.x, 0));
                let along_y = Dir8::from_delta(clash_core::IVec2::new(0, delta.y));
                [
                    along_x.and_then(|dir| self.around[dir.index()]),
                    along_y.and_then(|dir| self.around[dir.index()]),
                ]
            }
            _ => [None, None],
        };
        Some(Step {
            kind,
            from: self.here,
            to,
            sides,
            length_m: kind.length_m(),
        })
    }

    /// The steps a mover can make from the cell under some rules, each
    /// with its cost, in the order of [`StepKind::ALL`].
    pub fn steps<'a, R: MoveRules + ?Sized>(
        &'a self,
        rules: &'a R,
    ) -> impl Iterator<Item = (Step, Real)> + 'a {
        StepKind::ALL.into_iter().filter_map(move |kind| {
            let step = self.step(kind)?;
            Some((step, price(rules, &step)?))
        })
    }
}

/// What a mover can do: which cells it can be in, which steps it can
/// make and what each costs. The simulation's movement rules implement
/// this; [`MoveProfile`] is a plain walker.
///
/// A character's rules answer from what the character believes
/// (01 R11): whether a door is locked is not for the world to say.
pub trait MoveRules {
    /// Whether the mover can be in a cell at all. A search never enters
    /// a cell for which this is false, and gives up at once if its goal
    /// is such a cell.
    fn can_occupy(&self, cell: &CellGeometry) -> bool;

    /// The cost of a step into a cell the mover can occupy, in metres of
    /// ordinary travel, or `None` if the mover cannot make it. A cost
    /// below the step's own length is raised to that length: a search
    /// relies on no step being cheaper than the distance it covers.
    fn step_cost(&self, step: &Step) -> Option<Real>;
}

/// The cost of a step under some rules, or `None` if it cannot be made.
fn price<R: MoveRules + ?Sized>(rules: &R, step: &Step) -> Option<Real> {
    if !rules.can_occupy(&step.to) {
        return None;
    }
    Some(rules.step_cost(step)?.max(step.length_m))
}

/// Rules for something that walks: it needs footing, steps over low
/// things, goes up and down by ramps, stairs and ladders, and does not
/// cut corners.
///
/// | Step | Allowed when |
/// |------|--------------|
/// | Level | The cell entered has footing and nothing in the way taller than `step_up_m` (or something that `opens`, if the mover opens things), and is not liquid unless the mover swims. |
/// | Diagonal | As level, and neither of the two cells it passes between has something in the way taller than `step_up_m`, whether or not that could be opened. A cell that cannot be read is in the way. |
/// | Up | The cell left holds a ramp, stairs or ladder. |
/// | Down | The cell entered holds a ramp, stairs or ladder. |
///
/// A step costs its length, times `rough_factor` into a rough cell,
/// times `climb_factor` between storeys, plus `open_cost_m` where
/// something had to be opened.
///
/// ```
/// use clash_world::MoveProfile;
///
/// let walker = MoveProfile::walker();
/// let guard = MoveProfile { opens: true, ..walker };
/// assert!(guard.opens && !walker.opens);
/// ```
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct MoveProfile {
    /// The tallest thing the mover steps over, in metres.
    pub step_up_m: Real,
    /// How many times dearer a step into a rough cell is. At least 1.
    pub rough_factor: Real,
    /// How many times dearer a step between storeys is. At least 1.
    pub climb_factor: Real,
    /// Whether the mover climbs ladders.
    pub ladders: bool,
    /// Whether the mover enters liquid cells.
    pub swims: bool,
    /// Whether the mover passes what could stand aside: doors.
    pub opens: bool,
    /// What opening costs, as metres of walking.
    pub open_cost_m: Real,
}

impl MoveProfile {
    /// Someone on foot who steps over what ordinary walking steps over
    /// ([`WALK_STEP_M`]), climbs ladders, does not swim and does not
    /// open doors.
    pub const fn walker() -> MoveProfile {
        MoveProfile {
            step_up_m: WALK_STEP_M,
            rough_factor: PROFILE_ROUGH_FACTOR,
            climb_factor: Real::ONE,
            ladders: true,
            swims: false,
            opens: false,
            open_cost_m: PROFILE_OPEN_COST_M,
        }
    }

    /// Whether something in a cell stands in this mover's way.
    fn in_way(&self, cell: &CellGeometry) -> bool {
        cell.obstacle_m > self.step_up_m
    }
}

impl Default for MoveProfile {
    fn default() -> MoveProfile {
        MoveProfile::walker()
    }
}

impl MoveRules for MoveProfile {
    fn can_occupy(&self, cell: &CellGeometry) -> bool {
        if self.in_way(cell) && !(self.opens && cell.opens) {
            return false;
        }
        if cell.liquid {
            return self.swims;
        }
        cell.footing != Footing::None
    }

    fn step_cost(&self, step: &Step) -> Option<Real> {
        let between = |connection: Option<Connection>| match connection {
            Some(Connection::Ladder) => self.ladders,
            Some(_) => true,
            None => false,
        };
        let mut cost = step.length_m;
        match step.kind {
            StepKind::Level(dir) => {
                if dir.is_diagonal() {
                    let clear = step
                        .sides
                        .iter()
                        .all(|side| side.as_ref().is_some_and(|side| !self.in_way(side)));
                    if !clear {
                        return None;
                    }
                }
            }
            StepKind::Up => {
                if !between(step.from.connection) {
                    return None;
                }
                cost *= self.climb_factor;
            }
            StepKind::Down => {
                if !between(step.to.connection) {
                    return None;
                }
                cost *= self.climb_factor;
            }
        }
        if step.to.rough {
            cost *= self.rough_factor;
        }
        if self.in_way(&step.to) {
            cost += self.open_cost_m;
        }
        Some(cost)
    }
}

/// The limits of one search.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct PathLimits {
    /// How many cells the search may settle before it gives up.
    pub max_nodes: u32,
    /// The dearest path worth having, in metres of ordinary travel.
    pub max_cost_m: Real,
}

impl PathLimits {
    /// [`PATH_DEFAULT_MAX_NODES`] cells and no limit on cost.
    pub const DEFAULT: PathLimits = PathLimits {
        max_nodes: PATH_DEFAULT_MAX_NODES,
        max_cost_m: Real::INFINITY,
    };
}

impl Default for PathLimits {
    fn default() -> PathLimits {
        PathLimits::DEFAULT
    }
}

/// Why no path was found.
#[derive(Clone, PartialEq, Eq, Debug, thiserror::Error)]
pub enum PathFail {
    /// The start or the goal cannot be read.
    #[error("{0}")]
    World(#[from] WorldError),
    /// The start and the goal are in different grids.
    #[error("{from} and {to} are in different grids")]
    OtherGrid {
        /// The start.
        from: Location,
        /// The goal.
        to: Location,
    },
    /// The mover cannot be in the goal cell.
    #[error("the mover cannot be at {0}")]
    GoalBlocked(Location),
    /// Every cell the mover can reach was tried and the goal is not one
    /// of them.
    #[error("there is no way from {from} to {to}")]
    NoRoute {
        /// The start.
        from: Location,
        /// The goal.
        to: Location,
    },
    /// Every path within the cost limit was tried and none reaches the
    /// goal; a dearer one might.
    #[error("there is no way to {to} within {max_cost_m} m")]
    TooFar {
        /// The goal.
        to: Location,
        /// The limit that was set.
        max_cost_m: Real,
    },
    /// The search settled as many cells as it was allowed and had not
    /// reached the goal.
    #[error("no way to {to} was found within {max_nodes} cells searched")]
    BudgetSpent {
        /// The goal.
        to: Location,
        /// The limit that was set.
        max_nodes: u32,
    },
}

/// A way from one cell to another: the answer of [`Cosmos::find_path`].
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Path {
    grid: GridId,
    cells: Vec<CellPos>,
    cost_m: Real,
    settled: u32,
    stamp: RevStamp,
}

impl Path {
    /// The grid the path is in.
    pub fn grid(&self) -> GridId {
        self.grid
    }

    /// The cells of the path in order, from the start to the goal, both
    /// included. Each is next to the one before.
    pub fn cells(&self) -> &[CellPos] {
        &self.cells
    }

    /// How many steps the path takes: one less than its cells.
    pub fn steps(&self) -> usize {
        self.cells.len().saturating_sub(1)
    }

    /// What the path costs, in metres of ordinary travel.
    pub fn cost_m(&self) -> Real {
        self.cost_m
    }

    /// How many cells the search settled to find it.
    pub fn settled(&self) -> u32 {
        self.settled
    }

    /// The first cell.
    pub fn start(&self) -> Option<Location> {
        self.cells
            .first()
            .map(|cell| Location::new(self.grid, *cell))
    }

    /// The last cell.
    pub fn goal(&self) -> Option<Location> {
        self.cells
            .last()
            .map(|cell| Location::new(self.grid, *cell))
    }

    /// Whether the path can still be trusted: no chunk that holds one of
    /// its cells, or a cell beside, above or below one, has been edited
    /// since it was found. A path that is not current may still be
    /// walkable; a path that is current may no longer be the cheapest,
    /// if a wall came down somewhere else.
    pub fn is_current(&self, cosmos: &Cosmos) -> bool {
        self.stamp.is_current(cosmos)
    }

    /// The chunk revisions the path was read from.
    pub fn stamp(&self) -> &RevStamp {
        &self.stamp
    }
}

/// A cell the search has met.
#[derive(Clone, Copy, Debug)]
struct Node {
    /// `None` for a cell that cannot be read.
    geometry: Option<CellGeometry>,
    /// The cheapest cost found so far from the start.
    cost: Real,
    /// The node this one is reached from at that cost.
    parent: u32,
    settled: bool,
}

/// An entry of the open list: the estimate of the whole path through a
/// cell, the estimate of what is left, and the cell. The least comes
/// first; where two are equal the one nearer the goal, then the earlier
/// cell storey by storey, row by row.
type Open = Reverse<(Real, Real, CellPos, u32)>;

/// The working storage of [`Cosmos::find_path`]: keep one and search
/// with it again, and a search allocates only its answer.
#[derive(Debug, Default)]
pub struct PathFinder {
    nodes: Vec<Node>,
    index: LookupMap<CellPos, u32>,
    open: BinaryHeap<Open>,
}

/// The least a path between two cells can cost: straight and diagonal
/// steps on the level and a storey for each storey between.
fn estimate(topology: &Topology, from: CellPos, to: CellPos) -> Real {
    let delta = topology.delta(from, to);
    let (dx, dy) = (i64::from(delta.x).abs(), i64::from(delta.y).abs());
    let diagonal = dx.min(dy);
    let straight = dx.max(dy) - diagonal;
    Real::from_i64(straight) * CELL_M
        + Real::from_i64(diagonal) * step_cost(Dir8::NorthEast)
        + Real::from_i64(i64::from(delta.z).abs()) * STOREY_M
}

impl PathFinder {
    /// Working storage with nothing in it.
    pub fn new() -> PathFinder {
        PathFinder::default()
    }

    /// The node of a cell, made if this is the first time it is met.
    fn node(&mut self, view: &GridView<'_>, defs: &WorldDefs, cell: CellPos) -> Option<u32> {
        // Unreadable cells are remembered by the position given.
        let key = view.normalize(cell).unwrap_or(cell);
        if let Some(at) = self.index.get(&key) {
            return Some(*at);
        }
        let at = u32::try_from(self.nodes.len()).ok()?;
        self.nodes.push(Node {
            geometry: cell_geometry(view, defs, key).ok(),
            cost: Real::INFINITY,
            parent: at,
            settled: false,
        });
        self.index.insert(key, at);
        Some(at)
    }

    fn geometry(&self, at: u32) -> Option<CellGeometry> {
        self.nodes.get(at as usize)?.geometry
    }

    /// The cheapest way from one cell to another for a mover with the
    /// given rules. See [`Cosmos::find_path`].
    pub fn find<R: MoveRules + ?Sized>(
        &mut self,
        cosmos: &Cosmos,
        from: Location,
        to: Location,
        rules: &R,
        limits: PathLimits,
    ) -> Result<Path, PathFail> {
        self.nodes.clear();
        self.index.clear();
        self.open.clear();
        let view = cosmos.grid(from.grid)?;
        let defs = cosmos.defs();
        let start_geometry = cell_geometry(&view, defs, from.cell)?;
        let from = Location::new(from.grid, start_geometry.cell);
        if from.grid != to.grid {
            return Err(PathFail::OtherGrid { from, to });
        }
        let goal_geometry = cell_geometry(&view, defs, to.cell)?;
        let to = Location::new(to.grid, goal_geometry.cell);
        if from != to && !rules.can_occupy(&goal_geometry) {
            return Err(PathFail::GoalBlocked(to));
        }
        let topology = *view.topology();
        let no_route = PathFail::NoRoute { from, to };
        let Some(start) = self.node(&view, defs, from.cell) else {
            return Err(no_route);
        };
        if let Some(node) = self.nodes.get_mut(start as usize) {
            node.cost = Real::ZERO;
        }
        let left = estimate(&topology, from.cell, to.cell);
        self.open.push(Reverse((left, left, from.cell, start)));
        let mut settled = 0_u32;
        let mut cut_by_cost = false;
        while let Some(Reverse((_, _, cell, at))) = self.open.pop() {
            let Some(node) = self.nodes.get_mut(at as usize) else {
                continue;
            };
            if node.settled {
                continue;
            }
            node.settled = true;
            let cost = node.cost;
            let Some(here) = node.geometry else {
                continue;
            };
            if cell == to.cell {
                return Ok(self.finish(&view, from.grid, at, cost, settled));
            }
            if settled >= limits.max_nodes {
                return Err(PathFail::BudgetSpent {
                    to,
                    max_nodes: limits.max_nodes,
                });
            }
            settled += 1;
            for kind in StepKind::ALL {
                let Some(next) = self.node(&view, defs, cell.offset(kind.delta())) else {
                    continue;
                };
                let Some(there) = self.geometry(next) else {
                    continue;
                };
                if self
                    .nodes
                    .get(next as usize)
                    .is_none_or(|node| node.settled)
                {
                    continue;
                }
                let sides = match kind.sides() {
                    Some(offsets) => offsets.map(|offset| {
                        let side = self.node(&view, defs, cell.offset(offset))?;
                        self.geometry(side)
                    }),
                    None => [None, None],
                };
                let step = Step {
                    kind,
                    from: here,
                    to: there,
                    sides,
                    length_m: kind.length_m(),
                };
                let Some(step_cost) = price(rules, &step) else {
                    continue;
                };
                let reached = cost + step_cost;
                let left = estimate(&topology, there.cell, to.cell);
                if reached + left > limits.max_cost_m {
                    cut_by_cost = true;
                    continue;
                }
                if let Some(node) = self.nodes.get_mut(next as usize)
                    && reached < node.cost
                {
                    node.cost = reached;
                    node.parent = at;
                    self.open
                        .push(Reverse((reached + left, left, there.cell, next)));
                }
            }
        }
        Err(if cut_by_cost {
            PathFail::TooFar {
                to,
                max_cost_m: limits.max_cost_m,
            }
        } else {
            no_route
        })
    }

    /// Reads the path back from the goal's node to the start.
    fn finish(
        &self,
        view: &GridView<'_>,
        grid: GridId,
        goal: u32,
        cost_m: Real,
        settled: u32,
    ) -> Path {
        let mut cells = Vec::new();
        let mut at = goal;
        // A node's parent is itself only at the start. The count guards
        // against a chain that does not end, which cannot be built.
        for _ in 0..=self.nodes.len() {
            let Some(node) = self.nodes.get(at as usize) else {
                break;
            };
            if let Some(geometry) = node.geometry {
                cells.push(geometry.cell);
            }
            if node.parent == at {
                break;
            }
            at = node.parent;
        }
        cells.reverse();
        let mut stamp = RevStamp::new(RevParts::STRUCTURE);
        let around = IVec3::new(1, 1, 1);
        for cell in &cells {
            stamp.record_box(view, CellBox::around(*cell, around));
        }
        Path {
            grid,
            cells,
            cost_m,
            settled,
            stamp,
        }
    }
}

impl Cosmos {
    /// The raw geometry round a cell, for movement rules (02 §22): the
    /// cell itself, the eight round about, the one above and the one
    /// below, each with its flags, what would hold a mover up in it, the
    /// height of what is in the way, and any ramp, stairs or ladder.
    ///
    /// The world reports; it does not judge. Which steps a mover can
    /// make is [`NeighbourGeometry::steps`] with that mover's
    /// [`MoveRules`].
    pub fn neighbours(&self, loc: Location) -> Result<NeighbourGeometry, WorldError> {
        let view = self.grid(loc.grid)?;
        let defs = self.defs();
        let here = cell_geometry(&view, defs, loc.cell)?;
        let at = |offset: IVec3| cell_geometry(&view, defs, here.cell.offset(offset)).ok();
        Ok(NeighbourGeometry {
            grid: loc.grid,
            here,
            around: Dir8::ALL.map(|dir| at(dir.delta().with_z(0))),
            above: at(IVec3::new(0, 0, 1)),
            below: at(IVec3::new(0, 0, -1)),
        })
    }

    /// What holds up someone in a cell: its floor, the solid top of the
    /// block below, a ramp, stairs or ladder in the cell below, or
    /// nothing (02 §5.2). Something in a cell with no footing is
    /// unsupported.
    pub fn footing(&self, loc: Location) -> Result<Footing, WorldError> {
        let view = self.grid(loc.grid)?;
        let (column, cell) = locate(&view, loc.cell)?;
        Ok(footing_in(self.defs(), column, cell, column.flags(cell)))
    }

    /// The cheapest way from one cell to another for a mover with the
    /// given rules, as a search over cells (A*).
    ///
    /// Steps are the ten of [`StepKind::ALL`]; `rules` says which can be
    /// made and what each costs. The answer is a cheapest path: no other
    /// costs less. Where several cost the same the choice is fixed: the
    /// search settles cells in order of the estimated cost of the whole
    /// path through them, then of what is left to the goal, then of
    /// position (storey by storey, row by row), and tries steps in the
    /// order of [`StepKind::ALL`]; a cell keeps the first way found to
    /// it at its cheapest cost.
    ///
    /// Cells that are not loaded or lie beyond an edge are never
    /// entered. The search stops with a reason when the goal cannot be
    /// reached, when every path left would cost more than
    /// `limits.max_cost_m`, or when it has settled `limits.max_nodes`
    /// cells.
    ///
    /// This reads the world as it is. A character's search passes rules
    /// that answer from what the character believes (01 R11).
    ///
    /// Each call allocates its working storage. Keep a [`PathFinder`]
    /// to search many times without.
    pub fn find_path<R: MoveRules + ?Sized>(
        &self,
        from: Location,
        to: Location,
        rules: &R,
        limits: PathLimits,
    ) -> Result<Path, PathFail> {
        PathFinder::new().find(self, from, to, rules, limits)
    }
}
