//! Finding definitions and what refers to them (14 §9.2).
//!
//! A finished definition is walked beside what its type reads
//! ([`clash_data::shape`]), so that a string read as a reference is
//! known to be one, and resolved as the loader resolves it: in the
//! namespace of the mod that wrote it, then its imports, then `engine`
//! (04 §4.3). Inheritance (`copy_from`) and patches are read from the
//! files, which keep them. A string read as plain text that names a
//! definition, an expression naming `'core:prone'` or a rule attached by
//! name in an effect, is a *mention*: what it is depends on what reads
//! it, which the loader does not say, so it is listed apart.
//!
//! `find` reads the same walk: a condition names a field by its path in
//! the finished definition, lists and maps looked through.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;

use clash_data::shape::{JsonKind, NamedShape, Shape, Shapes, VariantForm};
use clash_data::{DefTypes, Defs, FileId, FinishedTree, Limits, Node, NodeKind, Num, Span};

use crate::data::{DEFAULT_MODS_DIR, EXIT_FAILED, EXIT_OK, Options, Output};
use crate::reference::{TypeEntry, type_entries};

/// How one definition refers to another.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum RefKind {
    /// A field read as a reference names it.
    Field,
    /// It is copied from it (`copy_from`).
    Copies,
    /// A patch changes it: the definition referring is the patch's own.
    Patches,
    /// A string read as plain text names it.
    Mentions,
}

impl RefKind {
    fn label(self) -> &'static str {
        match self {
            RefKind::Field => "refers to it",
            RefKind::Copies => "copies it",
            RefKind::Patches => "patches it",
            RefKind::Mentions => "names it in plain text",
        }
    }
}

/// One reference from a definition to another.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Reference {
    /// The type of the definition that refers.
    pub from_type: String,
    /// Its qualified id. For a patch, the id of what it patches.
    pub from_id: String,
    /// Where in it, as a path of keys (lists looked through).
    pub path: String,
    /// Where the reference was written.
    pub span: Span,
    /// The type of the definition referred to.
    pub to_type: String,
    /// Its qualified id.
    pub to_id: String,
    /// How it refers.
    pub kind: RefKind,
}

/// A value of a finished definition and what its type reads it as.
#[derive(Clone, Copy, Debug)]
enum Reading {
    /// A reference to a definition of a type.
    Ref(&'static str),
    /// A tag, which names no definition.
    Tag,
    /// A definition written in place, which has a tree of its own.
    Inline,
    /// Anything else.
    Plain,
}

/// A value of a finished definition, or a key of one of its maps.
#[derive(Clone, Copy, Debug)]
enum Value<'a> {
    Node(&'a Node),
    Key(&'a str),
}

/// One place in a finished definition.
#[derive(Clone, Debug)]
struct Entry<'a> {
    /// The keys leading to it, lists looked through.
    path: String,
    span: Span,
    value: Value<'a>,
    reading: Reading,
}

/// Every place in a finished definition, read as its type reads it.
fn entries<'a>(shapes: &Shapes, tree: &'a Node) -> Vec<Entry<'a>> {
    let mut out = Vec::new();
    walk(shapes, &shapes.root, tree, "", 0, &mut out);
    out
}

fn join(path: &str, key: &str) -> String {
    if path.is_empty() {
        key.to_owned()
    } else {
        format!("{path}.{key}")
    }
}

fn json_kind(node: &Node) -> Vec<JsonKind> {
    match &node.kind {
        NodeKind::Null => vec![JsonKind::Null],
        NodeKind::Bool(_) => vec![JsonKind::Bool],
        NodeKind::Number(Num::Int(_) | Num::UInt(_)) => vec![JsonKind::Integer, JsonKind::Number],
        NodeKind::Number(Num::Float(value)) if value.fract() == 0.0 => {
            vec![JsonKind::Number, JsonKind::Integer]
        }
        NodeKind::Number(_) | NodeKind::Quantity { .. } => vec![JsonKind::Number],
        NodeKind::String(_) => vec![JsonKind::String],
        NodeKind::Array(_) => vec![JsonKind::List],
        NodeKind::Object(_) => vec![JsonKind::Object],
    }
}

/// The deepest a walk goes, against a shape that names itself.
const MAX_DEPTH: usize = 64;

