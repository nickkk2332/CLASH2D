//! JSON Schemas of the definition types and of a data file (04 §13),
//! generated from what each type reads ([`clash_data::shape`]), so that
//! an editor checks keys, values, units and, with `--with-ids`, the ids
//! of references as they are typed.
//!
//! The schemas follow JSON Schema 2020-12 and use a small part of it:
//! `type`, `enum`, `const`, `properties`, `additionalProperties`,
//! `required`, `items`, `prefixItems`, `minItems`, `maxItems`,
//! `minimum`, `pattern`, `propertyNames`, `anyOf`, `allOf`,
//! `if`/`then`/`else`, `not` and `$ref` into `$defs`. The tests check
//! the bundled mods against them with a validator of that part
//! ([`validate`]).
//!
//! What a schema cannot know is left to the loader: whole-set checks
//! (duplicate ids, inheritance cycles), expressions, and the world's and
//! the interface's references, which are plain strings resolved when
//! their tables are built (04 §7.1). A definition that patches, copies
//! from another or is abstract need not give every required field, and
//! is checked against a variant of its type with nothing required.

#[cfg(test)]
mod regex;
#[cfg(test)]
mod tests;
#[cfg(test)]
pub(crate) mod validate;

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use clash_data::DefTypes;
use clash_data::shape::{JsonKind, Named, NamedShape, Shape, VariantForm};
use clash_data::units::{Base, Dim, RATE_UNITS, UNITS};

use crate::data::{EXIT_FAILED, EXIT_OK, Options, Output};
use crate::reference::json::Json;
use crate::reference::{TypeEntry, data_layer, field_doc, named_doc, type_entries};

/// The JSON Schema dialect the schemas are written in.
pub const DIALECT: &str = "https://json-schema.org/draft/2020-12/schema";

/// The file name of the schema of a data file.
pub const DATA_SCHEMA: &str = "data.schema.json";

/// The ids that may be named, by definition type: every qualified id,
/// and the bare name of each, for `--with-ids`.
pub type Ids = BTreeMap<String, Vec<String>>;

/// The ids of loaded definitions, by type: each qualified id and its
/// bare name.
pub fn ids_of(defs: &clash_data::Defs) -> Ids {
    let mut ids = Ids::new();
    for name in defs.type_names() {
        let Some(qualified) = defs.ids_of(name) else {
            continue;
        };
        let mut names: BTreeSet<String> = BTreeSet::new();
        for id in qualified {
            if id.contains('#') {
                // Written in place: it cannot be named.
                continue;
            }
            names.insert(id.clone());
            if let Some((_, bare)) = id.split_once(':') {
                names.insert(bare.to_owned());
            }
        }
        ids.insert(name.to_owned(), names.into_iter().collect());
    }
    ids
}

/// `schema [--type <type>] [--out <dir>] [--with-ids]`.
pub(crate) fn command(types: &DefTypes, options: &Options) -> Output {
    if !options.positional.is_empty() {
        return Output::usage("`schema` takes no arguments besides its options");
    }
    let entries = type_entries(types);
    let ids = if options.with_ids {
        let outcome = match &options.profile {
            Some(profile) => crate::data::load_profile(types, options, profile),
            None => {
                let mut loader = crate::data::loader(types, options);
                if options.mods.is_empty() {
                    loader = loader.search_path(crate::data::DEFAULT_MODS_DIR);
                }
                loader.load_all()
            }
        };
        match &outcome.defs {
            Some(defs) => Some(ids_of(defs)),
            None => {
                return Output {
                    out: outcome.diagnostics.render(),
                    err: "clash-tools: the mods did not load, so their ids cannot be listed\n"
                        .to_owned(),
                    code: EXIT_FAILED,
                };
            }
        }
    } else {
        None
    };
    let schemas = Schemas::new(&entries, ids.as_ref());
    if let Some(name) = &options.type_name {
        let Some(entry) = entries.iter().find(|entry| entry.name == name.as_str()) else {
            let mut err = format!("clash-tools: there is no definition type `{name}`\n");
            if let Some(near) =
                clash_data::diag::closest(name, entries.iter().map(|entry| entry.name))
            {
                err.push_str(&format!("did you mean `{near}`?\n"));
            }
            return Output {
                out: String::new(),
                err,
                code: EXIT_FAILED,
            };
        };
        let schema = schemas.type_schema(entry).pretty();
        return match &options.out {
            Some(dir) => write_files(dir, &[(format!("{}.schema.json", entry.name), schema)]),
            None => Output {
                out: schema,
                err: String::new(),
                code: EXIT_OK,
            },
        };
    }
    match &options.out {
        None => Output {
            out: schemas.data_schema().pretty(),
            err: String::new(),
            code: EXIT_OK,
        },
        Some(dir) => {
            let mut files = vec![(DATA_SCHEMA.to_owned(), schemas.data_schema().pretty())];
            for entry in &entries {
                files.push((
                    format!("{}.schema.json", entry.name),
                    schemas.type_schema(entry).pretty(),
                ));
            }
            write_files(dir, &files)
        }
    }
}

