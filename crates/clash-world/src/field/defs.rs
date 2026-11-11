//! The `field` and `field_reaction` definition types (02 §16): as mods
//! write them ([`FieldDef`], [`FieldReactionDef`]) and resolved
//! ([`Field`], [`FieldReaction`]), built with the rest of the world's
//! definitions by [`WorldDefs::build`](crate::WorldDefs::build).
//!
//! The engine knows what a field is and what each rule does. It never
//! knows what a particular field is: every behaviour comes from the
//! definition.

use core::fmt;
use std::collections::BTreeMap;

use clash_core::{Dur, Real, StateHash, StateHasher, define_id};
use clash_data::{Def, Time};
use serde::de::value::{MapAccessDeserializer, SeqAccessDeserializer};
use serde::de::{Deserializer, MapAccess, SeqAccess, Visitor};
use serde::{Deserialize, Serialize};

use crate::chunk::FieldId;
use crate::defs::{
    DamageType, DamageTypeId, DefRef, Energy, Length, Material, MaterialId, Pressure, Table,
};
use crate::grid::CellView;
use crate::light::{LightSource, LightSpec};

define_id! {
    /// A `field_reaction` definition (02 §16).
    pub struct FieldReactionId(u16);
}

/// The highest level a field can have in a cell.
pub const FIELD_MAX_LEVEL: u8 = u8::MAX;

/// The shortest step a field may have.
pub const FIELD_MIN_STEP: Dur = Dur::MILLISECOND;

/// The tag a material flagged `flammable` counts as carrying, for a
/// rule or a reaction that asks for a material tag.
pub const FLAMMABLE_TAG: &str = "flammable";

/// The largest share of a difference in level that `diffuse` or `flow`
/// may move to one neighbour in a step: with four neighbours beside a
/// cell, a larger share could move more than there is.
pub const FIELD_MAX_RATE: Real = Real::of(0.25);

/// The share of a difference in level that `flow` moves sideways to a
/// neighbour in a step, when the rule does not say.
pub const FLOW_DEFAULT_RATE: Real = Real::of(0.25);

/// The rules a field may have (02 §16). A fixed set: a mod configures
/// and combines them and cannot add one (02 §23).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FieldRuleKind {
    /// Lose level each step.
    Decay,
    /// Even out with neighbouring cells.
    Diffuse,
    /// Fall, then spread sideways.
    Flow,
    /// Appear in neighbouring cells, where they hold what it needs.
    Spread,
    /// Add to another field in the same cell.
    Emit,
    /// React with another field: shorthand for a `field_reaction`.
    Convert,
    /// Send packets of damage to the elements of the cell, and tell its
    /// occupants.
    Damage,
    /// Be carried by the movement of the medium.
    Drift,
    /// Tell the occupants of the cell of a dose for one of their pools.
    Expose,
}

impl FieldRuleKind {
    /// The rule's name in data.
    pub const fn name(self) -> &'static str {
        match self {
            FieldRuleKind::Decay => "decay",
            FieldRuleKind::Diffuse => "diffuse",
            FieldRuleKind::Flow => "flow",
            FieldRuleKind::Spread => "spread",
            FieldRuleKind::Emit => "emit",
            FieldRuleKind::Convert => "convert",
            FieldRuleKind::Damage => "damage",
            FieldRuleKind::Drift => "drift",
            FieldRuleKind::Expose => "expose",
        }
    }

    /// The parameters the rule takes, by their keys.
    pub const fn params(self) -> &'static [&'static str] {
        match self {
            FieldRuleKind::Decay => &["amount"],
            FieldRuleKind::Diffuse => &["rate", "rise", "sink"],
            FieldRuleKind::Flow => &["rate"],
            FieldRuleKind::Spread => &["needs", "chance", "amount", "from"],
            FieldRuleKind::Emit => &["field", "amount", "from"],
            FieldRuleKind::Convert => &["with", "uses", "gives", "max"],
            FieldRuleKind::Damage => &["kind", "energy_per_level", "intensity_per_level", "from"],
            FieldRuleKind::Drift => &["share", "spread"],
            FieldRuleKind::Expose => &["pool", "per_level", "route", "from"],
        }
    }
}

/// How an `expose` rule's dose reaches a body (02 §16). The world
/// reports the route; whether the dose gets through is the
/// simulation's.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExposureRoute {
    /// Those who breathe the cell's medium.
    Breath,
    /// The share of the body no worn layer guards.
    Skin,
    /// Everyone in the cell.
    Any,
}

impl ExposureRoute {
    /// The route's name in data.
    pub const fn name(self) -> &'static str {
        match self {
            ExposureRoute::Breath => "breath",
            ExposureRoute::Skin => "skin",
            ExposureRoute::Any => "any",
        }
    }
}

impl StateHash for ExposureRoute {
    fn state_hash(&self, hasher: &mut StateHasher) {
        hasher.write_u8(*self as u8);
    }
}

/// What becomes of a field in a tract that is loaded after time has
/// passed (02 §16).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CatchUp {
    /// It is as it was.
    #[default]
    Keep,
    /// It does not outlast unloading: it is gone.
    None,
    /// It is gone if as much as one of its steps has passed.
    BurnOut,
    /// It loses its decay for every step that passed, and half of what
    /// is left for every [`DISPERSE_HALF_STEPS`](super::DISPERSE_HALF_STEPS)
    /// steps.
    Disperse,
    /// It is as it was, and every cell of it is awake: it goes on to
    /// find its level.
    Settle,
}

