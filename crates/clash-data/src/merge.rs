//! From parsed files to finished trees: collecting definitions by type and
//! id, folding patches and overrides, and resolving inheritance (04 §5,
//! §6).
//!
//! Everything here works on JSON trees and knows nothing about what a
//! definition means. The stages:
//!
//! 1. **Collect.** Each object in each data file is checked for the keys
//!    every definition carries, given its qualified id and sorted into
//!    load order.
//! 2. **Fold.** For each `(type, id)` the objects are folded in load order
//!    into one authored definition: the original, then each patch; an
//!    override restarts the fold.
//! 3. **Finish.** `copy_from` is resolved, parents first. A finished tree
//!    is its parent's finished tree with each of its own layers applied as
//!    a change, then its pending `scale`s, then its pending `add`s.
//!
//! Within one change the order is fixed: plain fields, then `delete`,
//! then `extend`; `scale` and `add` wait until the end.

use std::collections::{BTreeMap, BTreeSet};

use crate::diag::{DefLabel, Diagnostic, Diagnostics, closest, code, did_you_mean};
use crate::json::{Member, Node, NodeKind, Num};
use crate::mods::{ENGINE_NAMESPACE, ModManifest, is_id_part};
use crate::source::{FileId, SourceMap, Span};
use crate::units::{Dim, parse_quantity};

/// The keys every definition may carry (04 §5). They never reach a
/// definition type.
pub const DIRECTIVE_KEYS: &[&str] = &[
    "type",
    "id",
    "abstract",
    "if_mod",
    "unless_mod",
    "obsolete",
    "copy_from",
    "patch",
    "override",
    "optional",
    "extend",
    "delete",
    "scale",
    "add",
];

/// What each of [`DIRECTIVE_KEYS`] does, for the generated reference
/// (04 §5, §6), with `//`, which is read anywhere.
pub const DIRECTIVE_DOCS: &[(&str, &str)] = &[
    (
        "type",
        "The definition type: one of the types of this reference. Required.",
    ),
    (
        "id",
        "The definition's id. A bare name defines it in the namespace of the mod that wrote it; a qualified `namespace:name` in another namespace is allowed only with `patch` or `override`. Required.",
    ),
    ("//", "A comment. Ignored anywhere, at any depth."),
    (
        "abstract",
        "`true`: the definition exists only to be inherited from with `copy_from`. It is never instantiated, and need not give every required field. Not inherited.",
    ),
    (
        "if_mod",
        "A mod id, or a list of them: the definition is read only if every one is in the profile.",
    ),
    (
        "unless_mod",
        "A mod id, or a list of them: the definition is read only if none is in the profile.",
    ),
    (
        "obsolete",
        "`true`: the definition stays loadable so that saves that name it work, and is left out of generation, spawning and lists. Referring to it from a definition that is not obsolete is a warning. Not inherited.",
    ),
    (
        "copy_from",
        "The id of a definition of the same type: this one is that one's finished definition with this object applied as a change (04 §6.2). It may be defined later or in another mod.",
    ),
    (
        "patch",
        "`true`: changes the existing definition of this id in place, field by field (04 §6.3). Patches apply in load order; patching what does not exist is an error unless `optional` is set.",
    ),
    (
        "override",
        "`true`: replaces the existing definition of this id completely (04 §6.4). Earlier patches are discarded.",
    ),
    (
        "optional",
        "With `patch`: if there is nothing to patch, skip this object rather than report an error.",
    ),
    (
        "extend",
        "An object of field paths and values to append to a list or a set, or merge into a map: `{ \"tags\": [\"x\"] }` (04 §6.1).",
    ),
    (
        "delete",
        "An object of field paths and entries to remove from a list or a set, or keys to remove from a map: `{ \"tags\": [\"y\"] }`.",
    ),
    (
        "scale",
        "An object of field paths and factors to multiply numbers by: `{ \"density\": 1.2 }`. Applied once the final value is known, before `add`.",
    ),
    (
        "add",
        "An object of field paths and amounts to add to numbers, which may carry a unit: `{ \"span\": 2 }`.",
    ),
];

/// A mod in load order.
#[derive(Clone, Debug)]
pub(crate) struct LoadedMod {
    pub(crate) manifest: ModManifest,
    pub(crate) root: String,
}

/// The mods of one load, in load order, and which mod wrote each file.
#[derive(Clone, Debug, Default)]
pub(crate) struct ModTable {
    pub(crate) mods: Vec<LoadedMod>,
    file_mod: BTreeMap<FileId, usize>,
    by_id: BTreeMap<String, usize>,
}

impl ModTable {
    pub(crate) fn push(&mut self, loaded: LoadedMod) -> usize {
        let index = self.mods.len();
        self.by_id.insert(loaded.manifest.id.clone(), index);
        self.file_mod.insert(loaded.manifest.file, index);
        self.mods.push(loaded);
        index
    }

    pub(crate) fn add_file(&mut self, file: FileId, mod_index: usize) {
        self.file_mod.insert(file, mod_index);
    }

    pub(crate) fn has(&self, id: &str) -> bool {
        self.by_id.contains_key(id)
    }

    pub(crate) fn mod_of(&self, file: FileId) -> Option<usize> {
        self.file_mod.get(&file).copied()
    }

    pub(crate) fn manifest_of(&self, file: FileId) -> Option<&ModManifest> {
        self.mod_of(file).map(|index| &self.mods[index].manifest)
    }

    /// The namespaces a bare name written in `file` is looked up in, in
    /// order: the writing mod's own, its imports, then `engine` (04 §4.3).
    pub(crate) fn search_order(&self, file: FileId) -> Vec<&str> {
        let mut order: Vec<&str> = Vec::new();
        if let Some(manifest) = self.manifest_of(file) {
            order.push(&manifest.id);
            for import in &manifest.imports {
                if !order.contains(&import.as_str()) {
                    order.push(import);
                }
            }
        }
        if !order.contains(&ENGINE_NAMESPACE) {
            order.push(ENGINE_NAMESPACE);
        }
        order
    }
}

/// Splits an id as written into namespace and name, checking the form of
/// both (04 §4.1). `None` if the form is wrong.
pub(crate) fn split_id(text: &str) -> Option<(Option<&str>, &str)> {
    match text.split_once(':') {
        Some((namespace, name)) => {
            (is_id_part(namespace) && is_id_part(name)).then_some((Some(namespace), name))
        }
        None => is_id_part(text).then_some((None, text)),
    }
}

/// What kind of object a definition was written as.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum RawKind {
    Original,
    Patch { optional: bool },
    Override,
}

