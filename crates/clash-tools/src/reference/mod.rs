//! The modding reference (04 §13, 14 §9.2): everything a mod can name,
//! gathered from the registries of a loaded engine.
//!
//! [`Catalog::gather`] loads no mods with the engine's own definition
//! types and registry, which runs every crate's registry step, and
//! collects what the engine registered: each definition type with the
//! shape of every field it reads ([`clash_data::shape`]); the kinds of
//! subject, properties, functions, events and hooks, verbs and queries
//! of the expression registry; the effects of the rule system; the
//! settings and reasons of the simulation; the units; and every
//! diagnostic code. Nothing is listed by hand: an item a later package
//! registers appears with no change here. [`markdown`] and [`json`]
//! write it out; [`Catalog::undocumented`] says what is registered with
//! no documentation.

pub mod json;
pub mod markdown;

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use clash_data::expr::ExprRegistry;
use clash_data::shape::{NamedShape, Shape, Shapes};
use clash_data::{CodeInfo, DefTypes};
use clash_sim::rule::EffectInfo;
use clash_sim::{ReasonDoc, SettingInfo};

use crate::data::{EXIT_FAILED, EXIT_OK, Options, Output};
use crate::typedocs;

/// `docs [--out <dir>] [--format md|json]`: writes the reference. With
/// no `--out`, the pages go to standard output, each after a line that
/// names it. What has no documentation is reported on standard error.
pub(crate) fn docs_command(types: &DefTypes, options: &Options) -> Output {
    if !options.positional.is_empty() {
        return Output::usage("`docs` takes no arguments besides its options");
    }
    let catalog = match Catalog::gather(types) {
        Ok(catalog) => catalog,
        Err(diagnostics) => {
            return Output {
                out: diagnostics,
                err: "clash-tools: the engine's own definitions do not load\n".to_owned(),
                code: EXIT_FAILED,
            };
        }
    };
    let pages: BTreeMap<String, String> = match options.format.as_deref() {
        Some("json") => [("reference.json".to_owned(), json::document(&catalog))]
            .into_iter()
            .collect(),
        _ => markdown::pages(&catalog),
    };
    let mut err = String::new();
    let missing = catalog.undocumented();
    if !missing.is_empty() {
        err.push_str(&format!(
            "clash-tools: {} registered items have no documentation:\n",
            missing.len()
        ));
        for line in &missing {
            err.push_str(&format!("  {line}\n"));
        }
    }
    let mut out = String::new();
    match &options.out {
        Some(dir) => {
            if let Err(error) = write_pages(dir, &pages) {
                return Output {
                    out,
                    err: format!("{err}clash-tools: cannot write the reference: {error}\n"),
                    code: EXIT_FAILED,
                };
            }
            let counts: Vec<String> = catalog
                .counts()
                .iter()
                .map(|(what, count)| format!("{count} {what}"))
                .collect();
            out.push_str(&format!(
                "wrote {} pages to {}: {}\n",
                pages.len(),
                dir.display(),
                counts.join(", ")
            ));
        }
        None => {
            for (path, page) in &pages {
                if pages.len() > 1 {
                    out.push_str(&format!("<!-- page: {path} -->\n"));
                }
                out.push_str(page);
            }
        }
    }
    Output {
        out,
        err,
        code: EXIT_OK,
    }
}

/// Writes the pages under `dir`, and removes the Markdown pages there
/// that are no longer written, so that a type that is gone leaves no
/// page behind.
pub fn write_pages(dir: &Path, pages: &BTreeMap<String, String>) -> std::io::Result<()> {
    for path in existing_pages(dir) {
        if !pages.contains_key(&path) {
            std::fs::remove_file(dir.join(&path))?;
        }
    }
    for (path, page) in pages {
        let target = dir.join(path);
        if let Some(parent) = target.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(target, page)?;
    }
    Ok(())
}

/// The Markdown and JSON pages under `dir`, by their path below it, with
/// `/` between parts.
pub fn existing_pages(dir: &Path) -> Vec<String> {
    fn walk(root: &Path, dir: &Path, out: &mut Vec<String>) {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };
        for entry in entries.filter_map(Result::ok) {
            let path = entry.path();
            if path.is_dir() {
                walk(root, &path, out);
            } else if path
                .extension()
                .is_some_and(|ext| ext == "md" || ext == "json")
                && let Ok(relative) = path.strip_prefix(root)
            {
                let parts: Vec<String> = relative
                    .iter()
                    .map(|part| part.to_string_lossy().into_owned())
                    .collect();
                out.push(parts.join("/"));
            }
        }
    }
    let mut out = Vec::new();
    walk(dir, dir, &mut out);
    out.sort();
    out
}

