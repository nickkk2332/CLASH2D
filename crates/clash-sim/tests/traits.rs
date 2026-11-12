//! Tests of traits (06 §5; package E3), against the real simulation: the
//! world content of `mods/core`, its bundled traits
//! (`mods/core/data/traits.json`), the bodies and items of
//! `tests/fixtures/item`, and a few traits of the tests' own.
//!
//! The kernel does not yet give creatures stats, so [`person`] attaches
//! them over `Sim::components_mut` with the stats package's own
//! functions, as the items tests do.

mod common;

use std::path::{Path, PathBuf};
use std::sync::{Arc, LazyLock};

use clash_core::{Dur, EntityId, Real, Rng, Tick};
use clash_data::{Defs, Loader, Runner};
use clash_expr::{FunctionId, Handle, Host, PropertyId, RuntimeWarning, Value};
use clash_sim::body::{self, BodyCx, BodyDefs};
use clash_sim::component::Containment;
use clash_sim::item::{self, ItemDef};
use clash_sim::movement::PostureDef;
use clash_sim::rule::{EocDef, RunView, VerbCall};
use clash_sim::stats::{self, SourceKind, StatDefs, StatsCx};
use clash_sim::traits::{
    self, Giver, GiverKind, Timer, TraitDef, TraitId, Traits, add_trait, remove_trait, withdraw,
};
use clash_sim::{AdvanceLimit, Delivery, Handler, Place, Reason, Sim, SimEvent, Subject};
use clash_time::{ClockId, Timeline};
use clash_world::Cause;
use common::{creature, on, save_and_load, sim_over, spawn};
use proptest::prelude::*;

// ---- Loading ----------------------------------------------------------------

fn fixtures(area: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join(format!("tests/fixtures/{area}"))
}

/// Traits, a posture and an item of the tests' own.
const TEST_DEFS: &str = r#"[
  { "type": "trait", "id": "hardy",
    "modifiers": [ { "attr": "quickness", "add": 0.5 } ], "tags": ["hardy"] },
  { "type": "trait", "id": "lingering", "duration": "30 s", "while_unloaded": "elapse",
    "tags": ["lingering"] },
  { "type": "trait", "id": "fleeting", "duration": "10 s", "while_unloaded": "elapse" },
  { "type": "trait", "id": "patient", "duration": "30 s" },
  { "type": "trait", "id": "armed_focus", "while": "self.has_tag('armed')",
    "modifiers": [ { "attr": "quickness", "mul": 2 } ], "tags": ["focused"] },
  { "type": "trait", "id": "steadied",
    "modifiers": [ { "attr": "quickness", "add": -0.25 } ], "tags": ["steadied"] },
  { "type": "posture", "id": "braced", "height": 0.8, "traits": ["steadied"] },
  { "type": "item", "id": "charm", "material": "steel", "mass": "0.1 kg", "length": "0.05 m",
    "grip": { "itemfix:arm": 1 }, "traits": ["hardy"], "holder_tags": ["hardy"] }
]"#;

/// The world files of `mods/core`, its bundled traits, and the tests'
/// own definitions, as paths within the mod and their text.
fn core_files() -> Vec<(String, String)> {
    let dir = common::mods_dir().join("core/data/world");
    let mut files: Vec<PathBuf> = std::fs::read_dir(&dir)
        .unwrap()
        .filter_map(|entry| entry.ok().map(|entry| entry.path()))
        .filter(|path| path.extension().is_some_and(|ext| ext == "json"))
        .collect();
    files.sort();
    let mut out: Vec<(String, String)> = files
        .iter()
        .map(|file| {
            let name = file.file_name().unwrap().to_string_lossy();
            (
                format!("data/world/{name}"),
                std::fs::read_to_string(file).unwrap(),
            )
        })
        .collect();
    let bundled = common::mods_dir().join("core/data/traits.json");
    out.push((
        "data/traits.json".to_owned(),
        std::fs::read_to_string(bundled).unwrap(),
    ));
    out.push(("data/test/traits.json".to_owned(), TEST_DEFS.to_owned()));
    out
}

