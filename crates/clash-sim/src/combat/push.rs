//! Push (07 §7): the share of a blow's energy set aside to move its
//! target.
//!
//! ```text
//! speed = sqrt(2 × energy × push × (1 − brace) / mass)
//! ```
//!
//! in the direction of the strike, raised by the attack's `lift`. The
//! hook `knockback_incoming` may change the speed and the direction, or
//! cancel. Then:
//!
//! | The target | Speed | Result |
//! |------------|-------|--------|
//! | Already in flight (07 §7.1) | any | The push is added to its velocity: an impulse of `mass × speed` along the direction (02 §15). `launched`, with `added`. |
//! | Standing | below `combat.stagger_at` | Nothing |
//! | Standing | below `combat.launch_at` | `staggered`, with its severity |
//! | Standing | at or above | It is launched as a body in flight from its cell, at half its height, with its mass, volume, cross-section and planes, and is put first in the attack's `knock_down` posture if it names one. `launched`. The world carries it until it lands (06 §11). |
//!
//! Bracing is a defence's (07 §5.1): until defences are built nothing
//! braces, and the whole push moves the target. A target in flight has
//! no footing to brace with in any case and takes all of it.

use clash_core::{EntityId, Real};
use clash_world::dims::{CELL_M, STOREY_M};
use clash_world::{Body, BodyOwner, Cause};

use super::events::{KNOCKBACK_INCOMING, LAUNCHED, STAGGERED};
use super::reason::{DEFAULT_LAUNCH_AT, DEFAULT_STAGGER_AT, LAUNCH_AT_SETTING, STAGGER_AT_SETTING};
use crate::Sim;
use crate::bus::EventPayload;
use crate::component::Physical;
use crate::entity::Place;
use crate::event::{Subject, Where};
use crate::life::events::{dir_field, field_dir};
use crate::movement::PostureId;
use crate::reason::Reason;

/// Millimetres in a metre, for a height within a cell.
const MM_PER_M: Real = Real::of(1000.0);
/// Half of one: the middle of a cell, half a body's height.
const HALF: Real = Real::of(0.5);
/// Degrees in a half turn, for turning `lift` into radians.
const HALF_TURN_DEG: Real = Real::of(180.0);

/// A push to apply.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Push {
    /// Whom it moves.
    pub target: EntityId,
    /// The joules set aside for it.
    pub energy: Real,
    /// The direction of the strike: east, south, up. Its length does
    /// not matter.
    pub dir: [Real; 3],
    /// Degrees above level it is raised.
    pub lift: Real,
    /// The share of it the target resists (07 §5.1), from 0 to 1.
    pub brace: Real,
    /// A posture the target is put in if it is launched.
    pub knock_down: Option<PostureId>,
}

/// What a push did.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Pushed {
    /// Nothing moved: no energy, no mass, or too slow.
    Nothing,
    /// A handler of `knockback_incoming` cancelled it.
    Cancelled,
    /// The target was staggered at this speed.
    Staggered(Real),
    /// The target was launched at this speed, or this was added to its
    /// flight.
    Launched {
        /// Metres a second.
        speed: Real,
        /// Whether it was in flight already.
        added: bool,
    },
}

/// A setting read as a number, or its default.
fn setting(sim: &Sim, key: &str, default: f64) -> Real {
    match sim.settings().get(key) {
        Some(clash_core::Value::Real(real)) => *real,
        Some(clash_core::Value::Int(int)) => Real::from_i64(*int),
        _ => Real::of(default),
    }
}

