//! Attributes, pools and tracks: the three numeric primitives every
//! stat, resource and progression scheme is built from
//! (`docs/06-entities-and-bodies.md` section 4).
//!
//! | Primitive | Is | Module |
//! |-----------|----|--------|
//! | Attribute | A number with a base and a stack of modifiers; the base may be an expression over other attributes | [`attr`] |
//! | Pool | A bounded quantity with a net rate, whose threshold crossings are scheduled, never polled | [`pool`] |
//! | Track | An accumulating counter with levels | [`track`] |
//!
//! | Part | Is |
//! |------|----|
//! | [`AttributeDef`], [`PoolDef`], [`TrackDef`] | The definition types `attribute`, `pool` and `track` |
//! | [`StatDefs`] | What is worked out from them once mods have loaded |
//! | [`Stats`] | The component: one entity's attributes, pools and tracks |
//! | [`StatsCx`] | What every function that changes stats is handed |
//! | [`Change`] | What a function reports having changed, for the caller to raise events from |
//! | [`StatsError`] | Why a change was refused, with a [`reason`] code |
//! | [`props`] | The kinds, properties and events registered with the expression language, and [`property`], which answers them |
//!
//! Nothing here knows the simulation. The functions take the store of
//! [`Stats`], the definitions, the timeline and an expression host, and
//! return what changed as plain values.
//!
//! # What the kernel holds and calls
//!
//! The simulation holds a [`StatDefs`] beside its definitions, made with
//! [`StatDefs::new`] after loading, and a
//! [`Runner`](clash_data::Runner). The [`Stats`] are in
//! `components.stats`.
//!
//! | When | The kernel |
//! |------|-----------|
//! | Building its definition types | Calls the crate's `register_defs`, which registers the three types, the cycle check and the [registry step](props::registry_step). |
//! | A creature or an item with stats is made | Calls [`attach`] with the entity's clock, then [`set_base`], [`add_pool`] and [`advance_track`] for what its templates give it. |
//! | A trait, an item, a wound or a rule confers a modifier or a rate | Calls [`add_modifier`] or [`set_rate`] with a [`Source`] naming it, and [`remove_source`] when it goes. |
//! | A verb changes a pool | Raises the hook `pool_changing` itself, then calls [`pool_add`], or [`pool_spend`] for a cost that is paid in full or not at all. |
//! | Something else about the entity changed that a stat's expression may read (its body, its mass, what it holds) | Calls [`refresh`]. |
//! | A wake-up pops for which [`is_pool_wake`] is true | Calls [`on_wake`]. |
//! | The entity moves to another clock (03 §3.4) | Calls [`set_clock`]. A change of a clock's rate needs no call: the timeline keeps each wake-up's remaining local time. |
//! | The entity is stored, unloaded or destroyed | Calls [`suspend`] before the component is taken out of the store, so that no wake-up is left behind. |
//! | A stored entity returns (03 §9) | Puts the component back, then calls [`resume`] with its clock and the local time that passed: zero to pause, the time away to let it elapse. |
//! | A function returns [`Change`]s | Raises `attr_changed`, `pool_crossed` and `track_advanced` from them, in order, and runs a track's `on_advance`. |
//! | An expression reads a property for which [`StatDefs::answers`] is true | Forwards to [`property`]. |
//!
//! The host a function is handed answers everything *but* this module's
//! properties, which are answered here from the store being changed. So
//! the host must not hold the stats itself: the kernel builds it over the
//! rest of its state.
//!
//! # The wake-up
//!
//! A pool with a crossing ahead keeps one wake-up on the timeline, on its
//! owner's clock, in the domain [`WakeDomain::Timer`]: draining and
//! regenerating are timers of 03 §9, sped and slowed with everything
//! else about their owner (03 §3.5). Its `id` is the owner's
//! [`EntityId::to_bits`](clash_core::EntityId::to_bits). Its `arg` is the
//! pool's dense id with the top bit, [`POOL_WAKE_TAG`], set, which is
//! what tells it from any other timer: [`is_pool_wake`].
//!
//! # What is not built here
//!
//! - **Raising events and running hooks.** Events are registered here
//!   and raised by the kernel from the [`Change`]s returned.
//! - **The host.** [`property`] answers; the kernel's host forwards.
//! - **Saving.** [`Stats`] serialises through `clash_core::codec` and
//!   hashes; writing its definition ids by qualified id through a save's
//!   id table is the simulation's save path.
//! - **Seats.** A definition's `seat` is read and kept. Moving what is
//!   seated in a mind to another entity belongs with minds (06 §8.2).
//! - **A pool that names a substance** (16 §9.5) waits for substances.
//! - **Effects.** A track's `on_advance` is kept as written for the rule
//!   runtime.
//!
//! [`WakeDomain::Timer`]: clash_time::WakeDomain::Timer

