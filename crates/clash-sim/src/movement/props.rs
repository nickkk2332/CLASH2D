//! Movement's properties (06 §13), all of an entity.
//!
//! None is marked safe for belief (D-466): each is about the entity it
//! is read of, which a mind context may read of its own subject and of
//! no other (05 §7.3). Whatever reads the world beyond the entity
//! (`can_step`, `step_fault`, `step_time`, `can_reach`, `path_cost`)
//! answers from what the entity believes ([`belief`](super::belief)),
//! so that a character may decide by it; the rest read the entity's own
//! state.

use clash_core::EntityId;
use clash_data::Def;
use clash_expr::Value;
use clash_world::{Location, StepKind};

use crate::Sim;
use crate::event::Subject;
use crate::registry::{EngineDefTypes, PropertyDef};

use super::belief::{believed_rules, believed_step, believed_step_time};
use super::defs::PostureDef;
use super::mover::{Journey, Mover};
use super::rules::mover_rules;
use super::step::{dir_name, fault_reason, parse_dir};
use super::{path, posture};

/// The entity a subject is, if it is one that exists.
fn entity(sim: &Sim, subject: Subject) -> Option<EntityId> {
    let id = subject.entity()?;
    sim.entities.contains(id).then_some(id)
}

/// The direction an argument names.
fn dir_arg(args: &[Value]) -> Option<StepKind> {
    args.first().and_then(Value::as_str).and_then(parse_dir)
}

/// The cell an argument names.
fn loc_arg(sim: &Sim, args: &[Value]) -> Option<Location> {
    match sim.subject_of(args.first()?)? {
        Subject::Loc(loc) | Subject::Element(loc, _) => Some(loc),
        _ => None,
    }
}

/// The path an entity is following.
fn journey(sim: &Sim, subject: Subject) -> Option<&Journey> {
    sim.components()
        .get::<Mover>(entity(sim, subject)?)?
        .journey()
}

/// Why an entity believes it cannot step in a direction, or `None` if
/// it believes it can.
fn believed_fault(sim: &Sim, id: EntityId, kind: StepKind) -> Option<String> {
    let Some(step) = believed_step(sim, id, kind) else {
        return Some("there is nothing there to step into".to_owned());
    };
    believed_rules(sim, id)
        .judge(&step)
        .err()
        .map(|fault| fault_reason(fault, id, &step).message)
}

/// The step that leads from where an entity stands to a cell next to
/// it, if the cell is next to it.
fn step_toward(sim: &Sim, id: EntityId, to: Location) -> Option<StepKind> {
    let from = sim.truth().place(id)?.loc()?;
    let delta = sim.truth().cosmos().delta(from, to)?;
    StepKind::ALL.into_iter().find(|kind| kind.delta() == delta)
}

