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
//! | How bright a cell is | [`Cosmos::light`], [`Cosmos::fill_light`]; see [`light`] |
//! | Changing cells | [`WorldCtx::edit`] gives a [`WorldEdit`]: the only way cells change |
//! | What happened | [`WorldEvent`], put into an [`EventSink`] |
//! | Who and what did it | [`Cause`] |
//! | Who is where | [`WorldCtx::place`], [`WorldCtx::relocate`], [`WorldCtx::remove`]; [`Occupant`], [`Footprint`], [`PlaneMask`]; [`Cosmos::occupants_at`] and the other queries with an [`OccupantFilter`] |
//! | Volumes that mean something | [`Area`], [`AreaSpec`], [`AreaShape`]; [`WorldCtx::create_area`], [`WorldCtx::enclose`]; [`Cosmos::areas_at`] |
//! | Places with identity | [`Site`], [`Cosmos::create_site`], [`Cosmos::site_of`] |
//! | Fire, smoke, water: values in cells that evolve by rule | [`field`]: [`WorldEdit::add_field`], [`Cosmos::field`], [`Cosmos::field_lights`], [`Cosmos::field_hides_sight`] |
//! | Harm to an element, and its destruction | [`WorldEdit::damage`] with a [`DamagePacket`] gives a [`DamageOutcome`]; the rule alone is [`resolve_packet`] |
//! | Whether a structure stands, and what carries what | [`Cosmos::assess_stability`] gives an [`Assessment`] |
//! | Stability checks waiting | [`Cosmos::stability_requests`]; ask for one with [`WorldCtx::check_stability`] |
//! | The world's scheduled activity | [`WorldCtx::handle_wake`], for every wake-up of the world's domain; [`WorldProcess`], [`WorldActivity`] |
//! | Collapses in progress and masses in flight | [`Cosmos::collapses`], [`Cosmos::falling_masses`]; [`Collapse`], [`FallingMass`] |
//! | What is known of space that is not loaded | [`RegionRecord`], [`TractRecord`]; [`Cosmos::tract_record`], [`Cosmos::add_pending_edit`] |
//! | Generating: the interfaces, and choosing a generator | [`RegionSurveyor`], [`TractRealizer`], [`GenCtx`]; [`GeneratorRegistry`], [`Generators`] |
//! | Loading and unloading tracts | [`WorldCtx::load_tracts`], [`WorldCtx::unload_tract`], with a [`TractIo`] |
//! | Where a save is kept | [`TractStore`], [`MemoryStore`], [`RedbStore`]; [`encode_tract`], [`decode_tract`]; [`SaveIds`] |
//! | Saving and loading the world | [`Cosmos::save_world`], [`Cosmos::load_world`]; [`save`] |
//! | Keeping tracts loaded | [`Ticket`], [`Cosmos::hold_ticket`], [`WorldCtx::reconcile_activity`] |
//! | Why something could not be done | [`WorldError`] |
//! | A world for a test | [`test_support`] |
//!
//! # Rules this crate keeps
//!
//! - Every position is a [`Location`]. The same coordinates mean
//!   different places in different grids.
//! - Reading a cell that is not loaded is an error value
//!   ([`WorldError::NotLoaded`]). It never loads and never panics.
//! - Cells change only through [`WorldEdit`], and an occupant's position
//!   only through [`WorldCtx::relocate`]. Whatever changes emits an
//!   event; the world never calls upward.
//! - Results come in a defined order: cells storey by storey, row by row.
//! - Nothing here names a material or a block. What exists is what the
//!   definitions say.

mod area;
mod cause;
mod chunk;
mod collapse;
mod cosmos;
mod damage;
pub mod defs;
pub mod dims;
mod edit;
mod error;
mod event;
pub mod field;
mod generate;
mod grid;
pub mod iter;
pub mod light;
pub mod load;
mod loading;
mod occupancy;
mod pos;
mod process;
pub mod query;
mod redb_store;
pub mod save;
pub mod saveid;
mod site;
mod stability;
pub mod storage;
mod store;
pub mod test_support;
mod ticket;
mod topology;
mod tract;
mod vars;

