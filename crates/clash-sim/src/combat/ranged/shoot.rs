//! Firing and throwing (07 §11.1 to §11.5): a shot is worked out, its
//! rounds taken from its store, and each projectile handed to the world
//! as a body, or cast as a ray.
//!
//! # A shot, in order
//!
//! | Step | What |
//! |------|------|
//! | 1 | Where it leaves from: the middle of the shooter's cell, at a height (`aim::shot_origin`) |
//! | 2 | Where it is aimed: a direction, a point, a cell at a height, or an entity at the height of a part |
//! | 3 | For each of `count` shots: the next round of the store, if there is one; its projectile and energy |
//! | 4 | Its energy: the effect's, or the round's times the weapon's efficiency; its speed: the effect's, or `√(2 E / m)` |
//! | 5 | Its direction: at the aim point, or along the arc that comes down on it (`lob`), turned by dispersion drawn from the stream `shots` |
//! | 6 | A body launched (02 §15), or a ray cast at once (`instant`); `projectile_fired` |
//! | 7 | The round is spent, and leaves its casing |
//!
//! Every refusal comes before anything changes, except that a store
//! that runs dry part way fires what it had.

use std::sync::Arc;

use clash_core::{EntityId, Real};
use clash_data::Defs;
use clash_world::{Body, BodyId, BodyOwner, Cause, Drive, Location, Projectile};

use super::aim::{self, Vec3};
use super::defs::{ProjectileDef, ProjectileId};
use super::events::{PROJECTILE_FIRED, THROWN};
use super::state::Shot;
use super::{reason, store};
use crate::Sim;
use crate::body::PartKey;
use crate::bus::{EventPayload, FieldValue};
use crate::entity::Place;
use crate::event::{Subject, Where};
use crate::reason::Reason;

/// The random stream dispersion is drawn from (01 §4).
pub const SHOT_STREAM: &str = "shots";

/// Where a shot is aimed.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Aim {
    /// Along a direction: east, south and up.
    Dir(Vec3),
    /// At a point in a cell: the cell, and the point in its grid's frame.
    At(Location, Vec3),
}

/// What a shot is asked to do: the arguments of `shoot`, read.
#[derive(Clone, PartialEq, Debug)]
pub struct ShotSpec {
    /// Who fires.
    pub shooter: EntityId,
    /// Where it is aimed.
    pub aim: Aim,
    /// Whom it is aimed at, if anyone: for events and windows.
    pub target: Option<EntityId>,
    /// The projectile, for a shot that takes none from a store.
    pub projectile: Option<ProjectileId>,
    /// The store its rounds come from, or an item whose feed it is.
    pub store: Option<EntityId>,
    /// The weapon whose `shot` numbers it takes.
    pub weapon: Option<EntityId>,
    /// The energy of each projectile, in joules, in place of the
    /// round's.
    pub energy: Option<Real>,
    /// The speed of each projectile, in m/s, in place of what its
    /// energy gives.
    pub speed: Option<Real>,
    /// Whether it is cast as a ray and strikes at once (07 §11.3).
    pub instant: bool,
    /// The spread, in radians, in place of the weapon's.
    pub spread: Option<Real>,
    /// How many shots: rounds from a store, or projectiles.
    pub count: u32,
    /// How loud, in place of the weapon's.
    pub noise: Option<Real>,
    /// How bright, in place of the weapon's.
    pub flash: Option<Real>,
    /// Whether it is sent along the arc that comes down on the aim
    /// point, not straight at it.
    pub lob: bool,
    /// The height above the shooter's footing it leaves from.
    pub height: Option<Real>,
    /// For a ray: how far it goes, in metres.
    pub range: Option<Real>,
    /// The kind of reaction window it opens for `target` (07 §11.2): it
    /// opens as the shot is fired and closes as the shot is expected to
    /// arrive. `None`, or no target, or an instant shot: none.
    pub window: Option<crate::action::WindowKindId>,
}

