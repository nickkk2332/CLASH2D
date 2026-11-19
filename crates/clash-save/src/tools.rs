//! What the save tools are built from (14 §12): checking a save and
//! listing the saves in a directory.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use clash_core::Hash128;
use clash_core::codec;
use clash_data::Defs;
use clash_gen::Generators;
use clash_world::{TRACT_BLOB_KIND, TractStore, decode_tract};

use crate::game::{
    SAVE_EXTENSION, SaveError, listed_tracts, load_game_from, load_warnings, open_existing,
    read_meta, read_meta_from,
};
use crate::meta::SaveMeta;

/// What checking a save found, when nothing is wrong with it.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Verified {
    /// The save's metadata.
    pub meta: SaveMeta,
    /// How many sections the save holds.
    pub sections: usize,
    /// How many tract blobs the save holds: each was read in full.
    pub tracts: usize,
    /// The state hash of what loads, which is the hash the save states.
    pub state_hash: Hash128,
    /// What differs from when the save was made: missing definitions
    /// that became placeholders, and mods that changed.
    pub warnings: Vec<String>,
}

/// Checks a saved game in a store: reads the frame of every tract blob,
/// loads the game, reads every tract blob in full against the save's
/// tables of ids, and compares the state hash of what loaded with the
/// hash the save states (14 §12, `save verify`).
///
/// The first thing wrong is the error, and it names the section or the
/// tract. The invariant checkers of 14 §7.3 are not built, so none is
/// run.
pub fn verify_store(
    store: &mut dyn TractStore,
    defs: Arc<Defs>,
    generators: Generators,
) -> Result<Verified, SaveError> {
    let meta = read_meta_from(store)?;
    let listed = listed_tracts(store)?;
    let blobs = |store: &dyn TractStore| -> Result<Vec<_>, SaveError> {
        let mut found = Vec::new();
        for &(realm, tract) in &listed {
            if let Some(blob) = store.get(realm, tract)? {
                found.push((realm, tract, blob));
            }
        }
        Ok(found)
    };
    // The frames first: a blob that has been written over is named
    // before loading trips on it.
    for (realm, tract, blob) in blobs(store)? {
        codec::open_blob(TRACT_BLOB_KIND, &blob).map_err(|error| SaveError::Tract {
            realm,
            tract,
            error: error.into(),
        })?;
    }
    let (mut sim, _) = load_game_from(store, defs, generators)?;
    {
        let cosmos = sim.truth().cosmos();
        for (realm, tract, blob) in blobs(store)? {
            decode_tract(cosmos.defs(), cosmos.save_ids(), &blob).map_err(|error| {
                SaveError::Tract {
                    realm,
                    tract,
                    error,
                }
            })?;
        }
    }
    let state_hash = sim.state_hash();
    if state_hash != meta.state_hash {
        return Err(SaveError::HashDiffers {
            saved: meta.state_hash,
            loaded: state_hash,
        });
    }
    let warnings = load_warnings(&mut sim, &meta);
    Ok(Verified {
        sections: store.section_names()?.len(),
        tracts: listed.len(),
        state_hash,
        warnings,
        meta,
    })
}

/// Checks a save file. See [`verify_store`].
pub fn verify(
    path: impl AsRef<Path>,
    defs: Arc<Defs>,
    generators: Generators,
) -> Result<Verified, SaveError> {
    let mut store = open_existing(path.as_ref())?;
    verify_store(&mut store, defs, generators)
}

/// One save file in a directory: its path, and its metadata or why it
/// could not be read.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct SaveEntry {
    /// The file.
    pub path: PathBuf,
    /// What it says about itself.
    pub meta: Result<SaveMeta, SaveError>,
}

/// The save files in a directory (those with the extension
/// [`SAVE_EXTENSION`]), in order of path, each with its metadata: what
/// a load screen lists. A file that is not a save is listed with the
/// reason. A directory that is not there holds no saves.
pub fn list_saves(dir: impl AsRef<Path>) -> Vec<SaveEntry> {
    let Ok(entries) = std::fs::read_dir(dir.as_ref()) else {
        return Vec::new();
    };
    let mut paths: Vec<PathBuf> = entries
        .filter_map(|entry| entry.ok().map(|entry| entry.path()))
        .filter(|path| {
            path.is_file()
                && path
                    .extension()
                    .is_some_and(|extension| extension == SAVE_EXTENSION)
        })
        .collect();
    paths.sort();
    paths
        .into_iter()
        .map(|path| SaveEntry {
            meta: read_meta(&path),
            path,
        })
        .collect()
}
