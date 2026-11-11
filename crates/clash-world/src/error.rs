//! [`WorldError`]: why a world operation could not be done (02 §23).

use clash_core::EntityId;
use clash_time::TimeError;
use thiserror::Error;

use crate::area::AreaId;
use crate::cosmos::RealmId;
use crate::defs::{AreaKindId, RealmDefId};
use crate::pos::{GridId, Location, TractPos};
use crate::site::SiteId;
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
    /// Something asked for is larger than the engine allows.
    #[error("{what} may be at most {limit}")]
    TooLarge {
        /// What is too large.
        what: &'static str,
        /// The most it may be.
        limit: u64,
    },
    /// A value given makes no sense.
    #[error("{what} cannot be that: {why}")]
    BadValue {
        /// What was given.
        what: &'static str,
        /// What is wrong with it.
        why: &'static str,
    },
    /// The occupant is already in the world. Move it with `relocate`.
    #[error("occupant {0} is already in the world")]
    AlreadyPlaced(EntityId),
    /// No occupant in the world has this id.
    #[error("occupant {0} is not in the world")]
    UnknownOccupant(EntityId),
    /// No area has this id.
    #[error("there is no area {0}")]
    UnknownArea(AreaId),
    /// No `area_kind` definition has this id.
    #[error("there is no area kind {0}")]
    UnknownAreaKind(AreaKindId),
    /// No site has this id.
    #[error("there is no site {0}")]
    UnknownSite(SiteId),
    /// A tract belongs to at most one site, and this one has one.
    #[error("tract {tract} of realm {realm} already belongs to site {site}")]
    TractHasSite {
        /// The realm.
        realm: RealmId,
        /// The tract.
        tract: TractPos,
        /// The site it belongs to.
        site: SiteId,
    },
    /// The space does not close: it leaks through this cell.
    #[error("the space is not closed: it leaks through {leak}")]
    NotEnclosed {
        /// A cell through which it leaks.
        leak: Location,
    },
    /// The cell is filled, so there is no space around it to enclose.
    #[error("{0} is filled: there is no space there to enclose")]
    NothingToEnclose(Location),
    /// The tract's region has not been surveyed, so the tract has no
    /// record yet.
    #[error("tract {tract} of realm {realm} has no record: its region has not been surveyed")]
    NotSurveyed {
        /// The realm.
        realm: RealmId,
        /// The tract.
        tract: TractPos,
    },
}
