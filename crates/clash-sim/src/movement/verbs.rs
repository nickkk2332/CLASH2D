//! Movement's verbs (06 §11, §13): what an action's effects call.
//!
//! | Verb | Arguments | Does |
//! |------|-----------|------|
//! | `step` | `who: entity`, `dir: string`, `drop: bool?` | [`step`](super::step) |
//! | `move_to` | `who: entity`, `to: loc` | [`move_to`](super::move_to) |
//! | `set_posture` | `who: entity`, `posture: id<posture>?` | [`set_posture`](super::set_posture) |
//! | `plan_path` | `who: entity`, `to: loc`, `max_cost: real?` | [`plan_path`](super::plan_path) |
//! | `follow_path` | `who: entity` | [`follow_path`](super::follow_path) |
//! | `clear_path` | `who: entity` | [`clear_path`](super::clear_path) |
//! | `set_movement` | `who: entity`, `modes: list<id<movement>>` | [`set_modes`](super::set_modes) |

use clash_core::EntityId;
use clash_data::Def;
use clash_expr::{Registry, Type, Value, VerbSpec};
use clash_world::{Cause, Location, PathLimits};

use crate::Sim;
use crate::event::Subject;
use crate::reason::Reason;
use crate::rule::VerbCall;
use crate::verb::VerbDef;

use super::defs::{MovementDef, MovementId, PostureDef, PostureId};
use super::{codes, path, posture, step};

/// The reason an argument is not what a verb takes.
fn bad_argument(name: &str, wanted: &str) -> Reason {
    Reason::new(codes::BAD_ARGUMENT, format!("`{name}` must be {wanted}")).with("argument", name)
}

/// The argument at a position, or null.
fn arg<'a>(call: &'a VerbCall<'_>, at: usize) -> &'a Value {
    const NULL: &Value = &Value::Null;
    call.args.get(at).unwrap_or(NULL)
}

/// The entity an argument names.
fn entity_arg(sim: &Sim, call: &VerbCall<'_>, at: usize, name: &str) -> Result<EntityId, Reason> {
    match sim.subject_of(arg(call, at)) {
        Some(Subject::Entity(id)) => Ok(id),
        _ => Err(bad_argument(name, "an entity")),
    }
}

/// The cell an argument names.
fn loc_arg(sim: &Sim, call: &VerbCall<'_>, at: usize, name: &str) -> Result<Location, Reason> {
    match sim.subject_of(arg(call, at)) {
        Some(Subject::Loc(loc) | Subject::Element(loc, _)) => Ok(loc),
        _ => Err(bad_argument(name, "a cell")),
    }
}

/// The definition of type `T` a value names: an id of that type, or its
/// qualified id as a text.
fn def_arg<T: Def>(sim: &Sim, value: &Value) -> Option<T::Id> {
    let registry = sim.defs().try_registry::<T>()?;
    match value {
        Value::Id(id) if sim.exprs().registry().def_type_id(T::TYPE) == Some(id.ty) => {
            let id = <T::Id as clash_core::DenseId>::from_index(usize::try_from(id.index).ok()?)?;
            registry.try_get(id).map(|_| id)
        }
        Value::Str(name) => registry.lookup(name),
        _ => None,
    }
}

/// The `entity` kind's type, if the registry has it.
fn entity_type(registry: &Registry) -> Option<Type> {
    registry.kind_id("entity").map(Type::Subject)
}

fn run_step(sim: &mut Sim, call: &VerbCall<'_>, cause: Cause) -> Result<Value, Reason> {
    let who = entity_arg(sim, call, 0, "who")?;
    let dir = arg(call, 1).as_str().unwrap_or("");
    let kind = step::parse_dir(dir).ok_or_else(|| {
        Reason::new(
            codes::BAD_DIRECTION,
            format!("\"{dir}\" is not a direction to step in"),
        )
        .with("dir", dir)
    })?;
    let drop = arg(call, 2).as_bool().unwrap_or(false);
    let stepped = step::step(sim, who, kind, drop, cause)?;
    Ok(Value::map([
        ("distance", Value::Real(stepped.length_m)),
        ("cost", Value::Real(stepped.cost_m)),
        ("time", Value::Real(stepped.time.as_secs_real())),
    ]))
}

fn run_move_to(sim: &mut Sim, call: &VerbCall<'_>, cause: Cause) -> Result<Value, Reason> {
    let who = entity_arg(sim, call, 0, "who")?;
    let to = loc_arg(sim, call, 1, "to")?;
    step::move_to(sim, who, to, cause)?;
    Ok(Value::Null)
}

fn run_set_posture(sim: &mut Sim, call: &VerbCall<'_>, cause: Cause) -> Result<Value, Reason> {
    let who = entity_arg(sim, call, 0, "who")?;
    let wanted = arg(call, 1);
    let posture: Option<PostureId> = if wanted.is_null() {
        None
    } else {
        Some(def_arg::<PostureDef>(sim, wanted).ok_or_else(|| {
            Reason::new(
                codes::POSTURE_UNKNOWN,
                format!("there is no posture {wanted}"),
            )
        })?)
    };
    posture::set_posture(sim, who, posture, cause)?;
    Ok(Value::Null)
}

fn run_plan_path(sim: &mut Sim, call: &VerbCall<'_>, cause: Cause) -> Result<Value, Reason> {
    let who = entity_arg(sim, call, 0, "who")?;
    let to = loc_arg(sim, call, 1, "to")?;
    let mut limits = PathLimits::DEFAULT;
    if let Some(max_cost) = arg(call, 2).as_number() {
        limits.max_cost_m = max_cost;
    }
    let planned = path::plan_path(sim, who, to, limits, cause)?;
    Ok(Value::map([
        (
            "steps",
            Value::Int(i64::try_from(planned.steps).unwrap_or(i64::MAX)),
        ),
        ("cost", Value::Real(planned.cost_m)),
    ]))
}

