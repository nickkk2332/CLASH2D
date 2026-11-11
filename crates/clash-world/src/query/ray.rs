//! Rays through cells: [`Cosmos::raycast`] and [`Cosmos::line_of_sight`]
//! (02 §22).
//!
//! A ray steps through 1 m × 1 m × 3 m cells in the order of
//! [`segment_cells`](crate::iter::segment_cells) and respects the heights
//! of what stands in them: a block or fixture fills the footprint of its
//! cell from the base up to its height, and a ray that stays above that
//! height passes over it.

use clash_core::{EntityId, Real};

use crate::cosmos::{Cosmos, GridView};
use crate::defs::{CellFlags, ElementState, EngineFlag, Layer, StateInfo, WorldDefs};
use crate::dims::{CELL_M, STOREY_M};
use crate::error::WorldError;
use crate::grid::Column;
use crate::iter::segment_cells;
use crate::pos::{CellPos, Location, PointM};
use crate::topology::{Extent, Topology};

use super::locate;

/// The furthest a ray is followed, in metres. A longer ray is cut to
/// this, so that a ray with nothing in its way still ends.
pub const RAY_MAX_M: Real = Real::of(10_000.0);

/// What a ray is, which decides what stops it (02 §5.4 flags).
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum RayBlocks {
    /// A line of sight. Stopped by a block or fixture flagged `opaque`.
    Sight,
    /// Something moving in a straight line. Stopped by a block or fixture
    /// that is not flagged `passable`.
    Movement,
    /// A projectile. Stopped by a block or fixture that closes the whole
    /// width of its cell; one that closes part of it is reported with its
    /// coverage and the ray goes on.
    Projectile,
}

/// What a ray meets and what stops it. More is added as more can be met:
/// build one from the constants and change fields from there.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
#[non_exhaustive]
pub struct RayFilter {
    /// What kind of ray it is.
    pub blocks: RayBlocks,
}

impl RayFilter {
    /// A line of sight.
    pub const SIGHT: RayFilter = RayFilter::new(RayBlocks::Sight);
    /// Something moving in a straight line.
    pub const MOVEMENT: RayFilter = RayFilter::new(RayBlocks::Movement);
    /// A projectile.
    pub const PROJECTILE: RayFilter = RayFilter::new(RayBlocks::Projectile);

    /// A filter for a kind of ray.
    pub const fn new(blocks: RayBlocks) -> RayFilter {
        RayFilter { blocks }
    }
}

/// The stretch of a ray inside one cell: where it enters and leaves, as
/// distances along the ray from its start, and how high above the base of
/// the cell it is at those two moments.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct RaySpan {
    /// Metres along the ray at which it enters the cell. Zero in the cell
    /// it starts in.
    pub enter_m: Real,
    /// Metres along the ray at which it leaves the cell, or ends in it.
    pub exit_m: Real,
    /// Its height above the base of the cell where it enters, 0 to 3 m.
    pub enter_alt_m: Real,
    /// Its height above the base of the cell where it leaves.
    pub exit_alt_m: Real,
}

impl RaySpan {
    /// The lowest the ray gets inside the cell, in metres above its base.
    pub fn lowest_m(&self) -> Real {
        self.enter_alt_m.min(self.exit_alt_m)
    }

    /// Where the ray first is lower than a height above the base of the
    /// cell, as metres along the ray, or `None` if it stays at or above
    /// that height all the way through. A ray exactly level with the top
    /// of something passes over it.
    pub fn dips_below(&self, height_m: Real) -> Option<Real> {
        if self.enter_alt_m < height_m {
            Some(self.enter_m)
        } else if self.exit_alt_m < height_m {
            let drop = self.enter_alt_m - self.exit_alt_m;
            let share = (self.enter_alt_m - height_m) / drop;
            Some(self.enter_m + (self.exit_m - self.enter_m) * share)
        } else {
            None
        }
    }
}

/// What a ray met.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum RayTarget {
    /// One of a cell's three elements.
    Element {
        /// Which of the three.
        layer: Layer,
        /// Its state when the ray was cast.
        state: ElementState,
    },
    /// An occupant, reported by a [`RayObstacles`].
    Occupant(EntityId),
}

