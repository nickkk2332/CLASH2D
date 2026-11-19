//! A whole save as one run of bytes: the in-memory variant, for tests,
//! for the console's `save` and `load`, and for the snapshots a tool
//! keeps.
//!
//! The bytes are a blob of kind [`PACK_BLOB_KIND`] holding what a store
//! holds: every tract blob and every section. Nothing touches a file.

use std::sync::Arc;

use clash_core::codec::{self, CodecError};
use clash_data::Defs;
use clash_gen::Generators;
use clash_sim::Sim;
use clash_world::{MemoryStore, RealmId, TractPos, TractStore};
use serde::{Deserialize, Serialize};

use crate::game::{SaveError, load_game_from, save_game_into};
use crate::meta::SaveMeta;

/// The kind of the blob a packed save is.
pub const PACK_BLOB_KIND: [u8; 4] = *b"SPAK";
/// The version of a packed save's layout.
pub const PACK_BLOB_VERSION: u32 = 1;

/// The name a packed save's own frame is reported under.
const PACK: &str = "the packed save";

/// What a store holds, in order.
#[derive(Serialize, Deserialize)]
struct Packed {
    /// The tract blobs: realm, column, row, bytes.
    tracts: Vec<(u32, i32, i32, Vec<u8>)>,
    /// The sections: name, bytes.
    sections: Vec<(String, Vec<u8>)>,
}

/// Everything a store in memory holds, as bytes.
pub fn pack_store(store: &MemoryStore) -> Result<Vec<u8>, SaveError> {
    let mut packed = Packed {
        tracts: Vec::new(),
        sections: Vec::new(),
    };
    for (realm, tract) in store.tracts() {
        if let Some(blob) = store.get(realm, tract)? {
            packed.tracts.push((realm.raw(), tract.x, tract.y, blob));
        }
    }
    for name in store.section_names()? {
        if let Some(blob) = store.section(&name)? {
            packed.sections.push((name, blob));
        }
    }
    codec::encode_blob(PACK_BLOB_KIND, PACK_BLOB_VERSION, &packed).map_err(in_pack)
}

/// A store in memory holding what [`pack_store`] wrote.
pub fn unpack_store(bytes: &[u8]) -> Result<MemoryStore, SaveError> {
    let (version, body) = codec::open_blob(PACK_BLOB_KIND, bytes).map_err(in_pack)?;
    if version != PACK_BLOB_VERSION {
        return Err(SaveError::Format {
            found: version,
            expected: PACK_BLOB_VERSION,
        });
    }
    let packed: Packed = codec::from_bytes(&body).map_err(in_pack)?;
    let mut store = MemoryStore::new();
    for (realm, x, y, blob) in &packed.tracts {
        store.put(RealmId::new(*realm), TractPos::new(*x, *y), blob);
    }
    for (name, blob) in &packed.sections {
        store.put_section(name, blob);
    }
    Ok(store)
}

fn in_pack(error: CodecError) -> SaveError {
    SaveError::Section { name: PACK, error }
}

/// Saves a whole game as bytes: [`save_game_into`] a store in memory,
/// packed. Nothing that is hashed changes.
pub fn save_game_bytes(sim: &mut Sim, meta: &SaveMeta) -> Result<Vec<u8>, SaveError> {
    let mut store = MemoryStore::new();
    save_game_into(sim, &mut store, meta)?;
    pack_store(&store)
}

/// Loads a whole game from the bytes [`save_game_bytes`] gave. See
/// [`load_game_from`]. Bytes that are not a packed save, or are damaged,
/// are an error.
pub fn load_game_bytes(
    bytes: &[u8],
    defs: Arc<Defs>,
    generators: Generators,
) -> Result<(Sim, SaveMeta), SaveError> {
    let mut store = unpack_store(bytes)?;
    load_game_from(&mut store, defs, generators)
}

/// The metadata of a packed save, without loading the game.
pub fn read_meta_bytes(bytes: &[u8]) -> Result<SaveMeta, SaveError> {
    crate::game::read_meta_from(&unpack_store(bytes)?)
}
