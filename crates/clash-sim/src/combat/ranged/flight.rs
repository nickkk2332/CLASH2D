//! A shot in flight (07 §11.1): what the simulation does with what the
//! world says of a projectile's body, or of an item thrown.
//!
//! | The world says | The simulation |
//! |----------------|----------------|
//! | `body_hit_occupant` | The creature is struck at the height the body met it, on a part whose band holds that height, through what it wears and its body's layers (`Sim::hit`): the same delivery as a strike's. What exits flies on: the body is given the speed that energy has, along its way (`WorldCtx::impulse`). With nothing left, a projectile is lodged where it is and ends. A creature with no part at that height is passed by. `projectile_hit` |
//! | `body_impact` | `projectile_hit` at the cell: what the element took is the world's to say (02 §15.2) |
//! | `body_landed`, `body_left_loaded` | The shot ends; a projectile leaves its field where it came down |
//!
//! At each of these the stretch of its path since it was last known is
//! looked along for those it passed within `projectile.near_miss` of,
//! each told once by `projectile_passed`.
//!
//! A projectile's `on_impact` runs at the first thing it strikes, and it
//! ends there.
//!
//! The hit is given no direction, so that the body wiring does not
//! also carry what exits to the block behind (06 §7.4): the projectile
//! itself carries it on.

use std::sync::Arc;

use clash_core::{EntityId, Real};
use clash_expr::Value;
use clash_world::defs::DamageTypeId;
use clash_world::dims::CELL_M;
use clash_world::{
    BodyId, BodyOwner, Cause, CellPos, DamagePacket, DamageResult, Location, OccupantFilter,
    WorldEvent,
};

use super::aim::{self, Vec3};
use super::defs::{ProjectileDef, ProjectileId};
use super::events::{PROJECTILE_HIT, PROJECTILE_PASSED};
use super::reason;
use super::state::Shot;
use super::store;
use crate::Sim;
use crate::body::props::part_handle;
use crate::body::{Hit, HitAt, PartKey};
use crate::bus::{EventPayload, FieldValue};
use crate::event::{Subject, Where};
use crate::life::events::handle_field;

/// Which shot: a projectile by its tag, or an item thrown.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum ShotKey {
    /// A projectile.
    Tag(u64),
    /// An item thrown.
    Item(EntityId),
}

/// What the world said of a shot in flight.
#[derive(Clone, PartialEq, Debug)]
pub(crate) enum FlightNews {
    /// It struck an occupant.
    Hit {
        key: ShotKey,
        body: BodyId,
        occupant: EntityId,
        loc: Location,
        pos: Vec3,
        vel: Vec3,
        kind: DamageTypeId,
        energy: Real,
        cause: Cause,
    },
    /// It struck an element.
    Impact {
        key: ShotKey,
        body: BodyId,
        loc: Location,
        pos: Vec3,
        energy: Real,
        intensity: Real,
        result: DamageResult,
        cause: Cause,
    },
    /// It came to rest.
    Landed {
        key: ShotKey,
        loc: Location,
        pos: Vec3,
        cause: Cause,
    },
    /// It left loaded space for good.
    Gone { key: ShotKey, cause: Cause },
}

impl FlightNews {
    /// What the world said of a shot, if the event is about one that is
    /// in flight.
    pub(crate) fn of(sim: &Sim, event: &WorldEvent) -> Option<FlightNews> {
        let key_of = |owner: &BodyOwner| match owner {
            BodyOwner::Projectile(projectile) => sim
                .ranged
                .shot(projectile.tag)
                .map(|_| ShotKey::Tag(projectile.tag)),
            BodyOwner::Entity(entity) => sim.ranged.thrown(*entity).map(|_| ShotKey::Item(*entity)),
            BodyOwner::Debris(_) => None,
        };
        match event {
            WorldEvent::BodyHitOccupant {
                body,
                owner,
                occupant,
                loc,
                pos,
                packet,
                vel,
                ..
            } => Some(FlightNews::Hit {
                key: key_of(owner)?,
                body: *body,
                occupant: *occupant,
                loc: *loc,
                pos: *pos,
                vel: *vel,
                kind: packet.kind,
                energy: packet.energy,
                cause: packet.cause,
            }),
            WorldEvent::BodyImpact {
                body,
                owner,
                loc,
                pos,
                energy,
                intensity,
                result,
                cause,
                ..
            } => Some(FlightNews::Impact {
                key: key_of(owner)?,
                body: *body,
                loc: *loc,
                pos: *pos,
                energy: *energy,
                intensity: *intensity,
                result: *result,
                cause: *cause,
            }),
            WorldEvent::BodyLanded {
                owner,
                loc,
                pos,
                cause,
                ..
            } => Some(FlightNews::Landed {
                key: key_of(owner)?,
                loc: *loc,
                pos: *pos,
                cause: *cause,
            }),
            WorldEvent::BodyLeftLoaded {
                owner,
                returns: false,
                cause,
                ..
            } => Some(FlightNews::Gone {
                key: key_of(owner)?,
                cause: *cause,
            }),
            _ => None,
        }
    }

