//! The definition types of 06 §4: `attribute`, `pool` and `track`, and
//! [`StatDefs`], the tables worked out from them once mods have loaded.

use std::collections::BTreeSet;
use std::sync::Arc;

use clash_core::{LookupMap, Real, define_id};
use clash_data::expr::Number;
use clash_data::{
    CompileCx, CrossCx, Def, DefLabel, Defs, Diagnostic, Expr, FinishCx, Located, Scope, Text,
    ValidateCx, code,
};
use clash_expr::{DefTypeId, KindId, PropertyId, Registry};
use serde::Deserialize;

use super::StatsError;
use super::props::{self, Prop};

define_id! {
    /// An `attribute` definition (06 §4.1).
    pub struct AttributeId(u16);
    /// A `pool` definition (06 §4.2).
    pub struct PoolId(u16);
    /// A `track` definition (06 §4.3).
    pub struct TrackId(u16);
}

/// The most thresholds one pool may declare.
pub const MAX_THRESHOLDS: usize = 256;

/// The highest level a track can reach whatever its curve says, so that
/// a curve that never rises cannot keep an advance going for ever.
pub const TRACK_LEVEL_CAP: u32 = 10_000;

/// Which goes where when a mind and a body part company (06 §8.2).
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Seat {
    /// Stays with the body.
    #[default]
    Body,
    /// Leaves with the mind.
    Mind,
}

/// A literal zero, for an expression field that was left out.
fn zero() -> Expr<Number> {
    Expr::literal(0)
}

/// The scope of an expression about the owner of a stat: the owner as
/// `self`, pure.
fn owner_scope() -> Scope {
    Scope::new().own("self", props::ENTITY_KIND).pure()
}

/// The attributes an expression reads directly, by the per-attribute
/// properties of its bytecode, sorted.
fn attrs_read(registry: &Registry, expr: &Expr<Number>) -> Vec<AttributeId> {
    let Some(code) = expr.compiled() else {
        return Vec::new();
    };
    let mut read: Vec<AttributeId> = code
        .properties_read()
        .into_iter()
        .filter_map(|property| props::attr_of_property(registry, property))
        .collect();
    read.sort_unstable();
    read.dedup();
    read
}

/// An attribute: a number with a base and a stack of modifiers
/// (06 §4.1).
#[derive(Debug, Deserialize)]
pub struct AttributeDef {
    /// What a reader calls it.
    #[serde(default)]
    pub name: Option<Text>,
    /// Whether it stays with the body or leaves with the mind.
    #[serde(default)]
    pub seat: Seat,
    /// The value before modifiers: a literal, or an expression about the
    /// owner (`self`), which makes the attribute derived. Zero if left
    /// out.
    #[serde(default = "zero")]
    pub base: Expr<Number>,
    /// The least the value can be, after modifiers.
    #[serde(default)]
    pub min: Option<Real>,
    /// The most the value can be, after modifiers.
    #[serde(default)]
    pub max: Option<Real>,
    /// The attributes `base` reads directly. Worked out when the
    /// definition is compiled.
    #[serde(skip)]
    reads: Vec<AttributeId>,
}

impl AttributeDef {
    /// The base, if it is a literal: the same for every owner.
    pub fn literal_base(&self) -> Option<Real> {
        self.base.constant().and_then(clash_expr::Value::as_number)
    }

    /// Whether the base is an expression about the owner.
    pub fn is_derived(&self) -> bool {
        self.literal_base().is_none()
    }

    /// The attributes the base reads directly, in order of id: what the
    /// attribute depends on.
    pub fn reads(&self) -> &[AttributeId] {
        &self.reads
    }

    /// Keeps a value within `min` and `max`, where they are given.
    pub fn bound(&self, value: Real) -> Real {
        let value = self.min.map_or(value, |min| value.max(min));
        self.max.map_or(value, |max| value.min(max))
    }
}

