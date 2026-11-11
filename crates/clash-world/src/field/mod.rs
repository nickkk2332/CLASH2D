//! Fields (02 §16): per-cell values from 0 to 255 that evolve by rule.
//! Fire, smoke, water and steam are fields, and the engine knows none
//! of them: a field is whatever its definition says.
//!
//! | Need | Use |
//! |------|-----|
//! | The `field` and `field_reaction` definitions | [`Field`], [`FieldRule`], [`FieldReaction`]; [`WorldDefs::fields`](crate::WorldDefs::fields) |
//! | Put a field in a cell, or take it away | [`WorldEdit::add_field`], [`WorldEdit::set_field`] |
//! | Read a field | [`Cosmos::field`], [`Cosmos::fields_at`], [`Cosmos::field_cell`], [`Cosmos::fields_in_box`] |
//! | Whether a cell's fields hide what is behind it | [`Cosmos::field_hides_sight`] |
//! | The light fields give off, as moving sources | [`Cosmos::field_lights`] |
//! | Tell the fields of a cell that damage arrived | [`WorldEdit::conduct`] |
//! | How the medium is moving at a cell | [`Cosmos::field_drift`] |
//! | How many cells are active, and the budget | [`Cosmos::active_field_cells`], [`Cosmos::set_field_max_active_cells`] |
//!
//! # Stepping
//!
//! Only **active** cells are stepped: cells that hold a field and are
//! awake. A cell of a field whose rules only move it (`flow`,
//! `diffuse`) falls asleep when a step leaves it untouched, and wakes
//! when a level or an element in or beside it changes. Every other
//! field's cells are awake for as long as they last.
//!
//! One process of the world steps the active cells of one clock and
//! one step interval ([`FieldStep`]). It wakes at whole multiples of
//! the interval in the clock's local time, so that a field in stopped
//! time does not burn and the outcome does not depend on when the
//! process began. Each step reads the levels as they stand, gathers
//! every change, and writes them all at the end in order of cell: the
//! result does not depend on the order cells are visited in. The one
//! random choice, whether a field spreads, is a positional hash of the
//! field, the cell and the number of the step.
//!
//! The world reports and does not judge: an occupant in a field is
//! told of with `OccupantExposed` (the `damage` rule) and
//! `OccupantDosed` (the `expose` rule), and what that does to it is
//! the simulation's.

mod defs;
mod step;
mod store;

#[cfg(test)]
mod tests;

use std::collections::BTreeSet;

use clash_core::{Dur, EntityId, IVec3, Real, StateHash, StateHasher};
use clash_time::{ClockId, ProcessId};
use serde::{Deserialize, Serialize};

pub(crate) use defs::build_defs;
pub use defs::{
    CatchUp, ColorList, ExposureRoute, FIELD_MAX_LEVEL, FIELD_MAX_RATE, FIELD_MIN_STEP,
    FLAMMABLE_TAG, FLOW_DEFAULT_RATE, Field, FieldConducts, FieldDef, FieldGlyph, FieldGlyphSpec,
    FieldLight, FieldLightSpec, FieldReaction, FieldReactionDef, FieldReactionId, FieldReacts,
    FieldRule, FieldRuleKind, FieldRuleSpec, MaterialTagSet, Reactant, ReactantSpec, SpreadsSpec,
    VerticalBias,
};
pub use store::{FieldCell, FieldLayer, FieldStore};

use crate::area::Drift;
use crate::cause::{Cause, CauseKind};
use crate::chunk::FieldId;
use crate::cosmos::{Cosmos, GridView, RealmId};
use crate::damage::DamagePacket;
use crate::dims::{CHUNK_XY_BITS, CHUNK_Z, CHUNK_Z_BITS};
use crate::edit::{WorldCtx, WorldEdit};
use crate::error::WorldError;
use crate::event::WorldEvent;
use crate::grid::Column;
use crate::light::MovingLight;
use crate::occupancy::{Footprint, OccupantFilter, QUERY_MAX_TRACTS, distinct_ids};
use crate::pos::{CellBox, CellPos, GridId, Location, TractPos};
use crate::process::WorldProcess;

/// The most active field cells before the weakest decay faster: the
/// default of the setting `field.max_active_cells` (02 §16).
pub const FIELD_MAX_ACTIVE_CELLS: u32 = 200_000;

/// The levels an active cell among the weakest loses each step, on top
/// of its own decay, while there are more active cells than the budget
/// allows.
pub const FIELD_OVER_BUDGET_DECAY: u8 = 16;

/// For the catch-up rule `disperse`: the number of steps in which a
/// field that nobody watched loses half of what it has left.
pub const DISPERSE_HALF_STEPS: i64 = 16;

