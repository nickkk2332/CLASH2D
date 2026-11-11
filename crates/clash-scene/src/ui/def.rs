//! The definition types of the interface toolkit: `screen`, its layout
//! nodes and every widget's fields (12 §10.1 to §10.3), `figure`,
//! `ui_part` and `preference`.
//!
//! These are what a mod writes, read with `serde`. Field names are the
//! JSON keys. Nothing here is checked against a theme or laid out; that
//! happens when a [`Screen`](super::Screen) is made from a definition.
//!
//! A property that may be computed is a [`Prop`]. Read by the mod
//! loader, a bound one knows where it was written, and its screen
//! compiles it with the loader's other expression fields
//! ([`ScreenDef::compile_bindings`]). Read by anything else, with
//! `serde_json` in a test, it is held as written and never compiled.

use core::fmt;
use std::collections::BTreeMap;

use clash_core::{Real, Value};
use clash_data::{CompileCx, FromSite, Kind, KindValue, Located, Site, by_site_or_plain};
use clash_expr::Type;
use serde::Deserialize;
use serde::de::value::{MapAccessDeserializer, SeqAccessDeserializer};
use serde::de::{self, Deserializer, IgnoredAny, MapAccess, SeqAccess, Visitor};

use crate::input::KeyChord;

use super::bind::{
    BindCx, Binding, ExprObject, ItType, ParamType, TYPE_PARTY_MEMBER, element_type,
};
pub use super::effect::UiEffects;
use super::screen::CONTEXT_DEBUG;

/// A widget property that is either written outright or computed.
///
/// A `Bound` property holds its [`Binding`]: the source of an expression
/// or a text template and, once the screen's definition has been
/// compiled, the compiled form. A screen shows a placeholder for one
/// that was never compiled or never evaluated.
#[derive(Clone, Debug, PartialEq)]
pub enum Prop<T> {
    /// A value written in the definition.
    Static(T),
    /// An expression or a text template.
    Bound(Binding),
}

impl<T: Default> Default for Prop<T> {
    fn default() -> Prop<T> {
        Prop::Static(T::default())
    }
}

impl<T> Prop<T> {
    /// The value, if it was written outright.
    pub fn as_static(&self) -> Option<&T> {
        match self {
            Prop::Static(value) => Some(value),
            Prop::Bound(_) => None,
        }
    }
}

/// Player-facing text (04 §7.1). A text with `{expression}` templates
/// (05 §4.6) is [`Prop::Bound`] and holds the whole source; one without
/// is [`Prop::Static`], with `{{` and `}}` already turned into single
/// braces. Either may hold inline style tags (12 §4.3).
pub type TextProp = Prop<String>;

/// Reads a text field's string: bound if it has a template in braces.
pub fn text_prop(source: &str) -> TextProp {
    text_prop_at(source, None)
}

/// Reads a text field's string written at `site`.
fn text_prop_at(source: &str, site: Option<Site>) -> TextProp {
    let mut chars = source.chars().peekable();
    let mut plain = String::with_capacity(source.len());
    while let Some(ch) = chars.next() {
        match ch {
            '{' | '}' if chars.peek() == Some(&ch) => {
                chars.next();
                plain.push(ch);
            }
            '{' => {
                return Prop::Bound(match site {
                    Some(site) => Binding::sited(source, site),
                    None => Binding::unsited(source),
                });
            }
            other => plain.push(other),
        }
    }
    Prop::Static(plain)
}

impl Prop<String> {
    /// Compiles the text's template, if it has one.
    pub(crate) fn compile(&mut self, bx: &mut BindCx<'_, '_>) {
        if let Prop::Bound(binding) = self {
            binding.compile_text(bx);
        }
    }
}

impl Prop<f64> {
    /// Compiles the number's expression, if it is one.
    pub(crate) fn compile(&mut self, bx: &mut BindCx<'_, '_>) {
        if let Prop::Bound(binding) = self {
            binding.compile_expr(bx, &Type::Real.nullable());
        }
    }
}

impl Prop<bool> {
    /// Compiles the condition, if it is an expression.
    pub(crate) fn compile(&mut self, bx: &mut BindCx<'_, '_>) {
        if let Prop::Bound(binding) = self {
            binding.compile_expr(bx, &Type::Bool.nullable());
        }
    }
}

/// Implements `Deserialize` for a property: with its place under the
/// mod loader, through `FromSite`, and with a plain visitor otherwise.
macro_rules! dual_mode {
    ($ty:ty, $plain:ident) => {
        impl<'de> Deserialize<'de> for $ty {
            fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
                by_site_or_plain(deserializer, $plain)
            }
        }
    };
}

/// A bound property written as `{ "expr": "…" }`, outside the loader.
fn plain_expr<'de, T, A: MapAccess<'de>>(map: A) -> Result<Prop<T>, A::Error> {
    ExprObject::deserialize(MapAccessDeserializer::new(map)).map(|e| Prop::Bound(e.expr))
}

/// A bound property written as a string or `{ "expr": "…" }`, under
/// the loader.
fn sited_expr<'de, T, A: MapAccess<'de>>(
    site: Site,
    kind: Kind,
    value: KindValue<'_, A>,
) -> Result<Prop<T>, A::Error> {
    match kind {
        Kind::Object => value.read::<ExprObject>().map(|e| Prop::Bound(e.expr)),
        _ => Ok(Prop::Bound(Binding::sited(value.read::<String>()?, site))),
    }
}

struct TextVisitor;

impl Visitor<'_> for TextVisitor {
    type Value = Prop<String>;
    fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("a text")
    }
    fn visit_str<E: de::Error>(self, text: &str) -> Result<Prop<String>, E> {
        Ok(text_prop(text))
    }
}

impl<'de> FromSite<'de> for Prop<String> {
    fn from_site<A: MapAccess<'de>>(
        site: Site,
        _kind: Kind,
        value: KindValue<'_, A>,
    ) -> Result<Self, A::Error> {
        // Whatever the kind, it is read as a string, so that a value of
        // another kind is reported as any wrong value is.
        let text = value.read::<String>()?;
        Ok(text_prop_at(&text, Some(site)))
    }
}

dual_mode!(Prop<String>, TextVisitor);

struct FlagVisitor;

impl<'de> Visitor<'de> for FlagVisitor {
    type Value = Prop<bool>;
    fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("true, false, or a condition as a string or `{ \"expr\": … }`")
    }
    fn visit_bool<E: de::Error>(self, value: bool) -> Result<Prop<bool>, E> {
        Ok(Prop::Static(value))
    }
    fn visit_str<E: de::Error>(self, source: &str) -> Result<Prop<bool>, E> {
        Ok(Prop::Bound(Binding::unsited(source)))
    }
    fn visit_map<A: MapAccess<'de>>(self, map: A) -> Result<Prop<bool>, A::Error> {
        plain_expr(map)
    }
}

impl<'de> FromSite<'de> for Prop<bool> {
    fn from_site<A: MapAccess<'de>>(
        site: Site,
        kind: Kind,
        value: KindValue<'_, A>,
    ) -> Result<Self, A::Error> {
        match kind {
            Kind::Bool => value.read::<bool>().map(Prop::Static),
            Kind::Null => value.read::<IgnoredAny>().map(|_| Prop::Static(true)),
            Kind::String | Kind::Object => sited_expr(site, kind, value),
            other => Err(other.unexpected(
                "true, false, or a condition as a string or `{ \"expr\": … }`",
            )),
        }
    }
}

dual_mode!(Prop<bool>, FlagVisitor);

struct NumberVisitor;

impl<'de> Visitor<'de> for NumberVisitor {
    type Value = Prop<f64>;
    fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("a number, or an expression as a string or `{ \"expr\": … }`")
    }
    fn visit_i64<E: de::Error>(self, value: i64) -> Result<Prop<f64>, E> {
        Ok(Prop::Static(value as f64))
    }
    fn visit_u64<E: de::Error>(self, value: u64) -> Result<Prop<f64>, E> {
        Ok(Prop::Static(value as f64))
    }
    fn visit_f64<E: de::Error>(self, value: f64) -> Result<Prop<f64>, E> {
        Ok(Prop::Static(value))
    }
    fn visit_str<E: de::Error>(self, source: &str) -> Result<Prop<f64>, E> {
        Ok(Prop::Bound(Binding::unsited(source)))
    }
    fn visit_map<A: MapAccess<'de>>(self, map: A) -> Result<Prop<f64>, A::Error> {
        plain_expr(map)
    }
}

impl<'de> FromSite<'de> for Prop<f64> {
    fn from_site<A: MapAccess<'de>>(
        site: Site,
        kind: Kind,
        value: KindValue<'_, A>,
    ) -> Result<Self, A::Error> {
        match kind {
            Kind::Number => value.read::<f64>().map(Prop::Static),
            Kind::Null => value.read::<IgnoredAny>().map(|_| Prop::Static(0.0)),
            Kind::String | Kind::Object => sited_expr(site, kind, value),
            other => Err(other
                .unexpected("a number, or an expression as a string or `{ \"expr\": … }`")),
        }
    }
}

dual_mode!(Prop<f64>, NumberVisitor);

struct NamesVisitor;

impl<'de> Visitor<'de> for NamesVisitor {
    type Value = Prop<Vec<String>>;
    fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("a list of names, or an expression as a string")
    }
    fn visit_str<E: de::Error>(self, source: &str) -> Result<Prop<Vec<String>>, E> {
        Ok(Prop::Bound(Binding::unsited(source)))
    }
    fn visit_seq<A: SeqAccess<'de>>(self, seq: A) -> Result<Prop<Vec<String>>, A::Error> {
        Vec::deserialize(SeqAccessDeserializer::new(seq)).map(Prop::Static)
    }
}

