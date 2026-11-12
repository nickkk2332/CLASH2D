//! Bytecode (05 §14, step 5): a checked tree flattened into operations
//! for a small stack machine.
//!
//! A [`Compiled`] expression is immutable, shared and cheap to clone: it
//! is what definitions hold. Each operation remembers the span of source
//! it came from, so that a run-time warning can say where.

use std::sync::Arc;

use crate::ast::Format;
use crate::builtins::Builtin;
use crate::check::{Checked, CheckedPart, CheckedTemplate, Outcome, check, check_template};
use crate::context::Context;
use crate::diag::{Diagnostic, Span, codes};
use crate::fold::fold;
use crate::hir::{ArithOp, CmpOp, Node, NodeKind};
use crate::registry::Registry;
use crate::types::Type;
use crate::value::{FunctionId, PropertyId, Value};

/// One operation of the stack machine.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Op {
    /// Push a constant.
    Const(u32),
    /// Push a bound subject.
    Subject(u16),
    /// Push a local.
    Local(u16),
    /// Push an event field.
    EventField(u16),
    /// Pop arguments and a subject; push the property.
    Property { id: PropertyId, argc: u8 },
    /// Pop a subject; push its variable, named by a constant.
    Var(u32),
    /// Pop arguments; push what the host's function gives.
    Call { id: FunctionId, argc: u8 },
    /// Pop arguments; push what a built-in gives.
    Builtin { f: Builtin, argc: u8 },
    /// Pop an index and an object; push the element.
    Index,
    /// Pop this many values; push a list of them.
    List(u32),
    /// Pop this many keys and values, in turn; push a map of them.
    Map(u32),
    /// Negate the top of the stack.
    Neg,
    /// Replace the top of the stack with its logical negation.
    Not,
    /// Replace the top of the stack with whether it counts as true.
    Truthy,
    /// Pop two values; push the result of arithmetic on them.
    Arith(ArithOp),
    /// Pop two values; push whether the comparison holds.
    Compare(CmpOp),
    /// Pop a collection and an item; push whether the item is in it.
    In,
    /// Replace the top of the stack with whether it is null, or, if
    /// `negated`, whether it is not.
    IsNull { negated: bool },
    /// Pop a value. If it counts as false, push false and jump.
    AndThen(u32),
    /// Pop a value. If it counts as true, push true and jump.
    OrElse(u32),
    /// If the top of the stack is not null, jump; otherwise pop it.
    Coalesce(u32),
    /// Pop a value. If it counts as false, jump.
    JumpIfNot(u32),
    /// Jump.
    Jump(u32),
}

/// The most arguments one call may be given.
pub const MAX_ARGUMENTS: usize = 255;

#[derive(Debug)]
pub(crate) struct Code {
    pub(crate) ops: Box<[Op]>,
    /// The source span of each operation, for warnings.
    pub(crate) spans: Box<[Span]>,
    pub(crate) consts: Box<[Value]>,
    pub(crate) ty: Type,
    /// The deepest the stack gets.
    pub(crate) max_stack: usize,
    pub(crate) origin: u64,
}

/// A compiled expression: bytecode for the [`Machine`](crate::Machine).
///
/// Cloning shares the code. A `Compiled` can be sent to and used from
/// any thread.
///
/// ```
/// use clash_expr::testing::TestHost;
/// use clash_expr::{Machine, Type, Value, compile};
///
/// let mut host = TestHost::new();
/// let context = host.context();
/// let compiled = compile("2 + 3 * 4", host.registry(), &context, &Type::Int)
///     .into_result()
///     .unwrap();
/// // The whole expression was worked out at load.
/// assert_eq!(compiled.constant(), Some(&Value::Int(14)));
/// assert_eq!(Machine::new().eval(&compiled, &mut host), Value::Int(14));
/// ```
#[derive(Clone, Debug)]
pub struct Compiled(pub(crate) Arc<Code>);

impl Compiled {
    /// The type the checker gave the expression.
    pub fn ty(&self) -> &Type {
        &self.0.ty
    }

    /// The origin number the expression was compiled with.
    pub fn origin(&self) -> u64 {
        self.0.origin
    }

