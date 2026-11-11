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
//! | Everything generation is built from: maps, grounds, tract definitions | [`GenDefs::from_defs`] |
//! | The generators there are, ready for loading | [`generators_with`], or [`registry_with`] to add more first; [`generators`] for the test generators alone |
//! | Realising on several threads | [`RayonRunner`] |
//! | Tracts from tract definitions, grounds and placed maps | [`StandardGenerator`] (`"generator": "standard"`), in [`realize`] |
//! | Placing maps, and applying them to the world as it is | [`place`]: [`apply_map`], [`place_map`], [`apply_pending_maps`] |
//! | Ground: strata, a surface, liquid, veins, caves, scatter | [`ground`] |
//! | The test generators | [`FlatGenerator`] (`"generator": "flat"`), [`CityGenerator`] (`"generator": "test_city"`) |
//! | Tracts without a world, for tools and tests | [`realize_tracts`] |
//! | Looking at what was made | [`ascii`] |
//! | The map format: definitions, compiled maps, resolution | [`map`] |
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
pub mod ground;
pub mod map;
pub mod noise;
mod parallel;
pub mod place;
pub mod realize;
mod standalone;
pub mod tractdef;

use std::sync::Arc;

use clash_data::{DefTypes, Defs, Diagnostic, Diagnostics, code};
use clash_world::WorldDefs;
pub use clash_world::{
    BatchRunner, GenCtx, GenError, GeneratorFactory, GeneratorRegistry, GeneratorRequest,
    Generators, RealizeJob, RealizedTract, RealmInfo, RegionSurveyResult, RegionSurveyor,
    SerialRunner, TractRealizer, TractRecords3x3, WorldGenerator, realize_batch,
};

pub use city::{CITY_GENERATOR, CityGenerator, CityParams};
pub use flat::{FLAT_GENERATOR, FlatGenerator};
pub use ground::{Ground, GroundDef, GroundId, Stratum};
pub use parallel::{PoolError, RayonRunner};
pub use place::{
    MapApplied, MapMode, PendingMap, PlaceError, PlacedMap, Placement, apply_map,
    apply_pending_maps, boxes_overlap, place_map, placement_box,
};
pub use realize::{
    DroppedPlacement, STANDARD_GENERATOR, StandardGenerator, generators_with, register_standard,
    registry_with,
};
pub use standalone::realize_tracts;
pub use tractdef::{GenDefs, TractDef, TractKind};

/// Registers the definition types of this crate with the loader: `map`,
/// `palette`, `ground` and `tract`.
///
/// The application and the tools call this, with every other crate's
/// function of the same name, before they load (04 §9).
pub fn register_defs(types: &mut DefTypes) {
    map::register_defs(types);
    types.register::<GroundDef>();
    types.register::<TractDef>();
}

/// Everything wrong with the generation definitions that only shows
/// when they are built against the world's tables (04 §8.1, the build
/// stage): maps (10 §3.7), grounds and tract definitions, and the
/// parameters realm definitions give this crate's generators. Empty
/// when they build.
///
/// If the world's own tables do not build there is nothing to check
/// against, and nothing is reported here: the world's check has said
/// why. A realm's parameters are read from the definition as a whole,
/// so a mistake in them is reported with the realm and the parameter's
/// path, and without a line.
pub fn check_defs(defs: &Defs) -> Diagnostics {
    let Ok(world) = WorldDefs::from_defs(defs) else {
        return Diagnostics::new();
    };
    let built = match GenDefs::from_defs(defs, &world) {
        Ok(built) => built,
        Err(diagnostics) => return diagnostics,
    };
    let mut diagnostics = Diagnostics::new();
    let mut registry = GeneratorRegistry::new();
    register_generators(&mut registry);
    register_standard(&mut registry, &built);
    for error in registry.bind(&world).param_errors() {
        diagnostics.push(Diagnostic::new(code::VALIDATION, error.to_string()));
    }
    diagnostics
}

/// Registers the generators of this crate that need no generation
/// definitions, under the names realm definitions give them by: `flat`
/// and `test_city`. The `standard` generator is registered by
/// [`register_standard`], which gives it its definitions; here it is a
/// stand-in that says so.
pub fn register_generators(registry: &mut GeneratorRegistry) {
    registry.register(FLAT_GENERATOR, FlatGenerator::factory);
    registry.register(CITY_GENERATOR, CityGenerator::factory);
    registry.register(STANDARD_GENERATOR, realize::standard_unbound);
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
