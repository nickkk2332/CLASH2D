//! Evaluating expressions against the simulation: [`SimHost`], the
//! simulation's implementation of the expression language's
//! [`Host`], and [`Bound`], what a caller binds for one evaluation
//! (05 §5.3, §14).
//!
//! An expression knows what `self.loc.tract.devastation` means and
//! nothing about who `self` is. Whoever evaluates one says: it binds a
//! value to each subject, local and event field of the scope the
//! expression was compiled in, **in the order the scope declared them**,
//! and the host answers the rest from the simulation.
//!
//! ```no_run
//! # use clash_core::EntityId;
//! # use clash_data::{Cond, Expr, Runner};
//! # use clash_data::expr::Number;
//! # use clash_sim::{Bound, Sim, Subject};
//! # fn demo(sim: &mut Sim, who: EntityId, weight: &Expr<Number>, ready: &Cond) {
//! // The fields were compiled in a scope that binds `self`: slot 0.
//! let bound = Bound::new().subject(sim.subject_value(Subject::Entity(who)));
//!
//! // Reading only: nothing can draw at random.
//! let mut runner = Runner::new();
//! let mut host = sim.host(bound.clone());
//! let kilograms = weight.eval(&mut runner.on(sim.defs(), &mut host));
//!
//! // A condition of a rule may draw from the rule stream.
//! let defs = sim.defs().clone();
//! let holds = sim.with_random(bound, |host| ready.holds(&mut runner.on(&defs, host)));
//! # let _ = (kilograms, holds);
//! # }
//! ```
//!
//! The host only reads (05 invariant E1). What an effect assigns to is
//! written through [`Sim::assign`](crate::Sim::assign).
//!
//! # What the host answers
//!
//! | Asked for | Answer |
//! |-----------|--------|
//! | A subject, a local, an event field, by slot | What was bound; null for a slot nothing was bound to |
//! | A property of a handle | The getter registered for it, read from truth; null for a subject that no longer exists or a property that is not the simulation's |
//! | A variable of a handle | What was set; else the declared default; else null |
//! | A function | The body registered for it; `exists`; the reader's text helpers ([`reader`](crate::reader)), for nobody in particular: no one is "you" |
//! | The random stream | The rule stream, inside [`Sim::with_random`]; none otherwise, and a random function then gives null with a warning |
//! | A warning | Kept, up to [`MAX_HOST_WARNINGS`], for the caller to take |

use clash_core::{EntityId, Rng};
use clash_data::Runner;
use clash_expr::{
    Compiled, DefRef, Fault, FunctionId, Handle, Host, PropertyId, QueryCall, RuntimeWarning, Value,
};

use crate::Sim;
use crate::event::Subject;

/// The name of the random stream rules draw from (05 §4.7).
pub const RULE_STREAM: &str = "rules";

/// How many run-time warnings a host keeps from one evaluation. More
/// are counted and dropped.
pub const MAX_HOST_WARNINGS: usize = 64;

/// What a caller binds for one evaluation: a value for each subject,
/// local and event field of the scope the expression was compiled in,
/// in the order the scope declared them.
#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct Bound {
    /// The bound subjects, by slot.
    pub subjects: Vec<Value>,
    /// The locals, by slot.
    pub locals: Vec<Value>,
    /// The fields of the triggering event, by slot.
    pub fields: Vec<Value>,
}

impl Bound {
    /// Nothing bound.
    pub fn new() -> Bound {
        Bound::default()
    }

    /// This with a value in the next subject slot.
    #[must_use]
    pub fn subject(mut self, value: Value) -> Bound {
        self.subjects.push(value);
        self
    }

    /// This with a value in the next local slot.
    #[must_use]
    pub fn local(mut self, value: Value) -> Bound {
        self.locals.push(value);
        self
    }

    /// This with a value in the next event field slot.
    #[must_use]
    pub fn field(mut self, value: Value) -> Bound {
        self.fields.push(value);
        self
    }
}

fn slot(values: &[Value], slot: u16) -> Value {
    values.get(usize::from(slot)).cloned().unwrap_or_default()
}

/// The simulation as the expression language's host: reads ground truth
/// (01 R11) on behalf of one evaluation. Made by [`Sim::host`] and
/// [`Sim::with_random`]; see the [module documentation](self).
///
/// Whether an expression may read truth at all was settled when it was
/// compiled: one compiled in a mind context holds no read that is not
/// the character's own or registered safe for belief (05 §7.3).
#[derive(Debug)]
pub struct SimHost<'a> {
    sim: &'a Sim,
    bound: Bound,
    rng: Option<Rng>,
    warnings: Vec<RuntimeWarning>,
    dropped: usize,
    /// Who a text is being written for (05 §4.6), if anyone.
    reader: Option<EntityId>,
    /// Whether a function compiled in its belief form reads truth
    /// instead ([`Sim::host_in_truth`]).
    truth: bool,
}

