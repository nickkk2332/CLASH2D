//! The faster lines of sight against the first ones (D-732): each walk
//! of [`sight_clear`] keeps the column it last read, and
//! [`Cosmos::sight_to_heights`] walks once for several heights. Both
//! must answer exactly as the reference, kept here, does. And a view's
//! cells on some storeys alone, against all its cells filtered.

use std::sync::Arc;

use clash_core::Real;
use clash_time::Timeline;
use proptest::prelude::*;

use super::locate;
use super::ray::{RAY_MAX_M, sight_clear};
use crate::cosmos::{Cosmos, GridView};
use crate::defs::{BlockState, CellFlags, EngineFlag, RawWorldDefs, StateInfo, WorldDefs};
use crate::dims::{CELL_M, STOREY_M};
use crate::error::WorldError;
use crate::grid::ColumnBuilder;
use crate::iter::segment_cells;
use crate::pos::{CellPos, GridId, Location, TractPos};
use crate::test_support::load_column;
use crate::topology::{Extent, Topology};

/// [`sight_clear`] as it was first written, a column found afresh for
/// every cell.
fn sight_clear_reference(
    view: &GridView<'_>,
    defs: &WorldDefs,
    a: [Real; 3],
    b: [Real; 3],
) -> Result<bool, WorldError> {
    let key = |p: [Real; 3]| (p[2], p[1], p[0]);
    let (a, b) = if key(a) <= key(b) { (a, b) } else { (b, a) };
    let b = nearest_image(view.topology(), a, b);
    let first = CellPos::containing(a);
    let last = CellPos::containing(b);
    let offset = [b[0] - a[0], b[1] - a[1], b[2] - a[2]];
    let length = (offset[0] * offset[0] + offset[1] * offset[1] + offset[2] * offset[2]).sqrt();
    if length > RAY_MAX_M {
        return Ok(false);
    }
    let dir = if length > Real::ZERO {
        Some(offset.map(|part| part / length))
    } else {
        None
    };
    let mut prev: Option<(i32, CellFlags)> = None;
    for cell in segment_cells(a, b) {
        let (column, norm) = locate(view, cell)?;
        let flags = column.flags(norm);
        if let Some((prev_z, prev_flags)) = prev
            && cell.z != prev_z
        {
            let upper = if cell.z > prev_z { flags } else { prev_flags };
            if upper.contains(CellFlags::HAS_FLOOR) {
                return Ok(false);
            }
        }
        if flags.contains(CellFlags::OPAQUE)
            && cell != first
            && cell != last
            && let Some(dir) = dir
        {
            let lowest = lowest_in(a, dir, length, cell);
            let block = defs.block_info(column.block(norm));
            let fixture = defs.fixture_info(column.fixture(norm));
            let hides =
                |info: &StateInfo| info.flags.has(EngineFlag::Opaque) && lowest < info.height;
            if hides(block) || hides(fixture) {
                return Ok(false);
            }
        }
        prev = Some((cell.z, flags));
    }
    Ok(true)
}

/// The lowest a line gets inside a cell, above the cell's base: the
/// first form of a ray's span.
fn lowest_in(origin: [Real; 3], dir: [Real; 3], max_m: Real, cell: CellPos) -> Real {
    let sizes = [CELL_M, CELL_M, STOREY_M];
    let coords = [cell.x, cell.y, cell.z];
    let mut enter = Real::ZERO;
    let mut exit = max_m.min(RAY_MAX_M);
    for axis in 0..3 {
        if dir[axis] == Real::ZERO {
            continue;
        }
        let low = Real::from(coords[axis]) * sizes[axis];
        let high = low + sizes[axis];
        let to_low = (low - origin[axis]) / dir[axis];
        let to_high = (high - origin[axis]) / dir[axis];
        enter = enter.max(to_low.min(to_high));
        exit = exit.min(to_low.max(to_high));
    }
    let exit = exit.max(enter);
    let base = Real::from(cell.z) * STOREY_M;
    let alt = |dist: Real| (origin[2] + dir[2] * dist - base).clamp(Real::ZERO, STOREY_M);
    alt(enter).min(alt(exit))
}

fn nearest_image(topology: &Topology, a: [Real; 3], mut b: [Real; 3]) -> [Real; 3] {
    for (axis, extent) in [topology.x, topology.y].into_iter().enumerate() {
        if let Extent::Wrapped(size) = extent
            && size > 0
        {
            let round = Real::from(size) * CELL_M;
            let turns = ((b[axis] - a[axis]) / round).round();
            b[axis] -= turns * round;
        }
    }
    b
}

fn def<T: serde::de::DeserializeOwned>(json: &str) -> T {
    serde_json::from_str(json).unwrap_or_else(|error| panic!("{json}: {error}"))
}

/// Blocks of four heights that hide, one that does not, and a floor.
struct Kit {
    defs: Arc<WorldDefs>,
    blocks: Vec<BlockState>,
}

