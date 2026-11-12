//! Tests of what abilities leave in and take from the world (08 §6,
//! §7.2, §7.3; packages A5 to A7) against the real simulation: the
//! world, life, traits, movement, perception and minds of `mods/core`,
//! its bundled powers (`mods/core/data/powers`), and a few definitions
//! of the tests' own.
//!
//! | Package | Done when | Test |
//! |---------|-----------|------|
//! | A5 | A dormant or suppressed ability confers nothing (A2) and returns intact | `a_suppressed_ability_confers_nothing_and_returns_intact` |
//! | A5 | Entering a suppressing area ends a running mode; leaving restores the ability, not the mode | `a_null_area_ends_a_mode_and_leaving_it_restores_the_ability` |
//! | A5 | A rule on `ability_suppressing` resists | `a_rule_on_ability_suppressing_resists` |
//! | A5 | Suppression by a trait, and for a time | `a_trait_suppresses_while_it_lasts`, `a_timed_suppression_lifts_on_its_owners_time` |

mod common;

use std::path::PathBuf;
use std::sync::{Arc, LazyLock};

use clash_core::{Dur, EntityId, Real};
use clash_data::{Defs, Loader};
use clash_expr::Value;
use clash_sim::ability::{
    self, Abilities, AbilityDef, AbilityId, AbilitySource, State, Suppressor, codes,
};
use clash_sim::action::{ActionDef, ActionDefId, Params};
use clash_sim::stats::{AttributeDef, PoolDef, PoolId, Stats};
use clash_sim::template::CharacterSpec;
use clash_sim::traits::{self, Giver, TraitDef, TraitId};
use clash_sim::{AdvanceLimit, Place, Sim, SimEvent, Subject};
use clash_world::{AreaId, AreaShape, AreaSpec, Cause, CellBox, CellPos, Location};

/// Definitions of the tests' own, in the namespace `core`.
const TEST_DEFS: &str = r#"[
  { "type": "pool", "id": "mana", "max": 100, "start": "max" },
  { "type": "variable", "id": "stubborn", "scope": "entity", "value": "bool", "default": false, "doc": "Whether a test's character resists suppression." },
  { "type": "trait", "id": "glowing", "tags": ["glowing"], "modifiers": [ { "attr": "strength", "add": 2 } ] },
  { "type": "trait", "id": "lit", "tags": ["lit"] },
  { "type": "trait", "id": "dampened", "suppresses": ["test"] },
  { "type": "action", "id": "zap", "params": { "energy": "real" },
    "phases": [ { "id": "go", "time": "100 ms" } ] },
  { "type": "ability", "id": "spark", "tags": ["test"],
    "params": { "power": "10 * ability.rank" },
    "always": { "traits": ["glowing"] },
    "grants": [ { "action": "zap", "params": { "energy": "ability.power" },
                  "cost": { "mana": 10 }, "cooldown": "5 s" } ],
    "modes": [ { "id": "aura", "trait": "lit", "upkeep": { "mana": 1 } } ] },
  { "type": "ability", "id": "knack", "tags": ["other"] },
  { "type": "area_kind", "id": "null_zone", "suppresses": ["test"] },
  { "type": "eoc", "id": "stubborn_gift", "on": "ability_suppressing", "if": "self.var.stubborn",
    "do": [ { "cancel_event": {} } ] },
  { "type": "template", "id": "tester", "copy_from": "person",
    "pools": ["blood", "pain", "stamina", "reserve", "mana"] }
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

/// Loads `mods/core`'s world, life, traits, movement, combat,
/// perception, minds and powers with more definitions, or renders why
/// not.
fn try_load(extra: &str) -> Result<Arc<Defs>, String> {
    let types = common::types();
    let mut files = dir_files("world");
    for sub in ["life", "powers", "ai", "perception", "combat"] {
        files.extend(dir_files(sub));
    }
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
    common::sim_over(Arc::clone(&DEFS), 31)
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
    let who = sim
        .spawn_template(
            &CharacterSpec::of([format!("core:{template}")]),
            common::on(x, y),
            cause(),
        )
        .unwrap_or_else(|reason| panic!("{reason:?}"))
        .entity;
    sim.pump();
    who
}

fn level(sim: &Sim, who: EntityId, name: &str) -> Real {
    sim.components()
        .get::<Stats>(who)
        .and_then(|stats| stats.pool_level(sim.timeline(), pool(name)))
        .unwrap()
}

fn strength(sim: &Sim, who: EntityId) -> Real {
    let attr = DEFS.lookup::<AttributeDef>("core:strength").unwrap();
    sim.components()
        .get::<Stats>(who)
        .and_then(|stats| stats.attr(attr))
        .unwrap()
}

