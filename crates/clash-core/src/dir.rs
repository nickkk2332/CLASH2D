//! [`Dir8`] and [`Vertical`]: the directions of the grid.
//!
//! Axes follow the world model (02 §3): `+x` is east, `+y` is south and
//! `+z` is up.

use serde::{Deserialize, Serialize};

use crate::hash::{StateHash, StateHasher};
use crate::ivec::IVec2;
use crate::rot::{Rot4, Turn};

/// One of the eight horizontal directions, in clockwise order from north.
///
/// In data a direction is written in `snake_case`: `"north"`,
/// `"south_east"`.
///
/// ```
/// use clash_core::{Dir8, IVec2, Rot4};
///
/// assert_eq!(Dir8::NorthEast.delta(), IVec2::new(1, -1));
/// assert_eq!(Dir8::North.opposite(), Dir8::South);
/// assert_eq!(Dir8::North.rotated(Rot4::R90), Dir8::East);
/// assert_eq!(Dir8::from_delta(IVec2::new(-1, 0)), Some(Dir8::West));
/// ```
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Dir8 {
    /// Toward `-y`.
    North,
    /// Toward `+x` and `-y`.
    NorthEast,
    /// Toward `+x`.
    East,
    /// Toward `+x` and `+y`.
    SouthEast,
    /// Toward `+y`.
    South,
    /// Toward `-x` and `+y`.
    SouthWest,
    /// Toward `-x`.
    West,
    /// Toward `-x` and `-y`.
    NorthWest,
}

impl Dir8 {
    /// All eight directions, clockwise from north.
    pub const ALL: [Dir8; 8] = [
        Dir8::North,
        Dir8::NorthEast,
        Dir8::East,
        Dir8::SouthEast,
        Dir8::South,
        Dir8::SouthWest,
        Dir8::West,
        Dir8::NorthWest,
    ];

    /// The four orthogonal directions, clockwise from north.
    pub const CARDINALS: [Dir8; 4] = [Dir8::North, Dir8::East, Dir8::South, Dir8::West];

    /// The four diagonal directions, clockwise from north-east.
    pub const DIAGONALS: [Dir8; 4] = [
        Dir8::NorthEast,
        Dir8::SouthEast,
        Dir8::SouthWest,
        Dir8::NorthWest,
    ];

    /// The direction's position in [`Dir8::ALL`], from 0 to 7.
    pub const fn index(self) -> usize {
        self as usize
    }

    /// The direction this many eighth-turns clockwise from north. Any
    /// integer is accepted: `9` and `-7` are both north-east.
    pub const fn from_eighth_turns(turns: i32) -> Dir8 {
        Dir8::ALL[turns.rem_euclid(8) as usize]
    }

    /// The step of one cell in this direction.
    pub const fn delta(self) -> IVec2 {
        match self {
            Dir8::North => IVec2::new(0, -1),
            Dir8::NorthEast => IVec2::new(1, -1),
            Dir8::East => IVec2::new(1, 0),
            Dir8::SouthEast => IVec2::new(1, 1),
            Dir8::South => IVec2::new(0, 1),
            Dir8::SouthWest => IVec2::new(-1, 1),
            Dir8::West => IVec2::new(-1, 0),
            Dir8::NorthWest => IVec2::new(-1, -1),
        }
    }

    /// The direction of a step of one cell, or `None` if the vector is not
    /// one of the eight single steps.
    pub const fn from_delta(delta: IVec2) -> Option<Dir8> {
        match (delta.x, delta.y) {
            (0, -1) => Some(Dir8::North),
            (1, -1) => Some(Dir8::NorthEast),
            (1, 0) => Some(Dir8::East),
            (1, 1) => Some(Dir8::SouthEast),
            (0, 1) => Some(Dir8::South),
            (-1, 1) => Some(Dir8::SouthWest),
            (-1, 0) => Some(Dir8::West),
            (-1, -1) => Some(Dir8::NorthWest),
            _ => None,
        }
    }

    /// The direction facing the other way.
    pub const fn opposite(self) -> Dir8 {
        self.turned_by_eighths(4)
    }

    /// The next direction clockwise, an eighth of a turn on.
    pub const fn clockwise(self) -> Dir8 {
        self.turned_by_eighths(1)
    }

    /// The next direction anticlockwise, an eighth of a turn back.
    pub const fn anticlockwise(self) -> Dir8 {
        self.turned_by_eighths(-1)
    }

    /// Whether this is north, east, south or west.
    pub const fn is_cardinal(self) -> bool {
        self.index().is_multiple_of(2)
    }

    /// Whether this is one of the four diagonals.
    pub const fn is_diagonal(self) -> bool {
        !self.is_cardinal()
    }

    /// The direction after a rotation by quarter-turns.
    pub const fn rotated(self, rot: Rot4) -> Dir8 {
        self.turned_by_eighths(rot.quarter_turns() * 2)
    }

    /// The direction after a mirror and a rotation.
    pub const fn turned(self, turn: Turn) -> Dir8 {
        // Mirroring swaps east and west: clockwise from north becomes
        // anticlockwise from north.
        let mirrored = if turn.mirror {
            Dir8::from_eighth_turns(-(self as i32))
        } else {
            self
        };
        mirrored.rotated(turn.rot)
    }

