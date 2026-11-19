//! Tests for package K3: event types and the `event` definition type,
//! the notification queue and its pump with world events passed on,
//! hooks with mutable fields and cancellation, delivery order, `consume`,
//! filter indexing and the cascade limit (05 §8; 03 §4.3 step 4).

mod common;

use std::sync::{Arc, Mutex};

use clash_core::{EntityId, Real, Tick, Value};
use clash_sim::{
    Advance, AdvanceLimit, Delivery, EventBody, EventPayload, EventType, FieldType, FieldValue,
    Handler, MAX_CASCADE_SETTING, MAX_HOOK_DEPTH, Reason, Sim, Subject, Where, codes,
};
use clash_world::{BlockState, Cause, CauseKind, FloorState};
use common::{at, creature, on, spawn};

const COMPLETED: &str = "core:ritual_completed";
const COMPLETING: &str = "core:ritual_completing";
const ECHO: &str = "core:echo";

/// What handlers were called, in order.
type Calls = Arc<Mutex<Vec<String>>>;

fn calls() -> Calls {
    Arc::new(Mutex::new(Vec::new()))
}

fn seen(calls: &Calls) -> Vec<String> {
    calls.lock().unwrap().clone()
}

/// A handler that writes its name down when it is called.
fn note(calls: &Calls, name: &str) -> Handler {
    let (calls, name) = (Arc::clone(calls), name.to_owned());
    Handler::new(move |_sim: &mut Sim, _delivery: &mut Delivery<'_>| {
        calls.lock().unwrap().push(name.clone());
    })
}

fn completed(caster: EntityId, x: i32, power: f64, school: &str) -> EventPayload {
    EventPayload::new(COMPLETED)
        .subject("caster", Subject::Entity(caster))
        .subject("here", Subject::Loc(at(x, 1)))
        .with("power", Real::of(power))
        .with("school", school)
}

/// K3: delivery order matches 05 §8.3. Global handlers first, then those
/// attached to each of the event's subjects in declared order; within
/// each group higher priority first, then lower order, then
/// registration.
#[test]
fn delivery_order_is_global_then_attached_by_subject() {
    let mut sim = common::sim(1);
    let caster = spawn(&mut sim, creature(), on(1, 1));
    let helper = spawn(&mut sim, creature(), on(2, 1));
    let bystander = spawn(&mut sim, creature(), on(9, 9));
    sim.pump();
    sim.drain_events();
    let log = calls();

    // Registered in an order that is not the order of delivery.
    sim.on_subject(
        Subject::Loc(at(1, 1)),
        COMPLETED,
        note(&log, "here").role("here"),
    )
    .unwrap();
    sim.on(COMPLETED, note(&log, "global low").order(5))
        .unwrap();
    sim.on_subject(Subject::Entity(caster), COMPLETED, note(&log, "caster"))
        .unwrap();
    sim.on(COMPLETED, note(&log, "global first of two").order(2))
        .unwrap();
    sim.on_subject(
        Subject::Entity(helper),
        COMPLETED,
        note(&log, "actor").role("actor"),
    )
    .unwrap();
    sim.on(COMPLETED, note(&log, "global high").priority(10).order(9))
        .unwrap();
    sim.on(COMPLETED, note(&log, "global second of two").order(2))
        .unwrap();
    sim.on_subject(
        Subject::Entity(caster),
        COMPLETED,
        note(&log, "caster high").priority(3),
    )
    .unwrap();
    // Never called: attached to someone who takes no part, attached to
    // the caster as a subject it is not, and listening for another event.
    sim.on_subject(
        Subject::Entity(bystander),
        COMPLETED,
        note(&log, "bystander"),
    )
    .unwrap();
    sim.on_subject(
        Subject::Entity(caster),
        COMPLETED,
        note(&log, "wrong role").role("here"),
    )
    .unwrap();
    sim.on(ECHO, note(&log, "other event")).unwrap();

    let cause = Cause::new(CauseKind::Eoc).by(helper);
    let raised = sim
        .raise(
            Where::Entity(caster),
            cause,
            completed(caster, 1, 3.0, "fire"),
        )
        .unwrap();
    assert!(!raised.is_cancelled());
    // A notification is delivered at the pump, not when it is raised.
    assert!(seen(&log).is_empty());
    assert_eq!(sim.queued_events().count(), 1);
    sim.pump();
    assert_eq!(
        seen(&log),
        [
            "global high",
            "global first of two",
            "global second of two",
            "global low",
            "caster high",
            "caster",
            "here",
            "actor"
        ]
    );
    // And then to presentation, with what every event carries.
    let events = sim.drain_events();
    let [event] = events.as_slice() else {
        panic!("one event expected, got {events:?}");
    };
    assert_eq!(event.name(), COMPLETED);
    assert_eq!(event.cause, cause);
    assert_eq!(event.subject("caster"), Some(Subject::Entity(caster)));
    assert_eq!(event.body.field("school"), Some(FieldValue::text("fire")));

    // A handler that is taken away is no longer called.
    let extra = sim.on(COMPLETED, note(&log, "extra")).unwrap();
    assert!(sim.remove_handler(extra));
    assert!(!sim.remove_handler(extra));
    log.lock().unwrap().clear();
    sim.raise(
        Where::Global,
        Cause::DEBUG,
        completed(bystander, 30, 1.0, "frost"),
    )
    .unwrap();
    sim.pump();
    assert_eq!(
        seen(&log),
        [
            "global high",
            "global first of two",
            "global second of two",
            "global low",
            "bystander"
        ]
    );
}

