//! Run-time warnings (05 §4.4, §4.5) and their rate limiting.
//!
//! An expression never fails at run time. Where something goes wrong
//! (arithmetic meets a null, a number is divided by zero, a value has the
//! wrong type) the operation gives null and the machine hands the host a
//! [`RuntimeWarning`]: what went wrong, where in the source string, and
//! the origin number the expression was compiled with, from which the
//! host knows the rule and the JSON path.
//!
//! A rule that divides by zero on every tick would otherwise fill the
//! log, so the host passes each warning through a [`WarningLimiter`]
//! before reporting it.

use core::fmt;

use clash_core::LookupMap;

use crate::diag::{Code, Span, codes};

/// Why an operation gave null.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Fault {
    /// Arithmetic or a function was given null where it needs a value.
    Null,
    /// A number was divided by zero.
    DivisionByZero,
    /// A function was given a number it has no answer for. The text says
    /// which: "the square root of a negative number".
    Domain(&'static str),
    /// Integer arithmetic went past the range of a 64-bit integer.
    Overflow,
    /// A value had a type the operation does not accept.
    Type {
        /// What was needed: "a number".
        wanted: &'static str,
        /// The type name of what was given.
        found: &'static str,
    },
    /// A list, a map or a text would have grown past the size limit
    /// (05 §13.2).
    TooLarge,
    /// A random function was called and the host has no stream to draw
    /// from.
    NoRandom,
    /// A function defined in data called itself, directly or through
    /// others, deeper than calls may nest (05 §13.2). Raised by the host
    /// that runs such functions; the call gave null.
    CallDepth,
    /// Something was asked for that the engine does not offer yet. The
    /// text says what: "queries". Raised by the host; the value is null.
    Unavailable(&'static str),
}

impl Fault {
    /// The stable code of this kind of warning.
    pub const fn code(self) -> Code {
        match self {
            Fault::Null => codes::RUN_NULL_ARITHMETIC,
            Fault::DivisionByZero => codes::RUN_DIVISION_BY_ZERO,
            Fault::Domain(_) => codes::RUN_DOMAIN,
            Fault::Overflow => codes::RUN_OVERFLOW,
            Fault::Type { .. } => codes::RUN_TYPE,
            Fault::TooLarge => codes::RUN_TOO_LARGE,
            Fault::NoRandom => codes::RUN_NO_RANDOM,
            Fault::CallDepth => codes::RUN_CALL_DEPTH,
            Fault::Unavailable(_) => codes::RUN_UNAVAILABLE,
        }
    }
}

impl fmt::Display for Fault {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Fault::Null => f.write_str("a value was needed and this is null"),
            Fault::DivisionByZero => f.write_str("division by zero"),
            Fault::Domain(what) => f.write_str(what),
            Fault::Overflow => f.write_str("the integer is too large to hold"),
            Fault::Type { wanted, found } => write!(f, "{wanted} was needed and this is a {found}"),
            Fault::TooLarge => f.write_str("the value would be larger than a value may be"),
            Fault::NoRandom => f.write_str("there is no random stream to draw from"),
            Fault::CallDepth => f.write_str("functions call each other too deeply"),
            Fault::Unavailable(what) => write!(f, "{what} are not available yet"),
        }
    }
}

/// Something that went wrong while an expression ran. The operation gave
/// null (or, for a comparison or a condition, false) and evaluation
/// carried on.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct RuntimeWarning {
    /// What went wrong.
    pub fault: Fault,
    /// Where: the bytes of the source string of the operation.
    pub span: Span,
    /// The origin number the expression was compiled with
    /// ([`Context::origin`](crate::Context::origin)).
    pub origin: u64,
}

impl RuntimeWarning {
    /// The stable code of this kind of warning.
    pub const fn code(&self) -> Code {
        self.fault.code()
    }
}

/// `warning[X902]: division by zero`. The host adds the rule, the JSON
/// path and the position.
impl fmt::Display for RuntimeWarning {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "warning[{}]: {}", self.code(), self.fault)
    }
}

/// How many times a site may warn before its warnings are thinned out.
pub const DEFAULT_WARNING_BURST: u32 = 3;

/// Decides which run-time warnings are worth reporting.
///
/// A **site** is one operation of one expression: an origin, a position
/// and a kind of fault. The first few warnings from a site are let
/// through; after that only the occurrences whose count is a power of
/// two are, so a site that warns a million times is reported about
/// twenty times. The limiter counts occurrences and not seconds, because
/// the simulation has no wall clock and the same run must report the
/// same warnings.
///
/// ```
/// use clash_expr::{Fault, RuntimeWarning, Span, WarningLimiter};
///
/// let mut limiter = WarningLimiter::new();
/// let warning = RuntimeWarning { fault: Fault::DivisionByZero, span: Span::new(4, 9), origin: 7 };
/// let reported: Vec<u64> = (0..100).filter_map(|_| limiter.admit(&warning)).collect();
/// assert_eq!(reported, [1, 2, 3, 4, 8, 16, 32, 64]);
/// assert_eq!(limiter.suppressed(), 92);
/// ```
#[derive(Clone, Debug)]
pub struct WarningLimiter {
    burst: u32,
    counts: LookupMap<(u64, Span, Code), u64>,
    suppressed: u64,
}

impl Default for WarningLimiter {
    fn default() -> WarningLimiter {
        WarningLimiter::new()
    }
}

