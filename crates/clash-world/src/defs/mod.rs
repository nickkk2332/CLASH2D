//! The world model's definitions: damage types, materials, atmospheres,
//! floors, blocks, fixtures and realms (02 §4.1, §5).
//!
//! Mods write definitions as JSON. Each kind has a plain struct read from
//! that JSON ([`MaterialDef`], [`BlockDef`], …) in which references are
//! still text. [`WorldDefs::build`] takes a set of them, in any order,
//! and produces [`WorldDefs`]: the same definitions sorted by id, given
//! dense numeric ids, with every reference resolved and the per-state
//! tables derived. `WorldDefs` is immutable, `Send + Sync`, and shared
//! behind an `Arc`.
//!
//! The structs are registered with the mod loader, and
//! [`WorldDefs::from_defs`] builds the tables from what it loaded: see
//! [`crate::load`].
//!
//! Nothing here names a particular material or block. What exists is
//! whatever the definitions say.

mod build;
mod flags;
mod quantity;
mod raw;
mod state;

use core::fmt;
use std::collections::BTreeMap;

use clash_core::{DenseId, Real};

pub use flags::{CellFlags, ElementFlags, EngineFlag};
pub use quantity::{Density, Energy, EnergyDensity, Length, Pressure, Speed, Temperature};
pub use raw::{
    AtmosphereDef, BlockDef, BoundaryLinkSpec, ContactSpec, ContactSurfaceSpec, DamageTypeDef,
    DefRef, DerivedGeneratorSpec, EdgeKind, EdgeSpec, EdgesSpec, ExitKind, FillSpec, FixtureDef,
    FloorDef, FloorStateSpec, GeneratorSpec, GlyphSpec, LinkMapSpec, MaterialDef, OrderedMap,
    RawWorldDefs, RealmDef, ResistSpec, StateSpec, StratumSpec, WrapAxis,
};
pub use state::{
    AtmosphereId, BlockId, BlockState, DamageTypeId, ElementState, FixtureId, FixtureState,
    FloorId, FloorState, Layer, MaterialId, RealmDefId, StatePart, StateRef, StateRefError,
};

use crate::topology::{Edge, Topology};

/// The reserved namespace of what the engine itself supplies (04 §4.4).
pub const ENGINE_NAMESPACE: &str = "engine";

/// The qualified id of the built-in "nothing here" of every element type
/// and of materials. It has dense id 0.
pub const NONE_ID: &str = "engine:none";

/// The least fill a full-height block needs for its top to be stood on
/// (02 §5.2).
pub const SOLID_TOP_MIN_FILL: Real = Real::of(0.95);

/// The tallest obstacle ordinary walking steps over, in metres. A block
/// or fixture taller than this stands in the way unless it is flagged
/// `passable`, `ramp`, `stairs` or `liquid`. This feeds the
/// `walk_blocked` bit only: the movement rules, which know the mover,
/// read the heights themselves.
pub const WALK_STEP_M: Real = Real::of(0.5);

/// A mistake in a definition, found while building [`WorldDefs`].
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct DefError {
    /// The definition type: `"material"`, `"block"`, ….
    pub kind: &'static str,
    /// The definition's id.
    pub id: String,
    /// Where in the definition, as a dotted path: `"resist.some_type"`.
    pub path: String,
    /// What is wrong.
    pub message: String,
}

impl fmt::Display for DefError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} `{}`", self.kind, self.id)?;
        if !self.path.is_empty() {
            write!(f, ", {}", self.path)?;
        }
        write!(f, ": {}", self.message)
    }
}

impl std::error::Error for DefError {}

/// The definitions of one type, in dense-id order.
///
/// Ids are assigned in sorted order of qualified id, so the same set of
/// definitions always gets the same numbers.
#[derive(Clone, Debug)]
pub struct Table<I, T> {
    names: Vec<String>,
    by_name: BTreeMap<String, I>,
    entries: Vec<T>,
}

impl<I: DenseId, T> Table<I, T> {
    /// The definition with an id.
    #[inline]
    pub fn get(&self, id: I) -> Option<&T> {
        self.entries.get(id.index())
    }

    /// The id of a definition by its qualified id.
    pub fn lookup(&self, qualified: &str) -> Option<I> {
        self.by_name.get(qualified).copied()
    }

