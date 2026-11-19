//! Positions at every scale, and the conversions between them (02 §3).
//!
//! | Type | Addresses |
//! |------|-----------|
//! | [`CellPos`] | one cell of a grid |
//! | [`CellIdx`] | one cell inside its chunk |
//! | [`ChunkPos`] | one chunk of a grid |
//! | [`TractPos`] | one tract: a column of chunks |
//! | [`RegionPos`] | one region of tracts |
//! | [`Location`] | a cell of a named grid: the engine's idea of "where" |
//! | [`PointM`] | a point in metres in a grid's frame |
//! | [`CellBox`] | a box of cells |
//!
//! Axes: `+x` east, `+y` south, `+z` up. Conversions between scales are
//! arithmetic shifts, which floor correctly for negative coordinates.
//! Arithmetic wraps on overflow and never panics.
//!
//! Positions order **storey by storey, row by row**: by `z`, then `y`,
//! then `x`. That is the order in which a chunk stores its cells, and the
//! order every query that returns cells uses.

use core::cmp::Ordering;
use core::fmt;
use core::ops::{Add, Sub};

use clash_core::{IVec2, IVec3, Real, StateHash, StateHasher, define_id};
use serde::{Deserialize, Serialize};

use crate::dims::{
    CELL_M, CHUNK_AREA, CHUNK_CELLS, CHUNK_XY_BITS, CHUNK_XY_MASK, CHUNK_Z_BITS, CHUNK_Z_MASK,
    REGION_TRACT_BITS, REGION_TRACT_MASK, STOREY_CELLS, STOREY_M, STOREY_MM, TRACT_XY_BITS,
};

define_id! {
    /// A grid of cells: a realm's base grid or a subgrid. Unique in the
    /// cosmos and never reused.
    pub struct GridId;
}

/// One cell of a grid: 1 m × 1 m and one storey tall.
///
/// ```
/// use clash_world::{CellPos, ChunkPos, TractPos};
///
/// let cell = CellPos::new(-1, 40, 9);
/// assert_eq!(cell.chunk(), ChunkPos::new(-1, 1, 1));
/// assert_eq!(cell.tract(), TractPos::new(-1, 1));
/// let (chunk, idx) = cell.split();
/// assert_eq!(chunk.cell(idx), cell);
/// ```
#[derive(Clone, Copy, PartialEq, Eq, Hash, Default, Debug, Serialize, Deserialize)]
pub struct CellPos {
    /// East is positive.
    pub x: i32,
    /// South is positive.
    pub y: i32,
    /// Up is positive. One step is one storey.
    pub z: i32,
}

impl CellPos {
    /// The cell at these coordinates.
    pub const fn new(x: i32, y: i32, z: i32) -> CellPos {
        CellPos { x, y, z }
    }

    /// The chunk this cell is in.
    pub const fn chunk(self) -> ChunkPos {
        ChunkPos {
            x: self.x >> CHUNK_XY_BITS,
            y: self.y >> CHUNK_XY_BITS,
            z: self.z >> CHUNK_Z_BITS,
        }
    }

    /// The cell's index inside its chunk.
    pub const fn idx(self) -> CellIdx {
        CellIdx::new(
            (self.x & CHUNK_XY_MASK) as u32,
            (self.y & CHUNK_XY_MASK) as u32,
            (self.z & CHUNK_Z_MASK) as u32,
        )
    }

    /// The cell's chunk and its index inside it.
    pub const fn split(self) -> (ChunkPos, CellIdx) {
        (self.chunk(), self.idx())
    }

    /// The tract this cell is in.
    pub const fn tract(self) -> TractPos {
        TractPos {
            x: self.x >> TRACT_XY_BITS,
            y: self.y >> TRACT_XY_BITS,
        }
    }

    /// The region this cell is in.
    pub const fn region(self) -> RegionPos {
        self.tract().region()
    }

    /// The index of this cell's column inside its tract: one of
    /// [`CHUNK_AREA`] horizontal positions, row by row.
    pub const fn column(self) -> usize {
        ((self.x & CHUNK_XY_MASK) | ((self.y & CHUNK_XY_MASK) << CHUNK_XY_BITS)) as usize
    }

