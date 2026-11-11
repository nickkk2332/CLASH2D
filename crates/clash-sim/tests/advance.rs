//! Tests for package K2: the advance loop with domain dispatch and its
//! three limits, world wake-ups under it, commands and their refusals,
//! the input log and replay (invariant T6), saving in the middle of a
//! run (01 R7), and the decision queue (invariant T8).

mod common;

use clash_core::{Dur, EntityId, Tick};
use clash_sim::{
    Advance, AdvanceLimit, Control, DebugCommand, DecisionKind, EventBody, FreeReason, Place, Sim,
    codes,
};
use clash_time::{Wake, WakeDomain, WakeId};
use clash_world::{BlockState, Cause, Layer, WorldEvent};
use common::{SIDE, at, creature, debug, item, on, spawn, walk_wake};
use proptest::prelude::*;

const SECOND: i64 = 1_000_000;

/// K2: wake-ups are dispatched to the handler of their domain at the
/// tick they are due, and each limit stops where it says.
#[test]
fn wake_ups_run_in_order_and_each_limit_stops_where_it_says() {
    let mut sim = common::walking_sim(1);
    // Nothing is scheduled: an advance without a limit must not move time.
    assert_eq!(
        sim.advance(AdvanceLimit::UntilDecision),
        Advance::Reached(Tick(0))
    );
    assert_eq!(
        sim.advance(AdvanceLimit::Work(5)),
        Advance::Reached(Tick(0))
    );
    // An advance to a tick moves time there even so.
    assert_eq!(
        sim.advance(AdvanceLimit::Until(Tick(SECOND))),
        Advance::Reached(Tick(SECOND))
    );

    let walker = spawn(&mut sim, creature(), on(10, 10));
    sim.schedule_for(walker, Dur::from_secs(2), walk_wake(walker, 0))
        .unwrap();
    sim.schedule_for(walker, Dur::from_secs(5), walk_wake(walker, 0))
        .unwrap();
    sim.pump();
    sim.drain_events();

    // Short of the first wake-up: nothing runs.
    assert_eq!(
        sim.advance(AdvanceLimit::Until(Tick(2 * SECOND))),
        Advance::Reached(Tick(2 * SECOND))
    );
    assert_eq!(sim.steps(), 0);
    // One wake-up of work: the first runs, at its own tick, and more is due.
    assert_eq!(sim.advance(AdvanceLimit::Work(1)), Advance::Yielded);
    assert_eq!((sim.now(), sim.steps()), (Tick(3 * SECOND), 1));
    // To the end: the second runs at its tick and time stays there.
    assert_eq!(
        sim.advance(AdvanceLimit::UntilDecision),
        Advance::Reached(Tick(6 * SECOND))
    );
    assert_eq!(sim.steps(), 2);
    let moves = sim.drain_events();
    assert!(moves.iter().all(|event| event.name() == "entity_relocated"));
    assert_eq!(
        moves.iter().map(|event| event.tick).collect::<Vec<_>>(),
        moves
            .iter()
            .map(|event| event.tick)
            .filter(|tick| [Tick(3 * SECOND), Tick(6 * SECOND)].contains(tick))
            .collect::<Vec<_>>()
    );
}

/// A wake-up of a domain nothing handles is reported and dropped; it
/// does not stop the loop.
#[test]
fn a_wake_up_nothing_handles_is_a_reported_fault() {
    let mut sim = common::sim(2);
    debug(
        &mut sim,
        DebugCommand::Schedule {
            entity: None,
            after: Dur::from_secs(1),
            wake: Wake::new(WakeDomain::Timer, 7, 9),
        },
    )
    .unwrap();
    assert_eq!(
        sim.advance(AdvanceLimit::UntilDecision),
        Advance::Reached(Tick(SECOND))
    );
    let events = sim.drain_events();
    let [fault] = events.as_slice() else {
        panic!("one fault expected, got {events:?}");
    };
    let EventBody::Fault { reason } = &fault.body else {
        panic!("a fault expected, got {fault:?}");
    };
    assert_eq!(reason.code, codes::ADVANCE_UNHANDLED_WAKE);
    assert_eq!(reason.value("id"), Some(&clash_core::Value::Int(7)));
}

