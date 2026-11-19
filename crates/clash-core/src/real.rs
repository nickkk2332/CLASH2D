//! [`Real`]: the deterministic number for physical magnitudes.

use core::cmp::Ordering;
use core::fmt;
use core::hash::{Hash, Hasher};
use core::iter::Sum;
use core::ops::{Add, AddAssign, Div, DivAssign, Mul, MulAssign, Neg, Sub, SubAssign};

use serde::{Deserialize, Serialize};

use crate::error::RealError;

/// A physical magnitude: energy, mass, force, pressure, velocity, a rate.
///
/// `Real` wraps an `f64` and keeps two promises (01 §4):
///
/// - **It is never NaN.** Because of that it has a total order and can be a
///   map key. Infinities are allowed.
/// - **Its results are the same bits on every supported target.** `+ - * /`
///   and [`sqrt`](Real::sqrt) are exact IEEE operations. Everything else goes
///   through the pure-Rust `libm` crate, never through the platform's C
///   library.
///
/// # The NaN fallback
///
/// Where IEEE arithmetic would give NaN (`0/0`, `∞ − ∞`, `0 · ∞`, the square
/// root or logarithm of a negative number), the plain operators and functions
/// give [`Real::ZERO`] instead. Code that must tell such a case apart uses
/// the `try_` functions, which return a [`RealError`], or [`Real::new`].
///
/// ```
/// use clash_core::Real;
///
/// let energy = Real::of(1.5e6) * Real::from(4);
/// assert_eq!(energy, Real::of(6.0e6));
/// assert_eq!(Real::ZERO / Real::ZERO, Real::ZERO);
/// assert!(Real::ZERO.try_div(Real::ZERO).is_err());
/// ```
#[derive(Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(try_from = "f64", into = "f64")]
pub struct Real(f64);

impl Real {
    /// Zero.
    pub const ZERO: Real = Real(0.0);
    /// One.
    pub const ONE: Real = Real(1.0);
    /// The ratio of a circle's circumference to its diameter.
    pub const PI: Real = Real(core::f64::consts::PI);
    /// A full turn in radians: 2π.
    pub const TAU: Real = Real(core::f64::consts::TAU);
    /// Euler's number.
    pub const E: Real = Real(core::f64::consts::E);
    /// Positive infinity. Larger than every other `Real`.
    pub const INFINITY: Real = Real(f64::INFINITY);
    /// Negative infinity. Smaller than every other `Real`.
    pub const NEG_INFINITY: Real = Real(f64::NEG_INFINITY);
    /// The largest finite value.
    pub const MAX: Real = Real(f64::MAX);
    /// The smallest (most negative) finite value.
    pub const MIN: Real = Real(f64::MIN);
    /// The difference between one and the next larger representable value.
    pub const EPSILON: Real = Real(f64::EPSILON);

    /// Makes a `Real`, refusing NaN.
    ///
    /// ```
    /// use clash_core::{Real, RealError};
    ///
    /// assert_eq!(Real::new(2.5).unwrap().get(), 2.5);
    /// assert_eq!(Real::new(f64::NAN), Err(RealError::NotANumber));
    /// ```
    pub const fn new(value: f64) -> Result<Real, RealError> {
        if value.is_nan() {
            Err(RealError::NotANumber)
        } else {
            Ok(Real(value))
        }
    }

    /// Makes a `Real`, turning NaN into zero (the NaN fallback).
    ///
    /// This is the way to write a constant or a literal:
    ///
    /// ```
    /// use clash_core::Real;
    ///
    /// const STANDARD_GRAVITY: Real = Real::of(9.806_65);
    /// assert_eq!(Real::of(f64::NAN), Real::ZERO);
    /// ```
    pub const fn of(value: f64) -> Real {
        if value.is_nan() {
            Real::ZERO
        } else {
            Real(value)
        }
    }

    /// The value as an `f64`. Never NaN.
    pub const fn get(self) -> f64 {
        self.0
    }

