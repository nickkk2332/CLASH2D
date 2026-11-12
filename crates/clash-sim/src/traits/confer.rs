//! What a trait confers on its holder (06 §5), applied when it is added
//! or gains a stack and taken back exactly when it goes.
//!
//! | What | Kept | Taken back by |
//! |------|------|---------------|
//! | Attribute modifiers, once for each stack | By the stats, under the instance's [`source_of`] | `stats::remove_source` |
//! | Pool rates, times the stacks | By the stats, under the same source | The same |
//! | Tags | On the entity, and in [`Traits`] where the entity did not carry them of its own | Settling the tags against what the instances want |
//! | Attached rules | In the rule state, and in [`Traits`] where they were not attached already | Settling the rules the same way |
//!
//! A dormant instance confers nothing. What the instances confer is
//! always what [`conferred`] works out from scratch.
//!
//! # The seam to stats
//!
//! [`with_stats`] is the one place this module reaches the stats, as it
//! is for items (`item::confer`): their tables are built for the call,
//! and the store and the timeline are taken out of the simulation while
//! it runs. When the kernel offers its own way to change stats, that
//! function is the one place to change.

use std::collections::BTreeSet;
use std::sync::Arc;

use clash_core::{EntityId, Real};
use clash_data::Runner;
use clash_time::Timeline;

use super::{TraitDef, TraitId, TraitInst, TraitKey, Traits, source_of};
use crate::Sim;
use crate::action::ChannelId;
use crate::event::Subject;
use crate::host::{Bound, SimHost};
use crate::item::Holds;
use crate::rule::EocId;
use crate::stats::{self, Changes, Modifier, PoolId, Source, StatDefs, Stats, StatsCx};

/// What the trait instances of an entity confer on it, worked out from
/// scratch.
#[derive(Clone, PartialEq, Debug, Default)]
pub struct Conferred {
    /// The attribute modifiers, in the order the stats keep them: by
    /// attribute and then source, those of one instance in the order its
    /// definition lists them, once for each stack.
    pub modifiers: Vec<Modifier>,
    /// The pool rates, by pool and then source: only for pools the
    /// entity has.
    pub rates: Vec<(PoolId, Source, Real)>,
    /// The tags, in order.
    pub tags: Vec<String>,
    /// The rules to attach, in order of id.
    pub rules: Vec<EocId>,
}

/// The definition of a trait.
pub(super) fn def_of(sim: &Sim, def: TraitId) -> Option<&TraitDef> {
    sim.defs.try_registry::<TraitDef>()?.try_get(def)
}

/// The rules a definition carries that can be attached to an entity.
fn attachable(sim: &Sim, def: &TraitDef) -> Vec<EocId> {
    let rules = sim.defs.try_registry::<crate::rule::EocDef>();
    def.rules
        .iter()
        .map(|rule| rule.id())
        .filter(|rule| {
            rules
                .and_then(|rules| rules.try_get(*rule))
                .is_some_and(crate::rule::EocDef::is_attached)
        })
        .collect()
}

/// What the trait instances of an entity confer on it, worked out from
/// scratch from its instances and their definitions. What its stats
/// hold from traits, the tags it has because of them and the rules
/// attached because of them always equal this.
pub fn conferred(sim: &Sim, entity: EntityId) -> Conferred {
    let mut out = Conferred::default();
    let Some(traits) = sim.components.get::<Traits>(entity) else {
        return out;
    };
    let stats = sim.components.get::<Stats>(entity);
    let mut tags = BTreeSet::new();
    let mut rules = BTreeSet::new();
    for inst in &traits.insts {
        if inst.dormant {
            continue;
        }
        let Some(def) = def_of(sim, inst.def) else {
            continue;
        };
        let source = source_of(inst.key);
        if stats.is_some() {
            for _ in 0..inst.stacks {
                for modifier in &def.modifiers {
                    out.modifiers.push(Modifier {
                        attr: modifier.attr.id(),
                        source,
                        add: modifier.add,
                        mul: modifier.mul,
                    });
                }
            }
        }
        if let Some(stats) = stats {
            for rate in &def.rates {
                if stats.has_pool(rate.pool.id()) {
                    out.rates.push((
                        rate.pool.id(),
                        source,
                        rate.rate * Real::from_i64(i64::from(inst.stacks)),
                    ));
                }
            }
        }
        tags.extend(def.tags.iter().cloned());
        rules.extend(attachable(sim, def));
    }
    // Stable: those of one source keep the order they were added in.
    out.modifiers
        .sort_by_key(|modifier| (modifier.attr, modifier.source));
    // One rate for each pool and source, as the stats keep them; two
    // rates of one instance on one pool add up.
    out.rates.sort_by_key(|(pool, source, _)| (*pool, *source));
    out.rates.dedup_by(|later, earlier| {
        if (later.0, later.1) == (earlier.0, earlier.1) {
            earlier.2 += later.2;
            true
        } else {
            false
        }
    });
    out.tags = tags.into_iter().collect();
    out.rules = rules.into_iter().collect();
    out
}

