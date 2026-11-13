//! What minds offer to expressions and rules (09 §17): the properties of
//! a mind, read of the character itself, and the verbs.
//!
//! # Properties
//!
//! Each is a property of an `entity`, and none is safe for belief: read
//! of oneself in a goal (`self.weight('duty')`), and of anyone by a
//! world rule or the inspector.
//!
//! | Property | Type | Is |
//! |----------|------|----|
//! | `weight(name)` | `real` | The mind's weight of that name, with what its `adjust` entries add now; 0 if it has none |
//! | `goal` | `string?` | The qualified id of the goal it holds |
//! | `goal_about` | `any` | What that goal is about: an entity, a memory as a map, a cell, or null |
//! | `step` | `string?` | The kind of the step it is at: `go`, `do`, … |
//! | `last_failure` | `string?` | The code of the reason its last step or goal failed |
//! | `home` | `loc` | Where it belongs: where it first thought, or what `set_home` said; where it is, for one that has not thought |
//! | `mind_def` | `string?` | The qualified id of the `mind` it thinks by |
//! | `hostile(x)` | `bool` | Whether it holds `x` hostile: by a stance it took, or by a tag `x` presents that its mind's `hostile_to` names, once it has perceived `x` |
//! | `fears(x)` | `bool` | The same for `fear_of` |
//! | `stance(x)` | `string?` | `hostile`, `feared` or null |
//!
//! # Verbs
//!
//! | Verb | Arguments | Does |
//! |------|-----------|------|
//! | `think` | `who` | Makes a mind think at the next batch |
//! | `set_goal` | `who`, `goal`, `about?` | Makes a mind take up a goal now, about an entity, a cell or nothing; it is held until it is done, fails or `clear_goal` |
//! | `clear_goal` | `who` | Makes a mind give up the goal it holds; it thinks again |
//! | `set_stance` | `who`, `of`, `stance` | Holds `of` hostile, feared, or neither (`none`) |
//! | `set_home` | `who`, `at` | Says where a mind belongs |
//! | `set_mind_def` | `who`, `mind` | Makes a character think by another `mind` |

use clash_expr::{Type, Value, VerbSpec};
use clash_world::Cause;

use super::defs::{GoalId, MindDef, MindDefId};
use super::reason as why;
use super::state::{AboutHeld, Stance};
use super::think::{self, goal_name, mind_def, mind_def_of};
use crate::Sim;
use crate::event::Subject;
use crate::life::args;
use crate::perception::belief;
use crate::reason::Reason;
use crate::registry::PropertyDef;
use crate::rule::VerbCall;
use crate::verb::VerbDef;

fn mind_of(sim: &Sim, subject: Subject) -> Option<clash_core::EntityId> {
    match subject {
        Subject::Entity(id) if sim.entities.contains(id) => Some(id),
        _ => None,
    }
}

fn entity_of(sim: &Sim, value: Option<&Value>) -> Option<clash_core::EntityId> {
    match sim.subject_of(value?)? {
        Subject::Entity(id) if sim.entities.contains(id) => Some(id),
        _ => None,
    }
}

fn opt<T: Into<Value>>(value: Option<T>) -> Value {
    value.map_or(Value::Null, Into::into)
}

