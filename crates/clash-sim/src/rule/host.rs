//! [`RuleHost`]: the seam between the rule runtime and the simulation.
//!
//! The interpreter walks compiled effects and knows nothing about what a
//! subject is or what a verb does. Everything outside a rule is asked of
//! the host: reads through [`clash_expr::Host`], which this trait
//! extends, and writes, verbs, events, time and reports through the
//! methods here. The simulation implements the trait once; a small host
//! for tests is [`testing::FakeHost`](super::testing::FakeHost).

use std::sync::Arc;

use clash_core::{Dur, LookupMap};
use clash_expr::{EventId, Handle, PropertyId, Value, VerbId};

use super::EocId;
use super::state::RuleState;
use crate::reason::Reason;

/// The default of `rules.max_steps` (05 §13.2).
pub const DEFAULT_MAX_STEPS: u64 = 100_000;
/// The default of `rules.max_call_depth` (05 §13.2).
pub const DEFAULT_MAX_CALL_DEPTH: usize = 32;
/// The default of `rules.max_queued` (05 §13.2).
pub const DEFAULT_MAX_QUEUED: usize = 1_000;
/// The default of `rules.max_value_depth` (05 §13.2).
pub const DEFAULT_MAX_VALUE_DEPTH: usize = 32;
/// The default of `rules.max_value_size` (05 §13.2).
pub const DEFAULT_MAX_VALUE_SIZE: usize = clash_expr::DEFAULT_MAX_VALUE_SIZE;

/// The settings that hold the limits, in the reserved namespace `rules`.
pub const LIMIT_SETTINGS: &[(&str, &str)] = &[
    (
        "rules.max_steps",
        "The most effects and loop iterations one run may execute. Default 100,000.",
    ),
    (
        "rules.max_call_depth",
        "How deeply rules may call each other. Default 32.",
    ),
    (
        "rules.max_queued",
        "The most rules that may be queued before the queue is next emptied. Default 1,000.",
    ),
    (
        "rules.max_value_size",
        "The most elements of a list or a map, or bytes of a text, an effect may write. Default 65,536.",
    ),
    (
        "rules.max_value_depth",
        "How deeply lists and maps may nest in a value an effect writes. Default 32.",
    ),
];

/// The limits a run is held to (05 §13.2). Exceeding one is a reported
/// fault, never a hang or a crash.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct RuleLimits {
    /// The most effects and loop iterations one run may execute,
    /// counting those of the rules it calls. `rules.max_steps`.
    pub max_steps: u64,
    /// How deeply rules may call each other. `rules.max_call_depth`.
    pub max_call_depth: usize,
    /// The most rules that may be queued before the queue is next
    /// emptied. `rules.max_queued`.
    pub max_queued: usize,
    /// The most elements of a list or a map, or bytes of a text, an
    /// effect may write. `rules.max_value_size`.
    pub max_value_size: usize,
    /// How deeply lists and maps may nest in a value an effect writes.
    /// `rules.max_value_depth`.
    pub max_value_depth: usize,
}

impl Default for RuleLimits {
    fn default() -> RuleLimits {
        RuleLimits {
            max_steps: DEFAULT_MAX_STEPS,
            max_call_depth: DEFAULT_MAX_CALL_DEPTH,
            max_queued: DEFAULT_MAX_QUEUED,
            max_value_size: DEFAULT_MAX_VALUE_SIZE,
            max_value_depth: DEFAULT_MAX_VALUE_DEPTH,
        }
    }
}

/// What is running when the host is asked for something: the rule, its
/// owner and the subjects bound where the effect is written.
#[derive(Clone, Copy, Debug)]
pub struct RunView<'a> {
    /// The rule whose effects are running. `None` for an effect list
    /// that belongs to another definition, such as an action's.
    pub rule: Option<EocId>,
    /// For an attached rule, what it is attached to: `self`.
    pub owner: Option<Handle>,
    pub(crate) names: &'a [Arc<str>],
    pub(crate) values: &'a [Value],
}

