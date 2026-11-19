//! The action runtime and control (plans/sim-kernel.md, packages T3 and
//! T4; 03 §5, §6): actions from the fixture mod `acts`
//! (`tests/fixtures/act/mods/acts`) run against a real simulation.
//!
//! T3: phases begin at the ticks their times, the tempo and the clock
//! rates give (`punch_timing`); a disabled slot interrupts the action
//! using it and blocks those that need it (`one_arm`); an action in a
//! stopped area freezes and resumes (`time_stop_windup`); a loop of
//! phases of no length ends at the transition limit; every refusal
//! carries its reason; a running action survives a save and a replay.
//!
//! T4: a directly controlled character is asked when it is free; one
//! command starts a two-phase action and the question comes back exactly
//! when it ends; a second character on a faster clock interleaves;
//! simultaneous decisions come in entity order (`party_order`); a plan
//! step that cannot start clears the plan and asks with the reason; a
//! delegated character takes orders and is never asked.

mod common;

use clash_core::{Dur, EntityId, Real, Tick};
use clash_expr::Value;
use clash_sim::action::{ActEvent, ActionDef, ActionDefId, How, Params};
use clash_sim::clock::ClockTarget;
use clash_sim::{
    Advance, AdvanceLimit, Command, CommandError, Control, DebugCommand, DecisionKind, EntitySpec,
    EventBody, FreeReason, Place, Sim, SimEvent, Subject,
};
use clash_world::{Cause, CellPos, Location};
use common::{
    GRID, act_fixtures, at, box_area, creature, debug, defs_with, on, save_and_load, sim_over,
};

const MS: i64 = 1_000;
const SECOND: i64 = 1_000_000;

fn sim() -> Sim {
    sim_over(defs_with(&act_fixtures(), &["acts"]), 21)
}

/// Makes an entity by command, so that a replay makes it too.
fn make(sim: &mut Sim, spec: EntitySpec, x: i32, y: i32) -> EntityId {
    let id = sim.truth().entities().next_id().unwrap();
    debug(
        sim,
        DebugCommand::CreateEntity {
            spec,
            place: on(x, y),
        },
    )
    .unwrap();
    id
}

fn action(sim: &Sim, name: &str) -> ActionDefId {
    sim.defs()
        .lookup::<ActionDef>(&format!("acts:{name}"))
        .unwrap_or_else(|| panic!("no action {name}"))
}

fn act_with(
    sim: &mut Sim,
    who: EntityId,
    name: &str,
    params: Params,
    how: How,
) -> Result<(), CommandError> {
    let action = action(sim, name);
    sim.submit(Command::Act {
        entity: who,
        action,
        params,
        how,
    })
}

fn act(sim: &mut Sim, who: EntityId, name: &str) -> Result<(), CommandError> {
    act_with(sim, who, name, Vec::new(), How::Start)
}

fn target(sim: &Sim, who: EntityId) -> Params {
    vec![(
        "target".to_owned(),
        sim.subject_value(Subject::Entity(who)).to_stored(),
    )]
}

fn var(sim: &Sim, who: EntityId, name: &str) -> Value {
    sim.var(Subject::Entity(who), &format!("acts:{name}"))
}

fn until(sim: &mut Sim, micros: i64) -> Advance {
    sim.advance(AdvanceLimit::Until(Tick(micros)))
}

/// The events of acting, with the tick each was raised at.
fn acted(events: &[SimEvent]) -> Vec<(i64, &ActEvent)> {
    events
        .iter()
        .filter_map(|event| match &event.body {
            EventBody::Act(act) => Some((event.tick.as_micros(), act)),
            _ => None,
        })
        .collect()
}

/// When each phase of an actor's actions began, in order.
fn phases(events: &[SimEvent], who: EntityId) -> Vec<(i64, String)> {
    acted(events)
        .into_iter()
        .filter_map(|(tick, event)| match event {
            ActEvent::PhaseEntered { actor, phase, .. } if *actor == who => {
                Some((tick, phase.clone()))
            }
            _ => None,
        })
        .collect()
}

fn faults(events: &[SimEvent]) -> Vec<String> {
    events
        .iter()
        .filter_map(|event| match &event.body {
            EventBody::Fault { reason } => Some(format!("{}: {}", reason.code, reason.message)),
            _ => None,
        })
        .collect()
}

fn refused(result: Result<(), CommandError>) -> String {
    result.unwrap_err().reason.code
}

/// Answers the free decision a directly controlled entity is asked when
/// it comes into the world, as a frontend does: by waiting no time.
fn settle(sim: &mut Sim, who: EntityId) {
    while matches!(
        sim.decisions().front_of(who),
        Some(DecisionKind::Free { .. })
    ) {
        sim.submit(Command::Wait {
            entity: who,
            dur: Dur::ZERO,
        })
        .unwrap();
    }
}

// ---- T3: phases and timing ----------------------------------------------------