fn walk<'a>(
    shapes: &Shapes,
    shape: &Shape,
    node: &'a Node,
    path: &str,
    depth: usize,
    out: &mut Vec<Entry<'a>>,
) {
    let mut shape = shape;
    while let Shape::Optional(inner) | Shape::Gated(_, inner) = shape {
        shape = inner;
    }
    let reading = match (shape, &node.kind) {
        (Shape::Ref(ty) | Shape::Inline(ty), NodeKind::String(_)) => Reading::Ref(ty),
        (Shape::Inline(_), NodeKind::Object(_)) => Reading::Inline,
        (Shape::Tag, _) => Reading::Tag,
        _ => Reading::Plain,
    };
    out.push(Entry {
        path: path.to_owned(),
        span: node.span,
        value: Value::Node(node),
        reading,
    });
    if depth > MAX_DEPTH || matches!(reading, Reading::Inline) {
        return;
    }
    match (shape, &node.kind) {
        (Shape::Named(key), _) => match shapes.named.get(key).map(|named| &named.shape) {
            Some(NamedShape::Struct(shape)) => {
                if let NodeKind::Object(members) = &node.kind {
                    for member in members {
                        let field = shape.fields.iter().find(|field| field.name == member.key);
                        let inner = field.map_or(&Shape::Unknown, |field| &field.shape);
                        walk(
                            shapes,
                            inner,
                            &member.value,
                            &join(path, &member.key),
                            depth + 1,
                            out,
                        );
                    }
                }
            }
            Some(NamedShape::Enum(shape)) => {
                if let NodeKind::Object(members) = &node.kind
                    && let [member] = members.as_slice()
                    && let Some(variant) = shape.variants.iter().find(|v| v.name == member.key)
                {
                    let inner = match &variant.form {
                        VariantForm::Newtype(inner) => inner.clone(),
                        VariantForm::Tuple(items) => Shape::Tuple(items.clone()),
                        VariantForm::Struct(key) => Shape::Named(key.clone()),
                        VariantForm::Unit | VariantForm::Unknown => Shape::Unknown,
                    };
                    walk(
                        shapes,
                        &inner,
                        &member.value,
                        &join(path, &member.key),
                        depth + 1,
                        out,
                    );
                } else {
                    walk_plain(shapes, node, path, depth, out);
                }
            }
            Some(NamedShape::Kinds(shape)) => {
                let inner = json_kind(node).into_iter().find_map(|kind| {
                    shape
                        .accepted
                        .iter()
                        .find(|(accepted, _)| *accepted == kind)
                        .map(|(_, inner)| inner)
                });
                match inner {
                    None | Some(Shape::Any | Shape::Unknown) => {
                        walk_plain(shapes, node, path, depth, out);
                    }
                    Some(inner) => {
                        // The kind's own reading of the same value.
                        out.pop();
                        walk(shapes, inner, node, path, depth + 1, out);
                    }
                }
            }
            None => walk_plain(shapes, node, path, depth, out),
        },
        (Shape::List(inner), NodeKind::Array(items)) => {
            for item in items {
                walk(shapes, inner, item, path, depth + 1, out);
            }
        }
        (Shape::Tuple(inners), NodeKind::Array(items)) => {
            for (index, item) in items.iter().enumerate() {
                let inner = inners.get(index).unwrap_or(&Shape::Unknown);
                walk(shapes, inner, item, path, depth + 1, out);
            }
        }
        (Shape::Map(key, value), NodeKind::Object(members)) => {
            let key_reading = match key.as_ref() {
                Shape::Ref(ty) | Shape::Inline(ty) => Reading::Ref(ty),
                Shape::Tag => Reading::Tag,
                _ => Reading::Plain,
            };
            for member in members {
                out.push(Entry {
                    path: path.to_owned(),
                    span: member.key_span,
                    value: Value::Key(&member.key),
                    reading: key_reading,
                });
                walk(
                    shapes,
                    value,
                    &member.value,
                    &join(path, &member.key),
                    depth + 1,
                    out,
                );
            }
        }
        _ => walk_plain(shapes, node, path, depth, out),
    }
}

