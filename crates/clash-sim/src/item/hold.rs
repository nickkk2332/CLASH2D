//! Holding (06 §9.2): how a thing inside another is held there, and the
//! functions that move things between holds.
//!
//! Containment is the kernel's: a held thing's place is
//! [`Place::In`] its holder. The [`Holds`] component of the holder says
//! how: in a grasp, worn, stowed. A thing inside another with no record
//! is [`Hold::Inside`]: enclosed, as whatever the kernel alone put there
//! is.
//!
//! # What each function checks
//!
//! | Function | Refuses |
//! |----------|---------|
//! | [`take`], [`give`] | A thing with no `grip`; a holder with no body, or with too few free slots among its working parts |
//! | [`wear`] | An item with no `wear`; a wearer with no body, or with no part the item goes on; a part already covered at that layer |
//! | [`put_in`], [`stow`] | A container with no `holds`; a thing it does not accept; a thing that does not fit by volume or by mass |
//! | [`drop`], [`take_off`], [`take_out`] | A thing that is not held, not worn, not contained |
//! | All of them | What [`Sim::relocate`] refuses: an unknown entity, a thing put inside itself, nesting too deep |
//!
//! Each raises the hooks `item_leaving` and `item_taking` before it
//! changes anything, and a handler may cancel either. None asks how far
//! away the thing is.

use clash_core::{EntityId, Real, StateHash, StateHasher, impl_state_hash};
use clash_data::TagTable;
use clash_world::Cause;
use serde::{Deserialize, Serialize};

use super::{
    ITEM_DROPPED, ITEM_LEAVING, ITEM_REMOVED, ITEM_TAKEN, ITEM_TAKING, ITEM_WORN, Item, ItemDef,
    PartSelector, confer, def_of, latent, reason, refuse, stack,
};
use crate::Sim;
use crate::action::{ChannelDef, ChannelId};
use crate::body::{Body, BodyPartDef, PartInst, PartKey};
use crate::bus::EventPayload;
use crate::component::Physical;
use crate::entity::{KernelEntity, MAX_CONTAINMENT_DEPTH, Place};
use crate::event::{Subject, Where};
use crate::reason::Reason;

/// Slots of one channel, supplied by one part, that a grasp occupies.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub struct GraspSlot {
    /// The part that supplies them.
    pub part: PartKey,
    /// The channel.
    pub channel: ChannelId,
    /// How many.
    pub count: u16,
}

impl_state_hash!(GraspSlot {
    part,
    channel,
    count
});

/// One part a worn item covers, and at which layer.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub struct WornOn {
    /// The part covered.
    pub part: PartKey,
    /// The layer: a higher one is further out.
    pub layer: i16,
    /// Which entry of the item's `wear` covers it.
    pub entry: u8,
}

impl_state_hash!(WornOn { part, layer, entry });

/// How a thing is held by what it is inside (06 §9.2).
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Hold {
    /// In a hand or the like: it occupies these slots of the holder's
    /// parts. Pieces split from a grasped stack share its slots.
    Grasp(Vec<GraspSlot>),
    /// Covering parts: an outer layer for hits on them.
    Worn(Vec<WornOn>),
    /// Inside a container's capacity.
    Stowed,
    /// Riding or seated (11), in this seat.
    Seat(u16),
    /// Enclosed: a person in a locker, something swallowed. What is
    /// inside another with no record is held this way.
    Inside,
    /// Attached: a mounted device, an implant.
    Fixed,
}

impl Hold {
    /// The word rules read: `grasp`, `worn`, `stowed`, `seat`, `inside`
    /// or `fixed`.
    pub const fn name(&self) -> &'static str {
        match self {
            Hold::Grasp(_) => "grasp",
            Hold::Worn(_) => "worn",
            Hold::Stowed => "stowed",
            Hold::Seat(_) => "seat",
            Hold::Inside => "inside",
            Hold::Fixed => "fixed",
        }
    }

    /// Whether a thing held this way confers what its definition says:
    /// it is in a grasp or worn.
    pub const fn in_use(&self) -> bool {
        matches!(self, Hold::Grasp(_) | Hold::Worn(_))
    }

    /// Whether the holder carries the thing about, as opposed to
    /// enclosing or seating it. What a carried thing lets go of falls
    /// past its holder, to where the holder's own carrier is.
    pub const fn carries(&self) -> bool {
        matches!(
            self,
            Hold::Grasp(_) | Hold::Worn(_) | Hold::Stowed | Hold::Fixed
        )
    }
}

impl StateHash for Hold {
    fn state_hash(&self, hasher: &mut StateHasher) {
        match self {
            Hold::Grasp(slots) => {
                hasher.write_u8(0);
                slots.state_hash(hasher);
            }
            Hold::Worn(on) => {
                hasher.write_u8(1);
                on.state_hash(hasher);
            }
            Hold::Stowed => hasher.write_u8(2),
            Hold::Seat(seat) => {
                hasher.write_u8(3);
                hasher.write_u16(*seat);
            }
            Hold::Inside => hasher.write_u8(4),
            Hold::Fixed => hasher.write_u8(5),
        }
    }
}

