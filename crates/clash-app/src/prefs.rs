//! What the player chose, kept between runs: preferences and key
//! bindings (12 §9.2, §10.4).
//!
//! The scene changes a preference or a binding itself and then tells
//! the application through the `set_pref` and `rebind` services. The
//! application writes each to a small JSON file in the user's data
//! directory ([`crate::paths`]), and gives them back to the scene when
//! it starts: preferences through `Scene::set_pref`, bindings through
//! `Scene::rebind`.
//!
//! ```json
//! { "preferences": { "ui:theme": "ui:plain", "ui:playback_speed": 2 },
//!   "bindings": [ { "keymap": "ui:default", "context": "play", "key": "w", "name": "north" } ] }
//! ```
//!
//! A preference is kept by its qualified id. One whose definition is
//! not loaded (another profile's) stays in the file untouched. A
//! binding is kept with the keymap it was made in, and given back only
//! while that keymap is the one in force.
//!
//! A file that cannot be read or is not what this module writes is
//! reported once and then left alone: nothing is written over it.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use clash_core::{Real, Value};
use serde_json::{Map, Value as Json, json};

/// The file preferences and bindings are kept in, in the user's data
/// directory.
pub const PREFS_FILE: &str = "preferences.json";

/// One key the player gave a meaning.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Binding {
    /// The qualified id of the keymap it was made in; empty where the
    /// profile has none.
    pub keymap: String,
    /// The context of the keymap.
    pub context: String,
    /// The key, as a keymap writes it.
    pub key: String,
    /// What it means there; empty for a key whose meaning was taken
    /// away.
    pub name: String,
}

/// The player's stored choices, and where they are kept.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct PrefStore {
    /// The file, if there is somewhere to keep it.
    path: Option<PathBuf>,
    /// Preferences by qualified id.
    prefs: BTreeMap<String, Value>,
    /// Bindings, oldest first.
    bindings: Vec<Binding>,
    /// Whether the file may be written: false once it was found to hold
    /// something else.
    writable: bool,
}

impl PrefStore {
    /// A store that keeps nothing between runs.
    pub fn in_memory() -> PrefStore {
        PrefStore {
            writable: true,
            ..PrefStore::default()
        }
    }

    /// Opens the store in a data directory, reading what it holds. A
    /// problem is returned with the store, to be shown; the store then
    /// keeps choices for the run only.
    pub fn open(dir: &Path) -> (PrefStore, Option<String>) {
        let path = dir.join(PREFS_FILE);
        let mut store = PrefStore {
            path: Some(path.clone()),
            writable: true,
            ..PrefStore::default()
        };
        let text = match std::fs::read_to_string(&path) {
            Ok(text) => text,
            // No file yet: nothing was chosen.
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return (store, None),
            Err(error) => {
                store.writable = false;
                let problem = format!("cannot read `{}`: {error}", path.display());
                return (store, Some(problem));
            }
        };
        match store.read(&text) {
            Ok(()) => (store, None),
            Err(reason) => {
                store.writable = false;
                let problem = format!(
                    "`{}` is not a preferences file ({reason}); it was left as it is and choices will last for this run only",
                    path.display()
                );
                (store, Some(problem))
            }
        }
    }

    /// Reads the file's text into the store.
    fn read(&mut self, text: &str) -> Result<(), String> {
        let root: Json = serde_json::from_str(text).map_err(|error| error.to_string())?;
        let root = root.as_object().ok_or("it does not hold an object")?;
        if let Some(prefs) = root.get("preferences") {
            let prefs = prefs.as_object().ok_or("`preferences` is not an object")?;
            for (id, value) in prefs {
                let value = from_json(value).ok_or_else(|| {
                    format!("the preference `{id}` is not a flag, a number or a text")
                })?;
                self.prefs.insert(id.clone(), value);
            }
        }
        if let Some(bindings) = root.get("bindings") {
            let bindings = bindings.as_array().ok_or("`bindings` is not a list")?;
            for binding in bindings {
                let text = |field: &str| {
                    binding
                        .get(field)
                        .and_then(Json::as_str)
                        .map(str::to_owned)
                        .ok_or_else(|| format!("a binding has no text `{field}`"))
                };
                self.bindings.push(Binding {
                    keymap: text("keymap")?,
                    context: text("context")?,
                    key: text("key")?,
                    name: text("name")?,
                });
            }
        }
        Ok(())
    }

    /// The stored preferences, by qualified id.
    pub fn prefs(&self) -> &BTreeMap<String, Value> {
        &self.prefs
    }

    /// The stored bindings, oldest first.
    pub fn bindings(&self) -> &[Binding] {
        &self.bindings
    }

