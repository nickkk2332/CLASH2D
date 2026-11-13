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
use crate::action::WindowId;

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
    /// The `shot` window (07 §11.2) each projectile in flight opened, by
    /// its tag. Hashed only when there is one.
    shot_windows: BTreeMap<u64, WindowId>,
    /// The window each item thrown and in flight opened. Hashed only
    /// when there is one.
    throw_windows: BTreeMap<EntityId, WindowId>,
}

impl StateHash for RangedState {
    fn state_hash(&self, hasher: &mut StateHasher) {
        self.shots.state_hash(hasher);
        self.thrown.state_hash(hasher);
        hasher.write_u64(self.next);
        // A simulation whose shots opened no window hashes as one from
        // before shots opened windows.
        if !self.shot_windows.is_empty() || !self.throw_windows.is_empty() {
            self.shot_windows.state_hash(hasher);
            self.throw_windows.state_hash(hasher);
        }
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
        self.shot_windows.remove(&tag);
        self.shots.remove(&tag)
    }

    /// Forgets an item that is no longer in flight.
    pub fn end_throw(&mut self, item: EntityId) -> Option<Shot> {
        self.throw_windows.remove(&item);
        self.thrown.remove(&item)
    }

    /// Keeps the `shot` window a projectile in flight opened (07 §11.2).
    pub fn set_shot_window(&mut self, tag: u64, window: WindowId) {
        if self.shots.contains_key(&tag) {
            self.shot_windows.insert(tag, window);
        }
    }

    /// Keeps the window an item thrown and in flight opened.
    pub fn set_throw_window(&mut self, item: EntityId, window: WindowId) {
        if self.thrown.contains_key(&item) {
            self.throw_windows.insert(item, window);
        }
    }

    /// The window a projectile in flight opened, if it opened one.
    pub fn shot_window(&self, tag: u64) -> Option<WindowId> {
        self.shot_windows.get(&tag).copied()
    }

    /// The window an item thrown and in flight opened, if it opened one.
    pub fn throw_window(&self, item: EntityId) -> Option<WindowId> {
        self.throw_windows.get(&item).copied()
    }

    /// The projectiles in flight that opened a window, by tag, and the
    /// items thrown that did, in order.
    pub fn of_window(&self, window: WindowId) -> (Vec<u64>, Vec<EntityId>) {
        let tags = self
            .shot_windows
            .iter()
            .filter(|(_, have)| **have == window)
            .map(|(tag, _)| *tag)
            .collect();
        let items = self
            .throw_windows
            .iter()
            .filter(|(_, have)| **have == window)
            .map(|(item, _)| *item)
            .collect();
        (tags, items)
    }

    /// Whether any shot in flight still belongs to a window.
    pub fn window_in_flight(&self, window: WindowId) -> bool {
        self.shot_windows.values().any(|have| *have == window)
            || self.throw_windows.values().any(|have| *have == window)
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
