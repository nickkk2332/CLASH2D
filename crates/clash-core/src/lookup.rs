//! [`LookupMap`]: a hash map that cannot be iterated.

// The one place in the simulation that may name a std hash map: this
// wrapper is what makes it safe to use.
#![allow(
    clippy::disallowed_types,
    reason = "LookupMap wraps a hash map and offers no iteration (01 §4)"
)]

use core::borrow::Borrow;
use core::fmt;
use core::hash::{BuildHasher, Hash, Hasher};
use core::marker::PhantomData;
use std::collections::HashMap;

use serde::de::{SeqAccess, Visitor};
use serde::ser::SerializeSeq;
use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::hash::{StateHash, StateHasher};

/// A map for looking things up by key, with no way to iterate it.
///
/// The order in which a hash map holds its entries depends on its history
/// and its hashing, so anything computed by walking one is not
/// deterministic. Simulation state therefore never holds a std hash map
/// (01 §4, enforced by lint). `LookupMap` is the alternative when all that
/// is needed is fast lookup: it offers `get`, `insert` and `remove`, and
/// has no `iter`, `keys`, `values`, `drain` or `retain`.
///
/// ```
/// use clash_core::LookupMap;
///
/// let mut ages = LookupMap::new();
/// ages.insert("oak", 300);
/// assert_eq!(ages.get("oak"), Some(&300));
/// assert_eq!(ages.remove("oak"), Some(300));
/// assert!(ages.is_empty());
/// ```
///
/// Iterating does not compile:
///
/// ```compile_fail
/// use clash_core::LookupMap;
///
/// let mut ages = LookupMap::new();
/// ages.insert("oak", 300);
/// for (name, age) in &ages {}
/// ```
///
/// ```compile_fail
/// use clash_core::LookupMap;
///
/// let mut ages = LookupMap::new();
/// ages.insert("oak", 300);
/// let total: i32 = ages.iter().map(|(_, age)| *age).sum();
/// ```
///
/// ```compile_fail
/// use clash_core::LookupMap;
///
/// let mut ages = LookupMap::new();
/// ages.insert("oak", 300);
/// let names: Vec<&str> = ages.keys().copied().collect();
/// ```
///
/// ```compile_fail
/// use clash_core::LookupMap;
///
/// let mut ages = LookupMap::new();
/// ages.insert("oak", 300);
/// let total: i32 = ages.into_iter().map(|(_, age)| age).sum();
/// ```
///
/// # When everything must be visited
///
/// Saving, hashing and the occasional sweep do need every entry. For those
/// there are [`sorted_keys`](LookupMap::sorted_keys),
/// [`sorted_entries`](LookupMap::sorted_entries) and
/// [`into_sorted_vec`](LookupMap::into_sorted_vec), which return the
/// contents **sorted by key**, so the order is the same on every run. Each
/// call allocates and sorts. If something is walked often, keep it in a
/// `BTreeMap` or a `Vec` instead.
///
/// Serialisation and [`StateHash`] also go in key order.
pub struct LookupMap<K, V> {
    map: HashMap<K, V, FixedState>,
}

impl<K, V> LookupMap<K, V> {
    /// An empty map.
    pub const fn new() -> LookupMap<K, V> {
        LookupMap {
            map: HashMap::with_hasher(FixedState),
        }
    }

    /// An empty map with room for at least `capacity` entries.
    pub fn with_capacity(capacity: usize) -> LookupMap<K, V> {
        LookupMap {
            map: HashMap::with_capacity_and_hasher(capacity, FixedState),
        }
    }

    /// The number of entries.
    pub fn len(&self) -> usize {
        self.map.len()
    }

    /// Whether there are no entries.
    pub fn is_empty(&self) -> bool {
        self.map.is_empty()
    }

    /// Removes every entry.
    pub fn clear(&mut self) {
        self.map.clear();
    }
}

impl<K: Eq + Hash, V> LookupMap<K, V> {
    /// The value for a key, if there is one.
    pub fn get<Q>(&self, key: &Q) -> Option<&V>
    where
        K: Borrow<Q>,
        Q: Eq + Hash + ?Sized,
    {
        self.map.get(key)
    }