impl<'de> FromSite<'de> for Prop<Vec<String>> {
    fn from_site<A: MapAccess<'de>>(
        site: Site,
        kind: Kind,
        value: KindValue<'_, A>,
    ) -> Result<Self, A::Error> {
        match kind {
            Kind::List => value.read::<Vec<String>>().map(Prop::Static),
            Kind::Null => value.read::<IgnoredAny>().map(|_| Prop::Static(Vec::new())),
            Kind::String | Kind::Object => sited_expr(site, kind, value),
            other => Err(other.unexpected("a list of names, or an expression as a string")),
        }
    }
}

dual_mode!(Prop<Vec<String>>, NamesVisitor);

struct ShadeVisitor;

impl<'de> Visitor<'de> for ShadeVisitor {
    type Value = Prop<BTreeMap<String, String>>;
    fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("an object of names, or an expression as a string")
    }
    fn visit_str<E: de::Error>(self, source: &str) -> Result<Self::Value, E> {
        Ok(Prop::Bound(Binding::unsited(source)))
    }
    fn visit_map<A: MapAccess<'de>>(self, map: A) -> Result<Self::Value, A::Error> {
        BTreeMap::deserialize(MapAccessDeserializer::new(map)).map(Prop::Static)
    }
}

impl<'de> FromSite<'de> for Prop<BTreeMap<String, String>> {
    fn from_site<A: MapAccess<'de>>(
        site: Site,
        kind: Kind,
        value: KindValue<'_, A>,
    ) -> Result<Self, A::Error> {
        match kind {
            Kind::Object => value
                .read::<BTreeMap<String, String>>()
                .map(Prop::Static),
            Kind::Null => value
                .read::<IgnoredAny>()
                .map(|_| Prop::Static(BTreeMap::new())),
            Kind::String => sited_expr(site, kind, value),
            other => Err(other.unexpected("an object of names, or an expression as a string")),
        }
    }
}

dual_mode!(Prop<BTreeMap<String, String>>, ShadeVisitor);

struct ValueVisitor;

impl<'de> Visitor<'de> for ValueVisitor {
    type Value = Prop<Literal>;
    fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("true, false, a number, or an expression as a string")
    }
    fn visit_bool<E: de::Error>(self, value: bool) -> Result<Prop<Literal>, E> {
        Ok(Prop::Static(Literal(Value::Bool(value))))
    }
    fn visit_i64<E: de::Error>(self, value: i64) -> Result<Prop<Literal>, E> {
        Ok(Prop::Static(Literal(Value::Int(value))))
    }
    fn visit_u64<E: de::Error>(self, value: u64) -> Result<Prop<Literal>, E> {
        i64::try_from(value)
            .map(|v| Prop::Static(Literal(Value::Int(v))))
            .map_err(|_| E::custom(format!("{value} is too large for an integer")))
    }
    fn visit_f64<E: de::Error>(self, value: f64) -> Result<Prop<Literal>, E> {
        Real::new(value)
            .map(|v| Prop::Static(Literal(Value::Real(v))))
            .map_err(E::custom)
    }
    fn visit_str<E: de::Error>(self, source: &str) -> Result<Prop<Literal>, E> {
        Ok(Prop::Bound(Binding::unsited(source)))
    }
    fn visit_map<A: MapAccess<'de>>(self, map: A) -> Result<Prop<Literal>, A::Error> {
        plain_expr(map)
    }
}

impl<'de> FromSite<'de> for Prop<Literal> {
    fn from_site<A: MapAccess<'de>>(
        site: Site,
        kind: Kind,
        value: KindValue<'_, A>,
    ) -> Result<Self, A::Error> {
        match kind {
            Kind::Bool | Kind::Number => value.read::<Literal>().map(Prop::Static),
            Kind::String | Kind::Object => sited_expr(site, kind, value),
            other => {
                Err(other.unexpected("true, false, a number, or an expression as a string"))
            }
        }
    }
}

dual_mode!(Prop<Literal>, ValueVisitor);

/// A literal value in a definition: `true`, `3`, `0.5` or `"text"`.
#[derive(Clone, Debug, PartialEq)]
pub struct Literal(pub Value);

impl<'de> Deserialize<'de> for Literal {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct LiteralVisitor;
        impl Visitor<'_> for LiteralVisitor {
            type Value = Literal;
            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str("true, false, a number or a text")
            }
            fn visit_bool<E: de::Error>(self, value: bool) -> Result<Literal, E> {
                Ok(Literal(Value::Bool(value)))
            }
            fn visit_i64<E: de::Error>(self, value: i64) -> Result<Literal, E> {
                Ok(Literal(Value::Int(value)))
            }
            fn visit_u64<E: de::Error>(self, value: u64) -> Result<Literal, E> {
                i64::try_from(value)
                    .map(|v| Literal(Value::Int(v)))
                    .map_err(|_| E::custom(format!("{value} is too large for an integer")))
            }
            fn visit_f64<E: de::Error>(self, value: f64) -> Result<Literal, E> {
                Real::new(value)
                    .map(|v| Literal(Value::Real(v)))
                    .map_err(E::custom)
            }
            fn visit_str<E: de::Error>(self, value: &str) -> Result<Literal, E> {
                Ok(Literal(Value::Str(value.to_owned())))
            }
        }
        deserializer.deserialize_any(LiteralVisitor)
    }
}

/// How large a node is along one axis.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Size {
    /// So many cells: `34`.
    Cells(u16),
    /// A share of the space its container has: `"50%"`.
    Percent(u16),
    /// As large as its content: `"fit"`.
    Fit,
    /// A share of whatever space is left over, by weight: `"fill"` or
    /// `{ "fill": 2 }`.
    Fill(u16),
}

impl<'de> Deserialize<'de> for Size {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct FillObject {
            fill: u16,
        }
        struct SizeVisitor;
        impl<'de> Visitor<'de> for SizeVisitor {
            type Value = Size;
            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str("a size: a number of cells, `\"50%\"`, `\"fit\"`, `\"fill\"` or `{ \"fill\": weight }`")
            }
            fn visit_u64<E: de::Error>(self, value: u64) -> Result<Size, E> {
                u16::try_from(value)
                    .map(Size::Cells)
                    .map_err(|_| E::custom(format!("{value} cells is too large")))
            }
            fn visit_i64<E: de::Error>(self, value: i64) -> Result<Size, E> {
                u16::try_from(value)
                    .map(Size::Cells)
                    .map_err(|_| E::custom(format!("a size cannot be {value} cells")))
            }
            fn visit_str<E: de::Error>(self, text: &str) -> Result<Size, E> {
                match text {
                    "fit" => Ok(Size::Fit),
                    "fill" => Ok(Size::Fill(1)),
                    other => other
                        .strip_suffix('%')
                        .and_then(|n| n.trim().parse::<u16>().ok())
                        .filter(|n| *n <= 100)
                        .map(Size::Percent)
                        .ok_or_else(|| E::invalid_value(de::Unexpected::Str(other), &self)),
                }
            }
            fn visit_map<A: MapAccess<'de>>(self, map: A) -> Result<Size, A::Error> {
                FillObject::deserialize(MapAccessDeserializer::new(map)).map(|f| Size::Fill(f.fill))
            }
        }
        deserializer.deserialize_any(SizeVisitor)
    }
}

/// Where content sits in a space wider than it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Align {
    /// Against the left edge.
    #[default]
    Left,
    /// In the middle.
    #[serde(alias = "center")]
    Centre,
    /// Against the right edge.
    Right,
}

/// Implements `Deserialize` for a type that may be written as a bare
/// string or as an object. The type derives its object form with
/// `#[serde(remote = "Self")]`.
macro_rules! string_or_object {
    ($ty:ident, $expecting:literal, $text:ident => $from:expr $(, seq $seq:ident => $from_seq:expr)?) => {
        impl<'de> Deserialize<'de> for $ty {
            fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<$ty, D::Error> {
                struct ShorthandVisitor;
                impl<'de> Visitor<'de> for ShorthandVisitor {
                    type Value = $ty;
                    fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                        f.write_str($expecting)
                    }
                    fn visit_str<E: de::Error>(self, $text: &str) -> Result<$ty, E> {
                        Ok($from)
                    }
                    $(
                    fn visit_seq<A: SeqAccess<'de>>(self, seq: A) -> Result<$ty, A::Error> {
                        let $seq = Deserialize::deserialize(SeqAccessDeserializer::new(seq))?;
                        Ok($from_seq)
                    }
                    )?
                    fn visit_map<A: MapAccess<'de>>(self, map: A) -> Result<$ty, A::Error> {
                        $ty::deserialize(MapAccessDeserializer::new(map))
                    }
                }
                deserializer.deserialize_any(ShorthandVisitor)
            }
        }
    };
}

/// One row of a `list`, `table`, `tree`, `menu` or `inspect` whose rows
/// are written in the definition.
///
/// A row may be written as a bare string, which is its `label`, or as a
/// list of strings, which are its `cells`.
#[derive(Clone, Debug, PartialEq, Default, Deserialize)]
#[serde(remote = "Self", deny_unknown_fields)]
pub struct RowDef {
    /// What the row shows in a `list`, `tree`, `menu` or `inspect`.
    #[serde(default)]
    pub label: TextProp,
    /// In an `inspect`, the value shown beside the label.
    #[serde(default)]
    pub value: Option<TextProp>,
    /// In a `menu`, the key that activates the row from anywhere on the
    /// screen.
    #[serde(default)]
    pub key: Option<KeyChord>,
    /// In a `table`, the row's cells, one per column.
    #[serde(default)]
    pub cells: Vec<TextProp>,
    /// In a `tree` or `inspect`, the rows beneath this one.
    #[serde(default)]
    pub children: Vec<RowDef>,
    /// In a `tree` or `inspect`, whether the row starts with its children
    /// shown.
    #[serde(default)]
    pub open: bool,
    /// A tooltip for the row.
    #[serde(default)]
    pub tip: Option<TextProp>,
    /// Effects run when this row is activated, before the widget's own
    /// `on_activate`.
    #[serde(default)]
    pub on_activate: UiEffects,
}

