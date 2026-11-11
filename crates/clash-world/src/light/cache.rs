//! The cache of static light: for each chunk asked about, how far each
//! cell is from the open sky and what the fixed sources give it.
//!
//! Two tables, both keyed by grid and chunk position, neither part of a
//! chunk:
//!
//! - A [`Shape`] is what one chunk looks like to light: for each cell
//!   whether it stops light, has a floor and is open to the sky, and the
//!   sources in it. It is read from the cells again when the chunk's
//!   structure revision moves, or that of any chunk of its column (a
//!   roof far above changes what is open to the sky). When what it reads
//!   differs from what it had, it notes the box of cells that changed.
//! - A [`Lit`] is the answer for one chunk, worked out from the shapes
//!   of the chunk and its twenty-six neighbours. It is stale when one of
//!   those shapes has changed, since it was worked out, in cells within
//!   light's reach of the chunk; a change further off, or one that light
//!   does not care about, leaves it standing.
//!
//! Every answer is a function of the cells within reach of its chunk
//! and of nothing else, so the order in which chunks are asked about
//! never shows in a result.

use clash_core::{IVec3, LookupMap, Real};

use super::defs::LightSource;
use super::solve::{self, Scratch};
use super::{LIGHT_EYE_MM, LightStats, SKY_DARK};
use crate::cosmos::{Cosmos, GridView};
use crate::defs::{BlockState, CellFlags, EngineFlag, FixtureState, Layer, StateInfo, WorldDefs};
use crate::dims::{CHUNK_AREA, CHUNK_CELLS, CHUNK_XY, CHUNK_Z, CHUNK_Z_BITS, STOREY_M, STOREY_MM};
use crate::grid::Column;
use crate::pos::{CellIdx, CellPos, ChunkPos, GridId, TractPos};

/// A shape bit: light does not pass the cell. Something `opaque` in it
/// stands taller than the height light travels at.
pub(super) const S_BLOCKS: u8 = 1;
/// A shape bit: the cell has a floor, which closes it from the cell
/// below.
pub(super) const S_FLOOR: u8 = 1 << 1;
/// A shape bit: the cell is open to the sky.
pub(super) const S_SKY: u8 = 1 << 2;

/// Chunks along one side of the block an answer is worked out from: the
/// chunk and its neighbours.
const BLOCK_SIDE: usize = 3;
/// Chunks in that block.
pub(super) const BLOCK_CHUNKS: usize = BLOCK_SIDE * BLOCK_SIDE * BLOCK_SIDE;
/// The place of the chunk itself in its block.
pub(super) const BLOCK_CENTRE: usize = BLOCK_CHUNKS / 2;

/// The changes a shape remembers. An answer that last looked before the
/// oldest of them is worked out again.
const CHANGE_LOG_MAX: usize = 8;

/// The most sources of light counted in one chunk, in cell order. A
/// chunk filled with something that glows is a budget, not a mistake.
pub const LIGHT_SOURCES_MAX_PER_CHUNK: usize = 128;

/// The most chunks whose light is kept. When more have been asked
/// about, those not asked about lately are dropped.
pub const LIGHT_CACHE_MAX_CHUNKS: usize = 1 << 10;

/// How many passes a chunk may go unasked before a full cache drops it.
const TRIM_AGE: u64 = 4;

/// The most tracts noted as loaded or unloaded since the cache was last
/// emptied. Past this the cache is emptied.
const RELOAD_NOTES_MAX: usize = 1 << 12;

/// Where a chunk of a block lies from the block's middle chunk, in
/// chunks. Places run storey by storey, row by row.
pub(super) fn block_offset(place: usize) -> IVec3 {
    let part = |n: usize| (n % BLOCK_SIDE) as i32 - 1;
    IVec3::new(
        part(place),
        part(place / BLOCK_SIDE),
        part(place / (BLOCK_SIDE * BLOCK_SIDE)),
    )
}

