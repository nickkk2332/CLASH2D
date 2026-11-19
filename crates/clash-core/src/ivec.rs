//! [`IVec2`] and [`IVec3`]: integer vectors for cell coordinates and
//! offsets.
//!
//! Axes follow the world model (02 §3): `+x` is east, `+y` is south and
//! `+z` is up.
//!
//! Nothing here panics. Component arithmetic wraps on overflow, so it
//! behaves identically in every build profile. Lengths and products are
//! returned as `i64` and saturate, which only vectors at the very ends of
//! the `i32` range can reach.

use core::fmt;
use core::ops::{Add, AddAssign, Mul, Neg, Sub, SubAssign};

use serde::{Deserialize, Serialize};

use crate::hash::{StateHash, StateHasher};
use crate::rot::{Rot4, Turn};

/// A two-dimensional integer vector: a position or an offset on one storey.
///
/// Vectors order by `x`, then by `y`.
///
/// ```
/// use clash_core::{IVec2, Rot4};
///
/// let step = IVec2::new(3, -1);
/// assert_eq!(step + IVec2::new(1, 1), IVec2::new(4, 0));
/// assert_eq!(step * 2, IVec2::new(6, -2));
/// assert_eq!(step.chebyshev(), 3);
/// assert_eq!(step.rotated(Rot4::R90), IVec2::new(1, 3));
/// ```
#[derive(
    Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default, Debug, Serialize, Deserialize,
)]
pub struct IVec2 {
    /// East is positive.
    pub x: i32,
    /// South is positive.
    pub y: i32,
}

impl IVec2 {
    /// The zero vector.
    pub const ZERO: IVec2 = IVec2 { x: 0, y: 0 };

    /// The vector with these components.
    pub const fn new(x: i32, y: i32) -> IVec2 {
        IVec2 { x, y }
    }

    /// This vector with a height added.
    pub const fn with_z(self, z: i32) -> IVec3 {
        IVec3 {
            x: self.x,
            y: self.y,
            z,
        }
    }

    /// The dot product. Saturates at the ends of the `i64` range.
    pub const fn dot(self, other: IVec2) -> i64 {
        (self.x as i64 * other.x as i64).saturating_add(self.y as i64 * other.y as i64)
    }

    /// The square of the straight-line length. Saturates at `i64::MAX`.
    pub const fn length_sq(self) -> i64 {
        self.dot(self)
    }

    /// The length when only orthogonal steps are allowed: `|x| + |y|`.
    pub const fn manhattan(self) -> i64 {
        (self.x as i64).abs() + (self.y as i64).abs()
    }

    /// The length when diagonal steps cost the same as orthogonal ones:
    /// the larger of `|x|` and `|y|`.
    pub const fn chebyshev(self) -> i64 {
        let x = (self.x as i64).abs();
        let y = (self.y as i64).abs();
        if x > y { x } else { y }
    }

    /// The smaller of each component: the north-west corner of the box
    /// around two points.
    pub fn component_min(self, other: IVec2) -> IVec2 {
        IVec2::new(self.x.min(other.x), self.y.min(other.y))
    }

    /// The larger of each component: the south-east corner of the box
    /// around two points.
    pub fn component_max(self, other: IVec2) -> IVec2 {
        IVec2::new(self.x.max(other.x), self.y.max(other.y))
    }

    /// The vector turned about the origin by quarter-turns.
    pub const fn rotated(self, rot: Rot4) -> IVec2 {
        rot.apply(self)
    }

    /// The vector mirrored and turned about the origin.
    pub const fn turned(self, turn: Turn) -> IVec2 {
        turn.apply(self)
    }
}

