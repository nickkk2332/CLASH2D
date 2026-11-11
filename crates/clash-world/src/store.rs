//! Where a save is kept (02 §11): the [`TractStore`] trait, an in-memory
//! store, and the tract blob.
//!
//! A store holds two kinds of thing, each a framed blob (01 §11.1):
//!
//! - **Tract blobs**, by realm and tract: the cells of a tract that has
//!   been changed, and the simulation's entity section for the tract. A
//!   tract that has never been changed and holds no entities has no blob
//!   (the pristine policy): its cells are generated again.
//! - **Sections**, by name: everything of a save that is not a tract.
//!   The world writes those whose names begin with `world.`
//!   (see [`crate::save`]); the simulation writes its own beside them.
//!
//! Two stores are built: [`MemoryStore`] here, for tests and tools and
//! for a game that is never saved, and [`RedbStore`](crate::RedbStore),
//! one database file for each save.
//!
//! ```
//! use clash_world::{MemoryStore, RealmId, TractPos, TractStore};
//!
//! let mut store = MemoryStore::new();
//! let (realm, tract) = (RealmId::new(0), TractPos::new(2, 3));
//! assert_eq!(store.get(realm, tract), Ok(None));
//! store.put(realm, tract, b"cells");
//! assert_eq!(store.get(realm, tract), Ok(Some(b"cells".to_vec())));
//! store.delete(realm, tract);
//! store.put_section("sim.clock", b"noon");
//! assert_eq!(store.section_names(), Ok(vec!["sim.clock".to_owned()]));
//! assert!(store.is_empty());
//! ```
//!
//! # The tract blob
//!
//! Kind `TRCT`, version 2. The body is, in the engine's encoding:
//!
//! | Field | Holds |
//! |-------|-------|
//! | `cells` | Optional. The column: its two fills, the chunk height of its first stored chunk, and its chunks without those at either end that hold only the fill. Each chunk is its three element layers with compacted palettes and its four sparse layers. Element states are written by save id ([`SaveIds`]). Absent for a tract whose cells are generated again |
//! | `entities` | The simulation's entity section: bytes the world neither reads nor drops |
//!
//! Equal columns written against equal tables give equal bodies. Version
//! 1, which held dense ids and no entity section, is refused: it was
//! never written to disk.

use std::collections::BTreeMap;

use clash_core::{CodecError, codec};
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::cosmos::RealmId;
use crate::defs::{DefKind, WorldDefs};
use crate::grid::{Column, ColumnImage};
use crate::pos::TractPos;
use crate::saveid::{MapIds, MissingDef, SaveIds, ToDense, ToSave};

/// Why the store, or a blob out of it, could not be used.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[non_exhaustive]
pub enum StoreError {
    /// The store itself failed: its file, its transaction.
    #[error("the save's store failed: {0}")]
    Backend(String),
    /// A blob is damaged or is not what it should be: its frame or its
    /// body could not be read (01 §11.1).
    #[error("a stored blob could not be read: {0}")]
    Codec(#[from] CodecError),
    /// A blob was written by a layout this build does not read.
    #[error("a stored blob has layout version {found}, and this build reads version {expected}")]
    Version {
        /// The version the blob states.
        found: u32,
        /// The version this build reads.
        expected: u32,
    },
    /// A blob reads, and what it holds cannot be.
    #[error("a stored blob holds something that cannot be: {0}")]
    Malformed(&'static str),
    /// A save names more definitions of one type than can be given ids,
    /// counting the placeholders for those that are missing.
    #[error("the save names more {0} definitions than there are ids for")]
    TooManyDefinitions(DefKind),
}

/// Where a save is kept: tract blobs by realm and tract, and named
/// sections for the rest (02 §11).
///
/// `put` and `delete` take effect for later reads at once; `commit`
/// makes everything done since the last commit durable, all of it or
/// none. What was not committed is gone when the store is dropped.
///
/// Commit only what [`Cosmos::save_world`](crate::Cosmos::save_world)
/// has just completed: tract blobs name definitions through the tables
/// that `save_world` writes, and a store committed without them holds
/// blobs it cannot read back.
pub trait TractStore {
    /// The blob of a tract, if one is stored.
    fn get(&self, realm: RealmId, pos: TractPos) -> Result<Option<Vec<u8>>, StoreError>;
    /// Stores a tract's blob, replacing any it had.
    fn put(&mut self, realm: RealmId, pos: TractPos, blob: &[u8]);
    /// Forgets a tract's blob.
    fn delete(&mut self, realm: RealmId, pos: TractPos);
    /// Makes what was put and deleted durable. Atomic.
    fn commit(&mut self) -> Result<(), StoreError>;
    /// A named section, if one is stored.
    fn section(&self, name: &str) -> Result<Option<Vec<u8>>, StoreError>;
    /// Stores a named section, replacing any of that name.
    fn put_section(&mut self, name: &str, blob: &[u8]);
    /// Forgets a named section.
    fn delete_section(&mut self, name: &str);
    /// The names of the sections stored, in order.
    fn section_names(&self) -> Result<Vec<String>, StoreError>;
}

/// A store that keeps its blobs in memory. For tests and tools, and for
/// a game that is never saved.
#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct MemoryStore {
    blobs: BTreeMap<(RealmId, TractPos), Vec<u8>>,
    sections: BTreeMap<String, Vec<u8>>,
    commits: u64,
}

impl MemoryStore {
    /// An empty store.
    pub fn new() -> MemoryStore {
        MemoryStore::default()
    }