static DEFS: LazyLock<Arc<Defs>> = LazyLock::new(|| {
    let types = common::types();
    let files = core_files();
    let files: Vec<(&str, &str)> = files
        .iter()
        .map(|(path, text)| (path.as_str(), text.as_str()))
        .collect();
    let outcome = Loader::new(&types)
        .expr_registry(&clash_sim::engine_registry())
        .memory_mod("core", r#"{ "id": "core" }"#, &files)
        .search_path(fixtures("item"))
        .load_mods(&["core", "itemfix"]);
    assert_eq!(outcome.diagnostics.render(), "");
    outcome.defs.expect("the definitions load")
});

struct Tables {
    body: BodyDefs,
    stats: StatDefs,
}

static TABLES: LazyLock<Tables> = LazyLock::new(|| {
    let world = clash_world::WorldDefs::from_defs(&DEFS).unwrap();
    Tables {
        body: BodyDefs::new(&DEFS, &world).unwrap(),
        stats: StatDefs::new(&DEFS).unwrap(),
    }
});

fn sim() -> Sim {
    sim_over(Arc::clone(&DEFS), 5)
}

fn trait_id(name: &str) -> TraitId {
    DEFS.lookup::<TraitDef>(&format!("core:{name}"))
        .unwrap_or_else(|| panic!("no trait `{name}`"))
}

fn attr(name: &str) -> stats::AttributeId {
    TABLES.stats.attribute_id(&format!("core:{name}")).unwrap()
}

fn pool(name: &str) -> stats::PoolId {
    TABLES.stats.pool_id(&format!("core:{name}")).unwrap()
}

// ---- Making people ----------------------------------------------------------

struct Apart;

impl Containment for Apart {
    fn container(&self, _id: EntityId) -> Option<EntityId> {
        None
    }

    fn contents(&self, _id: EntityId, _visit: &mut dyn FnMut(EntityId)) {}
}

struct Quiet;

impl Host for Quiet {
    fn subject(&mut self, _slot: u16) -> Value {
        Value::Null
    }

    fn local(&mut self, _slot: u16) -> Value {
        Value::Null
    }

    fn event_field(&mut self, _slot: u16) -> Value {
        Value::Null
    }

    fn property(&mut self, _property: PropertyId, _subject: Handle, _args: &[Value]) -> Value {
        Value::Null
    }

    fn variable(&mut self, _subject: Handle, _name: &str) -> Value {
        Value::Null
    }

    fn call(&mut self, _function: FunctionId, _args: &[Value]) -> Value {
        Value::Null
    }

    fn rng(&mut self) -> Option<&mut Rng> {
        None
    }

    fn warn(&mut self, _warning: RuntimeWarning) {}
}

/// A person of the fixtures' humanoid plan, with stats and the bundled
/// pools, standing at a cell. Stats and the body are attached on a
/// timeline of their own: the pools start full and schedule nothing.
fn person(sim: &mut Sim, x: i32, y: i32) -> EntityId {
    let id = spawn(sim, creature(), on(x, y));
    let plan = TABLES.body.plan_id("itemfix:humanoid").unwrap();
    let mut timeline = Timeline::new();
    let mut runner = Runner::new();
    let mut quiet = Quiet;
    let components = sim.components_mut();
    let mut cx = BodyCx {
        bodies: &mut components.body,
        defs: &TABLES.body,
        physical: &mut components.physical,
        containment: &Apart,
        stats: StatsCx {
            stats: &mut components.stats,
            defs: &TABLES.stats,
            timeline: &mut timeline,
            host: &mut quiet,
            runner: &mut runner,
        },
    };
    stats::attach(&mut cx.stats, id, ClockId::UNIVERSAL).unwrap();
    stats::add_pool(&mut cx.stats, id, pool("stamina")).unwrap();
    stats::add_pool(&mut cx.stats, id, pool("blood")).unwrap();
    body::attach(&mut cx, id, plan, Real::ONE, ClockId::UNIVERSAL).unwrap();
    id
}

// ---- Reading ----------------------------------------------------------------

fn quickness(sim: &Sim, who: EntityId) -> f64 {
    sim.components()
        .stats
        .get(who)
        .and_then(|stats| stats.attr(attr("quickness")))
        .unwrap()
        .get()
}

fn rate(sim: &Sim, who: EntityId, name: &str) -> f64 {
    sim.components()
        .stats
        .get(who)
        .and_then(|stats| stats.pool_rate(pool(name)))
        .unwrap()
        .get()
}

fn has_tag(sim: &Sim, who: EntityId, tag: &str) -> bool {
    sim.truth().entity(who).unwrap().tags.contains(tag)
}

fn tags(sim: &Sim, who: EntityId) -> Vec<String> {
    let mut tags: Vec<String> = sim
        .truth()
        .entity(who)
        .unwrap()
        .tags
        .iter()
        .map(|tag| tag.to_string())
        .collect();
    tags.sort();
    tags
}

fn traits_of(sim: &Sim, who: EntityId) -> Option<&Traits> {
    sim.components().traits.get(who)
}

fn stacks(sim: &Sim, who: EntityId, name: &str) -> u32 {
    traits_of(sim, who).map_or(0, |traits| traits.stacks(trait_id(name)))
}

fn has(sim: &Sim, who: EntityId, name: &str) -> bool {
    traits_of(sim, who).is_some_and(|traits| traits.has(trait_id(name)))
}

fn events(sim: &mut Sim) -> Vec<SimEvent> {
    sim.pump();
    sim.drain_events()
}

fn named<'a>(events: &'a [SimEvent], name: &str) -> Vec<&'a SimEvent> {
    events.iter().filter(|event| event.name() == name).collect()
}

fn text_field(event: &SimEvent, name: &str) -> String {
    event
        .body
        .field(name)
        .and_then(|value| {
            value
                .plain()
                .and_then(|plain| plain.as_str().map(str::to_owned))
        })
        .unwrap_or_default()
}

fn int_field(event: &SimEvent, name: &str) -> i64 {
    event
        .body
        .field(name)
        .and_then(|value| value.plain().and_then(clash_core::Value::as_int))
        .unwrap()
}

