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
//! | A7 | Revoking ends everything bound by its declared `on_end` (A3) | `revoking_an_ability_ends_everything_bound_to_it_by_its_on_end` |
//! | A7 | `summon_expires`: a servant obeys, then is gone when its summoner stops | `a_summoned_hound_obeys_and_is_gone_when_its_summoner_dies`, `what_a_mode_binds_ends_with_it_and_suppression_ends_what_ends_then` |
//! | A7 | `conjured_wall`: the street restored, an altered cell left alone | `a_conjured_wall_gives_the_street_back_and_leaves_an_altered_cell_alone`, `a_stone_wall_rises_across_the_way_and_sinks_back` |
//! | A7 | `release` leaves the thing in place, unbound | `release_leaves_the_thing_in_place_unbound` |
//! | A7 | Durations, `binding_ending`, costs from a bound companion, `dispel`, refusals | `a_binding_lasts_its_time_on_its_owners_clock_and_survives_a_save`, `a_rule_on_binding_ending_prolongs_it_or_changes_how_it_ends`, `a_cost_drawn_from_a_bound_companion_is_theirs_to_pay`, `dispel_ends_bindings_by_tag_and_by_place`, `binding_with_no_ability_or_bad_terms_is_refused_with_a_reason` |
//! | Content | Each bundled power used by a spawned person | `time_stop_…`, `a_null_field_…`, `a_gravity_well_…`, `floating_…`, `a_summoned_hound_…`, `a_stone_wall_…`, `a_shadow_…` |

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
    "pools": ["blood", "pain", "stamina", "reserve", "mana"] },
  { "type": "trait", "id": "hexed", "tags": ["hexed"] },
  { "type": "variable", "id": "watched", "scope": "entity", "value": "bool", "default": false, "doc": "Set by a bound rule." },
  { "type": "variable", "id": "keep", "scope": "entity", "value": "bool", "default": false, "doc": "Whether the next binding to run out is kept five seconds more." },
  { "type": "variable", "id": "free", "scope": "entity", "value": "bool", "default": false, "doc": "Whether bindings that end are released." },
  { "type": "eoc", "id": "watchful", "on": "tag_changed", "attached": true,
    "do": [ { "set": { "self.var.watched": true } } ] },
  { "type": "eoc", "id": "keeper", "on": "binding_ending", "if": "self.var.keep and event.why == 'duration'",
    "do": [ { "modify": { "event.prolong": "5" } }, { "set": { "self.var.keep": false } } ] },
  { "type": "eoc", "id": "freer", "on": "binding_ending", "if": "self.var.free",
    "do": [ { "modify": { "event.on_end": "release" } } ] },
  { "type": "ability", "id": "binder", "tags": ["test", "binding"],
    "grants": [ { "action": "zap", "params": { "energy": 1 },
                  "cost": { "blood": { "amount": 0.01, "from": "ability.bound('pet')" } } } ],
    "modes": [ { "id": "ward", "upkeep": { "mana": 1 },
                 "on_start": [ { "create_area": { "radius": 2, "anchor": "self", "as": "ward" } } ] } ] }
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

