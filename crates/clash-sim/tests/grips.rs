//! Close combat against the real simulation (07 §9 to §13, 03 §8):
//! people spawned from `core:person`, fighting with the bundled actions
//! of `mods/core/data/combat`.
//!
//! Shots meet reactions (07 §11.6): a shot opens a `shot` window for
//! whoever it is aimed at, noticed only by the quick (`bullet_time`); a
//! slow arrow is caught (`caught_arrow`); a raised shield interposes;
//! `parry` is unavailable with nothing that has a flat in hand.
//!
//! C7: a grip lasts exactly as long as its action (invariant C7); a
//! pinned arm's actions are unavailable until release, with the reason;
//! the held cannot step away and is dragged; a throw sends the held
//! through a window (`thrown_through`); a held creature takes a blow
//! meant for its holder (`human_shield`); save and load mid-grip.
//!
//! C8 and T6: a takedown locks an unaware victim in and strikes it, and
//! cues its cutscene; an aware victim cannot be taken down, with the
//! reason; a steel collar stops the blow; a choke knocks out; ending the
//! attacker's action ends it for both; save and load mid-takedown;
//! replay.
//!
//! C11 and C12: stances exclude each other; `auto_effort` holds a
//! guarded blow back; `strike_preview` draws nothing and changes
//! nothing; a guard lays hands on a thief who runs; two guards fight to
//! a conclusion under the bundled minds.

mod common;

use std::path::PathBuf;
use std::sync::{Arc, LazyLock};

use clash_core::{EntityId, Real, Tick};
use clash_data::Defs;
use clash_expr::Value;
use clash_sim::action::{ActionDef, ActionDefId, How, Params};
use clash_sim::body::props::part_handle;
use clash_sim::combat::grip;
use clash_sim::combat::ranged::{self, Aim, ProjectileDef, ShotSpec};
use clash_sim::decision::DecisionKind;
use clash_sim::template::{CharacterSpec, Selection};
use clash_sim::{
    Advance, AdvanceLimit, Command, Control, DebugCommand, Place, Sim, SimEvent, Subject,
};
use clash_world::{Cause, Layer};

const MS: i64 = 1_000;

// ---- Loading -------------------------------------------------------------------

