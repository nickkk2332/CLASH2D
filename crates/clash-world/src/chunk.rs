//! [`Chunk`]: the unit of cell storage and caching (02 §6.2).

use clash_core::{StateHash, StateHasher, Tick, VarMap, define_id};
use serde::{Deserialize, Serialize};

use crate::cause::Cause;
use crate::defs::{
    BlockState, CellFlags, ElementState, FixtureState, FloorState, Layer, WorldDefs,
};
use crate::dims::CHUNK_CELLS;
use crate::pos::CellIdx;
use crate::storage::{CellBits, Paletted, SparseCells};

define_id! {
    /// A `field` definition (02 §16). Fields are built by package W16.
    pub struct FieldId(u16);
    /// A `conduit` definition (16 §6). Conduits are built by the devices
    /// plan.
    pub struct ConduitId(u16);
    /// A `decal` definition: a kind of surface mark.
    pub struct DecalId(u16);
}

/// The most damage an element can hold and still stand, in 255ths of its
/// capacity (02 §12.2).
pub const DAMAGE_MAX: u8 = 254;

/// The conduits passing through one cell: a small sorted set (16 §6).
/// Stored only; the devices plan gives it behaviour.
#[derive(Clone, PartialEq, Eq, Default, Debug, Serialize, Deserialize)]
pub struct ConduitSet(pub Vec<ConduitId>);

impl StateHash for ConduitSet {
    fn state_hash(&self, hasher: &mut StateHasher) {
        self.0.state_hash(hasher);
    }
}

/// A surface mark with its provenance: blood, scorch, footprints.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub struct Decal {
    /// What kind of mark.
    pub kind: DecalId,
    /// How much of it, in the kind's own terms.
    pub amount: u8,
    /// Who and what left it.
    pub cause: Cause,
    /// When it was left, in universal time.
    pub tick: Tick,
}

impl StateHash for Decal {
    fn state_hash(&self, hasher: &mut StateHasher) {
        self.kind.state_hash(hasher);
        hasher.write_u8(self.amount);
        self.cause.state_hash(hasher);
        self.tick.state_hash(hasher);
    }
}

/// The marks in one cell, oldest first.
pub type DecalList = Vec<Decal>;

/// A chunk's fields. A typed placeholder: package W16 builds field
/// storage. It holds nothing and every field reads as zero.
#[derive(Clone, PartialEq, Eq, Default, Debug)]
pub struct FieldStore {}

impl StateHash for FieldStore {
    fn state_hash(&self, _hasher: &mut StateHasher) {}
}

pub use crate::occupancy::OccupantIndex;

/// Counters that tell every cache outside a chunk whether it is stale.
/// Each wraps round on overflow; compare for equality, not order.
#[derive(Clone, Copy, PartialEq, Eq, Default, Debug)]
pub struct ChunkRev {
    /// Bumped by every change to a floor, block or fixture.
    pub structure: u32,
    /// Bumped by every change to a field.
    pub fields: u32,
    /// Bumped by every change to who occupies the chunk's cells.
    pub occupants: u32,
}

/// The derived flags of every cell of a chunk, or a single value when
/// every cell agrees.
///
/// The flags of one cell are the bits of one byte, so that reading all of
/// a cell's flags is one load. A scan for one flag tests one bit of each
/// byte.
#[derive(Clone, Debug)]
enum DerivedCache {
    /// Every cell has these flags. The form of a chunk whose three
    /// element layers are uniform, so that it costs nothing.
    Uniform(CellFlags),
    /// The flags of each cell, by cell index.
    Cells(Box<[CellFlags; CHUNK_CELLS]>),
}

impl DerivedCache {
    #[inline]
    fn flags(&self, idx: CellIdx) -> CellFlags {
        match self {
            DerivedCache::Uniform(flags) => *flags,
            DerivedCache::Cells(cells) => cells[idx.index()],
        }
    }

    fn set(&mut self, idx: CellIdx, flags: CellFlags) {
        match self {
            DerivedCache::Uniform(all) if *all == flags => {}
            DerivedCache::Uniform(all) => {
                let mut cells = Box::new([*all; CHUNK_CELLS]);
                cells[idx.index()] = flags;
                *self = DerivedCache::Cells(cells);
            }
            DerivedCache::Cells(cells) => cells[idx.index()] = flags,
        }
    }

