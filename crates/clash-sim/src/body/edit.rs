//! Editing bodies (06 §6.5): adding, replacing and grafting parts,
//! changing what their layers are made of, resizing, and rebuilding a
//! body from another plan.
//!
//! Each function checks everything it can before it changes anything, so
//! that a refusal changes nothing, and then makes its change through the
//! same bookkeeping as every other body function: closing and healing are
//! brought up to date first, and afterwards each part's state, the
//! functions, the channel slots, the mass and size handed to `Physical`,
//! the leaks of wounds and the next wake-up are worked out again. So the
//! cached values always equal [`recompute`](super::recompute) (06
//! invariants B2, B3), whatever sequence of edits was made.
//!
//! | Edit | Function |
//! |------|----------|
//! | `add_part` | [`add_part`] |
//! | `remove_part` | [`remove_part`] |
//! | `replace_part` | [`replace_part`] |
//! | `graft_part` | [`graft_part`] |
//! | `set_layer` | [`set_layer`] |
//! | `scale_part`, `scale_body` | [`scale_part`], [`scale_body`] |
//! | `set_body_plan` | [`set_body_plan`] |
//! | `heal_part`, `damage_part` | [`heal_part`](super::heal_part), [`damage_part`] |
//! | `disable_part`, `restore_part` | [`disable`](super::disable), [`restore`](super::restore) |
//!
//! Like the rest of this module these know nothing of the simulation:
//! raising `body_changing` and `body_changed`, making items of what comes
//! off and working attributes out again are the caller's
//! (`crate::reshape`).
//!
//! # Sockets
//!
//! A part `on` another takes a socket of it: one its definition has, that
//! accepts the part's definition (any, if its `accepts` is empty) and has
//! room (fewer parts there than its `count`). Left unnamed, the socket is
//! the first of the parent's, in order of name, that would take it. A
//! part `in` another takes no socket.
//!
//! # Names
//!
//! A part's name is not shared with any other part that is there. A part
//! that is gone keeps its name only until a new part takes it: adding,
//! grafting or replanning a part of that name drops the listing of the
//! gone one and of what was gone with it, so that a limb that grows back
//! is the limb of that name again.

use clash_core::{Dur, EntityId, Real};
use clash_expr::{Host, Value};
use clash_world::Cause;
use clash_world::defs::MaterialId;
use thiserror::Error;

use super::defs::{
    BodyDefs, BodyPartDef, BodyPartId, BodyPlanId, MAX_NESTING, MAX_PARTS, Relation,
};
use super::{
    Body, BodyCx, BodyError, Change, Changes, DAMAGE_PARTS, Detach, Detached, LayerInst, PartInst,
    PartKey, PartState, Wound, body_of, change, present_part, slots_of_part, take_off,
};

/// The stable codes of the refusals of editing (14 §5), beside those of
/// [`super::reason`].
pub mod reason {
    /// The id is not a `body_part` definition. Values: none.
    pub const UNKNOWN_PART: &str = "unknown_part";
    /// The parent part has no socket of that name. Values: `entity`,
    /// `part`, `socket`.
    pub const NO_SUCH_SOCKET: &str = "no_such_socket";
    /// The socket does not take that kind of part. Values: `entity`,
    /// `part`, `socket`.
    pub const SOCKET_REFUSES: &str = "socket_refuses";
    /// The socket is full, or no socket of the parent would take the
    /// part. Values: `entity`, `part`.
    pub const NO_FREE_SOCKET: &str = "no_free_socket";
    /// A part that is there already has the name. Values: `entity`,
    /// `name`.
    pub const NAME_TAKEN: &str = "part_name_taken";
    /// The body would have more than 1024 parts. Values: none.
    pub const TOO_MANY_PARTS: &str = "too_many_parts";
    /// The part has no layer of that number. Values: `entity`, `part`,
    /// `layer`.
    pub const NO_SUCH_LAYER: &str = "no_such_layer";
    /// The id is not a material. Values: none.
    pub const UNKNOWN_MATERIAL: &str = "unknown_material";
    /// What was to be grafted holds no part. Values: none.
    pub const NOT_A_PART: &str = "not_a_part";
}

