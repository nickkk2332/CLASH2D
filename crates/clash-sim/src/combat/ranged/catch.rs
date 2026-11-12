//! Catching a shot in flight (07 §11.6): a reaction to a `shot` window
//! whose effect takes the projectile out of the air.
//!
//! `catch_shot` looks for what the window was opened for that is still
//! in flight: the projectiles of the shot, by the order they were
//! fired, and an item thrown. The first within `reach` of the catcher's
//! middle is taken out of flight where it is (`arrest`, 02 §15). An item
//! thrown is then in the catcher's hand; a projectile becomes the item
//! its definition names (`item`), in the catcher's hand, or nothing.
//! Whether a catch is attempted at all, and how often, is the
//! reaction's: the bundled `catch` holds a phase that tries at each tick
//! until something is caught, and draws its contest once.

use clash_core::{EntityId, Real};
use clash_world::{BodyId, BodyOwner, Cause, CellPos, Location};

use super::aim;
use super::store;
use crate::Sim;
use crate::action::WindowId;
use crate::entity::Place;
use crate::reason::Reason;

/// How far from the catcher's middle, in metres, a shot may be caught
/// when the effect names no reach: a little more than an arm's length.
pub const DEFAULT_CATCH_REACH: Real = Real::of(1.5);

/// What a catch took out of the air.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Caught {
    /// Nothing of the window's was within reach.
    Nothing,
    /// A projectile, and the item it became, if it names one.
    Projectile(Option<EntityId>),
    /// An item thrown.
    Item(EntityId),
}

/// The body of a projectile in flight, by its tag.
fn body_of_tag(sim: &Sim, tag: u64) -> Option<BodyId> {
    sim.truth()
        .cosmos()
        .bodies()
        .find(|flight| matches!(flight.body.owner, BodyOwner::Projectile(p) if p.tag == tag))
        .map(|flight| flight.id)
}

/// Whether a body is within `reach` metres of the catcher's middle.
fn within(sim: &Sim, who: EntityId, body: BodyId, reach: Real) -> bool {
    let (Some((loc, middle)), Some(flying)) = (
        aim::band_point(sim, who, None),
        sim.truth().cosmos().body(body),
    ) else {
        return false;
    };
    let at = Location::new(flying.grid, CellPos::containing(flying.pos));
    aim::between(sim, (loc, middle), (at, flying.pos))
        .is_some_and(|way| aim::norm(way) <= reach)
}

/// Catches what a `shot` window was opened for (07 §11.6): see the
/// [module documentation](self). Nothing within reach is not a refusal:
/// it gives [`Caught::Nothing`] and changes nothing.
///
/// Refused for a catcher not in the world (`ranged.not_in_world`) and a
/// reach below zero (`ranged.bad_number`).
pub fn catch_shot(
    sim: &mut Sim,
    who: EntityId,
    window: WindowId,
    reach: Option<Real>,
    cause: Cause,
) -> Result<Caught, Reason> {
    sim.known(who)?;
    let reach = reach.unwrap_or(DEFAULT_CATCH_REACH);
    if !(reach.is_finite() && reach >= Real::ZERO) {
        return Err(super::shoot::bad_number("reach", reach));
    }
    let Some((loc, _)) = aim::footing(sim, who) else {
        return Err(Reason::new(
            super::reason::NOT_IN_WORLD,
            format!("{who} is not standing in the world"),
        )
        .with("entity", who.to_string()));
    };
    let (tags, items) = sim.ranged.of_window(window);
    for tag in tags {
        let Some(body) = body_of_tag(sim, tag) else {
            continue;
        };
        if !within(sim, who, body, reach) {
            continue;
        }
        let projectile = sim.ranged.shot(tag).and_then(|shot| shot.projectile);
        super::flight::caught(sim, super::flight::ShotKey::Tag(tag), body);
        let item = projectile
            .and_then(|id| store::projectile_def(sim.defs(), id))
            .and_then(|def| def.item.as_ref().map(|item| item.id()));
        let Some(item) = item else {
            return Ok(Caught::Projectile(None));
        };
        let made = crate::item::create(sim, item, 1, Place::at(loc), cause)?;
        // A hand that is not free lets it fall where the catcher stands.
        let _ = crate::item::take(sim, who, made, cause);
        return Ok(Caught::Projectile(Some(made)));
    }
    for item in items {
        let Some(body) = sim.truth().cosmos().body_of(item) else {
            continue;
        };
        if !within(sim, who, body, reach) {
            continue;
        }
        super::flight::caught(sim, super::flight::ShotKey::Item(item), body);
        if crate::item::take(sim, who, item, cause).is_err() {
            // A hand that is not free lets it fall where the catcher
            // stands.
            let _ = sim.relocate(item, Place::at(loc), cause);
        }
        return Ok(Caught::Item(item));
    }
    Ok(Caught::Nothing)
}
