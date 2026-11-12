//! Which clock a thing is on, and scheduling for entities (03 §3.4, §3.5).
//!
//! | Thing | Clock |
//! |-------|-------|
//! | A location | The clock of the highest-priority area covering it that has one (ties: the area made first), else the realm's |
//! | An entity without a personal clock | The clock of its location |
//! | An entity with a personal clock | Its own, a child of its location's clock, or of its anchor if it has one |
//! | A contained entity | The clock of its outermost container, unless it has a personal clock |
//! | An entity that is not in the world | Its personal clock if it has one, else none. Nothing can be scheduled for it |
//!
//! The kernel does not store which clock an entity is on: it works it
//! out from where the entity is, every time. What it stores is which
//! wake-ups belong to which entity, so that when the answer changes
//! (the entity moved, an area came over it, its clock was taken away)
//! those wake-ups can be moved to the new clock with the local time they
//! had left.
//!
//! # Scheduling
//!
//! Schedule for an entity with [`Sim::schedule_for`], never on the
//! timeline directly: a wake-up the kernel does not know is the entity's
//! stays on the clock it was put on when the entity moves.

use std::collections::{BTreeMap, BTreeSet};

use clash_core::{Dur, EntityId, Real, StateHash, StateHasher};
use clash_time::{ClockId, Wake, WakeId};
use clash_world::{AreaId, Cause, Location, OccupantFilter, RealmId, distinct_ids};
use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::Sim;
use crate::event::{EventBody, Where};
use crate::reason::{Reason, codes};

/// Which entity each of its wake-ups belongs to.
///
/// The map from wake-up to owner is the state. The set that finds an
/// entity's wake-ups is derived from it and rebuilt on load.
#[derive(Clone, PartialEq, Eq, Default, Debug)]
pub(crate) struct WakeOwners {
    owner_of: BTreeMap<WakeId, EntityId>,
    /// Derived: the same pairs, by entity.
    owned: BTreeSet<(EntityId, WakeId)>,
}

impl WakeOwners {
    fn insert(&mut self, wake: WakeId, entity: EntityId) {
        self.owner_of.insert(wake, entity);
        self.owned.insert((entity, wake));
    }

    /// Forgets a wake-up that has fired or been cancelled.
    pub(crate) fn remove(&mut self, wake: WakeId) -> Option<EntityId> {
        let entity = self.owner_of.remove(&wake)?;
        self.owned.remove(&(entity, wake));
        Some(entity)
    }

    /// An entity's wake-ups, in the order they were scheduled.
    pub(crate) fn of(&self, entity: EntityId) -> Vec<WakeId> {
        self.owned
            .range((entity, WakeId::new(0))..=(entity, WakeId::new(u64::MAX)))
            .map(|(_, wake)| *wake)
            .collect()
    }
}

impl StateHash for WakeOwners {
    fn state_hash(&self, hasher: &mut StateHasher) {
        self.owner_of.state_hash(hasher);
    }
}

impl Serialize for WakeOwners {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        self.owner_of.serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for WakeOwners {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<WakeOwners, D::Error> {
        let owner_of = BTreeMap::<WakeId, EntityId>::deserialize(deserializer)?;
        let owned = owner_of
            .iter()
            .map(|(wake, entity)| (*entity, *wake))
            .collect();
        Ok(WakeOwners { owner_of, owned })
    }
}

/// Whose clock [`Sim::set_clock_rate`] changes.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ClockTarget {
    /// An area's clock.
    Area(AreaId),
    /// A realm's clock.
    Realm(RealmId),
}

impl Sim {
    // ---- Which clock -----------------------------------------------------

    /// The clock things at a location run on: that of the
    /// highest-priority area covering it that has a clock, and of equals
    /// the one made first, else its realm's (03 §3.4).
    pub fn location_clock(&self, loc: Location) -> ClockId {
        match self.cosmos.ambient(loc) {
            Ok(ambient) => ambient.clock,
            // A cell outside its grid: the realm's clock, if there is a
            // realm, and universal time if not.
            Err(_) => self
                .cosmos
                .grid(loc.grid)
                .map_or(ClockId::UNIVERSAL, |grid| grid.clock()),
        }
    }

    /// The clock of where an entity is: its own cell's, or that of the
    /// cell its outermost container is in. `None` if it is not in the
    /// world.
    fn site_clock(&self, id: EntityId) -> Option<ClockId> {
        self.entities
            .location(id)
            .map(|loc| self.location_clock(loc))
    }

