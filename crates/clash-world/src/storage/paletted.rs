//! [`Paletted`]: one value for every cell of a chunk, packed through a
//! palette (02 §6.1).

use clash_core::{StateHash, StateHasher};
use serde::{Deserialize, Serialize};

use crate::dims::CHUNK_CELLS;
use crate::pos::CellIdx;

/// Bits in one word of packed data.
const WORD_BITS: usize = u64::BITS as usize;

/// The widths a packed index may have, in bits. Each divides a word, so an
/// index never straddles two words.
const WIDTHS: [u8; 5] = [1, 2, 4, 8, 16];

/// The widest index.
const MAX_WIDTH: u8 = WIDTHS[WIDTHS.len() - 1];

/// From this width up, a full palette is cleared of unused entries before
/// it is widened: a wide palette that has filled up is usually mostly
/// stale.
const COMPACT_FROM_WIDTH: u8 = 8;

/// Words of packed data at a width.
const fn words_for(bits: u8) -> usize {
    CHUNK_CELLS * bits as usize / WORD_BITS
}

/// The narrowest width that can index a palette of `len` entries.
fn width_for(len: usize) -> u8 {
    WIDTHS
        .into_iter()
        .find(|bits| len <= 1_usize << bits)
        .unwrap_or(MAX_WIDTH)
}

#[inline]
fn read(data: &[u64], bits: u8, index: usize) -> usize {
    let at = index * bits as usize;
    let mask = (1_u64 << bits) - 1;
    ((data[at / WORD_BITS] >> (at % WORD_BITS)) & mask) as usize
}

#[inline]
fn write(data: &mut [u64], bits: u8, index: usize, value: usize) {
    let at = index * bits as usize;
    let shift = at % WORD_BITS;
    let mask = ((1_u64 << bits) - 1) << shift;
    let word = &mut data[at / WORD_BITS];
    *word = (*word & !mask) | (((value as u64) << shift) & mask);
}

#[derive(Clone, Debug)]
enum Repr<T> {
    /// Every cell holds this value.
    Uniform(T),
    /// Each cell holds an index into `palette`, `bits` wide, packed into
    /// `data`. Every index is less than `palette.len()`, no two palette
    /// entries are equal, and `bits` is one of [`WIDTHS`].
    Packed {
        palette: Vec<T>,
        bits: u8,
        data: Box<[u64]>,
    },
}

/// One value for every cell of a chunk.
///
/// A layer that holds one value throughout costs a few bytes. Otherwise
/// each cell holds an index 1, 2, 4, 8 or 16 bits wide into a palette of
/// the distinct values. The palette grows as new values are written and
/// is cleared of unused entries by [`compact`](Paletted::compact), which
/// saving does.
///
/// Two layers are equal, hash equal and serialise alike when their cells
/// hold the same values, whatever order those values were written in.
///
/// ```
/// use clash_world::CellIdx;
/// use clash_world::storage::Paletted;
///
/// let mut layer = Paletted::uniform(0_u8);
/// assert_eq!(layer.bits(), 0);
/// let at = CellIdx::new(4, 4, 0);
/// assert_eq!(layer.set(at, 7), 0);
/// assert_eq!(layer.get(at), 7);
/// assert_eq!(layer.bits(), 1);
/// layer.set(at, 0);
/// layer.compact();
/// assert_eq!(layer.as_uniform(), Some(0));
/// ```
#[derive(Clone, Debug)]
pub struct Paletted<T> {
    repr: Repr<T>,
}

impl<T: Copy + Eq> Paletted<T> {
    /// A layer holding one value in every cell.
    pub const fn uniform(value: T) -> Paletted<T> {
        Paletted {
            repr: Repr::Uniform(value),
        }
    }

    /// The value of a cell.
    #[inline]
    pub fn get(&self, idx: CellIdx) -> T {
        self.at(idx.index())
    }

    #[inline]
    fn at(&self, index: usize) -> T {
        match &self.repr {
            Repr::Uniform(value) => *value,
            Repr::Packed {
                palette,
                bits,
                data,
            } => palette[read(data, *bits, index)],
        }
    }

