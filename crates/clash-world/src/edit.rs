//! [`WorldCtx`] and [`WorldEdit`]: the one way cells change (02 §7.2,
//! 01 R3).
//!
//! Everything that changes the world does it through a [`WorldCtx`],
//! which holds the cosmos, the timeline and the sink that events go to.
//! Cells change through the [`WorldEdit`] it hands out for a cause. Every
//! change that takes effect keeps storage, the heightmap, the derived
//! flags and the revision counters right, marks the chunk dirty, asks
//! for a stability check where something structural was removed or
//! weakened, and emits exactly one event. A change that changes nothing
//! does none of this.
//!
//! ```
//! use std::sync::Arc;
//! use clash_time::Timeline;
//! use clash_world::{
//!     BlockState, Cause, CellPos, Cosmos, EventSink, GridId, Location, RawWorldDefs,
//!     WorldCtx, WorldDefs, WorldError,
//! };
//!
//! let defs = Arc::new(WorldDefs::build(RawWorldDefs::default()).unwrap());
//! let mut cosmos = Cosmos::new(defs);
//! let mut timeline = Timeline::new();
//! let mut events = EventSink::new();
//! let mut ctx = WorldCtx::new(&mut cosmos, &mut timeline, &mut events);
//!
//! // There is no grid here to edit: the edit is refused, with the reason.
//! let nowhere = Location::new(GridId::new(0), CellPos::new(0, 0, 0));
//! let refused = ctx.edit(Cause::DEBUG).set_block(nowhere, BlockState::NONE);
//! assert_eq!(refused, Err(WorldError::UnknownGrid(GridId::new(0))));
//! assert!(events.is_empty());
//! ```

use std::collections::BTreeMap;

use clash_core::{EntityId, StateHash, StateHasher, Tick, Value, VarKey};
use clash_time::Timeline;
use serde::{Deserialize, Serialize};

use crate::area::passage;
use crate::cause::Cause;
use crate::chunk::{DAMAGE_MAX, Decal, DecalId};
use crate::cosmos::Cosmos;
use crate::defs::{
    BlockState, CellFlags, ElementState, EngineFlag, FixtureState, FloorState, Layer, MaterialId,
    WorldDefs,
};
use crate::dims::CHUNK_Z_BITS;
use crate::error::WorldError;
use crate::event::{EventSink, WorldEvent};
use crate::iter::shell;
use crate::occupancy::{OccupantFilter, OccupantFlags, OccupantKind, distinct_ids};
use crate::pos::{CellPos, Location};

/// How far, in cells, an occupant is moved out of the way of an edit
/// that fills its cell (02 §23). The default of the setting
/// `edit.displace_range`.
pub const DISPLACE_RANGE_CELLS: u32 = 2;

/// A request to check whether what stands at a place is still held up
/// (02 §14.2).
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub struct StabilityRequest {
    /// Where a structural element was removed or weakened.
    pub loc: Location,
    /// When the first such change there was made.
    pub tick: Tick,
    /// Who and what made it.
    pub cause: Cause,
}

/// The stability checks that have been asked for and not yet made.
///
/// Edits only add to it. The collapse process takes requests out and
/// evaluates them (02 §14); until it is built they wait here. There is
/// one request for a cell however often it is asked for: the first.
#[derive(Clone, PartialEq, Eq, Debug, Default, Serialize, Deserialize)]
pub struct StabilityQueue {
    pending: BTreeMap<Location, (Tick, Cause)>,
}

impl StabilityQueue {
    /// How many cells are waiting for a check.
    pub fn len(&self) -> usize {
        self.pending.len()
    }

    /// Whether nothing is waiting.
    pub fn is_empty(&self) -> bool {
        self.pending.is_empty()
    }

    /// Whether a check has been asked for at a cell.
    pub fn contains(&self, loc: Location) -> bool {
        self.pending.contains_key(&loc)
    }

