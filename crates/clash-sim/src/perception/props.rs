//! What perception offers to expressions and rules (15 §6, §7.2, §15):
//! the belief properties of a mind, the functions `perceives` and
//! `could_perceive`, and the verbs.
//!
//! # Belief properties
//!
//! Each is a property of an `entity`, read of the character itself:
//! `self.believed_loc(x)`. A mind context may read any property of its
//! own subject (05 §7.3), so these are what a goal, a method or a
//! tactic decides by; none is marked safe for belief, because read of
//! another entity it would tell one mind what another believes. `x` is
//! any entity a rule has bound; the answer is about the persona the mind
//! perceives it as, and is null (false, for `knows` and `perceives`) if
//! the mind has never perceived it.

use clash_core::{EntityId, Real};
use clash_expr::{Registry, Type, Value, VerbSpec};
use clash_world::{Cause, Layer, Location};

use super::belief::{self, Knowing, Memory, PersonaId};
use super::defs::SignalKindId;
use super::detect::{self, local_now};
use super::paths;
use super::percept;
use super::reason as why;
use super::senses;
use super::signal::{self, Signal};
use crate::Sim;
use crate::event::Subject;
use crate::reason::Reason;
use crate::registry::{FunctionDef, PropertyDef};
use crate::rule::VerbCall;
use crate::verb::VerbDef;

/// The heading perception's functions are listed under.
const GROUP: &str = "Perception";

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

fn opt<T: Into<Value>>(value: Option<T>) -> Value {
    value.map_or(Value::Null, Into::into)
}

/// A place a value names: a cell, an element's cell, or where an entity
/// is, with the height it stands at.
fn place_of(sim: &Sim, value: Option<&Value>) -> Option<(Location, u16)> {
    match sim.subject_of(value?)? {
        Subject::Loc(loc) | Subject::Element(loc, _) => Some((loc, 0)),
        Subject::Entity(id) => {
            let spot = signal::source_of(sim, id)?;
            Some((spot.loc, spot.alt_mm))
        }
        _ => None,
    }
}

fn kind_of(sim: &Sim, value: Option<&Value>) -> Option<SignalKindId> {
    sim.perception_defs.kind_named(value?.as_str()?)
}

fn how_name(sim: &Sim, how: Knowing) -> String {
    match how {
        Knowing::Perceived(kind) => sim.perception_defs.kind_name(kind).to_owned(),
        Knowing::Told => "told".to_owned(),
        Knowing::Inferred => "inferred".to_owned(),
        Knowing::Carried => "carried".to_owned(),
    }
}

fn layer_named(name: &str) -> Option<Layer> {
    match name {
        "floor" => Some(Layer::Floor),
        "block" => Some(Layer::Block),
        "fixture" => Some(Layer::Fixture),
        _ => None,
    }
}

