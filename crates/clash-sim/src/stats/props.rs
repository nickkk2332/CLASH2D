//! What the expression language knows of stats: the kinds, properties
//! and events this module registers (05 §5.3; 06 §13), the function that
//! answers the properties from the stores, and the host that stands in
//! front of the simulation's while a stat's own expression is evaluated.
//!
//! # How a stat is written
//!
//! | Written | Type | Is |
//! |---------|------|----|
//! | `self.attr.strength` | `real` | An attribute's value, modifiers applied |
//! | `self.attr.value(id)` | `real` | The same, for an attribute chosen at run time |
//! | `self.pool.blood` | `real` | A pool's level now |
//! | `self.pool.level(id)` | `real` | The same, for a pool chosen at run time |
//! | `self.pool.frac('blood')` | `real` | The level as a fraction of the most the pool holds |
//! | `self.pool.max('blood')` | `real` | The most the pool holds |
//! | `self.pool.rate('blood')` | `real` | What the pool gains in a second, every rate summed |
//! | `self.track.swordsmanship` | `int` | A track's level |
//! | `self.track.level(id)` | `int` | The same, for a track chosen at run time |
//! | `self.track.amount('swordsmanship')` | `real` | What the track has accumulated |
//!
//! `attr`, `pool` and `track` are properties of an entity that give a
//! subject of the kind `attrs`, `pools` or `tracks`: the same entity,
//! seen as its stats. Each definition is a property of that kind, under
//! its qualified id (`self.attr.core:strength`) and, where no other
//! definition of the type and no property of the kind has the same bare
//! name, under its bare id too. A misspelt name is therefore a load
//! error with a suggestion.
//!
//! None of these is safe for belief: a character may read its own stats
//! and nobody else's. None is settable: stats change through the verbs,
//! so that hooks run and crossings are reported. Anything read of an
//! entity that has no stats, a pool it does not have, or an id that names
//! nothing is null.

use clash_core::{EntityId, Real, Rng};
use clash_data::RegistryCx;
use clash_expr::{
    DefRef, DefTypeId, EventSpec, Fault, FunctionId, Handle, Host, KindSpec, PropertyId,
    PropertySpec, Registry, RegistryError, RuntimeWarning, Span, Type, Value,
};
use clash_time::Timeline;

use super::defs::{
    AttributeDef, AttributeId, PoolDef, PoolId, StatDefs, StatKinds, TrackDef, TrackId,
};
use super::{Stats, StatsStore};
use clash_core::LookupMap;
use clash_data::{Def, Diagnostic, code};

/// The kind of subject that has stats.
pub const ENTITY_KIND: &str = "entity";
/// The kind `entity.attr` gives: an entity's attributes.
pub const ATTRS_KIND: &str = "attrs";
/// The kind `entity.pool` gives: an entity's pools.
pub const POOLS_KIND: &str = "pools";
/// The kind `entity.track` gives: an entity's tracks.
pub const TRACKS_KIND: &str = "tracks";

/// The property of an entity that gives its attributes.
pub const ATTR_PROPERTY: &str = "attr";
/// The property of an entity that gives its pools.
pub const POOL_PROPERTY: &str = "pool";
/// The property of an entity that gives its tracks.
pub const TRACK_PROPERTY: &str = "track";

/// `attrs.value(id)`: an attribute by id.
pub const VALUE_PROPERTY: &str = "value";
/// `pools.level(id)` and `tracks.level(id)`: a level by id.
pub const LEVEL_PROPERTY: &str = "level";
/// `pools.frac(id)`: a pool's level as a fraction of its `max`.
pub const FRAC_PROPERTY: &str = "frac";
/// `pools.max(id)`: the most a pool holds.
pub const MAX_PROPERTY: &str = "max";
/// `pools.rate(id)`: a pool's net rate.
pub const RATE_PROPERTY: &str = "rate";
/// `tracks.amount(id)`: what a track has accumulated.
pub const AMOUNT_PROPERTY: &str = "amount";

/// The name a pool's `start` and `regen` read its `max` by.
pub const MAX_NAME: &str = "max";
/// The local a track's `levels` reads the level asked about by.
pub const LEVEL_NAME: &str = "level";