    /// The requests waiting, by grid and then by cell.
    pub fn iter(&self) -> impl Iterator<Item = StabilityRequest> + '_ {
        self.pending
            .iter()
            .map(|(loc, (tick, cause))| StabilityRequest {
                loc: *loc,
                tick: *tick,
                cause: *cause,
            })
    }

    /// Asks for a check at a cell. A cell already waiting keeps the
    /// request it has.
    pub(crate) fn request(&mut self, loc: Location, tick: Tick, cause: Cause) {
        self.pending.entry(loc).or_insert((tick, cause));
    }

    /// Takes every request out, by grid and then by cell.
    pub(crate) fn take(&mut self) -> Vec<StabilityRequest> {
        let taken = self.iter().collect();
        self.pending.clear();
        taken
    }
}

impl StateHash for StabilityQueue {
    fn state_hash(&self, hasher: &mut StateHasher) {
        hasher.write_len(self.pending.len());
        for request in self.iter() {
            request.loc.state_hash(hasher);
            request.tick.state_hash(hasher);
            request.cause.state_hash(hasher);
        }
    }
}

impl Cosmos {
    /// The stability checks that edits have asked for and nothing has
    /// made yet.
    pub fn stability_requests(&self) -> &StabilityQueue {
        &self.stability
    }

    /// Takes the waiting stability checks out, by grid and then by cell.
    /// For whoever evaluates them.
    pub fn take_stability_requests(&mut self) -> Vec<StabilityRequest> {
        self.stability.take()
    }
}

/// What a change to the world needs: the cosmos, the timeline and where
/// events go (02 §7.2). The definitions are the cosmos's own.
///
/// Make one where the three are at hand; it is three references and
/// costs nothing.
#[derive(Debug)]
pub struct WorldCtx<'a> {
    /// The world.
    pub cosmos: &'a mut Cosmos,
    /// The timeline: the tick events are stamped with, and the clocks.
    pub timeline: &'a mut Timeline,
    /// Where events go.
    pub events: &'a mut EventSink<WorldEvent>,
}

impl<'a> WorldCtx<'a> {
    /// A context over a cosmos, a timeline and a sink.
    pub fn new(
        cosmos: &'a mut Cosmos,
        timeline: &'a mut Timeline,
        events: &'a mut EventSink<WorldEvent>,
    ) -> WorldCtx<'a> {
        WorldCtx {
            cosmos,
            timeline,
            events,
        }
    }

    /// Starts editing cells for a cause. Every change made through the
    /// returned handle carries the cause.
    pub fn edit(&mut self, cause: Cause) -> WorldEdit<'_> {
        WorldEdit {
            ctx: WorldCtx {
                cosmos: &mut *self.cosmos,
                timeline: &mut *self.timeline,
                events: &mut *self.events,
            },
            cause,
        }
    }
}

/// The only way cells change (01 R3). Made by [`WorldCtx::edit`].
///
/// Every function normalises the location first and refuses, changing
/// nothing, a cell that is outside its grid
/// ([`WorldError::OutsideRealm`]), not loaded
/// ([`WorldError::NotLoaded`]) or too far from the rest of its tract to
/// store ([`WorldError::OutOfReach`]). An edit is never refused because
/// someone is standing there.
#[derive(Debug)]
pub struct WorldEdit<'a> {
    ctx: WorldCtx<'a>,
    cause: Cause,
}

/// Whether a change to a block or floor removed or weakened something
/// structural: whether it calls for a stability check (02 §14.2).
///
/// Putting an element where there was none does not. Removing one,
/// replacing it by another definition or making it of another material
/// does. A change of state alone does only if the element stops being
/// `load_bearing` or an `anchor`, or loses volume.
fn weakened(defs: &WorldDefs, layer: Layer, old: ElementState, new: ElementState) -> bool {
    if old.is_none() {
        return false;
    }
    if new.def != old.def || new.material != old.material {
        return true;
    }
    let (was, is) = (defs.info(layer, old), defs.info(layer, new));
    let lost = |flag: EngineFlag| was.flags.has(flag) && !is.flags.has(flag);
    lost(EngineFlag::LoadBearing) || lost(EngineFlag::Anchor) || is.volume < was.volume
}

