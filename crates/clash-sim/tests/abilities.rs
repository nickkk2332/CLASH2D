//! Tests of abilities (08; packages A1 to A4) against the real
//! simulation: the world, life, traits and movement content of
//! `mods/core`, its bundled powers (`mods/core/data/powers`), and a few
//! definitions of the tests' own.
//!
//! | Package | Done when | Test |
//! |---------|-----------|------|
//! | A1 | A reactive value always equals a fresh evaluation (A6) | `a_kept_value_always_equals_a_fresh_evaluation` |
//! | A1 | Re-evaluated exactly when a dependency changed, at most once a pump | `a_condition_is_worked_out_again_only_when_what_it_read_changed` |
//! | A1 | A property registered without the events that change it fails | `a_kept_condition_may_read_only_what_says_how_it_changes` |
//! | A2 | Two sources, one instance, lasting as long as either (A8) | `one_ability_from_two_sources_lasts_as_long_as_either` |
//! | A2 | A mind keeps mind-seated abilities in another body | `a_mind_takes_its_abilities_and_leaves_the_body_its_own` |
//! | A2 | A parameter changes with rank and an unrelated trait | `a_parameter_follows_rank_and_a_modifier_from_a_trait` |
//! | A3 | A cost in full or no start (A4), with the shortfall | `a_cost_is_taken_in_full_or_nothing_starts` |
//! | A3 | Overdraw takes the shortfall at the ratio | `overdraw_takes_exactly_the_shortfall_at_its_ratio` |
//! | A3 | `ability_using` changes one use | `a_rule_on_ability_using_changes_one_use_only` |
//! | A3 | A gagged character cannot use what needs a voice | `a_gagged_character_cannot_chant` |
//! | A3 | A cost from something bound | `a_cost_drawn_from_another_is_theirs_to_pay` |
//! | A3 | Cooldowns survive a save | `a_cooldown_runs_on_its_owners_time_and_survives_a_save` |
//! | A4 | Trait and upkeep together (A5) | `a_modes_trait_and_upkeep_begin_and_end_together` |
//! | A4 | A mode ends at the analytic tick | `a_mode_ends_the_tick_its_pool_runs_dry_with_nothing_between` |
//! | A4 | Charge follows time held | `charge_delivered_follows_time_held` |

mod common;

use std::path::PathBuf;
use std::sync::{Arc, LazyLock};

use clash_core::{Dur, EntityId, Real, Tick};
use clash_data::{Defs, Loader};
use clash_expr::Value;
use clash_sim::ability::{
    self, Abilities, AbilityDef, AbilityId, AbilitySource, State, codes, reactive::Watcher,
};
use clash_sim::action::{ActionDef, ActionDefId, Params};
use clash_sim::stats::{PoolDef, PoolId, Stats};
use clash_sim::template::CharacterSpec;
use clash_sim::traits::{self, Giver, TraitDef, TraitId};
use clash_sim::{AdvanceLimit, Command, Control, DebugCommand, Sim, SimEvent, Subject};
use clash_world::Cause;
use proptest::prelude::*;

/// Definitions of the tests' own, in the namespace `core`.
const TEST_DEFS: &str = r#"[
  { "type": "pool", "id": "mana", "max": 100, "start": "max" },
  { "type": "pool", "id": "vigor", "max": 100, "start": "max" },
  { "type": "pool", "id": "wellspring", "max": 100, "start": "max" },
  { "type": "track", "id": "focus", "levels": "1 * $level" },
  { "type": "variable", "id": "charged", "scope": "entity", "value": "bool", "default": false, "doc": "A test's switch." },
  { "type": "variable", "id": "boost", "scope": "entity", "value": "bool", "default": false, "doc": "Whether the next use is boosted." },
  { "type": "variable", "id": "last_energy", "scope": "entity", "value": "real", "default": 0, "doc": "What the last zap was given." },
  { "type": "variable", "id": "last_charge", "scope": "entity", "value": "int", "default": 0, "doc": "What the last gathering held." },
  { "type": "variable", "id": "companion", "scope": "entity", "value": "entity", "doc": "Who pays for a borrowed power." },
  { "type": "trait", "id": "lit", "tags": ["lit"] },
  { "type": "trait", "id": "glowing", "tags": ["glowing"] },
  { "type": "trait", "id": "burning", "tags": ["burning"] },
  { "type": "trait", "id": "giver_a", "abilities": ["spark"] },
  { "type": "trait", "id": "giver_b", "abilities": ["spark"] },
  { "type": "trait", "id": "amplified", "ability_params": [ { "tag": "test", "param": "power", "mul": 1.5 } ] },
  { "type": "action", "id": "zap", "params": { "energy": "real" },
    "phases": [ { "id": "go", "time": "100 ms", "do": [ { "set": { "self.var.last_energy": "$energy" } } ] } ] },
  { "type": "action", "id": "chant", "uses": { "voice": 1 }, "phases": [ { "id": "go", "time": "1 s" } ] },
  { "type": "action", "id": "gather", "vars": { "charge": { "type": "int", "default": 0 } },
    "phases": [
      { "id": "hold", "time": "hold", "max": "5 s",
        "tick": { "every": "250 ms", "do": [ { "set": { "$charge": "$charge + 1" } } ] } },
      { "id": "loose", "time": 0, "do": [ { "set": { "self.var.last_charge": "$charge" } } ] } ] },
  { "type": "ability", "id": "spark", "tags": ["test"],
    "params": { "power": "10 * ability.rank" },
    "always": [ { "traits": ["glowing"] }, { "at_rank": 2, "traits": ["burning"] } ],
    "grants": [
      { "action": "zap", "params": { "energy": "ability.power" }, "cost": { "mana": 30 }, "cooldown": "5 s" },
      { "action": "chant" },
      { "action": "gather" } ],
    "modes": [ { "id": "aura", "trait": "lit", "upkeep": { "wellspring": 4 } } ] },
  { "type": "ability", "id": "dark_sight", "tags": ["test"], "seat": "body" },
  { "type": "ability", "id": "insight", "tags": ["test"], "seat": "mind" },
  { "type": "ability", "id": "blood_price", "tags": ["test"],
    "grants": [ { "action": "zap", "params": { "energy": 1 },
                  "cost": { "mana": { "amount": 30, "overdraw": { "from": "vigor", "ratio": 2.0 } } } } ] },
  { "type": "ability", "id": "borrowed", "tags": ["test"],
    "grants": [ { "action": "zap", "params": { "energy": 1 },
                  "cost": { "mana": { "amount": 30, "from": "self.var.companion" } } } ] },
  { "type": "ability", "id": "fickle", "tags": ["test"],
    "while": "self.var.charged and self.track.focus >= 1 and not self.has_trait('core:lit')",
    "always": { "traits": ["glowing"] } },
  { "type": "eoc", "id": "boost_once", "on": "ability_using", "if": "self.var.boost",
    "do": [ { "modify": { "event.params": "{'energy': 99}", "event.cost": "{'core:mana': 5}" } },
            { "set": { "self.var.boost": false } } ] },
  { "type": "template", "id": "tester", "copy_from": "person",
    "pools": ["blood", "pain", "stamina", "reserve", "mana", "vigor", "wellspring"] }
]"#;