/// One thing a holder holds, and how.
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
struct HoldEntry {
    entity: EntityId,
    hold: Hold,
}

impl_state_hash!(HoldEntry { entity, hold });

/// How an entity holds what is inside it (06 §9.2): the `Holds`
/// component.
///
/// It lists the things held in some way other than [`Hold::Inside`], in
/// order of id, and the tags the holder carries only because something
/// it holds confers them. An entity that holds nothing in particular
/// has no `Holds`.
///
/// Its fields change only through the functions of this module.
#[derive(Clone, PartialEq, Eq, Debug, Default, Serialize, Deserialize)]
pub struct Holds {
    /// What is held, in order of id.
    entries: Vec<HoldEntry>,
    /// The tags the holder has because of what it holds, in order.
    conferred_tags: Vec<String>,
}

impl_state_hash!(Holds {
    entries,
    conferred_tags
});

impl Holds {
    fn at(&self, entity: EntityId) -> Result<usize, usize> {
        self.entries
            .binary_search_by_key(&entity, |entry| entry.entity)
    }

    /// How a thing is held, if it is held in some way other than
    /// [`Hold::Inside`].
    pub fn get(&self, entity: EntityId) -> Option<&Hold> {
        self.at(entity).ok().map(|at| &self.entries[at].hold)
    }

    /// Everything held in some way other than [`Hold::Inside`], in
    /// order of id.
    pub fn iter(&self) -> impl Iterator<Item = (EntityId, &Hold)> {
        self.entries.iter().map(|entry| (entry.entity, &entry.hold))
    }

    /// The tags the holder has because of what it holds, in order.
    pub fn conferred_tags(&self) -> &[String] {
        &self.conferred_tags
    }

    pub(super) fn conferred_tags_mut(&mut self) -> &mut Vec<String> {
        &mut self.conferred_tags
    }

    fn set(&mut self, entity: EntityId, hold: Hold) {
        match self.at(entity) {
            Ok(at) => self.entries[at].hold = hold,
            Err(at) => self.entries.insert(at, HoldEntry { entity, hold }),
        }
    }

    fn remove(&mut self, entity: EntityId) -> Option<Hold> {
        self.at(entity).ok().map(|at| self.entries.remove(at).hold)
    }

    fn is_empty(&self) -> bool {
        self.entries.is_empty() && self.conferred_tags.is_empty()
    }
}

// ---- Reading -----------------------------------------------------------------

/// What an entity is directly inside, if it is inside anything. Truth.
pub fn holder_of(sim: &Sim, entity: EntityId) -> Option<EntityId> {
    sim.entities.get(entity)?.place().container()
}

/// How an entity is held by what it is inside. `None` for one that is
/// inside nothing. Truth.
pub fn hold_of(sim: &Sim, entity: EntityId) -> Option<Hold> {
    let holder = holder_of(sim, entity)?;
    Some(
        sim.components
            .get::<Holds>(holder)
            .and_then(|holds| holds.get(entity))
            .cloned()
            .unwrap_or(Hold::Inside),
    )
}

/// The slots each working part of a body supplies: part, channel and
/// how many, in order of part.
fn supply(sim: &Sim, body: &Body) -> Vec<(PartKey, ChannelId, u16)> {
    let Some(parts) = sim.defs.try_registry::<BodyPartDef>() else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for part in body.parts() {
        if !part.is_working() {
            continue;
        }
        if let Some(def) = parts.try_get(part.def()) {
            for (channel, count) in &def.channels {
                out.push((part.key(), channel.id(), *count));
            }
        }
    }
    out
}

/// The slots of one channel on one part that the holder's grasps
/// occupy, leaving out those of `except`.
fn occupied(
    sim: &Sim,
    holder: EntityId,
    part: PartKey,
    channel: ChannelId,
    except: Option<EntityId>,
) -> u16 {
    let Some(holds) = sim.components.get::<Holds>(holder) else {
        return 0;
    };
    let mut total: u16 = 0;
    for (entity, hold) in holds.iter() {
        if Some(entity) == except {
            continue;
        }
        if let Hold::Grasp(slots) = hold {
            for slot in slots {
                if slot.part == part && slot.channel == channel {
                    total = total.saturating_add(slot.count);
                }
            }
        }
    }
    total
}