string_or_object!(RowDef, "a row: a text, a list of cell texts, or an object",
    text => RowDef { label: text_prop(text), ..RowDef::default() },
    seq cells => RowDef { cells, ..RowDef::default() });

/// The rows of a `list`, `table`, `tree`, `menu` or `inspect`: written
/// out, or a query.
#[derive(Clone, Debug, PartialEq)]
pub enum RowsDef {
    /// Rows written in the definition, as a JSON list.
    Static(Vec<RowDef>),
    /// An expression giving a list, or a query. A query written as an
    /// object is kept as compact JSON until queries are built (05 §11),
    /// and gives no rows.
    Bound(Binding),
}

impl Default for RowsDef {
    fn default() -> RowsDef {
        RowsDef::Static(Vec::new())
    }
}

struct RowsVisitor;

impl<'de> Visitor<'de> for RowsVisitor {
    type Value = RowsDef;
    fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("a list of rows, or a query as a string or an object")
    }
    fn visit_str<E: de::Error>(self, source: &str) -> Result<RowsDef, E> {
        Ok(RowsDef::Bound(Binding::unsited(source)))
    }
    fn visit_seq<A: SeqAccess<'de>>(self, seq: A) -> Result<RowsDef, A::Error> {
        Vec::deserialize(SeqAccessDeserializer::new(seq)).map(RowsDef::Static)
    }
    fn visit_map<A: MapAccess<'de>>(self, map: A) -> Result<RowsDef, A::Error> {
        serde_json::Value::deserialize(MapAccessDeserializer::new(map))
            .map(|query| RowsDef::Bound(Binding::query(query.to_string(), Site::default())))
    }
}

impl<'de> FromSite<'de> for RowsDef {
    fn from_site<A: MapAccess<'de>>(
        site: Site,
        kind: Kind,
        value: KindValue<'_, A>,
    ) -> Result<Self, A::Error> {
        match kind {
            Kind::List => value.read::<Vec<RowDef>>().map(RowsDef::Static),
            Kind::Null => value.read::<IgnoredAny>().map(|_| RowsDef::default()),
            Kind::String => Ok(RowsDef::Bound(Binding::sited(
                value.read::<String>()?,
                site,
            ))),
            Kind::Object => value
                .read::<serde_json::Value>()
                .map(|query| RowsDef::Bound(Binding::query(query.to_string(), site))),
            other => {
                Err(other.unexpected("a list of rows, or a query as a string or an object"))
            }
        }
    }
}

dual_mode!(RowsDef, RowsVisitor);

/// One column of a `table`.
#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ColumnDef {
    /// The header.
    pub title: TextProp,
    /// What each row shows in this column when the rows come from a
    /// query: a template over the row, `it`. Rows written in the
    /// definition give their `cells` instead.
    #[serde(default)]
    pub value: Option<TextProp>,
    /// The column's width. Left out, the column fits its content.
    #[serde(default)]
    pub width: Option<Size>,
    /// Where the cells sit in the column.
    #[serde(default)]
    pub align: Align,
}

/// One option of a `choice`. May be written as a bare string, which is
/// both its value and its label.
#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(remote = "Self", deny_unknown_fields)]
pub struct OptionDef {
    /// The value stored in the bound variable when this option is chosen.
    pub value: Literal,
    /// What the option shows. Left out, the value.
    #[serde(default)]
    pub label: Option<TextProp>,
}

string_or_object!(OptionDef, "an option: a text, or an object with `value` and `label`",
    text => OptionDef { value: Literal(Value::Str(text.to_owned())), label: None });

/// The options of a `choice`: written out, or an expression.
#[derive(Clone, Debug, PartialEq)]
pub enum OptionsDef {
    /// Options written in the definition.
    Static(Vec<OptionDef>),
    /// An expression giving the options: a list of values, each shown as
    /// text, or of maps with `value` and `label`.
    Bound(Binding),
}

struct OptionsVisitor;

impl<'de> Visitor<'de> for OptionsVisitor {
    type Value = OptionsDef;
    fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("a list of options, or an expression as a string")
    }
    fn visit_str<E: de::Error>(self, source: &str) -> Result<OptionsDef, E> {
        Ok(OptionsDef::Bound(Binding::unsited(source)))
    }
    fn visit_seq<A: SeqAccess<'de>>(self, seq: A) -> Result<OptionsDef, A::Error> {
        Vec::deserialize(SeqAccessDeserializer::new(seq)).map(OptionsDef::Static)
    }
}

impl<'de> FromSite<'de> for OptionsDef {
    fn from_site<A: MapAccess<'de>>(
        site: Site,
        kind: Kind,
        value: KindValue<'_, A>,
    ) -> Result<Self, A::Error> {
        match kind {
            Kind::List => value.read::<Vec<OptionDef>>().map(OptionsDef::Static),
            Kind::Null => value
                .read::<IgnoredAny>()
                .map(|_| OptionsDef::Static(Vec::new())),
            Kind::String => Ok(OptionsDef::Bound(Binding::sited(
                value.read::<String>()?,
                site,
            ))),
            other => Err(other.unexpected("a list of options, or an expression as a string")),
        }
    }
}

dual_mode!(OptionsDef, OptionsVisitor);

/// The fields every widget has (12 §10.3), and every layout node may
/// carry.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct CommonDef {
    /// A name for the node, unique within its screen, so that a patch can
    /// change this node alone (04 §6.1) and effects can refer to it.
    pub id: Option<String>,
    /// The theme style, or colour, the node is drawn in.
    pub style: Option<String>,
    /// `if`: the node is shown, and takes space, only while this is true.
    pub show_if: Option<Prop<bool>>,
    /// A tooltip: shown by a `tooltip` widget while this node has the
    /// focus or the pointer.
    pub tip: Option<TextProp>,
}

