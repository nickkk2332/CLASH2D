//! The shape of a grid: how far it extends, which axes wrap, and which
//! edge a position beyond it has crossed (02 §4.2).

use clash_core::{IVec3, Real, StateHash, StateHasher};
use serde::{Deserialize, Serialize};

use crate::pos::{CellPos, distance_m};

/// One of the six faces of a realm.
///
/// When a position lies beyond several faces at once, the one reported is
/// the first in this order: west, east, north, south, bottom, top.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Edge {
    /// Beyond the lowest `x`.
    West,
    /// Beyond the highest `x`.
    East,
    /// Beyond the lowest `y`.
    North,
    /// Beyond the highest `y`.
    South,
    /// Below the lowest storey.
    Bottom,
    /// Above the highest storey.
    Top,
}

impl Edge {
    /// All six faces, in reporting order.
    pub const ALL: [Edge; 6] = [
        Edge::West,
        Edge::East,
        Edge::North,
        Edge::South,
        Edge::Bottom,
        Edge::Top,
    ];

    /// The name used in data.
    pub const fn name(self) -> &'static str {
        match self {
            Edge::West => "west",
            Edge::East => "east",
            Edge::North => "north",
            Edge::South => "south",
            Edge::Bottom => "bottom",
            Edge::Top => "top",
        }
    }
}

impl StateHash for Edge {
    fn state_hash(&self, hasher: &mut StateHasher) {
        hasher.write_u8(*self as u8);
    }
}

/// The extent of a grid along one horizontal axis.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Serialize, Deserialize)]
pub enum Extent {
    /// No limit: every coordinate is inside. A subgrid's own space.
    Unbounded,
    /// Coordinates `0..size` are inside and anything else is beyond an
    /// edge.
    Bounded(i32),
    /// Coordinates `0..size`, and anything else wraps round into them.
    Wrapped(i32),
}

impl Extent {
    /// Brings a coordinate inside, or says on which side it lies:
    /// `Err(false)` below zero, `Err(true)` at or above the size.
    fn normalize(self, value: i32) -> Result<i32, bool> {
        match self {
            Extent::Unbounded => Ok(value),
            Extent::Bounded(size) => {
                if value < 0 {
                    Err(false)
                } else if value >= size {
                    Err(true)
                } else {
                    Ok(value)
                }
            }
            Extent::Wrapped(size) => Ok(wrap(value, size)),
        }
    }

    /// The shortest signed step from `a` to `b` along this axis.
    fn delta(self, a: i32, b: i32) -> i32 {
        match self {
            Extent::Unbounded | Extent::Bounded(_) => b.wrapping_sub(a),
            Extent::Wrapped(size) if size <= 0 => 0,
            Extent::Wrapped(size) => {
                let size = i64::from(size);
                let a = i64::from(wrap(a, size as i32));
                let b = i64::from(wrap(b, size as i32));
                let forward = (b - a).rem_euclid(size);
                let backward = forward - size;
                // Exactly half way round is equally short both ways. The
                // lower coordinate goes forward and the higher goes back,
                // so that `delta(a, b) == -delta(b, a)` always.
                let step = if forward < -backward || (forward == -backward && a < b) {
                    forward
                } else {
                    backward
                };
                step as i32
            }
        }
    }
}

/// `value` brought into `0..size`. A size of zero or less gives zero.
fn wrap(value: i32, size: i32) -> i32 {
    if size <= 0 { 0 } else { value.rem_euclid(size) }
}

/// The shape of a grid.
///
/// ```
/// use clash_world::{CellPos, Edge, Extent, Topology};
///
/// // A world 100 cells round and 50 deep, ten storeys tall.
/// let shape = Topology::new(Extent::Wrapped(100), Extent::Bounded(50), 0, 9);
/// assert_eq!(shape.normalize(CellPos::new(-1, 3, 0)), Ok(CellPos::new(99, 3, 0)));
/// assert_eq!(shape.normalize(CellPos::new(5, 50, 0)), Err(Edge::South));
/// assert_eq!(shape.delta(CellPos::new(98, 0, 0), CellPos::new(1, 0, 0)).x, 3);
/// ```
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Serialize, Deserialize)]
pub struct Topology {
    /// The extent along `x`.
    pub x: Extent,
    /// The extent along `y`.
    pub y: Extent,
    /// The lowest storey.
    pub z_min: i32,
    /// The highest storey. Part of the grid.
    pub z_max: i32,
}

