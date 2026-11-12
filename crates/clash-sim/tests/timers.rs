//! Timers and continuations (plans/sim-kernel.md, package K7; 05 §9.4,
//! §10.4, §13.4; 03 §9): `after`, `every`, `cancel_timer`, a rule's own
//! `every`, the owner's clock, `pause` and `elapse` across leaving the
//! world, staggering, and the body of the test script
//! `timer_survives_save`.

mod common;

use std::collections::BTreeSet;

use clash_core::{EntityId, Tick};
use clash_expr::Value;
use clash_sim::rule::TimerState;
use clash_sim::{AdvanceLimit, EventBody, Place, Sim, Subject};
use clash_world::Cause;
use common::{box_area, creature, on, save_and_load, sim_over, spawn, try_defs_with};

const RULES: &str = r#"[
  { "type": "variable", "id": "fired", "scope": "entity", "value": "int", "default": 0, "doc": "" },
  { "type": "variable", "id": "left", "scope": "entity", "value": "real", "doc": "" },
  { "type": "variable", "id": "ticks", "scope": "global", "value": "int", "default": 0, "doc": "" },

  { "type": "eoc", "id": "pause_fuse", "subjects": { "self": "entity" },
    "doc": "A fuse that stands still while its owner is away.",
    "do": [ { "let": { "n": 7 } },
            { "after": "10 s", "name": "fuse", "do": [ { "add": { "self.var.fired": "$n" } } ] } ] },
  { "type": "eoc", "id": "elapse_fuse", "subjects": { "self": "entity" },
    "doc": "A fuse that burns while its owner is away.",
    "do": [ { "after": "10 s", "name": "fuse", "while_unloaded": "elapse",
              "do": [ { "add": { "self.var.fired": 1 } } ] } ] },
  { "type": "eoc", "id": "ticker", "subjects": { "self": "entity" },
    "doc": "Three ticks a second apart.",
    "do": [ { "every": "1 s", "times": 3, "name": "tick", "do": [ { "add": { "self.var.fired": 1 } } ] } ] },
  { "type": "eoc", "id": "defuse", "subjects": { "self": "entity" },
    "doc": "Cancels the fuse.",
    "do": [ { "cancel_timer": "fuse" } ] },
  { "type": "eoc", "id": "peek", "subjects": { "self": "entity" },
    "doc": "Reads how long the fuse has left.",
    "do": [ { "set": { "self.var.left": "self.timer('fuse')" } } ] },
  { "type": "eoc", "id": "heartbeat", "attached": true, "subjects": { "self": "entity" },
    "every": "10 s", "while_unloaded": "once",
    "doc": "Beats for what it is attached to.",
    "do": [ { "add": { "self.var.fired": 1 } } ] },
  { "type": "eoc", "id": "wire", "subjects": { "self": "entity" },
    "doc": "Attaches the heartbeat.",
    "do": [ { "attach_rule": { "to": "self", "eoc": "heartbeat" } } ] },
  { "type": "eoc", "id": "unwire", "subjects": { "self": "entity" },
    "doc": "Detaches the heartbeat.",
    "do": [ { "detach_rule": { "to": "self", "eoc": "heartbeat" } } ] },
  { "type": "eoc", "id": "clockwork", "every": "5 s",
    "doc": "Counts on the universal clock.",
    "do": [ { "add": { "global.var.ticks": 1 } } ] }
]"#;

fn sim() -> Sim {
    let defs = try_defs_with(&[("data/test/timers.json", RULES)])
        .unwrap_or_else(|error| panic!("the test definitions do not load:\n{error}"));
    sim_over(defs, 9)
}

fn call(sim: &mut Sim, rule: &str, who: EntityId) {
    let id = sim.rule_set().lookup(rule).unwrap();
    let me = sim.subject_value(Subject::Entity(who));
    sim.call_rule(id, &[("self", me)], &[], Cause::DEBUG)
        .unwrap();
}

fn until(sim: &mut Sim, secs: f64) {
    let tick = Tick((secs * 1e6) as i64);
    sim.advance(AdvanceLimit::Until(tick));
}

fn fired(sim: &Sim, who: EntityId) -> Value {
    sim.var(Subject::Entity(who), "core:fired")
}

fn faults(sim: &mut Sim) -> Vec<String> {
    sim.drain_events()
        .iter()
        .filter_map(|event| match &event.body {
            EventBody::Fault { reason } => Some(format!("{}: {}", reason.code, reason.message)),
            _ => None,
        })
        .collect()
}

