//! The `item` definition type (06 §9.1): what a thing is made of, how it
//! is held and worn, what it holds, what it confers, and whether it
//! stacks.
//!
//! The engine gives none of it a meaning. An item is its material, its
//! mass and size, the layers it adds over the parts it is worn on and the
//! modifiers its data confers: there is no armour class and no damage
//! bonus (01 R2).

use std::collections::BTreeMap;

use clash_core::{Real, define_id};
use clash_data::{
    CompileCx, Def, FromKind, Kind, KindValue, Length, Located, Mass, Ref, Scope, TagSet,
    Temperature, Text, Time, ValidateCx, Volume, by_kind,
};
use clash_world::defs::{GlyphSpec, MaterialDef};
use serde::Deserialize;
use serde::de::{Deserializer, MapAccess};

use crate::action::{ActionDef, ChannelDef};
use crate::body::SurfaceDef;
use crate::rule::Rules;
use crate::stats::AttributeDef;

define_id! {
    /// An `item` definition (06 §9.1).
    pub struct ItemId;
}

/// The word that selects every part of a body in a `wear` entry.
pub const ALL_PARTS: &str = "all";

/// The most `wear` entries one item may have.
pub const MAX_WEAR_ENTRIES: usize = 32;

fn one() -> Real {
    Real::ONE
}

/// Whether a number is finite and at least zero.
fn amount(value: Real) -> bool {
    value.is_finite() && value >= Real::ZERO
}

/// Whether a number is finite and above zero.
fn positive(value: Real) -> bool {
    value.is_finite() && value > Real::ZERO
}

/// Whether a number is a share: from 0 to 1.
fn share(value: Real) -> bool {
    (Real::ZERO..=Real::ONE).contains(&value)
}

/// Which parts of a body a `wear` entry covers: every outer part, or
/// those that answer to one of a list of names.
///
/// A part answers to a name if the name is its name in its body
/// (`l_arm`), the id of its definition, qualified or bare (`arm`), or a
/// tag its definition carries (`limb`).
///
/// In data: `"all"`, one name, or a list of names.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum PartSelector {
    /// Every part on the outside of the body.
    All,
    /// The outer parts that answer to one of these names.
    Named(Vec<String>),
}

impl<'de> FromKind<'de> for PartSelector {
    fn from_kind<A: MapAccess<'de>>(
        kind: Kind,
        value: KindValue<'_, A>,
    ) -> Result<PartSelector, A::Error> {
        match kind {
            Kind::String => {
                let name = value.read::<String>()?;
                Ok(if name == ALL_PARTS {
                    PartSelector::All
                } else {
                    PartSelector::Named(vec![name])
                })
            }
            Kind::List => value.read::<Vec<String>>().map(PartSelector::Named),
            other => Err(other.unexpected("\"all\", a part's name, or a list of names")),
        }
    }
}

impl<'de> Deserialize<'de> for PartSelector {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<PartSelector, D::Error> {
        by_kind(deserializer)
    }
}

/// One entry of an item's `wear`: a layer the item adds over some parts
/// of whoever wears it (06 §9.1, §7.2).
///
/// ```json
/// { "parts": ["torso", "arm"], "layer": 3, "material": "leather",
///   "thickness": "3 mm", "coverage": 0.9 }
/// ```
#[derive(Debug, Deserialize)]
pub struct WearDef {
    /// The parts it covers.
    pub parts: PartSelector,
    /// Its order among what is worn over a part: a higher layer is
    /// further out. A part takes one item at each layer. Default 0.
    #[serde(default)]
    pub layer: i16,
    /// What the layer is made of. Left out, the item's own material.
    #[serde(default)]
    pub material: Option<Ref<MaterialDef>>,
    /// How thick the layer is there.
    pub thickness: Length,
    /// The share of hits on a covered part that the layer is in the way
    /// of, from 0 to 1. Default 1.
    #[serde(default = "one")]
    pub coverage: Real,
}

/// An item's `wear`: one entry or a list of them.
#[derive(Debug, Default)]
pub struct Wear(pub Vec<WearDef>);

