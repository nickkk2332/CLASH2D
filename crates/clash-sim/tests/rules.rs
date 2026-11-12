//! The rule runtime from outside (plans/sim-kernel.md, package K5),
//! against the fake host of `clash_sim::rule::testing`.
//!
//! The fixtures are under `tests/fixtures/rule/mods`: `fx` holds a rule
//! for each effect of 05 §10.2, §10.3 and §10.5 and for each limit of
//! 05 §13.2; `alarm` is the body of the test script `rule_alarm`,
//! `winter` that of `winter_rules`, and `order` the rules that show who
//! answers an event and in what order. The mistakes a rule can hold are
//! written here, one small mod each.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use clash_core::{Dur, Hash128, Real, codec};
use clash_data::{DefTypes, Defs, LoadOutcome, Loader, explain};
use clash_expr::{Handle, Value};
use clash_sim::action::ActionDef;
use clash_sim::rule::testing::{FakeHost, REFUSED, SEALED};
use clash_sim::rule::{
    CORE_EFFECTS, EffectKind, EocDef, LIMIT_SETTINGS, Occurrence, RULE_REASONS, RuleFault, RuleSet,
    RuleState, Rules, reason,
};

// ---- the harness ---------------------------------------------------------

fn types() -> DefTypes {
    let mut types = DefTypes::new();
    clash_data::expr::register_defs(&mut types);
    clash_sim::register_defs(&mut types);
    types
}

fn fixtures() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/rule/mods")
}

fn explained(outcome: LoadOutcome) -> LoadOutcome {
    for diagnostic in &outcome.diagnostics {
        assert!(
            explain(&diagnostic.code).is_some(),
            "code {} has no explanation",
            diagnostic.code
        );
    }
    outcome
}

fn load(mods: &[&str]) -> LoadOutcome {
    let types = types();
    explained(
        Loader::new(&types)
            .expr_registry(&FakeHost::registry())
            .search_path(fixtures())
            .load_mods(mods),
    )
}

/// A mod written in a test, loaded beside the fixtures it requires.
fn load_written(requires: &[&str], data: &str) -> LoadOutcome {
    let types = types();
    let listed: Vec<String> = requires.iter().map(|id| format!("\"{id}\"")).collect();
    let manifest = format!(
        r#"{{ "id": "written", "requires": [{}] }}"#,
        listed.join(", ")
    );
    let mut mods: Vec<&str> = requires.to_vec();
    mods.push("written");
    explained(
        Loader::new(&types)
            .expr_registry(&FakeHost::registry())
            .search_path(fixtures())
            .memory_mod("written", &manifest, &[("data/written.json", data)])
            .load_mods(&mods),
    )
}

fn clean(outcome: LoadOutcome) -> Arc<Defs> {
    assert_eq!(outcome.diagnostics.render(), "");
    outcome.defs.expect("a clean load gives definitions")
}

fn host(mods: &[&str]) -> FakeHost {
    FakeHost::new(Arc::new(RuleSet::new(clean(load(mods)))))
}

/// Calls a rule by its qualified id.
fn call(
    host: &mut FakeHost,
    rule: &str,
    subjects: &[(&str, Value)],
    args: &[(&str, Value)],
) -> Value {
    let rules = host.rules().clone();
    let id = rules
        .lookup(rule)
        .unwrap_or_else(|| panic!("no rule `{rule}`"));
    rules
        .call(host, id, subjects, args)
        .unwrap_or_else(|reason| panic!("`{rule}` was refused: {reason}"))
}

fn me(host: &FakeHost) -> Value {
    Value::Subject(host.entity(1))
}

/// Calls a rule with the first entity as `self`.
fn call_me(host: &mut FakeHost, rule: &str, args: &[(&str, Value)]) -> Value {
    let myself = me(host);
    call(host, rule, &[("self", myself)], args)
}

fn real(value: f64) -> Value {
    Value::Real(Real::of(value))
}

/// A number by value: an int is accepted where a real is wanted and
/// stays an int (05 §3), so a real variable may hold either.
fn number(value: &Value) -> f64 {
    value
        .as_number()
        .unwrap_or_else(|| panic!("{value} is not a number"))
        .get()
}

/// Raises the fake host's `alarm_tripped` and pumps.
fn alarm(host: &mut FakeHost, who: Handle, loud: bool, kind: &str, energy: f64) -> usize {
    let here = Value::Subject(host.place(1));
    host.raise_event(
        "alarm_tripped",
        &[here, Value::Subject(who)],
        &[Value::Bool(loud), Value::from(kind), real(energy)],
    );
    host.pump()
}

fn noted(host: &FakeHost) -> Vec<&str> {
    host.verbs.iter().map(String::as_str).collect()
}

fn texts(value: &Value) -> Vec<String> {
    value
        .as_list()
        .unwrap_or_default()
        .iter()
        .map(Value::to_string)
        .collect()
}

// ---- the fixtures load ----------------------------------------------------

#[test]
fn the_fixtures_load_without_a_word() {
    let defs = clean(load(&["fx", "alarm", "winter", "order"]));
    let rules = defs.registry::<EocDef>();
    assert!(rules.len() > 40);
    // A rule with parameters is a verb in the registry, under its
    // qualified id, with its parameters as arguments.
    let registry = defs.expr_registry();
    let ignite = registry
        .verb_id("fx:ignite")
        .and_then(|verb| registry.verb(verb))
        .expect("the rule is a verb");
    let args: Vec<(&str, bool)> = ignite
        .spec
        .args
        .iter()
        .map(|arg| (arg.name.as_str(), arg.optional))
        .collect();
    assert_eq!(args, [("who", false), ("heat", true)]);
    assert_eq!(ignite.spec.doc, "Burns someone.");
    assert!(
        registry.verb_id("fx:plain").is_none(),
        "no parameters, no verb"
    );
    // The events the mods define are event types, hooks among them.
    let hook = registry
        .event_id("fx:ritual_completing")
        .and_then(|event| registry.event(event))
        .expect("the event is registered");
    assert!(hook.spec.hook);
    assert_eq!(hook.spec.mutable, ["power"]);

    // What a rule compiles to.
    let alarm = defs.get::<EocDef>(defs.lookup::<EocDef>("alarm:vault_alarm").unwrap());
    assert!(alarm.is_global() && !alarm.is_attached());
    assert_eq!(alarm.filter().len(), 1);
    let names: Vec<&str> = alarm.subject_names().iter().map(|name| &**name).collect();
    assert_eq!(names, ["here", "who", "global"]);
    let block = alarm.effects.block().expect("the effects are compiled");
    assert_eq!(block.path(), "do");
    assert_eq!(block.len(), 4);
    assert!(matches!(block.effects()[0].kind, EffectKind::Set { .. }));
    assert!(matches!(block.effects()[2].kind, EffectKind::Run { .. }));
    assert!(matches!(block.effects()[3].kind, EffectKind::Emit { .. }));
    assert_eq!(&*block.effects()[3].path, "do[3]");
    let curse = defs.get::<EocDef>(defs.lookup::<EocDef>("fx:curse").unwrap());
    assert!(curse.is_attached() && !curse.is_global());
    assert_eq!(curse.role(), 1, "`as` names the event's second subject");
    assert!(curse.once);
    let cooled = defs.get::<EocDef>(defs.lookup::<EocDef>("order:cooled").unwrap());
    assert_eq!(cooled.cooldown(), Some(Dur::from_secs(10)));
}

// ---- 05 §10.2: control flow ------------------------------------------------

#[test]
fn if_runs_then_or_else() {
    let mut host = host(&["fx"]);
    call(&mut host, "fx:branch", &[], &[("n", Value::Int(5))]);
    assert_eq!(
        noted(&host),
        ["note(big, null)", "note(not in between, null)"]
    );
    host.verbs.clear();
    call(&mut host, "fx:branch", &[], &[("n", Value::Int(0))]);
    assert_eq!(
        noted(&host),
        [
            "note(small, null)",
            "note(zero, null)",
            "note(not in between, null)"
        ]
    );
    host.verbs.clear();
    call(&mut host, "fx:branch", &[], &[("n", Value::Int(1))]);
    assert_eq!(noted(&host), ["note(small, null)"]);
    assert!(host.faults.is_empty());
}

#[test]
fn first_runs_the_first_entry_that_holds() {
    let mut host = host(&["fx"]);
    for (n, expected) in [(-3, "negative"), (4, "small"), (40, "large")] {
        host.verbs.clear();
        call(&mut host, "fx:first_of", &[], &[("n", Value::Int(n))]);
        assert_eq!(noted(&host), [format!("note({expected}, null)")]);
    }
}

#[test]
fn switch_branches_on_a_value() {
    let mut host = host(&["fx"]);
    let cases = [
        (Value::from("ash"), "grey"),
        (Value::Int(7), "seven"),
        (Value::Bool(true), "yes"),
        (Value::from("iron"), "other"),
        (Value::Null, "other"),
    ];
    for (what, expected) in cases {
        host.verbs.clear();
        call(&mut host, "fx:switch_on", &[], &[("what", what)]);
        assert_eq!(noted(&host), [format!("note({expected}, null)")]);
    }
}

#[test]
fn for_each_binds_each_element_in_order() {
    let mut host = host(&["fx"]);
    let xs = Value::list([Value::Int(1), Value::Int(2), Value::Int(3)]);
    assert_eq!(
        call(&mut host, "fx:each", &[], &[("xs", xs)]),
        Value::Int(36)
    );
    let none = Value::list([]);
    assert_eq!(
        call(&mut host, "fx:each", &[], &[("xs", none)]),
        Value::Int(30)
    );
    assert!(host.faults.is_empty());
}

