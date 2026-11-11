//! The one way an entity's place changes (01 R3, 03 §4.2).
//!
//! Every function that writes [`Place`] is in this file:
//!
//! | Function | Writes it when |
//! |----------|----------------|
//! | [`Sim::relocate`] | Anything in the simulation moves an entity, puts it inside another, takes it out, or takes it out of the world |
//! | `Sim::follow_displacement` | The world moved an occupant out of the way of an edit (02 §23) and the kernel follows |
//! | [`Sim::reindex`] | An entity could not be placed again after a load and leaves the world |
//!
//! `place` is a private field of the parent module, so nothing outside
//! these two files can name it. The parent module writes it once, when
//! an entity is made, as `Stored`.
//!
//! What `relocate` keeps true:
//!
//! - An entity is in the world's occupant index exactly when its place
//!   is `At` a cell, and at that cell.
//! - Containment has no loops and is at most
//!   [`MAX_CONTAINMENT_DEPTH`](super::MAX_CONTAINMENT_DEPTH) deep.
//! - An entity's wake-ups are on the clock of where it is (03 §3.4), and
//!   an entity that has left the world has none (invariant T9).

use clash_core::EntityId;
use clash_world::{
    Cause, CauseKind, EventSink, FOOTPRINT_MAX_SIDE, Footprint, Location, WorldCtx, WorldError,
};

use super::{MAX_CONTAINMENT_DEPTH, Place, Presence};
use crate::Sim;
use crate::event::{EventBody, Where};
use crate::reason::{Reason, codes};
use crate::sim::unknown_entity;

/// An entity that [`Sim::reindex`] could not put back where its place
/// said it was, and why. It has been taken out of the world.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Unplaced {
    /// Who.
    pub entity: EntityId,
    /// Where it was supposed to be.
    pub was: Place,
    /// Why it could not be placed there.
    pub reason: Reason,
}

impl Sim {
    /// Whether something with this footprint could be given this place,
    /// without changing anything. `id` is the entity, if it exists yet.
    /// Returns the place as it would be stored: a cell is normalised.
    pub(crate) fn check_place(
        &self,
        id: Option<EntityId>,
        footprint: Footprint,
        to: Place,
    ) -> Result<Place, Reason> {
        match to {
            Place::Stored => Ok(to),
            Place::In(container) => {
                self.known(container)?;
                // The container must not be the entity or anything in it.
                let mut at = container;
                let mut above = 1;
                loop {
                    if Some(at) == id {
                        return Err(Reason::new(
                            codes::RELOCATE_INSIDE_ITSELF,
                            format!(
                                "entity {at} cannot be put inside {container}, which is itself or inside it"
                            ),
                        )
                        .with("entity", at.to_string())
                        .with("container", container.to_string()));
                    }
                    match self.entities.get(at).map(|entity| entity.place) {
                        Some(Place::In(next)) if above <= MAX_CONTAINMENT_DEPTH => {
                            at = next;
                            above += 1;
                        }
                        _ => break,
                    }
                }
                let below = id.map_or(0, |id| self.height_below(id));
                if above + below > MAX_CONTAINMENT_DEPTH {
                    return Err(Reason::new(
                        codes::RELOCATE_TOO_DEEP,
                        format!(
                            "things may be nested at most {MAX_CONTAINMENT_DEPTH} deep, and this would be {}",
                            above + below
                        ),
                    )
                    .with("limit", MAX_CONTAINMENT_DEPTH as i64));
                }
                Ok(to)
            }
            Place::At { loc, alt_mm } => {
                let world = |error: WorldError| Reason::from_world(&error);
                if footprint.x > FOOTPRINT_MAX_SIDE
                    || footprint.y > FOOTPRINT_MAX_SIDE
                    || footprint.z > FOOTPRINT_MAX_SIDE
                {
                    return Err(world(WorldError::TooLarge {
                        what: "the side of an occupant's footprint, in cells",
                        limit: u64::from(FOOTPRINT_MAX_SIDE),
                    }));
                }
                let normalised = self.cosmos.normalize(loc).map_err(world)?;
                for cell in footprint.cells(loc.cell).cells() {
                    let cell = self
                        .cosmos
                        .normalize(Location::new(loc.grid, cell))
                        .map_err(world)?;
                    if !self.cosmos.is_loaded(cell) {
                        return Err(world(WorldError::NotLoaded(cell)));
                    }
                }
                Ok(Place::At {
                    loc: normalised,
                    alt_mm,
                })
            }
        }
    }

