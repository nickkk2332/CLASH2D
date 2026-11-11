//! The map format (10 §3): places written as rows of characters, one
//! layer for each storey, with a palette saying what each character
//! stands for.
//!
//! ```json
//! { "type": "map", "id": "hut", "size": [5, 4], "front": "south",
//!   "params": { "wall": { "pick": ["brick", "wood"] } },
//!   "palette": [ { "#": "wall@$wall", "+": "door", ".": {} } ],
//!   "default": { "floor": "boards" },
//!   "levels": { "0": [ "#####",
//!                      "#...#",
//!                      "#...#",
//!                      "##+##" ],
//!               "1": "roof" } }
//! ```
//!
//! # From files to cells
//!
//! 1. **Loading.** [`register_defs`] gives the loader the `map` and
//!    `palette` definition types ([`MapDef`], [`PaletteDef`]). The
//!    loader reads them, links their references and compiles their
//!    expressions in the generation context: the map's parameters as
//!    `$name`, the pure built-in functions, and the random functions.
//! 2. **Building.** [`MapDefs::from_defs`] turns loaded definitions and
//!    the world's tables into compiled maps: every state resolved to a
//!    typed state, every character to its entry, every nested map to
//!    its definition. The checks of 10 §3.7 are made here, each a
//!    located diagnostic. [`check_defs`] is this step for the linter.
//! 3. **Resolving.** [`MapDefs::resolve`] makes every choice of one
//!    placement (parameters, weighted alternatives, chances, nested
//!    maps, the turn) and gives a [`ResolvedMap`]: a grid of cells for
//!    each storey and the lists of everything else the map asks for.
//!
//! Putting a resolved map into the world is not done here: see
//! [`ResolvedMap`] for what that step reads.
//!
//! # What is carried, not acted on
//!
//! The world cannot yet take everything a map can say. Spawns, items,
//! conduits and wiring links, fields, devices, berths, watched cells,
//! cell variables, decals, patterns, joints, portals and vehicles are
//! read, checked as far as a map alone allows, turned with the map and
//! handed over in the [`ResolvedMap`] as data, with ids as written.
//! Nothing a map says is dropped.
//!
//! # What is not built
//!
//! - `region.*` fields in the generation context (10 §13): they come
//!   with region surveys.
//! - `gen.max_nesting` as a setting: the limit is [`MapLimits`], with
//!   the setting's default, until settings exist.
//! - A link between devices of two different nested maps, and telling
//!   apart two nested copies of a map that name the same device.
//! - `map check` (10 §3.7): reachability, required marks and whether
//!   the map stands.

mod ascii;
mod compile;
mod def;
mod facing;
mod resolve;

use clash_data::{DefTypes, Defs, Diagnostics};
use clash_world::WorldDefs;

pub use ascii::{ASCII_EMPTY, ASCII_NO_GLYPH};
pub use compile::{
    ClearedLayer, CompiledMap, CompiledParam, DEFAULT_MAX_NESTING, ItemSource, JointDir, LEAVE,
    MAX_MAP_LEVELS, MAX_MAP_SIDE, MapDefs, MapDevice, MapLimits, PRESET_ROOF, PRESETS, ParamType,
    PatternMatch, PortalTo, Sit, SpawnWho,
};
pub use def::{
    AREA_PREFIX, AltDef, AreaDef, Base, BerthDef, CountDef, DeviceDef, EntryDef, ItemsDef,
    JointDef, LevelDef, LevelsDef, LinkDef, Lit, MARK_PREFIX, MapDef, MapId, MapPickDef, MarkDef,
    Names, Ordered, PARAM_TYPES, PaletteDef, PaletteId, PaletteItem, ParamDef, ParamRule, PlaceDef,
    PointDef, PortalDef, SetDef, Sited, SpawnWhoDef, SpecDef, TurnDef, VehicleDef, WiringDef,
};
pub use facing::{direction_name, facing_of};
pub use resolve::{
    BerthCell, EvalOrder, MapLink, MapPortal, MapShape, MapVehicle, ResolveError, ResolvedArea,
    ResolvedCell, ResolvedItems, ResolvedJoint, ResolvedLevel, ResolvedMap, ResolvedSpawn,
    SpawnPlace,
};

/// Registers the definition types of the map format with the loader:
/// `map` and `palette`.
///
/// The application and the tools call this, with every other crate's
/// function of the same name, before they load (04 §9).
pub fn register_defs(types: &mut DefTypes) {
    types.register::<MapDef>();
    types.register::<PaletteDef>();
}

/// Everything wrong with the maps that only shows when they are built
/// against the world's tables (04 §8.1, the build stage; 10 §3.7).
/// Empty when they build.
///
/// If the world's own tables do not build there is nothing to check
/// maps against, and nothing is reported here: the world's check has
/// said why.
pub fn check_defs(defs: &Defs) -> Diagnostics {
    let Ok(world) = WorldDefs::from_defs(defs) else {
        return Diagnostics::new();
    };
    match MapDefs::from_defs(defs, &world) {
        Ok(_) => Diagnostics::new(),
        Err(diagnostics) => diagnostics,
    }
}