/// Notifications raised while handling an event join the back of the
/// queue (05 §8.3).
#[test]
fn events_raised_by_handlers_join_the_back_of_the_queue() {
    let mut sim = common::sim(2);
    let caster = spawn(&mut sim, creature(), on(1, 1));
    sim.pump();
    sim.drain_events();
    sim.on(
        COMPLETED,
        Handler::new(|sim: &mut Sim, delivery: &mut Delivery<'_>| {
            let depth = delivery.field("power").and_then(|power| power.as_real());
            let echo = EventPayload::new(ECHO).with("depth", depth.map_or(0, Real::to_i64));
            sim.raise(Where::Global, Cause::DEBUG, echo).unwrap();
        }),
    )
    .unwrap();
    for power in [1.0, 2.0] {
        sim.raise(
            Where::Global,
            Cause::DEBUG,
            completed(caster, 1, power, "fire"),
        )
        .unwrap();
    }
    sim.pump();
    let events = sim.drain_events();
    assert_eq!(common::names(&events), [COMPLETED, COMPLETED, ECHO, ECHO]);
    assert_eq!(events[2].body.field("depth"), Some(FieldValue::from(1)));
    assert_eq!(events[3].body.field("depth"), Some(FieldValue::from(2)));
}

/// What a test's "operation" does with a hook: asks, and goes ahead with
/// whatever the handlers left, or does not go ahead.
fn complete_ritual(sim: &mut Sim, caster: EntityId, power: f64) -> Result<Real, Reason> {
    let hook = EventPayload::new(COMPLETING)
        .subject("caster", Subject::Entity(caster))
        .with("power", Real::of(power));
    let raised = sim.raise(Where::Entity(caster), Cause::DEBUG.by(caster), hook)?;
    if let Some(reason) = raised.cancelled {
        return Err(reason);
    }
    let power = raised.field("power").and_then(FieldValue::as_real).unwrap();
    let entity = sim.entity_mut(caster).unwrap();
    entity.vars.set("core:charge", power);
    Ok(power)
}