/// One rule of a field as a mod writes it: the rule's name and its
/// parameters. Which parameters a rule takes is
/// [`FieldRuleKind::params`]; one that the rule does not take is a
/// load error.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FieldRuleSpec {
    /// Which rule.
    pub rule: FieldRuleKind,
    /// `decay`: levels lost each step. `emit`: levels added to the other
    /// field each step. `spread`: the level a cell it spreads to gets.
    #[serde(default)]
    pub amount: Option<u8>,
    /// `diffuse`, `flow`: the share of the difference in level with a
    /// neighbour that moves to it in a step.
    #[serde(default)]
    pub rate: Option<Real>,
    /// `diffuse`: the share of the level that moves to the cell above
    /// each step. Nothing then diffuses downward.
    #[serde(default)]
    pub rise: Option<Real>,
    /// `diffuse`: the share of the level that moves to the cell below
    /// each step. Nothing then diffuses upward.
    #[serde(default)]
    pub sink: Option<Real>,
    /// `spread`: the material tag a cell must hold an element of to be
    /// spread to.
    #[serde(default)]
    pub needs: Option<String>,
    /// `spread`: the chance, each step, of spreading to a neighbour.
    #[serde(default)]
    pub chance: Option<Real>,
    /// `spread`, `emit`, `damage`, `expose`: the least level at which
    /// the rule acts. Default 1.
    #[serde(default)]
    pub from: Option<u8>,
    /// `emit`: the field added to.
    #[serde(default)]
    pub field: Option<DefRef>,
    /// `convert`: the field reacted with.
    #[serde(default)]
    pub with: Option<DefRef>,
    /// `convert`: what one reaction uses of each of the two fields.
    #[serde(default)]
    pub uses: BTreeMap<String, u8>,
    /// `convert`: what one reaction gives, by field.
    #[serde(default)]
    pub gives: BTreeMap<String, u8>,
    /// `convert`: the most reactions in a cell in one step.
    #[serde(default)]
    pub max: Option<u32>,
    /// `damage`: the damage type.
    #[serde(default)]
    pub kind: Option<DefRef>,
    /// `damage`: joules delivered each step for each level.
    #[serde(default)]
    pub energy_per_level: Option<Energy>,
    /// `damage`: pascals of intensity for each level.
    #[serde(default)]
    pub intensity_per_level: Option<Pressure>,
    /// `drift`: the share of the medium's speed the field is carried at.
    /// Default 1.
    #[serde(default)]
    pub share: Option<Real>,
    /// `drift`: how much likelier `spread` is for each metre a second
    /// the medium moves toward a neighbour, and less likely against.
    /// Default 0.
    #[serde(default)]
    pub spread: Option<Real>,
    /// `expose`: the pool added to (06 §4.2), as written.
    #[serde(default)]
    pub pool: Option<String>,
    /// `expose`: the dose each step for each level.
    #[serde(default)]
    pub per_level: Option<Real>,
    /// `expose`: the route the dose takes. Default `any`.
    #[serde(default)]
    pub route: Option<ExposureRoute>,
}

/// The light a field gives off, as mods write it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FieldLightSpec {
    /// How bright a cell at the highest level is in its own cell, on
    /// the scale of 15 §3.3. A lower level gives in proportion.
    pub intensity: Real,
    /// How far it shines. Left out, as far as the light of the highest
    /// level matters.
    #[serde(default)]
    pub radius: Option<Length>,
    /// The least level that gives light. Default 1.
    #[serde(default)]
    pub from: Option<u8>,
}

/// One colour, or several from the lowest level to the highest.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ColorList(pub Vec<String>);

// Read by the kind of value written, as `EdgeSpec` is (04 §7.1).
impl<'de> Deserialize<'de> for ColorList {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<ColorList, D::Error> {
        struct Colors;

        impl<'de> Visitor<'de> for Colors {
            type Value = ColorList;

            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str("a colour name, or a list of them from the lowest level to the highest")
            }

            fn visit_str<E: serde::de::Error>(self, name: &str) -> Result<ColorList, E> {
                Ok(ColorList(vec![name.to_owned()]))
            }

            fn visit_seq<A: SeqAccess<'de>>(self, seq: A) -> Result<ColorList, A::Error> {
                Vec::<String>::deserialize(SeqAccessDeserializer::new(seq)).map(ColorList)
            }
        }

        deserializer.deserialize_any(Colors)
    }
}

/// How a field is drawn. A presentation hint: the world stores it and
/// gives it no meaning.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FieldGlyphSpec {
    /// The character.
    #[serde(default)]
    pub ch: Option<String>,
    /// The foreground colour, or colours by level.
    #[serde(default)]
    pub fg: Option<ColorList>,
    /// The background colour, or colours by level.
    #[serde(default)]
    pub bg: Option<ColorList>,
}

/// A `field` definition (02 §16): a per-cell value from 0 to 255 that
/// evolves by rule.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FieldDef {
    /// How often the field is stepped, in the local time of where it is.
    pub step: Time,
    /// Its rules, applied to each active cell each step, in order.
    #[serde(default)]
    pub rules: Vec<FieldRuleSpec>,
    /// What becomes of it in a tract loaded after time has passed.
    /// Default `keep`.
    #[serde(default)]
    pub catch_up: Option<CatchUp>,
    /// The kelvin each level adds to the temperature of its cell
    /// (11 §4.6). Stored and reported only, until temperature is built.
    #[serde(default)]
    pub warms: Option<Real>,
    /// What it becomes by temperature, like a material's (02 §5.4).
    /// Carried as written until change of state is built.
    #[serde(default)]
    pub changes: Vec<serde_json::Value>,
    /// The light it gives off (15 §3.3).
    #[serde(default)]
    pub light: Option<FieldLightSpec>,
    /// The level from which it hides what is behind it: a cell holding
    /// that much of it stops sight.
    #[serde(default)]
    pub opaque_from: Option<u8>,
    /// How it is drawn.
    #[serde(default)]
    pub glyph: Option<FieldGlyphSpec>,
    /// Free-form tags.
    #[serde(default)]
    pub tags: Vec<String>,
}

impl Def for FieldDef {
    type Id = FieldId;
    const TYPE: &'static str = "field";
    const DOC: &'static str =
        "A per-cell value from 0 to 255 that evolves by rule: fire, smoke, water (02 §16).";
}

/// What a reaction is between, as mods write it: a field by its id, or
/// `{ "material_tag": … }`, or `{ "damage": … }`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReactantSpec {
    /// A field, by id.
    Field(DefRef),
    /// Any element of a material that carries this tag.
    MaterialTag(String),
    /// A packet of this damage type arriving.
    Damage(DefRef),
}

