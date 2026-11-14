//! Game scenarios (13 §3 to §5, §9): beginning a game, settings that are
//! locked, rules in force only in their scenario, objectives kept
//! current without polling (invariant Z4), missions with ordering and
//! expiry, endings, and a game saved and loaded in the middle of a
//! mission.

mod common;

use std::sync::Arc;

use clash_core::{Dur, EntityId, Value};
use clash_data::Defs;
use clash_sim::scenario::{
    BeginScenario, MissionState, ObjectiveState, fresh_objective_state, make_world, objective_state,
};
use clash_sim::template::CharacterSpec;
use clash_sim::{Advance, AdvanceLimit, AppRequest, Command, DebugCommand, Place, Sim, Subject};
use clash_world::{Cause, Cosmos, MemoryStore, TractIo, WorldDefs};

/// Two scenarios on the flat test land, a setting, rules for one of
/// them, variables their objectives read and a mission.
const SCENARIOS: &str = r#"[
  { "type": "setting", "id": "pace", "name": "Pace", "kind": "choice",
    "options": ["slow", "quick"], "default": "slow" },
  { "type": "setting", "id": "bold", "kind": "flag", "default": false },

  { "type": "variable", "id": "marks", "scope": "global", "value": "int", "default": 0, "watch": true },
  { "type": "variable", "id": "ruined", "scope": "global", "value": "bool", "default": false, "watch": true },
  { "type": "variable", "id": "moves", "scope": "global", "value": "int", "default": 0 },
  { "type": "variable", "id": "begun_with", "scope": "global", "value": "string" },

  { "type": "rule_group", "id": "alpha_rules", "enabled": false },
  { "type": "eoc", "id": "count_moves", "on": "entity_relocated", "group": "alpha_rules",
    "do": [ { "add": { "global.var.moves": 1 } } ] },

  { "type": "scenario", "id": "alpha", "name": "Alpha",
    "worlds": [ { "realm": "testland", "seed": "ask" } ],
    "options": ["bold"],
    "settings": { "core:pace": "quick" },
    "locked": ["core:pace"],
    "rules": ["alpha_rules"],
    "start": { "characters": [ { "template": "person" } ],
               "at": [ { "name": "West", "cell": [10, 10] }, { "name": "East", "cell": [40, 12] } ],
               "time": { "day": 2, "at": "06:30" },
               "do": [ { "set": { "global.var.begun_with": "alpha" } } ] },
    "objectives": [
      { "id": "gather", "text": "Gather three marks.",
        "done": "global.var.marks >= 3", "failed": "global.var.ruined" },
      { "id": "report", "text": "Report back.", "after": ["gather"],
        "done": "global.var.marks >= 5" },
      { "id": "whole", "text": "Do it all." },
      { "id": "part_one", "text": "Part one.", "parent": "whole", "done": "global.var.marks >= 1" },
      { "id": "part_two", "text": "Part two.", "parent": "whole", "done": "global.var.marks >= 2" } ],
    "ends": [ { "if": "global.var.marks >= 9", "as": "won" } ],
    "saving": { "policy": "single", "on_party_gone": "end" } },

  { "type": "scenario", "id": "beta", "name": "Beta",
    "worlds": [ { "realm": "testland", "seed": 5 } ],
    "start": { "characters": [ { "template": "person" } ],
               "at": [ { "cell": [12, 10] } ] },
    "ends": [ { "on": "objective_changed", "filter": { "id": "fetch", "state": "done" }, "as": "fetched" } ],
    "saving": { "policy": "free", "on_party_gone": { "then": "continue", "by": ["create"] } } },

  { "type": "scenario", "id": "gamma", "name": "Gamma",
    "worlds": [ { "realm": "test_city" } ],
    "start": { "characters": [ { "template": "person" } ], "at": [ { "cell": [5, 5] } ] } },

  { "type": "variable", "id": "why", "scope": "global", "value": "string" },
  { "type": "variable", "id": "told", "scope": "global", "value": "string" },
  { "type": "eoc", "id": "give_up_later",
    "do": [ { "after": "1 s", "name": "give_up", "owner": "global",
              "do": [ { "set": { "global.var.why": "left" } },
                      { "fail_objective": "hold" } ] } ] },
  { "type": "scenario", "id": "delta", "name": "Delta",
    "worlds": [ { "realm": "testland", "seed": 5 } ],
    "start": { "characters": [ { "template": "person" } ], "at": [ { "cell": [12, 10] } ],
               "do": [ { "run": "core:give_up_later" } ] },
    "objectives": [
      { "id": "hold", "text": "Hold on.",
        "on_failed": [ { "set": { "global.var.told": { "expr": "global.var.why ?? 'unset'" } } } ] } ] },

  { "type": "mission", "id": "errand", "name": "Errand for {$who}",
    "params": { "who": "string", "count": "int" },
    "objectives": [
      { "id": "fetch", "text": "Fetch {$count} marks.", "done": "global.var.marks >= $count" },
      { "id": "back", "text": "Come back.", "after": ["fetch"], "done": "global.var.marks >= $count + 1" } ],
    "expires": "1 h" }
]"#;

