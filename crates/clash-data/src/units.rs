//! Quantities with units (04 §7.1).
//!
//! A physical quantity in data is a number in SI units, or a string with a
//! unit from a fixed list: `"250 ms"`, `"70 kPa"`, `"1.5 MJ"`, `"-5 C"`.
//! A rate is a unit, or nothing, followed by `/s`, `/min`, `/h` or `/d`:
//! `"0.012 kg/s"`, `"0.15 /h"`.
//!
//! The list is fixed and case-sensitive:
//!
//! | Measures | Units |
//! |----------|-------|
//! | time | `us ms s min h d` |
//! | length | `mm cm m km` |
//! | volume | `ml L` |
//! | mass | `g kg t` |
//! | energy | `J kJ MJ GJ` |
//! | power | `W kW MW` |
//! | force | `N kN` |
//! | pressure | `Pa kPa MPa GPa` |
//! | temperature | `K C` |
//! | speed | `m/s km/h` |
//!
//! Speed is a length per time and power is an energy per time, so
//! `"5 m/s"` and `"18 km/h"` are the rate rule applied to lengths, and a
//! field that holds a power accepts `"5 J/s"` as well as `"5 W"`.

use std::fmt;

use crate::diag::closest;

/// What a unit measures, before any "per time".
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub enum Base {
    /// A plain number: only meaningful as a rate, `"0.15 /h"`.
    Count,
    /// Seconds.
    Time,
    /// Metres.
    Length,
    /// Cubic metres.
    Volume,
    /// Kilograms.
    Mass,
    /// Joules.
    Energy,
    /// Newtons.
    Force,
    /// Pascals.
    Pressure,
    /// Kelvin.
    Temperature,
}

/// The dimension of a quantity: what it measures, and whether per time.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub struct Dim {
    /// What is measured.
    pub base: Base,
    /// Whether it is a rate: that much per second.
    pub per_time: bool,
}

impl Dim {
    /// A duration, in seconds.
    pub const TIME: Dim = Dim::of(Base::Time);
    /// A length, in metres.
    pub const LENGTH: Dim = Dim::of(Base::Length);
    /// A volume, in cubic metres.
    pub const VOLUME: Dim = Dim::of(Base::Volume);
    /// A mass, in kilograms.
    pub const MASS: Dim = Dim::of(Base::Mass);
    /// An energy, in joules.
    pub const ENERGY: Dim = Dim::of(Base::Energy);
    /// A force, in newtons.
    pub const FORCE: Dim = Dim::of(Base::Force);
    /// A pressure, in pascals.
    pub const PRESSURE: Dim = Dim::of(Base::Pressure);
    /// A temperature, in kelvin.
    pub const TEMPERATURE: Dim = Dim::of(Base::Temperature);
    /// A speed, in metres per second: a length per time.
    pub const SPEED: Dim = Dim::LENGTH.rate();
    /// A power, in watts: an energy per time.
    pub const POWER: Dim = Dim::ENERGY.rate();
    /// A plain rate, per second: `"0.15 /h"`.
    pub const FREQUENCY: Dim = Dim::of(Base::Count).rate();

    /// The dimension of a plain amount of `base`.
    pub const fn of(base: Base) -> Dim {
        Dim {
            base,
            per_time: false,
        }
    }

    /// This dimension per time.
    pub const fn rate(self) -> Dim {
        Dim {
            base: self.base,
            per_time: true,
        }
    }

