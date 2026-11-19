//! Stacks (06 §9.1): equal items of a definition that stacks may share
//! one entity with a count.
//!
//! Two items are **equal** when they were made from the same definition
//! and agree in material, damage, colour, tags, variables, name and
//! description, and neither holds anything or has contents still to be
//! made. Whatever would change one member of a stack splits it off
//! first ([`split`]); equal items in the same place and the same kind
//! of hold may join again ([`merge`], [`rejoin`]). Counts are conserved
//! throughout, and so is mass: a stack weighs one member's mass times
//! its count.

use clash_core::EntityId;
use clash_world::{Cause, OccupantFilter, OccupantKind};

use super::hold::{Hold, Holds, hold_of, notify, place_beside, set_entry, store_order};
use super::make::{ITEM_STREAM, set_count};
use super::{ITEM_MERGED, ITEM_SPLIT, Item, def_of, item_of, reason, refuse};
use crate::Sim;
use crate::bus::EventPayload;
use crate::component::Identity;
use crate::entity::{EntitySpec, Place};
use crate::event::Subject;
use crate::reason::Reason;

/// Whether an entity is an item of a definition that stacks.
fn stacks(sim: &Sim, entity: EntityId) -> bool {
    sim.components
        .get::<Item>(entity)
        .and_then(|item| def_of(&sim.defs, item))
        .is_some_and(|def| def.stack)
}

/// Whether two items are equal in every respect a stack's members must
/// be (06 §9.1).
pub(crate) fn equal(sim: &Sim, a: EntityId, b: EntityId) -> bool {
    let (Some(first), Some(second)) =
        (sim.components.get::<Item>(a), sim.components.get::<Item>(b))
    else {
        return false;
    };
    let (Some(one), Some(other)) = (sim.entities.get(a), sim.entities.get(b)) else {
        return false;
    };
    a != b
        && stacks(sim, a)
        && first.def == second.def
        && first.material == second.material
        && first.damage == second.damage
        && first.color == second.color
        && !first.latent
        && !second.latent
        && first.remains.is_none()
        && second.remains.is_none()
        && one.tags == other.tags
        && one.vars == other.vars
        && sim.entities.contents(a).is_empty()
        && sim.entities.contents(b).is_empty()
        && sim.components.get::<Identity>(a) == sim.components.get::<Identity>(b)
        && sim.components.get::<crate::substance::Substances>(a)
            == sim.components.get::<crate::substance::Substances>(b)
}

/// Whether two entities are in the same place and held the same way.
fn together(sim: &Sim, a: EntityId, b: EntityId) -> bool {
    let (Some(one), Some(other)) = (sim.entities.get(a), sim.entities.get(b)) else {
        return false;
    };
    one.place() == other.place()
        && hold_of(sim, a).map(|hold| hold.name()) == hold_of(sim, b).map(|hold| hold.name())
}

/// An equal stack that `who` holds in a grasp, for `what` to join.
pub(crate) fn grasped_equal(sim: &Sim, who: EntityId, what: EntityId) -> Option<EntityId> {
    sim.components
        .get::<Holds>(who)?
        .iter()
        .find(|(entity, hold)| matches!(hold, Hold::Grasp(_)) && equal(sim, what, *entity))
        .map(|(entity, _)| entity)
}

