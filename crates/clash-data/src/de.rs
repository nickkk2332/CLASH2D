//! Typed loading: a `serde` deserialiser over the spanned JSON tree
//! (04 §7).
//!
//! A definition type is a plain struct with `#[derive(serde::Deserialize)]`.
//! This deserialiser walks a finished tree, knows the JSON path and the
//! source position at every point, and turns every mistake into a
//! diagnostic with file, line, column and path:
//!
//! - **A value of the wrong kind** is reported and a harmless stand-in is
//!   fed to the type so that reading goes on and finds the next mistake.
//! - **An unknown key** is reported, with the closest known key as a
//!   suggestion, and skipped. A struct needs no `deny_unknown_fields`.
//! - **A missing field** is reported and the definition is read again
//!   with a stand-in for it, so that every missing field is found in one
//!   load.
//! - **Anything else a type refuses** is reported at the value it
//!   refused.
//!
//! The field types of [`crate::fields`] (`Ref`, quantities, `Tag`,
//! `Inline`, `Located`, `Gated`) ask for themselves under reserved names
//! that this deserialiser answers from the tree and the link tables. The
//! expression fields (`Expr`, `Cond`, `Text`) ask, through
//! [`by_site`](crate::by_site), for where they were written and at what
//! JSON path, which is what lets a mistake found when they are compiled
//! be reported like one found here.
//!
//! # What a definition type must avoid
//!
//! `#[serde(flatten)]`, `#[serde(untagged)]` and `#[serde(tag = "...")]`
//! make `serde` buffer the value and read it back through a deserialiser
//! of its own. Positions are lost, unknown keys go unnoticed and the
//! field types of this crate cannot be read inside the buffered part.
//! Use plain structs and externally tagged enums (a string, or an object
//! with one key). For a field that takes more than one shape, such as an
//! id or an object, implement [`FromKind`](crate::FromKind) and read it
//! with [`by_kind`](crate::by_kind), which keeps positions.

use std::cell::{Cell, RefCell};
use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

use serde::de::{
    self, DeserializeOwned, DeserializeSeed, Deserializer, EnumAccess, IntoDeserializer, MapAccess,
    SeqAccess, Unexpected, VariantAccess, Visitor,
};

use crate::diag::{DefLabel, Diagnostic, Diagnostics, Severity, closest, code, did_you_mean};
use crate::json::{Member, Node, NodeKind, Num};
use crate::merge::{ModTable, split_id, suggest_id};
use crate::source::{SourceMap, Span};
use crate::units::{Dim, QuantityError, micros_from_seconds, parse_quantity};

/// The reserved name [`Ref`](crate::Ref) asks for itself under.
pub(crate) const REF_MAGIC: &str = "$clash_data::Ref";
/// The reserved name [`Inline`](crate::Inline) asks for itself under.
pub(crate) const INLINE_MAGIC: &str = "$clash_data::Inline";
/// The reserved name [`Tag`](crate::Tag) asks for itself under.
pub(crate) const TAG_MAGIC: &str = "$clash_data::Tag";
/// The reserved name [`Located`](crate::Located) asks for itself under.
pub(crate) const LOCATED_MAGIC: &str = "$clash_data::Located";
/// The fields of a [`Located`](crate::Located), in order.
pub(crate) const LOCATED_FIELDS: &[&str] = &["file", "start", "end", "value"];
/// The reserved name [`Gated`](crate::Gated) asks for itself under.
pub(crate) const GATED_MAGIC: &str = "$clash_data::Gated";
/// The reserved name [`by_kind`](crate::by_kind) asks under.
pub(crate) const KIND_MAGIC: &str = "$clash_data::ByKind";
/// What [`by_kind`](crate::by_kind) is fed, in order.
pub(crate) const KIND_FIELDS: &[&str] = &["kind", "value"];
/// The reserved name [`by_site`](crate::by_site) asks under.
pub(crate) const SITE_MAGIC: &str = "$clash_data::BySite";
/// What [`by_site`](crate::by_site) is fed, in order: where the value
/// was written, its JSON path in the definition, what kind of value it
/// is and the value itself.
pub(crate) const SITE_FIELDS: &[&str] = &["file", "start", "end", "path", "kind", "value"];
/// The reserved name [`by_site_or_plain`](crate::by_site_or_plain) asks
/// under. It is asked for as a newtype struct, which every deserialiser
/// answers: this one with what [`by_site`](crate::by_site) is fed, any
/// other with itself.
pub(crate) const SITE_OR_PLAIN_MAGIC: &str = "$clash_data::BySiteOrPlain";
/// The reserved name a duration asks for itself under. It is answered in
/// whole microseconds.
pub(crate) const MICROS_MAGIC: &str = "$clash_data::quantity::micros";

/// The reserved names quantities ask for themselves under, and what each
/// measures. They are answered with an `f64` in SI units.
pub(crate) const QUANTITY_MAGICS: &[(&str, Dim)] = &[
    ("$clash_data::quantity::time", Dim::TIME),
    ("$clash_data::quantity::length", Dim::LENGTH),
    ("$clash_data::quantity::volume", Dim::VOLUME),
    ("$clash_data::quantity::mass", Dim::MASS),
    ("$clash_data::quantity::energy", Dim::ENERGY),
    ("$clash_data::quantity::force", Dim::FORCE),
    ("$clash_data::quantity::pressure", Dim::PRESSURE),
    ("$clash_data::quantity::temperature", Dim::TEMPERATURE),
    ("$clash_data::quantity::speed", Dim::SPEED),
    ("$clash_data::quantity::power", Dim::POWER),
    ("$clash_data::quantity::frequency", Dim::FREQUENCY),
    ("$clash_data::quantity::time_rate", Dim::TIME.rate()),
    ("$clash_data::quantity::volume_rate", Dim::VOLUME.rate()),
    ("$clash_data::quantity::mass_rate", Dim::MASS.rate()),
    ("$clash_data::quantity::force_rate", Dim::FORCE.rate()),
    ("$clash_data::quantity::pressure_rate", Dim::PRESSURE.rate()),
    (
        "$clash_data::quantity::temperature_rate",
        Dim::TEMPERATURE.rate(),
    ),
];

/// How many times one definition is read again to find more missing
/// fields before the loader settles for what it has.
const MAX_ATTEMPTS: usize = 64;

/// Why reading a value failed. Only [`DeError::Reported`] crosses a
/// value's boundary: everything else is turned into a diagnostic at the
/// value it happened in.
#[derive(Clone, Debug)]
pub enum DeError {
    /// A diagnostic has already been recorded.
    Reported,
    /// A required field is missing.
    Missing(&'static str),
    /// A string is not one of an enum's values.
    UnknownVariant {
        /// What was written.
        found: String,
        /// The allowed values.
        expected: &'static [&'static str],
    },
    /// A key is not one of a struct's fields. Only reaches here from a
    /// type that checks for itself.
    UnknownField {
        /// What was written.
        found: String,
        /// The known fields.
        expected: &'static [&'static str],
    },
    /// A value is of the wrong kind.
    Mismatch(String),
    /// A type refused a value, with its reason.
    Message(String),
}

impl fmt::Display for DeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            DeError::Reported => f.write_str("a diagnostic has been reported"),
            DeError::Missing(field) => write!(f, "missing field \"{field}\""),
            DeError::UnknownVariant { found, .. } => write!(f, "unknown value \"{found}\""),
            DeError::UnknownField { found, .. } => write!(f, "unknown key \"{found}\""),
            DeError::Mismatch(message) | DeError::Message(message) => f.write_str(message),
        }
    }
}

impl std::error::Error for DeError {}

impl de::Error for DeError {
    fn custom<T: fmt::Display>(message: T) -> DeError {
        DeError::Message(message.to_string())
    }

    fn missing_field(field: &'static str) -> DeError {
        DeError::Missing(field)
    }

