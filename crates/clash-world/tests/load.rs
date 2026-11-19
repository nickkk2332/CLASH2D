//! Tests for the join between the mod loader and the world's
//! definitions: the types are registered, the dense ids agree, and a
//! mistake that only shows when the tables are built is reported at the
//! value that is wrong, with file, line and JSON path.
//!
//! The engine names no material or block. These tests and the JSON they
//! read are the only places that do.

mod common;

use std::sync::Arc;

use clash_core::Real;
use clash_data::{Defs, Diagnostics, Severity};
use clash_time::Timeline;
use clash_world::defs::{
    AtmosphereDef, BlockDef, DamageTypeDef, FixtureDef, FloorDef, MaterialDef, NONE_ID, RealmDef,
};
use clash_world::test_support::load_flat;
use clash_world::{CellPos, Cosmos, Location, TractPos, WorldDefs};

/// Loads `core` and one more mod that requires it.
fn load_with(id: &str, files: &[(&str, &str)]) -> Arc<Defs> {
    let types = common::world_types();
    let manifest = format!(r#"{{ "id": "{id}", "requires": ["core"] }}"#);
    match common::loader(&types)
        .memory_mod(id, &manifest, files)
        .load_mods(&["core", id])
        .into_result()
    {
        Ok(defs) => defs,
        Err(diagnostics) => panic!("the mods do not load:\n{}", diagnostics.render()),
    }
}

fn build_err(defs: &Defs) -> Diagnostics {
    WorldDefs::from_defs(defs).expect_err("the tables should not build")
}

#[test]
fn every_world_type_is_registered() {
    let types = common::world_types();
    assert_eq!(
        types.names(),
        [
            "area_kind",
            "atmosphere",
            "block",
            "damage_type",
            "field",
            "field_reaction",
            "fixture",
            "floor",
            "material",
            "plane",
            "realm",
            "sky"
        ]
    );
    for info in types.infos() {
        assert!(!info.doc.is_empty(), "`{}` has no documentation", info.name);
    }
}

#[test]
fn the_loader_and_the_tables_give_the_same_dense_ids() {
    let defs = common::core_defs();
    let world = WorldDefs::from_defs(&defs).unwrap();

    // The built-in "nothing here" is dense id 0 on both sides.
    assert_eq!(defs.lookup::<MaterialDef>(NONE_ID).unwrap().index(), 0);
    assert_eq!(defs.lookup::<FloorDef>(NONE_ID).unwrap().index(), 0);
    assert_eq!(defs.lookup::<BlockDef>(NONE_ID).unwrap().index(), 0);
    assert_eq!(defs.lookup::<FixtureDef>(NONE_ID).unwrap().index(), 0);
    assert_eq!(world.blocks().lookup(NONE_ID).unwrap().index(), 0);

    assert_eq!(
        defs.registry::<DamageTypeDef>().names(),
        world
            .damage_types()
            .iter()
            .map(|(id, _)| world.damage_types().name(id))
            .collect::<Vec<_>>()
    );
    for name in defs.registry::<MaterialDef>().names() {
        assert_eq!(
            defs.lookup::<MaterialDef>(name),
            world.materials().lookup(name)
        );
    }
    for name in defs.registry::<BlockDef>().names() {
        assert_eq!(defs.lookup::<BlockDef>(name), world.blocks().lookup(name));
    }
    for name in defs.registry::<FloorDef>().names() {
        assert_eq!(defs.lookup::<FloorDef>(name), world.floors().lookup(name));
    }
    for name in defs.registry::<FixtureDef>().names() {
        assert_eq!(
            defs.lookup::<FixtureDef>(name),
            world.fixtures().lookup(name)
        );
    }
    for name in defs.registry::<AtmosphereDef>().names() {
        assert_eq!(
            defs.lookup::<AtmosphereDef>(name),
            world.atmospheres().lookup(name)
        );
    }
    for name in defs.registry::<RealmDef>().names() {
        assert_eq!(defs.lookup::<RealmDef>(name), world.realms().lookup(name));
    }
    assert_eq!(
        defs.registry::<BlockDef>().len(),
        world.blocks().len(),
        "the tables hold what the loader holds"
    );

    // A state resolved against the tables carries the loader's ids.
    let wall = world.block_state("core:wall@core:brick", None).unwrap();
    assert_eq!(Some(wall.def()), defs.lookup::<BlockDef>("core:wall"));
    assert_eq!(
        Some(wall.material()),
        defs.lookup::<MaterialDef>("core:brick")
    );
}

#[test]
fn a_cell_is_read_from_a_realm_made_from_a_loaded_definition() {
    let defs = common::core_defs();
    let world = WorldDefs::from_defs(&defs).unwrap();
    let mut timeline = Timeline::new();
    let mut cosmos = Cosmos::new(Arc::clone(&world));
    let realm_def = defs.lookup::<RealmDef>("core:testland").unwrap();
    let realm = cosmos.create_realm(&mut timeline, realm_def, 7).unwrap();
    let grid = cosmos.realm(realm).unwrap().base_grid;

    let rock = world.block_state("core:solid", None).unwrap();
    let grass = world.floor_state("core:grass", None).unwrap();
    load_flat(&mut cosmos, grid, [TractPos::new(0, 0)], rock, grass, 0).unwrap();

    let below = cosmos
        .cell(Location::new(grid, CellPos::new(3, 4, -1)))
        .unwrap();
    assert_eq!(below.block, rock);
    let surface = cosmos
        .cell(Location::new(grid, CellPos::new(3, 4, 0)))
        .unwrap();
    assert_eq!(surface.floor, grass);
    assert!(surface.block.is_none());
}

#[test]
fn a_block_of_a_material_that_does_not_exist_is_reported_where_it_is_written() {
    let defs = load_with(
        "bad",
        &[(
            "data/blocks.json",
            r#"[
  { "type": "block", "id": "gate",
    "height": 3.0, "fill": 0.1,
    "material": "unobtainium" }
]"#,
        )],
    );
    let diagnostics = build_err(&defs);
    assert_eq!(diagnostics.len(), 1, "{}", diagnostics.render());
    let diagnostic = &diagnostics.as_slice()[0];
    assert_eq!(diagnostic.severity, Severity::Error);
    assert_eq!(diagnostic.code, "E0501");
    assert_eq!(diagnostic.message, "there is no material `unobtainium`");
    let location = diagnostic.location.as_ref().unwrap();
    assert_eq!(location.file, "mods/bad/data/blocks.json");
    // The line and column of the reference, not of the definition.
    assert_eq!((location.line, location.column), (4, 17));
    assert_eq!(diagnostic.path, "material");
    let definition = diagnostic.definition.as_ref().unwrap();
    assert_eq!(
        (definition.type_name.as_str(), definition.id.as_str()),
        ("block", "bad:gate")
    );
    let text = diagnostics.render();
    assert!(
        text.contains("--> mods/bad/data/blocks.json:4:17"),
        "{text}"
    );
    assert!(text.contains("= in: block bad:gate, at material"), "{text}");
    // The same through the function a tool calls for the mistakes alone.
    assert_eq!(clash_world::check_defs(&defs).len(), 1);
}

