//! Joining this crate's definitions to the mod loader (04 §7 to §9).
//!
//! The definition types of this crate (`color`, `theme`, `keymap`,
//! `figure`, `screen`, `ambience` and `overlay`) are registered with
//! `clash-data` by [`register_defs`], and [`UiDefs`] is the one way from
//! loaded definitions to a [`Theme`], a [`Scene`], a [`Screen`], the
//! glyph tables a view of the world is drawn with ([`WorldGlyphs`]), an
//! [`Ambience`] and an [`Overlay`]:
//!
//! ```no_run
//! use clash_data::{DefTypes, Loader};
//! use clash_scene::defs::UiDefs;
//!
//! let mut types = DefTypes::new();
//! clash_scene::register_defs(&mut types);
//! // ... every other crate's `register_defs` ...
//! let defs = Loader::new(&types)
//!     .search_path("mods")
//!     .load_profile("default")
//!     .into_result()
//!     .map_err(|diagnostics| diagnostics.render())?;
//! let ui = UiDefs::from_defs(&defs).map_err(|diagnostics| diagnostics.render())?;
//! let mut scene = ui
//!     .scene("mymod:night", "mymod:keys")
//!     .map_err(|diagnostics| diagnostics.render())?;
//! let screen = ui
//!     .screen("mymod:title", scene.theme())
//!     .map_err(|diagnostics| diagnostics.render())?;
//! scene.open(screen);
//! # Ok::<(), String>(())
//! ```
//!
//! A definition is asked for by its qualified id, `namespace:name`
//! (04 §4.1).
//!
//! # What is checked where
//!
//! The loader reads each definition into its struct: unknown keys, wrong
//! types and keys that are not keys are its diagnostics. What needs more
//! than one definition is checked when a theme or a screen is made: a
//! colour a theme names, a style, a figure or an art asset a screen
//! names, the tint of an ambience, the shade of an overlay, and the
//! colours the world's glyphs and materials name. A mistake found then
//! is a [`Diagnostic`] at the value in the file that wrote it, found
//! through the finished tree the loader keeps for every definition.
//! [`check_defs`] makes every theme, and with every theme every screen,
//! ambience and overlay and the world's glyph tables, for a tool that
//! only wants the mistakes: the linter runs it after a load that had no
//! errors.
//!
//! # Bare names
//!
//! A theme and a screen name colours, figures and art by plain text, not
//! yet by reference fields the loader resolves. Until they do (package
//! U6b), a bare name is read from the namespace of the definition that
//! is being made (decision D-322):
//!
//! - A `color` or `figure` definition is known by its qualified id. Its
//!   bare name means the one in the namespace of the theme or screen
//!   being made, or else the first in order of qualified id.
//! - An art asset is a path within the mod that defines the screen:
//!   `assets/title.txt`. Every file under a mod's `assets/` whose name
//!   ends in `.txt` is read as art when a [`UiDefs`] is made.

use std::collections::BTreeMap;
use std::path::Path;
use std::sync::Arc;

use clash_core::define_id;
use clash_data::{Def, DefLabel, DefTypes, Defs, Diagnostic, Diagnostics, Span, code};
use clash_world::WorldDefs;

use crate::color::ColorDef;
use crate::input::KeymapDef;
use crate::scene::{NoHost, Scene};
use crate::theme::{Theme, ThemeDef, ThemeError};
use crate::ui::{FigureDef, Screen, ScreenDef, UiEnv, UiError, UiProblem};
use crate::view::{Ambience, AmbienceDef, GlyphProblem, Overlay, OverlayDef, WorldGlyphs};

define_id! {
    /// A `color` definition.
    pub struct ColorId(u16);
    /// A `theme` definition.
    pub struct ThemeId(u16);
    /// A `keymap` definition.
    pub struct KeymapId(u16);
    /// A `figure` definition.
    pub struct FigureId(u16);
    /// A `screen` definition.
    pub struct ScreenId(u16);
    /// An `ambience` definition.
    pub struct AmbienceId(u16);
    /// An `overlay` definition.
    pub struct OverlayId(u16);
}

impl Def for ColorDef {
    type Id = ColorId;
    const TYPE: &'static str = "color";
    const DOC: &'static str = "A colour: its `rgb`, the word text uses for it and the terminal colour shown in its place (12 §4.3).";
}

impl Def for ThemeDef {
    type Id = ThemeId;
    const TYPE: &'static str = "theme";
    const DOC: &'static str =
        "Colours and styles by name, border characters, a font and the engine's marks (12 §4.3).";
}