    /// The dimension's name in a message: `a length`, `a power`.
    pub fn name(self) -> &'static str {
        match (self.base, self.per_time) {
            (Base::Count, false) => "a plain number",
            (Base::Time, false) => "a time",
            (Base::Length, false) => "a length",
            (Base::Volume, false) => "a volume",
            (Base::Mass, false) => "a mass",
            (Base::Energy, false) => "an energy",
            (Base::Force, false) => "a force",
            (Base::Pressure, false) => "a pressure",
            (Base::Temperature, false) => "a temperature",
            (Base::Count, true) => "a rate",
            (Base::Time, true) => "a time per time",
            (Base::Length, true) => "a speed",
            (Base::Volume, true) => "a volume rate",
            (Base::Mass, true) => "a mass rate",
            (Base::Energy, true) => "a power",
            (Base::Force, true) => "a force rate",
            (Base::Pressure, true) => "a pressure rate",
            (Base::Temperature, true) => "a temperature rate",
        }
    }

    /// The SI unit a bare number in a field of this dimension is in.
    pub fn si_unit(self) -> &'static str {
        match (self.base, self.per_time) {
            (Base::Count, false) => "",
            (Base::Time, false) => "s",
            (Base::Length, false) => "m",
            (Base::Volume, false) => "m3",
            (Base::Mass, false) => "kg",
            (Base::Energy, false) => "J",
            (Base::Force, false) => "N",
            (Base::Pressure, false) => "Pa",
            (Base::Temperature, false) => "K",
            (Base::Count, true) => "/s",
            (Base::Time, true) => "s/s",
            (Base::Length, true) => "m/s",
            (Base::Volume, true) => "m3/s",
            (Base::Mass, true) => "kg/s",
            (Base::Energy, true) => "W",
            (Base::Force, true) => "N/s",
            (Base::Pressure, true) => "Pa/s",
            (Base::Temperature, true) => "K/s",
        }
    }

    /// The units a field of this dimension accepts, for a message.
    pub fn accepted_units(self) -> String {
        let mut units: Vec<String> = Vec::new();
        for unit in UNITS {
            if unit.dim == self {
                units.push(unit.symbol.to_owned());
            }
        }
        if self.per_time {
            for unit in UNITS {
                if unit.dim == Dim::of(self.base) {
                    units.push(format!("{}/s", unit.symbol));
                }
            }
            if self.base == Base::Count {
                units.push("/s".to_owned());
            }
            units.push("(or /min, /h, /d)".to_owned());
        }
        units.join(" ")
    }
}

impl fmt::Display for Dim {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}

/// One unit of the fixed list.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Unit {
    /// How it is written. Case matters.
    pub symbol: &'static str,
    /// What it measures.
    pub dim: Dim,
    /// The SI value of one of it is `mul / div`, kept as two whole numbers
    /// so that `250 ms` is exactly `250 / 1000`.
    pub mul: f64,
    /// See `mul`.
    pub div: f64,
    /// Added after scaling: 273.15 for degrees Celsius.
    pub offset: f64,
}

const fn unit(symbol: &'static str, dim: Dim, mul: f64, div: f64) -> Unit {
    Unit {
        symbol,
        dim,
        mul,
        div,
        offset: 0.0,
    }
}

/// Kelvin at zero degrees Celsius.
pub const CELSIUS_OFFSET: f64 = 273.15;

/// The fixed list of units (04 §7.1).
pub const UNITS: &[Unit] = &[
    unit("us", Dim::TIME, 1.0, 1.0e6),
    unit("ms", Dim::TIME, 1.0, 1.0e3),
    unit("s", Dim::TIME, 1.0, 1.0),
    unit("min", Dim::TIME, 60.0, 1.0),
    unit("h", Dim::TIME, 3600.0, 1.0),
    unit("d", Dim::TIME, 86_400.0, 1.0),
    unit("mm", Dim::LENGTH, 1.0, 1.0e3),
    unit("cm", Dim::LENGTH, 1.0, 1.0e2),
    unit("m", Dim::LENGTH, 1.0, 1.0),
    unit("km", Dim::LENGTH, 1.0e3, 1.0),
    unit("ml", Dim::VOLUME, 1.0, 1.0e6),
    unit("L", Dim::VOLUME, 1.0, 1.0e3),
    unit("g", Dim::MASS, 1.0, 1.0e3),
    unit("kg", Dim::MASS, 1.0, 1.0),
    unit("t", Dim::MASS, 1.0e3, 1.0),
    unit("J", Dim::ENERGY, 1.0, 1.0),
    unit("kJ", Dim::ENERGY, 1.0e3, 1.0),
    unit("MJ", Dim::ENERGY, 1.0e6, 1.0),
    unit("GJ", Dim::ENERGY, 1.0e9, 1.0),
    unit("W", Dim::POWER, 1.0, 1.0),
    unit("kW", Dim::POWER, 1.0e3, 1.0),
    unit("MW", Dim::POWER, 1.0e6, 1.0),
    unit("N", Dim::FORCE, 1.0, 1.0),
    unit("kN", Dim::FORCE, 1.0e3, 1.0),
    unit("Pa", Dim::PRESSURE, 1.0, 1.0),
    unit("kPa", Dim::PRESSURE, 1.0e3, 1.0),
    unit("MPa", Dim::PRESSURE, 1.0e6, 1.0),
    unit("GPa", Dim::PRESSURE, 1.0e9, 1.0),
    unit("K", Dim::TEMPERATURE, 1.0, 1.0),
    Unit {
        symbol: "C",
        dim: Dim::TEMPERATURE,
        mul: 1.0,
        div: 1.0,
        offset: CELSIUS_OFFSET,
    },
];

