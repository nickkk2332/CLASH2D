//! Combat (`docs/07-combat.md`): contests, attacks, the strike, aim,
//! delivery and push.
//!
//! Combat adds no timing system and no damage system (decision D-101).
//! An attack is a block on an action (03 §6); the `strike` effect, run
//! from one of the action's phases, resolves it against a target; what
//! reaches the target is a [`Hit`](crate::body::Hit) on its body
//! (06 §7) or a packet on an element of the world (02 §12); and the
//! share set aside to push it moves it, through the world's bodies in
//! flight (02 §15) when it is enough.
//!
//! | Part | Is | Module |
//! |------|----|--------|
//! | [`ContestDef`], [`contest_fn`], [`odds_fn`] | The `contest` definition type and the functions `contest` and `odds` (07 §4.1) | [`contest`] |
//!
//! # What is not built here yet
//!
//! Defences (07 §5), clashes (§8), grips and throws (§9), takedowns
//! (§10), projectiles and ranged weapons (§11) and previews (§13) are
//! the later packages of `docs/plans/combat.md`.

pub mod contest;

use clash_core::{Real, Rng};
use clash_data::{Def, DefTypes};
use clash_expr::{Registry, Value};

pub use contest::{ContestDef, ContestForm, ContestId};

use crate::Sim;
use crate::registry::FunctionDef;

/// The heading combat's functions are listed under in the reference.
const GROUP_COMBAT: &str = "Combat";

/// Registers combat's definition types with the loader: `contest`.
/// Called from the crate's `register_defs`.
pub fn register_defs(types: &mut DefTypes) {
    types.register::<ContestDef>();
}

/// Enters in a registry the definition types combat's functions name,
/// unless they are there already: `contest`. Called while the engine's
/// registry is built, before its functions.
pub fn register_types(registry: &mut Registry) {
    if registry.def_type_id(ContestDef::TYPE).is_none() {
        let _ = registry.add_def_type(ContestDef::TYPE, ContestDef::DOC);
    }
}

/// The contest a value names, if it names a loaded one.
fn contest_of<'a>(sim: &'a Sim, value: Option<&Value>) -> Option<&'a ContestDef> {
    let id = value?.as_id()?;
    let index = usize::try_from(id.index).ok()?;
    sim.defs()
        .try_registry::<ContestDef>()?
        .try_get(ContestId::from_index(index)?)
}

/// The three arguments of `contest` and `odds`: the contest, `a` and
/// `b`.
fn contest_args<'a>(sim: &'a Sim, args: &[Value]) -> Option<(&'a ContestDef, Real, Real)> {
    let def = contest_of(sim, args.first())?;
    let a = args.get(1)?.as_number()?;
    let b = args.get(2)?.as_number()?;
    Some((def, a, b))
}

/// The body of `contest(contest, a, b)` where a random stream is at
/// hand: a margin, drawn from `rng` (07 §4.1). Null for a contest that
/// is not loaded or a value that is null.
pub fn contest_fn(sim: &Sim, args: &[Value], rng: &mut Rng) -> Value {
    contest_args(sim, args).map_or(Value::Null, |(def, a, b)| {
        Value::Real(def.margin(a, b, rng))
    })
}

/// The body of `odds(contest, a, b)`: the chance that a margin of the
/// contest is at or above zero (07 §4.1). It draws nothing. Null for a
/// contest that is not loaded or a value that is null.
pub fn odds_fn(sim: &Sim, args: &[Value]) -> Value {
    contest_args(sim, args).map_or(Value::Null, |(def, a, b)| Value::Real(def.odds(a, b)))
}

/// The functions combat offers to expressions (07 §16).
pub fn functions() -> Vec<FunctionDef> {
    vec![
        FunctionDef::new(
            "contest",
            "real?",
            GROUP_COMBAT,
            "A margin of a contest (07 §4.1): the scaled difference, or log-ratio, of `a` and `b`, plus logistic noise of the contest's spread drawn from the rule stream. At or above zero, `a` won. Refused in a pure context. Null for a value that is null.",
            // Where nothing may be drawn there is no margin.
            |_, _| Value::Null,
        )
        .arg("contest", "id<contest>", "The contest.")
        .arg("a", "real", "The value of the side the margin favours when it is positive.")
        .arg("b", "real", "The value of the other side.")
        .safe()
        .drawing(contest_fn),
        FunctionDef::new(
            "odds",
            "real?",
            GROUP_COMBAT,
            "The chance that a margin of a contest is at or above zero (07 §4.1), from 0 to 1. Pure: it draws nothing. Null for a value that is null.",
            odds_fn,
        )
        .arg("contest", "id<contest>", "The contest.")
        .arg("a", "real", "The value of the side the margin favours.")
        .arg("b", "real", "The value of the other side.")
        .safe(),
    ]
}