/// The belief properties of a mind (15 §7.2) and `facing` (15 §15).
pub(crate) fn properties() -> Vec<PropertyDef> {
    let p = PropertyDef::new;
    vec![
        p(
            "entity",
            "knows",
            "bool",
            "Whether this mind has a track of what it perceives `x` as (15 §7.2). False for what it has never perceived.",
            |sim, subject, args| {
                Value::Bool(
                    mind_of(sim, subject)
                        .zip(entity_of(sim, args.first()))
                        .is_some_and(|(mind, x)| belief::track_of(sim, mind, x).is_some()),
                )
            },
        )
        .arg("x", "entity", "Whom or what it is asked about.")
        .changed_by(&["detected", "lost_track"]),
        p(
            "entity",
            "believed_loc",
            "loc?",
            "Where this mind last perceived `x`, or was told it was (15 §7.2). Null if it never has.",
            |sim, subject, args| {
                opt(mind_of(sim, subject)
                    .zip(entity_of(sim, args.first()))
                    .and_then(|(mind, x)| belief::believed_loc(sim, mind, x))
                    .map(|loc| sim.subject_value(Subject::Loc(loc))))
            },
        )
        .arg("x", "entity", "Whom or what it is asked about."),
        p(
            "entity",
            "last_seen",
            "real?",
            "Seconds of this mind's own time since it last perceived `x` or heard of it (15 §7.2). Null if it never has.",
            |sim, subject, args| {
                opt(mind_of(sim, subject)
                    .zip(entity_of(sim, args.first()))
                    .and_then(|(mind, x)| belief::last_seen(sim, mind, x)))
            },
        )
        .arg("x", "entity", "Whom or what it is asked about."),
        p(
            "entity",
            "last_seen_at",
            "real?",
            "The local time of this mind's clock, in seconds, when it last perceived `x` or heard of it (15 §7.2). Null if it never has.",
            |sim, subject, args| {
                opt(mind_of(sim, subject)
                    .zip(entity_of(sim, args.first()))
                    .and_then(|(mind, x)| belief::track_of(sim, mind, x))
                    .map(|track| track.at().as_secs_real()))
            },
        )
        .arg("x", "entity", "Whom or what it is asked about."),
        p(
            "entity",
            "confidence",
            "real?",
            "How sure this mind is of its track of `x`, 0 to 1: halved for every `mind.confidence_halflife` since it last perceived it (15 §7.2). Null if it never has.",
            |sim, subject, args| {
                opt(mind_of(sim, subject)
                    .zip(entity_of(sim, args.first()))
                    .and_then(|(mind, x)| belief::confidence(sim, mind, x)))
            },
        )
        .arg("x", "entity", "Whom or what it is asked about."),
        p(
            "entity",
            "awareness",
            "real?",
            "This mind's awareness of `x` now, the highest through any of its senses, 0 to 1 (15 §5.1). Null if it has none.",
            |sim, subject, args| {
                opt(mind_of(sim, subject)
                    .zip(entity_of(sim, args.first()))
                    .and_then(|(mind, x)| detect::awareness_of(sim, mind, x)))
            },
        )
        .arg("x", "entity", "Whom or what it is asked about.")
        .changed_by(&["suspicion_raised", "detected", "lost_track"]),
        p(
            "entity",
            "seen_how",
            "string?",
            "How this mind last knew of `x`: the qualified id of the kind of signal it perceived it by, or `told`, `inferred` or `carried` (15 §7.1). Null if it never has.",
            |sim, subject, args| {
                opt(mind_of(sim, subject)
                    .zip(entity_of(sim, args.first()))
                    .and_then(|(mind, x)| belief::track_of(sim, mind, x))
                    .map(|track| how_name(sim, track.how())))
            },
        )
        .arg("x", "entity", "Whom or what it is asked about."),
        p(
            "entity",
            "perceives",
            "bool",
            "Whether this mind has `x` detected now (15 §6): its awareness reached 1 through some sense and has not fallen to 0.",
            |sim, subject, args| {
                Value::Bool(
                    mind_of(sim, subject)
                        .zip(entity_of(sim, args.first()))
                        .is_some_and(|(mind, x)| detect::perceives(sim, mind, x)),
                )
            },
        )
        .arg("x", "entity", "Whom or what it is asked about.")
        .changed_by(&["detected", "lost_track"]),
        p(
            "entity",
            "could_perceive",
            "bool?",
            "Whether a signal of a kind at a place would reach this mind now through one of its senses (15 §6): a cell, or where an entity stands. Null for a place or a kind that names nothing.",
            |sim, subject, args| {
                let (Some(mind), Some((at, alt)), Some(kind)) = (
                    mind_of(sim, subject),
                    place_of(sim, args.first()),
                    kind_of(sim, args.get(1)),
                ) else {
                    return Value::Null;
                };
                Value::Bool(percept::could_perceive(sim, mind, at, alt, kind))
            },
        )
        .arg("at", "any", "A cell, or something that is somewhere.")
        .arg("kind", "string", "The signal kind's id, qualified or bare."),
        p(
            "entity",
            "facing",
            "real?",
            "Which way it faces, in degrees clockwise from north (15 §4). Null for something with no senses.",
            |sim, subject, _| {
                opt(mind_of(sim, subject)
                    .and_then(|id| sim.components.senses.get(id))
                    .map(senses::Senses::facing))
            },
        ),
        p(
            "entity",
            "remembers",
            "bool",
            "Whether this character's map memory holds a cell (15 §13): only directly controlled characters keep one.",
            |sim, subject, args| {
                Value::Bool(
                    mind_of(sim, subject)
                        .zip(place_of(sim, args.first()))
                        .is_some_and(|(mind, (at, _))| {
                            sim.cosmos.normalize(at).is_ok_and(|at| {
                                sim.components
                                    .map_memory
                                    .get(mind)
                                    .is_some_and(|memory| memory.cell(at).is_some())
                            })
                        }),
                )
            },
        )
        .arg("at", "any", "A cell, or something that is somewhere."),
        p(
            "entity",
            "knows_element",
            "bool?",
            "Whether this mind knows the floor, block or fixture of a cell for what it is (15 §7.3): it is not tagged `hidden`, or the mind has noted it. Null for a cell that cannot be read or a layer that is not `floor`, `block` or `fixture`.",
            |sim, subject, args| {
                let (Some(mind), Some((at, _)), Some(layer)) = (
                    mind_of(sim, subject),
                    place_of(sim, args.first()),
                    args.get(1).and_then(Value::as_str).and_then(layer_named),
                ) else {
                    return Value::Null;
                };
                match sim.cosmos.normalize(at) {
                    Ok(at) if sim.cosmos.is_loaded(at) => {
                        Value::Bool(paths::knows_element(sim, mind, at, layer))
                    }
                    _ => Value::Null,
                }
            },
        )
        .arg("at", "any", "A cell, or something that is somewhere.")
        .arg("layer", "string", "`floor`, `block` or `fixture`."),
    ]
}

