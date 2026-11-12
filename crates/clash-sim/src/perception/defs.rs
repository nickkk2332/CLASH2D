//! The definition types of 15 §3 and §4: `signal_kind`, `sense` and
//! `event_signal`, and [`PerceptionDefs`], the tables worked out from
//! them once mods have loaded.

use std::collections::BTreeMap;
use std::sync::Arc;

use clash_core::{Dur, Real, define_id};
use clash_data::expr::Number;
use clash_data::{
    CompileCx, Cond, CrossCx, Def, DefLabel, Defs, Diagnostic, Expr, Length, Located, Ref, Scope,
    Speed, Text, Time, ValidateCx, code,
};
use clash_world::defs::MaterialDef;
use clash_world::field::FieldDef;
use clash_world::{FieldId, WorldDefs};
use serde::Deserialize;

use crate::body::BodyFunctionDef;
use crate::stats::PoolDef;

define_id! {
    /// A `signal_kind` definition (15 §3.1).
    pub struct SignalKindId(u16);
    /// A `sense` definition (15 §4).
    pub struct SenseId(u16);
    /// An `event_signal` definition (15 §3.2).
    pub struct EventSignalId(u16);
}

/// The whole angle a sense covers when its definition leaves `arc` out:
/// all the way round.
pub const FULL_ARC_DEG: Real = Real::of(360.0);

/// How fast awareness builds when a sense's definition leaves `gain` out
/// (15 §5.1).
pub const DEFAULT_GAIN: Real = Real::of(4.0);

/// How fast awareness fades when a sense's definition leaves `decay`
/// out, per second.
pub const DEFAULT_DECAY: Real = Real::of(0.5);

/// The level at which something is suspected when a sense's definition
/// leaves `suspect` out (15 §5.1).
pub const DEFAULT_SUSPECT: Real = Real::of(0.3);

/// How often a sense sweeps when its definition leaves `interval` out, in
/// milliseconds of its owner's time.
pub const DEFAULT_INTERVAL_MS: i64 = 500;

/// How a signal of a kind reaches an observer (15 §3.1).
#[derive(Clone, Copy, PartialEq, Eq, Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SignalModel {
    /// Needs a line of sight; scaled by how much of the source is
    /// exposed and, for a kind that uses light, by the light at it.
    Line,
    /// Passes obstacles with loss, by what the elements on the straight
    /// path are made of.
    Through,
    /// Read from a field in the observer's own cell.
    Field,
}

impl SignalModel {
    /// The name data writes it with.
    pub const fn name(self) -> &'static str {
        match self {
            SignalModel::Line => "line",
            SignalModel::Through => "through",
            SignalModel::Field => "field",
        }
    }
}

/// How much of a signal of a kind what stands in its way holds back
/// (`through` model, 15 §3.1): a share from 0 to 1 by material, by a tag
/// of the material, or for any other material.
#[derive(Clone, Debug, Default, Deserialize)]
pub struct BlockSpec {
    /// What a material not named below and carrying none of the tags
    /// below holds back. Default 0.
    #[serde(default)]
    pub default: Real,
    /// What each named material holds back.
    #[serde(default)]
    pub materials: BTreeMap<Ref<MaterialDef>, Real>,
    /// What a material carrying a tag holds back. Where a material
    /// carries several, the largest share counts.
    #[serde(default)]
    pub tags: BTreeMap<String, Real>,
}

/// How a standing source of a kind adds to a pool of those it reaches
/// (15 §3.1).
#[derive(Clone, Debug, Deserialize)]
pub struct ExposesSpec {
    /// The pool added to.
    pub pool: Ref<PoolDef>,
    /// What each unit of strength received adds, each step.
    pub per_strength: Real,
    /// How often the source steps, in its own time.
    pub step: Time,
}