/// How many slots of a channel the working parts of an entity's body
/// supply that no grasp occupies. Zero for something with no body.
pub fn free_grasp(sim: &Sim, who: EntityId, channel: ChannelId) -> u16 {
    let Some(body) = sim.components.get::<Body>(who) else {
        return 0;
    };
    let mut free: u16 = 0;
    for (part, supplied_channel, supplied) in supply(sim, body) {
        if supplied_channel == channel {
            free = free
                .saturating_add(supplied.saturating_sub(occupied(sim, who, part, channel, None)));
        }
    }
    free
}

fn no_body(entity: EntityId) -> Reason {
    refuse(
        reason::NO_BODY,
        format!("{entity} has no body to hold or wear anything with"),
    )
    .with("entity", entity.to_string())
}

/// Finds free slots for a grip among the working parts of a holder's
/// body, a preferred part first and then in order of part.
fn allocate(
    sim: &Sim,
    who: EntityId,
    grip: &[(ChannelId, u16)],
    prefer: Option<PartKey>,
    except: Option<EntityId>,
) -> Result<Vec<GraspSlot>, Reason> {
    let body = sim
        .components
        .get::<Body>(who)
        .ok_or_else(|| no_body(who))?;
    let supply = supply(sim, body);
    let mut slots = Vec::new();
    for &(channel, needed) in grip {
        let mut remaining = needed;
        let mut free_in_all: u32 = 0;
        let preferred = supply.iter().filter(|(part, _, _)| Some(*part) == prefer);
        let others = supply.iter().filter(|(part, _, _)| Some(*part) != prefer);
        for &(part, supplied_channel, supplied) in preferred.chain(others) {
            if supplied_channel != channel {
                continue;
            }
            let free = supplied.saturating_sub(occupied(sim, who, part, channel, except));
            free_in_all += u32::from(free);
            let taken = free.min(remaining);
            if taken > 0 {
                slots.push(GraspSlot {
                    part,
                    channel,
                    count: taken,
                });
                remaining -= taken;
            }
        }
        if remaining > 0 {
            let name = sim
                .defs
                .try_registry::<ChannelDef>()
                .map_or("", |channels| channels.name(channel));
            return Err(refuse(
                reason::NO_FREE_GRASP,
                format!(
                    "{who} has {free_in_all} free of the channel `{name}`, and this needs {needed}"
                ),
            )
            .with("holder", who.to_string())
            .with("channel", name)
            .with("needed", i64::from(needed))
            .with("free", i64::from(free_in_all)));
        }
    }
    slots.sort_by_key(|slot| (slot.part, slot.channel));
    Ok(slots)
}

/// The grip of a thing: what its definition says, in order of channel.
fn grip_of(sim: &Sim, what: EntityId) -> Vec<(ChannelId, u16)> {
    sim.components
        .get::<Item>(what)
        .and_then(|item| def_of(&sim.defs, item))
        .map(|def| {
            def.grip
                .iter()
                .map(|(channel, count)| (channel.id(), *count))
                .collect()
        })
        .unwrap_or_default()
}

/// Whether a part answers to one of a selector's names.
fn selects(
    selector: &PartSelector,
    part: &PartInst,
    parts: Option<&clash_data::Registry<BodyPartDef>>,
    tags: &TagTable,
) -> bool {
    let names = match selector {
        PartSelector::All => return true,
        PartSelector::Named(names) => names,
    };
    names.iter().any(|name| {
        if part.name() == name {
            return true;
        }
        let Some(parts) = parts else {
            return false;
        };
        let qualified = parts.name(part.def());
        let bare = qualified
            .rsplit_once(':')
            .map_or(qualified, |(_, bare)| bare);
        if qualified == name || bare == name {
            return true;
        }
        parts
            .try_get(part.def())
            .is_some_and(|def| tags.lookup(name).is_some_and(|tag| def.tags.contains(tag)))
    })
}

/// The parts of a wearer's body an item's `wear` covers, in order of
/// part and then layer.
fn wear_targets(sim: &Sim, who: EntityId, def: &ItemDef) -> Result<Vec<WornOn>, Reason> {
    let body = sim
        .components
        .get::<Body>(who)
        .ok_or_else(|| no_body(who))?;
    let parts = sim.defs.try_registry::<BodyPartDef>();
    let tags = sim.defs.tags();
    let mut out: Vec<WornOn> = Vec::new();
    for (index, entry) in def.wear.0.iter().enumerate() {
        let Ok(index) = u8::try_from(index) else {
            break;
        };
        for part in body.parts() {
            if !body.is_exposed(part.key()) || !selects(&entry.parts, part, parts, tags) {
                continue;
            }
            // The first entry to cover a part at a layer has it.
            if out
                .iter()
                .any(|on| on.part == part.key() && on.layer == entry.layer)
            {
                continue;
            }
            out.push(WornOn {
                part: part.key(),
                layer: entry.layer,
                entry: index,
            });
        }
    }
    out.sort_by_key(|on| (on.part, on.layer));
    Ok(out)
}