/// A definition type and what it reads.
pub struct TypeEntry {
    /// The JSON name.
    pub name: &'static str,
    /// What it is.
    pub doc: &'static str,
    /// The experimental feature it is part of, if any.
    pub experimental: Option<&'static str>,
    /// The defaults `scale` and `add` act on, as a JSON object; empty
    /// for none.
    pub defaults: &'static str,
    /// The definitions the engine supplies itself, by name.
    pub builtins: Vec<&'static str>,
    /// What its fields are.
    pub shapes: Shapes,
}

/// Every registered definition type with what it reads, in order of
/// name.
pub fn type_entries(types: &DefTypes) -> Vec<TypeEntry> {
    let mut entries: Vec<TypeEntry> = types
        .infos()
        .iter()
        .filter_map(|info| type_entry(types, info.name))
        .collect();
    entries.sort_by(|a, b| a.name.cmp(b.name));
    entries
}

/// One registered definition type with what it reads, if there is one
/// of that name.
pub fn type_entry(types: &DefTypes, name: &str) -> Option<TypeEntry> {
    let infos = types.infos();
    let info = infos.iter().find(|info| info.name == name)?;
    Some(TypeEntry {
        name: info.name,
        doc: info.doc,
        experimental: info.experimental,
        defaults: types.defaults(info.name).unwrap_or(""),
        builtins: types.builtins(info.name).unwrap_or_default(),
        shapes: types.shapes(info.name)?,
    })
}

/// One effect of the rule system, and where it may stand.
pub struct EffectEntry {
    /// The effect.
    pub info: EffectInfo,
    /// Whether only an action's lists may hold it.
    pub action_only: bool,
}

/// One code of a table that gives a line for each.
pub struct ShortCode {
    /// The code.
    pub code: &'static str,
    /// What it means.
    pub doc: &'static str,
}

/// Everything the reference covers.
pub struct Catalog {
    /// Every definition type, by JSON name.
    pub types: Vec<TypeEntry>,
    /// The expression registry of a loaded engine.
    pub registry: ExprRegistry,
    /// The effects of the rule system.
    pub effects: Vec<EffectEntry>,
    /// The settings the engine reads.
    pub settings: Vec<SettingInfo>,
    /// The reasons the simulation gives.
    pub reasons: Vec<ReasonDoc>,
    /// The reasons the console's host gives.
    pub host_reasons: Vec<ShortCode>,
    /// The loader's diagnostic codes, with their explanations.
    pub codes: Vec<&'static CodeInfo>,
    /// The expression compiler's codes.
    pub expression_codes: Vec<ShortCode>,
    /// The console's and the test scripts' codes.
    pub console_codes: Vec<ShortCode>,
}

impl Catalog {
    /// Gathers the reference from the registries of an engine loaded
    /// with `types` and no mods. Fails, with the loader's diagnostics,
    /// only if the engine itself does not load.
    pub fn gather(types: &DefTypes) -> Result<Catalog, String> {
        let outcome = clash_scene::engine_loader(types).load_mods(&[]);
        let Some(defs) = outcome.defs else {
            return Err(outcome.diagnostics.render());
        };
        let registry = (**defs.expr_registry()).clone();
        let entries = type_entries(types);
        let mut effects: Vec<EffectEntry> = clash_sim::rule::CORE_EFFECTS
            .iter()
            .map(|info| EffectEntry {
                info: *info,
                action_only: false,
            })
            .chain(
                clash_sim::rule::ACTION_EFFECTS
                    .iter()
                    .map(|info| EffectEntry {
                        info: *info,
                        action_only: true,
                    }),
            )
            .collect();
        effects.sort_by(|a, b| a.info.name.cmp(b.info.name));
        let mut codes: Vec<&'static CodeInfo> = clash_data::CODES
            .iter()
            .chain(clash_data::expr::CODES)
            .collect();
        codes.sort_by(|a, b| a.code.id.cmp(b.code.id));
        let mut expression_codes: Vec<ShortCode> = clash_expr::codes::ALL
            .iter()
            .map(|(code, doc)| ShortCode {
                code: code.as_str(),
                doc,
            })
            .collect();
        expression_codes.sort_by(|a, b| a.code.cmp(b.code));
        let mut console_codes: Vec<ShortCode> = clash_sim::console::codes::ALL
            .iter()
            .map(|(code, doc)| ShortCode {
                code: code.as_str(),
                doc,
            })
            .collect();
        console_codes.sort_by(|a, b| a.code.cmp(b.code));
        let mut host_reasons: Vec<ShortCode> = clash_save::host::reasons::ALL
            .iter()
            .map(|(code, doc)| ShortCode { code, doc })
            .collect();
        host_reasons.sort_by(|a, b| a.code.cmp(b.code));
        Ok(Catalog {
            types: entries,
            registry,
            effects,
            settings: clash_sim::engine_settings(),
            reasons: clash_sim::engine_reasons(),
            host_reasons,
            codes,
            expression_codes,
            console_codes,
        })
    }

