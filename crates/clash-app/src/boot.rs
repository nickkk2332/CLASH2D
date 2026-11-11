//! Start-up: loading a profile into everything the application runs
//! on (04 §3.3, §9).
//!
//! [`load`] reads the profile's mods with the engine's definition types
//! and its expression registry (`clash_scene::engine_loader`), and
//! builds from the result the interface's definitions, the world's
//! tables and the generation definitions. Whatever is wrong is returned
//! as located diagnostics, rendered: file, line, column and JSON path.
//! Nothing here panics on bad data (01 R6).
//!
//! A profile need not hold a world. One that defines no way to start a
//! game is not asked for world tables that build: a total conversion
//! may be a title screen and nothing else (04 §14).

use std::path::{Path, PathBuf};
use std::sync::Arc;

use clash_data::{Defs, Diagnostics};
use clash_gen::GenDefs;
use clash_scene::{StartDef, UiDefs};
use clash_world::WorldDefs;

/// The tables a game's world is made from: the world's and the
/// generators'.
#[derive(Clone, Debug)]
pub struct WorldTables {
    /// The world's tables.
    pub world: Arc<WorldDefs>,
    /// What generation is built from.
    pub generation: Arc<GenDefs>,
}

/// A profile, loaded.
#[derive(Clone, Debug)]
pub struct Loaded {
    /// Every definition of the profile's mods.
    pub defs: Arc<Defs>,
    /// The interface's definitions.
    pub ui: UiDefs,
    /// The world's tables, or why they could not be built. A profile
    /// with no starts may have none.
    pub tables: Result<WorldTables, String>,
    /// What the load had to say that was not an error.
    pub warnings: String,
    /// The directories mods were searched for in.
    pub search: Vec<PathBuf>,
}

/// Loads a profile: by name, or from a file if `profile` names one.
/// Search directories are tried in order (04 §3.1).
pub fn load(search: &[PathBuf], profile: &str) -> Result<Loaded, String> {
    let types = clash_scene::engine_types();
    let mut loader = clash_scene::engine_loader(&types);
    for dir in search {
        loader = loader.search_path(dir);
    }
    let as_file = Path::new(profile);
    let outcome = if as_file.is_file() {
        loader.load_profile_file(as_file)
    } else {
        loader.load_profile(profile)
    };
    let render = |diagnostics: &Diagnostics| diagnostics.render();
    let Some(defs) = outcome.defs else {
        return Err(render(&outcome.diagnostics));
    };
    if outcome.diagnostics.has_errors() {
        return Err(render(&outcome.diagnostics));
    }
    let warnings = if outcome.diagnostics.is_empty() {
        String::new()
    } else {
        render(&outcome.diagnostics)
    };
    let ui = UiDefs::from_defs(&defs).map_err(|diagnostics| render(&diagnostics))?;
    // What only shows when themes and screens are made (12 §10.3).
    let checked = ui.check();
    if checked.has_errors() {
        return Err(render(&checked));
    }
    let tables = WorldDefs::from_defs(&defs)
        .and_then(|world| {
            GenDefs::from_defs(&defs, &world).map(|generation| WorldTables { world, generation })
        })
        .map_err(|diagnostics| render(&diagnostics));
    let has_starts = defs
        .try_registry::<StartDef>()
        .is_some_and(|starts| !starts.is_empty());
    if has_starts && let Err(problems) = &tables {
        // A game could be started and its world cannot be made.
        return Err(problems.clone());
    }
    Ok(Loaded {
        defs,
        ui,
        tables,
        warnings,
        search: search.to_vec(),
    })
}

/// The qualified id of the screen a loaded profile boots into, or why
/// there is none.
pub fn boot_screen(loaded: &Loaded) -> Result<String, String> {
    let profile = loaded
        .defs
        .profile()
        .ok_or("the definitions were not loaded from a profile")?;
    let Some((screen, _)) = &profile.boot_screen else {
        return Err(format!(
            "the profile `{}` names no `boot_screen`: there is nothing to show first",
            profile.id
        ));
    };
    Ok(screen.clone())
}