/// One thing a ray met.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct RayHit {
    /// The cell it is in.
    pub loc: Location,
    /// What it is.
    pub target: RayTarget,
    /// Metres along the ray from its start.
    pub dist_m: Real,
    /// Where: metres in the frame the ray was given in. Across a wrapped
    /// edge the point carries on past it; `loc` is where that is.
    pub point: [Real; 3],
    /// The share of the ray's path the thing closes, 0 to 1: the chance
    /// that a thin projectile at this height strikes it. 1 for anything
    /// that stops the ray.
    pub coverage: Real,
    /// Whether the ray ends here.
    pub stops: bool,
}

impl RayHit {
    /// A meeting with an occupant, for a [`RayObstacles`] to report. The
    /// point is filled in by the raycast.
    pub fn occupant(
        loc: Location,
        id: EntityId,
        dist_m: Real,
        coverage: Real,
        stops: bool,
    ) -> RayHit {
        RayHit {
            loc,
            target: RayTarget::Occupant(id),
            dist_m,
            point: [Real::ZERO; 3],
            coverage,
            stops,
        }
    }
}

/// How a ray ended.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum RayEnd {
    /// It went its whole length.
    Reached,
    /// Something stopped it: the last of its hits.
    Stopped,
    /// It could not be followed further: it came to space that is not
    /// loaded, to an edge of the realm, or its grid does not exist.
    Refused(WorldError),
}

/// Everything a ray met, nearest first, and how it ended. A buffer: cast
/// into the same one again and it allocates nothing.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct RayHits {
    hits: Vec<RayHit>,
    end: RayEnd,
    length_m: Real,
}

impl Default for RayHits {
    fn default() -> RayHits {
        RayHits::new()
    }
}

impl RayHits {
    /// An empty result.
    pub fn new() -> RayHits {
        RayHits {
            hits: Vec::new(),
            end: RayEnd::Reached,
            length_m: Real::ZERO,
        }
    }

    /// Empties the result, keeping its storage.
    pub fn clear(&mut self) {
        self.hits.clear();
        self.end = RayEnd::Reached;
        self.length_m = Real::ZERO;
    }

    /// What the ray met, nearest first. Only the last can be a stop.
    pub fn hits(&self) -> &[RayHit] {
        &self.hits
    }

    /// How the ray ended.
    pub fn end(&self) -> &RayEnd {
        &self.end
    }

    /// How far the ray got, in metres.
    pub fn length_m(&self) -> Real {
        self.length_m
    }

    /// What stopped the ray, if anything did.
    pub fn stop(&self) -> Option<&RayHit> {
        self.hits.last().filter(|hit| hit.stops)
    }

    /// Whether the ray went its whole length.
    pub fn reached(&self) -> bool {
        self.end == RayEnd::Reached
    }
}

/// Things other than terrain that a ray can meet. The occupant index
/// answers through this, so that one ray meets walls and bodies in the
/// order they stand along it.
pub trait RayObstacles {
    /// Reports what the ray meets inside one cell, apart from the cell's
    /// own elements, by pushing onto `out` in any order. Called once for
    /// each cell the ray passes through, nearest first, with the stretch
    /// of the ray inside that cell. A hit that `stops` ends the ray.
    ///
    /// The cell the ray starts in is asked about like any other: leave
    /// out whoever cast the ray.
    fn in_cell(&mut self, loc: Location, span: &RaySpan, out: &mut Vec<RayHit>);
}

/// Nothing but terrain.
#[derive(Clone, Copy, PartialEq, Eq, Default, Debug)]
pub struct NoObstacles;

impl RayObstacles for NoObstacles {
    fn in_cell(&mut self, _loc: Location, _span: &RaySpan, _out: &mut Vec<RayHit>) {}
}

/// A ray made ready to walk: a start, a direction of length one and a
/// length.
struct Ray {
    origin: [Real; 3],
    dir: [Real; 3],
    max_m: Real,
}

impl Ray {
    /// `None` when the ray has no direction or no length.
    fn new(origin: [Real; 3], dir: [Real; 3], max_m: Real) -> Option<Ray> {
        let length = (dir[0] * dir[0] + dir[1] * dir[1] + dir[2] * dir[2]).sqrt();
        let usable = length > Real::ZERO
            && length.is_finite()
            && max_m > Real::ZERO
            && origin.iter().all(|part| part.is_finite());
        usable.then(|| Ray {
            origin,
            dir: dir.map(|part| part / length),
            max_m: max_m.min(RAY_MAX_M),
        })
    }