/// The event raised when an attribute's value changes.
pub const ATTR_CHANGED: &str = "attr_changed";
/// The event raised when a pool's level crosses a threshold.
pub const POOL_CROSSED: &str = "pool_crossed";
/// The hook that runs before a verb changes a pool's level.
pub const POOL_CHANGING: &str = "pool_changing";
/// The event raised for each level a track reaches.
pub const TRACK_ADVANCED: &str = "track_advanced";

/// One of this module's registered properties.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum Prop {
    /// `entity.attr`.
    Attrs,
    /// `entity.pool`.
    Pools,
    /// `entity.track`.
    Tracks,
    /// `attrs.<attribute>`.
    Attr(AttributeId),
    /// `attrs.value(id)`.
    AttrValue,
    /// `pools.<pool>`.
    Pool(PoolId),
    /// `pools.level(id)`.
    PoolLevel,
    /// `pools.frac(id)`.
    PoolFrac,
    /// `pools.max(id)`.
    PoolMax,
    /// `pools.rate(id)`.
    PoolRate,
    /// `tracks.<track>`.
    Track(TrackId),
    /// `tracks.level(id)`.
    TrackLevel,
    /// `tracks.amount(id)`.
    TrackAmount,
}

/// The part of a qualified id after its namespace.
fn bare(qualified: &str) -> &str {
    qualified
        .rsplit_once(':')
        .map_or(qualified, |(_, bare)| bare)
}

/// The dense number of the definition a per-definition property name
/// stands for: a qualified id exactly, or a bare one if exactly one
/// definition of the type has it.
fn def_by_property_name(registry: &Registry, ty: DefTypeId, name: &str) -> Option<u32> {
    if name.contains(':') {
        return registry.def_ref(ty, name).map(|found| found.index);
    }
    let mut matches = registry
        .def_type(ty)?
        .ids()
        .filter(|(qualified, _)| bare(qualified) == name);
    let (_, index) = matches.next()?;
    matches.next().is_none().then_some(index)
}

/// The attribute a registered property reads, if it is the property of
/// one attribute.
pub(super) fn attr_of_property(registry: &Registry, property: PropertyId) -> Option<AttributeId> {
    let spec = &registry.property(property)?.spec;
    if Some(spec.kind) != registry.kind_id(ATTRS_KIND) || !spec.params.is_empty() {
        return None;
    }
    let ty = registry.def_type_id(AttributeDef::TYPE)?;
    let index = def_by_property_name(registry, ty, &spec.name)?;
    AttributeId::from_index(index as usize)
}

/// Which registered property is which, for [`StatDefs`].
pub(super) fn table(registry: &Registry, kinds: StatKinds) -> LookupMap<PropertyId, Prop> {
    let mut table = LookupMap::new();
    let fixed = [
        (kinds.entity, ATTR_PROPERTY, Prop::Attrs),
        (kinds.entity, POOL_PROPERTY, Prop::Pools),
        (kinds.entity, TRACK_PROPERTY, Prop::Tracks),
        (kinds.attrs, VALUE_PROPERTY, Prop::AttrValue),
        (kinds.pools, LEVEL_PROPERTY, Prop::PoolLevel),
        (kinds.pools, FRAC_PROPERTY, Prop::PoolFrac),
        (kinds.pools, MAX_PROPERTY, Prop::PoolMax),
        (kinds.pools, RATE_PROPERTY, Prop::PoolRate),
        (kinds.tracks, LEVEL_PROPERTY, Prop::TrackLevel),
        (kinds.tracks, AMOUNT_PROPERTY, Prop::TrackAmount),
    ];
    for (kind, name, prop) in fixed {
        if let Some(id) = registry.property_id(kind, name) {
            table.insert(id, prop);
        }
    }
    let per_def = [
        (kinds.attrs, AttributeDef::TYPE),
        (kinds.pools, PoolDef::TYPE),
        (kinds.tracks, TrackDef::TYPE),
    ];
    for (kind, type_name) in per_def {
        let Some(ty) = registry.def_type_id(type_name) else {
            continue;
        };
        for property in registry.properties_of(kind) {
            if table.contains_key(&property.id) || !property.spec.params.is_empty() {
                continue;
            }
            let Some(index) = def_by_property_name(registry, ty, &property.spec.name) else {
                continue;
            };
            let index = index as usize;
            let prop = if kind == kinds.attrs {
                AttributeId::from_index(index).map(Prop::Attr)
            } else if kind == kinds.pools {
                PoolId::from_index(index).map(Prop::Pool)
            } else {
                TrackId::from_index(index).map(Prop::Track)
            };
            if let Some(prop) = prop {
                table.insert(property.id, prop);
            }
        }
    }
    table
}

