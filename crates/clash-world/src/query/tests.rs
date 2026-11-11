//! Tests of the queries that need to edit cells already in the world,
//! which only this crate can do until `WorldEdit` exists: that an answer
//! that is kept goes stale after an edit, and after nothing else (02
//! §22, W18).

use std::sync::Arc;

use clash_core::{Real, Tick};
use clash_time::Timeline;

use super::{FovSet, Freshness, MoveProfile, PathLimits, RevParts, RevStamp};
use crate::cause::{Cause, CauseKind};
use crate::chunk::{Decal, DecalId};
use crate::cosmos::Cosmos;
use crate::defs::{BlockState, FloorState, RawWorldDefs, WorldDefs};
use crate::dims::CHUNK_Z_BITS;
use crate::grid::ColumnBuilder;
use crate::pos::{CellBox, CellPos, ChunkPos, GridId, Location, TractPos};
use crate::test_support::{load_column, unload_column};

/// Reads one definition from JSON.
fn def<T: serde::de::DeserializeOwned>(json: &str) -> T {
    serde_json::from_str(json).unwrap_or_else(|error| panic!("{json}: {error}"))
}

struct World {
    cosmos: Cosmos,
    grid: GridId,
    wall: BlockState,
}

impl World {
    fn loc(&self, x: i32, y: i32, z: i32) -> Location {
        Location::new(self.grid, CellPos::new(x, y, z))
    }

    /// Sets a block the way `WorldEdit` will: through the column.
    fn set_block(&mut self, cell: CellPos, state: BlockState) {
        let defs = Arc::clone(self.cosmos.defs());
        let column = self.cosmos.column_mut(self.grid, cell.tract()).unwrap();
        column.set_block(&defs, cell, state);
    }

    /// Writes to a cell without changing what stands in it: damage and
    /// a mark.
    fn scuff(&mut self, cell: CellPos) {
        let defs = Arc::clone(self.cosmos.defs());
        let column = self.cosmos.column_mut(self.grid, cell.tract()).unwrap();
        let chunk = column.chunk_mut(&defs, cell.z >> CHUNK_Z_BITS).unwrap();
        chunk.set_damage(cell.idx(), [0, 40, 0]);
        chunk.add_decal(
            cell.idx(),
            Decal {
                kind: DecalId::new(1),
                amount: 2,
                cause: Cause::new(CauseKind::Debug),
                tick: Tick(9),
            },
        );
    }

    fn view(&self, origin: Location, set: &mut FovSet) -> Freshness {
        self.cosmos
            .field_of_view(origin, 1500, Real::of(8.0), set)
            .unwrap()
    }
}