/// Scenario `punch_timing`: the phases of a punch begin at the ticks
/// its times give, over the tempo and on the actor's clock.
#[test]
fn punch_timing() {
    for (tempo, rate) in [(1.0, 1.0), (2.0, 1.0), (0.5, 1.0), (1.0, 2.0), (2.0, 4.0)] {
        let mut sim = sim();
        let who = make(&mut sim, creature().with_control(Control::Delegated), 2, 2);
        let foe = make(&mut sim, creature(), 3, 2);
        sim.set_var(
            Subject::Entity(who),
            "acts:quickness",
            Value::Real(Real::of(tempo)),
            Cause::DEBUG,
        )
        .unwrap();
        if rate != 1.0 {
            sim.set_time_rate(who, Some(Real::of(rate)), Cause::DEBUG)
                .unwrap();
        }
        sim.drain_events();
        let params = target(&sim, foe);
        act_with(&mut sim, who, "punch", params, How::Start).unwrap();
        until(&mut sim, 10 * SECOND);
        let events = sim.drain_events();
        assert_eq!(faults(&events), Vec::<String>::new());
        // Universal microseconds: local time over the tempo, over the
        // rate of the actor's clock.
        let scale = |local_ms: f64| (local_ms * 1_000.0 / tempo / rate).round() as i64;
        assert_eq!(
            phases(&events, who),
            [
                (0, "windup".to_owned()),
                (scale(300.0), "impact".to_owned()),
                (scale(300.0), "recovery".to_owned()),
            ],
            "tempo {tempo}, rate {rate}"
        );
        let finished: Vec<i64> = acted(&events)
            .into_iter()
            .filter(|(_, event)| matches!(event, ActEvent::ActionFinished { .. }))
            .map(|(tick, _)| tick)
            .collect();
        assert_eq!(finished, [scale(700.0)]);
        assert_eq!(var(&sim, who, "finished"), Value::Int(1));
        assert_eq!(var(&sim, who, "hits"), Value::Int(1));
        // Each phase says how long it lasts on the actor's clock and
        // when it is due.
        let entered: Vec<(Option<Dur>, Option<Tick>)> = acted(&events)
            .into_iter()
            .filter_map(|(_, event)| match event {
                ActEvent::PhaseEntered { duration, due, .. } => Some((*duration, *due)),
                _ => None,
            })
            .collect();
        assert_eq!(
            entered[0],
            (
                Some(Dur::from_micros((300_000.0 / tempo).round() as i64)),
                Some(Tick(scale(300.0)))
            )
        );
        // The window of the windup is published, aimed at the target.
        assert!(acted(&events).iter().any(|(_, event)| matches!(
            event,
            ActEvent::WindowOpened { aimed_at, closes, .. } if aimed_at == &[foe] && closes == "impact"
        )));
    }
}

/// The hook `phase_timing` changes how long a phase lasts.
#[test]
fn a_handler_of_phase_timing_changes_a_duration() {
    let mut sim = sim();
    let who = make(
        &mut sim,
        creature()
            .with_control(Control::Delegated)
            .with_tag("slowed"),
        2,
        2,
    );
    sim.drain_events();
    act(&mut sim, who, "step").unwrap();
    until(&mut sim, 5 * SECOND);
    assert_eq!(
        phases(&sim.drain_events(), who),
        [(0, "lift".to_owned()), (400 * MS, "land".to_owned())]
    );
}

/// Scenario `time_stop_windup`: an action inside an area whose clock
/// stands still freezes, and resumes when the clock moves again.
#[test]
fn time_stop_windup() {
    let mut sim = sim();
    let area = box_area(&mut sim, (10, 10), (14, 14), Some(0.0), 10);
    let who = make(
        &mut sim,
        creature().with_control(Control::Delegated),
        12,
        12,
    );
    sim.drain_events();
    act(&mut sim, who, "step").unwrap();
    until(&mut sim, 5 * SECOND);
    assert_eq!(phases(&sim.drain_events(), who), [(0, "lift".to_owned())]);
    assert_eq!(sim.actions_of(who).len(), 1);
    sim.set_clock_rate(ClockTarget::Area(area), Some(Real::ONE), Cause::DEBUG)
        .unwrap();
    until(&mut sim, 10 * SECOND);
    let events = sim.drain_events();
    assert_eq!(
        phases(&events, who),
        [(5 * SECOND + 200 * MS, "land".to_owned())]
    );
    assert_eq!(var(&sim, who, "paces"), Value::Int(1));
    assert!(sim.actions_of(who).is_empty());
}

