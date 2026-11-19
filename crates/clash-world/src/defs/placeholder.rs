//! Placeholder definitions: what stands in for a definition a save
//! refers to and the loaded mods no longer supply (04 §12.3), and the
//! kinds of definition a save can refer to.
//!
//! A placeholder is a definition of the right type, added after every
//! real one, that keeps the id the save wrote. Whatever was made from the
//! missing definition is kept under that id, so that loading the mod
//! again restores it. Placeholders are inert and visibly marked:
//!
//! | Type | The placeholder |
//! |------|-----------------|
//! | `floor` | A floor with no thickness. It can be stood on, weighs nothing and is destroyed by any blow that gets past its material's threshold. |
//! | `block` | A block with no volume, flagged `load_bearing`, `anchor` and `passable`: it does not fall, holds up what stood on it, and is neither seen nor walked into. Any blow that takes effect destroys it. |
//! | `fixture` | A fixture with no volume. |
//! | `material` | No density and no toughness, the default resistance of every damage type, never crushed, and a span that reaches as far as any check looks. |
//! | `area_kind` | No tags, variables, clock or rules. An area keeps what it was made with. |
//! | `realm` | No strata, no generator, barrier edges, and the extent it was saved with. Its stored tracts load; nothing new is generated. |
//! | `damage_type` | A default resistance nothing gets past: it harms no element. It is not `kinetic` and leaves no devastation. |
//! | `plane` | A plane with nothing on it but what the save put there. |
//!
//! Each element shows as [`PLACEHOLDER_GLYPH`], and each element,
//! material, damage type and plane carries the tag [`PLACEHOLDER_TAG`].

use clash_core::{DenseId, Hash128, Real};

use super::{
    AreaKind, BlockState, CellFlags, DamageType, EdgeRule, Element, ElementFlags, EngineFlag,
    GlyphSpec, Layer, Material, MaterialId, Plane, RealmKind, Resist, StateInfo, Table, WorldDefs,
};
use crate::occupancy::PlaneMask;
use crate::topology::{Extent, Topology};

/// The character a placeholder element is drawn as.
pub const PLACEHOLDER_GLYPH: &str = "?";

/// The tag every placeholder element and material carries.
pub const PLACEHOLDER_TAG: &str = "placeholder";

/// How far an element of a placeholder material reaches sideways from
/// support: as far as any check looks, so that a slab whose material has
/// gone stays where it is.
const PLACEHOLDER_SPAN: u32 = u32::MAX;

/// A kind of definition that saved state refers to by id (02 §11).
///
/// Each kind has a table of its own among the save's ids. A definition
/// type that joins the world's saved state adds its kind here and to
/// [`DefKind::ALL`], and answers for it in the four functions of
/// `WorldDefs` below.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub enum DefKind {
    /// A `floor` definition.
    Floor,
    /// A `block` definition.
    Block,
    /// A `fixture` definition.
    Fixture,
    /// A `material` definition.
    Material,
    /// An `area_kind` definition.
    AreaKind,
    /// A `realm` definition.
    Realm,
    /// A `damage_type` definition.
    DamageType,
    /// A `plane` definition.
    Plane,
}

impl DefKind {
    /// How many kinds there are.
    pub const COUNT: usize = 8;

    /// Every kind, in the order their tables are written.
    pub const ALL: [DefKind; DefKind::COUNT] = [
        DefKind::Floor,
        DefKind::Block,
        DefKind::Fixture,
        DefKind::Material,
        DefKind::AreaKind,
        DefKind::Realm,
        DefKind::DamageType,
        DefKind::Plane,
    ];

    /// The name of the definition type in data, which is also the name
    /// of the kind's table in a save.
    pub const fn name(self) -> &'static str {
        match self {
            DefKind::Floor => "floor",
            DefKind::Block => "block",
            DefKind::Fixture => "fixture",
            DefKind::Material => "material",
            DefKind::AreaKind => "area_kind",
            DefKind::Realm => "realm",
            DefKind::DamageType => "damage_type",
            DefKind::Plane => "plane",
        }
    }

    /// The kind with a name.
    pub fn from_name(name: &str) -> Option<DefKind> {
        DefKind::ALL.into_iter().find(|kind| kind.name() == name)
    }

    /// The kind's position in [`DefKind::ALL`].
    pub const fn index(self) -> usize {
        self as usize
    }

    /// The kind of the definitions on a layer.
    pub const fn of_layer(layer: Layer) -> DefKind {
        match layer {
            Layer::Floor => DefKind::Floor,
            Layer::Block => DefKind::Block,
            Layer::Fixture => DefKind::Fixture,
        }
    }

    /// Whether id 0 of the kind is the built-in `engine:none`.
    pub const fn has_none(self) -> bool {
        matches!(
            self,
            DefKind::Floor | DefKind::Block | DefKind::Fixture | DefKind::Material
        )
    }
}