/// The most cells a packet of damage is passed on to by one field in
/// one cell, whatever the reaction's reach.
pub const FIELD_CONDUCT_MAX_CELLS: usize = 1 << 12;

/// The steps a field takes from a cell, in order: north, east, south,
/// west, up, down. The first four are the cells beside it.
pub(crate) const FIELD_STEPS: [IVec3; 6] = [
    IVec3::new(0, -1, 0),
    IVec3::new(1, 0, 0),
    IVec3::new(0, 1, 0),
    IVec3::new(-1, 0, 0),
    IVec3::new(0, 0, 1),
    IVec3::new(0, 0, -1),
];

/// The process that steps the active field cells of one clock and one
/// step interval (02 §16). Plain state: what it steps is found again
/// at every wake.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug, Serialize, Deserialize)]
pub struct FieldStep {
    /// The clock whose cells it steps, and on which it wakes.
    pub clock: ClockId,
    /// The step interval of the fields it steps, in local time.
    pub step: Dur,
}

clash_core::impl_state_hash!(FieldStep { clock, step });

/// What the world keeps about its fields besides the cells themselves:
/// where they are, which processes step them, and the budget.
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub struct FieldState {
    /// The loaded tracts that hold any field at all, by grid.
    tracts: BTreeSet<(GridId, TractPos)>,
    /// The step processes, in order of clock and step interval.
    steps: Vec<(FieldStep, ProcessId)>,
    /// The most active cells before the weakest decay faster.
    max_active: u32,
    /// Whether the last step found more active cells than that.
    over_budget: bool,
    /// Cells woken and not yet seen to have a process that steps them.
    /// Empty between any two operations of the world: never state.
    #[serde(skip)]
    woken: Vec<(Location, FieldId)>,
}

impl FieldState {
    /// No field anywhere, and the default budget.
    pub fn new() -> FieldState {
        FieldState {
            tracts: BTreeSet::new(),
            steps: Vec::new(),
            max_active: FIELD_MAX_ACTIVE_CELLS,
            over_budget: false,
            woken: Vec::new(),
        }
    }

    /// The loaded tracts that hold any field, by grid and then row by
    /// row.
    pub fn tracts(&self) -> impl Iterator<Item = (GridId, TractPos)> + '_ {
        self.tracts.iter().copied()
    }

    /// The processes that step fields, in order of clock and step
    /// interval.
    pub fn steps(&self) -> impl Iterator<Item = FieldStep> + '_ {
        self.steps.iter().map(|(step, _)| *step)
    }

    /// Whether the last step found more active cells than the budget.
    pub fn is_over_budget(&self) -> bool {
        self.over_budget
    }
}

impl Default for FieldState {
    fn default() -> FieldState {
        FieldState::new()
    }
}

impl StateHash for FieldState {
    fn state_hash(&self, hasher: &mut StateHasher) {
        hasher.write_len(self.tracts.len());
        for (grid, tract) in &self.tracts {
            grid.state_hash(hasher);
            tract.state_hash(hasher);
        }
        hasher.write_len(self.steps.len());
        for (step, process) in &self.steps {
            step.state_hash(hasher);
            process.state_hash(hasher);
        }
        hasher.write_u32(self.max_active);
        hasher.write_bool(self.over_budget);
    }
}

/// One field in one cell, as [`Cosmos::fields_in_box`] lists them.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct FieldSample {
    /// The cell.
    pub cell: CellPos,
    /// The field.
    pub field: FieldId,
    /// Its level there, 1 to 255.
    pub level: u8,
}

/// What a write to a field in a cell did.
#[derive(Clone, Copy, Debug)]
pub(crate) struct FieldWrite {
    /// The level before.
    pub old: u8,
    /// The level after.
    pub new: u8,
    /// Who is responsible for the field in the cell.
    pub by: Option<EntityId>,
}

/// The fields of a cell of a column.
#[inline]
pub(crate) fn store_at(column: &Column, cell: CellPos) -> Option<&FieldStore> {
    column
        .chunk(cell.z >> CHUNK_Z_BITS)
        .map(|chunk| chunk.fields())
        .filter(|store| !store.is_empty())
}

/// The level of a field in a cell of a column.
#[inline]
pub(crate) fn level_in(column: &Column, cell: CellPos, field: FieldId) -> u8 {
    store_at(column, cell).map_or(0, |store| store.level(field, cell.idx()))
}

/// Whether a column holds any field in any cell.
pub(crate) fn column_has_fields(column: &Column) -> bool {
    column.chunks().any(|(_, chunk)| !chunk.fields().is_empty())
}