/// More phases of no length in one tick than `rules.max_transitions`
/// end the action with an error.
#[test]
fn a_loop_of_phases_of_no_length_ends_at_the_limit() {
    for (limit, entered) in [(None, 1_000), (Some(10), 10)] {
        let mut sim = sim();
        if let Some(limit) = limit {
            sim.set_setting("rules.max_transitions", limit);
        }
        let who = make(&mut sim, creature().with_control(Control::Delegated), 2, 2);
        sim.drain_events();
        act(&mut sim, who, "spin").unwrap();
        let events = sim.drain_events();
        assert_eq!(phases(&events, who).len(), entered);
        let interrupted: Vec<&str> = acted(&events)
            .into_iter()
            .filter_map(|(_, event)| match event {
                ActEvent::ActionInterrupted { reason, .. } => Some(reason.code.as_str()),
                _ => None,
            })
            .collect();
        assert_eq!(interrupted, ["engine:action_transition_limit"]);
        assert_eq!(var(&sim, who, "interrupted"), Value::Int(1));
        assert!(sim.actions_of(who).is_empty());
    }
}

/// A held phase ticks, ends when its `until` holds or when it is let go,
/// and goes where its `next` says.
#[test]
fn a_held_phase_ticks_until_it_is_let_go() {
    // Let go after four ticks: it is released.
    let mut sim = sim();
    let who = make(&mut sim, creature().with_control(Control::Delegated), 2, 2);
    act(&mut sim, who, "charge").unwrap();
    until(&mut sim, 1_100 * MS);
    let id = sim.actions_of(who)[0];
    sim.submit(Command::Cancel {
        entity: who,
        action: id,
        abandon: false,
    })
    .unwrap();
    until(&mut sim, 5 * SECOND);
    assert_eq!(var(&sim, who, "power"), Value::Int(4));

    // Let go after two: it fizzles.
    let mut sim = self::sim();
    let who = make(&mut sim, creature().with_control(Control::Delegated), 2, 2);
    act(&mut sim, who, "charge").unwrap();
    until(&mut sim, 600 * MS);
    let id = sim.actions_of(who)[0];
    sim.submit(Command::Cancel {
        entity: who,
        action: id,
        abandon: false,
    })
    .unwrap();
    assert_eq!(var(&sim, who, "power"), Value::Int(0));

    // Left alone: ten ticks, then `until` holds.
    let mut sim = self::sim();
    let who = make(&mut sim, creature().with_control(Control::Delegated), 2, 2);
    sim.drain_events();
    act(&mut sim, who, "charge").unwrap();
    until(&mut sim, 5 * SECOND);
    assert_eq!(var(&sim, who, "power"), Value::Int(10));
    assert_eq!(
        phases(&sim.drain_events(), who),
        [(0, "gather".to_owned()), (2_500 * MS, "release".to_owned())]
    );

    // Given up: cancelled.
    let mut sim = self::sim();
    let who = make(&mut sim, creature().with_control(Control::Delegated), 2, 2);
    act(&mut sim, who, "charge").unwrap();
    until(&mut sim, 300 * MS);
    let id = sim.actions_of(who)[0];
    sim.submit(Command::Cancel {
        entity: who,
        action: id,
        abandon: true,
    })
    .unwrap();
    assert_eq!(var(&sim, who, "cancelled"), Value::Int(0));
    assert_eq!(var(&sim, who, "power"), Value::Int(-1));
    assert!(sim.actions_of(who).is_empty());
}

// ---- T3: slots, cancelling, refusals ---------------------------------------------

/// Scenario `one_arm`: disabling a slot interrupts the action using it
/// and blocks the actions that need it.
#[test]
fn one_arm() {
    let mut sim = sim();
    let who = make(&mut sim, creature().with_control(Control::Delegated), 2, 2);
    let foe = make(&mut sim, creature(), 3, 2);
    // An entity with no body has one slot of each channel: two arms is
    // more than it has.
    assert_eq!(
        refused(act(&mut sim, who, "hug")),
        "engine:action_channel_lacking"
    );
    let params = target(&sim, foe);
    act_with(&mut sim, who, "punch", params.clone(), How::Start).unwrap();
    until(&mut sim, 100 * MS);
    sim.drain_events();
    let rule = sim.rule_set().lookup("acts:trip").unwrap();
    let me = sim.subject_value(Subject::Entity(who));
    sim.call_rule(rule, &[("self", me.clone())], &[], Cause::DEBUG)
        .unwrap();
    sim.pump();
    let events = sim.drain_events();
    let interrupted: Vec<(&str, &str)> = acted(&events)
        .into_iter()
        .filter_map(|(_, event)| match event {
            ActEvent::ActionInterrupted { reason, phase, .. } => {
                Some((reason.code.as_str(), phase.as_str()))
            }
            _ => None,
        })
        .collect();
    assert_eq!(interrupted, [("engine:action_channel_disabled", "windup")]);
    assert_eq!(var(&sim, who, "interrupted"), Value::Int(1));
    let error = act_with(&mut sim, who, "punch", params.clone(), How::Start).unwrap_err();
    assert_eq!(error.reason.code, "engine:action_channel_lacking");
    assert_eq!(
        error.reason.value("enabled"),
        Some(&clash_core::Value::Int(0))
    );
    // Given back, the arm can be used again.
    let rule = sim.rule_set().lookup("acts:untrip").unwrap();
    sim.call_rule(rule, &[("self", me)], &[], Cause::DEBUG)
        .unwrap();
    act_with(&mut sim, who, "punch", params, How::Start).unwrap();
    // An action on another channel, in the background, runs beside it.
    act(&mut sim, who, "shout").unwrap();
    assert_eq!(sim.actions_of(who).len(), 2);
}

