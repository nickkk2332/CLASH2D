//! The verbs of abilities (08 §12): what an effect calls to grant,
//! revoke, rank, use and switch abilities.
//!
//! | Verb | Arguments | Returns | Runs |
//! |------|-----------|---------|------|
//! | `grant_ability` | `to: entity`, `ability: id<ability>`, `rank: int?` | `ability`: the instance | The hook `ability_granting`, then [`grant`](super::grant) from a rule |
//! | `revoke_ability` | `from: entity`, `ability: id<ability>` | `bool`: whether it was held | The hook `ability_revoking`, then [`revoke`](super::revoke) |
//! | `set_rank` | `who: entity`, `ability: id<ability>`, `rank: int` | `int`: the rank before | [`set_rank`](super::set_rank) |
//! | `use_ability` | `who: entity`, `ability: id<ability>`, `action: id<action>?`, `params: map?` | `bool`: whether the action started | [`use_ability`](super::use_ability); a refusal raises `ability_refused` |
//! | `start_mode` | `who: entity`, `ability: id<ability>`, `mode: string` | `bool`: true | [`start_mode`](super::start_mode) |
//! | `end_mode` | `who: entity`, `ability: id<ability>`, `mode: string` | `bool`: true | [`end_mode`](super::end_mode) |
//! | `suppress_abilities` | `who: entity`, `tags: list<string>?`, `for: real?` | `int`: how many abilities it suppresses now | [`suppress_abilities`](super::suppress_abilities) |
//! | `restore_abilities` | `who: entity`, `tags: list<string>?` | `int`: how many suppressions were lifted | [`restore_abilities`](super::restore_abilities) |
//! | `summon`, `conjure`, `bind_trait`, `bind_rule`, `dismiss`, `dispel` | See [`super::bind`] | | Bindings (08 §7.3) |
//!
//! An argument missing or of the wrong kind is refused with
//! `ability.bad_argument`.

use clash_core::EntityId;
use clash_data::Def;
use clash_expr::{Registry, Value, VerbSpec};

use super::{
    ABILITY_GRANTING, ABILITY_REVOKING, AbilityDef, AbilityId, AbilitySource, codes, name_of,
};
use crate::Sim;
use crate::action::{ActionDef, ActionDefId, Params};
use crate::bus::EventPayload;
use crate::event::{Subject, Where};
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

fn entity(sim: &Sim, call: &VerbCall<'_>, index: usize, name: &str) -> Result<EntityId, Reason> {
    match sim.subject_of(arg(call, index)) {
        Some(Subject::Entity(id)) if sim.entities.contains(id) => Ok(id),
        _ => Err(bad_argument(name, "an entity that exists")),
    }
}

/// An argument that names an ability: an id, or its qualified id as
/// text.
fn ability(sim: &Sim, call: &VerbCall<'_>, index: usize) -> Result<AbilityId, Reason> {
    let ty = sim.exprs.registry().def_type_id(AbilityDef::TYPE);
    match arg(call, index) {
        Value::Id(id) if Some(id.ty) == ty => AbilityId::from_index(id.index as usize)
            .ok_or_else(|| bad_argument("ability", "an `ability` definition")),
        Value::Str(name) => {
            super::lookup(&sim.defs, name).ok_or_else(|| super::ops::unknown(&**name))
        }
        _ => Err(bad_argument("ability", "an `ability` definition")),
    }
}

/// An argument that names an action, or `None` for one left out.
fn action(sim: &Sim, call: &VerbCall<'_>, index: usize) -> Result<Option<ActionDefId>, Reason> {
    let ty = sim.exprs.registry().def_type_id(ActionDef::TYPE);
    match arg(call, index) {
        Value::Null => Ok(None),
        Value::Id(id) if Some(id.ty) == ty => Ok(ActionDefId::from_index(id.index as usize)),
        Value::Str(name) => sim
            .defs
            .lookup::<ActionDef>(name)
            .map(Some)
            .ok_or_else(|| bad_argument("action", "an `action` definition")),
        _ => Err(bad_argument("action", "an `action` definition")),
    }
}

