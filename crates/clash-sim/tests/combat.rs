//! Combat against the real simulation (07; plans/combat.md, packages C2,
//! C3 and C5): people spawned from `core:person`, armed with the items
//! of `mods/core`, fighting with the actions of
//! `mods/core/data/combat`, in the test city and on flat ground.
//!
//! C2: an edge and a fist at equal energy give the intensities of
//! 07 §6.1 and differ in depth as 06 §7.2 says; a blow below the outer
//! threshold comes back into the fist; energy is accounted for over
//! random strikes (invariant C2); `own_fist`; `one_cut`; a punch to an
//! unarmoured head wounds and bleeds, the same blow on a helmet dents
//! the helmet and wounds less; a slash severs a finger-sized part only
//! at the energy the body model predicts; save and load in the middle of
//! a strike's phases and replay change nothing.
//!
//! `own_fist` and `one_cut` are Rust tests: the console host has not
//! landed on the build branch.

mod common;

use std::sync::{Arc, LazyLock};

use clash_core::{EntityId, Real, Tick};
use clash_data::Defs;
use clash_expr::Value;
use clash_sim::action::{ActionDef, ActionDefId, How, Params};
use clash_sim::body::props::part_handle;
use clash_sim::body::{BodyPartDef, PartKey};
use clash_sim::combat::strike::{StrikeArgs, StrikeTally, strike};
use clash_sim::stats::PoolId;
use clash_sim::template::{CharacterSpec, Selection};
use clash_sim::{
    Advance, AdvanceLimit, Command, Control, DebugCommand, Place, Sim, SimEvent, Subject,
};
use clash_time::Timeline;
use clash_world::{Cause, CellPos, Cosmos, GridId, Layer, Location, WorldDefs};
use proptest::prelude::*;

const MS: i64 = 1_000;

/// Definitions the tests add: actions whose energy and push are given
/// with the command, so that one blow can be compared with another; a
/// blade long enough to cut down a rank; a closed helm.
const FIXTURES: &str = r#"[
  { "type": "item", "id": "visor", "name": "visor",
    "material": "steel", "mass": "2 kg", "volume": 0.0005, "length": "0.3 m",
    "grip": { "arm": 1 },
    "wear": { "parts": "head", "layer": 5, "thickness": "3 mm", "coverage": 1 } },

  { "type": "action", "id": "test_fist",
    "params": { "target": "entity", "part": { "type": "part?", "default": "null" },
                "energy": "real", "push": { "type": "real", "default": "0" } },
    "attacks": { "fist": { "with": { "part": "hand", "surface": "fist" },
                           "energy": "$energy", "push": "$push", "reach": 2 } },
    "phases": [ { "id": "hit", "time": 0,
      "do": [ { "strike": { "target": "$target", "part": "$part" } } ] } ] },

  { "type": "action", "id": "test_edge",
    "params": { "target": "entity", "part": { "type": "part?", "default": "null" },
                "energy": "real", "push": { "type": "real", "default": "0" } },
    "attacks": { "edge": { "with": { "held": "blade", "surface": "edge" },
                           "energy": "$energy", "push": "$push", "reach": 2 } },
    "phases": [ { "id": "hit", "time": 0,
      "do": [ { "strike": { "target": "$target", "part": "$part" } } ] } ] },

  { "type": "action", "id": "test_point",
    "params": { "target": "entity", "part": { "type": "part?", "default": "null" },
                "energy": "real", "push": { "type": "real", "default": "0" } },
    "attacks": { "point": { "with": { "held": "blade", "surface": "point" },
                            "energy": "$energy", "push": "$push", "reach": 2 } },
    "phases": [ { "id": "hit", "time": 0,
      "do": [ { "strike": { "target": "$target", "part": "$part" } } ] } ] },

  { "type": "action", "id": "test_ram",
    "params": { "target": "entity", "part": { "type": "part?", "default": "null" },
                "energy": "real", "push": { "type": "real", "default": "0" } },
    "attacks": { "ram": { "with": { "kind": "impact", "area": 0.05, "give": "5 cm", "span": "30 cm" },
                          "energy": "$energy", "push": "$push", "reach": 2 } },
    "phases": [ { "id": "hit", "time": 0,
      "do": [ { "strike": { "target": "$target", "part": "$part" } } ] } ] },

  { "type": "action", "id": "test_wall_punch",
    "params": { "at": "cell", "energy": "real" },
    "attacks": { "fist": { "with": { "part": "hand", "surface": "fist" }, "energy": "$energy" } },
    "phases": [ { "id": "hit", "time": 0,
      "do": [ { "strike": { "at": "$at" } } ] } ] },

  { "type": "action", "id": "test_cleave",
    "params": { "target": "entity", "part": { "type": "part?", "default": "null" }, "energy": "real" },
    "attacks": { "cleave": { "with": { "held": "blade", "surface": "edge" },
                             "energy": "$energy", "reach": 8,
                             "arc": { "line": "8 m", "max": 6, "through": true } } },
    "phases": [ { "id": "hit", "time": 0,
      "do": [ { "strike": { "target": "$target", "part": "$part" } } ] } ] }
]"#;

