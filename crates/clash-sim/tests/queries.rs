//! Queries and value sources (plans/sim-kernel.md, package K6; 05 §7.4,
//! §11): every kind the simulation answers, `where`, `sort`, `limit`
//! and `weight`, the value sources over a query, `exists` in a
//! condition, `for_each` over a query, the extent cap, load errors, and
//! the order of results against a brute-force reference.

mod common;

use clash_core::{EntityId, Real};
use clash_expr::{QueryCall, Value};
use clash_sim::{EntitySpec, EventBody, Place, Sim, Subject};
use clash_world::dims::STOREY_M;
use clash_world::{Cause, CellPos, Location, distance_m};
use common::{GRID, SIDE, at, creature, item, on, save_and_load, sim_over, spawn, try_defs_with};
use proptest::prelude::*;

const RULES: &str = r#"[
  { "type": "variable", "id": "near", "scope": "global", "value": "int", "default": 0, "doc": "" },
  { "type": "variable", "id": "nearest", "scope": "global", "value": "entity", "doc": "" },
  { "type": "variable", "id": "guards", "scope": "global", "value": "list<entity>", "doc": "" },
  { "type": "variable", "id": "weight", "scope": "global", "value": "number", "doc": "" },
  { "type": "variable", "id": "heaviest", "scope": "global", "value": "number", "doc": "" },
  { "type": "variable", "id": "thief_seen", "scope": "global", "value": "bool", "default": false, "doc": "" },
  { "type": "variable", "id": "ground", "scope": "global", "value": "int", "default": 0, "doc": "" },
  { "type": "variable", "id": "materials", "scope": "global", "value": "int", "default": 0, "doc": "" },
  { "type": "variable", "id": "listed", "scope": "global", "value": "int", "default": 0, "doc": "" },
  { "type": "variable", "id": "picked", "scope": "global", "value": "entity", "doc": "" },
  { "type": "variable", "id": "held", "scope": "global", "value": "int", "default": 0, "doc": "" },
  { "type": "variable", "id": "rank", "scope": "entity", "value": "int", "default": 0, "doc": "" },
  { "type": "variable", "id": "big", "scope": "global", "value": "int", "default": -1, "doc": "" },

  { "type": "eoc", "id": "survey", "subjects": { "self": "entity" },
    "doc": "Asks every kind of question of the world round `self`.",
    "do": [
      { "set": { "global.var.near": { "count": { "entities": { "around": "self", "radius": 3, "where": "it != self" } } } } },
      { "set": { "global.var.nearest": { "first": { "entities": { "around": "self", "radius": 30, "where": "it != self" } } } } },
      { "set": { "global.var.guards": { "collect": { "entities": { "tag": "guard" } } } } },
      { "set": { "global.var.weight": { "sum": { "entities": { "kind": "creature" } }, "of": "it.var.rank" } } },
      { "set": { "global.var.heaviest": { "max": { "entities": { "kind": "creature" } }, "of": "it.var.rank" } } },
      { "if": { "exists": { "entities": { "tag": "thief" } } },
        "then": [ { "set": { "global.var.thief_seen": true } } ] },
      { "set": { "global.var.ground": { "count": { "cells": { "around": "self", "radius": 1.5 } } } } },
      { "set": { "global.var.materials": { "count": { "defs": { "type": "material" } } } } },
      { "set": { "global.var.listed": { "sum": { "list": { "items": "[1, 2, 3, 4]", "where": "it > 1", "limit": 2 } } } } },
      { "set": { "global.var.picked": { "pick": { "entities": { "tag": "guard" } }, "weight": "it.var.rank" } } },
      { "set": { "global.var.held": { "count": { "contents": { "of": "self", "deep": true } } } } },
      { "for_each": { "entities": { "around": "self", "radius": 30, "where": "it.has_tag('guard')",
                                    "sort": "0 - it.var.rank", "limit": 2 } },
        "as": "g",
        "do": [ { "add_tag": { "to": "$g", "tag": "alerted" } } ] }
    ] },

  { "type": "variable", "id": "east", "scope": "global", "value": "number", "default": -1, "doc": "" },
  { "type": "variable", "id": "rank_near", "scope": "global", "value": "int", "default": -1, "doc": "" },

  { "type": "eoc", "id": "typed_locals", "subjects": { "self": "entity" },
    "doc": "A local made by `let` from a query is of the type of what it holds: `$cell.x` reads the cell's property, `$g.var.rank` the guard's variable (D-905).",
    "do": [
      { "let": { "cell": { "first": { "cells": { "around": "self", "radius": 1.5, "where": "it.x > self.loc.x" } } },
                 "g": { "first": { "entities": { "around": "self", "radius": 30, "where": "it.has_tag('guard')" } } } } },
      { "set": { "global.var.east": "$cell.x - self.loc.x", "global.var.rank_near": "$g.var.rank" } } ] },

  { "type": "eoc", "id": "wide", "subjects": { "self": "entity" },
    "params": { "reach": "real" },
    "doc": "A query whose size is only known when it runs.",
    "do": [ { "set": { "global.var.big": { "count": { "cells": { "around": "self", "radius": "$reach" } } } } } ] }
]"#;

