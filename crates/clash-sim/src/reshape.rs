//! Editing bodies in the simulation (06 §6.5): the verbs that add,
//! replace, graft, re-material and resize parts, rebuild a body from
//! another plan, and damage a part directly.
//!
//! The edits themselves are the body model's ([`body::edit`](crate::body::edit)), which
//! knows nothing of the simulation. Each function here does what the
//! simulation adds around one:
//!
//! 1. the hook `body_changing` runs and may cancel;
//! 2. the edit is made over the simulation's own stores
//!    (`Sim::with_body`), and refused with its reason if it cannot be;
//! 3. what the body model changed is raised as the events of 06 §7.6 and
//!    followed as every body change is (`Sim::raise_body_changes`):
//!    masses reach the world's records, the stats are worked out again,
//!    vital state is settled;
//! 4. what came off is reported as `part_detached` and leaves what its
//!    definition's `leaves` says (`item::part_item`); what the body
//!    grasped with parts that can no longer hold, and what it wore only
//!    over parts that are gone, is let go;
//! 5. `body_changed` is raised.
//!
//! A part is a subject of its own kind, which says whose body it is of,
//! so a verb that names a part takes no `who` (decision D-550). The
//! verbs `heal_part`, `disable_part`, `restore_part` and `remove_part`
//! are the body package's (`body::verbs`); [`remove_part`] here is the
//! edit, which also takes away the listings of what was gone already
//! beneath the part.

use clash_core::{EntityId, Real};
use clash_data::Def;
use clash_expr::{EventSpec, Handle, Registry, Type, Value, VerbSpec};
use clash_world::Cause;
use clash_world::defs::{MaterialDef, MaterialId};

use crate::Sim;
use crate::body::edit::{self, Attach, EditError};
use crate::body::props::{PART_KIND, part_handle, part_of_handle};
use crate::body::{
    Body, BodyCx, BodyPartDef, BodyPartId, BodyPlanDef, BodyPlanId, Changes, Detached, PartKey,
    Relation,
};
use crate::bus::{EventPayload, EventType, FieldType};
use crate::event::{CAUSE_SUBJECTS, EventBody, EventInfo, Subject, Where};
use crate::item;
use crate::life::{NullHost, args};
use crate::reason::{Reason, ReasonInfo};
use crate::rule::VerbCall;
use crate::verb::VerbDef;

/// The hook raised before a body is edited.
pub const BODY_CHANGING: &str = "body_changing";
/// Raised once a body has been edited.
pub const BODY_CHANGED: &str = "body_changed";

/// The refusals of editing beyond those of bodies (06 §7.6), for the
/// generated reference.
pub const EDIT_REASONS: &[ReasonInfo] = &[
    ReasonInfo {
        code: edit::reason::UNKNOWN_PART,
        doc: "The id is not a `body_part` definition. Values: none.",
    },
    ReasonInfo {
        code: edit::reason::NO_SUCH_SOCKET,
        doc: "The part a new part is to go on has no socket of that name. Values: `entity`, `part`, `socket`.",
    },
    ReasonInfo {
        code: edit::reason::SOCKET_REFUSES,
        doc: "The socket does not take that kind of part: its `accepts` does not list it. Values: `entity`, `part`, `socket`.",
    },
    ReasonInfo {
        code: edit::reason::NO_FREE_SOCKET,
        doc: "The socket is full, or no socket of the part would take the new one. Values: `entity`, `part`.",
    },
    ReasonInfo {
        code: edit::reason::NAME_TAKEN,
        doc: "A part that is there already has the name. Values: `entity`, `name`.",
    },
    ReasonInfo {
        code: edit::reason::TOO_MANY_PARTS,
        doc: "The body would have more than 1024 parts. Values: none.",
    },
    ReasonInfo {
        code: edit::reason::NO_SUCH_LAYER,
        doc: "The part has no layer of that number; the outermost is 0. Values: `entity`, `part`, `layer`.",
    },
    ReasonInfo {
        code: edit::reason::UNKNOWN_MATERIAL,
        doc: "The id is not a material. Values: none.",
    },
    ReasonInfo {
        code: edit::reason::NOT_A_PART,
        doc: "What was to be grafted is not a part that came off a body. Values: `item`, when an item was given.",
    },
];