/// The JSON files of a directory of `mods/core/data`, as paths within the
/// mod and their text, in file-name order.
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

/// `mods/core/data/items.json` without its colours, which are the
/// interface's definition type.
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

/// `mods/core`'s world, life, movement, items and combat, and the
/// fixtures, loaded once.
static DEFS: LazyLock<Arc<Defs>> = LazyLock::new(|| {
    let mut owned = core_dir("life");
    owned.extend(core_dir("combat"));
    let movement = common::mods_dir().join("core/data/movement.json");
    owned.push((
        "data/movement.json".to_owned(),
        std::fs::read_to_string(movement).unwrap(),
    ));
    owned.push(("data/items.json".to_owned(), items()));
    owned.push(("data/test/combat.json".to_owned(), FIXTURES.to_owned()));
    let files: Vec<(&str, &str)> = owned
        .iter()
        .map(|(path, text)| (path.as_str(), text.as_str()))
        .collect();
    common::try_defs_with(&files).unwrap_or_else(|error| panic!("{error}"))
});

fn person(name: &str) -> CharacterSpec {
    CharacterSpec::of(["core:person"]).with("core:person.name", Selection::Text(name.to_owned()))
}

fn debug(sim: &mut Sim, command: DebugCommand) {
    sim.submit(Command::Debug(command))
        .unwrap_or_else(|error| panic!("{error}"));
}

/// Spawns a person by command, so that a replay makes them too.
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

// ---- the test city --------------------------------------------------------

/// The base grid of the city.
const GRID: GridId = GridId::new(0);

/// The seed of the city: what `cargo run -p clash-gen --example
/// city_ascii -- 1 0` prints. The first block's building fills cells 4
/// to 14 each way; its east wall stands at x = 14, the sidewalk at 15
/// and the street beyond.
const CITY_SEED: u64 = 1;

fn cell(x: i32, y: i32) -> Location {
    Location::new(GRID, CellPos::new(x, y, 0))
}

/// A simulation over the test city with two people standing in the
/// street, `ada` at (17, 1) and `bram` east of her at (18, 1).
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
    debug(
        &mut sim,
        DebugCommand::SetControl {
            entity: ada,
            control: Control::Delegated,
        },
    );
    sim.pump();
    (sim, ada, bram)
}

// ---- helpers --------------------------------------------------------------

fn action(sim: &Sim, name: &str) -> ActionDefId {
    sim.defs()
        .lookup::<ActionDef>(&format!("core:{name}"))
        .unwrap_or_else(|| panic!("no action {name}"))
}

fn entity_value(sim: &Sim, who: EntityId) -> clash_core::Value {
    sim.subject_value(Subject::Entity(who)).to_stored()
}

