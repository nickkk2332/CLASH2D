//! Tests for W4: chunks, columns with fills and a heightmap, the derived
//! bitsets and revisions.

mod common;

use clash_core::Hash128;
use clash_world::dims::{CHUNK_AREA, CHUNK_XY, CHUNK_Z};
use clash_world::{
    BlockState, CellFlags, CellIdx, CellPos, Chunk, Column, ColumnBuilder, FixtureState,
    FloorState, HEIGHT_FULL, HEIGHT_NONE, WorldDefs,
};
use proptest::prelude::*;

struct Kit {
    defs: std::sync::Arc<WorldDefs>,
    rock: BlockState,
    wall: BlockState,
    window: BlockState,
    door_open: BlockState,
    slab: FloorState,
    grass: FloorState,
    desk: FixtureState,
    rubble: FixtureState,
}

fn kit() -> Kit {
    let defs = common::test_defs();
    Kit {
        rock: defs.block_state("solid", None).unwrap(),
        wall: defs.block_state("wall", None).unwrap(),
        window: defs.block_state("window", None).unwrap(),
        door_open: defs.block_state("door#open", None).unwrap(),
        slab: defs.floor_state("slab", None).unwrap(),
        grass: defs.floor_state("grass", None).unwrap(),
        desk: defs.fixture_state("desk", None).unwrap(),
        rubble: defs.fixture_state("rubble", None).unwrap(),
        defs,
    }
}

/// The heightmap entry of a cell column, found by looking at every storey
/// from well above the stored range to well below it.
fn brute_height(defs: &WorldDefs, column: &Column, index: usize) -> i32 {
    if !column.above_fill().is_none() {
        return HEIGHT_FULL;
    }
    // A column with nothing stored keeps its surface within a few chunks
    // of zero in these tests.
    let (lo, hi) = column.stored_range().unwrap_or((-2, 2));
    let x = (index as i32) % CHUNK_XY;
    let y = (index as i32) / CHUNK_XY;
    let top = (hi + 2) * CHUNK_Z;
    let bottom = (lo - 2) * CHUNK_Z;
    for z in (bottom..=top).rev() {
        let here = column.cell(CellPos::new(x, y, z));
        let below = column.cell(CellPos::new(x, y, z - 1));
        let stands = defs
            .block_info(below.block)
            .cell
            .contains(CellFlags::SOLID_TOP);
        if !here.is_empty() || stands {
            return z;
        }
    }
    HEIGHT_NONE
}

/// Checks a column's heightmap and derived bitsets against recomputation
/// from the cells.
fn check_caches(defs: &WorldDefs, column: &Column) -> Result<(), TestCaseError> {
    // The cell columns the random edits touch, and a sample of the rest.
    let edited = |index: usize| {
        let (x, y) = (index as i32 % CHUNK_XY, index as i32 / CHUNK_XY);
        x % 7 == 0 && x < 21 && y % 30 == 0
    };
    for index in (0..CHUNK_AREA).filter(|index| edited(*index) || index % 37 == 0) {
        prop_assert_eq!(
            column.height(index),
            brute_height(defs, column, index),
            "height of cell column {}",
            index
        );
    }
    for (chunk_z, chunk) in column.chunks() {
        for idx in CellIdx::all().filter(|idx| edited(idx.column()) || idx.index() % 13 == 0) {
            let expected = defs.cell_flags(chunk.floor(idx), chunk.block(idx), chunk.fixture(idx));
            prop_assert_eq!(
                chunk.flags(idx),
                expected,
                "flags in chunk {} at {:?}",
                chunk_z,
                idx
            );
            if let Some(cells) = chunk.flag_cells() {
                prop_assert_eq!(cells[idx.index()], expected);
            }
        }
        if let Some(flags) = chunk.uniform_flags() {
            prop_assert_eq!(flags, chunk.flags(CellIdx::new(0, 0, 0)));
        }
        // The bitset of one flag agrees with the per-cell flags.
        let opaque = chunk.cells_with(CellFlags::OPAQUE);
        for idx in CellIdx::all().filter(|idx| idx.index() % 13 == 0) {
            prop_assert_eq!(
                opaque.get(idx),
                chunk.flags(idx).contains(CellFlags::OPAQUE)
            );
        }
    }
    Ok(())
}

