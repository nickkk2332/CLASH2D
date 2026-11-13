//! Saving and loading (13 §9; 01 §11.2).
//!
//! The services `save`, `load` and `delete_save`, the list `app.saves`
//! and the flags `app.can_save` and `app.can_load` go through one trait,
//! [`SaveBackend`]. The binary uses [`FileSaves`]: one file for each
//! save, written and read by `clash-save` (`save_game`, `load_game`,
//! `read_meta`), in the `saves/` of the user's data directory. A binary
//! with nowhere to keep anything ([`NoSaves`]) lists nothing and refuses
//! each service with its reason.
//!
//! # A save
//!
//! A save is a file `saves/<id>.c2save`. Its id is made from the name
//! the player gave it: letters and digits kept, in lower case, anything
//! else a `-`; a name of nothing is `save`. Saving under a name that is
//! there already replaces that save. Beside the simulation the save
//! holds the metadata of 01 §11.2: the name; a summary the application
//! writes (who, where it began, and how long the game has run); the
//! seed; and, as the saver's own bytes, what the application needs to
//! carry the game on ([`SaveMeta`]: the scenario it began from, the
//! character's name and who the character is).
//!
//! # The list
//!
//! `app.saves` lists every save of the directory, newest first by when
//! its file was written. Each entry's `id` is the save's, its `name` the
//! name it was given, its `description` the summary and when it was
//! written (`2026-11-12 09:30`, UTC), and its `state`:
//!
//! | `state` | The save |
//! |---------|----------|
//! | `enabled` | Was made with the mods loaded now |
//! | `changed` | Was made with other mods, or other versions of them: it loads with warnings |
//! | `broken` | Cannot be read; its `description` says why |
//!
//! # Loading
//!
//! A load reads the whole game back (`clash_save::load_game`) over the
//! loaded definitions and the generators bound to them. What differs
//! from when it was made (`clash_save::load_warnings`: mods missing,
//! added or at another version, definitions that became placeholders)
//! comes back with it, for the application to show. Saving and loading
//! change nothing that is hashed (01 R7).

use std::path::{Path, PathBuf};
use std::time::SystemTime;

use clash_core::EntityId;
use clash_scene::AppEntry;
use clash_sim::Sim;

use crate::boot::Loaded;

/// Why nothing can be saved or loaded where there is no data directory.
pub const NO_DIRECTORY: &str =
    "there is no data directory to keep saves in: start the game with `--data dir`";

/// The directory saves are kept in, inside the user's data directory.
pub const SAVES_DIR: &str = "saves";

/// The id of a save given a name of nothing.
pub const DEFAULT_SAVE_ID: &str = "save";

/// The `state` of a save made with the mods loaded now.
pub const SAVE_MATCHES: &str = "enabled";
/// The `state` of a save made with other mods.
pub const SAVE_CHANGED: &str = "changed";
/// The `state` of a save that cannot be read.
pub const SAVE_BROKEN: &str = "broken";

/// What the application writes beside a saved simulation, and reads
/// back with it: what it needs to carry a game on that the simulation
/// does not hold. Kept in the save's bytes of the saver's own, as JSON.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct SaveMeta {
    /// The qualified id of the scenario the game began from.
    pub scenario: String,
    /// What the player called the character.
    pub name: String,
    /// The player's character, if the saver knows it.
    pub player: Option<EntityId>,
    /// The seed the game's world was made with.
    pub seed: u64,
    /// A line for a load screen: who, where and how long.
    pub summary: String,
}

impl SaveMeta {
    /// The saver's own bytes: JSON.
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut map = serde_json::Map::new();
        map.insert("scenario".to_owned(), self.scenario.clone().into());
        map.insert("name".to_owned(), self.name.clone().into());
        if let Some(player) = self.player {
            map.insert("player".to_owned(), player.to_bits().into());
        }
        serde_json::Value::Object(map).to_string().into_bytes()
    }

    /// Reads back what [`SaveMeta::to_bytes`] wrote, with the seed and
    /// the summary from the save. Bytes that are not that give an empty
    /// scenario, which the application refuses with a reason.
    pub fn from_save(meta: &clash_save::SaveMeta) -> SaveMeta {
        let read: serde_json::Value = serde_json::from_slice(&meta.app).unwrap_or_default();
        let text = |field: &str| {
            read.get(field)
                .and_then(serde_json::Value::as_str)
                .unwrap_or("")
                .to_owned()
        };
        SaveMeta {
            scenario: text("scenario"),
            name: text("name"),
            player: read
                .get("player")
                .and_then(serde_json::Value::as_u64)
                .map(EntityId::from_bits),
            seed: meta.seed,
            summary: meta.summary.clone(),
        }
    }
}

