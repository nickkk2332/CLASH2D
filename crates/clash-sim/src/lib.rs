//! The CLASH2D simulation: the kernel that owns `advance`, entities and
//! their components, bodies, actions and reaction windows, combat,
//! abilities, perception, AI, the rule (EOC) runtime and the cold
//! simulation. It is a pure state machine that takes commands, advances
//! time and emits events, and it never waits on input or the wall clock.
//! Specified in `docs/03-time-and-actions.md` and documents 05 to 09, 13,
//! 15 and 16.
//!
//! What is built so far is the kernel (03 §4, §5.2, §3.4; 05 §8): the
//! part every other package of the simulation stands on.
//!
//! # Where things are
//!
//! | Need | Use |
//! |------|-----|
//! | The simulation | [`Sim::new`], [`Sim::from_parts`] |
//! | To change it from outside | [`Sim::submit`] with a [`Command`]; a refusal is a [`CommandError`] holding a [`Reason`] |
//! | To move time | [`Sim::advance`] with an [`AdvanceLimit`], which answers with an [`Advance`] |
//! | What happened | [`Sim::drain_events`]: [`SimEvent`]s, each with its tick, clock, local time, cause and subjects |
//! | To read ground truth | [`Sim::truth`]: a [`Truth`]. Never from what a character decides by (01 R11) |
//! | Entities | [`Sim::create_entity`], [`Sim::spawn`], [`Sim::remove_entity`]; [`EntityTable`], [`KernelEntity`], [`EntitySpec`] |
//! | Where an entity is | [`Place`]; changed only by [`Sim::relocate`] |
//! | Something to happen later | [`Sim::schedule_for`] an entity, [`Sim::schedule_on`] a clock; a handler per domain with [`Sim::set_wake_handler`] |
//! | Clocks | [`Sim::location_clock`], [`Sim::set_time_rate`], [`Sim::anchor_clock`], [`Sim::set_clock_rate`] |
//! | Decisions | [`Sim::request_decision`], [`Sim::answer_decision`]; [`DecisionRequest`] |
//! | Events and hooks | [`Sim::emit`], [`Sim::raise`], [`Sim::on`], [`Sim::on_subject`]; [`EventType`], [`Handler`], [`Delivery`] |
//! | The same state twice | [`Sim::state_hash`], [`Sim::hashes`] |
//! | Saving and loading | [`Sim::save`], [`Sim::load`], then [`Sim::reindex`] |
//! | Replay | [`Sim::input_log`], [`Sim::replay`] |
//! | Why something was refused | [`Reason`]; the kernel's are listed in [`KERNEL_REASONS`] |
//! | The definition types of this crate | [`register_defs`] |
//!
//! # Rules this crate keeps
//!
//! - The simulation is a pure state machine: commands in, `advance`,
//!   events out. No I/O, no wall clock, no threads inside `advance`.
//! - An entity's place changes only through [`Sim::relocate`]; cells
//!   only through the world's `WorldEdit`.
//! - Nothing iterates a hash map. Entities are visited in id order, and
//!   all randomness comes from the named streams of `Sim::rng`.
//! - Every refusal carries a [`Reason`].
//! - What a character decides by never reads a [`Truth`].

pub mod action;
pub mod advance;
pub mod body;
pub mod bus;
pub mod clock;
pub mod command;
pub mod component;
pub mod console;
pub mod decision;
pub mod entity;
pub mod event;
pub mod later;
pub mod reason;
mod sim;
pub mod stats;

pub use advance::{
    Advance, AdvanceLimit, AppRequest, DEFAULT_MAX_CASCADE, MAX_CASCADE_SETTING,
    MAX_WAKES_PER_TICK, WakeHandler,
};
pub use bus::{
    Delivery, EventDef, EventDefId, EventPayload, EventType, FieldSpec, FieldType, FieldValue,
    Handler, HandlerFn, HandlerId, MAX_HOOK_DEPTH, Raised, SUBJECT_KINDS,
};
pub use clock::ClockTarget;
pub use command::{Command, CommandError, DebugCommand, InputLog, LogEntry};
pub use decision::{DecisionKind, DecisionQueue, DecisionRequest, FreeReason};
pub use entity::{
    Control, EntitySpec, EntityTable, KernelEntity, MAX_CONTAINMENT_DEPTH, Place, Presence, Tags,
    Unplaced,
};
pub use event::{
    CAUSE_SUBJECTS, EventBody, EventInfo, KERNEL_EVENTS, SimEvent, Subject, WORLD_EVENTS, Where,
};
pub use later::{ActionId, ActionTable, ChannelSet, EocId, Plan, RuleState, WindowTable};
pub use reason::{KERNEL_REASONS, Reason, ReasonInfo, codes};
pub use sim::{LoadError, SAVE_KIND, SAVE_VERSION, Sim, SimHashes, Truth};

/// Registers every definition type the simulation owns with the loader.
/// The application and the tools call this once (04 §9).
pub fn register_defs(types: &mut clash_data::DefTypes) {
    bus::register_defs(types);
    action::register_defs(types);
    stats::register_defs(types);
    body::register_defs(types);
}
