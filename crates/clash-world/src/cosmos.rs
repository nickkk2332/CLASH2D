//! [`Cosmos`]: every realm and every grid, and the read API all gameplay
//! goes through (02 §4, §7.1).

use std::collections::BTreeMap;
use std::sync::Arc;

use clash_core::{IVec3, Real, StateHash, StateHasher, Value, VarMap, define_id, impl_state_hash};
use clash_time::{ClockId, Timeline};
use serde::{Deserialize, Serialize};

use crate::chunk::{Decal, FieldId};
use crate::defs::{
    AtmosphereId, CellFlags, EngineFlag, RealmDefId, RealmKind, StateInfo, WorldDefs,
};
use crate::error::WorldError;
use crate::grid::{CellView, Column, Grid};
use crate::pos::{CellPos, GridId, Location, TractPos};
use crate::topology::Topology;

define_id! {
    /// A realm of the cosmos. Never reused.
    pub struct RealmId;
}

/// Whether a realm still exists.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RealmState {
    /// The realm exists.
    Active,
    /// The realm has been destroyed and awaits removal at the next save.
    Destroyed,
}

impl StateHash for RealmState {
    fn state_hash(&self, hasher: &mut StateHasher) {
        hasher.write_u8(*self as u8);
    }
}

/// One contiguous cell-scale space with its own rules (02 §4).
///
/// Region records, stamps, areas, sites, travelers and the realm plan
/// join this struct with the packages that build them.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Realm {
    /// The realm's id.
    pub id: RealmId,
    /// Its definition.
    pub def: RealmDefId,
    /// The seed its generation draws from.
    pub seed: u64,
    /// Its clock. Everything in the realm runs on it unless given another.
    pub clock: ClockId,
    /// Its base grid.
    pub base_grid: GridId,
    /// Realm-scope variables.
    pub vars: VarMap,
    /// Whether it still exists.
    pub state: RealmState,
}

impl_state_hash!(Realm {
    id,
    def,
    seed,
    clock,
    base_grid,
    vars,
    state
});

/// The ambient conditions of a cell: what its stratum says (02 §4.1).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Ambient {
    /// Gravity in m/s², along −z.
    pub gravity: Real,
    /// The atmosphere. `None` is vacuum.
    pub atmosphere: Option<AtmosphereId>,
    /// The clock things here run on.
    pub clock: ClockId,
}

/// A grid and what the cosmos knows about it.
#[derive(Clone, Debug)]
struct GridSlot {
    grid: Grid,
    /// The realm the grid belongs to.
    realm: RealmId,
    /// The definition of that realm.
    realm_def: RealmDefId,
    /// The realm's clock.
    clock: ClockId,
    topology: Topology,
}

impl StateHash for GridSlot {
    fn state_hash(&self, hasher: &mut StateHasher) {
        self.realm.state_hash(hasher);
        self.grid.state_hash(hasher);
    }
}

// The refusals are built out of line, to keep the reads that succeed
// short.

#[cold]
fn outside(grid: GridId, cell: CellPos, edge: crate::topology::Edge) -> WorldError {
    WorldError::OutsideRealm {
        loc: Location::new(grid, cell),
        edge,
    }
}

#[cold]
fn not_loaded(grid: GridId, cell: CellPos) -> WorldError {
    WorldError::NotLoaded(Location::new(grid, cell))
}

/// A read-only handle on one grid, for code that reads many cells of it:
/// it saves looking the grid up for every cell. All reads normalise the
/// position first, and reading an unloaded cell is an error value.
#[derive(Clone, Copy, Debug)]
pub struct GridView<'a> {
    id: GridId,
    slot: &'a GridSlot,
    defs: &'a WorldDefs,
}

impl<'a> GridView<'a> {
    /// The grid's id.
    pub fn id(&self) -> GridId {
        self.id
    }