/// One directory of `mods/core/data`, as paths within the mod and text.
fn dir_files(sub: &str) -> Vec<(String, String)> {
    let dir: PathBuf = common::mods_dir().join("core/data").join(sub);
    let mut files: Vec<PathBuf> = std::fs::read_dir(&dir)
        .unwrap()
        .filter_map(|entry| entry.ok().map(|entry| entry.path()))
        .filter(|path| path.extension().is_some_and(|ext| ext == "json"))
        .collect();
    files.sort();
    files
        .iter()
        .map(|file| {
            let name = file.file_name().unwrap().to_string_lossy();
            (
                format!("data/{sub}/{name}"),
                std::fs::read_to_string(file).unwrap(),
            )
        })
        .collect()
}

fn core_file(name: &str) -> (String, String) {
    let path = common::mods_dir().join("core/data").join(name);
    (
        format!("data/{name}"),
        std::fs::read_to_string(path).unwrap(),
    )
}

/// Loads `mods/core`'s world, life, traits, movement and powers with
/// more definitions, or renders why not.
fn try_load(extra: &str) -> Result<Arc<Defs>, String> {
    let types = common::types();
    let mut files = dir_files("world");
    files.extend(dir_files("life"));
    files.extend(dir_files("powers"));
    files.push(core_file("traits.json"));
    files.push(core_file("movement.json"));
    files.push((
        "data/test/channels.json".to_owned(),
        r#"[{ "type": "channel", "id": "arm", "name": "arm" }]"#.to_owned(),
    ));
    files.push(("data/test/extra.json".to_owned(), extra.to_owned()));
    let files: Vec<(&str, &str)> = files
        .iter()
        .map(|(path, text)| (path.as_str(), text.as_str()))
        .collect();
    Loader::new(&types)
        .expr_registry(&clash_sim::engine_registry())
        .memory_mod("core", r#"{ "id": "core" }"#, &files)
        .load_mods(&["core"])
        .into_result()
        .map_err(|diagnostics| diagnostics.render())
}

static DEFS: LazyLock<Arc<Defs>> =
    LazyLock::new(|| try_load(TEST_DEFS).unwrap_or_else(|error| panic!("{error}")));

fn sim() -> Sim {
    common::sim_over(Arc::clone(&DEFS), 11)
}

fn cause() -> Cause {
    Cause::DEBUG
}

fn ability_id(name: &str) -> AbilityId {
    DEFS.lookup::<AbilityDef>(&format!("core:{name}"))
        .unwrap_or_else(|| panic!("no ability `{name}`"))
}

fn trait_id(name: &str) -> TraitId {
    DEFS.lookup::<TraitDef>(&format!("core:{name}"))
        .unwrap_or_else(|| panic!("no trait `{name}`"))
}

fn pool(name: &str) -> PoolId {
    DEFS.lookup::<PoolDef>(&format!("core:{name}")).unwrap()
}

fn action(name: &str) -> ActionDefId {
    DEFS.lookup::<ActionDef>(&format!("core:{name}")).unwrap()
}

/// Someone made from a template, standing at a cell.
fn spawn(sim: &mut Sim, template: &str, x: i32, y: i32) -> EntityId {
    sim.spawn_template(
        &CharacterSpec::of([format!("core:{template}")]),
        common::on(x, y),
        cause(),
    )
    .unwrap_or_else(|reason| panic!("{reason:?}"))
    .entity
}

fn level(sim: &Sim, who: EntityId, name: &str) -> Real {
    sim.components()
        .get::<Stats>(who)
        .and_then(|stats| stats.pool_level(sim.timeline(), pool(name)))
        .unwrap()
}

fn var(sim: &Sim, who: EntityId, name: &str) -> Value {
    sim.var(Subject::Entity(who), &format!("core:{name}"))
}

fn set_var(sim: &mut Sim, who: EntityId, name: &str, value: Value) {
    sim.set_var(
        Subject::Entity(who),
        &format!("core:{name}"),
        value,
        cause(),
    )
    .unwrap();
}

fn holds(sim: &Sim, who: EntityId, name: &str) -> bool {
    sim.components()
        .get::<Abilities>(who)
        .is_some_and(|abilities| abilities.holds(ability_id(name)))
}

fn state(sim: &Sim, who: EntityId, name: &str) -> State {
    sim.components()
        .get::<Abilities>(who)
        .and_then(|abilities| abilities.of(ability_id(name)))
        .unwrap()
        .state()
}

fn has_trait(sim: &Sim, who: EntityId, name: &str) -> bool {
    sim.components()
        .get::<traits::Traits>(who)
        .is_some_and(|traits| traits.has(trait_id(name)))
}

fn grant(sim: &mut Sim, who: EntityId, name: &str) -> u64 {
    let number = ability::grant(
        sim,
        who,
        ability_id(name),
        AbilitySource::Rule,
        None,
        cause(),
    )
    .unwrap();
    sim.pump();
    number
}

fn advance(sim: &mut Sim, by: Dur) {
    let until = sim.now() + by;
    sim.advance(AdvanceLimit::Until(until));
}

fn use_it(sim: &mut Sim, who: EntityId, name: &str, act: &str) -> Result<bool, clash_sim::Reason> {
    let started = ability::use_ability(
        sim,
        who,
        ability_id(name),
        Some(action(act)),
        &Params::new(),
        cause(),
    )?;
    sim.pump();
    Ok(started.is_some())
}

fn names_of(events: &[SimEvent]) -> Vec<&str> {
    events.iter().map(SimEvent::name).collect()
}

fn field(event: &SimEvent, name: &str) -> Option<clash_sim::FieldValue> {
    let _ = &event;
    event.body.field(name)
}

fn text(event: &SimEvent, name: &str) -> String {
    match field(event, name) {
        Some(clash_sim::FieldValue::Plain(clash_core::Value::Str(text))) => text.to_string(),
        other => panic!("`{name}` is {other:?}"),
    }
}

// ---- A1: expressions kept current ----------------------------------------------

#[test]
fn a_kept_condition_may_read_only_what_says_how_it_changes() {
    // A pool's level moves all the time and names no event: refused.
    let pool_level = r#"[ { "type": "ability", "id": "pooled",
        "while": "self.pool.reserve > 10" } ]"#;
    let error = try_load(pool_level).unwrap_err();
    assert!(error.contains("pools.reserve"), "{error}");
    assert!(error.contains("kept current"), "{error}");
    // A tag names none either.
    let tagged = r#"[ { "type": "ability", "id": "tagged", "while": "self.has_tag('x')" } ]"#;
    let error = try_load(tagged).unwrap_err();
    assert!(error.contains("entity.has_tag"), "{error}");
    // A random draw is refused by the field's purity.
    let random = r#"[ { "type": "ability", "id": "chancy", "while": "chance(0.5)" } ]"#;
    assert!(try_load(random).is_err());
    // `now()` is a function with no events.
    let timed = r#"[ { "type": "ability", "id": "timed", "while": "now() > 5" } ]"#;
    let error = try_load(timed).unwrap_err();
    assert!(error.contains("`now`"), "{error}");
    // A structure is written as one expression.
    let structured = r#"[ { "type": "ability", "id": "structured",
        "while": { "all": [ "self.var.charged", "true" ] } } ]"#;
    let error = try_load(structured).unwrap_err();
    assert!(error.contains("one expression"), "{error}");
    // What names its events, a variable and a pure built-in are fine.
    let fine = r#"[ { "type": "ability", "id": "fine",
        "while": "max(self.track.pyromancy, 0) >= 1 and self.has_trait('core:dashing')" } ]"#;
    try_load(fine).unwrap();
    // Every property of abilities says what changes it, or is constant.
    let registry = clash_sim::engine_registry();
    let kind = registry.kind_id(ability::ABILITY_KIND).unwrap();
    for property in registry.properties() {
        if property.spec.kind == kind {
            let constant = ability::reactive::CONSTANT_PROPERTIES
                .iter()
                .any(|(of, name)| *of == "ability" && *name == property.spec.name);
            let moves = ["cooldown"].contains(&property.spec.name.as_str());
            assert!(
                constant || moves || !property.spec.changed_by.is_empty(),
                "`ability.{}` names no event that changes it",
                property.spec.name
            );
        }
    }
}

