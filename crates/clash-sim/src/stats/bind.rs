//! Stats in the running simulation: the functions of
//! [`stats`](mod@crate::stats) called through [`Sim`], with their events
//! raised (06 §4).
//!
//! | The simulation | Does |
//! |----------------|------|
//! | [`Sim::attach_stats`] | Gives an entity stats on the clock it is on. One that is not in the world gets them standing still |
//! | [`Sim::give_pool`], [`Sim::take_pool`] | Gives and takes a pool |
//! | [`Sim::set_attr_base`], [`Sim::add_modifier`], [`Sim::remove_modifier`], [`Sim::remove_stat_source`] | Change attributes |
//! | [`Sim::pool_add`] | Raises the hook `pool_changing`, then adds what its handlers left of the amount |
//! | [`Sim::pool_spend`], [`Sim::pool_rate`] | A cost in full or not at all; a rate under a source |
//! | [`Sim::advance_track`] | Adds to a track |
//! | [`Sim::refresh_stats`] | Works every attribute and pool out again: called when something a stat reads changed that is not an attribute |
//!
//! Every one raises `attr_changed`, `pool_crossed` and `track_advanced`
//! from what the function returned, in order, and then lets the
//! entity's definitions say whether it still goes on
//! ([`Sim::settle_vital`](crate::Sim::settle_vital)). A refusal is the
//! stats' own, as a [`Reason`], and changes nothing.
//!
//! # When stats are worked out again
//!
//! A stat's expression may read more than attributes (06 §4.1). The
//! wiring calls [`Sim::refresh_stats`] when a function of the entity's
//! body moved, when its mass or the mass of what it holds changed, and
//! when one of its variables changed that an attribute's `base` or a
//! pool's `max` or `regen` reads directly. Anything else such an
//! expression reads is not followed.
//!
//! # Not built
//!
//! A track's `on_advance` is kept as written and not run: it waits for
//! the rule runtime to be bound to the simulation.

use std::collections::BTreeSet;

use clash_core::{EntityId, Real};
use clash_data::Def;
use clash_time::ClockId;
use clash_world::Cause;

use crate::Sim;
use crate::event::Where;
use crate::life::events::{about, id_field};
use crate::life::stats_reason;
use crate::reason::Reason;
use crate::stats::{
    self, AttributeDef, AttributeId, Change, Changes, Modifier, PoolDef, PoolId, Source, StatDefs,
    StatsError, TrackDef, TrackId,
};

/// The qualified names of the variables that an attribute's `base` or a
/// pool's `max` or `regen` reads directly: a change to one of them on an
/// entity with stats works its stats out again.
pub(crate) fn variables_read(defs: &StatDefs) -> BTreeSet<String> {
    let mut read = BTreeSet::new();
    let mut note = |code: Option<&clash_expr::Compiled>| {
        if let Some(code) = code {
            read.extend(code.variables_read().into_iter().map(str::to_owned));
        }
    };
    for (_, def) in defs.defs().registry::<AttributeDef>().iter() {
        note(def.base.compiled());
    }
    for (_, def) in defs.defs().registry::<PoolDef>().iter() {
        note(def.max.compiled());
        note(def.regen.compiled());
    }
    read
}

impl Sim {
    /// The refusal that an entity has no stats, or not this pool.
    fn check_pool(&self, entity: EntityId, pool: PoolId) -> Result<(), Reason> {
        self.known(entity)?;
        let defs = self.life.stat_defs()?;
        let refusal = match self.components.stats.get(entity) {
            None => StatsError::NoStats { entity },
            Some(stats) if stats.has_pool(pool) => return Ok(()),
            Some(_) => match defs.pool(pool) {
                Some(_) => StatsError::NoSuchPool {
                    entity,
                    pool,
                    name: defs.defs().registry::<PoolDef>().name(pool).to_owned(),
                },
                None => StatsError::UnknownStat {
                    what: PoolDef::TYPE,
                },
            },
        };
        Err(stats_reason(&refusal))
    }