fn run_follow_path(sim: &mut Sim, call: &VerbCall<'_>, cause: Cause) -> Result<Value, Reason> {
    let who = entity_arg(sim, call, 0, "who")?;
    let stepped = path::follow_path(sim, who, cause)?;
    Ok(Value::map([
        ("distance", Value::Real(stepped.length_m)),
        ("cost", Value::Real(stepped.cost_m)),
        ("time", Value::Real(stepped.time.as_secs_real())),
    ]))
}

fn run_clear_path(sim: &mut Sim, call: &VerbCall<'_>, _cause: Cause) -> Result<Value, Reason> {
    let who = entity_arg(sim, call, 0, "who")?;
    path::clear_path(sim, who);
    Ok(Value::Null)
}

fn run_set_movement(sim: &mut Sim, call: &VerbCall<'_>, _cause: Cause) -> Result<Value, Reason> {
    let who = entity_arg(sim, call, 0, "who")?;
    let listed = arg(call, 1)
        .as_list()
        .ok_or_else(|| bad_argument("modes", "a list of ways of moving"))?;
    let mut modes: Vec<MovementId> = Vec::with_capacity(listed.len());
    for value in listed {
        modes.push(def_arg::<MovementDef>(sim, value).ok_or_else(|| {
            Reason::new(
                codes::MOVEMENT_UNKNOWN,
                format!("there is no way of moving {value}"),
            )
        })?);
    }
    step::set_modes(sim, who, &modes)?;
    Ok(Value::Null)
}

/// Movement's verbs, for the engine's table.
pub fn verbs() -> Vec<VerbDef> {
    vec![
        VerbDef {
            name: "step",
            build: |registry| {
                Some(
                    VerbSpec::new("step")
                        .arg("who", entity_type(registry)?, "Who steps.")
                        .arg(
                            "dir",
                            Type::Str,
                            "Which way: `north`, `north_east`, `east`, `south_east`, `south`, `south_west`, `west`, `north_west`, `up` or `down`.",
                        )
                        .optional_arg(
                            "drop",
                            Type::Bool,
                            "Whether to step off where there is nothing to stand on, and fall. False if left out.",
                        )
                        .doc("Takes one step to the cell next to the mover, checked against the world as it is now (06 §11). Takes no time itself: the action waits `self.step_time(dir)`. A refusal raises `step_refused`."),
                )
            },
            run: run_step,
        },
        VerbDef {
            name: "move_to",
            build: |registry| {
                let loc = registry.kind_id("loc")?;
                Some(
                    VerbSpec::new("move_to")
                        .arg("who", entity_type(registry)?, "Who is placed.")
                        .arg("to", Type::Subject(loc), "The cell.")
                        .doc("Puts an entity in a cell where it can be: nothing in its way there, room for it, nobody who blocks it. If nothing holds it up there it falls (06 §11)."),
                )
            },
            run: run_move_to,
        },
        VerbDef {
            name: "set_posture",
            build: |registry| {
                let posture = registry.def_type_id(PostureDef::TYPE)?;
                Some(
                    VerbSpec::new("set_posture")
                        .arg("who", entity_type(registry)?, "Whose posture.")
                        .optional_arg(
                            "posture",
                            Type::Id(posture),
                            "The posture. Left out or null: none.",
                        )
                        .doc("Puts an entity in a posture (06 §11.1). The hook `posture_changing` may cancel; `posture_changed` follows."),
                )
            },
            run: run_set_posture,
        },
        VerbDef {
            name: "plan_path",
            build: |registry| {
                let loc = registry.kind_id("loc")?;
                Some(
                    VerbSpec::new("plan_path")
                        .arg("who", entity_type(registry)?, "Who is to go.")
                        .arg("to", Type::Subject(loc), "Where to.")
                        .optional_arg(
                            "max_cost",
                            Type::Number,
                            "The dearest path worth having, in metres of travel. No limit if left out.",
                        )
                        .doc("Plans a path from what the mover believes and keeps it as the path it is following (06 §11). A refusal raises `path_failed`."),
                )
            },
            run: run_plan_path,
        },
        VerbDef {
            name: "follow_path",
            build: |registry| {
                Some(
                    VerbSpec::new("follow_path")
                        .arg("who", entity_type(registry)?, "Who steps.")
                        .direct("who")
                        .doc("Takes the next step of the path the mover is following, as `step` does (06 §11). A refused step marks the path stale: `plan_path` again."),
                )
            },
            run: run_follow_path,
        },
        VerbDef {
            name: "clear_path",
            build: |registry| {
                Some(
                    VerbSpec::new("clear_path")
                        .arg("who", entity_type(registry)?, "Whose path.")
                        .direct("who")
                        .doc("Forgets the path the mover is following (06 §11)."),
                )
            },
            run: run_clear_path,
        },
        VerbDef {
            name: "set_movement",
            build: |registry| {
                let movement = registry.def_type_id(MovementDef::TYPE)?;
                Some(
                    VerbSpec::new("set_movement")
                        .arg("who", entity_type(registry)?, "Whose ways of moving.")
                        .arg(
                            "modes",
                            Type::list(Type::Id(movement)),
                            "The ways of moving it has, in the order they are tried. Empty: the definitions flagged `default`, for a creature.",
                        )
                        .doc("Says which ways of moving an entity has (06 §11)."),
                )
            },
            run: run_set_movement,
        },
    ]
}