#[test]
fn every_mistake_found_while_building_has_a_place_and_a_path() {
    let defs = load_with(
        "bad",
        &[(
            "data/world.json",
            r#"[
  { "type": "material", "id": "mud", "density": 1500, "toughness": 1.0e4,
    "resist": { "blats": { "threshold": "1 kPa" } } },
  { "type": "block", "id": "blob", "material": "mud",
    "height": 4.0, "fill": 0.5,
    "on_destroy": "blob#burst" },
  { "type": "realm", "id": "nowhere", "size_regions": [1, 1], "z_range": [0, 8],
    "strata": [ { "from_z": 0, "gravity": 9.81 },
                { "from_z": 4, "gravity": 9.81, "atmosphere": "ether" } ] }
]"#,
        )],
    );
    let diagnostics = build_err(&defs);
    let found: Vec<(u32, &str, &str)> = diagnostics
        .iter()
        .map(|diagnostic| {
            let location = diagnostic.location.as_ref().expect("a place");
            assert_eq!(location.file, "mods/bad/data/world.json");
            (
                location.line,
                diagnostic.path.as_str(),
                diagnostic.message.as_str(),
            )
        })
        .collect();
    assert_eq!(
        found,
        [
            (3, "resist.blats", "there is no damage type `blats`"),
            (5, "height", "`height` must be from 0 to 3, not 4"),
            (6, "on_destroy", "block `bad:blob` has no state `burst`"),
            (9, "strata[1].atmosphere", "there is no atmosphere `ether`"),
        ],
        "{}",
        diagnostics.render()
    );
    // The unknown damage type is the key, so that is what is pointed at.
    let key = diagnostics.as_slice()[0].location.as_ref().unwrap();
    assert_eq!(key.column, 17);
    assert_eq!(key.end_column - key.column, "\"blats\"".len() as u32);
}

#[test]
fn a_mistake_written_by_a_patch_is_reported_in_the_file_of_the_patch() {
    let defs = load_with(
        "tweak",
        &[(
            "data/patch.json",
            r#"[ { "type": "block", "id": "core:wall", "patch": true, "fill": 1.5 } ]"#,
        )],
    );
    let diagnostics = build_err(&defs);
    assert_eq!(diagnostics.len(), 1, "{}", diagnostics.render());
    let diagnostic = &diagnostics.as_slice()[0];
    assert_eq!(diagnostic.message, "`fill` must be from 0 to 1, not 1.5");
    assert_eq!(
        diagnostic.location.as_ref().unwrap().file,
        "mods/tweak/data/patch.json"
    );
    assert_eq!(diagnostic.definition.as_ref().unwrap().id, "core:wall");
}

