//! The cold layer's records (09 §10.1, §14.2; package L1): calendars and
//! the time of day following a realm's clock, routines' parts with
//! their chances and delays, and agents that cost nothing between their
//! wake-ups (invariant N7). Over the flat test realm of `common`, with a
//! routine and a mind of the tests' own.

mod common;

use clash_core::{Dur, Real, Tick};
use clash_expr::{Context, Type, Value};
use clash_sim::agent::{self, AgentPlace, Day};
use clash_sim::{Bound, Sim, Subject};
use clash_world::CellPos;

const EXTRA: &str = r#"[
  { "type": "calendar", "id": "test_calendar",
    "parts": [ { "name": "night", "from": "00:00" }, { "name": "day", "from": "06:00" },
               { "name": "evening", "from": "18:00" } ],
    "year": 4, "seasons": [ { "name": "wet", "from": 0 }, { "name": "dry", "from": 2 } ] },
  { "type": "realm", "id": "calendar_land", "copy_from": "testland", "calendar": "test_calendar" },
  { "type": "routine", "id": "three_parts",
    "day": [ { "from": "06:00", "at": { "home": true }, "doing": "up" },
             { "from": "12:00", "at": { "home": true }, "doing": "busy", "chance": 0.5,
               "jitter": "30 min" },
             { "from": "22:00", "at": { "home": true }, "doing": "asleep" } ] },
  { "type": "mind", "id": "keeper", "goals": [], "routine": "three_parts" },
  { "type": "template", "id": "keeper", "mind": "keeper" }
]"#;

fn sim(seed: u64) -> Sim {
    common::sim_over_realm(common::life_defs(EXTRA), "core:calendar_land", seed)
}

fn realm_value(sim: &Sim, source: &str) -> Value {
    let registry = sim.exprs().registry();
    let realm = Type::Subject(registry.kind_id("realm").unwrap());
    let context = Context::new().subject("r", realm);
    let compiled = clash_expr::compile(source, registry, &context, &Type::Any)
        .value
        .unwrap_or_else(|| panic!("`{source}` compiles"));
    let realm = common::realm(sim);
    sim.eval(
        &compiled,
        Bound::new().subject(sim.subject_value(Subject::Realm(realm))),
    )
}

fn seconds(value: &Value) -> f64 {
    value.as_number().unwrap().get()
}

#[test]
fn the_time_of_day_follows_the_realms_clock_and_its_calendar_names_it() {
    let mut sim = sim(3);
    // The realm's sky starts its clock at eight in the morning.
    assert_eq!(seconds(&realm_value(&sim, "r.time_of_day")), 8.0 * 3600.0);
    assert_eq!(realm_value(&sim, "r.day"), Value::Int(1));
    assert_eq!(realm_value(&sim, "r.day_part"), Value::from("day"));
    assert_eq!(realm_value(&sim, "r.season"), Value::from("wet"));
    assert_eq!(seconds(&realm_value(&sim, "r.day_length")), 86_400.0);
    // Ten hours on: evening.
    sim.advance(clash_sim::AdvanceLimit::Until(
        Tick::ZERO + Dur::from_secs(10 * 3600),
    ));
    assert_eq!(seconds(&realm_value(&sim, "r.time_of_day")), 18.0 * 3600.0);
    assert_eq!(realm_value(&sim, "r.day_part"), Value::from("evening"));
    // The realm's clock runs twice as fast: an hour of universal time is
    // two of the realm's.
    let realm = common::realm(&sim);

    sim.set_clock_rate(
        clash_sim::ClockTarget::Realm(realm),
        Some(Real::of(2.0)),
        clash_world::Cause::new(clash_world::CauseKind::Debug),
    )
    .unwrap();
    let now = sim.now();
    sim.advance(clash_sim::AdvanceLimit::Until(now + Dur::from_secs(3600)));
    assert_eq!(seconds(&realm_value(&sim, "r.time_of_day")), 20.0 * 3600.0);
    // Two days and more later: day 3, the dry season.
    let now = sim.now();
    sim.advance(clash_sim::AdvanceLimit::Until(
        now + Dur::from_secs(24 * 3600),
    ));
    assert_eq!(realm_value(&sim, "r.day"), Value::Int(3));
    assert_eq!(realm_value(&sim, "r.season"), Value::from("dry"));
}