/// The net change to a channel's slots that an entity's traits make
/// while they last (03 §6.2): the sum of what each instance that is not
/// dormant adds (positive) or disables (negative). For the action
/// runtime, which applies it to the slots the body supplies.
pub fn channel_change(sim: &Sim, entity: EntityId, channel: ChannelId) -> i32 {
    let Some(traits) = sim.components.get::<Traits>(entity) else {
        return 0;
    };
    traits
        .insts
        .iter()
        .filter(|inst| !inst.dormant)
        .filter_map(|inst| def_of(sim, inst.def))
        .flat_map(|def| def.channels.iter())
        .filter(|(have, _)| have.id() == channel)
        .map(|(_, count)| i32::from(*count))
        .sum()
}

/// Runs a change to the stats: see the [module documentation](self).
/// `None` if the definitions were loaded without the stats' types.
pub(crate) fn with_stats<R>(
    sim: &mut Sim,
    run: impl FnOnce(&mut StatsCx<'_, SimHost<'_>>) -> R,
) -> Option<R> {
    let defs = StatDefs::new(&sim.defs).ok()?;
    let defs = &defs;
    let mut store = core::mem::take(&mut sim.components.stats);
    let mut timeline = core::mem::replace(&mut sim.timeline, Timeline::new());
    let mut runner = Runner::new();
    let result = {
        let mut host = sim.host(Bound::new());
        let mut cx = StatsCx {
            stats: &mut store,
            defs,
            timeline: &mut timeline,
            host: &mut host,
            runner: &mut runner,
        };
        run(&mut cx)
    };
    sim.timeline = timeline;
    sim.components.stats = store;
    Some(result)
}

/// Receives what conferring changed of an entity's stats. The events
/// those changes are reported by (`attr_changed`, `pool_crossed`) are
/// the stats' own to raise once the kernel raises them; until then the
/// changes end here.
fn stat_changes(_sim: &mut Sim, _changes: Changes) {}

/// An instance of an entity, by its key.
fn inst(sim: &Sim, entity: EntityId, key: TraitKey) -> Option<TraitInst> {
    sim.components.get::<Traits>(entity)?.get(key).cloned()
}