#[test]
fn inheritance_and_patches_reach_the_tables() {
    let defs = load_with(
        "more",
        &[(
            "data/world.json",
            r#"[
  { "type": "material", "id": "base_stone", "abstract": true,
    "density": 2600, "toughness": 1.5e6, "tags": ["mineral"] },
  { "type": "material", "id": "basalt", "copy_from": "base_stone",
    "toughness": 2.0e6, "extend": { "tags": ["igneous"] } },
  { "type": "material", "id": "core:wood", "patch": true, "scale": { "density": 2 } },
  { "type": "block", "id": "column", "material": "basalt", "height": 3.0, "fill": 0.3 }
]"#,
        )],
    );
    let world = WorldDefs::from_defs(&defs).unwrap();
    let materials = world.materials();
    // An abstract definition is inherited from and never built.
    assert!(materials.lookup("more:base_stone").is_none());
    let basalt = materials
        .get(materials.lookup("more:basalt").unwrap())
        .unwrap();
    assert_eq!(basalt.density, Real::of(2600.0));
    assert_eq!(basalt.toughness, Real::of(2.0e6));
    assert_eq!(basalt.tags, ["igneous", "mineral"]);
    let wood = materials
        .get(materials.lookup("core:wood").unwrap())
        .unwrap();
    assert_eq!(wood.density, Real::of(1200.0));
    // A bare reference finds the mod's own material.
    let column = world.block_state("more:column", None).unwrap();
    assert_eq!(Some(column.material()), materials.lookup("more:basalt"));
}

#[test]
fn what_the_loader_reads_is_reported_by_the_loader() {
    let types = common::world_types();
    let outcome = common::loader(&types)
        .memory_mod(
            "bad",
            &common::manifest("bad"),
            &[(
                "data/world.json",
                r#"[
  { "type": "material", "id": "ash", "density": "600 kPa", "thougness": 2.0e4, "toughness": 1 },
  { "type": "block", "id": "drift", "material": "ash", "height": "1 m" },
  { "type": "block", "id": "glint", "material": "ash", "height": 1.0, "fill": 0.1,
    "flags": ["shiny"] },
  { "type": "realm", "id": "nowhere", "size_regions": [1, 1], "z_range": [0, 8],
    "strata": [ { "from_z": 0, "gravity": 9.81 } ],
    "edges": { "top": { "exit": "chart", "chrat": "sol" } } },
  { "type": "realm", "id": "elsewhere", "size_regions": [1, 1], "z_range": [0, 8],
    "strata": [ { "from_z": 0, "gravity": "9.81 m" } ],
    "edges": { "north": "open" } }
]"#,
            )],
        )
        .load_mods(&["core", "bad"]);
    assert!(outcome.defs.is_none());
    let text = outcome.diagnostics.render();
    for needle in [
        "unknown key \"thougness\"",
        "did you mean \"toughness\"?",
        "is not a density",
        "missing field \"fill\"",
        "\"shiny\" is not one of the values allowed here",
        "unknown key \"chrat\"",
        "\"open\" is not one of the values allowed here",
        "= in: realm bad:nowhere, at edges.top",
        "expected a plain number, found the quantity \"9.81 m\"",
        "= in: realm bad:elsewhere, at edges.north",
    ] {
        assert!(text.contains(needle), "missing `{needle}` in:\n{text}");
    }
    // Every one of them has a place.
    assert!(
        outcome
            .diagnostics
            .iter()
            .all(|diagnostic| diagnostic.location.is_some())
    );
}

#[test]
fn the_result_does_not_depend_on_the_order_files_are_given_in() {
    let files = common::core_files();
    let load = |files: &[(String, String)]| {
        let types = common::world_types();
        let files: Vec<(&str, &str)> = files
            .iter()
            .map(|(path, text)| (path.as_str(), text.as_str()))
            .collect();
        clash_data::Loader::new(&types)
            .memory_mod("core", &common::manifest("core"), &files)
            .load_mods(&["core"])
            .into_result()
            .unwrap_or_else(|diagnostics| panic!("{}", diagnostics.render()))
    };
    let forward = load(&files);
    let mut reversed = files.clone();
    reversed.reverse();
    let backward = load(&reversed);
    assert_eq!(forward.fingerprint(), backward.fingerprint());
    assert_eq!(
        forward.registry::<BlockDef>().names(),
        backward.registry::<BlockDef>().names()
    );
    let (forward, backward) = (
        WorldDefs::from_defs(&forward).unwrap(),
        WorldDefs::from_defs(&backward).unwrap(),
    );
    for text in ["wall", "door#open", "solid@brick"] {
        assert_eq!(
            forward.block_state(text, None),
            backward.block_state(text, None)
        );
    }
}

#[test]
fn no_mods_at_all_build_empty_tables() {
    let types = common::world_types();
    let defs = clash_data::Loader::new(&types)
        .load_mods(&[])
        .into_result()
        .unwrap();
    let world = WorldDefs::from_defs(&defs).unwrap();
    // Only the built-in "nothing here".
    assert_eq!(world.blocks().len(), 1);
    assert_eq!(world.materials().len(), 1);
    assert!(world.realms().is_empty());
    assert!(clash_world::check_defs(&defs).is_empty());
}