#[test]
fn a_condition_is_worked_out_again_only_when_what_it_read_changed() {
    let mut sim = sim();
    let who = spawn(&mut sim, "tester", 10, 10);
    let other = spawn(&mut sim, "tester", 14, 10);
    sim.pump();
    let number = grant(&mut sim, who, "fickle");
    let watcher = Watcher::Ability {
        entity: who,
        number,
    };
    assert!(sim.reactive().watches(watcher));
    assert_eq!(state(&sim, who, "fickle"), State::Dormant);
    let before = sim.reactive().evaluations();
    // What it did not read: nothing is worked out.
    set_var(&mut sim, other, "charged", Value::Bool(true));
    set_var(&mut sim, who, "boost", Value::Bool(true));
    sim.pump();
    advance(&mut sim, Dur::from_secs(1));
    assert_eq!(sim.reactive().evaluations(), before);
    // Twice in one pump: once.
    set_var(&mut sim, who, "charged", Value::Bool(true));
    set_var(&mut sim, who, "charged", Value::Bool(false));
    set_var(&mut sim, who, "charged", Value::Bool(true));
    sim.pump();
    assert_eq!(sim.reactive().evaluations(), before + 1);
    assert_eq!(state(&sim, who, "fickle"), State::Dormant);
    // The track it reads: once more, and now it holds.
    sim.advance_track(
        who,
        DEFS.lookup::<clash_sim::stats::TrackDef>("core:focus")
            .unwrap(),
        Real::ONE,
        cause(),
    )
    .unwrap();
    sim.pump();
    // Once for the track; once more because becoming active gave it a
    // trait, and a trait given is what `has_trait` names as changing it.
    assert_eq!(sim.reactive().evaluations(), before + 3);
    assert_eq!(state(&sim, who, "fickle"), State::Active);
    assert!(has_trait(&sim, who, "glowing"));
    // A trait it reads.
    traits::add_trait(&mut sim, who, trait_id("lit"), Giver::RULE, None, cause()).unwrap();
    sim.pump();
    assert_eq!(state(&sim, who, "fickle"), State::Dormant);
    assert!(
        !has_trait(&sim, who, "glowing"),
        "a dormant ability confers nothing"
    );
}

/// What the tests change about someone in random order.
#[derive(Clone, Debug)]
enum Change {
    Charge(bool),
    Focus,
    Light(bool),
    Pump,
}

fn change() -> impl Strategy<Value = Change> {
    prop_oneof![
        any::<bool>().prop_map(Change::Charge),
        Just(Change::Focus),
        any::<bool>().prop_map(Change::Light),
        Just(Change::Pump),
    ]
}

