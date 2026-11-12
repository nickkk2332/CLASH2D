//! The `template` definition type (06 §10.1, §10.2): what a kind of
//! thing is made of, and the choices made within it.

use std::collections::{BTreeMap, BTreeSet};

use clash_core::Real;
use clash_data::expr::Any;
use clash_data::{
    CompileCx, Def, Expr, FromKind, Kind, KindValue, Located, Ref, Scope, Text, ValidateCx, by_kind,
};
use serde::de::MapAccess;
use serde::{Deserialize, Deserializer};

use crate::body::{BodyPartDef, BodyPlanDef};
use crate::component::TemplateId;
use crate::rule::EocDef;
use crate::stats::{AttributeDef, PoolDef, TrackDef};

/// The most characters a `text` choice takes when it states no
/// `max_length` of its own.
pub const DEFAULT_TEXT_LENGTH: u32 = 64;

/// How deep choices may sit inside the options of other choices.
pub const MAX_CHOICE_DEPTH: usize = 8;

/// What a template or an option says about a mind (06 §8.1): `true` for
/// a mind with no behaviour profile, `false` for none, or the id of a
/// behaviour profile for the AI layer (09), kept as written.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum MindField {
    /// A mind, or with `false` none.
    Has(bool),
    /// A mind with this behaviour profile.
    Profile(String),
}

impl<'de> FromKind<'de> for MindField {
    fn from_kind<A: MapAccess<'de>>(kind: Kind, value: KindValue<'_, A>) -> Result<Self, A::Error> {
        match kind {
            Kind::Bool => value.read::<bool>().map(MindField::Has),
            _ => value.read::<String>().map(MindField::Profile),
        }
    }
}

impl<'de> Deserialize<'de> for MindField {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<MindField, D::Error> {
        by_kind(deserializer)
    }
}

/// How a thing made from a template is drawn (12 §4): one character and
/// its colours, as the presentation names them. The engine keeps them
/// and gives them no meaning.
#[derive(Clone, PartialEq, Eq, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GlyphDef {
    /// The character.
    pub ch: String,
    /// The colour of the character, by name.
    #[serde(default)]
    pub fg: Option<String>,
    /// The colour behind it, by name.
    #[serde(default)]
    pub bg: Option<String>,
}

/// The kinds of choice (06 §10.2).
#[derive(Clone, Copy, PartialEq, Eq, Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChoiceKind {
    /// Picks `count` of the listed options.
    Pick,
    /// Spends a `budget` on the listed options, each at its `cost`.
    Points,
    /// Sets a number between `min` and `max`.
    Range,
    /// Types something.
    Text,
    /// Adds parts from an allowed set, one point of the `budget` each.
    Assemble,
}

impl ChoiceKind {
    /// The word a definition writes.
    pub const fn name(self) -> &'static str {
        match self {
            ChoiceKind::Pick => "pick",
            ChoiceKind::Points => "points",
            ChoiceKind::Range => "range",
            ChoiceKind::Text => "text",
            ChoiceKind::Assemble => "assemble",
        }
    }
}

/// What a `range` or `text` choice's value does to the thing made.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Sets {
    /// For `range`: the body's size is multiplied by the value.
    Scale,
    /// For `text`: the value is the thing's own name.
    Name,
    /// For `text`: the value is the thing's own description.
    Description,
}

/// A choice's default: what is chosen when nobody chooses. Its shape
/// depends on the kind: option ids for `pick`, `points` and
/// `assemble`, a number for `range`, a text for `text`.
#[derive(Clone, PartialEq, Debug)]
pub enum ChoiceDefault {
    /// Option ids, or part ids.
    Ids(Vec<String>),
    /// A number.
    Number(Real),
    /// A text.
    Text(String),
}

impl<'de> FromKind<'de> for ChoiceDefault {
    fn from_kind<A: MapAccess<'de>>(kind: Kind, value: KindValue<'_, A>) -> Result<Self, A::Error> {
        match kind {
            Kind::List => value.read::<Vec<String>>().map(ChoiceDefault::Ids),
            Kind::Number => value
                .read::<f64>()
                .map(|x| ChoiceDefault::Number(Real::of(x))),
            _ => value.read::<String>().map(ChoiceDefault::Text),
        }
    }
}

impl<'de> Deserialize<'de> for ChoiceDefault {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<ChoiceDefault, D::Error> {
        by_kind(deserializer)
    }
}

