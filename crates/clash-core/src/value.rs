//! [`Value`], [`VarKey`] and [`VarMap`]: the values content stores, and
//! where it stores them.
//!
//! Mods keep their own state in *variables* on entities, cells, areas,
//! sites and the simulation itself (05 §6). A variable holds a [`Value`];
//! a [`VarMap`] is the set of variables on one thing.
//!
//! This is the base that the expression language (document 05) extends.
//! [`Value`] is marked `#[non_exhaustive]`: more kinds of value will be
//! added, so a `match` on it outside this crate needs a wildcard arm.
//!
//! A variable may hold anything the expression language can compute
//! (05 §3, §6), so beside the four plain kinds a `Value` holds a list, a
//! map, a reference to a definition and a reference to a subject. The
//! two references are numbers that mean something only to the
//! simulation, which made them: this crate stores them and gives them no
//! meaning.

use core::borrow::Borrow;
use core::fmt;
use std::collections::BTreeMap;
use std::collections::btree_map;

use serde::{Deserialize, Serialize};

use crate::Real;
use crate::hash::{StateHash, StateHasher};

/// A value a variable can hold.
///
/// ```
/// use clash_core::{Real, Value};
///
/// let alarm = Value::from(true);
/// let kills = Value::from(3);
/// let heat = Value::from(Real::of(310.5));
/// let name = Value::from("ash");
///
/// assert_eq!(alarm.as_bool(), Some(true));
/// assert_eq!(kills.as_int(), Some(3));
/// assert_eq!(kills.as_real(), None);
/// assert_eq!(heat.type_name(), "real");
/// assert_eq!(name.to_string(), "ash");
/// ```
#[derive(Clone, PartialEq, Eq, Hash, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum Value {
    /// `true` or `false`.
    Bool(bool),
    /// A 64-bit integer.
    Int(i64),
    /// A deterministic real number, in SI units where it is a quantity.
    Real(Real),
    /// A text.
    Str(String),
    /// Nothing. Only inside a list or a map: a variable that holds
    /// nothing is simply not set.
    Null,
    /// A reference to a definition: the number of its type and its
    /// number within the type, as the simulation knows them (04 §7.2).
    Id {
        /// The type of definition.
        ty: u16,
        /// The definition's number within its type.
        index: u32,
    },
    /// A reference to a subject (05 §5.1): the number of its kind and
    /// sixty-four bits that mean something only to the simulation.
    Subject {
        /// The kind of subject.
        kind: u16,
        /// The simulation's own reference.
        payload: u64,
    },
    /// Ordered values.
    List(Vec<Value>),
    /// Values by key, in order of key.
    Map(BTreeMap<String, Value>),
}