/// Walks what no shape describes: every key and value, read as plain.
fn walk_plain<'a>(
    shapes: &Shapes,
    node: &'a Node,
    path: &str,
    depth: usize,
    out: &mut Vec<Entry<'a>>,
) {
    match &node.kind {
        NodeKind::Object(members) => {
            for member in members {
                out.push(Entry {
                    path: path.to_owned(),
                    span: member.key_span,
                    value: Value::Key(&member.key),
                    reading: Reading::Plain,
                });
                walk(
                    shapes,
                    &Shape::Unknown,
                    &member.value,
                    &join(path, &member.key),
                    depth + 1,
                    out,
                );
            }
        }
        NodeKind::Array(items) => {
            for item in items {
                walk(shapes, &Shape::Unknown, item, path, depth + 1, out);
            }
        }
        _ => {}
    }
}

/// Resolves ids as the loader does.
pub struct Resolver<'a> {
    defs: &'a Defs,
    /// Every qualified id, by type.
    ids: BTreeMap<&'a str, BTreeSet<&'a str>>,
    /// The types that have a definition of each qualified id.
    types_of: BTreeMap<&'a str, Vec<&'a str>>,
    /// The types that have a definition of each name, by its name alone.
    named: BTreeMap<&'a str, BTreeSet<&'a str>>,
}

impl<'a> Resolver<'a> {
    /// A resolver over the definitions of a load.
    pub fn new(defs: &'a Defs) -> Resolver<'a> {
        let mut ids: BTreeMap<&str, BTreeSet<&str>> = BTreeMap::new();
        let mut types_of: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
        let mut named: BTreeMap<&str, BTreeSet<&str>> = BTreeMap::new();
        for tree in defs.trees() {
            if tree.inline {
                continue;
            }
            let ty = tree.type_name.as_str();
            ids.entry(ty).or_default().insert(tree.id.as_str());
            types_of.entry(tree.id.as_str()).or_default().push(ty);
            if let Some((_, name)) = tree.id.split_once(':') {
                named.entry(name).or_default().insert(ty);
            }
        }
        Resolver {
            defs,
            ids,
            types_of,
            named,
        }
    }

    /// The mod that wrote the file, if a mod did.
    fn mod_of(&self, file: FileId) -> Option<&'a clash_data::LoadedModInfo> {
        let path = self.defs.sources().path(file);
        self.defs.mods().iter().find(|loaded| {
            path.strip_prefix(loaded.root.as_str())
                .is_some_and(|rest| rest.starts_with('/') || rest.starts_with('\\'))
        })
    }

    /// The namespaces a bare name written in a file is looked up in.
    fn search_order(&self, file: FileId) -> Vec<&'a str> {
        let mut order: Vec<&str> = Vec::new();
        if let Some(loaded) = self.mod_of(file) {
            order.push(&loaded.manifest.id);
            for import in &loaded.manifest.imports {
                if !order.contains(&import.as_str()) {
                    order.push(import);
                }
            }
        }
        if !order.contains(&clash_data::ENGINE_NAMESPACE) {
            order.push(clash_data::ENGINE_NAMESPACE);
        }
        order
    }

    /// The qualified id an id written at `span` names among the
    /// definitions of a type, if it names one.
    pub fn resolve(&self, ty: &str, text: &str, span: Span) -> Option<String> {
        let ids = self.ids.get(ty)?;
        if text.contains(':') {
            return ids.contains(text).then(|| text.to_owned());
        }
        self.search_order(span.file)
            .into_iter()
            .map(|namespace| format!("{namespace}:{text}"))
            .find(|candidate| ids.contains(candidate.as_str()))
    }

    /// The definitions a string read as plain text names: each
    /// qualified id in it, and the whole string as a bare name where it
    /// would resolve.
    fn mentioned(&self, text: &str, span: Span) -> Vec<(&'a str, String)> {
        let mut found = Vec::new();
        for token in qualified_tokens(text) {
            if let Some((&id, types)) = self.types_of.get_key_value(token) {
                for ty in types {
                    found.push((*ty, id.to_owned()));
                }
            }
        }
        if !text.is_empty()
            && !text.contains(':')
            && let Some(types) = self.named.get(text)
        {
            for ty in types {
                if let Some(id) = self.resolve(ty, text, span) {
                    found.push((*ty, id));
                }
            }
        }
        found
    }
}

