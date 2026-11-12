//! Ammunition, stores and what is fitted (07 §11.5): rounds as items, a
//! store's next round, the numbers of a shot through an item and what is
//! fitted to it, fitting, and loading.
//!
//! A **round** is an item whose definition has a `round` block. A
//! **store** is a container whose `holds` has an `order` (06 §9.1): it
//! gives up what is stowed in it in the order it was put in, first or
//! last. An item that is not a store fires from its **feed**: the first
//! store fitted to it, held `fixed` in it, in order of id. A pistol's
//! magazine is fitted to it; a quiver is worn; a revolver's cylinder is
//! the revolver.

use std::sync::Arc;

use clash_core::{EntityId, Real};
use clash_world::Cause;

use super::defs::{ProjectileDef, ProjectileId, RoundDef};
use super::reason;
use crate::Sim;
use crate::item::{self, Hold, Holds, Item, ItemDef};
use crate::reason::Reason;

/// The definition of an item, if it is one made from a definition.
pub fn item_def(sim: &Sim, entity: EntityId) -> Option<&ItemDef> {
    sim.components()
        .get::<Item>(entity)
        .and_then(|component| item::def_of(sim.defs(), component))
}

/// The `round` block of an item, if it is a round.
pub fn round_of(sim: &Sim, entity: EntityId) -> Option<&RoundDef> {
    item_def(sim, entity)?.round.as_ref()
}

/// The projectile a round fires.
pub fn round_projectile(sim: &Sim, entity: EntityId) -> Option<ProjectileId> {
    round_of(sim, entity).map(|round| round.projectile.id())
}

/// A projectile's definition.
pub fn projectile_def(defs: &Arc<clash_data::Defs>, id: ProjectileId) -> Option<&ProjectileDef> {
    defs.try_registry::<ProjectileDef>()?.try_get(id)
}

/// Whether an entity is a store: a container that keeps an order.
pub fn is_store(sim: &Sim, entity: EntityId) -> bool {
    item::store_order(sim, entity).is_some()
}

/// What an entity holds `fixed`, in order of id: what is fitted to it.
pub fn fitted(sim: &Sim, entity: EntityId) -> Vec<EntityId> {
    sim.components()
        .get::<Holds>(entity)
        .map(|holds| {
            holds
                .iter()
                .filter(|(_, hold)| matches!(hold, Hold::Fixed))
                .map(|(fixed, _)| fixed)
                .collect()
        })
        .unwrap_or_default()
}

/// The store an item fires from (07 §11.5): itself if it is a store,
/// else the first store fitted to it, in order of id.
pub fn feed_of(sim: &Sim, entity: EntityId) -> Option<EntityId> {
    if is_store(sim, entity) {
        return Some(entity);
    }
    fitted(sim, entity)
        .into_iter()
        .find(|fixed| is_store(sim, *fixed))
}

/// How many rounds a store holds: the members of what is stowed in it
/// that are rounds.
pub fn rounds_in(sim: &Sim, store: EntityId) -> u32 {
    let Some(holds) = sim.components().get::<Holds>(store) else {
        return 0;
    };
    holds
        .order()
        .iter()
        .filter(|stowed| round_of(sim, **stowed).is_some())
        .map(|stowed| sim.components().get::<Item>(*stowed).map_or(0, Item::count))
        .fold(0_u32, u32::saturating_add)
}

/// The round a store gives up next, if what it gives up next is a round.
pub fn next_round(sim: &Sim, store: EntityId) -> Option<EntityId> {
    item::next_in_store(sim, store).filter(|next| round_of(sim, *next).is_some())
}

/// Whether a container takes things that carry a tag: its `accepts`
/// lists it, or lists nothing.
fn accepts(sim: &Sim, container: EntityId, tag: &str) -> bool {
    let holds = item_def(sim, container)
        .and_then(|def| def.holds.as_ref())
        .or_else(|| crate::device::holds_of(sim, container));
    holds.is_some_and(|holds| holds.accepts.is_empty() || holds.accepts.iter().any(|t| t == tag))
}

/// The first store an entity has that takes things tagged `tag`
/// (07 §11.5): itself, then what it holds in any way but enclosed, in
/// order of id, and what is fitted to those. A quiver worn, a magazine
/// fitted to a held rifle, a belt.
pub fn store_of(sim: &Sim, who: EntityId, tag: &str) -> Option<EntityId> {
    let takes = |entity: EntityId| is_store(sim, entity) && accepts(sim, entity, tag);
    if takes(who) {
        return Some(who);
    }
    let holds = sim.components().get::<Holds>(who)?;
    for (held, _) in holds.iter() {
        if takes(held) {
            return Some(held);
        }
        if let Some(fixed) = fitted(sim, held).into_iter().find(|fixed| takes(*fixed)) {
            return Some(fixed);
        }
    }
    None
}

/// The first thing an entity holds in a grasp, in order of id, that
/// carries a tag: any thing, for an empty tag.
pub fn wielding(sim: &Sim, who: EntityId, tag: &str) -> Option<EntityId> {
    let holds = sim.components().get::<Holds>(who)?;
    holds
        .iter()
        .filter(|(_, hold)| matches!(hold, Hold::Grasp(_)))
        .map(|(held, _)| held)
        .find(|held| {
            tag.is_empty()
                || sim
                    .truth()
                    .entity(*held)
                    .is_some_and(|record| record.tags.contains(tag))
        })
}

