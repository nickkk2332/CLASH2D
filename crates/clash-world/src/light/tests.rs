//! Tests of light that reach into the cache: what it drops when it is
//! full, and the bounds it keeps (15 §3.3, S1).

use std::sync::Arc;

use clash_core::{Dur, IVec3, Real};
use clash_data::Time;
use clash_time::Timeline;

use super::cache::{BLOCK_CENTRE, BLOCK_CHUNKS, LIGHT_SOURCES_MAX_PER_CHUNK, block_offset};
use super::{LightSpec, SkyDef, SkyKeySpec, SkyTime};
use crate::cause::Cause;
use crate::cosmos::Cosmos;
use crate::defs::{BlockState, FloorState, RawWorldDefs, WorldDefs};
use crate::edit::WorldCtx;
use crate::event::EventSink;
use crate::grid::ColumnBuilder;
use crate::pos::{CellBox, CellPos, GridId, Location, TractPos};
use crate::test_support::load_column;

/// Reads one definition from JSON.
fn def<T: serde::de::DeserializeOwned>(json: &str) -> T {
    serde_json::from_str(json).unwrap_or_else(|error| panic!("{json}: {error}"))
}

struct World {
    cosmos: Cosmos,
    timeline: Timeline,
    grid: GridId,
    wall: BlockState,
    ember: FloorState,
}

const NOON: SkyTime = SkyTime::at(Dur::ZERO);

impl World {
    fn set_block(&mut self, cell: CellPos, state: BlockState) {
        let mut events = EventSink::new();
        let mut ctx = WorldCtx::new(&mut self.cosmos, &mut self.timeline, &mut events);
        let loc = Location::new(self.grid, cell);
        ctx.edit(Cause::DEBUG)
            .set_block(loc, state)
            .unwrap_or_else(|error| panic!("{error}"));
    }

    fn set_floor(&mut self, cell: CellPos, state: FloorState) {
        let mut events = EventSink::new();
        let mut ctx = WorldCtx::new(&mut self.cosmos, &mut self.timeline, &mut events);
        let loc = Location::new(self.grid, cell);
        ctx.edit(Cause::DEBUG)
            .set_floor(loc, state)
            .unwrap_or_else(|error| panic!("{error}"));
    }

    /// The light of the ground storey of both tracts, from this cosmos
    /// and from a copy of it that has worked nothing out.
    fn kept_and_afresh(&self) -> (Vec<Real>, Vec<Real>) {
        let cells = CellBox::new(CellPos::new(0, 0, 0), CellPos::new(63, 31, 0));
        let fill = |cosmos: &Cosmos| {
            let mut out = Vec::new();
            cosmos
                .fill_light(self.grid, cells, NOON, &[], &mut out)
                .unwrap_or_else(|error| panic!("{error}"));
            out
        };
        (fill(&self.cosmos), fill(&self.cosmos.clone()))
    }
}

/// Two tracts of floor on rock under a sky that never changes, with a
/// room that has one window.
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
    let mut ember: crate::defs::FloorDef = def(r#"{ "material": "stone", "thickness": 0.1 }"#);
    ember.light = Some(LightSpec {
        intensity: Real::of(0.5),
        radius: Some(crate::defs::Length(Real::of(2.0))),
    });
    raw.floors.push(("t:ember".into(), ember));
    raw.skies.push((
        "t:sun".into(),
        SkyDef {
            light: vec![SkyKeySpec {
                at: Time(Dur::ZERO),
                level: Real::ONE,
            }],
            ..SkyDef::default()
        },
    ));
    raw.realms.push((
        "t:land".into(),
        def(r#"{ "size_regions": [1, 1], "z_range": [-60, 60],
                 "strata": [ { "from_z": -60, "gravity": 9.81, "sky": "sun" } ] }"#),
    ));
    let defs = Arc::new(WorldDefs::build(raw).unwrap_or_else(|errors| panic!("{errors:?}")));
    let state = |text: &str| {
        defs.block_state(text, None)
            .unwrap_or_else(|error| panic!("{error}"))
    };
    let floor = |text: &str| {
        defs.floor_state(text, None)
            .unwrap_or_else(|error| panic!("{error}"))
    };
    let (rock, wall, slab) = (state("rock"), state("wall"), floor("slab"));
    let mut timeline = Timeline::new();
    let mut cosmos = Cosmos::new(Arc::clone(&defs));
    let land = defs.realms().lookup("t:land");
    let land = land.unwrap_or_else(|| panic!("no realm"));
    let realm = cosmos
        .create_realm(&mut timeline, land, 1)
        .unwrap_or_else(|error| panic!("{error}"));
    let grid = cosmos.realm(realm).map(|realm| realm.base_grid);
    let grid = grid.unwrap_or_else(|| panic!("no grid"));
    for tract in [TractPos::new(0, 0), TractPos::new(1, 0)] {
        let mut builder = ColumnBuilder::new(&defs, rock, BlockState::NONE, 0);
        builder.fill_floor(0, slab);
        if tract.x == 0 {
            // A room from (8, 8) to (20, 16) with a window at (8, 12).
            for y in 8..=16 {
                for x in 8..=20 {
                    let edge = x == 8 || x == 20 || y == 8 || y == 16;
                    if edge && (x, y) != (8, 12) {
                        builder.set_block(CellPos::new(x, y, 0), wall);
                    }
                    builder.set_floor(CellPos::new(x, y, 1), slab);
                }
            }
        }
        load_column(&mut cosmos, grid, tract, builder.build())
            .unwrap_or_else(|error| panic!("{error}"));
    }
    World {
        cosmos,
        timeline,
        grid,
        wall,
        ember: floor("ember"),
    }
}

