//! Navigation for minds, the beta subset (09 §5.1 to §5.4).
//!
//! Movement already plans paths over what a mover believes
//! ([`believed_path`](crate::movement::believed_path)): the mover's own
//! ways of moving and their speeds are its profile, static geometry is
//! common knowledge, hidden elements count once noted, and doors cost
//! the time to open them. What a mind adds here:
//!
//! | Need | Here |
//! |------|------|
//! | Other creatures cost a detour, not a wall (09 §5.3) | [`MindCosts`]: a step into a cell where the mind last saw someone, recently, costs the mind's `crowd_cost` metres more |
//! | Known hazards cost (09 §5.3) | [`MindCosts`]: a step within a hazard's `radius` of something the mind remembers that its `hazards` name costs that hazard's `cost` more |
//! | A mover wider than a cell (09 §5.4) | [`MindCosts`]: a cell counts as one it can be in only if every cell of its footprint is |
//! | A route in use is rechecked when the world changes under it (09 §5.2) | [`recheck_routes`]: a path whose remaining steps a change made impossible is marked stale, and the `go_to` action plans again |
//! | Somewhere to stand near a place | [`nearest_standable`] |
//! | Somewhere away from a danger that it can reach | [`away_from`] |
//!
//! Every function reads what the mind believes: the geometry of the
//! world (common knowledge, 15 §7.3), its own tracks and memories, and
//! nothing about another creature that it has not perceived.
//!
//! Not built: the region and tract graphs of 09 §5.2, links over
//! portals and subgrids, the flier's straight line, and flow fields
//! (package I2). A path is searched cell by cell, within
//! `PATH_DEFAULT_MAX_NODES`; measured, a path across a block of the
//! test city costs well within the budget of 09 §19 (see the timing
//! test in `tests/ai.rs`).

use clash_core::{EntityId, Real};
use clash_world::{CellGeometry, CellPos, Location, MoveRules, PathLimits, Step, StepKind};

use super::defs::{Hazard, MindDef};
use super::state::MindState;
use crate::Sim;
use crate::event::{EventBody, Subject, Where};
use crate::movement::believed_rules;
use crate::perception::belief;

/// How long, in seconds of its own time, a mind takes someone it
/// perceived to be still standing where it saw them, for its paths.
pub const CROWD_MEMORY_S: f64 = 5.0;

/// How far round a cell [`nearest_standable`] looks, in cells.
pub const STANDABLE_RADIUS: i32 = 3;

/// The most cells whose change [`recheck_routes`] keeps track of before
/// it checks every route in use.
pub const MAX_CHANGED_CELLS: usize = 4096;

/// How many places [`away_from`] tries: one in each of the eight
/// directions of the compass that does not lead back toward the danger.
pub const FLEE_DIRECTIONS: usize = 8;

/// The most cells a search for a place to flee to may settle.
pub const FLEE_MAX_NODES: u32 = 4_000;

/// Movement rules with what a mind adds: detours round creatures it
/// believes are in the way, the cost of hazards it knows of, and room
/// for a footprint wider than a cell. See the [module](self).
#[derive(Debug)]
pub struct MindCosts<'a, R> {
    inner: R,
    sim: &'a Sim,
    grid: Option<clash_world::GridId>,
    /// Cells where the mind recently saw someone, sorted.
    crowd: Vec<CellPos>,
    crowd_cost: Real,
    /// Places it remembers a hazard at: where, how far it reaches in
    /// cells, and what a step within it costs more.
    hazards: Vec<(CellPos, i32, Real)>,
    /// The footprint, in cells east and south.
    footprint: (i32, i32),
}

