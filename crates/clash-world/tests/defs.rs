//! Tests for W3: the definition types, the content of `mods/core/world`,
//! the derived per-state tables and the state reference parser.
//!
//! The engine names no material or block. These tests and the JSON they
//! read are the only places that do.

mod common;

use std::sync::Arc;

use clash_core::Real;
use clash_world::defs::{BlockDef, FixtureDef, MaterialDef, NONE_ID, RawWorldDefs, Resist};
use clash_world::{
    BlockId, BlockState, CellFlags, EngineFlag, FixtureState, FloorState, Layer, MaterialId,
    StateRefError, WorldDefs,
};

fn mpa(value: f64) -> Real {
    Real::of(value * 1.0e6)
}

fn kpa(value: f64) -> Real {
    Real::of(value * 1.0e3)
}

fn build_err(raw: RawWorldDefs) -> Vec<String> {
    WorldDefs::build(raw)
        .expect_err("the definitions should not build")
        .iter()
        .map(ToString::to_string)
        .collect()
}

#[test]
fn the_core_content_loads_and_holds_what_the_plan_lists() {
    let defs = common::test_defs();
    for id in [
        "impact", "cut", "pierce", "blast", "heat", "cold", "shock", "corrode",
    ] {
        assert!(
            defs.damage_types().lookup(&format!("core:{id}")).is_some(),
            "damage type {id}"
        );
    }
    for id in [
        "granite", "soil", "concrete", "brick", "glass", "wood", "steel",
    ] {
        assert!(
            defs.materials().lookup(&format!("core:{id}")).is_some(),
            "material {id}"
        );
    }
    for id in [
        "solid",
        "wall",
        "window",
        "door",
        "pillar",
        "ramp",
        "rubble_heap",
    ] {
        assert!(
            defs.blocks().lookup(&format!("core:{id}")).is_some(),
            "block {id}"
        );
    }
    for id in ["slab", "asphalt", "grass"] {
        assert!(
            defs.floors().lookup(&format!("core:{id}")).is_some(),
            "floor {id}"
        );
    }
    for id in ["rubble", "desk"] {
        assert!(
            defs.fixtures().lookup(&format!("core:{id}")).is_some(),
            "fixture {id}"
        );
    }
    for id in ["air", "thin_air"] {
        assert!(
            defs.atmospheres().lookup(&format!("core:{id}")).is_some(),
            "atmosphere {id}"
        );
    }
    assert_eq!(defs.damage_types().len(), 8);
    // Seven materials and the built-in none.
    assert_eq!(defs.materials().len(), 8);
    assert_eq!(
        defs.kinetic_damage_type(),
        defs.damage_types().lookup("core:impact")
    );
}

