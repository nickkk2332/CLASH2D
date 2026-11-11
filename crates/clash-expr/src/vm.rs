//! The stack machine (05 §14, step 5) that runs [`Compiled`]
//! expressions.
//!
//! A [`Machine`] is a value stack that is reused from one evaluation to
//! the next, so that evaluating arithmetic and comparisons on numbers
//! and booleans allocates nothing. It holds no simulation state: keep
//! one wherever expressions are evaluated. A host that evaluates an
//! expression from inside a call the machine made to it (a function
//! defined in data, say) uses a second machine for that.
//!
//! Nothing a compiled expression does can make the machine panic or run
//! without end: every jump goes forward, and whatever goes wrong becomes
//! null and a warning (05 §4.4, §4.5, invariant E8).

use crate::builtins;
use crate::code::{Compiled, CompiledText, Op, TextPart};
use crate::host::Host;
use crate::ops::{self, Limits};
use crate::value::Value;
use crate::warn::{Fault, RuntimeWarning};

/// The stack room a new machine starts with. An expression that needs
/// more makes the machine grow, once.
const INITIAL_STACK: usize = 32;

/// Where rendered text goes, told apart by where it came from.
///
/// A template's own text may carry style tags (12 §4.3). A value that
/// was computed is data: a sink that interprets tags should escape it,
/// so that a name containing `<` cannot restyle a line.
pub trait TextSink {
    /// Text written in the template itself.
    fn literal(&mut self, text: &str);

    /// The text of an evaluated expression.
    fn value(&mut self, text: &str);
}

/// A `String` takes both kinds of text as they are.
impl TextSink for String {
    fn literal(&mut self, text: &str) {
        self.push_str(text);
    }

    fn value(&mut self, text: &str) {
        self.push_str(text);
    }
}

/// The stack machine that evaluates [`Compiled`] expressions.
///
/// A machine is a value stack that is reused from one evaluation to the
/// next, so that evaluating arithmetic and comparisons on numbers and
/// booleans allocates nothing. It holds no simulation state: keep one
/// wherever expressions are evaluated. A host that evaluates an
/// expression from inside a call the machine made to it (a function
/// defined in data, say) uses a second machine for that.
///
/// Nothing a compiled expression does can make the machine panic or run
/// without end: every jump goes forward, and whatever goes wrong becomes
/// null and a warning (05 §4.4, §4.5, invariant E8).
///
/// ```
/// use clash_expr::testing::TestHost;
/// use clash_expr::{Machine, Type, Value, compile};
///
/// let mut host = TestHost::new();
/// let me = host.entity(1);
/// host.set(me, "mass", TestHost::real(70.0));
/// let context = host.context();
/// let heavy = compile("self.mass * 2 > 100", host.registry(), &context, &Type::Bool)
///     .into_result()
///     .unwrap();
///
/// let mut machine = Machine::new();
/// assert_eq!(machine.eval(&heavy, &mut host), Value::Bool(true));
/// assert!(machine.eval_condition(&heavy, &mut host));
/// ```
#[derive(Clone, Debug)]
pub struct Machine {
    stack: Vec<Value>,
    scratch: String,
    limits: Limits,
}

impl Default for Machine {
    fn default() -> Machine {
        Machine::new()
    }
}

impl Machine {
    /// A machine with the default limits.
    pub fn new() -> Machine {
        Machine::with_limits(Limits::default())
    }

    /// A machine with these limits.
    pub fn with_limits(limits: Limits) -> Machine {
        Machine {
            stack: Vec::with_capacity(INITIAL_STACK),
            scratch: String::new(),
            limits,
        }
    }

    /// The limits values are kept within.
    pub const fn limits(&self) -> Limits {
        self.limits
    }

    /// Changes the limits, when the setting behind them changes.
    pub const fn set_limits(&mut self, limits: Limits) {
        self.limits = limits;
    }

    /// Evaluates an expression as a condition: null counts as false
    /// (05 §4.4). A value that is neither a bool nor null is reported as
    /// a warning and counts as false.
    pub fn eval_condition<H: Host + ?Sized>(&mut self, code: &Compiled, host: &mut H) -> bool {
        let value = self.eval(code, host);
        match ops::truthy(&value) {
            Ok(truth) => truth,
            Err(fault) => {
                host.warn(RuntimeWarning {
                    fault,
                    span: code.0.spans.last().copied().unwrap_or_default(),
                    origin: code.0.origin,
                });
                false
            }
        }
    }

