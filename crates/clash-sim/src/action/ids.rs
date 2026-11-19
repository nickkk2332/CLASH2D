//! The ids of the action system and [`ChannelSet`].
//!
//! Nothing here depends on anything else in this crate: only on
//! `clash-core` and `serde`. The kernel entity (03 §4.2) and the action
//! runtime name these types, so they are kept where either can reach them
//! without the other.

use std::fmt;
use std::ops::{BitAnd, BitOr, Sub};

use clash_core::{StateHash, StateHasher, define_id};
use serde::{Deserialize, Serialize};

define_id! {
    /// An `action` definition (03 §6.1), by its dense number.
    pub struct ActionDefId;
    /// A `channel` definition (03 §6.2), by its dense number. Eight bits:
    /// a [`ChannelSet`] has one bit for each value.
    pub struct ChannelId(u8);
    /// A `window_kind` definition (03 §7.1), by its dense number.
    pub struct WindowKindId(u16);
    /// The position of a phase among the phases of its action (03 §6.4).
    /// The first phase is 0.
    pub struct PhaseIndex(u16);
}

/// How many 64-bit words a [`ChannelSet`] is made of.
const WORDS: usize = 4;

/// How many bits one word holds.
const WORD_BITS: usize = u64::BITS as usize;

/// A set of channels (03 §6.2): one bit for each `channel` definition.
///
/// It says *which* channels, not how many slots of each: an action's
/// needs are its `ChannelUse`, and an entity's slots are the action
/// runtime's. Two actions whose sets do not
/// [intersect](ChannelSet::intersects) can never want the same slot, which
/// is the test made before slots are counted.
///
/// The set holds every [`ChannelId`] there can be, so inserting never
/// fails. It is `Copy`, four words, and saved and hashed as those words.
///
/// ```
/// use clash_sim::action::{ChannelId, ChannelSet};
///
/// let arm = ChannelId::new(0);
/// let voice = ChannelId::new(3);
/// let mut set = ChannelSet::new();
/// assert!(set.insert(arm));
/// assert!(!set.insert(arm));
/// assert!(set.contains(arm) && !set.contains(voice));
/// assert!(!set.intersects(ChannelSet::of(voice)));
/// assert_eq!((set | ChannelSet::of(voice)).len(), 2);
/// assert_eq!(set.iter().collect::<Vec<_>>(), [arm]);
/// ```
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default, Serialize, Deserialize)]
pub struct ChannelSet([u64; WORDS]);

impl ChannelSet {
    /// How many different channels a set can hold: every [`ChannelId`].
    pub const CAPACITY: usize = WORDS * WORD_BITS;

    /// The set with nothing in it.
    pub const EMPTY: ChannelSet = ChannelSet([0; WORDS]);

    /// The set with nothing in it.
    pub const fn new() -> ChannelSet {
        ChannelSet::EMPTY
    }

    /// The set of one channel.
    pub const fn of(channel: ChannelId) -> ChannelSet {
        let mut set = ChannelSet::EMPTY;
        let (word, bit) = ChannelSet::place(channel);
        set.0[word] = bit;
        set
    }

    /// The word a channel's bit is in, and the bit.
    const fn place(channel: ChannelId) -> (usize, u64) {
        let index = channel.index();
        (index / WORD_BITS, 1 << (index % WORD_BITS))
    }

    /// Adds a channel. Returns whether it was new.
    pub const fn insert(&mut self, channel: ChannelId) -> bool {
        let (word, bit) = ChannelSet::place(channel);
        let new = self.0[word] & bit == 0;
        self.0[word] |= bit;
        new
    }

    /// Removes a channel. Returns whether it was there.
    pub const fn remove(&mut self, channel: ChannelId) -> bool {
        let (word, bit) = ChannelSet::place(channel);
        let there = self.0[word] & bit != 0;
        self.0[word] &= !bit;
        there
    }

    /// Whether the set holds a channel.
    pub const fn contains(self, channel: ChannelId) -> bool {
        let (word, bit) = ChannelSet::place(channel);
        self.0[word] & bit != 0
    }

    /// Whether the set holds nothing.
    pub const fn is_empty(self) -> bool {
        let mut word = 0;
        while word < WORDS {
            if self.0[word] != 0 {
                return false;
            }
            word += 1;
        }
        true
    }

