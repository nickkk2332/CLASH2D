//! Support shared by this crate's tests and its `city_ascii` example:
//! the world content of `mods/core`, loaded through the mod loader, and
//! the block of `test_city` the example prints.
//!
//! Only the files under `mods/core/data/world/` are read, as a mod named
//! `core`, so that nothing here depends on definition types other crates
//! own. The engine names no content; this file and the JSON it reads do.

// Each test binary, and the example, uses its own part of this module.
#![allow(dead_code)]

mod maps;
#[allow(unused_imports, reason = "each test file uses its own part of this")]
pub use maps::*;
mod standard;
#[allow(unused_imports, reason = "each test file uses its own part of this")]
pub use standard::*;

use std::path::{Path, PathBuf};
use std::sync::Arc;

use clash_data::{DefTypes, Loader};
use clash_gen::ascii::storey_text;
use clash_gen::realize_tracts;
use clash_world::{Column, RealmDefId, TractPos, WorldDefs};

/// The realm definition of the test city.
pub const CITY_REALM: &str = "core:test_city";

/// The realm definition of flat ground.
pub const FLAT_REALM: &str = "core:testland";

/// The repository's `mods` directory.
pub fn mods_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../mods")
}

/// The world definition files of `mods/core`, as paths within the mod
/// and their text, in file-name order.
fn core_files() -> Result<Vec<(String, String)>, String> {
    let dir = mods_dir().join("core/data/world");
    let entries = std::fs::read_dir(&dir).map_err(|error| format!("{}: {error}", dir.display()))?;
    let mut files: Vec<PathBuf> = entries
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
                .map_err(|error| format!("{}: {error}", file.display()))?;
            Ok((format!("data/world/{name}"), text))
        })
        .collect()
}

/// The world definitions of `mods/core`, with the files of one more mod
/// laid over them if any are given: `(path within the mod, text)`.
pub fn try_defs_with(extra: &[(&str, &str)]) -> Result<Arc<WorldDefs>, String> {
    let mut types = DefTypes::new();
    clash_world::register_defs(&mut types);
    let files = core_files()?;
    let files: Vec<(&str, &str)> = files
        .iter()
        .map(|(path, text)| (path.as_str(), text.as_str()))
        .collect();
    let mut loader = Loader::new(&types).memory_mod("core", r#"{ "id": "core" }"#, &files);
    let mut mods = vec!["core"];
    if !extra.is_empty() {
        loader = loader.memory_mod("extra", r#"{ "id": "extra", "requires": ["core"] }"#, extra);
        mods.push("extra");
    }
    let defs = loader
        .load_mods(&mods)
        .into_result()
        .map_err(|diagnostics| diagnostics.render())?;
    WorldDefs::from_defs(&defs).map_err(|diagnostics| diagnostics.render())
}

/// The world definitions of `mods/core`.
pub fn core_defs() -> Arc<WorldDefs> {
    match try_defs_with(&[]) {
        Ok(defs) => defs,
        Err(error) => panic!("mods/core does not load:\n{error}"),
    }
}

/// A realm definition by its qualified id.
pub fn realm_def(defs: &WorldDefs, id: &str) -> RealmDefId {
    defs.realms()
        .lookup(id)
        .unwrap_or_else(|| panic!("there is no realm definition `{id}`"))
}

/// The tracts of a block `side` tracts on a side, from the origin, row
/// by row.
pub fn block(side: i32) -> Vec<TractPos> {
    (0..side)
        .flat_map(|y| (0..side).map(move |x| TractPos::new(x, y)))
        .collect()
}

/// How many tracts on a side the block the example prints is.
pub const CITY_BLOCK_SIDE: i32 = 2;

/// One storey of a block of the test city as text: what the
/// `city_ascii` example prints. `storey` counts from the ground storey.
pub fn city_text(seed: u64, storey: i32) -> Result<String, String> {
    let defs = try_defs_with(&[])?;
    let generators = clash_gen::generators(&defs);
    let def = defs
        .realms()
        .lookup(CITY_REALM)
        .ok_or_else(|| format!("there is no realm definition `{CITY_REALM}`"))?;
    let surface_z = defs
        .realms()
        .get(def)
        .and_then(|kind| kind.generator_params.get("surface_z"))
        .and_then(|value| value.as_i64())
        .and_then(|z| i32::try_from(z).ok())
        .unwrap_or(0);
    let tracts = block(CITY_BLOCK_SIDE);
    let realised =
        realize_tracts(&generators, def, seed, &tracts).map_err(|error| error.to_string())?;
    let columns: Vec<(TractPos, &Column)> = tracts
        .iter()
        .copied()
        .zip(realised.iter().map(|tract| &tract.column))
        .collect();
    Ok(storey_text(
        &defs,
        &columns,
        surface_z.saturating_add(storey),
    ))
}
