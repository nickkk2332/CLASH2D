//! Saving and loading: the seam the save crate joins (13 §9, 01 §11).
//!
//! **A seam.** The services `save`, `load` and `delete_save`, the list
//! `app.saves` and the flags `app.can_save` and `app.can_load` all go
//! through one trait, [`SaveBackend`]. The save crate is being written
//! beside this one, with `clash_save::save_game(&mut Sim, path, meta)`
//! and `clash_save::load_game(path, defs, generators)`. Until it is
//! joined the application uses [`NoSaves`], which lists nothing and
//! refuses each service with [`NOT_BUILT`].
//!
//! Joining it is a second implementation of the trait, a few lines
//! each, given to [`crate::app::App`] in place of `NoSaves`.

use std::path::Path;

use clash_scene::AppEntry;
use clash_sim::Sim;

use crate::boot::Loaded;

/// Why nothing can be saved or loaded yet.
pub const NOT_BUILT: &str = "saving is not built into this binary yet";

/// The directory saves are kept in, inside the user's data directory.
pub const SAVES_DIR: &str = "saves";

/// What is written beside a saved simulation, and read back with it:
/// what the application needs to carry a game on that the simulation
/// does not hold.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct SaveMeta {
    /// The qualified id of the start the game began from.
    pub start: String,
    /// What the player called the character.
    pub name: String,
}

/// A game read back from a save.
#[derive(Debug)]
pub struct SavedGame {
    /// The simulation, ready to carry on.
    pub sim: Sim,
    /// What was written beside it.
    pub meta: SaveMeta,
}

/// What saving and loading need of whatever keeps saves.
pub trait SaveBackend {
    /// Whether this backend can save and load at all. With one that
    /// cannot, `app.can_save` and `app.can_load` are false.
    fn available(&self) -> bool;

    /// The saves there are, newest first: `app.saves`.
    fn list(&self, dir: Option<&Path>) -> Vec<AppEntry>;

    /// Saves the simulation under a name, at a safe point, with what
    /// the application needs beside it.
    fn save(
        &mut self,
        dir: Option<&Path>,
        sim: &mut Sim,
        meta: &SaveMeta,
        name: &str,
    ) -> Result<(), String>;

    /// Loads the save with an id: a simulation over the loaded
    /// definitions, ready to carry on, and what was written beside it.
    fn load(
        &mut self,
        dir: Option<&Path>,
        loaded: &Loaded,
        save: &str,
    ) -> Result<SavedGame, String>;

    /// Deletes the save with an id.
    fn delete(&mut self, dir: Option<&Path>, save: &str) -> Result<(), String>;
}

/// The backend of a binary that cannot save: it lists nothing and
/// refuses everything, saying so.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct NoSaves;

impl SaveBackend for NoSaves {
    fn available(&self) -> bool {
        false
    }

    fn list(&self, _dir: Option<&Path>) -> Vec<AppEntry> {
        Vec::new()
    }

    fn save(
        &mut self,
        _dir: Option<&Path>,
        _sim: &mut Sim,
        _meta: &SaveMeta,
        _name: &str,
    ) -> Result<(), String> {
        Err(NOT_BUILT.to_owned())
    }

    fn load(
        &mut self,
        _dir: Option<&Path>,
        _loaded: &Loaded,
        _save: &str,
    ) -> Result<SavedGame, String> {
        Err(NOT_BUILT.to_owned())
    }

    fn delete(&mut self, _dir: Option<&Path>, _save: &str) -> Result<(), String> {
        Err(NOT_BUILT.to_owned())
    }
}
