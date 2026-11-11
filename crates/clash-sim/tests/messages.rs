//! Messages and choices (plans/sim-kernel.md, package K9; 05 §10.6,
//! 03 §5.2): the `message` and `ask` effects against a real simulation,
//! from the fixture mod `talk` (`tests/fixtures/act/mods/talk`).
//!
//! What is shown: a line reads as its reader would put it (the review
//! addition "Text for a reader", test script `who_did_it`); a choice
//! put to a directly controlled entity stops time until `Choose` answers
//! it, across a save and a replay, and the option's effects run with the
//! subjects, locals and event of where the `ask` is written; a choice
//! put to anyone else takes the first option that may be taken.

mod common;

use clash_core::{Dur, EntityId, Tick};
use clash_expr::Value;
use clash_sim::component::Identity;
use clash_sim::reader::Known;
use clash_sim::{
    Advance, AdvanceLimit, ChoiceId, Command, Control, DebugCommand, DecisionKind, EntitySpec,
    EventBody, Sim, SimEvent, Subject, codes,
};
use clash_time::{Wake, WakeDomain, WakeId};
use clash_world::Cause;
use common::{act_fixtures, creature, debug, defs_with, on, save_and_load, sim_over};

const SECOND: i64 = 1_000_000;

fn sim() -> Sim {
    let mut sim = sim_over(defs_with(&act_fixtures(), &["talk"]), 11);
    sim.set_wake_handler(WakeDomain::Stepper, tag_wake);
    sim
}

