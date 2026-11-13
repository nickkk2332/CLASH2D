//! Making (16 §4, §8, §9.5; packages G2, G6, G17 and G19) against the
//! real simulation in the test city, with the bundled content of
//! `mods/core`: stations with modes, settings and stores; recipes worked
//! by hand and at a station, by a character's action and by a machine
//! alone, at a rate their completion follows exactly; know-how from
//! templates, teaching, books and patterns; substances drunk, eaten,
//! rubbed on and put on a blade; a bandage made and applied; save and
//! load in the middle of work; and replay.

mod common;

use std::sync::{Arc, LazyLock};

use clash_core::{EntityId, Real, Tick};
use clash_data::Defs;
use clash_expr::Value;
use clash_sim::action::{ActionDef, How, Params};
use clash_sim::device::{self, Device};
use clash_sim::interaction::Target;
use clash_sim::recipe::{self, KnowHowDef, RecipeDef};
use clash_sim::stats::{PoolDef, PoolId, Stats};
use clash_sim::substance::{self, PourTo, SubstanceDef};
use clash_sim::template::{CharacterSpec, Selection};
use clash_sim::{
    Advance, AdvanceLimit, Command, Control, DebugCommand, Place, Sim, SimEvent, Subject,
};
use clash_time::Timeline;
use clash_world::{Cause, CellPos, Cosmos, GridId, Layer, Location, WorldDefs};

/// What these tests add: a lamp whose element becomes a device when it
/// is switched off, with settings; two pieces of know-how, one needing
/// the other; a press that casts from a pattern loaded into it; a kiln
/// that fires on fuel; and an antidote.
const FIXTURES: &str = r#"[
  { "type": "device", "id": "test_lamp_device", "name": "lamp",
    "config": [ { "id": "brightness", "kind": "number", "default": 1, "min": 0, "max": 2 },
                { "id": "shade", "kind": "choice", "options": ["white", "amber"] } ] },
  { "type": "fixture", "id": "test_lamp", "material": "steel",
    "height": 2.5, "fill": 0.02,
    "states": { "on": {}, "off": {} },
    "device": "test_lamp_device",
    "glyph": { "ch": "!", "fg": "yellow" } },

  { "type": "know_how", "id": "test_casting", "name": "Casting" },
  { "type": "know_how", "id": "test_engraving", "name": "Engraving", "needs": ["test_casting"] },
  { "type": "item", "id": "test_pattern", "name": "pattern",
    "material": "steel", "mass": "0.1 kg", "length": "0.1 m", "grip": { "arm": 1 },
    "provides": ["test_casting"] },
  { "type": "item", "id": "test_ingot", "name": "ingot",
    "material": "steel", "mass": "1 kg", "length": "0.1 m", "grip": { "arm": 1 }, "stack": true },

  { "type": "device", "id": "test_press", "tags": ["test_press"],
    "modes": ["off", "on"], "speed": { "value": 1, "when": ["on"] }, "auto": { "when": ["on"] },
    "holds": { "volume": 0.1 } },
  { "type": "fixture", "id": "test_press", "material": "steel",
    "height": 1.0, "fill": 0.5, "device": "test_press", "glyph": { "ch": "P" } },
  { "type": "recipe", "id": "test_cast", "at": ["test_press"],
    "inputs": [ { "item": "test_ingot" } ], "know_how": ["test_casting"],
    "work": "10 s", "outputs": [ { "item": "knife" } ], "auto": true },

  { "type": "device", "id": "test_kiln", "tags": ["test_kiln"],
    "modes": ["off", "on"], "speed": { "value": 1, "when": ["on"] }, "auto": { "when": ["on"] },
    "stores": { "fuel": { "capacity": 10 } } },
  { "type": "fixture", "id": "test_kiln", "material": "brick",
    "height": 1.0, "fill": 0.5, "device": "test_kiln", "glyph": { "ch": "K" } },
  { "type": "recipe", "id": "test_fire", "at": ["test_kiln"],
    "while": { "fuel": 0.5 }, "work": "10 s", "auto": true },

  { "type": "substance", "id": "test_antidote", "name": "antidote",
    "in_body": { "routes": { "ingest": 1.0 }, "adds": [ { "pool": "toxin", "per_ml": -2.0 } ] } },
  { "type": "item", "id": "test_cup", "name": "cup",
    "material": "glass", "mass": "0.2 kg", "length": "0.1 m", "grip": { "arm": 1 },
    "contains": "300 ml", "filled": { "test_antidote": "50 ml" } }
]"#;

/// The definition files of a directory under `mods/core/data`, as
/// written.
fn core_dir(sub: &str) -> Vec<(String, String)> {
    let dir = common::mods_dir().join("core/data").join(sub);
    let mut files: Vec<std::path::PathBuf> = std::fs::read_dir(&dir)
        .unwrap()
        .filter_map(|entry| entry.ok().map(|entry| entry.path()))
        .filter(|path| path.extension().is_some_and(|ext| ext == "json"))
        .collect();
    files.sort();
    files
        .iter()
        .map(|file| {
            let name = file.file_name().unwrap().to_string_lossy();
            (
                format!("data/{sub}/{name}"),
                std::fs::read_to_string(file).unwrap(),
            )
        })
        .collect()
}

/// A file of `mods/core/data`, as written.
fn core_file(name: &str) -> (String, String) {
    let path = common::mods_dir().join("core/data").join(name);
    (
        format!("data/{name}"),
        std::fs::read_to_string(path).unwrap(),
    )
}