fn defs() -> Arc<Defs> {
    common::life_defs(SCENARIOS)
}

/// A game begun from a scenario, as the application begins one.
fn begun(
    defs: &Arc<Defs>,
    scenario: &str,
    seed: u64,
    place: u32,
    options: Vec<(String, Value)>,
) -> (Sim, EntityId) {
    let world = WorldDefs::from_defs(defs).expect("the world's tables build");
    let made = make_world(defs, world, scenario, seed).expect("the world is made");
    let mut sim = Sim::from_parts(Arc::clone(defs), made.cosmos, made.timeline, seed).with_io(
        clash_gen::generators(&WorldDefs::from_defs(defs).unwrap()),
        Box::new(MemoryStore::new()),
    );
    let player = sim.truth().entities().next_id().unwrap();
    sim.submit(Command::Debug(DebugCommand::Spawn {
        spec: CharacterSpec::of(["core:person"]),
        place: Place::Stored,
    }))
    .expect("the character is made");
    sim.submit(Command::BeginScenario(BeginScenario {
        scenario: scenario.to_owned(),
        party: vec![player],
        place,
        options,
    }))
    .expect("the game begins");
    (sim, player)
}

fn var(sim: &Sim, name: &str) -> clash_expr::Value {
    sim.var(Subject::Global, name)
}

fn set_var(sim: &mut Sim, name: &str, value: impl Into<clash_expr::Value>) {
    sim.set_var(Subject::Global, name, value.into(), Cause::DEBUG)
        .expect("the variable is set");
}

/// Runs the simulation on by a span, carrying out nothing it asks of the
/// application but collecting what it asks.
fn run(sim: &mut Sim, by: Dur) -> Vec<AppRequest> {
    let until = sim.now() + by;
    let mut asked = Vec::new();
    for _ in 0..10_000 {
        match sim.advance(AdvanceLimit::Until(until)) {
            Advance::Request(request) => asked.push(request),
            Advance::Reached(_) => break,
            Advance::NeedInput(request) => {
                sim.submit(Command::Wait {
                    entity: request.entity,
                    dur: until.since(sim.now()).max(Dur::from_millis(1)),
                })
                .expect("the wait is taken");
            }
            Advance::Yielded => {}
        }
    }
    asked
}

/// Invariant Z4: every objective of the scenario that is still active is
/// what a fresh evaluation says; and every settled one is as it was.
fn check_z4(sim: &Sim, ids: &[&str], settled: &mut Vec<(String, ObjectiveState)>) {
    for id in ids {
        let state = objective_state(sim, None, id).unwrap();
        if let Some((_, was)) = settled.iter().find(|(have, _)| have == id) {
            assert_eq!(state, *was, "`{id}` changed after it settled");
            continue;
        }
        match state {
            ObjectiveState::Active => {
                assert_eq!(
                    fresh_objective_state(sim, None, id),
                    Some(ObjectiveState::Active),
                    "`{id}` is active and a fresh evaluation settles it"
                );
            }
            ObjectiveState::Done | ObjectiveState::Failed => {
                settled.push(((*id).to_owned(), state));
            }
            ObjectiveState::Hidden => {}
        }
    }
}