/// The cause of what a field does of itself: the field, for whoever is
/// responsible for its being there (02 §7.3).
pub(crate) fn field_cause(by: Option<EntityId>) -> Cause {
    Cause {
        actor: None,
        instigator: by,
        kind: CauseKind::Field,
    }
}

impl GridView<'_> {
    /// Every field in a cell with its level, in id order.
    pub fn fields_at(&self, cell: CellPos) -> Result<Vec<(FieldId, u8)>, WorldError> {
        let cell = self.normalize(cell)?;
        let column = self
            .column(cell.tract())
            .ok_or(WorldError::NotLoaded(Location::new(self.id(), cell)))?;
        Ok(store_at(column, cell).map_or_else(Vec::new, |store| {
            store
                .at(cell.idx())
                .map(|(field, held)| (field, held.level))
                .collect()
        }))
    }

    /// One field in one cell: its level, whether it is active and who
    /// is responsible for it. `None` where the cell holds none of it.
    pub fn field_cell(
        &self,
        cell: CellPos,
        field: FieldId,
    ) -> Result<Option<FieldCell>, WorldError> {
        let cell = self.normalize(cell)?;
        let column = self
            .column(cell.tract())
            .ok_or(WorldError::NotLoaded(Location::new(self.id(), cell)))?;
        Ok(store_at(column, cell).and_then(|store| store.cell(field, cell.idx()).copied()))
    }

    /// Whether the fields of a cell hide what is behind it: one of them
    /// is at or above the level its definition gives as `opaque_from`.
    pub fn hides_sight(&self, cell: CellPos) -> Result<bool, WorldError> {
        let cell = self.normalize(cell)?;
        let column = self
            .column(cell.tract())
            .ok_or(WorldError::NotLoaded(Location::new(self.id(), cell)))?;
        let defs = self.defs();
        Ok(store_at(column, cell).is_some_and(|store| {
            store.at(cell.idx()).any(|(field, held)| {
                defs.fields()
                    .get(field)
                    .and_then(|def| def.opaque_from)
                    .is_some_and(|from| held.level >= from)
            })
        }))
    }

    /// How the medium is moving at a cell, in metres a second east,
    /// south and up, where an area sets a drift (11 §4.7). `None`
    /// where nothing moves it. The region's wind and current join this
    /// with the weather (09 §14.1).
    pub fn field_drift(&self, cell: CellPos) -> Result<Option<[Real; 3]>, WorldError> {
        let cell = self.normalize(cell)?;
        let Some(set) = self.ambient(cell)?.drift else {
            return Ok(None);
        };
        Ok(Some(match set.drift {
            Drift::Flow(velocity) => velocity,
            Drift::Vortex {
                speed,
                inflow,
                lift,
            } => {
                let out = self.topology().delta(set.centre, cell);
                let (x, y) = (
                    Real::from_i64(i64::from(out.x)),
                    Real::from_i64(i64::from(out.y)),
                );
                let far = (x * x + y * y).sqrt();
                if far <= Real::ZERO {
                    [Real::ZERO, Real::ZERO, lift]
                } else {
                    // Outward, and at right angles to that: round from
                    // east to south.
                    let (ox, oy) = (x / far, y / far);
                    [-oy * speed - ox * inflow, ox * speed - oy * inflow, lift]
                }
            }
        }))
    }
}

impl Cosmos {
    /// Every field in a cell with its level, in id order.
    pub fn fields_at(&self, loc: Location) -> Result<Vec<(FieldId, u8)>, WorldError> {
        self.grid(loc.grid)?.fields_at(loc.cell)
    }

    /// One field in one cell: its level, whether it is active and who
    /// is responsible for it. `None` where the cell holds none of it.
    pub fn field_cell(
        &self,
        loc: Location,
        field: FieldId,
    ) -> Result<Option<FieldCell>, WorldError> {
        self.grid(loc.grid)?.field_cell(loc.cell, field)
    }

    /// Whether the fields of a cell hide what is behind it, as
    /// something opaque taller than the eye would: one of them is at
    /// or above its `opaque_from`.
    pub fn field_hides_sight(&self, loc: Location) -> Result<bool, WorldError> {
        self.grid(loc.grid)?.hides_sight(loc.cell)
    }

    /// How the medium is moving at a cell, in metres a second east,
    /// south and up, where an area sets a drift. See
    /// [`GridView::field_drift`].
    pub fn field_drift(&self, loc: Location) -> Result<Option<[Real; 3]>, WorldError> {
        self.grid(loc.grid)?.field_drift(loc.cell)
    }

