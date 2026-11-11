//! [`Column`] and [`Grid`]: chunks stacked into tracts, and tracts
//! gathered into a grid (02 §6.3).

use clash_core::{LookupMap, StateHash, StateHasher};

use crate::chunk::Chunk;
use crate::defs::{BlockState, CellFlags, FixtureState, FloorState, WorldDefs};
use crate::dims::{CHUNK_AREA, CHUNK_XY, CHUNK_Z_BITS, CHUNK_Z_MASK};
use crate::pos::{CellIdx, CellPos, TractPos};

/// The height of a cell column in which every cell is open.
pub const HEIGHT_NONE: i32 = i32::MIN;

/// The height of a cell column that is filled all the way up, because the
/// fill above the stored range is not empty.
pub const HEIGHT_FULL: i32 = i32::MAX;

/// The states of a cell's three elements.
#[derive(Clone, Copy, PartialEq, Eq, Default, Debug)]
pub struct CellView {
    /// The surface at the base of the cell.
    pub floor: FloorState,
    /// What occupies the cell's volume.
    pub block: BlockState,
    /// A static prop standing in the cell.
    pub fixture: FixtureState,
}

impl CellView {
    /// Whether the cell holds no floor, no block and no fixture.
    pub const fn is_empty(self) -> bool {
        self.floor.is_none() && self.block.is_none() && self.fixture.is_none()
    }
}

/// One tract's cells: a run of chunks stacked over a range of heights,
/// with implicit content below and above it.
///
/// Reading outside the stored range returns the fill without allocating.
/// Writing there creates storage. The heightmap is kept up to date by
/// every structural write.
///
/// A column's cells are addressed by [`CellPos`]; only the position
/// inside the tract's footprint matters, so the same position can be
/// used whichever tract the column is.
#[derive(Clone, Debug)]
pub struct Column {
    /// The chunk height of `chunks[0]`. For a column with no storage, the
    /// chunk height at which the fill above begins.
    z_lo: i32,
    chunks: Vec<Chunk>,
    below_fill: BlockState,
    above_fill: BlockState,
    /// The derived flags of a cell of each fill.
    below_flags: CellFlags,
    above_flags: CellFlags,
    /// Per cell column: the highest storey that is not open.
    heightmap: Box<[i32; CHUNK_AREA]>,
}

impl Column {
    /// A column with no storage: `below_fill` in every storey below
    /// `surface_z`, and `above_fill` from there up. If `surface_z` is not
    /// at the base of a chunk, the chunk it falls in is stored.
    pub(crate) fn new(
        defs: &WorldDefs,
        below_fill: BlockState,
        above_fill: BlockState,
        surface_z: i32,
    ) -> Column {
        let mut column = Column {
            z_lo: surface_z >> CHUNK_Z_BITS,
            chunks: Vec::new(),
            below_fill,
            above_fill,
            below_flags: defs.cell_flags(FloorState::NONE, below_fill, FixtureState::NONE),
            above_flags: defs.cell_flags(FloorState::NONE, above_fill, FixtureState::NONE),
            heightmap: Box::new([HEIGHT_NONE; CHUNK_AREA]),
        };
        let chunk_base = column.z_lo << CHUNK_Z_BITS;
        if surface_z != chunk_base && below_fill != above_fill {
            let mut chunk = Chunk::uniform(defs, above_fill);
            for idx in CellIdx::all() {
                if chunk_base + idx.lz() < surface_z {
                    chunk.set_block(defs, idx, below_fill);
                }
            }
            column.chunks.push(chunk);
        }
        column.recompute_heightmap();
        column.clear_dirty();
        column
    }

    /// The range of chunk heights that have storage, lowest and highest,
    /// or `None` if nothing is stored.
    pub fn stored_range(&self) -> Option<(i32, i32)> {
        let count = i32::try_from(self.chunks.len()).ok()?;
        (count > 0).then(|| (self.z_lo, self.z_lo + count - 1))
    }

    /// The chunk at a chunk height, if it is stored.
    #[inline]
    pub fn chunk(&self, chunk_z: i32) -> Option<&Chunk> {
        let at = usize::try_from(i64::from(chunk_z) - i64::from(self.z_lo)).ok()?;
        self.chunks.get(at)
    }

