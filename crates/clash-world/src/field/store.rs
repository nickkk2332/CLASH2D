//! [`FieldStore`]: the fields of one chunk (02 §16). Sparse per field:
//! a chunk with no field costs one pointer.

use clash_core::{EntityId, StateHash, StateHasher};
use serde::{Deserialize, Serialize};

use crate::chunk::FieldId;
use crate::dims::CHUNK_CELLS;
use crate::pos::CellIdx;
use crate::storage::SparseCells;

/// One field in one cell.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub struct FieldCell {
    /// How much of the field the cell holds, 1 to 255.
    pub level: u8,
    /// Whether the cell is active: stepped with its field. A cell that
    /// is not is asleep until something near it changes.
    pub awake: bool,
    /// Who is responsible for the field being here: the instigator of
    /// whatever first brought it, passed along as it spreads (02 §7.3).
    pub by: Option<EntityId>,
}

impl StateHash for FieldCell {
    fn state_hash(&self, hasher: &mut StateHasher) {
        hasher.write_u8(self.level);
        hasher.write_bool(self.awake);
        self.by.state_hash(hasher);
    }
}

/// A layer holding more cells than this is kept as a table of every
/// cell of its chunk, so that a cell is found, added and taken away
/// without moving any other (D-734).
pub const FIELD_DENSE_FROM: usize = 512;

/// A layer kept as a table goes back to a list when it holds fewer
/// cells than this.
pub const FIELD_SPARSE_BELOW: usize = 128;

/// How a layer keeps its cells: a list in cell order, or a table of
/// every cell of the chunk with a count of those that hold the field.
/// Which it is changes nothing that can be seen: not what is read, not
/// the order cells come in, not what is saved or hashed.
#[derive(Clone, Debug)]
enum Cells {
    List(SparseCells<FieldCell>),
    Table {
        cells: Box<[Option<FieldCell>]>,
        len: usize,
    },
}

impl Default for Cells {
    fn default() -> Cells {
        Cells::List(SparseCells::new())
    }
}

impl Cells {
    #[inline]
    fn get(&self, idx: CellIdx) -> Option<&FieldCell> {
        match self {
            Cells::List(list) => list.get(idx),
            Cells::Table { cells, .. } => cells.get(idx.index())?.as_ref(),
        }
    }

    #[inline]
    fn get_mut(&mut self, idx: CellIdx) -> Option<&mut FieldCell> {
        match self {
            Cells::List(list) => list.get_mut(idx),
            Cells::Table { cells, .. } => cells.get_mut(idx.index())?.as_mut(),
        }
    }

    fn len(&self) -> usize {
        match self {
            Cells::List(list) => list.len(),
            Cells::Table { len, .. } => *len,
        }
    }

    fn insert(&mut self, idx: CellIdx, cell: FieldCell) -> Option<FieldCell> {
        let old = match self {
            Cells::List(list) => list.insert(idx, cell),
            Cells::Table { cells, len } => {
                let old = cells.get_mut(idx.index())?.replace(cell);
                if old.is_none() {
                    *len += 1;
                }
                old
            }
        };
        if let Cells::List(list) = self
            && list.len() > FIELD_DENSE_FROM
        {
            let mut cells = vec![None; CHUNK_CELLS].into_boxed_slice();
            for (at, cell) in list.iter() {
                cells[at.index()] = Some(*cell);
            }
            let len = list.len();
            *self = Cells::Table { cells, len };
        }
        old
    }

    fn remove(&mut self, idx: CellIdx) -> Option<FieldCell> {
        let old = match self {
            Cells::List(list) => list.remove(idx),
            Cells::Table { cells, len } => {
                let old = cells.get_mut(idx.index())?.take();
                if old.is_some() {
                    *len -= 1;
                }
                old
            }
        };
        if let Cells::Table { len, .. } = self
            && *len < FIELD_SPARSE_BELOW
        {
            let list = self.list();
            *self = Cells::List(list);
        }
        old
    }

    /// The cells as a list in cell order.
    fn list(&self) -> SparseCells<FieldCell> {
        match self {
            Cells::List(list) => list.clone(),
            Cells::Table { .. } => {
                let mut list = SparseCells::new();
                for (idx, cell) in self.iter() {
                    list.insert(idx, *cell);
                }
                list
            }
        }
    }

    fn iter(&self) -> CellsIter<'_> {
        match self {
            Cells::List(list) => CellsIter::List(list.entries().iter()),
            Cells::Table { cells, .. } => CellsIter::Table(cells.iter().enumerate()),
        }
    }

    fn heap_bytes(&self) -> usize {
        match self {
            Cells::List(list) => list.heap_bytes(),
            Cells::Table { cells, .. } => cells.len() * core::mem::size_of::<Option<FieldCell>>(),
        }
    }
}