impl core::fmt::Display for DefKind {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(self.name())
    }
}

/// Where the placeholders of each kind begin: the number of real
/// definitions of the kind, once any placeholder has been added.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub(super) struct Placeholders {
    first: [Option<u32>; DefKind::COUNT],
}

impl<I: DenseId, T> Table<I, T> {
    /// Adds a definition after every other and returns its id. It is
    /// not found by name: a reference in data to a definition that does
    /// not exist stays a mistake.
    fn push_unlisted(&mut self, name: &str, entry: T) -> Option<u32> {
        let position = self.entries.len();
        I::from_index(position)?;
        self.names.truncate(position);
        self.names.push(name.to_owned());
        self.entries.push(entry);
        u32::try_from(position).ok()
    }

    /// The id of a definition by its qualified id, as a plain number.
    fn number_of(&self, qualified: &str) -> Option<u32> {
        self.lookup(qualified)
            .and_then(|id| u32::try_from(id.index()).ok())
    }

    /// The qualified id of the definition with a number.
    fn name_at(&self, number: u32) -> Option<&str> {
        self.names
            .get(usize::try_from(number).ok()?)
            .map(String::as_str)
    }
}

/// A floor, block or fixture that stands in for one that is missing.
fn placeholder_element(layer: Layer) -> Element {
    let flags = match layer {
        Layer::Floor | Layer::Fixture => ElementFlags::NONE,
        Layer::Block => ElementFlags::of(&[
            EngineFlag::LoadBearing,
            EngineFlag::Anchor,
            EngineFlag::Passable,
        ]),
    };
    let cell = match layer {
        Layer::Floor => CellFlags::HAS_FLOOR,
        Layer::Block | Layer::Fixture => CellFlags::NONE,
    };
    Element {
        layer,
        material: MaterialId::new(0),
        state_names: Vec::new(),
        states: vec![StateInfo {
            flags,
            cell,
            ..StateInfo::NONE
        }],
        glyphs: vec![GlyphSpec {
            ch: Some(PLACEHOLDER_GLYPH.to_owned()),
            fg: None,
            bg: None,
        }],
        resist: Vec::new(),
        tags: vec![PLACEHOLDER_TAG.to_owned()],
        on_destroy: None,
        heap: None,
        contact: None,
        interactions: Vec::new(),
        device: None,
        lights: vec![None],
    }
}

impl WorldDefs {
    /// A fingerprint of the content the definitions were built from: the
    /// loader's, which covers every finished definition of every type
    /// (04 §9), or for tables built from raw definitions a hash of
    /// those. It is one of the inputs of a generator's fingerprint
    /// (02 §11). Placeholders do not change it.
    pub fn fingerprint(&self) -> Hash128 {
        self.fingerprint
    }

    /// Gives the tables the fingerprint of what they were built from.
    pub(crate) fn set_fingerprint(&mut self, fingerprint: Hash128) {
        self.fingerprint = fingerprint;
    }

    /// How many definitions of a kind there are, placeholders and any
    /// built-in one included.
    pub fn def_count(&self, kind: DefKind) -> usize {
        match kind {
            DefKind::Floor => self.floors.len(),
            DefKind::Block => self.blocks.len(),
            DefKind::Fixture => self.fixtures.len(),
            DefKind::Material => self.materials.len(),
            DefKind::AreaKind => self.area_kinds.len(),
            DefKind::Realm => self.realms.len(),
            DefKind::DamageType => self.damage_types.len(),
            DefKind::Plane => self.planes.len(),
        }
    }

    /// The qualified id of the definition of a kind with a dense id. For
    /// a placeholder this is the id the save wrote.
    pub fn def_name(&self, kind: DefKind, id: u32) -> Option<&str> {
        match kind {
            DefKind::Floor => self.floors.name_at(id),
            DefKind::Block => self.blocks.name_at(id),
            DefKind::Fixture => self.fixtures.name_at(id),
            DefKind::Material => self.materials.name_at(id),
            DefKind::AreaKind => self.area_kinds.name_at(id),
            DefKind::Realm => self.realms.name_at(id),
            DefKind::DamageType => self.damage_types.name_at(id),
            DefKind::Plane => self.planes.name_at(id),
        }
    }

