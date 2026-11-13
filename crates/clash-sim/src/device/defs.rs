//! The `device` definition type (16 §4.2), as far as package E12 needs
//! it, and the check that every element's `device` names one.

use std::collections::BTreeMap;

use clash_core::{Real, define_id};
use clash_data::{
    CompileCx, CrossCx, Def, DefLabel, Diagnostic, FromKind, Kind, KindValue, Ref, Scope, Text,
    ValidateCx, by_kind, code,
};
use clash_world::defs::{BlockDef, FixtureDef, FloorDef};
use serde::Deserialize;
use serde::de::{Deserializer, MapAccess};

use crate::interaction::InteractionDef;
use crate::item::HoldsDef;
use crate::recipe::{KnowHowDef, RecipeDef};

define_id! {
    /// A `device` definition (16 §4.2).
    pub struct DeviceId(u16);
}

/// What an element becomes when it needs state of its own, or a machine
/// standing alone (16 §4.2).
///
/// ```json
/// { "type": "device", "id": "locker_device", "name": "locker",
///   "holds": { "volume": 0.6, "mass": "200 kg" },
///   "tags": ["storage"] }
/// ```
///
/// Package G2 adds modes, config, stores, a speed, `auto` and what it
/// provides; ports, signals, slots and rules wait for later packages.
#[derive(Debug, Deserialize)]
pub struct DeviceDef {
    /// What a reader calls it.
    #[serde(default)]
    pub name: Option<Text>,
    /// What it holds (06 §9.2): its room by volume and mass and the tags
    /// it takes. Left out, nothing can be put in it.
    #[serde(default)]
    pub holds: Option<HoldsDef>,
    /// Tags the device entity carries.
    #[serde(default)]
    pub tags: Vec<String>,
    /// The interactions the device entity offers (16 §3.2), done to it
    /// as an entity. Those of the element it is bound to are the
    /// element's.
    #[serde(default)]
    pub interactions: Vec<Ref<InteractionDef>>,
    /// Named operating states (16 §4.2): the first is the one it starts
    /// in, and fields that say `when` apply in those named.
    #[serde(default)]
    pub modes: Vec<String>,
    /// Settings a user may change (16 §4.2 `config`).
    #[serde(default)]
    pub config: Vec<ConfigDef>,
    /// What it stores, by resource: a quantity it is filled with and
    /// draws on while it works (16 §5.2, without networks).
    #[serde(default)]
    pub stores: BTreeMap<String, StoreDef>,
    /// How fast it works recipes, and in which modes (16 §8.2). Left
    /// out, 1 in every mode.
    #[serde(default)]
    pub speed: Option<SpeedDef>,
    /// Whether it runs recipes by itself, and in which modes (16 §8.2).
    #[serde(default)]
    pub auto: Option<AutoDef>,
    /// Know-how it provides to whoever works at it (16 §8.5): a manual
    /// built in, a pattern loaded.
    #[serde(default)]
    pub provides: Vec<ProvidesDef>,
    /// The recipes it runs, in order of preference. Left out, every
    /// recipe whose `at` names one of its tags, in order of id.
    #[serde(default)]
    pub recipes: Vec<Ref<RecipeDef>>,
}

/// The kinds of setting (16 §4.2 `config`).
#[derive(Clone, Copy, PartialEq, Eq, Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConfigKind {
    /// On or off.
    Flag,
    /// A number, between `min` and `max` where they are given.
    Number,
    /// One of `options`.
    Choice,
    /// Any text.
    Text,
}

impl ConfigKind {
    /// The name as data writes it.
    pub const fn name(self) -> &'static str {
        match self {
            ConfigKind::Flag => "flag",
            ConfigKind::Number => "number",
            ConfigKind::Choice => "choice",
            ConfigKind::Text => "text",
        }
    }
}

/// A plain value a setting holds, as data writes it.
#[derive(Clone, PartialEq, Debug)]
pub enum ConfigValue {
    /// `true` or `false`.
    Bool(bool),
    /// A number.
    Number(Real),
    /// A text.
    Text(String),
}

impl ConfigValue {
    /// The value as the simulation stores it.
    pub fn stored(&self) -> clash_core::Value {
        match self {
            ConfigValue::Bool(on) => clash_core::Value::Bool(*on),
            ConfigValue::Number(number) => clash_core::Value::Real(*number),
            ConfigValue::Text(text) => clash_core::Value::Str(text.clone()),
        }
    }
}