/// The cells of a layer that hold the field, in cell order.
enum CellsIter<'a> {
    List(core::slice::Iter<'a, (CellIdx, FieldCell)>),
    Table(core::iter::Enumerate<core::slice::Iter<'a, Option<FieldCell>>>),
}

impl<'a> Iterator for CellsIter<'a> {
    type Item = (CellIdx, &'a FieldCell);

    #[inline]
    fn next(&mut self) -> Option<(CellIdx, &'a FieldCell)> {
        match self {
            CellsIter::List(list) => list.next().map(|(idx, cell)| (*idx, cell)),
            CellsIter::Table(table) => {
                table.find_map(|(at, cell)| Some((CellIdx::from_index(at)?, cell.as_ref()?)))
            }
        }
    }
}

/// One field's cells in a chunk.
#[derive(Clone, Default, Debug, Serialize, Deserialize)]
#[serde(from = "SparseCells<FieldCell>", into = "SparseCells<FieldCell>")]
pub struct FieldLayer {
    cells: Cells,
    /// How many of the cells are awake.
    awake: u32,
}

/// Two layers are equal when they hold the same cells, however each
/// keeps them.
impl PartialEq for FieldLayer {
    fn eq(&self, other: &FieldLayer) -> bool {
        self.awake == other.awake
            && self.cells.len() == other.cells.len()
            && self.cells.iter().eq(other.cells.iter())
    }
}

impl Eq for FieldLayer {}

impl From<SparseCells<FieldCell>> for FieldLayer {
    fn from(cells: SparseCells<FieldCell>) -> FieldLayer {
        let awake = cells.iter().filter(|(_, cell)| cell.awake).count();
        let mut layer = FieldLayer {
            cells: Cells::List(SparseCells::new()),
            awake: u32::try_from(awake).unwrap_or(u32::MAX),
        };
        if cells.len() > FIELD_DENSE_FROM {
            for (idx, cell) in cells.iter() {
                layer.cells.insert(idx, *cell);
            }
        } else {
            layer.cells = Cells::List(cells);
        }
        layer
    }
}

impl From<FieldLayer> for SparseCells<FieldCell> {
    fn from(layer: FieldLayer) -> SparseCells<FieldCell> {
        match layer.cells {
            Cells::List(list) => list,
            table => table.list(),
        }
    }
}

/// The cells in cell order, as a list of them is hashed.
impl StateHash for FieldLayer {
    fn state_hash(&self, hasher: &mut StateHasher) {
        hasher.write_len(self.cells.len());
        for (idx, cell) in self.cells.iter() {
            idx.state_hash(hasher);
            cell.state_hash(hasher);
        }
    }
}

impl FieldLayer {
    /// The field in a cell, if the cell holds any.
    #[inline]
    pub fn get(&self, idx: CellIdx) -> Option<&FieldCell> {
        self.cells.get(idx)
    }

    /// How many cells hold the field.
    pub fn len(&self) -> usize {
        self.cells.len()
    }

    /// Whether no cell holds the field.
    pub fn is_empty(&self) -> bool {
        self.cells.len() == 0
    }

    /// How many cells of the field are awake.
    pub fn awake(&self) -> u32 {
        self.awake
    }

    /// The cells that hold the field, in cell order.
    pub fn cells(&self) -> impl Iterator<Item = (CellIdx, &FieldCell)> + '_ {
        self.cells.iter()
    }
}

/// The fields of one chunk: for each field that is anywhere in it, the
/// cells that hold it, with their levels (02 §16).
///
/// A field in a cell has a level from 1 to 255; a cell that holds none
/// of a field is not stored. Reading is public. Changing is not: fields
/// change through `WorldEdit` and through the stepper.
#[derive(Clone, PartialEq, Eq, Default, Debug, Serialize, Deserialize)]
#[serde(transparent)]
pub struct FieldStore {
    /// By field, in id order. `None` when the chunk holds no field, as
    /// for most chunks: an empty store is one pointer wide.
    #[allow(
        clippy::box_collection,
        reason = "a chunk holds one of these and most chunks hold no field: boxed, an empty one is 8 bytes and not 24"
    )]
    layers: Option<Box<Vec<(FieldId, FieldLayer)>>>,
}

impl FieldStore {
    #[inline]
    fn list(&self) -> &[(FieldId, FieldLayer)] {
        self.layers.as_deref().map_or(&[], Vec::as_slice)
    }