/// The condition of `fickle`, worked out by hand.
fn fresh(sim: &Sim, who: EntityId) -> bool {
    let charged = var(sim, who, "charged") == Value::Bool(true);
    let focus = sim.components().get::<Stats>(who).map_or(0, |stats| {
        stats.track_level(
            DEFS.lookup::<clash_sim::stats::TrackDef>("core:focus")
                .unwrap(),
        )
    });
    charged && focus >= 1 && !has_trait(sim, who, "lit")
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(24))]

    #[test]
    fn a_kept_value_always_equals_a_fresh_evaluation(changes in prop::collection::vec(change(), 1..24)) {
        let mut sim = sim();
        let who = spawn(&mut sim, "tester", 10, 10);
        sim.pump();
        let number = grant(&mut sim, who, "fickle");
        let watcher = Watcher::Ability { entity: who, number };
        for change in changes {
            match change {
                Change::Charge(on) => set_var(&mut sim, who, "charged", Value::Bool(on)),
                Change::Focus => sim
                    .advance_track(who, DEFS.lookup::<clash_sim::stats::TrackDef>("core:focus").unwrap(), Real::ONE, cause())
                    .unwrap(),
                Change::Light(on) => {
                    if on {
                        traits::add_trait(&mut sim, who, trait_id("lit"), Giver::RULE, None, cause()).unwrap();
                    } else {
                        traits::remove_trait(&mut sim, who, trait_id("lit"), None, cause()).unwrap();
                    }
                }
                Change::Pump => advance(&mut sim, Dur::from_millis(100)),
            }
            sim.pump();
            let expected = fresh(&sim, who);
            prop_assert_eq!(sim.reactive().value(watcher), Some(expected));
            prop_assert_eq!(state(&sim, who, "fickle") == State::Active, expected);
            prop_assert_eq!(has_trait(&sim, who, "glowing"), expected);
        }
    }
}

// ---- A2: holding ---------------------------------------------------------------

#[test]
fn one_ability_from_two_sources_lasts_as_long_as_either() {
    let mut sim = sim();
    let who = spawn(&mut sim, "tester", 10, 10);
    sim.pump();
    traits::add_trait(
        &mut sim,
        who,
        trait_id("giver_a"),
        Giver::RULE,
        None,
        cause(),
    )
    .unwrap();
    sim.pump();
    let first = sim
        .components()
        .get::<Abilities>(who)
        .unwrap()
        .of(ability_id("spark"))
        .unwrap()
        .number();
    traits::add_trait(
        &mut sim,
        who,
        trait_id("giver_b"),
        Giver::RULE,
        None,
        cause(),
    )
    .unwrap();
    sim.pump();
    let abilities = sim.components().get::<Abilities>(who).unwrap();
    assert_eq!(abilities.instances().len(), 1, "one instance");
    let inst = abilities.of(ability_id("spark")).unwrap();
    assert_eq!(inst.number(), first);
    assert_eq!(inst.sources().len(), 2);
    traits::remove_trait(&mut sim, who, trait_id("giver_a"), None, cause()).unwrap();
    sim.pump();
    assert!(holds(&sim, who, "spark"), "the second source holds it");
    assert!(has_trait(&sim, who, "glowing"));
    sim.drain_events();
    traits::remove_trait(&mut sim, who, trait_id("giver_b"), None, cause()).unwrap();
    sim.pump();
    assert!(!holds(&sim, who, "spark"), "it ends with its last source");
    assert!(
        !has_trait(&sim, who, "glowing"),
        "what it conferred is taken back"
    );
    let events = sim.drain_events();
    let revoked = events
        .iter()
        .find(|event| event.name() == ability::ABILITY_REVOKED)
        .unwrap();
    assert_eq!(text(revoked, "how"), "lost");
    assert!(sim.ability_index().owner(first).is_none());
}

#[test]
fn revoking_holds_against_a_trait_until_a_rule_gives_it_back() {
    let mut sim = sim();
    let who = spawn(&mut sim, "tester", 10, 10);
    sim.pump();
    traits::add_trait(
        &mut sim,
        who,
        trait_id("giver_a"),
        Giver::RULE,
        None,
        cause(),
    )
    .unwrap();
    sim.pump();
    assert!(ability::revoke(&mut sim, who, ability_id("spark"), cause()).unwrap());
    sim.pump();
    assert!(!holds(&sim, who, "spark"));
    assert!(!has_trait(&sim, who, "glowing"));
    // Another trait listing it does not bring it back.
    traits::add_trait(
        &mut sim,
        who,
        trait_id("giver_b"),
        Giver::RULE,
        None,
        cause(),
    )
    .unwrap();
    sim.pump();
    assert!(!holds(&sim, who, "spark"));
    // A rule does.
    grant(&mut sim, who, "spark");
    assert!(holds(&sim, who, "spark"));
    assert!(has_trait(&sim, who, "glowing"));
}

#[test]
fn a_mind_takes_its_abilities_and_leaves_the_body_its_own() {
    let mut sim = sim();
    let first = spawn(&mut sim, "tester", 10, 10);
    let second = spawn(&mut sim, "tester", 12, 10);
    sim.pump();
    let insight = grant(&mut sim, first, "insight");
    grant(&mut sim, first, "dark_sight");
    grant(&mut sim, second, "dark_sight");
    sim.transfer_mind(first, second, true, cause()).unwrap();
    sim.pump();
    assert!(
        holds(&sim, second, "insight"),
        "the mind brings what it knows"
    );
    assert!(
        holds(&sim, second, "dark_sight"),
        "and gains what the body can do"
    );
    assert!(!holds(&sim, first, "insight"));
    assert!(holds(&sim, first, "dark_sight"), "the body keeps its own");
    assert_eq!(
        sim.ability_index().owner(insight),
        Some(second),
        "it is the same instance"
    );
}