/// The units that may follow the slash of a rate, with their length in
/// seconds.
pub const RATE_UNITS: &[(&str, f64)] = &[("s", 1.0), ("min", 60.0), ("h", 3600.0), ("d", 86_400.0)];

/// Microseconds in one second, as a float, for durations.
const MICROS_PER_SECOND: f64 = 1.0e6;

/// A quantity read from a string such as `"250 ms"`.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Quantity {
    /// The magnitude in SI units.
    pub value: f64,
    /// What it measures.
    pub dim: Dim,
    /// The number as written, before the unit was applied.
    pub number: f64,
    /// The unit as written, or `None` for a plain rate such as `"0.15 /h"`.
    pub unit: Option<&'static Unit>,
}

impl Quantity {
    /// A duration as whole microseconds, rounded to the nearest. `None`
    /// if this is not a time or does not fit.
    ///
    /// The number is multiplied by the unit's length in microseconds
    /// directly, so `"250 ms"` and `"1.1 s"` come out exact.
    pub fn as_micros(&self) -> Option<i64> {
        if self.dim != Dim::TIME {
            return None;
        }
        let unit = self.unit?;
        micros_from(self.number * (unit.mul * MICROS_PER_SECOND / unit.div))
    }
}

/// A number of seconds as whole microseconds, rounded to the nearest, or
/// `None` if it does not fit.
pub fn micros_from_seconds(seconds: f64) -> Option<i64> {
    micros_from(seconds * MICROS_PER_SECOND)
}

fn micros_from(micros: f64) -> Option<i64> {
    let rounded = micros.round();
    // 2^63 is the first value that does not fit; it is exact as a float.
    const LIMIT: f64 = 9_223_372_036_854_775_808.0;
    if (-LIMIT..LIMIT).contains(&rounded) {
        Some(rounded as i64)
    } else {
        None
    }
}

/// Why a string is not a quantity.
#[derive(Clone, PartialEq, Debug)]
pub enum QuantityError {
    /// The string does not start with a number.
    NoNumber,
    /// The number is too large to represent.
    NumberTooLarge,
    /// The unit is not in the list.
    UnknownUnit {
        /// The unit as written.
        unit: String,
        /// The closest unit in the list, if one is close.
        closest: Option<&'static str>,
    },
    /// What follows the slash is not `s`, `min`, `h` or `d`.
    BadRateUnit {
        /// What was written after the slash.
        unit: String,
    },
    /// The unit before the slash is already a rate: `"5 W/s"`.
    RateOfRate {
        /// The unit before the slash.
        unit: &'static str,
    },
}

impl fmt::Display for QuantityError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            QuantityError::NoNumber => {
                f.write_str("a quantity starts with a number, as in \"250 ms\"")
            }
            QuantityError::NumberTooLarge => f.write_str("the number is too large to represent"),
            QuantityError::UnknownUnit { unit, .. } => write!(f, "unknown unit \"{unit}\""),
            QuantityError::BadRateUnit { unit } => write!(
                f,
                "a rate is per `s`, `min`, `h` or `d`, not per \"{unit}\""
            ),
            QuantityError::RateOfRate { unit } => {
                write!(
                    f,
                    "\"{unit}\" is already a rate and cannot be per time again"
                )
            }
        }
    }
}