type ChunkKey = (GridId, ChunkPos);

/// What a shape was read from.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Seen {
    /// The chunk's tract was not loaded.
    Absent,
    /// The tract was loaded, with the chunk's structure revision here
    /// and the revisions of all the column's chunks adding up to this.
    At { rev: u32, column_rev: u32 },
}

/// The shape bits of every cell of a chunk.
#[derive(Clone, PartialEq, Eq, Debug)]
pub(super) enum ShapeCells {
    /// Every cell has these bits.
    All(u8),
    /// The bits of each cell, by cell index.
    Each(Box<[u8; CHUNK_CELLS]>),
}

impl ShapeCells {
    #[inline]
    fn at(&self, index: usize) -> u8 {
        match self {
            ShapeCells::All(bits) => *bits,
            ShapeCells::Each(cells) => cells[index],
        }
    }
}

/// A box of cells that changed, in the chunk's own coordinates.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
struct Change {
    /// The version the shape took with this change.
    version: u64,
    min: [i32; 3],
    max: [i32; 3],
}

/// What one chunk looks like to light.
#[derive(Clone, Debug)]
pub(super) struct Shape {
    seen: Option<Seen>,
    pub(super) cells: ShapeCells,
    /// The sources in the chunk, in cell order.
    pub(super) sources: Vec<(CellIdx, LightSource)>,
    /// The number the shape took when it last changed. Numbers are
    /// drawn from one count for the whole cache, so a shape dropped and
    /// read again never takes one it had before.
    version: u64,
    /// The latest changes, oldest first.
    changes: Vec<Change>,
    /// Changes up to this version are no longer in `changes`.
    forgotten: u64,
    /// The pass in which the shape was last compared with the world.
    checked: u64,
    /// Read the cells again at the next look, whatever the revisions
    /// say.
    reread: bool,
}

impl Shape {
    fn new() -> Shape {
        Shape {
            seen: None,
            cells: ShapeCells::All(S_BLOCKS),
            sources: Vec::new(),
            version: 0,
            changes: Vec::new(),
            forgotten: 0,
            checked: 0,
            reread: false,
        }
    }

    /// Whether the chunk's tract was loaded when the shape was read.
    pub(super) fn is_present(&self) -> bool {
        matches!(self.seen, Some(Seen::At { .. }))
    }

    /// Whether a change made since `seen_version`, in a chunk `offset`
    /// chunks from another, lies within `reach` cells of that other
    /// chunk.
    fn changed_near(&self, seen_version: u64, offset: IVec3, reach: IVec3) -> bool {
        if seen_version == self.version {
            return false;
        }
        if seen_version < self.forgotten || seen_version > self.version {
            return true;
        }
        let shift = [offset.x * CHUNK_XY, offset.y * CHUNK_XY, offset.z * CHUNK_Z];
        let reach = [reach.x, reach.y, reach.z];
        let side = [CHUNK_XY, CHUNK_XY, CHUNK_Z];
        self.changes
            .iter()
            .filter(|change| change.version > seen_version)
            .any(|change| {
                (0..3).all(|axis| {
                    change.min[axis] + shift[axis] - reach[axis] < side[axis]
                        && change.max[axis] + shift[axis] + reach[axis] >= 0
                })
            })
    }
}

/// How far each cell of a chunk is from the open sky, in spread steps.
#[derive(Clone, PartialEq, Eq, Debug)]
pub(super) enum Steps {
    /// Every cell is this far.
    All(u8),
    /// The distance of each cell, by cell index.
    Each(Box<[u8; CHUNK_CELLS]>),
}

/// What the fixed sources give the cells of one storey of a chunk.
type FixedStorey = Option<Box<[Real; CHUNK_AREA]>>;

