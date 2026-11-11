//! The CLASH2D expression language: the parser, the type checker, the
//! compiler and the evaluator for the expressions that conditions, numeric
//! arguments and effects in mods are written in. It is pure: it evaluates
//! against subjects and variables supplied by its caller and holds no
//! simulation state of its own. Specified in
//! `docs/05-expressions-and-eoc.md`.
//!
//! # Syntax
//!
//! [`parse_expr`] turns a source string into a syntax tree ([`Expr`]) and
//! [`parse_template`] does the same for a text template ([`Template`]).
//! [`print_expr`] and [`print_template`] write a tree back as source.
//! A mistake is a [`Diagnostic`] whose [`Span`] is the offending bytes of
//! the source string.

pub mod ast;
pub mod diag;
mod lexer;
mod parser;
mod print;
pub mod unit;

pub use ast::{
    BinaryOp, Expr, ExprKind, Format, MAX_FORMAT_PRECISION, Name, Number, Template, TemplatePart,
    UnaryOp,
};
pub use diag::{Code, Diagnostic, Severity, Span, closest_name, codes, offset_in_json_string};
pub use lexer::is_keyword;
pub use parser::{MAX_NESTING, MAX_SOURCE_LEN, MAX_TREE_HEIGHT, parse_expr, parse_template};
pub use print::{print_expr, print_template};
pub use unit::{Quantity, Unit};