    fn unknown_variant(variant: &str, expected: &'static [&'static str]) -> DeError {
        DeError::UnknownVariant {
            found: variant.to_owned(),
            expected,
        }
    }

    fn unknown_field(field: &str, expected: &'static [&'static str]) -> DeError {
        DeError::UnknownField {
            found: field.to_owned(),
            expected,
        }
    }

    fn invalid_type(unexpected: Unexpected<'_>, expected: &dyn de::Expected) -> DeError {
        let found = match unexpected {
            Unexpected::Bool(_) => "a boolean".to_owned(),
            Unexpected::Unsigned(_) | Unexpected::Signed(_) | Unexpected::Float(_) => {
                "a number".to_owned()
            }
            Unexpected::Str(_) | Unexpected::Char(_) => "a string".to_owned(),
            Unexpected::Unit | Unexpected::Option => "null".to_owned(),
            Unexpected::Seq => "a list".to_owned(),
            Unexpected::Map => "an object".to_owned(),
            other => other.to_string(),
        };
        DeError::Mismatch(format!("expected {expected}, found {found}"))
    }
}

/// One step of a JSON path.
#[derive(Clone, Debug)]
enum Seg {
    /// A key of an object, or the id of an entry in a list of objects
    /// that carry ids.
    Key(String),
    /// A position in a plain list.
    Index(usize),
}

fn render_path(path: &[Seg]) -> String {
    let mut out = String::new();
    for seg in path {
        match seg {
            Seg::Key(key) => {
                if !out.is_empty() {
                    out.push('.');
                }
                out.push_str(key);
            }
            Seg::Index(index) => out.push_str(&format!("[{index}]")),
        }
    }
    out
}

/// The ids of one definition type, as the link stage knows them.
#[derive(Clone, Debug, Default)]
pub(crate) struct IdTable {
    /// Qualified ids in dense order.
    pub(crate) names: Vec<String>,
    /// Qualified id to dense index.
    pub(crate) index: BTreeMap<String, u32>,
    /// Ids of abstract definitions: known, and not referable.
    pub(crate) abstracts: BTreeSet<String>,
}

/// An inline definition met while reading its parent (04 §5).
#[derive(Clone, Debug)]
pub(crate) struct InlineFound {
    pub(crate) type_name: &'static str,
    pub(crate) id: String,
    pub(crate) node: Node,
}

/// A use of an experimental feature (04 §8.4).
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct FeatureUse {
    pub(crate) feature: String,
    pub(crate) span: Span,
    pub(crate) label: Option<DefLabel>,
    pub(crate) path: String,
}

/// What reading needs from the link stage, shared by every definition of
/// one load.
pub(crate) struct LinkShared<'a> {
    pub(crate) mods: &'a ModTable,
    pub(crate) ids: &'a BTreeMap<&'static str, IdTable>,
    /// The first pass, which only finds inline definitions and tags:
    /// nothing is resolved and nothing is reported.
    pub(crate) discover: bool,
    /// Tag names in dense order. Empty while discovering.
    pub(crate) tags: &'a [String],
    pub(crate) found_tags: RefCell<BTreeSet<String>>,
    pub(crate) found_inline: RefCell<Vec<InlineFound>>,
    pub(crate) feature_uses: RefCell<BTreeSet<FeatureUse>>,
}

impl<'a> LinkShared<'a> {
    pub(crate) fn new(
        mods: &'a ModTable,
        ids: &'a BTreeMap<&'static str, IdTable>,
        tags: &'a [String],
        discover: bool,
    ) -> LinkShared<'a> {
        LinkShared {
            mods,
            ids,
            discover,
            tags,
            found_tags: RefCell::new(BTreeSet::new()),
            found_inline: RefCell::new(Vec::new()),
            feature_uses: RefCell::new(BTreeSet::new()),
        }
    }
}

/// How a reference came out.
pub(crate) enum Resolved {
    /// The dense index.
    Found(u32),
    /// Not found; this is the diagnostic to report.
    Failed(Box<Diagnostic>),
}

/// Looks an id up the way a reference written at `span` sees it
/// (04 §4.3). `warnings` receives the shadowed-import warning.
pub(crate) fn resolve_id(
    link: &LinkShared<'_>,
    sources: &SourceMap,
    type_name: &str,
    text: &str,
    span: Span,
    warnings: &mut Vec<Diagnostic>,
) -> Resolved {
    let Some((namespace, name)) = split_id(text) else {
        let mut diagnostic =
            Diagnostic::new(code::BAD_REFERENCE, format!("\"{text}\" is not an id"))
                .at(sources, span);
        diagnostic = if text.contains('#') {
            diagnostic
                .suggest("an inline definition cannot be referred to; give it an `id` of its own")
        } else {
            diagnostic.suggest("a reference is `name` or `namespace:name`")
        };
        return Resolved::Failed(Box::new(diagnostic));
    };
    let Some(table) = link.ids.get(type_name) else {
        return Resolved::Failed(Box::new(
            Diagnostic::new(
                code::UNKNOWN_REFERENCE,
                format!(
                    "no {type_name} \"{text}\": the engine has no definition type `{type_name}`"
                ),
            )
            .at(sources, span),
        ));
    };
    let candidates: Vec<String> = match namespace {
        Some(namespace) => vec![format!("{namespace}:{name}")],
        None => link
            .mods
            .search_order(span.file)
            .into_iter()
            .map(|namespace| format!("{namespace}:{name}"))
            .collect(),
    };
    let mut found: Option<(u32, &str)> = None;
    for candidate in &candidates {
        if let Some(&index) = table.index.get(candidate) {
            match found {
                None => found = Some((index, candidate)),
                Some((_, first)) => {
                    warnings.push(
                        Diagnostic::new(
                            code::SHADOWED_IMPORT,
                            format!(
                                "\"{text}\" means \"{first}\" here, and \"{candidate}\" also exists"
                            ),
                        )
                        .at(sources, span)
                        .suggest(format!("write \"{first}\" to be clear")),
                    );
                    break;
                }
            }
        }
    }
    if let Some((index, _)) = found {
        return Resolved::Found(index);
    }
    if let Some(candidate) = candidates
        .iter()
        .find(|candidate| table.abstracts.contains(*candidate))
    {
        return Resolved::Failed(Box::new(
            Diagnostic::new(
                code::ABSTRACT_REFERENCE,
                format!("{type_name} \"{candidate}\" is abstract and cannot be referred to"),
            )
            .at(sources, span)
            .suggest("an abstract definition exists only for `copy_from`"),
        ));
    }
    let own = link
        .mods
        .manifest_of(span.file)
        .map(|manifest| manifest.id.as_str());
    let suggestion = suggest_id(
        text,
        table
            .names
            .iter()
            .filter(|name| !name.contains('#'))
            .map(String::as_str),
        own,
    );
    Resolved::Failed(Box::new(
        Diagnostic::new(
            code::UNKNOWN_REFERENCE,
            format!("no {type_name} \"{text}\""),
        )
        .at(sources, span)
        .suggest_opt(did_you_mean(suggestion.as_deref())),
    ))
}

/// The state of reading one definition.
pub(crate) struct DeCx<'a> {
    pub(crate) sources: &'a SourceMap,
    pub(crate) label: Option<DefLabel>,
    pub(crate) link: Option<&'a LinkShared<'a>>,
    diagnostics: RefCell<Vec<Diagnostic>>,
    path: RefCell<Vec<Seg>>,
    /// The JSON path of the tree being read inside its definition, for a
    /// tree that is part of one. Empty for a whole definition.
    base: String,
    /// Missing fields to stand in for, by the path of their struct.
    inject: RefCell<BTreeMap<String, Vec<&'static str>>>,
    /// A missing field found in this attempt.
    new_inject: RefCell<Option<(String, &'static str)>>,
    failed: Cell<bool>,
    /// Above zero while a stand-in is being fed: errors there are already
    /// accounted for.
    standing_in: Cell<u32>,
    found_inline: RefCell<Vec<InlineFound>>,
    feature_uses: RefCell<Vec<FeatureUse>>,
}