fn prop(sim: &Sim, id: EntityId, name: &str, args: &[Value]) -> Value {
    sim.property_named(Subject::Entity(id), name, args)
}

fn close(a: f64, b: f64) -> bool {
    (a - b).abs() <= 1e-9 * a.abs().max(b.abs()).max(1.0)
}

fn give(sim: &mut Sim, who: EntityId, name: &str) -> traits::Added {
    add_trait(sim, who, trait_id(name), Giver::RULE, None, Cause::DEBUG).unwrap()
}

fn take(sim: &mut Sim, who: EntityId, name: &str) -> u32 {
    remove_trait(sim, who, trait_id(name), None, Cause::DEBUG).unwrap()
}

fn advance_to(sim: &mut Sim, seconds: i64) {
    sim.advance(AdvanceLimit::Until(Tick::ZERO + Dur::from_secs(seconds)));
}

fn handle(sim: &Sim, who: EntityId) -> Handle {
    match sim.subject_value(Subject::Entity(who)) {
        Value::Subject(handle) => handle,
        other => panic!("not a subject: {other:?}"),
    }
}

/// Calls a verb of the engine's table as the rule runtime will.
fn call(sim: &mut Sim, name: &str, args: &[Value]) -> Result<Value, Reason> {
    let registry = clash_sim::engine_registry();
    let verb = registry.verb_id(name).unwrap();
    let def = clash_sim::verb::engine_verbs()
        .into_iter()
        .find(|def| def.name == name)
        .unwrap();
    let view = RunView::new(&[], &[]);
    let call = VerbCall {
        verb,
        args,
        by: view,
    };
    (def.run)(sim, &call, Cause::DEBUG)
}

// ---- What a trait confers ---------------------------------------------------

#[test]
fn a_stimulant_confers_while_it_lasts_and_takes_back_exactly_what_it_added() {
    let mut sim = sim();
    let who = person(&mut sim, 3, 3);
    let before = (
        quickness(&sim, who),
        rate(&sim, who, "stamina"),
        tags(&sim, who),
    );
    events(&mut sim);

    let added = give(&mut sim, who, "stimulated");
    assert!(added.first && added.changed);
    assert_eq!(added.stacks, 1);
    assert!(close(quickness(&sim, who), 1.3));
    assert!(close(rate(&sim, who, "stamina"), 1.0 - 0.5));
    assert!(has_tag(&sim, who, "stimulated"));
    let seen = events(&mut sim);
    let added = named(&seen, "trait_added");
    assert_eq!(added.len(), 1);
    assert_eq!(text_field(added[0], "trait"), "core:stimulated");
    assert_eq!(text_field(added[0], "giver"), "rule");
    assert_eq!(text_field(added[0], "look"), "stimulated");

    assert_eq!(take(&mut sim, who, "stimulated"), 1);
    assert_eq!(quickness(&sim, who).to_bits(), before.0.to_bits());
    assert_eq!(rate(&sim, who, "stamina").to_bits(), before.1.to_bits());
    assert_eq!(tags(&sim, who), before.2);
    let seen = events(&mut sim);
    let removed = named(&seen, "trait_removed");
    assert_eq!(removed.len(), 1);
    assert_eq!(text_field(removed[0], "how"), "removed");
    assert_eq!(int_field(removed[0], "left"), 0);
    // Removing what it does not have changes nothing.
    assert_eq!(take(&mut sim, who, "stimulated"), 0);
}

#[test]
fn a_tag_the_entity_had_of_its_own_is_not_taken_with_the_trait() {
    let mut sim = sim();
    let who = person(&mut sim, 3, 3);
    sim.entity_mut(who).unwrap().tags.insert("hardy".to_owned());
    give(&mut sim, who, "hardy");
    assert!(traits_of(&sim, who).unwrap().conferred_tags().is_empty());
    take(&mut sim, who, "hardy");
    assert!(has_tag(&sim, who, "hardy"), "the tag was the entity's own");
}

#[test]
fn refreshing_starts_the_duration_again_and_stacking_adds_stacks() {
    let mut sim = sim();
    let who = person(&mut sim, 3, 3);
    give(&mut sim, who, "stimulated");
    advance_to(&mut sim, 20);
    let left = |sim: &Sim| match prop(sim, who, "trait_left", &[Value::text("core:stimulated")]) {
        Value::Real(left) => left.get(),
        other => panic!("{other:?}"),
    };
    assert!(close(left(&sim), 10.0));
    let again = give(&mut sim, who, "stimulated");
    assert!(!again.first && again.changed);
    assert_eq!(again.stacks, 1);
    assert!(close(left(&sim), 30.0));
    assert!(close(quickness(&sim, who), 1.3), "refreshing adds nothing");

    for expected in 1..=5 {
        let added = give(&mut sim, who, "bleeding");
        assert_eq!(added.stacks, expected);
    }
    // The fifth is the most.
    assert_eq!(give(&mut sim, who, "bleeding").stacks, 5);
    assert!(close(rate(&sim, who, "blood"), -0.05));
    assert_eq!(
        prop(&sim, who, "trait_stacks", &[Value::text("core:bleeding")]),
        Value::Int(5)
    );
    assert_eq!(
        remove_trait(&mut sim, who, trait_id("bleeding"), Some(2), Cause::DEBUG),
        Ok(2)
    );
    assert_eq!(stacks(&sim, who, "bleeding"), 3);
    assert!(close(rate(&sim, who, "blood"), -0.03));
    let seen = events(&mut sim);
    let removed = named(&seen, "trait_removed");
    assert_eq!(int_field(removed.last().unwrap(), "left"), 3);
}