impl<'a> RunView<'a> {
    /// A view of a run that binds these subjects, for a host that calls
    /// a verb outside any rule, and for tests.
    pub fn new(names: &'a [Arc<str>], values: &'a [Value]) -> RunView<'a> {
        RunView {
            rule: None,
            owner: None,
            names,
            values,
        }
    }

    /// The value bound under a name where the effect is written: `self`,
    /// `target`, `here`, `actor`. The innermost binding, if an `as`
    /// block rebound the name.
    pub fn subject(&self, name: &str) -> Option<&'a Value> {
        let at = self.names.iter().rposition(|have| &**have == name)?;
        self.values.get(at)
    }

    /// Every bound subject with its name, in slot order.
    pub fn subjects(&self) -> impl Iterator<Item = (&'a str, &'a Value)> + 'a {
        self.names
            .iter()
            .map(|name| &**name)
            .zip(self.values.iter())
    }
}

/// A verb being called (05 §10.1, §10.7).
#[derive(Clone, Copy, Debug)]
pub struct VerbCall<'a> {
    /// The verb, by the id the registry gave it.
    pub verb: VerbId,
    /// One value for each argument the verb declares, in the order of
    /// its [`VerbSpec`](clash_expr::VerbSpec). An optional argument that
    /// was left out is null. Each was checked against its declared type
    /// where the type was known at load, and may be null all the same.
    pub args: &'a [Value],
    /// What is running.
    pub by: RunView<'a>,
}

/// An event being raised by the `emit` effect (05 §8.1, §10.5).
#[derive(Clone, Copy, Debug)]
pub struct Raise<'a> {
    /// The event type, by the id the registry gave it.
    pub event: EventId,
    /// One value for each subject the type declares, in its order.
    pub subjects: &'a [Value],
    /// One value for each field the type declares, in its order; null
    /// for a field left out.
    pub fields: &'a [Value],
    /// What is running.
    pub by: RunView<'a>,
}

/// A line a rule's `message` puts in someone's log (05 §10.6).
#[derive(Clone, Copy, Debug)]
pub struct Message<'a> {
    /// Who it is for: the reader it was rendered for.
    pub to: Handle,
    /// The line.
    pub text: &'a str,
    /// How it is to be shown; empty if the effect did not say.
    pub style: &'a str,
    /// What is running.
    pub by: RunView<'a>,
}

/// One option of a choice a rule's `ask` puts (05 §10.6).
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct OfferOption {
    /// What it says, rendered for the one who chooses.
    pub label: String,
    /// Whether it may be taken.
    pub available: bool,
    /// For an option gathered when the effect ran: the rule to run if
    /// it is chosen and the values to run it with, a map by parameter
    /// name or null. `None` for an option written in the `ask`, whose
    /// effects are found where it is written.
    pub run: Option<(EocId, Value)>,
}

/// A choice being offered by the `ask` effect, with what its options'
/// effects need to run later: where the effect is written and what was
/// bound there.
#[derive(Clone, Copy, Debug)]
pub struct Offer<'a> {
    /// Who chooses.
    pub who: Handle,
    /// The question, rendered for the one who chooses.
    pub prompt: &'a str,
    /// The options, in the order written.
    pub options: &'a [OfferOption],
    /// The origin number of the `ask` effect: from it
    /// [`ExprDefs::origin`](clash_data::ExprDefs::origin) knows the
    /// definition and the JSON path it is written at.
    pub origin: u64,
    /// The locals of the list the `ask` is written in, by slot.
    pub locals: &'a [Value],
    /// The event being handled where the `ask` is written, and its
    /// fields.
    pub event: Option<(EventId, &'a [Value])>,
    /// What is running: the rule, its owner and its subjects.
    pub by: RunView<'a>,
}

/// What a host did with a choice.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Offered {
    /// It was settled at once: the option at this position is taken,
    /// or none is.
    Taken(Option<usize>),
    /// The host keeps it: whoever chooses will be asked, and the
    /// option's effects run when they answer
    /// ([`RuleSet::resume_choice`](super::RuleSet::resume_choice)).
    Pending,
}

/// What came of raising an event.
#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct Raised {
    /// The fields, in the order the type declares them. For a hook, as
    /// its handlers left them; for a notification, as they were given.
    pub fields: Vec<Value>,
    /// For a hook that a handler cancelled, why.
    pub cancelled: Option<Reason>,
}