fn key(sim: &Sim, who: EntityId, part: &str) -> PartKey {
    sim.components()
        .body
        .get(who)
        .unwrap()
        .part_named(part)
        .unwrap_or_else(|| panic!("no part {part}"))
        .key()
}

fn part_value(sim: &Sim, who: EntityId, part: &str) -> clash_core::Value {
    let kinds = sim.body_defs().unwrap().kinds();
    Value::Subject(part_handle(kinds, who, key(sim, who, part))).to_stored()
}

fn real(value: f64) -> clash_core::Value {
    clash_core::Value::Real(Real::of(value))
}

fn act(sim: &mut Sim, who: EntityId, name: &str, params: Params) {
    let action = action(sim, name);
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

fn faults(events: &[SimEvent]) -> Vec<String> {
    events
        .iter()
        .filter(|event| event.name() == "fault")
        .map(|event| format!("{event:?}"))
        .collect()
}

fn named<'a>(events: &'a [SimEvent], name: &str) -> Vec<&'a SimEvent> {
    events.iter().filter(|event| event.name() == name).collect()
}

fn field(event: &SimEvent, name: &str) -> Real {
    event
        .body
        .field(name)
        .and_then(|value| value.as_real())
        .unwrap_or_else(|| panic!("{} has no real `{name}`", event.name()))
}

fn int_field(event: &SimEvent, name: &str) -> i64 {
    event
        .body
        .field(name)
        .and_then(|value| value.plain().and_then(clash_core::Value::as_int))
        .unwrap_or_else(|| panic!("{} has no int `{name}`", event.name()))
}

fn about(event: &SimEvent, who: EntityId) -> bool {
    event.subject("self") == Some(Subject::Entity(who))
}

/// The damage of each layer of a part, outermost first.
fn layers(sim: &Sim, who: EntityId, part: &str) -> Vec<u8> {
    sim.components()
        .body
        .get(who)
        .unwrap()
        .part_named(part)
        .unwrap()
        .layers()
        .iter()
        .map(|layer| layer.damage())
        .collect()
}

fn pool(sim: &Sim, name: &str) -> PoolId {
    sim.stat_defs().unwrap().pool_id(name).unwrap()
}

fn level(sim: &Sim, who: EntityId, name: &str) -> f64 {
    let stats = sim.components().stats.get(who).unwrap();
    stats
        .pool_level(sim.timeline(), pool(sim, name))
        .unwrap()
        .get()
}

/// Makes an item of `mods/core` and puts it in someone's hand.
fn arm(sim: &mut Sim, who: EntityId, def: &str) -> EntityId {
    let item = sim.truth().entities().next_id().unwrap();
    debug(
        sim,
        DebugCommand::CreateItem {
            def: def.to_owned(),
            count: 1,
            place: Place::Stored,
        },
    );
    clash_sim::item::take(sim, who, item, Cause::DEBUG).unwrap();
    item
}

/// Makes an item and puts it on someone.
fn dress(sim: &mut Sim, who: EntityId, def: &str) -> EntityId {
    let item = sim.truth().entities().next_id().unwrap();
    debug(
        sim,
        DebugCommand::CreateItem {
            def: def.to_owned(),
            count: 1,
            place: Place::Stored,
        },
    );
    clash_sim::item::wear(sim, who, item, Cause::DEBUG).unwrap();
    item
}

/// Strikes with one of the fixture actions directly, through
/// `combat::strike::strike`, as the `strike` effect of its one phase
/// would: the locals are built from the parameters given.
fn strike_with(
    sim: &mut Sim,
    who: EntityId,
    name: &str,
    target: EntityId,
    part: Option<&str>,
    params: &[(&str, Value)],
) -> StrikeTally {
    let id = action(sim, name);
    let def = sim.defs().get::<ActionDef>(id);
    let locals: Vec<Value> = def
        .locals()
        .iter()
        .map(|local| {
            if local.name == "target" {
                return sim.subject_value(Subject::Entity(target));
            }
            if local.name == "part" {
                return part.map_or(Value::Null, |part| {
                    Value::from(part_value(sim, target, part))
                });
            }
            params
                .iter()
                .find(|(name, _)| *name == local.name)
                .map_or(Value::Null, |(_, value)| value.clone())
        })
        .collect();
    let part = part.map(|part| (target, key(sim, target, part)));
    let args = StrikeArgs {
        target: Some(target),
        at: None,
        part,
        effort: Real::ONE,
        sure: false,
    };
    let tally = strike(sim, who, id, 0, &args, &locals, Cause::DEBUG.by(who))
        .unwrap_or_else(|reason| panic!("{name}: {reason}"));
    sim.pump();
    tally
}