    /// Every field in the loaded cells of a box, by cell (storey by
    /// storey, row by row) and then by field. Refused for a box over
    /// more than [`QUERY_MAX_TRACTS`] tracts.
    pub fn fields_in_box(
        &self,
        grid: GridId,
        cells: CellBox,
    ) -> Result<Vec<FieldSample>, WorldError> {
        let view = self.grid(grid)?;
        let mut found: Vec<FieldSample> = Vec::new();
        if self.activity.fields.tracts.is_empty() {
            return Ok(found);
        }
        let across = |low: i32, high: i32| {
            u64::try_from((i64::from(high >> CHUNK_XY_BITS)) - i64::from(low >> CHUNK_XY_BITS) + 1)
                .unwrap_or(u64::MAX)
        };
        let tracts =
            across(cells.min.x, cells.max.x).saturating_mul(across(cells.min.y, cells.max.y));
        if tracts > QUERY_MAX_TRACTS {
            return Err(WorldError::TooLarge {
                what: "the tracts a box of fields covers",
                limit: QUERY_MAX_TRACTS,
            });
        }
        for tract in cells.tracts() {
            let origin = tract.origin(0);
            let Ok(base) = view.normalize(origin) else {
                continue;
            };
            let Some(column) = view.column(base.tract()) else {
                continue;
            };
            for (chunk_z, chunk) in column.chunks() {
                let low = chunk_z << CHUNK_Z_BITS;
                if chunk.fields().is_empty() || low > cells.max.z || low + CHUNK_Z - 1 < cells.min.z
                {
                    continue;
                }
                for (field, layer) in chunk.fields().layers() {
                    for (idx, held) in layer.cells() {
                        let asked = CellPos::new(
                            origin.x.wrapping_add(idx.lx()),
                            origin.y.wrapping_add(idx.ly()),
                            low + idx.lz(),
                        );
                        if cells.contains(asked) {
                            found.push(FieldSample {
                                cell: CellPos::new(base.x + idx.lx(), base.y + idx.ly(), asked.z),
                                field,
                                level: held.level,
                            });
                        }
                    }
                }
            }
        }
        found.sort_by_key(|sample| (sample.cell, sample.field));
        Ok(found)
    }

    /// The light the fields in a box of cells give off, as moving
    /// sources for [`Cosmos::light_with`] and [`Cosmos::fill_light`]
    /// (15 §3.3): one for each cell that holds a field whose definition
    /// has a `light`, at the level it has there, by cell and then by
    /// field. The world keeps no light for fields: whoever asks for
    /// light hands these in.
    pub fn field_lights(
        &self,
        grid: GridId,
        cells: CellBox,
    ) -> Result<Vec<MovingLight>, WorldError> {
        let defs = self.defs();
        if defs.fields().iter().all(|(_, field)| field.light.is_none()) {
            return Ok(Vec::new());
        }
        Ok(self
            .fields_in_box(grid, cells)?
            .into_iter()
            .filter_map(|sample| {
                let light = defs.fields().get(sample.field)?.light?;
                Some(MovingLight {
                    at: Location::new(grid, sample.cell),
                    source: light.at_level(sample.level)?,
                })
            })
            .collect())
    }

    /// What the world keeps about its fields: the tracts that hold
    /// any, and the processes that step them.
    pub fn field_state(&self) -> &FieldState {
        &self.activity.fields
    }

    /// How many field cells are active: awake, and stepped with their
    /// field.
    pub fn active_field_cells(&self) -> u64 {
        let mut awake = 0u64;
        for (grid, tract) in self.activity.fields.tracts() {
            let Some(column) = self.grid(grid).ok().and_then(|view| view.column(tract)) else {
                continue;
            };
            for (_, chunk) in column.chunks() {
                awake += u64::from(chunk.fields().awake());
            }
        }
        awake
    }

    /// The most active field cells before the weakest decay faster
    /// (02 §16).
    pub fn field_max_active_cells(&self) -> u32 {
        self.activity.fields.max_active
    }

    /// Sets the most active field cells before the weakest decay
    /// faster: the setting `field.max_active_cells`, until settings are
    /// bound.
    pub fn set_field_max_active_cells(&mut self, cells: u32) {
        self.activity.fields.max_active = cells;
    }

    /// Notes whether a loaded tract holds any field. Called when a
    /// column joins or leaves a grid.
    pub(crate) fn fields_column_changed(&mut self, grid: GridId, tract: TractPos) {
        let holds = self
            .grid(grid)
            .ok()
            .and_then(|view| view.column(tract))
            .is_some_and(column_has_fields);
        if holds {
            self.activity.fields.tracts.insert((grid, tract));
        } else if !self.activity.fields.tracts.is_empty() {
            self.activity.fields.tracts.remove(&(grid, tract));
        }
    }