    /// Whether the chunk holds no field at all.
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.layers.is_none()
    }

    /// The fields the chunk holds, each with its cells, in id order.
    pub fn layers(&self) -> impl Iterator<Item = (FieldId, &FieldLayer)> + '_ {
        self.list().iter().map(|(field, layer)| (*field, layer))
    }

    /// One field's cells, if the chunk holds any of it.
    #[inline]
    pub fn layer(&self, field: FieldId) -> Option<&FieldLayer> {
        let list = self.list();
        list.binary_search_by_key(&field, |(have, _)| *have)
            .ok()
            .map(|at| &list[at].1)
    }

    /// One field in one cell, if the cell holds any of it.
    #[inline]
    pub fn cell(&self, field: FieldId, idx: CellIdx) -> Option<&FieldCell> {
        self.layer(field)?.get(idx)
    }

    /// The level of a field in a cell: 0 where there is none.
    #[inline]
    pub fn level(&self, field: FieldId, idx: CellIdx) -> u8 {
        self.cell(field, idx).map_or(0, |cell| cell.level)
    }

    /// Every field in one cell with its level, in id order.
    pub fn at(&self, idx: CellIdx) -> impl Iterator<Item = (FieldId, &FieldCell)> + '_ {
        self.list()
            .iter()
            .filter_map(move |(field, layer)| Some((*field, layer.get(idx)?)))
    }

    /// How many cells of any field are awake.
    pub fn awake(&self) -> u32 {
        self.list()
            .iter()
            .fold(0, |sum, (_, layer)| sum.saturating_add(layer.awake))
    }

    /// The bytes the store holds on the heap.
    pub fn heap_bytes(&self) -> usize {
        self.layers.as_ref().map_or(0, |layers| {
            core::mem::size_of::<Vec<(FieldId, FieldLayer)>>()
                + layers.capacity() * core::mem::size_of::<(FieldId, FieldLayer)>()
                + layers
                    .iter()
                    .map(|(_, layer)| layer.cells.heap_bytes())
                    .sum::<usize>()
        })
    }

    /// Sets what a cell holds of a field, or with `None` takes it away,
    /// and returns what it held.
    pub(crate) fn set(
        &mut self,
        field: FieldId,
        idx: CellIdx,
        cell: Option<FieldCell>,
    ) -> Option<FieldCell> {
        let cell = cell.filter(|cell| cell.level > 0);
        let Some(cell) = cell else {
            let layers = self.layers.as_mut()?;
            let at = layers
                .binary_search_by_key(&field, |(have, _)| *have)
                .ok()?;
            let layer = &mut layers[at].1;
            let old = layer.cells.remove(idx)?;
            if old.awake {
                layer.awake = layer.awake.saturating_sub(1);
            }
            if layer.is_empty() {
                layers.remove(at);
                if layers.is_empty() {
                    self.layers = None;
                }
            }
            return Some(old);
        };
        let layers = self.layers.get_or_insert_with(Box::default);
        let at = match layers.binary_search_by_key(&field, |(have, _)| *have) {
            Ok(at) => at,
            Err(at) => {
                layers.insert(at, (field, FieldLayer::default()));
                at
            }
        };
        let layer = &mut layers[at].1;
        let old = layer.cells.insert(idx, cell);
        if old.is_some_and(|old| old.awake) {
            layer.awake = layer.awake.saturating_sub(1);
        }
        if cell.awake {
            layer.awake = layer.awake.saturating_add(1);
        }
        old
    }

    /// Wakes a cell of a field or puts it to sleep. Returns whether
    /// that changed anything.
    pub(crate) fn set_awake(&mut self, field: FieldId, idx: CellIdx, awake: bool) -> bool {
        let Some(layers) = self.layers.as_mut() else {
            return false;
        };
        let Ok(at) = layers.binary_search_by_key(&field, |(have, _)| *have) else {
            return false;
        };
        let layer = &mut layers[at].1;
        match layer.cells.get_mut(idx) {
            Some(cell) if cell.awake != awake => {
                cell.awake = awake;
                if awake {
                    layer.awake = layer.awake.saturating_add(1);
                } else {
                    layer.awake = layer.awake.saturating_sub(1);
                }
                true
            }
            _ => false,
        }
    }

    /// Whether any cell of the chunk is asleep.
    pub(crate) fn any_asleep(&self) -> bool {
        self.list()
            .iter()
            .any(|(_, layer)| (layer.awake as usize) < layer.cells.len())
    }
}