    /// Sets the value of a cell and returns the value it had.
    pub fn set(&mut self, idx: CellIdx, value: T) -> T {
        let index = idx.index();
        let old = self.at(index);
        if old == value {
            return old;
        }
        if self.is_full_without(value) {
            self.make_room();
        }
        match &mut self.repr {
            Repr::Uniform(current) => {
                let mut data = vec![0_u64; words_for(WIDTHS[0])].into_boxed_slice();
                write(&mut data, WIDTHS[0], index, 1);
                self.repr = Repr::Packed {
                    palette: vec![*current, value],
                    bits: WIDTHS[0],
                    data,
                };
            }
            Repr::Packed {
                palette,
                bits,
                data,
            } => {
                let slot = match palette.iter().position(|entry| *entry == value) {
                    Some(slot) => slot,
                    None => {
                        palette.push(value);
                        palette.len() - 1
                    }
                };
                write(data, *bits, index, slot);
            }
        }
        old
    }

    /// Whether the palette has no free entry and does not hold `value`.
    fn is_full_without(&self, value: T) -> bool {
        match &self.repr {
            Repr::Uniform(_) => false,
            Repr::Packed { palette, bits, .. } => {
                palette.len() >= 1_usize << bits && !palette.contains(&value)
            }
        }
    }

    /// Makes room for one more palette entry: by dropping unused entries
    /// when the palette is wide, and otherwise, or if that frees too
    /// little, by widening the indices.
    fn make_room(&mut self) {
        if let Repr::Packed { bits, .. } = &self.repr
            && *bits >= COMPACT_FROM_WIDTH
        {
            self.compact();
        }
        if let Repr::Packed {
            palette,
            bits,
            data,
        } = &mut self.repr
        {
            let capacity = 1_usize << *bits;
            // Widen unless at least a quarter of the palette is free, so
            // that making room is not needed again at once.
            if palette.len() * 4 > capacity * 3 && *bits < MAX_WIDTH {
                let wider = width_for(capacity + 1);
                let mut widened = vec![0_u64; words_for(wider)].into_boxed_slice();
                for index in 0..CHUNK_CELLS {
                    write(&mut widened, wider, index, read(data, *bits, index));
                }
                *bits = wider;
                *data = widened;
            }
        }
    }

    /// Sets every cell to one value.
    pub fn fill(&mut self, value: T) {
        self.repr = Repr::Uniform(value);
    }

    /// Drops palette entries no cell uses, narrows the indices to fit, and
    /// turns a layer whose cells all hold one value into a uniform one.
    /// What the cells hold does not change.
    ///
    /// The result is canonical: palette entries are in the order in which
    /// their values first appear, cell by cell. Two layers that hold the
    /// same values are identical after compaction.
    pub fn compact(&mut self) {
        let Repr::Packed {
            palette,
            bits,
            data,
        } = &self.repr
        else {
            return;
        };
        const UNSEEN: u32 = u32::MAX;
        let mut remap = vec![UNSEEN; palette.len()];
        let mut kept: Vec<T> = Vec::new();
        for index in 0..CHUNK_CELLS {
            let slot = read(data, *bits, index);
            if remap[slot] == UNSEEN {
                remap[slot] = kept.len() as u32;
                kept.push(palette[slot]);
            }
        }
        if let [only] = kept.as_slice() {
            self.repr = Repr::Uniform(*only);
            return;
        }
        let narrowed = width_for(kept.len());
        let mut packed = vec![0_u64; words_for(narrowed)].into_boxed_slice();
        for index in 0..CHUNK_CELLS {
            let slot = remap[read(data, *bits, index)] as usize;
            write(&mut packed, narrowed, index, slot);
        }
        self.repr = Repr::Packed {
            palette: kept,
            bits: narrowed,
            data: packed,
        };
    }