/// The name of a part of a holder's body, for a reason.
fn part_name(sim: &Sim, holder: EntityId, part: PartKey) -> String {
    sim.components
        .get::<Body>(holder)
        .and_then(|body| body.part(part))
        .map_or_else(|| part.raw().to_string(), |part| part.name().to_owned())
}

/// Where what a holder lets go of ends up: the holder's cell, or inside
/// whatever encloses the holder or its carrier.
pub(crate) fn ground_of(sim: &Sim, holder: EntityId) -> Place {
    let mut at = holder;
    for _ in 0..=MAX_CONTAINMENT_DEPTH {
        match sim.entities.get(at).map(KernelEntity::place) {
            Some(Place::At { loc, .. }) => return Place::at(loc),
            Some(Place::In(container)) => {
                if !hold_of(sim, at).is_some_and(|hold| hold.carries()) {
                    return Place::In(container);
                }
                at = container;
            }
            Some(Place::Stored) | None => return Place::Stored,
        }
    }
    Place::Stored
}

/// How many members an entity has: an item's count, or one.
fn count_of(sim: &Sim, entity: EntityId) -> i64 {
    sim.components
        .get::<Item>(entity)
        .map_or(1, |item| i64::from(item.count()))
}

// ---- The records -------------------------------------------------------------

/// Takes away a holder's `Holds` if it says nothing.
fn prune(sim: &mut Sim, holder: EntityId) {
    if sim
        .components
        .get::<Holds>(holder)
        .is_some_and(Holds::is_empty)
    {
        sim.components.remove::<Holds>(holder);
    }
}

/// Records how a holder holds something that is inside it, and applies
/// or takes back what the thing confers as it enters or leaves use.
pub(crate) fn set_entry(sim: &mut Sim, holder: EntityId, entity: EntityId, hold: Hold) {
    let was = sim
        .components
        .get::<Holds>(holder)
        .and_then(|holds| holds.get(entity))
        .is_some_and(Hold::in_use);
    let now = hold.in_use();
    if hold == Hold::Inside {
        if let Some(holds) = sim.components.get_mut::<Holds>(holder) {
            holds.remove(entity);
        }
    } else {
        sim.components
            .holds
            .get_or_insert_with(holder, Holds::default)
            .set(entity, hold);
    }
    match (was, now) {
        (false, true) => confer::add(sim, holder, entity),
        (true, false) => confer::remove(sim, holder, entity),
        _ => {}
    }
    prune(sim, holder);
}

/// What the kernel calls when an entity has left a container, by
/// whatever caller of [`Sim::relocate`]: the container forgets how it
/// held the entity, and what the entity conferred on it is taken back.
///
/// This is what keeps a hold record from naming something that is no
/// longer there, and what makes conferring end exactly when a thing is
/// put down (06 §9.2).
pub fn left_holder(sim: &mut Sim, entity: EntityId, holder: EntityId) {
    let Some(holds) = sim.components.get_mut::<Holds>(holder) else {
        return;
    };
    let Some(hold) = holds.remove(entity) else {
        return;
    };
    if hold.in_use() {
        confer::remove(sim, holder, entity);
    }
    prune(sim, holder);
}

// ---- Hooks and events --------------------------------------------------------

/// Where a thing was before a move.
struct Before {
    container: Option<EntityId>,
    hold: Option<Hold>,
}

impl Before {
    fn of(sim: &Sim, entity: EntityId) -> Result<Before, Reason> {
        sim.known(entity)?;
        Ok(Before {
            container: holder_of(sim, entity),
            hold: hold_of(sim, entity),
        })
    }

    fn worn(&self) -> bool {
        matches!(self.hold, Some(Hold::Worn(_)))
    }
}

/// Raises one of the two hooks. A handler's cancellation is the
/// refusal.
fn hook(
    sim: &mut Sim,
    name: &str,
    entity: EntityId,
    holder: EntityId,
    hold: &str,
    cause: Cause,
) -> Result<(), Reason> {
    let payload = EventPayload::new(name)
        .subject("self", Subject::Entity(entity))
        .subject("holder", Subject::Entity(holder))
        .with("hold", hold);
    match sim.raise(Where::Entity(entity), cause, payload) {
        Ok(raised) => raised.cancelled.map_or(Ok(()), Err),
        // The bus of this simulation does not know the hook: nobody
        // can be listening.
        Err(_) => Ok(()),
    }
}

/// Raises `item_leaving` for a thing that is in a hold.
fn leaving(sim: &mut Sim, entity: EntityId, before: &Before, cause: Cause) -> Result<(), Reason> {
    match (before.container, &before.hold) {
        (Some(holder), Some(hold)) => hook(sim, ITEM_LEAVING, entity, holder, hold.name(), cause),
        _ => Ok(()),
    }
}