/// Loads `mods/core`'s world, life, traits, movement,
/// perception, minds and powers with more definitions, or renders why
/// not.
fn try_load(extra: &str) -> Result<Arc<Defs>, String> {
    let types = common::types();
    let mut files = dir_files("world");
    for sub in ["life", "powers", "ai", "perception"] {
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
    assert!(above.len() >= 2 && !below.is_empty(), "{samples:?}");
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

// ---- A7: bindings -----------------------------------------------------------------

/// Calls a verb as a rule would, and delivers what it raised.
trait RunVerb {
    fn run_verb(&mut self, verb: &str, args: &[(&str, Value)]) -> Result<Value, clash_sim::Reason>;
}

impl RunVerb for Sim {
    fn run_verb(&mut self, verb: &str, args: &[(&str, Value)]) -> Result<Value, clash_sim::Reason> {
        let made = self.call_verb(verb, args, &[], cause());
        self.pump();
        made
    }
}

fn loc_value(sim: &Sim, x: i32, y: i32) -> Value {
    sim.subject_value(Subject::Loc(common::at(x, y)))
}

fn def_value(sim: &Sim, ty: &str, name: &str) -> Value {
    let registry = sim.defs().expr_registry();
    let ty = registry.def_type_id(ty).unwrap();
    Value::Id(registry.def_ref(ty, name).unwrap())
}

fn entity_of(sim: &Sim, value: &Value) -> EntityId {
    match sim.subject_of(value) {
        Some(Subject::Entity(id)) => id,
        other => panic!("{other:?}"),
    }
}

fn block_at(sim: &Sim, x: i32, y: i32) -> clash_world::ElementState {
    sim.truth()
        .cosmos()
        .grid(common::GRID)
        .unwrap()
        .element(CellPos::new(x, y, 0), clash_world::Layer::Block)
        .unwrap()
}

fn set_block(sim: &mut Sim, x: i32, y: i32, name: Option<&str>) {
    let state = name.map_or(clash_world::ElementState::NONE, |name| {
        sim.truth()
            .cosmos()
            .defs()
            .block_state(name, None)
            .unwrap()
            .raw()
    });
    sim.world(|ctx| {
        ctx.edit(cause())
            .set_element(common::at(x, y), clash_world::Layer::Block, state)
    })
    .unwrap();
    sim.pump();
}

fn wall_state(sim: &Sim) -> clash_world::ElementState {
    common::wall(sim).raw()
}

fn summon_dog(sim: &mut Sim, by: &Value, x: i32, y: i32, extra: &[(&str, Value)]) -> EntityId {
    let mut args = vec![
        ("template", def_value(sim, "template", "core:dog")),
        ("at", loc_value(sim, x, y)),
        ("ability", by.clone()),
    ];
    args.extend(extra.iter().cloned());
    let dog = sim.run_verb("summon", &args).unwrap();
    entity_of(sim, &dog)
}

fn ended(events: &[SimEvent]) -> Vec<(String, String, String)> {
    events
        .iter()
        .filter(|event| event.name() == ability::BINDING_ENDED)
        .map(|event| (text(event, "what"), text(event, "why"), text(event, "how")))
        .collect()
}

#[test]
fn revoking_an_ability_ends_everything_bound_to_it_by_its_on_end() {
    let mut sim = sim();
    let who = spawn(&mut sim, "tester", 10, 10);
    let other = spawn(&mut sim, "tester", 14, 10);
    let number = grant(&mut sim, who, "binder");
    let by = ability::handle_value(&sim, number);
    sim.run_verb(
        "bind_trait",
        &[
            ("to", entity_value(&sim, other)),
            ("trait", def_value(&sim, "trait", "core:hexed")),
            ("ability", by.clone()),
        ],
    )
    .unwrap();
    assert!(has_trait(&sim, other, "hexed"));
    let dog = summon_dog(&mut sim, &by, 12, 12, &[("as", Value::from("pet"))]);
    assert_eq!(
        eval_of(&sim, who, "self.ability('core:binder').bound('pet')"),
        entity_value(&sim, dog)
    );
    assert_eq!(eval_of(&sim, dog, "self.bound_by"), entity_value(&sim, who));
    let placed = sim
        .run_verb(
            "conjure",
            &[
                ("at", loc_value(&sim, 20, 20)),
                ("to", loc_value(&sim, 22, 20)),
                ("what", Value::from("wall")),
                ("ability", by.clone()),
            ],
        )
        .unwrap();
    assert_eq!(placed, Value::Int(3));
    sim.run_verb(
        "conjure",
        &[
            ("at", loc_value(&sim, 20, 24)),
            ("what", Value::from("wall")),
            ("ability", by.clone()),
            ("on_end", Value::from("release")),
        ],
    )
    .unwrap();
    assert_eq!(
        sim.run_verb(
            "bind_rule",
            &[
                ("to", entity_value(&sim, other)),
                ("eoc", def_value(&sim, "eoc", "core:watchful")),
                ("ability", by.clone()),
            ],
        )
        .unwrap(),
        Value::Bool(true)
    );
    let area = sim
        .run_verb(
            "create_area",
            &[
                ("at", loc_value(&sim, 30, 30)),
                ("radius", Value::Real(Real::of(2.0))),
                ("bound", Value::Bool(true)),
                ("ability", by.clone()),
            ],
        )
        .unwrap();
    let Some(Subject::Area(area)) = sim.subject_of(&area) else {
        panic!("{area:?}");
    };
    assert_eq!(sim.bindings().count(), 6);
    sim.drain_events();

    ability::revoke(&mut sim, who, ability_id("binder"), cause()).unwrap();
    sim.pump();
    // Invariant A3: each by its declared `on_end`.
    let events = sim.drain_events();
    let mut ends = ended(&events);
    ends.sort();
    let want = |what: &str, how: &str| (what.to_owned(), "revoked".to_owned(), how.to_owned());
    let mut expected = vec![
        want("trait", "remove"),
        want("entity", "remove"),
        want("edit", "restore"),
        want("edit", "release"),
        want("rule", "remove"),
        want("area", "remove"),
    ];
    expected.sort();
    assert_eq!(ends, expected);
    assert!(!has_trait(&sim, other, "hexed"));
    assert!(!sim.truth().entity(dog).is_some());
    for x in 20..=22 {
        assert_eq!(block_at(&sim, x, 20), clash_world::ElementState::NONE);
    }
    assert_eq!(
        block_at(&sim, 20, 24),
        wall_state(&sim),
        "released: it stays"
    );
    assert!(sim.truth().cosmos().area(area).is_none());
    let handle = sim.exprs().kinds().handle(Subject::Entity(other)).unwrap();
    let watchful = DEFS
        .lookup::<clash_sim::rule::EocDef>("core:watchful")
        .unwrap();
    assert!(!sim.rules.attached_to(handle).contains(&watchful));
    assert_eq!(sim.bindings().count(), 0);
}

#[test]
fn release_leaves_the_thing_in_place_unbound() {
    let mut sim = sim();
    let who = spawn(&mut sim, "tester", 10, 10);
    let number = grant(&mut sim, who, "binder");
    let by = ability::handle_value(&sim, number);
    let dog = summon_dog(&mut sim, &by, 12, 12, &[("as", Value::from("pet"))]);
    sim.drain_events();
    let count = sim
        .run_verb(
            "dismiss",
            &[
                ("who", entity_value(&sim, who)),
                ("ability", def_value(&sim, "ability", "core:binder")),
                ("as", Value::from("pet")),
                ("how", Value::from("release")),
            ],
        )
        .unwrap();
    assert_eq!(count, Value::Int(1));
    assert_eq!(
        ended(&sim.drain_events()),
        vec![(
            "entity".to_owned(),
            "dismissed".to_owned(),
            "release".to_owned()
        )]
    );
    assert!(sim.truth().entity(dog).is_some());
    assert_eq!(eval_of(&sim, dog, "self.bound_by"), Value::Null);
    assert_eq!(
        eval_of(&sim, who, "self.ability('core:binder').bound('pet')"),
        Value::Null
    );
    // Revoking now leaves it: it is bound to nothing.
    ability::revoke(&mut sim, who, ability_id("binder"), cause()).unwrap();
    sim.pump();
    assert!(sim.truth().entity(dog).is_some());
}

#[test]
fn a_conjured_wall_gives_the_street_back_and_leaves_an_altered_cell_alone() {
    let mut sim = sim();
    let who = spawn(&mut sim, "tester", 10, 10);
    let number = grant(&mut sim, who, "binder");
    let by = ability::handle_value(&sim, number);
    set_block(&mut sim, 21, 20, Some("pillar"));
    let pillar = block_at(&sim, 21, 20);
    sim.run_verb(
        "conjure",
        &[
            ("at", loc_value(&sim, 20, 20)),
            ("to", loc_value(&sim, 23, 20)),
            ("what", Value::from("wall")),
            ("ability", by.clone()),
            ("as", Value::from("wall")),
        ],
    )
    .unwrap();
    for x in 20..=23 {
        assert_eq!(block_at(&sim, x, 20), wall_state(&sim));
    }
    let cells = eval_of(&sim, who, "len(self.ability('core:binder').bound('wall'))");
    assert_eq!(cells, Value::Int(4));
    // One cell is changed since: a window put in.
    set_block(&mut sim, 22, 20, Some("window"));
    let window = block_at(&sim, 22, 20);
    // Saved and loaded in the middle: nothing changes.
    let hash = sim.state_hash();
    let (mut loaded, _) = common::save_and_load(&sim);
    loaded.pump();
    assert_eq!(loaded.state_hash(), hash);
    loaded
        .run_verb(
            "dismiss",
            &[
                ("who", entity_value(&loaded, who)),
                ("ability", def_value(&loaded, "ability", "core:binder")),
            ],
        )
        .unwrap();
    assert_eq!(block_at(&loaded, 20, 20), clash_world::ElementState::NONE);
    assert_eq!(
        block_at(&loaded, 21, 20),
        pillar,
        "what was there comes back"
    );
    assert_eq!(
        block_at(&loaded, 22, 20),
        window,
        "what was altered is left"
    );
    assert_eq!(block_at(&loaded, 23, 20), clash_world::ElementState::NONE);
    // A cell that is not loaded refuses the whole of it, changing nothing.
    let reason = loaded
        .run_verb(
            "conjure",
            &[
                ("at", loc_value(&loaded, 60, 20)),
                ("to", loc_value(&loaded, 70, 20)),
                ("what", Value::from("wall")),
                ("ability", by),
            ],
        )
        .unwrap_err();
    assert_eq!(reason.code, "world.not_loaded");
    assert_eq!(block_at(&loaded, 60, 20), clash_world::ElementState::NONE);
}

#[test]
fn a_binding_lasts_its_time_on_its_owners_clock_and_survives_a_save() {
    let mut sim = sim();
    let who = spawn(&mut sim, "tester", 10, 10);
    let number = grant(&mut sim, who, "binder");
    let by = ability::handle_value(&sim, number);
    let dog = summon_dog(
        &mut sim,
        &by,
        12,
        12,
        &[("for", Value::Real(Real::of(10.0)))],
    );
    advance(&mut sim, Dur::from_secs(6));
    assert!(sim.truth().entity(dog).is_some());
    let hash = sim.state_hash();
    let (mut loaded, _) = common::save_and_load(&sim);
    loaded.pump();
    assert_eq!(loaded.state_hash(), hash);
    advance(&mut loaded, Dur::from_secs(3));
    assert!(loaded.truth().entity(dog).is_some());
    loaded.drain_events();
    advance(&mut loaded, Dur::from_secs(2));
    assert!(!loaded.truth().entity(dog).is_some());
    assert_eq!(
        ended(&loaded.drain_events()),
        vec![(
            "entity".to_owned(),
            "duration".to_owned(),
            "remove".to_owned()
        )]
    );
    // The same from the start gives the same hash: nothing in it is
    // left to chance or to the order of a map.
    let mut again = crate::sim();
    let who_again = spawn(&mut again, "tester", 10, 10);
    let number = grant(&mut again, who_again, "binder");
    let by = ability::handle_value(&again, number);
    summon_dog(
        &mut again,
        &by,
        12,
        12,
        &[("for", Value::Real(Real::of(10.0)))],
    );
    advance(&mut again, Dur::from_secs(6));
    assert_eq!(again.state_hash(), hash);
}

#[test]
fn a_rule_on_binding_ending_prolongs_it_or_changes_how_it_ends() {
    let mut sim = sim();
    let who = spawn(&mut sim, "tester", 10, 10);
    let number = grant(&mut sim, who, "binder");
    let by = ability::handle_value(&sim, number);
    let set = |sim: &mut Sim, name: &str, on: bool| {
        sim.set_var(
            Subject::Entity(who),
            &format!("core:{name}"),
            Value::Bool(on),
            cause(),
        )
        .unwrap();
    };
    set(&mut sim, "keep", true);
    let dog = summon_dog(
        &mut sim,
        &by,
        12,
        12,
        &[("for", Value::Real(Real::of(4.0)))],
    );
    advance(&mut sim, Dur::from_secs(6));
    assert!(
        sim.truth().entity(dog).is_some(),
        "prolonged by five seconds"
    );
    advance(&mut sim, Dur::from_secs(4));
    assert!(!sim.truth().entity(dog).is_some());
    set(&mut sim, "free", true);
    let freed = summon_dog(
        &mut sim,
        &by,
        12,
        13,
        &[("for", Value::Real(Real::of(2.0)))],
    );
    sim.drain_events();
    advance(&mut sim, Dur::from_secs(3));
    let events = sim.drain_events();
    assert_eq!(
        ended(&events),
        vec![(
            "entity".to_owned(),
            "duration".to_owned(),
            "release".to_owned()
        )]
    );
    assert!(sim.truth().entity(freed).is_some(), "released, not removed");
    assert_eq!(sim.bindings().count(), 0);
}

#[test]
fn a_cost_drawn_from_a_bound_companion_is_theirs_to_pay() {
    let mut sim = sim();
    let who = spawn(&mut sim, "tester", 10, 10);
    let number = grant(&mut sim, who, "binder");
    let by = ability::handle_value(&sim, number);
    let reason =
        ability::available(&sim, who, ability_id("binder"), Some(action("zap"))).unwrap_err();
    assert_eq!(reason.code, codes::COST_SOURCE_GONE);
    let dog = summon_dog(&mut sim, &by, 12, 12, &[("as", Value::from("pet"))]);
    let before = level(&sim, dog, "blood");
    assert!(use_it(&mut sim, who, "binder", "zap").unwrap());
    assert!(level(&sim, dog, "blood") < before);
    sim.run_verb(
        "dismiss",
        &[
            ("who", entity_value(&sim, who)),
            ("ability", def_value(&sim, "ability", "core:binder")),
        ],
    )
    .unwrap();
    let reason =
        ability::available(&sim, who, ability_id("binder"), Some(action("zap"))).unwrap_err();
    assert_eq!(reason.code, codes::COST_SOURCE_GONE);
}

#[test]
fn what_a_mode_binds_ends_with_it_and_suppression_ends_what_ends_then() {
    let mut sim = sim();
    let who = spawn(&mut sim, "tester", 10, 10);
    let number = grant(&mut sim, who, "binder");
    let by = ability::handle_value(&sim, number);
    ability::start_mode(&mut sim, who, ability_id("binder"), "ward", cause()).unwrap();
    sim.pump();
    let ward = eval_of(&sim, who, "self.ability('core:binder').bound('ward')");
    let Some(Subject::Area(ward)) = sim.subject_of(&ward) else {
        panic!("{ward:?}");
    };
    // It moves with its owner, and goes when the mode ends.
    step_to(&mut sim, who, 11, 10);
    assert_eq!(
        sim.truth().cosmos().area(ward).unwrap().shape.centre(),
        Some(CellPos::new(11, 10, 0))
    );
    ability::end_mode(&mut sim, who, ability_id("binder"), "ward", cause()).unwrap();
    sim.pump();
    assert!(sim.truth().cosmos().area(ward).is_none());
    // One that ends only with its mode outlasts a suppression; one that
    // ends on everything does not.
    let kept = summon_dog(&mut sim, &by, 12, 12, &[("ends_on", texts(&["mode"]))]);
    let lost = summon_dog(&mut sim, &by, 12, 13, &[]);
    sim.call_verb(
        "suppress_abilities",
        &[
            ("who", entity_value(&sim, who)),
            ("tags", texts(&["binding"])),
        ],
        &[],
        cause(),
    )
    .unwrap();
    sim.pump();
    assert!(sim.truth().entity(kept).is_some());
    assert!(!sim.truth().entity(lost).is_some());
    // Its owner dying ends what ends then; `mode` does not.
    let third = summon_dog(
        &mut sim,
        &by,
        12,
        14,
        &[("ends_on", texts(&["owner_stops"]))],
    );
    sim.run_verb("die", &[("who", entity_value(&sim, who))])
        .unwrap();
    assert!(!sim.truth().entity(third).is_some());
    assert!(sim.truth().entity(kept).is_some());
}

#[test]
fn dispel_ends_bindings_by_tag_and_by_place() {
    let mut sim = sim();
    let who = spawn(&mut sim, "tester", 10, 10);
    let number = grant(&mut sim, who, "binder");
    let by = ability::handle_value(&sim, number);
    let near = summon_dog(&mut sim, &by, 30, 30, &[]);
    let far = summon_dog(&mut sim, &by, 50, 50, &[]);
    let none = sim
        .run_verb("dispel", &[("tags", texts(&["fire"]))])
        .unwrap();
    assert_eq!(none, Value::Int(0));
    let one = sim
        .run_verb(
            "dispel",
            &[
                ("tags", texts(&["binding"])),
                ("at", loc_value(&sim, 31, 30)),
                ("radius", Value::Real(Real::of(3.0))),
            ],
        )
        .unwrap();
    assert_eq!(one, Value::Int(1));
    assert!(!sim.truth().entity(near).is_some());
    assert!(sim.truth().entity(far).is_some());
    let rest = sim
        .run_verb("dispel", &[("owner", entity_value(&sim, who))])
        .unwrap();
    assert_eq!(rest, Value::Int(1));
    assert!(!sim.truth().entity(far).is_some());
}

#[test]
fn binding_with_no_ability_or_bad_terms_is_refused_with_a_reason() {
    let mut sim = sim();
    let who = spawn(&mut sim, "tester", 10, 10);
    let number = grant(&mut sim, who, "binder");
    let by = ability::handle_value(&sim, number);
    let reason = sim
        .run_verb(
            "summon",
            &[
                ("template", def_value(&sim, "template", "core:dog")),
                ("at", loc_value(&sim, 12, 12)),
            ],
        )
        .unwrap_err();
    assert_eq!(reason.code, codes::NO_BINDER);
    for (name, value) in [
        ("for", Value::Real(Real::of(-1.0))),
        ("ends_on", texts(&["sunset"])),
        ("on_end", Value::from("explode")),
    ] {
        let reason = sim
            .run_verb(
                "summon",
                &[
                    ("template", def_value(&sim, "template", "core:dog")),
                    ("at", loc_value(&sim, 12, 12)),
                    ("ability", by.clone()),
                    (name, value),
                ],
            )
            .unwrap_err();
        assert_eq!(reason.code, codes::BAD_BINDING, "{name}");
    }
    sim.set_setting("rules.max_bindings", clash_core::Value::Int(1));
    summon_dog(&mut sim, &by, 12, 12, &[]);
    let reason = sim
        .run_verb(
            "summon",
            &[
                ("template", def_value(&sim, "template", "core:dog")),
                ("at", loc_value(&sim, 13, 12)),
                ("ability", by),
            ],
        )
        .unwrap_err();
    assert_eq!(reason.code, codes::TOO_MANY_BINDINGS);
}

// ---- The bundled powers ------------------------------------------------------------

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

fn mode_on(sim: &mut Sim, who: EntityId, name: &str, mode: &str) {
    ability::start_mode(sim, who, ability_id(name), mode, cause())
        .unwrap_or_else(|reason| panic!("{reason:?}"));
    sim.pump();
}

fn gravity_at(sim: &Sim, x: i32, y: i32) -> Real {
    sim.truth()
        .cosmos()
        .ambient(common::at(x, y))
        .unwrap()
        .gravity
}

#[test]
fn time_stop_stops_everyone_else_while_its_maker_walks_on() {
    let mut sim = sim();
    let user = spawn(&mut sim, "power_sampler", 20, 20);
    let other = spawn(&mut sim, "person", 26, 20);
    mode_on(&mut sim, user, "time_stop", "stopped");
    assert!(has_trait(&sim, user, "outside_time"));
    // The maker walks up; the bubble comes over the other, who stops.
    step_to(&mut sim, user, 24, 20);
    let stopped = sim.truth().own_time(other);
    let user_before = sim.truth().own_time(user);
    advance(&mut sim, Dur::from_secs(4));
    let user_ran = sim.truth().own_time(user) - user_before;
    let other_ran = sim.truth().own_time(other) - stopped;
    assert_eq!(user_ran, Dur::from_secs(4));
    assert!(other_ran < Dur::from_millis(10), "{other_ran:?}");
    // Saved and loaded in the middle: nothing changes.
    let hash = sim.state_hash();
    let (mut loaded, _) = common::save_and_load(&sim);
    loaded.pump();
    assert_eq!(loaded.state_hash(), hash);
    // The reserve runs dry: the mode ends, the bubble goes, time resumes.
    loaded.drain_events();
    advance(&mut loaded, Dur::from_secs(20));
    let events = loaded.drain_events();
    let mode = events
        .iter()
        .find(|event| event.name() == ability::MODE_ENDED)
        .expect("the mode ends");
    assert_eq!(text(mode, "how"), "empty");
    assert!(ended(&events).contains(&("area".to_owned(), "mode".to_owned(), "remove".to_owned())));
    let resumed = loaded.truth().own_time(other);
    advance(&mut loaded, Dur::from_secs(2));
    assert_eq!(loaded.truth().own_time(other) - resumed, Dur::from_secs(2));
}

#[test]
fn a_null_field_ends_anothers_power_and_spares_its_makers_own() {
    let mut sim = sim();
    let maker = spawn(&mut sim, "power_sampler", 20, 20);
    let other = spawn(&mut sim, "power_sampler", 34, 20);
    mode_on(&mut sim, maker, "null_field", "on");
    mode_on(&mut sim, maker, "shield", "raised");
    assert!(!inst(&sim, maker, "shield").is_suppressed());
    assert_eq!(inst(&sim, maker, "shield").resisted().len(), 1);
    mode_on(&mut sim, other, "shield", "raised");
    assert!(has_trait(&sim, other, "shielded"));
    sim.drain_events();
    step_to(&mut sim, other, 22, 20);
    let events = sim.drain_events();
    let ended_mode = events
        .iter()
        .find(|event| event.name() == ability::MODE_ENDED)
        .expect("the other's shield ends");
    assert_eq!(text(ended_mode, "how"), "suppressed");
    assert!(!has_trait(&sim, other, "shielded"));
    assert!(has_trait(&sim, maker, "shielded"));
    // Its field moves with it: walking away leaves the other's powers
    // to come back, without the mode.
    step_to(&mut sim, maker, 10, 20);
    assert!(!inst(&sim, other, "shield").is_suppressed());
    assert!(!has_trait(&sim, other, "shielded"));
    assert!(ability::start_mode(&mut sim, other, ability_id("shield"), "raised", cause()).is_ok());
}

#[test]
fn a_gravity_well_weighs_six_times_for_a_while() {
    let mut sim = sim();
    let user = spawn(&mut sim, "power_sampler", 20, 20);
    let normal = gravity_at(&sim, 26, 20);
    assert!(power_at(&mut sim, user, "gravity_well", 26, 20));
    advance(&mut sim, Dur::from_secs(1));
    assert!((gravity_at(&sim, 26, 20) - normal * Real::of(6.0)).abs() < Real::of(1.0e-9));
    assert!((gravity_at(&sim, 26, 22) - normal * Real::of(6.0)).abs() < Real::of(1.0e-9));
    assert_eq!(gravity_at(&sim, 20, 20), normal);
    assert!(
        eval_of(
            &sim,
            user,
            "self.ability('core:gravity_well').bound('well') != null"
        ) == Value::Bool(true)
    );
    advance(&mut sim, Dur::from_secs(9));
    assert_eq!(gravity_at(&sim, 26, 20), normal, "its time is up");
}

#[test]
fn floating_keeps_its_maker_from_falling_when_thrown() {
    let mut sim = sim();
    let floater = spawn(&mut sim, "power_sampler", 20, 20);
    let sinker = spawn(&mut sim, "power_sampler", 20, 30);
    mode_on(&mut sim, floater, "float", "floating");
    assert_eq!(gravity_at(&sim, 20, 20), Real::ZERO);
    for who in [floater, sinker] {
        sim.run_verb(
            "launch",
            &[
                ("what", entity_value(&sim, who)),
                (
                    "vel",
                    Value::list([Real::of(4.0), Real::ZERO, Real::of(2.0)].map(Value::Real)),
                ),
            ],
        )
        .unwrap();
    }
    advance(&mut sim, Dur::from_millis(500));
    let vertical = |sim: &Sim, who: EntityId| {
        let body = sim.truth().cosmos().body_of(who).unwrap();
        sim.truth().cosmos().body(body).unwrap().vel[2]
    };
    assert!(
        vertical(&sim, floater) > Real::ZERO,
        "nothing pulls it down"
    );
    let height = |sim: &Sim, who: EntityId| {
        let body = sim.truth().cosmos().body_of(who).unwrap();
        sim.truth().cosmos().body(body).unwrap().pos[2]
    };
    assert!(height(&sim, floater) > Real::of(0.5));
    assert!(
        height(&sim, sinker) < Real::of(0.1),
        "the other has come down"
    );
}

#[test]
fn a_summoned_hound_obeys_and_is_gone_when_its_summoner_dies() {
    let mut sim = sim();
    let summoner = spawn(&mut sim, "power_sampler", 20, 20);
    let enemy = spawn(&mut sim, "person", 27, 20);
    clash_sim::ai::set_stance(
        &mut sim,
        summoner,
        enemy,
        Some(clash_sim::ai::Stance::Hostile),
    );
    assert!(power_at(&mut sim, summoner, "summon_hound", 21, 20));
    advance(&mut sim, Dur::from_millis(1500));
    let hound = sim
        .bindings()
        .find_map(|binding| match binding.what {
            ability::Bound::Entity { id } => Some(id),
            _ => None,
        })
        .expect("a hound came");
    assert_eq!(
        eval_of(&sim, hound, "self.bound_by"),
        entity_value(&sim, summoner)
    );
    // It took its summoner's quarrel.
    assert_eq!(
        clash_sim::ai::stance_toward(&sim, hound, enemy),
        Some(clash_sim::ai::Stance::Hostile)
    );
    assert_ne!(
        clash_sim::ai::stance_toward(&sim, hound, summoner),
        Some(clash_sim::ai::Stance::Hostile)
    );
    let distance = |sim: &Sim| {
        let a = sim.truth().location(hound).unwrap().cell;
        let b = sim.truth().location(enemy).unwrap().cell;
        (a.x - b.x).abs().max((a.y - b.y).abs())
    };
    let before = distance(&sim);
    advance(&mut sim, Dur::from_secs(4));
    assert!(distance(&sim) < before, "it goes for the one held hostile");
    // Its summoner stops: it is gone.
    sim.run_verb("die", &[("who", entity_value(&sim, summoner))])
        .unwrap();
    assert!(sim.truth().entity(hound).is_none());
}

#[test]
fn a_stone_wall_rises_across_the_way_and_sinks_back() {
    let mut sim = sim();
    let user = spawn(&mut sim, "power_sampler", 20, 20);
    set_block(&mut sim, 23, 21, Some("pillar"));
    let pillar = block_at(&sim, 23, 21);
    assert!(power_at(&mut sim, user, "stone_wall", 23, 20));
    advance(&mut sim, Dur::from_secs(1));
    for y in 19..=21 {
        assert_eq!(block_at(&sim, 23, y), wall_state(&sim), "{y}");
    }
    let hash = sim.state_hash();
    let (mut loaded, _) = common::save_and_load(&sim);
    loaded.pump();
    assert_eq!(loaded.state_hash(), hash);
    advance(&mut loaded, Dur::from_secs(31));
    assert_eq!(block_at(&loaded, 23, 19), clash_world::ElementState::NONE);
    assert_eq!(block_at(&loaded, 23, 20), clash_world::ElementState::NONE);
    assert_eq!(block_at(&loaded, 23, 21), pillar);
}

#[test]
fn a_shadow_darkens_round_its_maker_and_follows_them() {
    let mut sim = sim();
    let user = spawn(&mut sim, "power_sampler", 30, 30);
    for (x, y) in [(31, 30), (41, 30)] {
        let lamp = stone(&mut sim, x, y);
        let bright = clash_world::LightSource::shining(Real::of(2.0), Some(Real::of(8.0)))
            .unwrap()
            .unwrap();
        bubble(&mut sim, lamp, 0.0, |spec| spec.light = Some(bright));
    }
    let light_at = |sim: &Sim, x: i32, y: i32| {
        let cosmos = sim.truth().cosmos();
        let time = clash_world::SkyTime::now(cosmos, sim.timeline(), common::GRID).unwrap();
        cosmos.light(common::at(x, y), time).unwrap()
    };
    let lit_here = light_at(&sim, 30, 30);
    let lit_there = light_at(&sim, 40, 30);
    mode_on(&mut sim, user, "shadow", "shrouded");
    assert!(light_at(&sim, 30, 30) < lit_here);
    assert!(light_at(&sim, 30, 30) >= Real::ZERO);
    assert_eq!(light_at(&sim, 40, 30), lit_there);
    step_to(&mut sim, user, 40, 30);
    assert_eq!(light_at(&sim, 30, 30), lit_here);
    assert!(light_at(&sim, 40, 30) < lit_there);
    ability::end_mode(&mut sim, user, ability_id("shadow"), "shrouded", cause()).unwrap();
    sim.pump();
    assert_eq!(light_at(&sim, 40, 30), lit_there);
}