#[test]
fn a_trait_that_does_not_stack_given_again_changes_nothing() {
    let mut sim = sim();
    let who = person(&mut sim, 3, 3);
    give(&mut sim, who, "hardy");
    events(&mut sim);
    let again = give(&mut sim, who, "hardy");
    assert!(!again.changed);
    assert!(events(&mut sim).is_empty(), "nothing is raised");
    assert!(close(quickness(&sim, who), 1.5));
    // Another giver gives it again: a second instance, which confers too.
    add_trait(
        &mut sim,
        who,
        trait_id("hardy"),
        Giver::new(GiverKind::Template, 1),
        None,
        Cause::DEBUG,
    )
    .unwrap();
    assert!(close(quickness(&sim, who), 2.0));
    assert_eq!(stacks(&sim, who, "hardy"), 2);
    // `remove_trait` takes it from every giver.
    assert_eq!(take(&mut sim, who, "hardy"), 2);
    assert!(close(quickness(&sim, who), 1.0));
}

#[test]
fn traits_of_a_group_exclude_each_other_and_rules_come_and_go_with_them() {
    let mut sim = sim();
    let who = person(&mut sim, 3, 3);
    let me = handle(&sim, who);
    give(&mut sim, who, "guarding");
    assert!(has_tag(&sim, who, "guarding"));
    let rules = sim.rules.attached_to(me).to_vec();
    assert_eq!(rules.len(), 1, "the stance's rule is attached");
    let guard_rule = DEFS
        .try_registry::<EocDef>()
        .unwrap()
        .try_get(rules[0])
        .unwrap();
    assert!(guard_rule.is_attached());
    events(&mut sim);

    give(&mut sim, who, "lunging");
    assert!(!has(&sim, who, "guarding"));
    assert!(!has_tag(&sim, who, "guarding"));
    assert!(has_tag(&sim, who, "lunging"));
    assert!(
        sim.rules.attached_to(me).is_empty(),
        "the rule went with it"
    );
    assert!(close(quickness(&sim, who), 1.2));
    let seen = events(&mut sim);
    let removed = named(&seen, "trait_removed");
    assert_eq!(removed.len(), 1);
    assert_eq!(text_field(removed[0], "how"), "replaced");
    assert_eq!(text_field(removed[0], "trait"), "core:guarding");
}

#[test]
fn a_rule_attached_by_content_is_left_when_the_trait_goes() {
    let mut sim = sim();
    let who = person(&mut sim, 3, 3);
    let me = handle(&sim, who);
    let rule = DEFS
        .try_registry::<TraitDef>()
        .unwrap()
        .get(trait_id("guarding"))
        .rules[0]
        .id();
    assert!(sim.rules.attach(me, rule));
    give(&mut sim, who, "guarding");
    take(&mut sim, who, "guarding");
    assert_eq!(sim.rules.attached_to(me), [rule]);
}

#[test]
fn while_makes_a_trait_dormant_and_dormant_traits_confer_nothing() {
    let mut sim = sim();
    let who = person(&mut sim, 3, 3);
    give(&mut sim, who, "armed_focus");
    assert!(has(&sim, who, "armed_focus"));
    assert_eq!(
        prop(
            &sim,
            who,
            "trait_active",
            &[Value::text("core:armed_focus")]
        ),
        Value::Bool(false)
    );
    assert!(close(quickness(&sim, who), 1.0));
    assert!(!has_tag(&sim, who, "focused"));

    sim.entity_mut(who).unwrap().tags.insert("armed".to_owned());
    traits::reconsider(&mut sim, who);
    assert!(close(quickness(&sim, who), 2.0));
    assert!(has_tag(&sim, who, "focused"));

    sim.entity_mut(who).unwrap().tags.remove("armed");
    traits::reconsider(&mut sim, who);
    assert!(close(quickness(&sim, who), 1.0));
    assert!(!has_tag(&sim, who, "focused"));
}

// ---- Durations --------------------------------------------------------------

