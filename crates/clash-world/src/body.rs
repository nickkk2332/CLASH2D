//! Bodies (02 §15): things in flight that the world moves.
//!
//! A [`Body`] is a point mass with a cross-section: someone who has just
//! been hit very hard, a thrown rock, a bullet, a fragment of a wall. It
//! is launched with [`WorldCtx::launch`] and from then on is a process
//! of the world, on the clock of where it is. Each time it wakes it has
//! flown for the time it slept: gravity, drag against the moving medium
//! and its own drive change its velocity, and the straight stretch it
//! covered is followed through the cells with the projectile ray of
//! 02 §22. What it meets it strikes by the rule of 02 §15.2: the energy
//! of its motion along the direction of entry, delivered over its
//! cross-section and stopping depth. What gives way lets it through with
//! the energy that passed. What does not sends it back, or stops it.
//!
//! | To | Use |
//! |----|-----|
//! | Throw something | [`WorldCtx::launch`] with a [`Body`]; [`Body::of_mass`] fills in what is not known |
//! | Push it, catch it, turn it | [`WorldCtx::impulse`], [`WorldCtx::arrest`], [`WorldCtx::steer`] |
//! | See what is in flight | [`Cosmos::body`], [`Cosmos::bodies`], [`Cosmos::body_of`] |
//!
//! What the world reports: `BodyLaunched`, `BodyImpact` for every
//! element struck, `BodyHitOccupant` for every occupant struck,
//! `BodyLanded` when it comes to rest and `BodyLeftLoaded` when its
//! flight takes it out of loaded space. An occupant that is a body is
//! carried along: the world moves it from cell to cell and reports each
//! move as `OccupantDisplaced`.
//!
//! Nothing here depends on how the timeline is advanced: a body wakes
//! at the ticks its own flight gives, and what it does at each is a
//! function of its state and the world.

use std::collections::BTreeMap;

use clash_core::{Dur, EntityId, HashDomain, Real, StateHash, StateHasher, Tick, pos_hash};
use clash_core::{IVec3, define_id};
use clash_time::{Next, ProcessId};
use serde::{Deserialize, Serialize};

use crate::area::Drift;
use crate::cause::{Cause, CauseKind};
use crate::cosmos::{Ambient, Cosmos};
use crate::damage::{DamagePacket, DamageResult, DebrisSpec};
use crate::defs::{DamageTypeId, EdgeRule, ElementState, Layer, MaterialId};
use crate::dims::{CELL_M, STOREY_M, TRACT_XY};
use crate::edit::WorldCtx;
use crate::error::WorldError;
use crate::event::WorldEvent;
use crate::grid::{HEIGHT_FULL, HEIGHT_NONE};
use crate::occupancy::{OccupantFilter, OccupantFlags, PlaneMask};
use crate::pos::{CellPos, GridId, Location, PointM};
use crate::process::WorldProcess;
use crate::query::{
    NoObstacles, RayEnd, RayFilter, RayHit, RayHits, RayObstacles, RaySpan, RayTarget,
};
use crate::ticket::{Activity, Ticket, TicketAnchor, TicketId, TicketReason};
use crate::topology::Edge;
use crate::tract::PendingEdit;

define_id! {
    /// A body in flight. Never reused within one cosmos.
    pub struct BodyId;
}

/// The longest a body flies between two looks at the world, in local
/// time, so that gravity and drag integrate accurately (02 §15.1).
pub const BODY_MAX_STEP: Dur = Dur::from_millis(20);

/// The shortest a body flies between two looks at the world. A body so
/// fast that it crosses a cell in less crosses several cells a step.
pub const BODY_MIN_STEP: Dur = Dur::from_micros(10);

/// The speed, in m/s, below which a body that has been stopped stays
/// stopped (02 §15.2).
pub const REST_SPEED: Real = Real::of(0.5);

/// The distance, in metres, over which a body is stopped by what it
/// strikes, unless it says otherwise (02 §15).
pub const BODY_DEFAULT_DEPTH_M: Real = Real::of(0.1);

/// The drag coefficient of a body that states none.
pub const BODY_DEFAULT_DRAG: Real = Real::of(1.0);

/// The density, in kg/m³, the world assumes of a thing whose size it is
/// not told: its volume and cross-section follow from its mass.
pub const BODY_ASSUMED_DENSITY: Real = Real::of(1000.0);

/// The coefficient of sliding friction of a surface whose material
/// states none (02 §15.3).
pub const BODY_DEFAULT_FRICTION: Real = Real::of(0.5);

/// How many tracts beyond what is otherwise loaded a body's ticket will
/// follow it into: the default of the setting `body.max_ticket_tracts`
/// (02 §15.4).
pub const BODY_MAX_TICKET_TRACTS: u8 = 3;

/// The most debris bodies in flight at once: the default of the setting
/// `debris.max_bodies` (02 §12.3). Past it, what is destroyed is left
/// whole as rubble.
pub const DEBRIS_MAX_BODIES: u32 = 256;

/// The most fragments one destroyed element throws.
pub const DEBRIS_PER_ELEMENT: u32 = 3;

/// The share of a destroyed element's mass that flies as fragments. The
/// rest is left as rubble where it stood.
pub const DEBRIS_MASS_SHARE: Real = Real::of(0.25);

/// The share of the energy a packet had left over that its fragments
/// carry off as motion. It is taken out of what passes.
pub const DEBRIS_ENERGY_SHARE: Real = Real::of(0.01);

/// How far a fragment's direction strays from the direction of the
/// blow: each of its three parts by up to this much of a unit.
pub const DEBRIS_SCATTER: Real = Real::of(0.35);

/// The drag coefficient of a fragment.
pub const DEBRIS_DRAG: Real = Real::of(1.0);

/// How much of its speed a fragment keeps when it bounces.
pub const DEBRIS_RESTITUTION: Real = Real::of(0.1);

/// How far past, or short of, a face a body is put when it has gone
/// through it or been turned back, in metres.
const BODY_SKIN_M: Real = Real::of(1.0e-6);

/// How far below itself a body that lies on something looks for what
/// holds it up, in metres.
const BODY_PROBE_M: Real = Real::of(1.0e-4);

/// The most steps of [`BODY_MAX_STEP`] followed when a flight is
/// finished by arithmetic (02 §15.4). A flight longer than this, about
/// six hours, is lost.
const BODY_ARITHMETIC_MAX_STEPS: u32 = 1 << 20;

/// The domain of a body's own random choices.
const BODY_SEED: HashDomain = HashDomain::new("world.body");
/// The domain of the roll for what closes part of a cell.
const BODY_COVERAGE: HashDomain = HashDomain::new("world.body.coverage");
/// The domain of the scatter of fragments.
const DEBRIS_DIRECTION: HashDomain = HashDomain::new("world.debris");

type Vec3 = [Real; 3];

fn add(a: Vec3, b: Vec3) -> Vec3 {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}

fn sub(a: Vec3, b: Vec3) -> Vec3 {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

fn scale(a: Vec3, by: Real) -> Vec3 {
    [a[0] * by, a[1] * by, a[2] * by]
}

fn dot(a: Vec3, b: Vec3) -> Real {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

fn cross(a: Vec3, b: Vec3) -> Vec3 {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}

fn norm(a: Vec3) -> Real {
    dot(a, a).sqrt()
}

/// A vector of length one along `a`, or `None` for one of no length.
fn unit(a: Vec3) -> Option<Vec3> {
    let length = norm(a);
    (length > Real::ZERO && length.is_finite()).then(|| scale(a, Real::ONE / length))
}

const ZERO3: Vec3 = [Real::ZERO; 3];

/// A number from zero up to one, from a hash.
fn unit_of(hash: u64) -> Real {
    // The top 53 bits, as `Rng::unit` takes them.
    Real::of((hash >> 11) as f64 / (1_u64 << 53) as f64)
}

/// A piece of what was destroyed, in flight.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub struct Debris {
    /// What it is made of.
    pub material: MaterialId,
    /// How much of it there is, in cubic metres. Where it comes to rest
    /// it is left as that much of its material's rubble.
    pub volume: Real,
}

clash_core::impl_state_hash!(Debris { material, volume });

/// What a projectile strikes with (07 §11.1): the kind of damage its
/// surface does, and the area and give that decide how concentrated.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub struct Projectile {
    /// The kind of damage it does.
    pub kind: DamageTypeId,
    /// The area of its striking surface, in m².
    pub area: Real,
    /// How far the surface gives, in metres: its stopping distance.
    pub give: Real,
    /// A number the simulation gives it: which shot it is, of what. The
    /// world gives it no meaning and reports it with every event.
    pub tag: u64,
}

clash_core::impl_state_hash!(Projectile {
    kind,
    area,
    give,
    tag
});

