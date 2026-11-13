//! Saving and loading the whole world (02 §11).
//!
//! [`Cosmos::save_world`] writes everything the world holds into a
//! [`TractStore`], and [`Cosmos::load_world`] makes a cosmos out of what
//! a store holds. The two are invisible (01 R7): the cosmos that comes
//! back hashes equal to the one that was saved, and running on from
//! either gives the same result.
//!
//! # What is written
//!
//! | Section | Kind | Holds |
//! |---------|------|-------|
//! | `world.cosmos` | `WCOS` | The realms with their region and tract records, the table of grids, the global variables, and whether everything is stored |
//! | `world.areas` | `WARE` | Every area |
//! | `world.sites` | `WSIT` | Every site |
//! | `world.tickets` | `WTKT` | The tickets, which tracts they keep, and what the last reconciliation worked out |
//! | `world.stability` | `WSTB` | The stability checks waiting |
//! | `world.activity` | `WACT` | The world's processes and the collapses in progress |
//! | `world.tracts` | `WTRC` | Which tracts are loaded, and for each whether its cells are in the store or are generated again |
//! | `world.ids` | `WIDS` | The save's tables of ids ([`SaveIds`]) |
//!
//! Each section is a blob of 01 §11.1 with a kind and a version of its
//! own. The cells of loaded tracts go where the cells of unloaded ones
//! are, in tract blobs: a loaded tract that has changed is written, one
//! that has not is generated again when the world is loaded, like any
//! pristine tract.
//!
//! **A new world subsystem adds its section to `WORLD_SECTIONS`**, the
//! list in this module: a
//! name beginning with `world.`, a kind, a version, a function that
//! writes its state and one that puts it back. If its state names
//! definitions by dense id it implements `MapIds` (see
//! [`crate::saveid`]) and turns them in both functions. A section a save
//! does not hold leaves its subsystem as it is in a new cosmos, so a
//! save written before the subsystem existed still loads.
//!
//! Not written: who occupies which cell (the simulation owns the
//! entities and places them again), light, revision counters and every
//! other cache.
//!
//! # Saving
//!
//! At a safe point:
//!
//! 1. The simulation hands in the entity section of any tract it is
//!    about to let go of ([`Cosmos::set_tract_entities`]).
//! 2. `cosmos.save_world(store)`.
//! 3. The simulation writes its own sections with
//!    [`TractStore::put_section`]: the timeline with its clocks, its
//!    entities and the rest. Their names must not begin with `world.`.
//! 4. `store.commit()`, which makes all of it durable at once.
//!
//! # Loading
//!
//! 1. Load the mods and build the world's definitions.
//! 2. Bind the generators to those definitions.
//! 3. `Cosmos::load_world(&defs, &mut io)`. From here on the
//!    definitions to use are [`Cosmos::defs`]: they hold a placeholder
//!    for every definition the save names and the mods no longer supply
//!    (04 §12.3), after the definitions that were passed in.
//! 4. Restore the timeline from the simulation's section. The world
//!    does not touch the timeline while it loads: the clocks of realms
//!    and areas and the wake-ups of the world's processes are in the
//!    timeline as it was saved, and the cosmos holds only their ids. The
//!    two must come from the same save.
//! 5. Place the simulation's entities again with
//!    [`WorldCtx::place`](crate::WorldCtx::place), into an event sink
//!    that is then thrown away: placing changes nothing that is hashed.
//! 6. Read [`Cosmos::take_save_warnings`] and tell the player what has
//!    become a placeholder.
//!
//! ```
//! use std::sync::Arc;
//! use clash_core::Hash128;
//! use clash_time::Timeline;
//! use clash_world::{Cosmos, Generators, MemoryStore, RawWorldDefs, TractIo, TractStore, WorldDefs};
//!
//! let defs = Arc::new(WorldDefs::build(RawWorldDefs::default()).unwrap());
//! let mut cosmos = Cosmos::new(Arc::clone(&defs));
//! let mut store = MemoryStore::new();
//!
//! cosmos.save_world(&mut store).unwrap();
//! store.commit().unwrap();
//!
//! let generators = Generators::none(&defs);
//! let mut io = TractIo::new(&generators, &mut store);
//! let back = Cosmos::load_world(&defs, &mut io).unwrap();
//! assert_eq!(Hash128::of(&back), Hash128::of(&cosmos));
//! ```

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use clash_core::{CodecError, VarMap, codec};
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::area::AreaTable;
use crate::cosmos::{Cosmos, Realm, RealmId};
use crate::defs::{DefKind, WorldDefs};
use crate::edit::StabilityQueue;
use crate::error::WorldError;
use crate::generate::{RealizeJob, realize_batch};
use crate::grid::Column;
use crate::loading::{LoadError, TractIo};
use crate::pos::{GridId, TractPos};
use crate::process::WorldActivity;
use crate::saveid::{IdMap, MapIds, SaveIds, SaveIdsImage, ToDense, ToSave};
use crate::site::SiteTable;
use crate::store::{StoreError, TractStore, decode_tract, encode_image};
use crate::ticket::{Activity, TicketTable, TractKey};
use crate::topology::{Extent, Topology};

