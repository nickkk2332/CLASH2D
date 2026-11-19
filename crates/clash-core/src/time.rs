//! [`Tick`] and [`Dur`]: points and spans of simulation time.
//!
//! Time is an integer count of microseconds (01 §5.1). A [`Tick`] is a
//! point on the universal timeline. A [`Dur`] is a span of time, usually
//! local time on some clock (03 §3).
//!
//! Arithmetic saturates at the ends of the `i64` range instead of
//! overflowing, so it never panics and gives the same answer in every build
//! profile. `Tick::MAX` reads naturally as "never".
//!
//! Turning text such as `"250 ms"` into a `Dur` is the job of `clash-data`.

use core::fmt;
use core::iter::Sum;
use core::ops::{Add, AddAssign, Mul, Neg, Sub, SubAssign};

use serde::{Deserialize, Serialize};

use crate::Real;
use crate::hash::{StateHash, StateHasher};

/// Microseconds in one millisecond.
pub const MICROS_PER_MILLI: i64 = 1_000;
/// Microseconds in one second.
pub const MICROS_PER_SECOND: i64 = 1_000_000;
/// Microseconds in one minute.
pub const MICROS_PER_MINUTE: i64 = 60 * MICROS_PER_SECOND;
/// Microseconds in one hour.
pub const MICROS_PER_HOUR: i64 = 60 * MICROS_PER_MINUTE;
/// Microseconds in one day of 24 hours.
pub const MICROS_PER_DAY: i64 = 24 * MICROS_PER_HOUR;

/// A point in universal time: microseconds since the simulation began.
///
/// The range is about 292,000 years either side of zero.
///
/// ```
/// use clash_core::{Dur, Tick};
///
/// let start = Tick(2_000_000);
/// let end = start + Dur::from_millis(500);
/// assert_eq!(end, Tick(2_500_000));
/// assert_eq!(end - start, Dur::from_millis(500));
/// assert_eq!(end.to_string(), "@2.5s");
/// ```
#[derive(
    Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default, Debug, Serialize, Deserialize,
)]
#[serde(transparent)]
pub struct Tick(pub i64);

impl Tick {
    /// The start of universal time.
    pub const ZERO: Tick = Tick(0);
    /// The last representable tick. Use it to mean "never" or "no limit".
    pub const MAX: Tick = Tick(i64::MAX);

    /// The tick as a count of microseconds.
    pub const fn as_micros(self) -> i64 {
        self.0
    }

    /// The span from an earlier tick to this one. Negative if `earlier` is
    /// in fact later.
    pub const fn since(self, earlier: Tick) -> Dur {
        Dur(self.0.saturating_sub(earlier.0))
    }
}

/// A span of time in microseconds. It may be negative.
///
/// ```
/// use clash_core::{Dur, Real};
///
/// assert_eq!(Dur::from_secs(90), Dur::MINUTE + Dur::SECOND * 30);
/// assert_eq!(Dur::from_secs_real(Real::of(0.25)), Dur::from_millis(250));
/// assert_eq!(Dur::from_millis(1500).as_secs_real(), Real::of(1.5));
/// assert_eq!(Dur::from_millis(1500).to_string(), "1.5s");
/// ```
#[derive(
    Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default, Debug, Serialize, Deserialize,
)]
#[serde(transparent)]
pub struct Dur(pub i64);

impl Dur {
    /// No time at all.
    pub const ZERO: Dur = Dur(0);
    /// One microsecond: the smallest span there is.
    pub const MICROSECOND: Dur = Dur(1);
    /// One millisecond.
    pub const MILLISECOND: Dur = Dur(MICROS_PER_MILLI);
    /// One second.
    pub const SECOND: Dur = Dur(MICROS_PER_SECOND);
    /// One minute.
    pub const MINUTE: Dur = Dur(MICROS_PER_MINUTE);
    /// One hour.
    pub const HOUR: Dur = Dur(MICROS_PER_HOUR);
    /// One day of 24 hours.
    pub const DAY: Dur = Dur(MICROS_PER_DAY);
    /// The longest representable span.
    pub const MAX: Dur = Dur(i64::MAX);

    /// A span of this many microseconds.
    pub const fn from_micros(micros: i64) -> Dur {
        Dur(micros)
    }

    /// A span of this many milliseconds.
    pub const fn from_millis(millis: i64) -> Dur {
        Dur(millis.saturating_mul(MICROS_PER_MILLI))
    }

    /// A span of this many seconds.
    pub const fn from_secs(secs: i64) -> Dur {
        Dur(secs.saturating_mul(MICROS_PER_SECOND))
    }

    /// A span of this many seconds, rounded to the nearest microsecond.
    pub fn from_secs_real(secs: Real) -> Dur {
        Dur((secs * Real::from_i64(MICROS_PER_SECOND)).round().to_i64())
    }