    /// How many levels of things are nested inside an entity: zero for
    /// one that holds nothing.
    fn height_below(&self, id: EntityId) -> usize {
        let mut deepest = 0;
        let mut stack: Vec<(EntityId, usize)> = vec![(id, 0)];
        while let Some((at, depth)) = stack.pop() {
            deepest = deepest.max(depth);
            if depth < MAX_CONTAINMENT_DEPTH {
                stack.extend(
                    self.entities
                        .contents(at)
                        .iter()
                        .map(|inside| (*inside, depth + 1)),
                );
            }
        }
        deepest
    }

    /// Moves an entity: to a cell, into another entity, or out of the
    /// world. **This is the only function that changes an entity's
    /// place** (01 R3). Returns where it was.
    ///
    /// It keeps the world's occupant index in step (only an entity `At`
    /// a cell is in it), keeps containment free of loops, puts the
    /// entity and everything inside it on the clock of where it now is
    /// with the local time its wake-ups had left (03 §3.4), and emits
    /// `entity_relocated` after the world's own `area_exited` and
    /// `area_entered`. What is inside the entity goes with it and is not
    /// reported separately.
    ///
    /// An entity that leaves the world, and everything inside it, has
    /// its wake-ups cancelled (invariant T9). Whoever stores an entity
    /// takes what should survive, such as timers, off the timeline first
    /// (03 §9).
    ///
    /// A move to where it already is does nothing at all. Refused,
    /// changing nothing, if the entity is unknown, the container is
    /// unknown, is the entity itself or is inside it, the nesting would
    /// be too deep, or the world refuses the cell: outside the realm, or
    /// not loaded.
    pub fn relocate(&mut self, id: EntityId, to: Place, cause: Cause) -> Result<Place, Reason> {
        let entity = self.entities.get(id).ok_or_else(|| unknown_entity(id))?;
        let (from, presence) = (entity.place, entity.presence);
        let to = self.check_place(Some(id), presence.footprint, to)?;
        if from == to {
            return Ok(from);
        }
        self.index(id, &presence, to, cause)?;
        self.entities.set_place(id, to);
        self.sync_clocks_under(id);
        if self.entities.location(id).is_none() {
            self.drop_wakes(id);
            for inside in self.entities.contents_deep(id) {
                self.drop_wakes(inside);
            }
        }
        self.emit(
            Where::Entity(id),
            cause,
            EventBody::EntityRelocated {
                entity: id,
                from,
                to,
            },
        );
        Ok(from)
    }

    /// Makes the world's occupant index say what `to` says: the entity
    /// is in it exactly when `to` is a cell, and at that cell. Decided
    /// by what the index holds, not by where the entity was thought to
    /// be, so that an index emptied by an unloaded tract mends itself.
    fn index(
        &mut self,
        id: EntityId,
        presence: &Presence,
        to: Place,
        cause: Cause,
    ) -> Result<(), Reason> {
        let held = self.cosmos.occupant(id).map(|record| record.loc);
        let mut ctx = self.world_ctx();
        let done = match (held, to) {
            (Some(at), Place::At { loc, .. }) if at == loc => Ok(()),
            (Some(_), Place::At { loc, .. }) => ctx.relocate(id, loc, cause).map(|_| ()),
            (None, Place::At { loc, .. }) => {
                ctx.place(presence.occupant(id), loc, presence.footprint, cause)
            }
            (Some(_), Place::In(_) | Place::Stored) => ctx.remove(id, cause).map(|_| ()),
            (None, Place::In(_) | Place::Stored) => Ok(()),
        };
        done.map_err(|error| Reason::from_world(&error))
    }

