//! A small reader for definition files, until the mod loader is joined
//! to this crate.
//!
//! `clash-data` is the engine's loader (document 04): it merges mods,
//! resolves ids and reports every mistake with file, line and JSON path.
//! This module does none of that. It reads files in the format of 04 §5
//! (a JSON array of objects, or one object, each with `"type"` and
//! `"id"`), drops `"//"` comments, and deserialises the definition types
//! of this crate, so that tests and the widget gallery can load the `ui`
//! mod. It is to be replaced by the real loader, at which point the
//! definition types here are handed to it unchanged.
//!
//! The reader gives a definition's object to its type without the
//! `"type"` and `"id"` keys.

use std::collections::BTreeMap;
use std::path::Path;

use serde::de::DeserializeOwned;
use serde_json::Value;
use thiserror::Error;

use crate::color::ColorDef;
use crate::input::KeymapDef;
use crate::scene::{NoHost, Scene};
use crate::theme::{Theme, ThemeDef};
use crate::ui::{FigureDef, Screen, ScreenDef, UiEnv};

/// A mistake in a definition file.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[error("{file}: {what}: {message}")]
pub struct DefError {
    /// The file, as it was named to the reader.
    pub file: String,
    /// Which definition, such as ``screen `gallery` ``, or `file` for a
    /// mistake in the file as a whole.
    pub what: String,
    /// What is wrong.
    pub message: String,
}

/// One object of a definition file, not yet interpreted.
#[derive(Debug, Clone, PartialEq)]
pub struct RawDef {
    /// The value of `"type"`.
    pub kind: String,
    /// The value of `"id"`.
    pub id: String,
    /// The rest of the object, without comments.
    pub body: Value,
}

fn strip_comments(value: &mut Value) {
    match value {
        Value::Object(map) => {
            map.remove("//");
            map.values_mut().for_each(strip_comments);
        }
        Value::Array(items) => items.iter_mut().for_each(strip_comments),
        _ => {}
    }
}

/// Reads a definition file into its objects. Mistakes are collected; the
/// objects that could be read are returned alongside them.
pub fn read_defs(file: &str, text: &str) -> (Vec<RawDef>, Vec<DefError>) {
    let error = |what: String, message: String| DefError {
        file: file.to_owned(),
        what,
        message,
    };
    let parsed: Value = match serde_json::from_str(text) {
        Ok(value) => value,
        Err(e) => return (Vec::new(), vec![error("file".to_owned(), e.to_string())]),
    };
    let objects = match parsed {
        Value::Array(items) => items,
        object @ Value::Object(_) => vec![object],
        _ => {
            let message = "a definition file is a list of objects, or one object".to_owned();
            return (Vec::new(), vec![error("file".to_owned(), message)]);
        }
    };
    let mut defs = Vec::new();
    let mut errors = Vec::new();
    for (index, mut object) in objects.into_iter().enumerate() {
        let what = format!("object {index}");
        strip_comments(&mut object);
        let Value::Object(map) = &mut object else {
            errors.push(error(what, "not an object".to_owned()));
            continue;
        };
        let mut take = |key: &str| match map.remove(key) {
            Some(Value::String(text)) => Ok(text),
            Some(_) => Err(format!("`{key}` must be a text")),
            None => Err(format!("missing `{key}`")),
        };
        match (take("type"), take("id")) {
            (Ok(kind), Ok(id)) => defs.push(RawDef {
                kind,
                id,
                body: object,
            }),
            (Err(message), _) | (_, Err(message)) => errors.push(error(what, message)),
        }
    }
    (defs, errors)
}

/// The definitions of this crate's types read so far, by id.
#[derive(Debug, Clone, Default)]
pub struct UiDefs {
    /// `color` definitions.
    pub colors: BTreeMap<String, ColorDef>,
    /// `theme` definitions.
    pub themes: BTreeMap<String, ThemeDef>,
    /// `screen` definitions.
    pub screens: BTreeMap<String, ScreenDef>,
    /// `keymap` definitions.
    pub keymaps: BTreeMap<String, KeymapDef>,
    /// `figure` definitions.
    pub figures: BTreeMap<String, FigureDef>,
    /// The text of each art asset, by its path within the mod, such as
    /// `assets/title.txt`.
    pub art: BTreeMap<String, String>,
}