impl Def for KeymapDef {
    type Id = KeymapId;
    const TYPE: &'static str = "keymap";
    const DOC: &'static str = "For each context, the name each key has in it (12 §9.2).";
}

impl Def for FigureDef {
    type Id = FigureId;
    const TYPE: &'static str = "figure";
    const DOC: &'static str =
        "Rows of character art with named regions, drawn by the `figure` widget (12 §10.3).";
}

impl Def for ScreenDef {
    type Id = ScreenId;
    const TYPE: &'static str = "screen";
    const DOC: &'static str =
        "A screen of the interface: its variables, its layout tree and its keys (12 §10.1).";
}

impl Def for AmbienceDef {
    type Id = AmbienceId;
    const TYPE: &'static str = "ambience";
    const DOC: &'static str =
        "How a condition looks: a tint, a dimming and particles that fall or drift (12 §6.4).";
}

impl Def for OverlayDef {
    type Id = OverlayId;
    const TYPE: &'static str = "overlay";
    const DOC: &'static str =
        "Shading a map view draws over its cells where an expression over a cell holds (12 §6.2).";
}

/// Registers every definition type of the engine: this crate's and
/// those of every crate below it (expressions, the world, generation,
/// the simulation). The application, the tools, the examples and any
/// test that loads whole mods call this, so that all of them read mods
/// with the same types. A crate that gains definition types is added
/// here once.
pub fn register_engine_defs(types: &mut DefTypes) {
    clash_data::expr::register_defs(types);
    clash_world::register_defs(types);
    clash_gen::register_defs(types);
    clash_sim::register_defs(types);
    register_defs(types);
}

/// Every definition type of the engine, registered
/// ([`register_engine_defs`]).
pub fn engine_types() -> DefTypes {
    let mut types = DefTypes::new();
    register_engine_defs(&mut types);
    types
}

/// The expression registry of the engine: every kind of subject,
/// property, function and event the simulation offers (05 §5.3). It is
/// what a mod's expressions are checked against.
pub fn engine_registry() -> clash_data::expr::ExprRegistry {
    clash_sim::engine_registry()
}

/// A loader over definition types that checks every expression against
/// the engine's registry ([`engine_registry`]). The application, the
/// tools and the examples start from this, so that all of them read
/// mods with the same types and the same kinds and properties.
pub fn engine_loader(types: &DefTypes) -> clash_data::Loader<'_> {
    clash_data::Loader::new(types).expr_registry(&engine_registry())
}

/// Registers this crate's definition types with the loader: `color`,
/// `theme`, `keymap`, `figure`, `screen`, `ambience` and `overlay`.
///
/// The application and the tools call this, with every other crate's
/// function of the same name, before they load (04 §9).
pub fn register_defs(types: &mut DefTypes) {
    types.register::<ColorDef>();
    types.register::<ThemeDef>();
    types.register::<KeymapDef>();
    types.register::<FigureDef>();
    types.register::<ScreenDef>();
    types.register::<AmbienceDef>();
    types.register::<OverlayDef>();
}

/// The directory of a mod that holds its assets.
const ASSETS_DIR: &str = "assets";
/// The extension of an asset that is art: rows of characters.
const ART_EXTENSION: &str = "txt";

/// The namespace of a qualified id, or `""` if it has none.
fn namespace_of(id: &str) -> &str {
    id.split_once(':').map_or("", |(namespace, _)| namespace)
}

/// The definitions of one type by the names a definition in `namespace`
/// knows them by: every qualified id, and each bare name for the one in
/// `namespace` itself or else the first in order of qualified id.
fn seen_from<T: Def + Clone>(defs: &Defs, namespace: &str) -> BTreeMap<String, T> {
    let mut seen = BTreeMap::new();
    let Some(registry) = defs.try_registry::<T>() else {
        return seen;
    };
    let mut own = Vec::new();
    for (id, def) in registry.iter() {
        let qualified = registry.name(id);
        seen.insert(qualified.to_owned(), def.clone());
        let Some((home, bare)) = qualified.split_once(':') else {
            continue;
        };
        if home == namespace {
            own.push((bare, def));
        } else {
            seen.entry(bare.to_owned()).or_insert_with(|| def.clone());
        }
    }
    for (bare, def) in own {
        seen.insert(bare.to_owned(), def.clone());
    }
    seen
}

/// One step of a path into a definition.
enum Step<'a> {
    Key(&'a str),
    Index(usize),
}