    /// The id a reference means. A qualified reference is looked up
    /// exactly. A bare name is looked up in `namespace` first, if one is
    /// given, and then in every namespace in sorted order.
    ///
    /// The search of every namespace stands in for a mod's import list
    /// until the loader supplies one (04 §4.3).
    pub fn resolve(&self, reference: &str, namespace: Option<&str>) -> Option<I> {
        if reference.contains(':') {
            return self.lookup(reference);
        }
        if let Some(namespace) = namespace
            && let Some(id) = self.lookup(&format!("{namespace}:{reference}"))
        {
            return Some(id);
        }
        self.by_name
            .iter()
            .find(|(name, _)| {
                name.split_once(':')
                    .is_some_and(|(_, bare)| bare == reference)
            })
            .map(|(_, id)| *id)
    }

    /// The qualified id of a definition, or `"engine:unknown"` for an id
    /// that names none.
    pub fn name(&self, id: I) -> &str {
        self.names
            .get(id.index())
            .map_or("engine:unknown", String::as_str)
    }

    /// Every definition with its id, in id order.
    pub fn iter(&self) -> impl Iterator<Item = (I, &T)> + '_ {
        self.entries
            .iter()
            .enumerate()
            .filter_map(|(index, entry)| Some((I::from_index(index)?, entry)))
    }

    /// How many definitions there are, counting any built-in one.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Whether there are no definitions at all.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

/// How something resists one damage type (02 §12.2).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Resist {
    /// Intensity in pascals below which the damage type does nothing.
    pub threshold: Real,
    /// Multiplier on the energy that gets through.
    pub factor: Real,
}

impl Resist {
    /// No resistance: no threshold and a factor of one.
    pub const NONE: Resist = Resist {
        threshold: Real::ZERO,
        factor: Real::ONE,
    };
}

/// A resolved `damage_type` definition.
#[derive(Clone, Debug)]
pub struct DamageType {
    /// The resistance of anything that does not state its own.
    pub default_resist: Resist,
    /// Whether this is the type bodies and falling masses deliver.
    pub kinetic: bool,
    /// The kind of devastation it leaves, as written.
    pub devastation: Option<String>,
    /// Free-form tags, sorted.
    pub tags: Vec<String>,
}

/// A resolved `material` definition.
#[derive(Clone, Debug)]
pub struct Material {
    /// Mass per volume in kg/m³.
    pub density: Real,
    /// Energy to destroy one cubic metre, in J/m³.
    pub toughness: Real,
    /// How many cells an element of this material can reach sideways
    /// from support, if the definition says.
    pub span: Option<u32>,
    /// The compressive stress in pascals it can carry. `None` means it is
    /// never crushed.
    pub bearing: Option<Real>,
    /// Whether it is fuel for fire.
    pub flammable: bool,
    /// The fixture destroyed elements of this material leave behind.
    pub rubble: Option<FixtureId>,
    /// A colour name, for presentation.
    pub color: Option<String>,
    /// Free-form tags, sorted.
    pub tags: Vec<String>,
    /// Resistance to each damage type, by dense id: the material's own,
    /// or else the damage type's default.
    pub resist: Vec<Resist>,
}

/// A resolved `atmosphere` definition.
#[derive(Clone, Debug)]
pub struct Atmosphere {
    /// Mass per volume in kg/m³.
    pub density: Real,
    /// Ambient pressure in pascals.
    pub pressure: Real,
    /// The speed of sound in m/s.
    pub sound_speed: Real,
    /// Whether it can be breathed.
    pub breathable: bool,
    /// Free-form tags, sorted.
    pub tags: Vec<String>,
}

/// The derived values of one state of an element (02 §5.3).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct StateInfo {
    /// The engine flags of the state.
    pub flags: ElementFlags,
    /// What the element contributes to its cell's derived flags.
    pub cell: CellFlags,
    /// Metres from the cell base, 0 to 3. Zero for a floor.
    pub height: Real,
    /// Fraction of the cell volume occupied. Zero for a floor.
    pub fill: Real,
    /// Fraction of the cell's cross-section blocked. Zero for a floor.
    pub coverage: Real,
    /// Metres. Zero for a block or fixture.
    pub thickness: Real,
    /// Cubic metres: `fill × 3` for blocks and fixtures, `thickness × 1`
    /// for floors.
    pub volume: Real,
}