    /// The stored chunks with their chunk heights, lowest first.
    pub fn chunks(&self) -> impl Iterator<Item = (i32, &Chunk)> + '_ {
        (self.z_lo..).zip(self.chunks.iter())
    }

    /// What implicit cells hold below the stored range.
    pub fn below_fill(&self) -> BlockState {
        self.below_fill
    }

    /// What implicit cells hold above the stored range.
    pub fn above_fill(&self) -> BlockState {
        self.above_fill
    }

    /// Whether a chunk height lies below the stored range.
    #[inline]
    fn is_below(&self, chunk_z: i32) -> bool {
        chunk_z < self.z_lo
    }

    /// The fill that stands in for an unstored chunk height.
    #[inline]
    fn fill_at(&self, chunk_z: i32) -> BlockState {
        if self.is_below(chunk_z) {
            self.below_fill
        } else {
            self.above_fill
        }
    }

    /// The block of a cell.
    #[inline]
    pub fn block(&self, cell: CellPos) -> BlockState {
        let chunk_z = cell.z >> CHUNK_Z_BITS;
        match self.chunk(chunk_z) {
            Some(chunk) => chunk.block(cell.idx()),
            None => self.fill_at(chunk_z),
        }
    }

    /// The floor of a cell.
    #[inline]
    pub fn floor(&self, cell: CellPos) -> FloorState {
        match self.chunk(cell.z >> CHUNK_Z_BITS) {
            Some(chunk) => chunk.floor(cell.idx()),
            None => FloorState::NONE,
        }
    }

    /// The fixture of a cell.
    #[inline]
    pub fn fixture(&self, cell: CellPos) -> FixtureState {
        match self.chunk(cell.z >> CHUNK_Z_BITS) {
            Some(chunk) => chunk.fixture(cell.idx()),
            None => FixtureState::NONE,
        }
    }

    /// The states of a cell's three elements.
    pub fn cell(&self, cell: CellPos) -> CellView {
        let chunk_z = cell.z >> CHUNK_Z_BITS;
        match self.chunk(chunk_z) {
            Some(chunk) => {
                let idx = cell.idx();
                CellView {
                    floor: chunk.floor(idx),
                    block: chunk.block(idx),
                    fixture: chunk.fixture(idx),
                }
            }
            None => CellView {
                floor: FloorState::NONE,
                block: self.fill_at(chunk_z),
                fixture: FixtureState::NONE,
            },
        }
    }

    /// The derived flags of a cell.
    #[inline]
    pub fn flags(&self, cell: CellPos) -> CellFlags {
        let chunk_z = cell.z >> CHUNK_Z_BITS;
        match self.chunk(chunk_z) {
            Some(chunk) => chunk.flags(cell.idx()),
            None if self.is_below(chunk_z) => self.below_flags,
            None => self.above_flags,
        }
    }

    /// Whether a cell can be stood in: it has a floor, or the cell below
    /// has a block with a solid top (02 §5.2).
    #[inline]
    pub fn standable(&self, cell: CellPos) -> bool {
        self.flags(cell).contains(CellFlags::HAS_FLOOR)
            || self.flags(cell.below()).contains(CellFlags::SOLID_TOP)
    }

    /// Whether a cell is open: it holds nothing and cannot be stood in.
    /// Unsupported things in open cells fall.
    pub fn is_open(&self, cell: CellPos) -> bool {
        let chunk_z = cell.z >> CHUNK_Z_BITS;
        let empty = match self.chunk(chunk_z) {
            Some(chunk) => chunk.is_cell_empty(cell.idx()),
            None => self.fill_at(chunk_z).is_none(),
        };
        empty && !self.flags(cell.below()).contains(CellFlags::SOLID_TOP)
    }

    /// The highest storey of a cell column that is not open:
    /// [`HEIGHT_NONE`] if every cell is open, [`HEIGHT_FULL`] if the
    /// column is filled all the way up. A cell above this is under open
    /// sky.
    #[inline]
    pub fn height(&self, column: usize) -> i32 {
        self.heightmap[column & (CHUNK_AREA - 1)]
    }

    /// The highest storey of the cell column a cell is in that is not
    /// open. See [`height`](Column::height).
    #[inline]
    pub fn height_at(&self, cell: CellPos) -> i32 {
        self.heightmap[cell.column()]
    }

    /// The bytes the column takes in all.
    pub fn memory_bytes(&self) -> usize {
        core::mem::size_of::<Column>()
            + core::mem::size_of::<[i32; CHUNK_AREA]>()
            + (self.chunks.capacity() - self.chunks.len()) * core::mem::size_of::<Chunk>()
            + self
                .chunks
                .iter()
                .map(|chunk| chunk.memory_bytes())
                .sum::<usize>()
    }

    /// Whether any chunk differs from what is on disk.
    pub fn is_dirty(&self) -> bool {
        self.chunks.iter().any(|chunk| chunk.is_dirty())
    }

    /// The chunk at a chunk height, created along with any chunks between
    /// it and the stored range if it is not stored yet. `None` only if the
    /// height is too far from the stored range to index.
    pub(crate) fn chunk_mut(&mut self, defs: &WorldDefs, chunk_z: i32) -> Option<&mut Chunk> {
        if self.chunks.is_empty() {
            // The first chunk goes where the fills meet; the range then
            // grows from there to the height asked for.
            let fill = self.fill_at(self.z_lo);
            self.chunks.push(Chunk::uniform(defs, fill));
        }
        if chunk_z < self.z_lo {
            let missing = usize::try_from(i64::from(self.z_lo) - i64::from(chunk_z)).ok()?;
            let fill = self.below_fill;
            let new = (0..missing).map(|_| Chunk::uniform(defs, fill));
            self.chunks.splice(0..0, new);
            self.z_lo = chunk_z;
        }
        let at = usize::try_from(i64::from(chunk_z) - i64::from(self.z_lo)).ok()?;
        while self.chunks.len() <= at {
            self.chunks.push(Chunk::uniform(defs, self.above_fill));
        }
        self.chunks.get_mut(at)
    }

    /// Sets a cell's floor, creating storage if needed, and returns the
    /// floor it had.
    pub(crate) fn set_floor(
        &mut self,
        defs: &WorldDefs,
        cell: CellPos,
        state: FloorState,
    ) -> FloorState {
        let old = self.floor(cell);
        if old == state {
            return old;
        }
        if let Some(chunk) = self.chunk_mut(defs, cell.z >> CHUNK_Z_BITS) {
            chunk.set_floor(defs, cell.idx(), state);
            self.update_height(cell);
        }
        old
    }

    /// Sets a cell's block, creating storage if needed, and returns the
    /// block it had.
    pub(crate) fn set_block(
        &mut self,
        defs: &WorldDefs,
        cell: CellPos,
        state: BlockState,
    ) -> BlockState {
        let old = self.block(cell);
        if old == state {
            return old;
        }
        if let Some(chunk) = self.chunk_mut(defs, cell.z >> CHUNK_Z_BITS) {
            chunk.set_block(defs, cell.idx(), state);
            self.update_height(cell);
        }
        old
    }

    /// Sets a cell's fixture, creating storage if needed, and returns the
    /// fixture it had.
    pub(crate) fn set_fixture(
        &mut self,
        defs: &WorldDefs,
        cell: CellPos,
        state: FixtureState,
    ) -> FixtureState {
        let old = self.fixture(cell);
        if old == state {
            return old;
        }
        if let Some(chunk) = self.chunk_mut(defs, cell.z >> CHUNK_Z_BITS) {
            chunk.set_fixture(defs, cell.idx(), state);
            self.update_height(cell);
        }
        old
    }

    /// Brings the heightmap up to date after a structural write to a
    /// cell. The write can change whether that cell and the one above it
    /// are open, and nothing else.
    fn update_height(&mut self, cell: CellPos) {
        let column = cell.column();
        let height = self.heightmap[column];
        if height == HEIGHT_FULL {
            return;
        }
        let above = cell.above();
        let raised = if !self.is_open(above) {
            Some(above.z)
        } else if !self.is_open(cell) {
            Some(cell.z)
        } else {
            None
        };
        if let Some(z) = raised
            && z >= height
        {
            self.heightmap[column] = z;
        } else if height == cell.z || height == above.z {
            // The cell that set the height may have opened: look for the
            // next one down.
            self.heightmap[column] = self.scan_height(column, height);
        }
    }

    /// The highest storey at or below `from` of a cell column that is not
    /// open.
    fn scan_height(&self, column: usize, from: i32) -> i32 {
        // The lowest stored storey. Below it everything is the fill.
        let floor_z = self.z_lo << CHUNK_Z_BITS;
        let cell_at = |z: i32| {
            let idx = CellIdx::from_column(column, (z & CHUNK_Z_MASK) as u32);
            CellPos::new(idx.lx(), idx.ly(), z)
        };
        let mut z = from;
        while z >= floor_z {
            let chunk_z = z >> CHUNK_Z_BITS;
            let skip = match self.chunk(chunk_z) {
                Some(chunk) => chunk.is_stored_empty(),
                None => self.fill_at(chunk_z).is_none(),
            };
            if skip {
                // Nothing in this chunk. Only its lowest storey can be
                // other than open, by standing on the chunk below.
                z = chunk_z << CHUNK_Z_BITS;
            }
            if !self.is_open(cell_at(z)) {
                return z;
            }
            match z.checked_sub(1) {
                Some(next) => z = next,
                None => break,
            }
        }
        if self.below_fill.is_none() {
            HEIGHT_NONE
        } else {
            floor_z - 1
        }
    }

    /// Recomputes the whole heightmap from the cells.
    pub(crate) fn recompute_heightmap(&mut self) {
        if !self.above_fill.is_none() {
            self.heightmap.fill(HEIGHT_FULL);
            return;
        }
        // The first storey above the stored range can still be stood in.
        let count = i32::try_from(self.chunks.len()).unwrap_or(i32::MAX);
        let top = self.z_lo.saturating_add(count) << CHUNK_Z_BITS;
        for column in 0..CHUNK_AREA {
            self.heightmap[column] = self.scan_height(column, top);
        }
    }

    /// Returns every chunk to its smallest form and rebuilds the caches.
    /// What the cells hold does not change.
    pub(crate) fn compact(&mut self, defs: &WorldDefs) {
        for chunk in &mut self.chunks {
            chunk.compact(defs);
        }
        self.recompute_heightmap();
    }

    /// Marks every chunk as matching what is on disk, or what generation
    /// gives.
    pub(crate) fn clear_dirty(&mut self) {
        for chunk in &mut self.chunks {
            chunk.clear_dirty();
        }
    }

    /// The stored chunks that say something the fills do not: the stored
    /// range with pure-fill chunks trimmed from both ends, and the chunk
    /// height it starts at.
    fn significant(&self) -> (i32, &[Chunk]) {
        let below = self
            .chunks
            .iter()
            .take_while(|chunk| chunk.is_pure(self.below_fill))
            .count();
        let rest = &self.chunks[below..];
        let above = rest
            .iter()
            .rev()
            .take_while(|chunk| chunk.is_pure(self.above_fill))
            .count();
        let kept = &rest[..rest.len() - above];
        let start = self
            .z_lo
            .saturating_add(i32::try_from(below).unwrap_or(i32::MAX));
        if kept.is_empty() && self.below_fill == self.above_fill {
            // One fill everywhere: where the two meet means nothing.
            return (0, kept);
        }
        (start, kept)
    }
}