    /// How many channels the set holds.
    pub const fn len(self) -> usize {
        let mut count = 0;
        let mut word = 0;
        while word < WORDS {
            count += self.0[word].count_ones() as usize;
            word += 1;
        }
        count
    }

    /// Every channel in either set.
    #[must_use]
    pub const fn union(mut self, other: ChannelSet) -> ChannelSet {
        let mut word = 0;
        while word < WORDS {
            self.0[word] |= other.0[word];
            word += 1;
        }
        self
    }

    /// Every channel in both sets.
    #[must_use]
    pub const fn intersection(mut self, other: ChannelSet) -> ChannelSet {
        let mut word = 0;
        while word < WORDS {
            self.0[word] &= other.0[word];
            word += 1;
        }
        self
    }

    /// Every channel in this set that is not in `other`.
    #[must_use]
    pub const fn difference(mut self, other: ChannelSet) -> ChannelSet {
        let mut word = 0;
        while word < WORDS {
            self.0[word] &= !other.0[word];
            word += 1;
        }
        self
    }

    /// Whether the two sets share a channel.
    pub const fn intersects(self, other: ChannelSet) -> bool {
        !self.intersection(other).is_empty()
    }

    /// Whether every channel of this set is in `other`.
    pub const fn is_subset(self, other: ChannelSet) -> bool {
        self.difference(other).is_empty()
    }

    /// The channels, in order of id.
    pub fn iter(self) -> impl Iterator<Item = ChannelId> {
        self.0.into_iter().enumerate().flat_map(|(word, bits)| {
            Bits(bits).filter_map(move |bit| ChannelId::from_index(word * WORD_BITS + bit))
        })
    }
}

/// The positions of the set bits of a word, lowest first.
struct Bits(u64);

impl Iterator for Bits {
    type Item = usize;

    fn next(&mut self) -> Option<usize> {
        if self.0 == 0 {
            return None;
        }
        let bit = self.0.trailing_zeros() as usize;
        self.0 &= self.0 - 1;
        Some(bit)
    }
}

impl FromIterator<ChannelId> for ChannelSet {
    fn from_iter<I: IntoIterator<Item = ChannelId>>(channels: I) -> ChannelSet {
        let mut set = ChannelSet::EMPTY;
        set.extend(channels);
        set
    }
}

impl Extend<ChannelId> for ChannelSet {
    fn extend<I: IntoIterator<Item = ChannelId>>(&mut self, channels: I) {
        for channel in channels {
            self.insert(channel);
        }
    }
}

impl BitOr for ChannelSet {
    type Output = ChannelSet;

    fn bitor(self, other: ChannelSet) -> ChannelSet {
        self.union(other)
    }
}

impl BitAnd for ChannelSet {
    type Output = ChannelSet;

    fn bitand(self, other: ChannelSet) -> ChannelSet {
        self.intersection(other)
    }
}

impl Sub for ChannelSet {
    type Output = ChannelSet;

    fn sub(self, other: ChannelSet) -> ChannelSet {
        self.difference(other)
    }
}

/// `{0, 3}`: the numbers of the channels.
impl fmt::Debug for ChannelSet {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_set()
            .entries(self.iter().map(ChannelId::raw))
            .finish()
    }
}