impl<'a> SimHost<'a> {
    /// The simulation being read.
    pub fn sim(&self) -> &'a Sim {
        self.sim
    }

    /// What is bound, to change between evaluations: a loop binds each
    /// item in turn.
    pub fn bound_mut(&mut self) -> &mut Bound {
        &mut self.bound
    }

    /// The run-time warnings of the evaluations so far, oldest first,
    /// leaving none. Each carries the origin number that
    /// [`ExprDefs::origin`](clash_data::expr::ExprDefs::origin) turns
    /// into a definition and a JSON path.
    pub fn take_warnings(&mut self) -> Vec<RuntimeWarning> {
        core::mem::take(&mut self.warnings)
    }

    /// How many warnings were dropped because more than
    /// [`MAX_HOST_WARNINGS`] were raised.
    pub fn dropped_warnings(&self) -> usize {
        self.dropped
    }
}

impl Host for SimHost<'_> {
    fn subject(&mut self, at: u16) -> Value {
        slot(&self.bound.subjects, at)
    }

    fn local(&mut self, at: u16) -> Value {
        slot(&self.bound.locals, at)
    }

    fn event_field(&mut self, at: u16) -> Value {
        slot(&self.bound.fields, at)
    }

    fn property(&mut self, property: PropertyId, subject: Handle, args: &[Value]) -> Value {
        self.sim.property(property, subject, args)
    }

    fn variable(&mut self, subject: Handle, name: &str) -> Value {
        match self.sim.exprs.subject(subject) {
            Some(subject) => self.sim.var(subject, name),
            None => Value::Null,
        }
    }

    fn call(&mut self, function: FunctionId, args: &[Value]) -> Value {
        // A belief form held to truth reads what it is the form of.
        if self.truth
            && let Some(truth) = self
                .sim
                .exprs
                .function(function)
                .and_then(|def| def.truth_form)
        {
            return truth(self.sim, args);
        }
        // A function that draws at random draws from the host's stream,
        // and gives null where there is none (05 §7.3).
        if let Some(draws) = self.sim.exprs.function(function).and_then(|def| def.draws) {
            return match self.rng.as_mut() {
                Some(rng) => draws(self.sim, args, rng),
                None => Value::Null,
            };
        }
        self.sim.call_function_for(function, args, self.reader)
    }

    fn rng(&mut self) -> Option<&mut Rng> {
        self.rng.as_mut()
    }

    fn warn(&mut self, warning: RuntimeWarning) {
        if self.warnings.len() < MAX_HOST_WARNINGS {
            self.warnings.push(warning);
        } else {
            self.dropped += 1;
        }
    }

    fn def_name(&self, id: DefRef) -> Option<&str> {
        self.sim.exprs.registry().def_name(id)
    }

    fn query(&mut self, call: &QueryCall<'_>, out: &mut Vec<Value>) -> Result<(), Fault> {
        self.sim.answer_query(call, out)
    }
}

impl Sim {
    /// The handle of a subject as a value, to bind for an evaluation
    /// ([`Bound`]) or to store. Null for a cell or a tract beyond what a
    /// handle can name ([`subject`](crate::subject)).
    pub fn subject_value(&self, subject: Subject) -> Value {
        self.exprs.value(subject)
    }

    /// What a value names, if it is a handle of one of the simulation's
    /// subjects. Whether that subject still exists is
    /// [`exists`](Sim::exists).
    pub fn subject_of(&self, value: &Value) -> Option<Subject> {
        self.exprs.kinds().subject_of(value)
    }

    /// Whether what a subject refers to still exists: an entity that has
    /// not been removed, an area or a site that has not been removed, a
    /// realm that has not been destroyed, a cell inside its grid. A
    /// tract and the simulation always exist.
    pub fn exists(&self, subject: Subject) -> bool {
        match subject {
            Subject::Entity(id) => self.entities.contains(id),
            Subject::Loc(loc) | Subject::Element(loc, _) => self.cosmos.normalize(loc).is_ok(),
            Subject::Area(id) => self.cosmos.area(id).is_some(),
            Subject::Site(id) => self.cosmos.site(id).is_some(),
            Subject::Realm(id) | Subject::Tract(id, _) => self
                .cosmos
                .realm(id)
                .is_some_and(|realm| realm.state == clash_world::RealmState::Active),
            Subject::Global => true,
        }
    }

    /// The simulation's entries of the expression registry and what
    /// answers them. Not state.
    pub fn exprs(&self) -> &crate::registry::Bindings {
        &self.exprs
    }