/// A three-dimensional integer vector: a cell position or offset. `z`
/// counts storeys.
///
/// Vectors order by `x`, then by `y`, then by `z`.
///
/// ```
/// use clash_core::{IVec2, IVec3};
///
/// let cell = IVec3::new(10, 20, 2);
/// assert_eq!(cell.xy(), IVec2::new(10, 20));
/// assert_eq!(cell - IVec3::new(0, 0, 2), IVec2::new(10, 20).with_z(0));
/// ```
#[derive(
    Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default, Debug, Serialize, Deserialize,
)]
pub struct IVec3 {
    /// East is positive.
    pub x: i32,
    /// South is positive.
    pub y: i32,
    /// Up is positive.
    pub z: i32,
}

impl IVec3 {
    /// The zero vector.
    pub const ZERO: IVec3 = IVec3 { x: 0, y: 0, z: 0 };

    /// The vector with these components.
    pub const fn new(x: i32, y: i32, z: i32) -> IVec3 {
        IVec3 { x, y, z }
    }

    /// The horizontal part.
    pub const fn xy(self) -> IVec2 {
        IVec2 {
            x: self.x,
            y: self.y,
        }
    }

    /// The dot product. Saturates at the ends of the `i64` range.
    pub const fn dot(self, other: IVec3) -> i64 {
        (self.x as i64 * other.x as i64)
            .saturating_add(self.y as i64 * other.y as i64)
            .saturating_add(self.z as i64 * other.z as i64)
    }

    /// The square of the straight-line length, counting a storey as one
    /// unit. Saturates at `i64::MAX`.
    pub const fn length_sq(self) -> i64 {
        self.dot(self)
    }

    /// The length when only orthogonal steps are allowed:
    /// `|x| + |y| + |z|`.
    pub const fn manhattan(self) -> i64 {
        (self.x as i64).abs() + (self.y as i64).abs() + (self.z as i64).abs()
    }

    /// The length when diagonal steps cost the same as orthogonal ones:
    /// the largest of `|x|`, `|y|` and `|z|`.
    pub const fn chebyshev(self) -> i64 {
        let flat = self.xy().chebyshev();
        let z = (self.z as i64).abs();
        if flat > z { flat } else { z }
    }

    /// The smallest of each component: the lowest north-west corner of the
    /// box around two points.
    pub fn component_min(self, other: IVec3) -> IVec3 {
        IVec3::new(
            self.x.min(other.x),
            self.y.min(other.y),
            self.z.min(other.z),
        )
    }

    /// The largest of each component: the highest south-east corner of the
    /// box around two points.
    pub fn component_max(self, other: IVec3) -> IVec3 {
        IVec3::new(
            self.x.max(other.x),
            self.y.max(other.y),
            self.z.max(other.z),
        )
    }

    /// The vector turned about the vertical axis by quarter-turns. `z` is
    /// unchanged.
    pub const fn rotated(self, rot: Rot4) -> IVec3 {
        rot.apply(self.xy()).with_z(self.z)
    }

    /// The vector mirrored and turned about the vertical axis. `z` is
    /// unchanged.
    pub const fn turned(self, turn: Turn) -> IVec3 {
        turn.apply(self.xy()).with_z(self.z)
    }
}

impl Add for IVec2 {
    type Output = IVec2;
    fn add(self, rhs: IVec2) -> IVec2 {
        IVec2::new(self.x.wrapping_add(rhs.x), self.y.wrapping_add(rhs.y))
    }
}

impl Sub for IVec2 {
    type Output = IVec2;
    fn sub(self, rhs: IVec2) -> IVec2 {
        IVec2::new(self.x.wrapping_sub(rhs.x), self.y.wrapping_sub(rhs.y))
    }
}

impl Neg for IVec2 {
    type Output = IVec2;
    fn neg(self) -> IVec2 {
        IVec2::new(self.x.wrapping_neg(), self.y.wrapping_neg())
    }
}

impl Mul<i32> for IVec2 {
    type Output = IVec2;
    fn mul(self, rhs: i32) -> IVec2 {
        IVec2::new(self.x.wrapping_mul(rhs), self.y.wrapping_mul(rhs))
    }
}

impl AddAssign for IVec2 {
    fn add_assign(&mut self, rhs: IVec2) {
        *self = *self + rhs;
    }
}

