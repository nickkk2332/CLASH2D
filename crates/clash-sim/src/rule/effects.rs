//! [`Effects`]: a list of effects as a field of a definition (04 §7.1;
//! 05 §10).

use std::fmt;
use std::sync::Arc;

use clash_data::expr::Compile;
use clash_data::{
    CompileCx, FromSite, Kind, KindValue, Located, Member, Node, NodeKind, Num, Scope, Site, Span,
    ValidateCx,
};
use clash_expr::Type;
use serde::Deserialize;
use serde::de::{Deserializer, MapAccess, SeqAccess, Visitor};

use super::compile::{self, Within};
use super::tree::Block;

/// A list of effects (05 §10): a field type for any definition struct.
///
/// ```json
/// "on_start": [
///   { "set": { "self.var.busy": true } },
///   { "if": "self.var.kills > 3", "then": [ { "message": "Enough." } ] }
/// ]
/// ```
///
/// The field holds the list as it was written, with the place of every
/// value, and, once its definition has compiled it, the tree the
/// interpreter walks. A definition type uses it in three steps:
///
/// ```ignore
/// #[derive(serde::Deserialize)]
/// pub struct TrapDef {
///     #[serde(default)]
///     pub on_trigger: Effects,
/// }
///
/// impl Def for TrapDef {
///     // …
///     fn compile(&mut self, cx: &mut CompileCx<'_>) {
///         // What the effects may name: subjects and locals, in slot order.
///         let scope = Scope::new().subject("self", "entity").subject("here", "loc");
///         cx.compile(&mut self.on_trigger, &scope);
///     }
///
///     fn validate(&self, cx: &mut ValidateCx<'_>) {
///         // Calls of rules are checked against the rules they name.
///         self.on_trigger.validate(cx);
///     }
/// }
/// ```
///
/// and runs it with [`RuleSet::run_effects`](super::RuleSet::run_effects),
/// giving the values of the subjects and the locals in the order the
/// scope bound them. A list that is not handed over is never compiled
/// and does nothing when it is run.
///
/// Compiled this way a list is outside any rule: `return` in it is a
/// load error. A rule compiles its own lists itself.
#[derive(Clone, Debug, Default)]
pub struct Effects {
    node: Option<Node>,
    path: String,
    compiled: Option<Compiled>,
}

/// What compiling a list gives.
#[derive(Clone, Debug)]
pub(crate) struct Compiled {
    pub(crate) block: Block,
    /// How many locals a run needs room for: those of the scope and
    /// those the effects define.
    pub(crate) frame: usize,
    /// The names of the subjects of the scope, by slot.
    pub(crate) subjects: Arc<[Arc<str>]>,
}

impl Effects {
    /// A list with nothing in it, written nowhere.
    pub fn empty() -> Effects {
        Effects::default()
    }

    /// The list as JSON, with positions. `None` if the field was left
    /// out.
    pub fn node(&self) -> Option<&Node> {
        self.node.as_ref()
    }

    /// The effects as they were written, one node each, in the order
    /// they run.
    pub fn effects(&self) -> &[Node] {
        self.node
            .as_ref()
            .and_then(Node::as_array)
            .unwrap_or_default()
    }

    /// How many effects were written.
    pub fn len(&self) -> usize {
        self.effects().len()
    }

    /// Whether there is nothing to run.
    pub fn is_empty(&self) -> bool {
        self.effects().is_empty()
    }

    /// Where the list was written, if it was.
    pub fn span(&self) -> Option<Span> {
        self.node.as_ref().map(|node| node.span)
    }

    /// The JSON path of the list inside its definition:
    /// `phases.impact.do`.
    pub fn path(&self) -> &str {
        &self.path
    }

    /// Where the list was written and at what path, as the expression
    /// fields keep it.
    pub fn site(&self) -> Site {
        Site {
            span: self.span(),
            path: self.path.clone(),
        }
    }

    /// Whether the list has been compiled. One that has not does
    /// nothing when it is run.
    pub fn is_compiled(&self) -> bool {
        self.compiled.is_some()
    }

    /// The compiled tree, once the list's definition has compiled it.
    pub fn block(&self) -> Option<&Block> {
        self.compiled.as_ref().map(|compiled| &compiled.block)
    }

    /// How many locals a run of the list needs room for: those of its
    /// scope and those its effects define. Zero until it is compiled.
    pub fn frame_size(&self) -> usize {
        self.compiled.as_ref().map_or(0, |compiled| compiled.frame)
    }