    /// Whether this takes the place of what the world's event would
    /// otherwise do to the occupant (06 §7.4): a shot strikes with its
    /// own surface, not a blunt blow spread over the body.
    pub(crate) fn replaces_harm(&self) -> bool {
        matches!(self, FlightNews::Hit { .. })
    }
}

/// A striking surface of a shot.
#[derive(Clone, Copy, PartialEq, Debug)]
pub(crate) struct Surface {
    pub kind: DamageTypeId,
    pub area: Real,
    pub give: Real,
    pub span: Real,
}

impl Surface {
    fn intensity(&self, energy: Real) -> Real {
        let volume = self.area * self.give;
        if volume > Real::ZERO {
            energy / volume
        } else {
            Real::INFINITY
        }
    }
}

/// What came of a shot at a creature.
#[derive(Clone, Copy, PartialEq, Debug)]
pub(crate) enum Struck {
    /// Nothing of it was there: it went by.
    Passed,
    /// It struck: the part, the joules taken, the joules that exit.
    Hit {
        part: Option<PartKey>,
        absorbed: Real,
        exits: Real,
    },
    /// A handler of `hit_incoming` stopped it.
    Stopped,
}

/// Strikes a creature, or an item, that a shot met at `pos` (07 §11.1):
/// at the height above its footing that the shot is at, on a part whose
/// band holds it, through what it wears and its body's layers. A
/// creature with no part at that height is passed by.
pub(crate) fn strike_creature(
    sim: &mut Sim,
    target: EntityId,
    pos: Vec3,
    surface: Surface,
    energy: Real,
    cause: Cause,
) -> Struck {
    let intensity = surface.intensity(energy);
    let packet = DamagePacket::new(surface.kind, energy, intensity, cause);
    if sim.components().body.contains(target) {
        let Some((_, foot)) = aim::footing(sim, target) else {
            return Struck::Passed;
        };
        let height = pos[2] - foot[2];
        let there = match (sim.components().body.get(target), sim.body_defs()) {
            (Some(body), Some(defs)) => body.parts_at_height(defs, height).next().is_some(),
            _ => false,
        };
        if !there {
            return Struck::Passed;
        }
        let hit = Hit {
            target,
            at: HitAt::Height(height),
            packet,
            area: surface.area,
            span: surface.span,
            dir: None,
        };
        return match sim.hit(hit) {
            Ok(outcome) if outcome.cancelled.is_some() => Struck::Stopped,
            Ok(outcome) => {
                let report = &outcome.report;
                let mut absorbed = report.absorbed;
                for worn in &report.worn {
                    absorbed += worn.absorbed;
                }
                Struck::Hit {
                    part: report.struck.first().map(|struck| struck.part),
                    absorbed,
                    exits: outcome.exited(),
                }
            }
            Err(refusal) => {
                sim.fault(refusal);
                Struck::Passed
            }
        };
    }
    if sim.components().get::<crate::item::Item>(target).is_some() {
        return match crate::item::damage(sim, target, &packet) {
            Ok(hit) if hit.outcome.result == DamageResult::Unaffected => Struck::Stopped,
            Ok(hit) => Struck::Hit {
                part: None,
                absorbed: hit.outcome.absorbed,
                exits: hit.outcome.passed.max(Real::ZERO),
            },
            Err(refusal) => {
                sim.fault(refusal);
                Struck::Passed
            }
        };
    }
    Struck::Passed
}

/// What `projectile_hit` says.
pub(crate) struct HitNews {
    pub shooter: Option<EntityId>,
    pub projectile: Option<ProjectileId>,
    pub body: Option<BodyId>,
    pub target: Option<EntityId>,
    pub here: Location,
    pub part: Option<PartKey>,
    pub energy: Real,
    pub intensity: Real,
    pub absorbed: Real,
    pub passed: Real,
}

/// The qualified id of a projectile, or nothing for an item thrown.
fn projectile_name(sim: &Sim, projectile: Option<ProjectileId>) -> String {
    projectile
        .and_then(|id| {
            sim.defs()
                .try_registry::<ProjectileDef>()
                .map(|registry| registry.name(id).to_owned())
        })
        .unwrap_or_default()
}