impl SubAssign for IVec2 {
    fn sub_assign(&mut self, rhs: IVec2) {
        *self = *self - rhs;
    }
}

impl Add for IVec3 {
    type Output = IVec3;
    fn add(self, rhs: IVec3) -> IVec3 {
        IVec3::new(
            self.x.wrapping_add(rhs.x),
            self.y.wrapping_add(rhs.y),
            self.z.wrapping_add(rhs.z),
        )
    }
}

impl Sub for IVec3 {
    type Output = IVec3;
    fn sub(self, rhs: IVec3) -> IVec3 {
        IVec3::new(
            self.x.wrapping_sub(rhs.x),
            self.y.wrapping_sub(rhs.y),
            self.z.wrapping_sub(rhs.z),
        )
    }
}

impl Neg for IVec3 {
    type Output = IVec3;
    fn neg(self) -> IVec3 {
        IVec3::new(
            self.x.wrapping_neg(),
            self.y.wrapping_neg(),
            self.z.wrapping_neg(),
        )
    }
}

impl Mul<i32> for IVec3 {
    type Output = IVec3;
    fn mul(self, rhs: i32) -> IVec3 {
        IVec3::new(
            self.x.wrapping_mul(rhs),
            self.y.wrapping_mul(rhs),
            self.z.wrapping_mul(rhs),
        )
    }
}

impl AddAssign for IVec3 {
    fn add_assign(&mut self, rhs: IVec3) {
        *self = *self + rhs;
    }
}

impl SubAssign for IVec3 {
    fn sub_assign(&mut self, rhs: IVec3) {
        *self = *self - rhs;
    }
}

impl From<(i32, i32)> for IVec2 {
    fn from((x, y): (i32, i32)) -> IVec2 {
        IVec2::new(x, y)
    }
}

impl From<[i32; 2]> for IVec2 {
    fn from([x, y]: [i32; 2]) -> IVec2 {
        IVec2::new(x, y)
    }
}

impl From<(i32, i32, i32)> for IVec3 {
    fn from((x, y, z): (i32, i32, i32)) -> IVec3 {
        IVec3::new(x, y, z)
    }
}

impl From<[i32; 3]> for IVec3 {
    fn from([x, y, z]: [i32; 3]) -> IVec3 {
        IVec3::new(x, y, z)
    }
}

/// Written as `(x, y)`.
impl fmt::Display for IVec2 {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "({}, {})", self.x, self.y)
    }
}

/// Written as `(x, y, z)`.
impl fmt::Display for IVec3 {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "({}, {}, {})", self.x, self.y, self.z)
    }
}

impl StateHash for IVec2 {
    fn state_hash(&self, hasher: &mut StateHasher) {
        self.x.state_hash(hasher);
        self.y.state_hash(hasher);
    }
}