fn has_trait(sim: &Sim, who: EntityId, name: &str) -> bool {
    sim.components()
        .get::<traits::Traits>(who)
        .is_some_and(|traits| traits.has(trait_id(name)))
}

fn inst<'a>(sim: &'a Sim, who: EntityId, name: &str) -> &'a ability::AbilityInst {
    sim.components()
        .get::<Abilities>(who)
        .and_then(|abilities| abilities.of(ability_id(name)))
        .unwrap()
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

fn text(event: &SimEvent, name: &str) -> String {
    match event.body.field(name) {
        Some(clash_sim::FieldValue::Plain(clash_core::Value::Str(text))) => text.to_string(),
        other => panic!("`{name}` is {other:?}"),
    }
}

fn compile(sim: &Sim, source: &str) -> clash_expr::Compiled {
    let registry = sim.defs().expr_registry();
    let entity = registry.kind_id("entity").unwrap();
    let context = clash_expr::Context::new().subject("self", clash_expr::Type::Subject(entity));
    clash_expr::compile(source, registry, &context, &clash_expr::Type::Any)
        .into_result()
        .unwrap()
}

/// Evaluates an expression with someone as `self`.
fn eval_of(sim: &Sim, who: EntityId, source: &str) -> Value {
    sim.eval(
        &compile(sim, source),
        clash_sim::Bound::new().subject(sim.subject_value(Subject::Entity(who))),
    )
}

fn entity_value(sim: &Sim, who: EntityId) -> Value {
    sim.subject_value(Subject::Entity(who))
}

fn texts(items: &[&str]) -> Value {
    Value::list(items.iter().map(|item| Value::from(*item)))
}

/// An area of a kind over a box of storey 0.
fn area_of(sim: &mut Sim, kind: &str, min: (i32, i32), max: (i32, i32)) -> AreaId {
    let kind = sim
        .truth()
        .cosmos()
        .defs()
        .area_kinds()
        .resolve(&format!("core:{kind}"), None)
        .unwrap();
    let mut spec = AreaSpec::new(
        common::GRID,
        AreaShape::Box(CellBox::new(
            CellPos::new(min.0, min.1, 0),
            CellPos::new(max.0, max.1, 0),
        )),
    );
    spec.kind = Some(kind);
    let id = sim.world(|ctx| ctx.create_area(spec, cause())).unwrap();
    sim.pump();
    id
}

fn step_to(sim: &mut Sim, who: EntityId, x: i32, y: i32) {
    sim.relocate(who, common::on(x, y), cause()).unwrap();
    sim.pump();
}

// ---- A5: dormancy, suppression, loss ------------------------------------------

#[test]
fn a_suppressed_ability_confers_nothing_and_returns_intact() {
    let mut sim = sim();
    let who = spawn(&mut sim, "tester", 10, 10);
    grant(&mut sim, who, "spark");
    ability::set_rank(&mut sim, who, ability_id("spark"), 3, cause()).unwrap();
    sim.pump();
    let base = strength(&sim, who) - Real::of(2.0);
    assert!(has_trait(&sim, who, "glowing"));
    assert!(use_it(&mut sim, who, "spark", "zap").unwrap());
    advance(&mut sim, Dur::from_millis(500));
    let cooldown = inst(&sim, who, "spark").cooldowns().to_vec();
    assert_eq!(cooldown.len(), 1);
    sim.drain_events();

    let count = sim
        .call_verb(
            "suppress_abilities",
            &[("who", entity_value(&sim, who)), ("tags", texts(&["test"]))],
            &[],
            cause(),
        )
        .unwrap();
    sim.pump();
    assert_eq!(count, Value::Int(1));
    // Invariant A2: nothing conferred, nothing usable, no mode.
    assert!(!has_trait(&sim, who, "glowing"));
    assert_eq!(strength(&sim, who), base);
    assert!(inst(&sim, who, "spark").is_suppressed());
    assert_eq!(inst(&sim, who, "spark").state(), State::Active);
    let refused = use_it(&mut sim, who, "spark", "zap");
    assert!(matches!(refused, Ok(false) | Err(_)));
    let reason =
        ability::available(&sim, who, ability_id("spark"), Some(action("zap"))).unwrap_err();
    assert_eq!(reason.code, codes::SUPPRESSED);
    let reason =
        ability::start_mode(&mut sim, who, ability_id("spark"), "aura", cause()).unwrap_err();
    assert_eq!(reason.code, codes::SUPPRESSED);
    let state = eval_of(&sim, who, "self.ability('core:spark').state");
    assert_eq!(state, Value::from("suppressed"));
    let events = sim.drain_events();
    assert!(names_of(&events).contains(&ability::ABILITY_SUPPRESSED));
    // Another tag is not touched.
    grant(&mut sim, who, "knack");
    assert!(!inst(&sim, who, "knack").is_suppressed());

    let lifted = sim
        .call_verb(
            "restore_abilities",
            &[("who", entity_value(&sim, who))],
            &[],
            cause(),
        )
        .unwrap();
    sim.pump();
    assert_eq!(lifted, Value::Int(1));
    // Intact: its rank, its cooldown, what it confers.
    assert!(!inst(&sim, who, "spark").is_suppressed());
    assert!(has_trait(&sim, who, "glowing"));
    assert_eq!(strength(&sim, who), base + Real::of(2.0));
    assert_eq!(ability::rank(&sim, who, inst(&sim, who, "spark")), 3);
    assert_eq!(inst(&sim, who, "spark").cooldowns(), cooldown.as_slice());
    let events = sim.drain_events();
    assert!(names_of(&events).contains(&ability::ABILITY_RESTORED));
}