/// Wraps a mover's believed rules in what its mind adds (see
/// [`MindCosts`]). A mover with no mind state gains only its footprint.
pub fn with_mind_costs<'a, R: MoveRules>(
    sim: &'a Sim,
    entity: EntityId,
    inner: R,
) -> MindCosts<'a, R> {
    let grid = sim.entities.location(entity).map(|loc| loc.grid);
    let footprint = sim
        .entities
        .get(entity)
        .map(|record| record.presence().footprint)
        .map_or((1, 1), |footprint| {
            (i32::from(footprint.x.max(1)), i32::from(footprint.y.max(1)))
        });
    let mut costs = MindCosts {
        inner,
        sim,
        grid,
        crowd: Vec::new(),
        crowd_cost: Real::ZERO,
        hazards: Vec::new(),
        footprint,
    };
    let (Some(state), Some(def), Some(grid)) = (
        sim.components.mind_state.get(entity),
        super::think::mind_def_of(sim, entity).and_then(|id| super::think::mind_def(sim, id)),
        grid,
    ) else {
        return costs;
    };
    let now = super::think::local_now(sim, entity);
    costs.crowd_cost = def.crowd_cost;
    if def.crowd_cost > Real::ZERO
        && let Some(beliefs) = sim.components.beliefs.get(entity)
    {
        for track in beliefs.tracks() {
            let recent = (now - track.at()).as_secs_real() <= Real::of(CROWD_MEMORY_S);
            if let Some(loc) = track.loc()
                && loc.grid == grid
                && recent
                && track.persona().entity() != entity
            {
                costs.crowd.push(loc.cell);
            }
        }
        costs.crowd.sort();
        costs.crowd.dedup();
    }
    costs.hazards = known_hazards(sim, entity, def, state, grid);
    costs
}

/// The places a mind remembers a hazard its definition names, with how
/// far each reaches and what it costs.
fn known_hazards(
    sim: &Sim,
    entity: EntityId,
    def: &MindDef,
    _state: &MindState,
    grid: clash_world::GridId,
) -> Vec<(CellPos, i32, Real)> {
    let Some(beliefs) = sim.components.beliefs.get(entity) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for hazard in &def.hazards {
        for memory in beliefs.memories() {
            if !hazard.events.contains(&memory.event) {
                continue;
            }
            let Some(loc) = memory.loc.filter(|loc| loc.grid == grid) else {
                continue;
            };
            if super::think::memory_salience(sim, entity, memory) < hazard_floor(hazard) {
                continue;
            }
            out.push((loc.cell, radius_cells(hazard.radius), hazard.cost));
        }
    }
    out
}

/// The salience below which a remembered hazard is no longer avoided.
fn hazard_floor(hazard: &Hazard) -> Real {
    hazard.below
}

/// A distance in metres as a whole number of cells, at least 0.
fn radius_cells(metres: Real) -> i32 {
    (metres / clash_world::dims::CELL_M)
        .ceil()
        .clamp(Real::ZERO, Real::of(64.0))
        .to_i32()
}

impl<R: MoveRules> MindCosts<'_, R> {
    /// Whether every cell of the footprint whose corner is `cell` can be
    /// occupied, the corner itself already judged.
    fn room(&self, cell: &CellGeometry) -> bool {
        if self.footprint == (1, 1) {
            return true;
        }
        let Some(grid) = self.grid else {
            return true;
        };
        for dy in 0..self.footprint.1 {
            for dx in 0..self.footprint.0 {
                if (dx, dy) == (0, 0) {
                    continue;
                }
                let at = Location::new(grid, cell.cell.offset(clash_core::IVec3::new(dx, dy, 0)));
                let fits = self
                    .sim
                    .cosmos
                    .neighbours(at)
                    .is_ok_and(|around| self.inner.can_occupy(&around.here));
                if !fits {
                    return false;
                }
            }
        }
        true
    }

    fn extra(&self, cell: CellPos) -> Real {
        let mut extra = Real::ZERO;
        if self.crowd.binary_search(&cell).is_ok() {
            extra += self.crowd_cost;
        }
        for (at, radius, cost) in &self.hazards {
            let near = (at.x - cell.x).abs() <= *radius
                && (at.y - cell.y).abs() <= *radius
                && at.z == cell.z;
            if near {
                extra += *cost;
            }
        }
        extra
    }
}

impl<R: MoveRules> MoveRules for MindCosts<'_, R> {
    fn can_occupy(&self, cell: &CellGeometry) -> bool {
        self.inner.can_occupy(cell) && self.room(cell)
    }

    fn step_cost(&self, step: &Step) -> Option<Real> {
        let cost = self.inner.step_cost(step)?;
        Some(cost + self.extra(step.to.cell))
    }
}

