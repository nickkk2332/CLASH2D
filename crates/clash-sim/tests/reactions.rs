//! Windows, reactions and defences against the real simulation
//! (03 §7, 07 §5; packages T5 and C4): people spawned from
//! `core:person`, armed with the items of `mods/core`, answering blows
//! with the reactions of `mods/core/data/combat/reactions.json`.
//!
//! T5: `too_fast_to_notice`; a candidate in a phase that does not allow
//! cancelling has no reaction available, and one in a phase that does
//! is cancelled to make room; with no rule listening nothing happens; a
//! player-controlled defender is asked, and the reaction chosen runs, or
//! passes; who is offered a window (the review row, with the relation
//! supplied here); save and load while a window is open; replay.
//!
//! C4: the three worked exchanges of 07 §14 to their figures; a defence
//! applies only in force at impact (invariant C3); leaving and slipping;
//! a guard held up; defending someone else; energy accounted for; a
//! guard's mind answers a blow by its data and a townsperson's does not.

mod common;

use std::sync::{Arc, LazyLock};

use clash_core::{EntityId, Real, Tick};
use clash_data::Defs;
use clash_expr::Value;
use clash_sim::action::{ActionDef, ActionDefId, How, Notice, Params, WindowId};
use clash_sim::body::props::part_handle;
use clash_sim::decision::DecisionKind;
use clash_sim::template::{CharacterSpec, Selection};
use clash_sim::{
    Advance, AdvanceLimit, Command, Control, DebugCommand, Place, Sim, SimEvent, Subject,
};
use clash_world::Cause;

const MS: i64 = 1_000;

/// What the tests add: a swing whose wind-up and energy are given with
/// the command and that cannot miss its mark, so that the worked
/// exchanges come out to their figures; a parry with the bundled one's
/// numbers and no contest; actions that hold both arms, one that may be
/// broken off and one that may not; a slip with no contest; a
/// reaction that answers for someone else; and the stance that parries.
const FIXTURES: &str = r#"[
  { "type": "action", "id": "test_swing",
    "params": { "target": "entity", "part": { "type": "part?", "default": "null" },
                "windup": { "type": "real", "default": "0.3" },
                "energy": { "type": "real", "default": "180" },
                "push": { "type": "real", "default": "0" } },
    "attacks": { "cut": { "with": { "held": "blade", "surface": "edge" },
                          "energy": "$energy", "push": "$push", "reach": 0.8 } },
    "phases": [
      { "id": "windup", "time": "$windup", "cancel": true,
        "window": { "kind": "strike", "for": ["$target"], "closes": "impact" } },
      { "id": "impact", "time": 0,
        "do": [ { "strike": { "target": "$target", "part": "$part", "sure": true } } ] },
      { "id": "recover", "time": "200 ms" } ] },

  { "type": "action", "id": "test_parry", "name": "Test parry",
    "reaction_to": ["strike"], "uses": { "arm": 1 }, "ready": "guard",
    "phases": [ { "id": "set", "time": "80 ms" }, { "id": "guard", "time": "250 ms" },
                { "id": "recover", "time": "150 ms" } ],
    "defence": { "during": "guard", "form": "interpose",
                 "with": { "held": "blade", "surface": "flat" },
                 "covers": { "arc": 120 }, "handles": 1, "deflect": 0.9,
                 "holds": "(self.attr.strength ?? 10) * 60", "brace": 0.3 } },

  { "type": "action", "id": "test_slip", "reaction_to": ["strike"], "uses": { "leg": 1 },
    "ready": "low",
    "phases": [ { "id": "low", "time": "400 ms" } ],
    "defence": { "during": "low", "form": "avoid" } },

  { "type": "action", "id": "test_shield_him", "reaction_to": ["strike"], "on_behalf": true,
    "params": { "ward": { "type": "entity?", "default": "null" } },
    "uses": { "arm": 1 },
    "phases": [ { "id": "step_in", "time": "50 ms" }, { "id": "cover", "time": "400 ms" } ],
    "defence": { "during": "cover", "form": "interpose", "protects": "$ward",
                 "with": { "part": "arm", "surface": "forearm" },
                 "deflect": 0.5 } },

  { "type": "action", "id": "test_commit", "uses": { "arm": 2 },
    "phases": [ { "id": "hold", "time": "2 s" } ] },
  { "type": "action", "id": "test_loose", "uses": { "arm": 2 },
    "phases": [ { "id": "hold", "time": "2 s", "cancel": true } ] },

  { "type": "eoc", "id": "test_parry_on_notice", "on": "window_noticed", "attached": true,
    "do": [ { "react": { "who": "self", "window": "event.window", "action": "test_parry" } } ] },
  { "type": "eoc", "id": "test_slip_on_notice", "on": "window_noticed", "attached": true,
    "do": [ { "react": { "who": "self", "window": "event.window", "action": "test_slip" } } ] }
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