pub mod attr;
mod defs;
pub mod pool;
pub mod props;
pub mod track;

use clash_core::{EntityId, Real, StateHash, StateHasher, impl_state_hash};
use clash_data::{DefTypes, Runner};
use clash_expr::{Host, Value};
use clash_time::{ClockId, TimeError, Timeline, WakeId};
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::component::ComponentStore;

pub use attr::{
    Modifier, Source, SourceKind, add_modifier, attach, refresh, remove_modifier, set_base,
};
pub use defs::{
    AttributeDef, AttributeId, MAX_THRESHOLDS, PoolDef, PoolId, Seat, StatDefTypes, StatDefs,
    StatKinds, TRACK_LEVEL_CAP, TrackDef, TrackId,
};
pub use pool::{
    POOL_WAKE_TAG, PoolAdded, PoolRate, add_pool, is_pool_wake, on_wake, pool_add, pool_spend,
    pool_wake, remove_pool, remove_rate, resume, set_clock, set_rate, suspend,
};
pub use props::property;
pub use track::advance_track;

use pool::PoolInst;

/// The store of every entity's [`Stats`].
pub type StatsStore = ComponentStore<Stats>;

/// Registers this module's definition types, its whole-set check and its
/// registry step. Called from the crate's `register_defs`.
pub fn register_defs(types: &mut DefTypes) {
    types
        .register::<AttributeDef>()
        .register::<PoolDef>()
        .register::<TrackDef>()
        .add_cross_check(defs::check_attribute_cycles)
        .add_registry_step(props::registry_step);
}

/// What every function that changes stats is handed: the store, the
/// definitions, the timeline, and what evaluates expressions.
///
/// The host answers whatever a stat's expression reads that is not a
/// stat: `self.mass`, a body function, a variable. This module's own
/// properties are answered from `stats`, so the host need not, and
/// cannot, hold the store.
pub struct StatsCx<'a, H: Host + ?Sized> {
    /// Every entity's stats.
    pub stats: &'a mut StatsStore,
    /// The definitions.
    pub defs: &'a StatDefs,
    /// The timeline pools schedule their crossings on.
    pub timeline: &'a mut Timeline,
    /// The simulation's expression host.
    pub host: &'a mut H,
    /// The machines expressions run on.
    pub runner: &'a mut Runner,
}

/// Something a function changed that the rest of the simulation is told
/// of. The kernel raises an event from each (06 §13).
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Change {
    /// An attribute's value changed: `attr_changed`.
    AttrChanged {
        /// Whose.
        entity: EntityId,
        /// Which attribute.
        attr: AttributeId,
        /// Its value before.
        old: Real,
        /// Its value now.
        new: Real,
    },
    /// A pool's level crossed a threshold: `pool_crossed`.
    PoolCrossed {
        /// Whose.
        entity: EntityId,
        /// Which pool.
        pool: PoolId,
        /// The threshold, as the fraction of `max` the definition lists.
        threshold: Real,
        /// True if the level rose to it or past it; false if it fell to
        /// it or below.
        rising: bool,
        /// The level after the crossing.
        level: Real,
    },
    /// A track reached a level: `track_advanced`, once for each level.
    TrackAdvanced {
        /// Whose.
        entity: EntityId,
        /// Which track.
        track: TrackId,
        /// The level reached.
        level: u32,
    },
}

/// What changed, in the order it happened.
pub type Changes = Vec<Change>;

/// The stable codes of the reasons this module refuses for (14 §5). Each
/// is the code of one [`StatsError`]; the sentence is the error's
/// message and the values are [`StatsError::values`].
pub mod reason {
    /// The entity has no stats. Values: `entity`.
    pub const NO_STATS: &str = "no_stats";
    /// The entity already has stats. Values: `entity`.
    pub const HAS_STATS: &str = "has_stats";
    /// The id is not an attribute, pool or track of the loaded
    /// definitions. Values: `what`.
    pub const UNKNOWN_STAT: &str = "unknown_stat";
    /// The entity does not have the pool. Values: `entity`, `pool`.
    pub const NO_SUCH_POOL: &str = "no_such_pool";
    /// The entity already has the pool. Values: `entity`, `pool`.
    pub const HAS_POOL: &str = "has_pool";
    /// The pool holds less than was to be spent. Values: `entity`,
    /// `pool`, `wanted`, `have`.
    pub const POOL_TOO_LOW: &str = "pool_too_low";
    /// An amount was negative where it may not be, or not finite.
    /// Values: `amount`.
    pub const BAD_AMOUNT: &str = "bad_amount";
    /// The timeline refused: the stats' clock is gone. Values: none.
    pub const NO_CLOCK: &str = "no_clock";
    /// The definitions were loaded without this module's registrations.
    /// Values: none.
    pub const NOT_REGISTERED: &str = "stats_not_registered";
}