/// A game read back from a save.
#[derive(Debug)]
pub struct SavedGame {
    /// The simulation, ready to carry on.
    pub sim: Sim,
    /// What was written beside it.
    pub meta: SaveMeta,
    /// What differs from when it was made: for the player to be told.
    pub warnings: Vec<String>,
}

/// What saving and loading need of whatever keeps saves.
pub trait SaveBackend {
    /// Whether this backend can save and load at all, in a directory.
    /// With one that cannot, `app.can_save` and `app.can_load` are
    /// false.
    fn available(&self, dir: Option<&Path>) -> bool;

    /// The saves there are, newest first: `app.saves`. `loaded` is what
    /// each is compared with, to say whether its mods are the ones
    /// loaded now.
    fn list(&self, dir: Option<&Path>, loaded: &Loaded) -> Vec<AppEntry>;

    /// Saves the simulation under a name, at a safe point, with what
    /// the application needs beside it. Returns the save's id.
    fn save(
        &mut self,
        dir: Option<&Path>,
        sim: &mut Sim,
        meta: &SaveMeta,
        name: &str,
    ) -> Result<String, String>;

    /// Loads the save with an id: a simulation over the loaded
    /// definitions, ready to carry on, what was written beside it and
    /// what differs from when it was made.
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
    fn available(&self, _dir: Option<&Path>) -> bool {
        false
    }

    fn list(&self, _dir: Option<&Path>, _loaded: &Loaded) -> Vec<AppEntry> {
        Vec::new()
    }

    fn save(
        &mut self,
        _dir: Option<&Path>,
        _sim: &mut Sim,
        _meta: &SaveMeta,
        _name: &str,
    ) -> Result<String, String> {
        Err(NO_DIRECTORY.to_owned())
    }

    fn load(
        &mut self,
        _dir: Option<&Path>,
        _loaded: &Loaded,
        _save: &str,
    ) -> Result<SavedGame, String> {
        Err(NO_DIRECTORY.to_owned())
    }

    fn delete(&mut self, _dir: Option<&Path>, _save: &str) -> Result<(), String> {
        Err(NO_DIRECTORY.to_owned())
    }
}

/// Saves as files in the user's data directory, through `clash-save`.
/// See the [module](self) documentation.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct FileSaves;

/// The id a save is kept under, from the name the player gave it.
pub fn save_id(name: &str) -> String {
    let mut id = String::new();
    for ch in name.trim().chars() {
        if ch.is_alphanumeric() {
            id.extend(ch.to_lowercase());
        } else if !id.ends_with('-') {
            id.push('-');
        }
    }
    let id = id.trim_matches('-');
    if id.is_empty() {
        DEFAULT_SAVE_ID.to_owned()
    } else {
        id.to_owned()
    }
}

/// The file of a save, by its id, refusing an id that is not one this
/// backend makes: one that names another directory.
fn file_of(dir: &Path, save: &str) -> Result<PathBuf, String> {
    let fine = !save.is_empty()
        && save
            .chars()
            .all(|ch| ch.is_alphanumeric() || ch == '-' || ch == '_');
    if !fine {
        return Err(format!("there is no save `{save}`"));
    }
    Ok(dir.join(format!("{save}.{}", clash_save::SAVE_EXTENSION)))
}

/// A day and a time of day, in UTC, from seconds since 1970: how a load
/// screen says when a save was written.
pub fn utc_text(seconds: u64) -> String {
    let days = (seconds / 86_400) as i64;
    let rest = seconds % 86_400;
    // Days to a civil date (the proleptic Gregorian calendar).
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    format!(
        "{year:04}-{month:02}-{day:02} {:02}:{:02}",
        rest / 3_600,
        rest % 3_600 / 60
    )
}

/// When a file was last written, in seconds since 1970.
fn written(path: &Path) -> u64 {
    std::fs::metadata(path)
        .and_then(|meta| meta.modified())
        .ok()
        .and_then(|time| time.duration_since(SystemTime::UNIX_EPOCH).ok())
        .map_or(0, |since| since.as_secs())
}

impl SaveBackend for FileSaves {
    fn available(&self, dir: Option<&Path>) -> bool {
        dir.is_some()
    }

