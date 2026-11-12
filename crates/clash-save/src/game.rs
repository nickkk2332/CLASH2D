//! Saving and loading a whole game (01 §11; 02 §11.4).

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use clash_core::codec::{self, CodecError};
use clash_data::Defs;
use clash_gen::{GenDefs, Generators};
use clash_sim::{LoadError, Sim};
use clash_world::{
    Cosmos, MemoryStore, RealmId, RedbStore, StoreError, TractIo, TractPos, TractStore, WorldDefs,
};
use thiserror::Error;

use crate::meta::{FORMAT_VERSION, SaveMeta};

/// The section that holds a save's metadata ([`SaveMeta`]).
pub const META_SECTION: &str = "save.meta";
/// The kind of the blob in [`META_SECTION`].
pub const META_BLOB_KIND: [u8; 4] = *b"SMET";
/// The version of the blob in [`META_SECTION`].
pub const META_BLOB_VERSION: u32 = 1;

/// The section that lists the tracts a save holds a blob for.
pub const TRACTS_SECTION: &str = "save.tracts";
/// The kind of the blob in [`TRACTS_SECTION`].
pub const TRACTS_BLOB_KIND: [u8; 4] = *b"STRC";
/// The version of the blob in [`TRACTS_SECTION`].
pub const TRACTS_BLOB_VERSION: u32 = 1;

/// The section that holds the simulation's own state: the blob
/// [`Sim::save`] writes, with a kind and a version of its own.
pub const SIM_SECTION: &str = "sim.state";

/// The extension of a save file.
pub const SAVE_EXTENSION: &str = "c2save";

/// A tract in the list of [`TRACTS_SECTION`]: its realm, and its column
/// and row.
type TractEntry = (u32, i32, i32);

