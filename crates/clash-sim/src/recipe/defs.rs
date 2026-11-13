//! The `recipe` and `know_how` definition types (16 §8.1, §8.5).

use std::collections::BTreeMap;

use clash_core::{Real, define_id};
use clash_data::expr::Number;
use clash_data::{
    CompileCx, Cond, Def, Expr, FromKind, Kind, KindValue, Ref, Scope, Text, Time, ValidateCx,
    Volume, by_kind,
};
use serde::Deserialize;
use serde::de::{Deserializer, MapAccess};

use crate::item::ItemDef;
use crate::rule::Effects;
use crate::substance::SubstanceDef;

define_id! {
    /// A `recipe` definition (16 §8.1).
    pub struct RecipeId(u16);
}

define_id! {
    /// A `know_how` definition (16 §8.5).
    pub struct KnowHowId(u16);
}

/// What `at` is written as for a recipe done with no station.
pub const BY_HAND: &str = "hand";

/// The name the worker (or, for a machine working alone, the machine)
/// is bound under in a recipe's expressions.
pub const WORKER_SUBJECT: &str = "self";
/// The name the station is bound under: the device entity, or null by
/// hand.
pub const STATION_SUBJECT: &str = "station";
/// The name the place the work is done at is bound under.
pub const HERE_SUBJECT: &str = "here";

/// One thing a recipe uses up (16 §8.1 `inputs`): items by definition or
/// by tag, or an amount of a substance.
#[derive(Debug, Deserialize)]
pub struct InputDef {
    /// An item definition: so many of it.
    #[serde(default)]
    pub item: Option<Ref<ItemDef>>,
    /// Or any item carrying this tag.
    #[serde(default)]
    pub tag: Option<String>,
    /// Or an amount of a substance, drawn from containers (16 §9.5).
    #[serde(default)]
    pub substance: Option<Ref<SubstanceDef>>,
    /// How many items (default 1).
    #[serde(default = "one")]
    pub count: u32,
    /// How much of the substance.
    #[serde(default)]
    pub amount: Option<Volume>,
}

/// A tool a recipe needs and does not use up (16 §8.3).
#[derive(Debug, Deserialize)]
pub struct ToolDef {
    /// A quality items declare (`qualities`), at least `level`.
    #[serde(default)]
    pub quality: Option<String>,
    /// The least level of the quality (default 1).
    #[serde(default = "one_real")]
    pub level: Real,
    /// Or an item definition.
    #[serde(default)]
    pub item: Option<Ref<ItemDef>>,
    /// Or any item carrying this tag.
    #[serde(default)]
    pub tag: Option<String>,
}

/// One thing a recipe gives (16 §8.1 `outputs`).
#[derive(Debug, Deserialize)]
pub struct OutputDef {
    /// Items of a definition.
    #[serde(default)]
    pub item: Option<Ref<ItemDef>>,
    /// How many (default 1).
    #[serde(default = "one")]
    pub count: u32,
    /// Or an amount of a substance, put into containers at hand.
    #[serde(default)]
    pub substance: Option<Ref<SubstanceDef>>,
    /// How much of it.
    #[serde(default)]
    pub amount: Option<Volume>,
    /// The chance that it is given at all, from 0 to 1 (default 1).
    #[serde(default = "one_real")]
    pub chance: Real,
}

/// What a recipe needs while the work goes on (16 §8.1 `while`, §8.4):
/// rates drawn from the station's own stores, or a condition on the
/// place.
#[derive(Debug)]
pub enum WhileDef {
    /// For each resource, the quantity a second the work draws from the
    /// station's store of it.
    Rates(BTreeMap<String, Real>),
    /// A condition on the place: the work advances only while it holds.
    Place(Cond),
}

impl<'de> FromKind<'de> for WhileDef {
    fn from_kind<A: MapAccess<'de>>(
        kind: Kind,
        value: KindValue<'_, A>,
    ) -> Result<WhileDef, A::Error> {
        match kind {
            Kind::Object => value.read().map(WhileDef::Rates),
            Kind::String => value.read().map(WhileDef::Place),
            other => Err(other
                .unexpected("a map of resources to rates a second, or a condition on the place")),
        }
    }
}

impl<'de> Deserialize<'de> for WhileDef {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<WhileDef, D::Error> {
        by_kind(deserializer)
    }
}

