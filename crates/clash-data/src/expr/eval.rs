//! Evaluating compiled fields: [`Runner`] and [`Evaluator`].
//!
//! The stack machine of the expression crate asks its host for whatever
//! lies outside an expression, the result of a function among it. A
//! function defined in data (05 §12) is an expression of its own, so the
//! [`Evaluator`] stands between the machine and the simulation's host:
//! it answers calls of such functions by running their bodies, with the
//! arguments as locals, and passes everything else on.

use clash_core::{Real, Rng};
use clash_expr::{
    Compiled, CompiledText, DefRef, Fault, FunctionId, Handle, Host, Limits, Machine, PropertyId,
    QueryCall, RuntimeWarning, Span, Value,
};

use super::defs::{CompiledFunction, ExprDefs, FunctionDefId};
use super::field::{QueryOp, Ready, ReadyQuery};
use crate::defs::Defs;

/// How deep functions defined in data may call each other unless the
/// host says otherwise: the default of `rules.max_call_depth`
/// (05 §13.2).
pub const DEFAULT_MAX_CALL_DEPTH: usize = 32;

/// The deepest functions defined in data may ever call each other,
/// whatever `rules.max_call_depth` is set to.
pub const MAX_CALL_DEPTH_LIMIT: usize = 256;

/// The machines and frames evaluation runs on, kept from one evaluation
/// to the next so that evaluating allocates nothing it has allocated
/// before. Keep one wherever fields are evaluated, and make an
/// [`Evaluator`] from it for each host.
///
/// ```no_run
/// # use clash_data::{Defs, Expr, Runner};
/// # use clash_data::expr::Number;
/// # use clash_expr::testing::TestHost;
/// # fn demo(defs: &Defs, energy: &Expr<Number>, host: &mut TestHost) {
/// let mut runner = Runner::new();
/// let joules = energy.eval(&mut runner.on(defs, host));
/// # let _ = joules;
/// # }
/// ```
#[derive(Clone, Debug)]
pub struct Runner {
    /// Machines not in use. One is taken for each expression being
    /// evaluated, so a function called from an expression runs on a
    /// machine of its own.
    spare: Vec<Machine>,
    /// The arguments of the functions being run, innermost last.
    frames: Vec<Vec<Value>>,
    /// The candidates of the queries being run, innermost last: how deep
    /// in functions each query is, the slot `it` has there, and the
    /// candidate (05 §11).
    its: Vec<(usize, u16, Value)>,
    limits: Limits,
    max_depth: usize,
}

impl Default for Runner {
    fn default() -> Runner {
        Runner::new()
    }
}

impl Runner {
    /// A runner with the default limits.
    pub fn new() -> Runner {
        Runner {
            spare: Vec::new(),
            frames: Vec::new(),
            its: Vec::new(),
            limits: Limits::default(),
            max_depth: DEFAULT_MAX_CALL_DEPTH,
        }
    }

    /// Sets the limits values are kept within, when the setting behind
    /// them changes.
    pub fn set_limits(&mut self, limits: Limits) {
        self.limits = limits;
        for machine in &mut self.spare {
            machine.set_limits(limits);
        }
    }

    /// Sets how deep functions defined in data may call each other, when
    /// the setting behind it changes. Each level is a frame on the
    /// machine's own stack, so the depth is kept to
    /// [`MAX_CALL_DEPTH_LIMIT`] whatever is asked for: a setting cannot
    /// make evaluation overflow.
    pub fn set_max_call_depth(&mut self, depth: usize) {
        self.max_depth = depth.min(MAX_CALL_DEPTH_LIMIT);
    }

    /// How deep functions defined in data may call each other.
    pub fn max_call_depth(&self) -> usize {
        self.max_depth
    }

