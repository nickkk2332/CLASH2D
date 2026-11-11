//! [`Value`]: what an expression evaluates to, and the ids and handles
//! that values and the registry are written in.
//!
//! `clash_core::Value` holds the four kinds of value a variable could
//! hold when the foundation was built. The expression language needs
//! more: null, ids, references to subjects, lists and maps (05 §3). This
//! is that wider type. Every `clash_core::Value` converts into it, and it
//! converts back when it holds one of the four.
//!
//! A value is cheap to clone: texts, lists and maps are shared, not
//! copied. Values are immutable; the functions that "change" a list or a
//! map give a new one.

use core::fmt;
use std::collections::BTreeMap;
use std::sync::Arc;

use clash_core::{Real, StateHash, StateHasher, define_id};

define_id! {
    /// A kind of subject (05 §5.1): `entity`, `loc`, `site`. Assigned by
    /// the registry in order of registration.
    pub struct KindId(u16);
    /// A registered property of a kind of subject.
    pub struct PropertyId;
    /// A registered function.
    pub struct FunctionId;
    /// A registered event type.
    pub struct EventId;
    /// A registered verb.
    pub struct VerbId;
    /// A type of definition that ids can refer to: `material`, `block`.
    pub struct DefTypeId(u16);
    /// A declared variable.
    pub struct VariableId;
}

/// A reference to a subject: its kind, and sixty-four bits that mean
/// something only to the host that made it.
///
/// The expression language never looks inside the payload. An entity's
/// handle might carry `EntityId::to_bits`; a location's, packed
/// coordinates. A handle may outlive what it refers to: the host answers
/// reads through a stale one with null.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub struct Handle {
    /// The kind of subject.
    pub kind: KindId,
    /// The host's own reference.
    pub payload: u64,
}

impl Handle {
    /// A handle.
    pub const fn new(kind: KindId, payload: u64) -> Handle {
        Handle { kind, payload }
    }
}

impl StateHash for Handle {
    fn state_hash(&self, hasher: &mut StateHasher) {
        self.kind.state_hash(hasher);
        hasher.write_u64(self.payload);
    }
}

/// A reference to a definition: its type, and the number the host knows
/// it by (the dense id of 04 §7.2).
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub struct DefRef {
    /// The type of definition.
    pub ty: DefTypeId,
    /// The definition's number within its type.
    pub index: u32,
}

impl DefRef {
    /// A reference.
    pub const fn new(ty: DefTypeId, index: u32) -> DefRef {
        DefRef { ty, index }
    }
}

impl StateHash for DefRef {
    fn state_hash(&self, hasher: &mut StateHasher) {
        self.ty.state_hash(hasher);
        hasher.write_u32(self.index);
    }
}

/// A map value: entries in order of key. Keys are texts; an id used as a
/// key counts as its qualified id.
pub type Map = BTreeMap<Arc<str>, Value>;

/// A value of the expression language (05 §3).
///
/// ```
/// use clash_core::Real;
/// use clash_expr::Value;
///
/// let kills = Value::from(3);
/// let names = Value::list([Value::from("ash"), Value::from("iron")]);
/// assert_eq!(kills.as_int(), Some(3));
/// assert_eq!(kills.as_number(), Some(Real::of(3.0)));
/// assert_eq!(names.as_list().map(<[Value]>::len), Some(2));
/// assert_eq!(names.to_string(), "[ash, iron]");
/// assert!(Value::Null.is_null());
/// ```
#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub enum Value {
    /// Nothing.
    #[default]
    Null,
    /// `true` or `false`.
    Bool(bool),
    /// A 64-bit integer.
    Int(i64),
    /// A deterministic real number, in SI units where it is a quantity.
    Real(Real),
    /// A text.
    Str(Arc<str>),
    /// A definition.
    Id(DefRef),
    /// A reference to a subject.
    Subject(Handle),
    /// Ordered values.
    List(Arc<[Value]>),
    /// Values by key, in order of key.
    Map(Arc<Map>),
}

impl Value {
    /// A text value.
    pub fn text(text: impl AsRef<str>) -> Value {
        Value::Str(Arc::from(text.as_ref()))
    }

    /// A list value.
    pub fn list(items: impl IntoIterator<Item = Value>) -> Value {
        Value::List(items.into_iter().collect())
    }

    /// A map value. A key given twice keeps its last value.
    pub fn map<K: AsRef<str>>(entries: impl IntoIterator<Item = (K, Value)>) -> Value {
        Value::Map(Arc::new(
            entries
                .into_iter()
                .map(|(key, value)| (Arc::from(key.as_ref()), value))
                .collect(),
        ))
    }

