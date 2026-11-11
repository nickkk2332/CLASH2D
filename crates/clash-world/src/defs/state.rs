//! Element states: the ids of the definitions they are made from,
//! [`ElementState`] and its typed wrappers, and the text form
//! `"def@material#state"` (02 §5.1).

use core::fmt;

use clash_core::{StateHash, StateHasher, define_id};
use serde::{Deserialize, Serialize};
use thiserror::Error;

define_id! {
    /// A `damage_type` definition.
    pub struct DamageTypeId(u16);
    /// A `material` definition. Id 0 is the built-in `engine:none`.
    pub struct MaterialId(u16);
    /// An `atmosphere` definition.
    pub struct AtmosphereId(u16);
    /// A `floor` definition. Id 0 is the built-in `engine:none`: no floor.
    pub struct FloorId(u16);
    /// A `block` definition. Id 0 is the built-in `engine:none`: no block.
    pub struct BlockId(u16);
    /// A `fixture` definition. Id 0 is the built-in `engine:none`: no
    /// fixture.
    pub struct FixtureId(u16);
    /// A `realm` definition.
    pub struct RealmDefId(u16);
}

/// One of a cell's three elements.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Layer {
    /// The surface at the base of the cell.
    Floor,
    /// What occupies the cell's volume.
    Block,
    /// A static prop standing in the cell.
    Fixture,
}

impl Layer {
    /// The three layers, in the order a cell lists them.
    pub const ALL: [Layer; 3] = [Layer::Floor, Layer::Block, Layer::Fixture];

    /// The name of the layer's definition type in data.
    pub const fn name(self) -> &'static str {
        match self {
            Layer::Floor => "floor",
            Layer::Block => "block",
            Layer::Fixture => "fixture",
        }
    }

    /// The layer's position in [`Layer::ALL`], and in a cell's damage
    /// triple.
    pub const fn index(self) -> usize {
        self as usize
    }
}

impl StateHash for Layer {
    fn state_hash(&self, hasher: &mut StateHasher) {
        hasher.write_u8(*self as u8);
    }
}

/// The state of one element: which definition, made of which material,
/// and sixteen bits the definition gives meaning to (02 §5.1).
///
/// Separating definition from material means any shape can be made of any
/// material. For a definition that declares named `states`, `bits` is the
/// index of the active one. Otherwise `bits` is the definition's own: an
/// amount, a facing.
#[repr(C)]
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug, Serialize, Deserialize)]
pub struct ElementState {
    /// The id of the floor, block or fixture definition. 0 is none.
    pub def: u16,
    /// What it is made of.
    pub material: MaterialId,
    /// Definition-specific: the named state's index, an amount, a facing.
    pub bits: u16,
}

impl ElementState {
    /// No element.
    pub const NONE: ElementState = ElementState {
        def: 0,
        material: MaterialId::new(0),
        bits: 0,
    };

    /// Whether this is no element.
    #[inline]
    pub const fn is_none(self) -> bool {
        self.def == 0
    }

    /// This state, with every form of "no element" turned into
    /// [`ElementState::NONE`], so that equal contents are equal values.
    pub const fn normalised(self) -> ElementState {
        if self.def == 0 {
            ElementState::NONE
        } else {
            self
        }
    }
}

impl Default for ElementState {
    fn default() -> ElementState {
        ElementState::NONE
    }
}

impl StateHash for ElementState {
    fn state_hash(&self, hasher: &mut StateHasher) {
        hasher.write_u16(self.def);
        self.material.state_hash(hasher);
        hasher.write_u16(self.bits);
    }
}