/// Why a change to an entity's stats was refused. A refused change has
/// changed nothing.
///
/// Each is a reason in the form of 14 §5: [`code`](StatsError::code) is
/// its stable code, its message the sentence, and
/// [`values`](StatsError::values) the values involved.
#[derive(Debug, Clone, PartialEq, Error)]
#[non_exhaustive]
pub enum StatsError {
    /// The entity has no stats.
    #[error("{entity} has no stats")]
    NoStats {
        /// The entity.
        entity: EntityId,
    },
    /// The entity already has stats.
    #[error("{entity} already has stats")]
    HasStats {
        /// The entity.
        entity: EntityId,
    },
    /// The id is not one of the loaded definitions'.
    #[error("there is no {what} with this id")]
    UnknownStat {
        /// What sort of definition: "attribute", "pool", "track".
        what: &'static str,
    },
    /// The entity does not have the pool.
    #[error("{entity} has no pool `{name}`")]
    NoSuchPool {
        /// The entity.
        entity: EntityId,
        /// The pool.
        pool: PoolId,
        /// The pool's qualified id.
        name: String,
    },
    /// The entity already has the pool.
    #[error("{entity} already has the pool `{name}`")]
    HasPool {
        /// The entity.
        entity: EntityId,
        /// The pool.
        pool: PoolId,
        /// The pool's qualified id.
        name: String,
    },
    /// A pool holds less than was to be spent from it.
    #[error("the pool `{name}` holds {have}, which is less than the {wanted} wanted")]
    PoolTooLow {
        /// The entity.
        entity: EntityId,
        /// The pool.
        pool: PoolId,
        /// The pool's qualified id.
        name: String,
        /// What was to be spent.
        wanted: Real,
        /// What the pool holds.
        have: Real,
    },
    /// An amount was negative where it may not be, or not finite.
    #[error("an amount of {amount} cannot be used here: it must be finite and not negative")]
    BadAmount {
        /// What was given.
        amount: Real,
    },
    /// The timeline refused: the stats' clock is gone.
    #[error("the timeline refused: {0}")]
    Time(#[from] TimeError),
    /// The definitions were loaded without this module's registrations.
    #[error("the stats are not registered: the definitions lack {what}")]
    NotRegistered {
        /// What is missing.
        what: &'static str,
    },
}

impl StatsError {
    /// The reason's stable code: one of [`reason`].
    pub const fn code(&self) -> &'static str {
        match self {
            StatsError::NoStats { .. } => reason::NO_STATS,
            StatsError::HasStats { .. } => reason::HAS_STATS,
            StatsError::UnknownStat { .. } => reason::UNKNOWN_STAT,
            StatsError::NoSuchPool { .. } => reason::NO_SUCH_POOL,
            StatsError::HasPool { .. } => reason::HAS_POOL,
            StatsError::PoolTooLow { .. } => reason::POOL_TOO_LOW,
            StatsError::BadAmount { .. } => reason::BAD_AMOUNT,
            StatsError::Time(_) => reason::NO_CLOCK,
            StatsError::NotRegistered { .. } => reason::NOT_REGISTERED,
        }
    }

    /// The values involved, by name, as the expression language holds
    /// them. An entity is given as its id's bits and a pool as its
    /// qualified id.
    pub fn values(&self) -> Vec<(&'static str, Value)> {
        let bits = |entity: &EntityId| Value::Int(entity.to_bits() as i64);
        match self {
            StatsError::NoStats { entity } | StatsError::HasStats { entity } => {
                vec![("entity", bits(entity))]
            }
            StatsError::UnknownStat { what } => vec![("what", Value::text(what))],
            StatsError::NoSuchPool { entity, name, .. }
            | StatsError::HasPool { entity, name, .. } => {
                vec![("entity", bits(entity)), ("pool", Value::text(name))]
            }
            StatsError::PoolTooLow {
                entity,
                name,
                wanted,
                have,
                ..
            } => vec![
                ("entity", bits(entity)),
                ("pool", Value::text(name)),
                ("wanted", Value::Real(*wanted)),
                ("have", Value::Real(*have)),
            ],
            StatsError::BadAmount { amount } => vec![("amount", Value::Real(*amount))],
            StatsError::Time(_) | StatsError::NotRegistered { .. } => Vec::new(),
        }
    }
}