impl StateInfo {
    /// The values of "no element".
    pub const NONE: StateInfo = StateInfo {
        flags: ElementFlags::NONE,
        cell: CellFlags::NONE,
        height: Real::ZERO,
        fill: Real::ZERO,
        coverage: Real::ZERO,
        thickness: Real::ZERO,
        volume: Real::ZERO,
    };
}

/// A resolved striking surface of an element (02 §5.4).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Contact {
    /// The damage type it delivers.
    pub kind: DamageTypeId,
    /// The area of the surface in m².
    pub area: Real,
    /// How far it gives, in metres.
    pub give: Real,
    /// How wide it is, in metres.
    pub span: Option<Real>,
    /// The energy in joules it delivers of its own.
    pub energy: Real,
}

/// A resolved `floor`, `block` or `fixture` definition.
#[derive(Clone, Debug)]
pub struct Element {
    /// Which layer it belongs to.
    pub layer: Layer,
    /// The default material.
    pub material: MaterialId,
    /// The names of the declared states, in order. Empty when the
    /// definition declares none.
    pub state_names: Vec<String>,
    /// The derived values of each state. There is always at least one:
    /// a definition without declared states has a single unnamed state.
    pub states: Vec<StateInfo>,
    /// The glyph of each state: the definition's with the state's changes
    /// laid over it.
    pub glyphs: Vec<GlyphSpec>,
    /// Resistance overrides by damage type id. `None` defers to the
    /// material.
    pub resist: Vec<Option<Resist>>,
    /// Free-form tags, sorted.
    pub tags: Vec<String>,
    /// What it becomes when destroyed. `None`: it is removed.
    pub on_destroy: Option<ElementState>,
    /// The block a full cell of this fixture becomes.
    pub heap: Option<BlockId>,
    /// A striking surface of its own.
    pub contact: Option<Contact>,
    /// The verbs it offers. The world stores and reports them.
    pub interactions: Vec<String>,
    /// The entity definition of its device, as written. The world stores
    /// and reports it.
    pub device: Option<String>,
}

impl Element {
    /// The derived values for a state's bits. For a definition with
    /// declared states the bits are the state's index, and an index past
    /// the end means the first state. Otherwise there is one state
    /// whatever the bits.
    #[inline]
    pub fn info(&self, bits: u16) -> &StateInfo {
        match self.states.get(usize::from(bits)) {
            Some(info) => info,
            None => self.states.first().unwrap_or(&StateInfo::NONE),
        }
    }

    /// The index of a declared state by name.
    pub fn state_index(&self, name: &str) -> Option<u16> {
        self.state_names
            .iter()
            .position(|state| state == name)
            .and_then(|index| u16::try_from(index).ok())
    }

    /// Whether the definition carries a tag.
    pub fn has_tag(&self, tag: &str) -> bool {
        self.tags
            .binary_search_by(|have| have.as_str().cmp(tag))
            .is_ok()
    }
}

/// Ambient conditions from a storey upward.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Stratum {
    /// The lowest storey this stratum covers.
    pub from_z: i32,
    /// Gravity in m/s², along −z.
    pub gravity: Real,
    /// The atmosphere. `None` is vacuum.
    pub atmosphere: Option<AtmosphereId>,
    /// A presentation hint, as written.
    pub sky: Option<String>,
    /// Kelvin added to the region's temperature.
    pub temp_offset: Option<Real>,
    /// A temperature in kelvin replacing the region's.
    pub temp: Option<Real>,
}

/// What a face of a realm does.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum EdgeRule {
    /// Impassable and indestructible.
    Barrier,
    /// Whatever leaves is removed, and an event is emitted.
    Void,
    /// A boundary link, as written (02 §19).
    Link(BoundaryLinkSpec),
    /// The axis wraps: there is no face here.
    Wrapped,
}

/// A resolved `realm` definition (02 §4.1).
#[derive(Clone, Debug)]
pub struct RealmKind {
    /// The realm's extent, wrap and storeys.
    pub topology: Topology,
    /// Horizontal extent in regions.
    pub size_regions: [u32; 2],
    /// The storey up to which ocean tracts are filled.
    pub sea_level: Option<i32>,
    /// The calendar definition, as written.
    pub calendar: Option<String>,
    /// Which world generator plans the realm, as written.
    pub generator: Option<GeneratorSpec>,
    /// What implicit cells hold below a tract's stored range.
    pub below_fill: BlockState,
    /// What implicit cells hold above a tract's stored range.
    pub above_fill: BlockState,
    /// Ambient conditions by altitude, lowest first. Never empty.
    pub strata: Vec<Stratum>,
    /// Rate of the realm's clock relative to universal time.
    pub time_rate: Real,
    /// What each face does, in the order of [`Edge::ALL`].
    pub edges: [EdgeRule; 6],
}