    /// The span as a count of microseconds.
    pub const fn as_micros(self) -> i64 {
        self.0
    }

    /// The span in seconds.
    pub fn as_secs_real(self) -> Real {
        Real::from_i64(self.0) / Real::from_i64(MICROS_PER_SECOND)
    }

    /// Whether the span is no time at all.
    pub const fn is_zero(self) -> bool {
        self.0 == 0
    }

    /// Whether the span runs backwards.
    pub const fn is_negative(self) -> bool {
        self.0 < 0
    }

    /// The span multiplied by a factor, rounded to the nearest microsecond.
    pub fn mul_real(self, factor: Real) -> Dur {
        Dur((Real::from_i64(self.0) * factor).round().to_i64())
    }

    /// The span divided by a divisor, rounded to the nearest microsecond.
    ///
    /// Dividing by zero does not panic: a positive span gives [`Dur::MAX`],
    /// a negative one the most negative span, and an empty one stays empty.
    pub fn div_real(self, divisor: Real) -> Dur {
        Dur((Real::from_i64(self.0) / divisor).round().to_i64())
    }
}

impl Add<Dur> for Tick {
    type Output = Tick;
    fn add(self, rhs: Dur) -> Tick {
        Tick(self.0.saturating_add(rhs.0))
    }
}

impl Sub<Dur> for Tick {
    type Output = Tick;
    fn sub(self, rhs: Dur) -> Tick {
        Tick(self.0.saturating_sub(rhs.0))
    }
}

impl Sub for Tick {
    type Output = Dur;
    fn sub(self, rhs: Tick) -> Dur {
        self.since(rhs)
    }
}

impl AddAssign<Dur> for Tick {
    fn add_assign(&mut self, rhs: Dur) {
        *self = *self + rhs;
    }
}

impl SubAssign<Dur> for Tick {
    fn sub_assign(&mut self, rhs: Dur) {
        *self = *self - rhs;
    }
}

impl Add for Dur {
    type Output = Dur;
    fn add(self, rhs: Dur) -> Dur {
        Dur(self.0.saturating_add(rhs.0))
    }
}

impl Sub for Dur {
    type Output = Dur;
    fn sub(self, rhs: Dur) -> Dur {
        Dur(self.0.saturating_sub(rhs.0))
    }
}

impl Neg for Dur {
    type Output = Dur;
    fn neg(self) -> Dur {
        Dur(self.0.saturating_neg())
    }
}

impl Mul<i64> for Dur {
    type Output = Dur;
    fn mul(self, rhs: i64) -> Dur {
        Dur(self.0.saturating_mul(rhs))
    }
}

impl AddAssign for Dur {
    fn add_assign(&mut self, rhs: Dur) {
        *self = *self + rhs;
    }
}

impl SubAssign for Dur {
    fn sub_assign(&mut self, rhs: Dur) {
        *self = *self - rhs;
    }
}

impl Sum for Dur {
    fn sum<I: Iterator<Item = Dur>>(iter: I) -> Dur {
        iter.fold(Dur::ZERO, Add::add)
    }
}

/// Writes a count of microseconds as seconds, without trailing zeros.
fn write_seconds(f: &mut fmt::Formatter<'_>, micros: i64) -> fmt::Result {
    let sign = if micros < 0 { "-" } else { "" };
    let magnitude = micros.unsigned_abs();
    let per_second = MICROS_PER_SECOND.unsigned_abs();
    let whole = magnitude / per_second;
    let fraction = magnitude % per_second;
    if fraction == 0 {
        write!(f, "{sign}{whole}s")
    } else {
        let digits = format!("{fraction:06}");
        write!(f, "{sign}{whole}.{}s", digits.trim_end_matches('0'))
    }
}

/// Seconds after an `@`, such as `@2.5s`.
impl fmt::Display for Tick {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("@")?;
        write_seconds(f, self.0)
    }
}

/// Seconds, such as `1.5s` or `-0.000001s`.
impl fmt::Display for Dur {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write_seconds(f, self.0)
    }
}

impl StateHash for Tick {
    fn state_hash(&self, hasher: &mut StateHasher) {
        hasher.write_i64(self.0);
    }
}