/// A world process for a test: at each wake it raises a wall one cell
/// further east, through the world's own edit path, and sleeps again
/// until its count runs out. `id` is the row, `arg` the cells left.
fn mason(sim: &mut Sim, _id: WakeId, wake: Wake) {
    let wall = common::wall(sim);
    let x = 20 - wake.arg as i32;
    let _ = sim.world(|ctx| {
        ctx.edit(Cause::new(clash_world::CauseKind::Process))
            .set_block(at(x, wake.id as i32), wall)
    });
    if wake.arg > 1 {
        let clock = sim.location_clock(at(x, wake.id as i32));
        let next = Wake::new(WakeDomain::World, wake.id, wake.arg - 1);
        let _ = sim.schedule_on(clock, Dur::from_millis(250), next);
    }
}

/// K2: world processes run under the same loop. The world's wake-ups
/// are dispatched like any other, what they change is reported in
/// order, and an occupant in the way is displaced and followed.
#[test]
fn world_wake_ups_run_under_the_same_loop() {
    let mut sim = common::sim(3);
    sim.set_wake_handler(WakeDomain::World, mason);
    let guard = spawn(&mut sim, creature(), on(18, 4));
    sim.schedule_on(
        sim.location_clock(at(0, 4)),
        Dur::from_millis(250),
        Wake::new(WakeDomain::World, 4, 4),
    )
    .unwrap();
    sim.pump();
    sim.drain_events();

    assert_eq!(
        sim.advance(AdvanceLimit::UntilDecision),
        Advance::Reached(Tick(SECOND))
    );
    let wall = common::wall(&sim);
    let cosmos = sim.truth().cosmos();
    for x in 16..20 {
        let cell = cosmos.cell(at(x, 4)).unwrap();
        assert_eq!(cell.block, wall, "cell {x}");
    }
    let events = sim.drain_events();
    let changed: Vec<Tick> = events
        .iter()
        .filter(|event| event.name() == "element_changed")
        .map(|event| event.tick)
        .collect();
    assert_eq!(
        changed,
        [250_000, 500_000, 750_000, 1_000_000].map(Tick),
        "one edit at each wake"
    );
    assert!(events.iter().any(|event| matches!(
        &event.body,
        EventBody::World(WorldEvent::OccupantDisplaced { occupant, .. }) if *occupant == guard
    )));
    assert_ne!(sim.truth().place(guard), Some(on(18, 4)));
    assert_eq!(
        sim.truth()
            .cosmos()
            .occupant(guard)
            .map(|record| record.loc),
        sim.truth().place(guard).and_then(Place::loc)
    );
}

/// Every refusal carries a reason, changes nothing and is not logged.
#[test]
fn a_refused_command_says_why_and_changes_nothing() {
    let mut sim = common::sim(4);
    let ghost = EntityId::new(40, 0);
    let before = sim.state_hash();
    let refusals = [
        (
            DebugCommand::MoveEntity {
                entity: ghost,
                to: on(1, 1),
            },
            codes::ENTITY_UNKNOWN,
        ),
        (
            DebugCommand::RemoveEntity { entity: ghost },
            codes::ENTITY_UNKNOWN,
        ),
        (
            DebugCommand::CreateEntity {
                spec: creature(),
                place: on(SIDE + 5, 0),
            },
            codes::WORLD_NOT_LOADED,
        ),
        (
            DebugCommand::CreateEntity {
                spec: item(),
                place: Place::In(ghost),
            },
            codes::ENTITY_UNKNOWN,
        ),
        (
            DebugCommand::SetElement {
                loc: at(0, SIDE + 9),
                layer: Layer::Block,
                state: BlockState::NONE.raw(),
            },
            codes::WORLD_NOT_LOADED,
        ),
        (
            DebugCommand::SetControl {
                entity: ghost,
                control: Control::Direct,
            },
            codes::ENTITY_UNKNOWN,
        ),
        (
            DebugCommand::Schedule {
                entity: Some(ghost),
                after: Dur::SECOND,
                wake: walk_wake(ghost, 0),
            },
            codes::ENTITY_UNKNOWN,
        ),
    ];
    for (command, code) in refusals {
        let error = debug(&mut sim, command.clone()).unwrap_err();
        assert_eq!(error.reason.code, code, "{command:?}");
        assert!(!error.to_string().is_empty());
        assert_eq!(sim.state_hash(), before, "{command:?} changed the state");
    }
    assert!(sim.input_log().is_empty());
    assert!(sim.drain_events().is_empty());

    // An accepted command is logged with its tick and takes effect.
    sim.advance(AdvanceLimit::Until(Tick(3 * SECOND)));
    let next = sim.truth().entities().next_id().unwrap();
    debug(
        &mut sim,
        DebugCommand::CreateEntity {
            spec: creature(),
            place: on(3, 3),
        },
    )
    .unwrap();
    assert_eq!(sim.truth().place(next), Some(on(3, 3)));
    let [entry] = sim.input_log().entries() else {
        panic!("one entry expected");
    };
    assert_eq!((entry.tick, entry.step), (Tick(3 * SECOND), 0));
    assert_eq!(
        common::names(&sim.drain_events()),
        ["entity_created", "entity_relocated"]
    );
}

