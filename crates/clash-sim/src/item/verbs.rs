//! The verbs of items and holding (06 §9.2, §13): what an effect calls
//! to make, move, split and unmake items.
//!
//! Each verb runs the function of this module of the same meaning and
//! refuses with its reason (the table in [`ITEM_REASONS`]). An argument
//! that is missing or of the wrong kind is refused with
//! `item.bad_argument`. A verb that ends with an entity returns it: the
//! one that now holds the members it moved, which may be a stack they
//! joined.
//!
//! | Verb | Arguments | Returns | Runs |
//! |------|-----------|---------|------|
//! | `create_item` | `def: id<item>`, `count: int?`, `at: loc?`, `in: entity?` | the item | [`create`] |
//! | `take` | `who: entity`, `what: entity` | what is grasped | [`take`] |
//! | `drop` | `what: entity` | what lies there | [`drop`] |
//! | `give` | `what: entity`, `to: entity` | what `to` grasps | [`give`] |
//! | `wear` | `who: entity`, `what: entity` | what is worn | [`wear`] |
//! | `remove` | `what: entity` | what the wearer grasps | [`take_off`] |
//! | `stow` | `who: entity`, `what: entity` | what is stowed | [`stow`] |
//! | `put_in` | `what: entity`, `container: entity` | what is in it | [`put_in`] |
//! | `take_out` | `what: entity`, `to: entity?` | where it went | [`take_out`] |
//! | `split` | `item: entity`, `count: int` | the piece | [`split`] |
//! | `destroy_item` | `item: entity` | null | [`destroy`] |
//! | `set_color` | `of: entity`, `color: string?` | null | [`set_color`] |
//! | `repair_item` | `item: entity`, `share: real` | the item | [`repair`] |
//! | `materialise_contents` | `container: any` (an item, a device or an element) | `list<entity>` made | [`materialise`], [`crate::furnish::open_element`] |

use clash_data::Def;
use clash_expr::{Registry, Value, VerbSpec};

use super::{
    ItemDef, ItemId, create, destroy, drop, give, materialise, put_in, reason, refuse, repair,
    set_color, split, stow, take, take_off, take_out, wear,
};
use crate::Sim;
use crate::entity::Place;
use crate::event::Subject;
use crate::reason::Reason;
use crate::rule::VerbCall;
use crate::verb::VerbDef;

#[cfg(doc)]
use super::ITEM_REASONS;

/// The refusal for an argument that is missing or of the wrong kind.
fn bad_argument(name: &str, wanted: &str) -> Reason {
    refuse(
        reason::BAD_ARGUMENT,
        format!("the argument `{name}` must be {wanted}"),
    )
    .with("arg", name)
}

/// The value of an argument, by its position in the specification.
fn arg<'a>(call: &'a VerbCall<'_>, index: usize) -> &'a Value {
    call.args.get(index).unwrap_or(&Value::Null)
}

/// An argument that names an entity.
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

/// An optional argument that names an entity: `None` for null.
fn entity_or_null(
    sim: &Sim,
    call: &VerbCall<'_>,
    index: usize,
    name: &str,
) -> Result<Option<clash_core::EntityId>, Reason> {
    match arg(call, index) {
        Value::Null => Ok(None),
        _ => entity(sim, call, index, name).map(Some),
    }
}

/// A count of at least one; `default` for null.
fn count(call: &VerbCall<'_>, index: usize, name: &str, default: u32) -> Result<u32, Reason> {
    match arg(call, index) {
        Value::Null => Ok(default),
        value => value
            .as_int()
            .and_then(|count| u32::try_from(count).ok())
            .filter(|count| *count > 0)
            .ok_or_else(|| bad_argument(name, "a whole number of at least one")),
    }
}

/// An argument that names an `item` definition: an id, or its
/// qualified id as text.
fn item_def(sim: &Sim, call: &VerbCall<'_>, index: usize) -> Result<ItemId, Reason> {
    let wanted = || bad_argument("def", "an `item` definition");
    let ty = sim.exprs.registry().def_type_id(ItemDef::TYPE);
    match arg(call, index) {
        Value::Id(id) if Some(id.ty) == ty => {
            ItemId::from_index(id.index as usize).ok_or_else(wanted)
        }
        Value::Str(name) => sim
            .defs
            .try_registry::<ItemDef>()
            .and_then(|registry| registry.lookup(name))
            .ok_or_else(wanted),
        _ => Err(wanted()),
    }
}

