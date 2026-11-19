//! The action runtime and life together (03 §6.6, 06 §8.4; decisions
//! D-650 and D-651): a person made from the `core:person` template acts
//! only while its mind drives it. A stopped mind is refused with life's
//! reason, whether the start comes from a command, a rule or a plan; a
//! part that goes takes the actions holding its slots with it; death
//! abandons what was under way and asks no one; unconsciousness
//! interrupts it, asks no one, and waking asks again; and the `Timer`
//! domain is shared, life's timers first.

mod common;

use clash_core::{Dur, EntityId, Real};
use clash_expr::Value;
use clash_sim::action::{ActEvent, ActionDef, How};
use clash_sim::mind::VitalState;
use clash_sim::template::CharacterSpec;
use clash_sim::{
    Advance, AdvanceLimit, Command, CommandError, Control, DebugCommand, DecisionKind, EventBody,
    FreeReason, Sim, SimEvent, Subject,
};
use clash_time::{ClockId, Wake, WakeDomain, WakeId};
use clash_world::Cause;
use common::{creature, debug, names, on};

/// Actions that count how they end, in the namespace of the definitions
/// `common::life_defs` loads: one that needs both arms and one that
/// needs nothing.
const ACTIONS: &str = r#"[
  { "type": "variable", "id": "finished", "scope": "entity", "value": "int", "default": 0, "doc": "Actions finished." },
  { "type": "variable", "id": "interrupted", "scope": "entity", "value": "int", "default": 0, "doc": "Actions interrupted." },
  { "type": "variable", "id": "abandoned", "scope": "entity", "value": "int", "default": 0, "doc": "Actions abandoned." },
  { "type": "variable", "id": "timers", "scope": "entity", "value": "int", "default": 0, "doc": "Timers of a test's own handled." },
  { "type": "action", "id": "heave", "uses": { "arm": 2 },
    "phases": [ { "id": "lift", "time": "2 s" } ],
    "on_finish": [ { "add": { "self.var.finished": 1 } } ],
    "on_interrupt": [ { "add": { "self.var.interrupted": 1 } } ],
    "on_abandon": [ { "add": { "self.var.abandoned": 1 } } ] },
  { "type": "action", "id": "hum",
    "phases": [ { "id": "tune", "time": "2 s" } ],
    "on_finish": [ { "add": { "self.var.finished": 1 } } ],
    "on_interrupt": [ { "add": { "self.var.interrupted": 1 } } ],
    "on_abandon": [ { "add": { "self.var.abandoned": 1 } } ] }
]"#;

fn sim(seed: u64) -> Sim {
    common::sim_over(common::life_defs(ACTIONS), seed)
}

/// A person, standing in the world under the player's control, with
/// the question it is asked on coming under control answered.
fn person(sim: &mut Sim, x: i32, y: i32) -> EntityId {
    let who = sim
        .spawn_template(&CharacterSpec::of(["core:person"]), on(x, y), Cause::DEBUG)
        .unwrap_or_else(|reason| panic!("{reason:?}"))
        .entity;
    debug(
        sim,
        DebugCommand::SetControl {
            entity: who,
            control: Control::Direct,
        },
    )
    .unwrap();
    settle(sim, who);
    sim.drain_events();
    who
}

/// Answers the free decision a directly controlled entity is waiting
/// for, as a frontend does when it lets a moment pass.
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

fn act_how(sim: &mut Sim, who: EntityId, name: &str, how: How) -> Result<(), CommandError> {
    let action = sim
        .defs()
        .lookup::<ActionDef>(&format!("core:{name}"))
        .unwrap_or_else(|| panic!("no action {name}"));
    sim.submit(Command::Act {
        entity: who,
        action,
        params: Vec::new(),
        how,
    })
}

fn act(sim: &mut Sim, who: EntityId, name: &str) -> Result<(), CommandError> {
    act_how(sim, who, name, How::Start)
}

fn count(sim: &Sim, who: EntityId, name: &str) -> i64 {
    sim.var(Subject::Entity(who), &format!("core:{name}"))
        .as_int()
        .unwrap_or(0)
}

fn pain(sim: &mut Sim, who: EntityId, amount: f64) {
    let pool = sim.stat_defs().unwrap().pool_id("core:pain").unwrap();
    sim.pool_add(who, pool, Real::of(amount), Cause::DEBUG)
        .unwrap();
    sim.pump();
}

/// Why each action of an actor was interrupted, by code, in order.
fn interruptions(events: &[SimEvent], who: EntityId) -> Vec<String> {
    events
        .iter()
        .filter_map(|event| match &event.body {
            EventBody::Act(ActEvent::ActionInterrupted { actor, reason, .. }) if *actor == who => {
                Some(reason.code.clone())
            }
            _ => None,
        })
        .collect()
}