/// Defines a widget's fields: the four every widget has, then its own.
macro_rules! widget_def {
    (
        $(#[$meta:meta])*
        pub struct $name:ident {
            $( $(#[$fmeta:meta])* pub $field:ident : $ty:ty, )*
        }
    ) => {
        #[derive(Clone, Debug, PartialEq, Deserialize)]
        #[serde(deny_unknown_fields)]
        $(#[$meta])*
        pub struct $name {
            /// A name for the widget, unique within its screen. May be
            /// written on the layout node instead.
            #[serde(default)]
            pub id: Option<String>,
            /// The theme style, or colour, the widget is drawn in. May be
            /// written on the layout node instead.
            #[serde(default)]
            pub style: Option<String>,
            /// `if`: the widget is shown only while this is true. May be
            /// written on the layout node instead.
            #[serde(default, rename = "if")]
            pub show_if: Option<Prop<bool>>,
            /// A tooltip. May be written on the layout node instead.
            #[serde(default)]
            pub tip: Option<TextProp>,
            $( $(#[$fmeta])* pub $field : $ty, )*
        }

        impl $name {
            /// Takes the common fields out of the widget's own object.
            fn take_common(&mut self) -> CommonDef {
                CommonDef {
                    id: self.id.take(),
                    style: self.style.take(),
                    show_if: self.show_if.take(),
                    tip: self.tip.take(),
                }
            }
        }
    };
}

fn yes() -> bool {
    true
}

fn one() -> Prop<f64> {
    Prop::Static(1.0)
}

widget_def! {
    /// `text`: text with templates, wrapping and inline styles.
    ///
    /// May be written as a bare string: `{ "text": "Hello." }`.
    #[serde(remote = "Self")]
    pub struct TextDef {
        /// The text shown. Line breaks in it are kept.
        pub value: TextProp,
        /// Whether lines too long for the widget are wrapped (the
        /// default) or cut off.
        #[serde(default = "yes")]
        pub wrap: bool,
        /// Where each line sits in the widget's width.
        #[serde(default)]
        pub align: Align,
    }
}

string_or_object!(TextDef, "a text, or an object with `value`",
text => TextDef {
    id: None, style: None, show_if: None, tip: None,
    value: text_prop(text), wrap: true, align: Align::Left,
});

widget_def! {
    /// `art`: character art from a mod's asset file, drawn as it is
    /// written, without inline styles.
    pub struct ArtDef {
        /// The asset file, relative to the mod's directory:
        /// `"assets/title.txt"`.
        pub value: TextProp,
    }
}

widget_def! {
    /// `list`: rows from a query or a list, with selection.
    pub struct ListDef {
        /// The rows: a list written out, or a query.
        #[serde(default)]
        pub rows: RowsDef,
        /// How each row from a query is shown: a template over the row,
        /// `it`. Rows written in the definition show their `label`.
        #[serde(default)]
        pub item: Option<TextProp>,
        /// What is shown when there are no rows.
        #[serde(default)]
        pub empty: Option<TextProp>,
        /// The screen variable that holds the selected row's number, from
        /// 0. Left out, the widget keeps the selection itself.
        #[serde(default)]
        pub selected: Option<String>,
        /// Effects run when the selection moves, with the row as `it`.
        #[serde(default)]
        pub on_select: UiEffects,
        /// Effects run when a row is activated, with the row as `it`.
        #[serde(default)]
        pub on_activate: UiEffects,
    }
}

widget_def! {
    /// `table`: rows with columns and headers; sortable.
    pub struct TableDef {
        /// The rows: a list written out, each with `cells`, or a query.
        #[serde(default)]
        pub rows: RowsDef,
        /// The columns, left to right.
        pub columns: Vec<ColumnDef>,
        /// What is shown when there are no rows.
        #[serde(default)]
        pub empty: Option<TextProp>,
        /// The screen variable that holds the selected row's number, from
        /// 0, counted in the order the rows were given, not as sorted.
        #[serde(default)]
        pub selected: Option<String>,
        /// Effects run when the selection moves, with the row as `it`.
        #[serde(default)]
        pub on_select: UiEffects,
        /// Effects run when a row is activated, with the row as `it`.
        #[serde(default)]
        pub on_activate: UiEffects,
    }
}

widget_def! {
    /// `tree`: a collapsible hierarchy.
    pub struct TreeDef {
        /// The top rows: a list written out, each with its `children`, or
        /// a query.
        #[serde(default)]
        pub rows: RowsDef,
        /// How each row from a query is shown: a template over the row,
        /// `it`.
        #[serde(default)]
        pub item: Option<TextProp>,
        /// For rows from a query: an expression giving the rows beneath a
        /// row, `it`.
        #[serde(default)]
        pub children: Option<Binding>,
        /// What is shown when there are no rows.
        #[serde(default)]
        pub empty: Option<TextProp>,
        /// The screen variable that holds the selected row's number, from
        /// 0, counted through the whole tree in the order written.
        #[serde(default)]
        pub selected: Option<String>,
        /// Effects run when the selection moves, with the row as `it`.
        #[serde(default)]
        pub on_select: UiEffects,
        /// Effects run when a row without children is activated, with the
        /// row as `it`.
        #[serde(default)]
        pub on_activate: UiEffects,
    }
}

widget_def! {
    /// `menu`: options with hotkeys.
    pub struct MenuDef {
        /// The options: a list written out, each with its `label`, `key`
        /// and `on_activate`, or a query.
        #[serde(default)]
        pub rows: RowsDef,
        /// How each option from a query is shown: a template over the
        /// row, `it`.
        #[serde(default)]
        pub item: Option<TextProp>,
        /// The screen variable that holds the selected option's number,
        /// from 0.
        #[serde(default)]
        pub selected: Option<String>,
        /// Effects run when the selection moves, with the row as `it`.
        #[serde(default)]
        pub on_select: UiEffects,
        /// Effects run when an option is activated, with the row as `it`,
        /// after the option's own.
        #[serde(default)]
        pub on_activate: UiEffects,
    }
}

widget_def! {
    /// `bar` and `gauge`: a value against a maximum. A `bar` draws the
    /// share filled; a `gauge` writes the two numbers.
    pub struct BarDef {
        /// The value.
        pub value: Prop<f64>,
        /// The value at which the bar is full. Left out, 1.
        #[serde(default = "one")]
        pub max: Prop<f64>,
        /// Text shown before the bar.
        #[serde(default)]
        pub label: Option<TextProp>,
    }
}

widget_def! {
    /// `input`: a line of text to edit.
    pub struct InputDef {
        /// The screen variable (`"screen.name"` or just `"name"`) or
        /// preference (`"pref.id"`) edited. It must hold a text.
        pub bind: String,
        /// Text shown before the field.
        #[serde(default)]
        pub label: Option<TextProp>,
        /// The most characters the text may have.
        #[serde(default)]
        pub max_len: Option<u16>,
        /// Effects run after the text changes.
        #[serde(default)]
        pub on_change: UiEffects,
    }
}

widget_def! {
    /// `number`: a number to step up and down.
    pub struct NumberDef {
        /// The screen variable or preference edited, written as for
        /// `input`. It must hold a number; a whole number stays whole.
        pub bind: String,
        /// Text shown before the number.
        #[serde(default)]
        pub label: Option<TextProp>,
        /// The least value allowed.
        #[serde(default)]
        pub min: Option<Prop<f64>>,
        /// The greatest value allowed.
        #[serde(default)]
        pub max: Option<Prop<f64>>,
        /// How much one step changes the value. Left out, 1.
        #[serde(default = "one")]
        pub step: Prop<f64>,
        /// Effects run after the value changes.
        #[serde(default)]
        pub on_change: UiEffects,
    }
}

widget_def! {
    /// `toggle`: a flag to turn on and off.
    pub struct ToggleDef {
        /// The screen variable or preference edited, written as for
        /// `input`. It must hold true or false.
        pub bind: String,
        /// Text shown beside the mark.
        #[serde(default)]
        pub label: Option<TextProp>,
        /// Effects run after the flag changes.
        #[serde(default)]
        pub on_change: UiEffects,
    }
}

widget_def! {
    /// `choice`: one of several options.
    pub struct ChoiceDef {
        /// The screen variable or preference edited, written as for
        /// `input`. It holds the chosen option's `value`.
        pub bind: String,
        /// Text shown before the choice.
        #[serde(default)]
        pub label: Option<TextProp>,
        /// The options: a list written out, or an expression.
        pub options: OptionsDef,
        /// Effects run after the choice changes.
        #[serde(default)]
        pub on_change: UiEffects,
    }
}

widget_def! {
    /// `button`: something to activate.
    pub struct ButtonDef {
        /// The text on the button.
        pub label: TextProp,
        /// Effects run when the button is activated.
        #[serde(default)]
        pub on_activate: UiEffects,
    }
}

widget_def! {
    /// `log`: the message log (12 §7.4).
    pub struct LogDef {
        /// The categories shown: a list of category names, or an
        /// expression. Left out, every line is shown.
        #[serde(default)]
        pub filter: Option<Prop<Vec<String>>>,
    }
}

widget_def! {
    /// `map_view`, `section_view`, `region_map` and `chart_view`: views of
    /// the world (12 §6.2). What they draw is package U4; the toolkit
    /// gives them their place, focus and fields.
    pub struct ViewDef {
        /// An expression giving what the view is centred on. Left out,
        /// the camera's subject.
        #[serde(default)]
        pub centre: Option<Binding>,
        /// How far the view is zoomed out: 1 shows one cell per cell.
        #[serde(default = "one")]
        pub zoom: Prop<f64>,
        /// The `overlay` definition drawn over the cells.
        #[serde(default)]
        pub overlay: Option<String>,
        /// Effects run when a place in the view is picked, with the place
        /// as `it`.
        #[serde(default)]
        pub on_pick: UiEffects,
    }
}

widget_def! {
    /// `figure`: a character-art drawing with regions coloured by
    /// expression.
    pub struct FigureWidgetDef {
        /// The `figure` definition drawn.
        pub figure: String,
        /// The colour of each region: an object from region name to a
        /// colour or style name, or an expression giving one, with the
        /// region's name as `it`.
        #[serde(default)]
        pub shade: Option<Prop<BTreeMap<String, String>>>,
    }
}

widget_def! {
    /// `inspect`: a browsable list of a subject's properties.
    pub struct InspectDef {
        /// An expression giving the subject whose properties are listed.
        /// It is compiled; listing a subject's properties comes with the
        /// inspector (tooling plan).
        #[serde(default)]
        pub of: Option<Binding>,
        /// Properties written out instead: rows with `label`, `value` and
        /// `children`.
        #[serde(default)]
        pub rows: RowsDef,
        /// The screen variable that holds the selected row's number.
        #[serde(default)]
        pub selected: Option<String>,
        /// Effects run when the selection moves, with the property as
        /// `it`.
        #[serde(default)]
        pub on_select: UiEffects,
        /// Effects run when a property without children is activated.
        #[serde(default)]
        pub on_activate: UiEffects,
    }
}

widget_def! {
    /// `tooltip`: shows the `tip` of whatever has the focus or the
    /// pointer.
    pub struct TooltipDef {
        /// What is shown when nothing with a `tip` has the focus.
        #[serde(default)]
        pub value: Option<TextProp>,
    }
}

widget_def! {
    /// `spacer`: empty space. It fills what is left over unless given a
    /// size.
    pub struct SpacerDef {}
}

widget_def! {
    /// `console`, `canvas` and `trace_view`: the widgets of the
    /// development screens (document 14), allowed only in screens whose
    /// `context` is `debug`. Their content is built with the tooling
    /// packages; the toolkit gives them their place and focus.
    pub struct DebugDef {}
}

/// One page of a `tabs` container.
#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TabDef {
    /// The text on the page's tab.
    pub title: TextProp,
    /// What the page shows.
    pub content: NodeDef,
}

/// `tabs`: one child at a time, chosen from a strip of titles.
#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TabsDef {
    /// The pages, in order.
    pub pages: Vec<TabDef>,
    /// The screen variable that holds the shown page's number, from 0.
    /// Left out, the container keeps it itself.
    #[serde(default)]
    pub selected: Option<String>,
}

/// `grid`: children in rows and columns.
#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GridDef {
    /// The width of each column, left to right.
    pub columns: Vec<Size>,
    /// The height of each row, top to bottom. Rows past the end of the
    /// list, and all rows when it is left out, fit their tallest child.
    #[serde(default)]
    pub rows: Vec<Size>,
    /// The children, filling the grid row by row.
    pub cells: Vec<NodeDef>,
}

/// What a layout node is: one of the containers of 12 §10.2 or one of the
/// widgets of 12 §10.3. In JSON it is the one key of the node that names
/// its kind.
#[derive(Clone, Debug, PartialEq)]
pub enum NodeKind {
    /// `rows`: children one below another.
    Rows(Vec<NodeDef>),
    /// `cols`: children side by side.
    Cols(Vec<NodeDef>),
    /// `stack`: children over one another, the last on top. A child with
    /// a `width` or `height` smaller than the stack is centred in it.
    Stack(Vec<NodeDef>),
    /// `grid`: children in rows and columns.
    Grid(GridDef),
    /// `tabs`: one child at a time.
    Tabs(TabsDef),
    /// `scroll`: a child larger than its space.
    Scroll(Box<NodeDef>),
    /// `slot`: a named place where other mods add widgets (12 §10.6).
    Slot(String),
    /// `text`.
    Text(TextDef),
    /// `list`.
    List(ListDef),
    /// `table`.
    Table(TableDef),
    /// `tree`.
    Tree(TreeDef),
    /// `menu`.
    Menu(MenuDef),
    /// `bar`.
    Bar(BarDef),
    /// `gauge`.
    Gauge(BarDef),
    /// `input`.
    Input(InputDef),
    /// `number`.
    Number(NumberDef),
    /// `toggle`.
    Toggle(ToggleDef),
    /// `choice`.
    Choice(ChoiceDef),
    /// `button`.
    Button(ButtonDef),
    /// `log`.
    Log(LogDef),
    /// `map_view`.
    MapView(ViewDef),
    /// `section_view`.
    SectionView(ViewDef),
    /// `region_map`.
    RegionMap(ViewDef),
    /// `chart_view`.
    ChartView(ViewDef),
    /// `figure`.
    Figure(FigureWidgetDef),
    /// `art`.
    Art(ArtDef),
    /// `inspect`.
    Inspect(InspectDef),
    /// `tooltip`.
    Tooltip(TooltipDef),
    /// `spacer`.
    Spacer(SpacerDef),
    /// `console`. Debug context only.
    Console(DebugDef),
    /// `canvas`. Debug context only.
    Canvas(DebugDef),
    /// `trace_view`. Debug context only.
    TraceView(DebugDef),
}

/// The keys that name a node's kind: the containers, then the widgets.
pub const NODE_KINDS: &[&str] = &[
    "rows",
    "cols",
    "stack",
    "grid",
    "tabs",
    "scroll",
    "slot",
    "text",
    "list",
    "table",
    "tree",
    "menu",
    "bar",
    "gauge",
    "input",
    "number",
    "toggle",
    "choice",
    "button",
    "log",
    "map_view",
    "section_view",
    "region_map",
    "chart_view",
    "figure",
    "art",
    "inspect",
    "tooltip",
    "spacer",
    "console",
    "canvas",
    "trace_view",
];

/// The keys a node may carry besides the one that names its kind.
pub const NODE_FIELDS: &[&str] = &[
    "id", "style", "if", "tip", "width", "height", "fill", "fit", "frame",
];

impl NodeKind {
    /// The key that names this kind in JSON.
    pub fn name(&self) -> &'static str {
        match self {
            NodeKind::Rows(_) => "rows",
            NodeKind::Cols(_) => "cols",
            NodeKind::Stack(_) => "stack",
            NodeKind::Grid(_) => "grid",
            NodeKind::Tabs(_) => "tabs",
            NodeKind::Scroll(_) => "scroll",
            NodeKind::Slot(_) => "slot",
            NodeKind::Text(_) => "text",
            NodeKind::List(_) => "list",
            NodeKind::Table(_) => "table",
            NodeKind::Tree(_) => "tree",
            NodeKind::Menu(_) => "menu",
            NodeKind::Bar(_) => "bar",
            NodeKind::Gauge(_) => "gauge",
            NodeKind::Input(_) => "input",
            NodeKind::Number(_) => "number",
            NodeKind::Toggle(_) => "toggle",
            NodeKind::Choice(_) => "choice",
            NodeKind::Button(_) => "button",
            NodeKind::Log(_) => "log",
            NodeKind::MapView(_) => "map_view",
            NodeKind::SectionView(_) => "section_view",
            NodeKind::RegionMap(_) => "region_map",
            NodeKind::ChartView(_) => "chart_view",
            NodeKind::Figure(_) => "figure",
            NodeKind::Art(_) => "art",
            NodeKind::Inspect(_) => "inspect",
            NodeKind::Tooltip(_) => "tooltip",
            NodeKind::Spacer(_) => "spacer",
            NodeKind::Console(_) => "console",
            NodeKind::Canvas(_) => "canvas",
            NodeKind::TraceView(_) => "trace_view",
        }
    }

    /// Reads the value of a kind key. `None` if `key` names no kind.
    fn read<'de, A: MapAccess<'de>>(
        key: &str,
        map: &mut A,
    ) -> Result<Option<(NodeKind, CommonDef)>, A::Error> {
        macro_rules! widget {
            ($variant:ident) => {{
                let mut body = map.next_value()?;
                let common = widget_common(&mut body);
                (NodeKind::$variant(body), common)
            }};
        }
        /// Lets the macro call `take_common` on whichever widget type the
        /// variant holds.
        fn widget_common<T: TakeCommon>(body: &mut T) -> CommonDef {
            body.take_common_fields()
        }
        let plain = |kind: NodeKind| (kind, CommonDef::default());
        Ok(Some(match key {
            "rows" => plain(NodeKind::Rows(map.next_value()?)),
            "cols" => plain(NodeKind::Cols(map.next_value()?)),
            "stack" => plain(NodeKind::Stack(map.next_value()?)),
            "grid" => plain(NodeKind::Grid(map.next_value()?)),
            "tabs" => plain(NodeKind::Tabs(map.next_value()?)),
            "scroll" => plain(NodeKind::Scroll(map.next_value()?)),
            "slot" => plain(NodeKind::Slot(map.next_value()?)),
            "text" => widget!(Text),
            "list" => widget!(List),
            "table" => widget!(Table),
            "tree" => widget!(Tree),
            "menu" => widget!(Menu),
            "bar" => widget!(Bar),
            "gauge" => widget!(Gauge),
            "input" => widget!(Input),
            "number" => widget!(Number),
            "toggle" => widget!(Toggle),
            "choice" => widget!(Choice),
            "button" => widget!(Button),
            "log" => widget!(Log),
            "map_view" => widget!(MapView),
            "section_view" => widget!(SectionView),
            "region_map" => widget!(RegionMap),
            "chart_view" => widget!(ChartView),
            "figure" => widget!(Figure),
            "art" => widget!(Art),
            "inspect" => widget!(Inspect),
            "tooltip" => widget!(Tooltip),
            "spacer" => widget!(Spacer),
            "console" => widget!(Console),
            "canvas" => widget!(Canvas),
            "trace_view" => widget!(TraceView),
            _ => return Ok(None),
        }))
    }
}

/// A widget's object, from which the common fields can be taken.
trait TakeCommon {
    fn take_common_fields(&mut self) -> CommonDef;
}

macro_rules! take_common {
    ($($ty:ident),*) => {
        $(impl TakeCommon for $ty {
            fn take_common_fields(&mut self) -> CommonDef {
                self.take_common()
            }
        })*
    };
}

take_common!(
    TextDef,
    ArtDef,
    ListDef,
    TableDef,
    TreeDef,
    MenuDef,
    BarDef,
    InputDef,
    NumberDef,
    ToggleDef,
    ChoiceDef,
    ButtonDef,
    LogDef,
    ViewDef,
    FigureWidgetDef,
    InspectDef,
    TooltipDef,
    SpacerDef,
    DebugDef
);

/// A border around a node, with or without a title.
#[derive(Clone, Debug, PartialEq)]
pub enum FrameDef {
    /// `"frame": true`: a border with no title.
    Plain,
    /// `"frame": "Wounds"`: a border with a title.
    Titled(TextProp),
}

/// One node of a screen's layout tree (12 §10.2): an object with exactly
/// one key naming its kind, and any of the keys below.
///
/// ```json
/// { "width": 34, "frame": "Wounds", "list": { "rows": ["A cut", "A bruise"] } }
/// ```
#[derive(Clone, Debug, PartialEq)]
pub struct NodeDef {
    /// `id`, `style`, `if` and `tip`. For a widget they may be written in
    /// the widget's own object instead; writing one in both places is an
    /// error.
    pub common: CommonDef,
    /// `width`: the node's width. Left out, the node fills the width it
    /// is given, or, among `cols`, takes what its kind takes by default.
    pub width: Option<Size>,
    /// `height`: the node's height, likewise.
    pub height: Option<Size>,
    /// `fill`: among `rows` or `cols`, a share by weight of the space
    /// left over along the container's direction.
    pub fill: Option<u16>,
    /// `fit`: among `rows` or `cols`, as large as its content along the
    /// container's direction.
    pub fit: bool,
    /// `frame`: a border around the node, with a title if given a text.
    pub frame: Option<FrameDef>,
    /// The key that names the node's kind, and its value.
    pub kind: NodeKind,
}

impl<'de> Deserialize<'de> for NodeDef {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        /// The value of `frame`: a title, or true or false.
        struct FrameField(Option<FrameDef>);
        impl<'de> Deserialize<'de> for FrameField {
            fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
                struct FrameVisitor;
                impl Visitor<'_> for FrameVisitor {
                    type Value = FrameField;
                    fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                        f.write_str("a title, or true for a border with none")
                    }
                    fn visit_bool<E: de::Error>(self, on: bool) -> Result<FrameField, E> {
                        Ok(FrameField(on.then_some(FrameDef::Plain)))
                    }
                    fn visit_str<E: de::Error>(self, title: &str) -> Result<FrameField, E> {
                        Ok(FrameField(Some(FrameDef::Titled(text_prop(title)))))
                    }
                }
                deserializer.deserialize_any(FrameVisitor)
            }
        }

        struct NodeVisitor;
        impl<'de> Visitor<'de> for NodeVisitor {
            type Value = NodeDef;
            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str("a layout node: an object with one container or widget key")
            }
            fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<NodeDef, A::Error> {
                let mut common = CommonDef::default();
                let (mut width, mut height, mut fill, mut fit, mut frame) =
                    (None, None, None, false, None);
                let mut kind: Option<(NodeKind, CommonDef)> = None;
                let mut seen: Vec<String> = Vec::new();
                while let Some(key) = map.next_key::<String>()? {
                    if seen.contains(&key) {
                        return Err(de::Error::custom(format!("the key `{key}` is given twice")));
                    }
                    match key.as_str() {
                        "id" => common.id = Some(map.next_value()?),
                        "style" => common.style = Some(map.next_value()?),
                        "if" => common.show_if = Some(map.next_value()?),
                        "tip" => common.tip = Some(map.next_value()?),
                        "width" => width = Some(map.next_value()?),
                        "height" => height = Some(map.next_value()?),
                        "fill" => fill = Some(map.next_value()?),
                        "fit" => fit = map.next_value()?,
                        "frame" => frame = map.next_value::<FrameField>()?.0,
                        other => match NodeKind::read(other, &mut map)? {
                            Some(read) => {
                                if let Some((first, _)) = &kind {
                                    return Err(de::Error::custom(format!(
                                        "a layout node is one thing, but this one is both `{}` and `{other}`",
                                        first.name()
                                    )));
                                }
                                kind = Some(read);
                            }
                            None => {
                                let mut expected: Vec<&'static str> = NODE_FIELDS.to_vec();
                                expected.extend_from_slice(NODE_KINDS);
                                return Err(de::Error::custom(format!(
                                    "unknown key `{other}` in a layout node: it is not a container, a widget or a node field (known: {})",
                                    expected.join(", ")
                                )));
                            }
                        },
                    }
                    seen.push(key);
                }
                let Some((kind, inner)) = kind else {
                    return Err(de::Error::custom(
                        "a layout node needs one key that says what it is, such as `rows`, `text` or `list`",
                    ));
                };
                if fit && fill.is_some() {
                    return Err(de::Error::custom("a node cannot both `fit` and `fill`"));
                }
                // The common fields may be in the widget's own object.
                let name = kind.name();
                let both = |field: &str| {
                    de::Error::custom(format!(
                        "`{field}` is given both on the node and inside its `{name}`"
                    ))
                };
                if inner.id.is_some() {
                    if common.id.is_some() {
                        return Err(both("id"));
                    }
                    common.id = inner.id;
                }
                if inner.style.is_some() {
                    if common.style.is_some() {
                        return Err(both("style"));
                    }
                    common.style = inner.style;
                }
                if inner.show_if.is_some() {
                    if common.show_if.is_some() {
                        return Err(both("if"));
                    }
                    common.show_if = inner.show_if;
                }
                if inner.tip.is_some() {
                    if common.tip.is_some() {
                        return Err(both("tip"));
                    }
                    common.tip = inner.tip;
                }
                Ok(NodeDef {
                    common,
                    width,
                    height,
                    fill,
                    fit,
                    frame,
                    kind,
                })
            }
        }
        deserializer.deserialize_map(NodeVisitor)
    }
}

