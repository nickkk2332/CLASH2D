//! Furnishing from maps (10 §3.2, §4.1; 06 §9.3; 16 §4.3) and content
//! tables, against the simulation on flat ground: what a loaded tract's
//! requests for items, tables and devices become, how latent contents
//! of an element are opened, spilled, saved and loaded, and what is
//! refused with a reason. The bundled town is tested in
//! `crates/clash-save/tests/furnish.rs`.

mod common;

use std::sync::{Arc, LazyLock};

use clash_core::{EntityId, Real, VarMap};
use clash_data::Defs;
use clash_gen::place::{
    SPAWN_KIND_DEVICE, SPAWN_KIND_ITEM, SPAWN_KIND_TABLE, SPAWN_PARAM_CONFIG, SPAWN_PARAM_COUNT,
    SPAWN_PARAM_KIND, SPAWN_PARAM_MAP, SPAWN_PARAM_SEED,
};
use clash_sim::item::{ContentTableDef, Item, ItemDef, table};
use clash_sim::{Place, Sim, SimEvent, Subject, device, furnish};
use clash_world::{Cause, CellPos, Layer, Location, SpawnRequest};

const FIXTURES: &str = r#"[
  { "type": "item", "id": "t_bun", "name": "bun", "material": "wood",
    "mass": "0.1 kg", "length": "0.1 m", "stack": true },
  { "type": "item", "id": "t_spoon", "name": "spoon", "material": "steel",
    "mass": "0.05 kg", "length": "0.15 m" },
  { "type": "content_table", "id": "t_each", "each": true,
    "entries": [ { "item": "t_bun", "count": [2, 4] },
                 { "item": "t_spoon", "count": [1, 2], "chance": 0.5 },
                 { "table": "t_pick" } ] },
  { "type": "content_table", "id": "t_pick",
    "entries": [ { "item": "t_bun", "weight": 2 }, { "item": "t_spoon" },
                 { "none": true, "weight": 2 } ] },
  { "type": "content_table", "id": "t_nothing", "entries": [ { "none": true } ] },
  { "type": "device", "id": "t_box", "holds": { "volume": 0.5 } },
  { "type": "fixture", "id": "t_box", "material": "wood", "height": 1.0, "fill": 0.3,
    "states": { "closed": {}, "open": {} }, "device": "t_box", "glyph": { "ch": "B" } },
  { "type": "device", "id": "t_lamp",
    "config": [ { "id": "brightness", "kind": "number", "default": 1, "min": 0, "max": 2 } ] },
  { "type": "fixture", "id": "t_lamp", "material": "steel", "height": 2.0, "fill": 0.02,
    "device": "t_lamp", "glyph": { "ch": "!" } }
]"#;

static DEFS: LazyLock<Arc<Defs>> = LazyLock::new(|| {
    common::try_defs_with(&[("data/test/furnish.json", FIXTURES)])
        .unwrap_or_else(|error| panic!("{error}"))
});

fn sim() -> Sim {
    common::sim_over(Arc::clone(&DEFS), 3)
}

fn table_id(name: &str) -> clash_sim::item::ContentTableId {
    DEFS.lookup::<ContentTableDef>(&format!("core:{name}"))
        .unwrap()
}

fn request(what: &str, kind: &str, cell: CellPos, count: i64, seed: i64) -> SpawnRequest {
    let mut params = VarMap::new();
    params.set(SPAWN_PARAM_KIND, kind);
    params.set(SPAWN_PARAM_MAP, "core:t_map");
    params.set(SPAWN_PARAM_COUNT, count);
    params.set(SPAWN_PARAM_SEED, seed);
    SpawnRequest {
        what: what.to_owned(),
        at: cell,
        params,
    }
}

fn put_fixture(sim: &mut Sim, at: Location, written: &str) {
    let world = Arc::clone(sim.truth().cosmos().defs());
    let state = world.fixture_state(written, None).unwrap().raw();
    sim.world(|ctx| {
        ctx.edit(Cause::DEBUG)
            .set_element(at, Layer::Fixture, state)
    })
    .unwrap();
    sim.pump();
}