/// Three tracts of floor on rock: two side by side and one far off.
fn world() -> World {
    let mut raw = RawWorldDefs::default();
    raw.materials.push((
        "t:stone".into(),
        def(r#"{ "density": 2400, "toughness": 1.0e6 }"#),
    ));
    let block = r#"{ "material": "stone", "height": 3.0, "fill": FILL, "flags": ["opaque"] }"#;
    raw.blocks
        .push(("t:rock".into(), def(&block.replace("FILL", "1.0"))));
    raw.blocks
        .push(("t:wall".into(), def(&block.replace("FILL", "0.2"))));
    raw.floors.push((
        "t:slab".into(),
        def(r#"{ "material": "stone", "thickness": 0.2 }"#),
    ));
    raw.realms.push((
        "t:land".into(),
        def(r#"{ "size_regions": [1, 1], "z_range": [-60, 60],
                 "strata": [ { "from_z": -60, "gravity": 9.81, "atmosphere": null } ] }"#),
    ));
    let defs = Arc::new(WorldDefs::build(raw).unwrap_or_else(|errors| panic!("{errors:?}")));
    let rock = defs.block_state("rock", None).unwrap();
    let wall = defs.block_state("wall", None).unwrap();
    let slab = defs.floor_state("slab", None).unwrap();
    let mut timeline = Timeline::new();
    let mut cosmos = Cosmos::new(Arc::clone(&defs));
    let land = defs.realms().lookup("t:land").unwrap();
    let realm = cosmos.create_realm(&mut timeline, land, 1).unwrap();
    let grid = cosmos.realm(realm).unwrap().base_grid;
    for tract in [
        TractPos::new(0, 0),
        TractPos::new(1, 0),
        TractPos::new(5, 5),
    ] {
        let mut builder = ColumnBuilder::new(&defs, rock, BlockState::NONE, 0);
        builder.fill_floor(0, slab);
        load_column(&mut cosmos, grid, tract, builder.build()).unwrap();
    }
    World { cosmos, grid, wall }
}

#[test]
fn a_view_is_reused_until_a_cell_it_was_read_from_is_edited() {
    let mut world = world();
    let origin = world.loc(10, 10, 0);
    let behind = world.loc(14, 10, 0);
    let mut set = FovSet::new();
    assert!(
        !set.is_current(&world.cosmos),
        "an empty view answers nothing"
    );
    assert_eq!(world.view(origin, &mut set), Freshness::Computed);
    assert_eq!(world.view(origin, &mut set), Freshness::Reused);
    assert!(set.is_current(&world.cosmos) && set.contains(behind));
    let seen = set.len();

    // Nothing but an edit makes it stale. A write that changes nothing:
    world.set_block(CellPos::new(12, 10, 0), BlockState::NONE);
    // a write that is not to the structure:
    world.scuff(CellPos::new(11, 10, 0));
    // an edit in another tract, and one beside the view's reach:
    world.set_block(CellPos::new(170, 170, 0), world.wall);
    world.set_block(CellPos::new(50, 10, 0), world.wall);
    // edits in the same tract far above and far below what was read:
    world.set_block(CellPos::new(10, 10, 40), world.wall);
    world.set_block(CellPos::new(10, 10, -30), BlockState::NONE);
    assert!(set.is_current(&world.cosmos));
    assert_eq!(world.view(origin, &mut set), Freshness::Reused);
    assert_eq!(set.len(), seen);

    // An edit to a cell it was read from does.
    world.set_block(CellPos::new(12, 10, 0), world.wall);
    assert!(!set.is_current(&world.cosmos));
    assert_eq!(world.view(origin, &mut set), Freshness::Computed);
    assert!(!set.contains(behind) && set.len() < seen);
    assert_eq!(world.view(origin, &mut set), Freshness::Reused);
    // Taking the wall away again is another edit.
    world.set_block(CellPos::new(12, 10, 0), BlockState::NONE);
    assert_eq!(world.view(origin, &mut set), Freshness::Computed);
    assert_eq!(set.len(), seen);

    // Another question is another answer, and so is the first again.
    assert_eq!(
        world.view(world.loc(11, 10, 0), &mut set),
        Freshness::Computed
    );
    assert_eq!(world.view(origin, &mut set), Freshness::Computed);
    let lower = world
        .cosmos
        .field_of_view(origin, 400, Real::of(8.0), &mut set);
    assert_eq!(lower, Ok(Freshness::Computed));
    let wider = world
        .cosmos
        .field_of_view(origin, 400, Real::of(9.0), &mut set);
    assert_eq!(wider, Ok(Freshness::Computed));

    // A view that is emptied is worked out afresh.
    assert_eq!(world.view(origin, &mut set), Freshness::Computed);
    set.clear();
    assert_eq!(world.view(origin, &mut set), Freshness::Computed);
}

#[test]
fn a_view_goes_stale_when_a_tract_it_reached_is_unloaded_or_loaded() {
    let mut world = world();
    let origin = world.loc(28, 10, 0);
    let next_door = world.loc(34, 10, 0);
    let mut set = FovSet::new();
    assert_eq!(world.view(origin, &mut set), Freshness::Computed);
    assert!(set.contains(next_door));

    let column = unload_column(&mut world.cosmos, world.grid, TractPos::new(1, 0)).unwrap();
    assert!(!set.is_current(&world.cosmos));
    assert_eq!(world.view(origin, &mut set), Freshness::Computed);
    assert!(!set.contains(next_door));
    assert_eq!(world.view(origin, &mut set), Freshness::Reused);

    load_column(&mut world.cosmos, world.grid, TractPos::new(1, 0), column).unwrap();
    assert_eq!(world.view(origin, &mut set), Freshness::Computed);
    assert!(set.contains(next_door));
    // The far tract was never part of it.
    unload_column(&mut world.cosmos, world.grid, TractPos::new(5, 5)).unwrap();
    assert_eq!(world.view(origin, &mut set), Freshness::Reused);
}

#[test]
fn a_stamp_moves_with_the_structure_of_its_chunks_and_with_nothing_else() {
    let mut world = world();
    let view = world.cosmos.grid(world.grid).unwrap();
    let mut stamp = RevStamp::new(RevParts::STRUCTURE);
    assert!(stamp.is_empty() && stamp.is_current(&world.cosmos));
    // A box over the border of two tracts and of two chunks in height.
    stamp.record_box(
        &view,
        CellBox::new(CellPos::new(30, 4, -2), CellPos::new(33, 6, 1)),
    );
    // A cell of a chunk it already holds adds nothing; a tract that is
    // not loaded is held as absent.
    stamp.record_cell(&view, CellPos::new(31, 5, 0));
    stamp.record_cell(&view, CellPos::new(70, 5, 0));
    let chunks: Vec<ChunkPos> = stamp.chunks().collect();
    assert_eq!(
        chunks,
        [
            ChunkPos::new(0, 0, -1),
            ChunkPos::new(1, 0, -1),
            ChunkPos::new(0, 0, 0),
            ChunkPos::new(1, 0, 0),
            ChunkPos::new(2, 0, 0),
        ]
    );
    assert_eq!(stamp.parts(), RevParts::STRUCTURE);
    assert!(stamp.is_current(&world.cosmos));
    let before = stamp.clone();

    world.scuff(CellPos::new(31, 5, 0));
    world.set_block(CellPos::new(31, 5, 0), BlockState::NONE);
    world.set_block(CellPos::new(31, 5, 20), world.wall);
    assert!(stamp.is_current(&world.cosmos));

    // An edit below the surface, in the lower chunk.
    world.set_block(CellPos::new(31, 5, -1), BlockState::NONE);
    assert!(!stamp.is_current(&world.cosmos));
    // Putting the rock back is a second edit, not a return.
    let rock = world.cosmos.cell(world.loc(31, 6, -1)).unwrap().block;
    world.set_block(CellPos::new(31, 5, -1), rock);
    assert!(!stamp.is_current(&world.cosmos));

    // The same chunks stamped now are current, and differ from before.
    let view = world.cosmos.grid(world.grid).unwrap();
    let mut after = RevStamp::new(RevParts::STRUCTURE);
    for chunk in before.chunks() {
        after.record_cell(&view, chunk.origin());
    }
    assert!(after.is_current(&world.cosmos));
    assert_ne!(after, before);
    // A tract that appears where the stamp saw none makes it stale.
    let column = unload_column(&mut world.cosmos, world.grid, TractPos::new(5, 5)).unwrap();
    load_column(&mut world.cosmos, world.grid, TractPos::new(2, 0), column).unwrap();
    assert!(!after.is_current(&world.cosmos));
    // A stamp cleared holds nothing and is current again.
    after.clear();
    assert!(after.is_empty() && after.is_current(&world.cosmos));
    // Floors are structure too.
    let view = world.cosmos.grid(world.grid).unwrap();
    after.record_cell(&view, CellPos::new(3, 3, 0));
    let defs = Arc::clone(world.cosmos.defs());
    let column = world
        .cosmos
        .column_mut(world.grid, TractPos::new(0, 0))
        .unwrap();
    column.set_floor(&defs, CellPos::new(3, 3, 0), FloorState::NONE);
    assert!(!after.is_current(&world.cosmos));
}

#[test]
fn a_path_is_current_until_a_chunk_it_passes_is_edited() {
    let mut world = world();
    let walker = MoveProfile::walker();
    let from = world.loc(5, 5, 0);
    let to = world.loc(20, 5, 0);
    let find = |world: &World| {
        world
            .cosmos
            .find_path(from, to, &walker, PathLimits::DEFAULT)
            .unwrap()
    };
    let path = find(&world);
    assert_eq!(path.cost_m(), Real::of(15.0));
    assert!(path.is_current(&world.cosmos));
    assert!(
        path.stamp()
            .chunks()
            .eq([ChunkPos::new(0, 0, -1), ChunkPos::new(0, 0, 0)])
    );

    // Edits elsewhere, and writes that are not edits, leave it be.
    world.set_block(CellPos::new(40, 5, 0), world.wall);
    world.set_block(CellPos::new(10, 5, 30), world.wall);
    world.scuff(CellPos::new(10, 5, 0));
    assert!(path.is_current(&world.cosmos));
    assert_eq!(find(&world), path);

    // A wall across it does not.
    world.set_block(CellPos::new(10, 5, 0), world.wall);
    assert!(!path.is_current(&world.cosmos));
    let round = find(&world);
    assert!(round.is_current(&world.cosmos));
    assert!(round.cost_m() > path.cost_m());
    assert!(!round.cells().contains(&CellPos::new(10, 5, 0)));
}