/// `items.json` without its colour, which is the interface's type. It
/// holds no action, so its keys may be reordered.
fn items() -> String {
    let path = common::mods_dir().join("core/data/items.json");
    let text = std::fs::read_to_string(path).unwrap();
    let all: Vec<serde_json::Value> = serde_json::from_str(&text).unwrap();
    let kept: Vec<serde_json::Value> = all
        .into_iter()
        .filter(|def| def.get("type").and_then(serde_json::Value::as_str) != Some("color"))
        .collect();
    serde_json::to_string_pretty(&kept).unwrap()
}

/// `mods/core`'s world, life, movement, items, combat, interactions,
/// making and substances, and the fixtures, loaded once.
static DEFS: LazyLock<Arc<Defs>> = LazyLock::new(|| {
    let mut owned = core_dir("life");
    owned.extend(core_dir("combat"));
    owned.extend(core_dir("perception"));
    owned.extend(core_dir("making"));
    owned.extend(core_dir("substances"));
    owned.push(core_file("movement.json"));
    owned.push(core_file("interactions.json"));
    owned.push(core_file("interact.json"));
    owned.push(core_file("traits.json"));
    owned.push(("data/items.json".to_owned(), items()));
    owned.push(("data/test/making.json".to_owned(), FIXTURES.to_owned()));
    let files: Vec<(&str, &str)> = owned
        .iter()
        .map(|(path, text)| (path.as_str(), text.as_str()))
        .collect();
    common::try_defs_with(&files).unwrap_or_else(|error| panic!("{error}"))
});

// ---- the test city --------------------------------------------------------

const GRID: GridId = GridId::new(0);
const CITY_SEED: u64 = 1;
const SECOND: i64 = 1_000_000;

fn cell(x: i32, y: i32) -> Location {
    Location::new(GRID, CellPos::new(x, y, 0))
}

fn debug(sim: &mut Sim, command: DebugCommand) {
    sim.submit(Command::Debug(command))
        .unwrap_or_else(|error| panic!("{error}"));
}

fn person(name: &str) -> CharacterSpec {
    CharacterSpec::of(["core:person"]).with("core:person.name", Selection::Text(name.to_owned()))
}

fn spawn(sim: &mut Sim, name: &str, place: Place) -> EntityId {
    let id = sim.truth().entities().next_id().unwrap();
    debug(
        sim,
        DebugCommand::Spawn {
            spec: person(name),
            place,
        },
    );
    debug(
        sim,
        DebugCommand::SetControl {
            entity: id,
            control: Control::Delegated,
        },
    );
    id
}

/// A simulation over the test city with `ada` in the street at (17, 1)
/// and `bram` east of her.
fn city(seed: u64) -> (Sim, EntityId, EntityId) {
    let defs = Arc::clone(&DEFS);
    let world = WorldDefs::from_defs(&defs).unwrap_or_else(|d| panic!("{}", d.render()));
    let mut timeline = Timeline::new();
    let mut cosmos = Cosmos::new(Arc::clone(&world));
    let def = world.realms().lookup("core:test_city").unwrap();
    cosmos.create_realm(&mut timeline, def, CITY_SEED).unwrap();
    let mut sim = Sim::from_parts(defs, cosmos, timeline, seed);
    let ada = spawn(&mut sim, "Ada", Place::Stored);
    debug(
        &mut sim,
        DebugCommand::StartSession {
            entity: ada,
            at: cell(17, 1),
        },
    );
    let bram = spawn(&mut sim, "Bram", Place::at(cell(18, 1)));
    sim.pump();
    (sim, ada, bram)
}

/// Puts a fixture in a cell, as data writes it.
fn put_fixture(sim: &mut Sim, at: Location, written: &str) {
    let world = Arc::clone(sim.truth().cosmos().defs());
    let state = world
        .fixture_state(written, None)
        .unwrap_or_else(|error| panic!("{written}: {error}"))
        .raw();
    sim.world(|ctx| {
        ctx.edit(Cause::DEBUG)
            .set_element(at, Layer::Fixture, state)
    })
    .unwrap();
    sim.pump();
}

/// Makes items in a place.
fn make(sim: &mut Sim, def: &str, count: u32, place: Place) -> EntityId {
    let id = sim.truth().entities().next_id().unwrap();
    debug(
        sim,
        DebugCommand::CreateItem {
            def: def.to_owned(),
            count,
            place,
        },
    );
    id
}

/// Makes an item in someone's hand.
fn give(sim: &mut Sim, who: EntityId, def: &str, count: u32) -> EntityId {
    let at = sim.truth().location(who).unwrap();
    let item = make(sim, def, count, Place::at(at));
    clash_sim::item::take(sim, who, item, Cause::DEBUG).unwrap()
}

fn recipe_id(name: &str) -> recipe::RecipeId {
    DEFS.lookup::<RecipeDef>(&format!("core:{name}")).unwrap()
}

fn pool(name: &str) -> PoolId {
    DEFS.lookup::<PoolDef>(&format!("core:{name}")).unwrap()
}

fn level(sim: &Sim, who: EntityId, name: &str) -> Real {
    sim.components()
        .get::<Stats>(who)
        .and_then(|stats| stats.pool_level(sim.timeline(), pool(name)))
        .unwrap()
}

fn act(sim: &mut Sim, who: EntityId, name: &str, params: Params) {
    let action = sim
        .defs()
        .lookup::<ActionDef>(&format!("core:{name}"))
        .unwrap();
    sim.submit(Command::Act {
        entity: who,
        action,
        params,
        how: How::Start,
    })
    .unwrap_or_else(|error| panic!("{name}: {error}"));
}