/// The notification of editing, entered beside the kernel's own in every
/// simulation's event bus and in the engine's registry.
pub const EDIT_EVENTS: &[EventInfo] = &[EventInfo {
    name: BODY_CHANGED,
    subjects: &[("self", "entity")],
    fields: &[
        (
            "verb",
            "string",
            "Which edit: `add_part`, `remove_part`, `replace_part`, `graft_part`, `set_layer`, `scale_part`, `scale_body`, `set_body_plan` or `damage_part`.",
        ),
        (
            "part",
            "string",
            "The name of the part edited, or of the part a new one went on; empty for an edit of the whole body.",
        ),
        (
            "parts",
            "int",
            "How many parts the body has now that are there.",
        ),
    ],
    doc: "A body was edited (06 §6.5): its functions, channel slots, mass and attributes have been worked out again, and the events of what changed raised.",
}];

/// The hook of editing (05 §8.2): entered in the event bus of every
/// simulation and in the engine's registry.
pub fn hook_types() -> Vec<EventType> {
    vec![
        EventType::new(BODY_CHANGING)
            .subject("self", "entity")
            .field("verb", FieldType::Str, "Which edit, as `body_changed` names it.")
            .field(
                "part",
                FieldType::Str,
                "The name of the part to be edited, or of the part a new one is to go on; empty for an edit of the whole body.",
            )
            .hook(Vec::<String>::new())
            .doc("Runs before a body is edited by a verb of 06 §6.5 that reshapes it. A handler may cancel, with a reason: a body that cannot be reshaped."),
    ]
}

/// Enters in a registry what the verbs of editing name and their hook:
/// the definition types `body_part` and `body_plan`, unless they are
/// there already. Called while the engine's registry is built, before
/// its verbs.
pub fn register_types(registry: &mut Registry) {
    for (name, doc) in [
        (BodyPartDef::TYPE, BodyPartDef::DOC),
        (BodyPlanDef::TYPE, BodyPlanDef::DOC),
    ] {
        if registry.def_type_id(name).is_none() {
            let _ = registry.add_def_type(name, doc);
        }
    }
    for hook in hook_types() {
        if registry.event_id(&hook.name).is_some() {
            continue;
        }
        let mut spec = EventSpec::new(hook.name.clone()).doc(hook.doc.clone());
        let subjects = hook
            .subjects
            .iter()
            .map(|(name, kind)| (name.as_str(), kind.as_str()));
        for (name, kind) in subjects.chain(CAUSE_SUBJECTS.iter().copied()) {
            if let Some(kind) = registry.kind_id(kind) {
                spec = spec.subject(name.trim_end_matches('?'), kind);
            }
        }
        for field in &hook.fields {
            spec = spec.field(field.name.clone(), Type::Str, field.doc.clone());
        }
        let _ = registry.add_event(spec.hook(hook.mutable.iter().cloned()));
    }
}

// ---- Reasons -----------------------------------------------------------------

fn edit_reason(error: &EditError) -> Reason {
    let mut reason = Reason::new(error.code(), error.to_string());
    for (name, value) in error.values() {
        reason = reason.with(name, plain(&value));
    }
    reason
}

/// A value of the expression language as a reason carries it.
fn plain(value: &Value) -> clash_core::Value {
    match value {
        Value::Int(int) => clash_core::Value::Int(*int),
        Value::Real(real) => clash_core::Value::Real(*real),
        Value::Bool(truth) => clash_core::Value::Bool(*truth),
        Value::Str(text) => clash_core::Value::Str(text.to_string()),
        other => clash_core::Value::Str(other.to_string()),
    }
}

fn bad_argument(name: &str, wanted: &str) -> Reason {
    Reason::new(
        args::BAD_ARGUMENT,
        format!("the argument `{name}` must be {wanted}"),
    )
    .with("argument", name)
}

// ---- Running an edit ---------------------------------------------------------

/// What an edit gave back besides its changes.
struct Edited<T> {
    value: T,
    changes: Changes,
    detached: Vec<Detached>,
}

fn edited<T>((value, changes): (T, Changes)) -> Edited<T> {
    Edited {
        value,
        changes,
        detached: Vec::new(),
    }
}

fn just(changes: Changes) -> Edited<()> {
    Edited {
        value: (),
        changes,
        detached: Vec::new(),
    }
}

fn with_off((detached, changes): (Vec<Detached>, Changes)) -> Edited<()> {
    Edited {
        value: (),
        changes,
        detached,
    }
}