/// An entity as the value a verb returns.
fn returned(sim: &Sim, id: clash_core::EntityId) -> Value {
    sim.subject_value(Subject::Entity(id))
}

/// The specification of a verb, if the kinds and types it names are
/// registered.
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

const WHO: (&str, &str, bool, &str) = (
    "who",
    "entity",
    false,
    "The creature that holds or wears it: it needs a body.",
);

const WHAT: (&str, &str, bool, &str) = ("what", "entity", false, "The thing, wherever it is.");

/// The verbs of items and holding, for [`crate::verb::engine_verbs`].
pub fn verbs() -> Vec<VerbDef> {
    vec![
        VerbDef {
            name: "create_item",
            build: |registry| {
                spec(
                    registry,
                    "create_item",
                    "Makes `count` items of a definition as one entity (06 §9.1): in a container with `in`, at a cell with `at`, and otherwise out of the world. More than one needs a definition that stacks. Put `in` a container, it is merely inside it: `put_in` stows it. Returns the item. Refused with `item.unknown_def`, `item.bad_count`, `item.not_stackable` and what the kernel refuses of the place.",
                    &[
                        ("def", "id<item>", false, "What to make."),
                        ("count", "int", true, "How many, as one stack. Default 1."),
                        ("at", "loc", true, "The cell to put it in."),
                        ("in", "entity", true, "The entity to put it inside. Wins over `at`."),
                    ],
                )
                .map(|spec| spec.direct("def"))
            },
            run: |sim, call, cause| {
                let def = item_def(sim, call, 0)?;
                let count = count(call, 1, "count", 1)?;
                let place = match (arg(call, 2), entity_or_null(sim, call, 3, "in")?) {
                    (_, Some(container)) => Place::In(container),
                    (Value::Null, None) => Place::Stored,
                    (at, None) => match sim.subject_of(at) {
                        Some(Subject::Loc(loc)) => Place::at(loc),
                        _ => return Err(bad_argument("at", "a cell")),
                    },
                };
                let id = create(sim, def, count, place, cause)?;
                Ok(returned(sim, id))
            },
        },
        VerbDef {
            name: "take",
            build: |registry| {
                spec(
                    registry,
                    "take",
                    "Takes a thing into a grasp of `who` (06 §9.2): picks it up, draws it from a pack, takes it from another's hand, takes it off if `who` wears it. It occupies the slots its `grip` names on the working parts of `who`'s body. Raises `item_leaving` and `item_taking`, then `item_taken`. Returns what is grasped: the thing, or an equal stack in `who`'s grasp that it joined. Refused with `item.no_grip`, `item.no_body`, `item.no_free_grasp`, what the kernel refuses, and by a handler of either hook.",
                    &[WHO, WHAT],
                )
            },
            run: |sim, call, cause| {
                let who = entity(sim, call, 0, "who")?;
                let what = entity(sim, call, 1, "what")?;
                let id = take(sim, who, what, cause)?;
                Ok(returned(sim, id))
            },
        },
        VerbDef {
            name: "drop",
            build: |registry| {
                spec(
                    registry,
                    "drop",
                    "Puts a held thing down where its holder is (06 §9.2): in the holder's cell, or inside whatever encloses the holder. Raises `item_leaving`, then `item_removed` if it was worn, then `item_dropped`. Returns what lies there: the thing, or an equal stack it joined. Refused with `item.not_held` and by a handler of `item_leaving`.",
                    &[WHAT],
                )
                .map(|spec| spec.direct("what"))
            },
            run: |sim, call, cause| {
                let what = entity(sim, call, 0, "what")?;
                let id = drop(sim, what, cause)?;
                Ok(returned(sim, id))
            },
        },
        VerbDef {
            name: "give",
            build: |registry| {
                spec(
                    registry,
                    "give",
                    "Hands a held thing to another's grasp (06 §9.2): `take` by `to`. Returns what `to` grasps. Refused with `item.not_held` for a thing inside nothing, and as `take` is.",
                    &[
                        WHAT,
                        ("to", "entity", false, "Who takes it: it needs a body."),
                    ],
                )
            },
            run: |sim, call, cause| {
                let what = entity(sim, call, 0, "what")?;
                let to = entity(sim, call, 1, "to")?;
                let id = give(sim, what, to, cause)?;
                Ok(returned(sim, id))
            },
        },
        VerbDef {
            name: "wear",
            build: |registry| {
                spec(
                    registry,
                    "wear",
                    "Puts an item on `who` (06 §9.2): it covers the outer parts its `wear` selects, at its layers, and is a layer for hits on them. One member of a stack is split off and worn. Raises `item_leaving` and `item_taking`, then `item_worn`. Returns what is worn. Refused with `item.not_wearable`, `item.no_body`, `item.no_part_to_wear_on`, `item.layer_taken`, what the kernel refuses, and by a handler of either hook.",
                    &[WHO, WHAT],
                )
            },
            run: |sim, call, cause| {
                let who = entity(sim, call, 0, "who")?;
                let what = entity(sim, call, 1, "what")?;
                let id = wear(sim, who, what, cause)?;
                Ok(returned(sim, id))
            },
        },
        VerbDef {
            name: "remove",
            build: |registry| {
                spec(
                    registry,
                    "remove",
                    "Takes a worn item off into its wearer's grasp (06 §9.2): `take` by the wearer. To take it off and put it down, `drop` it. Raises `item_removed`, then `item_taken`. Returns what the wearer grasps. Refused with `item.not_worn`, and as `take` is.",
                    &[("what", "entity", false, "The worn item.")],
                )
                .map(|spec| spec.direct("what"))
            },
            run: |sim, call, cause| {
                let what = entity(sim, call, 0, "what")?;
                let id = take_off(sim, what, cause)?;
                Ok(returned(sim, id))
            },
        },
        VerbDef {
            name: "stow",
            build: |registry| {
                spec(
                    registry,
                    "stow",
                    "Puts a thing into the first container `who` wears or grasps that would take it, in order of id (06 §9.2): `put_in` that container. Returns what is stowed. Refused with `item.no_container` if none would.",
                    &[("who", "entity", false, "Whose containers."), WHAT],
                )
            },
            run: |sim, call, cause| {
                let who = entity(sim, call, 0, "who")?;
                let what = entity(sim, call, 1, "what")?;
                let id = stow(sim, who, what, cause)?;
                Ok(returned(sim, id))
            },
        },
        VerbDef {
            name: "put_in",
            build: |registry| {
                spec(
                    registry,
                    "put_in",
                    "Puts a thing into a container (06 §9.2): an item is stowed in its capacity, anything else is enclosed. A container whose contents have not been made is opened first. Raises `item_leaving` and `item_taking`, then `item_taken`. Returns what is in it: the thing, or an equal stack there that it joined. Refused with `item.not_a_container`, `item.not_accepted`, `item.does_not_fit`, what the kernel refuses, and by a handler of either hook.",
                    &[
                        WHAT,
                        (
                            "container",
                            "entity",
                            false,
                            "The container: an item with `holds`.",
                        ),
                    ],
                )
            },
            run: |sim, call, cause| {
                let what = entity(sim, call, 0, "what")?;
                let container = entity(sim, call, 1, "container")?;
                let id = put_in(sim, what, container, cause)?;
                Ok(returned(sim, id))
            },
        },
        VerbDef {
            name: "take_out",
            build: |registry| {
                spec(
                    registry,
                    "take_out",
                    "Takes a thing out of the container it is stowed in or enclosed by (06 §9.2): into the grasp of `to` as `take` does, or with no `to` to where the container is, as `drop` does. Returns where it went. Refused with `item.not_contained`, and as `take` or `drop` is.",
                    &[
                        WHAT,
                        (
                            "to",
                            "entity",
                            true,
                            "Who takes it into a grasp. Null to put it where the container is.",
                        ),
                    ],
                )
            },
            run: |sim, call, cause| {
                let what = entity(sim, call, 0, "what")?;
                let to = entity_or_null(sim, call, 1, "to")?;
                let id = take_out(sim, what, to, cause)?;
                Ok(returned(sim, id))
            },
        },
        VerbDef {
            name: "split",
            build: |registry| {
                spec(
                    registry,
                    "split",
                    "Splits `count` members off a stack as an entity of their own, in the same place and hold (06 §9.1). Raises `item_split`. Returns the piece. Refused with `item.bad_count` unless at least one goes and one stays.",
                    &[
                        ("item", "entity", false, "The stack."),
                        ("count", "int", false, "How many to split off."),
                    ],
                )
            },
            run: |sim, call, cause| {
                let item = entity(sim, call, 0, "item")?;
                let count = count(call, 1, "count", 0)?;
                let piece = split(sim, item, count, cause)?;
                Ok(returned(sim, piece))
            },
        },
        VerbDef {
            name: "destroy_item",
            build: |registry| {
                spec(
                    registry,
                    "destroy_item",
                    "Unmakes an item, with every member of its stack. What it held is first put down where it was; contents never made are never made. Refused with `item.not_an_item`.",
                    &[("item", "entity", false, "The item.")],
                )
                .map(|spec| spec.direct("item"))
            },
            run: |sim, call, cause| {
                let item = entity(sim, call, 0, "item")?;
                destroy(sim, item, cause)?;
                Ok(Value::Null)
            },
        },
        VerbDef {
            name: "set_color",
            build: |registry| {
                spec(
                    registry,
                    "set_color",
                    "Changes the colour an item carries (06 §9.1), for every member of its stack; null takes it away. Refused with `item.not_an_item`.",
                    &[
                        ("of", "entity", false, "The item."),
                        ("color", "string", true, "The colour's name. Null for none."),
                    ],
                )
            },
            run: |sim, call, _cause| {
                let of = entity(sim, call, 0, "of")?;
                let color = match arg(call, 1) {
                    Value::Null => None,
                    value => Some(
                        value
                            .as_str()
                            .ok_or_else(|| bad_argument("color", "a colour's name"))?
                            .to_owned(),
                    ),
                };
                set_color(sim, of, color.as_deref())?;
                Ok(Value::Null)
            },
        },
        VerbDef {
            name: "repair_item",
            build: |registry| {
                spec(
                    registry,
                    "repair_item",
                    "Takes a share of an item's whole capacity off its damage (06 §9.1): 1 mends it entirely. Raises `item_repaired`. An item that is equal again to a stack in the same hold rejoins it. Returns the entity that now holds it. Refused with `item.bad_amount` for a share that is not above zero.",
                    &[
                        ("item", "entity", false, "The item."),
                        (
                            "share",
                            "real",
                            false,
                            "The share of its capacity to mend, above zero.",
                        ),
                    ],
                )
            },
            run: |sim, call, cause| {
                let item = entity(sim, call, 0, "item")?;
                let share = arg(call, 1)
                    .as_real()
                    .ok_or_else(|| bad_argument("share", "a number"))?;
                let id = repair(sim, item, share, cause)?;
                Ok(returned(sim, id))
            },
        },
        VerbDef {
            name: "materialise_contents",
            build: |registry| {
                spec(
                    registry,
                    "materialise_contents",
                    "Opens a container whose contents have not been made (06 §9.3): makes them from its definition's `contents`, drawn from its own seed, and stows them; for an element whose contents generation left latent, promotes it to its device, which makes them inside it. Raises `contents_materialised`. Returns what was made, or for an element what its device holds: nothing for a container opened before. Refused with `item.not_an_item` for an entity that is not an item or a device, and as `promote` refuses for an element.",
                    &[("container", "any", false, "The container: an item, a device, or an element.")],
                )
                .map(|spec| spec.direct("container"))
            },
            run: |sim, call, cause| {
                let made = match sim.subject_of(arg(call, 0)) {
                    Some(Subject::Element(loc, layer)) => {
                        crate::furnish::open_element(sim, loc, layer, cause)?
                    }
                    Some(Subject::Entity(id)) if crate::device::def_of(sim, id).is_some() => {
                        Vec::new()
                    }
                    _ => {
                        let container = entity(sim, call, 0, "container")?;
                        materialise(sim, container, cause)?
                    }
                };
                Ok(Value::list(made.into_iter().map(|id| returned(sim, id))))
            },
        },
    ]
}
