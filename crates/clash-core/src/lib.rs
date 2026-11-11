//! The shared vocabulary of the CLASH2D engine.
//!
//! Every other crate in the workspace is written against the types here.
//! This crate depends on nothing in the workspace and knows nothing about
//! worlds, entities or rules. What it provides is what the determinism
//! contract (`docs/01-architecture.md` section 4) needs everyone to share:
//!
//! | Need | Use |
//! |------|-----|
//! | A physical magnitude | [`Real`]: an `f64` that is never NaN, with exact operators and `libm`-backed functions |
//! | A point or span of time | [`Tick`], [`Dur`]: integer microseconds |
//! | A cell position or offset | [`IVec2`], [`IVec3`] |
//! | A direction | [`Dir8`], [`Vertical`] |
//! | A quarter-turn, with or without a mirror | [`Rot4`], [`Turn`] |
//! | A typed dense id | [`define_id!`], [`DenseId`] |
//! | An entity's identity | [`EntityId`]: slot and generation |
//! | Fast lookup by key | [`LookupMap`]: a hash map that cannot be iterated |
//! | A small ordered set or map | a sorted `Vec` with the [`sorted`] helpers, or `BTreeMap` |
//! | Randomness decided by place | [`pos_hash`], [`pos_rng`], [`HashDomain`] |
//! | Randomness decided by sequence | [`RngStreams`], [`Rng`] |
//! | A state hash | [`StateHash`], [`StateHasher`], [`Hash128`], [`impl_state_hash!`] |
//! | A value a mod stores | [`Value`], [`VarKey`], [`VarMap`] |
//! | Bytes for a save or a stored blob | [`codec`]: a compact binary serde format and a checked, compressed frame |
//! | Reporting failure | the convention in [`error`] |
//!
//! # What simulation code must not use
//!
//! The workspace's `clippy.toml` denies these in simulation crates, and
//! each has a replacement above: `std` hash maps and sets (iteration order
//! is not deterministic), `f32`/`f64` functions such as `sin` and `powf`
//! (results differ between platforms), `std::time` (the simulation never
//! reads the wall clock) and any randomness that is not one of the two
//! kinds above.
//!
//! # Units
//!
//! Magnitudes are in SI units: metres, kilograms, seconds, joules,
//! pascals, kelvin (01 R8). A [`Real`] carries no unit of its own, so name
//! variables and fields for what they measure.
//!
//! # Where things are
//!
//! Everything in the table is exported from the crate root: write
//! `clash_core::Real`, not `clash_core::real::Real`. The modules are
//! public so that their own documentation can be read: each opens with
//! the conventions its types share.

pub mod codec;
pub mod dir;
pub mod error;
pub mod hash;
pub mod id;
pub mod ivec;
pub mod lookup;
pub mod real;
pub mod rng;
pub mod rot;
pub mod sorted;
pub mod time;
pub mod value;

pub use codec::CodecError;
pub use dir::{Dir8, Vertical};
pub use error::{ParseHashError, RealError, RngError};
pub use hash::{Hash128, StateHash, StateHasher};
pub use id::{DenseId, EntityId};
pub use ivec::{IVec2, IVec3};
pub use lookup::LookupMap;
pub use real::Real;
pub use rng::{HashDomain, Rng, RngStreams, pos_hash, pos_rng};
pub use rot::{Rot4, Turn};
pub use time::{
    Dur, MICROS_PER_DAY, MICROS_PER_HOUR, MICROS_PER_MILLI, MICROS_PER_MINUTE, MICROS_PER_SECOND,
    Tick,
};
pub use value::{Value, VarKey, VarMap};

/// Not part of the public interface. Used by this crate's macros.
#[doc(hidden)]
pub mod __private {
    pub use serde;
}