/// The kind with this name, registered if it is not there.
fn kind(
    registry: &mut Registry,
    name: &str,
    doc: &str,
) -> Result<clash_expr::KindId, RegistryError> {
    match registry.kind_id(name) {
        Some(kind) => Ok(kind),
        None => registry.add_kind(KindSpec::new(name).doc(doc)),
    }
}

/// The definition type with this name, registered if it is not there.
fn def_type(registry: &mut Registry, name: &str, doc: &str) -> Result<DefTypeId, RegistryError> {
    match registry.def_type_id(name) {
        Some(ty) => Ok(ty),
        None => registry.add_def_type(name, doc),
    }
}

/// Registers one property for each definition of a type: under its
/// qualified id, and under its bare id where nothing else has that name.
fn per_definition(
    registry: &mut Registry,
    kind: clash_expr::KindId,
    ty: DefTypeId,
    make: impl Fn(&str, &str) -> PropertySpec,
) -> Result<(), RegistryError> {
    let ids: Vec<String> = registry
        .def_type(ty)
        .map(|def_type| def_type.ids().map(|(name, _)| name.to_owned()).collect())
        .unwrap_or_default();
    for qualified in &ids {
        registry.add_property(make(qualified, qualified))?;
    }
    for qualified in &ids {
        let short = bare(qualified);
        let unique = ids.iter().filter(|other| bare(other) == short).count() == 1;
        if unique && registry.property_id(kind, short).is_none() {
            // A bare id that is a reserved word cannot be written after a
            // dot; the qualified one can.
            let _ = registry.add_property(make(short, qualified));
        }
    }
    Ok(())
}

