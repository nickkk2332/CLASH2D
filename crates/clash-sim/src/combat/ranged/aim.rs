//! Where a shot leaves from and goes (07 §11.1, §11.4): the point a
//! shooter fires from, the point at the height of a part, the elevation
//! that brings a shot down on a point under gravity, and dispersion.
//!
//! Points are in metres in a grid's frame: east, south and up, with
//! `z = storey × 3 + height`. Directions are of length one.

use clash_core::{EntityId, Real, Rng};
use clash_world::Location;
use clash_world::dims::{CELL_M, STOREY_M};

use crate::Sim;
use crate::body::PartKey;
use crate::combat::strike::point_of;

/// The share of its body's height a shooter fires from when the shot
/// says nothing else: about the shoulder of a person.
pub const SHOT_HEIGHT_SHARE: Real = Real::of(0.8);

/// The share of a body's height a shot is aimed at when no part is
/// named: its middle.
pub const BODY_MIDDLE_SHARE: Real = Real::of(0.5);

/// The elevation of the longest shot under gravity: a shot that cannot
/// reach its point is sent this way, as far as it can go.
const FURTHEST_ELEVATION: Real = Real::of(core::f64::consts::FRAC_PI_4);

/// A vector of three.
pub type Vec3 = [Real; 3];

/// The length of a vector.
pub fn norm(v: Vec3) -> Real {
    (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt()
}

/// A vector of length one along `v`, or `None` for one of no length.
pub fn unit(v: Vec3) -> Option<Vec3> {
    let length = norm(v);
    (length > Real::ZERO && length.is_finite()).then(|| scale(v, Real::ONE / length))
}

/// `v` times a number.
pub fn scale(v: Vec3, by: Real) -> Vec3 {
    [v[0] * by, v[1] * by, v[2] * by]
}

/// The sum of two vectors.
pub fn add(a: Vec3, b: Vec3) -> Vec3 {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}

/// `a` less `b`.
pub fn sub(a: Vec3, b: Vec3) -> Vec3 {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
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

/// The distance from a point to the segment between two others.
pub fn distance_to_segment(point: Vec3, from: Vec3, to: Vec3) -> Real {
    let along = sub(to, from);
    let length = dot(along, along);
    let share = if length > Real::ZERO {
        (dot(sub(point, from), along) / length).clamp(Real::ZERO, Real::ONE)
    } else {
        Real::ZERO
    };
    norm(sub(point, add(from, scale(along, share))))
}

/// How tall an entity's body stands, in metres; `None` for one with no
/// body.
pub fn body_height(sim: &Sim, entity: EntityId) -> Option<Real> {
    sim.components()
        .body
        .get(entity)
        .map(crate::body::Body::height)
}

/// Where an entity stands: its cell, and the point at the middle of its
/// cell at its footing. Through whatever it is inside.
pub fn footing(sim: &Sim, entity: EntityId) -> Option<(Location, Vec3)> {
    point_of(sim, entity)
}

/// The point a shooter fires from (07 §11.1): the middle of its cell, at
/// `height` metres above its footing, or at [`SHOT_HEIGHT_SHARE`] of its
/// body's height, or for something with no body at the middle of the
/// storey above its footing.
pub fn shot_origin(sim: &Sim, shooter: EntityId, height: Option<Real>) -> Option<(Location, Vec3)> {
    let (loc, mut point) = footing(sim, shooter)?;
    let rise = height
        .or_else(|| body_height(sim, shooter).map(|tall| tall * SHOT_HEIGHT_SHARE))
        .unwrap_or(STOREY_M * BODY_MIDDLE_SHARE);
    point[2] += rise;
    Some((loc, point))
}

/// The point at the height of a part of an entity, where it stands
/// (07 §11.4, `band_point`): the middle of the part's band; for no part,
/// or a part with no band, the middle of its body; for something with no
/// body, the middle of the storey above its footing. `None` for one
/// that is not in the world.
pub fn band_point(sim: &Sim, target: EntityId, part: Option<PartKey>) -> Option<(Location, Vec3)> {
    let (loc, mut point) = footing(sim, target)?;
    let body = sim.components().body.get(target);
    let band = match (body, part, sim.body_defs()) {
        (Some(body), Some(key), Some(defs)) => {
            body.part(key).and_then(|part| body.part_band(defs, part))
        }
        _ => None,
    };
    let rise = match (band, body) {
        (Some([low, high]), _) => (low + high) * BODY_MIDDLE_SHARE,
        (None, Some(body)) => body.height() * BODY_MIDDLE_SHARE,
        (None, None) => STOREY_M * BODY_MIDDLE_SHARE,
    };
    point[2] += rise;
    Some((loc, point))
}

/// The point at the middle of a cell, `height` metres above its base.
pub fn cell_point(loc: Location, height: Real) -> Vec3 {
    let middle = |at: i32| (Real::from_i64(i64::from(at)) + BODY_MIDDLE_SHARE) * CELL_M;
    [
        middle(loc.cell.x),
        middle(loc.cell.y),
        Real::from_i64(i64::from(loc.cell.z)) * STOREY_M + height,
    ]
}

/// The way from a point in one cell to a point in another, in metres,
/// the shorter way round where the grid wraps. Each point is given in
/// its own cell's frame, as [`cell_point`] gives it. `None` for two
/// grids.
pub fn between(sim: &Sim, from: (Location, Vec3), to: (Location, Vec3)) -> Option<Vec3> {
    let delta = sim.truth().cosmos().delta(from.0, to.0)?;
    let within = |(loc, point): (Location, Vec3)| sub(point, cell_point(loc, Real::ZERO));
    let (a, b) = (within(from), within(to));
    Some([
        Real::from_i64(i64::from(delta.x)) * CELL_M + b[0] - a[0],
        Real::from_i64(i64::from(delta.y)) * CELL_M + b[1] - a[1],
        Real::from_i64(i64::from(delta.z)) * STOREY_M + b[2] - a[2],
    ])
}

/// The direction that brings a shot at `speed` down on a point `to`
/// away, under gravity of `gravity` m/s² straight down, along the lower
/// of the two arcs that reach it, with no drag (07 §11.4,
/// `ballistic_dir`). `None` where no arc reaches it at that speed, or
/// for no way at all. With no gravity it is straight at it.
pub fn ballistic_dir(to: Vec3, speed: Real, gravity: Real) -> Option<Vec3> {
    let level = (to[0] * to[0] + to[1] * to[1]).sqrt();
    if gravity <= Real::ZERO || level <= Real::ZERO {
        return unit(to);
    }
    let v2 = speed * speed;
    let reach = v2 * v2 - gravity * (gravity * level * level + Real::of(2.0) * to[2] * v2);
    if !(speed > Real::ZERO) || reach < Real::ZERO {
        return None;
    }
    let tan = (v2 - reach.sqrt()) / (gravity * level);
    Some(elevated(to, tan.atan2(Real::ONE)))
}

/// The direction of a shot that goes as far as it can toward a point:
/// along its bearing, at the elevation of the longest shot.
pub fn furthest_dir(to: Vec3) -> Option<Vec3> {
    let level = (to[0] * to[0] + to[1] * to[1]).sqrt();
    if level <= Real::ZERO {
        return unit(to);
    }
    Some(elevated(to, FURTHEST_ELEVATION))
}

/// The direction along the bearing of `to` at an elevation, in radians.
fn elevated(to: Vec3, elevation: Real) -> Vec3 {
    let level = (to[0] * to[0] + to[1] * to[1]).sqrt();
    let (cos, sin) = (elevation.cos(), elevation.sin());
    [to[0] / level * cos, to[1] / level * cos, sin]
}

/// One number from a normal distribution with mean zero and spread one,
/// drawn with two numbers of a stream (Box and Muller's way).
fn normal(rng: &mut Rng) -> Real {
    // `unit` is in [0, 1): one less it is in (0, 1], whose logarithm is
    // finite.
    let first = Real::ONE - rng.unit();
    let second = rng.unit();
    (Real::of(-2.0) * first.ln()).sqrt() * (Real::of(2.0) * Real::PI * second).cos()
}

/// A direction turned by dispersion (07 §11.1): across and up by an
/// angle each drawn from a normal distribution whose standard deviation
/// is `spread` radians. Four numbers are drawn from the stream whatever
/// the spread, so that the stream stays in step when content changes
/// it.
pub fn disperse(dir: Vec3, spread: Real, rng: &mut Rng) -> Vec3 {
    let across = normal(rng) * spread;
    let up = normal(rng) * spread;
    if spread <= Real::ZERO {
        return dir;
    }
    // A frame about the direction: one way across it on the level, and
    // the way square to both.
    let level = cross(dir, [Real::ZERO, Real::ZERO, Real::ONE]);
    let side = unit(level).unwrap_or([Real::ONE, Real::ZERO, Real::ZERO]);
    let lift = unit(cross(side, dir)).unwrap_or([Real::ZERO, Real::ZERO, Real::ONE]);
    let turned = add(dir, add(scale(side, across.tan()), scale(lift, up.tan())));
    unit(turned).unwrap_or(dir)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn r(value: f64) -> Real {
        Real::of(value)
    }

    #[test]
    fn a_ballistic_arc_comes_down_on_its_point() {
        // Fly the arc in closed form: it reaches the point's distance at
        // the point's height.
        let to = [r(60.0), r(0.0), r(-1.0)];
        let speed = r(40.0);
        let g = r(9.81);
        let dir = ballistic_dir(to, speed, g).unwrap();
        let vx = dir[0] * speed;
        let vz = dir[2] * speed;
        let time = to[0] / vx;
        let z = vz * time - r(0.5) * g * time * time;
        assert!((z.get() - to[2].get()).abs() < 1e-6, "{z}");
        // Too far for the speed: no arc reaches it.
        assert!(ballistic_dir([r(500.0), r(0.0), r(0.0)], speed, g).is_none());
        // With no gravity it is straight at it.
        let straight = ballistic_dir([r(3.0), r(4.0), r(0.0)], speed, Real::ZERO).unwrap();
        assert!((straight[0].get() - 0.6).abs() < 1e-12, "{straight:?}");
    }

    #[test]
    fn dispersion_scatters_by_its_spread_and_none_leaves_the_direction() {
        let dir = [r(1.0), r(0.0), r(0.0)];
        let mut rng = Rng::from_seed(5);
        assert_eq!(disperse(dir, Real::ZERO, &mut rng), dir);
        let spread = 0.01;
        let mut sum = 0.0;
        let draws = 4000;
        for _ in 0..draws {
            let turned = disperse(dir, r(spread), &mut rng);
            let across = turned[1].get();
            sum += across * across;
        }
        let deviation = (sum / f64::from(draws)).sqrt();
        assert!((deviation - spread).abs() < spread * 0.1, "{deviation}");
    }

    #[test]
    fn a_point_on_a_segment_is_at_no_distance_from_it() {
        let from = [r(0.0), r(0.0), r(0.0)];
        let to = [r(10.0), r(0.0), r(0.0)];
        assert_eq!(
            distance_to_segment([r(5.0), r(0.0), r(0.0)], from, to),
            r(0.0)
        );
        assert_eq!(
            distance_to_segment([r(5.0), r(2.0), r(0.0)], from, to),
            r(2.0)
        );
        assert_eq!(
            distance_to_segment([r(13.0), r(4.0), r(0.0)], from, to),
            r(5.0)
        );
    }
}