    /// The grid's shape.
    pub fn topology(&self) -> &'a Topology {
        &self.slot.topology
    }

    /// The realm the grid belongs to.
    pub fn realm(&self) -> RealmId {
        self.slot.realm
    }

    /// The loaded tracts, row by row.
    pub fn loaded_tracts(&self) -> Vec<TractPos> {
        self.slot.grid.loaded_tracts()
    }

    /// The column of a loaded tract.
    pub fn column(&self, tract: TractPos) -> Option<&'a Column> {
        self.slot.grid.column(tract)
    }

    /// A position brought inside the grid, or the edge it lies beyond.
    #[inline]
    pub fn normalize(&self, cell: CellPos) -> Result<CellPos, WorldError> {
        match self.slot.topology.normalize(cell) {
            Ok(cell) => Ok(cell),
            Err(edge) => Err(outside(self.id, cell, edge)),
        }
    }

    /// The column that holds a cell, and the cell normalised.
    #[inline]
    fn locate(&self, cell: CellPos) -> Result<(&'a Column, CellPos), WorldError> {
        let cell = self.normalize(cell)?;
        match self.slot.grid.column(cell.tract()) {
            Some(column) => Ok((column, cell)),
            None => Err(not_loaded(self.id, cell)),
        }
    }

    /// Whether a cell is inside the grid and its tract is loaded.
    pub fn is_loaded(&self, cell: CellPos) -> bool {
        self.locate(cell).is_ok()
    }

    /// The states of a cell's three elements.
    #[inline]
    pub fn cell(&self, cell: CellPos) -> Result<CellView, WorldError> {
        let (column, cell) = self.locate(cell)?;
        Ok(column.cell(cell))
    }

    /// The derived flags of a cell: the fast path, from bitsets.
    #[inline]
    pub fn flags(&self, cell: CellPos) -> Result<CellFlags, WorldError> {
        let (column, cell) = self.locate(cell)?;
        Ok(column.flags(cell))
    }

    /// Whether a cell can be stood in (02 §5.2).
    pub fn standable(&self, cell: CellPos) -> Result<bool, WorldError> {
        let (column, cell) = self.locate(cell)?;
        Ok(column.standable(cell))
    }

    /// Whether a cell is open: it holds nothing and cannot be stood in.
    pub fn is_open(&self, cell: CellPos) -> Result<bool, WorldError> {
        let (column, cell) = self.locate(cell)?;
        Ok(column.is_open(cell))
    }

    /// The height in metres of the tallest block or fixture in a cell
    /// that is in the way: one not flagged `passable`.
    pub fn obstacle_height(&self, cell: CellPos) -> Result<Real, WorldError> {
        let view = self.cell(cell)?;
        let height = |info: &StateInfo| {
            if info.flags.has(EngineFlag::Passable) {
                Real::ZERO
            } else {
                info.height
            }
        };
        let block = height(self.defs.block_info(view.block));
        let fixture = height(self.defs.fixture_info(view.fixture));
        Ok(block.max(fixture))
    }

    /// The highest storey of a cell's column that is not open. See
    /// [`Column::height`].
    pub fn height(&self, cell: CellPos) -> Result<i32, WorldError> {
        let (column, cell) = self.locate(cell)?;
        Ok(column.height_at(cell))
    }

    /// Whether nothing stands between a cell and the sky above it.
    pub fn under_open_sky(&self, cell: CellPos) -> Result<bool, WorldError> {
        let (column, cell) = self.locate(cell)?;
        Ok(cell.z >= column.height_at(cell))
    }

    /// The accumulated damage of a cell's floor, block and fixture, each
    /// in 255ths of its capacity.
    pub fn damage(&self, cell: CellPos) -> Result<[u8; 3], WorldError> {
        let (column, cell) = self.locate(cell)?;
        Ok(column
            .chunk(cell.chunk().z)
            .map_or([0; 3], |chunk| chunk.damage(cell.idx())))
    }

    /// The marks in a cell, oldest first.
    pub fn decals(&self, cell: CellPos) -> Result<&'a [Decal], WorldError> {
        let (column, cell) = self.locate(cell)?;
        Ok(column
            .chunk(cell.chunk().z)
            .map_or(&[], |chunk| chunk.decals(cell.idx())))
    }

    /// A mod variable on a cell.
    pub fn var(&self, cell: CellPos, key: &str) -> Result<Option<&'a Value>, WorldError> {
        let (column, cell) = self.locate(cell)?;
        Ok(column
            .chunk(cell.chunk().z)
            .and_then(|chunk| chunk.vars(cell.idx()))
            .and_then(|vars| vars.get(key)))
    }

    /// The level of a field in a cell, 0 to 255. Fields are built by
    /// package W16: until then every field reads as zero.
    pub fn field(&self, cell: CellPos, _field: FieldId) -> Result<u8, WorldError> {
        self.locate(cell).map(|_| 0)
    }

    /// The ambient conditions of a cell. The cell must be inside the
    /// grid; its tract need not be loaded.
    pub fn ambient(&self, cell: CellPos) -> Result<Ambient, WorldError> {
        let cell = self.normalize(cell)?;
        let stratum = self
            .defs
            .realms()
            .get(self.slot.realm_def)
            .and_then(|kind| kind.stratum_at(cell.z));
        Ok(Ambient {
            gravity: stratum.map_or(Real::ZERO, |stratum| stratum.gravity),
            atmosphere: stratum.and_then(|stratum| stratum.atmosphere),
            clock: self.slot.clock,
        })
    }
}