// Read by the kind of value written, as `EdgeSpec` is (04 §7.1).
impl<'de> Deserialize<'de> for ReactantSpec {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<ReactantSpec, D::Error> {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Other {
            #[serde(default)]
            material_tag: Option<String>,
            #[serde(default)]
            damage: Option<DefRef>,
        }

        struct Reactant;

        impl<'de> Visitor<'de> for Reactant {
            type Value = ReactantSpec;

            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str("the id of a field, `{ \"material_tag\": … }` or `{ \"damage\": … }`")
            }

            fn visit_str<E: serde::de::Error>(self, id: &str) -> Result<ReactantSpec, E> {
                Ok(ReactantSpec::Field(DefRef(id.to_owned())))
            }

            fn visit_map<A: MapAccess<'de>>(self, map: A) -> Result<ReactantSpec, A::Error> {
                let other = Other::deserialize(MapAccessDeserializer::new(map))?;
                match (other.material_tag, other.damage) {
                    (Some(tag), None) => Ok(ReactantSpec::MaterialTag(tag)),
                    (None, Some(kind)) => Ok(ReactantSpec::Damage(kind)),
                    _ => Err(serde::de::Error::custom(
                        "give exactly one of `material_tag` and `damage`",
                    )),
                }
            }
        }

        deserializer.deserialize_any(Reactant)
    }
}

/// How a reaction to damage passes the packet on, as mods write it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SpreadsSpec {
    /// The share of the packet each connected cell gets, 0 to 1.
    pub share: Real,
    /// How many cells from where the packet arrived it reaches.
    pub reach: u32,
}

/// A `field_reaction` definition (02 §16): what two fields do to each
/// other, what a field does where it meets a tagged material, or what
/// a field does with damage that arrives in it.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FieldReactionDef {
    /// The two things that react.
    pub when: Vec<ReactantSpec>,
    /// What one reaction uses of each field that takes part.
    #[serde(default)]
    pub uses: BTreeMap<String, u8>,
    /// What one reaction gives, by field.
    #[serde(default)]
    pub gives: BTreeMap<String, u8>,
    /// The most reactions in a cell in one step. Default: as many as
    /// what is there allows.
    #[serde(default)]
    pub max: Option<u32>,
    /// For a reaction to damage: how the packet is passed on.
    #[serde(default)]
    pub spreads: Option<SpreadsSpec>,
}

impl Def for FieldReactionDef {
    type Id = FieldReactionId;
    const TYPE: &'static str = "field_reaction";
    const DOC: &'static str = "What two fields do to each other, what a field does where it meets a tagged material, or how a field passes on damage that arrives in it (02 §16).";
}

/// The materials that carry a tag: what a rule or reaction that names
/// a material tag tests.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct MaterialTagSet {
    tag: String,
    has: Vec<bool>,
}

impl MaterialTagSet {
    fn build(tag: &str, materials: &Table<MaterialId, Material>) -> MaterialTagSet {
        let has = materials
            .iter()
            .map(|(_, material)| {
                material
                    .tags
                    .binary_search_by(|have| have.as_str().cmp(tag))
                    .is_ok()
                    || (material.flammable && tag == FLAMMABLE_TAG)
            })
            .collect();
        MaterialTagSet {
            tag: tag.to_owned(),
            has,
        }
    }

    /// The tag.
    pub fn tag(&self) -> &str {
        &self.tag
    }

    /// Whether a material carries the tag.
    #[inline]
    pub fn has(&self, material: MaterialId) -> bool {
        self.has.get(material.index()).copied().unwrap_or(false)
    }

    /// Whether a cell holds a floor, block or fixture of a material
    /// that carries the tag.
    pub fn in_cell(&self, cell: &CellView) -> bool {
        (!cell.floor.is_none() && self.has(cell.floor.material()))
            || (!cell.block.is_none() && self.has(cell.block.material()))
            || (!cell.fixture.is_none() && self.has(cell.fixture.material()))
    }
}

/// Which way a `diffuse` rule is biased between storeys.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum VerticalBias {
    /// None: it evens out with the cells above and below as with those
    /// beside.
    Even,
    /// This share of the level moves to the cell above each step.
    Rise(Real),
    /// This share of the level moves to the cell below each step.
    Sink(Real),
}

/// One resolved rule of a field (02 §16).
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum FieldRule {
    /// Lose level each step.
    Decay {
        /// Levels lost each step.
        amount: u8,
    },
    /// Even out with neighbouring cells.
    Diffuse {
        /// The share of the difference in level with a neighbour that
        /// moves to it each step.
        rate: Real,
        /// What it does between storeys.
        vertical: VerticalBias,
    },
    /// Fall, then spread sideways.
    Flow {
        /// The share of the difference in level with a neighbour beside
        /// it that moves to it each step.
        rate: Real,
    },
    /// Appear in neighbouring cells.
    Spread {
        /// The materials a cell must hold an element of to be spread
        /// to. `None`: any cell the field can pass into.
        needs: Option<MaterialTagSet>,
        /// The chance, each step, of spreading to a neighbour.
        chance: Real,
        /// The level the neighbour gets.
        amount: u8,
        /// The least level that spreads.
        from: u8,
    },
    /// Add to another field in the same cell.
    Emit {
        /// The field added to.
        field: FieldId,
        /// Levels added each step.
        amount: u8,
        /// The least level that emits.
        from: u8,
    },
    /// React with another field. The reaction itself is among the
    /// field's [`reacts`](Field::reacts).
    Convert {
        /// The field reacted with.
        with: FieldId,
    },
    /// Send packets of damage to the elements of the cell, and tell
    /// its occupants.
    Damage {
        /// The damage type.
        kind: DamageTypeId,
        /// Joules delivered each step for each level.
        energy_per_level: Real,
        /// Pascals of intensity for each level.
        intensity_per_level: Real,
        /// The least level that does damage.
        from: u8,
    },
    /// Be carried by the movement of the medium.
    Drift {
        /// The share of the medium's speed the field is carried at.
        share: Real,
        /// How much likelier `spread` is for each metre a second the
        /// medium moves toward a neighbour.
        spread: Real,
    },
    /// Tell the occupants of the cell of a dose for one of their pools.
    Expose {
        /// The pool, as written.
        pool: String,
        /// The dose each step for each level.
        per_level: Real,
        /// The route the dose takes.
        route: ExposureRoute,
        /// The least level that exposes.
        from: u8,
    },
}

