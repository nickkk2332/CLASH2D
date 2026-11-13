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
//! | `scenarios` | The `scenario` definitions of the loaded mods (13 §3), in order of qualified id. |
//! | `saves` | What the save backend lists, newest first ([`crate::saves`]) |
//! | `profile`, `scenario` | The profile in use; the scenario of the game being played. |
//! | `creation` | The character being made for the next game: null while none is, else the creation session as [`crate::creation`] says. |
//! | `setup` | The game being set up (13 §12): null until a scenario is asked for, then the map [`Setup::value`] gives: its `id`, `name`, `doc`, `seed` (`ask`, `random` or the number), `places` (each `name` and `index`) and `options` (each `id`, `name`, `kind`, `value`, `next` and `choices`). |
//!
//! # The services
//!
//! | Service | Does |
//! |---------|------|
//! | `new_game` | Starts a game from the scenario with the id `scenario` (left out, the first) at its place `place`, with the character the player finished, or else the scenario's template with its defaults, the name and seed given and the options chosen ([`crate::session::new_game`]). Without `place`, a scenario that offers more than one place or asks about options is set up instead: `app.setup` holds it and the screen of the role `setup` is opened with `scenario` |
//! | `creation` | The creation session: `begin`, `select`, `clear`, `name`, `seed`, `finish` ([`crate::creation`]); and `option` with `key` and `value`, an answer to an option of the scenario being set up |
//! | `edit_creation` | The older form of `creation`'s `name` and `seed`: sets the `field` `name` or `seed` to a `value` |
//! | `set_pref`, `rebind` | Keeps what the scene has already changed ([`crate::prefs`]) |
//! | `save` | Saves the game under the `name` given (left out or empty, the character's name, or else the scenario's), through the save backend ([`crate::saves`]); refused unless the game's saving policy is `free` (13 §9.2), and once it has ended |
//! | `load` | Loads the `save` with that id and carries it on: from the title, or in a game whose saving policy is `free`. What differs from when it was saved is told (`warnings`) |
//! | `delete_save` | Deletes the `save` with that id |
//! | `quit` | Nothing more: the scene has noted it and the loop ends |
//! | `select_profile`, `reload_mods` | Refused, saying how to do it by starting again |
//!
//! # Saving policies (13 §9.2)
//!
//! A game has the policy its scenario names (`saving.policy`), or `free`:
//!
//! | Policy | `app.can_save` | `app.can_load` in the game | Saved by the application |
//! |--------|----------------|----------------------------|--------------------------|
//! | `free` | While a game is played | While there is a save | Never by itself |
//! | `single` | False | False | When the game is left, under the character's name; an ending removes it |
//! | `checkpoints` | False | False | When a rule asks for a checkpoint |
//! | `none` | False | False | Never |
//!
//! From the title, `app.can_load` holds while there is any save. A
//! rule's checkpoint is saved under every policy but `none`.

use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use clash_expr::Value as ExprValue;
use clash_scene::ui::{AppService, to_var};
use clash_scene::{AppCurrent, AppEntry, AppList, AppRequest, AppState};
use clash_sim::scenario::{ScenarioDef, SettingDef};
use serde_json::Value as Json;

use crate::boot::Loaded;
use crate::creation::{Creation, FIELD_NAME, FIELD_SEED};
use crate::paths::PROFILES_DIR;
use crate::prefs::{Binding, PrefStore};
use crate::saves::{SAVES_DIR, SaveBackend};
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

/// The saving policy of a game the player saves and loads at will.
pub const POLICY_FREE: &str = "free";
/// The saving policy of a game with one save, written on leaving.
pub const POLICY_SINGLE: &str = "single";
/// The saving policy of a game with no saves at all.
pub const POLICY_NONE: &str = "none";

/// The op of the service `creation` that answers an option of the
/// scenario being set up.
pub const OP_OPTION: &str = "option";

/// The game being set up on the new-game screens (13 §12): the scenario
/// chosen and the answers to its options so far.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Setup {
    /// The scenario, by qualified id.
    pub scenario: String,
    /// The answers given, by setting key.
    pub options: Vec<(String, clash_core::Value)>,
}

/// A setting's value as JSON, for checking against its definition.
fn json_of(value: &clash_core::Value) -> Json {
    match value {
        clash_core::Value::Bool(flag) => Json::Bool(*flag),
        clash_core::Value::Int(int) => Json::from(*int),
        clash_core::Value::Real(real) => Json::from(real.get()),
        clash_core::Value::Str(text) => Json::String(text.clone()),
        _ => Json::Null,
    }
}

