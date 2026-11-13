//! What a character could do, as the screens that offer it read it
//! (12 §10.7): the actions carrying a tag, the uses and modes of its
//! abilities, and the sites it knows by name. Each is a property of the
//! character read of itself in a mind context (`view.actions_with`,
//! `view.ability_uses`, `view.ability_modes`, `view.known_sites`), so
//! that a screen offers only what the character itself could know.
//!
//! **Whether an action could start** ([`could_start`]) is what a menu
//! can say before anything is chosen, changing nothing: the character
//! is in the world and its life lets it act; every parameter given is
//! one the action has; the parameters not given take their defaults,
//! worked out in the character's mind without the random streams (a
//! default that draws is left null); the action's `requires`, with its
//! `fail_text`, once every parameter without a default is given; every
//! attack's `with` finds its surface on the character (07 §3.1): a part
//! that answers or an item held that carries the tag; and the character
//! has enough working slots of each channel the action uses. What
//! depends on the moment it starts (footing, slots held by other
//! actions, the hooks) is the action runtime's to say then (03 §6.6).

use std::sync::Arc;

use clash_core::{EntityId, Real};
use clash_data::Def;
use clash_expr::{DefRef, Value};

use crate::Sim;
use crate::ability::{Abilities, AbilityDef};
use crate::action::def::{ActionDef, LocalSource, Slots};
use crate::action::reason as why;
use crate::action::{ActionDefId, run};
use crate::combat::attack::WithSource;
use crate::event::Subject;
use crate::host::Bound;
use crate::reader::capitalised;
use crate::reason::Reason;
use crate::registry::PropertyDef;

/// The parameter a menu fills with the creature it is about, where an
/// action has one by that name: the same convention as a mind's
/// `choose` step (D-693) and an interaction's `target` (16 §3.5.1).
pub const TARGET_PARAM: &str = "target";

/// The word of an ability's use that can be made now.
pub const STATE_READY: &str = "ready";
/// The word of an ability's use whose instance is suppressed (08 §6.2).
pub const STATE_SUPPRESSED: &str = "suppressed";
/// The word of an ability's use whose cooldown is running.
pub const STATE_COOLING: &str = "cooling";
/// The word of an ability's use whose instance is dormant.
pub const STATE_DORMANT: &str = "dormant";
/// The word of an ability's use that cannot be made now for another
/// reason, which `why` gives.
pub const STATE_UNAVAILABLE: &str = "unavailable";

/// The value of an action definition's id in expressions.
fn action_value(sim: &Sim, id: ActionDefId) -> Value {
    match sim.exprs().registry().def_type_id(ActionDef::TYPE) {
        Some(ty) => Value::Id(DefRef::new(ty, id.raw())),
        None => Value::Null,
    }
}

/// What a reader calls an action: its `name`, or else its qualified id.
pub fn action_label(sim: &Sim, actor: EntityId, id: ActionDefId) -> String {
    let defs = Arc::clone(sim.defs());
    let qualified = run::action_name(&defs, id);
    let Some(def) = run::action_def(&defs, id) else {
        return qualified;
    };
    match &def.name {
        Some(name) => {
            let bound = Bound {
                subjects: vec![sim.subject_value(Subject::Entity(actor))],
                locals: vec![Value::Null; def.locals().len()],
                fields: Vec::new(),
            };
            let mut host = sim.host_for_reader(bound, actor);
            let shown = name.render_on(&defs, &mut host);
            if shown.is_empty() { qualified } else { shown }
        }
        None => qualified,
    }
}