impl<'a> DeCx<'a> {
    pub(crate) fn new(
        sources: &'a SourceMap,
        label: Option<DefLabel>,
        link: Option<&'a LinkShared<'a>>,
    ) -> DeCx<'a> {
        DeCx {
            sources,
            label,
            link,
            diagnostics: RefCell::new(Vec::new()),
            path: RefCell::new(Vec::new()),
            base: String::new(),
            inject: RefCell::new(BTreeMap::new()),
            new_inject: RefCell::new(None),
            failed: Cell::new(false),
            standing_in: Cell::new(0),
            found_inline: RefCell::new(Vec::new()),
            feature_uses: RefCell::new(Vec::new()),
        }
    }

    fn discovering(&self) -> bool {
        self.link.is_some_and(|link| link.discover)
    }

    fn path_string(&self) -> String {
        let inner = render_path(&self.path.borrow());
        if self.base.is_empty() {
            inner
        } else if inner.is_empty() {
            self.base.clone()
        } else if inner.starts_with('[') {
            format!("{}{inner}", self.base)
        } else {
            format!("{}.{inner}", self.base)
        }
    }

    /// Records a diagnostic at the current path.
    fn push(&self, diagnostic: Diagnostic) {
        if diagnostic.severity == Severity::Error {
            self.failed.set(true);
        }
        if self.standing_in.get() > 0 {
            return;
        }
        let diagnostic = diagnostic
            .in_def(self.label.as_ref())
            .with_path(self.path_string());
        self.diagnostics.borrow_mut().push(diagnostic);
    }

    fn error_at(&self, span: Span, diagnostic: Diagnostic) {
        self.push(diagnostic.at(self.sources, span));
    }

    /// Turns whatever went wrong in the value at `node` into a diagnostic
    /// there.
    fn settle<T>(&self, node: &Node, result: Result<T, DeError>) -> Result<T, DeError> {
        let error = match result {
            Ok(value) => return Ok(value),
            Err(error) => error,
        };
        self.failed.set(true);
        if self.standing_in.get() > 0 {
            return Err(DeError::Reported);
        }
        match error {
            DeError::Reported => {}
            DeError::Missing(field) => {
                self.report_missing(node, field);
                *self.new_inject.borrow_mut() = Some((self.path_string(), field));
            }
            DeError::UnknownVariant { found, expected } => {
                let list: Vec<String> = expected.iter().map(|name| format!("\"{name}\"")).collect();
                self.error_at(
                    node.span,
                    Diagnostic::new(
                        code::UNKNOWN_VARIANT,
                        format!("\"{found}\" is not one of the values allowed here"),
                    )
                    .suggest_opt(did_you_mean(closest(&found, expected.iter().copied())))
                    .note(format!("allowed: {}", list.join(", "))),
                );
            }
            DeError::UnknownField { found, expected } => {
                let span = node
                    .member(&found)
                    .map_or(node.span, |member| member.key_span);
                self.error_at(
                    span,
                    Diagnostic::new(code::UNKNOWN_KEY, format!("unknown key \"{found}\""))
                        .suggest_opt(did_you_mean(closest(&found, expected.iter().copied()))),
                );
            }
            DeError::Mismatch(message) => {
                self.error_at(node.span, Diagnostic::new(code::TYPE_MISMATCH, message));
            }
            DeError::Message(message) => {
                self.error_at(node.span, Diagnostic::new(code::INVALID_VALUE, message));
            }
        }
        Err(DeError::Reported)
    }

    fn report_missing(&self, node: &Node, field: &'static str) {
        // Point at the opening brace, not the whole object.
        let span = Span::new(node.span.file, node.span.start, node.span.start + 1);
        self.error_at(
            span,
            Diagnostic::new(code::MISSING_FIELD, format!("missing field \"{field}\"")),
        );
    }

    /// Reads a child value at one more step of the path.
    fn child<T>(
        &self,
        node: &Node,
        seg: Seg,
        read: impl FnOnce(NodeDe<'_>) -> Result<T, DeError>,
    ) -> Result<T, DeError> {
        self.path.borrow_mut().push(seg);
        let result = read(NodeDe {
            node,
            cx: self,
            root: false,
            key: false,
        });
        let result = self.settle(node, result);
        self.path.borrow_mut().pop();
        result
    }

    /// Feeds a stand-in. Whatever goes wrong while it is fed has already
    /// been accounted for.
    fn stand_in<T>(&self, read: impl FnOnce(StandIn) -> Result<T, DeError>) -> Result<T, DeError> {
        self.failed.set(true);
        self.standing_in.set(self.standing_in.get() + 1);
        let result = read(StandIn);
        self.standing_in.set(self.standing_in.get() - 1);
        result.map_err(|_| DeError::Reported)
    }
}

/// What reading one definition produced besides the value.
pub(crate) struct ReadOutcome<T> {
    /// The value, if the definition is sound.
    pub(crate) value: Option<T>,
    /// What there was to say.
    pub(crate) diagnostics: Vec<Diagnostic>,
}

/// Reads `node` as a `T`, finding every mistake it can.
pub(crate) fn read_def<T: DeserializeOwned>(node: &Node, cx: &DeCx<'_>) -> ReadOutcome<T> {
    let mut value = None;
    for _ in 0..MAX_ATTEMPTS {
        cx.diagnostics.borrow_mut().clear();
        cx.path.borrow_mut().clear();
        cx.found_inline.borrow_mut().clear();
        cx.feature_uses.borrow_mut().clear();
        cx.failed.set(false);
        cx.standing_in.set(0);
        *cx.new_inject.borrow_mut() = None;
        let result = cx.settle(
            node,
            T::deserialize(NodeDe {
                node,
                cx,
                root: true,
                key: false,
            }),
        );
        match result {
            Ok(read) => {
                value = (!cx.failed.get()).then_some(read);
                break;
            }
            Err(_) => {
                let Some((path, field)) = cx.new_inject.borrow_mut().take() else {
                    break;
                };
                let mut inject = cx.inject.borrow_mut();
                let fields = inject.entry(path).or_default();
                if fields.contains(&field) {
                    break;
                }
                fields.push(field);
            }
        }
    }
    if let Some(link) = cx.link {
        link.found_inline
            .borrow_mut()
            .append(&mut cx.found_inline.borrow_mut());
        link.feature_uses
            .borrow_mut()
            .extend(cx.feature_uses.borrow_mut().drain(..));
    }
    let diagnostics = std::mem::take(&mut *cx.diagnostics.borrow_mut());
    ReadOutcome { value, diagnostics }
}

/// Reads any tree as a `T`, reporting mistakes with their positions.
///
/// This is the typed loader without the link stage, for trees that are
/// not definitions: a profile's `settings`, a tool's own file. References,
/// tags and inline definitions cannot be read this way.
///
/// ```
/// use clash_data::{from_node, parse_json, Diagnostics, Limits, SourceMap};
///
/// #[derive(serde::Deserialize)]
/// struct Window { width: u32, height: u32 }
///
/// let mut sources = SourceMap::new();
/// let file = sources.add("window.json", r#"{ "width": 80, "hieght": 24 }"#);
/// let (tree, _) = parse_json(file, sources.text(file), &Limits::default());
/// let mut diagnostics = Diagnostics::new();
/// let window: Option<Window> = from_node(&tree.unwrap(), &sources, &mut diagnostics, None);
/// assert!(window.is_none());
/// let text = diagnostics.render();
/// assert!(text.contains("unknown key \"hieght\""));
/// assert!(text.contains("did you mean \"height\"?"));
/// assert!(text.contains("missing field \"height\""));
/// ```
pub fn from_node<T: DeserializeOwned>(
    node: &Node,
    sources: &SourceMap,
    diagnostics: &mut Diagnostics,
    label: Option<&DefLabel>,
) -> Option<T> {
    let cx = DeCx::new(sources, label.cloned(), None);
    let outcome = read_def::<T>(node, &cx);
    diagnostics.extend(outcome.diagnostics);
    outcome.value
}

