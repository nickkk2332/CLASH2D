//! State hashing: the [`StateHash`] trait, the fixed 128-bit
//! [`StateHasher`] and its result, [`Hash128`].
//!
//! Golden-hash tests, save-transparency tests and replay all compare state
//! hashes (01 §4, §12), so a hash must depend only on the state: not on the
//! platform, the Rust release, memory addresses or the order in which a
//! hash map happens to hold things.
//!
//! # Writing an implementation
//!
//! Feed every field that is part of the state, in a fixed order:
//!
//! ```
//! use clash_core::{Hash128, Real, StateHash, StateHasher};
//!
//! struct Tank {
//!     capacity: Real,
//!     level: Real,
//!     label: String,
//! }
//!
//! impl StateHash for Tank {
//!     fn state_hash(&self, hasher: &mut StateHasher) {
//!         self.capacity.state_hash(hasher);
//!         self.level.state_hash(hasher);
//!         self.label.state_hash(hasher);
//!     }
//! }
//!
//! let tank = Tank { capacity: Real::of(40.0), level: Real::of(12.5), label: "aft".into() };
//! assert_eq!(Hash128::of(&tank), Hash128::of(&tank));
//! ```
//!
//! [`impl_state_hash!`](crate::impl_state_hash) writes exactly that for a
//! struct with named fields. For an enum, write a discriminant first
//! (`hasher.write_u8(n)`), then the variant's fields.
//!
//! Leave out caches and anything else that can be rebuilt from the rest.
//! Hash a collection only through an order that is part of the state: a
//! `Vec`, a `BTreeMap`, or a [`LookupMap`](crate::LookupMap), which hashes
//! in key order. Plain floats have no implementation on purpose: state
//! holds [`Real`](crate::Real).

use core::fmt;
use core::str::FromStr;
use std::collections::{BTreeMap, BTreeSet, VecDeque};

use serde::{Deserialize, Serialize};

use crate::error::ParseHashError;

/// A 128-bit state hash.
///
/// It is written, and parsed, as 32 lower-case hexadecimal digits, which is
/// also how it is serialised.
///
/// ```
/// use clash_core::Hash128;
///
/// let hash = Hash128::of(&(1_u32, "two"));
/// let text = hash.to_string();
/// assert_eq!(text.len(), 32);
/// assert_eq!(text.parse::<Hash128>(), Ok(hash));
/// ```
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct Hash128(pub u128);

impl Hash128 {
    /// The hash of one value: a new hasher, fed the value, finished.
    pub fn of<T: StateHash + ?Sized>(value: &T) -> Hash128 {
        let mut hasher = StateHasher::new();
        value.state_hash(&mut hasher);
        hasher.finish()
    }
}

impl fmt::Display for Hash128 {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:032x}", self.0)
    }
}

impl fmt::Debug for Hash128 {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Hash128({:032x})", self.0)
    }
}

impl FromStr for Hash128 {
    type Err = ParseHashError;

    fn from_str(text: &str) -> Result<Hash128, ParseHashError> {
        let length = text.chars().count();
        if length != 32 {
            return Err(ParseHashError::WrongLength(length));
        }
        let mut value = 0_u128;
        for character in text.chars() {
            let digit = character
                .to_digit(16)
                .ok_or(ParseHashError::NotHex(character))?;
            value = (value << 4) | u128::from(digit);
        }
        Ok(Hash128(value))
    }
}

impl Serialize for Hash128 {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_str(self)
    }
}

impl<'de> Deserialize<'de> for Hash128 {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Hash128, D::Error> {
        let text = String::deserialize(deserializer)?;
        text.parse().map_err(serde::de::Error::custom)
    }
}

/// The number of bytes the hasher mixes at a time.
const BLOCK: usize = 16;

const C1: u64 = 0x87c3_7b91_1142_53d5;
const C2: u64 = 0x4cf5_ad43_2745_937f;