/// One object from a data file, checked and qualified.
#[derive(Clone, Debug)]
pub(crate) struct RawDef {
    pub(crate) type_name: String,
    pub(crate) id: String,
    pub(crate) id_span: Span,
    pub(crate) mod_index: usize,
    pub(crate) kind: RawKind,
    pub(crate) node: Node,
    /// One of its directives is malformed. It keeps its id, so that what
    /// refers to it still resolves, but it cannot be finished.
    pub(crate) broken: bool,
}

/// A data file after parsing.
#[derive(Clone, Debug)]
pub(crate) struct ParsedFile {
    pub(crate) mod_index: usize,
    pub(crate) tree: Node,
}

pub(crate) type Key = (String, String);

/// A definition after merging (04 §6.5).
#[derive(Clone, Debug)]
pub(crate) struct Finished {
    pub(crate) type_name: String,
    pub(crate) id: String,
    /// Where the id of the object that started it was written.
    pub(crate) id_span: Span,
    /// The finished tree: fields only. `None` if merging failed; the
    /// reason has been reported.
    pub(crate) tree: Option<Node>,
    pub(crate) is_abstract: bool,
    pub(crate) obsolete: bool,
    /// Generated from an inline definition (04 §5).
    pub(crate) inline: bool,
}

/// What collecting, folding and finishing need to share.
pub(crate) struct MergeCx<'a> {
    pub(crate) sources: &'a SourceMap,
    pub(crate) mods: &'a ModTable,
    /// The JSON names of the registered definition types.
    pub(crate) type_names: &'a [&'static str],
    /// Whether a definition of an unregistered type is passed over
    /// instead of reported (`Loader::only_known_types`).
    pub(crate) skip_unknown_types: bool,
    /// Each type's declared defaults, for `scale` and `add` on a field
    /// the definition leaves out.
    pub(crate) defaults: &'a BTreeMap<&'static str, Node>,
    pub(crate) diagnostics: &'a mut Diagnostics,
}

impl MergeCx<'_> {
    fn report(&mut self, diagnostic: Diagnostic) {
        self.diagnostics.push(diagnostic);
    }

    fn bad_directive(&mut self, node: &Node, label: Option<&DefLabel>, message: String) {
        let diagnostic = Diagnostic::new(code::BAD_DIRECTIVE, message)
            .at(self.sources, node.span)
            .in_def(label);
        self.report(diagnostic);
    }
}

fn brace(node: &Node) -> Span {
    Span::new(node.span.file, node.span.start, node.span.start + 1)
}

/// Reads `if_mod` or `unless_mod`: a mod id or a list of them.
fn mod_list(node: &Node) -> Option<Vec<&str>> {
    match &node.kind {
        NodeKind::String(text) => Some(vec![text.as_str()]),
        NodeKind::Array(items) => items.iter().map(Node::as_str).collect(),
        _ => None,
    }
}

/// Stage 1: checks every object in every file and returns the definitions
/// in load order: by mod, then by file path, then by position in the file.
pub(crate) fn collect(files: &[ParsedFile], cx: &mut MergeCx<'_>) -> Vec<RawDef> {
    let mut out = Vec::new();
    for file in files {
        let objects: Vec<&Node> = match &file.tree.kind {
            NodeKind::Array(items) => items.iter().collect(),
            NodeKind::Object(_) => vec![&file.tree],
            _ => {
                let diagnostic = Diagnostic::new(
                    code::NOT_A_DEFINITION,
                    format!(
                        "a data file is a list of definitions or one definition, but this is {}",
                        file.tree.kind_name()
                    ),
                )
                .at(cx.sources, file.tree.span);
                cx.report(diagnostic);
                continue;
            }
        };
        for object in objects {
            if let Some(raw) = collect_one(object, file.mod_index, cx) {
                out.push(raw);
            }
        }
    }
    out
}