impl ShotSpec {
    /// A shot of one projectile by `shooter` along `aim`, with nothing
    /// else said.
    pub fn new(shooter: EntityId, aim: Aim) -> ShotSpec {
        ShotSpec {
            shooter,
            aim,
            target: None,
            projectile: None,
            store: None,
            weapon: None,
            energy: None,
            speed: None,
            instant: false,
            spread: None,
            count: 1,
            noise: None,
            flash: None,
            lob: false,
            height: None,
            range: None,
            window: None,
        }
    }
}

/// What came of a shot.
#[derive(Clone, PartialEq, Debug, Default)]
pub struct Fired {
    /// The bodies launched, in order.
    pub bodies: Vec<BodyId>,
    /// How many projectiles were fired: bodies and rays.
    pub projectiles: u32,
    /// How many rounds the store gave up.
    pub rounds: u32,
    /// For rays: how many things they struck.
    pub struck: u32,
}

fn refuse(code: &'static str, message: impl Into<String>) -> Reason {
    Reason::new(code, message)
}

/// The refusal for a number a shot cannot use.
pub(crate) fn bad_number(what: &str, value: Real) -> Reason {
    refuse(
        reason::BAD_NUMBER,
        format!("the {what} of a shot came out as {value}, which it cannot use"),
    )
    .with("what", what)
    .with("value", value)
}

/// One projectile to fire: what it is and how hard.
struct Load {
    projectile: ProjectileId,
    energy: Option<Real>,
    each: u16,
    round: Option<EntityId>,
}

/// The most shots one `shoot` may fire.
pub fn max_count(sim: &Sim) -> u32 {
    u32::try_from(sim.setting_int(reason::MAX_COUNT_SETTING, reason::DEFAULT_MAX_COUNT))
        .unwrap_or(u32::MAX)
}

