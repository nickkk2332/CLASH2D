//! Unit literals: `300ms`, `70kPa`, `2h` (05 §3, §4.7).
//!
//! A unit after a number scales it to SI units, and the result is a
//! `real`. Only the simple units of 04 §7.1 are accepted: compound ones
//! such as `m/s` are written in JSON unit strings. `C` is not accepted
//! either, because it is an offset and not a scale: `-5C` would read as
//! minus 278.15.

use core::fmt;

use clash_core::Real;

/// What a unit measures.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Quantity {
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
    /// Watts.
    Power,
    /// Newtons.
    Force,
    /// Pascals.
    Pressure,
    /// Kelvin.
    Temperature,
}

/// One row of the unit table.
#[derive(Clone, Copy, PartialEq, Debug)]
struct UnitDef {
    symbol: &'static str,
    quantity: Quantity,
    /// The number is multiplied by this…
    multiply: f64,
    /// …and divided by this. Keeping the two apart lets `250ms` be
    /// exactly a quarter of a second, which multiplying by 0.001 would
    /// not always give.
    divide: f64,
}

const fn up(symbol: &'static str, quantity: Quantity, multiply: f64) -> UnitDef {
    UnitDef {
        symbol,
        quantity,
        multiply,
        divide: 1.0,
    }
}

const fn down(symbol: &'static str, quantity: Quantity, divide: f64) -> UnitDef {
    UnitDef {
        symbol,
        quantity,
        multiply: 1.0,
        divide,
    }
}

const SECONDS_PER_MINUTE: f64 = 60.0;
const SECONDS_PER_HOUR: f64 = 3_600.0;
const SECONDS_PER_DAY: f64 = 86_400.0;

/// The simple units, as 04 §7.1 lists them.
const UNITS: &[UnitDef] = &[
    down("us", Quantity::Time, 1e6),
    down("ms", Quantity::Time, 1e3),
    up("s", Quantity::Time, 1.0),
    up("min", Quantity::Time, SECONDS_PER_MINUTE),
    up("h", Quantity::Time, SECONDS_PER_HOUR),
    up("d", Quantity::Time, SECONDS_PER_DAY),
    down("mm", Quantity::Length, 1e3),
    down("cm", Quantity::Length, 1e2),
    up("m", Quantity::Length, 1.0),
    up("km", Quantity::Length, 1e3),
    down("ml", Quantity::Volume, 1e6),
    down("L", Quantity::Volume, 1e3),
    down("g", Quantity::Mass, 1e3),
    up("kg", Quantity::Mass, 1.0),
    up("t", Quantity::Mass, 1e3),
    up("J", Quantity::Energy, 1.0),
    up("kJ", Quantity::Energy, 1e3),
    up("MJ", Quantity::Energy, 1e6),
    up("GJ", Quantity::Energy, 1e9),
    up("W", Quantity::Power, 1.0),
    up("kW", Quantity::Power, 1e3),
    up("MW", Quantity::Power, 1e6),
    up("N", Quantity::Force, 1.0),
    up("kN", Quantity::Force, 1e3),
    up("Pa", Quantity::Pressure, 1.0),
    up("kPa", Quantity::Pressure, 1e3),
    up("MPa", Quantity::Pressure, 1e6),
    up("GPa", Quantity::Pressure, 1e9),
    up("K", Quantity::Temperature, 1.0),
];

/// A unit that may follow a number in an expression.
///
/// ```
/// use clash_core::Real;
/// use clash_expr::Unit;
///
/// let ms = Unit::from_symbol("ms").unwrap();
/// assert_eq!(ms.apply(Real::of(250.0)), Real::of(0.25));
/// assert_eq!(ms.symbol(), "ms");
/// assert!(Unit::from_symbol("C").is_none());
/// assert!(Unit::from_symbol("m/s").is_none());
/// ```
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct Unit(u8);

impl Unit {
    /// The unit with this symbol, if it is one of the simple units.
    pub fn from_symbol(symbol: &str) -> Option<Unit> {
        UNITS
            .iter()
            .position(|unit| unit.symbol == symbol)
            .and_then(|index| u8::try_from(index).ok())
            .map(Unit)
    }

