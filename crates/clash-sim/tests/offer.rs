//! What a character could do, as screens read it (`clash_sim::offer`,
//! 12 §9.3): the actions carrying a tag, each with whether it could
//! start and why not, judged without changing anything; the lists of
//! abilities and known sites, empty for a creature with none; and an
//! ability's state, `suppressed` while it is.

mod common;

use clash_expr::Value;
use clash_sim::ability::{self, AbilityDef, AbilitySource};
use clash_sim::action::ActionDef;
use clash_sim::{Sim, Subject};
use clash_world::Cause;

const EXTRA: &str = r#"[
  { "type": "action", "id": "jab", "name": "Jab", "tags": ["strike"],
    "params": { "target": "entity" },
    "requires": "dist(self, $target) < 3", "fail_text": "That is out of reach.",
    "phases": [ { "id": "go", "time": "100 ms" } ] },
  { "type": "action", "id": "wave", "name": "Wave", "tags": ["greeting"],
    "phases": [ { "id": "go", "time": "100 ms" } ] },
  { "type": "ability", "id": "knack", "name": "Knack", "tags": ["test"],
    "grants": [ { "action": "wave" } ],
    "modes": [ { "id": "glow", "name": "Glow" } ] }
]"#;

/// A field of a row as text or a flag.
fn field(row: &Value, name: &str) -> Value {
    row.as_map()
        .and_then(|map| map.get(name))
        .cloned()
        .unwrap_or(Value::Null)
}

fn rows(sim: &Sim, who: clash_core::EntityId, property: &str, args: &[Value]) -> Vec<Value> {
    sim.property_named(Subject::Entity(who), property, args)
        .as_list()
        .map(<[Value]>::to_vec)
        .unwrap_or_default()
}

#[test]
fn the_actions_of_a_tag_say_whether_they_could_start_and_why_not() {
    let mut sim = common::sim_over(common::life_defs(EXTRA), 1);
    let me = common::spawn(&mut sim, common::creature(), common::on(5, 5));
    let near = common::spawn(&mut sim, common::creature(), common::on(6, 5));
    let far = common::spawn(&mut sim, common::creature(), common::on(15, 5));
    // A menu judges `requires` as the character believes (05 §7.3):
    // `dist` measures to where it knows the others are (D-780).
    for (other, at) in [(near, common::at(6, 5)), (far, common::at(15, 5))] {
        clash_sim::perception::add_track(
            &mut sim,
            me,
            other,
            Some(at),
            clash_sim::perception::Knowing::Told,
            clash_core::Real::ONE,
        )
        .unwrap();
    }
    let (near, far) = (
        sim.subject_value(Subject::Entity(near)),
        sim.subject_value(Subject::Entity(far)),
    );
    let before = sim.state_hash();

    // With no target its `requires` cannot be judged yet.
    let listed = rows(&sim, me, "actions_with", &[Value::from("strike")]);
    assert_eq!(listed.len(), 1);
    assert_eq!(field(&listed[0], "label"), Value::from("Jab"));
    assert_eq!(field(&listed[0], "id"), Value::from("core:jab"));
    assert_eq!(field(&listed[0], "available"), Value::Bool(true));

    // Someone within reach, and someone out of it, with the action's
    // own reason.
    let at_near = rows(&sim, me, "actions_with", &[Value::from("strike"), near]);
    assert_eq!(field(&at_near[0], "available"), Value::Bool(true));
    assert_eq!(field(&at_near[0], "why"), Value::from(""));
    let at_far = rows(
        &sim,
        me,
        "actions_with",
        &[Value::from("strike"), far.clone()],
    );
    assert_eq!(field(&at_far[0], "available"), Value::Bool(false));
    assert_eq!(
        field(&at_far[0], "why"),
        Value::from("That is out of reach.")
    );

    // Another tag, another list; a tag nobody carries, none.
    let waves = rows(&sim, me, "actions_with", &[Value::from("greeting")]);
    assert_eq!(field(&waves[0], "label"), Value::from("Wave"));
    assert!(rows(&sim, me, "actions_with", &[Value::from("dance")]).is_empty());

    // Asked directly: a parameter the action does not have is refused.
    let jab = sim.defs().lookup::<ActionDef>("core:jab").unwrap();
    let refused = clash_sim::offer::could_start(&sim, me, jab, &[("target", far)]).unwrap_err();
    assert_eq!(refused.message, "That is out of reach.");
    let wave = sim.defs().lookup::<ActionDef>("core:wave").unwrap();
    let wrong = clash_sim::offer::could_start(&sim, me, wave, &[("speed", Value::Int(3))]);
    assert!(wrong.is_err());

    // Nothing was changed by asking.
    assert_eq!(sim.state_hash(), before);
}

#[test]
fn a_creature_with_no_abilities_or_memory_lists_none() {
    let mut sim = common::sim_over(common::life_defs(EXTRA), 1);
    let me = common::spawn(&mut sim, common::creature(), common::on(5, 5));
    assert!(rows(&sim, me, "ability_uses", &[]).is_empty());
    assert!(rows(&sim, me, "ability_uses", &[Value::from("core:nothing")]).is_empty());
    assert!(rows(&sim, me, "ability_modes", &[]).is_empty());
    assert!(rows(&sim, me, "known_sites", &[]).is_empty());
}

#[test]
fn a_suppressed_ability_says_so_in_its_uses_and_its_modes() {
    let mut sim = common::sim_over(common::life_defs(EXTRA), 1);
    let me = common::spawn(&mut sim, common::creature(), common::on(5, 5));
    let knack = sim.defs().lookup::<AbilityDef>("core:knack").unwrap();
    ability::grant(&mut sim, me, knack, AbilitySource::Rule, None, Cause::DEBUG).unwrap();
    sim.pump();
    let uses = rows(&sim, me, "ability_uses", &[]);
    assert_eq!(uses.len(), 1);
    assert_eq!(field(&uses[0], "state"), Value::from("ready"));
    assert_eq!(field(&uses[0], "available"), Value::Bool(true));
    let modes = rows(&sim, me, "ability_modes", &[]);
    assert_eq!(field(&modes[0], "label"), Value::from("Glow"));
    assert_eq!(field(&modes[0], "available"), Value::Bool(true));

    ability::suppress_abilities(&mut sim, me, None, None, Cause::DEBUG).unwrap();
    sim.pump();
    let uses = rows(&sim, me, "ability_uses", &[]);
    assert_eq!(field(&uses[0], "state"), Value::from("suppressed"));
    assert_eq!(field(&uses[0], "available"), Value::Bool(false));
    assert_ne!(field(&uses[0], "why"), Value::from(""));
    let modes = rows(&sim, me, "ability_modes", &[]);
    assert_eq!(field(&modes[0], "available"), Value::Bool(false));
    assert_eq!(field(&modes[0], "why"), Value::from("Knack is suppressed."));
}