impl<'de> FromKind<'de> for Wear {
    fn from_kind<A: MapAccess<'de>>(kind: Kind, value: KindValue<'_, A>) -> Result<Wear, A::Error> {
        match kind {
            Kind::Object => value.read::<WearDef>().map(|entry| Wear(vec![entry])),
            Kind::List => value.read::<Vec<WearDef>>().map(Wear),
            other => Err(other.unexpected("a `wear` entry, or a list of them")),
        }
    }
}

impl<'de> Deserialize<'de> for Wear {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Wear, D::Error> {
        by_kind(deserializer)
    }
}

/// An item's `holds`: its capacity as a container (06 §9.1).
///
/// ```json
/// "holds": { "volume": 0.03, "mass": "25 kg", "accepts": ["small"] }
/// ```
#[derive(Debug, Default, Deserialize)]
pub struct HoldsDef {
    /// The most space what is stowed in it may take up. Left out, any
    /// amount.
    #[serde(default)]
    pub volume: Option<Volume>,
    /// The most what is stowed in it may weigh, with everything inside
    /// that. Left out, any amount.
    #[serde(default)]
    pub mass: Option<Mass>,
    /// The tags it takes: an item goes in if it carries at least one of
    /// them. Empty for anything.
    #[serde(default)]
    pub accepts: Vec<String>,
    /// For a store (07 §11.5): the order it gives up what is stowed in
    /// it. Left out, it keeps no order, and equal things put in join
    /// whatever equal stack is there.
    #[serde(default)]
    pub order: Option<StoreOrder>,
}

/// The order a store gives up what is stowed in it (07 §11.5): a
/// magazine, a quiver, a belt.
///
/// A store keeps what was put into it in the order it was put in. An
/// equal thing put in joins only the stack put in last, so that two
/// kinds of round loaded in turn stay in turn.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StoreOrder {
    /// What was put in first comes out first: a belt.
    FirstIn,
    /// What was put in last comes out first: a box magazine, a quiver.
    LastIn,
}

/// An attribute modifier an item confers on whoever holds it in a grasp
/// or wears it (06 §9.1, §4.1).
#[derive(Debug, Deserialize)]
pub struct ModifierDef {
    /// The attribute.
    pub attr: Ref<AttributeDef>,
    /// Added to the attribute's base. Default 0.
    #[serde(default)]
    pub add: Real,
    /// Multiplied in after every addition. Default 1.
    #[serde(default = "one")]
    pub mul: Real,
}

/// How many of something: one number, or a range drawn from evenly.
///
/// In data: `3`, or `[1, 5]`.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct CountRange {
    /// The least.
    pub min: u32,
    /// The most.
    pub max: u32,
}

impl Default for CountRange {
    fn default() -> CountRange {
        CountRange { min: 1, max: 1 }
    }
}

impl<'de> FromKind<'de> for CountRange {
    fn from_kind<A: MapAccess<'de>>(
        kind: Kind,
        value: KindValue<'_, A>,
    ) -> Result<CountRange, A::Error> {
        match kind {
            Kind::Number => value.read::<u32>().map(|count| CountRange {
                min: count,
                max: count,
            }),
            Kind::List => value
                .read::<[u32; 2]>()
                .map(|[min, max]| CountRange { min, max }),
            other => Err(other.unexpected("a count, or `[least, most]`")),
        }
    }
}

impl<'de> Deserialize<'de> for CountRange {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<CountRange, D::Error> {
        by_kind(deserializer)
    }
}

/// One entry of a container's `contents`: what it is found to hold when
/// it is first opened (06 §9.3).
#[derive(Debug, Deserialize)]
pub struct ContentDef {
    /// What is in it.
    pub item: Ref<ItemDef>,
    /// How many: a number, or `[least, most]`. Default 1.
    #[serde(default)]
    pub count: CountRange,
    /// The chance that it is there at all, from 0 to 1. Default 1.
    #[serde(default = "one")]
    pub chance: Real,
}