#[test]
fn repeat_counts_from_zero() {
    let mut host = host(&["fx"]);
    let sum = call(&mut host, "fx:count_up", &[], &[("times", Value::Int(5))]);
    assert_eq!(sum, Value::Int(10));
    let none = call(&mut host, "fx:count_up", &[], &[("times", Value::Int(0))]);
    assert_eq!(none, Value::Int(0));
}

#[test]
fn while_loops_until_its_condition_fails() {
    let mut host = host(&["fx"]);
    let steps = call(&mut host, "fx:halve", &[], &[("n", Value::Int(64))]);
    assert_eq!(steps, Value::Int(6));
}

#[test]
fn weighted_runs_one_entry_by_its_weight() {
    let mut host = host(&["fx"]);
    let who = host.entity(1);
    for _ in 0..400 {
        call_me(&mut host, "fx:lottery", &[]);
    }
    let heavy = host.var(who, "fx:n").as_int().unwrap();
    let light = host.var(who, "fx:x").as_number().unwrap().get() as i64;
    assert_eq!(heavy + light, 400, "exactly one entry runs each time");
    assert!(
        (250..=350).contains(&heavy),
        "three to one: {heavy} to {light}"
    );
    assert!(host.verbs.is_empty(), "an entry of no weight is never run");

    // The same seed gives the same choices.
    let mut again = FakeHost::new(host.rules().clone());
    for _ in 0..400 {
        call_me(&mut again, "fx:lottery", &[]);
    }
    assert_eq!(again.var(who, "fx:n"), Value::Int(heavy));

    // With no stream to draw from, nothing runs and that is reported.
    again.rng = None;
    call_me(&mut again, "fx:lottery", &[]);
    assert_eq!(again.var(who, "fx:n"), Value::Int(heavy));
    assert_eq!(again.fault_codes(), [reason::NO_RANDOM]);
}

#[test]
fn as_rebinds_a_subject_for_a_block_and_for_what_it_calls() {
    let mut host = host(&["fx"]);
    let (one, two) = (host.entity(1), host.entity(2));
    call(
        &mut host,
        "fx:rebind",
        &[("self", Value::Subject(one))],
        &[("other", Value::Subject(two))],
    );
    // Inside the block `self` was the other one: set to 5, bumped by 2.
    assert_eq!(host.var(two, "fx:n"), Value::Int(7));
    // After it, `self` is who it was.
    assert_eq!(host.var(one, "fx:n"), Value::Int(1));
}

#[test]
fn stop_ends_the_rule_and_return_gives_a_value() {
    let mut host = host(&["fx"]);
    assert_eq!(call(&mut host, "fx:stops", &[], &[]), Value::Null);
    assert_eq!(noted(&host), ["note(before, null)"]);
    assert_eq!(
        call(&mut host, "fx:gives", &[], &[("n", Value::Int(4))]),
        Value::Int(8)
    );
    assert_eq!(
        call(&mut host, "fx:gives", &[], &[("n", Value::Int(-4))]),
        Value::Int(0)
    );
}

#[test]
fn else_runs_when_the_condition_does_not_hold() {
    let mut host = host(&["fx"]);
    call(&mut host, "fx:either", &[], &[("n", Value::Int(1))]);
    call(&mut host, "fx:either", &[], &[("n", Value::Int(0))]);
    assert_eq!(
        noted(&host),
        ["note(held, null)", "note(did not hold, null)"]
    );
}

// ---- 05 §10.3: variables ---------------------------------------------------

#[test]
fn let_set_add_and_unset_write_places() {
    let mut host = host(&["fx"]);
    let who = host.entity(1);
    call_me(&mut host, "fx:writes", &[("v", Value::Int(4))]);
    // `$a` was 5: set into the variable, then one added.
    assert_eq!(host.var(who, "fx:n"), Value::Int(6));
    assert_eq!(host.var(who, "fx:tag"), Value::from("text"));
    // A settable property goes through the host's setter.
    assert_eq!(host.prop(who, "health"), Value::Int(50));
    assert_eq!(host.var(who, "fx:x"), real(0.5));
    assert_eq!(texts(&host.var(who, "fx:log")), ["one", "two"]);
    // `$a` was doubled before it went into the tally.
    assert_eq!(
        host.var(who, "fx:tally"),
        Value::map([("ash", Value::Int(11)), ("iron", Value::Int(2))])
    );

    call_me(&mut host, "fx:forgets", &[]);
    assert!(!host.has_var(who, "fx:n"));
    assert_eq!(host.var(who, "fx:n"), Value::Int(0), "back to its default");
    assert_eq!(
        host.var(who, "fx:tally"),
        Value::map([("iron", Value::Int(2))])
    );
    assert!(host.faults.is_empty() && host.warnings.is_empty());
}

#[test]
fn a_let_takes_a_value_source() {
    let mut host = host(&["fx"]);
    let value = call(&mut host, "fx:sourced", &[], &[("n", Value::Int(5))]);
    assert_eq!(value, Value::Int(12));
}

// ---- 05 §9.5, §10.5: rules and events --------------------------------------

#[test]
fn rules_are_run_called_as_verbs_and_give_values_back() {
    let mut host = host(&["fx"]);
    let (one, two) = (host.entity(1), host.entity(2));
    host.set_prop(one, "health", real(100.0));
    host.set_prop(two, "health", real(100.0));
    let say = host
        .rules()
        .rule_value(host.rules().lookup("fx:say").unwrap());
    call(
        &mut host,
        "fx:caller",
        &[("self", Value::Subject(one))],
        &[("foe", Value::Subject(two)), ("which", say)],
    );
    // As a verb with the default heat, then with heat 2.
    assert_eq!(host.prop(two, "health"), real(100.0 - 40.0 - 80.0));
    // By `run`, with `self` given as an argument.
    assert_eq!(host.prop(one, "health"), real(100.0 - 20.0));
    // What `gives` returned was taken by a `let`.
    assert_eq!(host.var(one, "fx:n"), Value::Int(14));
    let notes: Vec<&str> = noted(&host)
        .into_iter()
        .filter(|line| line.starts_with("note"))
        .collect();
    assert_eq!(notes, ["note(plain ran, null)", "note(computed, null)"]);
    assert!(host.faults.is_empty());
}

#[test]
fn a_called_rule_has_locals_of_its_own() {
    let mut host = host(&["fx"]);
    // `shadow` defines a `$secret` and changes its `$n`; its caller's are
    // untouched.
    assert_eq!(call(&mut host, "fx:keeps", &[], &[]), Value::Int(73));
}

#[test]
fn a_rule_is_called_from_outside_with_its_parameters_checked() {
    let mut host = host(&["fx"]);
    let rules = host.rules().clone();
    let ignite = rules.lookup("fx:ignite").unwrap();
    let missing = rules.call(&mut host, ignite, &[], &[]).unwrap_err();
    assert_eq!(missing.code, reason::BAD_ARGUMENT);
    assert!(
        missing
            .message
            .contains("needs a value for its parameter `who`")
    );
    let myself = me(&host);
    let unknown = rules
        .call(&mut host, ignite, &[], &[("whom", myself)])
        .unwrap_err();
    assert!(unknown.message.contains("has no parameter `whom`"));
    let wrong = rules
        .call(&mut host, ignite, &[], &[("who", Value::Int(3))])
        .unwrap_err();
    assert!(
        wrong
            .message
            .contains("was given a int for its parameter `who`")
    );
    assert!(host.verbs.is_empty() && host.faults.is_empty());
}

#[test]
fn a_computed_call_is_checked_when_it_runs() {
    let mut host = host(&["fx"]);
    let say = host
        .rules()
        .rule_value(host.rules().lookup("fx:say").unwrap());
    call(&mut host, "fx:blind_call", &[], &[("which", say)]);
    call(&mut host, "fx:blind_call", &[], &[("which", Value::Int(3))]);
    assert_eq!(
        host.fault_codes(),
        [reason::BAD_ARGUMENT, reason::UNKNOWN_RULE]
    );
    assert!(host.faults[0].message.contains("no parameter `bogus`"));
    assert_eq!(
        noted(&host),
        ["note(carried on, null)", "note(carried on, null)"]
    );
}

#[test]
fn queued_rules_run_when_the_rule_has_finished_in_order() {
    let mut host = host(&["fx"]);
    call(&mut host, "fx:queues", &[], &[]);
    assert_eq!(
        noted(&host),
        [
            "note(first, null)",
            "note(plain ran, null)",
            "note(queued, null)"
        ]
    );
    assert!(host.state.is_idle());
}