/// Raises a notification of this module.
pub(crate) fn notify(sim: &mut Sim, about: EntityId, payload: EventPayload, cause: Cause) {
    // The payload is built to the registered type: a refusal means the
    // type is not registered with this simulation's bus.
    let _ = sim.raise(Where::Entity(about), cause, payload);
}

/// Says that a worn thing came off, if it was worn.
fn announce_removed(sim: &mut Sim, entity: EntityId, before: &Before, cause: Cause) {
    if let (true, Some(holder)) = (before.worn(), before.container) {
        let payload = EventPayload::new(ITEM_REMOVED)
            .subject("self", Subject::Entity(entity))
            .subject("holder", Subject::Entity(holder));
        notify(sim, entity, payload, cause);
    }
}

/// Says that a thing entered a hold other than a worn one.
fn announce_taken(
    sim: &mut Sim,
    entity: EntityId,
    holder: EntityId,
    hold: &str,
    before: &Before,
    cause: Cause,
) {
    announce_removed(sim, entity, before, cause);
    let mut payload = EventPayload::new(ITEM_TAKEN)
        .subject("self", Subject::Entity(entity))
        .subject("holder", Subject::Entity(holder));
    if let Some(from) = before.container.filter(|from| *from != holder) {
        payload = payload.subject("from", Subject::Entity(from));
    }
    payload = payload
        .with("hold", hold)
        .with("count", count_of(sim, entity));
    notify(sim, entity, payload, cause);
}

/// Whether a thing could be put inside another, by the kernel's rules.
fn check_inside(sim: &Sim, what: EntityId, container: EntityId) -> Result<(), Reason> {
    let footprint = sim.known(what)?.presence().footprint;
    sim.check_place(Some(what), footprint, Place::In(container))
        .map(|_| ())
}

// ---- Grasps ------------------------------------------------------------------

/// Takes a thing into a grasp of `who` (06 §9.2): picking it up, drawing
/// it from a pack, taking it from another's hand. The thing may be
/// anywhere. Returns the entity now grasped: the thing itself, or an
/// equal stack already in `who`'s grasp that it joined.
///
/// The grasp occupies the slots the thing's `grip` names, on the
/// working parts of `who`'s body in order of part. Taking what `who`
/// wears takes it off.
///
/// Refused, changing nothing, for a thing with no `grip`
/// (`item.no_grip`), a holder with no body (`item.no_body`) or with too
/// few free slots (`item.no_free_grasp`), whatever the kernel refuses of
/// a thing put inside another, and by a handler of `item_leaving` or
/// `item_taking`.
pub fn take(
    sim: &mut Sim,
    who: EntityId,
    what: EntityId,
    cause: Cause,
) -> Result<EntityId, Reason> {
    let grip = grip_of(sim, what);
    take_with(sim, who, what, &grip, None, cause)
}

/// [`take`] with the grip given, and a part to use first: for a hand
/// the taker chose, and for what is not an item, such as a creature
/// carried (07 §9.2).
pub fn take_with(
    sim: &mut Sim,
    who: EntityId,
    what: EntityId,
    grip: &[(ChannelId, u16)],
    prefer: Option<PartKey>,
    cause: Cause,
) -> Result<EntityId, Reason> {
    sim.known(who)?;
    let before = Before::of(sim, what)?;
    check_inside(sim, what, who)?;
    if grip.is_empty() {
        return Err(refuse(
            reason::NO_GRIP,
            format!("{what} has no grip: it cannot be held in a grasp"),
        )
        .with("item", what.to_string()));
    }
    if before.container == Some(who) && matches!(before.hold, Some(Hold::Grasp(_))) {
        return Ok(what);
    }
    // An equal stack already in hand takes more members without another
    // hand.
    if stack::grasped_equal(sim, who, what).is_none() {
        allocate(sim, who, grip, prefer, Some(what))?;
    }
    leaving(sim, what, &before, cause)?;
    hook(sim, ITEM_TAKING, what, who, "grasp", cause)?;
    // Worked out again: a handler may have changed what is held.
    let join = stack::grasped_equal(sim, who, what);
    let slots = match join.and_then(|stack| hold_of(sim, stack)) {
        Some(Hold::Grasp(shared)) => shared,
        _ => allocate(sim, who, grip, prefer, Some(what))?,
    };
    sim.relocate(what, Place::In(who), cause)?;
    set_entry(sim, who, what, Hold::Grasp(slots));
    announce_taken(sim, what, who, "grasp", &before, cause);
    Ok(match join {
        Some(into) if stack::merge(sim, what, into, cause).is_ok() => into,
        _ => what,
    })
}

/// Hands a held thing to another holder's grasp (06 §9.2): [`take`] by
/// `to`. Refused with `item.not_held` for a thing that is inside
/// nothing.
pub fn give(sim: &mut Sim, what: EntityId, to: EntityId, cause: Cause) -> Result<EntityId, Reason> {
    let before = Before::of(sim, what)?;
    if before.container.is_none() {
        return Err(not_held(what));
    }
    take(sim, to, what, cause)
}