    /// The expression's value, if it was worked out entirely at load.
    pub fn constant(&self) -> Option<&Value> {
        match &*self.0.ops {
            [Op::Const(index)] => self.0.consts.get(*index as usize),
            _ => None,
        }
    }

    /// The number of operations. A rough measure of what evaluating
    /// costs.
    pub fn len(&self) -> usize {
        self.0.ops.len()
    }

    /// Whether there are no operations. Never true of an expression that
    /// compiled: even a constant is one operation.
    pub fn is_empty(&self) -> bool {
        self.0.ops.is_empty()
    }

    /// The properties the expression reads, in the order it reads them,
    /// each once. What a reactive expression would subscribe to
    /// (05 §7.5).
    pub fn properties_read(&self) -> Vec<PropertyId> {
        let mut read = Vec::new();
        for op in self.0.ops.iter() {
            if let Op::Property { id, .. } = op
                && !read.contains(id)
            {
                read.push(*id);
            }
        }
        read
    }

    /// The functions of the host the expression calls, in the order it
    /// calls them, each once: what a reactive expression would subscribe
    /// to beside its properties (05 §7.5). The language's own built-ins
    /// read nothing of the host and are not listed.
    pub fn functions_read(&self) -> Vec<FunctionId> {
        let mut read = Vec::new();
        for op in self.0.ops.iter() {
            if let Op::Call { id, .. } = op
                && !read.contains(id)
            {
                read.push(*id);
            }
        }
        read
    }

    /// The qualified names of the variables the expression reads, in the
    /// order it reads them, each once.
    pub fn variables_read(&self) -> Vec<&str> {
        let mut read: Vec<&str> = Vec::new();
        for op in self.0.ops.iter() {
            if let Op::Var(index) = op
                && let Some(name) = self.0.consts.get(*index as usize).and_then(Value::as_str)
                && !read.contains(&name)
            {
                read.push(name);
            }
        }
        read
    }
}

/// A compiled text template: literal pieces and compiled expressions.
#[derive(Clone, Debug)]
pub struct CompiledText {
    pub(crate) parts: Arc<[TextPart]>,
}

#[derive(Clone, Debug)]
pub(crate) enum TextPart {
    Text(Arc<str>),
    Expr {
        code: Compiled,
        format: Option<Format>,
    },
}

impl CompiledText {
    /// The text, if the template has no expressions left to evaluate.
    pub fn constant(&self) -> Option<&str> {
        match &*self.parts {
            [] => Some(""),
            [TextPart::Text(text)] => Some(text),
            _ => None,
        }
    }
}

/// Parses, checks, folds and compiles an expression.
///
/// `expected` is the type the field wants: [`Type::Any`] for no
/// constraint, `Type::Bool.nullable()` for a condition.
pub fn compile(
    source: &str,
    registry: &Registry,
    context: &Context,
    expected: &Type,
) -> Outcome<Compiled> {
    let Outcome { value, diagnostics } = check(source, registry, context, expected);
    match value {
        Some(checked) => compile_checked(checked, diagnostics),
        None => Outcome::failed(diagnostics),
    }
}

/// Parses, checks, folds and compiles a text template.
pub fn compile_template(
    source: &str,
    registry: &Registry,
    context: &Context,
) -> Outcome<CompiledText> {
    let Outcome { value, diagnostics } = check_template(source, registry, context);
    match value {
        Some(checked) => checked.compile_with(diagnostics),
        None => Outcome::failed(diagnostics),
    }
}

/// Folds and emits one checked tree, adding what is found on the way to
/// `diagnostics`. `None` if the tree cannot be emitted.
pub(crate) fn compile_node(
    root: Box<Node>,
    origin: u64,
    diagnostics: &mut Vec<Diagnostic>,
) -> Option<Compiled> {
    let outcome = compile_checked(Checked { root, origin }, Vec::new());
    diagnostics.extend(outcome.diagnostics);
    outcome.value
}