/// The words of a text that have the form of a qualified id.
fn qualified_tokens(text: &str) -> Vec<&str> {
    let is_part = |c: char| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_';
    let bytes: Vec<char> = text.chars().collect();
    let mut tokens = Vec::new();
    let mut start = None;
    let mut offset = 0;
    let mut offsets = Vec::with_capacity(bytes.len() + 1);
    for c in &bytes {
        offsets.push(offset);
        offset += c.len_utf8();
    }
    offsets.push(offset);
    for (index, c) in bytes.iter().enumerate() {
        let inside = is_part(*c) || *c == ':';
        match (start, inside) {
            (None, true) => start = Some(index),
            (Some(from), false) => {
                tokens.push(&text[offsets[from]..offsets[index]]);
                start = None;
            }
            _ => {}
        }
    }
    if let Some(from) = start {
        tokens.push(&text[offsets[from]..]);
    }
    tokens
        .into_iter()
        .filter(|token| {
            token.split_once(':').is_some_and(|(namespace, name)| {
                clash_data::is_id_part(namespace) && clash_data::is_id_part(name)
            })
        })
        .collect()
}

/// Every reference between the definitions of a load: the fields read
/// as references, inheritance and patches, and mentions.
pub fn references(defs: &Defs, entries: &[TypeEntry]) -> Vec<Reference> {
    let resolver = Resolver::new(defs);
    let shapes: BTreeMap<&str, &Shapes> = entries
        .iter()
        .map(|entry| (entry.name, &entry.shapes))
        .collect();
    let mut out = Vec::new();
    for tree in defs.trees() {
        let Some(shapes) = shapes.get(tree.type_name.as_str()) else {
            continue;
        };
        for entry in self::entries(shapes, &tree.tree) {
            let text = match entry.value {
                Value::Node(Node {
                    kind: NodeKind::String(text),
                    ..
                }) => text.as_str(),
                Value::Key(text) => text,
                Value::Node(_) => continue,
            };
            let push = |out: &mut Vec<Reference>, ty: &str, id: String, kind: RefKind| {
                out.push(Reference {
                    from_type: tree.type_name.clone(),
                    from_id: tree.id.clone(),
                    path: entry.path.clone(),
                    span: entry.span,
                    to_type: ty.to_owned(),
                    to_id: id,
                    kind,
                });
            };
            match entry.reading {
                Reading::Ref(ty) => {
                    if let Some(id) = resolver.resolve(ty, text, entry.span) {
                        push(&mut out, ty, id, RefKind::Field);
                    }
                }
                Reading::Plain => {
                    for (ty, id) in resolver.mentioned(text, entry.span) {
                        if !(ty == tree.type_name && id == tree.id) {
                            push(&mut out, ty, id, RefKind::Mentions);
                        }
                    }
                }
                Reading::Tag | Reading::Inline => {}
            }
        }
    }
    out.extend(written_links(defs, &resolver));
    out.sort_by(|a, b| {
        (a.kind, &a.from_type, &a.from_id, a.span.file, a.span.start).cmp(&(
            b.kind,
            &b.from_type,
            &b.from_id,
            b.span.file,
            b.span.start,
        ))
    });
    out.dedup();
    out
}

/// What the files say that the finished definitions forget: which
/// definition is copied from which, and what each patch changes.
fn written_links(defs: &Defs, resolver: &Resolver<'_>) -> Vec<Reference> {
    let sources = defs.sources();
    let mut out = Vec::new();
    for index in 0..sources.len() {
        let Ok(index) = u32::try_from(index) else {
            break;
        };
        let file = FileId(index);
        let Some(loaded) = resolver.mod_of(file) else {
            continue;
        };
        if !sources.path(file).ends_with(".json") {
            continue;
        }
        let (tree, _) = clash_data::parse_json(file, sources.text(file), &Limits::default());
        let Some(tree) = tree else {
            continue;
        };
        let definitions: Vec<&Node> = match &tree.kind {
            NodeKind::Array(items) => items.iter().collect(),
            NodeKind::Object(_) => vec![&tree],
            _ => Vec::new(),
        };
        for definition in definitions {
            let (Some(ty), Some(id)) = (
                definition.get("type").and_then(Node::as_str),
                definition.member("id"),
            ) else {
                continue;
            };
            let Some(id_text) = id.value.as_str() else {
                continue;
            };
            let own = if id_text.contains(':') {
                id_text.to_owned()
            } else {
                format!("{}:{id_text}", loaded.manifest.id)
            };
            if definition.get("patch").and_then(Node::as_bool) == Some(true) {
                if let Some(target) = resolver.resolve(ty, id_text, id.value.span) {
                    out.push(Reference {
                        from_type: ty.to_owned(),
                        from_id: target.clone(),
                        path: "patch".to_owned(),
                        span: id.value.span,
                        to_type: ty.to_owned(),
                        to_id: target,
                        kind: RefKind::Patches,
                    });
                }
                continue;
            }
            if let Some(parent) = definition.member("copy_from")
                && let Some(text) = parent.value.as_str()
                && let Some(target) = resolver.resolve(ty, text, parent.value.span)
            {
                out.push(Reference {
                    from_type: ty.to_owned(),
                    from_id: own,
                    path: "copy_from".to_owned(),
                    span: parent.value.span,
                    to_type: ty.to_owned(),
                    to_id: target,
                    kind: RefKind::Copies,
                });
            }
        }
    }
    out
}