impl Setup {
    /// The scenario's setting definitions it asks about, each with its
    /// qualified key, in order.
    fn asked<'a>(
        loaded: &'a Loaded,
        def: &ScenarioDef,
        scenario: &str,
    ) -> Vec<(String, &'a SettingDef)> {
        let namespace = scenario
            .split_once(':')
            .map_or("", |(namespace, _)| namespace);
        let Some(settings) = loaded.defs.try_registry::<SettingDef>() else {
            return Vec::new();
        };
        def.options
            .iter()
            .filter_map(|option| {
                let key = clash_sim::scenario::def::qualified(&option.value, namespace);
                let id = settings.lookup(&key)?;
                Some((key, settings.get(id)))
            })
            .collect()
    }

    /// The answer for a setting: the one given, or the setting's
    /// default.
    fn answer(&self, key: &str, def: &SettingDef) -> clash_core::Value {
        self.options
            .iter()
            .find(|(have, _)| have == key)
            .map(|(_, value)| value.clone())
            .or_else(|| clash_sim::scenario::stored_value(&def.default))
            .unwrap_or(clash_core::Value::Null)
    }

    /// The value after one in a setting's choices, round: what `next`
    /// of an option row is, for a screen that steps through them.
    fn next(value: &clash_core::Value, def: &SettingDef) -> clash_core::Value {
        match def.kind.as_str() {
            "flag" => clash_core::Value::Bool(!matches!(value, clash_core::Value::Bool(true))),
            "choice" => {
                let at = def
                    .options
                    .iter()
                    .position(
                        |option| matches!(value, clash_core::Value::Str(text) if text == option),
                    )
                    .map_or(0, |at| (at + 1) % def.options.len().max(1));
                def.options
                    .get(at)
                    .map_or(clash_core::Value::Null, |option| {
                        clash_core::Value::Str(option.clone())
                    })
            }
            _ => value.clone(),
        }
    }

    /// The answers to give `new_game`: one for each option asked.
    pub fn answers(&self, loaded: &Loaded) -> Vec<(String, clash_core::Value)> {
        let Ok((def, _)) = session::scenario_of(loaded, &self.scenario) else {
            return Vec::new();
        };
        Setup::asked(loaded, def, &self.scenario)
            .into_iter()
            .map(|(key, setting)| {
                let value = self.answer(&key, setting);
                (key, value)
            })
            .collect()
    }

    /// `app.setup`: see the [module](self) documentation.
    pub fn value(&self, loaded: &Loaded) -> ExprValue {
        let Ok((def, entry)) = session::scenario_of(loaded, &self.scenario) else {
            return ExprValue::Null;
        };
        let places = def
            .place_names()
            .into_iter()
            .enumerate()
            .map(|(index, name)| {
                ExprValue::map([
                    ("name", ExprValue::text(name)),
                    ("index", ExprValue::Int(index as i64)),
                ])
            });
        let options =
            Setup::asked(loaded, def, &self.scenario)
                .into_iter()
                .map(|(key, setting)| {
                    let value = self.answer(&key, setting);
                    let next = Setup::next(&value, setting);
                    let name = if setting.name.is_empty() {
                        key.clone()
                    } else {
                        setting.name.clone()
                    };
                    ExprValue::map([
                        ("id", ExprValue::text(key)),
                        ("name", ExprValue::text(name)),
                        ("kind", ExprValue::text(setting.kind.as_str())),
                        ("value", ExprValue::from(value)),
                        ("next", ExprValue::from(next)),
                        (
                            "choices",
                            ExprValue::list(
                                setting
                                    .options
                                    .iter()
                                    .map(|option| ExprValue::text(option.as_str())),
                            ),
                        ),
                    ])
                });
        let seed = def
            .world()
            .map_or_else(|| "ask".to_owned(), |world| world.seed.written());
        ExprValue::map([
            ("id", ExprValue::text(entry.id)),
            ("name", ExprValue::text(entry.name)),
            ("doc", ExprValue::text(entry.description)),
            ("seed", ExprValue::text(seed)),
            ("places", ExprValue::list(places)),
            ("options", ExprValue::list(options)),
        ])
    }

    /// Answers an option: refused for a setting the scenario does not
    /// ask about, or a value it does not take.
    fn give(&mut self, loaded: &Loaded, key: &str, value: clash_core::Value) -> Result<(), String> {
        let (def, _) = session::scenario_of(loaded, &self.scenario)?;
        let asked = Setup::asked(loaded, def, &self.scenario);
        let (_, setting) = asked
            .iter()
            .find(|(have, _)| have == key)
            .ok_or_else(|| format!("the scenario does not ask about `{key}`"))?;
        setting
            .check(&json_of(&value))
            .map_err(|why| format!("`{key}`: {why}"))?;
        self.options.retain(|(have, _)| have != key);
        self.options.push((key.to_owned(), value));
        Ok(())
    }
}

