//! The tree-walking evaluator: a checked expression evaluated directly,
//! node by node, without folding or bytecode.
//!
//! It is the plain statement of what an expression means. The stack
//! machine must agree with it on every expression, value for value,
//! warning for warning and random draw for random draw, and a property
//! test holds it to that. It is slower than the machine and allocates;
//! tools that show the value of each part of an expression build on it.

use crate::builtins;
use crate::check::{Checked, CheckedPart, CheckedTemplate};
use crate::diag::Span;
use crate::hir::{Node, NodeKind};
use crate::host::Host;
use crate::ops::{self, Limits};
use crate::value::Value;
use crate::vm::TextSink;
use crate::warn::{Fault, RuntimeWarning};

impl Checked {
    /// Evaluates the expression by walking its tree.
    ///
    /// ```
    /// use clash_expr::testing::TestHost;
    /// use clash_expr::{Limits, Type, Value, check};
    ///
    /// let mut host = TestHost::new();
    /// let context = host.context();
    /// let checked = check("1 + 2 * 3", host.registry(), &context, &Type::Any)
    ///     .into_result()
    ///     .unwrap();
    /// assert_eq!(checked.interpret(&mut host, &Limits::default()), Value::Int(7));
    /// ```
    pub fn interpret<H: Host + ?Sized>(&self, host: &mut H, limits: &Limits) -> Value {
        Walker {
            host,
            limits,
            origin: self.origin,
        }
        .walk(&self.root)
    }
}

impl CheckedTemplate {
    /// Renders the template by walking the tree of each expression.
    pub fn interpret<H: Host + ?Sized>(
        &self,
        host: &mut H,
        limits: &Limits,
        sink: &mut dyn TextSink,
    ) {
        let mut scratch = String::new();
        for part in &self.parts {
            match part {
                CheckedPart::Text(literal) => sink.literal(literal),
                CheckedPart::Expr { expr, format } => {
                    let value = expr.interpret(host, limits);
                    scratch.clear();
                    ops::write_formatted(&mut scratch, &value, *format, &*host);
                    sink.value(&scratch);
                }
            }
        }
    }
}

struct Walker<'a, H: Host + ?Sized> {
    host: &'a mut H,
    limits: &'a Limits,
    origin: u64,
}

impl<H: Host + ?Sized> Walker<'_, H> {
    fn warn(&mut self, fault: Fault, span: Span) {
        self.host.warn(RuntimeWarning {
            fault,
            span,
            origin: self.origin,
        });
    }

    /// The value of an operation, or null and a warning.
    fn settle(&mut self, result: Result<Value, Fault>, span: Span) -> Value {
        result.unwrap_or_else(|fault| {
            self.warn(fault, span);
            Value::Null
        })
    }

    /// The answer of a comparison, or false and a warning.
    fn settle_bool(&mut self, result: Result<bool, Fault>, span: Span) -> bool {
        result.unwrap_or_else(|fault| {
            self.warn(fault, span);
            false
        })
    }

    fn truth(&mut self, node: &Node) -> bool {
        let value = self.walk(node);
        self.settle_bool(ops::truthy(&value), node.site)
    }

    fn walk_all(&mut self, nodes: &[Node]) -> Vec<Value> {
        nodes.iter().map(|node| self.walk(node)).collect()
    }

    fn walk(&mut self, node: &Node) -> Value {
        let span = node.span;
        match &node.kind {
            NodeKind::Const(value) => value.clone(),
            NodeKind::Subject(slot) => self.host.subject(*slot),
            NodeKind::Local(slot) => self.host.local(*slot),
            NodeKind::EventField(slot) => self.host.event_field(*slot),
            NodeKind::Property { object, id, args } => {
                let object = self.walk(object);
                let args = self.walk_all(args);
                match object {
                    Value::Subject(handle) => self.host.property(*id, handle, &args),
                    Value::Null => Value::Null,
                    other => {
                        self.warn(ops::wrong(ops::WANT_SUBJECT, &other), span);
                        Value::Null
                    }
                }
            }
            NodeKind::Var { object, name } => match self.walk(object) {
                Value::Subject(handle) => self.host.variable(handle, name),
                Value::Null => Value::Null,
                other => {
                    self.warn(ops::wrong(ops::WANT_SUBJECT, &other), span);
                    Value::Null
                }
            },
            NodeKind::Call { id, args } => {
                let args = self.walk_all(args);
                self.host.call(*id, &args)
            }
            NodeKind::Builtin { f, args } => {
                let args = self.walk_all(args);
                let result = builtins::call(*f, &args, &mut *self.host, self.limits);
                self.settle(result, span)
            }
            NodeKind::Index { object, index } => {
                let object = self.walk(object);
                let index = self.walk(index);
                let result = ops::index(&object, &index, &*self.host);
                self.settle(result, span)
            }
            NodeKind::List(items) => {
                let items = self.walk_all(items);
                self.settle(ops::make_list(&items, self.limits), span)
            }
            NodeKind::Map(entries) => {
                let mut flat = Vec::with_capacity(entries.len() * 2);
                for (key, value) in entries {
                    flat.push(self.walk(key));
                    flat.push(self.walk(value));
                }
                let result = ops::make_map(&flat, self.limits, &*self.host);
                self.settle(result, span)
            }
            NodeKind::Neg(operand) => {
                let operand = self.walk(operand);
                self.settle(ops::neg(&operand), span)
            }
            NodeKind::Not(operand) => Value::Bool(!self.truth(operand)),
            NodeKind::Arith { op, lhs, rhs } => {
                let lhs = self.walk(lhs);
                let rhs = self.walk(rhs);
                self.settle(ops::arith(*op, &lhs, &rhs, self.limits), span)
            }
            NodeKind::Compare { op, lhs, rhs } => {
                let lhs = self.walk(lhs);
                let rhs = self.walk(rhs);
                let holds = ops::compare(*op, &lhs, &rhs, &*self.host);
                Value::Bool(self.settle_bool(holds, span))
            }
            NodeKind::In { item, collection } => {
                let item = self.walk(item);
                let collection = self.walk(collection);
                let holds = ops::contains(&item, &collection, &*self.host);
                Value::Bool(self.settle_bool(holds, span))
            }
            NodeKind::IsNull { operand, negated } => {
                Value::Bool(self.walk(operand).is_null() != *negated)
            }
            NodeKind::And(lhs, rhs) => Value::Bool(self.truth(lhs) && self.truth(rhs)),
            NodeKind::Or(lhs, rhs) => Value::Bool(self.truth(lhs) || self.truth(rhs)),
            NodeKind::Coalesce(lhs, rhs) => match self.walk(lhs) {
                Value::Null => self.walk(rhs),
                value => value,
            },
            NodeKind::If {
                cond,
                then,
                otherwise,
            } => {
                if self.truth(cond) {
                    self.walk(then)
                } else {
                    self.walk(otherwise)
                }
            }
        }
    }
}