impl Def for AttributeDef {
    type Id = AttributeId;
    const TYPE: &'static str = "attribute";
    const DOC: &'static str = "A number with a base and a stack of modifiers.";

    fn compile(&mut self, cx: &mut CompileCx<'_>) {
        cx.compile(&mut self.name, &Scope::new().pure());
        cx.compile(&mut self.base, &owner_scope());
        self.reads = attrs_read(cx.registry(), &self.base);
    }

    fn validate(&self, cx: &mut ValidateCx<'_>) {
        for (name, bound) in [("min", self.min), ("max", self.max)] {
            if bound.is_some_and(|bound| !bound.is_finite()) {
                cx.error(format!("`{name}` must be a finite number"));
            }
        }
        if let (Some(min), Some(max)) = (self.min, self.max)
            && min > max
        {
            cx.error(format!("`min` ({min}) is above `max` ({max})"));
        }
    }
}

/// A pool: a bounded quantity that fills, drains and regenerates
/// (06 §4.2).
#[derive(Debug, Deserialize)]
pub struct PoolDef {
    /// What a reader calls it.
    #[serde(default)]
    pub name: Option<Text>,
    /// Whether it stays with the body or leaves with the mind.
    #[serde(default)]
    pub seat: Seat,
    /// The most it holds: an expression about the owner (`self`).
    pub max: Expr<Number>,
    /// What it holds when its owner is given it. May name `max`. Zero if
    /// left out.
    #[serde(default = "zero")]
    pub start: Expr<Number>,
    /// What it gains in a second of its owner's time with nothing else
    /// acting on it; negative to decay. May name `max`. Zero if left out.
    #[serde(default = "zero")]
    pub regen: Expr<Number>,
    /// The fractions of `max` whose crossing is reported, lowest first.
    #[serde(default)]
    pub thresholds: Vec<Real>,
    /// What running out of it, or filling up with it, does to an owner
    /// that has a mind (06 §8.3): each entry names one of the
    /// `thresholds` and stops the owner while the level is on one side
    /// of it.
    #[serde(default)]
    pub stops: Vec<PoolStopDef>,
    /// The attributes `max` and `regen` read directly.
    #[serde(skip)]
    reads: Vec<AttributeId>,
}

/// Which side of a threshold a pool's level must be on for a
/// [`PoolStopDef`] to hold.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StopSide {
    /// Below the threshold, or fallen to it exactly.
    #[default]
    Below,
    /// Above the threshold, or risen to it exactly.
    Above,
}

/// One entry of a pool's `stops` (06 §8.3): while the pool's level is
/// on one side of one of its thresholds, an owner with a mind is
/// unconscious or dead.
#[derive(Clone, Copy, PartialEq, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PoolStopDef {
    /// The threshold: one of the pool's `thresholds`.
    pub at: Real,
    /// The side of it that stops the owner. Default `below`.
    #[serde(default)]
    pub when: StopSide,
    /// What it does: `unconscious` or `dead`.
    pub state: crate::mind::Stop,
}

impl PoolDef {
    /// The attributes `max` and `regen` read directly, in order of id:
    /// a change to one of them works both out again.
    pub fn reads(&self) -> &[AttributeId] {
        &self.reads
    }
}

impl Def for PoolDef {
    type Id = PoolId;
    const TYPE: &'static str = "pool";
    const DOC: &'static str = "A bounded quantity that fills, drains and regenerates.";

    fn finish(&mut self, _cx: &mut FinishCx<'_>) {
        // A mod may list its thresholds in any order.
        self.thresholds.sort_unstable();
    }

    fn compile(&mut self, cx: &mut CompileCx<'_>) {
        cx.compile(&mut self.name, &Scope::new().pure());
        let scope = owner_scope();
        cx.compile(&mut self.max, &scope);
        let with_max = scope.subject(props::MAX_NAME, "real");
        cx.compile(&mut self.start, &with_max);
        cx.compile(&mut self.regen, &with_max);
        let mut reads = attrs_read(cx.registry(), &self.max);
        reads.extend(attrs_read(cx.registry(), &self.regen));
        reads.sort_unstable();
        reads.dedup();
        self.reads = reads;
    }

