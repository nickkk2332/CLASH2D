//! The CLASH2D simulation: the kernel that owns `advance`, entities and
//! their components, bodies, actions and reaction windows, combat,
//! abilities, perception, AI, the rule (EOC) runtime and the cold
//! simulation. It is a pure state machine that takes commands, advances
//! time and emits events, and it never waits on input or the wall clock.
//! Specified in `docs/03-time-and-actions.md` and documents 05 to 09, 13,
//! 15 and 16.
//!
//! What is built so far is the kernel (03 §4, §5.2, §3.4; 05 §5, §6,
//! §8): the part every other package of the simulation stands on.
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
//! | Rules, effects and the interpreter | [`rule`]: [`rule::EocDef`], [`rule::Effects`], [`rule::RuleSet`], [`rule::RuleHost`] |
//! | The definition types of this crate | [`register_defs`] |
//! | Components | [`Sim::components`], [`Sim::components_mut`]; a `Physical` through [`Sim::set_shape`] and [`Sim::set_own_mass`]; see [`mass`] |
//! | Tickets, loading and unloading | [`Sim::start_session`], [`Sim::hold_ticket`], [`Sim::reconcile_activity`]; generators and store with [`Sim::with_io`]; see [`activity`] |
//! | What expressions can name | [`engine_registry`], given to the loader; kinds and handles in [`subject`]; the tables in [`registry`] |
//! | To evaluate an expression | [`Sim::host`] or [`Sim::with_random`] with a [`Bound`], through `clash_data::Runner`; [`Sim::eval`] for one |
//! | Variables | [`Sim::var`], [`Sim::set_var`]; see [`vars`] |
//! | Minds: goals, methods, thinking | [`ai`] |
//! | What `set` and `add` write | [`Sim::assign`], [`Sim::assign_to`], [`Sim::set_property`] |
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
//! - What a character decides by never reads a [`Truth`]. Every property
//!   and function this crate registers reads truth and is refused in a
//!   mind context unless it is marked safe for belief ([`registry`]).
//! - A variable changes only through [`Sim::set_var`], and a settable
//!   property only through its registered setter, which calls the
//!   function the engine itself uses.

pub mod action;
pub mod activity;
pub mod advance;
pub mod ai;
pub mod body;
pub mod bus;
pub mod clock;
pub mod command;
pub mod component;
pub mod console;
pub mod decision;
pub mod device;
pub mod entity;
pub mod event;
pub mod host;
pub mod interaction;
pub mod item;
pub mod kernel_verbs;
pub mod later;
pub mod life;
pub mod mass;
pub mod mind;
pub mod movement;
pub mod perception;
mod props;
pub mod reader;
pub mod reason;
pub mod registry;
pub mod rule;
mod sim;
pub mod stats;
pub mod subject;
pub mod template;
pub mod vars;
pub mod verb;
pub mod world_verbs;

pub use action::{ActionDefId, ChannelId, ChannelSet};
pub use activity::{Shelf, Shelved, SimStore, debug_ticket};
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
pub use decision::{
    ChoiceId, ChoiceOption, DecisionKind, DecisionQueue, DecisionRequest, FreeReason,
};
pub use entity::{
    Control, EntitySpec, EntityTable, KernelEntity, MAX_CONTAINMENT_DEPTH, Place, Presence, Tags,
    Unplaced,
};
pub use event::{
    CAUSE_SUBJECTS, EventBody, EventInfo, KERNEL_EVENTS, SimEvent, Subject, WORLD_EVENTS, Where,
};
pub use host::{Bound, MAX_HOST_WARNINGS, RULE_STREAM, SimHost};
pub use later::{ActionId, ActionTable, ChannelSlots, EocId, Plan, RuleState, WindowTable};
pub use reason::{KERNEL_REASONS, Reason, ReasonInfo, codes};
pub use registry::{
    ArgDef, Bindings, FunctionDef, PropertyDef, engine_functions, engine_properties,
    engine_registry, register_engine,
};
pub use sim::{LoadError, SAVE_KIND, SAVE_VERSION, Sim, SimHashes, Truth};
pub use subject::{Kinds, SUBJECT_KIND_DOCS};
pub use vars::{Assign, MAX_VAR_NESTING};

/// Registers every definition type the simulation owns with the loader,
/// and the step that enters in the expression registry what the engine
/// offers and the events mods define ([`registry`]). The application
/// and the tools call this once (04 §9), and give the loader
/// [`engine_registry`].
pub fn register_defs(types: &mut clash_data::DefTypes) {
    bus::register_defs(types);
    // Before the rules: the kernel's step enters a mod's events into the
    // expression registry with their cause subjects, and the rule
    // runtime's step leaves alone what is already there.
    registry::register_defs(types);
    rule::register_defs(types);
    action::register_defs(types);
    stats::register_defs(types);
    body::register_defs(types);
    template::register_defs(types);
    item::register_defs(types);
    movement::register_defs(types);
    perception::register_defs(types);
    device::register_defs(types);
    interaction::register_defs(types);
    ai::register_defs(types);
    // Last: the events of living things name the kinds the steps above
    // registered, and so do some of the engine's verbs.
    types.add_registry_step(life::events::registry_step);
}