impl<'de> FromKind<'de> for ConfigValue {
    fn from_kind<A: MapAccess<'de>>(
        kind: Kind,
        value: KindValue<'_, A>,
    ) -> Result<ConfigValue, A::Error> {
        match kind {
            Kind::Bool => value.read().map(ConfigValue::Bool),
            Kind::Number => value.read().map(ConfigValue::Number),
            Kind::String => value.read().map(ConfigValue::Text),
            other => Err(other.unexpected("`true`, `false`, a number or a text")),
        }
    }
}

impl<'de> Deserialize<'de> for ConfigValue {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<ConfigValue, D::Error> {
        by_kind(deserializer)
    }
}

/// One setting of a device (16 §4.2 `config`).
#[derive(Debug, Deserialize)]
pub struct ConfigDef {
    /// Its name: `self.config.<id>`.
    pub id: String,
    /// What a screen calls it.
    #[serde(default)]
    pub name: Option<Text>,
    /// What kind of value it holds.
    pub kind: ConfigKind,
    /// What it holds until it is set. Left out: false, 0, the first
    /// option, or empty.
    #[serde(default)]
    pub default: Option<ConfigValue>,
    /// For a choice, what it may be.
    #[serde(default)]
    pub options: Vec<String>,
    /// For a number, the least it may be.
    #[serde(default)]
    pub min: Option<Real>,
    /// For a number, the most it may be.
    #[serde(default)]
    pub max: Option<Real>,
}

impl ConfigDef {
    /// What it holds until it is set.
    pub fn default_value(&self) -> clash_core::Value {
        match (&self.default, self.kind) {
            (Some(value), _) => value.stored(),
            (None, ConfigKind::Flag) => clash_core::Value::Bool(false),
            (None, ConfigKind::Number) => clash_core::Value::Real(Real::ZERO),
            (None, ConfigKind::Choice) => {
                clash_core::Value::Str(self.options.first().cloned().unwrap_or_default())
            }
            (None, ConfigKind::Text) => clash_core::Value::Str(String::new()),
        }
    }
}

/// A store of a resource (16 §5.2 `stores`).
#[derive(Debug, Deserialize)]
pub struct StoreDef {
    /// The most it holds, in the resource's unit.
    pub capacity: Real,
    /// What it holds when the device is made (default 0).
    #[serde(default)]
    pub start: Real,
    /// The most it gives a second, if it is limited: a machine that
    /// wants more works at the share it gets (16 §5.4).
    #[serde(default)]
    pub rate: Option<Real>,
}

/// The modes a field applies in (16 §4.2 `when`): empty for all.
#[derive(Clone, Debug, Default, Deserialize)]
pub struct WhenModes(pub Vec<String>);

impl WhenModes {
    /// Whether it applies in a mode, by name; in any mode if it names
    /// none.
    pub fn holds(&self, mode: Option<&str>) -> bool {
        self.0.is_empty() || mode.is_some_and(|mode| self.0.iter().any(|named| named == mode))
    }
}

/// How fast a device works recipes (16 §8.2 `speed`): a number, or
/// `{ "value": 1.0, "when": ["lit"] }`, outside whose modes it does not
/// work at all.
#[derive(Clone, Debug)]
pub struct SpeedDef {
    /// The factor on the rate of work done at it.
    pub value: Real,
    /// The modes it works in.
    pub when: WhenModes,
}

#[derive(Deserialize)]
struct SpeedObject {
    #[serde(default = "one")]
    value: Real,
    #[serde(default)]
    when: Vec<String>,
}

fn one() -> Real {
    Real::ONE
}

impl<'de> FromKind<'de> for SpeedDef {
    fn from_kind<A: MapAccess<'de>>(
        kind: Kind,
        value: KindValue<'_, A>,
    ) -> Result<SpeedDef, A::Error> {
        match kind {
            Kind::Number => value.read().map(|value| SpeedDef {
                value,
                when: WhenModes::default(),
            }),
            Kind::Object => value.read::<SpeedObject>().map(|object| SpeedDef {
                value: object.value,
                when: WhenModes(object.when),
            }),
            other => Err(other.unexpected("a number, or an object with `value` and `when`")),
        }
    }
}

impl<'de> Deserialize<'de> for SpeedDef {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<SpeedDef, D::Error> {
        by_kind(deserializer)
    }
}