    /// Converts an integer, rounding to the nearest representable value.
    /// Integers up to 2⁵³ in magnitude convert exactly.
    pub const fn from_i64(value: i64) -> Real {
        Real(value as f64)
    }

    /// Converts to an integer, discarding the fraction (rounding toward
    /// zero). Values beyond the range of `i64` give `i64::MIN` or `i64::MAX`.
    ///
    /// To round another way, do it first: `x.floor().to_i64()`.
    pub const fn to_i64(self) -> i64 {
        self.0 as i64
    }

    /// Converts to an integer, discarding the fraction (rounding toward
    /// zero). Values beyond the range of `i32` give `i32::MIN` or `i32::MAX`.
    ///
    /// To round another way, do it first: `x.floor().to_i32()`.
    pub const fn to_i32(self) -> i32 {
        self.0 as i32
    }

    /// Whether the value is neither infinity.
    pub const fn is_finite(self) -> bool {
        self.0.is_finite()
    }

    /// Division that reports what the `/` operator hides.
    ///
    /// Dividing by zero is [`RealError::DivisionByZero`]. Dividing infinity
    /// by infinity is [`RealError::Undefined`].
    pub fn try_div(self, rhs: Real) -> Result<Real, RealError> {
        if rhs.0 == 0.0 {
            return Err(RealError::DivisionByZero);
        }
        Real::new(self.0 / rhs.0).map_err(|_| RealError::Undefined)
    }

    /// The square root. Exact (correctly rounded) on every target.
    ///
    /// The square root of a negative number is zero (the NaN fallback).
    pub fn sqrt(self) -> Real {
        Real::of(self.0.sqrt())
    }

    /// The square root, or [`RealError::NegativeSqrt`] for a negative number.
    pub fn try_sqrt(self) -> Result<Real, RealError> {
        if self.0 < 0.0 {
            Err(RealError::NegativeSqrt)
        } else {
            Ok(Real(self.0.sqrt()))
        }
    }

    /// The cube root. Defined for negative numbers.
    pub fn cbrt(self) -> Real {
        Real::of(libm::cbrt(self.0))
    }

    /// Raises to a whole power by repeated multiplication.
    ///
    /// `x.powi(2)` is exactly `x * x`, and `x.powi(3)` is exactly
    /// `x * x * x`. A negative power is one over the positive power.
    pub fn powi(self, power: i32) -> Real {
        let mut result = 1.0_f64;
        let mut base = self.0;
        let mut left = power.unsigned_abs();
        while left > 0 {
            if left & 1 == 1 {
                result *= base;
            }
            left >>= 1;
            if left > 0 {
                base *= base;
            }
        }
        if power < 0 {
            Real::ONE / Real::of(result)
        } else {
            Real::of(result)
        }
    }

    /// Raises to a real power.
    ///
    /// A negative number raised to a power that is not whole is zero (the
    /// NaN fallback).
    pub fn powf(self, power: Real) -> Real {
        Real::of(libm::pow(self.0, power.0))
    }

    /// Raises to a real power, or [`RealError::UndefinedPow`] where that has
    /// no real value: a negative number to a power that is not whole, or
    /// zero to a negative power.
    pub fn try_powf(self, power: Real) -> Result<Real, RealError> {
        if self.0 == 0.0 && power.0 < 0.0 {
            return Err(RealError::UndefinedPow);
        }
        Real::new(libm::pow(self.0, power.0)).map_err(|_| RealError::UndefinedPow)
    }

    /// The exponential function, `e` to this power.
    pub fn exp(self) -> Real {
        Real::of(libm::exp(self.0))
    }

    /// The natural logarithm.
    ///
    /// The logarithm of zero is negative infinity. The logarithm of a
    /// negative number is zero (the NaN fallback).
    pub fn ln(self) -> Real {
        Real::of(libm::log(self.0))
    }