#[test]
fn a_null_area_ends_a_mode_and_leaving_it_restores_the_ability() {
    let mut sim = sim();
    let who = spawn(&mut sim, "tester", 10, 10);
    grant(&mut sim, who, "spark");
    let zone = area_of(&mut sim, "null_zone", (14, 8), (18, 12));
    ability::start_mode(&mut sim, who, ability_id("spark"), "aura", cause()).unwrap();
    sim.pump();
    assert!(has_trait(&sim, who, "lit"));
    let rate = |sim: &Sim| {
        sim.components()
            .get::<Stats>(who)
            .and_then(|stats| stats.pool_rate(pool("mana")))
            .unwrap()
    };
    assert!(rate(&sim) < Real::ZERO);
    sim.drain_events();

    step_to(&mut sim, who, 15, 10);
    let events = sim.drain_events();
    let ended = events
        .iter()
        .find(|event| event.name() == ability::MODE_ENDED)
        .expect("the mode ends");
    assert_eq!(text(ended, "how"), "suppressed");
    let suppressed = events
        .iter()
        .find(|event| event.name() == ability::ABILITY_SUPPRESSED)
        .unwrap();
    assert_eq!(text(suppressed, "by"), "area");
    assert!(!has_trait(&sim, who, "lit"));
    assert!(!has_trait(&sim, who, "glowing"));
    assert_eq!(rate(&sim), Real::ZERO);
    assert_eq!(
        inst(&sim, who, "spark").suppressed_by(),
        &[Suppressor::Area(zone)]
    );

    // Save and load inside the area changes nothing.
    let hash = sim.state_hash();
    let (mut loaded, unplaced) = common::save_and_load(&sim);
    assert!(unplaced.is_empty());
    loaded.pump();
    assert_eq!(loaded.state_hash(), hash);

    step_to(&mut loaded, who, 10, 10);
    let events = loaded.drain_events();
    assert!(names_of(&events).contains(&ability::ABILITY_RESTORED));
    assert!(!names_of(&events).contains(&ability::MODE_STARTED));
    assert!(has_trait(&loaded, who, "glowing"));
    assert!(!has_trait(&loaded, who, "lit"));
    assert!(!inst(&loaded, who, "spark").mode_is_on(0));

    // The area coming over its owner does the same as the owner going in.
    loaded
        .world(|ctx| ctx.move_area(zone, clash_core::IVec3::new(-5, 0, 0), cause()))
        .unwrap();
    loaded.pump();
    assert!(inst(&loaded, who, "spark").is_suppressed());
    loaded.world(|ctx| ctx.remove_area(zone, cause())).unwrap();
    loaded.pump();
    assert!(!inst(&loaded, who, "spark").is_suppressed());
}