/// Raises `projectile_hit`.
pub(crate) fn announce_hit(sim: &mut Sim, news: &HitNews, cause: Cause) {
    let part = match (news.part, news.target, sim.body_defs()) {
        (Some(part), Some(target), Some(defs)) => {
            handle_field(part_handle(defs.kinds(), target, part))
        }
        _ => FieldValue::Null,
    };
    let mut payload = EventPayload::new(PROJECTILE_HIT)
        .subject("here", Subject::Loc(news.here))
        .with("projectile", projectile_name(sim, news.projectile).as_str())
        .with(
            "body",
            news.body.map_or(FieldValue::Null, |body| {
                FieldValue::from(i64::from(body.raw()))
            }),
        )
        .with("part", part)
        .with("energy", news.energy)
        .with("intensity", news.intensity)
        .with("absorbed", news.absorbed)
        .with("passed", news.passed)
        .with("instant", news.body.is_none());
    if let Some(shooter) = news.shooter.filter(|id| sim.truth().entity(*id).is_some()) {
        payload = payload.subject("self", Subject::Entity(shooter));
    }
    if let Some(target) = news.target {
        payload = payload.subject("target", Subject::Entity(target));
    }
    if let Err(refusal) = sim.raise(Where::Loc(news.here), cause, payload) {
        sim.fault(refusal);
    }
}

/// The setting of how close a shot must pass to raise
/// `projectile_passed`, in metres.
fn near_miss(sim: &Sim) -> Real {
    super::setting_real(sim, reason::NEAR_MISS_SETTING)
        .filter(|near| near.is_finite() && *near >= Real::ZERO)
        .unwrap_or(Real::of(reason::DEFAULT_NEAR_MISS))
}

fn shot_of(sim: &Sim, key: ShotKey) -> Option<Shot> {
    match key {
        ShotKey::Tag(tag) => sim.ranged.shot(tag).cloned(),
        ShotKey::Item(item) => sim.ranged.thrown(item).cloned(),
    }
}

fn shot_mut(sim: &mut Sim, key: ShotKey) -> Option<&mut Shot> {
    match key {
        ShotKey::Tag(tag) => sim.ranged.shot_mut(tag),
        ShotKey::Item(item) => sim.ranged.thrown_mut(item),
    }
}

/// Tells everyone a shot passed close to on the stretch from where it
/// was last known to `to`, once each: `projectile_passed`. Then it is
/// known to be at `to`.
fn pass_by(
    sim: &mut Sim,
    key: ShotKey,
    body: Option<BodyId>,
    to: Vec3,
    struck: Option<EntityId>,
    cause: Cause,
) {
    let Some(shot) = shot_of(sim, key) else {
        return;
    };
    let near = near_miss(sim);
    let from = shot.last;
    let middle = aim::scale(aim::add(from, to), Real::of(0.5));
    let reach = aim::norm(aim::sub(to, from)) * Real::of(0.5) + near;
    let centre = Location::new(shot.grid, CellPos::containing(middle));
    let found = sim
        .truth()
        .cosmos()
        .occupants_within(centre, reach, &OccupantFilter::ANY)
        .unwrap_or_default();
    // On the level first, by the middle of the cell, with a cell to
    // spare: only those it may have passed are looked at closely.
    let level = |point: Vec3| [point[0], point[1], Real::ZERO];
    let (flat_from, flat_to) = (level(from), level(to));
    let close: Vec<clash_world::Occupied> = found
        .into_iter()
        .filter(|occupied| {
            let middle = level(aim::cell_point(
                Location::new(shot.grid, occupied.cell),
                Real::ZERO,
            ));
            aim::distance_to_segment(middle, flat_from, flat_to) <= near + CELL_M
        })
        .collect();
    let mut ids = clash_world::distinct_ids(&close);
    ids.sort_unstable();
    let mut passed: Vec<(EntityId, Real)> = Vec::new();
    for id in ids {
        if Some(id) == shot.by
            || Some(id) == struck
            || shot.met.contains(&id)
            || !sim.components().body.contains(id)
        {
            continue;
        }
        let Some((_, point)) = aim::band_point(sim, id, None) else {
            continue;
        };
        let distance = aim::distance_to_segment(point, from, to);
        if distance <= near {
            passed.push((id, distance));
        }
    }
    if let Some(shot) = shot_mut(sim, key) {
        shot.last = to;
        shot.met.extend(passed.iter().map(|(id, _)| *id));
        if let Some(struck) = struck
            && !shot.met.contains(&struck)
        {
            shot.met.push(struck);
        }
    }
    for (id, distance) in passed {
        announce_passed(sim, &shot, body, id, distance, cause);
    }
}

