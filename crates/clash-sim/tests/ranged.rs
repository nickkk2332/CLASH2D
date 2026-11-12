//! Ranged combat against the real simulation (07 §11; plans/combat.md,
//! packages C9 and C10): people spawned from `core:person`, the
//! weapons, rounds and actions of `mods/core/data/combat`, on flat
//! ground.
//!
//! C9: `through_cover` (the same shot passes glass, slower, and is
//! stopped by a wall); a shot through a window breaks it and goes on;
//! a shot over a low wall hits only parts above it; an arrow sent along
//! `ballistic_dir` arrives and one sent flat falls short; a shot goes
//! through one person into the next; a near miss is told; an instant
//! shot strikes along its ray at once; a shot is heard by someone who
//! did not see it; a leg wound bleeds; save and load with a projectile
//! in flight and replay change nothing; `volley` (ignored: a timing).
//!
//! C10: `mixed_belt` (two kinds of round fire in the order loaded, each
//! with its own projectile and energy); an empty magazine makes the
//! shot unavailable with its reason; `count` above one fires that many;
//! a scope and a suppressor change spread and noise and stop when
//! taken off; a bow's energy follows the draw to its cap; reloading;
//! a thrown knife strikes with its point; a charge bursts on its fuse.
//!
//! `through_cover` and `mixed_belt` are also test scripts under
//! `scenarios/combat/`.

mod common;

use std::sync::{Arc, LazyLock};

use clash_core::{EntityId, Real, Tick};
use clash_data::Defs;
use clash_sim::action::{ActionDef, How, Params};
use clash_sim::combat::ranged::{self, Aim, ProjectileDef, ShotSpec, ThrowSpec, aim, store};
use clash_sim::perception;
use clash_sim::template::{CharacterSpec, Selection};
use clash_sim::{
    Advance, AdvanceLimit, Command, Control, DebugCommand, Place, Sim, SimEvent, Subject,
};
use clash_world::{Cause, Layer};

const MS: i64 = 1_000;

/// Definitions the tests add: a low wall of brick a metre high, and a
/// pulse that an instant shot casts.
const FIXTURES: &str = r#"[
  { "type": "block", "id": "low_wall", "material": "brick",
    "height": 1.0, "fill": 1.0, "flags": ["load_bearing"],
    "glyph": { "ch": "_", "fg": "material" } },
  { "type": "projectile", "id": "test_pulse", "mass": "1 g",
    "surface": { "kind": "heat", "area": 1.0e-4, "give": "1 mm" } },
  { "type": "action", "id": "test_kit",
    "phases": [ { "id": "arm", "time": 0, "do": [
      { "create_item": { "def": "core:rifle", "at": "self.loc" } },
      { "take": { "who": "self", "what": "self.loc.occupants[1]" } },
      { "create_item": { "def": "core:rifle_magazine", "at": "self.loc" } },
      { "fit": { "what": "self.loc.occupants[1]", "into": "self.wielding('firearm') ?? self" } },
      { "create_item": { "def": "core:rifle_round", "count": 5, "at": "self.loc" } },
      { "load": { "who": "self", "into": "self.wielding('firearm') ?? self", "from": "self.loc.occupants[1]" } } ] } ] }
]"#;

/// The JSON files of a directory of `mods/core/data`, in file-name
/// order.
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

/// `mods/core/data/items.json` without its colours.
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

static DEFS: LazyLock<Arc<Defs>> = LazyLock::new(|| {
    let mut owned = core_dir("life");
    owned.extend(core_dir("combat"));
    owned.extend(core_dir("perception"));
    let movement = common::mods_dir().join("core/data/movement.json");
    owned.push((
        "data/movement.json".to_owned(),
        std::fs::read_to_string(movement).unwrap(),
    ));
    owned.push(("data/items.json".to_owned(), items()));
    owned.push(("data/test/ranged.json".to_owned(), FIXTURES.to_owned()));
    let files: Vec<(&str, &str)> = owned
        .iter()
        .map(|(path, text)| (path.as_str(), text.as_str()))
        .collect();
    common::try_defs_with(&files).unwrap_or_else(|error| panic!("{error}"))
});

fn debug(sim: &mut Sim, command: DebugCommand) {
    sim.submit(Command::Debug(command))
        .unwrap_or_else(|error| panic!("{error}"));
}

fn person(name: &str) -> CharacterSpec {
    CharacterSpec::of(["core:person"]).with("core:person.name", Selection::Text(name.to_owned()))
}