/// One step of a random session.
#[derive(Clone, Debug)]
enum Step {
    Create {
        x: i32,
        y: i32,
        item: bool,
    },
    Move {
        who: usize,
        x: i32,
        y: i32,
    },
    Put {
        who: usize,
        into: usize,
    },
    Remove {
        who: usize,
    },
    Wall {
        x: i32,
        y: i32,
        up: bool,
    },
    Walk {
        who: usize,
        after_ms: i64,
        hops: u64,
    },
    Until {
        ms: i64,
    },
    Work {
        wakes: u32,
    },
}

fn step() -> impl Strategy<Value = Step> {
    let cell = 0..SIDE;
    prop_oneof![
        3 => (cell.clone(), cell.clone(), any::<bool>())
            .prop_map(|(x, y, item)| Step::Create { x, y, item }),
        4 => (any::<usize>(), cell.clone(), cell.clone())
            .prop_map(|(who, x, y)| Step::Move { who, x, y }),
        2 => (any::<usize>(), any::<usize>()).prop_map(|(who, into)| Step::Put { who, into }),
        1 => any::<usize>().prop_map(|who| Step::Remove { who }),
        2 => (cell.clone(), cell, any::<bool>()).prop_map(|(x, y, up)| Step::Wall { x, y, up }),
        5 => (any::<usize>(), 0..3_000_i64, 0..12_u64)
            .prop_map(|(who, after_ms, hops)| Step::Walk { who, after_ms, hops }),
        4 => (1..2_500_i64).prop_map(|ms| Step::Until { ms }),
        4 => (0..6_u32).prop_map(|wakes| Step::Work { wakes }),
    ]
}

/// Plays one step as a frontend would: commands through `submit`, time
/// through `advance`.
fn play(sim: &mut Sim, step: &Step) {
    let ids: Vec<EntityId> = sim.truth().entities().ids().collect();
    let pick = |index: usize| (!ids.is_empty()).then(|| ids[index % ids.len()]);
    let command = match step {
        Step::Create {
            x,
            y,
            item: is_item,
        } => Some(DebugCommand::CreateEntity {
            spec: if *is_item { item() } else { creature() },
            place: on(*x, *y),
        }),
        Step::Move { who, x, y } => pick(*who).map(|entity| DebugCommand::MoveEntity {
            entity,
            to: on(*x, *y),
        }),
        Step::Put { who, into } => {
            pick(*who)
                .zip(pick(*into))
                .map(|(entity, container)| DebugCommand::MoveEntity {
                    entity,
                    to: Place::In(container),
                })
        }
        Step::Remove { who } => pick(*who).map(|entity| DebugCommand::RemoveEntity { entity }),
        Step::Wall { x, y, up } => Some(DebugCommand::SetElement {
            loc: at(*x, *y),
            layer: Layer::Block,
            state: if *up {
                common::wall(sim).raw()
            } else {
                BlockState::NONE.raw()
            },
        }),
        Step::Walk {
            who,
            after_ms,
            hops,
        } => pick(*who).map(|entity| DebugCommand::Schedule {
            entity: Some(entity),
            after: Dur::from_millis(*after_ms),
            wake: walk_wake(entity, *hops),
        }),
        Step::Until { ms } => {
            let to = sim.now() + Dur::from_millis(*ms);
            assert_eq!(sim.advance(AdvanceLimit::Until(to)), Advance::Reached(to));
            None
        }
        Step::Work { wakes } => {
            let before = sim.steps();
            let outcome = sim.advance(AdvanceLimit::Work(*wakes));
            assert!(sim.steps() - before <= u64::from(*wakes));
            assert!(matches!(outcome, Advance::Yielded | Advance::Reached(_)));
            None
        }
    };
    if let Some(command) = command {
        // A refusal is part of the session: it must leave no trace.
        let _ = debug(sim, command);
    }
}