    fn validate(&self, cx: &mut ValidateCx<'_>) {
        if self.thresholds.len() > MAX_THRESHOLDS {
            cx.error(format!(
                "a pool has at most {MAX_THRESHOLDS} thresholds, and this has {}",
                self.thresholds.len()
            ));
        }
        for threshold in &self.thresholds {
            if !(Real::ZERO..=Real::ONE).contains(threshold) {
                cx.error(format!(
                    "a threshold is a fraction of `max`, from 0 to 1, and {threshold} is not"
                ));
            }
        }
        for pair in self.thresholds.windows(2) {
            if pair[0] == pair[1] {
                cx.error(format!("the threshold {} is listed twice", pair[0]));
            }
        }
        for stop in &self.stops {
            if !self.thresholds.contains(&stop.at) {
                cx.error(format!(
                    "a stop's `at` must be one of the pool's `thresholds`, and {} is not: a pool is looked at only when it crosses one",
                    stop.at
                ));
            }
        }
    }
}

/// A track: an accumulating counter with levels (06 §4.3).
#[derive(Debug, Deserialize)]
pub struct TrackDef {
    /// What a reader calls it.
    #[serde(default)]
    pub name: Option<Text>,
    /// Whether it stays with the body or leaves with the mind.
    #[serde(default)]
    pub seat: Seat,
    /// The total the counter must reach for the track to be at level
    /// `$level`, asked for each level in turn from 1. An expression about
    /// the owner (`self`).
    pub levels: Expr<Number>,
    /// The highest level, if there is one. No track passes
    /// [`TRACK_LEVEL_CAP`].
    #[serde(default)]
    pub max_level: Option<u32>,
    /// Effects to run for each level reached, with the owner as `self`
    /// and the level as `$level`. Kept as written: the rule runtime
    /// compiles and runs them.
    #[serde(default)]
    pub on_advance: Option<Located<serde_json::Value>>,
}

impl TrackDef {
    /// The highest level the track can reach.
    pub fn level_limit(&self) -> u32 {
        self.max_level
            .map_or(TRACK_LEVEL_CAP, |max| max.min(TRACK_LEVEL_CAP))
    }
}

impl Def for TrackDef {
    type Id = TrackId;
    const TYPE: &'static str = "track";
    const DOC: &'static str = "An accumulating counter with levels.";

    fn compile(&mut self, cx: &mut CompileCx<'_>) {
        cx.compile(&mut self.name, &Scope::new().pure());
        let scope = owner_scope().local(props::LEVEL_NAME, "int");
        cx.compile(&mut self.levels, &scope);
    }

    fn validate(&self, cx: &mut ValidateCx<'_>) {
        if let Some(effects) = &self.on_advance
            && !effects.value.is_array()
        {
            cx.error_at(effects.span, "`on_advance` is a list of effects");
        }
    }
}

/// The path from `from` back to itself through what attributes read, if
/// there is one, as indices: `[from, …, from]`.
fn cycle_through(reads: &[Vec<usize>], from: usize) -> Option<Vec<usize>> {
    // Depth first, by an explicit stack of (node, next edge to try).
    let mut seen = vec![false; reads.len()];
    let mut path: Vec<(usize, usize)> = vec![(from, 0)];
    while let Some((node, edge)) = path.last().copied() {
        let Some(&next) = reads.get(node).and_then(|edges| edges.get(edge)) else {
            path.pop();
            continue;
        };
        if let Some(last) = path.last_mut() {
            last.1 += 1;
        }
        if next == from {
            let mut cycle: Vec<usize> = path.iter().map(|(node, _)| *node).collect();
            cycle.push(from);
            return Some(cycle);
        }
        if next < seen.len() && !seen[next] {
            seen[next] = true;
            path.push((next, 0));
        }
    }
    None
}