#[test]
fn after_runs_later_with_the_locals_it_captured() {
    let mut sim = sim();
    let who = spawn(&mut sim, creature(), on(5, 5));
    call(&mut sim, "core:pause_fuse", who);
    until(&mut sim, 9.9);
    assert_eq!(fired(&sim, who), Value::Int(0));
    call(&mut sim, "core:peek", who);
    let left = sim
        .var(Subject::Entity(who), "core:left")
        .as_number()
        .unwrap();
    assert!((left.get() - 0.1).abs() < 1e-3, "{left:?}");
    until(&mut sim, 10.0);
    assert_eq!(fired(&sim, who), Value::Int(7));
    assert!(
        sim.timers().is_empty() || sim.timers().len() == 1,
        "only the clockwork is left"
    );
    assert_eq!(faults(&mut sim), Vec::<String>::new());
}

#[test]
fn every_repeats_and_a_name_replaces_and_cancels() {
    let mut sim = sim();
    let who = spawn(&mut sim, creature(), on(5, 5));
    call(&mut sim, "core:ticker", who);
    until(&mut sim, 10.0);
    assert_eq!(fired(&sim, who), Value::Int(3));

    // The same name again replaces the timer: one run, from the second.
    let other = spawn(&mut sim, creature(), on(6, 6));
    call(&mut sim, "core:pause_fuse", other);
    until(&mut sim, 15.0);
    call(&mut sim, "core:pause_fuse", other);
    until(&mut sim, 20.5);
    assert_eq!(fired(&sim, other), Value::Int(0));
    until(&mut sim, 25.0);
    assert_eq!(fired(&sim, other), Value::Int(7));

    // A cancelled timer never runs.
    call(&mut sim, "core:pause_fuse", other);
    call(&mut sim, "core:defuse", other);
    until(&mut sim, 60.0);
    assert_eq!(fired(&sim, other), Value::Int(7));
    assert_eq!(faults(&mut sim), Vec::<String>::new());
}

#[test]
fn a_global_recurring_rule_runs_on_the_universal_clock() {
    let mut sim = sim();
    until(&mut sim, 50.0);
    // Its first run is staggered within the first interval; then one
    // every five seconds.
    let ticks = sim.var(Subject::Global, "core:ticks").as_int().unwrap();
    assert!((9..=10).contains(&ticks), "{ticks}");
}

#[test]
fn timers_run_on_the_owners_clock() {
    let mut sim = sim();
    box_area(&mut sim, (0, 0), (20, 20), Some(0.5), 1);
    let slow = spawn(&mut sim, creature(), on(5, 5));
    let quick = spawn(&mut sim, creature(), on(40, 40));
    call(&mut sim, "core:pause_fuse", slow);
    call(&mut sim, "core:pause_fuse", quick);
    until(&mut sim, 10.0);
    assert_eq!(fired(&sim, quick), Value::Int(7));
    assert_eq!(fired(&sim, slow), Value::Int(0));
    until(&mut sim, 19.9);
    assert_eq!(fired(&sim, slow), Value::Int(0));
    until(&mut sim, 20.0);
    assert_eq!(fired(&sim, slow), Value::Int(7));
}

/// Takes an entity out of the world and puts it back, as its tract
/// unloading and loading again does.
fn away(sim: &mut Sim, who: EntityId) {
    sim.relocate(who, Place::Stored, Cause::DEBUG).unwrap();
}

fn back(sim: &mut Sim, who: EntityId) {
    sim.relocate(who, on(5, 5), Cause::DEBUG).unwrap();
}

#[test]
fn pause_stands_still_while_the_owner_is_away() {
    let mut sim = sim();
    let who = spawn(&mut sim, creature(), on(5, 5));
    call(&mut sim, "core:pause_fuse", who);
    until(&mut sim, 3.0);
    away(&mut sim, who);
    // Nothing of it is on the timeline while its owner is away.
    let id = sim
        .timers()
        .iter()
        .find(|(_, record)| record.name.is_some())
        .unwrap()
        .0;
    assert!(matches!(
        sim.timers().get(id).unwrap().state,
        TimerState::Away { .. }
    ));
    until(&mut sim, 100.0);
    assert_eq!(fired(&sim, who), Value::Int(0));
    // A save while it is away keeps it.
    let (mut loaded, _) = save_and_load(&sim);
    for sim in [&mut sim, &mut loaded] {
        back(sim, who);
        until(sim, 106.9);
        assert_eq!(fired(sim, who), Value::Int(0));
        until(sim, 107.0);
        assert_eq!(fired(sim, who), Value::Int(7));
    }
    assert_eq!(sim.state_hash(), loaded.state_hash());
}

