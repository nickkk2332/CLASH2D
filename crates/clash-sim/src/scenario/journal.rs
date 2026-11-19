//! What the journal shows (14 §13; 12 §10.7): the objectives and
//! missions of the game as the character knows them, with their
//! markers; and the functions rules read the game's state by.
//!
//! # `entity.objectives`
//!
//! A list of maps, read of the character itself in a mind context (the
//! journal is an ordinary screen): one row for each mission the
//! character took on and each objective it may see, in order. The
//! scenario's own objectives come first, and are the party's: they are
//! listed for every character of the party. Then each mission the
//! character took on, by number, followed by its objectives. Hidden
//! objectives are not listed.
//!
//! | Field | Is |
//! |-------|----|
//! | `kind` | `objective` or `mission` |
//! | `id` | The objective's id, or the mission definition's qualified id |
//! | `text` | What the character is told: the objective's `text`, or the mission's `name`, rendered for the character in its mind context |
//! | `state` | `active`, `done` or `failed`; for a mission, also `expired` |
//! | `mission` | The mission's number; 0 for the scenario's own |
//! | `depth` | How deep it is nested: a mission's objectives and an objective's children are one deeper |
//! | `marker` | The cell the marker stands at, as the character knows it, or null: for `last_known`, where the character last perceived the entity or was told it was (15 §7.2), so a marker follows belief and not the entity; for `site`, the site's anchor; for `at`, the cell |
//! | `where` | How far and which way the marker is from the character, in words (`north-east, 120 m`), `here`, or empty with no marker |
//!
//! Every value is worked out from what the character was told (the
//! objectives it was given) and what it perceived (where a marker's
//! entity was last seen): nothing of truth beyond the character's own
//! place.

use std::sync::Arc;

use clash_core::{EntityId, Real};
use clash_expr::Value;
use clash_world::Location;

use super::def::{MarkerDef, ObjectiveDef, ScenarioDef};
use super::run::mind_bound;
use super::state::{Objective, ObjectiveState};
use super::world::scenario_named;
use super::{MISSION_DONE, MISSION_EXPIRED, MISSION_FAILED, MISSION_STARTED, OBJECTIVE_CHANGED};
use crate::event::Subject;
use crate::registry::{FunctionDef, PropertyDef};
use crate::sim::Sim;

const GROUP_SCENARIO: &str = "Scenario";

/// The cell a marker stands at, as `taker` knows it.
fn marker_cell(
    sim: &Sim,
    marker: &MarkerDef,
    taker: EntityId,
    params: &[clash_core::Value],
) -> Option<Location> {
    let eval = |expr: &clash_data::Expr<clash_data::expr::Any>| {
        let mut host = sim.host(mind_bound(sim, taker, params));
        expr.eval_on(&sim.defs, &mut host)
    };
    if let Some(expr) = &marker.last_known {
        let Some(Subject::Entity(target)) = eval(expr).and_then(|value| sim.subject_of(&value))
        else {
            return None;
        };
        return crate::perception::belief::believed_loc(sim, taker, target);
    }
    if let Some(expr) = &marker.site {
        let Some(Subject::Site(site)) = eval(expr).and_then(|value| sim.subject_of(&value)) else {
            return None;
        };
        let site = sim.cosmos.site(site)?;
        let grid = sim.cosmos.realm(site.realm)?.base_grid;
        return Some(Location::new(grid, site.anchor));
    }
    if let Some(expr) = &marker.at {
        return match eval(expr).and_then(|value| sim.subject_of(&value)) {
            Some(Subject::Loc(loc) | Subject::Element(loc, _)) => Some(loc),
            _ => None,
        };
    }
    None
}

/// Which way and how far a cell is from another, in words.
fn where_words(from: Option<Location>, to: Option<Location>) -> String {
    let (Some(from), Some(to)) = (from, to) else {
        return String::new();
    };
    if from.grid != to.grid {
        return "elsewhere".to_owned();
    }
    let dx = i64::from(to.cell.x) - i64::from(from.cell.x);
    let dy = i64::from(to.cell.y) - i64::from(from.cell.y);
    let dz = i64::from(to.cell.z) - i64::from(from.cell.z);
    if dx == 0 && dy == 0 {
        return match dz.signum() {
            1 => "above".to_owned(),
            -1 => "below".to_owned(),
            _ => "here".to_owned(),
        };
    }
    // Which of eight ways: a step counts in an axis when it is at least
    // half of the other (tan 22.5° is about 0.41; a half is near enough).
    let (ax, ay) = (dx.abs(), dy.abs());
    let north_south = if ay * 2 >= ax {
        if dy < 0 { "north" } else { "south" }
    } else {
        ""
    };
    let east_west = if ax * 2 >= ay {
        if dx > 0 { "east" } else { "west" }
    } else {
        ""
    };
    let way = match (north_south, east_west) {
        ("", ew) => ew.to_owned(),
        (ns, "") => ns.to_owned(),
        (ns, ew) => format!("{ns}-{ew}"),
    };
    let metres = Real::from_i64(dx * dx + dy * dy).sqrt().round().to_i64();
    format!("{way}, {metres} m")
}

