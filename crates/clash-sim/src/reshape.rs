//! Editing bodies in the simulation (06 §6.5): the verbs that add,
//! remove, replace, graft, re-material and resize parts, rebuild a body
//! from another plan, and change a part's damage directly.
//!
//! The edits themselves are the body model's ([`body::edit`]), which
//! knows nothing of the simulation. Each function here does what the
//! simulation adds around one:
//!
//! 1. the hook `body_changing` runs and may cancel;
//! 2. the edit is made, and refused with its reason if it cannot be;
//! 3. what came off is made an item where the edit was asked to leave
//!    one (`item::part_item`), and what the body grasped with parts that
//!    can no longer hold, or wore on parts that are gone, is let go;
//! 4. the masses that changed are handed to the world's records, and the
//!    entity's attributes are worked out again (06 §4.1: an edit can
//!    change what a derived attribute reads);
//! 5. `body_changed` is raised.
//!
//! A part is named by its name in the body. `set_layer` takes a part as
//! a wear entry names one (06 §9.1): its name, the id of its definition,
//! a tag its definition carries, or `all`.
//!
//! # The seams
//!
//! The kernel does not yet hold the bodies' and the stats' tables, so
//! `with_body` builds them for the length of a call and takes the stores
//! and the timeline out of the simulation while it runs, as the items
//! package does for stats. The events the body model's changes are
//! reported by (`part_disabled`, `function_changed` and the rest of
//! 06 §7.6) are the kernel's to raise once it raises them: they reach
//! [`body_events`] and stop there.

use std::sync::Arc;

use clash_core::{EntityId, Real};
use clash_data::{Def, Runner};
use clash_expr::{EventSpec, Registry, Type, Value, VerbSpec};
use clash_time::Timeline;
use clash_world::Cause;
use clash_world::defs::{MaterialDef, MaterialId};

use crate::Sim;
use crate::body::edit::{self, Attach, EditError};
use crate::body::{
    self, Body, BodyCx, BodyDefs, BodyError, BodyPartDef, BodyPartId, BodyPlanDef, BodyPlanId,
    Change, Changes, Detached, PartKey, Relation,
};
use crate::bus::{EventPayload, EventType, FieldType};
use crate::event::{CAUSE_SUBJECTS, EventBody, EventInfo, Subject, Where};
use crate::host::{Bound, SimHost};
use crate::item;
use crate::reason::{Reason, ReasonInfo};
use crate::rule::VerbCall;
use crate::stats::{StatDefs, StatsCx};
use crate::verb::VerbDef;

/// The hook raised before a body is edited.
pub const BODY_CHANGING: &str = "body_changing";
/// Raised once a body has been edited.
pub const BODY_CHANGED: &str = "body_changed";

/// The code of the refusal of a verb's argument that is missing or of
/// the wrong kind.
pub const BAD_ARGUMENT: &str = "body.bad_argument";

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
    ReasonInfo {
        code: BAD_ARGUMENT,
        doc: "A verb of body editing was given an argument that is missing or of the wrong kind, or names no part of the body. Values: `arg`, the argument's name.",
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
            "Which edit: `add_part`, `remove_part`, `replace_part`, `graft_part`, `set_layer`, `scale_part`, `scale_body`, `set_body_plan`, `heal_part`, `damage_part`, `disable_part` or `restore_part`.",
        ),
        (
            "part",
            "string",
            "The name of the part edited; empty for an edit of the whole body.",
        ),
        (
            "parts",
            "int",
            "How many parts the body has now that are there.",
        ),
    ],
    doc: "A body was edited (06 §6.5): its functions, channel slots, mass and attributes have been worked out again.",
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
                "The name of the part to be edited; empty for an edit of the whole body.",
            )
            .hook(Vec::<String>::new())
            .doc("Runs before a body is edited by any verb of 06 §6.5. A handler may cancel, with a reason: a body that cannot be reshaped."),
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

fn body_reason(error: &BodyError) -> Reason {
    let mut reason = Reason::new(error.code(), error.to_string());
    for (name, value) in error.values() {
        reason = reason.with(name, plain(&value));
    }
    reason
}

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
        BAD_ARGUMENT,
        format!("the argument `{name}` must be {wanted}"),
    )
    .with("arg", name)
}