#[test]
fn a_parameter_follows_rank_and_a_modifier_from_a_trait() {
    let mut sim = sim();
    let who = spawn(&mut sim, "tester", 10, 10);
    sim.pump();
    grant(&mut sim, who, "spark");
    assert!(use_it(&mut sim, who, "spark", "zap").unwrap());
    advance(&mut sim, Dur::from_millis(200));
    assert_eq!(var(&sim, who, "last_energy"), Value::Real(Real::of(10.0)));
    // Rank 2: twice the power, and what applies from rank 2.
    ability::set_rank(&mut sim, who, ability_id("spark"), 2, cause()).unwrap();
    sim.pump();
    assert!(has_trait(&sim, who, "burning"));
    advance(&mut sim, Dur::from_secs(6));
    assert!(use_it(&mut sim, who, "spark", "zap").unwrap());
    advance(&mut sim, Dur::from_millis(200));
    assert_eq!(var(&sim, who, "last_energy"), Value::Real(Real::of(20.0)));
    // A trait that has nothing to do with it strengthens every `test`
    // ability's power by half.
    traits::add_trait(
        &mut sim,
        who,
        trait_id("amplified"),
        Giver::RULE,
        None,
        cause(),
    )
    .unwrap();
    sim.pump();
    advance(&mut sim, Dur::from_secs(6));
    assert!(use_it(&mut sim, who, "spark", "zap").unwrap());
    advance(&mut sim, Dur::from_millis(200));
    assert_eq!(var(&sim, who, "last_energy"), Value::Real(Real::of(30.0)));
    // Read in an expression, the same.
    let read = sim.eval(
        &compile(&sim, "self.ability('core:spark').power"),
        clash_sim::Bound::new().subject(sim.subject_value(Subject::Entity(who))),
    );
    assert_eq!(read, Value::Real(Real::of(30.0)));
    // Back to rank 1: what applied from rank 2 is taken back.
    ability::set_rank(&mut sim, who, ability_id("spark"), 1, cause()).unwrap();
    sim.pump();
    assert!(!has_trait(&sim, who, "burning"));
}

fn compile(sim: &Sim, source: &str) -> clash_expr::Compiled {
    let registry = sim.defs().expr_registry();
    let entity = registry.kind_id("entity").unwrap();
    let context = clash_expr::Context::new().subject("self", clash_expr::Type::Subject(entity));
    clash_expr::compile(source, registry, &context, &clash_expr::Type::Any)
        .into_result()
        .unwrap()
}

// ---- A3: using ------------------------------------------------------------------

#[test]
fn a_cost_is_taken_in_full_or_nothing_starts() {
    let mut sim = sim();
    let who = spawn(&mut sim, "tester", 10, 10);
    sim.pump();
    grant(&mut sim, who, "spark");
    sim.pool_spend(who, pool("mana"), Real::of(80.0), cause())
        .unwrap();
    let hash = sim.state_hash();
    let refused = use_it(&mut sim, who, "spark", "zap").unwrap_err();
    assert_eq!(refused.code, codes::COST_SHORT);
    let value = |name: &str| {
        refused
            .values
            .iter()
            .find(|(have, _)| have == name)
            .map(|(_, value)| value.clone())
            .unwrap()
    };
    assert_eq!(value("pool"), clash_core::Value::from("core:mana"));
    assert_eq!(value("wanted"), clash_core::Value::from(Real::of(30.0)));
    assert_eq!(value("have"), clash_core::Value::from(Real::of(20.0)));
    assert_eq!(value("short"), clash_core::Value::from(Real::of(10.0)));
    assert_eq!(sim.state_hash(), hash, "a refusal changes nothing");
    // Exactly enough: all of it is taken and the action starts.
    sim.pool_add(who, pool("mana"), Real::of(10.0), cause())
        .unwrap();
    assert!(use_it(&mut sim, who, "spark", "zap").unwrap());
    assert_eq!(level(&sim, who, "mana"), Real::ZERO);
    assert!(!sim.actions_of(who).is_empty());
}

#[test]
fn overdraw_takes_exactly_the_shortfall_at_its_ratio() {
    let mut sim = sim();
    let who = spawn(&mut sim, "tester", 10, 10);
    sim.pump();
    grant(&mut sim, who, "blood_price");
    sim.pool_spend(who, pool("mana"), Real::of(80.0), cause())
        .unwrap();
    assert!(use_it(&mut sim, who, "blood_price", "zap").unwrap());
    assert_eq!(level(&sim, who, "mana"), Real::ZERO);
    assert_eq!(
        level(&sim, who, "vigor"),
        Real::of(80.0),
        "10 short at 2 to 1"
    );
    // Not enough of either: refused, naming the pool that is short.
    sim.pool_spend(who, pool("vigor"), Real::of(75.0), cause())
        .unwrap();
    let refused = ability::available(&sim, who, ability_id("blood_price"), None).unwrap_err();
    assert_eq!(refused.code, codes::COST_SHORT);
    assert!(refused.message.contains("vigor"), "{}", refused.message);
}

#[test]
fn a_cost_drawn_from_another_is_theirs_to_pay() {
    let mut sim = sim();
    let who = spawn(&mut sim, "tester", 10, 10);
    let companion = spawn(&mut sim, "tester", 11, 10);
    sim.pump();
    grant(&mut sim, who, "borrowed");
    // Nobody to pay: unavailable, saying so.
    let refused = ability::available(&sim, who, ability_id("borrowed"), None).unwrap_err();
    assert_eq!(refused.code, codes::COST_SOURCE_GONE);
    let handle = sim.subject_value(Subject::Entity(companion));
    set_var(&mut sim, who, "companion", handle);
    sim.pump();
    assert!(use_it(&mut sim, who, "borrowed", "zap").unwrap());
    assert_eq!(level(&sim, companion, "mana"), Real::of(70.0));
    assert_eq!(level(&sim, who, "mana"), Real::of(100.0));
    // The companion short: unavailable, naming who is short.
    sim.pool_spend(companion, pool("mana"), Real::of(60.0), cause())
        .unwrap();
    let refused = ability::available(&sim, who, ability_id("borrowed"), None).unwrap_err();
    assert_eq!(refused.code, codes::COST_SHORT);
    // Gone: unavailable again.
    sim.remove_entity(companion, cause()).unwrap();
    sim.pump();
    let refused = ability::available(&sim, who, ability_id("borrowed"), None).unwrap_err();
    assert_eq!(refused.code, codes::COST_SOURCE_GONE);
}

#[test]
fn a_rule_on_ability_using_changes_one_use_only() {
    let mut sim = sim();
    let who = spawn(&mut sim, "tester", 10, 10);
    sim.pump();
    grant(&mut sim, who, "spark");
    set_var(&mut sim, who, "boost", Value::Bool(true));
    assert!(use_it(&mut sim, who, "spark", "zap").unwrap());
    advance(&mut sim, Dur::from_millis(200));
    assert_eq!(
        var(&sim, who, "last_energy").as_number(),
        Some(Real::of(99.0))
    );
    assert_eq!(level(&sim, who, "mana"), Real::of(95.0));
    advance(&mut sim, Dur::from_secs(6));
    assert!(use_it(&mut sim, who, "spark", "zap").unwrap());
    advance(&mut sim, Dur::from_millis(200));
    assert_eq!(var(&sim, who, "last_energy"), Value::Real(Real::of(10.0)));
    assert_eq!(level(&sim, who, "mana"), Real::of(65.0));
}