/// `perceives` and `could_perceive` (15 §6). Truth about two entities:
/// for window kinds and world rules. A mind asks the properties of the
/// same names of itself.
pub(crate) fn functions() -> Vec<FunctionDef> {
    let f = FunctionDef::new;
    vec![
        f(
            "perceives",
            "bool?",
            GROUP,
            "Whether `a` has `b` detected now (15 §6). Null if either is not an entity. Window kinds use `perceives(self, source)` as their `noticed_if`.",
            |sim, args| {
                match (entity_of(sim, args.first()), entity_of(sim, args.get(1))) {
                    (Some(a), Some(b)) => Value::Bool(detect::perceives(sim, a, b)),
                    _ => Value::Null,
                }
            },
        )
        .arg("a", "entity", "Who perceives.")
        .arg("b", "entity", "Whom or what.")
        .changed_by(&["detected", "lost_track"]),
        f(
            "could_perceive",
            "bool?",
            GROUP,
            "Whether a signal of a kind at a place would reach `a` now through one of its senses (15 §6). Null for a place, an entity or a kind that names nothing.",
            |sim, args| {
                let (Some(a), Some((at, alt)), Some(kind)) = (
                    entity_of(sim, args.first()),
                    place_of(sim, args.get(1)),
                    kind_of(sim, args.get(2)),
                ) else {
                    return Value::Null;
                };
                Value::Bool(percept::could_perceive(sim, a, at, alt, kind))
            },
        )
        .arg("a", "entity", "Who would perceive.")
        .arg("at", "any", "A cell, or something that is somewhere.")
        .arg("kind", "string", "The signal kind's id, qualified or bare."),
    ]
}

fn arg_entity(sim: &Sim, call: &VerbCall<'_>, at: usize, name: &str) -> Result<EntityId, Reason> {
    entity_of(sim, call.args.get(at)).ok_or_else(|| {
        Reason::new(why::BAD_VALUE, format!("`{name}` names no entity")).with("argument", name)
    })
}

