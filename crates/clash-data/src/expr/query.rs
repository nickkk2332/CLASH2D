//! Queries (05 §11) in expression fields: how one is compiled where it
//! is written, and how it is run.
//!
//! A query is an object with one key, the kind of query, whose value
//! holds its arguments and, for every kind, `where`, `sort` and
//! `limit`:
//!
//! ```json
//! { "entities": { "around": "self", "radius": 12,
//!                 "where": "it != self and it.has_tag('guard')",
//!                 "sort": "dist(it, self)", "limit": 3 } }
//! ```
//!
//! Two kinds belong to the language itself and are answered here:
//! `list`, the elements of a list value, and `defs`, the definitions of
//! a type. Every other kind is registered by the host
//! ([`QuerySpec`](clash_expr::QuerySpec)), which gives the candidates in
//! their default order ([`Host::query`]); `where`, `sort` and `limit`
//! are applied here, in that order, so that every kind is filtered,
//! sorted and cut the same way.
//!
//! `where`, `sort`, and the `weight` and `of` of a value source, see the
//! candidate as `it`, a subject bound after everything the field binds.
//! `sort` must be pure. `limit` is worked out once, where the query is
//! written.

use std::cmp::Ordering;
use std::fmt;
use std::sync::Arc;

use clash_expr::{
    DEFAULT_MAX_QUERY_CELLS, DefRef, Fault, Host, QueryCall, QueryId, RuntimeWarning, Span, Type,
    Value,
};
use serde::Deserialize;
use serde::de::{Deserializer, MapAccess, SeqAccess, Visitor};

use super::codes;
use super::eval::Evaluator;
use super::field::{Any, Cond, Expr, Int, Ready};
use super::scope::{CompileCx, Scope, TypeSpec};
use crate::diag::{Diagnostic, Diagnostics, closest, did_you_mean};
use crate::fields::{Located, Site};
use crate::json::{Member, Node, NodeKind, Num};

/// The query that goes through the elements of a list:
/// `{ "list": "$xs" }`, or `{ "list": { "items": "$xs", "where": … } }`.
pub const LIST_QUERY: &str = "list";
/// The query that goes through the definitions of a type:
/// `{ "defs": { "type": "material" } }`.
pub const DEFS_QUERY: &str = "defs";
/// The name the candidate is bound under in `where`, `sort`, `weight`
/// and `of`.
pub const IT: &str = "it";
/// The keys every query takes beside its arguments.
pub const QUERY_KEYS: &[&str] = &["where", "sort", "limit"];

/// Where the candidates of a compiled query come from.
#[derive(Clone, Debug)]
pub(crate) enum Source {
    /// The elements of a list value.
    List(Ready),
    /// The definitions of a type, in order of qualified id.
    Defs(Arc<[Value]>),
    /// The host, with one argument for each the kind declares.
    Host(QueryId, Vec<Option<Ready>>),
}

/// A query compiled where it is written (05 §11): what gives the
/// candidates, and the `where`, `sort` and `limit` applied to them.
///
/// Made by [`compile_query`]; run by [`Evaluator::query`].
#[derive(Clone, Debug)]
pub struct CompiledQuery {
    pub(crate) name: Arc<str>,
    pub(crate) source: Source,
    pub(crate) filter: Option<Cond>,
    pub(crate) sort: Option<Expr<Any>>,
    pub(crate) limit: Option<Expr<Int>>,
    /// The slot `it` was given.
    pub(crate) it: u16,
    pub(crate) element: Type,
    /// In a mind context, the slot of the subject whose belief answers.
    pub(crate) observer: Option<u16>,
    pub(crate) origin: u64,
}

impl CompiledQuery {
    /// The kind of query as written: `entities`, `list`.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// The type of each result.
    pub fn element(&self) -> &Type {
        &self.element
    }

    /// The origin number of the query, for run-time warnings.
    pub fn origin(&self) -> u64 {
        self.origin
    }