/// Whether `actor` could start an action now, as far as can be said
/// before it is started, with the parameters `given` by name: `Ok`, or
/// the reason it could not. Changes nothing and draws nothing. See the
/// [module](self) documentation.
pub fn could_start(
    sim: &Sim,
    actor: EntityId,
    id: ActionDefId,
    given: &[(&str, Value)],
) -> Result<(), Reason> {
    let defs = Arc::clone(sim.defs());
    let def = run::action_def(&defs, id).ok_or_else(|| {
        Reason::new(
            run::ACTION_UNKNOWN,
            format!("there is no action definition {id}"),
        )
    })?;
    let action = action_value(sim, id);
    if sim.truth().location(actor).is_none() {
        return Err(Reason::new(
            crate::reason::codes::ENTITY_STORED,
            format!("entity {actor} is not in the world, so it cannot act"),
        ));
    }
    sim.life_lets_act(actor)?;
    let refuse = |reason: &why::Reason, values: &[(&str, Value)]| {
        let mut all = vec![("action", action.clone())];
        all.extend(values.iter().cloned());
        sim.refusal(reason, actor, &all, None)
    };
    for (name, _) in given {
        let known = def.local_slot(name).is_some_and(|slot| {
            def.locals().get(usize::from(slot)).is_some_and(|local| {
                matches!(local.source, LocalSource::Param(_) | LocalSource::Role(_))
            })
        });
        if !known {
            return Err(refuse(
                &why::PARAMETER_INVALID,
                &[
                    ("param", Value::from(*name)),
                    ("expected", Value::from("a parameter of the action")),
                ],
            ));
        }
    }
    let value_of = |name: &str| {
        given
            .iter()
            .find(|(have, _)| *have == name)
            .map(|(_, value)| value.clone())
            .filter(|value| !value.is_null())
    };
    let actor_value = sim.subject_value(Subject::Entity(actor));
    // The locals, in slot order; whether one the action needs from its
    // caller is missing.
    let mut locals: Vec<Value> = Vec::with_capacity(def.locals().len());
    let mut missing = false;
    for local in def.locals() {
        let value = match local.source {
            LocalSource::Window => Value::Null,
            LocalSource::Role(0) => actor_value.clone(),
            LocalSource::Role(_) => value_of(&local.name).unwrap_or_else(|| {
                missing = true;
                Value::Null
            }),
            LocalSource::Param(position) => match value_of(&local.name) {
                Some(value) => value,
                None => match def
                    .params()
                    .get(usize::from(position))
                    .and_then(|param| param.default.as_ref())
                {
                    Some(default) => {
                        let bound = Bound {
                            subjects: vec![actor_value.clone()],
                            locals: locals.clone(),
                            fields: Vec::new(),
                        };
                        let mut host = sim.host_for_reader(bound, actor);
                        default.eval_on(&defs, &mut host).unwrap_or_default()
                    }
                    None => {
                        missing = true;
                        Value::Null
                    }
                },
            },
            LocalSource::Var(position) => def
                .vars()
                .get(usize::from(position))
                .and_then(|var| var.default.as_ref())
                .and_then(|default| default.constant().cloned())
                .unwrap_or(Value::Null),
        };
        locals.push(value);
    }
    if !missing {
        let bound = Bound {
            subjects: vec![actor_value.clone()],
            locals: locals.clone(),
            fields: Vec::new(),
        };
        let mut host = sim.host(bound);
        if !def.requires.holds_on(&defs, &mut host) {
            let sentence = def.fail_text.as_ref().map(|text| {
                let bound = Bound {
                    subjects: vec![actor_value.clone()],
                    locals: locals.clone(),
                    fields: Vec::new(),
                };
                let mut host = sim.host_for_reader(bound, actor);
                capitalised(&text.render_on(&defs, &mut host))
            });
            let mut reason = sim.refusal(&why::REQUIRES, actor, &[("action", action)], sentence);
            reason.code = why::REQUIRES.code.to_owned();
            return Err(reason);
        }
    }
    let name = run::action_name(&defs, id);
    for attack in def.attacks.iter() {
        if let Err(reason) =
            crate::combat::surface::find(sim, actor, &attack.with, &name, &attack.id.value)
        {
            // Said as a reader is told it: what the blow needs, in the
            // words the attack gives, or else the tag or the part in
            // plain words; never an id (decision D-864).
            let label = action_label(sim, actor, id);
            let said = attack.with.needs.as_deref().map(str::trim);
            let needs = match (attack.with.source(), said) {
                (Some(WithSource::Held(_)), Some(words)) if !words.is_empty() => {
                    format!("{label} needs {words} in hand.")
                }
                (Some(WithSource::Held(tag)), _) if !tag.is_empty() => format!(
                    "{label} needs {} in hand.",
                    crate::reader::with_indefinite(&plain_words(tag))
                ),
                (Some(WithSource::Held(_)), _) => format!("{label} needs something in hand."),
                (Some(WithSource::Part(_)), Some(words)) if !words.is_empty() => {
                    format!("{label} needs {words}.")
                }
                (Some(WithSource::Part(part)), _) => {
                    format!("{label} needs a working {}.", part_words(sim, part))
                }
                (Some(WithSource::Gripped), _) => format!("{label} needs something gripped."),
                _ => capitalised(&reason.message),
            };
            return Err(Reason::new(reason.code, needs));
        }
    }
    let uses = def.roles().first().map_or(def.uses(), |role| &role.uses);
    for need in uses.needs() {
        let enabled = sim.enabled_slots(actor, need.channel);
        let needed = match need.slots {
            Slots::Count(count) => count,
            Slots::All => enabled.max(1),
        };
        if enabled < needed {
            let channel = def_name::<crate::action::def::ChannelDef>(sim, need.channel.raw());
            return Err(refuse(
                &why::CHANNEL_LACKING,
                &[
                    ("channel_name", Value::from(channel.as_str())),
                    ("needed", Value::Int(i64::from(needed))),
                    ("enabled", Value::Int(i64::from(enabled))),
                ],
            ));
        }
    }
    Ok(())
}