/// Why an edit was refused. A refused edit has changed nothing.
#[derive(Debug, Clone, PartialEq, Error)]
#[non_exhaustive]
pub enum EditError {
    /// Refused as any body function refuses.
    #[error(transparent)]
    Body(#[from] BodyError),
    /// The id is not a part definition.
    #[error("there is no body part definition with this id")]
    UnknownPart,
    /// The parent has no such socket.
    #[error("the part `{part}` of {entity} has no socket `{socket}`")]
    NoSuchSocket {
        /// The entity.
        entity: EntityId,
        /// The parent's name.
        part: String,
        /// The socket asked for.
        socket: String,
    },
    /// The socket does not take that kind of part.
    #[error("the socket `{socket}` of `{part}` does not take that kind of part")]
    SocketRefuses {
        /// The entity.
        entity: EntityId,
        /// The parent's name.
        part: String,
        /// The socket.
        socket: String,
    },
    /// No socket of the parent has room for the part.
    #[error("no socket of the part `{part}` of {entity} has room for it")]
    NoFreeSocket {
        /// The entity.
        entity: EntityId,
        /// The parent's name.
        part: String,
    },
    /// The name is taken by a part that is there.
    #[error("{entity} already has a part named `{name}`")]
    NameTaken {
        /// The entity.
        entity: EntityId,
        /// The name.
        name: String,
    },
    /// The body would have too many parts.
    #[error("a body has at most {MAX_PARTS} parts")]
    TooManyParts,
    /// The part has no such layer.
    #[error("the part `{part}` of {entity} has no layer {layer}")]
    NoSuchLayer {
        /// The entity.
        entity: EntityId,
        /// The part's name.
        part: String,
        /// The layer asked for, outermost 0.
        layer: usize,
    },
    /// The id is not a material.
    #[error("there is no material with this id")]
    UnknownMaterial,
    /// What was to be grafted holds no part.
    #[error("there is no part to graft")]
    NotAPart,
}

impl EditError {
    /// The reason's stable code: one of [`reason`], or a body's own.
    pub const fn code(&self) -> &'static str {
        match self {
            EditError::Body(error) => error.code(),
            EditError::UnknownPart => reason::UNKNOWN_PART,
            EditError::NoSuchSocket { .. } => reason::NO_SUCH_SOCKET,
            EditError::SocketRefuses { .. } => reason::SOCKET_REFUSES,
            EditError::NoFreeSocket { .. } => reason::NO_FREE_SOCKET,
            EditError::NameTaken { .. } => reason::NAME_TAKEN,
            EditError::TooManyParts => reason::TOO_MANY_PARTS,
            EditError::NoSuchLayer { .. } => reason::NO_SUCH_LAYER,
            EditError::UnknownMaterial => reason::UNKNOWN_MATERIAL,
            EditError::NotAPart => reason::NOT_A_PART,
        }
    }

    /// The values involved, by name, as the expression language holds
    /// them. An entity is given as its id's bits.
    pub fn values(&self) -> Vec<(&'static str, Value)> {
        let bits = |entity: &EntityId| Value::Int(entity.to_bits() as i64);
        match self {
            EditError::Body(error) => error.values(),
            EditError::NoSuchSocket {
                entity,
                part,
                socket,
            }
            | EditError::SocketRefuses {
                entity,
                part,
                socket,
            } => vec![
                ("entity", bits(entity)),
                ("part", Value::text(part)),
                ("socket", Value::text(socket)),
            ],
            EditError::NoFreeSocket { entity, part } => {
                vec![("entity", bits(entity)), ("part", Value::text(part))]
            }
            EditError::NameTaken { entity, name } => {
                vec![("entity", bits(entity)), ("name", Value::text(name))]
            }
            EditError::NoSuchLayer {
                entity,
                part,
                layer,
            } => vec![
                ("entity", bits(entity)),
                ("part", Value::text(part)),
                (
                    "layer",
                    Value::Int(i64::try_from(*layer).unwrap_or(i64::MAX)),
                ),
            ],
            EditError::UnknownPart
            | EditError::TooManyParts
            | EditError::UnknownMaterial
            | EditError::NotAPart => Vec::new(),
        }
    }
}

/// Where a part goes: on or in a part that is there, and for a part on
/// it, which socket. `None` takes the first socket that would.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Attach {
    /// The part it is joined to.
    pub parent: PartKey,
    /// On it or in it.
    pub relation: Relation,
    /// For a part on it, the socket it takes.
    pub socket: Option<String>,
}

impl Attach {
    /// On a part, in the first socket that would take it.
    pub const fn on(parent: PartKey) -> Attach {
        Attach {
            parent,
            relation: Relation::On,
            socket: None,
        }
    }

    /// Inside a part.
    pub const fn inside(parent: PartKey) -> Attach {
        Attach {
            parent,
            relation: Relation::In,
            socket: None,
        }
    }
}

fn positive(value: Real) -> Result<Real, EditError> {
    if value.is_finite() && value > Real::ZERO {
        Ok(value)
    } else {
        Err(BodyError::BadAmount { amount: value }.into())
    }
}

fn part_def(defs: &BodyDefs, id: BodyPartId) -> Result<&BodyPartDef, EditError> {
    defs.part(id).ok_or(EditError::UnknownPart)
}

/// A part definition's layers, new, at a size.
fn fresh_layers(def: &BodyPartDef, size: Real) -> Vec<LayerInst> {
    def.layers
        .iter()
        .zip(def.weights())
        .map(|(layer, weight)| LayerInst {
            material: layer.material.id(),
            thickness: layer.thickness.get() * size,
            weight: *weight,
            damage: 0,
            progress: Dur::ZERO,
        })
        .collect()
}

