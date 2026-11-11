//! The CLASH2D world model: the cosmos and its realms, grids of cells in
//! paletted chunks, tracts and regions, fields, areas and sites, damage,
//! structural collapse, energy releases and bodies in flight. Cells change
//! only through `WorldEdit`, and the world emits events rather than calling
//! upward: it does not know that entities, rules or a renderer exist.
//! Specified in `docs/02-world-model.md`.

pub mod dims;
pub mod iter;
mod pos;
pub mod storage;
mod topology;

pub use pos::{
    CellBox, CellIdx, CellPos, ChunkPos, GridId, Location, PointM, RegionPos, TractPos, distance_m,
    distance_sq_cells,
};
pub use topology::{Edge, Extent, Topology};