/// The steps of a path such as `layout.cols[1].bar.label`.
fn steps(path: &str) -> Vec<Step<'_>> {
    let mut out = Vec::new();
    for part in path.split('.').filter(|part| !part.is_empty()) {
        let (key, mut rest) = match part.find('[') {
            Some(at) => part.split_at(at),
            None => (part, ""),
        };
        if !key.is_empty() {
            out.push(Step::Key(key));
        }
        while let Some(inner) = rest.strip_prefix('[') {
            let Some((index, after)) = inner.split_once(']') else {
                break;
            };
            match index.parse() {
                Ok(index) => out.push(Step::Index(index)),
                Err(_) => break,
            }
            rest = after;
        }
    }
    out
}

/// Where the value at a path of a definition was written, as far along
/// the path as the finished tree goes.
fn locate(defs: &Defs, type_name: &str, id: &str, path: &str) -> Option<Span> {
    let tree = defs.tree(type_name, id)?;
    let mut node = &tree.tree;
    // The whole definition: point at its opening brace, not at every line.
    let mut span = Span::new(node.span.file, node.span.start, node.span.start + 1);
    for step in steps(path) {
        let next = match step {
            Step::Key(key) => node.get(key),
            Step::Index(index) => node.as_array().and_then(|items| items.get(index)),
        };
        let Some(next) = next else {
            break;
        };
        node = next;
        span = node.span;
    }
    Some(span)
}

/// A mistake in a definition as a diagnostic at the value that is wrong.
fn mistake(defs: &Defs, type_name: &str, id: &str, path: &str, message: String) -> Diagnostic {
    let label = DefLabel::new(type_name, id);
    let mut diagnostic = Diagnostic::new(code::VALIDATION, message)
        .in_def(Some(&label))
        .with_path(path);
    if let Some(span) = locate(defs, type_name, id, path) {
        diagnostic = diagnostic.at(defs.sources(), span);
    }
    diagnostic
}

/// The diagnostic for an id that names no definition of a type.
fn unknown<T: Def>(defs: &Defs, id: &str) -> Diagnostic {
    let mut diagnostic = Diagnostic::new(
        code::UNKNOWN_REFERENCE,
        format!("there is no {} `{id}`", T::TYPE),
    );
    let known = defs.try_registry::<T>().map(|registry| registry.names());
    let near = known.and_then(|names| {
        clash_data::diag::closest(id, names.iter().map(String::as_str)).map(str::to_owned)
    });
    diagnostic = match near {
        Some(near) => diagnostic.suggest(format!("did you mean `{near}`?")),
        None if !id.contains(':') => {
            diagnostic.suggest("an id is written in full: `namespace:name`")
        }
        None => diagnostic,
    };
    diagnostic
}

/// Where in a theme a mistake is.
fn theme_error_path(error: &ThemeError) -> String {
    match error {
        ThemeError::UnknownColor { at, .. } | ThemeError::NotInAGlyph { at, .. } => at.clone(),
        ThemeError::ColorCycle { name } => format!("colors.{name}"),
        ThemeError::WideBorder { .. } => "borders".to_owned(),
        ThemeError::Mark { mark, .. } => format!("marks.{mark}"),
    }
}

/// The definitions of this crate out of a finished load, with the art
/// assets of the loaded mods: what themes, scenes and screens are made
/// from.
#[derive(Debug, Clone)]
pub struct UiDefs {
    defs: Arc<Defs>,
    /// The text of each art asset, by the id of its mod and then by its
    /// path within the mod, such as `assets/title.txt`.
    art: BTreeMap<String, BTreeMap<String, String>>,
}

/// Collects the art files under a directory, in sorted order, as paths
/// from `prefix` with forward slashes.
fn read_art(
    dir: &Path,
    prefix: &str,
    into: &mut BTreeMap<String, String>,
    diagnostics: &mut Diagnostics,
) {
    let unreadable = |path: &Path, error: std::io::Error| {
        Diagnostic::new(
            code::UNREADABLE_FILE,
            format!("cannot read `{}`: {error}", path.display()),
        )
    };
    let entries = match std::fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(error) => {
            diagnostics.push(unreadable(dir, error));
            return;
        }
    };
    let mut paths: Vec<_> = entries
        .filter_map(|entry| entry.ok().map(|entry| entry.path()))
        .collect();
    paths.sort();
    for path in paths {
        let Some(name) = path.file_name().map(|name| name.to_string_lossy()) else {
            continue;
        };
        // As for definition files: a name that starts with a dot is not
        // read (04 §3.1).
        if name.starts_with('.') {
            continue;
        }
        let shown = format!("{prefix}/{name}");
        if path.is_dir() {
            read_art(&path, &shown, into, diagnostics);
        } else if path.extension().and_then(|e| e.to_str()) == Some(ART_EXTENSION) {
            match std::fs::read_to_string(&path) {
                Ok(text) => {
                    into.insert(shown, text);
                }
                Err(error) => diagnostics.push(unreadable(&path, error)),
            }
        }
    }
}

