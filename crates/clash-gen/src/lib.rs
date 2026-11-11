//! World generation for CLASH2D: surveying regions and realising tracts
//! from maps, placements, ground rules and settlement plans. Generation is
//! a pure function of the realm's seed and the position being generated,
//! so the same world appears in every order of exploration and on every
//! thread count. Specified in `docs/10-world-generation.md`, behind the
//! interfaces of `docs/02-world-model.md` §9.3.
//!
//! The real generators are WG1 to WG9 of `docs/plans/worldgen.md`. What
//! exists so far:
//!
//! | Need | Use |
//! |------|-----|
//! | The generation interfaces | [`RegionSurveyor`], [`TractRealizer`], [`GenCtx`]: defined in `clash-world`, which calls them, and re-exported here |
//! | The generators there are, ready for loading | [`generators`], or [`registry`] to add more first |
//! | Realising on several threads | [`RayonRunner`] |
//! | The test generators | [`FlatGenerator`] (`"generator": "flat"`), [`CityGenerator`] (`"generator": "test_city"`) |
//! | Tracts without a world, for tools and tests | [`realize_tracts`] |
//! | Looking at what was made | [`ascii`] |
//! | Noise | [`noise`] |
//!
//! # Getting tracts generated and loaded
//!
//! A realm definition names its generator and gives it parameters:
//!
//! ```json
//! { "type": "realm", "id": "plain", "generator": "flat",
//!   "generator_params": { "surface_z": 0, "floor": "grass" } }
//! ```
//!
//! The caller binds the generators to the definitions once, and passes
//! them, with a tract store, to whatever loads:
//!
//! ```
//! use std::sync::Arc;
//! use clash_world::{Cosmos, MemoryStore, RawWorldDefs, TractIo, WorldDefs};
//!
//! let defs = Arc::new(WorldDefs::build(RawWorldDefs::default()).unwrap());
//! let generators = clash_gen::generators(&defs);
//! let mut store = MemoryStore::new();
//! let mut io = TractIo::new(&generators, &mut store);
//! // With a `WorldCtx` over a cosmos that has a realm:
//! //     ctx.load_tracts(&mut io, &[(realm, tract)]);
//! // or, with tickets held, at every safe point:
//! //     ctx.reconcile_activity(&mut io);
//! # let _ = (&mut io, Cosmos::new(defs));
//! ```

pub mod ascii;
mod city;
mod flat;
pub mod noise;
mod parallel;
mod standalone;

use std::sync::Arc;

use clash_world::WorldDefs;
pub use clash_world::{
    BatchRunner, GenCtx, GenError, GeneratorFactory, GeneratorRegistry, GeneratorRequest,
    Generators, RealizeJob, RealizedTract, RealmInfo, RegionSurveyResult, RegionSurveyor,
    SerialRunner, TractRealizer, TractRecords3x3, WorldGenerator, realize_batch,
};

pub use city::{CITY_GENERATOR, CityGenerator, CityParams};
pub use flat::{FLAT_GENERATOR, FlatGenerator};
pub use parallel::{PoolError, RayonRunner};
pub use standalone::realize_tracts;

/// Registers the generators of this crate under the names realm
/// definitions give them by: `flat` and `test_city`.
pub fn register_generators(registry: &mut GeneratorRegistry) {
    registry.register(FLAT_GENERATOR, FlatGenerator::factory);
    registry.register(CITY_GENERATOR, CityGenerator::factory);
}

/// A registry holding the generators of this crate, which realises
/// batches of tracts on the threads of the process's pool.
pub fn registry() -> GeneratorRegistry {
    let mut registry = GeneratorRegistry::new();
    register_generators(&mut registry);
    registry.set_runner(Arc::new(RayonRunner::global()));
    registry
}

/// The generators of this crate, bound to a set of definitions: what
/// loading tracts takes. Batches are realised in parallel.
pub fn generators(defs: &Arc<WorldDefs>) -> Generators {
    registry().bind(defs)
}