/// Splits `count` members off a stack as an entity of their own, in the
/// same place and the same hold (06 §9.1). A piece split from a grasped
/// stack shares its grasp. Returns the piece.
///
/// Refused with `item.bad_count` unless at least one member goes and at
/// least one stays, and with `item.not_an_item` for anything else.
pub fn split(sim: &mut Sim, item: EntityId, count: u32, cause: Cause) -> Result<EntityId, Reason> {
    let whole = item_of(sim, item)?.clone();
    if count == 0 || count >= whole.count {
        return Err(refuse(
            reason::BAD_COUNT,
            format!(
                "{count} cannot be split off a stack of {}: at least one goes and one stays",
                whole.count
            ),
        )
        .with("count", i64::from(count))
        .with("have", i64::from(whole.count)));
    }
    let entity = sim.known(item)?;
    let (place, hold) = (entity.place(), hold_of(sim, item));
    let spec = EntitySpec {
        presence: *entity.presence(),
        tags: entity.tags.clone(),
        vars: entity.vars.clone(),
        control: entity.control(),
    };
    let piece = sim.create_entity(spec, cause)?;
    let seed = sim.rng.stream(ITEM_STREAM).next_u64();
    sim.components.insert(
        piece,
        Item {
            count,
            seed,
            latent: false,
            ..whole.clone()
        },
    );
    if let Some(identity) = sim.components.get::<Identity>(item).cloned() {
        sim.components.insert(piece, identity);
    }
    // Each member holds what the others do (16 §9.5).
    if let Some(held) = sim
        .components
        .get::<crate::substance::Substances>(item)
        .cloned()
    {
        sim.components.insert(piece, held);
    }
    let undo = |sim: &mut Sim, reason: Reason| {
        let _ = sim.remove_entity(piece, cause);
        let _ = set_count(sim, item, whole.count, cause);
        reason
    };
    if let Err(reason) = set_count(sim, item, whole.count - count, cause) {
        return Err(undo(sim, reason));
    }
    if let Err(reason) = set_count(sim, piece, count, cause) {
        return Err(undo(sim, reason));
    }
    if place != Place::Stored
        && let Err(reason) = sim.relocate(piece, place, cause)
    {
        return Err(undo(sim, reason));
    }
    if let (Place::In(holder), Some(hold)) = (place, hold) {
        set_entry(sim, holder, piece, hold);
        // In a store a piece keeps the place of the stack it came from.
        place_beside(sim, holder, piece, item);
    }
    let payload = EventPayload::new(ITEM_SPLIT)
        .subject("self", Subject::Entity(item))
        .subject("piece", Subject::Entity(piece))
        .with("count", i64::from(count));
    notify(sim, item, payload, cause);
    Ok(piece)
}

/// Joins an item to an equal one in the same place and the same kind of
/// hold (06 §9.1): `into` gains its members and `item` is gone.
///
/// Refused with `item.not_equal` unless the two are equal in every
/// respect and together, and with `item.bad_count` if the stack would
/// hold more than a count can say.
pub fn merge(sim: &mut Sim, item: EntityId, into: EntityId, cause: Cause) -> Result<(), Reason> {
    let joining = item_of(sim, item)?.count;
    let have = item_of(sim, into)?.count;
    if !equal(sim, item, into) || !together(sim, item, into) {
        return Err(refuse(
            reason::NOT_EQUAL,
            format!("{item} and {into} are not equal items in the same hold"),
        )
        .with("item", item.to_string())
        .with("into", into.to_string()));
    }
    let Some(total) = have.checked_add(joining) else {
        return Err(refuse(reason::BAD_COUNT, "a stack cannot hold that many")
            .with("count", i64::from(joining))
            .with("have", i64::from(have)));
    };
    sim.remove_entity(item, cause)?;
    set_count(sim, into, total, cause)?;
    let payload = EventPayload::new(ITEM_MERGED)
        .subject("self", Subject::Entity(into))
        .with("count", i64::from(joining));
    notify(sim, into, payload, cause);
    Ok(())
}

/// Joins an item to an equal one that is where it is, if there is one:
/// the first in order of id among what shares its container and kind of
/// hold, or its cell. In a store (07 §11.5) it joins only the stack put
/// in just before it, so that the store keeps its order. Returns the
/// entity that now holds its members: the stack it joined, or the item
/// itself.
pub fn rejoin(sim: &mut Sim, item: EntityId, cause: Cause) -> EntityId {
    if !stacks(sim, item) {
        return item;
    }
    let Some(place) = sim.entities.get(item).map(|entity| entity.place()) else {
        return item;
    };
    let found = match place {
        // A store keeps its order: what is put in joins only what was
        // put in just before it (07 §11.5).
        Place::In(container) if store_order(sim, container).is_some() => sim
            .components
            .get::<Holds>(container)
            .and_then(|holds| {
                let order = holds.order();
                let at = order.iter().position(|stowed| *stowed == item)?;
                at.checked_sub(1)
                    .and_then(|before| order.get(before))
                    .copied()
            })
            .filter(|other| equal(sim, item, *other) && together(sim, item, *other)),
        Place::In(container) => sim
            .entities
            .contents(container)
            .iter()
            .copied()
            .find(|other| equal(sim, item, *other) && together(sim, item, *other)),
        Place::At { loc, .. } => sim
            .cosmos
            .occupants_at(loc, &OccupantFilter::ANY.of_kind(OccupantKind::Item))
            .ok()
            .and_then(|found| {
                found
                    .into_iter()
                    .map(|occupied| occupied.id)
                    .find(|other| equal(sim, item, *other) && together(sim, item, *other))
            }),
        Place::Stored => None,
    };
    match found {
        Some(into) if merge(sim, item, into, cause).is_ok() => into,
        _ => item,
    }
}