/// Loads the definitions a data command names.
fn load(types: &DefTypes, options: &Options) -> clash_data::LoadOutcome {
    match &options.profile {
        Some(profile) => crate::data::load_profile(types, options, profile),
        None => {
            let mut loader = crate::data::loader(types, options);
            if options.mods.is_empty() {
                loader = loader.search_path(DEFAULT_MODS_DIR);
            }
            loader.load_all()
        }
    }
}

fn not_loaded(outcome: &clash_data::LoadOutcome) -> Output {
    Output {
        out: outcome.diagnostics.render(),
        err: "clash-tools: the mods did not load, so there is nothing to search\n".to_owned(),
        code: EXIT_FAILED,
    }
}

/// `file:line`.
fn place(defs: &Defs, span: Span) -> String {
    let location = defs.sources().locate(span);
    format!("{}:{}", location.file, location.line)
}

/// The definitions an id given on the command line names: a qualified
/// id, or a name in any namespace.
fn named<'a>(defs: &'a Defs, type_name: Option<&str>, id: &str) -> Vec<&'a FinishedTree> {
    defs.trees()
        .iter()
        .filter(|tree| !tree.inline && type_name.is_none_or(|name| tree.type_name == name))
        .filter(|tree| {
            tree.id == id
                || (!id.contains(':')
                    && tree.id.split_once(':').is_some_and(|(_, name)| name == id))
        })
        .collect()
}

/// `refs [<type>] <id>`.
pub(crate) fn refs_command(types: &DefTypes, options: &Options) -> Output {
    let (type_name, id) = match options.positional.as_slice() {
        [id] => (None, id.as_str()),
        [type_name, id] => (Some(type_name.as_str()), id.as_str()),
        _ => return Output::usage("`refs` takes a definition type and an id, or an id alone"),
    };
    let outcome = load(types, options);
    let Some(defs) = &outcome.defs else {
        return not_loaded(&outcome);
    };
    let found = named(defs, type_name, id);
    if found.is_empty() {
        let what = type_name.unwrap_or("definition");
        let mut err = format!("clash-tools: there is no {what} `{id}`\n");
        let ids: Vec<&str> = defs
            .trees()
            .iter()
            .filter(|tree| !tree.inline && type_name.is_none_or(|name| tree.type_name == name))
            .map(|tree| tree.id.as_str())
            .collect();
        // A bare name is compared with bare names.
        let near = if id.contains(':') {
            clash_data::diag::closest(id, ids.iter().copied()).map(str::to_owned)
        } else {
            let names: Vec<&str> = ids
                .iter()
                .filter_map(|qualified| qualified.split_once(':').map(|(_, name)| name))
                .collect();
            clash_data::diag::closest(id, names.iter().copied()).and_then(|name| {
                ids.iter()
                    .find(|qualified| qualified.split_once(':').is_some_and(|(_, n)| n == name))
                    .map(|qualified| (*qualified).to_owned())
            })
        };
        if let Some(near) = near {
            let _ = writeln!(err, "did you mean `{near}`?");
        }
        return Output {
            out: String::new(),
            err,
            code: EXIT_FAILED,
        };
    }
    let entries = type_entries(types);
    let all = references(defs, &entries);
    let mut out = String::new();
    for (index, tree) in found.iter().enumerate() {
        if index > 0 {
            out.push('\n');
        }
        out.push_str(&describe(defs, tree, &all));
    }
    Output {
        out,
        err: String::new(),
        code: EXIT_OK,
    }
}

