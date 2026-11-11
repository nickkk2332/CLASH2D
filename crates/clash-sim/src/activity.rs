//! Activity under the simulation: tickets, loading and unloading tracts,
//! and what becomes of the entities that stand in them (02 §9.4, §9.5,
//! §10.2; 03 §4.3 step 5).
//!
//! The world keeps tracts loaded because tickets ask for them, and brings
//! what is loaded into line with the tickets when it is asked to:
//! [`WorldCtx::reconcile_activity`]. The simulation asks at every safe
//! point: after each wake-up [`Sim::advance`](crate::Sim::advance)
//! handles and after each command [`Sim::submit`](crate::Sim::submit)
//! accepts. For that it owns what loading needs besides the world: the
//! [`Generators`] and the tract store.
//!
//! # What the simulation does about its entities
//!
//! The world does not own entities and cannot take them out of a tract
//! it wants to unload (01 R4). It says `tract_unloading`, and the
//! simulation answers:
//!
//! | The world says | The simulation |
//! |----------------|----------------|
//! | `tract_unloading` | Takes every entity with a cell in the tract out of the world: its place becomes `Stored`, and where it stood is remembered on the [`Shelf`] under that tract. `entity_relocated` is emitted for each. What an entity holds stays inside it |
//! | `tract_loaded` | Puts back the entities shelved under the tract, each where it stood, with `entity_relocated`. One whose footprint reaches into a tract that is not loaded waits on the shelf under that tract |
//! | A loaded tract's spawn requests | Emits `spawn_requested` for each, for the package that makes entities from definitions (first-playable E9, E11) |
//! | A tract that could not be loaded or unloaded | Reports a `fault` with the reason `world.tract_failed` |
//!
//! A shelved entity keeps its id, its components and everything inside
//! it. Its wake-ups are cancelled when it leaves the world (invariant
//! T9): taking timers off the timeline first, and catching up when it
//! returns, is package T7's.
//!
//! # Starting a session
//!
//! An entity's ticket covers nothing until the entity has been seen in
//! the world, and an entity cannot be placed in a tract that is not
//! loaded. [`Sim::start_session`] breaks the circle the way the world
//! prescribes: it holds a fixed ticket over the place, reconciles, puts
//! the entity there, holds [`Ticket::player`] for it and lets the fixed
//! ticket go.
//!
//! # What is not state
//!
//! The generators and the store are given when the simulation is made
//! ([`Sim::with_io`](crate::Sim::with_io)) and again after a load. The
//! shelf is state: saved, loaded and hashed.

use core::fmt;
use std::collections::BTreeMap;

use clash_core::{EntityId, StateHash, StateHasher, sorted};
use clash_world::{
    Activity, Cause, CauseKind, Footprint, Generators, Location, RealmId, Reconciled, Ticket,
    TicketAnchor, TicketId, TicketReason, TractIo, TractPos, TractStore, WorldCtx, WorldError,
};
use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::Sim;
use crate::entity::Place;
use crate::event::{EventBody, Where};
use crate::reason::{Reason, codes};

/// How many times one safe point reconciles before it leaves the rest
/// to the next: once to announce the tracts that are to go, once to
/// unload them now that they are empty, and once more for what loading
/// put in the way.
const RECONCILE_ROUNDS: usize = 4;

/// A tract store the simulation can hold: any [`TractStore`] that can be
/// copied with the simulation and printed with it.
///
/// Every `TractStore` that is `Clone`, `Debug`, `Send` and `Sync` is
/// one: [`MemoryStore`](clash_world::MemoryStore) is. A store that
/// stands for a file is cloned as another handle on the same file; two
/// simulations must not both unload into it.
pub trait SimStore: TractStore + fmt::Debug + Send + Sync {
    /// A copy of the store, for a copy of the simulation.
    fn clone_store(&self) -> Box<dyn SimStore>;
}

impl<S> SimStore for S
where
    S: TractStore + Clone + fmt::Debug + Send + Sync + 'static,
{
    fn clone_store(&self) -> Box<dyn SimStore> {
        Box::new(self.clone())
    }
}

impl Clone for Box<dyn SimStore> {
    fn clone(&self) -> Box<dyn SimStore> {
        self.clone_store()
    }
}

/// An entity that was taken out of the world with its tract, and where
/// it stood.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub struct Shelved {
    /// Who.
    pub entity: EntityId,
    /// The cell it stood in.
    pub loc: Location,
    /// Its height above the base of that cell, in millimetres.
    pub alt_mm: u16,
}

clash_core::impl_state_hash!(Shelved {
    entity,
    loc,
    alt_mm
});

/// A tract of a realm.
type TractKey = (RealmId, TractPos);

