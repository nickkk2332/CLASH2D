//! [`SparseCells`]: a value for a few cells of a chunk.

use clash_core::{StateHash, StateHasher};
use serde::{Deserialize, Serialize};

use crate::pos::CellIdx;

/// A value for some of the cells of a chunk: a `Vec` of `(cell, value)`
/// kept sorted by cell.
///
/// ```
/// use clash_world::CellIdx;
/// use clash_world::storage::SparseCells;
///
/// let mut marks = SparseCells::new();
/// let at = CellIdx::new(1, 1, 0);
/// assert_eq!(marks.insert(at, "soot"), None);
/// assert_eq!(marks.get(at), Some(&"soot"));
/// assert_eq!(marks.remove(at), Some("soot"));
/// assert!(marks.is_empty());
/// ```
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct SparseCells<T> {
    entries: Vec<(CellIdx, T)>,
}

impl<T> SparseCells<T> {
    /// An empty layer.
    pub const fn new() -> SparseCells<T> {
        SparseCells {
            entries: Vec::new(),
        }
    }

    /// How many cells have a value.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Whether no cell has a value.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    fn find(&self, idx: CellIdx) -> Result<usize, usize> {
        self.entries.binary_search_by_key(&idx, |entry| entry.0)
    }

    /// The value for a cell, if it has one.
    pub fn get(&self, idx: CellIdx) -> Option<&T> {
        self.find(idx).ok().map(|at| &self.entries[at].1)
    }

    /// The value for a cell, to change in place.
    pub fn get_mut(&mut self, idx: CellIdx) -> Option<&mut T> {
        self.find(idx).ok().map(|at| &mut self.entries[at].1)
    }

    /// Gives a cell a value and returns the one it had.
    pub fn insert(&mut self, idx: CellIdx, value: T) -> Option<T> {
        match self.find(idx) {
            Ok(at) => Some(core::mem::replace(&mut self.entries[at].1, value)),
            Err(at) => {
                self.entries.insert(at, (idx, value));
                None
            }
        }
    }

    /// The value for a cell, made by `make` if the cell had none.
    pub fn get_or_insert_with(&mut self, idx: CellIdx, make: impl FnOnce() -> T) -> &mut T {
        let at = match self.find(idx) {
            Ok(at) => at,
            Err(at) => {
                self.entries.insert(at, (idx, make()));
                at
            }
        };
        &mut self.entries[at].1
    }

    /// Takes a cell's value away and returns it.
    pub fn remove(&mut self, idx: CellIdx) -> Option<T> {
        self.find(idx).ok().map(|at| self.entries.remove(at).1)
    }

    /// Removes every value.
    pub fn clear(&mut self) {
        self.entries.clear();
    }

    /// The cells that have a value, with their values, in cell order.
    pub fn iter(&self) -> impl Iterator<Item = (CellIdx, &T)> + '_ {
        self.entries.iter().map(|(idx, value)| (*idx, value))
    }

    /// The bytes this layer holds on the heap.
    pub fn heap_bytes(&self) -> usize {
        self.entries.capacity() * core::mem::size_of::<(CellIdx, T)>()
    }
}

impl<T> Default for SparseCells<T> {
    fn default() -> SparseCells<T> {
        SparseCells::new()
    }
}

impl<T> FromIterator<(CellIdx, T)> for SparseCells<T> {
    /// Collects pairs given in any order. A cell given twice keeps the
    /// later value.
    fn from_iter<I: IntoIterator<Item = (CellIdx, T)>>(iter: I) -> SparseCells<T> {
        let mut layer = SparseCells::new();
        for (idx, value) in iter {
            layer.insert(idx, value);
        }
        layer
    }
}

impl<T: StateHash> StateHash for SparseCells<T> {
    fn state_hash(&self, hasher: &mut StateHasher) {
        hasher.write_len(self.entries.len());
        for (idx, value) in &self.entries {
            idx.state_hash(hasher);
            value.state_hash(hasher);
        }
    }
}

impl<T: Serialize> Serialize for SparseCells<T> {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        self.entries.serialize(serializer)
    }
}

impl<'de, T: Deserialize<'de>> Deserialize<'de> for SparseCells<T> {
    fn deserialize<D: serde::Deserializer<'de>>(
        deserializer: D,
    ) -> Result<SparseCells<T>, D::Error> {
        let entries = Vec::<(CellIdx, T)>::deserialize(deserializer)?;
        if !entries.windows(2).all(|pair| pair[0].0 < pair[1].0) {
            return Err(serde::de::Error::custom(
                "sparse cells must be listed in cell order, each cell once",
            ));
        }
        Ok(SparseCells { entries })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use clash_core::Hash128;

    #[test]
    fn entries_stay_in_cell_order_whatever_the_insertion_order() {
        let a = CellIdx::new(5, 0, 0);
        let b = CellIdx::new(0, 1, 0);
        let c = CellIdx::new(0, 0, 1);
        let one: SparseCells<u8> = [(c, 3), (a, 1), (b, 2)].into_iter().collect();
        let two: SparseCells<u8> = [(a, 1), (b, 2), (c, 3)].into_iter().collect();
        assert_eq!(one, two);
        assert_eq!(Hash128::of(&one), Hash128::of(&two));
        assert_eq!(
            one.iter().map(|(idx, _)| idx).collect::<Vec<_>>(),
            [a, b, c]
        );
    }

    #[test]
    fn values_change_in_place_and_leave() {
        let at = CellIdx::new(2, 2, 2);
        let mut layer: SparseCells<u32> = SparseCells::new();
        *layer.get_or_insert_with(at, || 1) += 4;
        assert_eq!(layer.get(at), Some(&5));
        assert_eq!(layer.insert(at, 9), Some(5));
        *layer.get_mut(at).unwrap() = 2;
        assert_eq!(layer.len(), 1);
        assert_eq!(layer.remove(at), Some(2));
        assert_eq!(layer.remove(at), None);
    }

    #[test]
    fn a_layer_round_trips_and_refuses_disorder() {
        let layer: SparseCells<u8> = [(CellIdx::new(1, 0, 0), 7), (CellIdx::new(0, 0, 1), 8)]
            .into_iter()
            .collect();
        let json = serde_json::to_string(&layer).unwrap();
        assert_eq!(
            serde_json::from_str::<SparseCells<u8>>(&json).unwrap(),
            layer
        );
        assert!(serde_json::from_str::<SparseCells<u8>>("[[5, 1], [2, 1]]").is_err());
        assert!(serde_json::from_str::<SparseCells<u8>>("[[5, 1], [5, 2]]").is_err());
    }
}