    fn def(self) -> &'static UnitDef {
        // The index always comes from `from_symbol` or `all`.
        UNITS.get(usize::from(self.0)).unwrap_or(&UNITS[0])
    }

    /// The unit's symbol: `ms`, `kPa`.
    pub fn symbol(self) -> &'static str {
        self.def().symbol
    }

    /// What the unit measures.
    pub fn quantity(self) -> Quantity {
        self.def().quantity
    }

    /// A number in this unit, in SI units.
    pub fn apply(self, number: Real) -> Real {
        let def = self.def();
        number * Real::of(def.multiply) / Real::of(def.divide)
    }

    /// Every unit, in the order of 04 §7.1.
    pub fn all() -> impl Iterator<Item = Unit> {
        (0..UNITS.len())
            .filter_map(|index| u8::try_from(index).ok())
            .map(Unit)
    }
}

impl fmt::Debug for Unit {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Unit({})", self.symbol())
    }
}

impl fmt::Display for Unit {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.symbol())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_unit_is_found_by_its_symbol() {
        let mut count = 0;
        for unit in Unit::all() {
            assert_eq!(Unit::from_symbol(unit.symbol()), Some(unit));
            assert_eq!(unit.to_string(), unit.symbol());
            count += 1;
        }
        assert_eq!(count, UNITS.len());
        assert_eq!(Unit::from_symbol("furlong"), None);
        assert_eq!(Unit::from_symbol(""), None);
    }

    #[test]
    fn units_scale_to_si() {
        let cases: &[(&str, f64, f64, Quantity)] = &[
            ("us", 5.0, 5e-6, Quantity::Time),
            ("ms", 300.0, 0.3, Quantity::Time),
            ("s", 2.0, 2.0, Quantity::Time),
            ("min", 2.0, 120.0, Quantity::Time),
            ("h", 2.0, 7200.0, Quantity::Time),
            ("d", 1.0, 86_400.0, Quantity::Time),
            ("mm", 5.0, 0.005, Quantity::Length),
            ("cm", 5.0, 0.05, Quantity::Length),
            ("m", 5.0, 5.0, Quantity::Length),
            ("km", 1.5, 1500.0, Quantity::Length),
            ("ml", 250.0, 0.00025, Quantity::Volume),
            ("L", 2.0, 0.002, Quantity::Volume),
            ("g", 500.0, 0.5, Quantity::Mass),
            ("kg", 70.0, 70.0, Quantity::Mass),
            ("t", 2.0, 2000.0, Quantity::Mass),
            ("J", 3.0, 3.0, Quantity::Energy),
            ("kJ", 3.0, 3e3, Quantity::Energy),
            ("MJ", 1.5, 1.5e6, Quantity::Energy),
            ("GJ", 2.0, 2e9, Quantity::Energy),
            ("W", 60.0, 60.0, Quantity::Power),
            ("kW", 2.0, 2e3, Quantity::Power),
            ("MW", 2.0, 2e6, Quantity::Power),
            ("N", 9.0, 9.0, Quantity::Force),
            ("kN", 9.0, 9e3, Quantity::Force),
            ("Pa", 7.0, 7.0, Quantity::Pressure),
            ("kPa", 70.0, 7e4, Quantity::Pressure),
            ("MPa", 2.0, 2e6, Quantity::Pressure),
            ("GPa", 2.0, 2e9, Quantity::Pressure),
            ("K", 300.0, 300.0, Quantity::Temperature),
        ];
        assert_eq!(cases.len(), UNITS.len());
        for (symbol, number, si, quantity) in cases {
            let unit = Unit::from_symbol(symbol).unwrap();
            assert_eq!(unit.apply(Real::of(*number)), Real::of(*si), "{symbol}");
            assert_eq!(unit.quantity(), *quantity, "{symbol}");
        }
    }

    #[test]
    fn offsets_and_compound_units_are_not_simple() {
        for symbol in ["C", "m/s", "km/h", "kg/s"] {
            assert_eq!(Unit::from_symbol(symbol), None, "{symbol}");
        }
    }
}
