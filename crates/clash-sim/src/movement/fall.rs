//! Losing one's footing (06 §11; 02 §14.3, §15).
//!
//! The world says when what held an occupant up has gone
//! (`occupant_unsupported`, which the kernel passes on as
//! `entity_unsupported`). What that does is decided here: a mover that
//! can stay where it is, flying or swimming or holding on, stays; any
//! other entity falls.
//!
//! **A fall is not yet a flight.** 02 §15 makes whoever falls a body of
//! the world's until it lands. Until the simulation hands entities to
//! the world as bodies, [`fall`] is the whole of it: the entity is put
//! where something dropped from its cell comes to rest
//! (`Cosmos::resting_cell`), at once, and `landed` reports the height,
//! the speed it would have reached and the energy of the fall, for
//! whoever turns impacts into hits. This one function is what the
//! flight package replaces with `WorldCtx::launch`, and `landed` is
//! then raised when the world reports `body_landed`.

use clash_core::{EntityId, Real};
use clash_world::dims::STOREY_M;
use clash_world::{Cause, Location};

use crate::Sim;
use crate::bus::EventPayload;
use crate::component::Physical;
use crate::entity::Place;
use crate::event::{EventBody, Subject, Where};

use super::mover::Mover;
use super::rules::mover_rules;
use super::step::{held_up, standing};
use super::{FELL, LANDED};

/// Millimetres in a metre, for a height within a cell.
const MM_PER_M: Real = Real::of(1000.0);

/// How a fall ended.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Landing {
    /// Where the entity lost its footing.
    pub from: Location,
    /// Where it came to rest.
    pub to: Location,
    /// How far it fell, in metres.
    pub height_m: Real,
    /// How fast it was falling when it landed, in metres a second:
    /// `sqrt(2 × gravity × height)`, with nothing taken off for the air.
    pub speed: Real,
    /// The energy of the fall, in joules: mass × gravity × height.
    pub energy: Real,
}

/// What an entity does when the world reports nothing under it
/// (`entity_unsupported`): it stays if one of its ways of moving lets
/// it be where it is, and otherwise falls. Returns how the fall ended,
/// or `None` if there was none.
pub fn lose_footing(sim: &mut Sim, entity: EntityId, cause: Cause) -> Option<Landing> {
    let (loc, _) = standing(sim, entity).ok()?;
    let footprint = sim.known(entity).ok()?.presence().footprint;
    let rules = mover_rules(sim, entity);
    if held_up(sim, &rules, footprint, loc) {
        return None;
    }
    fall(sim, entity, cause)
}

/// Drops an entity from where it is to where it comes to rest, and
/// reports it: `fell` where it was, `entity_relocated`, then `landed`
/// where it is with the height, speed and energy of the fall. Whatever
/// path it was following is forgotten. Returns how the fall ended, or
/// `None` if it did not fall: it is not standing in the world, there is
/// no gravity where it is, or there is nowhere below to come to rest.
///
/// This stands in for a flight as a body of the world's (02 §15):
/// until the simulation hands entities to the world as bodies, a fall is
/// over at once. The flight package replaces this one function with
/// `WorldCtx::launch`, and raises `landed` when the world reports
/// `body_landed`.
pub fn fall(sim: &mut Sim, entity: EntityId, cause: Cause) -> Option<Landing> {
    let (from, alt_mm) = standing(sim, entity).ok()?;
    let cosmos = sim.truth().cosmos();
    let gravity = cosmos.ambient(from).ok()?.gravity;
    if gravity <= Real::ZERO {
        return None;
    }
    let to = cosmos.resting_cell(from).ok()?;
    let storeys = from
        .cell
        .z
        .checked_sub(to.cell.z)
        .filter(|storeys| *storeys > 0)?;
    let height_m = Real::from_i64(i64::from(storeys)) * STOREY_M
        + Real::from_i64(i64::from(alt_mm)) / MM_PER_M;
    let mass = sim
        .components()
        .get::<Physical>(entity)
        .map_or(sim.known(entity).ok()?.presence().mass, |physical| {
            physical.mass().get()
        });
    let landing = Landing {
        from,
        to,
        height_m,
        speed: (Real::of(2.0) * gravity * height_m).sqrt(),
        energy: mass * gravity * height_m,
    };
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
    sim.relocate(entity, Place::at(to), cause).ok()?;
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
    Some(landing)
}
