//! The rule runtime (05 §9, §10, §13): rules as definitions, effects as
//! compiled trees, and the interpreter that runs them.
//!
//! A **rule** (an `eoc`) says: when an event happens, or when it is
//! called, if a condition holds, do a list of effects. This module is
//! everything about rules that does not need a simulation: it is
//! written against a trait, [`RuleHost`], which the simulation
//! implements, and is tested against a small host,
//! [`testing::FakeHost`].
//!
//! | Part | Is |
//! |------|----|
//! | [`EocDef`], [`RuleGroupDef`], [`ReasonDef`] | The `eoc`, `rule_group` and `reason` definition types |
//! | [`Effects`] | A list of effects as a field of any definition struct |
//! | [`Block`], [`Effect`], [`EffectKind`], [`Arg`] | The compiled form the interpreter walks |
//! | [`CORE_EFFECTS`] | The effects of the rule system itself, for the reference |
//! | [`RuleSet`] | The rules of a set of definitions, indexed by event, and the functions that run them |
//! | [`RuleState`] | What is kept between runs: attachments, when rules last ran, switched groups |
//! | [`RuleHost`], [`VerbCall`], [`Raise`], [`Raised`], [`RunView`] | The seam to the simulation |
//! | [`RuleFault`], [`RULE_REASONS`], [`FaultLimiter`] | What is reported when something goes wrong while a rule runs |
//! | [`RuleLimits`], [`LIMIT_SETTINGS`] | The limits of 05 §13.2 |
//!
//! # Loading
//!
//! [`register_defs`] registers the three definition types and two
//! steps that tell the expression registry what they declare: the
//! events mods define, and each rule with parameters as a verb. A
//! definition is compiled when mods load, and a mistake in a rule is a
//! diagnostic with file, line, column and JSON path: an unknown effect
//! or argument with the closest name, an argument of the wrong type, a
//! `return` outside a rule, a write to something that cannot be set.
//!
//! # Verbs
//!
//! An effect whose key is not one of [`CORE_EFFECTS`] is a **verb**.
//! The subsystem that owns a verb registers its name and arguments in
//! the expression registry, before mods load:
//!
//! ```
//! use clash_expr::{Registry, Type, VerbSpec};
//!
//! let mut registry = Registry::with_builtins();
//! let verb = registry
//!     .add_verb(
//!         VerbSpec::new("note")
//!             .arg("text", Type::Str, "What to write down.")
//!             .optional_arg("loud", Type::Bool, "Whether to shout it.")
//!             .direct("text")
//!             .doc("Writes a line down."),
//!     )
//!     .unwrap();
//! # let _ = verb;
//! ```
//!
//! and carries it out when [`RuleHost::call_verb`] is called with the
//! id it was given. The verb receives a [`VerbCall`]: one value for
//! each argument it declared, in the order it declared them, null for
//! an optional one left out; and a [`RunView`] of what is running, from
//! which it reads the rule, its owner and the subjects bound where the
//! effect is written (`self`, `target`, `actor`). It answers with a
//! value, or with a [`Reason`](crate::Reason) if it declines, which is
//! reported as a fault. A verb goes through the same functions the
//! engine uses (05 invariant E2).
//!
//! # Binding to the simulation
//!
//! The kernel has events and hooks whose handlers are Rust closures
//! ([`Sim::on`](crate::Sim::on)). Rules join it in six steps, none of
//! which changes anything here:
//!
//! 1. **The registry.** Before mods load, the simulation's kinds,
//!    properties, functions, events and verbs are in the registry the
//!    loader is given, and [`crate::register_defs`] is called, which
//!    calls this module's. The engine's own events must be there, each
//!    with its subjects and fields, for a rule to listen for one; the
//!    events mods define are entered by this module's step.
//! 2. **`RuleHost` for the simulation.** The reads are those of
//!    `clash_expr::Host` that package K4 builds over `Sim`. The rest:
//!    `rule_state` is `&mut sim.rules`; `rule_limits` reads the
//!    settings of [`LIMIT_SETTINGS`]; `local_now` the local time of
//!    the clock the subject is on; `global` the handle of the `global`
//!    subject; `simulated` whether an entity is in the world;
//!    `rules_of` the `rules` of the subject's definitions;
//!    `set_variable`, `unset_variable` and `set_property` the setters
//!    of K4; `call_verb` a table of functions by verb id; `raise`
//!    builds an `EventPayload` and calls [`Sim::raise`](crate::Sim::raise),
//!    taking `Where` and `Cause` from the subjects of the
//!    [`RunView`]; `fault` goes through a [`FaultLimiter`] to the
//!    kernel's fault report as [`RuleSet::fault_reason`] gives it.
//! 3. **A `RuleSet`.** Built once from the definitions with
//!    [`RuleSet::new`] when the simulation is set up, and again after a
//!    load. It is not state.
//! 4. **One handler for each event type.** For every event type that
//!    [`RuleSet::listens`] for, register with
//!    `Sim::on(name, Handler::new(…))` a closure that:
//!    - makes an [`Occurrence`] from the [`Delivery`](crate::Delivery):
//!      the event's id in the registry; one value for each subject and
//!      each field its `EventSpec` declares, in that order, a subject as
//!      its handle and null for one the event does not have;
//!      `as_hook()` if `delivery.is_hook()`; `consumed` as
//!      `delivery.is_consumed()` says;
//!    - calls [`RuleSet::deliver`] with the simulation as the host;
//!    - writes the answer back: `delivery.set_field` for each slot of
//!      [`Occurrence::changed`], `delivery.consume()` if it was
//!      consumed, `delivery.cancel(reason)` if it was cancelled.
//!
//!    Global and attached rules are both found by `deliver`, so nothing
//!    is registered with `Sim::on_subject`: an attachment is state, not
//!    a handler, and needs no registering after a load. The handler's
//!    priority places rules among the engine's own handlers. Handlers
//!    are not state: the same registration is made after a load.
//! 5. **The save.** [`RuleState`] replaces the kernel's placeholder in
//!    `Sim::rules`; it serialises through the engine's codec and is
//!    already part of the `rules` hash. The kernel raises its save
//!    version. Nothing else of the rule runtime is saved.
//! 6. **Destroyed subjects.** When an entity is removed, the kernel
//!    calls [`RuleState::forget`] with its handle.
//!
//! # Timers and queries
//!
//! - Timers (05 §10.4, §13.4): `after` and `every` hand the host a
//!   [`TimerSet`] ([`RuleHost::set_timer`]); the simulation keeps a
//!   [`TimerRecord`] with a [`Continuation`] in the [`RuleState`] and
//!   runs it on its owner's clock, through [`RuleSet::resume_timer`].
//!   A rule's own `every` is a timer too, run by
//!   [`RuleSet::run_recurring`]. A host without timers refuses with
//!   [`reason::TIMERS_UNAVAILABLE`].
//! - Queries (05 §11): `for_each` over a query is compiled by the data
//!   layer ([`Items::Compiled`]) and run through the expression host,
//!   whose `query` the simulation answers (`clash_sim::query`).
//! - The world's verbs are package K8's.
//!
//! # Messages and choices
//!
//! `message` and `ask` (05 §10.6) are effects of the rule system
//! itself and not verbs, because a verb's argument is one value: it
//! cannot be a text template rendered for a reader, nor a list of
//! effects to run later. The compiler reads them into
//! [`EffectKind::Message`] and [`EffectKind::Ask`]; the interpreter
//! renders the texts between two calls of [`RuleHost::set_reader`] and
//! hands the line to [`RuleHost::message`] and the choice to
//! [`RuleHost::offer`]. A choice the host keeps is a [`PendingChoice`]
//! in the [`RuleState`]: where the `ask` is written, and the subjects,
//! locals and event fields bound there. [`RuleSet::resume_choice`] finds
//! the option's effects again from that, after a save and a load too,
//! and runs them.