/// A file of `mods/core/data`, without the definitions of a type.
fn core_file(name: &str, without: &[&str]) -> (String, String) {
    let path = common::mods_dir().join("core/data").join(name);
    let text = std::fs::read_to_string(path).unwrap();
    let all: Vec<serde_json::Value> = serde_json::from_str(&text).unwrap();
    let count = all.len();
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
    // Written out again, an object's keys come in another order, and a
    // parameter's default may name one written before it: a file that
    // loses nothing is kept as it is.
    if kept.len() == count {
        return (format!("data/{name}"), text);
    }
    (
        format!("data/{name}"),
        serde_json::to_string_pretty(&kept).unwrap(),
    )
}

/// `mods/core`'s life, combat, perception, movement and items, the
/// guards' surfaces, and the fixtures; without rules if `rules` is
/// false.
fn load(rules: bool) -> Arc<Defs> {
    load_with(rules, false)
}

/// [`load`], with the bundled minds if `minds` is true.
fn load_with(rules: bool, minds: bool) -> Arc<Defs> {
    let strip: &[&str] = if rules { &[] } else { &["eoc"] };
    let mut owned = core_dir("life");
    if minds {
        owned.extend(core_dir("ai"));
    }
    owned.extend(core_dir("perception"));
    for (path, _) in core_dir("combat") {
        let name = path.trim_start_matches("data/");
        owned.push(core_file(name, strip));
    }
    owned.push(core_file("movement.json", &[]));
    owned.push(core_file("items.json", &["color"]));
    owned.push(core_file("shields.json", &[]));
    let fixtures: Vec<serde_json::Value> = serde_json::from_str(FIXTURES).unwrap();
    let fixtures: Vec<serde_json::Value> = fixtures
        .into_iter()
        .filter(|def| {
            !strip.contains(
                &def.get("type")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or(""),
            )
        })
        .collect();
    owned.push((
        "data/test/reactions.json".to_owned(),
        serde_json::to_string_pretty(&fixtures).unwrap(),
    ));
    let files: Vec<(&str, &str)> = owned
        .iter()
        .map(|(path, text)| (path.as_str(), text.as_str()))
        .collect();
    common::try_defs_with(&files).unwrap_or_else(|error| panic!("{error}"))
}

static DEFS: LazyLock<Arc<Defs>> = LazyLock::new(|| load(true));
static BARE: LazyLock<Arc<Defs>> = LazyLock::new(|| load(false));
static MINDS: LazyLock<Arc<Defs>> = LazyLock::new(|| load_with(true, true));

// ---- helpers ----------------------------------------------------------------

fn debug(sim: &mut Sim, command: DebugCommand) {
    sim.submit(Command::Debug(command))
        .unwrap_or_else(|error| panic!("{error}"));
}