    const fn turned_by_eighths(self, eighths: i32) -> Dir8 {
        Dir8::from_eighth_turns(self as i32 + eighths)
    }
}

/// One of the two vertical directions.
///
/// In data it is written `"up"` or `"down"`.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Vertical {
    /// Toward `+z`: the storey above.
    Up,
    /// Toward `-z`: the storey below.
    Down,
}

impl Vertical {
    /// Both vertical directions: up, then down.
    pub const ALL: [Vertical; 2] = [Vertical::Up, Vertical::Down];

    /// The change in `z` of one step: `+1` for up, `-1` for down.
    pub const fn delta(self) -> i32 {
        match self {
            Vertical::Up => 1,
            Vertical::Down => -1,
        }
    }

    /// The other vertical direction.
    pub const fn opposite(self) -> Vertical {
        match self {
            Vertical::Up => Vertical::Down,
            Vertical::Down => Vertical::Up,
        }
    }
}

impl StateHash for Dir8 {
    fn state_hash(&self, hasher: &mut StateHasher) {
        hasher.write_u8(*self as u8);
    }
}

impl StateHash for Vertical {
    fn state_hash(&self, hasher: &mut StateHasher) {
        hasher.write_u8(*self as u8);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Hash128;

    #[test]
    fn deltas_follow_the_axes() {
        assert_eq!(Dir8::North.delta(), IVec2::new(0, -1));
        assert_eq!(Dir8::East.delta(), IVec2::new(1, 0));
        assert_eq!(Dir8::South.delta(), IVec2::new(0, 1));
        assert_eq!(Dir8::West.delta(), IVec2::new(-1, 0));
        assert_eq!(Dir8::SouthWest.delta(), IVec2::new(-1, 1));
        for dir in Dir8::ALL {
            assert_eq!(Dir8::from_delta(dir.delta()), Some(dir));
            assert_eq!(dir.delta().chebyshev(), 1);
        }
        assert_eq!(Dir8::from_delta(IVec2::ZERO), None);
        assert_eq!(Dir8::from_delta(IVec2::new(2, 0)), None);
    }

    #[test]
    fn the_order_is_clockwise_from_north() {
        for (index, dir) in Dir8::ALL.into_iter().enumerate() {
            assert_eq!(dir.index(), index);
            assert_eq!(Dir8::from_eighth_turns(index as i32), dir);
            assert_eq!(dir.clockwise().index(), (index + 1) % 8);
            assert_eq!(dir.clockwise().anticlockwise(), dir);
        }
        assert_eq!(Dir8::from_eighth_turns(9), Dir8::NorthEast);
        assert_eq!(Dir8::from_eighth_turns(-7), Dir8::NorthEast);
        assert_eq!(Dir8::North.anticlockwise(), Dir8::NorthWest);
    }

    #[test]
    fn opposites_and_kinds() {
        for dir in Dir8::ALL {
            assert_eq!(dir.opposite().delta(), -dir.delta());
            assert_eq!(dir.opposite().opposite(), dir);
            assert_ne!(dir.is_cardinal(), dir.is_diagonal());
            assert_eq!(dir.is_diagonal(), dir.delta().manhattan() == 2);
        }
        assert!(Dir8::CARDINALS.iter().all(|dir| dir.is_cardinal()));
        assert!(Dir8::DIAGONALS.iter().all(|dir| dir.is_diagonal()));
    }

    #[test]
    fn turning_a_direction_turns_its_delta() {
        for dir in Dir8::ALL {
            for rot in Rot4::ALL {
                assert_eq!(dir.rotated(rot).delta(), rot.apply(dir.delta()));
            }
            for turn in Turn::ALL {
                assert_eq!(dir.turned(turn).delta(), turn.apply(dir.delta()));
            }
        }
        assert_eq!(Dir8::East.turned(Turn::new(Rot4::R0, true)), Dir8::West);
        assert_eq!(
            Dir8::NorthEast.turned(Turn::new(Rot4::R0, true)),
            Dir8::NorthWest
        );
    }

    #[test]
    fn vertical_directions() {
        assert_eq!(Vertical::Up.delta(), 1);
        assert_eq!(Vertical::Down.delta(), -1);
        assert_eq!(Vertical::Up.opposite(), Vertical::Down);
        assert_eq!(Vertical::Down.opposite(), Vertical::Up);
        assert_eq!(Vertical::ALL, [Vertical::Up, Vertical::Down]);
    }

    #[test]
    fn hashing_and_serialising() {
        assert_eq!(Hash128::of(&Dir8::SouthEast), Hash128::of(&3_u8));
        assert_eq!(Hash128::of(&Vertical::Down), Hash128::of(&1_u8));
        assert_eq!(
            serde_json::to_string(&Dir8::SouthEast).unwrap(),
            "\"south_east\""
        );
        assert_eq!(
            serde_json::from_str::<Dir8>("\"north\"").unwrap(),
            Dir8::North
        );
        assert_eq!(serde_json::to_string(&Vertical::Up).unwrap(), "\"up\"");
        assert!(serde_json::from_str::<Dir8>("\"upward\"").is_err());
    }
}