/// Whether anything asked for a decision among the events.
fn asked(events: &[SimEvent]) -> bool {
    names(events)
        .iter()
        .any(|name| *name == "decision_requested" || *name == "decision_needed")
}

#[test]
fn a_mind_that_has_stopped_is_refused_with_lifes_reason() {
    let mut sim = sim(1);
    let ada = person(&mut sim, 5, 5);
    act(&mut sim, ada, "hum").unwrap();
    assert_eq!(sim.actions_of(ada).len(), 1);

    // Knocked out: no command, no rule and no plan starts anything.
    pain(&mut sim, ada, 95.0);
    assert_eq!(sim.vital(ada), Some(VitalState::Unconscious));
    let hum = sim.defs().lookup::<ActionDef>("core:hum").unwrap();
    for how in [How::Start, How::Enqueue] {
        let refused = act_how(&mut sim, ada, "hum", how).unwrap_err();
        assert_eq!(refused.reason.code, "entity_unconscious");
    }
    let refused = sim
        .start_action(ada, hum, &Vec::new(), Cause::DEBUG)
        .unwrap_err();
    assert_eq!(refused.code, "entity_unconscious");
    assert_eq!(
        sim.life_lets_act(ada).unwrap_err().code,
        "entity_unconscious"
    );

    // Dead: the player no longer controls it, and a rule cannot make it
    // act either.
    sim.die(ada, Cause::DEBUG).unwrap();
    assert_eq!(
        act(&mut sim, ada, "hum").unwrap_err().reason.code,
        "command.not_controlled"
    );
    let refused = sim
        .start_action(ada, hum, &Vec::new(), Cause::DEBUG)
        .unwrap_err();
    assert_eq!(refused.code, "entity_dead");

    // A body with no mind does nothing unless something drives it.
    let mut sim = self::sim(7);
    let bea = person(&mut sim, 5, 5);
    let cal = person(&mut sim, 6, 5);
    sim.remove_mind(bea, Cause::DEBUG).unwrap();
    let refused = sim
        .start_action(bea, hum, &Vec::new(), Cause::DEBUG)
        .unwrap_err();
    assert_eq!(refused.code, "no_mind");
    sim.set_driver(bea, Some(cal), Cause::DEBUG).unwrap();
    sim.start_action(bea, hum, &Vec::new(), Cause::DEBUG)
        .unwrap();
    assert_eq!(sim.actions_of(bea).len(), 1);

    // Something with neither a mind nor a body is not life's to stop
    // (D-650): a tool's or a test's creature, a device.
    let thing = sim.truth().entities().next_id().unwrap();
    debug(
        &mut sim,
        DebugCommand::CreateEntity {
            spec: creature().with_control(Control::Delegated),
            place: on(7, 7),
        },
    )
    .unwrap();
    assert!(sim.vital(thing).is_none());
    act(&mut sim, thing, "hum").unwrap();
    assert_eq!(sim.actions_of(thing).len(), 1);
}

#[test]
fn losing_an_arm_interrupts_what_held_its_slot() {
    let mut sim = sim(2);
    let ada = person(&mut sim, 5, 5);
    act(&mut sim, ada, "heave").unwrap();
    act(&mut sim, ada, "hum").unwrap();
    sim.drain_events();
    let arm = sim
        .components()
        .body
        .get(ada)
        .unwrap()
        .part_named("l_arm")
        .unwrap()
        .key();
    sim.disable_part(ada, arm, Cause::DEBUG).unwrap();
    sim.pump();
    let events = sim.drain_events();
    let told = names(&events);
    let at = |name: &str| told.iter().position(|have| *have == name).unwrap();
    assert!(at("slots_lost") < at("action_interrupted"), "{told:?}");
    // The action that needed both arms is interrupted with the reason;
    // the one that needs none goes on.
    assert_eq!(
        interruptions(&events, ada),
        ["engine:action_channel_disabled"]
    );
    assert_eq!(count(&sim, ada, "interrupted"), 1);
    assert_eq!(sim.actions_of(ada).len(), 1);
    // It cannot start again with one arm.
    assert_eq!(
        act(&mut sim, ada, "heave").unwrap_err().reason.code,
        "engine:action_channel_lacking"
    );
    // The hum ends in its own time, and the player is asked.
    let Advance::NeedInput(request) = sim.advance(AdvanceLimit::UntilDecision) else {
        panic!("the player is asked when the hum ends");
    };
    assert_eq!(request.entity, ada);
    assert_eq!(count(&sim, ada, "finished"), 1);
}