/// Why a game could not be saved or loaded. Never a panic: a file that
/// is missing, cut short or written over, and a section or a tract
/// that does not read, each say what and where.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[non_exhaustive]
pub enum SaveError {
    /// There is no file at the path.
    #[error("there is no save at `{0}`")]
    NoSuchFile(PathBuf),
    /// The file could not be made or opened.
    #[error("the save `{path}` could not be opened: {message}")]
    File {
        /// The file.
        path: PathBuf,
        /// What went wrong.
        message: String,
    },
    /// The store failed, or a tract blob in it is damaged.
    #[error(transparent)]
    Store(#[from] StoreError),
    /// The world could not be written or read back.
    #[error(transparent)]
    World(#[from] clash_world::SaveError),
    /// The simulation's own state could not be read back.
    #[error(transparent)]
    Sim(#[from] LoadError),
    /// A section is damaged or was written with another layout.
    #[error("section `{name}` of the save could not be used: {error}")]
    Section {
        /// The section.
        name: &'static str,
        /// What is wrong with it.
        error: CodecError,
    },
    /// The store does not hold a section every saved game has. It holds
    /// no saved game, or not all of one.
    #[error("the save has no section `{0}`")]
    MissingSection(&'static str),
    /// The save was written in a format this build does not read.
    #[error("the save is in format {found}, and this build reads format {expected}")]
    Format {
        /// The format the save states.
        found: u32,
        /// The format this build reads.
        expected: u32,
    },
    /// What the world or the generators are built from does not build
    /// from the definitions given. The text is the diagnostics.
    #[error("the definitions do not build:\n{0}")]
    Defs(String),
    /// A tract's blob is damaged.
    #[error("tract {tract} of realm {realm} could not be read: {error}")]
    Tract {
        /// The realm.
        realm: RealmId,
        /// The tract.
        tract: TractPos,
        /// What is wrong with it.
        error: StoreError,
    },
    /// The state that loads is not the state that was saved.
    #[error("the save states the hash {saved}, and what loads from it hashes to {loaded}")]
    HashDiffers {
        /// The hash in the save's metadata.
        saved: clash_core::Hash128,
        /// The hash of what was loaded.
        loaded: clash_core::Hash128,
    },
}

/// What a save wrote.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct SaveReport {
    /// What the world wrote: its sections and the loaded tracts that
    /// had changed.
    pub world: clash_world::SaveReport,
    /// How many tract blobs the save holds in all.
    pub tracts: usize,
    /// How many sections the save holds in all.
    pub sections: usize,
    /// The bytes written by this save: the world's, the simulation's
    /// and the metadata. Tract blobs carried over unchanged are not
    /// counted.
    pub bytes: usize,
}

/// The generators of `clash-gen`, the `standard` one included, bound to
/// a set of definitions: what [`load_game`] takes, and what a new
/// simulation is given with `Sim::with_io` so that every bundled realm
/// can be generated.
///
/// Fails if the world's tables or the generation definitions do not
/// build; the error holds the located diagnostics.
pub fn generators_for(defs: &Arc<Defs>) -> Result<Generators, SaveError> {
    let world =
        WorldDefs::from_defs(defs).map_err(|diagnostics| SaveError::Defs(diagnostics.render()))?;
    let built = GenDefs::from_defs(defs, &world)
        .map_err(|diagnostics| SaveError::Defs(diagnostics.render()))?;
    Ok(clash_gen::generators_with(&built))
}

/// Every tract the save must hold a blob for, if the simulation's own
/// store or the world has one: those that have been loaded at some
/// time, and those that are loaded now.
fn known_tracts(cosmos: &Cosmos) -> BTreeSet<(RealmId, TractPos)> {
    let mut known = BTreeSet::new();
    for realm in cosmos.realms() {
        for (region, record) in &realm.regions {
            for (index, tract) in record.tracts.iter().enumerate() {
                if tract.state.is_realized() {
                    known.insert((realm.id, region.tract(index)));
                }
            }
        }
        for tract in cosmos.loaded_tracts(realm.id) {
            known.insert((realm.id, tract));
        }
    }
    known
}

/// The tracts a store lists in [`TRACTS_SECTION`]. A store with no such
/// section lists none.
pub(crate) fn listed_tracts(store: &dyn TractStore) -> Result<Vec<(RealmId, TractPos)>, SaveError> {
    let Some(blob) = store.section(TRACTS_SECTION)? else {
        return Ok(Vec::new());
    };
    let entries: Vec<TractEntry> = read_section(TRACTS_SECTION, TRACTS_BLOB_KIND, &blob)?;
    Ok(entries
        .into_iter()
        .map(|(realm, x, y)| (RealmId::new(realm), TractPos::new(x, y)))
        .collect())
}

/// Opens a section's blob and reads its body.
fn read_section<T: serde::de::DeserializeOwned>(
    name: &'static str,
    kind: [u8; 4],
    blob: &[u8],
) -> Result<T, SaveError> {
    let wrap = |error: CodecError| SaveError::Section { name, error };
    let (_, body) = codec::open_blob(kind, blob).map_err(wrap)?;
    codec::from_bytes(&body).map_err(wrap)
}

/// Saves a whole game into a store and commits it (01 §11; 02 §11.4).
///
/// In the order the world asks for:
///
/// 1. The tract blobs the simulation holds for tracts that are not
///    loaded (those that were changed and then unloaded) are brought
///    into the store, and blobs the store holds that are no longer the
///    game's are taken out. The kernel keeps the entities of unloaded
///    tracts in its own state, so it has no entity section to hand in.
/// 2. `Cosmos::save_world` writes the world's sections and the loaded
///    tracts that have changed.
/// 3. The simulation's own state goes in as [`SIM_SECTION`], beside the
///    list of tracts ([`TRACTS_SECTION`]) and the metadata
///    ([`META_SECTION`]).
/// 4. One commit makes all of it durable, or none of it.
///
/// `meta` gives the name, the summary, the seed and the saver's own
/// bytes; the rest of the metadata is filled in from the simulation.
/// Nothing that is hashed changes, and nothing is unloaded: the
/// simulation goes on as if it had not been saved (01 R7).
///
/// Call it at a safe point: between two calls of `Sim::advance`. If it
/// fails, nothing has been committed.
pub fn save_game_into(
    sim: &mut Sim,
    store: &mut dyn TractStore,
    meta: &SaveMeta,
) -> Result<SaveReport, SaveError> {
    // 1. What is not loaded: as the simulation's own store has it.
    let known = known_tracts(sim.truth().cosmos());
    for (realm, tract) in listed_tracts(store).unwrap_or_default() {
        if !known.contains(&(realm, tract)) {
            store.delete(realm, tract);
        }
    }
    for &(realm, tract) in &known {
        let have = sim.store().get(realm, tract)?;
        if have != store.get(realm, tract)? {
            match have {
                Some(blob) => store.put(realm, tract, &blob),
                None => store.delete(realm, tract),
            }
        }
    }

    // 2. The world.
    let world = sim.world(|ctx| ctx.cosmos.save_world(store))?;

    // 3. The simulation, the list of tracts and the metadata.
    let wrap = |name: &'static str| move |error: CodecError| SaveError::Section { name, error };
    let state = sim.save().map_err(wrap(SIM_SECTION))?;
    store.put_section(SIM_SECTION, &state);

    let mut listed: Vec<TractEntry> = Vec::new();
    for &(realm, tract) in &known {
        if store.get(realm, tract)?.is_some() {
            listed.push((realm.raw(), tract.x, tract.y));
        }
    }
    let tracts = codec::encode_blob(TRACTS_BLOB_KIND, TRACTS_BLOB_VERSION, &listed)
        .map_err(wrap(TRACTS_SECTION))?;
    store.put_section(TRACTS_SECTION, &tracts);

    let mut meta = meta.clone();
    meta.format = FORMAT_VERSION;
    meta.stamp_defs(sim.defs());
    meta.tick = sim.now();
    meta.state_hash = sim.state_hash();
    let stamped =
        codec::encode_blob(META_BLOB_KIND, META_BLOB_VERSION, &meta).map_err(wrap(META_SECTION))?;
    store.put_section(META_SECTION, &stamped);

    // 4. All of it, or none.
    store.commit()?;
    Ok(SaveReport {
        world,
        tracts: listed.len(),
        sections: store.section_names()?.len(),
        bytes: world.bytes + state.len() + tracts.len() + stamped.len(),
    })
}

/// Saves a whole game into a file: one `redb` database for each save
/// (02 §11.5). The file is made if it is not there, with the
/// directories above it; a save that is there is replaced, in one
/// transaction, so the file holds all of this save or all of the one
/// before it. See [`save_game_into`].
pub fn save_game(
    sim: &mut Sim,
    path: impl AsRef<Path>,
    meta: &SaveMeta,
) -> Result<SaveReport, SaveError> {
    let path = path.as_ref();
    if let Some(parent) = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        std::fs::create_dir_all(parent).map_err(|error| SaveError::File {
            path: path.to_path_buf(),
            message: error.to_string(),
        })?;
    }
    let mut store = open(path)?;
    save_game_into(sim, &mut store, meta)
}

/// Opens a save file, which is made if it is not there.
fn open(path: &Path) -> Result<RedbStore, SaveError> {
    RedbStore::open(path).map_err(|error| SaveError::File {
        path: path.to_path_buf(),
        message: error.to_string(),
    })
}

/// Opens a save file that must be there.
pub(crate) fn open_existing(path: &Path) -> Result<RedbStore, SaveError> {
    if !path.is_file() {
        return Err(SaveError::NoSuchFile(path.to_path_buf()));
    }
    open(path)
}

/// A save's metadata, out of a store, without loading the game.
pub fn read_meta_from(store: &dyn TractStore) -> Result<SaveMeta, SaveError> {
    let blob = store
        .section(META_SECTION)?
        .ok_or(SaveError::MissingSection(META_SECTION))?;
    let meta: SaveMeta = read_section(META_SECTION, META_BLOB_KIND, &blob)?;
    if meta.format != FORMAT_VERSION {
        return Err(SaveError::Format {
            found: meta.format,
            expected: FORMAT_VERSION,
        });
    }
    Ok(meta)
}

/// A save file's metadata, without loading the game: for a load screen.
pub fn read_meta(path: impl AsRef<Path>) -> Result<SaveMeta, SaveError> {
    read_meta_from(&open_existing(path.as_ref())?)
}

/// Loads a whole game out of a store (02 §11.4), which is read and not
/// changed.
///
/// `defs` are the definitions loaded now and `generators` the
/// generators bound to them ([`generators_for`]); the world's tables
/// are the ones the generators were bound to. In order: the world is
/// loaded, with a placeholder for every definition the save names and
/// the mods no longer supply; the simulation's state is put over it,
/// timeline and all, from the same save; the simulation is given the
/// generators and a store in memory holding every tract blob of the
/// save; and every entity is placed in the world again.
///
/// What comes back hashes equal to what was saved and goes on as it
/// would have (01 R7). What is not state is as a new simulation has
/// it: wake-up handlers, event handlers and event types that were
/// registered on top are registered again by whoever registered them.
/// [`load_warnings`] says what differs from when the save was made.
pub fn load_game_from(
    store: &mut dyn TractStore,
    defs: Arc<Defs>,
    generators: Generators,
) -> Result<(Sim, SaveMeta), SaveError> {
    let meta = read_meta_from(store)?;
    let world = Arc::clone(generators.defs());
    let cosmos = Cosmos::load_world(&world, &mut TractIo::new(&generators, store))?;
    let state = store
        .section(SIM_SECTION)?
        .ok_or(SaveError::MissingSection(SIM_SECTION))?;
    let sim = Sim::load(defs, cosmos, &state)?;

    // The store the simulation unloads into and loads from: every tract
    // blob of the save, in memory.
    let mut working = MemoryStore::new();
    for (realm, tract) in listed_tracts(store)? {
        if let Some(blob) = store.get(realm, tract)? {
            working.put(realm, tract, &blob);
        }
    }
    let mut sim = sim.with_io(generators, Box::new(working));
    // The world does not save who stands where: placed again, into a
    // sink that is thrown away.
    sim.reindex();
    Ok((sim, meta))
}

/// Loads a whole game out of a save file. See [`load_game_from`].
///
/// A file that is not there, is not a save, or has been cut short or
/// written over is an error, never a panic.
pub fn load_game(
    path: impl AsRef<Path>,
    defs: Arc<Defs>,
    generators: Generators,
) -> Result<(Sim, SaveMeta), SaveError> {
    let mut store = open_existing(path.as_ref())?;
    load_game_from(&mut store, defs, generators)
}

/// What a player should be told after a load: each definition the save
/// names that the loaded mods do not supply, with where it was met and
/// that a placeholder stands in for it (02 §11.2), and what differs
/// between the mods the save was made with and those loaded now. Empty
/// when the save was made with the definitions it was loaded with.
///
/// The world's warnings are taken: a second call gives only what was
/// met since, as tracts that hold missing definitions are read.
pub fn load_warnings(sim: &mut Sim, meta: &SaveMeta) -> Vec<String> {
    let mut warnings = meta.differences(sim.defs());
    let world = sim.world(|ctx| ctx.cosmos.take_save_warnings());
    warnings.extend(world.iter().map(ToString::to_string));
    warnings
}
