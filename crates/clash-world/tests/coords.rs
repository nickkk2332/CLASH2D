//! Property tests for W1: conversions between scales for negative
//! coordinates, wrapping, the symmetry of `delta`, and segment traversal
//! against a brute-force reference.

use clash_core::{IVec3, Real};
use clash_world::dims::{
    CELL_M, CHUNK_AREA, CHUNK_XY, CHUNK_Z, REGION_TRACT_COUNT, REGION_TRACTS, STOREY_M,
};
use clash_world::iter::segment_cells;
use clash_world::{CellBox, CellPos, ChunkPos, Extent, RegionPos, Topology, TractPos};
use proptest::prelude::*;

fn cell() -> impl Strategy<Value = CellPos> {
    let span = -5_000_000..5_000_000;
    (span.clone(), span.clone(), span).prop_map(|(x, y, z)| CellPos::new(x, y, z))
}

fn extent() -> impl Strategy<Value = Extent> {
    prop_oneof![
        Just(Extent::Unbounded),
        (1..5000_i32).prop_map(Extent::Bounded),
        (1..5000_i32).prop_map(Extent::Wrapped),
    ]
}

fn topology() -> impl Strategy<Value = Topology> {
    (extent(), extent(), -300..0_i32, 0..300_i32)
        .prop_map(|(x, y, z_min, z_max)| Topology::new(x, y, z_min, z_max))
}

proptest! {
    #[test]
    fn a_cell_splits_into_chunk_and_index_and_back(cell in cell()) {
        let (chunk, idx) = cell.split();
        prop_assert_eq!(chunk.cell(idx), cell);
        // The chunk is the floor of the division, also below zero.
        prop_assert_eq!(chunk, ChunkPos::new(
            cell.x.div_euclid(CHUNK_XY),
            cell.y.div_euclid(CHUNK_XY),
            cell.z.div_euclid(CHUNK_Z),
        ));
        prop_assert_eq!(idx.lx(), cell.x.rem_euclid(CHUNK_XY));
        prop_assert_eq!(idx.ly(), cell.y.rem_euclid(CHUNK_XY));
        prop_assert_eq!(idx.lz(), cell.z.rem_euclid(CHUNK_Z));
        prop_assert_eq!(idx.column(), cell.column());
        prop_assert!(cell.column() < CHUNK_AREA);
        // The chunk's origin is its lowest cell.
        let origin = chunk.origin();
        prop_assert!(origin.x <= cell.x && cell.x < origin.x + CHUNK_XY);
        prop_assert!(origin.z <= cell.z && cell.z < origin.z + CHUNK_Z);
    }

    #[test]
    fn tracts_and_regions_nest(cell in cell()) {
        let tract = cell.tract();
        prop_assert_eq!(tract, cell.chunk().tract());
        prop_assert_eq!(tract, TractPos::new(cell.x.div_euclid(CHUNK_XY), cell.y.div_euclid(CHUNK_XY)));
        prop_assert_eq!(tract.cell(cell.column(), cell.z), cell);
        let region = cell.region();
        prop_assert_eq!(region, RegionPos::new(
            tract.x.div_euclid(REGION_TRACTS),
            tract.y.div_euclid(REGION_TRACTS),
        ));
        let index = tract.index_in_region();
        prop_assert!(index < REGION_TRACT_COUNT);
        prop_assert_eq!(region.tract(index), tract);
        prop_assert_eq!(tract.chunk(cell.chunk().z), cell.chunk());
    }

    #[test]
    fn cells_and_points_agree(cell in cell(), fx in 0.0..1.0_f64, fy in 0.0..1.0_f64, fz in 0.0..1.0_f64) {
        let point = [
            (Real::from(cell.x) + Real::of(fx)) * CELL_M,
            (Real::from(cell.y) + Real::of(fy)) * CELL_M,
            (Real::from(cell.z) + Real::of(fz)) * STOREY_M,
        ];
        prop_assert_eq!(CellPos::containing(point), cell);
        prop_assert_eq!(CellPos::containing(cell.centre_m()), cell);
    }

    #[test]
    fn normalising_wraps_or_names_an_edge(shape in topology(), cell in cell()) {
        match shape.normalize(cell) {
            Ok(inside) => {
                prop_assert_eq!(shape.normalize(inside), Ok(inside), "idempotent");
                prop_assert_eq!(inside.z, cell.z);
                for (extent, before, after) in [(shape.x, cell.x, inside.x), (shape.y, cell.y, inside.y)] {
                    match extent {
                        Extent::Unbounded => prop_assert_eq!(before, after),
                        Extent::Bounded(size) => {
                            prop_assert_eq!(before, after);
                            prop_assert!((0..size).contains(&after));
                        }
                        Extent::Wrapped(size) => {
                            prop_assert!((0..size).contains(&after));
                            prop_assert_eq!((i64::from(before) - i64::from(after)).rem_euclid(i64::from(size)), 0);
                        }
                    }
                }
            }
            Err(_) => {
                let x_out = matches!(shape.x, Extent::Bounded(size) if !(0..size).contains(&cell.x));
                let y_out = matches!(shape.y, Extent::Bounded(size) if !(0..size).contains(&cell.y));
                let z_out = cell.z < shape.z_min || cell.z > shape.z_max;
                prop_assert!(x_out || y_out || z_out);
            }
        }
    }

    #[test]
    fn delta_is_antisymmetric_and_shortest(shape in topology(), a in cell(), b in cell()) {
        let there = shape.delta(a, b);
        let back = shape.delta(b, a);
        prop_assert_eq!(there, -back);
        for (extent, step) in [(shape.x, there.x), (shape.y, there.y)] {
            if let Extent::Wrapped(size) = extent {
                prop_assert!(i64::from(step.abs()) * 2 <= i64::from(size));
            }
        }
        // Going by the delta arrives at the same place.
        let mut a_in = a;
        let mut b_in = b;
        a_in.z = 0;
        b_in.z = 0;
        if let (Ok(from), Ok(to)) = (shape.normalize(a_in), shape.normalize(b_in)) {
            let step = shape.delta(from, to);
            prop_assert_eq!(shape.normalize(from.offset(step)), Ok(to));
        }
        prop_assert_eq!(shape.delta(a, a), IVec3::ZERO);
    }
}