#[test]
fn reads_outside_the_stored_range_return_the_fill_and_writes_create_storage() {
    let kit = kit();
    let mut builder = ColumnBuilder::new(&kit.defs, kit.rock, BlockState::NONE, 0);
    assert_eq!(builder.column().stored_range(), None);
    let bytes = builder.column().memory_bytes();

    // Far below: rock. Far above: nothing. Neither read allocates.
    let deep = CellPos::new(5, 5, -100_000);
    let high = CellPos::new(5, 5, 100_000);
    assert_eq!(builder.column().block(deep), kit.rock);
    assert_eq!(builder.column().cell(deep).floor, FloorState::NONE);
    assert!(builder.column().flags(deep).contains(CellFlags::SOLID_TOP));
    assert!(builder.column().cell(high).is_empty());
    assert_eq!(builder.column().flags(high), CellFlags::NONE);
    assert_eq!(builder.column().stored_range(), None);
    assert_eq!(builder.column().memory_bytes(), bytes);

    // Bare rock is stood on without any floor: the surface is storey 0.
    assert!(builder.column().standable(CellPos::new(5, 5, 0)));
    assert!(!builder.column().standable(CellPos::new(5, 5, 1)));
    assert!(builder.column().is_open(CellPos::new(5, 5, 1)));
    assert!(!builder.column().is_open(CellPos::new(5, 5, 0)));
    assert_eq!(builder.column().height(0), 0);

    // Writing above the range creates storage up to there.
    builder.set_block(CellPos::new(5, 5, 20), kit.wall);
    assert_eq!(builder.column().stored_range(), Some((0, 2)));
    assert_eq!(builder.column().block(CellPos::new(5, 5, 20)), kit.wall);
    assert!(builder.column().memory_bytes() > bytes);
    // Writing below it creates storage down to there, still rock around.
    builder.set_block(CellPos::new(1, 1, -9), BlockState::NONE);
    assert_eq!(builder.column().stored_range(), Some((-2, 2)));
    assert_eq!(
        builder.column().block(CellPos::new(1, 1, -9)),
        BlockState::NONE
    );
    assert_eq!(builder.column().block(CellPos::new(2, 1, -9)), kit.rock);
    assert_eq!(builder.column().block(CellPos::new(1, 1, -17)), kit.rock);
    check_caches(&kit.defs, builder.column()).unwrap();
}

#[test]
fn a_uniform_chunk_is_small() {
    let kit = kit();
    let chunk = Chunk::uniform(&kit.defs, kit.rock);
    assert!(
        chunk.memory_bytes() <= 200,
        "{} bytes",
        chunk.memory_bytes()
    );
    assert_eq!(
        chunk.uniform_flags(),
        Some(kit.defs.block_info(kit.rock).cell)
    );
    assert!(chunk.flag_cells().is_none());
    assert!(!chunk.is_dirty());
}