// ---- The seams ---------------------------------------------------------------

/// Runs a body function over the simulation's stores: see the
/// [module documentation](self). Refused if the definitions were loaded
/// without the bodies' or the stats' types.
fn with_body<R>(
    sim: &mut Sim,
    run: impl FnOnce(&mut BodyCx<'_, SimHost<'_>>) -> R,
) -> Result<R, Reason> {
    let world = Arc::clone(sim.cosmos.defs());
    let body_defs = BodyDefs::new(&sim.defs, &world).map_err(|error| body_reason(&error))?;
    let stat_defs =
        StatDefs::new(&sim.defs).map_err(|error| Reason::new(error.code(), error.to_string()))?;
    let mut bodies = core::mem::take(&mut sim.components.body);
    let mut physical = core::mem::take(&mut sim.components.physical);
    let mut stats = core::mem::take(&mut sim.components.stats);
    let mut timeline = core::mem::replace(&mut sim.timeline, Timeline::new());
    let mut runner = Runner::new();
    let result = {
        let mut host = sim.host(Bound::new());
        let mut cx = BodyCx {
            bodies: &mut bodies,
            defs: &body_defs,
            physical: &mut physical,
            containment: &sim.entities,
            stats: StatsCx {
                stats: &mut stats,
                defs: &stat_defs,
                timeline: &mut timeline,
                host: &mut host,
                runner: &mut runner,
            },
        };
        run(&mut cx)
    };
    sim.timeline = timeline;
    sim.components.stats = stats;
    sim.components.physical = physical;
    sim.components.body = bodies;
    Ok(result)
}

/// Receives what an edit changed of the body, for the kernel to raise
/// the events of 06 §7.6 from once it raises them; until then they end
/// here. What the simulation must do about them now (letting go, masses)
/// is done by the caller.
pub fn body_events(_sim: &mut Sim, _changes: &Changes) {}

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

/// The body of an entity, or the refusal that it has none.
fn body_of(sim: &Sim, who: EntityId) -> Result<&Body, Reason> {
    sim.known(who)?;
    sim.components
        .get::<Body>(who)
        .ok_or_else(|| body_reason(&BodyError::NoBody { entity: who }))
}

