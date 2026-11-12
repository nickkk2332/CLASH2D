//! The rule runtime bound to the simulation (plans/sim-kernel.md,
//! package K5): the bodies of the test scripts `rule_alarm` and
//! `winter_rules` against a real simulation, from fixture mods loaded
//! through the loader (`tests/fixtures/act/mods`).
//!
//! What the binding is done by: a rule reacts to a kernel event, sets a
//! variable on a real entity, runs another rule and emits a hook that a
//! second rule cancels with a reason; a save in the middle changes
//! nothing; replaying the input log gives the same hash.

mod common;

use clash_core::EntityId;
use clash_expr::Value;
use clash_sim::{DebugCommand, EntitySpec, EventBody, Sim, SimEvent, Subject};
use clash_world::Cause;
use common::{act_fixtures, at, creature, debug, defs_with, names, on, save_and_load, sim_over};

fn sim_of(mods: &[&str]) -> Sim {
    sim_over(defs_with(&act_fixtures(), mods), 3)
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

fn walk(sim: &mut Sim, who: EntityId, x: i32, y: i32) {
    debug(
        sim,
        DebugCommand::MoveEntity {
            entity: who,
            to: on(x, y),
        },
    )
    .unwrap();
}

fn global(sim: &Sim, name: &str) -> Value {
    sim.var(Subject::Global, name)
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

// ---- the test script `rule_alarm` -------------------------------------------

/// The first half of the script: a thief walks into the vault.
fn alarm_first_half(sim: &mut Sim) -> (EntityId, EntityId) {
    let thief = make(sim, creature().with_tag("thief"), 2, 2);
    let guard = make(sim, creature(), 3, 3);
    // Not in the vault, and not a thief: nothing.
    walk(sim, thief, 5, 5);
    walk(sim, guard, 12, 12);
    assert_eq!(global(sim, "alarm:alarms"), Value::Int(0));
    // The thief steps in: the rule sets a variable on the cell, adds to
    // a global one, runs a rule that sets one on the thief, and raises
    // a hook that a second rule doubles.
    walk(sim, thief, 12, 5);
    (thief, guard)
}

/// The second half: the same cell again, and then another.
fn alarm_second_half(sim: &mut Sim, thief: EntityId) {
    walk(sim, thief, 5, 5);
    walk(sim, thief, 12, 5);
    assert_eq!(global(sim, "alarm:alarms"), Value::Int(1));
    // A second alarm: this time a rule cancels the hook with a reason.
    walk(sim, thief, 13, 5);
}

#[test]
fn rule_alarm() {
    let mut sim = sim_of(&["alarm"]);
    let (thief, guard) = alarm_first_half(&mut sim);
    let cell = Subject::Loc(at(12, 5));
    assert_eq!(sim.var(cell, "alarm:tripped"), Value::Bool(true));
    assert_eq!(global(&sim, "alarm:alarms"), Value::Int(1));
    assert_eq!(
        sim.var(Subject::Entity(thief), "alarm:alerted"),
        Value::Bool(true)
    );
    assert_eq!(
        sim.var(Subject::Entity(guard), "alarm:alerted"),
        Value::Bool(false)
    );
    // Two were asked for, a handler of the hook doubled them, and the
    // rule that raised it used the changed value.
    assert_eq!(global(&sim, "alarm:squads"), Value::Int(4));
    assert_eq!(global(&sim, "alarm:refused"), Value::Null);

    // A save in the middle: the loaded simulation carries on alike.
    let (mut loaded, unplaced) = save_and_load(&sim);
    assert!(unplaced.is_empty());
    assert_eq!(loaded.state_hash(), sim.state_hash());

    let first = sim.drain_events();
    assert_eq!(faults(&first), Vec::<String>::new());
    alarm_second_half(&mut sim, thief);
    alarm_second_half(&mut loaded, thief);
    assert_eq!(loaded.state_hash(), sim.state_hash());

    assert_eq!(global(&sim, "alarm:alarms"), Value::Int(2));
    assert_eq!(global(&sim, "alarm:squads"), Value::Int(4));
    assert_eq!(
        global(&sim, "alarm:refused"),
        Value::from("alarm:no_one_left")
    );
    let second = sim.drain_events();
    assert_eq!(faults(&second), Vec::<String>::new());
    // The cancellation is reported, with the reason the rule gave.
    let cancelled: Vec<&SimEvent> = second
        .iter()
        .filter(|event| event.name() == "event_cancelled")
        .collect();
    assert_eq!(cancelled.len(), 1);
    match &cancelled[0].body {
        EventBody::EventCancelled { event, reason } => {
            assert_eq!(event, "alarm:reinforcements_due");
            assert_eq!(reason.code, "alarm:no_one_left");
            assert_eq!(reason.message, "There is nobody left to send.");
        }
        other => panic!("unexpected {other:?}"),
    }
    // The hook itself is not a notification: it was delivered at once
    // and is not among what happened.
    assert!(!names(&second).contains(&"alarm:reinforcements_due"));
    assert!(sim.rules.is_idle());

    // Replaying the input log on a simulation set up alike ends in the
    // same state (invariant T6).
    let mut replayed = sim_of(&["alarm"]);
    replayed.replay(sim.input_log()).unwrap();
    assert_eq!(replayed.state_hash(), sim.state_hash());
    assert_eq!(global(&replayed, "alarm:squads"), Value::Int(4));
}

#[test]
fn a_rule_emits_with_the_cause_filled_in() {
    let mut sim = sim_of(&["alarm"]);
    let thief = make(&mut sim, creature().with_tag("thief"), 2, 2);
    sim.drain_events();
    // A notification this time: listen for the kernel's own event that
    // the rule's write raises.
    walk(&mut sim, thief, 12, 5);
    let events = sim.drain_events();
    let changed = events
        .iter()
        .find(|event| event.name() == "cell_var_changed")
        .expect("the rule set a variable of the cell");
    // What a rule changes is caused by the rule, and the entity bound
    // as `self` where the effect is written is its actor.
    assert_eq!(changed.cause.kind, clash_world::CauseKind::Eoc);
    assert_eq!(changed.cause.actor, Some(thief));
    assert_eq!(changed.cause.instigator, Some(thief));
}

#[test]
fn attached_rules_run_for_their_owner_and_are_forgotten_with_it() {
    let mut sim = sim_of(&["alarm"]);
    let walker = make(&mut sim, creature(), 2, 2);
    let other = make(&mut sim, creature(), 3, 3);
    let rule = sim.rule_set().lookup("alarm:count_steps").unwrap();
    let handle = sim
        .subject_value(Subject::Entity(walker))
        .as_subject()
        .unwrap();
    assert!(sim.rules.attach(handle, rule));
    let steps = |sim: &Sim, who| sim.var(Subject::Entity(who), "alarm:steps");

    walk(&mut sim, walker, 4, 4);
    walk(&mut sim, other, 6, 6);
    walk(&mut sim, walker, 5, 4);
    assert_eq!(steps(&sim, walker), Value::Int(2));
    assert_eq!(steps(&sim, other), Value::Int(0));

    // Attachments are state: they survive a save and a load.
    let (mut loaded, _) = save_and_load(&sim);
    walk(&mut loaded, walker, 6, 4);
    assert_eq!(steps(&loaded, walker), Value::Int(3));

    // Removing the owner forgets what was kept for it.
    assert_eq!(sim.rules.attached_to(handle), [rule]);
    debug(&mut sim, DebugCommand::RemoveEntity { entity: walker }).unwrap();
    assert!(sim.rules.attached_to(handle).is_empty());
    assert_eq!(faults(&sim.drain_events()), Vec::<String>::new());
}

// ---- the kernel's verbs (03 §13) -------------------------------------------

/// Calls a rule of the `verbs` fixture with an entity as `self`, and
/// gives what happened.
fn verb(sim: &mut Sim, rule: &str, who: EntityId, args: &[(&str, Value)]) -> Vec<SimEvent> {
    let id = sim
        .rule_set()
        .lookup(&format!("verbs:{rule}"))
        .unwrap_or_else(|| panic!("no rule `{rule}`"));
    let me = sim.subject_value(Subject::Entity(who));
    sim.call_rule(id, &[("self", me)], args, Cause::DEBUG)
        .unwrap();
    sim.pump();
    sim.drain_events()
}

#[test]
fn each_kernel_verb_goes_through_the_engines_own_function() {
    use clash_core::Real;
    use clash_sim::Control;

    let mut sim = sim_of(&["verbs"]);
    let who = make(&mut sim, creature(), 4, 4);
    sim.drain_events();
    let half = Value::Real(Real::of(0.5));

    // set_time_rate: a personal clock, its rate, and none again.
    let events = verb(
        &mut sim,
        "haste",
        who,
        &[("rate", Value::Real(Real::of(2.0)))],
    );
    assert_eq!(names(&events), ["clock_rate_changed"]);
    assert_eq!(events[0].cause.kind, clash_world::CauseKind::Eoc);
    assert_eq!(events[0].cause.actor, Some(who));
    let clock = sim.truth().entity(who).unwrap().clock().unwrap();
    assert_eq!(sim.timeline().rate(clock), Some(Real::of(2.0)));
    assert_eq!(
        names(&verb(&mut sim, "haste", who, &[("rate", half.clone())])),
        ["clock_rate_changed"]
    );
    assert_eq!(
        names(&verb(&mut sim, "unhaste", who, &[])),
        ["clock_rate_changed"]
    );
    assert!(sim.truth().entity(who).unwrap().clock().is_none());

    // anchor_clock: gives a clock at rate 1 under the realm's, and lifts.
    let events = verb(&mut sim, "pin", who, &[]);
    assert_eq!(names(&events), ["clock_rate_changed"]);
    let realm_clock = sim
        .truth()
        .cosmos()
        .realm(common::realm(&sim))
        .unwrap()
        .clock;
    assert_eq!(sim.truth().entity(who).unwrap().anchor(), Some(realm_clock));
    assert!(verb(&mut sim, "unpin", who, &[]).is_empty());
    assert_eq!(sim.truth().entity(who).unwrap().anchor(), None);

    // set_clock_rate: the realm's clock.
    let events = verb(&mut sim, "slow_realm", who, &[]);
    assert_eq!(names(&events), ["clock_rate_changed"]);
    assert_eq!(sim.timeline().rate(realm_clock), Some(Real::of(0.5)));

    // set_control: the entity comes under direct control and, with
    // nothing to do, is asked.
    let events = verb(&mut sim, "take_over", who, &[]);
    assert_eq!(names(&events)[0], "control_changed");
    assert_eq!(sim.truth().entity(who).unwrap().control(), Control::Direct);

    // add_tag and remove_tag: `tag_changed`, once each, which a rule
    // can listen for.
    let events = verb(&mut sim, "mark", who, &[]);
    assert_eq!(names(&events), ["tag_changed"]);
    assert!(sim.truth().entity(who).unwrap().tags.contains("marked"));
    assert!(verb(&mut sim, "mark", who, &[]).is_empty());
    assert_eq!(global(&sim, "verbs:marks"), Value::Int(1));
    let events = verb(&mut sim, "unmark", who, &[]);
    assert_eq!(names(&events), ["tag_changed"]);
    assert_eq!(
        events[0].body.field("added"),
        Some(clash_sim::FieldValue::from(false))
    );
    assert!(verb(&mut sim, "unmark", who, &[]).is_empty());
    assert_eq!(sim.rule_fault_count(), 0);

    // A verb that declines says why, and the rule carries on.
    let events = verb(&mut sim, "sideways", who, &[]);
    let reported = faults(&events);
    assert_eq!(reported.len(), 1, "{reported:?}");
    assert!(reported[0].starts_with("rule.verb_refused"));
    match &events[0].body {
        EventBody::Fault { reason } => {
            assert_eq!(
                reason.value("reason"),
                Some(&clash_core::Value::from("verb.bad_argument"))
            );
            assert_eq!(
                reason.value("rule"),
                Some(&clash_core::Value::from("verbs:sideways"))
            );
        }
        other => panic!("unexpected {other:?}"),
    }
}

#[test]
fn every_kernel_verb_is_registered_with_its_arguments() {
    let registry = clash_sim::engine_registry();
    for verb in clash_sim::kernel_verbs::kernel_verbs() {
        let id = registry
            .verb_id(verb.name)
            .unwrap_or_else(|| panic!("`{}` is not registered", verb.name));
        let spec = &registry.verb(id).unwrap().spec;
        assert!(!spec.doc.is_empty(), "`{}` has no documentation", verb.name);
        assert!(!spec.args.is_empty());
        for arg in &spec.args {
            assert!(
                !arg.doc.is_empty(),
                "`{}` does not say what `{}` is",
                verb.name,
                arg.name
            );
        }
    }
    let document =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../docs/03-time-and-actions.md");
    let text = std::fs::read_to_string(document).unwrap();
    for verb in clash_sim::kernel_verbs::kernel_verbs() {
        assert!(
            text.contains(&format!("| `{}` |", verb.name)),
            "03 §13 does not state the verb `{}` with its arguments",
            verb.name
        );
    }
    for info in clash_sim::kernel_verbs::VERB_REASONS {
        assert!(text.contains(&format!("`{}`", info.code)));
    }
}

// ---- the test script `winter_rules` ------------------------------------------

#[test]
fn winter_rules() {
    let mut sim = sim_of(&["winter"]);
    let who = make(&mut sim, creature(), 2, 2);
    let chills = |sim: &Sim| sim.var(Subject::Entity(who), "winter:chills");
    let moves = |sim: &Sim| sim.var(Subject::Entity(who), "winter:moves");
    let call = |sim: &mut Sim, rule: &str| {
        let id = sim.rule_set().lookup(rule).unwrap();
        let me = sim.subject_value(Subject::Entity(who));
        sim.call_rule(id, &[("self", me)], &[], Cause::DEBUG)
            .unwrap()
    };
    // Entering the world was a move too.
    assert_eq!(moves(&sim), Value::Int(1));

    // The group is off: its rule does not run; a rule in no group does.
    walk(&mut sim, who, 3, 2);
    assert_eq!(chills(&sim), Value::Int(0));
    assert_eq!(moves(&sim), Value::Int(2));

    // A setting its `enabled` reads turns it on.
    sim.set_setting("harsh_winter", true);
    walk(&mut sim, who, 4, 2);
    assert_eq!(chills(&sim), Value::Int(1));
    sim.set_setting("harsh_winter", false);
    walk(&mut sim, who, 5, 2);
    assert_eq!(chills(&sim), Value::Int(1));
    // So does a variable.
    sim.set_var(
        Subject::Global,
        "winter:deep_winter",
        Value::Bool(true),
        Cause::DEBUG,
    )
    .unwrap();
    walk(&mut sim, who, 6, 2);
    assert_eq!(chills(&sim), Value::Int(2));
    sim.set_var(
        Subject::Global,
        "winter:deep_winter",
        Value::Bool(false),
        Cause::DEBUG,
    )
    .unwrap();

    // `set_rule_group` switches it from a rule, whatever `enabled` says.
    call(&mut sim, "winter:freeze");
    walk(&mut sim, who, 7, 2);
    assert_eq!(chills(&sim), Value::Int(3));
    call(&mut sim, "winter:thaw");
    sim.set_setting("harsh_winter", true);
    walk(&mut sim, who, 8, 2);
    assert_eq!(chills(&sim), Value::Int(3));
    // A rule of the group that is called does not run either.
    assert_eq!(call(&mut sim, "winter:shiver"), Value::Null);
    assert_eq!(chills(&sim), Value::Int(3));

    // The switch is state: it survives a save and a load.
    let (mut loaded, _) = save_and_load(&sim);
    assert_eq!(loaded.state_hash(), sim.state_hash());
    loaded.set_setting("harsh_winter", true);
    walk(&mut loaded, who, 9, 2);
    assert_eq!(chills(&loaded), Value::Int(3), "still off");

    // Left to `enabled` again, the setting governs.
    call(&mut sim, "winter:seasons");
    walk(&mut sim, who, 9, 2);
    assert_eq!(chills(&sim), Value::Int(4));
    assert_eq!(call(&mut sim, "winter:shiver"), Value::Int(1));
    assert_eq!(chills(&sim), Value::Int(14));
    assert_eq!(faults(&sim.drain_events()), Vec::<String>::new());
    assert_eq!(sim.rule_fault_count(), 0);
}