fn compile_checked(mut checked: Checked, mut diagnostics: Vec<Diagnostic>) -> Outcome<Compiled> {
    fold(&mut checked.root, &mut diagnostics);
    let mut emitter = Emitter::default();
    emitter.emit(&checked.root);
    if let Some(error) = emitter.error.take() {
        diagnostics.push(error);
    }
    let code = Code {
        ops: emitter.ops.into(),
        spans: emitter.spans.into(),
        consts: emitter.consts.into(),
        ty: checked.root.ty.clone(),
        max_stack: emitter.max_depth,
        origin: checked.origin,
    };
    Outcome::new(Compiled(Arc::new(code)), diagnostics)
}

impl Checked {
    /// Folds constants and emits bytecode. Diagnostics found on the way
    /// (an operation on constants that always fails) are in the outcome.
    pub fn compile(&self) -> Outcome<Compiled> {
        compile_checked(self.clone(), Vec::new())
    }
}

impl CheckedTemplate {
    /// Folds constants and emits bytecode for each expression. Pieces
    /// that come to constant text are joined to the text around them.
    pub fn compile(&self) -> Outcome<CompiledText> {
        self.clone().compile_with(Vec::new())
    }

    fn compile_with(self, mut diagnostics: Vec<Diagnostic>) -> Outcome<CompiledText> {
        let mut parts: Vec<TextPart> = Vec::new();
        let mut text = String::new();
        for part in self.parts {
            match part {
                CheckedPart::Text(literal) => text.push_str(&literal),
                CheckedPart::Expr { expr, format } => {
                    let outcome = compile_checked(expr, Vec::new());
                    diagnostics.extend(outcome.diagnostics);
                    let Some(code) = outcome.value else { continue };
                    // A constant text needs no evaluating. Other
                    // constants are left for the machine, which knows how
                    // the host names things.
                    if let (Some(Value::Str(constant)), None) = (code.constant(), format) {
                        text.push_str(constant);
                        continue;
                    }
                    if !text.is_empty() {
                        parts.push(TextPart::Text(Arc::from(core::mem::take(&mut text))));
                    }
                    parts.push(TextPart::Expr { code, format });
                }
            }
        }
        if !text.is_empty() {
            parts.push(TextPart::Text(Arc::from(text)));
        }
        Outcome::new(
            CompiledText {
                parts: parts.into(),
            },
            diagnostics,
        )
    }
}

#[derive(Default)]
struct Emitter {
    ops: Vec<Op>,
    spans: Vec<Span>,
    consts: Vec<Value>,
    depth: usize,
    max_depth: usize,
    error: Option<Diagnostic>,
}

impl Emitter {
    /// Adds an operation that takes `pops` values off the stack and puts
    /// `pushes` on.
    fn op(&mut self, op: Op, span: Span, pops: usize, pushes: usize) -> usize {
        self.depth = self.depth.saturating_sub(pops) + pushes;
        self.max_depth = self.max_depth.max(self.depth);
        self.ops.push(op);
        self.spans.push(span);
        self.ops.len() - 1
    }

    fn constant(&mut self, value: Value) -> u32 {
        let index = self.consts.len();
        self.consts.push(value);
        u32::try_from(index).unwrap_or(u32::MAX)
    }

    /// The position the next operation will have, as a jump target.
    fn here(&self) -> u32 {
        u32::try_from(self.ops.len()).unwrap_or(u32::MAX)
    }

    /// Points the jump at `at` to the next operation.
    fn land(&mut self, at: usize) {
        let target = self.here();
        if let Some(
            Op::AndThen(to) | Op::OrElse(to) | Op::Coalesce(to) | Op::JumpIfNot(to) | Op::Jump(to),
        ) = self.ops.get_mut(at)
        {
            *to = target;
        }
    }

    fn argc(&mut self, count: usize, span: Span) -> u8 {
        u8::try_from(count).unwrap_or_else(|_| {
            self.error.get_or_insert_with(|| {
                Diagnostic::error(
                    codes::ARGUMENT_COUNT,
                    span,
                    format!(
                        "{count} arguments are given; the most a call may have is {MAX_ARGUMENTS}"
                    ),
                )
            });
            u8::MAX
        })
    }

    fn count(count: usize) -> u32 {
        u32::try_from(count).unwrap_or(u32::MAX)
    }

