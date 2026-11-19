//! Tests for the tract store and the two functions that turn a tract
//! into a blob and back (02 §11). Loading and unloading through them is
//! tested with the generators, in `clash-gen`; saving a whole world, the
//! file store and what a change of mods does are in `save.rs`.

mod common;

use std::sync::Arc;

use clash_core::{Hash128, Value, VarKey};
use clash_time::Timeline;
use clash_world::test_support::load_column;
use clash_world::{
    BlockState, Cause, CellPos, Column, ColumnBuilder, Cosmos, DecalId, EventSink, FixtureState,
    FloorState, GridId, Layer, Location, MemoryStore, RealmId, SaveIds, StoreError,
    TRACT_BLOB_KIND, TRACT_BLOB_VERSION, TractPos, TractStore, WorldCtx, WorldDefs, decode_tract,
    encode_tract,
};
use proptest::prelude::*;

/// The states the test columns are built from.
struct Palette {
    blocks: Vec<BlockState>,
    floors: Vec<FloorState>,
    fixtures: Vec<FixtureState>,
}

fn palette(defs: &WorldDefs) -> Palette {
    let block = |name: &str| defs.block_state(name, None).unwrap();
    Palette {
        blocks: vec![
            BlockState::NONE,
            block("wall"),
            block("wall@brick"),
            block("door#open"),
            block("pillar"),
            block("solid"),
        ],
        floors: vec![
            FloorState::NONE,
            defs.floor_state("slab", None).unwrap(),
            defs.floor_state("grass", None).unwrap(),
        ],
        fixtures: vec![
            FixtureState::NONE,
            defs.fixture_state("desk", None).unwrap(),
            defs.fixture_state("rubble#3", None).unwrap(),
        ],
    }
}

/// One write to a column being built: a layer, a cell and which state
/// of the palette.
type Write = (u8, (i32, i32, i32), usize);

fn build(defs: &WorldDefs, surface_z: i32, writes: &[Write]) -> Column {
    let states = palette(defs);
    let rock = defs.block_state("solid@granite", None).unwrap();
    let mut column = ColumnBuilder::new(defs, rock, BlockState::NONE, surface_z);
    for &(layer, (x, y, z), pick) in writes {
        let cell = CellPos::new(x, y, z);
        match layer % 3 {
            0 => column.set_floor(cell, states.floors[pick % states.floors.len()]),
            1 => column.set_block(cell, states.blocks[pick % states.blocks.len()]),
            _ => column.set_fixture(cell, states.fixtures[pick % states.fixtures.len()]),
        };
    }
    column.build()
}

/// A column as a tract blob with no entity section, against tables
/// of ids that have named nothing yet.
fn encode_column(defs: &WorldDefs, column: &Column) -> (Vec<u8>, SaveIds) {
    let mut ids = SaveIds::new(defs);
    let blob = encode_tract(defs, &mut ids, Some(column), &[]).unwrap();
    (blob, ids)
}

/// The cells out of a tract blob.
fn decode_column(defs: &WorldDefs, ids: &SaveIds, blob: &[u8]) -> Result<Column, StoreError> {
    decode_tract(defs, ids, blob).map(|tract| tract.cells.expect("the blob holds cells"))
}

fn same_cells(a: &Column, b: &Column, writes: &[Write]) {
    assert_eq!(Hash128::of(a), Hash128::of(b));
    assert_eq!(a.below_fill(), b.below_fill());
    assert_eq!(a.above_fill(), b.above_fill());
    for &(_, (x, y, z), _) in writes {
        for dz in -1..=1 {
            let cell = CellPos::new(x, y, z + dz);
            assert_eq!(a.cell(cell), b.cell(cell), "{cell}");
            assert_eq!(a.flags(cell), b.flags(cell), "{cell}");
            assert_eq!(a.height_at(cell), b.height_at(cell), "{cell}");
            assert_eq!(a.standable(cell), b.standable(cell), "{cell}");
        }
    }
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(48))]

    /// Whatever a column holds, the blob gives back the same cells, with
    /// the caches derived from them right.
    #[test]
    fn a_column_comes_back_out_of_its_blob(
        surface_z in -20_i32..20,
        writes in prop::collection::vec(
            (0_u8..3, (0_i32..32, 0_i32..32, -30_i32..40), 0_usize..6),
            0..200,
        ),
    ) {
        let defs = common::test_defs();
        let column = build(&defs, surface_z, &writes);
        let (blob, ids) = encode_column(&defs, &column);
        let back = decode_column(&defs, &ids, &blob).unwrap();
        same_cells(&column, &back, &writes);
        prop_assert!(!back.is_dirty());
        // Equal cells give equal bodies, and so the same blob again:
        // against the same tables, and against new ones, which name the
        // definitions in the order the cells first hold them.
        let mut same = ids.clone();
        prop_assert_eq!(&encode_tract(&defs, &mut same, Some(&back), &[]).unwrap(), &blob);
        prop_assert_eq!(&same, &ids);
        prop_assert_eq!(encode_column(&defs, &back).0, blob);
    }

    /// The entity section comes back byte for byte, with cells and
    /// without, and the world never looks inside it.
    #[test]
    fn the_entity_section_is_kept_as_it_was_given(
        entities in prop::collection::vec(any::<u8>(), 0..300),
        with_cells in any::<bool>(),
    ) {
        let defs = common::test_defs();
        let column = build(&defs, 0, &[(1, (3, 3, 0), 1)]);
        let mut ids = SaveIds::new(&defs);
        let cells = with_cells.then_some(&column);
        let blob = encode_tract(&defs, &mut ids, cells, &entities).unwrap();
        let back = decode_tract(&defs, &ids, &blob).unwrap();
        prop_assert_eq!(&back.entities, &entities);
        prop_assert_eq!(back.cells.is_some(), with_cells);
        prop_assert!(back.missing.is_empty());
        if let Some(cells) = &back.cells {
            prop_assert_eq!(Hash128::of(cells), Hash128::of(&column));
        }
    }
}

