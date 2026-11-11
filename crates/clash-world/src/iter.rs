//! Ways of walking cells, each in a defined order (02 §3.3).
//!
//! | Need | Use |
//! |------|-----|
//! | Every cell of a box | [`CellBox::cells`](crate::CellBox::cells) |
//! | The neighbours of a cell | [`neighbours8`], [`neighbours10`] |
//! | Cells by distance from a centre | [`shell_offsets`], [`shell`] |
//! | The cells a straight line passes through | [`segment_cells`] |
//!
//! Results never depend on anything but the arguments, so two runs walk
//! the same cells in the same order.

use clash_core::{Dir8, IVec3, Real, Vertical};

use crate::dims::{CELL_M, STOREY_CELLS, STOREY_M};
use crate::pos::{CellPos, distance_sq_cells};

/// The cost of one step to a neighbour, in metres: a cell width for an
/// orthogonal step and √2 of one for a diagonal step (02 §3.3).
pub fn step_cost(dir: Dir8) -> Real {
    if dir.is_diagonal() {
        Real::of(core::f64::consts::SQRT_2) * CELL_M
    } else {
        CELL_M
    }
}

/// The eight horizontal neighbours of a cell, clockwise from north.
pub fn neighbours8(cell: CellPos) -> [CellPos; 8] {
    Dir8::ALL.map(|dir| cell.offset(dir.delta().with_z(0)))
}

/// The ten neighbours of a cell: the eight horizontal ones clockwise from
/// north, then the cell above, then the cell below.
pub fn neighbours10(cell: CellPos) -> [CellPos; 10] {
    let mut out = [cell; 10];
    for (slot, dir) in out.iter_mut().zip(Dir8::ALL) {
        *slot = cell.offset(dir.delta().with_z(0));
    }
    for (slot, vertical) in out[Dir8::ALL.len()..].iter_mut().zip(Vertical::ALL) {
        *slot = cell.offset(IVec3::new(0, 0, vertical.delta()));
    }
    out
}

/// The offsets whose physical distance from the centre, in metres between
/// cell centres with a storey counting three, is at least `radius` and
/// less than `radius + 1`: the shell of that radius.
///
/// The shells of radius 0, 1, 2, … are disjoint and together cover all of
/// space. Offsets are listed storey by storey, row by row.
///
/// ```
/// use clash_core::IVec3;
/// use clash_world::iter::shell_offsets;
///
/// assert_eq!(shell_offsets(0), [IVec3::ZERO]);
/// // One metre out: the four orthogonal and four diagonal neighbours.
/// // The cells above and below are three metres away.
/// assert_eq!(shell_offsets(1).len(), 8);
/// assert!(shell_offsets(3).contains(&IVec3::new(0, 0, 1)));
/// ```
pub fn shell_offsets(radius: u32) -> Vec<IVec3> {
    let inner = i64::from(radius);
    let outer = inner + 1;
    let inner_sq = inner * inner;
    let outer_sq = outer * outer;
    let reach = i32::try_from(inner).unwrap_or(i32::MAX);
    let storeys = i32::try_from(outer / STOREY_CELLS).unwrap_or(i32::MAX);
    let mut out = Vec::new();
    for z in -storeys..=storeys {
        for y in -reach..=reach {
            for x in -reach..=reach {
                let offset = IVec3::new(x, y, z);
                let dist_sq = distance_sq_cells(offset);
                if dist_sq >= inner_sq && dist_sq < outer_sq {
                    out.push(offset);
                }
            }
        }
    }
    out
}

/// The cells of the shell of a radius around a centre, storey by storey,
/// row by row. See [`shell_offsets`].
pub fn shell(centre: CellPos, radius: u32) -> impl Iterator<Item = CellPos> {
    shell_offsets(radius)
        .into_iter()
        .map(move |offset| centre.offset(offset))
}

/// The cells a straight segment passes through, in order from `a` to `b`.
/// Points are in metres; a cell is 1 m × 1 m × 3 m.
///
/// The walk visits:
///
/// - first, the cell that contains `a`;
/// - then every cell in which the segment runs for some length;
/// - last, the cell that contains `b`.
///
/// A point exactly on a boundary belongs to the cell on its positive side.
/// Where the segment passes exactly through an edge or a corner shared by
/// several cells, it steps straight to the cell diagonally across and
/// does not visit the cells it only touches.
///
/// ```
/// use clash_core::Real;
/// use clash_world::CellPos;
/// use clash_world::iter::segment_cells;
///
/// let a = [Real::of(0.5), Real::of(0.5), Real::of(1.0)];
/// let b = [Real::of(2.5), Real::of(0.5), Real::of(4.0)];
/// let cells: Vec<CellPos> = segment_cells(a, b).collect();
/// assert_eq!(
///     cells,
///     [CellPos::new(0, 0, 0), CellPos::new(1, 0, 0), CellPos::new(1, 0, 1), CellPos::new(2, 0, 1)]
/// );
/// ```
pub fn segment_cells(a: [Real; 3], b: [Real; 3]) -> SegmentCells {
    let start = CellPos::containing(a);
    let end = CellPos::containing(b);
    let sizes = [CELL_M, CELL_M, STOREY_M];
    let cell = [start.x, start.y, start.z];
    let target = [end.x, end.y, end.z];
    let mut walk = SegmentCells {
        origin: a,
        dir: [b[0] - a[0], b[1] - a[1], b[2] - a[2]],
        sizes,
        cell,
        target,
        next_t: [Real::INFINITY; 3],
        started: false,
    };
    for axis in 0..3 {
        walk.next_t[axis] = walk.crossing(axis);
    }
    walk
}