#[test]
fn a_duration_runs_out_on_the_holders_own_clock() {
    let mut sim = sim();
    let who = person(&mut sim, 3, 3);
    let slow = person(&mut sim, 5, 5);
    sim.set_time_rate(slow, Some(Real::of(0.5)), Cause::DEBUG)
        .unwrap();
    give(&mut sim, who, "stimulated");
    give(&mut sim, slow, "stimulated");
    events(&mut sim);

    advance_to(&mut sim, 29);
    assert!(has(&sim, who, "stimulated"));
    advance_to(&mut sim, 30);
    assert!(!has(&sim, who, "stimulated"));
    assert!(close(quickness(&sim, who), 1.0));
    let seen = events(&mut sim);
    let expired = named(&seen, "trait_expired");
    assert_eq!(expired.len(), 1);
    let removed = named(&seen, "trait_removed");
    assert_eq!(text_field(removed[0], "how"), "expired");
    let at = |name: &str| seen.iter().position(|event| event.name() == name).unwrap();
    assert!(at("trait_expired") < at("trait_removed"));

    // Half as fast: thirty seconds of its own time are a minute.
    assert!(has(&sim, slow, "stimulated"));
    advance_to(&mut sim, 59);
    assert!(has(&sim, slow, "stimulated"));
    advance_to(&mut sim, 60);
    assert!(!has(&sim, slow, "stimulated"));
}

#[test]
fn a_paused_duration_stands_still_while_its_holder_is_away() {
    let mut sim = sim();
    let who = person(&mut sim, 3, 3);
    give(&mut sim, who, "patient");
    advance_to(&mut sim, 10);
    sim.relocate(who, Place::Stored, Cause::DEBUG).unwrap();
    let held = traits_of(&sim, who).unwrap().instances()[0].timer();
    assert_eq!(held, Timer::Held(Dur::from_secs(20)));
    assert!(sim.truth().wakes_of(who).is_empty(), "nothing waits for it");
    advance_to(&mut sim, 500);
    assert!(has(&sim, who, "patient"));
    sim.relocate(who, on(3, 3), Cause::DEBUG).unwrap();
    advance_to(&mut sim, 519);
    assert!(has(&sim, who, "patient"));
    advance_to(&mut sim, 520);
    assert!(!has(&sim, who, "patient"));
}

#[test]
fn an_elapsing_duration_counts_the_time_away_and_ends_at_once_if_it_is_over() {
    let mut sim = sim();
    let who = person(&mut sim, 3, 3);
    give(&mut sim, who, "lingering"); // 30 s
    give(&mut sim, who, "fleeting"); // 10 s
    give(&mut sim, who, "patient"); // 30 s, paused
    advance_to(&mut sim, 5);
    sim.relocate(who, Place::Stored, Cause::DEBUG).unwrap();
    advance_to(&mut sim, 20);
    events(&mut sim);
    sim.relocate(who, on(3, 3), Cause::DEBUG).unwrap();
    // Fifteen seconds away: `fleeting` had five left and is over;
    // `lingering` has ten left; `patient` still twenty-five.
    assert!(!has(&sim, who, "fleeting"));
    assert!(has(&sim, who, "lingering"));
    let seen = events(&mut sim);
    let expired: Vec<String> = named(&seen, "trait_expired")
        .iter()
        .map(|event| text_field(event, "trait"))
        .collect();
    assert_eq!(expired, ["core:fleeting"]);
    advance_to(&mut sim, 29);
    assert!(has(&sim, who, "lingering"));
    advance_to(&mut sim, 30);
    assert!(!has(&sim, who, "lingering"));
    advance_to(&mut sim, 44);
    assert!(has(&sim, who, "patient"));
    advance_to(&mut sim, 45);
    assert!(!has(&sim, who, "patient"));
}

#[test]
fn several_overdue_durations_end_in_the_order_they_ran_out() {
    let mut sim = sim();
    let who = person(&mut sim, 3, 3);
    give(&mut sim, who, "lingering"); // 30 s
    give(&mut sim, who, "fleeting"); // 10 s
    sim.relocate(who, Place::Stored, Cause::DEBUG).unwrap();
    advance_to(&mut sim, 100);
    events(&mut sim);
    sim.relocate(who, on(3, 3), Cause::DEBUG).unwrap();
    let seen = events(&mut sim);
    let expired: Vec<String> = named(&seen, "trait_expired")
        .iter()
        .map(|event| text_field(event, "trait"))
        .collect();
    assert_eq!(expired, ["core:fleeting", "core:lingering"]);
}

#[test]
fn a_trait_given_out_of_the_world_waits_with_all_its_time() {
    let mut sim = sim();
    let who = person(&mut sim, 3, 3);
    sim.relocate(who, Place::Stored, Cause::DEBUG).unwrap();
    give(&mut sim, who, "lingering");
    advance_to(&mut sim, 100);
    sim.relocate(who, on(3, 3), Cause::DEBUG).unwrap();
    assert!(
        has(&sim, who, "lingering"),
        "no time passed for it in the world"
    );
    advance_to(&mut sim, 130);
    assert!(!has(&sim, who, "lingering"));
}

#[test]
fn a_running_duration_survives_save_and_load() {
    let mut sim = sim();
    let who = person(&mut sim, 3, 3);
    give(&mut sim, who, "stimulated");
    give(&mut sim, who, "guarding");
    advance_to(&mut sim, 12);
    events(&mut sim);
    let (mut loaded, unplaced) = save_and_load(&sim);
    assert!(unplaced.is_empty());
    assert_eq!(loaded.state_hash(), sim.state_hash());
    for run in [&mut sim, &mut loaded] {
        advance_to(run, 29);
        assert!(has(run, who, "stimulated"));
        advance_to(run, 30);
        assert!(!has(run, who, "stimulated"));
    }
    assert_eq!(loaded.state_hash(), sim.state_hash());
}