    /// The clock an entity is on (03 §3.4): its personal clock if it has
    /// one; else, if it is inside something, the clock its outermost
    /// container is on; else the clock of its location. `None` for an
    /// entity that is unknown, or is not in the world and has no clock
    /// of its own.
    pub(crate) fn clock_of(&self, id: EntityId) -> Option<ClockId> {
        let entity = self.entities.get(id)?;
        if let Some(personal) = entity.clock {
            return Some(personal);
        }
        let top = self.entities.outermost(id)?;
        if top != id
            && let Some(personal) = self.entities.get(top).and_then(|top| top.clock)
        {
            return Some(personal);
        }
        self.site_clock(top)
    }

    /// How much local time has passed for an entity, on whatever clocks
    /// it has been on, while it was in the world or had a clock of its
    /// own. Zero for an unknown entity.
    pub(crate) fn own_time(&self, id: EntityId) -> Dur {
        let Some(entity) = self.entities.get(id) else {
            return Dur::ZERO;
        };
        let time = entity.time;
        match time.mark {
            Some((clock, at)) => match self.timeline.local_now(clock) {
                Ok(now) => time.total + (now - at),
                // The clock is gone, and what passed on it with it.
                Err(_) => time.total,
            },
            None => time.total,
        }
    }

    /// Brings an entity's own time up to date and marks it afresh on the
    /// clock it is on. Called before a clock is taken away, so that the
    /// time that passed on it is not lost.
    fn fold_own_time(&mut self, id: EntityId) {
        let total = self.own_time(id);
        let mark = self
            .clock_of(id)
            .and_then(|clock| Some((clock, self.timeline.local_now(clock).ok()?)));
        if let Some(entity) = self.entities.get_mut(id) {
            entity.time.total = total;
            entity.time.mark = mark;
        }
    }

    /// Puts one entity on the clock that now applies to it: reparents its
    /// personal clock, or moves its wake-ups, each keeping the local time
    /// it had left.
    fn sync_clock(&mut self, id: EntityId) {
        let Some(entity) = self.entities.get(id) else {
            return;
        };
        let (personal, anchor) = (entity.clock, entity.anchor);
        let marked = entity.time.mark.map(|(clock, _)| clock);
        if let Some(personal) = personal {
            let parent = anchor
                .filter(|anchor| self.timeline.has_clock(*anchor))
                .or_else(|| self.site_clock(id));
            if let Some(parent) = parent {
                // A parent that would make a loop is left as it was.
                let _ = self.timeline.reparent(personal, parent);
            }
        }
        let target = self.clock_of(id);
        if personal.is_none()
            && let Some(target) = target
        {
            for wake in self.wakes.of(id) {
                let _ = self.timeline.rehome(wake, target);
            }
        }
        if marked != target {
            self.fold_own_time(id);
        }
        // Pools and wounds keep wake-ups of their own (06 §4.2, §7.3).
        self.life_follow(id);
        // Timers go off the timeline as their owner leaves the world and
        // come back as it returns (03 §9).
        self.timers_follow(id);
    }

    /// Puts an entity and everything inside it on the clocks that now
    /// apply. Does nothing where nothing changed.
    pub(crate) fn sync_clocks_under(&mut self, id: EntityId) {
        self.sync_clock(id);
        for inside in self.entities.contents_deep(id) {
            self.sync_clock(inside);
        }
    }

    // ---- Scheduling ------------------------------------------------------

    /// Schedules a wake-up for an entity, `after` more local time on the
    /// clock the entity is on. The wake-up is the entity's: it follows
    /// the entity from clock to clock keeping the local time it has
    /// left, and is cancelled if the entity leaves the world or is
    /// removed.
    ///
    /// Refused for an unknown entity, and for one that is not in the
    /// world, even if it has a clock of its own: nothing waits on behalf
    /// of an entity that has left the simulation (invariant T9).
    pub fn schedule_for(
        &mut self,
        entity: EntityId,
        after: Dur,
        wake: Wake,
    ) -> Result<WakeId, Reason> {
        self.known(entity)?;
        // Nothing waits on behalf of an entity that is not in the world,
        // even one with a clock of its own (invariant T9).
        let clock = self
            .entities
            .location(entity)
            .and_then(|_| self.clock_of(entity))
            .ok_or_else(|| {
                Reason::new(
                    codes::ENTITY_STORED,
                    format!(
                        "entity {entity} is not in the world, so nothing can be scheduled for it"
                    ),
                )
                .with("entity", entity.to_string())
            })?;
        let id = self
            .timeline
            .schedule(clock, after, wake)
            .map_err(|error| Reason::from_time(&error))?;
        self.wakes.insert(id, entity);
        Ok(id)
    }