fn not_held(what: EntityId) -> Reason {
    refuse(reason::NOT_HELD, format!("{what} is not held by anything"))
        .with("item", what.to_string())
}

/// Puts a held thing down where its holder is (06 §9.2): in the
/// holder's cell, or inside whatever encloses the holder. Returns the
/// entity that now lies there: the thing itself, or an equal stack it
/// joined.
///
/// Refused with `item.not_held` for a thing that is inside nothing, and
/// by a handler of `item_leaving`.
pub fn drop(sim: &mut Sim, what: EntityId, cause: Cause) -> Result<EntityId, Reason> {
    put_down(sim, what, cause, false)
}

/// [`drop`] that no hook can stop: what a part that can no longer hold
/// was holding.
pub(crate) fn drop_forced(sim: &mut Sim, what: EntityId, cause: Cause) -> Result<EntityId, Reason> {
    put_down(sim, what, cause, true)
}

fn put_down(sim: &mut Sim, what: EntityId, cause: Cause, forced: bool) -> Result<EntityId, Reason> {
    let before = Before::of(sim, what)?;
    let Some(holder) = before.container else {
        return Err(not_held(what));
    };
    let to = ground_of(sim, holder);
    let footprint = sim.known(what)?.presence().footprint;
    sim.check_place(Some(what), footprint, to)?;
    if !forced {
        leaving(sim, what, &before, cause)?;
    }
    sim.relocate(what, to, cause)?;
    announce_removed(sim, what, &before, cause);
    let payload = EventPayload::new(ITEM_DROPPED)
        .subject("self", Subject::Entity(what))
        .subject("holder", Subject::Entity(holder))
        .with("count", count_of(sim, what));
    notify(sim, what, payload, cause);
    Ok(stack::rejoin(sim, what, cause))
}

// ---- Wearing -----------------------------------------------------------------

/// Puts an item on `who` (06 §9.2): it covers the parts its `wear`
/// selects and is a layer for hits on them. The item may be anywhere.
/// One member of a stack is split off and worn. Returns the entity
/// worn.
///
/// Refused, changing nothing, for an item with no `wear`
/// (`item.not_wearable`), a wearer with no body (`item.no_body`) or with
/// no outer part the item goes on (`item.no_part_to_wear_on`), a part
/// that already wears something at that layer (`item.layer_taken`),
/// whatever the kernel refuses, and by a handler of `item_leaving` or
/// `item_taking`.
pub fn wear(
    sim: &mut Sim,
    who: EntityId,
    what: EntityId,
    cause: Cause,
) -> Result<EntityId, Reason> {
    sim.known(who)?;
    let before = Before::of(sim, what)?;
    let item = super::item_of(sim, what)?;
    let count = item.count();
    let defs = std::sync::Arc::clone(&sim.defs);
    let def = def_of(&defs, item)
        .filter(|def| def.is_wearable())
        .ok_or_else(|| {
            refuse(
                reason::NOT_WEARABLE,
                format!("{what} is not something that can be worn"),
            )
            .with("item", what.to_string())
        })?;
    check_inside(sim, what, who)?;
    let targets = wear_targets(sim, who, def)?;
    if targets.is_empty() {
        return Err(refuse(
            reason::NO_PART,
            format!("{who} has no part that {what} goes on"),
        )
        .with("item", what.to_string())
        .with("wearer", who.to_string()));
    }
    if let Some(holds) = sim.components.get::<Holds>(who) {
        for (other, hold) in holds.iter() {
            let Hold::Worn(on) = hold else { continue };
            if other == what {
                continue;
            }
            let clash = on.iter().find(|have| {
                targets
                    .iter()
                    .any(|want| want.part == have.part && want.layer == have.layer)
            });
            if let Some(clash) = clash {
                let part = part_name(sim, who, clash.part);
                return Err(refuse(
                    reason::LAYER_TAKEN,
                    format!(
                        "the part `{part}` of {who} already wears {other} at layer {}",
                        clash.layer
                    ),
                )
                .with("item", what.to_string())
                .with("part", part)
                .with("layer", i64::from(clash.layer))
                .with("by", other.to_string()));
            }
        }
    }
    if before.container == Some(who) && before.hold == Some(Hold::Worn(targets.clone())) {
        return Ok(what);
    }
    leaving(sim, what, &before, cause)?;
    hook(sim, ITEM_TAKING, what, who, "worn", cause)?;
    // One of a stack is worn; the rest stay where they are.
    let (what, before) = if count > 1 {
        let piece = stack::split(sim, what, 1, cause)?;
        (piece, Before::of(sim, piece)?)
    } else {
        (what, before)
    };
    sim.relocate(what, Place::In(who), cause)?;
    let parts = targets.len();
    set_entry(sim, who, what, Hold::Worn(targets));
    let payload = EventPayload::new(ITEM_WORN)
        .subject("self", Subject::Entity(what))
        .subject("holder", Subject::Entity(who))
        .with("parts", i64::try_from(parts).unwrap_or(i64::MAX));
    let _ = before;
    notify(sim, what, payload, cause);
    Ok(what)
}