    /// Every named shape any definition type reaches, by key, with the
    /// definition types that reach it.
    pub fn named(&self) -> BTreeMap<&str, (&clash_data::shape::Named, Vec<&'static str>)> {
        let mut all: BTreeMap<&str, (&clash_data::shape::Named, Vec<&'static str>)> =
            BTreeMap::new();
        for entry in &self.types {
            for (key, named) in &entry.shapes.named {
                all.entry(key.as_str())
                    .or_insert_with(|| (named, Vec::new()))
                    .1
                    .push(entry.name);
            }
        }
        all
    }

    /// Counts of what the reference covers, by kind of item, for the
    /// index page and the report.
    pub fn counts(&self) -> Vec<(&'static str, usize)> {
        let named = self.named();
        let fields: usize = named
            .values()
            .map(|(named, _)| match &named.shape {
                NamedShape::Struct(shape) if !is_data_layer(&named.rust) => shape.fields.len(),
                _ => 0,
            })
            .sum();
        let events = self.registry.events().filter(|e| !e.spec.hook).count();
        let hooks = self.registry.events().filter(|e| e.spec.hook).count();
        vec![
            ("definition types", self.types.len()),
            ("fields of definitions", fields),
            ("kinds of subject", self.registry.kinds().count()),
            ("properties", self.registry.properties().count()),
            ("functions", self.registry.functions().count()),
            ("events", events),
            ("hooks", hooks),
            ("verbs", self.registry.verbs().count()),
            ("effects", self.effects.len()),
            ("queries", self.registry.queries().count()),
            ("settings", self.settings.len()),
            ("reasons", self.reasons.len() + self.host_reasons.len()),
            (
                "diagnostic codes",
                self.codes.len() + self.expression_codes.len() + self.console_codes.len(),
            ),
        ]
    }

    /// Every registered item that has no documentation, one line each,
    /// sorted. The reference's test fails while this is not empty.
    pub fn undocumented(&self) -> Vec<String> {
        let mut missing = BTreeSet::new();
        for entry in &self.types {
            if entry.doc.trim().is_empty() {
                missing.insert(format!("definition type `{}`", entry.name));
            }
        }
        for key in clash_data::DIRECTIVE_KEYS {
            if !clash_data::DIRECTIVE_DOCS
                .iter()
                .any(|(known, _)| known == key)
            {
                missing.insert(format!("key `{key}` every definition carries"));
            }
        }
        for (named, users) in self.named().values() {
            if is_data_layer(&named.rust) {
                continue;
            }
            let found = typedocs::find(owner_path(&named.rust));
            let used_in = users.first().copied().unwrap_or("");
            let label = if named.name.contains('.') {
                named.name.clone()
            } else {
                format!("`{}`", named.name)
            };
            match &named.shape {
                NamedShape::Struct(shape) => {
                    let is_root = self
                        .types
                        .iter()
                        .any(|entry| entry.shapes.root == Shape::Named(named.rust.clone()));
                    if !is_root && named_doc(named).is_none() {
                        missing.insert(format!("object {label} (in `{used_in}`)"));
                    }
                    for field in &shape.fields {
                        if field_doc(named, field.name).is_none() {
                            missing.insert(format!(
                                "field `{}` of {label} (in `{used_in}`)",
                                field.name
                            ));
                        }
                    }
                }
                NamedShape::Enum(shape) => {
                    if found.is_none_or(|doc| doc.doc.trim().is_empty()) {
                        missing.insert(format!("enum {label} (in `{used_in}`)"));
                    }
                    for variant in &shape.variants {
                        if found.and_then(|doc| doc.item(variant.name)).is_none() {
                            missing.insert(format!(
                                "value `{}` of {label} (in `{used_in}`)",
                                variant.name
                            ));
                        }
                    }
                }
                NamedShape::Kinds(_) => {
                    if found.is_none_or(|doc| doc.doc.trim().is_empty()) {
                        missing.insert(format!("field type {label} (in `{used_in}`)"));
                    }
                }
            }
        }
        let registry = &self.registry;
        for kind in registry.kinds() {
            if kind.spec.doc.trim().is_empty() {
                missing.insert(format!("kind of subject `{}`", kind.spec.name));
            }
        }
        for property in registry.properties() {
            let name = registry.property_signature(property);
            let name = name.split(" -> ").next().unwrap_or(&name).to_owned();
            if property.spec.doc.trim().is_empty() {
                missing.insert(format!("property `{name}`"));
            }
            for param in &property.spec.params {
                if param.doc.trim().is_empty() {
                    missing.insert(format!("argument `{}` of property `{name}`", param.name));
                }
            }
        }
        for function in registry.functions() {
            if function.spec.doc.trim().is_empty() {
                missing.insert(format!("function `{}`", function.spec.name));
            }
            for param in &function.spec.params {
                if param.doc.trim().is_empty() {
                    missing.insert(format!(
                        "argument `{}` of function `{}`",
                        param.name, function.spec.name
                    ));
                }
            }
        }
        for event in registry.events() {
            let what = if event.spec.hook { "hook" } else { "event" };
            if event.spec.doc.trim().is_empty() {
                missing.insert(format!("{what} `{}`", event.spec.name));
            }
            for field in &event.spec.fields {
                if field.doc.trim().is_empty() {
                    missing.insert(format!(
                        "field `{}` of {what} `{}`",
                        field.name, event.spec.name
                    ));
                }
            }
        }
        for verb in registry.verbs() {
            if verb.spec.doc.trim().is_empty() {
                missing.insert(format!("verb `{}`", verb.spec.name));
            }
            for arg in &verb.spec.args {
                if arg.doc.trim().is_empty() {
                    missing.insert(format!(
                        "argument `{}` of verb `{}`",
                        arg.name, verb.spec.name
                    ));
                }
            }
        }
        for query in registry.queries() {
            if query.spec.doc.trim().is_empty() {
                missing.insert(format!("query `{}`", query.spec.name));
            }
            for arg in &query.spec.args {
                if arg.doc.trim().is_empty() {
                    missing.insert(format!(
                        "argument `{}` of query `{}`",
                        arg.name, query.spec.name
                    ));
                }
            }
        }
        for variable in registry.variables() {
            if variable.spec.doc.trim().is_empty() {
                missing.insert(format!("variable `{}`", variable.spec.name));
            }
        }
        for effect in &self.effects {
            if effect.info.doc.trim().is_empty() {
                missing.insert(format!("effect `{}`", effect.info.name));
            }
        }
        for setting in &self.settings {
            if setting.doc.trim().is_empty() {
                missing.insert(format!("setting `{}`", setting.name));
            }
        }
        for reason in &self.reasons {
            if reason.doc.trim().is_empty() {
                missing.insert(format!("reason `{}`", reason.code));
            }
        }
        for code in self
            .expression_codes
            .iter()
            .chain(&self.console_codes)
            .chain(&self.host_reasons)
        {
            if code.doc.trim().is_empty() {
                missing.insert(format!("code `{}`", code.code));
            }
        }
        missing.into_iter().collect()
    }
}

/// The documentation of a field of a named struct, from the doc
/// comments.
pub fn field_doc(named: &clash_data::shape::Named, field: &str) -> Option<&'static str> {
    if let Some((owner, variant)) = struct_variant(&named.rust) {
        // A struct written as a value of an enum: its fields are
        // documented on the enum's own value, which says what it holds.
        return typedocs::find(owner)
            .and_then(|doc| doc.item(&format!("{variant}.{field}")))
            .or_else(|| typedocs::find(&named.rust).and_then(|doc| doc.item(field)));
    }
    typedocs::find(&named.rust).and_then(|doc| doc.item(field))
}