/// Whether a device runs recipes by itself (16 §8.2 `auto`): `true`, or
/// `{ "when": ["on"] }` for only in those modes.
#[derive(Clone, Debug)]
pub struct AutoDef {
    /// Whether it does at all.
    pub on: bool,
    /// The modes it does in.
    pub when: WhenModes,
}

#[derive(Deserialize)]
struct WhenObject {
    #[serde(default)]
    when: Vec<String>,
}

impl<'de> FromKind<'de> for AutoDef {
    fn from_kind<A: MapAccess<'de>>(
        kind: Kind,
        value: KindValue<'_, A>,
    ) -> Result<AutoDef, A::Error> {
        match kind {
            Kind::Bool => value.read().map(|on| AutoDef {
                on,
                when: WhenModes::default(),
            }),
            Kind::Object => value.read::<WhenObject>().map(|object| AutoDef {
                on: true,
                when: WhenModes(object.when),
            }),
            other => Err(other.unexpected("`true`, `false`, or an object with `when`")),
        }
    }
}

impl<'de> Deserialize<'de> for AutoDef {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<AutoDef, D::Error> {
        by_kind(deserializer)
    }
}

/// Know-how a device provides (16 §8.5 `provides`): its id, or
/// `{ "know_how": id, "when": [modes] }`.
#[derive(Debug)]
pub struct ProvidesDef {
    /// The know-how.
    pub know_how: Ref<KnowHowDef>,
    /// The modes it provides it in.
    pub when: WhenModes,
}

#[derive(Deserialize)]
struct ProvidesObject {
    know_how: Ref<KnowHowDef>,
    #[serde(default)]
    when: Vec<String>,
}

impl<'de> FromKind<'de> for ProvidesDef {
    fn from_kind<A: MapAccess<'de>>(
        kind: Kind,
        value: KindValue<'_, A>,
    ) -> Result<ProvidesDef, A::Error> {
        match kind {
            Kind::String => value.read().map(|know_how| ProvidesDef {
                know_how,
                when: WhenModes::default(),
            }),
            Kind::Object => value.read::<ProvidesObject>().map(|object| ProvidesDef {
                know_how: object.know_how,
                when: WhenModes(object.when),
            }),
            other => Err(other.unexpected("a know-how id, or an object with `know_how` and `when`")),
        }
    }
}

impl<'de> Deserialize<'de> for ProvidesDef {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<ProvidesDef, D::Error> {
        by_kind(deserializer)
    }
}

impl DeviceDef {
    /// The position of a mode among its `modes`, by name.
    pub fn mode_index(&self, name: &str) -> Option<usize> {
        self.modes.iter().position(|mode| mode == name)
    }

    /// A setting, by id.
    pub fn config_entry(&self, id: &str) -> Option<&ConfigDef> {
        self.config.iter().find(|entry| entry.id == id)
    }

    /// Every `when` of the definition, with where it is written.
    fn whens(&self) -> Vec<(String, &WhenModes)> {
        let mut whens = Vec::new();
        if let Some(speed) = &self.speed {
            whens.push(("speed.when".to_owned(), &speed.when));
        }
        if let Some(auto) = &self.auto {
            whens.push(("auto.when".to_owned(), &auto.when));
        }
        for (index, provides) in self.provides.iter().enumerate() {
            whens.push((format!("provides {index}.when"), &provides.when));
        }
        whens
    }
}

impl Def for DeviceDef {
    type Id = DeviceId;
    const TYPE: &'static str = "device";
    const DOC: &'static str = "What an element becomes when it needs state of its own, or a machine standing alone: what it holds, its tags, modes, settings and stores, how fast and whether by itself it works recipes, and the know-how it provides (16 §4.2).";

    fn compile(&mut self, cx: &mut CompileCx<'_>) {
        cx.compile(&mut self.name, &Scope::new().pure());
        for entry in &mut self.config {
            cx.compile(&mut entry.name, &Scope::new().pure());
        }
    }