    fn at(&self, dist_m: Real) -> [Real; 3] {
        [
            self.origin[0] + self.dir[0] * dist_m,
            self.origin[1] + self.dir[1] * dist_m,
            self.origin[2] + self.dir[2] * dist_m,
        ]
    }

    /// The stretch of the ray inside a cell it passes through.
    fn span(&self, cell: CellPos) -> RaySpan {
        let sizes = [CELL_M, CELL_M, STOREY_M];
        let coords = [cell.x, cell.y, cell.z];
        let mut enter = Real::ZERO;
        let mut exit = self.max_m;
        for axis in 0..3 {
            let dir = self.dir[axis];
            if dir == Real::ZERO {
                continue;
            }
            let low = Real::from(coords[axis]) * sizes[axis];
            let high = low + sizes[axis];
            let to_low = (low - self.origin[axis]) / dir;
            let to_high = (high - self.origin[axis]) / dir;
            enter = enter.max(to_low.min(to_high));
            exit = exit.min(to_low.max(to_high));
        }
        // A cell the ray only touches has no length inside it.
        let exit = exit.max(enter);
        let base = Real::from(cell.z) * STOREY_M;
        let alt =
            |dist: Real| (self.origin[2] + self.dir[2] * dist - base).clamp(Real::ZERO, STOREY_M);
        RaySpan {
            enter_m: enter,
            exit_m: exit,
            enter_alt_m: alt(enter),
            exit_alt_m: alt(exit),
        }
    }
}

/// The share of a ray's path that an element closes at a height below
/// its top. An element's `coverage` is the share of the whole
/// cross-section of its cell that it closes, top to bottom (02 §5.3); a
/// ray is already known to be below its top, so the share that matters
/// is of the part of the cross-section up to that top.
fn lateral_coverage(info: &StateInfo) -> Real {
    (info.coverage * STOREY_M / info.height).min(Real::ONE)
}

/// Tests the block and the fixture of a cell against a ray and pushes
/// what it meets.
fn meet_elements(
    defs: &WorldDefs,
    column: &Column,
    loc: Location,
    flags: CellFlags,
    span: &RaySpan,
    filter: RayFilter,
    out: &mut Vec<RayHit>,
) {
    if filter.blocks == RayBlocks::Sight && !flags.contains(CellFlags::OPAQUE) {
        return;
    }
    let elements = [
        (Layer::Block, column.block(loc.cell).raw()),
        (Layer::Fixture, column.fixture(loc.cell).raw()),
    ];
    for (layer, state) in elements {
        if state.is_none() {
            continue;
        }
        let info = defs.info(layer, state);
        let Some(dist_m) = span.dips_below(info.height) else {
            continue;
        };
        let coverage = match filter.blocks {
            RayBlocks::Sight if info.flags.has(EngineFlag::Opaque) => Real::ONE,
            RayBlocks::Movement if !info.flags.has(EngineFlag::Passable) => Real::ONE,
            RayBlocks::Projectile => lateral_coverage(info),
            _ => continue,
        };
        if coverage <= Real::ZERO {
            continue;
        }
        out.push(RayHit {
            loc,
            target: RayTarget::Element { layer, state },
            dist_m,
            point: [Real::ZERO; 3],
            coverage,
            stops: coverage >= Real::ONE,
        });
    }
}

/// `b` moved by whole turns round the wrapped axes of a grid to the copy
/// of itself nearest to `a`.
fn nearest_image(topology: &Topology, a: [Real; 3], mut b: [Real; 3]) -> [Real; 3] {
    for (axis, extent) in [topology.x, topology.y].into_iter().enumerate() {
        if let Extent::Wrapped(size) = extent
            && size > 0
        {
            let round = Real::from(size) * CELL_M;
            let turns = ((b[axis] - a[axis]) / round).round();
            b[axis] -= turns * round;
        }
    }
    b
}

