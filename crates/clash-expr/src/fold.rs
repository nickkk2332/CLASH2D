//! Constant folding (05 §14, step 4): whatever can be worked out at load
//! is.
//!
//! Folding never changes what an expression evaluates to or which
//! warnings it raises. An operation on constants that would fail is left
//! in place to fail at run time, and reported at load as a warning. A
//! branch that can never be taken is dropped only where evaluation would
//! have skipped it anyway. Nothing that asks the host (a property, a
//! variable, a host function, a random draw, the name of an id) is
//! folded.

use clash_core::Rng;

use crate::builtins::{self, Builtin};
use crate::diag::{Diagnostic, codes};
use crate::hir::{Node, NodeKind};
use crate::host::Host;
use crate::ops::{self, Limits, Res};
use crate::types::Type;
use crate::value::{FunctionId, Handle, PropertyId, Value};
use crate::warn::RuntimeWarning;

/// A host with nothing in it. Folding only runs operations that never
/// ask it anything.
struct NoHost;

impl Host for NoHost {
    fn subject(&mut self, _slot: u16) -> Value {
        Value::Null
    }

    fn local(&mut self, _slot: u16) -> Value {
        Value::Null
    }

    fn event_field(&mut self, _slot: u16) -> Value {
        Value::Null
    }

    fn property(&mut self, _property: PropertyId, _subject: Handle, _args: &[Value]) -> Value {
        Value::Null
    }

    fn variable(&mut self, _subject: Handle, _name: &str) -> Value {
        Value::Null
    }

    fn call(&mut self, _function: FunctionId, _args: &[Value]) -> Value {
        Value::Null
    }

    fn rng(&mut self) -> Option<&mut Rng> {
        None
    }

    fn warn(&mut self, _warning: RuntimeWarning) {}
}

/// Whether a value can be operated on without a host: it holds no id,
/// whose name only the host knows.
fn plain(value: &Value) -> bool {
    match value {
        Value::Id(_) => false,
        Value::List(items) => items.iter().all(plain),
        Value::Map(entries) => entries.values().all(plain),
        _ => true,
    }
}

/// The node's value, if it is a constant that needs no host.
fn constant(node: &Node) -> Option<&Value> {
    node.as_const().filter(|value| plain(value))
}

/// The values of nodes that are all such constants.
fn constants<'a>(nodes: impl IntoIterator<Item = &'a Node>) -> Option<Vec<Value>> {
    nodes
        .into_iter()
        .map(|node| constant(node).cloned())
        .collect()
}

const fn is_random(builtin: Builtin) -> bool {
    matches!(
        builtin,
        Builtin::Rand | Builtin::RandInt | Builtin::Chance | Builtin::Pick
    )
}

/// What folding a node came to.
enum Folded {
    /// Leave the node as it is.
    Keep,
    /// The node is this operation on constants: a value, or a fault that
    /// is left for run time.
    Value(Res),
    /// The node is the same as one of its children.
    Child(Node),
}

fn take(node: &mut Node) -> Node {
    let placeholder = Node {
        kind: NodeKind::Const(Value::Null),
        ty: Type::Any,
        span: node.span,
        site: node.site,
        own: false,
    };
    core::mem::replace(node, placeholder)
}