/// A phase's `cancel` decides whether `Cancel` may end it, and a
/// recovery that allows it for a follow-up gives way to one.
#[test]
fn cancelling_follows_the_phase() {
    let mut sim = sim();
    let who = make(&mut sim, creature().with_control(Control::Delegated), 2, 2);
    let foe = make(&mut sim, creature(), 3, 2);
    let params = target(&sim, foe);
    act_with(&mut sim, who, "punch", params.clone(), How::Start).unwrap();
    let id = sim.actions_of(who)[0];
    // In the windup it may be abandoned.
    sim.submit(Command::Cancel {
        entity: who,
        action: id,
        abandon: true,
    })
    .unwrap();
    assert_eq!(var(&sim, who, "cancelled"), Value::Int(1));
    assert!(sim.actions_of(who).is_empty());

    // In the recovery, only for a follow-up.
    act_with(&mut sim, who, "punch", params, How::Start).unwrap();
    until(&mut sim, 400 * MS);
    let id = sim.actions_of(who)[0];
    let error = sim
        .submit(Command::Cancel {
            entity: who,
            action: id,
            abandon: true,
        })
        .unwrap_err();
    assert_eq!(error.reason.code, "engine:action_cancel_only_for");
    assert_eq!(
        refused(act(&mut sim, who, "hook")),
        "engine:action_channel_busy"
    );
    // The jab is a follow-up: the recovery is cut short for it.
    act(&mut sim, who, "jab").unwrap();
    assert_eq!(var(&sim, who, "cancelled"), Value::Int(2));
    assert_eq!(sim.actions_of(who).len(), 1);
    until(&mut sim, 2 * SECOND);
    assert_eq!(var(&sim, who, "hits"), Value::Int(2));
    // Nothing is running: nothing to cancel.
    let error = sim
        .submit(Command::Cancel {
            entity: who,
            action: id,
            abandon: true,
        })
        .unwrap_err();
    assert_eq!(error.reason.code, "action.not_running");
}

/// Every refusal of a start carries its reason, and changes nothing.
#[test]
fn every_refusal_carries_its_reason() {
    let mut sim = sim();
    let who = make(&mut sim, creature().with_control(Control::Delegated), 2, 2);
    let nobody = make(&mut sim, creature(), 4, 4);
    let cell = |x: i32| sim.subject_value(Subject::Loc(at(x, 1))).to_stored();
    let route =
        |cells: Vec<clash_core::Value>| vec![("route".to_owned(), clash_core::Value::List(cells))];
    let cases: Vec<(&str, Params, &str)> = vec![
        ("punch", Vec::new(), "engine:action_parameter_missing"),
        (
            "punch",
            vec![("target".to_owned(), clash_core::Value::Int(5))],
            "engine:action_parameter_invalid",
        ),
        (
            "jab",
            vec![("force".to_owned(), clash_core::Value::Int(5))],
            "engine:action_parameter_invalid",
        ),
        ("trace", route(Vec::new()), "engine:action_list_length"),
        (
            "trace",
            route(vec![cell(1), cell(2), cell(3), cell(4)]),
            "engine:action_list_length",
        ),
        (
            "trace",
            route(vec![clash_core::Value::Int(1)]),
            "engine:action_parameter_invalid",
        ),
        (
            "count_off",
            vec![("count".to_owned(), clash_core::Value::Int(0))],
            "engine:action_requires",
        ),
    ];
    let hash = sim.state_hash();
    for (name, params, code) in cases {
        let error = act_with(&mut sim, who, name, params, How::Start).unwrap_err();
        assert_eq!(error.reason.code, code, "{name}");
        assert!(!error.reason.message.is_empty());
        assert_eq!(sim.state_hash(), hash, "{name} changed the state");
    }
    // `fail_text` is what a failed `requires` says.
    let error = act_with(
        &mut sim,
        who,
        "count_off",
        vec![("count".to_owned(), clash_core::Value::Int(0))],
        How::Start,
    )
    .unwrap_err();
    assert_eq!(error.reason.message, "There is nothing to count.");
    // Nobody controls the other: it takes no commands.
    assert_eq!(
        refused(act(&mut sim, nobody, "jab")),
        "command.not_controlled"
    );
    assert!(sim.input_log().len() == 2);
}

/// `footing`: an action that needs it is refused to an actor with
/// nothing under it; one without it runs.
#[test]
fn footing() {
    let mut sim = sim();
    let who = make(&mut sim, creature().with_control(Control::Delegated), 2, 2);
    let air = Place::at(Location::new(GRID, CellPos::new(2, 2, 1)));
    debug(
        &mut sim,
        DebugCommand::MoveEntity {
            entity: who,
            to: air,
        },
    )
    .unwrap();
    let error = act(&mut sim, who, "lunge").unwrap_err();
    assert_eq!(error.reason.code, "engine:action_no_footing");
    act(&mut sim, who, "float").unwrap();
}