/// One attribute of one entity.
#[derive(Clone, Copy, PartialEq, Debug, Serialize, Deserialize)]
struct AttrInst {
    /// Which attribute.
    attr: AttributeId,
    /// The entity's own base, in place of the definition's.
    base: Option<Real>,
    /// The value, modifiers applied. Derived, and cached.
    value: Real,
}

impl_state_hash!(AttrInst { attr, base, value });

/// One track of one entity.
#[derive(Clone, Copy, PartialEq, Debug, Serialize, Deserialize)]
struct TrackInst {
    /// Which track.
    track: TrackId,
    /// What has accumulated.
    amount: Real,
    /// The level reached.
    level: u32,
}

impl_state_hash!(TrackInst {
    track,
    amount,
    level
});

/// One entity's attributes, pools and tracks (06 §4): the `Stats`
/// component.
///
/// - It holds a value for **every attribute** defined, from the moment
///   it is [attached](attach): one never given a base of its own or a
///   modifier has its definition's base. Values are cached, and are
///   always what [`refresh`] would work out from scratch (06 invariant
///   B3).
/// - It holds the **pools** it was [given](add_pool) and no others.
/// - A **track** it was never advanced on is at level 0 with nothing
///   accumulated.
///
/// Its fields change only through the functions of this module, which
/// report what changed. Everything in it is saved and hashed, the cached
/// values included.
#[derive(Clone, PartialEq, Debug, Serialize, Deserialize)]
pub struct Stats {
    /// The clock pools run on: the owner's.
    clock: ClockId,
    /// Whether the owner is out of the simulation: pools stand still and
    /// schedule nothing until [`resume`].
    suspended: bool,
    /// The attributes, in order of id.
    attrs: Vec<AttrInst>,
    /// Every modifier, in order of attribute and then source; those of
    /// one attribute and source in the order they were added.
    modifiers: Vec<Modifier>,
    /// The pools, in order of id.
    pools: Vec<PoolInst>,
    /// The tracks advanced so far, in order of id.
    tracks: Vec<TrackInst>,
}

impl StateHash for Stats {
    fn state_hash(&self, hasher: &mut StateHasher) {
        self.clock.state_hash(hasher);
        hasher.write_bool(self.suspended);
        self.attrs.state_hash(hasher);
        self.modifiers.state_hash(hasher);
        self.pools.state_hash(hasher);
        self.tracks.state_hash(hasher);
    }
}

impl Stats {
    /// Stats with nothing in them yet, on a clock: no attribute has a
    /// value until [`refresh`] works them out. To give an entity stats,
    /// use [`attach`], which does both. This is for stats made apart
    /// from a store.
    pub fn new(clock: ClockId) -> Stats {
        Stats {
            clock,
            suspended: false,
            attrs: Vec::new(),
            modifiers: Vec::new(),
            pools: Vec::new(),
            tracks: Vec::new(),
        }
    }

    /// The clock the pools run on.
    pub fn clock(&self) -> ClockId {
        self.clock
    }

    /// Whether the pools stand still: the owner is out of the simulation
    /// ([`suspend`]).
    pub fn is_suspended(&self) -> bool {
        self.suspended
    }

    fn attr_at(&self, attr: AttributeId) -> Result<usize, usize> {
        self.attrs.binary_search_by_key(&attr, |inst| inst.attr)
    }

    /// The value of an attribute, modifiers applied. `None` only for an
    /// attribute defined since these stats were last [refreshed](refresh).
    pub fn attr(&self, attr: AttributeId) -> Option<Real> {
        self.attr_at(attr).ok().map(|at| self.attrs[at].value)
    }

    /// The entity's own base for an attribute, if it was given one in
    /// place of the definition's.
    pub fn own_base(&self, attr: AttributeId) -> Option<Real> {
        self.attr_at(attr).ok().and_then(|at| self.attrs[at].base)
    }

    /// Every modifier, in order of attribute and then source.
    pub fn modifiers(&self) -> &[Modifier] {
        &self.modifiers
    }

    /// The modifiers of one attribute, in the order they are applied.
    pub fn modifiers_of(&self, attr: AttributeId) -> &[Modifier] {
        let start = self
            .modifiers
            .partition_point(|modifier| modifier.attr < attr);
        let end = self
            .modifiers
            .partition_point(|modifier| modifier.attr <= attr);
        &self.modifiers[start..end]
    }