/// The iterator returned by [`segment_cells`].
#[derive(Clone, Debug)]
pub struct SegmentCells {
    origin: [Real; 3],
    dir: [Real; 3],
    sizes: [Real; 3],
    cell: [i32; 3],
    target: [i32; 3],
    /// For each axis, how far along the segment (0 at `a`, 1 at `b`) the
    /// next boundary on that axis is crossed; infinite when no step is
    /// left on the axis.
    next_t: [Real; 3],
    started: bool,
}

impl SegmentCells {
    /// How far along the segment the walk leaves its current cell along
    /// an axis. Computed afresh from the boundary's coordinate each time,
    /// so that error does not build up along a long segment.
    fn crossing(&self, axis: usize) -> Real {
        let here = self.cell[axis];
        let target = self.target[axis];
        if here == target {
            return Real::INFINITY;
        }
        let boundary_cell = if target > here {
            i64::from(here) + 1
        } else {
            i64::from(here)
        };
        let boundary = Real::from_i64(boundary_cell) * self.sizes[axis];
        (boundary - self.origin[axis]) / self.dir[axis]
    }
}

impl Iterator for SegmentCells {
    type Item = CellPos;

    fn next(&mut self) -> Option<CellPos> {
        if !self.started {
            self.started = true;
            return Some(CellPos::new(self.cell[0], self.cell[1], self.cell[2]));
        }
        if self.cell == self.target {
            return None;
        }
        let soonest = self.next_t[0].min(self.next_t[1]).min(self.next_t[2]);
        // Every axis that crosses a boundary at this same moment steps
        // together: through an edge or a corner, straight across.
        let mut moved = false;
        for axis in 0..3 {
            if self.cell[axis] != self.target[axis] && self.next_t[axis] == soonest {
                self.cell[axis] += if self.target[axis] > self.cell[axis] {
                    1
                } else {
                    -1
                };
                self.next_t[axis] = self.crossing(axis);
                moved = true;
            }
        }
        if !moved {
            // Only reachable with points that are not finite. End the walk
            // at the last cell instead of never ending it.
            self.cell = self.target;
        }
        Some(CellPos::new(self.cell[0], self.cell[1], self.cell[2]))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn neighbours_come_in_a_fixed_order() {
        let cell = CellPos::new(5, 5, 2);
        let eight = neighbours8(cell);
        assert_eq!(eight[0], CellPos::new(5, 4, 2));
        assert_eq!(eight[2], CellPos::new(6, 5, 2));
        let ten = neighbours10(cell);
        assert_eq!(ten[..8], eight);
        assert_eq!(ten[8], CellPos::new(5, 5, 3));
        assert_eq!(ten[9], CellPos::new(5, 5, 1));
    }

    #[test]
    fn a_diagonal_step_costs_root_two() {
        assert_eq!(step_cost(Dir8::East), Real::of(1.0));
        assert_eq!(
            step_cost(Dir8::SouthEast),
            Real::of(core::f64::consts::SQRT_2)
        );
    }

    #[test]
    fn shells_partition_space() {
        let mut seen = std::collections::BTreeSet::new();
        let limit = 7_u32;
        for radius in 0..=limit {
            let offsets = shell_offsets(radius);
            let mut sorted = offsets.clone();
            sorted.sort_by_key(|o| (o.z, o.y, o.x));
            assert_eq!(offsets, sorted, "shell {radius} is in order");
            for offset in offsets {
                let dist_sq = distance_sq_cells(offset);
                assert!(dist_sq >= i64::from(radius * radius));
                assert!(dist_sq < i64::from((radius + 1) * (radius + 1)));
                assert!(
                    seen.insert((offset.x, offset.y, offset.z)),
                    "no offset twice"
                );
            }
        }
        // Everything nearer than the limit was listed.
        let reach = limit as i32 + 1;
        for z in -reach..=reach {
            for y in -reach..=reach {
                for x in -reach..=reach {
                    let offset = IVec3::new(x, y, z);
                    let inside = distance_sq_cells(offset) < i64::from((limit + 1) * (limit + 1));
                    assert_eq!(seen.contains(&(x, y, z)), inside);
                }
            }
        }
    }

    #[test]
    fn a_segment_through_a_corner_steps_diagonally() {
        let a = [Real::of(0.5), Real::of(0.5), Real::of(1.5)];
        let b = [Real::of(2.5), Real::of(2.5), Real::of(1.5)];
        let cells: Vec<CellPos> = segment_cells(a, b).collect();
        assert_eq!(
            cells,
            [
                CellPos::new(0, 0, 0),
                CellPos::new(1, 1, 0),
                CellPos::new(2, 2, 0)
            ]
        );
    }

    #[test]
    fn a_segment_of_no_length_visits_one_cell() {
        let a = [Real::of(-0.5), Real::of(0.5), Real::of(-0.5)];
        let cells: Vec<CellPos> = segment_cells(a, a).collect();
        assert_eq!(cells, [CellPos::new(-1, 0, -1)]);
    }

    #[test]
    fn a_segment_runs_backward_as_it_runs_forward() {
        let a = [Real::of(0.2), Real::of(0.7), Real::of(0.4)];
        let b = [Real::of(5.9), Real::of(-3.1), Real::of(7.4)];
        let forward: Vec<CellPos> = segment_cells(a, b).collect();
        let mut backward: Vec<CellPos> = segment_cells(b, a).collect();
        backward.reverse();
        assert_eq!(forward, backward);
    }
}