fn arg_real(call: &VerbCall<'_>, at: usize, name: &str) -> Result<Real, Reason> {
    call.args.get(at).and_then(Value::as_number).ok_or_else(|| {
        Reason::new(why::BAD_VALUE, format!("`{name}` is a number")).with("argument", name)
    })
}

fn arg_kind(sim: &Sim, call: &VerbCall<'_>, at: usize) -> Result<SignalKindId, Reason> {
    let name = call.args.get(at).and_then(Value::as_str).unwrap_or("");
    sim.perception_defs.kind_named(name).ok_or_else(|| {
        Reason::new(
            why::UNKNOWN_KIND,
            format!("there is no signal kind `{name}`"),
        )
        .with("kind", name)
    })
}

fn verb_emit(sim: &mut Sim, call: &VerbCall<'_>, cause: Cause) -> Result<Value, Reason> {
    let kind = arg_kind(sim, call, 0)?;
    let (at, alt) = place_of(sim, call.args.get(1))
        .ok_or_else(|| Reason::new(why::BAD_VALUE, "`at` names no place").with("argument", "at"))?;
    let strength = arg_real(call, 2, "strength")?;
    let mut signal = Signal::new(kind, at, strength, cause).at_height(alt);
    if let Some(about) = entity_of(sim, call.args.get(3)) {
        signal = signal.about(about);
    }
    let emitted = signal::emit_signal(sim, signal)?;
    Ok(Value::Int(i64::from(emitted.received + emitted.delayed)))
}

fn verb_face(sim: &mut Sim, call: &VerbCall<'_>, _cause: Cause) -> Result<Value, Reason> {
    let who = arg_entity(sim, call, 0, "who")?;
    match call.args.get(1) {
        Some(Value::Int(_) | Value::Real(_)) => {
            senses::set_facing(sim, who, arg_real(call, 1, "toward")?)?;
        }
        other => {
            let (at, _) = place_of(sim, other).ok_or_else(|| {
                Reason::new(
                    why::BAD_VALUE,
                    "`toward` is a place, an entity or a number of degrees",
                )
                .with("argument", "toward")
            })?;
            senses::face_toward(sim, who, at)?;
        }
    }
    Ok(Value::Null)
}

fn verb_set_awareness(sim: &mut Sim, call: &VerbCall<'_>, _cause: Cause) -> Result<Value, Reason> {
    let who = arg_entity(sim, call, 0, "who")?;
    let of = arg_entity(sim, call, 1, "of")?;
    let name = call.args.get(2).and_then(Value::as_str).unwrap_or("");
    let sense = sim.perception_defs.sense_named(name).ok_or_else(|| {
        Reason::new(why::UNKNOWN_SENSE, format!("there is no sense `{name}`")).with("sense", name)
    })?;
    let level = arg_real(call, 3, "level")?;
    detect::set_awareness(sim, who, of, sense, level)?;
    Ok(Value::Null)
}

fn verb_add_track(sim: &mut Sim, call: &VerbCall<'_>, _cause: Cause) -> Result<Value, Reason> {
    let who = arg_entity(sim, call, 0, "who")?;
    let of = arg_entity(sim, call, 1, "of")?;
    let at = place_of(sim, call.args.get(2)).map(|(at, _)| at);
    let how = match call.args.get(3).and_then(Value::as_str) {
        None | Some("told") => Knowing::Told,
        Some("inferred") => Knowing::Inferred,
        Some("carried") => Knowing::Carried,
        Some(other) => {
            return Err(Reason::new(
                why::BAD_VALUE,
                format!("`how` is `told`, `inferred` or `carried`, not `{other}`"),
            )
            .with("argument", "how"));
        }
    };
    let confidence = call
        .args
        .get(4)
        .and_then(Value::as_number)
        .unwrap_or(Real::ONE);
    belief::add_track(sim, who, of, at, how, confidence)?;
    Ok(Value::Null)
}