/// The colour an item carries (06 §9.1): none, one, or one picked from a
/// list when the item is made.
///
/// In data: a colour's name, or a list of names.
#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub enum ColorChoice {
    /// The item carries no colour of its own.
    #[default]
    None,
    /// Always this one.
    One(String),
    /// One of these, picked by the item's seed when it is made.
    AnyOf(Vec<String>),
}

impl<'de> FromKind<'de> for ColorChoice {
    fn from_kind<A: MapAccess<'de>>(
        kind: Kind,
        value: KindValue<'_, A>,
    ) -> Result<ColorChoice, A::Error> {
        match kind {
            Kind::String => value.read::<String>().map(ColorChoice::One),
            Kind::List => value.read::<Vec<String>>().map(ColorChoice::AnyOf),
            other => Err(other.unexpected("a colour's name, or a list of names")),
        }
    }
}

impl<'de> Deserialize<'de> for ColorChoice {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<ColorChoice, D::Error> {
        by_kind(deserializer)
    }
}

/// A stage of an item's ageing (06 §9.4). Carried as data: ageing is not
/// built.
#[derive(Debug, Deserialize)]
pub struct StageDef {
    /// The age, from 0 to 1, at which the stage is reached.
    pub at: Real,
    /// The trait the item is given there, by id.
    #[serde(default, rename = "trait")]
    pub gives: Option<String>,
}

/// An item's `ages` (06 §9.4). Carried as data: ageing is not built.
#[derive(Debug, Deserialize)]
pub struct AgesDef {
    /// How long the item lasts at a rate of 1.
    pub life: Time,
    /// The rate, as an expression about where the item is. Kept as it
    /// was written until ageing compiles it.
    #[serde(default)]
    pub rate: Option<Located<String>>,
    /// The stages on the way.
    #[serde(default)]
    pub stages: Vec<StageDef>,
    /// What the item becomes at the end. Left out, it is removed.
    #[serde(default)]
    pub into: Option<Ref<ItemDef>>,
}

/// What a container keeps for its contents (06 §9.4). Carried as data:
/// ageing is not built.
#[derive(Debug, Default, Deserialize)]
pub struct KeepsDef {
    /// The temperature its contents read in place of the room's.
    #[serde(default)]
    pub temperature: Option<Temperature>,
    /// A factor on the ageing of its contents.
    #[serde(default)]
    pub ageing: Option<Real>,
}