/// A kind of signal (15 §3.1): how strength reaches an observer.
///
/// ```json
/// { "type": "signal_kind", "id": "sound", "model": "through", "falloff": 2,
///   "delay": true, "noise": true,
///   "block": { "default": 0.2, "tags": { "masonry": 0.7 } } }
/// ```
#[derive(Debug, Deserialize)]
pub struct SignalKindDef {
    /// What a reader calls it.
    #[serde(default)]
    pub name: Option<Text>,
    /// How strength reaches an observer.
    pub model: SignalModel,
    /// The power of the distance strength falls off by. Default 1.
    #[serde(default = "one")]
    pub falloff: Real,
    /// Whether a `line` signal is scaled by the light at its source.
    #[serde(default)]
    pub uses_light: bool,
    /// Whether the signal takes `distance / speed` to arrive.
    #[serde(default)]
    pub delay: bool,
    /// The speed a delayed signal travels at. Left out, the speed of
    /// sound of the atmosphere where it starts; in vacuum such a signal
    /// goes nowhere.
    #[serde(default)]
    pub speed: Option<Speed>,
    /// For the `field` model: the field read.
    #[serde(default)]
    pub field: Option<Ref<FieldDef>>,
    /// For the `through` model: what holds it back.
    #[serde(default)]
    pub block: BlockSpec,
    /// The standing signature of an entity that has not been given one
    /// of this kind. Default 0.
    #[serde(default)]
    pub standing: Real,
    /// The share of a standing signature that leaves a container its
    /// owner is inside, and of a signal that reaches an observer inside
    /// one. Default 1.
    #[serde(default = "one")]
    pub contained: Real,
    /// Whether this is the kind a bare `noise` number means (15 §3.2). At
    /// most one kind may say so.
    #[serde(default)]
    pub noise: bool,
    /// Whether receiving a signal of this kind tells who or what it is
    /// about. Default false: a sound tells where, not who.
    #[serde(default)]
    pub identifies: bool,
    /// What a standing source of this kind does to the pools of those it
    /// reaches.
    #[serde(default)]
    pub exposes: Option<ExposesSpec>,
    /// What it is, for the reference.
    #[serde(default)]
    pub doc: String,
}

fn one() -> Real {
    Real::ONE
}

fn full_arc() -> Real {
    FULL_ARC_DEG
}

fn default_gain() -> Real {
    DEFAULT_GAIN
}

fn default_decay() -> Real {
    DEFAULT_DECAY
}

fn default_suspect() -> Real {
    DEFAULT_SUSPECT
}

fn default_interval() -> Time {
    Time(Dur::from_millis(DEFAULT_INTERVAL_MS))
}

fn share(value: Real) -> bool {
    value.is_finite() && value >= Real::ZERO && value <= Real::ONE
}

impl Def for SignalKindDef {
    type Id = SignalKindId;
    const TYPE: &'static str = "signal_kind";
    const DOC: &'static str = "A kind of signal: light off a body, a sound, a scent. How its strength reaches an observer (15 §3.1).";

    fn compile(&mut self, cx: &mut CompileCx<'_>) {
        cx.compile(&mut self.name, &Scope::new().pure());
    }

    fn validate(&self, cx: &mut ValidateCx<'_>) {
        if !self.falloff.is_finite() || self.falloff < Real::ZERO {
            cx.error("`falloff` must be a number of zero or more");
        }
        if !self.standing.is_finite() || self.standing < Real::ZERO {
            cx.error("`standing` must be a number of zero or more");
        }
        if !share(self.contained) {
            cx.error("`contained` is a share from 0 to 1");
        }
        if !share(self.block.default)
            || self.block.materials.values().any(|value| !share(*value))
            || self.block.tags.values().any(|value| !share(*value))
        {
            cx.error("what `block` holds back is a share from 0 to 1");
        }
        if let Some(speed) = self.speed
            && (!speed.get().is_finite() || speed.get() <= Real::ZERO)
        {
            cx.error("`speed` must be above zero");
        }
        match self.model {
            SignalModel::Field => {
                if self.field.is_none() {
                    cx.error("a signal kind of the `field` model names the `field` it reads");
                }
                if self.delay {
                    cx.error("a `field` signal is read where it is and cannot be delayed");
                }
            }
            SignalModel::Line | SignalModel::Through => {
                if self.field.is_some() {
                    cx.error("only a signal kind of the `field` model names a `field`");
                }
            }
        }
        if self.uses_light && self.model != SignalModel::Line {
            cx.error("only a `line` signal can use light");
        }
        if let Some(exposes) = &self.exposes
            && (!exposes.per_strength.is_finite() || exposes.step.dur() <= Dur::ZERO)
        {
            cx.error("`exposes` needs a finite `per_strength` and a `step` above zero");
        }
    }
}