fn kit() -> Kit {
    let mut raw = RawWorldDefs::default();
    raw.materials.push((
        "t:stone".into(),
        def(r#"{ "density": 2400, "toughness": 1.0e6 }"#),
    ));
    for (name, height, flags) in [
        ("rock", 3.0, r#"["opaque"]"#),
        ("low", 0.5, r#"["opaque"]"#),
        ("waist", 1.1, r#"["opaque"]"#),
        ("high", 2.0, r#"["opaque"]"#),
        ("glass", 3.0, "[]"),
    ] {
        let json = format!(
            r#"{{ "material": "stone", "height": {height}, "fill": 0.3, "flags": {flags} }}"#
        );
        raw.blocks.push((format!("t:{name}"), def(&json)));
    }
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
    let blocks = ["rock", "low", "waist", "high", "glass"]
        .iter()
        .map(|name| defs.block_state(name, None).unwrap())
        .collect();
    Kit { defs, blocks }
}

/// Two tracts side by side, things standing about on two storeys and a
/// floor over some cells of the upper. Cells east of the second tract
/// are not loaded.
fn scene(kit: &Kit, things: &[(i32, i32, i32, usize)], floors: &[(i32, i32)]) -> (Cosmos, GridId) {
    let rock = kit.blocks[0];
    let slab = kit.defs.floor_state("slab", None).unwrap();
    let mut timeline = Timeline::new();
    let mut cosmos = Cosmos::new(Arc::clone(&kit.defs));
    let land = kit.defs.realms().lookup("t:land").unwrap();
    let realm = cosmos.create_realm(&mut timeline, land, 1).unwrap();
    let grid = cosmos.realm(realm).unwrap().base_grid;
    for tract in [TractPos::new(0, 0), TractPos::new(1, 0)] {
        let mut builder = ColumnBuilder::new(&kit.defs, rock, BlockState::NONE, 0);
        builder.fill_floor(0, slab);
        for (x, y, z, which) in things {
            let cell = CellPos::new(*x, *y, *z);
            if cell.tract() == tract {
                builder.set_block(cell, kit.blocks[*which % kit.blocks.len()]);
            }
        }
        for (x, y) in floors {
            let cell = CellPos::new(*x, *y, 1);
            if cell.tract() == tract {
                builder.set_floor(cell, slab);
            }
        }
        load_column(&mut cosmos, grid, tract, builder.build()).unwrap();
    }
    (cosmos, grid)
}

fn spot() -> impl Strategy<Value = (i32, i32, i32, u16)> {
    (0_i32..72, 0_i32..24, 0_i32..2, 0_u16..3000)
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(64))]

    /// Lines of sight, one at a time and several heights at once,
    /// answer as the first lines of sight did, on random scenes of two
    /// storeys with unloaded space beside them.
    #[test]
    fn faster_lines_of_sight_answer_as_the_first_ones(
        things in prop::collection::vec((0_i32..72, 0_i32..24, 0_i32..2, 0_usize..5), 0..160),
        floors in prop::collection::vec((0_i32..72, 0_i32..24), 0..80),
        eyes in prop::collection::vec(spot(), 6),
        targets in prop::collection::vec((spot(), prop::collection::vec(0_u16..3000, 1..6)), 6),
    ) {
        let kit = kit();
        let (cosmos, grid) = scene(&kit, &things, &floors);
        let view = cosmos.grid(grid).unwrap();
        let defs = cosmos.defs();
        for (ex, ey, ez, eye_alt) in &eyes {
            let eye = Location::new(grid, CellPos::new(*ex, *ey, *ez));
            for ((tx, ty, tz, _), alts) in &targets {
                let target = Location::new(grid, CellPos::new(*tx, *ty, *tz));
                let mut expected = 0_u32;
                for (index, alt) in alts.iter().enumerate() {
                    let a = eye.point_at(*eye_alt).pos;
                    let b = target.point_at(*alt).pos;
                    let first = sight_clear_reference(&view, defs, a, b);
                    prop_assert_eq!(&sight_clear(&view, defs, a, b), &first);
                    prop_assert_eq!(&sight_clear(&view, defs, b, a), &first);
                    if first == Ok(true) {
                        expected |= 1 << index;
                    }
                }
                prop_assert_eq!(cosmos.sight_to_heights(eye, *eye_alt, target, alts), expected);
            }
        }
    }
}

#[test]
fn heights_over_another_grid_or_none_are_not_seen() {
    let kit = kit();
    let (cosmos, grid) = scene(&kit, &[], &[]);
    let eye = Location::new(grid, CellPos::new(2, 2, 0));
    let target = Location::new(grid, CellPos::new(9, 2, 0));
    assert_eq!(
        cosmos.sight_to_heights(eye, 1500, target, &[200, 900]),
        0b11
    );
    assert_eq!(cosmos.sight_to_heights(eye, 1500, target, &[]), 0);
    let elsewhere = Location::new(GridId::new(7), target.cell);
    assert_eq!(cosmos.sight_to_heights(eye, 1500, elsewhere, &[200]), 0);
    // Past the most that is asked about, nothing is seen.
    let many = [500_u16; 40];
    assert_eq!(cosmos.sight_to_heights(eye, 1500, target, &many), u32::MAX);
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(32))]

    /// A view's cells on some storeys alone are its cells filtered to
    /// those storeys, in the same order: what a party member's sweep
    /// walks instead of every storey of open sky.
    #[test]
    fn a_views_cells_on_some_storeys_are_its_cells_filtered(
        things in prop::collection::vec((0_i32..72, 0_i32..24, 0_i32..2, 0_usize..5), 0..120),
        floors in prop::collection::vec((0_i32..72, 0_i32..24), 0..60),
        eye in spot(),
        radius in prop::sample::select(vec![4.0_f64, 9.5, 20.0]),
        low in -8_i32..4,
        span in 0_i32..8,
    ) {
        let kit = kit();
        let (cosmos, grid) = scene(&kit, &things, &floors);
        let (x, y, z, alt) = eye;
        let mut view = super::FovSet::new();
        let origin = Location::new(grid, CellPos::new(x.min(63), y, z));
        if cosmos.field_of_view(origin, alt, Real::of(radius), &mut view).is_err() {
            return Ok(());
        }
        let high = low + span;
        let all: Vec<_> = view
            .offsets()
            .filter(|offset| (low..=high).contains(&offset.z))
            .collect();
        let some: Vec<_> = view.offsets_between(low, high).collect();
        prop_assert_eq!(some, all);
        prop_assert_eq!(view.offsets_between(high + 1, low).count(), 0);
    }
}