/// What a recipe gives off while it is under way (16 §8.1 `emits`).
#[derive(Debug, Default, Deserialize)]
pub struct EmitsDef {
    /// Levels added to fields of the place, by field id, when work
    /// starts and at each step.
    #[serde(default)]
    pub field: BTreeMap<String, u8>,
    /// How loud it is: carried for perception.
    #[serde(default)]
    pub noise: Real,
}

/// What a recipe mends (16 §14 `repairs`): the most damaged item at hand
/// carrying the tag gets a share of its capacity back.
#[derive(Debug, Deserialize)]
pub struct RepairsDef {
    /// The tag what it mends carries.
    pub tag: String,
    /// The share of the item's capacity taken off its damage.
    pub share: Real,
}

/// A recipe (16 §8.1): what is used up, what is needed, how much work it
/// takes and what it gives. Crafting by hand, processing at a station,
/// cooking, brewing and growing are all recipes.
///
/// ```json
/// { "type": "recipe", "id": "bread", "name": "Bake bread",
///   "at": ["oven"],
///   "inputs": [ { "item": "flour" }, { "substance": "water", "amount": "200 ml" } ],
///   "work": "20 s",
///   "outputs": [ { "item": "loaf" } ] }
/// ```
#[derive(Debug, Deserialize)]
pub struct RecipeDef {
    /// What a menu calls it.
    #[serde(default)]
    pub name: Option<Text>,
    /// What a closer look says.
    #[serde(default)]
    pub description: Option<Text>,
    /// The tags of the station it is done at, any one of which will do;
    /// `hand` (the default) for none.
    #[serde(default = "by_hand")]
    pub at: Vec<String>,
    /// What it uses up.
    #[serde(default)]
    pub inputs: Vec<InputDef>,
    /// What must be at hand and is not used up.
    #[serde(default)]
    pub tools: Vec<ToolDef>,
    /// The know-how it needs: the worker knows each, or the station
    /// provides it.
    #[serde(default)]
    pub know_how: Vec<Ref<KnowHowDef>>,
    /// A condition on the worker, `self`, with the station as `station`
    /// and the place as `here`. Truth, tested when the work starts.
    #[serde(default)]
    pub needs: Cond,
    /// What a refusal for `needs` says.
    #[serde(default)]
    pub fail_text: Option<Text>,
    /// What the work needs while it goes on.
    #[serde(default, rename = "while")]
    pub during: Option<WhileDef>,
    /// How much work it takes: the time it takes at a rate of 1.
    #[serde(default)]
    pub work: Time,
    /// The worker's rate, an expression of `self` (skill, tools,
    /// injuries), multiplied by the station's speed. Left out, 1.
    #[serde(default)]
    pub rate: Option<Expr<Number>>,
    /// How often a running process looks again at its rate, its
    /// condition and what it gives off. Left out, the setting
    /// `recipe.step`.
    #[serde(default)]
    pub check_every: Option<Time>,
    /// What it gives.
    #[serde(default)]
    pub outputs: Vec<OutputDef>,
    /// What it mends.
    #[serde(default)]
    pub repairs: Option<RepairsDef>,
    /// Effects run when it completes: anything outputs cannot express,
    /// with `self` the worker (or the machine), `station` and `here`.
    #[serde(default)]
    pub result: Effects,
    /// What it gives off while under way.
    #[serde(default)]
    pub emits: Option<EmitsDef>,
    /// Whether a machine may run it unattended.
    #[serde(default)]
    pub auto: bool,
}

fn one() -> u32 {
    1
}

fn one_real() -> Real {
    Real::ONE
}

fn by_hand() -> Vec<String> {
    vec![BY_HAND.to_owned()]
}

impl RecipeDef {
    /// The scope of `needs`, `fail_text` and `result`: the worker as
    /// `self`, the station as `station`, the place as `here`. Truth.
    pub fn scope() -> Scope {
        Scope::new()
            .own(WORKER_SUBJECT, "entity")
            .subject(STATION_SUBJECT, "entity")
            .subject(HERE_SUBJECT, "loc")
    }

    /// Whether it is done with no station.
    pub fn by_hand(&self) -> bool {
        self.at.is_empty() || self.at.iter().any(|tag| tag == BY_HAND)
    }
}

impl Def for RecipeDef {
    type Id = RecipeId;
    const TYPE: &'static str = "recipe";
    const DOC: &'static str = "What is used up, what is needed, how much work it takes and what it gives: crafting by hand, processing at a station, cooking, brewing and growing (16 §8).";