    /// Sets the level of a field in a cell, which must be normalised,
    /// and says what that did. A level of zero takes the field away.
    /// A cell that gains a field it had none of is awake if anything
    /// steps the field, and whoever is named becomes responsible for
    /// it unless someone already is.
    ///
    /// Emits nothing: whoever calls this reports what it did.
    pub(crate) fn write_field(
        &mut self,
        loc: Location,
        field: FieldId,
        level: u8,
        by: Option<EntityId>,
    ) -> Result<FieldWrite, WorldError> {
        let (live, rests) = match self.defs().fields().get(field) {
            Some(def) => (def.live, def.rests),
            None => {
                return Err(WorldError::BadValue {
                    what: "a field",
                    why: "no field definition has this id",
                });
            }
        };
        let cell = loc.cell;
        let (chunk_z, idx) = (cell.z >> CHUNK_Z_BITS, cell.idx());
        let parts = self.grid_mut(loc.grid)?;
        let defs = parts.defs;
        let column = parts
            .grid
            .column_mut(cell.tract())
            .ok_or(WorldError::NotLoaded(loc))?;
        let held = store_at(column, cell).and_then(|store| store.cell(field, idx).copied());
        let old = held.map_or(0, |held| held.level);
        let by = held.and_then(|held| held.by).or(by);
        if old == level {
            return Ok(FieldWrite {
                old,
                new: level,
                by,
            });
        }
        if !column.can_store(chunk_z) {
            return Err(WorldError::OutOfReach(loc));
        }
        let chunk = column
            .chunk_mut(defs, chunk_z)
            .ok_or(WorldError::OutOfReach(loc))?;
        let store = chunk.fields_mut();
        let was_empty = store.is_empty();
        let woke = live && !held.is_some_and(|held| held.awake);
        store.set(
            field,
            idx,
            (level > 0).then_some(FieldCell {
                level,
                awake: live,
                by,
            }),
        );
        // Whatever else is in the cell has something new beside it.
        let mut woken: Vec<FieldId> = Vec::new();
        if store.any_asleep() {
            let asleep: Vec<FieldId> = store
                .at(idx)
                .filter(|(other, held)| *other != field && !held.awake)
                .map(|(other, _)| other)
                .filter(|other| defs.fields().get(*other).is_some_and(|def| def.live))
                .collect();
            for other in asleep {
                store.set_awake(other, idx, true);
                woken.push(other);
            }
        }
        let now_empty = store.is_empty();
        let gone = now_empty && !column_has_fields(column);

        let state = &mut self.activity.fields;
        if was_empty && level > 0 {
            state.tracts.insert((loc.grid, cell.tract()));
        } else if gone {
            state.tracts.remove(&(loc.grid, cell.tract()));
        }
        if woke && level > 0 {
            state.woken.push((loc, field));
        }
        state
            .woken
            .extend(woken.into_iter().map(|other| (loc, other)));
        if rests {
            // A level beside a sleeping cell has changed: it may have
            // somewhere to go.
            for step in FIELD_STEPS {
                self.wake_field(Location::new(loc.grid, cell.offset(step)), field);
            }
        }
        Ok(FieldWrite {
            old,
            new: level,
            by,
        })
    }

    /// Wakes a cell of a field, if it holds any and is asleep.
    fn wake_field(&mut self, loc: Location, field: FieldId) {
        let Ok(parts) = self.grid_mut(loc.grid) else {
            return;
        };
        let Ok(cell) = parts.topology.normalize(loc.cell) else {
            return;
        };
        let Some(column) = parts.grid.column_mut(cell.tract()) else {
            return;
        };
        let asleep = store_at(column, cell)
            .and_then(|store| store.cell(field, cell.idx()))
            .is_some_and(|held| !held.awake);
        if !asleep {
            return;
        }
        if let Some(chunk) = column.chunk_mut(parts.defs, cell.z >> CHUNK_Z_BITS) {
            chunk.fields_mut().set_awake(field, cell.idx(), true);
            self.activity
                .fields
                .woken
                .push((Location::new(loc.grid, cell), field));
        }
    }

    /// Wakes every sleeping field of a cell that anything steps.
    fn wake_fields_at(&mut self, loc: Location) {
        let Ok(parts) = self.grid_mut(loc.grid) else {
            return;
        };
        let Ok(cell) = parts.topology.normalize(loc.cell) else {
            return;
        };
        let Some(column) = parts.grid.column_mut(cell.tract()) else {
            return;
        };
        let defs = parts.defs;
        let asleep: Vec<FieldId> = match store_at(column, cell) {
            Some(store) if store.any_asleep() => store
                .at(cell.idx())
                .filter(|(_, held)| !held.awake)
                .map(|(field, _)| field)
                .filter(|field| defs.fields().get(*field).is_some_and(|def| def.live))
                .collect(),
            _ => return,
        };
        if asleep.is_empty() {
            return;
        }
        if let Some(chunk) = column.chunk_mut(defs, cell.z >> CHUNK_Z_BITS) {
            let store = chunk.fields_mut();
            for field in &asleep {
                store.set_awake(*field, cell.idx(), true);
            }
            let loc = Location::new(loc.grid, cell);
            self.activity
                .fields
                .woken
                .extend(asleep.into_iter().map(|field| (loc, field)));
        }
    }