/// What a definition refers to and what refers to it.
pub fn describe(defs: &Defs, tree: &FinishedTree, all: &[Reference]) -> String {
    let mut out = format!(
        "{} {}  defined at {}\n",
        tree.type_name,
        tree.id,
        place(defs, tree.tree.span)
    );
    // Its own references, those of what is written in place in it, and
    // its own patches and inheritance.
    let inline_prefix = format!("{}#", tree.id);
    let outgoing: Vec<&Reference> = all
        .iter()
        .filter(|reference| {
            reference.kind != RefKind::Patches
                && (reference.from_type == tree.type_name && reference.from_id == tree.id
                    || reference.from_id.starts_with(&inline_prefix))
        })
        .collect();
    let incoming: Vec<&Reference> = all
        .iter()
        .filter(|reference| reference.to_type == tree.type_name && reference.to_id == tree.id)
        .collect();
    let _ = writeln!(out, "\nrefers to ({}):", outgoing.len());
    if outgoing.is_empty() {
        out.push_str("  nothing\n");
    }
    for reference in &outgoing {
        let how = match reference.kind {
            RefKind::Copies => " (copies it)",
            RefKind::Mentions => " (named in plain text)",
            _ => "",
        };
        let _ = writeln!(
            out,
            "  {} {}  at `{}`, {}{how}",
            reference.to_type,
            reference.to_id,
            reference.path,
            place(defs, reference.span)
        );
    }
    let sure = incoming
        .iter()
        .filter(|reference| reference.kind != RefKind::Mentions)
        .count();
    let _ = writeln!(out, "\nreferred to by ({sure}):");
    if sure == 0 {
        out.push_str("  nothing\n");
    }
    for reference in incoming.iter().filter(|r| r.kind != RefKind::Mentions) {
        let line = match reference.kind {
            RefKind::Field => format!(
                "{} {}, at `{}`",
                reference.from_type, reference.from_id, reference.path
            ),
            RefKind::Patches => "a patch, which changes it".to_owned(),
            other => format!(
                "{} {}, which {}",
                reference.from_type,
                reference.from_id,
                other.label()
            ),
        };
        let _ = writeln!(out, "  {}: {line}", place(defs, reference.span));
    }
    let mentions: Vec<&&Reference> = incoming
        .iter()
        .filter(|reference| reference.kind == RefKind::Mentions)
        .collect();
    if !mentions.is_empty() {
        let _ = writeln!(
            out,
            "\nnamed in plain text by ({}): strings the loader does not read as references, \
             which a table of their own may resolve (the world's do), or which may mean \
             something else of the same name:",
            mentions.len()
        );
        for reference in mentions {
            let _ = writeln!(
                out,
                "  {}: {} {}, at `{}`",
                place(defs, reference.span),
                reference.from_type,
                reference.from_id,
                reference.path
            );
        }
    }
    out
}

/// A test `find` applies to a definition.
#[derive(Clone, Debug, PartialEq)]
pub enum Condition {
    /// The path has a value that is not `null`.
    Has(String),
    /// It has none.
    Lacks(String),
    /// A value at the path compares with the operand.
    Compare(String, Op, String),
    /// The id, name, description or documentation holds the text,
    /// whatever its case.
    Text(String),
}

/// How a value at a path is compared.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Op {
    /// `==`.
    Eq,
    /// `!=`: no value at the path equals the operand.
    Ne,
    /// `<`.
    Lt,
    /// `<=`.
    Le,
    /// `>`.
    Gt,
    /// `>=`.
    Ge,
    /// `~`: a value holds the text, whatever its case.
    Contains,
}

