//! The engine's fixed flags: those an element state carries
//! ([`ElementFlags`], 02 §5.4) and those derived for a whole cell
//! ([`CellFlags`], 02 §6.2).

use core::fmt;
use core::ops::{BitAnd, BitOr, BitOrAssign};

use clash_core::{StateHash, StateHasher};
use serde::{Deserialize, Serialize};

/// One of the engine's flags on an element state. The set is fixed: mods
/// combine these and cannot add one (free-form `tags` are for that).
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EngineFlag {
    /// Blocks sight.
    Opaque,
    /// Does not block ordinary walking regardless of height.
    Passable,
    /// Can support blocks and floors above it.
    LoadBearing,
    /// Always supported. Natural solid ground.
    Anchor,
    /// A vertical connection to the storey above, walked up.
    Ramp,
    /// A vertical connection to the storey above, climbed on foot.
    Stairs,
    /// A vertical connection to the storey above, climbed by hand.
    Ladder,
    /// The cell is filled with a still liquid.
    Liquid,
    /// A floor that lets fields and small things through.
    Grate,
    /// Movement penalty.
    Rough,
    /// Affected by the light-damage tier of large energy releases.
    Fragile,
}

impl EngineFlag {
    /// Every flag, in declaration order.
    pub const ALL: [EngineFlag; 11] = [
        EngineFlag::Opaque,
        EngineFlag::Passable,
        EngineFlag::LoadBearing,
        EngineFlag::Anchor,
        EngineFlag::Ramp,
        EngineFlag::Stairs,
        EngineFlag::Ladder,
        EngineFlag::Liquid,
        EngineFlag::Grate,
        EngineFlag::Rough,
        EngineFlag::Fragile,
    ];

    /// The name used in data.
    pub const fn name(self) -> &'static str {
        match self {
            EngineFlag::Opaque => "opaque",
            EngineFlag::Passable => "passable",
            EngineFlag::LoadBearing => "load_bearing",
            EngineFlag::Anchor => "anchor",
            EngineFlag::Ramp => "ramp",
            EngineFlag::Stairs => "stairs",
            EngineFlag::Ladder => "ladder",
            EngineFlag::Liquid => "liquid",
            EngineFlag::Grate => "grate",
            EngineFlag::Rough => "rough",
            EngineFlag::Fragile => "fragile",
        }
    }

    const fn bit(self) -> u16 {
        1 << self as u16
    }
}

/// The engine flags of one element state, as a bitset.
///
/// ```
/// use clash_world::{ElementFlags, EngineFlag};
///
/// let wall = ElementFlags::of(&[EngineFlag::Opaque, EngineFlag::LoadBearing]);
/// assert!(wall.has(EngineFlag::Opaque));
/// assert!(!wall.has(EngineFlag::Passable));
/// assert_eq!(wall.without(EngineFlag::Opaque), ElementFlags::of(&[EngineFlag::LoadBearing]));
/// ```
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default, Serialize, Deserialize)]
pub struct ElementFlags(u16);

impl ElementFlags {
    /// No flags.
    pub const NONE: ElementFlags = ElementFlags(0);

    /// The set of the given flags.
    pub const fn of(flags: &[EngineFlag]) -> ElementFlags {
        let mut bits = 0;
        let mut at = 0;
        while at < flags.len() {
            bits |= flags[at].bit();
            at += 1;
        }
        ElementFlags(bits)
    }

    /// Whether a flag is set.
    #[inline]
    pub const fn has(self, flag: EngineFlag) -> bool {
        self.0 & flag.bit() != 0
    }

    /// This set with a flag added.
    pub const fn with(self, flag: EngineFlag) -> ElementFlags {
        ElementFlags(self.0 | flag.bit())
    }

    /// This set with a flag removed.
    pub const fn without(self, flag: EngineFlag) -> ElementFlags {
        ElementFlags(self.0 & !flag.bit())
    }

    /// Whether no flag is set.
    pub const fn is_empty(self) -> bool {
        self.0 == 0
    }

    /// The flags that are set, in declaration order.
    pub fn iter(self) -> impl Iterator<Item = EngineFlag> {
        EngineFlag::ALL
            .into_iter()
            .filter(move |flag| self.has(*flag))
    }
}

impl fmt::Debug for ElementFlags {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_set()
            .entries(self.iter().map(EngineFlag::name))
            .finish()
    }
}

impl StateHash for ElementFlags {
    fn state_hash(&self, hasher: &mut StateHasher) {
        hasher.write_u16(self.0);
    }
}