    /// The name of the value's type as the language writes it. A list is
    /// `list` and a map `map`, whatever they hold; a subject is `subject`
    /// and an id `id`, because their kinds are names only the registry
    /// knows.
    pub const fn type_name(&self) -> &'static str {
        match self {
            Value::Null => "null",
            Value::Bool(_) => "bool",
            Value::Int(_) => "int",
            Value::Real(_) => "real",
            Value::Str(_) => "string",
            Value::Id(_) => "id",
            Value::Subject(_) => "subject",
            Value::List(_) => "list",
            Value::Map(_) => "map",
        }
    }

    /// Whether this is null.
    pub const fn is_null(&self) -> bool {
        matches!(self, Value::Null)
    }

    /// The boolean, if this is one.
    pub const fn as_bool(&self) -> Option<bool> {
        match self {
            Value::Bool(value) => Some(*value),
            _ => None,
        }
    }

    /// The integer, if this is one.
    pub const fn as_int(&self) -> Option<i64> {
        match self {
            Value::Int(value) => Some(*value),
            _ => None,
        }
    }

    /// The real number, if this is one. An integer is not converted.
    pub const fn as_real(&self) -> Option<Real> {
        match self {
            Value::Real(value) => Some(*value),
            _ => None,
        }
    }

    /// The number as a real, if this is an integer or a real.
    pub const fn as_number(&self) -> Option<Real> {
        match self {
            Value::Int(value) => Some(Real::from_i64(*value)),
            Value::Real(value) => Some(*value),
            _ => None,
        }
    }

    /// The text, if this is one.
    pub fn as_str(&self) -> Option<&str> {
        match self {
            Value::Str(value) => Some(value),
            _ => None,
        }
    }

    /// The definition, if this is an id.
    pub const fn as_id(&self) -> Option<DefRef> {
        match self {
            Value::Id(value) => Some(*value),
            _ => None,
        }
    }

    /// The handle, if this is a subject.
    pub const fn as_subject(&self) -> Option<Handle> {
        match self {
            Value::Subject(value) => Some(*value),
            _ => None,
        }
    }

    /// The elements, if this is a list.
    pub fn as_list(&self) -> Option<&[Value]> {
        match self {
            Value::List(items) => Some(items),
            _ => None,
        }
    }

    /// The entries, if this is a map.
    pub fn as_map(&self) -> Option<&Map> {
        match self {
            Value::Map(entries) => Some(entries),
            _ => None,
        }
    }
}

impl From<bool> for Value {
    fn from(value: bool) -> Value {
        Value::Bool(value)
    }
}

impl From<i64> for Value {
    fn from(value: i64) -> Value {
        Value::Int(value)
    }
}

impl From<i32> for Value {
    fn from(value: i32) -> Value {
        Value::Int(i64::from(value))
    }
}

impl From<Real> for Value {
    fn from(value: Real) -> Value {
        Value::Real(value)
    }
}

impl From<&str> for Value {
    fn from(value: &str) -> Value {
        Value::Str(Arc::from(value))
    }
}

impl From<String> for Value {
    fn from(value: String) -> Value {
        Value::Str(Arc::from(value))
    }
}

impl From<Arc<str>> for Value {
    fn from(value: Arc<str>) -> Value {
        Value::Str(value)
    }
}

impl From<Handle> for Value {
    fn from(value: Handle) -> Value {
        Value::Subject(value)
    }
}

impl From<DefRef> for Value {
    fn from(value: DefRef) -> Value {
        Value::Id(value)
    }
}

impl From<Vec<Value>> for Value {
    fn from(items: Vec<Value>) -> Value {
        Value::List(items.into())
    }
}

impl From<Map> for Value {
    fn from(entries: Map) -> Value {
        Value::Map(Arc::new(entries))
    }
}

impl<T: Into<Value>> From<Option<T>> for Value {
    fn from(value: Option<T>) -> Value {
        value.map_or(Value::Null, Into::into)
    }
}

/// Every value a variable could hold in the foundation is a value here.
impl From<clash_core::Value> for Value {
    fn from(value: clash_core::Value) -> Value {
        match value {
            clash_core::Value::Bool(value) => Value::Bool(value),
            clash_core::Value::Int(value) => Value::Int(value),
            clash_core::Value::Real(value) => Value::Real(value),
            clash_core::Value::Str(value) => Value::Str(Arc::from(value)),
            // `clash_core::Value` may grow. Until this type follows, a
            // kind it does not know reads as nothing.
            _ => Value::Null,
        }
    }
}

/// A boolean, an integer, a real or a text converts back. Anything else
/// is handed back unchanged as the error.
impl TryFrom<Value> for clash_core::Value {
    type Error = Value;

    fn try_from(value: Value) -> Result<clash_core::Value, Value> {
        match value {
            Value::Bool(value) => Ok(clash_core::Value::Bool(value)),
            Value::Int(value) => Ok(clash_core::Value::Int(value)),
            Value::Real(value) => Ok(clash_core::Value::Real(value)),
            Value::Str(value) => Ok(clash_core::Value::Str(value.as_ref().to_owned())),
            other => Err(other),
        }
    }
}

