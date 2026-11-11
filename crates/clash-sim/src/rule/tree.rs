//! The compiled form of a list of effects (05 §14, step 5): a tree of
//! nodes that the interpreter walks with an explicit stack.
//!
//! A [`Block`] is a list of [`Effect`]s and knows its JSON path inside
//! its definition, so a place in a rule can be named: a continuation
//! (05 §13.4) is a rule, the path of a block and a position in it. An
//! effect knows its origin number, from which a fault or a trace finds
//! the definition, the path and the place in the file.
//!
//! The tree is immutable and lives in the definitions.

use std::sync::Arc;

use clash_data::expr::Any;
use clash_data::{Cond, Expr, Node, Span};
use clash_expr::{EventId, Place, VerbId};

use super::{EocId, ReasonDefId, RuleGroupId};

/// An argument of an effect: an expression field, evaluated when the
/// effect runs (05 §10.1).
pub type Arg = Expr<Any>;

/// A compiled list of effects, run in order.
#[derive(Clone, Debug)]
pub struct Block {
    pub(crate) effects: Arc<[Effect]>,
    pub(crate) path: Arc<str>,
}

impl Block {
    /// A block with nothing in it.
    pub fn empty() -> Block {
        Block {
            effects: Arc::from(Vec::new()),
            path: Arc::from(""),
        }
    }

    /// The effects, in the order they run.
    pub fn effects(&self) -> &[Effect] {
        &self.effects
    }

    /// The JSON path of the list inside its definition: `do`,
    /// `do[2].then`.
    pub fn path(&self) -> &str {
        &self.path
    }

    /// How many effects the block holds. A `let`, a `set`, an `add` or
    /// an `unset` of several entries is one effect for each.
    pub fn len(&self) -> usize {
        self.effects.len()
    }

    /// Whether there is nothing to run.
    pub fn is_empty(&self) -> bool {
        self.effects.is_empty()
    }
}

/// One compiled effect.
#[derive(Clone, Debug)]
pub struct Effect {
    /// What it does.
    pub kind: EffectKind,
    /// The origin number of the effect: what
    /// [`ExprDefs::origin`](clash_data::ExprDefs::origin) turns into the
    /// definition, the JSON path and the place in the file.
    pub origin: u64,
    /// Where the effect was written.
    pub span: Span,
    /// The JSON path of the effect inside its definition: `do[2]`.
    pub path: Arc<str>,
}