impl UiDefs {
    /// Takes this crate's definitions out of a finished load and reads
    /// the art assets of its mods: every file under a mod's `assets/`
    /// whose name ends in `.txt`.
    ///
    /// An asset that cannot be read is reported. Nothing else is checked
    /// here; a theme or a screen is checked when it is made.
    pub fn from_defs(defs: &Arc<Defs>) -> Result<UiDefs, Diagnostics> {
        let mut diagnostics = Diagnostics::new();
        let mut art = BTreeMap::new();
        for loaded in defs.mods() {
            let assets = Path::new(&loaded.root).join(ASSETS_DIR);
            if !assets.is_dir() {
                continue;
            }
            let mut files = BTreeMap::new();
            read_art(&assets, ASSETS_DIR, &mut files, &mut diagnostics);
            art.insert(loaded.manifest.id.clone(), files);
        }
        if diagnostics.has_errors() {
            return Err(diagnostics);
        }
        Ok(UiDefs {
            defs: Arc::clone(defs),
            art,
        })
    }

    /// The loaded definitions these were taken from.
    pub fn defs(&self) -> &Arc<Defs> {
        &self.defs
    }

    /// Adds an art asset to a mod, as if it had been read from the mod's
    /// directory: for a mod that is not on disk.
    pub fn add_art(&mut self, mod_id: &str, path: &str, text: &str) {
        self.art
            .entry(mod_id.to_owned())
            .or_default()
            .insert(path.to_owned(), text.to_owned());
    }

    /// The qualified ids of the definitions of one of this crate's types,
    /// in id order.
    pub fn ids<T: Def>(&self) -> &[String] {
        self.defs
            .try_registry::<T>()
            .map_or(&[], |registry| registry.names())
    }

    /// Builds the theme with this qualified id against the `color`
    /// definitions.
    pub fn theme(&self, id: &str) -> Result<Theme, Diagnostics> {
        let defs = &*self.defs;
        let mut diagnostics = Diagnostics::new();
        let Some(def) = defs
            .lookup::<ThemeDef>(id)
            .map(|found| defs.get::<ThemeDef>(found))
        else {
            diagnostics.push(unknown::<ThemeDef>(defs, id));
            return Err(diagnostics);
        };
        let colors = seen_from::<ColorDef>(defs, namespace_of(id));
        Theme::build(def, &colors).map_err(|errors| {
            for error in &errors {
                let path = theme_error_path(error);
                diagnostics.push(mistake(defs, ThemeDef::TYPE, id, &path, error.to_string()));
            }
            diagnostics.sort();
            diagnostics
        })
    }

    /// The keymap with this qualified id.
    pub fn keymap(&self, id: &str) -> Result<&KeymapDef, Diagnostics> {
        let defs = &*self.defs;
        match defs.lookup::<KeymapDef>(id) {
            Some(found) => Ok(defs.get::<KeymapDef>(found)),
            None => {
                let mut diagnostics = Diagnostics::new();
                diagnostics.push(unknown::<KeymapDef>(defs, id));
                Err(diagnostics)
            }
        }
    }

    /// A scene with no simulation behind it, using the theme and the
    /// keymap with these qualified ids.
    pub fn scene(&self, theme: &str, keymap: &str) -> Result<Scene<NoHost>, Diagnostics> {
        let built = self.theme(theme);
        let found = self.keymap(keymap);
        match (built, found) {
            (Ok(theme), Ok(keymap)) => Ok(Scene::new(NoHost, theme, keymap.clone())),
            (theme, keymap) => {
                let mut diagnostics = Diagnostics::new();
                if let Err(errors) = theme {
                    diagnostics.extend(errors.iter().cloned());
                }
                if let Err(errors) = keymap {
                    diagnostics.extend(errors.iter().cloned());
                }
                Err(diagnostics)
            }
        }
    }

    /// Makes the screen with this qualified id, styled by `theme`.
    pub fn screen(&self, id: &str, theme: &Theme) -> Result<Screen, Diagnostics> {
        self.make_screen(id, theme, true)
    }