    /// Whether results come in an order `sort` gave them.
    pub fn is_sorted(&self) -> bool {
        self.sort.is_some()
    }
}

/// Any JSON value read back into the loader's tree with its positions:
/// how a query written in an expression field keeps where each of its
/// parts was written.
pub(crate) struct NodeTree(pub(crate) Node);

impl<'de> Deserialize<'de> for NodeTree {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<NodeTree, D::Error> {
        let located = Located::<NodeBody>::deserialize(deserializer)?;
        Ok(NodeTree(Node::new(located.value.0, located.span)))
    }
}

/// What a value holds, without its own position.
struct NodeBody(NodeKind);

impl<'de> Deserialize<'de> for NodeBody {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<NodeBody, D::Error> {
        struct BodyVisitor;

        impl<'de> Visitor<'de> for BodyVisitor {
            type Value = NodeBody;

            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str("any JSON value")
            }

            fn visit_unit<E>(self) -> Result<NodeBody, E> {
                Ok(NodeBody(NodeKind::Null))
            }

            fn visit_bool<E>(self, value: bool) -> Result<NodeBody, E> {
                Ok(NodeBody(NodeKind::Bool(value)))
            }

            fn visit_i64<E>(self, value: i64) -> Result<NodeBody, E> {
                Ok(NodeBody(NodeKind::Number(Num::Int(value))))
            }

            fn visit_u64<E>(self, value: u64) -> Result<NodeBody, E> {
                Ok(NodeBody(NodeKind::Number(Num::UInt(value))))
            }

            fn visit_f64<E>(self, value: f64) -> Result<NodeBody, E> {
                Ok(NodeBody(NodeKind::Number(Num::Float(value))))
            }

            fn visit_str<E>(self, value: &str) -> Result<NodeBody, E> {
                Ok(NodeBody(NodeKind::String(value.to_owned())))
            }

            fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<NodeBody, A::Error> {
                let mut items = Vec::new();
                while let Some(NodeTree(item)) = seq.next_element::<NodeTree>()? {
                    items.push(item);
                }
                Ok(NodeBody(NodeKind::Array(items)))
            }

            fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<NodeBody, A::Error> {
                let mut members = Vec::new();
                while let Some(key) = map.next_key::<Located<String>>()? {
                    let NodeTree(value) = map.next_value::<NodeTree>()?;
                    members.push(Member {
                        key: key.value,
                        key_span: key.span,
                        value,
                    });
                }
                Ok(NodeBody(NodeKind::Object(members)))
            }
        }

        deserializer.deserialize_any(BodyVisitor)
    }
}

impl From<NodeTree> for serde_json::Value {
    fn from(tree: NodeTree) -> serde_json::Value {
        to_json(&tree.0)
    }
}

/// A tree as plain JSON, without its positions.
pub(crate) fn to_json(node: &Node) -> serde_json::Value {
    match &node.kind {
        NodeKind::Null => serde_json::Value::Null,
        NodeKind::Bool(value) => serde_json::Value::Bool(*value),
        NodeKind::Number(Num::Int(value)) => serde_json::Value::from(*value),
        NodeKind::Number(Num::UInt(value)) => serde_json::Value::from(*value),
        NodeKind::Number(Num::Float(value)) | NodeKind::Quantity { value, .. } => {
            serde_json::Number::from_f64(*value).map_or(serde_json::Value::Null, Into::into)
        }
        NodeKind::String(text) => serde_json::Value::String(text.clone()),
        NodeKind::Array(items) => serde_json::Value::Array(items.iter().map(to_json).collect()),
        NodeKind::Object(members) => serde_json::Value::Object(
            members
                .iter()
                .map(|member| (member.key.clone(), to_json(&member.value)))
                .collect(),
        ),
    }
}

/// What compiling a query gives besides the query: the scope in which
/// `it` is bound, for the `weight` and `of` of a value source.
pub(crate) struct Compiled {
    pub(crate) query: CompiledQuery,
    pub(crate) it_scope: Scope,
}

