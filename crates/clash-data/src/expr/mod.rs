//! Expressions as definition fields (04 §7.3; 05 §6, §7, §12, §14).
//!
//! The expression language lives in `clash-expr`, which knows nothing
//! about files. This module is where the two meet: the field types a
//! definition struct holds expressions in, the stage of the loader that
//! compiles them, the `function` and `variable` definition types, and
//! what evaluates a compiled field.
//!
//! # For a crate that owns definition types
//!
//! Hold expressions in the field types, and say in
//! [`Def::compile`](crate::Def::compile) what is in scope in each:
//!
//! ```
//! use clash_core::define_id;
//! use clash_data::expr::Number;
//! use clash_data::{CompileCx, Cond, Def, Expr, Scope, Text};
//!
//! define_id! {
//!     /// A trap.
//!     pub struct TrapId;
//! }
//!
//! /// Something that goes off when a condition holds.
//! #[derive(Debug, serde::Deserialize)]
//! pub struct TrapDef {
//!     #[serde(default)]
//!     pub when: Cond,
//!     pub energy: Expr<Number>,
//!     pub log: Text,
//! }
//!
//! impl Def for TrapDef {
//!     type Id = TrapId;
//!     const TYPE: &'static str = "trap";
//!
//!     fn compile(&mut self, cx: &mut CompileCx<'_>) {
//!         let scope = Scope::new().subject("self", "entity").subject("here", "loc");
//!         cx.compile(&mut self.when, &scope);
//!         cx.compile(&mut self.energy, &scope);
//!         cx.compile(&mut self.log, &scope.clone().pure());
//!     }
//! }
//! ```
//!
//! | Field type | Holds |
//! |------------|-------|
//! | [`Expr<T>`] | A literal, an expression or a value source, giving a `T`: [`Number`], [`Int`], [`Bool`], [`Str`], [`Any`] or [`IdOf<D>`](IdOf) |
//! | [`Cond`] | A condition: an expression, or `all`, `any` and `not` of them |
//! | [`Text`](crate::Text) | Player-facing text with `{expression}` templates |
//!
//! A [`Scope`] says what a field may name: the subjects bound there and
//! their kinds, the locals, the event, whether the value must be pure,
//! and whether it is a mind context (05 §7.3). An expression that names
//! something its field does not offer fails at load, with file, line and
//! column inside the string, the definition and the JSON path.
//!
//! # For a crate that loads
//!
//! The simulation fills a [`clash_expr::Registry`] with its kinds,
//! properties, functions, events and verbs, and gives it to the loader
//! with [`Loader::expr_registry`](crate::Loader::expr_registry). Call
//! [`register_defs`] so that mods can define functions and variables.
//! The loader adds to a copy of the registry what the mods define (the
//! ids of every definition type, `function` definitions, `variable`
//! declarations and whatever a crate's
//! [registry step](crate::DefTypes::add_registry_step) adds) and
//! compiles every field against it.
//!
//! Afterwards [`Defs::exprs`](crate::Defs::exprs) holds the finished
//! registry, the compiled functions and the variable declarations, and a
//! [`Runner`] evaluates:
//!
//! ```no_run
//! # use clash_data::{Defs, Expr, Runner};
//! # use clash_data::expr::Number;
//! # use clash_expr::testing::TestHost;
//! # fn demo(defs: &Defs, energy: &Expr<Number>, host: &mut TestHost) {
//! let mut runner = Runner::new();
//! let mut eval = runner.on(defs, host);
//! let joules = energy.eval(&mut eval); // Option<Real>
//! # let _ = joules;
//! # }
//! ```
//!
//! # What is not built yet
//!
//! Queries (05 §11). A value source over a query, `{ "exists": query }`
//! in a condition and `{ "run": … }` are read and kept, and give null
//! (or false) with a run-time warning when evaluated. Effects are the
//! rule runtime's.

pub mod codes;
mod defs;
mod eval;
mod field;
mod scope;
mod stage;

pub use clash_expr::Registry as ExprRegistry;
pub use clash_expr::{Compiled, CompiledText, Context, Host, Machine, Type, Value};

pub use codes::CODES;
pub use defs::{
    CompiledFunction, ExprDefs, FunctionDef, FunctionDefId, FunctionParam, Functions, ParamDecl,
    Params, VariableDecl, VariableDef, VariableDefId, Variables,
};
pub use eval::{DEFAULT_MAX_CALL_DEPTH, Evaluator, MAX_CALL_DEPTH_LIMIT, Runner};
pub use field::{
    Any, Bool, Compile, Cond, Expr, ExprType, IdOf, Int, Number, Query, QueryOp, Str, ValueSource,
    Written,
};
pub use scope::{CompileCx, Origin, Scope, TypeSpec};

pub(crate) use scope::CompileEnv;
pub(crate) use stage::{StageInput, compile_stage};

use crate::defs::DefTypes;

/// Registers the definition types of this module: `function` and
/// `variable` (05 §17). The application and the tools call this beside
/// each other crate's function of the same name.
pub fn register_defs(types: &mut DefTypes) {
    types.register::<FunctionDef>().register::<VariableDef>();
}