/// A kind of item (06 §9.1).
///
/// ```json
/// { "type": "item", "id": "coat", "name": "coat",
///   "material": "leather", "mass": "1.8 kg", "volume": 0.002,
///   "grip": { "arm": 1 },
///   "wear": { "parts": ["torso", "arm"], "layer": 3, "thickness": "3 mm", "coverage": 0.9 },
///   "tags": ["clothing"] }
/// ```
#[derive(Debug, Deserialize)]
pub struct ItemDef {
    /// What a reader calls it.
    #[serde(default)]
    pub name: Option<Text>,
    /// What it is called by someone who has not learned what it is
    /// (05 §4.6).
    #[serde(default)]
    pub unknown_name: Option<Text>,
    /// What a closer look says of it.
    #[serde(default)]
    pub description: Option<Text>,
    /// What it is made of. It takes damage as that material does.
    pub material: Ref<MaterialDef>,
    /// The mass of one.
    pub mass: Mass,
    /// The space one takes up. Left out, its mass over its material's
    /// density.
    #[serde(default)]
    pub volume: Option<Volume>,
    /// How long it is along its greatest dimension.
    #[serde(default)]
    pub length: Length,
    /// Free-form tags. An item is made with them as its own.
    #[serde(default)]
    pub tags: TagSet,
    /// The parts of it that can be brought to bear (07 §3.2). Carried
    /// for combat.
    #[serde(default)]
    pub surfaces: Vec<SurfaceDef>,
    /// The channel slots needed to hold it in a grasp. An item with none
    /// cannot be held in one.
    #[serde(default)]
    pub grip: BTreeMap<Ref<ChannelDef>, u16>,
    /// The layers it adds over the parts it is worn on. An item with
    /// none cannot be worn.
    #[serde(default)]
    pub wear: Wear,
    /// Its capacity as a container. An item with none holds nothing.
    #[serde(default)]
    pub holds: Option<HoldsDef>,
    /// Whether equal items may share one entity with a count.
    #[serde(default)]
    pub stack: bool,
    /// Attribute modifiers on whoever holds it in a grasp or wears it.
    #[serde(default)]
    pub modifiers: Vec<ModifierDef>,
    /// Tags added to whoever holds it in a grasp or wears it, while
    /// they do.
    #[serde(default)]
    pub holder_tags: Vec<String>,
    /// Actions whoever holds or wears it may use. Carried for the
    /// action runtime.
    #[serde(default)]
    pub grants: Vec<Ref<ActionDef>>,
    /// Traits given to whoever holds or wears it, by id. Carried until
    /// traits are built.
    #[serde(default)]
    pub traits: Vec<String>,
    /// Rules attached to whoever holds or wears it. Carried for the
    /// rule runtime.
    #[serde(default)]
    pub rules: Rules,
    /// While worn, the share of exposure it keeps out, by route
    /// (02 §16). Carried until exposure reads it.
    #[serde(default)]
    pub guards: BTreeMap<String, Real>,
    /// A volume of substances it can hold as a mixture (16 §9.5).
    /// Carried until substances are built.
    #[serde(default)]
    pub contains: Option<Volume>,
    /// The colour an item of this kind carries.
    #[serde(default)]
    pub color: ColorChoice,
    /// How it is drawn.
    #[serde(default)]
    pub glyph: Option<GlyphSpec>,
    /// What a container of this kind is found to hold when it is first
    /// opened (06 §9.3).
    #[serde(default)]
    pub contents: Vec<ContentDef>,
    /// How it ages. Carried as data.
    #[serde(default)]
    pub ages: Option<AgesDef>,
    /// What it keeps for its contents. Carried as data.
    #[serde(default)]
    pub keeps: Option<KeepsDef>,
    /// The interactions it offers (16 §3.2), done to it as an entity.
    #[serde(default)]
    pub interactions: Vec<Ref<crate::interaction::InteractionDef>>,
    /// For ammunition (07 §11.5): the projectile it fires and with what
    /// energy.
    #[serde(default)]
    pub round: Option<crate::combat::ranged::RoundDef>,
    /// For what fires shots, or is fitted to what does (07 §11.5): the
    /// numbers a shot through it takes, and what may be fitted to it.
    #[serde(default)]
    pub shot: Option<crate::combat::ranged::ShotDef>,
}

impl ItemDef {
    /// Whether an item of this kind can be worn.
    pub fn is_wearable(&self) -> bool {
        !self.wear.0.is_empty()
    }

    /// Whether an item of this kind can be held in a grasp.
    pub fn is_graspable(&self) -> bool {
        !self.grip.is_empty()
    }

    /// Whether an item of this kind holds other things.
    pub fn is_container(&self) -> bool {
        self.holds.is_some()
    }
}

impl Def for ItemDef {
    type Id = ItemId;
    const TYPE: &'static str = "item";
    const DOC: &'static str = "A kind of item: what it is made of, how it is held and worn, what it holds and what it confers (06 §9.1).";

    fn compile(&mut self, cx: &mut CompileCx<'_>) {
        // A name is about nothing in particular: nothing is bound.
        let scope = Scope::new().pure();
        cx.compile(&mut self.name, &scope);
        cx.compile(&mut self.unknown_name, &scope);
        cx.compile(&mut self.description, &scope);
    }