/// The length of a vector.
fn length(v: [Real; 3]) -> Real {
    (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt()
}

/// The direction of a push: the strike's direction on the level,
/// raised by `lift` degrees, of length one. Straight up for a strike
/// with no level direction and a lift; `None` for none at all.
pub fn push_dir(dir: [Real; 3], lift: Real) -> Option<[Real; 3]> {
    let level = (dir[0] * dir[0] + dir[1] * dir[1]).sqrt();
    let angle = lift * Real::PI / HALF_TURN_DEG;
    if level > Real::ZERO {
        let (cos, sin) = (angle.cos(), angle.sin());
        Some([dir[0] / level * cos, dir[1] / level * cos, sin])
    } else {
        let up = length(dir);
        (up > Real::ZERO).then(|| [Real::ZERO, Real::ZERO, dir[2].signum()])
    }
}

/// The mass of an entity, as the world is told it: its `Physical`, or
/// its presence.
pub fn mass_of(sim: &Sim, entity: EntityId) -> Real {
    sim.components().get::<Physical>(entity).map_or_else(
        || {
            sim.known(entity)
                .map_or(Real::ZERO, |known| known.presence().mass)
        },
        |physical| physical.mass().get(),
    )
}

/// The speed a push of so many joules gives a mass (07 §7).
pub fn push_speed(energy: Real, brace: Real, mass: Real) -> Real {
    if mass <= Real::ZERO || energy <= Real::ZERO {
        return Real::ZERO;
    }
    let free = (Real::ONE - brace).clamp(Real::ZERO, Real::ONE);
    (Real::of(2.0) * energy * free / mass).sqrt()
}

/// Applies a push (07 §7): see the [module documentation](self).
pub fn push(sim: &mut Sim, push: &Push, cause: Cause) -> Pushed {
    let mass = mass_of(sim, push.target);
    let mut speed = push_speed(push.energy, push.brace, mass);
    let Some(mut dir) = push_dir(push.dir, push.lift) else {
        return Pushed::Nothing;
    };
    if speed <= Real::ZERO {
        return Pushed::Nothing;
    }
    let asked = EventPayload::new(KNOCKBACK_INCOMING)
        .subject("self", Subject::Entity(push.target))
        .with("speed", speed)
        .with("dir", dir_field(Some(dir)));
    match sim.raise(Where::Entity(push.target), cause, asked) {
        Ok(raised) => {
            if raised.cancelled.is_some() {
                return Pushed::Cancelled;
            }
            if let Some(changed) = raised.field("speed").and_then(|field| field.as_real()) {
                speed = changed.max(Real::ZERO);
            }
            if let Some(changed) = field_dir(raised.field("dir"))
                && length(changed) > Real::ZERO
            {
                let l = length(changed);
                dir = [changed[0] / l, changed[1] / l, changed[2] / l];
            }
        }
        Err(reason) => sim.fault(reason),
    }
    if !(speed.is_finite() && speed > Real::ZERO) {
        return Pushed::Nothing;
    }
    let vel = [dir[0] * speed, dir[1] * speed, dir[2] * speed];

    // In the air: the push is added to the flight (07 §7.1).
    if let Some(body) = sim.truth().cosmos().body_of(push.target) {
        let j = [vel[0] * mass, vel[1] * mass, vel[2] * mass];
        if let Err(error) = sim.world(|ctx| ctx.impulse(body, j)) {
            sim.fault(Reason::from_world(&error));
            return Pushed::Nothing;
        }
        launched(sim, push.target, speed, dir, true, cause);
        return Pushed::Launched { speed, added: true };
    }

    let stagger_at = setting(sim, STAGGER_AT_SETTING, DEFAULT_STAGGER_AT);
    let launch_at = setting(sim, LAUNCH_AT_SETTING, DEFAULT_LAUNCH_AT);
    if speed < stagger_at {
        return Pushed::Nothing;
    }
    if speed < launch_at {
        let span = launch_at - stagger_at;
        let severity = if span > Real::ZERO {
            ((speed - stagger_at) / span).clamp(Real::ZERO, Real::ONE)
        } else {
            Real::ONE
        };
        sim.emit(
            Where::Entity(push.target),
            cause,
            crate::event::EventBody::Custom(
                EventPayload::new(STAGGERED)
                    .subject("self", Subject::Entity(push.target))
                    .with("speed", speed)
                    .with("severity", severity)
                    .with("dir", dir_field(Some(dir))),
            ),
        );
        return Pushed::Staggered(speed);
    }
    if let Some(posture) = push.knock_down {
        // A posture the body cannot take changes nothing: the push
        // still launches it.
        if matches!(sim.truth().place(push.target), Some(Place::At { .. })) {
            let _ = crate::movement::set_posture(sim, push.target, Some(posture), cause);
        }
    }
    match launch_body(sim, push.target, vel, cause) {
        Ok(true) => {
            launched(sim, push.target, speed, dir, false, cause);
            Pushed::Launched {
                speed,
                added: false,
            }
        }
        Ok(false) => Pushed::Nothing,
        Err(reason) => {
            sim.fault(reason);
            Pushed::Nothing
        }
    }
}

/// Hands an entity standing in the world to the world as a body in
/// flight (02 §15) at a velocity: from its cell's middle at half its
/// height, with its mass, volume, cross-section and planes. Movement
/// takes it back when it lands (06 §11). What a push that launches
/// does, and what a throw of a creature held does (07 §9.2). `Ok(false)`
/// for one not standing in a cell; `Err` for the world's refusal.
pub(crate) fn launch_body(
    sim: &mut Sim,
    target: EntityId,
    vel: [Real; 3],
    cause: Cause,
) -> Result<bool, Reason> {
    let Some(Place::At { loc, alt_mm }) = sim.truth().place(target) else {
        return Ok(false);
    };
    let presence = match sim.known(target) {
        Ok(known) => *known.presence(),
        Err(_) => return Ok(false),
    };
    let mass = mass_of(sim, target);
    let physical = sim.components().get::<Physical>(target).copied();
    let middle = |at: i32| (Real::from_i64(i64::from(at)) + HALF) * CELL_M;
    let standing = Real::from_i64(i64::from(loc.cell.z)) * STOREY_M
        + Real::from_i64(i64::from(alt_mm)) / MM_PER_M;
    let half_height = physical
        .as_ref()
        .map_or(Real::ZERO, |physical| physical.shape().height.get() * HALF);
    let pos = [
        middle(loc.cell.x),
        middle(loc.cell.y),
        standing + half_height,
    ];
    let mut body = Body::of_mass(BodyOwner::Entity(target), loc.grid, pos, vel, mass, cause);
    if let Some(physical) = &physical {
        if physical.cross_section() > Real::ZERO {
            body.area = physical.cross_section();
        }
        if physical.volume().get() > Real::ZERO {
            body.volume = physical.volume().get();
        }
    }
    body.plane = presence.plane;
    sim.world(|ctx| ctx.launch(body))
        .map(|_| true)
        .map_err(|error| Reason::from_world(&error))
}

/// Raises `launched`.
pub(crate) fn launched(
    sim: &mut Sim,
    target: EntityId,
    speed: Real,
    dir: [Real; 3],
    added: bool,
    cause: Cause,
) {
    sim.emit(
        Where::Entity(target),
        cause,
        crate::event::EventBody::Custom(
            EventPayload::new(LAUNCHED)
                .subject("self", Subject::Entity(target))
                .with("speed", speed)
                .with("dir", dir_field(Some(dir)))
                .with("added", added),
        ),
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn push_speed_follows_the_formula() {
        // 07 §7: a megajoule with a third set aside sends eighty
        // kilograms away at ninety metres a second.
        let speed = push_speed(Real::of(1.0e6 / 3.0), Real::ZERO, Real::of(80.0));
        assert!((speed.get() - 91.287).abs() < 0.01, "{speed}");
        // A braced target is pushed by the unbraced share only.
        let braced = push_speed(Real::of(1000.0), Real::of(0.75), Real::of(80.0));
        let free = push_speed(Real::of(250.0), Real::ZERO, Real::of(80.0));
        assert_eq!(braced, free);
        assert_eq!(
            push_speed(Real::of(10.0), Real::ZERO, Real::ZERO),
            Real::ZERO
        );
    }

    #[test]
    fn lift_raises_the_push() {
        let level = push_dir([Real::of(2.0), Real::ZERO, Real::ZERO], Real::ZERO).unwrap();
        assert_eq!(level, [Real::ONE, Real::ZERO, Real::ZERO]);
        let raised = push_dir([Real::of(2.0), Real::ZERO, Real::ZERO], Real::of(90.0)).unwrap();
        assert!(
            raised[0].abs() < Real::of(1e-12) && (raised[2] - Real::ONE).abs() < Real::of(1e-12)
        );
        assert!(push_dir([Real::ZERO; 3], Real::of(10.0)).is_none());
    }
}
