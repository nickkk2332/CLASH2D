//! The application's state: what `app` reads and what the `app` effect
//! asks for (12 §10.4, §10.5).
//!
//! The scene owns its host and its application behind their traits, and
//! hands neither back. The frame loop must still reach the simulation,
//! to advance it. So both are handles on one [`Shared`] state: the
//! scene's [`App`] and [`crate::host::GameHost`] on one side, the loop
//! on the other. Each borrows it for the length of one call.
//!
//! # The lists
//!
//! | `app.` | Holds |
//! |--------|-------|
//! | `profiles` | Each `profiles/<name>.json` beside a searched mods directory and in the `profiles/` of each loaded mod. `state` is `current` for the one in use, `broken` for one that cannot be read. |
//! | `mods` | Each directory of a searched mods directory that holds a `mod.json`. `state` is `enabled` for a mod of the profile in use, else `disabled`; `broken` for a manifest that cannot be read. |
//! | `scenarios` | The `start` definitions of the loaded mods, in id order: the stand-in for game scenarios (`clash_scene::StartDef`). |
//! | `saves` | What the save backend lists ([`crate::saves`]). |
//! | `profile`, `scenario` | The profile in use; the start of the game being played. |
//! | `creation` | A map of what the player has chosen for the next game: `name` and `seed`, set with `edit_creation`. |
//!
//! # The services
//!
//! | Service | Does |
//! |---------|------|
//! | `new_game` | Starts a game from the start with the id `scenario` (left out, the first), with the `seed` and `name` of `app.creation` ([`crate::session::new_game`]) |
//! | `edit_creation` | Sets a `field` of `app.creation` to a `value` |
//! | `set_pref`, `rebind` | Keeps what the scene has already changed ([`crate::prefs`]) |
//! | `save`, `load`, `delete_save` | Through the save backend, which is not built into this binary yet |
//! | `quit` | Nothing more: the scene has noted it and the loop ends |
//! | `select_profile`, `reload_mods` | Refused, saying how to do it by starting again |

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use clash_expr::Value as ExprValue;
use clash_scene::ui::{AppService, to_var};
use clash_scene::{AppCurrent, AppEntry, AppList, AppRequest, AppState, StartDef};
use serde_json::Value as Json;

use crate::boot::Loaded;
use crate::paths::PROFILES_DIR;
use crate::prefs::{Binding, PrefStore};
use crate::saves::{SAVES_DIR, SaveBackend, SaveMeta};
use crate::session::{self, Session};

/// The state of an entry that is in use.
pub const STATE_CURRENT: &str = "current";
/// The state of a mod that is loaded, or a profile that could be
/// started.
pub const STATE_ENABLED: &str = "enabled";
/// The state of a mod that was found and is not loaded.
pub const STATE_DISABLED: &str = "disabled";
/// The state of an entry whose file cannot be read.
pub const STATE_BROKEN: &str = "broken";

/// The field of `app.creation` that holds the character's name.
pub const CREATION_NAME: &str = "name";
/// The field of `app.creation` that holds the seed of the world.
pub const CREATION_SEED: &str = "seed";

/// The name a save is given when none is.
pub const DEFAULT_SAVE: &str = "save";

/// Everything the application holds outside the scene. See the
/// [module](self) documentation.
pub struct Shared {
    /// The profile, loaded.
    pub loaded: Loaded,
    /// The profile in use: `app.profile`.
    pub profile: AppEntry,
    /// `app.profiles`.
    pub profiles: Vec<AppEntry>,
    /// `app.mods`.
    pub mods: Vec<AppEntry>,
    /// `app.scenarios`: the starts.
    pub starts: Vec<AppEntry>,
    /// `app.creation`: what the player has chosen for the next game.
    pub creation: BTreeMap<String, ExprValue>,
    /// The game being played, if one is.
    pub session: Option<Session>,
    /// What keeps saves.
    pub saves: Box<dyn SaveBackend>,
    /// The player's stored choices.
    pub store: PrefStore,
    /// The user's data directory, if there is one.
    pub data_dir: Option<PathBuf>,
    /// The qualified id of the keymap in force, for stored bindings.
    pub keymap: String,
    /// Whether a game was started and the loop has yet to show it.
    pub started: bool,
}

impl Shared {
    /// The directory saves are kept in, if there is a data directory.
    pub fn saves_dir(&self) -> Option<PathBuf> {
        self.data_dir.as_ref().map(|dir| dir.join(SAVES_DIR))
    }
}

/// The text of a JSON file's field, if the file is an object that has
/// one.
fn json_text(root: &Json, field: &str) -> Option<String> {
    root.get(field).and_then(Json::as_str).map(str::to_owned)
}