/// The static light of one chunk.
#[derive(Clone, Debug)]
pub(super) struct Lit {
    pub(super) sky: Steps,
    /// What the fixed sources give, storey by storey. `None` for a
    /// storey they do not reach.
    pub(super) fixed: [FixedStorey; CHUNK_Z as usize],
    /// The versions of the shapes of the block when this was worked out.
    deps: [u64; BLOCK_CHUNKS],
    /// The pass in which this was last asked for.
    used: u64,
}

impl Lit {
    /// The steps from the open sky and the fixed light of a cell.
    #[inline]
    pub(super) fn at(&self, idx: CellIdx) -> (u8, Real) {
        let steps = match &self.sky {
            Steps::All(steps) => *steps,
            Steps::Each(cells) => cells[idx.index()],
        };
        let fixed = match &self.fixed[idx.lz() as usize] {
            Some(storey) => storey[idx.column()],
            None => Real::ZERO,
        };
        (steps, fixed)
    }

    /// The steps from the open sky of the cells of one storey: one value
    /// for all, or each by its place in the storey.
    #[inline]
    pub(super) fn sky_storey(&self, lz: usize) -> Result<u8, &[u8]> {
        match &self.sky {
            Steps::All(steps) => Ok(*steps),
            Steps::Each(cells) => Err(&cells[lz * CHUNK_AREA..(lz + 1) * CHUNK_AREA]),
        }
    }

    /// The fixed light of the cells of one storey, by place in the
    /// storey, if any reaches it.
    #[inline]
    pub(super) fn fixed_storey(&self, lz: usize) -> Option<&[Real; CHUNK_AREA]> {
        self.fixed[lz].as_deref()
    }
}

/// The cache: shapes and answers, by grid and chunk.
#[derive(Debug)]
pub(crate) struct LightCache {
    shapes: LookupMap<ChunkKey, Shape>,
    lit: LookupMap<ChunkKey, Lit>,
    /// Tracts loaded or unloaded, each with the pass it happened after.
    /// A shape of such a tract last compared at or before that pass is
    /// read again whatever its revisions say.
    reloaded: LookupMap<(GridId, TractPos), u64>,
    /// Counts the questions asked. Within one, the world cannot change,
    /// so each shape is compared with it at most once.
    pass: u64,
    /// The last version number given to a shape.
    versions: u64,
    stats: LightStats,
    scratch: Scratch,
}

impl LightCache {
    pub(crate) fn new() -> LightCache {
        LightCache {
            shapes: LookupMap::new(),
            lit: LookupMap::new(),
            reloaded: LookupMap::new(),
            pass: 1,
            versions: 0,
            stats: LightStats::default(),
            scratch: Scratch::default(),
        }
    }

    /// Forgets everything worked out.
    pub(crate) fn clear(&mut self) {
        self.shapes.clear();
        self.lit.clear();
        self.reloaded.clear();
    }

    /// Notes that a tract was loaded or unloaded: its chunks are read
    /// again when next asked about, whatever their revisions say.
    pub(crate) fn tract_reloaded(&mut self, grid: GridId, tract: TractPos) {
        if self.shapes.is_empty() {
            return;
        }
        if self.reloaded.len() >= RELOAD_NOTES_MAX {
            // Too many to keep note of: start again from nothing.
            self.clear();
            return;
        }
        self.reloaded.insert((grid, tract), self.pass);
    }

    pub(crate) fn stats(&self) -> LightStats {
        LightStats {
            cached_chunks: self.lit.len(),
            ..self.stats
        }
    }

    /// Starts a question. The world does not change until the next.
    pub(super) fn begin(&mut self) {
        self.pass += 1;
        if self.lit.len() > LIGHT_CACHE_MAX_CHUNKS {
            self.trim();
        }
    }

