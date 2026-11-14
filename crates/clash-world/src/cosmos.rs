//! [`Cosmos`]: every realm and every grid, and the read API all gameplay
//! goes through (02 §4, §7.1).

use std::collections::BTreeMap;
use std::sync::Arc;

use clash_core::{
    EntityId, IVec3, LookupMap, Real, StateHash, StateHasher, Value, VarMap, define_id,
    impl_state_hash,
};
use clash_time::{ClockId, Timeline};
use serde::{Deserialize, Serialize};

use crate::area::{AreaDrift, AreaId, AreaTable};
use crate::chunk::{Decal, FieldId};
use crate::defs::{
    AtmosphereId, CellFlags, ElementState, EngineFlag, Layer, RealmDefId, RealmKind, StateInfo,
    WorldDefs,
};
use crate::edit::StabilityQueue;
use crate::error::WorldError;
use crate::grid::{CellView, Column, Grid};
use crate::occupancy::OccupantRecord;
use crate::pos::{CellPos, GridId, Location, RegionPos, TractPos};
use crate::process::WorldActivity;
use crate::site::SiteTable;
use crate::ticket::TicketTable;
use crate::topology::Topology;
use crate::tract::RegionRecord;

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
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
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
    /// The records of the regions that have been surveyed, and through
    /// them of every tract the engine knows anything about (02 §9.1).
    pub regions: BTreeMap<RegionPos, RegionRecord>,
}

impl_state_hash!(Realm {
    id,
    def,
    seed,
    clock,
    base_grid,
    vars,
    state,
    regions
});