    /// Runs a function of the stats that may evaluate their expressions
    /// and raises the events of what it changed.
    fn change_stats(
        &mut self,
        entity: EntityId,
        cause: Cause,
        change: impl FnOnce(&mut stats::StatsCx<'_, crate::SimHost<'_>>) -> Result<Changes, StatsError>,
    ) -> Result<(), Reason> {
        self.known(entity)?;
        let changes = self
            .with_stats(entity, change)?
            .map_err(|error| stats_reason(&error))?;
        self.raise_stat_changes(entity, changes, cause);
        Ok(())
    }

    /// Gives an entity stats (06 §4): every attribute at its base, no
    /// pools, no tracks. Its pools will run on the clock it is on; an
    /// entity that is not in the world is given them standing still,
    /// and they start when it enters it.
    ///
    /// Refused for an unknown entity and for one that has stats.
    pub fn attach_stats(&mut self, entity: EntityId, _cause: Cause) -> Result<(), Reason> {
        self.known(entity)?;
        let clock = self.life_clock(entity);
        self.with_stats(entity, |cx| {
            stats::attach(cx, entity, clock.unwrap_or(ClockId::UNIVERSAL))?;
            if clock.is_none() {
                stats::suspend(cx, entity)?;
            }
            Ok(())
        })?
        .map_err(|error: StatsError| stats_reason(&error))
    }

    /// Gives an entity a pool, at its definition's `start` (06 §4.2).
    /// Refused if it has no stats or has the pool already.
    pub fn give_pool(
        &mut self,
        entity: EntityId,
        pool: PoolId,
        cause: Cause,
    ) -> Result<(), Reason> {
        self.change_stats(entity, cause, |cx| {
            stats::add_pool(cx, entity, pool).map(|()| Changes::new())
        })?;
        self.settle_vital(entity, cause);
        Ok(())
    }

    /// Takes a pool away from an entity, with its rates.
    pub fn take_pool(
        &mut self,
        entity: EntityId,
        pool: PoolId,
        cause: Cause,
    ) -> Result<(), Reason> {
        self.change_stats(entity, cause, |cx| {
            stats::remove_pool(cx, entity, pool).map(|()| Changes::new())
        })?;
        self.settle_vital(entity, cause);
        Ok(())
    }

    /// Works every attribute and every pool of an entity out from
    /// scratch (06 §4.1): what to call when something a stat's
    /// expression reads has changed that is not an attribute.
    pub fn refresh_stats(&mut self, entity: EntityId, cause: Cause) -> Result<(), Reason> {
        self.change_stats(entity, cause, |cx| stats::refresh(cx, entity))
    }

    /// The verb `set_attr_base` (06 §4.4): gives an entity a base of its
    /// own for an attribute, or with `None` gives the definition's back.
    pub fn set_attr_base(
        &mut self,
        entity: EntityId,
        attr: AttributeId,
        base: Option<Real>,
        cause: Cause,
    ) -> Result<(), Reason> {
        if let Some(base) = base
            && !base.is_finite()
        {
            return Err(stats_reason(&StatsError::BadAmount { amount: base }));
        }
        self.change_stats(entity, cause, |cx| stats::set_base(cx, entity, attr, base))
    }

    /// The verb `add_modifier` (06 §4.4): adds a modifier to an
    /// attribute under its source.
    pub fn add_modifier(
        &mut self,
        entity: EntityId,
        modifier: Modifier,
        cause: Cause,
    ) -> Result<(), Reason> {
        for amount in [modifier.add, modifier.mul] {
            if !amount.is_finite() {
                return Err(stats_reason(&StatsError::BadAmount { amount }));
            }
        }
        self.change_stats(entity, cause, |cx| {
            stats::add_modifier(cx, entity, modifier)
        })
    }

