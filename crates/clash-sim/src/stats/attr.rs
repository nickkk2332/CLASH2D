//! Attributes (06 §4.1): a base, a stack of modifiers, a cached value.
//!
//! # How a value is worked out
//!
//! ```text
//! value = bound((base + Σ add) × Π mul)
//! ```
//!
//! - `base` is the entity's own, if it was given one with [`set_base`];
//!   else the definition's, a literal or an expression about the owner.
//!   An expression that comes to null counts as zero.
//! - Every `add` is summed and then every `mul` multiplied in, in the
//!   order the modifiers are kept: by [`Source`], and for one source in
//!   the order they were added. The order is fixed because
//!   floating-point sums depend on it.
//! - `bound` keeps the result within the definition's `min` and `max`,
//!   where it gives them.
//!
//! # What depends on what
//!
//! A derived attribute depends on the attributes its `base` reads
//! directly, as `self.attr.<id>`, which is known when mods load. A
//! dependency that leads back to itself is a load error. When an
//! attribute's value changes, exactly those that read it are worked out
//! again, in an order in which each comes after what it reads, and so on
//! through those that change; so are the pools whose `max` or `regen`
//! read it. Nothing else is touched.
//!
//! An expression may read more than attributes: `self.mass`, a body
//! function, a variable. Nothing here can see those change, so whoever
//! changes them calls [`refresh`], which works every attribute and pool
//! of the entity out from scratch. An attribute of the owner read
//! through a function, which loading cannot see, is refused when it is
//! read (see [`props`](super::props)).
//!
//! The cached values are saved and hashed. A value is stored afresh
//! whenever it is worked out, so the cache is bit for bit what
//! [`refresh`] gives (06 invariant B3).

use clash_core::{EntityId, Real, StateHash, StateHasher, impl_state_hash};
use clash_expr::{Host, Value};
use clash_time::ClockId;
use serde::{Deserialize, Serialize};

use super::defs::AttributeId;
use super::props::Scoped;
use super::{AttrInst, Change, Changes, Stats, StatsCx, StatsError, pool, stats_mut, stats_of};

/// What kind of thing added a modifier or a rate.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug, Serialize, Deserialize)]
#[repr(u8)]
pub enum SourceKind {
    /// A trait instance (06 §5).
    Trait,
    /// An item held or worn (06 §9).
    Item,
    /// A wound (06 §7.3).
    Wound,
    /// An ability (08).
    Ability,
    /// A template applied at creation (06 §10).
    Template,
    /// A rule, through the verbs `add_modifier` and `pool_rate`.
    Rule,
    /// Anything else.
    Other,
}

impl StateHash for SourceKind {
    fn state_hash(&self, hasher: &mut StateHasher) {
        hasher.write_u8(*self as u8);
    }
}

/// Who added a modifier or a pool rate: a kind of thing and a number
/// that tells one of that kind from another on the same entity. What
/// the number is belongs to whoever adds: a trait instance's number, an
/// item's entity bits, a wound's index, a rule's key.
///
/// Removing a source ([`remove_source`](super::remove_source)) removes
/// exactly what was added under it.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug, Serialize, Deserialize)]
pub struct Source {
    /// What kind of thing.
    pub kind: SourceKind,
    /// Which one.
    pub id: u64,
}

impl_state_hash!(Source { kind, id });

impl Source {
    /// A source.
    pub const fn new(kind: SourceKind, id: u64) -> Source {
        Source { kind, id }
    }
}

/// A change to an attribute that lasts as long as its source does
/// (06 §4.1): an amount added, a factor multiplied in, or both.
#[derive(Clone, Copy, PartialEq, Debug, Serialize, Deserialize)]
pub struct Modifier {
    /// The attribute modified.
    pub attr: AttributeId,
    /// Who added it.
    pub source: Source,
    /// Added to the base. Zero for none.
    pub add: Real,
    /// Multiplied in after every addition. One for none.
    pub mul: Real,
}

impl_state_hash!(Modifier {
    attr,
    source,
    add,
    mul
});

impl Modifier {
    /// A modifier that adds an amount.
    pub const fn add(attr: AttributeId, source: Source, amount: Real) -> Modifier {
        Modifier {
            attr,
            source,
            add: amount,
            mul: Real::ONE,
        }
    }

    /// A modifier that multiplies by a factor.
    pub const fn mul(attr: AttributeId, source: Source, factor: Real) -> Modifier {
        Modifier {
            attr,
            source,
            add: Real::ZERO,
            mul: factor,
        }
    }
}