#[test]
fn standing_follows_floors_and_solid_tops() {
    let kit = kit();
    let mut builder = ColumnBuilder::new(&kit.defs, kit.rock, BlockState::NONE, 0);
    let at = |z| CellPos::new(3, 3, z);
    // A slab three storeys up, on nothing: standable, with open air under.
    builder.set_floor(at(3), kit.slab);
    assert!(builder.column().standable(at(3)));
    assert!(builder.column().is_open(at(2)));
    assert_eq!(builder.column().height_at(at(0)), 3);
    // A wall is not stood on: its top is not solid.
    builder.set_block(at(5), kit.wall);
    assert!(!builder.column().standable(at(6)));
    assert_eq!(builder.column().height_at(at(0)), 5);
    // Solid rock is.
    builder.set_block(at(7), kit.rock);
    assert!(builder.column().standable(at(8)));
    assert_eq!(builder.column().height_at(at(0)), 8);
    // Taking things away exposes what is below, one at a time.
    builder.set_block(at(7), BlockState::NONE);
    assert_eq!(builder.column().height_at(at(0)), 5);
    builder.set_block(at(5), BlockState::NONE);
    assert_eq!(builder.column().height_at(at(0)), 3);
    builder.set_floor(at(3), FloorState::NONE);
    assert_eq!(builder.column().height_at(at(0)), 0);
    // Digging out the ground makes a hole.
    builder.set_block(at(-1), BlockState::NONE);
    assert!(!builder.column().standable(at(0)));
    assert!(builder.column().standable(at(-1)));
    assert_eq!(builder.column().height_at(at(0)), -1);
    // A neighbouring cell column is untouched.
    assert_eq!(builder.column().height_at(CellPos::new(4, 3, 0)), 0);
    check_caches(&kit.defs, builder.column()).unwrap();
}

#[test]
fn a_surface_inside_a_chunk_is_stored_and_one_at_its_base_is_not() {
    let kit = kit();
    let aligned = ColumnBuilder::new(&kit.defs, kit.rock, BlockState::NONE, 16).build();
    assert_eq!(aligned.stored_range(), None);
    assert_eq!(aligned.height(7), 16);
    let inside = ColumnBuilder::new(&kit.defs, kit.rock, BlockState::NONE, 3).build();
    assert_eq!(inside.stored_range(), Some((0, 0)));
    assert_eq!(inside.block(CellPos::new(0, 0, 2)), kit.rock);
    assert!(inside.block(CellPos::new(0, 0, 3)).is_none());
    assert_eq!(inside.height(7), 3);
    check_caches(&kit.defs, &inside).unwrap();
    // No ground at all: nothing to stand on anywhere.
    let void = ColumnBuilder::new(&kit.defs, BlockState::NONE, BlockState::NONE, 0).build();
    assert_eq!(void.height(0), HEIGHT_NONE);
    // A filled sky: no cell is under open sky.
    let buried = ColumnBuilder::new(&kit.defs, kit.rock, kit.rock, 0).build();
    assert_eq!(buried.height(0), HEIGHT_FULL);
}

#[test]
fn revisions_move_only_when_something_changes() {
    let kit = kit();
    let mut builder = ColumnBuilder::new(&kit.defs, kit.rock, BlockState::NONE, 0);
    let cell = CellPos::new(9, 9, 1);
    builder.set_block(cell, kit.wall);
    let chunk = |builder: &ColumnBuilder<'_>| builder.column().chunk(0).unwrap().rev();
    let first = chunk(&builder);
    assert!(builder.column().is_dirty());
    // The same write again changes nothing and bumps nothing.
    builder.set_block(cell, kit.wall);
    assert_eq!(chunk(&builder), first);
    builder.set_fixture(cell, kit.desk);
    let second = chunk(&builder);
    assert_ne!(second.structure, first.structure);
    assert_eq!(second.fields, first.fields);
    assert_eq!(second.occupants, first.occupants);
    // A finished column is clean.
    let column = builder.build();
    assert!(!column.is_dirty());
    assert_eq!(column.fixture(cell), kit.desk);
}