/// Compiles a query written at `node`, in the scope of the field it is
/// in. Every mistake is reported through `cx` with its position and
/// JSON path, and gives `None`.
///
/// For a field type of another crate that holds a query, such as the
/// `for_each` of a list of effects.
pub fn compile_query(
    node: &Node,
    site: &Site,
    cx: &mut CompileCx<'_>,
    scope: &Scope,
) -> Option<CompiledQuery> {
    compile(node, site, cx, scope).map(|compiled| compiled.query)
}

/// Reports a query that is not well formed.
fn bad(cx: &mut CompileCx<'_>, span: crate::source::Span, path: &str, message: impl Into<String>) {
    bad_with(cx, span, path, message, None);
}

fn bad_with(
    cx: &mut CompileCx<'_>,
    span: crate::source::Span,
    path: &str,
    message: impl Into<String>,
    suggestion: Option<String>,
) {
    let diagnostic = Diagnostic::new(codes::BAD_QUERY, message)
        .at(cx.sources(), span)
        .with_path(path)
        .suggest_opt(suggestion);
    cx.report(diagnostic);
}

fn child(path: &str, key: &str) -> String {
    if path.is_empty() {
        key.to_owned()
    } else {
        format!("{path}.{key}")
    }
}

/// Reads a part of a query as a field of type `T`, reporting what is
/// wrong with it.
fn read<T: serde::de::DeserializeOwned>(
    node: &Node,
    path: &str,
    cx: &mut CompileCx<'_>,
) -> Option<T> {
    let mut read = Diagnostics::new();
    let value: Option<T> = crate::from_node_at(node, cx.sources(), &mut read, None, path);
    for diagnostic in read.iter() {
        cx.report(diagnostic.clone());
    }
    value
}