// ---- Hooks ------------------------------------------------------------------

#[test]
fn the_hook_can_refuse_a_trait_or_shorten_it() {
    let mut sim = sim();
    let who = person(&mut sim, 3, 3);
    sim.on(
        "trait_adding",
        Handler::new(|_sim: &mut Sim, delivery: &mut Delivery<'_>| {
            let name = delivery
                .field("trait")
                .and_then(|value| {
                    value
                        .plain()
                        .and_then(|plain| plain.as_str().map(str::to_owned))
                })
                .unwrap_or_default();
            if name == "core:guarding" {
                let _ = delivery.cancel(Reason::new("test.no_guard", "no guard here"));
            } else if name == "core:stimulated" {
                let _ = delivery.set_field("duration", Real::of(5.0));
            }
        }),
    )
    .unwrap();
    let refused = add_trait(
        &mut sim,
        who,
        trait_id("guarding"),
        Giver::RULE,
        None,
        Cause::DEBUG,
    );
    assert_eq!(refused.unwrap_err().code, "test.no_guard");
    assert!(!has(&sim, who, "guarding"));
    assert!(!has_tag(&sim, who, "guarding"));

    give(&mut sim, who, "stimulated");
    advance_to(&mut sim, 5);
    assert!(!has(&sim, who, "stimulated"), "the handler shortened it");
}

#[test]
fn the_removing_hook_can_keep_a_trait() {
    let mut sim = sim();
    let who = person(&mut sim, 3, 3);
    give(&mut sim, who, "hardy");
    sim.on(
        "trait_removing",
        Handler::new(|_sim: &mut Sim, delivery: &mut Delivery<'_>| {
            let _ = delivery.cancel(Reason::new("test.cursed", "it will not come off"));
        }),
    )
    .unwrap();
    let kept = remove_trait(&mut sim, who, trait_id("hardy"), None, Cause::DEBUG);
    assert_eq!(kept.unwrap_err().code, "test.cursed");
    assert!(has(&sim, who, "hardy"));
}

// ---- Refusals, verbs and properties -----------------------------------------

#[test]
fn refusals_carry_reasons() {
    let mut sim = sim();
    let who = person(&mut sim, 3, 3);
    let refused = add_trait(
        &mut sim,
        who,
        trait_id("hardy"),
        Giver::RULE,
        Some(Dur::ZERO),
        Cause::DEBUG,
    );
    assert_eq!(refused.unwrap_err().code, "trait.bad_duration");
    let refused = remove_trait(&mut sim, who, trait_id("hardy"), Some(0), Cause::DEBUG);
    assert_eq!(refused.unwrap_err().code, "trait.bad_stacks");
    let gone = EntityId::new(999, 0);
    let refused = add_trait(
        &mut sim,
        gone,
        trait_id("hardy"),
        Giver::RULE,
        None,
        Cause::DEBUG,
    );
    assert_eq!(refused.unwrap_err().code, "entity.unknown");
    let me = sim.subject_value(Subject::Entity(who));
    let refused = call(
        &mut sim,
        "add_trait",
        &[me, Value::text("core:no_such_trait"), Value::Null],
    );
    assert_eq!(refused.unwrap_err().code, "trait.unknown");
}

#[test]
fn the_verbs_give_and_take_traits() {
    let mut sim = sim();
    let who = person(&mut sim, 3, 3);
    let me = sim.subject_value(Subject::Entity(who));
    let stacks = call(
        &mut sim,
        "add_trait",
        &[
            me.clone(),
            Value::text("core:bleeding"),
            Value::Real(Real::of(4.0)),
        ],
    )
    .unwrap();
    assert_eq!(stacks, Value::Int(1));
    call(
        &mut sim,
        "add_trait",
        &[me.clone(), Value::text("core:bleeding"), Value::Null],
    )
    .unwrap();
    assert_eq!(stacks_of(&sim, who), 2);
    let taken = call(
        &mut sim,
        "remove_trait",
        &[me.clone(), Value::text("core:bleeding"), Value::Int(1)],
    )
    .unwrap();
    assert_eq!(taken, Value::Int(1));
    assert_eq!(
        prop(&sim, who, "has_trait", &[Value::text("core:bleeding")]),
        Value::Bool(true)
    );
    match prop(&sim, who, "traits", &[]) {
        Value::List(items) => assert_eq!(items.len(), 1),
        other => panic!("{other:?}"),
    }
    let taken = call(
        &mut sim,
        "remove_trait",
        &[me, Value::text("core:bleeding"), Value::Null],
    )
    .unwrap();
    assert_eq!(taken, Value::Int(1));
    assert_eq!(
        prop(&sim, who, "has_trait", &[Value::text("core:bleeding")]),
        Value::Bool(false)
    );
}