#[test]
fn a_game_begins_where_and_when_its_scenario_says_with_its_setup_applied() {
    let defs = defs();
    let (sim, player) = begun(&defs, "core:alpha", 9, 1, vec![]);
    let game = sim
        .scenario()
        .game
        .as_ref()
        .expect("a game is being played");
    assert_eq!(game.scenario, "core:alpha");
    assert_eq!(game.party, [player]);
    assert_eq!(game.place, 1);
    let at = sim
        .truth()
        .location(player)
        .expect("the character is in the world");
    assert_eq!((at.cell.x, at.cell.y), (40, 12), "the place chosen");
    // Day two at half past six: a day and six and a half hours on the
    // realm's clock (the test sky starts at midnight).
    let clock = sim.truth().cosmos().grid(at.grid).unwrap().clock();
    let local = sim.timeline().local_now(clock).unwrap();
    assert_eq!(local, Dur::from_secs(24 * 3600 + 6 * 3600 + 30 * 60));
    assert_eq!(
        var(&sim, "core:begun_with"),
        clash_expr::Value::text("alpha"),
        "`start.do` ran"
    );
    assert_eq!(
        sim.settings().get("core:pace"),
        Some(&Value::Str("quick".to_owned())),
        "the scenario's settings"
    );
    assert_eq!(
        sim.truth().entity(player).unwrap().control(),
        clash_sim::Control::Direct
    );
}

#[test]
fn the_same_answers_give_the_same_state_and_others_do_not() {
    let defs = defs();
    let one = begun(
        &defs,
        "core:alpha",
        9,
        0,
        vec![("core:bold".to_owned(), Value::Bool(true))],
    )
    .0;
    let two = begun(
        &defs,
        "core:alpha",
        9,
        0,
        vec![("core:bold".to_owned(), Value::Bool(true))],
    )
    .0;
    assert_eq!(one.state_hash(), two.state_hash());
    assert_eq!(
        one.settings().get("core:bold"),
        Some(&Value::Bool(true)),
        "the option answered"
    );
    let other_seed = begun(&defs, "core:alpha", 10, 0, vec![]).0;
    assert_ne!(one.state_hash(), other_seed.state_hash());
    let other_place = begun(&defs, "core:alpha", 9, 1, vec![]).0;
    assert_ne!(one.state_hash(), other_place.state_hash());
    // A fixed seed is the scenario's, whatever was asked.
    let fixed_a = begun(&defs, "core:beta", 5, 0, vec![]).0;
    assert_eq!(
        fixed_a
            .truth()
            .cosmos()
            .realms()
            .next()
            .map(|realm| realm.seed),
        Some(5)
    );
}

#[test]
fn a_begin_that_cannot_be_done_is_refused_with_its_reason() {
    let defs = defs();
    let (mut sim, player) = begun(&defs, "core:alpha", 9, 0, vec![]);
    let again = sim.submit(Command::BeginScenario(BeginScenario {
        scenario: "core:beta".to_owned(),
        party: vec![player],
        place: 0,
        options: vec![],
    }));
    assert_eq!(again.unwrap_err().reason.code, "scenario.begun");

    let world = WorldDefs::from_defs(&defs).unwrap();
    let made = make_world(&defs, world, "core:alpha", 1).unwrap();
    let mut fresh = Sim::from_parts(Arc::clone(&defs), made.cosmos, made.timeline, 1);
    let player = fresh.truth().entities().next_id().unwrap();
    fresh
        .submit(Command::Debug(DebugCommand::Spawn {
            spec: CharacterSpec::of(["core:person"]),
            place: Place::Stored,
        }))
        .unwrap();
    let before = fresh.state_hash();
    let refusals = [
        ("core:nowhere", 0, vec![], "scenario.unknown"),
        ("core:alpha", 7, vec![], "scenario.bad_place"),
        (
            "core:alpha",
            0,
            vec![("core:pace".to_owned(), Value::Str("slow".to_owned()))],
            "setting.not_an_option",
        ),
        (
            "core:alpha",
            0,
            vec![("core:bold".to_owned(), Value::Int(3))],
            "setting.bad_value",
        ),
        ("core:gamma", 0, vec![], "scenario.bad_world"),
    ];
    for (scenario, place, options, code) in refusals {
        let refused = fresh.submit(Command::BeginScenario(BeginScenario {
            scenario: scenario.to_owned(),
            party: vec![player],
            place,
            options,
        }));
        assert_eq!(refused.unwrap_err().reason.code, code, "{scenario} {place}");
        assert_eq!(fresh.state_hash(), before, "a refusal changes nothing");
    }
}