/// A wake-up for tests: gives the entity it names the tag its argument
/// numbers.
fn tag_wake(sim: &mut Sim, _id: WakeId, wake: Wake) {
    let tag = ["approached", "cornered"][usize::try_from(wake.arg).unwrap_or(0) % 2];
    let entity = EntityId::from_bits(wake.id);
    let _ = sim.set_tag(entity, tag, true, Cause::DEBUG.by(entity));
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

/// Has a tag given to `who` after `secs` seconds, by command.
fn tag_later(sim: &mut Sim, who: EntityId, secs: i64, tag: u64) {
    debug(
        sim,
        DebugCommand::Schedule {
            entity: Some(who),
            after: Dur::from_secs(secs),
            wake: Wake::new(WakeDomain::Stepper, who.to_bits(), tag),
        },
    )
    .unwrap();
}

fn var(sim: &Sim, who: EntityId, name: &str) -> Value {
    sim.var(Subject::Entity(who), name)
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

/// The lines of the `message` events, with who each is for and how it
/// is to be shown.
fn lines(events: &[SimEvent]) -> Vec<(EntityId, String, String)> {
    events
        .iter()
        .filter_map(|event| match &event.body {
            EventBody::Act(clash_sim::action::ActEvent::Message { to, text, style }) => {
                Some((*to, text.clone(), style.clone()))
            }
            _ => None,
        })
        .collect()
}

/// The choices made, in order: who, which option, its label.
fn made(events: &[SimEvent]) -> Vec<(EntityId, u32, String)> {
    events
        .iter()
        .filter_map(|event| match &event.body {
            EventBody::Act(clash_sim::action::ActEvent::ChoiceMade {
                who, option, label, ..
            }) => Some((*who, *option, label.clone())),
            _ => None,
        })
        .collect()
}

fn named(sim: &mut Sim, who: EntityId, name: &str) {
    let mut identity = Identity::new(None);
    identity.rename(Some(name.to_owned()));
    sim.components_mut().insert(who, identity);
}

/// What a reader knows of others in these tests: someone tagged
/// `acquainted` knows everyone by name, and anyone else sees a
/// stranger.
fn fixture_lens(sim: &Sim, reader: EntityId, of: EntityId) -> Option<Known> {
    let knows = sim
        .truth()
        .entity(reader)
        .is_some_and(|entity| entity.tags.contains("acquainted"));
    if knows {
        clash_sim::reader::plain_lens(sim, reader, of)
    } else {
        Some(Known::Common("tall figure in red".to_owned()))
    }
}

fn call(sim: &mut Sim, rule: &str, who: EntityId, args: &[(&str, Value)]) {
    let id = sim.rule_set().lookup(rule).unwrap();
    let me = sim.subject_value(Subject::Entity(who));
    sim.call_rule(id, &[("self", me)], args, Cause::DEBUG)
        .unwrap();
    sim.pump();
}

/// The review addition "Text for a reader", test script `who_did_it`:
/// one line reads "You…" to the one who did it, names them to a reader
/// who knows them, and describes them to one who does not.
#[test]
fn who_did_it() {
    let mut sim = sim();
    sim.set_reader_lens(fixture_lens);
    let ringer = make(&mut sim, creature(), 1, 1);
    let friend = make(&mut sim, creature().with_tag("acquainted"), 2, 2);
    let stranger = make(&mut sim, creature(), 3, 3);
    named(&mut sim, ringer, "Mara");
    sim.drain_events();

    let readers = Value::list(
        [ringer, friend, stranger]
            .map(|id| sim.subject_value(Subject::Entity(id)))
            .to_vec(),
    );
    call(&mut sim, "talk:ring", ringer, &[("readers", readers)]);
    let events = sim.drain_events();
    assert_eq!(faults(&events), Vec::<String>::new());
    let sound = "sound".to_owned();
    assert_eq!(
        lines(&events),
        [
            (ringer, "You ring the bell.".to_owned(), sound.clone()),
            (friend, "Mara rings the bell.".to_owned(), sound.clone()),
            (
                stranger,
                "The tall figure in red rings the bell.".to_owned(),
                sound
            ),
        ]
    );
    // Each line is an event about its reader, caused by the ringer.
    let first = events
        .iter()
        .find(|event| event.name() == "message")
        .unwrap();
    assert_eq!(common::entity_of(first, "self"), Some(ringer));
    assert_eq!(first.cause.actor, Some(ringer));

    // A message with its text alone is for `self`, and a finished line
    // starts with a capital.
    call(&mut sim, "talk:note", friend, &[]);
    assert_eq!(
        lines(&sim.drain_events()),
        [(friend, "You are ready.".to_owned(), String::new())]
    );
}

/// The helpers in plain forms: with no reader, nobody is "you" and
/// everyone goes by their own name.
#[test]
fn text_without_a_reader_names_everyone_plainly() {
    let mut sim = sim();
    let who = make(&mut sim, creature(), 1, 1);
    named(&mut sim, who, "Mara");
    let me = sim.subject_value(Subject::Entity(who));
    let defs = std::sync::Arc::clone(sim.defs());
    let registry = defs.expr_registry();
    let helper = |name: &str| registry.function_id(name).unwrap();
    let cases = [
        ("the", vec![me.clone()], "Mara"),
        ("name", vec![me.clone()], "Mara"),
        ("they", vec![me.clone()], "they"),
        ("verb", vec![me.clone(), Value::from("carry")], "carries"),
        ("count", vec![Value::Int(3), Value::from("box")], "3 boxes"),
        ("count", vec![Value::Int(1), Value::from("box")], "1 box"),
        ("plural", vec![Value::from("knife")], "knives"),
        (
            "list",
            vec![Value::list([me, Value::from("rope")])],
            "Mara and a rope",
        ),
    ];
    for (name, args, wanted) in cases {
        assert_eq!(
            sim.call_function(helper(name), &args),
            Value::from(wanted),
            "{name}"
        );
    }
}

/// The first half of the choice script: an entity the player controls
/// is approached a second in, and asked.
fn asked(sim: &mut Sim) -> (EntityId, ChoiceId) {
    let who = make(sim, creature().with_control(Control::Direct), 4, 4);
    answer_free(sim, who);
    tag_later(sim, who, 1, 0);
    let Advance::NeedInput(request) = sim.advance(AdvanceLimit::Until(Tick(10 * SECOND))) else {
        panic!("a choice was due");
    };
    assert_eq!((request.entity, sim.now()), (who, Tick(SECOND)));
    let DecisionKind::Choice {
        choice,
        prompt,
        options,
    } = request.kind
    else {
        panic!("not a choice: {:?}", request.kind);
    };
    assert_eq!(prompt, "Pay 5 coins?");
    let shown: Vec<(&str, bool)> = options
        .iter()
        .map(|option| (option.label.as_str(), option.available))
        .collect();
    assert_eq!(shown, [("Haggle", false), ("Pay 5", true), ("Leave", true)]);
    (who, choice)
}

/// Answers whatever free decision an entity is waiting for.
fn answer_free(sim: &mut Sim, who: EntityId) {
    while matches!(
        sim.decisions().front_of(who),
        Some(DecisionKind::Free { .. })
    ) {
        sim.answer_decision(who).unwrap();
    }
}

/// Package K9: a choice offered to a directly controlled entity stops
/// time until answered; the chosen option's effects run with the
/// original subjects, locals and event, after a save too; replaying the
/// input log gives the same state.
#[test]
fn a_choice_stops_time_until_answered() {
    let mut sim = sim();
    let (who, choice) = asked(&mut sim);

    // Time does not move, whatever the limit, until it is answered.
    let hash = sim.state_hash();
    for limit in [
        AdvanceLimit::Until(Tick(10 * SECOND)),
        AdvanceLimit::Work(10),
        AdvanceLimit::UntilDecision,
    ] {
        assert!(matches!(sim.advance(limit), Advance::NeedInput(_)));
    }
    assert_eq!((sim.now(), sim.state_hash()), (Tick(SECOND), hash));

    // What cannot be chosen is refused, and changes nothing.
    let refusals = [
        (who, choice, 0, "rule.choice_refused"),
        (who, choice, 7, "rule.choice_refused"),
        (who, ChoiceId::new(99), 1, codes::DECISION_NONE_PENDING),
    ];
    for (entity, choice, option, code) in refusals {
        let error = sim
            .submit(Command::Choose {
                entity,
                choice,
                option,
            })
            .unwrap_err();
        assert_eq!(error.reason.code, code);
        assert_eq!(sim.state_hash(), hash);
    }

    // A save while the choice waits: the loaded simulation answers it
    // alike.
    let (mut loaded, unplaced) = save_and_load(&sim);
    assert!(unplaced.is_empty());
    loaded.set_wake_handler(WakeDomain::Stepper, tag_wake);
    assert_eq!(loaded.state_hash(), hash);
    sim.drain_events();
    for sim in [&mut sim, &mut loaded] {
        sim.submit(Command::Choose {
            entity: who,
            choice,
            option: 1,
        })
        .unwrap();
        // The option ran where the `ask` is written: `$price` and
        // `event.tag` are what they were.
        assert_eq!(var(sim, who, "talk:paid"), Value::Int(5));
        assert_eq!(var(sim, who, "talk:said"), Value::from("approached"));
        assert!(sim.decisions().is_empty());
        let events = sim.drain_events();
        assert_eq!(faults(&events), Vec::<String>::new());
        assert_eq!(made(&events), [(who, 1, "Pay 5".to_owned())]);
    }
    assert_eq!(loaded.state_hash(), sim.state_hash());
    let answered = sim.state_hash();

    // The input log replays to the state the choice left.
    let mut replayed = self::sim();
    replayed.replay(sim.input_log()).unwrap();
    assert_eq!(replayed.state_hash(), answered);

    // Time moves again.
    assert_eq!(
        sim.advance(AdvanceLimit::Until(Tick(10 * SECOND))),
        Advance::Reached(Tick(10 * SECOND))
    );
}

/// Package K9: a choice offered to anyone not under direct control
/// takes the first option that may be taken, at once.
#[test]
fn a_choice_offered_to_another_takes_the_first_available_option() {
    let mut sim = sim();
    let who = make(&mut sim, creature(), 4, 4);
    sim.set_tag(who, "approached", true, Cause::DEBUG).unwrap();
    sim.pump();
    assert_eq!(var(&sim, who, "talk:paid"), Value::Int(5));
    assert_eq!(var(&sim, who, "talk:said"), Value::from("approached"));
    assert!(sim.decisions().is_empty());
    let events = sim.drain_events();
    assert_eq!(faults(&events), Vec::<String>::new());
    assert_eq!(made(&events), [(who, 1, "Pay 5".to_owned())]);
}

/// Options gathered when the `ask` runs name a rule and its arguments;
/// one that names no rule may not be taken.
#[test]
fn options_gathered_at_run_time_run_their_rule() {
    let mut sim = sim();
    let player = make(&mut sim, creature().with_control(Control::Direct), 4, 4);
    answer_free(&mut sim, player);
    let other = make(&mut sim, creature(), 6, 6);
    sim.set_tag(other, "cornered", true, Cause::DEBUG).unwrap();
    sim.set_tag(player, "cornered", true, Cause::DEBUG).unwrap();
    sim.pump();
    // The other shouted at once; the player is asked.
    assert_eq!(var(&sim, other, "talk:said"), Value::from("LOUD"));
    let request = sim.pending_decision().unwrap();
    let DecisionKind::Choice {
        choice, options, ..
    } = request.kind
    else {
        panic!("not a choice");
    };
    assert_eq!(
        options
            .iter()
            .map(|option| (option.label.as_str(), option.available))
            .collect::<Vec<_>>(),
        [("Mumble", false), ("Shout", true)]
    );
    let (mut loaded, _) = save_and_load(&sim);
    for sim in [&mut sim, &mut loaded] {
        sim.submit(Command::Choose {
            entity: player,
            choice,
            option: 1,
        })
        .unwrap();
        assert_eq!(var(sim, player, "talk:said"), Value::from("LOUD"));
        assert_eq!(sim.var(Subject::Global, "talk:shouts"), Value::Int(2));
    }
    assert_eq!(loaded.state_hash(), sim.state_hash());
    assert_eq!(faults(&sim.drain_events()), Vec::<String>::new());
}

/// A choice whose chooser stops being directly controlled is settled as
/// anyone else's is; one whose chooser is removed is forgotten.
#[test]
fn a_choice_follows_who_decides() {
    let mut sim = sim();
    let (who, _) = asked(&mut sim);
    sim.set_control(who, Control::Delegated, Cause::DEBUG)
        .unwrap();
    sim.pump();
    assert_eq!(var(&sim, who, "talk:paid"), Value::Int(5));
    assert!(sim.decisions().is_empty());

    let mut sim = self::sim();
    let (who, _) = asked(&mut sim);
    let hash_before = sim.hashes().rules;
    sim.remove_entity(who, Cause::DEBUG).unwrap();
    sim.pump();
    assert!(sim.decisions().is_empty());
    assert_ne!(sim.hashes().rules, hash_before);
    assert_eq!(
        sim.advance(AdvanceLimit::Until(Tick(5 * SECOND))),
        Advance::Reached(Tick(5 * SECOND))
    );
}

/// Mistakes in `message` and `ask` are load errors at the effect.
#[test]
fn messages_and_choices_that_are_not_well_formed_do_not_load() {
    let cases = [
        (
            r#"[ { "type": "eoc", "id": "bad", "do": [ { "message": "hello" } ] } ]"#,
            "no `self` is bound",
        ),
        (
            r#"[ { "type": "eoc", "id": "bad", "subjects": { "self": "entity" },
                   "do": [ { "ask": { "prompt": "Well?" } } ] } ]"#,
            "`ask` needs `options`",
        ),
        (
            r#"[ { "type": "eoc", "id": "bad", "subjects": { "self": "entity" },
                   "do": [ { "ask": { "prompt": "Well?", "options": [] } } ] } ]"#,
            "at least one option",
        ),
        (
            r#"[ { "type": "eoc", "id": "bad", "subjects": { "self": "entity" },
                   "do": [ { "ask": { "prompt": "Well?", "options": [ { "do": [] } ] } } ] } ]"#,
            "needs `label`",
        ),
        (
            r#"[ { "type": "eoc", "id": "bad", "subjects": { "self": "entity" },
                   "do": [ { "message": { "to": "self" } } ] } ]"#,
            "`message` needs `text`",
        ),
    ];
    for (text, wanted) in cases {
        let error = common::try_defs(text).unwrap_err();
        assert!(error.contains(wanted), "{wanted:?} not in:\n{error}");
    }
}
