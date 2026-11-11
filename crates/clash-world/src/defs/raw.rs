//! The definition types of the world model as mods write them (02 §4.1,
//! §5.4, §5.5): plain structs read from JSON, with references still held
//! as text. [`WorldDefs::build`](crate::WorldDefs::build) turns a set of
//! them into resolved tables.
//!
//! Each struct holds the keys of one definition apart from the keys every
//! definition has (`type`, `id`, comments and the inheritance keys of
//! 04 §5), which the loader consumes. Unknown keys are errors.

use core::fmt;
use core::marker::PhantomData;
use std::collections::BTreeMap;

use clash_core::Real;
use serde::Deserialize;
use serde::de::{Deserializer, MapAccess, Visitor};

use super::flags::EngineFlag;
use super::quantity::{Density, Energy, EnergyDensity, Length, Pressure, Speed, Temperature};

/// A reference to another definition by its id, bare (`"alloy"`) or
/// qualified (`"mymod:alloy"`). Resolved when definitions are built.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Deserialize)]
#[serde(transparent)]
pub struct DefRef(pub String);

impl DefRef {
    /// The id as written.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl From<&str> for DefRef {
    fn from(id: &str) -> DefRef {
        DefRef(id.to_owned())
    }
}

/// A JSON object read as a list of its entries in the order they were
/// written. Used where the order of keys carries meaning (04 §5): the
/// first state of an element is its default.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OrderedMap<V>(pub Vec<(String, V)>);

impl<V> Default for OrderedMap<V> {
    fn default() -> OrderedMap<V> {
        OrderedMap(Vec::new())
    }
}

impl<'de, V: Deserialize<'de>> Deserialize<'de> for OrderedMap<V> {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<OrderedMap<V>, D::Error> {
        struct Entries<V>(PhantomData<V>);

        impl<'de, V: Deserialize<'de>> Visitor<'de> for Entries<V> {
            type Value = OrderedMap<V>;

            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str("an object")
            }

            fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<OrderedMap<V>, A::Error> {
                let mut entries: Vec<(String, V)> = Vec::new();
                while let Some((key, value)) = map.next_entry::<String, V>()? {
                    if entries.iter().any(|(seen, _)| *seen == key) {
                        return Err(serde::de::Error::custom(format!(
                            "the key `{key}` is given twice"
                        )));
                    }
                    entries.push((key, value));
                }
                Ok(OrderedMap(entries))
            }
        }

        deserializer.deserialize_map(Entries(PhantomData))
    }
}

fn one() -> Real {
    Real::ONE
}

/// How a material or element resists one damage type (02 §5.4, §12.2).
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ResistSpec {
    /// Intensity below which the damage type does nothing.
    pub threshold: Pressure,
    /// Multiplier on the energy that gets through. Default 1.
    #[serde(default = "one")]
    pub factor: Real,
}

/// A `damage_type` definition.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DamageTypeDef {
    /// The resistance of anything that does not state its own. Default:
    /// no threshold, factor 1.
    #[serde(default)]
    pub default_resist: Option<ResistSpec>,
    /// Whether this is the type bodies and falling masses deliver. At
    /// most one loaded damage type may say so (02 §23).
    #[serde(default)]
    pub kinetic: bool,
    /// The kind of devastation this type leaves (02 §17.2). Stored as
    /// written until that definition type exists.
    #[serde(default)]
    pub devastation: Option<DefRef>,
    /// Free-form tags.
    #[serde(default)]
    pub tags: Vec<String>,
}

/// A `material` definition (02 §5.4).
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MaterialDef {
    /// Mass per volume.
    pub density: Density,
    /// Energy to destroy one cubic metre.
    pub toughness: EnergyDensity,
    /// Resistance by damage type id.
    #[serde(default)]
    pub resist: BTreeMap<String, ResistSpec>,
    /// How many cells an element of this material can reach sideways from
    /// support.
    #[serde(default)]
    pub span: Option<u32>,
    /// The stress it can carry in compression before it is crushed.
    /// Absent means it is never crushed.
    #[serde(default)]
    pub bearing: Option<Pressure>,
    /// Whether it is fuel for fire.
    #[serde(default)]
    pub flammable: bool,
    /// The fixture destroyed elements of this material leave behind.
    #[serde(default)]
    pub rubble: Option<DefRef>,
    /// A colour name, for presentation.
    #[serde(default)]
    pub color: Option<String>,
    /// Free-form tags.
    #[serde(default)]
    pub tags: Vec<String>,
}