/// The documentation of a named shape itself.
pub fn named_doc(named: &clash_data::shape::Named) -> Option<&'static str> {
    if let Some((owner, variant)) = struct_variant(&named.rust) {
        return typedocs::find(owner).and_then(|doc| doc.item(variant));
    }
    typedocs::find(&named.rust)
        .map(|doc| doc.doc)
        .filter(|doc| !doc.trim().is_empty())
}

/// The Rust path documentation is found under for a named shape: for a
/// struct written as a value of an enum, the enum.
fn owner_path(rust: &str) -> &str {
    struct_variant(rust).map_or(rust, |(owner, _)| owner)
}

/// For the key of a struct written as a value of an enum, the enum's
/// path and the value's Rust name.
fn struct_variant(rust: &str) -> Option<(&str, &str)> {
    // `crate::module::Enum::Value`, where `Value` is not a module: the
    // enum is the segment before, and it starts with a capital.
    let (owner, variant) = rust.rsplit_once("::")?;
    let owner_last = owner.rsplit("::").next().unwrap_or(owner);
    let generic_free = !rust.contains('<');
    (generic_free
        && owner_last.chars().next().is_some_and(char::is_uppercase)
        && variant.chars().next().is_some_and(char::is_lowercase))
    .then_some((owner, variant))
}

