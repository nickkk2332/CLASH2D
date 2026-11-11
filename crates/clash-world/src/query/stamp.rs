//! [`RevStamp`]: the chunk revisions a cached answer was worked out from,
//! so that the cache can tell whether it is stale (02 §6.2, §22).

use crate::chunk::ChunkRev;
use crate::cosmos::{Cosmos, GridView};
use crate::pos::{CellBox, CellPos, ChunkPos, GridId};

/// Which of a chunk's revision counters a cached answer depends on.
///
/// ```
/// use clash_world::RevParts;
///
/// let both = RevParts::STRUCTURE | RevParts::OCCUPANTS;
/// assert!(both.contains(RevParts::STRUCTURE));
/// assert!(!both.contains(RevParts::FIELDS));
/// ```
#[derive(Clone, Copy, PartialEq, Eq, Hash, Default, Debug)]
pub struct RevParts(u8);

impl RevParts {
    /// Changes to floors, blocks and fixtures.
    pub const STRUCTURE: RevParts = RevParts(1 << 0);
    /// Changes to fields.
    pub const FIELDS: RevParts = RevParts(1 << 1);
    /// Changes to who occupies the cells.
    pub const OCCUPANTS: RevParts = RevParts(1 << 2);
    /// Every counter.
    pub const ALL: RevParts = RevParts(0b111);

    /// Whether every part of `other` is included here.
    pub const fn contains(self, other: RevParts) -> bool {
        self.0 & other.0 == other.0
    }

    /// Both sets together.
    pub const fn union(self, other: RevParts) -> RevParts {
        RevParts(self.0 | other.0)
    }

    /// A chunk's counters with those not included here set to zero.
    fn mask(self, rev: ChunkRev) -> ChunkRev {
        let keep = |part: RevParts, value: u32| if self.contains(part) { value } else { 0 };
        ChunkRev {
            structure: keep(RevParts::STRUCTURE, rev.structure),
            fields: keep(RevParts::FIELDS, rev.fields),
            occupants: keep(RevParts::OCCUPANTS, rev.occupants),
        }
    }
}

impl core::ops::BitOr for RevParts {
    type Output = RevParts;
    fn bitor(self, other: RevParts) -> RevParts {
        self.union(other)
    }
}

/// What a stamp saw of one chunk.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Seen {
    /// The chunk's tract was not loaded, or the chunk lies outside the
    /// grid.
    Absent,
    /// The chunk's tract was loaded and its counters stood here. A chunk
    /// that holds only its tract's fill has never been written: zero.
    At(ChunkRev),
}

/// The revisions of a set of chunks at one moment: the key of a cached
/// answer that was worked out from the cells of those chunks.
///
/// A stamp stops being current when a cell of one of its chunks is
/// edited, or when one of their tracts is loaded or unloaded, and for no
/// other reason: a write that changes nothing, a write to another chunk
/// and the passing of time leave it current. Counters are compared for
/// equality, so their wrapping round does not matter.
///
/// One limit, until tracts are loaded through tickets (package W8): a
/// tract that is unloaded and loaded again between two looks at a stamp
/// starts its counters afresh, and can look unchanged. Whoever keeps a
/// cached answer across the unloading of its tracts drops it.
///
/// ```
/// use clash_world::{RevParts, RevStamp};
///
/// let stamp = RevStamp::new(RevParts::STRUCTURE);
/// assert!(stamp.is_empty());
/// ```
#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct RevStamp {
    grid: Option<GridId>,
    parts: RevParts,
    /// Sorted by chunk position, storey by storey, row by row.
    seen: Vec<(ChunkPos, Seen)>,
}

impl RevStamp {
    /// An empty stamp that will watch these counters.
    pub fn new(parts: RevParts) -> RevStamp {
        RevStamp {
            grid: None,
            parts,
            seen: Vec::new(),
        }
    }

    /// Forgets every chunk, keeping the storage for the next use.
    pub fn clear(&mut self) {
        self.grid = None;
        self.seen.clear();
    }

    /// Whether the stamp holds no chunk.
    pub fn is_empty(&self) -> bool {
        self.seen.is_empty()
    }

    /// The counters the stamp watches.
    pub fn parts(&self) -> RevParts {
        self.parts
    }

    /// The chunks the stamp holds, storey by storey, row by row.
    pub fn chunks(&self) -> impl Iterator<Item = ChunkPos> + '_ {
        self.seen.iter().map(|(chunk, _)| *chunk)
    }

    /// What a chunk looks like now.
    fn look(parts: RevParts, view: &GridView<'_>, chunk: ChunkPos) -> Seen {
        match view.column(chunk.tract()) {
            Some(column) => {
                let rev = column
                    .chunk(chunk.z)
                    .map_or(ChunkRev::default(), |c| c.rev());
                Seen::At(parts.mask(rev))
            }
            None => Seen::Absent,
        }
    }

    /// Adds the chunk that holds a cell, as it stands now. A stamp holds
    /// chunks of one grid: adding from another grid starts it afresh.
    pub fn record_cell(&mut self, view: &GridView<'_>, cell: CellPos) {
        if self.grid != Some(view.id()) {
            self.clear();
            self.grid = Some(view.id());
        }
        // A position beyond an edge keeps its own chunk, which no tract
        // holds: it reads as absent now and always.
        let chunk = view.normalize(cell).unwrap_or(cell).chunk();
        if let Err(at) = self.seen.binary_search_by_key(&chunk, |(have, _)| *have) {
            let seen = RevStamp::look(self.parts, view, chunk);
            self.seen.insert(at, (chunk, seen));
        }
    }

    /// Adds every chunk that a box of cells touches, as each stands now.
    pub fn record_box(&mut self, view: &GridView<'_>, cells: CellBox) {
        let lo = cells.min.chunk();
        let hi = cells.max.chunk();
        for z in lo.z..=hi.z {
            for y in lo.y..=hi.y {
                for x in lo.x..=hi.x {
                    self.record_cell(view, ChunkPos::new(x, y, z).origin());
                }
            }
        }
    }

    /// Whether every chunk of the stamp still stands as it did when it
    /// was added. An empty stamp is current.
    pub fn is_current(&self, cosmos: &Cosmos) -> bool {
        let Some(grid) = self.grid else {
            return self.seen.is_empty();
        };
        let Ok(view) = cosmos.grid(grid) else {
            return false;
        };
        self.seen
            .iter()
            .all(|(chunk, seen)| RevStamp::look(self.parts, &view, *chunk) == *seen)
    }
}