impl RealmKind {
    /// The stratum that covers a storey: the highest whose `from_z` is at
    /// or below it, or the lowest stratum for a storey below them all.
    pub fn stratum_at(&self, z: i32) -> Option<&Stratum> {
        let above = self.strata.partition_point(|stratum| stratum.from_z <= z);
        self.strata.get(above.saturating_sub(1))
    }

    /// What a face does.
    pub fn edge(&self, edge: Edge) -> &EdgeRule {
        &self.edges[edge as usize]
    }
}

/// Every definition of the world model, resolved and immutable.
///
/// Built once by [`WorldDefs::build`] and shared behind an `Arc`.
#[derive(Clone, Debug)]
pub struct WorldDefs {
    damage_types: Table<DamageTypeId, DamageType>,
    materials: Table<MaterialId, Material>,
    atmospheres: Table<AtmosphereId, Atmosphere>,
    floors: Table<FloorId, Element>,
    blocks: Table<BlockId, Element>,
    fixtures: Table<FixtureId, Element>,
    realms: Table<RealmDefId, RealmKind>,
    kinetic: Option<DamageTypeId>,
}

// `WorldDefs` is shared between threads behind an `Arc`.
const _: fn() = || {
    fn shared<T: Send + Sync>() {}
    shared::<WorldDefs>();
};

impl WorldDefs {
    /// Builds the resolved definitions from raw ones given in any order.
    ///
    /// Definitions are sorted by qualified id and numbered densely in that
    /// order, per type; id 0 of materials and of each element type is the
    /// built-in `engine:none`. References are resolved, values are checked
    /// against their ranges, and the per-state tables are derived.
    ///
    /// Every mistake found is returned, not only the first. This never
    /// panics, whatever the definitions hold.
    pub fn build(raw: RawWorldDefs) -> Result<WorldDefs, Vec<DefError>> {
        build::build(raw)
    }

    /// The `damage_type` definitions.
    pub fn damage_types(&self) -> &Table<DamageTypeId, DamageType> {
        &self.damage_types
    }

    /// The `material` definitions.
    pub fn materials(&self) -> &Table<MaterialId, Material> {
        &self.materials
    }

    /// The `atmosphere` definitions.
    pub fn atmospheres(&self) -> &Table<AtmosphereId, Atmosphere> {
        &self.atmospheres
    }

    /// The `floor` definitions.
    pub fn floors(&self) -> &Table<FloorId, Element> {
        &self.floors
    }

    /// The `block` definitions.
    pub fn blocks(&self) -> &Table<BlockId, Element> {
        &self.blocks
    }

    /// The `fixture` definitions.
    pub fn fixtures(&self) -> &Table<FixtureId, Element> {
        &self.fixtures
    }

    /// The `realm` definitions.
    pub fn realms(&self) -> &Table<RealmDefId, RealmKind> {
        &self.realms
    }

    /// The damage type flagged `kinetic`, which bodies and falling masses
    /// deliver, if one is loaded.
    pub fn kinetic_damage_type(&self) -> Option<DamageTypeId> {
        self.kinetic
    }

    fn elements(&self, layer: Layer) -> &[Element] {
        match layer {
            Layer::Floor => &self.floors.entries,
            Layer::Block => &self.blocks.entries,
            Layer::Fixture => &self.fixtures.entries,
        }
    }

    /// The definition of an element on a layer. An id that names none
    /// gives the built-in `none`.
    #[inline]
    pub fn element(&self, layer: Layer, state: ElementState) -> Option<&Element> {
        let elements = self.elements(layer);
        elements.get(usize::from(state.def)).or(elements.first())
    }

    /// The derived values of an element state on a layer.
    #[inline]
    pub fn info(&self, layer: Layer, state: ElementState) -> &StateInfo {
        match self.element(layer, state) {
            Some(element) => element.info(state.bits),
            None => &StateInfo::NONE,
        }
    }

    /// The derived values of a floor state.
    #[inline]
    pub fn floor_info(&self, state: FloorState) -> &StateInfo {
        self.info(Layer::Floor, state.raw())
    }