impl Value {
    /// The name of the value's type as the expression language writes it:
    /// `"bool"`, `"int"`, `"real"`, `"string"`, `"null"`, `"id"`,
    /// `"subject"`, `"list"` or `"map"`.
    pub const fn type_name(&self) -> &'static str {
        match self {
            Value::Bool(_) => "bool",
            Value::Int(_) => "int",
            Value::Real(_) => "real",
            Value::Str(_) => "string",
            Value::Null => "null",
            Value::Id { .. } => "id",
            Value::Subject { .. } => "subject",
            Value::List(_) => "list",
            Value::Map(_) => "map",
        }
    }

    /// How deeply lists and maps are nested in the value: zero for a
    /// plain value, one for a list of plain values. Whoever stores a
    /// value keeps this within what the codec can write (01 §11.1).
    pub fn nesting(&self) -> usize {
        match self {
            Value::List(items) => 1 + items.iter().map(Value::nesting).max().unwrap_or(0),
            Value::Map(entries) => 1 + entries.values().map(Value::nesting).max().unwrap_or(0),
            _ => 0,
        }
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

    /// The text, if this is one.
    pub fn as_str(&self) -> Option<&str> {
        match self {
            Value::Str(value) => Some(value),
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

impl From<String> for Value {
    fn from(value: String) -> Value {
        Value::Str(value)
    }
}

impl From<&str> for Value {
    fn from(value: &str) -> Value {
        Value::Str(value.to_owned())
    }
}

/// The value as plain text: `true`, `42`, `3.5`, or a text as it is,
/// without quotes; `null`; a list as `[a, b]` and a map as
/// `{key: value}`. A definition is written `id#N` and a subject
/// `subject#K:N`, because only the simulation knows their names.
impl fmt::Display for Value {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Value::Bool(value) => fmt::Display::fmt(value, f),
            Value::Int(value) => fmt::Display::fmt(value, f),
            Value::Real(value) => fmt::Display::fmt(value, f),
            Value::Str(value) => f.write_str(value),
            Value::Null => f.write_str("null"),
            Value::Id { index, .. } => write!(f, "id#{index}"),
            Value::Subject { kind, payload } => write!(f, "subject#{kind}:{payload}"),
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

/// A one-byte tag for the kind of value, then the value. The tags and
/// what follows them are those of the expression language's value, so a
/// value hashes the same in either form.
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
                value.state_hash(hasher);
            }
            Value::Null => hasher.write_u8(4),
            Value::Id { ty, index } => {
                hasher.write_u8(5);
                hasher.write_u16(*ty);
                hasher.write_u32(*index);
            }
            Value::Subject { kind, payload } => {
                hasher.write_u8(6);
                hasher.write_u16(*kind);
                hasher.write_u64(*payload);
            }
            Value::List(items) => {
                hasher.write_u8(7);
                hasher.write_len(items.len());
                for item in items {
                    item.state_hash(hasher);
                }
            }
            Value::Map(entries) => {
                hasher.write_u8(8);
                hasher.write_len(entries.len());
                for (key, value) in entries {
                    hasher.write_str(key);
                    value.state_hash(hasher);
                }
            }
        }
    }
}

/// The name of a variable: a qualified id such as `core:alarm_raised`
/// (04 §4.1).
///
/// A `VarKey` is a text. Whether the name is declared, and what type its
/// values must have, is checked by the layers that load and run rules, not
/// here.
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Debug, Serialize, Deserialize)]
#[serde(transparent)]
pub struct VarKey(String);

impl VarKey {
    /// The key with this name.
    pub fn new(name: impl Into<String>) -> VarKey {
        VarKey(name.into())
    }

    /// The name.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl Borrow<str> for VarKey {
    fn borrow(&self) -> &str {
        &self.0
    }
}

impl From<&str> for VarKey {
    fn from(name: &str) -> VarKey {
        VarKey(name.to_owned())
    }
}

impl From<String> for VarKey {
    fn from(name: String) -> VarKey {
        VarKey(name)
    }
}

impl fmt::Display for VarKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl StateHash for VarKey {
    fn state_hash(&self, hasher: &mut StateHasher) {
        self.0.state_hash(hasher);
    }
}

/// The variables on one thing: a map from [`VarKey`] to [`Value`], kept in
/// order of key.
///
/// It iterates, serialises and hashes in that order, so it is safe to hold
/// in simulation state.
///
/// ```
/// use clash_core::{Value, VarMap};
///
/// let mut vars = VarMap::new();
/// vars.set("core:kills", 2);
/// vars.set("core:alarm_raised", true);
///
/// assert_eq!(vars.get("core:kills"), Some(&Value::Int(2)));
/// assert_eq!(vars.get("core:missing"), None);
/// let names: Vec<&str> = vars.iter().map(|(key, _)| key.as_str()).collect();
/// assert_eq!(names, ["core:alarm_raised", "core:kills"]);
/// ```
#[derive(Clone, PartialEq, Eq, Default, Debug, Serialize, Deserialize)]
#[serde(transparent)]
pub struct VarMap {
    vars: BTreeMap<VarKey, Value>,
}

impl VarMap {
    /// A map with no variables.
    pub const fn new() -> VarMap {
        VarMap {
            vars: BTreeMap::new(),
        }
    }