/// An `atmosphere` definition (02 §4.1).
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AtmosphereDef {
    /// Mass per volume.
    pub density: Density,
    /// Ambient pressure.
    pub pressure: Pressure,
    /// The speed of sound in it.
    pub sound_speed: Speed,
    /// Whether it can be breathed.
    #[serde(default)]
    pub breathable: bool,
    /// Free-form tags.
    #[serde(default)]
    pub tags: Vec<String>,
}

/// How an element is drawn. A presentation hint: the world stores it and
/// gives it no meaning.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GlyphSpec {
    /// The character.
    #[serde(default)]
    pub ch: Option<String>,
    /// The foreground colour: a colour name, or `"material"`.
    #[serde(default)]
    pub fg: Option<String>,
    /// The background colour: a colour name, or `"material"`.
    #[serde(default)]
    pub bg: Option<String>,
}

impl GlyphSpec {
    /// This glyph with the fields `over` gives laid over it.
    pub fn overlaid(&self, over: &GlyphSpec) -> GlyphSpec {
        GlyphSpec {
            ch: over.ch.clone().or_else(|| self.ch.clone()),
            fg: over.fg.clone().or_else(|| self.fg.clone()),
            bg: over.bg.clone().or_else(|| self.bg.clone()),
        }
    }
}

/// A named state of a block or fixture: what it changes from the
/// definition's own values.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StateSpec {
    /// Flags this state adds.
    #[serde(default)]
    pub add_flags: Vec<EngineFlag>,
    /// Flags this state removes.
    #[serde(default)]
    pub remove_flags: Vec<EngineFlag>,
    /// Height in this state.
    #[serde(default)]
    pub height: Option<Length>,
    /// Fill in this state.
    #[serde(default)]
    pub fill: Option<Real>,
    /// Coverage in this state.
    #[serde(default)]
    pub coverage: Option<Real>,
    /// Glyph fields this state changes.
    #[serde(default)]
    pub glyph: Option<GlyphSpec>,
}

/// A named state of a floor.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FloorStateSpec {
    /// Flags this state adds.
    #[serde(default)]
    pub add_flags: Vec<EngineFlag>,
    /// Flags this state removes.
    #[serde(default)]
    pub remove_flags: Vec<EngineFlag>,
    /// Thickness in this state.
    #[serde(default)]
    pub thickness: Option<Length>,
    /// Glyph fields this state changes.
    #[serde(default)]
    pub glyph: Option<GlyphSpec>,
}

/// The striking surface of an element's `contact` (07 §6.5).
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ContactSurfaceSpec {
    /// The damage type it delivers.
    pub kind: DefRef,
    /// The area of the surface, in square metres.
    pub area: Real,
    /// How far it gives.
    pub give: Length,
    /// How wide it is.
    #[serde(default)]
    pub span: Option<Length>,
}

/// What an element does to whatever walks into it, lands on it or is
/// thrown against it (02 §5.4).
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ContactSpec {
    /// The striking surface.
    pub surface: ContactSurfaceSpec,
    /// The energy it delivers of its own.
    pub energy: Energy,
}

/// A `block` definition: what occupies a cell's volume (02 §5.4).
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BlockDef {
    /// The default material.
    pub material: DefRef,
    /// Height from the cell base, 0 to 3 m.
    pub height: Length,
    /// Fraction of the cell volume occupied, 0 to 1.
    pub fill: Real,
    /// Fraction of the cell's cross-section blocked. Default `height / 3`.
    #[serde(default)]
    pub coverage: Option<Real>,
    /// Engine flags.
    #[serde(default)]
    pub flags: Vec<EngineFlag>,
    /// Free-form tags.
    #[serde(default)]
    pub tags: Vec<String>,
    /// Resistance by damage type id, overriding the material's.
    #[serde(default)]
    pub resist: BTreeMap<String, ResistSpec>,
    /// Named states. The first is the default.
    #[serde(default)]
    pub states: OrderedMap<StateSpec>,
    /// The block state this becomes when destroyed.
    #[serde(default)]
    pub on_destroy: Option<String>,
    /// A striking surface of its own.
    #[serde(default)]
    pub contact: Option<ContactSpec>,
    /// The verbs it offers (02 §5.5). Stored and reported only.
    #[serde(default)]
    pub interactions: Vec<String>,
    /// The entity definition of its device (02 §5.5). Stored and reported
    /// only.
    #[serde(default)]
    pub device: Option<DefRef>,
    /// How it is drawn.
    #[serde(default)]
    pub glyph: Option<GlyphSpec>,
}