fn collect_one(object: &Node, mod_index: usize, cx: &mut MergeCx<'_>) -> Option<RawDef> {
    if object.as_object().is_none() {
        let diagnostic = Diagnostic::new(
            code::NOT_A_DEFINITION,
            format!(
                "a definition is an object, but this is {}",
                object.kind_name()
            ),
        )
        .at(cx.sources, object.span);
        cx.report(diagnostic);
        return None;
    }

    // `if_mod` and `unless_mod` come first: an object meant for a mod that
    // is not loaded is not looked at any further.
    for (key, wanted) in [("if_mod", true), ("unless_mod", false)] {
        let Some(node) = object.get(key) else {
            continue;
        };
        let Some(ids) = mod_list(node) else {
            cx.bad_directive(
                node,
                None,
                format!(
                    "`{key}` is a mod id or a list of mod ids, but this is {}",
                    node.kind_name()
                ),
            );
            return None;
        };
        let included = if wanted {
            ids.iter().all(|id| cx.mods.has(id))
        } else {
            !ids.iter().any(|id| cx.mods.has(id))
        };
        if !included {
            return None;
        }
    }

    let type_name = match object.get("type") {
        Some(node) => match node.as_str() {
            Some(text) => text,
            None => {
                let diagnostic = Diagnostic::new(
                    code::MISSING_TYPE,
                    format!("`type` must be a string, but this is {}", node.kind_name()),
                )
                .at(cx.sources, node.span);
                cx.report(diagnostic);
                return None;
            }
        },
        None => {
            let diagnostic = Diagnostic::new(code::MISSING_TYPE, "this definition has no `type`")
                .at(cx.sources, brace(object))
                .suggest("add \"type\": the definition type, such as \"material\"");
            cx.report(diagnostic);
            return None;
        }
    };
    if !cx.type_names.contains(&type_name) {
        if cx.skip_unknown_types {
            return None;
        }
        let suggestion = did_you_mean(closest(type_name, cx.type_names.iter().copied()));
        let span = object.get("type").map_or(object.span, |node| node.span);
        let diagnostic = Diagnostic::new(
            code::UNKNOWN_TYPE,
            format!("unknown definition type \"{type_name}\""),
        )
        .at(cx.sources, span)
        .suggest_opt(suggestion);
        cx.report(diagnostic);
        return None;
    }
    let (id_text, id_span) = match object.get("id") {
        Some(node) => match node.as_str() {
            Some(text) => (text, node.span),
            None => {
                let diagnostic = Diagnostic::new(
                    code::MISSING_ID,
                    format!("`id` must be a string, but this is {}", node.kind_name()),
                )
                .at(cx.sources, node.span);
                cx.report(diagnostic);
                return None;
            }
        },
        None => {
            let diagnostic =
                Diagnostic::new(code::MISSING_ID, format!("this {type_name} has no `id`"))
                    .at(cx.sources, brace(object))
                    .suggest("add \"id\": a name in lowercase letters, digits and underscores");
            cx.report(diagnostic);
            return None;
        }
    };
    let own = cx.mods.mods[mod_index].manifest.id.as_str();
    let Some((namespace, name)) = split_id(id_text) else {
        let diagnostic = Diagnostic::new(
            code::BAD_ID,
            format!("\"{id_text}\" is not a valid id"),
        )
        .at(cx.sources, id_span)
        .suggest("an id is `name` or `namespace:name`, in lowercase letters, digits and underscores, each starting with a letter");
        cx.report(diagnostic);
        return None;
    };
    let namespace = namespace.unwrap_or(own);
    let id = format!("{namespace}:{name}");
    let label = DefLabel::new(type_name, id.clone());

    let mut broken = false;
    let mut flag = |key: &str, cx: &mut MergeCx<'_>| -> bool {
        match object.get(key) {
            None => false,
            Some(node) => node.as_bool().unwrap_or_else(|| {
                cx.bad_directive(
                    node,
                    Some(&label),
                    format!(
                        "`{key}` is `true` or `false`, but this is {}",
                        node.kind_name()
                    ),
                );
                broken = true;
                false
            }),
        }
    };
    let is_patch = flag("patch", cx);
    let is_override = flag("override", cx);
    let optional = flag("optional", cx);
    flag("abstract", cx);
    flag("obsolete", cx);

    if is_patch && is_override {
        let node = object.get("override").unwrap_or(object);
        cx.bad_directive(
            node,
            Some(&label),
            "a definition cannot be both a patch and an override".to_owned(),
        );
        // There is no telling which was meant, so it is neither.
        return None;
    }
    if optional && !is_patch {
        let node = object.get("optional").unwrap_or(object);
        cx.bad_directive(
            node,
            Some(&label),
            "`optional` only means something together with `\"patch\": true`".to_owned(),
        );
        broken = true;
    }
    if let Some(node) = object.get("copy_from")
        && node.as_str().is_none()
    {
        cx.bad_directive(
            node,
            Some(&label),
            format!("`copy_from` is an id, but this is {}", node.kind_name()),
        );
        broken = true;
    }
    for key in ["extend", "delete", "scale", "add"] {
        if let Some(node) = object.get(key)
            && node.as_object().is_none()
        {
            cx.bad_directive(
                node,
                Some(&label),
                format!(
                    "`{key}` is an object whose keys are field paths, but this is {}",
                    node.kind_name()
                ),
            );
            broken = true;
        }
    }

    if namespace == ENGINE_NAMESPACE {
        let diagnostic = Diagnostic::new(
            code::FOREIGN_NAMESPACE,
            format!("the namespace \"{ENGINE_NAMESPACE}\" is reserved for the engine"),
        )
        .at(cx.sources, id_span)
        .in_def(Some(&label));
        cx.report(diagnostic);
        return None;
    }
    if namespace != own && !is_patch && !is_override {
        let diagnostic = Diagnostic::new(
            code::FOREIGN_NAMESPACE,
            format!(
                "mod \"{own}\" cannot define \"{id}\": that id is in the namespace of \"{namespace}\""
            ),
        )
        .at(cx.sources, id_span)
        .in_def(Some(&label))
        .suggest("to change an existing definition add \"patch\": true; to replace it add \"override\": true");
        cx.report(diagnostic);
        return None;
    }

    let kind = if is_override && !is_patch {
        RawKind::Override
    } else if is_patch {
        RawKind::Patch { optional }
    } else {
        RawKind::Original
    };
    Some(RawDef {
        type_name: type_name.to_owned(),
        id,
        id_span,
        mod_index,
        kind,
        node: object.clone(),
        broken,
    })
}

/// One object of a fold: the original, an override, or a patch.
#[derive(Clone, Debug)]
struct Layer {
    node: Node,
    mod_index: usize,
    is_patch: bool,
}

/// A definition after folding: its layers in the order they apply.
#[derive(Clone, Debug)]
struct Authored {
    id_span: Span,
    layers: Vec<Layer>,
    broken: bool,
}

impl Authored {
    /// The value of a directive: the last layer that gives it wins.
    fn directive(&self, key: &str) -> Option<&Node> {
        self.layers
            .iter()
            .rev()
            .find_map(|layer| layer.node.get(key))
    }

    fn flag(&self, key: &str) -> bool {
        self.directive(key).and_then(Node::as_bool).unwrap_or(false)
    }
}

/// Stage 2: folds the collected objects by `(type, id)` in load order.
fn fold(raws: Vec<RawDef>, cx: &mut MergeCx<'_>) -> BTreeMap<Key, Authored> {
    // Where each id is first defined, so that a patch that comes too
    // early can say so.
    let mut defined_at: BTreeMap<Key, Span> = BTreeMap::new();
    for raw in &raws {
        if raw.kind == RawKind::Original {
            defined_at
                .entry((raw.type_name.clone(), raw.id.clone()))
                .or_insert(raw.id_span);
        }
    }
    let mut out: BTreeMap<Key, Authored> = BTreeMap::new();
    for raw in raws {
        let key = (raw.type_name.clone(), raw.id.clone());
        let label = DefLabel::new(raw.type_name.clone(), raw.id.clone());
        let layer = Layer {
            node: raw.node,
            mod_index: raw.mod_index,
            is_patch: matches!(raw.kind, RawKind::Patch { .. }),
        };
        match (raw.kind, out.get_mut(&key)) {
            (RawKind::Original, None) => {
                out.insert(
                    key,
                    Authored {
                        id_span: raw.id_span,
                        layers: vec![layer],
                        broken: raw.broken,
                    },
                );
            }
            (RawKind::Original, Some(first)) => {
                let diagnostic = Diagnostic::new(
                    code::DUPLICATE_ID,
                    format!("{} \"{}\" is already defined", raw.type_name, raw.id),
                )
                .at(cx.sources, raw.id_span)
                .in_def(Some(&label))
                .suggest("to change it add \"patch\": true; to replace it add \"override\": true")
                .note_at(cx.sources, first.id_span, "first defined here");
                cx.report(diagnostic);
            }
            (RawKind::Override, Some(existing)) => {
                // An override restarts the fold: earlier patches are gone.
                existing.id_span = raw.id_span;
                existing.layers = vec![layer];
                existing.broken = raw.broken;
            }
            (RawKind::Patch { .. }, Some(existing)) => {
                existing.layers.push(layer);
                existing.broken |= raw.broken;
            }
            (RawKind::Patch { optional: true }, None) => {}
            (RawKind::Patch { optional: false } | RawKind::Override, None) => {
                let (code, verb) = if raw.kind == RawKind::Override {
                    (code::BAD_OVERRIDE_TARGET, "override")
                } else {
                    (code::BAD_PATCH_TARGET, "patch")
                };
                let mut diagnostic = Diagnostic::new(
                    code,
                    format!(
                        "nothing to {verb}: {} \"{}\" is not defined before this",
                        raw.type_name, raw.id
                    ),
                )
                .at(cx.sources, raw.id_span)
                .in_def(Some(&label));
                diagnostic = match defined_at.get(&key) {
                    Some(&later) => diagnostic
                        .suggest(
                            "list the defining mod in `requires` or `after` so that it loads first",
                        )
                        .note_at(cx.sources, later, "it is defined later, here"),
                    None => {
                        let names: Vec<&str> = defined_at
                            .keys()
                            .filter(|(type_name, _)| *type_name == raw.type_name)
                            .map(|(_, id)| id.as_str())
                            .collect();
                        let mut diagnostic =
                            diagnostic.suggest_opt(did_you_mean(closest(&raw.id, names)));
                        if raw.kind != RawKind::Override && diagnostic.suggestion.is_none() {
                            diagnostic =
                                diagnostic.suggest("if it may be absent, add \"optional\": true");
                        }
                        diagnostic
                    }
                };
                cx.report(diagnostic);
            }
        }
    }
    out
}