/// The ambient conditions of a cell: what its stratum says (02 §4.1),
/// with what the areas that cover it change (02 §8.3).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Ambient {
    /// Gravity in m/s², along −z: the stratum's, scaled or replaced by
    /// an area's override.
    pub gravity: Real,
    /// The atmosphere. `None` is vacuum.
    pub atmosphere: Option<AtmosphereId>,
    /// The clock things here run on: an area's if one that covers the
    /// cell has a clock, else the realm's.
    pub clock: ClockId,
    /// Gravity as an acceleration in m/s² east, south and up. Straight
    /// down at `gravity` unless an area gives it another direction.
    pub gravity_vector: [Real; 3],
    /// How the medium is moving, if an area that covers the cell says.
    pub drift: Option<AreaDrift>,
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
    areas: &'a AreaTable,
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

    /// The clock of the realm the grid belongs to.
    pub fn clock(&self) -> ClockId {
        self.slot.clock
    }

    /// The loaded tracts, row by row.
    pub fn loaded_tracts(&self) -> Vec<TractPos> {
        self.slot.grid.loaded_tracts()
    }

    /// The column of a loaded tract.
    pub fn column(&self, tract: TractPos) -> Option<&'a Column> {
        self.slot.grid.column(tract)
    }

    /// The element of a cell on a layer.
    pub fn element(&self, cell: CellPos, layer: Layer) -> Result<ElementState, WorldError> {
        let (column, cell) = self.locate(cell)?;
        Ok(column.element(layer, cell))
    }

    /// The areas that cover a cell, in id order. The cell must be inside
    /// the grid; its tract need not be loaded.
    pub fn areas_at(&self, cell: CellPos) -> Result<Vec<AreaId>, WorldError> {
        let cell = self.normalize(cell)?;
        Ok(self
            .areas
            .at(&self.slot.topology, self.id, cell)
            .map(|area| area.id)
            .collect())
    }

    /// Whether any area touches a tract of the grid: if none does, every
    /// cell of it has its stratum's ambient conditions.
    #[inline]
    pub(crate) fn any_area_in(&self, tract: TractPos) -> bool {
        !self.areas.in_tract(self.id, tract).is_empty()
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

    /// The level of a field in a cell, 0 to 255 (02 §16). Zero where
    /// the cell holds none of it.
    pub fn field(&self, cell: CellPos, field: FieldId) -> Result<u8, WorldError> {
        let (column, cell) = self.locate(cell)?;
        Ok(crate::field::level_in(column, cell, field))
    }

    /// The definitions the grid's cosmos is made from.
    pub(crate) fn defs(&self) -> &'a WorldDefs {
        self.defs
    }

    /// The clock of a cell: the clock [`ambient`](Self::ambient) gives
    /// it, worked out without the rest of its ambient conditions (the
    /// grid's, or that of the highest-priority area covering the cell
    /// that has one, of equals the one made first; D-928). The cell must
    /// be inside the grid.
    pub fn clock_at(&self, cell: CellPos) -> Result<ClockId, WorldError> {
        let cell = self.normalize(cell)?;
        let mut clock: Option<(i32, ClockId)> = None;
        if !self.areas.is_empty() {
            for area in self.areas.at(&self.slot.topology, self.id, cell) {
                if let Some(own) = area.clock
                    && clock.is_none_or(|(best, _)| area.priority > best)
                {
                    clock = Some((area.priority, own));
                }
            }
        }
        Ok(clock.map_or(self.slot.clock, |(_, own)| own))
    }

    /// The ambient conditions of a cell: its stratum's, with what the
    /// areas that cover it change. Where areas overlap, the one with the
    /// highest priority applies, and of equals the one made first:
    /// separately for gravity, for drift and for the clock, among the
    /// areas that set each. The cell must be inside the grid; its tract
    /// need not be loaded.
    pub fn ambient(&self, cell: CellPos) -> Result<Ambient, WorldError> {
        let cell = self.normalize(cell)?;
        let stratum = self
            .defs
            .realms()
            .get(self.slot.realm_def)
            .and_then(|kind| kind.stratum_at(cell.z));
        let gravity = stratum.map_or(Real::ZERO, |stratum| stratum.gravity);
        let mut ambient = Ambient {
            gravity,
            atmosphere: stratum.and_then(|stratum| stratum.atmosphere),
            clock: self.slot.clock,
            gravity_vector: [Real::ZERO, Real::ZERO, -gravity],
            drift: None,
        };
        if !self.areas.is_empty() {
            self.areas
                .apply_ambient(&self.slot.topology, self.id, cell, &mut ambient);
        }
        Ok(ambient)
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
    /// Every area, of every grid (02 §8.3).
    pub(crate) areas: AreaTable,
    /// Every site, of every realm (02 §8.4).
    pub(crate) sites: SiteTable,
    /// Where each occupant is. With the chunks' occupant indices this is
    /// the spatial index of 02 §8.1: rebuilt on load, never saved or
    /// hashed.
    pub(crate) occupants: LookupMap<EntityId, OccupantRecord>,
    /// The stability checks edits have asked for (02 §14.2).
    pub(crate) stability: StabilityQueue,
    /// The requests to keep tracts loaded, and which tracts they keep
    /// (02 §10).
    pub(crate) tickets: TicketTable,
    /// The world's processes and the collapses in progress (02 §14.3).
    pub(crate) activity: WorldActivity,
    /// The light worked out so far (15 §3.3). Derived: never saved or
    /// hashed, and empty in a clone.
    pub(crate) light: crate::light::LightSlot,
    /// What the cosmos keeps for saving (02 §11): the save's tables of
    /// ids, the entity sections waiting to be stored, and warnings. Not
    /// hashed.
    pub(crate) save: crate::save::SaveState,
    /// Counts the times the cells of any grid could have changed: every
    /// borrow of a grid, a column or a grid's slot to change it, but for
    /// the occupant index. Equal counts mean no cell has changed in
    /// between, which spares a cache comparing revisions chunk by chunk
    /// (D-731). Derived: not state, never saved or hashed.
    epoch: u64,
    /// Emptied tables the field stepper sums changes of level in, kept
    /// for its next step. Scratch: not state, never saved or hashed.
    pub(crate) field_tables: Vec<Box<[u32]>>,
}

/// The parts of the cosmos that a change to one grid touches, borrowed
/// apart so that they can be changed together.
pub(crate) struct GridMut<'a> {
    /// The definitions.
    pub defs: &'a WorldDefs,
    /// The grid's loaded columns.
    pub grid: &'a mut Grid,
    /// The grid's shape.
    pub topology: Topology,
    /// Every area.
    pub areas: &'a mut AreaTable,
    /// The stability checks asked for.
    pub stability: &'a mut StabilityQueue,
}

impl Cosmos {
    /// An empty cosmos over a set of definitions.
    pub fn new(defs: Arc<WorldDefs>) -> Cosmos {
        Cosmos {
            realms: BTreeMap::new(),
            grids: Vec::new(),
            next_realm: 0,
            vars: VarMap::new(),
            areas: AreaTable::new(),
            sites: SiteTable::new(),
            occupants: LookupMap::new(),
            stability: StabilityQueue::default(),
            tickets: TicketTable::new(),
            activity: WorldActivity::new(),
            light: crate::light::LightSlot::new(),
            save: crate::save::SaveState::new(&defs),
            epoch: 0,
            field_tables: Vec::new(),
            defs,
        }
    }