    /// Makes a screen. Without `styled`, what depends on the theme is
    /// not reported: for checking a screen when no theme could be built.
    fn make_screen(&self, id: &str, theme: &Theme, styled: bool) -> Result<Screen, Diagnostics> {
        let defs = &*self.defs;
        let mut diagnostics = Diagnostics::new();
        let Some(def) = defs
            .lookup::<ScreenDef>(id)
            .map(|found| defs.get::<ScreenDef>(found))
        else {
            diagnostics.push(unknown::<ScreenDef>(defs, id));
            return Err(diagnostics);
        };
        let namespace = namespace_of(id);
        let figures = seen_from::<FigureDef>(defs, namespace);
        let no_art = BTreeMap::new();
        let env = UiEnv {
            theme,
            figures: &figures,
            art: self.art.get(namespace).unwrap_or(&no_art),
        };
        Screen::new(id, def, &env).map_err(|errors| {
            for UiError { path, problem, .. } in &errors {
                let of_theme =
                    matches!(problem, UiProblem::UnknownStyle { .. } | UiProblem::Text(_));
                if of_theme && !styled {
                    continue;
                }
                diagnostics.push(mistake(
                    defs,
                    ScreenDef::TYPE,
                    id,
                    path,
                    problem.to_string(),
                ));
            }
            diagnostics.sort();
            diagnostics
        })
    }

    /// The glyph tables a view of the world is drawn with: the look of
    /// every element state and material of `world` under `theme`.
    ///
    /// A glyph or a material that names a colour the theme does not
    /// know, or a glyph whose `ch` is not one character, is an error at
    /// the value that is wrong. [`WorldGlyphs::build`] is the lenient
    /// form, which draws what it can.
    pub fn world_glyphs(
        &self,
        world: &WorldDefs,
        theme: &Theme,
    ) -> Result<WorldGlyphs, Diagnostics> {
        let (glyphs, problems) = WorldGlyphs::build(world, theme);
        if problems.is_empty() {
            return Ok(glyphs);
        }
        let mut diagnostics = Diagnostics::new();
        for problem in &problems {
            diagnostics.push(self.glyph_mistake(problem));
        }
        diagnostics.sort();
        Err(diagnostics)
    }

    /// A problem with the look of a world definition, as a diagnostic at
    /// the value that is wrong: in the state that wrote it, or else in
    /// the definition's own glyph.
    fn glyph_mistake(&self, problem: &GlyphProblem) -> Diagnostic {
        const MATERIAL: &str = "material";
        let defs = &*self.defs;
        let path = if problem.kind == MATERIAL {
            problem.field.to_owned()
        } else {
            let written_by_state = problem.state.as_deref().filter(|state| {
                defs.tree(problem.kind, &problem.id)
                    .and_then(|tree| {
                        tree.tree
                            .get("states")?
                            .get(state)?
                            .get("glyph")?
                            .get(problem.field)
                    })
                    .is_some()
            });
            match written_by_state {
                Some(state) => format!("states.{state}.glyph.{}", problem.field),
                None => format!("glyph.{}", problem.field),
            }
        };
        mistake(
            defs,
            problem.kind,
            &problem.id,
            &path,
            problem.message.clone(),
        )
    }

    /// Makes the ambience with this qualified id, its tint resolved by
    /// `theme`.
    pub fn ambience(&self, id: &str, theme: &Theme) -> Result<Ambience, Diagnostics> {
        let defs = &*self.defs;
        let mut diagnostics = Diagnostics::new();
        let Some(def) = defs
            .lookup::<AmbienceDef>(id)
            .map(|found| defs.get::<AmbienceDef>(found))
        else {
            diagnostics.push(unknown::<AmbienceDef>(defs, id));
            return Err(diagnostics);
        };
        Ambience::compile(def, theme).map_err(|errors| {
            for error in &errors {
                diagnostics.push(mistake(
                    defs,
                    AmbienceDef::TYPE,
                    id,
                    error.path(),
                    error.to_string(),
                ));
            }
            diagnostics.sort();
            diagnostics
        })
    }

    /// Makes the overlay with this qualified id, its shade resolved by
    /// `theme`.
    pub fn overlay(&self, id: &str, theme: &Theme) -> Result<Overlay, Diagnostics> {
        let defs = &*self.defs;
        let mut diagnostics = Diagnostics::new();
        let Some(def) = defs
            .lookup::<OverlayDef>(id)
            .map(|found| defs.get::<OverlayDef>(found))
        else {
            diagnostics.push(unknown::<OverlayDef>(defs, id));
            return Err(diagnostics);
        };
        Overlay::compile(id, def, theme).map_err(|errors| {
            for error in &errors {
                diagnostics.push(mistake(
                    defs,
                    OverlayDef::TYPE,
                    id,
                    error.path(),
                    error.to_string(),
                ));
            }
            diagnostics.sort();
            diagnostics
        })
    }