    /// How many tracts are stored.
    pub fn len(&self) -> usize {
        self.blobs.len()
    }

    /// Whether no tract is stored.
    pub fn is_empty(&self) -> bool {
        self.blobs.is_empty()
    }

    /// The tracts stored, by realm and then row by row.
    pub fn tracts(&self) -> impl Iterator<Item = (RealmId, TractPos)> + '_ {
        self.blobs.keys().copied()
    }

    /// How often the store has been committed.
    pub fn commits(&self) -> u64 {
        self.commits
    }

    /// The bytes held in all: every tract blob and every section.
    pub fn stored_bytes(&self) -> usize {
        self.blobs.values().map(Vec::len).sum::<usize>()
            + self.sections.values().map(Vec::len).sum::<usize>()
    }
}

impl TractStore for MemoryStore {
    fn get(&self, realm: RealmId, pos: TractPos) -> Result<Option<Vec<u8>>, StoreError> {
        Ok(self.blobs.get(&(realm, pos)).cloned())
    }

    fn put(&mut self, realm: RealmId, pos: TractPos, blob: &[u8]) {
        self.blobs.insert((realm, pos), blob.to_vec());
    }

    fn delete(&mut self, realm: RealmId, pos: TractPos) {
        self.blobs.remove(&(realm, pos));
    }

    fn commit(&mut self) -> Result<(), StoreError> {
        self.commits = self.commits.wrapping_add(1);
        Ok(())
    }

    fn section(&self, name: &str) -> Result<Option<Vec<u8>>, StoreError> {
        Ok(self.sections.get(name).cloned())
    }

    fn put_section(&mut self, name: &str, blob: &[u8]) {
        self.sections.insert(name.to_owned(), blob.to_vec());
    }

    fn delete_section(&mut self, name: &str) {
        self.sections.remove(name);
    }

    fn section_names(&self) -> Result<Vec<String>, StoreError> {
        Ok(self.sections.keys().cloned().collect())
    }
}

/// The kind of a tract blob (01 §11.1).
pub const TRACT_BLOB_KIND: [u8; 4] = *b"TRCT";

/// The version of a tract blob's body this build writes and reads.
pub const TRACT_BLOB_VERSION: u32 = 2;

/// Bytes written as bytes: a length and then the bytes, in one piece.
pub(crate) mod opaque {
    use serde::de::{Error, SeqAccess, Visitor};
    use serde::{Deserializer, Serializer};

    pub(crate) fn serialize<S: Serializer>(bytes: &[u8], serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_bytes(bytes)
    }

    pub(crate) fn deserialize<'de, D: Deserializer<'de>>(
        deserializer: D,
    ) -> Result<Vec<u8>, D::Error> {
        struct Bytes;

        impl<'de> Visitor<'de> for Bytes {
            type Value = Vec<u8>;

            fn expecting(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
                f.write_str("bytes")
            }

            fn visit_bytes<E: Error>(self, bytes: &[u8]) -> Result<Vec<u8>, E> {
                Ok(bytes.to_vec())
            }

            fn visit_byte_buf<E: Error>(self, bytes: Vec<u8>) -> Result<Vec<u8>, E> {
                Ok(bytes)
            }

            // What a format that describes itself gives for bytes.
            fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<Vec<u8>, A::Error> {
                let mut bytes = Vec::new();
                while let Some(byte) = seq.next_element()? {
                    bytes.push(byte);
                }
                Ok(bytes)
            }
        }

        deserializer.deserialize_byte_buf(Bytes)
    }
}