/// The items directly in a place, by definition and count, in id order.
fn items(sim: &Sim, wanted: impl Fn(Place) -> bool) -> Vec<(String, u32)> {
    let registry = DEFS.try_registry::<ItemDef>().unwrap();
    sim.components()
        .store::<Item>()
        .iter()
        .filter(|(id, _)| sim.truth().place(*id).is_some_and(&wanted))
        .map(|(_, item)| (registry.name(item.def().unwrap()).to_owned(), item.count()))
        .collect()
}

fn lying(sim: &Sim, at: Location) -> Vec<(String, u32)> {
    items(
        sim,
        |place| matches!(place, Place::At { loc, .. } if loc == at),
    )
}

fn inside(sim: &Sim, holder: EntityId) -> Vec<(String, u32)> {
    items(sim, |place| place == Place::In(holder))
}

fn named<'a>(events: &'a [SimEvent], name: &str) -> Vec<&'a SimEvent> {
    events.iter().filter(|event| event.name() == name).collect()
}

/// `element.latent`, as rules read it.
fn latent_property(sim: &Sim, at: Location) -> clash_core::Value {
    let registry = sim.exprs().registry();
    let kind = registry.kind_id("element").unwrap();
    let property = registry.property_id(kind, "latent").unwrap();
    let handle = sim.subject_value(Subject::Element(at, Layer::Fixture));
    let clash_expr::Value::Subject(handle) = handle else {
        panic!("an element is a subject");
    };
    sim.property(property, handle, &[]).to_stored()
}

// ---- Content tables -----------------------------------------------------------------

#[test]
fn a_table_gives_the_same_things_for_the_same_seed_and_others_for_others() {
    let each = table_id("t_each");
    let once = table::draw(&DEFS, each, 3, 41);
    assert_eq!(once, table::draw(&DEFS, each, 3, 41));
    // Each draw of `each` gives two to four buns, and maybe spoons and
    // what one pick of the nested table gives.
    let buns: u32 = once
        .iter()
        .filter(|drawn| DEFS.registry::<ItemDef>().name(drawn.item) == "core:t_bun")
        .map(|drawn| drawn.count)
        .sum();
    assert!((6..=15).contains(&buns), "{buns}");
    let differ =
        (0..20).any(|seed| table::draw(&DEFS, each, 1, seed) != table::draw(&DEFS, each, 1, 41));
    assert!(differ);
    // A pick gives one thing or nothing, by weight; nothing at all from
    // a table of nothing.
    let pick = table_id("t_pick");
    let mut none = 0;
    for seed in 0..200 {
        let drawn = table::draw(&DEFS, pick, 1, seed);
        assert!(drawn.len() <= 1);
        if drawn.is_empty() {
            none += 1;
        }
    }
    assert!((50..=110).contains(&none), "{none} of 200 empty");
    assert!(table::draw(&DEFS, table_id("t_nothing"), 5, 9).is_empty());
}

#[test]
fn a_table_that_nests_itself_or_names_two_things_fails_to_load() {
    let looped = r#"[
      { "type": "content_table", "id": "a", "entries": [ { "table": "b" } ] },
      { "type": "content_table", "id": "b", "entries": [ { "table": "a" }, { "none": true } ] } ]"#;
    let error = common::try_defs_with(&[
        ("data/test/furnish.json", FIXTURES),
        ("data/test/loop.json", looped),
    ])
    .unwrap_err();
    assert!(error.contains("tables may not nest themselves"), "{error}");
    assert!(error.contains("entries[0].table"), "{error}");
    let two = r#"[ { "type": "content_table", "id": "c",
                    "entries": [ { "item": "t_bun", "none": true, "weight": 0 } ] } ]"#;
    let error = common::try_defs_with(&[
        ("data/test/furnish.json", FIXTURES),
        ("data/test/two.json", two),
    ])
    .unwrap_err();
    assert!(
        error.contains("exactly one of `item`, `table` and `none`"),
        "{error}"
    );
    assert!(error.contains("`weight` must be above zero"), "{error}");
}

// ---- Requests --------------------------------------------------------------------