/// A sense (15 §4): what an entity perceives signals of a kind with.
///
/// ```json
/// { "type": "sense", "id": "human_sight", "signal": "sight",
///   "range": 60, "arc": 200, "threshold": 0.02,
///   "gain": 4.0, "decay": 0.5, "interval": "500 ms" }
/// ```
#[derive(Debug, Deserialize)]
pub struct SenseDef {
    /// What a reader calls it.
    #[serde(default)]
    pub name: Option<Text>,
    /// The kind of signal it receives.
    pub signal: Ref<SignalKindDef>,
    /// Beyond this nothing is considered.
    pub range: Length,
    /// The whole angle it covers, in degrees, centred on the facing. 360
    /// if left out.
    #[serde(default = "full_arc")]
    pub arc: Real,
    /// The least received strength that registers: an expression about
    /// the owner (`self`), pure.
    pub threshold: Expr<Number>,
    /// How fast awareness builds (15 §5.1). Default 4.
    #[serde(default = "default_gain")]
    pub gain: Real,
    /// How fast awareness fades, per second. Default 0.5.
    #[serde(default = "default_decay")]
    pub decay: Real,
    /// How often it sweeps for standing signatures, in its owner's time.
    /// Default 500 ms.
    #[serde(default = "default_interval")]
    pub interval: Time,
    /// The awareness at which something is suspected. Default 0.3.
    #[serde(default = "default_suspect")]
    pub suspect: Real,
    /// The body function it works by. An owner with a body whose capacity
    /// of it is zero receives nothing through it; an owner with no body
    /// is not asked.
    #[serde(default)]
    pub function: Option<Ref<BodyFunctionDef>>,
    /// Whether it receives from every plane, not only those its owner is
    /// on (15 §5.3).
    #[serde(default)]
    pub planes: bool,
    /// How hard it looks through a disguise (15 §8.2). Kept for
    /// `identity_check`.
    #[serde(default)]
    pub scrutiny: Real,
    /// What it is, for the reference.
    #[serde(default)]
    pub doc: String,
}

impl Def for SenseDef {
    type Id = SenseId;
    const TYPE: &'static str = "sense";
    const DOC: &'static str = "What an entity perceives signals of one kind with: its range, arc, threshold and how fast awareness builds and fades (15 §4).";

    fn compile(&mut self, cx: &mut CompileCx<'_>) {
        cx.compile(&mut self.name, &Scope::new().pure());
        cx.compile(
            &mut self.threshold,
            &Scope::new().own("self", "entity").pure(),
        );
    }

    fn validate(&self, cx: &mut ValidateCx<'_>) {
        let range = self.range.get();
        if !range.is_finite() || range < Real::ZERO {
            cx.error("`range` must be a distance of zero or more");
        }
        if !self.arc.is_finite() || self.arc <= Real::ZERO || self.arc > FULL_ARC_DEG {
            cx.error("`arc` is an angle above 0 and at most 360 degrees");
        }
        for (name, value) in [
            ("gain", self.gain),
            ("decay", self.decay),
            ("scrutiny", self.scrutiny),
        ] {
            if !value.is_finite() || value < Real::ZERO {
                cx.error(format!("`{name}` must be a number of zero or more"));
            }
        }
        if !self.suspect.is_finite() || self.suspect <= Real::ZERO || self.suspect >= Real::ONE {
            cx.error("`suspect` is a level above 0 and below 1");
        }
        if self.interval.dur() <= Dur::ZERO {
            cx.error("`interval` must be above zero");
        }
    }
}