/// Fires a shot (07 §11.1): see the [module documentation](self).
///
/// Refused, having changed nothing, for a shooter not standing in the
/// world (`ranged.not_in_world`), a count of none or more than
/// `shoot.max_count` (`ranged.too_many`), a store that holds no round
/// next (`ranged.empty`), a shot with neither a store nor a projectile
/// (`ranged.no_projectile`), and an energy, speed or spread it cannot
/// use (`ranged.bad_number`, `ranged.no_energy`).
pub fn shoot(sim: &mut Sim, spec: &ShotSpec, cause: Cause) -> Result<Fired, Reason> {
    let defs = Arc::clone(sim.defs());
    sim.known(spec.shooter)?;
    let limit = max_count(sim);
    if spec.count == 0 || spec.count > limit {
        return Err(refuse(
            reason::TOO_MANY,
            format!(
                "a shot fires from 1 to {limit} at once, and this asked for {}",
                spec.count
            ),
        )
        .with("count", i64::from(spec.count))
        .with("limit", i64::from(limit)));
    }
    let (from_loc, origin) = aim::shot_origin(sim, spec.shooter, spec.height)
        .ok_or_else(|| not_in_world(spec.shooter))?;
    let weapon_number = |sim: &Sim, name: &str| {
        spec.weapon
            .and_then(|weapon| store::shot_number(sim, weapon, name))
    };
    let spread = spec
        .spread
        .or_else(|| weapon_number(sim, "spread"))
        .unwrap_or(Real::ZERO);
    if !(spread.is_finite() && spread >= Real::ZERO) {
        return Err(bad_number("spread", spread));
    }
    let efficiency = weapon_number(sim, "efficiency").unwrap_or(Real::ONE);
    let noise = spec
        .noise
        .or_else(|| weapon_number(sim, "noise"))
        .unwrap_or(Real::ZERO);
    let flash = spec
        .flash
        .or_else(|| weapon_number(sim, "flash"))
        .unwrap_or(Real::ZERO);
    for (what, value) in [("noise", noise), ("flash", flash)] {
        if !(value.is_finite() && value >= Real::ZERO) {
            return Err(bad_number(what, value));
        }
    }
    let feed = match spec.store {
        Some(store) => Some(store::feed_of(sim, store).ok_or_else(|| {
            refuse(
                reason::NOT_A_STORE,
                format!("{store} is not a store and has none fitted"),
            )
            .with("store", store.to_string())
        })?),
        None => None,
    };
    // Whatever would refuse the first shot refuses before anything
    // changes.
    let first = next_load(sim, &defs, spec, feed)?;
    let toward = match spec.aim {
        Aim::Dir(dir) => dir,
        Aim::At(loc, point) => aim::between(sim, (from_loc, origin), (loc, point))
            .ok_or_else(|| no_aim("a place in another grid"))?,
    };
    if aim::unit(toward).is_none() {
        return Err(no_aim(
            "nowhere: the aim point is where the shot leaves from",
        ));
    }
    let gravity = sim
        .truth()
        .cosmos()
        .ambient(from_loc)
        .map_or(Real::ZERO, |ambient| ambient.gravity);
    let mut fired = Fired::default();
    let mut load = Some(first);
    let mut first_speed = None;
    for shot in 0..spec.count {
        let current = match load.take() {
            Some(current) => current,
            None => match next_load(sim, &defs, spec, feed) {
                Ok(next) => next,
                // A store that has run dry fires no more.
                Err(_) if shot > 0 => break,
                Err(refusal) => return Err(refusal),
            },
        };
        let Some(def) = store::projectile_def(&defs, current.projectile) else {
            return Err(no_projectile());
        };
        let mass = def.mass.get();
        let energy = spec
            .energy
            .or(current.energy.map(|energy| energy * efficiency));
        let speed = match (spec.speed, energy) {
            (Some(speed), _) => speed,
            (None, Some(energy)) if energy.is_finite() && energy >= Real::ZERO => {
                (Real::of(2.0) * energy / mass).sqrt()
            }
            (None, Some(energy)) => return Err(bad_number("energy", energy)),
            (None, None) => {
                return Err(refuse(
                    reason::NO_ENERGY,
                    "a shot names neither a speed nor an energy, and has no round to take one from",
                ));
            }
        };
        if !(speed.is_finite() && speed > Real::ZERO) {
            return Err(bad_number("speed", speed));
        }
        first_speed.get_or_insert(speed);
        let energy = energy.unwrap_or(Real::of(0.5) * mass * speed * speed);
        let base = if spec.lob {
            aim::ballistic_dir(toward, speed, gravity)
                .or_else(|| aim::furthest_dir(toward))
                .unwrap_or(toward)
        } else {
            toward
        };
        let base = aim::unit(base).unwrap_or(base);
        for _ in 0..current.each {
            let dir = aim::disperse(base, spread, sim.rng.stream(SHOT_STREAM));
            if spec.instant {
                // Told before it strikes, as a projectile in flight is.
                announce(
                    sim,
                    spec,
                    &defs,
                    current.projectile,
                    None,
                    (speed, energy, noise, flash),
                    cause,
                );
                let range = spec.range.unwrap_or_else(|| instant_range(sim));
                fired.struck += super::ray::cast(
                    sim,
                    &super::ray::Ray {
                        shooter: spec.shooter,
                        projectile: current.projectile,
                        from: (from_loc, origin),
                        dir,
                        energy,
                        range,
                    },
                    cause,
                );
            } else {
                let body = launch(
                    sim,
                    spec,
                    def,
                    current.projectile,
                    (from_loc, origin),
                    aim::scale(dir, speed),
                    cause,
                )?;
                announce(
                    sim,
                    spec,
                    &defs,
                    current.projectile,
                    Some(body),
                    (speed, energy, noise, flash),
                    cause,
                );
                fired.bodies.push(body);
            }
            fired.projectiles += 1;
        }
        if let Some(round) = current.round {
            spend(sim, round, from_loc, cause)?;
            fired.rounds += 1;
        }
    }
    if let (Some(kind), Some(speed)) = (spec.window, first_speed)
        && !fired.bodies.is_empty()
    {
        let aimed = aimed_at(sim, spec.target, spec.aim);
        let tags: Vec<u64> = fired
            .bodies
            .iter()
            .filter_map(|body| match sim.truth().cosmos().body(*body)?.owner {
                BodyOwner::Projectile(projectile) => Some(projectile.tag),
                _ => None,
            })
            .collect();
        if let Some(window) =
            open_shot_window(sim, kind, spec.shooter, aimed, aim::norm(toward), speed)
        {
            for tag in tags {
                sim.ranged.set_shot_window(tag, window);
            }
        }
    }
    Ok(fired)
}