/// The engine's one state hasher.
///
/// The algorithm is fixed and is part of the cross-platform contract: the
/// result is MurmurHash3 (the `x64_128` variant, seed 0) of the bytes
/// written, with `h1` as the upper 64 bits of the [`Hash128`] and `h2` as
/// the lower. The reference vectors in this crate's tests pin it down.
///
/// Integers are written little-endian at their own width. Anything of
/// variable length ([`write_str`](StateHasher::write_str), slices, maps)
/// writes its length first, so that `["ab", "c"]` and `["a", "bc"]` hash
/// differently.
///
/// It is not a cryptographic hash. It detects divergence; it does not
/// resist an adversary.
#[derive(Clone, Debug)]
pub struct StateHasher {
    h1: u64,
    h2: u64,
    /// Bytes written but not yet mixed. Only the first `pending` are live.
    buffer: [u8; BLOCK],
    pending: usize,
    /// Total number of bytes written.
    length: u64,
}

impl StateHasher {
    /// A hasher that has been fed nothing.
    pub const fn new() -> StateHasher {
        StateHasher {
            h1: 0,
            h2: 0,
            buffer: [0; BLOCK],
            pending: 0,
            length: 0,
        }
    }

    /// Feeds raw bytes, with no length in front.
    pub fn write_bytes(&mut self, mut bytes: &[u8]) {
        self.length = self.length.wrapping_add(bytes.len() as u64);
        if self.pending > 0 {
            let take = (BLOCK - self.pending).min(bytes.len());
            self.buffer[self.pending..self.pending + take].copy_from_slice(&bytes[..take]);
            self.pending += take;
            bytes = &bytes[take..];
            if self.pending < BLOCK {
                return;
            }
            let block = self.buffer;
            self.mix_block(&block);
            self.pending = 0;
        }
        let mut blocks = bytes.chunks_exact(BLOCK);
        for block in &mut blocks {
            let mut whole = [0_u8; BLOCK];
            whole.copy_from_slice(block);
            self.mix_block(&whole);
        }
        let rest = blocks.remainder();
        self.buffer[..rest.len()].copy_from_slice(rest);
        self.pending = rest.len();
    }

    /// Feeds one byte.
    pub fn write_u8(&mut self, value: u8) {
        self.write_bytes(&[value]);
    }

    /// Feeds a `u16`, little-endian.
    pub fn write_u16(&mut self, value: u16) {
        self.write_bytes(&value.to_le_bytes());
    }

    /// Feeds a `u32`, little-endian.
    pub fn write_u32(&mut self, value: u32) {
        self.write_bytes(&value.to_le_bytes());
    }

    /// Feeds a `u64`, little-endian.
    pub fn write_u64(&mut self, value: u64) {
        self.write_bytes(&value.to_le_bytes());
    }

    /// Feeds a `u128`, little-endian.
    pub fn write_u128(&mut self, value: u128) {
        self.write_bytes(&value.to_le_bytes());
    }

    /// Feeds an `i64`, little-endian two's complement.
    pub fn write_i64(&mut self, value: i64) {
        self.write_bytes(&value.to_le_bytes());
    }

    /// Feeds a boolean as one byte, `1` or `0`.
    pub fn write_bool(&mut self, value: bool) {
        self.write_u8(u8::from(value));
    }

    /// Feeds a length or a count as a `u64`, whatever the platform's
    /// pointer width.
    pub fn write_len(&mut self, length: usize) {
        self.write_u64(length as u64);
    }

    /// Feeds a text: its length in bytes, then its UTF-8 bytes.
    pub fn write_str(&mut self, text: &str) {
        self.write_len(text.len());
        self.write_bytes(text.as_bytes());
    }

