//! The sizes of the world's units, defined once (02 §3.1, 01 R9).
//!
//! Every size is a power of two, so that converting between units is a
//! shift and floors correctly for negative coordinates. Nothing outside
//! this module writes one of these sizes as a literal: `cargo xtask check`
//! looks for them.
//!
//! | Unit | Size |
//! |------|------|
//! | Cell | [`CELL_M`] × [`CELL_M`] × [`STOREY_M`] metres |
//! | Chunk | [`CHUNK_XY`] × [`CHUNK_XY`] × [`CHUNK_Z`] cells |
//! | Tract | [`TRACT_XY`] × [`TRACT_XY`] cells, all storeys |
//! | Region | [`REGION_TRACTS`] × [`REGION_TRACTS`] tracts |

use clash_core::Real;

/// Bits of a chunk's horizontal extent: a chunk is `1 << 5` cells on a side.
pub const CHUNK_XY_BITS: u32 = 5;

/// Bits of a chunk's vertical extent: a chunk is `1 << 3` storeys tall.
pub const CHUNK_Z_BITS: u32 = 3;

/// Bits of a region's extent in tracts: a region is `1 << 6` tracts on a
/// side.
pub const REGION_TRACT_BITS: u32 = 6;

/// The height of one storey, and so of one cell, in metres.
pub const STOREY_M: Real = Real::of(3.0);

/// The width and depth of one cell in metres.
pub const CELL_M: Real = Real::of(1.0);

/// The height of one storey in millimetres. An occupant's height above the
/// base of its cell runs from zero up to, but not including, this.
pub const STOREY_MM: u16 = 3000;

/// How many cell widths tall a storey is, as a whole number. Used where
/// distances are compared exactly, in integers.
pub const STOREY_CELLS: i64 = 3;

/// Cells along one horizontal side of a chunk.
pub const CHUNK_XY: i32 = 1 << CHUNK_XY_BITS;

/// Storeys in a chunk.
pub const CHUNK_Z: i32 = 1 << CHUNK_Z_BITS;

/// Mask for a cell's horizontal coordinate inside its chunk.
pub const CHUNK_XY_MASK: i32 = CHUNK_XY - 1;

/// Mask for a cell's storey inside its chunk.
pub const CHUNK_Z_MASK: i32 = CHUNK_Z - 1;

/// Cell columns in a chunk, and in a tract: one per horizontal position.
pub const CHUNK_AREA: usize = 1 << (2 * CHUNK_XY_BITS);

/// Cells in a chunk.
pub const CHUNK_CELLS: usize = 1 << (2 * CHUNK_XY_BITS + CHUNK_Z_BITS);

/// Bits of a tract's horizontal extent. A tract's footprint is a chunk's
/// footprint by design.
pub const TRACT_XY_BITS: u32 = CHUNK_XY_BITS;

/// Cells along one side of a tract.
pub const TRACT_XY: i32 = 1 << TRACT_XY_BITS;

/// Tracts along one side of a region.
pub const REGION_TRACTS: i32 = 1 << REGION_TRACT_BITS;

/// Mask for a tract's coordinate inside its region.
pub const REGION_TRACT_MASK: i32 = REGION_TRACTS - 1;

/// Tracts in a region.
pub const REGION_TRACT_COUNT: usize = 1 << (2 * REGION_TRACT_BITS);

/// Bits of a region's extent in cells.
pub const REGION_CELL_BITS: u32 = REGION_TRACT_BITS + TRACT_XY_BITS;

/// Cells along one side of a region.
pub const REGION_CELLS: i32 = 1 << REGION_CELL_BITS;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sizes_are_those_of_the_specification() {
        // 02 §3.1: chunk 32 × 32 × 8 cells, tract 32 × 32, region 64 × 64
        // tracts, which is 2048 m square.
        assert_eq!(CHUNK_XY, 32);
        assert_eq!(CHUNK_Z, 8);
        assert_eq!(CHUNK_AREA, 1024);
        assert_eq!(CHUNK_CELLS, 8192);
        assert_eq!(TRACT_XY, 32);
        assert_eq!(REGION_TRACTS, 64);
        assert_eq!(REGION_TRACT_COUNT, 4096);
        assert_eq!(REGION_CELLS, 2048);
        assert_eq!(STOREY_M, Real::of(3.0));
        assert_eq!(CELL_M, Real::of(1.0));
        assert_eq!(Real::from_i64(STOREY_CELLS) * CELL_M, STOREY_M);
        assert_eq!(Real::from(STOREY_MM), STOREY_M * Real::of(1000.0));
    }

    #[test]
    fn a_cell_index_fits_in_sixteen_bits() {
        assert!(CHUNK_CELLS <= usize::from(u16::MAX) + 1);
    }
}
