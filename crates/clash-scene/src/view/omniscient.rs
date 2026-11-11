//! The omniscient mode: filling a view model from truth (12 §5).
//!
//! **For debugging and for tools.** Everything else that fills a
//! [`ViewModel`] reports what a character perceived. [`fill`] reads the
//! world itself and records what is really there, whoever could or could
//! not see it. The world viewer uses it, and so does a screen in the
//! `debug` context when the application lets it.
//!
//! This is the only module of the crate that names the world's read API
//! (`Cosmos`): the view model, the composer and the painters cannot reach
//! the world, and an ordinary screen has no route here. A test of the
//! crate reads its source to keep that so.

use clash_world::{CellBox, Cosmos, GridId, Location, WorldError};

use super::model::{CellContents, DecalSeen, LIGHT_FULL, ViewModel};

/// What one [`fill`] did.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct Filled {
    /// Cells recorded as seen, with what they really hold.
    pub seen: u64,
    /// Cells of the box whose tract is not loaded: forgotten, since
    /// truth has nothing to say of them.
    pub not_loaded: u64,
    /// Cells of the box beyond an edge of the grid: skipped.
    pub outside: u64,
}

/// Records every cell of a box of a grid as seen now, holding what the
/// world really holds there, and tells the view model the grid's shape.
///
/// What was known of those cells before is replaced, so calling this
/// again after the world has changed shows the change. Cells outside the
/// box are left as they were.
///
/// Recorded per cell: the three elements, the most marked decal, and
/// whether the cell is under open sky. Not recorded, because the world
/// cannot answer yet: fields (which read as zero until package W16),
/// occupants and items (package W7), and light (package S1), which is
/// taken to be full.
///
/// Fails only if the grid does not exist.
pub fn fill(
    model: &mut ViewModel,
    cosmos: &Cosmos,
    grid: GridId,
    cells: CellBox,
) -> Result<Filled, WorldError> {
    let view = cosmos.grid(grid)?;
    model.set_grid_shape(grid, *view.topology());
    let mut filled = Filled::default();
    for cell in cells.cells() {
        let loc = Location::new(grid, cell);
        let states = match view.cell(cell) {
            Ok(states) => states,
            Err(WorldError::NotLoaded(at)) => {
                model.forget_cell(at);
                filled.not_loaded += 1;
                continue;
            }
            Err(_) => {
                filled.outside += 1;
                continue;
            }
        };
        // Reading the cell succeeded, so these reads do too.
        let decal = view
            .decals(cell)
            .unwrap_or_default()
            .iter()
            .max_by_key(|decal| decal.amount)
            .map(|decal| DecalSeen {
                kind: decal.kind,
                amount: decal.amount,
            });
        let open_sky = view.under_open_sky(cell).unwrap_or(false);
        // The cell is recorded where it really is: a position beyond a
        // wrapped edge is the cell it wraps to.
        let at = view
            .normalize(cell)
            .map_or(loc, |cell| Location::new(grid, cell));
        model.observe_cell(
            at,
            CellContents {
                floor: states.floor,
                block: states.block,
                fixture: states.fixture,
                field: None,
                decal,
                pile: false,
                open_sky,
                light: LIGHT_FULL,
            },
        );
        filled.seen += 1;
    }
    Ok(filled)
}
