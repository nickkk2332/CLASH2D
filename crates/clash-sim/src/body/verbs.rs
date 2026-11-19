//! The verbs of bodies, hits and wounds (06 §6.5, §7), as the engine's
//! verb table lists them: each reads its arguments and calls the
//! function of [`bind`](super::bind) that the engine itself uses.
//!
//! | Verb | Arguments | Gives |
//! |------|-----------|-------|
//! | `hit` | `target`, `kind`, `energy`, `intensity`, `part?`, `height?`, `spread?`, `area?`, `span?`, `dir?` | A map: `absorbed`, `returned`, `exited` (joules), `destroyed`, `cancelled` |
//! | `close_wound`, `reopen_wound` | `wound` | null |
//! | `heal_part` | `part`, `share` | null |
//! | `disable_part`, `restore_part` | `part` | null |
//! | `remove_part` | `part`, `sever?` | null |
//!
//! A part and a wound are subjects of their own kinds, which say whose
//! body they are of, so these verbs take no `who`.

use clash_core::{EntityId, Real};
use clash_data::Def;
use clash_expr::{Registry, Type, Value, VerbSpec};
use clash_world::defs::{DamageTypeDef, DamageTypeId};
use clash_world::{Cause, DamagePacket};

use super::props::{PART_KIND, WOUND_KIND, part_of_handle, wound_of_handle};
use super::{Detach, Hit, HitAt, PartKey, WoundId};
use crate::Sim;
use crate::life::args;
use crate::reason::Reason;
use crate::rule::VerbCall;
use crate::verb::VerbDef;

/// The reason for a part or a wound that is not what the verb needs.
fn not_there(name: &str, what: &str) -> Reason {
    Reason::new(
        args::BAD_ARGUMENT,
        format!("the argument `{name}` must be {what}"),
    )
    .with("argument", name)
}

/// The part an argument names, and whose body it is of.
fn part_arg(
    sim: &Sim,
    call: &VerbCall<'_>,
    at: usize,
    name: &str,
) -> Result<Option<(EntityId, PartKey)>, Reason> {
    let Some(handle) = args::optional_handle(call, at, name)? else {
        return Ok(None);
    };
    let kind = sim.body_defs().map(|defs| defs.kinds().part);
    if Some(handle.kind) != kind {
        return Err(not_there(name, "a part of a body"));
    }
    part_of_handle(&sim.components().body, handle)
        .map(Some)
        .ok_or_else(|| not_there(name, "a part of a body that still exists"))
}

/// The wound an argument names, and whose body it is on.
fn wound_arg(
    sim: &Sim,
    call: &VerbCall<'_>,
    at: usize,
    name: &str,
) -> Result<(EntityId, WoundId), Reason> {
    let handle =
        args::optional_handle(call, at, name)?.ok_or_else(|| not_there(name, "a wound"))?;
    let kind = sim.body_defs().map(|defs| defs.kinds().wound);
    if Some(handle.kind) != kind {
        return Err(not_there(name, "a wound"));
    }
    wound_of_handle(&sim.components().body, handle)
        .ok_or_else(|| not_there(name, "a wound on a body that still exists"))
}

/// A verb that takes one part.
fn of_part(registry: &Registry, name: &str, doc: &str) -> Option<VerbSpec> {
    let part = registry.kind_id(PART_KIND)?;
    Some(
        VerbSpec::new(name)
            .arg("part", Type::Subject(part), "The part.")
            .direct("part")
            .doc(doc),
    )
}

/// A verb that takes one wound.
fn of_wound(registry: &Registry, name: &str, doc: &str) -> Option<VerbSpec> {
    let wound = registry.kind_id(WOUND_KIND)?;
    Some(
        VerbSpec::new(name)
            .arg("wound", Type::Subject(wound), "The wound.")
            .direct("wound")
            .doc(doc),
    )
}

/// Runs the verb `hit`.
fn run_hit(sim: &mut Sim, call: &VerbCall<'_>, cause: Cause) -> Result<Value, Reason> {
    let target = args::entity(sim, call, 0, "target")?;
    let kind = args::def(sim, call, 1, "kind", DamageTypeDef::TYPE)?;
    let kind = DamageTypeId::from_index(kind)
        .ok_or_else(|| not_there("kind", "the id of a `damage_type`"))?;
    let energy = args::real(call, 2, "energy")?;
    let intensity = args::real(call, 3, "intensity")?;
    let part = part_arg(sim, call, 4, "part")?;
    let height = args::optional_real(call, 5, "height")?;
    let spread = args::flag(call, 6, "spread")?;
    let area = args::optional_real(call, 7, "area")?.unwrap_or(Real::ZERO);
    let span = args::optional_real(call, 8, "span")?.unwrap_or(Real::ZERO);
    let dir = args::optional_dir(call, 9, "dir")?;
    let at = match (spread, part, height) {
        (true, _, _) => HitAt::Spread,
        (false, Some((owner, key)), _) if owner == target => HitAt::Part(key),
        (false, Some(_), _) => return Err(not_there("part", "a part of the target's body")),
        (false, None, Some(height)) => HitAt::Height(height),
        (false, None, None) => HitAt::Any,
    };
    let outcome = sim.hit(Hit {
        target,
        at,
        packet: DamagePacket::new(kind, energy, intensity, cause),
        area,
        span,
        dir,
    })?;
    Ok(Value::map([
        ("absorbed", Value::Real(outcome.report.absorbed)),
        ("returned", Value::Real(outcome.report.returned)),
        ("exited", Value::Real(outcome.exited())),
        ("destroyed", Value::Bool(outcome.report.destroyed)),
        ("cancelled", Value::Bool(outcome.cancelled.is_some())),
    ]))
}