    /// The cell an offset away. Wraps on overflow.
    pub const fn offset(self, by: IVec3) -> CellPos {
        CellPos {
            x: self.x.wrapping_add(by.x),
            y: self.y.wrapping_add(by.y),
            z: self.z.wrapping_add(by.z),
        }
    }

    /// The cell directly above.
    pub const fn above(self) -> CellPos {
        self.offset(IVec3::new(0, 0, 1))
    }

    /// The cell directly below.
    pub const fn below(self) -> CellPos {
        self.offset(IVec3::new(0, 0, -1))
    }

    /// The coordinates as a vector.
    pub const fn to_ivec3(self) -> IVec3 {
        IVec3::new(self.x, self.y, self.z)
    }

    /// The horizontal coordinates as a vector.
    pub const fn xy(self) -> IVec2 {
        IVec2::new(self.x, self.y)
    }

    /// The centre of the cell in metres, in its grid's frame.
    pub fn centre_m(self) -> [Real; 3] {
        let half = Real::of(0.5);
        [
            (Real::from(self.x) + half) * CELL_M,
            (Real::from(self.y) + half) * CELL_M,
            (Real::from(self.z) + half) * STOREY_M,
        ]
    }

    /// The cell that contains a point given in metres. A point exactly on
    /// a boundary belongs to the cell on its positive side.
    pub fn containing(point_m: [Real; 3]) -> CellPos {
        CellPos {
            x: (point_m[0] / CELL_M).floor().to_i32(),
            y: (point_m[1] / CELL_M).floor().to_i32(),
            z: (point_m[2] / STOREY_M).floor().to_i32(),
        }
    }
}

impl From<IVec3> for CellPos {
    fn from(v: IVec3) -> CellPos {
        CellPos::new(v.x, v.y, v.z)
    }
}

impl From<CellPos> for IVec3 {
    fn from(cell: CellPos) -> IVec3 {
        cell.to_ivec3()
    }
}

impl Add<IVec3> for CellPos {
    type Output = CellPos;
    fn add(self, by: IVec3) -> CellPos {
        self.offset(by)
    }
}

/// The plain offset from one cell to another, ignoring any wrap. Use
/// [`Topology::delta`](crate::Topology::delta) where a realm may wrap.
impl Sub for CellPos {
    type Output = IVec3;
    fn sub(self, from: CellPos) -> IVec3 {
        IVec3::new(
            self.x.wrapping_sub(from.x),
            self.y.wrapping_sub(from.y),
            self.z.wrapping_sub(from.z),
        )
    }
}

impl Ord for CellPos {
    fn cmp(&self, other: &CellPos) -> Ordering {
        (self.z, self.y, self.x).cmp(&(other.z, other.y, other.x))
    }
}

impl PartialOrd for CellPos {
    fn partial_cmp(&self, other: &CellPos) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl fmt::Display for CellPos {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "({}, {}, {})", self.x, self.y, self.z)
    }
}

impl StateHash for CellPos {
    fn state_hash(&self, hasher: &mut StateHasher) {
        self.x.state_hash(hasher);
        self.y.state_hash(hasher);
        self.z.state_hash(hasher);
    }
}

/// A cell's index inside its chunk: `lx | ly << 5 | lz << 10`.
///
/// Indices order storey by storey, row by row, like [`CellPos`].
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default, Debug)]
pub struct CellIdx(u16);

impl CellIdx {
    /// The index for local coordinates. Each is masked to its range.
    pub const fn new(lx: u32, ly: u32, lz: u32) -> CellIdx {
        let lx = lx & CHUNK_XY_MASK as u32;
        let ly = ly & CHUNK_XY_MASK as u32;
        let lz = lz & CHUNK_Z_MASK as u32;
        CellIdx((lx | (ly << CHUNK_XY_BITS) | (lz << (2 * CHUNK_XY_BITS))) as u16)
    }

    /// The index for a position in a chunk's cell array, or `None` if the
    /// position is past the end.
    pub const fn from_index(index: usize) -> Option<CellIdx> {
        if index < CHUNK_CELLS {
            Some(CellIdx(index as u16))
        } else {
            None
        }
    }