/// A word written as an id (`flat_edge`, `core:blade`) as plain words:
/// its namespace dropped and its underscores spaces.
fn plain_words(id: &str) -> String {
    bare(id).replace('_', " ")
}

/// What a reader calls a part a blow is made with: the name of the body
/// part definition it names, or else the words it is written in.
fn part_words(sim: &Sim, part: &str) -> String {
    let named = sim
        .defs()
        .try_registry::<crate::body::BodyPartDef>()
        .and_then(|parts| {
            let id = parts.lookup(part).or_else(|| {
                parts
                    .names()
                    .iter()
                    .filter(|qualified| bare(qualified) == part)
                    .min()
                    .and_then(|qualified| parts.lookup(qualified))
            })?;
            parts
                .try_get(id)?
                .name
                .as_ref()
                .map(|name| name.raw().trim().to_owned())
        })
        .filter(|name| !name.is_empty());
    named.unwrap_or_else(|| plain_words(part))
}

/// The qualified id of a definition by its dense number.
fn def_name<T: Def>(sim: &Sim, raw: impl Into<u32>) -> String {
    let raw: u32 = raw.into();
    sim.defs()
        .try_registry::<T>()
        .and_then(|registry| {
            registry
                .names()
                .get(raw as usize)
                .map(|name| name.to_string())
        })
        .unwrap_or_default()
}

/// A row of a menu: `value` and `label` for a picker, the rest for a
/// screen that shows it.
fn row(fields: Vec<(&str, Value)>) -> Value {
    Value::map(fields)
}