    /// Evaluates an expression as a number: an int is converted, and
    /// anything else, null included, is `None`.
    ///
    /// The checker accepts an int wherever a real is wanted, and leaves
    /// it an int. Whoever needs a real reads the value this way, or with
    /// [`Value::as_number`].
    pub fn eval_real<H: Host + ?Sized>(
        &mut self,
        code: &Compiled,
        host: &mut H,
    ) -> Option<clash_core::Real> {
        self.eval(code, host).as_number()
    }

    /// Evaluates an expression as an int. Anything else, a real
    /// included, is `None`.
    pub fn eval_int<H: Host + ?Sized>(&mut self, code: &Compiled, host: &mut H) -> Option<i64> {
        self.eval(code, host).as_int()
    }

    /// Renders a text template into a sink.
    pub fn render<H: Host + ?Sized>(
        &mut self,
        text: &CompiledText,
        host: &mut H,
        sink: &mut dyn TextSink,
    ) {
        for part in text.parts.iter() {
            match part {
                TextPart::Text(literal) => sink.literal(literal),
                TextPart::Expr { code, format } => {
                    let value = self.eval(code, host);
                    self.scratch.clear();
                    ops::write_formatted(&mut self.scratch, &value, *format, &*host);
                    sink.value(&self.scratch);
                }
            }
        }
    }

    /// Renders a text template as a `String`.
    pub fn render_to_string<H: Host + ?Sized>(
        &mut self,
        text: &CompiledText,
        host: &mut H,
    ) -> String {
        let mut out = String::new();
        self.render(text, host, &mut out);
        out
    }