#[test]
fn a_gagged_character_cannot_chant() {
    let mut sim = sim();
    let who = spawn(&mut sim, "tester", 10, 10);
    sim.pump();
    grant(&mut sim, who, "spark");
    let voice = DEFS
        .lookup::<clash_sim::action::ChannelDef>("core:voice")
        .unwrap();
    let registry = sim.defs().expr_registry();
    let channel_ty = registry.def_type_id("channel").unwrap();
    sim.call_verb(
        "disable_channel",
        &[
            ("who", sim.subject_value(Subject::Entity(who))),
            (
                "channel",
                Value::Id(clash_expr::DefRef::new(channel_ty, voice.index() as u32)),
            ),
        ],
        &[],
        cause(),
    )
    .unwrap();
    sim.drain_events();
    // A rule listens for `ability_using` in these definitions, so the
    // refusal comes after the hook and is raised, not returned.
    assert!(!use_it(&mut sim, who, "spark", "chant").unwrap());
    let events = sim.drain_events();
    let refused = events
        .iter()
        .find(|event| event.name() == ability::ABILITY_REFUSED)
        .expect("the refusal is raised");
    assert_eq!(text(refused, "reason"), "engine:action_channel_lacking");
    assert!(sim.actions_of(who).is_empty());
}

#[test]
fn a_cooldown_runs_on_its_owners_time_and_survives_a_save() {
    let mut sim = sim();
    let who = spawn(&mut sim, "tester", 10, 10);
    sim.pump();
    grant(&mut sim, who, "spark");
    assert!(use_it(&mut sim, who, "spark", "zap").unwrap());
    advance(&mut sim, Dur::from_secs(2));
    let refused = use_it(&mut sim, who, "spark", "zap").unwrap_err();
    assert_eq!(refused.code, codes::COOLING_DOWN);
    let (mut loaded, _) = common::save_and_load(&sim);
    assert_eq!(loaded.state_hash(), sim.state_hash());
    let refused = use_it(&mut loaded, who, "spark", "zap").unwrap_err();
    assert_eq!(refused.code, codes::COOLING_DOWN);
    // Twice as fast a clock: the rest goes by in half the time.
    loaded
        .set_time_rate(who, Some(Real::of(2.0)), cause())
        .unwrap();
    advance(&mut loaded, Dur::from_millis(1_400));
    assert_eq!(
        use_it(&mut loaded, who, "spark", "zap").unwrap_err().code,
        codes::COOLING_DOWN
    );
    advance(&mut loaded, Dur::from_millis(200));
    assert!(use_it(&mut loaded, who, "spark", "zap").unwrap());
}

#[test]
fn a_use_by_command_is_logged_and_replays_to_the_same_hash() {
    let mut first = sim();
    first
        .submit(Command::Debug(DebugCommand::Spawn {
            spec: CharacterSpec::of(["core:power_sampler"]),
            place: common::on(10, 10),
        }))
        .unwrap();
    let who = first.truth().entities().ids().last().unwrap();
    first
        .submit(Command::Debug(DebugCommand::SetControl {
            entity: who,
            control: Control::Direct,
        }))
        .unwrap();
    first
        .submit(Command::UseAbility {
            entity: who,
            ability: ability_id("dash"),
            action: None,
            params: Params::new(),
        })
        .unwrap();
    first
        .submit(Command::SetMode {
            entity: who,
            ability: ability_id("shield"),
            mode: "raised".to_owned(),
            on: true,
        })
        .unwrap();
    // A use refused before anything changed is not logged.
    let refused = first
        .submit(Command::UseAbility {
            entity: who,
            ability: ability_id("dash"),
            action: None,
            params: Params::new(),
        })
        .unwrap_err();
    assert_eq!(refused.reason.code, codes::COOLING_DOWN);
    first
        .submit(Command::Wait {
            entity: who,
            dur: Dur::from_secs(5),
        })
        .unwrap();
    first.advance(AdvanceLimit::Until(Tick::ZERO + Dur::from_secs(5)));
    let log = first.input_log().clone();
    let mut second = sim();
    second.replay(&log).unwrap();
    second.advance(AdvanceLimit::Until(Tick::ZERO + Dur::from_secs(5)));
    assert_eq!(second.state_hash(), first.state_hash());
}

// ---- A4: modes and charging ----------------------------------------------------------

#[test]
fn a_modes_trait_and_upkeep_begin_and_end_together() {
    let mut sim = sim();
    let who = spawn(&mut sim, "tester", 10, 10);
    sim.pump();
    grant(&mut sim, who, "spark");
    let rate = |sim: &Sim| {
        sim.components()
            .get::<Stats>(who)
            .and_then(|stats| stats.pool_rate(pool("wellspring")))
            .unwrap()
    };
    ability::start_mode(&mut sim, who, ability_id("spark"), "aura", cause()).unwrap();
    sim.pump();
    assert!(has_trait(&sim, who, "lit"));
    assert_eq!(rate(&sim), Real::of(-4.0));
    let refused =
        ability::start_mode(&mut sim, who, ability_id("spark"), "aura", cause()).unwrap_err();
    assert_eq!(refused.code, codes::MODE_ON);
    ability::end_mode(&mut sim, who, ability_id("spark"), "aura", cause()).unwrap();
    sim.pump();
    assert!(!has_trait(&sim, who, "lit"));
    assert_eq!(rate(&sim), Real::ZERO);
    // Revoked, it ends both too.
    ability::start_mode(&mut sim, who, ability_id("spark"), "aura", cause()).unwrap();
    ability::revoke(&mut sim, who, ability_id("spark"), cause()).unwrap();
    sim.pump();
    assert!(!has_trait(&sim, who, "lit"));
    assert_eq!(rate(&sim), Real::ZERO);
}

