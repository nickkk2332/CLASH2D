//! [`WorldError`]: why a world operation could not be done (02 §23).

use clash_time::TimeError;
use thiserror::Error;

use crate::cosmos::RealmId;
use crate::defs::RealmDefId;
use crate::pos::{GridId, Location};
use crate::topology::Edge;

/// Why a world operation could not be done. An operation that returns
/// one has changed nothing.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[non_exhaustive]
pub enum WorldError {
    /// The cell's tract is not loaded. Reading unloaded space never loads
    /// it and never panics: hold a ticket, or use the coarse layer.
    #[error("{0} is not loaded")]
    NotLoaded(Location),
    /// The cell lies beyond an edge of its realm.
    #[error("{loc} is outside the realm, beyond its {edge} edge")]
    OutsideRealm {
        /// The position as given.
        loc: Location,
        /// The edge it lies beyond.
        edge: Edge,
    },
    /// No grid has this id.
    #[error("there is no grid {0}")]
    UnknownGrid(GridId),
    /// No realm has this id.
    #[error("there is no realm {0}")]
    UnknownRealm(RealmId),
    /// No realm definition has this id.
    #[error("there is no realm definition {0}")]
    UnknownRealmDef(RealmDefId),
    /// Every id of some kind has been used.
    #[error("no {0} ids are left")]
    IdsExhausted(&'static str),
    /// The storage for a cell could not be reached: it lies too far from
    /// everything else stored in its tract.
    #[error("{0} is too far from the rest of its tract to store")]
    OutOfReach(Location),
    /// The timeline refused something the operation needed.
    #[error("the timeline refused: {0}")]
    Time(#[from] TimeError),
}