/// `requires_at_start` is tested when the action begins: if it fails the
/// action has begun and is interrupted at once.
#[test]
fn requires_at_start_interrupts_at_once() {
    let mut sim = sim();
    let who = make(&mut sim, creature().with_control(Control::Delegated), 2, 2);
    sim.set_var(
        Subject::Entity(who),
        "acts:nerve",
        Value::Int(0),
        Cause::DEBUG,
    )
    .unwrap();
    sim.drain_events();
    act(&mut sim, who, "lunge").unwrap();
    let events = sim.drain_events();
    let names: Vec<&str> = acted(&events)
        .iter()
        .map(|(_, event)| event.name())
        .collect();
    assert_eq!(
        names,
        ["action_started", "action_interrupted", "decision_needed"]
    );
    assert_eq!(var(&sim, who, "interrupted"), Value::Int(1));
}

/// A handler of `action_starting` that cancels refuses the action with
/// its reason; the command was taken, and `action_refused` says why.
#[test]
fn a_handler_of_action_starting_refuses() {
    let mut sim = sim();
    let who = make(
        &mut sim,
        creature()
            .with_control(Control::Delegated)
            .with_tag("tired"),
        2,
        2,
    );
    sim.drain_events();
    act(&mut sim, who, "jab").unwrap();
    assert!(sim.actions_of(who).is_empty());
    let events = sim.drain_events();
    let refusals: Vec<&str> = acted(&events)
        .into_iter()
        .filter_map(|(_, event)| match event {
            ActEvent::ActionRefused { reason, action, .. } if action == "acts:jab" => {
                Some(reason.code.as_str())
            }
            _ => None,
        })
        .collect();
    assert_eq!(refusals, ["acts:too_tired"]);
}

/// The review addition "List parameters": a list is held to its bounds,
/// indexable in the action's expressions, and survives a save in the
/// middle of the action; `check` never refuses by itself.
#[test]
fn list_parameters_survive_a_save() {
    let mut sim = sim();
    let who = make(&mut sim, creature().with_control(Control::Delegated), 2, 2);
    let cells: Vec<clash_core::Value> = [5, 6]
        .map(|x| sim.subject_value(Subject::Loc(at(x, 1))).to_stored())
        .to_vec();
    act_with(
        &mut sim,
        who,
        "trace",
        vec![("route".to_owned(), clash_core::Value::List(cells))],
        How::Start,
    )
    .unwrap();
    until(&mut sim, 500 * MS);
    let (mut loaded, unplaced) = save_and_load(&sim);
    assert!(unplaced.is_empty());
    assert_eq!(loaded.state_hash(), sim.state_hash());
    for sim in [&mut sim, &mut loaded] {
        until(sim, 2 * SECOND);
        assert_eq!(var(sim, who, "route_len"), Value::Int(2));
        assert_eq!(
            var(sim, who, "route_start"),
            sim.subject_value(Subject::Loc(at(5, 1)))
        );
    }
    assert_eq!(loaded.state_hash(), sim.state_hash());
}

/// An actor that leaves the world, or is removed, abandons what it was
/// doing: `on_abandon` runs, and nobody is asked anything.
#[test]
fn leaving_the_world_abandons_actions() {
    let mut sim = sim();
    let who = make(&mut sim, creature().with_control(Control::Direct), 2, 2);
    settle(&mut sim, who);
    let foe = make(&mut sim, creature(), 3, 2);
    let params = target(&sim, foe);
    act_with(&mut sim, who, "punch", params.clone(), How::Start).unwrap();
    sim.drain_events();
    debug(
        &mut sim,
        DebugCommand::MoveEntity {
            entity: who,
            to: Place::Stored,
        },
    )
    .unwrap();
    let names: Vec<&str> = acted(&sim.drain_events())
        .iter()
        .map(|(_, event)| event.name())
        .collect();
    assert_eq!(names, ["action_abandoned"]);
    assert_eq!(var(&sim, who, "abandoned"), Value::Int(1));
    assert!(sim.decisions().is_empty());
    assert!(sim.truth().wakes_of(who).is_empty());
    // Back in the world, it is asked again.
    debug(
        &mut sim,
        DebugCommand::MoveEntity {
            entity: who,
            to: on(2, 2),
        },
    )
    .unwrap();
    assert!(matches!(
        sim.pending_decision().map(|request| request.kind),
        Some(DecisionKind::Free {
            why: FreeReason::Started
        })
    ));
    settle(&mut sim, who);
    act_with(&mut sim, who, "punch", params, How::Start).unwrap();
    sim.drain_events();
    debug(&mut sim, DebugCommand::RemoveEntity { entity: who }).unwrap();
    let names: Vec<&str> = acted(&sim.drain_events())
        .iter()
        .map(|(_, event)| event.name())
        .collect();
    assert_eq!(names, ["action_abandoned"]);
    assert!(sim.actions.is_empty());
}