/// The rules a mind judges its own movement by: its ways of moving,
/// through what it expects to be able to open, as it knows the world,
/// with what [`MindCosts`] adds.
pub fn mind_rules(
    sim: &Sim,
    entity: EntityId,
    grid: clash_world::GridId,
) -> MindCosts<'_, impl MoveRules + '_> {
    let mut rules = believed_rules(sim, entity);
    if super::doors::opens_doors(sim, entity) {
        rules = rules.through_openable();
    }
    let rules = rules.avoiding(&super::doors::shut_cells(sim, entity, grid));
    let rules = crate::perception::believed_move_rules_with(sim, entity, rules).on_grid(grid);
    with_mind_costs(sim, entity, rules)
}

/// The nearest cell to `to`, on its storey and within
/// [`STANDABLE_RADIUS`] cells, that the mover believes it can be in:
/// `to` itself if it can. Ties go to the first in order of row, then
/// column.
pub fn nearest_standable(sim: &Sim, entity: EntityId, to: Location) -> Option<Location> {
    let to = sim.cosmos.normalize(to).ok()?;
    let rules = mind_rules(sim, entity, to.grid);
    let fits = |loc: Location| {
        sim.cosmos
            .normalize(loc)
            .ok()
            .and_then(|loc| sim.cosmos.neighbours(loc).ok().map(|around| (loc, around)))
            .filter(|(_, around)| rules.can_occupy(&around.here))
            .map(|(loc, _)| loc)
    };
    if let Some(found) = fits(to) {
        return Some(found);
    }
    for ring in 1..=STANDABLE_RADIUS {
        for dy in -ring..=ring {
            for dx in -ring..=ring {
                if dx.abs() != ring && dy.abs() != ring {
                    continue;
                }
                let at = Location::new(to.grid, to.cell.offset(clash_core::IVec3::new(dx, dy, 0)));
                if let Some(found) = fits(at) {
                    return Some(found);
                }
            }
        }
    }
    None
}

/// Somewhere a mind believes it can reach that is `distance` metres
/// further from `from` than where it stands: of the places that far off
/// in each direction of the compass that does not lead back toward
/// `from`, the one that ends furthest from it for the least walking.
/// `None` if it believes it can reach none of them.
pub fn away_from(sim: &Sim, entity: EntityId, from: Location, distance: Real) -> Option<Location> {
    let here = sim.entities.location(entity)?;
    let away = sim.cosmos.delta(from, here).unwrap_or_default();
    let reach = radius_cells(distance).max(1);
    let rules = mind_rules(sim, entity, here.grid);
    let limits = PathLimits {
        max_nodes: FLEE_MAX_NODES,
        max_cost_m: distance * Real::of(4.0),
    };
    let mut best: Option<(Real, Location)> = None;
    for dir in clash_core::Dir8::ALL.into_iter().take(FLEE_DIRECTIONS) {
        let step = dir.delta();
        // Not back toward the danger.
        if away != clash_core::IVec3::ZERO && step.x * away.x + step.y * away.y < 0 {
            continue;
        }
        let aim = Location::new(
            here.grid,
            here.cell
                .offset(clash_core::IVec3::new(step.x * reach, step.y * reach, 0)),
        );
        let Some(spot) = nearest_standable(sim, entity, aim) else {
            continue;
        };
        let Ok(path) = sim.cosmos.find_path(here, spot, &rules, limits) else {
            continue;
        };
        let gained = sim.cosmos.delta(from, spot).map_or(Real::ZERO, metres);
        let score = gained * Real::of(2.0) - path.cost_m();
        if best.is_none_or(|(have, _)| score > have) {
            best = Some((score, spot));
        }
    }
    best.map(|(_, spot)| spot)
}

/// The length of a step between cells, in metres, a storey counting
/// [`STOREY_M`](clash_world::dims::STOREY_M).
pub fn metres(by: clash_core::IVec3) -> Real {
    let x = Real::from_i64(i64::from(by.x)) * clash_world::dims::CELL_M;
    let y = Real::from_i64(i64::from(by.y)) * clash_world::dims::CELL_M;
    let z = Real::from_i64(i64::from(by.z)) * clash_world::dims::STOREY_M;
    (x * x + y * y + z * z).sqrt()
}

