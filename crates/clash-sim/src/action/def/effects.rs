//! [`EffectsSource`]: a list of effects, kept as it was written until the
//! rule runtime can compile it.

use std::fmt;

use clash_data::{FromSite, Kind, KindValue, Located, Member, Node, NodeKind, Num, Site, Span};
use serde::Deserialize;
use serde::de::{Deserializer, MapAccess, SeqAccess, Visitor};

/// A list of effects (05 §10) as it was written: the JSON, in order, with
/// the place of every value in it.
///
/// This stands where the rule runtime's compiled effect list will. An
/// action's `on_start`, a phase's `do` and the other effect lists of
/// 03 §6 are read into one of these and nothing inside is looked at: a
/// verb that does not exist loads without complaint until effects are
/// compiled. The rule runtime replaces the type; until then it can
/// compile from [`node`](EffectsSource::node), which is the loader's own
/// tree, with the scope the action gives its effects
/// ([`ActionDef::effect_scope`](super::ActionDef::effect_scope)).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct EffectsSource {
    node: Option<Node>,
    path: String,
}

impl EffectsSource {
    /// A list with nothing in it, written nowhere.
    pub fn empty() -> EffectsSource {
        EffectsSource::default()
    }

    /// The list as JSON, with positions. `None` if the field was left
    /// out.
    pub fn node(&self) -> Option<&Node> {
        self.node.as_ref()
    }

    /// The effects, one node each, in the order they run.
    pub fn effects(&self) -> &[Node] {
        self.node
            .as_ref()
            .and_then(Node::as_array)
            .unwrap_or_default()
    }

    /// How many effects the list holds.
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
}

impl<'de> FromSite<'de> for EffectsSource {
    fn from_site<A: MapAccess<'de>>(
        site: Site,
        kind: Kind,
        value: KindValue<'_, A>,
    ) -> Result<EffectsSource, A::Error> {
        if kind != Kind::List {
            return Err(kind.unexpected("a list of effects"));
        }
        let Tree(node) = value.read::<Tree>()?;
        Ok(EffectsSource {
            node: Some(node),
            path: site.path,
        })
    }
}

impl<'de> Deserialize<'de> for EffectsSource {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<EffectsSource, D::Error> {
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