    /// The value for a key, to change in place, if there is one.
    pub fn get_mut<Q>(&mut self, key: &Q) -> Option<&mut V>
    where
        K: Borrow<Q>,
        Q: Eq + Hash + ?Sized,
    {
        self.map.get_mut(key)
    }

    /// Whether there is a value for a key.
    pub fn contains_key<Q>(&self, key: &Q) -> bool
    where
        K: Borrow<Q>,
        Q: Eq + Hash + ?Sized,
    {
        self.map.contains_key(key)
    }

    /// Sets the value for a key and returns the value it replaced, if any.
    pub fn insert(&mut self, key: K, value: V) -> Option<V> {
        self.map.insert(key, value)
    }

    /// Removes a key and returns its value, if it had one.
    pub fn remove<Q>(&mut self, key: &Q) -> Option<V>
    where
        K: Borrow<Q>,
        Q: Eq + Hash + ?Sized,
    {
        self.map.remove(key)
    }

    /// The value for a key, first inserting the result of `make` if the key
    /// has none.
    pub fn get_or_insert_with(&mut self, key: K, make: impl FnOnce() -> V) -> &mut V {
        self.map.entry(key).or_insert_with(make)
    }
}

impl<K: Ord, V> LookupMap<K, V> {
    /// Every key, in ascending order. Allocates and sorts on each call.
    pub fn sorted_keys(&self) -> Vec<K>
    where
        K: Clone,
    {
        let mut keys: Vec<K> = self.map.keys().cloned().collect();
        keys.sort_unstable();
        keys
    }

    /// Every entry, in ascending order of key. Allocates and sorts on each
    /// call.
    pub fn sorted_entries(&self) -> Vec<(&K, &V)> {
        let mut entries: Vec<(&K, &V)> = self.map.iter().collect();
        entries.sort_unstable_by(|a, b| a.0.cmp(b.0));
        entries
    }

    /// Takes the map apart into its entries, in ascending order of key.
    pub fn into_sorted_vec(self) -> Vec<(K, V)> {
        let mut entries: Vec<(K, V)> = self.map.into_iter().collect();
        entries.sort_unstable_by(|a, b| a.0.cmp(&b.0));
        entries
    }
}

impl<K, V> Default for LookupMap<K, V> {
    fn default() -> LookupMap<K, V> {
        LookupMap::new()
    }
}

impl<K: Clone, V: Clone> Clone for LookupMap<K, V> {
    fn clone(&self) -> LookupMap<K, V> {
        LookupMap {
            map: self.map.clone(),
        }
    }
}

/// Two maps are equal when they hold the same entries, however each was
/// built.
impl<K: Eq + Hash, V: PartialEq> PartialEq for LookupMap<K, V> {
    fn eq(&self, other: &LookupMap<K, V>) -> bool {
        self.map == other.map
    }
}

impl<K: Eq + Hash, V: Eq> Eq for LookupMap<K, V> {}

/// Shows only the number of entries: printing them would expose the
/// internal order.
impl<K, V> fmt::Debug for LookupMap<K, V> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("LookupMap")
            .field("len", &self.map.len())
            .finish_non_exhaustive()
    }
}

impl<K: Eq + Hash, V> FromIterator<(K, V)> for LookupMap<K, V> {
    fn from_iter<I: IntoIterator<Item = (K, V)>>(iter: I) -> LookupMap<K, V> {
        let mut map = LookupMap::new();
        map.extend(iter);
        map
    }
}

/// Inserts each pair in turn. A later pair replaces an earlier one with the
/// same key.
impl<K: Eq + Hash, V> Extend<(K, V)> for LookupMap<K, V> {
    fn extend<I: IntoIterator<Item = (K, V)>>(&mut self, iter: I) {
        self.map.extend(iter);
    }
}

/// The number of entries, then each key and value in key order.
impl<K: Ord + StateHash, V: StateHash> StateHash for LookupMap<K, V> {
    fn state_hash(&self, hasher: &mut StateHasher) {
        hasher.write_len(self.len());
        for (key, value) in self.sorted_entries() {
            key.state_hash(hasher);
            value.state_hash(hasher);
        }
    }
}