/// K3: a hook handler changes a field and the operation uses the changed
/// value; a cancelled hook prevents it.
#[test]
fn a_hook_can_change_a_field_or_cancel_the_operation() {
    let mut sim = common::sim(3);
    let caster = spawn(&mut sim, creature(), on(1, 1));
    let warded = spawn(&mut sim, creature(), on(2, 1));
    sim.pump();
    sim.drain_events();
    let charge = |sim: &Sim, who| {
        sim.truth()
            .entity(who)
            .unwrap()
            .vars
            .get("core:charge")
            .cloned()
    };

    // With nobody listening the operation goes ahead as asked.
    assert_eq!(complete_ritual(&mut sim, caster, 2.0), Ok(Real::of(2.0)));

    // Armour that halves a hit: a handler changes the mutable field.
    let halve = sim
        .on(
            COMPLETING,
            Handler::new(|_sim: &mut Sim, delivery: &mut Delivery<'_>| {
                assert!(delivery.is_hook());
                let power = delivery
                    .field("power")
                    .and_then(|power| power.as_real())
                    .unwrap();
                delivery.set_field("power", power * Real::of(0.5)).unwrap();
            }),
        )
        .unwrap();
    // A second handler sees what the first left.
    sim.on(
        COMPLETING,
        Handler::new(|_sim: &mut Sim, delivery: &mut Delivery<'_>| {
            let power = delivery
                .field("power")
                .and_then(|power| power.as_real())
                .unwrap();
            delivery.set_field("power", power + Real::ONE).unwrap();
        })
        .order(1),
    )
    .unwrap();
    // A hook is delivered at once: nothing waits for the pump.
    assert_eq!(complete_ritual(&mut sim, caster, 8.0), Ok(Real::of(5.0)));
    assert_eq!(charge(&sim, caster), Some(Value::Real(Real::of(5.0))));
    assert_eq!(
        sim.queued_events().count(),
        0,
        "a hook is not a notification"
    );

    // A ward that turns it aside: a handler attached to one entity
    // cancels, with a reason, and no later handler runs.
    let log = calls();
    sim.on_subject(
        Subject::Entity(warded),
        COMPLETING,
        Handler::new(|_sim: &mut Sim, delivery: &mut Delivery<'_>| {
            assert_eq!(delivery.owner(), delivery.event().subject("caster"));
            delivery
                .cancel(Reason::new("test.warded", "the circle is warded").with("ward", 3))
                .unwrap();
        })
        .priority(5),
    )
    .unwrap();
    sim.on_subject(
        Subject::Entity(warded),
        COMPLETING,
        note(&log, "after the ward"),
    )
    .unwrap();
    let refused = complete_ritual(&mut sim, warded, 8.0).unwrap_err();
    assert_eq!(refused.code, "test.warded");
    assert_eq!(refused.to_string(), "the circle is warded");
    assert_eq!(charge(&sim, warded), None, "the operation did not happen");
    assert!(seen(&log).is_empty());
    // The other caster is not warded.
    assert_eq!(complete_ritual(&mut sim, caster, 4.0), Ok(Real::of(3.0)));

    // The cancellation is reported.
    sim.pump();
    let events = sim.drain_events();
    let [cancelled] = events.as_slice() else {
        panic!("one event expected, got {events:?}");
    };
    assert_eq!(
        cancelled.body,
        EventBody::EventCancelled {
            event: COMPLETING.to_owned(),
            reason: refused
        }
    );

    // What a handler may not do is refused with a reason and reported:
    // changing a field that is not mutable, cancelling a notification.
    sim.remove_handler(halve);
    sim.on(
        COMPLETED,
        Handler::new(|_sim: &mut Sim, delivery: &mut Delivery<'_>| {
            let fixed = delivery.set_field("power", Real::ZERO).unwrap_err();
            assert_eq!(fixed.code, codes::EVENTS_FIELD_FIXED);
            assert!(delivery.cancel(Reason::new("x", "y")).is_err());
        }),
    )
    .unwrap();
    sim.raise(
        Where::Global,
        Cause::DEBUG,
        completed(caster, 1, 9.0, "fire"),
    )
    .unwrap();
    sim.pump();
    let events = sim.drain_events();
    assert_eq!(common::names(&events), ["fault", "fault", COMPLETED]);
    assert_eq!(
        events[2].body.field("power"),
        Some(FieldValue::from(Real::of(9.0)))
    );
}