    /// The position in a chunk's cell array.
    #[inline]
    pub const fn index(self) -> usize {
        self.0 as usize
    }

    /// The packed number.
    pub const fn raw(self) -> u16 {
        self.0
    }

    /// The local `x`, from zero.
    pub const fn lx(self) -> i32 {
        self.0 as i32 & CHUNK_XY_MASK
    }

    /// The local `y`, from zero.
    pub const fn ly(self) -> i32 {
        (self.0 as i32 >> CHUNK_XY_BITS) & CHUNK_XY_MASK
    }

    /// The local storey, from zero.
    pub const fn lz(self) -> i32 {
        (self.0 as i32 >> (2 * CHUNK_XY_BITS)) & CHUNK_Z_MASK
    }

    /// The index of the cell's column inside the chunk's footprint.
    #[inline]
    pub const fn column(self) -> usize {
        self.0 as usize & (CHUNK_AREA - 1)
    }

    /// The index of the cell at a local storey of a column.
    pub const fn from_column(column: usize, lz: u32) -> CellIdx {
        let column = column & (CHUNK_AREA - 1);
        CellIdx((column as u32 | ((lz & CHUNK_Z_MASK as u32) << (2 * CHUNK_XY_BITS))) as u16)
    }

    /// Every index of a chunk, in order.
    pub fn all() -> impl Iterator<Item = CellIdx> {
        (0..CHUNK_CELLS).map(|index| CellIdx(index as u16))
    }
}

impl Serialize for CellIdx {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        self.0.serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for CellIdx {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<CellIdx, D::Error> {
        let raw = u16::deserialize(deserializer)?;
        CellIdx::from_index(usize::from(raw))
            .ok_or_else(|| serde::de::Error::custom("cell index is outside the chunk"))
    }
}

impl StateHash for CellIdx {
    fn state_hash(&self, hasher: &mut StateHasher) {
        hasher.write_u16(self.0);
    }
}

/// One chunk of a grid.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Default, Debug, Serialize, Deserialize)]
pub struct ChunkPos {
    /// Chunks east.
    pub x: i32,
    /// Chunks south.
    pub y: i32,
    /// Chunks up.
    pub z: i32,
}

impl ChunkPos {
    /// The chunk at these chunk coordinates.
    pub const fn new(x: i32, y: i32, z: i32) -> ChunkPos {
        ChunkPos { x, y, z }
    }

    /// The chunk's cell with the lowest coordinates.
    pub const fn origin(self) -> CellPos {
        CellPos {
            x: self.x.wrapping_shl(CHUNK_XY_BITS),
            y: self.y.wrapping_shl(CHUNK_XY_BITS),
            z: self.z.wrapping_shl(CHUNK_Z_BITS),
        }
    }

    /// The cell at an index of this chunk.
    pub const fn cell(self, idx: CellIdx) -> CellPos {
        let origin = self.origin();
        CellPos {
            x: origin.x | idx.lx(),
            y: origin.y | idx.ly(),
            z: origin.z | idx.lz(),
        }
    }

    /// The tract this chunk belongs to.
    pub const fn tract(self) -> TractPos {
        TractPos {
            x: self.x,
            y: self.y,
        }
    }
}

impl Ord for ChunkPos {
    fn cmp(&self, other: &ChunkPos) -> Ordering {
        (self.z, self.y, self.x).cmp(&(other.z, other.y, other.x))
    }
}

impl PartialOrd for ChunkPos {
    fn partial_cmp(&self, other: &ChunkPos) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl StateHash for ChunkPos {
    fn state_hash(&self, hasher: &mut StateHasher) {
        self.x.state_hash(hasher);
        self.y.state_hash(hasher);
        self.z.state_hash(hasher);
    }
}

/// One tract: a column of chunks over all storeys. The unit of
/// generation, loading, saving and coarse simulation.
///
/// Tracts order row by row: by `y`, then `x`.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Default, Debug, Serialize, Deserialize)]
pub struct TractPos {
    /// Tracts east.
    pub x: i32,
    /// Tracts south.
    pub y: i32,
}

impl TractPos {
    /// The tract at these tract coordinates.
    pub const fn new(x: i32, y: i32) -> TractPos {
        TractPos { x, y }
    }