/// What a body is.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BodyOwner {
    /// An entity: a creature thrown, an item hurled. While it is an
    /// occupant the world carries it along from cell to cell.
    Entity(EntityId),
    /// A fragment of something destroyed. It strikes with the damage
    /// type flagged `kinetic`, and is left as rubble where it rests.
    Debris(Debris),
    /// A projectile: it strikes with a surface of its own.
    Projectile(Projectile),
}

impl StateHash for BodyOwner {
    fn state_hash(&self, hasher: &mut StateHasher) {
        match self {
            BodyOwner::Entity(entity) => {
                hasher.write_u8(0);
                entity.state_hash(hasher);
            }
            BodyOwner::Debris(debris) => {
                hasher.write_u8(1);
                debris.state_hash(hasher);
            }
            BodyOwner::Projectile(projectile) => {
                hasher.write_u8(2);
                projectile.state_hash(hasher);
            }
        }
    }
}

/// Thrust along a body's path for a time, and how fast it can turn
/// (02 §15, 07 §11.1).
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub struct Drive {
    /// The acceleration its thrust gives it along its path, in m/s².
    pub accel: Real,
    /// How much longer the thrust lasts, in seconds of local time.
    pub burn: Real,
    /// How fast it can turn its path toward the point it is steered
    /// for, in radians a second.
    pub turn: Real,
}

clash_core::impl_state_hash!(Drive { accel, burn, turn });

/// A point mass in flight, with a cross-section (02 §15). There is no
/// rotation.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub struct Body {
    /// What it is.
    pub owner: BodyOwner,
    /// The grid it is in.
    pub grid: GridId,
    /// Where it is, in metres in its grid's frame: east, south and up,
    /// with `z = storey × 3 + altitude`.
    pub pos: [Real; 3],
    /// Its velocity in m/s.
    pub vel: [Real; 3],
    /// Its mass in kilograms.
    pub mass: Real,
    /// Its cross-section in m²: what drag acts on, how wide it is to
    /// what closes part of a cell, and, unless it is a projectile, the
    /// area its impact is spread over.
    pub area: Real,
    /// Its volume in m³, for buoyancy (11 §4.4). Stored.
    pub volume: Real,
    /// Its drag coefficient.
    pub drag: Real,
    /// The distance in metres over which it is stopped by what it
    /// strikes, unless it is a projectile.
    pub depth: Real,
    /// How much of its speed along the direction of entry it keeps when
    /// it is turned back, 0 to 1.
    pub restitution: Real,
    /// Its drive, if it has one.
    pub drive: Option<Drive>,
    /// The planes it is on: it strikes only occupants that share one.
    pub plane: PlaneMask,
    /// Who and what set it going. What it breaks is traced to the
    /// instigator of this.
    pub cause: Cause,
}

clash_core::impl_state_hash!(Body {
    owner,
    grid,
    pos,
    vel,
    mass,
    area,
    volume,
    drag,
    depth,
    restitution,
    drive,
    plane,
    cause
});

impl Body {
    /// A body of a mass, with the rest filled in for a thing whose size
    /// is not known: the volume its mass has at
    /// [`BODY_ASSUMED_DENSITY`], the cross-section of a lump of that
    /// volume, the default drag and stopping depth, no bounce, no
    /// drive, on the default plane. Change what is known afterwards.
    ///
    /// ```
    /// use clash_core::{EntityId, Real};
    /// use clash_world::{Body, BodyOwner, Cause, GridId};
    ///
    /// let thrown = Body::of_mass(
    ///     BodyOwner::Entity(EntityId::new(7, 0)),
    ///     GridId::new(0),
    ///     [Real::of(4.5), Real::of(4.5), Real::of(1.5)],
    ///     [Real::of(20.0), Real::ZERO, Real::ZERO],
    ///     Real::of(80.0),
    ///     Cause::DEBUG,
    /// );
    /// assert_eq!(thrown.volume, Real::of(0.08));
    /// assert!(thrown.area > Real::of(0.18) && thrown.area < Real::of(0.19));
    /// assert_eq!(thrown.kinetic_energy(), Real::of(16_000.0));
    /// ```
    pub fn of_mass(
        owner: BodyOwner,
        grid: GridId,
        pos: [Real; 3],
        vel: [Real; 3],
        mass: Real,
        cause: Cause,
    ) -> Body {
        let volume = mass.max(Real::ZERO) / BODY_ASSUMED_DENSITY;
        let side = volume.cbrt();
        Body {
            owner,
            grid,
            pos,
            vel,
            mass,
            area: side * side,
            volume,
            drag: BODY_DEFAULT_DRAG,
            depth: BODY_DEFAULT_DEPTH_M,
            restitution: Real::ZERO,
            drive: None,
            plane: PlaneMask::DEFAULT,
            cause,
        }
    }

    /// The cell it is in, before any wrap.
    pub fn cell(&self) -> CellPos {
        CellPos::containing(self.pos)
    }

    /// The cell it is in.
    pub fn location(&self) -> Location {
        Location::new(self.grid, self.cell())
    }

    /// How fast it is going, in m/s.
    pub fn speed(&self) -> Real {
        norm(self.vel)
    }

    /// The energy of its motion, in joules.
    pub fn kinetic_energy(&self) -> Real {
        Real::of(0.5) * self.mass * dot(self.vel, self.vel)
    }

    /// The entity it is, if it is one.
    pub fn entity(&self) -> Option<EntityId> {
        match self.owner {
            BodyOwner::Entity(entity) => Some(entity),
            BodyOwner::Debris(_) | BodyOwner::Projectile(_) => None,
        }
    }

    /// The cause of what it does on its way: a process, with the entity
    /// it is as the actor, on behalf of whoever set it going (02 §7.3).
    pub fn doing(&self) -> Cause {
        Cause {
            actor: self.entity().or(self.cause.actor),
            instigator: self.cause.instigator.or(self.cause.actor),
            kind: CauseKind::Process,
        }
    }
}

/// A body in flight, as a process of the world: the body and what its
/// flight has come to.
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub struct BodyFlight {
    /// Its id.
    pub id: BodyId,
    /// The body, as of when it last woke.
    pub body: Body,
    /// The point a driven body is turning toward, in metres in its
    /// grid's frame.
    pub steer: Option<[Real; 3]>,
    /// What its own random choices are drawn from: whether it strikes
    /// something that closes part of a cell.
    pub seed: u64,
    /// The local time it is sleeping for, which its next wake flies.
    pub step: Dur,
    /// Whether it lies on something and slides, held back by friction
    /// (02 §15.3).
    pub resting: bool,
    /// Whether it is beyond loaded space, on its way back in: it is
    /// where and as it will be when it wakes (02 §15.4).
    pub away: bool,
    /// The occupant it struck last, which it does not strike again
    /// while it is still passing it.
    pub last_hit: Option<EntityId>,
    /// The ticket it holds ahead of itself (02 §10.1).
    pub ticket: Option<TicketId>,
    /// How many tracts that were not loaded its ticket has been put on.
    pub ticket_tracts: u8,
}

clash_core::impl_state_hash!(BodyFlight {
    id,
    body,
    steer,
    seed,
    step,
    resting,
    away,
    last_hit,
    ticket,
    ticket_tracts
});

impl BodyFlight {
    /// The cell the body is in.
    pub fn location(&self) -> Location {
        self.body.location()
    }
}

/// A body that left loaded space and came down in a tract that is not
/// loaded (02 §15.4), kept with that tract until it loads.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub struct PendingImpact {
    /// The cell it came down in, in its realm's base grid.
    pub cell: CellPos,
    /// When it came down, in universal time.
    pub tick: Tick,
    /// What it was.
    pub owner: BodyOwner,
    /// Its mass in kilograms.
    pub mass: Real,
    /// The velocity it arrived with, in m/s.
    pub vel: [Real; 3],
    /// The energy it arrived with, in joules.
    pub energy: Real,
    /// Who and what set it going.
    pub cause: Cause,
}

clash_core::impl_state_hash!(PendingImpact {
    cell,
    tick,
    owner,
    mass,
    vel,
    energy,
    cause
});

/// What the world keeps about its bodies besides their processes.
#[derive(Clone, PartialEq, Eq, Debug, Default, Serialize, Deserialize)]
pub struct BodyState {
    /// The process of each body in flight.
    ids: BTreeMap<BodyId, ProcessId>,
    next: u32,
    /// How many of them are debris.
    debris: u32,
}

impl StateHash for BodyState {
    fn state_hash(&self, hasher: &mut StateHasher) {
        self.ids.state_hash(hasher);
        hasher.write_u32(self.next);
        hasher.write_u32(self.debris);
    }
}

/// A set of planes without those that no definition names, where
/// there are `count` planes. The set of every plane stays as it is.
fn known_planes(count: usize, planes: PlaneMask) -> PlaneMask {
    if planes == PlaneMask::ALL {
        return planes;
    }
    let known = match u32::try_from(count) {
        Ok(count) if count < PlaneMask::BITS => (1u32 << count) - 1,
        _ => u32::MAX,
    };
    PlaneMask::from_bits(planes.bits() & known)
}