/// A handler may consume an event; later handlers marked
/// `skip_if_consumed` are then skipped, and the others still run.
#[test]
fn a_consumed_event_skips_the_handlers_that_ask_to_be_skipped() {
    let mut sim = common::sim(4);
    let caster = spawn(&mut sim, creature(), on(1, 1));
    let log = calls();
    sim.on(
        COMPLETED,
        note(&log, "before").priority(9).skip_if_consumed(),
    )
    .unwrap();
    let consumer = Arc::clone(&log);
    sim.on(
        COMPLETED,
        Handler::new(move |_sim: &mut Sim, delivery: &mut Delivery<'_>| {
            assert!(!delivery.is_consumed());
            delivery.consume();
            consumer.lock().unwrap().push("consumer".to_owned());
        })
        .priority(5),
    )
    .unwrap();
    sim.on(COMPLETED, note(&log, "skipped").skip_if_consumed())
        .unwrap();
    sim.on(COMPLETED, note(&log, "stays").order(1)).unwrap();
    sim.on_subject(
        Subject::Entity(caster),
        COMPLETED,
        note(&log, "attached, skipped").skip_if_consumed(),
    )
    .unwrap();
    sim.raise(
        Where::Global,
        Cause::DEBUG,
        completed(caster, 1, 1.0, "fire"),
    )
    .unwrap();
    sim.pump();
    assert_eq!(seen(&log), ["before", "consumer", "stays"]);
}

/// K3, 05 §8.4: a filter is a test on an event's discrete fields, and
/// handlers are indexed by it: an event looks only at the handlers whose
/// filter it could pass.
#[test]
fn filters_select_handlers_and_index_them() {
    let mut sim = common::sim(5);
    let caster = spawn(&mut sim, creature(), on(1, 1));
    sim.pump();
    let log = calls();
    for school in 0..200 {
        sim.on(
            COMPLETED,
            note(&log, &format!("school {school}")).filter("school", format!("s{school}").as_str()),
        )
        .unwrap();
    }
    // Two entries: the first indexes, the second is still tested.
    sim.on(
        COMPLETED,
        note(&log, "strong s7")
            .filter("school", "s7")
            .filter("power", Real::of(9.0)),
    )
    .unwrap();
    sim.on(COMPLETED, note(&log, "any").order(1)).unwrap();

    let before = sim.handlers_looked_at();
    sim.raise(Where::Global, Cause::DEBUG, completed(caster, 1, 2.0, "s7"))
        .unwrap();
    sim.pump();
    assert_eq!(seen(&log), ["school 7", "any"]);
    // Of 202 handlers, three were looked at: the two indexed under `s7`
    // and the one with no filter.
    assert_eq!(sim.handlers_looked_at() - before, 3);

    log.lock().unwrap().clear();
    sim.raise(Where::Global, Cause::DEBUG, completed(caster, 1, 9.0, "s7"))
        .unwrap();
    sim.raise(
        Where::Global,
        Cause::DEBUG,
        completed(caster, 1, 9.0, "none of them"),
    )
    .unwrap();
    sim.pump();
    assert_eq!(seen(&log), ["school 7", "strong s7", "any", "any"]);
}

/// K3: world events are forwarded into the queue and reach handlers,
/// filtered on their discrete fields.
#[test]
fn world_events_reach_handlers() {
    let mut sim = common::sim(6);
    let log = calls();
    sim.on(
        "element_changed",
        note(&log, "block").filter("layer", "block"),
    )
    .unwrap();
    sim.on("element_changed", note(&log, "any layer").order(1))
        .unwrap();
    let entered = Arc::clone(&log);
    sim.on(
        "area_entered",
        Handler::new(move |sim: &mut Sim, delivery: &mut Delivery<'_>| {
            let who = delivery
                .event()
                .subject("target")
                .and_then(|subject| subject.entity());
            // By the time a handler runs, the kernel is in step with
            // what the world reported.
            let place = who.and_then(|who| sim.truth().place(who));
            entered
                .lock()
                .unwrap()
                .push(format!("entered at {place:?}"));
        }),
    )
    .unwrap();

    let wall = common::wall(&sim);
    sim.world(|ctx| {
        let mut edit = ctx.edit(Cause::DEBUG);
        edit.set_block(at(4, 4), wall).unwrap();
        edit.set_floor(at(5, 4), FloorState::NONE).unwrap();
    });
    sim.pump();
    assert_eq!(seen(&log), ["block", "any layer", "any layer"]);

    log.lock().unwrap().clear();
    common::box_area(&mut sim, (10, 10), (12, 12), None, 0);
    let walker = spawn(&mut sim, creature(), on(1, 1));
    sim.relocate(walker, on(11, 11), Cause::DEBUG).unwrap();
    sim.pump();
    assert_eq!(seen(&log), [format!("entered at {:?}", Some(on(11, 11)))]);
    assert_eq!(
        common::names(&sim.drain_events()),
        [
            "element_changed",
            "element_changed",
            "entity_created",
            "entity_relocated",
            "area_entered",
            "entity_relocated"
        ]
    );
    // An edit that changes nothing reports nothing.
    sim.world(|ctx| {
        ctx.edit(Cause::DEBUG)
            .set_block(at(30, 30), BlockState::NONE)
    })
    .unwrap();
    sim.pump();
    assert!(sim.drain_events().is_empty());
}

