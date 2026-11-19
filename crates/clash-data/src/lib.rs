//! Mod loading for CLASH2D: how JSON on disk becomes the immutable
//! definitions the simulation runs on. Specified in
//! `docs/04-data-and-modding.md`.
//!
//! This crate is the loading framework only. It knows nothing about what
//! a block or an action is: definition types live in the crate that gives
//! them meaning and register themselves here (01 §3).
//!
//! # For a crate that owns definition types
//!
//! Write a plain struct that derives `serde::Deserialize`, implement
//! [`Def`] for it (about ten lines; the example is on the trait), and
//! register it in a function the application and tools call:
//!
//! ```
//! # use clash_core::define_id;
//! # use clash_data::{Def, DefTypes};
//! # define_id! { pub struct ThingId; }
//! # #[derive(serde::Deserialize)] pub struct ThingDef {}
//! # impl Def for ThingDef { type Id = ThingId; const TYPE: &'static str = "thing"; }
//! pub fn register_defs(types: &mut DefTypes) {
//!     types.register::<ThingDef>();
//! }
//! ```
//!
//! Fields are ordinary types plus those of [`fields`]: quantities with
//! units ([`Length`], [`Mass`], [`Time`], [`Energy`], [`Power`],
//! [`Force`], [`Temperature`], [`Volume`], [`Pressure`], [`Speed`],
//! [`Rate`], [`Frequency`]), references ([`Ref`], [`Inline`]), [`Tag`]
//! and [`TagSet`], [`Text`], [`Located`] and [`Gated`], and the
//! expression fields [`Expr`] and [`Cond`]. A field that takes more than
//! one shape (an id or an object) is read with [`by_kind`]. What a
//! definition struct must avoid is listed in [`de`].
//!
//! Expressions, conditions and text templates are compiled when mods
//! load. A type says what is in scope in each such field in
//! [`Def::compile`]; [`expr`] explains how.
//!
//! # For a crate that loads
//!
//! ```no_run
//! use clash_data::{DefTypes, Loader};
//!
//! let mut types = DefTypes::new();
//! // Each crate's `register_defs(&mut types)` goes here.
//! // The simulation's registry of kinds, properties and functions goes in
//! // with `.expr_registry(&registry)`.
//! let outcome = Loader::new(&types).search_path("mods").load_profile("default");
//! eprint!("{}", outcome.diagnostics.render());
//! if let Some(defs) = outcome.defs {
//!     // An `Arc<Defs>`: immutable, `Send + Sync`, cheap to share.
//!     println!("fingerprint {}", defs.fingerprint());
//! }
//! ```
//!
//! # The pipeline
//!
//! | Stage | Module | Catches |
//! |-------|--------|---------|
//! | Mods | [`mods`] | Bad `mod.json`, missing mods, cycles |
//! | Syntax | [`json`] | Malformed JSON |
//! | Structure, merge | `merge` | Missing `type` or `id`, duplicates, bad patches, inheritance cycles |
//! | Typed load, link | [`de`], [`fields`] | Unknown keys, wrong types, bad units, unknown references |
//! | Compile | [`expr`] | Expression syntax, names, types, purity and belief; bad functions and variables |
//! | Semantic, cross | [`defs`] | Each type's `validate`; whole-set checks |
//!
//! Every mistake is a [`Diagnostic`] with file, line, column, definition
//! and JSON path (04 §8.2), and loading reports every one it can find in
//! one pass.
//!
//! # What is not built yet
//!
//! - `Effects` fields (the rule runtime) and queries. A crate that
//!   compiles a syntax of its own reads it as a [`Located<String>`] or
//!   with [`by_site`], builds its context with [`CompileCx::context`]
//!   and reports with [`CompileCx::report_compiler`].
//! - The `setting` and `migration` definition types, obsolete-reference
//!   warnings and save placeholders (packages P6 and P7).
//! - JSON Schema and reference generation (package P5).
//! - Patching, from another mod, an inline definition that has an `id`
//!   of its own.

pub mod de;
pub mod defs;
pub mod diag;
pub mod expr;
pub mod fields;
pub mod json;
pub mod load;
mod merge;
pub mod mods;
pub mod shape;
pub mod source;
pub mod units;

pub use de::{from_node, from_node_at};
pub use defs::{
    CrossCheck, CrossCx, Def, DefTypes, Defs, DefsView, FinishCx, FinishedTree, LoadedModInfo,
    Registry, RegistryCx, RegistryStep, TagTable, TypeInfo, ValidateCx,
};
pub use diag::{
    CODES, Code, CodeInfo, DefLabel, Diagnostic, Diagnostics, Note, Severity, code, explain,
};
pub use expr::{
    CompileCx, CompiledQuery, Cond, Evaluator, Expr, ExprDefs, Runner, Scope, ValueSource,
    compile_query,
};
pub use fields::{
    Energy, Feature, Force, Frequency, FromKind, FromSite, Gated, HasRate, Inline, Kind, KindValue,
    Length, Located, Mass, Power, Pressure, Quantity, Rate, Ref, Seconds, Site, Speed, Tag, TagSet,
    Temperature, Text, Time, Volume, by_kind, by_site, by_site_or_plain, dur,
};
pub use json::{COMMENT_KEY, Limits, Member, Node, NodeKind, Num, parse_json};
pub use load::{LoadOutcome, Loader};
pub use merge::{DIRECTIVE_DOCS, DIRECTIVE_KEYS};
pub use mods::{
    ENGINE_NAMESPACE, ENGINE_VERSION, ModManifest, Profile, Requirement, Version, VersionReq,
    is_id_part, resolve_in_mod,
};
pub use source::{FileId, Location, SourceMap, Span};
pub use units::Dim;