impl WorldEdit<'_> {
    /// The cause every change made through this handle carries.
    pub fn cause(&self) -> Cause {
        self.cause
    }

    /// Sets a cell's floor and returns the floor it had.
    ///
    /// If that changes the cell: the new floor starts undamaged, a
    /// stability check is asked for if a floor was removed or weakened,
    /// and `ElementChanged` is emitted.
    pub fn set_floor(
        &mut self,
        loc: Location,
        state: FloorState,
    ) -> Result<FloorState, WorldError> {
        self.change(loc, Layer::Floor, state.raw(), false)
            .map(FloorState::from_raw)
    }

    /// Sets a cell's block and returns the block it had.
    ///
    /// If that changes the cell: the new block starts undamaged, a
    /// stability check is asked for if a block was removed or weakened,
    /// `ElementChanged` is emitted, and an occupant the block leaves no
    /// room for is moved out of the way (`OccupantDisplaced`) or, with
    /// nowhere to go, left where it is (`OccupantEntombed`).
    pub fn set_block(
        &mut self,
        loc: Location,
        state: BlockState,
    ) -> Result<BlockState, WorldError> {
        self.change(loc, Layer::Block, state.raw(), false)
            .map(BlockState::from_raw)
    }

    /// Sets a cell's fixture and returns the fixture it had.
    ///
    /// If that changes the cell: the new fixture starts undamaged,
    /// `ElementChanged` is emitted, and an occupant the fixture leaves no
    /// room for is moved out of the way or left where it is, as for
    /// [`set_block`](WorldEdit::set_block).
    pub fn set_fixture(
        &mut self,
        loc: Location,
        state: FixtureState,
    ) -> Result<FixtureState, WorldError> {
        self.change(loc, Layer::Fixture, state.raw(), false)
            .map(FixtureState::from_raw)
    }

    /// Sets the element of a cell on a layer and returns the one it had.
    /// The same as [`set_floor`](WorldEdit::set_floor),
    /// [`set_block`](WorldEdit::set_block) or
    /// [`set_fixture`](WorldEdit::set_fixture), for code that has the
    /// layer as a value.
    pub fn set_element(
        &mut self,
        loc: Location,
        layer: Layer,
        state: ElementState,
    ) -> Result<ElementState, WorldError> {
        self.change(loc, layer, state, false)
    }

    /// Makes the element of a cell on a layer of another material, and
    /// returns the state it had. The element stays: it keeps its
    /// definition, its state and its accumulated damage. Where there is
    /// no element nothing changes.
    pub fn set_material(
        &mut self,
        loc: Location,
        layer: Layer,
        material: MaterialId,
    ) -> Result<ElementState, WorldError> {
        let old = self.ctx.cosmos.grid(loc.grid)?.element(loc.cell, layer)?;
        if old.is_none() {
            return Ok(old);
        }
        self.change(loc, layer, ElementState { material, ..old }, true)
    }

    fn change(
        &mut self,
        loc: Location,
        layer: Layer,
        new: ElementState,
        keep_damage: bool,
    ) -> Result<ElementState, WorldError> {
        let tick = self.ctx.timeline.now();
        let cause = self.cause;
        let new = new.normalised();
        let parts = self.ctx.cosmos.grid_mut(loc.grid)?;
        let cell = parts
            .topology
            .normalize(loc.cell)
            .map_err(|edge| WorldError::OutsideRealm { loc, edge })?;
        let loc = Location::new(loc.grid, cell);
        let defs = parts.defs;
        let column = parts
            .grid
            .column_mut(cell.tract())
            .ok_or(WorldError::NotLoaded(loc))?;
        let old = column.element(layer, cell);
        if old == new {
            return Ok(old);
        }
        if !column.can_store(cell.z >> CHUNK_Z_BITS) {
            return Err(WorldError::OutOfReach(loc));
        }

        let enclosures = parts.areas.has_enclosures();
        let passage_before = enclosures.then(|| passage(defs, column, cell));
        let blocked_before = column.flags(cell).contains(CellFlags::WALK_BLOCKED);

        match layer {
            Layer::Floor => {
                column.set_floor(defs, cell, FloorState::from_raw(new));
            }
            Layer::Block => {
                column.set_block(defs, cell, BlockState::from_raw(new));
            }
            Layer::Fixture => {
                column.set_fixture(defs, cell, FixtureState::from_raw(new));
            }
        }
        if !keep_damage {
            // A replaced element's damage goes with it.
            let mut damage = column.damage(cell);
            damage[layer.index()] = 0;
            column.set_damage(defs, cell, damage);
        }
        if layer != Layer::Fixture && weakened(defs, layer, old, new) {
            parts.stability.request(loc, tick, cause);
        }
        let passage_after = enclosures.then(|| passage(defs, column, cell));
        let newly_blocked = !blocked_before && column.flags(cell).contains(CellFlags::WALK_BLOCKED);
        let crowded = newly_blocked
            && column
                .occupants_in(cell.z >> CHUNK_Z_BITS)
                .is_some_and(|index| !index.at(cell.idx()).is_empty());

        self.ctx.events.emit(WorldEvent::ElementChanged {
            tick,
            loc,
            layer,
            old,
            new,
            cause,
        });

        if let (Some(before), Some(after)) = (passage_before, passage_after)
            && before != after
        {
            // The change only opened the cell if it closed neither the
            // cell nor its floor.
            let opened = (after.0 || !before.0) && (after.1 || !before.1);
            self.ctx.recheck_enclosures(loc, opened, cause);
        }
        if crowded {
            self.make_room(loc);
        }
        Ok(old)
    }

    /// Moves whoever stands in a cell that an edit has just filled to
    /// the nearest cell with room, or reports that there is none.
    fn make_room(&mut self, loc: Location) {
        let cause = self.cause;
        // A device stands in the cell of its own element.
        let filter = OccupantFilter::ANY.of_kinds(crate::occupancy::OccupantKinds::of(&[
            OccupantKind::Creature,
            OccupantKind::Item,
            OccupantKind::Other,
        ]));
        let Ok(crowd) = self.ctx.cosmos.occupants_at(loc, &filter) else {
            return;
        };
        for id in distinct_ids(&crowd) {
            let Some(from) = self.ctx.cosmos.occupant(id).map(|record| record.loc) else {
                continue;
            };
            let tick = self.ctx.timeline.now();
            match self.room_near(id) {
                Some(to) if self.ctx.relocate(id, to, cause).is_ok() => {
                    self.ctx.events.emit(WorldEvent::OccupantDisplaced {
                        tick,
                        occupant: id,
                        from,
                        to,
                        cause,
                    });
                }
                _ => self.ctx.events.emit(WorldEvent::OccupantEntombed {
                    tick,
                    occupant: id,
                    loc: from,
                    cause,
                }),
            }
        }
    }

    /// The nearest position within [`DISPLACE_RANGE_CELLS`] at which an
    /// occupant has room: none of its cells is walked into, none holds
    /// someone it cannot walk into, and it can be stood in if where the
    /// occupant is now can be.
    fn room_near(&self, id: EntityId) -> Option<Location> {
        let cosmos = &*self.ctx.cosmos;
        let record = cosmos.occupant(id)?;
        let view = cosmos.grid(record.loc.grid).ok()?;
        let stands = view.standable(record.loc.cell).unwrap_or(false);
        let others = OccupantFilter::ANY
            .on(record.occupant.plane)
            .with_flags(OccupantFlags::BLOCKS_MOVE)
            .except(id);
        let has_room = |at: CellPos| {
            if stands && !view.standable(at).unwrap_or(false) {
                return false;
            }
            record.footprint.cells(at).cells().all(|cell| {
                let free = view
                    .flags(cell)
                    .is_ok_and(|flags| !flags.contains(CellFlags::WALK_BLOCKED));
                free && !cosmos
                    .is_occupied(Location::new(view.id(), cell), &others)
                    .unwrap_or(true)
            })
        };
        (1..=DISPLACE_RANGE_CELLS)
            .flat_map(|radius| shell(record.loc.cell, radius))
            .find(|at| has_room(*at))
            .and_then(|at| view.normalize(at).ok())
            .map(|cell| Location::new(view.id(), cell))
    }

    /// Sets the accumulated damage of a cell's element on a layer, in
    /// 255ths of its capacity, and returns what it was. More than
    /// [`DAMAGE_MAX`] counts as that. Where there is no element nothing
    /// changes.
    ///
    /// If that changes the damage: a stability check is asked for if a
    /// floor or block took more damage, and `ElementDamaged` is emitted.
    /// What a packet of damage does to an element, and its destruction,
    /// are built on this (02 §12).
    pub fn set_damage(
        &mut self,
        loc: Location,
        layer: Layer,
        damage: u8,
    ) -> Result<u8, WorldError> {
        let tick = self.ctx.timeline.now();
        let cause = self.cause;
        let parts = self.ctx.cosmos.grid_mut(loc.grid)?;
        let cell = parts
            .topology
            .normalize(loc.cell)
            .map_err(|edge| WorldError::OutsideRealm { loc, edge })?;
        let loc = Location::new(loc.grid, cell);
        let column = parts
            .grid
            .column_mut(cell.tract())
            .ok_or(WorldError::NotLoaded(loc))?;
        let state = column.element(layer, cell);
        let mut all = column.damage(cell);
        let old = all[layer.index()];
        let new = damage.min(DAMAGE_MAX);
        if state.is_none() || old == new {
            return Ok(old);
        }
        all[layer.index()] = new;
        column.set_damage(parts.defs, cell, all);
        if layer != Layer::Fixture && new > old {
            parts.stability.request(loc, tick, cause);
        }
        self.ctx.events.emit(WorldEvent::ElementDamaged {
            tick,
            loc,
            layer,
            state,
            old,
            new,
            cause,
        });
        Ok(old)
    }

    /// The column a cell is in, ready to write to, with the cell
    /// normalised.
    fn column_for(
        &mut self,
        loc: Location,
    ) -> Result<(&mut crate::grid::Column, &WorldDefs, Location), WorldError> {
        let parts = self.ctx.cosmos.grid_mut(loc.grid)?;
        let cell = parts
            .topology
            .normalize(loc.cell)
            .map_err(|edge| WorldError::OutsideRealm { loc, edge })?;
        let loc = Location::new(loc.grid, cell);
        let column = parts
            .grid
            .column_mut(cell.tract())
            .ok_or(WorldError::NotLoaded(loc))?;
        Ok((column, parts.defs, loc))
    }

    /// Leaves a mark of a kind in a cell, stamped with this edit's cause
    /// and the time. Emits `DecalAdded`.
    pub fn add_decal(
        &mut self,
        loc: Location,
        kind: DecalId,
        amount: u8,
    ) -> Result<(), WorldError> {
        let tick = self.ctx.timeline.now();
        let decal = Decal {
            kind,
            amount,
            cause: self.cause,
            tick,
        };
        let (column, defs, loc) = self.column_for(loc)?;
        if !column.can_store(loc.cell.z >> CHUNK_Z_BITS) {
            return Err(WorldError::OutOfReach(loc));
        }
        column.add_decal(defs, loc.cell, decal);
        self.ctx
            .events
            .emit(WorldEvent::DecalAdded { tick, loc, decal });
        Ok(())
    }

    /// Removes every mark of a kind from a cell and returns how many
    /// there were. Emits `DecalsRemoved` if there were any.
    pub fn remove_decals(&mut self, loc: Location, kind: DecalId) -> Result<usize, WorldError> {
        let tick = self.ctx.timeline.now();
        let cause = self.cause;
        let (column, _, loc) = self.column_for(loc)?;
        let count = column.remove_decals(loc.cell, kind);
        if count > 0 {
            self.ctx.events.emit(WorldEvent::DecalsRemoved {
                tick,
                loc,
                kind,
                count: u32::try_from(count).unwrap_or(u32::MAX),
                cause,
            });
        }
        Ok(count)
    }

    /// Sets a mod variable on a cell and returns the value it had. Emits
    /// `CellVarChanged` unless it already had that value.
    pub fn set_var(
        &mut self,
        loc: Location,
        key: VarKey,
        value: Value,
    ) -> Result<Option<Value>, WorldError> {
        self.write_var(loc, key, Some(value))
    }

    /// Clears a mod variable on a cell and returns the value it had.
    /// Emits `CellVarChanged` if it had one.
    pub fn clear_var(&mut self, loc: Location, key: VarKey) -> Result<Option<Value>, WorldError> {
        self.write_var(loc, key, None)
    }

    fn write_var(
        &mut self,
        loc: Location,
        key: VarKey,
        value: Option<Value>,
    ) -> Result<Option<Value>, WorldError> {
        let tick = self.ctx.timeline.now();
        let cause = self.cause;
        let (column, defs, loc) = self.column_for(loc)?;
        let old = column.var(loc.cell, key.as_str()).cloned();
        if old == value {
            return Ok(old);
        }
        if !column.can_store(loc.cell.z >> CHUNK_Z_BITS) {
            return Err(WorldError::OutOfReach(loc));
        }
        column.set_var(defs, loc.cell, key.clone(), value.clone());
        self.ctx.events.emit(WorldEvent::CellVarChanged {
            tick,
            loc,
            key,
            old: old.clone(),
            new: value,
            cause,
        });
        Ok(old)
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;
    use crate::defs::RawWorldDefs;
    use crate::grid::COLUMN_MAX_CHUNKS;
    use crate::pos::TractPos;
    use crate::test_support::load_flat;

    /// A realm tall enough that a cell can lie further from the stored
    /// cells of its tract than a column will stretch.
    const TALL: &str = r#"{
        "size_regions": [1, 1],
        "z_range": [-400000, 400000],
        "strata": [ { "from_z": 0, "gravity": 9.81 } ]
    }"#;

    #[test]
    fn a_write_too_far_from_the_stored_cells_is_refused() {
        let mut raw = RawWorldDefs::default();
        let realm = serde_json::from_str(TALL).unwrap_or_else(|error| panic!("{error}"));
        raw.realms.push(("test:tall".to_owned(), realm));
        let defs = Arc::new(WorldDefs::build(raw).unwrap_or_else(|errors| panic!("{errors:?}")));
        let mut cosmos = Cosmos::new(Arc::clone(&defs));
        let mut timeline = Timeline::new();
        let mut events = EventSink::new();
        let def = defs
            .realms()
            .lookup("test:tall")
            .unwrap_or_else(|| unreachable!());
        let realm = cosmos
            .create_realm(&mut timeline, def, 1)
            .unwrap_or_else(|error| panic!("{error}"));
        let grid = cosmos.realm(realm).map(|realm| realm.base_grid);
        let grid = grid.unwrap_or_else(|| unreachable!());
        load_flat(
            &mut cosmos,
            grid,
            [TractPos::new(0, 0)],
            BlockState::NONE,
            FloorState::NONE,
            0,
        )
        .unwrap_or_else(|error| panic!("{error}"));

        let storeys = i32::try_from(COLUMN_MAX_CHUNKS << CHUNK_Z_BITS).unwrap_or(i32::MAX);
        let far = Location::new(grid, CellPos::new(3, 3, storeys));
        let near = Location::new(grid, CellPos::new(3, 3, 40));
        // Any block will do for writing: one whose definition is not
        // none. With no definitions loaded, the bits make the difference.
        let something = FloorState::from_raw(ElementState {
            def: 1,
            material: MaterialId::new(0),
            bits: 0,
        });
        let mut ctx = WorldCtx::new(&mut cosmos, &mut timeline, &mut events);
        let mut edit = ctx.edit(Cause::DEBUG);
        assert_eq!(
            edit.set_floor(far, something),
            Err(WorldError::OutOfReach(far))
        );
        assert_eq!(
            edit.add_decal(far, DecalId::new(0), 1),
            Err(WorldError::OutOfReach(far))
        );
        assert_eq!(
            edit.set_var(far, VarKey::new("test:x"), Value::from(1)),
            Err(WorldError::OutOfReach(far))
        );
        // Nearer, the same writes are made.
        assert_eq!(edit.set_floor(near, something), Ok(FloorState::NONE));
        assert_eq!(edit.add_decal(near, DecalId::new(0), 1), Ok(()));
        assert_eq!(events.len(), 2);
        // And below the stored cells the same limit holds.
        let deep = Location::new(grid, CellPos::new(3, 3, 40 - storeys));
        let mut ctx = WorldCtx::new(&mut cosmos, &mut timeline, &mut events);
        assert_eq!(
            ctx.edit(Cause::DEBUG).set_floor(deep, something),
            Err(WorldError::OutOfReach(deep))
        );
    }
}