/// Takes a worn item off into its wearer's grasp (06 §9.2): [`take`] by
/// the wearer. To take it off and put it down, [`drop`] it. Refused
/// with `item.not_worn` for an item that is not worn.
pub fn take_off(sim: &mut Sim, what: EntityId, cause: Cause) -> Result<EntityId, Reason> {
    let before = Before::of(sim, what)?;
    match (before.container, before.worn()) {
        (Some(wearer), true) => take(sim, wearer, what, cause),
        _ => {
            Err(refuse(reason::NOT_WORN, format!("{what} is not worn"))
                .with("item", what.to_string()))
        }
    }
}

// ---- Containers --------------------------------------------------------------

fn not_a_container(container: EntityId) -> Reason {
    refuse(
        reason::NOT_A_CONTAINER,
        format!("{container} is not a container"),
    )
    .with("container", container.to_string())
}

/// The volume a thing takes up and what it weighs with everything
/// inside it, as its `Physical` has them.
fn measures(sim: &Sim, what: EntityId) -> (Real, Real) {
    sim.components
        .get::<Physical>(what)
        .map_or((Real::ZERO, Real::ZERO), |physical| {
            (physical.volume().get(), physical.mass().get())
        })
}

/// Whether a container would take a thing: it accepts one of the
/// thing's tags and has room for it by volume and by mass.
pub(crate) fn fits(sim: &Sim, container: EntityId, what: EntityId) -> Result<(), Reason> {
    let holds = sim
        .components
        .get::<Item>(container)
        .and_then(|item| def_of(&sim.defs, item))
        .and_then(|def| def.holds.as_ref())
        // A device holds what its definition says (16 §4.2).
        .or_else(|| crate::device::holds_of(sim, container))
        .ok_or_else(|| not_a_container(container))?;
    if !holds.accepts.is_empty() {
        let tags = &sim.known(what)?.tags;
        if !holds.accepts.iter().any(|tag| tags.contains(tag)) {
            return Err(refuse(
                reason::NOT_ACCEPTED,
                format!("{container} does not take things like {what}"),
            )
            .with("item", what.to_string())
            .with("container", container.to_string()));
        }
    }
    let (volume, mass) = measures(sim, what);
    let mut used_volume = Real::ZERO;
    let mut used_mass = Real::ZERO;
    for inside in sim.entities.contents(container) {
        if *inside == what {
            continue;
        }
        let (inside_volume, inside_mass) = measures(sim, *inside);
        used_volume += inside_volume;
        used_mass += inside_mass;
    }
    let too_much = |what_measure: &'static str, needed: Real, room: Real| {
        refuse(
            reason::DOES_NOT_FIT,
            format!("{what} needs {needed} of {what_measure} and {container} has room for {room}"),
        )
        .with("item", what.to_string())
        .with("container", container.to_string())
        .with("what", what_measure)
        .with("needed", needed)
        .with("room", room)
    };
    if let Some(limit) = holds.volume
        && used_volume + volume > limit.get()
    {
        return Err(too_much(
            "volume",
            volume,
            (limit.get() - used_volume).max(Real::ZERO),
        ));
    }
    if let Some(limit) = holds.mass
        && used_mass + mass > limit.get()
    {
        return Err(too_much(
            "mass",
            mass,
            (limit.get() - used_mass).max(Real::ZERO),
        ));
    }
    Ok(())
}

/// Puts a thing into a container (06 §9.2): an item is stowed in its
/// capacity, anything else is enclosed. The thing may be anywhere.
/// Returns the entity now in the container: the thing itself, or an
/// equal stack there that it joined.
///
/// A container whose contents have not been made yet is opened first
/// (06 §9.3), so that its room is known.
///
/// Refused for a container with no `holds` (`item.not_a_container`), a
/// thing it does not accept (`item.not_accepted`), a thing that does not
/// fit by volume or by mass (`item.does_not_fit`), whatever the kernel
/// refuses, and by a handler of `item_leaving` or `item_taking`.
pub fn put_in(
    sim: &mut Sim,
    what: EntityId,
    container: EntityId,
    cause: Cause,
) -> Result<EntityId, Reason> {
    sim.known(container)?;
    let before = Before::of(sim, what)?;
    check_inside(sim, what, container)?;
    fits(sim, container, what)?;
    let hold = if sim.components.get::<Item>(what).is_some() {
        Hold::Stowed
    } else {
        Hold::Inside
    };
    if before.container == Some(container) && before.hold.as_ref() == Some(&hold) {
        return Ok(what);
    }
    if latent::is_latent(sim, container) {
        latent::materialise(sim, container, cause)?;
        fits(sim, container, what)?;
    }
    leaving(sim, what, &before, cause)?;
    hook(sim, ITEM_TAKING, what, container, hold.name(), cause)?;
    sim.relocate(what, Place::In(container), cause)?;
    let name = hold.name();
    set_entry(sim, container, what, hold);
    announce_taken(sim, what, container, name, &before, cause);
    Ok(stack::rejoin(sim, what, cause))
}