    /// Schedules a wake-up on a clock, for something that is not an
    /// entity: a world process, a timer owned by an area or a realm.
    pub fn schedule_on(
        &mut self,
        clock: ClockId,
        after: Dur,
        wake: Wake,
    ) -> Result<WakeId, Reason> {
        self.timeline
            .schedule(clock, after, wake)
            .map_err(|error| Reason::from_time(&error))
    }

    /// Cancels a wake-up, an entity's or not. Returns whether there was
    /// one to cancel.
    pub fn cancel_wake(&mut self, id: WakeId) -> bool {
        self.wakes.remove(id);
        self.timeline.cancel(id)
    }

    /// The entity a scheduled wake-up belongs to, if it is an entity's.
    pub fn wake_owner(&self, id: WakeId) -> Option<EntityId> {
        self.wakes.owner_of.get(&id).copied()
    }

    /// Cancels every wake-up of an entity. Returns how many there were.
    pub(crate) fn drop_wakes(&mut self, entity: EntityId) -> usize {
        let wakes = self.wakes.of(entity);
        for wake in &wakes {
            self.wakes.remove(*wake);
            self.timeline.cancel(*wake);
        }
        wakes.len()
    }

    // ---- The three operations (03 §13) -----------------------------------

    /// Gives an entity a personal clock running at `rate` relative to
    /// the clock of where it is, or changes the rate of the one it has
    /// (03 §3.5). Everything about the entity runs on it: its wake-ups
    /// move onto it with the local time they had left, and so do those
    /// of whatever it holds that has no clock of its own.
    ///
    /// `None` takes the personal clock away, and any anchor with it: the
    /// entity goes back to the clock of where it is.
    ///
    /// Emits `clock_rate_changed` if anything changed. Refused for an
    /// unknown entity and for a rate that is negative or not finite.
    pub fn set_time_rate(
        &mut self,
        entity: EntityId,
        rate: Option<Real>,
        cause: Cause,
    ) -> Result<(), Reason> {
        let record = self.known(entity)?;
        let (personal, anchor) = (record.clock, record.anchor);
        let time = |error| Reason::from_time(&error);
        let (clock, old) = match (personal, rate) {
            (None, None) => return Ok(()),
            (Some(clock), Some(rate)) => {
                let old = self.timeline.rate(clock);
                if old == Some(rate) {
                    return Ok(());
                }
                self.timeline.set_rate(clock, rate).map_err(time)?;
                (clock, old)
            }
            (None, Some(rate)) => {
                let parent = anchor
                    .filter(|anchor| self.timeline.has_clock(*anchor))
                    .or_else(|| self.site_clock(entity))
                    .unwrap_or(ClockId::UNIVERSAL);
                let clock = self.timeline.create_clock(parent, rate).map_err(time)?;
                for wake in self.wakes.of(entity) {
                    let _ = self.timeline.rehome(wake, clock);
                }
                if let Some(record) = self.entities.get_mut(entity) {
                    record.clock = Some(clock);
                }
                self.sync_clocks_under(entity);
                (clock, None)
            }
            (Some(clock), None) => {
                let old = self.timeline.rate(clock);
                if let Some(record) = self.entities.get_mut(entity) {
                    record.clock = None;
                    record.anchor = None;
                }
                // While the old clock still exists: its wake-ups move to
                // the clock of where the entity is, and the time that
                // passed on it is counted.
                self.sync_clocks_under(entity);
                let _ = self.timeline.destroy_clock(clock);
                (clock, old)
            }
        };
        self.emit(
            Where::Entity(entity),
            cause,
            EventBody::ClockRateChanged {
                clock,
                entity: Some(entity),
                area: None,
                old,
                new: rate,
            },
        );
        Ok(())
    }

