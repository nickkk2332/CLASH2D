//! The errors of this crate.

use clash_core::Real;
use thiserror::Error;

use crate::process::ProcessId;
use crate::timeline::{ClockId, WakeId};

/// Why an operation on a [`Timeline`](crate::Timeline) or a
/// [`ProcessTable`](crate::ProcessTable) was refused.
///
/// A refused operation changes nothing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
#[non_exhaustive]
pub enum TimeError {
    /// There is no clock with this id. It was never created, or it has been
    /// destroyed.
    #[error("unknown clock `{0}`")]
    UnknownClock(ClockId),
    /// There is no wake-up with this id. It was never scheduled, or it has
    /// fired or been cancelled.
    #[error("unknown wake-up `{0}`")]
    UnknownWake(WakeId),
    /// There is no process with this id, or it is being run right now.
    #[error("unknown process `{0}`")]
    UnknownProcess(ProcessId),
    /// A clock's rate must be zero or more, and finite.
    #[error("a clock's rate must be zero or more and finite, but `{0}` was given")]
    BadRate(Real),
    /// The universal clock is the root of the tree and the measure of
    /// time. Its rate cannot be set, and it cannot be moved or destroyed.
    #[error("the universal clock cannot be changed, moved or destroyed")]
    UniversalClock,
    /// The move would put a clock under itself.
    #[error(
        "clock `{clock}` cannot be placed under clock `{parent}`, which is itself or one of its descendants"
    )]
    ClockLoop {
        /// The clock that was to be moved.
        clock: ClockId,
        /// The parent it was to be moved under.
        parent: ClockId,
    },
    /// Every clock id has been used.
    #[error("no clock ids are left")]
    ClockIdsExhausted,
    /// Every process id has been used.
    #[error("no process ids are left")]
    ProcessIdsExhausted,
}