#[test]
fn a_mode_ends_the_tick_its_pool_runs_dry_with_nothing_between() {
    let mut sim = sim();
    let who = spawn(&mut sim, "tester", 10, 10);
    sim.pump();
    grant(&mut sim, who, "spark");
    advance(&mut sim, Dur::from_secs(1));
    sim.pool_spend(who, pool("wellspring"), Real::of(1.0), cause())
        .unwrap();
    sim.drain_events();
    let started = sim.now();
    ability::start_mode(&mut sim, who, ability_id("spark"), "aura", cause()).unwrap();
    sim.pump();
    // 99 at 4 a second: 24.75 seconds.
    let due = started + Dur::from_micros(24_750_000);
    let wakes = sim.truth().wakes_of(who);
    assert_eq!(wakes.len(), 1, "one wake-up for the whole of it: {wakes:?}");
    assert_eq!(sim.timeline().due(wakes[0]), Some(due));
    sim.advance(AdvanceLimit::Until(due - Dur::from_micros(1)));
    assert!(has_trait(&sim, who, "lit"));
    let steps = sim.steps();
    sim.advance(AdvanceLimit::Until(due));
    assert_eq!(sim.steps(), steps + 1, "nothing else ran on the way");
    assert!(!has_trait(&sim, who, "lit"));
    let events = sim.drain_events();
    let ended = events
        .iter()
        .find(|event| event.name() == ability::MODE_ENDED)
        .unwrap();
    assert_eq!(ended.tick, due);
    assert_eq!(text(ended, "how"), "empty");
    assert_eq!(level(&sim, who, "wellspring"), Real::ZERO);
}

#[test]
fn charge_delivered_follows_time_held() {
    let mut sim = sim();
    let who = spawn(&mut sim, "tester", 10, 10);
    sim.pump();
    grant(&mut sim, who, "spark");
    for (held, charge) in [(1_000, 4), (2_000, 8)] {
        assert!(use_it(&mut sim, who, "spark", "gather").unwrap());
        advance(&mut sim, Dur::from_millis(held));
        let running = *sim.actions_of(who).first().unwrap();
        sim.cancel_action(who, running, false, cause()).unwrap();
        sim.pump();
        assert_eq!(var(&sim, who, "last_charge"), Value::Int(charge));
    }
    let _ = names_of(&[]);
}

// ---- The bundled powers ------------------------------------------------------------

/// Evaluates an expression with someone as `self`.
fn eval_of(sim: &Sim, who: EntityId, source: &str) -> Value {
    sim.eval(
        &compile(sim, source),
        clash_sim::Bound::new().subject(sim.subject_value(Subject::Entity(who))),
    )
}

/// Evaluates an expression with a cell as `here`.
fn eval_at(sim: &Sim, x: i32, y: i32, source: &str) -> Value {
    let registry = sim.defs().expr_registry();
    let loc = registry.kind_id("loc").unwrap();
    let context = clash_expr::Context::new().subject("here", clash_expr::Type::Subject(loc));
    let code = clash_expr::compile(source, registry, &context, &clash_expr::Type::Any)
        .into_result()
        .unwrap();
    sim.eval(
        &code,
        clash_sim::Bound::new().subject(sim.subject_value(Subject::Loc(common::at(x, y)))),
    )
}

fn set_cell(
    sim: &mut Sim,
    x: i32,
    y: i32,
    layer: clash_world::Layer,
    state: clash_world::ElementState,
) {
    sim.submit(Command::Debug(DebugCommand::SetElement {
        loc: common::at(x, y),
        layer,
        state,
    }))
    .unwrap();
}

/// Uses one of the sampler's powers toward a cell.
fn power_at(sim: &mut Sim, who: EntityId, name: &str, x: i32, y: i32) -> bool {
    let to = sim
        .subject_value(Subject::Loc(common::at(x, y)))
        .to_stored();
    let started = ability::use_ability(
        sim,
        who,
        ability_id(name),
        None,
        &vec![("to".to_owned(), to)],
        cause(),
    )
    .unwrap_or_else(|reason| panic!("{reason:?}"));
    sim.pump();
    started.is_some()
}

#[test]
fn dash_makes_its_user_quicker_for_a_while_at_a_price() {
    let mut sim = sim();
    let who = spawn(&mut sim, "power_sampler", 10, 10);
    sim.pump();
    let speed = |sim: &Sim| clash_sim::movement::mover_rules(sim, who).best_speed();
    let walking = speed(&sim);
    assert!(walking > Real::ZERO);
    let started = ability::use_ability(
        &mut sim,
        who,
        ability_id("dash"),
        None,
        &Params::new(),
        cause(),
    )
    .unwrap();
    assert!(started.is_some());
    advance(&mut sim, Dur::from_millis(200));
    assert!(has_trait(&sim, who, "dashing"));
    let stamina = level(&sim, who, "stamina");
    assert!(
        (stamina - Real::of(75.2)).abs() < Real::of(1.0e-6),
        "{stamina}"
    );
    let dashing = speed(&sim);
    assert!(
        dashing > walking * Real::of(2.9),
        "{dashing} against {walking}"
    );
    let refused = ability::available(&sim, who, ability_id("dash"), None).unwrap_err();
    assert_eq!(refused.code, codes::COOLING_DOWN);
    // Rank 1: three seconds of it.
    advance(&mut sim, Dur::from_secs(3));
    assert!(!has_trait(&sim, who, "dashing"));
    assert_eq!(speed(&sim), walking);
}

#[test]
fn force_push_throws_people_into_walls_and_hurts_them() {
    let mut sim = sim();
    let wall = common::wall(&sim).raw();
    for y in 7..=13 {
        set_cell(&mut sim, 16, y, clash_world::Layer::Block, wall);
    }
    let user = spawn(&mut sim, "power_sampler", 10, 10);
    let target = spawn(&mut sim, "person", 12, 10);
    sim.pump();
    sim.drain_events();
    assert!(power_at(&mut sim, user, "force_push", 12, 10));
    assert_eq!(level(&sim, user, "reserve"), Real::of(70.0));
    advance(&mut sim, Dur::from_secs(4));
    let events = sim.drain_events();
    let thrown = events.iter().any(|event| {
        event.name() == "body_launched" && event.subject("target") == Some(Subject::Entity(target))
    });
    assert!(thrown, "{:?}", names_of(&events));
    let struck = events.iter().any(|event| {
        event.name() == "body_impact"
            && event.subject("target") == Some(Subject::Entity(target))
            && text(event, "layer") == "block"
    });
    assert!(
        struck,
        "the person strikes the wall: {:?}",
        names_of(&events)
    );
    let wounds = eval_of(&sim, target, "len(self.wounds)").as_int().unwrap();
    assert!(wounds > 0, "and is hurt");
    assert_eq!(
        sim.truth().location(user),
        Some(common::at(10, 10)),
        "the cone goes away from its user"
    );
}