/// The entities that left the world because their tract was unloaded,
/// by the tract whose loading brings each back (02 §9.5).
///
/// The lists by tract are the state. Which tract an entity is under is
/// derived from them and rebuilt on load.
#[derive(Clone, PartialEq, Eq, Default, Debug)]
pub struct Shelf {
    /// By tract, each list in id order.
    by_tract: BTreeMap<TractKey, Vec<Shelved>>,
    /// Derived: the tract each shelved entity is under.
    home: BTreeMap<EntityId, TractKey>,
}

impl Shelf {
    /// How many entities are shelved.
    pub fn len(&self) -> usize {
        self.home.len()
    }

    /// Whether none is.
    pub fn is_empty(&self) -> bool {
        self.home.is_empty()
    }

    /// Where a shelved entity stood, and the tract it is waiting for.
    pub fn of(&self, entity: EntityId) -> Option<(RealmId, TractPos, Shelved)> {
        let (realm, tract) = *self.home.get(&entity)?;
        let shelved = self
            .by_tract
            .get(&(realm, tract))?
            .iter()
            .find(|shelved| shelved.entity == entity)?;
        Some((realm, tract, *shelved))
    }

    /// The entities waiting for a tract, in id order.
    pub fn waiting_for(&self, realm: RealmId, tract: TractPos) -> &[Shelved] {
        self.by_tract
            .get(&(realm, tract))
            .map_or(&[], Vec::as_slice)
    }

    /// The tracts something is waiting for, by realm and then row by row.
    pub fn tracts(&self) -> impl Iterator<Item = (RealmId, TractPos)> + '_ {
        self.by_tract.keys().copied()
    }

    /// Shelves an entity under a tract. One that is shelved already is
    /// moved.
    fn put(&mut self, key: TractKey, shelved: Shelved) {
        self.forget(shelved.entity);
        let list = self.by_tract.entry(key).or_default();
        let at = list.partition_point(|have| have.entity < shelved.entity);
        list.insert(at, shelved);
        self.home.insert(shelved.entity, key);
    }

    /// Takes everything waiting for a tract off the shelf, in id order.
    fn take(&mut self, key: TractKey) -> Vec<Shelved> {
        let taken = self.by_tract.remove(&key).unwrap_or_default();
        for shelved in &taken {
            self.home.remove(&shelved.entity);
        }
        taken
    }

    /// Takes one entity off the shelf, if it is on it.
    pub(crate) fn forget(&mut self, entity: EntityId) -> Option<Shelved> {
        let key = self.home.remove(&entity)?;
        let list = self.by_tract.get_mut(&key)?;
        let at = list.iter().position(|shelved| shelved.entity == entity)?;
        let shelved = list.remove(at);
        if list.is_empty() {
            self.by_tract.remove(&key);
        }
        Some(shelved)
    }
}

impl StateHash for Shelf {
    fn state_hash(&self, hasher: &mut StateHasher) {
        self.by_tract.state_hash(hasher);
    }
}

impl Serialize for Shelf {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        self.by_tract.serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for Shelf {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Shelf, D::Error> {
        let by_tract = BTreeMap::<TractKey, Vec<Shelved>>::deserialize(deserializer)?;
        let home = by_tract
            .iter()
            .flat_map(|(key, list)| list.iter().map(|shelved| (shelved.entity, *key)))
            .collect();
        Ok(Shelf { by_tract, home })
    }
}

/// The cause of what the simulation does to its entities when tracts
/// load and unload.
const ACTIVITY_CAUSE: Cause = Cause::new(CauseKind::Coarse);

impl Sim {
    /// The generators tracts are made with.
    pub fn generators(&self) -> &Generators {
        &self.generators
    }

    /// The store changed tracts are kept in while they are not loaded.
    pub fn store(&self) -> &dyn SimStore {
        self.store.as_ref()
    }

    /// The entities that left the world with their tracts.
    pub fn shelf(&self) -> &Shelf {
        &self.shelf
    }

    /// Reconciles interest tickets with what is loaded (02 §10.2): the
    /// safe point of 03 §4.3. Tracts the tickets no longer keep are
    /// unloaded, their entities shelved; tracts they want are loaded,
    /// their entities put back and their spawn requests reported.
    ///
    /// [`advance`](Sim::advance) and [`submit`](Sim::submit) call this.
    /// Simulation code that has changed tickets and needs the tracts at
    /// once, as [`start_session`](Sim::start_session) does, calls it
    /// itself. When nothing has changed it costs a look at each ticket.
    pub fn reconcile_activity(&mut self) {
        for _ in 0..RECONCILE_ROUNDS {
            let reconciled = {
                let mut io = TractIo::new(&self.generators, &mut *self.store);
                let mut ctx =
                    WorldCtx::new(&mut self.cosmos, &mut self.timeline, &mut self.world_events);
                ctx.reconcile_activity(&mut io)
            };
            // What the world said, in order: entities leave the tracts
            // that are to go and return to those that came.
            self.absorb_world();
            let again = !reconciled.waiting.is_empty();
            self.report_reconciled(reconciled);
            if !again {
                break;
            }
        }
    }

