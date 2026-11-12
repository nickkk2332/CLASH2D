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
//! | A6 | Moving an anchored area raises the events of the others moving the other way | `moving_an_anchored_area_is_moving_the_others_the_other_way` (property test), `an_anchored_area_follows_its_anchor_and_the_anchor_never_leaves_it` |
//! | A6 | A body falling through weightlessness stops accelerating | `a_body_falling_through_weightlessness_stops_gaining_speed_inside_it` |
//! | A6 | An anchored area with a clock rehomes what it comes over | `an_anchored_clock_area_stops_what_it_comes_over_and_not_its_maker` |
//! | A6 | Light is never negative | `darkness_lowers_light_and_never_below_zero` |
//! | A6 | `create_area` with an anchor, gravity and a light | `create_area_takes_an_anchor_gravity_and_a_light` |

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
}

// ---- A6: anchored areas and ambient overrides ------------------------------------

/// A plain occupant standing at a cell.
fn stone(sim: &mut Sim, x: i32, y: i32) -> EntityId {
    let id = common::spawn(sim, common::item(), common::on(x, y));
    sim.pump();
    id
}

/// An area of no kind round a cell, anchored to an entity, with what a
/// test sets on it.
fn bubble(sim: &mut Sim, anchor: EntityId, radius: f64, set: impl FnOnce(&mut AreaSpec)) -> AreaId {
    let at = sim.truth().location(anchor).unwrap();
    let mut spec = AreaSpec::new(
        common::GRID,
        AreaShape::Radius {
            centre: at.cell,
            radius: Real::of(radius),
        },
    );
    spec.anchor = Some(clash_world::AreaAnchor::Entity(anchor));
    set(&mut spec);
    let id = sim.world(|ctx| ctx.create_area(spec, cause())).unwrap();
    sim.pump();
    id
}

/// The entering and leaving of an area the events report, by occupant,
/// sorted: `true` for entering.
fn crossings(events: &[SimEvent], area: AreaId) -> Vec<(EntityId, bool)> {
    let mut out: Vec<(EntityId, bool)> = events
        .iter()
        .filter_map(|event| match &event.body {
            clash_sim::EventBody::World(clash_world::WorldEvent::AreaEntered {
                area: at,
                occupant,
                ..
            }) if *at == area => Some((*occupant, true)),
            clash_sim::EventBody::World(clash_world::WorldEvent::AreaExited {
                area: at,
                occupant,
                ..
            }) if *at == area => Some((*occupant, false)),
            _ => None,
        })
        .collect();
    out.sort();
    out
}

proptest::proptest! {
    #![proptest_config(proptest::prelude::ProptestConfig::with_cases(24))]

    /// Moving an anchored area raises the same events as its occupants
    /// moving the other way (02 §8.3).
    #[test]
    fn moving_an_anchored_area_is_moving_the_others_the_other_way(
        others in proptest::collection::vec((22i32..42, 22i32..42), 1..8),
        moves in proptest::collection::vec((-4i32..=4, -4i32..=4), 1..5),
        radius in 1.0f64..6.0,
    ) {
        let mut carried = sim();
        let anchor = stone(&mut carried, 32, 32);
        let placed: Vec<EntityId> = others.iter().map(|(x, y)| stone(&mut carried, *x, *y)).collect();
        let area = bubble(&mut carried, anchor, radius, |_| {});
        // The same world, the area left where it is.
        let mut still = carried.clone();
        still.world(|ctx| ctx.cosmos.set_area_anchor(area, None)).unwrap();
        let mut at = (32, 32);
        let mut shifted = (0, 0);
        for (dx, dy) in moves {
            at = (at.0 + dx, at.1 + dy);
            shifted = (shifted.0 + dx, shifted.1 + dy);
            carried.drain_events();
            step_to(&mut carried, anchor, at.0, at.1);
            let by_area = crossings(&carried.drain_events(), area);
            still.drain_events();
            for (who, (x, y)) in placed.iter().zip(&others) {
                still.relocate(*who, common::on(x - shifted.0, y - shifted.1), cause()).unwrap();
            }
            still.pump();
            let by_others = crossings(&still.drain_events(), area);
            proptest::prop_assert_eq!(by_area, by_others);
        }
    }
}

#[test]
fn an_anchored_area_follows_its_anchor_and_the_anchor_never_leaves_it() {
    let mut sim = sim();
    let who = spawn(&mut sim, "tester", 30, 30);
    let other = stone(&mut sim, 34, 30);
    let area = bubble(&mut sim, who, 2.0, |_| {});
    sim.drain_events();
    step_to(&mut sim, who, 33, 30);
    let events = sim.drain_events();
    assert_eq!(crossings(&events, area), vec![(other, true)]);
    let centre = sim.truth().cosmos().area(area).unwrap().shape.centre();
    assert_eq!(centre, Some(CellPos::new(33, 30, 0)));
    step_to(&mut sim, who, 20, 30);
    assert_eq!(crossings(&sim.drain_events(), area), vec![(other, false)]);
    // Saved and loaded, it still follows.
    let (mut loaded, _) = common::save_and_load(&sim);
    loaded.pump();
    step_to(&mut loaded, who, 21, 31);
    let centre = loaded.truth().cosmos().area(area).unwrap().shape.centre();
    assert_eq!(centre, Some(CellPos::new(21, 31, 0)));
}