impl StateHash for Dur {
    fn state_hash(&self, hasher: &mut StateHasher) {
        hasher.write_i64(self.0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Hash128;

    #[test]
    fn units_are_consistent() {
        assert_eq!(Dur::MILLISECOND, Dur::MICROSECOND * 1_000);
        assert_eq!(Dur::SECOND, Dur::from_millis(1_000));
        assert_eq!(Dur::MINUTE, Dur::from_secs(60));
        assert_eq!(Dur::HOUR, Dur::MINUTE * 60);
        assert_eq!(Dur::DAY, Dur::HOUR * 24);
        assert_eq!(Dur::from_micros(5).as_micros(), 5);
        assert_eq!(Dur::SECOND.as_micros(), 1_000_000);
    }

    #[test]
    fn ticks_and_durations_combine() {
        let tick = Tick(10) + Dur(5);
        assert_eq!(tick, Tick(15));
        assert_eq!(tick - Dur(20), Tick(-5));
        assert_eq!(Tick(15) - Tick(10), Dur(5));
        assert_eq!(Tick(10).since(Tick(15)), Dur(-5));
        let mut moving = Tick::ZERO;
        moving += Dur::SECOND;
        moving -= Dur::MILLISECOND;
        assert_eq!(moving, Tick(999_000));
        assert_eq!(moving.as_micros(), 999_000);

        let mut span = Dur(7);
        span += Dur(3);
        span -= Dur(20);
        assert_eq!(span, Dur(-10));
        assert!(span.is_negative());
        assert_eq!(-span, Dur(10));
        assert!(Dur::ZERO.is_zero());
        assert_eq!([Dur(1), Dur(2), Dur(3)].into_iter().sum::<Dur>(), Dur(6));
        assert!(Tick(1) < Tick(2));
        assert!(Dur(-1) < Dur::ZERO);
    }

    #[test]
    fn arithmetic_saturates_instead_of_overflowing() {
        assert_eq!(Tick::MAX + Dur::SECOND, Tick::MAX);
        assert_eq!(Tick(i64::MIN) - Dur::SECOND, Tick(i64::MIN));
        assert_eq!(Tick::MAX - Tick(i64::MIN), Dur::MAX);
        assert_eq!(Dur::MAX + Dur::MAX, Dur::MAX);
        assert_eq!(Dur::MAX * 2, Dur::MAX);
        assert_eq!(-Dur(i64::MIN), Dur::MAX);
        assert_eq!(Dur::from_secs(i64::MAX), Dur::MAX);
        assert_eq!(Dur::from_millis(i64::MIN), Dur(i64::MIN));
    }

    #[test]
    fn real_conversions_round_to_the_nearest_microsecond() {
        assert_eq!(Dur::from_secs_real(Real::of(1.5)), Dur(1_500_000));
        assert_eq!(Dur::from_secs_real(Real::of(0.000_000_4)), Dur(0));
        assert_eq!(Dur::from_secs_real(Real::of(0.000_000_6)), Dur(1));
        assert_eq!(Dur::from_secs_real(Real::of(-2.0)), Dur(-2_000_000));
        assert_eq!(Dur::from_secs_real(Real::INFINITY), Dur::MAX);
        assert_eq!(Dur(250_000).as_secs_real(), Real::of(0.25));
        assert_eq!(Dur::SECOND.mul_real(Real::of(0.5)), Dur(500_000));
        assert_eq!(Dur(3).mul_real(Real::of(0.5)), Dur(2));
        assert_eq!(Dur::SECOND.div_real(Real::of(4.0)), Dur(250_000));
        assert_eq!(Dur::SECOND.div_real(Real::ZERO), Dur::MAX);
        assert_eq!((-Dur::SECOND).div_real(Real::ZERO), Dur(i64::MIN));
        assert_eq!(Dur::ZERO.div_real(Real::ZERO), Dur::ZERO);
    }

    #[test]
    fn display_is_in_seconds() {
        assert_eq!(Dur::ZERO.to_string(), "0s");
        assert_eq!(Dur::SECOND.to_string(), "1s");
        assert_eq!(Dur(1_500_000).to_string(), "1.5s");
        assert_eq!(Dur(1).to_string(), "0.000001s");
        assert_eq!(Dur(-250_000).to_string(), "-0.25s");
        assert_eq!(Dur(i64::MIN).to_string(), "-9223372036854.775808s");
        assert_eq!(Tick(16_000_000).to_string(), "@16s");
        assert_eq!(Tick(-1).to_string(), "@-0.000001s");
    }

    #[test]
    fn serde_is_a_bare_number() {
        assert_eq!(serde_json::to_string(&Tick(42)).unwrap(), "42");
        assert_eq!(serde_json::to_string(&Dur(-7)).unwrap(), "-7");
        assert_eq!(serde_json::from_str::<Tick>("42").unwrap(), Tick(42));
        assert_eq!(serde_json::from_str::<Dur>("-7").unwrap(), Dur(-7));
    }

    #[test]
    fn state_hash_is_the_count() {
        assert_eq!(Hash128::of(&Tick(9)), Hash128::of(&9_i64));
        assert_eq!(Hash128::of(&Dur(-9)), Hash128::of(&-9_i64));
    }
}