/// A running action, a plan and a wait survive a save and a load, and
/// the input log replays to the same state.
#[test]
fn acting_survives_a_save_and_a_replay() {
    let script = |sim: &mut Sim| {
        let who = make(sim, creature().with_control(Control::Delegated), 2, 2);
        let other = make(sim, creature().with_control(Control::Delegated), 6, 6);
        act(sim, who, "charge").unwrap();
        act(sim, other, "step").unwrap();
        act_with(sim, other, "step", Vec::new(), How::Enqueue).unwrap();
        until(sim, 700 * MS);
        (who, other)
    };
    let mut sim = sim();
    let (who, other) = script(&mut sim);
    // The input log replays to the state of its last command.
    let mut replayed = self::sim();
    replayed.replay(sim.input_log()).unwrap();
    until(&mut replayed, 700 * MS);
    assert_eq!(replayed.state_hash(), sim.state_hash());
    let (mut loaded, _) = save_and_load(&sim);
    assert_eq!(loaded.state_hash(), sim.state_hash());
    for sim in [&mut sim, &mut loaded, &mut replayed] {
        until(sim, 4 * SECOND);
        assert_eq!(var(sim, who, "power"), Value::Int(10));
        assert_eq!(var(sim, other, "paces"), Value::Int(2));
    }
    assert_eq!(loaded.state_hash(), sim.state_hash());
    assert_eq!(replayed.state_hash(), sim.state_hash());
}

/// An `ask` in an action's list is answered after a save: the choice
/// finds where it is written again.
#[test]
fn a_choice_put_by_an_action_is_answered_after_a_save() {
    let mut sim = sim();
    let who = make(&mut sim, creature().with_control(Control::Direct), 2, 2);
    settle(&mut sim, who);
    act(&mut sim, who, "ponder").unwrap();
    let Some(DecisionKind::Choice { choice, .. }) = sim.pending_decision().map(|r| r.kind) else {
        panic!("a choice was due");
    };
    let (mut loaded, _) = save_and_load(&sim);
    for sim in [&mut sim, &mut loaded] {
        sim.submit(Command::Choose {
            entity: who,
            choice,
            option: 0,
        })
        .unwrap();
        assert_eq!(var(sim, who, "answers"), Value::Int(2));
        assert_eq!(faults(&sim.drain_events()), Vec::<String>::new());
    }
    assert_eq!(loaded.state_hash(), sim.state_hash());
}

/// The verbs of 03 §13 from rules: each raises what it says.
#[test]
fn the_verbs_of_actions() {
    let mut sim = sim();
    let who = make(&mut sim, creature().with_control(Control::Delegated), 2, 2);
    let me = sim.subject_value(Subject::Entity(who));
    let call = |sim: &mut Sim, rule: &str| {
        let id = sim.rule_set().lookup(&format!("acts:{rule}")).unwrap();
        sim.call_rule(id, &[("self", me.clone())], &[], Cause::DEBUG)
            .unwrap();
        sim.pump();
        sim.drain_events()
    };
    let names = |events: &[SimEvent]| -> Vec<String> {
        acted(events)
            .iter()
            .map(|(_, event)| event.name().to_owned())
            .collect()
    };
    // `start_action` that cannot start raises `action_refused`.
    let events = call(&mut sim, "try_hug");
    assert_eq!(names(&events), ["action_refused"]);
    assert_eq!(faults(&events), Vec::<String>::new());
    // `enqueue_action` plans; the first starts at once.
    let events = call(&mut sim, "plan_two");
    assert_eq!(names(&events), ["action_started", "phase_entered"]);
    assert_eq!(sim.truth().entity(who).unwrap().plan.len(), 1);
    // `interrupt_plan` empties the plan and asks.
    let events = call(&mut sim, "rethink");
    assert_eq!(names(&events), ["plan_cleared", "decision_needed"]);
    // `interrupt` by channel.
    let events = call(&mut sim, "stop_all");
    assert_eq!(names(&events), ["action_interrupted", "decision_needed"]);
    assert_eq!(var(&sim, who, "interrupted"), Value::Int(0));
    // `cancel_action` cancels only where the phase allows it.
    act(&mut sim, who, "charge").unwrap();
    sim.drain_events();
    let events = call(&mut sim, "drop_it");
    assert_eq!(names(&events), ["action_cancelled", "decision_needed"]);
    // `wait` occupies; when it is over the entity is free again.
    let events = call(&mut sim, "rest");
    assert!(names(&events).is_empty());
    assert!(!sim.is_free(who));
    until(&mut sim, 2_500 * MS);
    let events = sim.drain_events();
    let free: Vec<i64> = acted(&events)
        .into_iter()
        .filter(|(_, event)| matches!(event, ActEvent::DecisionNeeded { .. }))
        .map(|(tick, _)| tick)
        .collect();
    assert_eq!(free, [2 * SECOND]);
}

