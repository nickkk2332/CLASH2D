//! World generation for CLASH2D: surveying regions and realising tracts
//! from maps, placements, ground rules and settlement plans. Generation is
//! a pure function of the realm's seed and the position being generated,
//! so the same world appears in every order of exploration and on every
//! thread count. Specified in `docs/10-world-generation.md`.
//!
//! This is a stub. It is filled in by WG1 to WG9 of `docs/plans/worldgen.md`.
//! What exists so far is [`noise`]: the deterministic noise functions and
//! the description of a noise field that those packages build on; and
//! [`map`]: the map format, from definitions to resolved maps.

pub mod map;
pub mod noise;

pub use map::{check_defs, register_defs};