fn defs() -> std::sync::Arc<clash_data::Defs> {
    try_defs_with(&[("data/test/queries.json", RULES)])
        .unwrap_or_else(|error| panic!("the test definitions do not load:\n{error}"))
}

fn global(sim: &Sim, name: &str) -> Value {
    sim.var(Subject::Global, name)
}

fn entity(sim: &Sim, id: EntityId) -> Value {
    sim.subject_value(Subject::Entity(id))
}

fn set_rank(sim: &mut Sim, id: EntityId, rank: i64) {
    sim.set_var(
        Subject::Entity(id),
        "core:rank",
        Value::Int(rank),
        Cause::DEBUG,
    )
    .unwrap();
}

fn call(sim: &mut Sim, rule: &str, who: EntityId, args: &[(&str, Value)]) {
    let id = sim.rule_set().lookup(rule).unwrap();
    let me = entity(sim, who);
    sim.call_rule(id, &[("self", me)], args, Cause::DEBUG)
        .unwrap();
}

fn faults(sim: &mut Sim) -> Vec<String> {
    sim.drain_events()
        .iter()
        .filter_map(|event| match &event.body {
            EventBody::Fault { reason } => Some(format!("{}: {}", reason.code, reason.message)),
            _ => None,
        })
        .collect()
}

/// A watcher at (10, 10), guards and a thief round it, and a pack.
fn scene() -> (Sim, EntityId, [EntityId; 3], EntityId) {
    let mut sim = sim_over(defs(), 11);
    let watcher = spawn(&mut sim, creature(), on(10, 10));
    let near = spawn(&mut sim, creature().with_tag("guard"), on(12, 10));
    let far = spawn(&mut sim, creature().with_tag("guard"), on(20, 10));
    let other = spawn(&mut sim, creature().with_tag("guard"), on(10, 25));
    let thief = spawn(&mut sim, creature().with_tag("thief"), on(9, 9));
    let pack = spawn(&mut sim, item(), Place::In(watcher));
    let _coin = spawn(&mut sim, item(), Place::In(pack));
    set_rank(&mut sim, near, 1);
    set_rank(&mut sim, far, 5);
    set_rank(&mut sim, other, 3);
    let _ = thief;
    (sim, watcher, [near, far, other], thief)
}

