//! The reference as one JSON document, for editors and other tools, and
//! the small JSON writer the schemas use too.

use std::fmt::Write as _;

use clash_data::expr::ExprRegistry;
use clash_data::shape::{JsonKind, NamedShape, Shape, VariantForm, Via};
use clash_expr::ParamSpec;

use super::{Catalog, field_doc, named_doc};

/// A JSON value to write. Objects keep the order their members are
/// given in.
#[derive(Clone, Debug, PartialEq)]
pub enum Json {
    /// `null`.
    Null,
    /// `true` or `false`.
    Bool(bool),
    /// A whole number.
    Int(i64),
    /// A number.
    Real(f64),
    /// A string.
    Str(String),
    /// A list.
    Array(Vec<Json>),
    /// An object.
    Object(Vec<(String, Json)>),
}

impl Json {
    /// A string.
    pub fn str(text: impl Into<String>) -> Json {
        Json::Str(text.into())
    }

    /// An object from its members.
    pub fn object<K: Into<String>>(members: impl IntoIterator<Item = (K, Json)>) -> Json {
        Json::Object(
            members
                .into_iter()
                .map(|(key, value)| (key.into(), value))
                .collect(),
        )
    }

    /// The JSON text, indented by two spaces, with a newline at the end.
    pub fn pretty(&self) -> String {
        let mut out = String::new();
        self.write(&mut out, 0);
        out.push('\n');
        out
    }

    fn write(&self, out: &mut String, depth: usize) {
        let indent = "  ".repeat(depth + 1);
        let close = "  ".repeat(depth);
        match self {
            Json::Null => out.push_str("null"),
            Json::Bool(value) => out.push_str(if *value { "true" } else { "false" }),
            Json::Int(value) => {
                let _ = write!(out, "{value}");
            }
            Json::Real(value) => {
                if value.is_finite() && value.fract() == 0.0 && value.abs() < 1e15 {
                    let _ = write!(out, "{value:.1}");
                } else if value.is_finite() {
                    let _ = write!(out, "{value}");
                } else {
                    out.push_str("null");
                }
            }
            Json::Str(text) => quote(text, out),
            Json::Array(items) if items.is_empty() => out.push_str("[]"),
            Json::Array(items) => {
                out.push_str("[\n");
                for (index, item) in items.iter().enumerate() {
                    out.push_str(&indent);
                    item.write(out, depth + 1);
                    if index + 1 < items.len() {
                        out.push(',');
                    }
                    out.push('\n');
                }
                out.push_str(&close);
                out.push(']');
            }
            Json::Object(members) if members.is_empty() => out.push_str("{}"),
            Json::Object(members) => {
                out.push_str("{\n");
                for (index, (key, value)) in members.iter().enumerate() {
                    out.push_str(&indent);
                    quote(key, out);
                    out.push_str(": ");
                    value.write(out, depth + 1);
                    if index + 1 < members.len() {
                        out.push(',');
                    }
                    out.push('\n');
                }
                out.push_str(&close);
                out.push('}');
            }
        }
    }
}

/// Writes a string as JSON, escaped.
fn quote(text: &str, out: &mut String) {
    out.push('"');
    for c in text.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => {
                let _ = write!(out, "\\u{:04x}", c as u32);
            }
            c => out.push(c),
        }
    }
    out.push('"');
}

fn params(registry: &ExprRegistry, params: &[ParamSpec]) -> Json {
    Json::Array(
        params
            .iter()
            .map(|param| {
                Json::object([
                    ("name", Json::str(&param.name)),
                    ("type", Json::str(registry.type_name(&param.ty))),
                    ("optional", Json::Bool(param.optional)),
                    ("doc", Json::str(&param.doc)),
                ])
            })
            .collect(),
    )
}