fn typed<T: DeserializeOwned>(
    def: &RawDef,
    file: &str,
    into: &mut BTreeMap<String, T>,
) -> Result<(), DefError> {
    let what = format!("{} `{}`", def.kind, def.id);
    let error = |message: String| DefError {
        file: file.to_owned(),
        what: what.clone(),
        message,
    };
    if into.contains_key(&def.id) {
        return Err(error("defined twice".to_owned()));
    }
    let value = serde_json::from_value(def.body.clone()).map_err(|e| error(e.to_string()))?;
    into.insert(def.id.clone(), value);
    Ok(())
}

impl UiDefs {
    /// Reads one definition file. Every mistake is reported; definitions
    /// without mistakes are kept.
    pub fn add_file(&mut self, file: &str, text: &str) -> Vec<DefError> {
        let (defs, mut errors) = read_defs(file, text);
        for def in defs {
            let result = match def.kind.as_str() {
                "color" => typed(&def, file, &mut self.colors),
                "theme" => typed(&def, file, &mut self.themes),
                "screen" => typed(&def, file, &mut self.screens),
                "keymap" => typed(&def, file, &mut self.keymaps),
                "figure" => typed(&def, file, &mut self.figures),
                other => Err(DefError {
                    file: file.to_owned(),
                    what: format!("{other} `{}`", def.id),
                    message: format!(
                        "this reader knows the types color, theme, screen, keymap and figure, not `{other}`"
                    ),
                }),
            };
            errors.extend(result.err());
        }
        errors
    }

    /// Reads a mod directory: every `.json` file under `data/` as
    /// definitions and every file under `assets/` whose name ends in
    /// `.txt` as art, in sorted order.
    pub fn load_mod(dir: &Path) -> Result<UiDefs, Vec<DefError>> {
        fn walk(dir: &Path, out: &mut Vec<std::path::PathBuf>) -> std::io::Result<()> {
            let mut entries: Vec<_> = std::fs::read_dir(dir)?.collect::<Result<_, _>>()?;
            entries.sort_by_key(std::fs::DirEntry::path);
            for entry in entries {
                let path = entry.path();
                if path.is_dir() {
                    walk(&path, out)?;
                } else {
                    out.push(path);
                }
            }
            Ok(())
        }
        let io_error = |path: &Path, error: std::io::Error| DefError {
            file: path.display().to_string(),
            what: "file".to_owned(),
            message: error.to_string(),
        };
        let mut defs = UiDefs::default();
        let mut errors = Vec::new();
        for (sub, extension) in [("data", "json"), ("assets", "txt")] {
            let root = dir.join(sub);
            if !root.is_dir() {
                continue;
            }
            let mut files = Vec::new();
            if let Err(error) = walk(&root, &mut files) {
                errors.push(io_error(&root, error));
                continue;
            }
            for path in files {
                if path.extension().and_then(|e| e.to_str()) != Some(extension) {
                    continue;
                }
                let text = match std::fs::read_to_string(&path) {
                    Ok(text) => text,
                    Err(error) => {
                        errors.push(io_error(&path, error));
                        continue;
                    }
                };
                // Name files relative to the mod, with forward slashes,
                // so that messages and asset names do not depend on
                // where the mod is installed.
                let name = path
                    .strip_prefix(dir)
                    .unwrap_or(&path)
                    .components()
                    .map(|part| part.as_os_str().to_string_lossy())
                    .collect::<Vec<_>>()
                    .join("/");
                if extension == "json" {
                    errors.extend(defs.add_file(&name, &text));
                } else {
                    defs.art.insert(name, text);
                }
            }
        }
        if errors.is_empty() {
            Ok(defs)
        } else {
            Err(errors)
        }
    }

    /// Builds the theme with this id against the colours read.
    pub fn theme(&self, id: &str) -> Result<Theme, Vec<DefError>> {
        let what = format!("theme `{id}`");
        let error = |message: String| DefError {
            file: "definitions".to_owned(),
            what: what.clone(),
            message,
        };
        let def = self
            .themes
            .get(id)
            .ok_or_else(|| vec![error("not defined".to_owned())])?;
        Theme::build(def, &self.colors)
            .map_err(|errors| errors.iter().map(|e| error(e.to_string())).collect())
    }