/// Whether nothing opaque stands on the straight line between two points
/// of one grid. See [`Cosmos::line_of_sight`].
fn sight_clear(
    view: &GridView<'_>,
    defs: &WorldDefs,
    a: [Real; 3],
    b: [Real; 3],
) -> Result<bool, WorldError> {
    // Always walked from the same end, so that the answer cannot depend
    // on which point was given first.
    let key = |p: [Real; 3]| (p[2], p[1], p[0]);
    let (a, b) = if key(a) <= key(b) { (a, b) } else { (b, a) };
    let b = nearest_image(view.topology(), a, b);
    let first = CellPos::containing(a);
    let last = CellPos::containing(b);
    let offset = [b[0] - a[0], b[1] - a[1], b[2] - a[2]];
    let length = (offset[0] * offset[0] + offset[1] * offset[1] + offset[2] * offset[2]).sqrt();
    if length > RAY_MAX_M {
        // Too far to follow, like a ray of that length.
        return Ok(false);
    }
    let ray = Ray::new(a, offset, length);
    let mut prev: Option<(i32, CellFlags)> = None;
    for cell in segment_cells(a, b) {
        let (column, norm) = locate(view, cell)?;
        let flags = column.flags(norm);
        if let Some((prev_z, prev_flags)) = prev
            && cell.z != prev_z
        {
            // Between storeys the line passes through the base of the
            // upper cell, where its floor is.
            let upper = if cell.z > prev_z { flags } else { prev_flags };
            if upper.contains(CellFlags::HAS_FLOOR) {
                return Ok(false);
            }
        }
        if flags.contains(CellFlags::OPAQUE)
            && cell != first
            && cell != last
            && let Some(ray) = &ray
        {
            let lowest = ray.span(cell).lowest_m();
            let block = defs.block_info(column.block(norm));
            let fixture = defs.fixture_info(column.fixture(norm));
            let hides =
                |info: &StateInfo| info.flags.has(EngineFlag::Opaque) && lowest < info.height;
            if hides(block) || hides(fixture) {
                return Ok(false);
            }
        }
        prev = Some((cell.z, flags));
    }
    Ok(true)
}

impl Cosmos {
    /// Follows a ray through the terrain and reports what it meets,
    /// nearest first, and how it ended (02 §22).
    ///
    /// The ray starts at `from`, runs along `dir` (of any length) and is
    /// followed for `max_m` metres, or [`RAY_MAX_M`] if that is less. It
    /// respects heights: a block or fixture fills its cell's footprint up
    /// to its height, and a ray that stays at or above that passes over.
    ///
    /// | Ray | Stopped by | Also reports |
    /// |-----|------------|--------------|
    /// | [`Sight`](RayBlocks::Sight) | an `opaque` block or fixture; a floor | — |
    /// | [`Movement`](RayBlocks::Movement) | a block or fixture that is not `passable`; a floor | — |
    /// | [`Projectile`](RayBlocks::Projectile) | a block or fixture that closes the whole width below its top; a floor | one that closes part of it, with its coverage |
    ///
    /// A floor is met where the ray passes between two storeys, and is
    /// the floor of the upper cell. The block and fixture of the cell the
    /// ray starts in never stop it: whoever casts it is taken to be clear
    /// of what shares its cell.
    ///
    /// A ray that comes to space that is not loaded, or to an edge of the
    /// realm, ends there with [`RayEnd::Refused`] and keeps what it met
    /// before. Nothing is loaded and nothing panics. On a wrapped axis
    /// the ray carries on round. A ray with no direction or no length,
    /// or whose start is not a finite point, goes nowhere and meets
    /// nothing.
    ///
    /// This is terrain only. [`raycast_into`](Cosmos::raycast_into) takes
    /// a [`RayObstacles`] for whatever else stands in the way, and a
    /// buffer to reuse.
    pub fn raycast(&self, from: PointM, dir: [Real; 3], max_m: Real, filter: RayFilter) -> RayHits {
        let mut hits = RayHits::new();
        self.raycast_into(from, dir, max_m, filter, &mut NoObstacles, &mut hits);
        hits
    }

