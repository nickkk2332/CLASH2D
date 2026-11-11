//! The CLASH2D world model: the cosmos and its realms, grids of cells in
//! paletted chunks, tracts and regions, fields, areas and sites, damage,
//! structural collapse, energy releases and bodies in flight. Cells change
//! only through `WorldEdit`, and the world emits events rather than calling
//! upward: it does not know that entities, rules or a renderer exist.
//! Specified in `docs/02-world-model.md`.
//!
//! # Where things are
//!
//! | Need | Use |
//! |------|-----|
//! | The sizes of cells, chunks, tracts and regions | [`dims`] |
//! | A position | [`Location`]: a [`GridId`] and a [`CellPos`]. Also [`ChunkPos`], [`TractPos`], [`RegionPos`], [`CellIdx`], [`PointM`], [`CellBox`] |
//! | The shape of a grid: extent, wrap, edges | [`Topology`], [`Extent`], [`Edge`] |
//! | Cells in a defined order: neighbours, shells, a segment | [`iter`] |
//! | What a chunk is made of | [`storage`]: `Paletted`, `SparseCells`, `CellBits` |
//! | Definitions: materials, blocks, floors, fixtures, realms | [`defs`], built into [`WorldDefs`] |
//! | The state of an element | [`FloorState`], [`BlockState`], [`FixtureState`], [`ElementState`] |
//! | Stored cells | [`Chunk`], [`Column`], [`Grid`]; built with [`ColumnBuilder`] |
//! | The world, and reading it | [`Cosmos`], [`GridView`], [`CellView`], [`CellFlags`], [`Ambient`] |
//! | Why something could not be done | [`WorldError`] |
//! | A world for a test | [`test_support`] |
//!
//! # Rules this crate keeps
//!
//! - Every position is a [`Location`]. The same coordinates mean
//!   different places in different grids.
//! - Reading a cell that is not loaded is an error value
//!   ([`WorldError::NotLoaded`]). It never loads and never panics.
//! - Results come in a defined order: cells storey by storey, row by row.
//! - Nothing here names a material or a block. What exists is what the
//!   definitions say.

mod area;
mod cause;
mod chunk;
mod cosmos;
pub mod defs;
pub mod dims;
mod edit;
mod error;
mod event;
mod grid;
pub mod iter;
mod occupancy;
mod pos;
mod site;
pub mod storage;
pub mod test_support;
mod topology;

pub use area::{
    AREA_MAX_CELLS, AREA_MAX_TRACTS, AmbientOverride, Area, AreaAnchor, AreaDrift, AreaId,
    AreaShape, AreaSpec, AreaTable, Drift, Enclosure, FIELD_BLOCK_COVERAGE, GravityOverride,
};
pub use cause::{Cause, CauseKind};
pub use chunk::{
    Chunk, ChunkRev, ConduitId, ConduitSet, DAMAGE_MAX, Decal, DecalId, DecalList, FieldId,
    FieldStore, OccupantIndex,
};
pub use cosmos::{Ambient, Cosmos, GridView, Realm, RealmId, RealmState};
pub use defs::{
    AreaKindId, AtmosphereId, BlockId, BlockState, CellFlags, DamageTypeId, DefError, ElementFlags,
    ElementState, EngineFlag, FixtureId, FixtureState, FloorId, FloorState, Layer, MaterialId,
    PlaneId, RawWorldDefs, RealmDefId, StatePart, StateRef, StateRefError, WorldDefs,
};
pub use edit::{DISPLACE_RANGE_CELLS, StabilityQueue, StabilityRequest, WorldCtx, WorldEdit};
pub use error::WorldError;
pub use event::{EventSink, WorldEvent};
pub use grid::{
    COLUMN_MAX_CHUNKS, CellView, Column, ColumnBuilder, Grid, HEIGHT_FULL, HEIGHT_NONE,
};
pub use occupancy::{
    FOOTPRINT_MAX_SIDE, Footprint, Occupant, OccupantEntry, OccupantFilter, OccupantFlags,
    OccupantKind, OccupantKinds, OccupantRecord, Occupied, PlaneMask, QUERY_MAX_LINE_CELLS,
    QUERY_MAX_TRACTS, distinct_ids,
};
pub use pos::{
    CellBox, CellIdx, CellPos, ChunkPos, GridId, Location, PointM, RegionPos, TractPos, distance_m,
    distance_sq_cells,
};
pub use site::{Site, SiteDefId, SiteId, SiteSpec, SiteTable};
pub use topology::{Edge, Extent, Topology};
