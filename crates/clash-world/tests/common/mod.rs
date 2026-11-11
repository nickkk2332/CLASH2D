//! Test support shared by this crate's tests: the world content of
//! `mods/core`, loaded through the mod loader (`clash-data`) with the
//! world's definition types registered.
//!
//! Only the files under `mods/core/data/world/` are read, as a mod named
//! `core`, so that these tests do not depend on definition types other
//! crates own. Definitions a test writes inline are loaded the same way,
//! as a mod of their own.

// Each test binary uses its own part of this module.
#![allow(dead_code)]

use std::path::{Path, PathBuf};
use std::sync::Arc;

use clash_data::{DefTypes, Defs, Loader};
use clash_world::WorldDefs;
use clash_world::defs::RawWorldDefs;

/// The repository's `mods` directory.
pub fn mods_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../mods")
}

/// Where the core mod's world definitions are.
pub fn core_dir() -> PathBuf {
    mods_dir().join("core/data/world")
}

/// The world's definition types, registered.
pub fn world_types() -> DefTypes {
    let mut types = DefTypes::new();
    clash_world::register_defs(&mut types);
    types
}

/// The manifest of a mod with this id and nothing else.
pub fn manifest(id: &str) -> String {
    format!(r#"{{ "id": "{id}" }}"#)
}

/// The world definition files of `mods/core`, as paths within the mod
/// and their text, in file-name order.
pub fn core_files() -> Vec<(String, String)> {
    let dir = core_dir();
    let mut files: Vec<PathBuf> = std::fs::read_dir(&dir)
        .unwrap_or_else(|error| panic!("{}: {error}", dir.display()))
        .filter_map(|entry| entry.ok().map(|entry| entry.path()))
        .filter(|path| {
            path.extension()
                .is_some_and(|extension| extension == "json")
        })
        .collect();
    files.sort();
    files
        .iter()
        .map(|file| {
            let name = file.file_name().unwrap_or_default().to_string_lossy();
            let text = std::fs::read_to_string(file)
                .unwrap_or_else(|error| panic!("{}: {error}", file.display()));
            (format!("data/world/{name}"), text)
        })
        .collect()
}

/// A loader that has the world content of `mods/core` as the mod `core`.
/// A test adds mods of its own with `memory_mod` and loads.
pub fn loader(types: &DefTypes) -> Loader<'_> {
    let files = core_files();
    let files: Vec<(&str, &str)> = files
        .iter()
        .map(|(path, text)| (path.as_str(), text.as_str()))
        .collect();
    Loader::new(types).memory_mod("core", &manifest("core"), &files)
}

/// The loaded definitions of `mods/core`'s world content.
pub fn core_defs() -> Arc<Defs> {
    let types = world_types();
    match loader(&types).load_mods(&["core"]).into_result() {
        Ok(defs) => defs,
        Err(diagnostics) => panic!("mods/core does not load:\n{}", diagnostics.render()),
    }
}

/// Loads the definitions in one JSON text, as a mod named `namespace`
/// and so defined in that namespace, and adds them to `raw`, or says why
/// not.
pub fn try_add_json(raw: &mut RawWorldDefs, namespace: &str, text: &str) -> Result<(), String> {
    let types = world_types();
    let outcome = Loader::new(&types)
        .memory_mod(
            namespace,
            &manifest(namespace),
            &[("data/inline.json", text)],
        )
        .load_mods(&[namespace]);
    let defs = outcome
        .into_result()
        .map_err(|diagnostics| diagnostics.render())?;
    let more = RawWorldDefs::from_defs(&defs);
    raw.damage_types.extend(more.damage_types);
    raw.materials.extend(more.materials);
    raw.atmospheres.extend(more.atmospheres);
    raw.floors.extend(more.floors);
    raw.blocks.extend(more.blocks);
    raw.fixtures.extend(more.fixtures);
    raw.realms.extend(more.realms);
    raw.planes.extend(more.planes);
    raw.area_kinds.extend(more.area_kinds);
    raw.skies.extend(more.skies);
    Ok(())
}

/// Loads the definitions in one JSON text into `raw`. Panics on a
/// mistake, naming `origin`.
pub fn add_json(raw: &mut RawWorldDefs, namespace: &str, text: &str, origin: &str) {
    if let Err(error) = try_add_json(raw, namespace, text) {
        panic!("{origin}:\n{error}");
    }
}

/// The raw definitions of `mods/core`'s world content.
pub fn core_raw() -> RawWorldDefs {
    RawWorldDefs::from_defs(&core_defs())
}

/// The raw definitions the tests build on: those of `mods/core`, which
/// hold two small realms for tests, `core:testland` and
/// `core:training_room`.
pub fn test_raw() -> RawWorldDefs {
    core_raw()
}

/// The built definitions of `mods/core`'s world content.
pub fn test_defs() -> Arc<WorldDefs> {
    match WorldDefs::from_defs(&core_defs()) {
        Ok(defs) => defs,
        Err(diagnostics) => panic!(
            "the test definitions do not build:\n{}",
            diagnostics.render()
        ),
    }
}
