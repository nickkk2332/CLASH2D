//! The engine's error convention, and the error types of this crate.
//!
//! # The convention
//!
//! Every crate in the workspace reports failure the same way:
//!
//! - **Failure is a `Result`.** A function that can fail because of what a
//!   mod or a caller gave it returns `Result<T, SomethingError>`. A panic
//!   means an engine bug, never bad content (01 R6). Where a harmless,
//!   defined answer exists (clamping, a fallback value), a function may take
//!   it instead of failing, and its documentation says so.
//! - **One enum per concern**, named `<Thing>Error`, derived with
//!   `thiserror`:
//!   `#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]`. Add
//!   `#[non_exhaustive]` so that a new case is not a breaking change.
//! - **Messages are for a person.** Lower case, no full stop, and they name
//!   the offending value: ``unknown clock `12` ``. A message must make sense
//!   after "error: ".
//! - **Errors are plain data.** No boxed trait objects, no backtraces and no
//!   wall-clock time, so an error can be stored, compared in a test and
//!   shown by a tool.
//! - **A higher crate wraps a lower crate's error** in a variant marked
//!   `#[from]` rather than flattening it to text.
//! - **Mistakes in mod data are diagnostics**, with file, line, column and
//!   JSON path. Those are built by `clash-data`, not by these enums.

use thiserror::Error;

/// Why a [`Real`](crate::Real) could not be made or computed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
#[non_exhaustive]
pub enum RealError {
    /// The value was NaN.
    #[error("value is not a number")]
    NotANumber,
    /// The divisor was zero.
    #[error("division by zero")]
    DivisionByZero,
    /// The square root of a negative number was asked for.
    #[error("square root of a negative number")]
    NegativeSqrt,
    /// The logarithm of zero or of a negative number was asked for.
    #[error("logarithm of a number that is not positive")]
    NonPositiveLog,
    /// A power with no real value was asked for: a negative number to a
    /// power that is not whole, or zero to a negative power.
    #[error("power is not defined for this base and exponent")]
    UndefinedPow,
    /// The result has no defined value, such as infinity over infinity.
    #[error("result is not defined")]
    Undefined,
}

/// Why a text is not a [`Hash128`](crate::Hash128).
#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[non_exhaustive]
pub enum ParseHashError {
    /// The text was not 32 characters long.
    #[error("a hash is 32 hexadecimal digits, but this has {0} characters")]
    WrongLength(usize),
    /// The text held a character that is not a hexadecimal digit.
    #[error("`{0}` is not a hexadecimal digit")]
    NotHex(char),
}

/// Why a saved state is not a usable [`Rng`](crate::Rng).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
#[non_exhaustive]
pub enum RngError {
    /// Every word of the state was zero. Such a generator would produce
    /// nothing but zeros.
    #[error("a random generator's state cannot be all zeros")]
    ZeroState,
}
