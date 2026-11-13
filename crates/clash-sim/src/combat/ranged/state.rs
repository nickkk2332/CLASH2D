//! What ranged combat keeps between events (07 §11): the shots in
//! flight.
//!
//! The world carries a projectile as a body with a `Projectile` owner
//! and a tag (02 §15). The tag is the key of a [`Shot`] here: which
//! projectile it is, who fired it and with what, and where it was last
//! known to be, for the near misses on its way. An item thrown is a body
//! the item is; its shot is kept by the item's id.
//!
//! It is state: saved with the simulation and hashed. A simulation that
//! has never fired keeps nothing, and its hash is what it was before
//! shots were kept.

use std::collections::BTreeMap;

use clash_core::{EntityId, Real, StateHash, StateHasher, impl_state_hash};
use clash_world::GridId;
use serde::{Deserialize, Serialize};

use super::defs::ProjectileId;

/// One shot in flight: a projectile, or an item thrown.
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub struct Shot {
    /// The projectile, for a projectile; `None` for an item thrown.
    pub projectile: Option<ProjectileId>,
    /// For an item thrown, which of its surfaces strikes, by position
    /// among the item's; `None` for a blunt blow, as the world strikes.
    pub surface: Option<u8>,
    /// Who fired or threw it.
    pub by: Option<EntityId>,
    /// What it was fired with.
    pub weapon: Option<EntityId>,
    /// Whom it was aimed at.
    pub aimed: Option<EntityId>,
    /// The grid it flies in.
    pub grid: GridId,
    /// Where it was last known to be, in metres in its grid: where it
    /// was fired from, or where it last struck something.
    pub last: [Real; 3],
    /// Whom it has struck or passed close to, which it is not reported
    /// as passing again.
    pub met: Vec<EntityId>,
    /// Whether its `on_impact` has run.
    pub burst: bool,
}

impl_state_hash!(Shot {
    projectile,
    surface,
    by,
    weapon,
    aimed,
    grid,
    last,
    met,
    burst
});

/// Every shot in flight.
#[derive(Clone, PartialEq, Eq, Debug, Default, Serialize, Deserialize)]
pub struct RangedState {
    /// Projectiles in flight, by the tag their bodies carry.
    shots: BTreeMap<u64, Shot>,
    /// Items thrown and still in flight, by the item.
    thrown: BTreeMap<EntityId, Shot>,
    /// The tag the next projectile is given.
    next: u64,
}

impl StateHash for RangedState {
    fn state_hash(&self, hasher: &mut StateHasher) {
        self.shots.state_hash(hasher);
        self.thrown.state_hash(hasher);
        hasher.write_u64(self.next);
    }
}

impl RangedState {
    /// Whether nothing has ever been fired: then nothing is hashed.
    pub fn is_pristine(&self) -> bool {
        self.shots.is_empty() && self.thrown.is_empty() && self.next == 0
    }

    /// Keeps a projectile in flight, and returns the tag its body is to
    /// carry.
    pub fn fire(&mut self, shot: Shot) -> u64 {
        let tag = self.next;
        self.next = self.next.wrapping_add(1);
        self.shots.insert(tag, shot);
        tag
    }

    /// Takes back a tag given out for a body that could not be launched.
    pub fn unfire(&mut self, tag: u64) {
        self.shots.remove(&tag);
    }

    /// Keeps an item thrown.
    pub fn throw(&mut self, item: EntityId, shot: Shot) {
        self.thrown.insert(item, shot);
    }

    /// A projectile in flight, by its tag.
    pub fn shot(&self, tag: u64) -> Option<&Shot> {
        self.shots.get(&tag)
    }

    /// A projectile in flight, to change.
    pub fn shot_mut(&mut self, tag: u64) -> Option<&mut Shot> {
        self.shots.get_mut(&tag)
    }

    /// An item thrown and in flight.
    pub fn thrown(&self, item: EntityId) -> Option<&Shot> {
        self.thrown.get(&item)
    }

    /// An item thrown, to change.
    pub fn thrown_mut(&mut self, item: EntityId) -> Option<&mut Shot> {
        self.thrown.get_mut(&item)
    }

    /// Forgets a projectile that has come to an end.
    pub fn end_shot(&mut self, tag: u64) -> Option<Shot> {
        self.shots.remove(&tag)
    }

    /// Forgets an item that is no longer in flight.
    pub fn end_throw(&mut self, item: EntityId) -> Option<Shot> {
        self.thrown.remove(&item)
    }

    /// How many projectiles are in flight.
    pub fn in_flight(&self) -> usize {
        self.shots.len()
    }

    /// The tags of the projectiles in flight, in order.
    pub fn tags(&self) -> impl Iterator<Item = u64> + '_ {
        self.shots.keys().copied()
    }
}