/// Hashes what the cells hold, not how much of it happens to be stored: a
/// stored chunk that holds exactly the fill hashes like an unstored one.
impl StateHash for Column {
    fn state_hash(&self, hasher: &mut StateHasher) {
        let (start, chunks) = self.significant();
        self.below_fill.state_hash(hasher);
        self.above_fill.state_hash(hasher);
        start.state_hash(hasher);
        hasher.write_len(chunks.len());
        for chunk in chunks {
            chunk.state_hash(hasher);
        }
    }
}

/// Builds the cells of one tract before it becomes part of the world.
///
/// This is how generation and tests make columns: start from the fills,
/// set cells, and [`build`](ColumnBuilder::build). Once a column is part
/// of a grid, its cells change only through `WorldEdit` (01 R3).
///
/// Cells are addressed by [`CellPos`]; only the position inside the
/// tract's footprint matters.
///
/// ```
/// use clash_world::{BlockState, CellPos, ColumnBuilder, RawWorldDefs, WorldDefs};
///
/// let defs = WorldDefs::build(RawWorldDefs::default()).unwrap();
/// let builder = ColumnBuilder::new(&defs, BlockState::NONE, BlockState::NONE, 0);
/// let column = builder.build();
/// assert!(column.cell(CellPos::new(3, 4, 5)).is_empty());
/// assert_eq!(column.stored_range(), None);
/// ```
#[derive(Debug)]
pub struct ColumnBuilder<'a> {
    defs: &'a WorldDefs,
    column: Column,
}