#[test]
fn items_lie_where_a_map_puts_them_and_in_a_container_wait_to_be_opened() {
    let mut sim = sim();
    let floor = common::at(5, 5);
    let chest = common::at(8, 5);
    put_fixture(&mut sim, chest, "t_box");
    assert!(furnish::honour(
        &mut sim,
        floor,
        &request("t_bun", SPAWN_KIND_ITEM, floor.cell, 3, 7)
    ));
    assert!(furnish::honour(
        &mut sim,
        floor,
        &request("t_spoon", SPAWN_KIND_ITEM, floor.cell, 2, 8)
    ));
    assert!(furnish::honour(
        &mut sim,
        chest,
        &request("t_each", SPAWN_KIND_TABLE, chest.cell, 2, 9)
    ));
    sim.pump();
    // A bun stacks and is one entity of three; spoons do not, and are
    // two.
    assert_eq!(
        lying(&sim, floor),
        vec![
            ("core:t_bun".to_owned(), 3),
            ("core:t_spoon".to_owned(), 1),
            ("core:t_spoon".to_owned(), 1)
        ]
    );
    // The chest is still a plain element: what is in it is latent.
    assert!(device::device_at(&sim, chest, Layer::Fixture).is_none());
    assert_eq!(latent_property(&sim, chest), clash_core::Value::Bool(true));
    assert!(lying(&sim, chest).is_empty());
    // A request that is no furnishing is left alone.
    assert!(!furnish::honour(
        &mut sim,
        floor,
        &request("person", "creature", floor.cell, 1, 1)
    ));

    // Opening it makes what the table gives for its seed, inside it.
    let open = sim
        .truth()
        .cosmos()
        .defs()
        .fixture_state("t_box#open", None)
        .unwrap()
        .raw();
    sim.world(|ctx| {
        ctx.edit(Cause::DEBUG)
            .set_element(chest, Layer::Fixture, open)
    })
    .unwrap();
    sim.pump();
    let events = sim.drain_events();
    let made = named(&events, "contents_materialised");
    assert_eq!(made.len(), 1);
    let box_device = device::device_at(&sim, chest, Layer::Fixture).unwrap();
    let held = inside(&sim, box_device);
    let drawn = table::draw(&DEFS, table_id("t_each"), 2, 9);
    let expected: u32 = drawn.iter().map(|drawn| drawn.count).sum();
    assert_eq!(held.iter().map(|(_, count)| count).sum::<u32>(), expected);
    assert!(!furnish::is_latent(&sim, chest, Layer::Fixture));
}

#[test]
fn latent_contents_are_the_same_through_a_save_and_whoever_opens_them() {
    let chest = common::at(8, 5);
    let fill = |sim: &mut Sim| {
        put_fixture(sim, chest, "t_box");
        furnish::honour(
            sim,
            chest,
            &request("t_each", SPAWN_KIND_TABLE, chest.cell, 3, 11),
        );
        furnish::honour(
            sim,
            chest,
            &request("t_bun", SPAWN_KIND_ITEM, chest.cell, 2, 12),
        );
        sim.pump();
    };
    let mut first = sim();
    fill(&mut first);
    let (mut loaded, unplaced) = common::save_and_load(&first);
    assert!(unplaced.is_empty());
    assert_eq!(loaded.hashes().kernel, first.hashes().kernel);
    assert!(furnish::is_latent(&loaded, chest, Layer::Fixture));
    let opened = furnish::open_element(&mut first, chest, Layer::Fixture, Cause::DEBUG).unwrap();
    let reloaded = furnish::open_element(&mut loaded, chest, Layer::Fixture, Cause::DEBUG).unwrap();
    assert_eq!(opened.len(), reloaded.len());
    let one = device::device_at(&first, chest, Layer::Fixture).unwrap();
    let two = device::device_at(&loaded, chest, Layer::Fixture).unwrap();
    assert_eq!(inside(&first, one), inside(&loaded, two));
    // Opened again, nothing more is made.
    assert!(
        furnish::open_element(&mut first, chest, Layer::Fixture, Cause::DEBUG)
            .unwrap()
            .is_empty()
    );
}