/// What merging produced.
pub(crate) struct Merged {
    /// Every definition, by type and id, abstract ones included.
    pub(crate) finished: BTreeMap<Key, Finished>,
    /// The abstract definitions something inherits from.
    pub(crate) used_parents: BTreeSet<Key>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Visit {
    Unvisited,
    InProgress,
    Done,
}

/// Stages 2 and 3: folds, then resolves inheritance, parents first.
pub(crate) fn merge(raws: Vec<RawDef>, cx: &mut MergeCx<'_>) -> Merged {
    let authored = fold(raws, cx);

    // Resolve every `copy_from` to a key before finishing anything.
    let mut parents: BTreeMap<Key, Option<Key>> = BTreeMap::new();
    let mut broken: BTreeSet<Key> = BTreeSet::new();
    let mut used_parents: BTreeSet<Key> = BTreeSet::new();
    for (key, definition) in &authored {
        if definition.broken {
            broken.insert(key.clone());
        }
        let parent = match definition.directive("copy_from") {
            Some(node) => match resolve_parent(key, node, &authored, cx) {
                Some(parent) => {
                    used_parents.insert(parent.clone());
                    Some(parent)
                }
                None => {
                    broken.insert(key.clone());
                    None
                }
            },
            None => None,
        };
        parents.insert(key.clone(), parent);
    }

    let mut state: BTreeMap<&Key, Visit> =
        authored.keys().map(|key| (key, Visit::Unvisited)).collect();
    let mut trees: BTreeMap<Key, Option<Node>> = BTreeMap::new();
    for start in authored.keys() {
        if state[start] != Visit::Unvisited {
            continue;
        }
        // An explicit stack: a chain of parents may be as long as the
        // mods care to make it.
        let mut stack: Vec<&Key> = vec![start];
        while let Some(&top) = stack.last() {
            let parent = parents[top].as_ref();
            if state[top] == Visit::Unvisited {
                state.insert(top, Visit::InProgress);
                if let Some(parent) = parent {
                    let (parent_key, _) = authored
                        .get_key_value(parent)
                        .expect("a resolved parent is an authored definition");
                    match state[parent_key] {
                        Visit::Unvisited => {
                            stack.push(parent_key);
                            continue;
                        }
                        Visit::InProgress => {
                            let from = stack.iter().position(|key| *key == parent_key).unwrap_or(0);
                            let mut names: Vec<&str> =
                                stack[from..].iter().map(|key| key.1.as_str()).collect();
                            names.push(&parent_key.1);
                            let label = DefLabel::new(top.0.clone(), top.1.clone());
                            let span = authored[top]
                                .directive("copy_from")
                                .map_or(authored[top].id_span, |node| node.span);
                            let diagnostic = Diagnostic::new(
                                code::INHERITANCE_CYCLE,
                                format!(
                                    "definitions inherit from each other in a circle: {}",
                                    names.join(" -> ")
                                ),
                            )
                            .at(cx.sources, span)
                            .in_def(Some(&label));
                            cx.report(diagnostic);
                            broken.insert(top.clone());
                        }
                        Visit::Done => {}
                    }
                }
            }
            // The parent, if any, is finished (or this is broken).
            let tree = if broken.contains(top) {
                None
            } else {
                match parent {
                    None => finish_one(top, &authored[top], None, cx),
                    Some(parent) => match trees.get(parent).and_then(Option::as_ref) {
                        Some(base) => finish_one(top, &authored[top], Some(base), cx),
                        // The parent is broken, and that has been reported.
                        None => None,
                    },
                }
            };
            trees.insert(top.clone(), tree);
            state.insert(top, Visit::Done);
            stack.pop();
        }
    }

    let mut finished = BTreeMap::new();
    for (key, definition) in &authored {
        finished.insert(
            key.clone(),
            Finished {
                type_name: key.0.clone(),
                id: key.1.clone(),
                id_span: definition.id_span,
                tree: trees.remove(key).flatten(),
                is_abstract: definition.flag("abstract"),
                obsolete: definition.flag("obsolete"),
                inline: false,
            },
        );
    }
    Merged {
        finished,
        used_parents,
    }
}

fn resolve_parent(
    key: &Key,
    node: &Node,
    authored: &BTreeMap<Key, Authored>,
    cx: &mut MergeCx<'_>,
) -> Option<Key> {
    let text = node.as_str()?;
    let label = DefLabel::new(key.0.clone(), key.1.clone());
    let Some((namespace, name)) = split_id(text) else {
        let diagnostic = Diagnostic::new(code::BAD_ID, format!("\"{text}\" is not a valid id"))
            .at(cx.sources, node.span)
            .in_def(Some(&label))
            .with_path("copy_from");
        cx.report(diagnostic);
        return None;
    };
    let candidates: Vec<String> = match namespace {
        Some(namespace) => vec![format!("{namespace}:{name}")],
        None => cx
            .mods
            .search_order(node.span.file)
            .into_iter()
            .map(|namespace| format!("{namespace}:{name}"))
            .collect(),
    };
    for candidate in &candidates {
        let parent = (key.0.clone(), candidate.clone());
        if authored.contains_key(&parent) {
            return Some(parent);
        }
    }
    // Not of this type. Of another?
    let other_type = candidates.iter().find_map(|candidate| {
        authored
            .keys()
            .find(|(type_name, id)| id == candidate && *type_name != key.0)
            .map(|(type_name, id)| (type_name.as_str(), id.as_str()))
    });
    let diagnostic = match other_type {
        Some((other, id)) => Diagnostic::new(
            code::PARENT_TYPE_MISMATCH,
            format!(
                "\"{id}\" is a {other}, and a {} can only copy from a {}",
                key.0, key.0
            ),
        ),
        None => {
            let own = cx
                .mods
                .manifest_of(node.span.file)
                .map(|manifest| manifest.id.as_str());
            let suggestion = suggest_id(
                text,
                authored
                    .keys()
                    .filter(|(type_name, id)| *type_name == key.0 && *id != key.1)
                    .map(|(_, id)| id.as_str()),
                own,
            );
            Diagnostic::new(
                code::UNKNOWN_PARENT,
                format!("no {} \"{text}\" to copy from", key.0),
            )
            .suggest_opt(did_you_mean(suggestion.as_deref()))
        }
    };
    let diagnostic = diagnostic
        .at(cx.sources, node.span)
        .in_def(Some(&label))
        .with_path("copy_from");
    cx.report(diagnostic);
    None
}

/// The id a reference that resolves to nothing probably meant: the
/// closest existing id of that type (04 §4.3).
///
/// A bare name is compared with the name part of every id, so that a
/// name that exists in a namespace the mod does not import is found. The
/// suggestion is written the way the mod would have to write it: bare
/// for an id in its own namespace, qualified otherwise.
pub(crate) fn suggest_id<'a>(
    text: &str,
    ids: impl Iterator<Item = &'a str>,
    own_namespace: Option<&str>,
) -> Option<String> {
    if text.contains(':') {
        return closest(text, ids).map(str::to_owned);
    }
    let limit = (text.chars().count() / 3).max(1);
    let mut best: Option<(usize, &str)> = None;
    for id in ids {
        let Some((_, name)) = id.split_once(':') else {
            continue;
        };
        // Generated ids of inline definitions cannot be referred to.
        if name.contains('#') {
            continue;
        }
        let distance = crate::diag::edit_distance(text, name);
        if distance > limit {
            continue;
        }
        let better = match best {
            None => true,
            Some((best_distance, best_id)) => {
                distance < best_distance || (distance == best_distance && id < best_id)
            }
        };
        if better {
            best = Some((distance, id));
        }
    }
    let (_, id) = best?;
    match id.split_once(':') {
        Some((namespace, name)) if Some(namespace) == own_namespace => Some(name.to_owned()),
        _ => Some(id.to_owned()),
    }
}