/// K3, scenario `rule_cascade_limit`: a self-raising event is cut off,
/// reported with a trace, and the simulation continues. (Kept as a test
/// until the scenario runner exists to hold it under `scenarios/`.)
#[test]
fn rule_cascade_limit() {
    let mut sim = common::walking_sim(7);
    sim.set_setting(MAX_CASCADE_SETTING, 50);
    let runs = calls();
    let counter = Arc::clone(&runs);
    // A rule that answers every echo with another.
    sim.on(
        ECHO,
        Handler::new(move |sim: &mut Sim, delivery: &mut Delivery<'_>| {
            counter.lock().unwrap().push(String::new());
            let depth = match delivery.field("depth") {
                Some(FieldValue::Plain(Value::Int(depth))) => depth,
                other => panic!("an echo has a depth, not {other:?}"),
            };
            sim.raise(
                Where::Global,
                Cause::new(CauseKind::Eoc),
                EventPayload::new(ECHO).with("depth", depth + 1),
            )
            .unwrap();
        }),
    )
    .unwrap();

    sim.raise(
        Where::Global,
        Cause::DEBUG,
        EventPayload::new(ECHO).with("depth", 0),
    )
    .unwrap();
    // The pump returns: content cannot hang the engine (T7).
    sim.pump();
    assert_eq!(seen(&runs).len(), 50);
    assert_eq!(sim.queued_events().count(), 0, "the remainder was dropped");
    let events = sim.drain_events();
    assert_eq!(events.len(), 51);
    assert!(events[..50].iter().all(|event| event.name() == ECHO));
    let EventBody::Fault { reason } = &events[50].body else {
        panic!("the cascade is reported, not {:?}", events[50]);
    };
    assert_eq!(reason.code, codes::EVENTS_CASCADE_LIMIT);
    assert_eq!(reason.value("limit"), Some(&Value::Int(50)));
    assert_eq!(reason.value("dropped"), Some(&Value::Int(1)));
    let Some(Value::Str(trace)) = reason.value("trace") else {
        panic!("the report carries a trace");
    };
    assert!(trace.starts_with(ECHO) && trace.contains(" > "), "{trace}");

    // The simulation carries on: time moves, wake-ups run, and the next
    // pump starts a fresh count.
    let walker = spawn(&mut sim, creature(), on(5, 5));
    sim.schedule_for(
        walker,
        clash_core::Dur::from_secs(1),
        common::walk_wake(walker, 3),
    )
    .unwrap();
    assert!(matches!(
        sim.advance(AdvanceLimit::Until(Tick(30_000_000))),
        Advance::Reached(Tick(30_000_000))
    ));
    assert_eq!(sim.steps(), 4);
    sim.drain_events();
    sim.raise(
        Where::Global,
        Cause::DEBUG,
        EventPayload::new(ECHO).with("depth", 0),
    )
    .unwrap();
    sim.pump();
    assert_eq!(seen(&runs).len(), 100);
    // The same from the same start: the cut-off is deterministic.
    assert_eq!(sim.drain_events().len(), 51);
}

