//! The CLASH2D expression language: the parser, the type checker, the
//! compiler and the evaluator for the expressions that conditions, numeric
//! arguments and effects in mods are written in. It is pure: it evaluates
//! against subjects and variables supplied by its caller and holds no
//! simulation state of its own. Specified in
//! `docs/05-expressions-and-eoc.md`.
//!
//! # The pipeline
//!
//! | Step (05 §14) | Function | Gives |
//! |---------------|----------|-------|
//! | 1. Parse | [`parse_expr`], [`parse_template`] | A syntax tree with positions: [`Expr`], [`Template`] |
//! | 2, 3. Resolve names, check types | [`check`], [`check_template`] | A checked tree: [`Checked`], [`CheckedTemplate`] |
//! | 4, 5. Fold constants, emit | [`Checked::compile`] | Bytecode: [`Compiled`], [`CompiledText`] |
//! | All of them | [`compile`], [`compile_template`] | The same, from source |
//! | Run | [`Machine::eval`], [`Machine::render`] | A [`Value`], or text |
//! | For `set` and `add` | [`compile_place`] | What may be assigned to: a [`Place`] |
//!
//! Every step reports through [`Diagnostic`]s, whose [`Span`] is a range
//! of bytes of the source string. This crate knows nothing about files;
//! the data layer maps a span to a line and a column
//! ([`offset_in_json_string`] helps).
//!
//! # What the host supplies
//!
//! The host is the simulation. It fills in a [`Registry`] at start-up
//! with the kinds of subject, the properties, the functions, the events,
//! the verbs, the declared variables and the definition ids it has. It
//! describes each place an expression appears with a [`Context`]. And it
//! answers the machine's questions while an expression runs through the
//! [`Host`] trait. A small host for tests is in [`testing`].
//!
//! ```
//! use clash_expr::testing::TestHost;
//! use clash_expr::{Machine, Type, Value, compile, compile_template};
//!
//! let mut host = TestHost::new();
//! let me = host.entity(1);
//! host.set(me, "mass", TestHost::real(70.0));
//! host.set(me, "material", host.material("core:glass"));
//! let context = host.context();
//!
//! // A condition. The id is checked against the registry at load.
//! let fragile = compile(
//!     "self.material == 'glass' and self.mass < 100",
//!     host.registry(),
//!     &context,
//!     &Type::Bool.nullable(),
//! )
//! .into_result()
//! .unwrap();
//! let mut machine = Machine::new();
//! assert!(machine.eval_condition(&fragile, &mut host));
//!
//! // A misspelt id is an error at load, with a suggestion.
//! let outcome = compile("self.material == 'glas'", host.registry(), &context, &Type::Bool);
//! assert_eq!(outcome.diagnostics[0].suggestion.as_deref(), Some("did you mean 'glass'?"));
//!
//! // A text template.
//! let line = compile_template("{cap(the(self))} weigh {self.mass:.1} kg.", host.registry(), &context)
//!     .into_result()
//!     .unwrap();
//! assert_eq!(machine.render_to_string(&line, &mut host), "You weigh 70.0 kg.");
//! # let _ = Value::Null;
//! ```
//!
//! # What never happens
//!
//! Nothing a mod can write makes this crate panic. A bad expression is a
//! diagnostic at load. A fault while running (a null where a value is
//! needed, a division by zero, a value of the wrong type) gives null and
//! a [`RuntimeWarning`], as 05 §4.4 and §4.5 lay down.

pub mod ast;
mod builtins;
mod check;
mod code;
pub mod context;
pub mod diag;
mod fold;
mod hir;
mod host;
mod interp;
mod lexer;
mod ops;
mod parser;
mod place;
mod print;
pub mod registry;
pub mod testing;
mod types;
pub mod unit;
mod value;
mod vm;
mod warn;

pub use ast::{
    BinaryOp, Expr, ExprKind, Format, MAX_FORMAT_PRECISION, Name, Number, Template, TemplatePart,
    UnaryOp,
};
pub use check::{Checked, CheckedTemplate, Outcome, check, check_template};
pub use code::{Compiled, CompiledText, MAX_ARGUMENTS, compile, compile_template};
pub use context::{Binding, Context, Purity, Reads, Undeclared};
pub use diag::{Code, Diagnostic, Severity, Span, closest_name, codes, offset_in_json_string};
pub use host::Host;
pub use lexer::is_keyword;
pub use ops::{DEFAULT_MAX_VALUE_SIZE, Limits};
pub use parser::{MAX_NESTING, MAX_SOURCE_LEN, MAX_TREE_HEIGHT, parse_expr, parse_template};
pub use place::{Place, PlaceRoot, compile_place};
pub use print::{print_expr, print_template};
pub use registry::{
    DefType, Event, EventSpec, Function, FunctionSpec, Kind, KindSpec, ParamSpec, Property,
    PropertySpec, Registry, RegistryError, Resolved, Scope, Variable, VariableSpec, Verb, VerbSpec,
};
pub use types::Type;
pub use unit::{Quantity, Unit};
pub use value::{
    DefRef, DefTypeId, EventId, FunctionId, Handle, KindId, Map, PropertyId, Value, VariableId,
    VerbId,
};
pub use vm::{Machine, TextSink};
pub use warn::{DEFAULT_WARNING_BURST, Fault, RuntimeWarning, WarningLimiter};