/// How long a body going at a speed sleeps: the time it takes to cross
/// a cell, and at most [`BODY_MAX_STEP`].
fn step_for(speed: Real) -> Dur {
    if speed <= Real::ZERO {
        return BODY_MAX_STEP;
    }
    let crossing = Dur::from_secs_real(CELL_M / speed);
    Dur(crossing
        .as_micros()
        .clamp(BODY_MIN_STEP.as_micros(), BODY_MAX_STEP.as_micros()))
}

/// The velocity of the medium at a point, as an area set it (11 §4.7).
/// A vortex turns about the middle of its area, from east toward south,
/// draws inward and lifts.
fn drift_at(ambient: &Ambient, pos: Vec3) -> Vec3 {
    let Some(set) = ambient.drift else {
        return ZERO3;
    };
    match set.drift {
        Drift::Flow(velocity) => velocity,
        Drift::Vortex {
            speed,
            inflow,
            lift,
        } => {
            let centre = set.centre.centre_m();
            let out = [pos[0] - centre[0], pos[1] - centre[1], Real::ZERO];
            match unit(out) {
                Some(out) => [
                    -out[1] * speed - out[0] * inflow,
                    out[0] * speed - out[1] * inflow,
                    lift,
                ],
                None => [Real::ZERO, Real::ZERO, lift],
            }
        }
    }
}

/// A velocity turned toward a direction by at most an angle, keeping
/// its speed.
fn turned(vel: Vec3, toward: Vec3, most: Real) -> Vec3 {
    let (Some(from), Some(to)) = (unit(vel), unit(toward)) else {
        return vel;
    };
    let speed = norm(vel);
    let angle = norm(cross(from, to)).atan2(dot(from, to));
    if angle <= most {
        return scale(to, speed);
    }
    if most <= Real::ZERO {
        return vel;
    }
    // The part of `to` that is square to `from` says which way to turn.
    let side = sub(to, scale(from, dot(from, to)));
    let side = match unit(side) {
        Some(side) => side,
        // Straight back: any way round is as good; turn about the
        // axis the heading is least along.
        None => {
            let least = (0..3)
                .min_by_key(|axis| from[*axis].abs())
                .unwrap_or_default();
            let mut axis = ZERO3;
            axis[least] = Real::ONE;
            match unit(cross(from, axis)) {
                Some(side) => side,
                None => return vel,
            }
        }
    };
    let heading = add(scale(from, most.cos()), scale(side, most.sin()));
    scale(heading, speed)
}

/// The velocity of a body after a stretch of flight (02 §15.1): half of
/// gravity's pull, its thrust, drag against the moving medium, the
/// other half of gravity, and its turn toward what it is steered for.
/// With no drag the result is exact for any length of step.
fn flown(
    body: &mut Body,
    steer: Option<Vec3>,
    gravity: Vec3,
    density: Real,
    drift: Vec3,
    dt: Real,
) -> Vec3 {
    let half = dt * Real::of(0.5);
    let mut vel = add(body.vel, scale(gravity, half));
    if let Some(drive) = &mut body.drive
        && drive.burn > Real::ZERO
    {
        let burning = drive.burn.min(dt);
        if let Some(heading) = unit(vel) {
            vel = add(vel, scale(heading, drive.accel * burning));
        }
        drive.burn = (drive.burn - dt).max(Real::ZERO);
    }
    let hold = Real::of(0.5) * density * body.drag * body.area / body.mass;
    if hold > Real::ZERO {
        // Quadratic drag over the step, in the frame of the medium:
        // the closed form for drag alone, which never overshoots.
        let through = sub(vel, drift);
        let slowed = scale(through, Real::ONE / (Real::ONE + hold * norm(through) * dt));
        vel = add(slowed, drift);
    }
    vel = add(vel, scale(gravity, half));
    if let (Some(drive), Some(target)) = (body.drive, steer)
        && drive.turn > Real::ZERO
    {
        vel = turned(vel, sub(target, body.pos), drive.turn * dt);
    }
    vel
}

/// What became of a body in one stretch of flight.
enum Fate {
    /// It flies on.
    Flying,
    /// It has landed, or left for good.
    Ended,
    /// It is beyond loaded space and comes back in after this long.
    Away(Dur),
}

/// How many cells of a path are kept without allocating: a body
/// seldom crosses more in one stretch.
const PATH_INLINE_CELLS: usize = 6;

/// The cells a path passes through, each with how far along the path
/// it is entered, in order.
struct Entered {
    near: [(Location, Real); PATH_INLINE_CELLS],
    count: usize,
    /// Those past the first few.
    far: Vec<(Location, Real)>,
}

impl Entered {
    fn new(grid: GridId) -> Entered {
        let nowhere = (Location::new(grid, CellPos::new(0, 0, 0)), Real::ZERO);
        Entered {
            near: [nowhere; PATH_INLINE_CELLS],
            count: 0,
            far: Vec::new(),
        }
    }

    fn push(&mut self, entry: (Location, Real)) {
        match self.near.get_mut(self.count) {
            Some(slot) => *slot = entry,
            None => self.far.push(entry),
        }
        self.count += 1;
    }

    fn get(&self, at: usize) -> Option<(Location, Real)> {
        if at >= self.count {
            return None;
        }
        match self.near.get(at) {
            Some(entry) => Some(*entry),
            None => self.far.get(at - PATH_INLINE_CELLS).copied(),
        }
    }
}

/// The occupants a body's path meets, and the cells it enters, in
/// order: what the projectile ray asks for each cell.
struct Path<'a> {
    cosmos: &'a Cosmos,
    filter: OccupantFilter,
    /// Each cell the path passes through, with how far along it enters.
    entered: Entered,
    /// The occupants already met on this path.
    met: Vec<EntityId>,
}

impl RayObstacles for Path<'_> {
    fn in_cell(&mut self, loc: Location, span: &RaySpan, out: &mut Vec<RayHit>) {
        let first = self.entered.count == 0;
        self.entered.push((loc, span.enter_m));
        // Whoever shares the cell a stretch starts in was met on the
        // way into it, or is whoever threw the body.
        if first || !self.cosmos.is_occupied(loc, &self.filter).unwrap_or(false) {
            return;
        }
        let Ok(found) = self.cosmos.occupants_at(loc, &self.filter) else {
            return;
        };
        for occupied in found {
            if !self.met.contains(&occupied.id) {
                self.met.push(occupied.id);
                out.push(RayHit::occupant(
                    loc,
                    occupied.id,
                    span.enter_m,
                    Real::ONE,
                    false,
                ));
            }
        }
    }
}

impl Path<'_> {
    /// The axis along which the path enters the cell of a hit: 0, 1 or
    /// 2 for east, south and up. A floor is met from above or below,
    /// and so is anything the path comes down onto inside its cell.
    fn entry_axis(&self, hit: &RayHit, along: Vec3) -> usize {
        const UP: usize = 2;
        if matches!(
            hit.target,
            RayTarget::Element {
                layer: Layer::Floor,
                ..
            }
        ) {
            return UP;
        }
        let found = (0..self.entered.count).rev().find_map(|at| {
            let (loc, enter) = self.entered.get(at)?;
            (loc == hit.loc && enter <= hit.dist_m).then_some((at, loc, enter))
        });
        let Some((at, here, enter)) = found else {
            return UP;
        };
        let before = at.checked_sub(1).and_then(|at| self.entered.get(at));
        let Some((before, _)) = before.filter(|_| hit.dist_m <= enter) else {
            // It dips onto the thing from above, inside the cell.
            return UP;
        };
        let before = before.cell;
        let stepped = [
            here.cell.x != before.x,
            here.cell.y != before.y,
            here.cell.z != before.z,
        ];
        // Through an edge or a corner: the way it is going fastest.
        (0..3)
            .filter(|axis| stepped[*axis])
            .max_by_key(|axis| along[*axis].abs())
            .unwrap_or(UP)
    }
}

impl Cosmos {
    /// A body in flight, as of when it last woke.
    pub fn body(&self, id: BodyId) -> Option<&Body> {
        self.body_flight(id).map(|flight| &flight.body)
    }

    /// A body in flight with what its flight has come to.
    pub fn body_flight(&self, id: BodyId) -> Option<&BodyFlight> {
        let process = *self.activity.bodies.ids.get(&id)?;
        match self.activity.processes.get(process)? {
            WorldProcess::Body(flight) => Some(flight),
            _ => None,
        }
    }