/// A simulation over flat ground with people standing in a row from
/// (10, 10) eastward, one for each name.
fn row(seed: u64, names: &[&str]) -> (Sim, Vec<EntityId>) {
    let mut sim = common::sim_over(Arc::clone(&DEFS), seed);
    let people = names
        .iter()
        .enumerate()
        .map(|(at, name)| {
            let x = 10 + i32::try_from(at).unwrap();
            spawn(&mut sim, name, common::on(x, 10))
        })
        .collect();
    sim.pump();
    (sim, people)
}

// ---- C2: surfaces and delivery --------------------------------------------

#[test]
fn an_edge_and_a_fist_at_equal_energy_differ_by_intensity_and_depth() {
    let energy = 60.0;
    let (mut sim, people) = row(3, &["Ada", "Bram", "Cora"]);
    let (ada, bram, cora) = (people[0], people[1], people[2]);
    arm(&mut sim, ada, "core:knife");
    sim.drain_events();

    // The fist: 5e-3 m² over 2 cm of give.
    let fist = strike_with(
        &mut sim,
        ada,
        "test_fist",
        bram,
        Some("torso"),
        &[("energy", Value::Real(Real::of(energy)))],
    );
    let told = sim.drain_events();
    let landed = named(&told, "strike_landed");
    assert_eq!(landed.len(), 1, "{:?}", faults(&told));
    let intensity = field(landed[0], "intensity").get();
    assert!(
        (intensity - energy / (5.0e-3 * 0.02)).abs() < 1e-6,
        "{intensity}"
    );
    let fist_depth = named(&told, "hit_landed")
        .into_iter()
        .find(|event| about(event, bram))
        .map(|event| int_field(event, "depth"))
        .unwrap();

    // The edge of the knife: 1e-5 m² over 2 mm.
    let edge = strike_with(
        &mut sim,
        ada,
        "test_edge",
        cora,
        Some("torso"),
        &[("energy", Value::Real(Real::of(energy)))],
    );
    let told = sim.drain_events();
    let landed = named(&told, "strike_landed");
    assert_eq!(landed.len(), 1, "{:?}", faults(&told));
    let intensity = field(landed[0], "intensity").get();
    assert!(
        (intensity - energy / (1.0e-5 * 0.002)).abs() < 1e-3,
        "{intensity}"
    );
    let edge_depth = named(&told, "hit_landed")
        .into_iter()
        .find(|event| about(event, cora))
        .map(|event| int_field(event, "depth"))
        .unwrap();

    // 06 §7.2: the fist's energy stays in the first layers; the edge
    // spends little in each and goes through every one.
    let torso_layers = 3;
    assert_eq!(fist_depth, 1, "the fist breaches only the skin");
    assert_eq!(edge_depth, torso_layers, "the edge goes through");
    assert!(fist.absorbed > edge.absorbed);
    assert_eq!(fist.passed, Real::ZERO);
    assert!(edge.passed > Real::ZERO);
}

