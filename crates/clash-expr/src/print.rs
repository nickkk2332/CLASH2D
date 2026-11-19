//! The printer: a syntax tree back to source text.
//!
//! Printing a tree the parser produced and parsing the result gives the
//! same tree. Brackets are written only where the grammar needs them, so
//! the printed form is also the canonical way to write an expression.

use core::fmt::Write as _;

use crate::ast::{BinaryOp, Expr, ExprKind, Number, Template, TemplatePart, UnaryOp};

/// How tightly a construct binds: the productions of the grammar from
/// loosest to tightest.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Level {
    Expr,
    Or,
    And,
    Not,
    Compare,
    Coalesce,
    Sum,
    Term,
    Unary,
    Power,
    Postfix,
    Primary,
}

/// The level a node is produced at.
const fn level_of(expr: &Expr) -> Level {
    match &expr.kind {
        ExprKind::If { .. } => Level::Expr,
        ExprKind::Binary { op, .. } => binary_level(*op),
        ExprKind::Unary {
            op: UnaryOp::Not, ..
        } => Level::Not,
        ExprKind::Unary {
            op: UnaryOp::Neg, ..
        } => Level::Unary,
        ExprKind::Member { .. } | ExprKind::Call { .. } | ExprKind::Index { .. } => Level::Postfix,
        ExprKind::Null
        | ExprKind::Bool(_)
        | ExprKind::Number { .. }
        | ExprKind::Str(_)
        | ExprKind::Name(_)
        | ExprKind::Local(_)
        | ExprKind::List(_)
        | ExprKind::Map(_) => Level::Primary,
    }
}

const fn binary_level(op: BinaryOp) -> Level {
    match op {
        BinaryOp::Or => Level::Or,
        BinaryOp::And => Level::And,
        BinaryOp::Eq
        | BinaryOp::Ne
        | BinaryOp::Lt
        | BinaryOp::Le
        | BinaryOp::Gt
        | BinaryOp::Ge
        | BinaryOp::In => Level::Compare,
        BinaryOp::Coalesce => Level::Coalesce,
        BinaryOp::Add | BinaryOp::Sub => Level::Sum,
        BinaryOp::Mul | BinaryOp::Div | BinaryOp::Rem => Level::Term,
        BinaryOp::Pow => Level::Power,
    }
}

/// The levels the grammar allows on each side of a binary operator.
const fn operand_levels(op: BinaryOp) -> (Level, Level) {
    match op {
        BinaryOp::Or => (Level::Or, Level::And),
        BinaryOp::And => (Level::And, Level::Not),
        BinaryOp::Eq
        | BinaryOp::Ne
        | BinaryOp::Lt
        | BinaryOp::Le
        | BinaryOp::Gt
        | BinaryOp::Ge
        | BinaryOp::In => (Level::Coalesce, Level::Coalesce),
        BinaryOp::Coalesce => (Level::Coalesce, Level::Sum),
        BinaryOp::Add | BinaryOp::Sub => (Level::Sum, Level::Term),
        BinaryOp::Mul | BinaryOp::Div | BinaryOp::Rem => (Level::Term, Level::Unary),
        BinaryOp::Pow => (Level::Postfix, Level::Unary),
    }
}

/// Writes an expression as source text.
///
/// ```
/// use clash_expr::{parse_expr, print_expr};
///
/// let tree = parse_expr("( a+b )*c - -d").unwrap();
/// assert_eq!(print_expr(&tree), "(a + b) * c - -d");
/// ```
pub fn print_expr(expr: &Expr) -> String {
    let mut out = String::new();
    write_expr(&mut out, expr, Level::Expr);
    out
}