    /// Passes on what a reconciliation returned beside its events.
    fn report_reconciled(&mut self, reconciled: Reconciled) {
        for loaded in reconciled.loaded {
            let Some(grid) = self.cosmos.realm(loaded.realm).map(|realm| realm.base_grid) else {
                continue;
            };
            for request in loaded.spawns {
                let loc = Location::new(grid, request.at);
                self.emit(
                    Where::Loc(loc),
                    ACTIVITY_CAUSE,
                    EventBody::SpawnRequested {
                        realm: loaded.realm,
                        loc,
                        request,
                    },
                );
            }
        }
        for failure in reconciled.failed {
            self.fault(
                Reason::new(
                    codes::WORLD_TRACT_FAILED,
                    format!(
                        "tract {} of realm {} could not be loaded or unloaded: {}",
                        failure.tract, failure.realm, failure.error
                    ),
                )
                .with("realm", i64::from(failure.realm.raw()))
                .with("x", i64::from(failure.tract.x))
                .with("y", i64::from(failure.tract.y)),
            );
        }
    }

    /// Takes every entity with a cell in a tract out of the world and
    /// shelves it under the tract: the answer to `tract_unloading`.
    pub(crate) fn shelve_tract(&mut self, realm: RealmId, tract: TractPos) {
        let Some(grid) = self.cosmos.realm(realm).map(|realm| realm.base_grid) else {
            return;
        };
        let mut standing: Vec<EntityId> = Vec::new();
        if let Ok(view) = self.cosmos.grid(grid)
            && let Some(column) = view.column(tract)
        {
            for (_, chunk) in column.chunks() {
                for entry in chunk.occupants().iter() {
                    sorted::insert(&mut standing, entry.id);
                }
            }
        }
        for entity in standing {
            let Some(Place::At { loc, alt_mm }) = self.entities.get(entity).map(|e| e.place())
            else {
                continue;
            };
            if self.relocate(entity, Place::Stored, ACTIVITY_CAUSE).is_ok() {
                self.shelf.put(
                    (realm, tract),
                    Shelved {
                        entity,
                        loc,
                        alt_mm,
                    },
                );
            }
        }
    }

    /// Puts back the entities shelved under a tract: the answer to
    /// `tract_loaded`. One whose footprint reaches into a tract that is
    /// not loaded goes back on the shelf, under that tract.
    pub(crate) fn unshelve_tract(&mut self, realm: RealmId, tract: TractPos) {
        for shelved in self.shelf.take((realm, tract)) {
            let Some(entity) = self.entities.get(shelved.entity) else {
                continue;
            };
            if entity.place() != Place::Stored {
                // Something has given it another place meanwhile.
                continue;
            }
            let footprint = entity.presence().footprint;
            if let Some(missing) = self.first_unloaded(shelved.loc, footprint) {
                self.shelf.put((realm, missing), shelved);
                continue;
            }
            let to = Place::At {
                loc: shelved.loc,
                alt_mm: shelved.alt_mm,
            };
            if let Err(reason) = self.relocate(shelved.entity, to, ACTIVITY_CAUSE) {
                // Where it stood is no longer a place. It stays out of
                // the world, and whoever looks after it is told why.
                self.fault(reason.with("entity", shelved.entity.to_string()));
            }
        }
    }

    /// The first tract a footprint at a cell needs that is not loaded.
    fn first_unloaded(&self, loc: Location, footprint: Footprint) -> Option<TractPos> {
        footprint
            .cells(loc.cell)
            .cells()
            .filter_map(|cell| {
                self.cosmos
                    .normalize(Location::new(loc.grid, cell))
                    .ok()
                    .filter(|cell| !self.cosmos.is_loaded(*cell))
            })
            .map(|cell| cell.cell.tract())
            .next()
    }