/// Reads a tree that is part of a definition as a `T`, as [`from_node`]
/// does, given the JSON path of the tree inside its definition.
///
/// A definition type that keeps part of itself as a tree and reads it
/// when it is compiled, as a list of effects is, reads each piece with
/// this, so that a field read by [`by_site`](crate::by_site) inside it
/// (an [`Expr`](crate::Expr), a [`Cond`](crate::Cond), a
/// [`Text`](crate::Text)) knows its whole path, and a mistake is
/// reported at it.
///
/// ```
/// use clash_data::{from_node_at, parse_json, Cond, Diagnostics, Limits, SourceMap};
///
/// let mut sources = SourceMap::new();
/// let file = sources.add("rule.json", r#"{ "if": "self.mass > 3" }"#);
/// let (tree, _) = parse_json(file, sources.text(file), &Limits::default());
/// let tree = tree.unwrap();
/// let mut diagnostics = Diagnostics::new();
/// let cond: Option<Cond> =
///     from_node_at(tree.get("if").unwrap(), &sources, &mut diagnostics, None, "do[2].if");
/// assert_eq!(cond.unwrap().site().path, "do[2].if");
/// ```
pub fn from_node_at<T: DeserializeOwned>(
    node: &Node,
    sources: &SourceMap,
    diagnostics: &mut Diagnostics,
    label: Option<&DefLabel>,
    path: &str,
) -> Option<T> {
    let mut cx = DeCx::new(sources, label.cloned(), None);
    path.clone_into(&mut cx.base);
    let outcome = read_def::<T>(node, &cx);
    diagnostics.extend(outcome.diagnostics);
    outcome.value
}

/// The deserialiser for one value of the tree.
pub(crate) struct NodeDe<'c> {
    node: &'c Node,
    cx: &'c DeCx<'c>,
    /// The top of a definition: `type` and `id` are not fields.
    root: bool,
    /// An object key standing as a value: numbers are read out of the
    /// string.
    key: bool,
}

/// The keys at the top of a definition that are not its fields. Finished
/// trees have none; an inline definition may carry them.
const ROOT_KEYS: &[&str] = &["type", "id"];

/// The stand-in for a quantity that could not be read: zero, in the form
/// the field asked for.
fn zero_quantity<'de, V: Visitor<'de>>(
    _stand_in: StandIn,
    micros: bool,
    visitor: V,
) -> Result<V::Value, DeError> {
    if micros {
        visitor.visit_i64(0)
    } else {
        visitor.visit_f64(0.0)
    }
}

impl<'c> NodeDe<'c> {
    /// Reports a value of the wrong kind and feeds a stand-in.
    fn mismatch<T>(
        self,
        expected: &str,
        feed: impl FnOnce(StandIn) -> Result<T, DeError>,
    ) -> Result<T, DeError> {
        self.cx.error_at(
            self.node.span,
            Diagnostic::new(
                code::TYPE_MISMATCH,
                format!("expected {expected}, found {}", self.node.kind_name()),
            ),
        );
        self.cx.stand_in(feed)
    }

    /// The value as a whole number, if it is one.
    fn whole(&self) -> Result<i128, Option<String>> {
        match &self.node.kind {
            NodeKind::Number(Num::Int(value)) => Ok(i128::from(*value)),
            NodeKind::Number(Num::UInt(value)) => Ok(i128::from(*value)),
            NodeKind::Number(Num::Float(value)) => {
                if value.fract() == 0.0 && value.abs() < 1.0e18 {
                    Ok(*value as i128)
                } else {
                    Err(Some(format!(
                        "this field holds a whole number, and {value} is not one"
                    )))
                }
            }
            NodeKind::String(text) if self.key => text.parse::<i128>().map_err(|_| None),
            _ => Err(None),
        }
    }