/// A shape as JSON: `{ "kind": …, … }`.
pub fn shape(shape: &Shape) -> Json {
    let kind = |name: &str| ("kind", Json::str(name));
    match shape {
        Shape::Unknown => Json::object([kind("unknown")]),
        Shape::Bool => Json::object([kind("bool")]),
        Shape::Int { signed } => Json::object([kind("int"), ("signed", Json::Bool(*signed))]),
        Shape::Real => Json::object([kind("number")]),
        Shape::Char => Json::object([kind("char")]),
        Shape::String => Json::object([kind("string")]),
        Shape::Quantity(dim) => Json::object([
            kind("quantity"),
            ("measures", Json::str(dim.name())),
            ("unit", Json::str(dim.si_unit())),
        ]),
        Shape::Duration => Json::object([
            kind("quantity"),
            ("measures", Json::str("a time")),
            ("unit", Json::str("s")),
        ]),
        Shape::Ref(ty) => Json::object([kind("ref"), ("type", Json::str(*ty))]),
        Shape::Inline(ty) => Json::object([kind("inline"), ("type", Json::str(*ty))]),
        Shape::Tag => Json::object([kind("tag")]),
        Shape::List(inner) => Json::object([kind("list"), ("of", self::shape(inner))]),
        Shape::Tuple(items) => Json::object([
            kind("tuple"),
            ("of", Json::Array(items.iter().map(self::shape).collect())),
        ]),
        Shape::Map(key, value) => Json::object([
            kind("map"),
            ("key", self::shape(key)),
            ("value", self::shape(value)),
        ]),
        Shape::Optional(inner) => Json::object([kind("optional"), ("of", self::shape(inner))]),
        Shape::Named(key) => Json::object([kind("named"), ("key", Json::str(key))]),
        Shape::Gated(feature, inner) => Json::object([
            kind("experimental"),
            ("feature", Json::str(*feature)),
            ("of", self::shape(inner)),
        ]),
        Shape::Any => Json::object([kind("any")]),
        Shape::Unit => Json::object([kind("null")]),
    }
}

fn kind_name(kind: JsonKind) -> Json {
    Json::str(kind.name())
}