// ---- T4: control, plans, free decisions -----------------------------------------

/// Package T4, as the brief states it: a character starts a two-phase
/// action from one command, the question comes back exactly when it
/// ends, and a second character on a faster clock interleaves.
#[test]
fn a_two_phase_action_from_one_command() {
    let mut sim = sim();
    let player = make(&mut sim, creature().with_control(Control::Direct), 2, 2);
    // The player is asked first, as one that has just started.
    let Advance::NeedInput(request) = sim.advance(AdvanceLimit::UntilDecision) else {
        panic!("the player is asked first");
    };
    assert_eq!(
        (request.entity, request.kind.clone()),
        (
            player,
            DecisionKind::Free {
                why: FreeReason::Started
            }
        )
    );
    // A pacer, on a clock twice as fast, paces whenever it is free: a
    // rule stands in for the AI.
    let pacer = make(&mut sim, creature().with_tag("pacer"), 8, 8);
    sim.set_time_rate(pacer, Some(Real::of(2.0)), Cause::DEBUG)
        .unwrap();
    debug(
        &mut sim,
        DebugCommand::SetControl {
            entity: pacer,
            control: Control::Delegated,
        },
    )
    .unwrap();
    sim.drain_events();

    act(&mut sim, player, "step").unwrap();
    let Advance::NeedInput(request) = sim.advance(AdvanceLimit::UntilDecision) else {
        panic!("the player is asked when the step ends");
    };
    assert_eq!(sim.now(), Tick(500 * MS));
    let DecisionKind::Free {
        why: FreeReason::Finished(_),
    } = request.kind
    else {
        panic!("not a finish: {:?}", request.kind);
    };
    let events = sim.drain_events();
    assert_eq!(
        phases(&events, player),
        [(0, "lift".to_owned()), (200 * MS, "land".to_owned())]
    );
    // The pacer, which began when it came under delegated control,
    // stepped twice in that time, on its own clock.
    assert_eq!(
        phases(&events, pacer),
        [
            (100 * MS, "land".to_owned()),
            (250 * MS, "lift".to_owned()),
            (350 * MS, "land".to_owned()),
            (500 * MS, "lift".to_owned()),
        ]
    );
    assert_eq!(var(&sim, pacer, "paces"), Value::Int(2));
    // And time does not move until the player answers.
    let hash = sim.state_hash();
    assert!(matches!(
        sim.advance(AdvanceLimit::Until(Tick(9 * SECOND))),
        Advance::NeedInput(_)
    ));
    assert_eq!((sim.now(), sim.state_hash()), (Tick(500 * MS), hash));
    assert_eq!(faults(&events), Vec::<String>::new());
}

/// Scenario `party_order`: decisions raised at one tick come in entity
/// id order, and time does not move between them.
#[test]
fn party_order() {
    let mut sim = sim();
    let first = make(&mut sim, creature().with_control(Control::Direct), 2, 2);
    let second = make(&mut sim, creature().with_control(Control::Direct), 4, 4);
    settle(&mut sim, second);
    settle(&mut sim, first);
    act(&mut sim, second, "step").unwrap();
    act(&mut sim, first, "step").unwrap();
    for who in [first, second] {
        let Advance::NeedInput(request) = sim.advance(AdvanceLimit::UntilDecision) else {
            panic!("a decision was due");
        };
        assert_eq!((request.entity, sim.now()), (who, Tick(500 * MS)));
        sim.submit(Command::Wait {
            entity: who,
            dur: Dur::from_secs(1),
        })
        .unwrap();
    }
    // Both waited a second: asked again together, in order.
    let Advance::NeedInput(request) = sim.advance(AdvanceLimit::UntilDecision) else {
        panic!("a decision was due");
    };
    assert_eq!(
        (request.entity, request.kind, sim.now()),
        (
            first,
            DecisionKind::Free {
                why: FreeReason::Waited
            },
            Tick(1_500 * MS)
        )
    );
}

/// A plan runs its steps in order, and a step whose `requires` fails
/// when it comes due clears the plan and asks with the reason.
#[test]
fn a_plan_step_that_cannot_start_clears_the_plan() {
    let mut sim = sim();
    let who = make(&mut sim, creature().with_control(Control::Direct), 2, 2);
    settle(&mut sim, who);
    act_with(&mut sim, who, "step", Vec::new(), How::Enqueue).unwrap();
    act_with(&mut sim, who, "step", Vec::new(), How::Enqueue).unwrap();
    act_with(
        &mut sim,
        who,
        "count_off",
        vec![("count".to_owned(), clash_core::Value::Int(0))],
        How::Enqueue,
    )
    .unwrap();
    act_with(&mut sim, who, "step", Vec::new(), How::Enqueue).unwrap();
    sim.drain_events();
    let Advance::NeedInput(request) = sim.advance(AdvanceLimit::UntilDecision) else {
        panic!("a decision was due");
    };
    assert_eq!(sim.now(), Tick(SECOND));
    let DecisionKind::Free {
        why: FreeReason::PlanEmpty(Some(reason)),
    } = request.kind
    else {
        panic!("not an emptied plan: {:?}", request.kind);
    };
    assert_eq!(reason.code, "engine:action_requires");
    assert_eq!(var(&sim, who, "paces"), Value::Int(2));
    assert!(sim.truth().entity(who).unwrap().plan.is_empty());
    let events = sim.drain_events();
    assert!(acted(&events).iter().any(|(_, event)| matches!(
        event,
        ActEvent::PlanCleared { reason: Some(reason), .. } if reason.code == "engine:action_requires"
    )));
    // `ClearPlan` empties one that is waiting.
    act(&mut sim, who, "step").unwrap();
    act_with(&mut sim, who, "step", Vec::new(), How::Enqueue).unwrap();
    sim.submit(Command::ClearPlan { entity: who }).unwrap();
    assert!(sim.truth().entity(who).unwrap().plan.is_empty());
}