/// A `scale` or `add` waiting for the final value (04 §6.1).
struct Pending {
    path: Vec<String>,
    key_span: Span,
    operand: Node,
}

/// The state of finishing one definition.
struct Finisher<'a, 'b> {
    cx: &'a mut MergeCx<'b>,
    label: DefLabel,
    failed: bool,
    /// The last patch to replace each field: the mod and the value.
    replaced: BTreeMap<String, (usize, Node)>,
    scales: Vec<Pending>,
    adds: Vec<Pending>,
}

/// Stage 3 for one definition: its parent's finished tree, or nothing,
/// with each layer applied as a change.
fn finish_one(
    key: &Key,
    definition: &Authored,
    base: Option<&Node>,
    cx: &mut MergeCx<'_>,
) -> Option<Node> {
    let mut tree = match base {
        Some(base) => base.clone(),
        None => Node::empty_object(definition.id_span),
    };
    // The definition is where its first object is, whatever it inherits.
    if let Some(first) = definition.layers.first() {
        tree.span = first.node.span;
    }
    let mut finisher = Finisher {
        cx,
        label: DefLabel::new(key.0.clone(), key.1.clone()),
        failed: false,
        replaced: BTreeMap::new(),
        scales: Vec::new(),
        adds: Vec::new(),
    };
    for layer in &definition.layers {
        finisher.apply_layer(&mut tree, layer);
    }
    let scales = std::mem::take(&mut finisher.scales);
    for pending in &scales {
        finisher.apply_number(&mut tree, pending, true, &key.0);
    }
    let adds = std::mem::take(&mut finisher.adds);
    for pending in &adds {
        finisher.apply_number(&mut tree, pending, false, &key.0);
    }
    (!finisher.failed).then_some(tree)
}

fn is_id_list(items: &[Node]) -> bool {
    !items.is_empty() && items.iter().all(|item| entry_id(item).is_some())
}

fn entry_id(item: &Node) -> Option<&str> {
    item.as_object()?;
    item.get("id")?.as_str()
}

/// Why a path could not be followed.
enum StepError {
    Missing,
    NotContainer(&'static str),
}

/// One step of a dotted path: a key of an object, or the id of an entry
/// in a list of objects that carry ids.
fn step<'n>(node: &'n mut Node, segment: &str) -> Result<&'n mut Node, StepError> {
    let kind_name = node.kind_name();
    match &mut node.kind {
        NodeKind::Object(members) => members
            .iter_mut()
            .find(|member| member.key == segment)
            .map(|member| &mut member.value)
            .ok_or(StepError::Missing),
        NodeKind::Array(items) if is_id_list(items) => items
            .iter_mut()
            .find(|item| entry_id(item) == Some(segment))
            .ok_or(StepError::Missing),
        _ => Err(StepError::NotContainer(kind_name)),
    }
}

/// A number a `scale` or `add` can act on.
#[derive(Clone, Copy)]
enum Numeric {
    Int(i64),
    Float(f64),
    Quantity(f64, Dim),
}

impl Numeric {
    /// Reads a value as a number, converting one written with a unit
    /// (04 §6.1). `difference` drops the offset of degrees Celsius: an
    /// amount to add is a difference, not a temperature.
    fn of(node: &Node, difference: bool) -> Option<Numeric> {
        match &node.kind {
            NodeKind::Number(Num::Int(value)) => Some(Numeric::Int(*value)),
            NodeKind::Number(number) => Some(Numeric::Float(number.as_f64())),
            NodeKind::Quantity { value, dim } => Some(Numeric::Quantity(*value, *dim)),
            NodeKind::String(text) => {
                let quantity = parse_quantity(text).ok()?;
                let offset = match (difference, quantity.unit) {
                    (true, Some(unit)) if !quantity.dim.per_time => unit.offset,
                    _ => 0.0,
                };
                Some(Numeric::Quantity(quantity.value - offset, quantity.dim))
            }
            _ => None,
        }
    }