/// The whole reference as one JSON document.
pub fn document(catalog: &Catalog) -> String {
    let registry = &catalog.registry;
    let mut types = Vec::new();
    for entry in &catalog.types {
        let mut named = Vec::new();
        for (key, item) in &entry.shapes.named {
            let body = match &item.shape {
                NamedShape::Struct(shape) => Json::object([
                    ("is", Json::str("struct")),
                    (
                        "fields",
                        Json::Array(
                            shape
                                .fields
                                .iter()
                                .map(|field| {
                                    Json::object([
                                        ("name", Json::str(field.name)),
                                        ("shape", self::shape(&field.shape)),
                                        ("required", Json::Bool(field.required)),
                                        (
                                            "doc",
                                            Json::str(field_doc(item, field.name).unwrap_or("")),
                                        ),
                                    ])
                                })
                                .collect(),
                        ),
                    ),
                ]),
                NamedShape::Enum(shape) => {
                    let docs = crate::typedocs::find(&item.rust);
                    Json::object([
                        ("is", Json::str("enum")),
                        (
                            "values",
                            Json::Array(
                                shape
                                    .variants
                                    .iter()
                                    .map(|variant| {
                                        let form = match &variant.form {
                                            VariantForm::Unit | VariantForm::Unknown => {
                                                Json::object([("kind", Json::str("unit"))])
                                            }
                                            VariantForm::Newtype(inner) => Json::object([
                                                ("kind", Json::str("value")),
                                                ("of", self::shape(inner)),
                                            ]),
                                            VariantForm::Tuple(items) => Json::object([
                                                ("kind", Json::str("tuple")),
                                                (
                                                    "of",
                                                    Json::Array(
                                                        items.iter().map(self::shape).collect(),
                                                    ),
                                                ),
                                            ]),
                                            VariantForm::Struct(key) => Json::object([
                                                ("kind", Json::str("struct")),
                                                ("key", Json::str(key)),
                                            ]),
                                        };
                                        Json::object([
                                            ("name", Json::str(variant.name)),
                                            ("form", form),
                                            (
                                                "doc",
                                                Json::str(
                                                    docs.and_then(|docs| docs.item(variant.name))
                                                        .unwrap_or(""),
                                                ),
                                            ),
                                        ])
                                    })
                                    .collect(),
                            ),
                        ),
                    ])
                }
                NamedShape::Kinds(shape) => Json::object([
                    ("is", Json::str("kinds")),
                    (
                        "via",
                        Json::str(match shape.via {
                            Via::ByKind => "by_kind",
                            Via::BySite => "by_site",
                            Via::Any => "any",
                        }),
                    ),
                    (
                        "accepted",
                        Json::Array(
                            shape
                                .accepted
                                .iter()
                                .map(|(kind, inner)| {
                                    Json::object([
                                        ("kind", kind_name(*kind)),
                                        ("shape", self::shape(inner)),
                                    ])
                                })
                                .collect(),
                        ),
                    ),
                    (
                        "expected",
                        shape.expected.as_ref().map_or(Json::Null, Json::str),
                    ),
                ]),
            };
            named.push((
                key.clone(),
                Json::object([
                    ("name", Json::str(&item.name)),
                    ("doc", Json::str(named_doc(item).unwrap_or(""))),
                    ("shape", body),
                ]),
            ));
        }
        types.push(Json::object([
            ("name", Json::str(entry.name)),
            ("doc", Json::str(entry.doc)),
            (
                "experimental",
                entry.experimental.map_or(Json::Null, Json::str),
            ),
            (
                "builtins",
                Json::Array(entry.builtins.iter().map(|name| Json::str(*name)).collect()),
            ),
            ("root", self::shape(&entry.shapes.root)),
            ("named", Json::Object(named)),
        ]));
    }
    let mut kinds: Vec<_> = registry.kinds().collect();
    kinds.sort_by(|a, b| a.spec.name.cmp(&b.spec.name));
    let kinds = Json::Array(
        kinds
            .iter()
            .map(|kind| {
                let mut properties: Vec<_> = registry.properties_of(kind.id).collect();
                properties.sort_by(|a, b| a.spec.name.cmp(&b.spec.name));
                Json::object([
                    ("name", Json::str(&kind.spec.name)),
                    ("doc", Json::str(&kind.spec.doc)),
                    (
                        "properties",
                        Json::Array(
                            properties
                                .iter()
                                .map(|property| {
                                    Json::object([
                                        ("name", Json::str(&property.spec.name)),
                                        ("args", params(registry, &property.spec.params)),
                                        ("type", Json::str(registry.type_name(&property.spec.ty))),
                                        ("belief_safe", Json::Bool(property.spec.belief_safe)),
                                        ("settable", Json::Bool(property.spec.settable)),
                                        (
                                            "changed_by",
                                            Json::Array(
                                                property
                                                    .spec
                                                    .changed_by
                                                    .iter()
                                                    .map(Json::str)
                                                    .collect(),
                                            ),
                                        ),
                                        ("doc", Json::str(&property.spec.doc)),
                                    ])
                                })
                                .collect(),
                        ),
                    ),
                ])
            })
            .collect(),
    );
    let mut functions: Vec<_> = registry.functions().collect();
    functions.sort_by(|a, b| a.spec.name.cmp(&b.spec.name));
    let functions = Json::Array(
        functions
            .iter()
            .map(|function| {
                Json::object([
                    ("name", Json::str(&function.spec.name)),
                    ("group", Json::str(&function.spec.group)),
                    ("args", params(registry, &function.spec.params)),
                    ("variadic", Json::Bool(function.spec.variadic)),
                    (
                        "returns",
                        Json::str(registry.type_name(&function.spec.returns)),
                    ),
                    ("belief_safe", Json::Bool(function.spec.belief_safe)),
                    ("random", Json::Bool(function.spec.random)),
                    ("doc", Json::str(&function.spec.doc)),
                ])
            })
            .collect(),
    );
    let mut events: Vec<_> = registry.events().collect();
    events.sort_by(|a, b| a.spec.name.cmp(&b.spec.name));
    let events = Json::Array(
        events
            .iter()
            .map(|event| {
                Json::object([
                    ("name", Json::str(&event.spec.name)),
                    ("hook", Json::Bool(event.spec.hook)),
                    (
                        "subjects",
                        Json::Array(
                            event
                                .spec
                                .subjects
                                .iter()
                                .map(|(name, kind)| {
                                    Json::object([
                                        ("name", Json::str(name)),
                                        (
                                            "kind",
                                            Json::str(
                                                registry
                                                    .kind(*kind)
                                                    .map_or("", |kind| kind.spec.name.as_str()),
                                            ),
                                        ),
                                    ])
                                })
                                .collect(),
                        ),
                    ),
                    ("fields", params(registry, &event.spec.fields)),
                    (
                        "mutable",
                        Json::Array(event.spec.mutable.iter().map(Json::str).collect()),
                    ),
                    ("doc", Json::str(&event.spec.doc)),
                ])
            })
            .collect(),
    );
    let mut verbs: Vec<_> = registry.verbs().collect();
    verbs.sort_by(|a, b| a.spec.name.cmp(&b.spec.name));
    let verbs = Json::Array(
        verbs
            .iter()
            .map(|verb| {
                Json::object([
                    ("name", Json::str(&verb.spec.name)),
                    ("args", params(registry, &verb.spec.args)),
                    (
                        "direct",
                        verb.spec.direct.as_ref().map_or(Json::Null, Json::str),
                    ),
                    ("doc", Json::str(&verb.spec.doc)),
                ])
            })
            .collect(),
    );
    let mut queries: Vec<_> = registry.queries().collect();
    queries.sort_by(|a, b| a.spec.name.cmp(&b.spec.name));
    let queries = Json::Array(
        queries
            .iter()
            .map(|query| {
                Json::object([
                    ("name", Json::str(&query.spec.name)),
                    ("args", params(registry, &query.spec.args)),
                    (
                        "element",
                        Json::str(registry.type_name(&query.spec.element)),
                    ),
                    ("belief_safe", Json::Bool(query.spec.belief_safe)),
                    ("doc", Json::str(&query.spec.doc)),
                ])
            })
            .collect(),
    );
    let effects = Json::Array(
        catalog
            .effects
            .iter()
            .map(|effect| {
                Json::object([
                    ("name", Json::str(effect.info.name)),
                    ("form", Json::str(effect.info.form)),
                    ("action_only", Json::Bool(effect.action_only)),
                    ("doc", Json::str(effect.info.doc)),
                ])
            })
            .collect(),
    );
    let settings = Json::Array(
        catalog
            .settings
            .iter()
            .map(|setting| {
                Json::object([
                    ("name", Json::str(setting.name)),
                    (
                        "default",
                        match &setting.default {
                            clash_data::expr::Value::Bool(value) => Json::Bool(*value),
                            clash_data::expr::Value::Int(value) => Json::Int(*value),
                            clash_data::expr::Value::Real(value) => Json::Real(value.get()),
                            other => Json::str(other.to_string()),
                        },
                    ),
                    ("doc", Json::str(setting.doc)),
                ])
            })
            .collect(),
    );
    let reasons = Json::Array(
        catalog
            .reasons
            .iter()
            .map(|reason| {
                Json::object([
                    ("code", Json::str(reason.code)),
                    ("doc", Json::str(reason.doc)),
                    ("text", reason.text.map_or(Json::Null, Json::str)),
                ])
            })
            .chain(catalog.host_reasons.iter().map(|code| {
                Json::object([
                    ("code", Json::str(code.code)),
                    ("doc", Json::str(code.doc)),
                    ("text", Json::Null),
                ])
            }))
            .collect(),
    );
    let codes = Json::Array(
        catalog
            .codes
            .iter()
            .map(|info| {
                Json::object([
                    ("code", Json::str(info.code.id)),
                    ("stage", Json::str(info.stage)),
                    ("title", Json::str(info.title)),
                    ("explanation", Json::str(info.explanation)),
                ])
            })
            .chain(
                catalog
                    .expression_codes
                    .iter()
                    .chain(&catalog.console_codes)
                    .map(|code| {
                        Json::object([
                            ("code", Json::str(code.code)),
                            ("stage", Json::Null),
                            ("title", Json::str(code.doc)),
                            ("explanation", Json::Null),
                        ])
                    }),
            )
            .collect(),
    );
    Json::object([
        ("types", Json::Array(types)),
        ("kinds", kinds),
        ("functions", functions),
        ("events", events),
        ("verbs", verbs),
        ("effects", effects),
        ("queries", queries),
        ("settings", settings),
        ("reasons", reasons),
        ("codes", codes),
    ])
    .pretty()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn json_is_written_indented_and_escaped() {
        let value = Json::object([
            (
                "a",
                Json::Array(vec![Json::Int(1), Json::Real(2.5), Json::Real(3.0)]),
            ),
            ("b", Json::str("say \"hi\"\n")),
            ("c", Json::Object(Vec::new())),
        ]);
        assert_eq!(
            value.pretty(),
            "{\n  \"a\": [\n    1,\n    2.5,\n    3.0\n  ],\n  \"b\": \"say \\\"hi\\\"\\n\",\n  \"c\": {}\n}\n"
        );
    }
}