#[test]
fn death_abandons_what_was_under_way_and_asks_no_one() {
    let mut sim = sim(3);
    let ada = person(&mut sim, 5, 5);
    act(&mut sim, ada, "heave").unwrap();
    act_how(&mut sim, ada, "hum", How::Enqueue).unwrap();
    sim.drain_events();

    sim.die(ada, Cause::DEBUG).unwrap();
    sim.pump();
    let events = sim.drain_events();
    let told = names(&events);
    assert!(told.contains(&"entity_died"), "{told:?}");
    assert!(told.contains(&"action_abandoned"), "{told:?}");
    assert!(told.contains(&"plan_cleared"), "{told:?}");
    assert!(!asked(&events), "{told:?}");
    assert_eq!(count(&sim, ada, "abandoned"), 1);
    assert_eq!(count(&sim, ada, "interrupted"), 0);
    assert!(sim.actions_of(ada).is_empty());
    // Nothing is left for it on the timeline, and nothing more happens.
    assert_eq!(
        sim.advance(AdvanceLimit::UntilDecision),
        Advance::Reached(sim.now())
    );
    let after = sim.drain_events();
    assert!(!asked(&after), "{:?}", names(&after));
    assert_eq!(count(&sim, ada, "finished"), 0);
}

#[test]
fn unconsciousness_interrupts_without_asking_and_waking_asks() {
    let mut sim = sim(4);
    let ada = person(&mut sim, 5, 5);
    act(&mut sim, ada, "heave").unwrap();
    sim.drain_events();
    let fell = sim.now();

    pain(&mut sim, ada, 95.0);
    let events = sim.drain_events();
    assert!(names(&events).contains(&"entity_unconscious"));
    assert_eq!(interruptions(&events, ada), ["entity_unconscious"]);
    assert!(!asked(&events), "{:?}", names(&events));
    assert_eq!(count(&sim, ada, "interrupted"), 1);
    assert!(sim.actions_of(ada).is_empty());
    assert!(sim.pending_decision().is_none());

    // The pain fades by its own rate; when it falls under the mark the
    // person wakes, on a timer of the pool's, and is asked.
    let Advance::NeedInput(request) = sim.advance(AdvanceLimit::UntilDecision) else {
        panic!("the person is asked on waking");
    };
    assert_eq!(request.entity, ada);
    assert_eq!(
        request.kind,
        DecisionKind::Free {
            why: FreeReason::Started
        }
    );
    assert_eq!(sim.vital(ada), Some(VitalState::Awake));
    // From 95 to under 90 at half a point a second.
    let slept = sim.now().since(fell).as_secs_real();
    assert!(slept > Real::of(9.9) && slept < Real::of(10.1), "{slept}");
    let events = sim.drain_events();
    let told = names(&events);
    assert!(told.contains(&"entity_woke"), "{told:?}");
    assert!(told.contains(&"decision_requested"), "{told:?}");
    act(&mut sim, ada, "heave").unwrap();
}

/// A handler of the `Timer` domain of a test's own: it counts what it
/// is handed on the entity whose bits the timer carries.
fn own_timer(sim: &mut Sim, _id: WakeId, wake: Wake) {
    let who = Subject::Entity(EntityId::from_bits(wake.id));
    let had = sim.var(who, "core:timers").as_int().unwrap_or(0);
    sim.set_var(who, "core:timers", Value::Int(had + 1), Cause::DEBUG)
        .unwrap();
}

#[test]
fn the_timer_domain_is_shared_with_life_first() {
    let mut sim = sim(5);
    let ada = person(&mut sim, 5, 5);
    sim.set_wake_handler(WakeDomain::Timer, own_timer);
    // A timer of the test's own, and the pool's, which wakes the person.
    sim.schedule_on(
        ClockId::UNIVERSAL,
        Dur::from_secs(1),
        Wake::new(WakeDomain::Timer, ada.to_bits(), 7),
    )
    .unwrap();
    pain(&mut sim, ada, 95.0);
    sim.drain_events();
    assert!(matches!(
        sim.advance(AdvanceLimit::UntilDecision),
        Advance::NeedInput(_)
    ));
    let events = sim.drain_events();
    let told = names(&events);
    assert!(told.contains(&"entity_woke"), "{told:?}");
    assert!(!told.contains(&"fault"), "{told:?}");
    assert_eq!(count(&sim, ada, "timers"), 1, "the test's own timer");

    // With no handler of its own, a timer that is not life's is
    // reported, and life's still run.
    let mut sim = self::sim(6);
    let bea = person(&mut sim, 5, 5);
    sim.schedule_on(
        ClockId::UNIVERSAL,
        Dur::from_secs(1),
        Wake::new(WakeDomain::Timer, bea.to_bits(), 7),
    )
    .unwrap();
    pain(&mut sim, bea, 95.0);
    sim.drain_events();
    assert!(matches!(
        sim.advance(AdvanceLimit::UntilDecision),
        Advance::NeedInput(_)
    ));
    let events = sim.drain_events();
    let faults: Vec<&SimEvent> = events
        .iter()
        .filter(|event| event.name() == "fault")
        .collect();
    assert_eq!(faults.len(), 1, "{:?}", names(&events));
    assert!(names(&events).contains(&"entity_woke"));
}