/// Reads a small JSON file whole.
fn read_json(path: &Path) -> Option<Json> {
    serde_json::from_str(&std::fs::read_to_string(path).ok()?).ok()
}

/// The directories of a directory, or the files with an extension, in
/// name order.
fn listed(dir: &Path, extension: Option<&str>) -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut paths: Vec<PathBuf> = entries
        .filter_map(|entry| entry.ok().map(|entry| entry.path()))
        .filter(|path| {
            let hidden = path
                .file_name()
                .is_some_and(|name| name.to_string_lossy().starts_with('.'));
            let wanted = match extension {
                Some(extension) => {
                    path.is_file() && path.extension().and_then(|e| e.to_str()) == Some(extension)
                }
                None => path.is_dir(),
            };
            wanted && !hidden
        })
        .collect();
    paths.sort();
    paths
}

/// The profiles that can be started: `app.profiles`.
pub fn list_profiles(loaded: &Loaded) -> Vec<AppEntry> {
    let current = loaded.defs.profile().map(|profile| profile.id.clone());
    let mut dirs: Vec<PathBuf> = loaded
        .search
        .iter()
        .filter_map(|mods| mods.parent().map(|beside| beside.join(PROFILES_DIR)))
        .collect();
    dirs.extend(
        loaded
            .defs
            .mods()
            .iter()
            .map(|loaded_mod| Path::new(&loaded_mod.root).join(PROFILES_DIR)),
    );
    let mut entries: Vec<AppEntry> = Vec::new();
    for dir in dirs {
        for path in listed(&dir, Some("json")) {
            let Some(stem) = path
                .file_stem()
                .map(|stem| stem.to_string_lossy().into_owned())
            else {
                continue;
            };
            let read = read_json(&path);
            let id = read
                .as_ref()
                .and_then(|root| json_text(root, "id"))
                .unwrap_or(stem);
            if entries.iter().any(|entry| entry.id == id) {
                // The first directory to hold a profile supplies it.
                continue;
            }
            let state = match &read {
                None => STATE_BROKEN,
                Some(_) if current.as_deref() == Some(id.as_str()) => STATE_CURRENT,
                Some(_) => STATE_ENABLED,
            };
            entries.push(AppEntry {
                name: read
                    .as_ref()
                    .and_then(|root| json_text(root, "name"))
                    .unwrap_or_else(|| id.clone()),
                description: String::new(),
                state: state.to_owned(),
                id,
            });
        }
    }
    entries
}

/// The mods that were found: `app.mods`.
pub fn list_mods(loaded: &Loaded) -> Vec<AppEntry> {
    let mut entries: Vec<AppEntry> = Vec::new();
    for search in &loaded.search {
        for dir in listed(search, None) {
            let manifest = dir.join("mod.json");
            if !manifest.is_file() {
                continue;
            }
            let read = read_json(&manifest);
            let folder = dir
                .file_name()
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_default();
            let id = read
                .as_ref()
                .and_then(|root| json_text(root, "id"))
                .unwrap_or(folder);
            if entries.iter().any(|entry| entry.id == id) {
                continue;
            }
            let enabled = loaded
                .defs
                .mods()
                .iter()
                .any(|loaded_mod| loaded_mod.manifest.id == id);
            let state = match (&read, enabled) {
                (None, _) => STATE_BROKEN,
                (Some(_), true) => STATE_ENABLED,
                (Some(_), false) => STATE_DISABLED,
            };
            let field = |name: &str| read.as_ref().and_then(|root| json_text(root, name));
            entries.push(AppEntry {
                name: field("name").unwrap_or_else(|| id.clone()),
                description: field("description").unwrap_or_default(),
                state: state.to_owned(),
                id,
            });
        }
    }
    entries
}

/// The starts of the loaded mods, in id order: `app.scenarios`.
pub fn list_starts(loaded: &Loaded) -> Vec<AppEntry> {
    let Some(starts) = loaded.defs.try_registry::<StartDef>() else {
        return Vec::new();
    };
    starts
        .iter()
        .map(|(id, def)| {
            let id = starts.name(id).to_owned();
            AppEntry {
                name: if def.name.is_empty() {
                    id.clone()
                } else {
                    def.name.clone()
                },
                description: def.doc.clone(),
                state: STATE_ENABLED.to_owned(),
                id,
            }
        })
        .collect()
}

/// The scene's handle on the application's state: the [`AppState`] it
/// is given.
#[derive(Clone)]
pub struct App {
    shared: Rc<RefCell<Shared>>,
}

impl App {
    /// A handle on shared state.
    pub fn new(shared: Rc<RefCell<Shared>>) -> App {
        App { shared }
    }
}

