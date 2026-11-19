//! Contests (07 §4.1; plans/combat.md, package C1) through the loader
//! and the simulation: `odds` is pure and may be read where a character
//! previews, `contest` draws from the rule stream and is refused where
//! nothing may be drawn, and the margins a rule draws fall at or above
//! zero as often as `odds` says.

mod common;

use clash_core::Real;
use clash_expr::Value;
use clash_sim::Subject;
use clash_world::Cause;
use common::{creature, on, sim_over, spawn};

const CONTESTS: &str = r#"[
  { "type": "contest", "id": "aim", "form": "difference", "factor": 2, "spread": 1, "near": 0.5 },
  { "type": "contest", "id": "plain", "spread": 0 },
  { "type": "contest", "id": "might", "form": "ratio", "factor": 3, "spread": 0.5 },
  { "type": "eoc", "id": "draw", "subjects": { "self": "entity" }, "returns": "real",
    "params": { "a": "real", "b": "real" },
    "doc": "A margin of `aim`.",
    "do": [ { "return": "contest('aim', $a, $b) ?? 0" } ] },
  { "type": "eoc", "id": "chance", "subjects": { "self": "entity" }, "returns": "real",
    "params": { "a": "real", "b": "real" },
    "doc": "The odds of `aim`.",
    "do": [ { "return": "odds('aim', $a, $b) ?? 0" } ] },
  { "type": "action", "id": "wary", "params": { "foe": "entity" },
    "requires": "odds('might', 2, 1) > 0.5",
    "phases": [ { "id": "look", "time": 0 } ] }
]"#;

fn defs() -> std::sync::Arc<clash_data::Defs> {
    common::try_defs(CONTESTS).unwrap_or_else(|error| panic!("{error}"))
}

fn call(sim: &mut clash_sim::Sim, rule: &str, a: f64, b: f64) -> Real {
    let id = sim.rule_set().lookup(&format!("core:{rule}")).unwrap();
    let me = sim.truth().entities().ids().next().unwrap();
    let me = sim.subject_value(Subject::Entity(me));
    let value = sim
        .call_rule(
            id,
            &[("self", me)],
            &[
                ("a", Value::Real(Real::of(a))),
                ("b", Value::Real(Real::of(b))),
            ],
            Cause::DEBUG,
        )
        .unwrap();
    value.as_number().unwrap()
}

#[test]
fn odds_is_pure_and_contest_is_refused_where_nothing_may_be_drawn() {
    // `requires` is compiled pure (03 §6.1): `odds` loads there.
    defs();
    let drawn = r#"[
      { "type": "contest", "id": "aim" },
      { "type": "action", "id": "rash", "requires": "contest('aim', 2, 1) > 0",
        "phases": [ { "id": "go", "time": 0 } ] }
    ]"#;
    let error = common::try_defs(drawn).unwrap_err();
    assert!(error.contains("contest"), "{error}");
    assert!(error.contains("requires"), "{error}");
}

#[test]
fn margins_drawn_by_rules_follow_the_odds() {
    let mut sim = sim_over(defs(), 31);
    spawn(&mut sim, creature(), on(3, 3));
    let draws = 4000;
    for (a, b) in [(1.0, 1.0), (1.5, 1.0), (0.5, 1.0)] {
        let odds = call(&mut sim, "chance", a, b).get();
        let hits = (0..draws)
            .filter(|_| call(&mut sim, "draw", a, b) >= Real::ZERO)
            .count();
        let observed = hits as f64 / f64::from(draws);
        let tolerance = 4.0 * (odds * (1.0 - odds) / f64::from(draws)).sqrt();
        assert!(
            (observed - odds).abs() <= tolerance.max(0.01),
            "{a} against {b}: odds {odds}, observed {observed}"
        );
    }
    // The same seed draws the same margins.
    let mut again = sim_over(defs(), 31);
    spawn(&mut again, creature(), on(3, 3));
    let mut first = sim_over(defs(), 31);
    spawn(&mut first, creature(), on(3, 3));
    for _ in 0..10 {
        assert_eq!(
            call(&mut first, "draw", 1.0, 1.2),
            call(&mut again, "draw", 1.0, 1.2)
        );
    }
}