/// Something that went wrong while a rule ran. The effect was skipped,
/// or the run was ended, and the simulation carries on (05 §13.2,
/// invariant E8).
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct RuleFault {
    /// The stable code, one of [`RULE_REASONS`](super::RULE_REASONS).
    pub code: &'static str,
    /// What happened, as a sentence.
    pub message: String,
    /// The origin number of the effect: from it
    /// [`RuleSet::fault_reason`](super::RuleSet::fault_reason) and
    /// [`RuleSet::fault_diagnostic`](super::RuleSet::fault_diagnostic)
    /// know the definition, the JSON path and the place in the file.
    /// Zero for a fault with no effect behind it.
    pub origin: u64,
    /// The rule that was running, if one was.
    pub rule: Option<EocId>,
    /// The reason the host gave, for a write, a verb or an event it
    /// refused.
    pub refusal: Option<Reason>,
}

/// What the rule runtime needs from the simulation, and nothing more.
///
/// Reads go through [`clash_expr::Host`]: properties, variables, the
/// functions the simulation registered, the random stream and run-time
/// warnings from expressions. The interpreter answers `subject`, `local`
/// and `event_field` itself from the run in progress, so a host may give
/// null for those three.
///
/// None of these may panic on content. A write, a verb or an event the
/// simulation declines is refused with a [`Reason`], which the
/// interpreter reports as a [`RuleFault`] before it carries on.
///
/// The interpreter never holds the [`RuleState`] across another call to
/// the host, so a host may run rules from inside any of these (a hook
/// raised by `emit` is delivered at once).
pub trait RuleHost: clash_expr::Host {
    /// What the rule runtime keeps between runs: attachments, when each
    /// rule last ran, switched groups, and the queue.
    fn rule_state(&mut self) -> &mut RuleState;

    /// The limits runs are held to: the settings of [`LIMIT_SETTINGS`].
    fn rule_limits(&self) -> RuleLimits {
        RuleLimits::default()
    }

    /// The local time of the clock a subject is on, or universal time
    /// for `None`: what `cooldown` is measured against (05 §9.1).
    fn local_now(&mut self, owner: Option<Handle>) -> Dur;

    /// The `global` subject, which a rule group's `enabled` reads; null
    /// if the simulation has none.
    fn global(&mut self) -> Value {
        Value::Null
    }

    /// Whether a subject is being simulated. The rules attached to one
    /// that is stored do not run (05 §13.5).
    fn simulated(&mut self, subject: Handle) -> bool {
        let _ = subject;
        true
    }

    /// Adds to `out` the rules a subject carries through its
    /// definitions: the `rules` of its area kind, its template, its
    /// traits (05 §9.3). Rules attached at run time are kept by the
    /// [`RuleState`] and are not the host's to list.
    fn rules_of(&mut self, subject: Handle, out: &mut Vec<EocId>) {
        let _ = (subject, out);
    }

    /// Told what is running just before each of the three writes
    /// below, which carry only what is written: a host that says who
    /// caused a change reads the rule, its owner and its subjects here.
    fn writing(&mut self, by: &RunView<'_>) {
        let _ = by;
    }

    /// Sets a variable of a subject, by qualified name, through the
    /// simulation's own path for it (so that a watched variable raises
    /// `var_changed`).
    fn set_variable(&mut self, subject: Handle, name: &str, value: Value) -> Result<(), Reason>;

    /// Removes a variable of a subject, so that it reads as its default
    /// again.
    fn unset_variable(&mut self, subject: Handle, name: &str) -> Result<(), Reason>;

    /// Sets a settable property of a subject, through the same function
    /// the engine uses (01 R3).
    fn set_property(
        &mut self,
        property: PropertyId,
        subject: Handle,
        value: Value,
    ) -> Result<(), Reason>;