/// Whom a shot's window is for (07 §11.2): the entity it was aimed at;
/// for one aimed at a cell, whoever with a body stands there, by id.
fn aimed_at(sim: &Sim, target: Option<EntityId>, at: Aim) -> Vec<EntityId> {
    if let Some(target) = target {
        return vec![target];
    }
    let Aim::At(loc, _) = at else {
        return Vec::new();
    };
    let found = sim
        .truth()
        .cosmos()
        .occupants_at(loc, &clash_world::OccupantFilter::ANY)
        .unwrap_or_default();
    let mut ids: Vec<EntityId> = found
        .into_iter()
        .map(|occupied| occupied.id)
        .filter(|id| sim.components().body.contains(*id))
        .collect();
    ids.sort_unstable();
    ids.dedup();
    ids
}

/// Opens the `shot` window of a shot or a throw (07 §11.2): for whoever
/// it is aimed at, from now until it is expected to arrive, at `speed`
/// over `distance` metres. Those who notice it in time may answer it
/// (03 §7). Nothing is opened for a shot aimed at nobody; a window that
/// cannot be opened is a fault.
fn open_shot_window(
    sim: &mut Sim,
    kind: crate::action::WindowKindId,
    source: EntityId,
    aimed_at: Vec<EntityId>,
    distance: Real,
    speed: Real,
) -> Option<crate::action::WindowId> {
    if aimed_at.is_empty() || !(speed > Real::ZERO && distance.is_finite()) {
        return None;
    }
    let open_for = clash_core::Dur::from_secs_real(distance.max(Real::ZERO) / speed);
    match sim.open_window(crate::action::OpenWindow {
        kind,
        source: Some(source),
        aimed_at,
        open_for,
    }) {
        Ok(window) => Some(window),
        Err(reason) => {
            sim.fault(reason);
            None
        }
    }
}

/// The setting of how far an instant shot goes, or its default.
fn instant_range(sim: &Sim) -> Real {
    super::setting_real(sim, reason::INSTANT_RANGE_SETTING)
        .filter(|range| range.is_finite() && *range > Real::ZERO)
        .unwrap_or(Real::of(reason::DEFAULT_INSTANT_RANGE))
}

fn not_in_world(entity: EntityId) -> Reason {
    refuse(
        reason::NOT_IN_WORLD,
        format!("{entity} is not standing in the world"),
    )
    .with("entity", entity.to_string())
}

fn no_aim(why: &str) -> Reason {
    refuse(reason::NO_AIM, format!("a shot is aimed at {why}")).with("at", why)
}

fn no_projectile() -> Reason {
    refuse(
        reason::NO_PROJECTILE,
        "a shot names no projectile and has no store to take a round from",
    )
}