/// The verbs of 06 §6.5 and §7 that are built, for the engine's verb
/// table.
pub fn verbs() -> Vec<VerbDef> {
    vec![
        VerbDef {
            name: "hit",
            build: |registry| {
                let entity = registry.kind_id("entity")?;
                let part = registry.kind_id(PART_KIND)?;
                let kind = registry.def_type_id(DamageTypeDef::TYPE)?;
                Some(
                    VerbSpec::new("hit")
                        .arg("target", Type::Subject(entity), "Whose body is hit.")
                        .arg("kind", Type::Id(kind), "The kind of damage.")
                        .arg("energy", Type::Number, "Joules delivered.")
                        .arg("intensity", Type::Number, "How concentrated the delivery is, in pascals.")
                        .optional_arg("part", Type::Subject(part), "The part it is aimed at. Left out with `height` and `spread`, it lands on one outer part chosen by exposed area.")
                        .optional_arg("height", Type::Number, "The height above the target's footing it arrives at, in metres: it lands on an outer part whose band holds that height.")
                        .optional_arg("spread", Type::Bool, "Whether it is spread over every outer part.")
                        .optional_arg("area", Type::Number, "The area of contact, in square metres. Not used by a spread hit.")
                        .optional_arg("span", Type::Number, "The length of contact across the part, in metres. Not used by a spread hit.")
                        .optional_arg("dir", Type::list(Type::Number), "The direction it travels in: east, south and up.")
                        .doc("Applies a hit to a body: the hook `hit_incoming`, then the layers (06 §7). Gives a map of what came of it: `absorbed`, `returned` and `exited` in joules, `destroyed` and `cancelled`."),
                )
            },
            run: run_hit,
        },
        VerbDef {
            name: "close_wound",
            build: |registry| {
                of_wound(
                    registry,
                    "close_wound",
                    "Closes an open wound: it stops leaking and what it goes through begins to heal (06 §7.3).",
                )
            },
            run: |sim, call, cause| {
                let (entity, wound) = wound_arg(sim, call, 0, "wound")?;
                sim.close_wound(entity, wound, cause)?;
                Ok(Value::Null)
            },
        },
        VerbDef {
            name: "reopen_wound",
            build: |registry| {
                of_wound(
                    registry,
                    "reopen_wound",
                    "Opens a closed wound again: it leaks as it did and its time to close starts over (06 §7.3).",
                )
            },
            run: |sim, call, cause| {
                let (entity, wound) = wound_arg(sim, call, 0, "wound")?;
                sim.reopen_wound(entity, wound, cause)?;
                Ok(Value::Null)
            },
        },
        VerbDef {
            name: "heal_part",
            build: |registry| {
                Some(
                    of_part(
                        registry,
                        "heal_part",
                        "Takes a share of a whole layer off the damage of every layer of a part. A wound whose layers are whole again is healed (06 §6.5).",
                    )?
                    .arg("share", Type::Number, "The share of a layer, above zero; 1 makes every layer whole."),
                )
            },
            run: |sim, call, cause| {
                let (entity, part) = part_arg(sim, call, 0, "part")?
                    .ok_or_else(|| not_there("part", "a part of a body"))?;
                let share = args::real(call, 1, "share")?;
                sim.heal_part(entity, part, share, cause)?;
                Ok(Value::Null)
            },
        },
        VerbDef {
            name: "disable_part",
            build: |registry| {
                of_part(
                    registry,
                    "disable_part",
                    "Disables a part from outside, whatever its condition, until `restore_part` (06 §6.5).",
                )
            },
            run: |sim, call, cause| {
                let (entity, part) = part_arg(sim, call, 0, "part")?
                    .ok_or_else(|| not_there("part", "a part of a body"))?;
                sim.disable_part(entity, part, cause)?;
                Ok(Value::Null)
            },
        },
        VerbDef {
            name: "restore_part",
            build: |registry| {
                of_part(
                    registry,
                    "restore_part",
                    "Takes back what `disable_part` did: the part works again if its condition allows (06 §6.5).",
                )
            },
            run: |sim, call, cause| {
                let (entity, part) = part_arg(sim, call, 0, "part")?
                    .ok_or_else(|| not_there("part", "a part of a body"))?;
                sim.restore_part(entity, part, cause)?;
                Ok(Value::Null)
            },
        },
        VerbDef {
            name: "remove_part",
            build: |registry| {
                Some(
                    of_part(
                        registry,
                        "remove_part",
                        "Takes a part off a body with everything on it and in it, and reports it as `part_detached`. The root cannot be taken off (06 §6.5).",
                    )?
                    .optional_arg("sever", Type::Bool, "True to leave the parts listed as gone, as a hit that severs does; left out, the body simply no longer has them."),
                )
            },
            run: |sim, call, cause| {
                let (entity, part) = part_arg(sim, call, 0, "part")?
                    .ok_or_else(|| not_there("part", "a part of a body"))?;
                let how = if args::flag(call, 1, "sever")? {
                    Detach::Severed
                } else {
                    Detach::Removed
                };
                sim.remove_part(entity, part, how, cause)?;
                Ok(Value::Null)
            },
        },
    ]
}