/// The part that is there with this name, if one is.
fn present_named<'a>(body: &'a Body, name: &str) -> Option<&'a PartInst> {
    body.parts
        .iter()
        .find(|part| part.is_present() && part.name == name)
}

/// Refuses a name a part that is there has.
fn free_name(body: &Body, entity: EntityId, name: &str) -> Result<(), EditError> {
    match present_named(body, name) {
        Some(_) => Err(EditError::NameTaken {
            entity,
            name: name.to_owned(),
        }),
        None => Ok(()),
    }
}

/// Drops the listings of gone parts with any of these names, and of what
/// was gone with them.
fn drop_gone_named(body: &mut Body, names: &[String]) {
    let roots: Vec<PartKey> = body
        .parts
        .iter()
        .filter(|part| !part.is_present() && names.contains(&part.name))
        .map(PartInst::key)
        .collect();
    let mut dropped: Vec<PartKey> = Vec::new();
    for root in roots {
        for key in body.subtree(root) {
            if body.part(key).is_some_and(|part| !part.is_present()) {
                dropped.push(key);
            }
        }
    }
    if !dropped.is_empty() {
        body.parts.retain(|part| !dropped.contains(&part.key));
    }
}

/// The socket of a parent a part of a definition takes, checked.
fn socket_for(
    body: &Body,
    defs: &BodyDefs,
    entity: EntityId,
    parent: &PartInst,
    asked: Option<&str>,
    def: BodyPartId,
) -> Result<String, EditError> {
    let parent_def = part_def(defs, parent.def)?;
    let has_room =
        |name: &str, count: u16| body.in_socket(parent.key, name).count() < usize::from(count);
    let accepts = |socket: &super::defs::SocketDef| {
        socket.accepts.is_empty() || socket.accepts.iter().any(|accepted| accepted.id() == def)
    };
    match asked {
        Some(name) => {
            let socket = parent_def
                .sockets
                .get(name)
                .ok_or_else(|| EditError::NoSuchSocket {
                    entity,
                    part: parent.name.clone(),
                    socket: name.to_owned(),
                })?;
            if !accepts(socket) {
                return Err(EditError::SocketRefuses {
                    entity,
                    part: parent.name.clone(),
                    socket: name.to_owned(),
                });
            }
            if !has_room(name, socket.count) {
                return Err(EditError::NoFreeSocket {
                    entity,
                    part: parent.name.clone(),
                });
            }
            Ok(name.to_owned())
        }
        None => parent_def
            .sockets
            .iter()
            .find(|(name, socket)| accepts(socket) && has_room(name, socket.count))
            .map(|(name, _)| name.clone())
            .ok_or_else(|| EditError::NoFreeSocket {
                entity,
                part: parent.name.clone(),
            }),
    }
}

/// A part planned for a body, not yet given its key.
struct Planned {
    name: String,
    def: BodyPartId,
    /// The position of its parent among what is planned, or a part of
    /// the body.
    parent: Parent,
    relation: Relation,
    socket: Option<String>,
    side: Option<String>,
    scale: Real,
    layers: Vec<LayerInst>,
}

#[derive(Clone, Copy)]
enum Parent {
    Body(PartKey),
    Planned(usize),
}

/// A part and the organs its definition brings, and theirs, as a plan of
/// 06 §6.2 would lay them out.
fn with_organs(
    defs: &BodyDefs,
    body_scale: Real,
    first: Planned,
) -> Result<Vec<Planned>, EditError> {
    let mut planned = vec![first];
    let mut depth: Vec<usize> = vec![0];
    let mut next = 0;
    while next < planned.len() {
        let def = part_def(defs, planned[next].def)?;
        for organ in &def.inside {
            if depth[next] >= MAX_NESTING {
                return Err(EditError::TooManyParts);
            }
            let organ_def = part_def(defs, organ.part.id())?;
            let scale = planned[next].scale;
            planned.push(Planned {
                name: format!("{}.{}", planned[next].name, organ.id.value),
                def: organ.part.id(),
                parent: Parent::Planned(next),
                relation: Relation::In,
                socket: None,
                side: planned[next].side.clone(),
                scale,
                layers: fresh_layers(organ_def, body_scale * scale),
            });
            depth.push(depth[next] + 1);
            if planned.len() > MAX_PARTS {
                return Err(EditError::TooManyParts);
            }
        }
        next += 1;
    }
    Ok(planned)
}

/// Checks that a body has room for `more` parts and keys for them.
fn room_for(body: &Body, more: usize) -> Result<(), EditError> {
    let present = body.parts.iter().filter(|part| part.is_present()).count();
    let keys_left = usize::from(u16::MAX - body.next_part);
    if present + more > MAX_PARTS || more > keys_left {
        return Err(EditError::TooManyParts);
    }
    Ok(())
}