    /// A property of a subject, by the id the registry gave it, read
    /// from truth. Null for a property that is not the simulation's and
    /// for a subject that no longer exists.
    pub fn property(&self, property: PropertyId, subject: Handle, args: &[Value]) -> Value {
        // The stats' and the bodies' properties are registered for each
        // loaded definition and answered by their own modules (06 §4.4,
        // §6.7).
        if let Some(value) = self.life_property(property, subject, args) {
            return value;
        }
        // The kind `ability` is answered by abilities (08 §12).
        if let Some(value) = crate::ability::property(self, property, subject, args) {
            return value;
        }
        match (self.exprs.property(property), self.exprs.subject(subject)) {
            (Some(def), Some(subject)) => (def.get)(self, subject, args),
            _ => Value::Null,
        }
    }

    /// A property of a subject, by the names rules write. For tools and
    /// tests; an expression asks by id.
    pub fn property_named(&self, subject: Subject, name: &str, args: &[Value]) -> Value {
        let found = self
            .exprs
            .property_id(subject.kind(), name)
            .and_then(|id| self.exprs.property(id));
        match found {
            Some(def) => (def.get)(self, subject, args),
            None => Value::Null,
        }
    }

    /// The result of a function the simulation registered, by the id
    /// the registry gave it. Null for one that is not the simulation's.
    pub fn call_function(&self, function: FunctionId, args: &[Value]) -> Value {
        self.call_function_for(function, args, None)
    }

    /// The result of a function, as [`call_function`](Sim::call_function)
    /// gives it, where a text is being written for a reader (05 §4.6):
    /// the text helpers then say "you" of the reader and name everyone
    /// else as the reader knows them ([`reader`](crate::reader)).
    pub fn call_function_for(
        &self,
        function: FunctionId,
        args: &[Value],
        reader: Option<EntityId>,
    ) -> Value {
        if let Some(def) = self.exprs.function(function) {
            return (def.call)(self, args);
        }
        let Some(name) = self
            .exprs
            .registry()
            .function(function)
            .map(|found| found.spec.name.as_str())
        else {
            return Value::Null;
        };
        let first = args.first().cloned().unwrap_or_default();
        match name {
            "exists" => Value::Bool(match &first {
                Value::Null => false,
                Value::Subject(handle) => self
                    .exprs
                    .subject(*handle)
                    .is_some_and(|subject| self.exists(subject)),
                Value::Id(id) => self.exprs.registry().def_name(*id).is_some(),
                _ => true,
            }),
            other => self
                .reader_text(other, &first, args.get(1), reader)
                .map_or(Value::Null, Value::from),
        }
    }

    /// A host over the simulation for evaluations that only read. A
    /// random function gives null, with a warning: use
    /// [`with_random`](Sim::with_random) where draws are allowed.
    pub fn host(&self, bound: Bound) -> SimHost<'_> {
        SimHost {
            sim: self,
            bound,
            rng: None,
            warnings: Vec::new(),
            dropped: 0,
            reader: None,
            truth: false,
        }
    }

    /// A host as [`host`](Sim::host) makes it, for checking against
    /// truth an expression compiled in a mind context: a function
    /// compiled there in its belief form (05 §7.3) reads truth instead,
    /// `believed_dist(c, a, b)` as `dist(a, b)`. What an action's
    /// `requires`, previewed in its actor's mind, is held to when the
    /// action starts (03 §6.6).
    pub fn host_in_truth(&self, bound: Bound) -> SimHost<'_> {
        SimHost {
            truth: true,
            ..self.host(bound)
        }
    }

    /// A host as [`host`](Sim::host) makes it, for a text written for a
    /// reader (05 §4.6): the text helpers say "you" of `reader` and name
    /// everyone else as `reader` knows them ([`reader`](crate::reader)).
    pub fn host_for_reader(&self, bound: Bound, reader: EntityId) -> SimHost<'_> {
        SimHost {
            reader: Some(reader),
            ..self.host(bound)
        }
    }

    /// Runs evaluations that may draw at random: the host draws from
    /// the rule stream ([`RULE_STREAM`]), and what it drew is the
    /// stream's state afterwards. Conditions and effect arguments are
    /// evaluated this way (05 §7.3).
    pub fn with_random<R>(&mut self, bound: Bound, run: impl FnOnce(&mut SimHost<'_>) -> R) -> R {
        let rng = self.rng.stream(RULE_STREAM).clone();
        let mut host = SimHost {
            sim: &*self,
            bound,
            rng: Some(rng),
            warnings: Vec::new(),
            dropped: 0,
            reader: None,
            truth: false,
        };
        let result = run(&mut host);
        if let Some(rng) = host.rng.take() {
            *self.rng.stream(RULE_STREAM) = rng;
        }
        result
    }

    /// Evaluates one compiled expression, reading only: a convenience
    /// for tools and tests. Whoever evaluates many keeps a
    /// [`Runner`] and a host.
    pub fn eval(&self, code: &Compiled, bound: Bound) -> Value {
        let mut host = self.host(bound);
        Runner::new().on(&self.defs, &mut host).value(code)
    }
}