/// One of the numbers of a shot through an item (07 §11.5): the product
/// of the item's own `shot` number of that name and those of everything
/// fitted to it that states it. `None` if neither the item nor anything
/// fitted states it.
pub fn shot_number(sim: &Sim, weapon: EntityId, name: &str) -> Option<Real> {
    let mut found: Option<Real> = None;
    let mut take = |entity: EntityId| {
        if let Some(value) = item_def(sim, entity)
            .and_then(|def| def.shot.as_ref())
            .and_then(|shot| shot.number(name))
        {
            found = Some(found.map_or(value, |have| have * value));
        }
    };
    take(weapon);
    for fixed in fitted(sim, weapon) {
        take(fixed);
    }
    found
}

/// Fits an item to another (07 §11.5): a scope to a rifle, a magazine
/// to a pistol. It is held `fixed` in it; taking it into a grasp takes
/// it off again. What it gives a shot through the other it gives while
/// it is fitted, and no longer.
///
/// Refused with `ranged.cannot_fit` for an item the other's `shot` does
/// not fit (none of its tags is in `fits`), and for one whose tag is
/// taken by something fitted already; and as the kernel refuses a thing
/// put inside another.
pub fn fit(
    sim: &mut Sim,
    what: EntityId,
    into: EntityId,
    cause: Cause,
) -> Result<EntityId, Reason> {
    sim.known(what)?;
    sim.known(into)?;
    let cannot = |why: &str| {
        Reason::new(
            reason::CANNOT_FIT,
            format!("{what} cannot be fitted to {into}: {why}"),
        )
        .with("item", what.to_string())
        .with("into", into.to_string())
    };
    let fits: Vec<String> = item_def(sim, into)
        .and_then(|def| def.shot.as_ref())
        .map(|shot| shot.fits.clone())
        .unwrap_or_default();
    let tags = sim
        .truth()
        .entity(what)
        .map(|record| record.tags.clone())
        .unwrap_or_default();
    let Some(slot) = fits.iter().find(|tag| tags.contains(tag.as_str())) else {
        return Err(cannot("nothing fits it there"));
    };
    if item::holder_of(sim, what) == Some(into)
        && matches!(item::hold_of(sim, what), Some(Hold::Fixed))
    {
        return Ok(what);
    }
    let taken = fitted(sim, into).into_iter().any(|fixed| {
        sim.truth()
            .entity(fixed)
            .is_some_and(|record| record.tags.contains(slot.as_str()))
    });
    if taken {
        return Err(cannot("something is fitted there already"));
    }
    item::hold::place_in(sim, what, into, Hold::Fixed, cause)?;
    Ok(what)
}

/// Loads rounds into a store (07 §11.5): one at a time, from `from`, a
/// container whose rounds are taken in order of id or a stack of rounds
/// itself; with none, from whatever `who` carries, at any depth, in the
/// order `carried` lists it, but never from the store. Only rounds the
/// store takes are loaded, and at most `count` of them; with no count,
/// as many as fit. `into` is the store, or an item whose feed it is.
/// Returns how many were loaded.
///
/// Refused with `ranged.not_a_store` for something with no feed, and
/// with `ranged.nothing_to_load`, or the reason the first round was
/// refused, when not one was loaded.
pub fn load(
    sim: &mut Sim,
    who: EntityId,
    into: EntityId,
    from: Option<EntityId>,
    count: Option<u32>,
    cause: Cause,
) -> Result<u32, Reason> {
    sim.known(who)?;
    let store = feed_of(sim, into).ok_or_else(|| {
        Reason::new(
            reason::NOT_A_STORE,
            format!("{into} is not a store and has none fitted"),
        )
        .with("store", into.to_string())
    })?;
    let limit = count.unwrap_or(u32::MAX);
    let mut loaded = 0_u32;
    let mut first_refusal = None;
    while loaded < limit {
        let Some(next) = next_source(sim, who, store, from) else {
            break;
        };
        let have = sim.components().get::<Item>(next).map_or(1, Item::count);
        let piece = if have > 1 {
            item::split(sim, next, 1, cause)?
        } else {
            next
        };
        match item::put_in(sim, piece, store, cause) {
            Ok(_) => loaded += 1,
            Err(refusal) => {
                // It goes back where it came from.
                if piece != next {
                    item::rejoin(sim, piece, cause);
                }
                first_refusal = Some(refusal);
                break;
            }
        }
    }
    if loaded == 0 {
        return Err(first_refusal.unwrap_or_else(|| {
            Reason::new(
                reason::NOTHING_TO_LOAD,
                format!("there are no rounds {store} takes to load it with"),
            )
            .with("store", store.to_string())
        }));
    }
    Ok(loaded)
}

/// The next round to load into a store: from `from`, or from what `who`
/// carries, one the store would take and that is not in it already.
fn next_source(
    sim: &Sim,
    who: EntityId,
    store: EntityId,
    from: Option<EntityId>,
) -> Option<EntityId> {
    let candidates: Vec<EntityId> = match from {
        Some(from) if round_of(sim, from).is_some() => vec![from],
        Some(from) => sim.truth().entities().contents(from).to_vec(),
        None => sim.truth().entities().contents_deep(who),
    };
    candidates.into_iter().find(|candidate| {
        *candidate != store
            && item::holder_of(sim, *candidate) != Some(store)
            && round_of(sim, *candidate).is_some()
            && takes_round(sim, store, *candidate)
    })
}

/// Whether a store would take a round by its tags.
fn takes_round(sim: &Sim, store: EntityId, round: EntityId) -> bool {
    let holds = item_def(sim, store)
        .and_then(|def| def.holds.as_ref())
        .or_else(|| crate::device::holds_of(sim, store));
    let Some(holds) = holds else {
        return false;
    };
    holds.accepts.is_empty()
        || sim.truth().entity(round).is_some_and(|record| {
            holds
                .accepts
                .iter()
                .any(|tag| record.tags.contains(tag.as_str()))
        })
}
