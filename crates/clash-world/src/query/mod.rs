//! The geometric questions the world answers (02 §22). Perception, AI
//! and the movement rules, which interpret the answers, are built on
//! them elsewhere.
//!
//! | Need | Use |
//! |------|-----|
//! | What a ray meets, where and how far along | [`Cosmos::raycast`], [`Cosmos::raycast_into`] with a [`RayFilter`]; the answer is a [`RayHits`] |
//! | Whether one point can be seen from another | [`Cosmos::line_of_sight`], [`Cosmos::line_of_sight_between`] |
//! | Every cell a viewer sees | [`Cosmos::field_of_view`] into a [`FovSet`] kept between calls |
//! | The geometry round a cell, for movement rules | [`Cosmos::neighbours`], a [`NeighbourGeometry`] |
//! | What holds up someone in a cell, if anything | [`Cosmos::footing`], a [`Footing`] |
//! | The steps a mover can make, and their costs | [`NeighbourGeometry::steps`] with a [`MoveRules`]; [`MoveProfile`] is a plain walker |
//! | A way from one cell to another | [`Cosmos::find_path`], or a [`PathFinder`] kept between calls; the answer is a [`Path`] |
//! | Whether a cached answer is stale | [`RevStamp`]; [`FovSet::is_current`], [`Path::is_current`] |
//!
//! # What every query keeps to
//!
//! - **Truth.** These read the world as it is. What a character decides
//!   by must come from what it has perceived (01 R11): a mind's rules
//!   wrap these, they do not hand them to a mind.
//! - **Unloaded space.** Nothing here loads a tract or panics on one that
//!   is missing. A ray ends where loaded space ends and says so; a line
//!   of sight through it is an error; a view does not see into it; a
//!   path does not enter it.
//! - **Wrap.** On a wrapped axis every query carries on round.
//! - **Order.** Hits come nearest first; cells come storey by storey,
//!   row by row; steps come clockwise from north, then up, then down.
//!   Nothing depends on the order of a hash map.
//! - **Caches.** An answer that is kept carries the structure revisions
//!   of the chunks it was read from, and is stale when one of them has
//!   moved: after an edit, and after nothing else.
//!
//! # Not built yet
//!
//! Occupants join a ray through [`RayObstacles`] once the occupant index
//! exists. Subgrids are composited into every query by package W20a and
//! sight through portals is package W19. Fields do not block sight.

mod fov;
mod path;
mod ray;
mod stamp;

#[cfg(test)]
mod tests;

pub use fov::{FOV_MAX_REACH, FovOffsets, FovSet, Freshness};
pub use path::{
    CellGeometry, Connection, Footing, MoveProfile, MoveRules, NeighbourGeometry,
    PATH_DEFAULT_MAX_NODES, PROFILE_OPEN_COST_M, PROFILE_ROUGH_FACTOR, Path, PathFail, PathFinder,
    PathLimits, Step, StepKind,
};
pub use ray::{
    NoObstacles, RAY_MAX_M, RayBlocks, RayEnd, RayFilter, RayHit, RayHits, RayObstacles, RaySpan,
    RayTarget,
};
pub use stamp::{RevParts, RevStamp};

use crate::cosmos::{Cosmos, GridView};
use crate::error::WorldError;
use crate::grid::Column;
use crate::pos::{CellPos, Location};

/// The column that holds a cell, and the cell brought inside its grid.
/// An error value if the cell lies beyond an edge or its tract is not
/// loaded.
fn locate<'a>(view: &GridView<'a>, cell: CellPos) -> Result<(&'a Column, CellPos), WorldError> {
    let cell = view.normalize(cell)?;
    match view.column(cell.tract()) {
        Some(column) => Ok((column, cell)),
        None => Err(WorldError::NotLoaded(Location::new(view.id(), cell))),
    }
}

// The queries are read-only and may run in parallel (01 §4): everything
// they take and give back can cross threads.
const _: fn() = || {
    fn shared<T: Send + Sync>() {}
    shared::<Cosmos>();
    shared::<FovSet>();
    shared::<PathFinder>();
};