    /// The verb `remove_modifier` (06 §4.4): removes the modifiers a
    /// source added, to one attribute or with `None` to all.
    pub fn remove_modifier(
        &mut self,
        entity: EntityId,
        source: Source,
        attr: Option<AttributeId>,
        cause: Cause,
    ) -> Result<(), Reason> {
        self.change_stats(entity, cause, |cx| match attr {
            Some(attr) => stats::remove_modifier(cx, entity, source, attr),
            None => stats::remove_modifiers(cx, entity, source),
        })
    }

    /// Takes away everything a source added to an entity: its modifiers
    /// and its pool rates (06 §4.1). What a trait, an item or an ability
    /// calls when it goes.
    pub fn remove_stat_source(
        &mut self,
        entity: EntityId,
        source: Source,
        cause: Cause,
    ) -> Result<(), Reason> {
        self.change_stats(entity, cause, |cx| stats::remove_source(cx, entity, source))
    }

    /// The verb `pool_add` (06 §4.4): adds to a pool, negative to drain,
    /// after the hook `pool_changing`, whose handlers may change the
    /// amount or cancel. Returns what was in fact added, which is less
    /// where the pool's bounds stopped it.
    ///
    /// Refused, changing nothing, if the entity has no such pool, the
    /// amount is not finite, or a handler cancelled; the reason is then
    /// the handler's.
    pub fn pool_add(
        &mut self,
        entity: EntityId,
        pool: PoolId,
        amount: Real,
        cause: Cause,
    ) -> Result<Real, Reason> {
        self.check_pool(entity, pool)?;
        if !amount.is_finite() {
            return Err(stats_reason(&StatsError::BadAmount { amount }));
        }
        let defs = self.life.stat_defs()?;
        let payload = about(stats::props::POOL_CHANGING, entity)
            .with("pool", id_field(Some(defs.def_types().pool), pool.index()))
            .with("amount", amount);
        let raised = self.raise(Where::Entity(entity), cause, payload)?;
        if let Some(reason) = raised.cancelled {
            return Err(reason);
        }
        let amount = raised
            .field("amount")
            .and_then(crate::FieldValue::as_real)
            .filter(|amount| amount.is_finite())
            .unwrap_or(amount);
        self.dose_pool(entity, pool, amount, cause)
    }

    /// Adds to a pool without the hook `pool_changing`: for what is not
    /// a verb (a dose from a field, 02 §16). Returns what was added.
    pub(crate) fn dose_pool(
        &mut self,
        entity: EntityId,
        pool: PoolId,
        amount: Real,
        cause: Cause,
    ) -> Result<Real, Reason> {
        let added = self
            .with_plain_stats(|cx| stats::pool_add(cx, entity, pool, amount))?
            .map_err(|error| stats_reason(&error))?;
        self.raise_stat_changes(entity, added.changes, cause);
        Ok(added.applied)
    }

    /// Takes an amount from a pool in full or not at all (08 §5.2).
    /// Refused with `pool_too_low` if the pool holds less.
    pub fn pool_spend(
        &mut self,
        entity: EntityId,
        pool: PoolId,
        amount: Real,
        cause: Cause,
    ) -> Result<(), Reason> {
        self.check_pool(entity, pool)?;
        let changes = self
            .with_plain_stats(|cx| stats::pool_spend(cx, entity, pool, amount))?
            .map_err(|error| stats_reason(&error))?;
        self.raise_stat_changes(entity, changes, cause);
        Ok(())
    }

    /// The verb `pool_rate` (06 §4.4): applies a rate to a pool under a
    /// source, replacing the one that source had applied; `None` takes
    /// it away.
    pub fn pool_rate(
        &mut self,
        entity: EntityId,
        pool: PoolId,
        source: Source,
        rate: Option<Real>,
        cause: Cause,
    ) -> Result<(), Reason> {
        self.check_pool(entity, pool)?;
        let changes = self
            .with_plain_stats(|cx| match rate {
                Some(rate) => stats::set_rate(cx, entity, pool, source, rate),
                None => stats::remove_rate(cx, entity, pool, source),
            })?
            .map_err(|error| stats_reason(&error))?;
        self.raise_stat_changes(entity, changes, cause);
        Ok(())
    }

