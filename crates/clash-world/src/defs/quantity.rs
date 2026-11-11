//! Physical quantities as mods write them: a number in SI units, or a
//! string with a unit (`"0.2 m"`, `"70 kPa"`, `"-5 C"`).
//!
//! The unit list is that of 04 §7.1, with the compound units this crate's
//! definitions need.

// replaced by clash-data quantity types in wave B2

use core::fmt;

use clash_core::Real;
use serde::de::{self, Deserializer, Visitor};

/// One unit of a quantity: its suffix, and how to reach the SI value
/// (`value * scale + offset`).
pub(crate) struct Unit {
    suffix: &'static str,
    scale: f64,
    offset: f64,
}

const fn unit(suffix: &'static str, scale: f64) -> Unit {
    Unit {
        suffix,
        scale,
        offset: 0.0,
    }
}

/// The offset of the Celsius scale from kelvin.
const CELSIUS_ZERO_K: f64 = 273.15;

const LENGTH: &[Unit] = &[
    unit("mm", 1.0e-3),
    unit("cm", 1.0e-2),
    unit("m", 1.0),
    unit("km", 1.0e3),
];
const PRESSURE: &[Unit] = &[
    unit("Pa", 1.0),
    unit("kPa", 1.0e3),
    unit("MPa", 1.0e6),
    unit("GPa", 1.0e9),
];
const DENSITY: &[Unit] = &[
    unit("kg/m3", 1.0),
    unit("g/cm3", 1.0e3),
    unit("t/m3", 1.0e3),
];
const ENERGY_DENSITY: &[Unit] = &[
    unit("J/m3", 1.0),
    unit("kJ/m3", 1.0e3),
    unit("MJ/m3", 1.0e6),
    unit("GJ/m3", 1.0e9),
    unit("Pa", 1.0),
    unit("kPa", 1.0e3),
    unit("MPa", 1.0e6),
    unit("GPa", 1.0e9),
];
const SPEED: &[Unit] = &[unit("m/s", 1.0), unit("km/h", 1.0 / 3.6)];
const ENERGY: &[Unit] = &[
    unit("J", 1.0),
    unit("kJ", 1.0e3),
    unit("MJ", 1.0e6),
    unit("GJ", 1.0e9),
];
const TEMPERATURE: &[Unit] = &[
    unit("K", 1.0),
    Unit {
        suffix: "C",
        scale: 1.0,
        offset: CELSIUS_ZERO_K,
    },
];

/// Reads a quantity written as text: a number, a space, a unit. The space
/// may be left out.
pub(crate) fn parse(text: &str, what: &str, units: &[Unit]) -> Result<Real, String> {
    let text = text.trim();
    let (number, suffix) = match text.rsplit_once(char::is_whitespace) {
        Some((number, suffix)) => (number.trim(), suffix),
        None => {
            // No space: the longest unit the text ends with.
            let found = units
                .iter()
                .filter(|unit| text.ends_with(unit.suffix))
                .max_by_key(|unit| unit.suffix.len());
            match found {
                Some(unit) => (&text[..text.len() - unit.suffix.len()], unit.suffix),
                None => (text, ""),
            }
        }
    };
    let expected = || {
        let names: Vec<&str> = units.iter().map(|unit| unit.suffix).collect();
        names.join(", ")
    };
    let Some(unit) = units.iter().find(|unit| unit.suffix == suffix) else {
        return Err(format!(
            "`{text}` is not {what}: expected a number, or a number with one of the units {}",
            expected()
        ));
    };
    let value: f64 = number.parse().map_err(|_| {
        format!(
            "`{text}` is not {what}: `{number}` is not a number (units: {})",
            expected()
        )
    })?;
    finite(value * unit.scale + unit.offset, what)
}

fn finite(value: f64, what: &str) -> Result<Real, String> {
    if value.is_finite() {
        Ok(Real::of(value))
    } else {
        Err(format!("{what} must be a finite number"))
    }
}

struct QuantityVisitor {
    what: &'static str,
    units: &'static [Unit],
}

impl Visitor<'_> for QuantityVisitor {
    type Value = Real;

    fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{} as a number in SI units or a string with a unit",
            self.what
        )
    }

    fn visit_f64<E: de::Error>(self, value: f64) -> Result<Real, E> {
        finite(value, self.what).map_err(E::custom)
    }

    fn visit_i64<E: de::Error>(self, value: i64) -> Result<Real, E> {
        Ok(Real::from_i64(value))
    }

    fn visit_u64<E: de::Error>(self, value: u64) -> Result<Real, E> {
        finite(value as f64, self.what).map_err(E::custom)
    }

    fn visit_str<E: de::Error>(self, text: &str) -> Result<Real, E> {
        parse(text, self.what, self.units).map_err(E::custom)
    }
}