    /// Replaces every value the layer may hold by what `turn` makes of
    /// it, cell for cell. Values that become equal are merged, and the
    /// layer is left compacted.
    pub fn map_values(&mut self, mut turn: impl FnMut(T) -> T) {
        match &mut self.repr {
            Repr::Uniform(value) => *value = turn(*value),
            Repr::Packed { palette, .. } => {
                let turned: Vec<T> = palette.iter().map(|value| turn(*value)).collect();
                let distinct = turned
                    .iter()
                    .enumerate()
                    .all(|(at, value)| !turned[..at].contains(value));
                if distinct {
                    *palette = turned;
                } else {
                    // Rare: two values have become one. Write the cells
                    // out again so that no two palette entries are equal.
                    let cells: Vec<T> = self.iter().collect();
                    let mut rebuilt = Paletted::uniform(turn(cells[0]));
                    for (index, value) in cells.into_iter().enumerate() {
                        if let Some(idx) = CellIdx::from_index(index) {
                            rebuilt.set(idx, turn(value));
                        }
                    }
                    *self = rebuilt;
                }
                self.compact();
            }
        }
    }

    /// The one value every cell holds, if the layer is stored as uniform.
    /// A layer whose cells happen to be equal is stored as uniform only
    /// after [`compact`](Paletted::compact).
    pub fn as_uniform(&self) -> Option<T> {
        match &self.repr {
            Repr::Uniform(value) => Some(*value),
            Repr::Packed { .. } => None,
        }
    }

    /// The width of a cell's index in bits: 0 for a uniform layer,
    /// otherwise 1, 2, 4, 8 or 16.
    pub fn bits(&self) -> u8 {
        match &self.repr {
            Repr::Uniform(_) => 0,
            Repr::Packed { bits, .. } => *bits,
        }
    }

    /// The distinct values the layer may hold. Some may be unused until
    /// the layer is compacted.
    pub fn palette(&self) -> &[T] {
        match &self.repr {
            Repr::Uniform(value) => core::slice::from_ref(value),
            Repr::Packed { palette, .. } => palette,
        }
    }

    /// The value of every cell, in index order.
    pub fn iter(&self) -> impl Iterator<Item = T> + '_ {
        (0..CHUNK_CELLS).map(|index| self.at(index))
    }

    /// The bytes this layer holds on the heap.
    pub fn heap_bytes(&self) -> usize {
        match &self.repr {
            Repr::Uniform(_) => 0,
            Repr::Packed { palette, data, .. } => {
                palette.capacity() * core::mem::size_of::<T>()
                    + data.len() * core::mem::size_of::<u64>()
            }
        }
    }

    /// The bytes this layer takes in all.
    pub fn memory_bytes(&self) -> usize {
        core::mem::size_of::<Paletted<T>>() + self.heap_bytes()
    }

    /// Calls `visit` with each run of consecutive cells holding one value:
    /// the value and how many cells.
    fn for_each_run(&self, mut visit: impl FnMut(T, u32)) {
        match &self.repr {
            Repr::Uniform(value) => visit(*value, CHUNK_CELLS as u32),
            Repr::Packed {
                palette,
                bits,
                data,
            } => {
                let mut slot = read(data, *bits, 0);
                let mut length = 0_u32;
                for index in 0..CHUNK_CELLS {
                    let here = read(data, *bits, index);
                    if here != slot {
                        visit(palette[slot], length);
                        slot = here;
                        length = 0;
                    }
                    length += 1;
                }
                visit(palette[slot], length);
            }
        }
    }
}

impl<T: Copy + Eq + Default> Default for Paletted<T> {
    fn default() -> Paletted<T> {
        Paletted::uniform(T::default())
    }
}

/// Layers are equal when their cells hold the same values.
impl<T: Copy + Eq> PartialEq for Paletted<T> {
    fn eq(&self, other: &Paletted<T>) -> bool {
        if let (Repr::Uniform(a), Repr::Uniform(b)) = (&self.repr, &other.repr) {
            return a == b;
        }
        self.iter().eq(other.iter())
    }
}

impl<T: Copy + Eq> Eq for Paletted<T> {}

