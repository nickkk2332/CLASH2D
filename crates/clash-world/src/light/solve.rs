//! Working out the static light of one chunk from the shapes of the
//! chunk and its neighbours: how far each cell is from the open sky, and
//! what the fixed sources within reach give it.

use clash_core::{IVec3, Real};

use super::cache::{
    BLOCK_CENTRE, BLOCK_CHUNKS, S_BLOCKS, S_FLOOR, S_SKY, Shape, ShapeCells, Steps, block_offset,
};
use super::{LIGHT_EYE_MM, SKY_DARK, STEP_DIAGONAL, STEP_SIDE, STEP_STOREY};
use crate::cosmos::Cosmos;
use crate::dims::{CELL_M, CHUNK_AREA, CHUNK_CELLS, CHUNK_XY, CHUNK_Z, STOREY_CELLS};
use crate::pos::{CellIdx, ChunkPos, GridId, Location, distance_sq_cells};
use crate::query::FovSet;

/// Buffers kept between one working-out and the next.
#[derive(Debug, Default)]
pub(super) struct Scratch {
    /// The shape bits of the cells within reach of the chunk.
    bits: Vec<u8>,
    /// Their distance from the open sky so far.
    steps: Vec<u8>,
    /// Cells waiting to pass light on, by their distance.
    waiting: Vec<Vec<u32>>,
    /// What a source sees.
    fov: FovSet,
}

/// The ten ways light steps from a cell to the next, each with its cost
/// in spread steps: four sideways, four diagonal, down and up.
const WAYS: [(i32, i32, i32, u8); 10] = [
    (0, -1, 0, STEP_SIDE),
    (1, 0, 0, STEP_SIDE),
    (0, 1, 0, STEP_SIDE),
    (-1, 0, 0, STEP_SIDE),
    (1, -1, 0, STEP_DIAGONAL),
    (1, 1, 0, STEP_DIAGONAL),
    (-1, 1, 0, STEP_DIAGONAL),
    (-1, -1, 0, STEP_DIAGONAL),
    (0, 0, -1, STEP_STOREY),
    (0, 0, 1, STEP_STOREY),
];

/// The cells within reach of a chunk, as a box with the chunk in its
/// middle.
#[derive(Clone, Copy)]
struct Region {
    /// Cells of margin on each side.
    reach: i32,
    /// Storeys of margin below and above.
    storeys: i32,
    /// Cells along `x` and along `y`.
    side: i32,
    /// Storeys.
    height: i32,
}

impl Region {
    fn new(reach: i32) -> Region {
        let storeys = reach / STOREY_CELLS as i32;
        Region {
            reach,
            storeys,
            side: CHUNK_XY + 2 * reach,
            height: CHUNK_Z + 2 * storeys,
        }
    }

    fn len(&self) -> usize {
        (self.side * self.side * self.height) as usize
    }

    #[inline]
    fn index(&self, x: i32, y: i32, z: i32) -> usize {
        ((z * self.side + y) * self.side + x) as usize
    }

    #[inline]
    fn holds(&self, x: i32, y: i32, z: i32) -> bool {
        (0..self.side).contains(&x) && (0..self.side).contains(&y) && (0..self.height).contains(&z)
    }

    /// Whether a cell of the region is a cell of the chunk itself.
    #[inline]
    fn in_chunk(&self, x: i32, y: i32, z: i32) -> bool {
        (self.reach..self.reach + CHUNK_XY).contains(&x)
            && (self.reach..self.reach + CHUNK_XY).contains(&y)
            && (self.storeys..self.storeys + CHUNK_Z).contains(&z)
    }
}

/// Whether light steps from the cell at `(x, y, z)` by one of the ten
/// ways. Sideways it always does. Diagonally it needs both cells beside
/// the corner clear, so that it never slips between two walls that touch
/// at their corners. Between storeys it needs the upper cell to have no
/// floor.
#[inline]
fn steps_through(
    region: &Region,
    bits: &[u8],
    (x, y, z): (i32, i32, i32),
    (dx, dy, dz): (i32, i32, i32),
) -> bool {
    if dz < 0 {
        bits[region.index(x, y, z)] & S_FLOOR == 0
    } else if dz > 0 {
        bits[region.index(x, y, z + 1)] & S_FLOOR == 0
    } else if dx != 0 && dy != 0 {
        bits[region.index(x + dx, y, z)] & S_BLOCKS == 0
            && bits[region.index(x, y + dy, z)] & S_BLOCKS == 0
    } else {
        true
    }
}