impl StateHash for IVec3 {
    fn state_hash(&self, hasher: &mut StateHasher) {
        self.x.state_hash(hasher);
        self.y.state_hash(hasher);
        self.z.state_hash(hasher);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Hash128;

    #[test]
    fn two_dimensional_arithmetic() {
        let a = IVec2::new(3, -4);
        let b = IVec2::new(-1, 2);
        assert_eq!(a + b, IVec2::new(2, -2));
        assert_eq!(a - b, IVec2::new(4, -6));
        assert_eq!(-a, IVec2::new(-3, 4));
        assert_eq!(a * 3, IVec2::new(9, -12));
        let mut c = a;
        c += b;
        c -= IVec2::new(1, 1);
        assert_eq!(c, IVec2::new(1, -3));
        assert_eq!(a.dot(b), -11);
        assert_eq!(a.length_sq(), 25);
        assert_eq!(a.manhattan(), 7);
        assert_eq!(a.chebyshev(), 4);
        assert_eq!(a.component_min(b), IVec2::new(-1, -4));
        assert_eq!(a.component_max(b), IVec2::new(3, 2));
        assert_eq!(IVec2::from((3, -4)), a);
        assert_eq!(IVec2::from([3, -4]), a);
        assert_eq!(a.to_string(), "(3, -4)");
        assert_eq!(IVec2::default(), IVec2::ZERO);
    }

    #[test]
    fn three_dimensional_arithmetic() {
        let a = IVec3::new(1, 2, 3);
        let b = IVec3::new(-4, 5, -6);
        assert_eq!(a + b, IVec3::new(-3, 7, -3));
        assert_eq!(a - b, IVec3::new(5, -3, 9));
        assert_eq!(-a, IVec3::new(-1, -2, -3));
        assert_eq!(a * -2, IVec3::new(-2, -4, -6));
        let mut c = a;
        c += b;
        c -= IVec3::new(0, 0, 1);
        assert_eq!(c, IVec3::new(-3, 7, -4));
        assert_eq!(a.dot(b), -12);
        assert_eq!(b.length_sq(), 77);
        assert_eq!(b.manhattan(), 15);
        assert_eq!(b.chebyshev(), 6);
        assert_eq!(a.component_min(b), IVec3::new(-4, 2, -6));
        assert_eq!(a.component_max(b), IVec3::new(1, 5, 3));
        assert_eq!(a.xy(), IVec2::new(1, 2));
        assert_eq!(IVec2::new(1, 2).with_z(3), a);
        assert_eq!(IVec3::from((1, 2, 3)), a);
        assert_eq!(IVec3::from([1, 2, 3]), a);
        assert_eq!(a.to_string(), "(1, 2, 3)");
    }

    #[test]
    fn extremes_wrap_or_saturate_without_panicking() {
        let far = IVec2::new(i32::MAX, i32::MIN);
        assert_eq!(far + IVec2::new(1, -1), IVec2::new(i32::MIN, i32::MAX));
        assert_eq!(-far, IVec2::new(-i32::MAX, i32::MIN));
        assert_eq!(far.manhattan(), i32::MAX as i64 + (i32::MAX as i64 + 1));
        assert_eq!(far.chebyshev(), i32::MAX as i64 + 1);
        assert_eq!(far.length_sq(), (1_i64 << 62) + ((1_i64 << 31) - 1).pow(2));
        let corner = IVec3::new(i32::MIN, i32::MIN, i32::MIN);
        assert_eq!(corner.length_sq(), i64::MAX);
        assert_eq!(corner.chebyshev(), 1 << 31);
    }

    #[test]
    fn rotation_about_the_vertical_axis_leaves_height_alone() {
        let v = IVec3::new(2, 1, 5);
        assert_eq!(v.rotated(Rot4::R90), IVec3::new(-1, 2, 5));
        assert_eq!(v.rotated(Rot4::R180), IVec3::new(-2, -1, 5));
        assert_eq!(v.turned(Turn::new(Rot4::R0, true)), IVec3::new(-2, 1, 5));
    }

    #[test]
    fn hashing_and_serialising() {
        assert_eq!(
            Hash128::of(&IVec2::new(1, -2)),
            Hash128::of(&(1_i32, -2_i32))
        );
        assert_eq!(
            Hash128::of(&IVec3::new(1, -2, 3)),
            Hash128::of(&(1_i32, -2_i32, 3_i32))
        );
        let json = serde_json::to_string(&IVec3::new(1, -2, 3)).unwrap();
        assert_eq!(json, r#"{"x":1,"y":-2,"z":3}"#);
        assert_eq!(
            serde_json::from_str::<IVec3>(&json).unwrap(),
            IVec3::new(1, -2, 3)
        );
    }

    #[test]
    fn vectors_order_by_x_then_y_then_z() {
        assert!(IVec2::new(1, 9) < IVec2::new(2, 0));
        assert!(IVec2::new(1, 1) < IVec2::new(1, 2));
        assert!(IVec3::new(1, 1, 9) < IVec3::new(1, 2, 0));
    }
}