    fn heap_bytes(&self) -> usize {
        match self {
            DerivedCache::Uniform(_) => 0,
            DerivedCache::Cells(_) => core::mem::size_of::<[CellFlags; CHUNK_CELLS]>(),
        }
    }
}

/// The sparse layers of a chunk, kept together off to one side because
/// most chunks have none.
#[derive(Clone, PartialEq, Eq, Default, Debug)]
struct SparseLayers {
    damage: SparseCells<[u8; 3]>,
    conduits: SparseCells<ConduitSet>,
    decals: SparseCells<DecalList>,
    vars: SparseCells<VarMap>,
}

impl SparseLayers {
    fn is_empty(&self) -> bool {
        self.damage.is_empty()
            && self.conduits.is_empty()
            && self.decals.is_empty()
            && self.vars.is_empty()
    }
}

/// 32 × 32 × 8 cells: three paletted element layers, the sparse layers,
/// and the caches derived from them.
///
/// Reading is public. Changing a chunk that is part of the world is not:
/// cells change only through `WorldEdit` (01 R3). A chunk that is not yet
/// part of the world is built through a
/// [`ColumnBuilder`](crate::ColumnBuilder).
#[derive(Clone, Debug)]
pub struct Chunk {
    floors: Paletted<FloorState>,
    blocks: Paletted<BlockState>,
    fixtures: Paletted<FixtureState>,
    /// Damage, conduits, decals and cell variables. `None` when all four
    /// are empty.
    sparse: Option<Box<SparseLayers>>,
    fields: FieldStore,
    occupants: OccupantIndex,
    derived: DerivedCache,
    rev: ChunkRev,
    /// Differs from what is on disk.
    dirty: bool,
}

impl Chunk {
    /// A chunk with one block state in every cell and nothing else.
    pub fn uniform(defs: &WorldDefs, block: BlockState) -> Chunk {
        Chunk {
            floors: Paletted::uniform(FloorState::NONE),
            blocks: Paletted::uniform(block),
            fixtures: Paletted::uniform(FixtureState::NONE),
            sparse: None,
            fields: FieldStore::default(),
            occupants: OccupantIndex::default(),
            derived: DerivedCache::Uniform(defs.cell_flags(
                FloorState::NONE,
                block,
                FixtureState::NONE,
            )),
            rev: ChunkRev::default(),
            dirty: false,
        }
    }

    /// The floor of a cell.
    #[inline]
    pub fn floor(&self, idx: CellIdx) -> FloorState {
        self.floors.get(idx)
    }

    /// The block of a cell.
    #[inline]
    pub fn block(&self, idx: CellIdx) -> BlockState {
        self.blocks.get(idx)
    }

    /// The fixture of a cell.
    #[inline]
    pub fn fixture(&self, idx: CellIdx) -> FixtureState {
        self.fixtures.get(idx)
    }

    /// The element of a cell on a layer.
    pub fn element(&self, layer: Layer, idx: CellIdx) -> ElementState {
        match layer {
            Layer::Floor => self.floor(idx).raw(),
            Layer::Block => self.block(idx).raw(),
            Layer::Fixture => self.fixture(idx).raw(),
        }
    }

    /// Whether any state a layer's palette holds passes a test. A palette
    /// may hold states no cell uses until the chunk is compacted, so
    /// this can say yes where a scan of the cells would say no, and
    /// never the other way round: a quick way to rule a chunk out.
    pub fn any_state(&self, layer: Layer, mut test: impl FnMut(ElementState) -> bool) -> bool {
        match layer {
            Layer::Floor => self.floors.palette().iter().any(|state| test(state.raw())),
            Layer::Block => self.blocks.palette().iter().any(|state| test(state.raw())),
            Layer::Fixture => self
                .fixtures
                .palette()
                .iter()
                .any(|state| test(state.raw())),
        }
    }

    /// The derived flags of a cell, from the chunk's bitsets.
    #[inline]
    pub fn flags(&self, idx: CellIdx) -> CellFlags {
        self.derived.flags(idx)
    }