/// Reads one condition as written after `where=`.
pub fn parse_condition(text: &str) -> Result<Condition, String> {
    const OPS: [(&str, Op); 7] = [
        ("==", Op::Eq),
        ("!=", Op::Ne),
        (">=", Op::Ge),
        ("<=", Op::Le),
        ("~", Op::Contains),
        (">", Op::Gt),
        ("<", Op::Lt),
    ];
    let earliest = OPS
        .iter()
        .filter_map(|(symbol, op)| text.find(symbol).map(|at| (at, *symbol, *op)))
        .min_by_key(|(at, symbol, _)| (*at, std::cmp::Reverse(symbol.len())));
    let path_ok = |path: &str| {
        !path.is_empty()
            && path
                .split('.')
                .all(|part| !part.is_empty() && !part.contains(char::is_whitespace))
    };
    match earliest {
        Some((at, symbol, op)) => {
            let path = text[..at].trim();
            let operand = text[at + symbol.len()..].trim();
            let operand = operand
                .strip_prefix('"')
                .and_then(|rest| rest.strip_suffix('"'))
                .or_else(|| {
                    operand
                        .strip_prefix('\'')
                        .and_then(|rest| rest.strip_suffix('\''))
                })
                .unwrap_or(operand);
            if !path_ok(path) {
                return Err(format!("`{text}` names no field before `{symbol}`"));
            }
            Ok(Condition::Compare(path.to_owned(), op, operand.to_owned()))
        }
        None => {
            let text = text.trim();
            match text.strip_prefix('!') {
                Some(path) if path_ok(path.trim()) => Ok(Condition::Lacks(path.trim().to_owned())),
                None if path_ok(text) => Ok(Condition::Has(text.to_owned())),
                _ => Err(format!(
                    "`{text}` is not a condition: `path`, `!path` or `path<op>value`"
                )),
            }
        }
    }
}

/// A number a value stands for: a number, or a quantity in SI units.
fn number_of(text: &str) -> Option<f64> {
    text.trim().parse::<f64>().ok().or_else(|| {
        clash_data::units::parse_quantity(text)
            .ok()
            .map(|q| q.value)
    })
}

/// What a value is when it is compared: its text, the qualified id it
/// names when it is a reference, and the number it stands for.
struct Compared {
    text: String,
    id: Option<String>,
    number: Option<f64>,
}

fn compared(resolver: &Resolver<'_>, entry: &Entry<'_>) -> Option<Compared> {
    let text = match entry.value {
        Value::Key(key) => key.to_owned(),
        Value::Node(node) => match &node.kind {
            NodeKind::String(text) => text.clone(),
            NodeKind::Bool(value) => value.to_string(),
            NodeKind::Number(number) => {
                return Some(Compared {
                    text: node.to_json(),
                    id: None,
                    number: Some(number.as_f64()),
                });
            }
            NodeKind::Quantity { value, .. } => {
                return Some(Compared {
                    text: value.to_string(),
                    id: None,
                    number: Some(*value),
                });
            }
            NodeKind::Null | NodeKind::Array(_) | NodeKind::Object(_) => return None,
        },
    };
    let id = match entry.reading {
        Reading::Ref(ty) => resolver.resolve(ty, &text, entry.span),
        _ => None,
    };
    let number = number_of(&text);
    Some(Compared { text, id, number })
}

fn compare(value: &Compared, op: Op, operand: &str) -> bool {
    let equal = || {
        if let Some(id) = &value.id {
            return id == operand
                || (!operand.contains(':')
                    && id.split_once(':').is_some_and(|(_, name)| name == operand));
        }
        if value.text == operand {
            return true;
        }
        matches!((value.number, number_of(operand)), (Some(a), Some(b)) if a == b)
    };
    let order = || match (value.number, number_of(operand)) {
        (Some(a), Some(b)) => a.partial_cmp(&b),
        _ => None,
    };
    match op {
        Op::Eq => equal(),
        Op::Ne => !equal(),
        Op::Lt => order().is_some_and(|o| o.is_lt()),
        Op::Le => order().is_some_and(|o| o.is_le()),
        Op::Gt => order().is_some_and(|o| o.is_gt()),
        Op::Ge => order().is_some_and(|o| o.is_ge()),
        Op::Contains => {
            let operand = operand.to_lowercase();
            value.text.to_lowercase().contains(&operand)
                || value.id.as_ref().is_some_and(|id| id.contains(&operand))
        }
    }
}