#[test]
fn a_concrete_wall_matches_the_worked_example() {
    // 02 §12.2: a wall cell holds 0.6 m³ of concrete, so its capacity is
    // 0.6 MJ.
    let defs = common::test_defs();
    let wall = defs.block_state("wall", Some("core")).unwrap();
    let concrete = defs.materials().lookup("core:concrete").unwrap();
    assert_eq!(wall.material(), concrete);
    let raw = wall.raw();
    assert_eq!(
        defs.volume(Layer::Block, raw),
        Real::of(0.2) * Real::of(3.0)
    );
    assert!((defs.volume(Layer::Block, raw) - Real::of(0.6)).abs() < Real::of(1.0e-12));
    assert!((defs.capacity(Layer::Block, raw) - Real::of(0.6e6)).abs() < Real::of(1.0e-6));
    assert!((defs.mass(Layer::Block, raw) - Real::of(1440.0)).abs() < Real::of(1.0e-9));

    // Resolution order: the element's override, then the material, then
    // the damage type's default.
    let kind = |id: &str| defs.damage_types().lookup(&format!("core:{id}")).unwrap();
    let one = Real::ONE;
    assert_eq!(
        defs.resist(Layer::Block, raw, kind("blast")),
        Resist {
            threshold: kpa(70.0),
            factor: one
        }
    );
    assert_eq!(
        defs.resist(Layer::Block, raw, kind("impact")),
        Resist {
            threshold: mpa(20.0),
            factor: one
        }
    );
    assert_eq!(
        defs.resist(Layer::Block, raw, kind("cut")),
        Resist {
            threshold: mpa(200.0),
            factor: Real::of(0.2)
        }
    );
    assert_eq!(
        defs.resist(Layer::Block, raw, kind("pierce")),
        Resist {
            threshold: mpa(1.0),
            factor: one
        }
    );
    // Solid rock of the same material keeps the material's threshold.
    let rock = defs.block_state("solid@concrete", Some("core")).unwrap();
    assert_eq!(
        defs.resist(Layer::Block, rock.raw(), kind("blast"))
            .threshold,
        mpa(100.0)
    );

    let info = defs.block_info(wall);
    assert!(info.flags.has(EngineFlag::Opaque));
    assert!(info.flags.has(EngineFlag::LoadBearing));
    assert_eq!(info.height, Real::of(3.0));
    assert_eq!(info.coverage, Real::ONE);
    assert_eq!(info.cell, CellFlags::OPAQUE | CellFlags::WALK_BLOCKED);
}

#[test]
fn any_shape_can_be_made_of_any_material() {
    let defs = common::test_defs();
    let brick = defs.block_state("wall@brick", Some("core")).unwrap();
    let steel = defs.block_state("core:wall@core:steel", None).unwrap();
    assert_eq!(brick.def(), steel.def());
    assert_ne!(brick.material(), steel.material());
    let ratio = defs.mass(Layer::Block, steel.raw()) / defs.mass(Layer::Block, brick.raw());
    assert!((ratio - Real::of(7850.0 / 1900.0)).abs() < Real::of(1.0e-12));
    assert_eq!(
        defs.state_text(Layer::Block, brick.raw()),
        "core:wall@core:brick"
    );
}

#[test]
fn named_states_change_flags_and_geometry() {
    let defs = common::test_defs();
    let closed = defs.block_state("door", Some("core")).unwrap();
    let open = defs.block_state("door#open", Some("core")).unwrap();
    assert_eq!(closed.bits(), 0);
    assert_eq!(open.bits(), 1);
    assert_eq!(defs.block_state("door#closed", None).unwrap(), closed);
    assert_eq!(
        defs.block_state("door@steel#open", None)
            .unwrap()
            .material(),
        { defs.materials().lookup("core:steel").unwrap() }
    );

    let closed_info = defs.block_info(closed);
    assert!(closed_info.flags.has(EngineFlag::Opaque));
    assert!(!closed_info.flags.has(EngineFlag::Passable));
    assert_eq!(closed_info.coverage, Real::ONE);
    assert_eq!(
        closed_info.cell,
        CellFlags::OPAQUE | CellFlags::WALK_BLOCKED
    );

    let open_info = defs.block_info(open);
    assert!(!open_info.flags.has(EngineFlag::Opaque));
    assert!(open_info.flags.has(EngineFlag::Passable));
    assert_eq!(open_info.coverage, Real::ZERO);
    assert_eq!(open_info.cell, CellFlags::NONE);

    let door = defs.blocks().get(closed.def()).unwrap();
    assert_eq!(door.state_names, ["closed", "open"]);
    assert_eq!(door.interactions, ["open", "close"]);
    assert_eq!(door.glyphs[0].ch.as_deref(), Some("+"));
    assert_eq!(door.glyphs[1].ch.as_deref(), Some("'"));
    assert_eq!(door.glyphs[1].fg.as_deref(), Some("material"));
    // Bits past the last state mean the first.
    assert_eq!(defs.block_info(closed.with_bits(7)), closed_info);
    assert_eq!(
        defs.state_text(Layer::Block, open.raw()),
        "core:door@core:wood#open"
    );
}