    fn members(&self) -> Option<&'c [Member]> {
        self.node.as_object()
    }

    fn visit_object<'de, V: Visitor<'de>>(
        self,
        fields: Option<&'static [&'static str]>,
        visitor: V,
    ) -> Result<V::Value, DeError> {
        let Some(members) = self.members() else {
            return self.mismatch("an object", |stand_in| stand_in.deserialize_map(visitor));
        };
        let injected = if self.cx.inject.borrow().is_empty() {
            Vec::new()
        } else {
            self.cx
                .inject
                .borrow()
                .get(&self.cx.path_string())
                .cloned()
                .unwrap_or_default()
        };
        if let Some(fields) = fields {
            for member in members {
                let key = member.key.as_str();
                if fields.contains(&key) || (self.root && ROOT_KEYS.contains(&key)) {
                    continue;
                }
                self.cx.error_at(
                    member.key_span,
                    Diagnostic::new(code::UNKNOWN_KEY, format!("unknown key \"{key}\""))
                        .suggest_opt(did_you_mean(closest(key, fields.iter().copied()))),
                );
            }
        }
        visitor.visit_map(MapDe {
            node: self.node,
            members,
            at: 0,
            cx: self.cx,
            fields,
            root: self.root,
            injected,
            inject_at: 0,
        })
    }

    fn visit_array<'de, V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, DeError> {
        let Some(items) = self.node.as_array() else {
            return self.mismatch("a list", |stand_in| stand_in.deserialize_seq(visitor));
        };
        let mut seq = SeqDe {
            items,
            at: 0,
            cx: self.cx,
        };
        let value = visitor.visit_seq(&mut seq)?;
        if seq.at < items.len() {
            return Err(DeError::Message(format!(
                "this list has {} entries, and only {} belong here",
                items.len(),
                seq.at
            )));
        }
        Ok(value)
    }

    fn quantity<'de, V: Visitor<'de>>(
        self,
        want: Dim,
        micros: bool,
        visitor: V,
    ) -> Result<V::Value, DeError> {
        let feed = |value: f64, visitor: V| -> Result<V::Value, DeError> {
            if micros {
                match micros_from_seconds(value) {
                    Some(micros) => visitor.visit_i64(micros),
                    None => Err(DeError::Message(
                        "this time is too long to represent".to_owned(),
                    )),
                }
            } else {
                visitor.visit_f64(value)
            }
        };
        let wrong_dimension = |found: Dim, shown: String| {
            Diagnostic::new(
                code::WRONG_DIMENSION,
                format!(
                    "{shown} is {}, but this field holds {}",
                    found.name(),
                    want.name()
                ),
            )
            .suggest(format!("use one of: {}", want.accepted_units()))
        };
        match &self.node.kind {
            NodeKind::Number(number) => feed(number.as_f64(), visitor),
            NodeKind::Quantity { value, dim } => {
                if *dim != want {
                    self.cx.error_at(
                        self.node.span,
                        wrong_dimension(*dim, "this value".to_owned()),
                    );
                    return self
                        .cx
                        .stand_in(|stand_in| zero_quantity(stand_in, micros, visitor));
                }
                feed(*value, visitor)
            }
            NodeKind::String(text) => match parse_quantity(text) {
                Ok(quantity) => {
                    if quantity.dim != want {
                        self.cx.error_at(
                            self.node.span,
                            wrong_dimension(quantity.dim, format!("\"{text}\"")),
                        );
                        return self
                            .cx
                            .stand_in(|stand_in| zero_quantity(stand_in, micros, visitor));
                    }
                    match (micros, quantity.as_micros()) {
                        (true, Some(micros)) => visitor.visit_i64(micros),
                        _ => feed(quantity.value, visitor),
                    }
                }
                Err(error) => {
                    let (message, suggestion) = match &error {
                        QuantityError::UnknownUnit { unit, closest } if !unit.is_empty() => (
                            error.to_string(),
                            did_you_mean(*closest)
                                .or_else(|| Some(format!("use one of: {}", want.accepted_units()))),
                        ),
                        QuantityError::UnknownUnit { .. } => (
                            format!("\"{text}\" has no unit"),
                            Some(format!(
                                "write a bare number in SI units without quotation marks, or add one of: {}",
                                want.accepted_units()
                            )),
                        ),
                        _ => (
                            format!("\"{text}\" is not a quantity: {error}"),
                            Some(format!(
                                "this field holds {}: one of {}",
                                want.name(),
                                want.accepted_units()
                            )),
                        ),
                    };
                    self.cx.error_at(
                        self.node.span,
                        Diagnostic::new(code::BAD_UNIT, message).suggest_opt(suggestion),
                    );
                    self.cx
                        .stand_in(|stand_in| zero_quantity(stand_in, micros, visitor))
                }
            },
            _ => self.mismatch("a number in SI units or a string with a unit", |stand_in| {
                zero_quantity(stand_in, micros, visitor)
            }),
        }
    }

    /// Answers a [`Ref`](crate::Ref): the dense index of the definition
    /// the string names.
    fn reference<'de, V: Visitor<'de>>(
        self,
        type_name: &'static str,
        visitor: V,
    ) -> Result<V::Value, DeError> {
        let Some(text) = self.node.as_str() else {
            return self.mismatch("an id", |stand_in| stand_in.deserialize_u32(visitor));
        };
        let Some(link) = self.cx.link else {
            return Err(DeError::Message(
                "a reference to a definition cannot be read outside the mod loader".to_owned(),
            ));
        };
        if link.discover {
            return visitor.visit_u32(0);
        }
        let mut warnings = Vec::new();
        let resolved = resolve_id(
            link,
            self.cx.sources,
            type_name,
            text,
            self.node.span,
            &mut warnings,
        );
        for warning in warnings {
            self.cx.push(warning);
        }
        match resolved {
            Resolved::Found(index) => visitor.visit_u32(index),
            Resolved::Failed(diagnostic) => {
                self.cx.push(*diagnostic);
                self.cx
                    .stand_in(|stand_in| stand_in.deserialize_u32(visitor))
            }
        }
    }

    /// Answers an [`Inline`](crate::Inline): a reference, or a definition
    /// written in place, which gets the id `<parent id>#<path>` (04 §5).
    fn inline<'de, V: Visitor<'de>>(
        self,
        type_name: &'static str,
        visitor: V,
    ) -> Result<V::Value, DeError> {
        if self.node.as_str().is_some() {
            return self.reference(type_name, visitor);
        }
        if self.node.as_object().is_none() {
            return self.mismatch("an id or a definition written in place", |stand_in| {
                stand_in.deserialize_u32(visitor)
            });
        }
        let Some(link) = self.cx.link else {
            return Err(DeError::Message(
                "an inline definition cannot be read outside the mod loader".to_owned(),
            ));
        };
        if let Some(written) = self.node.get("type")
            && written.as_str() != Some(type_name)
        {
            self.cx.error_at(
                written.span,
                Diagnostic::new(
                    code::TYPE_MISMATCH,
                    format!("a definition written in place here is a {type_name}"),
                )
                .suggest("leave `type` out"),
            );
            return self
                .cx
                .stand_in(|stand_in| stand_in.deserialize_u32(visitor));
        }
        let id = match self.node.get("id") {
            // An id of its own: in the namespace of the mod that wrote it.
            Some(written) => {
                let own = link
                    .mods
                    .manifest_of(written.span.file)
                    .map(|manifest| manifest.id.as_str());
                match (written.as_str().and_then(split_id), own) {
                    (Some((None, name)), Some(own)) => format!("{own}:{name}"),
                    (Some((Some(namespace), name)), Some(own)) if namespace == own => {
                        format!("{own}:{name}")
                    }
                    _ => {
                        self.cx.error_at(
                            written.span,
                            Diagnostic::new(
                                code::BAD_ID,
                                "the id of a definition written in place is a bare name in the mod's own namespace",
                            ),
                        );
                        return self
                            .cx
                            .stand_in(|stand_in| stand_in.deserialize_u32(visitor));
                    }
                }
            }
            None => {
                let parent = self.cx.label.as_ref().map_or("", |label| label.id.as_str());
                format!("{parent}#{}", self.cx.path_string())
            }
        };
        if link.discover {
            self.cx.found_inline.borrow_mut().push(InlineFound {
                type_name,
                id,
                node: self.node.clone(),
            });
            return visitor.visit_u32(0);
        }
        match link
            .ids
            .get(type_name)
            .and_then(|table| table.index.get(&id))
        {
            Some(&index) => visitor.visit_u32(index),
            // Not found by the first pass, which stopped where this one
            // will: the definition is already broken.
            None => self
                .cx
                .stand_in(|stand_in| stand_in.deserialize_u32(visitor)),
        }
    }

    /// Answers a [`Tag`](crate::Tag): the dense index of the string.
    fn tag<'de, V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, DeError> {
        let Some(text) = self.node.as_str() else {
            return self.mismatch("a tag", |stand_in| stand_in.deserialize_u32(visitor));
        };
        let Some(link) = self.cx.link else {
            return Err(DeError::Message(
                "a tag cannot be read outside the mod loader".to_owned(),
            ));
        };
        if text.is_empty() {
            return Err(DeError::Message("a tag cannot be empty".to_owned()));
        }
        if link.discover {
            link.found_tags.borrow_mut().insert(text.to_owned());
            return visitor.visit_u32(0);
        }
        match link.tags.binary_search_by(|name| name.as_str().cmp(text)) {
            Ok(index) => visitor.visit_u32(crate::source::to_u32(index)),
            Err(_) => self
                .cx
                .stand_in(|stand_in| stand_in.deserialize_u32(visitor)),
        }
    }
}

macro_rules! whole_number {
    ($($method:ident, $visit:ident, $ty:ty;)*) => {
        $(
            fn $method<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, DeError> {
                match self.whole() {
                    Ok(value) => match <$ty>::try_from(value) {
                        Ok(value) => visitor.$visit(value),
                        Err(_) => {
                            self.cx.error_at(
                                self.node.span,
                                Diagnostic::new(
                                    code::NUMBER_OUT_OF_RANGE,
                                    format!(
                                        "{value} does not fit: this field holds a whole number from {} to {}",
                                        <$ty>::MIN,
                                        <$ty>::MAX
                                    ),
                                ),
                            );
                            self.cx.stand_in(|stand_in| stand_in.$method(visitor))
                        }
                    },
                    Err(Some(message)) => {
                        self.cx.error_at(
                            self.node.span,
                            Diagnostic::new(code::NUMBER_OUT_OF_RANGE, message),
                        );
                        self.cx.stand_in(|stand_in| stand_in.$method(visitor))
                    }
                    Err(None) => self.mismatch("a whole number", |stand_in| stand_in.$method(visitor)),
                }
            }
        )*
    };
}

impl<'de, 'c> Deserializer<'de> for NodeDe<'c> {
    type Error = DeError;

    fn deserialize_any<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, DeError> {
        match &self.node.kind {
            NodeKind::Null => visitor.visit_unit(),
            NodeKind::Bool(value) => visitor.visit_bool(*value),
            NodeKind::Number(Num::Int(value)) => visitor.visit_i64(*value),
            NodeKind::Number(Num::UInt(value)) => visitor.visit_u64(*value),
            NodeKind::Number(Num::Float(value)) => visitor.visit_f64(*value),
            NodeKind::Quantity { value, .. } => visitor.visit_f64(*value),
            NodeKind::String(text) => visitor.visit_str(text),
            NodeKind::Array(_) => self.visit_array(visitor),
            NodeKind::Object(_) => self.visit_object(None, visitor),
        }
    }

    fn deserialize_bool<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, DeError> {
        match self.node.as_bool() {
            Some(value) => visitor.visit_bool(value),
            None => self.mismatch("`true` or `false`", |stand_in| {
                stand_in.deserialize_bool(visitor)
            }),
        }
    }

    whole_number! {
        deserialize_i8, visit_i8, i8;
        deserialize_i16, visit_i16, i16;
        deserialize_i32, visit_i32, i32;
        deserialize_i64, visit_i64, i64;
        deserialize_u8, visit_u8, u8;
        deserialize_u16, visit_u16, u16;
        deserialize_u32, visit_u32, u32;
        deserialize_u64, visit_u64, u64;
    }

    fn deserialize_f32<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, DeError> {
        self.deserialize_f64(visitor)
    }