/// The body of a tract blob (see the module documentation).
#[derive(Serialize, Deserialize)]
struct TractBody {
    cells: Option<ColumnImage>,
    #[serde(with = "opaque")]
    entities: Vec<u8>,
}

/// What a tract blob holds, read back.
#[derive(Clone, Debug)]
pub struct StoredTract {
    /// The tract's cells, with their caches built and nothing marked as
    /// changed. `None` for a tract whose cells are generated again.
    pub cells: Option<Column>,
    /// The simulation's entity section, as it was written.
    pub entities: Vec<u8>,
    /// The definitions the cells are made from that no loaded mod
    /// supplies: they are placeholders now (04 §12.3).
    pub missing: Vec<MissingDef>,
}

/// Writes a tract blob: the cells, if they are to be stored, with their
/// states by save id, and the simulation's entity section, framed and
/// compressed (01 §11.1). Occupants, caches and revisions are not
/// written.
///
/// Writing may add to `ids`: the tables must be saved with the store
/// the blob goes to.
pub fn encode_tract(
    defs: &WorldDefs,
    ids: &mut SaveIds,
    cells: Option<&Column>,
    entities: &[u8],
) -> Result<Vec<u8>, StoreError> {
    encode_image(defs, ids, cells.map(Column::image), entities)
}

/// Writes a tract blob from a column's stored form, which still holds
/// dense ids.
pub(crate) fn encode_image(
    defs: &WorldDefs,
    ids: &mut SaveIds,
    cells: Option<ColumnImage>,
    entities: &[u8],
) -> Result<Vec<u8>, StoreError> {
    let cells = match cells {
        Some(mut image) => {
            let mut map = ToSave::new(defs, ids);
            image.map_ids(&mut map);
            map.finish()?;
            Some(image)
        }
        None => None,
    };
    let body = TractBody {
        cells,
        entities: entities.to_vec(),
    };
    Ok(codec::encode_blob(
        TRACT_BLOB_KIND,
        TRACT_BLOB_VERSION,
        &body,
    )?)
}

/// Reads a tract blob back. A blob that fails any check is an error,
/// never a panic: a damaged frame, another layout, a state named by a
/// save id the tables do not hold.
///
/// `defs` are the definitions `ids` were bound to: those of the cosmos
/// the tract belongs to.
pub fn decode_tract(
    defs: &WorldDefs,
    ids: &SaveIds,
    blob: &[u8],
) -> Result<StoredTract, StoreError> {
    let (found, body) = codec::open_blob(TRACT_BLOB_KIND, blob)?;
    if found != TRACT_BLOB_VERSION {
        return Err(StoreError::Version {
            found,
            expected: TRACT_BLOB_VERSION,
        });
    }
    let body: TractBody = codec::from_bytes(&body)?;
    let mut missing = Vec::new();
    let cells = match body.cells {
        Some(mut image) => {
            let mut map = ToDense::new(defs, ids);
            image.map_ids(&mut map);
            missing = map.finish()?;
            Some(Column::from_image(defs, image).ok_or(StoreError::Malformed(
                "more chunks than a column stacks, or chunks past the highest storey",
            ))?)
        }
        None => None,
    };
    Ok(StoredTract {
        cells,
        entities: body.entities,
        missing,
    })
}

/// A tract blob with its entity section taken out, or `None` if nothing
/// is left to store: the blob held no cells.
pub(crate) fn without_entities(blob: &[u8]) -> Result<Option<Vec<u8>>, StoreError> {
    let (version, body) = codec::open_blob(TRACT_BLOB_KIND, blob)?;
    if version != TRACT_BLOB_VERSION {
        return Err(StoreError::Version {
            found: version,
            expected: TRACT_BLOB_VERSION,
        });
    }
    let mut body: TractBody = codec::from_bytes(&body)?;
    if body.cells.is_none() {
        return Ok(None);
    }
    body.entities.clear();
    Ok(Some(codec::encode_blob(
        TRACT_BLOB_KIND,
        TRACT_BLOB_VERSION,
        &body,
    )?))
}