impl FieldRule {
    /// Which rule this is.
    pub const fn kind(&self) -> FieldRuleKind {
        match self {
            FieldRule::Decay { .. } => FieldRuleKind::Decay,
            FieldRule::Diffuse { .. } => FieldRuleKind::Diffuse,
            FieldRule::Flow { .. } => FieldRuleKind::Flow,
            FieldRule::Spread { .. } => FieldRuleKind::Spread,
            FieldRule::Emit { .. } => FieldRuleKind::Emit,
            FieldRule::Convert { .. } => FieldRuleKind::Convert,
            FieldRule::Damage { .. } => FieldRuleKind::Damage,
            FieldRule::Drift { .. } => FieldRuleKind::Drift,
            FieldRule::Expose { .. } => FieldRuleKind::Expose,
        }
    }
}

/// The light a field gives off, resolved.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct FieldLight {
    /// The source a cell at the highest level is.
    pub full: LightSource,
    /// The least level that gives light.
    pub from: u8,
}

impl FieldLight {
    /// The source a cell at a level is: the full intensity in
    /// proportion to the level, at the full radius. `None` below the
    /// least level that gives light.
    pub fn at_level(&self, level: u8) -> Option<LightSource> {
        if level < self.from || level == 0 {
            return None;
        }
        let share = Real::from_i64(i64::from(level)) / Real::from_i64(i64::from(FIELD_MAX_LEVEL));
        Some(LightSource {
            intensity: self.full.intensity * share,
            radius: self.full.radius,
        })
    }
}

/// How a field is drawn, as written: a presentation hint.
#[derive(Clone, Default, PartialEq, Eq, Debug)]
pub struct FieldGlyph {
    /// The character.
    pub ch: Option<String>,
    /// The foreground colours, from the lowest level to the highest.
    pub fg: Vec<String>,
    /// The background colours, from the lowest level to the highest.
    pub bg: Vec<String>,
}

impl FieldGlyph {
    /// The colour of a list that a level takes: the list is spread
    /// evenly over the levels from 1 to 255.
    pub fn color_at(colors: &[String], level: u8) -> Option<&str> {
        let count = colors.len();
        if count == 0 {
            return None;
        }
        let at = usize::from(level.saturating_sub(1)) * count / usize::from(FIELD_MAX_LEVEL);
        colors.get(at.min(count - 1)).map(String::as_str)
    }
}

/// What a field does with something else in its cell each step: the
/// reactions it takes part in, compiled for the stepper.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum FieldReacts {
    /// With another field in the same cell.
    With {
        /// The other field.
        other: FieldId,
        /// Whether this field is the one the reaction names first. The
        /// reaction runs at the steps of that field where it is awake,
        /// and of the other where it is not.
        first: bool,
        /// What one reaction uses of this field.
        use_self: u8,
        /// What one reaction uses of the other.
        use_other: u8,
        /// What one reaction gives, by field.
        gives: Vec<(FieldId, u8)>,
        /// The most reactions in a cell in one step.
        max: u32,
    },
    /// With an element of a tagged material in the same cell.
    Material {
        /// The materials that carry the tag.
        set: MaterialTagSet,
        /// What one reaction uses of this field.
        use_self: u8,
        /// What one reaction gives, by field.
        gives: Vec<(FieldId, u8)>,
        /// The most reactions in a cell in one step.
        max: u32,
    },
}

/// How a field passes on a packet of damage that arrives in it.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct FieldConducts {
    /// The damage type it passes on.
    pub kind: DamageTypeId,
    /// The share of the packet each connected cell gets.
    pub share: Real,
    /// How many cells from where the packet arrived it reaches.
    pub reach: u32,
}

/// A resolved `field` definition (02 §16).
#[derive(Clone, Debug)]
pub struct Field {
    /// How often it is stepped, in local time. More than nothing.
    pub step: Dur,
    /// Its rules, in the order written.
    pub rules: Vec<FieldRule>,
    /// The reactions it takes part in each step: those of the
    /// `field_reaction` definitions in id order, then those of `convert`
    /// rules in field and rule order.
    pub reacts: Vec<FieldReacts>,
    /// How it passes on damage that arrives in it.
    pub conducts: Vec<FieldConducts>,
    /// What becomes of it in a tract loaded after time has passed.
    pub catch_up: CatchUp,
    /// The kelvin each level adds to the temperature of its cell.
    pub warms: Real,
    /// What it becomes by temperature, as written.
    pub changes: Vec<serde_json::Value>,
    /// The light it gives off.
    pub light: Option<FieldLight>,
    /// The level from which a cell of it stops sight.
    pub opaque_from: Option<u8>,
    /// How it is drawn, as written.
    pub glyph: Option<FieldGlyph>,
    /// Free-form tags, sorted.
    pub tags: Vec<String>,
    /// Whether anything ever steps it: it has a rule or takes part in
    /// a reaction. A cell of a field that does not is never awake.
    pub live: bool,
    /// Whether a cell of it goes to sleep when a step leaves it
    /// untouched: its rules only move it (`flow`, `diffuse`) and it
    /// reacts with no material. Such a cell wakes when a level or an
    /// element in or beside it changes.
    pub rests: bool,
}

impl Field {
    /// Whether the definition carries a tag.
    pub fn has_tag(&self, tag: &str) -> bool {
        self.tags
            .binary_search_by(|have| have.as_str().cmp(tag))
            .is_ok()
    }

    /// The levels a `decay` rule takes each step, all of them together.
    pub fn decay(&self) -> u32 {
        self.rules
            .iter()
            .map(|rule| match rule {
                FieldRule::Decay { amount } => u32::from(*amount),
                _ => 0,
            })
            .sum()
    }