/// What an effect does (05 §10).
#[derive(Clone, Debug)]
#[non_exhaustive]
pub enum EffectKind {
    /// `if`: run one of two blocks.
    If {
        /// The test.
        cond: Cond,
        /// Run when it holds.
        then: Block,
        /// Run when it does not.
        otherwise: Block,
    },
    /// `first`: run the first entry whose condition holds.
    First {
        /// The entries, in order. An entry with no condition always
        /// holds.
        entries: Vec<(Option<Cond>, Block)>,
    },
    /// `switch`: run the case whose key the value reads as.
    Switch {
        /// The value branched on.
        on: Arg,
        /// The cases, by key. An id is compared by its qualified id.
        cases: Vec<(Arc<str>, Block)>,
        /// Run when no case matches.
        default: Block,
    },
    /// `for_each`: run the body once for each item.
    ForEach {
        /// What is iterated.
        items: Items,
        /// The slot of the local each item is bound to.
        slot: u16,
        /// The body.
        body: Block,
    },
    /// `repeat`: run the body a number of times.
    Repeat {
        /// How many times.
        times: Arg,
        /// The slot of the local `$i`, which counts from zero.
        slot: u16,
        /// The body.
        body: Block,
    },
    /// `while`: run the body for as long as a condition holds.
    While {
        /// The test, made before each pass.
        cond: Cond,
        /// The body.
        body: Block,
    },
    /// `weighted`: run one entry, chosen at random by weight.
    Weighted {
        /// The entries with their weights.
        entries: Vec<(Arg, Block)>,
    },
    /// `as`: bind subjects for a block.
    Rebind {
        /// The names bound and what each is bound to, evaluated before
        /// any is bound.
        binds: Vec<(Arc<str>, Arg)>,
        /// The body.
        body: Block,
    },
    /// `stop`: end this rule.
    Stop,
    /// `return`: end this rule and give a value to whoever called it.
    Return {
        /// The value; null if none is written.
        value: Option<Arg>,
    },
    /// One entry of `let`: give a local a value.
    Let {
        /// The local's slot.
        slot: u16,
        /// The value.
        value: Arg,
    },
    /// One entry of `set`, or of `modify`: assign to a place.
    Set {
        /// What is assigned to.
        place: Place,
        /// The value.
        value: Arg,
    },
    /// One entry of `add`: add to a number or append to a list.
    Add {
        /// What is added to.
        place: Place,
        /// What is added.
        value: Arg,
    },
    /// One entry of `unset`: remove a variable or an entry.
    Unset {
        /// What is removed.
        place: Place,
    },
    /// `run`, a rule used as a verb, or a `run` source in a `let`: call
    /// a rule.
    Run {
        /// Which rule, and with what.
        call: Call,
        /// The slot of the local that takes what the rule returns.
        into: Option<u16>,
    },
    /// `queue`: call a rule once what is running has finished.
    Queue {
        /// Which rule, and with what.
        call: Call,
    },
    /// `attach_rule`.
    AttachRule {
        /// What the rule is attached to.
        to: Arg,
        /// The rule.
        rule: Arg,
    },
    /// `detach_rule`.
    DetachRule {
        /// What the rule is taken from.
        to: Arg,
        /// The rule.
        rule: Arg,
    },
    /// `set_rule_group`.
    SetRuleGroup {
        /// The group.
        group: RuleGroupId,
        /// On, off, or null to leave it to the group's `enabled` again.
        on: Arg,
    },
    /// `emit`, or an `emit` source in a `let`: raise an event.
    Emit {
        /// Which event, and with what.
        emit: Emit,
        /// The slot of the local that takes the outcome.
        into: Option<u16>,
    },
    /// `cancel_event`: prevent what a hook precedes.
    CancelEvent {
        /// The reason whoever asked is told.
        reason: Option<ReasonDefId>,
        /// The values the reason carries, by name.
        values: Vec<(Arc<str>, Arg)>,
    },
    /// `consume`: mark the event handled.
    Consume,
    /// A verb a subsystem registered.
    Verb {
        /// The verb.
        verb: VerbId,
        /// One argument for each the verb declares, in its order;
        /// `None` for an optional one left out.
        args: Vec<Option<Arg>>,
    },
    /// `after`, `every` or `cancel_timer`. Read and kept; timers are
    /// built by a later package.
    Timer(Box<Timer>),
}

/// What `for_each` iterates.
#[derive(Clone, Debug)]
pub enum Items {
    /// The elements of a list.
    List(Arg),
    /// The results of a query (05 §11), kept as it was written until
    /// queries are built.
    Query(Node),
}

/// Which rule a call runs.
#[derive(Clone, Debug)]
pub enum Target {
    /// A rule named where the call is written.
    Rule(EocId),
    /// A rule whose id is worked out when the call runs.
    Computed(Arg),
}

/// A call of a rule (05 §9.5).
#[derive(Clone, Debug)]
pub struct Call {
    /// The rule.
    pub rule: Target,
    /// The arguments, by parameter name, in the order written.
    pub with: Vec<(Arc<str>, Arg)>,
}

/// An event to raise (05 §8.1).
#[derive(Clone, Debug)]
pub struct Emit {
    /// The event type.
    pub event: EventId,
    /// One argument for each subject the type declares, in its order.
    pub subjects: Vec<Option<Arg>>,
    /// One argument for each field the type declares, in its order;
    /// `None` for one left out, which is raised as null.
    pub fields: Vec<Option<Arg>>,
}

/// A timer effect (05 §10.4), read and kept.
#[derive(Clone, Debug)]
pub struct Timer {
    /// Which of the three.
    pub kind: TimerKind,
    /// The timer's name, if it has one.
    pub name: Option<Arg>,
    /// Whose clock it runs on, if said.
    pub owner: Option<Arg>,
    /// What happens while the owner is not simulated, as written.
    pub while_unloaded: Option<String>,
    /// The effects to run when it fires.
    pub body: Block,
}

/// The three timer effects.
#[derive(Clone, Debug)]
pub enum TimerKind {
    /// `after`: run once, later.
    After {
        /// How much later, in seconds of the owner's clock.
        delay: Arg,
    },
    /// `every`: run repeatedly.
    Every {
        /// The interval, in seconds of the owner's clock.
        interval: Arg,
        /// How many times; for ever if not said.
        times: Option<Arg>,
    },
    /// `cancel_timer`: cancel a named timer.
    Cancel {
        /// The timer's name.
        name: Arg,
    },
}