    /// An evaluator over a host, for fields of these definitions.
    pub fn on<'a, H: Host + ?Sized>(
        &'a mut self,
        defs: &'a Defs,
        host: &'a mut H,
    ) -> Evaluator<'a, H> {
        self.over(defs.exprs(), host)
    }

    /// An evaluator over a host, given the expression side of a set of
    /// definitions.
    pub fn over<'a, H: Host + ?Sized>(
        &'a mut self,
        exprs: &'a ExprDefs,
        host: &'a mut H,
    ) -> Evaluator<'a, H> {
        self.frames.clear();
        self.its.clear();
        Evaluator {
            runner: self,
            exprs,
            host,
        }
    }

    fn take_machine(&mut self) -> Machine {
        self.spare
            .pop()
            .unwrap_or_else(|| Machine::with_limits(self.limits))
    }
}

/// Evaluates compiled fields against a host, running the functions that
/// mods define. Made by [`Runner::on`]; the fields take one in
/// [`Expr::eval`](crate::Expr::eval), [`Cond::holds`](crate::Cond::holds)
/// and [`Text::render`](crate::Text::render).
///
/// It is itself a [`Host`], so code that drives a
/// [`Machine`] directly can hand it this in place of the simulation's
/// host and get the functions of mods answered.
pub struct Evaluator<'a, H: Host + ?Sized> {
    runner: &'a mut Runner,
    exprs: &'a ExprDefs,
    host: &'a mut H,
}

impl<'a, H: Host + ?Sized> Evaluator<'a, H> {
    /// The host underneath.
    pub fn host(&mut self) -> &mut H {
        self.host
    }

    /// Evaluates an expression.
    pub fn value(&mut self, code: &Compiled) -> Value {
        let mut machine = self.runner.take_machine();
        let value = machine.eval(code, self);
        self.runner.spare.push(machine);
        value
    }

    /// Evaluates an expression as a condition: null counts as false.
    pub fn condition(&mut self, code: &Compiled) -> bool {
        let mut machine = self.runner.take_machine();
        let holds = machine.eval_condition(code, self);
        self.runner.spare.push(machine);
        holds
    }

    /// Renders a text template.
    pub fn text(&mut self, text: &CompiledText) -> String {
        let mut machine = self.runner.take_machine();
        let rendered = machine.render_to_string(text, self);
        self.runner.spare.push(machine);
        rendered
    }

    /// Calls a function by the id the registry gave it: one defined in
    /// data is run here, any other is the host's.
    pub fn call(&mut self, function: FunctionId, args: &[Value]) -> Value {
        Host::call(self, function, args)
    }

    /// Calls a function by its qualified name, such as
    /// `ashlands:fall_damage`. Null if there is none of that name.
    pub fn call_named(&mut self, name: &str, args: &[Value]) -> Value {
        match self.exprs.registry().function_id(name) {
            Some(function) => Host::call(self, function, args),
            None => Value::Null,
        }
    }

    /// Calls the function of a `function` definition by its dense id.
    /// This is how one written in place inside another definition, which
    /// has no name to be called by, is run. Null if the id is not one of
    /// these definitions'.
    pub fn call_def(&mut self, function: FunctionDefId, args: &[Value]) -> Value {
        let exprs = self.exprs;
        match exprs.functions().of(function) {
            Some(defined) => self.run_function(defined, args),
            None => Value::Null,
        }
    }

    pub(crate) fn run(&mut self, ready: &Ready) -> Value {
        match ready {
            Ready::Constant(value) => value.clone(),
            Ready::Code(code) => self.value(code),
            Ready::Call(call) => {
                let args: Vec<Value> = call.args.iter().map(|arg| self.run(arg)).collect();
                Host::call(self, call.function, &args)
            }
            Ready::Unavailable(what, origin) => {
                self.unavailable(what, *origin);
                Value::Null
            }
            Ready::Query(query) => self.query_source(query),
        }
    }

    /// Evaluates something with `it` bound to a candidate of a query
    /// (05 §11).
    pub(crate) fn with_it<T>(
        &mut self,
        slot: u16,
        value: Value,
        work: impl FnOnce(&mut Self) -> T,
    ) -> T {
        let depth = self.runner.frames.len();
        self.runner.its.push((depth, slot, value));
        let result = work(self);
        self.runner.its.pop();
        result
    }