/// The value as plain text: `null`, `true`, `42`, `3.5`, a text as it
/// is, a list as `[a, b]`, a map as `{key: value}`.
///
/// An id is written `id#N` and a subject `subject#K:N`, because only the
/// host knows their names. Text shown to a player goes through the
/// machine, which asks the host.
impl fmt::Display for Value {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Value::Null => f.write_str("null"),
            Value::Bool(value) => fmt::Display::fmt(value, f),
            Value::Int(value) => fmt::Display::fmt(value, f),
            Value::Real(value) => fmt::Display::fmt(value, f),
            Value::Str(value) => f.write_str(value),
            Value::Id(id) => write!(f, "id#{}", id.index),
            Value::Subject(handle) => write!(f, "subject#{}:{}", handle.kind, handle.payload),
            Value::List(items) => {
                f.write_str("[")?;
                for (index, item) in items.iter().enumerate() {
                    if index > 0 {
                        f.write_str(", ")?;
                    }
                    fmt::Display::fmt(item, f)?;
                }
                f.write_str("]")
            }
            Value::Map(entries) => {
                f.write_str("{")?;
                for (index, (key, value)) in entries.iter().enumerate() {
                    if index > 0 {
                        f.write_str(", ")?;
                    }
                    write!(f, "{key}: {value}")?;
                }
                f.write_str("}")
            }
        }
    }
}