    fn list(&self, dir: Option<&Path>, loaded: &Loaded) -> Vec<AppEntry> {
        let Some(dir) = dir else {
            return Vec::new();
        };
        let mut found: Vec<(u64, AppEntry)> = clash_save::list_saves(dir)
            .into_iter()
            .map(|entry| {
                let id = entry
                    .path
                    .file_stem()
                    .map(|stem| stem.to_string_lossy().into_owned())
                    .unwrap_or_default();
                let when = written(&entry.path);
                let at = utc_text(when);
                let listed = match entry.meta {
                    Ok(meta) => {
                        let state = if meta.differences(&loaded.defs).is_empty() {
                            SAVE_MATCHES
                        } else {
                            SAVE_CHANGED
                        };
                        AppEntry {
                            name: if meta.name.is_empty() {
                                id.clone()
                            } else {
                                meta.name.clone()
                            },
                            description: if meta.summary.is_empty() {
                                at
                            } else {
                                format!("{} · {at}", meta.summary)
                            },
                            state: state.to_owned(),
                            id,
                        }
                    }
                    Err(error) => AppEntry {
                        name: id.clone(),
                        description: error.to_string(),
                        state: SAVE_BROKEN.to_owned(),
                        id,
                    },
                };
                (when, listed)
            })
            .collect();
        // Newest first; of equal times, by id.
        found.sort_by(|(a, first), (b, second)| b.cmp(a).then_with(|| first.id.cmp(&second.id)));
        found.into_iter().map(|(_, entry)| entry).collect()
    }

    fn save(
        &mut self,
        dir: Option<&Path>,
        sim: &mut Sim,
        meta: &SaveMeta,
        name: &str,
    ) -> Result<String, String> {
        let dir = dir.ok_or(NO_DIRECTORY)?;
        let id = save_id(name);
        let path = file_of(dir, &id)?;
        let shown = if name.trim().is_empty() {
            id.clone()
        } else {
            name.trim().to_owned()
        };
        let written = clash_save::SaveMeta::new(shown)
            .summary(meta.summary.clone())
            .seed(meta.seed)
            .app(meta.to_bytes());
        clash_save::save_game(sim, &path, &written).map_err(|error| error.to_string())?;
        Ok(id)
    }

    fn load(
        &mut self,
        dir: Option<&Path>,
        loaded: &Loaded,
        save: &str,
    ) -> Result<SavedGame, String> {
        let dir = dir.ok_or(NO_DIRECTORY)?;
        let path = file_of(dir, save)?;
        let tables = loaded
            .tables
            .as_ref()
            .map_err(|problems| format!("the world cannot be made:\n{problems}"))?;
        let generators = clash_gen::generators_with(&tables.generation);
        let (mut sim, meta) =
            clash_save::load_game(&path, std::sync::Arc::clone(&loaded.defs), generators)
                .map_err(|error| error.to_string())?;
        let warnings = clash_save::load_warnings(&mut sim, &meta);
        Ok(SavedGame {
            sim,
            meta: SaveMeta::from_save(&meta),
            warnings,
        })
    }

    fn delete(&mut self, dir: Option<&Path>, save: &str) -> Result<(), String> {
        let dir = dir.ok_or(NO_DIRECTORY)?;
        let path = file_of(dir, save)?;
        std::fs::remove_file(&path)
            .map_err(|error| format!("the save `{save}` could not be deleted: {error}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_name_becomes_an_id_a_file_can_have() {
        assert_eq!(save_id("The Long Night"), "the-long-night");
        assert_eq!(save_id("  ../../etc  "), "etc");
        assert_eq!(save_id("Día 3!"), "día-3");
        assert_eq!(save_id(""), DEFAULT_SAVE_ID);
        assert_eq!(save_id("?!"), DEFAULT_SAVE_ID);
        assert!(file_of(Path::new("saves"), "../x").is_err());
        assert!(file_of(Path::new("saves"), "").is_err());
    }

    #[test]
    fn times_are_written_as_a_day_and_an_hour() {
        assert_eq!(utc_text(0), "1970-01-01 00:00");
        assert_eq!(utc_text(951_782_400), "2000-02-29 00:00");
        assert_eq!(utc_text(1_794_475_800), "2026-11-12 09:30");
    }

    #[test]
    fn what_the_application_keeps_reads_back() {
        let meta = SaveMeta {
            scenario: "core:town".to_owned(),
            name: "Ada".to_owned(),
            player: Some(EntityId::from_bits(7)),
            seed: 3,
            summary: "Ada in the town".to_owned(),
        };
        let saved = clash_save::SaveMeta::new("x")
            .summary(meta.summary.clone())
            .seed(3)
            .app(meta.to_bytes());
        assert_eq!(SaveMeta::from_save(&saved), meta);
        // Bytes of someone else's: nothing to carry on from.
        let other = clash_save::SaveMeta::new("x").app(b"\x00\x01".to_vec());
        assert_eq!(SaveMeta::from_save(&other).scenario, "");
    }
}
