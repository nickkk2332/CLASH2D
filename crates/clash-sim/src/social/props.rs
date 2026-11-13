//! What the social layer offers to expressions and rules (15 §15): the
//! properties a mind reads of itself, the function `intervenes`, and
//! the verbs.
//!
//! Every property is a property of an `entity` read of the character
//! itself (`self.rel('fear', x)`), as perception's belief properties are
//! (decision D-616): a mind context may read any property of its own
//! subject, and none is marked safe for belief, because read of another
//! entity it would tell one mind what another holds. Each answers by
//! the persona the mind takes `x` for.

use clash_core::{EntityId, Real};
use clash_expr::{Registry, Type, Value, VerbSpec};
use clash_world::Cause;

use super::defs::{LinesDef, MessageKindDef};
use super::order::{self, OrderPlan};
use super::reason as why;
use super::relation::{self, Toward, kind_named, named};
use super::speech::SayArgs;
use super::{group, persona};
use crate::Sim;
use crate::action::ActionDefId;
use crate::event::Subject;
use crate::perception::{Knowing, PersonaId};
use crate::reason::Reason;
use crate::registry::{FunctionDef, PropertyDef};
use crate::rule::VerbCall;
use crate::verb::VerbDef;

/// The heading the social layer's functions are listed under.
const GROUP: &str = "Social";

fn entity_of(sim: &Sim, value: Option<&Value>) -> Option<EntityId> {
    match sim.subject_of(value?)? {
        Subject::Entity(id) => sim.entities.contains(id).then_some(id),
        _ => None,
    }
}

fn mind_of(sim: &Sim, subject: Subject) -> Option<EntityId> {
    match subject {
        Subject::Entity(id) if sim.entities.contains(id) => Some(id),
        _ => None,
    }
}

fn text(value: impl Into<String>) -> Value {
    Value::from(value.into().as_str())
}

/// Whom a value names for a mind's relations: an entity, as the persona
/// the mind takes it for; a text, as a group.
fn toward_for(sim: &Sim, mind: EntityId, value: Option<&Value>) -> Option<Toward> {
    match value? {
        Value::Str(name) => group::group_key(sim, name).map(Toward::Group),
        other => {
            entity_of(sim, Some(other)).map(|x| Toward::Persona(persona::persona_of(sim, mind, x)))
        }
    }
}

/// The latest memory of an event type a mind holds.
fn latest_memory<'a>(
    sim: &'a Sim,
    mind: EntityId,
    event: &str,
) -> Option<&'a crate::perception::Memory> {
    super::memories_of(sim, mind)
        .iter()
        .rev()
        .find(|memory| memory.event == event)
}