/// Movement's properties, for the engine's table.
pub fn properties() -> Vec<PropertyDef> {
    let p = PropertyDef::new;
    vec![
        p(
            "entity",
            "posture",
            "id<posture>?",
            "The posture the entity is in (06 §11.1). Null in none.",
            |sim, subject, _| {
                let found = entity(sim, subject).and_then(|id| posture::posture_of(sim, id));
                match found {
                    Some(id) => EngineDefTypes::id(
                        sim.exprs().registry().def_type_id(PostureDef::TYPE),
                        id.index(),
                    ),
                    None => Value::Null,
                }
            },
        )
        .changed_by(&["posture_changed"]),
        p(
            "entity",
            "speed",
            "real?",
            "How fast the entity moves now by the fastest of its ways of moving that works, its posture included, in metres for each second of its own time. Null if it cannot move.",
            |sim, subject, _| {
                let rules = entity(sim, subject).map(|id| mover_rules(sim, id));
                match rules {
                    Some(rules) if rules.can_move() => Value::Real(rules.best_speed()),
                    _ => Value::Null,
                }
            },
        ),
        p(
            "entity",
            "can_step",
            "bool",
            "Whether the entity believes it can step in a direction from where it stands: a cell is there and one of its ways of moving makes the step. Who else is in the cell is not asked.",
            |sim, subject, args| {
                let can = entity(sim, subject)
                    .zip(dir_arg(args))
                    .is_some_and(|(id, kind)| believed_fault(sim, id, kind).is_none());
                Value::Bool(can)
            },
        )
        .arg("dir", "string", "The direction: `north` to `north_west`, `up` or `down`."),
        p(
            "entity",
            "step_fault",
            "string?",
            "Why the entity believes it cannot step in a direction, as a sentence; null if it believes it can.",
            |sim, subject, args| {
                let Some(id) = entity(sim, subject) else {
                    return Value::Null;
                };
                let Some(kind) = dir_arg(args) else {
                    return Value::text("that is not a direction");
                };
                believed_fault(sim, id, kind).map_or(Value::Null, Value::text)
            },
        )
        .arg("dir", "string", "The direction: `north` to `north_west`, `up` or `down`."),
        p(
            "entity",
            "step_time",
            "real?",
            "How long the entity expects a step in a direction to take, in seconds of its own time: the length of the step it believes lies there, with what its way of moving makes of rough ground and climbing, over that way's speed now. What a phase's `time` waits before the step is taken and checked against the world as it then is. Null if it believes it cannot make the step, or `dir` is not a direction.",
            |sim, subject, args| {
                entity(sim, subject)
                    .zip(dir_arg(args))
                    .and_then(|(id, kind)| believed_step_time(sim, id, kind))
                    .map_or(Value::Null, |time| Value::Real(time.as_secs_real()))
            },
        )
        .arg("dir", "string", "The direction: `north` to `north_west`, `up` or `down`."),
        p(
            "entity",
            "can_reach",
            "bool",
            "Whether a cell is next to the entity and it believes it can step into it.",
            |sim, subject, args| {
                let can = entity(sim, subject)
                    .zip(loc_arg(sim, args))
                    .and_then(|(id, to)| Some((id, step_toward(sim, id, to)?)))
                    .is_some_and(|(id, kind)| believed_fault(sim, id, kind).is_none());
                Value::Bool(can)
            },
        )
        .arg("to", "loc", "The cell."),
        p(
            "entity",
            "path_cost",
            "real?",
            "What the way from where the entity stands to a cell costs, as it believes the way to be: metres of travel at its best speed (09 §6.4). Null if it believes there is none. A search each time it is read.",
            |sim, subject, args| {
                entity(sim, subject)
                    .zip(loc_arg(sim, args))
                    .and_then(|(id, to)| path::path_cost(sim, id, to))
                    .map_or(Value::Null, Value::Real)
            },
        )
        .arg("to", "loc", "The cell."),
        p(
            "entity",
            "path_left",
            "int?",
            "How many steps of the path the entity is following are left. Null if it is following none.",
            |sim, subject, _| {
                journey(sim, subject).map_or(Value::Null, |journey| {
                    Value::Int(i64::try_from(journey.remaining().len()).unwrap_or(i64::MAX))
                })
            },
        ),
        p(
            "entity",
            "path_next",
            "string?",
            "The direction of the next step of the path the entity is following. Null if it is following none, or is not next to the path's next cell.",
            |sim, subject, _| {
                let next = entity(sim, subject).zip(journey(sim, subject)).and_then(
                    |(id, journey)| {
                        let from = sim.truth().place(id)?.loc()?;
                        step_toward(sim, id, Location::new(from.grid, journey.next()?))
                    },
                );
                next.map_or(Value::Null, |kind| Value::text(dir_name(kind)))
            },
        ),
        p(
            "entity",
            "path_stale",
            "bool",
            "Whether the path the entity is following must be planned again before its next step: a step of it was refused, or the entity is not where the path left it.",
            |sim, subject, _| {
                Value::Bool(journey(sim, subject).is_some_and(Journey::is_stale))
            },
        ),
    ]
}