/// An event that gives off a signal (15 §3.2): breaking glass makes a
/// sound in proportion to the energy involved.
///
/// ```json
/// { "type": "event_signal", "id": "wall_breaks", "event": "element_destroyed",
///   "signal": "sound", "strength": "1 + event.absorbed / 2000", "salience": 0.5 }
/// ```
#[derive(Debug, Deserialize)]
pub struct EventSignalDef {
    /// The event type: an engine event by its name, a mod's by its
    /// qualified id.
    pub event: Located<String>,
    /// The kind of signal it gives off.
    pub signal: Ref<SignalKindDef>,
    /// How strong: an expression in the event's scope, pure.
    pub strength: Expr<Number>,
    /// Whether this occurrence gives it off at all: a condition in the
    /// event's scope. Default true.
    #[serde(default, rename = "if")]
    pub condition: Cond,
    /// The subject of the event, by name, that the signal is about: whose
    /// awareness it builds in those who receive it. Left out, it is about
    /// nothing: breaking glass tells where, not who broke it.
    #[serde(default)]
    pub about: Option<Located<String>>,
    /// The salience a percept of it is remembered with (15 §7.1); 0, the
    /// default, is not remembered.
    #[serde(default)]
    pub salience: Real,
    /// What it is, for the reference.
    #[serde(default)]
    pub doc: String,
}

impl Def for EventSignalDef {
    type Id = EventSignalId;
    const TYPE: &'static str = "event_signal";
    const DOC: &'static str = "An event that gives off a signal where it happens, with a strength worked out from the event (15 §3.2).";

    fn compile(&mut self, cx: &mut CompileCx<'_>) {
        let scope = cx.event_scope(&self.event.value).pure();
        cx.compile(&mut self.strength, &scope);
        cx.compile(&mut self.condition, &scope);
        if let Some(about) = &self.about {
            let registry = cx.registry();
            let known = registry
                .event_id(&self.event.value)
                .and_then(|id| registry.event(id))
                .is_some_and(|event| {
                    event
                        .spec
                        .subjects
                        .iter()
                        .any(|(name, _)| *name == about.value)
                });
            if !known {
                let diagnostic = Diagnostic::new(
                    code::VALIDATION,
                    format!(
                        "the event `{}` has no subject `{}` for the signal to be about",
                        self.event.value, about.value
                    ),
                )
                .with_path("about")
                .at(cx.sources(), about.span);
                cx.report(diagnostic);
            }
        }
    }

    fn validate(&self, cx: &mut ValidateCx<'_>) {
        if !share(self.salience) {
            cx.error("`salience` is a number from 0 to 1");
        }
    }
}

/// Reports a second signal kind flagged `noise` (15 §3.2): a bare
/// `noise` number must mean one kind.
pub(super) fn check_one_noise(cx: &mut CrossCx<'_>) {
    let view = cx.defs();
    let noisy: Vec<&str> = view
        .iter::<SignalKindDef>()
        .filter(|(_, def)| def.noise)
        .filter_map(|(id, _)| view.name::<SignalKindDef>(id))
        .collect();
    if noisy.len() > 1 {
        let label = DefLabel::new(SignalKindDef::TYPE, noisy[1]);
        cx.report(
            Diagnostic::new(
                code::VALIDATION,
                format!(
                    "more than one signal kind is flagged `noise`: {}",
                    noisy.join(", ")
                ),
            )
            .in_def(Some(&label))
            .with_path("noise")
            .suggest("a bare `noise` number means one kind: flag one"),
        );
    }
}