    /// The natural logarithm, or [`RealError::NonPositiveLog`] for zero or a
    /// negative number.
    pub fn try_ln(self) -> Result<Real, RealError> {
        if self.0 > 0.0 {
            Ok(Real::of(libm::log(self.0)))
        } else {
            Err(RealError::NonPositiveLog)
        }
    }

    /// The base-10 logarithm.
    ///
    /// The logarithm of zero is negative infinity. The logarithm of a
    /// negative number is zero (the NaN fallback).
    pub fn log10(self) -> Real {
        Real::of(libm::log10(self.0))
    }

    /// The base-10 logarithm, or [`RealError::NonPositiveLog`] for zero or a
    /// negative number.
    pub fn try_log10(self) -> Result<Real, RealError> {
        if self.0 > 0.0 {
            Ok(Real::of(libm::log10(self.0)))
        } else {
            Err(RealError::NonPositiveLog)
        }
    }

    /// The sine of an angle in radians. The sine of an infinity is zero (the
    /// NaN fallback).
    pub fn sin(self) -> Real {
        Real::of(libm::sin(self.0))
    }

    /// The cosine of an angle in radians. The cosine of an infinity is zero
    /// (the NaN fallback).
    pub fn cos(self) -> Real {
        Real::of(libm::cos(self.0))
    }

    /// The tangent of an angle in radians. The tangent of an infinity is
    /// zero (the NaN fallback).
    pub fn tan(self) -> Real {
        Real::of(libm::tan(self.0))
    }

    /// The angle in radians, from −π to π, of the point whose `y` is `self`
    /// and whose `x` is the argument, measured from the positive x axis.
    ///
    /// ```
    /// use clash_core::Real;
    ///
    /// let straight_up = Real::ONE.atan2(Real::ZERO);
    /// assert_eq!(straight_up, Real::of(std::f64::consts::FRAC_PI_2));
    /// ```
    pub fn atan2(self, x: Real) -> Real {
        Real::of(libm::atan2(self.0, x.0))
    }

    /// The largest whole number that is not greater than this.
    pub fn floor(self) -> Real {
        Real::of(libm::floor(self.0))
    }

    /// The smallest whole number that is not less than this.
    pub fn ceil(self) -> Real {
        Real::of(libm::ceil(self.0))
    }

    /// The nearest whole number. Halves round away from zero.
    pub fn round(self) -> Real {
        Real::of(libm::round(self.0))
    }

    /// The whole part, discarding the fraction.
    pub fn trunc(self) -> Real {
        Real::of(libm::trunc(self.0))
    }

    /// The magnitude, without the sign.
    pub const fn abs(self) -> Real {
        Real(self.0.abs())
    }

    /// `1` for a positive number, `-1` for a negative one and `0` for zero.
    pub fn signum(self) -> Real {
        if self.0 > 0.0 {
            Real::ONE
        } else if self.0 < 0.0 {
            Real(-1.0)
        } else {
            Real::ZERO
        }
    }

    /// The smaller of two values.
    pub fn min(self, other: Real) -> Real {
        if other.0 < self.0 { other } else { self }
    }

    /// The larger of two values.
    pub fn max(self, other: Real) -> Real {
        if other.0 > self.0 { other } else { self }
    }

    /// Limits the value to the range from `low` to `high`.
    ///
    /// Never panics. If `low` is greater than `high` the result is `high`.
    pub fn clamp(self, low: Real, high: Real) -> Real {
        self.max(low).min(high)
    }
}

impl Default for Real {
    fn default() -> Real {
        Real::ZERO
    }
}

impl Eq for Real {}

impl Ord for Real {
    fn cmp(&self, other: &Real) -> Ordering {
        // Neither side is NaN, so the comparison always has an answer.
        // Negative zero equals positive zero.
        self.0.partial_cmp(&other.0).unwrap_or(Ordering::Equal)
    }
}

