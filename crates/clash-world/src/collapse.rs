//! Collapse (02 §14.2, §14.3): what the world does about a stability
//! check that finds something no longer standing.
//!
//! An edit that removes or weakens something structural asks for a
//! check of its cell. The checks asked for within the settle delay are
//! made together, so that one blast is one evaluation
//! ([`Cosmos::assess_stability`]). Blocks that carry more than they can
//! bear are crushed. Elements that nothing holds up come away, each
//! vertical run of them as one [`FallingMass`], which falls on the
//! timeline, strikes what it lands on, goes through it if that gives
//! way, and becomes rubble where it comes to rest. Everything it breaks
//! asks for a check of its own: a collapse that goes on floor by floor
//! is not scripted, it happens when the numbers say so.
//!
//! All of it is driven by [`WorldCtx::handle_wake`], and all of it is
//! plain state: the checks waiting, the masses in flight and the
//! collapses they belong to are saved and hashed with the cosmos.

use std::collections::BTreeMap;

use clash_core::{Dur, EntityId, IVec3, Real, StateHash, StateHasher, Tick, define_id};
use clash_time::{ClockId, Next, ProcessId};
use serde::{Deserialize, Serialize};

use crate::area::{AmbientOverride, AreaId, AreaShape};
use crate::cause::{Cause, CauseKind};
use crate::cosmos::Cosmos;
use crate::damage::{DamageOutcome, DamagePacket, Destruction, Wreck};
use crate::defs::{DamageTypeId, EngineFlag, Layer, MaterialId};
use crate::dims::{CELL_M, CHUNK_Z_BITS, STOREY_M, TRACT_XY};
use crate::edit::{StabilityRequest, WorldCtx};
use crate::error::WorldError;
use crate::event::WorldEvent;
use crate::occupancy::{OccupantFilter, distinct_ids};
use crate::pos::{CellBox, CellPos, GridId, Location};
use crate::process::{SettleCheck, WorldProcess};
use crate::stability::{Assessment, SUPPORT_REACH_CELLS, SUPPORT_REACH_STOREYS};

define_id! {
    /// A collapse in progress. Never reused within one cosmos.
    pub struct CollapseId;
}

/// How long after an edit asks for a stability check the check is made,
/// in local time where the edit was. Checks asked for meanwhile are made
/// with it (02 §14.2).
pub const SETTLE_DELAY: Dur = Dur::from_millis(50);

/// The distance, in metres, over which a falling mass is stopped by
/// what it lands on: the default stopping depth of a body (02 §15). Its
/// impact is spread over one cell's cross-section and this depth.
pub const FALL_IMPACT_DEPTH_M: Real = Real::of(0.1);

/// The most cells looked at when a change of gravity over an area asks
/// for checks of what stands in it. An area larger than this is checked
/// in part; the rest is checked when something next disturbs it.
const AREA_CHECK_MAX_CELLS: u64 = 1 << 20;

/// Part of a falling mass: so much of one material.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub struct DebrisPart {
    /// What it is made of.
    pub material: MaterialId,
    /// How much of it there is, in cubic metres.
    pub volume: Real,
}

clash_core::impl_state_hash!(DebrisPart { material, volume });

/// A mass of debris on its way down (02 §14.3): the elements of one
/// vertical run that nothing held up, with whatever stood on them.
///
/// It is a process of the world. It wakes when it should land, strikes
/// what it finds there, and either goes through and sleeps again or
/// comes to rest as rubble.
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub struct FallingMass {
    /// The collapse it is part of.
    pub collapse: CollapseId,
    /// The cell its base was in when it last started to fall.
    pub at: Location,
    /// The storey it is on its way to.
    pub land_z: i32,
    /// Its mass in kilograms.
    pub mass: Real,
    /// The energy in joules it kept from what it has broken through.
    pub energy: Real,
    /// What it is made of, by material in id order.
    pub parts: Vec<DebrisPart>,
    /// Whether it has already struck the cell it is in, and gone through
    /// the floor there. Whoever is in that cell has been told.
    pub struck: bool,
    /// Who and what brought it down.
    pub cause: Cause,
}