    /// Pins an entity's personal clock under a chosen clock, whatever
    /// the entity is standing in (03 §3.4). Anchoring to the realm's
    /// clock is how someone keeps moving inside a bubble of stopped
    /// time. An entity with no personal clock is given one at rate 1.
    ///
    /// `None` lifts the anchor: the personal clock stays and goes back
    /// under the clock of where the entity is.
    ///
    /// Refused, changing nothing, for an unknown entity, an unknown
    /// clock, and an anchor that is the entity's own clock or under it.
    pub fn anchor_clock(
        &mut self,
        entity: EntityId,
        anchor: Option<ClockId>,
        cause: Cause,
    ) -> Result<(), Reason> {
        let personal = self.known(entity)?.clock;
        let Some(anchor) = anchor else {
            if let Some(record) = self.entities.get_mut(entity) {
                record.anchor = None;
            }
            self.sync_clocks_under(entity);
            return Ok(());
        };
        if !self.timeline.has_clock(anchor) {
            return Err(Reason::from_time(&clash_time::TimeError::UnknownClock(
                anchor,
            )));
        }
        match personal {
            Some(personal) => self
                .timeline
                .reparent(personal, anchor)
                .map_err(|error| Reason::from_time(&error))?,
            None => {
                // The anchor first, so that the new clock is made under it.
                if let Some(record) = self.entities.get_mut(entity) {
                    record.anchor = Some(anchor);
                }
                if let Err(reason) = self.set_time_rate(entity, Some(Real::ONE), cause) {
                    if let Some(record) = self.entities.get_mut(entity) {
                        record.anchor = None;
                    }
                    return Err(reason);
                }
            }
        }
        if let Some(record) = self.entities.get_mut(entity) {
            record.anchor = Some(anchor);
        }
        self.sync_clocks_under(entity);
        Ok(())
    }

    /// Changes the rate of an area's or a realm's clock (03 §13).
    ///
    /// For an area, a rate gives it a clock if it has none and `None`
    /// takes its clock away; whoever is inside is put on the clock that
    /// now applies, keeping the local time their wake-ups had left. A
    /// realm always has a clock: `None` is refused for one.
    ///
    /// Emits `clock_rate_changed` if anything changed. Refused for an
    /// unknown area or realm and for a rate that is negative or not
    /// finite.
    pub fn set_clock_rate(
        &mut self,
        target: ClockTarget,
        rate: Option<Real>,
        cause: Cause,
    ) -> Result<(), Reason> {
        let world = |error| Reason::from_world(&error);
        match target {
            ClockTarget::Area(area) => {
                let before = self
                    .cosmos
                    .area(area)
                    .ok_or_else(|| world(clash_world::WorldError::UnknownArea(area)))?
                    .clock;
                let old = before.and_then(|clock| self.timeline.rate(clock));
                if old == rate {
                    return Ok(());
                }
                let inside = distinct_ids(
                    &self
                        .cosmos
                        .occupants_in_area(area, &OccupantFilter::ANY)
                        .map_err(world)?,
                );
                if rate.is_none() {
                    // The clock is about to go: count the time on it.
                    for id in &inside {
                        self.fold_own_time(*id);
                        for held in self.entities.contents_deep(*id) {
                            self.fold_own_time(held);
                        }
                    }
                }
                self.world_ctx()
                    .set_area_clock_rate(area, rate)
                    .map_err(world)?;
                let after = self.cosmos.area(area).and_then(|area| area.clock);
                if before != after {
                    for id in inside {
                        self.sync_clocks_under(id);
                    }
                }
                if let Some(clock) = after.or(before) {
                    self.emit(
                        Where::Global,
                        cause,
                        EventBody::ClockRateChanged {
                            clock,
                            entity: None,
                            area: Some(area),
                            old,
                            new: rate,
                        },
                    );
                }
                Ok(())
            }
            ClockTarget::Realm(realm) => {
                let clock = self
                    .cosmos
                    .realm(realm)
                    .ok_or_else(|| world(clash_world::WorldError::UnknownRealm(realm)))?
                    .clock;
                let Some(rate) = rate else {
                    return Err(Reason::new(
                        codes::CLOCK_BAD_RATE,
                        "a realm always has a clock: its rate can be changed, not taken away",
                    ));
                };
                let old = self.timeline.rate(clock);
                if old == Some(rate) {
                    return Ok(());
                }
                self.timeline
                    .set_rate(clock, rate)
                    .map_err(|error| Reason::from_time(&error))?;
                self.emit(
                    Where::Clock(clock),
                    cause,
                    EventBody::ClockRateChanged {
                        clock,
                        entity: None,
                        area: None,
                        old,
                        new: Some(rate),
                    },
                );
                Ok(())
            }
        }
    }
}