macro_rules! typed_state {
    ($(#[$meta:meta])* $name:ident, $id:ident, $layer:expr) => {
        $(#[$meta])*
        #[derive(
            Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug, Default, Serialize,
            Deserialize,
        )]
        #[serde(transparent)]
        pub struct $name(ElementState);

        impl $name {
            /// No element on this layer.
            pub const NONE: $name = $name(ElementState::NONE);

            /// The layer this state belongs to.
            pub const LAYER: Layer = $layer;

            /// A state of a definition, made of a material, with the
            /// definition's own bits. Definition 0 gives
            #[doc = concat!("[`", stringify!($name), "::NONE`].")]
            pub const fn new(def: $id, material: MaterialId, bits: u16) -> $name {
                $name(
                    ElementState {
                        def: def.raw(),
                        material,
                        bits,
                    }
                    .normalised(),
                )
            }

            /// The state with these raw parts.
            pub const fn from_raw(state: ElementState) -> $name {
                $name(state.normalised())
            }

            /// The raw parts.
            #[inline]
            pub const fn raw(self) -> ElementState {
                self.0
            }

            /// The definition.
            #[inline]
            pub const fn def(self) -> $id {
                $id::new(self.0.def)
            }

            /// The material.
            #[inline]
            pub const fn material(self) -> MaterialId {
                self.0.material
            }

            /// The definition's own bits.
            #[inline]
            pub const fn bits(self) -> u16 {
                self.0.bits
            }

            /// Whether there is no element.
            #[inline]
            pub const fn is_none(self) -> bool {
                self.0.is_none()
            }

            /// The same element made of another material.
            pub const fn with_material(self, material: MaterialId) -> $name {
                $name::from_raw(ElementState { material, ..self.0 })
            }

            /// The same element with other bits.
            pub const fn with_bits(self, bits: u16) -> $name {
                $name::from_raw(ElementState { bits, ..self.0 })
            }
        }

        impl StateHash for $name {
            fn state_hash(&self, hasher: &mut StateHasher) {
                self.0.state_hash(hasher);
            }
        }

        impl From<$name> for ElementState {
            fn from(state: $name) -> ElementState {
                state.0
            }
        }
    };
}

typed_state!(
    /// The state of a cell's floor.
    FloorState,
    FloorId,
    Layer::Floor
);
typed_state!(
    /// The state of a cell's block.
    BlockState,
    BlockId,
    Layer::Block
);
typed_state!(
    /// The state of a cell's fixture.
    FixtureState,
    FixtureId,
    Layer::Fixture
);

/// Why a state reference could not be read or resolved.
#[derive(Clone, PartialEq, Eq, Debug, Error)]
pub enum StateRefError {
    /// The text is not of the form `def`, `def@material`, `def#state` or
    /// `def@material#state`.
    #[error(
        "`{0}` is not a state: write `def`, `def@material`, `def#state` or `def@material#state`"
    )]
    Malformed(String),
    /// No definition of the layer has this id.
    #[error("there is no {layer} `{id}`")]
    UnknownDef {
        /// The layer's definition type.
        layer: &'static str,
        /// The id as written.
        id: String,
    },
    /// No material has this id.
    #[error("there is no material `{0}`")]
    UnknownMaterial(String),
    /// The definition has no state of this name.
    #[error("{layer} `{def}` has no state `{state}`")]
    UnknownState {
        /// The layer's definition type.
        layer: &'static str,
        /// The definition's id.
        def: String,
        /// The state as written.
        state: String,
    },
}

/// The state part of a [`StateRef`].
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum StatePart<'a> {
    /// A named state: `#open`.
    Named(&'a str),
    /// The bits as a number: `#3`.
    Bits(u16),
}

/// A state as written in data: `"gate"` (default material and state),
/// `"gate@alloy"` (material), `"gate#open"` (named state),
/// `"gate@alloy#open"`, or `"gate#3"` (the bits as a number).
///
/// ```
/// use clash_world::{StatePart, StateRef};
///
/// let state = StateRef::parse("mymod:gate@alloy#open").unwrap();
/// assert_eq!(state.def, "mymod:gate");
/// assert_eq!(state.material, Some("alloy"));
/// assert_eq!(state.state, Some(StatePart::Named("open")));
/// assert!(StateRef::parse("gate@").is_err());
/// ```
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct StateRef<'a> {
    /// The definition's id, bare or qualified.
    pub def: &'a str,
    /// The material's id, bare or qualified, if one is given.
    pub material: Option<&'a str>,
    /// The state, if one is given.
    pub state: Option<StatePart<'a>>,
}