/// A delegated character takes orders and is never asked: whatever
/// decides for it is told with `decision_needed`.
#[test]
fn a_delegated_character_takes_orders_and_is_never_asked() {
    let mut sim = sim();
    let who = make(&mut sim, creature().with_control(Control::Delegated), 2, 2);
    sim.drain_events();
    act(&mut sim, who, "step").unwrap();
    // Something listens for `decision_needed` (the fixture's stand-in
    // for the AI, which answers only pacers): an entity it leaves alone
    // idles for a second at a time and is asked again.
    assert_eq!(
        sim.advance(AdvanceLimit::Until(Tick(2_600 * MS))),
        Advance::Reached(Tick(2_600 * MS))
    );
    assert!(sim.decisions().is_empty());
    let events = sim.drain_events();
    let needed: Vec<(i64, &str)> = acted(&events)
        .into_iter()
        .filter_map(|(tick, event)| match event {
            ActEvent::DecisionNeeded { why, .. } => Some((tick, clash_sim::action::free_name(why))),
            _ => None,
        })
        .collect();
    assert_eq!(
        needed,
        [
            (500 * MS, "finished"),
            (1_500 * MS, "waited"),
            (2_500 * MS, "waited")
        ]
    );
    // An order cuts the idle short.
    act(&mut sim, who, "step").unwrap();
    assert!(sim.actions.wait_of(who).is_none());
}

/// `Wait` asks again when it is over; a wait of no length answers and
/// asks nothing more.
#[test]
fn waiting_asks_again() {
    let mut sim = sim();
    let who = make(&mut sim, creature().with_control(Control::Direct), 2, 2);
    settle(&mut sim, who);
    assert_eq!(
        sim.advance(AdvanceLimit::UntilDecision),
        Advance::Reached(Tick::ZERO)
    );
    sim.submit(Command::Wait {
        entity: who,
        dur: Dur::from_secs(3),
    })
    .unwrap();
    let Advance::NeedInput(request) = sim.advance(AdvanceLimit::UntilDecision) else {
        panic!("a decision was due");
    };
    assert_eq!(sim.now(), Tick(3 * SECOND));
    assert_eq!(request.elapsed, Dur::from_secs(3));
    let error = sim
        .submit(Command::Wait {
            entity: who,
            dur: Dur::from_secs(-1),
        })
        .unwrap_err();
    assert_eq!(error.reason.code, "command.bad_argument");
}

/// Every verb, event and reason of acting is registered with what it
/// takes and carries, and stated in 03 §13 (definition of done).
#[test]
fn every_verb_event_and_reason_of_acting_is_stated() {
    // The verbs name definition types, which the loader registers.
    let sim = sim();
    let registry = sim.defs().expr_registry();
    let document =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../docs/03-time-and-actions.md");
    let text = std::fs::read_to_string(document).unwrap();
    for verb in clash_sim::action::verbs::verbs() {
        let id = registry
            .verb_id(verb.name)
            .unwrap_or_else(|| panic!("`{}` is not registered", verb.name));
        let spec = &registry.verb(id).unwrap().spec;
        assert!(!spec.doc.is_empty(), "`{}` has no documentation", verb.name);
        for arg in &spec.args {
            assert!(!arg.doc.is_empty(), "`{}`: `{}`", verb.name, arg.name);
        }
        assert!(
            text.contains(&format!("| `{}` |", verb.name)),
            "03 §13 does not state the verb `{}`",
            verb.name
        );
    }
    for info in clash_sim::action::ACT_EVENTS {
        assert!(registry.event_id(info.name).is_some(), "{}", info.name);
        assert!(
            text.contains(&format!("| `{}` |", info.name)),
            "03 §13 does not state the event `{}`",
            info.name
        );
    }
    for info in clash_sim::action::RUNTIME_REASONS {
        assert!(text.contains(&format!("`{}`", info.code)), "{}", info.code);
    }
    assert!(text.contains(&format!("`{}`", clash_sim::action::IDLE_WAIT_SETTING)));
}