/// A `fixture` definition: a static prop standing in a cell (02 §5.4).
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FixtureDef {
    /// The default material.
    pub material: DefRef,
    /// Height from the cell base, 0 to 3 m.
    pub height: Length,
    /// Fraction of the cell volume occupied, 0 to 1.
    pub fill: Real,
    /// Fraction of the cell's cross-section blocked. Default `height / 3`.
    #[serde(default)]
    pub coverage: Option<Real>,
    /// Engine flags.
    #[serde(default)]
    pub flags: Vec<EngineFlag>,
    /// Free-form tags.
    #[serde(default)]
    pub tags: Vec<String>,
    /// Resistance by damage type id, overriding the material's.
    #[serde(default)]
    pub resist: BTreeMap<String, ResistSpec>,
    /// Named states. The first is the default.
    #[serde(default)]
    pub states: OrderedMap<StateSpec>,
    /// The fixture state this becomes when destroyed.
    #[serde(default)]
    pub on_destroy: Option<String>,
    /// The block a full cell of this fixture becomes (02 §12.3).
    #[serde(default)]
    pub heap: Option<DefRef>,
    /// A striking surface of its own.
    #[serde(default)]
    pub contact: Option<ContactSpec>,
    /// The verbs it offers (02 §5.5). Stored and reported only.
    #[serde(default)]
    pub interactions: Vec<String>,
    /// The entity definition of its device (02 §5.5). Stored and reported
    /// only.
    #[serde(default)]
    pub device: Option<DefRef>,
    /// How it is drawn.
    #[serde(default)]
    pub glyph: Option<GlyphSpec>,
}

/// A `floor` definition: the surface at the base of a cell (02 §5.4).
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FloorDef {
    /// The default material.
    pub material: DefRef,
    /// Thickness.
    pub thickness: Length,
    /// Engine flags.
    #[serde(default)]
    pub flags: Vec<EngineFlag>,
    /// Free-form tags.
    #[serde(default)]
    pub tags: Vec<String>,
    /// Resistance by damage type id, overriding the material's.
    #[serde(default)]
    pub resist: BTreeMap<String, ResistSpec>,
    /// Named states. The first is the default.
    #[serde(default)]
    pub states: OrderedMap<FloorStateSpec>,
    /// The floor state this becomes when destroyed.
    #[serde(default)]
    pub on_destroy: Option<String>,
    /// A striking surface of its own.
    #[serde(default)]
    pub contact: Option<ContactSpec>,
    /// The verbs it offers (02 §5.5). Stored and reported only.
    #[serde(default)]
    pub interactions: Vec<String>,
    /// The entity definition of its device (02 §5.5). Stored and reported
    /// only.
    #[serde(default)]
    pub device: Option<DefRef>,
    /// How it is drawn.
    #[serde(default)]
    pub glyph: Option<GlyphSpec>,
}

/// A horizontal axis that may wrap.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WrapAxis {
    /// East and west meet.
    X,
    /// North and south meet.
    Y,
}

/// What implicit cells contain outside a tract's stored range.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FillSpec {
    /// The block state below. Absent or `null` is nothing.
    #[serde(default)]
    pub below: Option<String>,
    /// The block state above. Absent or `null` is nothing.
    #[serde(default)]
    pub above: Option<String>,
}

/// Ambient conditions from a storey upward (02 §4.1).
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StratumSpec {
    /// The lowest storey this stratum covers.
    pub from_z: i32,
    /// Gravity in m/s², along −z.
    pub gravity: Real,
    /// The atmosphere. Absent or `null` is vacuum.
    #[serde(default)]
    pub atmosphere: Option<DefRef>,
    /// A presentation hint.
    #[serde(default)]
    pub sky: Option<DefRef>,
    /// Kelvin added to the region's temperature.
    #[serde(default)]
    pub temp_offset: Option<Real>,
    /// A temperature replacing the region's.
    #[serde(default)]
    pub temp: Option<Temperature>,
}

/// What a face of a realm does to whatever reaches it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EdgeKind {
    /// Impassable and indestructible.
    Barrier,
    /// Whatever leaves is removed, and an event is emitted.
    Void,
}

/// Where a boundary link leads.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExitKind {
    /// Another realm, at mapped coordinates.
    Realm,
    /// A named entry point of another realm.
    Entry,
    /// A node of a chart.
    Chart,
}

/// How coordinates map across a boundary link to another realm.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LinkMapSpec {
    /// A scale as numerator and denominator.
    pub scale: [i32; 2],
    /// An offset in cells.
    pub offset: [i32; 3],
}