#[test]
fn a_body_falling_through_weightlessness_stops_gaining_speed_inside_it() {
    let mut sim = sim();
    // Weightless from storey 1 to storey 2, over a few cells.
    let mut spec = AreaSpec::new(
        common::GRID,
        AreaShape::Box(CellBox::new(
            CellPos::new(20, 20, 1),
            CellPos::new(24, 24, 2),
        )),
    );
    spec.ambient = Some(clash_world::AmbientOverride {
        gravity: Some(clash_world::GravityOverride::Scale(Real::ZERO)),
        drift: None,
    });
    sim.world(|ctx| ctx.create_area(spec, cause())).unwrap();
    let material = sim
        .truth()
        .cosmos()
        .defs()
        .materials()
        .resolve("core:granite", None)
        .unwrap();
    let mut body = clash_world::Body::of_mass(
        clash_world::BodyOwner::Debris(clash_world::Debris {
            material,
            volume: Real::of(0.001),
        }),
        common::GRID,
        [Real::of(22.5), Real::of(22.5), Real::of(14.0)],
        [Real::ZERO, Real::ZERO, Real::of(-1.0)],
        Real::of(2.0),
        cause(),
    );
    body.drag = Real::ZERO;
    let id = sim.world(|ctx| ctx.launch(body)).unwrap();
    let mut samples: Vec<(Real, Real)> = Vec::new();
    for _ in 0..200 {
        advance(&mut sim, Dur::from_millis(20));
        let Some(body) = sim.truth().cosmos().body(id) else {
            break;
        };
        samples.push((body.pos[2], body.vel[2]));
    }
    let storey = clash_world::dims::STOREY_M;
    let inside: Vec<Real> = samples
        .iter()
        .filter(|(z, _)| *z > storey + Real::of(0.5) && *z < storey * Real::of(3.0) - Real::of(0.5))
        .map(|(_, vz)| *vz)
        .collect();
    let above: Vec<Real> = samples
        .iter()
        .filter(|(z, _)| *z > storey * Real::of(3.0) + Real::of(0.2))
        .map(|(_, vz)| *vz)
        .collect();
    let below: Vec<Real> = samples
        .iter()
        .filter(|(z, _)| *z < storey - Real::of(0.2) && *z > Real::of(0.2))
        .map(|(_, vz)| *vz)
        .collect();
    assert!(inside.len() >= 3, "{samples:?}");
    assert!(above.len() >= 2 && below.len() >= 1, "{samples:?}");
    // It gains speed falling toward the area, keeps the speed it had
    // inside, and gains again below.
    assert!(above.windows(2).all(|pair| pair[1] < pair[0]), "{above:?}");
    let first = inside[0];
    assert!(
        inside
            .iter()
            .all(|vz| (*vz - first).abs() < Real::of(1.0e-9)),
        "{inside:?}"
    );
    assert!(below.iter().all(|vz| *vz < first), "{below:?} {first}");
}

#[test]
fn an_anchored_clock_area_stops_what_it_comes_over_and_not_its_maker() {
    let mut sim = sim();
    let maker = stone(&mut sim, 12, 40);
    let other = stone(&mut sim, 30, 40);
    let realm = sim
        .truth()
        .cosmos()
        .realm(common::realm(&sim))
        .unwrap()
        .clock;
    sim.anchor_clock(maker, Some(realm), cause()).unwrap();
    bubble(&mut sim, maker, 4.0, |spec| {
        spec.clock_rate = Some(Real::ZERO)
    });
    let other_time = |sim: &Sim| sim.truth().own_time(other);
    let before = other_time(&sim);
    advance(&mut sim, Dur::from_secs(2));
    assert!(other_time(&sim) > before);
    // The bubble comes over the other as its maker walks up.
    step_to(&mut sim, maker, 27, 40);
    let stopped = other_time(&sim);
    let maker_before = sim.truth().own_time(maker);
    advance(&mut sim, Dur::from_secs(5));
    assert_eq!(other_time(&sim), stopped);
    assert_eq!(
        sim.truth().own_time(maker) - maker_before,
        Dur::from_secs(5)
    );
    // Saved and loaded inside, it is still stopped.
    let hash = sim.state_hash();
    let (mut loaded, _) = common::save_and_load(&sim);
    loaded.pump();
    assert_eq!(loaded.state_hash(), hash);
    advance(&mut loaded, Dur::from_secs(1));
    assert_eq!(loaded.truth().own_time(other), stopped);
    // Walking away lets it go on.
    step_to(&mut loaded, maker, 12, 40);
    advance(&mut loaded, Dur::from_secs(1));
    assert!(loaded.truth().own_time(other) > stopped);
}