impl PartialOrd for Real {
    fn partial_cmp(&self, other: &Real) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Hash for Real {
    fn hash<H: Hasher>(&self, state: &mut H) {
        // Equal values must hash alike, and the two zeros are equal.
        let bits = if self.0 == 0.0 { 0 } else { self.0.to_bits() };
        state.write_u64(bits);
    }
}

impl fmt::Debug for Real {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Debug::fmt(&self.0, f)
    }
}

impl fmt::Display for Real {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(&self.0, f)
    }
}

impl Add for Real {
    type Output = Real;
    fn add(self, rhs: Real) -> Real {
        Real::of(self.0 + rhs.0)
    }
}

impl Sub for Real {
    type Output = Real;
    fn sub(self, rhs: Real) -> Real {
        Real::of(self.0 - rhs.0)
    }
}

impl Mul for Real {
    type Output = Real;
    fn mul(self, rhs: Real) -> Real {
        Real::of(self.0 * rhs.0)
    }
}

/// Division by zero follows IEEE: a non-zero number over zero is an
/// infinity, and zero over zero is zero (the NaN fallback). Use
/// [`Real::try_div`] to be told instead.
impl Div for Real {
    type Output = Real;
    fn div(self, rhs: Real) -> Real {
        Real::of(self.0 / rhs.0)
    }
}

impl Neg for Real {
    type Output = Real;
    fn neg(self) -> Real {
        Real(-self.0)
    }
}

impl AddAssign for Real {
    fn add_assign(&mut self, rhs: Real) {
        *self = *self + rhs;
    }
}

impl SubAssign for Real {
    fn sub_assign(&mut self, rhs: Real) {
        *self = *self - rhs;
    }
}

impl MulAssign for Real {
    fn mul_assign(&mut self, rhs: Real) {
        *self = *self * rhs;
    }
}

impl DivAssign for Real {
    fn div_assign(&mut self, rhs: Real) {
        *self = *self / rhs;
    }
}

/// Adds from left to right. The order of the items is part of the result,
/// so sum only over sequences whose order is deterministic.
impl Sum for Real {
    fn sum<I: Iterator<Item = Real>>(iter: I) -> Real {
        iter.fold(Real::ZERO, Add::add)
    }
}

impl<'a> Sum<&'a Real> for Real {
    fn sum<I: Iterator<Item = &'a Real>>(iter: I) -> Real {
        iter.copied().sum()
    }
}

impl TryFrom<f64> for Real {
    type Error = RealError;
    fn try_from(value: f64) -> Result<Real, RealError> {
        Real::new(value)
    }
}

impl From<Real> for f64 {
    fn from(value: Real) -> f64 {
        value.0
    }
}

macro_rules! real_from_exact {
    ($($int:ty),*) => {
        $(
            impl From<$int> for Real {
                fn from(value: $int) -> Real {
                    Real(f64::from(value))
                }
            }
        )*
    };
}

real_from_exact!(i8, i16, i32, u8, u16, u32);

#[cfg(test)]
mod tests {
    use super::*;

    fn r(value: f64) -> Real {
        Real::of(value)
    }

    #[test]
    fn construction_refuses_or_replaces_nan() {
        assert_eq!(Real::new(f64::NAN), Err(RealError::NotANumber));
        assert_eq!(Real::try_from(f64::NAN), Err(RealError::NotANumber));
        assert_eq!(Real::of(f64::NAN), Real::ZERO);
        assert_eq!(Real::new(1.25).map(Real::get), Ok(1.25));
        assert_eq!(Real::new(f64::INFINITY), Ok(Real::INFINITY));
    }

    #[test]
    fn operators_never_produce_nan() {
        let inf = Real::INFINITY;
        let cases = [
            Real::ZERO / Real::ZERO,
            inf - inf,
            inf + Real::NEG_INFINITY,
            Real::ZERO * inf,
            inf / inf,
            r(-1.0).sqrt(),
            r(-1.0).ln(),
            r(-1.0).log10(),
            r(-8.0).powf(r(0.5)),
            inf.sin(),
            inf.cos(),
            inf.tan(),
        ];
        for value in cases {
            assert!(!value.get().is_nan());
            assert_eq!(value, Real::ZERO);
        }
        let mut total = inf;
        total -= inf;
        assert_eq!(total, Real::ZERO);
    }