/// The properties of minds (09 §17).
pub fn properties() -> Vec<PropertyDef> {
    let p = PropertyDef::new;
    vec![
        p(
            "entity",
            "weight",
            "real",
            "This character's mind's weight of a name (09 §3.1), with what the mind's `adjust` entries whose conditions hold add; 0 if it has none.",
            |sim, subject, args| {
                let Some(mind) = mind_of(sim, subject) else {
                    return Value::Null;
                };
                let name = args.first().and_then(Value::as_str).unwrap_or("");
                let weight = think::weights(sim, mind)
                    .into_iter()
                    .find(|(have, _)| have == name)
                    .map_or(clash_core::Real::ZERO, |(_, weight)| weight);
                Value::Real(weight)
            },
        )
        .arg("name", "string", "The weight's name, as the mind writes it."),
        p(
            "entity",
            "goal",
            "string?",
            "The qualified id of the goal this character's mind holds (09 §3.3). Null if none.",
            |sim, subject, _| {
                opt(mind_of(sim, subject)
                    .and_then(|mind| sim.components.mind_state.get(mind))
                    .and_then(|state| state.held())
                    .map(|held| goal_name(sim, held.goal)))
            },
        )
        .changed_by(&["goal_adopted", "goal_done", "goal_failed"]),
        p(
            "entity",
            "goal_about",
            "any",
            "What the goal this character's mind holds is about: an entity, a memory as a map, a cell, or null (09 §3.2).",
            |sim, subject, _| {
                let Some(held) = mind_of(sim, subject).and_then(|mind| {
                    Some((mind, sim.components.mind_state.get(mind)?.held()?.about.clone()))
                }) else {
                    return Value::Null;
                };
                about_value(sim, held.0, &held.1)
            },
        )
        .changed_by(&["goal_adopted"]),
        p(
            "entity",
            "step",
            "string?",
            "The kind of the step this character's mind is at: `go`, `do`, `choose`, `wait`, `look`, `say`, `set`, `settle`, `stance`, `keep_near` or `keep_from` (09 §4). Null if it holds no goal or has chosen no method.",
            |sim, subject, _| {
                opt(mind_of(sim, subject)
                    .and_then(|mind| sim.components.mind_state.get(mind))
                    .and_then(|state| state.held())
                    .and_then(|held| think::current_step_kind(sim, held))
                    .map(|kind| kind.name()))
            },
        ),
        p(
            "entity",
            "last_failure",
            "string?",
            "The code of the reason this character's last step or goal failed (09 §16). Null if none has.",
            |sim, subject, _| {
                opt(mind_of(sim, subject)
                    .and_then(|mind| sim.components.mind_state.get(mind))
                    .and_then(|state| state.last_failure())
                    .map(|reason| reason.code.clone()))
            },
        )
        .changed_by(&["step_failed", "goal_failed"]),
        p(
            "entity",
            "home",
            "loc",
            "Where this character's mind belongs: for a resident of a settlement, its home (06 §12.2); else where it first thought, or what `set_home` said; for one that has not thought, where it is. Null only for one that is nowhere.",
            |sim, subject, _| {
                opt(mind_of(sim, subject)
                    .and_then(|mind| {
                        crate::agent::resident_home(sim, mind)
                            .or_else(|| {
                                sim.components
                                    .mind_state
                                    .get(mind)
                                    .and_then(|state| state.home())
                            })
                            .or_else(|| sim.entities.location(mind))
                    })
                    .map(|loc| sim.subject_value(Subject::Loc(loc))))
            },
        ),
        p(
            "entity",
            "mind_def",
            "string?",
            "The qualified id of the `mind` definition this character thinks by. Null for one that thinks by none.",
            |sim, subject, _| {
                opt(mind_of(sim, subject)
                    .and_then(|mind| mind_def_of(sim, mind))
                    .and_then(|id| {
                        sim.defs
                            .try_registry::<MindDef>()
                            .map(|registry| registry.name(id).to_owned())
                    }))
            },
        ),
        p(
            "entity",
            "hostile",
            "bool",
            "Whether this character's mind holds `x` hostile (09 §3.4): by a stance it took, or, once it has perceived `x`, by a tag `x` presents that its mind's `hostile_to` names. Relations (15 §9) will take its place.",
            |sim, subject, args| {
                Value::Bool(
                    mind_of(sim, subject)
                        .zip(entity_of(sim, args.first()))
                        .is_some_and(|(mind, x)| {
                            think::stance_toward(sim, mind, x) == Some(Stance::Hostile)
                        }),
                )
            },
        )
        .arg("x", "entity", "Whom it is asked about.")
        .changed_by(&["detected", "perceived"]),
        p(
            "entity",
            "fears",
            "bool",
            "Whether this character's mind fears `x`: by a stance it took, or, once it has perceived `x`, by a tag `x` presents that its mind's `fear_of` names.",
            |sim, subject, args| {
                Value::Bool(
                    mind_of(sim, subject)
                        .zip(entity_of(sim, args.first()))
                        .is_some_and(|(mind, x)| {
                            think::stance_toward(sim, mind, x) == Some(Stance::Feared)
                        }),
                )
            },
        )
        .arg("x", "entity", "Whom it is asked about.")
        .changed_by(&["detected", "perceived"]),
        p(
            "entity",
            "stance",
            "string?",
            "How this character's mind stands toward `x`: `hostile`, `feared`, or null for neither.",
            |sim, subject, args| {
                opt(mind_of(sim, subject)
                    .zip(entity_of(sim, args.first()))
                    .and_then(|(mind, x)| think::stance_toward(sim, mind, x))
                    .map(Stance::name))
            },
        )
        .arg("x", "entity", "Whom it is asked about."),
    ]
}