/// The whole-set check of the attributes: one derived from itself,
/// directly or through others, is an error at its `base` (06 §4.1).
pub(super) fn check_attribute_cycles(cx: &mut CrossCx<'_>) {
    let view = cx.defs();
    let attributes: Vec<(AttributeId, &AttributeDef)> = view.iter::<AttributeDef>().collect();
    let size = attributes
        .iter()
        .map(|(id, _)| id.index() + 1)
        .max()
        .unwrap_or(0);
    let mut reads: Vec<Vec<usize>> = vec![Vec::new(); size];
    for (id, def) in &attributes {
        reads[id.index()] = def.reads.iter().map(|read| read.index()).collect();
    }
    let name = |index: usize| {
        AttributeId::from_index(index)
            .and_then(|id| view.name::<AttributeDef>(id))
            .unwrap_or("?")
    };
    for (id, def) in &attributes {
        let Some(cycle) = cycle_through(&reads, id.index()) else {
            continue;
        };
        let chain: Vec<&str> = cycle.iter().map(|index| name(*index)).collect();
        let label = DefLabel::new(AttributeDef::TYPE, name(id.index()));
        let mut diagnostic = Diagnostic::new(
            code::VALIDATION,
            format!(
                "attribute `{}` is derived from itself: {}",
                name(id.index()),
                chain.join(" → ")
            ),
        )
        .in_def(Some(&label))
        .with_path("base")
        .suggest("a derived attribute may read others only if none of them reads it back");
        if let Some(span) = def.base.span() {
            diagnostic = diagnostic.at(cx.sources(), span);
        }
        cx.report(diagnostic);
    }
}

/// The kinds of subject this module registers, and the kind `entity`.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct StatKinds {
    /// `entity`: what has stats.
    pub entity: KindId,
    /// `attrs`: the attributes of one entity, reached as `entity.attr`.
    pub attrs: KindId,
    /// `pools`: the pools of one entity, reached as `entity.pool`.
    pub pools: KindId,
    /// `tracks`: the tracks of one entity, reached as `entity.track`.
    pub tracks: KindId,
}

/// The definition types of this module as the expression registry
/// numbers them.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct StatDefTypes {
    /// `attribute`.
    pub attribute: DefTypeId,
    /// `pool`.
    pub pool: DefTypeId,
    /// `track`.
    pub track: DefTypeId,
}

/// What the functions of [`stats`](super) need of a set of loaded
/// definitions: the three registries, the order in which derived
/// attributes are worked out, and which registered property is which.
///
/// Built once after loading with [`StatDefs::new`], and immutable.
#[derive(Debug)]
pub struct StatDefs {
    defs: Arc<Defs>,
    kinds: StatKinds,
    def_types: StatDefTypes,
    props: LookupMap<PropertyId, Prop>,
    /// Every attribute, each after the attributes it reads.
    order: Vec<AttributeId>,
}