    #[test]
    fn division_by_zero_follows_ieee_and_try_div_reports_it() {
        assert_eq!(r(1.0) / Real::ZERO, Real::INFINITY);
        assert_eq!(r(-1.0) / Real::ZERO, Real::NEG_INFINITY);
        assert_eq!(r(1.0).try_div(Real::ZERO), Err(RealError::DivisionByZero));
        assert_eq!(
            Real::ZERO.try_div(Real::ZERO),
            Err(RealError::DivisionByZero)
        );
        assert_eq!(
            Real::INFINITY.try_div(Real::INFINITY),
            Err(RealError::Undefined)
        );
        assert_eq!(r(6.0).try_div(r(4.0)), Ok(r(1.5)));
    }

    #[test]
    fn try_functions_report_inputs_outside_their_domain() {
        assert_eq!(r(-4.0).try_sqrt(), Err(RealError::NegativeSqrt));
        assert_eq!(r(4.0).try_sqrt(), Ok(r(2.0)));
        assert_eq!(Real::ZERO.try_sqrt(), Ok(Real::ZERO));
        assert_eq!(Real::ZERO.try_ln(), Err(RealError::NonPositiveLog));
        assert_eq!(r(-1.0).try_ln(), Err(RealError::NonPositiveLog));
        assert_eq!(Real::ONE.try_ln(), Ok(Real::ZERO));
        assert_eq!(Real::ZERO.try_log10(), Err(RealError::NonPositiveLog));
        assert_eq!(r(1000.0).try_log10(), Ok(r(3.0)));
        assert_eq!(r(-8.0).try_powf(r(0.5)), Err(RealError::UndefinedPow));
        assert_eq!(Real::ZERO.try_powf(r(-1.0)), Err(RealError::UndefinedPow));
        assert_eq!(r(-2.0).try_powf(r(3.0)), Ok(r(-8.0)));
        assert_eq!(r(2.0).try_powf(r(10.0)), Ok(r(1024.0)));
    }

    #[test]
    fn the_order_is_total_and_zeros_are_equal() {
        let mut values = [
            r(3.0),
            Real::INFINITY,
            r(-2.5),
            Real::ZERO,
            Real::NEG_INFINITY,
            r(-0.0),
            Real::MAX,
            Real::MIN,
        ];
        values.sort();
        let sorted: Vec<f64> = values.iter().map(|value| value.get()).collect();
        assert_eq!(
            sorted,
            [
                f64::NEG_INFINITY,
                f64::MIN,
                -2.5,
                0.0,
                0.0,
                3.0,
                f64::MAX,
                f64::INFINITY
            ]
        );
        assert_eq!(r(-0.0), Real::ZERO);
        assert_eq!(r(-0.0).cmp(&Real::ZERO), Ordering::Equal);
    }

    #[test]
    fn equal_values_hash_alike() {
        fn hash_of(value: Real) -> u64 {
            let mut hasher = crate::lookup::FixedHasher::default();
            value.hash(&mut hasher);
            hasher.finish()
        }
        assert_eq!(hash_of(r(-0.0)), hash_of(Real::ZERO));
        assert_ne!(hash_of(r(1.0)), hash_of(r(2.0)));
    }

    #[test]
    fn min_max_and_clamp_never_panic() {
        assert_eq!(r(1.0).min(r(2.0)), r(1.0));
        assert_eq!(r(1.0).max(r(2.0)), r(2.0));
        assert_eq!(r(5.0).clamp(r(0.0), r(3.0)), r(3.0));
        assert_eq!(r(-5.0).clamp(r(0.0), r(3.0)), r(0.0));
        assert_eq!(r(2.0).clamp(r(0.0), r(3.0)), r(2.0));
        // A reversed range does not panic; the upper bound wins.
        assert_eq!(r(2.0).clamp(r(3.0), r(0.0)), r(0.0));
    }