    /// A scene with no simulation behind it, using the theme and the
    /// keymap with these ids.
    pub fn scene(&self, theme: &str, keymap: &str) -> Result<Scene<NoHost>, Vec<DefError>> {
        let theme = self.theme(theme)?;
        let keymap = self.keymaps.get(keymap).cloned().ok_or_else(|| {
            vec![DefError {
                file: "definitions".to_owned(),
                what: format!("keymap `{keymap}`"),
                message: "not defined".to_owned(),
            }]
        })?;
        Ok(Scene::new(NoHost, theme, keymap))
    }

    /// Makes the screen with this id, styled by `theme`.
    pub fn screen(&self, id: &str, theme: &Theme) -> Result<Screen, Vec<DefError>> {
        let what = format!("screen `{id}`");
        let error = |message: String| DefError {
            file: "definitions".to_owned(),
            what: what.clone(),
            message,
        };
        let def = self
            .screens
            .get(id)
            .ok_or_else(|| vec![error("not defined".to_owned())])?;
        let env = UiEnv {
            theme,
            figures: &self.figures,
            art: &self.art,
        };
        Screen::new(id, def, &env).map_err(|errors| {
            errors
                .iter()
                .map(|e| error(format!("at `{}`: {}", e.path, e.problem)))
                .collect()
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_file_is_a_list_or_one_object_and_comments_are_dropped() {
        let (defs, errors) = read_defs(
            "a.json",
            r##"[ { "//": "a comment", "type": "color", "id": "crimson", "rgb": "#a01828" },
                 { "type": "theme", "id": "default", "colors": { "//": "inner comment", "text": "#c8c8c8" } } ]"##,
        );
        assert!(errors.is_empty(), "{errors:?}");
        assert_eq!(defs.len(), 2);
        assert_eq!(
            (defs[0].kind.as_str(), defs[0].id.as_str()),
            ("color", "crimson")
        );
        assert_eq!(defs[0].body, serde_json::json!({ "rgb": "#a01828" }));
        assert_eq!(
            defs[1].body,
            serde_json::json!({ "colors": { "text": "#c8c8c8" } })
        );

        let (one, errors) = read_defs(
            "b.json",
            r##"{ "type": "color", "id": "x", "rgb": "#000000" }"##,
        );
        assert!(errors.is_empty());
        assert_eq!(one.len(), 1);
    }

    #[test]
    fn mistakes_are_reported_with_file_and_definition() {
        let mut defs = UiDefs::default();
        let errors = defs.add_file(
            "mods/x/data/ui.json",
            r##"[ { "type": "color", "id": "good", "rgb": "#102030" },
                 { "type": "color", "id": "bad", "rgb": "#nothex" },
                 { "type": "color", "rgb": "#000000" },
                 { "type": "screen", "id": "s", "layout": { "lst": {} } },
                 { "type": "material", "id": "ash" },
                 { "type": "color", "id": "good", "rgb": "#000000" },
                 7 ]"##,
        );
        let text: Vec<String> = errors.iter().map(ToString::to_string).collect();
        assert_eq!(text.len(), 6, "{text:#?}");
        let has = |needle: &str| text.iter().any(|line| line.contains(needle));
        assert!(
            has("mods/x/data/ui.json: color `bad`: `#nothex` is not a colour"),
            "{text:#?}"
        );
        assert!(has("object 2: missing `id`"), "{text:#?}");
        assert!(has("screen `s`: unknown key `lst`"), "{text:#?}");
        assert!(has("material `ash`"), "{text:#?}");
        assert!(has("color `good`: defined twice"), "{text:#?}");
        assert!(has("object 6: not an object"), "{text:#?}");
        assert_eq!(defs.colors.len(), 1);

        let errors = UiDefs::default().add_file("broken.json", "[ {");
        assert_eq!(errors.len(), 1);
        assert!(
            errors[0].to_string().starts_with("broken.json: file: "),
            "{}",
            errors[0]
        );
    }

    #[test]
    fn a_missing_theme_or_screen_is_an_error() {
        let defs = UiDefs::default();
        assert!(
            defs.theme("default").unwrap_err()[0]
                .to_string()
                .contains("theme `default`: not defined")
        );
        let errors = defs.screen("title", &Theme::default()).unwrap_err();
        assert!(
            errors[0]
                .to_string()
                .contains("screen `title`: not defined")
        );
    }
}
