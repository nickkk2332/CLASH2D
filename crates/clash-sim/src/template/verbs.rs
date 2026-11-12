//! The verbs of templates and coming and going (06 §10, §12.2), as the
//! engine's verb table lists them.
//!
//! | Verb | Arguments | Gives |
//! |------|-----------|-------|
//! | `spawn` | `template`, `at?`, `in?`, `also?`, `random?`, `seed?`, `anonymous?` | The entity made |
//! | `make_notable` | `who` | null |

use clash_core::Real;
use clash_data::Def;
use clash_expr::{Type, Value, VerbSpec};

use super::{CharacterSpec, TemplateDef};
use crate::entity::Place;
use crate::event::Subject;
use crate::life::args;
use crate::reason::Reason;
use crate::verb::VerbDef;

/// The verbs of templates, for the engine's verb table.
pub fn verbs() -> Vec<VerbDef> {
    vec![
        VerbDef {
            name: "spawn",
            build: |registry| {
                let template = registry.def_type_id(TemplateDef::TYPE)?;
                let entity = registry.kind_id("entity")?;
                let loc = registry.kind_id("loc")?;
                Some(
                    VerbSpec::new("spawn")
                        .arg("template", Type::Id(template), "The first template: a species, say.")
                        .optional_arg("at", Type::Subject(loc), "The cell to put it in.")
                        .optional_arg("in", Type::Subject(entity), "The entity to put it inside, if not `at` a cell. Left out with `at`, it is made out of the world.")
                        .optional_arg("also", Type::list(Type::Id(template)), "More templates, applied after the first in order.")
                        .optional_arg("random", Type::Bool, "True to make a random selection for every choice; left out, each takes its default.")
                        .optional_arg("seed", Type::Int, "What random selections are drawn from. Left out, 0.")
                        .optional_arg("anonymous", Type::Bool, "True to make it an anonymous individual of a population (06 §12.2).")
                        .doc("Makes an entity from templates and puts it in a place (06 §10.3). Gives the entity made."),
                )
            },
            run: |sim, call, cause| {
                let first = args::def(sim, call, 0, "template", TemplateDef::TYPE)?;
                let names = sim.template_defs();
                let mut templates = vec![first];
                if let Value::List(more) = args::value(call, 3) {
                    for item in more.iter() {
                        match item {
                            Value::Id(id) => templates.push(id.index as usize),
                            _ => {
                                return Err(Reason::new(
                                    args::BAD_ARGUMENT,
                                    "the argument `also` must be a list of templates",
                                )
                                .with("argument", "also"));
                            }
                        }
                    }
                }
                let mut spec = CharacterSpec::of(
                    templates
                        .iter()
                        .filter_map(|index| {
                            super::TemplateId::from_index(*index)
                                .and_then(|id| names.name(id))
                                .map(str::to_owned)
                        })
                        .collect::<Vec<_>>(),
                );
                if args::flag(call, 4, "random")? {
                    let seed = args::optional_real(call, 5, "seed")?.unwrap_or(Real::ZERO);
                    spec = spec.random(seed.to_i64() as u64);
                }
                spec.anonymous = args::flag(call, 6, "anonymous")?;
                let place = match (
                    args::optional_handle(call, 1, "at")?,
                    args::optional_entity(sim, call, 2, "in")?,
                ) {
                    (Some(handle), _) => match sim.exprs().kinds().subject(handle) {
                        Some(Subject::Loc(loc)) => Place::at(loc),
                        _ => {
                            return Err(Reason::new(
                                args::BAD_ARGUMENT,
                                "the argument `at` must be a cell",
                            )
                            .with("argument", "at"));
                        }
                    },
                    (None, Some(container)) => Place::In(container),
                    (None, None) => Place::Stored,
                };
                let spawned = if spec.anonymous {
                    sim.spawn_anonymous(&spec, place, cause)?
                } else {
                    sim.spawn_template(&spec, place, cause)?
                };
                Ok(sim.subject_value(Subject::Entity(spawned.entity)))
            },
        },
        VerbDef {
            name: "make_notable",
            build: |registry| {
                let entity = registry.kind_id("entity")?;
                Some(
                    VerbSpec::new("make_notable")
                        .arg("who", Type::Subject(entity), "Who is to be remembered.")
                        .direct("who")
                        .doc("Makes an anonymous individual notable, so that it is kept when its tract unloads (06 §12.2)."),
                )
            },
            run: |sim, call, cause| {
                let entity = args::entity(sim, call, 0, "who")?;
                sim.make_notable(entity, cause);
                Ok(Value::Null)
            },
        },
    ]
}