    /// The hash of everything fed so far. The hasher can go on being fed.
    pub fn finish(&self) -> Hash128 {
        let mut h1 = self.h1;
        let mut h2 = self.h2;

        // The bytes that did not fill a block.
        let mut tail = [0_u8; BLOCK];
        tail[..self.pending].copy_from_slice(&self.buffer[..self.pending]);
        let (k1, k2) = split_block(&tail);
        if self.pending > 8 {
            h2 ^= mix_k2(k2);
        }
        if self.pending > 0 {
            h1 ^= mix_k1(k1);
        }

        h1 ^= self.length;
        h2 ^= self.length;
        h1 = h1.wrapping_add(h2);
        h2 = h2.wrapping_add(h1);
        h1 = fmix64(h1);
        h2 = fmix64(h2);
        h1 = h1.wrapping_add(h2);
        h2 = h2.wrapping_add(h1);

        Hash128((u128::from(h1) << 64) | u128::from(h2))
    }

    fn mix_block(&mut self, block: &[u8; BLOCK]) {
        let (k1, k2) = split_block(block);

        self.h1 ^= mix_k1(k1);
        self.h1 = self.h1.rotate_left(27).wrapping_add(self.h2);
        self.h1 = self.h1.wrapping_mul(5).wrapping_add(0x52dc_e729);

        self.h2 ^= mix_k2(k2);
        self.h2 = self.h2.rotate_left(31).wrapping_add(self.h1);
        self.h2 = self.h2.wrapping_mul(5).wrapping_add(0x3849_5ab5);
    }
}

impl Default for StateHasher {
    fn default() -> StateHasher {
        StateHasher::new()
    }
}

/// The two little-endian words of a block.
fn split_block(block: &[u8; BLOCK]) -> (u64, u64) {
    let mut low = [0_u8; 8];
    let mut high = [0_u8; 8];
    low.copy_from_slice(&block[..8]);
    high.copy_from_slice(&block[8..]);
    (u64::from_le_bytes(low), u64::from_le_bytes(high))
}

fn mix_k1(k1: u64) -> u64 {
    k1.wrapping_mul(C1).rotate_left(31).wrapping_mul(C2)
}

fn mix_k2(k2: u64) -> u64 {
    k2.wrapping_mul(C2).rotate_left(33).wrapping_mul(C1)
}

fn fmix64(mut k: u64) -> u64 {
    k ^= k >> 33;
    k = k.wrapping_mul(0xff51_afd7_ed55_8ccd);
    k ^= k >> 33;
    k = k.wrapping_mul(0xc4ce_b9fe_1a85_ec53);
    k ^ (k >> 33)
}

/// State that can be hashed for golden tests, save transparency and replay.
///
/// Every type that is part of simulation state implements this (01 §4). See
/// the [module documentation](self) for how to write an implementation.
pub trait StateHash {
    /// Feeds this value to the hasher: every field that is part of the
    /// state, in a fixed order.
    fn state_hash(&self, hasher: &mut StateHasher);
}

/// Implements [`StateHash`] for a struct by hashing the named fields in the
/// order given.
///
/// List every field that is part of the state. A field left out is not
/// hashed, which is right only for caches.
///
/// ```
/// use clash_core::{impl_state_hash, Hash128, IVec3, Real};
///
/// struct Crate {
///     at: IVec3,
///     mass: Real,
/// }
/// impl_state_hash!(Crate { at, mass });
///
/// let a = Crate { at: IVec3::new(1, 2, 0), mass: Real::of(80.0) };
/// let b = Crate { at: IVec3::new(1, 2, 0), mass: Real::of(81.0) };
/// assert_ne!(Hash128::of(&a), Hash128::of(&b));
/// ```
#[macro_export]
macro_rules! impl_state_hash {
    ($type:ty { $($field:ident),* $(,)? }) => {
        impl $crate::StateHash for $type {
            fn state_hash(&self, hasher: &mut $crate::StateHasher) {
                $( $crate::StateHash::state_hash(&self.$field, hasher); )*
                let _ = hasher;
            }
        }
    };
}