    /// The derived flags of every cell, by cell index, for inner loops
    /// that scan many cells. `None` when every cell of the chunk agrees,
    /// in which case [`uniform_flags`](Chunk::uniform_flags) gives the
    /// answer for all.
    pub fn flag_cells(&self) -> Option<&[CellFlags; CHUNK_CELLS]> {
        match &self.derived {
            DerivedCache::Uniform(_) => None,
            DerivedCache::Cells(cells) => Some(cells),
        }
    }

    /// The cells of the chunk that have every flag of `flags`, as a
    /// bitset.
    pub fn cells_with(&self, flags: CellFlags) -> CellBits {
        match &self.derived {
            DerivedCache::Uniform(all) => CellBits::filled(all.contains(flags)),
            DerivedCache::Cells(cells) => {
                let mut bits = CellBits::new();
                for idx in CellIdx::all() {
                    if cells[idx.index()].contains(flags) {
                        bits.set(idx, true);
                    }
                }
                bits
            }
        }
    }

    /// The derived flags every cell of the chunk has, when they all agree
    /// and the chunk keeps no per-cell flags.
    pub fn uniform_flags(&self) -> Option<CellFlags> {
        match &self.derived {
            DerivedCache::Uniform(flags) => Some(*flags),
            DerivedCache::Cells(_) => None,
        }
    }

    /// Whether a cell holds no floor, no block and no fixture.
    #[inline]
    pub fn is_cell_empty(&self, idx: CellIdx) -> bool {
        self.floors.get(idx).is_none()
            && self.blocks.get(idx).is_none()
            && self.fixtures.get(idx).is_none()
    }

    /// Whether the chunk is stored as holding nothing at all in any cell.
    /// A chunk emptied cell by cell says so only once it is compacted.
    pub fn is_stored_empty(&self) -> bool {
        self.floors.as_uniform() == Some(FloorState::NONE)
            && self.blocks.as_uniform() == Some(BlockState::NONE)
            && self.fixtures.as_uniform() == Some(FixtureState::NONE)
    }

    /// The accumulated damage of a cell's floor, block and fixture, each
    /// in 255ths of its capacity.
    pub fn damage(&self, idx: CellIdx) -> [u8; 3] {
        self.sparse
            .as_ref()
            .and_then(|sparse| sparse.damage.get(idx).copied())
            .unwrap_or_default()
    }

    /// The conduits passing through a cell.
    pub fn conduits(&self, idx: CellIdx) -> &[ConduitId] {
        self.sparse
            .as_ref()
            .and_then(|sparse| sparse.conduits.get(idx))
            .map_or(&[], |set| set.0.as_slice())
    }

    /// The marks in a cell, oldest first.
    pub fn decals(&self, idx: CellIdx) -> &[Decal] {
        self.sparse
            .as_ref()
            .and_then(|sparse| sparse.decals.get(idx))
            .map_or(&[], Vec::as_slice)
    }

    /// The mod variables on a cell, if it has any.
    pub fn vars(&self, idx: CellIdx) -> Option<&VarMap> {
        self.sparse.as_ref().and_then(|sparse| sparse.vars.get(idx))
    }

    /// The chunk's fields.
    pub fn fields(&self) -> &FieldStore {
        &self.fields
    }

    /// The chunk's occupant index.
    pub fn occupants(&self) -> &OccupantIndex {
        &self.occupants
    }

    /// The chunk's revision counters.
    pub fn rev(&self) -> ChunkRev {
        self.rev
    }

    /// Whether the chunk differs from what is on disk.
    pub fn is_dirty(&self) -> bool {
        self.dirty
    }

    /// The bytes the chunk takes in all.
    pub fn memory_bytes(&self) -> usize {
        let sparse = self.sparse.as_ref().map_or(0, |sparse| {
            core::mem::size_of::<SparseLayers>()
                + sparse.damage.heap_bytes()
                + sparse.conduits.heap_bytes()
                + sparse.decals.heap_bytes()
                + sparse.vars.heap_bytes()
        });
        core::mem::size_of::<Chunk>()
            + self.floors.heap_bytes()
            + self.blocks.heap_bytes()
            + self.fixtures.heap_bytes()
            + self.derived.heap_bytes()
            + self.occupants.heap_bytes()
            + sparse
    }

    /// The chunk's occupant index, to change. Bumps the occupant
    /// revision: whoever asks for this is about to change who is where.
    pub(crate) fn occupants_mut(&mut self) -> &mut OccupantIndex {
        self.rev.occupants = self.rev.occupants.wrapping_add(1);
        &mut self.occupants
    }

