//! The field stepper (02 §16): what one process does at each step for
//! the active cells of its clock and step interval, and how a field
//! passes on damage that arrives in it.
//!
//! A step has three parts. First everything is read and nothing is
//! changed: the reactions and then the rules of every active cell say
//! what they would add to or take from which cell, what they would
//! strike and whom they would dose. Then the changes of level are put
//! in order of cell and written, each cell once. Last the elements are
//! struck and the occupants told, in order. So no cell ever sees
//! another half way through a step.

use std::collections::{BTreeMap, VecDeque};

use clash_core::{Dur, EntityId, HashDomain, IVec3, LookupMap, Real, pos_hash};
use clash_time::Next;

use super::{
    FIELD_CONDUCT_MAX_CELLS, FIELD_OVER_BUDGET_DECAY, FIELD_STEPS, Field, FieldReacts, FieldRule,
    FieldStep, MaterialTagSet, VerticalBias, field_cause, level_in, store_at,
};
use crate::area::{FIELD_BLOCK_COVERAGE, passage};
use crate::chunk::FieldId;
use crate::cosmos::{Cosmos, GridView};
use crate::damage::DamagePacket;
use crate::defs::{DamageTypeId, EngineFlag, Layer, WorldDefs};
use crate::dims::{CELL_M, CHUNK_Z_BITS, STOREY_M};
use crate::edit::WorldCtx;
use crate::error::WorldError;
use crate::event::WorldEvent;
use crate::field::ExposureRoute;
use crate::grid::Column;
use crate::occupancy::{OccupantFilter, distinct_ids};
use crate::pos::{CellPos, GridId, Location, TractPos};

/// What the positional hash that decides whether a field spreads is
/// for (01 §4).
const SPREAD_DOMAIN: HashDomain = HashDomain::new("world.field.spread");

/// The highest level, as a whole number to reckon with.
const FULL: i32 = u8::MAX as i32;

/// One active cell of a step: a field in a cell, as it stood when the
/// step began.
#[derive(Clone, Copy, Debug)]
struct Work {
    grid: GridId,
    cell: CellPos,
    field: FieldId,
    level: u8,
    by: Option<EntityId>,
}

/// A field in a cell: what a change of level is for.
type Slot = (GridId, CellPos, FieldId);

/// The changes of level a step asks for of one field in one cell,
/// summed, and who is responsible for what was brought there: of
/// several, the lowest id.
#[derive(Clone, Copy, Debug)]
struct Change {
    sum: i32,
    by: Option<EntityId>,
}

/// Elements to strike at the end of a step.
#[derive(Clone, Copy, Debug)]
struct Strike {
    loc: Location,
    packet: DamagePacket,
}

/// What an occupant met in a step: the strongest cell of one field's
/// rule among the cells it fills.
#[derive(Clone, Copy, Debug)]
struct Met {
    level: u8,
    loc: Location,
    by: Option<EntityId>,
}

/// Whether any area that touches a tract has a clock of its own: if
/// none has, every cell of the tract runs on its realm's clock.
pub(super) fn tract_has_clocks(cosmos: &Cosmos, grid: GridId, tract: TractPos) -> bool {
    cosmos.areas.in_tract(grid, tract).iter().any(|id| {
        cosmos
            .areas
            .get(*id)
            .is_some_and(|area| area.clock.is_some())
    })
}

/// Reads the cells of one grid for a step, keeping hold of the column
/// it last read.
struct Reader<'a> {
    view: GridView<'a>,
    defs: &'a WorldDefs,
    last: Option<(TractPos, &'a Column)>,
}

impl<'a> Reader<'a> {
    fn new(view: GridView<'a>) -> Reader<'a> {
        Reader {
            view,
            defs: view.defs(),
            last: None,
        }
    }

    /// The column of a normalised cell, if its tract is loaded.
    fn column(&mut self, cell: CellPos) -> Option<&'a Column> {
        let tract = cell.tract();
        if let Some((have, column)) = self.last
            && have == tract
        {
            return Some(column);
        }
        let column = self.view.column(tract)?;
        self.last = Some((tract, column));
        Some(column)
    }