/// The properties of the social layer (15 §15).
pub fn properties() -> Vec<PropertyDef> {
    let p = PropertyDef::new;
    vec![
        p(
            "entity",
            "persona",
            "string",
            "Its true persona, as text (`7v1/0`) (15 §8.1).",
            |sim, subject, _| {
                mind_of(sim, subject)
                    .map_or(Value::Null, |id| text(PersonaId::true_of(id).to_string()))
            },
        ),
        p(
            "entity",
            "presenting",
            "string",
            "The persona it presents now, as text: its true one, one it made, or someone else's it copied (15 §8.1).",
            |sim, subject, _| {
                mind_of(sim, subject)
                    .map_or(Value::Null, |id| text(persona::presenting(sim, id).to_string()))
            },
        ),
        p(
            "entity",
            "persona_of",
            "string?",
            "The persona this mind takes `x` for, as text (15 §8.2): what `x` presents, unless this mind has seen through it. Null for something that is not an entity.",
            |sim, subject, args| {
                match (mind_of(sim, subject), entity_of(sim, args.first())) {
                    (Some(mind), Some(x)) => text(persona::persona_of(sim, mind, x).to_string()),
                    _ => Value::Null,
                }
            },
        )
        .arg("x", "entity", "Whom it is asked about."),
        p(
            "entity",
            "knows_name",
            "bool",
            "Whether this mind knows the name of whom it takes `x` for (05 §4.6): learnt with `learn_name`, or its own.",
            |sim, subject, args| {
                Value::Bool(match (mind_of(sim, subject), entity_of(sim, args.first())) {
                    (Some(mind), Some(x)) => {
                        persona::knows_name(sim, mind, persona::persona_of(sim, mind, x))
                    }
                    _ => false,
                })
            },
        )
        .arg("x", "entity", "Whom it is asked about."),
        p(
            "entity",
            "rel",
            "real?",
            "What this mind holds of a kind of relation toward `x` (15 §9.1): an entity, as the persona it takes it for, or a group by key or id. Its own relation, else its groups' (15 §9.2), else the kind's default. Null for a kind or an `x` that names nothing.",
            |sim, subject, args| {
                let Some(mind) = mind_of(sim, subject) else {
                    return Value::Null;
                };
                let Some(kind) = args.first().and_then(Value::as_str).and_then(|name| kind_named(sim, name)) else {
                    return Value::Null;
                };
                toward_for(sim, mind, args.get(1))
                    .and_then(|toward| relation::rel_toward(sim, mind, kind, &toward))
                    .map_or(Value::Null, Value::Real)
            },
        )
        .arg("kind", "string", "The relation kind's id, qualified or bare.")
        .arg("x", "any", "An entity, or a group's key or id.")
        .changed_by(&["relation_changed", "group_joined", "group_left"]),
        p(
            "entity",
            "stance_to",
            "string",
            "How this mind stands toward `x` (09 §3.4): `opposes` if it holds a kind flagged `opposes` above its default by more than any flagged `sides_with`, `sides_with` the other way round, else `neutral`. Read through its groups as `rel` is.",
            |sim, subject, args| {
                let stance = match mind_of(sim, subject) {
                    Some(mind) => toward_for(sim, mind, args.first())
                        .map_or(relation::Stance::Neutral, |toward| {
                            relation::stance_toward(sim, mind, &toward)
                        }),
                    None => relation::Stance::Neutral,
                };
                text(stance.name())
            },
        )
        .arg("x", "any", "An entity, or a group's key or id.")
        .changed_by(&["relation_changed", "group_joined", "group_left"]),
        p(
            "entity",
            "in_group",
            "bool",
            "Whether it belongs to a group (15 §9.2), named by key or id.",
            |sim, subject, args| {
                Value::Bool(
                    mind_of(sim, subject)
                        .zip(args.first().and_then(Value::as_str).and_then(|name| group::group_key(sim, name)))
                        .is_some_and(|(id, key)| group::groups_of(sim, id).contains(&key)),
                )
            },
        )
        .arg("group", "string", "The group's key or id.")
        .changed_by(&["group_joined", "group_left"]),
        p(
            "entity",
            "role_in",
            "string?",
            "Its role in a group (15 §9.2), or null if it has none or is not a member.",
            |sim, subject, args| {
                mind_of(sim, subject)
                    .zip(args.first().and_then(Value::as_str).and_then(|name| group::group_key(sim, name)))
                    .and_then(|(id, key)| group::role_in(sim, id, &key))
                    .map_or(Value::Null, text)
            },
        )
        .arg("group", "string", "The group's key or id.")
        .changed_by(&["group_joined", "group_left", "role_changed"]),
        p(
            "entity",
            "standing_orders",
            "int",
            "How many orders it keeps that still stand (15 §10).",
            |sim, subject, _| {
                Value::Int(mind_of(sim, subject).map_or(0, |id| {
                    i64::try_from(order::standing_orders(sim, id).len()).unwrap_or(i64::MAX)
                }))
            },
        )
        .changed_by(&["order_given", "order_cancelled"]),
        p(
            "entity",
            "remembers_actor",
            "bool",
            "Whether this mind remembers whom it takes `x` for as the actor of an event of a type (15 §7.1): `''` for any type. What a witness blames, whoever truly did it (scene S15).",
            |sim, subject, args| {
                let (Some(mind), Some(x)) = (mind_of(sim, subject), entity_of(sim, args.first()))
                else {
                    return Value::Bool(false);
                };
                let event = args.get(1).and_then(Value::as_str).unwrap_or("");
                let persona = persona::persona_of(sim, mind, x);
                Value::Bool(super::memories_of(sim, mind).iter().any(|memory| {
                    (event.is_empty() || memory.event == event)
                        && memory
                            .roles
                            .iter()
                            .any(|(role, played)| role == "actor" && *played == Some(persona))
                }))
            },
        )
        .arg("x", "entity", "Whom it is asked about.")
        .arg("event", "string", "The event type, or `''` for any.")
        .changed_by(&["memory_added", "memory_forgotten"]),
        p(
            "entity",
            "memory_confidence",
            "real?",
            "How sure this mind is of its latest memory of an event type, 0 to 1 (15 §7.1): 1 for what it perceived, less for what it was told (15 §11). Null if it has none.",
            |sim, subject, args| {
                mind_of(sim, subject)
                    .zip(args.first().and_then(Value::as_str))
                    .and_then(|(mind, event)| latest_memory(sim, mind, event))
                    .map_or(Value::Null, |memory| Value::Real(memory.confidence))
            },
        )
        .arg("event", "string", "The event type.")
        .changed_by(&["memory_added", "memory_forgotten"]),
        p(
            "entity",
            "memory_source",
            "string?",
            "Who told this mind its latest memory of an event type, as the persona it took them for, as text (15 §11). Null if it has none, or perceived it, or did not make out who told it.",
            |sim, subject, args| {
                mind_of(sim, subject)
                    .zip(args.first().and_then(Value::as_str))
                    .and_then(|(mind, event)| latest_memory(sim, mind, event))
                    .and_then(|memory| memory.source)
                    .map_or(Value::Null, |source| text(source.to_string()))
            },
        )
        .arg("event", "string", "The event type.")
        .changed_by(&["memory_added", "memory_forgotten"]),
        p(
            "entity",
            "told_of",
            "bool",
            "Whether this mind's latest memory of an event type was told rather than perceived (15 §11).",
            |sim, subject, args| {
                Value::Bool(
                    mind_of(sim, subject)
                        .zip(args.first().and_then(Value::as_str))
                        .and_then(|(mind, event)| latest_memory(sim, mind, event))
                        .is_some_and(|memory| memory.how == Knowing::Told),
                )
            },
        )
        .arg("event", "string", "The event type.")
        .changed_by(&["memory_added", "memory_forgotten"]),
    ]
}

