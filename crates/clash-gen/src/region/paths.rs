//! Path lines: straight stretches of a path's centre line, and how
//! they are laid over a tract's surface by the ground's `paths` step
//! (10 §5.1, §8.3).

use clash_world::dims::TRACT_XY;
use clash_world::{CellPos, FloorState, PathKindId, TractPos};

use crate::tractdef::GenDefs;

/// A straight stretch of a path's centre line, running east to west or
/// north to south, both ends included. A path is as wide as its kind
/// says, about its centre line.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub struct Seg {
    /// The kind of path.
    pub kind: PathKindId,
    /// One end, `(x, y)`.
    pub from: (i32, i32),
    /// The other end, in the same row or column.
    pub to: (i32, i32),
    /// Whether it is a path between regions rather than a street: such a
    /// path is not laid inside a settlement, whose streets meet it.
    pub minor: bool,
}

/// How a cell of a path is laid: its surface, or the strip along a side.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
enum Rank {
    Edge,
    Surface,
}

/// The offsets across a path of a footprint so many cells wide: the
/// lowest and the highest.
pub(crate) fn across(footprint: i32) -> (i32, i32) {
    let low = -((footprint - 1) / 2);
    (low, low + footprint - 1)
}

impl Seg {
    /// The lowest and the highest x and y its centre line covers.
    pub fn span(&self) -> (i32, i32, i32, i32) {
        (
            self.from.0.min(self.to.0),
            self.from.1.min(self.to.1),
            self.from.0.max(self.to.0),
            self.from.1.max(self.to.1),
        )
    }

    /// Whether it runs east to west.
    pub fn runs_east(&self) -> bool {
        self.from.1 == self.to.1
    }

    /// The cells of its centre line, in order from `from`.
    pub fn line(&self) -> Vec<(i32, i32)> {
        let (dx, dy) = (
            (self.to.0 - self.from.0).signum(),
            (self.to.1 - self.from.1).signum(),
        );
        let steps = (self.to.0 - self.from.0)
            .abs()
            .max((self.to.1 - self.from.1).abs());
        (0..=steps)
            .map(|step| (self.from.0 + dx * step, self.from.1 + dy * step))
            .collect()
    }

    /// The lowest and the highest x and y it covers with its whole
    /// width, for a footprint so many cells across.
    pub fn reach(&self, footprint: i32) -> (i32, i32, i32, i32) {
        let (x0, y0, x1, y1) = self.span();
        let (low, high) = across(footprint);
        if self.runs_east() {
            (x0, y0 + low, x1, y1 + high)
        } else {
            (x0 + low, y0, x1 + high, y1)
        }
    }

    /// Whether a column lies within so many cells of the path's
    /// footprint.
    pub fn near(&self, footprint: i32, x: i32, y: i32, margin: i32) -> bool {
        let (x0, y0, x1, y1) = self.reach(footprint);
        x >= x0 - margin && x <= x1 + margin && y >= y0 - margin && y <= y1 + margin
    }
}

/// The floor each column of a tract is given by the paths that cross
/// it, row by row from the tract's north-west corner. Where paths
/// overlap, a surface is laid over a strip, and of two of the same the
/// first in `segs` is laid. `skip` leaves a column out of a minor path.
pub(crate) fn paint(
    defs: &GenDefs,
    tract: TractPos,
    segs: &[Seg],
    skip: &dyn Fn(i32, i32) -> bool,
) -> Vec<Option<FloorState>> {
    let side = TRACT_XY as usize;
    let mut laid: Vec<Option<(Rank, FloorState)>> = vec![None; side * side];
    let origin: CellPos = tract.origin(0);
    for seg in segs {
        let Some(kind) = defs.path_kind(seg.kind) else {
            continue;
        };
        let footprint = kind.footprint();
        let (rx0, ry0, rx1, ry1) = seg.reach(footprint);
        let x0 = rx0.max(origin.x);
        let y0 = ry0.max(origin.y);
        let x1 = rx1.min(origin.x + TRACT_XY - 1);
        let y1 = ry1.min(origin.y + TRACT_XY - 1);
        if x0 > x1 || y0 > y1 {
            continue;
        }
        let (low, _) = across(footprint);
        let strip = kind.edge.map_or(0, |(_, width)| width);
        for y in y0..=y1 {
            for x in x0..=x1 {
                if seg.minor && skip(x, y) {
                    continue;
                }
                let offset = if seg.runs_east() {
                    y - seg.from.1
                } else {
                    x - seg.from.0
                };
                let from_side = offset - low;
                let (rank, floor) = match kind.edge {
                    Some((edge, _)) if from_side < strip || from_side >= footprint - strip => {
                        (Rank::Edge, edge)
                    }
                    _ => (Rank::Surface, kind.surface),
                };
                let at = ((y - origin.y) as usize) * side + (x - origin.x) as usize;
                if let Some(slot) = laid.get_mut(at)
                    && slot.is_none_or(|(have, _)| rank > have)
                {
                    *slot = Some((rank, floor));
                }
            }
        }
    }
    laid.into_iter()
        .map(|slot| slot.map(|(_, floor)| floor))
        .collect()
}