    fn value(self) -> f64 {
        match self {
            Numeric::Int(value) => value as f64,
            Numeric::Float(value) | Numeric::Quantity(value, _) => value,
        }
    }

    fn into_kind(self) -> Option<NodeKind> {
        match self {
            Numeric::Int(value) => Some(NodeKind::Number(Num::Int(value))),
            Numeric::Float(value) => value
                .is_finite()
                .then_some(NodeKind::Number(Num::Float(value))),
            Numeric::Quantity(value, dim) => value
                .is_finite()
                .then_some(NodeKind::Quantity { value, dim }),
        }
    }
}

impl Finisher<'_, '_> {
    fn error(&mut self, diagnostic: Diagnostic, path: &[String]) {
        self.failed = true;
        let diagnostic = diagnostic
            .in_def(Some(&self.label))
            .with_path(path.join("."));
        self.cx.report(diagnostic);
    }

    fn apply_layer(&mut self, tree: &mut Node, layer: &Layer) {
        let Some(members) = layer.node.as_object() else {
            return;
        };
        let mut path: Vec<String> = Vec::new();
        for member in members {
            if DIRECTIVE_KEYS.contains(&member.key.as_str()) {
                continue;
            }
            self.merge_member(tree, member, &mut path, layer);
        }
        if let Some(delete) = layer.node.get("delete") {
            for member in delete.as_object().unwrap_or(&[]) {
                self.apply_delete(tree, member);
            }
        }
        if let Some(extend) = layer.node.get("extend") {
            for member in extend.as_object().unwrap_or(&[]) {
                self.apply_extend(tree, member, layer);
            }
        }
        for (key, is_scale) in [("scale", true), ("add", false)] {
            let Some(node) = layer.node.get(key) else {
                continue;
            };
            for member in node.as_object().unwrap_or(&[]) {
                let pending = Pending {
                    path: member.key.split('.').map(str::to_owned).collect(),
                    key_span: member.key_span,
                    operand: member.value.clone(),
                };
                if is_scale {
                    self.scales.push(pending);
                } else {
                    self.adds.push(pending);
                }
            }
        }
    }

    /// Notes that `layer` set the value at `path`, and warns if another
    /// mod's patch set it to something else before (04 §6.3).
    fn record_replace(&mut self, path: &[String], value: &Node, layer: &Layer) {
        if !layer.is_patch {
            return;
        }
        if let NodeKind::Object(members) = &value.kind {
            let mut path = path.to_vec();
            for member in members {
                path.push(member.key.clone());
                self.record_replace(&path, &member.value, layer);
                path.pop();
            }
            return;
        }
        let shown = path.join(".");
        if let Some((earlier, earlier_value)) = self.replaced.get(&shown)
            && *earlier != layer.mod_index
            && !earlier_value.same_value(value)
        {
            let mods = &self.cx.mods.mods;
            let earlier_id = mods[*earlier].manifest.id.as_str();
            let later = &mods[layer.mod_index].manifest;
            if !later.knows_of(earlier_id) {
                let diagnostic = Diagnostic::new(
                    code::DISPUTED_FIELD,
                    format!(
                        "mods \"{earlier_id}\" and \"{}\" both replace `{shown}` of {}; \"{}\" wins",
                        later.id, self.label, later.id
                    ),
                )
                .at(self.cx.sources, value.span)
                .in_def(Some(&self.label))
                .with_path(shown.clone())
                .suggest(format!(
                    "if \"{}\" means to win, list \"{earlier_id}\" in its `requires` or `after`",
                    later.id
                ))
                .note_at(
                    self.cx.sources,
                    earlier_value.span,
                    format!("\"{earlier_id}\" set it here"),
                );
                self.cx.report(diagnostic);
            }
        }
        self.replaced
            .insert(shown, (layer.mod_index, value.clone()));
    }

    fn merge_member(
        &mut self,
        target: &mut Node,
        member: &Member,
        path: &mut Vec<String>,
        layer: &Layer,
    ) {
        path.push(member.key.clone());
        let existing = target.as_object_mut().and_then(|members| {
            members
                .iter_mut()
                .find(|existing| existing.key == member.key)
        });
        match existing {
            Some(existing) => self.merge_into(&mut existing.value, &member.value, path, layer),
            None => {
                self.record_replace(path, &member.value, layer);
                if let Some(members) = target.as_object_mut() {
                    // Keys added by a change come after those already
                    // there (04 §5).
                    members.push(member.clone());
                }
            }
        }
        path.pop();
    }

    fn merge_into(
        &mut self,
        target: &mut Node,
        change: &Node,
        path: &mut Vec<String>,
        layer: &Layer,
    ) {
        match (&mut target.kind, &change.kind) {
            (NodeKind::Object(_), NodeKind::Object(members)) => {
                for member in members {
                    self.merge_member(target, member, path, layer);
                }
            }
            (NodeKind::Array(items), NodeKind::Array(entries))
                if is_id_list(items) && is_id_list(entries) =>
            {
                self.merge_id_list(items, entries, path, layer);
            }
            _ => {
                self.record_replace(path, change, layer);
                *target = change.clone();
            }
        }
    }