/// A part of an entity's body by its name, there or gone.
pub fn part_named(sim: &Sim, who: EntityId, name: &str) -> Result<PartKey, Reason> {
    body_of(sim, who)?
        .part_named(name)
        .map(body::PartInst::key)
        .ok_or_else(|| {
            Reason::new(
                body::reason::NO_SUCH_PART,
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

/// Runs one edit with everything the simulation does around it. `leave`
/// says whether what comes off is left as items, which are returned.
fn edit<T>(
    sim: &mut Sim,
    who: EntityId,
    verb: &str,
    part: &str,
    leave: bool,
    cause: Cause,
    run: impl FnOnce(&mut BodyCx<'_, SimHost<'_>>) -> Result<Edited<T>, EditError>,
) -> Result<(T, Vec<EntityId>), Reason> {
    body_of(sim, who)?;
    let asked = EventPayload::new(BODY_CHANGING)
        .subject("self", Subject::Entity(who))
        .with("verb", verb)
        .with("part", part);
    if let Some(reason) = sim.raise(Where::Entity(who), cause, asked)?.cancelled {
        return Err(reason);
    }
    let edited = with_body(sim, run)?.map_err(|error| edit_reason(&error))?;
    let mut items = Vec::new();
    for detached in &edited.detached {
        if leave && let Ok(Some(item)) = item::part_item(sim, detached, cause) {
            items.push(item);
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
                .map(body::PartInst::key)
                .collect()
        })
        .unwrap_or_default();
    item::hold::uncover(sim, who, &|part| !present.contains(&part), None, cause);
    for change in &edited.changes {
        if let Change::MassChanged(mass) = change
            && let Some(mut presence) = sim
                .entities
                .get(mass.entity)
                .map(|record| *record.presence())
        {
            presence.mass = mass.after.get();
            let _ = sim.set_presence(mass.entity, presence, cause);
        }
    }
    if sim.components.stats.contains(who) {
        let _ = crate::traits::with_stats(sim, |cx| crate::stats::refresh(cx, who));
    }
    body_events(sim, &edited.changes);
    let parts = sim.components.get::<Body>(who).map_or(0, |body| {
        body.parts().iter().filter(|part| part.is_present()).count()
    });
    sim.emit(
        Where::Entity(who),
        cause,
        EventBody::Custom(
            EventPayload::new(BODY_CHANGED)
                .subject("self", Subject::Entity(who))
                .with("verb", verb)
                .with("part", part)
                .with("parts", i64::try_from(parts).unwrap_or(i64::MAX)),
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
    let (key, _) = edit(sim, who, "add_part", &parent, false, cause, |cx| {
        edit::add_part(cx, who, def, at, name, side, scale).map(edited)
    })?;
    Ok(key)
}

/// Takes a part off with everything on it and in it (06 §6.5
/// `remove_part`): the body no longer has them. With `leave`, they are
/// left as an item where the body is, which is returned.
pub fn remove_part(
    sim: &mut Sim,
    who: EntityId,
    part: PartKey,
    leave: bool,
    cause: Cause,
) -> Result<Option<EntityId>, Reason> {
    let name = name_of(sim, who, part);
    let ((), items) = edit(sim, who, "remove_part", &name, leave, cause, |cx| {
        let (detached, changes) = edit::remove_part(cx, who, part, cause)?;
        Ok(Edited {
            value: (),
            changes,
            detached: vec![detached],
        })
    })?;
    Ok(items.into_iter().next())
}

/// Swaps a part's definition (06 §6.5 `replace_part`): see
/// [`edit::replace_part`]. With `leave`, what came off because it no
/// longer fits is left as items, which are returned.
pub fn replace_part(
    sim: &mut Sim,
    who: EntityId,
    part: PartKey,
    with: BodyPartId,
    leave: bool,
    cause: Cause,
) -> Result<Vec<EntityId>, Reason> {
    let name = name_of(sim, who, part);
    let ((), items) = edit(sim, who, "replace_part", &name, leave, cause, |cx| {
        let (detached, changes) = edit::replace_part(cx, who, part, with, cause)?;
        Ok(Edited {
            value: (),
            changes,
            detached,
        })
    })?;
    Ok(items)
}

/// Attaches a part that came off a body (06 §6.5 `graft_part`): an item
/// left by a part (`item::part_item`), from this body or another. The
/// item is used up. Returns the key of the part grafted.
pub fn graft_part(
    sim: &mut Sim,
    who: EntityId,
    part: EntityId,
    at: &Attach,
    cause: Cause,
) -> Result<PartKey, Reason> {
    let not_a_part = || edit_reason(&EditError::NotAPart).with("item", part.to_string());
    sim.known(part)?;
    let remains = sim
        .components
        .get::<item::Item>(part)
        .and_then(item::Item::remains)
        .cloned()
        .ok_or_else(not_a_part)?;
    let parent = name_of(sim, who, at.parent);
    let (key, _) = edit(sim, who, "graft_part", &parent, false, cause, |cx| {
        edit::graft_part(cx, who, &remains, at).map(edited)
    })?;
    item::destroy(sim, part, cause)?;
    Ok(key)
}

/// The parts that are there and answer to a name as a wear entry's
/// `parts` does (06 §9.1): their name, their definition's id (qualified
/// or bare) or a tag their definition carries, or every part for `all`.
pub fn parts_answering(sim: &Sim, who: EntityId, name: &str) -> Result<Vec<PartKey>, Reason> {
    let body = body_of(sim, who)?;
    let registry = sim.defs.try_registry::<BodyPartDef>();
    let tag = sim.defs.tags().lookup(name);
    let found: Vec<PartKey> = body
        .parts()
        .iter()
        .filter(|part| part.is_present())
        .filter(|part| {
            if name == item::ALL_PARTS || part.name() == name {
                return true;
            }
            let Some(registry) = registry else {
                return false;
            };
            let qualified = registry.name(part.def());
            let bare = qualified.rsplit(':').next().unwrap_or(qualified);
            qualified == name
                || bare == name
                || tag.is_some_and(|tag| {
                    registry
                        .try_get(part.def())
                        .is_some_and(|def| def.tags.contains(tag))
                })
        })
        .map(body::PartInst::key)
        .collect();
    if found.is_empty() {
        return Err(Reason::new(
            body::reason::NO_SUCH_PART,
            format!("{who} has no part that answers to `{name}`"),
        )
        .with("entity", who.to_string())
        .with("part", name));
    }
    Ok(found)
}

/// Changes a layer's material or thickness on some parts (06 §6.5
/// `set_layer`): see [`edit::set_layer`]. `parts` names them as
/// [`parts_answering`] reads a name. Returns how many parts changed.
pub fn set_layer(
    sim: &mut Sim,
    who: EntityId,
    parts: &str,
    layer: Option<usize>,
    material: Option<MaterialId>,
    thickness: Option<Real>,
    cause: Cause,
) -> Result<usize, Reason> {
    let keys = parts_answering(sim, who, parts)?;
    let count = keys.len();
    edit(sim, who, "set_layer", parts, false, cause, |cx| {
        edit::set_layer(cx, who, &keys, layer, material, thickness).map(|changes| Edited {
            value: (),
            changes,
            detached: Vec::new(),
        })
    })?;
    Ok(count)
}

fn just(changes: Changes) -> Edited<()> {
    Edited {
        value: (),
        changes,
        detached: Vec::new(),
    }
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
    edit(sim, who, "scale_part", &name, false, cause, |cx| {
        edit::scale_part(cx, who, part, scale).map(just)
    })
    .map(|_| ())
}

/// Gives a body a new overall size (06 §6.5 `scale_body`).
pub fn scale_body(sim: &mut Sim, who: EntityId, scale: Real, cause: Cause) -> Result<(), Reason> {
    edit(sim, who, "scale_body", "", false, cause, |cx| {
        edit::scale_body(cx, who, scale).map(just)
    })
    .map(|_| ())
}

/// Rebuilds a body from another plan (06 §6.5 `set_body_plan`): see
/// [`edit::set_body_plan`]. With `leave`, what the new plan does not
/// map is left as items, which are returned.
pub fn set_body_plan(
    sim: &mut Sim,
    who: EntityId,
    plan: BodyPlanId,
    leave: bool,
    cause: Cause,
) -> Result<Vec<EntityId>, Reason> {
    let ((), items) = edit(sim, who, "set_body_plan", "", leave, cause, |cx| {
        let (detached, changes) = edit::set_body_plan(cx, who, plan, cause)?;
        Ok(Edited {
            value: (),
            changes,
            detached,
        })
    })?;
    Ok(items)
}

/// Takes a share of a whole layer off the damage of every layer of a
/// part (06 §6.5 `heal_part`).
pub fn heal_part(
    sim: &mut Sim,
    who: EntityId,
    part: PartKey,
    share: Real,
    cause: Cause,
) -> Result<(), Reason> {
    let name = name_of(sim, who, part);
    edit(sim, who, "heal_part", &name, false, cause, |cx| {
        Ok(just(body::heal_part(cx, who, part, share)?))
    })
    .map(|_| ())
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
    edit(sim, who, "damage_part", &name, false, cause, |cx| {
        edit::damage_part(cx, who, part, layer, share).map(just)
    })
    .map(|_| ())
}

/// Disables a part from outside, whatever its condition, or with
/// `restore` lets it go again (06 §6.5 `disable_part`, `restore_part`).
pub fn hold_part(
    sim: &mut Sim,
    who: EntityId,
    part: PartKey,
    restore: bool,
    cause: Cause,
) -> Result<(), Reason> {
    let name = name_of(sim, who, part);
    let verb = if restore {
        "restore_part"
    } else {
        "disable_part"
    };
    edit(sim, who, verb, &name, false, cause, |cx| {
        let changes = if restore {
            body::restore(cx, who, part)?
        } else {
            body::disable(cx, who, part)?
        };
        Ok(just(changes))
    })
    .map(|_| ())
}

// ---- The verbs ---------------------------------------------------------------

fn arg<'a>(call: &'a VerbCall<'_>, index: usize) -> &'a Value {
    call.args.get(index).unwrap_or(&Value::Null)
}

fn entity(sim: &Sim, call: &VerbCall<'_>, index: usize, name: &str) -> Result<EntityId, Reason> {
    match sim.subject_of(arg(call, index)) {
        Some(Subject::Entity(id)) => Ok(id),
        _ => Err(bad_argument(name, "an entity")),
    }
}

fn text<'a>(call: &'a VerbCall<'_>, index: usize, name: &str) -> Result<&'a str, Reason> {
    arg(call, index)
        .as_str()
        .ok_or_else(|| bad_argument(name, "a text"))
}

fn text_or_null<'a>(
    call: &'a VerbCall<'_>,
    index: usize,
    name: &str,
) -> Result<Option<&'a str>, Reason> {
    match arg(call, index) {
        Value::Null => Ok(None),
        _ => text(call, index, name).map(Some),
    }
}

fn real(call: &VerbCall<'_>, index: usize, name: &str) -> Result<Real, Reason> {
    arg(call, index)
        .as_number()
        .ok_or_else(|| bad_argument(name, "a number"))
}

fn real_or_null(call: &VerbCall<'_>, index: usize, name: &str) -> Result<Option<Real>, Reason> {
    match arg(call, index) {
        Value::Null => Ok(None),
        _ => real(call, index, name).map(Some),
    }
}

fn flag(call: &VerbCall<'_>, index: usize, name: &str) -> Result<bool, Reason> {
    match arg(call, index) {
        Value::Null => Ok(false),
        Value::Bool(value) => Ok(*value),
        _ => Err(bad_argument(name, "true or false")),
    }
}

fn layer(call: &VerbCall<'_>, index: usize) -> Result<Option<usize>, Reason> {
    match arg(call, index) {
        Value::Null => Ok(None),
        value => value
            .as_int()
            .and_then(|layer| usize::try_from(layer).ok())
            .map(Some)
            .ok_or_else(|| bad_argument("layer", "a layer's number, 0 for the outermost")),
    }
}

/// A definition an argument names: an id of the type, or its qualified
/// id as text.
fn def_arg<T: Def>(
    sim: &Sim,
    call: &VerbCall<'_>,
    index: usize,
    name: &str,
) -> Result<T::Id, Reason> {
    let ty = sim.exprs.registry().def_type_id(T::TYPE);
    let wanted = || bad_argument(name, &format!("a `{}` definition", T::TYPE));
    match arg(call, index) {
        Value::Id(id) if Some(id.ty) == ty => {
            <T::Id as clash_core::DenseId>::from_index(id.index as usize).ok_or_else(wanted)
        }
        Value::Str(text) => sim
            .defs
            .try_registry::<T>()
            .and_then(|registry| registry.lookup(text))
            .ok_or_else(wanted),
        _ => Err(wanted()),
    }
}

/// Where a verb puts a part: `on` the part named, in `socket`, or inside
/// it with `inside`.
fn attach_arg(
    sim: &Sim,
    who: EntityId,
    call: &VerbCall<'_>,
    on: usize,
    socket: usize,
    inside: usize,
) -> Result<Attach, Reason> {
    let parent = part_named(sim, who, text(call, on, "on")?)?;
    let inside = flag(call, inside, "inside")?;
    Ok(Attach {
        parent,
        relation: if inside { Relation::In } else { Relation::On },
        socket: text_or_null(call, socket, "socket")?.map(str::to_owned),
    })
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

const WHO: (&str, &str, bool, &str) = ("who", "entity", false, "Whose body.");
const PART: (&str, &str, bool, &str) = (
    "part",
    "string",
    false,
    "The part, by its name in the body.",
);
const LEAVE: (&str, &str, bool, &str) = (
    "leave",
    "bool",
    true,
    "Whether what comes off is left as an item where the body is. Default false: it is gone.",
);

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
                    "Adds a new part of a definition to a body (06 §6.5), on a part in a socket that takes it, or with `inside` inside it, with the organs its definition brings. A part of that name that is gone is replaced. Returns the new part's name. Refused with `unknown_part`, `no_such_socket`, `socket_refuses`, `no_free_socket`, `part_name_taken`, `too_many_parts`, `bad_amount` and by a handler of `body_changing`.",
                    &[
                        WHO,
                        ("part", "id<body_part>", false, "What kind of part."),
                        ("on", "string", false, "The part it is joined to, by name."),
                        (
                            "socket",
                            "string",
                            true,
                            "The socket it takes. Left out, the first that would take it.",
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
                let who = entity(sim, call, 0, "who")?;
                let def = def_arg::<BodyPartDef>(sim, call, 1, "part")?;
                let at = attach_arg(sim, who, call, 2, 3, 4)?;
                let name = text_or_null(call, 5, "name")?;
                let side = text_or_null(call, 6, "side")?.map(str::to_owned);
                let scale = real_or_null(call, 7, "scale")?.unwrap_or(Real::ONE);
                let key = add_part(sim, who, def, &at, name, side, scale, cause)?;
                Ok(Value::text(name_of(sim, who, key)))
            },
        },
        VerbDef {
            name: "remove_part",
            build: |registry| {
                spec(
                    registry,
                    "remove_part",
                    "Takes a part off a body with everything on it and in it (06 §6.5): the body no longer has them. Returns the item left, or null. Refused with `no_such_part`, `part_missing`, `root_part` and by a handler of `body_changing`.",
                    &[WHO, PART, LEAVE],
                )
            },
            run: |sim, call, cause| {
                let who = entity(sim, call, 0, "who")?;
                let part = part_named(sim, who, text(call, 1, "part")?)?;
                let leave = flag(call, 2, "leave")?;
                let item = remove_part(sim, who, part, leave, cause)?;
                Ok(item.map_or(Value::Null, |item| sim.subject_value(Subject::Entity(item))))
            },
        },
        VerbDef {
            name: "replace_part",
            build: |registry| {
                spec(
                    registry,
                    "replace_part",
                    "Swaps a part's definition (06 §6.5), keeping its name, its place and what is attached to it; its layers are new and its wounds gone. What is on it and no longer fits the new definition's sockets comes off. Returns how many items were left. Refused with `unknown_part`, `socket_refuses` and as `remove_part` is.",
                    &[
                        WHO,
                        PART,
                        ("with", "id<body_part>", false, "The new definition."),
                        LEAVE,
                    ],
                )
            },
            run: |sim, call, cause| {
                let who = entity(sim, call, 0, "who")?;
                let part = part_named(sim, who, text(call, 1, "part")?)?;
                let with = def_arg::<BodyPartDef>(sim, call, 2, "with")?;
                let leave = flag(call, 3, "leave")?;
                let items = replace_part(sim, who, part, with, leave, cause)?;
                Ok(Value::Int(i64::try_from(items.len()).unwrap_or(i64::MAX)))
            },
        },
        VerbDef {
            name: "graft_part",
            build: |registry| {
                spec(
                    registry,
                    "graft_part",
                    "Attaches a part that came off a body, from this body or another (06 §6.5): the item a part left. Its parts keep their names, layers, damage and size; the item is used up. Returns the grafted part's name. Refused with `not_a_part` and as `add_part` is.",
                    &[
                        WHO,
                        ("item", "entity", false, "The item a part left."),
                        ("on", "string", false, "The part it is joined to, by name."),
                        (
                            "socket",
                            "string",
                            true,
                            "The socket it takes. Left out, the first that would take it.",
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
                let who = entity(sim, call, 0, "who")?;
                let part = entity(sim, call, 1, "item")?;
                let at = attach_arg(sim, who, call, 2, 3, 4)?;
                let key = graft_part(sim, who, part, &at, cause)?;
                Ok(Value::text(name_of(sim, who, key)))
            },
        },
        VerbDef {
            name: "set_layer",
            build: |registry| {
                spec(
                    registry,
                    "set_layer",
                    "Changes what a layer is made of, how thick it is, or both, on every part that answers to `part` (06 §6.5). Returns how many parts changed. Refused with `no_such_part`, `no_such_layer`, `unknown_material`, `bad_amount` and by a handler of `body_changing`.",
                    &[
                        WHO,
                        (
                            "part",
                            "string",
                            false,
                            "The parts: a part's name, the id of its definition, a tag its definition carries, or `all`.",
                        ),
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
                let who = entity(sim, call, 0, "who")?;
                let parts = text(call, 1, "part")?.to_owned();
                let layer = layer(call, 2)?;
                let material = match arg(call, 3) {
                    Value::Null => None,
                    _ => Some(def_arg::<MaterialDef>(sim, call, 3, "material")?),
                };
                let thickness = real_or_null(call, 4, "thickness")?;
                let count = set_layer(sim, who, &parts, layer, material, thickness, cause)?;
                Ok(Value::Int(i64::try_from(count).unwrap_or(i64::MAX)))
            },
        },
        VerbDef {
            name: "scale_part",
            build: |registry| {
                spec(
                    registry,
                    "scale_part",
                    "Gives a part a new size relative to its body (06 §6.5): its length, girth and layers go with it, its mass with the cube. Refused with `no_such_part`, `part_missing`, `bad_amount` and by a handler of `body_changing`.",
                    &[
                        WHO,
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
                let who = entity(sim, call, 0, "who")?;
                let part = part_named(sim, who, text(call, 1, "part")?)?;
                let scale = real(call, 2, "scale")?;
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
                    "Gives a body a new overall size (06 §6.5): every part, its height and its mass go with it. Refused with `no_body`, `bad_amount` and by a handler of `body_changing`.",
                    &[
                        WHO,
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
                let who = entity(sim, call, 0, "who")?;
                let scale = real(call, 1, "scale")?;
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
                    "Rebuilds a body from another plan (06 §6.5), keeping each part the new plan names with the same definition, damage and wounds included; the rest comes off. Returns how many items were left. Refused with `unknown_plan`, `too_many_parts` and by a handler of `body_changing`.",
                    &[WHO, ("plan", "id<body_plan>", false, "The plan."), LEAVE],
                )
            },
            run: |sim, call, cause| {
                let who = entity(sim, call, 0, "who")?;
                let plan = def_arg::<BodyPlanDef>(sim, call, 1, "plan")?;
                let leave = flag(call, 2, "leave")?;
                let items = set_body_plan(sim, who, plan, leave, cause)?;
                Ok(Value::Int(i64::try_from(items.len()).unwrap_or(i64::MAX)))
            },
        },
        VerbDef {
            name: "heal_part",
            build: |registry| {
                spec(
                    registry,
                    "heal_part",
                    "Takes a share of a whole layer off the damage of every layer of a part (06 §6.5): 1 mends it. A wound whose layers are whole again is healed. Refused with `no_such_part`, `part_missing`, `bad_amount` and by a handler of `body_changing`.",
                    &[
                        WHO,
                        PART,
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
                let who = entity(sim, call, 0, "who")?;
                let part = part_named(sim, who, text(call, 1, "part")?)?;
                let share = real(call, 2, "share")?;
                heal_part(sim, who, part, share, cause)?;
                Ok(Value::Null)
            },
        },
        VerbDef {
            name: "damage_part",
            build: |registry| {
                spec(
                    registry,
                    "damage_part",
                    "Adds a share of a whole layer to the damage of one layer of a part, or of each (06 §6.5). No wound is made, and no layer is taken past 254 255ths. Refused with `no_such_part`, `part_missing`, `no_such_layer`, `bad_amount` and by a handler of `body_changing`.",
                    &[
                        WHO,
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
                let who = entity(sim, call, 0, "who")?;
                let part = part_named(sim, who, text(call, 1, "part")?)?;
                let layer = layer(call, 2)?;
                let share = real(call, 3, "share")?;
                damage_part(sim, who, part, layer, share, cause)?;
                Ok(Value::Null)
            },
        },
        VerbDef {
            name: "disable_part",
            build: |registry| {
                spec(
                    registry,
                    "disable_part",
                    "Disables a part from outside, whatever its condition (06 §6.5): it contributes nothing and its channel slots are gone until `restore_part`. Refused with `no_such_part`, `part_missing` and by a handler of `body_changing`.",
                    &[WHO, PART],
                )
            },
            run: |sim, call, cause| {
                let who = entity(sim, call, 0, "who")?;
                let part = part_named(sim, who, text(call, 1, "part")?)?;
                hold_part(sim, who, part, false, cause)?;
                Ok(Value::Null)
            },
        },
        VerbDef {
            name: "restore_part",
            build: |registry| {
                spec(
                    registry,
                    "restore_part",
                    "Lets a part disabled from outside go again (06 §6.5): it works if its condition allows. Refused with `no_such_part`, `part_missing` and by a handler of `body_changing`.",
                    &[WHO, PART],
                )
            },
            run: |sim, call, cause| {
                let who = entity(sim, call, 0, "who")?;
                let part = part_named(sim, who, text(call, 1, "part")?)?;
                hold_part(sim, who, part, true, cause)?;
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
        let registry = crate::engine_registry();
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