#[test]
fn a_locked_setting_cannot_be_changed_in_its_scenario() {
    let defs = defs();
    let (mut sim, _) = begun(&defs, "core:alpha", 9, 0, vec![]);
    let refused = sim.submit(Command::Setting {
        key: "core:pace".to_owned(),
        value: Value::Str("slow".to_owned()),
    });
    let reason = refused.unwrap_err().reason;
    assert_eq!(reason.code, "setting.locked");
    assert_eq!(
        sim.settings().get("core:pace"),
        Some(&Value::Str("quick".to_owned()))
    );
    // One that is not locked changes, within what its definition takes.
    sim.submit(Command::Setting {
        key: "core:bold".to_owned(),
        value: Value::Bool(true),
    })
    .expect("not locked");
    let bad = sim.submit(Command::Setting {
        key: "core:bold".to_owned(),
        value: Value::Str("very".to_owned()),
    });
    assert_eq!(bad.unwrap_err().reason.code, "setting.bad_value");

    // In another scenario the same setting can be changed.
    let (mut other, _) = begun(&defs, "core:beta", 5, 0, vec![]);
    other
        .submit(Command::Setting {
            key: "core:pace".to_owned(),
            value: Value::Str("quick".to_owned()),
        })
        .expect("beta does not lock it");
    assert_eq!(
        other.settings().get("core:pace"),
        Some(&Value::Str("quick".to_owned()))
    );
}

#[test]
fn a_scenarios_rules_do_not_run_in_another_scenario() {
    let defs = defs();
    let (mut alpha, a) = begun(&defs, "core:alpha", 9, 0, vec![]);
    let (mut beta, b) = begun(&defs, "core:beta", 5, 0, vec![]);
    for (sim, who) in [(&mut alpha, a), (&mut beta, b)] {
        let at = sim.truth().location(who).unwrap();
        let next =
            clash_world::Location::new(at.grid, at.cell.offset(clash_core::IVec3::new(1, 0, 0)));
        sim.submit(Command::Debug(DebugCommand::MoveEntity {
            entity: who,
            to: Place::at(next),
        }))
        .unwrap();
        run(sim, Dur::from_secs(1));
    }
    assert!(
        var(&alpha, "core:moves").as_int().unwrap_or(0) > 0,
        "alpha's rule counts moves in alpha"
    );
    assert_eq!(
        var(&beta, "core:moves").as_int().unwrap_or(0),
        0,
        "alpha's rule does not run in beta"
    );
}

#[test]
fn objectives_equal_a_fresh_evaluation_until_they_settle_then_stay() {
    let defs = defs();
    let (mut sim, _) = begun(&defs, "core:alpha", 9, 0, vec![]);
    let ids = ["gather", "report", "whole", "part_one", "part_two"];
    let mut settled = Vec::new();
    run(&mut sim, Dur::from_secs(1));
    assert_eq!(
        objective_state(&sim, None, "report"),
        Some(ObjectiveState::Hidden),
        "after `gather`"
    );
    check_z4(&sim, &ids, &mut settled);
    for marks in 1..=6 {
        set_var(&mut sim, "core:marks", marks);
        run(&mut sim, Dur::from_millis(500));
        check_z4(&sim, &ids, &mut settled);
        if marks == 2 {
            // The parent is done once both its parts are.
            assert_eq!(
                objective_state(&sim, None, "whole"),
                Some(ObjectiveState::Done)
            );
        }
        if marks == 3 {
            assert_eq!(
                objective_state(&sim, None, "gather"),
                Some(ObjectiveState::Done)
            );
            assert_eq!(
                objective_state(&sim, None, "report"),
                Some(ObjectiveState::Active),
                "shown once `gather` is done"
            );
        }
    }
    // Settled is settled: what made it done going away, or a failure
    // condition coming to hold, changes nothing.
    set_var(&mut sim, "core:marks", 0);
    set_var(&mut sim, "core:ruined", true);
    run(&mut sim, Dur::from_millis(500));
    check_z4(&sim, &ids, &mut settled);
    assert_eq!(
        objective_state(&sim, None, "gather"),
        Some(ObjectiveState::Done)
    );
    assert_eq!(
        objective_state(&sim, None, "report"),
        Some(ObjectiveState::Done)
    );
}