/// A boundary link: an edge that sends whatever crosses it somewhere else
/// (02 §19). Stored as written; package W19 gives it behaviour.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BoundaryLinkSpec {
    /// Where it leads.
    pub exit: ExitKind,
    /// The realm definition, for `realm` and `entry` exits.
    #[serde(default)]
    pub realm: Option<DefRef>,
    /// The coordinate mapping, for `realm` exits.
    #[serde(default)]
    pub map: Option<LinkMapSpec>,
    /// The entry point, for `entry` exits.
    #[serde(default)]
    pub entry: Option<String>,
    /// The chart, for `chart` exits.
    #[serde(default)]
    pub chart: Option<DefRef>,
    /// The chart's node, for `chart` exits.
    #[serde(default)]
    pub node: Option<String>,
}

/// One face of a realm: `"barrier"`, `"void"`, or a boundary link.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(untagged)]
pub enum EdgeSpec {
    /// `"barrier"` or `"void"`.
    Kind(EdgeKind),
    /// A boundary link.
    Link(BoundaryLinkSpec),
}

/// The faces of a realm. A face not given is a barrier.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EdgesSpec {
    /// Above the highest storey.
    #[serde(default)]
    pub top: Option<EdgeSpec>,
    /// Below the lowest storey.
    #[serde(default)]
    pub bottom: Option<EdgeSpec>,
    /// Beyond the lowest `y`.
    #[serde(default)]
    pub north: Option<EdgeSpec>,
    /// Beyond the highest `y`.
    #[serde(default)]
    pub south: Option<EdgeSpec>,
    /// Beyond the highest `x`.
    #[serde(default)]
    pub east: Option<EdgeSpec>,
    /// Beyond the lowest `x`.
    #[serde(default)]
    pub west: Option<EdgeSpec>,
}

/// A generator that derives a realm from another (02 §21).
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DerivedGeneratorSpec {
    /// The kind of generator: `"derived"`.
    #[serde(rename = "type")]
    pub kind: String,
    /// The realm definition it derives from.
    pub source: DefRef,
    /// `"pristine"` or `"live"`.
    pub mode: String,
    /// A map transform applied on top.
    #[serde(default)]
    pub transform: Option<DefRef>,
}

/// Which world generator plans a realm. Stored as written; generation is
/// document 10's.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(untagged)]
pub enum GeneratorSpec {
    /// A generator by id.
    Named(DefRef),
    /// A generator that takes another realm as input.
    Derived(DerivedGeneratorSpec),
}

/// A `realm` definition (02 §4.1).
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RealmDef {
    /// Horizontal extent in regions, along `x` and `y`.
    pub size_regions: [u32; 2],
    /// The storey up to which ocean tracts are filled with still liquid.
    #[serde(default)]
    pub sea_level: Option<i32>,
    /// The calendar definition. Stored as written.
    #[serde(default)]
    pub calendar: Option<DefRef>,
    /// Axes that wrap around.
    #[serde(default)]
    pub wrap: Vec<WrapAxis>,
    /// Lowest and highest storey.
    pub z_range: [i32; 2],
    /// Which world generator plans the realm.
    #[serde(default)]
    pub generator: Option<GeneratorSpec>,
    /// What implicit cells contain below and above a tract's stored range.
    #[serde(default)]
    pub fill: Option<FillSpec>,
    /// Ambient conditions by altitude.
    pub strata: Vec<StratumSpec>,
    /// Rate of the realm's clock relative to universal time. Default 1.
    #[serde(default = "one")]
    pub time_rate: Real,
    /// What each face does. Wrapped axes have no faces.
    #[serde(default)]
    pub edges: EdgesSpec,
}

/// The raw definitions of the world model, each with its qualified id
/// (`namespace:name`), in any order. The input of
/// [`WorldDefs::build`](crate::WorldDefs::build).
#[derive(Debug, Clone, Default)]
pub struct RawWorldDefs {
    /// `damage_type` definitions.
    pub damage_types: Vec<(String, DamageTypeDef)>,
    /// `material` definitions.
    pub materials: Vec<(String, MaterialDef)>,
    /// `atmosphere` definitions.
    pub atmospheres: Vec<(String, AtmosphereDef)>,
    /// `floor` definitions.
    pub floors: Vec<(String, FloorDef)>,
    /// `block` definitions.
    pub blocks: Vec<(String, BlockDef)>,
    /// `fixture` definitions.
    pub fixtures: Vec<(String, FixtureDef)>,
    /// `realm` definitions.
    pub realms: Vec<(String, RealmDef)>,
}