macro_rules! state_hash_int {
    ($($int:ty),*) => {
        $(
            impl StateHash for $int {
                fn state_hash(&self, hasher: &mut StateHasher) {
                    hasher.write_bytes(&self.to_le_bytes());
                }
            }
        )*
    };
}

state_hash_int!(u8, u16, u32, u64, u128, i8, i16, i32, i64, i128);

/// Hashed as a `u64`, so the result does not depend on pointer width.
impl StateHash for usize {
    fn state_hash(&self, hasher: &mut StateHasher) {
        hasher.write_u64(*self as u64);
    }
}

/// Hashed as an `i64`, so the result does not depend on pointer width.
impl StateHash for isize {
    fn state_hash(&self, hasher: &mut StateHasher) {
        hasher.write_i64(*self as i64);
    }
}

impl StateHash for bool {
    fn state_hash(&self, hasher: &mut StateHasher) {
        hasher.write_bool(*self);
    }
}

impl StateHash for char {
    fn state_hash(&self, hasher: &mut StateHasher) {
        hasher.write_u32(u32::from(*self));
    }
}

impl StateHash for () {
    fn state_hash(&self, _hasher: &mut StateHasher) {}
}

impl StateHash for str {
    fn state_hash(&self, hasher: &mut StateHasher) {
        hasher.write_str(self);
    }
}

impl StateHash for String {
    fn state_hash(&self, hasher: &mut StateHasher) {
        hasher.write_str(self);
    }
}

impl StateHash for Hash128 {
    fn state_hash(&self, hasher: &mut StateHasher) {
        hasher.write_u128(self.0);
    }
}

/// Hashed by its exact bits, so the two zeros hash differently even though
/// they compare equal: they are different states.
impl StateHash for crate::Real {
    fn state_hash(&self, hasher: &mut StateHasher) {
        hasher.write_u64(self.get().to_bits());
    }
}

impl<T: StateHash + ?Sized> StateHash for &T {
    fn state_hash(&self, hasher: &mut StateHasher) {
        (**self).state_hash(hasher);
    }
}

impl<T: StateHash + ?Sized> StateHash for Box<T> {
    fn state_hash(&self, hasher: &mut StateHasher) {
        (**self).state_hash(hasher);
    }
}

/// `None` is the byte `0`. `Some(x)` is the byte `1`, then `x`.
impl<T: StateHash> StateHash for Option<T> {
    fn state_hash(&self, hasher: &mut StateHasher) {
        match self {
            None => hasher.write_u8(0),
            Some(value) => {
                hasher.write_u8(1);
                value.state_hash(hasher);
            }
        }
    }
}

/// The length, then each item in order.
impl<T: StateHash> StateHash for [T] {
    fn state_hash(&self, hasher: &mut StateHasher) {
        hasher.write_len(self.len());
        for item in self {
            item.state_hash(hasher);
        }
    }
}

/// The length, then each item in order.
impl<T: StateHash> StateHash for Vec<T> {
    fn state_hash(&self, hasher: &mut StateHasher) {
        self.as_slice().state_hash(hasher);
    }
}

/// Each item in order. The length is fixed by the type and is not written.
impl<T: StateHash, const N: usize> StateHash for [T; N] {
    fn state_hash(&self, hasher: &mut StateHasher) {
        for item in self {
            item.state_hash(hasher);
        }
    }
}

/// The length, then each item from front to back.
impl<T: StateHash> StateHash for VecDeque<T> {
    fn state_hash(&self, hasher: &mut StateHasher) {
        hasher.write_len(self.len());
        for item in self {
            item.state_hash(hasher);
        }
    }
}

/// The length, then each key and value in key order.
impl<K: StateHash, V: StateHash> StateHash for BTreeMap<K, V> {
    fn state_hash(&self, hasher: &mut StateHasher) {
        hasher.write_len(self.len());
        for (key, value) in self {
            key.state_hash(hasher);
            value.state_hash(hasher);
        }
    }
}