/// What the names of the world's sections begin with. The simulation's
/// own sections must begin otherwise.
pub const WORLD_SECTION_PREFIX: &str = "world.";

/// The section that holds the save's tables of ids.
pub const IDS_SECTION: &str = "world.ids";
/// The kind of the blob in [`IDS_SECTION`].
pub const IDS_BLOB_KIND: [u8; 4] = *b"WIDS";
/// The version of the blob in [`IDS_SECTION`].
pub const IDS_BLOB_VERSION: u32 = 1;

/// The section that lists the loaded tracts.
pub const TRACTS_SECTION: &str = "world.tracts";
/// The kind of the blob in [`TRACTS_SECTION`].
pub const TRACTS_BLOB_KIND: [u8; 4] = *b"WTRC";
/// The version of the blob in [`TRACTS_SECTION`].
pub const TRACTS_BLOB_VERSION: u32 = 1;

/// The most warnings a cosmos keeps until they are taken. Past this the
/// oldest are kept and the rest dropped: the same definition missing
/// from a thousand tracts is one thing to tell the player.
pub const SAVE_WARNINGS_MAX: usize = 1000;

/// Where a definition that is missing was met.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub enum WarnAt {
    /// In the save's tables of ids: the save names it somewhere.
    Ids,
    /// In a section of the world's state.
    Section(&'static str),
    /// In the cells of a tract.
    Tract {
        /// The realm.
        realm: RealmId,
        /// The tract.
        tract: TractPos,
    },
}

impl core::fmt::Display for WarnAt {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            WarnAt::Ids => f.write_str("the save's ids"),
            WarnAt::Section(name) => write!(f, "section `{name}`"),
            WarnAt::Tract { realm, tract } => write!(f, "tract {tract} of realm {realm}"),
        }
    }
}

/// A definition a save names that no loaded mod supplies, and where it
/// was met. What was made from it is a placeholder now, and is restored
/// when the definition is back (04 §12.3).
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub struct SaveWarning {
    /// The definition type.
    pub kind: DefKind,
    /// The qualified id the save wrote.
    pub id: String,
    /// Where it was met.
    pub at: WarnAt,
}

impl core::fmt::Display for SaveWarning {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(
            f,
            "{}: {} `{}` is not defined by the loaded mods; a placeholder stands in for it",
            self.at, self.kind, self.id
        )
    }
}