fn run_for(sim: &mut Sim, micros: i64) {
    let until = Tick(sim.now().as_micros() + micros);
    loop {
        match sim.advance(AdvanceLimit::Until(until)) {
            Advance::Reached(_) => return,
            Advance::Yielded => {}
            other => panic!("{other:?}"),
        }
    }
}

fn named<'a>(events: &'a [SimEvent], name: &str) -> Vec<&'a SimEvent> {
    events.iter().filter(|event| event.name() == name).collect()
}

fn no_faults(events: &[SimEvent]) {
    let faults: Vec<String> = events
        .iter()
        .filter(|event| event.name() == "fault")
        .map(|event| format!("{event:?}"))
        .collect();
    assert!(faults.is_empty(), "{faults:#?}");
}

fn text(value: &str) -> clash_core::Value {
    clash_core::Value::Str(value.to_owned())
}

fn element(sim: &Sim, at: Location) -> clash_core::Value {
    sim.subject_value(Subject::Element(at, Layer::Fixture))
        .to_stored()
}

fn entity_value(sim: &Sim, who: EntityId) -> clash_core::Value {
    sim.subject_value(Subject::Entity(who)).to_stored()
}

/// Whether two simulations agree in everything but the world: the
/// helper that saves and loads hands back only the base grid's columns,
/// and the test city has more.
fn same_state(a: &Sim, b: &Sim) -> bool {
    let (a, b) = (a.hashes(), b.hashes());
    a.timeline == b.timeline
        && a.entities == b.entities
        && a.components == b.components
        && a.rules == b.rules
        && a.settings == b.settings
        && a.rng == b.rng
        && a.kernel == b.kernel
}

fn device_at(sim: &Sim, at: Location) -> EntityId {
    device::device_at(sim, at, Layer::Fixture).expect("a device there")
}

// ---- G2: devices ------------------------------------------------------------

#[test]
fn four_hundred_passive_lamps_create_no_entities_and_switching_one_off_creates_one() {
    let (mut sim, _, _) = city(41);
    for x in 0..20 {
        for y in 0..20 {
            put_fixture(
                &mut sim,
                Location::new(GRID, CellPos::new(16 + x, 30 + y, 0)),
                "test_lamp#on",
            );
        }
    }
    assert_eq!(sim.components().store::<Device>().len(), 0);
    let world = Arc::clone(sim.truth().cosmos().defs());
    let off = world.fixture_state("test_lamp#off", None).unwrap().raw();
    let one = Location::new(GRID, CellPos::new(20, 35, 0));
    sim.world(|ctx| ctx.edit(Cause::DEBUG).set_element(one, Layer::Fixture, off))
        .unwrap();
    sim.pump();
    assert_eq!(sim.components().store::<Device>().len(), 1);
    assert!(device::device_at(&sim, one, Layer::Fixture).is_some());
}

#[test]
fn set_config_is_kept_saved_and_raises_device_configured() {
    let (mut sim, _, _) = city(42);
    let at = cell(20, 1);
    put_fixture(&mut sim, at, "test_lamp#on");
    let lamp =
        device::device_for(&mut sim, Target::Element(at, Layer::Fixture), Cause::DEBUG).unwrap();
    assert_eq!(
        device::config_value(&sim, lamp, "brightness"),
        Some(clash_core::Value::Real(Real::ONE))
    );
    device::set_config(
        &mut sim,
        lamp,
        "brightness",
        &Value::Real(Real::of(1.5)),
        Cause::DEBUG,
    )
    .unwrap();
    let refused = device::set_config(
        &mut sim,
        lamp,
        "brightness",
        &Value::Real(Real::of(3.0)),
        Cause::DEBUG,
    )
    .unwrap_err();
    assert_eq!(refused.code, "device.bad_config_value");
    let refused = device::set_config(&mut sim, lamp, "shade", &Value::text("green"), Cause::DEBUG)
        .unwrap_err();
    assert_eq!(refused.code, "device.bad_config_value");
    sim.pump();
    let events = sim.drain_events();
    let configured = named(&events, "device_configured");
    assert_eq!(configured.len(), 1);
    let (mut loaded, _) = common::save_and_load(&sim);
    assert!(same_state(&loaded, &sim));
    assert_eq!(
        device::config_value(&loaded, lamp, "brightness"),
        Some(clash_core::Value::Real(Real::of(1.5)))
    );
    // A device whose settings are as made is plain again.
    device::set_config(
        &mut loaded,
        lamp,
        "brightness",
        &Value::Real(Real::ONE),
        Cause::DEBUG,
    )
    .unwrap();
    assert!(device::device_at(&loaded, at, Layer::Fixture).is_none());
}

// ---- G6: recipes and work ---------------------------------------------------