#[test]
fn elapse_lets_time_pass_and_fires_once_on_return_if_overdue() {
    let mut sim = sim();
    let early = spawn(&mut sim, creature(), on(5, 5));
    let late = spawn(&mut sim, creature(), on(6, 6));
    call(&mut sim, "core:elapse_fuse", early);
    call(&mut sim, "core:elapse_fuse", late);
    until(&mut sim, 3.0);
    away(&mut sim, early);
    away(&mut sim, late);
    // Back before it was due: what passed is taken off.
    until(&mut sim, 6.0);
    back(&mut sim, early);
    until(&mut sim, 9.9);
    assert_eq!(fired(&sim, early), Value::Int(0));
    until(&mut sim, 10.0);
    assert_eq!(fired(&sim, early), Value::Int(1));
    // Back long after: it fires once, at once.
    until(&mut sim, 40.0);
    assert_eq!(fired(&sim, late), Value::Int(0));
    sim.relocate(late, on(6, 6), Cause::DEBUG).unwrap();
    until(&mut sim, 40.0);
    assert_eq!(fired(&sim, late), Value::Int(1));
    until(&mut sim, 200.0);
    assert_eq!(fired(&sim, late), Value::Int(1));
    assert_eq!(faults(&mut sim), Vec::<String>::new());
}

#[test]
fn an_attached_recurring_rule_runs_for_its_owner_and_once_on_return() {
    let mut sim = sim();
    let who = spawn(&mut sim, creature(), on(5, 5));
    call(&mut sim, "core:wire", who);
    until(&mut sim, 30.0);
    assert_eq!(fired(&sim, who), Value::Int(3));
    away(&mut sim, who);
    until(&mut sim, 95.0);
    back(&mut sim, who);
    until(&mut sim, 95.0);
    // `once`: one run for all that was missed.
    assert_eq!(fired(&sim, who), Value::Int(4));
    call(&mut sim, "core:unwire", who);
    until(&mut sim, 200.0);
    assert_eq!(fired(&sim, who), Value::Int(4));
}

#[test]
fn a_thousand_owners_of_one_recurring_rule_do_not_fire_at_one_tick() {
    let mut sim = sim();
    let mut owners = Vec::new();
    for slot in 0..1000 {
        let who = spawn(&mut sim, creature(), on(slot % 60, slot / 60));
        call(&mut sim, "core:wire", who);
        owners.push(who);
    }
    let due: Vec<Tick> = sim
        .timers()
        .iter()
        .filter_map(|(_, record)| match record.state {
            TimerState::Waiting { wake } => sim.timeline().due(wake),
            TimerState::Away { .. } => None,
        })
        .collect();
    assert_eq!(due.len(), 1001, "a heartbeat each and the clockwork");
    let distinct: BTreeSet<Tick> = due.iter().copied().collect();
    assert!(distinct.len() > 990, "{} distinct ticks", distinct.len());
    assert!(due.iter().all(|tick| tick.0 > 0 && tick.0 <= 10_000_000));
    // Every one runs once in its first interval.
    until(&mut sim, 10.0);
    for who in owners {
        assert_eq!(fired(&sim, who), Value::Int(1));
    }
}

/// The body of the test script `timer_survives_save`: timers and
/// continuations fire identically with and without a save between.
#[test]
fn timer_survives_save() {
    let mut straight = sim();
    let who = spawn(&mut straight, creature(), on(5, 5));
    let other = spawn(&mut straight, creature(), on(7, 7));
    call(&mut straight, "core:pause_fuse", who);
    call(&mut straight, "core:ticker", other);
    call(&mut straight, "core:wire", other);
    until(&mut straight, 2.5);
    let (mut loaded, unplaced) = save_and_load(&straight);
    assert!(unplaced.is_empty());
    assert_eq!(loaded.state_hash(), straight.state_hash());
    for sim in [&mut straight, &mut loaded] {
        until(sim, 31.0);
    }
    assert_eq!(fired(&straight, who), Value::Int(7));
    assert_eq!(fired(&loaded, who), Value::Int(7));
    assert_eq!(fired(&straight, other), fired(&loaded, other));
    assert_eq!(straight.state_hash(), loaded.state_hash());
}
