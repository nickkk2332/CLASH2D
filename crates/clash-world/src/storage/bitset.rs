//! [`CellBits`]: one bit for every cell of a chunk.

use clash_core::{StateHash, StateHasher};
use serde::{Deserialize, Serialize};

use crate::dims::CHUNK_CELLS;
use crate::pos::CellIdx;

/// Bits in one word of the set.
const WORD_BITS: usize = u64::BITS as usize;

/// Words in the set.
const WORDS: usize = CHUNK_CELLS / WORD_BITS;

/// A fixed set of bits, one for every cell of a chunk.
///
/// ```
/// use clash_world::CellIdx;
/// use clash_world::storage::CellBits;
///
/// let mut walls = CellBits::new();
/// let at = CellIdx::new(3, 4, 1);
/// assert!(!walls.set(at, true));
/// assert!(walls.get(at));
/// assert_eq!(walls.count(), 1);
/// assert_eq!(walls.iter().collect::<Vec<_>>(), [at]);
/// ```
#[derive(Clone, PartialEq, Eq)]
pub struct CellBits {
    words: [u64; WORDS],
}

impl CellBits {
    /// A set with every bit clear.
    pub const fn new() -> CellBits {
        CellBits { words: [0; WORDS] }
    }

    /// A set with every bit set to `value`.
    pub const fn filled(value: bool) -> CellBits {
        CellBits {
            words: [if value { u64::MAX } else { 0 }; WORDS],
        }
    }

    /// The bit for a cell.
    #[inline]
    pub const fn get(&self, idx: CellIdx) -> bool {
        let index = idx.index();
        (self.words[index / WORD_BITS] >> (index % WORD_BITS)) & 1 != 0
    }

    /// Sets the bit for a cell and returns what it was.
    #[inline]
    pub const fn set(&mut self, idx: CellIdx, value: bool) -> bool {
        let index = idx.index();
        let word = &mut self.words[index / WORD_BITS];
        let mask = 1_u64 << (index % WORD_BITS);
        let old = *word & mask != 0;
        if value {
            *word |= mask;
        } else {
            *word &= !mask;
        }
        old
    }

    /// Sets every bit to `value`.
    pub const fn fill(&mut self, value: bool) {
        self.words = [if value { u64::MAX } else { 0 }; WORDS];
    }

    /// How many bits are set.
    pub fn count(&self) -> usize {
        self.words
            .iter()
            .map(|word| word.count_ones() as usize)
            .sum()
    }

    /// Whether no bit is set.
    pub fn is_empty(&self) -> bool {
        self.words.iter().all(|word| *word == 0)
    }

    /// The cells whose bit is set, in index order.
    pub fn iter(&self) -> impl Iterator<Item = CellIdx> + '_ {
        self.words.iter().enumerate().flat_map(|(at, word)| {
            let mut rest = *word;
            core::iter::from_fn(move || {
                if rest == 0 {
                    return None;
                }
                let bit = rest.trailing_zeros() as usize;
                rest &= rest - 1;
                CellIdx::from_index(at * WORD_BITS + bit)
            })
        })
    }
}

impl Default for CellBits {
    fn default() -> CellBits {
        CellBits::new()
    }
}

impl core::fmt::Debug for CellBits {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "CellBits({} set)", self.count())
    }
}

impl StateHash for CellBits {
    fn state_hash(&self, hasher: &mut StateHasher) {
        for word in &self.words {
            hasher.write_u64(*word);
        }
    }
}

impl Serialize for CellBits {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        self.words.as_slice().serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for CellBits {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<CellBits, D::Error> {
        let words = Vec::<u64>::deserialize(deserializer)?;
        let words: [u64; WORDS] = words.try_into().map_err(|given: Vec<u64>| {
            serde::de::Error::custom(format!(
                "a cell bitset has {WORDS} words, but {} were given",
                given.len()
            ))
        })?;
        Ok(CellBits { words })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bits_set_clear_and_count() {
        let mut bits = CellBits::new();
        assert!(bits.is_empty());
        let first = CellIdx::from_index(0).unwrap();
        let last = CellIdx::from_index(CHUNK_CELLS - 1).unwrap();
        assert!(!bits.set(first, true));
        assert!(!bits.set(last, true));
        assert!(bits.set(last, true));
        assert_eq!(bits.count(), 2);
        assert_eq!(bits.iter().collect::<Vec<_>>(), [first, last]);
        assert!(bits.set(first, false));
        assert!(!bits.get(first));
        bits.fill(true);
        assert_eq!(bits.count(), CHUNK_CELLS);
        assert_eq!(bits, CellBits::filled(true));
    }

    #[test]
    fn bits_round_trip_and_refuse_a_wrong_length() {
        let mut bits = CellBits::new();
        bits.set(CellIdx::new(1, 2, 3), true);
        let json = serde_json::to_string(&bits).unwrap();
        assert_eq!(serde_json::from_str::<CellBits>(&json).unwrap(), bits);
        assert!(serde_json::from_str::<CellBits>("[1, 2]").is_err());
    }
}
