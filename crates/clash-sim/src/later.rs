//! Typed placeholders for what later packages build.
//!
//! The kernel fixes where these live in the simulation's state
//! (03 §4.1, §4.2) so that saves, hashes and the packages that fill them
//! in agree on the shape from the start. Each already serialises and
//! hashes, as nothing. The package named on each replaces its body and
//! raises the save's layout version ([`SAVE_VERSION`](crate::SAVE_VERSION)).

use clash_core::{define_id, impl_state_hash};
use serde::{Deserialize, Serialize};

define_id! {
    /// A running action of an entity (03 §6). Given out by the action
    /// table, which package T3 builds.
    pub struct ActionId;

    /// A rule definition (05 §9): the dense id of an `eoc`. The
    /// definition type is registered by package K5.
    pub struct EocId;
}

/// Every running action (03 §6). Built by package T3.
#[derive(Clone, PartialEq, Eq, Default, Debug, Serialize, Deserialize)]
pub struct ActionTable {}
impl_state_hash!(ActionTable {});

/// Every open reaction window (03 §7). Built by package T5.
#[derive(Clone, PartialEq, Eq, Default, Debug, Serialize, Deserialize)]
pub struct WindowTable {}
impl_state_hash!(WindowTable {});

/// What the rule runtime keeps between runs: attachments made at run
/// time, continuations, cooldowns, rule groups (05 §9, §13.4). Built by
/// packages K5 and K7.
#[derive(Clone, PartialEq, Eq, Default, Debug, Serialize, Deserialize)]
pub struct RuleState {}
impl_state_hash!(RuleState {});

/// An entity's channel slots and which action holds each (03 §6.2).
/// Built by package T3; document 06 derives the slots from the body.
///
/// Not to be confused with [`ChannelSet`](crate::action::ChannelSet),
/// which is a set of channel ids: what an action definition uses.
#[derive(Clone, PartialEq, Eq, Default, Debug, Serialize, Deserialize)]
pub struct ChannelSlots {}
impl_state_hash!(ChannelSlots {});

/// The things an entity will do next (03 §5.3). Built by package T4.
#[derive(Clone, PartialEq, Eq, Default, Debug, Serialize, Deserialize)]
pub struct Plan {}
impl_state_hash!(Plan {});