/// What the hot queries need to know about a cell, derived from its three
/// elements and kept in a chunk's bitsets (02 §6.2).
///
/// | Flag | Set when |
/// |------|----------|
/// | [`WALK_BLOCKED`](CellFlags::WALK_BLOCKED) | the block or fixture stands in the way of ordinary walking: it is taller than a step, and is not `passable`, a `ramp`, `stairs` or `liquid` |
/// | [`OPAQUE`](CellFlags::OPAQUE) | the block or fixture is `opaque` |
/// | [`HAS_FLOOR`](CellFlags::HAS_FLOOR) | the cell has a floor |
/// | [`SOLID_TOP`](CellFlags::SOLID_TOP) | the block can be stood on from the cell above: full height, fill of at least 0.95, not `liquid` |
/// | [`FRAGILE`](CellFlags::FRAGILE) | any of the three elements is `fragile` |
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default, Serialize, Deserialize)]
pub struct CellFlags(u8);

impl CellFlags {
    /// No flags: an empty cell.
    pub const NONE: CellFlags = CellFlags(0);
    /// Something in the cell stands in the way of ordinary walking.
    pub const WALK_BLOCKED: CellFlags = CellFlags(1 << 0);
    /// Something in the cell blocks sight.
    pub const OPAQUE: CellFlags = CellFlags(1 << 1);
    /// The cell has a floor.
    pub const HAS_FLOOR: CellFlags = CellFlags(1 << 2);
    /// The cell's block can be stood on from the cell above.
    pub const SOLID_TOP: CellFlags = CellFlags(1 << 3);
    /// Something in the cell is fragile.
    pub const FRAGILE: CellFlags = CellFlags(1 << 4);

    /// The flags and their names, in bit order.
    pub const NAMED: [(CellFlags, &'static str); 5] = [
        (CellFlags::WALK_BLOCKED, "walk_blocked"),
        (CellFlags::OPAQUE, "opaque"),
        (CellFlags::HAS_FLOOR, "has_floor"),
        (CellFlags::SOLID_TOP, "solid_top"),
        (CellFlags::FRAGILE, "fragile"),
    ];

    /// Whether every flag of `other` is set here.
    #[inline]
    pub const fn contains(self, other: CellFlags) -> bool {
        self.0 & other.0 == other.0
    }

    /// Whether any flag of `other` is set here.
    #[inline]
    pub const fn intersects(self, other: CellFlags) -> bool {
        self.0 & other.0 != 0
    }

    /// Whether no flag is set.
    pub const fn is_empty(self) -> bool {
        self.0 == 0
    }

    /// Both sets together.
    #[inline]
    pub const fn union(self, other: CellFlags) -> CellFlags {
        CellFlags(self.0 | other.0)
    }

    /// The raw bits.
    pub const fn bits(self) -> u8 {
        self.0
    }

    /// `flag` if `on`, otherwise nothing.
    #[inline]
    pub(crate) const fn when(flag: CellFlags, on: bool) -> CellFlags {
        if on { flag } else { CellFlags::NONE }
    }
}

impl BitOr for CellFlags {
    type Output = CellFlags;
    #[inline]
    fn bitor(self, other: CellFlags) -> CellFlags {
        self.union(other)
    }
}

impl BitOrAssign for CellFlags {
    fn bitor_assign(&mut self, other: CellFlags) {
        self.0 |= other.0;
    }
}

impl BitAnd for CellFlags {
    type Output = CellFlags;
    fn bitand(self, other: CellFlags) -> CellFlags {
        CellFlags(self.0 & other.0)
    }
}

impl fmt::Debug for CellFlags {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_set()
            .entries(
                CellFlags::NAMED
                    .iter()
                    .filter(|(flag, _)| self.contains(*flag))
                    .map(|(_, name)| *name),
            )
            .finish()
    }
}

impl StateHash for CellFlags {
    fn state_hash(&self, hasher: &mut StateHasher) {
        hasher.write_u8(self.0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn flag_names_match_their_data_form() {
        for flag in EngineFlag::ALL {
            let json = format!("\"{}\"", flag.name());
            assert_eq!(serde_json::from_str::<EngineFlag>(&json).unwrap(), flag);
        }
        assert!(serde_json::from_str::<EngineFlag>("\"shiny\"").is_err());
        let all = ElementFlags::of(&EngineFlag::ALL);
        assert_eq!(all.iter().count(), EngineFlag::ALL.len());
    }

    #[test]
    fn cell_flags_combine() {
        let both = CellFlags::OPAQUE | CellFlags::HAS_FLOOR;
        assert!(both.contains(CellFlags::OPAQUE));
        assert!(!both.contains(CellFlags::OPAQUE | CellFlags::FRAGILE));
        assert!(both.intersects(CellFlags::OPAQUE | CellFlags::FRAGILE));
        assert_eq!(format!("{both:?}"), r#"{"opaque", "has_floor"}"#);
        assert_eq!(both & CellFlags::OPAQUE, CellFlags::OPAQUE);
    }
}
