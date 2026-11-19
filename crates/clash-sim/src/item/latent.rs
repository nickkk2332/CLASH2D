//! Latent contents (06 §9.3): a container does not make what it holds
//! until something looks.
//!
//! A container whose definition lists `contents` is made **latent**. The
//! first time it is opened ([`materialise`]: by a rule, by putting
//! something into it, by a packet that damages it) its contents are
//! drawn from that list with a generator seeded by the container's own
//! seed, and stowed in it. The seed was fixed when the container was
//! made, so the contents are the same whenever and in whatever order
//! containers are opened. Until then they weigh nothing and cost
//! nothing.
//!
//! What the list gives is stowed whether or not it fits: the list is its
//! author's to keep within the container's `holds`.

use std::sync::Arc;

use clash_core::{EntityId, Rng};
use clash_world::Cause;

use super::hold::{self, Hold, notify};
use super::make::create_seeded;
use super::{CONTENTS_MATERIALISED, Item, ItemDef, def_of, item_of};
use crate::Sim;
use crate::bus::EventPayload;
use crate::entity::Place;
use crate::event::Subject;
use crate::reason::Reason;

/// What a container's seed is mixed with before its contents are drawn,
/// so that they and its colour are drawn apart.
const CONTENTS_SALT: u64 = 0x636f_6e74_656e_7473;

/// Whether an entity is a container whose contents have not been made.
pub(crate) fn is_latent(sim: &Sim, container: EntityId) -> bool {
    sim.components
        .get::<Item>(container)
        .is_some_and(Item::is_latent)
}

/// Makes a container's latent contents (06 §9.3) and stows them in it.
/// Returns the entities made, in the order its definition lists them;
/// none for a container that was opened before or never had any.
///
/// Each entry of the definition's `contents` is there with its `chance`,
/// in a number drawn evenly from its `count`: as one stack if its
/// definition stacks, and otherwise as so many items. Emits
/// `contents_materialised`. Refused with `item.not_an_item` for
/// anything that is not an item.
pub fn materialise(
    sim: &mut Sim,
    container: EntityId,
    cause: Cause,
) -> Result<Vec<EntityId>, Reason> {
    let item = item_of(sim, container)?;
    if !item.is_latent() {
        return Ok(Vec::new());
    }
    let seed = item.seed();
    let defs = Arc::clone(&sim.defs);
    let entries = def_of(&defs, item).map_or(&[][..], |def| def.contents.as_slice());
    if let Some(item) = sim.components.get_mut::<Item>(container) {
        item.latent = false;
    }
    let registry = defs.try_registry::<ItemDef>();
    let mut rng = Rng::from_seed(seed ^ CONTENTS_SALT);
    let mut made = Vec::new();
    for entry in entries {
        // Every draw is made whether or not it matters, so that one
        // entry's numbers never depend on another's outcome.
        let there = rng.chance(entry.chance);
        let count = rng.range(i64::from(entry.count.min), i64::from(entry.count.max));
        let each_seed = rng.next_u64();
        let count = u32::try_from(count).unwrap_or(0);
        if !there || count == 0 {
            continue;
        }
        let stacks = registry
            .and_then(|registry| registry.try_get(entry.item.id()))
            .is_some_and(|def| def.stack);
        let (entities, members) = if stacks { (1, count) } else { (count, 1) };
        let mut seeds = Rng::from_seed(each_seed);
        for _ in 0..entities {
            let id = create_seeded(
                sim,
                entry.item.id(),
                members,
                Place::In(container),
                seeds.next_u64(),
                cause,
            )?;
            hold::set_entry(sim, container, id, Hold::Stowed);
            made.push(id);
        }
    }
    let payload = EventPayload::new(CONTENTS_MATERIALISED)
        .subject("self", Subject::Entity(container))
        .with("count", i64::try_from(made.len()).unwrap_or(i64::MAX));
    notify(sim, container, payload, cause);
    Ok(made)
}