/// All realms and grids, and what connects them (02 §4).
///
/// Charts, tickets, bodies and portals join this struct with the
/// packages that build them.
#[derive(Clone, Debug)]
pub struct Cosmos {
    defs: Arc<WorldDefs>,
    realms: BTreeMap<RealmId, Realm>,
    /// Base grids and subgrids, indexed by grid id. Ids are never reused,
    /// so the slot of a grid that has gone stays empty. A dense table
    /// spares every read of a cell a hash lookup.
    grids: Vec<Option<Box<GridSlot>>>,
    next_realm: u32,
    vars: VarMap,
}

impl Cosmos {
    /// An empty cosmos over a set of definitions.
    pub fn new(defs: Arc<WorldDefs>) -> Cosmos {
        Cosmos {
            defs,
            realms: BTreeMap::new(),
            grids: Vec::new(),
            next_realm: 0,
            vars: VarMap::new(),
        }
    }

    #[inline]
    fn slot(&self, grid: GridId) -> Option<&GridSlot> {
        self.grids.get(grid.index())?.as_deref()
    }

    fn slot_mut(&mut self, grid: GridId) -> Option<&mut GridSlot> {
        self.grids.get_mut(grid.index())?.as_deref_mut()
    }

    /// The definitions the cosmos is made from.
    pub fn defs(&self) -> &Arc<WorldDefs> {
        &self.defs
    }

    /// Creates a realm from a definition, with a clock of its own under
    /// universal time at the definition's rate, and an empty base grid.
    pub fn create_realm(
        &mut self,
        timeline: &mut Timeline,
        def: RealmDefId,
        seed: u64,
    ) -> Result<RealmId, WorldError> {
        let defs = Arc::clone(&self.defs);
        let kind: &RealmKind = defs
            .realms()
            .get(def)
            .ok_or(WorldError::UnknownRealmDef(def))?;
        let next_realm = self
            .next_realm
            .checked_add(1)
            .ok_or(WorldError::IdsExhausted("realm"))?;
        let base_grid =
            GridId::from_index(self.grids.len()).ok_or(WorldError::IdsExhausted("grid"))?;
        let clock = timeline.create_clock(ClockId::UNIVERSAL, kind.time_rate)?;
        let id = RealmId::new(self.next_realm);
        self.next_realm = next_realm;
        self.grids.push(Some(Box::new(GridSlot {
            grid: Grid::new(),
            realm: id,
            realm_def: def,
            clock,
            topology: kind.topology,
        })));
        self.realms.insert(
            id,
            Realm {
                id,
                def,
                seed,
                clock,
                base_grid,
                vars: VarMap::new(),
                state: RealmState::Active,
            },
        );
        Ok(id)
    }

    /// A realm by id.
    pub fn realm(&self, id: RealmId) -> Option<&Realm> {
        self.realms.get(&id)
    }