/// What the next shot fires: the store's next round, or the projectile
/// named.
fn next_load(
    sim: &Sim,
    defs: &Arc<Defs>,
    spec: &ShotSpec,
    feed: Option<EntityId>,
) -> Result<Load, Reason> {
    match feed {
        Some(store) => {
            let Some(round) = store::next_round(sim, store) else {
                return Err(
                    refuse(reason::EMPTY, format!("{store} has no round left to fire"))
                        .with("store", store.to_string()),
                );
            };
            let def = store::round_of(sim, round).ok_or_else(no_projectile)?;
            let projectile = spec.projectile.unwrap_or_else(|| def.projectile.id());
            if store::projectile_def(defs, projectile).is_none() {
                return Err(no_projectile());
            }
            Ok(Load {
                projectile,
                energy: Some(def.energy.get()),
                each: def.count,
                round: Some(round),
            })
        }
        None => {
            let projectile = spec.projectile.ok_or_else(no_projectile)?;
            if store::projectile_def(defs, projectile).is_none() {
                return Err(no_projectile());
            }
            Ok(Load {
                projectile,
                energy: None,
                each: 1,
                round: None,
            })
        }
    }
}

/// Hands one projectile to the world as a body.
fn launch(
    sim: &mut Sim,
    spec: &ShotSpec,
    def: &ProjectileDef,
    projectile: ProjectileId,
    from: (Location, Vec3),
    vel: Vec3,
    cause: Cause,
) -> Result<BodyId, Reason> {
    let (loc, origin) = from;
    let tag = sim.ranged.fire(Shot {
        projectile: Some(projectile),
        surface: None,
        by: Some(spec.shooter),
        weapon: spec.weapon,
        aimed: spec.target,
        grid: loc.grid,
        last: origin,
        met: Vec::new(),
        burst: false,
    });
    let mass = def.mass.get();
    let mut body = Body::of_mass(
        BodyOwner::Projectile(Projectile {
            kind: def.surface.kind.id(),
            area: def.surface.area,
            give: def.surface.give.get(),
            tag,
        }),
        loc.grid,
        origin,
        vel,
        mass,
        cause,
    );
    body.area = def.cross_section();
    body.drag = def.drag;
    body.depth = def.surface.give.get();
    body.restitution = def.restitution;
    body.drive = def.thrust.as_ref().map(|thrust| Drive {
        accel: thrust.accel,
        burn: thrust.burn.dur().as_secs_real(),
        turn: Real::ZERO,
    });
    if let Some(presence) = sim.truth().entity(spec.shooter).map(|e| *e.presence()) {
        body.plane = presence.plane;
    }
    match sim.world(|ctx| ctx.launch(body)) {
        Ok(id) => Ok(id),
        Err(error) => {
            sim.ranged.unfire(tag);
            Err(Reason::from_world(&error))
        }
    }
}

/// Raises `projectile_fired`.
fn announce(
    sim: &mut Sim,
    spec: &ShotSpec,
    defs: &Arc<Defs>,
    projectile: ProjectileId,
    body: Option<BodyId>,
    (speed, energy, noise, flash): (Real, Real, Real, Real),
    cause: Cause,
) {
    let name = defs
        .try_registry::<ProjectileDef>()
        .map_or("", |registry| registry.name(projectile))
        .to_owned();
    let mut payload = EventPayload::new(PROJECTILE_FIRED)
        .subject("self", Subject::Entity(spec.shooter))
        .with("projectile", name.as_str())
        .with(
            "body",
            body.map_or(FieldValue::Null, |body| {
                FieldValue::from(i64::from(body.raw()))
            }),
        )
        .with("speed", speed)
        .with("energy", energy)
        .with("noise", noise)
        .with("flash", flash)
        .with("instant", spec.instant);
    if let Some(target) = spec.target {
        payload = payload.subject("target", Subject::Entity(target));
    }
    if let Some(weapon) = spec.weapon {
        payload = payload.subject("weapon", Subject::Entity(weapon));
    }
    if let Err(reason) = sim.raise(Where::Entity(spec.shooter), cause, payload) {
        sim.fault(reason);
    }
}