#[allow(clippy::too_many_lines)]
pub(crate) fn compile(
    node: &Node,
    site: &Site,
    cx: &mut CompileCx<'_>,
    scope: &Scope,
) -> Option<Compiled> {
    let errors = cx.error_count();
    let path = site.path.as_str();
    let registry = cx.registry();
    let Some([head]) = node.as_object() else {
        bad(
            cx,
            node.span,
            path,
            format!(
                "a query is an object with one key, the kind of query: `{{ \"entities\": {{ … }} }}`; this is {}",
                node.kind_name()
            ),
        );
        return None;
    };
    let name = head.key.as_str();
    let at = child(path, name);
    let body = &head.value;

    // The parts every kind takes, and what remains.
    let mut members: &[Member] = body.as_object().unwrap_or_default();
    let mut items_node: Option<&Node> = None;
    let mut long_list = false;
    if name == LIST_QUERY {
        match body.as_object() {
            Some(found) if found.iter().any(|member| member.key == "items") => {
                long_list = true;
                members = found;
            }
            _ => {
                items_node = Some(body);
                members = &[];
            }
        }
    } else if body.as_object().is_none() {
        bad(
            cx,
            body.span,
            &at,
            format!(
                "`{name}` takes an object: its arguments, and `where`, `sort` and `limit`; this is {}",
                body.kind_name()
            ),
        );
        return None;
    }
    let get = |key: &str| members.iter().find(|member| member.key == key);

    // What gives the candidates, and their type.
    let (source, element, args_allowed): (Option<Source>, Type, Vec<String>) = match name {
        LIST_QUERY => {
            if long_list {
                items_node = get("items").map(|member| &member.value);
            }
            let items_path = if long_list {
                child(&at, "items")
            } else {
                at.clone()
            };
            let mut list: Option<Expr<Any>> =
                items_node.and_then(|items| read(items, &items_path, cx));
            if let Some(list) = &mut list {
                cx.compile(
                    list,
                    &scope
                        .clone()
                        .expecting(TypeSpec::Exact(Type::list(Type::Any))),
                );
            }
            let element = list
                .as_ref()
                .and_then(|list| list.compiled())
                .and_then(|code| code.ty().element().cloned())
                .unwrap_or(Type::Any);
            let source = list.and_then(|list| list.ready.clone()).map(Source::List);
            (source, element, vec!["items".to_owned()])
        }
        DEFS_QUERY => {
            let ty_name = get("type").and_then(|member| member.value.as_str());
            let Some(ty_name) = ty_name else {
                bad(
                    cx,
                    body.span,
                    &at,
                    "`defs` needs `type`: the name of a definition type, as a string",
                );
                return None;
            };
            let Some(ty) = registry.def_type_id(ty_name) else {
                let known = registry.def_types().map(|def_type| def_type.name.as_str());
                let span = get("type").map_or(body.span, |member| member.value.span);
                bad_with(
                    cx,
                    span,
                    &child(&at, "type"),
                    format!("there is no definition type `{ty_name}`"),
                    did_you_mean(closest(ty_name, known)),
                );
                return None;
            };
            let values: Arc<[Value]> = registry
                .def_type(ty)
                .map(|def_type| {
                    def_type
                        .ids()
                        .map(|(_, index)| Value::Id(DefRef::new(ty, index)))
                        .collect()
                })
                .unwrap_or_else(|| Arc::from(Vec::new()));
            (
                Some(Source::Defs(values)),
                Type::Id(ty),
                vec!["type".to_owned()],
            )
        }
        _ => {
            let Some(kind) = registry.query_id(name).and_then(|id| registry.query(id)) else {
                let known = registry
                    .queries()
                    .map(|kind| kind.spec.name.as_str())
                    .chain([LIST_QUERY, DEFS_QUERY]);
                bad_with(
                    cx,
                    head.key_span,
                    &at,
                    format!("there is no kind of query `{name}`"),
                    did_you_mean(closest(name, known)),
                );
                return None;
            };
            let spec = &kind.spec;
            if scope.is_belief() && !spec.belief_safe {
                let diagnostic = Diagnostic::new(
                        codes::NOT_BELIEF_SAFE,
                        format!(
                            "`{name}` reads the world as it is, and this is a mind context, where a character decides by what it believes (05 §7.3). A query is answered from belief once the perception package registers it as safe for belief: `QuerySpec::safe` and the observer of `Host::query` are the seam (15 §7). Until then a character cannot ask `{name}`"
                        ),
                    )
                    .at(cx.sources(), head.key_span)
                    .with_path(at.clone());
                cx.report(diagnostic);
                return None;
            }
            // Shapes that exclude each other, and arguments that go
            // together.
            let written: Vec<&str> = members
                .iter()
                .filter(|member| spec.arg_index(&member.key).is_some())
                .map(|member| member.key.as_str())
                .collect();
            let shapes: Vec<&str> = written
                .iter()
                .copied()
                .filter(|key| spec.one_of.iter().any(|shape| shape == key))
                .collect();
            if shapes.len() > 1 {
                bad(
                    cx,
                    body.span,
                    &at,
                    format!(
                        "`{name}` takes at most one of {}; this has {}",
                        spec.one_of
                            .iter()
                            .map(|shape| format!("`{shape}`"))
                            .collect::<Vec<_>>()
                            .join(", "),
                        shapes
                            .iter()
                            .map(|shape| format!("`{shape}`"))
                            .collect::<Vec<_>>()
                            .join(" and ")
                    ),
                );
                return None;
            }
            for (arg, with) in &spec.needs {
                if written.contains(&arg.as_str()) && !written.contains(&with.as_str()) {
                    let span = get(arg).map_or(body.span, |member| member.key_span);
                    bad(
                        cx,
                        span,
                        &child(&at, arg),
                        format!("`{arg}` of `{name}` goes with `{with}`, which is not here"),
                    );
                    return None;
                }
            }
            let mut args: Vec<Option<Ready>> = Vec::with_capacity(spec.args.len());
            let mut constants: Vec<Option<Value>> = Vec::with_capacity(spec.args.len());
            for param in &spec.args {
                let Some(member) = get(&param.name) else {
                    args.push(None);
                    constants.push(None);
                    continue;
                };
                let arg_path = child(&at, &param.name);
                let mut field: Option<Expr<Any>> = read(&member.value, &arg_path, cx);
                if let Some(field) = &mut field {
                    cx.compile(
                        field,
                        &scope
                            .clone()
                            .expecting(TypeSpec::Exact(param.ty.clone().nullable())),
                    );
                }
                constants.push(field.as_ref().and_then(|field| field.constant().cloned()));
                args.push(field.and_then(|field| field.ready.clone()));
            }
            if let Some(extent) = spec.extent
                && let Some(cells) = extent(&constants)
                && cells > DEFAULT_MAX_QUERY_CELLS
            {
                let diagnostic = Diagnostic::new(
                        codes::QUERY_TOO_LARGE,
                        format!(
                            "this `{name}` covers {cells} cells, more than a query may ({DEFAULT_MAX_QUERY_CELLS}, `rules.max_query_cells`)"
                        ),
                    )
                    .at(cx.sources(), body.span)
                    .with_path(at.clone())
                    .suggest("make it smaller, or narrow it with `in_area` or `box`");
                cx.report(diagnostic);
                return None;
            }
            let allowed = spec.args.iter().map(|param| param.name.clone()).collect();
            (
                Some(Source::Host(kind.id, args)),
                spec.element.clone(),
                allowed,
            )
        }
    };

    // Keys that are neither arguments nor the three every query takes.
    for member in members {
        let key = member.key.as_str();
        if QUERY_KEYS.contains(&key) || args_allowed.iter().any(|allowed| allowed == key) {
            continue;
        }
        let known = args_allowed
            .iter()
            .map(String::as_str)
            .chain(QUERY_KEYS.iter().copied());
        bad_with(
            cx,
            member.key_span,
            &child(&at, key),
            format!(
                "`{name}` has no `{key}`: it takes {}",
                args_allowed
                    .iter()
                    .map(String::as_str)
                    .chain(QUERY_KEYS.iter().copied())
                    .map(|key| format!("`{key}`"))
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
            did_you_mean(closest(key, known)),
        );
    }

    // `it`, after everything the field binds.
    let it = u16::try_from(scope.subject_count()).unwrap_or(u16::MAX);
    let it_scope = scope
        .clone()
        .subject(IT, TypeSpec::Exact(element.clone()))
        .without_expectation();
    let filter = get("where").and_then(|member| {
        let mut cond: Cond = read(&member.value, &child(&at, "where"), cx)?;
        cx.compile(&mut cond, &it_scope);
        Some(cond)
    });
    let sort = get("sort").and_then(|member| {
        let mut key: Expr<Any> = read(&member.value, &child(&at, "sort"), cx)?;
        cx.compile(&mut key, &it_scope.clone().pure());
        Some(key)
    });
    let limit = get("limit").and_then(|member| {
        let mut limit: Expr<Int> = read(&member.value, &child(&at, "limit"), cx)?;
        cx.compile(&mut limit, &scope.clone().without_expectation());
        Some(limit)
    });
    let observer = if scope.is_belief() {
        scope.own_slot().and_then(|slot| u16::try_from(slot).ok())
    } else {
        None
    };
    let origin = cx.origin(site);
    if cx.error_count() > errors {
        return None;
    }
    Some(Compiled {
        query: CompiledQuery {
            name: Arc::from(name),
            source: source?,
            filter,
            sort,
            limit,
            it,
            element,
            observer,
            origin,
        },
        it_scope,
    })
}

/// How sort keys compare: numbers by value before texts in order, and
/// anything else last, as it came.
fn compare_keys(a: &Value, b: &Value) -> Ordering {
    fn rank(value: &Value) -> u8 {
        match value {
            Value::Int(_) | Value::Real(_) => 0,
            Value::Str(_) => 1,
            _ => 2,
        }
    }
    match (a, b) {
        (Value::Str(a), Value::Str(b)) => a.cmp(b),
        _ => match (a.as_number(), b.as_number()) {
            (Some(a), Some(b)) => a.partial_cmp(&b).unwrap_or(Ordering::Equal),
            _ => rank(a).cmp(&rank(b)),
        },
    }
}

impl<H: Host + ?Sized> Evaluator<'_, H> {
    /// Runs a query: its candidates, those `where` lets through, in the
    /// order `sort` gives or else their default order, at most `limit`
    /// of them. `None` if it was refused, which has been reported as a
    /// run-time warning.
    pub fn query(&mut self, query: &CompiledQuery) -> Option<Vec<Value>> {
        self.query_up_to(query, None)
    }

    /// Whether a query has any result: what `{ "exists": query }` asks.
    pub fn query_exists(&mut self, query: &CompiledQuery) -> bool {
        self.query_up_to(query, Some(1))
            .is_some_and(|found| !found.is_empty())
    }

    pub(crate) fn warn_query(&mut self, fault: Fault, origin: u64) {
        self.warn_at(RuntimeWarning {
            fault,
            span: Span::default(),
            origin,
        });
    }

    /// Runs a query, stopping once `enough` results are certain when
    /// nothing is to be sorted.
    pub(crate) fn query_up_to(
        &mut self,
        query: &CompiledQuery,
        enough: Option<usize>,
    ) -> Option<Vec<Value>> {
        let candidates: Vec<Value> = match &query.source {
            Source::List(list) => match self.run(list) {
                Value::List(items) => items.to_vec(),
                Value::Null => Vec::new(),
                other => {
                    let found = other.type_name();
                    self.warn_query(
                        Fault::Type {
                            wanted: "a list",
                            found,
                        },
                        query.origin,
                    );
                    return None;
                }
            },
            Source::Defs(values) => values.to_vec(),
            Source::Host(id, args) => {
                let values: Vec<Value> = args
                    .iter()
                    .map(|arg| arg.as_ref().map_or(Value::Null, |arg| self.run(arg)))
                    .collect();
                let observer = query.observer.map(|slot| self.subject_of_field(slot));
                let mut out = Vec::new();
                let call = QueryCall {
                    query: *id,
                    args: &values,
                    observer: observer.as_ref(),
                };
                if let Err(fault) = self.host_query(&call, &mut out) {
                    self.warn_query(fault, query.origin);
                    return None;
                }
                out
            }
        };
        let limit = match &query.limit {
            Some(limit) => match limit.value(self) {
                Value::Int(count) => Some(usize::try_from(count.max(0)).unwrap_or(usize::MAX)),
                Value::Null => None,
                other => {
                    let found = other.type_name();
                    self.warn_query(
                        Fault::Type {
                            wanted: "an int",
                            found,
                        },
                        query.origin,
                    );
                    None
                }
            },
            None => None,
        };
        // Without sorting, the first results are known as they are found.
        let stop = if query.sort.is_none() {
            match (limit, enough) {
                (Some(a), Some(b)) => Some(a.min(b)),
                (a, b) => a.or(b),
            }
        } else {
            None
        };
        let mut kept: Vec<Value> = Vec::with_capacity(candidates.len());
        for candidate in candidates {
            if stop.is_some_and(|stop| kept.len() >= stop) {
                break;
            }
            let passes = match &query.filter {
                Some(filter) => {
                    self.with_it(query.it, candidate.clone(), |eval| filter.holds(eval))
                }
                None => true,
            };
            if passes {
                kept.push(candidate);
            }
        }
        if let Some(sort) = &query.sort {
            let mut keyed: Vec<(Value, Value)> = kept
                .into_iter()
                .map(|item| {
                    let key = self.with_it(query.it, item.clone(), |eval| sort.value(eval));
                    (key, item)
                })
                .collect();
            // Stable: ties keep the default order.
            keyed.sort_by(|(a, _), (b, _)| compare_keys(a, b));
            kept = keyed.into_iter().map(|(_, item)| item).collect();
        }
        if let Some(limit) = limit {
            kept.truncate(limit);
        }
        Some(kept)
    }
}