/// What a held `about` is as expressions read it now.
fn about_value(sim: &Sim, mind: clash_core::EntityId, about: &AboutHeld) -> Value {
    match about {
        AboutHeld::None => Value::Null,
        AboutHeld::Track(persona) => sim.subject_value(Subject::Entity(persona.entity())),
        AboutHeld::Place(loc) => sim.subject_value(Subject::Loc(*loc)),
        AboutHeld::Memory(key) => sim
            .components
            .beliefs
            .get(mind)
            .and_then(|beliefs| {
                beliefs.memories().iter().find(|memory| {
                    memory.event == key.event && memory.loc == key.loc && memory.at == key.at
                })
            })
            .map_or(Value::Null, |memory| {
                let mut map = clash_expr::Map::new();
                map.insert("event".into(), Value::from(memory.event.as_str()));
                map.insert(
                    "loc".into(),
                    memory
                        .loc
                        .map_or(Value::Null, |loc| sim.subject_value(Subject::Loc(loc))),
                );
                Value::from(map)
            }),
    }
}

/// A mind that thinks by a `mind` definition, or the reason it does not.
fn thinking(sim: &Sim, who: clash_core::EntityId) -> Result<MindDefId, Reason> {
    mind_def_of(sim, who).ok_or_else(|| {
        Reason::new(
            why::NO_MIND_DEF,
            format!("entity {who} has no mind that thinks by a `mind` definition"),
        )
        .with("entity", who.to_string())
    })
}

fn verb_think(sim: &mut Sim, call: &VerbCall<'_>, _cause: Cause) -> Result<Value, Reason> {
    let who = args::entity(sim, call, 0, "who")?;
    thinking(sim, who)?;
    think::prompt(sim, who);
    Ok(Value::Null)
}

fn verb_set_goal(sim: &mut Sim, call: &VerbCall<'_>, _cause: Cause) -> Result<Value, Reason> {
    let who = args::entity(sim, call, 0, "who")?;
    let def = thinking(sim, who)?;
    let goal = args::optional_def(
        sim,
        call,
        1,
        "goal",
        <super::defs::GoalDef as clash_data::Def>::TYPE,
    )?
    .and_then(GoalId::from_index)
    .filter(|goal| think::goal_def(sim, *goal).is_some())
    .ok_or_else(|| {
        Reason::new(why::UNKNOWN_GOAL, "`goal` names no goal").with("argument", "goal")
    })?;
    let about = match sim.subject_of(args::value(call, 2)) {
        Some(Subject::Entity(entity)) => AboutHeld::Track(belief::persona_of(sim, who, entity)),
        Some(Subject::Loc(loc)) => AboutHeld::Place(loc),
        _ => AboutHeld::None,
    };
    if mind_def(sim, def).is_none() {
        return Err(Reason::new(
            why::NO_MIND_DEF,
            "the mind definition is not loaded",
        ));
    }
    super::ensure_state(sim, who);
    think::adopt(sim, who, goal, about, clash_core::Real::ONE, true);
    think::prompt(sim, who);
    Ok(Value::Null)
}

fn verb_clear_goal(sim: &mut Sim, call: &VerbCall<'_>, _cause: Cause) -> Result<Value, Reason> {
    let who = args::entity(sim, call, 0, "who")?;
    thinking(sim, who)?;
    let had = sim
        .components
        .mind_state
        .get_mut(who)
        .and_then(|state| state.held.take())
        .is_some();
    think::prompt(sim, who);
    Ok(Value::Bool(had))
}

fn verb_set_stance(sim: &mut Sim, call: &VerbCall<'_>, _cause: Cause) -> Result<Value, Reason> {
    let who = args::entity(sim, call, 0, "who")?;
    thinking(sim, who)?;
    let of = args::entity(sim, call, 1, "of")?;
    let stance = match args::text(call, 2, "stance")? {
        "hostile" => Some(Stance::Hostile),
        "feared" => Some(Stance::Feared),
        "none" => None,
        other => {
            return Err(Reason::new(
                why::BAD_VALUE,
                format!("`stance` is `hostile`, `feared` or `none`, not `{other}`"),
            )
            .with("argument", "stance"));
        }
    };
    think::set_stance(sim, who, of, stance);
    think::prompt(sim, who);
    Ok(Value::Null)
}

fn verb_set_home(sim: &mut Sim, call: &VerbCall<'_>, _cause: Cause) -> Result<Value, Reason> {
    let who = args::entity(sim, call, 0, "who")?;
    thinking(sim, who)?;
    let Some(Subject::Loc(at)) = sim.subject_of(args::value(call, 1)) else {
        return Err(Reason::new(why::BAD_VALUE, "`at` is a cell").with("argument", "at"));
    };
    super::ensure_state(sim, who);
    if let Some(state) = sim.components.mind_state.get_mut(who) {
        state.home = Some(at);
    }
    Ok(Value::Null)
}