    /// Keeps a preference's value.
    pub fn set_pref(&mut self, id: &str, value: Value) -> Result<(), String> {
        if self.prefs.get(id) == Some(&value) {
            return Ok(());
        }
        self.prefs.insert(id.to_owned(), value);
        self.write()
    }

    /// Keeps a binding. A later binding of the same key in the same
    /// context of the same keymap replaces the earlier one.
    pub fn rebind(&mut self, binding: Binding) -> Result<(), String> {
        self.bindings.retain(|have| {
            (&have.keymap, &have.context, &have.key)
                != (&binding.keymap, &binding.context, &binding.key)
        });
        self.bindings.push(binding);
        self.write()
    }

    /// The store as the file holds it.
    fn to_json(&self) -> Json {
        let prefs: Map<String, Json> = self
            .prefs
            .iter()
            .map(|(id, value)| (id.clone(), to_json(value)))
            .collect();
        let bindings: Vec<Json> = self
            .bindings
            .iter()
            .map(|binding| {
                json!({
                    "keymap": binding.keymap,
                    "context": binding.context,
                    "key": binding.key,
                    "name": binding.name,
                })
            })
            .collect();
        json!({ "preferences": prefs, "bindings": bindings })
    }

    /// Writes the file, through a temporary one beside it so that a
    /// write that fails leaves the old file whole.
    fn write(&self) -> Result<(), String> {
        let Some(path) = &self.path else {
            return Ok(());
        };
        if !self.writable {
            return Ok(());
        }
        let failed = |error: std::io::Error| {
            format!(
                "the choice could not be kept in `{}`: {error}",
                path.display()
            )
        };
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).map_err(failed)?;
        }
        let mut text = serde_json::to_string_pretty(&self.to_json())
            .map_err(|error| failed(std::io::Error::other(error)))?;
        text.push('\n');
        let temporary = path.with_extension("json.tmp");
        std::fs::write(&temporary, text).map_err(failed)?;
        std::fs::rename(&temporary, path).map_err(failed)
    }
}

/// A stored value as JSON.
fn to_json(value: &Value) -> Json {
    match value {
        Value::Bool(flag) => Json::Bool(*flag),
        Value::Int(number) => json!(number),
        Value::Real(number) => json!(number.get()),
        Value::Str(text) => Json::String(text.clone()),
        _ => Json::Null,
    }
}

/// A value a preference can hold, out of JSON.
fn from_json(value: &Json) -> Option<Value> {
    match value {
        Json::Bool(flag) => Some(Value::Bool(*flag)),
        Json::String(text) => Some(Value::Str(text.clone())),
        Json::Number(number) => match number.as_i64() {
            Some(whole) => Some(Value::Int(whole)),
            None => Real::new(number.as_f64()?).ok().map(Value::Real),
        },
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("clash-app-prefs-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    #[test]
    fn choices_are_kept_and_read_back() {
        let dir = scratch("kept");
        let (mut store, problem) = PrefStore::open(&dir);
        assert_eq!(problem, None);
        store
            .set_pref("ui:theme", Value::Str("ui:plain".to_owned()))
            .unwrap();
        store.set_pref("ui:playback_speed", Value::Int(2)).unwrap();
        store
            .set_pref("other:half", Value::Real(Real::of(0.5)))
            .unwrap();
        let west = |name: &str| Binding {
            keymap: "ui:default".to_owned(),
            context: "play".to_owned(),
            key: "a".to_owned(),
            name: name.to_owned(),
        };
        store.rebind(west("west")).unwrap();
        store.rebind(west("east")).unwrap();

        let (again, problem) = PrefStore::open(&dir);
        assert_eq!(problem, None);
        assert_eq!(again.prefs(), store.prefs());
        assert_eq!(
            again.bindings(),
            [west("east")],
            "the later binding replaced the earlier"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_file_that_is_something_else_is_reported_and_left_alone() {
        let dir = scratch("other");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join(PREFS_FILE);
        std::fs::write(&path, "[1, 2, 3]").unwrap();
        let (mut store, problem) = PrefStore::open(&dir);
        assert!(problem.unwrap().contains("is not a preferences file"));
        store
            .set_pref("ui:theme", Value::Str("x".to_owned()))
            .unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "[1, 2, 3]");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_store_without_a_directory_keeps_choices_for_the_run() {
        let mut store = PrefStore::in_memory();
        store.set_pref("a:b", Value::Bool(true)).unwrap();
        assert_eq!(store.prefs().get("a:b"), Some(&Value::Bool(true)));
    }
}