#[test]
fn a_rule_on_ability_suppressing_resists() {
    let mut sim = sim();
    let who = spawn(&mut sim, "tester", 10, 10);
    grant(&mut sim, who, "spark");
    sim.set_var(
        Subject::Entity(who),
        "core:stubborn",
        Value::Bool(true),
        cause(),
    )
    .unwrap();
    area_of(&mut sim, "null_zone", (14, 8), (18, 12));
    ability::start_mode(&mut sim, who, ability_id("spark"), "aura", cause()).unwrap();
    sim.pump();
    sim.drain_events();
    step_to(&mut sim, who, 15, 10);
    let events = sim.drain_events();
    // The hook was cancelled: the notice says so, and nothing was
    // suppressed.
    assert!(
        events
            .iter()
            .any(|event| matches!(&event.body, clash_sim::EventBody::EventCancelled { event, .. } if event == ability::ABILITY_SUPPRESSING)),
        "{:?}",
        names_of(&events)
    );
    assert!(!names_of(&events).contains(&ability::ABILITY_SUPPRESSED));
    assert!(!inst(&sim, who, "spark").is_suppressed());
    assert_eq!(inst(&sim, who, "spark").resisted().len(), 1);
    assert!(has_trait(&sim, who, "lit"));
    // A verb's suppression is resisted the same way.
    let count = sim
        .call_verb(
            "suppress_abilities",
            &[("who", entity_value(&sim, who))],
            &[],
            cause(),
        )
        .unwrap();
    assert_eq!(count, Value::Int(0));
    // Without the will to resist, the next one takes hold.
    sim.set_var(
        Subject::Entity(who),
        "core:stubborn",
        Value::Bool(false),
        cause(),
    )
    .unwrap();
    sim.call_verb(
        "suppress_abilities",
        &[("who", entity_value(&sim, who))],
        &[],
        cause(),
    )
    .unwrap();
    sim.pump();
    assert!(inst(&sim, who, "spark").is_suppressed());
    assert!(!has_trait(&sim, who, "lit"));
}

#[test]
fn a_trait_suppresses_while_it_lasts() {
    let mut sim = sim();
    let who = spawn(&mut sim, "tester", 10, 10);
    grant(&mut sim, who, "spark");
    traits::add_trait(
        &mut sim,
        who,
        trait_id("dampened"),
        Giver::RULE,
        Some(Dur::from_secs(3)),
        cause(),
    )
    .unwrap();
    sim.pump();
    assert!(inst(&sim, who, "spark").is_suppressed());
    assert!(!has_trait(&sim, who, "glowing"));
    // An ability gained while it lasts is suppressed from the start.
    ability::revoke(&mut sim, who, ability_id("spark"), cause()).unwrap();
    sim.pump();
    grant(&mut sim, who, "spark");
    assert!(inst(&sim, who, "spark").is_suppressed());
    assert!(!has_trait(&sim, who, "glowing"));
    advance(&mut sim, Dur::from_secs(4));
    assert!(!has_trait(&sim, who, "dampened"));
    assert!(!inst(&sim, who, "spark").is_suppressed());
    assert!(has_trait(&sim, who, "glowing"));
}

#[test]
fn a_timed_suppression_lifts_on_its_owners_time() {
    let mut sim = sim();
    let who = spawn(&mut sim, "tester", 10, 10);
    grant(&mut sim, who, "spark");
    sim.call_verb(
        "suppress_abilities",
        &[
            ("who", entity_value(&sim, who)),
            ("for", Value::Real(Real::of(30.0))),
        ],
        &[],
        cause(),
    )
    .unwrap();
    sim.pump();
    assert!(inst(&sim, who, "spark").is_suppressed());
    advance(&mut sim, Dur::from_secs(20));
    let hash = sim.state_hash();
    let (mut loaded, _) = common::save_and_load(&sim);
    loaded.pump();
    assert_eq!(loaded.state_hash(), hash);
    advance(&mut loaded, Dur::from_secs(9));
    assert!(inst(&loaded, who, "spark").is_suppressed());
    advance(&mut loaded, Dur::from_secs(2));
    assert!(!inst(&loaded, who, "spark").is_suppressed());
    assert!(has_trait(&loaded, who, "glowing"));
    // Out of the world its own time stands still, and so does the
    // suppression.
    loaded
        .call_verb(
            "suppress_abilities",
            &[
                ("who", entity_value(&loaded, who)),
                ("for", Value::Real(Real::of(5.0))),
            ],
            &[],
            cause(),
        )
        .unwrap();
    loaded.pump();
    loaded.relocate(who, Place::Stored, cause()).unwrap();
    loaded.pump();
    advance(&mut loaded, Dur::from_secs(20));
    loaded.relocate(who, common::on(10, 10), cause()).unwrap();
    loaded.pump();
    assert!(inst(&loaded, who, "spark").is_suppressed());
    advance(&mut loaded, Dur::from_secs(6));
    assert!(!inst(&loaded, who, "spark").is_suppressed());
    // A refusal names its reason.
    let reason = loaded
        .call_verb(
            "suppress_abilities",
            &[
                ("who", entity_value(&loaded, who)),
                ("for", Value::Real(Real::of(-1.0))),
            ],
            &[],
            cause(),
        )
        .unwrap_err();
    assert_eq!(reason.code, codes::BAD_DURATION);
    let _ = level(&loaded, who, "mana");
    let _ = Location::new(common::GRID, CellPos::new(0, 0, 0));
}
