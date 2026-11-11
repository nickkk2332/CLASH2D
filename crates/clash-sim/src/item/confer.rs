//! What a held or worn item confers on its holder (06 §9.1): attribute
//! modifiers, under the item as their source (06 §4.1), and tags.
//!
//! It is applied when the item enters a grasp or is put on, and taken
//! back when it leaves, whoever moved it: the one call that ends a hold,
//! [`left_holder`](super::left_holder), is made by the kernel's
//! `relocate`. What a holder has from its items is therefore always what
//! [`conferred`] works out from scratch.
//!
//! # The seam to stats
//!
//! [`with_stats`] is the one place this package reaches the stats. The
//! stats' functions want the store of `Stats`, the timeline and a host
//! over the rest of the simulation, all at once, and the kernel does not
//! hand those out yet. So the store and the timeline are taken out of
//! the simulation for the length of the call and put back. While they
//! are out, a derived attribute that reads the time or another entity's
//! stats reads nothing; one that reads its owner's attributes, a body
//! function or a mass reads what it should.

use std::collections::BTreeSet;
use std::sync::Arc;

use clash_core::EntityId;
use clash_data::Runner;
use clash_time::Timeline;

use super::{Holds, Item, def_of};
use crate::Sim;
use crate::host::{Bound, SimHost};
use crate::stats::{self, Changes, Modifier, Source, SourceKind, StatDefs, Stats, StatsCx};

/// The source an item's modifiers are added under.
pub(crate) fn source_of(item: EntityId) -> Source {
    Source::new(SourceKind::Item, item.to_bits())
}

/// What the things an entity holds confer on it.
#[derive(Clone, PartialEq, Debug, Default)]
pub struct Conferred {
    /// The modifiers, in the order the stats keep them: by attribute
    /// and then source, and those of one item in the order its
    /// definition lists them.
    pub modifiers: Vec<Modifier>,
    /// The tags, in order.
    pub tags: Vec<String>,
}

/// What the things an entity holds in a grasp or wears confer on it,
/// worked out from scratch from what it holds and their definitions.
/// The modifiers its stats hold from items, and the tags it has because
/// of them, always equal this.
pub fn conferred(sim: &Sim, holder: EntityId) -> Conferred {
    let mut modifiers = Vec::new();
    let mut tags = BTreeSet::new();
    if let Some(holds) = sim.components.get::<Holds>(holder) {
        for (entity, hold) in holds.iter() {
            if !hold.in_use() {
                continue;
            }
            let Some(def) = sim
                .components
                .get::<Item>(entity)
                .and_then(|item| def_of(&sim.defs, item))
            else {
                continue;
            };
            let source = source_of(entity);
            for modifier in &def.modifiers {
                modifiers.push(Modifier {
                    attr: modifier.attr.id(),
                    source,
                    add: modifier.add,
                    mul: modifier.mul,
                });
            }
            tags.extend(def.holder_tags.iter().cloned());
        }
    }
    modifiers.sort_by_key(|modifier| (modifier.attr, modifier.source));
    Conferred {
        modifiers,
        tags: tags.into_iter().collect(),
    }
}