#[test]
fn damage_marks_and_variables_are_stored_with_the_cells() {
    let defs = common::test_defs();
    let mut timeline = Timeline::new();
    let mut cosmos = Cosmos::new(Arc::clone(&defs));
    let def = defs.realms().lookup("core:testland").unwrap();
    let realm = cosmos.create_realm(&mut timeline, def, 1).unwrap();
    let grid: GridId = cosmos.realm(realm).unwrap().base_grid;
    let tract = TractPos::new(0, 0);
    let writes: Vec<Write> = vec![(1, (3, 3, 0), 1), (0, (3, 3, 0), 1), (2, (4, 3, 0), 1)];
    load_column(&mut cosmos, grid, tract, build(&defs, 0, &writes)).unwrap();
    let at = Location::new(grid, CellPos::new(3, 3, 0));
    let key = VarKey::new("core:mark");
    let mut events = EventSink::new();
    {
        let mut ctx = WorldCtx::new(&mut cosmos, &mut timeline, &mut events);
        let mut edit = ctx.edit(Cause::DEBUG);
        edit.set_damage(at, Layer::Block, 40).unwrap();
        edit.add_decal(at, DecalId::new(2), 7).unwrap();
        edit.set_var(at, key.clone(), Value::from(12)).unwrap();
    }
    let view = cosmos.grid(grid).unwrap();
    let column = view.column(tract).unwrap();
    assert!(column.is_dirty());
    let (blob, ids) = encode_column(&defs, column);
    let back = decode_column(&defs, &ids, &blob).unwrap();
    same_cells(column, &back, &writes);
    assert_eq!(back.damage(at.cell), [0, 40, 0]);
    assert_eq!(back.var(at.cell, "core:mark"), Some(&Value::from(12)));
    let decals = back.chunk(0).unwrap().decals(at.cell.idx());
    assert_eq!(decals, cosmos.decals(at).unwrap());
    assert_eq!(decals.len(), 1);
    // What is derived or belongs to one loading is not stored.
    assert!(!back.is_dirty());
    assert!(!back.has_occupants());
    assert_eq!(back.rev_base(), 0);
}

#[test]
fn a_column_that_is_all_fill_is_a_small_blob() {
    let defs = common::test_defs();
    let ground = build(&defs, 0, &[]);
    assert_eq!(ground.stored_range(), None);
    let (blob, ids) = encode_column(&defs, &ground);
    assert!(blob.len() < 64, "{} bytes", blob.len());
    let back = decode_column(&defs, &ids, &blob).unwrap();
    assert_eq!(Hash128::of(&back), Hash128::of(&ground));
    assert!(back.standable(CellPos::new(5, 5, 0)));
    assert!(!back.standable(CellPos::new(5, 5, 1)));
    // A surface that is not at the base of a chunk stores that chunk.
    let split = build(&defs, 3, &[]);
    let (blob, ids) = encode_column(&defs, &split);
    let back = decode_column(&defs, &ids, &blob).unwrap();
    assert_eq!(Hash128::of(&back), Hash128::of(&split));
    assert!(back.standable(CellPos::new(0, 0, 3)));
    assert_ne!(Hash128::of(&back), Hash128::of(&ground));
}