/// One row of the list.
fn row(fields: Vec<(&str, Value)>) -> Value {
    Value::map(fields)
}

/// The rows of a list of objectives, as the character they are for
/// knows them: the scenario's own (`mission` 0) or a mission's, one
/// deeper.
fn objective_rows(
    sim: &Sim,
    defs: &[ObjectiveDef],
    states: &[Objective],
    reader: EntityId,
    params: &[clash_core::Value],
    mission: u32,
    out: &mut Vec<Value>,
) {
    let (taker, base) = (reader, i64::from(mission > 0));
    let here = sim.truth().location(reader);
    for (def, state) in defs.iter().zip(states) {
        if state.state == ObjectiveState::Hidden {
            continue;
        }
        let text = {
            let mut host = sim.host_for_reader(mind_bound(sim, taker, params), reader);
            def.text.render_on(&sim.defs, &mut host)
        };
        let marker = def
            .marker
            .as_ref()
            .filter(|_| state.state == ObjectiveState::Active)
            .and_then(|marker| marker_cell(sim, marker, taker, params));
        // How deep: one more for each parent.
        let mut depth = base;
        let mut parent = def.parent.clone();
        while let Some(name) = parent {
            depth += 1;
            parent = defs
                .iter()
                .find(|other| other.id.value == name)
                .and_then(|other| other.parent.clone())
                .filter(|_| depth < 32);
        }
        out.push(row(vec![
            ("kind", Value::from("objective")),
            ("id", Value::from(state.id.as_str())),
            ("text", Value::from(text.as_str())),
            ("state", Value::from(state.state.name())),
            ("mission", Value::Int(i64::from(mission))),
            ("depth", Value::Int(depth)),
            (
                "marker",
                marker.map_or(Value::Null, |loc| sim.subject_value(Subject::Loc(loc))),
            ),
            ("where", Value::from(where_words(here, marker).as_str())),
        ]));
    }
}

/// The journal of a character: see the [module](self) documentation.
pub fn journal(sim: &Sim, reader: EntityId) -> Vec<Value> {
    let mut rows = Vec::new();
    let Some(game) = &sim.scenario.game else {
        return rows;
    };
    let defs = Arc::clone(&sim.defs);
    if game.party.contains(&reader)
        && let Ok(def) = scenario_named(&defs, &game.scenario)
    {
        let def: &ScenarioDef = def;
        objective_rows(
            sim,
            &def.objectives,
            &game.objectives,
            reader,
            &[],
            0,
            &mut rows,
        );
    }
    let missions = defs.try_registry::<super::MissionDef>();
    for mission in game.missions.iter().filter(|m| m.taker == reader) {
        let Some(def) = missions.and_then(|r| r.lookup(&mission.def).map(|id| r.get(id))) else {
            continue;
        };
        let name = match &def.name {
            Some(name) => {
                let mut host =
                    sim.host_for_reader(mind_bound(sim, reader, &mission.params), reader);
                name.render_on(&sim.defs, &mut host)
            }
            None => mission.def.clone(),
        };
        rows.push(row(vec![
            ("kind", Value::from("mission")),
            ("id", Value::from(mission.def.as_str())),
            ("text", Value::from(name.as_str())),
            ("state", Value::from(mission.state.name())),
            ("mission", Value::Int(i64::from(mission.serial))),
            ("depth", Value::Int(0)),
            ("marker", Value::Null),
            ("where", Value::from("")),
        ]));
        objective_rows(
            sim,
            &def.objectives,
            &mission.objectives,
            reader,
            &mission.params,
            mission.serial,
            &mut rows,
        );
    }
    rows
}