    /// Carries out a verb. What comes back is the verb's result, null
    /// for most, or the reason it declined.
    fn call_verb(&mut self, call: &VerbCall<'_>) -> Result<Value, Reason>;

    /// Raises an event. A notification joins the queue of the pump; a
    /// hook is delivered at once, and what comes back says whether a
    /// handler cancelled it and holds its fields as the handlers left
    /// them. A hook raised from inside too many others is skipped by
    /// the host, which reports it (05 §13.2).
    fn raise(&mut self, raise: &Raise<'_>) -> Result<Raised, Reason>;

    /// Receives a fault. The host decides, usually through a
    /// [`FaultLimiter`], whether to report it.
    fn fault(&mut self, fault: RuleFault);

    /// Says who the text about to be rendered is for, or with `None`
    /// that the rendering is over (05 §4.6). The text helpers the host
    /// answers (`name`, `the`, `they`, `verb`) read as that reader
    /// would read them.
    fn set_reader(&mut self, reader: Option<Handle>) {
        let _ = reader;
    }

    /// Delivers a line of the `message` effect to whoever shows it. A
    /// host with nowhere to show it drops it.
    fn message(&mut self, message: &Message<'_>) -> Result<(), Reason> {
        let _ = message;
        Ok(())
    }

    /// Offers a choice of the `ask` effect. A host that can ask keeps
    /// what the offer carries and answers `Pending`; one that cannot
    /// takes the first option that is available, which is what this
    /// does.
    fn offer(&mut self, offer: &Offer<'_>) -> Offered {
        Offered::Taken(offer.options.iter().position(|option| option.available))
    }

    /// Sets a timer of an `after` or an `every` (05 §10.4): keeps a
    /// continuation of where the effect is written and what was bound
    /// there, and schedules it on its owner's clock. A host without
    /// timers refuses, which is what this does.
    fn set_timer(&mut self, timer: &super::TimerSet<'_>) -> Result<(), Reason> {
        let _ = timer;
        Err(Reason::new(
            super::reason::TIMERS_UNAVAILABLE,
            "this host keeps no timers",
        ))
    }

    /// Cancels an owner's timer by name (`cancel_timer`). Gives whether
    /// there was one.
    fn cancel_timer(&mut self, owner: Option<Handle>, here: Option<Handle>, name: &str) -> bool {
        let _ = (owner, here, name);
        false
    }

    /// Starts or stops a rule's own `every` for an owner, as the rule
    /// is attached to it or detached from it (05 §9.4).
    fn recurring(&mut self, owner: Handle, rule: EocId, on: bool) {
        let _ = (owner, rule, on);
    }
}

/// How many times a site may report before its reports are thinned out.
pub const DEFAULT_FAULT_BURST: u32 = clash_expr::DEFAULT_WARNING_BURST;

/// Decides which faults are worth reporting, as
/// [`WarningLimiter`](clash_expr::WarningLimiter) does for run-time
/// warnings: a site, which is one effect and one kind of fault, reports
/// its first three and after that the occurrences whose count is a
/// power of two. It counts occurrences and not seconds, so the same run
/// reports the same faults (05 §4.5).
///
/// ```
/// use clash_sim::rule::{FaultLimiter, RuleFault};
///
/// let fault = RuleFault {
///     code: "rule.step_limit",
///     message: "the run was ended".to_owned(),
///     origin: 7,
///     rule: None,
///     refusal: None,
/// };
/// let mut limiter = FaultLimiter::new();
/// let reported: Vec<u64> = (0..20).filter_map(|_| limiter.admit(&fault)).collect();
/// assert_eq!(reported, [1, 2, 3, 4, 8, 16]);
/// ```
#[derive(Clone, Debug)]
pub struct FaultLimiter {
    burst: u32,
    counts: LookupMap<(u64, &'static str), u64>,
    suppressed: u64,
}

impl Default for FaultLimiter {
    fn default() -> FaultLimiter {
        FaultLimiter::new()
    }
}

impl FaultLimiter {
    /// A limiter that lets the first [`DEFAULT_FAULT_BURST`] faults of
    /// each site through.
    pub fn new() -> FaultLimiter {
        FaultLimiter {
            burst: DEFAULT_FAULT_BURST,
            counts: LookupMap::new(),
            suppressed: 0,
        }
    }

    /// Counts a fault. Gives `Some` if it should be reported, with how
    /// many times its site has now faulted, and `None` if it should be
    /// dropped.
    pub fn admit(&mut self, fault: &RuleFault) -> Option<u64> {
        let count = self
            .counts
            .get_or_insert_with((fault.origin, fault.code), || 0);
        *count += 1;
        let count = *count;
        if count <= u64::from(self.burst) || count.is_power_of_two() {
            Some(count)
        } else {
            self.suppressed += 1;
            None
        }
    }

    /// How many faults have been dropped.
    pub const fn suppressed(&self) -> u64 {
        self.suppressed
    }

    /// Forgets every count, for a new game or a reload of definitions.
    pub fn reset(&mut self) {
        self.counts.clear();
        self.suppressed = 0;
    }
}