    fn emit(&mut self, node: &Node) {
        let span = node.span;
        match &node.kind {
            NodeKind::Const(value) => {
                let index = self.constant(value.clone());
                self.op(Op::Const(index), span, 0, 1);
            }
            NodeKind::Subject(slot) => {
                self.op(Op::Subject(*slot), span, 0, 1);
            }
            NodeKind::Local(slot) => {
                self.op(Op::Local(*slot), span, 0, 1);
            }
            NodeKind::EventField(slot) => {
                self.op(Op::EventField(*slot), span, 0, 1);
            }
            NodeKind::Property { object, id, args } => {
                self.emit(object);
                for arg in args {
                    self.emit(arg);
                }
                let argc = self.argc(args.len(), span);
                self.op(Op::Property { id: *id, argc }, span, args.len() + 1, 1);
            }
            NodeKind::Var { object, name } => {
                self.emit(object);
                let index = self.constant(Value::Str(Arc::clone(name)));
                self.op(Op::Var(index), span, 1, 1);
            }
            NodeKind::Call { id, args } => {
                for arg in args {
                    self.emit(arg);
                }
                let argc = self.argc(args.len(), span);
                self.op(Op::Call { id: *id, argc }, span, args.len(), 1);
            }
            NodeKind::Builtin { f, args } => {
                for arg in args {
                    self.emit(arg);
                }
                let argc = self.argc(args.len(), span);
                self.op(Op::Builtin { f: *f, argc }, span, args.len(), 1);
            }
            NodeKind::Index { object, index } => {
                self.emit(object);
                self.emit(index);
                self.op(Op::Index, span, 2, 1);
            }
            NodeKind::List(items) => {
                for item in items {
                    self.emit(item);
                }
                self.op(Op::List(Self::count(items.len())), span, items.len(), 1);
            }
            NodeKind::Map(entries) => {
                for (key, value) in entries {
                    self.emit(key);
                    self.emit(value);
                }
                let count = Self::count(entries.len());
                self.op(Op::Map(count), span, entries.len() * 2, 1);
            }
            NodeKind::Neg(operand) => {
                self.emit(operand);
                self.op(Op::Neg, span, 1, 1);
            }
            NodeKind::Not(operand) => {
                self.emit(operand);
                self.op(Op::Not, operand.site, 1, 1);
            }
            NodeKind::Arith { op, lhs, rhs } => {
                self.emit(lhs);
                self.emit(rhs);
                self.op(Op::Arith(*op), span, 2, 1);
            }
            NodeKind::Compare { op, lhs, rhs } => {
                self.emit(lhs);
                self.emit(rhs);
                self.op(Op::Compare(*op), span, 2, 1);
            }
            NodeKind::In { item, collection } => {
                self.emit(item);
                self.emit(collection);
                self.op(Op::In, span, 2, 1);
            }
            NodeKind::IsNull { operand, negated } => {
                self.emit(operand);
                self.op(Op::IsNull { negated: *negated }, span, 1, 1);
            }
            NodeKind::And(lhs, rhs) => {
                self.emit(lhs);
                let jump = self.op(Op::AndThen(0), lhs.site, 1, 0);
                self.emit(rhs);
                self.op(Op::Truthy, rhs.site, 1, 1);
                self.land(jump);
            }
            NodeKind::Or(lhs, rhs) => {
                self.emit(lhs);
                let jump = self.op(Op::OrElse(0), lhs.site, 1, 0);
                self.emit(rhs);
                self.op(Op::Truthy, rhs.site, 1, 1);
                self.land(jump);
            }
            NodeKind::Coalesce(lhs, rhs) => {
                self.emit(lhs);
                let jump = self.op(Op::Coalesce(0), span, 1, 0);
                self.emit(rhs);
                self.land(jump);
            }
            NodeKind::If {
                cond,
                then,
                otherwise,
            } => {
                self.emit(cond);
                let to_else = self.op(Op::JumpIfNot(0), cond.site, 1, 0);
                self.emit(then);
                let to_end = self.op(Op::Jump(0), span, 0, 0);
                // The other branch starts from the depth before `then`.
                self.depth = self.depth.saturating_sub(1);
                self.land(to_else);
                self.emit(otherwise);
                self.land(to_end);
            }
        }
    }
}