#[test]
fn value_sources_over_queries_read_the_world() {
    let (mut sim, watcher, [near, far, other], _thief) = scene();
    faults(&mut sim);
    call(&mut sim, "core:survey", watcher, &[]);
    assert_eq!(faults(&mut sim), Vec::<String>::new());

    // Within 3 m: the near guard and the thief, not the watcher.
    assert_eq!(global(&sim, "core:near"), Value::Int(2));
    // Nearest first: the thief stands diagonally, about 1.4 m away.
    let thief = global(&sim, "core:nearest");
    assert_ne!(thief, Value::Null);
    assert_ne!(thief, entity(&sim, near));
    // Every guard in the world, by id.
    assert_eq!(
        global(&sim, "core:guards"),
        Value::list([entity(&sim, near), entity(&sim, far), entity(&sim, other)])
    );
    assert_eq!(global(&sim, "core:weight"), Value::Int(9));
    assert_eq!(global(&sim, "core:heaviest"), Value::Int(5));
    assert_eq!(global(&sim, "core:thief_seen"), Value::Bool(true));
    // On flat ground the cells within 1.5 m are the nine of the storey.
    assert_eq!(global(&sim, "core:ground"), Value::Int(9));
    let materials = sim
        .defs()
        .expr_registry()
        .def_type_id("material")
        .and_then(|ty| sim.defs().expr_registry().def_type(ty))
        .map_or(0, |ty| ty.ids().count());
    assert!(materials > 0);
    assert_eq!(global(&sim, "core:materials"), Value::Int(materials as i64));
    // 2 + 3: the limit cuts after the filter.
    assert_eq!(global(&sim, "core:listed"), Value::Int(5));
    let picked = global(&sim, "core:picked");
    assert!(
        [near, far, other]
            .iter()
            .any(|id| entity(&sim, *id) == picked)
    );
    // The pack and the coin in it.
    assert_eq!(global(&sim, "core:held"), Value::Int(2));
    // Sorted by rank, highest first, and cut to two.
    let tagged = |sim: &Sim, id: EntityId| sim.truth().entity(id).unwrap().tags.contains("alerted");
    assert!(tagged(&sim, far));
    assert!(tagged(&sim, other));
    assert!(!tagged(&sim, near));
}

#[test]
fn a_local_made_from_a_query_reads_the_properties_of_what_it_holds() {
    let (mut sim, watcher, _, _thief) = scene();
    faults(&mut sim);
    call(&mut sim, "core:typed_locals", watcher, &[]);
    assert_eq!(faults(&mut sim), Vec::<String>::new());
    // A cell east of the watcher, and the nearest guard's rank.
    assert_eq!(global(&sim, "core:east"), Value::Int(1));
    assert_eq!(global(&sim, "core:rank_near"), Value::Int(1));
}

#[test]
fn queries_run_the_same_after_a_save() {
    let (mut sim, watcher, _, _) = scene();
    let (mut loaded, unplaced) = save_and_load(&sim);
    assert!(unplaced.is_empty());
    call(&mut sim, "core:survey", watcher, &[]);
    call(&mut loaded, "core:survey", watcher, &[]);
    assert_eq!(sim.state_hash(), loaded.state_hash());
}

#[test]
fn a_query_larger_than_the_cap_is_refused_when_it_runs() {
    let (mut sim, watcher, _, _) = scene();
    faults(&mut sim);
    call(
        &mut sim,
        "core:wide",
        watcher,
        &[("reach", Value::Real(Real::of(2.0)))],
    );
    assert_eq!(global(&sim, "core:big"), Value::Int(13));
    sim.set_setting("rules.max_query_cells", 20_i64);
    call(
        &mut sim,
        "core:wide",
        watcher,
        &[("reach", Value::Real(Real::of(2.0)))],
    );
    // Refused: the value source gave null, which unsets the variable.
    assert_eq!(global(&sim, "core:big"), Value::Int(-1));
    let reported = faults(&mut sim);
    assert!(
        reported
            .iter()
            .any(|fault| fault.contains("max_query_cells")),
        "{reported:?}"
    );
}

fn load_error(rules: &str) -> String {
    match try_defs_with(&[("data/test/bad.json", rules)]) {
        Ok(_) => panic!("loaded:\n{rules}"),
        Err(error) => error,
    }
}

#[test]
fn a_constant_query_larger_than_the_cap_is_a_load_error() {
    let error = load_error(
        r#"[{ "type": "eoc", "id": "huge", "subjects": { "self": "entity" },
              "do": [ { "for_each": { "cells": { "around": "self", "radius": 500 } }, "as": "c", "do": [] } ] }]"#,
    );
    assert!(error.contains("E0678"), "{error}");
}