/// Hashes the runs of equal values in cell order, so the hash depends on
/// what the cells hold and not on the palette.
impl<T: Copy + Eq + StateHash> StateHash for Paletted<T> {
    fn state_hash(&self, hasher: &mut StateHasher) {
        self.for_each_run(|value, length| {
            value.state_hash(hasher);
            hasher.write_u32(length);
        });
    }
}

/// The stored form of a layer.
#[derive(Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Stored<T> {
    Uniform(T),
    Packed {
        palette: Vec<T>,
        bits: u8,
        data: Vec<u64>,
    },
}

/// Serialises the compacted, canonical form.
impl<T: Copy + Eq + Serialize> Serialize for Paletted<T> {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut canonical = self.clone();
        canonical.compact();
        let stored = match canonical.repr {
            Repr::Uniform(value) => Stored::Uniform(value),
            Repr::Packed {
                palette,
                bits,
                data,
            } => Stored::Packed {
                palette,
                bits,
                data: data.into_vec(),
            },
        };
        stored.serialize(serializer)
    }
}

impl<'de, T: Copy + Ord + Deserialize<'de>> Deserialize<'de> for Paletted<T> {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Paletted<T>, D::Error> {
        use serde::de::Error;
        match Stored::<T>::deserialize(deserializer)? {
            Stored::Uniform(value) => Ok(Paletted::uniform(value)),
            Stored::Packed {
                palette,
                bits,
                data,
            } => {
                if !WIDTHS.contains(&bits) {
                    return Err(D::Error::custom(format!(
                        "a palette index is 1, 2, 4, 8 or 16 bits wide, not {bits}"
                    )));
                }
                if data.len() != words_for(bits) {
                    return Err(D::Error::custom(format!(
                        "a layer {bits} bits wide has {} words of data, but {} were given",
                        words_for(bits),
                        data.len()
                    )));
                }
                if palette.is_empty() || palette.len() > 1_usize << bits {
                    return Err(D::Error::custom(format!(
                        "a layer {bits} bits wide has 1 to {} palette entries, but {} were given",
                        1_usize << bits,
                        palette.len()
                    )));
                }
                let mut sorted = palette.clone();
                sorted.sort_unstable();
                if sorted.windows(2).any(|pair| pair[0] == pair[1]) {
                    return Err(D::Error::custom("a palette holds each value once"));
                }
                if (0..CHUNK_CELLS).any(|index| read(&data, bits, index) >= palette.len()) {
                    return Err(D::Error::custom(
                        "a cell refers to a palette entry that does not exist",
                    ));
                }
                Ok(Paletted {
                    repr: Repr::Packed {
                        palette,
                        bits,
                        data: data.into_boxed_slice(),
                    },
                })
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use clash_core::Hash128;

    fn idx(index: usize) -> CellIdx {
        CellIdx::from_index(index).unwrap()
    }

    #[test]
    fn a_uniform_layer_is_under_200_bytes() {
        let layer = Paletted::uniform([0_u16; 3]);
        assert!(layer.memory_bytes() < 200, "{} bytes", layer.memory_bytes());
        assert_eq!(layer.heap_bytes(), 0);
    }

    #[test]
    fn a_sixteen_entry_palette_uses_four_bits_per_cell() {
        let mut layer = Paletted::uniform(0_u16);
        for value in 0..16_u16 {
            layer.set(idx(usize::from(value) * 3), value);
        }
        assert_eq!(layer.palette().len(), 16);
        assert_eq!(layer.bits(), 4);
        let packed_bytes = CHUNK_CELLS * 4 / 8;
        assert!(layer.heap_bytes() >= packed_bytes);
        assert!(layer.heap_bytes() < packed_bytes + 16 * 2 * 4);
        // One more value and four bits no longer suffice.
        layer.set(idx(100), 99);
        assert_eq!(layer.bits(), 8);
    }

    #[test]
    fn widths_grow_through_every_step() {
        let mut layer = Paletted::uniform(0_u32);
        let mut seen = vec![layer.bits()];
        for value in 1..=300_u32 {
            layer.set(idx(value as usize), value);
            if seen.last() != Some(&layer.bits()) {
                seen.push(layer.bits());
            }
        }
        assert_eq!(seen, [0, 1, 2, 4, 8, 16]);
        for value in 1..=300_u32 {
            assert_eq!(layer.get(idx(value as usize)), value);
        }
        assert_eq!(layer.get(idx(0)), 0);
    }

    #[test]
    fn different_histories_hash_and_compare_equal() {
        // The same contents reached three ways: written forwards, written
        // backwards through other values, and loaded from the stored form.
        let mut forward = Paletted::uniform(0_u16);
        for index in 0..40 {
            forward.set(idx(index), (index % 5) as u16);
        }
        let mut backward = Paletted::uniform(9_u16);
        for index in (0..CHUNK_CELLS).rev() {
            backward.set(idx(index), 77);
            let value = if index < 40 { (index % 5) as u16 } else { 0 };
            backward.set(idx(index), value);
        }
        assert_ne!(forward.palette(), backward.palette());
        assert_eq!(forward, backward);
        assert_eq!(Hash128::of(&forward), Hash128::of(&backward));

        let json = serde_json::to_string(&backward).unwrap();
        assert_eq!(json, serde_json::to_string(&forward).unwrap());
        let loaded: Paletted<u16> = serde_json::from_str(&json).unwrap();
        assert_eq!(loaded, forward);
        assert_eq!(Hash128::of(&loaded), Hash128::of(&forward));

        // A different content hashes differently.
        forward.set(idx(8000), 1);
        assert_ne!(Hash128::of(&forward), Hash128::of(&backward));
    }

    #[test]
    fn uniformity_is_detected_whatever_the_form() {
        let mut layer = Paletted::uniform(3_u8);
        layer.set(idx(17), 4);
        layer.set(idx(17), 3);
        assert_eq!(layer.as_uniform(), None);
        assert_eq!(layer, Paletted::uniform(3));
        assert_eq!(Hash128::of(&layer), Hash128::of(&Paletted::uniform(3_u8)));
        assert_eq!(serde_json::to_string(&layer).unwrap(), r#"{"uniform":3}"#);
        layer.compact();
        assert_eq!(layer.as_uniform(), Some(3));
    }

    #[test]
    fn a_churned_wide_palette_is_cleared_instead_of_widened() {
        let mut layer = Paletted::uniform(0_u32);
        // One cell cycles through far more values than there are cells.
        for value in 1..100_000_u32 {
            layer.set(idx(5), value);
        }
        assert_eq!(layer.get(idx(5)), 99_999);
        assert!(layer.palette().len() <= 1 << COMPACT_FROM_WIDTH);
        assert!(layer.bits() <= COMPACT_FROM_WIDTH);
    }

    #[test]
    fn bad_stored_forms_are_refused() {
        let good = r#"{"packed":{"palette":[1,2],"bits":1,"data":[]}}"#;
        assert!(
            serde_json::from_str::<Paletted<u8>>(good).is_err(),
            "too little data"
        );
        let words = vec!["0"; words_for(1)].join(",");
        let ok = format!(r#"{{"packed":{{"palette":[1,2],"bits":1,"data":[{words}]}}}}"#);
        assert!(serde_json::from_str::<Paletted<u8>>(&ok).is_ok());
        let odd_width = format!(r#"{{"packed":{{"palette":[1,2],"bits":3,"data":[{words}]}}}}"#);
        assert!(serde_json::from_str::<Paletted<u8>>(&odd_width).is_err());
        let twice = format!(r#"{{"packed":{{"palette":[1,1],"bits":1,"data":[{words}]}}}}"#);
        assert!(serde_json::from_str::<Paletted<u8>>(&twice).is_err());
        let ones = vec!["1"; words_for(1)].join(",");
        let dangling = format!(r#"{{"packed":{{"palette":[1],"bits":1,"data":[{ones}]}}}}"#);
        assert!(serde_json::from_str::<Paletted<u8>>(&dangling).is_err());
        let empty = format!(r#"{{"packed":{{"palette":[],"bits":1,"data":[{words}]}}}}"#);
        assert!(serde_json::from_str::<Paletted<u8>>(&empty).is_err());
    }
}