#[test]
fn a_rule_is_attached_and_detached_with_its_own_state() {
    let mut host = host(&["fx"]);
    let (one, two) = (host.entity(1), host.entity(2));
    let curse = host.rules().lookup("fx:curse").unwrap();

    // Not attached: the rule answers nothing, although it listens.
    alarm(&mut host, one, true, "x", 1.0);
    assert_eq!(host.var(one, "fx:n"), Value::Int(0));

    call(
        &mut host,
        "fx:lay_curse",
        &[],
        &[("victim", Value::Subject(one))],
    );
    call(
        &mut host,
        "fx:lay_curse",
        &[],
        &[("victim", Value::Subject(one))],
    );
    assert_eq!(host.state.attached_to(one), [curse], "twice is once");
    alarm(&mut host, two, true, "x", 1.0);
    assert_eq!(host.var(one, "fx:n"), Value::Int(0), "someone else's alarm");
    alarm(&mut host, one, true, "x", 1.0);
    assert_eq!(host.var(one, "fx:n"), Value::Int(1));
    // It runs `once` for its owner.
    alarm(&mut host, one, true, "x", 1.0);
    assert_eq!(host.var(one, "fx:n"), Value::Int(1));
    assert_eq!(
        host.state
            .record(curse, Some(one))
            .map(|record| record.runs),
        Some(1)
    );

    // Detached, it is gone, and so is what was kept for it.
    call(
        &mut host,
        "fx:lift_curse",
        &[],
        &[("victim", Value::Subject(one))],
    );
    assert!(host.state.attached_to(one).is_empty());
    assert_eq!(host.state.record(curse, Some(one)), None);
    assert_eq!(host.state, RuleState::default());
    alarm(&mut host, one, true, "x", 1.0);
    assert_eq!(host.var(one, "fx:n"), Value::Int(1));

    // Attached again it starts afresh.
    call(
        &mut host,
        "fx:lay_curse",
        &[],
        &[("victim", Value::Subject(one))],
    );
    alarm(&mut host, one, true, "x", 1.0);
    assert_eq!(host.var(one, "fx:n"), Value::Int(2));
    assert!(host.faults.is_empty());
}

#[test]
fn only_an_attached_rule_of_the_right_kind_is_attached() {
    let mut host = host(&["fx"]);
    let one = host.entity(1);
    let place = host.place(1);
    call(
        &mut host,
        "fx:lay_wrongly",
        &[],
        &[
            ("victim", Value::Subject(one)),
            ("where", Value::Subject(place)),
        ],
    );
    assert_eq!(
        host.fault_codes(),
        [reason::NOT_ATTACHABLE, reason::NOT_ATTACHABLE]
    );
    assert_eq!(host.state, RuleState::default());
    assert_eq!(noted(&host), ["note(carried on, null)"]);
}

// ---- 05 §8.1, §8.2: a hook of a mod's own ----------------------------------

#[test]
fn a_handler_changes_a_mutable_field_of_a_custom_hook() {
    let mut host = host(&["fx"]);
    let who = host.entity(1);
    // Raised by a rule, which takes the outcome with a `let`.
    let power = call_me(&mut host, "fx:ritual", &[("p", real(10.0))]);
    assert_eq!(power, real(20.0));
    assert_eq!(host.events.len(), 2);
    assert!(host.events[0].starts_with("fx:ritual_completing["));
    assert!(host.events[1].starts_with("fx:ritual_completed["));
    // The notification is delivered at the pump. One rule consumes it,
    // and the one marked `skip_if_consumed` is skipped.
    assert_eq!(host.pending(), 1);
    assert_eq!(number(&host.var(who, "fx:x")), 0.0);
    assert_eq!(host.pump(), 1);
    assert_eq!(number(&host.var(who, "fx:x")), 20.0);
    assert!(host.verbs.is_empty());

    // Raised by the engine, which goes on with the changed value.
    let raised = host.raise_event(
        "fx:ritual_completing",
        &[me(&host)],
        &[real(3.0), Value::from("fire")],
    );
    assert_eq!(raised.fields, [real(6.0), Value::from("fire")]);
    assert_eq!(raised.cancelled, None);
    // The handler's condition is tested against the event.
    let raised = host.raise_event(
        "fx:ritual_completing",
        &[me(&host)],
        &[real(3.0), Value::from("frost")],
    );
    assert_eq!(raised.fields[0], real(3.0));
    assert!(host.faults.is_empty());
}

#[test]
fn a_handler_cancels_a_custom_hook_with_a_reason() {
    let mut host = host(&["fx"]);
    let who = host.entity(1);
    host.set_prop(who, "name", "Mara");
    host.set_var(who, "fx:tag", "warded");

    let raised = host.raise_event(
        "fx:ritual_completing",
        &[me(&host)],
        &[real(3.0), Value::from("fire")],
    );
    let why = raised.cancelled.expect("the ward cancels");
    assert_eq!(why.code, "fx:warded");
    assert_eq!(why.message, "the Mara is warded against fire rituals");
    assert_eq!(why.value("school"), Some(&clash_core::Value::from("fire")));
    assert!(why.value("caster").is_some());
    // A cancellation stops delivery: the amplifier, which comes later,
    // never ran.
    assert_eq!(raised.fields[0], real(3.0));

    // The rule that raised it is told, and does not go on.
    let power = call_me(&mut host, "fx:ritual", &[("p", real(10.0))]);
    assert_eq!(power, Value::Int(0));
    assert_eq!(noted(&host), ["note(refused: fx:warded, null)"]);
    assert_eq!(host.pending(), 0, "the ritual was not completed");

    // With no reason of its own, the engine's stands in.
    host.set_var(who, "fx:tag", "vetoed");
    let raised = host.raise_event(
        "fx:ritual_completing",
        &[me(&host)],
        &[real(3.0), Value::from("fire")],
    );
    let why = raised.cancelled.expect("the veto cancels");
    assert_eq!(why.code, "events.cancelled");
    assert_eq!(
        why.value("event"),
        Some(&clash_core::Value::from("fx:ritual_completing"))
    );
    assert!(host.faults.is_empty());
}

// ---- 05 §13.2: every limit reports and the simulation continues ------------

fn fault<'a>(host: &'a FakeHost, code: &str) -> &'a RuleFault {
    host.faults
        .iter()
        .find(|fault| fault.code == code)
        .unwrap_or_else(|| panic!("no fault `{code}` among {:?}", host.fault_codes()))
}