    /// Makes every theme, and with every theme every screen, ambience
    /// and overlay and the world's glyph tables, and returns what is
    /// wrong with them. With no theme loaded they are made with the
    /// engine's plain one over the loaded colours. When no loaded theme
    /// can be built, screens are checked for everything but their
    /// styles, and what only names colours is not checked.
    ///
    /// Everything must stand with every loaded theme, because the player
    /// may choose any of them: a style or a colour one theme lacks is
    /// reported with that theme named.
    pub fn check(&self) -> Diagnostics {
        let mut diagnostics = Diagnostics::new();
        let mut themes: Vec<(&str, Theme)> = Vec::new();
        for id in self.ids::<ThemeDef>() {
            match self.theme(id) {
                Ok(theme) => themes.push((id, theme)),
                Err(errors) => diagnostics.extend(errors.iter().cloned()),
            }
        }
        let styled = !themes.is_empty() || self.ids::<ThemeDef>().is_empty();
        if themes.is_empty() {
            let colors = seen_from::<ColorDef>(&self.defs, "");
            let plain = Theme::build(&ThemeDef::default(), &colors).unwrap_or_default();
            themes.push(("", plain));
        }
        let several = themes.len() > 1;
        // Makes one thing with every theme and reports each mistake
        // once, naming the theme it showed with where there are several.
        let mut with_each_theme = |make: &dyn Fn(&Theme) -> Option<Diagnostics>| {
            let mut found: Vec<Diagnostic> = Vec::new();
            for (theme_id, theme) in &themes {
                let Some(errors) = make(theme) else {
                    continue;
                };
                for error in errors.iter() {
                    if found.iter().any(|seen| seen == error) {
                        continue;
                    }
                    found.push(error.clone());
                    diagnostics.push(if several {
                        error.clone().note(format!("with the theme `{theme_id}`"))
                    } else {
                        error.clone()
                    });
                }
            }
        };
        for id in self.ids::<ScreenDef>() {
            with_each_theme(&|theme| self.make_screen(id, theme, styled).err());
        }
        if styled {
            for id in self.ids::<AmbienceDef>() {
                with_each_theme(&|theme| self.ambience(id, theme).err());
            }
            for id in self.ids::<OverlayDef>() {
                with_each_theme(&|theme| self.overlay(id, theme).err());
            }
            // The world's own mistakes are the world's to report: its
            // glyphs are checked only when its tables build.
            if let Ok(world) = WorldDefs::from_defs(&self.defs) {
                with_each_theme(&|theme| self.world_glyphs(&world, theme).err());
            }
        }
        diagnostics.sort();
        diagnostics
    }
}

/// Everything wrong with this crate's definitions that only shows when
/// themes and screens are made (see the module documentation). Empty
/// when they can all be made.
pub fn check_defs(defs: &Arc<Defs>) -> Diagnostics {
    match UiDefs::from_defs(defs) {
        Ok(ui) => ui.check(),
        Err(diagnostics) => diagnostics,
    }
}

#[cfg(test)]
mod tests {
    use clash_data::Loader;

    use super::*;

    fn types() -> DefTypes {
        let mut types = DefTypes::new();
        register_defs(&mut types);
        types
    }

    /// Loads mods given as `(id, text of data/ui.json)`, in that order.
    fn load(mods: &[(&str, &str)]) -> Arc<Defs> {
        let types = types();
        let mut loader = Loader::new(&types);
        for (id, text) in mods {
            let manifest = format!(r#"{{ "id": "{id}" }}"#);
            loader = loader.memory_mod(id, &manifest, &[("data/ui.json", text)]);
        }
        let ids: Vec<&str> = mods.iter().map(|(id, _)| *id).collect();
        match loader.load_mods(&ids).into_result() {
            Ok(defs) => defs,
            Err(diagnostics) => panic!("the mods do not load:\n{}", diagnostics.render()),
        }
    }

    const PLAIN: &str = r##"[
  { "type": "color", "id": "crimson", "rgb": "#a01828", "word": "red", "fallback": "red" },
  { "type": "theme", "id": "plain",
    "colors": { "text": "#c8c8c8", "blood": "crimson" },
    "styles": { "title": { "fg": "blood", "bold": true } } },
  { "type": "keymap", "id": "keys", "menu": { "enter": "accept", "q": "back" } },
  { "type": "figure", "id": "dot", "rows": ["o"] },
  { "type": "screen", "id": "hello",
    "layout": { "rows": [
      { "text": "<title>Hello.</>" },
      { "figure": { "figure": "dot" } } ] } }
]"##;