/// `entity.actions_with(tag, target?)`: one row for each action that
/// carries the tag (what could start first, then what strikes with
/// something held, then fewer slots, then by qualified id: D-791), with
/// whether the entity could start it now (given `target` for an action
/// that has a parameter of that name) and why not.
pub fn actions_with(sim: &Sim, actor: EntityId, tag: &str, target: &Value) -> Vec<Value> {
    let defs = Arc::clone(sim.defs());
    let (Some(tag), Some(registry)) = (defs.tags().lookup(tag), defs.try_registry::<ActionDef>())
    else {
        return Vec::new();
    };
    let mut rows: Vec<((bool, bool, u32), Value)> = Vec::new();
    for (id, def) in registry.iter() {
        if !def.tags.contains(tag) {
            continue;
        }
        // What could start comes first; among those, what strikes with
        // something held, then what takes fewer slots: the quicker and
        // lighter use of the hands.
        let held = def
            .attacks
            .iter()
            .any(|attack| matches!(attack.with.source(), Some(WithSource::Held(_))));
        let mut given: Vec<(&str, Value)> = Vec::new();
        if !target.is_null()
            && (def.param(TARGET_PARAM).is_some()
                || def
                    .roles()
                    .iter()
                    .skip(1)
                    .any(|role| role.name.value == TARGET_PARAM))
        {
            given.push((TARGET_PARAM, target.clone()));
        }
        let verdict = could_start(sim, actor, id, &given);
        let label = action_label(sim, actor, id);
        let slots: u32 = def
            .roles()
            .first()
            .map_or(def.uses(), |role| &role.uses)
            .needs()
            .iter()
            .map(|need| match need.slots {
                Slots::Count(count) => u32::from(count),
                Slots::All => u32::from(u16::MAX),
            })
            .sum();
        rows.push((
            (verdict.is_err(), !held, slots),
            row(vec![
                ("value", action_value(sim, id)),
                ("label", Value::from(label.as_str())),
                ("id", Value::from(registry.name(id))),
                ("name", Value::from(label.as_str())),
                ("available", Value::Bool(verdict.is_ok())),
                (
                    "why",
                    Value::from(
                        verdict
                            .err()
                            .map(|reason| capitalised(&reason.message))
                            .unwrap_or_default()
                            .as_str(),
                    ),
                ),
            ]),
        ));
    }
    // A stable sort: the order of qualified id within each.
    rows.sort_by_key(|(order, _)| *order);
    rows.into_iter().map(|(_, row)| row).collect()
}

/// What a reader calls an ability: its `name`, or else its qualified id.
fn ability_label(sim: &Sim, owner: EntityId, def: &AbilityDef, number: u64, id: &str) -> String {
    match &def.name {
        Some(name) => {
            let bound = Bound::new()
                .subject(sim.subject_value(Subject::Entity(owner)))
                .subject(crate::ability::handle_value(sim, number));
            let mut host = sim.host_for_reader(bound, owner);
            let shown = name.render_on(sim.defs(), &mut host);
            if shown.is_empty() {
                id.to_owned()
            } else {
                shown
            }
        }
        None => id.to_owned(),
    }
}

/// The part of a qualified id after its namespace.
fn bare(id: &str) -> &str {
    id.rsplit(':').next().unwrap_or(id)
}

/// A number as a reader is shown it: whole if it is, else to a tenth.
fn shown_number(value: Real) -> String {
    let value = value.get();
    if (value - value.round()).abs() < 1e-9 {
        format!("{}", value.round() as i64)
    } else {
        format!("{value:.1}")
    }
}