/// Whether two values differ in any bit. A value that only changed the
/// sign of its zero is stored and passed on, and not reported.
fn differs(a: Real, b: Real) -> bool {
    a.get().to_bits() != b.get().to_bits()
}

/// Works one attribute of an entity out from its base and modifiers and
/// stores it. Returns the value before and after.
fn work_out<H: Host + ?Sized>(
    cx: &mut StatsCx<'_, H>,
    entity: EntityId,
    attr: AttributeId,
) -> Result<(Real, Real), StatsError> {
    let def = cx
        .defs
        .attribute(attr)
        .ok_or(StatsError::UnknownStat { what: "attribute" })?;
    let stats = stats_of(cx.stats, entity)?;
    let base = match (stats.own_base(attr), def.literal_base()) {
        (Some(own), _) => own,
        (None, Some(literal)) => literal,
        (None, None) => {
            let mut host = Scoped {
                stats: &*cx.stats,
                defs: cx.defs,
                timeline: &*cx.timeline,
                inner: &mut *cx.host,
                owner: entity,
                second: Value::Null,
                local: Value::Null,
                allowed: Some(def.reads()),
            };
            let mut eval = cx.runner.over(cx.defs.defs().exprs(), &mut host);
            def.base.eval(&mut eval).unwrap_or(Real::ZERO)
        }
    };
    let modifiers = stats.modifiers_of(attr);
    let mut value = base;
    for modifier in modifiers {
        value += modifier.add;
    }
    for modifier in modifiers {
        value *= modifier.mul;
    }
    let value = def.bound(value);

    let stats = stats_mut(cx.stats, entity)?;
    let at = match stats.attr_at(attr) {
        Ok(at) => at,
        Err(at) => {
            stats.attrs.insert(
                at,
                AttrInst {
                    attr,
                    base: None,
                    value,
                },
            );
            return Ok((value, value));
        }
    };
    let old = core::mem::replace(&mut stats.attrs[at].value, value);
    Ok((old, value))
}

/// Works out the attributes marked `stale` and everything that reads
/// one that changed, each after what it reads; then the pools that read
/// one that changed, or every pool if `all_pools`.
fn settle<H: Host + ?Sized>(
    cx: &mut StatsCx<'_, H>,
    entity: EntityId,
    mut stale: Vec<bool>,
    all_pools: bool,
    out: &mut Changes,
) -> Result<(), StatsError> {
    let defs = cx.defs;
    let mut changed = vec![false; stale.len()];
    for &attr in defs.attribute_order() {
        let index = attr.index();
        let Some(def) = defs.attribute(attr) else {
            continue;
        };
        if !stale[index] {
            // Reads something that changed, and has no base of its own
            // standing in for the expression that reads it.
            let reads_changed = def.reads().iter().any(|read| changed[read.index()]);
            if !reads_changed || stats_of(cx.stats, entity)?.own_base(attr).is_some() {
                continue;
            }
            stale[index] = true;
        }
        let (old, new) = work_out(cx, entity, attr)?;
        if differs(old, new) {
            changed[index] = true;
            if old != new {
                out.push(Change::AttrChanged {
                    entity,
                    attr,
                    old,
                    new,
                });
            }
        }
    }
    let pools: Vec<_> = stats_of(cx.stats, entity)?.pools().collect();
    for id in pools {
        // A pool whose definition is gone stays as it is.
        let Some(def) = defs.pool(id) else {
            continue;
        };
        if all_pools || def.reads().iter().any(|read| changed[read.index()]) {
            pool::rework(cx, entity, id, out)?;
        }
    }
    Ok(())
}

/// Works out the attributes named, and what follows from them.
fn settle_some<H: Host + ?Sized>(
    cx: &mut StatsCx<'_, H>,
    entity: EntityId,
    attrs: &[AttributeId],
    out: &mut Changes,
) -> Result<(), StatsError> {
    let mut stale = vec![false; cx.defs.attribute_count()];
    for attr in attrs {
        if let Some(slot) = stale.get_mut(attr.index()) {
            *slot = true;
        }
    }
    settle(cx, entity, stale, false, out)
}

/// Gives an entity stats: every attribute at its base, no pools, no
/// tracks. `clock` is the entity's clock, which its pools will run on.
///
/// Refused if the entity already has stats.
pub fn attach<H: Host + ?Sized>(
    cx: &mut StatsCx<'_, H>,
    entity: EntityId,
    clock: ClockId,
) -> Result<(), StatsError> {
    if cx.stats.contains(entity) {
        return Err(StatsError::HasStats { entity });
    }
    cx.timeline.local_now(clock)?;
    cx.stats.insert(entity, Stats::new(clock));
    let stale = vec![true; cx.defs.attribute_count()];
    // Nothing is reported: there was no value before.
    settle(cx, entity, stale, false, &mut Changes::new())
}

