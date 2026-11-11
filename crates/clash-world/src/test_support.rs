//! Support for tests that need a world to stand in.
//!
//! **For tests and tools only.** The functions here put columns of cells
//! into a cosmos directly, without the loading procedure of 02 §9.4, which
//! package W8 builds. The simulation must never call them: once tracts are
//! loaded through tickets they are how a test sets a scene and nothing
//! more.
//!
//! Cells already in the world still change only through `WorldEdit`.
//! What these functions add is whole columns, built beforehand with a
//! [`ColumnBuilder`].
//!
//! A test builds its definitions, creates a realm with
//! [`Cosmos::create_realm`], and then loads ground into its base grid:
//!
//! ```
//! use std::sync::Arc;
//! use clash_world::test_support::{flat_column, load_column};
//! use clash_world::{
//!     BlockState, Cosmos, FloorState, GridId, RawWorldDefs, TractPos, WorldDefs, WorldError,
//! };
//!
//! // No definitions and no realm here, to keep the example short: the
//! // column is empty and there is no grid to put it in.
//! let defs = Arc::new(WorldDefs::build(RawWorldDefs::default()).unwrap());
//! let mut cosmos = Cosmos::new(Arc::clone(&defs));
//! let column = flat_column(&defs, BlockState::NONE, FloorState::NONE, 0);
//! let nowhere = GridId::new(0);
//! assert_eq!(
//!     load_column(&mut cosmos, nowhere, TractPos::new(0, 0), column).err(),
//!     Some(WorldError::UnknownGrid(nowhere))
//! );
//! ```

use crate::cosmos::Cosmos;
use crate::defs::{BlockState, FloorState, WorldDefs};
use crate::error::WorldError;
use crate::grid::{Column, ColumnBuilder};
use crate::pos::{GridId, TractPos};

/// Makes a tract of a grid loaded, holding the cells of `column`. Returns
/// the column that was loaded there before, if any. Whoever stood in a
/// column that is replaced is taken out of the occupant index, without
/// events.
pub fn load_column(
    cosmos: &mut Cosmos,
    grid: GridId,
    tract: TractPos,
    column: Column,
) -> Result<Option<Column>, WorldError> {
    cosmos.insert_column(grid, tract, column)
}

/// Unloads a tract of a grid and returns its cells. Whoever stood in it
/// is taken out of the occupant index, without events: every occupant is
/// in loaded space.
pub fn unload_column(cosmos: &mut Cosmos, grid: GridId, tract: TractPos) -> Option<Column> {
    cosmos.remove_column(grid, tract)
}

/// A column of flat ground: `ground` in every storey below `surface_z`,
/// nothing from there up, and `floor` laid over the whole of storey
/// `surface_z` unless it is none.
pub fn flat_column(
    defs: &WorldDefs,
    ground: BlockState,
    floor: FloorState,
    surface_z: i32,
) -> Column {
    let mut builder = ColumnBuilder::new(defs, ground, BlockState::NONE, surface_z);
    if !floor.is_none() {
        builder.fill_floor(surface_z, floor);
    }
    builder.build()
}

/// Loads flat ground (see [`flat_column`]) into each of the given tracts
/// of a grid.
pub fn load_flat(
    cosmos: &mut Cosmos,
    grid: GridId,
    tracts: impl IntoIterator<Item = TractPos>,
    ground: BlockState,
    floor: FloorState,
    surface_z: i32,
) -> Result<(), WorldError> {
    let defs = std::sync::Arc::clone(cosmos.defs());
    for tract in tracts {
        let column = flat_column(&defs, ground, floor, surface_z);
        cosmos.insert_column(grid, tract, column)?;
    }
    Ok(())
}