/// Why a world could not be saved or loaded.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[non_exhaustive]
pub enum SaveError {
    /// The store failed, or a tract blob is damaged.
    #[error(transparent)]
    Store(#[from] StoreError),
    /// A section is damaged, was written by another layout, or holds
    /// something that cannot be.
    #[error("section `{name}` of the save could not be used: {error}")]
    Section {
        /// The section.
        name: String,
        /// What is wrong with it.
        error: StoreError,
    },
    /// The store does not hold a section a saved world always has. It
    /// holds no saved world, or not all of one.
    #[error("the save has no section `{0}`")]
    MissingSection(&'static str),
    /// The store holds a section of the world that this build does not
    /// know: loading would drop what it holds.
    #[error("the save has a section `{0}` that this build does not read")]
    UnknownSection(String),
    /// A tract that was loaded when the world was saved has no cells in
    /// the store.
    #[error("tract {tract} of realm {realm} was saved as stored, and the store does not hold it")]
    MissingTract {
        /// The realm.
        realm: RealmId,
        /// The tract.
        tract: TractPos,
    },
    /// A tract that was loaded when the world was saved could not be
    /// made again.
    #[error("tract {tract} of realm {realm} could not be loaded again: {error}")]
    Tract {
        /// The realm.
        realm: RealmId,
        /// The tract.
        tract: TractPos,
        /// Why.
        error: Box<LoadError>,
    },
    /// The world refused.
    #[error(transparent)]
    World(#[from] WorldError),
}

/// What a save wrote.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct SaveReport {
    /// How many sections were written.
    pub sections: usize,
    /// How many loaded tracts are in the world.
    pub loaded_tracts: usize,
    /// How many tract blobs were written for them: the tracts that have
    /// changed, or all of them when everything is stored.
    pub tract_blobs: usize,
    /// The bytes written in all, sections and tract blobs.
    pub bytes: usize,
}

/// What a cosmos keeps for saving. None of it is state: it is not
/// hashed.
#[derive(Clone, Debug)]
pub(crate) struct SaveState {
    /// The save's tables of ids. Every tract blob in the store is
    /// written against them.
    ids: SaveIds,
    /// Whether tracts that have never been changed are stored too
    /// (`save.store_pristine`).
    store_pristine: bool,
    /// The simulation's entity sections for tracts that are loaded and
    /// about to be let go of.
    entities: BTreeMap<(RealmId, TractPos), Vec<u8>>,
    /// Missing definitions met and not yet reported.
    warnings: Vec<SaveWarning>,
}

impl SaveState {
    pub(crate) fn new(defs: &WorldDefs) -> SaveState {
        SaveState {
            ids: SaveIds::new(defs),
            store_pristine: false,
            entities: BTreeMap::new(),
            warnings: Vec::new(),
        }
    }
}

/// One named part of a saved world. See the module documentation.
pub(crate) struct WorldSection {
    /// The section's name in the store. Begins with
    /// [`WORLD_SECTION_PREFIX`].
    pub name: &'static str,
    /// The kind of its blob.
    pub kind: [u8; 4],
    /// The version of its body this build writes and reads.
    pub version: u32,
    /// Whether every saved world has it. A save without one that is not
    /// required leaves the subsystem as it is in a new cosmos.
    pub required: bool,
    /// Writes the subsystem's state as the section's body, with the ids
    /// of definitions turned by the map.
    pub write: fn(&Cosmos, &mut dyn IdMap) -> Result<Vec<u8>, CodecError>,
    /// Puts the subsystem's state back from a body, turning the ids of
    /// definitions by the map.
    pub read: fn(&mut Cosmos, &[u8], &mut dyn IdMap) -> Result<(), StoreError>,
}

/// Every part of the world that is saved as a section, in the order
/// they are written and read. **A new world subsystem adds its section
/// here.** The loaded tracts and the tables of ids are sections too,
/// handled by [`Cosmos::save_world`] itself because they need the
/// store.
pub(crate) const WORLD_SECTIONS: &[WorldSection] = &[
    WorldSection {
        name: "world.cosmos",
        kind: *b"WCOS",
        version: 1,
        required: true,
        write: write_cosmos,
        read: read_cosmos,
    },
    WorldSection {
        name: "world.areas",
        kind: *b"WARE",
        // 2: an area carries a light, and the table lists the areas
        // that do (08 §7.2).
        version: 2,
        required: true,
        write: write_areas,
        read: read_areas,
    },
    WorldSection {
        name: "world.sites",
        kind: *b"WSIT",
        version: 1,
        required: true,
        write: |cosmos, _| codec::to_bytes(&cosmos.sites),
        read: |cosmos, body, _| {
            cosmos.sites = codec::from_bytes::<SiteTable>(body)?;
            Ok(())
        },
    },
    WorldSection {
        name: "world.tickets",
        kind: *b"WTKT",
        version: 1,
        required: true,
        write: write_tickets,
        read: read_tickets,
    },
    WorldSection {
        name: "world.stability",
        kind: *b"WSTB",
        version: 1,
        required: true,
        write: |cosmos, _| codec::to_bytes(&cosmos.stability),
        read: |cosmos, body, _| {
            cosmos.stability = codec::from_bytes::<StabilityQueue>(body)?;
            Ok(())
        },
    },
    WorldSection {
        name: "world.activity",
        kind: *b"WACT",
        version: 1,
        required: true,
        write: write_activity,
        read: read_activity,
    },
];

/// The body of `world.cosmos`.
#[derive(Serialize, Deserialize)]
struct CosmosBody {
    /// The number the next realm will be given.
    next_realm: u32,
    /// The global variables.
    vars: VarMap,
    /// The realm each grid belongs to, by grid id.
    grids: Vec<Option<RealmId>>,
    /// Whether tracts that have never been changed are stored too.
    store_pristine: bool,
    /// The realms, each with the extent of its definition as it was,
    /// for the placeholder of a realm definition that has gone.
    realms: Vec<(Realm, Topology)>,
}

fn write_cosmos(cosmos: &Cosmos, map: &mut dyn IdMap) -> Result<Vec<u8>, CodecError> {
    let realms = cosmos
        .realms()
        .map(|realm| {
            let topology = cosmos
                .topology(realm.base_grid)
                .copied()
                .unwrap_or_else(|_| Topology::new(Extent::Unbounded, Extent::Unbounded, 0, 0));
            let mut realm = realm.clone();
            realm.map_ids(map);
            (realm, topology)
        })
        .collect();
    codec::to_bytes(&CosmosBody {
        next_realm: cosmos.next_realm_number(),
        vars: cosmos.vars().clone(),
        grids: cosmos.grid_realms(),
        store_pristine: cosmos.save.store_pristine,
        realms,
    })
}

fn read_cosmos(cosmos: &mut Cosmos, body: &[u8], map: &mut dyn IdMap) -> Result<(), StoreError> {
    let body: CosmosBody = codec::from_bytes(body)?;
    let realms = body
        .realms
        .into_iter()
        .map(|(mut realm, _)| {
            realm.map_ids(map);
            realm
        })
        .collect();
    cosmos
        .restore_realms(body.next_realm, body.vars, realms, body.grids)
        .map_err(StoreError::Malformed)?;
    cosmos.save.store_pristine = body.store_pristine;
    Ok(())
}

/// The extent of each realm definition a saved world uses, by the save
/// id of the definition: read before the ids are bound, for the
/// placeholders of realm definitions that have gone.
fn realm_shapes(body: &[u8]) -> Result<BTreeMap<u32, Topology>, StoreError> {
    let body: CosmosBody = codec::from_bytes(body)?;
    Ok(body
        .realms
        .iter()
        .map(|(realm, topology)| (u32::from(realm.def.raw()), *topology))
        .collect())
}

fn write_areas(cosmos: &Cosmos, map: &mut dyn IdMap) -> Result<Vec<u8>, CodecError> {
    let mut areas = cosmos.areas.clone();
    areas.map_ids(map);
    codec::to_bytes(&areas)
}

fn read_areas(cosmos: &mut Cosmos, body: &[u8], map: &mut dyn IdMap) -> Result<(), StoreError> {
    let mut areas: AreaTable = codec::from_bytes(body)?;
    areas.map_ids(map);
    cosmos.areas = areas;
    Ok(())
}

/// What the last reconciliation worked out, beside the table.
type TicketsDerived = (Vec<(TractKey, Activity)>, bool);

fn write_tickets(cosmos: &Cosmos, _: &mut dyn IdMap) -> Result<Vec<u8>, CodecError> {
    let derived: TicketsDerived = cosmos.tickets.derived();
    codec::to_bytes(&(&cosmos.tickets, derived))
}

fn read_tickets(cosmos: &mut Cosmos, body: &[u8], _: &mut dyn IdMap) -> Result<(), StoreError> {
    let (mut tickets, (levels, stale)): (TicketTable, TicketsDerived) = codec::from_bytes(body)?;
    tickets.restore_derived(levels, stale);
    cosmos.tickets = tickets;
    Ok(())
}

fn write_activity(cosmos: &Cosmos, map: &mut dyn IdMap) -> Result<Vec<u8>, CodecError> {
    let mut activity = cosmos.activity.clone();
    activity.map_ids(map);
    codec::to_bytes(&activity)
}

fn read_activity(cosmos: &mut Cosmos, body: &[u8], map: &mut dyn IdMap) -> Result<(), StoreError> {
    let mut activity: WorldActivity = codec::from_bytes(body)?;
    activity.map_ids(map);
    cosmos.activity = activity;
    Ok(())
}

/// Where the cells of a loaded tract come from when the world is
/// loaded.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
enum CellSource {
    /// The tract's blob in the store.
    Stored,
    /// The realm's generator: the tract had not changed.
    Generated,
}

/// One loaded tract, in `world.tracts`.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
struct LoadedEntry {
    grid: GridId,
    tract: TractPos,
    source: CellSource,
    /// Whether the tract had changed since it was loaded or generated,
    /// so that unloading it must write it.
    dirty: bool,
}

/// Opens a section's blob and checks its version.
fn open_section(
    name: &str,
    kind: [u8; 4],
    version: u32,
    blob: &[u8],
) -> Result<Vec<u8>, SaveError> {
    let wrap = |error: StoreError| SaveError::Section {
        name: name.to_owned(),
        error,
    };
    let (found, body) = codec::open_blob(kind, blob).map_err(|error| wrap(error.into()))?;
    if found != version {
        return Err(wrap(StoreError::Version {
            found,
            expected: version,
        }));
    }
    Ok(body)
}

impl Cosmos {
    /// The save's tables of ids (02 §11).
    pub fn save_ids(&self) -> &SaveIds {
        &self.save.ids
    }

    /// The save's tables of ids, to add to.
    pub(crate) fn save_ids_mut(&mut self) -> &mut SaveIds {
        &mut self.save.ids
    }

    /// Whether tracts that have never been changed are stored too.
    pub fn store_pristine(&self) -> bool {
        self.save.store_pristine
    }

    /// Sets whether tracts that have never been changed are stored too:
    /// the setting `save.store_pristine` (02 §11). Off, such a tract is
    /// not stored and is generated again, so it follows the mods; on,
    /// every tract is stored when it is unloaded or the world is saved,
    /// and is read back as it was.
    pub fn set_store_pristine(&mut self, store_pristine: bool) {
        self.save.store_pristine = store_pristine;
    }

    /// Hands in the simulation's entity section for a loaded tract:
    /// bytes the world stores with the tract when it is unloaded or the
    /// world is saved, and hands back in
    /// [`LoadedTract::entities`](crate::LoadedTract) when the tract is
    /// next loaded. The world neither reads nor drops them. Handing in
    /// nothing withdraws a section handed in before.
    ///
    /// Refused for a tract that is not loaded.
    pub fn set_tract_entities(
        &mut self,
        realm: RealmId,
        tract: TractPos,
        entities: Vec<u8>,
    ) -> Result<(), WorldError> {
        let tract = self.normalize_tract(realm, tract)?;
        if !self.is_tract_loaded(realm, tract) {
            let grid = self.realm_info(realm)?.base_grid;
            return Err(WorldError::NotLoaded(crate::pos::Location::new(
                grid,
                tract.origin(0),
            )));
        }
        if entities.is_empty() {
            self.save.entities.remove(&(realm, tract));
        } else {
            self.save.entities.insert((realm, tract), entities);
        }
        Ok(())
    }

    /// The entity section handed in for a loaded tract and not yet
    /// stored. Empty if there is none.
    pub fn tract_entities(&self, realm: RealmId, tract: TractPos) -> &[u8] {
        self.save
            .entities
            .get(&(realm, tract))
            .map_or(&[], Vec::as_slice)
    }

    /// Forgets the entity section handed in for a tract: it has been
    /// stored, or the tract has gone.
    pub(crate) fn drop_tract_entities(&mut self, realm: RealmId, tract: TractPos) {
        self.save.entities.remove(&(realm, tract));
    }

    /// Notes a missing definition, once for each place it is met.
    pub(crate) fn warn(&mut self, warning: SaveWarning) {
        if self.save.warnings.len() < SAVE_WARNINGS_MAX && !self.save.warnings.contains(&warning) {
            self.save.warnings.push(warning);
        }
    }

    /// The missing definitions met since this was last called, each with
    /// where: when the world was loaded, and as tracts that hold them
    /// are read from the store. In the order they were met.
    pub fn take_save_warnings(&mut self) -> Vec<SaveWarning> {
        core::mem::take(&mut self.save.warnings)
    }

    /// Writes the whole world into a store (02 §11): its sections, the
    /// cells of the loaded tracts that have changed, and the tables of
    /// ids. Call it at a safe point, then write the simulation's own
    /// sections and [`commit`](TractStore::commit). See the module
    /// documentation.
    ///
    /// Nothing that is hashed changes. The store is left holding
    /// everything of the world as it is now; nothing is durable until
    /// it is committed.
    pub fn save_world(&mut self, store: &mut dyn TractStore) -> Result<SaveReport, SaveError> {
        // The tables grow as state is written. They are kept whatever
        // happens: blobs already in the store are written against them.
        let mut ids = self.save.ids.clone();
        let outcome = self.write_world(store, &mut ids);
        self.save.ids = ids;
        outcome
    }

    fn write_world(
        &self,
        store: &mut dyn TractStore,
        ids: &mut SaveIds,
    ) -> Result<SaveReport, SaveError> {
        let defs = Arc::clone(self.defs());
        let mut report = SaveReport::default();
        let mut written: BTreeSet<&'static str> = BTreeSet::new();
        let mut put = |store: &mut dyn TractStore,
                       report: &mut SaveReport,
                       name: &'static str,
                       blob: Vec<u8>| {
            store.put_section(name, &blob);
            report.sections += 1;
            report.bytes += blob.len();
            written.insert(name);
        };
        for section in WORLD_SECTIONS {
            let wrap = |error: StoreError| SaveError::Section {
                name: section.name.to_owned(),
                error,
            };
            let mut map = ToSave::new(&defs, ids);
            let body = (section.write)(self, &mut map).map_err(|error| wrap(error.into()))?;
            map.finish().map_err(wrap)?;
            let blob = codec::seal_blob(section.kind, section.version, &body)
                .map_err(|error| wrap(error.into()))?;
            put(store, &mut report, section.name, blob);
        }

        // The loaded tracts: their cells where the cells of unloaded
        // tracts are, and a list of which are loaded.
        let mut loaded = Vec::new();
        for realm in self.realms() {
            let grid = realm.base_grid;
            let view = self.grid(grid)?;
            for tract in view.loaded_tracts() {
                let Some(column) = view.column(tract) else {
                    continue;
                };
                let record = self.tract_record(realm.id, tract);
                let stored = record.is_some_and(|record| record.state.is_stored());
                let dirty = column.is_dirty();
                // Cells that the realm's generator gives again need not
                // be stored: those of a tract the loading procedure
                // loaded, which has not changed since it was generated.
                let regenerable = !stored
                    && record.is_some_and(|record| record.state.is_realized())
                    && self.tickets.is_kept(realm.id, tract);
                let keep_cells = dirty || !regenerable || self.save.store_pristine;
                let entities = self.tract_entities(realm.id, tract);
                let image = if keep_cells && (dirty || !stored || !entities.is_empty()) {
                    Some(Some(column.image()))
                } else if !keep_cells && !entities.is_empty() {
                    Some(None)
                } else {
                    // What the store holds for the tract is right as it
                    // is: its cells as they were read, or nothing.
                    None
                };
                if let Some(cells) = image {
                    let blob = encode_image(&defs, ids, cells, entities)?;
                    store.put(realm.id, tract, &blob);
                    report.tract_blobs += 1;
                    report.bytes += blob.len();
                }
                loaded.push(LoadedEntry {
                    grid,
                    tract,
                    source: if keep_cells {
                        CellSource::Stored
                    } else {
                        CellSource::Generated
                    },
                    dirty,
                });
            }
        }
        report.loaded_tracts = loaded.len();
        let wrap = |name: &'static str| {
            move |error: CodecError| SaveError::Section {
                name: name.to_owned(),
                error: error.into(),
            }
        };
        let blob = codec::encode_blob(TRACTS_BLOB_KIND, TRACTS_BLOB_VERSION, &loaded)
            .map_err(wrap(TRACTS_SECTION))?;
        put(store, &mut report, TRACTS_SECTION, blob);

        // Last, because everything before it may have added to them.
        let blob = codec::encode_blob(IDS_BLOB_KIND, IDS_BLOB_VERSION, &ids.image())
            .map_err(wrap(IDS_SECTION))?;
        put(store, &mut report, IDS_SECTION, blob);

        // A section of the world that was not written now is from a
        // world that held something this one does not.
        for name in store.section_names()? {
            if name.starts_with(WORLD_SECTION_PREFIX) && !written.contains(name.as_str()) {
                store.delete_section(&name);
            }
        }
        Ok(report)
    }

    /// Makes a cosmos out of a saved world (02 §11): what
    /// [`save_world`](Cosmos::save_world) wrote into `io.store`. See the
    /// module documentation for what to do before and after.
    ///
    /// The tracts that were loaded are loaded again without events:
    /// those that had changed are read from the store, and the rest are
    /// generated by `io.generators`, as pristine tracts always are. The
    /// cosmos that comes back hashes equal to the one that was saved.
    ///
    /// A definition the save names and `defs` does not hold becomes a
    /// placeholder (04 §12.3): [`Cosmos::defs`] of the result holds
    /// them, and [`Cosmos::take_save_warnings`] lists them.
    ///
    /// Damaged or missing data is an error, never a panic, and nothing
    /// is changed in the store.
    pub fn load_world(defs: &Arc<WorldDefs>, io: &mut TractIo<'_>) -> Result<Cosmos, SaveError> {
        let store: &dyn TractStore = &*io.store;
        let section = |name: &'static str, kind: [u8; 4], version: u32| {
            let blob = store
                .section(name)?
                .ok_or(SaveError::MissingSection(name))?;
            open_section(name, kind, version, &blob)
        };
        let in_section = |name: &'static str| {
            move |error: StoreError| SaveError::Section {
                name: name.to_owned(),
                error,
            }
        };