/// The properties of scenarios.
pub fn properties() -> Vec<PropertyDef> {
    vec![
        PropertyDef::new(
            "entity",
            "objectives",
            "list<map<any>>",
            "The game's objectives and missions as the character knows them, for its journal (14 §13): one row for each, with `kind`, `id`, `text`, `state`, `mission`, `depth`, `marker` and `where`. Read of the character itself in a mind context; a marker follows what the character last perceived of its target.",
            |sim, subject, _| match subject {
                Subject::Entity(who) => Value::list(journal(sim, who)),
                _ => Value::Null,
            },
        )
        .changed_by(&[
            OBJECTIVE_CHANGED,
            MISSION_STARTED,
            MISSION_DONE,
            MISSION_FAILED,
            MISSION_EXPIRED,
        ]),
        PropertyDef::new(
            "entity",
            "site",
            "site?",
            "The site of the cell the entity is in, its own or that of whatever it is inside (02 §8.4): null outside every site and out of the world. Kept current for an objective by `entity_relocated`.",
            |sim, subject, _| {
                let Subject::Entity(who) = subject else {
                    return Value::Null;
                };
                sim.truth()
                    .location(who)
                    .and_then(|loc| sim.cosmos.site_of(loc).ok().flatten())
                    .map_or(Value::Null, |site| sim.subject_value(Subject::Site(site)))
            },
        )
        .changed_by(&["entity_relocated"]),
    ]
}

/// The game's own objective of an id, or a mission's.
fn state_of(sim: &Sim, args: &[Value]) -> Value {
    let Some(id) = args.first().and_then(Value::as_str) else {
        return Value::Null;
    };
    let mission = args
        .get(1)
        .and_then(Value::as_int)
        .and_then(|n| u32::try_from(n).ok())
        .filter(|n| *n > 0);
    super::objective_state(sim, mission, id).map_or(Value::Null, |state| Value::from(state.name()))
}

/// The functions of scenarios.
pub fn functions() -> Vec<FunctionDef> {
    vec![
        FunctionDef::new(
            "objective_state",
            "string?",
            GROUP_SCENARIO,
            "Where an objective of the game stands (13 §5.1): `hidden`, `active`, `done` or `failed`; null if the game has none of that id.",
            state_of,
        )
        .arg("id", "string", "The objective's id.")
        .optional("mission", "int", "The number of the mission it is part of; left out, one of the scenario's own.")
        .safe()
        .changed_by(&[OBJECTIVE_CHANGED, MISSION_STARTED]),
        FunctionDef::new(
            "mission_state",
            "string?",
            GROUP_SCENARIO,
            "Where a mission of the game stands, by its number (13 §5.2): `active`, `done`, `failed` or `expired`; null if there is none.",
            |sim, args| {
                let serial = args
                    .first()
                    .and_then(Value::as_int)
                    .and_then(|n| u32::try_from(n).ok());
                serial
                    .and_then(|serial| sim.scenario.game.as_ref()?.mission(serial))
                    .map_or(Value::Null, |mission| Value::from(mission.state.name()))
            },
        )
        .arg("mission", "int", "The mission's number.")
        .safe()
        .changed_by(&[MISSION_STARTED, MISSION_DONE, MISSION_FAILED, MISSION_EXPIRED]),
        FunctionDef::new(
            "scenario_id",
            "string?",
            GROUP_SCENARIO,
            "The qualified id of the scenario being played (13 §3); null if none is.",
            |sim, _| {
                sim.scenario
                    .game
                    .as_ref()
                    .map_or(Value::Null, |game| Value::from(game.scenario.as_str()))
            },
        )
        .safe()
        .changed_by(&[super::SCENARIO_STARTED]),
        FunctionDef::new(
            "party",
            "list<entity>",
            GROUP_SCENARIO,
            "The characters the player plays in the game, in the order they were taken up: the last is the one played now (13 §9.3). Empty with no game.",
            |sim, _| {
                let party = sim
                    .scenario
                    .game
                    .as_ref()
                    .map(|game| game.party.clone())
                    .unwrap_or_default();
                Value::list(
                    party
                        .into_iter()
                        .map(|who| sim.subject_value(Subject::Entity(who))),
                )
            },
        )
        .safe()
        .changed_by(&["control_changed"]),
        FunctionDef::new(
            "fallen",
            "list<entity>",
            GROUP_SCENARIO,
            "The characters the player played who are gone, in the order they fell (13 §9.3). Empty with no game.",
            |sim, _| {
                let fallen = sim
                    .scenario
                    .game
                    .as_ref()
                    .map(|game| game.fallen.clone())
                    .unwrap_or_default();
                Value::list(
                    fallen
                        .into_iter()
                        .map(|who| sim.subject_value(Subject::Entity(who))),
                )
            },
        )
        .safe()
        .changed_by(&["control_changed"]),
        FunctionDef::new(
            "played_for",
            "real",
            GROUP_SCENARIO,
            "How long the game has run, in seconds of universal time since it began; 0 with no game.",
            |sim, _| Value::Real(super::run::played(sim).as_secs_real()),
        )
        .safe(),
    ]
}