impl WarningLimiter {
    /// A limiter that lets the first [`DEFAULT_WARNING_BURST`] warnings
    /// of each site through.
    pub fn new() -> WarningLimiter {
        WarningLimiter::with_burst(DEFAULT_WARNING_BURST)
    }

    /// A limiter that lets the first `burst` warnings of each site
    /// through.
    pub fn with_burst(burst: u32) -> WarningLimiter {
        WarningLimiter {
            burst,
            counts: LookupMap::new(),
            suppressed: 0,
        }
    }

    /// Counts a warning. Gives `Some` if it should be reported, with how
    /// many times its site has now warned, and `None` if it should be
    /// dropped.
    pub fn admit(&mut self, warning: &RuntimeWarning) -> Option<u64> {
        let site = (warning.origin, warning.span, warning.code());
        let count = self.counts.get_or_insert_with(site, || 0);
        *count += 1;
        let count = *count;
        if count <= u64::from(self.burst) || count.is_power_of_two() {
            Some(count)
        } else {
            self.suppressed += 1;
            None
        }
    }

    /// How many warnings have been dropped.
    pub const fn suppressed(&self) -> u64 {
        self.suppressed
    }

    /// How many sites have warned.
    pub fn sites(&self) -> usize {
        self.counts.len()
    }

    /// Forgets every count, so that each site may warn afresh. For the
    /// start of a new game or a reload of definitions.
    pub fn reset(&mut self) {
        self.counts.clear();
        self.suppressed = 0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn warning(fault: Fault, start: u32, origin: u64) -> RuntimeWarning {
        RuntimeWarning {
            fault,
            span: Span::new(start, start + 1),
            origin,
        }
    }

    #[test]
    fn every_fault_has_a_code_and_a_message() {
        let faults = [
            (Fault::Null, "X901"),
            (Fault::DivisionByZero, "X902"),
            (
                Fault::Domain("the square root of a negative number"),
                "X903",
            ),
            (Fault::Overflow, "X904"),
            (
                Fault::Type {
                    wanted: "a number",
                    found: "string",
                },
                "X905",
            ),
            (Fault::TooLarge, "X906"),
            (Fault::NoRandom, "X907"),
        ];
        for (fault, code) in faults {
            assert_eq!(fault.code().as_str(), code);
            assert!(!fault.to_string().is_empty());
            assert!(!fault.code().explanation().is_empty());
            let warning = warning(fault, 0, 0);
            assert_eq!(warning.code(), fault.code());
            assert!(
                warning
                    .to_string()
                    .starts_with(&format!("warning[{code}]: "))
            );
        }
        assert_eq!(
            Fault::Type {
                wanted: "a number",
                found: "string"
            }
            .to_string(),
            "a number was needed and this is a string"
        );
    }

    #[test]
    fn the_faults_a_host_raises_have_codes_and_messages() {
        let faults = [
            (
                Fault::CallDepth,
                "X908",
                "functions call each other too deeply",
            ),
            (
                Fault::Unavailable("queries"),
                "X909",
                "queries are not available yet",
            ),
        ];
        for (fault, code, message) in faults {
            assert_eq!(fault.code().as_str(), code);
            assert_eq!(fault.to_string(), message);
            assert!(!fault.code().explanation().is_empty());
        }
    }

    #[test]
    fn a_site_is_let_through_at_first_and_then_thinned() {
        let mut limiter = WarningLimiter::new();
        let noisy = warning(Fault::DivisionByZero, 4, 1);
        let mut reported = Vec::new();
        for _ in 0..1_000_000 {
            if let Some(count) = limiter.admit(&noisy) {
                reported.push(count);
            }
        }
        assert_eq!(reported.len(), 21);
        assert_eq!(&reported[..6], &[1, 2, 3, 4, 8, 16]);
        assert_eq!(reported.last(), Some(&524_288));
        assert_eq!(limiter.suppressed(), 1_000_000 - 21);
        assert_eq!(limiter.sites(), 1);
    }

    #[test]
    fn sites_are_limited_separately() {
        let mut limiter = WarningLimiter::with_burst(1);
        let a = warning(Fault::DivisionByZero, 4, 1);
        // Another position, another origin and another fault are each
        // another site.
        let others = [
            warning(Fault::DivisionByZero, 5, 1),
            warning(Fault::DivisionByZero, 4, 2),
            warning(Fault::Null, 4, 1),
        ];
        assert_eq!(limiter.admit(&a), Some(1));
        assert_eq!(limiter.admit(&a), Some(2));
        assert_eq!(limiter.admit(&a), None);
        for other in &others {
            assert_eq!(limiter.admit(other), Some(1));
        }
        assert_eq!(limiter.sites(), 4);
        assert_eq!(limiter.suppressed(), 1);
        // Two domain faults at one place are one site, whatever they say.
        let mut limiter = WarningLimiter::with_burst(0);
        assert_eq!(limiter.admit(&warning(Fault::Domain("a"), 0, 0)), Some(1));
        assert_eq!(limiter.admit(&warning(Fault::Domain("b"), 0, 0)), Some(2));
        assert_eq!(limiter.admit(&warning(Fault::Domain("c"), 0, 0)), None);

        limiter.reset();
        assert_eq!(limiter.sites(), 0);
        assert_eq!(limiter.suppressed(), 0);
        assert_eq!(limiter.admit(&warning(Fault::Domain("c"), 0, 0)), Some(1));
        assert_eq!(WarningLimiter::default().admit(&a), Some(1));
    }
}