/// A simulation with a few walkers already under way.
fn busy_sim(seed: u64) -> Sim {
    let mut sim = common::walking_sim(seed);
    for (x, y) in [(5, 5), (30, 30), (50, 12)] {
        debug(
            &mut sim,
            DebugCommand::CreateEntity {
                spec: creature(),
                place: on(x, y),
            },
        )
        .unwrap();
    }
    for entity in sim.truth().entities().ids().collect::<Vec<_>>() {
        debug(
            &mut sim,
            DebugCommand::Schedule {
                entity: Some(entity),
                after: Dur::from_millis(100),
                wake: walk_wake(entity, 8),
            },
        )
        .unwrap();
    }
    sim
}

/// Runs to a tick comfortably after everything a session scheduled.
fn finish(sim: &mut Sim, end: Tick) -> clash_core::Hash128 {
    assert_eq!(sim.advance(AdvanceLimit::Until(end)), Advance::Reached(end));
    sim.state_hash()
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(40))]

    /// K2, invariant T6: an input log replayed on a fresh simulation
    /// from the same seed and definitions reproduces the state hash,
    /// and the same events.
    #[test]
    fn replaying_an_input_log_reproduces_the_hash(
        seed in 0..4_u64,
        steps in prop::collection::vec(step(), 1..50),
    ) {
        let mut original = busy_sim(seed);
        for step in &steps {
            play(&mut original, step);
        }
        let end = original.now() + Dur::from_secs(60);
        let log = original.input_log().clone();
        let hash = finish(&mut original, end);

        let mut replayed = common::walking_sim(seed);
        replayed.replay(&log).unwrap();
        prop_assert_eq!(replayed.input_log(), &log);
        prop_assert_eq!(finish(&mut replayed, end), hash);
        prop_assert_eq!(replayed.drain_events(), original.drain_events());

        // A different seed draws different walks: the hash is of the
        // state, not of the script. (Only where something walked.)
        if original.steps() > 6 {
            let mut other = common::walking_sim(seed + 100);
            if other.replay(&log).is_ok() {
                prop_assert_ne!(finish(&mut other, end), hash);
            }
        }
    }

    /// K2: `Work(n)` returns `Yielded` and resumes exactly. A run made
    /// of small budgets handles the same wake-ups in the same order as
    /// one long call.
    #[test]
    fn work_yields_and_resumes_exactly(
        seed in 0..4_u64,
        steps in prop::collection::vec(step(), 1..30),
        budget in 1..5_u32,
    ) {
        let mut whole = busy_sim(seed);
        for step in &steps {
            play(&mut whole, step);
        }
        let mut pieces = whole.clone();
        prop_assert_eq!(pieces.state_hash(), whole.state_hash());

        let start = whole.steps();
        let done = whole.advance(AdvanceLimit::UntilDecision);
        prop_assert!(matches!(done, Advance::Reached(_)));
        let mut yields = 0_u64;
        let reached = loop {
            let before = pieces.steps();
            match pieces.advance(AdvanceLimit::Work(budget)) {
                Advance::Yielded => {
                    prop_assert_eq!(pieces.steps() - before, u64::from(budget));
                    yields += 1;
                }
                other => break other,
            }
        };
        prop_assert_eq!(reached, done);
        prop_assert_eq!(pieces.steps(), whole.steps());
        // Every full budget but perhaps the last ended in a yield.
        prop_assert!(yields >= ((whole.steps() - start) / u64::from(budget)).saturating_sub(1));
        prop_assert_eq!(pieces.hashes(), whole.hashes());
        prop_assert_eq!(pieces.drain_events(), whole.drain_events());
    }

    /// 01 R7: saving and loading in the middle of a random session
    /// changes nothing. The run that was saved, loaded and reindexed
    /// ends in the same state, having reported the same events.
    #[test]
    fn save_and_load_in_the_middle_changes_nothing(
        seed in 0..4_u64,
        steps in prop::collection::vec(step(), 2..40),
        split in any::<prop::sample::Index>(),
    ) {
        let split = split.index(steps.len());
        let mut straight = busy_sim(seed);
        let mut broken = busy_sim(seed);
        for (index, step) in steps.iter().enumerate() {
            if index == split {
                let (mut loaded, unplaced) = common::save_and_load(&broken);
                prop_assert!(unplaced.is_empty());
                prop_assert_eq!(loaded.hashes(), broken.hashes());
                // Handlers are not state: they are registered again.
                loaded.set_wake_handler(WakeDomain::Stepper, common::walker);
                broken = loaded;
            }
            play(&mut straight, step);
            play(&mut broken, step);
        }
        let end = straight.now() + Dur::from_secs(60);
        prop_assert_eq!(finish(&mut broken, end), finish(&mut straight, end));
        prop_assert_eq!(broken.input_log(), straight.input_log());
        prop_assert_eq!(broken.drain_events(), straight.drain_events());
    }
}