/// A `screen` definition (12 §10.1).
///
/// ```json
/// { "type": "screen", "id": "condition",
///   "title": "Condition", "context": "menu", "min_size": [40, 12],
///   "vars": { "tab": 0 },
///   "layout": { "cols": [
///     { "width": 34, "frame": "Wounds", "list": { "rows": ["A cut"], "empty": "Unhurt." } },
///     { "fill": 1, "rows": [ { "bar": { "label": "Blood", "value": 0.8, "style": "blood" } } ] } ] },
///   "keys": { "back": [ { "close": {} } ] } }
/// ```
#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ScreenDef {
    /// The screen's title.
    #[serde(default)]
    pub title: Option<TextProp>,
    /// The keymap context (12 §9.2) the screen's keys are named in. Left
    /// out, `menu`. `debug` also lets the screen see truth and use the
    /// development widgets.
    #[serde(default)]
    pub context: Option<String>,
    /// The values the screen is opened with: each name and its type, such
    /// as `"who": "entity"`. Read by bindings as `$name`.
    #[serde(default)]
    pub params: BTreeMap<String, String>,
    /// The screen's own variables and their starting values. Read by
    /// bindings as `screen.name`; written by widgets and the `set` effect.
    #[serde(default)]
    pub vars: BTreeMap<String, Literal>,
    /// The smallest grid the screen can be shown on, as `[columns, rows]`.
    /// On a smaller one the screen is replaced by a notice saying how
    /// large the window must be.
    #[serde(default)]
    pub min_size: Option<[u16; 2]>,
    /// The layout tree.
    pub layout: NodeDef,
    /// What the screen does when a key with one of these names (12 §9.2)
    /// is pressed and no widget used it: each a list of effects.
    #[serde(default)]
    pub keys: BTreeMap<String, UiEffects>,
    /// `popup`: the screen does not fill the frame. It is drawn over the
    /// screens beneath it, which stay on show.
    #[serde(default)]
    pub popup: Option<PopupDef>,
}