    /// The region this tract is in.
    pub const fn region(self) -> RegionPos {
        RegionPos {
            x: self.x >> REGION_TRACT_BITS,
            y: self.y >> REGION_TRACT_BITS,
        }
    }

    /// The tract's position inside its region's table of tracts, row by
    /// row.
    pub const fn index_in_region(self) -> usize {
        ((self.x & REGION_TRACT_MASK) | ((self.y & REGION_TRACT_MASK) << REGION_TRACT_BITS))
            as usize
    }

    /// The tract's cell with the lowest horizontal coordinates, at a
    /// storey.
    pub const fn origin(self, z: i32) -> CellPos {
        CellPos {
            x: self.x.wrapping_shl(TRACT_XY_BITS),
            y: self.y.wrapping_shl(TRACT_XY_BITS),
            z,
        }
    }

    /// The chunk of this tract at a chunk height.
    pub const fn chunk(self, chunk_z: i32) -> ChunkPos {
        ChunkPos {
            x: self.x,
            y: self.y,
            z: chunk_z,
        }
    }

    /// The cell of a column of this tract at a storey.
    pub const fn cell(self, column: usize, z: i32) -> CellPos {
        let origin = self.origin(z);
        let column = column & (CHUNK_AREA - 1);
        CellPos {
            x: origin.x | (column as i32 & CHUNK_XY_MASK),
            y: origin.y | (column as i32 >> CHUNK_XY_BITS),
            z,
        }
    }

    /// The tract an offset away. Wraps on overflow.
    pub const fn offset(self, by: IVec2) -> TractPos {
        TractPos {
            x: self.x.wrapping_add(by.x),
            y: self.y.wrapping_add(by.y),
        }
    }
}

impl Ord for TractPos {
    fn cmp(&self, other: &TractPos) -> Ordering {
        (self.y, self.x).cmp(&(other.y, other.x))
    }
}

impl PartialOrd for TractPos {
    fn partial_cmp(&self, other: &TractPos) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl fmt::Display for TractPos {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "({}, {})", self.x, self.y)
    }
}

impl StateHash for TractPos {
    fn state_hash(&self, hasher: &mut StateHasher) {
        self.x.state_hash(hasher);
        self.y.state_hash(hasher);
    }
}

/// One region of tracts. The unit of survey and regional simulation.
///
/// Regions order row by row: by `y`, then `x`.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Default, Debug, Serialize, Deserialize)]
pub struct RegionPos {
    /// Regions east.
    pub x: i32,
    /// Regions south.
    pub y: i32,
}

impl RegionPos {
    /// The region at these region coordinates.
    pub const fn new(x: i32, y: i32) -> RegionPos {
        RegionPos { x, y }
    }

    /// The region's tract with the lowest coordinates.
    pub const fn origin(self) -> TractPos {
        TractPos {
            x: self.x.wrapping_shl(REGION_TRACT_BITS),
            y: self.y.wrapping_shl(REGION_TRACT_BITS),
        }
    }

    /// The tract at a position in this region's table of tracts.
    pub const fn tract(self, index: usize) -> TractPos {
        let origin = self.origin();
        TractPos {
            x: origin.x | (index as i32 & REGION_TRACT_MASK),
            y: origin.y | ((index as i32 >> REGION_TRACT_BITS) & REGION_TRACT_MASK),
        }
    }
}

impl Ord for RegionPos {
    fn cmp(&self, other: &RegionPos) -> Ordering {
        (self.y, self.x).cmp(&(other.y, other.x))
    }
}

impl PartialOrd for RegionPos {
    fn partial_cmp(&self, other: &RegionPos) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl StateHash for RegionPos {
    fn state_hash(&self, hasher: &mut StateHasher) {
        self.x.state_hash(hasher);
        self.y.state_hash(hasher);
    }
}

/// A cell of a named grid. Every position in the engine is a `Location`,
/// never bare coordinates: the same coordinates mean different places in
/// different grids (02 §3.2).
///
/// Locations order by grid, then by cell.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug, Serialize, Deserialize)]
pub struct Location {
    /// The grid.
    pub grid: GridId,
    /// The cell of that grid.
    pub cell: CellPos,
}

impl Location {
    /// A cell of a grid.
    pub const fn new(grid: GridId, cell: CellPos) -> Location {
        Location { grid, cell }
    }