/// Serialised as a sequence of `(key, value)` pairs in key order.
impl<K: Ord + Serialize, V: Serialize> Serialize for LookupMap<K, V> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut sequence = serializer.serialize_seq(Some(self.len()))?;
        for entry in self.sorted_entries() {
            sequence.serialize_element(&entry)?;
        }
        sequence.end()
    }
}

/// Reads a sequence of `(key, value)` pairs. A key that appears twice is an
/// error.
impl<'de, K, V> Deserialize<'de> for LookupMap<K, V>
where
    K: Eq + Hash + Deserialize<'de>,
    V: Deserialize<'de>,
{
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<LookupMap<K, V>, D::Error> {
        struct Pairs<K, V>(PhantomData<(K, V)>);

        impl<'de, K, V> Visitor<'de> for Pairs<K, V>
        where
            K: Eq + Hash + Deserialize<'de>,
            V: Deserialize<'de>,
        {
            type Value = LookupMap<K, V>;

            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str("a sequence of key and value pairs")
            }

            fn visit_seq<A: SeqAccess<'de>>(self, mut access: A) -> Result<Self::Value, A::Error> {
                let mut map = LookupMap::with_capacity(access.size_hint().unwrap_or(0).min(4096));
                while let Some((key, value)) = access.next_element::<(K, V)>()? {
                    if map.insert(key, value).is_some() {
                        return Err(serde::de::Error::custom("a key appears twice"));
                    }
                }
                Ok(map)
            }
        }

        deserializer.deserialize_seq(Pairs(PhantomData))
    }
}

/// Builds [`FixedHasher`]s. Unlike the std default it reads no OS
/// randomness, so a `LookupMap` behaves identically on every run.
#[derive(Clone, Copy, Default)]
pub(crate) struct FixedState;

impl BuildHasher for FixedState {
    type Hasher = FixedHasher;

    fn build_hasher(&self) -> FixedHasher {
        FixedHasher::default()
    }
}

/// A small, fast hasher for map keys: multiply-and-rotate per word with a
/// strong final mix. It is for bucketing only and is not part of any
/// contract: nothing observable depends on the values it produces.
#[derive(Clone, Copy, Default)]
pub(crate) struct FixedHasher {
    state: u64,
}

impl FixedHasher {
    const MULTIPLIER: u64 = 0x517c_c1b7_2722_0a95;

    fn add(&mut self, word: u64) {
        self.state = (self.state.rotate_left(5) ^ word).wrapping_mul(Self::MULTIPLIER);
    }
}

impl Hasher for FixedHasher {
    fn write(&mut self, bytes: &[u8]) {
        for chunk in bytes.chunks(8) {
            let mut word = [0_u8; 8];
            word[..chunk.len()].copy_from_slice(chunk);
            self.add(u64::from_le_bytes(word));
        }
    }

    fn write_u8(&mut self, value: u8) {
        self.add(u64::from(value));
    }

    fn write_u16(&mut self, value: u16) {
        self.add(u64::from(value));
    }

    fn write_u32(&mut self, value: u32) {
        self.add(u64::from(value));
    }

    fn write_u64(&mut self, value: u64) {
        self.add(value);
    }

    fn write_usize(&mut self, value: usize) {
        self.add(value as u64);
    }