/// Where a popup sits in the frame.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Place {
    /// In the middle.
    #[default]
    #[serde(alias = "center")]
    Centre,
    /// Against the top edge, in the middle of it.
    Top,
    /// Against the bottom edge.
    Bottom,
    /// Against the left edge.
    Left,
    /// Against the right edge.
    Right,
    /// In the top left corner.
    TopLeft,
    /// In the top right corner.
    TopRight,
    /// In the bottom left corner.
    BottomLeft,
    /// In the bottom right corner.
    BottomRight,
}

/// `popup`: the size and place of a screen that does not fill the frame
/// (12 §10.1).
///
/// ```json
/// "popup": { "width": 40, "height": "fit", "place": "centre", "dim": true }
/// ```
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PopupDef {
    /// The popup's width: cells, a percentage of the frame, or `"fit"`.
    /// Left out, it fits its content.
    #[serde(default)]
    pub width: Option<Size>,
    /// The popup's height, likewise.
    #[serde(default)]
    pub height: Option<Size>,
    /// Where it sits in the frame. Left out, in the middle.
    #[serde(default)]
    pub place: Place,
    /// Whether everything beneath it is drawn dimmed.
    #[serde(default)]
    pub dim: bool,
}

/// A `ui_part` definition (12 §10.6): a widget another mod puts into a
/// slot of a screen.
///
/// ```json
/// { "type": "ui_part", "id": "mana_bar", "into": "ui:hud.bars", "order": 20,
///   "if": "view.has_trait('attuned')",
///   "widget": { "bar": { "label": "Mana", "value": "view.pool('mana').frac" } } }
/// ```
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UiPartDef {
    /// The slot, by its qualified name: the namespace of the screen that
    /// places it, a colon, and the slot's name, as in `ui:hud.bars`. A
    /// bare name means a slot of the part's own mod.
    pub into: Located<String>,
    /// Where the part comes among the others in the slot: lower first,
    /// and parts of equal order by qualified id. Left out, 0.
    #[serde(default)]
    pub order: i32,
    /// `if`: the part is shown, and takes space, only while this holds.
    #[serde(default, rename = "if")]
    pub show_if: Option<Prop<bool>>,
    /// Whether the slot may be missing. Left out, a part aimed at a slot
    /// that does not exist is a load error.
    #[serde(default)]
    pub optional: bool,
    /// The widget, or any layout node.
    pub widget: NodeDef,
}

impl UiPartDef {
    /// The qualified name of the slot the part goes into, for a part
    /// defined in `namespace`.
    pub fn slot(&self, namespace: &str) -> String {
        if self.into.value.contains(':') {
            self.into.value.clone()
        } else {
            format!("{namespace}:{}", self.into.value)
        }
    }

    /// The part as the node that is put into its slot: its widget, shown
    /// only while the part's `if` holds.
    pub(crate) fn node(&self) -> NodeDef {
        let Some(condition) = &self.show_if else {
            return self.widget.clone();
        };
        if self.widget.common.show_if.is_none() {
            let mut node = self.widget.clone();
            node.common.show_if = Some(condition.clone());
            return node;
        }
        // Both must hold: the part's condition goes on a node around the
        // widget, which keeps its own.
        NodeDef {
            common: CommonDef {
                show_if: Some(condition.clone()),
                ..CommonDef::default()
            },
            width: None,
            height: None,
            fill: self.widget.fill,
            fit: self.widget.fit,
            frame: None,
            kind: NodeKind::Rows(vec![self.widget.clone()]),
        }
    }

    /// Compiles the part's bindings. A part may be placed in any screen,
    /// so it sees `view`, `party`, `app` and the screen's variables, not
    /// its parameters, and is held to what an ordinary screen may know.
    pub fn compile_bindings(&mut self, cx: &mut CompileCx<'_>) {
        let mut bx = BindCx {
            cx,
            debug: false,
            params: Vec::new(),
            vars: None,
            it: None,
        };
        if let Some(condition) = &mut self.show_if {
            condition.compile(&mut bx);
        }
        self.widget.compile(&mut bx);
    }
}

/// What kind of value a preference holds.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PrefKind {
    /// True or false.
    Flag,
    /// A number, within `min` and `max` if they are given.
    Number,
    /// A text.
    Text,
    /// One of the texts in `options`.
    Choice,
}

/// A `preference` definition (12 §10.4): a player's choice about display
/// and input, kept outside saves and invisible to rules.
///
/// ```json
/// { "type": "preference", "id": "damage_numbers", "kind": "flag", "default": false,
///   "name": "Show damage numbers", "doc": "Float the energy of each hit over its target." }
/// ```
#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PreferenceDef {
    /// What kind of value it holds: `flag`, `number`, `text` or
    /// `choice`.
    pub kind: PrefKind,
    /// The value it has until the player or a profile sets one.
    pub default: Literal,
    /// For a `choice`: the texts it may hold.
    #[serde(default)]
    pub options: Vec<String>,
    /// For a `number`: the least value it may hold.
    #[serde(default)]
    pub min: Option<f64>,
    /// For a `number`: the greatest value it may hold.
    #[serde(default)]
    pub max: Option<f64>,
    /// What the preference is called where the player sets it.
    #[serde(default)]
    pub name: String,
    /// What it does, for the options screen and the reference.
    #[serde(default)]
    pub doc: String,
}