/// Hashes the fields in id order, each with its cells in cell order.
impl StateHash for FieldStore {
    fn state_hash(&self, hasher: &mut StateHasher) {
        let list = self.list();
        hasher.write_len(list.len());
        for (field, layer) in list {
            field.state_hash(hasher);
            layer.state_hash(hasher);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use clash_core::Hash128;

    fn cell(level: u8, awake: bool) -> FieldCell {
        FieldCell {
            level,
            awake,
            by: None,
        }
    }

    #[test]
    fn a_store_appears_and_vanishes_with_what_it_holds() {
        let mut store = FieldStore::default();
        let empty = Hash128::of(&store);
        assert!(store.is_empty());
        assert_eq!(core::mem::size_of::<FieldStore>(), 8);
        let (smoke, fire) = (FieldId::new(3), FieldId::new(1));
        let at = CellIdx::new(4, 5, 6);

        assert_eq!(store.set(smoke, at, Some(cell(40, true))), None);
        assert_eq!(store.set(fire, at, Some(cell(9, false))), None);
        assert_eq!(store.level(smoke, at), 40);
        assert_eq!(store.level(fire, CellIdx::new(0, 0, 0)), 0);
        assert_eq!(store.awake(), 1);
        assert!(store.any_asleep());
        // Fields come in id order whatever order they arrived in.
        let listed: Vec<FieldId> = store.at(at).map(|(field, _)| field).collect();
        assert_eq!(listed, [fire, smoke]);
        assert_ne!(Hash128::of(&store), empty);

        assert!(store.set_awake(fire, at, true));
        assert!(!store.set_awake(fire, at, true));
        assert_eq!(store.awake(), 2);
        // A level of nothing is no cell.
        assert_eq!(
            store.set(fire, at, Some(cell(0, true))),
            Some(cell(9, true))
        );
        assert_eq!(store.awake(), 1);
        assert_eq!(store.set(smoke, at, None), Some(cell(40, true)));
        assert_eq!(store.set(smoke, at, None), None);
        assert!(store.is_empty());
        assert_eq!(Hash128::of(&store), empty);
        assert_eq!(store.heap_bytes(), 0);
    }

    /// D-734: a layer that grows past [`FIELD_DENSE_FROM`] cells and
    /// shrinks back reads, iterates, hashes and saves exactly as a list
    /// of its cells does, the reference.
    #[test]
    fn a_layer_kept_as_a_table_is_its_list_to_every_reader() {
        let field = FieldId::new(1);
        let mut store = FieldStore::default();
        let mut reference: SparseCells<FieldCell> = SparseCells::new();
        let mut state = 0x2545_f491_u32;
        let mut became_table = false;
        for step in 0..6000_u32 {
            state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            let idx = CellIdx::from_index((state >> 8) as usize % CHUNK_CELLS).unwrap();
            // Grow for the first half, then mostly shrink.
            let adding = if step < 3000 {
                state % 5 != 0
            } else {
                state % 5 == 0
            };
            let level = (state >> 24) as u8;
            if adding && level > 0 {
                let new = cell(level, state % 3 != 0);
                assert_eq!(store.set(field, idx, Some(new)), reference.insert(idx, new));
            } else {
                assert_eq!(store.set(field, idx, None), reference.remove(idx));
            }
            if let Some(layer) = store.layer(field) {
                became_table |= matches!(layer.cells, Cells::Table { .. });
                assert_eq!(layer.len(), reference.len());
                assert_eq!(layer.get(idx), reference.get(idx));
                if step % 97 == 0 {
                    assert!(layer.cells().eq(reference.iter()));
                    let awake = reference.iter().filter(|(_, cell)| cell.awake).count();
                    assert_eq!(layer.awake() as usize, awake);
                    assert_eq!(store.awake() as usize, awake);
                    let as_list = FieldLayer::from(reference.clone());
                    assert_eq!(layer, &as_list);
                    assert_eq!(Hash128::of(layer), Hash128::of(&reference));
                    let bytes = clash_core::codec::to_bytes(layer).unwrap();
                    assert_eq!(bytes, clash_core::codec::to_bytes(&reference).unwrap());
                    let back: FieldLayer = clash_core::codec::from_bytes(&bytes).unwrap();
                    assert_eq!(&back, layer);
                }
            } else {
                assert!(reference.is_empty());
            }
        }
        assert!(became_table, "the layer never grew into a table");
    }

    #[test]
    fn a_store_round_trips_and_counts_again_who_is_awake() {
        let mut store = FieldStore::default();
        let field = FieldId::new(2);
        store.set(field, CellIdx::new(1, 1, 1), Some(cell(7, true)));
        store.set(field, CellIdx::new(2, 1, 1), Some(cell(8, false)));
        let bytes = clash_core::codec::to_bytes(&store).unwrap();
        let back: FieldStore = clash_core::codec::from_bytes(&bytes).unwrap();
        assert_eq!(back, store);
        assert_eq!(back.awake(), 1);
        let json = serde_json::to_string(&store).unwrap();
        assert_eq!(serde_json::from_str::<FieldStore>(&json).unwrap(), store);
    }
}