impl StatDefs {
    /// The tables for a set of loaded definitions.
    ///
    /// Fails if the definitions were loaded without this crate's
    /// `register_defs`, which registers the three types and the
    /// properties that go with them.
    pub fn new(defs: &Arc<Defs>) -> Result<StatDefs, StatsError> {
        let missing = |what: &'static str| StatsError::NotRegistered { what };
        if defs.try_registry::<AttributeDef>().is_none() {
            return Err(missing("the definition type `attribute`"));
        }
        if defs.try_registry::<PoolDef>().is_none() {
            return Err(missing("the definition type `pool`"));
        }
        if defs.try_registry::<TrackDef>().is_none() {
            return Err(missing("the definition type `track`"));
        }
        let registry = defs.expr_registry();
        let kind = |name: &'static str| registry.kind_id(name).ok_or(missing(name));
        let kinds = StatKinds {
            entity: kind(props::ENTITY_KIND)?,
            attrs: kind(props::ATTRS_KIND)?,
            pools: kind(props::POOLS_KIND)?,
            tracks: kind(props::TRACKS_KIND)?,
        };
        let def_type = |name: &'static str| registry.def_type_id(name).ok_or(missing(name));
        let def_types = StatDefTypes {
            attribute: def_type(AttributeDef::TYPE)?,
            pool: def_type(PoolDef::TYPE)?,
            track: def_type(TrackDef::TYPE)?,
        };
        let props = props::table(registry, kinds);

        // Kahn's algorithm, taking the lowest id of those ready, so that
        // the order depends only on the definitions.
        let attributes = defs.registry::<AttributeDef>();
        let count = attributes.len();
        let mut waiting: Vec<usize> = vec![0; count];
        let mut readers: Vec<Vec<usize>> = vec![Vec::new(); count];
        for (id, def) in attributes.iter() {
            for read in def.reads() {
                if read.index() < count {
                    waiting[id.index()] += 1;
                    readers[read.index()].push(id.index());
                }
            }
        }
        let mut ready: BTreeSet<usize> = (0..count).filter(|index| waiting[*index] == 0).collect();
        let mut order = Vec::with_capacity(count);
        while let Some(index) = ready.pop_first() {
            if let Some(id) = AttributeId::from_index(index) {
                order.push(id);
            }
            for &reader in &readers[index] {
                waiting[reader] -= 1;
                if waiting[reader] == 0 {
                    ready.insert(reader);
                }
            }
        }
        if order.len() != count {
            // The loader refuses a cycle, so this is a set of definitions
            // that did not come through it.
            return Err(StatsError::NotRegistered {
                what: "an order for derived attributes: one is derived from itself",
            });
        }
        Ok(StatDefs {
            defs: Arc::clone(defs),
            kinds,
            def_types,
            props,
            order,
        })
    }

    /// The definitions these tables were made from.
    pub fn defs(&self) -> &Arc<Defs> {
        &self.defs
    }

    /// The kinds of subject the stats are read through.
    pub fn kinds(&self) -> StatKinds {
        self.kinds
    }

    /// The three definition types as the expression registry numbers
    /// them, for making an id value.
    pub fn def_types(&self) -> StatDefTypes {
        self.def_types
    }

    /// An attribute's definition, if the id is one of these definitions'.
    pub fn attribute(&self, id: AttributeId) -> Option<&AttributeDef> {
        self.defs.registry::<AttributeDef>().try_get(id)
    }

    /// A pool's definition, if the id is one of these definitions'.
    pub fn pool(&self, id: PoolId) -> Option<&PoolDef> {
        self.defs.registry::<PoolDef>().try_get(id)
    }

    /// A track's definition, if the id is one of these definitions'.
    pub fn track(&self, id: TrackId) -> Option<&TrackDef> {
        self.defs.registry::<TrackDef>().try_get(id)
    }

    /// The attribute with this qualified id.
    pub fn attribute_id(&self, qualified: &str) -> Option<AttributeId> {
        self.defs.lookup::<AttributeDef>(qualified)
    }

    /// The pool with this qualified id.
    pub fn pool_id(&self, qualified: &str) -> Option<PoolId> {
        self.defs.lookup::<PoolDef>(qualified)
    }

    /// The track with this qualified id.
    pub fn track_id(&self, qualified: &str) -> Option<TrackId> {
        self.defs.lookup::<TrackDef>(qualified)
    }

    /// Every attribute, each after the attributes its base reads: the
    /// order in which they are worked out.
    pub fn attribute_order(&self) -> &[AttributeId] {
        &self.order
    }

    /// How many attributes are defined.
    pub fn attribute_count(&self) -> usize {
        self.order.len()
    }

    /// What a registered property is, if it is one of this module's.
    pub(super) fn prop(&self, property: PropertyId) -> Option<Prop> {
        self.props.get(&property).copied()
    }

    /// Whether a registered property is one this module answers.
    pub fn answers(&self, property: PropertyId) -> bool {
        self.props.contains_key(&property)
    }
}