#[test]
fn derived_cell_flags_follow_the_elements() {
    let defs = common::test_defs();
    let solid = defs.block_state("solid", None).unwrap();
    assert_eq!(
        defs.block_info(solid).cell,
        CellFlags::OPAQUE | CellFlags::WALK_BLOCKED | CellFlags::SOLID_TOP
    );
    let window = defs.block_state("window", None).unwrap();
    assert_eq!(
        defs.block_info(window).cell,
        CellFlags::WALK_BLOCKED | CellFlags::FRAGILE
    );
    let ramp = defs.block_state("ramp", None).unwrap();
    assert_eq!(defs.block_info(ramp).cell, CellFlags::NONE);

    let slab = defs.floor_state("slab", None).unwrap();
    assert_eq!(defs.floor_info(slab).cell, CellFlags::HAS_FLOOR);
    assert_eq!(defs.floor_info(slab).volume, Real::of(0.2));
    assert_eq!(defs.floor_info(slab).thickness, Real::of(0.2));
    let asphalt = defs.floor_state("asphalt", None).unwrap();
    assert_eq!(defs.floor_info(asphalt).thickness, Real::of(0.1));

    // Rubble is low enough to walk over; a desk is not.
    let rubble = defs.fixture_state("rubble", None).unwrap();
    assert_eq!(defs.fixture_info(rubble).cell, CellFlags::NONE);
    assert!(defs.fixture_info(rubble).flags.has(EngineFlag::Rough));
    let desk = defs.fixture_state("desk", None).unwrap();
    assert_eq!(defs.fixture_info(desk).cell, CellFlags::WALK_BLOCKED);
    assert!((defs.fixture_info(desk).coverage - Real::of(0.8 / 3.0)).abs() < Real::of(1e-12));

    assert_eq!(
        defs.cell_flags(slab, window, desk),
        CellFlags::HAS_FLOOR | CellFlags::WALK_BLOCKED | CellFlags::FRAGILE
    );
    assert_eq!(
        defs.cell_flags(FloorState::NONE, BlockState::NONE, FixtureState::NONE),
        CellFlags::NONE
    );
}

#[test]
fn references_between_definitions_resolve() {
    let defs = common::test_defs();
    let rubble = defs.fixtures().lookup("core:rubble").unwrap();
    let heap = defs.blocks().lookup("core:rubble_heap").unwrap();
    assert_eq!(defs.fixtures().get(rubble).unwrap().heap, Some(heap));
    let concrete = defs.materials().lookup("core:concrete").unwrap();
    let material = defs.materials().get(concrete).unwrap();
    assert_eq!(material.rubble, Some(rubble));
    assert_eq!(material.density, Real::of(2400.0));
    assert_eq!(material.toughness, Real::of(1.0e6));
    assert_eq!(material.span, Some(4));
    assert_eq!(material.bearing, Some(mpa(30.0)));
    assert!(material.tags.contains(&"mineral".to_owned()));
    let steel = defs.materials().lookup("core:steel").unwrap();
    assert_eq!(
        defs.materials().get(steel).unwrap().toughness,
        Real::of(1.0e8)
    );
    let air = defs.atmospheres().lookup("core:air").unwrap();
    assert!(defs.atmospheres().get(air).unwrap().breathable);
    assert_eq!(
        defs.atmospheres().get(air).unwrap().pressure,
        Real::of(101_325.0)
    );
}