fn write_files(dir: &Path, files: &[(String, String)]) -> Output {
    if let Err(error) = std::fs::create_dir_all(dir) {
        return Output {
            out: String::new(),
            err: format!("clash-tools: cannot make {}: {error}\n", dir.display()),
            code: EXIT_FAILED,
        };
    }
    for (name, text) in files {
        if let Err(error) = std::fs::write(dir.join(name), text) {
            return Output {
                out: String::new(),
                err: format!("clash-tools: cannot write {name}: {error}\n"),
                code: EXIT_FAILED,
            };
        }
    }
    Output {
        out: format!("wrote {} schemas to {}\n", files.len(), dir.display()),
        err: String::new(),
        code: EXIT_OK,
    }
}

/// The keys every definition carries (04 §5), with their schemas.
fn directive_properties() -> Vec<(String, Json)> {
    let string = || Json::object([("type", Json::str("string"))]);
    let boolean = || Json::object([("type", Json::str("boolean"))]);
    let object = || Json::object([("type", Json::str("object"))]);
    let mods = || {
        Json::object([(
            "anyOf",
            Json::Array(vec![
                string(),
                Json::object([("type", Json::str("array")), ("items", string())]),
            ]),
        )])
    };
    let mut out = Vec::new();
    for (key, doc) in clash_data::DIRECTIVE_DOCS {
        let schema = match *key {
            "type" | "id" | "copy_from" => string(),
            "//" => Json::object(Vec::<(String, Json)>::new()),
            "abstract" | "obsolete" | "patch" | "override" | "optional" => boolean(),
            "if_mod" | "unless_mod" => mods(),
            _ => object(),
        };
        out.push(((*key).to_owned(), with_description(schema, doc)));
    }
    out
}

fn with_description(schema: Json, doc: &str) -> Json {
    match schema {
        Json::Object(mut members) if !doc.is_empty() => {
            members.push((
                "description".to_owned(),
                Json::str(crate::typedocs::clean(doc)),
            ));
            Json::Object(members)
        }
        other => other,
    }
}

/// A pattern matching a string quantity of a dimension (04 §7.1): a
/// number and a unit of the dimension, as the loader reads one.
pub fn quantity_pattern(dim: Dim) -> String {
    const NUMBER: &str = r"[+-]?([0-9]+(\.[0-9]+)?|\.[0-9]+)([eE][+-]?[0-9]+)?";
    let escape = |unit: &str| unit.replace('/', "\\/");
    let mut plain: Vec<String> = UNITS
        .iter()
        .filter(|unit| unit.dim == dim)
        .map(|unit| escape(unit.symbol))
        .collect();
    let mut tail = String::new();
    if dim.per_time {
        let per: Vec<&str> = RATE_UNITS.iter().map(|(unit, _)| *unit).collect();
        let base: Vec<String> = UNITS
            .iter()
            .filter(|unit| unit.dim == Dim::of(dim.base))
            .map(|unit| escape(unit.symbol))
            .collect();
        let before = if dim.base == Base::Count {
            String::new()
        } else {
            format!("({})", base.join("|"))
        };
        tail = format!("{before}\\s*\\/\\s*({})", per.join("|"));
    }
    plain.sort();
    let mut alternatives = Vec::new();
    if !plain.is_empty() {
        alternatives.push(format!("({})", plain.join("|")));
    }
    if !tail.is_empty() {
        alternatives.push(tail);
    }
    format!("^\\s*{NUMBER}\\s*({})\\s*$", alternatives.join("|"))
}