/// A signal kind as the simulation uses it.
#[derive(Clone, Debug)]
pub struct KindInfo {
    /// How strength reaches an observer.
    pub model: SignalModel,
    /// The power of the distance strength falls off by.
    pub falloff: Real,
    /// Whether a `line` signal is scaled by light.
    pub uses_light: bool,
    /// Whether it is delayed by distance over speed.
    pub delay: bool,
    /// Its own speed, if it gives one.
    pub speed: Option<Real>,
    /// The field read, for the `field` model.
    pub field: Option<FieldId>,
    /// What each material of the world holds back, by the material's
    /// index in the world's table.
    pub block: Vec<Real>,
    /// What any other material holds back.
    pub block_default: Real,
    /// The standing signature of an entity not given one.
    pub standing: Real,
    /// The share that passes a container's walls.
    pub contained: Real,
    /// Whether receiving it tells who it is about.
    pub identifies: bool,
    /// The furthest a sense of this kind reaches, in metres: how far an
    /// emitted signal of it is followed.
    pub reach: Real,
}

impl KindInfo {
    /// What a material, by its index in the world's table, holds back.
    pub fn block_of(&self, material: usize) -> Real {
        self.block
            .get(material)
            .copied()
            .unwrap_or(self.block_default)
    }
}

/// The tables of perception, worked out from loaded definitions. Not
/// state: made with the simulation and again after a load.
#[derive(Clone, Debug)]
pub struct PerceptionDefs {
    defs: Option<Arc<Defs>>,
    kinds: Vec<KindInfo>,
    /// For each sense, its kind.
    sense_kind: Vec<SignalKindId>,
    /// The event signals of each event type, by the event's name.
    by_event: BTreeMap<String, Vec<EventSignalId>>,
    noise: Option<SignalKindId>,
}

impl PerceptionDefs {
    /// No signal kinds, senses or event signals: what a simulation over
    /// definitions loaded without this package's types has.
    pub fn empty() -> PerceptionDefs {
        PerceptionDefs {
            defs: None,
            kinds: Vec::new(),
            sense_kind: Vec::new(),
            by_event: BTreeMap::new(),
            noise: None,
        }
    }

    /// The tables of a set of loaded definitions and the world's tables
    /// made from them.
    pub fn new(defs: &Arc<Defs>, world: &WorldDefs) -> PerceptionDefs {
        let (Some(kinds), Some(senses)) = (
            defs.try_registry::<SignalKindDef>(),
            defs.try_registry::<SenseDef>(),
        ) else {
            return PerceptionDefs::empty();
        };
        let materials = world.materials();
        let mut reach = vec![Real::ZERO; kinds.len()];
        let mut sense_kind = Vec::with_capacity(senses.len());
        for (_, sense) in senses.iter() {
            let kind = sense.signal.id();
            sense_kind.push(kind);
            if let Some(slot) = reach.get_mut(kind.index()) {
                *slot = (*slot).max(sense.range.get());
            }
        }
        let mut infos = Vec::with_capacity(kinds.len());
        let mut noise = None;
        for (id, kind) in kinds.iter() {
            if kind.noise && noise.is_none() {
                noise = Some(id);
            }
            let mut block = Vec::with_capacity(materials.len());
            for (material_id, material) in materials.iter() {
                let name = materials.name(material_id);
                let named = kind
                    .block
                    .materials
                    .iter()
                    .find(|(wanted, _)| def_name::<MaterialDef>(defs, wanted.id()) == name)
                    .map(|(_, share)| *share);
                let tagged = material
                    .tags
                    .iter()
                    .filter_map(|tag| kind.block.tags.get(tag).copied())
                    .reduce(Real::max);
                block.push(named.or(tagged).unwrap_or(kind.block.default));
            }
            let field = kind.field.and_then(|field| {
                world
                    .fields()
                    .lookup(def_name::<FieldDef>(defs, field.id()))
            });
            infos.push(KindInfo {
                model: kind.model,
                falloff: kind.falloff,
                uses_light: kind.uses_light,
                delay: kind.delay,
                speed: kind.speed.map(Speed::get),
                field,
                block,
                block_default: kind.block.default,
                standing: kind.standing,
                contained: kind.contained,
                identifies: kind.identifies,
                reach: reach.get(id.index()).copied().unwrap_or(Real::ZERO),
            });
        }
        let mut by_event: BTreeMap<String, Vec<EventSignalId>> = BTreeMap::new();
        if let Some(signals) = defs.try_registry::<EventSignalDef>() {
            for (id, signal) in signals.iter() {
                by_event
                    .entry(signal.event.value.clone())
                    .or_default()
                    .push(id);
            }
        }
        PerceptionDefs {
            defs: Some(Arc::clone(defs)),
            kinds: infos,
            sense_kind,
            by_event,
            noise,
        }
    }