    /// The cell a step away, and its column: `None` beyond an edge of
    /// the grid or where nothing is loaded. Neither lets a field
    /// through.
    fn beside(&mut self, cell: CellPos, step: IVec3) -> Option<(CellPos, &'a Column)> {
        let next = self.view.normalize(cell.offset(step)).ok()?;
        let column = self.column(next)?;
        Some((next, column))
    }

    /// Whether a field can pass from a cell to the one a step away:
    /// the block there does not cover its cell, and between storeys no
    /// floor that is not a grate lies between (02 §16).
    fn passes(
        &self,
        here: &Column,
        cell: CellPos,
        there: &Column,
        next: CellPos,
        step: IVec3,
    ) -> bool {
        let (admits, open_below) = passage(self.defs, there, next);
        if !admits {
            return false;
        }
        match step.z {
            z if z > 0 => open_below,
            z if z < 0 => passage(self.defs, here, cell).1,
            _ => true,
        }
    }

    /// Whether a field that spreads to what it needs can reach the
    /// cell a step away: it can pass there, or what stops it is itself
    /// of a material it needs.
    fn spreads(
        &self,
        needs: Option<&MaterialTagSet>,
        here: &Column,
        cell: CellPos,
        there: &Column,
        next: CellPos,
        step: IVec3,
    ) -> bool {
        let fuel = |material| needs.is_some_and(|set| set.has(material));
        let block = there.block(next);
        if self.defs.block_info(block).coverage >= FIELD_BLOCK_COVERAGE && !fuel(block.material()) {
            return false;
        }
        if step.z != 0 {
            // A floor lies at the base of its cell: going up it is the
            // upper cell's, going down this one's.
            let floor = if step.z > 0 {
                there.floor(next)
            } else {
                here.floor(cell)
            };
            let stops =
                !floor.is_none() && !self.defs.floor_info(floor).flags.has(EngineFlag::Grate);
            if stops && !fuel(floor.material()) {
                return false;
            }
        }
        true
    }
}

/// What a step has gathered so far.
#[derive(Default)]
struct Gathered {
    /// The changes of level, summed as they are asked for. Sums do not
    /// depend on the order they are made in, and the changes are
    /// written in order of cell.
    changes: LookupMap<Slot, Change>,
    /// What reactions use of each field in each cell this step, so
    /// that nothing is used twice.
    used: BTreeMap<(GridId, CellPos, FieldId), i32>,
    strikes: Vec<Strike>,
    exposed: BTreeMap<(EntityId, FieldId, usize), Met>,
    dosed: BTreeMap<(EntityId, FieldId, usize), Met>,
}

impl Gathered {
    fn push(&mut self, work: &Work, cell: CellPos, field: FieldId, amount: i32) {
        if amount == 0 {
            return;
        }
        let slot = (work.grid, cell, field);
        match self.changes.get_mut(&slot) {
            Some(change) => {
                change.sum = change.sum.saturating_add(amount);
                change.by = match (change.by, work.by) {
                    (Some(have), Some(more)) => Some(have.min(more)),
                    (have, more) => have.or(more),
                };
            }
            None => {
                self.changes.insert(
                    slot,
                    Change {
                        sum: amount,
                        by: work.by,
                    },
                );
            }
        }
    }

    /// What is left of a field in a cell for this step's reactions.
    fn left(&self, grid: GridId, cell: CellPos, field: FieldId, level: u8) -> i32 {
        let used = if self.used.is_empty() {
            0
        } else {
            self.used.get(&(grid, cell, field)).copied().unwrap_or(0)
        };
        (i32::from(level) - used).max(0)
    }

    fn use_up(&mut self, grid: GridId, cell: CellPos, field: FieldId, amount: i32) {
        if amount > 0 {
            *self.used.entry((grid, cell, field)).or_insert(0) += amount;
        }
    }

    /// Notes that an occupant met a field's rule in a cell, keeping
    /// the strongest cell for each.
    fn meet(
        met: &mut BTreeMap<(EntityId, FieldId, usize), Met>,
        column: &Column,
        work: &Work,
        rule: usize,
    ) {
        let Some(index) = column.occupants_in(work.cell.z >> CHUNK_Z_BITS) else {
            return;
        };
        for entry in index.at(work.cell.idx()) {
            let found = Met {
                level: work.level,
                loc: Location::new(work.grid, work.cell),
                by: work.by,
            };
            met.entry((entry.id, work.field, rule))
                .and_modify(|have| {
                    if found.level > have.level {
                        *have = found;
                    }
                })
                .or_insert(found);
        }
    }
}