/// Raises `projectile_passed` for one it passed close to.
fn announce_passed(
    sim: &mut Sim,
    shot: &Shot,
    body: Option<BodyId>,
    who: EntityId,
    distance: Real,
    cause: Cause,
) {
    let mut payload = EventPayload::new(PROJECTILE_PASSED)
        .subject("self", Subject::Entity(who))
        .with("projectile", projectile_name(sim, shot.projectile).as_str())
        .with(
            "body",
            body.map_or(FieldValue::Null, |body| {
                FieldValue::from(i64::from(body.raw()))
            }),
        )
        .with("distance", distance);
    if let Some(by) = shot.by.filter(|id| sim.truth().entity(*id).is_some()) {
        payload = payload.subject("shooter", Subject::Entity(by));
    }
    if let Err(refusal) = sim.raise(Where::Entity(who), cause, payload) {
        sim.fault(refusal);
    }
}

/// The surface a shot strikes with: a projectile's, an item's surface,
/// or the blunt blow the world gave for an item thrown with none.
fn surface_of(sim: &Sim, key: ShotKey, shot: &Shot, body: BodyId, kind: DamageTypeId) -> Surface {
    match key {
        ShotKey::Tag(_) => {
            let def = shot
                .projectile
                .and_then(|id| store::projectile_def(sim.defs(), id));
            if let Some(def) = def {
                return Surface {
                    kind: def.surface.kind.id(),
                    area: def.surface.area,
                    give: def.surface.give.get(),
                    span: def.surface.span.get(),
                };
            }
        }
        ShotKey::Item(item) => {
            let found = shot
                .surface
                .and_then(|at| store::item_def(sim, item)?.surfaces.get(usize::from(at)));
            if let Some(surface) = found {
                return Surface {
                    kind: surface.kind.id(),
                    area: surface.area,
                    give: surface.give.get(),
                    span: surface.span.get(),
                };
            }
        }
    }
    let (area, give) = sim
        .truth()
        .cosmos()
        .body(body)
        .map_or((Real::ONE, clash_world::BODY_DEFAULT_DEPTH_M), |flying| {
            (flying.area, flying.depth)
        });
    Surface {
        kind,
        area,
        give,
        span: Real::ZERO,
    }
}

/// Carries out what the world said of a shot in flight: see the
/// [module documentation](self).
pub(crate) fn follow(sim: &mut Sim, news: FlightNews) {
    match news {
        FlightNews::Hit {
            key,
            body,
            occupant,
            loc,
            pos,
            vel,
            kind,
            energy,
            cause,
        } => {
            let Some(shot) = shot_of(sim, key) else {
                return;
            };
            pass_by(sim, key, Some(body), pos, Some(occupant), cause);
            let surface = surface_of(sim, key, &shot, body, kind);
            let struck = strike_creature(sim, occupant, pos, surface, energy, cause);
            let exits = match struck {
                Struck::Passed => energy,
                Struck::Stopped => Real::ZERO,
                Struck::Hit {
                    part,
                    absorbed,
                    exits,
                } => {
                    announce_hit(
                        sim,
                        &HitNews {
                            shooter: shot.by,
                            projectile: shot.projectile,
                            body: Some(body),
                            target: Some(occupant),
                            here: loc,
                            part,
                            energy,
                            intensity: surface.intensity(energy),
                            absorbed,
                            passed: exits,
                        },
                        cause,
                    );
                    if burst(sim, key, loc, Some(occupant), cause) {
                        end(sim, key, Some(body), Some(loc), cause);
                        return;
                    }
                    exits
                }
            };
            fly_on(sim, key, body, vel, exits, Some(loc), cause);
        }
        FlightNews::Impact {
            key,
            body,
            loc,
            pos,
            energy,
            intensity,
            result,
            cause,
        } => {
            pass_by(sim, key, Some(body), pos, None, cause);
            let ShotKey::Tag(_) = key else {
                return;
            };
            let Some(shot) = shot_of(sim, key) else {
                return;
            };
            let left = sim
                .truth()
                .cosmos()
                .body(body)
                .map_or(Real::ZERO, clash_world::Body::kinetic_energy);
            let passed = if result == DamageResult::Destroyed {
                left
            } else {
                Real::ZERO
            };
            announce_hit(
                sim,
                &HitNews {
                    shooter: shot.by,
                    projectile: shot.projectile,
                    body: Some(body),
                    target: None,
                    here: loc,
                    part: None,
                    energy,
                    intensity,
                    absorbed: energy,
                    passed,
                },
                cause,
            );
            // What it did not go through and does not bounce off holds
            // it: a bullet in a wall, an arrow in the ground.
            let lodged = result != DamageResult::Destroyed
                && shot
                    .projectile
                    .and_then(|id| store::projectile_def(sim.defs(), id))
                    .is_some_and(|def| def.restitution <= Real::ZERO);
            if burst(sim, key, loc, None, cause) || lodged {
                end(sim, key, Some(body), Some(loc), cause);
            }
        }
        FlightNews::Landed {
            key,
            loc,
            pos,
            cause,
        } => {
            pass_by(sim, key, None, pos, None, cause);
            end(sim, key, None, Some(loc), cause);
        }
        FlightNews::Gone { key, cause } => end(sim, key, None, None, cause),
    }
}