#[test]
fn id_zero_is_the_built_in_none() {
    let defs = common::test_defs();
    assert_eq!(defs.blocks().lookup(NONE_ID), Some(BlockId::new(0)));
    assert_eq!(defs.materials().lookup(NONE_ID), Some(MaterialId::new(0)));
    assert_eq!(defs.block_state("none", None).unwrap(), BlockState::NONE);
    assert_eq!(
        defs.block_state("engine:none@steel#4", None).unwrap(),
        BlockState::NONE
    );
    assert_eq!(
        defs.volume(Layer::Block, BlockState::NONE.raw()),
        Real::ZERO
    );
    assert_eq!(
        defs.state_text(Layer::Floor, FloorState::NONE.raw()),
        NONE_ID
    );
    // A stale id reads as nothing instead of panicking.
    let stale = BlockState::new(BlockId::new(60_000), MaterialId::new(60_000), 9);
    assert_eq!(defs.block_info(stale).cell, CellFlags::NONE);
    assert_eq!(defs.mass(Layer::Block, stale.raw()), Real::ZERO);
    let impact = defs.kinetic_damage_type().unwrap();
    assert_eq!(
        defs.resist(Layer::Block, stale.raw(), impact).threshold,
        kpa(100.0)
    );
}

#[test]
fn ids_are_dense_sorted_and_independent_of_input_order() {
    let forward = WorldDefs::build(common::test_raw()).unwrap();
    let mut shuffled = common::test_raw();
    shuffled.materials.reverse();
    shuffled.blocks.reverse();
    shuffled.damage_types.rotate_left(3);
    shuffled.fixtures.reverse();
    shuffled.floors.rotate_left(1);
    let backward = WorldDefs::build(shuffled).unwrap();

    let names = |defs: &WorldDefs| -> Vec<String> {
        defs.blocks()
            .iter()
            .map(|(id, _)| defs.blocks().name(id).to_owned())
            .collect()
    };
    assert_eq!(names(&forward), names(&backward));
    let listed = names(&forward);
    assert_eq!(listed[0], NONE_ID);
    let mut sorted = listed[1..].to_vec();
    sorted.sort();
    assert_eq!(listed[1..], sorted);
    for id in ["core:concrete", "core:steel", "core:wood"] {
        assert_eq!(
            forward.materials().lookup(id),
            backward.materials().lookup(id)
        );
    }
    for id in ["core:blast", "core:impact"] {
        assert_eq!(
            forward.damage_types().lookup(id),
            backward.damage_types().lookup(id)
        );
    }
    assert_eq!(
        forward.block_state("wall@brick", None).unwrap(),
        backward.block_state("wall@brick", None).unwrap()
    );
}

#[test]
fn state_references_report_what_is_missing() {
    let defs = common::test_defs();
    assert!(matches!(
        defs.block_state("wal", None),
        Err(StateRefError::UnknownDef { layer: "block", .. })
    ));
    assert!(matches!(
        defs.block_state("wall@brik", None),
        Err(StateRefError::UnknownMaterial(material)) if material == "brik"
    ));
    assert!(matches!(
        defs.block_state("door#ajar", None),
        Err(StateRefError::UnknownState { .. })
    ));
    assert!(matches!(
        defs.block_state("wall@", None),
        Err(StateRefError::Malformed(_))
    ));
    // A floor is not a block.
    assert!(defs.block_state("slab", None).is_err());
    assert!(defs.floor_state("slab", None).is_ok());
    let message = defs.block_state("door#ajar", None).unwrap_err().to_string();
    assert_eq!(message, "block `core:door` has no state `ajar`");
}