/// Registers everything the expression language knows of stats: the
/// kinds `attrs`, `pools` and `tracks`, the properties of the
/// [module](self) table, and the events `attr_changed`, `pool_crossed`,
/// `pool_changing` and `track_advanced`.
///
/// The registry must already hold the ids of the `attribute`, `pool` and
/// `track` definitions (the loader puts them there before any registry
/// step runs). The kind `entity` is the kernel's; it is registered here
/// only if nobody has, so that a tool without a kernel can still load
/// stats.
pub fn register(registry: &mut Registry) -> Result<(), RegistryError> {
    let entity = kind(
        registry,
        ENTITY_KIND,
        "Any entity: a creature, an item, a device.",
    )?;
    let attrs = registry.add_kind(
        KindSpec::new(ATTRS_KIND)
            .doc("The attributes of one entity, reached as `entity.attr` (06 §4.1)."),
    )?;
    let pools = registry.add_kind(
        KindSpec::new(POOLS_KIND)
            .doc("The pools of one entity, reached as `entity.pool` (06 §4.2)."),
    )?;
    let tracks = registry.add_kind(
        KindSpec::new(TRACKS_KIND)
            .doc("The tracks of one entity, reached as `entity.track` (06 §4.3)."),
    )?;
    let attribute = def_type(registry, AttributeDef::TYPE, AttributeDef::DOC)?;
    let pool = def_type(registry, PoolDef::TYPE, PoolDef::DOC)?;
    let track = def_type(registry, TrackDef::TYPE, TrackDef::DOC)?;

    // The events first: the properties name them.
    registry.add_event(
        EventSpec::new(ATTR_CHANGED)
            .subject("self", entity)
            .field(
                "attr",
                Type::Id(attribute),
                "The attribute whose value changed.",
            )
            .field("old", Type::Real, "Its value before.")
            .field("new", Type::Real, "Its value now.")
            .doc(
                "An attribute's value changed: its base, a modifier, or something its base reads.",
            ),
    )?;
    registry.add_event(
        EventSpec::new(POOL_CROSSED)
            .subject("self", entity)
            .field("pool", Type::Id(pool), "The pool.")
            .field("threshold", Type::Real, "The threshold crossed, as the fraction of `max` the definition lists.")
            .field("rising", Type::Bool, "True if the level rose to the threshold or past it; false if it fell to it or below.")
            .field("level", Type::Real, "The pool's level after the crossing.")
            .doc("A pool's level crossed one of its thresholds, by a rate, a verb or a change of its `max`. Raised once for each threshold crossed, in the order they were crossed."),
    )?;
    registry.add_event(
        EventSpec::new(POOL_CHANGING)
            .subject("self", entity)
            .field("pool", Type::Id(pool), "The pool.")
            .field("amount", Type::Real, "What is about to be added; negative to take away.")
            .hook(["amount"])
            .doc("Runs before a verb changes a pool's level. A handler may change the amount or cancel. Rates and regeneration do not raise it."),
    )?;
    registry.add_event(
        EventSpec::new(TRACK_ADVANCED)
            .subject("self", entity)
            .field("track", Type::Id(track), "The track.")
            .field("level", Type::Int, "The level reached.")
            .doc("A track reached a level. Raised once for each level, lowest first, when one advance passes several."),
    )?;

    let accessor = |name: &str, kind, what: &str| {
        PropertySpec::new(entity, name, Type::Subject(kind))
            .doc(format!(
                "The entity's {what}: follow with a definition's id, as in `self.{name}.<id>`."
            ))
            .yields_own(true)
    };
    registry.add_property(accessor(ATTR_PROPERTY, attrs, "attributes"))?;
    registry.add_property(accessor(POOL_PROPERTY, pools, "pools"))?;
    registry.add_property(accessor(TRACK_PROPERTY, tracks, "tracks"))?;

    registry.add_property(
        PropertySpec::new(attrs, VALUE_PROPERTY, Type::Real)
            .param("attribute", Type::Id(attribute), "Which attribute.")
            .doc("The value of an attribute chosen by id, modifiers applied. Null for something with no stats.")
            .changed_by([ATTR_CHANGED]),
    )?;
    let of_pool = |name: &str, doc: &str| {
        PropertySpec::new(pools, name, Type::Real)
            .param("pool", Type::Id(pool), "Which pool.")
            .doc(format!(
                "{doc} Null for something that does not have the pool."
            ))
    };
    registry.add_property(of_pool(LEVEL_PROPERTY, "A pool's level now."))?;
    registry.add_property(of_pool(
        FRAC_PROPERTY,
        "A pool's level as a fraction of the most it holds, from 0 to 1; 0 for a pool that holds nothing.",
    ))?;
    registry.add_property(of_pool(MAX_PROPERTY, "The most a pool holds."))?;
    registry.add_property(of_pool(
        RATE_PROPERTY,
        "What a pool gains in a second of its owner's time: its regeneration and every rate applied, summed. Negative while it drains.",
    ))?;
    registry.add_property(
        PropertySpec::new(tracks, LEVEL_PROPERTY, Type::Int)
            .param("track", Type::Id(track), "Which track.")
            .doc("A track's level, 0 for one never advanced. Null for something with no stats.")
            .changed_by([TRACK_ADVANCED]),
    )?;
    registry.add_property(
        PropertySpec::new(tracks, AMOUNT_PROPERTY, Type::Real)
            .param("track", Type::Id(track), "Which track.")
            .doc("What a track has accumulated, 0 for one never advanced. Null for something with no stats."),
    )?;

    per_definition(registry, attrs, attribute, |name, qualified| {
        PropertySpec::new(attrs, name, Type::Real)
            .doc(format!(
                "The value of the attribute `{qualified}`, modifiers applied. Null for something with no stats."
            ))
            .changed_by([ATTR_CHANGED])
    })?;
    per_definition(registry, pools, pool, |name, qualified| {
        PropertySpec::new(pools, name, Type::Real).doc(format!(
            "The level of the pool `{qualified}` now. Null for something that does not have it."
        ))
    })?;
    per_definition(registry, tracks, track, |name, qualified| {
        PropertySpec::new(tracks, name, Type::Int)
            .doc(format!(
                "The level of the track `{qualified}`, 0 if never advanced. Null for something with no stats."
            ))
            .changed_by([TRACK_ADVANCED])
    })?;
    Ok(())
}