/// Gives a shot that struck something the speed of what exits, along
/// its way; a projectile with nothing left is lodged where it is.
fn fly_on(
    sim: &mut Sim,
    key: ShotKey,
    body: BodyId,
    vel: Vec3,
    exits: Real,
    at: Option<Location>,
    cause: Cause,
) {
    let Some(flying) = sim.truth().cosmos().body(body).copied() else {
        return;
    };
    let Some(way) = aim::unit(vel) else {
        return;
    };
    if exits <= Real::ZERO {
        if let ShotKey::Tag(_) = key {
            end(sim, key, Some(body), at, cause);
        }
        return;
    }
    let speed = (Real::of(2.0) * exits / flying.mass).sqrt();
    let wanted = aim::scale(way, speed);
    let impulse = aim::scale(aim::sub(wanted, flying.vel), flying.mass);
    if let Err(error) = sim.world(|ctx| ctx.impulse(body, impulse)) {
        sim.fault(crate::reason::Reason::from_world(&error));
    }
}

/// Runs a projectile's `on_impact` the first time it strikes something.
/// Returns whether it did: the projectile ends there.
fn burst(
    sim: &mut Sim,
    key: ShotKey,
    here: Location,
    target: Option<EntityId>,
    cause: Cause,
) -> bool {
    let ShotKey::Tag(tag) = key else {
        return false;
    };
    let Some(shot) = sim.ranged.shot(tag).cloned() else {
        return false;
    };
    let defs = Arc::clone(sim.defs());
    let Some(def) = shot
        .projectile
        .and_then(|id| store::projectile_def(&defs, id))
    else {
        return false;
    };
    if shot.burst || def.on_impact.is_empty() {
        return false;
    }
    if let Some(shot) = sim.ranged.shot_mut(tag) {
        shot.burst = true;
    }
    let shooter = shot
        .by
        .filter(|id| sim.truth().entity(*id).is_some())
        .map_or(Value::Null, |id| sim.subject_value(Subject::Entity(id)));
    let subjects = [
        shooter,
        sim.subject_value(Subject::Loc(here)),
        target.map_or(Value::Null, |id| sim.subject_value(Subject::Entity(id))),
    ];
    let mut locals = Vec::new();
    sim.run_effects(&def.on_impact, &subjects, &mut locals, None, cause);
    true
}

/// Ends a shot: a projectile still in flight is taken out where it is,
/// leaves its field where it ends, and is forgotten; an item thrown is
/// forgotten, and lies where the world leaves it.
fn end(sim: &mut Sim, key: ShotKey, body: Option<BodyId>, at: Option<Location>, cause: Cause) {
    match key {
        ShotKey::Tag(tag) => {
            if let Some(body) = body
                && sim.truth().cosmos().body(body).is_some()
            {
                let _ = sim.world(|ctx| ctx.arrest(body));
            }
            let Some(shot) = sim.ranged.end_shot(tag) else {
                return;
            };
            let defs = Arc::clone(sim.defs());
            let leaves = shot
                .projectile
                .and_then(|id| store::projectile_def(&defs, id))
                .and_then(|def| def.leaves.as_ref());
            if let (Some(leaves), Some(at)) = (leaves, at) {
                let name = defs
                    .try_registry::<clash_world::field::FieldDef>()
                    .map(|registry| registry.name(leaves.field.id()).to_owned())
                    .unwrap_or_default();
                let field = sim.truth().cosmos().defs().fields().resolve(&name, None);
                if let Some(field) = field {
                    let amount = leaves.amount;
                    let _ = sim.world(|ctx| ctx.edit(cause).add_field(at, field, amount));
                }
            }
        }
        ShotKey::Item(item) => {
            sim.ranged.end_throw(item);
        }
    }
}