    /// Drops what has not been asked about lately.
    pub(super) fn trim(&mut self) {
        let oldest = self.pass.saturating_sub(TRIM_AGE);
        for key in self.lit.sorted_keys() {
            if self.lit.get(&key).is_some_and(|lit| lit.used < oldest) {
                self.lit.remove(&key);
            }
        }
        for key in self.shapes.sorted_keys() {
            if self
                .shapes
                .get(&key)
                .is_some_and(|shape| shape.checked < oldest)
            {
                self.shapes.remove(&key);
            }
        }
        if self.lit.len() > LIGHT_CACHE_MAX_CHUNKS {
            // One question about more chunks than are kept.
            self.lit.clear();
        }
        // A shape dropped here is read afresh when asked about, so no
        // note of a reloaded tract is needed for it: but a shape kept
        // might yet miss one. Make every kept shape read its cells
        // again, and the notes can go.
        self.reloaded.clear();
        for key in self.shapes.sorted_keys() {
            if let Some(shape) = self.shapes.get_mut(&key) {
                shape.reread = true;
            }
        }
    }

    /// Brings the shape of a chunk up to date with the world and returns
    /// its version.
    fn refresh(&mut self, view: &GridView<'_>, defs: &WorldDefs, chunk: ChunkPos) -> u64 {
        let key = (view.id(), chunk);
        let pass = self.pass;
        let reloaded = if self.reloaded.is_empty() {
            None
        } else {
            self.reloaded.get(&(view.id(), chunk.tract())).copied()
        };
        let shape = self.shapes.get_or_insert_with(key, Shape::new);
        if shape.checked == pass {
            return shape.version;
        }
        let column = view.column(chunk.tract());
        let now = match column {
            Some(column) => Seen::At {
                rev: column
                    .chunk(chunk.z)
                    .map_or(0, |stored| stored.rev().structure),
                column_rev: column.chunks().fold(0_u32, |sum, (_, stored)| {
                    sum.wrapping_add(stored.rev().structure)
                }),
            },
            None => Seen::Absent,
        };
        let forced = shape.reread || reloaded.is_some_and(|after| shape.checked <= after);
        shape.checked = pass;
        shape.reread = false;
        if shape.seen == Some(now) && !forced {
            return shape.version;
        }
        let (cells, sources) = match column {
            Some(column) => read_shape(defs, column, chunk.z),
            None => (ShapeCells::All(S_BLOCKS), Vec::new()),
        };
        self.stats.shapes_read += 1;
        let first = shape.seen.is_none();
        let was_present = shape.is_present();
        shape.seen = Some(now);
        let changed = if first || was_present != shape.is_present() {
            Some(([0; 3], [CHUNK_XY - 1, CHUNK_XY - 1, CHUNK_Z - 1]))
        } else {
            changed_box(&shape.cells, &shape.sources, &cells, &sources)
        };
        if let Some((min, max)) = changed {
            self.versions += 1;
            shape.version = self.versions;
            shape.cells = cells;
            shape.sources = sources;
            if first {
                // Nothing was worked out from the shape before this.
                shape.forgotten = shape.version;
            } else {
                shape.changes.push(Change {
                    version: shape.version,
                    min,
                    max,
                });
                if shape.changes.len() > CHANGE_LOG_MAX {
                    shape.forgotten = shape.changes.remove(0).version;
                }
            }
        }
        shape.version
    }