/// The registry step of this module (04 §7.3): [`register`], run by the
/// loader once the ids of the mods being loaded are known and before any
/// expression is compiled.
pub fn registry_step(cx: &mut RegistryCx<'_>) {
    if let Err(error) = register(cx.registry()) {
        cx.report(Diagnostic::new(
            code::VALIDATION,
            format!("the stats could not be registered with the expression language: {error}"),
        ));
    }
}

/// The definition an id argument names, as a dense number: an id value,
/// or a text holding a qualified id, which is what an argument typed
/// `any` at load can turn out to be.
fn id_arg(defs: &StatDefs, ty: DefTypeId, args: &[Value]) -> Option<usize> {
    match args.first()? {
        Value::Id(id) if id.ty == ty => Some(id.index as usize),
        Value::Str(text) => defs
            .defs()
            .expr_registry()
            .def_ref(ty, text)
            .map(|found: DefRef| found.index as usize),
        _ => None,
    }
}

/// The entity a handle of one of this module's kinds, or of the kind
/// `entity`, stands for.
fn entity_of(subject: Handle) -> EntityId {
    EntityId::from_bits(subject.payload)
}

fn real(value: Option<Real>) -> Value {
    value.map_or(Value::Null, Value::Real)
}

fn int(value: Option<u32>) -> Value {
    value.map_or(Value::Null, |value| Value::Int(i64::from(value)))
}

/// Answers a property this module knows, given which it is.
fn answer_prop(
    stats: &StatsStore,
    defs: &StatDefs,
    timeline: &Timeline,
    prop: Prop,
    subject: Handle,
    args: &[Value],
) -> Value {
    let kinds = defs.kinds();
    let types = defs.def_types();
    let wanted = match prop {
        Prop::Attrs | Prop::Pools | Prop::Tracks => kinds.entity,
        Prop::Attr(_) | Prop::AttrValue => kinds.attrs,
        Prop::Pool(_) | Prop::PoolLevel | Prop::PoolFrac | Prop::PoolMax | Prop::PoolRate => {
            kinds.pools
        }
        Prop::Track(_) | Prop::TrackLevel | Prop::TrackAmount => kinds.tracks,
    };
    if subject.kind != wanted {
        return Value::Null;
    }
    let accessor = |kind| Value::Subject(Handle::new(kind, subject.payload));
    let entity = entity_of(subject);
    let own: Option<&Stats> = stats.get(entity);
    let attr = |args| id_arg(defs, types.attribute, args).and_then(AttributeId::from_index);
    let pool = |args| id_arg(defs, types.pool, args).and_then(PoolId::from_index);
    let track = |args| id_arg(defs, types.track, args).and_then(TrackId::from_index);
    match prop {
        Prop::Attrs => accessor(kinds.attrs),
        Prop::Pools => accessor(kinds.pools),
        Prop::Tracks => accessor(kinds.tracks),
        Prop::Attr(id) => real(own.and_then(|stats| stats.attr(id))),
        Prop::AttrValue => real(own.zip(attr(args)).and_then(|(stats, id)| stats.attr(id))),
        Prop::Pool(id) => real(own.and_then(|stats| stats.pool_level(timeline, id))),
        Prop::PoolLevel => real(
            own.zip(pool(args))
                .and_then(|(stats, id)| stats.pool_level(timeline, id)),
        ),
        Prop::PoolFrac => real(
            own.zip(pool(args))
                .and_then(|(stats, id)| stats.pool_frac(timeline, id)),
        ),
        Prop::PoolMax => real(
            own.zip(pool(args))
                .and_then(|(stats, id)| stats.pool_max(id)),
        ),
        Prop::PoolRate => real(
            own.zip(pool(args))
                .and_then(|(stats, id)| stats.pool_rate(id)),
        ),
        Prop::Track(id) => int(own.map(|stats| stats.track_level(id))),
        Prop::TrackLevel => int(own
            .zip(track(args))
            .map(|(stats, id)| stats.track_level(id))),
        Prop::TrackAmount => real(
            own.zip(track(args))
                .map(|(stats, id)| stats.track_amount(id)),
        ),
    }
}