        // A section this build does not know would be dropped.
        for name in store.section_names()? {
            let known = name == IDS_SECTION
                || name == TRACTS_SECTION
                || WORLD_SECTIONS.iter().any(|section| section.name == name);
            if name.starts_with(WORLD_SECTION_PREFIX) && !known {
                return Err(SaveError::UnknownSection(name));
            }
        }

        // The ids first: everything else is written against them.
        let image: SaveIdsImage =
            codec::from_bytes(&section(IDS_SECTION, IDS_BLOB_KIND, IDS_BLOB_VERSION)?)
                .map_err(|error| in_section(IDS_SECTION)(StoreError::from(error)))?;
        let first = &WORLD_SECTIONS[0];
        let shapes = realm_shapes(&section(first.name, first.kind, first.version)?)
            .map_err(in_section(first.name))?;
        let (ids, defs, missing) =
            SaveIds::bind(image, defs, &shapes).map_err(in_section(IDS_SECTION))?;

        let mut cosmos = Cosmos::new(Arc::clone(&defs));
        for missing in missing {
            cosmos.warn(SaveWarning {
                kind: missing.kind,
                id: missing.id,
                at: WarnAt::Ids,
            });
        }
        for part in WORLD_SECTIONS {
            let Some(blob) = store.section(part.name)? else {
                if part.required {
                    return Err(SaveError::MissingSection(part.name));
                }
                continue;
            };
            let body = open_section(part.name, part.kind, part.version, &blob)?;
            let mut map = ToDense::new(&defs, &ids);
            (part.read)(&mut cosmos, &body, &mut map).map_err(in_section(part.name))?;
            for missing in map.finish().map_err(in_section(part.name))? {
                cosmos.warn(SaveWarning {
                    kind: missing.kind,
                    id: missing.id,
                    at: WarnAt::Section(part.name),
                });
            }
        }