    fn validate(&self, cx: &mut ValidateCx<'_>) {
        if !amount(self.mass.get()) {
            cx.error("`mass` must be a finite amount of zero or more");
        }
        if self.volume.is_some_and(|volume| !positive(volume.get())) {
            cx.error("`volume` must be above zero");
        }
        if !amount(self.length.get()) {
            cx.error("`length` must be a finite amount of zero or more");
        }
        if self.grip.values().any(|slots| *slots == 0) {
            cx.error("an item needs at least one slot of each channel its `grip` lists");
        }
        if self.wear.0.len() > MAX_WEAR_ENTRIES {
            cx.error(format!(
                "an item has at most {MAX_WEAR_ENTRIES} `wear` entries, and this has {}",
                self.wear.0.len()
            ));
        }
        for (index, entry) in self.wear.0.iter().enumerate() {
            if !positive(entry.thickness.get()) {
                cx.error(format!("wear {index}: `thickness` must be above zero"));
            }
            if !share(entry.coverage) {
                cx.error(format!("wear {index}: `coverage` is a share, from 0 to 1"));
            }
            if matches!(&entry.parts, PartSelector::Named(names) if names.is_empty()) {
                cx.error(format!(
                    "wear {index}: `parts` names no part: write \"all\", or at least one name"
                ));
            }
        }
        if let Some(holds) = &self.holds {
            if holds.volume.is_some_and(|volume| !amount(volume.get())) {
                cx.error("holds: `volume` must be a finite amount of zero or more");
            }
            if holds.mass.is_some_and(|mass| !amount(mass.get())) {
                cx.error("holds: `mass` must be a finite amount of zero or more");
            }
            if self.stack {
                cx.error(
                    "a container cannot `stack`: members of a stack are equal, contents included",
                );
            }
        }
        if !self.contents.is_empty() && self.holds.is_none() {
            cx.error("`contents` are what a container is found to hold: give the item `holds`");
        }
        for (index, entry) in self.contents.iter().enumerate() {
            if !share(entry.chance) {
                cx.error(format!("contents {index}: `chance` is from 0 to 1"));
            }
            if entry.count.min > entry.count.max {
                cx.error(format!(
                    "contents {index}: `count` is `[least, most]`, and {} is more than {}",
                    entry.count.min, entry.count.max
                ));
            }
        }
        for (index, modifier) in self.modifiers.iter().enumerate() {
            if !modifier.add.is_finite() || !modifier.mul.is_finite() {
                cx.error(format!("modifiers {index}: `add` and `mul` must be finite"));
            }
        }
        for (index, surface) in self.surfaces.iter().enumerate() {
            if !positive(surface.area) || !positive(surface.give.get()) {
                cx.error(format!(
                    "surface `{}`: `area` and `give` must be above zero",
                    surface.id
                ));
            }
            if !amount(surface.span.get()) {
                cx.error(format!(
                    "surface `{}`: `span` must not be negative",
                    surface.id
                ));
            }
            if self.surfaces[..index]
                .iter()
                .any(|other| other.id == surface.id)
            {
                cx.error(format!("the surface `{}` is listed twice", surface.id));
            }
        }
        for (route, kept) in &self.guards {
            if !share(*kept) {
                cx.error(format!(
                    "guards: the share kept out by `{route}` is from 0 to 1"
                ));
            }
        }
        if self.contains.is_some_and(|volume| !amount(volume.get())) {
            cx.error("`contains` must be a finite volume of zero or more");
        }
        if matches!(&self.color, ColorChoice::AnyOf(names) if names.is_empty()) {
            cx.error("`color` lists no colour: leave it out, or name at least one");
        }
        if let Some(ages) = &self.ages {
            if ages.life.dur() <= clash_core::Dur::ZERO {
                cx.error("ages: `life` must be above zero");
            }
            for (index, stage) in ages.stages.iter().enumerate() {
                if !share(stage.at) {
                    cx.error(format!("ages: stage {index}: `at` is an age, from 0 to 1"));
                }
            }
        }
        if let Some(keeps) = &self.keeps
            && keeps.ageing.is_some_and(|factor| !amount(factor))
        {
            cx.error("keeps: `ageing` must be a finite factor of zero or more");
        }
        if let Some(round) = &self.round {
            round.validate(cx);
        }
        if let Some(shot) = &self.shot {
            shot.validate(cx);
        }
    }
}