    /// Every realm, in id order.
    pub fn realms(&self) -> impl Iterator<Item = &Realm> + '_ {
        self.realms.values()
    }

    /// The definition of a realm.
    pub fn realm_kind(&self, id: RealmId) -> Option<&RealmKind> {
        self.defs.realms().get(self.realm(id)?.def)
    }

    /// The realm a grid belongs to.
    pub fn realm_of(&self, grid: GridId) -> Option<RealmId> {
        self.slot(grid).map(|slot| slot.realm)
    }

    /// Global variables.
    pub fn vars(&self) -> &VarMap {
        &self.vars
    }

    /// A read-only handle on one grid, for reading many cells of it.
    #[inline]
    pub fn grid(&self, grid: GridId) -> Result<GridView<'_>, WorldError> {
        match self.slot(grid) {
            Some(slot) => Ok(GridView {
                id: grid,
                slot,
                defs: &self.defs,
            }),
            None => Err(WorldError::UnknownGrid(grid)),
        }
    }

    /// The shape of a grid.
    pub fn topology(&self, grid: GridId) -> Result<&Topology, WorldError> {
        self.grid(grid).map(|view| view.topology())
    }

    /// A location with its cell brought inside its grid: wrapped on
    /// wrapped axes, or an error naming the edge it lies beyond.
    pub fn normalize(&self, loc: Location) -> Result<Location, WorldError> {
        let cell = self.grid(loc.grid)?.normalize(loc.cell)?;
        Ok(Location::new(loc.grid, cell))
    }

    /// The shortest offset from one location to another in the same grid,
    /// respecting wrap. `None` if they are in different grids or the grid
    /// is unknown.
    pub fn delta(&self, from: Location, to: Location) -> Option<IVec3> {
        if from.grid != to.grid {
            return None;
        }
        let topology = self.topology(from.grid).ok()?;
        Some(topology.delta(from.cell, to.cell))
    }

    /// Whether a location is inside its grid and its tract is loaded.
    pub fn is_loaded(&self, loc: Location) -> bool {
        self.grid(loc.grid)
            .is_ok_and(|view| view.is_loaded(loc.cell))
    }

    /// The states of a cell's three elements.
    #[inline]
    pub fn cell(&self, loc: Location) -> Result<CellView, WorldError> {
        self.grid(loc.grid)?.cell(loc.cell)
    }

    /// The derived flags of a cell: the fast path, from bitsets.
    #[inline]
    pub fn flags(&self, loc: Location) -> Result<CellFlags, WorldError> {
        self.grid(loc.grid)?.flags(loc.cell)
    }

    /// Whether a cell can be stood in: it has a floor, or the cell below
    /// has a block with a solid top (02 §5.2).
    pub fn standable(&self, loc: Location) -> Result<bool, WorldError> {
        self.grid(loc.grid)?.standable(loc.cell)
    }

    /// Whether a cell is open: it holds nothing and cannot be stood in.
    pub fn is_open(&self, loc: Location) -> Result<bool, WorldError> {
        self.grid(loc.grid)?.is_open(loc.cell)
    }

    /// The height in metres of the tallest block or fixture in a cell
    /// that is in the way.
    pub fn obstacle_height(&self, loc: Location) -> Result<Real, WorldError> {
        self.grid(loc.grid)?.obstacle_height(loc.cell)
    }

    /// The highest storey of a cell's column that is not open.
    pub fn height(&self, loc: Location) -> Result<i32, WorldError> {
        self.grid(loc.grid)?.height(loc.cell)
    }

    /// Whether nothing stands between a cell and the sky above it.
    pub fn under_open_sky(&self, loc: Location) -> Result<bool, WorldError> {
        self.grid(loc.grid)?.under_open_sky(loc.cell)
    }

    /// The accumulated damage of a cell's floor, block and fixture.
    pub fn damage(&self, loc: Location) -> Result<[u8; 3], WorldError> {
        self.grid(loc.grid)?.damage(loc.cell)
    }

    /// The marks in a cell, oldest first.
    pub fn decals(&self, loc: Location) -> Result<&[Decal], WorldError> {
        self.grid(loc.grid)?.decals(loc.cell)
    }

    /// A mod variable on a cell.
    pub fn var(&self, loc: Location, key: &str) -> Result<Option<&Value>, WorldError> {
        self.grid(loc.grid)?.var(loc.cell, key)
    }

    /// The level of a field in a cell. Zero until fields are built.
    pub fn field(&self, loc: Location, field: FieldId) -> Result<u8, WorldError> {
        self.grid(loc.grid)?.field(loc.cell, field)
    }

    /// The ambient conditions of a cell: gravity, atmosphere and clock.
    /// The cell must be inside its grid; its tract need not be loaded.
    pub fn ambient(&self, loc: Location) -> Result<Ambient, WorldError> {
        self.grid(loc.grid)?.ambient(loc.cell)
    }

    /// The column of a loaded tract, to change. Crate-private: cells
    /// change only through `WorldEdit`.
    #[allow(dead_code, reason = "a write primitive for WorldEdit, package W6")]
    pub(crate) fn column_mut(&mut self, grid: GridId, tract: TractPos) -> Option<&mut Column> {
        self.slot_mut(grid)?.grid.column_mut(tract)
    }

    /// Makes a tract of a grid loaded with these cells, and returns the
    /// column it replaced, if any.
    pub(crate) fn insert_column(
        &mut self,
        grid: GridId,
        tract: TractPos,
        column: Column,
    ) -> Result<Option<Column>, WorldError> {
        let slot = self.slot_mut(grid).ok_or(WorldError::UnknownGrid(grid))?;
        Ok(slot.grid.insert_column(tract, column))
    }

    /// Unloads a tract of a grid and returns its column.
    pub(crate) fn remove_column(&mut self, grid: GridId, tract: TractPos) -> Option<Column> {
        self.slot_mut(grid)?.grid.remove_column(tract)
    }
}

/// Hashes the realms in id order, then the grids in id order with their
/// loaded columns in tract order, then the global variables.
impl StateHash for Cosmos {
    fn state_hash(&self, hasher: &mut StateHasher) {
        self.realms.state_hash(hasher);
        hasher.write_len(self.grids.len());
        for slot in &self.grids {
            slot.as_deref().state_hash(hasher);
        }
        hasher.write_u32(self.next_realm);
        self.vars.state_hash(hasher);
    }
}