    /// The location an offset away in the same grid.
    pub const fn offset(self, by: IVec3) -> Location {
        Location {
            grid: self.grid,
            cell: self.cell.offset(by),
        }
    }

    /// The centre of the cell, in metres.
    pub fn centre(self) -> PointM {
        PointM {
            grid: self.grid,
            pos: self.cell.centre_m(),
        }
    }

    /// The point at a height above the base of the cell, over the middle
    /// of its footprint.
    pub fn point_at(self, alt_mm: u16) -> PointM {
        let mut pos = self.cell.centre_m();
        let alt = Real::from(alt_mm.min(STOREY_MM - 1)) / Real::of(MM_PER_M);
        pos[2] = Real::from(self.cell.z) * STOREY_M + alt;
        PointM {
            grid: self.grid,
            pos,
        }
    }
}

impl fmt::Display for Location {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "grid {} {}", self.grid, self.cell)
    }
}

impl StateHash for Location {
    fn state_hash(&self, hasher: &mut StateHasher) {
        self.grid.state_hash(hasher);
        self.cell.state_hash(hasher);
    }
}

/// Millimetres in a metre.
const MM_PER_M: f64 = 1000.0;

/// A point in metres in a grid's frame: `x` east, `y` south, and
/// `z = storey × 3 + height above the storey's base` (02 §3.2).
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Serialize, Deserialize)]
pub struct PointM {
    /// The grid whose frame the point is in.
    pub grid: GridId,
    /// Metres east, south and up.
    pub pos: [Real; 3],
}

impl PointM {
    /// A point of a grid.
    pub const fn new(grid: GridId, pos: [Real; 3]) -> PointM {
        PointM { grid, pos }
    }

    /// The cell that contains the point.
    pub fn cell(self) -> CellPos {
        CellPos::containing(self.pos)
    }

    /// The location of the cell that contains the point.
    pub fn location(self) -> Location {
        Location::new(self.grid, self.cell())
    }

    /// The point's height above the base of its cell, in millimetres:
    /// zero up to, but not including, a storey.
    pub fn alt_mm(self) -> u16 {
        let base = Real::from(self.cell().z) * STOREY_M;
        let mm = ((self.pos[2] - base) * Real::of(MM_PER_M)).floor().to_i64();
        mm.clamp(0, i64::from(STOREY_MM) - 1) as u16
    }
}

impl StateHash for PointM {
    fn state_hash(&self, hasher: &mut StateHasher) {
        self.grid.state_hash(hasher);
        self.pos.state_hash(hasher);
    }
}

/// The physical length of an offset between cell centres, in metres, with
/// the vertical part scaled by the height of a storey (02 §3.3).
pub fn distance_m(offset: IVec3) -> Real {
    Real::from_i64(distance_sq_cells(offset)).sqrt() * CELL_M
}

/// The square of the physical length of an offset, in cell widths, as an
/// exact integer. A storey counts as [`STOREY_CELLS`] cell widths.
pub const fn distance_sq_cells(offset: IVec3) -> i64 {
    let x = offset.x as i64;
    let y = offset.y as i64;
    let z = (offset.z as i64).saturating_mul(STOREY_CELLS);
    x.saturating_mul(x)
        .saturating_add(y.saturating_mul(y))
        .saturating_add(z.saturating_mul(z))
}

/// A box of cells, with both corners included.
///
/// ```
/// use clash_world::{CellBox, CellPos};
///
/// let room = CellBox::new(CellPos::new(4, 0, 0), CellPos::new(0, 2, 0));
/// assert_eq!(room.min, CellPos::new(0, 0, 0));
/// assert_eq!(room.cells().count(), 15);
/// assert!(room.contains(CellPos::new(2, 1, 0)));
/// ```
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug, Serialize, Deserialize)]
pub struct CellBox {
    /// The corner with the lowest coordinates.
    pub min: CellPos,
    /// The corner with the highest coordinates. Part of the box.
    pub max: CellPos,
}