impl<'a> StateRef<'a> {
    /// Reads a state reference. Resolving it against definitions is
    /// [`WorldDefs::block_state`](crate::WorldDefs::block_state) and its
    /// siblings.
    pub fn parse(text: &'a str) -> Result<StateRef<'a>, StateRefError> {
        let malformed = || StateRefError::Malformed(text.to_owned());
        let trimmed = text.trim();
        let (head, state) = match trimmed.split_once('#') {
            Some((head, state)) => (head, Some(state)),
            None => (trimmed, None),
        };
        let (def, material) = match head.split_once('@') {
            Some((def, material)) => (def, Some(material)),
            None => (head, None),
        };
        if !is_id(def) || !material.is_none_or(is_id) {
            return Err(malformed());
        }
        let state = match state {
            None => None,
            Some(name) if is_name(name) => Some(StatePart::Named(name)),
            Some(number) => Some(StatePart::Bits(number.parse().map_err(|_| malformed())?)),
        };
        Ok(StateRef {
            def,
            material,
            state,
        })
    }
}

impl fmt::Display for StateRef<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.def)?;
        if let Some(material) = self.material {
            write!(f, "@{material}")?;
        }
        match self.state {
            Some(StatePart::Named(name)) => write!(f, "#{name}"),
            Some(StatePart::Bits(bits)) => write!(f, "#{bits}"),
            None => Ok(()),
        }
    }
}

/// Whether text is a name: `[a-z][a-z0-9_]*` (04 §4.1).
pub(crate) fn is_name(text: &str) -> bool {
    let mut chars = text.chars();
    chars.next().is_some_and(|first| first.is_ascii_lowercase())
        && chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
}

/// Whether text is an id, bare (`name`) or qualified (`namespace:name`).
pub(crate) fn is_id(text: &str) -> bool {
    match text.split_once(':') {
        Some((namespace, name)) => is_name(namespace) && is_name(name),
        None => is_name(text),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_element_state_is_six_bytes() {
        assert_eq!(core::mem::size_of::<ElementState>(), 6);
        assert_eq!(core::mem::size_of::<BlockState>(), 6);
    }

    #[test]
    fn every_form_of_nothing_is_none() {
        let odd = BlockState::new(BlockId::new(0), MaterialId::new(9), 4);
        assert_eq!(odd, BlockState::NONE);
        assert!(odd.is_none());
        let wall = BlockState::new(BlockId::new(3), MaterialId::new(2), 0);
        assert!(!wall.is_none());
        assert_eq!(wall.with_material(MaterialId::new(5)).material().raw(), 5);
        assert_eq!(wall.with_bits(1).bits(), 1);
        assert_eq!(wall.def(), BlockId::new(3));
        assert_eq!(BlockState::default(), BlockState::NONE);
    }

    #[test]
    fn state_references_parse() {
        let plain = StateRef::parse("wall").unwrap();
        assert_eq!(
            (plain.def, plain.material, plain.state),
            ("wall", None, None)
        );
        let made = StateRef::parse("wall@brick").unwrap();
        assert_eq!(made.material, Some("brick"));
        let open = StateRef::parse("door#open").unwrap();
        assert_eq!(open.state, Some(StatePart::Named("open")));
        let all = StateRef::parse(" core:door@core:steel#open ").unwrap();
        assert_eq!(all.to_string(), "core:door@core:steel#open");
        let amount = StateRef::parse("rubble#12").unwrap();
        assert_eq!(amount.state, Some(StatePart::Bits(12)));
        for bad in [
            "",
            "@brick",
            "wall@",
            "wall#",
            "Wall",
            "wall@brick@steel",
            "a:b:c",
            "wall#70000",
            "wall #open",
            "wall#open#shut",
        ] {
            assert!(StateRef::parse(bad).is_err(), "{bad:?} should not parse");
        }
    }
}