fn stacks_of(sim: &Sim, who: EntityId) -> u32 {
    stacks(sim, who, "bleeding")
}

// ---- Givers: items and postures ---------------------------------------------

#[test]
fn an_item_gives_its_traits_to_whoever_holds_it() {
    let mut sim = sim();
    let who = person(&mut sim, 3, 3);
    let charm_def = DEFS.lookup::<ItemDef>("core:charm").unwrap();
    let charm = item::create(&mut sim, charm_def, 1, on(3, 3), Cause::DEBUG).unwrap();
    item::take(&mut sim, who, charm, Cause::DEBUG).unwrap();
    assert!(has(&sim, who, "hardy"));
    assert!(has_tag(&sim, who, "hardy"));
    assert!(close(quickness(&sim, who), 1.5));
    let inst = &traits_of(&sim, who).unwrap().instances()[0];
    assert_eq!(inst.giver(), Giver::item(charm));
    item::drop(&mut sim, charm, Cause::DEBUG).unwrap();
    assert!(!has(&sim, who, "hardy"));
    assert!(!has_tag(&sim, who, "hardy"));
    assert!(close(quickness(&sim, who), 1.0));
    let seen = events(&mut sim);
    let removed = named(&seen, "trait_removed");
    assert_eq!(text_field(removed[0], "how"), "withdrawn");
}

#[test]
fn a_tag_both_an_item_and_a_trait_confer_stays_while_either_does() {
    let mut sim = sim();
    let who = person(&mut sim, 3, 3);
    let charm_def = DEFS.lookup::<ItemDef>("core:charm").unwrap();
    let charm = item::create(&mut sim, charm_def, 1, on(3, 3), Cause::DEBUG).unwrap();
    // A rule gives the trait, then the charm is taken and let go.
    give(&mut sim, who, "hardy");
    item::take(&mut sim, who, charm, Cause::DEBUG).unwrap();
    item::drop(&mut sim, charm, Cause::DEBUG).unwrap();
    assert!(
        has_tag(&sim, who, "hardy"),
        "the rule's trait still confers it"
    );
    take(&mut sim, who, "hardy");
    assert!(!has_tag(&sim, who, "hardy"));
    // The charm first, then the rule's trait taken away while it is held.
    item::take(&mut sim, who, charm, Cause::DEBUG).unwrap();
    give(&mut sim, who, "hardy");
    withdraw(&mut sim, who, Giver::RULE, Cause::DEBUG);
    assert!(has_tag(&sim, who, "hardy"), "the charm still confers it");
    item::drop(&mut sim, charm, Cause::DEBUG).unwrap();
    assert!(!has_tag(&sim, who, "hardy"));
}

#[test]
fn a_posture_gives_its_traits_while_it_is_held() {
    let mut sim = sim();
    let who = person(&mut sim, 3, 3);
    let braced = DEFS.lookup::<PostureDef>("core:braced").unwrap();
    clash_sim::movement::set_posture(&mut sim, who, Some(braced), Cause::DEBUG).unwrap();
    assert!(has(&sim, who, "steadied"));
    assert!(close(quickness(&sim, who), 0.75));
    assert_eq!(
        traits_of(&sim, who).unwrap().instances()[0].giver(),
        Giver::posture(braced)
    );
    clash_sim::movement::set_posture(&mut sim, who, None, Cause::DEBUG).unwrap();
    assert!(!has(&sim, who, "steadied"));
    assert!(close(quickness(&sim, who), 1.0));
}

#[test]
fn withdrawing_a_giver_takes_only_its_own() {
    let mut sim = sim();
    let who = person(&mut sim, 3, 3);
    let giver = Giver::new(GiverKind::Other, 7);
    give(&mut sim, who, "hardy");
    add_trait(&mut sim, who, trait_id("hardy"), giver, None, Cause::DEBUG).unwrap();
    add_trait(
        &mut sim,
        who,
        trait_id("bleeding"),
        giver,
        None,
        Cause::DEBUG,
    )
    .unwrap();
    assert_eq!(withdraw(&mut sim, who, giver, Cause::DEBUG), 2);
    assert_eq!(stacks(&sim, who, "hardy"), 1);
    assert!(!has(&sim, who, "bleeding"));
}

// ---- Removing exactly what was added ----------------------------------------