/// A seed drawn for a scenario whose seed is `random`: from the clock,
/// which the application may read (01 R5 binds the simulation).
fn drawn_seed() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(1, |since| since.as_secs() ^ u64::from(since.subsec_nanos()))
}

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
    /// `app.scenarios`: the game scenarios.
    pub scenarios: Vec<AppEntry>,
    /// `app.setup`: the game being set up, if one is.
    pub setup: Option<Setup>,
    /// A scenario `new_game` asked to be set up: the loop opens the
    /// screen of the role `setup` for it.
    pub wants_setup: Option<String>,
    /// The introduction of a game just begun, `(title, text)`: the loop
    /// opens the screen of the role `intro` with it once the game is
    /// shown.
    pub intro: Option<(String, String)>,
    /// `app.creation`: the character being made for the next game, its
    /// name and the seed.
    pub creation: Creation,
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
    /// Whether a game was started or loaded and the loop has yet to
    /// show it.
    pub started: bool,
    /// What a load had to say: what differs from when the save was
    /// made. The loop shows it once the game is.
    pub warnings: Vec<String>,
    /// Lines a service has for the log, taken by the loop.
    pub said: Vec<String>,
    /// What the service `talk` last gathered, which `app.talk` reads:
    /// null when nothing was.
    pub talk: ExprValue,
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