/// `entity.ability_uses(ability?)`: one row for each use of each
/// ability the entity holds (`clash_sim::ability::options`), or of the
/// one named.
pub fn ability_uses(sim: &Sim, owner: EntityId, only: &Value) -> Vec<Value> {
    let defs = Arc::clone(sim.defs());
    let (Some(abilities), Some(registry)) = (
        sim.components().get::<Abilities>(owner),
        defs.try_registry::<AbilityDef>(),
    ) else {
        return Vec::new();
    };
    let ability_ty = sim.exprs().registry().def_type_id(AbilityDef::TYPE);
    let wanted = match only {
        Value::Null => None,
        Value::Id(id) if Some(id.ty) == ability_ty => Some(id.index),
        Value::Str(text) if text.is_empty() => return Vec::new(),
        Value::Str(text) => match crate::ability::lookup(&defs, text) {
            Some(id) => Some(id.raw()),
            None => return Vec::new(),
        },
        _ => return Vec::new(),
    };
    let mut rows = Vec::new();
    for option in crate::ability::options(sim, owner) {
        if wanted.is_some_and(|wanted| wanted != option.ability.raw()) {
            continue;
        }
        let Some(inst) = abilities.by_number(option.number) else {
            continue;
        };
        let def = registry.get(option.ability);
        let id = registry.name(option.ability).to_owned();
        let name = ability_label(sim, owner, def, option.number, &id);
        let action_name = action_label(sim, owner, option.action);
        let given: Vec<Value> = def
            .grants
            .iter()
            .find(|grant| grant.action.id() == option.action)
            .map(|grant| {
                grant
                    .params
                    .keys()
                    .map(|key| Value::from(key.as_str()))
                    .collect()
            })
            .unwrap_or_default();
        let cost = option
            .cost
            .iter()
            .map(|line| {
                let what = bare(&line.what);
                format!("{} {what}", shown_number(line.amount))
            })
            .collect::<Vec<_>>()
            .join(", ");
        let state = if inst.state() == crate::ability::State::Dormant {
            STATE_DORMANT
        } else if inst.is_suppressed() {
            STATE_SUPPRESSED
        } else if option.unavailable.is_none() {
            STATE_READY
        } else if option.cooldown_left > clash_core::Dur::ZERO {
            STATE_COOLING
        } else {
            STATE_UNAVAILABLE
        };
        let label = if name == action_name {
            name.clone()
        } else {
            format!("{name}: {action_name}")
        };
        rows.push(row(vec![
            ("value", action_value(sim, option.action)),
            (
                "ability",
                ability_ty.map_or(Value::Null, |ty| {
                    Value::Id(DefRef::new(ty, option.ability.raw()))
                }),
            ),
            ("label", Value::from(label.as_str())),
            ("id", Value::from(id.as_str())),
            ("name", Value::from(name.as_str())),
            ("action_name", Value::from(action_name.as_str())),
            (
                "rank",
                Value::Int(i64::from(crate::ability::rank(sim, owner, inst))),
            ),
            ("state", Value::from(state)),
            ("cost", Value::from(cost.as_str())),
            ("cooldown", Value::Real(option.cooldown_left.as_secs_real())),
            ("given", Value::list(given)),
            ("available", Value::Bool(option.unavailable.is_none())),
            (
                "why",
                Value::from(
                    option
                        .unavailable
                        .map(|reason| capitalised(&reason.message))
                        .unwrap_or_default()
                        .as_str(),
                ),
            ),
        ]));
    }
    rows
}