#[test]
fn darkness_lowers_light_and_never_below_zero() {
    let mut sim = sim();
    let who = stone(&mut sim, 30, 30);
    let lamp = stone(&mut sim, 31, 30);
    let lamp_far = stone(&mut sim, 50, 30);
    let light_at = |sim: &Sim, x: i32, y: i32| {
        let cosmos = sim.truth().cosmos();
        let time = clash_world::SkyTime::now(cosmos, sim.timeline(), common::GRID).unwrap();
        cosmos.light(common::at(x, y), time).unwrap()
    };
    // Two areas that carry light, one by the other far off.
    let bright = clash_world::LightSource::shining(Real::of(2.0), Some(Real::of(8.0)))
        .unwrap()
        .unwrap();
    bubble(&mut sim, lamp, 0.0, |spec| spec.light = Some(bright));
    bubble(&mut sim, lamp_far, 0.0, |spec| spec.light = Some(bright));
    let open = light_at(&sim, 30, 30);
    let far = light_at(&sim, 50, 30);
    assert!(open > Real::ZERO && far > Real::ZERO);
    let source = clash_world::LightSource::shining(Real::of(-50.0), Some(Real::of(6.0)))
        .unwrap()
        .unwrap();
    bubble(&mut sim, who, 0.0, |spec| spec.light = Some(source));
    let dark = light_at(&sim, 30, 30);
    assert!(dark < open);
    assert_eq!(
        dark,
        Real::ZERO,
        "a strong darkness puts out the light, and no more"
    );
    assert_eq!(
        light_at(&sim, 50, 30),
        far,
        "out of its reach nothing changes"
    );
    // A box filled at once agrees cell by cell, and nothing is negative.
    let cosmos = sim.truth().cosmos();
    let time = clash_world::SkyTime::now(cosmos, sim.timeline(), common::GRID).unwrap();
    let cells = CellBox::new(CellPos::new(24, 24, 0), CellPos::new(36, 36, 0));
    let mut filled = Vec::new();
    cosmos
        .fill_light(common::GRID, cells, time, &[], &mut filled)
        .unwrap();
    for (level, cell) in filled.iter().zip(cells.cells()) {
        assert!(*level >= Real::ZERO);
        let one = cosmos
            .light(Location::new(common::GRID, cell), time)
            .unwrap();
        assert!((one - *level).abs() < Real::of(1.0e-9), "{cell:?}");
    }
    // The darkness moves with its anchor.
    step_to(&mut sim, who, 50, 30);
    assert_eq!(light_at(&sim, 30, 30), open);
    assert!(light_at(&sim, 50, 30) < far);
}

#[test]
fn create_area_takes_an_anchor_gravity_and_a_light() {
    let mut sim = sim();
    let who = spawn(&mut sim, "tester", 30, 30);
    let made = sim
        .call_verb(
            "create_area",
            &[
                ("at", sim.subject_value(Subject::Loc(common::at(30, 30)))),
                ("radius", Value::Real(Real::of(3.0))),
                ("anchor", entity_value(&sim, who)),
                ("gravity", Value::Real(Real::of(3.0))),
                ("light", Value::Real(Real::of(-2.0))),
                ("light_radius", Value::Real(Real::of(5.0))),
            ],
            &[],
            cause(),
        )
        .unwrap();
    let Some(Subject::Area(area)) = sim.subject_of(&made) else {
        panic!("{made:?}");
    };
    let made = sim.truth().cosmos().area(area).unwrap().clone();
    assert_eq!(made.anchor, Some(clash_world::AreaAnchor::Entity(who)));
    assert_eq!(
        made.ambient.and_then(|over| over.gravity),
        Some(clash_world::GravityOverride::Scale(Real::of(3.0)))
    );
    assert_eq!(
        made.light.map(|light| light.intensity),
        Some(Real::of(-2.0))
    );
    let gravity = sim
        .truth()
        .cosmos()
        .ambient(common::at(30, 30))
        .unwrap()
        .gravity;
    let outside = sim
        .truth()
        .cosmos()
        .ambient(common::at(60, 30))
        .unwrap()
        .gravity;
    assert!((gravity - outside * Real::of(3.0)).abs() < Real::of(1.0e-9));
    // What it cannot use is refused with a reason.
    for (name, value) in [
        ("gravity", Value::from("down")),
        ("light_radius", Value::Real(Real::of(99.0))),
    ] {
        let mut args = vec![
            ("at", sim.subject_value(Subject::Loc(common::at(30, 30)))),
            ("light", Value::Real(Real::of(1.0))),
        ];
        args.push((name, value));
        let reason = sim
            .call_verb("create_area", &args, &[], cause())
            .unwrap_err();
        assert_eq!(reason.code, "world.bad_argument", "{name}");
    }
}