macro_rules! quantity {
    ($(#[$meta:meta])* $name:ident, $what:literal, $units:ident) => {
        $(#[$meta])*
        #[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
        pub struct $name(pub Real);

        impl $name {
            /// The value in SI units.
            pub const fn get(self) -> Real {
                self.0
            }
        }

        impl<'de> serde::Deserialize<'de> for $name {
            fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<$name, D::Error> {
                deserializer
                    .deserialize_any(QuantityVisitor {
                        what: $what,
                        units: $units,
                    })
                    .map($name)
            }
        }
    };
}

quantity!(
    /// A length in metres. In data: a number, or `mm`, `cm`, `m`, `km`.
    Length,
    "a length",
    LENGTH
);
quantity!(
    /// A pressure in pascals. In data: a number, or `Pa`, `kPa`, `MPa`,
    /// `GPa`.
    Pressure,
    "a pressure",
    PRESSURE
);
quantity!(
    /// A density in kilograms per cubic metre. In data: a number, or
    /// `kg/m3`, `g/cm3`, `t/m3`.
    Density,
    "a density",
    DENSITY
);
quantity!(
    /// An energy per volume in joules per cubic metre, which is the same
    /// unit as the pascal. In data: a number, or `J/m3`, `kJ/m3`, `MJ/m3`,
    /// `GJ/m3`, or a pressure unit.
    EnergyDensity,
    "an energy per volume",
    ENERGY_DENSITY
);
quantity!(
    /// A speed in metres per second. In data: a number, or `m/s`, `km/h`.
    Speed,
    "a speed",
    SPEED
);
quantity!(
    /// An energy in joules. In data: a number, or `J`, `kJ`, `MJ`, `GJ`.
    Energy,
    "an energy",
    ENERGY
);
quantity!(
    /// A temperature in kelvin. In data: a number of kelvin, or `K`, `C`.
    Temperature,
    "a temperature",
    TEMPERATURE
);

#[cfg(test)]
mod tests {
    use super::*;

    fn pressure(json: &str) -> Result<Pressure, serde_json::Error> {
        serde_json::from_str(json)
    }

    #[test]
    fn numbers_are_si_and_strings_carry_units() {
        assert_eq!(pressure("70000").unwrap().get(), Real::of(70_000.0));
        assert_eq!(pressure("7.0e4").unwrap().get(), Real::of(70_000.0));
        assert_eq!(pressure(r#""70 kPa""#).unwrap().get(), Real::of(70_000.0));
        assert_eq!(pressure(r#""20 MPa""#).unwrap().get(), Real::of(2.0e7));
        assert_eq!(pressure(r#""20MPa""#).unwrap().get(), Real::of(2.0e7));
        assert_eq!(pressure(r#""  1.5 GPa ""#).unwrap().get(), Real::of(1.5e9));
        let length: Length = serde_json::from_str(r#""20 cm""#).unwrap();
        assert_eq!(length.get(), Real::of(0.2));
        let density: Density = serde_json::from_str(r#""2400 kg/m3""#).unwrap();
        assert_eq!(density.get(), Real::of(2400.0));
        let toughness: EnergyDensity = serde_json::from_str(r#""1 MJ/m3""#).unwrap();
        assert_eq!(toughness.get(), Real::of(1.0e6));
        let cold: Temperature = serde_json::from_str(r#""-120 C""#).unwrap();
        assert_eq!(cold.get(), Real::of(-120.0 + CELSIUS_ZERO_K));
        let speed: Speed = serde_json::from_str(r#""36 km/h""#).unwrap();
        assert_eq!(speed.get(), Real::of(36.0 / 3.6));
    }

    #[test]
    fn a_wrong_unit_is_an_error_that_lists_the_right_ones() {
        let error = pressure(r#""70 kg""#).unwrap_err().to_string();
        assert!(error.contains("is not a pressure"), "{error}");
        assert!(error.contains("kPa"), "{error}");
        assert!(pressure(r#""seventy kPa""#).is_err());
        assert!(pressure(r#""kPa""#).is_err());
        assert!(pressure("true").is_err());
        assert!(pressure(r#""1e999 Pa""#).is_err());
    }
}