/// Declares a definition struct with what a template or an option gives
/// (06 §10.1) as fields of its own, beside the struct's other fields,
/// and [`Gives`] to read them.
macro_rules! giving {
    (
        $(#[$meta:meta])*
        pub struct $name:ident {
            $( $(#[$fmeta:meta])* pub $field:ident : $ty:ty, )*
        }
    ) => {
        $(#[$meta])*
        pub struct $name {
            $( $(#[$fmeta])* pub $field: $ty, )*
            /// Bases of its own for attributes. A later contribution
            /// replaces an earlier one's.
            #[serde(default)]
            pub attributes: BTreeMap<Ref<AttributeDef>, Real>,
            /// Pools it is given, at their definitions' `start`.
            #[serde(default)]
            pub pools: Vec<Ref<PoolDef>>,
            /// What each track starts with: added to its counter.
            #[serde(default)]
            pub tracks: BTreeMap<Ref<TrackDef>, Real>,
            /// Tags of the entity.
            #[serde(default)]
            pub tags: Vec<String>,
            /// Variables of the entity and their values: constant
            /// expressions.
            #[serde(default)]
            pub variables: BTreeMap<String, Expr<Any>>,
            /// Rules attached to the entity (05 §9.3).
            #[serde(default)]
            pub rules: Vec<Ref<EocDef>>,
            /// Traits it starts with, by id, as written: given by
            /// whoever gives traits (06 §5), who is told of them by
            /// `entity_spawned`.
            #[serde(default)]
            pub traits: Vec<String>,
            /// Abilities it is given, by id, as written, with the
            /// template as their source (08 §3.3): it holds them until
            /// they are revoked.
            #[serde(default)]
            pub abilities: Vec<String>,
            /// What it starts with, by the id of an item or a content
            /// table, as written: made by the item package (06 §9),
            /// which is told of them by `entity_spawned`.
            #[serde(default)]
            pub items: Vec<String>,
            /// A factor on the body's size.
            #[serde(default)]
            pub scale: Option<Real>,
            /// A mind, none, or a behaviour profile.
            #[serde(default)]
            pub mind: Option<MindField>,
            /// Senses it is given (15 §4), by id as written: given
            /// when it is made, where perception's definitions are
            /// loaded.
            #[serde(default)]
            pub senses: Vec<String>,
        }

        impl $name {
            /// What it gives.
            pub fn gives(&self) -> Gives<'_> {
                Gives {
                    attributes: &self.attributes,
                    pools: &self.pools,
                    tracks: &self.tracks,
                    tags: &self.tags,
                    variables: &self.variables,
                    rules: &self.rules,
                    traits: &self.traits,
                    abilities: &self.abilities,
                    items: &self.items,
                    scale: self.scale,
                    mind: self.mind.as_ref(),
                    senses: &self.senses,
                }
            }

            fn compile_gives(&mut self, cx: &mut CompileCx<'_>) {
                let scope = Scope::new().pure();
                for value in self.variables.values_mut() {
                    cx.compile(value, &scope);
                }
            }
        }
    };
}

/// What a template, or an option of one of its choices, gives the thing
/// made (06 §10.1), as written. Everything is optional.
#[derive(Clone, Copy, Debug)]
pub struct Gives<'a> {
    /// Bases of its own for attributes.
    pub attributes: &'a BTreeMap<Ref<AttributeDef>, Real>,
    /// Pools it is given.
    pub pools: &'a [Ref<PoolDef>],
    /// What each track starts with.
    pub tracks: &'a BTreeMap<Ref<TrackDef>, Real>,
    /// Tags.
    pub tags: &'a [String],
    /// Variables and their values.
    pub variables: &'a BTreeMap<String, Expr<Any>>,
    /// Rules attached.
    pub rules: &'a [Ref<EocDef>],
    /// Traits it starts with, as written.
    pub traits: &'a [String],
    /// Abilities it is given, as written.
    pub abilities: &'a [String],
    /// Items it starts with, as written.
    pub items: &'a [String],
    /// A factor on the body's size.
    pub scale: Option<Real>,
    /// A mind, none, or a behaviour profile.
    pub mind: Option<&'a MindField>,
    /// Senses it is given, by id as written.
    pub senses: &'a [String],
}

impl Gives<'_> {
    fn validate(&self, cx: &mut ValidateCx<'_>) {
        // A sense is checked where senses are loaded at all.
        let senses: Vec<&str> = cx
            .defs()
            .iter::<crate::perception::SenseDef>()
            .filter_map(|(id, _)| cx.defs().name::<crate::perception::SenseDef>(id))
            .collect();
        for sense in self.senses {
            let known = senses.iter().any(|name| {
                *name == sense.as_str() || name.rsplit(':').next() == Some(sense.as_str())
            });
            if !senses.is_empty() && !known {
                cx.error(format!("there is no sense `{sense}`"));
            }
        }
        for base in self.attributes.values() {
            if !base.is_finite() {
                cx.error("a base given to an attribute must be a finite number");
            }
        }
        for amount in self.tracks.values() {
            if !amount.is_finite() || *amount < Real::ZERO {
                cx.error("what a track starts with must be a finite number of zero or more");
            }
        }
        if let Some(scale) = self.scale
            && !(scale.is_finite() && scale > Real::ZERO)
        {
            cx.error("`scale` must be a finite number above zero");
        }
        for (name, value) in self.variables {
            if value.is_compiled() && value.constant().is_none() {
                let message = format!("the value of the variable `{name}` must be a constant");
                match value.span() {
                    Some(span) => cx.error_at(span, message),
                    None => cx.error(message),
                }
            }
        }
        let mut pools = BTreeSet::new();
        for pool in self.pools {
            if !pools.insert(*pool) {
                cx.error("a pool is listed twice in `pools`");
            }
        }
    }
}

giving! {
    /// One option of a `pick` or `points` choice: whatever a template
    /// gives, and choices of its own.
    #[derive(Debug, Deserialize)]
    pub struct OptionDef {
        /// Its id within the choice.
        pub id: Located<String>,
        /// What a reader calls it.
        #[serde(default)]
        pub name: Option<Text>,
        /// What it costs from a `points` budget. Default 1.
        #[serde(default)]
        pub cost: Option<Real>,
        /// Choices that open once it is chosen.
        #[serde(default)]
        pub choices: Vec<ChoiceDef>,
    }
}

impl OptionDef {
    /// What it costs from a `points` budget.
    pub fn cost(&self) -> Real {
        self.cost.unwrap_or(Real::ONE)
    }
}

/// One choice of a template (06 §10.2).
#[derive(Debug, Deserialize)]
pub struct ChoiceDef {
    /// Its id within the template, or within the option it sits in.
    pub id: Located<String>,
    /// What a reader calls it.
    #[serde(default)]
    pub name: Option<Text>,
    /// What sort of choice it is.
    pub kind: ChoiceKind,
    /// For `pick`: how many options are picked. Default 1.
    #[serde(default)]
    pub count: Option<u32>,
    /// For `points` and `assemble`: what may be spent.
    #[serde(default)]
    pub budget: Option<Real>,
    /// For `pick` and `points`: the options.
    #[serde(default)]
    pub options: Vec<OptionDef>,
    /// For `range`: the least and the most the number may be.
    #[serde(default)]
    pub min: Option<Real>,
    /// For `range`: the most the number may be.
    #[serde(default)]
    pub max: Option<Real>,
    /// For `text`: the most characters it takes.
    #[serde(default)]
    pub max_length: Option<u32>,
    /// For `text`: what a random choice picks from.
    #[serde(default)]
    pub suggestions: Vec<String>,
    /// For `assemble`: the parts that may be added.
    #[serde(default)]
    pub parts: Vec<Ref<BodyPartDef>>,
    /// For `range` and `text`: what the value does.
    #[serde(default)]
    pub sets: Option<Sets>,
    /// For `range`: the attribute whose base the value becomes.
    #[serde(default)]
    pub attribute: Option<Ref<AttributeDef>>,
    /// What is chosen when nobody chooses.
    #[serde(default)]
    pub default: Option<ChoiceDefault>,
}

impl ChoiceDef {
    /// For `pick`: how many options are picked.
    pub fn count(&self) -> u32 {
        self.count.unwrap_or(1)
    }

    /// For `points` and `assemble`: what may be spent.
    pub fn budget(&self) -> Real {
        self.budget.unwrap_or(Real::ZERO)
    }

    /// For `text`: the most characters it takes.
    pub fn max_length(&self) -> u32 {
        self.max_length.unwrap_or(DEFAULT_TEXT_LENGTH)
    }

    /// The option with this id.
    pub fn option(&self, id: &str) -> Option<&OptionDef> {
        self.options.iter().find(|option| option.id.value == id)
    }

    fn compile(&mut self, cx: &mut CompileCx<'_>) {
        cx.compile(&mut self.name, &Scope::new().pure());
        for option in &mut self.options {
            cx.compile(&mut option.name, &Scope::new().pure());
            option.compile_gives(cx);
            for choice in &mut option.choices {
                choice.compile(cx);
            }
        }
    }

    fn validate(&self, cx: &mut ValidateCx<'_>, depth: usize) {
        let span = self.id.span;
        if depth > MAX_CHOICE_DEPTH {
            cx.error_at(
                span,
                format!("choices sit at most {MAX_CHOICE_DEPTH} deep inside options"),
            );
            return;
        }
        let kind = self.kind.name();
        let only = |field: &str, kinds: &[ChoiceKind]| {
            (!kinds.contains(&self.kind))
                .then(|| format!("`{field}` is not a field of a `{kind}` choice"))
        };
        let mut wrong = Vec::new();
        if self.count.is_some() {
            wrong.extend(only("count", &[ChoiceKind::Pick]));
        }
        if self.budget.is_some() {
            wrong.extend(only("budget", &[ChoiceKind::Points, ChoiceKind::Assemble]));
        }
        if !self.options.is_empty() {
            wrong.extend(only("options", &[ChoiceKind::Pick, ChoiceKind::Points]));
        }
        if self.min.is_some() || self.max.is_some() || self.attribute.is_some() {
            wrong.extend(only("min`, `max` or `attribute", &[ChoiceKind::Range]));
        }
        if self.max_length.is_some() || !self.suggestions.is_empty() {
            wrong.extend(only("max_length` or `suggestions", &[ChoiceKind::Text]));
        }
        if !self.parts.is_empty() {
            wrong.extend(only("parts", &[ChoiceKind::Assemble]));
        }
        for message in wrong {
            cx.error_at(span, message);
        }
        match (self.kind, self.sets) {
            (_, None) | (ChoiceKind::Range, Some(Sets::Scale)) => {}
            (ChoiceKind::Text, Some(Sets::Name | Sets::Description)) => {}
            (_, Some(_)) => cx.error_at(
                span,
                "`sets` is `scale` for a `range` choice and `name` or `description` for a `text` choice",
            ),
        }
        let mut ids = BTreeSet::new();
        for option in &self.options {
            if !ids.insert(option.id.value.as_str()) {
                cx.error_at(
                    option.id.span,
                    "an option's id is listed twice in its choice",
                );
            }
            if !(option.cost().is_finite() && option.cost() >= Real::ZERO) {
                cx.error_at(option.id.span, "an option's `cost` must be zero or more");
            }
            option.gives().validate(cx);
            check_choice_ids(cx, &option.choices);
            for choice in &option.choices {
                choice.validate(cx, depth + 1);
            }
        }
        match self.kind {
            ChoiceKind::Pick => {
                if self.count() as usize > self.options.len() {
                    cx.error_at(
                        span,
                        "a `pick` choice's `count` is more than it has options",
                    );
                }
            }
            ChoiceKind::Points | ChoiceKind::Assemble => {
                if !(self.budget().is_finite() && self.budget() >= Real::ZERO) {
                    cx.error_at(span, "`budget` must be a finite number of zero or more");
                }
                if self.kind == ChoiceKind::Assemble && self.parts.is_empty() {
                    cx.error_at(span, "an `assemble` choice lists the `parts` it allows");
                }
            }
            ChoiceKind::Range => match (self.min, self.max) {
                (Some(min), Some(max)) if min.is_finite() && max.is_finite() && min <= max => {}
                _ => cx.error_at(
                    span,
                    "a `range` choice needs a finite `min` and `max`, `min` no more than `max`",
                ),
            },
            ChoiceKind::Text => {}
        }
        match (self.kind, &self.default) {
            (_, None) => {}
            (ChoiceKind::Pick | ChoiceKind::Points, Some(ChoiceDefault::Ids(ids))) => {
                for id in ids {
                    if self.option(id).is_none() {
                        cx.error_at(span, format!("the default names `{id}`, which is no option"));
                    }
                }
            }
            (ChoiceKind::Assemble, Some(ChoiceDefault::Ids(_))) => {}
            (ChoiceKind::Range, Some(ChoiceDefault::Number(value))) => {
                if let (Some(min), Some(max)) = (self.min, self.max)
                    && !(*value >= min && *value <= max)
                {
                    cx.error_at(span, "the default is outside `min` and `max`");
                }
            }
            (ChoiceKind::Text, Some(ChoiceDefault::Text(_))) => {}
            (_, Some(_)) => cx.error_at(
                span,
                format!("the default of a `{kind}` choice is of the wrong shape: option ids for `pick`, `points` and `assemble`, a number for `range`, a text for `text`"),
            ),
        }
    }
}

/// Reports choice ids listed twice among siblings.
fn check_choice_ids(cx: &mut ValidateCx<'_>, choices: &[ChoiceDef]) {
    let mut ids = BTreeSet::new();
    for choice in choices {
        if !ids.insert(choice.id.value.as_str()) {
            cx.error_at(choice.id.span, "a choice's id is listed twice beside it");
        }
        if choice.id.value.contains('.') || choice.id.value.is_empty() {
            cx.error_at(choice.id.span, "a choice's id is not empty and has no `.`");
        }
    }
}

giving! {
    /// A template (06 §10.1): a bundle of body, stats, tags, variables,
    /// rules, starting traits and items, presentation, and choices.
    /// Species, backgrounds and classes are all templates; `kind` is a
    /// free-form label the engine gives no meaning.
    #[derive(Debug, Deserialize)]
    pub struct TemplateDef {
        /// A free-form label: `species`, `background`, whatever content
        /// calls it.
        #[serde(default)]
        pub kind: Option<String>,
        /// What a reader calls it.
        #[serde(default)]
        pub name: Option<Text>,
        /// The body plan a thing made from it is built from. A later
        /// template's replaces an earlier one's.
        #[serde(default)]
        pub body: Option<Ref<BodyPlanDef>>,
        /// How it is drawn. A later template's replaces an earlier one's.
        #[serde(default)]
        pub glyph: Option<GlyphDef>,
        /// What is left of a body destroyed entirely (06 §7.5), by id, as
        /// written: for whoever makes remains.
        #[serde(default)]
        pub remains: Option<String>,
        /// Whether a thing made from it blocks movement through its cell
        /// (06 §11). Default true for one with a body.
        #[serde(default)]
        pub blocks: Option<bool>,
        /// The choices made within it.
        #[serde(default)]
        pub choices: Vec<ChoiceDef>,
        /// The groups a thing made from it joins (15 §9.2), by id, each
        /// with its role there, `""` for none: joined openly, with
        /// `group_joined`, when `entity_spawned` is delivered.
        #[serde(default)]
        pub groups: BTreeMap<String, String>,
    }
}

impl TemplateDef {
    /// Every ability it or an option of its choices names, as written,
    /// for the check that each is defined.
    pub fn all_abilities(&self) -> Vec<&str> {
        fn of_choices<'a>(choices: &'a [ChoiceDef], out: &mut Vec<&'a str>) {
            for choice in choices {
                for option in &choice.options {
                    out.extend(option.abilities.iter().map(String::as_str));
                    of_choices(&option.choices, out);
                }
            }
        }
        let mut out: Vec<&str> = self.abilities.iter().map(String::as_str).collect();
        of_choices(&self.choices, &mut out);
        out
    }
}

impl Def for TemplateDef {
    type Id = TemplateId;
    const TYPE: &'static str = "template";
    const DOC: &'static str = "What a kind of thing is made of: a body, stats, tags, rules, starting traits and items, and choices (06 §10).";

    fn compile(&mut self, cx: &mut CompileCx<'_>) {
        cx.compile(&mut self.name, &Scope::new().pure());
        self.compile_gives(cx);
        for choice in &mut self.choices {
            choice.compile(cx);
        }
    }

    fn validate(&self, cx: &mut ValidateCx<'_>) {
        self.gives().validate(cx);
        if let Some(glyph) = &self.glyph
            && glyph.ch.chars().count() != 1
        {
            cx.error("a glyph's `ch` is exactly one character");
        }
        check_choice_ids(cx, &self.choices);
        for choice in &self.choices {
            choice.validate(cx, 0);
        }
    }
}