    /// The verb `advance_track` (06 §4.4): adds to a track's counter.
    /// `track_advanced` is raised once for each level reached.
    pub fn advance_track(
        &mut self,
        entity: EntityId,
        track: TrackId,
        amount: Real,
        cause: Cause,
    ) -> Result<(), Reason> {
        self.change_stats(entity, cause, |cx| {
            stats::advance_track(cx, entity, track, amount)
        })
    }

    /// Runs a function of the stats that evaluates nothing, over the
    /// stores as they stand.
    pub(crate) fn with_plain_stats<R>(
        &mut self,
        run: impl FnOnce(&mut stats::StatsCx<'_, crate::life::NullHost>) -> R,
    ) -> Result<R, Reason> {
        let defs = self.life.stat_defs()?;
        let mut host = crate::life::NullHost;
        Ok(run(&mut stats::StatsCx {
            stats: &mut self.components.stats,
            defs: &defs,
            timeline: &mut self.timeline,
            host: &mut host,
            runner: &mut self.life.runner,
        }))
    }

    /// Raises the events of what a function of the stats changed, in
    /// order (06 §4.4), and then lets the entity's definitions say
    /// whether it still goes on, if a pool crossed a threshold.
    pub(crate) fn raise_stat_changes(&mut self, entity: EntityId, changes: Changes, cause: Cause) {
        if changes.is_empty() {
            return;
        }
        let Ok(defs) = self.life.stat_defs() else {
            return;
        };
        let types = defs.def_types();
        let mut crossed = false;
        for change in changes {
            let payload = match change {
                Change::AttrChanged {
                    entity,
                    attr,
                    old,
                    new,
                } => about(stats::props::ATTR_CHANGED, entity)
                    .with("attr", id_field(Some(types.attribute), attr.index()))
                    .with("old", old)
                    .with("new", new),
                Change::PoolCrossed {
                    entity,
                    pool,
                    threshold,
                    rising,
                    level,
                } => {
                    crossed = true;
                    about(stats::props::POOL_CROSSED, entity)
                        .with("pool", id_field(Some(types.pool), pool.index()))
                        .with("threshold", threshold)
                        .with("rising", rising)
                        .with("level", level)
                }
                Change::TrackAdvanced {
                    entity,
                    track,
                    level,
                } => about(stats::props::TRACK_ADVANCED, entity)
                    .with("track", id_field(Some(types.track), track.index()))
                    .with("level", i64::from(level)),
            };
            if let Err(reason) = self.raise(Where::Entity(entity), cause, payload) {
                self.fault(reason);
            }
        }
        if crossed {
            self.settle_vital(entity, cause);
        }
    }

    /// Works an entity's stats out again because one of its variables
    /// changed, if a stat's expression reads that variable directly, and
    /// makes it notable.
    pub(crate) fn life_var_changed(&mut self, entity: EntityId, name: &str, cause: Cause) {
        // A variable set on an anonymous individual makes it notable
        // (06 §12.2).
        self.make_notable(entity, cause);
        if self.components.stats.contains(entity)
            && self.life.reads_variable(name)
            && let Err(reason) = self.refresh_stats(entity, cause)
        {
            self.fault(reason);
        }
    }

    /// Works an entity's stats out again because its mass, or the mass
    /// of what it holds, changed.
    pub(crate) fn life_mass_changed(&mut self, entity: EntityId, cause: Cause) {
        if self.components.stats.contains(entity)
            && let Err(reason) = self.refresh_stats(entity, cause)
        {
            self.fault(reason);
        }
    }
}

// The three stat definition types are named in the documentation above.
const _: (&str, &str, &str) = (AttributeDef::TYPE, PoolDef::TYPE, TrackDef::TYPE);