/// An optional list of texts: tags.
fn tags(call: &VerbCall<'_>, index: usize, name: &str) -> Result<Option<Vec<String>>, Reason> {
    match arg(call, index) {
        Value::Null => Ok(None),
        Value::List(items) => items
            .iter()
            .map(|item| {
                item.as_str()
                    .map(str::to_owned)
                    .ok_or_else(|| bad_argument(name, "a list of texts"))
            })
            .collect::<Result<Vec<_>, _>>()
            .map(Some),
        _ => Err(bad_argument(name, "a list of texts")),
    }
}

/// An optional number.
fn real(call: &VerbCall<'_>, index: usize, name: &str) -> Result<Option<clash_core::Real>, Reason> {
    match arg(call, index) {
        Value::Null => Ok(None),
        value => value
            .as_number()
            .map(Some)
            .ok_or_else(|| bad_argument(name, "a number")),
    }
}

fn text<'a>(call: &'a VerbCall<'_>, index: usize, name: &str) -> Result<&'a str, Reason> {
    match arg(call, index) {
        Value::Str(text) => Ok(text),
        _ => Err(bad_argument(name, "a text")),
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

const WHO: (&str, &str, bool, &str) = ("who", "entity", false, "Whose ability.");
const ABILITY: (&str, &str, bool, &str) = ("ability", "id<ability>", false, "The ability.");
const MODE: (&str, &str, bool, &str) =
    ("mode", "string", false, "The mode's id within the ability.");

/// The verbs of abilities, for [`crate::verb::engine_verbs`]: those
/// here and those of bindings ([`super::bind::verbs`]).
pub fn verbs() -> Vec<VerbDef> {
    let mut all = own_verbs();
    all.extend(super::bind::verbs());
    all
}

fn own_verbs() -> Vec<VerbDef> {
    vec![
        VerbDef {
            name: "grant_ability",
            build: |registry| {
                spec(
                    registry,
                    "grant_ability",
                    "Gives an entity an ability from a rule (08 §3.3): a pact, a lesson, a theft. New to it, it gains an instance at `rank` (or its definition's), watched by its `while`, conferring what it lists if that holds; held already, the rule becomes another source of the one instance and `rank` sets its rank; revoked, it is held again. The hook `ability_granting` runs first; `ability_granted` follows for an ability newly held. Returns the instance. Refused with `ability.unknown`, `ability.bad_rank` and by a handler of the hook.",
                    &[
                        ("to", "entity", false, "Who is given it."),
                        ABILITY,
                        (
                            "rank",
                            "int",
                            true,
                            "The rank, for an ability with no track. Left out, its definition's.",
                        ),
                    ],
                )
            },
            run: |sim, call, cause| {
                let to = entity(sim, call, 0, "to")?;
                let def = ability(sim, call, 1)?;
                let rank = match arg(call, 2) {
                    Value::Null => None,
                    value => Some(
                        value
                            .as_int()
                            .and_then(|rank| u32::try_from(rank).ok())
                            .ok_or_else(|| {
                                Reason::new(
                                    codes::BAD_RANK,
                                    "a rank is a whole number of zero or more",
                                )
                                .with("rank", value.as_int().unwrap_or(-1))
                            })?,
                    ),
                };
                if sim.bus.has_listeners(ABILITY_GRANTING) {
                    let name = name_of(&sim.defs, def);
                    let payload = EventPayload::new(ABILITY_GRANTING)
                        .subject("self", Subject::Entity(to))
                        .with("ability", name.as_str())
                        .with("source", AbilitySource::Rule.name());
                    if let Some(reason) = sim.raise(Where::Entity(to), cause, payload)?.cancelled {
                        return Err(reason);
                    }
                }
                let number = super::grant(sim, to, def, AbilitySource::Rule, rank, cause)?;
                Ok(super::handle_value(sim, number))
            },
        },
        VerbDef {
            name: "revoke_ability",
            build: |registry| {
                spec(
                    registry,
                    "revoke_ability",
                    "Takes an ability away from an entity whatever its sources (08 §6.3): its modes end, what it conferred is taken back and `ability_revoked` follows. A trait that lists it does not bring it back while that trait lasts; a rule's `grant_ability` does. The hook `ability_revoking` runs first. Returns whether it was held. Refused with `ability.unknown` and by a handler of the hook.",
                    &[("from", "entity", false, "Whose ability."), ABILITY],
                )
            },
            run: |sim, call, cause| {
                let from = entity(sim, call, 0, "from")?;
                let def = ability(sim, call, 1)?;
                let held = sim
                    .components
                    .get::<super::Abilities>(from)
                    .is_some_and(|abilities| abilities.holds(def));
                if held && sim.bus.has_listeners(ABILITY_REVOKING) {
                    let name = name_of(&sim.defs, def);
                    let payload = EventPayload::new(ABILITY_REVOKING)
                        .subject("self", Subject::Entity(from))
                        .with("ability", name.as_str());
                    if let Some(reason) = sim.raise(Where::Entity(from), cause, payload)?.cancelled
                    {
                        return Err(reason);
                    }
                }
                Ok(Value::Bool(super::revoke(sim, from, def, cause)?))
            },
        },
        VerbDef {
            name: "set_rank",
            build: |registry| {
                spec(
                    registry,
                    "set_rank",
                    "Sets the rank of an ability with no track (08 §4.1): what applies from a rank is brought into line, modes above it end, and `rank_changed` follows if it changed. Returns the rank before. Refused with `ability.not_held`, `ability.rank_from_track` and `ability.bad_rank`.",
                    &[
                        WHO,
                        ABILITY,
                        ("rank", "int", false, "The rank, from 0 to 10,000."),
                    ],
                )
            },
            run: |sim, call, cause| {
                let who = entity(sim, call, 0, "who")?;
                let def = ability(sim, call, 1)?;
                let rank = arg(call, 2)
                    .as_int()
                    .ok_or_else(|| bad_argument("rank", "a whole number"))?;
                let old = super::set_rank(sim, who, def, rank, cause)?;
                Ok(Value::Int(i64::from(old)))
            },
        },
        VerbDef {
            name: "use_ability",
            build: |registry| {
                spec(
                    registry,
                    "use_ability",
                    "Uses an ability (08 §5): starts the action one of its grants makes available, through the action runtime, with its cost taken in full and its cooldown started, after the hook `ability_using`. Returns whether the action started; when it did not, `ability_refused` says why (`ability.not_held`, `ability.dormant`, `ability.rank_too_low`, `ability.cooling_down`, `ability.condition`, `ability.cost_short`, `ability.cost_source_gone`, `ability.items_short`, a refusal of the action runtime, or a handler's).",
                    &[
                        WHO,
                        ABILITY,
                        (
                            "action",
                            "id<action>",
                            true,
                            "The action of the grant. Left out, the ability's only grant.",
                        ),
                        (
                            "params",
                            "any",
                            true,
                            "The action's parameters, a map by name. The grant's own `params` stand in for any of the same name.",
                        ),
                    ],
                )
            },
            run: |sim, call, cause| {
                let who = entity(sim, call, 0, "who")?;
                let def = ability(sim, call, 1)?;
                let action = action(sim, call, 2)?;
                let mut params = Params::new();
                match arg(call, 3) {
                    Value::Null => {}
                    Value::Map(map) => {
                        for (name, value) in map.iter() {
                            params.push((name.to_string(), value.to_stored()));
                        }
                    }
                    _ => return Err(bad_argument("params", "a map of parameters by name")),
                }
                match super::use_ability(sim, who, def, action, &params, cause) {
                    Ok(started) => Ok(Value::Bool(started.is_some())),
                    Err(reason) => {
                        let ability_name = name_of(&sim.defs, def);
                        let action_name = action
                            .map(|action| crate::action::run::action_name(&sim.defs, action))
                            .unwrap_or_default();
                        super::using::refused(
                            sim,
                            who,
                            &ability_name,
                            &action_name,
                            &reason,
                            cause,
                        );
                        Ok(Value::Bool(false))
                    }
                }
            },
        },
        VerbDef {
            name: "start_mode",
            build: |registry| {
                spec(
                    registry,
                    "start_mode",
                    "Switches a mode of an ability on (08 §5.4): what switching costs is taken, its trait given and its upkeep applied together, its `on_start` run; `mode_started` follows. It ends when switched off, when its upkeep runs a pool dry, or when the ability goes dormant or is lost. Returns true. Refused with `ability.not_held`, `ability.dormant`, `ability.rank_too_low`, `ability.mode_unknown`, `ability.mode_on`, `ability.upkeep`, `ability.cost_short`.",
                    &[WHO, ABILITY, MODE],
                )
            },
            run: |sim, call, cause| {
                let who = entity(sim, call, 0, "who")?;
                let def = ability(sim, call, 1)?;
                let mode = text(call, 2, "mode")?;
                super::start_mode(sim, who, def, mode, cause)?;
                Ok(Value::Bool(true))
            },
        },
        VerbDef {
            name: "end_mode",
            build: |registry| {
                spec(
                    registry,
                    "end_mode",
                    "Switches a mode of an ability off (08 §5.4): its trait and its upkeep go together, its `on_end` runs and `mode_ended` follows with `how` `ended`. Returns true. Refused with `ability.not_held`, `ability.mode_unknown`, `ability.mode_off`.",
                    &[WHO, ABILITY, MODE],
                )
            },
            run: |sim, call, cause| {
                let who = entity(sim, call, 0, "who")?;
                let def = ability(sim, call, 1)?;
                let mode = text(call, 2, "mode")?;
                super::end_mode(sim, who, def, mode, cause)?;
                Ok(Value::Bool(true))
            },
        },
        VerbDef {
            name: "suppress_abilities",
            build: |registry| {
                spec(
                    registry,
                    "suppress_abilities",
                    "Suppresses an entity's abilities that have any of `tags`, or every one (08 §6.2): each confers nothing, its modes end (`how` `suppressed`), what was bound to end on suppression ends, and its grants cannot be used, until the suppression lifts. Abilities gained while it lasts are suppressed too. Before it takes hold of each ability the hook `ability_suppressing` runs, and a handler may resist. It lasts `for` seconds of the entity's own time, or until `restore_abilities`. Returns how many of its abilities it suppresses now. Refused with `ability.bad_duration`.",
                    &[
                        WHO,
                        (
                            "tags",
                            "list<string>",
                            true,
                            "The tags of the abilities it suppresses: any of them. Left out, every ability.",
                        ),
                        (
                            "for",
                            "real",
                            true,
                            "How long it lasts, in seconds of the entity's own time. Left out, until it is lifted.",
                        ),
                    ],
                )
            },
            run: |sim, call, cause| {
                let who = entity(sim, call, 0, "who")?;
                let tags = tags(call, 1, "tags")?;
                let seconds = real(call, 2, "for")?;
                let count = super::suppress_abilities(sim, who, tags, seconds, cause)?;
                Ok(Value::Int(i64::from(count)))
            },
        },
        VerbDef {
            name: "restore_abilities",
            build: |registry| {
                spec(
                    registry,
                    "restore_abilities",
                    "Lifts early the suppressions `suppress_abilities` imposed on an entity that name any of `tags`, or every one (08 §12). What a trait of its or an area it is inside suppresses lasts as long as they do. Abilities no longer suppressed confer again (`ability_restored`); their modes stay off. Returns how many suppressions were lifted.",
                    &[
                        WHO,
                        (
                            "tags",
                            "list<string>",
                            true,
                            "Lifts only the suppressions that name any of these tags, or that name none. Left out, every one.",
                        ),
                    ],
                )
            },
            run: |sim, call, cause| {
                let who = entity(sim, call, 0, "who")?;
                let tags = tags(call, 1, "tags")?;
                let count = super::restore_abilities(sim, who, tags.as_deref(), cause)?;
                Ok(Value::Int(i64::from(count)))
            },
        },
    ]
}