    fn deserialize_f64<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, DeError> {
        match &self.node.kind {
            NodeKind::Number(number) => visitor.visit_f64(number.as_f64()),
            NodeKind::String(text) if self.key => match text.parse::<f64>() {
                Ok(value) if value.is_finite() => visitor.visit_f64(value),
                _ => self.mismatch("a number", |stand_in| stand_in.deserialize_f64(visitor)),
            },
            NodeKind::String(text) if parse_quantity(text).is_ok() => {
                self.cx.error_at(
                    self.node.span,
                    Diagnostic::new(
                        code::TYPE_MISMATCH,
                        format!("expected a plain number, found the quantity \"{text}\""),
                    )
                    .suggest(
                        "this field takes no unit; write the number alone, without quotation marks",
                    ),
                );
                self.cx
                    .stand_in(|stand_in| stand_in.deserialize_f64(visitor))
            }
            _ => self.mismatch("a number", |stand_in| stand_in.deserialize_f64(visitor)),
        }
    }

    fn deserialize_char<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, DeError> {
        self.deserialize_str(visitor)
    }

    fn deserialize_str<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, DeError> {
        match self.node.as_str() {
            Some(text) => visitor.visit_str(text),
            None => self.mismatch("a string", |stand_in| stand_in.deserialize_str(visitor)),
        }
    }

    fn deserialize_string<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, DeError> {
        self.deserialize_str(visitor)
    }

    fn deserialize_bytes<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, DeError> {
        self.deserialize_any(visitor)
    }

    fn deserialize_byte_buf<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, DeError> {
        self.deserialize_any(visitor)
    }

    fn deserialize_option<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, DeError> {
        match self.node.kind {
            NodeKind::Null => visitor.visit_none(),
            _ => visitor.visit_some(self),
        }
    }

    fn deserialize_unit<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, DeError> {
        match self.node.kind {
            NodeKind::Null => visitor.visit_unit(),
            _ => self.mismatch("null", |stand_in| stand_in.deserialize_unit(visitor)),
        }
    }

    fn deserialize_unit_struct<V: Visitor<'de>>(
        self,
        _name: &'static str,
        visitor: V,
    ) -> Result<V::Value, DeError> {
        self.deserialize_unit(visitor)
    }

    fn deserialize_newtype_struct<V: Visitor<'de>>(
        self,
        name: &'static str,
        visitor: V,
    ) -> Result<V::Value, DeError> {
        if name == TAG_MAGIC {
            return self.tag(visitor);
        }
        if name == MICROS_MAGIC {
            return self.quantity(Dim::TIME, true, visitor);
        }
        if let Some(&(_, dim)) = QUANTITY_MAGICS.iter().find(|(magic, _)| *magic == name) {
            return self.quantity(dim, false, visitor);
        }
        if name == SITE_OR_PLAIN_MAGIC {
            return visitor.visit_map(SiteDe {
                node: self.node,
                cx: self.cx,
                root: self.root,
                at: 0,
            });
        }
        visitor.visit_newtype_struct(self)
    }

    fn deserialize_seq<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, DeError> {
        self.visit_array(visitor)
    }

    fn deserialize_tuple<V: Visitor<'de>>(
        self,
        _len: usize,
        visitor: V,
    ) -> Result<V::Value, DeError> {
        self.visit_array(visitor)
    }

    fn deserialize_tuple_struct<V: Visitor<'de>>(
        self,
        _name: &'static str,
        _len: usize,
        visitor: V,
    ) -> Result<V::Value, DeError> {
        self.visit_array(visitor)
    }

    fn deserialize_map<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, DeError> {
        self.visit_object(None, visitor)
    }

    fn deserialize_struct<V: Visitor<'de>>(
        self,
        name: &'static str,
        fields: &'static [&'static str],
        visitor: V,
    ) -> Result<V::Value, DeError> {
        match name {
            REF_MAGIC => self.reference(fields.first().copied().unwrap_or(""), visitor),
            INLINE_MAGIC => self.inline(fields.first().copied().unwrap_or(""), visitor),
            GATED_MAGIC => {
                if !self.cx.discovering() && self.cx.link.is_some() {
                    self.cx.feature_uses.borrow_mut().push(FeatureUse {
                        feature: fields.first().copied().unwrap_or("").to_owned(),
                        span: self.node.span,
                        label: self.cx.label.clone(),
                        path: self.cx.path_string(),
                    });
                }
                visitor.visit_newtype_struct(self)
            }
            LOCATED_MAGIC => visitor.visit_map(LocatedDe {
                node: self.node,
                cx: self.cx,
                root: self.root,
                at: 0,
            }),
            KIND_MAGIC => visitor.visit_map(KindDe {
                node: self.node,
                cx: self.cx,
                root: self.root,
                at: 0,
            }),
            SITE_MAGIC => visitor.visit_map(SiteDe {
                node: self.node,
                cx: self.cx,
                root: self.root,
                at: 0,
            }),
            _ => self.visit_object(Some(fields), visitor),
        }
    }

    fn deserialize_enum<V: Visitor<'de>>(
        self,
        _name: &'static str,
        _variants: &'static [&'static str],
        visitor: V,
    ) -> Result<V::Value, DeError> {
        match &self.node.kind {
            NodeKind::String(text) => visitor.visit_enum(EnumDe {
                variant: text,
                value: None,
                cx: self.cx,
            }),
            NodeKind::Object(members) if members.len() == 1 => visitor.visit_enum(EnumDe {
                variant: &members[0].key,
                value: Some(&members[0].value),
                cx: self.cx,
            }),
            _ => Err(DeError::Mismatch(format!(
                "expected a string or an object with one key, found {}",
                self.node.kind_name()
            ))),
        }
    }

    fn deserialize_identifier<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, DeError> {
        self.deserialize_str(visitor)
    }

    fn deserialize_ignored_any<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, DeError> {
        visitor.visit_unit()
    }
}

struct MapDe<'c> {
    node: &'c Node,
    members: &'c [Member],
    at: usize,
    cx: &'c DeCx<'c>,
    fields: Option<&'static [&'static str]>,
    root: bool,
    injected: Vec<&'static str>,
    inject_at: usize,
}

impl MapDe<'_> {
    fn skipped(&self, key: &str) -> bool {
        (self.root && ROOT_KEYS.contains(&key))
            || self.fields.is_some_and(|fields| !fields.contains(&key))
    }
}

impl<'de> MapAccess<'de> for MapDe<'_> {
    type Error = DeError;

    fn next_key_seed<K: DeserializeSeed<'de>>(
        &mut self,
        seed: K,
    ) -> Result<Option<K::Value>, DeError> {
        while let Some(member) = self.members.get(self.at) {
            if self.skipped(&member.key) {
                self.at += 1;
                continue;
            }
            // The key stands as a value of its own, so that a key that is
            // a reference or a number is read, and reported, like one.
            let key_node = Node::new(NodeKind::String(member.key.clone()), member.key_span);
            let cx = self.cx;
            cx.path.borrow_mut().push(Seg::Key(member.key.clone()));
            let result = seed.deserialize(NodeDe {
                node: &key_node,
                cx,
                root: false,
                key: true,
            });
            let result = cx.settle(&key_node, result);
            cx.path.borrow_mut().pop();
            return result.map(Some);
        }
        // Stand-ins for the fields found missing in earlier attempts.
        while let Some(&field) = self.injected.get(self.inject_at) {
            if self.members.iter().any(|member| member.key == field) {
                self.inject_at += 1;
                continue;
            }
            return seed
                .deserialize(IntoDeserializer::<DeError>::into_deserializer(field))
                .map(Some);
        }
        Ok(None)
    }

    fn next_value_seed<V: DeserializeSeed<'de>>(&mut self, seed: V) -> Result<V::Value, DeError> {
        if let Some(member) = self.members.get(self.at) {
            self.at += 1;
            return self
                .cx
                .child(&member.value, Seg::Key(member.key.clone()), |de| {
                    seed.deserialize(de)
                });
        }
        let field = self.injected.get(self.inject_at).copied().unwrap_or("");
        self.inject_at += 1;
        self.cx.report_missing(self.node, field);
        self.cx.stand_in(|stand_in| seed.deserialize(stand_in))
    }
}

struct SeqDe<'c> {
    items: &'c [Node],
    at: usize,
    cx: &'c DeCx<'c>,
}