    /// [`raycast`](Cosmos::raycast) into a buffer the caller keeps, with
    /// whatever `obstacles` reports met along with the terrain, in order
    /// of distance. Where two things are met at the same distance, what
    /// `obstacles` reported comes first, then the block, then the
    /// fixture.
    pub fn raycast_into(
        &self,
        from: PointM,
        dir: [Real; 3],
        max_m: Real,
        filter: RayFilter,
        obstacles: &mut dyn RayObstacles,
        out: &mut RayHits,
    ) {
        out.clear();
        let view = match self.grid(from.grid) {
            Ok(view) => view,
            Err(error) => {
                out.end = RayEnd::Refused(error);
                return;
            }
        };
        let Some(ray) = Ray::new(from.pos, dir, max_m) else {
            return;
        };
        let defs = self.defs();
        let first = CellPos::containing(ray.origin);
        let mut prev: Option<(i32, Location, &Column)> = None;
        out.length_m = ray.max_m;
        for cell in segment_cells(ray.origin, ray.at(ray.max_m)) {
            let span = ray.span(cell);
            let (column, norm) = match locate(&view, cell) {
                Ok(found) => found,
                Err(error) => {
                    out.end = RayEnd::Refused(error);
                    out.length_m = span.enter_m;
                    break;
                }
            };
            let loc = Location::new(from.grid, norm);
            let flags = column.flags(norm);
            if let Some((prev_z, prev_loc, prev_column)) = prev
                && cell.z != prev_z
            {
                // Between storeys the ray passes through the base of the
                // upper cell, where its floor is.
                let (upper, upper_column) = if cell.z > prev_z {
                    (loc, column)
                } else {
                    (prev_loc, prev_column)
                };
                let floor = upper_column.floor(upper.cell);
                if !floor.is_none() {
                    out.hits.push(RayHit {
                        loc: upper,
                        target: RayTarget::Element {
                            layer: Layer::Floor,
                            state: floor.raw(),
                        },
                        dist_m: span.enter_m,
                        point: [Real::ZERO; 3],
                        coverage: Real::ONE,
                        stops: true,
                    });
                    out.end = RayEnd::Stopped;
                    out.length_m = span.enter_m;
                    break;
                }
            }
            let start = out.hits.len();
            obstacles.in_cell(loc, &span, &mut out.hits);
            if cell != first {
                meet_elements(defs, column, loc, flags, &span, filter, &mut out.hits);
            }
            out.hits[start..].sort_by_key(|hit| hit.dist_m);
            if let Some(at) = out.hits[start..].iter().position(|hit| hit.stops) {
                out.hits.truncate(start + at + 1);
                out.end = RayEnd::Stopped;
                out.length_m = out.hits.last().map_or(span.enter_m, |hit| hit.dist_m);
                break;
            }
            prev = Some((cell.z, loc, column));
        }
        for hit in &mut out.hits {
            hit.point = ray.at(hit.dist_m);
        }
    }

    /// Whether the straight line between two points is clear to sight
    /// (02 §22): nothing `opaque` stands in a cell strictly between them
    /// taller than the line passes, and the line passes through no floor.
    ///
    /// What is in the two cells that hold the points does not block: one
    /// sees out of one's own cell, and one sees the thing one looks at,
    /// a wall included. The answer is the same whichever point is given
    /// first. On a wrapped axis the line takes the shorter way round.
    ///
    /// `Ok(false)` if the points are in different grids, or further
    /// apart than [`RAY_MAX_M`]. An error if the line comes to space that
    /// is not loaded, or to an edge of the realm, before anything blocks
    /// it: what is there is not known.
    ///
    /// Terrain only: occupants that block sight are tested with
    /// [`raycast_into`](Cosmos::raycast_into).
    pub fn line_of_sight(&self, a: PointM, b: PointM) -> Result<bool, WorldError> {
        let view = self.grid(a.grid)?;
        if a.grid != b.grid {
            return Ok(false);
        }
        sight_clear(&view, self.defs(), a.pos, b.pos)
    }

    /// [`line_of_sight`](Cosmos::line_of_sight) between a height in one
    /// cell and a height in another, each over the middle of its cell and
    /// given in millimetres above the cell's base.
    pub fn line_of_sight_between(
        &self,
        a: Location,
        a_alt_mm: u16,
        b: Location,
        b_alt_mm: u16,
    ) -> Result<bool, WorldError> {
        self.line_of_sight(a.point_at(a_alt_mm), b.point_at(b_alt_mm))
    }
}