/// Puts planned parts into a body with new keys, in order. Returns the
/// key of the first.
fn insert_planned(body: &mut Body, planned: Vec<Planned>) -> PartKey {
    let names: Vec<String> = planned.iter().map(|part| part.name.clone()).collect();
    drop_gone_named(body, &names);
    let first = PartKey::new(body.next_part);
    let mut keys: Vec<PartKey> = Vec::with_capacity(planned.len());
    for part in planned {
        let key = PartKey::new(body.next_part);
        body.next_part = body.next_part.saturating_add(1);
        let parent = match part.parent {
            Parent::Body(key) => key,
            Parent::Planned(at) => keys[at],
        };
        keys.push(key);
        body.parts.push(PartInst {
            key,
            name: part.name,
            def: part.def,
            parent: Some(parent),
            relation: part.relation,
            socket: part.socket,
            side: part.side,
            scale: part.scale,
            layers: part.layers,
            state: PartState::Whole,
            held: false,
        });
    }
    // New keys are the highest: the parts stay in order of key.
    first
}

/// Takes a part off a body as an edit does: with everything on it and in
/// it, and with the listings of what was gone already beneath it, so
/// that nothing listed hangs off a part that is not.
fn take_off_whole(
    body: &mut Body,
    defs: &BodyDefs,
    entity: EntityId,
    root: PartKey,
    cause: Cause,
    work: &mut super::Work,
) -> Option<Detached> {
    let below = body.subtree(root);
    let off = take_off(body, defs, entity, root, Detach::Removed, cause, work);
    body.parts.retain(|part| !below.contains(&part.key));
    off
}

/// Takes a part off a body with everything on it and in it (06 §6.5
/// `remove_part`): the body no longer has them, nor the listings of
/// what was gone already beneath it. Returns what came off, for whoever
/// makes an item of it.
///
/// Refused, changing nothing, for a part that is not there and for the
/// root.
pub fn remove_part<H: Host + ?Sized>(
    cx: &mut BodyCx<'_, H>,
    entity: EntityId,
    part: PartKey,
    cause: Cause,
) -> Result<(Detached, Changes), EditError> {
    let found = present_part(body_of(cx.bodies, entity)?, entity, part)?;
    if found.parent.is_none() {
        return Err(BodyError::RootPart {
            entity,
            part,
            name: found.name.clone(),
        }
        .into());
    }
    let mut detached = None;
    let changes = change(cx, entity, |body, defs, work| {
        detached = take_off_whole(body, defs, entity, part, cause, work);
        Ok(())
    })?;
    let detached = detached.ok_or(BodyError::NoSuchPart { entity, part })?;
    Ok((detached, changes))
}

/// Adds a new part to a body (06 §6.5 `add_part`): of a definition, on a
/// part that is there in a socket that takes it, or inside it, with the
/// organs its definition brings. `name` is its name in the body: left
/// out, its definition's bare id, with `_2`, `_3` and so on after it if a
/// part that is there has that. `scale` is its size relative to the body.
///
/// Refused, changing nothing, for a parent that is not there, an id that
/// is not a part definition, a socket that does not exist, does not take
/// it or is full (or, unnamed, no socket that would), a name a part that
/// is there has, a scale that is not above zero, and a body that would
/// have more than 1024 parts. Returns the new part's key.
pub fn add_part<H: Host + ?Sized>(
    cx: &mut BodyCx<'_, H>,
    entity: EntityId,
    def: BodyPartId,
    at: &Attach,
    name: Option<&str>,
    side: Option<String>,
    scale: Real,
) -> Result<(PartKey, Changes), EditError> {
    let scale = positive(scale)?;
    let defs = cx.defs;
    let body = body_of(cx.bodies, entity)?;
    let parent = present_part(body, entity, at.parent)?;
    let new_def = part_def(defs, def)?;
    let socket = match at.relation {
        Relation::On => Some(socket_for(
            body,
            defs,
            entity,
            parent,
            at.socket.as_deref(),
            def,
        )?),
        Relation::In => None,
    };
    let name = match name {
        Some(name) => {
            free_name(body, entity, name)?;
            name.to_owned()
        }
        None => {
            let qualified = defs.defs().registry::<BodyPartDef>().name(def);
            let bare = qualified.rsplit(':').next().unwrap_or(qualified).to_owned();
            let mut name = bare.clone();
            let mut count = 1;
            while present_named(body, &name).is_some() && count <= MAX_PARTS {
                count += 1;
                name = format!("{bare}_{count}");
            }
            name
        }
    };
    let planned = with_organs(
        defs,
        body.scale,
        Planned {
            name,
            def,
            parent: Parent::Body(at.parent),
            relation: at.relation,
            socket,
            side,
            scale,
            layers: fresh_layers(new_def, body.scale * scale),
        },
    )?;
    for part in &planned[1..] {
        free_name(body, entity, &part.name)?;
    }
    room_for(body, planned.len())?;
    let mut key = PartKey::new(0);
    let changes = change(cx, entity, |body, _, _| {
        key = insert_planned(body, planned);
        Ok(())
    })?;
    Ok((key, changes))
}