impl Topology {
    /// A shape with these extents and storeys `z_min..=z_max`.
    pub const fn new(x: Extent, y: Extent, z_min: i32, z_max: i32) -> Topology {
        Topology { x, y, z_min, z_max }
    }

    /// A shape without limits: every position is inside. Used by subgrids.
    pub const fn unbounded() -> Topology {
        Topology {
            x: Extent::Unbounded,
            y: Extent::Unbounded,
            z_min: i32::MIN,
            z_max: i32::MAX,
        }
    }

    /// Wraps a position on wrapped axes, or reports which edge it lies
    /// beyond. All world access normalises first.
    #[inline]
    pub fn normalize(&self, cell: CellPos) -> Result<CellPos, Edge> {
        let x = self
            .x
            .normalize(cell.x)
            .map_err(|high| if high { Edge::East } else { Edge::West })?;
        let y = self
            .y
            .normalize(cell.y)
            .map_err(|high| if high { Edge::South } else { Edge::North })?;
        if cell.z < self.z_min {
            return Err(Edge::Bottom);
        }
        if cell.z > self.z_max {
            return Err(Edge::Top);
        }
        Ok(CellPos::new(x, y, cell.z))
    }

    /// Whether a position is inside without wrapping.
    pub fn contains(&self, cell: CellPos) -> bool {
        self.normalize(cell) == Ok(cell)
    }

    /// The shortest offset from `a` to `b`, taking the shorter way round
    /// on wrapped axes. `delta(a, b)` is always the negative of
    /// `delta(b, a)`.
    pub fn delta(&self, a: CellPos, b: CellPos) -> IVec3 {
        IVec3::new(
            self.x.delta(a.x, b.x),
            self.y.delta(a.y, b.y),
            b.z.wrapping_sub(a.z),
        )
    }

    /// The physical distance between two cell centres in metres, the
    /// shorter way round (02 §3.3).
    pub fn distance_m(&self, a: CellPos, b: CellPos) -> Real {
        distance_m(self.delta(a, b))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn edges_are_reported_in_a_fixed_order() {
        let shape = Topology::new(Extent::Bounded(10), Extent::Bounded(10), 0, 4);
        assert_eq!(shape.normalize(CellPos::new(-1, -1, -1)), Err(Edge::West));
        assert_eq!(shape.normalize(CellPos::new(10, -1, 9)), Err(Edge::East));
        assert_eq!(shape.normalize(CellPos::new(0, -1, 9)), Err(Edge::North));
        assert_eq!(shape.normalize(CellPos::new(0, 10, 9)), Err(Edge::South));
        assert_eq!(shape.normalize(CellPos::new(0, 0, -1)), Err(Edge::Bottom));
        assert_eq!(shape.normalize(CellPos::new(0, 0, 5)), Err(Edge::Top));
        assert_eq!(
            shape.normalize(CellPos::new(9, 9, 4)),
            Ok(CellPos::new(9, 9, 4))
        );
        assert!(shape.contains(CellPos::new(0, 0, 0)));
    }

    #[test]
    fn wrapped_axes_have_no_edges() {
        let shape = Topology::new(Extent::Wrapped(10), Extent::Wrapped(4), 0, 0);
        assert_eq!(
            shape.normalize(CellPos::new(-11, 9, 0)),
            Ok(CellPos::new(9, 1, 0))
        );
        assert!(!shape.contains(CellPos::new(-1, 0, 0)));
    }

    #[test]
    fn half_way_round_is_antisymmetric() {
        let shape = Topology::new(Extent::Wrapped(10), Extent::Bounded(10), 0, 0);
        let a = CellPos::new(2, 0, 0);
        let b = CellPos::new(7, 0, 0);
        assert_eq!(shape.delta(a, b).x, 5);
        assert_eq!(shape.delta(b, a).x, -5);
    }

    #[test]
    fn the_unbounded_shape_contains_everything() {
        let shape = Topology::unbounded();
        let far = CellPos::new(i32::MIN, i32::MAX, i32::MIN);
        assert_eq!(shape.normalize(far), Ok(far));
        assert_eq!(
            shape.delta(CellPos::new(1, 1, 1), CellPos::new(4, -1, 0)),
            IVec3::new(3, -2, -1)
        );
    }
}