#[test]
fn completion_follows_every_change_of_rate_exactly() {
    let (mut sim, _, _) = city(43);
    let at = cell(20, 1);
    put_fixture(&mut sim, at, "test_kiln");
    let kiln =
        device::device_for(&mut sim, Target::Element(at, Layer::Fixture), Cause::DEBUG).unwrap();
    device::add_resource(&mut sim, kiln, "fuel", Real::of(10.0), Cause::DEBUG).unwrap();
    let start = sim.now().as_micros();
    device::set_mode(&mut sim, kiln, "on", Cause::DEBUG).unwrap();
    assert!(recipe::has_process(&sim, kiln));
    run_for(&mut sim, 4 * SECOND);
    assert_eq!(recipe::progress(&sim, kiln), Some(Real::of(0.4)));
    // Off: it stops where it is, and draws nothing.
    device::set_mode(&mut sim, kiln, "off", Cause::DEBUG).unwrap();
    run_for(&mut sim, 6 * SECOND);
    assert_eq!(recipe::progress(&sim, kiln), Some(Real::of(0.4)));
    assert_eq!(device::store_level(&sim, kiln, "fuel"), Some(Real::of(8.0)));
    sim.drain_events();
    device::set_mode(&mut sim, kiln, "on", Cause::DEBUG).unwrap();
    run_for(&mut sim, 6 * SECOND - 1);
    assert!(recipe::has_process(&sim, kiln));
    run_for(&mut sim, 1);
    let events = sim.drain_events();
    no_faults(&events);
    let completed = named(&events, "recipe_completed");
    assert_eq!(completed.len(), 1);
    assert_eq!(completed[0].tick.as_micros() - start, 16 * SECOND);
    assert_eq!(device::store_level(&sim, kiln, "fuel"), Some(Real::of(5.0)));
}

#[test]
fn a_store_that_runs_dry_stalls_the_work_and_fuel_resumes_it() {
    let (mut sim, _, _) = city(44);
    let at = cell(20, 1);
    put_fixture(&mut sim, at, "test_kiln");
    let kiln =
        device::device_for(&mut sim, Target::Element(at, Layer::Fixture), Cause::DEBUG).unwrap();
    device::add_resource(&mut sim, kiln, "fuel", Real::of(2.0), Cause::DEBUG).unwrap();
    device::set_mode(&mut sim, kiln, "on", Cause::DEBUG).unwrap();
    run_for(&mut sim, 10 * SECOND);
    let events = sim.drain_events();
    let stalled = named(&events, "recipe_stalled");
    assert_eq!(stalled.len(), 1);
    assert_eq!(recipe::progress(&sim, kiln), Some(Real::of(0.4)));
    let resumed_at = sim.now().as_micros();
    device::add_resource(&mut sim, kiln, "fuel", Real::of(5.0), Cause::DEBUG).unwrap();
    run_for(&mut sim, 7 * SECOND);
    let events = sim.drain_events();
    let completed = named(&events, "recipe_completed");
    assert_eq!(completed.len(), 1);
    assert_eq!(completed[0].tick.as_micros() - resumed_at, 6 * SECOND);
}

#[test]
fn a_machine_on_half_supply_takes_twice_as_long() {
    let (mut sim, _, _) = city(45);
    let at = cell(20, 1);
    put_fixture(&mut sim, at, "still");
    let ale = make(&mut sim, "core:mug_of_ale", 1, Place::at(at));
    device::put_inside(&mut sim, ale, at, Layer::Fixture, Cause::DEBUG).unwrap();
    let still = device_at(&sim, at);
    device::add_resource(&mut sim, still, "fuel", Real::of(10.0), Cause::DEBUG).unwrap();
    let start = sim.now().as_micros();
    device::set_mode(&mut sim, still, "lit", Cause::DEBUG).unwrap();
    run_for(&mut sim, 300 * SECOND);
    let events = sim.drain_events();
    no_faults(&events);
    let completed = named(&events, "recipe_completed");
    assert_eq!(completed.len(), 1);
    // Two minutes of work at half supply.
    assert_eq!(completed[0].tick.as_micros() - start, 240 * SECOND);
}

#[test]
fn a_bench_recipe_finds_inputs_on_a_rack_within_range_and_not_beyond() {
    let (mut sim, ada, _) = city(46);
    put_fixture(&mut sim, cell(17, 2), "workbench");
    put_fixture(&mut sim, cell(23, 2), "rack");
    put_fixture(&mut sim, cell(24, 2), "rack");
    give(&mut sim, ada, "core:knife", 1);
    let far = make(&mut sim, "core:log", 1, Place::at(cell(24, 2)));
    device::put_inside(&mut sim, far, cell(24, 2), Layer::Fixture, Cause::DEBUG).unwrap();
    let bench = Target::Element(cell(17, 2), Layer::Fixture);
    let refused = recipe::work::check(&sim, Some(ada), recipe_id("club"), Some(bench)).unwrap_err();
    assert_eq!(refused.code, "recipe.missing_input");
    let near = make(&mut sim, "core:log", 1, Place::at(cell(23, 2)));
    device::put_inside(&mut sim, near, cell(23, 2), Layer::Fixture, Cause::DEBUG).unwrap();
    assert!(recipe::work::check(&sim, Some(ada), recipe_id("club"), Some(bench)).is_ok());
    let params = vec![
        ("recipe".to_owned(), text("core:club")),
        ("station".to_owned(), element(&sim, cell(17, 2))),
    ];
    act(&mut sim, ada, "make", params);
    run_for(&mut sim, 61 * SECOND);
    let events = sim.drain_events();
    no_faults(&events);
    assert_eq!(named(&events, "recipe_completed").len(), 1);
    // The near rack gave its log; the far one kept its own.
    assert!(device::device_at(&sim, cell(23, 2), Layer::Fixture).is_none());
    assert_eq!(sim.truth().contents(device_at(&sim, cell(24, 2))).len(), 1);
    assert!(
        !sim.actions_of(ada).iter().any(|_| true),
        "the work action ended"
    );
}