#[test]
fn a_run_that_never_ends_is_ended_by_the_step_budget() {
    let mut host = host(&["fx"]);
    host.limits.max_steps = 500;
    assert_eq!(call(&mut host, "fx:spin", &[], &[]), Value::Null);
    assert_eq!(host.fault_codes(), [reason::STEP_LIMIT]);
    assert_eq!(noted(&host), ["note(spinning, null)"]);
    // The fault knows the rule and where in it.
    let rules = host.rules().clone();
    let reported = rules.fault_reason(&host.faults[0]);
    assert_eq!(
        reported.value("rule"),
        Some(&clash_core::Value::from("fx:spin"))
    );
    assert_eq!(
        reported.value("path"),
        Some(&clash_core::Value::from("do[1]"))
    );
    let diagnostic = rules.fault_diagnostic(&host.faults[0]);
    let location = diagnostic.location.expect("a fault has a place");
    assert!(location.file.ends_with("fx/data/limits.json"));
    assert!(location.source_line.contains(r#""while": "true""#));

    // A rule that was called is ended with its caller: the budget is
    // the whole run's.
    host.verbs.clear();
    call(&mut host, "fx:spin_inside", &[], &[]);
    assert_eq!(noted(&host), ["note(spinning, null)"]);

    // And the simulation carries on.
    host.verbs.clear();
    call(&mut host, "fx:plain", &[], &[]);
    assert_eq!(noted(&host), ["note(plain ran, null)"]);
    assert!(host.state.is_idle());
}

#[test]
fn a_rule_that_calls_itself_is_refused_at_the_call_depth() {
    let mut host = host(&["fx"]);
    let who = host.entity(1);
    call_me(&mut host, "fx:deeper", &[("n", Value::Int(0))]);
    assert_eq!(host.fault_codes(), [reason::CALL_DEPTH]);
    // Thirty-two deep, and no deeper.
    assert_eq!(host.var(who, "fx:n"), Value::Int(31));
    // Every caller carried on after the call that was refused.
    assert_eq!(number(&host.var(who, "fx:x")), 32.0);
}

fn depth(value: &Value) -> usize {
    let mut depth = 0;
    let mut at = value.clone();
    while let Value::List(items) = at {
        depth += 1;
        at = items.first().cloned().unwrap_or_default();
    }
    depth
}

#[test]
fn a_value_that_wraps_itself_stops_at_the_depth_limit() {
    let mut host = host(&["fx"]);
    let who = host.entity(1);
    call_me(&mut host, "fx:nest", &[]);
    assert_eq!(depth(&host.var(who, "fx:nest")), 32);
    // A hundred tries each for the variable and the local; all but
    // thirty-one of each were refused, and the reports were thinned.
    assert_eq!(host.fault_count, 2 * (100 - 31));
    assert!(host.faults.len() < 20);
    assert!(
        host.faults
            .iter()
            .all(|fault| fault.code == reason::VALUE_TOO_DEEP)
    );
    assert_eq!(noted(&host), ["note(done, null)"]);
}

#[test]
fn a_list_stops_growing_at_the_size_limit() {
    let mut host = host(&["fx"]);
    let who = host.entity(1);
    host.limits.max_value_size = 8;
    call_me(&mut host, "fx:swell", &[]);
    assert_eq!(
        host.var(who, "fx:log").as_list().map(<[Value]>::len),
        Some(8)
    );
    assert_eq!(host.fault_count, 2);
    assert_eq!(
        fault(&host, reason::VALUE_TOO_LARGE).rule,
        host.rules().lookup("fx:swell")
    );
    assert_eq!(noted(&host), ["note(done, null)"]);
}

#[test]
fn the_queue_holds_so_many_rules_and_no_more() {
    let mut host = host(&["fx"]);
    host.limits.max_queued = 5;
    call(&mut host, "fx:flood", &[], &[]);
    assert_eq!(host.fault_count, 15);
    assert!(
        host.faults
            .iter()
            .all(|fault| fault.code == reason::QUEUE_LIMIT)
    );
    let ran = noted(&host)
        .iter()
        .filter(|line| **line == "note(plain ran, null)")
        .count();
    assert_eq!(ran, 5);
    assert_eq!(noted(&host)[0], "note(flooded, null)");
    // The count starts again once the queue has been emptied.
    host.verbs.clear();
    host.limits.max_queued = 100;
    call(&mut host, "fx:flood", &[], &[]);
    assert_eq!(host.verbs.len(), 21);
    assert_eq!(host.fault_count, 15);
}

#[test]
fn a_hook_raised_from_inside_itself_is_skipped_at_the_hook_depth() {
    let mut host = host(&["fx"]);
    let who = host.entity(1);
    let raised = host.raise_event("fx:echo", &[me(&host)], &[Value::Int(0)]);
    assert_eq!(raised.cancelled, None);
    assert_eq!(host.var(who, "fx:n"), Value::Int(8));
    assert_eq!(host.fault_codes(), ["events.hook_depth"]);
}

#[test]
fn timers_and_queries_are_kept_and_say_so_when_they_run() {
    let mut host = host(&["fx"]);
    call(&mut host, "fx:later", &[], &[]);
    // The fake host keeps no timers: `after` and `every` are refused,
    // and cancelling a timer it does not have does nothing.
    assert_eq!(
        host.fault_codes(),
        [reason::TIMERS_UNAVAILABLE, reason::TIMERS_UNAVAILABLE]
    );
    assert_eq!(noted(&host), ["note(now, null)"]);
    // The effects inside are compiled all the same, so a mistake in
    // them is found at load.
    let later = host
        .rules()
        .rule(host.rules().lookup("fx:later").unwrap())
        .unwrap();
    let block = later.effects.block().unwrap();
    let EffectKind::Timer(timer) = &block.effects()[0].kind else {
        panic!("`after` is kept as a timer");
    };
    assert_eq!(timer.body.len(), 1);
    assert_eq!(timer.body.path(), "do[0].do");
    assert_eq!(timer.while_unloaded.as_deref(), Some("elapse"));

    host.faults.clear();
    host.verbs.clear();
    call(&mut host, "fx:asks", &[], &[]);
    // The fake host answers no query: the body runs for nothing, with a
    // warning, and the rule carries on.
    assert!(host.fault_codes().is_empty());
    assert_eq!(
        host.warnings.last().map(|warning| warning.fault),
        Some(clash_expr::Fault::Unavailable("queries"))
    );
    assert_eq!(noted(&host), ["note(asked, null)"]);
}

#[test]
fn what_the_simulation_refuses_is_reported_and_the_rule_carries_on() {
    let mut host = host(&["fx"]);
    call_me(&mut host, "fx:refused", &[("nobody", Value::Null)]);
    assert_eq!(
        host.fault_codes(),
        [
            reason::VERB_REFUSED,
            reason::WRITE_REFUSED,
            reason::EMIT_REFUSED,
            reason::NO_SUBJECT,
            reason::TYPE_MISMATCH,
            reason::BAD_KEY,
        ]
    );
    // Each carries the reason the simulation gave.
    let verb = fault(&host, reason::VERB_REFUSED);
    let refusal = verb.refusal.as_ref().expect("the verb's reason");
    assert_eq!(
        (refusal.code.as_str(), refusal.message.as_str()),
        (REFUSED, "not today")
    );
    assert_eq!(
        fault(&host, reason::WRITE_REFUSED)
            .refusal
            .as_ref()
            .map(|reason| reason.code.as_str()),
        Some(SEALED)
    );
    let rules = host.rules().clone();
    let reported = rules.fault_reason(verb);
    assert_eq!(
        reported.value("reason"),
        Some(&clash_core::Value::from(REFUSED))
    );
    assert_eq!(
        noted(&host),
        ["refuse(not today)", "note(carried on, null)"]
    );

    // A value of the wrong type is not returned.
    host.faults.clear();
    let value = call(
        &mut host,
        "fx:returns_wrongly",
        &[],
        &[("what", Value::from("text"))],
    );
    assert_eq!(value, Value::Null);
    assert_eq!(host.fault_codes(), [reason::TYPE_MISMATCH]);
    let value = call(
        &mut host,
        "fx:returns_wrongly",
        &[],
        &[("what", Value::Int(3))],
    );
    assert_eq!(value, Value::Int(3));
}

// ---- the test script `rule_alarm` -------------------------------------------

#[test]
fn rule_alarm() {
    let mut host = host(&["alarm"]);
    let (guard, thief) = (host.entity(1), host.entity(2));
    let (vault, site) = (host.place(1), host.site(1));
    host.set_prop(vault, "site", Value::Subject(site));
    let global = host.global_handle();

    // Something that is not glass breaking: the filter keeps the rule
    // from being looked at.
    assert_eq!(alarm(&mut host, guard, true, "wood", 5.0), 1);
    assert_eq!(host.var(site, "alarm:alarm_raised"), Value::Bool(false));

    // Glass, loudly: the rule sets a variable, adds to another, runs a
    // rule and raises an event, which a third rule answers.
    assert_eq!(alarm(&mut host, guard, true, "glass", 5.0), 2);
    assert_eq!(host.var(site, "alarm:alarm_raised"), Value::Bool(true));
    assert_eq!(host.var(global, "alarm:alarms"), Value::Int(1));
    assert_eq!(host.var(guard, "alarm:alerted"), Value::Bool(true));
    assert_eq!(host.var(thief, "alarm:alerted"), Value::Bool(false));
    let told = format!("note(the alarm is raised, {})", Value::Subject(guard));
    assert_eq!(
        noted(&host),
        [
            told.as_str(),
            "note(a squad sets out, null)",
            "note(a squad sets out, null)"
        ]
    );
    assert!(
        host.events.iter().any(|event| {
            event.starts_with("alarm:reinforcements_due[") && event.ends_with("(2)")
        })
    );

    // Again: the alarm has been raised, so the condition fails.
    assert_eq!(alarm(&mut host, thief, true, "glass", 5.0), 1);
    assert_eq!(host.var(global, "alarm:alarms"), Value::Int(1));
    assert_eq!(host.var(thief, "alarm:alerted"), Value::Bool(false));
    assert!(host.faults.is_empty() && host.warnings.is_empty());
    assert!(host.state.is_idle());

    // The same events give the same state twice.
    let mut again = FakeHost::new(host.rules().clone());
    again.set_prop(vault, "site", Value::Subject(site));
    alarm(&mut again, guard, true, "wood", 5.0);
    alarm(&mut again, guard, true, "glass", 5.0);
    alarm(&mut again, thief, true, "glass", 5.0);
    assert_eq!(again.verbs, host.verbs);
    assert_eq!(again.events, host.events);
    assert_eq!(Hash128::of(&again.state), Hash128::of(&host.state));
}

// ---- the test script `winter_rules` ------------------------------------------

#[test]
fn winter_rules() {
    let mut host = host(&["winter"]);
    let who = host.entity(1);
    let global = host.global_handle();
    let chills = |host: &mut FakeHost| host.var(who, "winter:chills").as_int().unwrap();

    // The group is off: its rule does not run; a rule in no group does.
    alarm(&mut host, who, true, "x", 1.0);
    assert_eq!(chills(&mut host), 0);
    assert_eq!(host.var(who, "winter:alarms"), Value::Int(1));

    // A setting its `enabled` reads turns it on.
    host.settings
        .insert("harsh_winter".to_owned(), Value::Bool(true));
    alarm(&mut host, who, true, "x", 1.0);
    assert_eq!(chills(&mut host), 1);
    host.settings.clear();
    alarm(&mut host, who, true, "x", 1.0);
    assert_eq!(chills(&mut host), 1);
    // So does a variable.
    host.set_var(global, "winter:deep_winter", true);
    alarm(&mut host, who, true, "x", 1.0);
    assert_eq!(chills(&mut host), 2);
    host.set_var(global, "winter:deep_winter", false);

    // `set_rule_group` switches it from a rule, whatever `enabled` says.
    call(&mut host, "winter:freeze", &[], &[]);
    alarm(&mut host, who, true, "x", 1.0);
    assert_eq!(chills(&mut host), 3);
    call(&mut host, "winter:thaw", &[], &[]);
    host.settings
        .insert("harsh_winter".to_owned(), Value::Bool(true));
    alarm(&mut host, who, true, "x", 1.0);
    assert_eq!(chills(&mut host), 3);

    // A rule of the group that is called does not run either.
    let shiver = call_me(&mut host, "winter:shiver", &[]);
    assert_eq!(shiver, Value::Null);
    assert_eq!(chills(&mut host), 3);

    // The switch is state: it survives a save and a load.
    let saved = codec::to_bytes(&host.state).expect("the state is encoded");
    let loaded: RuleState = codec::from_bytes(&saved).expect("and decoded");
    assert_eq!(loaded, host.state);
    assert_eq!(Hash128::of(&loaded), Hash128::of(&host.state));
    assert_ne!(Hash128::of(&loaded), Hash128::of(&RuleState::default()));
    let mut resumed = FakeHost::new(host.rules().clone());
    resumed.state = loaded;
    resumed
        .settings
        .insert("harsh_winter".to_owned(), Value::Bool(true));
    alarm(&mut resumed, who, true, "x", 1.0);
    assert_eq!(
        resumed.var(who, "winter:chills"),
        Value::Int(0),
        "still off"
    );

    // Left to `enabled` again, the setting governs.
    call(&mut host, "winter:seasons", &[], &[]);
    assert_eq!(host.state, RuleState::default());
    alarm(&mut host, who, true, "x", 1.0);
    assert_eq!(chills(&mut host), 4);
    let shiver = call_me(&mut host, "winter:shiver", &[]);
    assert_eq!(shiver, Value::Int(1));
    assert_eq!(chills(&mut host), 14);
    assert!(host.faults.is_empty() && host.warnings.is_empty());
}

// ---- 05 §8.3, §8.4, §9: who answers an event, and in what order --------------

fn trace(host: &mut FakeHost) -> Vec<String> {
    let global = host.global_handle();
    let lines = texts(&host.var(global, "order:trace"));
    host.set_var(global, "order:trace", Value::list([]));
    lines
}

#[test]
fn rules_answer_an_event_in_the_order_of_05() {
    let mut host = host(&["order"]);
    let who = host.entity(1);
    let here = host.place(1);
    let rules = host.rules().clone();
    // One rule carried by a definition, one attached at run time.
    host.carry(here, rules.lookup("order:on_here").unwrap());
    assert!(
        host.state
            .attach(who, rules.lookup("order:on_who").unwrap())
    );
    // A rule attached to something that takes no part costs nothing.
    assert!(
        host.state
            .attach(host.entity(9), rules.lookup("order:on_who").unwrap())
    );

    alarm(&mut host, who, true, "glass", 1.0);
    // Global rules by priority and then id; then the rules attached to
    // each subject, in the order the event declares its subjects.
    assert_eq!(
        trace(&mut host),
        ["z_high", "filtered", "a_low", "b_low", "on_here", "on_who"]
    );
    assert_eq!(host.var(who, "order:count"), Value::Int(1));
    assert_eq!(host.var(host.entity(9), "order:count"), Value::Int(0));

    // The rest of a filter is tested after the index found the rule.
    alarm(&mut host, who, false, "glass", 1.0);
    assert_eq!(
        trace(&mut host),
        ["z_high", "a_low", "b_low", "on_here", "on_who"]
    );

    // 05 §13.5: the rules attached to something that is stored do not
    // run.
    host.store(who);
    alarm(&mut host, who, true, "wood", 1.0);
    assert_eq!(trace(&mut host), ["z_high", "a_low", "b_low", "on_here"]);
    host.restore(who);
    alarm(&mut host, who, true, "wood", 1.0);
    assert_eq!(trace(&mut host).last().map(String::as_str), Some("on_who"));
}

#[test]
fn once_and_cooldown_count_runs_in_which_the_condition_held() {
    let mut host = host(&["order"]);
    let (one, two) = (host.entity(1), host.entity(2));
    let rules = host.rules().clone();
    let common = ["z_high", "a_low", "b_low"];
    let extra = |host: &mut FakeHost| -> Vec<String> {
        trace(host)
            .into_iter()
            .filter(|line| !common.contains(&line.as_str()))
            .collect()
    };

    // `once`: a run in which the condition did not hold does not count.
    alarm(&mut host, one, false, "once", 1.0);
    assert!(extra(&mut host).is_empty());
    alarm(&mut host, one, true, "once", 1.0);
    assert_eq!(extra(&mut host), ["once"]);
    alarm(&mut host, one, true, "once", 1.0);
    assert!(extra(&mut host).is_empty());

    // `cooldown`: on the clock of the owner; universal time for a
    // global rule.
    let each = rules.lookup("order:cooled_each").unwrap();
    host.state.attach(one, each);
    host.state.attach(two, each);
    host.clock = Dur::from_secs(100);
    alarm(&mut host, one, true, "cool", 1.0);
    alarm(&mut host, two, true, "cool", 1.0);
    assert_eq!(extra(&mut host), ["cooled"], "the second came too soon");
    assert_eq!(host.var(one, "order:count"), Value::Int(1));
    assert_eq!(host.var(two, "order:count"), Value::Int(1));

    host.clock = Dur::from_secs(109);
    alarm(&mut host, one, true, "cool", 1.0);
    assert!(extra(&mut host).is_empty());
    assert_eq!(host.var(one, "order:count"), Value::Int(1));

    // Ten seconds on: everyone's clock but the second one's, which
    // runs slow.
    host.clock = Dur::from_secs(110);
    host.clocks.insert(two, Dur::from_secs(104));
    alarm(&mut host, one, true, "cool", 1.0);
    alarm(&mut host, two, true, "cool", 1.0);
    assert_eq!(extra(&mut host), ["cooled"]);
    assert_eq!(host.var(one, "order:count"), Value::Int(2));
    assert_eq!(host.var(two, "order:count"), Value::Int(1));
    let record = host
        .state
        .record(each, Some(one))
        .expect("a rule with a cooldown is recorded");
    assert_eq!((record.runs, record.last), (2, 110_000_000));

    // What is kept for a subject goes when the subject does.
    host.state.forget(one);
    assert_eq!(host.state.record(each, Some(one)), None);
    assert!(host.state.attached_to(one).is_empty());
    assert_eq!(host.state.attached_to(two), [each]);
}

#[test]
fn a_consumed_event_skips_the_rules_that_ask_to_be_skipped() {
    let mut host = host(&["order"]);
    let who = host.entity(1);
    alarm(&mut host, who, true, "eat", 1.0);
    assert_eq!(
        trace(&mut host),
        ["z_high", "eater", "a_low", "b_low", "rude"]
    );
}

#[test]
fn a_rule_runs_do_or_else_by_its_condition() {
    let mut host = host(&["order"]);
    let who = host.entity(1);
    let picked = |host: &mut FakeHost| -> Vec<String> {
        trace(host)
            .into_iter()
            .filter(|line| line == "yes" || line == "no")
            .collect()
    };
    alarm(&mut host, who, true, "pick", 12.0);
    assert_eq!(picked(&mut host), ["yes"]);
    alarm(&mut host, who, false, "pick", 12.0);
    assert_eq!(picked(&mut host), ["no"]);
    // A filter compares numbers by value: the int written matches the
    // real the event carries, and no other.
    alarm(&mut host, who, true, "pick", 11.0);
    assert!(picked(&mut host).is_empty());
}

// ---- an effect list of another definition ------------------------------------

#[test]
fn an_actions_effect_list_runs_against_the_actions_locals() {
    let data = r#"[
  { "type": "action", "id": "charge",
    "vars": { "power": { "type": "int", "default": 0 } },
    "on_start": [ { "note": "started" }, { "stop": {} }, { "note": "never" } ],
    "phases": [
      { "id": "gather", "time": 0,
        "do": [ { "let": { "step": 2 } },
                { "set": { "$power": "$power + $step" } },
                { "set": { "self.var.n": "$power" } } ] }
    ] }
]"#;
    let defs = clean(load_written(&["fx"], data));
    let action = defs.get::<ActionDef>(defs.lookup::<ActionDef>("written:charge").unwrap());
    let rules = Arc::new(RuleSet::new(defs.clone()));
    let mut host = FakeHost::new(rules.clone());
    let who = host.entity(1);

    // The action's own locals, in its slots; what the list defines with
    // `let` comes after them and is gone when the list ends.
    let gather = &action.phases()[0].on_enter;
    assert_eq!(gather.frame_size(), 2);
    let mut locals = vec![Value::Int(5)];
    let actor = [me(&host)];
    let outcome = rules.run_effects(&mut host, gather, &actor, &mut locals, None);
    assert!(!outcome.aborted);
    assert_eq!(locals, [Value::Int(7)]);
    rules.run_effects(&mut host, gather, &actor, &mut locals, None);
    assert_eq!(locals, [Value::Int(9)]);
    assert_eq!(host.var(who, "fx:n"), Value::Int(9));

    rules.run_effects(&mut host, &action.on_start, &actor, &mut locals, None);
    assert_eq!(noted(&host), ["note(started, null)"]);
    // A list that was left out does nothing.
    let nothing = rules.run_effects(&mut host, &action.on_finish, &actor, &mut locals, None);
    assert_eq!(nothing.value, Value::Null);
    assert!(host.faults.is_empty() && host.state.is_idle());
}

// ---- mistakes are found at load, each with its place -------------------------

/// A mistake: the rule it is in, text on the line it is reported at,
/// its code, its JSON path, what is said, and what is suggested.
type Mistake = (
    &'static str,
    &'static str,
    &'static str,
    &'static str,
    &'static str,
    &'static str,
);

#[rustfmt::skip]
const MISTAKES: &[Mistake] = &[
    // What the brief of the package names.
    ("unknown_effect", r#""notee""#, "E0681", "do[0].notee", "unknown effect `notee`", "note"),
    ("wrong_type", r#""'a lot'""#, "E0645", "do[0].hurt.amount", "must be real", ""),
    ("wrong_literal", "true }", "E0645", "do[0].hurt.amount", "must be real", ""),
    ("not_settable", r#""self.name""#, "E0631", "do[0].set.self.name", "cannot be set", ""),
    // Arguments.
    ("unknown_argument", r#""knd""#, "E0682", "do[0].hurt.knd", "unknown argument `knd`", "kind"),
    ("missing_argument", r#""hurt""#, "E0682", "do[0].hurt", "missing argument `amount`", ""),
    ("by_name_only", r#""hurt": 3"#, "E0682", "do[0].hurt", "takes its arguments by name", ""),
    ("two_keys", r#"{ "note": "a", "refuse""#, "E0682", "do[0]", "an effect is an object with one key", ""),
    ("not_an_object", r#"[ "note" ]"#, "E0682", "do[0]", "an effect is an object with one key", ""),
    // Rules as verbs, and calls.
    ("no_such_rule", r#""nobody""#, "E0626", "do[0].run", "nobody", ""),
    ("unknown_parameter", r#""whom""#, "E0682", "do[0].fx:ignite.whom", "unknown parameter `whom`", "who"),
    ("missing_parameter", r#""fx:ignite""#, "E0682", "do[0].fx:ignite", "missing parameter `who`", ""),
    ("takes_none", r#""x": 1"#, "E0682", "do[0].run.with.x", "takes no parameters", ""),
    ("not_a_verb", r#""fx:plain""#, "E0681", "do[0].fx:plain", "a rule with no `params`", ""),
    ("peeks", "$secret", "E0622", "do[0].note.expr", "unknown local `$secret`", ""),
    ("needs_self", r#""fx:bump""#, "E0682", "do[0]", "is given the subject `self` by whoever calls it", ""),
    ("source_misplaced", r#""run": "fx:plain""#, "E0683", "do[0].note", "is taken by a `let`", ""),
    // A rule is still refused in an expression: only functions are called there.
    ("rule_in_expression", "fx:gives(3)", "E0624", "do[0].note.expr", "fx:gives", ""),
    // Events and hooks.
    ("no_such_event", r#""alarm_triped""#, "E0684", "on", "there is no event `alarm_triped`", "alarm_tripped"),
    ("as_unknown", r#""whoo""#, "E0684", "as", "has no subject `whoo`", "who"),
    ("as_unattached", r#""who""#, "E0684", "as", "is not attached", ""),
    ("filter_unknown", r#""glass""#, "E0684", "filter.knd", "has no field `knd`", "kind"),
    ("filter_wrong_type", r#""yes""#, "E0621", "filter.loud", "yes", ""),
    ("modify_notification", r#""event.energy""#, "E0683", "do[0].modify.event.energy", "is a notification", ""),
    ("modify_fixed", r#""event.kind""#, "E0683", "do[0].modify.event.kind", "may not change `kind`", ""),
    ("modify_a_variable", r#""self.var.n""#, "E0683", "do[0].modify.self.var.n", "changes a field of the hook", ""),
    ("cancel_notification", r#""cancel_event""#, "E0683", "do[0].cancel_event", "is not in a rule that listens for a hook", ""),
    ("consume_nothing", r#""consume""#, "E0683", "do[0].consume", "not in a rule that listens for one", ""),
    ("emit_unknown", r#""ritual_complete""#, "E0682", "do[0].emit.event", "there is no event `ritual_complete`", "ritual_completed"),
    ("emit_no_subject", r#"{ "event": "ritual_completed", "power": 1 }"#, "E0682", "do[0].emit", "needs its subject `caster`", ""),
    ("emit_unknown_field", r#""powr""#, "E0682", "do[0].emit.powr", "has no subject or field `powr`", "power"),
    ("emit_wrong_type", r#""strong""#, "E0621", "do[0].emit.power", "strong", ""),
    ("reason_unknown", r#""wraded""#, "E0626", "do[0].cancel_event.reason", "wraded", ""),
    ("group_unknown", r#""sumer""#, "E0626", "do[0].set_rule_group.group", "sumer", ""),
    // Control flow and variables.
    ("if_alone", r#"{ "if": "true" }"#, "E0682", "do[0]", "`if` needs `then`, `else` or both", ""),
    ("if_extra", r#""do": [] }"#, "E0682", "do[0].do", "`if` has no `do`", ""),
    ("repeat_text", r#""many""#, "E0621", "do[0].repeat", "many", ""),
    ("loop_no_body", r#"{ "while": "true" }"#, "E0682", "do[0]", "`while` needs `do`", ""),
    ("for_each_no_name", r#"{ "for_each": "[1]""#, "E0682", "do[0]", "`for_each` needs `as`", ""),
    ("let_bad_name", r#""Total""#, "E0682", "do[0].let.Total", "cannot be written `$Total`", ""),
    ("let_scoped", "$inner", "E0622", "do[1].note.expr", "unknown local `$inner`", ""),
    ("set_wrong_type", r#""'six'""#, "E0645", "do[0].set.self.var.n", "must be int", ""),
    ("add_to_text", r#""self.var.tag""#, "E0682", "do[0].add.self.var.tag", "adds to a number or appends to a list", ""),
    ("unset_property", r#""self.health""#, "E0682", "do[0].unset[0]", "removes a variable, a local or an entry", ""),
    ("return_wrong_type", r#""'text'""#, "E0645", "do[0].return", "must be int", ""),
    ("timer_no_time", r#""0 s""#, "E0682", "do[0].after", "above zero", ""),
    // The rule itself.
    ("param_type", r#""entty""#, "E0671", "params.who", "is not a type", "entity"),
    ("param_order", r#""late": "int""#, "E0684", "params.late", "parameters that may be left out come last", ""),
    ("param_default", r#""$late""#, "E0622", "params.late.default", "unknown local", ""),
    ("subject_kind", r#""int""#, "E0684", "subjects.self", "is not a kind of subject", ""),
    ("both_triggers", r#""10 s""#, "E0684", "every", "by an event or by an interval, not both", ""),
    ("attached_to_nothing", r#""attached_to_nothing""#, "E0684", "", "an attached rule listens for an event", ""),
];

const BROKEN: &str = r#"[
  { "type": "eoc", "id": "unknown_effect", "do": [ { "notee": "x" } ] },
  { "type": "eoc", "id": "wrong_type", "params": { "who": "entity" },
    "do": [ { "hurt": { "who": "$who", "amount": "'a lot'" } } ] },
  { "type": "eoc", "id": "wrong_literal", "params": { "who": "entity" },
    "do": [ { "hurt": { "who": "$who", "amount": true } } ] },
  { "type": "eoc", "id": "not_settable", "subjects": { "self": "entity" },
    "do": [ { "set": { "self.name": "x" } } ] },

  { "type": "eoc", "id": "unknown_argument", "params": { "who": "entity" },
    "do": [ { "hurt": { "who": "$who", "amount": 1, "knd": "x" } } ] },
  { "type": "eoc", "id": "missing_argument", "params": { "who": "entity" },
    "do": [ { "hurt": { "who": "$who" } } ] },
  { "type": "eoc", "id": "by_name_only", "do": [ { "hurt": 3 } ] },
  { "type": "eoc", "id": "two_keys", "do": [ { "note": "a", "refuse": {} } ] },
  { "type": "eoc", "id": "not_an_object", "do": [ [ "note" ] ] },

  { "type": "eoc", "id": "no_such_rule", "do": [ { "run": "nobody" } ] },
  { "type": "eoc", "id": "unknown_parameter", "params": { "who": "entity" },
    "do": [ { "fx:ignite": { "who": "$who", "whom": "$who" } } ] },
  { "type": "eoc", "id": "missing_parameter", "do": [ { "fx:ignite": { "heat": 2 } } ] },
  { "type": "eoc", "id": "takes_none",
    "do": [ { "run": { "eoc": "fx:plain", "with": { "x": 1 } } } ] },
  { "type": "eoc", "id": "not_a_verb", "do": [ { "fx:plain": {} } ] },
  { "type": "eoc", "id": "peeks", "params": { "who": "entity" },
    "do": [ { "note": { "expr": "str($secret)" } } ] },
  { "type": "eoc", "id": "needs_self", "do": [ { "fx:bump": { "by": 1 } } ] },
  { "type": "eoc", "id": "source_misplaced", "do": [ { "note": { "run": "fx:plain" } } ] },
  { "type": "eoc", "id": "rule_in_expression",
    "do": [ { "note": { "expr": "str(fx:gives(3))" } } ] },

  { "type": "eoc", "id": "no_such_event", "on": "alarm_triped", "do": [] },
  { "type": "eoc", "id": "as_unknown", "on": "alarm_tripped", "attached": true, "as": "whoo", "do": [] },
  { "type": "eoc", "id": "as_unattached", "on": "alarm_tripped", "as": "who", "do": [] },
  { "type": "eoc", "id": "filter_unknown", "on": "alarm_tripped", "filter": { "knd": "glass" }, "do": [] },
  { "type": "eoc", "id": "filter_wrong_type", "on": "alarm_tripped", "filter": { "loud": "yes" }, "do": [] },
  { "type": "eoc", "id": "modify_notification", "on": "alarm_tripped",
    "do": [ { "modify": { "event.energy": 1 } } ] },
  { "type": "eoc", "id": "modify_fixed", "on": "strike_incoming",
    "do": [ { "modify": { "event.kind": "soft" } } ] },
  { "type": "eoc", "id": "modify_a_variable", "on": "strike_incoming", "attached": true,
    "do": [ { "modify": { "self.var.n": 1 } } ] },
  { "type": "eoc", "id": "cancel_notification", "on": "alarm_tripped",
    "do": [ { "cancel_event": {} } ] },
  { "type": "eoc", "id": "consume_nothing", "do": [ { "consume": {} } ] },
  { "type": "eoc", "id": "emit_unknown", "subjects": { "self": "entity" },
    "do": [ { "emit": { "event": "ritual_complete", "caster": "self" } } ] },
  { "type": "eoc", "id": "emit_no_subject",
    "do": [ { "emit": { "event": "ritual_completed", "power": 1 } } ] },
  { "type": "eoc", "id": "emit_unknown_field", "subjects": { "self": "entity" },
    "do": [ { "emit": { "event": "ritual_completed", "caster": "self", "powr": 1 } } ] },
  { "type": "eoc", "id": "emit_wrong_type", "subjects": { "self": "entity" },
    "do": [ { "emit": { "event": "ritual_completed", "caster": "self", "power": "strong" } } ] },
  { "type": "eoc", "id": "reason_unknown", "on": "strike_incoming",
    "do": [ { "cancel_event": { "reason": "wraded" } } ] },
  { "type": "eoc", "id": "group_unknown",
    "do": [ { "set_rule_group": { "group": "sumer", "on": true } } ] },

  { "type": "eoc", "id": "if_alone", "do": [ { "if": "true" } ] },
  { "type": "eoc", "id": "if_extra", "do": [ { "if": "true", "then": [], "do": [] } ] },
  { "type": "eoc", "id": "repeat_text", "do": [ { "repeat": "many", "do": [] } ] },
  { "type": "eoc", "id": "loop_no_body", "do": [ { "while": "true" } ] },
  { "type": "eoc", "id": "for_each_no_name", "do": [ { "for_each": "[1]", "do": [] } ] },
  { "type": "eoc", "id": "let_bad_name", "do": [ { "let": { "Total": 1 } } ] },
  { "type": "eoc", "id": "let_scoped",
    "do": [ { "if": "true", "then": [ { "let": { "inner": 1 } } ] },
            { "note": { "expr": "str($inner)" } } ] },
  { "type": "eoc", "id": "set_wrong_type", "subjects": { "self": "entity" },
    "do": [ { "set": { "self.var.n": "'six'" } } ] },
  { "type": "eoc", "id": "add_to_text", "subjects": { "self": "entity" },
    "do": [ { "add": { "self.var.tag": 1 } } ] },
  { "type": "eoc", "id": "unset_property", "subjects": { "self": "entity" },
    "do": [ { "unset": [ "self.health" ] } ] },
  { "type": "eoc", "id": "return_wrong_type", "returns": "int",
    "do": [ { "return": "'text'" } ] },
  { "type": "eoc", "id": "timer_no_time", "do": [ { "after": "0 s", "do": [] } ] },

  { "type": "eoc", "id": "param_type", "params": { "who": "entty" }, "do": [] },
  { "type": "eoc", "id": "param_order",
    "params": { "early": { "type": "int", "default": 1 }, "late": "int" }, "do": [] },
  { "type": "eoc", "id": "param_default",
    "params": { "late": { "type": "int", "default": "$late" } }, "do": [] },
  { "type": "eoc", "id": "subject_kind", "subjects": { "self": "int" }, "do": [] },
  { "type": "eoc", "id": "both_triggers", "on": "alarm_tripped", "every": "10 s", "do": [] },
  { "type": "eoc", "id": "attached_to_nothing", "attached": true, "do": [] }
]"#;

#[test]
fn every_mistake_in_a_rule_is_a_diagnostic_with_its_place() {
    let outcome = load_written(&["fx"], BROKEN);
    assert!(
        outcome.defs.is_none(),
        "a load with errors gives no definitions"
    );
    let found = outcome.diagnostics.as_slice();
    let mut matched = vec![false; found.len()];
    let mut wrong: Vec<String> = Vec::new();
    for (rule, on_line, code, path, says, suggests) in MISTAKES {
        let id = format!("written:{rule}");
        let at = found.iter().position(|diagnostic| {
            diagnostic.code == *code
                && diagnostic.path == *path
                && diagnostic.message.contains(says)
                && diagnostic
                    .definition
                    .as_ref()
                    .is_some_and(|label| label.id == id)
        });
        let Some(at) = at else {
            let of_rule: Vec<String> = found
                .iter()
                .filter(|diagnostic| {
                    diagnostic
                        .definition
                        .as_ref()
                        .is_some_and(|label| label.id == id)
                })
                .map(|diagnostic| {
                    format!(
                        "{} at `{}`: {}",
                        diagnostic.code, diagnostic.path, diagnostic.message
                    )
                })
                .collect();
            wrong.push(format!(
                "`{rule}`: no {code} at `{path}` saying \"{says}\"; it has: {of_rule:#?}"
            ));
            continue;
        };
        matched[at] = true;
        let diagnostic = &found[at];
        let Some(location) = diagnostic.location.as_ref() else {
            wrong.push(format!("`{rule}`: the diagnostic has no place"));
            continue;
        };
        if !location.file.ends_with("written.json") {
            wrong.push(format!("`{rule}` is reported in {}", location.file));
        }
        if !location.source_line.contains(on_line) {
            wrong.push(format!(
                "`{rule}` is reported at the line `{}`, which lacks `{on_line}`",
                location.source_line.trim()
            ));
        }
        let suggestion = diagnostic.suggestion.as_deref().unwrap_or("");
        if !suggests.is_empty() && !suggestion.contains(suggests) {
            wrong.push(format!(
                "`{rule}` suggests \"{suggestion}\", not `{suggests}`"
            ));
        }
    }
    assert!(wrong.is_empty(), "{}", wrong.join("\n"));
    // Nothing else was said about these rules that is an error.
    let stray: Vec<String> = found
        .iter()
        .zip(&matched)
        .filter(|(diagnostic, matched)| !**matched && diagnostic.is_error())
        .map(|(diagnostic, _)| diagnostic.render())
        .collect();
    assert!(stray.is_empty(), "unexpected errors:\n{}", stray.join("\n"));
}

#[test]
fn a_return_outside_a_rule_is_a_located_error() {
    let data = r#"[
  { "type": "action", "id": "gives_back",
    "phases": [ { "id": "act", "time": 0, "do": [ { "return": 1 } ] } ] }
]"#;
    let outcome = load_written(&["fx"], data);
    let errors: Vec<_> = outcome
        .diagnostics
        .iter()
        .filter(|diagnostic| diagnostic.is_error())
        .collect();
    assert_eq!(errors.len(), 1, "{}", outcome.diagnostics.render());
    let error = errors[0];
    assert_eq!(error.code, "E0683");
    assert_eq!(error.path, "phases.act.do[0].return");
    assert!(error.message.contains("is not in a rule"));
    let location = error.location.as_ref().expect("it has a place");
    assert_eq!((location.line, location.column), (3, 53));
    assert_eq!(
        error.definition.as_ref().map(|label| label.to_string()),
        Some("action written:gives_back".to_owned())
    );
}

#[test]
fn an_effect_list_that_is_not_a_list_and_nesting_too_deep_are_refused() {
    let outcome = load_written(
        &[],
        r#"[ { "type": "eoc", "id": "flat", "do": { "note": "x" } } ]"#,
    );
    let error = &outcome.diagnostics.as_slice()[0];
    assert_eq!((error.code.as_str(), error.path.as_str()), ("E0201", "do"));
    assert!(error.message.contains("expected a list of effects"));

    // Thirty-three lists inside one another: one more than may nest.
    // Each takes two levels of JSON, so this is the deepest a file can
    // hold, and the compiler refuses it before the interpreter could
    // meet it.
    let mut nested = String::from(r#"{ "note": "deep" }"#);
    for _ in 0..30 {
        nested = format!(r#"{{ "repeat": 1, "do": [ {nested} ] }}"#);
    }
    let data = format!(r#"[ {{ "type": "eoc", "id": "deep", "do": [ {nested} ] }} ]"#);
    let defs = clean(load_written(&[], &data));
    let rules = Arc::new(RuleSet::new(defs));
    let mut host = FakeHost::new(rules);
    call(&mut host, "written:deep", &[], &[]);
    assert_eq!(noted(&host), ["note(deep, null)"]);
}

#[test]
fn a_broken_rule_is_inert_in_a_lenient_load() {
    let types = types();
    let data = r#"[
  { "type": "eoc", "id": "broken", "on": "alarm_tripped", "do": [ { "notee": "x" } ] },
  { "type": "eoc", "id": "sound", "on": "alarm_tripped", "do": [ { "note": "heard" } ] }
]"#;
    let outcome = Loader::new(&types)
        .expr_registry(&FakeHost::registry())
        .memory_mod(
            "written",
            r#"{ "id": "written" }"#,
            &[("data/w.json", data)],
        )
        .lenient(true)
        .load_mods(&["written"]);
    assert_eq!(outcome.diagnostics.error_count(), 1);
    let defs = outcome.defs.expect("a lenient load carries on");
    let mut host = FakeHost::new(Arc::new(RuleSet::new(defs)));
    let who = host.entity(1);
    alarm(&mut host, who, true, "x", 1.0);
    assert_eq!(noted(&host), ["note(heard, null)"]);
    assert!(host.faults.is_empty());
}

// ---- the documents state what is built ----------------------------------------

#[test]
fn document_05_states_every_effect_limit_and_reason() {
    let document =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../docs/05-expressions-and-eoc.md");
    let text = std::fs::read_to_string(document).expect("document 05 can be read");
    for effect in CORE_EFFECTS {
        assert!(!effect.doc.is_empty() && !effect.form.is_empty());
        assert!(
            text.contains(&format!("\"{}\"", effect.name))
                || text.contains(&format!("`{}`", effect.name)),
            "05 §10 does not state the effect `{}`",
            effect.name
        );
    }
    for (setting, doc) in LIMIT_SETTINGS {
        assert!(!doc.is_empty());
        assert!(
            text.contains(&format!("`{setting}`")),
            "05 does not list `{setting}`"
        );
    }
    for info in RULE_REASONS {
        assert!(
            text.contains(&format!("`{}`", info.code)),
            "05 §13.2 does not list the reason `{}`",
            info.code
        );
    }
    // Every field of `eoc`, `rule_group` and `reason`.
    for field in [
        "on",
        "attached",
        "as",
        "filter",
        "every",
        "while_unloaded",
        "if",
        "do",
        "else",
        "params",
        "subjects",
        "returns",
        "priority",
        "once",
        "cooldown",
        "skip_if_consumed",
        "ui_callable",
        "group",
        "doc",
        "enabled",
    ] {
        assert!(
            text.contains(&format!("| `{field}` |")),
            "05 §9.1 has no row for `{field}`"
        );
    }
}

#[test]
fn every_core_effect_compiles() {
    // One rule that writes every effect of the table, so that none is
    // listed and not built.
    let data = r#"[
  { "type": "rule_group", "id": "g" },
  { "type": "event", "id": "ping", "subjects": { "who": "entity" }, "hook": true, "fields": { "n": "int" }, "mutable": ["n"] },
  { "type": "eoc", "id": "helper", "attached": true, "on": "ping", "do": [] },
  { "type": "eoc", "id": "all", "on": "ping",
    "do": [
      { "if": "true", "then": [] },
      { "first": [ { "do": [] } ] },
      { "switch": "event.n", "cases": { "1": [] } },
      { "for_each": "[1]", "as": "x", "do": [] },
      { "repeat": 1, "do": [] },
      { "while": "false", "do": [] },
      { "weighted": [ { "weight": 1, "do": [] } ] },
      { "as": { "self": "who" }, "do": [] },
      { "let": { "x": 1 } },
      { "set": { "$x": 2 } },
      { "add": { "$x": 1 } },
      { "unset": [ "$x" ] },
      { "run": "helper" },
      { "queue": "helper" },
      { "attach_rule": { "to": "who", "eoc": "helper" } },
      { "detach_rule": { "to": "who", "eoc": "helper" } },
      { "set_rule_group": { "group": "g", "on": true } },
      { "emit": { "event": "ping", "who": "who", "n": 1 } },
      { "modify": { "event.n": 2 } },
      { "cancel_event": {} },
      { "consume": {} },
      { "after": "1 s", "do": [] },
      { "every": "1 s", "do": [] },
      { "cancel_timer": "t" },
      { "message": { "to": "who", "text": "{name(who)} is pinged." } },
      { "ask": { "who": "who", "prompt": "Answer?", "options": [ { "label": "Yes", "do": [] } ] } },
      { "return": null },
      { "stop": {} }
    ] }
]"#;
    let defs = clean(load_written(&[], data));
    let all = defs.get::<EocDef>(defs.lookup::<EocDef>("written:all").unwrap());
    let block = all.effects.block().unwrap();
    assert_eq!(block.len(), CORE_EFFECTS.len());
    for effect in CORE_EFFECTS {
        assert!(
            data.contains(&format!("{{ \"{}\":", effect.name)),
            "`{}` is not written",
            effect.name
        );
    }
}

// ---- 05 §19: what a run costs --------------------------------------------------

/// Not a check: prints what delivering an event and running a rule cost,
/// for the budgets of 05 §19. Run with
/// `cargo test -p clash-sim --release --test rules -- --ignored --nocapture time_per_run`.
#[test]
#[ignore = "a measurement, not a check"]
#[allow(
    clippy::disallowed_types,
    clippy::disallowed_methods,
    reason = "a measurement needs a clock; this test is not part of the simulation and nothing \
              in the crate reads one"
)]
fn time_per_run() {
    let data = r#"[
  { "type": "variable", "id": "seen", "scope": "entity", "value": "int", "default": 0 },
  { "type": "eoc", "id": "marks", "on": "alarm_tripped",
    "filter": { "kind": "glass" },
    "if": "event.loud",
    "do": [ { "set": { "who.var.seen": 1 } } ] },
  { "type": "eoc", "id": "counts", "params": { "n": "int" }, "returns": "int",
    "do": [ { "let": { "sum": 0 } },
            { "repeat": "$n", "do": [ { "add": { "$sum": "$i" } } ] },
            { "return": "$sum" } ] }
]"#;
    let defs = clean(load_written(&[], data));
    let rules = Arc::new(RuleSet::new(defs));
    let mut host = FakeHost::new(rules.clone());
    let who = Value::Subject(host.entity(1));
    let here = Value::Subject(host.place(1));
    let rounds = 1_000_000_u32;
    let per =
        |elapsed: std::time::Duration, rounds: u32| elapsed.as_nanos() as f64 / f64::from(rounds);

    // An event that nothing listens for.
    let quiet = host.event("sim_started");
    let start = std::time::Instant::now();
    for _ in 0..rounds {
        let mut occurrence = Occurrence::new(quiet, Vec::new(), Vec::new());
        rules.deliver(&mut host, &mut occurrence);
    }
    println!(
        "an event with no handlers: {:.0} ns",
        per(start.elapsed(), rounds)
    );

    // An event whose one rule is filtered out by the index.
    let tripped = host.event("alarm_tripped");
    let fields = |kind: &str| vec![Value::Bool(true), Value::from(kind), real(1.0)];
    let subjects = vec![here, who];
    let start = std::time::Instant::now();
    for _ in 0..rounds {
        let mut occurrence = Occurrence::new(tripped, subjects.clone(), fields("wood"));
        rules.deliver(&mut host, &mut occurrence);
    }
    println!(
        "an event a filter turns away: {:.0} ns",
        per(start.elapsed(), rounds)
    );

    // An event one rule answers: bindings, filter, condition, one `set`.
    let start = std::time::Instant::now();
    for _ in 0..rounds {
        let mut occurrence = Occurrence::new(tripped, subjects.clone(), fields("glass"));
        rules.deliver(&mut host, &mut occurrence);
    }
    println!(
        "an event one rule answers with one `set`: {:.0} ns",
        per(start.elapsed(), rounds)
    );

    // A call that runs a loop of a thousand passes, each one `add`.
    let counts = rules.lookup("written:counts").unwrap();
    let calls = 2_000_u32;
    let start = std::time::Instant::now();
    for _ in 0..calls {
        let value = rules
            .call(&mut host, counts, &[], &[("n", Value::Int(1_000))])
            .unwrap();
        assert_eq!(value, Value::Int(499_500));
    }
    println!(
        "one pass of a loop with one `add`: {:.0} ns",
        per(start.elapsed(), calls * 1_000)
    );
    assert!(host.faults.is_empty());
}

// ---- 05 §9.3: rules a definition carries --------------------------------------

clash_core::define_id! {
    /// A definition that carries rules, for the test below.
    struct CarrierId;
}

/// Stands for any definition whose instances are subjects: an area
/// kind, a template, a trait.
#[derive(Debug, serde::Deserialize)]
struct CarrierDef {
    #[serde(default)]
    rules: Rules,
}

impl clash_data::Def for CarrierDef {
    type Id = CarrierId;
    const TYPE: &'static str = "carrier";
    const DOC: &'static str = "A definition that carries rules.";
}

#[test]
fn a_definition_carries_rules_by_id_and_written_in_place() {
    let data = r#"[
  { "type": "variable", "id": "seen", "scope": "entity", "value": "int", "default": 0 },
  { "type": "carrier", "id": "watchful",
    "rules": [
      { "on": "alarm_tripped", "as": "who",
        "do": [ { "add": { "self.var.seen": 1 } } ] },
      "named"
    ] },
  { "type": "eoc", "id": "named", "on": "alarm_tripped", "attached": true, "as": "who",
    "do": [ { "add": { "self.var.seen": 10 } } ] }
]"#;
    let mut types = types();
    types.register::<CarrierDef>();
    let outcome = Loader::new(&types)
        .expr_registry(&FakeHost::registry())
        .memory_mod(
            "written",
            r#"{ "id": "written" }"#,
            &[("data/w.json", data)],
        )
        .load_mods(&["written"]);
    let defs = clean(outcome);
    let carrier = defs.get::<CarrierDef>(defs.lookup::<CarrierDef>("written:watchful").unwrap());
    let carried: Vec<_> = carrier.rules.iter().map(|rule| rule.id()).collect();
    assert_eq!(carried.len(), 2);

    // A rule written in place is attached without saying so, binds its
    // owner as `self`, and has no name to be called by.
    let rules = defs.registry::<EocDef>();
    let inline = rules.get(carried[0]);
    assert!(rules.name(carried[0]).contains('#'));
    assert!(inline.is_attached() && !inline.is_global());
    assert_eq!(inline.role(), 1);
    assert!(inline.subject_names().iter().any(|name| &**name == "self"));
    assert_eq!(rules.name(carried[1]), "written:named");

    // The host answers `rules_of` from what the definition carries, and
    // both run for an instance that takes part in the event.
    let mut host = FakeHost::new(Arc::new(RuleSet::new(defs.clone())));
    let (one, two) = (host.entity(1), host.entity(2));
    for rule in &carried {
        host.carry(one, *rule);
    }
    alarm(&mut host, one, true, "x", 1.0);
    alarm(&mut host, two, true, "x", 1.0);
    assert_eq!(host.var(one, "written:seen"), Value::Int(11));
    assert_eq!(host.var(two, "written:seen"), Value::Int(0));
    assert!(host.faults.is_empty());
}