    /// Gives the chunk an occupant index made while the chunk had no
    /// storage.
    pub(crate) fn adopt_occupants(&mut self, index: OccupantIndex) {
        self.rev.occupants = self.rev.occupants.wrapping_add(1);
        self.occupants = index;
    }

    /// Whether the chunk holds exactly one block state in every cell and
    /// nothing else: what an implicit chunk of that fill would hold.
    pub(crate) fn is_pure(&self, block: BlockState) -> bool {
        self.sparse.as_ref().is_none_or(|sparse| sparse.is_empty())
            && self.floors == Paletted::uniform(FloorState::NONE)
            && self.fixtures == Paletted::uniform(FixtureState::NONE)
            && self.blocks == Paletted::uniform(block)
    }

    /// Notes a structural change to a cell: brings the derived flags up
    /// to date, bumps the revision and marks the chunk dirty.
    fn structure_changed(&mut self, defs: &WorldDefs, idx: CellIdx) {
        let flags = defs.cell_flags(
            self.floors.get(idx),
            self.blocks.get(idx),
            self.fixtures.get(idx),
        );
        self.derived.set(idx, flags);
        self.rev.structure = self.rev.structure.wrapping_add(1);
        self.dirty = true;
    }

    /// Sets a cell's floor and returns the one it had. A write that
    /// changes nothing bumps nothing.
    pub(crate) fn set_floor(
        &mut self,
        defs: &WorldDefs,
        idx: CellIdx,
        state: FloorState,
    ) -> FloorState {
        let old = self.floors.set(idx, state);
        if old != state {
            self.structure_changed(defs, idx);
        }
        old
    }

    /// Sets a cell's block and returns the one it had. A write that
    /// changes nothing bumps nothing.
    pub(crate) fn set_block(
        &mut self,
        defs: &WorldDefs,
        idx: CellIdx,
        state: BlockState,
    ) -> BlockState {
        let old = self.blocks.set(idx, state);
        if old != state {
            self.structure_changed(defs, idx);
        }
        old
    }

    /// Sets a cell's fixture and returns the one it had. A write that
    /// changes nothing bumps nothing.
    pub(crate) fn set_fixture(
        &mut self,
        defs: &WorldDefs,
        idx: CellIdx,
        state: FixtureState,
    ) -> FixtureState {
        let old = self.fixtures.set(idx, state);
        if old != state {
            self.structure_changed(defs, idx);
        }
        old
    }

    fn sparse_mut(&mut self) -> &mut SparseLayers {
        self.sparse.get_or_insert_with(Box::default)
    }

    fn drop_empty_sparse(&mut self) {
        if self.sparse.as_ref().is_some_and(|sparse| sparse.is_empty()) {
            self.sparse = None;
        }
    }

    /// Sets the accumulated damage of a cell's three elements and returns
    /// what it was.
    pub(crate) fn set_damage(&mut self, idx: CellIdx, damage: [u8; 3]) -> [u8; 3] {
        let old = self.damage(idx);
        if old == damage {
            return old;
        }
        if damage == [0; 3] {
            self.sparse_mut().damage.remove(idx);
            self.drop_empty_sparse();
        } else {
            self.sparse_mut().damage.insert(idx, damage);
        }
        self.dirty = true;
        old
    }

    /// Adds a mark to a cell.
    pub(crate) fn add_decal(&mut self, idx: CellIdx, decal: Decal) {
        self.sparse_mut()
            .decals
            .get_or_insert_with(idx, Vec::new)
            .push(decal);
        self.dirty = true;
    }

    /// Removes every mark of a kind from a cell and returns how many
    /// there were.
    pub(crate) fn remove_decals(&mut self, idx: CellIdx, kind: DecalId) -> usize {
        let Some(sparse) = self.sparse.as_mut() else {
            return 0;
        };
        let Some(list) = sparse.decals.get_mut(idx) else {
            return 0;
        };
        let before = list.len();
        list.retain(|decal| decal.kind != kind);
        let removed = before - list.len();
        if list.is_empty() {
            sparse.decals.remove(idx);
        }
        if removed > 0 {
            self.dirty = true;
            self.drop_empty_sparse();
        }
        removed
    }