#[test]
fn a_crop_advances_only_while_its_condition_holds() {
    let (mut sim, _, _) = city(47);
    let bed = cell(30, 1);
    put_fixture(&mut sim, bed, "herb_bed");
    let world = Arc::clone(sim.truth().cosmos().defs());
    let wall = world.block_state("wall", None).unwrap().raw();
    for (x, y) in [(29, 1), (31, 1), (30, 0), (30, 2)] {
        sim.world(|ctx| {
            ctx.edit(Cause::DEBUG)
                .set_element(cell(x, y), Layer::Block, wall)
        })
        .unwrap();
    }
    let seed = make(&mut sim, "core:herbs", 1, Place::at(bed));
    device::put_inside(&mut sim, seed, bed, Layer::Fixture, Cause::DEBUG).unwrap();
    sim.pump();
    let device = device_at(&sim, bed);
    assert!(recipe::has_process(&sim, device));
    run_for(&mut sim, 65 * SECOND);
    assert_eq!(recipe::progress(&sim, device), Some(Real::ZERO));
    let water = world.fields().resolve("water", Some("core")).unwrap();
    sim.world(|ctx| ctx.edit(Cause::DEBUG).add_field(bed, water, 100))
        .unwrap();
    run_for(&mut sim, 60 * SECOND);
    let wet = recipe::progress(&sim, device).unwrap();
    assert!(wet > Real::ZERO);
    sim.world(|ctx| ctx.edit(Cause::DEBUG).set_field(bed, water, 0))
        .unwrap();
    run_for(&mut sim, 31 * SECOND);
    let dried = recipe::progress(&sim, device).unwrap();
    run_for(&mut sim, 300 * SECOND);
    assert_eq!(recipe::progress(&sim, device), Some(dried));
}

// ---- G17: know-how ----------------------------------------------------------

fn know_how(name: &str) -> recipe::KnowHowId {
    DEFS.lookup::<KnowHowDef>(&format!("core:{name}")).unwrap()
}

#[test]
fn a_recipe_that_needs_know_how_is_unavailable_until_known_or_provided() {
    let (mut sim, ada, _) = city(48);
    put_fixture(&mut sim, cell(17, 2), "workbench");
    give(&mut sim, ada, "core:club", 1);
    give(&mut sim, ada, "core:herbs", 2);
    let bench = Target::Element(cell(17, 2), Layer::Fixture);
    let refused =
        recipe::work::check(&sim, Some(ada), recipe_id("healing_salve"), Some(bench)).unwrap_err();
    assert_eq!(refused.code, "recipe.unknown_how");
    // A herbal on the bench provides it.
    let book = make(&mut sim, "core:herbal", 1, Place::at(cell(17, 2)));
    device::put_inside(&mut sim, book, cell(17, 2), Layer::Fixture, Cause::DEBUG).unwrap();
    assert!(recipe::work::check(&sim, Some(ada), recipe_id("healing_salve"), Some(bench)).is_ok());
    // Taken away again, it does not; read, it is known.
    let book = clash_sim::item::take_out(&mut sim, book, None, Cause::DEBUG).unwrap();
    assert!(recipe::work::check(&sim, Some(ada), recipe_id("healing_salve"), Some(bench)).is_err());
    assert_eq!(
        recipe::know_how::learn_from(&mut sim, ada, book, Cause::DEBUG),
        Ok(1)
    );
    assert!(recipe::know_how::knows(&sim, ada, know_how("herbalism")));
    assert!(recipe::work::check(&sim, Some(ada), recipe_id("healing_salve"), Some(bench)).is_ok());
}

#[test]
fn a_machine_working_alone_runs_a_recipe_only_with_a_pattern_loaded() {
    let (mut sim, _, _) = city(49);
    let at = cell(20, 1);
    put_fixture(&mut sim, at, "test_press");
    let ingot = make(&mut sim, "core:test_ingot", 1, Place::at(at));
    device::put_inside(&mut sim, ingot, at, Layer::Fixture, Cause::DEBUG).unwrap();
    let press = device_at(&sim, at);
    device::set_mode(&mut sim, press, "on", Cause::DEBUG).unwrap();
    sim.pump();
    assert!(!recipe::has_process(&sim, press));
    let pattern = make(&mut sim, "core:test_pattern", 1, Place::at(at));
    device::put_inside(&mut sim, pattern, at, Layer::Fixture, Cause::DEBUG).unwrap();
    sim.pump();
    assert!(recipe::has_process(&sim, press));
    run_for(&mut sim, 11 * SECOND);
    let events = sim.drain_events();
    no_faults(&events);
    assert_eq!(named(&events, "recipe_completed").len(), 1);
}

#[test]
fn teach_is_refused_while_what_it_needs_is_not_known() {
    let (mut sim, ada, _) = city(50);
    let refused = recipe::know_how::teach(
        &mut sim,
        ada,
        know_how("test_engraving"),
        "taught",
        Cause::DEBUG,
    )
    .unwrap_err();
    assert_eq!(refused.code, "know_how.needs");
    assert_eq!(
        recipe::know_how::teach(
            &mut sim,
            ada,
            know_how("test_casting"),
            "taught",
            Cause::DEBUG
        ),
        Ok(true)
    );
    assert_eq!(
        recipe::know_how::teach(
            &mut sim,
            ada,
            know_how("test_engraving"),
            "taught",
            Cause::DEBUG
        ),
        Ok(true)
    );
    assert_eq!(
        recipe::know_how::unlearn(&mut sim, ada, know_how("test_casting"), Cause::DEBUG),
        Ok(true)
    );
    assert!(!recipe::know_how::knows(
        &sim,
        ada,
        know_how("test_casting")
    ));
}