pub use area::{
    AREA_MAX_CELLS, AREA_MAX_TRACTS, AmbientOverride, Area, AreaAnchor, AreaDrift, AreaId,
    AreaShape, AreaSpec, AreaTable, Drift, Enclosure, FIELD_BLOCK_COVERAGE, GravityOverride,
};
pub use cause::{Cause, CauseKind};
pub use chunk::{
    Chunk, ChunkRev, ConduitId, ConduitSet, DAMAGE_MAX, Decal, DecalId, DecalList, FieldId,
    FieldStore, OccupantIndex,
};
pub use collapse::{
    Collapse, CollapseId, CollapseState, CollapseTally, DebrisPart, FALL_IMPACT_DEPTH_M,
    FallingMass, SETTLE_DELAY,
};
pub use cosmos::{Ambient, Cosmos, GridView, Realm, RealmId, RealmState};
pub use damage::{
    DAMAGE_PARTS, DamageOutcome, DamagePacket, DamageResult, DebrisSpec, Destruction,
    RUBBLE_CELL_UNITS, RUBBLE_UNITS_PER_M3, resolve_packet,
};
pub use defs::{
    AreaKindId, AtmosphereId, BlockId, BlockState, CellFlags, DamageTypeId, DefError, DefKind,
    ElementFlags, ElementState, EngineFlag, FixtureId, FixtureState, FloorId, FloorState, Layer,
    MaterialId, PlaneId, RawWorldDefs, RealmDefId, StatePart, StateRef, StateRefError, WorldDefs,
};
pub use edit::{DISPLACE_RANGE_CELLS, StabilityQueue, StabilityRequest, WorldCtx, WorldEdit};
pub use error::WorldError;
pub use event::{EventSink, WorldEvent};
pub use field::{
    CatchUp, ExposureRoute, FIELD_MAX_ACTIVE_CELLS, FIELD_OVER_BUDGET_DECAY, Field, FieldCell,
    FieldReaction, FieldReactionId, FieldRule, FieldSample, FieldState, FieldStep,
};
pub use generate::GeneratorMaker;
pub use generate::{
    BatchRunner, GenCtx, GenError, GeneratorFactory, GeneratorRegistry, GeneratorRequest,
    Generators, PortalSpec, RealizeJob, RealizedTract, RealmInfo, RegionSurveyResult,
    RegionSurveyor, SerialRunner, SubgridSpec, TractRealizer, TractRecords3x3, WorldGenerator,
    realize_batch,
};
pub use grid::{
    COLUMN_MAX_CHUNKS, CellView, Column, ColumnBuilder, Grid, HEIGHT_FULL, HEIGHT_NONE,
};
pub use light::{
    LightSource, LightSpec, LightStats, MovingLight, Sky, SkyDef, SkyId, SkyTime, StaticLight,
};
pub use load::{check_defs, register_defs};
pub use loading::{LoadError, LoadedTract, TractIo};
pub use occupancy::{
    FOOTPRINT_MAX_SIDE, Footprint, Occupant, OccupantEntry, OccupantFilter, OccupantFlags,
    OccupantKind, OccupantKinds, OccupantRecord, Occupied, PlaneMask, QUERY_MAX_LINE_CELLS,
    QUERY_MAX_TRACTS, distinct_ids,
};
pub use pos::{
    CellBox, CellIdx, CellPos, ChunkPos, GridId, Location, PointM, RegionPos, TractPos, distance_m,
    distance_sq_cells,
};
pub use process::{SettleCheck, WorldActivity, WorldProcess};
pub use query::{
    CellGeometry, Connection, Footing, FovSet, Freshness, MoveProfile, MoveRules,
    NeighbourGeometry, NoObstacles, Path, PathFail, PathFinder, PathLimits, RayBlocks, RayEnd,
    RayFilter, RayHit, RayHits, RayObstacles, RaySpan, RayTarget, RevParts, RevStamp, Step,
    StepKind,
};
pub use redb_store::RedbStore;
pub use save::{SaveError, SaveReport, SaveWarning, WarnAt};
pub use saveid::{MissingDef, SaveIds, SaveIdsImage};
pub use site::{Site, SiteDefId, SiteId, SiteSpec, SiteTable};
pub use stability::{
    Assessment, SUPPORT_MAX_ELEMENTS, SUPPORT_MAX_PASSES, SUPPORT_REACH_CELLS,
    SUPPORT_REACH_STOREYS,
};
pub use store::{
    MemoryStore, StoreError, StoredTract, TRACT_BLOB_KIND, TRACT_BLOB_VERSION, TractStore,
    decode_tract, encode_tract,
};
pub use ticket::{
    Activity, DEFAULT_MAX_LOADED_TRACTS, DEFAULT_PRIORITY, HOT_RADIUS, PINNED, Reconciled,
    RefusedTicket, Ticket, TicketAnchor, TicketId, TicketReason, TicketTable, TractFailure,
    UNLOAD_MARGIN, WARM_MARGIN,
};
pub use topology::{Edge, Extent, Topology};
pub use tract::{
    CellEdit, DevastationKindId, MapId, PathKindId, PathLine, PendingEdit, Placement, RegionKindId,
    RegionRecord, RegionSurvey, SiteIdx, SpawnRequest, Stamp, TractDefId, TractExtra, TractRecord,
    TractState,
};
pub use tract::{MapMode, PendingMap};
