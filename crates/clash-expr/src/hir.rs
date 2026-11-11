//! The checked tree: an expression after names are resolved and types
//! are known (05 §14, steps 2 and 3).
//!
//! Where the syntax tree says "the name `self`", this says "subject slot
//! 0, an entity". Every decision that depends on the registry or the
//! context has been taken, so what follows (folding, emitting, walking)
//! needs neither.

use std::sync::Arc;

use crate::builtins::Builtin;
use crate::diag::Span;
use crate::types::Type;
use crate::value::{FunctionId, PropertyId, Value};

/// A node of the checked tree.
#[derive(Clone, PartialEq, Debug)]
pub(crate) struct Node {
    pub(crate) kind: NodeKind,
    /// The type the checker gave the node.
    pub(crate) ty: Type,
    /// Where it was written. A fault in the node's own operation is
    /// reported here.
    pub(crate) span: Span,
    /// Where the operand in this position was written. An operation
    /// above that finds fault with this operand (a value that is not a
    /// bool where one is wanted) reports it here. The same as `span`
    /// until folding puts another node in this one's place: the node
    /// that moves in brings its own `span` and inherits the `site`.
    pub(crate) site: Span,
    /// Whether the value is the character's own state, in the sense of
    /// the mind context (05 §7.3). Used only while checking.
    pub(crate) own: bool,
}

impl Node {
    pub(crate) fn new(kind: NodeKind, ty: Type, span: Span) -> Box<Node> {
        Box::new(Node {
            kind,
            ty,
            span,
            site: span,
            own: false,
        })
    }

    /// The constant this node is, if it is one.
    pub(crate) const fn as_const(&self) -> Option<&Value> {
        match &self.kind {
            NodeKind::Const(value) => Some(value),
            _ => None,
        }
    }
}

/// An arithmetic operator.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub(crate) enum ArithOp {
    Add,
    Sub,
    Mul,
    Div,
    Rem,
    Pow,
}

impl ArithOp {
    pub(crate) const fn symbol(self) -> &'static str {
        match self {
            ArithOp::Add => "+",
            ArithOp::Sub => "-",
            ArithOp::Mul => "*",
            ArithOp::Div => "/",
            ArithOp::Rem => "%",
            ArithOp::Pow => "^",
        }
    }
}

/// A comparison operator.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub(crate) enum CmpOp {
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
}

impl CmpOp {
    pub(crate) const fn symbol(self) -> &'static str {
        match self {
            CmpOp::Eq => "==",
            CmpOp::Ne => "!=",
            CmpOp::Lt => "<",
            CmpOp::Le => "<=",
            CmpOp::Gt => ">",
            CmpOp::Ge => ">=",
        }
    }
}

/// What a checked node is.
#[derive(Clone, PartialEq, Debug)]
pub(crate) enum NodeKind {
    /// A value known at load.
    Const(Value),
    /// A bound subject, by slot.
    Subject(u16),
    /// A local, by slot.
    Local(u16),
    /// A field of the triggering event, by slot.
    EventField(u16),
    /// A property of a subject. A property of null is null.
    Property {
        object: Box<Node>,
        id: PropertyId,
        args: Vec<Node>,
    },
    /// A variable on a subject, by its qualified name.
    Var { object: Box<Node>, name: Arc<str> },
    /// A function the host computes.
    Call { id: FunctionId, args: Vec<Node> },
    /// A function this crate computes.
    Builtin { f: Builtin, args: Vec<Node> },
    /// An element of a list or an entry of a map.
    Index { object: Box<Node>, index: Box<Node> },
    /// A list built from its elements.
    List(Vec<Node>),
    /// A map built from its entries.
    Map(Vec<(Node, Node)>),
    /// Arithmetic negation.
    Neg(Box<Node>),
    /// Logical negation. Null counts as false.
    Not(Box<Node>),
    /// Arithmetic. Null in, null out, with a warning.
    Arith {
        op: ArithOp,
        lhs: Box<Node>,
        rhs: Box<Node>,
    },
    /// A comparison. False if either side is null.
    Compare {
        op: CmpOp,
        lhs: Box<Node>,
        rhs: Box<Node>,
    },
    /// Membership in a list or among a map's keys.
    In {
        item: Box<Node>,
        collection: Box<Node>,
    },
    /// `x == null` or, negated, `x != null`.
    IsNull { operand: Box<Node>, negated: bool },
    /// Short-circuit conjunction.
    And(Box<Node>, Box<Node>),
    /// Short-circuit disjunction.
    Or(Box<Node>, Box<Node>),
    /// The left side unless it is null, then the right.
    Coalesce(Box<Node>, Box<Node>),
    /// A choice between two values.
    If {
        cond: Box<Node>,
        then: Box<Node>,
        otherwise: Box<Node>,
    },
}