    /// The static light of a chunk whose tract is loaded, worked out if
    /// it is not there or no longer stands. `chunk` is inside its grid.
    pub(super) fn lit(&mut self, cosmos: &Cosmos, view: &GridView<'_>, chunk: ChunkPos) -> &Lit {
        let defs: &WorldDefs = cosmos.defs();
        let key = (view.id(), chunk);
        // The block: the chunk and its neighbours, where they exist.
        let mut chunks = [None; BLOCK_CHUNKS];
        let mut versions = [0_u64; BLOCK_CHUNKS];
        for place in 0..BLOCK_CHUNKS {
            let offset = block_offset(place);
            let origin = chunk.origin().offset(IVec3::new(
                offset.x * CHUNK_XY,
                offset.y * CHUNK_XY,
                offset.z * CHUNK_Z,
            ));
            if let Ok(origin) = view.normalize(origin) {
                let neighbour = origin.chunk();
                chunks[place] = Some(neighbour);
                versions[place] = self.refresh(view, defs, neighbour);
            }
        }
        let reach = defs.light_reach();
        let pass = self.pass;
        let stands = match self.lit.get_mut(&key) {
            Some(lit) => {
                let stale = (0..BLOCK_CHUNKS).any(|place| {
                    lit.deps[place] != versions[place]
                        && chunks[place]
                            .and_then(|neighbour| self.shapes.get(&(view.id(), neighbour)))
                            .is_none_or(|shape| {
                                shape.changed_near(lit.deps[place], block_offset(place), reach)
                            })
                });
                if !stale {
                    // Whatever changed was out of reach.
                    lit.deps = versions;
                    lit.used = pass;
                }
                !stale
            }
            None => false,
        };
        if !stands {
            let shapes: [Option<&Shape>; BLOCK_CHUNKS] = core::array::from_fn(|place| {
                chunks[place].and_then(|neighbour| self.shapes.get(&(view.id(), neighbour)))
            });
            let sky = solve::sky_steps(&shapes, defs.sky_reach(), &mut self.scratch);
            let fixed = solve::fixed_light(cosmos, view.id(), &chunks, &shapes, &mut self.scratch);
            self.stats.chunks_lit += 1;
            self.lit.insert(
                key,
                Lit {
                    sky,
                    fixed,
                    deps: versions,
                    used: pass,
                },
            );
        }
        // The entry is there: it stood or was just put in.
        self.lit.get_or_insert_with(key, || Lit {
            sky: Steps::All(SKY_DARK),
            fixed: Default::default(),
            deps: versions,
            used: pass,
        })
    }
}

/// Reads the shape of one chunk height of a loaded column.
fn read_shape(
    defs: &WorldDefs,
    column: &Column,
    chunk_z: i32,
) -> (ShapeCells, Vec<(CellIdx, LightSource)>) {
    let base = chunk_z << CHUNK_Z_BITS;
    let eye_m = Real::from(LIGHT_EYE_MM) * STOREY_M / Real::from(STOREY_MM);
    let taller = |info: &StateInfo| info.flags.has(EngineFlag::Opaque) && info.height > eye_m;
    let bits_of = |flags: CellFlags, block: BlockState, fixture: FixtureState| {
        let mut bits = 0;
        if flags.contains(CellFlags::OPAQUE)
            && (taller(defs.block_info(block)) || taller(defs.fixture_info(fixture)))
        {
            bits |= S_BLOCKS;
        }
        if flags.contains(CellFlags::HAS_FLOOR) {
            bits |= S_FLOOR;
        }
        bits
    };

    let stored = column.chunk(chunk_z);
    let mut cells = Box::new([0_u8; CHUNK_CELLS]);
    // Whether every cell holds the same block and the same fixture, so
    // that one cell answers for all. Cells that share their flags may
    // still differ in height.
    let alike = stored.is_none_or(|chunk| {
        let one = |layer: Layer| {
            let mut states = 0;
            chunk.any_state(layer, |_| {
                states += 1;
                false
            });
            states == 1
        };
        chunk.uniform_flags().is_some() && one(Layer::Block) && one(Layer::Fixture)
    });
    match stored {
        Some(chunk) if !alike => {
            let each = chunk.flag_cells();
            let all = chunk.uniform_flags().unwrap_or(CellFlags::NONE);
            for idx in CellIdx::all() {
                let flags = each.map_or(all, |each| each[idx.index()]);
                // The elements are looked up only where something is
                // opaque.
                cells[idx.index()] = if flags.contains(CellFlags::OPAQUE) {
                    bits_of(flags, chunk.block(idx), chunk.fixture(idx))
                } else {
                    bits_of(flags, BlockState::NONE, FixtureState::NONE)
                };
            }
        }
        _ => {
            let first = CellPos::new(0, 0, base);
            let bits = bits_of(
                column.flags(first),
                column.block(first),
                column.fixture(first),
            );
            cells.fill(bits);
        }
    }
    // Open to the sky: at or above the column's highest storey that is
    // not open (02 §6.3).
    for at in 0..CHUNK_AREA {
        let from = (i64::from(column.height(at)) - i64::from(base)).clamp(0, i64::from(CHUNK_Z));
        for lz in from as u32..CHUNK_Z as u32 {
            cells[CellIdx::from_column(at, lz).index()] |= S_SKY;
        }
    }
    let first = cells[0];
    let cells = if cells.iter().all(|bits| *bits == first) {
        ShapeCells::All(first)
    } else {
        ShapeCells::Each(cells)
    };

    // Sources: only where a layer's palette holds something that glows.
    let mut sources = Vec::new();
    if let Some(chunk) = stored.filter(|_| defs.has_light_sources()) {
        let layers = [Layer::Floor, Layer::Block, Layer::Fixture].map(|layer| {
            chunk
                .any_state(layer, |state| defs.light_of(layer, state).is_some())
                .then_some(layer)
        });
        if layers.iter().any(Option::is_some) {
            'cells: for idx in CellIdx::all() {
                for layer in layers.into_iter().flatten() {
                    if let Some(source) = defs.light_of(layer, chunk.element(layer, idx)) {
                        if sources.len() == LIGHT_SOURCES_MAX_PER_CHUNK {
                            break 'cells;
                        }
                        sources.push((idx, source));
                    }
                }
            }
        }
    }
    (cells, sources)
}