    /// The bodies in flight, in the order they were launched.
    pub fn bodies(&self) -> impl Iterator<Item = &BodyFlight> + '_ {
        self.activity
            .processes
            .iter()
            .filter_map(|(_, process)| match process {
                WorldProcess::Body(flight) => Some(flight),
                _ => None,
            })
    }

    /// The body an entity is, if it is in flight.
    pub fn body_of(&self, entity: EntityId) -> Option<BodyId> {
        self.bodies()
            .find(|flight| flight.body.entity() == Some(entity))
            .map(|flight| flight.id)
    }

    /// How many fragments of destroyed elements are in flight.
    pub fn debris_in_flight(&self) -> u32 {
        self.activity.bodies.debris
    }

    /// The density, in kg/m³, of what fills open cells where these
    /// ambient conditions hold. Zero in vacuum.
    fn medium_density(&self, ambient: &Ambient) -> Real {
        ambient
            .atmosphere
            .and_then(|id| self.defs().atmospheres().get(id))
            .map_or(Real::ZERO, |atmosphere| atmosphere.density)
    }
}

impl WorldCtx<'_> {
    /// Sets a body flying (02 §15; the verb `launch`). From now on the
    /// world moves it: it is a process on the clock of where it is.
    /// Emits `BodyLaunched`.
    ///
    /// An entity that is an occupant stays one: the world carries it
    /// along, and reports each move as `OccupantDisplaced`.
    ///
    /// Refused for a mass that is not above zero, a number that is not
    /// finite, a stopping depth or a restitution out of range, a
    /// material or a kind of damage that does not exist, a place that
    /// is not loaded, and an entity that is already in flight.
    ///
    /// A plane that no definition names has nothing on it: such a bit
    /// of `plane` is dropped. [`PlaneMask::ALL`] stays every plane.
    pub fn launch(&mut self, mut body: Body) -> Result<BodyId, WorldError> {
        let bad = |why: &'static str| WorldError::BadValue {
            what: "a body",
            why,
        };
        let finite = body
            .pos
            .iter()
            .chain(body.vel.iter())
            .chain([body.mass, body.area, body.volume, body.drag, body.depth].iter())
            .all(|part| part.is_finite());
        if !finite {
            return Err(bad("every number of it must be finite"));
        }
        if body.mass <= Real::ZERO {
            return Err(bad("its mass must be above zero"));
        }
        if body.area < Real::ZERO || body.drag < Real::ZERO || body.depth <= Real::ZERO {
            return Err(bad(
                "its cross-section and drag cannot be negative, and its stopping depth must be above zero",
            ));
        }
        if body.restitution < Real::ZERO || body.restitution > Real::ONE {
            return Err(bad("its restitution is from 0 to 1"));
        }
        let defs = self.cosmos.defs();
        match body.owner {
            BodyOwner::Entity(_) => {}
            BodyOwner::Debris(debris) => {
                if defs.materials().get(debris.material).is_none() {
                    return Err(bad("there is no such material"));
                }
            }
            BodyOwner::Projectile(projectile) => {
                if projectile.area < Real::ZERO || projectile.give <= Real::ZERO {
                    return Err(bad(
                        "a projectile's surface cannot be negative, and its give must be above zero",
                    ));
                }
                if defs.damage_types().get(projectile.kind).is_none() {
                    return Err(bad("there is no such kind of damage"));
                }
            }
        }
        body.plane = known_planes(defs.planes().len(), body.plane);
        let at = self.cosmos.normalize(body.location())?;
        if !self.cosmos.is_loaded(at) {
            return Err(WorldError::NotLoaded(at));
        }
        if let Some(entity) = body.entity()
            && self.cosmos.body_of(entity).is_some()
        {
            return Err(bad("the entity is already in flight"));
        }
        self.spawn_body(body)
    }

    /// Makes a body a process and announces it.
    fn spawn_body(&mut self, body: Body) -> Result<BodyId, WorldError> {
        let state = &self.cosmos.activity.bodies;
        let next = state
            .next
            .checked_add(1)
            .ok_or(WorldError::IdsExhausted("body"))?;
        let id = BodyId::new(state.next);
        let at = body.location();
        let realm_seed = self
            .cosmos
            .realm_of(body.grid)
            .and_then(|realm| self.cosmos.realm(realm))
            .map_or(0, |realm| realm.seed);
        let number = i32::from_ne_bytes(id.raw().to_ne_bytes());
        let seed = pos_hash(realm_seed, BODY_SEED, number, 0, 0, 0);
        let clock = self.cosmos.ambient(at)?.clock;
        let mut flight = BodyFlight {
            id,
            body,
            steer: None,
            seed,
            step: step_for(body.speed()),
            resting: false,
            away: false,
            last_hit: None,
            ticket: None,
            ticket_tracts: 0,
        };
        self.lead_ticket(&mut flight);
        let step = flight.step;
        let process = self.cosmos.activity.processes.spawn(
            self.timeline,
            clock,
            step,
            WorldProcess::Body(flight),
        )?;
        let state = &mut self.cosmos.activity.bodies;
        state.next = next;
        state.ids.insert(id, process);
        if matches!(body.owner, BodyOwner::Debris(_)) {
            state.debris = state.debris.saturating_add(1);
        }
        self.events.emit(WorldEvent::BodyLaunched {
            tick: self.timeline.now(),
            body: id,
            owner: body.owner,
            loc: at,
            pos: body.pos,
            vel: body.vel,
            mass: body.mass,
            cause: body.cause,
        });
        Ok(id)
    }

    /// Takes a body out of the table to change it, flown up to now.
    /// `None` if it ended on the way.
    fn take_body(&mut self, id: BodyId) -> Result<Option<(ProcessId, BodyFlight)>, WorldError> {
        let process = *self
            .cosmos
            .activity
            .bodies
            .ids
            .get(&id)
            .ok_or(WorldError::UnknownBody(id))?;
        let table = &mut self.cosmos.activity.processes;
        let left = table
            .wake_of(process)
            .and_then(|wake| self.timeline.remaining(wake))
            .unwrap_or(Dur::ZERO);
        let Some(WorldProcess::Body(mut flight)) = table.take(self.timeline, process) else {
            return Err(WorldError::UnknownBody(id));
        };
        if flight.away {
            // It is not here to be touched: it goes on as it was.
            let _ = self.cosmos.activity.processes.settle(
                self.timeline,
                process,
                WorldProcess::Body(flight),
                Next::Sleep(left),
            );
            return Err(WorldError::BadValue {
                what: "a body",
                why: "it is beyond loaded space",
            });
        }
        let flown = Dur((flight.step.as_micros() - left.as_micros()).max(0));
        match self.fly(&mut flight, flown.as_secs_real()) {
            Fate::Flying => Ok(Some((process, flight))),
            fate => {
                self.put_back(process, flight, fate);
                Ok(None)
            }
        }
    }

    /// Hands a body back to the table with what became of it.
    fn put_back(&mut self, process: ProcessId, mut flight: BodyFlight, fate: Fate) {
        let next = match fate {
            Fate::Flying => {
                flight.step = step_for(flight.body.speed());
                Next::Sleep(flight.step)
            }
            Fate::Away(wait) => {
                flight.step = Dur::ZERO;
                Next::Sleep(wait)
            }
            Fate::Ended => Next::Done,
        };
        let at = flight.location();
        let _ = self.cosmos.activity.processes.settle(
            self.timeline,
            process,
            WorldProcess::Body(flight),
            next,
        );
        if matches!(next, Next::Sleep(_))
            && let Ok(ambient) = self.cosmos.ambient(at)
        {
            self.keep_clock(process, ambient.clock);
        }
    }

    /// Pushes a body: adds an impulse, in kg·m/s east, south and up, to
    /// its motion as it is now (the verb `impulse`). A push with any
    /// lift takes it off what it lies on.
    ///
    /// Refused for a body that is not in flight, one that is beyond
    /// loaded space, and an impulse that is not finite.
    pub fn impulse(&mut self, id: BodyId, j: [Real; 3]) -> Result<(), WorldError> {
        if !j.iter().all(|part| part.is_finite()) {
            return Err(WorldError::BadValue {
                what: "an impulse",
                why: "it must be finite",
            });
        }
        let Some((process, mut flight)) = self.take_body(id)? else {
            return Ok(());
        };
        let body = &mut flight.body;
        body.vel = add(body.vel, scale(j, Real::ONE / body.mass));
        if body.vel[2] > Real::ZERO {
            flight.resting = false;
        }
        flight.last_hit = None;
        self.put_back(process, flight, Fate::Flying);
        Ok(())
    }

    /// Tells a driven body where to turn: toward a point, in metres in
    /// its grid's frame, no faster than its turn rate; or, with no
    /// point, to fly straight (the verb `steer`). The world does not
    /// know what it is aimed at (07 §11.1).
    ///
    /// Refused for a body that is not in flight or is beyond loaded
    /// space, and a point that is not finite.
    pub fn steer(&mut self, id: BodyId, toward: Option<[Real; 3]>) -> Result<(), WorldError> {
        if let Some(point) = toward
            && !point.iter().all(|part| part.is_finite())
        {
            return Err(WorldError::BadValue {
                what: "a point to steer for",
                why: "it must be finite",
            });
        }
        let Some((process, mut flight)) = self.take_body(id)? else {
            return Ok(());
        };
        flight.steer = toward;
        self.put_back(process, flight, Fate::Flying);
        Ok(())
    }

    /// Takes a body out of flight where it is now and returns it: it
    /// was caught, or recovered control (the verb `arrest`). Nothing is
    /// emitted; an occupant it was stays where the flight left it.
    ///
    /// Refused for a body that is not in flight.
    pub fn arrest(&mut self, id: BodyId) -> Result<Body, WorldError> {
        let process = *self
            .cosmos
            .activity
            .bodies
            .ids
            .get(&id)
            .ok_or(WorldError::UnknownBody(id))?;
        let away = matches!(
            self.cosmos.activity.processes.get(process),
            Some(WorldProcess::Body(flight)) if flight.away
        );
        if away {
            let table = &mut self.cosmos.activity.processes;
            let Some(WorldProcess::Body(flight)) = table.remove(self.timeline, process) else {
                return Err(WorldError::UnknownBody(id));
            };
            self.end_body(&flight);
            return Ok(flight.body);
        }
        let Some((process, flight)) = self.take_body(id)? else {
            // It landed on the way here.
            return Err(WorldError::UnknownBody(id));
        };
        self.end_body(&flight);
        self.cosmos
            .activity
            .processes
            .remove(self.timeline, process);
        Ok(flight.body)
    }

    /// A body has woken: it has flown for the time it slept (02 §15.1).
    pub(crate) fn run_body(&mut self, mut flight: BodyFlight) -> (BodyFlight, Next) {
        flight.away = false;
        let dt = flight.step.as_secs_real();
        match self.fly(&mut flight, dt) {
            Fate::Flying => {
                flight.step = step_for(flight.body.speed());
                let step = flight.step;
                (flight, Next::Sleep(step))
            }
            Fate::Away(wait) => {
                flight.step = Dur::ZERO;
                (flight, Next::Sleep(wait))
            }
            Fate::Ended => (flight, Next::Done),
        }
    }

    /// Forgets a body that is no longer in flight: its id, its ticket
    /// and its place in the debris budget.
    fn end_body(&mut self, flight: &BodyFlight) {
        let state = &mut self.cosmos.activity.bodies;
        if state.ids.remove(&flight.id).is_some()
            && matches!(flight.body.owner, BodyOwner::Debris(_))
        {
            state.debris = state.debris.saturating_sub(1);
        }
        if let Some(ticket) = flight.ticket {
            self.cosmos.release_ticket(ticket);
        }
    }

    /// Flies a body for a stretch of local time, in seconds.
    fn fly(&mut self, flight: &mut BodyFlight, dt: Real) -> Fate {
        let here = flight.location();
        let ambient = match self.cosmos.ambient(here) {
            Ok(ambient) if self.cosmos.is_loaded(here) => ambient,
            // Where it is has been unloaded under it, or is no place.
            _ => return self.leave_loaded(flight, here),
        };
        if dt <= Real::ZERO {
            return Fate::Flying;
        }
        if flight.resting {
            match self.held_up(flight) {
                Some(under) if ambient.gravity > Real::ZERO => {
                    return self.slide(flight, dt, ambient.gravity, under);
                }
                _ => flight.resting = false,
            }
        }
        let density = self.cosmos.medium_density(&ambient);
        let drift = drift_at(&ambient, flight.body.pos);
        let before = flight.body.vel;
        let driven = flight
            .body
            .drive
            .is_some_and(|drive| drive.burn > Real::ZERO);
        let after = flown(
            &mut flight.body,
            flight.steer,
            ambient.gravity_vector,
            density,
            drift,
            dt,
        );
        let pulled = ambient.gravity_vector != ZERO3 || driven || norm(drift) >= REST_SPEED;
        if !pulled && norm(after) < REST_SPEED {
            // Nothing moves it and it is hardly moving: it hangs here.
            self.land(flight);
            return Fate::Ended;
        }
        flight.body.vel = after;
        let moved = scale(add(before, after), dt * Real::of(0.5));
        self.travel(flight, moved)
    }

    /// What a body that lies on something lies on: the material of the
    /// floor of its cell or of what stands under it. `None` if nothing
    /// holds it up any more.
    fn held_up(&self, flight: &BodyFlight) -> Option<MaterialId> {
        let body = &flight.body;
        let down = [Real::ZERO, Real::ZERO, -Real::ONE];
        let mut hits = RayHits::new();
        self.cosmos.raycast_into(
            PointM::new(body.grid, body.pos),
            down,
            BODY_PROBE_M,
            RayFilter::PROJECTILE,
            &mut NoObstacles,
            &mut hits,
        );
        hits.hits().iter().find_map(|hit| match hit.target {
            RayTarget::Element { layer, state } if self.strikes(flight, hit, layer) => {
                Some(state.material)
            }
            _ => None,
        })
    }

    /// Whether a body strikes an element its path meets. What closes
    /// the whole width of the cell is always struck. What closes a
    /// share of it is struck by that share widened by the body's own
    /// width, on a roll that is the body's own: the same body meets
    /// the same thing the same way however often it is asked.
    fn strikes(&self, flight: &BodyFlight, hit: &RayHit, layer: Layer) -> bool {
        if hit.coverage >= Real::ONE {
            return true;
        }
        let width = flight.body.area.max(Real::ZERO).sqrt() / CELL_M;
        let chance = hit.coverage + width;
        let cell = hit.loc.cell;
        let roll = pos_hash(
            flight.seed,
            BODY_COVERAGE,
            cell.x,
            cell.y,
            cell.z,
            layer.index() as u64,
        );
        unit_of(roll) < chance
    }

    /// Slides a body that lies on something (02 §15.3): friction slows
    /// it, it still strikes what is in its way, and when it stops it
    /// has landed.
    fn slide(
        &mut self,
        flight: &mut BodyFlight,
        dt: Real,
        gravity: Real,
        under: MaterialId,
    ) -> Fate {
        let friction = self
            .cosmos
            .defs()
            .materials()
            .get(under)
            .and_then(|material| material.friction)
            .unwrap_or(BODY_DEFAULT_FRICTION);
        let vel = flight.body.vel;
        let along = [vel[0], vel[1], Real::ZERO];
        let speed = norm(along);
        let lost = friction * gravity * dt;
        if speed < REST_SPEED || speed <= lost {
            self.land(flight);
            return Fate::Ended;
        }
        let kept = (speed - lost) / speed;
        flight.body.vel = scale(along, kept);
        let moved = scale(along, (Real::ONE + kept) * Real::of(0.5) * dt);
        self.travel(flight, moved)
    }

    /// Moves a body along a straight stretch and deals with what it
    /// meets on the way (02 §15.2). The first thing it strikes ends the
    /// stretch there.
    fn travel(&mut self, flight: &mut BodyFlight, moved: Vec3) -> Fate {
        let length = norm(moved);
        if length <= Real::ZERO || !length.is_finite() {
            return Fate::Flying;
        }
        let from = flight.body.pos;
        let started = flight.location();
        let mut filter = OccupantFilter::ANY
            .on(flight.body.plane)
            .with_flags(OccupantFlags::COLLIDES);
        if let Some(entity) = flight.body.entity() {
            filter = filter.except(entity);
        }
        let mut hits = RayHits::new();
        let axes: Vec<usize> = {
            let cosmos: &Cosmos = self.cosmos;
            let mut path = Path {
                cosmos,
                filter,
                entered: Entered::new(flight.body.grid),
                met: Vec::new(),
            };
            cosmos.raycast_into(
                PointM::new(flight.body.grid, from),
                moved,
                length,
                RayFilter::PROJECTILE,
                &mut path,
                &mut hits,
            );
            hits.hits()
                .iter()
                .map(|hit| path.entry_axis(hit, moved))
                .collect()
        };

        let mut passed_last = false;
        for (hit, axis) in hits.hits().iter().zip(axes) {
            match hit.target {
                RayTarget::Occupant(occupant) => {
                    if flight.last_hit == Some(occupant) {
                        passed_last = true;
                        continue;
                    }
                    let way = moved[axis].signum();
                    self.hit_occupant(flight, hit, axis, way, occupant);
                    self.carry(flight);
                    return Fate::Flying;
                }
                RayTarget::Element { layer, state } => {
                    // Which way along the axis the path went into it.
                    let way = moved[axis].signum();
                    if way == Real::ZERO || !self.strikes(flight, hit, layer) {
                        continue;
                    }
                    self.hit_element(flight, hit, axis, way, layer, state);
                    self.carry(flight);
                    return Fate::Flying;
                }
            }
        }
        if !passed_last {
            flight.last_hit = None;
        }

        let heading = scale(moved, Real::ONE / length);
        match hits.end() {
            RayEnd::Reached | RayEnd::Stopped => {
                flight.body.pos = add(from, moved);
                self.carry(flight);
                if flight.location().cell.tract() != started.cell.tract() {
                    self.lead_ticket(flight);
                }
                Fate::Flying
            }
            RayEnd::Refused(WorldError::OutsideRealm { edge, .. }) => {
                let edge = *edge;
                let reached = hits.length_m();
                self.meet_edge(flight, edge, add(from, scale(heading, reached)))
            }
            RayEnd::Refused(_) => {
                // The end of loaded space: it goes on beyond it, the
                // whole of the way it had to go.
                flight.body.pos = add(from, moved);
                self.leave_loaded(flight, started)
            }
        }
    }

    /// The surface a body strikes with: the kind of damage, and the
    /// area and depth its energy is delivered over. `None` for a body
    /// that is not a projectile where no damage type is flagged
    /// `kinetic`: it then harms nothing.
    fn surface(&self, body: &Body) -> Option<(DamageTypeId, Real, Real)> {
        match body.owner {
            BodyOwner::Projectile(projectile) => {
                Some((projectile.kind, projectile.area, projectile.give))
            }
            BodyOwner::Entity(_) | BodyOwner::Debris(_) => self
                .cosmos
                .defs()
                .kinetic_damage_type()
                .map(|kind| (kind, body.area, body.depth)),
        }
    }

    /// A body strikes an element (02 §15.2): its path went into the
    /// element's cell along `axis`, toward `way` (1 or −1).
    fn hit_element(
        &mut self,
        flight: &mut BodyFlight,
        hit: &RayHit,
        axis: usize,
        way: Real,
        layer: Layer,
        state: ElementState,
    ) {
        let cause = flight.body.doing();
        let body = flight.body;
        // How fast it is closing on the thing. A body that turned back
        // within the stretch is already leaving: it only touches.
        let entering = body.vel[axis] * way;
        if entering <= Real::ZERO {
            flight.body.pos = hit.point;
            flight.body.pos[axis] -= way * BODY_SKIN_M;
            return;
        }
        let energy = Real::of(0.5) * body.mass * entering * entering;
        let surface = self.surface(&body);
        let intensity = surface.map_or(Real::ZERO, |(_, area, depth)| energy / (area * depth));
        let outcome = surface.and_then(|(kind, _, _)| {
            let packet = DamagePacket::new(kind, energy, intensity, cause);
            let mut edit = self.edit(cause);
            let outcome = edit
                .damage_along(hit.loc, layer, packet, unit(body.vel))
                .ok();
            // The packet has arrived in the cell: the fields there that
            // pass its kind of damage on do so (02 §16).
            let _ = edit.conduct(hit.loc, packet);
            outcome
        });
        let result = outcome.map_or(DamageResult::Unaffected, |outcome| outcome.result);
        let mut at = hit.point;
        let kept = match (result, outcome) {
            (DamageResult::Destroyed, Some(outcome)) => {
                // It goes through, with the energy that passed. A floor
                // lies between two cells: the body stays on the near
                // side of where it was, and meets what is in the cell
                // beyond when it enters it.
                let through = match layer {
                    Layer::Floor => -way,
                    Layer::Block | Layer::Fixture => way,
                };
                at[axis] += through * BODY_SKIN_M;
                way * (Real::of(2.0) * outcome.passed.max(Real::ZERO) / body.mass).sqrt()
            }
            _ => {
                at[axis] -= way * BODY_SKIN_M;
                let back = body.restitution * entering;
                if back < REST_SPEED {
                    Real::ZERO
                } else {
                    -way * back
                }
            }
        };
        flight.body.pos = at;
        flight.body.vel[axis] = kept;
        if axis == 2 && way < Real::ZERO && kept == Real::ZERO {
            // It has come down onto something that holds.
            flight.resting = true;
        }
        let contact = self
            .cosmos
            .defs()
            .element(layer, state)
            .and_then(|element| element.contact);
        self.events.emit(WorldEvent::BodyImpact {
            tick: self.timeline.now(),
            body: flight.id,
            owner: body.owner,
            loc: hit.loc,
            layer,
            state,
            pos: hit.point,
            energy: energy - Real::of(0.5) * body.mass * kept * kept,
            intensity,
            result,
            contact,
            cause,
        });
    }

    /// A body strikes an occupant (02 §15.2): its path went into the
    /// occupant's cell along `axis`, toward `way` (1 or −1). The two
    /// share their momentum along the body's path; what the blow does
    /// to the occupant is for the simulation.
    fn hit_occupant(
        &mut self,
        flight: &mut BodyFlight,
        hit: &RayHit,
        axis: usize,
        way: Real,
        occupant: EntityId,
    ) {
        let cause = flight.body.doing();
        let body = flight.body;
        let energy = body.kinetic_energy();
        let surface = self.surface(&body);
        let (kind, intensity) = match surface {
            Some((kind, area, depth)) => (Some(kind), energy / (area * depth)),
            None => (None, Real::ZERO),
        };
        let struck = self
            .cosmos
            .occupant(occupant)
            .map_or(Real::ZERO, |record| record.occupant.mass);
        let (kept, push) = if struck > Real::ZERO {
            let both = body.mass + struck;
            (
                scale(body.vel, (body.mass - body.restitution * struck) / both),
                scale(body.vel, body.mass * (Real::ONE + body.restitution) / both),
            )
        } else {
            // The world was told no mass for it: there is nothing to
            // push against, and the body goes on as it was.
            (body.vel, ZERO3)
        };
        let mut at = hit.point;
        let onward = dot(kept, body.vel) > Real::ZERO;
        at[axis] += if onward { way } else { -way } * BODY_SKIN_M;
        flight.body.pos = at;
        flight.body.vel = kept;
        flight.last_hit = Some(occupant);
        let Some(kind) = kind else {
            return;
        };
        let packet = DamagePacket::new(kind, energy, intensity, cause);
        self.events.emit(WorldEvent::BodyHitOccupant {
            tick: self.timeline.now(),
            body: flight.id,
            owner: body.owner,
            occupant,
            loc: hit.loc,
            pos: hit.point,
            packet,
            vel: body.vel,
            push,
            cause,
        });
        // The packet has arrived in the occupant's cell: the fields
        // there that pass its kind of damage on do so (02 §16).
        let _ = self.edit(cause).conduct(hit.loc, packet);
    }

    /// A body has come to an edge of its realm. A barrier turns it
    /// back like a wall nothing can break. Anything else it leaves by.
    fn meet_edge(&mut self, flight: &mut BodyFlight, edge: Edge, at: Vec3) -> Fate {
        let rule = self
            .cosmos
            .realm_of(flight.body.grid)
            .and_then(|realm| self.cosmos.realm_kind(realm))
            .map(|kind| kind.edge(edge).clone());
        if !matches!(rule, Some(EdgeRule::Barrier)) {
            // W19: a void edge raises `LeftRealm`, and a boundary link
            // carries the body across. Until then it is gone.
            let was = flight.location();
            flight.body.pos = at;
            self.gone(flight, was, None, None);
            return Fate::Ended;
        }
        let axis = match edge {
            Edge::West | Edge::East => 0,
            Edge::North | Edge::South => 1,
            Edge::Bottom | Edge::Top => 2,
        };
        let way = match edge {
            Edge::West | Edge::North | Edge::Bottom => -Real::ONE,
            Edge::East | Edge::South | Edge::Top => Real::ONE,
        };
        let mut at = at;
        at[axis] -= way * BODY_SKIN_M;
        flight.body.pos = at;
        let entering = flight.body.vel[axis] * way;
        if entering > Real::ZERO {
            let back = flight.body.restitution * entering;
            let kept = if back < REST_SPEED {
                Real::ZERO
            } else {
                -way * back
            };
            flight.body.vel[axis] = kept;
            if axis == 2 && way < Real::ZERO && kept == Real::ZERO {
                flight.resting = true;
            }
        }
        self.carry(flight);
        Fate::Flying
    }

    /// Keeps what goes with a body where the body is: its position is
    /// brought round a wrapped axis, and an occupant it is is moved to
    /// its cell, with `OccupantDisplaced`.
    fn carry(&mut self, flight: &mut BodyFlight) {
        let grid = flight.body.grid;
        let cell = flight.body.cell();
        let Ok(inside) = self.cosmos.grid(grid).and_then(|view| view.normalize(cell)) else {
            return;
        };
        if inside != cell {
            flight.body.pos[0] += Real::from(inside.x - cell.x) * CELL_M;
            flight.body.pos[1] += Real::from(inside.y - cell.y) * CELL_M;
        }
        let Some(entity) = flight.body.entity() else {
            return;
        };
        let Some(record) = self.cosmos.occupant(entity) else {
            return;
        };
        // The body is the middle of what fills several cells.
        let corner = inside.offset(IVec3::new(
            -(i32::from(record.footprint.x) - 1) / 2,
            -(i32::from(record.footprint.y) - 1) / 2,
            0,
        ));
        let to = Location::new(grid, corner);
        let cause = flight.body.doing();
        let from = record.loc;
        // Where it cannot follow, because its cells are not all there,
        // it stays behind until it can.
        if let Ok(was) = self.relocate(entity, to, cause)
            && let Some(now) = self.cosmos.occupant(entity).map(|record| record.loc)
            && now != was
        {
            self.events.emit(WorldEvent::OccupantDisplaced {
                tick: self.timeline.now(),
                occupant: entity,
                from,
                to: now,
                cause,
            });
        }
    }

    /// A body has come to rest (02 §15.3): `BodyLanded`, and it is no
    /// longer a body. A fragment is left as rubble.
    fn land(&mut self, flight: &mut BodyFlight) {
        self.carry(flight);
        let cause = flight.body.doing();
        let at = flight.location();
        let at = self.cosmos.normalize(at).unwrap_or(at);
        self.events.emit(WorldEvent::BodyLanded {
            tick: self.timeline.now(),
            body: flight.id,
            owner: flight.body.owner,
            loc: at,
            pos: flight.body.pos,
            cause,
        });
        if let BodyOwner::Debris(debris) = flight.body.owner {
            // Rubble that finds no loaded cell to lie in is lost.
            let _ = self
                .edit(cause)
                .leave_rubble(at, debris.material, debris.volume);
        }
        self.end_body(flight);
    }

    /// Announces that a body has left loaded space, and if it is gone
    /// for good forgets it.
    fn gone(
        &mut self,
        flight: &BodyFlight,
        from: Location,
        lands: Option<Location>,
        at: Option<Tick>,
    ) {
        self.events.emit(WorldEvent::BodyLeftLoaded {
            tick: self.timeline.now(),
            body: flight.id,
            owner: flight.body.owner,
            loc: from,
            lands,
            at,
            returns: false,
            energy: flight.body.kinetic_energy(),
            cause: flight.body.doing(),
        });
        self.end_body(flight);
    }

    /// Holds a body's ticket on the tract ahead of it, where that is
    /// not loaded, so that reconciliation loads it before the body
    /// arrives (02 §10.1): warm, radius 1. The ticket follows it into
    /// at most [`BODY_MAX_TICKET_TRACTS`] tracts that were not loaded;
    /// beyond that the flight is finished by arithmetic (02 §15.4).
    fn lead_ticket(&mut self, flight: &mut BodyFlight) {
        let body = &flight.body;
        let Some(heading) = unit([body.vel[0], body.vel[1], Real::ZERO]) else {
            return;
        };
        let Some(realm) = self.cosmos.realm_of(body.grid) else {
            return;
        };
        let reach = Real::from(TRACT_XY) * CELL_M;
        let ahead = CellPos::containing(add(body.pos, scale(heading, reach))).tract();
        let Ok(ahead) = self.cosmos.normalize_tract(realm, ahead) else {
            return;
        };
        if self.cosmos.is_tract_loaded(realm, ahead)
            || flight.ticket_tracts >= BODY_MAX_TICKET_TRACTS
        {
            return;
        }
        let moved = match flight.ticket {
            Some(ticket) => self.cosmos.move_ticket(ticket, ahead).is_ok(),
            None => {
                let ticket = Ticket::new(
                    realm,
                    TicketAnchor::Fixed(ahead),
                    1,
                    Activity::Warm,
                    TicketReason::Body,
                );
                flight.ticket = self.cosmos.hold_ticket(ticket).ok();
                flight.ticket.is_some()
            }
        };
        if moved {
            flight.ticket_tracts = flight.ticket_tracts.saturating_add(1);
        }
    }

    /// Finishes by arithmetic the flight of a body that has left loaded
    /// space (02 §15.4): gravity, drag and drift are followed with
    /// nothing in the way, until it comes back into a loaded tract or
    /// comes down to the ground.
    ///
    /// The tract records keep no heights yet: the ground is taken to
    /// lie where it did under `from`, the last loaded cell the body was
    /// in.
    fn leave_loaded(&mut self, flight: &mut BodyFlight, from: Location) -> Fate {
        let grid = flight.body.grid;
        let Ok(view) = self.cosmos.grid(grid) else {
            self.gone(flight, from, None, None);
            return Fate::Ended;
        };
        let floor = view.topology().z_min;
        let ground = match view.height(from.cell) {
            Ok(top) if top != HEIGHT_NONE && top != HEIGHT_FULL => top,
            _ => floor,
        };
        let ground = Real::from(ground) * STOREY_M;
        let rate = self
            .timeline
            .effective_rate(view.clock())
            .unwrap_or(Real::ONE);
        let now = self.timeline.now();
        // When so much local time of the realm has passed.
        let tick_after =
            |seconds: Real| (rate > Real::ZERO).then(|| now + Dur::from_secs_real(seconds / rate));
        let dt = BODY_MAX_STEP.as_secs_real();
        let mut body = flight.body;
        let mut steps = 0_u32;
        loop {
            let at = Location::new(grid, body.cell());
            let Ok(ambient) = self.cosmos.ambient(at) else {
                // Past an edge of the realm.
                break;
            };
            let elapsed = dt * Real::from(steps);
            if steps > 0 && self.cosmos.is_loaded(at) {
                // It comes back in here: it is put back as a body, to
                // wake when it arrives.
                let lands = self.cosmos.normalize(at).unwrap_or(at);
                let wait = Dur::from_secs_real(elapsed);
                self.events.emit(WorldEvent::BodyLeftLoaded {
                    tick: now,
                    body: flight.id,
                    owner: body.owner,
                    loc: from,
                    lands: Some(lands),
                    at: tick_after(elapsed),
                    returns: true,
                    energy: body.kinetic_energy(),
                    cause: body.doing(),
                });
                flight.body = body;
                flight.away = true;
                flight.resting = false;
                return Fate::Away(wait);
            }
            if steps >= BODY_ARITHMETIC_MAX_STEPS {
                break;
            }
            let density = self.cosmos.medium_density(&ambient);
            let drift = drift_at(&ambient, body.pos);
            let before = body.vel;
            let after = flown(
                &mut body,
                flight.steer,
                ambient.gravity_vector,
                density,
                drift,
                dt,
            );
            let moved = scale(add(before, after), dt * Real::of(0.5));
            let next = add(body.pos, moved);
            body.vel = after;
            if next[2] <= ground && moved[2] < Real::ZERO {
                // It comes down here, part of the way through the step.
                let share = ((body.pos[2] - ground) / -moved[2]).clamp(Real::ZERO, Real::ONE);
                body.pos = add(body.pos, scale(moved, share));
                body.pos[2] = ground;
                body.vel = add(before, scale(sub(after, before), share));
                let seconds = elapsed + dt * share;
                flight.body = body;
                return self.come_down(flight, from, tick_after(seconds));
            }
            body.pos = next;
            steps += 1;
        }
        flight.body = body;
        self.gone(flight, from, None, None);
        Fate::Ended
    }

    /// A body has come down where nothing is loaded: its impact is kept
    /// with the tract, to be found when the tract loads (02 §15.4).
    fn come_down(&mut self, flight: &mut BodyFlight, from: Location, when: Option<Tick>) -> Fate {
        let body = flight.body;
        let at = Location::new(body.grid, body.cell());
        let Ok(lands) = self.cosmos.normalize(at) else {
            self.gone(flight, from, None, None);
            return Fate::Ended;
        };
        if let (Some(realm), Some(tick)) = (self.cosmos.realm_of(body.grid), when) {
            let impact = PendingEdit::Impact(PendingImpact {
                cell: lands.cell,
                tick,
                owner: body.owner,
                mass: body.mass,
                vel: body.vel,
                energy: body.kinetic_energy(),
                cause: body.doing(),
            });
            // A tract whose region has not been surveyed has no record
            // to keep it on: the event is then all that is left of it.
            let _ = self
                .cosmos
                .add_pending_edit(realm, lands.cell.tract(), impact);
        }
        self.gone(flight, from, Some(lands), when);
        Fate::Ended
    }

    /// Throws the fragments of a destroyed element as debris bodies
    /// (02 §12.3), and returns the mass in kilograms that left as
    /// bodies and the energy in joules they carry. The rest of the
    /// element is left as rubble, and the energy is taken out of what
    /// the packet passes on.
    ///
    /// Nothing is thrown by a blow with no direction, or when
    /// [`DEBRIS_MAX_BODIES`] fragments are in flight already.
    pub(crate) fn throw_debris(&mut self, spec: &DebrisSpec) -> (Real, Real) {
        let nothing = (Real::ZERO, Real::ZERO);
        let Some(along) = spec.dir.and_then(unit) else {
            return nothing;
        };
        let room = DEBRIS_MAX_BODIES.saturating_sub(self.cosmos.activity.bodies.debris);
        let pieces = DEBRIS_PER_ELEMENT.min(room);
        let density = self.cosmos.defs().density(spec.material);
        if pieces == 0
            || spec.mass <= Real::ZERO
            || spec.energy <= Real::ZERO
            || density <= Real::ZERO
        {
            return nothing;
        }
        let count = Real::from(pieces);
        let mass = spec.mass * DEBRIS_MASS_SHARE / count;
        let energy = spec.energy * DEBRIS_ENERGY_SHARE / count;
        let speed = (Real::of(2.0) * energy / mass).sqrt();
        let volume = mass / density;
        let side = volume.cbrt();
        let cell = spec.from.cell;
        // Fragments start in the middle of the cell: half way up what a
        // block or fixture was, just over where a floor was.
        let lift = match spec.layer {
            Layer::Floor => BODY_PROBE_M,
            Layer::Block | Layer::Fixture => STOREY_M * Real::of(0.5),
        };
        let start = [
            Real::from(cell.x) * CELL_M + CELL_M * Real::of(0.5),
            Real::from(cell.y) * CELL_M + CELL_M * Real::of(0.5),
            Real::from(cell.z) * STOREY_M + lift,
        ];
        let tick = self.timeline.now().as_micros().unsigned_abs();
        let realm_seed = self
            .cosmos
            .realm_of(spec.from.grid)
            .and_then(|realm| self.cosmos.realm(realm))
            .map_or(0, |realm| realm.seed);
        let mut thrown = (Real::ZERO, Real::ZERO);
        for piece in 0..pieces {
            // Where each goes depends on where and when the blow fell
            // and which piece it is, and on nothing else.
            let mut heading = along;
            for (axis, part) in heading.iter_mut().enumerate() {
                let salt = (u64::from(piece) << 8) | (spec.layer.index() as u64) << 4 | axis as u64;
                let roll = pos_hash(
                    realm_seed ^ tick,
                    DEBRIS_DIRECTION,
                    cell.x,
                    cell.y,
                    cell.z,
                    salt,
                );
                *part += (unit_of(roll) * Real::of(2.0) - Real::ONE) * DEBRIS_SCATTER;
            }
            let heading = unit(heading).unwrap_or(along);
            let body = Body {
                owner: BodyOwner::Debris(Debris {
                    material: spec.material,
                    volume,
                }),
                grid: spec.from.grid,
                pos: start,
                vel: scale(heading, speed),
                mass,
                area: side * side,
                volume,
                drag: DEBRIS_DRAG,
                depth: BODY_DEFAULT_DEPTH_M,
                restitution: DEBRIS_RESTITUTION,
                drive: None,
                plane: PlaneMask::ALL,
                cause: spec.cause,
            };
            if self.spawn_body(body).is_ok() {
                thrown.0 += mass;
                thrown.1 += energy;
            }
        }
        thrown
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn r(value: f64) -> Real {
        Real::of(value)
    }

    fn lump(vel: Vec3) -> Body {
        Body::of_mass(
            BodyOwner::Entity(EntityId::new(1, 0)),
            GridId::new(0),
            ZERO3,
            vel,
            r(2.0),
            Cause::DEBUG,
        )
    }

    #[test]
    fn a_plane_that_is_not_defined_is_dropped_and_every_plane_stays() {
        let wild = PlaneMask::from_bits(0b1011);
        assert_eq!(known_planes(2, wild), PlaneMask::from_bits(0b11));
        assert_eq!(known_planes(1, wild), PlaneMask::DEFAULT);
        assert_eq!(known_planes(40, wild), wild);
        assert_eq!(known_planes(1, PlaneMask::ALL), PlaneMask::ALL);
    }

    #[test]
    fn a_step_is_the_time_to_cross_a_cell_within_bounds() {
        assert_eq!(step_for(Real::ZERO), BODY_MAX_STEP);
        assert_eq!(step_for(r(1.0)), BODY_MAX_STEP);
        assert_eq!(step_for(r(100.0)), Dur::from_millis(10));
        assert_eq!(step_for(r(900.0)), Dur::from_micros(1111));
        assert_eq!(step_for(r(1.0e9)), BODY_MIN_STEP);
    }

    #[test]
    fn with_no_drag_the_velocity_is_exact() {
        let mut body = lump([r(3.0), Real::ZERO, r(4.0)]);
        let gravity = [Real::ZERO, Real::ZERO, r(-10.0)];
        let after = flown(&mut body, None, gravity, Real::ZERO, ZERO3, r(0.5));
        assert_eq!(after, [r(3.0), Real::ZERO, r(-1.0)]);
    }

    #[test]
    fn drag_brings_a_falling_body_to_its_terminal_speed() {
        // ½ ρ C A v² = m g: v = √(2 m g / (ρ C A)).
        let mut body = lump(ZERO3);
        body.area = r(0.05);
        body.drag = r(1.0);
        let gravity = [Real::ZERO, Real::ZERO, r(-9.81)];
        let density = r(1.225);
        let dt = BODY_MAX_STEP.as_secs_real();
        for _ in 0..5000 {
            body.vel = flown(&mut body, None, gravity, density, ZERO3, dt);
        }
        let terminal = (2.0 * 2.0 * 9.81 / (1.225 * 0.05_f64)).sqrt();
        let speed = -body.vel[2].get();
        assert!(
            (speed - terminal).abs() < terminal * 1.0e-3,
            "{speed} {terminal}"
        );
    }

    #[test]
    fn the_medium_carries_what_flies_in_it() {
        // In a steady wind a light thing ends up going with the wind.
        let mut body = lump(ZERO3);
        body.mass = r(0.01);
        body.area = r(0.01);
        let wind = [r(8.0), Real::ZERO, Real::ZERO];
        let dt = BODY_MAX_STEP.as_secs_real();
        for _ in 0..2000 {
            body.vel = flown(&mut body, None, ZERO3, r(1.225), wind, dt);
        }
        assert!((body.vel[0].get() - 8.0).abs() < 0.05, "{:?}", body.vel);
    }

    #[test]
    fn a_turn_is_no_more_than_it_may_be_and_keeps_the_speed() {
        let east = [r(10.0), Real::ZERO, Real::ZERO];
        let south = [Real::ZERO, r(1.0), Real::ZERO];
        let part = turned(east, south, r(0.25));
        assert!((norm(part).get() - 10.0).abs() < 1.0e-9);
        let angle = part[1].atan2(part[0]).get();
        assert!((angle - 0.25).abs() < 1.0e-9, "{angle}");
        // Enough to finish the turn: it points where it was told.
        let whole = turned(east, south, r(2.0));
        assert!((whole[1].get() - 10.0).abs() < 1.0e-9 && whole[0].get().abs() < 1.0e-9);
        // Straight back still turns, and by no more.
        let back = turned(east, [r(-1.0), Real::ZERO, Real::ZERO], r(0.5));
        let cos = dot(back, east).get() / 100.0;
        assert!((cos - r(0.5).cos().get()).abs() < 1.0e-9, "{cos}");
        // With no turn allowed, or nowhere to turn to, it flies on.
        assert_eq!(turned(east, south, Real::ZERO), east);
        assert_eq!(turned(east, ZERO3, r(1.0)), east);
    }

    #[test]
    fn thrust_lasts_as_long_as_the_burn() {
        let mut body = lump([r(10.0), Real::ZERO, Real::ZERO]);
        body.drive = Some(Drive {
            accel: r(50.0),
            burn: r(0.03),
            turn: Real::ZERO,
        });
        let first = flown(&mut body, None, ZERO3, Real::ZERO, ZERO3, r(0.02));
        assert_eq!(first[0], r(11.0));
        body.vel = first;
        let second = flown(&mut body, None, ZERO3, Real::ZERO, ZERO3, r(0.02));
        assert!((second[0].get() - 11.5).abs() < 1.0e-12);
        body.vel = second;
        assert_eq!(body.drive.map(|drive| drive.burn), Some(Real::ZERO));
        let third = flown(&mut body, None, ZERO3, Real::ZERO, ZERO3, r(0.02));
        assert_eq!(third, second);
    }

    #[test]
    fn a_vortex_turns_draws_in_and_lifts() {
        let ambient = Ambient {
            gravity: Real::ZERO,
            atmosphere: None,
            clock: clash_time::ClockId::UNIVERSAL,
            gravity_vector: ZERO3,
            drift: Some(crate::area::AreaDrift {
                area: crate::area::AreaId::new(0),
                drift: Drift::Vortex {
                    speed: r(5.0),
                    inflow: r(1.0),
                    lift: r(2.0),
                },
                centre: CellPos::new(0, 0, 0),
            }),
        };
        // Ten metres east of the middle: carried south, drawn west, lifted.
        let at = [r(10.5), r(0.5), r(1.5)];
        assert_eq!(drift_at(&ambient, at), [r(-1.0), r(5.0), r(2.0)]);
        // At the middle there is only the lift.
        let middle = [r(0.5), r(0.5), r(1.5)];
        assert_eq!(drift_at(&ambient, middle), [Real::ZERO, Real::ZERO, r(2.0)]);
    }
}