    /// Merges entries into a list of objects that carry ids: an entry
    /// with a known id changes that entry; a new one is appended, or put
    /// `before` or `after` the entry it names.
    fn merge_id_list(
        &mut self,
        items: &mut Vec<Node>,
        entries: &[Node],
        path: &mut Vec<String>,
        layer: &Layer,
    ) {
        for entry in entries {
            let Some(id) = entry_id(entry).map(str::to_owned) else {
                continue;
            };
            let mut clean = entry.clone();
            let mut anchor: Option<(bool, Node)> = None;
            if let Some(members) = clean.as_object_mut() {
                members.retain(|member| match member.key.as_str() {
                    "before" => {
                        anchor = Some((true, member.value.clone()));
                        false
                    }
                    "after" => {
                        anchor = Some((false, member.value.clone()));
                        false
                    }
                    _ => true,
                });
            }
            path.push(id.clone());
            let position = items.iter().position(|item| entry_id(item) == Some(&id));
            let moving = match position {
                Some(position) => {
                    self.merge_into(&mut items[position], &clean, path, layer);
                    anchor.is_some().then(|| items.remove(position))
                }
                None => {
                    self.record_replace(path, &clean, layer);
                    Some(clean)
                }
            };
            if let Some(moving) = moving {
                match &anchor {
                    None => items.push(moving),
                    Some((before, anchor_node)) => {
                        let at = anchor_node.as_str().and_then(|anchor_id| {
                            items
                                .iter()
                                .position(|item| entry_id(item) == Some(anchor_id))
                        });
                        match at {
                            Some(at) => items.insert(if *before { at } else { at + 1 }, moving),
                            None => {
                                let shown = anchor_node.as_str().map_or_else(
                                    || anchor_node.kind_name().to_owned(),
                                    |text| format!("\"{text}\""),
                                );
                                let known: Vec<&str> = items.iter().filter_map(entry_id).collect();
                                let suggestion = anchor_node
                                    .as_str()
                                    .and_then(|text| did_you_mean(closest(text, known)));
                                let diagnostic = Diagnostic::new(
                                    code::BAD_ANCHOR,
                                    format!(
                                        "no entry {shown} to put \"{id}\" {}",
                                        if *before { "before" } else { "after" }
                                    ),
                                )
                                .at(self.cx.sources, anchor_node.span)
                                .suggest_opt(suggestion);
                                self.error(diagnostic, path);
                                items.push(moving);
                            }
                        }
                    }
                }
            }
            path.pop();
        }
    }

    /// Reports a dotted path that could not be followed at step `index`.
    fn path_error(
        &mut self,
        path: &[String],
        index: usize,
        error: StepError,
        key_span: Span,
        what: &str,
    ) {
        let full = path.join(".");
        let so_far = path[..index].join(".");
        let segment = &path[index];
        let message = match error {
            StepError::Missing if index == 0 => {
                format!("cannot {what} `{full}`: there is no `{segment}` here")
            }
            StepError::Missing => {
                format!("cannot {what} `{full}`: `{so_far}` has no `{segment}`")
            }
            StepError::NotContainer(kind) => format!(
                "cannot {what} `{full}`: `{so_far}` is {kind}, which a path cannot step into"
            ),
        };
        let diagnostic = Diagnostic::new(code::BAD_PATH, message).at(self.cx.sources, key_span);
        self.error(diagnostic, path);
    }