impl<'a> ColumnBuilder<'a> {
    /// Starts a column holding `below_fill` in every storey below
    /// `surface_z` and `above_fill` from there up.
    pub fn new(
        defs: &'a WorldDefs,
        below_fill: BlockState,
        above_fill: BlockState,
        surface_z: i32,
    ) -> ColumnBuilder<'a> {
        ColumnBuilder {
            defs,
            column: Column::new(defs, below_fill, above_fill, surface_z),
        }
    }

    /// Sets a cell's floor.
    pub fn set_floor(&mut self, cell: CellPos, state: FloorState) -> &mut Self {
        self.column.set_floor(self.defs, cell, state);
        self
    }

    /// Sets a cell's block.
    pub fn set_block(&mut self, cell: CellPos, state: BlockState) -> &mut Self {
        self.column.set_block(self.defs, cell, state);
        self
    }

    /// Sets a cell's fixture.
    pub fn set_fixture(&mut self, cell: CellPos, state: FixtureState) -> &mut Self {
        self.column.set_fixture(self.defs, cell, state);
        self
    }

    /// Sets the floor of every cell of one storey of the tract.
    pub fn fill_floor(&mut self, z: i32, state: FloorState) -> &mut Self {
        for y in 0..CHUNK_XY {
            for x in 0..CHUNK_XY {
                self.column
                    .set_floor(self.defs, CellPos::new(x, y, z), state);
            }
        }
        self
    }

    /// The column as built so far.
    pub fn column(&self) -> &Column {
        &self.column
    }

    /// Finishes the column: compacts it and marks it as unchanged.
    pub fn build(mut self) -> Column {
        self.column.compact(self.defs);
        self.column.clear_dirty();
        self.column
    }
}