/// Uses up a round fired, and leaves its casing where the shooter is.
fn spend(sim: &mut Sim, round: EntityId, at: Location, cause: Cause) -> Result<(), Reason> {
    let casing = store::round_of(sim, round).and_then(|def| def.casing.as_ref().map(|c| c.id()));
    crate::item::spend(sim, round, 1, cause)?;
    if let Some(casing) = casing {
        crate::item::create(sim, casing, 1, Place::at(at), cause)?;
    }
    Ok(())
}

/// What a throw is asked to do: the arguments of `throw`, read.
#[derive(Clone, PartialEq, Debug)]
pub struct ThrowSpec {
    /// Who throws.
    pub thrower: EntityId,
    /// What is thrown: an item it holds.
    pub what: EntityId,
    /// Where it is aimed.
    pub aim: Aim,
    /// Whom it is aimed at, if anyone.
    pub target: Option<EntityId>,
    /// The energy it is thrown with, in joules.
    pub energy: Option<Real>,
    /// The speed it is thrown at, in m/s, in place of what the energy
    /// gives.
    pub speed: Option<Real>,
    /// The spread, in radians.
    pub spread: Real,
    /// The surface of the item it strikes with, by id; `None` for a
    /// blunt blow.
    pub surface: Option<String>,
    /// Whether it is sent along the arc that comes down on the aim
    /// point.
    pub lob: bool,
}

/// Throws an item (07 §11.1): it leaves the thrower's grasp and flies
/// as a body that is the item, with its own mass and cross-section, and
/// strikes what it meets with the surface named, through the same
/// delivery as a projectile's. Raises `thrown`.
///
/// Refused for a thing `thrower` does not hold (`ranged.not_held`), a
/// surface the item does not have (`ranged.no_surface`), a thrower not
/// standing in the world, and a speed or energy it cannot use.
pub fn throw(sim: &mut Sim, spec: &ThrowSpec, cause: Cause) -> Result<BodyId, Reason> {
    throw_opening(sim, spec, None, cause)
}