/// How far, in metres, an entity believes it stands from a place: its
/// own place is its own to know. `None` if it is not in the world or
/// the place is in another grid.
pub fn distance_to(sim: &Sim, entity: EntityId, to: Location) -> Option<Real> {
    let here = sim.entities.location(entity)?;
    sim.cosmos.delta(here, to).map(metres)
}

/// Notes a cell whose geometry changed, for [`recheck_routes`].
pub(super) fn changed(sim: &mut Sim, loc: Location) {
    if sim.ai.all_changed {
        return;
    }
    if sim.ai.changed.len() >= MAX_CHANGED_CELLS {
        sim.ai.changed.clear();
        sim.ai.all_changed = true;
        return;
    }
    sim.ai.changed.insert(loc);
}

/// Rechecks every route in use that passes next to a cell whose
/// geometry changed (09 §5.2): a path whose remaining steps the mover
/// now believes it cannot make is marked stale, so that the `go_to`
/// action plans again before its next step, and `route_blocked` is
/// raised with the first cell that cannot be entered. Static geometry
/// is common knowledge, so a mover knows of a wall that fell across its
/// way as soon as it fell.
pub fn recheck_routes(sim: &mut Sim) {
    if sim.ai.changed.is_empty() && !sim.ai.all_changed {
        return;
    }
    let changed: Vec<Location> = core::mem::take(&mut sim.ai.changed).into_iter().collect();
    let all = core::mem::take(&mut sim.ai.all_changed);
    let movers: Vec<EntityId> = sim.components.mover.owners().to_vec();
    let mut blocked: Vec<(EntityId, Location)> = Vec::new();
    for mover in movers {
        let Some(journey) = sim
            .components
            .mover
            .get(mover)
            .and_then(crate::movement::Mover::journey)
        else {
            continue;
        };
        if journey.is_stale() {
            continue;
        }
        let Some(here) = sim.entities.location(mover) else {
            continue;
        };
        let grid = here.grid;
        let remaining = journey.remaining();
        let touched = all
            || remaining.iter().any(|cell| {
                changed.iter().any(|loc| {
                    loc.grid == grid
                        && (loc.cell.x - cell.x).abs() <= 1
                        && (loc.cell.y - cell.y).abs() <= 1
                        && (loc.cell.z - cell.z).abs() <= 1
                })
            });
        if !touched {
            continue;
        }
        if let Some(cell) = first_blocked(sim, mover, here, remaining) {
            blocked.push((mover, Location::new(grid, cell)));
        }
    }
    for (mover, at) in blocked {
        if crate::movement::mark_journey_stale(sim, mover) {
            let payload = crate::bus::EventPayload::new(super::ROUTE_BLOCKED)
                .subject("self", Subject::Entity(mover))
                .subject("here", Subject::Loc(at));
            sim.emit(
                Where::Entity(mover),
                clash_world::Cause::new(clash_world::CauseKind::Process),
                EventBody::Custom(payload),
            );
        }
    }
}

/// The first cell of a path that its mover now believes it cannot step
/// into from the one before, if any.
fn first_blocked(sim: &Sim, mover: EntityId, here: Location, cells: &[CellPos]) -> Option<CellPos> {
    let rules = mind_rules(sim, mover, here.grid);
    let mut from = here;
    for cell in cells {
        let to = Location::new(here.grid, *cell);
        let kind = sim
            .cosmos
            .delta(from, to)
            .and_then(|delta| StepKind::ALL.into_iter().find(|kind| kind.delta() == delta));
        let step = kind.and_then(|kind| sim.cosmos.neighbours(from).ok()?.step(kind));
        let fine =
            step.is_some_and(|step| rules.can_occupy(&step.to) && rules.step_cost(&step).is_some());
        if !fine {
            return Some(*cell);
        }
        from = to;
    }
    None
}

/// Where a mind believes an entity is: its own place if it is the mind
/// itself, else its track's last known place.
pub fn believed_place(sim: &Sim, mind: EntityId, of: EntityId) -> Option<Location> {
    if of == mind {
        return sim.entities.location(mind);
    }
    belief::believed_loc(sim, mind, of)
}