        // The tracts that were loaded.
        let loaded: Vec<LoadedEntry> = codec::from_bytes(&section(
            TRACTS_SECTION,
            TRACTS_BLOB_KIND,
            TRACTS_BLOB_VERSION,
        )?)
        .map_err(|error| in_section(TRACTS_SECTION)(StoreError::from(error)))?;
        let mut jobs: Vec<RealizeJob> = Vec::new();
        let mut columns: Vec<(GridId, TractPos, bool, Result<Column, usize>)> = Vec::new();
        for entry in loaded {
            let LoadedEntry {
                grid,
                tract,
                source,
                dirty,
            } = entry;
            let realm = cosmos.realm_of(grid).ok_or_else(|| {
                in_section(TRACTS_SECTION)(StoreError::Malformed(
                    "a loaded tract is in a grid that was not saved",
                ))
            })?;
            let stored = match store.get(realm, tract)? {
                Some(blob) => Some(decode_tract(&defs, &ids, &blob)?),
                None => None,
            };
            let mut cells = None;
            if let Some(found) = stored {
                for missing in found.missing {
                    cosmos.warn(SaveWarning {
                        kind: missing.kind,
                        id: missing.id,
                        at: WarnAt::Tract { realm, tract },
                    });
                }
                // An entity section handed in and not yet handed back
                // waits where it waited.
                if !found.entities.is_empty() {
                    cosmos.save.entities.insert((realm, tract), found.entities);
                }
                cells = found.cells;
            }
            let column = match (cells, source) {
                (Some(column), _) => Ok(column),
                (None, CellSource::Stored) => {
                    return Err(SaveError::MissingTract { realm, tract });
                }
                (None, CellSource::Generated) => {
                    let failed = |error: LoadError| SaveError::Tract {
                        realm,
                        tract,
                        error: Box::new(error),
                    };
                    let info = cosmos.realm_info(realm)?;
                    let job = cosmos
                        .realize_job(io.generators, &info, tract)
                        .map_err(failed)?;
                    jobs.push(job);
                    Err(jobs.len() - 1)
                }
            };
            columns.push((grid, tract, dirty, column));
        }
        let mut realised: Vec<Option<Column>> = realize_batch(&defs, io.generators.runner(), &jobs)
            .into_iter()
            .map(|made| Some(made.column))
            .collect();
        for (grid, tract, dirty, column) in columns {
            let mut column = match column {
                Ok(column) => column,
                Err(at) => {
                    realised
                        .get_mut(at)
                        .and_then(Option::take)
                        .ok_or(WorldError::BadValue {
                            what: "a batch of tracts",
                            why: "fewer tracts were realised than were asked for",
                        })?
                }
            };
            if dirty {
                column.mark_dirty();
            }
            cosmos.insert_column(grid, tract, column)?;
        }
        cosmos.save.ids = ids;
        Ok(cosmos)
    }
}