clash_core::impl_state_hash!(FallingMass {
    collapse,
    at,
    land_z,
    mass,
    energy,
    parts,
    struck,
    cause
});

/// What one instigator's share of a collapse came to (02 §7.4).
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub struct CollapseTally {
    /// Who is responsible, if anyone.
    pub instigator: Option<EntityId>,
    /// How many elements fell, were crushed or were broken by what fell.
    pub elements: u32,
    /// The mass brought down, in kilograms.
    pub mass: Real,
    /// How many times an occupant was under something that landed.
    pub exposed: u32,
}

clash_core::impl_state_hash!(CollapseTally {
    instigator,
    elements,
    mass,
    exposed
});

/// A collapse in progress: from the check that first found something
/// giving way until nothing of it is in flight and no check is waiting
/// near it.
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub struct Collapse {
    /// Its id.
    pub id: CollapseId,
    /// The grid it is in.
    pub grid: GridId,
    /// The box of cells it has touched so far.
    pub bounds: CellBox,
    /// The cell whose change started it.
    pub origin: Location,
    /// Who and what started it.
    pub cause: Cause,
    /// When it started.
    pub started: Tick,
    /// How many of its masses are in flight.
    pub in_flight: u32,
    /// What it has come to so far, by instigator in order.
    pub tallies: Vec<CollapseTally>,
}

clash_core::impl_state_hash!(Collapse {
    id,
    grid,
    bounds,
    origin,
    cause,
    started,
    in_flight,
    tallies
});

/// The collapses in progress and the checks waiting to be made.
#[derive(Clone, PartialEq, Eq, Debug, Default, Serialize, Deserialize)]
pub struct CollapseState {
    /// The process waiting out the settle delay on each clock.
    settles: BTreeMap<ClockId, ProcessId>,
    collapses: BTreeMap<CollapseId, Collapse>,
    next: u32,
}

impl CollapseState {
    /// Whether no collapse is in progress and no check is waiting.
    pub fn is_idle(&self) -> bool {
        self.settles.is_empty() && self.collapses.is_empty()
    }
}

impl StateHash for CollapseState {
    fn state_hash(&self, hasher: &mut StateHasher) {
        self.settles.state_hash(hasher);
        self.collapses.state_hash(hasher);
        hasher.write_u32(self.next);
    }
}

/// A box grown by a cell on every side.
fn grown(cells: CellBox) -> CellBox {
    CellBox::around(cells.min, IVec3::new(1, 1, 1))
        .union_with(CellBox::around(cells.max, IVec3::new(1, 1, 1)))
}

/// The smallest box that holds two boxes.
trait Union {
    fn union_with(self, other: CellBox) -> CellBox;
}

impl Union for CellBox {
    fn union_with(self, other: CellBox) -> CellBox {
        CellBox {
            min: CellPos::new(
                self.min.x.min(other.min.x),
                self.min.y.min(other.min.y),
                self.min.z.min(other.min.z),
            ),
            max: CellPos::new(
                self.max.x.max(other.max.x),
                self.max.y.max(other.max.y),
                self.max.z.max(other.max.z),
            ),
        }
    }
}

/// A drop of so many storeys, in metres. Never negative.
fn height(storeys: i32) -> Real {
    Real::from_i64(i64::from(storeys.max(0))) * STOREY_M
}

/// How long something moving down at `speed` m/s takes to fall `drop`
/// metres under `gravity` m/s². `None` if it never would.
fn fall_time(speed: Real, gravity: Real, drop: Real) -> Option<Dur> {
    if drop <= Real::ZERO {
        return Some(Dur::ZERO);
    }
    let seconds = if gravity > Real::ZERO {
        ((speed * speed + Real::of(2.0) * gravity * drop).sqrt() - speed) / gravity
    } else if speed > Real::ZERO {
        drop / speed
    } else {
        return None;
    };
    Some(Dur::from_secs_real(seconds))
}