#[test]
fn queries_that_are_not_well_formed_are_load_errors() {
    let unknown = load_error(
        r#"[{ "type": "eoc", "id": "a", "subjects": { "self": "entity" },
              "do": [ { "let": { "n": { "count": { "entites": { "tag": "x" } } } } } ] }]"#,
    );
    assert!(
        unknown.contains("E0677") && unknown.contains("entities"),
        "{unknown}"
    );
    let shapes = load_error(
        r#"[{ "type": "eoc", "id": "a", "subjects": { "self": "entity" },
              "do": [ { "let": { "n": { "count": { "entities": { "at": "self.loc", "box": "[self.loc, self.loc]" } } } } } ] }]"#,
    );
    assert!(shapes.contains("E0677"), "{shapes}");
    let alone = load_error(
        r#"[{ "type": "eoc", "id": "a", "subjects": { "self": "entity" },
              "do": [ { "let": { "n": { "count": { "entities": { "radius": 3 } } } } } ] }]"#,
    );
    assert!(
        alone.contains("E0677") && alone.contains("around"),
        "{alone}"
    );
    let typo = load_error(
        r#"[{ "type": "eoc", "id": "a", "subjects": { "self": "entity" },
              "do": [ { "let": { "n": { "count": { "entities": { "tags": "x" } } } } } ] }]"#,
    );
    assert!(typo.contains("tag"), "{typo}");
    let wrong = load_error(
        r#"[{ "type": "eoc", "id": "a", "subjects": { "self": "entity" },
              "do": [ { "let": { "n": { "count": { "entities": { "where": "it + 1" } } } } } ] }]"#,
    );
    assert!(wrong.contains("E06"), "{wrong}");
}

#[test]
fn a_mind_context_may_not_ask_the_world() {
    let error = load_error(
        r#"[{ "type": "action", "id": "spot",
              "requires": { "exists": { "entities": { "around": "self", "radius": 5 } } },
              "phases": [ { "id": "look", "time": "1 s" } ] }]"#,
    );
    assert!(
        error.contains("E0662") && error.contains("perception"),
        "{error}"
    );
    // What every character may know is allowed.
    try_defs_with(&[(
        "data/test/ok.json",
        r#"[{ "type": "action", "id": "count_stuff",
              "requires": { "exists": { "defs": { "type": "material" } } },
              "phases": [ { "id": "look", "time": "1 s" } ] }]"#,
    )])
    .unwrap();
}

// ---- the order of results against a brute-force reference ------------------

fn query(sim: &Sim, kind: &str, args: Vec<Value>) -> Vec<Value> {
    let id = sim.defs().expr_registry().query_id(kind).unwrap();
    let mut out = Vec::new();
    sim.answer_query(
        &QueryCall {
            query: id,
            args: &args,
            observer: None,
        },
        &mut out,
    )
    .unwrap();
    out
}

fn loc_value(sim: &Sim, x: i32, y: i32, z: i32) -> Value {
    sim.subject_value(Subject::Loc(Location::new(GRID, CellPos::new(x, y, z))))
}

/// Every entity standing within `radius` of a cell, by the nearest cell
/// it fills and then by id, worked out the slow way.
fn entities_near(sim: &Sim, centre: Location, radius: Real) -> Vec<Value> {
    let mut found: Vec<(Real, EntityId)> = Vec::new();
    for record in sim.truth().entities().iter() {
        let Place::At { loc, .. } = record.place() else {
            continue;
        };
        let footprint = record.presence().footprint;
        let best = footprint
            .cells(loc.cell)
            .cells()
            .filter_map(|cell| {
                let cell = sim
                    .truth()
                    .cosmos()
                    .normalize(Location::new(GRID, cell))
                    .ok()?;
                sim.truth().cosmos().delta(centre, cell).map(distance_m)
            })
            .fold(None, |best: Option<Real>, d| {
                Some(best.map_or(d, |b| b.min(d)))
            });
        if let Some(distance) = best.filter(|d| *d <= radius) {
            found.push((distance, record.id()));
        }
    }
    found.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap().then(a.1.cmp(&b.1)));
    found.into_iter().map(|(_, id)| entity(sim, id)).collect()
}