#[test]
fn a_column_hashes_by_what_it_holds_not_by_what_is_stored() {
    let kit = kit();
    let fresh = ColumnBuilder::new(&kit.defs, kit.rock, BlockState::NONE, 0).build();
    // The same cells after storage was created above and below and every
    // change undone.
    let mut builder = ColumnBuilder::new(&kit.defs, kit.rock, BlockState::NONE, 0);
    builder.set_block(CellPos::new(1, 2, 30), kit.wall);
    builder.set_block(CellPos::new(1, 2, -30), BlockState::NONE);
    let changed = Hash128::of(builder.column());
    builder.set_block(CellPos::new(1, 2, 30), BlockState::NONE);
    builder.set_block(CellPos::new(1, 2, -30), kit.rock);
    assert!(builder.column().stored_range().is_some());
    assert_eq!(Hash128::of(builder.column()), Hash128::of(&fresh));
    assert_ne!(changed, Hash128::of(&fresh));
    let undone = builder.build();
    assert_eq!(Hash128::of(&undone), Hash128::of(&fresh));

    // Where the ground ends is part of what a column holds.
    let higher = ColumnBuilder::new(&kit.defs, kit.rock, BlockState::NONE, 8).build();
    assert_ne!(Hash128::of(&higher), Hash128::of(&fresh));
    // The same content written in another order hashes the same.
    let mut one = ColumnBuilder::new(&kit.defs, kit.rock, BlockState::NONE, 0);
    one.set_floor(CellPos::new(0, 0, 0), kit.grass);
    one.set_block(CellPos::new(4, 4, 1), kit.window);
    let mut two = ColumnBuilder::new(&kit.defs, kit.rock, BlockState::NONE, 0);
    two.set_block(CellPos::new(4, 4, 1), kit.window);
    two.set_block(CellPos::new(6, 6, 1), kit.wall);
    two.set_block(CellPos::new(6, 6, 1), BlockState::NONE);
    two.set_floor(CellPos::new(0, 0, 0), kit.grass);
    assert_eq!(Hash128::of(one.column()), Hash128::of(two.column()));
}

/// One random edit of a column.
#[derive(Clone, Debug)]
struct Edit {
    x: i32,
    y: i32,
    z: i32,
    layer: u8,
    choice: u8,
}

fn edit() -> impl Strategy<Value = Edit> {
    // A few cell columns and a short range of storeys, so that edits pile
    // up on each other and cross chunk boundaries.
    (0..3_i32, 0..2_i32, -12..20_i32, 0..3_u8, 0..5_u8).prop_map(|(x, y, z, layer, choice)| Edit {
        x: x * 7,
        y: y * 30,
        z,
        layer,
        choice,
    })
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(48))]

    #[test]
    fn caches_equal_recomputation_after_random_edits(
        surface in -9..9_i32,
        solid_ground in any::<bool>(),
        edits in prop::collection::vec(edit(), 1..120),
    ) {
        let kit = kit();
        let ground = if solid_ground { kit.rock } else { BlockState::NONE };
        let blocks = [BlockState::NONE, kit.rock, kit.wall, kit.window, kit.door_open];
        let floors = [FloorState::NONE, kit.slab, kit.grass, FloorState::NONE, kit.slab];
        let fixtures = [FixtureState::NONE, kit.desk, kit.rubble, FixtureState::NONE, kit.desk];
        let mut builder = ColumnBuilder::new(&kit.defs, ground, BlockState::NONE, surface);
        check_caches(&kit.defs, builder.column())?;
        for (step, edit) in edits.iter().enumerate() {
            let cell = CellPos::new(edit.x, edit.y, edit.z);
            let choice = usize::from(edit.choice);
            match edit.layer {
                0 => builder.set_floor(cell, floors[choice]),
                1 => builder.set_block(cell, blocks[choice]),
                _ => builder.set_fixture(cell, fixtures[choice]),
            };
            // Checking after every edit would be slow: check now and then,
            // and always at the end.
            if step % 16 == 0 {
                check_caches(&kit.defs, builder.column())?;
            }
        }
        check_caches(&kit.defs, builder.column())?;
        let before = Hash128::of(builder.column());
        let column = builder.build();
        check_caches(&kit.defs, &column)?;
        prop_assert_eq!(Hash128::of(&column), before, "compaction does not change content");
    }
}