impl CellBox {
    /// The box spanned by two corners, given in any order.
    pub fn new(a: CellPos, b: CellPos) -> CellBox {
        CellBox {
            min: CellPos::new(a.x.min(b.x), a.y.min(b.y), a.z.min(b.z)),
            max: CellPos::new(a.x.max(b.x), a.y.max(b.y), a.z.max(b.z)),
        }
    }

    /// The box of one cell.
    pub const fn single(cell: CellPos) -> CellBox {
        CellBox {
            min: cell,
            max: cell,
        }
    }

    /// The box reaching `reach` cells from a centre on every axis.
    pub fn around(centre: CellPos, reach: IVec3) -> CellBox {
        let reach = IVec3::new(reach.x.abs(), reach.y.abs(), reach.z.abs());
        CellBox::new(
            CellPos::new(
                centre.x.saturating_sub(reach.x),
                centre.y.saturating_sub(reach.y),
                centre.z.saturating_sub(reach.z),
            ),
            CellPos::new(
                centre.x.saturating_add(reach.x),
                centre.y.saturating_add(reach.y),
                centre.z.saturating_add(reach.z),
            ),
        )
    }

    /// Whether a cell is inside the box.
    pub const fn contains(self, cell: CellPos) -> bool {
        cell.x >= self.min.x
            && cell.x <= self.max.x
            && cell.y >= self.min.y
            && cell.y <= self.max.y
            && cell.z >= self.min.z
            && cell.z <= self.max.z
    }

    /// The part of this box that is also inside another, if any.
    pub fn intersection(self, other: CellBox) -> Option<CellBox> {
        let min = CellPos::new(
            self.min.x.max(other.min.x),
            self.min.y.max(other.min.y),
            self.min.z.max(other.min.z),
        );
        let max = CellPos::new(
            self.max.x.min(other.max.x),
            self.max.y.min(other.max.y),
            self.max.z.min(other.max.z),
        );
        (min.x <= max.x && min.y <= max.y && min.z <= max.z).then_some(CellBox { min, max })
    }

    /// The box moved by an offset.
    pub const fn offset(self, by: IVec3) -> CellBox {
        CellBox {
            min: self.min.offset(by),
            max: self.max.offset(by),
        }
    }

    /// How many cells the box holds.
    pub const fn volume(self) -> u64 {
        let dx = (self.max.x as i64 - self.min.x as i64 + 1) as u64;
        let dy = (self.max.y as i64 - self.min.y as i64 + 1) as u64;
        let dz = (self.max.z as i64 - self.min.z as i64 + 1) as u64;
        dx.saturating_mul(dy).saturating_mul(dz)
    }

    /// Every cell of the box: storey by storey, row by row.
    pub fn cells(self) -> impl Iterator<Item = CellPos> {
        let CellBox { min, max } = self;
        (min.z..=max.z).flat_map(move |z| {
            (min.y..=max.y).flat_map(move |y| (min.x..=max.x).map(move |x| CellPos::new(x, y, z)))
        })
    }

    /// Every tract the box touches, row by row.
    pub fn tracts(self) -> impl Iterator<Item = TractPos> {
        let min = self.min.tract();
        let max = self.max.tract();
        (min.y..=max.y).flat_map(move |y| (min.x..=max.x).map(move |x| TractPos::new(x, y)))
    }
}

