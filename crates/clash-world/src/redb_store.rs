//! [`RedbStore`]: a save on disk, one database file for each save
//! (02 §11; 01 §13).
//!
//! The file is a `redb` database with two tables: tract blobs by realm
//! and tract, and sections by name. What is put and deleted is kept in
//! memory until [`commit`](TractStore::commit), which writes all of it
//! in one transaction: the file holds either everything of a save or
//! everything of the one before it, whatever happens in between. A game
//! that is not saved leaves its file as it was.
//!
//! Every blob in the file is framed and checksummed by the engine
//! (01 §11.1), and the file is checked when it is opened, so a file
//! that has been cut short or written over is an error value and never
//! a panic.
//!
//! The simulation never waits on this (01 R5): the store is read when
//! tracts are loaded and written when the game is saved, both at safe
//! points.
//!
//! ```no_run
//! use clash_world::{RealmId, RedbStore, TractPos, TractStore};
//!
//! let mut store = RedbStore::open("saves/first.c2save").unwrap();
//! store.put(RealmId::new(0), TractPos::new(4, 2), b"...");
//! store.put_section("sim.timeline", b"...");
//! store.commit().unwrap();
//! ```

use std::collections::BTreeMap;
use std::path::Path;

use redb::{Database, ReadableDatabase, ReadableTable, TableDefinition, TableError};

use crate::cosmos::RealmId;
use crate::pos::TractPos;
use crate::store::{StoreError, TractStore};

/// A tract's key in the file: its realm, then its row and its column,
/// so that the tracts of a realm are kept row by row.
type TractKey = (u32, i32, i32);

/// The table of tract blobs.
const TRACTS: TableDefinition<TractKey, &[u8]> = TableDefinition::new("tracts");

/// The table of named sections.
const SECTIONS: TableDefinition<&str, &[u8]> = TableDefinition::new("sections");

/// A failure of the database, as the store reports it.
fn backend(error: impl core::fmt::Display) -> StoreError {
    StoreError::Backend(error.to_string())
}

fn key(realm: RealmId, pos: TractPos) -> TractKey {
    (realm.raw(), pos.y, pos.x)
}

/// A save on disk: a [`TractStore`] over one `redb` database file.
pub struct RedbStore {
    db: Database,
    /// The tract blobs put (`Some`) and deleted (`None`) since the last
    /// commit.
    tracts: BTreeMap<TractKey, Option<Vec<u8>>>,
    /// The sections put and deleted since the last commit.
    sections: BTreeMap<String, Option<Vec<u8>>>,
}

impl core::fmt::Debug for RedbStore {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("RedbStore")
            .field("pending_tracts", &self.tracts.len())
            .field("pending_sections", &self.sections.len())
            .finish_non_exhaustive()
    }
}

impl RedbStore {
    /// Opens a save file, or makes an empty one if there is no file at
    /// the path. The file is checked as it is opened: one that is not a
    /// save, or has been damaged, is an error.
    pub fn open(path: impl AsRef<Path>) -> Result<RedbStore, StoreError> {
        let db = Database::builder()
            .set_check_integrity_on_open(true)
            .create(path)
            .map_err(backend)?;
        Ok(RedbStore {
            db,
            tracts: BTreeMap::new(),
            sections: BTreeMap::new(),
        })
    }

    /// How many tract blobs and sections have been put or deleted since
    /// the last commit.
    pub fn pending(&self) -> usize {
        self.tracts.len() + self.sections.len()
    }

    /// Forgets everything put and deleted since the last commit: the
    /// store is again what the file holds.
    pub fn discard(&mut self) {
        self.tracts.clear();
        self.sections.clear();
    }