/// One end of a test segment: usually anywhere, sometimes exactly on the
/// half-metre lattice so that edges and corners are hit.
fn coordinate() -> impl Strategy<Value = f64> {
    prop_oneof![
        3 => -12.0..12.0_f64,
        1 => (-24..24_i32).prop_map(|half| f64::from(half) * 0.5),
    ]
}

fn point() -> impl Strategy<Value = [Real; 3]> {
    (coordinate(), coordinate(), coordinate())
        .prop_map(|(x, y, z)| [Real::of(x), Real::of(y), Real::of(z)])
}

/// The cells a segment passes through, found by testing every cell of the
/// bounding box: the interval of the segment inside the cell's slab on
/// each axis, intersected. A cell counts when the interval has length, or
/// when it holds an end of the segment.
fn brute_force(a: [Real; 3], b: [Real; 3]) -> Vec<CellPos> {
    let start = CellPos::containing(a);
    let end = CellPos::containing(b);
    let sizes = [CELL_M, CELL_M, STOREY_M];
    let dir = [b[0] - a[0], b[1] - a[1], b[2] - a[2]];
    let mut found: Vec<(Real, Real, CellPos)> = Vec::new();
    for cell in CellBox::new(start, end).cells() {
        let coords = [cell.x, cell.y, cell.z];
        let lows = [start.x, start.y, start.z];
        let mut enter = Real::ZERO;
        let mut leave = Real::ONE;
        let mut inside = true;
        for axis in 0..3 {
            if dir[axis] == Real::ZERO || lows[axis] == [end.x, end.y, end.z][axis] {
                // The segment does not change cell along this axis.
                inside &= coords[axis] == lows[axis];
                continue;
            }
            let low = (Real::from(coords[axis]) * sizes[axis] - a[axis]) / dir[axis];
            let high = (Real::from(coords[axis] + 1) * sizes[axis] - a[axis]) / dir[axis];
            let (first, second) = if dir[axis] > Real::ZERO {
                (low, high)
            } else {
                (high, low)
            };
            // The first and last cells along an axis are open-ended: the
            // segment starts and stops inside them.
            if coords[axis] != lows[axis] {
                enter = enter.max(first);
            }
            if coords[axis] != [end.x, end.y, end.z][axis] {
                leave = leave.min(second);
            }
        }
        if !inside {
            continue;
        }
        if enter < leave || cell == start || cell == end {
            found.push((enter, leave, cell));
        }
    }
    found.sort_by(|p, q| {
        let rank = |cell: CellPos| i32::from(cell == end) - i32::from(cell == start);
        (p.0, p.1, rank(p.2)).cmp(&(q.0, q.1, rank(q.2)))
    });
    found.into_iter().map(|(_, _, cell)| cell).collect()
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(2000))]

    #[test]
    fn a_segment_visits_exactly_the_cells_brute_force_finds(a in point(), b in point()) {
        let walked: Vec<CellPos> = segment_cells(a, b).collect();
        let expected = brute_force(a, b);
        prop_assert_eq!(&walked, &expected);
        // The walk starts and ends where the segment does, never repeats a
        // cell, and each step moves at most one cell along each axis.
        prop_assert_eq!(walked.first().copied(), Some(CellPos::containing(a)));
        prop_assert_eq!(walked.last().copied(), Some(CellPos::containing(b)));
        for pair in walked.windows(2) {
            let step = pair[1] - pair[0];
            prop_assert!(step != IVec3::ZERO);
            prop_assert!(step.x.abs() <= 1 && step.y.abs() <= 1 && step.z.abs() <= 1);
        }
    }
}