/// `share` of a level, rounded down.
fn share_of(level: i32, share: Real) -> i32 {
    (Real::from_i64(i64::from(level)) * share)
        .floor()
        .to_i32()
        .max(0)
}

/// Takes up to `want` from what a cell has left to give this step.
fn take(left: &mut i32, want: i32) -> i32 {
    let taken = want.clamp(0, (*left).max(0));
    *left -= taken;
    taken
}

/// What one rule would move out of a cell in a step: so much to each of
/// up to six neighbours.
struct Moves {
    to: [(CellPos, i32); 6],
    count: usize,
}

impl Moves {
    fn new(cell: CellPos) -> Moves {
        Moves {
            to: [(cell, 0); 6],
            count: 0,
        }
    }

    fn want(&mut self, next: CellPos, amount: i32) {
        if amount > 0 && self.count < self.to.len() {
            self.to[self.count] = (next, amount);
            self.count += 1;
        }
    }

    /// Moves what is wanted out of a cell. If the cell has less left
    /// than its neighbours want together, each gets its share of what
    /// there is, rounded down, so that none is favoured by its place in
    /// the order.
    fn make(&self, out: &mut Gathered, work: &Work, left: &mut i32) {
        let wanted = &self.to[..self.count];
        let all: i64 = wanted.iter().map(|(_, amount)| i64::from(*amount)).sum();
        let have = i64::from((*left).max(0));
        if all <= 0 || have <= 0 {
            return;
        }
        for (next, amount) in wanted {
            let moved = if all > have {
                i64::from(*amount) * have / all
            } else {
                i64::from(*amount)
            };
            let moved = i32::try_from(moved).unwrap_or(0);
            out.push(work, work.cell, work.field, -moved);
            out.push(work, *next, work.field, moved);
            *left -= moved;
        }
    }
}

impl Cosmos {
    /// The active cells a step process steps, in order of grid, tract,
    /// chunk from the lowest, field and cell; and how many cells are
    /// active in all.
    fn field_work(&self, step: FieldStep) -> (Vec<Work>, u64) {
        let defs = self.defs();
        let mut work: Vec<Work> = Vec::new();
        let mut awake = 0u64;
        for (grid, tract) in self.activity.fields.tracts() {
            let Ok(view) = self.grid(grid) else {
                continue;
            };
            let Some(column) = view.column(tract) else {
                continue;
            };
            let clocked = tract_has_clocks(self, grid, tract);
            let here = clocked || view.clock() == step.clock;
            for (chunk_z, chunk) in column.chunks() {
                for (field, layer) in chunk.fields().layers() {
                    awake += u64::from(layer.awake());
                    if !here || layer.awake() == 0 {
                        continue;
                    }
                    if defs
                        .fields()
                        .get(field)
                        .is_none_or(|def| def.step != step.step)
                    {
                        continue;
                    }
                    for (idx, held) in layer.cells() {
                        if !held.awake {
                            continue;
                        }
                        let cell = tract.cell(idx.column(), (chunk_z << CHUNK_Z_BITS) + idx.lz());
                        if clocked
                            && view.ambient(cell).map(|ambient| ambient.clock) != Ok(step.clock)
                        {
                            continue;
                        }
                        work.push(Work {
                            grid,
                            cell,
                            field,
                            level: held.level,
                            by: held.by,
                        });
                    }
                }
            }
        }
        (work, awake)
    }

    /// The level at or below which an active cell is among the weakest
    /// `over` of them all.
    fn weakest_level(&self, over: u64) -> u8 {
        let mut by_level = [0u64; 256];
        for (grid, tract) in self.activity.fields.tracts() {
            let Some(column) = self.grid(grid).ok().and_then(|view| view.column(tract)) else {
                continue;
            };
            for (_, chunk) in column.chunks() {
                for (_, layer) in chunk.fields().layers() {
                    for (_, held) in layer.cells() {
                        if held.awake {
                            by_level[usize::from(held.level)] += 1;
                        }
                    }
                }
            }
        }
        let mut below = 0u64;
        for (level, count) in by_level.iter().enumerate() {
            below += count;
            if below >= over {
                return u8::try_from(level).unwrap_or(u8::MAX);
            }
        }
        u8::MAX
    }