/// The schemas of a set of definition types.
pub struct Schemas<'a> {
    entries: &'a [TypeEntry],
    ids: Option<&'a Ids>,
    /// Every named shape any type reaches, by key.
    named: BTreeMap<&'a str, &'a Named>,
    /// The name each named shape has in `$defs`.
    def_names: BTreeMap<&'a str, String>,
}

impl<'a> Schemas<'a> {
    /// The schemas of `entries`; with `ids`, references list the ids
    /// they may name.
    pub fn new(entries: &'a [TypeEntry], ids: Option<&'a Ids>) -> Schemas<'a> {
        let mut named: BTreeMap<&str, &Named> = BTreeMap::new();
        for entry in entries {
            for (key, item) in &entry.shapes.named {
                named.entry(key.as_str()).or_insert(item);
            }
        }
        let mut def_names = BTreeMap::new();
        let mut used = BTreeSet::new();
        for (key, item) in &named {
            let base: String = item
                .name
                .chars()
                .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
                .collect();
            let mut name = base.clone();
            let mut n = 2;
            while !used.insert(name.clone()) {
                name = format!("{base}_{n}");
                n += 1;
            }
            def_names.insert(*key, name);
        }
        Schemas {
            entries,
            ids,
            named,
            def_names,
        }
    }

    fn def_ref(&self, key: &str, partial: bool) -> Json {
        let name = self.def_names.get(key).cloned().unwrap_or_default();
        let suffix = if partial { ".partial" } else { "" };
        Json::object([("$ref", Json::str(format!("#/$defs/{name}{suffix}")))])
    }

    fn ids_enum(&self, ty: &str) -> Option<Json> {
        let ids = self.ids?.get(ty)?;
        Some(Json::Array(ids.iter().map(Json::str).collect()))
    }

    /// The schema of a value of a shape.
    fn shape(&self, shape: &Shape, partial: bool) -> Json {
        let ty = |name: &str| ("type", Json::str(name));
        match shape {
            Shape::Unknown | Shape::Any => Json::Object(Vec::new()),
            Shape::Bool => Json::object([ty("boolean")]),
            Shape::Int { signed } => {
                if *signed {
                    Json::object([ty("integer")])
                } else {
                    Json::object([ty("integer"), ("minimum", Json::Int(0))])
                }
            }
            Shape::Real => Json::object([ty("number")]),
            Shape::Char | Shape::String | Shape::Tag => Json::object([ty("string")]),
            Shape::Quantity(dim) => quantity(*dim),
            Shape::Duration => quantity(Dim::TIME),
            Shape::Ref(def) => match self.ids_enum(def) {
                Some(ids) => Json::object([ty("string"), ("enum", ids)]),
                None => Json::object([ty("string")]),
            },
            Shape::Inline(def) => {
                let by_id = match self.ids_enum(def) {
                    Some(ids) => Json::object([ty("string"), ("enum", ids)]),
                    None => Json::object([ty("string")]),
                };
                Json::object([(
                    "anyOf",
                    Json::Array(vec![by_id, Json::object([ty("object")])]),
                )])
            }
            Shape::List(inner) => {
                Json::object([ty("array"), ("items", self.shape(inner, partial))])
            }
            Shape::Tuple(items) => {
                let count = i64::try_from(items.len()).unwrap_or(i64::MAX);
                Json::object([
                    ty("array"),
                    (
                        "prefixItems",
                        Json::Array(items.iter().map(|item| self.shape(item, partial)).collect()),
                    ),
                    ("minItems", Json::Int(count)),
                    ("maxItems", Json::Int(count)),
                ])
            }
            Shape::Map(key, value) => {
                let mut members = vec![
                    ("type".to_owned(), Json::str("object")),
                    (
                        "properties".to_owned(),
                        Json::object([("//", Json::Object(Vec::new()))]),
                    ),
                    (
                        "additionalProperties".to_owned(),
                        self.shape(value, partial),
                    ),
                ];
                if let Shape::Ref(def) = key.as_ref()
                    && let Some(Json::Array(mut ids)) = self.ids_enum(def)
                {
                    ids.push(Json::str("//"));
                    members.push((
                        "propertyNames".to_owned(),
                        Json::object([("enum", Json::Array(ids))]),
                    ));
                }
                Json::Object(members)
            }
            Shape::Optional(inner) => Json::object([(
                "anyOf",
                Json::Array(vec![self.shape(inner, partial), Json::object([ty("null")])]),
            )]),
            Shape::Gated(feature, inner) => with_description(
                self.shape(inner, partial),
                &format!("Experimental feature `{feature}` (04 §8.4)."),
            ),
            Shape::Unit => Json::object([ty("null")]),
            Shape::Named(key) => match self.named.get(key.as_str()) {
                Some(_) => self.def_ref(key, partial),
                None => Json::Object(Vec::new()),
            },
        }
    }

    /// The schema of a named shape, strict or partial.
    fn named_schema(&self, item: &Named, partial: bool) -> Json {
        let description = named_doc(item)
            .map(crate::typedocs::clean)
            .or_else(|| data_layer(&item.rust).map(|(_, name)| format!("A {name} (04 §7.1).")))
            .unwrap_or_default();
        let schema = match &item.shape {
            NamedShape::Struct(shape) => {
                let mut properties: Vec<(String, Json)> = Vec::new();
                let mut required = Vec::new();
                for field in &shape.fields {
                    let schema = self.shape(&field.shape, partial);
                    let doc = field_doc(item, field.name).unwrap_or("");
                    properties.push((field.name.to_owned(), with_description(schema, doc)));
                    if field.required && !partial {
                        required.push(Json::str(field.name));
                    }
                }
                properties.push(("//".to_owned(), Json::Object(Vec::new())));
                if partial && shape.fields.iter().any(|field| field.name == "id") {
                    // An entry of a list merged by id may be moved.
                    properties.push((
                        "before".to_owned(),
                        Json::object([("type", Json::str("string"))]),
                    ));
                    properties.push((
                        "after".to_owned(),
                        Json::object([("type", Json::str("string"))]),
                    ));
                }
                let mut members = vec![
                    ("type".to_owned(), Json::str("object")),
                    ("properties".to_owned(), Json::Object(properties)),
                    ("additionalProperties".to_owned(), Json::Bool(false)),
                ];
                if !required.is_empty() {
                    members.push(("required".to_owned(), Json::Array(required)));
                }
                Json::Object(members)
            }
            NamedShape::Enum(shape) => {
                let all_unit = shape.variants.iter().all(|variant| {
                    matches!(variant.form, VariantForm::Unit | VariantForm::Unknown)
                });
                if all_unit {
                    Json::object([(
                        "enum",
                        Json::Array(
                            shape
                                .variants
                                .iter()
                                .map(|variant| Json::str(variant.name))
                                .collect(),
                        ),
                    )])
                } else {
                    let branches = shape
                        .variants
                        .iter()
                        .map(|variant| {
                            let one = |value: Json| {
                                Json::object([
                                    ("type", Json::str("object")),
                                    ("properties", Json::object([(variant.name, value)])),
                                    ("required", Json::Array(vec![Json::str(variant.name)])),
                                    ("additionalProperties", Json::Bool(false)),
                                ])
                            };
                            match &variant.form {
                                VariantForm::Unit | VariantForm::Unknown => {
                                    Json::object([("const", Json::str(variant.name))])
                                }
                                VariantForm::Newtype(inner) => one(self.shape(inner, partial)),
                                VariantForm::Tuple(items) => {
                                    one(self.shape(&Shape::Tuple(items.clone()), partial))
                                }
                                VariantForm::Struct(key) => one(self.def_ref(key, partial)),
                            }
                        })
                        .collect();
                    Json::object([("anyOf", Json::Array(branches))])
                }
            }
            NamedShape::Kinds(shape) => {
                if shape.accepted.is_empty() {
                    Json::Object(Vec::new())
                } else {
                    let mut branches = Vec::new();
                    let has_number = shape
                        .accepted
                        .iter()
                        .any(|(kind, _)| *kind == JsonKind::Number);
                    for (kind, inner) in &shape.accepted {
                        if *kind == JsonKind::Integer && has_number {
                            continue;
                        }
                        branches.push(self.kind_schema(*kind, inner, partial));
                    }
                    if branches.len() == 1 {
                        branches.remove(0)
                    } else {
                        Json::object([("anyOf", Json::Array(branches))])
                    }
                }
            }
        };
        with_description(schema, &description)
    }

    /// The schema of one kind of JSON a field of several kinds takes,
    /// read as `inner`.
    fn kind_schema(&self, kind: JsonKind, inner: &Shape, partial: bool) -> Json {
        let plain = |name: &str| Json::object([("type", Json::str(name))]);
        let natural = match kind {
            JsonKind::String => "string",
            JsonKind::Object => "object",
            JsonKind::Integer => "integer",
            JsonKind::Number => "number",
            JsonKind::Bool => "boolean",
            JsonKind::List => "array",
            JsonKind::Null => "null",
        };
        let detailed = match (kind, inner) {
            (_, Shape::Unknown | Shape::Any) => None,
            (JsonKind::String, Shape::Ref(_) | Shape::Inline(_) | Shape::Tag | Shape::String) => {
                Some(self.shape(inner, partial))
            }
            (JsonKind::String, Shape::Quantity(dim)) => Some(Json::object([
                ("type", Json::str("string")),
                ("pattern", Json::str(quantity_pattern(*dim))),
            ])),
            (JsonKind::List, Shape::List(_) | Shape::Tuple(_))
            | (JsonKind::Object, Shape::Map(..)) => Some(self.shape(inner, partial)),
            (JsonKind::Object | JsonKind::String | JsonKind::List, Shape::Named(key)) => {
                // Only where the named shape is of this kind of JSON.
                match self.named.get(key.as_str()).map(|item| &item.shape) {
                    Some(NamedShape::Struct(_)) if kind == JsonKind::Object => {
                        Some(self.def_ref(key, partial))
                    }
                    Some(NamedShape::Enum(_) | NamedShape::Kinds(_)) => Some(Json::object([(
                        "allOf",
                        Json::Array(vec![plain(natural), self.def_ref(key, partial)]),
                    )])),
                    _ => None,
                }
            }
            _ => None,
        };
        detailed.unwrap_or_else(|| plain(natural))
    }

    /// The `$defs` every schema of these types may refer to: every named
    /// shape, strict and partial, and every definition type's.
    fn defs(&self, only: Option<&TypeEntry>) -> Vec<(String, Json)> {
        let mut defs = Vec::new();
        let keys: Vec<&str> = match only {
            Some(entry) => entry.shapes.named.keys().map(String::as_str).collect(),
            None => self.named.keys().copied().collect(),
        };
        for key in keys {
            let Some(item) = self.named.get(key) else {
                continue;
            };
            let name = self.def_names.get(key).cloned().unwrap_or_default();
            defs.push((name.clone(), self.named_schema(item, false)));
            defs.push((format!("{name}.partial"), self.named_schema(item, true)));
        }
        defs
    }

    /// The schema of one definition of a type, strict or partial: the
    /// keys every definition carries and the type's own fields.
    fn definition(&self, entry: &TypeEntry, partial: bool) -> Json {
        let mut properties = directive_properties();
        let mut required = vec![Json::str("type"), Json::str("id")];
        if let Some(index) = properties.iter().position(|(key, _)| key == "type") {
            properties[index].1 = Json::object([("const", Json::str(entry.name))]);
        }
        let root = match &entry.shapes.root {
            Shape::Named(key) => self.named.get(key.as_str()).copied(),
            _ => None,
        };
        // A type whose fields are its own: each is a property. A type
        // read as a map, as a keymap is, takes any other key with the
        // map's values; anything else is left to the loader.
        let others = match (root, &entry.shapes.root) {
            (Some(root), _) => match &root.shape {
                NamedShape::Struct(shape) => {
                    for field in &shape.fields {
                        let schema = self.shape(&field.shape, partial);
                        let doc = field_doc(root, field.name).unwrap_or("");
                        properties.push((field.name.to_owned(), with_description(schema, doc)));
                        if field.required && !partial {
                            required.push(Json::str(field.name));
                        }
                    }
                    Json::Bool(false)
                }
                _ => Json::Object(Vec::new()),
            },
            (None, Shape::Map(_, value)) => self.shape(value, partial),
            (None, _) => Json::Object(Vec::new()),
        };
        Json::object([
            ("type", Json::str("object")),
            ("description", Json::str(crate::typedocs::clean(entry.doc))),
            ("properties", Json::Object(properties)),
            ("additionalProperties", others),
            ("required", Json::Array(required)),
        ])
    }

    /// Whether a definition may leave out required fields: it patches,
    /// copies from another or is abstract.
    fn partial_test() -> Json {
        let present = |key: &str| Json::object([("required", Json::Array(vec![Json::str(key)]))]);
        Json::object([(
            "anyOf",
            Json::Array(vec![
                present("patch"),
                present("copy_from"),
                present("abstract"),
            ]),
        )])
    }

    /// A definition of one type, strict unless it patches, copies or is
    /// abstract.
    fn either(&self, entry: &TypeEntry) -> Json {
        Json::object([
            ("if", Schemas::partial_test()),
            (
                "then",
                Json::object([(
                    "$ref",
                    Json::str(format!("#/$defs/type.{}.partial", entry.name)),
                )]),
            ),
            (
                "else",
                Json::object([("$ref", Json::str(format!("#/$defs/type.{}", entry.name)))]),
            ),
        ])
    }

    /// The schema of one definition type, standing alone.
    pub fn type_schema(&self, entry: &TypeEntry) -> Json {
        let mut defs = vec![
            (
                format!("type.{}", entry.name),
                self.definition(entry, false),
            ),
            (
                format!("type.{}.partial", entry.name),
                self.definition(entry, true),
            ),
        ];
        defs.extend(self.defs(Some(entry)));
        let mut members = vec![
            ("$schema".to_owned(), Json::str(DIALECT)),
            (
                "title".to_owned(),
                Json::str(format!("CLASH2D definition: {}", entry.name)),
            ),
        ];
        if let Json::Object(either) = self.either(entry) {
            members.extend(either);
        }
        members.push(("$defs".to_owned(), Json::Object(defs)));
        Json::Object(members)
    }

    /// The schema of a data file: a definition of any type, or a list of
    /// them, each checked against its type by `type`.
    pub fn data_schema(&self) -> Json {
        let names: Vec<Json> = self
            .entries
            .iter()
            .map(|entry| Json::str(entry.name))
            .collect();
        let mut by_type = Vec::new();
        let mut defs = Vec::new();
        for entry in self.entries {
            by_type.push(Json::object([
                (
                    "if",
                    Json::object([
                        (
                            "properties",
                            Json::object([(
                                "type",
                                Json::object([("const", Json::str(entry.name))]),
                            )]),
                        ),
                        ("required", Json::Array(vec![Json::str("type")])),
                    ]),
                ),
                ("then", self.either(entry)),
            ]));
            defs.push((
                format!("type.{}", entry.name),
                self.definition(entry, false),
            ));
            defs.push((
                format!("type.{}.partial", entry.name),
                self.definition(entry, true),
            ));
        }
        let definition = Json::object([
            ("type", Json::str("object")),
            (
                "properties",
                Json::object([("type", Json::object([("enum", Json::Array(names))]))]),
            ),
            (
                "required",
                Json::Array(vec![Json::str("type"), Json::str("id")]),
            ),
            ("allOf", Json::Array(by_type)),
        ]);
        defs.push(("definition".to_owned(), definition));
        defs.extend(self.defs(None));
        Json::object([
            ("$schema", Json::str(DIALECT)),
            ("title", Json::str("CLASH2D data file")),
            (
                "description",
                Json::str(
                    "A JSON array of definitions, or a single one (04 §5). Generated by `clash-tools schema` from the engine's definition types.",
                ),
            ),
            (
                "anyOf",
                Json::Array(vec![
                    Json::object([
                        ("type", Json::str("array")),
                        (
                            "items",
                            Json::object([("$ref", Json::str("#/$defs/definition"))]),
                        ),
                    ]),
                    Json::object([("$ref", Json::str("#/$defs/definition"))]),
                ]),
            ),
            ("$defs", Json::Object(defs)),
        ])
    }
}

/// The schema of a quantity: a number in SI units, or a string with a
/// unit of its dimension.
fn quantity(dim: Dim) -> Json {
    Json::object([
        (
            "anyOf",
            Json::Array(vec![
                Json::object([("type", Json::str("number"))]),
                Json::object([
                    ("type", Json::str("string")),
                    ("pattern", Json::str(quantity_pattern(dim))),
                ]),
            ]),
        ),
        (
            "description",
            Json::str(format!(
                "{}: a number in `{}`, or a string with a unit: {}",
                dim.name(),
                dim.si_unit(),
                dim.accepted_units()
            )),
        ),
    ])
}