    /// Puts a cell of a field to sleep.
    pub(crate) fn rest_field(&mut self, loc: Location, field: FieldId) {
        let Ok(parts) = self.grid_mut(loc.grid) else {
            return;
        };
        let Some(column) = parts.grid.column_mut(loc.cell.tract()) else {
            return;
        };
        let awake = store_at(column, loc.cell)
            .and_then(|store| store.cell(field, loc.cell.idx()))
            .is_some_and(|held| held.awake);
        if awake && let Some(chunk) = column.chunk_mut(parts.defs, loc.cell.z >> CHUNK_Z_BITS) {
            chunk.fields_mut().set_awake(field, loc.cell.idx(), false);
        }
    }
}

impl WorldCtx<'_> {
    /// Reports what a write to a field did: `FieldAppeared` where the
    /// cell had none of it, with `FieldContact` for whoever is in the
    /// cell, and `FieldVanished` where it has none left.
    pub(crate) fn field_written(
        &mut self,
        loc: Location,
        field: FieldId,
        write: &FieldWrite,
        cause: Cause,
    ) {
        if write.old == write.new {
            return;
        }
        let tick = self.timeline.now();
        if write.old == 0 {
            self.events.emit(WorldEvent::FieldAppeared {
                tick,
                loc,
                field,
                level: write.new,
                cause,
            });
            if self
                .cosmos
                .is_occupied(loc, &OccupantFilter::ANY)
                .unwrap_or(false)
                && let Ok(found) = self.cosmos.occupants_at(loc, &OccupantFilter::ANY)
            {
                for occupant in distinct_ids(&found) {
                    self.events.emit(WorldEvent::FieldContact {
                        tick,
                        occupant,
                        loc,
                        field,
                        level: write.new,
                        cause,
                    });
                }
            }
        } else if write.new == 0 {
            self.events.emit(WorldEvent::FieldVanished {
                tick,
                loc,
                field,
                cause,
            });
        }
    }

    /// Sees to it that every cell woken since this was last called has
    /// a process that steps it.
    pub(crate) fn fields_woken(&mut self) {
        if self.cosmos.activity.fields.woken.is_empty() {
            return;
        }
        let woken = core::mem::take(&mut self.cosmos.activity.fields.woken);
        let mut wanted: BTreeSet<FieldStep> = BTreeSet::new();
        for (loc, field) in woken {
            let Some(step) = self.cosmos.defs().fields().get(field).map(|def| def.step) else {
                continue;
            };
            if let Ok(ambient) = self.cosmos.ambient(loc) {
                wanted.insert(FieldStep {
                    clock: ambient.clock,
                    step,
                });
            }
        }
        for step in wanted {
            self.ensure_field_step(step);
        }
    }

    /// Sees to it that a process steps the fields of a clock and a step
    /// interval. It first wakes at the next whole multiple of the
    /// interval in the clock's local time.
    pub(crate) fn ensure_field_step(&mut self, step: FieldStep) {
        let activity = &mut self.cosmos.activity;
        let slot = activity
            .fields
            .steps
            .binary_search_by_key(&step, |(have, _)| *have);
        if let Ok(at) = slot {
            if activity.processes.contains(activity.fields.steps[at].1) {
                return;
            }
            activity.fields.steps.remove(at);
        }
        let Ok(local) = self.timeline.local_now(step.clock) else {
            return;
        };
        let span = step.step.as_micros().max(1);
        let first = Dur::from_micros(span - local.as_micros().rem_euclid(span));
        let spawned = activity.processes.spawn(
            self.timeline,
            step.clock,
            first,
            WorldProcess::FieldStep(step),
        );
        if let Ok(id) = spawned {
            let at = activity
                .fields
                .steps
                .binary_search_by_key(&step, |(have, _)| *have)
                .unwrap_or_else(|at| at);
            activity.fields.steps.insert(at, (step, id));
        }
    }

    /// Forgets the process that stepped a clock and a step interval: it
    /// has ended.
    pub(crate) fn forget_field_step(&mut self, step: FieldStep) {
        let steps = &mut self.cosmos.activity.fields.steps;
        if let Ok(at) = steps.binary_search_by_key(&step, |(have, _)| *have) {
            steps.remove(at);
        }
    }

    /// Sees to it that every active field cell has a process that steps
    /// it, on the clock of where it is. Called when an area's clock
    /// comes, goes or moves, and when a tract with fields is loaded;
    /// whoever restores a saved world without its timeline calls it
    /// too.
    pub fn resume_fields(&mut self) {
        let mut wanted: BTreeSet<FieldStep> = BTreeSet::new();
        for (grid, tract) in self.cosmos.activity.fields.tracts() {
            let Ok(view) = self.cosmos.grid(grid) else {
                continue;
            };
            let Some(column) = view.column(tract) else {
                continue;
            };
            let clocked = step::tract_has_clocks(self.cosmos, grid, tract);
            for (chunk_z, chunk) in column.chunks() {
                for (field, layer) in chunk.fields().layers() {
                    if layer.awake() == 0 {
                        continue;
                    }
                    let Some(def) = self.cosmos.defs().fields().get(field) else {
                        continue;
                    };
                    if !clocked {
                        wanted.insert(FieldStep {
                            clock: view.clock(),
                            step: def.step,
                        });
                        continue;
                    }
                    for (idx, held) in layer.cells() {
                        let cell = tract.cell(idx.column(), (chunk_z << CHUNK_Z_BITS) + idx.lz());
                        if held.awake
                            && let Ok(ambient) = view.ambient(cell)
                        {
                            wanted.insert(FieldStep {
                                clock: ambient.clock,
                                step: def.step,
                            });
                        }
                    }
                }
            }
        }
        for step in wanted {
            self.ensure_field_step(step);
        }
    }

    /// An element of a cell has changed: every sleeping field in the
    /// cell and in the cells beside, above and below it wakes, for it
    /// may have somewhere to go.
    pub(crate) fn fields_disturbed(&mut self, loc: Location) {
        if self.cosmos.activity.fields.tracts.is_empty() {
            return;
        }
        self.cosmos.wake_fields_at(loc);
        for step in FIELD_STEPS {
            self.cosmos
                .wake_fields_at(Location::new(loc.grid, loc.cell.offset(step)));
        }
        self.fields_woken();
    }

    /// An occupant has come to fill these cells of a grid: for each
    /// that it did not fill before and that holds a field,
    /// `FieldContact` is emitted for each field there.
    pub(crate) fn fields_met(
        &mut self,
        occupant: EntityId,
        grid: GridId,
        cells: &[CellPos],
        was: Option<(Location, Footprint)>,
        cause: Cause,
    ) {
        if self.cosmos.activity.fields.tracts.is_empty() {
            return;
        }
        let before = was
            .filter(|(from, _)| from.grid == grid)
            .map(|(from, footprint)| footprint.cells(from.cell));
        let tick = self.timeline.now();
        let Ok(view) = self.cosmos.grid(grid) else {
            return;
        };
        for cell in cells {
            if before.is_some_and(|before| before.contains(*cell)) {
                continue;
            }
            let Some(store) = view
                .column(cell.tract())
                .and_then(|column| store_at(column, *cell))
            else {
                continue;
            };
            for (field, held) in store.at(cell.idx()) {
                self.events.emit(WorldEvent::FieldContact {
                    tick,
                    occupant,
                    loc: Location::new(grid, *cell),
                    field,
                    level: held.level,
                    cause,
                });
            }
        }
    }

    /// A tract has been read back from the store: its fields catch up
    /// on the local time that passed while it was not loaded (02 §9.4
    /// step 4), each by its definition's `catch_up`, and those still
    /// active are stepped again.
    pub(crate) fn fields_loaded(&mut self, realm: RealmId, grid: GridId, tract: TractPos) {
        let left = self
            .cosmos
            .tract_record(realm, tract)
            .and_then(|record| record.extra.as_ref())
            .map(|extra| extra.last_loaded);
        let now = self
            .cosmos
            .realm(realm)
            .and_then(|realm| self.timeline.local_now(realm.clock).ok());
        let elapsed = match (left, now) {
            (Some(left), Some(now)) => (now.as_micros() - left.as_micros()).max(0),
            _ => 0,
        };
        self.fields_catch_up(grid, tract, Dur::from_micros(elapsed));
    }

    /// The fields of a loaded tract catch up on a span of local time
    /// that passed while nothing stepped them, each by its
    /// definition's `catch_up` (02 §16), and those still active are
    /// stepped again.
    pub(crate) fn fields_catch_up(&mut self, grid: GridId, tract: TractPos, elapsed: Dur) {
        let Some(column) = self
            .cosmos
            .grid(grid)
            .ok()
            .and_then(|view| view.column(tract))
        else {
            return;
        };
        if !column_has_fields(column) {
            return;
        }
        let elapsed = elapsed.as_micros().max(0);
        let mut held: Vec<(CellPos, FieldId, FieldCell)> = Vec::new();
        for (chunk_z, chunk) in column.chunks() {
            for (field, layer) in chunk.fields().layers() {
                for (idx, cell) in layer.cells() {
                    let at = tract.cell(idx.column(), (chunk_z << CHUNK_Z_BITS) + idx.lz());
                    held.push((at, field, *cell));
                }
            }
        }
        let defs = std::sync::Arc::clone(self.cosmos.defs());
        for (cell, field, was) in held {
            let Some(def) = defs.fields().get(field) else {
                continue;
            };
            let steps = elapsed / def.step.as_micros().max(1);
            let level = match def.catch_up {
                CatchUp::Keep | CatchUp::Settle => was.level,
                CatchUp::None => 0,
                CatchUp::BurnOut if steps >= 1 => 0,
                CatchUp::BurnOut => was.level,
                CatchUp::Disperse => {
                    let lost = i64::from(def.decay()).saturating_mul(steps);
                    let left = (i64::from(was.level) - lost).max(0);
                    let halvings = (steps / DISPERSE_HALF_STEPS).min(i64::from(u8::BITS));
                    u8::try_from(left >> halvings).unwrap_or(0)
                }
            };
            let loc = Location::new(grid, cell);
            if level != was.level {
                if let Ok(write) = self.cosmos.write_field(loc, field, level, was.by) {
                    let mut cause = field_cause(write.by);
                    cause.kind = CauseKind::Coarse;
                    self.field_written(loc, field, &write, cause);
                }
            } else if def.catch_up == CatchUp::Settle {
                self.cosmos.wake_field(loc, field);
            }
        }
        self.cosmos.activity.fields.woken.clear();
        self.resume_fields();
    }
}