#[test]
fn interactions_devices_and_contact_are_stored_and_reported() {
    // The examples of 02 §5.4 and §5.5.
    let mut raw = common::test_raw();
    common::add_json(
        &mut raw,
        "test",
        r#"[
          { "type": "fixture", "id": "locker", "material": "steel",
            "height": 2.0, "fill": 0.1, "flags": ["opaque"],
            "states": { "closed": {}, "open": { "remove_flags": ["opaque"] } },
            "interactions": ["open", "close", "hide_inside", "search"],
            "device": "locker_device",
            "glyph": { "ch": "]", "fg": "material" } },
          { "type": "fixture", "id": "spikes", "material": "steel", "height": 0.5, "fill": 0.05,
            "contact": { "surface": { "kind": "pierce", "area": 3.0e-5, "give": 0.003, "span": 0.02 },
                         "energy": 60 } },
          { "type": "block", "id": "pane", "material": "glass", "height": 3.0, "fill": 0.02,
            "on_destroy": "frame@steel" },
          { "type": "block", "id": "frame", "material": "wood", "height": 3.0, "fill": 0.01,
            "flags": ["passable"] }
        ]"#,
        "inline",
    );
    let defs = WorldDefs::build(raw).unwrap();
    let locker = defs.fixture_state("locker", None).unwrap();
    let element = defs.fixtures().get(locker.def()).unwrap();
    assert_eq!(
        element.interactions,
        ["open", "close", "hide_inside", "search"]
    );
    assert_eq!(element.device.as_deref(), Some("locker_device"));
    // Bare names resolve in the definition's own namespace first, then
    // elsewhere: `steel` is the core mod's.
    assert_eq!(
        locker.material(),
        defs.materials().lookup("core:steel").unwrap()
    );

    let spikes = defs.fixture_state("test:spikes", None).unwrap();
    let contact = defs.fixtures().get(spikes.def()).unwrap().contact.unwrap();
    assert_eq!(
        contact.kind,
        defs.damage_types().lookup("core:pierce").unwrap()
    );
    assert_eq!(contact.area, Real::of(3.0e-5));
    assert_eq!(contact.give, Real::of(0.003));
    assert_eq!(contact.span, Some(Real::of(0.02)));
    assert_eq!(contact.energy, Real::of(60.0));

    let pane = defs.block_state("pane", None).unwrap();
    let becomes = defs.blocks().get(pane.def()).unwrap().on_destroy.unwrap();
    assert_eq!(
        BlockState::from_raw(becomes),
        defs.block_state("frame@steel", None).unwrap()
    );
}

#[test]
fn every_mistake_is_reported_and_none_panics() {
    let mut raw = common::test_raw();
    common::add_json(
        &mut raw,
        "bad",
        r#"[
          { "type": "material", "id": "mud", "density": -5, "toughness": 1.0e4,
            "resist": { "blats": { "threshold": "1 kPa" } }, "rubble": "gravel" },
          { "type": "block", "id": "blob", "material": "jelly", "height": 4.0, "fill": 1.5,
            "resist": { "impact": { "threshold": "-1 Pa" } }, "on_destroy": "blob#burst" },
          { "type": "fixture", "id": "pile", "material": "soil", "height": 1.0, "fill": 0.5,
            "heap": "mountain", "coverage": 2.0 },
          { "type": "floor", "id": "film", "material": "soil", "thickness": "-1 mm" },
          { "type": "damage_type", "id": "shove", "kinetic": true },
          { "type": "material", "id": "core:concrete", "density": 1, "toughness": 1 },
          { "type": "material", "id": "engine:void", "density": 1, "toughness": 1 }
        ]"#,
        "inline",
    );
    raw.materials
        .push(("Not An Id".to_owned(), raw.materials[0].1.clone()));
    let errors = build_err(raw);
    let has = |needle: &str| errors.iter().any(|error| error.contains(needle));
    assert!(
        has("material `bad:mud`, density: must be zero or more"),
        "{errors:#?}"
    );
    assert!(
        has("material `bad:mud`, resist.blats: there is no damage type `blats`"),
        "{errors:#?}"
    );
    assert!(
        has("material `bad:mud`, rubble: there is no fixture `gravel`"),
        "{errors:#?}"
    );
    assert!(
        has("block `bad:blob`, material: there is no material `jelly`"),
        "{errors:#?}"
    );
    assert!(
        has("block `bad:blob`, height: must be from 0 to 3"),
        "{errors:#?}"
    );
    assert!(
        has("block `bad:blob`, fill: must be from 0 to 1"),
        "{errors:#?}"
    );
    assert!(
        has("block `bad:blob`, resist.impact.threshold: must be zero or more"),
        "{errors:#?}"
    );
    assert!(
        has("block `bad:blob`, on_destroy: block `bad:blob` has no state `burst`"),
        "{errors:#?}"
    );
    assert!(
        has("fixture `bad:pile`, heap: there is no block `mountain`"),
        "{errors:#?}"
    );
    assert!(
        has("fixture `bad:pile`, coverage: must be from 0 to 1"),
        "{errors:#?}"
    );
    assert!(
        has("floor `bad:film`, thickness: must be from 0 to 3"),
        "{errors:#?}"
    );
    assert!(
        has("damage_type `core:impact`, kinetic: only one damage type may be kinetic")
            || has("damage_type `bad:shove`, kinetic: only one damage type may be kinetic"),
        "{errors:#?}"
    );
    assert!(
        has("material `core:concrete`, id: this id is defined more than once"),
        "{errors:#?}"
    );
    assert!(
        has("material `engine:void`, id: the namespace `engine` is reserved"),
        "{errors:#?}"
    );
    assert!(
        has("material `Not An Id`, id: an id is `namespace:name`"),
        "{errors:#?}"
    );
}