/// [`throw`], opening a reaction window of the kind given for whoever
/// it is aimed at (07 §11.2), as a shot's does: the `throw` verb's
/// `window`.
pub fn throw_opening(
    sim: &mut Sim,
    spec: &ThrowSpec,
    window: Option<crate::action::WindowKindId>,
    cause: Cause,
) -> Result<BodyId, Reason> {
    sim.known(spec.thrower)?;
    // A creature the thrower grips is thrown as a body (07 §9.2).
    if crate::combat::grip::grips(sim, spec.thrower, spec.what) {
        return throw_held(sim, spec, cause);
    }
    if crate::item::holder_of(sim, spec.what) != Some(spec.thrower) {
        return Err(refuse(
            reason::NOT_HELD,
            format!("{} does not hold {}", spec.thrower, spec.what),
        )
        .with("item", spec.what.to_string())
        .with("who", spec.thrower.to_string()));
    }
    let surface = match &spec.surface {
        None => None,
        Some(name) => {
            let found = store::item_def(sim, spec.what).and_then(|def| {
                def.surfaces
                    .iter()
                    .position(|surface| &surface.id == name)
                    .and_then(|at| u8::try_from(at).ok())
            });
            Some(found.ok_or_else(|| {
                refuse(
                    reason::NO_SURFACE,
                    format!("{} has no surface `{name}` to strike with", spec.what),
                )
                .with("item", spec.what.to_string())
                .with("surface", name.as_str())
            })?)
        }
    };
    let (from_loc, origin) =
        aim::shot_origin(sim, spec.thrower, None).ok_or_else(|| not_in_world(spec.thrower))?;
    let physical = sim
        .components()
        .get::<crate::component::Physical>(spec.what)
        .copied();
    let mass = physical.map_or(Real::ZERO, |physical| physical.mass().get());
    if mass <= Real::ZERO {
        return Err(bad_number("mass", mass));
    }
    let speed = match (spec.speed, spec.energy) {
        (Some(speed), _) => speed,
        (None, Some(energy)) if energy.is_finite() && energy >= Real::ZERO => {
            (Real::of(2.0) * energy / mass).sqrt()
        }
        (None, Some(energy)) => return Err(bad_number("energy", energy)),
        (None, None) => {
            return Err(refuse(
                reason::NO_ENERGY,
                "a throw names neither a speed nor an energy",
            ));
        }
    };
    if !(speed.is_finite() && speed > Real::ZERO) {
        return Err(bad_number("speed", speed));
    }
    if !(spec.spread.is_finite() && spec.spread >= Real::ZERO) {
        return Err(bad_number("spread", spec.spread));
    }
    let toward = match spec.aim {
        Aim::Dir(dir) => dir,
        Aim::At(loc, point) => aim::between(sim, (from_loc, origin), (loc, point))
            .ok_or_else(|| no_aim("a place in another grid"))?,
    };
    let gravity = sim
        .truth()
        .cosmos()
        .ambient(from_loc)
        .map_or(Real::ZERO, |ambient| ambient.gravity);
    let base = if spec.lob {
        aim::ballistic_dir(toward, speed, gravity)
            .or_else(|| aim::furthest_dir(toward))
            .unwrap_or(toward)
    } else {
        toward
    };
    let base = aim::unit(base).ok_or_else(|| no_aim("nowhere"))?;
    let dir = aim::disperse(base, spec.spread, sim.rng.stream(SHOT_STREAM));
    // It leaves the hand where the thrower stands, and flies from there.
    crate::item::drop(sim, spec.what, cause)?;
    let presence = sim
        .truth()
        .entity(spec.what)
        .map(|entity| *entity.presence());
    let mut body = Body::of_mass(
        BodyOwner::Entity(spec.what),
        from_loc.grid,
        origin,
        aim::scale(dir, speed),
        mass,
        cause,
    );
    if let Some(physical) = physical {
        if physical.cross_section() > Real::ZERO {
            body.area = physical.cross_section();
        }
        if physical.volume().get() > Real::ZERO {
            body.volume = physical.volume().get();
        }
    }
    if let Some(presence) = presence {
        body.plane = presence.plane;
    }
    sim.ranged.throw(
        spec.what,
        Shot {
            projectile: None,
            surface,
            by: Some(spec.thrower),
            weapon: None,
            aimed: spec.target,
            grid: from_loc.grid,
            last: origin,
            met: Vec::new(),
            burst: false,
        },
    );
    let id = match sim.world(|ctx| ctx.launch(body)) {
        Ok(id) => id,
        Err(error) => {
            sim.ranged.end_throw(spec.what);
            return Err(Reason::from_world(&error));
        }
    };
    let mut payload = EventPayload::new(THROWN)
        .subject("self", Subject::Entity(spec.thrower))
        .subject("what", Subject::Entity(spec.what))
        .with("speed", speed)
        .with("energy", Real::of(0.5) * mass * speed * speed);
    if let Some(target) = spec.target {
        payload = payload.subject("target", Subject::Entity(target));
    }
    if let Err(reason) = sim.raise(Where::Entity(spec.thrower), cause, payload) {
        sim.fault(reason);
    }
    if let Some(kind) = window {
        let aimed = aimed_at(sim, spec.target, spec.aim);
        if let Some(window) =
            open_shot_window(sim, kind, spec.thrower, aimed, aim::norm(toward), speed)
        {
            sim.ranged.set_throw_window(spec.what, window);
        }
    }
    Ok(id)
}

