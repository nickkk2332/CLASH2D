//! Evaluating compiled fields: [`Runner`] and [`Evaluator`].
//!
//! The stack machine of the expression crate asks its host for whatever
//! lies outside an expression, the result of a function among it. A
//! function defined in data (05 §12) is an expression of its own, so the
//! [`Evaluator`] stands between the machine and the simulation's host:
//! it answers calls of such functions by running their bodies, with the
//! arguments as locals, and passes everything else on.

use clash_core::Rng;
use clash_expr::{
    Compiled, CompiledText, DefRef, Fault, FunctionId, Handle, Host, Limits, Machine, PropertyId,
    RuntimeWarning, Span, Value,
};

use super::defs::{CompiledFunction, ExprDefs, FunctionDefId};
use super::field::Ready;
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
        }
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

impl<H: Host + ?Sized> Host for Evaluator<'_, H> {
    fn subject(&mut self, slot: u16) -> Value {
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
}