    /// Starts a session for an entity at a place that may not be loaded
    /// yet: holds a fixed ticket over the place, reconciles so that its
    /// tracts load, puts the entity there, holds the ticket of a
    /// controlled character for it ([`Ticket::player`]) and lets the
    /// fixed ticket go. Returns the entity's ticket, which follows it
    /// from then on.
    ///
    /// Refused, with nothing left held, for an unknown entity, a place
    /// outside its realm or in a grid that is not a realm's base grid,
    /// and a place whose tract could not be loaded. Only the last of
    /// these leaves a trace: the realm has been surveyed there.
    pub fn start_session(
        &mut self,
        entity: EntityId,
        at: Location,
        cause: Cause,
    ) -> Result<TicketId, Reason> {
        let (realm, at) = self.check_session(entity, at)?;
        let world = |error: WorldError| Reason::from_world(&error);
        let scout = Ticket::player(realm, entity);
        let scout = Ticket {
            anchor: TicketAnchor::Fixed(at.cell.tract()),
            ..scout
        };
        let scout = self.cosmos.hold_ticket(scout).map_err(world)?;
        self.reconcile_activity();
        let placed = self.relocate(entity, Place::at(at), cause);
        let held = match placed {
            Ok(_) => self
                .cosmos
                .hold_ticket(Ticket::player(realm, entity))
                .map_err(world),
            Err(reason) => Err(reason),
        };
        self.cosmos.release_ticket(scout);
        self.reconcile_activity();
        held
    }

    /// Whether a session could be started for an entity at a place, as
    /// far as can be told without loading anything: the realm the place
    /// is in and the place normalised, or the reason. Changes nothing.
    pub(crate) fn check_session(
        &self,
        entity: EntityId,
        at: Location,
    ) -> Result<(RealmId, Location), Reason> {
        let footprint = self.known(entity)?.presence().footprint;
        let world = |error: WorldError| Reason::from_world(&error);
        let at = self.cosmos.normalize(at).map_err(world)?;
        for cell in footprint.cells(at.cell).cells() {
            self.cosmos
                .normalize(Location::new(at.grid, cell))
                .map_err(world)?;
        }
        let realm = self
            .cosmos
            .realm_of(at.grid)
            .filter(|realm| {
                self.cosmos
                    .realm(*realm)
                    .is_some_and(|realm| realm.base_grid == at.grid)
            })
            .ok_or_else(|| world(WorldError::UnknownGrid(at.grid)))?;
        Ok((realm, at))
    }

    /// Holds a ticket: asks for the tracts round its anchor to be kept
    /// loaded from the next safe point on (02 §10.1).
    pub fn hold_ticket(&mut self, ticket: Ticket) -> Result<TicketId, Reason> {
        self.cosmos
            .hold_ticket(ticket)
            .map_err(|error| Reason::from_world(&error))
    }

    /// Releases a ticket. The tracts only it kept are unloaded at the
    /// next safe point.
    pub fn release_ticket(&mut self, ticket: TicketId) -> Option<Ticket> {
        self.cosmos.release_ticket(ticket)
    }
}

/// A fixed ticket of a tool or a test over one tract and those within
/// `radius` of it, warm.
pub fn debug_ticket(realm: RealmId, tract: TractPos, radius: u8) -> Ticket {
    Ticket::new(
        realm,
        TicketAnchor::Fixed(tract),
        radius,
        Activity::Warm,
        TicketReason::Debug,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use clash_core::{Hash128, codec};
    use clash_world::{CellPos, GridId};

    fn shelved(slot: u32, x: i32) -> Shelved {
        Shelved {
            entity: EntityId::new(slot, 0),
            loc: Location::new(GridId::new(0), CellPos::new(x, 0, 0)),
            alt_mm: 0,
        }
    }

    #[test]
    fn the_shelf_keeps_each_tract_in_id_order_and_survives_the_codec() {
        let realm = RealmId::new(0);
        let (near, far) = (TractPos::new(0, 0), TractPos::new(3, 1));
        let mut shelf = Shelf::default();
        shelf.put((realm, near), shelved(5, 1));
        shelf.put((realm, near), shelved(2, 2));
        shelf.put((realm, far), shelved(9, 100));
        assert_eq!(shelf.len(), 3);
        let ids: Vec<u32> = shelf
            .waiting_for(realm, near)
            .iter()
            .map(|shelved| shelved.entity.slot())
            .collect();
        assert_eq!(ids, [2, 5]);
        assert_eq!(
            shelf.of(EntityId::new(9, 0)).map(|(_, tract, _)| tract),
            Some(far)
        );

        let bytes = codec::to_bytes(&shelf).unwrap();
        let back: Shelf = codec::from_bytes(&bytes).unwrap();
        assert_eq!(back, shelf);
        assert_eq!(Hash128::of(&back), Hash128::of(&shelf));

        // Shelved again elsewhere, an entity is in one place only.
        shelf.put((realm, far), shelved(5, 1));
        assert_eq!(shelf.waiting_for(realm, near).len(), 1);
        assert_eq!(shelf.forget(EntityId::new(2, 0)), Some(shelved(2, 2)));
        assert!(shelf.waiting_for(realm, near).is_empty());
        assert_eq!(shelf.take((realm, far)).len(), 2);
        assert!(shelf.is_empty());
        assert_eq!(shelf.tracts().count(), 0);
    }
}