#[test]
fn know_how_moves_with_a_transferred_mind_and_stays_out_of_the_body_left() {
    let (mut sim, ada, bram) = city(51);
    recipe::know_how::teach(&mut sim, ada, know_how("herbalism"), "taught", Cause::DEBUG).unwrap();
    sim.transfer_mind(ada, bram, true, Cause::DEBUG).unwrap();
    assert!(recipe::know_how::knows(&sim, bram, know_how("herbalism")));
    assert!(!recipe::know_how::knows(&sim, ada, know_how("herbalism")));
}

// ---- G19: substances --------------------------------------------------------

#[test]
fn a_litre_poured_out_drained_back_and_drawn_again_is_a_litre() {
    let (mut sim, ada, _) = city(52);
    let skin = give(&mut sim, ada, "core:waterskin", 1);
    let litre = substance::fill_of(&sim, skin);
    assert_eq!(litre, Real::of(0.001));
    let floor = cell(25, 1);
    let poured = substance::pour(&mut sim, skin, PourTo::Cell(floor), None, Cause::DEBUG).unwrap();
    assert_eq!(poured, litre);
    assert_eq!(substance::fill_of(&sim, skin), Real::ZERO);
    let back = substance::fill(&mut sim, skin, Subject::Loc(floor), Cause::DEBUG).unwrap();
    assert_eq!(back, litre);
    let cup = make(&mut sim, "core:mug", 1, Place::at(cell(17, 1)));
    let moved = substance::pour(&mut sim, skin, PourTo::Item(cup), None, Cause::DEBUG).unwrap();
    // A mug takes 400 ml of the litre; the rest stays in the skin.
    assert_eq!(moved + substance::fill_of(&sim, skin), litre);
}

#[test]
fn spiked_drink_ingested_on_the_skin_and_an_antidote() {
    let (mut sim, ada, bram) = city(53);
    let poison = DEFS.lookup::<SubstanceDef>("core:nightshade").unwrap();
    let vial = give(&mut sim, ada, "core:poison_vial", 1);
    let ale = give(&mut sim, ada, "core:mug_of_ale", 1);
    // There is no room in a full mug: half of it is drunk first.
    substance::dose_from(
        &mut sim,
        bram,
        ale,
        Some(Real::of(0.0002)),
        "ingest",
        Cause::DEBUG,
    )
    .unwrap();
    substance::pour(
        &mut sim,
        vial,
        PourTo::Item(ale),
        Some(Real::of(0.000_005)),
        Cause::DEBUG,
    )
    .unwrap();
    assert_eq!(substance::amount_of(&sim, ale, poison), Real::of(0.000_005));
    let got = substance::dose_from(&mut sim, bram, ale, None, "ingest", Cause::DEBUG).unwrap();
    assert!(got > Real::of(204.9));
    // Five millilitres swallowed at four a millilitre, less what the
    // body cleared in no time at all.
    let swallowed = level(&sim, bram, "toxin");
    assert_eq!(swallowed, Real::of(20.0));
    // The same on the skin gets a twentieth of it in.
    substance::dose_body(
        &mut sim,
        ada,
        &[(poison, Real::of(0.000_005))],
        "skin",
        Cause::DEBUG,
    )
    .unwrap();
    assert_eq!(level(&sim, ada, "toxin"), Real::of(1.0));
    // An antidote takes it down.
    let cup = give(&mut sim, bram, "core:test_cup", 1);
    substance::dose_from(&mut sim, bram, cup, None, "ingest", Cause::DEBUG).unwrap();
    assert_eq!(level(&sim, bram, "toxin"), Real::ZERO);
    sim.pump();
    let events = sim.drain_events();
    assert!(!named(&events, "tasted").is_empty());
    assert!(named(&events, "dosed").len() >= 3);
}

#[test]
fn a_poisoned_blade_doses_through_the_skin_and_loses_its_coating() {
    let (mut sim, ada, bram) = city(54);
    let knife = give(&mut sim, ada, "core:knife", 1);
    let vial = make(&mut sim, "core:poison_vial", 1, Place::at(cell(17, 1)));
    substance::coat(&mut sim, knife, vial, Real::of(0.000_004), Cause::DEBUG).unwrap();
    let poison = DEFS.lookup::<SubstanceDef>("core:nightshade").unwrap();
    let kinds = sim.body_defs().unwrap().kinds();
    let arm = sim
        .components()
        .body
        .get(bram)
        .unwrap()
        .part_named("r_arm")
        .unwrap()
        .key();
    let part = Value::Subject(clash_sim::body::props::part_handle(kinds, bram, arm)).to_stored();
    let params = vec![
        ("target".to_owned(), entity_value(&sim, bram)),
        ("part".to_owned(), part),
    ];
    act(&mut sim, ada, "slash", params);
    run_for(&mut sim, 2 * SECOND);
    let events = sim.drain_events();
    no_faults(&events);
    assert!(!named(&events, "strike_landed").is_empty());
    assert!(level(&sim, bram, "toxin") > Real::of(23.0));
    let left = substance::coating_of(&sim, knife)
        .iter()
        .find(|(id, _)| *id == poison)
        .map(|(_, volume)| *volume)
        .unwrap();
    assert!((left - Real::of(0.000_001_5)).abs() < Real::of(1.0e-12));
}

// ---- Using what is made -----------------------------------------------------