/// Reads a quantity from the text of a string value.
///
/// ```
/// use clash_data::units::{parse_quantity, Dim};
///
/// let wait = parse_quantity("250 ms").unwrap();
/// assert_eq!((wait.value, wait.dim), (0.25, Dim::TIME));
/// assert_eq!(wait.as_micros(), Some(250_000));
///
/// assert_eq!(parse_quantity("1.5 MJ").unwrap().value, 1.5e6);
/// assert_eq!(parse_quantity("-5 C").unwrap().value, 268.15);
/// assert_eq!(parse_quantity("18 km/h").unwrap().value, 5.0);
/// assert_eq!(parse_quantity("0.15 /h").unwrap().dim, Dim::FREQUENCY);
/// assert!(parse_quantity("3 furlongs").is_err());
/// ```
pub fn parse_quantity(text: &str) -> Result<Quantity, QuantityError> {
    let text = text.trim();
    let number_len = number_prefix(text);
    if number_len == 0 {
        return Err(QuantityError::NoNumber);
    }
    let number: f64 = text[..number_len]
        .parse()
        .map_err(|_| QuantityError::NoNumber)?;
    if !number.is_finite() {
        return Err(QuantityError::NumberTooLarge);
    }
    let rest = text[number_len..].trim();
    let (symbol, per) = match rest.split_once('/') {
        Some((symbol, per)) => (symbol.trim(), Some(per.trim())),
        None => (rest, None),
    };
    let unit = if symbol.is_empty() {
        None
    } else {
        Some(find_unit(symbol)?)
    };
    let mut value = number;
    let mut dim = Dim::of(Base::Count);
    if let Some(unit) = unit {
        value = number * unit.mul / unit.div;
        dim = unit.dim;
        // A temperature per time is a difference, so the offset of
        // degrees Celsius does not apply to it.
        if per.is_none() {
            value += unit.offset;
        }
    }
    if let Some(per) = per {
        let Some(&(_, seconds)) = RATE_UNITS.iter().find(|(name, _)| *name == per) else {
            return Err(QuantityError::BadRateUnit {
                unit: per.to_owned(),
            });
        };
        if dim.per_time {
            return Err(QuantityError::RateOfRate {
                unit: unit.map_or("", |unit| unit.symbol),
            });
        }
        value /= seconds;
        dim = dim.rate();
    } else if unit.is_none() {
        // A bare number inside a string: `"12"`. Not a quantity.
        return Err(QuantityError::UnknownUnit {
            unit: String::new(),
            closest: None,
        });
    }
    if !value.is_finite() {
        return Err(QuantityError::NumberTooLarge);
    }
    Ok(Quantity {
        value,
        dim,
        number,
        unit,
    })
}

fn find_unit(symbol: &str) -> Result<&'static Unit, QuantityError> {
    if let Some(unit) = UNITS.iter().find(|unit| unit.symbol == symbol) {
        return Ok(unit);
    }
    // Units are case-sensitive; a different case is the commonest slip.
    let by_case = UNITS
        .iter()
        .find(|unit| unit.symbol.eq_ignore_ascii_case(symbol))
        .map(|unit| unit.symbol);
    Err(QuantityError::UnknownUnit {
        unit: symbol.to_owned(),
        closest: by_case.or_else(|| closest(symbol, UNITS.iter().map(|unit| unit.symbol))),
    })
}

/// The length of the number at the start of `text`: an optional sign,
/// digits with an optional fraction, and an optional exponent.
fn number_prefix(text: &str) -> usize {
    let bytes = text.as_bytes();
    let mut at = 0;
    if matches!(bytes.first(), Some(b'+' | b'-')) {
        at += 1;
    }
    let digits_start = at;
    while bytes.get(at).is_some_and(u8::is_ascii_digit) {
        at += 1;
    }
    let mut digits = at - digits_start;
    if bytes.get(at) == Some(&b'.') {
        let fraction_start = at + 1;
        let mut end = fraction_start;
        while bytes.get(end).is_some_and(u8::is_ascii_digit) {
            end += 1;
        }
        if end > fraction_start {
            digits += end - fraction_start;
            at = end;
        }
    }
    if digits == 0 {
        return 0;
    }
    if matches!(bytes.get(at), Some(b'e' | b'E')) {
        let mut end = at + 1;
        if matches!(bytes.get(end), Some(b'+' | b'-')) {
            end += 1;
        }
        let exponent_start = end;
        while bytes.get(end).is_some_and(u8::is_ascii_digit) {
            end += 1;
        }
        if end > exponent_start {
            at = end;
        }
    }
    at
}

