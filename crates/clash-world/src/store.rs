//! Where the cells of changed tracts are kept while they are not loaded
//! (02 §11): the [`TractStore`] trait, an in-memory store, and the two
//! functions that turn a column into a blob and back.
//!
//! Unloading a tract that has changed writes its column to the store;
//! loading it reads the column back. A tract that has never been changed
//! is not stored: it is generated again (the pristine policy).
//!
//! What is here is the simplest correct form. Package W10 owns
//! persistence and replaces [`encode_column`] and [`decode_column`]
//! with the final blob layout: states written by save id through a
//! save-wide table of string ids, the simulation's entity section, and
//! the generator fingerprint of the pristine policy. It also adds the
//! on-disk store. Until then a blob holds runtime ids and is good for
//! the mod set that wrote it.
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
//! assert!(store.is_empty());
//! ```

use std::collections::BTreeMap;

use clash_core::{CodecError, codec};
use thiserror::Error;

use crate::cosmos::RealmId;
use crate::defs::WorldDefs;
use crate::grid::{Column, ColumnImage};
use crate::pos::TractPos;

/// Why the tract store, or a blob out of it, could not be used.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[non_exhaustive]
pub enum StoreError {
    /// The store itself failed: its file, its transaction.
    #[error("the tract store failed: {0}")]
    Backend(String),
    /// A blob is damaged or is not a tract: its frame or its body could
    /// not be read (01 §11.1).
    #[error("a stored tract could not be read: {0}")]
    Codec(#[from] CodecError),
    /// A blob was written by a layout this build does not read.
    #[error("a stored tract has layout version {found}, and this build reads version {expected}")]
    Version {
        /// The version the blob states.
        found: u32,
        /// The version this build reads.
        expected: u32,
    },
    /// A blob reads, and what it holds cannot be a column.
    #[error("a stored tract holds something that cannot be: {0}")]
    Malformed(&'static str),
}

/// Where tract blobs are kept, by realm and tract (02 §11).
///
/// `put` and `delete` take effect for later `get`s at once; `commit`
/// makes everything done since the last commit durable, all of it or
/// none.
pub trait TractStore {
    /// The blob of a tract, if one is stored.
    fn get(&self, realm: RealmId, pos: TractPos) -> Result<Option<Vec<u8>>, StoreError>;
    /// Stores a tract's blob, replacing any it had.
    fn put(&mut self, realm: RealmId, pos: TractPos, blob: &[u8]);
    /// Forgets a tract's blob.
    fn delete(&mut self, realm: RealmId, pos: TractPos);
    /// Makes what was put and deleted durable. Atomic.
    fn commit(&mut self) -> Result<(), StoreError>;
}

/// A tract store that keeps its blobs in memory. For tests and tools,
/// and for a game that is never saved.
#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct MemoryStore {
    blobs: BTreeMap<(RealmId, TractPos), Vec<u8>>,
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
}

/// The kind of a tract blob (01 §11.1).
pub const TRACT_BLOB_KIND: [u8; 4] = *b"TRCT";

/// The version of a tract blob's body this build writes and reads.
pub const TRACT_BLOB_VERSION: u32 = 2;

/// Writes a column as a tract blob: its fills and the chunks that say
/// something the fills do not, with compacted palettes and the sparse
/// layers, framed and compressed (01 §11.1). Occupants, caches and
/// revisions are not written.
pub fn encode_column(column: &Column) -> Result<Vec<u8>, StoreError> {
    let blob = codec::encode_blob(TRACT_BLOB_KIND, TRACT_BLOB_VERSION, &column.image())?;
    Ok(blob)
}

/// Reads a column back out of a tract blob. A blob that fails any check
/// is an error, never a panic.
pub fn decode_column(defs: &WorldDefs, blob: &[u8]) -> Result<Column, StoreError> {
    let (found, body) = codec::open_blob(TRACT_BLOB_KIND, blob)?;
    if found != TRACT_BLOB_VERSION {
        return Err(StoreError::Version {
            found,
            expected: TRACT_BLOB_VERSION,
        });
    }
    let image: ColumnImage = codec::from_bytes(&body)?;
    Column::from_image(defs, image).ok_or(StoreError::Malformed(
        "more chunks than a column stacks, or chunks past the highest storey",
    ))
}