    fn compile(&mut self, cx: &mut CompileCx<'_>) {
        let pure = Scope::new().pure();
        cx.compile(&mut self.name, &pure);
        cx.compile(&mut self.description, &pure);
        let truth = RecipeDef::scope();
        cx.compile(&mut self.needs, &truth);
        cx.compile(&mut self.fail_text, &truth.clone().pure());
        cx.compile(&mut self.result, &truth);
        if let Some(rate) = &mut self.rate {
            cx.compile(rate, &truth.clone().pure());
        }
        if let Some(WhileDef::Place(condition)) = &mut self.during {
            cx.compile(condition, &truth.pure());
        }
    }

    fn validate(&self, cx: &mut ValidateCx<'_>) {
        if self.work.0.as_micros() < 0 {
            cx.error("`work` is a duration of zero or more");
        }
        for (index, input) in self.inputs.iter().enumerate() {
            let kinds = [
                input.item.is_some(),
                input.tag.is_some(),
                input.substance.is_some(),
            ];
            if kinds.iter().filter(|set| **set).count() != 1 {
                cx.error(format!(
                    "inputs {index}: an input names one of `item`, `tag` or `substance`"
                ));
            }
            if input.substance.is_some()
                && !input
                    .amount
                    .is_some_and(|amount| amount.get().is_finite() && amount.get() > Real::ZERO)
            {
                cx.error(format!(
                    "inputs {index}: a substance is used up by an `amount` above zero"
                ));
            }
            if input.substance.is_none() && input.count == 0 {
                cx.error(format!("inputs {index}: `count` is one or more"));
            }
        }
        for (index, tool) in self.tools.iter().enumerate() {
            let kinds = [
                tool.quality.is_some(),
                tool.item.is_some(),
                tool.tag.is_some(),
            ];
            if kinds.iter().filter(|set| **set).count() != 1 {
                cx.error(format!(
                    "tools {index}: a tool names one of `quality`, `item` or `tag`"
                ));
            }
        }
        for (index, output) in self.outputs.iter().enumerate() {
            if output.item.is_some() == output.substance.is_some() {
                cx.error(format!(
                    "outputs {index}: an output names one of `item` or `substance`"
                ));
            }
            if !(output.chance.is_finite()
                && output.chance >= Real::ZERO
                && output.chance <= Real::ONE)
            {
                cx.error(format!("outputs {index}: `chance` is from 0 to 1"));
            }
            if output.substance.is_some()
                && !output
                    .amount
                    .is_some_and(|amount| amount.get().is_finite() && amount.get() > Real::ZERO)
            {
                cx.error(format!(
                    "outputs {index}: a substance is given by an `amount` above zero"
                ));
            }
        }
        if let Some(WhileDef::Rates(rates)) = &self.during {
            for (resource, rate) in rates {
                if !(rate.is_finite() && *rate >= Real::ZERO) {
                    cx.error(format!(
                        "while.{resource}: a rate is a finite quantity a second, zero or more"
                    ));
                }
            }
        }
        if let Some(repairs) = &self.repairs
            && !(repairs.share.is_finite() && repairs.share > Real::ZERO)
        {
            cx.error("`repairs.share` is above zero");
        }
        if self
            .check_every
            .is_some_and(|every| every.0.as_micros() <= 0)
        {
            cx.error("`check_every` is a duration above zero");
        }
        self.result.validate(cx);
    }
}

/// Something someone may know how to do (16 §8.5): a set of named
/// things held by a mind, asked for by recipes.
///
/// ```json
/// { "type": "know_how", "id": "smelting", "name": "Smelting",
///   "needs": ["fire_making"], "group": "metalwork" }
/// ```
#[derive(Debug, Deserialize)]
pub struct KnowHowDef {
    /// What a reader calls it.
    #[serde(default)]
    pub name: Option<Text>,
    /// What a closer look says.
    #[serde(default)]
    pub description: Option<Text>,
    /// What must be known before it can be taught.
    #[serde(default)]
    pub needs: Vec<Ref<KnowHowDef>>,
    /// A free-form heading it is listed under.
    #[serde(default)]
    pub group: Option<String>,
}

impl Def for KnowHowDef {
    type Id = KnowHowId;
    const TYPE: &'static str = "know_how";
    const DOC: &'static str = "Something someone may know how to do, held by a mind and asked for by recipes; what must be known first (16 §8.5).";

    fn compile(&mut self, cx: &mut CompileCx<'_>) {
        let pure = Scope::new().pure();
        cx.compile(&mut self.name, &pure);
        cx.compile(&mut self.description, &pure);
    }
}