/// The JSON files of a directory of `mods/core/data`, in file-name order.
fn core_dir(sub: &str) -> Vec<(String, String)> {
    let dir = common::mods_dir().join("core/data").join(sub);
    let mut files: Vec<PathBuf> = std::fs::read_dir(&dir)
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

/// A file of `mods/core/data` without the definitions of some types.
fn core_file(name: &str, without: &[&str]) -> (String, String) {
    let path = common::mods_dir().join("core/data").join(name);
    let text = std::fs::read_to_string(path).unwrap();
    let all: Vec<serde_json::Value> = serde_json::from_str(&text).unwrap();
    let kept: Vec<serde_json::Value> = all
        .into_iter()
        .filter(|def| {
            !without.contains(
                &def.get("type")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or(""),
            )
        })
        .collect();
    (
        format!("data/{name}"),
        serde_json::to_string_pretty(&kept).unwrap(),
    )
}

/// `mods/core`'s life, minds, perception, combat, movement, items,
/// traits and shields: nothing outside `core`.
fn load() -> Arc<Defs> {
    let mut owned = core_dir("life");
    owned.extend(core_dir("ai"));
    owned.extend(core_dir("perception"));
    owned.extend(core_dir("combat"));
    owned.push(core_file("movement.json", &[]));
    owned.push(core_file("items.json", &["color"]));
    owned.push(core_file("shields.json", &[]));
    owned.push(core_file("traits.json", &[]));
    let files: Vec<(&str, &str)> = owned
        .iter()
        .map(|(path, text)| (path.as_str(), text.as_str()))
        .collect();
    common::try_defs_with(&files).unwrap_or_else(|error| panic!("{error}"))
}

static DEFS: LazyLock<Arc<Defs>> = LazyLock::new(load);

// ---- Helpers ---------------------------------------------------------------------

fn debug(sim: &mut Sim, command: DebugCommand) {
    sim.submit(Command::Debug(command))
        .unwrap_or_else(|error| panic!("{error}"));
}

/// Spawns someone from templates by command, delegated, so that a
/// replay makes them too.
fn spawn_as(sim: &mut Sim, templates: &[&str], name: &str, x: i32, y: i32) -> EntityId {
    let id = sim.truth().entities().next_id().unwrap();
    let spec = CharacterSpec::of(templates.iter().copied())
        .with("core:person.name", Selection::Text(name.to_owned()));
    debug(
        sim,
        DebugCommand::Spawn {
            spec,
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

fn spawn(sim: &mut Sim, name: &str, x: i32) -> EntityId {
    spawn_as(sim, &["core:person"], name, x, 10)
}

fn flat(seed: u64) -> Sim {
    common::sim_over(Arc::clone(&DEFS), seed)
}

fn action(sim: &Sim, name: &str) -> ActionDefId {
    sim.defs()
        .lookup::<ActionDef>(name)
        .unwrap_or_else(|| panic!("no action {name}"))
}

fn entity_value(sim: &Sim, who: EntityId) -> clash_core::Value {
    sim.subject_value(Subject::Entity(who)).to_stored()
}

fn part_value(sim: &Sim, who: EntityId, part: &str) -> clash_core::Value {
    let key = sim
        .components()
        .body
        .get(who)
        .unwrap()
        .part_named(part)
        .unwrap()
        .key();
    let kinds = sim.body_defs().unwrap().kinds();
    Value::Subject(part_handle(kinds, who, key)).to_stored()
}

fn set_attr(sim: &mut Sim, who: EntityId, name: &str, value: f64) {
    let id = sim.stat_defs().unwrap().attribute_id(name).unwrap();
    sim.set_attr_base(who, id, Some(Real::of(value)), Cause::DEBUG)
        .unwrap();
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

/// Makes an item of `mods/core` and has someone wear it.
fn wear(sim: &mut Sim, who: EntityId, def: &str) -> EntityId {
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

/// Gives someone sight, aware of another and facing them.
fn watch(sim: &mut Sim, who: EntityId, of: EntityId) {
    let sight = sim
        .defs()
        .lookup::<clash_sim::perception::defs::SenseDef>("core:human_sight")
        .unwrap();
    let sees = sim
        .components()
        .senses
        .get(who)
        .is_some_and(|senses| senses.slot(sight).is_some());
    if !sees {
        clash_sim::perception::attach_sense(sim, who, sight).unwrap();
    }
    let at = sim.truth().location(of).unwrap();
    clash_sim::perception::face_toward(sim, who, at).unwrap();
    clash_sim::perception::set_awareness(sim, who, of, sight, Real::ONE).unwrap();
    assert!(clash_sim::perception::perceives(sim, who, of));
}

fn attach(sim: &mut Sim, who: EntityId, rule: &str) {
    let rule = sim.rule_set().lookup(rule).unwrap();
    let handle = sim
        .subject_value(Subject::Entity(who))
        .as_subject()
        .unwrap();
    assert!(sim.rules.attach(handle, rule));
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

fn try_act(sim: &mut Sim, who: EntityId, name: &str, params: Params) -> Result<(), String> {
    let action = action(sim, name);
    sim.submit(Command::Act {
        entity: who,
        action,
        params,
        how: How::Start,
    })
    .map(|_| ())
    .map_err(|error| error.to_string())
}

fn target(sim: &Sim, name: &str, who: EntityId) -> (String, clash_core::Value) {
    (name.to_owned(), entity_value(sim, who))
}

/// Advances by so many microseconds. Whoever is asked what to do next
/// waits.
fn run_for(sim: &mut Sim, micros: i64) {
    let until = Tick(sim.now().as_micros() + micros);
    loop {
        match sim.advance(AdvanceLimit::Until(until)) {
            Advance::Reached(_) => return,
            Advance::Yielded => {}
            Advance::NeedInput(request) if matches!(request.kind, DecisionKind::Free { .. }) => {
                sim.submit(Command::Wait {
                    entity: request.entity,
                    dur: clash_core::Dur::from_secs(5),
                })
                .unwrap();
            }
            other => panic!("{other:?}"),
        }
    }
}

fn named<'a>(events: &'a [SimEvent], name: &str) -> Vec<&'a SimEvent> {
    events.iter().filter(|event| event.name() == name).collect()
}

fn about(event: &SimEvent, who: EntityId) -> bool {
    event.subject("self") == Some(Subject::Entity(who))
}

fn text(event: &SimEvent, name: &str) -> String {
    match event.body.field(name) {
        Some(clash_sim::FieldValue::Plain(clash_core::Value::Str(text))) => text.to_string(),
        other => panic!("{} has no text `{name}`: {other:?}", event.name()),
    }
}

fn faults(events: &[SimEvent]) -> Vec<String> {
    events
        .iter()
        .filter(|event| event.name() == "fault")
        .map(|event| format!("{event:?}"))
        .collect()
}

fn dump(events: &[SimEvent]) {
    if std::env::var("GRIPS_DEBUG").is_ok() {
        for event in events {
            eprintln!(
                "{} {} {:?}",
                event.tick.as_micros(),
                event.name(),
                event.body
            );
        }
    }
}

fn wounds(sim: &Sim, who: EntityId) -> usize {
    sim.components()
        .body
        .get(who)
        .map_or(0, |body| body.wounds().len())
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

fn x_of(sim: &Sim, who: EntityId) -> i32 {
    sim.truth().location(who).unwrap().cell.x
}

fn projectile(sim: &Sim, name: &str) -> ranged::ProjectileId {
    sim.defs()
        .lookup::<ProjectileDef>(&format!("core:{name}"))
        .unwrap_or_else(|| panic!("no projectile {name}"))
}

/// A shot of a projectile at someone, at a height, with the bundled
/// `shot` window.
fn shot_at(sim: &Sim, shooter: EntityId, at: EntityId, name: &str, energy: f64) -> ShotSpec {
    let aim: Aim = ranged::shoot::aim_at_entity(sim, at, None, Some(Real::of(1.2))).unwrap();
    let mut spec = ShotSpec::new(shooter, aim);
    spec.target = Some(at);
    spec.projectile = Some(projectile(sim, name));
    spec.energy = Some(Real::of(energy));
    spec.spread = Some(Real::ZERO);
    spec.window = sim
        .defs()
        .lookup::<clash_sim::action::WindowKindDef>("core:shot");
    spec
}

// ---- Shots meet reactions (07 §11.6) -------------------------------------------

#[test]
fn bullet_time_a_fast_clock_steps_out_of_a_shot() {
    // A pistol bullet at 500 J leaves at 350 m/s and crosses 30 m in
    // about 85 ms. Bram, wits 10, needs 200 ms of his own time to
    // notice it: he is never offered the chance, and is hit. On a clock
    // twenty times as fast he needs 10 ms, steps across the line in
    // 3 ms more, and the bullet goes by.
    for (rate, hit) in [(None, true), (Some(20.0), false)] {
        let mut sim = flat(5);
        let ada = spawn(&mut sim, "Ada", 10);
        let bram = spawn(&mut sim, "Bram", 40);
        watch(&mut sim, bram, ada);
        attach(&mut sim, bram, "core:step_aside_on_notice");
        if let Some(rate) = rate {
            sim.set_time_rate(bram, Some(Real::of(rate)), Cause::DEBUG)
                .unwrap();
        }
        sim.pump();
        sim.drain_events();
        let spec = shot_at(&sim, ada, bram, "pistol_bullet", 500.0);
        ranged::shoot(&mut sim, &spec, Cause::DEBUG.by(ada)).unwrap();
        assert_eq!(sim.windows().len(), 1, "the shot opened a window");
        run_for(&mut sim, 400 * MS);
        let told = sim.drain_events();
        dump(&told);
        assert_eq!(faults(&told), Vec::<String>::new());
        let noticed = named(&told, "window_noticed").len();
        let struck = wounds(&sim, bram) > 0;
        if hit {
            assert_eq!(noticed, 0, "an ordinary person is offered no reaction");
            assert!(struck, "and is hit");
        } else {
            assert_eq!(noticed, 1, "on a fast clock it is noticed");
            assert!(!struck, "and stepped out of");
            assert_ne!(sim.truth().location(bram).unwrap().cell.y, 10);
        }
        assert_eq!(sim.windows().len(), 0, "the window closed");
    }
}

#[test]
fn caught_arrow_a_quick_hand_catches_a_slow_shot() {
    // An arrow at 40 J leaves at about 57 m/s and crosses 15 m in a
    // quarter of a second. Bram, wits 40, notices it in 50 ms, and
    // catches it out of the air: it ends in his hand, an arrow. Cora,
    // wits 5, needs 400 ms: the window closes before she notices, and
    // she is hit.
    let mut sim = flat(7);
    let ada = spawn(&mut sim, "Ada", 10);
    let bram = spawn(&mut sim, "Bram", 25);
    let cora = spawn_as(&mut sim, &["core:person"], "Cora", 10, 20);
    set_attr(&mut sim, bram, "core:wits", 40.0);
    set_attr(&mut sim, cora, "core:wits", 5.0);
    watch(&mut sim, bram, ada);
    watch(&mut sim, cora, ada);
    attach(&mut sim, bram, "core:catch_on_notice");
    attach(&mut sim, cora, "core:catch_on_notice");
    sim.pump();
    sim.drain_events();
    let spec = shot_at(&sim, ada, bram, "arrow", 40.0);
    ranged::shoot(&mut sim, &spec, Cause::DEBUG.by(ada)).unwrap();
    run_for(&mut sim, 1_000 * MS);
    let told = sim.drain_events();
    dump(&told);
    assert_eq!(faults(&told), Vec::<String>::new());
    assert_eq!(wounds(&sim, bram), 0, "caught, not struck");
    let held = clash_sim::combat::ranged::store::wielding(&sim, bram, "");
    let arrow = held.expect("an arrow in his hand");
    let def = sim
        .components()
        .get::<clash_sim::item::Item>(arrow)
        .and_then(|item| item.def())
        .map(|def| {
            sim.defs()
                .registry::<clash_sim::item::ItemDef>()
                .name(def)
                .to_owned()
        });
    assert_eq!(def.as_deref(), Some("core:arrow"));
    assert!(named(&told, "projectile_hit").is_empty());

    let spec = shot_at(&sim, ada, cora, "arrow", 40.0);
    ranged::shoot(&mut sim, &spec, Cause::DEBUG.by(ada)).unwrap();
    run_for(&mut sim, 1_000 * MS);
    let told = sim.drain_events();
    assert!(
        named(&told, "window_noticed").is_empty(),
        "too slow to be offered it"
    );
    assert!(!named(&told, "projectile_hit").is_empty());
}

#[test]
fn a_shield_raised_in_time_takes_a_shot() {
    // Bram, quick, raises his buckler against an arrow: it is put in the
    // way (`strike_defended`), the arrow meets the wood with its own
    // point and stops in it, and nothing reaches him.
    let mut sim = flat(9);
    let ada = spawn(&mut sim, "Ada", 10);
    let bram = spawn(&mut sim, "Bram", 25);
    set_attr(&mut sim, bram, "core:wits", 40.0);
    arm(&mut sim, bram, "core:buckler");
    watch(&mut sim, bram, ada);
    attach(&mut sim, bram, "core:shield_on_notice");
    sim.pump();
    sim.drain_events();
    let spec = shot_at(&sim, ada, bram, "arrow", 20.0);
    ranged::shoot(&mut sim, &spec, Cause::DEBUG.by(ada)).unwrap();
    run_for(&mut sim, 1_000 * MS);
    let told = sim.drain_events();
    dump(&told);
    assert_eq!(faults(&told), Vec::<String>::new());
    let defended = named(&told, "strike_defended");
    assert_eq!(defended.len(), 1, "the shield was put in the way");
    assert_eq!(text(defended[0], "form"), "interpose");
    assert_eq!(text(defended[0], "attack"), "shot");
    assert_eq!(wounds(&sim, bram), 0);
}

#[test]
fn parry_needs_something_with_a_flat_in_hand() {
    // Bram sees Ada's punch coming. Bare-handed, the bundled `parry` is
    // listed and refused, with its reason; with a club in hand it may be
    // chosen.
    for armed in [false, true] {
        let mut sim = flat(11);
        let ada = spawn(&mut sim, "Ada", 10);
        let bram = spawn(&mut sim, "Bram", 11);
        set_attr(&mut sim, bram, "core:wits", 20.0);
        if armed {
            arm(&mut sim, bram, "core:club");
        }
        watch(&mut sim, bram, ada);
        sim.pump();
        {
            let params = vec![target(&sim, "target", bram)];
            act(&mut sim, ada, "core:punch", params)
        };
        run_for(&mut sim, 150 * MS);
        let window = sim
            .windows()
            .iter()
            .find(|(_, window)| window.source == Some(ada))
            .map(|(id, _)| id)
            .expect("a window is open");
        let options = sim.reaction_options(bram, window);
        let parry = action(&sim, "core:parry");
        let option = options
            .iter()
            .find(|option| option.action == parry)
            .expect("parry is listed");
        match (&option.refused, armed) {
            (Some(reason), false) => {
                assert_eq!(reason.code, "engine:action_requires");
                assert_eq!(reason.message, "There is nothing in hand to parry with.");
            }
            (None, true) => {}
            (refused, _) => panic!("armed {armed}: {refused:?}"),
        }
    }
}

// ---- C7: grips and throws -----------------------------------------------------------

/// Ada grabs Bram's right arm, and holds on.
fn grabbed(seed: u64) -> (Sim, EntityId, EntityId) {
    let mut sim = flat(seed);
    let ada = spawn(&mut sim, "Ada", 10);
    let bram = spawn(&mut sim, "Bram", 11);
    sim.pump();
    let params = vec![
        target(&sim, "target", bram),
        ("part".to_owned(), part_value(&sim, bram, "r_arm")),
    ];
    act(&mut sim, ada, "core:grab", params);
    run_for(&mut sim, 400 * MS);
    (sim, ada, bram)
}

#[test]
fn a_grip_lasts_exactly_as_long_as_its_action() {
    // Invariant C7: the grip is there while the grab holds, and goes as
    // the action ends, by a cancel or by an interrupt.
    let (mut sim, ada, bram) = grabbed(13);
    let told = sim.drain_events();
    dump(&told);
    assert_eq!(faults(&told), Vec::<String>::new());
    assert_eq!(named(&told, "seized").len(), 1);
    assert_eq!(grip::holder_of(&sim, bram), Some(ada));
    let running = sim.actions_of(ada)[0];
    assert!(sim.grips().of_action(running).next().is_some());
    sim.submit(Command::Cancel {
        entity: ada,
        action: running,
        abandon: true,
    })
    .unwrap();
    sim.pump();
    assert_eq!(grip::holder_of(&sim, bram), None, "gone with its action");
    let told = sim.drain_events();
    let ended = named(&told, "grip_broken");
    assert_eq!(ended.len(), 1);
    assert_eq!(text(ended[0], "how"), "ended");

    let (mut sim, ada, bram) = grabbed(13);
    let running = sim.actions_of(ada)[0];
    sim.interrupt_action(
        running,
        clash_sim::Reason::new("test.stop", "stopped"),
        Cause::DEBUG,
    );
    assert_eq!(grip::holder_of(&sim, bram), None);
    assert!(sim.grips().is_empty());
}

#[test]
fn a_pinned_arm_s_actions_are_unavailable_until_release() {
    let (mut sim, ada, bram) = grabbed(15);
    // Bram's right arm is pinned: a shove, which needs both arms, is
    // refused with the reason the runtime gives for slots lacking.
    let params = vec![target(&sim, "target", ada)];
    let shove = try_act(&mut sim, bram, "core:shove", params);
    let error = shove.expect_err("both arms are not free");
    assert!(
        error.contains("action_channel_lacking") || error.contains("arm"),
        "{error}"
    );
    // Released, it can be done.
    grip::release_grip(&mut sim, ada, None, Cause::DEBUG);
    sim.pump();
    assert!(grip::holder_of(&sim, bram).is_none());
    {
        let params = vec![target(&sim, "target", ada)];
        try_act(&mut sim, bram, "core:shove", params)
    }
    .expect("the arm is free again");
}

#[test]
fn the_held_cannot_step_away_and_is_dragged() {
    let (mut sim, ada, bram) = grabbed(17);
    sim.drain_events();
    // Bram steps east, away from Ada: refused, held.
    act(
        &mut sim,
        bram,
        "core:step",
        vec![("dir".to_owned(), clash_core::Value::from("east"))],
    );
    run_for(&mut sim, 1_500 * MS);
    let told = sim.drain_events();
    let refused = named(&told, "step_refused");
    assert!(
        refused
            .iter()
            .any(|event| text(event, "code") == grip::GRIPPED),
        "{refused:?}"
    );
    assert_eq!(x_of(&sim, bram), 11);
    // Ada steps west: Bram is dragged into where she stood.
    act(
        &mut sim,
        ada,
        "core:step",
        vec![("dir".to_owned(), clash_core::Value::from("west"))],
    );
    run_for(&mut sim, 1_500 * MS);
    assert_eq!(x_of(&sim, ada), 9);
    assert_eq!(x_of(&sim, bram), 10, "dragged after her");
    assert_eq!(grip::holder_of(&sim, bram), Some(ada));
}

#[test]
fn thrown_through_a_window() {
    // A titan of strength 1500 takes hold of Bram and throws him east,
    // into a window a step behind him at 45 kJ: thirty-four metres a
    // second, over five hundred kilopascals on the glass, which breaks;
    // he lands outside.
    let mut sim = flat(19);
    let ada = spawn(&mut sim, "Ada", 10);
    let bram = spawn(&mut sim, "Bram", 11);
    set_attr(&mut sim, ada, "core:strength", 1500.0);
    set_block(&mut sim, 12, 10, "window");
    let thrown_mass = clash_sim::combat::push::mass_of(&sim, bram).get();
    sim.pump();
    sim.drain_events();
    {
        let params = vec![target(&sim, "target", bram)];
        act(&mut sim, ada, "core:throw_person", params)
    };
    run_for(&mut sim, 4_000 * MS);
    let told = sim.drain_events();
    dump(&told);
    assert_eq!(faults(&told), Vec::<String>::new());
    let thrown = named(&told, "thrown");
    assert_eq!(thrown.len(), 1);
    let speed = thrown[0]
        .body
        .field("speed")
        .unwrap()
        .as_real()
        .unwrap()
        .get();
    let mass = thrown_mass;
    let expected = (2.0 * 1500.0 * 30.0 / mass).sqrt();
    assert!((speed - expected).abs() < 1e-6, "{speed} vs {expected}");
    assert!(
        named(&told, "element_destroyed").iter().any(|_| true),
        "the window broke"
    );
    assert!(
        x_of(&sim, bram) > 12,
        "landed outside: x {}",
        x_of(&sim, bram)
    );
    assert!(sim.grips().is_empty());
}

#[test]
fn human_shield_a_blow_lands_on_the_one_held() {
    // Ada holds Bram up in front of her; Cora slashes at Ada with a
    // knife. The blow lands on Bram, with the knife's edge; Ada is not
    // touched.
    let mut sim = flat(21);
    let ada = spawn(&mut sim, "Ada", 10);
    let bram = spawn(&mut sim, "Bram", 11);
    let cora = spawn_as(&mut sim, &["core:person"], "Cora", 10, 11);
    arm(&mut sim, cora, "core:knife");
    set_attr(&mut sim, cora, "core:strength", 20.0);
    set_attr(&mut sim, cora, "core:wits", 60.0);
    sim.pump();
    {
        let params = vec![target(&sim, "target", bram)];
        act(&mut sim, ada, "core:human_shield", params)
    };
    run_for(&mut sim, 500 * MS);
    assert_eq!(grip::holder_of(&sim, bram), Some(ada));
    sim.drain_events();
    {
        let params = vec![target(&sim, "target", ada)];
        act(&mut sim, cora, "core:slash", params)
    };
    run_for(&mut sim, 800 * MS);
    let told = sim.drain_events();
    dump(&told);
    assert_eq!(faults(&told), Vec::<String>::new());
    let defended = named(&told, "strike_defended");
    assert_eq!(defended.len(), 1);
    assert_eq!(text(defended[0], "form"), "interpose");
    assert!(wounds(&sim, bram) > 0, "the one held took it");
    let through: f64 = named(&told, "strike_landed")
        .iter()
        .filter(|event| event.subject("target") == Some(Subject::Entity(ada)))
        .filter_map(|event| event.body.field("energy").and_then(|value| value.as_real()))
        .map(Real::get)
        .sum();
    assert!(
        through < 95.0,
        "only what went through him reached her: {through} J"
    );
}

#[test]
fn save_and_load_mid_grip_change_nothing() {
    let (mut sim, ada, bram) = grabbed(23);
    sim.take_input_log();
    let (mut loaded, unplaced) = common::save_and_load(&sim);
    assert!(unplaced.is_empty());
    assert_eq!(loaded.state_hash(), sim.state_hash());
    assert_eq!(grip::holder_of(&loaded, bram), Some(ada));
    for each in [&mut sim, &mut loaded] {
        act(
            each,
            ada,
            "core:step",
            vec![("dir".to_owned(), clash_core::Value::from("west"))],
        );
        run_for(each, 2_000 * MS);
    }
    assert_eq!(loaded.state_hash(), sim.state_hash());
    assert_eq!(x_of(&loaded, bram), 10);
}

// ---- C8 and T6: takedowns ------------------------------------------------------------

/// Ada stands behind Bram, who faces away and has not seen her.
fn behind(seed: u64) -> (Sim, EntityId, EntityId) {
    let mut sim = flat(seed);
    let ada = spawn(&mut sim, "Ada", 10);
    let bram = spawn(&mut sim, "Bram", 11);
    sim.pump();
    (sim, ada, bram)
}

fn take_down(sim: &mut Sim, ada: EntityId, bram: EntityId, name: &str) {
    let params = vec![target(sim, "victim", bram)];
    act(sim, ada, name, params);
}

#[test]
fn takedown_locks_the_unaware_victim_in_and_strikes() {
    let (mut sim, ada, bram) = behind(25);
    sim.drain_events();
    take_down(&mut sim, ada, bram, "core:knock_out");
    sim.pump();
    let joint = sim.actions_of(ada)[0];
    // Both are locked in: the action is Bram's too, holding his arms and
    // legs, and he can start nothing that needs them.
    assert_eq!(sim.actions_of(bram), &[joint]);
    assert!(sim.actions.held_of(joint, bram).is_some());
    assert!(
        {
            let params = vec![target(&sim, "target", ada)];
            try_act(&mut sim, bram, "core:shove", params)
        }
        .is_err()
    );
    run_for(&mut sim, 1_000 * MS);
    let told = sim.drain_events();
    dump(&told);
    assert_eq!(faults(&told), Vec::<String>::new());
    let cue = named(&told, "cutscene_cued");
    assert_eq!(cue.len(), 1, "the cutscene is cued");
    assert_eq!(text(cue[0], "cutscene"), "knock_out");
    let landed = named(&told, "strike_landed");
    assert_eq!(landed.len(), 1);
    assert!(
        matches!(
            landed[0].body.field("margin"),
            Some(clash_sim::FieldValue::Null)
        ),
        "sure: it did not aim"
    );
    assert!(wounds(&sim, bram) > 0 || sim.vital(bram) != Some(clash_sim::mind::VitalState::Awake));
    // Let go together.
    assert!(!sim.actions_of(bram).contains(&joint));
    assert!(!sim.actions_of(ada).contains(&joint));
}

#[test]
fn an_aware_victim_cannot_be_taken_down() {
    let (mut sim, ada, bram) = behind(27);
    watch(&mut sim, bram, ada);
    sim.drain_events();
    take_down(&mut sim, ada, bram, "core:knock_out");
    run_for(&mut sim, 1_000 * MS);
    let told = sim.drain_events();
    let interrupted = named(&told, "action_interrupted");
    assert_eq!(interrupted.len(), 1);
    assert!(format!("{:?}", interrupted[0].body).contains("action_start_condition"));
    assert!(named(&told, "strike_landed").is_empty());
    assert!(named(&told, "cutscene_cued").is_empty());
    assert_eq!(wounds(&sim, bram), 0);
    assert!(sim.actions_of(bram).is_empty());
}

#[test]
fn a_steel_collar_stops_the_takedown_blow() {
    let (mut sim, ada, bram) = behind(29);
    wear(&mut sim, bram, "core:gorget");
    sim.drain_events();
    take_down(&mut sim, ada, bram, "core:knock_out");
    run_for(&mut sim, 1_000 * MS);
    let told = sim.drain_events();
    dump(&told);
    assert_eq!(faults(&told), Vec::<String>::new());
    assert_eq!(named(&told, "strike_landed").len(), 1, "it was delivered");
    assert_eq!(wounds(&sim, bram), 0, "and the layer stopped it");
    assert!(
        !named(&told, "strike_recoiled").is_empty(),
        "back into the fist"
    );
}

#[test]
fn a_choke_knocks_out_and_the_action_goes_on() {
    let (mut sim, ada, bram) = behind(31);
    sim.drain_events();
    take_down(&mut sim, ada, bram, "core:choke");
    run_for(&mut sim, 6_000 * MS);
    let told = sim.drain_events();
    dump(&told);
    assert_eq!(faults(&told), Vec::<String>::new());
    assert_eq!(
        sim.vital(bram),
        Some(clash_sim::mind::VitalState::Unconscious)
    );
    assert_eq!(
        named(&told, "action_finished").len(),
        1,
        "it ran to its end"
    );
    assert!(sim.actions_of(bram).is_empty());
}

#[test]
fn interrupting_the_attacker_ends_it_for_both() {
    let (mut sim, ada, bram) = behind(33);
    take_down(&mut sim, ada, bram, "core:choke");
    run_for(&mut sim, 500 * MS);
    let joint = sim.actions_of(ada)[0];
    assert_eq!(sim.actions_of(bram), &[joint]);
    sim.interrupt_action(
        joint,
        clash_sim::Reason::new("test.stop", "stopped"),
        Cause::DEBUG,
    );
    assert!(sim.actions_of(bram).is_empty());
    assert!(sim.actions_of(ada).is_empty());
    assert_eq!(sim.vital(bram), Some(clash_sim::mind::VitalState::Awake));
}

#[test]
fn save_load_and_replay_mid_takedown_change_nothing() {
    let (mut sim, ada, bram) = behind(35);
    sim.take_input_log();
    let (mut replayed, _) = common::save_and_load(&sim);
    take_down(&mut sim, ada, bram, "core:choke");
    run_for(&mut sim, 1_500 * MS);
    let (mut loaded, unplaced) = common::save_and_load(&sim);
    assert!(unplaced.is_empty());
    assert_eq!(loaded.state_hash(), sim.state_hash());
    run_for(&mut sim, 4_000 * MS);
    run_for(&mut loaded, 4_000 * MS);
    assert_eq!(loaded.state_hash(), sim.state_hash());
    replayed.replay(sim.input_log()).unwrap();
    let left = sim.now().as_micros() - replayed.now().as_micros();
    run_for(&mut replayed, left);
    assert_eq!(replayed.state_hash(), sim.state_hash());
}

// ---- C11 and C12: stances, effort, previews, minds -------------------------------------------

fn trait_value(sim: &Sim, name: &str) -> clash_core::Value {
    let id = sim
        .defs()
        .lookup::<clash_sim::traits::TraitDef>(name)
        .unwrap();
    let ty = sim.defs().expr_registry().def_type_id("trait").unwrap();
    Value::Id(clash_expr::DefRef::new(ty, id.raw())).to_stored()
}

#[test]
fn stances_exclude_each_other_and_hold_a_guarded_blow_back() {
    let (mut sim, ada, bram) = behind(37);
    for stance in ["core:aggressive", "core:defensive"] {
        let value = trait_value(&sim, stance);
        act(
            &mut sim,
            ada,
            "core:take_stance",
            vec![("stance".to_owned(), value)],
        );
        run_for(&mut sim, 500 * MS);
    }
    let has = |sim: &Sim, tag: &str| {
        sim.property_named(Subject::Entity(ada), "has_tag", &[Value::from(tag)])
            == Value::Bool(true)
    };
    assert!(has(&sim, "defensive"));
    assert!(!has(&sim, "aggressive"), "taking one leaves the other");
    sim.drain_events();
    {
        let params = vec![target(&sim, "target", bram)];
        act(&mut sim, ada, "core:punch", params)
    };
    run_for(&mut sim, 800 * MS);
    let told = sim.drain_events();
    let effort = named(&told, "strike_landed")
        .first()
        .and_then(|event| event.body.field("effort"))
        .and_then(|value| value.as_real())
        .map(Real::get);
    assert_eq!(effort, Some(0.7), "a guarded blow is held back");
}

#[test]
fn strike_preview_draws_nothing_and_changes_nothing() {
    let (mut sim, ada, bram) = behind(39);
    watch(&mut sim, bram, ada);
    sim.pump();
    let before = (sim.state_hash(), sim.rng.clone());
    let punch = action(&sim, "core:punch");
    let preview =
        clash_sim::combat::preview::strike_preview(&sim, ada, punch, None, bram, None, Real::ONE)
            .expect("a preview");
    assert_eq!(sim.state_hash(), before.0);
    assert_eq!(sim.rng, before.1);
    assert!(preview.in_reach);
    // Between equals the bundled `melee` lands half its blows on the
    // mark (07 §4.2).
    assert!((preview.odds.get() - 0.5).abs() < 1e-9, "{}", preview.odds);
    assert!(preview.odds_near > preview.odds);
    assert!((preview.energy.get() - 80.0 * 0.9).abs() < 1e-6);
    // The same through the function the application calls.
    let read = clash_sim::combat::preview::strike_preview(
        &sim,
        ada,
        punch,
        None,
        bram,
        None,
        Real::of(0.5),
    )
    .unwrap();
    assert!((read.energy.get() - 36.0).abs() < 1e-6);
    assert_eq!(sim.state_hash(), before.0);
    // Against one who has not seen it coming, the bundled `evasion` is
    // nothing: it always lands.
    let (sim, ada, bram) = behind(39);
    let unaware =
        clash_sim::combat::preview::strike_preview(&sim, ada, punch, None, bram, None, Real::ONE)
            .unwrap();
    assert_eq!(unaware.odds, Real::ONE);
}

// ---- Minds -------------------------------------------------------------------------------------

#[test]
fn a_guard_lays_hands_on_a_thief_who_runs() {
    let mut sim = flat(41);
    let guard = spawn_as(&mut sim, &["core:person", "core:guard"], "Watch", 10, 10);
    let thief = spawn(&mut sim, "Thief", 11);
    sim.entity_mut(thief).unwrap().tags.insert("outlaw");
    sim.pump();
    let away = sim
        .subject_value(Subject::Loc(common::at(30, 10)))
        .to_stored();
    act(&mut sim, thief, "core:go_to", vec![("to".to_owned(), away)]);
    run_for(&mut sim, 6_000 * MS);
    let told = sim.drain_events();
    dump(&told);
    let seized = named(&told, "seized");
    assert!(
        seized.iter().any(|event| about(event, guard)),
        "the guard took hold of him"
    );
    assert!(
        x_of(&sim, thief) < 20,
        "he did not get far: x {}",
        x_of(&sim, thief)
    );
}

#[test]
fn two_guards_fight_to_a_conclusion_under_the_bundled_minds() {
    // One holds the other an outlaw; once struck, the other holds him
    // hostile in turn. Nothing outside `core` decides: they go at each
    // other until one of them is down.
    let mut sim = flat(43);
    let a = spawn_as(&mut sim, &["core:person", "core:guard"], "Ada", 10, 10);
    let b = spawn_as(&mut sim, &["core:person", "core:guard"], "Bram", 13, 10);
    sim.entity_mut(b).unwrap().tags.insert("outlaw");
    sim.pump();
    let mut over = false;
    for _ in 0..240 {
        run_for(&mut sim, 1_000 * MS);
        sim.drain_events();
        let down = |who| sim.vital(who) != Some(clash_sim::mind::VitalState::Awake);
        if down(a) || down(b) {
            over = true;
            break;
        }
    }
    assert!(over, "the fight came to an end");
}

// ---- The documents state what is built ------------------------------------------------------

#[test]
fn document_07_states_grips_takedowns_shots_and_previews() {
    let document = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../docs/07-combat.md");
    let text = std::fs::read_to_string(document).unwrap();
    let registry = DEFS.expr_registry();
    let events = clash_sim::combat::GRIP_EVENTS
        .iter()
        .chain(clash_sim::combat::takedown::TAKEDOWN_EVENTS);
    for info in events {
        assert!(registry.event_id(info.name).is_some(), "{}", info.name);
        assert!(
            text.contains(&format!("| `{}`", info.name)),
            "{}",
            info.name
        );
        for (field, _, doc) in info.fields {
            assert!(!doc.is_empty());
            assert!(
                text.contains(&format!("`{field}`")),
                "{}.{field}",
                info.name
            );
        }
    }
    for info in clash_sim::combat::GRIP_REASONS {
        assert!(text.contains(&format!("`{}`", info.code)), "{}", info.code);
    }
    for verb in grip::verbs() {
        let id = registry.verb_id(verb.name).unwrap();
        let spec = &registry.verb(id).unwrap().spec;
        assert!(
            text.contains(&format!("| `{}` |", verb.name)),
            "{}",
            verb.name
        );
        for arg in &spec.args {
            assert!(!arg.doc.is_empty(), "{}.{}", verb.name, arg.name);
            assert!(
                text.contains(&format!("`{}: ", arg.name)),
                "{}.{}",
                verb.name,
                arg.name
            );
        }
    }
    let properties = grip::properties()
        .into_iter()
        .chain(clash_sim::combat::takedown::properties());
    for property in properties {
        assert!(
            text.contains(&format!("`entity.{}`", property.name)),
            "{}",
            property.name
        );
    }
    assert!(text.contains("`entity.wielding_surface"));
    for function in clash_sim::combat::preview::functions() {
        assert!(text.contains(&format!("| `{}` |", function.name)));
    }
    assert!(text.contains("| `seize_resolving` (hook)"));
    let kernel =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../docs/03-time-and-actions.md");
    let kernel = std::fs::read_to_string(kernel).unwrap();
    assert!(kernel.contains("| `cutscene` |"));
}