#[test]
fn a_person_makes_a_bandage_and_binds_a_wound_with_it() {
    let (mut sim, ada, _) = city(55);
    // A cut to the arm, as a blade's edge gives one.
    let world = Arc::clone(sim.truth().cosmos().defs());
    let cut = world.damage_types().lookup("core:cut").unwrap();
    let arm = sim
        .components()
        .body
        .get(ada)
        .unwrap()
        .part_named("r_arm")
        .unwrap()
        .key();
    let hit = clash_sim::body::Hit {
        target: ada,
        at: clash_sim::body::HitAt::Part(arm),
        packet: clash_world::DamagePacket::new(cut, Real::of(150.0), Real::of(2.0e7), Cause::DEBUG),
        area: Real::of(1.0e-5),
        span: Real::of(0.1),
        dir: None,
    };
    sim.hit(hit).unwrap();
    sim.pump();
    give(&mut sim, ada, "core:cloth", 1);
    assert!(!sim.components().body.get(ada).unwrap().wounds().is_empty());
    act(
        &mut sim,
        ada,
        "make",
        vec![("recipe".to_owned(), text("core:bandage"))],
    );
    run_for(&mut sim, 6 * SECOND);
    let events = sim.drain_events();
    no_faults(&events);
    assert_eq!(named(&events, "recipe_completed").len(), 1);
    let bandage = sim
        .truth()
        .contents(ada)
        .iter()
        .copied()
        .find(|id| {
            sim.truth()
                .entity(*id)
                .is_some_and(|entity| entity.tags.contains("bandage"))
        })
        .expect("a bandage in hand");
    let params = vec![("what".to_owned(), entity_value(&sim, bandage))];
    act(&mut sim, ada, "apply", params);
    run_for(&mut sim, 5 * SECOND);
    let events = sim.drain_events();
    no_faults(&events);
    assert!(!named(&events, "wound_closed").is_empty());
}

#[test]
fn eating_bread_restores_nourishment() {
    let (mut sim, ada, _) = city(56);
    sim.pool_spend(ada, pool("nourishment"), Real::of(50.0), Cause::DEBUG)
        .unwrap();
    let start = level(&sim, ada, "nourishment");
    let loaf = give(&mut sim, ada, "core:loaf", 2);
    let params = vec![("what".to_owned(), entity_value(&sim, loaf))];
    act(&mut sim, ada, "eat", params);
    run_for(&mut sim, 3 * SECOND + 1);
    let events = sim.drain_events();
    no_faults(&events);
    let fed = level(&sim, ada, "nourishment");
    // Forty for the loaf, less what three seconds of hunger took.
    assert!(fed > start + Real::of(39.9) && fed <= start + Real::of(40.0));
    // One of the two is gone.
    assert_eq!(
        sim.components()
            .get::<clash_sim::item::Item>(loaf)
            .map(clash_sim::item::Item::count),
        Some(1)
    );
}

// ---- Saving and replay ------------------------------------------------------

#[test]
fn save_and_load_in_the_middle_of_work_changes_nothing() {
    let (mut sim, ada, _) = city(57);
    give(&mut sim, ada, "core:stick", 1);
    give(&mut sim, ada, "core:cloth", 1);
    act(
        &mut sim,
        ada,
        "make",
        vec![("recipe".to_owned(), text("core:torch"))],
    );
    run_for(&mut sim, 4 * SECOND);
    let (mut loaded, unplaced) = common::save_and_load(&sim);
    assert!(unplaced.is_empty());
    assert!(same_state(&loaded, &sim));
    run_for(&mut sim, 7 * SECOND);
    run_for(&mut loaded, 7 * SECOND);
    assert!(same_state(&loaded, &sim));
    let events = loaded.drain_events();
    assert_eq!(named(&events, "recipe_completed").len(), 1);
}

#[test]
fn replay_reproduces_the_hash() {
    let (mut sim, ada, _) = city(58);
    make(&mut sim, "core:stick", 1, Place::In(ada));
    make(&mut sim, "core:cloth", 1, Place::In(ada));
    act(
        &mut sim,
        ada,
        "make",
        vec![("recipe".to_owned(), text("core:torch"))],
    );
    run_for(&mut sim, 11 * SECOND);
    let defs = Arc::clone(&DEFS);
    let world = WorldDefs::from_defs(&defs).unwrap();
    let mut timeline = Timeline::new();
    let mut cosmos = Cosmos::new(Arc::clone(&world));
    let def = world.realms().lookup("core:test_city").unwrap();
    cosmos.create_realm(&mut timeline, def, CITY_SEED).unwrap();
    let mut replayed = Sim::from_parts(defs, cosmos, timeline, 58);
    replayed.replay(sim.input_log()).unwrap();
    let now = sim.now().as_micros() - replayed.now().as_micros();
    run_for(&mut replayed, now);
    assert_eq!(replayed.state_hash(), sim.state_hash());
}

// ---- What a screen lists ----------------------------------------------------

#[test]
fn recipe_options_say_what_is_at_hand_and_why_not() {
    let (mut sim, ada, _) = city(59);
    put_fixture(&mut sim, cell(17, 2), "workbench");
    give(&mut sim, ada, "core:cloth", 1);
    let rows = sim.property_named(Subject::Entity(ada), "recipe_options", &[]);
    let rows = rows.as_list().unwrap();
    let row = |id: &str| {
        rows.iter()
            .find(|row| {
                row.as_map()
                    .and_then(|map| map.get("id"))
                    .and_then(Value::as_str)
                    == Some(id)
            })
            .and_then(Value::as_map)
            .cloned()
            .unwrap_or_else(|| panic!("no row for {id}"))
    };
    assert_eq!(
        row("core:bandage").get("available"),
        Some(&Value::Bool(true))
    );
    let club = row("core:club");
    assert_eq!(club.get("available"), Some(&Value::Bool(false)));
    assert_eq!(club.get("where").and_then(Value::as_str), Some("south"));
    let salve = row("core:healing_salve");
    assert!(
        salve
            .get("why")
            .and_then(Value::as_str)
            .is_some_and(|why| why.contains("Herbalism"))
    );
}

