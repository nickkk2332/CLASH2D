//! Losing one's footing (06 §11; 02 §14.3, §15).
//!
//! The world says when what held an occupant up has gone
//! (`occupant_unsupported`, which the kernel passes on as
//! `entity_unsupported`). What that does is decided here: a mover that
//! can stay where it is, flying or swimming, stays; any other entity
//! falls.
//!
//! **A fall is a flight** (02 §15). [`fall`] raises `fell` and hands the
//! entity to the world as a body of its own (`WorldCtx::launch` with
//! `BodyOwner::Entity`), at rest where it stands. From then on the world
//! moves it: it carries the occupant from cell to cell, which the kernel
//! follows as `entity_relocated`, and reports what it strikes
//! (`body_impact`, `body_hit_occupant`). When the world reports
//! `body_landed` the entity is taken back ([`take_back`]): `landed` is
//! raised with the height, the speed and the energy of the fall, for
//! whoever turns impacts into hits. Any entity the world carries is
//! taken back so, whoever launched it: where it set off is kept from
//! `body_launched` ([`launched`]).
//!
//! Where the world will not take the entity as a body (it has no mass,
//! say), it is dropped where something from its cell comes to rest
//! (`Cosmos::resting_cell`) at once, and `landed` follows straight away.

use clash_core::{EntityId, Real};
use clash_world::dims::{CELL_M, STOREY_M};
use clash_world::{Body, BodyId, BodyOwner, Cause, Location};

use crate::Sim;
use crate::bus::EventPayload;
use crate::component::Physical;
use crate::entity::Place;
use crate::event::{EventBody, Subject, Where};

use super::mover::{Flight, Mover};
use super::rules::mover_rules;
use super::step::{held_up, standing};
use super::{FELL, LANDED};

/// Millimetres in a metre, for a height within a cell.
const MM_PER_M: Real = Real::of(1000.0);

/// Half of one, for the middle of a cell.
const HALF: Real = Real::of(0.5);

/// How a fall ended.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Landing {
    /// The cell the entity set off from.
    pub from: Location,
    /// Where it came to rest.
    pub to: Location,
    /// How far it fell, in metres: from where it set off to where it
    /// came to rest, and zero for one that came to rest higher.
    pub height_m: Real,
    /// How fast something falling that far is falling when it lands, in
    /// metres a second: `sqrt(2 × gravity × height)`, with nothing taken
    /// off for the air.
    pub speed: Real,
    /// The energy of the fall, in joules: mass × gravity × height.
    pub energy: Real,
}

/// How a fall began.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Fall {
    /// The entity is a body the world moves. `landed` is raised when the
    /// world reports that it has come to rest.
    Flying(BodyId),
    /// The world would not take it as a body: it was dropped where it
    /// comes to rest at once, and `landed` has been raised.
    Dropped(Landing),
}

/// The height of a place in its grid, in metres.
fn height_of(loc: Location, alt_mm: u16) -> Real {
    Real::from_i64(i64::from(loc.cell.z)) * STOREY_M + Real::from_i64(i64::from(alt_mm)) / MM_PER_M
}

/// Whether the world is carrying an entity as a body now.
pub(super) fn in_flight(sim: &Sim, entity: EntityId) -> bool {
    sim.truth().cosmos().body_of(entity).is_some()
}

/// What an entity does when the world reports nothing under it
/// (`entity_unsupported`): it stays if one of its ways of moving lets
/// it be where it is, or if the world is carrying it already, and
/// otherwise falls. Returns how the fall began, or `None` if there was
/// none.
pub fn lose_footing(sim: &mut Sim, entity: EntityId, cause: Cause) -> Option<Fall> {
    let (loc, _) = standing(sim, entity).ok()?;
    if in_flight(sim, entity) {
        return None;
    }
    let footprint = sim.known(entity).ok()?.presence().footprint;
    let rules = mover_rules(sim, entity);
    if held_up(sim, &rules, footprint, loc) {
        return None;
    }
    fall(sim, entity, cause)
}