#[test]
fn an_objective_is_not_looked_at_again_while_nothing_it_reads_changes() {
    let defs = defs();
    let (mut sim, _) = begun(&defs, "core:alpha", 9, 0, vec![]);
    run(&mut sim, Dur::from_secs(1));
    // Behind the objectives' back (no event): a fresh evaluation would
    // say done, and the objective is not looked at, because nothing it
    // watches was delivered.
    set_var(&mut sim, "core:ruined", true);
    sim.set_var(
        Subject::Global,
        "core:marks",
        clash_expr::Value::Int(0),
        Cause::DEBUG,
    )
    .unwrap();
    run(&mut sim, Dur::from_secs(1));
    assert_eq!(
        objective_state(&sim, None, "gather"),
        Some(ObjectiveState::Failed),
        "`ruined` is watched: its change is delivered as `var_changed`"
    );
}

#[test]
fn a_condition_ending_ends_the_game_and_asks_the_application() {
    let defs = defs();
    let (mut sim, _) = begun(&defs, "core:alpha", 9, 0, vec![]);
    set_var(&mut sim, "core:marks", 9);
    let asked = run(&mut sim, Dur::from_millis(500));
    assert!(
        asked.contains(&AppRequest::EndScenario {
            outcome: "won".to_owned()
        }),
        "{asked:?}"
    );
    let game = sim.scenario().game.as_ref().unwrap();
    assert_eq!(game.ended.as_deref(), Some("won"));
    // Ended is ended: nothing more is settled.
    let refused = sim.submit(Command::Setting {
        key: "core:bold".to_owned(),
        value: Value::Bool(true),
    });
    assert!(refused.is_ok(), "settings are not the game's to refuse");
}

/// Saves a simulation and loads it again as a fresh process would: the
/// world through its own save, the simulation through its blob; then
/// everyone is placed again.
fn save_and_load(sim: &Sim) -> Sim {
    let blob = sim.save().unwrap();
    let mut cosmos = sim.truth().cosmos().clone();
    let mut store = MemoryStore::new();
    cosmos.save_world(&mut store).unwrap();
    let generators = clash_gen::generators(cosmos.defs());
    let mut io = TractIo {
        generators: &generators,
        store: &mut store,
    };
    let cosmos = Cosmos::load_world(cosmos.defs(), &mut io).unwrap();
    let mut loaded = Sim::load(Arc::clone(sim.defs()), cosmos, &blob).unwrap();
    assert_eq!(loaded.reindex(), []);
    loaded
}

/// Starts a mission from a rule, as content does.
fn start_errand(sim: &mut Sim, taker: EntityId, count: i64) {
    let effects = sim
        .compile_effects(
            &format!(
                r#"[ {{ "start_mission": {{ "mission": "core:errand", "taker": "self", "with": "{{ 'who': 'Ada', 'count': {count} }}" }} }} ]"#
            ),
            &[("self", clash_expr::Type::Any)],
            &[],
        )
        .expect("the effect compiles");
    let who = sim.subject_value(Subject::Entity(taker));
    let mut locals = Vec::new();
    sim.run_effects(&effects, &[who], &mut locals, None, Cause::DEBUG);
}

#[test]
fn a_mission_shows_its_objectives_in_order_and_is_done_when_they_are() {
    let defs = defs();
    let (mut sim, player) = begun(&defs, "core:beta", 5, 0, vec![]);
    start_errand(&mut sim, player, 2);
    run(&mut sim, Dur::from_millis(500));
    let game = sim.scenario().game.as_ref().unwrap();
    assert_eq!(game.missions.len(), 1);
    let mission = &game.missions[0];
    assert_eq!(mission.serial, 1);
    assert_eq!(mission.taker, player);
    assert_eq!(
        objective_state(&sim, Some(1), "fetch"),
        Some(ObjectiveState::Active)
    );
    assert_eq!(
        objective_state(&sim, Some(1), "back"),
        Some(ObjectiveState::Hidden)
    );
    // The journal names it as the taker knows it.
    let journal = clash_sim::scenario::journal::journal(&sim, player);
    let texts: Vec<String> = journal
        .iter()
        .filter_map(|row| row.as_map()?.get("text")?.as_str().map(str::to_owned))
        .collect();
    assert_eq!(texts, ["Errand for Ada", "Fetch 2 marks."]);

    set_var(&mut sim, "core:marks", 2);
    let asked = run(&mut sim, Dur::from_millis(500));
    assert_eq!(
        objective_state(&sim, Some(1), "back"),
        Some(ObjectiveState::Active)
    );
    // Beta ends on an objective `fetch` being done: of any list.
    assert!(
        asked.contains(&AppRequest::EndScenario {
            outcome: "fetched".to_owned()
        }),
        "{asked:?}"
    );
}