    /// Sets or clears a mod variable on a cell and returns the value it
    /// had.
    pub(crate) fn set_var(
        &mut self,
        idx: CellIdx,
        key: clash_core::VarKey,
        value: Option<clash_core::Value>,
    ) -> Option<clash_core::Value> {
        let old = match value {
            Some(value) => {
                let vars = self.sparse_mut().vars.get_or_insert_with(idx, VarMap::new);
                let old = vars.get(key.as_str()).cloned();
                if old.as_ref() == Some(&value) {
                    return old;
                }
                vars.set(key, value);
                old
            }
            None => {
                let sparse = self.sparse.as_mut()?;
                let vars = sparse.vars.get_mut(idx)?;
                let old = vars.remove(key.as_str())?;
                if vars.is_empty() {
                    sparse.vars.remove(idx);
                }
                self.drop_empty_sparse();
                Some(old)
            }
        };
        self.dirty = true;
        old
    }

    /// Rebuilds the derived flags of every cell from the palettes.
    pub(crate) fn rebuild_derived(&mut self, defs: &WorldDefs) {
        if let (Some(floor), Some(block), Some(fixture)) = (
            self.floors.as_uniform(),
            self.blocks.as_uniform(),
            self.fixtures.as_uniform(),
        ) {
            self.derived = DerivedCache::Uniform(defs.cell_flags(floor, block, fixture));
            return;
        }
        let mut cells = Box::new([CellFlags::NONE; CHUNK_CELLS]);
        for idx in CellIdx::all() {
            cells[idx.index()] = defs.cell_flags(
                self.floors.get(idx),
                self.blocks.get(idx),
                self.fixtures.get(idx),
            );
        }
        self.derived = DerivedCache::Cells(cells);
    }

    /// Drops unused palette entries and returns layers and caches to
    /// their smallest form. What the cells hold does not change.
    pub(crate) fn compact(&mut self, defs: &WorldDefs) {
        self.floors.compact();
        self.blocks.compact();
        self.fixtures.compact();
        self.drop_empty_sparse();
        self.rebuild_derived(defs);
    }

    /// Marks the chunk as matching what is on disk, or what generation
    /// gives.
    pub(crate) fn clear_dirty(&mut self) {
        self.dirty = false;
    }

    /// Starts every revision counter at `base`: where the counters of a
    /// chunk begin when its tract is loaded, so that they continue from
    /// where no earlier loading left them (02 §22).
    pub(crate) fn set_rev_base(&mut self, base: u32) {
        self.rev = ChunkRev {
            structure: base,
            fields: base,
            occupants: base,
        };
    }

    /// What the chunk holds, in the form it is stored in.
    pub(crate) fn image(&self) -> ChunkImage {
        let sparse = self.sparse.as_deref().cloned().unwrap_or_default();
        ChunkImage {
            floors: self.floors.clone(),
            blocks: self.blocks.clone(),
            fixtures: self.fixtures.clone(),
            damage: sparse.damage,
            conduits: sparse.conduits,
            decals: sparse.decals,
            vars: sparse.vars,
        }
    }

    /// A chunk holding what an image says, with its caches built and its
    /// revisions at zero. It matches what is on disk.
    pub(crate) fn from_image(defs: &WorldDefs, image: ChunkImage) -> Chunk {
        let sparse = SparseLayers {
            damage: image.damage,
            conduits: image.conduits,
            decals: image.decals,
            vars: image.vars,
        };
        let mut chunk = Chunk {
            floors: image.floors,
            blocks: image.blocks,
            fixtures: image.fixtures,
            sparse: (!sparse.is_empty()).then(|| Box::new(sparse)),
            fields: FieldStore::default(),
            occupants: OccupantIndex::default(),
            derived: DerivedCache::Uniform(CellFlags::NONE),
            rev: ChunkRev::default(),
            dirty: false,
        };
        chunk.compact(defs);
        chunk
    }
}

/// What a chunk holds, in the form a tract is stored in (02 §11): the
/// three element layers with compacted palettes and the sparse layers.
/// Occupants, caches, revisions and the dirty flag are not stored.
/// Fields join it with package W16.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) struct ChunkImage {
    floors: Paletted<FloorState>,
    blocks: Paletted<BlockState>,
    fixtures: Paletted<FixtureState>,
    damage: SparseCells<[u8; 3]>,
    conduits: SparseCells<ConduitSet>,
    decals: SparseCells<DecalList>,
    vars: SparseCells<VarMap>,
}