/// A replay that cannot be carried out says where it diverged.
#[test]
fn a_replay_that_diverges_names_the_entry() {
    let mut original = busy_sim(1);
    // As after a save: the log starts afresh from here.
    assert_eq!(original.take_input_log().len(), 6);
    original.advance(AdvanceLimit::Until(Tick(2 * SECOND)));
    let walker = original.truth().entities().ids().next().unwrap();
    debug(&mut original, DebugCommand::RemoveEntity { entity: walker }).unwrap();
    let log = original.input_log().clone();
    assert!(log.entries()[0].step > 0);

    // Replayed on something other than that save, the log does not fit:
    // here nothing is walking, so the command's moment never comes.
    let mut other = common::walking_sim(1);
    let reason = other.replay(&log).unwrap_err();
    assert_eq!(reason.code, codes::REPLAY_DIVERGED);
    assert_eq!(reason.value("entry"), Some(&clash_core::Value::Int(0)));

    // Taking the log leaves an empty one behind.
    assert_eq!(original.take_input_log(), log);
    assert!(original.input_log().is_empty());
}

/// A handler for tests: asks the entity the wake-up names to choose.
fn ask(sim: &mut Sim, _id: WakeId, wake: Wake) {
    let entity = EntityId::from_bits(wake.id);
    let kind = DecisionKind::Free {
        why: FreeReason::PlanEmpty(None),
    };
    let _ = sim.request_decision(entity, kind, Cause::DEBUG);
}

fn ask_wake(entity: EntityId) -> Wake {
    Wake::new(WakeDomain::Action, entity.to_bits(), 0)
}