    fn validate(&self, cx: &mut ValidateCx<'_>) {
        for (index, mode) in self.modes.iter().enumerate() {
            if self.modes[..index].contains(mode) {
                cx.error(format!("modes: `{mode}` is named twice"));
            }
        }
        for (path, when) in self.whens() {
            for mode in &when.0 {
                if !self.modes.contains(mode) {
                    cx.error(format!(
                        "{path}: `{mode}` is not one of the device's `modes`"
                    ));
                }
            }
        }
        for (index, entry) in self.config.iter().enumerate() {
            if self.config[..index]
                .iter()
                .any(|other| other.id == entry.id)
            {
                cx.error(format!("config: `{}` is named twice", entry.id));
            }
            let fits = match (entry.kind, &entry.default) {
                (_, None) | (ConfigKind::Flag, Some(ConfigValue::Bool(_))) => true,
                (ConfigKind::Number, Some(ConfigValue::Number(number))) => number.is_finite(),
                (ConfigKind::Choice, Some(ConfigValue::Text(text))) => {
                    entry.options.contains(text)
                }
                (ConfigKind::Text, Some(ConfigValue::Text(_))) => true,
                _ => false,
            };
            if !fits {
                cx.error(format!(
                    "config {index}: the `default` is not a value of the kind `{}`",
                    entry.kind.name()
                ));
            }
            if entry.kind == ConfigKind::Choice && entry.options.is_empty() {
                cx.error(format!("config {index}: a choice has `options`"));
            }
        }
        for (resource, store) in &self.stores {
            if !(store.capacity.is_finite() && store.capacity > Real::ZERO) {
                cx.error(format!("stores.{resource}: `capacity` is above zero"));
            }
            if !(store.start.is_finite()
                && store.start >= Real::ZERO
                && store.start <= store.capacity)
            {
                cx.error(format!(
                    "stores.{resource}: `start` is from zero to the capacity"
                ));
            }
            if store
                .rate
                .is_some_and(|rate| !(rate.is_finite() && rate >= Real::ZERO))
            {
                cx.error(format!("stores.{resource}: `rate` is zero or more"));
            }
        }
        if let Some(speed) = &self.speed
            && !(speed.value.is_finite() && speed.value >= Real::ZERO)
        {
            cx.error("`speed` is zero or more");
        }
    }
}

/// The namespace of a qualified id.
fn namespace_of(qualified: &str) -> &str {
    qualified
        .split_once(':')
        .map_or("", |(namespace, _)| namespace)
}

/// Whether a reference written in a definition of `owner` names a
/// definition of `T`: as written if qualified, else in the owner's
/// namespace.
pub(crate) fn names<T: Def>(cx: &CrossCx<'_>, owner: &str, written: &str) -> bool {
    let defs = cx.defs();
    if written.contains(':') {
        return defs.lookup::<T>(written).is_some();
    }
    defs.lookup::<T>(&format!("{}:{written}", namespace_of(owner)))
        .is_some()
}

/// Reports every element whose `device` names no `device` definition
/// (02 §5.5), as a warning: such an element is never promoted. A load
/// that defines no device at all is not checked. A cross check of the
/// loader.
pub fn check_element_devices(cx: &mut CrossCx<'_>) {
    let defs = cx.defs();
    // A load with no devices at all (the world's content alone, for a
    // test or a tool) is not one to promote any.
    if defs.iter::<DeviceDef>().next().is_none() {
        return;
    }
    let mut missing: Vec<(&'static str, String, String)> = Vec::new();
    let mut look = |ty: &'static str, name: &str, device: Option<&str>| {
        if let Some(device) = device
            && !names::<DeviceDef>(cx, name, device)
        {
            missing.push((ty, name.to_owned(), device.to_owned()));
        }
    };
    for (id, def) in defs.iter::<FloorDef>() {
        let name = defs.name::<FloorDef>(id).unwrap_or_default();
        look(
            FloorDef::TYPE,
            name,
            def.device.as_ref().map(|d| d.as_str()),
        );
    }
    for (id, def) in defs.iter::<BlockDef>() {
        let name = defs.name::<BlockDef>(id).unwrap_or_default();
        look(
            BlockDef::TYPE,
            name,
            def.device.as_ref().map(|d| d.as_str()),
        );
    }
    for (id, def) in defs.iter::<FixtureDef>() {
        let name = defs.name::<FixtureDef>(id).unwrap_or_default();
        look(
            FixtureDef::TYPE,
            name,
            def.device.as_ref().map(|d| d.as_str()),
        );
    }
    for (ty, element, device) in missing {
        let diagnostic = Diagnostic::new(
            code::VALIDATION_WARNING,
            format!("there is no device definition `{device}` for this element to become"),
        )
        .in_def(Some(&DefLabel::new(ty, element)))
        .with_path("device");
        cx.report(diagnostic);
    }
}