    /// The seed positional choices in a grid are made from: its
    /// realm's.
    fn field_seed(&self, grid: GridId) -> u64 {
        self.realm_of(grid)
            .and_then(|realm| self.realm(realm))
            .map_or(0, |realm| realm.seed)
    }
}

/// What one step needs to know besides the cells.
struct StepCx {
    /// The number of the step: the clock's local time in step
    /// intervals.
    number: u64,
    /// The step interval in seconds.
    seconds: Real,
    /// With more active cells than the budget allows: the level at or
    /// below which a cell decays faster.
    cut: Option<u8>,
    seed: u64,
}

/// Gathers what the reactions of one active cell do this step.
fn react(reader: &mut Reader<'_>, work: &Work, def: &Field, out: &mut Gathered) {
    if def.reacts.is_empty() {
        return;
    }
    let Some(column) = reader.column(work.cell) else {
        return;
    };
    let store = store_at(column, work.cell);
    let idx = work.cell.idx();
    for reacts in &def.reacts {
        match reacts {
            FieldReacts::With {
                other,
                first,
                use_self,
                use_other,
                gives,
                max,
            } => {
                let Some(partner) = store.and_then(|store| store.cell(*other, idx)) else {
                    continue;
                };
                // The reaction runs at the steps of the field it names
                // first, where that one is awake.
                if !*first && partner.awake {
                    continue;
                }
                let mine = out.left(work.grid, work.cell, work.field, work.level);
                let theirs = out.left(work.grid, work.cell, *other, partner.level);
                let (use_self, use_other) = (i32::from(*use_self), i32::from(*use_other));
                let mut times = if use_self == 0 && use_other == 0 {
                    1
                } else {
                    i32::MAX
                };
                if use_self > 0 {
                    times = times.min(mine / use_self);
                }
                if use_other > 0 {
                    times = times.min(theirs / use_other);
                }
                let times = times.min(i32::try_from(*max).unwrap_or(i32::MAX));
                if times <= 0 || mine <= 0 || theirs <= 0 {
                    continue;
                }
                out.push(work, work.cell, work.field, -use_self * times);
                out.push(work, work.cell, *other, -use_other * times);
                out.use_up(work.grid, work.cell, work.field, use_self * times);
                out.use_up(work.grid, work.cell, *other, use_other * times);
                for (given, amount) in gives {
                    out.push(
                        work,
                        work.cell,
                        *given,
                        i32::from(*amount).saturating_mul(times),
                    );
                }
            }
            FieldReacts::Material {
                set,
                use_self,
                gives,
                max,
            } => {
                if !set.in_cell(&column.cell(work.cell)) {
                    continue;
                }
                let mine = out.left(work.grid, work.cell, work.field, work.level);
                let use_self = i32::from(*use_self);
                let times = if use_self == 0 { 1 } else { mine / use_self };
                let times = times.min(i32::try_from(*max).unwrap_or(i32::MAX));
                if times <= 0 || mine <= 0 {
                    continue;
                }
                out.push(work, work.cell, work.field, -use_self * times);
                out.use_up(work.grid, work.cell, work.field, use_self * times);
                for (given, amount) in gives {
                    out.push(
                        work,
                        work.cell,
                        *given,
                        i32::from(*amount).saturating_mul(times),
                    );
                }
            }
        }
    }
}