/// The function `intervenes` (15 §9.1).
pub(crate) fn functions() -> Vec<FunctionDef> {
    vec![
        FunctionDef::new(
            "intervenes",
            "bool?",
            GROUP,
            "Whether `a` holds toward `b`, as the persona it takes `b` for, a relation of a kind flagged `intervenes` above its default (15 §9.1): whether `a` is a candidate for windows opened against `b` (03 §7.1). Null if either is not an entity. Reads what `a` holds: for window kinds and world rules, not safe for belief.",
            |sim, args| match (entity_of(sim, args.first()), entity_of(sim, args.get(1))) {
                (Some(a), Some(b)) => Value::Bool(relation::intervenes(sim, a, b)),
                _ => Value::Null,
            },
        )
        .arg("a", "entity", "Who would step in.")
        .arg("b", "entity", "For whom."),
    ]
}

// ---- Verbs ---------------------------------------------------------------------

fn bad(name: &str, wanted: &str) -> Reason {
    Reason::new(why::BAD_ARGUMENT, format!("`{name}` must be {wanted}")).with("argument", name)
}

fn arg<'a>(call: &VerbCall<'a>, at: usize) -> &'a Value {
    const NULL: &Value = &Value::Null;
    call.args.get(at).unwrap_or(NULL)
}

fn arg_entity(sim: &Sim, call: &VerbCall<'_>, at: usize, name: &str) -> Result<EntityId, Reason> {
    entity_of(sim, Some(arg(call, at))).ok_or_else(|| bad(name, "an entity that exists"))
}

fn opt_entity(
    sim: &Sim,
    call: &VerbCall<'_>,
    at: usize,
    name: &str,
) -> Result<Option<EntityId>, Reason> {
    match arg(call, at) {
        Value::Null => Ok(None),
        _ => arg_entity(sim, call, at, name).map(Some),
    }
}

fn opt_text<'a>(call: &VerbCall<'a>, at: usize, name: &str) -> Result<Option<&'a str>, Reason> {
    match arg(call, at) {
        Value::Null => Ok(None),
        Value::Str(text) => Ok(Some(text)),
        _ => Err(bad(name, "a text")),
    }
}

fn req_text<'a>(call: &VerbCall<'a>, at: usize, name: &str) -> Result<&'a str, Reason> {
    opt_text(call, at, name)?.ok_or_else(|| bad(name, "a text"))
}

fn opt_real(call: &VerbCall<'_>, at: usize, name: &str) -> Result<Option<Real>, Reason> {
    match arg(call, at) {
        Value::Null => Ok(None),
        other => other
            .as_number()
            .map(Some)
            .ok_or_else(|| bad(name, "a number")),
    }
}

fn req_real(call: &VerbCall<'_>, at: usize, name: &str) -> Result<Real, Reason> {
    opt_real(call, at, name)?.ok_or_else(|| bad(name, "a number"))
}

fn opt_flag(call: &VerbCall<'_>, at: usize, name: &str) -> Result<bool, Reason> {
    match arg(call, at) {
        Value::Null => Ok(false),
        Value::Bool(flag) => Ok(*flag),
        _ => Err(bad(name, "true or false")),
    }
}

fn opt_texts(call: &VerbCall<'_>, at: usize, name: &str) -> Result<Option<Vec<String>>, Reason> {
    match arg(call, at) {
        Value::Null => Ok(None),
        other => other
            .as_list()
            .ok_or_else(|| bad(name, "a list of texts"))?
            .iter()
            .map(|item| {
                item.as_str()
                    .map(str::to_owned)
                    .ok_or_else(|| bad(name, "a list of texts"))
            })
            .collect::<Result<Vec<_>, _>>()
            .map(Some),
    }
}

fn persona_arg(call: &VerbCall<'_>, at: usize, name: &str) -> Result<Option<PersonaId>, Reason> {
    match opt_text(call, at, name)? {
        None => Ok(None),
        Some(written) => PersonaId::parse(written).map(Some).ok_or_else(|| {
            Reason::new(
                why::UNKNOWN_PERSONA,
                format!("`{written}` names no persona"),
            )
            .with("persona", written)
        }),
    }
}

fn kind_arg(sim: &Sim, call: &VerbCall<'_>, at: usize) -> Result<super::RelationKindId, Reason> {
    let name = req_text(call, at, "kind")?;
    kind_named(sim, name).ok_or_else(|| relation::unknown_kind(name))
}

fn persona_value(persona: PersonaId) -> Value {
    text(persona.to_string())
}

fn spec_of(call: &VerbCall<'_>, first: usize) -> Result<persona::PersonaSpec, Reason> {
    Ok(persona::PersonaSpec {
        name: opt_text(call, first, "name")?.map(str::to_owned),
        pronouns: None,
        tags: opt_texts(call, first + 1, "tags")?,
        claims: opt_texts(call, first + 2, "claims")?,
        quality: opt_real(call, first + 3, "quality")?,
        apparent: opt_text(call, first + 4, "apparent")?.map(str::to_owned),
    })
}