#[test]
fn a_mission_expires_when_its_time_runs_out() {
    let defs = defs();
    let (mut sim, player) = begun(&defs, "core:alpha", 9, 0, vec![]);
    start_errand(&mut sim, player, 50);
    run(&mut sim, Dur::from_secs(59 * 60));
    let state = |sim: &Sim| sim.scenario().game.as_ref().unwrap().missions[0].state;
    assert_eq!(state(&sim), MissionState::Active);
    run(&mut sim, Dur::from_secs(2 * 60));
    assert_eq!(state(&sim), MissionState::Expired);
}

#[test]
fn saved_and_loaded_in_the_middle_of_a_mission_nothing_changes() {
    let defs = defs();
    let (mut sim, player) = begun(&defs, "core:beta", 5, 0, vec![]);
    start_errand(&mut sim, player, 3);
    set_var(&mut sim, "core:marks", 1);
    run(&mut sim, Dur::from_secs(2));

    let mut loaded = save_and_load(&sim);
    assert_eq!(loaded.state_hash(), sim.state_hash(), "the same state");
    assert_eq!(loaded.scenario().game, sim.scenario().game);

    for twin in [&mut sim, &mut loaded] {
        set_var(twin, "core:marks", 3);
        run(twin, Dur::from_secs(2));
    }
    assert_eq!(loaded.state_hash(), sim.state_hash(), "and the same after");
    assert_eq!(
        objective_state(&loaded, Some(1), "back"),
        Some(ObjectiveState::Active)
    );
}

#[test]
fn the_journal_and_the_site_are_read_of_the_character() {
    let defs = defs();
    let (mut sim, player) = begun(&defs, "core:alpha", 9, 0, vec![]);
    run(&mut sim, Dur::from_millis(500));
    let properties = clash_sim::scenario::journal::properties();
    let read = |name: &str| {
        let property = properties
            .iter()
            .find(|property| property.name == name)
            .unwrap_or_else(|| panic!("no property `{name}`"));
        (property.get)(&sim, Subject::Entity(player), &[])
    };
    let rows = read("objectives");
    let rows = rows.as_list().expect("a list");
    // `gather`, `whole` and its two parts: `report` waits for `gather`.
    assert_eq!(rows.len(), 4);
    let first = rows[0].as_map().unwrap();
    assert_eq!(
        first.get("id").and_then(clash_expr::Value::as_str),
        Some("gather")
    );
    assert_eq!(
        first.get("state").and_then(clash_expr::Value::as_str),
        Some("active")
    );
    let depths: Vec<i64> = rows
        .iter()
        .filter_map(|row| row.as_map()?.get("depth")?.as_int())
        .collect();
    assert_eq!(depths, [0, 0, 1, 1], "parts are under the whole");
    // The flat test land has no sites.
    assert_eq!(read("site"), clash_expr::Value::Null);
}