/// A chunked container of cells: a realm's base grid, or a subgrid
/// (02 §6.3). It holds the columns of the tracts that are loaded.
#[derive(Clone, Debug, Default)]
pub struct Grid {
    columns: LookupMap<TractPos, Column>,
}

impl Grid {
    /// A grid with nothing loaded.
    pub fn new() -> Grid {
        Grid {
            columns: LookupMap::new(),
        }
    }

    /// The column of a tract, if it is loaded.
    #[inline]
    pub fn column(&self, tract: TractPos) -> Option<&Column> {
        self.columns.get(&tract)
    }

    /// Whether a tract is loaded.
    pub fn is_loaded(&self, tract: TractPos) -> bool {
        self.columns.contains_key(&tract)
    }

    /// How many tracts are loaded.
    pub fn loaded_count(&self) -> usize {
        self.columns.len()
    }

    /// The loaded tracts, row by row.
    pub fn loaded_tracts(&self) -> Vec<TractPos> {
        self.columns.sorted_keys()
    }

    /// The column of a tract, to change. Crate-private: cells change only
    /// through `WorldEdit`.
    #[allow(dead_code, reason = "a write primitive for WorldEdit, package W6")]
    pub(crate) fn column_mut(&mut self, tract: TractPos) -> Option<&mut Column> {
        self.columns.get_mut(&tract)
    }

    /// Makes a tract loaded with these cells, and returns the column it
    /// replaced, if any.
    pub(crate) fn insert_column(&mut self, tract: TractPos, column: Column) -> Option<Column> {
        self.columns.insert(tract, column)
    }

    /// Unloads a tract and returns its column.
    pub(crate) fn remove_column(&mut self, tract: TractPos) -> Option<Column> {
        self.columns.remove(&tract)
    }
}

/// Hashes the loaded columns in tract order.
impl StateHash for Grid {
    fn state_hash(&self, hasher: &mut StateHasher) {
        self.columns.state_hash(hasher);
    }
}