/// How far each cell of a chunk is from the open sky, in spread steps:
/// nothing for a cell open to the sky, [`SKY_DARK`] for one that sky
/// light does not reach within `reach` cells.
///
/// Light starts in every cell open to the sky and steps from cell to
/// cell by the ten ways, through cells that do not stop it. A cell that
/// stops light is lit by its neighbours and passes nothing on. A chunk
/// that is not loaded stops light.
pub(super) fn sky_steps(
    shapes: &[Option<&Shape>; BLOCK_CHUNKS],
    reach: i32,
    scratch: &mut Scratch,
) -> Steps {
    let Some(own) = shapes[BLOCK_CENTRE].filter(|shape| shape.is_present()) else {
        return Steps::All(SKY_DARK);
    };
    // Answers that need no working out.
    if let ShapeCells::All(bits) = own.cells
        && bits & S_SKY != 0
    {
        return Steps::All(0);
    }
    let closed = |shape: &Option<&Shape>| match shape {
        Some(shape) => {
            matches!(shape.cells, ShapeCells::All(bits) if bits & S_SKY == 0 && bits & S_BLOCKS != 0)
        }
        None => true,
    };
    if shapes.iter().all(closed) {
        return Steps::All(SKY_DARK);
    }

    let region = Region::new(reach.max(0));
    let Scratch {
        bits,
        steps,
        waiting,
        ..
    } = scratch;
    // What is not loaded, or lies beyond an edge, stops light.
    bits.clear();
    bits.resize(region.len(), S_BLOCKS);
    steps.clear();
    steps.resize(region.len(), SKY_DARK);
    // Which chunks of the block hold a cell that is not open to the sky
    // and either passes light on or belongs to the chunk itself: only
    // those need looking through.
    let mut worth = [false; BLOCK_CHUNKS];
    for (place, shape) in shapes.iter().enumerate() {
        let Some(shape) = shape else { continue };
        let offset = block_offset(place);
        let low = [
            region.reach + offset.x * CHUNK_XY,
            region.reach + offset.y * CHUNK_XY,
            region.storeys + offset.z * CHUNK_Z,
        ];
        let from = [low[0].max(0), low[1].max(0), low[2].max(0)];
        let to = [
            (low[0] + CHUNK_XY).min(region.side),
            (low[1] + CHUNK_XY).min(region.side),
            (low[2] + CHUNK_Z).min(region.height),
        ];
        if (0..3).any(|axis| from[axis] >= to[axis]) {
            continue;
        }
        worth[place] = match shape.cells {
            ShapeCells::All(all) => all & S_SKY == 0 && (all & S_BLOCKS == 0 || place == BLOCK_CENTRE),
            ShapeCells::Each(_) => true,
        };
        let run = (to[0] - from[0]) as usize;
        for z in from[2]..to[2] {
            for y in from[1]..to[1] {
                let at = region.index(from[0], y, z);
                match &shape.cells {
                    ShapeCells::All(all) => bits[at..at + run].fill(*all),
                    ShapeCells::Each(cells) => {
                        let source = (((z - low[2]) * CHUNK_XY + (y - low[1])) * CHUNK_XY
                            + (from[0] - low[0])) as usize;
                        bits[at..at + run].copy_from_slice(&cells[source..source + run]);
                    }
                }
            }
        }
        // Cells open to the sky are as near to it as can be.
        if !matches!(shape.cells, ShapeCells::All(all) if all & S_SKY == 0) {
            for z in from[2]..to[2] {
                for y in from[1]..to[1] {
                    let at = region.index(from[0], y, z);
                    for cell in at..at + run {
                        if bits[cell] & S_SKY != 0 {
                            steps[cell] = 0;
                        }
                    }
                }
            }
        }
    }

    let furthest = (reach.max(0) as usize * usize::from(STEP_SIDE)).min(usize::from(SKY_DARK) - 1);
    waiting.iter_mut().for_each(Vec::clear);
    waiting.resize_with(furthest + 1, Vec::new);

    // The first step: every cell that is not open to the sky takes the
    // cheapest step from a neighbour that is.
    for (place, _) in worth.iter().enumerate().filter(|(_, worth)| **worth) {
        let offset = block_offset(place);
        let low = [
            region.reach + offset.x * CHUNK_XY,
            region.reach + offset.y * CHUNK_XY,
            region.storeys + offset.z * CHUNK_Z,
        ];
        for z in low[2].max(0)..(low[2] + CHUNK_Z).min(region.height) {
            for y in low[1].max(0)..(low[1] + CHUNK_XY).min(region.side) {
                for x in low[0].max(0)..(low[0] + CHUNK_XY).min(region.side) {
                    let at = region.index(x, y, z);
                    let here = bits[at];
                    if here & S_SKY != 0
                        || (here & S_BLOCKS != 0 && place != BLOCK_CENTRE)
                    {
                        continue;
                    }
                    let mut best = SKY_DARK;
                    for (dx, dy, dz, cost) in WAYS {
                        // The neighbour the light would come from.
                        let (fx, fy, fz) = (x - dx, y - dy, z - dz);
                        if cost >= best || !region.holds(fx, fy, fz) {
                            continue;
                        }
                        let from = bits[region.index(fx, fy, fz)];
                        if from & S_SKY != 0
                            && from & S_BLOCKS == 0
                            && steps_through(&region, bits, (fx, fy, fz), (dx, dy, dz))
                        {
                            best = cost;
                        }
                    }
                    if usize::from(best) <= furthest {
                        steps[at] = best;
                        waiting[usize::from(best)].push(at as u32);
                    }
                }
            }
        }
    }

    // Then outward, nearest first.
    for near in 0..=furthest {
        let cells = core::mem::take(&mut waiting[near]);
        for &cell in &cells {
            let at = cell as usize;
            if usize::from(steps[at]) != near || bits[at] & S_BLOCKS != 0 {
                continue;
            }
            let x = at as i32 % region.side;
            let y = (at as i32 / region.side) % region.side;
            let z = at as i32 / (region.side * region.side);
            for (dx, dy, dz, cost) in WAYS {
                let far = near + usize::from(cost);
                let (tx, ty, tz) = (x + dx, y + dy, z + dz);
                if far > furthest || !region.holds(tx, ty, tz) {
                    continue;
                }
                let to = region.index(tx, ty, tz);
                if usize::from(steps[to]) <= far
                    || (bits[to] & S_BLOCKS != 0 && !region.in_chunk(tx, ty, tz))
                    || !steps_through(&region, bits, (x, y, z), (dx, dy, dz))
                {
                    continue;
                }
                steps[to] = far as u8;
                waiting[far].push(to as u32);
            }
        }
        waiting[near] = cells;
    }

    // The chunk's own cells, out of the middle of the region.
    let mut out = Box::new([SKY_DARK; CHUNK_CELLS]);
    for lz in 0..CHUNK_Z {
        for ly in 0..CHUNK_XY {
            let from = region.index(region.reach, region.reach + ly, region.storeys + lz);
            let to = ((lz * CHUNK_XY + ly) * CHUNK_XY) as usize;
            out[to..to + CHUNK_XY as usize]
                .copy_from_slice(&steps[from..from + CHUNK_XY as usize]);
        }
    }
    let first = out[0];
    if out.iter().all(|steps| *steps == first) {
        Steps::All(first)
    } else {
        Steps::Each(out)
    }
}