/// Answers a property of this module from the stores: what the
/// simulation's [`Host::property`] forwards.
///
/// `None` means the property is not one of this module's, and the caller
/// should look elsewhere. `Some(Value::Null)` means it is, and there is
/// nothing to read: the entity has no stats, lacks the pool, or the id
/// names nothing.
///
/// A pool's level is its level at the timeline's present moment, which
/// is why the timeline is needed.
pub fn property(
    stats: &StatsStore,
    defs: &StatDefs,
    timeline: &Timeline,
    property: PropertyId,
    subject: Handle,
    args: &[Value],
) -> Option<Value> {
    let prop = defs.prop(property)?;
    Some(answer_prop(stats, defs, timeline, prop, subject, args))
}

/// The host a stat's own expression is evaluated against: it binds the
/// owner as `self`, answers this module's properties from the store
/// being changed, and passes everything else to the simulation's host.
///
/// While a derived attribute, or a pool's `max` or `regen`, is worked
/// out, the owner's attributes may be read only if the definition reads
/// them directly, which is what makes it depend on them. A read of
/// another, which can only be reached through a function, is null with a
/// warning: it would otherwise go stale unnoticed.
pub(super) struct Scoped<'a, H: Host + ?Sized> {
    pub(super) stats: &'a StatsStore,
    pub(super) defs: &'a StatDefs,
    pub(super) timeline: &'a Timeline,
    pub(super) inner: &'a mut H,
    /// The owner.
    pub(super) owner: EntityId,
    /// The value of the second bound name (`max`), if the field has one.
    pub(super) second: Value,
    /// The value of the first local (`$level`), if the field has one.
    pub(super) local: Value,
    /// The owner's attributes the expression may read; `None` for any.
    pub(super) allowed: Option<&'a [AttributeId]>,
}

impl<H: Host + ?Sized> Scoped<'_, H> {
    /// The attribute of the owner a read is of, if it is one the
    /// expression does not depend on.
    fn hidden(&self, prop: Prop, subject: Handle, args: &[Value]) -> bool {
        let Some(allowed) = self.allowed else {
            return false;
        };
        if subject.kind != self.defs.kinds().attrs || entity_of(subject) != self.owner {
            return false;
        }
        let attr = match prop {
            Prop::Attr(attr) => Some(attr),
            Prop::AttrValue => id_arg(self.defs, self.defs.def_types().attribute, args)
                .and_then(AttributeId::from_index),
            _ => None,
        };
        attr.is_some_and(|attr| !allowed.contains(&attr))
    }
}

impl<H: Host + ?Sized> Host for Scoped<'_, H> {
    fn subject(&mut self, slot: u16) -> Value {
        match slot {
            0 => Value::Subject(Handle::new(self.defs.kinds().entity, self.owner.to_bits())),
            1 => self.second.clone(),
            _ => Value::Null,
        }
    }

    fn local(&mut self, slot: u16) -> Value {
        match slot {
            0 => self.local.clone(),
            _ => Value::Null,
        }
    }

    fn event_field(&mut self, _slot: u16) -> Value {
        Value::Null
    }

    fn property(&mut self, property: PropertyId, subject: Handle, args: &[Value]) -> Value {
        let Some(prop) = self.defs.prop(property) else {
            return self.inner.property(property, subject, args);
        };
        if self.hidden(prop, subject, args) {
            self.inner.warn(RuntimeWarning {
                fault: Fault::Unavailable(
                    "attributes read inside a function called from a stat's own expression",
                ),
                span: Span::default(),
                origin: 0,
            });
            return Value::Null;
        }
        answer_prop(self.stats, self.defs, self.timeline, prop, subject, args)
    }

    fn variable(&mut self, subject: Handle, name: &str) -> Value {
        self.inner.variable(subject, name)
    }

    fn call(&mut self, function: FunctionId, args: &[Value]) -> Value {
        self.inner.call(function, args)
    }

    fn rng(&mut self) -> Option<&mut Rng> {
        // A stat's expressions are pure.
        None
    }

    fn warn(&mut self, warning: RuntimeWarning) {
        self.inner.warn(warning);
    }

    fn def_name(&self, id: DefRef) -> Option<&str> {
        self.inner.def_name(id)
    }
}