    fn pool_at(&self, pool: PoolId) -> Result<usize, usize> {
        self.pools.binary_search_by_key(&pool, |inst| inst.pool)
    }

    fn pool_inst(&self, pool: PoolId) -> Option<&PoolInst> {
        self.pool_at(pool).ok().map(|at| &self.pools[at])
    }

    /// The local time on the stats' clock, or `None` if the clock is
    /// gone. While suspended, time stands still for each pool.
    fn local_now(&self, timeline: &Timeline) -> Option<clash_core::Dur> {
        if self.suspended {
            return None;
        }
        timeline.local_now(self.clock).ok()
    }

    /// Whether the entity has this pool.
    pub fn has_pool(&self, pool: PoolId) -> bool {
        self.pool_at(pool).is_ok()
    }

    /// The pools the entity has, in order of id.
    pub fn pools(&self) -> impl Iterator<Item = PoolId> + '_ {
        self.pools.iter().map(|inst| inst.pool)
    }

    /// A pool's level at the timeline's present moment.
    pub fn pool_level(&self, timeline: &Timeline, pool: PoolId) -> Option<Real> {
        let inst = self.pool_inst(pool)?;
        Some(match self.local_now(timeline) {
            Some(now) => inst.level_at(now),
            None => inst.level_as_synced(),
        })
    }

    /// A pool's level as a fraction of the most it holds, from 0 to 1. A
    /// pool that holds nothing is at 0.
    pub fn pool_frac(&self, timeline: &Timeline, pool: PoolId) -> Option<Real> {
        let level = self.pool_level(timeline, pool)?;
        let max = self.pool_max(pool)?;
        Some(if max > Real::ZERO {
            level / max
        } else {
            Real::ZERO
        })
    }

    /// The most a pool holds.
    pub fn pool_max(&self, pool: PoolId) -> Option<Real> {
        self.pool_inst(pool).map(PoolInst::max)
    }

    /// A pool's net rate: its regeneration and every rate applied,
    /// summed, per second of the owner's time.
    pub fn pool_rate(&self, pool: PoolId) -> Option<Real> {
        self.pool_inst(pool).map(PoolInst::net)
    }

    /// The rates applied to a pool, in order of source. Its own
    /// regeneration is not among them.
    pub fn pool_rates(&self, pool: PoolId) -> &[PoolRate] {
        self.pool_inst(pool).map_or(&[], PoolInst::rates)
    }

    /// How many of a pool's thresholds its level counts as at or above.
    pub fn pool_band(&self, pool: PoolId) -> Option<usize> {
        self.pool_inst(pool).map(PoolInst::band)
    }

    /// The wake-up a pool has scheduled for its next crossing, if it has
    /// one ahead.
    pub fn pool_wake_id(&self, pool: PoolId) -> Option<WakeId> {
        self.pool_inst(pool).and_then(PoolInst::wake)
    }

    fn track_at(&self, track: TrackId) -> Result<usize, usize> {
        self.tracks.binary_search_by_key(&track, |inst| inst.track)
    }

    /// A track's level: 0 for one never advanced.
    pub fn track_level(&self, track: TrackId) -> u32 {
        self.track_at(track).map_or(0, |at| self.tracks[at].level)
    }

    /// What a track has accumulated: 0 for one never advanced.
    pub fn track_amount(&self, track: TrackId) -> Real {
        self.track_at(track)
            .map_or(Real::ZERO, |at| self.tracks[at].amount)
    }
}

/// The stats of an entity, or the refusal that it has none.
fn stats_of(stats: &StatsStore, entity: EntityId) -> Result<&Stats, StatsError> {
    stats.get(entity).ok_or(StatsError::NoStats { entity })
}

/// The stats of an entity to change, or the refusal that it has none.
fn stats_mut(stats: &mut StatsStore, entity: EntityId) -> Result<&mut Stats, StatsError> {
    stats.get_mut(entity).ok_or(StatsError::NoStats { entity })
}

/// Takes away everything a source added to an entity: its modifiers and
/// its pool rates, and nothing else (06 §4.1). The attributes and pools
/// it touched are brought up to date.
pub fn remove_source<H: Host + ?Sized>(
    cx: &mut StatsCx<'_, H>,
    entity: EntityId,
    source: Source,
) -> Result<Changes, StatsError> {
    let mut out = Changes::new();
    attr::drop_modifiers(cx, entity, source, None, &mut out)?;
    pool::drop_rates(cx, entity, source, &mut out)?;
    Ok(out)
}