    #[test]
    fn the_types_are_registered_with_their_documentation() {
        let types = types();
        assert_eq!(
            types.names(),
            [
                "ambience", "color", "figure", "keymap", "overlay", "screen", "theme"
            ]
        );
        for info in types.infos() {
            assert!(!info.doc.is_empty(), "`{}` has no documentation", info.name);
        }
    }

    #[test]
    fn a_theme_a_scene_and_a_screen_are_made_from_loaded_definitions() {
        let defs = load(&[("demo", PLAIN)]);
        let ui = UiDefs::from_defs(&defs).unwrap();
        assert_eq!(ui.ids::<ScreenDef>(), ["demo:hello"]);
        let theme = ui.theme("demo:plain").unwrap();
        // A bare colour name finds the definition in the theme's own mod,
        // and so does its qualified id.
        assert_eq!(theme.color("blood"), theme.color("crimson"));
        assert_eq!(theme.color("crimson"), theme.color("demo:crimson"));
        assert_eq!(theme.color_word("crimson"), Some("red"));
        let mut scene = ui.scene("demo:plain", "demo:keys").unwrap();
        let screen = ui.screen("demo:hello", scene.theme()).unwrap();
        scene.open(screen);
        assert!(ui.check().is_empty(), "{}", ui.check().render());
        assert!(check_defs(&defs).is_empty());
    }

    #[test]
    fn an_id_that_names_nothing_is_an_error_with_a_suggestion() {
        let defs = load(&[("demo", PLAIN)]);
        let ui = UiDefs::from_defs(&defs).unwrap();
        let text = ui.theme("demo:plian").unwrap_err().render();
        assert!(text.contains("there is no theme `demo:plian`"), "{text}");
        assert!(text.contains("did you mean `demo:plain`?"), "{text}");
        let text = ui.keymap("keys").unwrap_err().render();
        assert!(text.contains("there is no keymap `keys`"), "{text}");
        let text = ui.screen("hello", &Theme::default()).unwrap_err().render();
        assert!(text.contains("there is no screen `hello`"), "{text}");
        let text = ui.scene("nope:theme", "nope:keys").unwrap_err().render();
        assert!(text.contains("there is no theme `nope:theme`"), "{text}");
        assert!(text.contains("there is no keymap `nope:keys`"), "{text}");
    }