/// The body of an entity, or the refusal that it has none.
fn body_of(sim: &Sim, who: EntityId) -> Result<&Body, Reason> {
    sim.known(who)?;
    sim.components.get::<Body>(who).ok_or_else(|| {
        Reason::new(crate::body::reason::NO_BODY, format!("{who} has no body"))
            .with("entity", who.to_string())
    })
}

/// A part of an entity's body by its name, there or gone.
pub fn part_named(sim: &Sim, who: EntityId, name: &str) -> Result<PartKey, Reason> {
    body_of(sim, who)?
        .part_named(name)
        .map(crate::body::PartInst::key)
        .ok_or_else(|| {
            Reason::new(
                crate::body::reason::NO_SUCH_PART,
                format!("{who} has no part named `{name}`"),
            )
            .with("entity", who.to_string())
            .with("part", name)
        })
}

/// The name of a part, or the empty text.
fn name_of(sim: &Sim, who: EntityId, part: PartKey) -> String {
    sim.components
        .get::<Body>(who)
        .and_then(|body| body.part(part))
        .map_or_else(String::new, |part| part.name().to_owned())
}

/// Runs one edit with everything the simulation does around it, and
/// returns what the edit gave and the items left of what came off.
fn edit<T>(
    sim: &mut Sim,
    who: EntityId,
    verb: &str,
    part: &str,
    cause: Cause,
    run: impl FnOnce(&mut BodyCx<'_, NullHost>) -> Result<Edited<T>, EditError>,
) -> Result<(T, Vec<EntityId>), Reason> {
    body_of(sim, who)?;
    let asked = EventPayload::new(BODY_CHANGING)
        .subject("self", Subject::Entity(who))
        .with("verb", verb)
        .with("part", part);
    if let Some(reason) = sim.raise(Where::Entity(who), cause, asked)?.cancelled {
        return Err(reason);
    }
    let edited = sim
        .with_body(|cx, _| run(cx))?
        .map_err(|error| edit_reason(&error))?;
    sim.raise_body_changes(who, edited.changes.clone(), cause);
    let mut items = Vec::new();
    for detached in &edited.detached {
        sim.report_detached(detached.clone());
        match item::part_item(sim, detached, cause) {
            Ok(Some(item)) => items.push(item),
            Ok(None) => {}
            Err(reason) => sim.fault(reason),
        }
    }
    item::on_body_changes(sim, &edited.changes, cause);
    // What was worn only over parts that are no longer there is let go.
    let present: Vec<PartKey> = sim
        .components
        .get::<Body>(who)
        .map(|body| {
            body.parts()
                .iter()
                .filter(|part| part.is_present())
                .map(crate::body::PartInst::key)
                .collect()
        })
        .unwrap_or_default();
    item::hold::uncover(sim, who, &|part| !present.contains(&part), None, cause);
    sim.emit(
        Where::Entity(who),
        cause,
        EventBody::Custom(
            EventPayload::new(BODY_CHANGED)
                .subject("self", Subject::Entity(who))
                .with("verb", verb)
                .with("part", part)
                .with("parts", i64::try_from(present.len()).unwrap_or(i64::MAX)),
        ),
    );
    Ok((edited.value, items))
}

// ---- The edits ---------------------------------------------------------------

/// Adds a new part (06 §6.5 `add_part`): see [`edit::add_part`]. Returns
/// its key.
#[allow(clippy::too_many_arguments)]
pub fn add_part(
    sim: &mut Sim,
    who: EntityId,
    def: BodyPartId,
    at: &Attach,
    name: Option<&str>,
    side: Option<String>,
    scale: Real,
    cause: Cause,
) -> Result<PartKey, Reason> {
    let parent = name_of(sim, who, at.parent);
    let (key, _) = edit(sim, who, "add_part", &parent, cause, |cx| {
        edit::add_part(cx, who, def, at, name, side, scale).map(edited)
    })?;
    Ok(key)
}

/// Takes a part off with everything on it and in it, and the listings
/// of what was gone already beneath it (06 §6.5 `remove_part`): see
/// [`edit::remove_part`]. What came off leaves what its definition
/// says; the item, if any, is returned.
pub fn remove_part(
    sim: &mut Sim,
    who: EntityId,
    part: PartKey,
    cause: Cause,
) -> Result<Option<EntityId>, Reason> {
    let name = name_of(sim, who, part);
    let ((), items) = edit(sim, who, "remove_part", &name, cause, |cx| {
        let (detached, changes) = edit::remove_part(cx, who, part, cause)?;
        Ok(with_off((vec![detached], changes)))
    })?;
    Ok(items.into_iter().next())
}

/// Swaps a part's definition (06 §6.5 `replace_part`): see
/// [`edit::replace_part`]. Returns the items left of what came off
/// because it no longer fits.
pub fn replace_part(
    sim: &mut Sim,
    who: EntityId,
    part: PartKey,
    with: BodyPartId,
    cause: Cause,
) -> Result<Vec<EntityId>, Reason> {
    let name = name_of(sim, who, part);
    let ((), items) = edit(sim, who, "replace_part", &name, cause, |cx| {
        edit::replace_part(cx, who, part, with, cause).map(with_off)
    })?;
    Ok(items)
}

/// Attaches a part that came off a body (06 §6.5 `graft_part`): the item
/// a part left (`item::part_item`), from this body or another. The item
/// is used up. Returns the key of the part grafted.
pub fn graft_part(
    sim: &mut Sim,
    who: EntityId,
    part: EntityId,
    at: &Attach,
    cause: Cause,
) -> Result<PartKey, Reason> {
    sim.known(part)?;
    let remains = sim
        .components
        .get::<item::Item>(part)
        .and_then(item::Item::remains)
        .cloned()
        .ok_or_else(|| edit_reason(&EditError::NotAPart).with("item", part.to_string()))?;
    let parent = name_of(sim, who, at.parent);
    let (key, _) = edit(sim, who, "graft_part", &parent, cause, |cx| {
        edit::graft_part(cx, who, &remains, at).map(edited)
    })?;
    item::destroy(sim, part, cause)?;
    Ok(key)
}

/// Changes a layer's material or thickness on some parts of one body
/// (06 §6.5 `set_layer`): see [`edit::set_layer`].
pub fn set_layer(
    sim: &mut Sim,
    who: EntityId,
    parts: &[PartKey],
    layer: Option<usize>,
    material: Option<MaterialId>,
    thickness: Option<Real>,
    cause: Cause,
) -> Result<(), Reason> {
    let name = match parts {
        [one] => name_of(sim, who, *one),
        _ => String::new(),
    };
    edit(sim, who, "set_layer", &name, cause, |cx| {
        edit::set_layer(cx, who, parts, layer, material, thickness).map(just)
    })
    .map(|_| ())
}

/// Gives a part a new size relative to its body (06 §6.5 `scale_part`).
pub fn scale_part(
    sim: &mut Sim,
    who: EntityId,
    part: PartKey,
    scale: Real,
    cause: Cause,
) -> Result<(), Reason> {
    let name = name_of(sim, who, part);
    edit(sim, who, "scale_part", &name, cause, |cx| {
        edit::scale_part(cx, who, part, scale).map(just)
    })
    .map(|_| ())
}

/// Gives a body a new overall size (06 §6.5 `scale_body`).
pub fn scale_body(sim: &mut Sim, who: EntityId, scale: Real, cause: Cause) -> Result<(), Reason> {
    edit(sim, who, "scale_body", "", cause, |cx| {
        edit::scale_body(cx, who, scale).map(just)
    })
    .map(|_| ())
}

/// Rebuilds a body from another plan (06 §6.5 `set_body_plan`): see
/// [`edit::set_body_plan`]. Returns the items left of what the new plan
/// does not map.
pub fn set_body_plan(
    sim: &mut Sim,
    who: EntityId,
    plan: BodyPlanId,
    cause: Cause,
) -> Result<Vec<EntityId>, Reason> {
    let ((), items) = edit(sim, who, "set_body_plan", "", cause, |cx| {
        edit::set_body_plan(cx, who, plan, cause).map(with_off)
    })?;
    Ok(items)
}

/// Adds a share of a whole layer to the damage of one layer of a part or
/// of each (06 §6.5 `damage_part`): see [`edit::damage_part`].
pub fn damage_part(
    sim: &mut Sim,
    who: EntityId,
    part: PartKey,
    layer: Option<usize>,
    share: Real,
    cause: Cause,
) -> Result<(), Reason> {
    let name = name_of(sim, who, part);
    edit(sim, who, "damage_part", &name, cause, |cx| {
        edit::damage_part(cx, who, part, layer, share).map(just)
    })
    .map(|_| ())
}

// ---- The verbs ---------------------------------------------------------------

/// The part an argument names, and whose body it is of.
fn part_arg(
    sim: &Sim,
    call: &VerbCall<'_>,
    at: usize,
    name: &str,
) -> Result<(EntityId, PartKey), Reason> {
    let handle: Handle =
        args::optional_handle(call, at, name)?.ok_or_else(|| bad_argument(name, "a part"))?;
    let kind = sim.body_defs().map(|defs| defs.kinds().part);
    if Some(handle.kind) != kind {
        return Err(bad_argument(name, "a part of a body"));
    }
    part_of_handle(&sim.components().body, handle)
        .ok_or_else(|| bad_argument(name, "a part of a body that still exists"))
}

/// A part as a verb gives it back.
fn part_value(sim: &Sim, who: EntityId, part: PartKey) -> Value {
    sim.body_defs().map_or(Value::Null, |defs| {
        Value::Subject(part_handle(defs.kinds(), who, part))
    })
}

fn layer_arg(call: &VerbCall<'_>, at: usize) -> Result<Option<usize>, Reason> {
    match args::value(call, at) {
        Value::Null => Ok(None),
        value => value
            .as_int()
            .and_then(|layer| usize::try_from(layer).ok())
            .map(Some)
            .ok_or_else(|| bad_argument("layer", "a layer's number, 0 for the outermost")),
    }
}

/// Where a verb puts a part: on the part `on`, in `socket`, or inside
/// it with `inside`.
fn attach_arg(
    sim: &Sim,
    call: &VerbCall<'_>,
    on: usize,
    socket: usize,
    inside: usize,
) -> Result<(EntityId, Attach), Reason> {
    let (who, parent) = part_arg(sim, call, on, "on")?;
    let inside = args::flag(call, inside, "inside")?;
    Ok((
        who,
        Attach {
            parent,
            relation: if inside { Relation::In } else { Relation::On },
            socket: args::optional_text(call, socket, "socket")?.map(str::to_owned),
        },
    ))
}

/// A verb's specification, from its arguments as the language writes
/// their types; `part` stands for the kind of a body's part.
fn spec(
    registry: &Registry,
    name: &str,
    doc: &str,
    arguments: &[(&str, &str, bool, &str)],
) -> Option<VerbSpec> {
    let mut spec = VerbSpec::new(name).doc(doc);
    for (arg, written, optional, arg_doc) in arguments {
        let ty = if *written == PART_KIND {
            // `self.part(name)` may be null: a null part is refused when
            // the verb runs rather than warned of where it is written.
            Type::Subject(registry.kind_id(PART_KIND)?).nullable()
        } else {
            registry.parse_type(written)?
        };
        spec = if *optional {
            spec.optional_arg(*arg, ty.nullable(), *arg_doc)
        } else {
            spec.arg(*arg, ty, *arg_doc)
        };
    }
    Some(spec)
}

const PART: (&str, &str, bool, &str) = ("part", PART_KIND, false, "The part.");

/// The verbs of body editing, for [`crate::verb::engine_verbs`]. Each
/// raises `body_changing` first, which may cancel, and `body_changed`
/// after.
#[allow(clippy::too_many_lines)]
pub fn verbs() -> Vec<VerbDef> {
    vec![
        VerbDef {
            name: "add_part",
            build: |registry| {
                spec(
                    registry,
                    "add_part",
                    "Adds a new part of a definition to a body (06 §6.5), on the part `on` in a socket that takes it, or with `inside` inside it, with the organs its definition brings. A part of that name that is gone is replaced. Gives the new part. Refused with `unknown_part`, `no_such_socket`, `socket_refuses`, `no_free_socket`, `part_name_taken`, `too_many_parts`, `bad_amount` and by a handler of `body_changing`.",
                    &[
                        ("on", PART_KIND, false, "The part it is joined to."),
                        ("part", "id<body_part>", false, "What kind of part."),
                        (
                            "socket",
                            "string",
                            true,
                            "The socket it takes. Left out, the first, by name, that would take it.",
                        ),
                        (
                            "inside",
                            "bool",
                            true,
                            "Whether it goes inside the part rather than on it.",
                        ),
                        (
                            "name",
                            "string",
                            true,
                            "Its name in the body. Left out, its definition's id, numbered if that is taken.",
                        ),
                        ("side", "string", true, "Its side label."),
                        (
                            "scale",
                            "real",
                            true,
                            "Its size relative to the body, above zero. Default 1.",
                        ),
                    ],
                )
            },
            run: |sim, call, cause| {
                let (who, at) = attach_arg(sim, call, 0, 2, 3)?;
                let def =
                    BodyPartId::from_index(args::def(sim, call, 1, "part", BodyPartDef::TYPE)?)
                        .ok_or_else(|| bad_argument("part", "a `body_part`"))?;
                let name = args::optional_text(call, 4, "name")?;
                let side = args::optional_text(call, 5, "side")?.map(str::to_owned);
                let scale = args::optional_real(call, 6, "scale")?.unwrap_or(Real::ONE);
                let key = add_part(sim, who, def, &at, name, side, scale, cause)?;
                Ok(part_value(sim, who, key))
            },
        },
        VerbDef {
            name: "replace_part",
            build: |registry| {
                spec(
                    registry,
                    "replace_part",
                    "Swaps a part's definition (06 §6.5), keeping its name, its place and what is attached to it; its layers are new and its wounds gone. What is on it and no longer fits the new definition's sockets comes off, reported as `part_detached`. Gives null. Refused with `unknown_part`, `socket_refuses`, `part_missing` and by a handler of `body_changing`.",
                    &[
                        PART,
                        ("with", "id<body_part>", false, "The new definition."),
                    ],
                )
            },
            run: |sim, call, cause| {
                let (who, part) = part_arg(sim, call, 0, "part")?;
                let with =
                    BodyPartId::from_index(args::def(sim, call, 1, "with", BodyPartDef::TYPE)?)
                        .ok_or_else(|| bad_argument("with", "a `body_part`"))?;
                replace_part(sim, who, part, with, cause)?;
                Ok(Value::Null)
            },
        },
        VerbDef {
            name: "graft_part",
            build: |registry| {
                spec(
                    registry,
                    "graft_part",
                    "Attaches a part that came off a body, from this body or another (06 §6.5): the item it left. Its parts keep their names, layers, damage and size, not their wounds; the item is used up. Gives the grafted part. Refused with `not_a_part` and as `add_part` is.",
                    &[
                        ("item", "entity", false, "The item a part left."),
                        ("on", PART_KIND, false, "The part it is joined to."),
                        (
                            "socket",
                            "string",
                            true,
                            "The socket it takes. Left out, the first, by name, that would take it.",
                        ),
                        (
                            "inside",
                            "bool",
                            true,
                            "Whether it goes inside the part rather than on it.",
                        ),
                    ],
                )
            },
            run: |sim, call, cause| {
                let part = args::entity(sim, call, 0, "item")?;
                let (who, at) = attach_arg(sim, call, 1, 2, 3)?;
                let key = graft_part(sim, who, part, &at, cause)?;
                Ok(part_value(sim, who, key))
            },
        },
        VerbDef {
            name: "set_layer",
            build: |registry| {
                spec(
                    registry,
                    "set_layer",
                    "Changes what a layer of a part is made of, how thick it is, or both (06 §6.5). A layer keeps its damage. Gives null. Refused with `no_such_layer`, `unknown_material`, `part_missing`, `bad_amount` and by a handler of `body_changing`.",
                    &[
                        PART,
                        (
                            "layer",
                            "int",
                            true,
                            "The layer, 0 for the outermost. Left out, every layer.",
                        ),
                        (
                            "material",
                            "id<material>",
                            true,
                            "What it is to be made of. Left out, as it is.",
                        ),
                        (
                            "thickness",
                            "real",
                            true,
                            "How thick it is to be, in metres, above zero. Left out, as it is.",
                        ),
                    ],
                )
            },
            run: |sim, call, cause| {
                let (who, part) = part_arg(sim, call, 0, "part")?;
                let layer = layer_arg(call, 1)?;
                let material = args::optional_def(sim, call, 2, "material", MaterialDef::TYPE)?
                    .map(|index| {
                        MaterialId::from_index(index)
                            .ok_or_else(|| bad_argument("material", "a `material`"))
                    })
                    .transpose()?;
                let thickness = args::optional_real(call, 3, "thickness")?;
                set_layer(sim, who, &[part], layer, material, thickness, cause)?;
                Ok(Value::Null)
            },
        },
        VerbDef {
            name: "scale_part",
            build: |registry| {
                spec(
                    registry,
                    "scale_part",
                    "Gives a part a new size relative to its body (06 §6.5): its length, girth and layers go with it, its mass with the cube. Gives null. Refused with `part_missing`, `bad_amount` and by a handler of `body_changing`.",
                    &[
                        PART,
                        (
                            "scale",
                            "real",
                            false,
                            "Its size relative to the body, above zero.",
                        ),
                    ],
                )
            },
            run: |sim, call, cause| {
                let (who, part) = part_arg(sim, call, 0, "part")?;
                let scale = args::real(call, 1, "scale")?;
                scale_part(sim, who, part, scale, cause)?;
                Ok(Value::Null)
            },
        },
        VerbDef {
            name: "scale_body",
            build: |registry| {
                spec(
                    registry,
                    "scale_body",
                    "Gives a body a new overall size (06 §6.5): every part, its height and its mass go with it. Gives null. Refused with `no_body`, `bad_amount` and by a handler of `body_changing`.",
                    &[
                        ("who", "entity", false, "Whose body."),
                        (
                            "scale",
                            "real",
                            false,
                            "Its overall size factor, above zero.",
                        ),
                    ],
                )
            },
            run: |sim, call, cause| {
                let who = args::entity(sim, call, 0, "who")?;
                let scale = args::real(call, 1, "scale")?;
                scale_body(sim, who, scale, cause)?;
                Ok(Value::Null)
            },
        },
        VerbDef {
            name: "set_body_plan",
            build: |registry| {
                spec(
                    registry,
                    "set_body_plan",
                    "Rebuilds a body from another plan (06 §6.5), keeping each part the new plan names with the same definition, damage and wounds included; what the plan does not map comes off, reported as `part_detached`. Gives null. Refused with `unknown_plan`, `too_many_parts`, `no_body` and by a handler of `body_changing`.",
                    &[
                        ("who", "entity", false, "Whose body."),
                        ("plan", "id<body_plan>", false, "The plan."),
                    ],
                )
            },
            run: |sim, call, cause| {
                let who = args::entity(sim, call, 0, "who")?;
                let plan =
                    BodyPlanId::from_index(args::def(sim, call, 1, "plan", BodyPlanDef::TYPE)?)
                        .ok_or_else(|| bad_argument("plan", "a `body_plan`"))?;
                set_body_plan(sim, who, plan, cause)?;
                Ok(Value::Null)
            },
        },
        VerbDef {
            name: "damage_part",
            build: |registry| {
                spec(
                    registry,
                    "damage_part",
                    "Adds a share of a whole layer to the damage of one layer of a part, or of each (06 §6.5). No wound is made, and no layer is taken past 254 255ths. Gives null. Refused with `part_missing`, `no_such_layer`, `bad_amount` and by a handler of `body_changing`.",
                    &[
                        PART,
                        (
                            "layer",
                            "int",
                            true,
                            "The layer, 0 for the outermost. Left out, every layer.",
                        ),
                        (
                            "share",
                            "real",
                            false,
                            "The share of a whole layer, above zero.",
                        ),
                    ],
                )
            },
            run: |sim, call, cause| {
                let (who, part) = part_arg(sim, call, 0, "part")?;
                let layer = layer_arg(call, 1)?;
                let share = args::real(call, 2, "share")?;
                damage_part(sim, who, part, layer, share, cause)?;
                Ok(Value::Null)
            },
        },
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_verb_event_and_hook_is_in_the_engine_registry() {
        // As loading leaves it: the kinds of bodies are entered by their
        // registry step, and the verbs that name them after.
        let mut registry = crate::engine_registry();
        crate::body::props::register(&mut registry).unwrap();
        crate::verb::register_engine_verbs(&mut registry);
        for verb in verbs() {
            let id = registry
                .verb_id(verb.name)
                .unwrap_or_else(|| panic!("`{}` is not registered", verb.name));
            assert!(
                !registry.verb(id).unwrap().spec.doc.is_empty(),
                "{}",
                verb.name
            );
        }
        for info in EDIT_EVENTS {
            assert!(registry.event_id(info.name).is_some(), "{}", info.name);
        }
        for hook in hook_types() {
            let id = registry.event_id(&hook.name).unwrap();
            assert!(registry.event(id).unwrap().spec.hook);
        }
    }

    #[test]
    fn every_reason_is_listed_once_and_documented() {
        let mut codes: Vec<&str> = EDIT_REASONS.iter().map(|info| info.code).collect();
        let count = codes.len();
        codes.sort_unstable();
        codes.dedup();
        assert_eq!(codes.len(), count);
        assert!(EDIT_REASONS.iter().all(|info| !info.doc.is_empty()));
    }
}