/// Swaps a part's definition (06 §6.5 `replace_part`), keeping its key,
/// its name, its place and what is attached to it. Its layers are the new
/// definition's, new, at its size, and its wounds are gone with the old
/// tissue. A part on it whose socket the new definition lacks, does not
/// take it, or has no room left for it comes off, with everything on it
/// and in it, and is returned for whoever makes items. The organs the new
/// definition brings that the part does not have are added.
///
/// Refused, changing nothing, for a part that is not there, an id that is
/// not a part definition, and a part whose socket would not take the new
/// definition.
pub fn replace_part<H: Host + ?Sized>(
    cx: &mut BodyCx<'_, H>,
    entity: EntityId,
    part: PartKey,
    with: BodyPartId,
    cause: Cause,
) -> Result<(Vec<Detached>, Changes), EditError> {
    let defs = cx.defs;
    let body = body_of(cx.bodies, entity)?;
    let found = present_part(body, entity, part)?.clone();
    let new_def = part_def(defs, with)?;
    if let (Some(parent), Relation::On, Some(socket)) =
        (found.parent, found.relation, found.socket.as_deref())
        && let Some(parent) = body.part(parent)
    {
        let parent_def = part_def(defs, parent.def)?;
        let takes = parent_def.sockets.get(socket).is_some_and(|socket| {
            socket.accepts.is_empty() || socket.accepts.iter().any(|accepted| accepted.id() == with)
        });
        if !takes {
            return Err(EditError::SocketRefuses {
                entity,
                part: parent.name.clone(),
                socket: socket.to_owned(),
            });
        }
    }
    // What is on it and no longer fits comes off.
    let mut used: Vec<(String, u16)> = Vec::new();
    let mut misfits: Vec<PartKey> = Vec::new();
    for child in body.children(part).filter(|child| child.is_present()) {
        if child.relation != Relation::On {
            continue;
        }
        let Some(socket) = child.socket.as_deref() else {
            continue;
        };
        let fits = new_def.sockets.get(socket).is_some_and(|def| {
            let accepts = def.accepts.is_empty() || def.accepts.iter().any(|a| a.id() == child.def);
            let taken = used
                .iter()
                .find(|(name, _)| name == socket)
                .map_or(0, |(_, count)| *count);
            accepts && taken < def.count
        });
        if fits {
            match used.iter_mut().find(|(name, _)| name == socket) {
                Some((_, count)) => *count += 1,
                None => used.push((socket.to_owned(), 1)),
            }
        } else {
            misfits.push(child.key);
        }
    }
    // The organs the new definition brings that are not there, each in
    // its parent: the part, an organ that is there, or one added here.
    let all = with_organs(
        defs,
        body.scale,
        Planned {
            name: found.name.clone(),
            def: with,
            parent: Parent::Body(part),
            relation: found.relation,
            socket: None,
            side: found.side.clone(),
            scale: found.scale,
            layers: Vec::new(),
        },
    )?;
    let names: Vec<String> = all.iter().map(|organ| organ.name.clone()).collect();
    let mut placed: Vec<Option<usize>> = vec![None; all.len()];
    let mut organs: Vec<Planned> = Vec::new();
    for (index, mut organ) in all.into_iter().enumerate().skip(1) {
        if present_named(body, &organ.name).is_some() {
            continue;
        }
        organ.parent = match organ.parent {
            Parent::Planned(0) | Parent::Body(_) => Parent::Body(part),
            Parent::Planned(parent) => match placed[parent] {
                Some(at) => Parent::Planned(at),
                None => match present_named(body, &names[parent]) {
                    Some(there) => Parent::Body(there.key),
                    None => continue,
                },
            },
        };
        placed[index] = Some(organs.len());
        organs.push(organ);
    }
    room_for(body, organs.len())?;
    let size = body.scale * found.scale;
    let mut detached = Vec::new();
    let changes = change(cx, entity, |body, defs, work| {
        for misfit in misfits {
            if let Some(off) = take_off_whole(body, defs, entity, misfit, cause, work) {
                detached.push(off);
            }
        }
        let Some(at) = body.part_at(part) else {
            return Ok(());
        };
        let working = body.parts[at].is_working();
        if working {
            let (channels, slots) = slots_of_part(defs, &body.parts[at]);
            if !slots.is_empty() {
                work.changes.push(Change::SlotsLost {
                    entity,
                    part,
                    channels,
                    slots,
                });
            }
        }
        // Its wounds go with the old tissue.
        let (gone, kept): (Vec<Wound>, Vec<Wound>) = core::mem::take(&mut body.wounds)
            .into_iter()
            .partition(|wound| wound.part == part);
        body.wounds = kept;
        work.gone.extend(gone.iter().map(Wound::id));
        let replaced = &mut body.parts[at];
        replaced.def = with;
        replaced.layers = defs
            .part(with)
            .map_or_else(Vec::new, |def| fresh_layers(def, size));
        if working && !replaced.held {
            let (channels, slots) = slots_of_part(defs, &body.parts[at]);
            if !slots.is_empty() {
                work.changes.push(Change::SlotsGained {
                    entity,
                    part,
                    channels,
                    slots,
                });
            }
        }
        insert_planned(body, organs);
        Ok(())
    })?;
    Ok((detached, changes))
}