    /// The names of the subjects the list was compiled with, by slot.
    pub fn subject_names(&self) -> &[Arc<str>] {
        self.compiled
            .as_ref()
            .map_or(&[], |compiled| &compiled.subjects)
    }

    pub(crate) fn compiled(&self) -> Option<&Compiled> {
        self.compiled.as_ref()
    }

    /// Compiles the list as part of a rule that returns values of
    /// `returns`.
    pub(crate) fn compile_in_rule(
        &mut self,
        cx: &mut CompileCx<'_>,
        scope: &Scope,
        returns: &Type,
    ) {
        self.compile_within(cx, scope, &Within::Rule(returns.clone()));
    }

    fn compile_within(&mut self, cx: &mut CompileCx<'_>, scope: &Scope, within: &Within) {
        let site = self.site();
        self.compiled = match &self.node {
            Some(node) => compile::compile_list(node, &site, cx, scope, within),
            None => Some(compile::nothing(&site, cx, scope)),
        };
    }

    /// Checks the calls of rules in the list against the rules they
    /// name, which cannot be seen while the list is compiled: a rule
    /// that needs a subject its caller does not bind is reported at the
    /// call. A definition type calls this from
    /// [`Def::validate`](clash_data::Def::validate).
    pub fn validate(&self, cx: &mut ValidateCx<'_>) {
        if let Some(compiled) = &self.compiled {
            compile::check_calls(&compiled.block, &compiled.subjects, cx);
        }
    }
}

impl Compile for Effects {
    fn compile_in(&mut self, cx: &mut CompileCx<'_>, scope: &Scope) {
        self.compile_within(cx, scope, &Within::List);
    }
}

impl<'de> FromSite<'de> for Effects {
    fn from_site<A: MapAccess<'de>>(
        site: Site,
        kind: Kind,
        value: KindValue<'_, A>,
    ) -> Result<Effects, A::Error> {
        if kind != Kind::List {
            return Err(kind.unexpected("a list of effects"));
        }
        let Tree(node) = value.read::<Tree>()?;
        Ok(Effects {
            node: Some(node),
            path: site.path,
            compiled: None,
        })
    }
}

impl<'de> Deserialize<'de> for Effects {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Effects, D::Error> {
        clash_data::by_site(deserializer)
    }
}

/// Any JSON value, read back into the loader's tree with its positions.
struct Tree(Node);

impl<'de> Deserialize<'de> for Tree {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Tree, D::Error> {
        let located = Located::<Body>::deserialize(deserializer)?;
        Ok(Tree(Node::new(located.value.0, located.span)))
    }
}

/// What a value holds, without its own position.
struct Body(NodeKind);

impl<'de> Deserialize<'de> for Body {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Body, D::Error> {
        struct BodyVisitor;

        impl<'de> Visitor<'de> for BodyVisitor {
            type Value = Body;

            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str("any JSON value")
            }

            fn visit_unit<E>(self) -> Result<Body, E> {
                Ok(Body(NodeKind::Null))
            }

            fn visit_bool<E>(self, value: bool) -> Result<Body, E> {
                Ok(Body(NodeKind::Bool(value)))
            }

            fn visit_i64<E>(self, value: i64) -> Result<Body, E> {
                Ok(Body(NodeKind::Number(Num::Int(value))))
            }

            fn visit_u64<E>(self, value: u64) -> Result<Body, E> {
                Ok(Body(NodeKind::Number(Num::UInt(value))))
            }

            fn visit_f64<E>(self, value: f64) -> Result<Body, E> {
                Ok(Body(NodeKind::Number(Num::Float(value))))
            }

            fn visit_str<E>(self, value: &str) -> Result<Body, E> {
                Ok(Body(NodeKind::String(value.to_owned())))
            }

            fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<Body, A::Error> {
                let mut items = Vec::new();
                while let Some(Tree(item)) = seq.next_element::<Tree>()? {
                    items.push(item);
                }
                Ok(Body(NodeKind::Array(items)))
            }

            fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Body, A::Error> {
                let mut members = Vec::new();
                while let Some(key) = map.next_key::<Located<String>>()? {
                    let Tree(value) = map.next_value::<Tree>()?;
                    members.push(Member {
                        key: key.value,
                        key_span: key.span,
                        value,
                    });
                }
                Ok(Body(NodeKind::Object(members)))
            }
        }

        deserializer.deserialize_any(BodyVisitor)
    }
}
