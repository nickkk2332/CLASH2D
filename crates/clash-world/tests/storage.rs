//! Property tests for W2: `Paletted` against a plain `Vec` model, and
//! `SparseCells` against a `BTreeMap` model.

use std::collections::BTreeMap;

use clash_core::Hash128;
use clash_world::CellIdx;
use clash_world::dims::CHUNK_CELLS;
use clash_world::storage::{CellBits, Paletted, SparseCells};
use proptest::prelude::*;

/// One step of a random history of a layer.
#[derive(Clone, Debug)]
enum Op {
    Set(usize, u16),
    /// Sets a run of cells, each to a different value, to push the palette
    /// through its wider forms.
    Spread(usize, u16, u16),
    Fill(u16),
    Compact,
    RoundTrip,
}

fn op() -> impl Strategy<Value = Op> {
    prop_oneof![
        8 => (0..CHUNK_CELLS, 0..6_u16).prop_map(|(at, value)| Op::Set(at, value)),
        2 => (0..CHUNK_CELLS, any::<u16>()).prop_map(|(at, value)| Op::Set(at, value)),
        1 => (0..CHUNK_CELLS, 1..400_u16, any::<u16>())
            .prop_map(|(at, count, first)| Op::Spread(at, count, first)),
        1 => (0..6_u16).prop_map(Op::Fill),
        1 => Just(Op::Compact),
        1 => Just(Op::RoundTrip),
    ]
}

fn idx(index: usize) -> CellIdx {
    CellIdx::from_index(index % CHUNK_CELLS).unwrap()
}

/// A layer built directly from a model, cell by cell.
fn fresh(model: &[u16]) -> Paletted<u16> {
    let mut layer = Paletted::uniform(model[0]);
    for (index, value) in model.iter().enumerate() {
        layer.set(idx(index), *value);
    }
    layer
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(64))]

    #[test]
    fn a_paletted_layer_behaves_like_a_vec(start in 0..6_u16, ops in prop::collection::vec(op(), 1..60)) {
        let mut model = vec![start; CHUNK_CELLS];
        let mut layer = Paletted::uniform(start);
        for op in ops {
            match op {
                Op::Set(at, value) => {
                    let old = layer.set(idx(at), value);
                    prop_assert_eq!(old, model[at]);
                    model[at] = value;
                }
                Op::Spread(at, count, first) => {
                    for step in 0..usize::from(count) {
                        let cell = (at + step) % CHUNK_CELLS;
                        let value = first.wrapping_add(step as u16);
                        prop_assert_eq!(layer.set(idx(cell), value), model[cell]);
                        model[cell] = value;
                    }
                }
                Op::Fill(value) => {
                    layer.fill(value);
                    model.fill(value);
                }
                Op::Compact => {
                    layer.compact();
                    let all_equal = model.iter().all(|value| *value == model[0]);
                    prop_assert_eq!(layer.as_uniform().is_some(), all_equal);
                    let mut distinct = model.clone();
                    distinct.sort_unstable();
                    distinct.dedup();
                    prop_assert_eq!(layer.palette().len(), distinct.len());
                    prop_assert!(distinct.len() <= 1 << layer.bits().max(1) || all_equal);
                }
                Op::RoundTrip => {
                    let json = serde_json::to_string(&layer).unwrap();
                    layer = serde_json::from_str(&json).unwrap();
                }
            }
            prop_assert!(matches!(layer.bits(), 0 | 1 | 2 | 4 | 8 | 16));
            prop_assert!(layer.palette().len() <= 1 << layer.bits());
        }
        prop_assert!(layer.iter().eq(model.iter().copied()));
        for index in (0..CHUNK_CELLS).step_by(97) {
            prop_assert_eq!(layer.get(idx(index)), model[index]);
        }
        // Equal contents compare and hash equal whatever the history.
        let rebuilt = fresh(&model);
        prop_assert_eq!(&layer, &rebuilt);
        prop_assert_eq!(Hash128::of(&layer), Hash128::of(&rebuilt));
        prop_assert_eq!(
            serde_json::to_string(&layer).unwrap(),
            serde_json::to_string(&rebuilt).unwrap()
        );
    }

    #[test]
    fn sparse_cells_behave_like_an_ordered_map(
        ops in prop::collection::vec((0..200_usize, prop::option::of(any::<u8>())), 0..200)
    ) {
        let mut model: BTreeMap<CellIdx, u8> = BTreeMap::new();
        let mut layer: SparseCells<u8> = SparseCells::new();
        for (at, value) in ops {
            // Spread the cells over the chunk so that order is exercised.
            let cell = idx(at * 41);
            match value {
                Some(value) => prop_assert_eq!(layer.insert(cell, value), model.insert(cell, value)),
                None => prop_assert_eq!(layer.remove(cell), model.remove(&cell)),
            }
            prop_assert_eq!(layer.len(), model.len());
        }
        let listed: Vec<(CellIdx, u8)> = layer.iter().map(|(cell, value)| (cell, *value)).collect();
        let expected: Vec<(CellIdx, u8)> = model.into_iter().collect();
        prop_assert_eq!(&listed, &expected);
        let json = serde_json::to_string(&layer).unwrap();
        let loaded: SparseCells<u8> = serde_json::from_str(&json).unwrap();
        prop_assert_eq!(Hash128::of(&loaded), Hash128::of(&layer));
        prop_assert_eq!(loaded, layer);
    }

    #[test]
    fn cell_bits_behave_like_a_vec_of_bools(
        ops in prop::collection::vec((0..CHUNK_CELLS, any::<bool>()), 0..300)
    ) {
        let mut model = vec![false; CHUNK_CELLS];
        let mut bits = CellBits::new();
        for (at, value) in ops {
            prop_assert_eq!(bits.set(idx(at), value), model[at]);
            model[at] = value;
        }
        prop_assert_eq!(bits.count(), model.iter().filter(|bit| **bit).count());
        let set: Vec<usize> = bits.iter().map(CellIdx::index).collect();
        let expected: Vec<usize> = (0..CHUNK_CELLS).filter(|index| model[*index]).collect();
        prop_assert_eq!(set, expected);
    }
}