/// Hashes what the cells hold: the three element layers, the sparse
/// layers and the fields. Occupants, caches, revisions and the dirty flag
/// are not state.
impl StateHash for Chunk {
    fn state_hash(&self, hasher: &mut StateHasher) {
        self.floors.state_hash(hasher);
        self.blocks.state_hash(hasher);
        self.fixtures.state_hash(hasher);
        match &self.sparse {
            Some(sparse) => {
                sparse.damage.state_hash(hasher);
                sparse.conduits.state_hash(hasher);
                sparse.decals.state_hash(hasher);
                sparse.vars.state_hash(hasher);
            }
            None => {
                // Four empty layers.
                for _ in 0..4 {
                    hasher.write_len(0);
                }
            }
        }
        self.fields.state_hash(hasher);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cause::CauseKind;
    use crate::defs::RawWorldDefs;
    use clash_core::{Hash128, Value, VarKey};

    /// Definitions holding nothing but the built-in none: enough for the
    /// sparse layers, which do not depend on what the elements are.
    fn defs() -> WorldDefs {
        WorldDefs::build(RawWorldDefs::default()).unwrap_or_else(|_| unreachable!())
    }

    fn decal(kind: u16) -> Decal {
        Decal {
            kind: DecalId::new(kind),
            amount: 3,
            cause: Cause::new(CauseKind::Debug),
            tick: Tick(5),
        }
    }

    #[test]
    fn sparse_layers_appear_and_vanish_with_their_contents() {
        let defs = defs();
        let mut chunk = Chunk::uniform(&defs, BlockState::NONE);
        let empty = Hash128::of(&chunk);
        let small = chunk.memory_bytes();
        let at = CellIdx::new(4, 5, 6);

        assert_eq!(chunk.set_damage(at, [0, 9, 0]), [0; 3]);
        assert!(chunk.is_dirty());
        assert_eq!(chunk.damage(at), [0, 9, 0]);
        assert_eq!(chunk.damage(CellIdx::new(0, 0, 0)), [0; 3]);
        assert_ne!(Hash128::of(&chunk), empty);
        assert!(chunk.memory_bytes() > small);

        chunk.add_decal(at, decal(1));
        chunk.add_decal(at, decal(2));
        chunk.add_decal(at, decal(1));
        assert_eq!(chunk.decals(at).len(), 3);
        assert_eq!(chunk.remove_decals(at, DecalId::new(1)), 2);
        assert_eq!(chunk.decals(at), [decal(2)]);
        assert_eq!(chunk.remove_decals(at, DecalId::new(7)), 0);

        let key = VarKey::new("core:mark");
        assert_eq!(chunk.set_var(at, key.clone(), Some(Value::from(4))), None);
        assert_eq!(
            chunk.vars(at).and_then(|vars| vars.get("core:mark")),
            Some(&Value::from(4))
        );
        assert_eq!(
            chunk.set_var(at, key.clone(), Some(Value::from(5))),
            Some(Value::from(4))
        );

        // Taking everything away again leaves the chunk as it began.
        assert_eq!(chunk.set_var(at, key.clone(), None), Some(Value::from(5)));
        assert_eq!(chunk.set_var(at, key, None), None);
        assert_eq!(chunk.remove_decals(at, DecalId::new(2)), 1);
        assert_eq!(chunk.set_damage(at, [0; 3]), [0, 9, 0]);
        assert_eq!(Hash128::of(&chunk), empty);
        assert_eq!(chunk.memory_bytes(), small);
        assert!(chunk.vars(at).is_none());
        assert!(chunk.decals(at).is_empty());
        assert!(chunk.conduits(at).is_empty());
        // None of this is structure: the structure revision has not moved.
        assert_eq!(chunk.rev(), ChunkRev::default());
        chunk.clear_dirty();
        assert!(!chunk.is_dirty());
        // A write that changes nothing does not dirty the chunk.
        chunk.set_damage(at, [0; 3]);
        assert!(!chunk.is_dirty());
    }
}