impl FallingMass {
    /// How fast it is going down, in m/s, from the energy it kept.
    fn speed(&self) -> Real {
        if self.mass > Real::ZERO {
            (Real::of(2.0) * self.energy / self.mass).sqrt()
        } else {
            Real::ZERO
        }
    }

    fn take(&mut self, wreck: Wreck) {
        self.mass += wreck.mass;
        add_part(&mut self.parts, wreck);
    }
}

/// Adds what was left of an element to the parts of a mass, keeping
/// them in order of material.
fn add_part(parts: &mut Vec<DebrisPart>, wreck: Wreck) {
    if wreck.volume <= Real::ZERO {
        return;
    }
    match parts.binary_search_by_key(&wreck.material, |part| part.material) {
        Ok(at) => parts[at].volume += wreck.volume,
        Err(at) => parts.insert(
            at,
            DebrisPart {
                material: wreck.material,
                volume: wreck.volume,
            },
        ),
    }
}

impl Cosmos {
    /// The collapses in progress, in id order.
    pub fn collapses(&self) -> impl Iterator<Item = &Collapse> + '_ {
        self.activity.collapse.collapses.values()
    }

    /// A collapse in progress, by id.
    pub fn collapse(&self, id: CollapseId) -> Option<&Collapse> {
        self.activity.collapse.collapses.get(&id)
    }

    /// The masses of debris in flight, in the order they came away.
    pub fn falling_masses(&self) -> impl Iterator<Item = &FallingMass> + '_ {
        self.activity
            .processes
            .iter()
            .filter_map(|(_, process)| match process {
                WorldProcess::Fall(mass) => Some(mass),
                WorldProcess::Settle(_) => None,
            })
    }
}

