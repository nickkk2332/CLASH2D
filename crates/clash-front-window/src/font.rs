//! Where a theme's font is: [`theme_font`].
//!
//! A theme names its font as a file inside a mod (12 §4.3):
//!
//! ```json
//! "font": { "file": "assets/font.ttf", "size": 16 }
//! ```
//!
//! A file reference inside a mod resolves inside that mod's directory
//! (04 §15). The mod meant is the one whose file wrote the theme's
//! `font`: the theme's own mod, or a mod that patched the theme with a
//! font of its own.

use std::path::{Path, PathBuf};

use clash_data::{Def, Defs, resolve_in_mod};
use clash_scene::{FontDef, ThemeDef};

/// A theme's font: the file and the height of a cell in pixels.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct FontSource {
    /// The font file.
    pub path: PathBuf,
    /// The height of a cell, in pixels at the screen's ordinary scale.
    pub size: u16,
}

/// The font of the theme with this qualified id: `None` if the theme
/// names none, or there is no such theme. A file reference that leaves
/// its mod's directory is an error, given as a sentence; whether the
/// file exists is for whoever opens it.
///
/// ```no_run
/// # fn defs() -> std::sync::Arc<clash_data::Defs> { unimplemented!() }
/// let font = clash_front_window::theme_font(&defs(), "ui:default").unwrap().unwrap();
/// assert!(font.path.ends_with("assets/font.ttf"));
/// ```
pub fn theme_font(defs: &Defs, theme_id: &str) -> Result<Option<FontSource>, String> {
    let Some(theme) = defs
        .try_registry::<ThemeDef>()
        .and_then(|themes| themes.lookup(theme_id).map(|id| themes.get(id)))
    else {
        return Ok(None);
    };
    let Some(FontDef { file, size }) = &theme.font else {
        return Ok(None);
    };
    let written_in = defs
        .tree(ThemeDef::TYPE, theme_id)
        .and_then(|tree| tree.tree.get("font"))
        .and_then(|font| font.get("file"))
        .map(|node| defs.sources().path(node.span.file).to_owned());
    // The mod whose directory holds the file that wrote it: the longest
    // root that is a directory above it.
    let root = written_in
        .as_deref()
        .and_then(|source| {
            defs.mods()
                .iter()
                .filter(|loaded| Path::new(source).starts_with(&loaded.root))
                .max_by_key(|loaded| loaded.root.len())
        })
        .or_else(|| {
            let namespace = theme_id.split_once(':').map_or("", |(space, _)| space);
            defs.mods()
                .iter()
                .find(|loaded| loaded.manifest.id == namespace)
        })
        .ok_or_else(|| {
            format!("the theme `{theme_id}` names a font, and no loaded mod's directory holds it")
        })?;
    let path = resolve_in_mod(Path::new(&root.root), file)
        .map_err(|reason| format!("the font of the theme `{theme_id}`: {reason}"))?;
    Ok(Some(FontSource { path, size: *size }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use clash_data::Loader;

    fn mods_dir() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../mods")
    }

    #[test]
    fn the_bundled_themes_name_the_bundled_font() {
        let types = clash_scene::engine_types();
        let defs = Loader::new(&types)
            .search_path(mods_dir())
            .load_mod_with_requirements("ui")
            .into_result()
            .unwrap_or_else(|e| panic!("{}", e.render()));
        for theme in ["ui:default", "ui:night"] {
            let font = theme_font(&defs, theme).unwrap().unwrap();
            assert!(
                font.path.ends_with("mods/ui/assets/font.ttf"),
                "{}",
                font.path.display()
            );
            assert!(font.path.is_file());
            assert_eq!(font.size, 16);
        }
        assert_eq!(theme_font(&defs, "ui:nothing").unwrap(), None);
    }
}