impl<'de> SeqAccess<'de> for SeqDe<'_> {
    type Error = DeError;

    fn next_element_seed<T: DeserializeSeed<'de>>(
        &mut self,
        seed: T,
    ) -> Result<Option<T::Value>, DeError> {
        let Some(item) = self.items.get(self.at) else {
            return Ok(None);
        };
        // An entry that carries an id is named by it, so that its path
        // stays the same when another entry is put before it.
        let seg = match item.get("id").and_then(Node::as_str) {
            Some(id) if item.as_object().is_some() => Seg::Key(id.to_owned()),
            _ => Seg::Index(self.at),
        };
        self.at += 1;
        self.cx
            .child(item, seg, |de| seed.deserialize(de))
            .map(Some)
    }

    fn size_hint(&self) -> Option<usize> {
        Some(self.items.len() - self.at)
    }
}

/// Feeds a [`Located`](crate::Located): the span, then the value itself.
struct LocatedDe<'c> {
    node: &'c Node,
    cx: &'c DeCx<'c>,
    root: bool,
    at: usize,
}

impl<'de> MapAccess<'de> for LocatedDe<'_> {
    type Error = DeError;

    fn next_key_seed<K: DeserializeSeed<'de>>(
        &mut self,
        seed: K,
    ) -> Result<Option<K::Value>, DeError> {
        match LOCATED_FIELDS.get(self.at) {
            Some(&field) => seed
                .deserialize(IntoDeserializer::<DeError>::into_deserializer(field))
                .map(Some),
            None => Ok(None),
        }
    }

    fn next_value_seed<V: DeserializeSeed<'de>>(&mut self, seed: V) -> Result<V::Value, DeError> {
        let at = self.at;
        self.at += 1;
        let span = self.node.span;
        let number = |value: u32| IntoDeserializer::<DeError>::into_deserializer(value);
        match at {
            0 => seed.deserialize(number(span.file.0)),
            1 => seed.deserialize(number(span.start)),
            2 => seed.deserialize(number(span.end)),
            _ => seed.deserialize(NodeDe {
                node: self.node,
                cx: self.cx,
                root: self.root,
                key: false,
            }),
        }
    }
}

/// The name of a JSON kind as [`Kind`](crate::Kind) reads it.
fn kind_word(node: &Node) -> &'static str {
    match node.kind {
        NodeKind::Null => "null",
        NodeKind::Bool(_) => "bool",
        NodeKind::Number(_) | NodeKind::Quantity { .. } => "number",
        NodeKind::String(_) => "string",
        NodeKind::Array(_) => "list",
        NodeKind::Object(_) => "object",
    }
}

/// Feeds [`by_kind`](crate::by_kind): what kind of value is here, then the
/// value itself, to be read as whatever type that kind calls for.
struct KindDe<'c> {
    node: &'c Node,
    cx: &'c DeCx<'c>,
    root: bool,
    at: usize,
}

impl<'de> MapAccess<'de> for KindDe<'_> {
    type Error = DeError;

    fn next_key_seed<K: DeserializeSeed<'de>>(
        &mut self,
        seed: K,
    ) -> Result<Option<K::Value>, DeError> {
        match KIND_FIELDS.get(self.at) {
            Some(&field) => seed
                .deserialize(IntoDeserializer::<DeError>::into_deserializer(field))
                .map(Some),
            None => Ok(None),
        }
    }

    fn next_value_seed<V: DeserializeSeed<'de>>(&mut self, seed: V) -> Result<V::Value, DeError> {
        let at = self.at;
        self.at += 1;
        if at == 0 {
            seed.deserialize(IntoDeserializer::<DeError>::into_deserializer(kind_word(
                self.node,
            )))
        } else {
            seed.deserialize(NodeDe {
                node: self.node,
                cx: self.cx,
                root: self.root,
                key: false,
            })
        }
    }
}

/// Feeds [`by_site`](crate::by_site): the span, the JSON path, the kind
/// of value and then the value itself.
struct SiteDe<'c> {
    node: &'c Node,
    cx: &'c DeCx<'c>,
    root: bool,
    at: usize,
}

impl<'de> MapAccess<'de> for SiteDe<'_> {
    type Error = DeError;

    fn next_key_seed<K: DeserializeSeed<'de>>(
        &mut self,
        seed: K,
    ) -> Result<Option<K::Value>, DeError> {
        match SITE_FIELDS.get(self.at) {
            Some(&field) => seed
                .deserialize(IntoDeserializer::<DeError>::into_deserializer(field))
                .map(Some),
            None => Ok(None),
        }
    }

    fn next_value_seed<V: DeserializeSeed<'de>>(&mut self, seed: V) -> Result<V::Value, DeError> {
        let at = self.at;
        self.at += 1;
        let span = self.node.span;
        let number = |value: u32| IntoDeserializer::<DeError>::into_deserializer(value);
        match at {
            0 => seed.deserialize(number(span.file.0)),
            1 => seed.deserialize(number(span.start)),
            2 => seed.deserialize(number(span.end)),
            3 => seed.deserialize(IntoDeserializer::<DeError>::into_deserializer(
                self.cx.path_string(),
            )),
            4 => seed.deserialize(IntoDeserializer::<DeError>::into_deserializer(kind_word(
                self.node,
            ))),
            _ => seed.deserialize(NodeDe {
                node: self.node,
                cx: self.cx,
                root: self.root,
                key: false,
            }),
        }
    }
}

/// Feeds [`by_site`](crate::by_site) a stand-in: `null`, written nowhere.
struct SiteStandIn {
    at: usize,
}

impl<'de> MapAccess<'de> for SiteStandIn {
    type Error = DeError;

    fn next_key_seed<K: DeserializeSeed<'de>>(
        &mut self,
        seed: K,
    ) -> Result<Option<K::Value>, DeError> {
        match SITE_FIELDS.get(self.at) {
            Some(&field) => seed
                .deserialize(IntoDeserializer::<DeError>::into_deserializer(field))
                .map(Some),
            None => Ok(None),
        }
    }

    fn next_value_seed<V: DeserializeSeed<'de>>(&mut self, seed: V) -> Result<V::Value, DeError> {
        let at = self.at;
        self.at += 1;
        match at {
            0..=2 => seed.deserialize(IntoDeserializer::<DeError>::into_deserializer(0_u32)),
            3 => seed.deserialize(IntoDeserializer::<DeError>::into_deserializer("")),
            4 => seed.deserialize(IntoDeserializer::<DeError>::into_deserializer("null")),
            _ => seed.deserialize(StandIn),
        }
    }
}

/// Feeds [`by_kind`](crate::by_kind) a stand-in: `null`.
struct KindStandIn {
    at: usize,
}

impl<'de> MapAccess<'de> for KindStandIn {
    type Error = DeError;

    fn next_key_seed<K: DeserializeSeed<'de>>(
        &mut self,
        seed: K,
    ) -> Result<Option<K::Value>, DeError> {
        match KIND_FIELDS.get(self.at) {
            Some(&field) => seed
                .deserialize(IntoDeserializer::<DeError>::into_deserializer(field))
                .map(Some),
            None => Ok(None),
        }
    }

    fn next_value_seed<V: DeserializeSeed<'de>>(&mut self, seed: V) -> Result<V::Value, DeError> {
        let at = self.at;
        self.at += 1;
        if at == 0 {
            seed.deserialize(IntoDeserializer::<DeError>::into_deserializer("null"))
        } else {
            seed.deserialize(StandIn)
        }
    }
}

struct EnumDe<'c> {
    variant: &'c str,
    value: Option<&'c Node>,
    cx: &'c DeCx<'c>,
}

impl<'de, 'c> EnumAccess<'de> for EnumDe<'c> {
    type Error = DeError;
    type Variant = VariantDe<'c>;

    fn variant_seed<V: DeserializeSeed<'de>>(
        self,
        seed: V,
    ) -> Result<(V::Value, VariantDe<'c>), DeError> {
        let name =
            seed.deserialize(IntoDeserializer::<DeError>::into_deserializer(self.variant))?;
        Ok((
            name,
            VariantDe {
                variant: self.variant,
                value: self.value,
                cx: self.cx,
            },
        ))
    }
}