    /// Follows an occupant that the world moved out of the way of an
    /// edit (02 §23): the index already has it at `to`. Returns where it
    /// was, or `None` if nothing changed.
    pub(crate) fn follow_displacement(&mut self, id: EntityId, to: Location) -> Option<Place> {
        let from = self.entities.get(id)?.place;
        let alt_mm = match from {
            Place::At { alt_mm, .. } => alt_mm,
            Place::In(_) | Place::Stored => 0,
        };
        let to = Place::At { loc: to, alt_mm };
        if from == to {
            return None;
        }
        self.entities.set_place(id, to);
        self.sync_clocks_under(id);
        Some(from)
    }

    /// Changes what the world is told about an entity: its kind, planes,
    /// mass, flags and footprint. Its place does not change. If it is in
    /// the world and its footprint changes, the world works out afresh
    /// which areas it is inside and reports those it entered and left.
    ///
    /// Refused, changing nothing, if the entity is unknown or the new
    /// footprint does not fit where it is.
    pub fn set_presence(
        &mut self,
        id: EntityId,
        presence: Presence,
        cause: Cause,
    ) -> Result<(), Reason> {
        let entity = self.entities.get(id).ok_or_else(|| unknown_entity(id))?;
        let (old, place) = (entity.presence, entity.place);
        if old == presence {
            return Ok(());
        }
        if let Place::At { loc, .. } = place
            && self.cosmos.occupant(id).is_some()
        {
            self.check_place(Some(id), presence.footprint, place)?;
            let mut ctx = self.world_ctx();
            let world = |error: WorldError| Reason::from_world(&error);
            if presence.footprint != old.footprint {
                ctx.relocate_as(id, loc, presence.footprint, cause)
                    .map_err(world)?;
            }
            ctx.update_occupant(presence.occupant(id)).map_err(world)?;
        }
        if let Some(entity) = self.entities.get_mut(id) {
            entity.presence = presence;
        }
        self.absorb_world();
        Ok(())
    }

    /// Rebuilds the world's occupant index from the entities' places.
    ///
    /// The world neither saves nor hashes who occupies it (02 §8.1): the
    /// index is derived from the simulation's entities. Call this after
    /// [`Sim::load`], once the cosmos holds its loaded tracts. Whoever
    /// the index holds is taken out, and every entity that is `At` a
    /// cell is placed again, in id order. Nothing is reported, because
    /// nothing happened.
    ///
    /// An entity that cannot be placed where its place says (its tract
    /// is not loaded, or the cell no longer exists) is taken out of the
    /// world instead, with `entity_relocated`, and listed in what is
    /// returned. After a save and a load of the same world the list is
    /// empty, the state hash is what it was, and every occupant query
    /// answers as before.
    pub fn reindex(&mut self) -> Vec<Unplaced> {
        // What happens to the index here is not an event.
        let mut unheard = EventSink::new();
        let mut ctx = WorldCtx::new(&mut self.cosmos, &mut self.timeline, &mut unheard);
        let held: Vec<EntityId> = ctx
            .cosmos
            .occupants()
            .iter()
            .map(|record| record.occupant.id)
            .collect();
        for id in held {
            let _ = ctx.remove(id, Cause::DEBUG);
        }
        let mut unplaced = Vec::new();
        for entity in self.entities.iter() {
            let Place::At { loc, .. } = entity.place else {
                continue;
            };
            let presence = entity.presence;
            if let Err(error) = ctx.place(
                presence.occupant(entity.id()),
                loc,
                presence.footprint,
                Cause::DEBUG,
            ) {
                unplaced.push(Unplaced {
                    entity: entity.id(),
                    was: entity.place,
                    reason: Reason::from_world(&error),
                });
            }
        }
        for lost in &unplaced {
            self.entities.set_place(lost.entity, Place::Stored);
            self.sync_clocks_under(lost.entity);
            self.drop_wakes(lost.entity);
            for inside in self.entities.contents_deep(lost.entity) {
                self.drop_wakes(inside);
            }
            self.emit(
                Where::Global,
                Cause::new(CauseKind::Coarse),
                EventBody::EntityRelocated {
                    entity: lost.entity,
                    from: lost.was,
                    to: Place::Stored,
                },
            );
        }
        unplaced
    }
}