#[test]
fn unknown_keys_and_wrong_types_are_refused_when_read() {
    let unknown = serde_json::from_str::<MaterialDef>(
        r#"{ "density": 600, "thougness": 2.0e4, "toughness": 1 }"#,
    );
    assert!(
        unknown
            .unwrap_err()
            .to_string()
            .contains("unknown field `thougness`")
    );
    let missing = serde_json::from_str::<BlockDef>(r#"{ "material": "wood", "height": 1.0 }"#);
    assert!(
        missing
            .unwrap_err()
            .to_string()
            .contains("missing field `fill`")
    );
    let flag = serde_json::from_str::<FixtureDef>(
        r#"{ "material": "wood", "height": 1.0, "fill": 0.1, "flags": ["shiny"] }"#,
    );
    assert!(
        flag.unwrap_err()
            .to_string()
            .contains("unknown variant `shiny`")
    );
    let unit = serde_json::from_str::<MaterialDef>(r#"{ "density": "600 kPa", "toughness": 1 }"#);
    assert!(unit.unwrap_err().to_string().contains("is not a density"));
    let twice = serde_json::from_str::<BlockDef>(
        r#"{ "material": "wood", "height": 1.0, "fill": 0.1,
             "states": { "open": {}, "open": {} } }"#,
    );
    assert!(twice.unwrap_err().to_string().contains("given twice"));
}

#[test]
fn states_keep_the_order_they_were_written_in() {
    // Read straight from text, as the loader will: the first state written
    // is the default, whatever its name.
    let def: BlockDef = serde_json::from_str(
        r#"{ "material": "wood", "height": 3.0, "fill": 0.05,
             "states": { "shut": {}, "ajar": { "coverage": 0.5 }, "open": { "coverage": 0.0 } } }"#,
    )
    .unwrap();
    let mut raw = common::test_raw();
    raw.blocks.push(("test:gate".to_owned(), def));
    let defs = WorldDefs::build(raw).unwrap();
    let gate = defs.block_state("gate", None).unwrap();
    assert_eq!(
        defs.blocks().get(gate.def()).unwrap().state_names,
        ["shut", "ajar", "open"]
    );
    assert_eq!(defs.block_state("gate#ajar", None).unwrap().bits(), 1);
}

#[test]
fn definitions_are_shared_between_threads() {
    let defs: Arc<WorldDefs> = common::test_defs();
    let wall = defs.block_state("wall", None).unwrap();
    let handles: Vec<_> = (0..2)
        .map(|_| {
            let defs = Arc::clone(&defs);
            std::thread::spawn(move || defs.capacity(Layer::Block, wall.raw()))
        })
        .collect();
    for handle in handles {
        assert!(handle.join().unwrap() > Real::ZERO);
    }
}