/// `entity.ability_modes`: one row for each mode of each ability the
/// entity holds, with whether it is on.
pub fn ability_modes(sim: &Sim, owner: EntityId) -> Vec<Value> {
    let defs = Arc::clone(sim.defs());
    let (Some(abilities), Some(registry)) = (
        sim.components().get::<Abilities>(owner),
        defs.try_registry::<AbilityDef>(),
    ) else {
        return Vec::new();
    };
    let ability_ty = sim.exprs().registry().def_type_id(AbilityDef::TYPE);
    let mut rows = Vec::new();
    for inst in abilities.instances().iter().filter(|inst| inst.is_held()) {
        let def = registry.get(inst.def());
        let id = registry.name(inst.def()).to_owned();
        let name = ability_label(sim, owner, def, inst.number(), &id);
        let rank = crate::ability::rank(sim, owner, inst);
        let dormant = inst.state() == crate::ability::State::Dormant;
        for (index, mode) in def.modes.iter().enumerate() {
            let on = inst.mode_is_on(index);
            let mode_name = mode
                .name
                .as_ref()
                .map(|text| {
                    let bound = Bound::new()
                        .subject(sim.subject_value(Subject::Entity(owner)))
                        .subject(crate::ability::handle_value(sim, inst.number()));
                    let mut host = sim.host_for_reader(bound, owner);
                    text.render_on(&defs, &mut host)
                })
                .filter(|shown| !shown.is_empty())
                .unwrap_or_else(|| mode.id.value.clone());
            let why = if dormant {
                format!("{} is dormant.", capitalised(&name))
            } else if inst.is_suppressed() {
                format!("{} is suppressed.", capitalised(&name))
            } else if mode.at_rank.is_some_and(|needed| rank < needed) {
                format!(
                    "{} needs rank {} for this, and is at {rank}.",
                    capitalised(&name),
                    mode.at_rank.unwrap_or(0)
                )
            } else {
                String::new()
            };
            rows.push(row(vec![
                (
                    "ability",
                    ability_ty.map_or(Value::Null, |ty| {
                        Value::Id(DefRef::new(ty, inst.def().raw()))
                    }),
                ),
                ("id", Value::from(id.as_str())),
                ("name", Value::from(name.as_str())),
                ("mode", Value::from(mode.id.value.as_str())),
                ("label", Value::from(mode_name.as_str())),
                ("rank", Value::Int(i64::from(rank))),
                ("on", Value::Bool(on)),
                ("available", Value::Bool(why.is_empty())),
                ("why", Value::from(why.as_str())),
            ]));
        }
    }
    rows
}

/// A direction and a distance from one cell to another, as a journal
/// writes them: `12 m north-east`, `here`.
fn way(from: clash_world::CellPos, to: clash_world::CellPos) -> String {
    let (dx, dy) = (
        i64::from(to.x) - i64::from(from.x),
        i64::from(to.y) - i64::from(from.y),
    );
    let metres = ((dx * dx + dy * dy) as f64).sqrt();
    if metres < 0.5 {
        return "here".to_owned();
    }
    // A compass point: north is toward lower y.
    let (ns, ew) = (
        if dy * 5 < -(dx.abs() * 2) {
            "north"
        } else if dy * 5 > dx.abs() * 2 {
            "south"
        } else {
            ""
        },
        if dx * 5 > dy.abs() * 2 {
            "east"
        } else if dx * 5 < -(dy.abs() * 2) {
            "west"
        } else {
            ""
        },
    );
    let point = match (ns, ew) {
        ("", ew) => ew.to_owned(),
        (ns, "") => ns.to_owned(),
        (ns, ew) => format!("{ns}-{ew}"),
    };
    format!("{} m {point}", metres.round() as i64)
}

/// `entity.known_sites`: the sites whose anchor cell the entity's map
/// memory holds (15 §13), each with its `name`, `at` and `where` from
/// the entity, nearest first.
pub fn known_sites(sim: &Sim, who: EntityId) -> Vec<Value> {
    let Some(here) = sim.truth().location(who) else {
        return Vec::new();
    };
    let cosmos = sim.truth().cosmos();
    let mut found: Vec<(i64, Value)> = Vec::new();
    for site in cosmos.sites() {
        let Some(grid) = cosmos.realm(site.realm).map(|realm| realm.base_grid) else {
            continue;
        };
        let at = clash_world::Location::new(grid, site.anchor);
        let remembered = cosmos.normalize(at).is_ok_and(|at| {
            sim.components
                .map_memory
                .get(who)
                .is_some_and(|memory| memory.cell(at).is_some())
        });
        if grid != here.grid || !remembered {
            continue;
        }
        let (dx, dy) = (
            i64::from(site.anchor.x) - i64::from(here.cell.x),
            i64::from(site.anchor.y) - i64::from(here.cell.y),
        );
        found.push((
            dx * dx + dy * dy,
            row(vec![
                ("name", Value::from(site.name.as_str())),
                ("at", sim.subject_value(Subject::Loc(at))),
                ("where", Value::from(way(here.cell, site.anchor).as_str())),
            ]),
        ));
    }
    found.sort_by_key(|(far, _)| *far);
    found.into_iter().map(|(_, value)| value).collect()
}