impl PreferenceDef {
    /// Why `value` cannot be this preference's value, if it cannot.
    pub fn refuses(&self, value: &Value) -> Option<String> {
        match (self.kind, value) {
            (PrefKind::Flag, Value::Bool(_)) | (PrefKind::Text, Value::Str(_)) => None,
            (PrefKind::Number, Value::Int(_) | Value::Real(_)) => {
                let number = match value {
                    Value::Int(n) => *n as f64,
                    Value::Real(r) => r.get(),
                    _ => 0.0,
                };
                if self.min.is_some_and(|min| number < min) {
                    Some(format!("{value} is below the least it may be"))
                } else if self.max.is_some_and(|max| number > max) {
                    Some(format!("{value} is above the most it may be"))
                } else {
                    None
                }
            }
            (PrefKind::Choice, Value::Str(text)) => (!self.options.contains(text)).then(|| {
                format!(
                    "`{text}` is not one of its options ({})",
                    self.options.join(", ")
                )
            }),
            (kind, other) => Some(format!(
                "it holds {}, not {}",
                match kind {
                    PrefKind::Flag => "true or false",
                    PrefKind::Number => "a number",
                    PrefKind::Text | PrefKind::Choice => "a text",
                },
                other.type_name()
            )),
        }
    }

    /// What is wrong with the definition itself, if anything.
    pub fn problem(&self) -> Option<String> {
        if self.kind == PrefKind::Choice && self.options.is_empty() {
            return Some("a `choice` needs `options`".to_owned());
        }
        if self.kind != PrefKind::Choice && !self.options.is_empty() {
            return Some("`options` belongs with the kind `choice`".to_owned());
        }
        if let (Some(min), Some(max)) = (self.min, self.max)
            && min > max
        {
            return Some("`min` is above `max`".to_owned());
        }
        self.refuses(&self.default.0)
            .map(|reason| format!("the default cannot be its value: {reason}"))
    }
}

/// What `it` is for the rows of a listing written out: the row's label.
fn label_it() -> ItType {
    ItType {
        ty: Type::Str,
        own: false,
    }
}

impl RowDef {
    fn compile(&mut self, bx: &mut BindCx<'_, '_>) {
        self.label.compile(bx);
        if let Some(value) = &mut self.value {
            value.compile(bx);
        }
        for cell in &mut self.cells {
            cell.compile(bx);
        }
        if let Some(tip) = &mut self.tip {
            tip.compile(bx);
        }
        for child in &mut self.children {
            child.compile(bx);
        }
        bx.with_it(Some(label_it()), |bx| self.on_activate.compile(bx));
    }
}

/// Compiles the rows of a listing and says what `it` is for each.
fn compile_rows(rows: &mut RowsDef, bx: &mut BindCx<'_, '_>) -> ItType {
    match rows {
        RowsDef::Static(defs) => {
            for row in defs {
                row.compile(bx);
            }
            label_it()
        }
        RowsDef::Bound(binding) => {
            binding.compile_expr(bx, &Type::list(Type::Any).nullable());
            ItType {
                ty: element_type(binding.ty()),
                own: binding.is_own(),
            }
        }
    }
}

fn compile_opt(text: &mut Option<TextProp>, bx: &mut BindCx<'_, '_>) {
    if let Some(text) = text {
        text.compile(bx);
    }
}

impl NodeDef {
    /// Compiles every binding of the node and of what is inside it.
    pub(crate) fn compile(&mut self, bx: &mut BindCx<'_, '_>) {
        if let Some(condition) = &mut self.common.show_if {
            condition.compile(bx);
        }
        compile_opt(&mut self.common.tip, bx);
        if let Some(FrameDef::Titled(title)) = &mut self.frame {
            title.compile(bx);
        }
        match &mut self.kind {
            NodeKind::Rows(children) | NodeKind::Cols(children) | NodeKind::Stack(children) => {
                for child in children {
                    child.compile(bx);
                }
            }
            NodeKind::Grid(grid) => {
                for child in &mut grid.cells {
                    child.compile(bx);
                }
            }
            NodeKind::Tabs(tabs) => {
                for page in &mut tabs.pages {
                    page.title.compile(bx);
                    page.content.compile(bx);
                }
            }
            NodeKind::Scroll(child) => child.compile(bx),
            NodeKind::Slot(_) | NodeKind::Spacer(_) => {}
            NodeKind::Console(_) | NodeKind::Canvas(_) | NodeKind::TraceView(_) => {}
            NodeKind::Text(text) => text.value.compile(bx),
            NodeKind::Art(art) => art.value.compile(bx),
            NodeKind::List(w) => {
                let it = compile_rows(&mut w.rows, bx);
                compile_opt(&mut w.empty, bx);
                bx.with_it(Some(it), |bx| {
                    compile_opt(&mut w.item, bx);
                    w.on_select.compile(bx);
                    w.on_activate.compile(bx);
                });
            }
            NodeKind::Menu(w) => {
                let it = compile_rows(&mut w.rows, bx);
                bx.with_it(Some(it), |bx| {
                    compile_opt(&mut w.item, bx);
                    w.on_select.compile(bx);
                    w.on_activate.compile(bx);
                });
            }
            NodeKind::Tree(w) => {
                let it = compile_rows(&mut w.rows, bx);
                compile_opt(&mut w.empty, bx);
                bx.with_it(Some(it), |bx| {
                    compile_opt(&mut w.item, bx);
                    if let Some(children) = &mut w.children {
                        children.compile_expr(bx, &Type::list(Type::Any).nullable());
                    }
                    w.on_select.compile(bx);
                    w.on_activate.compile(bx);
                });
            }
            NodeKind::Table(w) => {
                let it = compile_rows(&mut w.rows, bx);
                compile_opt(&mut w.empty, bx);
                for column in &mut w.columns {
                    column.title.compile(bx);
                }
                bx.with_it(Some(it), |bx| {
                    for column in &mut w.columns {
                        compile_opt(&mut column.value, bx);
                    }
                    w.on_select.compile(bx);
                    w.on_activate.compile(bx);
                });
            }
            NodeKind::Inspect(w) => {
                if let Some(of) = &mut w.of {
                    of.compile_expr(bx, &Type::Any);
                }
                let it = compile_rows(&mut w.rows, bx);
                bx.with_it(Some(it), |bx| {
                    w.on_select.compile(bx);
                    w.on_activate.compile(bx);
                });
            }
            NodeKind::Bar(bar) | NodeKind::Gauge(bar) => {
                bar.value.compile(bx);
                bar.max.compile(bx);
                compile_opt(&mut bar.label, bx);
            }
            NodeKind::Input(w) => {
                compile_opt(&mut w.label, bx);
                w.on_change.compile(bx);
            }
            NodeKind::Number(w) => {
                compile_opt(&mut w.label, bx);
                if let Some(min) = &mut w.min {
                    min.compile(bx);
                }
                if let Some(max) = &mut w.max {
                    max.compile(bx);
                }
                w.step.compile(bx);
                w.on_change.compile(bx);
            }
            NodeKind::Toggle(w) => {
                compile_opt(&mut w.label, bx);
                w.on_change.compile(bx);
            }
            NodeKind::Choice(w) => {
                compile_opt(&mut w.label, bx);
                match &mut w.options {
                    OptionsDef::Static(options) => {
                        for option in options {
                            compile_opt(&mut option.label, bx);
                        }
                    }
                    OptionsDef::Bound(binding) => {
                        binding.compile_expr(bx, &Type::list(Type::Any).nullable());
                    }
                }
                w.on_change.compile(bx);
            }
            NodeKind::Button(w) => {
                w.label.compile(bx);
                w.on_activate.compile(bx);
            }
            NodeKind::Log(w) => {
                if let Some(Prop::Bound(binding)) = &mut w.filter {
                    binding.compile_expr(bx, &Type::list(Type::Str).nullable());
                }
            }
            NodeKind::MapView(v)
            | NodeKind::SectionView(v)
            | NodeKind::RegionMap(v)
            | NodeKind::ChartView(v) => {
                if let Some(centre) = &mut v.centre {
                    centre.compile_expr(bx, &Type::Any);
                }
                v.zoom.compile(bx);
                // The place that was picked, whatever a view picks.
                let it = ItType {
                    ty: Type::Any,
                    own: false,
                };
                bx.with_it(Some(it), |bx| v.on_pick.compile(bx));
            }
            NodeKind::Figure(w) => {
                if let Some(Prop::Bound(binding)) = &mut w.shade {
                    // The region's name.
                    bx.with_it(Some(label_it()), |bx| {
                        binding.compile_expr(bx, &Type::Str.nullable());
                    });
                }
            }
            NodeKind::Tooltip(w) => compile_opt(&mut w.value, bx),
        }
    }
}

impl ScreenDef {
    /// Whether the screen's context is `debug`: it may see truth.
    pub fn is_debug(&self) -> bool {
        self.context.as_deref() == Some(CONTEXT_DEBUG)
    }

    /// Compiles every binding of the screen (12 §10.4): each bound
    /// property and each expression of its effects, in a pure context
    /// that offers `view`, `party`, `app`, the screen's variables as
    /// `screen.name`, its parameters as `$name` and, in a row, `it`.
    /// Unless the screen's context is `debug`, it is a mind context:
    /// only the party's own characters are read in full.
    ///
    /// The loader calls this through `Def::compile`. What is wrong is
    /// reported at the file, line and column inside the string.
    pub fn compile_bindings(&mut self, cx: &mut CompileCx<'_>) {
        let debug = self.is_debug();
        let entity = cx
            .registry()
            .kind_id(super::bind::KIND_ENTITY)
            .map_or(Type::Any, Type::Subject);
        let mut params = Vec::with_capacity(self.params.len());
        for (name, written) in &self.params {
            let (ty, own) = if written == TYPE_PARTY_MEMBER {
                (entity.clone(), true)
            } else {
                let path = format!("params.{name}");
                (cx.parse_type(written, None, &path).unwrap_or(Type::Any), false)
            };
            params.push(ParamType {
                name: name.clone(),
                ty,
                own,
            });
        }
        let mut bx = BindCx {
            cx,
            debug,
            params,
            vars: Some(self.vars.clone()),
            it: None,
        };
        compile_opt(&mut self.title, &mut bx);
        self.layout.compile(&mut bx);
        for effects in self.keys.values_mut() {
            effects.compile(&mut bx);
        }
    }
}