struct VariantDe<'c> {
    variant: &'c str,
    value: Option<&'c Node>,
    cx: &'c DeCx<'c>,
}

impl<'c> VariantDe<'c> {
    fn needs_value(&self) -> DeError {
        DeError::Mismatch(format!(
            "\"{0}\" takes a value: write it as an object, {{ \"{0}\": ... }}",
            self.variant
        ))
    }
}

impl<'de> VariantAccess<'de> for VariantDe<'_> {
    type Error = DeError;

    fn unit_variant(self) -> Result<(), DeError> {
        match self.value {
            None => Ok(()),
            Some(node) if matches!(node.kind, NodeKind::Null) => Ok(()),
            Some(_) => Err(DeError::Mismatch(format!(
                "\"{0}\" takes no value: write it as the string \"{0}\"",
                self.variant
            ))),
        }
    }

    fn newtype_variant_seed<T: DeserializeSeed<'de>>(self, seed: T) -> Result<T::Value, DeError> {
        match self.value {
            Some(node) => self
                .cx
                .child(node, Seg::Key(self.variant.to_owned()), |de| {
                    seed.deserialize(de)
                }),
            None => Err(self.needs_value()),
        }
    }

    fn tuple_variant<V: Visitor<'de>>(self, _len: usize, visitor: V) -> Result<V::Value, DeError> {
        match self.value {
            Some(node) => self
                .cx
                .child(node, Seg::Key(self.variant.to_owned()), |de| {
                    de.deserialize_seq(visitor)
                }),
            None => Err(self.needs_value()),
        }
    }

    fn struct_variant<V: Visitor<'de>>(
        self,
        fields: &'static [&'static str],
        visitor: V,
    ) -> Result<V::Value, DeError> {
        match self.value {
            Some(node) => self
                .cx
                .child(node, Seg::Key(self.variant.to_owned()), |de| {
                    de.deserialize_struct("", fields, visitor)
                }),
            None => Err(self.needs_value()),
        }
    }
}

/// Feeds a harmless value of whatever kind is asked for, so that reading
/// can go on past a mistake that has been reported. The definition being
/// read is already marked broken; what is fed is never kept.
#[derive(Clone, Copy)]
struct StandIn;

struct NoEntries;

impl<'de> MapAccess<'de> for NoEntries {
    type Error = DeError;

    fn next_key_seed<K: DeserializeSeed<'de>>(
        &mut self,
        _seed: K,
    ) -> Result<Option<K::Value>, DeError> {
        Ok(None)
    }

    fn next_value_seed<V: DeserializeSeed<'de>>(&mut self, _seed: V) -> Result<V::Value, DeError> {
        Err(DeError::Reported)
    }
}

impl<'de> SeqAccess<'de> for NoEntries {
    type Error = DeError;

    fn next_element_seed<T: DeserializeSeed<'de>>(
        &mut self,
        _seed: T,
    ) -> Result<Option<T::Value>, DeError> {
        Ok(None)
    }
}

/// Feeds a [`Located`](crate::Located) stand-in.
struct LocatedStandIn {
    at: usize,
}

impl<'de> MapAccess<'de> for LocatedStandIn {
    type Error = DeError;

    fn next_key_seed<K: DeserializeSeed<'de>>(
        &mut self,
        seed: K,
    ) -> Result<Option<K::Value>, DeError> {
        match LOCATED_FIELDS.get(self.at) {
            Some(&field) => seed
                .deserialize(IntoDeserializer::<DeError>::into_deserializer(field))
                .map(Some),
            None => Ok(None),
        }
    }

    fn next_value_seed<V: DeserializeSeed<'de>>(&mut self, seed: V) -> Result<V::Value, DeError> {
        let at = self.at;
        self.at += 1;
        if at < 3 {
            seed.deserialize(IntoDeserializer::<DeError>::into_deserializer(0_u32))
        } else {
            seed.deserialize(StandIn)
        }
    }
}

macro_rules! stand_in_number {
    ($($method:ident, $visit:ident, $zero:expr;)*) => {
        $(
            fn $method<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, DeError> {
                visitor.$visit($zero)
            }
        )*
    };
}

impl<'de> Deserializer<'de> for StandIn {
    type Error = DeError;

    fn deserialize_any<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, DeError> {
        visitor.visit_unit()
    }

    fn deserialize_bool<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, DeError> {
        visitor.visit_bool(false)
    }

    stand_in_number! {
        deserialize_i8, visit_i8, 0;
        deserialize_i16, visit_i16, 0;
        deserialize_i32, visit_i32, 0;
        deserialize_i64, visit_i64, 0;
        deserialize_u8, visit_u8, 0;
        deserialize_u16, visit_u16, 0;
        deserialize_u32, visit_u32, 0;
        deserialize_u64, visit_u64, 0;
        deserialize_f32, visit_f32, 0.0;
        deserialize_f64, visit_f64, 0.0;
    }

    fn deserialize_char<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, DeError> {
        visitor.visit_char(' ')
    }

    fn deserialize_str<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, DeError> {
        visitor.visit_str("")
    }

    fn deserialize_string<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, DeError> {
        visitor.visit_str("")
    }

    fn deserialize_bytes<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, DeError> {
        visitor.visit_bytes(&[])
    }

    fn deserialize_byte_buf<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, DeError> {
        visitor.visit_bytes(&[])
    }

    fn deserialize_option<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, DeError> {
        visitor.visit_none()
    }

    fn deserialize_unit<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, DeError> {
        visitor.visit_unit()
    }

    fn deserialize_unit_struct<V: Visitor<'de>>(
        self,
        _name: &'static str,
        visitor: V,
    ) -> Result<V::Value, DeError> {
        visitor.visit_unit()
    }

    fn deserialize_newtype_struct<V: Visitor<'de>>(
        self,
        name: &'static str,
        visitor: V,
    ) -> Result<V::Value, DeError> {
        if name == TAG_MAGIC {
            return visitor.visit_u32(0);
        }
        if name == MICROS_MAGIC {
            return visitor.visit_i64(0);
        }
        if QUANTITY_MAGICS.iter().any(|(magic, _)| *magic == name) {
            return visitor.visit_f64(0.0);
        }
        if name == SITE_OR_PLAIN_MAGIC {
            return visitor.visit_map(SiteStandIn { at: 0 });
        }
        visitor.visit_newtype_struct(self)
    }

    fn deserialize_seq<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, DeError> {
        visitor.visit_seq(NoEntries)
    }

    fn deserialize_tuple<V: Visitor<'de>>(
        self,
        _len: usize,
        visitor: V,
    ) -> Result<V::Value, DeError> {
        visitor.visit_seq(NoEntries)
    }

    fn deserialize_tuple_struct<V: Visitor<'de>>(
        self,
        _name: &'static str,
        _len: usize,
        visitor: V,
    ) -> Result<V::Value, DeError> {
        visitor.visit_seq(NoEntries)
    }

    fn deserialize_map<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, DeError> {
        visitor.visit_map(NoEntries)
    }

    fn deserialize_struct<V: Visitor<'de>>(
        self,
        name: &'static str,
        _fields: &'static [&'static str],
        visitor: V,
    ) -> Result<V::Value, DeError> {
        match name {
            REF_MAGIC | INLINE_MAGIC => visitor.visit_u32(0),
            GATED_MAGIC => visitor.visit_newtype_struct(self),
            LOCATED_MAGIC => visitor.visit_map(LocatedStandIn { at: 0 }),
            KIND_MAGIC => visitor.visit_map(KindStandIn { at: 0 }),
            SITE_MAGIC => visitor.visit_map(SiteStandIn { at: 0 }),
            _ => visitor.visit_map(NoEntries),
        }
    }

    fn deserialize_enum<V: Visitor<'de>>(
        self,
        _name: &'static str,
        _variants: &'static [&'static str],
        _visitor: V,
    ) -> Result<V::Value, DeError> {
        // There is no harmless value of an unknown enum.
        Err(DeError::Reported)
    }

    fn deserialize_identifier<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, DeError> {
        visitor.visit_str("")
    }

    fn deserialize_ignored_any<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, DeError> {
        visitor.visit_unit()
    }
}