/// The game scenarios of the loaded mods, in order of qualified id:
/// `app.scenarios`.
pub fn list_scenarios(loaded: &Loaded) -> Vec<AppEntry> {
    let Some(scenarios) = loaded.defs.try_registry::<ScenarioDef>() else {
        return Vec::new();
    };
    let mut entries: Vec<AppEntry> = scenarios
        .iter()
        .map(|(id, def)| {
            let id = scenarios.name(id).to_owned();
            AppEntry {
                name: def.shown_name(&id),
                description: def.doc.clone(),
                state: STATE_ENABLED.to_owned(),
                id,
            }
        })
        .collect();
    entries.sort_by(|a, b| a.id.cmp(&b.id));
    entries
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

/// Starts a game: the `new_game` service. The character is the one the
/// player finished, or else the scenario's template with its defaults
/// ([`Creation::spec_for`]). A scenario that offers a choice of place
/// or asks about options, asked for with no `place`, is set up instead:
/// see the [module](self) documentation.
/// The service `talk` (15 §11; decision D-799): what the character
/// played could say to `to`, the options the rules leave on the hook
/// `talk_options`, kept for `app.talk`. The hook is raised on a copy of
/// the simulation, which is then dropped, so that asking changes nothing
/// in the game and the input log still replays it. Someone the character
/// does not perceive now gets no options and a reason; so does a
/// refusal of the hook. `to` null or left out forgets what was gathered.
fn gather_talk(shared: &mut Shared, request: &AppRequest) -> Result<(), String> {
    let Some(to) = request
        .args
        .get("to")
        .filter(|to| **to != ExprValue::Null)
        .cloned()
    else {
        shared.talk = ExprValue::Null;
        return Ok(());
    };
    let session = shared
        .session
        .as_ref()
        .ok_or("no game is being played, so there is nobody to talk to")?;
    let gathered = |options: Vec<ExprValue>, why: &str| {
        ExprValue::map([
            ("to", to.clone()),
            ("options", ExprValue::list(options)),
            ("why", ExprValue::from(why)),
        ])
    };
    let entity = match session.sim.subject_of(&to) {
        Some(clash_sim::Subject::Entity(entity)) => entity,
        _ => {
            shared.talk = gathered(Vec::new(), "There is nobody there to talk to.");
            return Ok(());
        }
    };
    // Whoever the character sees standing in view may be talked to, as
    // the target cycle offers them, whether or not its awareness of them
    // has yet come to a detection (decision D-878).
    let in_view = session
        .percepts
        .sight
        .sprites
        .contains_key(&clash_scene::SpriteId::new(entity.to_bits()));
    if !in_view && !clash_sim::perception::perceives(&session.sim, session.player, entity) {
        shared.talk = gathered(Vec::new(), "You cannot see or hear them now.");
        return Ok(());
    }
    let mut copy = session.sim.clone();
    shared.talk = match clash_sim::social::talk_options(&mut copy, session.player, entity) {
        Ok(options) => gathered(
            options
                .into_iter()
                .map(|option| {
                    let text_or_null = |text: Option<String>| {
                        text.map_or(ExprValue::Null, |text| ExprValue::from(text.as_str()))
                    };
                    ExprValue::map([
                        ("id", ExprValue::from(option.id.as_str())),
                        ("text", ExprValue::from(option.text.as_str())),
                        ("says", ExprValue::Bool(option.kind.is_some())),
                        ("kind", text_or_null(option.kind)),
                        ("lines", text_or_null(option.lines)),
                    ])
                })
                .collect(),
            "",
        ),
        Err(reason) => gathered(Vec::new(), &sentence(&reason.message)),
    };
    Ok(())
}

/// A reason's message as a sentence: a capital first letter and a full
/// stop.
fn sentence(message: &str) -> String {
    let mut chars = message.trim().chars();
    let mut out: String = chars
        .next()
        .map(|first| first.to_uppercase().chain(chars).collect())
        .unwrap_or_default();
    if !out.ends_with('.') {
        out.push('.');
    }
    out
}

fn start_game(shared: &mut Shared, request: &AppRequest) -> Result<(), String> {
    let scenario = match request.text("scenario") {
        Some(id) => id.to_owned(),
        None => shared
            .scenarios
            .first()
            .map(|entry| entry.id.clone())
            .ok_or("the loaded mods define no game scenario to start")?,
    };
    let (def, entry) = session::scenario_of(&shared.loaded, &scenario)?;
    let place = match request.args.get("place").and_then(ExprValue::as_int) {
        Some(place) => {
            Some(u32::try_from(place).map_err(|_| format!("there is no place {place}"))?)
        }
        None => None,
    };
    let offers_choice = def.start.at.len() > 1 || !def.options.is_empty();
    let intro = (!def.intro.is_empty()).then(|| (entry.name.clone(), def.intro.clone()));
    if shared
        .setup
        .as_ref()
        .is_none_or(|setup| setup.scenario != scenario)
    {
        shared.setup = Some(Setup {
            scenario: scenario.clone(),
            options: Vec::new(),
        });
    }
    let Some(place) = place.or((!offers_choice).then_some(0)) else {
        shared.wants_setup = Some(scenario);
        return Ok(());
    };
    let options = shared
        .setup
        .as_ref()
        .map(|setup| setup.answers(&shared.loaded))
        .unwrap_or_default();
    let template = session::scenario_template(&shared.loaded, &scenario)?;
    let defs = &shared.loaded.defs;
    let spec = shared.creation.spec_for(defs, template.as_deref())?;
    let answers = session::Answers {
        seed: shared.creation.seed(),
        drawn: drawn_seed(),
        place,
        options,
    };
    let name = shared.creation.name().to_owned();
    let session = session::new_game(&shared.loaded, &scenario, &answers, spec, &name)?;
    shared.session = Some(session);
    shared.creation.begun();
    shared.setup = None;
    shared.intro = intro;
    shared.started = true;
    Ok(())
}

/// The older service `edit_creation { field, value }`: the name or the
/// seed of the next game.
fn edit_creation(shared: &mut Shared, request: &AppRequest) -> Result<(), String> {
    let field = request
        .text("field")
        .ok_or("`edit_creation` needs the `field` to change, as a text")?;
    let value = request.args.get("value").cloned().unwrap_or_default();
    match field {
        FIELD_NAME => {
            let name = value.as_str().unwrap_or("").to_owned();
            shared.creation.set_name(&shared.loaded.defs, &name)
        }
        FIELD_SEED => {
            let seed = value
                .as_int()
                .ok_or("the seed of a game is a whole number")?
                .unsigned_abs();
            shared.creation.set_seed(seed);
            Ok(())
        }
        other => Err(format!(
            "the character being made has no field `{other}` to set: it has `{FIELD_NAME}` and `{FIELD_SEED}`"
        )),
    }
}

impl AppState for App {
    fn entries(&self, list: AppList) -> Vec<AppEntry> {
        let shared = self.shared.borrow();
        match list {
            AppList::Saves => shared
                .saves
                .list(shared.saves_dir().as_deref(), &shared.loaded),
            AppList::Profiles => shared.profiles.clone(),
            AppList::Mods => shared.mods.clone(),
            AppList::Scenarios => shared.scenarios.clone(),
        }
    }

    fn current(&self, which: AppCurrent) -> Option<AppEntry> {
        let shared = self.shared.borrow();
        match which {
            AppCurrent::Profile => Some(shared.profile.clone()),
            AppCurrent::Scenario => shared
                .session
                .as_ref()
                .map(|session| session.scenario.clone()),
        }
    }

    fn can_save(&self) -> bool {
        let shared = self.shared.borrow();
        shared.saves.available(shared.saves_dir().as_deref())
            && shared
                .session
                .as_ref()
                .is_some_and(|session| session.saving == POLICY_FREE && session.ended.is_none())
    }

    fn can_load(&self) -> bool {
        let shared = self.shared.borrow();
        let dir = shared.saves_dir();
        let allowed = shared
            .session
            .as_ref()
            .is_none_or(|session| session.saving == POLICY_FREE);
        allowed
            && shared.saves.available(dir.as_deref())
            && !shared.saves.list(dir.as_deref(), &shared.loaded).is_empty()
    }

    fn creation(&self) -> ExprValue {
        let shared = self.shared.borrow();
        shared.creation.value(&shared.loaded.defs)
    }

    fn talk(&self) -> ExprValue {
        self.shared.borrow().talk.clone()
    }

    fn setup(&self) -> ExprValue {
        let shared = self.shared.borrow();
        shared
            .setup
            .as_ref()
            .map_or(ExprValue::Null, |setup| setup.value(&shared.loaded))
    }

    fn service(&mut self, request: &AppRequest) -> Result<(), String> {
        let mut shared = self.shared.borrow_mut();
        let shared = &mut *shared;
        let dir = shared.saves_dir();
        match request.service {
            AppService::NewGame => {
                shared.talk = ExprValue::Null;
                start_game(shared, request)
            }
            AppService::Talk => gather_talk(shared, request),
            AppService::EditCreation => edit_creation(shared, request),
            AppService::Creation if request.text("op") == Some(OP_OPTION) => {
                let key = request
                    .text("key")
                    .ok_or("`option` needs the `key` of the setting it answers")?
                    .to_owned();
                let value = request
                    .args
                    .get("value")
                    .map(ExprValue::to_stored)
                    .ok_or("`option` needs the `value` chosen")?;
                let loaded = &shared.loaded;
                shared
                    .setup
                    .as_mut()
                    .ok_or("no game is being set up to answer an option of")?
                    .give(loaded, &key, value)
            }
            AppService::Creation => {
                let defs = std::sync::Arc::clone(&shared.loaded.defs);
                shared.creation.service(&defs, request)
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
                let Some(session) = shared.session.as_mut() else {
                    return Err("no game is being played, so there is nothing to save".to_owned());
                };
                if session.saving != POLICY_FREE {
                    return Err(format!(
                        "this game cannot be saved by hand: its saving policy is `{}` (13 §9.2)",
                        session.saving
                    ));
                }
                if let Some(outcome) = &session.ended {
                    return Err(format!(
                        "this game has ended, {outcome}: there is nothing to save"
                    ));
                }
                let name = request
                    .text("name")
                    .filter(|name| !name.trim().is_empty())
                    .map_or_else(|| session.default_save_name(), str::to_owned);
                let meta = session.save_meta();
                let id = shared
                    .saves
                    .save(dir.as_deref(), &mut session.sim, &meta, &name)?;
                shared.said.push(format!("Saved as “{}” ({id}).", name.trim()));
                Ok(())
            }
            AppService::Load => {
                let save = request
                    .text("save")
                    .ok_or("`load` needs the id of the `save` to load")?;
                if let Some(session) = &shared.session
                    && session.saving != POLICY_FREE
                {
                    return Err(format!(
                        "another game cannot be loaded into this one: its saving policy is `{}` (13 §9.2)",
                        session.saving
                    ));
                }
                let saved = shared.saves.load(dir.as_deref(), &shared.loaded, save)?;
                let warnings = saved.warnings.clone();
                let session = session::resume(&shared.loaded, saved)?;
                shared.session = Some(session);
                shared.talk = ExprValue::Null;
                shared.started = true;
                shared.warnings = warnings;
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