impl WorldEdit<'_> {
    /// Adds to the level of a field in a cell, or with a negative
    /// amount takes from it, and returns the level it had (02 §16). A
    /// level stops at 0 and at 255.
    ///
    /// A cell that gains a field it had none of emits `FieldAppeared`,
    /// and `FieldContact` for each occupant in it; one that loses the
    /// last of a field emits `FieldVanished`. Other changes of level
    /// emit nothing: they bump the chunk's field revision. Whoever
    /// instigated this edit is responsible for the field in the cell,
    /// and for what it then does, unless someone already is.
    ///
    /// An edit may put a field into any loaded cell. The rules never
    /// move one into a cell whose block covers it.
    ///
    /// Refused, changing nothing, for a cell outside its grid, not
    /// loaded or out of reach of its tract's storage, and for a field
    /// no definition has.
    pub fn add_field(
        &mut self,
        loc: Location,
        field: FieldId,
        amount: i16,
    ) -> Result<u8, WorldError> {
        self.put_field(loc, field, |old| {
            let level = (i32::from(old) + i32::from(amount)).clamp(0, i32::from(FIELD_MAX_LEVEL));
            u8::try_from(level).unwrap_or(FIELD_MAX_LEVEL)
        })
    }

    /// Sets the level of a field in a cell and returns the level it
    /// had. Zero takes the field away. Otherwise as
    /// [`add_field`](WorldEdit::add_field).
    pub fn set_field(
        &mut self,
        loc: Location,
        field: FieldId,
        level: u8,
    ) -> Result<u8, WorldError> {
        self.put_field(loc, field, |_| level)
    }

    fn put_field(
        &mut self,
        loc: Location,
        field: FieldId,
        level: impl FnOnce(u8) -> u8,
    ) -> Result<u8, WorldError> {
        let loc = self.ctx.cosmos.normalize(loc)?;
        let old = self.ctx.cosmos.field(loc, field)?;
        let cause = self.cause;
        let write = self
            .ctx
            .cosmos
            .write_field(loc, field, level(old), cause.instigator)?;
        self.ctx.field_written(loc, field, &write, cause);
        self.ctx.fields_woken();
        Ok(old)
    }

    /// Tells the fields of a cell that a packet of damage has arrived
    /// in it (02 §16), and returns how many other cells it was passed
    /// on to.
    ///
    /// For each field in the cell that reacts to the packet's damage
    /// type by spreading it, every cell joined to this one through
    /// that field, within the reaction's reach, gets a packet with the
    /// reaction's share of the energy and of the intensity: its
    /// elements are struck, and each occupant in it is told with
    /// `OccupantExposed`. The cell the packet arrived in gets nothing
    /// more. Whoever delivers a packet to a cell calls this once.
    pub fn conduct(&mut self, loc: Location, packet: DamagePacket) -> Result<u32, WorldError> {
        self.ctx.conduct(loc, packet)
    }
}