// ---- The document states what is built --------------------------------------

#[test]
fn document_16_states_every_event_verb_property_and_reason_of_making() {
    let document = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../docs/16-devices-and-machines.md");
    let text = std::fs::read_to_string(document).expect("document 16 can be read");
    let registry = DEFS.expr_registry();
    let events = recipe::event_types()
        .into_iter()
        .chain(substance::event_types())
        .chain(device::event_types());
    for event in events {
        assert!(
            registry.event_id(&event.name).is_some(),
            "`{}` is not registered",
            event.name
        );
        assert!(
            text.contains(&format!("| `{}`", event.name)),
            "16 does not state the event `{}`",
            event.name
        );
        for field in &event.fields {
            assert!(
                text.contains(&format!("`{}`", field.name)),
                "16 does not state `{}.{}`",
                event.name,
                field.name
            );
        }
    }
    let verbs = recipe::verbs()
        .into_iter()
        .chain(substance::verbs())
        .chain(device::verbs());
    for verb in verbs {
        assert!(
            registry.verb_id(verb.name).is_some(),
            "`{}` is not registered",
            verb.name
        );
        assert!(
            text.contains(&format!("`{}`", verb.name)),
            "16 does not state the verb `{}`",
            verb.name
        );
    }
    let properties = recipe::properties()
        .into_iter()
        .chain(substance::properties())
        .chain(device::properties());
    for property in properties {
        assert!(
            text.contains(&format!("`{}.{}", property.kind, property.name)),
            "16 does not state the property `{}.{}`",
            property.kind,
            property.name
        );
    }
    let reasons = recipe::RECIPE_REASONS
        .iter()
        .chain(substance::SUBSTANCE_REASONS)
        .chain(device::DEVICE_REASONS);
    for reason in reasons {
        assert!(
            text.contains(&format!("`{}`", reason.code)),
            "16 does not state the reason `{}`",
            reason.code
        );
    }
}

#[test]
fn every_property_of_making_reads_what_it_says() {
    let (mut sim, ada, _) = city(60);
    let at = cell(20, 1);
    put_fixture(&mut sim, at, "test_kiln");
    let kiln =
        device::device_for(&mut sim, Target::Element(at, Layer::Fixture), Cause::DEBUG).unwrap();
    device::add_resource(&mut sim, kiln, "fuel", Real::of(4.0), Cause::DEBUG).unwrap();
    device::set_mode(&mut sim, kiln, "on", Cause::DEBUG).unwrap();
    let get = |sim: &Sim, subject: Subject, name: &str, args: &[Value]| {
        sim.property_named(subject, name, args)
    };
    let kiln_subject = Subject::Entity(kiln);
    let element = Subject::Element(at, Layer::Fixture);
    assert_eq!(get(&sim, kiln_subject, "mode", &[]), Value::from("on"));
    assert_eq!(get(&sim, element, "mode", &[]), Value::from("on"));
    assert_eq!(
        get(&sim, kiln_subject, "store", &[Value::from("fuel")]),
        Value::Real(Real::of(4.0))
    );
    assert_eq!(
        get(&sim, element, "store", &[Value::from("fuel")]),
        Value::Real(Real::of(4.0))
    );
    assert!(get(&sim, kiln_subject, "config", &[]).as_map().is_some());
    let making = get(&sim, kiln_subject, "making", &[]);
    assert_eq!(
        making
            .as_map()
            .and_then(|map| map.get("recipe"))
            .and_then(Value::as_str),
        Some("core:test_fire")
    );
    let me = Subject::Entity(ada);
    let bandage = Value::from("core:bandage");
    assert_eq!(
        get(&sim, me, "can_make", &[bandage.clone(), Value::Null]),
        Value::Bool(false)
    );
    assert!(
        get(&sim, me, "make_fault", &[bandage, Value::Null])
            .as_str()
            .is_some()
    );
    assert!(get(&sim, me, "recipe_options", &[]).as_list().is_some());
    let herbalism = Value::from("core:herbalism");
    assert_eq!(
        get(&sim, me, "knows_how", &[herbalism.clone()]),
        Value::Bool(false)
    );
    recipe::know_how::teach(&mut sim, ada, know_how("herbalism"), "taught", Cause::DEBUG).unwrap();
    assert_eq!(get(&sim, me, "knows_how", &[herbalism]), Value::Bool(true));
    assert_eq!(
        get(&sim, me, "know_how", &[]).as_list().map(<[Value]>::len),
        Some(1)
    );
    assert!(
        get(&sim, me, "know_how_list", &[])
            .as_list()
            .is_some_and(|rows| rows.len() >= 3)
    );
    let skin = give(&mut sim, ada, "core:waterskin", 1);
    let held = Subject::Entity(skin);
    assert_eq!(get(&sim, held, "fill", &[]), Value::Real(Real::of(0.001)));
    assert_eq!(get(&sim, held, "room", &[]), Value::Real(Real::ZERO));
    assert_eq!(
        get(&sim, held, "holds_substance", &[Value::from("core:water")]),
        Value::Real(Real::of(0.001))
    );
    assert!(
        get(&sim, held, "mixture", &[])
            .as_map()
            .is_some_and(|map| map.len() == 1)
    );
    assert!(
        get(&sim, held, "coating", &[])
            .as_map()
            .is_some_and(|map| map.is_empty())
    );
}
