//! Typed placeholders for what later packages build.
//!
//! The kernel fixes where these live in the simulation's state
//! (03 §4.1, §4.2) so that saves, hashes and the packages that fill them
//! in agree on the shape from the start. Each already serialises and
//! hashes, as nothing. The package named on each replaces its body and
//! raises the save's layout version ([`SAVE_VERSION`](crate::SAVE_VERSION)).

use clash_core::define_id;

define_id! {
    /// A running action of an entity (03 §6). Given out by the action
    /// table, which package T3 builds.
    pub struct ActionId;
}

/// A rule definition (05 §9): the dense id of an `eoc`. Built by
/// package K5, in [`rule`](crate::rule).
pub use crate::rule::EocId;

/// Every running action (03 §6). Built by package T3, in
/// [`action`](crate::action).
pub use crate::action::ActionTable;

/// Every open reaction window (03 §7). Built by package T5, in
/// [`action`](crate::action).
pub use crate::action::WindowTable;

/// What the rule runtime keeps between runs: rules attached at run
/// time, when rules last ran, switched rule groups (05 §9). Built by
/// package K5, in [`rule`](crate::rule); continuations (05 §13.4) are
/// package K7's.
pub use crate::rule::RuleState;

/// An entity's channel slots (03 §6.2). Built by package T3, in
/// [`action`](crate::action).
pub use crate::action::ChannelSlots;

/// The things an entity will do next (03 §5.3). Built by package T4, in
/// [`action`](crate::action).
pub use crate::action::Plan;
