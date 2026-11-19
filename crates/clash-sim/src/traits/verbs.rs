//! The verbs of traits (06 §5, §13): what an effect calls to give and
//! take away traits.
//!
//! | Verb | Arguments | Returns | Runs |
//! |------|-----------|---------|------|
//! | `add_trait` | `to: entity`, `trait: id<trait>`, `duration: real?` | `int`: the stacks the instance has | [`add_trait`] with [`Giver::RULE`] |
//! | `remove_trait` | `from: entity`, `trait: id<trait>`, `stacks: int?` | `int`: the stacks that went | [`remove_trait`] |
//!
//! An argument missing or of the wrong kind is refused with
//! `trait.bad_argument`.

use clash_core::{Dur, Real};
use clash_data::Def;
use clash_expr::{Registry, Value, VerbSpec};

use super::{Giver, TraitDef, TraitId, add_trait, codes, remove_trait};
use crate::Sim;
use crate::event::Subject;
use crate::reason::Reason;
use crate::rule::VerbCall;
use crate::verb::VerbDef;

fn bad_argument(name: &str, wanted: &str) -> Reason {
    Reason::new(
        codes::BAD_ARGUMENT,
        format!("the argument `{name}` must be {wanted}"),
    )
    .with("arg", name)
}

fn arg<'a>(call: &'a VerbCall<'_>, index: usize) -> &'a Value {
    call.args.get(index).unwrap_or(&Value::Null)
}

fn entity(
    sim: &Sim,
    call: &VerbCall<'_>,
    index: usize,
    name: &str,
) -> Result<clash_core::EntityId, Reason> {
    match sim.subject_of(arg(call, index)) {
        Some(Subject::Entity(id)) => Ok(id),
        _ => Err(bad_argument(name, "an entity")),
    }
}

/// An argument that names a trait: an id, or its qualified id as text.
fn trait_def(sim: &Sim, call: &VerbCall<'_>, index: usize) -> Result<TraitId, Reason> {
    let ty = sim.exprs.registry().def_type_id(TraitDef::TYPE);
    match arg(call, index) {
        Value::Id(id) if Some(id.ty) == ty => TraitId::from_index(id.index as usize)
            .ok_or_else(|| bad_argument("trait", "a `trait` definition")),
        Value::Str(name) => {
            super::lookup(&sim.defs, name).ok_or_else(|| super::ops::unknown(&**name))
        }
        _ => Err(bad_argument("trait", "a `trait` definition")),
    }
}

fn spec(
    registry: &Registry,
    name: &str,
    doc: &str,
    args: &[(&str, &str, bool, &str)],
) -> Option<VerbSpec> {
    let mut spec = VerbSpec::new(name).doc(doc);
    for (arg, written, optional, arg_doc) in args {
        let ty = registry.parse_type(written)?;
        spec = if *optional {
            spec.optional_arg(*arg, ty.nullable(), *arg_doc)
        } else {
            spec.arg(*arg, ty, *arg_doc)
        };
    }
    Some(spec)
}

/// The verbs of traits, for [`crate::verb::engine_verbs`].
pub fn verbs() -> Vec<VerbDef> {
    vec![
        VerbDef {
            name: "add_trait",
            build: |registry| {
                spec(
                    registry,
                    "add_trait",
                    "Gives an entity a trait (06 §5), as every rule gives it. New to it, the trait gains an instance with one stack, and traits of its `group` are removed first; had already, its `stacking` says what happens: `none` nothing, `refresh` its duration starts again, `stack` it gains a stack up to `max` and its duration starts again. The hook `trait_adding` runs first; `trait_added` follows. Returns how many stacks the instance has. Refused with `trait.unknown`, `trait.bad_duration` and by a handler of the hook.",
                    &[
                        ("to", "entity", false, "Who is given it."),
                        ("trait", "id<trait>", false, "The trait."),
                        (
                            "duration",
                            "real",
                            true,
                            "How long it lasts this time, in seconds of the entity's own time, above zero. Left out, its definition's `duration`.",
                        ),
                    ],
                )
            },
            run: |sim, call, cause| {
                let to = entity(sim, call, 0, "to")?;
                let def = trait_def(sim, call, 1)?;
                let duration = match arg(call, 2) {
                    Value::Null => None,
                    value => {
                        let seconds = value
                            .as_number()
                            .ok_or_else(|| bad_argument("duration", "a number of seconds"))?;
                        if !(seconds.is_finite() && seconds > Real::ZERO) {
                            return Err(Reason::new(
                                codes::BAD_DURATION,
                                format!("a trait cannot last {seconds} seconds: a duration is a finite time above zero"),
                            )
                            .with("duration", seconds));
                        }
                        Some(Dur::from_secs_real(seconds).max(Dur::from_micros(1)))
                    }
                };
                let added = add_trait(sim, to, def, Giver::RULE, duration, cause)?;
                Ok(Value::Int(i64::from(added.stacks)))
            },
        },
        VerbDef {
            name: "remove_trait",
            build: |registry| {
                spec(
                    registry,
                    "remove_trait",
                    "Removes a trait from an entity, whoever gave it (06 §5): every stack, or with `stacks` that many, taken from its givers in the order they gave it. The hook `trait_removing` runs first; `trait_removed` follows for each giver's instance it touched. Returns how many stacks went: 0 for a trait the entity does not have, which changes nothing. Refused with `trait.unknown`, `trait.bad_stacks` and by a handler of the hook.",
                    &[
                        ("from", "entity", false, "Whose trait."),
                        ("trait", "id<trait>", false, "The trait."),
                        (
                            "stacks",
                            "int",
                            true,
                            "How many stacks to take away, at least one. Left out, all of them.",
                        ),
                    ],
                )
            },
            run: |sim, call, cause| {
                let from = entity(sim, call, 0, "from")?;
                let def = trait_def(sim, call, 1)?;
                let stacks = match arg(call, 2) {
                    Value::Null => None,
                    value => Some(
                        value
                            .as_int()
                            .and_then(|count| u16::try_from(count).ok())
                            .filter(|count| *count > 0)
                            .ok_or_else(|| {
                                Reason::new(
                                    codes::BAD_STACKS,
                                    "a count of stacks to remove must be a whole number of at least one",
                                )
                                .with("stacks", value.as_int().unwrap_or(0))
                            })?,
                    ),
                };
                let taken = remove_trait(sim, from, def, stacks, cause)?;
                Ok(Value::Int(i64::from(taken)))
            },
        },
    ]
}