    /// Evaluates an expression.
    pub fn eval<H: Host + ?Sized>(&mut self, code: &Compiled, host: &mut H) -> Value {
        let code = &*code.0;
        let Machine { stack, limits, .. } = self;
        stack.clear();
        stack.reserve(code.max_stack);
        let ops = &*code.ops;
        let mut pc = 0_usize;
        while let Some(op) = ops.get(pc) {
            let mut fault: Option<Fault> = None;
            let mut next = pc + 1;
            match *op {
                Op::Const(index) => {
                    stack.push(code.consts.get(index as usize).cloned().unwrap_or_default());
                }
                Op::Subject(slot) => stack.push(host.subject(slot)),
                Op::Local(slot) => stack.push(host.local(slot)),
                Op::EventField(slot) => stack.push(host.event_field(slot)),
                Op::Property { id, argc } => {
                    let base = stack.len().saturating_sub(usize::from(argc) + 1);
                    let value = match stack.get(base) {
                        Some(Value::Subject(handle)) => {
                            let handle = *handle;
                            host.property(id, handle, stack.get(base + 1..).unwrap_or(&[]))
                        }
                        Some(Value::Null) | None => Value::Null,
                        Some(other) => {
                            fault = Some(ops::wrong(ops::WANT_SUBJECT, other));
                            Value::Null
                        }
                    };
                    stack.truncate(base);
                    stack.push(value);
                }
                Op::Var(index) => {
                    let value = match stack.pop() {
                        Some(Value::Subject(handle)) => {
                            match code.consts.get(index as usize).and_then(Value::as_str) {
                                Some(name) => host.variable(handle, name),
                                None => Value::Null,
                            }
                        }
                        Some(Value::Null) | None => Value::Null,
                        Some(other) => {
                            fault = Some(ops::wrong(ops::WANT_SUBJECT, &other));
                            Value::Null
                        }
                    };
                    stack.push(value);
                }
                Op::Call { id, argc } => {
                    let base = stack.len().saturating_sub(usize::from(argc));
                    let value = host.call(id, stack.get(base..).unwrap_or(&[]));
                    stack.truncate(base);
                    stack.push(value);
                }
                Op::Builtin { f, argc } => {
                    let base = stack.len().saturating_sub(usize::from(argc));
                    let result = builtins::call(f, stack.get(base..).unwrap_or(&[]), host, limits);
                    stack.truncate(base);
                    stack.push(settle(result, &mut fault));
                }
                Op::Index => {
                    let index = stack.pop().unwrap_or_default();
                    let object = stack.pop().unwrap_or_default();
                    stack.push(settle(ops::index(&object, &index, &*host), &mut fault));
                }
                Op::List(count) => {
                    let base = stack.len().saturating_sub(count as usize);
                    let result = ops::make_list(stack.get(base..).unwrap_or(&[]), limits);
                    stack.truncate(base);
                    stack.push(settle(result, &mut fault));
                }
                Op::Map(count) => {
                    let base = stack.len().saturating_sub(count as usize * 2);
                    let result = ops::make_map(stack.get(base..).unwrap_or(&[]), limits, &*host);
                    stack.truncate(base);
                    stack.push(settle(result, &mut fault));
                }
                Op::Neg => {
                    let operand = stack.pop().unwrap_or_default();
                    stack.push(settle(ops::neg(&operand), &mut fault));
                }
                Op::Not => {
                    let operand = stack.pop().unwrap_or_default();
                    stack.push(Value::Bool(!truth(&operand, &mut fault)));
                }
                Op::Truthy => {
                    let operand = stack.pop().unwrap_or_default();
                    stack.push(Value::Bool(truth(&operand, &mut fault)));
                }
                Op::Arith(op) => {
                    let rhs = stack.pop().unwrap_or_default();
                    let lhs = stack.pop().unwrap_or_default();
                    stack.push(settle(ops::arith(op, &lhs, &rhs, limits), &mut fault));
                }
                Op::Compare(op) => {
                    let rhs = stack.pop().unwrap_or_default();
                    let lhs = stack.pop().unwrap_or_default();
                    let holds = ops::compare(op, &lhs, &rhs, &*host);
                    stack.push(Value::Bool(settle_bool(holds, &mut fault)));
                }
                Op::In => {
                    let collection = stack.pop().unwrap_or_default();
                    let item = stack.pop().unwrap_or_default();
                    let holds = ops::contains(&item, &collection, &*host);
                    stack.push(Value::Bool(settle_bool(holds, &mut fault)));
                }
                Op::IsNull { negated } => {
                    let operand = stack.pop().unwrap_or_default();
                    stack.push(Value::Bool(operand.is_null() != negated));
                }
                Op::AndThen(target) => {
                    let operand = stack.pop().unwrap_or_default();
                    if !truth(&operand, &mut fault) {
                        stack.push(Value::Bool(false));
                        next = target as usize;
                    }
                }
                Op::OrElse(target) => {
                    let operand = stack.pop().unwrap_or_default();
                    if truth(&operand, &mut fault) {
                        stack.push(Value::Bool(true));
                        next = target as usize;
                    }
                }
                Op::Coalesce(target) => {
                    if stack.last().is_some_and(|top| !top.is_null()) {
                        next = target as usize;
                    } else {
                        stack.pop();
                    }
                }
                Op::JumpIfNot(target) => {
                    let operand = stack.pop().unwrap_or_default();
                    if !truth(&operand, &mut fault) {
                        next = target as usize;
                    }
                }
                Op::Jump(target) => next = target as usize,
            }
            if let Some(fault) = fault {
                host.warn(RuntimeWarning {
                    fault,
                    span: code.spans.get(pc).copied().unwrap_or_default(),
                    origin: code.origin,
                });
            }
            // Jumps only go forward, so the loop ends. A target that did
            // not would be a bug in the compiler; stopping is the safe
            // answer to it.
            if next <= pc {
                break;
            }
            pc = next;
        }
        stack.pop().unwrap_or_default()
    }
}

/// The value of an operation, or null and a note of what went wrong.
#[inline]
fn settle(result: Result<Value, Fault>, fault: &mut Option<Fault>) -> Value {
    match result {
        Ok(value) => value,
        Err(found) => {
            *fault = Some(found);
            Value::Null
        }
    }
}

/// The answer of a comparison, or false and a note of what went wrong.
#[inline]
fn settle_bool(result: Result<bool, Fault>, fault: &mut Option<Fault>) -> bool {
    match result {
        Ok(value) => value,
        Err(found) => {
            *fault = Some(found);
            false
        }
    }
}

/// Whether a value counts as true, noting it if it is neither a bool
/// nor null.
#[inline]
fn truth(value: &Value, fault: &mut Option<Fault>) -> bool {
    settle_bool(ops::truthy(value), fault)
}