/// Attaches a part that came off a body, with everything that came off
/// with it (06 §6.5 `graft_part`): from this body or another. Each part
/// keeps its name, its layers and their damage, and its size; each gets
/// a new key. Its wounds do not come with it: the damage of its layers
/// does, and heals as any damage does.
///
/// Refused, changing nothing, for a parent that is not there, a socket
/// that does not exist, does not take the part or is full, a name a part
/// that is there has, nothing to graft, and a body that would have more
/// than 1024 parts. Returns the key of the part grafted.
pub fn graft_part<H: Host + ?Sized>(
    cx: &mut BodyCx<'_, H>,
    entity: EntityId,
    part: &Detached,
    at: &Attach,
) -> Result<(PartKey, Changes), EditError> {
    let defs = cx.defs;
    let body = body_of(cx.bodies, entity)?;
    let parent = present_part(body, entity, at.parent)?;
    let root = part
        .parts
        .iter()
        .find(|inst| inst.key == part.root)
        .ok_or(EditError::NotAPart)?;
    part_def(defs, root.def)?;
    let socket = match at.relation {
        Relation::On => Some(socket_for(
            body,
            defs,
            entity,
            parent,
            at.socket.as_deref(),
            root.def,
        )?),
        Relation::In => None,
    };
    for inst in &part.parts {
        free_name(body, entity, &inst.name)?;
    }
    room_for(body, part.parts.len())?;
    // The root first, then the rest in order of key, each after its
    // parent.
    let mut order: Vec<&PartInst> = vec![root];
    let mut next = 0;
    while next < order.len() {
        let key = order[next].key;
        let mut children: Vec<&PartInst> = part
            .parts
            .iter()
            .filter(|inst| inst.parent == Some(key) && inst.key != part.root)
            .collect();
        children.sort_by_key(|inst| inst.key);
        order.extend(children);
        next += 1;
    }
    // Its size stays what it was.
    let rescale = part.scale / body.scale;
    let planned: Vec<Planned> = order
        .iter()
        .enumerate()
        .map(|(at_index, inst)| {
            let (parent, relation, socket) = if at_index == 0 {
                (Parent::Body(at.parent), at.relation, socket.clone())
            } else {
                let parent = order
                    .iter()
                    .position(|other| Some(other.key) == inst.parent)
                    .unwrap_or(0);
                (Parent::Planned(parent), inst.relation, inst.socket.clone())
            };
            Planned {
                name: inst.name.clone(),
                def: inst.def,
                parent,
                relation,
                socket,
                side: inst.side.clone(),
                scale: inst.scale * rescale,
                layers: inst.layers.clone(),
            }
        })
        .collect();
    let mut key = PartKey::new(0);
    let changes = change(cx, entity, |body, _, _| {
        key = insert_planned(body, planned);
        Ok(())
    })?;
    Ok((key, changes))
}

/// Changes what a layer is made of, how thick it is, or both
/// (06 §6.5 `set_layer`), on each of some parts: layer `layer`
/// (outermost 0), or with `None` every layer. A thickness is in metres,
/// at the part's size. A layer keeps its damage; the leaks of wounds
/// through it follow its new material.
///
/// Refused, changing nothing, for a part that is not there, a layer it
/// does not have, an id that is not a material, and a thickness that is
/// not above zero.
pub fn set_layer<H: Host + ?Sized>(
    cx: &mut BodyCx<'_, H>,
    entity: EntityId,
    parts: &[PartKey],
    layer: Option<usize>,
    material: Option<MaterialId>,
    thickness: Option<Real>,
) -> Result<Changes, EditError> {
    let thickness = thickness.map(positive).transpose()?;
    if let Some(material) = material
        && cx.defs.tissue(material).is_none()
    {
        return Err(EditError::UnknownMaterial);
    }
    let body = body_of(cx.bodies, entity)?;
    for key in parts {
        let found = present_part(body, entity, *key)?;
        if let Some(layer) = layer
            && layer >= found.layers.len()
        {
            return Err(EditError::NoSuchLayer {
                entity,
                part: found.name.clone(),
                layer,
            });
        }
    }
    Ok(change(cx, entity, |body, _, _| {
        for key in parts {
            let Some(found) = body.part_mut(*key) else {
                continue;
            };
            for (index, inst) in found.layers.iter_mut().enumerate() {
                if layer.is_some_and(|layer| layer != index) {
                    continue;
                }
                if let Some(material) = material {
                    inst.material = material;
                }
                if let Some(thickness) = thickness {
                    inst.thickness = thickness;
                }
            }
        }
        Ok(())
    })?)
}