/// Makes an entity fall from where it stands: raises `fell` there,
/// forgets whatever path it was following, and hands it to the world as
/// a body at rest in the middle of its cell, with its mass, volume,
/// cross-section and planes. Returns how the fall began, or `None` if it
/// did not fall: it is not standing in the world, the world carries it
/// already, there is no gravity where it is, or nothing below to come to
/// rest on.
///
/// See the module documentation of [`movement`](super) on losing one's
/// footing: `landed` follows when the world says the body has come to
/// rest, or at once where the world will not take it as a body.
pub fn fall(sim: &mut Sim, entity: EntityId, cause: Cause) -> Option<Fall> {
    let (from, alt_mm) = standing(sim, entity).ok()?;
    if in_flight(sim, entity) {
        return None;
    }
    let cosmos = sim.truth().cosmos();
    let gravity = cosmos.ambient(from).ok()?.gravity;
    if gravity <= Real::ZERO {
        return None;
    }
    let rest = cosmos.resting_cell(from).ok()?;
    if rest.cell.z >= from.cell.z {
        return None;
    }
    let presence = *sim.known(entity).ok()?.presence();
    let physical = sim.components().get::<Physical>(entity);
    let mass = physical.map_or(presence.mass, |physical| physical.mass().get());
    let middle = |at: i32| (Real::from_i64(i64::from(at)) + HALF) * CELL_M;
    let pos = [
        middle(from.cell.x),
        middle(from.cell.y),
        height_of(from, alt_mm),
    ];
    let mut body = Body::of_mass(
        BodyOwner::Entity(entity),
        from.grid,
        pos,
        [Real::ZERO; 3],
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
    body.plane = presence.plane;
    if let Some(mover) = sim.components_mut().get_mut::<Mover>(entity) {
        mover.journey = None;
    }
    sim.emit(
        Where::Entity(entity),
        cause,
        EventBody::Custom(
            EventPayload::new(FELL)
                .subject("self", Subject::Entity(entity))
                .subject("here", Subject::Loc(from)),
        ),
    );
    match sim.world(|ctx| ctx.launch(body)) {
        Ok(id) => Some(Fall::Flying(id)),
        Err(_) => {
            // The world will not carry it: it comes down at once.
            sim.relocate(entity, Place::at(rest), cause).ok()?;
            let landing = land(sim, entity, height_of(from, alt_mm), from, rest, 0, cause);
            Some(Fall::Dropped(landing))
        }
    }
}

/// Notes where an entity the world has begun to carry set off from
/// (`body_launched`), so that its fall can be measured when it lands.
/// Whatever path it was following is forgotten.
pub fn launched(sim: &mut Sim, entity: EntityId, loc: Location, z_m: Real) {
    if !sim.entities.contains(entity) {
        return;
    }
    let mover = sim
        .components_mut()
        .store_mut::<Mover>()
        .get_or_insert_with(entity, Mover::default);
    mover.journey = None;
    mover.flight = Some(Flight { from: loc, z_m });
}

/// Takes an entity back from the world once its body has come to rest
/// in `loc`, at `z_m` metres in its grid (`body_landed`): raises
/// `landed` with how far it fell from where it set off. Returns the
/// landing, or `None` for an entity that no longer exists.
pub fn take_back(
    sim: &mut Sim,
    entity: EntityId,
    loc: Location,
    z_m: Real,
    cause: Cause,
) -> Option<Landing> {
    if !sim.entities.contains(entity) {
        return None;
    }
    let flight = sim
        .components_mut()
        .get_mut::<Mover>(entity)
        .and_then(|mover| mover.flight.take());
    // Where in its cell it came to rest, to the millimetre the kernel
    // keeps.
    let within = ((z_m - height_of(loc, 0)) * MM_PER_M).round();
    let alt_mm = u16::try_from(within.to_i64().max(0)).unwrap_or(u16::MAX);
    let (from, from_z) = flight.map_or((loc, z_m), |flight| (flight.from, flight.z_m));
    Some(land(sim, entity, from_z, from, loc, alt_mm, cause))
}

/// Works out a landing and raises `landed`.
fn land(
    sim: &mut Sim,
    entity: EntityId,
    from_z: Real,
    from: Location,
    to: Location,
    alt_mm: u16,
    cause: Cause,
) -> Landing {
    let gravity = sim
        .truth()
        .cosmos()
        .ambient(to)
        .map_or(Real::ZERO, |ambient| ambient.gravity)
        .max(Real::ZERO);
    let height_m = (from_z - height_of(to, alt_mm)).max(Real::ZERO);
    let mass = sim.components().get::<Physical>(entity).map_or_else(
        || {
            sim.known(entity)
                .map_or(Real::ZERO, |known| known.presence().mass)
        },
        |physical| physical.mass().get(),
    );
    let landing = Landing {
        from,
        to,
        height_m,
        speed: (Real::of(2.0) * gravity * height_m).sqrt(),
        energy: mass * gravity * height_m,
    };
    sim.emit(
        Where::Entity(entity),
        cause,
        EventBody::Custom(
            EventPayload::new(LANDED)
                .subject("self", Subject::Entity(entity))
                .subject("here", Subject::Loc(to))
                .with("height", landing.height_m)
                .with("speed", landing.speed)
                .with("energy", landing.energy),
        ),
    );
    landing
}
