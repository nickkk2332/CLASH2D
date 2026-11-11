//! The verbs of minds and vital state (06 §8.2, §8.3), as the engine's
//! verb table lists them.
//!
//! | Verb | Arguments | Does |
//! |------|-----------|------|
//! | `die` | `who` | The hook `entity_dying`, then the mind stops for good |
//! | `revive` | `who` | A dead entity's mind drives it again |
//! | `create_mind` | `in`, `def?` | Gives an entity a new mind |
//! | `remove_mind` | `from` | Removes and discards it |
//! | `transfer_mind` | `from`, `to`, `swap?` | Moves a mind, with what is seated in it and the control mode |
//! | `set_driver` | `of`, `driver?` | Makes one entity act for another's body; left out, nobody does |

use clash_expr::{Registry, Type, Value, VerbSpec};

use crate::life::args;
use crate::verb::VerbDef;

/// A verb that takes one entity.
fn of_entity(
    registry: &Registry,
    name: &str,
    arg: &str,
    what: &str,
    doc: &str,
) -> Option<VerbSpec> {
    let entity = registry.kind_id("entity")?;
    Some(
        VerbSpec::new(name)
            .arg(arg, Type::Subject(entity), what)
            .direct(arg)
            .doc(doc),
    )
}

/// The verbs of 06 §8.2 and §8.3, for the engine's verb table.
pub fn verbs() -> Vec<VerbDef> {
    vec![
        VerbDef {
            name: "die",
            build: |registry| {
                of_entity(
                    registry,
                    "die",
                    "who",
                    "Who stops.",
                    "An entity's mind stops driving it for good, after the hook `entity_dying`. Control of it is released; the entity remains (06 §8.3).",
                )
            },
            run: |sim, call, cause| {
                let entity = args::entity(sim, call, 0, "who")?;
                sim.die(entity, cause)?;
                Ok(Value::Null)
            },
        },
        VerbDef {
            name: "revive",
            build: |registry| {
                of_entity(
                    registry,
                    "revive",
                    "who",
                    "Who starts again.",
                    "A dead entity's mind drives it again (06 §8.3).",
                )
            },
            run: |sim, call, cause| {
                let entity = args::entity(sim, call, 0, "who")?;
                sim.revive(entity, cause)?;
                Ok(Value::Null)
            },
        },
        VerbDef {
            name: "create_mind",
            build: |registry| {
                Some(
                    of_entity(
                        registry,
                        "create_mind",
                        "in",
                        "What is given a mind.",
                        "Gives an entity a new mind, awake. Refused for one that has a mind (06 §8.2).",
                    )?
                    .optional_arg(
                        "def",
                        Type::Str,
                        "The behaviour profile for the AI layer: the id of a `mind` definition, kept as written.",
                    ),
                )
            },
            run: |sim, call, cause| {
                let entity = args::entity(sim, call, 0, "in")?;
                let profile = args::optional_text(call, 1, "def")?.map(str::to_owned);
                sim.create_mind(entity, profile, cause)?;
                Ok(Value::Null)
            },
        },
        VerbDef {
            name: "remove_mind",
            build: |registry| {
                of_entity(
                    registry,
                    "remove_mind",
                    "from",
                    "What loses its mind.",
                    "Removes an entity's mind and discards it, with what was seated in it (06 §8.2).",
                )
            },
            run: |sim, call, cause| {
                let entity = args::entity(sim, call, 0, "from")?;
                sim.remove_mind(entity, cause)?;
                Ok(Value::Null)
            },
        },
        VerbDef {
            name: "transfer_mind",
            build: |registry| {
                let entity = registry.kind_id("entity")?;
                Some(
                    VerbSpec::new("transfer_mind")
                        .arg("from", Type::Subject(entity), "Whose mind moves.")
                        .arg("to", Type::Subject(entity), "Where it goes.")
                        .optional_arg(
                            "swap",
                            Type::Bool,
                            "True to let a mind that `to` has go the other way; left out, `to` must have none.",
                        )
                        .doc("Moves a mind to another entity, with the attributes, pools and tracks seated in it and the control mode, after the hook `mind_transferring` (06 §8.2)."),
                )
            },
            run: |sim, call, cause| {
                let from = args::entity(sim, call, 0, "from")?;
                let to = args::entity(sim, call, 1, "to")?;
                let swap = args::flag(call, 2, "swap")?;
                sim.transfer_mind(from, to, swap, cause)?;
                Ok(Value::Null)
            },
        },
        VerbDef {
            name: "set_driver",
            build: |registry| {
                let entity = registry.kind_id("entity")?;
                Some(
                    VerbSpec::new("set_driver")
                        .arg("of", Type::Subject(entity), "Whose body is acted for.")
                        .optional_arg(
                            "driver",
                            Type::Subject(entity),
                            "Who acts for it. Left out, nobody does.",
                        )
                        .doc("Makes one entity act for another's body without moving any mind, after the hook `mind_transferring` (06 §8.2)."),
                )
            },
            run: |sim, call, cause| {
                let of = args::entity(sim, call, 0, "of")?;
                let driver = args::optional_entity(sim, call, 1, "driver")?;
                sim.set_driver(of, driver, cause)?;
                Ok(Value::Null)
            },
        },
    ]
}