#[test]
fn a_blow_below_the_outer_threshold_comes_back_into_the_fist() {
    // 10 J over 1e-4 m³ is 100 kPa: skin wants 200 kPa before it is
    // affected at all.
    let (mut sim, people) = row(5, &["Ada", "Bram"]);
    let (ada, bram) = (people[0], people[1]);
    sim.drain_events();
    let before = layers(&sim, bram, "head");
    let tally = strike_with(
        &mut sim,
        ada,
        "test_fist",
        bram,
        Some("head"),
        &[("energy", Value::Real(Real::of(10.0)))],
    );
    let told = sim.drain_events();
    assert!(tally.landed);
    assert_eq!(tally.returned, Real::of(10.0));
    assert_eq!(tally.absorbed, Real::ZERO);
    assert_eq!(layers(&sim, bram, "head"), before);
    let recoiled = named(&told, "strike_recoiled");
    assert_eq!(recoiled.len(), 1, "{:?}", faults(&told));
    assert_eq!(field(recoiled[0], "energy"), Real::of(10.0));
    assert!(about(recoiled[0], ada));
}

#[test]
fn own_fist() {
    // A strong, fragile striker: a kilojoule into a concrete wall, whose
    // impact threshold is 20 MPa. The fist's 10 MPa does nothing to the
    // wall, and all of it comes back into the hand.
    let (mut sim, people) = row(7, &["Ada"]);
    let ada = people[0];
    let wall = common::wall(&sim);
    let at = common::at(11, 10);
    debug(
        &mut sim,
        DebugCommand::SetElement {
            loc: at,
            layer: Layer::Block,
            state: wall.raw(),
        },
    );
    sim.drain_events();
    let hand = sim
        .components()
        .body
        .get(ada)
        .unwrap()
        .parts()
        .iter()
        .find(|part| part.name().ends_with("hand"))
        .map(|part| part.name().to_owned())
        .unwrap();
    let params = vec![
        (
            "at".to_owned(),
            sim.subject_value(Subject::Loc(at)).to_stored(),
        ),
        ("energy".to_owned(), real(1000.0)),
    ];
    act(&mut sim, ada, "test_wall_punch", params);
    run_for(&mut sim, 10 * MS);
    let told = sim.drain_events();
    assert_eq!(faults(&told), Vec::<String>::new());
    let landed = named(&told, "strike_landed");
    assert_eq!(landed.len(), 1);
    assert_eq!(field(landed[0], "returned"), Real::of(1000.0));
    // The wall stands, undamaged.
    let damage = sim.truth().cosmos().damage(at).unwrap();
    assert_eq!(damage[Layer::Block.index()], 0);
    // The hand took the blow: its skin and muscle are breached, its
    // bone damaged.
    let hurt = layers(&sim, ada, &hand);
    assert!(hurt.iter().all(|damage| *damage > 0), "{hurt:?}");
    assert!(
        named(&told, "wound_inflicted")
            .iter()
            .any(|event| about(event, ada)),
        "the striker is wounded by its own blow"
    );
    // The same blow at a tenth of the strength hurts the knuckles and
    // not the bone.
    let (mut weak, people) = row(7, &["Ada"]);
    let ada = people[0];
    debug(
        &mut weak,
        DebugCommand::SetElement {
            loc: at,
            layer: Layer::Block,
            state: wall.raw(),
        },
    );
    let params = vec![
        (
            "at".to_owned(),
            weak.subject_value(Subject::Loc(at)).to_stored(),
        ),
        ("energy".to_owned(), real(80.0)),
    ];
    act(&mut weak, ada, "test_wall_punch", params);
    run_for(&mut weak, 10 * MS);
    let hurt = layers(&weak, ada, &hand);
    assert!(hurt[0] > 0 && hurt[2] == 0, "{hurt:?}");
}