/// The persona arguments `create_persona` and `set_persona` share.
fn persona_args(spec: VerbSpec) -> VerbSpec {
    spec.optional_arg("name", Type::Str, "Its name.")
        .optional_arg(
            "tags",
            Type::list(Type::Str),
            "Its tags: a uniform, an insignia, a role.",
        )
        .optional_arg(
            "claims",
            Type::list(Type::Str),
            "The groups it presents as belonging to, by key or id.",
        )
        .optional_arg(
            "quality",
            Type::Real,
            "How well it holds as a disguise, 0 to 1.",
        )
        .optional_arg("apparent", Type::Str, "The template it looks like, by id.")
}

/// Whom a value names for a group's relations: a text, a group; an
/// entity, the persona it presents.
fn toward_for_group(sim: &Sim, value: &Value) -> Result<Toward, Reason> {
    match value {
        Value::Str(name) => group::require(sim, name).map(Toward::Group),
        other => entity_of(sim, Some(other))
            .map(|x| Toward::Persona(persona::presenting(sim, x)))
            .ok_or_else(|| bad("toward", "an entity or a group's key or id")),
    }
}

/// The message kind a text names.
fn message_kind(sim: &Sim, name: &str) -> Result<super::MessageKindId, Reason> {
    named::<MessageKindDef>(sim, name).ok_or_else(|| {
        Reason::new(
            why::UNKNOWN_MESSAGE_KIND,
            format!("there is no message kind `{name}`"),
        )
        .with("kind", name)
    })
}

/// The first message kind that carries orders, in order of id.
fn order_kind(sim: &Sim) -> Option<super::MessageKindId> {
    let registry = sim.defs.try_registry::<MessageKindDef>()?;
    let mut found: Vec<(&str, super::MessageKindId)> = registry
        .iter()
        .filter(|(_, def)| def.carries == super::Carries::Order)
        .map(|(id, _)| (registry.name(id), id))
        .collect();
    found.sort_unstable_by(|a, b| a.0.cmp(b.0));
    found.first().map(|(_, id)| *id)
}

fn signal_arg(
    sim: &Sim,
    call: &VerbCall<'_>,
    at: usize,
) -> Result<Option<crate::perception::SignalKindId>, Reason> {
    match opt_text(call, at, "signal")? {
        None => Ok(None),
        Some(name) => sim
            .perception_defs
            .kind_named(name)
            .map(Some)
            .ok_or_else(|| {
                Reason::new(
                    crate::perception::reason::UNKNOWN_KIND,
                    format!("there is no signal kind `{name}`"),
                )
                .with("kind", name)
            }),
    }
}

fn verb_say(sim: &mut Sim, call: &VerbCall<'_>, cause: Cause) -> Result<Value, Reason> {
    let who = arg_entity(sim, call, 0, "who")?;
    let kind = message_kind(sim, req_text(call, 1, "kind")?)?;
    let mut args = SayArgs::new(kind);
    args.to = opt_entity(sim, call, 2, "to")?;
    args.to_group = opt_text(call, 3, "group")?.map(str::to_owned);
    args.about = opt_entity(sim, call, 4, "about")?;
    args.event = opt_text(call, 5, "event")?.map(str::to_owned);
    args.lines = match opt_text(call, 6, "line")? {
        None => None,
        Some(name) => Some(named::<LinesDef>(sim, name).ok_or_else(|| {
            Reason::new(
                why::UNKNOWN_LINES,
                format!("there is no table of lines `{name}`"),
            )
            .with("lines", name)
        })?),
    };
    args.language = opt_text(call, 7, "language")?.map(str::to_owned);
    args.signal = signal_arg(sim, call, 8)?;
    args.strength = opt_real(call, 9, "strength")?;
    let sent = super::say(sim, who, args, cause)?;
    Ok(Value::Int(i64::from(sent)))
}

fn verb_order(sim: &mut Sim, call: &VerbCall<'_>, cause: Cause) -> Result<Value, Reason> {
    let from = arg_entity(sim, call, 0, "from")?;
    let to = arg_entity(sim, call, 1, "to")?;
    let action = match arg(call, 2) {
        Value::Id(id) if sim.defs().expr_registry().def_type_id("action") == Some(id.ty) => {
            Some(ActionDefId::new(id.index))
        }
        _ => None,
    }
    .ok_or_else(|| bad("action", "the id of an action"))?;
    let params = arg(call, 3).as_map().map_or_else(Vec::new, |map| {
        map.iter()
            .map(|(key, value)| (key.to_string(), value.to_stored()))
            .collect()
    });
    let until = match opt_real(call, 4, "until")? {
        None => None,
        Some(seconds) if seconds.is_finite() && seconds > Real::ZERO => {
            Some(clash_core::Dur::from_secs_real(seconds))
        }
        Some(_) => return Err(bad("until", "a time above zero, in seconds")),
    };
    let kind = match opt_text(call, 5, "kind")? {
        Some(name) => message_kind(sim, name)?,
        None => order_kind(sim).ok_or_else(|| {
            Reason::new(why::NOT_AN_ORDER, "no message kind carries orders").with("kind", "")
        })?,
    };
    let mut args = SayArgs::new(kind);
    args.to = Some(to);
    args.signal = signal_arg(sim, call, 6)?;
    args.strength = opt_real(call, 7, "strength")?;
    let plan = OrderPlan {
        action,
        params,
        until,
    };
    let sent = order::give_order(sim, from, args, plan, cause)?;
    Ok(Value::Int(i64::from(sent)))
}