impl StateHash for CellBox {
    fn state_hash(&self, hasher: &mut StateHasher) {
        self.min.state_hash(hasher);
        self.max.state_hash(hasher);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dims::{CHUNK_XY, CHUNK_Z, REGION_TRACTS};

    #[test]
    fn a_location_is_sixteen_bytes() {
        assert_eq!(core::mem::size_of::<Location>(), 16);
    }

    #[test]
    fn cell_index_packs_as_specified() {
        let idx = CellIdx::new(3, 2, 1);
        assert_eq!(
            idx.raw(),
            3 | (2 << CHUNK_XY_BITS) | (1 << (2 * CHUNK_XY_BITS))
        );
        assert_eq!((idx.lx(), idx.ly(), idx.lz()), (3, 2, 1));
        assert_eq!(idx.column(), 3 | (2 << CHUNK_XY_BITS));
        assert_eq!(CellIdx::from_column(idx.column(), 1), idx);
        assert_eq!(CellIdx::from_index(CHUNK_CELLS), None);
        assert_eq!(CellIdx::all().count(), CHUNK_CELLS);
    }

    #[test]
    fn negative_coordinates_floor() {
        let cell = CellPos::new(-1, -CHUNK_XY, -1);
        assert_eq!(cell.chunk(), ChunkPos::new(-1, -1, -1));
        assert_eq!(cell.idx(), CellIdx::new(31, 0, 7));
        assert_eq!(cell.tract(), TractPos::new(-1, -1));
        assert_eq!(
            CellPos::new(-CHUNK_XY * REGION_TRACTS - 1, 0, 0).region(),
            RegionPos::new(-2, 0)
        );
        assert_eq!(
            ChunkPos::new(-1, -1, -1).origin(),
            CellPos::new(-CHUNK_XY, -CHUNK_XY, -CHUNK_Z)
        );
    }

    #[test]
    fn cells_order_storey_by_storey_then_row_by_row() {
        let mut cells = vec![
            CellPos::new(1, 0, 1),
            CellPos::new(0, 1, 0),
            CellPos::new(5, 0, 0),
            CellPos::new(0, 0, 1),
        ];
        cells.sort();
        assert_eq!(
            cells,
            [
                CellPos::new(5, 0, 0),
                CellPos::new(0, 1, 0),
                CellPos::new(0, 0, 1),
                CellPos::new(1, 0, 1),
            ]
        );
        let listed: Vec<CellPos> = CellBox::new(CellPos::new(0, 0, 0), CellPos::new(2, 2, 1))
            .cells()
            .collect();
        let mut sorted = listed.clone();
        sorted.sort();
        assert_eq!(listed, sorted);
    }

    #[test]
    fn points_and_cells_agree() {
        let grid = GridId::new(0);
        let loc = Location::new(grid, CellPos::new(-3, 7, -2));
        assert_eq!(loc.centre().location(), loc);
        let point = loc.point_at(1500);
        assert_eq!(point.location(), loc);
        assert_eq!(point.alt_mm(), 1500);
        assert_eq!(point.pos[2], Real::of(-4.5));
        // A point on a boundary belongs to the cell on its positive side.
        let edge = PointM::new(grid, [Real::of(2.0), Real::of(-1.0), Real::of(3.0)]);
        assert_eq!(edge.cell(), CellPos::new(2, -1, 1));
        assert_eq!(edge.alt_mm(), 0);
    }

    #[test]
    fn distance_scales_height_by_the_storey() {
        assert_eq!(distance_m(IVec3::new(3, 4, 0)), Real::of(5.0));
        assert_eq!(distance_m(IVec3::new(0, 0, 2)), Real::of(6.0));
        assert_eq!(distance_sq_cells(IVec3::new(1, 1, 1)), 11);
    }

    #[test]
    fn boxes_intersect_and_count() {
        let a = CellBox::new(CellPos::new(0, 0, 0), CellPos::new(9, 9, 0));
        let b = CellBox::new(CellPos::new(5, 5, 0), CellPos::new(20, 20, 3));
        assert_eq!(
            a.intersection(b),
            Some(CellBox::new(CellPos::new(5, 5, 0), CellPos::new(9, 9, 0)))
        );
        assert_eq!(a.intersection(b.offset(IVec3::new(50, 0, 0))), None);
        assert_eq!(a.volume(), 100);
        assert_eq!(b.tracts().count(), 1);
        assert_eq!(
            CellBox::around(CellPos::new(0, 0, 0), IVec3::new(40, 0, 0))
                .tracts()
                .collect::<Vec<_>>(),
            [
                TractPos::new(-2, 0),
                TractPos::new(-1, 0),
                TractPos::new(0, 0),
                TractPos::new(1, 0)
            ]
        );
    }
}