/// Spawns a person by command, so that a replay makes them too.
fn spawn(sim: &mut Sim, name: &str, x: i32, control: Control) -> EntityId {
    let id = sim.truth().entities().next_id().unwrap();
    let spec = CharacterSpec::of(["core:person"])
        .with("core:person.name", Selection::Text(name.to_owned()));
    debug(
        sim,
        DebugCommand::Spawn {
            spec,
            place: common::on(x, 10),
        },
    );
    debug(
        sim,
        DebugCommand::SetControl {
            entity: id,
            control,
        },
    );
    id
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

fn real(value: f64) -> clash_core::Value {
    clash_core::Value::Real(Real::of(value))
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

/// Two people on flat ground, `ada` at (10, 10) and `bram` east of her
/// at (11, 10), both delegated; `bram` sees `ada` and faces her, holds a
/// knife, and has wits 14: he notices a blow in a seventh of a second.
fn duel(defs: &Arc<Defs>, seed: u64) -> (Sim, EntityId, EntityId) {
    let mut sim = common::sim_over(Arc::clone(defs), seed);
    let ada = spawn(&mut sim, "Ada", 10, Control::Delegated);
    let bram = spawn(&mut sim, "Bram", 11, Control::Delegated);
    arm(&mut sim, ada, "core:knife");
    arm(&mut sim, bram, "core:knife");
    set_attr(&mut sim, bram, "core:wits", 14.0);
    watch(&mut sim, bram, ada);
    sim.pump();
    (sim, ada, bram)
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

/// `ada` swings at `bram`'s right arm with a wind-up of so many seconds
/// and so many joules.
fn swing(sim: &mut Sim, ada: EntityId, bram: EntityId, windup: f64, energy: f64) {
    let params = vec![
        ("target".to_owned(), entity_value(sim, bram)),
        ("part".to_owned(), part_value(sim, bram, "r_arm")),
        ("windup".to_owned(), real(windup)),
        ("energy".to_owned(), real(energy)),
    ];
    act(sim, ada, "core:test_swing", params);
}

/// Advances by so many microseconds. A player asked what to do next
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

fn field(event: &SimEvent, name: &str) -> f64 {
    event
        .body
        .field(name)
        .and_then(|value| value.as_real())
        .unwrap_or_else(|| panic!("{} has no real `{name}`", event.name()))
        .get()
}

fn faults(events: &[SimEvent]) -> Vec<String> {
    events
        .iter()
        .filter(|event| event.name() == "fault")
        .map(|event| format!("{event:?}"))
        .collect()
}

/// The open window whose source is `who`.
fn window_of(sim: &Sim, who: EntityId) -> WindowId {
    sim.windows()
        .iter()
        .find(|(_, window)| window.source == Some(who))
        .map(|(id, _)| id)
        .expect("a window is open")
}

fn noticed_by(events: &[SimEvent], who: EntityId) -> usize {
    named(events, "window_noticed")
        .into_iter()
        .filter(|event| about(event, who))
        .count()
}

// ---- T5: noticing --------------------------------------------------------------

#[test]
fn too_fast_to_notice() {
    // Bram, wits 10, takes 200 ms of his own time to notice. A wind-up of
    // 400 ms is noticed; one of 100 ms closes first.
    let (mut sim, ada, bram) = duel(&DEFS, 3);
    set_attr(&mut sim, bram, "core:wits", 10.0);
    sim.drain_events();
    let opened = sim.now();
    swing(&mut sim, ada, bram, 0.4, 1.0);
    run_for(&mut sim, 199 * MS);
    assert_eq!(noticed_by(&sim.drain_events(), bram), 0, "not yet");
    run_for(&mut sim, 2 * MS);
    let told = sim.drain_events();
    let noticed = named(&told, "window_noticed");
    assert_eq!(noticed.len(), 1);
    assert!(about(noticed[0], bram));
    assert_eq!(noticed[0].tick.as_micros() - opened.as_micros(), 200 * MS);
    assert!((field(noticed[0], "time_left") - 0.2).abs() < 1e-6);
    run_for(&mut sim, 1_000 * MS);
    let closed = sim.drain_events();
    let closed = named(&closed, "window_closed");
    assert_eq!(closed.len(), 1);
    assert_eq!(field(closed[0], "noticed") as i64, 1);

    swing(&mut sim, ada, bram, 0.1, 1.0);
    run_for(&mut sim, 1_000 * MS);
    let told = sim.drain_events();
    assert_eq!(noticed_by(&told, bram), 0, "the fast one closes first");
    let closed = named(&told, "window_closed");
    assert_eq!(field(closed[0], "noticed") as i64, 0);
    assert!(faults(&told).is_empty(), "{:?}", faults(&told));

    // On a clock four times as fast he needs 50 ms of universal time,
    // and notices both.
    sim.set_time_rate(bram, Some(Real::of(4.0)), Cause::DEBUG)
        .unwrap();
    for windup in [0.4, 0.1] {
        let opened = sim.now();
        swing(&mut sim, ada, bram, windup, 1.0);
        run_for(&mut sim, 1_000 * MS);
        let told = sim.drain_events();
        let noticed = named(&told, "window_noticed");
        assert_eq!(noticed.len(), 1, "a wind-up of {windup} s");
        assert_eq!(noticed[0].tick.as_micros() - opened.as_micros(), 50 * MS);
    }
}

#[test]
fn a_window_closes_when_its_action_ends_without_reaching_the_deadline() {
    let (mut sim, ada, bram) = duel(&DEFS, 4);
    swing(&mut sim, ada, bram, 0.4, 1.0);
    run_for(&mut sim, 50 * MS);
    let window = window_of(&sim, ada);
    let candidate = *sim.windows().get(window).unwrap().candidate(bram).unwrap();
    assert!(matches!(candidate.state, Notice::Pending(_)));
    let running = sim.actions_of(ada)[0];
    sim.submit(Command::Cancel {
        entity: ada,
        action: running,
        abandon: false,
    })
    .unwrap();
    assert!(sim.windows().get(window).is_none());
    run_for(&mut sim, 1_000 * MS);
    let told = sim.drain_events();
    assert_eq!(
        noticed_by(&told, bram),
        0,
        "the wait to notice was cancelled"
    );
    assert_eq!(named(&told, "window_closed").len(), 1);
}

// ---- T5: reacting ----------------------------------------------------------

#[test]
fn a_candidate_committed_to_something_has_no_reaction_available() {
    let (mut sim, ada, bram) = duel(&DEFS, 5);
    // Both arms held in a phase that cannot be broken off.
    act(&mut sim, bram, "core:test_commit", Vec::new());
    swing(&mut sim, ada, bram, 0.4, 1.0);
    run_for(&mut sim, 250 * MS);
    let window = window_of(&sim, ada);
    assert!(
        sim.windows()
            .get(window)
            .unwrap()
            .candidate(bram)
            .unwrap()
            .state
            .noticed()
    );
    let parry = action(&sim, "core:test_parry");
    let options = sim.reaction_options(bram, window);
    let option = options
        .iter()
        .find(|option| option.action == parry)
        .unwrap();
    let refused = option.refused.as_ref().expect("no parry is available");
    assert_eq!(refused.code, "engine:reaction_cannot_cancel");
    let hash = sim.state_hash();
    let refused = sim
        .submit(Command::React {
            entity: bram,
            window,
            action: parry,
            params: Vec::new(),
        })
        .unwrap_err();
    assert_eq!(refused.reason.code, "engine:reaction_cannot_cancel");
    assert_eq!(sim.state_hash(), hash, "a refusal changes nothing");

    // Both arms held in a phase that may be broken off: the parry is
    // available, in time, and starting it cancels what held them.
    let (mut sim, ada, bram) = duel(&DEFS, 5);
    act(&mut sim, bram, "core:test_loose", Vec::new());
    swing(&mut sim, ada, bram, 0.4, 1.0);
    run_for(&mut sim, 250 * MS);
    let window = window_of(&sim, ada);
    let options = sim.reaction_options(bram, window);
    let option = options
        .iter()
        .find(|option| option.action == parry)
        .unwrap();
    assert!(option.refused.is_none(), "{:?}", option.refused);
    assert!(option.in_time, "ready 80 ms after 250 ms of 400");
    sim.drain_events();
    sim.submit(Command::React {
        entity: bram,
        window,
        action: parry,
        params: Vec::new(),
    })
    .unwrap();
    let told = sim.drain_events();
    assert_eq!(named(&told, "action_cancelled").len(), 1);
    assert_eq!(named(&told, "action_started").len(), 1);
    // The window and the action know the reaction answers it.
    let state = sim
        .windows()
        .get(window)
        .unwrap()
        .candidate(bram)
        .unwrap()
        .state;
    assert!(matches!(state, Notice::Reacted(_)));
}

#[test]
fn with_no_rule_listening_nothing_happens() {
    // Directly controlled, and nothing listens for `window_noticed`: he
    // notices, and is asked nothing and does nothing.
    let (mut sim, ada, bram) = duel(&BARE, 6);
    debug(
        &mut sim,
        DebugCommand::SetControl {
            entity: bram,
            control: Control::Direct,
        },
    );
    sim.submit(Command::Wait {
        entity: bram,
        dur: clash_core::Dur::from_secs(5),
    })
    .unwrap();
    sim.drain_events();
    swing(&mut sim, ada, bram, 0.4, 1.0);
    let told = loop {
        match sim.advance(AdvanceLimit::Until(Tick(
            sim.now().as_micros() + 1_000 * MS,
        ))) {
            Advance::Reached(_) => break sim.drain_events(),
            Advance::Yielded => {}
            other => panic!("nobody is asked anything: {other:?}"),
        }
    };
    assert_eq!(noticed_by(&told, bram), 1);
    assert!(sim.actions_of(bram).is_empty());
    assert!(named(&told, "decision_requested").is_empty());
    assert!(named(&told, "strike_defended").is_empty());
    assert_eq!(named(&told, "strike_landed").len(), 1);
}

/// Advances until the simulation asks a decision, which it returns.
fn until_asked(sim: &mut Sim) -> clash_sim::decision::DecisionRequest {
    for _ in 0..1_000 {
        match sim.advance(AdvanceLimit::UntilDecision) {
            Advance::NeedInput(request) => return request,
            Advance::Yielded => {}
            other => panic!("expected a decision: {other:?}"),
        }
    }
    panic!("no decision");
}

/// A duel in which Bram is played: he is asked what to do, and waits.
fn played(seed: u64) -> (Sim, EntityId, EntityId) {
    let (mut sim, ada, bram) = duel(&DEFS, seed);
    debug(
        &mut sim,
        DebugCommand::SetControl {
            entity: bram,
            control: Control::Direct,
        },
    );
    sim.submit(Command::Wait {
        entity: bram,
        dur: clash_core::Dur::from_secs(5),
    })
    .unwrap();
    (sim, ada, bram)
}

#[test]
fn a_player_controlled_defender_is_asked_and_the_reaction_chosen_runs() {
    let (mut sim, ada, bram) = played(7);
    sim.drain_events();
    swing(&mut sim, ada, bram, 0.3, 180.0);
    let request = until_asked(&mut sim);
    assert_eq!(request.entity, bram);
    let DecisionKind::Reaction {
        window,
        deadline,
        options,
    } = request.kind
    else {
        panic!("a reaction is asked: {:?}", request.kind);
    };
    // Asked when he noticed, at a seventh of a second, with the rest of
    // the wind-up left; time does not move until he answers.
    let asked_at = sim.now();
    assert!(deadline.is_some_and(|deadline| deadline > asked_at));
    let left = sim.window_time_left(window, bram).unwrap();
    assert!(left.as_secs_real() > Real::of(0.1));
    let parry = action(&sim, "core:parry");
    let dodge = action(&sim, "core:dodge");
    let shown: Vec<ActionDefId> = options.iter().map(|option| option.action).collect();
    assert!(
        shown.contains(&parry) && shown.contains(&dodge),
        "{shown:?}"
    );
    let option = options
        .iter()
        .find(|option| option.action == parry)
        .unwrap();
    assert!(option.in_time && option.refused.is_none(), "{option:?}");
    assert_eq!(option.label, "Parry");
    assert!(matches!(
        sim.advance(AdvanceLimit::UntilDecision),
        Advance::NeedInput(_)
    ));
    assert_eq!(sim.now(), asked_at);

    // He parries; the guard is up when the blow lands.
    sim.submit(Command::React {
        entity: bram,
        window,
        action: parry,
        params: Vec::new(),
    })
    .unwrap();
    assert!(sim.pending_decision().is_none());
    run_for(&mut sim, 600 * MS);
    let told = sim.drain_events();
    let defended = named(&told, "strike_defended");
    assert_eq!(defended.len(), 1, "the parry was tried");
    assert_eq!(defended[0].subject("defender"), Some(Subject::Entity(bram)));
    assert!(faults(&told).is_empty(), "{:?}", faults(&told));
}

#[test]
fn a_player_who_passes_does_not_react() {
    let (mut sim, ada, bram) = played(8);
    swing(&mut sim, ada, bram, 0.3, 1.0);
    let request = until_asked(&mut sim);
    let DecisionKind::Reaction { window, .. } = request.kind else {
        panic!("a reaction is asked");
    };
    sim.submit(Command::Pass {
        entity: bram,
        window,
    })
    .unwrap();
    // Not asked twice.
    assert!(
        sim.submit(Command::Pass {
            entity: bram,
            window
        })
        .is_err()
    );
    run_for(&mut sim, 600 * MS);
    let told = sim.drain_events();
    assert!(named(&told, "strike_defended").is_empty());
    assert_eq!(named(&told, "strike_landed").len(), 1);
}

#[test]
fn a_save_and_a_load_while_a_window_is_open_change_nothing() {
    let (mut sim, ada, bram) = duel(&DEFS, 9);
    attach(&mut sim, bram, "core:test_parry_on_notice");
    swing(&mut sim, ada, bram, 0.3, 180.0);
    // Open, and Bram still noticing it.
    run_for(&mut sim, 100 * MS);
    assert!(!sim.windows().is_empty());
    let (mut loaded, unplaced) = common::save_and_load(&sim);
    assert!(unplaced.is_empty());
    assert_eq!(loaded.state_hash(), sim.state_hash());
    for _ in 0..5 {
        run_for(&mut sim, 100 * MS);
        run_for(&mut loaded, 100 * MS);
        assert_eq!(loaded.state_hash(), sim.state_hash());
    }
    let told = loaded.drain_events();
    assert_eq!(named(&told, "strike_defended").len(), 1);
}

#[test]
fn a_replay_reproduces_the_hash() {
    // A save and the log that follows it replay to the same state
    // (invariant T6): the player is asked, parries, and is asked again.
    let (mut sim, ada, bram) = played(10);
    sim.take_input_log();
    let (mut replayed, unplaced) = common::save_and_load(&sim);
    assert!(unplaced.is_empty());
    swing(&mut sim, ada, bram, 0.3, 180.0);
    let request = until_asked(&mut sim);
    let DecisionKind::Reaction { window, .. } = request.kind else {
        panic!("a reaction is asked");
    };
    let parry = action(&sim, "core:parry");
    sim.submit(Command::React {
        entity: bram,
        window,
        action: parry,
        params: Vec::new(),
    })
    .unwrap();
    run_for(&mut sim, 1_000 * MS);
    replayed.replay(sim.input_log()).unwrap();
    let left = sim.now().as_micros() - replayed.now().as_micros();
    run_for(&mut replayed, left);
    assert_eq!(replayed.state_hash(), sim.state_hash());
}

// ---- T5: who is offered a window ------------------------------------------------

/// Whether the first answers for the second: Bram guards Ada.
fn guards(_sim: &Sim, holder: EntityId, toward: EntityId) -> bool {
    holder.index() == 1 && toward.index() == 0
}

#[test]
fn who_is_offered_a_window() {
    // Cleo strikes at Ada. Bram, who guards her and stands within reach,
    // is offered the window and may answer it only with a reaction that
    // answers for someone else; Dan, a player within reach, is offered
    // it too; Eve, a player beyond reach, is not.
    let mut sim = common::sim_over(Arc::clone(&DEFS), 11);
    sim.set_intervenes(Some(guards));
    let ada = spawn(&mut sim, "Ada", 10, Control::Delegated);
    let bram = spawn(&mut sim, "Bram", 12, Control::Delegated);
    let cleo = spawn(&mut sim, "Cleo", 11, Control::Delegated);
    let dan = spawn(&mut sim, "Dan", 13, Control::Delegated);
    let eve = spawn(&mut sim, "Eve", 16, Control::Delegated);
    assert_eq!((ada.index(), bram.index()), (0, 1));
    for who in [dan, eve] {
        debug(
            &mut sim,
            DebugCommand::SetControl {
                entity: who,
                control: Control::Direct,
            },
        );
        sim.submit(Command::Wait {
            entity: who,
            dur: clash_core::Dur::from_secs(5),
        })
        .unwrap();
    }
    arm(&mut sim, cleo, "core:knife");
    swing(&mut sim, cleo, ada, 0.4, 1.0);
    let window = window_of(&sim, cleo);
    let offered: Vec<(EntityId, clash_sim::action::Offered)> = sim
        .windows()
        .get(window)
        .unwrap()
        .candidates
        .iter()
        .map(|candidate| (candidate.entity, candidate.offered))
        .collect();
    use clash_sim::action::Offered;
    assert_eq!(
        offered,
        [
            (ada, Offered::Named),
            (bram, Offered::Intervenes),
            (dan, Offered::Controlled)
        ]
    );
    // Bram sees Cleo and notices; of the reactions only the one that
    // answers for another is his to take, and the parry is refused.
    watch(&mut sim, bram, cleo);
    let window = {
        let running = sim.actions_of(cleo)[0];
        sim.submit(Command::Cancel {
            entity: cleo,
            action: running,
            abandon: false,
        })
        .unwrap();
        swing(&mut sim, cleo, ada, 0.4, 1.0);
        window_of(&sim, cleo)
    };
    run_for(&mut sim, 300 * MS);
    let options = sim.reaction_options(bram, window);
    let shield = action(&sim, "core:test_shield_him");
    assert_eq!(
        options
            .iter()
            .map(|option| option.action)
            .collect::<Vec<_>>(),
        [shield]
    );
    let parry = action(&sim, "core:test_parry");
    let refused = sim
        .react(bram, window, parry, &Vec::new(), Cause::DEBUG)
        .unwrap_err();
    assert_eq!(refused.code, "engine:reaction_not_on_behalf");
}

// ---- C4: the worked exchanges of 07 §14 ------------------------------------------

/// The parry of 07 §14: Bram, strength 10, holds 600 J and deflects 0.9.
fn exchange(seed: u64, strength: f64, wits: f64) -> (Sim, EntityId, EntityId, Vec<SimEvent>) {
    let (mut sim, ada, bram) = duel(&DEFS, seed);
    set_attr(&mut sim, ada, "core:strength", strength);
    set_attr(&mut sim, bram, "core:wits", wits);
    attach(&mut sim, bram, "core:test_parry_on_notice");
    sim.drain_events();
    let energy = strength * 5.0;
    swing(&mut sim, ada, bram, 0.3, energy);
    run_for(&mut sim, 1_000 * MS);
    let told = sim.drain_events();
    (sim, ada, bram, told)
}

fn knife_of(sim: &Sim, who: EntityId) -> Option<EntityId> {
    sim.truth().contents(who).iter().copied().find(|item| {
        sim.components()
            .get::<clash_sim::item::Item>(*item)
            .is_some()
    })
}

#[test]
fn parry_in_time() {
    // A slash of 180 J. Bram notices at about 140 ms, his guard is up at
    // about 220 ms, the blow lands at 300 ms: 162 J is turned aside and
    // the other 18 J lands on the flat of his blade, which steel does not
    // notice. Nothing reaches him.
    let (sim, _ada, bram, told) = exchange(13, 36.0, 14.0);
    assert!(faults(&told).is_empty(), "{:?}", faults(&told));
    let defended = named(&told, "strike_defended");
    assert_eq!(defended.len(), 1);
    assert!((field(defended[0], "turned") - 162.0).abs() < 1e-6);
    let stopped = named(&told, "strike_stopped");
    assert_eq!(stopped.len(), 1, "nothing was delivered");
    assert!(named(&told, "strike_landed").is_empty());
    let recoiled = named(&told, "strike_recoiled");
    assert!(
        (field(recoiled[0], "energy") - 18.0).abs() < 1e-6,
        "the blade gave it back"
    );
    assert!(sim.components().body.get(bram).unwrap().wounds().is_empty());
    let knife = knife_of(&sim, bram).unwrap();
    assert_eq!(
        sim.components()
            .get::<clash_sim::item::Item>(knife)
            .unwrap()
            .damage(),
        0
    );
}

#[test]
fn broken_blade() {
    // The same, with Ada a hundred times stronger (400 kJ): 540 J turned
    // aside, the rest on Bram's blade, which breaks; nearly all goes on
    // to the arm, which is severed, and the push launches him.
    let (sim, _ada, bram, told) = exchange(14, 80_000.0, 14.0);
    let defended = named(&told, "strike_defended");
    assert_eq!(defended.len(), 1);
    assert!((field(defended[0], "turned") - 540.0).abs() < 1e-6);
    assert!(
        knife_of(&sim, bram).is_none_or(|knife| sim
            .components()
            .get::<clash_sim::item::Item>(knife)
            .unwrap()
            .is_broken()),
        "the blade broke"
    );
    let landed = named(&told, "strike_landed");
    assert_eq!(landed.len(), 1, "the blow came through");
    // The blade took a few tens of kilojoules and passed the rest.
    assert!(field(landed[0], "energy") > 350_000.0, "{:?}", landed[0]);
    assert!(!named(&told, "part_severed").is_empty(), "the arm came off");
}

#[test]
fn too_slow() {
    // With wits 6 Bram needs a third of a second: the window closes
    // before he knows it opened, and there is no parry at all.
    let (sim, _ada, bram, told) = exchange(15, 36.0, 6.0);
    assert_eq!(noticed_by(&told, bram), 0);
    assert!(named(&told, "strike_defended").is_empty());
    assert_eq!(named(&told, "strike_landed").len(), 1);
    assert!(!sim.components().body.get(bram).unwrap().wounds().is_empty());
}

#[test]
fn energy_is_accounted_for_through_a_guard() {
    // Invariant C2 through the defence step: what was turned aside,
    // absorbed by blade and body, pushed, returned and passed on adds up
    // to no more than what was put in.
    for (seed, strength) in [(16, 36.0), (17, 400.0), (18, 4_000.0), (19, 80_000.0)] {
        let (mut sim, ada, bram) = duel(&DEFS, seed);
        attach(&mut sim, bram, "core:test_parry_on_notice");
        let id = action(&sim, "core:test_swing");
        swing(&mut sim, ada, bram, 0.3, strength * 5.0);
        run_for(&mut sim, 290 * MS);
        // Strike directly at the same instant, with the guard up.
        let def = sim.defs().get::<ActionDef>(id);
        let locals: Vec<Value> = def
            .locals()
            .iter()
            .map(|local| match local.name.as_str() {
                "target" => sim.subject_value(Subject::Entity(bram)),
                "energy" => Value::Real(Real::of(strength * 5.0)),
                "push" => Value::Real(Real::of(0.1)),
                _ => Value::Null,
            })
            .collect();
        let args = clash_sim::combat::StrikeArgs {
            target: Some(bram),
            at: None,
            part: None,
            effort: Real::ONE,
            sure: true,
        };
        let tally = clash_sim::combat::strike::strike(
            &mut sim,
            ada,
            id,
            0,
            &args,
            &locals,
            Cause::DEBUG.by(ada),
        )
        .unwrap();
        assert!(tally.turned > Real::ZERO, "the guard turned some of it");
        assert!(
            tally.accounted() <= tally.put_in * Real::of(1.000_001),
            "{tally:?}"
        );
    }
}

// ---- C4: in force, leaving, slipping, guards ------------------------------------

#[test]
fn a_defence_applies_only_in_force_at_impact() {
    // Invariant C3: a parry started too late is not up when the blow
    // lands. Bram reacts by hand at 250 ms of a 300 ms wind-up: his guard
    // would be up at 330 ms.
    let (mut sim, ada, bram) = duel(&DEFS, 20);
    swing(&mut sim, ada, bram, 0.3, 180.0);
    run_for(&mut sim, 250 * MS);
    let window = window_of(&sim, ada);
    let parry = action(&sim, "core:test_parry");
    let options = sim.reaction_options(bram, window);
    let option = options
        .iter()
        .find(|option| option.action == parry)
        .unwrap();
    assert!(!option.in_time, "it would be ready after the blow lands");
    sim.react(bram, window, parry, &Vec::new(), Cause::DEBUG)
        .unwrap();
    sim.drain_events();
    run_for(&mut sim, 500 * MS);
    let told = sim.drain_events();
    assert!(named(&told, "strike_defended").is_empty());
    assert_eq!(named(&told, "strike_landed").len(), 1);
}

#[test]
fn a_defender_who_steps_out_of_reach_is_missed_without_a_contest() {
    let (mut sim, ada, bram) = duel(&DEFS, 21);
    attach(&mut sim, bram, "core:step_back_on_notice");
    sim.drain_events();
    swing(&mut sim, ada, bram, 0.3, 180.0);
    run_for(&mut sim, 600 * MS);
    let told = sim.drain_events();
    assert_eq!(
        sim.truth().location(bram).unwrap().cell.x,
        12,
        "he stepped back, east"
    );
    let missed = named(&told, "strike_missed");
    assert_eq!(missed.len(), 1);
    assert_eq!(
        missed[0]
            .body
            .field("why")
            .and_then(|why| why.plain().cloned()),
        Some(clash_core::Value::Str("reach".to_owned()))
    );
    assert!(named(&told, "strike_defended").is_empty(), "no contest");
}

#[test]
fn a_defender_who_slips_a_blow_is_missed() {
    let (mut sim, ada, bram) = duel(&DEFS, 22);
    attach(&mut sim, bram, "core:test_slip_on_notice");
    sim.drain_events();
    swing(&mut sim, ada, bram, 0.3, 180.0);
    run_for(&mut sim, 600 * MS);
    let told = sim.drain_events();
    let missed = named(&told, "strike_missed");
    assert_eq!(missed.len(), 1);
    assert_eq!(
        missed[0]
            .body
            .field("why")
            .and_then(|why| why.plain().cloned()),
        Some(clash_core::Value::Str("defence".to_owned()))
    );
    assert_eq!(named(&told, "strike_defended").len(), 1);
}

#[test]
fn a_guard_held_up_defends_until_it_is_let_go() {
    let (mut sim, ada, bram) = duel(&DEFS, 23);
    set_attr(&mut sim, bram, "core:wits", 1_000.0);
    act(&mut sim, bram, "core:guard", Vec::new());
    run_for(&mut sim, 300 * MS);
    assert!(clash_sim::combat::defending(&sim, bram));
    assert_eq!(
        sim.property_named(Subject::Entity(bram), "defending", &[]),
        Value::Bool(true)
    );
    assert_eq!(
        sim.property_named(Subject::Entity(ada), "defending", &[]),
        Value::Bool(false)
    );
    sim.drain_events();
    swing(&mut sim, ada, bram, 0.3, 180.0);
    run_for(&mut sim, 600 * MS);
    let told = sim.drain_events();
    assert_eq!(named(&told, "strike_defended").len(), 1, "the guard met it");
    // Held, it meets the next too.
    swing(&mut sim, ada, bram, 0.3, 180.0);
    run_for(&mut sim, 600 * MS);
    assert_eq!(named(&sim.drain_events(), "strike_defended").len(), 1);
    // Let go, it meets nothing.
    let guard = sim.actions_of(bram)[0];
    sim.submit(Command::Cancel {
        entity: bram,
        action: guard,
        abandon: true,
    })
    .unwrap();
    assert!(!clash_sim::combat::defending(&sim, bram));
    swing(&mut sim, ada, bram, 0.3, 180.0);
    run_for(&mut sim, 600 * MS);
    assert!(named(&sim.drain_events(), "strike_defended").is_empty());
}

#[test]
fn a_defence_may_protect_someone_beside_the_defender() {
    // Cleo strikes at Ada; Bram, beside her, puts his arm in the way.
    let mut sim = common::sim_over(Arc::clone(&DEFS), 24);
    sim.set_intervenes(Some(guards));
    let ada = spawn(&mut sim, "Ada", 10, Control::Delegated);
    let bram = spawn(&mut sim, "Bram", 11, Control::Delegated);
    let cleo = spawn(&mut sim, "Cleo", 9, Control::Delegated);
    arm(&mut sim, cleo, "core:knife");
    set_attr(&mut sim, bram, "core:wits", 1_000.0);
    watch(&mut sim, bram, cleo);
    sim.pump();
    swing(&mut sim, cleo, ada, 0.3, 180.0);
    run_for(&mut sim, 50 * MS);
    let window = window_of(&sim, cleo);
    let shield = action(&sim, "core:test_shield_him");
    let params = vec![("ward".to_owned(), entity_value(&sim, ada))];
    sim.react(bram, window, shield, &params, Cause::DEBUG)
        .unwrap();
    sim.drain_events();
    run_for(&mut sim, 600 * MS);
    let told = sim.drain_events();
    let defended = named(&told, "strike_defended");
    assert_eq!(defended.len(), 1);
    assert_eq!(defended[0].subject("defender"), Some(Subject::Entity(bram)));
    assert_eq!(defended[0].subject("target"), Some(Subject::Entity(ada)));
    assert!((field(defended[0], "turned") - 90.0).abs() < 1e-6);
}

// ---- minds react ------------------------------------------------------------------

#[test]
fn a_guards_mind_answers_a_blow_by_its_data_and_a_townspersons_does_not() {
    // Bram thinks by the bundled `guard` mind, whose `reactions` block
    // first: he takes the blow on his forearm. Cleo thinks as a
    // townsperson and answers nothing.
    for (templates, defended) in [
        (&["core:person", "core:guard"][..], true),
        (&["core:person"][..], false),
    ] {
        let mut sim = common::sim_over(Arc::clone(&MINDS), 25);
        let ada = spawn(&mut sim, "Ada", 10, Control::Delegated);
        let id = sim.truth().entities().next_id().unwrap();
        debug(
            &mut sim,
            DebugCommand::Spawn {
                spec: CharacterSpec::of(templates.iter().copied()),
                place: common::on(11, 10),
            },
        );
        let bram = id;
        arm(&mut sim, ada, "core:knife");
        set_attr(&mut sim, bram, "core:wits", 14.0);
        watch(&mut sim, bram, ada);
        sim.pump();
        sim.drain_events();
        swing(&mut sim, ada, bram, 0.3, 50.0);
        run_for(&mut sim, 600 * MS);
        let told = sim.drain_events();
        assert_eq!(noticed_by(&told, bram), 1, "{templates:?} noticed");
        let blocked: Vec<&SimEvent> = named(&told, "strike_defended")
            .into_iter()
            .filter(|event| {
                event
                    .body
                    .field("defence")
                    .and_then(|field| field.plain().cloned())
                    == Some(clash_core::Value::Str("core:block".to_owned()))
            })
            .collect();
        assert_eq!(!blocked.is_empty(), defended, "{templates:?}");
    }
}

// ---- the documents state what is built ----------------------------------------

#[test]
fn documents_state_every_window_event_verb_and_defence_field() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../docs");
    let three = std::fs::read_to_string(root.join("03-time-and-actions.md")).unwrap();
    let seven = std::fs::read_to_string(root.join("07-combat.md")).unwrap();
    let registry = DEFS.expr_registry();
    for info in clash_sim::action::WINDOW_EVENTS {
        assert!(registry.event_id(info.name).is_some(), "{}", info.name);
        assert!(
            three.contains(&format!("| `{}`", info.name)),
            "03 §13: {}",
            info.name
        );
        for (field, _, _) in info.fields {
            assert!(
                three.contains(&format!("`{field}`")),
                "{}.{field}",
                info.name
            );
        }
    }
    for verb in clash_sim::action::react::verbs() {
        assert!(registry.verb_id(verb.name).is_some(), "{}", verb.name);
        assert!(
            three.contains(&format!("| `{}`", verb.name)),
            "03 §13: {}",
            verb.name
        );
    }
    for name in [
        "window_open",
        "window_source",
        "window_kind_of",
        "window_time_left",
        "away_from",
    ] {
        assert!(registry.function_id(name).is_some(), "{name}");
        assert!(three.contains(&format!("`{name}(")), "03: {name}");
    }
    for field in [
        "during", "form", "with", "protects", "covers", "handles", "contest", "deflect", "holds",
        "brace", "evasion",
    ] {
        assert!(seven.contains(&format!("| `{field}`")), "07 §5.1: {field}");
    }
    assert!(seven.contains("| `strike_defended`"));
    assert!(seven.contains("| `defence_resolving` (hook)"));
    assert!(three.contains("`React`") && three.contains("`Pass`"));
}