    #[test]
    fn rounding_functions() {
        assert_eq!(r(2.5).floor(), r(2.0));
        assert_eq!(r(-2.5).floor(), r(-3.0));
        assert_eq!(r(2.5).ceil(), r(3.0));
        assert_eq!(r(-2.5).ceil(), r(-2.0));
        assert_eq!(r(2.5).round(), r(3.0));
        assert_eq!(r(-2.5).round(), r(-3.0));
        assert_eq!(r(2.4).round(), r(2.0));
        assert_eq!(r(2.9).trunc(), r(2.0));
        assert_eq!(r(-2.9).trunc(), r(-2.0));
        assert_eq!(r(-2.9).abs(), r(2.9));
        assert_eq!(r(-2.9).signum(), r(-1.0));
        assert_eq!(r(2.9).signum(), Real::ONE);
        assert_eq!(Real::ZERO.signum(), Real::ZERO);
    }

    #[test]
    fn integer_conversions_saturate() {
        assert_eq!(r(2.9).to_i64(), 2);
        assert_eq!(r(-2.9).to_i64(), -2);
        assert_eq!(r(-2.9).floor().to_i32(), -3);
        assert_eq!(Real::INFINITY.to_i64(), i64::MAX);
        assert_eq!(Real::NEG_INFINITY.to_i64(), i64::MIN);
        assert_eq!(r(1e12).to_i32(), i32::MAX);
        assert_eq!(Real::from_i64(1 << 53).get(), 9_007_199_254_740_992.0);
        assert_eq!(Real::from(7_u8), r(7.0));
        assert_eq!(Real::from(-7_i32), r(-7.0));
        assert_eq!(f64::from(r(1.5)), 1.5);
    }

    #[test]
    fn powi_is_repeated_multiplication() {
        let x = r(1.1);
        assert_eq!(x.powi(0), Real::ONE);
        assert_eq!(x.powi(1), x);
        assert_eq!(x.powi(2), x * x);
        assert_eq!(x.powi(3), x * x * x);
        assert_eq!(r(2.0).powi(10), r(1024.0));
        assert_eq!(r(2.0).powi(-2), r(0.25));
        assert_eq!(r(-2.0).powi(3), r(-8.0));
        assert_eq!(r(2.0).powi(i32::MIN), Real::ZERO);
        assert_eq!(Real::ZERO.powi(-1), Real::INFINITY);
    }

    #[test]
    fn sums_add_from_left_to_right() {
        let values = [r(1.0), r(2.0), r(3.5)];
        assert_eq!(values.iter().sum::<Real>(), r(6.5));
        assert_eq!(values.into_iter().sum::<Real>(), r(6.5));
        assert_eq!(core::iter::empty::<Real>().sum::<Real>(), Real::ZERO);
    }

    #[test]
    fn serde_round_trips_and_refuses_nan() {
        for value in [r(-1.5e33), r(0.1), r(-0.0), Real::MAX, Real::EPSILON] {
            let text = serde_json::to_string(&value).unwrap();
            let back = serde_json::from_str::<Real>(&text).unwrap();
            assert_eq!(back.get().to_bits(), value.get().to_bits(), "{text}");
        }
        assert_eq!(serde_json::to_string(&r(2.5)).unwrap(), "2.5");
        assert_eq!(serde_json::from_str::<Real>("3").unwrap(), r(3.0));
        assert!(serde_json::from_str::<Real>("\"x\"").is_err());
        // The check runs for any format that can carry a NaN.
        assert!(Real::try_from(f64::NAN).is_err());
    }

    #[test]
    fn formatting_is_the_number_itself() {
        assert_eq!(format!("{}", r(2.5)), "2.5");
        assert_eq!(format!("{:?}", r(2.5)), "2.5");
        assert_eq!(format!("{:.1}", r(2.25)), "2.2");
    }
}