/// What an entity's stats, tags and rules hold from its traits must equal
/// what a recomputation from its instances gives.
fn check_conferred(sim: &Sim, who: EntityId, base_tags: &[String]) {
    let wanted = traits::conferred(sim, who);
    let stats = sim.components().stats.get(who).unwrap();
    let mut modifiers: Vec<stats::Modifier> = stats
        .modifiers()
        .iter()
        .filter(|modifier| modifier.source.kind == SourceKind::Trait)
        .copied()
        .collect();
    modifiers.sort_by_key(|modifier| (modifier.attr, modifier.source));
    assert_eq!(modifiers, wanted.modifiers, "modifiers");
    let mut rates = Vec::new();
    for pool in stats.pools() {
        for rate in stats.pool_rates(pool) {
            if rate.source.kind == SourceKind::Trait {
                rates.push((pool, rate.source, rate.rate));
            }
        }
    }
    assert_eq!(rates, wanted.rates, "rates");
    let traits = traits_of(sim, who);
    let recorded = traits.map_or(&[][..], Traits::conferred_tags);
    let mut expected: Vec<String> = base_tags.to_vec();
    for tag in &wanted.tags {
        if !expected.contains(tag) {
            expected.push(tag.clone());
        }
    }
    expected.sort();
    assert_eq!(tags(sim, who), expected, "tags");
    for tag in recorded {
        assert!(
            wanted.tags.contains(tag),
            "`{tag}` is recorded and wanted by none"
        );
    }
    let mut rules = sim.rules.attached_to(handle(sim, who)).to_vec();
    rules.sort();
    assert_eq!(rules, wanted.rules, "rules");
    // Every running duration is on the timeline.
    for inst in traits.map_or(&[][..], Traits::instances) {
        if let Timer::Running(wake) = inst.timer() {
            assert!(sim.timeline().wake(wake).is_some());
        }
    }
}

#[derive(Clone, Debug)]
enum Op {
    Give(usize, usize),
    Remove(usize, Option<u16>),
    Withdraw(usize),
    Advance(i64),
    Arm(bool),
    Away(i64),
}

const NAMES: &[&str] = &[
    "stimulated",
    "crashing",
    "guarding",
    "lunging",
    "bleeding",
    "hardy",
    "armed_focus",
    "lingering",
    "fleeting",
];

const GIVERS: &[Giver] = &[
    Giver::RULE,
    Giver::new(GiverKind::Template, 1),
    Giver::new(GiverKind::Other, 2),
];

fn op() -> impl Strategy<Value = Op> {
    prop_oneof![
        4 => (0..NAMES.len(), 0..GIVERS.len()).prop_map(|(name, giver)| Op::Give(name, giver)),
        2 => (0..NAMES.len(), proptest::option::of(1_u16..4))
            .prop_map(|(name, stacks)| Op::Remove(name, stacks)),
        1 => (0..GIVERS.len()).prop_map(Op::Withdraw),
        2 => (1_i64..25).prop_map(Op::Advance),
        1 => any::<bool>().prop_map(Op::Arm),
        1 => (1_i64..40).prop_map(Op::Away),
    ]
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(48))]

    /// Removing a trait removes exactly what it added, whatever was added,
    /// stacked, removed, withdrawn and let run out before, and with
    /// everything gone the entity is as it was.
    #[test]
    fn removing_traits_removes_exactly_what_they_added(ops in proptest::collection::vec(op(), 1..40)) {
        let mut sim = sim();
        let who = person(&mut sim, 3, 3);
        let initial = quickness(&sim, who);
        let initial_rates = (rate(&sim, who, "stamina"), rate(&sim, who, "blood"));
        let mut base = tags(&sim, who);
        let mut now = 0;
        for op in ops {
            match op {
                Op::Give(name, giver) => {
                    let _ = add_trait(&mut sim, who, trait_id(NAMES[name]), GIVERS[giver], None, Cause::DEBUG);
                }
                Op::Remove(name, stacks) => {
                    remove_trait(&mut sim, who, trait_id(NAMES[name]), stacks, Cause::DEBUG).unwrap();
                }
                Op::Withdraw(giver) => {
                    withdraw(&mut sim, who, GIVERS[giver], Cause::DEBUG);
                }
                Op::Advance(seconds) => {
                    now += seconds;
                    advance_to(&mut sim, now);
                }
                Op::Arm(armed) => {
                    let record = sim.entity_mut(who).unwrap();
                    if armed {
                        record.tags.insert("armed".to_owned());
                    } else {
                        record.tags.remove("armed");
                    }
                    base.retain(|tag| tag != "armed");
                    if armed {
                        base.push("armed".to_owned());
                    }
                    traits::reconsider(&mut sim, who);
                }
                Op::Away(seconds) => {
                    sim.relocate(who, Place::Stored, Cause::DEBUG).unwrap();
                    now += seconds;
                    advance_to(&mut sim, now);
                    sim.relocate(who, on(3, 3), Cause::DEBUG).unwrap();
                }
            }
            sim.pump();
            check_conferred(&sim, who, &base);
        }
        for name in NAMES {
            remove_trait(&mut sim, who, trait_id(name), None, Cause::DEBUG).unwrap();
        }
        check_conferred(&sim, who, &base);
        prop_assert!(traits_of(&sim, who).is_none_or(|traits| traits.instances().is_empty()));
        prop_assert_eq!(quickness(&sim, who).to_bits(), initial.to_bits());
        prop_assert_eq!(rate(&sim, who, "stamina").to_bits(), initial_rates.0.to_bits());
        prop_assert_eq!(rate(&sim, who, "blood").to_bits(), initial_rates.1.to_bits());
        prop_assert!(sim.rules.attached_to(handle(&sim, who)).is_empty());
        prop_assert!(sim.truth().wakes_of(who).is_empty());
    }
}
