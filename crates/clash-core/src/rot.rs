//! [`Rot4`] and [`Turn`]: the exact ways to turn something on the grid.
//!
//! A grid can be turned only by quarter-turns, and mirrored, without
//! leaving the grid. [`Rot4`] is the four quarter-turns. [`Turn`] adds an
//! optional mirror, giving all eight ways a map can be placed (10 §3.3).
//! Both are exact integer arithmetic.
//!
//! Axes follow the world model (02 §3): `+x` is east and `+y` is south, so
//! on a map drawn with north at the top a positive quarter-turn is
//! **clockwise**: north turns to east.

use serde::{Deserialize, Serialize};

use crate::hash::{StateHash, StateHasher};
use crate::ivec::IVec2;

/// A rotation about the vertical axis by a whole number of quarter-turns,
/// clockwise on the map.
///
/// ```
/// use clash_core::{IVec2, Rot4};
///
/// let north = IVec2::new(0, -1);
/// let east = IVec2::new(1, 0);
/// assert_eq!(Rot4::R90.apply(north), east);
/// assert_eq!(Rot4::R90.then(Rot4::R270), Rot4::R0);
/// assert_eq!(Rot4::R90.inverse(), Rot4::R270);
/// ```
#[derive(
    Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default, Debug, Serialize, Deserialize,
)]
#[serde(rename_all = "snake_case")]
pub enum Rot4 {
    /// No turn.
    #[default]
    R0,
    /// A quarter-turn clockwise: north to east.
    R90,
    /// A half-turn: north to south.
    R180,
    /// A quarter-turn anticlockwise: north to west.
    R270,
}

impl Rot4 {
    /// The four rotations, in order of quarter-turns.
    pub const ALL: [Rot4; 4] = [Rot4::R0, Rot4::R90, Rot4::R180, Rot4::R270];

    /// The rotation by this many clockwise quarter-turns. Any integer is
    /// accepted: `5` and `-3` are both one quarter-turn.
    pub const fn from_quarter_turns(turns: i32) -> Rot4 {
        match turns.rem_euclid(4) {
            0 => Rot4::R0,
            1 => Rot4::R90,
            2 => Rot4::R180,
            _ => Rot4::R270,
        }
    }

    /// The number of clockwise quarter-turns, from 0 to 3.
    pub const fn quarter_turns(self) -> i32 {
        self as i32
    }

    /// The rotation in degrees clockwise: 0, 90, 180 or 270.
    pub const fn degrees(self) -> i32 {
        self.quarter_turns() * 90
    }

    /// This rotation followed by another.
    pub const fn then(self, next: Rot4) -> Rot4 {
        Rot4::from_quarter_turns(self.quarter_turns() + next.quarter_turns())
    }

    /// The rotation that undoes this one.
    pub const fn inverse(self) -> Rot4 {
        Rot4::from_quarter_turns(-self.quarter_turns())
    }

    /// Turns a vector about the origin.
    pub const fn apply(self, v: IVec2) -> IVec2 {
        match self {
            Rot4::R0 => v,
            Rot4::R90 => IVec2::new(v.y.wrapping_neg(), v.x),
            Rot4::R180 => IVec2::new(v.x.wrapping_neg(), v.y.wrapping_neg()),
            Rot4::R270 => IVec2::new(v.y, v.x.wrapping_neg()),
        }
    }
}

/// A quarter-turn rotation with an optional mirror: one of the eight ways
/// to place a map on the grid.
///
/// Applying a turn mirrors first, swapping east and west, and then
/// rotates.
///
/// ```
/// use clash_core::{IVec2, Rot4, Turn};
///
/// let flip = Turn::new(Rot4::R0, true);
/// assert_eq!(flip.apply(IVec2::new(2, 5)), IVec2::new(-2, 5));
///
/// let turn = Turn::new(Rot4::R90, true);
/// let v = IVec2::new(2, 5);
/// assert_eq!(turn.inverse().apply(turn.apply(v)), v);
/// assert_eq!(flip.then(flip), Turn::IDENTITY);
/// ```
#[derive(
    Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default, Debug, Serialize, Deserialize,
)]
pub struct Turn {
    /// The rotation, applied after the mirror.
    pub rot: Rot4,
    /// Whether east and west are swapped before rotating.
    pub mirror: bool,
}

impl Turn {
    /// The turn that changes nothing.
    pub const IDENTITY: Turn = Turn {
        rot: Rot4::R0,
        mirror: false,
    };

    /// All eight turns: the four rotations, then the four rotations of the
    /// mirror image.
    pub const ALL: [Turn; 8] = [
        Turn::new(Rot4::R0, false),
        Turn::new(Rot4::R90, false),
        Turn::new(Rot4::R180, false),
        Turn::new(Rot4::R270, false),
        Turn::new(Rot4::R0, true),
        Turn::new(Rot4::R90, true),
        Turn::new(Rot4::R180, true),
        Turn::new(Rot4::R270, true),
    ];

    /// The turn that mirrors if asked and then rotates.
    pub const fn new(rot: Rot4, mirror: bool) -> Turn {
        Turn { rot, mirror }
    }

    /// Mirrors and turns a vector about the origin.
    pub const fn apply(self, v: IVec2) -> IVec2 {
        let mirrored = if self.mirror {
            IVec2::new(v.x.wrapping_neg(), v.y)
        } else {
            v
        };
        self.rot.apply(mirrored)
    }