#[test]
fn a_damaged_blob_is_an_error_and_never_a_panic() {
    let defs = common::test_defs();
    let writes: Vec<Write> = (0..40)
        .map(|i| (i as u8, (i, 31 - i % 32, i % 9), i as usize))
        .collect();
    let column = build(&defs, 0, &writes);
    let (blob, ids) = encode_column(&defs, &column);
    assert!(decode_column(&defs, &ids, &blob).is_ok());
    // Cut short anywhere.
    for len in 0..blob.len() {
        assert!(
            decode_tract(&defs, &ids, &blob[..len]).is_err(),
            "cut to {len}"
        );
    }
    // Any one byte changed: the frame or the checksum catches it. A
    // change the compression happens to absorb leaves the cells as they
    // were; nothing reads back as other cells.
    let mut caught = 0;
    for at in 0..blob.len() {
        let mut damaged = blob.clone();
        damaged[at] ^= 0x41;
        match decode_column(&defs, &ids, &damaged) {
            Err(_) => caught += 1,
            Ok(back) => assert_eq!(Hash128::of(&back), Hash128::of(&column), "byte {at}"),
        }
    }
    assert!(caught * 10 >= blob.len() * 9, "{caught} of {}", blob.len());
    // Something that is not a tract at all, and a tract of a layout
    // this build does not read.
    assert!(matches!(
        decode_tract(&defs, &ids, b"cells"),
        Err(StoreError::Codec(_))
    ));
    let other = clash_core::codec::seal_blob(*b"SAVE", 1, &[1, 2, 3]).unwrap();
    assert!(matches!(
        decode_tract(&defs, &ids, &other),
        Err(StoreError::Codec(_))
    ));
    // A tract of a layout this build does not read: a later one, and
    // the first one, which held dense ids and was never written to disk.
    for version in [TRACT_BLOB_VERSION + 1, 1] {
        let blob = clash_core::codec::seal_blob(TRACT_BLOB_KIND, version, &[0]).unwrap();
        assert_eq!(
            decode_tract(&defs, &ids, &blob).err(),
            Some(StoreError::Version {
                found: version,
                expected: TRACT_BLOB_VERSION
            })
        );
    }
    // A body that reads as something else is refused by the decoder.
    let wrong = clash_core::codec::seal_blob(TRACT_BLOB_KIND, TRACT_BLOB_VERSION, &[9; 5]).unwrap();
    assert!(decode_tract(&defs, &ids, &wrong).is_err());
    // Cells that name a definition the tables do not hold: the blob is
    // sound and was written against other tables.
    let fresh = SaveIds::new(&defs);
    assert!(matches!(
        decode_tract(&defs, &fresh, &blob),
        Err(StoreError::Malformed(_))
    ));
}

#[test]
fn the_memory_store_keeps_blobs_by_realm_and_tract() {
    let mut store = MemoryStore::new();
    let (here, there) = (RealmId::new(0), RealmId::new(1));
    let tract = TractPos::new(-3, 8);
    store.put(here, tract, b"one");
    store.put(there, tract, b"two");
    store.put(here, TractPos::new(0, 0), b"three");
    assert_eq!(store.len(), 3);
    assert_eq!(store.get(here, tract), Ok(Some(b"one".to_vec())));
    assert_eq!(store.get(there, tract), Ok(Some(b"two".to_vec())));
    assert_eq!(store.get(there, TractPos::new(0, 0)), Ok(None));
    // By realm, then row by row.
    assert_eq!(
        store.tracts().collect::<Vec<_>>(),
        [(here, TractPos::new(0, 0)), (here, tract), (there, tract)]
    );
    store.put(here, tract, b"again");
    assert_eq!(store.get(here, tract), Ok(Some(b"again".to_vec())));
    store.delete(here, tract);
    store.delete(here, tract);
    assert_eq!(store.get(here, tract), Ok(None));
    assert_eq!(store.commits(), 0);
    assert_eq!(store.commit(), Ok(()));
    assert_eq!(store.commits(), 1);
    assert_eq!(store.len(), 2);
    // Sections are kept by name, beside the tracts.
    assert_eq!(store.section("sim.clock"), Ok(None));
    store.put_section("sim.clock", b"noon");
    store.put_section("a", b"first");
    store.put_section("sim.clock", b"dusk");
    assert_eq!(store.section("sim.clock"), Ok(Some(b"dusk".to_vec())));
    assert_eq!(
        store.section_names(),
        Ok(vec!["a".to_owned(), "sim.clock".to_owned()])
    );
    store.delete_section("a");
    store.delete_section("a");
    assert_eq!(store.section_names(), Ok(vec!["sim.clock".to_owned()]));
    assert_eq!(store.len(), 2);
    assert_eq!(store.stored_bytes(), 3 + 5 + 4);
}