/// Starts a game: the `new_game` service.
fn start_game(shared: &mut Shared, request: &AppRequest) -> Result<(), String> {
    let start = match request.text("scenario") {
        Some(id) => id.to_owned(),
        None => shared
            .starts
            .first()
            .map(|start| start.id.clone())
            .ok_or("the loaded mods define no way to start a game")?,
    };
    let seed = shared
        .creation
        .get(CREATION_SEED)
        .and_then(ExprValue::as_int)
        .map_or(crate::cli::DEFAULT_SEED, |seed| seed.unsigned_abs());
    let name = shared
        .creation
        .get(CREATION_NAME)
        .and_then(ExprValue::as_str)
        .unwrap_or("")
        .trim()
        .to_owned();
    let session = session::new_game(&shared.loaded, &start, seed, &name)?;
    shared.session = Some(session);
    shared.started = true;
    Ok(())
}

impl AppState for App {
    fn entries(&self, list: AppList) -> Vec<AppEntry> {
        let shared = self.shared.borrow();
        match list {
            AppList::Saves => shared.saves.list(shared.saves_dir().as_deref()),
            AppList::Profiles => shared.profiles.clone(),
            AppList::Mods => shared.mods.clone(),
            AppList::Scenarios => shared.starts.clone(),
        }
    }

    fn current(&self, which: AppCurrent) -> Option<AppEntry> {
        let shared = self.shared.borrow();
        match which {
            AppCurrent::Profile => Some(shared.profile.clone()),
            AppCurrent::Scenario => shared.session.as_ref().map(|session| session.start.clone()),
        }
    }

    fn can_save(&self) -> bool {
        let shared = self.shared.borrow();
        shared.saves.available() && shared.session.is_some()
    }

    fn can_load(&self) -> bool {
        let shared = self.shared.borrow();
        shared.saves.available() && !shared.saves.list(shared.saves_dir().as_deref()).is_empty()
    }

    fn creation(&self) -> ExprValue {
        let shared = self.shared.borrow();
        ExprValue::map(
            shared
                .creation
                .iter()
                .map(|(field, value)| (field.as_str(), value.clone())),
        )
    }

    fn service(&mut self, request: &AppRequest) -> Result<(), String> {
        let mut shared = self.shared.borrow_mut();
        let shared = &mut *shared;
        let dir = shared.saves_dir();
        match request.service {
            AppService::NewGame => start_game(shared, request),
            AppService::EditCreation => {
                let field = request
                    .text("field")
                    .ok_or("`edit_creation` needs the `field` to change, as a text")?;
                let value = request.args.get("value").cloned().unwrap_or_default();
                shared.creation.insert(field.to_owned(), value);
                Ok(())
            }
            AppService::SetPref => {
                let (Some(id), Some(value)) = (
                    request.text("pref"),
                    request.args.get("value").and_then(to_var),
                ) else {
                    return Ok(());
                };
                shared.store.set_pref(id, value)
            }
            AppService::Rebind => {
                let text = |name: &str| request.text(name).unwrap_or("").to_owned();
                let binding = Binding {
                    keymap: shared.keymap.clone(),
                    context: text("context"),
                    key: text("key"),
                    name: text("name"),
                };
                shared.store.rebind(binding)
            }
            AppService::Save => {
                let name = request.text("name").unwrap_or(DEFAULT_SAVE).to_owned();
                let Some(session) = shared.session.as_mut() else {
                    return Err("no game is being played, so there is nothing to save".to_owned());
                };
                let meta = SaveMeta {
                    start: session.start.id.clone(),
                    name: session.name.clone(),
                };
                shared
                    .saves
                    .save(dir.as_deref(), &mut session.sim, &meta, &name)
            }
            AppService::Load => {
                let save = request
                    .text("save")
                    .ok_or("`load` needs the id of the `save` to load")?;
                let saved = shared.saves.load(dir.as_deref(), &shared.loaded, save)?;
                let session = session::resume(&shared.loaded, saved)?;
                shared.session = Some(session);
                shared.started = true;
                Ok(())
            }
            AppService::DeleteSave => {
                let save = request
                    .text("save")
                    .ok_or("`delete_save` needs the id of the `save` to delete")?;
                shared.saves.delete(dir.as_deref(), save)
            }
            // The scene has noted it: the loop ends.
            AppService::Quit => Ok(()),
            AppService::SelectProfile => Err(
                "another profile cannot be started from inside the game yet: leave, and start it with `clash --profile name`"
                    .to_owned(),
            ),
            AppService::ReloadMods => Err(
                "mods cannot be read again while the game runs yet: leave and start it again"
                    .to_owned(),
            ),
        }
    }
}