/// What the fixed sources of a block give the cells of its middle chunk,
/// storey by storey: for each source, in the order of the block's
/// chunks and then of cells, `intensity / (1 + d²)` to every cell of the
/// chunk that the source sees within its radius. A source sees what an
/// eye in its cell would, at the height light travels at.
pub(super) fn fixed_light(
    cosmos: &Cosmos,
    grid: GridId,
    chunks: &[Option<ChunkPos>; BLOCK_CHUNKS],
    shapes: &[Option<&Shape>; BLOCK_CHUNKS],
    scratch: &mut Scratch,
) -> [Option<Box<[Real; CHUNK_AREA]>>; CHUNK_Z as usize] {
    let mut fixed: [Option<Box<[Real; CHUNK_AREA]>>; CHUNK_Z as usize] = Default::default();
    for place in 0..BLOCK_CHUNKS {
        let (Some(chunk), Some(shape)) = (chunks[place], shapes[place]) else {
            continue;
        };
        let offset = block_offset(place);
        for (idx, source) in &shape.sources {
            // The source's cell, counted from the middle chunk's corner.
            let at = IVec3::new(
                offset.x * CHUNK_XY + idx.lx(),
                offset.y * CHUNK_XY + idx.ly(),
                offset.z * CHUNK_Z + idx.lz(),
            );
            let reach = source.reach_cells();
            let storeys = reach / STOREY_CELLS as i32;
            let low = IVec3::new(
                (at.x - reach).max(0),
                (at.y - reach).max(0),
                (at.z - storeys).max(0),
            );
            let high = IVec3::new(
                (at.x + reach).min(CHUNK_XY - 1),
                (at.y + reach).min(CHUNK_XY - 1),
                (at.z + storeys).min(CHUNK_Z - 1),
            );
            if low.x > high.x || low.y > high.y || low.z > high.z {
                continue;
            }
            let origin = Location::new(grid, chunk.cell(*idx));
            if cosmos
                .field_of_view(origin, LIGHT_EYE_MM, source.radius, &mut scratch.fov)
                .is_err()
            {
                continue;
            }
            for z in low.z..=high.z {
                for y in low.y..=high.y {
                    for x in low.x..=high.x {
                        let away = IVec3::new(x - at.x, y - at.y, z - at.z);
                        if !scratch.fov.contains_offset(away) {
                            continue;
                        }
                        let distance_sq = Real::from_i64(distance_sq_cells(away)) * CELL_M * CELL_M;
                        let storey = fixed[z as usize]
                            .get_or_insert_with(|| Box::new([Real::ZERO; CHUNK_AREA]));
                        storey[CellIdx::new(x as u32, y as u32, 0).index()] +=
                            source.at_distance_sq(distance_sq);
                    }
                }
            }
        }
    }
    fixed
}