    /// The count of the times cells could have changed (D-731): if it
    /// reads the same twice, no cell of any grid changed in between.
    pub(crate) fn epoch(&self) -> u64 {
        self.epoch
    }

    /// The count of the times the cells of any grid could have changed
    /// (D-731), for caches outside the world (D-921): while it reads the
    /// same, no cell of any grid has changed, and no tract has been
    /// loaded or let go. It is not state, never saved or hashed: a cache
    /// keeps it beside what it keeps and compares it only within one
    /// cosmos, starting empty in any copy of it.
    pub fn cell_epoch(&self) -> u64 {
        self.epoch
    }

    /// The parts of the cosmos that a change to one grid touches.
    pub(crate) fn grid_mut(&mut self, grid: GridId) -> Result<GridMut<'_>, WorldError> {
        self.epoch = self.epoch.wrapping_add(1);
        self.grid_mut_occupants(grid)
    }

    /// [`grid_mut`](Cosmos::grid_mut) for the occupant index alone,
    /// which is not part of what cells hold: the count of changes to
    /// cells is left as it is. Nothing else may be changed through it.
    pub(crate) fn grid_mut_occupants(&mut self, grid: GridId) -> Result<GridMut<'_>, WorldError> {
        let slot = self
            .grids
            .get_mut(grid.index())
            .and_then(|slot| slot.as_deref_mut())
            .ok_or(WorldError::UnknownGrid(grid))?;
        Ok(GridMut {
            defs: &self.defs,
            topology: slot.topology,
            grid: &mut slot.grid,
            areas: &mut self.areas,
            stability: &mut self.stability,
        })
    }

    #[inline]
    fn slot(&self, grid: GridId) -> Option<&GridSlot> {
        self.grids.get(grid.index())?.as_deref()
    }

    fn slot_mut(&mut self, grid: GridId) -> Option<&mut GridSlot> {
        self.epoch = self.epoch.wrapping_add(1);
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
        self.epoch = self.epoch.wrapping_add(1);
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
                regions: BTreeMap::new(),
            },
        );
        Ok(id)
    }

    /// A realm by id.
    pub fn realm(&self, id: RealmId) -> Option<&Realm> {
        self.realms.get(&id)
    }

    /// A realm by id, to change. Crate-private: a realm's records change
    /// through the functions that survey, load and unload.
    pub(crate) fn realm_mut(&mut self, id: RealmId) -> Option<&mut Realm> {
        self.realms.get_mut(&id)
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

    /// Sets or clears a global variable and returns the value it had.
    pub fn set_global_var(
        &mut self,
        key: clash_core::VarKey,
        value: Option<Value>,
    ) -> Option<Value> {
        match value {
            Some(value) => self.vars.set(key, value),
            None => self.vars.remove(key.as_str()),
        }
    }

    /// A read-only handle on one grid, for reading many cells of it.
    #[inline]
    pub fn grid(&self, grid: GridId) -> Result<GridView<'_>, WorldError> {
        match self.slot(grid) {
            Some(slot) => Ok(GridView {
                id: grid,
                slot,
                defs: &self.defs,
                areas: &self.areas,
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

    /// The level of a field in a cell, 0 to 255 (02 §16).
    pub fn field(&self, loc: Location, field: FieldId) -> Result<u8, WorldError> {
        self.grid(loc.grid)?.field(loc.cell, field)
    }

    /// The ambient conditions of a cell: gravity, atmosphere and clock.
    /// The cell must be inside its grid; its tract need not be loaded.
    pub fn ambient(&self, loc: Location) -> Result<Ambient, WorldError> {
        self.grid(loc.grid)?.ambient(loc.cell)
    }

    /// The clock of a cell, as [`ambient`](Self::ambient) gives it, worked
    /// out alone (D-928).
    pub fn clock_at(&self, loc: Location) -> Result<ClockId, WorldError> {
        self.grid(loc.grid)?.clock_at(loc.cell)
    }

    /// The column of a loaded tract, to change. Crate-private: cells
    /// change only through `WorldEdit`.
    #[allow(dead_code, reason = "kept for the packages that load and store tracts")]
    pub(crate) fn column_mut(&mut self, grid: GridId, tract: TractPos) -> Option<&mut Column> {
        self.slot_mut(grid)?.grid.column_mut(tract)
    }

    /// Makes a tract of a grid loaded with these cells, and returns the
    /// column it replaced, if any. Whoever stood in the column replaced
    /// is taken out of the occupant index, without events.
    pub(crate) fn insert_column(
        &mut self,
        grid: GridId,
        tract: TractPos,
        column: Column,
    ) -> Result<Option<Column>, WorldError> {
        self.purge_occupants(grid, tract);
        self.light_tract_reloaded(grid, tract);
        let slot = self.slot_mut(grid).ok_or(WorldError::UnknownGrid(grid))?;
        let replaced = slot.grid.insert_column(tract, column);
        self.fields_column_changed(grid, tract);
        Ok(replaced)
    }

    /// The realm each grid belongs to, by grid id, with `None` where a
    /// grid has gone: the shape of the grid table, for a save.
    pub(crate) fn grid_realms(&self) -> Vec<Option<RealmId>> {
        self.grids
            .iter()
            .map(|slot| slot.as_deref().map(|slot| slot.realm))
            .collect()
    }

    /// The number the next realm will be given.
    pub(crate) fn next_realm_number(&self) -> u32 {
        self.next_realm
    }

    /// Puts back the realms, the grid table and the global variables
    /// read from a save, into a cosmos that has none. Every grid comes
    /// back with nothing loaded. Refused, changing nothing, if a grid
    /// names a realm that is not among those given or a realm names a
    /// definition that does not exist.
    pub(crate) fn restore_realms(
        &mut self,
        next_realm: u32,
        vars: VarMap,
        realms: Vec<Realm>,
        grids: Vec<Option<RealmId>>,
    ) -> Result<(), &'static str> {
        let realms: BTreeMap<RealmId, Realm> =
            realms.into_iter().map(|realm| (realm.id, realm)).collect();
        let mut slots = Vec::with_capacity(grids.len());
        for owner in grids {
            let Some(owner) = owner else {
                slots.push(None);
                continue;
            };
            let realm = realms
                .get(&owner)
                .ok_or("a grid belongs to a realm that was not saved")?;
            let kind = self
                .defs
                .realms()
                .get(realm.def)
                .ok_or("a realm has a definition that does not exist")?;
            slots.push(Some(Box::new(GridSlot {
                grid: Grid::new(),
                realm: owner,
                realm_def: realm.def,
                clock: realm.clock,
                topology: kind.topology,
            })));
        }
        if realms.values().any(|realm| {
            slots
                .get(realm.base_grid.index())
                .is_none_or(Option::is_none)
        }) {
            return Err("a realm's base grid was not saved");
        }
        self.realms = realms;
        self.epoch = self.epoch.wrapping_add(1);
        self.grids = slots;
        self.next_realm = next_realm;
        self.vars = vars;
        Ok(())
    }

    /// Unloads a tract of a grid and returns its column. Whoever stood
    /// in it is taken out of the occupant index, without events.
    pub(crate) fn remove_column(&mut self, grid: GridId, tract: TractPos) -> Option<Column> {
        self.purge_occupants(grid, tract);
        self.light_tract_reloaded(grid, tract);
        let slot = self.slot_mut(grid)?;
        let realm = slot.realm;
        let column = slot.grid.remove_column(tract)?;
        // A tract that is gone is no longer one the tickets keep.
        self.tickets.forget(realm, tract);
        self.fields_column_changed(grid, tract);
        Some(column)
    }
}

/// Hashes the realms in id order with their region records, then the
/// grids in id order with their loaded columns in tract order, then the
/// global variables, the areas, the sites, the stability checks waiting,
/// the tickets and what the world has under way. Who occupies which cell
/// is not hashed: the simulation above owns the entities.
impl StateHash for Cosmos {
    fn state_hash(&self, hasher: &mut StateHasher) {
        self.realms.state_hash(hasher);
        hasher.write_len(self.grids.len());
        for slot in &self.grids {
            slot.as_deref().state_hash(hasher);
        }
        hasher.write_u32(self.next_realm);
        self.vars.state_hash(hasher);
        self.areas.state_hash(hasher);
        self.sites.state_hash(hasher);
        self.stability.state_hash(hasher);
        self.tickets.state_hash(hasher);
        self.activity.state_hash(hasher);
    }
}