    /// The number of variables.
    pub fn len(&self) -> usize {
        self.vars.len()
    }

    /// Whether there are no variables.
    pub fn is_empty(&self) -> bool {
        self.vars.is_empty()
    }

    /// The value of a variable, if it has been set.
    pub fn get(&self, key: &str) -> Option<&Value> {
        self.vars.get(key)
    }

    /// Whether a variable has been set.
    pub fn contains(&self, key: &str) -> bool {
        self.vars.contains_key(key)
    }

    /// Sets a variable and returns the value it had before, if any.
    pub fn set(&mut self, key: impl Into<VarKey>, value: impl Into<Value>) -> Option<Value> {
        self.vars.insert(key.into(), value.into())
    }

    /// Unsets a variable and returns the value it had, if any.
    pub fn remove(&mut self, key: &str) -> Option<Value> {
        self.vars.remove(key)
    }

    /// Unsets every variable.
    pub fn clear(&mut self) {
        self.vars.clear();
    }

    /// Every variable and its value, in order of key.
    pub fn iter(&self) -> btree_map::Iter<'_, VarKey, Value> {
        self.vars.iter()
    }
}

impl<'a> IntoIterator for &'a VarMap {
    type Item = (&'a VarKey, &'a Value);
    type IntoIter = btree_map::Iter<'a, VarKey, Value>;

    fn into_iter(self) -> Self::IntoIter {
        self.vars.iter()
    }
}

impl<K: Into<VarKey>, V: Into<Value>> FromIterator<(K, V)> for VarMap {
    fn from_iter<I: IntoIterator<Item = (K, V)>>(iter: I) -> VarMap {
        VarMap {
            vars: iter
                .into_iter()
                .map(|(key, value)| (key.into(), value.into()))
                .collect(),
        }
    }
}

