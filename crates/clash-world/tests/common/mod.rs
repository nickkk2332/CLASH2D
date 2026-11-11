//! Test support shared by this crate's tests: a tiny reader of definition
//! files in the format of `docs/04-data-and-modding.md` section 5, and the
//! test content of `mods/core/world`.
//!
//! This is not the mod loader. It reads a JSON array of objects, takes
//! each object's `"type"` and `"id"`, drops comments, and hands the rest
//! to the definition struct of that type. It knows nothing of inheritance,
//! patches or load order, and because it goes through `serde_json`'s
//! value tree it sees the keys of an object in alphabetical order, not as
//! written.

// Each test binary uses its own part of this module.
#![allow(dead_code)]

use std::path::{Path, PathBuf};
use std::sync::Arc;

use clash_world::WorldDefs;
use clash_world::defs::RawWorldDefs;
use serde_json::Value;

/// Where the core mod's world definitions are.
pub fn core_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../mods/core/world")
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

/// Reads the definitions in one JSON text into `raw`, or says why not.
/// Bare ids are defined in `namespace`.
pub fn try_add_json(raw: &mut RawWorldDefs, namespace: &str, text: &str) -> Result<(), String> {
    let value: Value = serde_json::from_str(text).map_err(|error| error.to_string())?;
    let objects = match value {
        Value::Array(objects) => objects,
        single => vec![single],
    };
    for mut object in objects {
        strip_comments(&mut object);
        let map = object.as_object_mut().ok_or("a definition is an object")?;
        let take = |map: &mut serde_json::Map<String, Value>, key: &str| {
            map.remove(key)
                .and_then(|value| value.as_str().map(str::to_owned))
                .ok_or(format!("a definition needs a string `{key}`"))
        };
        let kind = take(map, "type")?;
        let id = take(map, "id")?;
        let id = if id.contains(':') {
            id
        } else {
            format!("{namespace}:{id}")
        };
        let parse_error = |error: serde_json::Error| format!("{kind} {id}: {error}");
        match kind.as_str() {
            "damage_type" => raw.damage_types.push((
                id.clone(),
                serde_json::from_value(object).map_err(parse_error)?,
            )),
            "material" => raw.materials.push((
                id.clone(),
                serde_json::from_value(object).map_err(parse_error)?,
            )),
            "atmosphere" => raw.atmospheres.push((
                id.clone(),
                serde_json::from_value(object).map_err(parse_error)?,
            )),
            "floor" => raw.floors.push((
                id.clone(),
                serde_json::from_value(object).map_err(parse_error)?,
            )),
            "block" => raw.blocks.push((
                id.clone(),
                serde_json::from_value(object).map_err(parse_error)?,
            )),
            "fixture" => raw.fixtures.push((
                id.clone(),
                serde_json::from_value(object).map_err(parse_error)?,
            )),
            "realm" => raw.realms.push((
                id.clone(),
                serde_json::from_value(object).map_err(parse_error)?,
            )),
            other => return Err(format!("unknown definition type `{other}`")),
        }
    }
    Ok(())
}

/// Reads the definitions in one JSON text into `raw`. Panics on a
/// mistake, naming `origin`.
pub fn add_json(raw: &mut RawWorldDefs, namespace: &str, text: &str, origin: &str) {
    if let Err(error) = try_add_json(raw, namespace, text) {
        panic!("{origin}: {error}");
    }
}

/// Reads every `*.json` file of a directory, in file-name order.
pub fn read_dir(dir: &Path, namespace: &str) -> RawWorldDefs {
    let mut files: Vec<PathBuf> = std::fs::read_dir(dir)
        .unwrap_or_else(|error| panic!("{}: {error}", dir.display()))
        .filter_map(|entry| entry.ok().map(|entry| entry.path()))
        .filter(|path| {
            path.extension()
                .is_some_and(|extension| extension == "json")
        })
        .collect();
    files.sort();
    let mut raw = RawWorldDefs::default();
    for file in files {
        let text = std::fs::read_to_string(&file)
            .unwrap_or_else(|error| panic!("{}: {error}", file.display()));
        add_json(&mut raw, namespace, &text, &file.display().to_string());
    }
    raw
}

/// The raw definitions of `mods/core/world`.
pub fn core_raw() -> RawWorldDefs {
    read_dir(&core_dir(), "core")
}

/// A small realm for tests: two regions by one, wrapping east to west,
/// with thin air high up and vacuum above that.
pub const TEST_REALMS: &str = r#"[
  { "type": "realm", "id": "testland",
    "size_regions": [2, 1],
    "wrap": ["x"],
    "z_range": [-16, 200],
    "generator": "flat",
    "fill": { "below": "solid@granite", "above": null },
    "strata": [
      { "from_z": -16, "gravity": 9.81, "atmosphere": "air", "sky": "day_night" },
      { "from_z": 100, "gravity": 9.81, "atmosphere": "thin_air", "temp_offset": -40 },
      { "from_z": 150, "gravity": 0.0, "atmosphere": null, "temp": "-120 C" }
    ],
    "time_rate": 1.0,
    "edges": { "top": "void", "bottom": "barrier", "north": "barrier", "south": "void" } },

  { "type": "realm", "id": "training_room",
    "size_regions": [1, 1],
    "wrap": [],
    "z_range": [0, 64],
    "generator": "white_void",
    "strata": [ { "from_z": 0, "gravity": 98.1, "atmosphere": "air" } ],
    "time_rate": 365.0,
    "edges": { "top": "barrier", "bottom": "barrier", "north": "barrier",
               "south": "barrier", "east": "barrier", "west": "barrier" } }
]"#;

/// The raw definitions of `mods/core/world` with the test realms added.
pub fn test_raw() -> RawWorldDefs {
    let mut raw = core_raw();
    add_json(&mut raw, "core", TEST_REALMS, "TEST_REALMS");
    raw
}

/// The built definitions of `mods/core/world` with the test realms.
pub fn test_defs() -> Arc<WorldDefs> {
    match WorldDefs::build(test_raw()) {
        Ok(defs) => Arc::new(defs),
        Err(errors) => {
            let lines: Vec<String> = errors.iter().map(ToString::to_string).collect();
            panic!("the test definitions do not build:\n{}", lines.join("\n"));
        }
    }
}
