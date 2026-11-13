//! Instant shots (07 §11.3): nothing flies. A ray is cast along the
//! shot's direction, and what it meets is struck at once, in order:
//! elements by a packet (02 §12), creatures by a hit at the height the
//! ray passes them (06 §7), each with what passed the one before. A
//! beam, and any shot too fast for its flight to matter.
//!
//! What closes part of a cell is struck on a draw from the stream
//! `shots` against the share it closes.

use clash_core::{EntityId, Real};
use clash_world::{
    Cause, DamagePacket, DamageResult, Location, OccupantFilter, PointM, RayFilter, RayHit,
    RayHits, RayObstacles, RaySpan, RayTarget,
};

use super::aim::Vec3;
use super::defs::ProjectileId;
use super::flight::{self, Struck};
use super::shoot::SHOT_STREAM;
use crate::Sim;

/// A ray to cast.
pub(crate) struct Ray {
    /// Who fired it.
    pub shooter: EntityId,
    /// What it strikes as.
    pub projectile: ProjectileId,
    /// Where it starts: the cell, and the point in its grid's frame.
    pub from: (Location, Vec3),
    /// Its direction, of length one.
    pub dir: Vec3,
    /// The energy it carries, in joules.
    pub energy: Real,
    /// How far it goes, in metres.
    pub range: Real,
}

/// The creatures a ray passes: those with bodies, other than whoever
/// cast it, each once.
struct Creatures<'a> {
    sim: &'a Sim,
    shooter: EntityId,
    met: Vec<EntityId>,
}

impl RayObstacles for Creatures<'_> {
    fn in_cell(&mut self, loc: Location, span: &RaySpan, out: &mut Vec<RayHit>) {
        let Ok(found) = self
            .sim
            .truth()
            .cosmos()
            .occupants_at(loc, &OccupantFilter::ANY)
        else {
            return;
        };
        for occupied in found {
            let id = occupied.id;
            if id != self.shooter
                && !self.met.contains(&id)
                && self.sim.components().body.contains(id)
            {
                self.met.push(id);
                out.push(RayHit::occupant(loc, id, span.enter_m, Real::ONE, false));
            }
        }
    }
}

/// Casts a ray and strikes what it meets (07 §11.3). Returns how many
/// things it struck.
pub(crate) fn cast(sim: &mut Sim, ray: &Ray, cause: Cause) -> u32 {
    let defs = std::sync::Arc::clone(sim.defs());
    let Some(def) = super::store::projectile_def(&defs, ray.projectile) else {
        return 0;
    };
    let (kind, area, give, span) = (
        def.surface.kind.id(),
        def.surface.area,
        def.surface.give.get(),
        def.surface.span.get(),
    );
    let (from_loc, origin) = ray.from;
    let mut hits = RayHits::new();
    {
        let mut creatures = Creatures {
            sim,
            shooter: ray.shooter,
            met: Vec::new(),
        };
        sim.truth().cosmos().raycast_into(
            PointM::new(from_loc.grid, origin),
            ray.dir,
            ray.range,
            RayFilter::PROJECTILE,
            &mut creatures,
            &mut hits,
        );
    }
    let mut energy = ray.energy;
    let mut struck = 0;
    for hit in hits.hits().to_vec() {
        if energy <= Real::ZERO {
            break;
        }
        match hit.target {
            RayTarget::Element { layer, state } => {
                if state.is_none() {
                    continue;
                }
                let roll = sim.rng.stream(SHOT_STREAM).unit();
                if hit.coverage < Real::ONE && roll >= hit.coverage {
                    continue;
                }
                let intensity = energy / (area * give);
                let packet = DamagePacket::new(kind, energy, intensity, cause);
                let Ok(outcome) = sim.world(|ctx| {
                    ctx.edit(cause)
                        .damage_along(hit.loc, layer, packet, Some(ray.dir))
                }) else {
                    break;
                };
                struck += 1;
                let passed = if outcome.result == DamageResult::Destroyed {
                    outcome.passed.max(Real::ZERO)
                } else {
                    Real::ZERO
                };
                flight::announce_hit(
                    sim,
                    &flight::HitNews {
                        shooter: Some(ray.shooter),
                        projectile: Some(ray.projectile),
                        body: None,
                        target: None,
                        here: hit.loc,
                        part: None,
                        energy,
                        intensity,
                        absorbed: energy - passed,
                        passed,
                    },
                    cause,
                );
                energy = passed;
            }
            RayTarget::Occupant(occupant) => {
                let surface = flight::Surface {
                    kind,
                    area,
                    give,
                    span,
                };
                match flight::strike_creature(sim, occupant, hit.point, surface, energy, cause) {
                    Struck::Passed => {}
                    Struck::Hit {
                        part,
                        absorbed,
                        exits,
                    } => {
                        struck += 1;
                        flight::announce_hit(
                            sim,
                            &flight::HitNews {
                                shooter: Some(ray.shooter),
                                projectile: Some(ray.projectile),
                                body: None,
                                target: Some(occupant),
                                here: hit.loc,
                                part,
                                energy,
                                intensity: energy / (area * give),
                                absorbed,
                                passed: exits,
                            },
                            cause,
                        );
                        energy = exits;
                    }
                    Struck::Stopped => {
                        energy = Real::ZERO;
                    }
                }
            }
        }
    }
    struck
}