impl StateHash for VarMap {
    fn state_hash(&self, hasher: &mut StateHasher) {
        self.vars.state_hash(hasher);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Hash128;

    #[test]
    fn values_know_their_type() {
        let values = [
            Value::from(true),
            Value::from(7_i64),
            Value::from(Real::of(1.5)),
            Value::from("ash"),
        ];
        let names: Vec<&str> = values.iter().map(Value::type_name).collect();
        assert_eq!(names, ["bool", "int", "real", "string"]);
        assert_eq!(values[0].as_bool(), Some(true));
        assert_eq!(values[1].as_int(), Some(7));
        assert_eq!(values[2].as_real(), Some(Real::of(1.5)));
        assert_eq!(values[3].as_str(), Some("ash"));
        // Accessors do not convert between types.
        assert_eq!(values[1].as_real(), None);
        assert_eq!(values[2].as_int(), None);
        assert_eq!(values[0].as_str(), None);
        assert_eq!(values[3].as_bool(), None);
        assert_eq!(Value::from(7_i32), Value::Int(7));
        assert_eq!(Value::from("ash".to_owned()), Value::from("ash"));
    }

    #[test]
    fn wider_values_are_stored_named_hashed_and_serialised() {
        let list = Value::List(vec![Value::from(1), Value::Null, Value::from("x")]);
        let map = Value::Map(
            [
                ("b".to_owned(), Value::from(2)),
                ("a".to_owned(), list.clone()),
            ]
            .into_iter()
            .collect(),
        );
        let id = Value::Id { ty: 3, index: 9 };
        let subject = Value::Subject {
            kind: 1,
            payload: 77,
        };
        let names: Vec<&str> = [&Value::Null, &id, &subject, &list, &map]
            .iter()
            .map(|value| value.type_name())
            .collect();
        assert_eq!(names, ["null", "id", "subject", "list", "map"]);
        assert_eq!(list.to_string(), "[1, null, x]");
        assert_eq!(map.to_string(), "{a: [1, null, x], b: 2}");
        assert_eq!(id.to_string(), "id#9");
        assert_eq!(subject.to_string(), "subject#1:77");
        assert_eq!(Value::from(1).nesting(), 0);
        assert_eq!(list.nesting(), 1);
        assert_eq!(map.nesting(), 2);

        let all = [Value::Null, id, subject, list, map];
        for (index, value) in all.iter().enumerate() {
            for other in &all[index + 1..] {
                assert_ne!(Hash128::of(value), Hash128::of(other));
            }
            let bytes = crate::codec::to_bytes(value).unwrap();
            assert_eq!(&crate::codec::from_bytes::<Value>(&bytes).unwrap(), value);
            let json = serde_json::to_string(value).unwrap();
            assert_eq!(&serde_json::from_str::<Value>(&json).unwrap(), value);
        }
    }

    #[test]
    fn values_display_plainly() {
        assert_eq!(Value::from(false).to_string(), "false");
        assert_eq!(Value::from(-3).to_string(), "-3");
        assert_eq!(Value::from(Real::of(2.5)).to_string(), "2.5");
        assert_eq!(Value::from("two words").to_string(), "two words");
    }

    #[test]
    fn values_of_different_types_hash_differently() {
        let hashes = [
            Hash128::of(&Value::from(false)),
            Hash128::of(&Value::from(0)),
            Hash128::of(&Value::from(Real::ZERO)),
            Hash128::of(&Value::from("")),
            Hash128::of(&Value::from(true)),
            Hash128::of(&Value::from(1)),
        ];
        for (index, hash) in hashes.iter().enumerate() {
            for other in &hashes[index + 1..] {
                assert_ne!(hash, other);
            }
        }
    }

    #[test]
    fn values_serialise_with_their_type() {
        let cases = [
            (Value::from(true), r#"{"bool":true}"#),
            (Value::from(3), r#"{"int":3}"#),
            (Value::from(Real::of(3.0)), r#"{"real":3.0}"#),
            (Value::from("x"), r#"{"str":"x"}"#),
        ];
        for (value, json) in cases {
            assert_eq!(serde_json::to_string(&value).unwrap(), json);
            assert_eq!(serde_json::from_str::<Value>(json).unwrap(), value);
        }
    }

    #[test]
    fn a_var_map_sets_gets_and_removes() {
        let mut vars = VarMap::new();
        assert!(vars.is_empty());
        assert_eq!(vars.set("m:b", 1), None);
        assert_eq!(vars.set(VarKey::new("m:a"), "text"), None);
        assert_eq!(vars.set("m:b".to_owned(), 2), Some(Value::Int(1)));
        assert_eq!(vars.len(), 2);
        assert!(vars.contains("m:a"));
        assert_eq!(vars.get("m:b"), Some(&Value::Int(2)));
        assert_eq!(vars.remove("m:a"), Some(Value::from("text")));
        assert_eq!(vars.remove("m:a"), None);
        vars.clear();
        assert!(vars.is_empty());
    }

    #[test]
    fn a_var_map_is_ordered_by_key_however_it_was_built() {
        let forward: VarMap = [("m:a", 1), ("m:b", 2), ("m:c", 3)].into_iter().collect();
        let backward: VarMap = [("m:c", 3), ("m:b", 2), ("m:a", 1)].into_iter().collect();
        assert_eq!(forward, backward);
        assert_eq!(Hash128::of(&forward), Hash128::of(&backward));
        let keys: Vec<String> = (&backward)
            .into_iter()
            .map(|(key, _)| key.to_string())
            .collect();
        assert_eq!(keys, ["m:a", "m:b", "m:c"]);

        let json = serde_json::to_string(&backward).unwrap();
        assert_eq!(json, r#"{"m:a":{"int":1},"m:b":{"int":2},"m:c":{"int":3}}"#);
        assert_eq!(serde_json::from_str::<VarMap>(&json).unwrap(), forward);

        let mut changed = forward.clone();
        changed.set("m:b", 20);
        assert_ne!(Hash128::of(&changed), Hash128::of(&forward));
    }
}