/// The verbs of the social layer (15 §15).
pub(crate) fn verbs() -> Vec<VerbDef> {
    vec![
        VerbDef {
            name: "create_persona",
            build: |registry: &Registry| {
                let entity = registry.kind_id("entity")?;
                Some(
                    persona_args(VerbSpec::new("create_persona").arg("who", Type::Subject(entity), "Whose it is."))
                        .doc("Makes a persona for an entity (15 §8.3): a cover identity, a costume. It is presented only once assumed. Returns the persona, as text."),
                )
            },
            run: |sim, call, _| {
                let who = arg_entity(sim, call, 0, "who")?;
                let made = persona::create_persona(sim, who, spec_of(call, 1)?)?;
                Ok(persona_value(made))
            },
        },
        VerbDef {
            name: "assume_persona",
            build: |registry: &Registry| {
                let entity = registry.kind_id("entity")?;
                Some(
                    VerbSpec::new("assume_persona")
                        .arg("who", Type::Subject(entity), "Who presents it.")
                        .arg("persona", Type::Str, "One of its own personas, as text: one it made, or its true one.")
                        .doc("An entity presents another of its own personas (15 §8.3). Whoever perceives it at that moment sees the change. Refused with `social.not_own_persona` and `social.unknown_persona`."),
                )
            },
            run: |sim, call, _| {
                let who = arg_entity(sim, call, 0, "who")?;
                let chosen =
                    persona_arg(call, 1, "persona")?.ok_or_else(|| bad("persona", "a persona"))?;
                persona::assume_persona(sim, who, chosen)?;
                Ok(Value::Null)
            },
        },
        VerbDef {
            name: "drop_persona",
            build: |registry: &Registry| {
                let entity = registry.kind_id("entity")?;
                Some(
                    VerbSpec::new("drop_persona")
                        .arg("who", Type::Subject(entity), "Who drops it.")
                        .doc("An entity presents its true persona again (15 §8.3)."),
                )
            },
            run: |sim, call, _| {
                let who = arg_entity(sim, call, 0, "who")?;
                persona::drop_persona(sim, who)?;
                Ok(Value::Null)
            },
        },
        VerbDef {
            name: "copy_persona",
            build: |registry: &Registry| {
                let entity = registry.kind_id("entity")?;
                Some(
                    VerbSpec::new("copy_persona")
                        .arg("who", Type::Subject(entity), "Who takes on the likeness.")
                        .arg("of", Type::Subject(entity), "Whose likeness: what it presents now.")
                        .optional_arg("quality", Type::Real, "How well the likeness holds, 0 to 1; 1 if left out.")
                        .doc("An entity presents what another presents (15 §8.3): a shapeshifter, a perfect mask. What it does is remembered by witnesses as done by that persona; the world's causes still name it. Returns the persona presented, as text."),
                )
            },
            run: |sim, call, _| {
                let who = arg_entity(sim, call, 0, "who")?;
                let of = arg_entity(sim, call, 1, "of")?;
                let quality = opt_real(call, 2, "quality")?;
                Ok(persona_value(persona::copy_persona(sim, who, of, quality)?))
            },
        },
        VerbDef {
            name: "reveal_persona",
            build: |registry: &Registry| {
                let entity = registry.kind_id("entity")?;
                Some(
                    VerbSpec::new("reveal_persona")
                        .arg("who", Type::Subject(entity), "The mind that learns it.")
                        .arg("of", Type::Subject(entity), "Whose persona it sees through.")
                        .doc("Links, in one mind's beliefs, what an entity presents to its true persona (15 §8.3): it takes it for its true persona from now on, and for good where the persona is one the entity made. Raises `disguise_seen_through`. Returns whether anything was hidden from it."),
                )
            },
            run: |sim, call, _| {
                let who = arg_entity(sim, call, 0, "who")?;
                let of = arg_entity(sim, call, 1, "of")?;
                Ok(Value::Bool(persona::reveal_persona(sim, who, of)?))
            },
        },
        VerbDef {
            name: "set_persona",
            build: |registry: &Registry| {
                let entity = registry.kind_id("entity")?;
                Some(
                    persona_args(
                        VerbSpec::new("set_persona")
                            .arg("who", Type::Subject(entity), "Whose persona.")
                            .optional_arg("persona", Type::Str, "Which of its own, as text; left out, the one it presents if it is its own, else its true one."),
                    )
                    .doc("Sets what a persona of an entity is (15 §8.1): its name, tags, claims, quality and the template it looks like; each left out stays as it is. Returns the persona, as text."),
                )
            },
            run: |sim, call, _| {
                let who = arg_entity(sim, call, 0, "who")?;
                let chosen = persona_arg(call, 1, "persona")?;
                let set = persona::set_persona(sim, who, chosen, spec_of(call, 2)?)?;
                Ok(persona_value(set))
            },
        },
        VerbDef {
            name: "create_illusion",
            build: |registry: &Registry| {
                let entity = registry.kind_id("entity")?;
                let loc = registry.kind_id("loc")?;
                Some(
                    VerbSpec::new("create_illusion")
                        .arg("at", Type::Subject(loc), "The cell it stands in.")
                        .arg("signals", Type::map(Type::Real), "What it gives off: for each signal kind, by id, a standing strength.")
                        .optional_arg("of", Type::Subject(entity), "Whom it is taken for: it presents what they present.")
                        .doc("Makes an entity that is only signals (15 §8.3): no body, blocking nothing, perceived by the senses of the kinds it gives off. Tagged `illusion`. Returns it."),
                )
            },
            run: |sim, call, cause| {
                let at = match sim.subject_of(arg(call, 0)) {
                    Some(Subject::Loc(loc)) => loc,
                    _ => return Err(bad("at", "a cell")),
                };
                let mut signals = Vec::new();
                if let Some(map) = arg(call, 1).as_map() {
                    for (name, strength) in map.iter() {
                        let kind = sim.perception_defs.kind_named(name).ok_or_else(|| {
                            Reason::new(
                                crate::perception::reason::UNKNOWN_KIND,
                                format!("there is no signal kind `{name}`"),
                            )
                            .with("kind", name.as_ref())
                        })?;
                        signals.push((
                            kind,
                            strength
                                .as_number()
                                .ok_or_else(|| bad("signals", "a map of numbers"))?,
                        ));
                    }
                }
                let of = opt_entity(sim, call, 2, "of")?;
                let made: EntityId = persona::create_illusion(sim, at, of, &signals, cause)?;
                Ok(sim.subject_value(Subject::Entity(made)))
            },
        },
        VerbDef {
            name: "learn_name",
            build: |registry: &Registry| {
                let entity = registry.kind_id("entity")?;
                Some(
                    VerbSpec::new("learn_name")
                        .arg("who", Type::Subject(entity), "The mind that learns it.")
                        .arg("of", Type::Subject(entity), "Whose name: of whom it takes them for.")
                        .doc("A mind learns the name of whom it takes an entity for (15 §8.1): text it reads names them from now on instead of describing them (05 §4.6)."),
                )
            },
            run: |sim, call, _| {
                let who = arg_entity(sim, call, 0, "who")?;
                let of = arg_entity(sim, call, 1, "of")?;
                persona::learn_name(sim, who, of)?;
                Ok(Value::Null)
            },
        },
        VerbDef {
            name: "set_relation",
            build: |registry: &Registry| {
                let entity = registry.kind_id("entity")?;
                Some(
                    VerbSpec::new("set_relation")
                        .arg("who", Type::Subject(entity), "The mind that holds it.")
                        .arg("toward", Type::Any, "An entity, as the persona `who` takes it for, or a group's key or id.")
                        .arg("kind", Type::Str, "The relation kind's id, qualified or bare.")
                        .arg("value", Type::Real, "The value, kept within the kind's range; at its default the relation is no longer held.")
                        .doc("Sets what a mind holds of a kind toward someone (15 §9.1). Raises `relation_changed`. Losing a kind flagged `orders` withdraws the orders kept from them. Returns the value now."),
                )
            },
            run: |sim, call, cause| {
                let who = arg_entity(sim, call, 0, "who")?;
                let toward = toward_for(sim, who, Some(arg(call, 1)))
                    .ok_or_else(|| bad("toward", "an entity or a group's key or id"))?;
                let kind = kind_arg(sim, call, 2)?;
                let value = req_real(call, 3, "value")?;
                Ok(Value::Real(relation::set_relation(
                    sim, who, toward, kind, value, cause,
                )?))
            },
        },
        VerbDef {
            name: "add_relation",
            build: |registry: &Registry| {
                let entity = registry.kind_id("entity")?;
                Some(
                    VerbSpec::new("add_relation")
                        .arg("who", Type::Subject(entity), "The mind that holds it.")
                        .arg("toward", Type::Any, "An entity, as the persona `who` takes it for, or a group's key or id.")
                        .arg("kind", Type::Str, "The relation kind's id, qualified or bare.")
                        .arg("amount", Type::Real, "How much to add: to its own value, or to the kind's default.")
                        .doc("Adds to what a mind holds of a kind toward someone (15 §9.1). Raises `relation_changed`. Returns the value now."),
                )
            },
            run: |sim, call, cause| {
                let who = arg_entity(sim, call, 0, "who")?;
                let toward = toward_for(sim, who, Some(arg(call, 1)))
                    .ok_or_else(|| bad("toward", "an entity or a group's key or id"))?;
                let kind = kind_arg(sim, call, 2)?;
                let amount = req_real(call, 3, "amount")?;
                Ok(Value::Real(relation::add_relation(
                    sim, who, toward, kind, amount, cause,
                )?))
            },
        },
        VerbDef {
            name: "clear_relation",
            build: |registry: &Registry| {
                let entity = registry.kind_id("entity")?;
                Some(
                    VerbSpec::new("clear_relation")
                        .arg("who", Type::Subject(entity), "The mind that holds it.")
                        .arg("toward", Type::Any, "An entity, as the persona `who` takes it for, or a group's key or id.")
                        .optional_arg("kind", Type::Str, "The relation kind; left out, every kind.")
                        .doc("Takes away what a mind holds toward someone of its own (15 §9.1); what its groups hold stays. Raises `relation_changed` for each. Returns how many went."),
                )
            },
            run: |sim, call, cause| {
                let who = arg_entity(sim, call, 0, "who")?;
                let toward = toward_for(sim, who, Some(arg(call, 1)))
                    .ok_or_else(|| bad("toward", "an entity or a group's key or id"))?;
                let kind = match opt_text(call, 2, "kind")? {
                    None => None,
                    Some(name) => {
                        Some(kind_named(sim, name).ok_or_else(|| relation::unknown_kind(name))?)
                    }
                };
                let gone = relation::clear_relation(sim, who, toward, kind, cause)?;
                Ok(Value::Int(i64::try_from(gone).unwrap_or(i64::MAX)))
            },
        },
        VerbDef {
            name: "join_group",
            build: |registry: &Registry| {
                let entity = registry.kind_id("entity")?;
                Some(
                    VerbSpec::new("join_group")
                        .arg("who", Type::Subject(entity), "Who joins.")
                        .arg("group", Type::Str, "The group's key or id.")
                        .optional_arg("role", Type::Str, "Its role: one of the group definition's.")
                        .optional_arg("secret", Type::Bool, "True to belong in secret: its true persona does not claim the group.")
                        .doc("An entity joins a group (15 §9.2) and is told what the group knows. Raises `group_joined`."),
                )
            },
            run: |sim, call, cause| {
                let who = arg_entity(sim, call, 0, "who")?;
                let name = req_text(call, 1, "group")?;
                let role = opt_text(call, 2, "role")?;
                let secret = opt_flag(call, 3, "secret")?;
                group::join_group(sim, who, name, role, secret, cause)?;
                Ok(Value::Null)
            },
        },
        VerbDef {
            name: "leave_group",
            build: |registry: &Registry| {
                let entity = registry.kind_id("entity")?;
                Some(
                    VerbSpec::new("leave_group")
                        .arg("who", Type::Subject(entity), "Who leaves.")
                        .arg("group", Type::Str, "The group's key or id.")
                        .doc("An entity leaves a group (15 §9.2). Raises `group_left`. Returns whether it was a member."),
                )
            },
            run: |sim, call, cause| {
                let who = arg_entity(sim, call, 0, "who")?;
                let name = req_text(call, 1, "group")?;
                Ok(Value::Bool(group::leave_group(sim, who, name, cause)?))
            },
        },
        VerbDef {
            name: "set_role",
            build: |registry: &Registry| {
                let entity = registry.kind_id("entity")?;
                Some(
                    VerbSpec::new("set_role")
                        .arg("who", Type::Subject(entity), "The member.")
                        .arg("group", Type::Str, "The group's key or id.")
                        .optional_arg("role", Type::Str, "Its new role; left out, none.")
                        .doc("Gives a member of a group another role (15 §9.2). Raises `role_changed`."),
                )
            },
            run: |sim, call, cause| {
                let who = arg_entity(sim, call, 0, "who")?;
                let name = req_text(call, 1, "group")?;
                let role = opt_text(call, 2, "role")?;
                group::set_role(sim, who, name, role, cause)?;
                Ok(Value::Null)
            },
        },
        VerbDef {
            name: "create_group",
            build: |_registry: &Registry| {
                Some(
                    VerbSpec::new("create_group")
                        .arg("def", Type::Str, "The group definition's id.")
                        .optional_arg("name", Type::Str, "What it is called.")
                        .doc("Makes a group in play (15 §9.2): a gang, a cult, a raised horde. Raises `group_created`. Returns its key."),
                )
            },
            run: |sim, call, cause| {
                let def = req_text(call, 0, "def")?;
                let name = opt_text(call, 1, "name")?;
                Ok(text(group::create_group(sim, def, name, cause)?))
            },
        },
        VerbDef {
            name: "dissolve_group",
            build: |_registry: &Registry| {
                Some(
                    VerbSpec::new("dissolve_group")
                        .arg("group", Type::Str, "The group's key or id.")
                        .doc("Dissolves a group (15 §9.2): every member leaves it, with `group_left`, and what it kept goes. Raises `group_dissolved`."),
                )
            },
            run: |sim, call, cause| {
                let name = req_text(call, 0, "group")?;
                group::dissolve_group(sim, name, cause)?;
                Ok(Value::Null)
            },
        },
        VerbDef {
            name: "set_group_relation",
            build: |_registry: &Registry| {
                Some(
                    VerbSpec::new("set_group_relation")
                        .arg("group", Type::Str, "The group's key or id.")
                        .arg("toward", Type::Any, "A group's key or id, or an entity, as the persona it presents.")
                        .arg("kind", Type::Str, "The relation kind's id, qualified or bare.")
                        .arg("value", Type::Real, "The value, kept within the kind's range.")
                        .doc("Sets what a group holds of a kind toward someone (15 §9.2): its members who hold nothing of their own use it. Returns the value now."),
                )
            },
            run: |sim, call, _| {
                let name = req_text(call, 0, "group")?;
                let toward = toward_for_group(sim, arg(call, 1))?;
                let kind = kind_arg(sim, call, 2)?;
                let value = req_real(call, 3, "value")?;
                Ok(Value::Real(group::set_group_relation(
                    sim, name, toward, kind, value,
                )?))
            },
        },
        VerbDef {
            name: "report_to_group",
            build: |registry: &Registry| {
                let entity = registry.kind_id("entity")?;
                Some(
                    VerbSpec::new("report_to_group")
                        .arg("who", Type::Subject(entity), "The member who reports.")
                        .arg("group", Type::Str, "The group's key or id.")
                        .optional_arg("event", Type::Str, "The event type of the memories reported.")
                        .optional_arg("about", Type::Subject(entity), "Whom: the memories naming whom it takes them for, and its track of them.")
                        .doc("A member adds what it knows to its group's knowledge (15 §9.2), which counts as told to every other member at once. Returns how many things were added."),
                )
            },
            run: |sim, call, _| {
                let who = arg_entity(sim, call, 0, "who")?;
                let name = req_text(call, 1, "group")?;
                let event = opt_text(call, 2, "event")?;
                let about = opt_entity(sim, call, 3, "about")?;
                let added = group::report_to_group(sim, who, name, event, about)?;
                Ok(Value::Int(i64::try_from(added).unwrap_or(i64::MAX)))
            },
        },
        VerbDef {
            name: "say",
            build: |registry: &Registry| {
                let entity = registry.kind_id("entity")?;
                Some(
                    VerbSpec::new("say")
                        .arg("who", Type::Subject(entity), "Who speaks.")
                        .arg("kind", Type::Str, "The message kind's id.")
                        .optional_arg("to", Type::Subject(entity), "To whom; left out with no `group`, to nobody in particular.")
                        .optional_arg("group", Type::Str, "To which group, by key or id.")
                        .optional_arg("about", Type::Subject(entity), "What it is about. A report carries what the speaker knows of whom it takes them for.")
                        .optional_arg("event", Type::Str, "For a report: the event type of the memories it carries.")
                        .optional_arg("line", Type::Str, "The `lines` table it is said with.")
                        .optional_arg("language", Type::Str, "The language it is said in, for `understands`.")
                        .optional_arg("signal", Type::Str, "The signal kind it is said with, if not the message kind's.")
                        .optional_arg("strength", Type::Real, "How loud, if not the message kind's.")
                        .doc("Says something aloud (15 §11): a signal carrying a message, which whoever perceives it and understands it hears (`message_heard`). Said to them, a report gives its beliefs and an order is received. Returns how many it was sent to."),
                )
            },
            run: verb_say,
        },
        VerbDef {
            name: "order",
            build: |registry: &Registry| {
                let entity = registry.kind_id("entity")?;
                let action = registry.def_type_id("action")?;
                Some(
                    VerbSpec::new("order")
                        .arg("from", Type::Subject(entity), "Who gives it.")
                        .arg("to", Type::Subject(entity), "To whom.")
                        .arg("action", Type::Id(action), "What it says to do.")
                        .optional_arg("with", Type::map(Type::Any), "The action's parameters, by name.")
                        .optional_arg("until", Type::Real, "How long it stands once received, in seconds; for good if left out.")
                        .optional_arg("kind", Type::Str, "The message kind it is said as; left out, the first that carries orders.")
                        .optional_arg("signal", Type::Str, "The signal kind it is said with, if not the message kind's.")
                        .optional_arg("strength", Type::Real, "How loud, if not the message kind's.")
                        .doc("Gives an order aloud (15 §10). It must be heard; the one it is given to obeys if it holds toward whom it takes the giver for a relation of a kind flagged `orders` (hook `order_received`), and then keeps it (`order_given`), else refuses it (`order_refused`). Returns how many it was sent to."),
                )
            },
            run: verb_order,
        },
        VerbDef {
            name: "cancel_order",
            build: |registry: &Registry| {
                let entity = registry.kind_id("entity")?;
                Some(
                    VerbSpec::new("cancel_order")
                        .arg("who", Type::Subject(entity), "Who keeps it.")
                        .optional_arg("order", Type::Int, "Its number, from `order_given`; left out, every order it keeps.")
                        .doc("Withdraws an order an entity keeps, or all of them (15 §10). Raises `order_cancelled`. Returns how many went."),
                )
            },
            run: |sim, call, cause| {
                let who = arg_entity(sim, call, 0, "who")?;
                let chosen = match arg(call, 1) {
                    Value::Null => None,
                    Value::Int(id) if *id >= 0 => Some(id.unsigned_abs()),
                    _ => return Err(bad("order", "an order's number")),
                };
                let gone = order::cancel_order(sim, who, chosen, cause)?;
                Ok(Value::Int(i64::try_from(gone).unwrap_or(i64::MAX)))
            },
        },
    ]
}