#[test]
fn under_continue_the_player_carries_on_as_a_newcomer() {
    let defs = defs();
    let (mut sim, first) = begun(&defs, "core:beta", 5, 0, vec![]);
    run(&mut sim, Dur::from_secs(1));
    let fell_at = sim.truth().location(first).unwrap();
    let effects = sim
        .compile_effects(
            r#"[ { "die": { "who": "self" } } ]"#,
            &[("self", clash_expr::Type::Any)],
            &[],
        )
        .unwrap();
    let who = sim.subject_value(Subject::Entity(first));
    let outcome = sim.run_effects(&effects, &[who], &mut Vec::new(), None, Cause::DEBUG);
    assert_eq!(
        sim.vital(first),
        Some(clash_sim::mind::VitalState::Dead),
        "{outcome:?} {:?}",
        sim.drain_events()
            .iter()
            .map(|event| event.name().to_owned())
            .collect::<Vec<_>>()
    );
    let asked = run(&mut sim, Dur::from_secs(1));
    let game = sim.scenario().game.as_ref().unwrap();
    assert_eq!(game.fallen, [first]);
    let next = *game.party.last().unwrap();
    assert_ne!(next, first);
    assert!(
        asked.contains(&AppRequest::TakeControl { entity: next }),
        "{asked:?}"
    );
    assert_eq!(
        sim.truth().location(first),
        Some(fell_at),
        "the body is where it fell"
    );
    assert_eq!(
        sim.truth().entity(next).unwrap().control(),
        clash_sim::Control::Direct
    );
    assert!(game.ended.is_none());
}

#[test]
fn under_end_the_game_ends_when_the_party_is_gone() {
    let defs = defs();
    let (mut sim, first) = begun(&defs, "core:alpha", 9, 0, vec![]);
    run(&mut sim, Dur::from_secs(1));
    let effects = sim
        .compile_effects(
            r#"[ { "die": { "who": "self" } } ]"#,
            &[("self", clash_expr::Type::Any)],
            &[],
        )
        .unwrap();
    let who = sim.subject_value(Subject::Entity(first));
    sim.run_effects(&effects, &[who], &mut Vec::new(), None, Cause::DEBUG);
    let asked = run(&mut sim, Dur::from_secs(1));
    assert!(
        asked.contains(&AppRequest::EndScenario {
            outcome: "lost".to_owned()
        }),
        "{asked:?}"
    );
}

/// A scenario whose `start.do` sets a timer, as the last stand sets its
/// dawn: what it runs when it falls due is found again in the
/// scenario's own effects (decision D-879).
const TIMED: &str = r#"[
  { "type": "variable", "id": "rang", "scope": "global", "value": "bool", "default": false },
  { "type": "scenario", "id": "timed", "name": "Timed",
    "worlds": [ { "realm": "testland", "seed": "ask" } ],
    "start": { "characters": [ { "template": "person" } ],
               "at": [ { "name": "West", "cell": [10, 10] } ],
               "do": [ { "after": "2 s", "owner": "global",
                         "do": [ { "set": { "global.var.rang": true } },
                                 { "complete_objective": "wait" } ] } ] },
    "objectives": [ { "id": "wait", "text": "Wait for it." } ] }
]"#;

#[test]
fn a_timer_set_in_a_scenarios_start_runs_when_it_falls_due_after_a_load_too() {
    let defs = common::life_defs(TIMED);
    let (mut sim, _) = begun(&defs, "core:timed", 3, 0, vec![]);
    run(&mut sim, Dur::from_millis(1_000));
    assert_eq!(var(&sim, "core:rang"), clash_expr::Value::Bool(false));
    let mut loaded = save_and_load(&sim);
    for sim in [&mut sim, &mut loaded] {
        run(sim, Dur::from_millis(1_500));
        assert_eq!(var(sim, "core:rang"), clash_expr::Value::Bool(true));
        assert_eq!(
            objective_state(sim, None, "wait"),
            Some(ObjectiveState::Done)
        );
        assert_eq!(sim.rule_fault_count(), 0, "the timer was found");
    }
    assert_eq!(loaded.state_hash(), sim.state_hash());
}

#[test]
fn a_variable_set_just_before_an_objective_fails_is_read_in_its_on_failed() {
    // The second play-testing's known issue 11: the errand's timer set
    // `errand_lost` and failed the objective in one effect list, and
    // `on_failed` did not find it `'left'`. The variable is read, set:
    // the errand had written `"'left'"`, which a text-typed place takes
    // as it is written, quotes and all (05 §7.1), and which the loader
    // now warns of (W0679, decision D-946).
    let defs = defs();
    let (mut sim, _) = begun(&defs, "core:delta", 5, 0, Vec::new());
    run(&mut sim, Dur::from_secs(2));
    assert_eq!(
        objective_state(&sim, None, "hold"),
        Some(ObjectiveState::Failed)
    );
    assert_eq!(var(&sim, "core:told"), clash_expr::Value::from("left"));
}