impl StateHash for ChannelSet {
    fn state_hash(&self, hasher: &mut StateHasher) {
        self.0.state_hash(hasher);
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use clash_core::{DenseId, Hash128};
    use proptest::prelude::*;

    use super::*;

    fn channel(raw: u8) -> ChannelId {
        ChannelId::new(raw)
    }

    #[test]
    fn a_set_holds_every_channel_id() {
        assert_eq!(ChannelSet::CAPACITY, usize::from(u8::MAX) + 1);
        assert!(ChannelId::from_index(ChannelSet::CAPACITY).is_none());
        let mut set = ChannelSet::new();
        for raw in 0..=u8::MAX {
            assert!(set.insert(channel(raw)));
        }
        assert_eq!(set.len(), ChannelSet::CAPACITY);
        assert_eq!(set.iter().count(), ChannelSet::CAPACITY);
        assert!(set.contains(channel(u8::MAX)));
        assert!(set.remove(channel(u8::MAX)));
        assert!(!set.remove(channel(u8::MAX)));
        assert_eq!(set.len(), ChannelSet::CAPACITY - 1);
    }

    #[test]
    fn it_is_small_and_copies() {
        assert_eq!(size_of::<ChannelSet>(), WORDS * size_of::<u64>());
        let set = ChannelSet::of(channel(70));
        let copy = set;
        assert_eq!(set, copy);
        assert_eq!(format!("{set:?}"), "{70}");
        assert_eq!(format!("{:?}", ChannelSet::EMPTY), "{}");
        assert!(ChannelSet::default().is_empty());
    }

    #[test]
    fn it_survives_json_and_the_binary_codec() {
        let set: ChannelSet = [0, 63, 64, 200, 255].into_iter().map(channel).collect();
        let json = serde_json::to_string(&set).unwrap();
        assert_eq!(serde_json::from_str::<ChannelSet>(&json).unwrap(), set);
        let bytes = clash_core::codec::to_bytes(&set).unwrap();
        assert_eq!(
            clash_core::codec::from_bytes::<ChannelSet>(&bytes).unwrap(),
            set
        );
    }

    #[test]
    fn the_hash_tells_sets_apart() {
        let a = ChannelSet::of(channel(1));
        let b = ChannelSet::of(channel(65));
        assert_ne!(Hash128::of(&a), Hash128::of(&b));
        assert_eq!(Hash128::of(&a), Hash128::of(&ChannelSet::of(channel(1))));
        assert_ne!(Hash128::of(&a), Hash128::of(&ChannelSet::EMPTY));
    }

    #[test]
    fn the_other_ids_are_dense() {
        assert_eq!(ActionDefId::new(7).index(), 7);
        assert_eq!(WindowKindId::from_index(3), Some(WindowKindId::new(3)));
        assert_eq!(PhaseIndex::new(2).to_string(), "2");
        assert_eq!(
            <PhaseIndex as DenseId>::from_index(0),
            Some(PhaseIndex::new(0))
        );
    }

    fn model(raw: &[u8]) -> (ChannelSet, BTreeSet<u8>) {
        (
            raw.iter().copied().map(channel).collect(),
            raw.iter().copied().collect(),
        )
    }

    fn raws(set: ChannelSet) -> Vec<u8> {
        set.iter().map(ChannelId::raw).collect()
    }

    proptest! {
        /// The set agrees with a `BTreeSet` of the same channels.
        #[test]
        fn it_agrees_with_an_ordered_set(
            a in proptest::collection::vec(any::<u8>(), 0..40),
            b in proptest::collection::vec(any::<u8>(), 0..40),
        ) {
            let (set_a, model_a) = model(&a);
            let (set_b, model_b) = model(&b);
            prop_assert_eq!(set_a.len(), model_a.len());
            prop_assert_eq!(set_a.is_empty(), model_a.is_empty());
            prop_assert_eq!(raws(set_a), model_a.iter().copied().collect::<Vec<_>>());
            prop_assert_eq!(
                raws(set_a | set_b),
                model_a.union(&model_b).copied().collect::<Vec<_>>()
            );
            prop_assert_eq!(
                raws(set_a & set_b),
                model_a.intersection(&model_b).copied().collect::<Vec<_>>()
            );
            prop_assert_eq!(
                raws(set_a - set_b),
                model_a.difference(&model_b).copied().collect::<Vec<_>>()
            );
            prop_assert_eq!(set_a.intersects(set_b), !model_a.is_disjoint(&model_b));
            prop_assert_eq!(set_a.is_subset(set_b), model_a.is_subset(&model_b));
            for raw in 0..=u8::MAX {
                prop_assert_eq!(set_a.contains(channel(raw)), model_a.contains(&raw));
            }
            // Equal sets are equal, hash alike and encode alike however
            // they were built.
            let mut reversed = ChannelSet::new();
            for raw in a.iter().rev() {
                reversed.insert(channel(*raw));
            }
            prop_assert_eq!(reversed, set_a);
            prop_assert_eq!(Hash128::of(&reversed), Hash128::of(&set_a));
        }
    }
}