/// Folds a checked tree in place. Warnings about operations that always
/// fail are added to `diagnostics`.
pub(crate) fn fold(node: &mut Node, diagnostics: &mut Vec<Diagnostic>) {
    let limits = Limits::default();
    let host = &mut NoHost;
    let folded = match &mut node.kind {
        NodeKind::Const(_)
        | NodeKind::Subject(_)
        | NodeKind::Local(_)
        | NodeKind::EventField(_) => Folded::Keep,
        NodeKind::Property { object, args, .. } => {
            fold(object, diagnostics);
            for arg in args {
                fold(arg, diagnostics);
            }
            Folded::Keep
        }
        NodeKind::Var { object, .. } => {
            fold(object, diagnostics);
            Folded::Keep
        }
        NodeKind::Call { args, .. } => {
            for arg in args {
                fold(arg, diagnostics);
            }
            Folded::Keep
        }
        NodeKind::Builtin { f, args } => {
            for arg in args.iter_mut() {
                fold(arg, diagnostics);
            }
            match constants(args.iter()) {
                Some(values) if !is_random(*f) => {
                    Folded::Value(builtins::call(*f, &values, host, &limits))
                }
                _ => Folded::Keep,
            }
        }
        NodeKind::Index { object, index } => {
            fold(object, diagnostics);
            fold(index, diagnostics);
            match (constant(object), constant(index)) {
                (Some(object), Some(index)) => Folded::Value(ops::index(object, index, host)),
                _ => Folded::Keep,
            }
        }
        NodeKind::List(items) => {
            for item in items.iter_mut() {
                fold(item, diagnostics);
            }
            match constants(items.iter()) {
                Some(values) => Folded::Value(ops::make_list(&values, &limits)),
                None => Folded::Keep,
            }
        }
        NodeKind::Map(entries) => {
            for (key, value) in entries.iter_mut() {
                fold(key, diagnostics);
                fold(value, diagnostics);
            }
            let flat = entries.iter().flat_map(|(key, value)| [key, value]);
            match constants(flat) {
                Some(values) => Folded::Value(ops::make_map(&values, &limits, host)),
                None => Folded::Keep,
            }
        }
        NodeKind::Neg(operand) => {
            fold(operand, diagnostics);
            constant(operand).map_or(Folded::Keep, |value| Folded::Value(ops::neg(value)))
        }
        NodeKind::Not(operand) => {
            fold(operand, diagnostics);
            constant(operand).map_or(Folded::Keep, |value| {
                Folded::Value(ops::truthy(value).map(|truth| Value::Bool(!truth)))
            })
        }
        NodeKind::Arith { op, lhs, rhs } => {
            fold(lhs, diagnostics);
            fold(rhs, diagnostics);
            match (constant(lhs), constant(rhs)) {
                (Some(a), Some(b)) => Folded::Value(ops::arith(*op, a, b, &limits)),
                _ => Folded::Keep,
            }
        }
        NodeKind::Compare { op, lhs, rhs } => {
            fold(lhs, diagnostics);
            fold(rhs, diagnostics);
            match (constant(lhs), constant(rhs)) {
                (Some(a), Some(b)) => Folded::Value(ops::compare(*op, a, b, host).map(Value::Bool)),
                _ => Folded::Keep,
            }
        }
        NodeKind::In { item, collection } => {
            fold(item, diagnostics);
            fold(collection, diagnostics);
            match (constant(item), constant(collection)) {
                (Some(item), Some(collection)) => {
                    Folded::Value(ops::contains(item, collection, host).map(Value::Bool))
                }
                _ => Folded::Keep,
            }
        }
        NodeKind::IsNull { operand, negated } => {
            fold(operand, diagnostics);
            operand.as_const().map_or(Folded::Keep, |value| {
                Folded::Value(Ok(Value::Bool(value.is_null() != *negated)))
            })
        }
        NodeKind::And(lhs, rhs) => {
            fold(lhs, diagnostics);
            fold(rhs, diagnostics);
            match (lhs.as_const().map(ops::truthy), rhs.as_const()) {
                (Some(Ok(false)), _) => Folded::Value(Ok(Value::Bool(false))),
                (Some(Ok(true)), Some(rhs)) => Folded::Value(ops::truthy(rhs).map(Value::Bool)),
                _ => Folded::Keep,
            }
        }
        NodeKind::Or(lhs, rhs) => {
            fold(lhs, diagnostics);
            fold(rhs, diagnostics);
            match (lhs.as_const().map(ops::truthy), rhs.as_const()) {
                (Some(Ok(true)), _) => Folded::Value(Ok(Value::Bool(true))),
                (Some(Ok(false)), Some(rhs)) => Folded::Value(ops::truthy(rhs).map(Value::Bool)),
                _ => Folded::Keep,
            }
        }
        NodeKind::Coalesce(lhs, rhs) => {
            fold(lhs, diagnostics);
            fold(rhs, diagnostics);
            match lhs.as_const() {
                Some(Value::Null) => Folded::Child(take(rhs)),
                Some(_) => Folded::Child(take(lhs)),
                None => Folded::Keep,
            }
        }
        NodeKind::If {
            cond,
            then,
            otherwise,
        } => {
            fold(cond, diagnostics);
            fold(then, diagnostics);
            fold(otherwise, diagnostics);
            match cond.as_const().map(ops::truthy) {
                Some(Ok(true)) => Folded::Child(take(then)),
                Some(Ok(false)) => Folded::Child(take(otherwise)),
                _ => Folded::Keep,
            }
        }
    };
    match folded {
        Folded::Keep => {}
        Folded::Value(Ok(value)) => node.kind = NodeKind::Const(value),
        Folded::Value(Err(fault)) => diagnostics.push(
            Diagnostic::warning(
                codes::ALWAYS_FAILS,
                node.span,
                format!("this fails every time it runs: {fault}"),
            )
            .with_suggestion("it gives null, with a warning, each time"),
        ),
        // The child takes the node's place, and keeps the place's site: what
        // an operation above reports about this operand is still reported
        // where the operand was written.
        Folded::Child(child) => {
            let site = node.site;
            *node = child;
            node.site = site;
        }
    }
}