    #[test]
    fn mistakes_are_reported_where_they_are_written() {
        let defs = load(&[(
            "bad",
            r##"[
  { "type": "theme", "id": "plain",
    "colors": { "text": "#c8c8c8", "glow": "octarine" },
    "styles": { "title": { "fg": "nothing" } } },
  { "type": "screen", "id": "hello",
    "layout": { "rows": [
      { "text": "Fine." },
      { "text": "Styled.", "style": "sparkly" },
      { "figure": { "figure": "nobody" } },
      { "art": { "value": "assets/missing.txt" } } ] } }
]"##,
        )]);
        let diagnostics = check_defs(&defs);
        let found: Vec<(u32, &str, &str, &str)> = diagnostics
            .iter()
            .map(|diagnostic| {
                let location = diagnostic.location.as_ref().expect("a place");
                assert_eq!(location.file, "mods/bad/data/ui.json");
                (
                    location.line,
                    diagnostic.definition.as_ref().unwrap().id.as_str(),
                    diagnostic.path.as_str(),
                    diagnostic.message.as_str(),
                )
            })
            .collect();
        assert_eq!(
            found,
            [
                (
                    3,
                    "bad:plain",
                    "colors.glow",
                    "unknown colour `octarine` in `colors.glow`"
                ),
                (
                    4,
                    "bad:plain",
                    "styles.title.fg",
                    "unknown colour `nothing` in `styles.title.fg`"
                ),
                (
                    9,
                    "bad:hello",
                    "layout.rows[2].figure",
                    "unknown figure `nobody`"
                ),
                (
                    10,
                    "bad:hello",
                    "layout.rows[3].art.value",
                    "unknown art asset `assets/missing.txt`"
                ),
            ],
            "{}",
            diagnostics.render()
        );
        let text = diagnostics.render();
        assert!(text.contains("--> mods/bad/data/ui.json:9:"), "{text}");
        assert!(
            text.contains("= in: screen bad:hello, at layout.rows[2].figure"),
            "{text}"
        );
        // The style is not reported while no theme stands: with one that
        // does, it is.
        let ui = UiDefs::from_defs(&defs).unwrap();
        let text = ui
            .screen("bad:hello", &Theme::default())
            .unwrap_err()
            .render();
        assert!(text.contains("unknown style or colour `sparkly`"), "{text}");
        assert!(
            text.contains("= in: screen bad:hello, at layout.rows[1].text.style"),
            "{text}"
        );
    }

    #[test]
    fn what_the_loader_reads_is_reported_by_the_loader() {
        let types = types();
        let outcome = Loader::new(&types)
            .memory_mod(
                "bad",
                r#"{ "id": "bad" }"#,
                &[(
                    "data/ui.json",
                    r##"[
  { "type": "color", "id": "off", "rgb": "#nothex" },
  { "type": "color", "id": "pale", "rgb": "#102030", "fallback": "pink" },
  { "type": "theme", "id": "plain", "colours": {} },
  { "type": "keymap", "id": "keys", "menu": { "retrun": "accept" } },
  { "type": "screen", "id": "s", "layout": { "lst": {} } },
  { "type": "screen", "id": "t", "layout": { "list": { "rows": [ { "lable": "x" } ] } } },
  { "type": "screen", "id": "u" }
]"##,
                )],
            )
            .load_all();
        assert!(outcome.defs.is_none());
        let text = outcome.diagnostics.render();
        for needle in [
            "`#nothex` is not a colour",
            "\"pink\" is not one of the values allowed here",
            "unknown key \"colours\"",
            "did you mean \"colors\"?",
            "retrun",
            "unknown key `lst` in a layout node",
            "lable",
            "= in: screen bad:t, at layout.list.rows[0]",
            "missing field \"layout\"",
        ] {
            assert!(text.contains(needle), "missing `{needle}` in:\n{text}");
        }
        assert!(
            outcome
                .diagnostics
                .iter()
                .all(|diagnostic| diagnostic.location.is_some())
        );
    }

    #[test]
    fn a_bare_name_means_the_definition_in_the_same_mod_first() {
        let defs = load(&[
            (
                "alpha",
                r##"[ { "type": "color", "id": "glow", "rgb": "#111111" },
                     { "type": "color", "id": "dusk", "rgb": "#333333" } ]"##,
            ),
            (
                "beta",
                r##"[ { "type": "color", "id": "glow", "rgb": "#222222" },
                     { "type": "theme", "id": "night", "colors": { "text": "glow", "dim": "dusk" } } ]"##,
            ),
        ]);
        let ui = UiDefs::from_defs(&defs).unwrap();
        let theme = ui.theme("beta:night").unwrap();
        let rgb = |name: &str| theme.color(name).unwrap().rgb.to_string();
        assert_eq!(rgb("text"), "#222222");
        assert_eq!(rgb("glow"), "#222222");
        assert_eq!(rgb("alpha:glow"), "#111111");
        // A name the theme's own mod does not define is found elsewhere.
        assert_eq!(rgb("dim"), "#333333");
    }

    #[test]
    fn a_screen_must_stand_with_every_loaded_theme() {
        let defs = load(&[(
            "demo",
            r##"[
  { "type": "theme", "id": "rich", "styles": { "fancy": { "bold": true } } },
  { "type": "theme", "id": "poor" },
  { "type": "screen", "id": "hello", "layout": { "text": "Hi.", "style": "fancy" } }
]"##,
        )]);
        let diagnostics = check_defs(&defs);
        assert_eq!(diagnostics.len(), 1, "{}", diagnostics.render());
        let text = diagnostics.render();
        assert!(text.contains("unknown style or colour `fancy`"), "{text}");
        assert!(text.contains("with the theme `demo:poor`"), "{text}");
    }

    #[test]
    fn art_is_read_by_mod_and_may_be_added() {
        let defs = load(&[(
            "demo",
            r#"[ { "type": "screen", "id": "title",
                   "layout": { "art": { "value": "assets/title.txt" } } } ]"#,
        )]);
        let mut ui = UiDefs::from_defs(&defs).unwrap();
        let theme = Theme::default();
        let text = ui.screen("demo:title", &theme).unwrap_err().render();
        assert!(
            text.contains("unknown art asset `assets/title.txt`"),
            "{text}"
        );
        ui.add_art("demo", "assets/title.txt", "CLASH\n");
        assert!(ui.screen("demo:title", &theme).is_ok());
    }

    #[test]
    fn paths_are_followed_into_lists() {
        let steps: Vec<String> = steps("layout.cols[1].bar[0][2].label")
            .iter()
            .map(|step| match step {
                Step::Key(key) => (*key).to_owned(),
                Step::Index(index) => format!("#{index}"),
            })
            .collect();
        assert_eq!(steps, ["layout", "cols", "#1", "bar", "#0", "#2", "label"]);
    }
}