    /// This turn followed by another.
    pub const fn then(self, next: Turn) -> Turn {
        // Moving a mirror past a rotation reverses the rotation.
        let first = if next.mirror {
            self.rot.inverse()
        } else {
            self.rot
        };
        Turn {
            rot: first.then(next.rot),
            mirror: self.mirror != next.mirror,
        }
    }

    /// The turn that undoes this one.
    pub const fn inverse(self) -> Turn {
        if self.mirror {
            // A mirrored turn is a reflection, and undoes itself.
            self
        } else {
            Turn {
                rot: self.rot.inverse(),
                mirror: false,
            }
        }
    }
}

impl From<Rot4> for Turn {
    fn from(rot: Rot4) -> Turn {
        Turn::new(rot, false)
    }
}

impl StateHash for Rot4 {
    fn state_hash(&self, hasher: &mut StateHasher) {
        hasher.write_u8(*self as u8);
    }
}

impl StateHash for Turn {
    fn state_hash(&self, hasher: &mut StateHasher) {
        self.rot.state_hash(hasher);
        hasher.write_bool(self.mirror);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Hash128;

    const NORTH: IVec2 = IVec2::new(0, -1);
    const EAST: IVec2 = IVec2::new(1, 0);
    const SOUTH: IVec2 = IVec2::new(0, 1);
    const WEST: IVec2 = IVec2::new(-1, 0);

    #[test]
    fn a_positive_quarter_turn_is_clockwise_on_the_map() {
        assert_eq!(Rot4::R0.apply(NORTH), NORTH);
        assert_eq!(Rot4::R90.apply(NORTH), EAST);
        assert_eq!(Rot4::R90.apply(EAST), SOUTH);
        assert_eq!(Rot4::R90.apply(SOUTH), WEST);
        assert_eq!(Rot4::R90.apply(WEST), NORTH);
        assert_eq!(Rot4::R180.apply(NORTH), SOUTH);
        assert_eq!(Rot4::R270.apply(NORTH), WEST);
    }

    #[test]
    fn quarter_turn_counts_wrap() {
        assert_eq!(Rot4::from_quarter_turns(0), Rot4::R0);
        assert_eq!(Rot4::from_quarter_turns(5), Rot4::R90);
        assert_eq!(Rot4::from_quarter_turns(-3), Rot4::R90);
        assert_eq!(Rot4::from_quarter_turns(-1), Rot4::R270);
        assert_eq!(Rot4::from_quarter_turns(i32::MIN), Rot4::R0);
        assert_eq!(Rot4::R270.quarter_turns(), 3);
        assert_eq!(Rot4::R270.degrees(), 270);
        for (index, rot) in Rot4::ALL.into_iter().enumerate() {
            assert_eq!(rot.quarter_turns(), index as i32);
        }
    }

    #[test]
    fn rotations_compose_and_invert() {
        assert_eq!(Rot4::R90.then(Rot4::R90), Rot4::R180);
        assert_eq!(Rot4::R270.then(Rot4::R180), Rot4::R90);
        assert_eq!(Rot4::R0.inverse(), Rot4::R0);
        assert_eq!(Rot4::R90.inverse(), Rot4::R270);
        assert_eq!(Rot4::R180.inverse(), Rot4::R180);
    }

    #[test]
    fn a_mirror_swaps_east_and_west_only() {
        let flip = Turn::new(Rot4::R0, true);
        assert_eq!(flip.apply(EAST), WEST);
        assert_eq!(flip.apply(WEST), EAST);
        assert_eq!(flip.apply(NORTH), NORTH);
        assert_eq!(flip.apply(SOUTH), SOUTH);
    }

    #[test]
    fn a_turn_mirrors_before_it_rotates() {
        let turn = Turn::new(Rot4::R90, true);
        // East mirrors to west, and west turns clockwise to north.
        assert_eq!(turn.apply(EAST), NORTH);
        assert_eq!(turn.apply(NORTH), EAST);
    }

    #[test]
    fn the_eight_turns_are_distinct_and_closed() {
        let probe = IVec2::new(2, 1);
        let mut images: Vec<IVec2> = Turn::ALL.iter().map(|turn| turn.apply(probe)).collect();
        images.sort();
        images.dedup();
        assert_eq!(images.len(), 8);
        for a in Turn::ALL {
            for b in Turn::ALL {
                assert!(Turn::ALL.contains(&a.then(b)));
                assert_eq!(a.then(b).apply(probe), b.apply(a.apply(probe)));
            }
            assert_eq!(a.then(a.inverse()), Turn::IDENTITY);
            assert_eq!(a.inverse().then(a), Turn::IDENTITY);
        }
        assert_eq!(Turn::default(), Turn::IDENTITY);
        assert_eq!(Turn::from(Rot4::R180), Turn::new(Rot4::R180, false));
    }

    #[test]
    fn hashing_and_serialising() {
        assert_eq!(Hash128::of(&Rot4::R270), Hash128::of(&3_u8));
        assert_eq!(
            Hash128::of(&Turn::new(Rot4::R90, true)),
            Hash128::of(&(1_u8, true))
        );
        assert_eq!(serde_json::to_string(&Rot4::R90).unwrap(), "\"r90\"");
        let json = serde_json::to_string(&Turn::new(Rot4::R180, true)).unwrap();
        assert_eq!(json, r#"{"rot":"r180","mirror":true}"#);
        assert_eq!(
            serde_json::from_str::<Turn>(&json).unwrap(),
            Turn::new(Rot4::R180, true)
        );
    }
}