/// A hook raised from inside too many others is skipped and reported
/// (05 §13.2), so that hooks cannot recurse without end.
#[test]
fn hooks_nest_only_so_deep() {
    let mut sim = common::sim(8);
    let caster = spawn(&mut sim, creature(), on(1, 1));
    sim.pump();
    sim.drain_events();
    let runs = calls();
    let counter = Arc::clone(&runs);
    sim.on(
        COMPLETING,
        Handler::new(move |sim: &mut Sim, delivery: &mut Delivery<'_>| {
            counter.lock().unwrap().push(String::new());
            let Some(Subject::Entity(caster)) = delivery.event().subject("caster") else {
                return;
            };
            let _ = complete_ritual(sim, caster, 1.0);
        }),
    )
    .unwrap();
    assert!(complete_ritual(&mut sim, caster, 1.0).is_ok());
    assert_eq!(seen(&runs).len(), usize::from(MAX_HOOK_DEPTH));
    sim.pump();
    let events = sim.drain_events();
    let [fault] = events.as_slice() else {
        panic!("one fault expected, got {events:?}");
    };
    assert!(matches!(
        &fault.body,
        EventBody::Fault { reason } if reason.code == codes::EVENTS_HOOK_DEPTH
    ));
}

/// Event types are registered with their subjects and fields, and what
/// is raised is checked against them. Every refusal carries a reason.
#[test]
fn event_types_are_registered_and_payloads_checked() {
    let mut sim = common::sim(9);
    let caster = spawn(&mut sim, creature(), on(1, 1));
    sim.pump();
    sim.drain_events();

    // The `event` definitions of the loaded mods.
    let ty = sim.event_type(COMPLETING).unwrap();
    assert!(ty.hook);
    assert_eq!(ty.mutable, ["power"]);
    assert_eq!(ty.subjects, [("caster".to_owned(), "entity".to_owned())]);
    assert_eq!(ty.fields[0].ty, "real");
    let ty = sim.event_type(COMPLETED).unwrap();
    assert_eq!(ty.doc, "A ritual circle finished charging.");
    assert_eq!(
        ty.subjects
            .iter()
            .map(|(name, _)| name.as_str())
            .collect::<Vec<_>>(),
        ["caster", "here"],
        "subjects keep the order they were written in"
    );
    // The engine's own, each with documentation, subjects and fields.
    for name in [
        "entity_relocated",
        "element_changed",
        "area_entered",
        "fault",
    ] {
        let ty = sim
            .event_type(name)
            .unwrap_or_else(|| panic!("`{name}` is not registered"));
        assert!(!ty.doc.is_empty(), "`{name}` has no documentation");
    }
    let names: Vec<&str> = sim.event_types().map(|ty| ty.name.as_str()).collect();
    assert!(names.is_sorted(), "the reference lists them in order");

    // A package registers its own, here a hook.
    let parry = EventType::new("parry_incoming")
        .doc("A blow is about to be parried. A handler may change `force` or cancel.")
        .subject("self", "entity")
        .field(
            "force",
            FieldType::Real,
            "The force of the blow, in newtons.",
        )
        .hook(["force"]);
    sim.register_event(parry.clone()).unwrap();
    assert_eq!(
        sim.register_event(parry).unwrap_err().code,
        codes::EVENTS_BAD_PAYLOAD
    );
    let bad_kind = EventType::new("x").subject("self", "wizard");
    assert_eq!(
        sim.register_event(bad_kind).unwrap_err().code,
        codes::EVENTS_BAD_PAYLOAD
    );
    let bad_mutable = EventType::new("y").hook(["nothing"]);
    assert_eq!(
        sim.register_event(bad_mutable).unwrap_err().code,
        codes::EVENTS_BAD_PAYLOAD
    );

    // Raising: each mistake is refused with a reason and changes nothing.
    let hash = sim.state_hash();
    let good = completed(caster, 1, 1.0, "fire");
    let refuse = |sim: &mut Sim, payload: EventPayload| {
        sim.raise(Where::Global, Cause::DEBUG, payload).unwrap_err()
    };
    let unknown = refuse(&mut sim, EventPayload::new("core:no_such_event"));
    assert_eq!(unknown.code, codes::EVENTS_UNKNOWN_TYPE);
    let mut missing = good.clone();
    missing.subjects.pop();
    assert_eq!(refuse(&mut sim, missing).code, codes::EVENTS_BAD_PAYLOAD);
    let wrong_kind = EventPayload::new(COMPLETED)
        .subject("caster", Subject::Loc(at(1, 1)))
        .subject("here", Subject::Loc(at(1, 1)));
    let reason = refuse(&mut sim, wrong_kind);
    assert_eq!(reason.code, codes::EVENTS_BAD_PAYLOAD);
    assert_eq!(reason.value("name"), Some(&Value::from("caster")));
    assert_eq!(
        refuse(&mut sim, good.clone().with("colour", "red")).code,
        codes::EVENTS_BAD_PAYLOAD
    );
    let mut wrong_type = good.clone();
    wrong_type.fields[0].1 = FieldValue::text("a lot");
    assert_eq!(refuse(&mut sim, wrong_type).code, codes::EVENTS_BAD_PAYLOAD);
    // The engine's own events are raised only by the engine.
    assert_eq!(
        refuse(&mut sim, EventPayload::new("entity_removed")).code,
        codes::EVENTS_BAD_PAYLOAD
    );
    assert_eq!(
        sim.on("core:no_such_event", Handler::new(|_, _| {}))
            .unwrap_err()
            .code,
        codes::EVENTS_UNKNOWN_TYPE
    );
    assert_eq!(sim.state_hash(), hash);
    assert_eq!(sim.queued_events().count(), 0);

    // A declared field left out is null; an integer serves as a real.
    let sparse = EventPayload::new(COMPLETED)
        .subject("here", Subject::Loc(at(1, 1)))
        .subject("caster", Subject::Entity(caster))
        .with("power", 4);
    let raised = sim.raise(Where::Global, Cause::DEBUG, sparse).unwrap();
    assert_eq!(raised.field("school"), Some(&FieldValue::Null));
    assert_eq!(
        raised.payload.subjects[0].0, "caster",
        "put in declared order"
    );
}