/// Works every attribute and every pool of an entity out from scratch:
/// what to call when something a stat's expression may read has changed
/// that is not an attribute (06 §6.5), and what the cached values are
/// measured against (invariant B3).
///
/// An attribute defined since the stats were made is given its value.
pub fn refresh<H: Host + ?Sized>(
    cx: &mut StatsCx<'_, H>,
    entity: EntityId,
) -> Result<Changes, StatsError> {
    stats_of(cx.stats, entity)?;
    let mut out = Changes::new();
    let stale = vec![true; cx.defs.attribute_count()];
    settle(cx, entity, stale, true, &mut out)?;
    Ok(out)
}

/// Gives an entity a base of its own for an attribute, in place of the
/// definition's, or with `None` gives the definition's back. Modifiers
/// still apply to it.
pub fn set_base<H: Host + ?Sized>(
    cx: &mut StatsCx<'_, H>,
    entity: EntityId,
    attr: AttributeId,
    base: Option<Real>,
) -> Result<Changes, StatsError> {
    if cx.defs.attribute(attr).is_none() {
        return Err(StatsError::UnknownStat { what: "attribute" });
    }
    let stats = stats_mut(cx.stats, entity)?;
    match stats.attr_at(attr) {
        Ok(at) => stats.attrs[at].base = base,
        Err(at) => stats.attrs.insert(
            at,
            AttrInst {
                attr,
                base,
                value: Real::ZERO,
            },
        ),
    }
    let mut out = Changes::new();
    settle_some(cx, entity, &[attr], &mut out)?;
    Ok(out)
}

/// Adds a modifier to an attribute of an entity. A source may add
/// several to one attribute; all apply.
pub fn add_modifier<H: Host + ?Sized>(
    cx: &mut StatsCx<'_, H>,
    entity: EntityId,
    modifier: Modifier,
) -> Result<Changes, StatsError> {
    if cx.defs.attribute(modifier.attr).is_none() {
        return Err(StatsError::UnknownStat { what: "attribute" });
    }
    let stats = stats_mut(cx.stats, entity)?;
    let key = (modifier.attr, modifier.source);
    let at = stats
        .modifiers
        .partition_point(|other| (other.attr, other.source) <= key);
    stats.modifiers.insert(at, modifier);
    let mut out = Changes::new();
    settle_some(cx, entity, &[modifier.attr], &mut out)?;
    Ok(out)
}

/// Removes the modifiers a source added, to one attribute or to all,
/// and brings what they touched up to date.
pub(super) fn drop_modifiers<H: Host + ?Sized>(
    cx: &mut StatsCx<'_, H>,
    entity: EntityId,
    source: Source,
    only: Option<AttributeId>,
    out: &mut Changes,
) -> Result<(), StatsError> {
    let stats = stats_mut(cx.stats, entity)?;
    let mut touched: Vec<AttributeId> = Vec::new();
    stats.modifiers.retain(|modifier| {
        let goes = modifier.source == source && only.is_none_or(|attr| attr == modifier.attr);
        if goes && touched.last() != Some(&modifier.attr) {
            touched.push(modifier.attr);
        }
        !goes
    });
    if touched.is_empty() {
        return Ok(());
    }
    settle_some(cx, entity, &touched, out)
}

/// Removes the modifiers one source added to one attribute.
pub fn remove_modifier<H: Host + ?Sized>(
    cx: &mut StatsCx<'_, H>,
    entity: EntityId,
    source: Source,
    attr: AttributeId,
) -> Result<Changes, StatsError> {
    let mut out = Changes::new();
    drop_modifiers(cx, entity, source, Some(attr), &mut out)?;
    Ok(out)
}

/// Removes every modifier one source added, to any attribute, and leaves
/// the rates it applied to pools: the verb `remove_modifier` with no
/// attribute named (06 §4.4).
pub fn remove_modifiers<H: Host + ?Sized>(
    cx: &mut StatsCx<'_, H>,
    entity: EntityId,
    source: Source,
) -> Result<Changes, StatsError> {
    let mut out = Changes::new();
    drop_modifiers(cx, entity, source, None, &mut out)?;
    Ok(out)
}