    /// The value of a subject of the field being evaluated, by slot, as
    /// an expression of the field would read it.
    pub(crate) fn subject_of_field(&mut self, slot: u16) -> Value {
        Host::subject(self, slot)
    }

    /// Asks the host underneath for the candidates of a query.
    pub(crate) fn host_query(
        &mut self,
        call: &QueryCall<'_>,
        out: &mut Vec<Value>,
    ) -> Result<(), Fault> {
        self.host.query(call, out)
    }

    /// Hands a run-time warning to the host.
    pub(crate) fn warn_at(&mut self, warning: RuntimeWarning) {
        self.host.warn(warning);
    }

    /// What a value source over a query gives (05 §7.4).
    fn query_source(&mut self, source: &ReadyQuery) -> Value {
        let query = &source.query;
        let origin = query.origin();
        let found = match source.op {
            QueryOp::First => self.query_up_to(query, Some(1)),
            _ => self.query(query),
        };
        let Some(found) = found else {
            return Value::Null;
        };
        match source.op {
            QueryOp::Count => Value::Int(i64::try_from(found.len()).unwrap_or(i64::MAX)),
            QueryOp::Collect => {
                if found.len() > self.runner.limits.max_value_size {
                    self.warn_query(Fault::TooLarge, origin);
                    return Value::Null;
                }
                Value::list(found)
            }
            QueryOp::First => found.into_iter().next().unwrap_or(Value::Null),
            QueryOp::Pick => self.pick_from(source, found),
            QueryOp::Sum | QueryOp::Min | QueryOp::Max => {
                let mut numbers: Vec<Value> = Vec::with_capacity(found.len());
                for item in found {
                    let value = match &source.of {
                        Some(of) => self.with_it(query.it, item, |eval| of.value(eval)),
                        None => item,
                    };
                    match value {
                        Value::Int(_) | Value::Real(_) => numbers.push(value),
                        Value::Null => {}
                        other => {
                            let found = other.type_name();
                            self.warn_query(
                                Fault::Type {
                                    wanted: "a number",
                                    found,
                                },
                                origin,
                            );
                        }
                    }
                }
                fold_numbers(source.op, &numbers)
            }
        }
    }

    /// One result of a query at random, weighted by `weight` if it has
    /// one: one draw from the host's stream, none for no results.
    fn pick_from(&mut self, source: &ReadyQuery, found: Vec<Value>) -> Value {
        if found.is_empty() {
            return Value::Null;
        }
        let origin = source.query.origin();
        let Some(weight) = &source.weight else {
            let Some(rng) = self.host.rng() else {
                self.warn_query(Fault::NoRandom, origin);
                return Value::Null;
            };
            return rng.pick(&found).cloned().unwrap_or(Value::Null);
        };
        let mut weights = Vec::with_capacity(found.len());
        let mut total = Real::ZERO;
        for item in &found {
            let value = self.with_it(source.query.it, item.clone(), |eval| weight.value(eval));
            let weight = value
                .as_number()
                .filter(|w| *w > Real::ZERO)
                .unwrap_or(Real::ZERO);
            total += weight;
            weights.push(weight);
        }
        if total <= Real::ZERO || !total.get().is_finite() {
            return Value::Null;
        }
        let Some(rng) = self.host.rng() else {
            self.warn_query(Fault::NoRandom, origin);
            return Value::Null;
        };
        let mut roll = rng.unit() * total;
        let mut chosen = Value::Null;
        for (item, weight) in found.into_iter().zip(weights) {
            if weight <= Real::ZERO {
                continue;
            }
            chosen = item;
            if roll < weight {
                break;
            }
            roll -= weight;
        }
        chosen
    }

    /// Tells the host that something not built yet was asked for.
    pub(crate) fn unavailable(&mut self, what: &'static str, origin: u64) {
        self.host.warn(RuntimeWarning {
            fault: Fault::Unavailable(what),
            span: Span::default(),
            origin,
        });
    }