    fn finish(&self) -> u64 {
        // The splitmix64 finaliser: spreads every input bit over the whole
        // word, so both the high and the low bits are good bucket indices.
        let mut z = self.state;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        z ^ (z >> 31)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hash::Hash128;

    #[test]
    fn lookup_insert_and_remove() {
        let mut map: LookupMap<u32, &str> = LookupMap::new();
        assert!(map.is_empty());
        assert_eq!(map.insert(1, "one"), None);
        assert_eq!(map.insert(2, "two"), None);
        assert_eq!(map.insert(1, "uno"), Some("one"));
        assert_eq!(map.len(), 2);
        assert_eq!(map.get(&1), Some(&"uno"));
        assert_eq!(map.get(&3), None);
        assert!(map.contains_key(&2));
        if let Some(value) = map.get_mut(&2) {
            *value = "dos";
        }
        assert_eq!(map.remove(&2), Some("dos"));
        assert_eq!(map.remove(&2), None);
        assert_eq!(*map.get_or_insert_with(9, || "nine"), "nine");
        assert_eq!(*map.get_or_insert_with(9, || "neuf"), "nine");
        map.clear();
        assert!(map.is_empty());
    }

    #[test]
    fn string_keys_are_looked_up_by_str() {
        let mut map: LookupMap<String, u8> = LookupMap::with_capacity(4);
        map.insert("glass".to_owned(), 1);
        assert_eq!(map.get("glass"), Some(&1));
        assert!(map.contains_key("glass"));
        assert_eq!(map.remove("glass"), Some(1));
    }

    #[test]
    fn sorted_access_ignores_insertion_order() {
        let forward: LookupMap<u32, u32> = (0..500).map(|n| (n, n * n)).collect();
        let backward: LookupMap<u32, u32> = (0..500).rev().map(|n| (n, n * n)).collect();
        assert_eq!(forward, backward);
        assert_eq!(forward.sorted_keys(), (0..500).collect::<Vec<u32>>());
        assert_eq!(forward.sorted_keys(), backward.sorted_keys());
        assert_eq!(forward.sorted_entries(), backward.sorted_entries());
        assert_eq!(forward.sorted_entries()[3], (&3, &9));
        assert_eq!(Hash128::of(&forward), Hash128::of(&backward));
        assert_eq!(
            serde_json::to_string(&forward).unwrap(),
            serde_json::to_string(&backward).unwrap()
        );
        let entries = backward.into_sorted_vec();
        assert_eq!(entries.len(), 500);
        assert!(entries.windows(2).all(|pair| pair[0].0 < pair[1].0));
    }

    #[test]
    fn the_hash_follows_the_contents() {
        let mut a: LookupMap<u32, u32> = LookupMap::new();
        a.insert(1, 10);
        a.insert(2, 20);
        let mut b = a.clone();
        assert_eq!(Hash128::of(&a), Hash128::of(&b));
        b.insert(2, 21);
        assert_ne!(a, b);
        assert_ne!(Hash128::of(&a), Hash128::of(&b));

        let mut by_hand = StateHasher::new();
        by_hand.write_len(2);
        for word in [1_u32, 10, 2, 20] {
            by_hand.write_u32(word);
        }
        assert_eq!(Hash128::of(&a), by_hand.finish());
    }

    #[test]
    fn serde_round_trips_in_key_order() {
        let map: LookupMap<u8, String> = [(2, "b".to_owned()), (1, "a".to_owned())]
            .into_iter()
            .collect();
        let json = serde_json::to_string(&map).unwrap();
        assert_eq!(json, r#"[[1,"a"],[2,"b"]]"#);
        let back: LookupMap<u8, String> = serde_json::from_str(&json).unwrap();
        assert_eq!(back, map);
        assert!(serde_json::from_str::<LookupMap<u8, String>>(r#"[[1,"a"],[1,"b"]]"#).is_err());
    }

    #[test]
    fn debug_does_not_show_entries() {
        let map: LookupMap<u8, u8> = [(1, 2)].into_iter().collect();
        assert_eq!(format!("{map:?}"), "LookupMap { len: 1, .. }");
    }

    #[test]
    fn the_key_hasher_spreads_similar_keys() {
        fn hash(value: u64) -> u64 {
            let mut hasher = FixedHasher::default();
            hasher.write_u64(value);
            hasher.finish()
        }
        // Keys that differ only in high bits must still differ in the low
        // bits a table uses to pick a bucket.
        let mut low_bits: Vec<u64> = (0..64).map(|n| hash(n << 32) & 0xfff).collect();
        low_bits.sort_unstable();
        low_bits.dedup();
        assert!(low_bits.len() > 56, "only {} distinct", low_bits.len());

        let mut bytes = FixedHasher::default();
        bytes.write(b"hello world");
        let mut other = FixedHasher::default();
        other.write(b"hello worle");
        assert_ne!(bytes.finish(), other.finish());
    }
}