/// Whether a Rust type is one of the field types of the data layer or of
/// the rule runtime that the reference describes once, on the values
/// page, rather than wherever it is used: expressions, conditions,
/// texts, lists of effects, and plain JSON.
pub fn is_data_layer(rust: &str) -> bool {
    data_layer(rust).is_some()
}

/// The field types the values page describes, by the start of their
/// Rust path: the anchor on that page and what a field of it is called.
pub const DATA_LAYER: &[(&str, &str, &str)] = &[
    ("clash_data::expr::field::Expr", "expressions", "expression"),
    ("clash_data::expr::field::Cond", "conditions", "condition"),
    (
        "clash_data::expr::field::CondObject",
        "conditions",
        "condition",
    ),
    ("clash_data::expr::field::Literal", "expressions", "literal"),
    (
        "clash_data::expr::field::SitedString",
        "expressions",
        "expression",
    ),
    (
        "clash_data::expr::field::SourceObject",
        "value-sources",
        "value source",
    ),
    (
        "clash_data::expr::field::NodeBody",
        "value-sources",
        "value source",
    ),
    (
        "clash_data::expr::defs::ParamValue",
        "parameters",
        "parameter",
    ),
    (
        "clash_data::expr::defs::ParamObject",
        "parameters",
        "parameter",
    ),
    ("clash_data::expr::query", "queries", "query"),
    ("clash_data::fields::Text", "texts", "text"),
    (
        "clash_sim::rule::effects::Effects",
        "effects",
        "list of effects",
    ),
    (
        "clash_sim::rule::effects::Body",
        "effects",
        "list of effects",
    ),
    ("serde_json::value::Value", "json", "any JSON value"),
];

/// The values page's anchor and the name of a field type of the data
/// layer, if `rust` is one.
pub fn data_layer(rust: &str) -> Option<(&'static str, &'static str)> {
    DATA_LAYER
        .iter()
        .find(|(prefix, _, _)| {
            rust == *prefix
                || rust.strip_prefix(prefix).is_some_and(|rest| {
                    rest.starts_with('<') || rest.starts_with("::") || rest.starts_with('#')
                })
        })
        .map(|(_, anchor, name)| (*anchor, *name))
}

/// What an expression's type parameter is called: `number`, `bool`.
pub fn expr_type(rust: &str) -> Option<String> {
    let inner = rust.split_once('<')?.1.strip_suffix('>')?;
    let last = inner.rsplit("::").next().unwrap_or(inner);
    let last = last.split('<').next().unwrap_or(last);
    Some(match last {
        "Number" => "number".to_owned(),
        "Int" => "int".to_owned(),
        "Bool" => "bool".to_owned(),
        "Str" => "string".to_owned(),
        "Any" => "any".to_owned(),
        "IdOf" => {
            // `IdOf<some::Def>`: the id of a definition.
            let def = inner.split_once('<')?.1.trim_end_matches('>');
            let def = def.rsplit("::").next().unwrap_or(def);
            format!("id of {def}")
        }
        other => other.to_lowercase(),
    })
}

/// The named shapes a shape refers to directly.
pub fn referred(shape: &Shape, out: &mut Vec<String>) {
    match shape {
        Shape::Named(key) => out.push(key.clone()),
        Shape::List(inner) | Shape::Optional(inner) | Shape::Gated(_, inner) => {
            referred(inner, out);
        }
        Shape::Tuple(items) => {
            for item in items {
                referred(item, out);
            }
        }
        Shape::Map(key, value) => {
            referred(key, out);
            referred(value, out);
        }
        _ => {}
    }
}

#[cfg(test)]
pub(crate) mod tests;