/// Whether a definition meets a condition.
fn meets(
    resolver: &Resolver<'_>,
    tree: &FinishedTree,
    entries: &[Entry<'_>],
    condition: &Condition,
) -> bool {
    fn values_at<'e, 'a>(
        entries: &'e [Entry<'a>],
        path: &'e str,
    ) -> impl Iterator<Item = &'e Entry<'a>> {
        entries.iter().filter(move |entry| {
            entry.path == path
                && !matches!(
                    entry.value,
                    Value::Node(Node {
                        kind: NodeKind::Null,
                        ..
                    })
                )
        })
    }
    let at = |path| values_at(entries, path);
    match condition {
        Condition::Has(path) => at(path).next().is_some(),
        Condition::Lacks(path) => at(path).next().is_none(),
        Condition::Compare(path, Op::Ne, operand) => !at(path)
            .filter_map(|entry| compared(resolver, entry))
            .any(|value| compare(&value, Op::Eq, operand)),
        Condition::Compare(path, op, operand) => at(path)
            .filter_map(|entry| compared(resolver, entry))
            .any(|value| compare(&value, *op, operand)),
        Condition::Text(text) => {
            let text = text.to_lowercase();
            tree.id.to_lowercase().contains(&text)
                || entries.iter().any(|entry| {
                    matches!(entry.path.as_str(), "name" | "description" | "doc")
                        && matches!(entry.value, Value::Node(Node { kind: NodeKind::String(value), .. }) if value.to_lowercase().contains(&text))
                })
        }
    }
}

/// The definitions of a load that meet every condition, of one type or
/// of any.
pub fn find<'a>(
    defs: &'a Defs,
    entries: &[TypeEntry],
    type_name: Option<&str>,
    conditions: &[Condition],
) -> Vec<&'a FinishedTree> {
    let resolver = Resolver::new(defs);
    let shapes: BTreeMap<&str, &Shapes> = entries
        .iter()
        .map(|entry| (entry.name, &entry.shapes))
        .collect();
    defs.trees()
        .iter()
        .filter(|tree| !tree.inline && type_name.is_none_or(|name| tree.type_name == name))
        .filter(|tree| {
            let Some(shapes) = shapes.get(tree.type_name.as_str()) else {
                return false;
            };
            let entries = self::entries(shapes, &tree.tree);
            conditions
                .iter()
                .all(|condition| meets(&resolver, tree, &entries, condition))
        })
        .collect()
}

/// `find <type> [where=<condition>]... [<text>]...`.
pub(crate) fn find_command(types: &DefTypes, options: &Options) -> Output {
    let Some((type_name, rest)) = options.positional.split_first() else {
        return Output::usage("`find` takes a definition type, or `*` for any, and conditions");
    };
    let type_name = (type_name != "*").then_some(type_name.as_str());
    if let Some(name) = type_name
        && !types.names().contains(&name)
    {
        let mut err = format!("clash-tools: there is no definition type `{name}`\n");
        if let Some(near) = clash_data::diag::closest(name, types.names()) {
            let _ = writeln!(err, "did you mean `{near}`?");
        }
        return Output {
            out: String::new(),
            err,
            code: EXIT_FAILED,
        };
    }
    let mut conditions = Vec::new();
    for word in rest {
        match word.strip_prefix("where=") {
            Some(text) => match parse_condition(text) {
                Ok(condition) => conditions.push(condition),
                Err(message) => return Output::usage(&message),
            },
            None => conditions.push(Condition::Text(word.clone())),
        }
    }
    let outcome = load(types, options);
    let Some(defs) = &outcome.defs else {
        return not_loaded(&outcome);
    };
    let entries: Vec<TypeEntry> = match type_name {
        Some(name) => crate::reference::type_entry(types, name)
            .into_iter()
            .collect(),
        None => type_entries(types),
    };
    let found = find(defs, &entries, type_name, &conditions);
    let mut out = String::new();
    for tree in &found {
        let name = tree
            .tree
            .get("name")
            .and_then(Node::as_str)
            .map(|name| format!("  \"{name}\""))
            .unwrap_or_default();
        let _ = writeln!(
            out,
            "{} {}{name}  {}",
            tree.type_name,
            tree.id,
            place(defs, tree.tree.span)
        );
    }
    let _ = writeln!(
        out,
        "{} definition{}",
        found.len(),
        if found.len() == 1 { "" } else { "s" }
    );
    Output {
        out,
        err: String::new(),
        code: EXIT_OK,
    }
}

#[cfg(test)]
mod tests;