/// The properties of this module, for the engine's table.
pub fn properties() -> Vec<PropertyDef> {
    vec![
        PropertyDef::new(
            "entity",
            "actions_with",
            "list<map<any>>",
            "What the entity could start of the actions that carry a tag, as it believes (12 §10.7): one row for each, those that could start first, then those that strike with something held, then those that take fewer slots, then by qualified id, with `value` (the action's id), `label` and `name` (what a reader calls it), `id` (its qualified id as a text), `available` (whether it could start now) and `why` (the refusal's sentence where it could not, else empty). With `target`, that is given to an action that has a parameter or role named `target`; its `requires` is judged once every parameter without a default is given. Read of the character itself in a mind context.",
            |sim, subject, args| match subject {
                Subject::Entity(id) => {
                    let tag = args.first().and_then(Value::as_str).unwrap_or("");
                    let target = args.get(1).cloned().unwrap_or(Value::Null);
                    Value::list(actions_with(sim, id, tag, &target))
                }
                _ => Value::list([]),
            },
        )
        .arg(
            "tag",
            "string",
            "The tag the actions carry: the `ui` mod asks for `strike`, `shot`, `grip`, `takedown`.",
        )
        .optional_arg("target", "any", "Who the actions are aimed at, if anyone."),
        PropertyDef::new(
            "entity",
            "ability_uses",
            "list<map<any>>",
            "Every use of every ability the entity holds (08 §5.1, `ability::options`), or of the one named, for an abilities screen: one row for each grant, with `value` (the action's id), `ability` (the ability's id), `label`, `id` (the ability's qualified id), `name`, `action_name`, `rank`, `state` (`ready`, `cooling`, `dormant`, `suppressed` or `unavailable`), `cost` (a text: `25 stamina`), `cooldown` (seconds of its own time left), `given` (the names of the parameters the grant supplies), `available` and `why`. Read of the character itself in a mind context.",
            |sim, subject, args| match subject {
                Subject::Entity(id) => Value::list(ability_uses(
                    sim,
                    id,
                    args.first().unwrap_or(&Value::Null),
                )),
                _ => Value::list([]),
            },
        )
        .optional_arg("ability", "any", "An ability: its id, or its qualified id as text."),
        PropertyDef::new(
            "entity",
            "ability_modes",
            "list<map<any>>",
            "Every mode of every ability the entity holds (08 §5.4): one row for each, with `ability`, `id`, `name` (the ability's), `mode` (the mode's id), `label` (what the mode is called), `rank`, `on`, `available` and `why`. Read of the character itself in a mind context.",
            |sim, subject, _| match subject {
                Subject::Entity(id) => Value::list(ability_modes(sim, id)),
                _ => Value::list([]),
            },
        ),
        PropertyDef::new(
            "entity",
            "known_sites",
            "list<map<any>>",
            "The sites whose anchor the entity's map memory holds (15 §13), on its grid, nearest first: each with `name`, `at` (the anchor) and `where` (`12 m north-east`). Only directly controlled characters keep a map memory. Read of the character itself in a mind context.",
            |sim, subject, _| match subject {
                Subject::Entity(id) => Value::list(known_sites(sim, id)),
                _ => Value::list([]),
            },
        ),
    ]
}

/// A property with one more argument that may be left out.
trait OptionalArg {
    fn optional_arg(self, name: &'static str, ty: &'static str, doc: &'static str) -> Self;
}

impl OptionalArg for PropertyDef {
    fn optional_arg(
        mut self,
        name: &'static str,
        ty: &'static str,
        doc: &'static str,
    ) -> PropertyDef {
        self.args.push(crate::registry::ArgDef {
            name,
            ty,
            doc,
            optional: true,
        });
        self
    }
}