/// A `figure` definition: rows of character art with named regions, drawn
/// by the `figure` widget.
///
/// ```json
/// { "type": "figure", "id": "body",
///   "rows": [ " O ", "/|\\", "/ \\" ],
///   "map":  [ " h ", "ata", "l l" ],
///   "regions": { "h": "head", "t": "torso", "a": "arms", "l": "legs" } }
/// ```
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FigureDef {
    /// The drawing: one string per row, one character per cell.
    pub rows: Vec<String>,
    /// Which region each cell belongs to: rows of the same shape as
    /// `rows`, each character a key of `regions` or a space for none.
    #[serde(default)]
    pub map: Vec<String>,
    /// The name of the region each character of `map` stands for.
    #[serde(default)]
    pub regions: BTreeMap<String, String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn node(json: &str) -> Result<NodeDef, serde_json::Error> {
        serde_json::from_str(json)
    }

    #[test]
    fn the_screen_of_the_specification_loads() {
        let screen: ScreenDef = serde_json::from_str(
            r#"{ "title": "{$who.name}", "context": "menu",
                 "params": { "who": "entity" }, "vars": { "tab": 0 },
                 "layout": { "cols": [
                   { "width": 34, "frame": "Wounds",
                     "list": { "rows": { "wounds": { "of": "$who" } },
                               "item": "{it.part.name}: {it.kind.name}", "empty": "Unhurt." } },
                   { "fill": 1, "rows": [
                     { "bar": { "label": "Blood", "value": "$who.pool.blood.frac", "style": "blood" } },
                     { "figure": { "figure": "body", "shade": "part_shade($who, it)" } },
                     { "slot": "condition.extra" } ] } ] },
                 "keys": { "back": [ { "close": {} } ],
                           "tab":  [ { "set": { "screen.tab": "(screen.tab + 1) % 3" } } ] } }"#,
        )
        .unwrap();
        assert_eq!(screen.title, Some(Prop::Bound("{$who.name}".into())));
        assert_eq!(screen.vars["tab"], Literal(Value::Int(0)));
        assert_eq!(screen.keys["back"].names().collect::<Vec<_>>(), ["close"]);
        let NodeKind::Cols(cols) = &screen.layout.kind else {
            panic!("cols")
        };
        assert_eq!(cols[0].width, Some(Size::Cells(34)));
        assert_eq!(
            cols[0].frame,
            Some(FrameDef::Titled(Prop::Static("Wounds".into())))
        );
        let NodeKind::List(list) = &cols[0].kind else {
            panic!("list")
        };
        assert_eq!(
            list.rows,
            RowsDef::Bound(r#"{"wounds":{"of":"$who"}}"#.into())
        );
        assert_eq!(list.empty, Some(Prop::Static("Unhurt.".into())));
        assert_eq!(cols[1].fill, Some(1));
        let NodeKind::Rows(rows) = &cols[1].kind else {
            panic!("rows")
        };
        assert_eq!(
            rows[0].common.style.as_deref(),
            Some("blood"),
            "style inside the widget moves to the node"
        );
        let NodeKind::Bar(bar) = &rows[0].kind else {
            panic!("bar")
        };
        assert_eq!(bar.value, Prop::Bound("$who.pool.blood.frac".into()));
        assert_eq!(bar.max, Prop::Static(1.0));
        assert_eq!(rows[2].kind, NodeKind::Slot("condition.extra".into()));
    }

    #[test]
    fn an_unknown_widget_is_an_error_that_names_it() {
        let err = node(r#"{ "lst": { "rows": [] } }"#)
            .unwrap_err()
            .to_string();
        assert!(err.contains("unknown key `lst`"), "{err}");
        assert!(err.contains("list"), "{err}");
        let err = node(r#"{ "width": 3 }"#).unwrap_err().to_string();
        assert!(err.contains("needs one key that says what it is"), "{err}");
        let err = node(r#"{ "text": "a", "button": { "label": "b" } }"#)
            .unwrap_err()
            .to_string();
        assert!(err.contains("both `text` and `button`"), "{err}");
        let err = node(r#"{ "text": { "value": "a", "colour": "red" } }"#)
            .unwrap_err()
            .to_string();
        assert!(err.contains("unknown field `colour`"), "{err}");
        let err = node(r#"{ "style": "a", "text": { "value": "a", "style": "b" } }"#)
            .unwrap_err()
            .to_string();
        assert!(err.contains("`style` is given both"), "{err}");
        let err = node(r#"{ "fit": true, "fill": 1, "spacer": {} }"#)
            .unwrap_err()
            .to_string();
        assert!(err.contains("both `fit` and `fill`"), "{err}");
        let err = node(r#"["text"]"#).unwrap_err().to_string();
        assert!(err.contains("a layout node"), "{err}");
    }

    #[test]
    fn sizes_are_cells_percentages_fit_or_fill() {
        let size = |json: &str| serde_json::from_str::<Size>(json);
        assert_eq!(size("34").unwrap(), Size::Cells(34));
        assert_eq!(size(r#""50%""#).unwrap(), Size::Percent(50));
        assert_eq!(size(r#""fit""#).unwrap(), Size::Fit);
        assert_eq!(size(r#""fill""#).unwrap(), Size::Fill(1));
        assert_eq!(size(r#"{ "fill": 3 }"#).unwrap(), Size::Fill(3));
        for bad in [
            "-1",
            "70000",
            r#""150%""#,
            r#""wide""#,
            r#"{ "fil": 1 }"#,
            "1.5",
        ] {
            assert!(size(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn properties_are_static_or_bound() {
        assert_eq!(
            serde_json::from_str::<Prop<f64>>("3").unwrap(),
            Prop::Static(3.0)
        );
        assert_eq!(
            serde_json::from_str::<Prop<f64>>("0.25").unwrap(),
            Prop::Static(0.25)
        );
        assert_eq!(
            serde_json::from_str::<Prop<f64>>(r#""view.hp""#).unwrap(),
            Prop::Bound("view.hp".into())
        );
        assert_eq!(
            serde_json::from_str::<Prop<f64>>(r#"{ "expr": "a + b" }"#).unwrap(),
            Prop::Bound("a + b".into())
        );
        assert_eq!(
            serde_json::from_str::<Prop<bool>>("false").unwrap(),
            Prop::Static(false)
        );
        assert_eq!(
            serde_json::from_str::<Prop<bool>>(r#""screen.tab == 1""#).unwrap(),
            Prop::Bound("screen.tab == 1".into())
        );
        assert!(serde_json::from_str::<Prop<bool>>("3").is_err());
        assert_eq!(
            text_prop("plain {{braces}}"),
            Prop::Static("plain {braces}".into())
        );
        assert_eq!(
            text_prop("Hello, {name(view)}."),
            Prop::Bound("Hello, {name(view)}.".into())
        );
        assert_eq!(
            serde_json::from_str::<Prop<Vec<String>>>(r#"["combat", "speech"]"#).unwrap(),
            Prop::Static(vec!["combat".into(), "speech".into()])
        );
    }

    #[test]
    fn rows_have_shorthands() {
        let rows: RowsDef = serde_json::from_str(
            r#"[ "Plain", ["a", "b"], { "label": "Full", "key": "n", "children": ["Child"], "open": true,
                 "on_activate": [ { "open": "new_game" } ] } ]"#,
        )
        .unwrap();
        let RowsDef::Static(rows) = rows else {
            panic!("static")
        };
        assert_eq!(rows[0].label, Prop::Static("Plain".into()));
        assert_eq!(rows[1].cells.len(), 2);
        assert_eq!(rows[2].key, Some(KeyChord::char('n')));
        assert_eq!(rows[2].children[0].label, Prop::Static("Child".into()));
        assert_eq!(rows[2].on_activate.names().collect::<Vec<_>>(), ["open"]);
        assert!(serde_json::from_str::<RowsDef>(r#"[ { "lable": "x" } ]"#).is_err());
        assert_eq!(
            serde_json::from_str::<RowsDef>(r#""view.items""#).unwrap(),
            RowsDef::Bound("view.items".into())
        );
    }

    #[test]
    fn literals_keep_their_kind() {
        let lit = |json: &str| serde_json::from_str::<Literal>(json).unwrap().0;
        assert_eq!(lit("true"), Value::Bool(true));
        assert_eq!(lit("3"), Value::Int(3));
        assert_eq!(lit("0.5"), Value::Real(Real::new(0.5).unwrap()));
        assert_eq!(lit(r#""x""#), Value::Str("x".into()));
        assert!(serde_json::from_str::<Literal>("[1]").is_err());
    }

    #[test]
    fn every_kind_key_is_read_and_named() {
        for kind in NODE_KINDS {
            let body = match *kind {
                "rows" | "cols" | "stack" => "[]".to_owned(),
                "grid" => r#"{ "columns": [1], "cells": [] }"#.to_owned(),
                "tabs" => r#"{ "pages": [] }"#.to_owned(),
                "scroll" => r#"{ "spacer": {} }"#.to_owned(),
                "slot" => r#""a.b""#.to_owned(),
                "text" | "art" => r#"{ "value": "x" }"#.to_owned(),
                "table" => r#"{ "columns": [] }"#.to_owned(),
                "bar" | "gauge" => r#"{ "value": 1 }"#.to_owned(),
                "input" | "number" | "toggle" => r#"{ "bind": "x" }"#.to_owned(),
                "choice" => r#"{ "bind": "x", "options": ["a"] }"#.to_owned(),
                "button" => r#"{ "label": "x" }"#.to_owned(),
                "figure" => r#"{ "figure": "x" }"#.to_owned(),
                _ => "{}".to_owned(),
            };
            let parsed = node(&format!(r#"{{ "{kind}": {body} }}"#))
                .unwrap_or_else(|e| panic!("{kind}: {e}"));
            assert_eq!(parsed.kind.name(), *kind);
        }
    }
}