impl WorldCtx<'_> {
    /// Asks for a stability check of a cell (02 §14.2), as an edit that
    /// removes or weakens something structural does. For whoever builds:
    /// putting an element in place asks for none, and what is built
    /// must stand (02 §14.5).
    ///
    /// The check is made after [`SETTLE_DELAY`], with every other check
    /// asked for meanwhile. Refused for a cell outside its grid or not
    /// loaded.
    pub fn check_stability(&mut self, loc: Location, cause: Cause) -> Result<(), WorldError> {
        let loc = self.cosmos.normalize(loc)?;
        if !self.cosmos.is_loaded(loc) {
            return Err(WorldError::NotLoaded(loc));
        }
        let tick = self.timeline.now();
        if self.cosmos.stability.request(loc, tick, cause) {
            self.stability_asked(loc);
        }
        Ok(())
    }

    /// Sees to it that a check newly asked for at a cell will be made:
    /// a wake-up waits out the settle delay on the clock of that cell.
    pub(crate) fn stability_asked(&mut self, loc: Location) {
        if let Ok(ambient) = self.cosmos.ambient(loc) {
            self.ensure_settle(ambient.clock);
        }
    }

    fn ensure_settle(&mut self, clock: ClockId) {
        let activity = &mut self.cosmos.activity;
        if let Some(waiting) = activity.collapse.settles.get(&clock)
            && activity.processes.contains(*waiting)
        {
            return;
        }
        let check = WorldProcess::Settle(SettleCheck { clock });
        let table = &mut activity.processes;
        let spawned = table
            .spawn(self.timeline, clock, SETTLE_DELAY, check.clone())
            .or_else(|_| table.spawn(self.timeline, ClockId::UNIVERSAL, SETTLE_DELAY, check));
        if let Ok(id) = spawned {
            activity.collapse.settles.insert(clock, id);
        }
    }

    /// Sets what an area changes about the ambient conditions inside
    /// it, as [`Cosmos::set_area_ambient`] does, and if that changes
    /// gravity asks for a stability check of what stands in the area:
    /// a field of heavy gravity can bring a building down (02 §14.5).
    pub fn set_area_ambient(
        &mut self,
        id: AreaId,
        ambient: Option<AmbientOverride>,
        cause: Cause,
    ) -> Result<(), WorldError> {
        let area = self.cosmos.area(id).ok_or(WorldError::UnknownArea(id))?;
        let before = area.ambient.and_then(|ambient| ambient.gravity);
        let (grid, shape) = (area.grid, area.shape.clone());
        self.cosmos.set_area_ambient(id, ambient)?;
        if before != ambient.and_then(|ambient| ambient.gravity) {
            self.check_stability_in(grid, &shape, cause);
        }
        Ok(())
    }

    /// Does what a change to an area calls for in the world's own
    /// activity. Where the area sets gravity (`weighs`), what stands in
    /// the shape is checked. Where it has a clock (`timed`), whatever is
    /// falling goes onto the clock of where it is.
    pub(crate) fn area_changed(
        &mut self,
        grid: GridId,
        shape: &AreaShape,
        weighs: bool,
        timed: bool,
        cause: Cause,
    ) {
        if timed {
            self.sync_process_clocks();
        }
        if weighs {
            self.check_stability_in(grid, shape, cause);
        }
    }

    /// Asks for a check of every floor and every block that is not an
    /// anchor in the loaded cells of a shape.
    fn check_stability_in(&mut self, grid: GridId, shape: &AreaShape, cause: Cause) {
        let Some(bounds) = shape.bounds() else {
            return;
        };
        let Ok(view) = self.cosmos.grid(grid) else {
            return;
        };
        let topology = *view.topology();
        let defs = self.cosmos.defs();
        let mut found: Vec<Location> = Vec::new();
        let mut scanned: u64 = 0;
        'scan: for tract in bounds.tracts() {
            let origin = tract.origin(0);
            let Ok(base) = topology.normalize(origin) else {
                continue;
            };
            let Some(column) = view.column(base.tract()) else {
                continue;
            };
            let Some((low, high)) = column.stored_range() else {
                continue;
            };
            let z_low = bounds.min.z.max(low << CHUNK_Z_BITS);
            let z_high = bounds.max.z.min(((high + 1) << CHUNK_Z_BITS) - 1);
            let x_low = bounds.min.x.max(origin.x);
            let x_high = bounds.max.x.min(origin.x.saturating_add(TRACT_XY - 1));
            let y_low = bounds.min.y.max(origin.y);
            let y_high = bounds.max.y.min(origin.y.saturating_add(TRACT_XY - 1));
            for z in z_low..=z_high {
                for y in y_low..=y_high {
                    for x in x_low..=x_high {
                        scanned += 1;
                        if scanned > AREA_CHECK_MAX_CELLS {
                            break 'scan;
                        }
                        let Ok(cell) = topology.normalize(CellPos::new(x, y, z)) else {
                            continue;
                        };
                        if !shape.contains(&topology, cell) {
                            continue;
                        }
                        let held = column.cell(cell);
                        let anchored = defs.block_info(held.block).flags.has(EngineFlag::Anchor);
                        if !held.floor.is_none() || (!held.block.is_none() && !anchored) {
                            found.push(Location::new(grid, cell));
                        }
                    }
                }
            }
        }
        let tick = self.timeline.now();
        for loc in found {
            if self.cosmos.stability.request(loc, tick, cause) {
                self.stability_asked(loc);
            }
        }
    }

    /// Makes the stability checks that have waited out the settle delay
    /// on a clock: those asked for where that clock applies. Checks
    /// asked for where another clock applies wait for their own.
    pub(crate) fn run_settle(&mut self, clock: ClockId) {
        self.cosmos.activity.collapse.settles.remove(&clock);
        let mut due: Vec<StabilityRequest> = Vec::new();
        for request in self.cosmos.take_stability_requests() {
            match self
                .cosmos
                .ambient(request.loc)
                .map(|ambient| ambient.clock)
            {
                Ok(other) if other != clock => {
                    self.cosmos
                        .stability
                        .request(request.loc, request.tick, request.cause);
                    self.ensure_settle(other);
                }
                Ok(_) => due.push(request),
                // The place has gone, and the question with it.
                Err(_) => {}
            }
        }

        // One evaluation for the changes that lie together.
        let near = |a: CellPos, b: CellPos| {
            let apart = |a: i32, b: i32| (i64::from(a) - i64::from(b)).abs();
            apart(a.x, b.x) <= i64::from(SUPPORT_REACH_CELLS)
                && apart(a.y, b.y) <= i64::from(SUPPORT_REACH_CELLS)
                && apart(a.z, b.z) <= i64::from(SUPPORT_REACH_STOREYS)
        };
        while let Some(first) = due.first().copied() {
            let (together, rest): (Vec<StabilityRequest>, Vec<StabilityRequest>) =
                due.into_iter().partition(|request| {
                    request.loc.grid == first.loc.grid && near(first.loc.cell, request.loc.cell)
                });
            due = rest;
            self.check_together(&together);
        }
        self.settle_collapses();
    }

    /// Evaluates the structure round a group of changes, and brings
    /// down what no longer stands.
    fn check_together(&mut self, requests: &[StabilityRequest]) {
        let Some(first) = requests.first() else {
            return;
        };
        let cells: Vec<CellPos> = requests.iter().map(|request| request.loc.cell).collect();
        let Ok(assessment) = self.cosmos.assess_stability(first.loc.grid, &cells) else {
            return;
        };
        if assessment.is_stable() {
            return;
        }
        // The first change made is what started it.
        let origin = requests
            .iter()
            .min_by_key(|request| (request.tick, request.loc))
            .unwrap_or(first);
        self.bring_down(&assessment, origin);
    }

    /// Acts on a check that found something giving way: crushes what is
    /// overloaded and sets falling what nothing holds up.
    fn bring_down(&mut self, assessment: &Assessment, origin: &StabilityRequest) {
        let grid = assessment.grid();
        // A process passes the instigator along (02 §7.3).
        let cause = Cause {
            actor: None,
            instigator: origin.cause.instigator,
            kind: CauseKind::Process,
        };
        let touched = assessment
            .crushed()
            .iter()
            .copied()
            .chain(assessment.falling().iter().map(|(cell, _)| *cell));
        let Some(bounds) = touched
            .map(CellBox::single)
            .reduce(|all, cell| all.union_with(cell))
        else {
            return;
        };
        let id = self.join_collapse(grid, bounds, origin.loc, cause);

        for cell in assessment.crushed() {
            let loc = Location::new(grid, *cell);
            let crushed = self.edit(cause).wreck(
                loc,
                Layer::Block,
                Destruction::Crushed,
                DamageOutcome::UNAFFECTED,
                None,
                true,
            );
            if let Ok(Some(wreck)) = crushed {
                self.tally(id, cause, 1, wreck.mass, 0);
            }
        }

        // Each vertical run of falling elements is one mass.
        let falling = assessment.falling();
        let mut at = 0;
        while at < falling.len() {
            let start = at;
            let (base, _) = falling[at];
            let mut top = base.z;
            while at < falling.len() {
                let (cell, _) = falling[at];
                if cell.x != base.x || cell.y != base.y || cell.z > top.saturating_add(1) {
                    break;
                }
                top = cell.z;
                at += 1;
            }
            self.detach(id, grid, &falling[start..at], cause);
        }
    }

    /// The collapse that a new failure in a box belongs to: one already
    /// in progress there, or a new one.
    fn join_collapse(
        &mut self,
        grid: GridId,
        bounds: CellBox,
        origin: Location,
        cause: Cause,
    ) -> CollapseId {
        let tick = self.timeline.now();
        let state = &mut self.cosmos.activity.collapse;
        let near = grown(bounds);
        let joined = state
            .collapses
            .values_mut()
            .find(|collapse| collapse.grid == grid && collapse.bounds.intersection(near).is_some());
        if let Some(collapse) = joined {
            collapse.bounds = collapse.bounds.union_with(bounds);
            return collapse.id;
        }
        let id = CollapseId::new(state.next);
        state.next = state.next.saturating_add(1);
        state.collapses.insert(
            id,
            Collapse {
                id,
                grid,
                bounds,
                origin,
                cause,
                started: tick,
                in_flight: 0,
                tallies: Vec::new(),
            },
        );
        // W9: hold a ticket over the collapse's bounds until it settles,
        // so that the tracts it touches stay loaded (02 §14.3, W5).
        self.events.emit(WorldEvent::CollapseStarted {
            tick,
            collapse: id,
            loc: origin,
            cause,
        });
        id
    }

    /// Adds to what a collapse has come to, under the instigator of a
    /// cause.
    fn tally(&mut self, id: CollapseId, cause: Cause, elements: u32, mass: Real, exposed: u32) {
        let Some(collapse) = self.cosmos.activity.collapse.collapses.get_mut(&id) else {
            return;
        };
        let who = cause.instigator;
        let at = match collapse
            .tallies
            .binary_search_by_key(&who, |tally| tally.instigator)
        {
            Ok(at) => at,
            Err(at) => {
                collapse.tallies.insert(
                    at,
                    CollapseTally {
                        instigator: who,
                        elements: 0,
                        mass: Real::ZERO,
                        exposed: 0,
                    },
                );
                at
            }
        };
        let tally = &mut collapse.tallies[at];
        tally.elements = tally.elements.saturating_add(elements);
        tally.mass += mass;
        tally.exposed = tally.exposed.saturating_add(exposed);
    }

    /// Takes one vertical run of unsupported elements out of the world,
    /// with the fixtures in their cells, and sets them falling as one
    /// mass.
    fn detach(&mut self, id: CollapseId, grid: GridId, run: &[(CellPos, Layer)], cause: Cause) {
        let Some((base, _)) = run.first() else {
            return;
        };
        let mut mass = FallingMass {
            collapse: id,
            at: Location::new(grid, *base),
            land_z: base.z,
            mass: Real::ZERO,
            energy: Real::ZERO,
            parts: Vec::new(),
            struck: false,
            cause,
        };
        let mut elements = 0;
        let mut emptied = None;
        for (cell, layer) in run {
            let loc = Location::new(grid, *cell);
            let layers = if emptied == Some(*cell) {
                [Some(*layer), None]
            } else {
                // What stood in the cell goes with it.
                emptied = Some(*cell);
                [Some(*layer), Some(Layer::Fixture)]
            };
            for layer in layers.into_iter().flatten() {
                let fell = self.edit(cause).wreck(
                    loc,
                    layer,
                    Destruction::Fell,
                    DamageOutcome::UNAFFECTED,
                    None,
                    false,
                );
                if let Ok(Some(wreck)) = fell {
                    mass.take(wreck);
                    elements += 1;
                }
            }
        }
        if elements == 0 {
            return;
        }
        self.tally(id, cause, elements, mass.mass, 0);

        let Ok(at) = self.cosmos.normalize(mass.at) else {
            return;
        };
        mass.at = at;
        mass.land_z = self
            .cosmos
            .resting_cell(at)
            .map_or(at.cell.z, |rest| rest.cell.z);
        let (gravity, clock) = match self.cosmos.ambient(at) {
            Ok(ambient) => (ambient.gravity, ambient.clock),
            Err(_) => (Real::ZERO, ClockId::UNIVERSAL),
        };
        let wait = fall_time(Real::ZERO, gravity, height(at.cell.z - mass.land_z));
        let spawned = wait.and_then(|wait| {
            self.cosmos
                .activity
                .processes
                .spawn(self.timeline, clock, wait, WorldProcess::Fall(mass.clone()))
                .ok()
        });
        match spawned {
            Some(_) => {
                if let Some(collapse) = self.cosmos.activity.collapse.collapses.get_mut(&id) {
                    collapse.in_flight = collapse.in_flight.saturating_add(1);
                }
            }
            // It cannot be put on the timeline: it comes down at once.
            None => self.leave_debris(&mass),
        }
    }

    /// A falling mass has reached where it was going. It strikes what
    /// it lands on; if that gives way it goes on down with the energy
    /// left over and the mass of what it broke, and if not it comes to
    /// rest as rubble (02 §14.3).
    pub(crate) fn run_fall(&mut self, mut mass: FallingMass) -> (FallingMass, Next) {
        let grid = mass.at.grid;
        let Ok(rest) = self.cosmos.resting_cell(mass.at) else {
            // Where it was is no longer there.
            self.end_fall(mass.collapse);
            return (mass, Next::Done);
        };
        let gravity = self
            .cosmos
            .ambient(mass.at)
            .map_or(Real::ZERO, |ambient| ambient.gravity);

        if rest.cell.z < mass.land_z {
            // What it was to land on has gone meanwhile. It keeps going.
            mass.energy += mass.mass * gravity * height(mass.at.cell.z - mass.land_z);
            mass.at.cell.z = mass.land_z;
            mass.land_z = rest.cell.z;
            let further = height(mass.at.cell.z - mass.land_z);
            if let Some(wait) = fall_time(mass.speed(), gravity, further) {
                return (mass, Next::Sleep(wait));
            }
        }

        let energy = mass.energy + mass.mass * gravity * height(mass.at.cell.z - rest.cell.z);
        // Having gone through a cell's floor, it strikes what is under
        // the floor: the same cell, whose occupants it has already met.
        let again = mass.struck && rest.cell == mass.at.cell;
        mass.at = rest;
        mass.land_z = rest.cell.z;
        mass.energy = energy;

        // The floor it lands on, or the solid top of the block below.
        let target = self.cosmos.grid(grid).ok().and_then(|view| {
            let held = view.cell(rest.cell).ok()?;
            if !held.floor.is_none() {
                return Some((rest, Layer::Floor));
            }
            let below = view.normalize(rest.cell.below()).ok()?;
            let under = view.cell(below).ok()?;
            (!under.block.is_none()).then_some((Location::new(grid, below), Layer::Block))
        });
        let kinetic = self.cosmos.defs().kinetic_damage_type();
        if let (Some(kind), Some((target, layer))) = (kinetic, target)
            && energy > Real::ZERO
        {
            let intensity = energy / (CELL_M * CELL_M * FALL_IMPACT_DEPTH_M);
            if !again {
                self.expose(rest, kind, energy, intensity, &mass);
            }
            let packet = DamagePacket::new(kind, energy, intensity, mass.cause);
            let down = [Real::ZERO, Real::ZERO, -Real::ONE];
            let struck = self
                .edit(mass.cause)
                .strike(target, layer, packet, Some(down), false);
            if let Ok((outcome, Some(wreck))) = struck
                && outcome.absorbed > Real::ZERO
            {
                // It went through, and takes what it broke with it.
                mass.take(wreck);
                self.tally(mass.collapse, mass.cause, 1, wreck.mass, 0);
                mass.energy = outcome.passed;
                mass.struck = target.cell == rest.cell;
                mass.at = target;
                mass.land_z = self
                    .cosmos
                    .resting_cell(target)
                    .map_or(target.cell.z, |rest| rest.cell.z);
                let gravity = self
                    .cosmos
                    .ambient(target)
                    .map_or(Real::ZERO, |ambient| ambient.gravity);
                let further = height(target.cell.z - mass.land_z);
                if let Some(wait) = fall_time(mass.speed(), gravity, further) {
                    return (mass, Next::Sleep(wait));
                }
            }
        }

        self.leave_debris(&mass);
        self.end_fall(mass.collapse);
        (mass, Next::Done)
    }

    /// Tells every occupant of a cell that something has landed on it
    /// (02 §14.3). What that does to them is for the simulation.
    fn expose(
        &mut self,
        loc: Location,
        kind: DamageTypeId,
        energy: Real,
        intensity: Real,
        mass: &FallingMass,
    ) {
        let Ok(found) = self.cosmos.occupants_at(loc, &OccupantFilter::ANY) else {
            return;
        };
        let tick = self.timeline.now();
        let mut exposed = 0;
        for occupant in distinct_ids(&found) {
            self.events.emit(WorldEvent::OccupantExposed {
                tick,
                occupant,
                loc,
                kind,
                energy,
                intensity,
                cause: mass.cause,
            });
            exposed += 1;
        }
        if exposed > 0 {
            self.tally(mass.collapse, mass.cause, 0, Real::ZERO, exposed);
        }
    }

    /// Leaves a mass that has come to rest as rubble, each material as
    /// its own.
    fn leave_debris(&mut self, mass: &FallingMass) {
        let mut edit = self.edit(mass.cause);
        for part in &mass.parts {
            // Rubble that finds no loaded cell to lie in is lost.
            let _ = edit.leave_rubble(mass.at, part.material, part.volume);
        }
    }

    /// One mass of a collapse is no longer in flight.
    fn end_fall(&mut self, id: CollapseId) {
        if let Some(collapse) = self.cosmos.activity.collapse.collapses.get_mut(&id) {
            collapse.in_flight = collapse.in_flight.saturating_sub(1);
        }
        self.settle_collapses();
    }

    /// Ends every collapse that has nothing in flight and no check
    /// waiting near it, with `CollapseSettled`.
    fn settle_collapses(&mut self) {
        let tick = self.timeline.now();
        let idle: Vec<CollapseId> = self
            .cosmos
            .collapses()
            .filter(|collapse| collapse.in_flight == 0)
            .map(|collapse| collapse.id)
            .collect();
        for id in idle {
            let Some(collapse) = self.cosmos.collapse(id) else {
                continue;
            };
            let near = grown(collapse.bounds);
            let waiting = self.cosmos.stability.iter().any(|request| {
                request.loc.grid == collapse.grid && near.contains(request.loc.cell)
            });
            if waiting {
                continue;
            }
            let Some(collapse) = self.cosmos.activity.collapse.collapses.remove(&id) else {
                continue;
            };
            // W9: release the collapse's ticket.
            self.events.emit(WorldEvent::CollapseSettled {
                tick,
                collapse: id,
                loc: collapse.origin,
                bounds: collapse.bounds,
                tallies: collapse.tallies,
                cause: collapse.cause,
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_fall_takes_the_time_gravity_gives_it() {
        // One storey under 9.81 m/s²: √(2 × 3 / 9.81) seconds.
        let wait = fall_time(Real::ZERO, Real::of(9.81), STOREY_M);
        assert_eq!(wait, Some(Dur::from_micros(782_062)));
        // Already moving, it takes less.
        let moving = fall_time(Real::of(5.0), Real::of(9.81), STOREY_M);
        assert!(moving < wait);
        // No drop takes no time; no gravity and no speed never arrives.
        assert_eq!(
            fall_time(Real::ZERO, Real::of(9.81), Real::ZERO),
            Some(Dur::ZERO)
        );
        assert_eq!(fall_time(Real::ZERO, Real::ZERO, STOREY_M), None);
        assert_eq!(
            fall_time(Real::of(3.0), Real::ZERO, STOREY_M),
            Some(Dur::SECOND)
        );
    }

    #[test]
    fn boxes_grow_and_join() {
        let cell = CellBox::single(CellPos::new(4, 5, 6));
        let wide = grown(cell);
        assert_eq!(wide.min, CellPos::new(3, 4, 5));
        assert_eq!(wide.max, CellPos::new(5, 6, 7));
        let both = cell.union_with(CellBox::single(CellPos::new(0, 9, 6)));
        assert_eq!(both.min, CellPos::new(0, 5, 6));
        assert_eq!(both.max, CellPos::new(4, 9, 6));
    }

    #[test]
    fn parts_keep_to_material_order() {
        let part = |material: u16, volume: f64| Wreck {
            material: MaterialId::new(material),
            volume: Real::of(volume),
            mass: Real::ZERO,
        };
        let mut parts = Vec::new();
        add_part(&mut parts, part(5, 1.0));
        add_part(&mut parts, part(2, 0.5));
        add_part(&mut parts, part(5, 0.25));
        add_part(&mut parts, part(9, 0.0));
        let listed: Vec<(u16, f64)> = parts
            .iter()
            .map(|part| (part.material.raw(), part.volume.get()))
            .collect();
        assert_eq!(listed, [(2, 0.5), (5, 1.25)]);
    }
}