/// Brings the stats of an entity into line with one instance: whatever
/// was kept under its source is taken away, and what it confers now (with
/// its stacks, unless it is dormant or gone) is applied.
pub(super) fn restate(sim: &mut Sim, entity: EntityId, key: TraitKey) {
    if !sim.components.stats.contains(entity) {
        return;
    }
    let source = source_of(key);
    let wanted = inst(sim, entity, key).filter(|inst| !inst.dormant);
    let defs = Arc::clone(&sim.defs);
    let def = wanted
        .as_ref()
        .and_then(|inst| defs.try_registry::<TraitDef>()?.try_get(inst.def));
    let had = sim.components.get::<Stats>(entity).is_some_and(|stats| {
        stats
            .modifiers()
            .iter()
            .any(|modifier| modifier.source == source)
            || stats.pools().any(|pool| {
                stats
                    .pool_rates(pool)
                    .iter()
                    .any(|rate| rate.source == source)
            })
    });
    let adds = def.is_some_and(|def| !def.modifiers.is_empty() || !def.rates.is_empty());
    if !had && !adds {
        return;
    }
    let stacks = wanted.as_ref().map_or(0, |inst| inst.stacks);
    let changes = with_stats(sim, |cx| {
        let mut out = Changes::new();
        if had {
            out.extend(stats::remove_source(cx, entity, source).unwrap_or_default());
        }
        let Some(def) = def else {
            return out;
        };
        for _ in 0..stacks {
            for modifier in &def.modifiers {
                let modifier = Modifier {
                    attr: modifier.attr.id(),
                    source,
                    add: modifier.add,
                    mul: modifier.mul,
                };
                // An attribute that is gone since the trait was defined
                // is not modified.
                if let Ok(changes) = stats::add_modifier(cx, entity, modifier) {
                    out.extend(changes);
                }
            }
        }
        // Two rates of one trait on one pool are one rate under its
        // source, as `conferred` works it out.
        let mut rates: Vec<(PoolId, Real)> = Vec::new();
        for rate in &def.rates {
            let amount = rate.rate * Real::from_i64(i64::from(stacks));
            match rates.iter_mut().find(|(pool, _)| *pool == rate.pool.id()) {
                Some((_, sum)) => *sum += amount,
                None => rates.push((rate.pool.id(), amount)),
            }
        }
        for (pool, amount) in rates {
            // A pool the entity does not have is not drained.
            if let Ok(changes) = stats::set_rate(cx, entity, pool, source, amount) {
                out.extend(changes);
            }
        }
        out
    });
    stat_changes(sim, changes.unwrap_or_default());
}

/// Brings the tags an entity has because of its traits into line with
/// what its instances want: a tag it did not carry of its own is added
/// and recorded; a recorded tag no instance wants is taken off, unless
/// an item it holds confers it.
pub(super) fn settle_tags(sim: &mut Sim, entity: EntityId) {
    let wanted = conferred(sim, entity).tags;
    let item_tags: Vec<String> = sim
        .components
        .get::<Holds>(entity)
        .map(|holds| holds.conferred_tags().to_vec())
        .unwrap_or_default();
    let Some(traits) = sim.components.get_mut::<Traits>(entity) else {
        return;
    };
    let Some(record) = sim.entities.get_mut(entity) else {
        return;
    };
    let mut gone = Vec::new();
    traits.tags.retain(|tag| {
        let keep = wanted.binary_search(tag).is_ok();
        if !keep {
            gone.push(tag.clone());
        }
        keep
    });
    for tag in gone {
        if item_tags.binary_search(&tag).is_err() {
            record.tags.remove(&tag);
        }
    }
    for tag in wanted {
        let Err(at) = traits.tags.binary_search(&tag) else {
            continue;
        };
        // A tag carried of its own (not because of an item) is the
        // entity's, and is not counted.
        let own = record.tags.contains(&tag) && item_tags.binary_search(&tag).is_err();
        if own {
            continue;
        }
        record.tags.insert(tag.clone());
        traits.tags.insert(at, tag);
    }
}

/// Brings the rules attached to an entity because of its traits into
/// line with what its instances want: a rule not attached already is
/// attached and recorded; a recorded rule no instance wants is detached.
pub(super) fn settle_rules(sim: &mut Sim, entity: EntityId) {
    let wanted = conferred(sim, entity).rules;
    let Some(handle) = sim.exprs.kinds().handle(Subject::Entity(entity)) else {
        return;
    };
    let Some(traits) = sim.components.get_mut::<Traits>(entity) else {
        return;
    };
    let rules = &mut sim.rules;
    traits.rules.retain(|rule| {
        let keep = wanted.binary_search(rule).is_ok();
        if !keep {
            rules.detach(handle, *rule);
        }
        keep
    });
    for rule in wanted {
        if let Err(at) = traits.rules.binary_search(&rule)
            && rules.attach(handle, rule)
        {
            traits.rules.insert(at, rule);
        }
    }
}

/// Brings everything an entity has from its traits into line with
/// them, after `changed` instances were added, restacked, made dormant
/// or removed.
pub(super) fn settle(sim: &mut Sim, entity: EntityId, changed: &[TraitKey]) {
    for key in changed {
        restate(sim, entity, *key);
    }
    settle_tags(sim, entity);
    settle_rules(sim, entity);
}