/// The length, then each item in order.
impl<T: StateHash> StateHash for BTreeSet<T> {
    fn state_hash(&self, hasher: &mut StateHasher) {
        hasher.write_len(self.len());
        for item in self {
            item.state_hash(hasher);
        }
    }
}

macro_rules! state_hash_tuple {
    ($($name:ident),+) => {
        /// Each element in order.
        impl<$($name: StateHash),+> StateHash for ($($name,)+) {
            #[allow(non_snake_case)]
            fn state_hash(&self, hasher: &mut StateHasher) {
                let ($($name,)+) = self;
                $( $name.state_hash(hasher); )+
            }
        }
    };
}

state_hash_tuple!(A);
state_hash_tuple!(A, B);
state_hash_tuple!(A, B, C);
state_hash_tuple!(A, B, C, D);
state_hash_tuple!(A, B, C, D, E);
state_hash_tuple!(A, B, C, D, E, F);

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Real;

    fn hash_bytes(bytes: &[u8]) -> Hash128 {
        let mut hasher = StateHasher::new();
        hasher.write_bytes(bytes);
        hasher.finish()
    }

    #[test]
    fn splitting_the_input_does_not_change_the_hash() {
        let data: Vec<u8> = (0..=255).collect();
        let whole = hash_bytes(&data);
        for split in 0..data.len() {
            let mut hasher = StateHasher::new();
            hasher.write_bytes(&data[..split]);
            hasher.write_bytes(&data[split..]);
            assert_eq!(hasher.finish(), whole, "split at {split}");
        }
        let mut hasher = StateHasher::new();
        for byte in &data {
            hasher.write_u8(*byte);
        }
        assert_eq!(hasher.finish(), whole);
    }

    #[test]
    fn finish_does_not_disturb_the_hasher() {
        let mut hasher = StateHasher::new();
        hasher.write_str("first");
        let early = hasher.finish();
        assert_eq!(hasher.finish(), early);
        hasher.write_str("second");
        let mut straight = StateHasher::new();
        straight.write_str("first");
        straight.write_str("second");
        assert_eq!(hasher.finish(), straight.finish());
        assert_ne!(hasher.finish(), early);
    }

    #[test]
    fn typed_writes_are_little_endian_bytes() {
        let mut typed = StateHasher::new();
        typed.write_u8(0x01);
        typed.write_u16(0x0302);
        typed.write_u32(0x0706_0504);
        typed.write_u64(0x0f0e_0d0c_0b0a_0908);
        typed.write_bool(true);
        typed.write_i64(-1);
        typed.write_u128(1);
        let mut bytes: Vec<u8> = (1..=15).collect();
        bytes.push(1);
        bytes.extend([0xff; 8]);
        bytes.push(1);
        bytes.extend([0; 15]);
        assert_eq!(typed.finish(), hash_bytes(&bytes));
    }

    #[test]
    fn variable_length_values_cannot_be_confused() {
        let a = vec!["ab".to_owned(), "c".to_owned()];
        let b = vec!["a".to_owned(), "bc".to_owned()];
        assert_ne!(Hash128::of(&a), Hash128::of(&b));
        let nested_a: Vec<Vec<u8>> = vec![vec![1, 2], vec![]];
        let nested_b: Vec<Vec<u8>> = vec![vec![1], vec![2]];
        assert_ne!(Hash128::of(&nested_a), Hash128::of(&nested_b));
        assert_ne!(Hash128::of(&Some(0_u8)), Hash128::of(&None::<u8>));
        assert_ne!(Hash128::of(&[0_u8; 0]), Hash128::of(&[0_u8; 1]));
    }

    #[test]
    fn containers_hash_as_their_contents_in_order() {
        let list = vec![3_u32, 1, 2];
        let mut by_hand = StateHasher::new();
        by_hand.write_len(3);
        by_hand.write_u32(3);
        by_hand.write_u32(1);
        by_hand.write_u32(2);
        assert_eq!(Hash128::of(&list), by_hand.finish());
        assert_eq!(Hash128::of(list.as_slice()), by_hand.finish());
        assert_eq!(Hash128::of(&VecDeque::from(list.clone())), by_hand.finish());
        assert_ne!(Hash128::of(&list), Hash128::of(&vec![1_u32, 2, 3]));

        let map: BTreeMap<u8, bool> = [(2, true), (1, false)].into_iter().collect();
        let mut by_hand = StateHasher::new();
        by_hand.write_len(2);
        by_hand.write_u8(1);
        by_hand.write_bool(false);
        by_hand.write_u8(2);
        by_hand.write_bool(true);
        assert_eq!(Hash128::of(&map), by_hand.finish());

        let set: BTreeSet<u16> = [9, 4].into_iter().collect();
        let mut by_hand = StateHasher::new();
        by_hand.write_len(2);
        by_hand.write_u16(4);
        by_hand.write_u16(9);
        assert_eq!(Hash128::of(&set), by_hand.finish());

        assert_eq!(Hash128::of(&(1_u8, 2_u8)), Hash128::of(&[1_u8, 2]));
        assert_eq!(Hash128::of(&Box::new(7_u64)), Hash128::of(&7_u64));
        assert_eq!(Hash128::of(&&7_u64), Hash128::of(&7_u64));
        assert_eq!(Hash128::of(&()), StateHasher::new().finish());
    }

    #[test]
    fn pointer_width_does_not_show() {
        assert_eq!(Hash128::of(&5_usize), Hash128::of(&5_u64));
        assert_eq!(Hash128::of(&-5_isize), Hash128::of(&-5_i64));
        assert_eq!(Hash128::of(&'a'), Hash128::of(&97_u32));
    }

    #[test]
    fn reals_hash_by_exact_bits() {
        assert_eq!(Hash128::of(&Real::of(1.5)), Hash128::of(&1.5_f64.to_bits()));
        assert_ne!(Hash128::of(&Real::of(-0.0)), Hash128::of(&Real::ZERO));
    }

    #[test]
    fn the_macro_hashes_the_named_fields_in_order() {
        struct Sample {
            a: u8,
            b: String,
            cache: u32,
        }
        impl_state_hash!(Sample { a, b });

        let sample = Sample {
            a: 7,
            b: "x".to_owned(),
            cache: 99,
        };
        let mut by_hand = StateHasher::new();
        by_hand.write_u8(7);
        by_hand.write_str("x");
        assert_eq!(Hash128::of(&sample), by_hand.finish());
        assert_eq!(sample.cache, 99);

        struct Empty {}
        impl_state_hash!(Empty {});
        assert_eq!(Hash128::of(&Empty {}), StateHasher::new().finish());
    }

    #[test]
    fn text_form_round_trips_and_rejects_mistakes() {
        let hash = Hash128(0x0123_4567_89ab_cdef_0011_2233_4455_6677);
        assert_eq!(hash.to_string(), "0123456789abcdef0011223344556677");
        assert_eq!(
            format!("{hash:?}"),
            "Hash128(0123456789abcdef0011223344556677)"
        );
        assert_eq!("0123456789ABCDEF0011223344556677".parse(), Ok(hash));
        assert_eq!(
            "0123".parse::<Hash128>(),
            Err(ParseHashError::WrongLength(4))
        );
        assert_eq!(
            "0123456789abcdef001122334455667g".parse::<Hash128>(),
            Err(ParseHashError::NotHex('g'))
        );
        let json = serde_json::to_string(&hash).unwrap();
        assert_eq!(json, "\"0123456789abcdef0011223344556677\"");
        assert_eq!(serde_json::from_str::<Hash128>(&json).unwrap(), hash);
        assert!(serde_json::from_str::<Hash128>("\"xyz\"").is_err());
    }
}