    /// Whether the field has a rule of a kind.
    pub fn has_rule(&self, kind: FieldRuleKind) -> bool {
        self.rules.iter().any(|rule| rule.kind() == kind)
    }
}

/// What a resolved reaction is between.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Reactant {
    /// A field.
    Field(FieldId),
    /// Any element of a material that carries a tag.
    MaterialTag(MaterialTagSet),
    /// A packet of a damage type arriving.
    Damage(DamageTypeId),
}

/// A resolved `field_reaction` definition (02 §16).
#[derive(Clone, Debug)]
pub struct FieldReaction {
    /// The two things that react, in the order written.
    pub when: [Reactant; 2],
    /// What one reaction uses, by field in id order.
    pub uses: Vec<(FieldId, u8)>,
    /// What one reaction gives, by field in id order.
    pub gives: Vec<(FieldId, u8)>,
    /// The most reactions in a cell in one step.
    pub max: u32,
    /// For a reaction to damage: the share of the packet each connected
    /// cell gets and how far it reaches.
    pub spreads: Option<(Real, u32)>,
}

/// Where a mistake goes: the definition type, the definition's id, a
/// dotted path inside it and what is wrong.
pub(crate) type ErrorSink<'a> = &'a mut dyn FnMut(&'static str, &str, &str, String);

const FIELD: &str = "field";
const REACTION: &str = "field_reaction";

/// Sorts definitions by id and drops, with an error, one whose id is
/// not `namespace:name` or is given twice.
fn sorted<D>(
    kind: &'static str,
    mut defs: Vec<(String, D)>,
    error: ErrorSink<'_>,
) -> Vec<(String, D)> {
    defs.sort_by(|a, b| a.0.cmp(&b.0));
    let mut kept: Vec<(String, D)> = Vec::with_capacity(defs.len());
    for (id, def) in defs {
        let qualified = id
            .split_once(':')
            .is_some_and(|(namespace, name)| !namespace.is_empty() && !name.is_empty());
        if !qualified {
            error(kind, &id, "id", "an id is `namespace:name`".to_owned());
        } else if id.starts_with("engine:") {
            error(
                kind,
                &id,
                "id",
                "the namespace `engine` is reserved".to_owned(),
            );
        } else if kept.last().is_some_and(|(previous, _)| *previous == id) {
            error(
                kind,
                &id,
                "id",
                "this id is defined more than once".to_owned(),
            );
        } else {
            kept.push((id, def));
        }
    }
    kept
}

fn namespace_of(id: &str) -> &str {
    id.split_once(':').map_or("", |(namespace, _)| namespace)
}

fn sorted_tags(given: &[String]) -> Vec<String> {
    let mut tags = given.to_vec();
    tags.sort();
    tags.dedup();
    tags
}

/// What building needs to look things up in, and where it reports.
struct Build<'a, 'e> {
    names: &'a Table<FieldId, ()>,
    damage_types: &'a Table<DamageTypeId, DamageType>,
    materials: &'a Table<MaterialId, Material>,
    error: ErrorSink<'e>,
}

impl Build<'_, '_> {
    fn field(
        &mut self,
        kind: &'static str,
        id: &str,
        path: &str,
        reference: &str,
    ) -> Option<FieldId> {
        let found = self.names.resolve(reference, Some(namespace_of(id)));
        if found.is_none() {
            (self.error)(kind, id, path, format!("there is no field `{reference}`"));
        }
        found
    }

    fn damage_type(
        &mut self,
        kind: &'static str,
        id: &str,
        path: &str,
        reference: &str,
    ) -> Option<DamageTypeId> {
        let found = self.damage_types.resolve(reference, Some(namespace_of(id)));
        if found.is_none() {
            (self.error)(
                kind,
                id,
                path,
                format!("there is no damage type `{reference}`"),
            );
        }
        found
    }

    /// A share from 0 to `high`, or the nearest that is and a mistake.
    fn share(&mut self, id: &str, path: &str, value: Real, low_open: bool, high: Real) -> Real {
        let below = if low_open {
            value <= Real::ZERO
        } else {
            value < Real::ZERO
        };
        if below || value > high || !value.is_finite() {
            let from = if low_open { "more than 0" } else { "from 0" };
            (self.error)(
                FIELD,
                id,
                path,
                format!("must be {from} to {high}, not {value}"),
            );
            return value.clamp(Real::ZERO, high);
        }
        value
    }

    /// The amounts a reaction uses or gives, by field in id order.
    fn amounts(
        &mut self,
        kind: &'static str,
        id: &str,
        path: &str,
        written: &BTreeMap<String, u8>,
    ) -> Vec<(FieldId, u8)> {
        let mut amounts: Vec<(FieldId, u8)> = Vec::with_capacity(written.len());
        for (name, amount) in written {
            let at = format!("{path}.{name}");
            let Some(field) = self.field(kind, id, &at, name) else {
                continue;
            };
            if *amount == 0 {
                (self.error)(kind, id, &at, "must be 1 or more".to_owned());
            } else if amounts.iter().any(|(have, _)| *have == field) {
                (self.error)(kind, id, &at, "this field is named twice".to_owned());
            } else {
                amounts.push((field, *amount));
            }
        }
        amounts.sort_by_key(|(field, _)| *field);
        amounts
    }

    fn rule(&mut self, id: &str, position: usize, spec: &FieldRuleSpec) -> Option<FieldRule> {
        let path = |param: &str| format!("rules.{position}.{param}");
        let given: [(&str, bool); 20] = [
            ("amount", spec.amount.is_some()),
            ("rate", spec.rate.is_some()),
            ("rise", spec.rise.is_some()),
            ("sink", spec.sink.is_some()),
            ("needs", spec.needs.is_some()),
            ("chance", spec.chance.is_some()),
            ("from", spec.from.is_some()),
            ("field", spec.field.is_some()),
            ("with", spec.with.is_some()),
            ("uses", !spec.uses.is_empty()),
            ("gives", !spec.gives.is_empty()),
            ("max", spec.max.is_some()),
            ("kind", spec.kind.is_some()),
            ("energy_per_level", spec.energy_per_level.is_some()),
            ("intensity_per_level", spec.intensity_per_level.is_some()),
            ("share", spec.share.is_some()),
            ("spread", spec.spread.is_some()),
            ("pool", spec.pool.is_some()),
            ("per_level", spec.per_level.is_some()),
            ("route", spec.route.is_some()),
        ];
        let takes = spec.rule.params();
        let mut good = true;
        for (param, _) in given
            .iter()
            .filter(|(param, given)| *given && !takes.contains(param))
        {
            (self.error)(
                FIELD,
                id,
                &path(param),
                format!(
                    "the rule `{}` has no `{param}`: it takes {}",
                    spec.rule.name(),
                    takes
                        .iter()
                        .map(|name| format!("`{name}`"))
                        .collect::<Vec<_>>()
                        .join(", ")
                ),
            );
            good = false;
        }
        let missing = |build: &mut Build<'_, '_>, param: &str| {
            (build.error)(
                FIELD,
                id,
                &path(param),
                format!("the rule `{}` needs `{param}`", spec.rule.name()),
            );
        };
        let from = spec.from.unwrap_or(1).max(1);
        let rule = match spec.rule {
            FieldRuleKind::Decay => match spec.amount {
                Some(amount) if amount > 0 => Some(FieldRule::Decay { amount }),
                Some(_) => {
                    (self.error)(FIELD, id, &path("amount"), "must be 1 or more".to_owned());
                    None
                }
                None => {
                    missing(self, "amount");
                    None
                }
            },
            FieldRuleKind::Diffuse => {
                let rate = match spec.rate {
                    Some(rate) => self.share(id, &path("rate"), rate, true, FIELD_MAX_RATE),
                    None => {
                        missing(self, "rate");
                        Real::ZERO
                    }
                };
                let vertical = match (spec.rise, spec.sink) {
                    (Some(_), Some(_)) => {
                        (self.error)(
                            FIELD,
                            id,
                            &path("sink"),
                            "a field rises or sinks, not both".to_owned(),
                        );
                        VerticalBias::Even
                    }
                    (Some(rise), None) => {
                        VerticalBias::Rise(self.share(id, &path("rise"), rise, true, Real::ONE))
                    }
                    (None, Some(sink)) => {
                        VerticalBias::Sink(self.share(id, &path("sink"), sink, true, Real::ONE))
                    }
                    (None, None) => VerticalBias::Even,
                };
                Some(FieldRule::Diffuse { rate, vertical })
            }
            FieldRuleKind::Flow => {
                let rate = match spec.rate {
                    Some(rate) => self.share(id, &path("rate"), rate, true, FIELD_MAX_RATE),
                    None => FLOW_DEFAULT_RATE,
                };
                Some(FieldRule::Flow { rate })
            }
            FieldRuleKind::Spread => {
                let chance = match spec.chance {
                    Some(chance) => self.share(id, &path("chance"), chance, false, Real::ONE),
                    None => Real::ONE,
                };
                Some(FieldRule::Spread {
                    needs: spec
                        .needs
                        .as_deref()
                        .map(|tag| MaterialTagSet::build(tag, self.materials)),
                    chance,
                    amount: spec.amount.unwrap_or(1).max(1),
                    from,
                })
            }
            FieldRuleKind::Emit => {
                let field = match &spec.field {
                    Some(field) => self.field(FIELD, id, &path("field"), field.as_str()),
                    None => {
                        missing(self, "field");
                        None
                    }
                };
                let amount = match spec.amount {
                    Some(amount) if amount > 0 => Some(amount),
                    Some(_) => {
                        (self.error)(FIELD, id, &path("amount"), "must be 1 or more".to_owned());
                        None
                    }
                    None => {
                        missing(self, "amount");
                        None
                    }
                };
                match (field, amount) {
                    (Some(field), Some(amount)) => Some(FieldRule::Emit {
                        field,
                        amount,
                        from,
                    }),
                    _ => None,
                }
            }
            FieldRuleKind::Convert => match &spec.with {
                Some(with) => self
                    .field(FIELD, id, &path("with"), with.as_str())
                    .map(|with| FieldRule::Convert { with }),
                None => {
                    missing(self, "with");
                    None
                }
            },
            FieldRuleKind::Damage => {
                let kind = match &spec.kind {
                    Some(kind) => self.damage_type(FIELD, id, &path("kind"), kind.as_str()),
                    None => {
                        missing(self, "kind");
                        None
                    }
                };
                let quantity =
                    |build: &mut Build<'_, '_>, param: &str, value: Option<Real>| match value {
                        Some(value) if value >= Real::ZERO && value.is_finite() => Some(value),
                        Some(value) => {
                            (build.error)(
                                FIELD,
                                id,
                                &path(param),
                                format!("must be zero or more, not {value}"),
                            );
                            None
                        }
                        None => {
                            (build.error)(
                                FIELD,
                                id,
                                &path(param),
                                format!("the rule `damage` needs `{param}`"),
                            );
                            None
                        }
                    };
                let energy = quantity(
                    self,
                    "energy_per_level",
                    spec.energy_per_level.map(|energy| energy.get()),
                );
                let intensity = quantity(
                    self,
                    "intensity_per_level",
                    spec.intensity_per_level.map(|intensity| intensity.get()),
                );
                match (kind, energy, intensity) {
                    (Some(kind), Some(energy_per_level), Some(intensity_per_level)) => {
                        Some(FieldRule::Damage {
                            kind,
                            energy_per_level,
                            intensity_per_level,
                            from,
                        })
                    }
                    _ => None,
                }
            }
            FieldRuleKind::Drift => {
                let share = match spec.share {
                    Some(share) => self.share(id, &path("share"), share, false, Real::ONE),
                    None => Real::ONE,
                };
                let spread = match spec.spread {
                    Some(spread) if spread >= Real::ZERO && spread.is_finite() => spread,
                    Some(spread) => {
                        (self.error)(
                            FIELD,
                            id,
                            &path("spread"),
                            format!("must be zero or more, not {spread}"),
                        );
                        Real::ZERO
                    }
                    None => Real::ZERO,
                };
                Some(FieldRule::Drift { share, spread })
            }
            FieldRuleKind::Expose => {
                let pool = match &spec.pool {
                    Some(pool) if !pool.is_empty() => Some(pool.clone()),
                    Some(_) => {
                        (self.error)(FIELD, id, &path("pool"), "must name a pool".to_owned());
                        None
                    }
                    None => {
                        missing(self, "pool");
                        None
                    }
                };
                let per_level = match spec.per_level {
                    Some(dose) if dose.is_finite() => Some(dose),
                    Some(_) => {
                        (self.error)(FIELD, id, &path("per_level"), "must be finite".to_owned());
                        None
                    }
                    None => {
                        missing(self, "per_level");
                        None
                    }
                };
                match (pool, per_level) {
                    (Some(pool), Some(per_level)) => Some(FieldRule::Expose {
                        pool,
                        per_level,
                        route: spec.route.unwrap_or(ExposureRoute::Any),
                        from,
                    }),
                    _ => None,
                }
            }
        };
        if good { rule } else { None }
    }
}

/// A reaction between two fields, before it is handed to both.
struct Pair {
    first: FieldId,
    second: FieldId,
    use_first: u8,
    use_second: u8,
    gives: Vec<(FieldId, u8)>,
    max: u32,
}

fn used(uses: &[(FieldId, u8)], field: FieldId) -> u8 {
    uses.iter()
        .find(|(have, _)| *have == field)
        .map_or(0, |(_, amount)| *amount)
}

/// Builds the resolved fields and reactions from raw ones given in any
/// order. Every mistake found goes to `error`; a definition with a
/// mistake is built as far as it makes sense, so that every mistake is
/// found in one go.
pub(crate) fn build_defs(
    fields: Vec<(String, FieldDef)>,
    reactions: Vec<(String, FieldReactionDef)>,
    damage_types: &Table<DamageTypeId, DamageType>,
    materials: &Table<MaterialId, Material>,
    error: ErrorSink<'_>,
) -> (Table<FieldId, Field>, Table<FieldReactionId, FieldReaction>) {
    let fields = sorted(FIELD, fields, error);
    let reactions = sorted(REACTION, reactions, error);
    if FieldId::from_index(fields.len()).is_none() {
        error(
            FIELD,
            fields.last().map_or("", |(id, _)| id.as_str()),
            "id",
            "there are too many definitions of this type".to_owned(),
        );
    }
    if FieldReactionId::from_index(reactions.len()).is_none() {
        error(
            REACTION,
            reactions.last().map_or("", |(id, _)| id.as_str()),
            "id",
            "there are too many definitions of this type".to_owned(),
        );
    }
    let names: Table<FieldId, ()> = Table::from_sorted(
        fields.iter().map(|(id, _)| id.clone()).collect(),
        fields.iter().map(|_| ()).collect(),
    );
    let mut build = Build {
        names: &names,
        damage_types,
        materials,
        error,
    };

    let mut built: Vec<Field> = Vec::with_capacity(fields.len());
    let mut pairs: Vec<Pair> = Vec::new();
    let mut converts: Vec<Pair> = Vec::new();
    for (position, (id, def)) in fields.iter().enumerate() {
        let step = def.step.dur();
        if step < FIELD_MIN_STEP {
            (build.error)(
                FIELD,
                id,
                "step",
                format!("must be {FIELD_MIN_STEP} or more, not {step}"),
            );
        }
        let mut rules: Vec<FieldRule> = Vec::with_capacity(def.rules.len());
        for (at, spec) in def.rules.iter().enumerate() {
            let Some(rule) = build.rule(id, at, spec) else {
                continue;
            };
            if let FieldRule::Convert { with } = &rule
                && let Some(this) = FieldId::from_index(position)
            {
                let uses = build.amounts(FIELD, id, &format!("rules.{at}.uses"), &spec.uses);
                let gives = build.amounts(FIELD, id, &format!("rules.{at}.gives"), &spec.gives);
                if *with == this {
                    (build.error)(
                        FIELD,
                        id,
                        &format!("rules.{at}.with"),
                        "a field does not react with itself".to_owned(),
                    );
                } else if uses
                    .iter()
                    .any(|(field, _)| *field != this && field != with)
                {
                    (build.error)(
                        FIELD,
                        id,
                        &format!("rules.{at}.uses"),
                        "a reaction uses only the fields that take part in it".to_owned(),
                    );
                } else {
                    converts.push(Pair {
                        first: this,
                        second: *with,
                        use_first: used(&uses, this),
                        use_second: used(&uses, *with),
                        gives,
                        max: spec.max.unwrap_or(u32::MAX).max(1),
                    });
                }
            }
            rules.push(rule);
        }
        let light = def.light.as_ref().and_then(|light| {
            let spec = LightSpec {
                intensity: light.intensity,
                radius: light.radius,
            };
            match LightSource::from_spec(&spec) {
                Ok(full) => full.map(|full| FieldLight {
                    full,
                    from: light.from.unwrap_or(1).max(1),
                }),
                Err(message) => {
                    (build.error)(FIELD, id, "light", message);
                    None
                }
            }
        });
        let warms = def.warms.unwrap_or(Real::ZERO);
        if !warms.is_finite() {
            (build.error)(FIELD, id, "warms", "must be finite".to_owned());
        }
        if def.opaque_from == Some(0) {
            (build.error)(FIELD, id, "opaque_from", "must be 1 or more".to_owned());
        }
        built.push(Field {
            step: step.max(FIELD_MIN_STEP),
            rules,
            reacts: Vec::new(),
            conducts: Vec::new(),
            catch_up: def.catch_up.unwrap_or_default(),
            warms,
            changes: def.changes.clone(),
            light,
            opaque_from: def.opaque_from,
            glyph: def.glyph.as_ref().map(|glyph| FieldGlyph {
                ch: glyph.ch.clone(),
                fg: glyph.fg.clone().unwrap_or_default().0,
                bg: glyph.bg.clone().unwrap_or_default().0,
            }),
            tags: sorted_tags(&def.tags),
            live: false,
            rests: false,
        });
    }

    let mut resolved: Vec<FieldReaction> = Vec::with_capacity(reactions.len());
    for (id, def) in &reactions {
        let mut when: Vec<Reactant> = Vec::with_capacity(2);
        for (position, reactant) in def.when.iter().enumerate() {
            let path = format!("when.{position}");
            let found = match reactant {
                ReactantSpec::Field(field) => build
                    .field(REACTION, id, &path, field.as_str())
                    .map(Reactant::Field),
                ReactantSpec::MaterialTag(tag) => {
                    Some(Reactant::MaterialTag(MaterialTagSet::build(tag, materials)))
                }
                ReactantSpec::Damage(kind) => build
                    .damage_type(REACTION, id, &path, kind.as_str())
                    .map(Reactant::Damage),
            };
            when.extend(found);
        }
        let uses = build.amounts(REACTION, id, "uses", &def.uses);
        let gives = build.amounts(REACTION, id, "gives", &def.gives);
        let max = def.max.unwrap_or(u32::MAX).max(1);
        let spreads = def.spreads.map(|spreads| {
            if spreads.share < Real::ZERO || spreads.share > Real::ONE || !spreads.share.is_finite()
            {
                (build.error)(
                    REACTION,
                    id,
                    "spreads.share",
                    format!("must be from 0 to 1, not {}", spreads.share),
                );
            }
            (spreads.share.clamp(Real::ZERO, Real::ONE), spreads.reach)
        });
        if def.when.len() != 2 {
            (build.error)(
                REACTION,
                id,
                "when",
                "a reaction is between two things".to_owned(),
            );
            continue;
        }
        let Ok(when) = <[Reactant; 2]>::try_from(when) else {
            // A reactant that names nothing has been reported.
            continue;
        };
        let takes_part = |field: FieldId| {
            when.iter()
                .any(|reactant| matches!(reactant, Reactant::Field(have) if *have == field))
        };
        if uses.iter().any(|(field, _)| !takes_part(*field)) {
            (build.error)(
                REACTION,
                id,
                "uses",
                "a reaction uses only the fields that take part in it".to_owned(),
            );
            continue;
        }
        let wrong = |build: &mut Build<'_, '_>, path: &str, message: &str| {
            (build.error)(REACTION, id, path, message.to_owned());
        };
        match (&when[0], &when[1]) {
            (Reactant::Field(first), Reactant::Field(second)) => {
                if first == second {
                    wrong(&mut build, "when", "a field does not react with itself");
                } else if spreads.is_some() {
                    wrong(&mut build, "spreads", "only a reaction to damage spreads");
                } else {
                    pairs.push(Pair {
                        first: *first,
                        second: *second,
                        use_first: used(&uses, *first),
                        use_second: used(&uses, *second),
                        gives: gives.clone(),
                        max,
                    });
                }
            }
            (Reactant::Field(field), Reactant::MaterialTag(set))
            | (Reactant::MaterialTag(set), Reactant::Field(field)) => {
                if spreads.is_some() {
                    wrong(&mut build, "spreads", "only a reaction to damage spreads");
                } else if let Some(entry) = built.get_mut(field.index()) {
                    entry.reacts.push(FieldReacts::Material {
                        set: set.clone(),
                        use_self: used(&uses, *field),
                        gives: gives.clone(),
                        max,
                    });
                }
            }
            (Reactant::Field(field), Reactant::Damage(kind))
            | (Reactant::Damage(kind), Reactant::Field(field)) => match spreads {
                Some((share, reach)) if uses.is_empty() && gives.is_empty() => {
                    if let Some(entry) = built.get_mut(field.index()) {
                        entry.conducts.push(FieldConducts {
                            kind: *kind,
                            share,
                            reach,
                        });
                    }
                }
                Some(_) => wrong(
                    &mut build,
                    "uses",
                    "a reaction to damage only `spreads`: it uses and gives nothing",
                ),
                None => wrong(
                    &mut build,
                    "spreads",
                    "a reaction to damage needs `spreads`",
                ),
            },
            _ => wrong(
                &mut build,
                "when",
                "a reaction is between two fields, a field and a material tag, or a damage \
                 type and a field",
            ),
        }
        resolved.push(FieldReaction {
            when,
            uses,
            gives,
            max,
            spreads,
        });
    }
    // A definition that was skipped keeps its place in the table, so
    // that ids stay those of the loader: the load fails anyway.
    while resolved.len() < reactions.len() {
        resolved.push(FieldReaction {
            when: [
                Reactant::Field(FieldId::new(0)),
                Reactant::Field(FieldId::new(0)),
            ],
            uses: Vec::new(),
            gives: Vec::new(),
            max: 1,
            spreads: None,
        });
    }

    for pair in pairs.into_iter().chain(converts) {
        if let Some(first) = built.get_mut(pair.first.index()) {
            first.reacts.push(FieldReacts::With {
                other: pair.second,
                first: true,
                use_self: pair.use_first,
                use_other: pair.use_second,
                gives: pair.gives.clone(),
                max: pair.max,
            });
        }
        if let Some(second) = built.get_mut(pair.second.index()) {
            second.reacts.push(FieldReacts::With {
                other: pair.first,
                first: false,
                use_self: pair.use_second,
                use_other: pair.use_first,
                gives: pair.gives,
                max: pair.max,
            });
        }
    }
    for field in &mut built {
        field.live = !field.rules.is_empty() || !field.reacts.is_empty();
        field.rests = field.live
            && field
                .rules
                .iter()
                .all(|rule| matches!(rule, FieldRule::Flow { .. } | FieldRule::Diffuse { .. }))
            && field
                .reacts
                .iter()
                .all(|reacts| matches!(reacts, FieldReacts::With { .. }));
    }

    let field_names = fields.into_iter().map(|(id, _)| id).collect();
    let reaction_names = reactions.into_iter().map(|(id, _)| id).collect();
    (
        Table::from_sorted(field_names, built),
        Table::from_sorted(reaction_names, resolved),
    )
}