/// Runs a change to the stats: see the [module documentation](self).
/// `None` if the definitions were loaded without the stats' types.
fn with_stats<R>(sim: &mut Sim, run: impl FnOnce(&mut StatsCx<'_, SimHost<'_>>) -> R) -> Option<R> {
    // Built for the call: see the module documentation.
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

/// Applies what an item confers to a holder it has just come into use
/// by.
pub(super) fn add(sim: &mut Sim, holder: EntityId, item: EntityId) {
    let defs = Arc::clone(&sim.defs);
    let Some(def) = sim
        .components
        .get::<Item>(item)
        .and_then(|item| def_of(&defs, item))
    else {
        return;
    };
    if !def.modifiers.is_empty() && sim.components.stats.contains(holder) {
        let source = source_of(item);
        let changes = with_stats(sim, |cx| {
            let mut out = Changes::new();
            for modifier in &def.modifiers {
                let modifier = Modifier {
                    attr: modifier.attr.id(),
                    source,
                    add: modifier.add,
                    mul: modifier.mul,
                };
                // An attribute that is gone since the item was defined
                // is not modified.
                if let Ok(changes) = stats::add_modifier(cx, holder, modifier) {
                    out.extend(changes);
                }
            }
            out
        });
        stat_changes(sim, changes.unwrap_or_default());
    }
    if def.holder_tags.is_empty() {
        return;
    }
    let mut fresh = Vec::new();
    if let Some(entity) = sim.entities.get_mut(holder) {
        for tag in &def.holder_tags {
            if entity.tags.insert(tag.clone()) {
                fresh.push(tag.clone());
            }
        }
    }
    if !fresh.is_empty() {
        let tags = sim
            .components
            .holds
            .get_or_insert_with(holder, Holds::default)
            .conferred_tags_mut();
        for tag in fresh {
            if let Err(at) = tags.binary_search(&tag) {
                tags.insert(at, tag);
            }
        }
    }
}

/// Takes back from a holder the tags it has from its items that
/// nothing it holds confers any more.
fn settle_tags(sim: &mut Sim, holder: EntityId) {
    let wanted = conferred(sim, holder).tags;
    let Some(holds) = sim.components.get_mut::<Holds>(holder) else {
        return;
    };
    let tags = holds.conferred_tags_mut();
    let gone: Vec<String> = tags
        .iter()
        .filter(|tag| !wanted.contains(tag))
        .cloned()
        .collect();
    if gone.is_empty() {
        return;
    }
    tags.retain(|tag| wanted.contains(tag));
    if let Some(entity) = sim.entities.get_mut(holder) {
        for tag in &gone {
            entity.tags.remove(tag);
        }
    }
}

/// Whether an entity's stats hold a modifier under a source.
fn has_source(sim: &Sim, holder: EntityId, source: Source) -> bool {
    sim.components
        .get::<Stats>(holder)
        .is_some_and(|stats| stats.modifiers().iter().any(|have| have.source == source))
}

/// Takes back what an item conferred on a holder it has just left the
/// use of. The holder's record of the item is already gone or changed.
pub(super) fn remove(sim: &mut Sim, holder: EntityId, item: EntityId) {
    let source = source_of(item);
    if has_source(sim, holder, source) {
        let changes = with_stats(sim, |cx| {
            stats::remove_source(cx, holder, source).unwrap_or_default()
        });
        stat_changes(sim, changes.unwrap_or_default());
    }
    settle_tags(sim, holder);
}

/// Works out again, from scratch, everything an entity has from the
/// things it holds: every modifier an item added is removed and what
/// [`conferred`] gives is applied. For when a holder is given stats
/// while it holds things, or definitions were reloaded.
pub fn reconfer(sim: &mut Sim, holder: EntityId) {
    let mut sources: Vec<Source> = sim
        .components
        .get::<Stats>(holder)
        .map(|stats| {
            stats
                .modifiers()
                .iter()
                .filter(|modifier| modifier.source.kind == SourceKind::Item)
                .map(|modifier| modifier.source)
                .collect()
        })
        .unwrap_or_default();
    sources.sort_unstable();
    sources.dedup();
    if !sources.is_empty() {
        let changes = with_stats(sim, |cx| {
            let mut out = Changes::new();
            for source in sources {
                out.extend(stats::remove_source(cx, holder, source).unwrap_or_default());
            }
            out
        });
        stat_changes(sim, changes.unwrap_or_default());
    }
    let in_use: Vec<EntityId> = sim
        .components
        .get::<Holds>(holder)
        .map(|holds| {
            holds
                .iter()
                .filter(|(_, hold)| hold.in_use())
                .map(|(entity, _)| entity)
                .collect()
        })
        .unwrap_or_default();
    for item in in_use {
        add(sim, holder, item);
    }
    settle_tags(sim, holder);
}