/// Gives a part a new size relative to its body (06 §6.5 `scale_part`):
/// its length, girth and layers' thickness go with it, its area with the
/// square and its mass with the cube.
///
/// Refused for a part that is not there and a scale that is not above
/// zero.
pub fn scale_part<H: Host + ?Sized>(
    cx: &mut BodyCx<'_, H>,
    entity: EntityId,
    part: PartKey,
    scale: Real,
) -> Result<Changes, EditError> {
    let scale = positive(scale)?;
    present_part(body_of(cx.bodies, entity)?, entity, part)?;
    Ok(change(cx, entity, |body, _, _| {
        if let Some(found) = body.part_mut(part) {
            let factor = scale / found.scale;
            found.scale = scale;
            for layer in &mut found.layers {
                layer.thickness *= factor;
            }
        }
        Ok(())
    })?)
}

/// Gives a body a new overall size (06 §6.5 `scale_body`): every part's
/// length, girth and layers go with it, as do its height and mass.
///
/// Refused for an entity with no body and a scale that is not above
/// zero.
pub fn scale_body<H: Host + ?Sized>(
    cx: &mut BodyCx<'_, H>,
    entity: EntityId,
    scale: Real,
) -> Result<Changes, EditError> {
    let scale = positive(scale)?;
    body_of(cx.bodies, entity)?;
    Ok(change(cx, entity, |body, _, _| {
        let factor = scale / body.scale;
        body.scale = scale;
        for part in &mut body.parts {
            for layer in &mut part.layers {
                layer.thickness *= factor;
            }
        }
        Ok(())
    })?)
}

/// Adds damage to a part's layers directly (06 §6.5 `damage_part`): a
/// share of a whole layer, to layer `layer` or with `None` to each. No
/// wound is made, and damage that breaches nothing heals as any does. A
/// layer is left at least one 255th: taking a part away is
/// `remove_part`'s, destroying one a hit's.
///
/// Refused for a part that is not there, a layer it does not have, and a
/// share that is not above zero.
pub fn damage_part<H: Host + ?Sized>(
    cx: &mut BodyCx<'_, H>,
    entity: EntityId,
    part: PartKey,
    layer: Option<usize>,
    share: Real,
) -> Result<Changes, EditError> {
    let share = positive(share)?;
    let found = present_part(body_of(cx.bodies, entity)?, entity, part)?;
    if let Some(layer) = layer
        && layer >= found.layers.len()
    {
        return Err(EditError::NoSuchLayer {
            entity,
            part: found.name.clone(),
            layer,
        });
    }
    let whole = Real::from_i64(i64::from(DAMAGE_PARTS));
    let added = (share.min(Real::ONE) * whole).ceil().to_i64();
    let added = u8::try_from(added).unwrap_or(DAMAGE_PARTS);
    Ok(change(cx, entity, |body, _, _| {
        if let Some(found) = body.part_mut(part) {
            for (index, inst) in found.layers.iter_mut().enumerate() {
                if layer.is_some_and(|layer| layer != index) {
                    continue;
                }
                inst.damage = inst.damage.saturating_add(added).min(DAMAGE_PARTS - 1);
            }
        }
        Ok(())
    })?)
}

