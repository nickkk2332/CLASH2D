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
//! and [`TagSet`], [`Text`], [`Located`] and [`Gated`]. What a
//! definition struct must avoid is listed in [`de`].
//!
//! # For a crate that loads
//!
//! ```no_run
//! use clash_data::{DefTypes, Loader};
//!
//! let mut types = DefTypes::new();
//! // Each crate's `register_defs(&mut types)` goes here.
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
//! | Semantic, cross | [`defs`] | Each type's `validate`; whole-set checks |
//!
//! Every mistake is a [`Diagnostic`] with file, line, column, definition
//! and JSON path (04 §8.2), and loading reports every one it can find in
//! one pass.
//!
//! # What is not built yet
//!
//! - `Expr`, `Cond` and `Effects` fields and the compiling of [`Text`]
//!   templates (the expression packages). An expression field can be read
//!   today as a [`Located<String>`]; [`SourceMap::span_in_string`] maps an
//!   offset inside it back to the file.
//! - The `setting` and `migration` definition types, obsolete-reference
//!   warnings and save placeholders (packages P6 and P7).
//! - JSON Schema and reference generation (package P5).
//! - Patching, from another mod, an inline definition that has an `id`
//!   of its own.

pub mod de;
pub mod defs;
pub mod diag;
pub mod fields;
pub mod json;
pub mod load;
mod merge;
pub mod mods;
pub mod source;
pub mod units;

pub use de::from_node;
pub use defs::{
    CrossCheck, CrossCx, Def, DefTypes, Defs, DefsView, FinishCx, FinishedTree, LoadedModInfo,
    Registry, TagTable, TypeInfo, ValidateCx,
};
pub use diag::{
    CODES, Code, CodeInfo, DefLabel, Diagnostic, Diagnostics, Note, Severity, code, explain,
};
pub use fields::{
    Energy, Feature, Force, Frequency, Gated, HasRate, Inline, Length, Located, Mass, Power,
    Pressure, Quantity, Rate, Ref, Seconds, Speed, Tag, TagSet, Temperature, Text, Time, Volume,
    dur,
};
pub use json::{COMMENT_KEY, Limits, Member, Node, NodeKind, Num, parse_json};
pub use load::{LoadOutcome, Loader};
pub use merge::DIRECTIVE_KEYS;
pub use mods::{
    ENGINE_NAMESPACE, ENGINE_VERSION, ModManifest, Profile, Requirement, Version, VersionReq,
    is_id_part, resolve_in_mod,
};
pub use source::{FileId, Location, SourceMap, Span};
pub use units::Dim;