/// Puts a thing into the first container `who` wears or holds that
/// would take it, in order of id (06 §9.2). Refused with
/// `item.no_container` if none would.
pub fn stow(
    sim: &mut Sim,
    who: EntityId,
    what: EntityId,
    cause: Cause,
) -> Result<EntityId, Reason> {
    sim.known(who)?;
    sim.known(what)?;
    let carried: Vec<EntityId> = sim.entities.contents(who).to_vec();
    for container in carried {
        if container == what || !hold_of(sim, container).is_some_and(|hold| hold.in_use()) {
            continue;
        }
        if check_inside(sim, what, container).is_ok() && fits(sim, container, what).is_ok() {
            return put_in(sim, what, container, cause);
        }
    }
    Err(refuse(
        reason::NO_CONTAINER,
        format!("{who} wears and holds nothing that would take {what}"),
    )
    .with("who", who.to_string())
    .with("item", what.to_string()))
}

/// Takes a thing out of the container it is stowed in or enclosed by
/// (06 §9.2): into `to`'s grasp if one is named, as [`take`] does, and
/// otherwise to where the container is, as [`drop`] does. Refused with
/// `item.not_contained` for a thing that is not in a container.
pub fn take_out(
    sim: &mut Sim,
    what: EntityId,
    to: Option<EntityId>,
    cause: Cause,
) -> Result<EntityId, Reason> {
    let before = Before::of(sim, what)?;
    if !matches!(before.hold, Some(Hold::Stowed | Hold::Inside)) {
        return Err(refuse(
            reason::NOT_CONTAINED,
            format!("{what} is not in a container"),
        )
        .with("item", what.to_string()));
    }
    match to {
        Some(to) => take(sim, to, what, cause),
        None => drop(sim, what, cause),
    }
}

/// Puts a thing inside another and records how it is held, with no
/// check of capacity, grasp or wear and no hook: for the packages that
/// seat riders and fix devices, and for setting a scene. What the
/// kernel refuses of the move is refused.
pub fn place_in(
    sim: &mut Sim,
    what: EntityId,
    container: EntityId,
    hold: Hold,
    cause: Cause,
) -> Result<(), Reason> {
    sim.relocate(what, Place::In(container), cause)?;
    set_entry(sim, container, what, hold);
    Ok(())
}

// ---- What a body's changes do ------------------------------------------------

/// Drops what a holder grasps with a part that can no longer hold.
pub(crate) fn let_go(sim: &mut Sim, holder: EntityId, part: PartKey, cause: Cause) {
    let Some(holds) = sim.components.get::<Holds>(holder) else {
        return;
    };
    let held: Vec<EntityId> = holds
        .iter()
        .filter(|(_, hold)| {
            matches!(hold, Hold::Grasp(slots) if slots.iter().any(|slot| slot.part == part))
        })
        .map(|(entity, _)| entity)
        .collect();
    for entity in held {
        // A thing that cannot be put down where its holder is stays
        // recorded: there is nowhere for it to fall.
        let _ = drop_forced(sim, entity, cause);
    }
}

/// Brings what a holder wears into line with the parts it still has:
/// an item no longer covers a part that is gone, and one left covering
/// nothing goes into `into` (the part that came off) or is dropped.
pub(crate) fn uncover(
    sim: &mut Sim,
    holder: EntityId,
    gone: &dyn Fn(PartKey) -> bool,
    into: Option<EntityId>,
    cause: Cause,
) {
    let Some(holds) = sim.components.get::<Holds>(holder) else {
        return;
    };
    let worn: Vec<(EntityId, Vec<WornOn>)> = holds
        .iter()
        .filter_map(|(entity, hold)| match hold {
            Hold::Worn(on) if on.iter().any(|on| gone(on.part)) => Some((entity, on.clone())),
            _ => None,
        })
        .collect();
    for (entity, on) in worn {
        let left: Vec<WornOn> = on.into_iter().filter(|on| !gone(on.part)).collect();
        if !left.is_empty() {
            set_entry(sim, holder, entity, Hold::Worn(left));
            continue;
        }
        let went = match into {
            Some(part) => sim.relocate(entity, Place::In(part), cause).is_ok(),
            None => false,
        };
        if !went {
            let _ = drop_forced(sim, entity, cause);
        }
    }
}