/// Throws a creature the thrower grips (07 §9.2): the grip ends, and the
/// held is launched as a body in flight from where it stands, at the
/// middle of its body, at `speed`, or `√(2 E / m)` of its own mass,
/// toward the aim; the world resolves what it meets (02 §15.2), and
/// movement takes it back when it lands. Raises `launched` and
/// `thrown`.
fn throw_held(sim: &mut Sim, spec: &ThrowSpec, cause: Cause) -> Result<BodyId, Reason> {
    let held = spec.what;
    let mass = crate::combat::push::mass_of(sim, held);
    if mass <= Real::ZERO {
        return Err(bad_number("mass", mass));
    }
    let speed = match (spec.speed, spec.energy) {
        (Some(speed), _) => speed,
        (None, Some(energy)) if energy.is_finite() && energy >= Real::ZERO => {
            (Real::of(2.0) * energy / mass).sqrt()
        }
        (None, Some(energy)) => return Err(bad_number("energy", energy)),
        (None, None) => {
            return Err(refuse(
                reason::NO_ENERGY,
                "a throw names neither a speed nor an energy",
            ));
        }
    };
    if !(speed.is_finite() && speed > Real::ZERO) {
        return Err(bad_number("speed", speed));
    }
    if !(spec.spread.is_finite() && spec.spread >= Real::ZERO) {
        return Err(bad_number("spread", spec.spread));
    }
    let from = aim::band_point(sim, held, None).ok_or_else(|| not_in_world(held))?;
    let toward = match spec.aim {
        Aim::Dir(dir) => dir,
        Aim::At(loc, point) => aim::between(sim, from, (loc, point))
            .ok_or_else(|| no_aim("a place in another grid"))?,
    };
    // Thrown at itself, or at where it stands: away from the thrower, on
    // the level.
    let toward = if spec.target == Some(held)
        || aim::norm([toward[0], toward[1], Real::ZERO]) < Real::of(0.5)
    {
        let away = crate::combat::strike::offset(sim, spec.thrower, held)
            .ok_or_else(|| no_aim("nowhere"))?;
        [away[0], away[1], Real::ZERO]
    } else {
        toward
    };
    let gravity = sim
        .truth()
        .cosmos()
        .ambient(from.0)
        .map_or(Real::ZERO, |ambient| ambient.gravity);
    let base = if spec.lob {
        aim::ballistic_dir(toward, speed, gravity)
            .or_else(|| aim::furthest_dir(toward))
            .unwrap_or(toward)
    } else {
        toward
    };
    let base = aim::unit(base).ok_or_else(|| no_aim("nowhere"))?;
    let dir = aim::disperse(base, spec.spread, sim.rng.stream(SHOT_STREAM));
    crate::combat::grip::let_fly(sim, spec.thrower, held, cause)?;
    if !crate::combat::push::launch_body(sim, held, aim::scale(dir, speed), cause)? {
        return Err(not_in_world(held));
    }
    let body = sim
        .truth()
        .cosmos()
        .body_of(held)
        .ok_or_else(|| not_in_world(held))?;
    crate::combat::push::launched(sim, held, speed, dir, false, cause);
    let mut payload = EventPayload::new(THROWN)
        .subject("self", Subject::Entity(spec.thrower))
        .subject("what", Subject::Entity(held))
        .with("speed", speed)
        .with("energy", Real::of(0.5) * mass * speed * speed);
    if let Some(target) = spec.target {
        payload = payload.subject("target", Subject::Entity(target));
    }
    if let Err(reason) = sim.raise(Where::Entity(spec.thrower), cause, payload) {
        sim.fault(reason);
    }
    Ok(body)
}

/// The point to aim at for an entity: the middle of a part's band, or
/// `height` above its footing, or the middle of its body.
pub fn aim_at_entity(
    sim: &Sim,
    target: EntityId,
    part: Option<PartKey>,
    height: Option<Real>,
) -> Option<Aim> {
    if let (Some(height), None) = (height, part) {
        let (loc, mut point) = aim::footing(sim, target)?;
        point[2] += height;
        return Some(Aim::At(loc, point));
    }
    let (loc, point) = aim::band_point(sim, target, part)?;
    Some(Aim::At(loc, point))
}