/// A one-byte tag for the kind of value, then the value. The four kinds
/// shared with `clash_core::Value` keep its tags and its encoding, so a
/// value that fits both hashes the same in either. A map hashes in order
/// of key, whatever order it was built in.
impl StateHash for Value {
    fn state_hash(&self, hasher: &mut StateHasher) {
        match self {
            Value::Bool(value) => {
                hasher.write_u8(0);
                value.state_hash(hasher);
            }
            Value::Int(value) => {
                hasher.write_u8(1);
                value.state_hash(hasher);
            }
            Value::Real(value) => {
                hasher.write_u8(2);
                value.state_hash(hasher);
            }
            Value::Str(value) => {
                hasher.write_u8(3);
                hasher.write_str(value);
            }
            Value::Null => hasher.write_u8(4),
            Value::Id(id) => {
                hasher.write_u8(5);
                id.state_hash(hasher);
            }
            Value::Subject(handle) => {
                hasher.write_u8(6);
                handle.state_hash(hasher);
            }
            Value::List(items) => {
                hasher.write_u8(7);
                hasher.write_len(items.len());
                for item in items.iter() {
                    item.state_hash(hasher);
                }
            }
            Value::Map(entries) => {
                hasher.write_u8(8);
                hasher.write_len(entries.len());
                for (key, value) in entries.iter() {
                    hasher.write_str(key);
                    value.state_hash(hasher);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use clash_core::Hash128;

    #[test]
    fn values_know_their_type_and_convert() {
        let handle = Handle::new(KindId::new(2), 77);
        let id = DefRef::new(DefTypeId::new(1), 5);
        let values = [
            Value::Null,
            Value::from(true),
            Value::from(7_i64),
            Value::from(Real::of(1.5)),
            Value::from("ash"),
            Value::from(id),
            Value::from(handle),
            Value::list([Value::from(1)]),
            Value::map([("a", Value::from(1))]),
        ];
        let names: Vec<&str> = values.iter().map(Value::type_name).collect();
        assert_eq!(
            names,
            [
                "null", "bool", "int", "real", "string", "id", "subject", "list", "map"
            ]
        );
        assert!(values[0].is_null());
        assert_eq!(values[1].as_bool(), Some(true));
        assert_eq!(values[2].as_int(), Some(7));
        assert_eq!(values[2].as_real(), None);
        assert_eq!(values[2].as_number(), Some(Real::of(7.0)));
        assert_eq!(values[3].as_real(), Some(Real::of(1.5)));
        assert_eq!(values[3].as_number(), Some(Real::of(1.5)));
        assert_eq!(values[4].as_str(), Some("ash"));
        assert_eq!(values[5].as_id(), Some(id));
        assert_eq!(values[6].as_subject(), Some(handle));
        assert_eq!(values[7].as_list(), Some(&[Value::Int(1)][..]));
        assert_eq!(values[8].as_map().map(Map::len), Some(1));
        // Accessors do not convert between kinds.
        assert_eq!(values[4].as_bool(), None);
        assert_eq!(values[0].as_int(), None);
        assert_eq!(values[0].as_number(), None);
        assert_eq!(values[1].as_str(), None);
        assert_eq!(values[1].as_id(), None);
        assert_eq!(values[1].as_subject(), None);
        assert_eq!(values[1].as_list(), None);
        assert_eq!(values[1].as_map(), None);
        assert_eq!(Value::default(), Value::Null);
        assert_eq!(Value::from(7_i32), Value::Int(7));
        assert_eq!(Value::from("ash".to_owned()), Value::text("ash"));
        assert_eq!(Value::from(Arc::<str>::from("ash")), Value::text("ash"));
        assert_eq!(Value::from(vec![Value::Int(1)]), values[7]);
        assert_eq!(Value::from(None::<i64>), Value::Null);
        assert_eq!(Value::from(Some(2_i64)), Value::Int(2));
        let mut map = Map::new();
        map.insert(Arc::from("a"), Value::Int(1));
        assert_eq!(Value::from(map), values[8]);
    }

    #[test]
    fn values_display_plainly() {
        assert_eq!(Value::Null.to_string(), "null");
        assert_eq!(Value::from(false).to_string(), "false");
        assert_eq!(Value::from(-3).to_string(), "-3");
        assert_eq!(Value::from(Real::of(2.5)).to_string(), "2.5");
        assert_eq!(Value::from("two words").to_string(), "two words");
        assert_eq!(
            Value::from(DefRef::new(DefTypeId::new(0), 4)).to_string(),
            "id#4"
        );
        assert_eq!(
            Value::from(Handle::new(KindId::new(1), 9)).to_string(),
            "subject#1:9"
        );
        assert_eq!(
            Value::list([Value::from(1), Value::Null, Value::from("x")]).to_string(),
            "[1, null, x]"
        );
        assert_eq!(
            Value::map([("b", Value::from(2)), ("a", Value::from(1))]).to_string(),
            "{a: 1, b: 2}"
        );
        assert_eq!(Value::list([]).to_string(), "[]");
    }

    #[test]
    fn core_values_convert_both_ways() {
        let cases = [
            (clash_core::Value::from(true), Value::from(true)),
            (clash_core::Value::from(3), Value::from(3)),
            (
                clash_core::Value::from(Real::of(0.5)),
                Value::from(Real::of(0.5)),
            ),
            (clash_core::Value::from("x"), Value::from("x")),
        ];
        for (core, value) in cases {
            assert_eq!(Value::from(core.clone()), value);
            assert_eq!(clash_core::Value::try_from(value.clone()), Ok(core.clone()));
            // What both can hold hashes the same in either.
            assert_eq!(Hash128::of(&core), Hash128::of(&value));
        }
        for wider in [
            Value::Null,
            Value::list([]),
            Value::map([("a", Value::Null)]),
            Value::from(Handle::new(KindId::new(0), 0)),
            Value::from(DefRef::new(DefTypeId::new(0), 0)),
        ] {
            assert_eq!(clash_core::Value::try_from(wider.clone()), Err(wider));
        }
    }

    #[test]
    fn a_map_hashes_by_key_whatever_order_it_was_built_in() {
        let forward = Value::map([
            ("a", Value::from(1)),
            ("b", Value::from(2)),
            ("c", Value::from(3)),
        ]);
        let backward = Value::map([
            ("c", Value::from(3)),
            ("b", Value::from(2)),
            ("a", Value::from(1)),
        ]);
        assert_eq!(forward, backward);
        assert_eq!(Hash128::of(&forward), Hash128::of(&backward));
        let keys: Vec<&str> = backward
            .as_map()
            .unwrap()
            .keys()
            .map(|key| key.as_ref())
            .collect();
        assert_eq!(keys, ["a", "b", "c"]);
        let changed = Value::map([
            ("a", Value::from(1)),
            ("b", Value::from(20)),
            ("c", Value::from(3)),
        ]);
        assert_ne!(Hash128::of(&changed), Hash128::of(&forward));
        // A key given twice keeps its last value.
        assert_eq!(
            Value::map([("a", Value::from(1)), ("a", Value::from(2))]),
            Value::map([("a", Value::from(2))])
        );
    }

    #[test]
    fn values_of_different_kinds_hash_differently() {
        let values = [
            Value::Null,
            Value::from(false),
            Value::from(0),
            Value::from(Real::ZERO),
            Value::from(""),
            Value::from(DefRef::new(DefTypeId::new(0), 0)),
            Value::from(Handle::new(KindId::new(0), 0)),
            Value::list([]),
            Value::map(core::iter::empty::<(&str, Value)>()),
            Value::list([Value::Null]),
        ];
        for (index, value) in values.iter().enumerate() {
            for other in &values[index + 1..] {
                assert_ne!(Hash128::of(value), Hash128::of(other), "{value} / {other}");
            }
        }
    }

    #[test]
    fn a_value_is_small_and_can_cross_threads() {
        fn assert_send_sync<T: Send + Sync>() {}
        assert_send_sync::<Value>();
        assert!(core::mem::size_of::<Value>() <= 24);
    }
}