#[test]
fn a_packet_opens_a_latent_container_and_destroying_it_spills_what_it_held() {
    let mut sim = sim();
    let chest = common::at(8, 5);
    put_fixture(&mut sim, chest, "t_box");
    furnish::honour(
        &mut sim,
        chest,
        &request("t_bun", SPAWN_KIND_ITEM, chest.cell, 4, 13),
    );
    sim.world(|ctx| ctx.edit(Cause::DEBUG).set_damage(chest, Layer::Fixture, 30))
        .unwrap();
    sim.pump();
    let box_device = device::device_at(&sim, chest, Layer::Fixture).expect("opened by the blow");
    assert_eq!(inside(&sim, box_device), vec![("core:t_bun".to_owned(), 4)]);

    // Another, destroyed before anyone opened it: what it held is left
    // in the cell.
    let other = common::at(12, 5);
    put_fixture(&mut sim, other, "t_box");
    furnish::honour(
        &mut sim,
        other,
        &request("t_spoon", SPAWN_KIND_ITEM, other.cell, 1, 14),
    );
    let impact = sim
        .truth()
        .cosmos()
        .defs()
        .damage_types()
        .lookup("core:impact")
        .unwrap();
    sim.world(|ctx| {
        ctx.edit(Cause::DEBUG)
            .destroy(other, Layer::Fixture, impact)
    })
    .unwrap();
    sim.pump();
    assert!(!furnish::is_latent(&sim, other, Layer::Fixture));
    let rest = sim.truth().cosmos().resting_cell(other).unwrap();
    assert_eq!(lying(&sim, rest), vec![("core:t_spoon".to_owned(), 1)]);
}

#[test]
fn a_device_a_map_configures_is_promoted_with_its_settings() {
    let mut sim = sim();
    let lamp = common::at(6, 6);
    put_fixture(&mut sim, lamp, "t_lamp");
    assert!(device::device_at(&sim, lamp, Layer::Fixture).is_none());
    let mut wanted = request("hall_lamp", SPAWN_KIND_DEVICE, lamp.cell, 0, 0);
    let config = [(
        "brightness".to_owned(),
        clash_core::Value::Real(Real::of(1.5)),
    )]
    .into_iter()
    .collect();
    wanted
        .params
        .set(SPAWN_PARAM_CONFIG, clash_core::Value::Map(config));
    assert!(furnish::honour(&mut sim, lamp, &wanted));
    let promoted = device::device_at(&sim, lamp, Layer::Fixture).expect("promoted");
    assert_eq!(
        device::config_value(&sim, promoted, "brightness"),
        Some(clash_core::Value::Real(Real::of(1.5)))
    );
}

#[test]
fn what_cannot_be_furnished_is_a_fault_with_its_reason_the_map_and_the_cell() {
    let mut sim = sim();
    let floor = common::at(5, 5);
    furnish::honour(
        &mut sim,
        floor,
        &request("no_such_thing", SPAWN_KIND_ITEM, floor.cell, 1, 1),
    );
    furnish::honour(
        &mut sim,
        floor,
        &request("no_such_table", SPAWN_KIND_TABLE, floor.cell, 1, 1),
    );
    let lamp = common::at(6, 6);
    put_fixture(&mut sim, lamp, "t_lamp");
    let mut wrong = request("", SPAWN_KIND_DEVICE, lamp.cell, 0, 0);
    let config = [("colour".to_owned(), clash_core::Value::Int(1))]
        .into_iter()
        .collect();
    wrong
        .params
        .set(SPAWN_PARAM_CONFIG, clash_core::Value::Map(config));
    furnish::honour(&mut sim, lamp, &wrong);
    sim.pump();
    let events = sim.drain_events();
    let faults: Vec<String> = named(&events, "fault")
        .iter()
        .map(|event| format!("{event:?}"))
        .collect();
    assert_eq!(faults.len(), 3, "{faults:#?}");
    assert!(faults[0].contains("item.unknown_def") && faults[0].contains("core:t_map"));
    assert!(faults[1].contains("content_table"));
    assert!(faults[2].contains("device.unknown_config"));
    for fault in &faults {
        assert!(fault.contains("\"x\""), "{fault}");
    }
    assert!(lying(&sim, floor).is_empty());
}