    /// Follows the first `steps` steps of a dotted path.
    fn walk<'n>(
        &mut self,
        tree: &'n mut Node,
        path: &[String],
        steps: usize,
        key_span: Span,
        what: &str,
    ) -> Option<&'n mut Node> {
        let mut current = tree;
        for (index, segment) in path.iter().enumerate().take(steps) {
            match step(current, segment) {
                Ok(next) => current = next,
                Err(error) => {
                    self.path_error(path, index, error, key_span, what);
                    return None;
                }
            }
        }
        Some(current)
    }

    fn apply_delete(&mut self, tree: &mut Node, member: &Member) {
        let path: Vec<String> = member.key.split('.').map(str::to_owned).collect();
        let Some(removals) = member.value.as_array() else {
            let diagnostic = Diagnostic::new(
                code::BAD_DELETE,
                format!(
                    "`delete` takes a list of what to remove from `{}`, but this is {}",
                    member.key,
                    member.value.kind_name()
                ),
            )
            .at(self.cx.sources, member.value.span);
            self.error(diagnostic, &path);
            return;
        };
        let Some(target) = self.walk(tree, &path, path.len(), member.key_span, "delete from")
        else {
            return;
        };
        let target_kind = target.kind_name();
        let mut problems: Vec<Diagnostic> = Vec::new();
        match &mut target.kind {
            NodeKind::Array(items) => {
                let by_id = is_id_list(items);
                for removal in removals {
                    let position = if by_id {
                        removal
                            .as_str()
                            .and_then(|id| items.iter().position(|item| entry_id(item) == Some(id)))
                    } else {
                        items.iter().position(|item| item.same_value(removal))
                    };
                    match position {
                        Some(position) => {
                            items.remove(position);
                        }
                        None => {
                            let known: Vec<&str> = if by_id {
                                items.iter().filter_map(entry_id).collect()
                            } else {
                                items.iter().filter_map(Node::as_str).collect()
                            };
                            let suggestion = removal
                                .as_str()
                                .and_then(|text| did_you_mean(closest(text, known)));
                            problems.push(
                                Diagnostic::new(
                                    code::BAD_DELETE,
                                    format!(
                                        "cannot delete {} from `{}`: it is not there",
                                        removal.to_json(),
                                        member.key
                                    ),
                                )
                                .at(self.cx.sources, removal.span)
                                .suggest_opt(suggestion),
                            );
                        }
                    }
                }
            }
            NodeKind::Object(members) => {
                for removal in removals {
                    let position = removal
                        .as_str()
                        .and_then(|key| members.iter().position(|existing| existing.key == key));
                    match position {
                        Some(position) => {
                            members.remove(position);
                        }
                        None => {
                            let known: Vec<&str> = members
                                .iter()
                                .map(|existing| existing.key.as_str())
                                .collect();
                            let suggestion = removal
                                .as_str()
                                .and_then(|text| did_you_mean(closest(text, known)));
                            problems.push(
                                Diagnostic::new(
                                    code::BAD_DELETE,
                                    format!(
                                        "cannot delete key {} from `{}`: it is not there",
                                        removal.to_json(),
                                        member.key
                                    ),
                                )
                                .at(self.cx.sources, removal.span)
                                .suggest_opt(suggestion),
                            );
                        }
                    }
                }
            }
            _ => problems.push(
                Diagnostic::new(
                    code::BAD_DELETE,
                    format!(
                        "cannot delete from `{}`: it is {target_kind}, not a list or a map",
                        member.key
                    ),
                )
                .at(self.cx.sources, member.key_span),
            ),
        }
        for problem in problems {
            self.error(problem, &path);
        }
    }

    fn apply_extend(&mut self, tree: &mut Node, member: &Member, layer: &Layer) {
        let mut path: Vec<String> = member.key.split('.').map(str::to_owned).collect();
        if !matches!(member.value.kind, NodeKind::Array(_) | NodeKind::Object(_)) {
            let diagnostic = Diagnostic::new(
                code::BAD_EXTEND,
                format!(
                    "`extend` takes a list to append or a map to merge into `{}`, but this is {}",
                    member.key,
                    member.value.kind_name()
                ),
            )
            .at(self.cx.sources, member.value.span);
            self.error(diagnostic, &path);
            return;
        }
        let Some(last) = path.last().cloned() else {
            return;
        };
        let last_index = path.len() - 1;
        let Some(parent) = self.walk(tree, &path, last_index, member.key_span, "extend") else {
            return;
        };
        // A list or map that is not there yet starts empty.
        if let NodeKind::Object(members) = &mut parent.kind
            && !members.iter().any(|existing| existing.key == last)
        {
            let empty = match member.value.kind {
                NodeKind::Array(_) => NodeKind::Array(Vec::new()),
                _ => NodeKind::Object(Vec::new()),
            };
            members.push(Member {
                key: last.clone(),
                key_span: member.key_span,
                value: Node::new(empty, member.value.span),
            });
        }
        let target = match step(parent, &last) {
            Ok(target) => target,
            Err(error) => {
                self.path_error(&path, last_index, error, member.key_span, "extend");
                return;
            }
        };
        let target_kind = target.kind_name();
        match (&mut target.kind, &member.value.kind) {
            (NodeKind::Array(items), NodeKind::Array(additions)) => {
                if is_id_list(additions) && (items.is_empty() || is_id_list(items)) {
                    self.merge_id_list(items, additions, &mut path, layer);
                } else {
                    items.extend(additions.iter().cloned());
                }
            }
            (NodeKind::Object(_), NodeKind::Object(_)) => {
                self.merge_into(target, &member.value, &mut path, layer);
            }
            _ => {
                let diagnostic = Diagnostic::new(
                    code::BAD_EXTEND,
                    format!(
                        "cannot extend `{}` with {}: it is {target_kind}",
                        member.key,
                        member.value.kind_name()
                    ),
                )
                .at(self.cx.sources, member.value.span)
                .suggest("give a list to append to a list, or an object to merge into a map");
                self.error(diagnostic, &path);
            }
        }
    }

    /// Applies one pending `scale` or `add` to the final value.
    fn apply_number(
        &mut self,
        tree: &mut Node,
        pending: &Pending,
        is_scale: bool,
        type_name: &str,
    ) {
        let (code, verb) = if is_scale {
            (code::BAD_SCALE, "scale")
        } else {
            (code::BAD_ADD, "add to")
        };
        let path = &pending.path;
        let shown = path.join(".");
        let operand = if is_scale {
            match &pending.operand.kind {
                NodeKind::Number(number) => Some(match number {
                    Num::Int(value) => Numeric::Int(*value),
                    other => Numeric::Float(other.as_f64()),
                }),
                _ => None,
            }
        } else {
            Numeric::of(&pending.operand, true)
        };
        let Some(operand) = operand else {
            let message = if is_scale {
                format!(
                    "`scale` multiplies `{shown}` by a number, but this is {}",
                    pending.operand.kind_name()
                )
            } else {
                format!(
                    "`add` adds a number or a quantity with a unit to `{shown}`, but this is {}",
                    pending.operand.to_json()
                )
            };
            let diagnostic =
                Diagnostic::new(code, message).at(self.cx.sources, pending.operand.span);
            self.error(diagnostic, path);
            return;
        };

        let Some(last) = path.last() else {
            return;
        };
        let last_index = path.len() - 1;
        let Some(parent) = self.walk(tree, path, last_index, pending.key_span, verb) else {
            return;
        };
        // On a field the definition leaves out, act on the type's default.
        if let NodeKind::Object(members) = &mut parent.kind
            && !members.iter().any(|existing| existing.key == *last)
        {
            let default = self.cx.defaults.get(type_name).and_then(|defaults| {
                path.iter()
                    .try_fold(defaults, |node, segment| node.get(segment))
            });
            match default {
                Some(default) => members.push(Member {
                    key: last.clone(),
                    key_span: pending.key_span,
                    value: default.clone(),
                }),
                None => {
                    let diagnostic = Diagnostic::new(
                        code,
                        format!(
                            "cannot {verb} `{shown}`: it has no value here and the type declares no default for it"
                        ),
                    )
                    .at(self.cx.sources, pending.key_span)
                    .suggest("write the value in full");
                    self.error(diagnostic, path);
                    return;
                }
            }
        }
        let target = match step(parent, last) {
            Ok(target) => target,
            Err(error) => {
                self.path_error(path, last_index, error, pending.key_span, verb);
                return;
            }
        };
        let Some(current) = Numeric::of(target, false) else {
            let diagnostic = Diagnostic::new(
                code,
                format!(
                    "cannot {verb} `{shown}`: it is {}, not a number",
                    target.kind_name()
                ),
            )
            .at(self.cx.sources, pending.key_span)
            .note_at(self.cx.sources, target.span, "the value is set here");
            self.error(diagnostic, path);
            return;
        };
        let result = if is_scale {
            match (current, operand) {
                (Numeric::Int(a), Numeric::Int(b)) => match a.checked_mul(b) {
                    Some(product) => Numeric::Int(product),
                    None => Numeric::Float(a as f64 * b as f64),
                },
                (Numeric::Quantity(value, dim), factor) => {
                    Numeric::Quantity(value * factor.value(), dim)
                }
                (value, factor) => Numeric::Float(value.value() * factor.value()),
            }
        } else {
            match (current, operand) {
                (Numeric::Int(a), Numeric::Int(b)) => match a.checked_add(b) {
                    Some(sum) => Numeric::Int(sum),
                    None => Numeric::Float(a as f64 + b as f64),
                },
                (Numeric::Quantity(a, dim), Numeric::Quantity(b, other)) => {
                    if dim != other {
                        let diagnostic = Diagnostic::new(
                            code,
                            format!(
                                "cannot add {} to `{shown}`, which is {}",
                                other.name(),
                                dim.name()
                            ),
                        )
                        .at(self.cx.sources, pending.operand.span)
                        .note_at(
                            self.cx.sources,
                            target.span,
                            "the value is set here",
                        );
                        self.error(diagnostic, path);
                        return;
                    }
                    Numeric::Quantity(a + b, dim)
                }
                (Numeric::Quantity(a, dim), amount) => Numeric::Quantity(a + amount.value(), dim),
                (value, Numeric::Quantity(b, dim)) => Numeric::Quantity(value.value() + b, dim),
                (value, amount) => Numeric::Float(value.value() + amount.value()),
            }
        };
        match result.into_kind() {
            Some(kind) => {
                // The value now comes from whoever changed it last.
                *target = Node::new(kind, pending.operand.span);
            }
            None => {
                let diagnostic = Diagnostic::new(
                    code,
                    format!("cannot {verb} `{shown}`: the result is too large to represent"),
                )
                .at(self.cx.sources, pending.operand.span);
                self.error(diagnostic, path);
            }
        }
    }
}