#[test]
fn one_cut() {
    // A blade drawn once along a rank of six. Each unarmoured head takes
    // about 7 J of a knife's edge (06 §7.2, at the edge's contact) and
    // passes the rest; the sixth wears a visor of steel, which the 10 J
    // that reaches it cannot affect (800 MPa for a cut).
    let names = ["Ada", "Bram", "Cora", "Dov", "Esme", "Finn", "Gus"];
    let (mut sim, people) = row(11, &names);
    let swordsman = people[0];
    let rank = &people[1..];
    arm(&mut sim, swordsman, "core:knife");
    dress(&mut sim, rank[5], "core:visor");
    let first_head = part_value(&sim, rank[0], "head");
    sim.drain_events();
    let params = vec![
        ("target".to_owned(), entity_value(&sim, rank[0])),
        ("part".to_owned(), first_head),
        ("energy".to_owned(), real(46.0)),
    ];
    act(&mut sim, swordsman, "test_cleave", params);
    run_for(&mut sim, 10 * MS);
    let told = sim.drain_events();
    assert_eq!(faults(&told), Vec::<String>::new());
    let landed = named(&told, "strike_landed");
    assert_eq!(landed.len(), 6, "every one of the six is struck");
    for (at, who) in rank.iter().enumerate().take(5) {
        let hurt = layers(&sim, *who, "head");
        assert!(
            hurt.iter().all(|damage| *damage > 0),
            "the blade passed through head {at}: {hurt:?}"
        );
        assert!(field(landed[at], "passed") > Real::ZERO);
    }
    let sixth = landed[5];
    assert_eq!(field(sixth, "passed"), Real::ZERO, "it stops at the sixth");
    assert!(field(sixth, "returned") > Real::ZERO);
    assert_eq!(layers(&sim, rank[5], "head"), vec![0, 0, 0]);
    // Each takes what passed the one before.
    for pair in landed.windows(2) {
        assert!(
            (field(pair[1], "energy").get() - field(pair[0], "passed").get()).abs() < 1e-9,
            "{} then {}",
            field(pair[0], "passed"),
            field(pair[1], "energy")
        );
    }
    // The knife takes the recoil.
    assert_eq!(named(&told, "strike_recoiled").len(), 1);
}

#[test]
fn a_punch_to_an_unarmoured_head_wounds_and_the_blood_drains() {
    let (mut sim, ada, bram) = city(13);
    let head = part_value(&sim, bram, "head");
    let full = level(&sim, bram, "core:blood");
    sim.drain_events();
    let params = vec![
        ("target".to_owned(), entity_value(&sim, bram)),
        ("part".to_owned(), head),
    ];
    act(&mut sim, ada, "punch", params);
    run_for(&mut sim, 400 * MS);
    let told = sim.drain_events();
    assert_eq!(faults(&told), Vec::<String>::new());
    let landed = named(&told, "strike_landed");
    assert_eq!(landed.len(), 1);
    assert!(
        named(&told, "wound_inflicted")
            .iter()
            .any(|event| about(event, bram))
    );
    let hurt = layers(&sim, bram, "head");
    assert!(hurt[0] > 0 && hurt[1] > 0, "skin and muscle: {hurt:?}");
    // Bleeding is on the timeline: a minute later there is less blood.
    run_for(&mut sim, 60_000 * MS);
    assert!(level(&sim, bram, "core:blood") < full);
}

#[test]
fn the_same_punch_on_a_helmet_dents_the_helmet_and_wounds_less() {
    // A blow hard enough to affect steel (250 MPa for impact): a fist
    // with 25.8 kJ, from a very strong person, at full effort.
    let energy = 25_800.0;
    let mut wounds = Vec::new();
    for helmeted in [false, true] {
        let (mut sim, ada, bram) = city(17);
        let helmet = helmeted.then(|| dress(&mut sim, bram, "core:visor"));
        sim.drain_events();
        let tally = strike_with(
            &mut sim,
            ada,
            "test_fist",
            bram,
            Some("head"),
            &[("energy", Value::Real(Real::of(energy)))],
        );
        assert!(tally.landed);
        let told = sim.drain_events();
        assert_eq!(faults(&told), Vec::<String>::new());
        let body = sim.components().body.get(bram).unwrap();
        wounds.push(body.wounds().len());
        if let Some(helmet) = helmet {
            let item = sim
                .components()
                .get::<clash_sim::item::Item>(helmet)
                .unwrap();
            assert!(item.damage() > 0, "the helmet is dented");
        }
    }
    assert!(wounds[1] < wounds[0], "{wounds:?}");
}