/// The box of cells in which two shapes of one chunk differ, if they do.
fn changed_box(
    old: &ShapeCells,
    old_sources: &[(CellIdx, LightSource)],
    new: &ShapeCells,
    new_sources: &[(CellIdx, LightSource)],
) -> Option<([i32; 3], [i32; 3])> {
    let mut found: Option<([i32; 3], [i32; 3])> = None;
    let mut add = |idx: CellIdx| {
        let at = [idx.lx(), idx.ly(), idx.lz()];
        let (min, max) = found.get_or_insert((at, at));
        for axis in 0..3 {
            min[axis] = min[axis].min(at[axis]);
            max[axis] = max[axis].max(at[axis]);
        }
    };
    match (old, new) {
        _ if old == new => {}
        (ShapeCells::All(_), ShapeCells::All(_)) => {
            add(CellIdx::default());
            add(CellIdx::new(
                (CHUNK_XY - 1) as u32,
                (CHUNK_XY - 1) as u32,
                (CHUNK_Z - 1) as u32,
            ));
        }
        _ => {
            for idx in CellIdx::all() {
                if old.at(idx.index()) != new.at(idx.index()) {
                    add(idx);
                }
            }
        }
    }
    if old_sources != new_sources {
        // Two sorted lists: whatever is in one and not the other.
        let (mut a, mut b) = (0, 0);
        while a < old_sources.len() || b < new_sources.len() {
            match (old_sources.get(a), new_sources.get(b)) {
                (Some(left), Some(right)) if left == right => {
                    a += 1;
                    b += 1;
                }
                (Some(left), Some(right)) if left.0 == right.0 => {
                    add(left.0);
                    a += 1;
                    b += 1;
                }
                (Some(left), Some(right)) if left.0 < right.0 => {
                    add(left.0);
                    a += 1;
                }
                (Some(_), Some(right)) => {
                    add(right.0);
                    b += 1;
                }
                (Some(left), None) => {
                    add(left.0);
                    a += 1;
                }
                (None, Some(right)) => {
                    add(right.0);
                    b += 1;
                }
                (None, None) => break,
            }
        }
    }
    found
}