fn verb_set_mind_def(sim: &mut Sim, call: &VerbCall<'_>, _cause: Cause) -> Result<Value, Reason> {
    let who = args::entity(sim, call, 0, "who")?;
    if sim.mind(who).is_none() {
        return Err(
            Reason::new(crate::mind::reason::NO_MIND, format!("{who} has no mind"))
                .with("entity", who.to_string()),
        );
    }
    let index = args::optional_def(
        sim,
        call,
        1,
        "mind",
        <super::defs::MindDef as clash_data::Def>::TYPE,
    )?
    .ok_or_else(|| Reason::new(why::BAD_VALUE, "`mind` names no mind").with("argument", "mind"))?;
    let name = sim
        .defs
        .try_registry::<MindDef>()
        .filter(|registry| index < registry.len())
        .and_then(|registry| MindDefId::from_index(index).map(|id| registry.name(id).to_owned()))
        .ok_or_else(|| {
            Reason::new(why::BAD_VALUE, "`mind` names no mind").with("argument", "mind")
        })?;
    super::ensure_state_with(sim, who, &name);
    if let Some(state) = sim.components.mind_state.get_mut(who) {
        state.def = Some(name);
        state.held = None;
    }
    think::prompt(sim, who);
    Ok(Value::Null)
}

/// The verbs of minds (09 §17).
pub fn verbs() -> Vec<VerbDef> {
    vec![
        VerbDef {
            name: "think",
            build: |registry| {
                let entity = registry.kind_id("entity")?;
                Some(
                    VerbSpec::new("think")
                        .arg("who", Type::Subject(entity), "The character whose mind thinks.")
                        .doc("Makes a mind think at the next batch (09 §3.3). Refused for a character that thinks by no `mind` definition."),
                )
            },
            run: verb_think,
        },
        VerbDef {
            name: "set_goal",
            build: |registry| {
                let entity = registry.kind_id("entity")?;
                let goal = registry.def_type_id(<super::defs::GoalDef as clash_data::Def>::TYPE)?;
                Some(
                    VerbSpec::new("set_goal")
                        .arg("who", Type::Subject(entity), "The character.")
                        .arg("goal", Type::Id(goal), "The goal.")
                        .optional_arg("about", Type::Any, "What it is about: an entity, as the mind perceives it, or a cell. Left out, nothing.")
                        .doc("Makes a mind take up a goal now, stopping what the character is doing where it can (09 §17). It is held until it is done, fails or `clear_goal`, whatever else scores higher."),
                )
            },
            run: verb_set_goal,
        },
        VerbDef {
            name: "clear_goal",
            build: |registry| {
                let entity = registry.kind_id("entity")?;
                Some(
                    VerbSpec::new("clear_goal")
                        .arg("who", Type::Subject(entity), "The character.")
                        .doc("Makes a mind give up the goal it holds and think again (09 §17). Gives whether it held one."),
                )
            },
            run: verb_clear_goal,
        },
        VerbDef {
            name: "set_stance",
            build: |registry| {
                let entity = registry.kind_id("entity")?;
                Some(
                    VerbSpec::new("set_stance")
                        .arg("who", Type::Subject(entity), "The character whose mind takes the stance.")
                        .arg("of", Type::Subject(entity), "Whom: the stance is kept toward what the mind perceives them as.")
                        .arg("stance", Type::Str, "`hostile`, `feared` or `none`.")
                        .doc("Makes a mind hold someone hostile or feared, or neither (09 §3.4). Relations (15 §9) will take its place."),
                )
            },
            run: verb_set_stance,
        },
        VerbDef {
            name: "set_home",
            build: |registry| {
                let entity = registry.kind_id("entity")?;
                let loc = registry.kind_id("loc")?;
                Some(
                    VerbSpec::new("set_home")
                        .arg("who", Type::Subject(entity), "The character.")
                        .arg("at", Type::Subject(loc), "The cell it belongs at.")
                        .doc("Says where a mind belongs: what `self.home` reads."),
                )
            },
            run: verb_set_home,
        },
        VerbDef {
            name: "set_mind_def",
            build: |registry| {
                let entity = registry.kind_id("entity")?;
                let mind = registry.def_type_id(<super::defs::MindDef as clash_data::Def>::TYPE)?;
                Some(
                    VerbSpec::new("set_mind_def")
                        .arg("who", Type::Subject(entity), "The character, which must have a mind.")
                        .arg("mind", Type::Id(mind), "The `mind` definition it thinks by from now on.")
                        .doc("Makes a character think by another `mind` definition (09 §17): it gives up the goal it holds and thinks again."),
                )
            },
            run: verb_set_mind_def,
        },
    ]
}
