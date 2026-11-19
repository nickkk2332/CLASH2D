//! The syntax tree: what the parser produces and the printer writes.
//!
//! A tree records what was written and where, and nothing about what it
//! means. Names are not resolved and types are not known until the
//! checker has run.
//!
//! The parser never produces a negative number: `-5` is [`UnaryOp::Neg`]
//! applied to `5`. A tree built by hand should keep to that, or printing
//! it and parsing it again gives a different tree.

use core::fmt;

use clash_core::Real;

use crate::diag::Span;
use crate::unit::Unit;

/// One node of an expression's syntax tree, with where it was written.
#[derive(Clone, PartialEq, Debug)]
pub struct Expr {
    /// What the node is.
    pub kind: ExprKind,
    /// The bytes of the source it covers.
    pub span: Span,
}

impl Expr {
    /// A node.
    pub const fn new(kind: ExprKind, span: Span) -> Expr {
        Expr { kind, span }
    }

    /// The same tree with every span set to zero, for comparing two trees
    /// by shape alone.
    #[must_use]
    pub fn without_spans(&self) -> Expr {
        let strip = |expr: &Expr| Box::new(expr.without_spans());
        let strip_all = |list: &[Expr]| list.iter().map(Expr::without_spans).collect();
        let kind = match &self.kind {
            ExprKind::Null
            | ExprKind::Bool(_)
            | ExprKind::Number { .. }
            | ExprKind::Str(_)
            | ExprKind::Name(_)
            | ExprKind::Local(_) => self.kind.clone(),
            ExprKind::List(items) => ExprKind::List(strip_all(items)),
            ExprKind::Map(entries) => ExprKind::Map(
                entries
                    .iter()
                    .map(|(key, value)| (key.without_spans(), value.without_spans()))
                    .collect(),
            ),
            ExprKind::Member { object, name } => ExprKind::Member {
                object: strip(object),
                name: name.clone(),
            },
            ExprKind::Call { callee, args } => ExprKind::Call {
                callee: strip(callee),
                args: strip_all(args),
            },
            ExprKind::Index { object, index } => ExprKind::Index {
                object: strip(object),
                index: strip(index),
            },
            ExprKind::Unary { op, operand } => ExprKind::Unary {
                op: *op,
                operand: strip(operand),
            },
            ExprKind::Binary { op, lhs, rhs } => ExprKind::Binary {
                op: *op,
                lhs: strip(lhs),
                rhs: strip(rhs),
            },
            ExprKind::If {
                cond,
                then,
                otherwise,
            } => ExprKind::If {
                cond: strip(cond),
                then: strip(then),
                otherwise: strip(otherwise),
            },
        };
        Expr {
            kind,
            span: Span::default(),
        }
    }
}

/// What a node of the syntax tree is (05 §4.1).
#[derive(Clone, PartialEq, Debug)]
pub enum ExprKind {
    /// `null`.
    Null,
    /// `true` or `false`.
    Bool(bool),
    /// A number, with the unit that followed it if one did: `42`, `3.5`,
    /// `300ms`.
    Number {
        /// The number as written, before the unit is applied.
        value: Number,
        /// The unit, if any.
        unit: Option<Unit>,
    },
    /// A text in single quotes: `'ash'`.
    Str(String),
    /// A bare name: a bound subject such as `self`, or `event`, or, where
    /// it is called, a function.
    Name(Name),
    /// A local variable or parameter: `$power`.
    Local(Name),
    /// A list: `[1, 2, 3]`.
    List(Vec<Expr>),
    /// A map: `{'ash': 3, 'iron': 1}`. Entries are kept in the order
    /// written.
    Map(Vec<(Expr, Expr)>),
    /// A property or entry read: `object.name`.
    Member {
        /// What is read from.
        object: Box<Expr>,
        /// The name after the dot.
        name: Name,
    },
    /// A call: `callee(args)`. The callee is a [`Name`](ExprKind::Name)
    /// for a function and a [`Member`](ExprKind::Member) for a property
    /// that takes arguments; the parser accepts anything and the checker
    /// refuses the rest.
    Call {
        /// What is called.
        callee: Box<Expr>,
        /// The arguments.
        args: Vec<Expr>,
    },
    /// An element or entry read: `object[index]`.
    Index {
        /// The list, map or text.
        object: Box<Expr>,
        /// The position or key.
        index: Box<Expr>,
    },
    /// `-operand` or `not operand`.
    Unary {
        /// The operator.
        op: UnaryOp,
        /// What it applies to.
        operand: Box<Expr>,
    },
    /// `lhs op rhs`.
    Binary {
        /// The operator.
        op: BinaryOp,
        /// The left operand.
        lhs: Box<Expr>,
        /// The right operand.
        rhs: Box<Expr>,
    },
    /// `if cond then then else otherwise`.
    If {
        /// The condition.
        cond: Box<Expr>,
        /// The value when it holds.
        then: Box<Expr>,
        /// The value when it does not.
        otherwise: Box<Expr>,
    },
}