/// Rebuilds a body from another plan (06 §6.5 `set_body_plan`), mapping
/// parts by name: a part of the new plan with the name and the definition
/// of one the body has keeps that one, its key, layers, damage, wounds
/// and state (a limb that was lost stays lost), and takes the new plan's
/// place, socket, side and size. Every other part of the plan is new.
/// What the body has that the new plan does not map comes off, each
/// piece with what was on it and in it, and is returned for whoever
/// makes items of it; what was gone already is forgotten. A part the new
/// plan puts on one that is gone is gone with it. The body's overall
/// scale stays; its height is the new plan's.
///
/// Refused, changing nothing, for an entity with no body and a plan a
/// body cannot be built from.
pub fn set_body_plan<H: Host + ?Sized>(
    cx: &mut BodyCx<'_, H>,
    entity: EntityId,
    plan: BodyPlanId,
    cause: Cause,
) -> Result<(Vec<Detached>, Changes), EditError> {
    let defs = cx.defs;
    let layout = defs.layout(plan).ok_or(BodyError::UnknownPlan)?;
    let body = body_of(cx.bodies, entity)?;
    // Which part of the body each part of the plan keeps, if any.
    let mut kept: Vec<Option<PartKey>> = Vec::with_capacity(layout.parts.len());
    for planned in &layout.parts {
        kept.push(
            body.parts
                .iter()
                .find(|part| part.name == planned.name && part.def == planned.def)
                .map(PartInst::key),
        );
    }
    let new_count = kept.iter().filter(|key| key.is_none()).count();
    if layout.parts.len() > MAX_PARTS || new_count > usize::from(u16::MAX - body.next_part) {
        return Err(EditError::TooManyParts);
    }
    for planned in &layout.parts {
        part_def(defs, planned.def)?;
    }
    let mut detached = Vec::new();
    let changes = change(cx, entity, |body, defs, work| {
        let is_kept = |key: PartKey| kept.contains(&Some(key));
        // What the plan does not map and is there comes off, in pieces:
        // each piece starts at a part whose parent is kept or none.
        let leaving: Vec<PartKey> = body
            .parts
            .iter()
            .filter(|part| part.is_present() && !is_kept(part.key))
            .map(PartInst::key)
            .collect();
        let starts: Vec<PartKey> = leaving
            .iter()
            .copied()
            .filter(|key| {
                body.part(*key)
                    .and_then(|part| part.parent)
                    .is_none_or(|parent| !leaving.contains(&parent))
            })
            .collect();
        for start in starts {
            let mut keys = vec![start];
            let mut next = 0;
            while next < keys.len() {
                let parent = keys[next];
                keys.extend(
                    body.children(parent)
                        .filter(|child| leaving.contains(&child.key))
                        .map(PartInst::key),
                );
                next += 1;
            }
            keys.sort_unstable();
            let mut parts = Vec::with_capacity(keys.len());
            let mut mass = Real::ZERO;
            for key in &keys {
                let Some(part) = body.part(*key) else {
                    continue;
                };
                mass += body.part_mass(defs, part);
                if part.is_working() {
                    let (channels, slots) = slots_of_part(defs, part);
                    if !slots.is_empty() {
                        work.changes.push(Change::SlotsLost {
                            entity,
                            part: *key,
                            channels,
                            slots,
                        });
                    }
                }
                parts.push(part.clone());
            }
            parts.sort_by_key(|part| (part.key != start, part.key));
            let (taken, rest): (Vec<Wound>, Vec<Wound>) = core::mem::take(&mut body.wounds)
                .into_iter()
                .partition(|wound| keys.contains(&wound.part));
            body.wounds = rest;
            work.gone.extend(taken.iter().map(Wound::id));
            detached.push(Detached {
                from: entity,
                how: Detach::Removed,
                plan: body.plan,
                scale: body.scale,
                root: start,
                parent: body.part(start).and_then(|part| part.parent),
                parts,
                wounds: taken,
                mass,
                cause,
            });
        }
        // Wounds on parts that are gone and not kept go too.
        let (lost, rest): (Vec<Wound>, Vec<Wound>) = core::mem::take(&mut body.wounds)
            .into_iter()
            .partition(|wound| !is_kept(wound.part));
        body.wounds = rest;
        work.gone.extend(lost.iter().map(Wound::id));

        // The new tree.
        let mut keys: Vec<PartKey> = Vec::with_capacity(layout.parts.len());
        for key in &kept {
            keys.push(match key {
                Some(key) => *key,
                None => {
                    let key = PartKey::new(body.next_part);
                    body.next_part = body.next_part.saturating_add(1);
                    key
                }
            });
        }
        let mut parts: Vec<PartInst> = Vec::with_capacity(layout.parts.len());
        for (index, planned) in layout.parts.iter().enumerate() {
            let (parent, relation) = match planned.parent {
                Some((parent, relation)) => (Some(keys[parent]), relation),
                None => (None, Relation::On),
            };
            let part = match kept[index].and_then(|key| body.part(key)) {
                Some(old) => {
                    let mut part = old.clone();
                    let factor = planned.scale / old.scale;
                    for layer in &mut part.layers {
                        layer.thickness *= factor;
                    }
                    part.parent = parent;
                    part.relation = relation;
                    part.socket.clone_from(&planned.socket);
                    part.side.clone_from(&planned.side);
                    part.scale = planned.scale;
                    part
                }
                None => PartInst {
                    key: keys[index],
                    name: planned.name.clone(),
                    def: planned.def,
                    parent,
                    relation,
                    socket: planned.socket.clone(),
                    side: planned.side.clone(),
                    scale: planned.scale,
                    layers: defs.part(planned.def).map_or_else(Vec::new, |def| {
                        fresh_layers(def, body.scale * planned.scale)
                    }),
                    state: PartState::Whole,
                    held: false,
                },
            };
            parts.push(part);
        }
        parts.sort_by_key(PartInst::key);
        body.parts = parts;
        body.plan = plan;
        // A part on or in one that is gone is gone with it.
        loop {
            let orphan = body.parts.iter().position(|part| {
                part.is_present()
                    && part
                        .parent
                        .and_then(|parent| body.part(parent))
                        .is_some_and(|parent| !parent.is_present())
            });
            let Some(at) = orphan else { break };
            let part = &mut body.parts[at];
            part.state = PartState::Severed;
            part.layers.clear();
            part.held = false;
            let key = part.key;
            let (gone, rest): (Vec<Wound>, Vec<Wound>) = core::mem::take(&mut body.wounds)
                .into_iter()
                .partition(|wound| wound.part == key);
            body.wounds = rest;
            work.gone.extend(gone.iter().map(Wound::id));
        }
        Ok(())
    })?;
    Ok((detached, changes))
}