    /// A signal kind's table.
    pub fn kind(&self, kind: SignalKindId) -> Option<&KindInfo> {
        self.kinds.get(kind.index())
    }

    /// How many signal kinds there are.
    pub fn kind_count(&self) -> usize {
        self.kinds.len()
    }

    /// A signal kind by its qualified id, or by its bare id where no
    /// other kind has it.
    pub fn kind_named(&self, name: &str) -> Option<SignalKindId> {
        lookup::<SignalKindDef>(self.defs.as_deref()?, name)
    }

    /// A sense by its qualified id, or by its bare id where no other
    /// sense has it.
    pub fn sense_named(&self, name: &str) -> Option<SenseId> {
        lookup::<SenseDef>(self.defs.as_deref()?, name)
    }

    /// A signal kind's qualified id.
    pub fn kind_name(&self, kind: SignalKindId) -> &str {
        self.defs
            .as_deref()
            .map_or("", |defs| def_name::<SignalKindDef>(defs, kind))
    }

    /// A sense's qualified id.
    pub fn sense_name(&self, sense: SenseId) -> &str {
        self.defs
            .as_deref()
            .map_or("", |defs| def_name::<SenseDef>(defs, sense))
    }

    /// A sense's definition.
    pub fn sense(&self, sense: SenseId) -> Option<&SenseDef> {
        self.defs
            .as_deref()?
            .try_registry::<SenseDef>()?
            .try_get(sense)
    }

    /// The kind a sense receives.
    pub fn sense_kind(&self, sense: SenseId) -> Option<SignalKindId> {
        self.sense_kind.get(sense.index()).copied()
    }

    /// An event signal's definition.
    pub fn event_signal(&self, id: EventSignalId) -> Option<&EventSignalDef> {
        self.defs
            .as_deref()?
            .try_registry::<EventSignalDef>()?
            .try_get(id)
    }

    /// The event signals an event type gives off, in id order.
    pub fn signals_of(&self, event: &str) -> &[EventSignalId] {
        self.by_event.get(event).map_or(&[], Vec::as_slice)
    }

    /// Whether any event signal is defined.
    pub fn has_event_signals(&self) -> bool {
        !self.by_event.is_empty()
    }

    /// The kind a bare `noise` number means, if one is flagged.
    pub fn noise(&self) -> Option<SignalKindId> {
        self.noise
    }

    /// The definitions these tables were made from.
    pub fn defs(&self) -> Option<&Arc<Defs>> {
        self.defs.as_ref()
    }
}

/// A definition's qualified id, or `""` if the type is not registered or
/// the id names none.
fn def_name<T: Def>(defs: &Defs, id: T::Id) -> &str {
    defs.try_registry::<T>()
        .map_or("", |registry| registry.name(id))
}

/// A definition by its qualified id, or by its bare id where exactly one
/// definition of the type has it.
fn lookup<T: Def>(defs: &Defs, name: &str) -> Option<T::Id> {
    if let Some(id) = defs.lookup::<T>(name) {
        return Some(id);
    }
    if name.contains(':') {
        return None;
    }
    let registry = defs.try_registry::<T>()?;
    let mut found = None;
    for (id, _) in registry.iter() {
        let qualified = registry.name(id);
        if qualified.rsplit_once(':').map(|(_, bare)| bare) == Some(name) {
            if found.is_some() {
                return None;
            }
            found = Some(id);
        }
    }
    found
}