/// Spawns a person by command, so that a replay makes them too.
fn spawn(sim: &mut Sim, name: &str, x: i32, y: i32) -> EntityId {
    let id = sim.truth().entities().next_id().unwrap();
    debug(
        sim,
        DebugCommand::Spawn {
            spec: person(name),
            place: common::on(x, y),
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

fn flat(seed: u64) -> Sim {
    common::sim_over(Arc::clone(&DEFS), seed)
}

fn set_block(sim: &mut Sim, x: i32, y: i32, block: &str) {
    let state = sim
        .truth()
        .cosmos()
        .defs()
        .block_state(block, None)
        .unwrap()
        .raw();
    debug(
        sim,
        DebugCommand::SetElement {
            loc: common::at(x, y),
            layer: Layer::Block,
            state,
        },
    );
}

/// Makes an item by command, out of the world.
fn make(sim: &mut Sim, def: &str, count: u32) -> EntityId {
    let item = sim.truth().entities().next_id().unwrap();
    debug(
        sim,
        DebugCommand::CreateItem {
            def: def.to_owned(),
            count,
            place: Place::Stored,
        },
    );
    item
}

fn hold(sim: &mut Sim, who: EntityId, def: &str) -> EntityId {
    let item = make(sim, def, 1);
    clash_sim::item::take(sim, who, item, Cause::DEBUG).unwrap();
    item
}

fn projectile(sim: &Sim, name: &str) -> ranged::ProjectileId {
    sim.defs()
        .lookup::<ProjectileDef>(&format!("core:{name}"))
        .unwrap_or_else(|| panic!("no projectile {name}"))
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

/// Prints what is told, with `RANGED_DEBUG` set.
fn dump(events: &[SimEvent]) {
    if std::env::var("RANGED_DEBUG").is_ok() {
        for event in events {
            eprintln!("{} {:?}", event.name(), event.body);
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

fn real(event: &SimEvent, name: &str) -> f64 {
    event
        .body
        .field(name)
        .and_then(|value| value.as_real())
        .unwrap_or_else(|| panic!("{} has no real `{name}`", event.name()))
        .get()
}

fn text(event: &SimEvent, name: &str) -> String {
    event
        .body
        .field(name)
        .and_then(|value| value.plain().and_then(|v| v.as_str().map(str::to_owned)))
        .unwrap_or_default()
}

fn about(event: &SimEvent, name: &str, who: EntityId) -> bool {
    event.subject(name) == Some(Subject::Entity(who))
}

/// A shot with no dispersion of a projectile at an energy, from
/// `shooter` toward `target` at `height` above its footing.
fn level_shot(
    sim: &Sim,
    shooter: EntityId,
    target: EntityId,
    name: &str,
    energy: f64,
    height: f64,
) -> ShotSpec {
    let aim = ranged::shoot::aim_at_entity(sim, target, None, Some(Real::of(height))).unwrap();
    let mut spec = ShotSpec::new(shooter, aim);
    spec.target = Some(target);
    spec.projectile = Some(projectile(sim, name));
    spec.energy = Some(Real::of(energy));
    spec.spread = Some(Real::ZERO);
    spec
}

fn fire(sim: &mut Sim, spec: &ShotSpec) -> ranged::Fired {
    let cause = Cause::DEBUG.by(spec.shooter);
    let fired = ranged::shoot(sim, spec, cause).unwrap_or_else(|reason| panic!("{reason}"));
    sim.pump();
    fired
}

fn wounds(sim: &Sim, who: EntityId) -> usize {
    sim.components()
        .body
        .get(who)
        .map_or(0, |body| body.wounds().len())
}

// ---- C9: projectiles ---------------------------------------------------------

#[test]
fn through_cover() {
    // A rifle bullet at 3.4 kJ through a window: the glass, 0.06 m³ of
    // it worth 3 kJ, is destroyed, and the bullet goes on with what was
    // left into the one behind. The same shot at a wall of concrete
    // (0.6 MJ) is stopped there, and the one behind is untouched.
    let mut sim = flat(3);
    let shooter = spawn(&mut sim, "Ada", 10, 10);
    let behind = spawn(&mut sim, "Bram", 15, 10);
    set_block(&mut sim, 12, 10, "window");
    sim.pump();
    sim.drain_events();
    let spec = level_shot(&sim, shooter, behind, "rifle_bullet", 3400.0, 1.3);
    let fired = fire(&mut sim, &spec);
    assert_eq!(fired.projectiles, 1);
    run_for(&mut sim, 200 * MS);
    let told = sim.drain_events();
    assert_eq!(faults(&told), Vec::<String>::new());
    let hits = named(&told, "projectile_hit");
    let glass = hits
        .iter()
        .find(|event| event.subject("target").is_none())
        .expect("the window was struck");
    assert!(real(glass, "passed") > 0.0, "it went through the glass");
    assert!(real(glass, "passed") < 3400.0, "slower");
    assert_eq!(
        named(&told, "element_destroyed").len(),
        1,
        "the window broke"
    );
    let person = hits
        .iter()
        .find(|event| about(event, "target", behind))
        .expect("it reached the one behind");
    assert!(real(person, "energy") < 3400.0);
    assert!(wounds(&sim, behind) > 0);

    // The wall.
    let mut sim = flat(3);
    let shooter = spawn(&mut sim, "Ada", 10, 10);
    let behind = spawn(&mut sim, "Bram", 15, 10);
    set_block(&mut sim, 12, 10, "wall");
    sim.pump();
    sim.drain_events();
    let spec = level_shot(&sim, shooter, behind, "rifle_bullet", 3400.0, 1.3);
    fire(&mut sim, &spec);
    run_for(&mut sim, 300 * MS);
    let told = sim.drain_events();
    assert_eq!(faults(&told), Vec::<String>::new());
    let hits = named(&told, "projectile_hit");
    assert_eq!(hits.len(), 1, "{hits:?}");
    assert_eq!(real(hits[0], "passed"), 0.0);
    assert!(named(&told, "element_destroyed").is_empty());
    assert_eq!(wounds(&sim, behind), 0);
    assert_eq!(sim.truth().cosmos().bodies().count(), 0, "it ended there");
}

#[test]
fn a_shot_through_a_person_flies_on_into_the_next() {
    let mut sim = flat(5);
    let shooter = spawn(&mut sim, "Ada", 10, 10);
    let first = spawn(&mut sim, "Bram", 13, 10);
    let second = spawn(&mut sim, "Cora", 14, 10);
    sim.pump();
    sim.drain_events();
    let spec = level_shot(&sim, shooter, first, "rifle_bullet", 3400.0, 1.2);
    fire(&mut sim, &spec);
    run_for(&mut sim, 200 * MS);
    let told = sim.drain_events();
    assert_eq!(faults(&told), Vec::<String>::new());
    let hits = named(&told, "projectile_hit");
    let one = hits
        .iter()
        .find(|event| about(event, "target", first))
        .unwrap();
    let two = hits
        .iter()
        .find(|event| about(event, "target", second))
        .unwrap();
    assert!(real(one, "passed") > 0.0);
    // What the second meets is what passed the first, less nothing on
    // the way but air.
    assert!(real(two, "energy") <= real(one, "passed") + 1e-6);
    assert!(real(two, "energy") > real(one, "passed") * 0.9);
    assert!(wounds(&sim, first) > 0 && wounds(&sim, second) > 0);
}

#[test]
fn a_shot_over_a_low_wall_hits_only_what_is_above_it() {
    // A wall of brick a metre high stands just before the target. Shots
    // aimed from its foot to its head: what is aimed low is stopped by
    // the wall, and every part struck has a band reaching above it.
    let wall_height = 1.0;
    let mut stopped = 0;
    let mut struck = 0;
    for step in 0..18 {
        let height = 0.1 + 0.1 * f64::from(step);
        let mut sim = flat(7);
        let shooter = spawn(&mut sim, "Ada", 10, 10);
        let target = spawn(&mut sim, "Bram", 14, 10);
        set_block(&mut sim, 13, 10, "low_wall");
        sim.pump();
        sim.drain_events();
        let spec = level_shot(&sim, shooter, target, "pistol_bullet", 500.0, height);
        fire(&mut sim, &spec);
        run_for(&mut sim, 200 * MS);
        let told = sim.drain_events();
        assert_eq!(faults(&told), Vec::<String>::new());
        for event in named(&told, "projectile_hit") {
            if !about(event, "target", target) {
                stopped += 1;
                continue;
            }
            struck += 1;
            let part = event
                .body
                .field("part")
                .and_then(|value| part_of(&sim, &value))
                .expect("a part was struck");
            let body = sim.components().body.get(target).unwrap();
            let defs = sim.body_defs().unwrap();
            let band = body.part_band(defs, body.part(part).unwrap()).unwrap();
            assert!(
                band[1].get() > wall_height,
                "aimed at {height} m it struck a part whose band is {band:?}"
            );
        }
    }
    assert!(stopped > 0, "low shots meet the wall");
    assert!(struck > 0, "high shots pass over it");
}

fn part_of(sim: &Sim, field: &clash_sim::bus::FieldValue) -> Option<clash_sim::body::PartKey> {
    let clash_core::Value::Subject { kind, payload } = field.plain()? else {
        return None;
    };
    let handle = clash_expr::Handle::new(clash_expr::KindId::new(*kind), *payload);
    clash_sim::body::props::part_of_handle(&sim.components().body, handle).map(|(_, key)| key)
}

#[test]
fn an_arrow_sent_along_its_arc_arrives_and_one_sent_flat_falls_short() {
    // Sixty metres at 60 J: about 69 m/s. Flat from the shoulder it is on
    // the ground within forty metres.
    let lob = |flat_shot: bool| {
        let mut sim = flat(9);
        let archer = spawn(&mut sim, "Ada", 2, 10);
        let target = spawn(&mut sim, "Bram", 62, 10);
        sim.pump();
        sim.drain_events();
        let mut spec = level_shot(&sim, archer, target, "arrow", 60.0, 1.2);
        if flat_shot {
            spec.aim = Aim::Dir([Real::ONE, Real::ZERO, Real::ZERO]);
        } else {
            spec.lob = true;
        }
        fire(&mut sim, &spec);
        run_for(&mut sim, 3_000 * MS);
        let told = sim.drain_events();
        assert_eq!(faults(&told), Vec::<String>::new());
        let arrived = named(&told, "projectile_hit")
            .iter()
            .any(|event| about(event, "target", target));
        let landed_x = named(&told, "body_landed")
            .first()
            .map(|event| format!("{event:?}"));
        if std::env::var("RANGED_DEBUG").is_ok() {
            for event in &told {
                if event.name().starts_with("body") || event.name().starts_with("projectile") {
                    eprintln!("{event:?}");
                }
            }
        }
        (arrived, landed_x)
    };
    let (arrived, _) = lob(false);
    assert!(arrived, "along ballistic_dir it arrives");
    let (arrived, landed) = lob(true);
    assert!(!arrived, "flat it falls short: {landed:?}");
}

#[test]
fn ballistic_dir_and_band_point_answer_rules() {
    let mut sim = flat(9);
    let archer = spawn(&mut sim, "Ada", 2, 10);
    let target = spawn(&mut sim, "Bram", 30, 10);
    sim.pump();
    let point = aim::band_point(&sim, target, None).unwrap().1;
    // The middle of a body standing on the ground.
    assert!(point[2].get() > 0.5 && point[2].get() < 1.2, "{point:?}");
    let from = aim::shot_origin(&sim, archer, None).unwrap();
    let to = aim::band_point(&sim, target, None).unwrap();
    let way = aim::between(&sim, from, to).unwrap();
    let dir = aim::ballistic_dir(way, Real::of(60.0), Real::of(9.81)).unwrap();
    assert!(dir[2] > Real::ZERO, "it rises");
}

#[test]
fn a_near_miss_is_told_to_whoever_it_passed() {
    let mut sim = flat(11);
    let shooter = spawn(&mut sim, "Ada", 10, 10);
    let target = spawn(&mut sim, "Bram", 20, 10);
    let beside = spawn(&mut sim, "Cora", 15, 11);
    let far = spawn(&mut sim, "Dov", 15, 20);
    sim.pump();
    sim.drain_events();
    let spec = level_shot(&sim, shooter, target, "pistol_bullet", 500.0, 1.3);
    fire(&mut sim, &spec);
    run_for(&mut sim, 200 * MS);
    let told = sim.drain_events();
    let passed = named(&told, "projectile_passed");
    assert!(passed.iter().any(|event| about(event, "self", beside)));
    assert!(!passed.iter().any(|event| about(event, "self", far)));
    assert!(!passed.iter().any(|event| about(event, "self", target)));
    assert!(passed.iter().all(|event| about(event, "shooter", shooter)));
}

#[test]
fn an_instant_shot_strikes_along_its_ray_at_once() {
    let mut sim = flat(13);
    let shooter = spawn(&mut sim, "Ada", 10, 10);
    let target = spawn(&mut sim, "Bram", 14, 10);
    sim.pump();
    sim.drain_events();
    let mut spec = level_shot(&sim, shooter, target, "test_pulse", 2000.0, 1.2);
    spec.instant = true;
    let fired = fire(&mut sim, &spec);
    assert!(fired.bodies.is_empty(), "nothing flies");
    assert!(fired.struck > 0);
    let told = sim.drain_events();
    let hit = named(&told, "projectile_hit");
    assert!(hit.iter().any(|event| about(event, "target", target)));
    assert!(hit.iter().all(|event| {
        event
            .body
            .field("instant")
            .and_then(|v| v.plain().and_then(|v| v.as_bool()))
            == Some(true)
    }));
    assert!(wounds(&sim, target) > 0);
}

#[test]
fn a_shot_is_heard_by_someone_who_does_not_see_it() {
    let mut sim = flat(15);
    let shooter = spawn(&mut sim, "Ada", 10, 10);
    let target = spawn(&mut sim, "Bram", 10, 14);
    let listener = spawn(&mut sim, "Cora", 30, 10);
    for sense in ["human_sight", "human_hearing"] {
        let id = sim.perception_defs().sense_named(sense).unwrap();
        perception::attach_sense(&mut sim, listener, id).unwrap();
    }
    // Facing away, behind a wall: out of sight.
    perception::set_facing(&mut sim, listener, Real::of(90.0)).unwrap();
    for y in 7..14 {
        set_block(&mut sim, 25, y, "wall");
    }
    let pistol = hold(&mut sim, shooter, "core:pistol");
    let magazine = make(&mut sim, "core:pistol_magazine", 1);
    store::fit(&mut sim, magazine, pistol, Cause::DEBUG).unwrap();
    let rounds = make(&mut sim, "core:pistol_round", 5);
    store::load(&mut sim, shooter, pistol, Some(rounds), None, Cause::DEBUG).unwrap();
    run_for(&mut sim, 1_000 * MS);
    assert!(!perception::perceives(&sim, listener, shooter));
    sim.drain_events();
    let aim = ranged::shoot::aim_at_entity(&sim, target, None, None).unwrap();
    let mut spec = ShotSpec::new(shooter, aim);
    spec.store = Some(pistol);
    spec.weapon = Some(pistol);
    fire(&mut sim, &spec);
    run_for(&mut sim, 500 * MS);
    let told = sim.drain_events();
    let fired = named(&told, "projectile_fired");
    assert_eq!(fired.len(), 1);
    assert_eq!(real(fired[0], "noise"), 140.0);
    let heard = named(&told, "perceived").into_iter().any(|event| {
        about(event, "self", listener)
            && event.body.field("event").and_then(|v| v.plain().cloned())
                == Some(clash_core::Value::from("projectile_fired"))
    });
    assert!(heard, "the shot was heard");
}

#[test]
fn a_shot_in_the_leg_wounds_and_the_blood_drains() {
    let mut sim = flat(17);
    let shooter = spawn(&mut sim, "Ada", 10, 10);
    let target = spawn(&mut sim, "Bram", 14, 10);
    sim.pump();
    sim.drain_events();
    let spec = level_shot(&sim, shooter, target, "pistol_bullet", 500.0, 0.3);
    fire(&mut sim, &spec);
    run_for(&mut sim, 100 * MS);
    let told = sim.drain_events();
    dump(&told);
    let hit = named(&told, "projectile_hit")
        .into_iter()
        .find(|event| about(event, "target", target))
        .expect("the leg was struck");
    let part = hit
        .body
        .field("part")
        .and_then(|value| part_of(&sim, &value))
        .unwrap();
    let name = sim
        .components()
        .body
        .get(target)
        .unwrap()
        .part(part)
        .unwrap()
        .name()
        .to_owned();
    assert!(name.contains("leg") || name.contains("foot"), "{name}");
    assert!(wounds(&sim, target) > 0);
    let pool = sim.stat_defs().unwrap().pool_id("core:blood").unwrap();
    let blood = |sim: &Sim| {
        sim.components()
            .stats
            .get(target)
            .unwrap()
            .pool_level(sim.timeline(), pool)
            .unwrap()
            .get()
    };
    let before = blood(&sim);
    run_for(&mut sim, 60_000 * MS);
    assert!(blood(&sim) < before, "the blood drains");
}

#[test]
fn save_and_load_with_a_projectile_in_flight_change_nothing() {
    let mut sim = flat(19);
    let archer = spawn(&mut sim, "Ada", 2, 10);
    let target = spawn(&mut sim, "Bram", 40, 10);
    sim.pump();
    let mut spec = level_shot(&sim, archer, target, "arrow", 60.0, 1.2);
    spec.lob = true;
    fire(&mut sim, &spec);
    run_for(&mut sim, 150 * MS);
    assert_eq!(
        sim.truth().cosmos().bodies().count(),
        1,
        "the arrow is in flight"
    );
    let (mut loaded, unplaced) = common::save_and_load(&sim);
    assert!(unplaced.is_empty());
    assert_eq!(loaded.state_hash(), sim.state_hash());
    for _ in 0..10 {
        run_for(&mut sim, 100 * MS);
        run_for(&mut loaded, 100 * MS);
        assert_eq!(loaded.state_hash(), sim.state_hash());
    }
    assert_eq!(sim.truth().cosmos().bodies().count(), 0, "it has come down");
}

// ---- C10: ranged weapons -------------------------------------------------------

/// A rifle in hand with a magazine fitted, loaded from stacks of rounds
/// in the order given.
fn rifle_loaded(sim: &mut Sim, who: EntityId, loads: &[(&str, u32)]) -> EntityId {
    let rifle = hold(sim, who, "core:rifle");
    let magazine = make(sim, "core:rifle_magazine", 1);
    store::fit(sim, magazine, rifle, Cause::DEBUG).unwrap();
    for (def, count) in loads {
        let rounds = make(sim, def, *count);
        let loaded = store::load(sim, who, rifle, Some(rounds), None, Cause::DEBUG).unwrap();
        assert_eq!(loaded, *count);
    }
    rifle
}

fn act(sim: &mut Sim, who: EntityId, name: &str, params: Params) -> Result<(), String> {
    let action = sim
        .defs()
        .lookup::<ActionDef>(&format!("core:{name}"))
        .unwrap_or_else(|| panic!("no action {name}"));
    sim.submit(Command::Act {
        entity: who,
        action,
        params,
        how: How::Start,
    })
    .map(|_| ())
    .map_err(|error| error.to_string())
}

fn at_param(sim: &Sim, target: EntityId) -> Params {
    let loc = sim.truth().place(target).unwrap().loc().unwrap();
    vec![
        (
            "target".to_owned(),
            sim.subject_value(Subject::Entity(target)).to_stored(),
        ),
        (
            "at".to_owned(),
            sim.subject_value(Subject::Loc(loc)).to_stored(),
        ),
    ]
}

#[test]
fn mixed_belt() {
    // Three ball, two tracer, one ball: fired in that order, each with
    // its own projectile and energy.
    let mut sim = flat(21);
    let shooter = spawn(&mut sim, "Ada", 10, 10);
    let target = spawn(&mut sim, "Bram", 30, 10);
    let rifle = rifle_loaded(
        &mut sim,
        shooter,
        &[
            ("core:rifle_round", 3),
            ("core:rifle_round_tracer", 2),
            ("core:rifle_round", 1),
        ],
    );
    assert_eq!(
        store::rounds_in(&sim, store::feed_of(&sim, rifle).unwrap()),
        6
    );
    sim.pump();
    sim.drain_events();
    let mut params = at_param(&sim, target);
    params.push(("count".to_owned(), clash_core::Value::Int(6)));
    act(&mut sim, shooter, "shoot", params).unwrap();
    run_for(&mut sim, 2_000 * MS);
    let told = sim.drain_events();
    assert_eq!(faults(&told), Vec::<String>::new());
    let fired: Vec<(String, f64)> = named(&told, "projectile_fired")
        .iter()
        .map(|event| (text(event, "projectile"), real(event, "energy")))
        .collect();
    let expected = [
        ("core:rifle_bullet", 3400.0),
        ("core:rifle_bullet", 3400.0),
        ("core:rifle_bullet", 3400.0),
        ("core:tracer_bullet", 3200.0),
        ("core:tracer_bullet", 3200.0),
        ("core:rifle_bullet", 3400.0),
    ];
    assert_eq!(fired.len(), expected.len(), "{fired:?}");
    for ((name, energy), (want, want_energy)) in fired.iter().zip(expected) {
        assert_eq!(name, want);
        assert!((energy - want_energy).abs() < 1e-6, "{energy}");
    }
    // The magazine is empty, and six casings lie where the shooter is.
    assert_eq!(
        store::rounds_in(&sim, store::feed_of(&sim, rifle).unwrap()),
        0
    );
    let casings: u32 = sim
        .truth()
        .cosmos()
        .occupants_at(common::at(10, 10), &clash_world::OccupantFilter::ANY)
        .unwrap()
        .iter()
        .filter_map(|occupied| sim.components().get::<clash_sim::item::Item>(occupied.id))
        .map(clash_sim::item::Item::count)
        .sum();
    assert_eq!(casings, 6);
}

#[test]
fn an_empty_magazine_makes_the_shot_unavailable_with_its_reason() {
    let mut sim = flat(23);
    let shooter = spawn(&mut sim, "Ada", 10, 10);
    let target = spawn(&mut sim, "Bram", 20, 10);
    let rifle = rifle_loaded(&mut sim, shooter, &[]);
    sim.pump();
    let params = at_param(&sim, target);
    let refused = act(&mut sim, shooter, "shoot", params).unwrap_err();
    assert!(refused.contains("no loaded weapon"), "{refused}");
    // The effect itself refuses an empty store with its own reason.
    let aim = ranged::shoot::aim_at_entity(&sim, target, None, None).unwrap();
    let mut spec = ShotSpec::new(shooter, aim);
    spec.store = Some(rifle);
    let reason = ranged::shoot(&mut sim, &spec, Cause::DEBUG).unwrap_err();
    assert_eq!(reason.code, "ranged.empty");
    // Loaded, it is available.
    let rounds = make(&mut sim, "core:rifle_round", 2);
    store::load(&mut sim, shooter, rifle, Some(rounds), None, Cause::DEBUG).unwrap();
    let params = at_param(&sim, target);
    act(&mut sim, shooter, "shoot", params).unwrap();
}

#[test]
fn a_count_above_one_fires_that_many() {
    let mut sim = flat(25);
    let shooter = spawn(&mut sim, "Ada", 10, 10);
    let target = spawn(&mut sim, "Bram", 30, 10);
    let rifle = rifle_loaded(&mut sim, shooter, &[("core:rifle_round", 10)]);
    sim.pump();
    sim.drain_events();
    let aim = ranged::shoot::aim_at_entity(&sim, target, None, None).unwrap();
    let mut spec = ShotSpec::new(shooter, aim);
    spec.store = Some(rifle);
    spec.weapon = Some(rifle);
    spec.count = 4;
    let fired = fire(&mut sim, &spec);
    assert_eq!(fired.projectiles, 4);
    assert_eq!(fired.rounds, 4);
    assert_eq!(fired.bodies.len(), 4);
    assert_eq!(
        store::rounds_in(&sim, store::feed_of(&sim, rifle).unwrap()),
        6
    );
    // More than there is fires what there is.
    spec.count = 9;
    let fired = fire(&mut sim, &spec);
    assert_eq!(fired.rounds, 6);
}

#[test]
fn a_fitted_scope_and_suppressor_change_spread_and_noise_until_taken_off() {
    let mut sim = flat(27);
    let shooter = spawn(&mut sim, "Ada", 10, 10);
    let target = spawn(&mut sim, "Bram", 30, 10);
    let rifle = rifle_loaded(&mut sim, shooter, &[("core:rifle_round", 10)]);
    let bare_spread = store::shot_number(&sim, rifle, "spread").unwrap();
    let scope = make(&mut sim, "core:scope", 1);
    let suppressor = make(&mut sim, "core:suppressor", 1);
    store::fit(&mut sim, scope, rifle, Cause::DEBUG).unwrap();
    store::fit(&mut sim, suppressor, rifle, Cause::DEBUG).unwrap();
    let spread = store::shot_number(&sim, rifle, "spread").unwrap();
    assert!((spread.get() - bare_spread.get() * 0.3).abs() < 1e-12);
    // A second sight does not fit where one is.
    let other = make(&mut sim, "core:scope", 1);
    let refused = store::fit(&mut sim, other, rifle, Cause::DEBUG).unwrap_err();
    assert_eq!(refused.code, "ranged.cannot_fit");
    sim.pump();
    sim.drain_events();
    let noise_of_a_shot = |sim: &mut Sim| {
        let aim = ranged::shoot::aim_at_entity(sim, target, None, None).unwrap();
        let mut spec = ShotSpec::new(shooter, aim);
        spec.store = Some(rifle);
        spec.weapon = Some(rifle);
        fire(sim, &spec);
        let told = sim.drain_events();
        real(named(&told, "projectile_fired")[0], "noise")
    };
    let quiet = noise_of_a_shot(&mut sim);
    assert!((quiet - 160.0 * 0.15).abs() < 1e-9, "{quiet}");
    // Taken off into a hand, they give nothing.
    let free = spawn(&mut sim, "Cora", 11, 11);
    clash_sim::item::take(&mut sim, free, scope, Cause::DEBUG).unwrap();
    clash_sim::item::take(&mut sim, free, suppressor, Cause::DEBUG).unwrap();
    assert_eq!(store::shot_number(&sim, rifle, "spread"), Some(bare_spread));
    let loud = noise_of_a_shot(&mut sim);
    assert!((loud - 160.0).abs() < 1e-9, "{loud}");
    // The dispersion drawn follows the spread: over many shots the
    // scoped rifle's directions scatter less.
    let scatter = |sim: &mut Sim, spread: Real| {
        let mut sum = 0.0;
        for _ in 0..200 {
            let dir = aim::disperse(
                [Real::ONE, Real::ZERO, Real::ZERO],
                spread,
                sim.rng.stream(ranged::SHOT_STREAM),
            );
            sum += dir[1].get().abs();
        }
        sum
    };
    let wide = scatter(&mut sim, bare_spread);
    let narrow = scatter(&mut sim, spread);
    assert!(narrow < wide * 0.5, "{narrow} {wide}");
}

#[test]
fn a_bows_energy_follows_the_draw_up_to_its_cap() {
    let energy_after = |draw_ms: i64| {
        let mut sim = flat(29);
        let archer = spawn(&mut sim, "Ada", 10, 10);
        let target = spawn(&mut sim, "Bram", 30, 10);
        hold(&mut sim, archer, "core:bow");
        let quiver = make(&mut sim, "core:quiver", 1);
        clash_sim::item::wear(&mut sim, archer, quiver, Cause::DEBUG).unwrap();
        let arrows = make(&mut sim, "core:arrow", 10);
        clash_sim::item::put_in(&mut sim, arrows, quiver, Cause::DEBUG).unwrap();
        sim.pump();
        sim.drain_events();
        let params = at_param(&sim, target);
        act(&mut sim, archer, "draw_and_loose", params).unwrap();
        // Nock, then draw.
        run_for(&mut sim, 400 * MS + draw_ms * MS);
        if let Some(id) = sim.actions_of(archer).first().copied() {
            let _ = sim.submit(Command::Cancel {
                entity: archer,
                action: id,
                abandon: false,
            });
        }
        run_for(&mut sim, 1_000 * MS);
        let told = sim.drain_events();
        assert_eq!(faults(&told), Vec::<String>::new());
        let fired = named(&told, "projectile_fired");
        assert_eq!(
            fired.len(),
            1,
            "{:?}",
            told.iter().map(SimEvent::name).collect::<Vec<_>>()
        );
        assert_eq!(text(fired[0], "projectile"), "core:arrow");
        real(fired[0], "energy")
    };
    // A strength of 10 draws 100 J a second; the bow holds 80 J.
    let short = energy_after(300);
    let longer = energy_after(600);
    let full = energy_after(1_400);
    assert!(short > 0.0 && longer > short, "{short} {longer}");
    assert!((longer - 2.0 * short).abs() < 10.0, "{short} {longer}");
    assert!((full - 80.0).abs() < 1e-9, "{full}");
}

#[test]
fn reloading_takes_rounds_from_what_the_shooter_carries() {
    let mut sim = flat(31);
    let shooter = spawn(&mut sim, "Ada", 10, 10);
    let rifle = rifle_loaded(&mut sim, shooter, &[]);
    let pack = make(&mut sim, "core:backpack", 1);
    clash_sim::item::wear(&mut sim, shooter, pack, Cause::DEBUG).unwrap();
    let rounds = make(&mut sim, "core:rifle_round", 40);
    clash_sim::item::put_in(&mut sim, rounds, pack, Cause::DEBUG).unwrap();
    sim.pump();
    act(&mut sim, shooter, "reload", Vec::new()).unwrap();
    run_for(&mut sim, 3_000 * MS);
    let told = sim.drain_events();
    assert_eq!(faults(&told), Vec::<String>::new());
    // The magazine holds thirty; ten are left in the pack.
    assert_eq!(
        store::rounds_in(&sim, store::feed_of(&sim, rifle).unwrap()),
        30
    );
    assert_eq!(
        sim.components()
            .get::<clash_sim::item::Item>(rounds)
            .map(clash_sim::item::Item::count),
        Some(10)
    );
}

#[test]
fn a_thrown_knife_strikes_with_its_point() {
    let mut sim = flat(33);
    let thrower = spawn(&mut sim, "Ada", 10, 10);
    let target = spawn(&mut sim, "Bram", 14, 10);
    let knife = hold(&mut sim, thrower, "core:throwing_knife");
    sim.pump();
    sim.drain_events();
    let aim = ranged::shoot::aim_at_entity(&sim, target, None, Some(Real::of(1.2))).unwrap();
    let spec = ThrowSpec {
        thrower,
        what: knife,
        aim,
        target: Some(target),
        energy: Some(Real::of(40.0)),
        speed: None,
        spread: Real::ZERO,
        surface: Some("point".to_owned()),
        lob: true,
    };
    ranged::throw(&mut sim, &spec, Cause::DEBUG.by(thrower)).unwrap();
    run_for(&mut sim, 8_000 * MS);
    let told = sim.drain_events();
    dump(&told);
    assert_eq!(faults(&told), Vec::<String>::new());
    assert_eq!(named(&told, "thrown").len(), 1);
    let hit = named(&told, "projectile_hit")
        .into_iter()
        .find(|event| about(event, "target", target))
        .expect("the knife struck");
    // The point's contact: 40 J over 2e-6 m² × 2 mm.
    assert!(real(hit, "intensity") > 1.0e9);
    assert!(wounds(&sim, target) > 0);
    // It lies on the ground near the target, an item again.
    assert!(sim.truth().cosmos().body_of(knife).is_none());
}

#[test]
fn a_thrown_charge_bursts_on_its_fuse() {
    let mut sim = flat(35);
    let thrower = spawn(&mut sim, "Ada", 10, 10);
    let target = spawn(&mut sim, "Bram", 18, 10);
    let charge = hold(&mut sim, thrower, "core:charge");
    sim.pump();
    sim.drain_events();
    let params = at_param(&sim, target);
    act(&mut sim, thrower, "throw", params).unwrap();
    run_for(&mut sim, 1_500 * MS);
    let told = sim.drain_events();
    dump(&told);
    assert_eq!(named(&told, "thrown").len(), 1);
    assert!(named(&told, "release_started").is_empty(), "not yet");
    run_for(&mut sim, 3_000 * MS);
    let told = sim.drain_events();
    dump(&told);
    assert_eq!(named(&told, "release_started").len(), 1, "it burst");
    assert!(sim.truth().entity(charge).is_none(), "and is gone");
}

#[test]
fn replay_reproduces_the_hash() {
    // Everything by command: two people, a kit taken up by an action,
    // and a burst from the bundled `shoot`.
    let mut sim = flat(37);
    let shooter = spawn(&mut sim, "Ada", 10, 10);
    let target = spawn(&mut sim, "Bram", 20, 10);
    act(&mut sim, shooter, "test_kit", Vec::new()).unwrap();
    run_for(&mut sim, 100 * MS);
    let mut params = at_param(&sim, target);
    params.push(("count".to_owned(), clash_core::Value::Int(3)));
    act(&mut sim, shooter, "shoot", params).unwrap();
    run_for(&mut sim, 1_000 * MS);
    let told = sim.drain_events();
    assert_eq!(faults(&told), Vec::<String>::new());
    assert_eq!(named(&told, "projectile_fired").len(), 3);
    let mut replayed = flat(37);
    replayed.replay(sim.input_log()).unwrap();
    let now = sim.now().as_micros() - replayed.now().as_micros();
    run_for(&mut replayed, now);
    assert_eq!(replayed.state_hash(), sim.state_hash());
}

/// A simulation over flat ground four tracts long (128 cells east) and
/// two wide.
fn long_flat(seed: u64) -> Sim {
    use clash_world::test_support::load_flat;
    use clash_world::{Cosmos, TractPos, WorldDefs};
    let defs = Arc::clone(&DEFS);
    let world = WorldDefs::from_defs(&defs).unwrap_or_else(|d| panic!("{}", d.render()));
    let mut timeline = clash_time::Timeline::new();
    let mut cosmos = Cosmos::new(Arc::clone(&world));
    let realm_def = world.realms().lookup("core:testland").unwrap();
    cosmos.create_realm(&mut timeline, realm_def, 7).unwrap();
    let rock = world.block_state("solid", None).unwrap();
    let grass = world.floor_state("grass", None).unwrap();
    let tracts = [0, 1]
        .into_iter()
        .flat_map(|y| (0..4).map(move |x| TractPos::new(x, y)));
    load_flat(&mut cosmos, common::GRID, tracts, rock, grass, 0).unwrap();
    Sim::from_parts(defs, cosmos, timeline, seed)
}

#[test]
#[ignore = "a timing: run with --release --ignored"]
fn volley() {
    // Five hundred projectiles over a hundred metres within 40 ms (07 §18):
    // fifty shooters in a file each fire ten pistol bullets east at a
    // wall a hundred metres off, which stops every one.
    let mut sim = long_flat(39);
    let mut shooters = Vec::new();
    for y in 0..50 {
        shooters.push(spawn(&mut sim, "Ada", 2, 5 + y));
        set_block(&mut sim, 103, 5 + y, "wall");
    }
    sim.pump();
    sim.drain_events();
    let pistol = projectile(&sim, "pistol_bullet");
    let started = std::time::Instant::now();
    for shooter in &shooters {
        let mut spec = ShotSpec::new(*shooter, Aim::Dir([Real::ONE, Real::ZERO, Real::ZERO]));
        spec.projectile = Some(pistol);
        spec.energy = Some(Real::of(500.0));
        spec.spread = Some(Real::of(0.002));
        spec.count = 10;
        ranged::shoot(&mut sim, &spec, Cause::DEBUG).unwrap();
    }
    run_for(&mut sim, 400 * MS);
    let took = started.elapsed();
    assert_eq!(
        sim.truth().cosmos().bodies().count(),
        0,
        "all have struck the wall"
    );
    let told = sim.drain_events();
    assert_eq!(named(&told, "projectile_fired").len(), 500);
    eprintln!("volley of 500 over 100 m: {took:?}");
    assert!(took.as_millis() < 40, "{took:?}");
}

#[test]
fn the_properties_of_ranged_combat_read_stores_weapons_and_what_is_fitted() {
    let mut sim = flat(41);
    let shooter = spawn(&mut sim, "Ada", 10, 10);
    let rifle = rifle_loaded(
        &mut sim,
        shooter,
        &[("core:rifle_round", 3), ("core:rifle_round_tracer", 1)],
    );
    let scope = make(&mut sim, "core:scope", 1);
    store::fit(&mut sim, scope, rifle, Cause::DEBUG).unwrap();
    let quiver = make(&mut sim, "core:quiver", 1);
    clash_sim::item::wear(&mut sim, shooter, quiver, Cause::DEBUG).unwrap();
    let prop = |sim: &Sim, id: EntityId, name: &str, args: &[clash_expr::Value]| {
        sim.property_named(Subject::Entity(id), name, args)
    };
    let entity = |sim: &Sim, value: &clash_expr::Value| match sim.subject_of(value) {
        Some(Subject::Entity(id)) => Some(id),
        _ => None,
    };
    let tag = |text: &str| clash_expr::Value::from(text);
    assert_eq!(
        entity(&sim, &prop(&sim, shooter, "wielding", &[tag("firearm")])),
        Some(rifle)
    );
    assert_eq!(
        entity(&sim, &prop(&sim, shooter, "wielding", &[tag("bow")])),
        None
    );
    let feed = entity(&sim, &prop(&sim, rifle, "feed", &[])).unwrap();
    assert_eq!(store::feed_of(&sim, rifle), Some(feed));
    assert_eq!(prop(&sim, rifle, "rounds", &[]), clash_expr::Value::Int(4));
    let next = entity(&sim, &prop(&sim, rifle, "next_round", &[])).unwrap();
    assert_eq!(
        store::round_projectile(&sim, next),
        Some(projectile(&sim, "rifle_bullet"))
    );
    assert_eq!(
        entity(&sim, &prop(&sim, shooter, "store_of", &[tag("arrow")])),
        Some(quiver)
    );
    assert_eq!(
        entity(
            &sim,
            &prop(&sim, shooter, "store_of", &[tag("rifle_round")])
        ),
        Some(feed)
    );
    let spread = prop(&sim, rifle, "shot", &[tag("spread")])
        .as_number()
        .unwrap();
    assert!((spread.get() - 0.0015 * 0.3).abs() < 1e-12);
    assert_eq!(
        prop(&sim, rifle, "shot", &[tag("energy")]),
        clash_expr::Value::Null
    );
}

// ---- the document states what is built --------------------------------------

#[test]
fn document_07_states_every_ranged_event_reason_setting_function_and_verb() {
    let document = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../docs/07-combat.md");
    let text = std::fs::read_to_string(document).expect("document 07 can be read");
    let registry = DEFS.expr_registry();
    for info in ranged::RANGED_EVENTS {
        assert!(registry.event_id(info.name).is_some(), "`{}`", info.name);
        assert!(
            text.contains(&format!("| `{}`", info.name)),
            "07 does not state the event `{}`",
            info.name
        );
        for (field, _, _) in info.fields {
            assert!(
                text.contains(&format!("`{field}`")),
                "{}.{field}",
                info.name
            );
        }
    }
    for info in ranged::RANGED_REASONS {
        assert!(
            text.contains(&format!("`{}`", info.code)),
            "07 does not state the reason `{}`",
            info.code
        );
    }
    for (setting, doc) in ranged::RANGED_SETTINGS {
        assert!(!doc.is_empty());
        assert!(text.contains(&format!("`{setting}`")), "{setting}");
    }
    for function in ranged::functions() {
        assert!(registry.function_id(function.name).is_some());
        assert!(
            text.contains(&format!("| `{}` |", function.name)),
            "07 does not state the function `{}`",
            function.name
        );
    }
    for verb in ranged::verbs() {
        let id = registry.verb_id(verb.name).unwrap();
        let spec = &registry.verb(id).unwrap().spec;
        assert!(
            text.contains(&format!("| `{}` |", verb.name)),
            "07 does not state the verb `{}`",
            verb.name
        );
        for arg in &spec.args {
            assert!(!arg.doc.is_empty(), "{}.{}", verb.name, arg.name);
            assert!(
                text.contains(&format!("`{}`", arg.name)),
                "{}.{}",
                verb.name,
                arg.name
            );
        }
    }
    for property in ranged::properties() {
        assert!(
            text.contains(&format!("`entity.{}", property.name)),
            "07 does not state `entity.{}`",
            property.name
        );
    }
}