/// Gathers what the rules of one active cell do this step.
fn rule(reader: &mut Reader<'_>, cx: &StepCx, work: &Work, def: &Field, out: &mut Gathered) {
    let Some(here) = reader.column(work.cell) else {
        return;
    };
    let (cell, field) = (work.cell, work.field);
    let level = i32::from(work.level);
    // What the cell has left to give: nothing leaves it twice.
    let mut left = out.left(work.grid, cell, field, work.level);
    if cx.cut.is_some_and(|cut| work.level <= cut) {
        let lost = take(&mut left, i32::from(FIELD_OVER_BUDGET_DECAY));
        out.push(work, cell, field, -lost);
    }
    // How the medium moves here, for the rules that ask.
    let carried = def.rules.iter().find_map(|rule| match rule {
        FieldRule::Drift { share, spread } => Some((*share, *spread)),
        _ => None,
    });
    let drift = match carried {
        Some(_) => reader.view.field_drift(cell).ok().flatten(),
        None => None,
    };

    for (position, rule) in def.rules.iter().enumerate() {
        match rule {
            FieldRule::Decay { amount } => {
                let lost = take(&mut left, i32::from(*amount));
                out.push(work, cell, field, -lost);
            }
            FieldRule::Diffuse { rate, vertical } => {
                let steps = match vertical {
                    VerticalBias::Even => &FIELD_STEPS[..],
                    _ => &FIELD_STEPS[..4],
                };
                let mut moves = Moves::new(cell);
                for step in steps {
                    let Some((next, there)) = reader.beside(cell, *step) else {
                        continue;
                    };
                    if !reader.passes(here, cell, there, next, *step) {
                        continue;
                    }
                    let beside = i32::from(level_in(there, next, field));
                    if level > beside {
                        moves.want(next, share_of(level - beside, *rate));
                    }
                }
                let bias = match vertical {
                    VerticalBias::Even => None,
                    VerticalBias::Rise(share) => Some((FIELD_STEPS[4], *share)),
                    VerticalBias::Sink(share) => Some((FIELD_STEPS[5], *share)),
                };
                if let Some((step, share)) = bias
                    && let Some((next, there)) = reader.beside(cell, step)
                    && reader.passes(here, cell, there, next, step)
                {
                    let room = FULL - i32::from(level_in(there, next, field));
                    moves.want(next, share_of(level, share).min(room));
                }
                moves.make(out, work, &mut left);
            }
            FieldRule::Flow { rate } => {
                let down = FIELD_STEPS[5];
                if let Some((next, there)) = reader.beside(cell, down)
                    && reader.passes(here, cell, there, next, down)
                {
                    let room = FULL - i32::from(level_in(there, next, field));
                    let want = left.min(room);
                    let fell = take(&mut left, want);
                    out.push(work, cell, field, -fell);
                    out.push(work, next, field, fell);
                }
                // What could not fall levels out sideways.
                let standing = left;
                let mut moves = Moves::new(cell);
                for step in &FIELD_STEPS[..4] {
                    let Some((next, there)) = reader.beside(cell, *step) else {
                        continue;
                    };
                    if !reader.passes(here, cell, there, next, *step) {
                        continue;
                    }
                    let beside = i32::from(level_in(there, next, field));
                    if standing > beside {
                        moves.want(next, share_of(standing - beside, *rate));
                    }
                }
                moves.make(out, work, &mut left);
            }
            FieldRule::Spread {
                needs,
                chance,
                amount,
                from,
            } => {
                if work.level < *from {
                    continue;
                }
                for (way, step) in FIELD_STEPS.iter().enumerate() {
                    let Some((next, there)) = reader.beside(cell, *step) else {
                        continue;
                    };
                    if level_in(there, next, field) > 0 {
                        continue;
                    }
                    if needs
                        .as_ref()
                        .is_some_and(|set| !set.in_cell(&there.cell(next)))
                    {
                        continue;
                    }
                    if !reader.spreads(needs.as_ref(), here, cell, there, next, *step) {
                        continue;
                    }
                    // Likelier the way the medium moves, less likely
                    // against it.
                    let along = match (drift, carried) {
                        (Some(velocity), Some((_, bias))) => {
                            let toward = velocity[0] * Real::from_i64(i64::from(step.x))
                                + velocity[1] * Real::from_i64(i64::from(step.y))
                                + velocity[2] * Real::from_i64(i64::from(step.z));
                            (Real::ONE + bias * toward).max(Real::ZERO)
                        }
                        _ => Real::ONE,
                    };
                    let odds = (*chance * along).min(Real::ONE);
                    let salt = (cx.number << 24)
                        ^ (u64::from(field.raw()) << 8)
                        ^ ((position as u64 & 0x1f) << 3)
                        ^ way as u64;
                    let hash = pos_hash(cx.seed, SPREAD_DOMAIN, cell.x, cell.y, cell.z, salt);
                    // The top 53 bits, as a number from 0 up to 1.
                    let roll = Real::of((hash >> 11) as f64 / (1u64 << 53) as f64);
                    if roll < odds {
                        out.push(work, next, field, i32::from(*amount));
                    }
                }
            }
            FieldRule::Emit {
                field: other,
                amount,
                from,
            } => {
                if work.level >= *from {
                    out.push(work, cell, *other, i32::from(*amount));
                }
            }
            FieldRule::Convert { .. } => {}
            FieldRule::Damage {
                kind,
                energy_per_level,
                intensity_per_level,
                from,
            } => {
                if work.level < *from {
                    continue;
                }
                if !here.cell(cell).is_empty() {
                    let amount = Real::from_i64(i64::from(level));
                    out.strikes.push(Strike {
                        loc: Location::new(work.grid, cell),
                        packet: DamagePacket::new(
                            *kind,
                            amount * *energy_per_level,
                            amount * *intensity_per_level,
                            field_cause(work.by),
                        ),
                    });
                }
                Gathered::meet(&mut out.exposed, here, work, position);
            }
            FieldRule::Drift { share, .. } => {
                let Some(velocity) = drift else {
                    continue;
                };
                // How much of a cell the medium crosses in a step, at
                // most all of it: a metre beside, a storey up or down.
                let have = left;
                let axes = [
                    (velocity[0], CELL_M, IVec3::new(1, 0, 0)),
                    (velocity[1], CELL_M, IVec3::new(0, 1, 0)),
                    (velocity[2], STOREY_M, IVec3::new(0, 0, 1)),
                ];
                let mut moves = Moves::new(cell);
                for (speed, size, axis) in axes {
                    if speed == Real::ZERO {
                        continue;
                    }
                    let step = if speed > Real::ZERO {
                        axis
                    } else {
                        IVec3::new(-axis.x, -axis.y, -axis.z)
                    };
                    let crossed = (speed.abs() * cx.seconds / size).min(Real::ONE) * *share;
                    let Some((next, there)) = reader.beside(cell, step) else {
                        continue;
                    };
                    if !reader.passes(here, cell, there, next, step) {
                        continue;
                    }
                    let room = FULL - i32::from(level_in(there, next, field));
                    moves.want(next, share_of(have, crossed).min(room));
                }
                moves.make(out, work, &mut left);
            }
            FieldRule::Expose { from, .. } => {
                if work.level >= *from {
                    Gathered::meet(&mut out.dosed, here, work, position);
                }
            }
        }
    }
}