/// The energy at which a cut from a knife's edge takes off an eye
/// (25 mm across, a finger's size), from the body model: the one layer
/// of organ under the edge's contact.
fn severing_energy(sim: &Sim) -> f64 {
    let defs = sim.defs();
    let eye = defs
        .lookup::<BodyPartDef>("core:eye")
        .map(|id| defs.get::<BodyPartDef>(id))
        .unwrap();
    let layer = &eye.layers[0];
    let organ = sim
        .truth()
        .cosmos()
        .defs()
        .materials()
        .get(layer.material.id())
        .unwrap()
        .toughness;
    let contact = 1.0e-5_f64.min((eye.length.get() * eye.girth.get()).get());
    organ.get() * contact * layer.thickness.get().get()
}

#[test]
fn a_slash_severs_a_finger_sized_part_only_at_the_energy_the_body_model_predicts() {
    let probe = row(19, &["Ada"]).0;
    let needed = severing_energy(&probe);
    assert!(needed > 0.0);
    for (energy, severs) in [(needed * 0.9, false), (needed * 1.1, true)] {
        let (mut sim, people) = row(19, &["Ada", "Bram"]);
        let (ada, bram) = (people[0], people[1]);
        arm(&mut sim, ada, "core:knife");
        sim.drain_events();
        strike_with(
            &mut sim,
            ada,
            "test_edge",
            bram,
            Some("l_eye"),
            &[("energy", Value::Real(Real::of(energy)))],
        );
        let told = sim.drain_events();
        assert_eq!(faults(&told), Vec::<String>::new());
        let state = sim
            .components()
            .body
            .get(bram)
            .unwrap()
            .part_named("l_eye")
            .unwrap()
            .state();
        assert_eq!(
            state.name() == "severed",
            severs,
            "{energy} J against {needed} J: {}",
            state.name()
        );
    }
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(48))]

    /// Invariant C2: what a strike delivered, pushed and returned, and
    /// what passed beyond, add up to no more than what was put in, and
    /// to all of it when it landed.
    #[test]
    fn energy_is_accounted_for(
        surface in 0usize..4,
        energy in 0.0f64..5000.0,
        push in 0.0f64..1.0,
        part in prop::sample::select(vec!["head", "torso", "l_arm", "r_hand", "l_leg", "l_eye"]),
        aimed in any::<bool>(),
        behind in any::<bool>(),
        seed in 0u64..1000,
    ) {
        let names: &[&str] = if behind { &["Ada", "Bram", "Cora"] } else { &["Ada", "Bram"] };
        let (mut sim, people) = row(seed, names);
        let (ada, bram) = (people[0], people[1]);
        arm(&mut sim, ada, "core:knife");
        let action = ["test_fist", "test_edge", "test_point", "test_ram"][surface];
        let tally = strike_with(
            &mut sim,
            ada,
            action,
            bram,
            aimed.then_some(part),
            &[
                ("energy", Value::Real(Real::of(energy))),
                ("push", Value::Real(Real::of(push))),
            ],
        );
        let tolerance = 1e-6 * (1.0 + energy);
        prop_assert!(tally.accounted().get() <= tally.put_in.get() + tolerance,
            "{tally:?}");
        if tally.landed {
            prop_assert!((tally.accounted().get() - tally.put_in.get()).abs() <= tolerance,
                "{tally:?}");
        }
        prop_assert!((tally.put_in.get() - energy).abs() <= 1e-9);
        prop_assert_eq!(faults(&sim.drain_events()), Vec::<String>::new());
    }
}

// ---- save, load and replay --------------------------------------------------

