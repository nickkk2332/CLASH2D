//! What the tests of `clash_gen::map` share: a small world to draw maps
//! from, and the way from JSON text to compiled maps.

#![allow(dead_code, reason = "each test file uses its own part of this")]

use std::sync::Arc;

use clash_data::{DefTypes, Defs, Diagnostic, Diagnostics, Loader};
use clash_expr::Registry;
use clash_gen::map::{MapDefs, MapLimits};
use clash_world::WorldDefs;

/// The id of the mod the tests' definitions live in.
pub const MOD: &str = "t";

/// A world small enough to read at a glance. `mat` is both a floor and
/// a fixture, for the short form's ambiguity. `arrow` has a facing;
/// `lamp` names two directions and lacks the other two, so it has none.
pub const WORLD: &str = r##"[
  { "type": "material", "id": "stone", "density": 2600, "toughness": 1.0e6 },
  { "type": "material", "id": "oak", "density": 700, "toughness": 1.0e5 },
  { "type": "floor", "id": "boards", "material": "oak", "thickness": 0.05,
    "glyph": { "ch": "." } },
  { "type": "floor", "id": "paving", "material": "stone", "thickness": 0.1,
    "glyph": { "ch": "," } },
  { "type": "floor", "id": "mat", "material": "oak", "thickness": 0.01,
    "glyph": { "ch": "_" } },
  { "type": "block", "id": "wall", "material": "stone", "height": 3.0, "fill": 0.2,
    "glyph": { "ch": "#" } },
  { "type": "block", "id": "door", "material": "oak", "height": 3.0, "fill": 0.05,
    "states": { "closed": {}, "open": { "glyph": { "ch": "'" } } },
    "glyph": { "ch": "+" } },
  { "type": "fixture", "id": "table", "material": "oak", "height": 0.8, "fill": 0.1,
    "glyph": { "ch": "T" } },
  { "type": "fixture", "id": "mat", "material": "oak", "height": 0.1, "fill": 0.01,
    "glyph": { "ch": "m" } },
  { "type": "fixture", "id": "arrow", "material": "oak", "height": 0.5, "fill": 0.05,
    "states": { "north": { "glyph": { "ch": "^" } }, "east": { "glyph": { "ch": ">" } },
                "south": { "glyph": { "ch": "v" } }, "west": { "glyph": { "ch": "<" } } },
    "glyph": { "ch": "^" } },
  { "type": "fixture", "id": "lamp", "material": "oak", "height": 0.5, "fill": 0.05,
    "states": { "lit_north": {}, "lit_east": {} },
    "glyph": { "ch": "l" } },
  { "type": "area_kind", "id": "room" }
]"##;

/// The definition types a map needs: the expression language's, the
/// world's and the map format's.
pub fn types() -> DefTypes {
    let mut types = DefTypes::new();
    clash_data::expr::register_defs(&mut types);
    clash_world::register_defs(&mut types);
    clash_gen::register_defs(&mut types);
    types
}

/// The definition types of this crate and of the world, for reading the
/// bundled `core` mod: without `function` and `variable`, because the
/// bundled ones name kinds only the simulation registers. With
/// `Loader::only_known_types` they are passed over.
pub fn bundled_types() -> DefTypes {
    let mut types = DefTypes::new();
    clash_world::register_defs(&mut types);
    clash_gen::register_defs(&mut types);
    types
}

/// Loads the test world with some maps, given as the text of a JSON
/// array. The file the maps are in is `data/maps.json`.
pub fn load(maps: &str, registry: Option<&Registry>) -> (Option<Arc<Defs>>, Diagnostics) {
    let types = types();
    let mut loader = Loader::new(&types).memory_mod(
        MOD,
        r#"{ "id": "t" }"#,
        &[("data/world.json", WORLD), ("data/maps.json", maps)],
    );
    if let Some(registry) = registry {
        loader = loader.expr_registry(registry);
    }
    let outcome = loader.load_all();
    (outcome.defs, outcome.diagnostics)
}

/// The loaded definitions and the world's tables for some maps that
/// must load.
pub fn loaded(maps: &str) -> (Arc<Defs>, Arc<WorldDefs>) {
    let (defs, diagnostics) = load(maps, None);
    let defs = defs.unwrap_or_else(|| panic!("the maps do not load:\n{}", diagnostics.render()));
    let world = WorldDefs::from_defs(&defs).unwrap_or_else(|diagnostics| {
        panic!("the world does not build:\n{}", diagnostics.render())
    });
    (defs, world)
}

/// The compiled maps of some maps that must build.
pub fn build(maps: &str) -> Arc<MapDefs> {
    let (defs, world) = loaded(maps);
    MapDefs::from_defs(&defs, &world)
        .unwrap_or_else(|diagnostics| panic!("the maps do not build:\n{}", diagnostics.render()))
}

/// What building some maps that load reports, with limits given.
pub fn build_errors_with(maps: &str, limits: MapLimits) -> Diagnostics {
    let (defs, world) = loaded(maps);
    match MapDefs::from_defs_with(&defs, &world, limits) {
        Ok(_) => panic!("the maps build, and were meant not to"),
        Err(diagnostics) => diagnostics,
    }
}

/// What building some maps that load reports.
pub fn build_errors(maps: &str) -> Diagnostics {
    build_errors_with(maps, MapLimits::default())
}

/// The one diagnostic whose message holds `needle`. It must say where
/// it is: a file, a line and a column.
pub fn the_one<'a>(diagnostics: &'a Diagnostics, needle: &str) -> &'a Diagnostic {
    let found: Vec<&Diagnostic> = diagnostics
        .iter()
        .filter(|diagnostic| diagnostic.message.contains(needle))
        .collect();
    assert_eq!(
        found.len(),
        1,
        "expected one diagnostic saying {needle:?}, among:\n{}",
        diagnostics.render()
    );
    let diagnostic = found[0];
    let location = diagnostic
        .location
        .as_ref()
        .unwrap_or_else(|| panic!("no location on:\n{}", diagnostic.render()));
    assert!(location.file.ends_with("maps.json"), "{}", location.file);
    assert!(location.line >= 1 && location.column >= 1);
    diagnostic
}

/// The line, from one, of the first place `needle` is written in a
/// text.
pub fn line_of(text: &str, needle: &str) -> u32 {
    let at = text
        .find(needle)
        .unwrap_or_else(|| panic!("{needle:?} is not in the text"));
    text[..at].matches('\n').count() as u32 + 1
}

/// The column, from one, of the first place `needle` is written in a
/// text.
pub fn column_of(text: &str, needle: &str) -> u32 {
    let at = text
        .find(needle)
        .unwrap_or_else(|| panic!("{needle:?} is not in the text"));
    let line_start = text[..at].rfind('\n').map_or(0, |newline| newline + 1);
    text[line_start..at].chars().count() as u32 + 1
}