fn verb_clear_track(sim: &mut Sim, call: &VerbCall<'_>, _cause: Cause) -> Result<Value, Reason> {
    let who = arg_entity(sim, call, 0, "who")?;
    let of = entity_of(sim, call.args.get(1));
    let cleared = match of {
        Some(of) => belief::clear_track(sim, who, of)?,
        None => false,
    };
    Ok(Value::Bool(cleared))
}

fn verb_remember(sim: &mut Sim, call: &VerbCall<'_>, _cause: Cause) -> Result<Value, Reason> {
    let who = arg_entity(sim, call, 0, "who")?;
    let event = call
        .args
        .get(1)
        .and_then(Value::as_str)
        .ok_or_else(|| Reason::new(why::BAD_VALUE, "`event` is a text").with("argument", "event"))?
        .to_owned();
    let at = place_of(sim, call.args.get(2)).map(|(at, _)| at);
    let salience = call
        .args
        .get(3)
        .and_then(Value::as_number)
        .unwrap_or(Real::of(0.5));
    // Whom the mind takes the actor for: the persona it perceives them
    // as (15 §8.2).
    let actor: Option<PersonaId> =
        entity_of(sim, call.args.get(4)).map(|actor| belief::persona_of(sim, who, actor));
    let memory = Memory {
        event,
        loc: at,
        at: local_now(sim, who),
        roles: vec![("actor".to_owned(), actor)],
        how: Knowing::Inferred,
        confidence: Real::ONE,
        salience,
        source: None,
    };
    belief::remember(sim, who, memory)?;
    Ok(Value::Null)
}

fn sense_arg(sim: &Sim, call: &VerbCall<'_>, at: usize) -> Result<super::SenseId, Reason> {
    let name = call.args.get(at).and_then(Value::as_str).unwrap_or("");
    sim.perception_defs.sense_named(name).ok_or_else(|| {
        Reason::new(why::UNKNOWN_SENSE, format!("there is no sense `{name}`")).with("sense", name)
    })
}

fn verb_attach_sense(sim: &mut Sim, call: &VerbCall<'_>, _cause: Cause) -> Result<Value, Reason> {
    let who = arg_entity(sim, call, 0, "who")?;
    let sense = sense_arg(sim, call, 1)?;
    senses::attach_sense(sim, who, sense)?;
    Ok(Value::Null)
}

fn verb_detach_sense(sim: &mut Sim, call: &VerbCall<'_>, _cause: Cause) -> Result<Value, Reason> {
    let who = arg_entity(sim, call, 0, "who")?;
    let sense = sense_arg(sim, call, 1)?;
    senses::detach_sense(sim, who, sense)?;
    Ok(Value::Null)
}