impl WorldCtx<'_> {
    /// One step of the fields of a clock and a step interval (02 §16).
    /// Returns when the process is to wake next, or that it has
    /// nothing left to step.
    pub(crate) fn run_field_step(&mut self, step: FieldStep) -> Next {
        let Ok(local) = self.timeline.local_now(step.clock) else {
            // The clock has gone, and its cells are on another now.
            self.forget_field_step(step);
            self.resume_fields();
            return Next::Done;
        };
        let span = step.step.as_micros().max(1);
        // The process wakes at whole multiples of its interval: the
        // nearest one is the number of this step.
        let number = (local.as_micros() + span / 2).div_euclid(span);
        let (work, awake) = self.cosmos.field_work(step);
        if work.is_empty() {
            self.forget_field_step(step);
            return Next::Done;
        }
        let tick = self.timeline.now();

        // The budget (02 §16): past it the weakest cells decay faster.
        let max = u64::from(self.cosmos.activity.fields.max_active);
        let cut = (awake > max).then(|| self.cosmos.weakest_level(awake - max));
        if cut.is_some() && !self.cosmos.activity.fields.over_budget {
            self.events.emit(WorldEvent::FieldBudgetExceeded {
                tick,
                loc: Location::new(work[0].grid, work[0].cell),
                active: awake,
                max,
                cause: field_cause(None),
            });
        }
        self.cosmos.activity.fields.over_budget = cut.is_some();

        // Read everything; change nothing.
        let mut out = Gathered::default();
        {
            let cosmos = &*self.cosmos;
            let defs = cosmos.defs();
            let mut cx = StepCx {
                number: u64::try_from(number).unwrap_or(0),
                seconds: step.step.as_secs_real(),
                cut,
                seed: 0,
            };
            let mut reader: Option<Reader<'_>> = None;
            for pass in 0..2 {
                for item in &work {
                    if reader
                        .as_ref()
                        .is_none_or(|reader| reader.view.id() != item.grid)
                    {
                        let Ok(view) = cosmos.grid(item.grid) else {
                            continue;
                        };
                        cx.seed = cosmos.field_seed(item.grid);
                        reader = Some(Reader::new(view));
                    }
                    let (Some(reader), Some(def)) =
                        (reader.as_mut(), defs.fields().get(item.field))
                    else {
                        continue;
                    };
                    if pass == 0 {
                        react(reader, item, def, &mut out);
                    } else {
                        rule(reader, &cx, item, def, &mut out);
                    }
                }
            }
        }

        // A cell that only moves, and that nothing was moved into or
        // out of, goes to sleep. This comes before the writes, which
        // wake whatever they change the surroundings of.
        for item in &work {
            let rests = self
                .cosmos
                .defs()
                .fields()
                .get(item.field)
                .is_some_and(|def| def.rests);
            let touched = out
                .changes
                .contains_key(&(item.grid, item.cell, item.field));
            if rests && !touched {
                self.cosmos
                    .rest_field(Location::new(item.grid, item.cell), item.field);
            }
        }

        // Write every change of level, each cell once, in order of
        // cell and then of field.
        for ((grid, cell, field), change) in core::mem::take(&mut out.changes).into_sorted_vec() {
            let loc = Location::new(grid, cell);
            let Ok(old) = self.cosmos.field(loc, field) else {
                continue;
            };
            let level =
                u8::try_from((i32::from(old) + change.sum).clamp(0, FULL)).unwrap_or(u8::MAX);
            if let Ok(write) = self.cosmos.write_field(loc, field, level, change.by) {
                self.field_written(loc, field, &write, field_cause(write.by));
            }
        }

        // Strike the elements, then tell the occupants.
        for strike in &out.strikes {
            let mut edit = self.edit(strike.packet.cause);
            for layer in [Layer::Floor, Layer::Block, Layer::Fixture] {
                let _ = edit.damage(strike.loc, layer, strike.packet);
            }
        }
        let defs = std::sync::Arc::clone(self.cosmos.defs());
        for ((occupant, field, position), met) in &out.exposed {
            let Some(FieldRule::Damage {
                kind,
                energy_per_level,
                intensity_per_level,
                ..
            }) = defs
                .fields()
                .get(*field)
                .and_then(|def| def.rules.get(*position))
            else {
                continue;
            };
            let amount = Real::from_i64(i64::from(met.level));
            self.events.emit(WorldEvent::OccupantExposed {
                tick,
                occupant: *occupant,
                loc: met.loc,
                kind: *kind,
                energy: amount * *energy_per_level,
                intensity: amount * *intensity_per_level,
                cause: field_cause(met.by),
            });
        }
        for ((occupant, field, position), met) in &out.dosed {
            let Some(FieldRule::Expose {
                pool,
                per_level,
                route,
                ..
            }) = defs
                .fields()
                .get(*field)
                .and_then(|def| def.rules.get(*position))
            else {
                continue;
            };
            self.dose(tick, *occupant, *field, met, pool, *per_level, *route);
        }

        self.fields_woken();
        let next = (number + 1).saturating_mul(span) - local.as_micros();
        Next::Sleep(Dur::from_micros(next.max(1)))
    }

    #[allow(
        clippy::too_many_arguments,
        reason = "one event's fields, gathered in one place"
    )]
    fn dose(
        &mut self,
        tick: clash_core::Tick,
        occupant: EntityId,
        field: FieldId,
        met: &Met,
        pool: &str,
        per_level: Real,
        route: ExposureRoute,
    ) {
        self.events.emit(WorldEvent::OccupantDosed {
            tick,
            occupant,
            loc: met.loc,
            field,
            level: met.level,
            pool: pool.to_owned(),
            amount: Real::from_i64(i64::from(met.level)) * per_level,
            route,
            cause: field_cause(met.by),
        });
    }

    /// Passes a packet of damage that has arrived in a cell on through
    /// the fields there that spread it. See
    /// [`WorldEdit::conduct`](crate::WorldEdit::conduct).
    pub(crate) fn conduct(
        &mut self,
        loc: Location,
        packet: DamagePacket,
    ) -> Result<u32, WorldError> {
        let loc = self.cosmos.normalize(loc)?;
        if !self.cosmos.is_loaded(loc) {
            return Err(WorldError::NotLoaded(loc));
        }
        let defs = std::sync::Arc::clone(self.cosmos.defs());
        let mut reached = 0u32;
        for (field, _) in self.cosmos.fields_at(loc)? {
            let Some(def) = defs.fields().get(field) else {
                continue;
            };
            for conducts in def
                .conducts
                .iter()
                .filter(|conducts| conducts.kind == packet.kind)
            {
                let cells = self.cosmos.joined_through(loc, field, conducts.reach);
                let passed = DamagePacket::new(
                    packet.kind,
                    packet.energy * conducts.share,
                    packet.intensity * conducts.share,
                    packet.cause,
                );
                for cell in cells {
                    self.strike_cell(Location::new(loc.grid, cell), passed);
                    reached = reached.saturating_add(1);
                }
            }
        }
        Ok(reached)
    }

    /// Delivers a packet to a cell: to its floor, block and fixture,
    /// and, as `OccupantExposed`, to each occupant in it.
    fn strike_cell(&mut self, loc: Location, packet: DamagePacket) {
        let mut edit = self.edit(packet.cause);
        for layer in [Layer::Floor, Layer::Block, Layer::Fixture] {
            let _ = edit.damage(loc, layer, packet);
        }
        self.expose_occupants(
            loc,
            packet.kind,
            packet.energy,
            packet.intensity,
            packet.cause,
        );
    }

    fn expose_occupants(
        &mut self,
        loc: Location,
        kind: DamageTypeId,
        energy: Real,
        intensity: Real,
        cause: crate::cause::Cause,
    ) {
        if !self
            .cosmos
            .is_occupied(loc, &OccupantFilter::ANY)
            .unwrap_or(false)
        {
            return;
        }
        let Ok(found) = self.cosmos.occupants_at(loc, &OccupantFilter::ANY) else {
            return;
        };
        let tick = self.timeline.now();
        for occupant in distinct_ids(&found) {
            self.events.emit(WorldEvent::OccupantExposed {
                tick,
                occupant,
                loc,
                kind,
                energy,
                intensity,
                cause,
            });
        }
    }
}