    /// The dense id of the definition of a kind with a qualified id.
    /// Placeholders are not found: they are not definitions data may
    /// refer to.
    pub fn def_lookup(&self, kind: DefKind, qualified: &str) -> Option<u32> {
        match kind {
            DefKind::Floor => self.floors.number_of(qualified),
            DefKind::Block => self.blocks.number_of(qualified),
            DefKind::Fixture => self.fixtures.number_of(qualified),
            DefKind::Material => self.materials.number_of(qualified),
            DefKind::AreaKind => self.area_kinds.number_of(qualified),
            DefKind::Realm => self.realms.number_of(qualified),
            DefKind::DamageType => self.damage_types.number_of(qualified),
            DefKind::Plane => self.planes.number_of(qualified),
        }
    }

    /// Whether a definition is a placeholder: it stands in for one a
    /// save refers to and no loaded mod supplies (04 §12.3).
    pub fn is_placeholder(&self, kind: DefKind, id: u32) -> bool {
        self.placeholders.first[kind.index()].is_some_and(|first| id >= first)
    }

    /// Whether an element state is made from a placeholder: its
    /// definition or its material.
    pub fn is_placeholder_state(&self, layer: Layer, state: super::ElementState) -> bool {
        self.is_placeholder(DefKind::of_layer(layer), u32::from(state.def))
            || self.is_placeholder(DefKind::Material, u32::from(state.material.raw()))
    }

    /// The placeholders, by kind and then in the order they were added,
    /// each with the id it keeps.
    pub fn placeholders(&self) -> Vec<(DefKind, &str)> {
        let mut found = Vec::new();
        for kind in DefKind::ALL {
            let Some(first) = self.placeholders.first[kind.index()] else {
                continue;
            };
            let count = u32::try_from(self.def_count(kind)).unwrap_or(u32::MAX);
            for id in first..count {
                if let Some(name) = self.def_name(kind, id) {
                    found.push((kind, name));
                }
            }
        }
        found
    }