    /// How many tract blobs the save holds, counting what has been put
    /// and deleted since the last commit.
    pub fn tract_count(&self) -> Result<usize, StoreError> {
        let read = self.db.begin_read().map_err(backend)?;
        let mut keys: Vec<TractKey> = match read.open_table(TRACTS) {
            Ok(table) => {
                let mut keys = Vec::new();
                for entry in table.iter().map_err(backend)? {
                    keys.push(entry.map_err(backend)?.0.value());
                }
                keys
            }
            Err(TableError::TableDoesNotExist(_)) => Vec::new(),
            Err(error) => return Err(backend(error)),
        };
        keys.retain(|key| !self.tracts.contains_key(key));
        let pending = self.tracts.values().filter(|blob| blob.is_some()).count();
        Ok(keys.len() + pending)
    }
}

impl TractStore for RedbStore {
    fn get(&self, realm: RealmId, pos: TractPos) -> Result<Option<Vec<u8>>, StoreError> {
        let key = key(realm, pos);
        if let Some(pending) = self.tracts.get(&key) {
            return Ok(pending.clone());
        }
        let read = self.db.begin_read().map_err(backend)?;
        match read.open_table(TRACTS) {
            Ok(table) => Ok(table
                .get(key)
                .map_err(backend)?
                .map(|blob| blob.value().to_vec())),
            // Nothing has ever been committed.
            Err(TableError::TableDoesNotExist(_)) => Ok(None),
            Err(error) => Err(backend(error)),
        }
    }

    fn put(&mut self, realm: RealmId, pos: TractPos, blob: &[u8]) {
        self.tracts.insert(key(realm, pos), Some(blob.to_vec()));
    }

    fn delete(&mut self, realm: RealmId, pos: TractPos) {
        self.tracts.insert(key(realm, pos), None);
    }

    fn commit(&mut self) -> Result<(), StoreError> {
        let write = self.db.begin_write().map_err(backend)?;
        {
            let mut tracts = write.open_table(TRACTS).map_err(backend)?;
            for (key, blob) in &self.tracts {
                match blob {
                    Some(blob) => {
                        tracts.insert(*key, blob.as_slice()).map_err(backend)?;
                    }
                    None => {
                        tracts.remove(*key).map_err(backend)?;
                    }
                }
            }
            let mut sections = write.open_table(SECTIONS).map_err(backend)?;
            for (name, blob) in &self.sections {
                match blob {
                    Some(blob) => {
                        sections
                            .insert(name.as_str(), blob.as_slice())
                            .map_err(backend)?;
                    }
                    None => {
                        sections.remove(name.as_str()).map_err(backend)?;
                    }
                }
            }
        }
        // If the transaction is dropped before this, nothing of it is
        // in the file, and what was put is still here to try again.
        write.commit().map_err(backend)?;
        self.discard();
        Ok(())
    }

    fn section(&self, name: &str) -> Result<Option<Vec<u8>>, StoreError> {
        if let Some(pending) = self.sections.get(name) {
            return Ok(pending.clone());
        }
        let read = self.db.begin_read().map_err(backend)?;
        match read.open_table(SECTIONS) {
            Ok(table) => Ok(table
                .get(name)
                .map_err(backend)?
                .map(|blob| blob.value().to_vec())),
            Err(TableError::TableDoesNotExist(_)) => Ok(None),
            Err(error) => Err(backend(error)),
        }
    }

    fn put_section(&mut self, name: &str, blob: &[u8]) {
        self.sections.insert(name.to_owned(), Some(blob.to_vec()));
    }

    fn delete_section(&mut self, name: &str) {
        self.sections.insert(name.to_owned(), None);
    }

    fn section_names(&self) -> Result<Vec<String>, StoreError> {
        let read = self.db.begin_read().map_err(backend)?;
        let mut names: Vec<String> = match read.open_table(SECTIONS) {
            Ok(table) => {
                let mut names = Vec::new();
                for entry in table.iter().map_err(backend)? {
                    names.push(entry.map_err(backend)?.0.value().to_owned());
                }
                names
            }
            Err(TableError::TableDoesNotExist(_)) => Vec::new(),
            Err(error) => return Err(backend(error)),
        };
        names.retain(|name| !self.sections.contains_key(name));
        names.extend(
            self.sections
                .iter()
                .filter(|(_, blob)| blob.is_some())
                .map(|(name, _)| name.clone()),
        );
        names.sort();
        Ok(names)
    }
}