/// A mistake in an `event` definition is a located diagnostic at load.
#[test]
fn a_bad_event_definition_is_a_load_error() {
    let cases = [
        (
            r#"[{ "type": "event", "id": "a", "subjects": { "who": "wizard" } }]"#,
            "unknown kind `wizard`",
        ),
        (
            r#"[{ "type": "event", "id": "b", "fields": { "power": "tensor" } }]"#,
            "unknown type `tensor`",
        ),
        (
            r#"[{ "type": "event", "id": "c", "fields": { "power": "real" }, "mutable": ["power"] }]"#,
            "only a hook has mutable fields",
        ),
        (
            r#"[{ "type": "event", "id": "d", "hook": true, "mutable": ["power"] }]"#,
            "not one of the event's fields",
        ),
        (
            r#"[{ "type": "event", "id": "e", "subjects": { "x": "entity" }, "fields": { "x": "int" } }]"#,
            "both a subject and a field",
        ),
        (
            r#"[{ "type": "event", "id": "f", "subject": { "x": "entity" } }]"#,
            "subject",
        ),
    ];
    for (text, expected) in cases {
        let error = common::try_defs(text)
            .err()
            .unwrap_or_else(|| panic!("{text} loaded"));
        assert!(error.contains(expected), "{text}:\n{error}");
        assert!(error.contains("data/test/extra.json"), "{error}");
    }
    assert!(common::try_defs(r#"[{ "type": "event", "id": "plain" }]"#).is_ok());
}

/// Events waiting to be delivered are state: they survive a save, and a
/// loaded simulation with the same handlers delivers them the same way.
#[test]
fn queued_events_survive_a_save() {
    let mut sim = common::sim(10);
    let caster = spawn(&mut sim, creature(), on(1, 1));
    sim.raise(
        Where::Entity(caster),
        Cause::DEBUG,
        completed(caster, 1, 1.0, "fire"),
    )
    .unwrap();
    assert_eq!(sim.queued_events().count(), 3);
    let (mut loaded, unplaced) = common::save_and_load(&sim);
    assert!(unplaced.is_empty());
    assert_eq!(loaded.state_hash(), sim.state_hash());
    for target in [&mut sim, &mut loaded] {
        target.pump();
    }
    assert_eq!(loaded.state_hash(), sim.state_hash());
    assert_eq!(loaded.drain_events(), sim.drain_events());
}