    /// The derived values of a block state.
    #[inline]
    pub fn block_info(&self, state: BlockState) -> &StateInfo {
        self.info(Layer::Block, state.raw())
    }

    /// The derived values of a fixture state.
    #[inline]
    pub fn fixture_info(&self, state: FixtureState) -> &StateInfo {
        self.info(Layer::Fixture, state.raw())
    }

    /// The derived flags of a cell holding these three elements.
    #[inline]
    pub fn cell_flags(
        &self,
        floor: FloorState,
        block: BlockState,
        fixture: FixtureState,
    ) -> CellFlags {
        self.floor_info(floor).cell | self.block_info(block).cell | self.fixture_info(fixture).cell
    }

    /// The engine flags of an element state.
    pub fn flags(&self, layer: Layer, state: ElementState) -> ElementFlags {
        self.info(layer, state).flags
    }

    /// The volume of an element state in cubic metres.
    pub fn volume(&self, layer: Layer, state: ElementState) -> Real {
        self.info(layer, state).volume
    }

    /// The mass of an element state in kilograms: volume × the density of
    /// its material.
    pub fn mass(&self, layer: Layer, state: ElementState) -> Real {
        let density = self
            .materials
            .get(state.material)
            .map_or(Real::ZERO, |m| m.density);
        self.volume(layer, state) * density
    }

    /// The energy in joules an undamaged element state absorbs before it
    /// is destroyed: volume × the toughness of its material (02 §12.2).
    pub fn capacity(&self, layer: Layer, state: ElementState) -> Real {
        let toughness = self
            .materials
            .get(state.material)
            .map_or(Real::ZERO, |m| m.toughness);
        self.volume(layer, state) * toughness
    }

    /// How an element state resists a damage type: the definition's
    /// override, else its material's, else the damage type's default
    /// (02 §5.4).
    pub fn resist(&self, layer: Layer, state: ElementState, kind: DamageTypeId) -> Resist {
        let own = self
            .element(layer, state)
            .and_then(|element| element.resist.get(kind.index()).copied().flatten());
        if let Some(resist) = own {
            return resist;
        }
        self.materials
            .get(state.material)
            .and_then(|material| material.resist.get(kind.index()).copied())
            .or_else(|| self.damage_types.get(kind).map(|kind| kind.default_resist))
            .unwrap_or(Resist::NONE)
    }

    /// The floor state a reference means. Bare names are looked up in
    /// `namespace` first, if given, then everywhere.
    pub fn floor_state(
        &self,
        text: &str,
        namespace: Option<&str>,
    ) -> Result<FloorState, StateRefError> {
        build::resolve_state(&self.floors, &self.materials, Layer::Floor, text, namespace)
            .map(FloorState::from_raw)
    }

    /// The block state a reference means. Bare names are looked up in
    /// `namespace` first, if given, then everywhere.
    pub fn block_state(
        &self,
        text: &str,
        namespace: Option<&str>,
    ) -> Result<BlockState, StateRefError> {
        build::resolve_state(&self.blocks, &self.materials, Layer::Block, text, namespace)
            .map(BlockState::from_raw)
    }

    /// The fixture state a reference means. Bare names are looked up in
    /// `namespace` first, if given, then everywhere.
    pub fn fixture_state(
        &self,
        text: &str,
        namespace: Option<&str>,
    ) -> Result<FixtureState, StateRefError> {
        build::resolve_state(
            &self.fixtures,
            &self.materials,
            Layer::Fixture,
            text,
            namespace,
        )
        .map(FixtureState::from_raw)
    }

    /// An element state written out in full, the way data writes it:
    /// `mymod:gate@mymod:alloy#open`.
    pub fn state_text(&self, layer: Layer, state: ElementState) -> String {
        let names = match layer {
            Layer::Floor => &self.floors.names,
            Layer::Block => &self.blocks.names,
            Layer::Fixture => &self.fixtures.names,
        };
        let def = names
            .get(usize::from(state.def))
            .map_or(NONE_ID, String::as_str);
        if state.is_none() {
            return def.to_owned();
        }
        let mut text = format!("{def}@{}", self.materials.name(state.material));
        match self
            .element(layer, state)
            .and_then(|e| e.state_names.get(usize::from(state.bits)))
        {
            Some(name) => text.push_str(&format!("#{name}")),
            None if state.bits != 0 => text.push_str(&format!("#{}", state.bits)),
            None => {}
        }
        text
    }
}