/// K2, 03 §5.2, invariant T8: decisions are returned one at a time in
/// entity id order, time does not move until each is answered, and a
/// request says how much of the entity's own time has passed.
#[test]
fn decisions_stop_time_and_come_in_entity_order() {
    let mut sim = common::walking_sim(5);
    sim.set_wake_handler(WakeDomain::Action, ask);
    let first = spawn(&mut sim, creature().with_control(Control::Direct), on(1, 1));
    let npc = spawn(&mut sim, creature(), on(2, 2));
    let second = spawn(&mut sim, creature(), on(3, 3));
    debug(
        &mut sim,
        DebugCommand::SetControl {
            entity: second,
            control: Control::Direct,
        },
    )
    .unwrap();
    // The second is on a clock of its own, twice as fast.
    sim.set_time_rate(second, Some(clash_core::Real::of(2.0)), Cause::DEBUG)
        .unwrap();
    for entity in [second, npc, first] {
        sim.schedule_on(
            clash_time::ClockId::UNIVERSAL,
            Dur::from_secs(4),
            ask_wake(entity),
        )
        .unwrap();
    }
    sim.schedule_for(npc, Dur::from_secs(6), walk_wake(npc, 0))
        .unwrap();
    sim.pump();
    sim.drain_events();

    // Both are asked at the same tick; the lower id is returned first.
    let Advance::NeedInput(request) = sim.advance(AdvanceLimit::UntilDecision) else {
        panic!("a decision was due");
    };
    assert_eq!(sim.now(), Tick(4 * SECOND));
    assert_eq!(request.entity, first);
    assert_eq!(request.elapsed, Dur::from_secs(4));
    assert!(matches!(
        request.kind,
        DecisionKind::Free {
            why: FreeReason::PlanEmpty(None)
        }
    ));
    // An entity the player does not control is never asked.
    assert_eq!(
        sim.decisions().entities().collect::<Vec<_>>(),
        [first, second]
    );

    // Time does not move, whatever the limit, until it is answered.
    let hash = sim.state_hash();
    for limit in [
        AdvanceLimit::Until(Tick(60 * SECOND)),
        AdvanceLimit::Work(10),
        AdvanceLimit::UntilDecision,
    ] {
        assert_eq!(sim.advance(limit), Advance::NeedInput(request.clone()));
    }
    assert_eq!((sim.now(), sim.state_hash()), (Tick(4 * SECOND), hash));

    sim.answer_decision(first).unwrap();
    let Advance::NeedInput(request) = sim.advance(AdvanceLimit::UntilDecision) else {
        panic!("the second decision was due");
    };
    assert_eq!((request.entity, sim.now()), (second, Tick(4 * SECOND)));
    // Four seconds on a clock twice as fast.
    assert_eq!(request.elapsed, Dur::from_secs(8));
    sim.answer_decision(second).unwrap();
    assert_eq!(
        sim.answer_decision(second).unwrap_err().code,
        codes::DECISION_NONE_PENDING
    );

    // With every decision answered, time moves again.
    assert_eq!(
        sim.advance(AdvanceLimit::UntilDecision),
        Advance::Reached(Tick(6 * SECOND))
    );
    let events = sim.drain_events();
    let asked: Vec<EntityId> = events
        .iter()
        .filter(|event| event.name() == "decision_requested")
        .filter_map(|event| common::entity_of(event, "self"))
        .collect();
    assert_eq!(
        asked,
        [second, first],
        "asked in the order the wake-ups ran"
    );

    // Elapsed time is counted from the last answer.
    sim.schedule_on(
        clash_time::ClockId::UNIVERSAL,
        Dur::from_secs(1),
        ask_wake(second),
    )
    .unwrap();
    let Advance::NeedInput(request) = sim.advance(AdvanceLimit::UntilDecision) else {
        panic!("a decision was due");
    };
    assert_eq!(request.elapsed, Dur::from_secs(6));

    // Taking control away withdraws the question.
    sim.set_control(second, Control::None, Cause::DEBUG)
        .unwrap();
    assert!(sim.decisions().is_empty());
    assert!(matches!(
        sim.advance(AdvanceLimit::UntilDecision),
        Advance::Reached(_)
    ));
}

/// A rule's request to the application is returned at the safe point
/// after the wake-up that raised it.
#[test]
fn an_application_request_is_returned_at_the_next_safe_point() {
    fn checkpoint(sim: &mut Sim, _id: WakeId, _wake: Wake) {
        sim.request_app(clash_sim::AppRequest::Checkpoint {
            name: "before the vault".to_owned(),
        });
    }
    let mut sim = common::sim(6);
    sim.set_wake_handler(WakeDomain::Rule, checkpoint);
    for after in [1, 2] {
        sim.schedule_on(
            clash_time::ClockId::UNIVERSAL,
            Dur::from_secs(after),
            Wake::new(WakeDomain::Rule, 0, 0),
        )
        .unwrap();
    }
    let outcome = sim.advance(AdvanceLimit::Until(Tick(10 * SECOND)));
    assert!(matches!(
        outcome,
        Advance::Request(clash_sim::AppRequest::Checkpoint { .. })
    ));
    assert_eq!(sim.now(), Tick(SECOND), "returned before the next wake-up");
    // The request is state until it is returned, and survives a save.
    let (loaded, _) = common::save_and_load(&sim);
    assert_eq!(loaded.state_hash(), sim.state_hash());
}