#[test]
fn the_block_of_a_chunk_runs_storey_by_storey_and_row_by_row() {
    assert_eq!(block_offset(0), IVec3::new(-1, -1, -1));
    assert_eq!(block_offset(1), IVec3::new(0, -1, -1));
    assert_eq!(block_offset(3), IVec3::new(-1, 0, -1));
    assert_eq!(block_offset(BLOCK_CENTRE), IVec3::ZERO);
    assert_eq!(block_offset(BLOCK_CHUNKS - 1), IVec3::new(1, 1, 1));
}

#[test]
fn a_cache_that_has_dropped_things_still_answers_rightly() {
    let mut world = world();
    let (kept, afresh) = world.kept_and_afresh();
    assert_eq!(kept, afresh);
    let inside = Location::new(world.grid, CellPos::new(10, 12, 0));
    let lit = world.cosmos.light(inside, NOON);
    assert!(
        lit.as_ref()
            .is_ok_and(|level| *level > Real::ZERO && *level < Real::ONE)
    );

    // Dropped at once: everything was asked about just now, so the
    // answers stay and every shape is read again at the next look.
    world.cosmos.light.get_mut().trim();
    assert_eq!(world.cosmos.light_stats().cached_chunks, 2);
    let before = world.cosmos.light_stats().chunks_lit;
    let (kept, afresh) = world.kept_and_afresh();
    assert_eq!(kept, afresh);
    assert_eq!(world.cosmos.light_stats().chunks_lit, before);
    // An edit after that is still seen.
    world.set_block(CellPos::new(8, 12, 0), world.wall);
    let (kept, afresh) = world.kept_and_afresh();
    assert_eq!(kept, afresh);
    assert_eq!(world.cosmos.light(inside, NOON), Ok(Real::ZERO));

    // Dropped after going unasked for a while: nothing is kept, and
    // what is asked next is worked out from the cells as they are.
    world.set_block(CellPos::new(8, 12, 0), BlockState::NONE);
    for _ in 0..8 {
        world.cosmos.light.get_mut().begin();
    }
    world.cosmos.light.get_mut().trim();
    assert_eq!(world.cosmos.light_stats().cached_chunks, 0);
    let (kept, afresh) = world.kept_and_afresh();
    assert_eq!(kept, afresh);
    assert_eq!(world.cosmos.light(inside, NOON), lit);

    // Emptied outright, the same.
    world.cosmos.clear_light();
    assert_eq!(world.cosmos.light_stats().cached_chunks, 0);
    let (kept, afresh) = world.kept_and_afresh();
    assert_eq!(kept, afresh);
}

#[test]
fn a_chunk_counts_only_so_many_sources() {
    let mut world = world();
    // More glowing floor in the second tract than a chunk counts: rows
    // of twenty, in cell order.
    let rows = LIGHT_SOURCES_MAX_PER_CHUNK as i32 / 20 + 2;
    for y in 0..rows {
        for x in 0..20 {
            world.set_floor(CellPos::new(40 + x, 2 + y, 0), world.ember);
        }
    }
    let fixed = |world: &World, x: i32, y: i32| {
        world
            .cosmos
            .static_light(Location::new(world.grid, CellPos::new(x, y, 0)))
            .map(|light| light.fixed)
    };
    // The first rows glow and the last, past the count, do not.
    assert!(fixed(&world, 45, 2).is_ok_and(|light| light > Real::of(0.5)));
    assert_eq!(fixed(&world, 45, 2 + rows + 1), Ok(Real::ZERO));
    let (kept, afresh) = world.kept_and_afresh();
    assert_eq!(kept, afresh);
}