/// Every loaded cell within `radius` of a cell, nearest first and then
/// storey by storey and row by row, worked out the slow way.
fn cells_near(sim: &Sim, centre: Location, radius: Real) -> Vec<Value> {
    let mut found: Vec<(Real, CellPos)> = Vec::new();
    let storeys = (radius / STOREY_M).ceil().to_i32() + 1;
    for z in -storeys..=storeys {
        for y in 0..SIDE {
            for x in 0..SIDE {
                let loc = Location::new(GRID, CellPos::new(x, y, z));
                if !sim.truth().cosmos().is_loaded(loc) {
                    continue;
                }
                let Some(distance) = sim.truth().cosmos().delta(centre, loc).map(distance_m) else {
                    continue;
                };
                if distance <= radius {
                    found.push((distance, loc.cell));
                }
            }
        }
    }
    found.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap().then(a.1.cmp(&b.1)));
    found
        .into_iter()
        .map(|(_, cell)| loc_value(sim, cell.x, cell.y, cell.z))
        .collect()
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(24))]

    #[test]
    fn entities_near_a_place_come_nearest_first_as_the_reference_says(
        spots in prop::collection::vec((0..SIDE, 0..SIDE, 0..3u8), 1..30),
        centre in (0..SIDE, 0..SIDE),
        radius in 0.0f64..40.0,
    ) {
        let mut sim = sim_over(defs(), 5);
        for (x, y, tags) in spots {
            let mut spec: EntitySpec = if tags == 0 { creature() } else { item() };
            if tags == 2 { spec = spec.with_tag("guard"); }
            spawn(&mut sim, spec, on(x, y));
        }
        let radius = Real::of(radius);
        let centre = at(centre.0, centre.1);
        let mut args = vec![Value::Null; 11];
        args[0] = sim.subject_value(Subject::Loc(centre));
        args[1] = Value::Real(radius);
        let found = query(&sim, "entities", args.clone());
        prop_assert_eq!(&found, &entities_near(&sim, centre, radius));
        // A filter keeps the order.
        args[8] = Value::from("guard");
        let guards = query(&sim, "entities", args);
        let expected: Vec<Value> = found
            .into_iter()
            .filter(|value| match sim.subject_of(value) {
                Some(Subject::Entity(id)) => sim.truth().entity(id).unwrap().tags.contains("guard"),
                _ => false,
            })
            .collect();
        prop_assert_eq!(guards, expected);
    }

    #[test]
    fn cells_near_a_place_come_nearest_first_as_the_reference_says(
        centre in (0..SIDE, 0..SIDE, 0..2i32),
        radius in 0.0f64..5.0,
    ) {
        let sim = sim_over(defs(), 5);
        let radius = Real::of(radius);
        let centre = Location::new(GRID, CellPos::new(centre.0, centre.1, centre.2));
        let mut args = vec![Value::Null; 5];
        args[0] = sim.subject_value(Subject::Loc(centre));
        args[1] = Value::Real(radius);
        prop_assert_eq!(query(&sim, "cells", args), cells_near(&sim, centre, radius));
    }

    #[test]
    fn entities_in_a_box_come_by_cell_then_id(
        spots in prop::collection::vec((0..SIDE, 0..SIDE), 1..30),
        corner in (0..SIDE, 0..SIDE),
        size in (0..20i32, 0..20i32),
    ) {
        let mut sim = sim_over(defs(), 5);
        for (x, y) in spots {
            spawn(&mut sim, item(), on(x, y));
        }
        let (x1, y1) = ((corner.0 + size.0).min(SIDE - 1), (corner.1 + size.1).min(SIDE - 1));
        let mut args = vec![Value::Null; 11];
        args[2] = Value::list([loc_value(&sim, x1, y1, 0), loc_value(&sim, corner.0, corner.1, 0)]);
        let found = query(&sim, "entities", args);
        let mut expected: Vec<(CellPos, EntityId)> = sim
            .truth()
            .entities()
            .iter()
            .filter_map(|record| {
                let cell = record.place().loc()?.cell;
                let inside = (corner.0..=x1).contains(&cell.x) && (corner.1..=y1).contains(&cell.y);
                inside.then_some((cell, record.id()))
            })
            .collect();
        expected.sort();
        let expected: Vec<Value> = expected.into_iter().map(|(_, id)| entity(&sim, id)).collect();
        prop_assert_eq!(found, expected);
    }
}