#[cfg(test)]
mod tests {
    use super::*;

    fn value(text: &str) -> f64 {
        parse_quantity(text).unwrap().value
    }

    fn dim(text: &str) -> Dim {
        parse_quantity(text).unwrap().dim
    }

    #[test]
    fn every_unit_of_the_list_converts_to_si() {
        let cases: &[(&str, f64, Dim)] = &[
            ("1 us", 1.0e-6, Dim::TIME),
            ("250 ms", 0.25, Dim::TIME),
            ("3 s", 3.0, Dim::TIME),
            ("2 min", 120.0, Dim::TIME),
            ("2 h", 7200.0, Dim::TIME),
            ("1 d", 86_400.0, Dim::TIME),
            ("5 mm", 0.005, Dim::LENGTH),
            ("20 cm", 0.2, Dim::LENGTH),
            ("3 m", 3.0, Dim::LENGTH),
            ("1.5 km", 1500.0, Dim::LENGTH),
            ("500 ml", 0.0005, Dim::VOLUME),
            ("2 L", 0.002, Dim::VOLUME),
            ("500 g", 0.5, Dim::MASS),
            ("80 kg", 80.0, Dim::MASS),
            ("2 t", 2000.0, Dim::MASS),
            ("60 J", 60.0, Dim::ENERGY),
            ("2 kJ", 2000.0, Dim::ENERGY),
            ("1.5 MJ", 1.5e6, Dim::ENERGY),
            ("4 GJ", 4.0e9, Dim::ENERGY),
            ("40 W", 40.0, Dim::POWER),
            ("2 kW", 2000.0, Dim::POWER),
            ("3 MW", 3.0e6, Dim::POWER),
            ("9 N", 9.0, Dim::FORCE),
            ("2 kN", 2000.0, Dim::FORCE),
            ("5 Pa", 5.0, Dim::PRESSURE),
            ("70 kPa", 70_000.0, Dim::PRESSURE),
            ("20 MPa", 2.0e7, Dim::PRESSURE),
            ("1 GPa", 1.0e9, Dim::PRESSURE),
            ("300 K", 300.0, Dim::TEMPERATURE),
            ("0 C", 273.15, Dim::TEMPERATURE),
            ("-5 C", 268.15, Dim::TEMPERATURE),
            ("5 m/s", 5.0, Dim::SPEED),
            ("18 km/h", 5.0, Dim::SPEED),
        ];
        for &(text, expected, expected_dim) in cases {
            let quantity = parse_quantity(text).unwrap_or_else(|error| panic!("{text}: {error}"));
            assert_eq!(quantity.value, expected, "{text}");
            assert_eq!(quantity.dim, expected_dim, "{text}");
        }
        // Every unit in the table is covered above.
        for unit in UNITS {
            assert!(
                cases.iter().any(|(text, _, _)| text
                    .split_whitespace()
                    .nth(1)
                    .is_some_and(|symbol| symbol.split('/').next() == Some(unit.symbol))),
                "unit {} has no case",
                unit.symbol
            );
        }
    }

    #[test]
    fn rates_take_any_unit_per_time() {
        assert_eq!(value("0.012 kg/s"), 0.012);
        assert_eq!(dim("0.012 kg/s"), Dim::MASS.rate());
        assert_eq!(value("0.02 K/s"), 0.02);
        assert_eq!(value("6 K/min"), 0.1);
        assert_eq!(dim("0.02 K/s"), Dim::TEMPERATURE.rate());
        // A rate of temperature is a difference: no Celsius offset.
        assert_eq!(value("2 C/s"), 2.0);
        assert_eq!(value("0.15 /h"), 0.15 / 3600.0);
        assert_eq!(dim("0.15 /h"), Dim::FREQUENCY);
        assert_eq!(value("86400 /d"), 1.0);
        assert_eq!(value("3 L/min"), 0.003 / 60.0);
        // Power is energy per time, either way round.
        assert_eq!(dim("5 J/s"), Dim::POWER);
        assert_eq!(value("3600 kJ/h"), 1000.0);
    }

