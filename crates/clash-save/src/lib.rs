//! Saving and loading for CLASH2D: the whole-game save, its format
//! version, and the console's host over the real simulation. A save
//! holds the mod list, the id table, the cosmos per tract, entities,
//! the timeline, variables and the random streams, and loading one is
//! invisible to the simulation: running on from a loaded save gives
//! the same state hash as never having saved (01 R7). Specified in
//! `docs/01-architecture.md` section 11 and `docs/02-world-model.md`
//! section 11.4.
//!
//! # Saving and loading a game
//!
//! | Need | Use |
//! |------|-----|
//! | To save into a file | [`save_game`] with a [`SaveMeta`] |
//! | To load from a file | [`load_game`], with the definitions and [`generators_for`] them |
//! | What a save says about itself, without loading it | [`read_meta`]; every save of a directory with [`list_saves`] |
//! | What changed since the save was made | [`load_warnings`], after loading |
//! | The same against any store | [`save_game_into`], [`load_game_from`], [`read_meta_from`] |
//! | The same as bytes, in memory | [`save_game_bytes`], [`load_game_bytes`], [`read_meta_bytes`] |
//! | To check a save | [`verify`], [`verify_store`] |
//!
//! ```no_run
//! use clash_save::{SaveMeta, generators_for, load_game, save_game};
//! # fn demo(sim: &mut clash_sim::Sim) -> Result<(), clash_save::SaveError> {
//! let meta = SaveMeta::new("The long night").summary("Day 3").seed(7);
//! save_game(sim, "saves/night.c2save", &meta)?;
//!
//! let defs = sim.defs().clone();
//! let (loaded, meta) = load_game("saves/night.c2save", defs.clone(), generators_for(&defs)?)?;
//! assert_eq!(loaded.state_hash(), sim.state_hash());
//! assert_eq!(meta.name, "The long night");
//! # Ok(())
//! # }
//! ```
//!
//! # What a save file holds
//!
//! One `redb` database for each save (`clash_world::RedbStore`), written
//! in one transaction:
//!
//! | Part | Holds |
//! |------|-------|
//! | Tract blobs, by realm and tract | The cells of every tract that has changed, loaded or not (02 §11.1) |
//! | `world.*` sections | The world: realms and records, areas, sites, tickets, stability, activity, the loaded tracts and the save's tables of ids (02 §11.4) |
//! | [`SIM_SECTION`] | The simulation's own state: timeline and clocks, entities and components, rule state, settings, random streams, queues, the input log |
//! | [`TRACTS_SECTION`] | Which tracts the save holds a blob for |
//! | [`META_SECTION`] | [`SaveMeta`] |
//!
//! # The console's host
//!
//! [`host::GameHost`] implements the console's `ConsoleHost` over a real
//! simulation: the test-script runner, the world viewer and the game's
//! console overlay all drive it. See the [`host`] module.

mod game;
pub mod host;
mod meta;
mod pack;
mod tools;

pub use game::{
    META_BLOB_KIND, META_BLOB_VERSION, META_SECTION, SAVE_EXTENSION, SIM_SECTION, SaveError,
    SaveReport, TRACTS_BLOB_KIND, TRACTS_BLOB_VERSION, TRACTS_SECTION, generators_for, load_game,
    load_game_from, load_warnings, read_meta, read_meta_from, save_game, save_game_into,
};
pub use meta::{FORMAT_VERSION, ModStamp, SaveMeta};
pub use pack::{
    PACK_BLOB_KIND, PACK_BLOB_VERSION, load_game_bytes, pack_store, read_meta_bytes,
    save_game_bytes, unpack_store,
};
pub use tools::{SaveEntry, Verified, list_saves, verify, verify_store};