/// The verbs of perception (15 §15).
pub(crate) fn verbs() -> Vec<VerbDef> {
    vec![
        VerbDef {
            name: "attach_sense",
            build: |registry: &Registry| {
                let entity = registry.kind_id("entity")?;
                Some(
                    VerbSpec::new("attach_sense")
                        .arg("who", Type::Subject(entity), "Who is given the sense.")
                        .arg("sense", Type::Str, "The sense's id, qualified or bare.")
                        .doc("Gives an entity a sense (15 §4): it sweeps from its next interval. Refused with `perception.unknown_sense` and `perception.has_sense`."),
                )
            },
            run: verb_attach_sense,
        },
        VerbDef {
            name: "detach_sense",
            build: |registry: &Registry| {
                let entity = registry.kind_id("entity")?;
                Some(
                    VerbSpec::new("detach_sense")
                        .arg("who", Type::Subject(entity), "Who loses the sense.")
                        .arg("sense", Type::Str, "The sense's id, qualified or bare.")
                        .doc("Takes a sense from an entity (15 §4), with what it was aware of through it. Refused with `perception.unknown_sense` and `perception.no_sense`."),
                )
            },
            run: verb_detach_sense,
        },
        VerbDef {
            name: "emit_signal",
            build: |registry: &Registry| {
                let entity = registry.kind_id("entity")?;
                Some(
                    VerbSpec::new("emit_signal")
                        .arg("kind", Type::Str, "The signal kind's id, qualified or bare.")
                        .arg("at", Type::Any, "Where it comes from: a cell, or something that is somewhere.")
                        .arg("strength", Type::Real, "How strong it is where it starts.")
                        .optional_arg("about", Type::Subject(entity), "The entity it is about, if any.")
                        .doc("Gives off a signal (15 §3.2): every observer it reaches receives it, at once or, for a kind with `delay`, when its front arrives. Returns how many observers it was sent to."),
                )
            },
            run: verb_emit,
        },
        VerbDef {
            name: "face",
            build: |registry: &Registry| {
                let entity = registry.kind_id("entity")?;
                Some(
                    VerbSpec::new("face")
                        .arg("who", Type::Subject(entity), "Who turns.")
                        .arg("toward", Type::Any, "A cell, something that is somewhere, or a number of degrees clockwise from north.")
                        .doc("Turns an entity with senses to face a place or a direction (15 §4)."),
                )
            },
            run: verb_face,
        },
        VerbDef {
            name: "set_awareness",
            build: |registry: &Registry| {
                let entity = registry.kind_id("entity")?;
                Some(
                    VerbSpec::new("set_awareness")
                        .arg("who", Type::Subject(entity), "The observer.")
                        .arg("of", Type::Subject(entity), "Whom or what it is aware of.")
                        .arg("sense", Type::Str, "The sense's id, qualified or bare, which the observer must have.")
                        .arg("level", Type::Real, "The level, 0 to 1. 0 forgets the pair.")
                        .doc("Sets an observer's awareness of something through one of its senses, raising what crossing a level raises (15 §5.1)."),
                )
            },
            run: verb_set_awareness,
        },
        VerbDef {
            name: "add_track",
            build: |registry: &Registry| {
                let entity = registry.kind_id("entity")?;
                Some(
                    VerbSpec::new("add_track")
                        .arg("who", Type::Subject(entity), "The mind.")
                        .arg("of", Type::Subject(entity), "Whom it gains a track of.")
                        .optional_arg("at", Type::Any, "Where it is believed to be: a cell, or something that is somewhere.")
                        .optional_arg("how", Type::Str, "`told` (the default), `inferred` or `carried`.")
                        .optional_arg("confidence", Type::Real, "0 to 1; 1 if left out.")
                        .doc("Gives a mind a track of someone, or brings the one it has up to date (15 §7.1)."),
                )
            },
            run: verb_add_track,
        },
        VerbDef {
            name: "clear_track",
            build: |registry: &Registry| {
                let entity = registry.kind_id("entity")?;
                Some(
                    VerbSpec::new("clear_track")
                        .arg("who", Type::Subject(entity), "The mind.")
                        .arg("of", Type::Subject(entity), "Whose track it loses.")
                        .doc(
                            "Takes a mind's track of someone away. Returns whether there was one.",
                        ),
                )
            },
            run: verb_clear_track,
        },
        VerbDef {
            name: "remember",
            build: |registry: &Registry| {
                let entity = registry.kind_id("entity")?;
                Some(
                    VerbSpec::new("remember")
                        .arg("who", Type::Subject(entity), "The mind.")
                        .arg("event", Type::Str, "What happened: an event type or any name.")
                        .optional_arg("at", Type::Any, "Where: a cell, or something that is somewhere.")
                        .optional_arg("salience", Type::Real, "How much it matters, 0 to 1; 0.5 if left out.")
                        .optional_arg("actor", Type::Subject(entity), "Who the mind takes to have done it.")
                        .doc("A mind remembers something, as inferred (15 §7.1). The least salient memory is forgotten to make room."),
                )
            },
            run: verb_remember,
        },
    ]
}