#[test]
fn a_save_in_the_middle_of_a_strike_changes_nothing() {
    // On flat ground: the test harness's round trip of the city's
    // columns does not keep the city's own hash.
    let (mut sim, people) = row(23, &["Ada", "Bram"]);
    let (ada, bram) = (people[0], people[1]);
    let head = part_value(&sim, bram, "head");
    let params = vec![
        ("target".to_owned(), entity_value(&sim, bram)),
        ("part".to_owned(), head),
    ];
    act(&mut sim, ada, "punch", params);
    // Halfway through the windup.
    run_for(&mut sim, 120 * MS);
    let (mut loaded, unplaced) = common::save_and_load(&sim);
    assert!(unplaced.is_empty());
    assert_eq!(loaded.state_hash(), sim.state_hash());
    for _ in 0..4 {
        run_for(&mut sim, 200 * MS);
        run_for(&mut loaded, 200 * MS);
        assert_eq!(loaded.state_hash(), sim.state_hash());
    }
    let told = sim.drain_events();
    assert_eq!(
        named(&told, "strike_landed").len() + named(&told, "strike_missed").len(),
        1,
        "the punch was resolved"
    );
}

#[test]
fn a_replay_reproduces_the_hash() {
    let (mut sim, ada, bram) = city(29);
    let head = part_value(&sim, bram, "head");
    let params = vec![
        ("target".to_owned(), entity_value(&sim, bram)),
        ("part".to_owned(), head),
    ];
    act(&mut sim, ada, "punch", params);
    run_for(&mut sim, 2_000 * MS);
    let params = vec![("target".to_owned(), entity_value(&sim, bram))];
    act(&mut sim, ada, "kick", params);
    run_for(&mut sim, 2_000 * MS);
    let told = sim.drain_events();
    assert_eq!(
        named(&told, "strike_landed").len() + named(&told, "strike_missed").len(),
        2
    );
    let defs = Arc::clone(sim.defs());
    let world = WorldDefs::from_defs(&defs).unwrap();
    let mut timeline = Timeline::new();
    let mut cosmos = Cosmos::new(Arc::clone(&world));
    let def = world.realms().lookup("core:test_city").unwrap();
    cosmos.create_realm(&mut timeline, def, CITY_SEED).unwrap();
    let mut replayed = Sim::from_parts(defs, cosmos, timeline, 29);
    replayed.replay(sim.input_log()).unwrap();
    let now = sim.now().as_micros() - replayed.now().as_micros();
    run_for(&mut replayed, now);
    assert_eq!(replayed.state_hash(), sim.state_hash());
}

// ---- the document states what is built --------------------------------------

#[test]
fn document_07_states_every_event_reason_setting_and_function() {
    let document = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../docs/07-combat.md");
    let text = std::fs::read_to_string(document).expect("document 07 can be read");
    let registry = DEFS.expr_registry();
    for info in clash_sim::combat::COMBAT_EVENTS {
        assert!(
            registry.event_id(info.name).is_some(),
            "`{}` is not registered",
            info.name
        );
        assert!(
            text.contains(&format!("| `{}`", info.name)),
            "07 §16.1 does not state the event `{}`",
            info.name
        );
        for (field, _, _) in info.fields {
            assert!(
                text.contains(&format!("`{field}`")),
                "07 does not state `{}.{field}`",
                info.name
            );
        }
    }
    for info in clash_sim::combat::COMBAT_REASONS {
        assert!(
            text.contains(&format!("`{}`", info.code)),
            "07 does not state the reason `{}`",
            info.code
        );
    }
    for (setting, doc) in clash_sim::combat::COMBAT_SETTINGS {
        assert!(!doc.is_empty());
        assert!(text.contains(&format!("`{setting}`")), "{setting}");
    }
    for function in clash_sim::combat::functions() {
        assert!(registry.function_id(function.name).is_some());
        assert!(
            text.contains(&format!("| `{}` |", function.name)),
            "07 §4.1 does not state the function `{}`",
            function.name
        );
        for arg in &function.args {
            assert!(text.contains(&format!("`{}: ", arg.name)), "{}", arg.name);
        }
    }
    for effect in clash_sim::rule::ACTION_EFFECTS {
        assert!(text.contains(&format!("`{}`", effect.name)));
    }
    assert!(text.contains("| `strike_resolving` (hook)"));
    assert!(text.contains("| `knockback_incoming` (hook)"));
}
