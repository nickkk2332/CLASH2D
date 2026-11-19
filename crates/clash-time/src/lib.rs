//! Time for the CLASH2D engine: the timeline, the tree of clocks and the
//! process table.
//!
//! There are no turns. There is one [`Timeline`]: a queue of wake-ups
//! ordered by time, and everything that will happen is in it. The
//! simulation pops the earliest wake-up, runs it, and repeats
//! (`docs/03-time-and-actions.md` sections 2 and 3).
//!
//! | Need | Use |
//! |------|-----|
//! | The current time | [`Timeline::now`], [`Timeline::local_now`] |
//! | Something to happen later | [`Timeline::schedule`] with a [`Wake`], which gives a [`WakeId`] |
//! | To call it off, or ask how long is left | [`Timeline::cancel`], [`Timeline::remaining`] |
//! | To run what is due | [`Timeline::pop`], [`Timeline::peek`], [`Timeline::advance_to`] |
//! | Time that runs at another speed | [`Timeline::create_clock`], [`Timeline::set_rate`]; a rate of zero stops it |
//! | To move a thing between clocks | [`Timeline::rehome`], [`Timeline::reparent`], [`Timeline::destroy_clock`] |
//! | World activity that wakes, works and sleeps | [`ProcessTable`], [`Next`], [`ProcessId`] |
//!
//! Time is integer microseconds: [`Tick`] is a point in universal time and
//! [`Dur`] a span, usually of local time on some clock. Both come from
//! `clash-core` and are re-exported here.
//!
//! This crate knows nothing about what a wake-up means. A [`Wake`] is an
//! opaque payload, and the simulation kernel decides what to do with one
//! by its [`WakeDomain`]. Nothing here reads the wall clock.

mod error;
mod process;
mod timeline;

pub use clash_core::{Dur, Tick};
pub use error::TimeError;
pub use process::{Next, ProcessId, ProcessTable};
pub use timeline::{ClockId, Timeline, Wake, WakeDomain, WakeId};