    fn run_function(&mut self, function: &CompiledFunction, args: &[Value]) -> Value {
        if self.runner.frames.len() >= self.runner.max_depth {
            self.host.warn(RuntimeWarning {
                fault: Fault::CallDepth,
                span: Span::default(),
                origin: function.origin,
            });
            return Value::Null;
        }
        let Some(body) = &function.body else {
            return Value::Null;
        };
        let mut frame = Vec::with_capacity(function.params.len());
        for (index, param) in function.params.iter().enumerate() {
            frame.push(match args.get(index) {
                Some(value) => value.clone(),
                None => param.default.clone().unwrap_or_default(),
            });
        }
        self.runner.frames.push(frame);
        let value = self.run(body);
        self.runner.frames.pop();
        value
    }
}

/// The total, least or greatest of some numbers: an int if all are, a
/// real otherwise; `0` for no numbers' total and null for their least
/// or greatest.
fn fold_numbers(op: QueryOp, numbers: &[Value]) -> Value {
    let all_ints = numbers.iter().all(|number| matches!(number, Value::Int(_)));
    match op {
        QueryOp::Sum if all_ints => {
            let mut total: i64 = 0;
            for number in numbers {
                if let Value::Int(int) = number {
                    match total.checked_add(*int) {
                        Some(sum) => total = sum,
                        None => return Value::Null,
                    }
                }
            }
            Value::Int(total)
        }
        QueryOp::Sum => Value::Real(
            numbers
                .iter()
                .filter_map(Value::as_number)
                .fold(Real::ZERO, |total, number| total + number),
        ),
        _ => {
            let mut best: Option<&Value> = None;
            for number in numbers {
                let better = match (best.and_then(Value::as_number), number.as_number()) {
                    (None, _) => true,
                    (Some(have), Some(new)) if op == QueryOp::Min => new < have,
                    (Some(have), Some(new)) => new > have,
                    _ => false,
                };
                if better {
                    best = Some(number);
                }
            }
            best.cloned().unwrap_or(Value::Null)
        }
    }
}

impl<H: Host + ?Sized> Host for Evaluator<'_, H> {
    fn subject(&mut self, slot: u16) -> Value {
        // The candidate of a query being run at this depth.
        let depth = self.runner.frames.len();
        if let Some((_, _, value)) = self
            .runner
            .its
            .iter()
            .rev()
            .find(|(at, it, _)| *at == depth && *it == slot)
        {
            return value.clone();
        }
        // A function sees its parameters and nothing of its caller.
        if self.runner.frames.is_empty() {
            self.host.subject(slot)
        } else {
            Value::Null
        }
    }

    fn local(&mut self, slot: u16) -> Value {
        match self.runner.frames.last() {
            Some(frame) => frame.get(usize::from(slot)).cloned().unwrap_or_default(),
            None => self.host.local(slot),
        }
    }

    fn event_field(&mut self, slot: u16) -> Value {
        if self.runner.frames.is_empty() {
            self.host.event_field(slot)
        } else {
            Value::Null
        }
    }

    fn property(&mut self, property: PropertyId, subject: Handle, args: &[Value]) -> Value {
        self.host.property(property, subject, args)
    }

    fn variable(&mut self, subject: Handle, name: &str) -> Value {
        self.host.variable(subject, name)
    }

    fn call(&mut self, function: FunctionId, args: &[Value]) -> Value {
        let exprs = self.exprs;
        match exprs.functions().get(function) {
            Some(defined) => self.run_function(defined, args),
            None => self.host.call(function, args),
        }
    }

    fn rng(&mut self) -> Option<&mut Rng> {
        self.host.rng()
    }

    fn warn(&mut self, warning: RuntimeWarning) {
        self.host.warn(warning);
    }

    fn def_name(&self, id: DefRef) -> Option<&str> {
        self.exprs
            .registry()
            .def_name(id)
            .or_else(|| self.host.def_name(id))
    }

    fn query(&mut self, call: &QueryCall<'_>, out: &mut Vec<Value>) -> Result<(), Fault> {
        self.host.query(call, out)
    }
}