impl Cosmos {
    /// The cells joined to a cell through a field: those that hold it
    /// and can be reached from the cell by steps north, east, south,
    /// west, up and down through cells that hold it, at most `reach`
    /// steps away. In the order a breadth-first walk finds them,
    /// without the cell itself, and at most
    /// [`FIELD_CONDUCT_MAX_CELLS`] of them.
    fn joined_through(&self, from: Location, field: FieldId, reach: u32) -> Vec<CellPos> {
        let mut found: Vec<CellPos> = Vec::new();
        let Ok(view) = self.grid(from.grid) else {
            return found;
        };
        let mut seen: LookupMap<CellPos, ()> = LookupMap::new();
        let mut queue: VecDeque<(CellPos, u32)> = VecDeque::from([(from.cell, 0)]);
        seen.insert(from.cell, ());
        while let Some((cell, steps)) = queue.pop_front() {
            if steps >= reach {
                continue;
            }
            for step in FIELD_STEPS {
                let Ok(next) = view.normalize(cell.offset(step)) else {
                    continue;
                };
                if seen.contains_key(&next) {
                    continue;
                }
                let holds = view
                    .column(next.tract())
                    .is_some_and(|column| level_in(column, next, field) > 0);
                if !holds {
                    continue;
                }
                seen.insert(next, ());
                found.push(next);
                if found.len() >= FIELD_CONDUCT_MAX_CELLS {
                    return found;
                }
                queue.push_back((next, steps + 1));
            }
        }
        found
    }
}
