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
//! | Definitions from mods | [`register_defs`], then [`WorldDefs::from_defs`]; see [`load`] |
//! | The state of an element | [`FloorState`], [`BlockState`], [`FixtureState`], [`ElementState`] |
//! | Stored cells | [`Chunk`], [`Column`], [`Grid`]; built with [`ColumnBuilder`] |
//! | The world, and reading it | [`Cosmos`], [`GridView`], [`CellView`], [`CellFlags`], [`Ambient`] |
//! | Rays, lines of sight, fields of view, the geometry round a cell, paths | [`query`] |
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

mod cause;
mod chunk;
mod cosmos;
pub mod defs;
pub mod dims;
mod error;
mod grid;
pub mod iter;
pub mod load;
mod pos;
pub mod query;
pub mod storage;
pub mod test_support;
mod topology;

pub use cause::{Cause, CauseKind};
pub use chunk::{
    Chunk, ChunkRev, ConduitId, ConduitSet, DAMAGE_MAX, Decal, DecalId, DecalList, FieldId,
    FieldStore, OccupantIndex,
};
pub use cosmos::{Ambient, Cosmos, GridView, Realm, RealmId, RealmState};
pub use defs::{
    AtmosphereId, BlockId, BlockState, CellFlags, DamageTypeId, DefError, ElementFlags,
    ElementState, EngineFlag, FixtureId, FixtureState, FloorId, FloorState, Layer, MaterialId,
    RawWorldDefs, RealmDefId, StatePart, StateRef, StateRefError, WorldDefs,
};
pub use error::WorldError;
pub use grid::{CellView, Column, ColumnBuilder, Grid, HEIGHT_FULL, HEIGHT_NONE};
pub use load::{check_defs, register_defs};
pub use pos::{
    CellBox, CellIdx, CellPos, ChunkPos, GridId, Location, PointM, RegionPos, TractPos, distance_m,
    distance_sq_cells,
};
pub use query::{
    CellGeometry, Connection, Footing, FovSet, Freshness, MoveProfile, MoveRules,
    NeighbourGeometry, NoObstacles, Path, PathFail, PathFinder, PathLimits, RayBlocks, RayEnd,
    RayFilter, RayHit, RayHits, RayObstacles, RaySpan, RayTarget, RevParts, RevStamp, Step,
    StepKind,
};
pub use topology::{Edge, Extent, Topology};