    #[test]
    fn numbers_take_signs_fractions_and_exponents() {
        assert_eq!(value("2.0e4 J"), 2.0e4);
        assert_eq!(value("+3 m"), 3.0);
        assert_eq!(value("-0.5 m"), -0.5);
        assert_eq!(value("1E3 m"), 1000.0);
        assert_eq!(value("250ms"), 0.25);
        assert_eq!(value("  3   s  "), 3.0);
        assert_eq!(value("1e3 kg"), 1000.0);
    }

    #[test]
    fn durations_are_exact_in_microseconds() {
        let micros = |text: &str| parse_quantity(text).unwrap().as_micros();
        assert_eq!(micros("250 ms"), Some(250_000));
        assert_eq!(micros("1.1 s"), Some(1_100_000));
        assert_eq!(micros("1 us"), Some(1));
        assert_eq!(micros("0.4 us"), Some(0));
        assert_eq!(micros("2 h"), Some(7_200_000_000));
        assert_eq!(micros("1 d"), Some(86_400_000_000));
        assert_eq!(micros("-3 s"), Some(-3_000_000));
        assert_eq!(micros("1e300 d"), None);
        assert_eq!(micros("3 m"), None);
        assert_eq!(micros_from_seconds(0.25), Some(250_000));
        assert_eq!(micros_from_seconds(1.0e300), None);
    }

    #[test]
    fn mistakes_are_told_apart() {
        assert_eq!(parse_quantity("fast"), Err(QuantityError::NoNumber));
        assert_eq!(parse_quantity(""), Err(QuantityError::NoNumber));
        assert_eq!(parse_quantity(". m"), Err(QuantityError::NoNumber));
        assert_eq!(
            parse_quantity("3 furlongs"),
            Err(QuantityError::UnknownUnit {
                unit: "furlongs".to_owned(),
                closest: None
            })
        );
        assert_eq!(
            parse_quantity("3 kgs"),
            Err(QuantityError::UnknownUnit {
                unit: "kgs".to_owned(),
                closest: Some("kg")
            })
        );
        // Case matters, and the slip is pointed out.
        assert_eq!(
            parse_quantity("3 mj"),
            Err(QuantityError::UnknownUnit {
                unit: "mj".to_owned(),
                closest: Some("MJ")
            })
        );
        assert_eq!(
            parse_quantity("3 kg/week"),
            Err(QuantityError::BadRateUnit {
                unit: "week".to_owned()
            })
        );
        assert_eq!(
            parse_quantity("3 W/s"),
            Err(QuantityError::RateOfRate { unit: "W" })
        );
        assert!(matches!(
            parse_quantity("12"),
            Err(QuantityError::UnknownUnit { .. })
        ));
        assert_eq!(
            parse_quantity("1e999 m"),
            Err(QuantityError::NumberTooLarge)
        );
    }

    #[test]
    fn a_dimension_names_itself_and_its_units() {
        assert_eq!(Dim::LENGTH.name(), "a length");
        assert_eq!(Dim::SPEED.name(), "a speed");
        assert_eq!(Dim::POWER.to_string(), "a power");
        assert_eq!(Dim::MASS.rate().name(), "a mass rate");
        assert_eq!(Dim::LENGTH.accepted_units(), "mm cm m km");
        assert_eq!(Dim::TEMPERATURE.accepted_units(), "K C");
        assert_eq!(
            Dim::POWER.accepted_units(),
            "W kW MW J/s kJ/s MJ/s GJ/s (or /min, /h, /d)"
        );
        assert_eq!(Dim::FREQUENCY.accepted_units(), "/s (or /min, /h, /d)");
        assert_eq!(Dim::PRESSURE.si_unit(), "Pa");
        assert_eq!(Dim::SPEED.si_unit(), "m/s");
    }
}