    /// Adds a placeholder for a missing definition and returns its dense
    /// id, or `None` if the kind's ids are used up. A realm's placeholder
    /// takes the extent the realm was saved with; without one it is a
    /// single region.
    pub(crate) fn add_placeholder(
        &mut self,
        kind: DefKind,
        qualified: &str,
        topology: Option<Topology>,
    ) -> Option<u32> {
        let real = u32::try_from(self.def_count(kind)).ok()?;
        let id = match kind {
            DefKind::Floor => self
                .floors
                .push_unlisted(qualified, placeholder_element(Layer::Floor)),
            DefKind::Block => self
                .blocks
                .push_unlisted(qualified, placeholder_element(Layer::Block)),
            DefKind::Fixture => self
                .fixtures
                .push_unlisted(qualified, placeholder_element(Layer::Fixture)),
            DefKind::Material => {
                let resist = self
                    .damage_types
                    .iter()
                    .map(|(_, kind)| kind.default_resist)
                    .collect();
                self.materials.push_unlisted(
                    qualified,
                    Material {
                        density: Real::ZERO,
                        toughness: Real::ZERO,
                        span: Some(PLACEHOLDER_SPAN),
                        bearing: None,
                        flammable: false,
                        rubble: None,
                        color: None,
                        tags: vec![PLACEHOLDER_TAG.to_owned()],
                        resist,
                        friction: None,
                    },
                )
            }
            DefKind::AreaKind => self
                .area_kinds
                .push_unlisted(qualified, AreaKind::default()),
            DefKind::Realm => {
                let region = crate::dims::REGION_CELLS;
                let topology = topology.unwrap_or_else(|| {
                    Topology::new(Extent::Bounded(region), Extent::Bounded(region), 0, 0)
                });
                self.realms.push_unlisted(
                    qualified,
                    RealmKind {
                        topology,
                        size_regions: [1, 1],
                        sea_level: None,
                        calendar: None,
                        generator: None,
                        generator_params: std::collections::BTreeMap::new(),
                        below_fill: BlockState::NONE,
                        above_fill: BlockState::NONE,
                        strata: Vec::new(),
                        time_rate: Real::ONE,
                        edges: core::array::from_fn(|_| EdgeRule::Barrier),
                    },
                )
            }
            // No element or material states a resistance to it, so each
            // takes this one: nothing gets past it.
            DefKind::DamageType => self.damage_types.push_unlisted(
                qualified,
                DamageType {
                    default_resist: Resist {
                        threshold: Real::INFINITY,
                        factor: Real::ZERO,
                    },
                    kinetic: false,
                    devastation: None,
                    tags: vec![PLACEHOLDER_TAG.to_owned()],
                },
            ),
            // A plane's id is its bit in a mask: there is room for so
            // many, placeholders included.
            DefKind::Plane => {
                if real >= PlaneMask::BITS {
                    return None;
                }
                self.planes.push_unlisted(
                    qualified,
                    Plane {
                        tags: vec![PLACEHOLDER_TAG.to_owned()],
                    },
                )
            }
        }?;
        let first = &mut self.placeholders.first[kind.index()];
        if first.is_none() {
            *first = Some(real);
        }
        Some(id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::defs::{ElementState, RawWorldDefs};

    #[test]
    fn a_placeholder_keeps_its_id_and_is_inert() {
        let mut defs = WorldDefs::build(RawWorldDefs::default()).unwrap_or_else(|_| unreachable!());
        assert!(defs.placeholders().is_empty());
        let before = defs.fingerprint();
        let block = defs
            .add_placeholder(DefKind::Block, "gone:gate", None)
            .unwrap_or_default();
        // After the built-in none.
        assert_eq!(block, 1);
        assert_eq!(defs.def_name(DefKind::Block, block), Some("gone:gate"));
        assert_eq!(defs.def_lookup(DefKind::Block, "gone:gate"), None);
        assert!(defs.is_placeholder(DefKind::Block, block));
        assert!(!defs.is_placeholder(DefKind::Block, 0));
        assert!(!defs.is_placeholder(DefKind::Floor, 1));
        assert_eq!(defs.placeholders(), [(DefKind::Block, "gone:gate")]);
        assert_eq!(defs.fingerprint(), before);

        let state = ElementState {
            def: 1,
            material: MaterialId::new(0),
            bits: 3,
        };
        assert!(defs.is_placeholder_state(Layer::Block, state));
        let info = defs.info(Layer::Block, state);
        assert!(info.flags.has(EngineFlag::Anchor));
        assert!(info.flags.has(EngineFlag::Passable));
        assert_eq!(info.cell, CellFlags::NONE);
        assert_eq!(defs.volume(Layer::Block, state), Real::ZERO);
        assert_eq!(defs.capacity(Layer::Block, state), Real::ZERO);
        assert_eq!(
            defs.state_text(Layer::Block, state),
            "gone:gate@engine:none#3"
        );

        let floor = defs
            .add_placeholder(DefKind::Floor, "gone:planks", None)
            .unwrap_or_default();
        let floor = ElementState {
            def: u16::try_from(floor).unwrap_or_default(),
            material: MaterialId::new(0),
            bits: 0,
        };
        assert_eq!(defs.info(Layer::Floor, floor).cell, CellFlags::HAS_FLOOR);
    }

    #[test]
    fn kinds_have_names() {
        for kind in DefKind::ALL {
            assert_eq!(DefKind::from_name(kind.name()), Some(kind));
            assert_eq!(DefKind::ALL[kind.index()], kind);
        }
        assert_eq!(DefKind::from_name("creature"), None);
        assert_eq!(DefKind::of_layer(Layer::Fixture), DefKind::Fixture);
    }

    #[test]
    fn a_damage_type_that_has_gone_harms_nothing_and_a_plane_keeps_its_bit() {
        use crate::defs::DamageTypeId;

        let mut defs = WorldDefs::build(RawWorldDefs::default()).unwrap_or_else(|_| unreachable!());
        let real = u32::try_from(defs.def_count(DefKind::DamageType)).unwrap_or_default();
        let gone = defs
            .add_placeholder(DefKind::DamageType, "gone:acid", None)
            .unwrap_or_default();
        assert_eq!(gone, real);
        assert!(defs.is_placeholder(DefKind::DamageType, gone));
        assert_eq!(defs.def_name(DefKind::DamageType, gone), Some("gone:acid"));
        assert_eq!(defs.def_lookup(DefKind::DamageType, "gone:acid"), None);
        let kind = DamageTypeId::new(u16::try_from(gone).unwrap_or_default());
        let resist = defs.resist(Layer::Block, ElementState::NONE, kind);
        assert_eq!(resist.factor, Real::ZERO);
        assert_eq!(resist.threshold, Real::INFINITY);
        assert_ne!(defs.kinetic_damage_type(), Some(kind));

        // The built-in plane is plane 0, and a plane that has gone takes
        // the next bit. There is room for as many as a mask has bits.
        assert_eq!(
            defs.def_name(DefKind::Plane, 0),
            Some(crate::defs::DEFAULT_PLANE_ID)
        );
        for bit in 1..PlaneMask::BITS {
            let name = format!("gone:plane_{bit}");
            assert_eq!(defs.add_placeholder(DefKind::Plane, &name, None), Some(bit));
        }
        assert_eq!(
            defs.add_placeholder(DefKind::Plane, "gone:one_more", None),
            None
        );
        assert!(defs.is_placeholder(DefKind::Plane, 1));
        assert!(!defs.is_placeholder(DefKind::Plane, 0));
    }
}