mod bind;
mod compile;
mod def;
mod effects;
mod host;
mod interp;
pub mod reason;
mod state;
pub mod testing;
mod timer;
mod tree;
mod value;

pub(crate) use bind::Runtime;
pub use bind::SimRules;
pub use compile::{
    ACTION_EFFECTS, CORE_EFFECTS, EMIT_CANCELLED, EMIT_REASON, EffectInfo, MAX_BLOCK_DEPTH,
};
pub use def::{
    EOC_TYPE, EocDef, EocId, GLOBAL_SUBJECT, OWNER_SUBJECT, REASON_TYPE, RULE_GROUP_TYPE,
    ReasonDef, ReasonDefId, RuleGroupDef, RuleGroupId, RuleParam, RuleSubject, Rules,
    SubjectSource, WhileUnloaded, register_defs,
};
pub use effects::Effects;
pub use host::{
    DEFAULT_FAULT_BURST, DEFAULT_MAX_CALL_DEPTH, DEFAULT_MAX_QUEUED, DEFAULT_MAX_STEPS,
    DEFAULT_MAX_VALUE_DEPTH, DEFAULT_MAX_VALUE_SIZE, FaultLimiter, LIMIT_SETTINGS, Message, Offer,
    OfferOption, Offered, Raise, Raised, RuleFault, RuleHost, RuleLimits, RunView, StrikeCall,
    StrikeOutcome, VerbCall,
};
pub use interp::RunOutcome;
pub use reason::RULE_REASONS;
pub use state::{
    EffectRoots, EffectSite, Occurrence, PendingChoice, RuleSet, RuleState, RunRecord,
    StoredOption, SubjectKey,
};
pub use timer::{
    Continuation, MAX_MISSED_RUNS, TimerBody, TimerPlan, TimerRecord, TimerSet, TimerState, Timers,
    WhileAway, stagger,
};
pub(crate) use timer::{properties as timer_properties, timer_wake};
pub use tree::{
    Arg, Ask, AskOption, AskOptions, Block, Call, Effect, EffectKind, Emit, Items, Strike, Target,
    Timer, TimerKind,
};
pub(crate) use value::conforms;