#[test]
fn firebolt_sets_a_fire_that_spreads() {
    let mut sim = sim();
    let boards = sim
        .truth()
        .cosmos()
        .defs()
        .floor_state("boards", None)
        .unwrap()
        .raw();
    for x in 13..=22 {
        set_cell(&mut sim, x, 10, clash_world::Layer::Floor, boards);
    }
    let user = spawn(&mut sim, "power_sampler", 10, 10);
    sim.pump();
    assert!(power_at(&mut sim, user, "firebolt", 15, 10));
    // Held to its end: three seconds, twelve charges.
    advance(&mut sim, Dur::from_millis(3_100));
    let set = eval_at(&sim, 15, 10, "here.field('core:fire') ?? 0")
        .as_int()
        .unwrap();
    assert!(set > 200, "{set}");
    let mut spread = false;
    for _ in 0..40 {
        advance(&mut sim, Dur::from_secs(1));
        spread |= [13, 14, 16, 17].iter().any(|x| {
            eval_at(&sim, *x, 10, "here.field('core:fire') ?? 0")
                .as_int()
                .unwrap_or(0)
                > 0
        });
    }
    assert!(spread, "the fire spreads along the boards");
    // One use fed the track a point: not yet a level.
    assert_eq!(
        eval_of(&sim, user, "self.track.pyromancy").as_int(),
        Some(0)
    );
}

#[test]
fn shield_drains_its_pool_while_on_and_stops_when_it_is_empty() {
    let mut sim = sim();
    let who = spawn(&mut sim, "power_sampler", 10, 10);
    sim.pump();
    let started = sim.now();
    ability::start_mode(&mut sim, who, ability_id("shield"), "raised", cause()).unwrap();
    sim.pump();
    assert!(has_trait(&sim, who, "shielded"));
    advance(&mut sim, Dur::from_secs(10));
    // Five a second against two of regeneration.
    let left = level(&sim, who, "reserve");
    assert!((left - Real::of(70.0)).abs() < Real::of(0.01), "{left}");
    sim.drain_events();
    advance(&mut sim, Dur::from_secs(30));
    assert!(!has_trait(&sim, who, "shielded"));
    let events = sim.drain_events();
    let ended = events
        .iter()
        .find(|event| event.name() == ability::MODE_ENDED)
        .unwrap();
    assert_eq!(text(ended, "how"), "empty");
    assert_eq!(ended.tick, started + Dur::from_micros(33_333_334));
    // And the pool fills again.
    advance(&mut sim, Dur::from_secs(5));
    assert!(level(&sim, who, "reserve") > Real::of(9.0));
}

#[test]
fn heal_closes_the_wounds_of_someone_touched() {
    let mut sim = sim();
    let healer = spawn(&mut sim, "power_sampler", 10, 10);
    let hurt = spawn(&mut sim, "person", 11, 10);
    sim.pump();
    let registry = Arc::clone(sim.defs().expr_registry());
    let damage = registry.def_type_id("damage_type").unwrap();
    let cut = registry.def_ref(damage, "core:cut").unwrap();
    sim.call_verb(
        "hit",
        &[
            ("target", sim.subject_value(Subject::Entity(hurt))),
            ("kind", Value::Id(cut)),
            ("energy", Value::Real(Real::of(150.0))),
            ("intensity", Value::Real(Real::of(2.0e7))),
        ],
        &[],
        cause(),
    )
    .unwrap();
    sim.pump();
    let open = |sim: &Sim| {
        sim.components()
            .get::<clash_sim::body::Body>(hurt)
            .map_or(0, |body| {
                body.wounds()
                    .iter()
                    .filter(|wound| wound.state() == clash_sim::body::WoundState::Open)
                    .count()
            })
    };
    assert!(open(&sim) > 0, "the cut leaves a wound");
    let target = sim.subject_value(Subject::Entity(hurt)).to_stored();
    let started = ability::use_ability(
        &mut sim,
        healer,
        ability_id("heal"),
        None,
        &vec![("target".to_owned(), target)],
        cause(),
    )
    .unwrap();
    assert!(started.is_some());
    advance(&mut sim, Dur::from_millis(1_200));
    assert_eq!(open(&sim), 0, "every open wound is closed");
    let reserve = level(&sim, healer, "reserve");
    assert!(
        (reserve - Real::of(77.4)).abs() < Real::of(1.0e-6),
        "{reserve}"
    );
}

#[test]
fn a_power_too_dear_is_refused_with_what_is_short() {
    let mut sim = sim();
    let who = spawn(&mut sim, "power_sampler", 10, 10);
    sim.pump();
    sim.pool_spend(who, pool("reserve"), Real::of(90.0), cause())
        .unwrap();
    let to = sim
        .subject_value(Subject::Loc(common::at(12, 10)))
        .to_stored();
    let refused = ability::use_ability(
        &mut sim,
        who,
        ability_id("force_push"),
        None,
        &vec![("to".to_owned(), to)],
        cause(),
    )
    .unwrap_err();
    assert_eq!(refused.code, codes::COST_SHORT);
    assert!(refused.message.contains("20 short"), "{}", refused.message);
    // Every option says the same, and what it costs.
    let options = ability::options(&sim, who);
    let push = options
        .iter()
        .find(|option| option.ability == ability_id("force_push"))
        .unwrap();
    assert_eq!(
        push.unavailable.as_ref().map(|reason| reason.code.as_str()),
        Some(codes::COST_SHORT)
    );
    assert_eq!(push.cost[0].what, "core:reserve");
    assert_eq!(push.cost[0].amount, Real::of(30.0));
    assert_eq!(options.len(), 4, "the shield has no grant, only a mode");
}