/// A number as written: with a point or an exponent it is real, without
/// it is an integer.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Number {
    /// A whole number.
    Int(i64),
    /// A number with a fraction or an exponent.
    Real(Real),
}

/// A name, with its namespace if it was written with one: `clamp`,
/// `ashlands:fall_damage`.
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub struct Name {
    /// The part before the colon, if there is one.
    pub namespace: Option<String>,
    /// The name itself.
    pub name: String,
}

impl Name {
    /// A name without a namespace.
    pub fn bare(name: impl Into<String>) -> Name {
        Name {
            namespace: None,
            name: name.into(),
        }
    }

    /// A name in a namespace.
    pub fn qualified(namespace: impl Into<String>, name: impl Into<String>) -> Name {
        Name {
            namespace: Some(namespace.into()),
            name: name.into(),
        }
    }

    /// Whether the name was written with a namespace.
    pub const fn is_qualified(&self) -> bool {
        self.namespace.is_some()
    }
}

impl fmt::Display for Name {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if let Some(namespace) = &self.namespace {
            write!(f, "{namespace}:")?;
        }
        f.write_str(&self.name)
    }
}

/// An operator with one operand.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum UnaryOp {
    /// Arithmetic negation, `-x`.
    Neg,
    /// Logical negation, `not x`.
    Not,
}

/// An operator with two operands.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum BinaryOp {
    /// `or`
    Or,
    /// `and`
    And,
    /// `==`
    Eq,
    /// `!=`
    Ne,
    /// `<`
    Lt,
    /// `<=`
    Le,
    /// `>`
    Gt,
    /// `>=`
    Ge,
    /// `in`
    In,
    /// `??`
    Coalesce,
    /// `+`
    Add,
    /// `-`
    Sub,
    /// `*`
    Mul,
    /// `/`
    Div,
    /// `%`
    Rem,
    /// `^`
    Pow,
}

impl BinaryOp {
    /// The operator as written.
    pub const fn symbol(self) -> &'static str {
        match self {
            BinaryOp::Or => "or",
            BinaryOp::And => "and",
            BinaryOp::Eq => "==",
            BinaryOp::Ne => "!=",
            BinaryOp::Lt => "<",
            BinaryOp::Le => "<=",
            BinaryOp::Gt => ">",
            BinaryOp::Ge => ">=",
            BinaryOp::In => "in",
            BinaryOp::Coalesce => "??",
            BinaryOp::Add => "+",
            BinaryOp::Sub => "-",
            BinaryOp::Mul => "*",
            BinaryOp::Div => "/",
            BinaryOp::Rem => "%",
            BinaryOp::Pow => "^",
        }
    }

    /// Whether this is one of the comparisons of the `compare` production,
    /// which do not chain.
    pub const fn is_comparison(self) -> bool {
        matches!(
            self,
            BinaryOp::Eq
                | BinaryOp::Ne
                | BinaryOp::Lt
                | BinaryOp::Le
                | BinaryOp::Gt
                | BinaryOp::Ge
                | BinaryOp::In
        )
    }
}

impl fmt::Display for BinaryOp {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.symbol())
    }
}

/// A text template (05 §4.6): literal text with expressions in braces.
#[derive(Clone, PartialEq, Debug)]
pub struct Template {
    /// The pieces, in order.
    pub parts: Vec<TemplatePart>,
}

impl Template {
    /// The same template with every span set to zero.
    #[must_use]
    pub fn without_spans(&self) -> Template {
        Template {
            parts: self
                .parts
                .iter()
                .map(|part| match part {
                    TemplatePart::Text(text) => TemplatePart::Text(text.clone()),
                    TemplatePart::Expr { expr, format, .. } => TemplatePart::Expr {
                        expr: expr.without_spans(),
                        format: *format,
                        span: Span::default(),
                    },
                })
                .collect(),
        }
    }
}

/// One piece of a text template.
#[derive(Clone, PartialEq, Debug)]
pub enum TemplatePart {
    /// Literal text, with `{{` and `}}` already turned into single braces.
    Text(String),
    /// An expression in braces, with its number format if it has one.
    Expr {
        /// The expression.
        expr: Expr,
        /// The format after the colon, if any.
        format: Option<Format>,
        /// The whole piece, braces included.
        span: Span,
    },
}

/// How a number in a text template is written: `{x:.1}`.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct Format {
    /// The number of digits after the decimal point.
    pub precision: u8,
}

/// The most digits after the point a [`Format`] may ask for. An `f64`
/// carries no more than seventeen significant digits.
pub const MAX_FORMAT_PRECISION: u8 = 17;