#[test]
fn a_routines_parts_are_kept_by_chance_and_begun_late_by_each_character() {
    let sim = sim(3);
    let (_, def) = agent::routine::routine_named(&sim, "three_parts").unwrap();
    let day = Day {
        length: Dur::DAY,
        starts_at: Dur::ZERO,
    };
    let hour = |h: i64| Dur::from_secs(h * 3600);
    let mut kept = 0;
    let mut late = 0;
    for seed in 0..200_u64 {
        // At one in the afternoon of day ten: busy, or still up.
        let (entry, began) = agent::routine::entry_at(def, day, hour(24 * 10 + 13), seed).unwrap();
        match entry {
            1 => {
                kept += 1;
                let since_noon = began - hour(24 * 10 + 12);
                assert!(since_noon >= Dur::ZERO && since_noon <= Dur::from_secs(30 * 60));
                if since_noon > Dur::ZERO {
                    late += 1;
                }
            }
            0 => assert_eq!(began, hour(24 * 10 + 6)),
            other => panic!("part {other} at one in the afternoon"),
        }
        // The same every time it is asked.
        assert_eq!(
            agent::routine::entry_at(def, day, hour(24 * 10 + 13), seed),
            Some((entry, began))
        );
        // At night, asleep since ten.
        let (entry, began) = agent::routine::entry_at(def, day, hour(24 * 10 + 23), seed).unwrap();
        assert_eq!((entry, began), (2, hour(24 * 10 + 22)));
        // Before six, last night's part runs on.
        let (entry, began) = agent::routine::entry_at(def, day, hour(24 * 10 + 3), seed).unwrap();
        assert_eq!((entry, began), (2, hour(24 * 9 + 22)));
    }
    assert!(
        (60..140).contains(&kept),
        "about half keep the part: {kept}"
    );
    assert!(
        late > kept / 2,
        "most begin it a little late: {late} of {kept}"
    );
}

#[test]
fn ten_thousand_idle_agents_schedule_nothing_beyond_their_next_wake_up() {
    // Invariant N7: every cold resident has one entry in the one
    // schedule, and the timeline one wake-up for them all.
    let mut sim = sim(5);
    let realm = common::realm(&sim);
    for i in 0..10_000 {
        // Far beyond the tracts that are loaded.
        let home = CellPos::new(2_000 + (i % 100) * 3, 2_000 + (i / 100) * 3, 0);
        agent::add_resident(&mut sim, realm, home, "keeper").expect("a resident");
    }
    let layer = sim.agents();
    assert_eq!(layer.residents().count(), 10_000);
    assert!(
        layer
            .residents()
            .all(|(_, resident)| resident.entity.is_none())
    );
    assert_eq!(layer.scheduled(), 10_000);
    let first = layer.timeline_wake().expect("one wake-up on the timeline");
    let entries = sim.timeline().wake_count();
    // A day passes: they wake at their parts, and each has one entry
    // again after every wake-up.
    let steps = sim.steps();
    sim.advance(clash_sim::AdvanceLimit::Until(Tick::ZERO + Dur::DAY));
    let woken = sim.steps() - steps;
    assert_eq!(sim.agents().scheduled(), 10_000);
    assert_eq!(
        sim.timeline().wake_count(),
        entries,
        "nothing more is scheduled"
    );
    assert!(sim.agents().timeline_wake().unwrap() > first);
    // Every wake-up the timeline handled was a part of somebody's day:
    // at most three parts and a late noon for each, and many share one.
    assert!(woken > 0 && woken < 10_000, "{woken} wake-ups");
    // Each is where its routine has it, at home.
    for (key, resident) in sim.agents().residents().take(50) {
        let state = resident.agent.as_ref().unwrap();
        assert!(matches!(state.at, AgentPlace::At { loc } if loc.cell == key.home));
        assert!(["up", "busy", "asleep"].contains(&state.doing.what.as_str()));
        assert!(state.wake.is_some_and(|wake| wake > sim.now()));
    }
}

fn entity_value(sim: &Sim, entity: clash_core::EntityId, source: &str) -> Value {
    let registry = sim.exprs().registry();
    let kind = Type::Subject(registry.kind_id("entity").unwrap());
    let context = Context::new().subject("self", kind);
    let compiled = clash_expr::compile(source, registry, &context, &Type::Any)
        .value
        .unwrap_or_else(|| panic!("`{source}` compiles"));
    sim.eval(
        &compiled,
        Bound::new().subject(sim.subject_value(Subject::Entity(entity))),
    )
}

#[test]
fn a_resident_in_view_says_whose_home_it_has_and_where_its_routine_puts_it() {
    // 09 §10.6: the properties of a resident in the world.
    let mut sim = sim(3);
    let realm = common::realm(&sim);
    let home = CellPos::new(10, 10, 0);
    let key = agent::add_resident(&mut sim, realm, home, "keeper").expect("a resident");
    let entity = sim
        .agents()
        .resident(&key)
        .and_then(|resident| resident.entity)
        .expect("a home in a loaded tract is in view");
    let at_home = |source: &str| entity_value(&sim, entity, source);
    assert_eq!(at_home("self.resident"), Value::Bool(true));
    assert_eq!(
        at_home("self.resident_home.x == 10 and self.resident_home.y == 10"),
        Value::Bool(true)
    );
    assert_eq!(at_home("self.routine"), Value::from("core:three_parts"));
    // Every part of its day is at home: it is there, doing one of them.
    let doing = at_home("self.routine_doing");
    assert!(
        ["up", "busy", "asleep"].contains(&doing.as_str().unwrap_or_default()),
        "{doing:?}"
    );
    assert_eq!(at_home("len(self.routine_places)"), Value::Int(1));
    assert_eq!(
        at_home("self.routine_places[0] == self.routine_place"),
        Value::Bool(true)
    );
    assert_eq!(
        at_home("dist(self.routine_place, self.resident_home) < 4"),
        Value::Bool(true)
    );
    assert_eq!(at_home("self.routine_away"), Value::Bool(false));
    // In the world it is no agent: what it does is its mind's.
    assert_eq!(at_home("self.agent_doing"), Value::Null);
}