/// Writes a text template as source text.
///
/// ```
/// use clash_expr::{parse_template, print_template};
///
/// let template = parse_template("{{x}} is {  x :.2}").unwrap();
/// assert_eq!(print_template(&template), "{{x}} is {x:.2}");
/// ```
pub fn print_template(template: &Template) -> String {
    let mut out = String::new();
    for part in &template.parts {
        match part {
            TemplatePart::Text(text) => {
                for character in text.chars() {
                    match character {
                        '{' => out.push_str("{{"),
                        '}' => out.push_str("}}"),
                        other => out.push(other),
                    }
                }
            }
            TemplatePart::Expr { expr, format, .. } => {
                out.push('{');
                let inner = print_expr(expr);
                // `{{` would read as a literal brace.
                if inner.starts_with('{') {
                    out.push('(');
                    out.push_str(&inner);
                    out.push(')');
                } else {
                    out.push_str(&inner);
                }
                if let Some(format) = format {
                    let _ = write!(out, ":.{}", format.precision);
                }
                out.push('}');
            }
        }
    }
    out
}

/// Writes `expr` where the grammar wants something at `needed` or
/// tighter, in brackets if it is looser.
fn write_expr(out: &mut String, expr: &Expr, needed: Level) {
    if level_of(expr) < needed {
        out.push('(');
        write_bare(out, expr);
        out.push(')');
    } else {
        write_bare(out, expr);
    }
}

fn write_list(out: &mut String, items: &[Expr]) {
    for (index, item) in items.iter().enumerate() {
        if index > 0 {
            out.push_str(", ");
        }
        write_expr(out, item, Level::Expr);
    }
}

fn write_bare(out: &mut String, expr: &Expr) {
    match &expr.kind {
        ExprKind::Null => out.push_str("null"),
        ExprKind::Bool(value) => out.push_str(if *value { "true" } else { "false" }),
        ExprKind::Number { value, unit } => {
            match value {
                Number::Int(int) => {
                    let _ = write!(out, "{int}");
                }
                // The debug form of a float always carries a point or an
                // exponent, so it reads back as a real and to the same
                // bits.
                Number::Real(real) => {
                    let _ = write!(out, "{:?}", real.get());
                }
            }
            if let Some(unit) = unit {
                out.push_str(unit.symbol());
            }
        }
        ExprKind::Str(text) => {
            out.push('\'');
            for character in text.chars() {
                if character == '\'' {
                    out.push('\'');
                }
                out.push(character);
            }
            out.push('\'');
        }
        ExprKind::Name(name) => {
            let _ = write!(out, "{name}");
        }
        ExprKind::Local(name) => {
            let _ = write!(out, "${name}");
        }
        ExprKind::List(items) => {
            out.push('[');
            write_list(out, items);
            out.push(']');
        }
        ExprKind::Map(entries) => {
            out.push('{');
            for (index, (key, value)) in entries.iter().enumerate() {
                if index > 0 {
                    out.push_str(", ");
                }
                write_expr(out, key, Level::Expr);
                out.push_str(": ");
                write_expr(out, value, Level::Expr);
            }
            out.push('}');
        }
        ExprKind::Member { object, name } => {
            write_expr(out, object, Level::Postfix);
            let _ = write!(out, ".{name}");
        }
        ExprKind::Call { callee, args } => {
            write_expr(out, callee, Level::Postfix);
            out.push('(');
            write_list(out, args);
            out.push(')');
        }
        ExprKind::Index { object, index } => {
            write_expr(out, object, Level::Postfix);
            out.push('[');
            write_expr(out, index, Level::Expr);
            out.push(']');
        }
        ExprKind::Unary { op, operand } => match op {
            UnaryOp::Neg => {
                out.push('-');
                write_expr(out, operand, Level::Unary);
            }
            UnaryOp::Not => {
                out.push_str("not ");
                write_expr(out, operand, Level::Not);
            }
        },
        ExprKind::Binary { op, lhs, rhs } => {
            let (left, right) = operand_levels(*op);
            write_expr(out, lhs, left);
            let _ = write!(out, " {op} ");
            write_expr(out, rhs, right);
        }
        ExprKind::If {
            cond,
            then,
            otherwise,
        } => {
            out.push_str("if ");
            write_expr(out, cond, Level::Expr);
            out.push_str(" then ");
            write_expr(out, then, Level::Expr);
            out.push_str(" else ");
            write_expr(out, otherwise, Level::Expr);
        }
    }
}
