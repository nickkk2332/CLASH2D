//! Behaviour of `clash_gen::noise`: ranges, continuity, independence and
//! the descriptions read from data. The exact bits are pinned in
//! `noise_reference.rs`.

use clash_core::{HashDomain, Real, Rng, pos_hash};
use clash_gen::noise::{
    Cellular, DEFAULT_GAIN, DEFAULT_LACUNARITY, DEFAULT_RANGE, Extent, FieldSpec, Fractal,
    MAX_OCTAVES, NoiseSpec, NoiseSpecError, Shape, billow_2d, billow_3d, cellular_2d, cellular_3d,
    fbm_2d, fbm_3d, gradient_2d, gradient_3d, ridged_2d, ridged_3d, value_2d, value_3d, warp_2d,
    warp_3d,
};

const HEIGHT: HashDomain = HashDomain::new("test.height");
const MOISTURE: HashDomain = HashDomain::new("test.moisture");
const SEED: u64 = 0x5eed;

/// How many positions the wide sampling tests try.
const SAMPLES: usize = 40_000;

fn r(value: f64) -> Real {
    Real::of(value)
}

/// A position somewhere within a thousand cells of the origin.
fn position(rng: &mut Rng) -> [Real; 3] {
    let mut part = || (rng.unit() - r(0.5)) * r(2000.0);
    [part(), part(), part()]
}

/// The smallest, largest and mean of a function over many positions.
fn survey(mut function: impl FnMut([Real; 3]) -> Real) -> (Real, Real, Real) {
    let mut rng = Rng::from_seed(99);
    let mut low = Real::INFINITY;
    let mut high = Real::NEG_INFINITY;
    let mut total = Real::ZERO;
    for _ in 0..SAMPLES {
        let value = function(position(&mut rng));
        low = low.min(value);
        high = high.max(value);
        total += value;
    }
    (low, high, total / r(SAMPLES as f64))
}

/// Pearson's correlation of two functions over many positions.
fn correlation(mut a: impl FnMut([Real; 3]) -> Real, mut b: impl FnMut([Real; 3]) -> Real) -> f64 {
    let mut rng = Rng::from_seed(7);
    let pairs: Vec<(f64, f64)> = (0..SAMPLES)
        .map(|_| {
            let at = position(&mut rng);
            (a(at).get(), b(at).get())
        })
        .collect();
    let count = pairs.len() as f64;
    let mean_a = pairs.iter().map(|pair| pair.0).sum::<f64>() / count;
    let mean_b = pairs.iter().map(|pair| pair.1).sum::<f64>() / count;
    let mut covariance = 0.0;
    let mut spread_a = 0.0;
    let mut spread_b = 0.0;
    for (value_a, value_b) in pairs {
        covariance += (value_a - mean_a) * (value_b - mean_b);
        spread_a += (value_a - mean_a) * (value_a - mean_a);
        spread_b += (value_b - mean_b) * (value_b - mean_b);
    }
    covariance / (spread_a * spread_b).sqrt()
}

// ---- The same inputs give the same outputs ---------------------------------

#[test]
fn every_function_is_a_pure_function_of_its_inputs() {
    let fractal = Fractal::new(4);
    let mut rng = Rng::from_seed(3);
    for _ in 0..200 {
        let [x, y, z] = position(&mut rng);
        assert_eq!(value_2d(SEED, HEIGHT, x, y), value_2d(SEED, HEIGHT, x, y));
        assert_eq!(
            value_3d(SEED, HEIGHT, x, y, z),
            value_3d(SEED, HEIGHT, x, y, z)
        );
        assert_eq!(
            gradient_2d(SEED, HEIGHT, x, y),
            gradient_2d(SEED, HEIGHT, x, y)
        );
        assert_eq!(
            gradient_3d(SEED, HEIGHT, x, y, z),
            gradient_3d(SEED, HEIGHT, x, y, z)
        );
        assert_eq!(
            fbm_2d(SEED, HEIGHT, fractal, x, y),
            fbm_2d(SEED, HEIGHT, fractal, x, y)
        );
        assert_eq!(
            ridged_3d(SEED, HEIGHT, fractal, x, y, z),
            ridged_3d(SEED, HEIGHT, fractal, x, y, z)
        );
        assert_eq!(
            cellular_2d(SEED, HEIGHT, x, y),
            cellular_2d(SEED, HEIGHT, x, y)
        );
        assert_eq!(
            cellular_3d(SEED, HEIGHT, x, y, z),
            cellular_3d(SEED, HEIGHT, x, y, z)
        );
        assert_eq!(
            warp_3d(SEED, HEIGHT, fractal, r(0.5), x, y, z),
            warp_3d(SEED, HEIGHT, fractal, r(0.5), x, y, z)
        );
    }
}

#[test]
fn the_order_of_sampling_does_not_matter() {
    let points: Vec<(Real, Real)> = (0..64)
        .map(|index| (r(f64::from(index) * 0.37), r(f64::from(index) * -1.13)))
        .collect();
    let sample = |&(x, y): &(Real, Real)| fbm_2d(SEED, HEIGHT, Fractal::new(5), x, y);
    let forward: Vec<Real> = points.iter().map(sample).collect();
    let mut backward: Vec<Real> = points.iter().rev().map(sample).collect();
    backward.reverse();
    assert_eq!(forward, backward);
}

// ---- What the lattice is ------------------------------------------------------

#[test]
fn value_noise_at_a_whole_position_is_that_position_s_value() {
    // The documented definition, written out again here: the upper 53
    // bits of the positional hash, with the salt for value noise.
    const VALUE_SALT: u64 = 1 << 32;
    for (x, y, z) in [(0, 0, 0), (3, -7, 0), (-250, 1000, 12), (1, 1, -1)] {
        let hash = pos_hash(SEED, HEIGHT, x, y, z, VALUE_SALT);
        let expected = r((hash >> 11) as f64 / 9_007_199_254_740_992.0);
        let (fx, fy, fz) = (Real::from(x), Real::from(y), Real::from(z));
        assert_eq!(value_3d(SEED, HEIGHT, fx, fy, fz), expected);
        if z == 0 {
            assert_eq!(value_2d(SEED, HEIGHT, fx, fy), expected);
        }
    }
    // The plane is the slice of space at z = 0.
    let mut rng = Rng::from_seed(12);
    for _ in 0..200 {
        let [x, y, _] = position(&mut rng);
        assert_eq!(
            value_2d(SEED, HEIGHT, x, y),
            value_3d(SEED, HEIGHT, x, y, Real::ZERO)
        );
    }
}

#[test]
fn gradient_noise_is_zero_at_every_whole_position() {
    for x in -20..20 {
        for y in -20..20 {
            let (fx, fy) = (Real::from(x), Real::from(y));
            assert_eq!(gradient_2d(SEED, HEIGHT, fx, fy), Real::ZERO);
            assert_eq!(
                gradient_3d(SEED, HEIGHT, fx, fy, Real::from(x - y)),
                Real::ZERO
            );
        }
    }
}

// ---- Ranges -------------------------------------------------------------------

#[test]
fn value_noise_stays_between_zero_and_one_and_uses_the_whole_range() {
    let (low, high, mean) = survey(|[x, y, _]| value_2d(SEED, HEIGHT, x, y));
    assert!(low >= Real::ZERO && high <= Real::ONE, "{low} to {high}");
    assert!(low < r(0.02) && high > r(0.98), "{low} to {high}");
    assert!((mean - r(0.5)).abs() < r(0.02), "mean {mean}");

    let (low, high, mean) = survey(|[x, y, z]| value_3d(SEED, HEIGHT, x, y, z));
    assert!(low >= Real::ZERO && high <= Real::ONE, "{low} to {high}");
    assert!(low < r(0.05) && high > r(0.95), "{low} to {high}");
    assert!((mean - r(0.5)).abs() < r(0.02), "mean {mean}");
}

#[test]
fn gradient_noise_stays_between_minus_one_and_one_and_reaches_far_into_it() {
    let (low, high, mean) = survey(|[x, y, _]| gradient_2d(SEED, HEIGHT, x, y));
    assert!(low >= r(-1.0) && high <= Real::ONE, "{low} to {high}");
    assert!(low < r(-0.75) && high > r(0.75), "{low} to {high}");
    assert!(mean.abs() < r(0.02), "mean {mean}");

    let (low, high, mean) = survey(|[x, y, z]| gradient_3d(SEED, HEIGHT, x, y, z));
    assert!(low >= r(-1.0) && high <= Real::ONE, "{low} to {high}");
    assert!(low < r(-0.6) && high > r(0.6), "{low} to {high}");
    assert!(mean.abs() < r(0.02), "mean {mean}");
}

#[test]
fn fractal_sums_keep_the_range_of_one_octave() {
    let fractals = [
        Fractal::new(1),
        Fractal::new(5),
        Fractal::new(MAX_OCTAVES),
        Fractal {
            octaves: 6,
            lacunarity: r(3.1),
            gain: r(0.9),
        },
        Fractal {
            octaves: 4,
            lacunarity: Real::ONE,
            gain: Real::ONE,
        },
    ];
    for fractal in fractals {
        let (low, high, mean) = survey(|[x, y, _]| fbm_2d(SEED, HEIGHT, fractal, x, y));
        assert!(
            low >= r(-1.0) && high <= Real::ONE,
            "{fractal:?}: {low} to {high}"
        );
        assert!(
            low < r(-0.3) && high > r(0.3),
            "{fractal:?}: {low} to {high}"
        );
        assert!(mean.abs() < r(0.02), "{fractal:?}: mean {mean}");

        let (low, high, _) = survey(|[x, y, z]| fbm_3d(SEED, HEIGHT, fractal, x, y, z));
        assert!(
            low >= r(-1.0) && high <= Real::ONE,
            "{fractal:?}: {low} to {high}"
        );
        assert!(
            low < r(-0.25) && high > r(0.25),
            "{fractal:?}: {low} to {high}"
        );

        for (name, low, high, _) in [
            {
                let (low, high, mean) = survey(|[x, y, _]| ridged_2d(SEED, HEIGHT, fractal, x, y));
                ("ridged_2d", low, high, mean)
            },
            {
                let (low, high, mean) =
                    survey(|[x, y, z]| ridged_3d(SEED, HEIGHT, fractal, x, y, z));
                ("ridged_3d", low, high, mean)
            },
            {
                let (low, high, mean) = survey(|[x, y, _]| billow_2d(SEED, HEIGHT, fractal, x, y));
                ("billow_2d", low, high, mean)
            },
            {
                let (low, high, mean) =
                    survey(|[x, y, z]| billow_3d(SEED, HEIGHT, fractal, x, y, z));
                ("billow_3d", low, high, mean)
            },
        ] {
            assert!(
                low >= Real::ZERO && high <= Real::ONE,
                "{name} {fractal:?}: {low} to {high}"
            );
            assert!(high - low > r(0.3), "{name} {fractal:?}: {low} to {high}");
        }
    }
}

#[test]
fn ridges_are_high_where_the_noise_crosses_zero_and_billows_are_low() {
    // At a whole position every octave of the sum is zero.
    let fractal = Fractal::new(3);
    let (x, y) = (r(4.0), r(-9.0));
    assert_eq!(fbm_2d(SEED, HEIGHT, fractal, x, y), Real::ZERO);
    assert_eq!(ridged_2d(SEED, HEIGHT, fractal, x, y), Real::ONE);
    assert_eq!(billow_2d(SEED, HEIGHT, fractal, x, y), Real::ZERO);
    // One octave of each is the fold of plain gradient noise.
    let one = Fractal::new(1);
    let (x, y) = (r(4.3), r(-9.6));
    let plain = gradient_2d(SEED, HEIGHT, x, y);
    assert_eq!(fbm_2d(SEED, HEIGHT, one, x, y), plain);
    assert_eq!(billow_2d(SEED, HEIGHT, one, x, y), plain.abs());
    let crest = Real::ONE - plain.abs();
    assert_eq!(ridged_2d(SEED, HEIGHT, one, x, y), crest * crest);
}

#[test]
fn the_number_of_octaves_is_limited() {
    let (x, y) = (r(17.3), r(2.9));
    let most = fbm_2d(SEED, HEIGHT, Fractal::new(MAX_OCTAVES), x, y);
    assert_eq!(fbm_2d(SEED, HEIGHT, Fractal::new(u32::MAX), x, y), most);
    assert_eq!(
        fbm_2d(SEED, HEIGHT, Fractal::new(MAX_OCTAVES + 1), x, y),
        most
    );
    // No octaves at all counts as one.
    assert_eq!(
        fbm_2d(SEED, HEIGHT, Fractal::new(0), x, y),
        fbm_2d(SEED, HEIGHT, Fractal::new(1), x, y)
    );
    assert_ne!(fbm_2d(SEED, HEIGHT, Fractal::new(2), x, y), most);
}

#[test]
fn cellular_distances_are_ordered_and_bounded() {
    let mut rng = Rng::from_seed(21);
    let mut farthest = Real::ZERO;
    let mut cells = Vec::new();
    for _ in 0..SAMPLES {
        let [x, y, z] = position(&mut rng);
        let plane = cellular_2d(SEED, HEIGHT, x, y);
        assert!(plane.nearest >= Real::ZERO && plane.nearest <= plane.second);
        // √2 and √(5⁄2): the far corner of a cell, and the far corner of
        // a neighbour seen from the middle.
        assert!(plane.nearest <= r(2.0).sqrt(), "{plane:?}");
        assert!(plane.second <= r(2.5).sqrt(), "{plane:?}");

        let space = cellular_3d(SEED, HEIGHT, x, y, z);
        assert!(space.nearest >= Real::ZERO && space.nearest <= space.second);
        // √3: the far corner of a cube.
        assert!(space.second <= r(3.0).sqrt(), "{space:?}");
        farthest = farthest.max(space.second).max(plane.second);
        cells.push(plane.cell);
    }
    assert!(farthest > Real::ONE && farthest < r(2.0));
    // Positions a thousand cells apart are in different patches.
    let total = cells.len();
    cells.sort_unstable();
    cells.dedup();
    assert!(cells.len() > total * 9 / 10);
}

#[test]
fn a_patch_has_one_name_and_its_point_is_at_distance_zero() {
    // Walk a line and note where the name changes: at each change the two
    // nearest points are equally near.
    let mut last: Option<Cellular> = None;
    let mut changes = 0;
    for step in 0..20_000 {
        let x = r(f64::from(step) * 0.001);
        let here = cellular_2d(SEED, HEIGHT, x, r(0.4));
        if let Some(before) = last
            && before.cell != here.cell
        {
            changes += 1;
            assert!(here.second - here.nearest < r(0.002), "{here:?}");
        }
        last = Some(here);
    }
    // Twenty cells walked: somewhere near twenty patches crossed.
    assert!((10..40).contains(&changes), "{changes} changes");
}

#[test]
fn warping_moves_a_position_by_at_most_its_strength() {
    let fractal = Fractal::new(3);
    let strength = r(2.5);
    let mut rng = Rng::from_seed(31);
    let mut furthest = Real::ZERO;
    for _ in 0..5000 {
        let [x, y, z] = position(&mut rng);
        let (wx, wy) = warp_2d(SEED, HEIGHT, fractal, strength, x, y);
        assert!((wx - x).abs() <= strength && (wy - y).abs() <= strength);
        let (sx, sy, sz) = warp_3d(SEED, HEIGHT, fractal, strength, x, y, z);
        for moved in [sx - x, sy - y, sz - z] {
            assert!(moved.abs() <= strength);
            furthest = furthest.max(moved.abs());
        }
        // No strength, no movement.
        assert_eq!(warp_2d(SEED, HEIGHT, fractal, Real::ZERO, x, y), (x, y));
    }
    assert!(furthest > r(0.5));
    // The axes are pushed independently.
    let along_x = correlation(
        |[x, y, _]| warp_2d(SEED, HEIGHT, fractal, Real::ONE, x, y).0 - x,
        |[x, y, _]| warp_2d(SEED, HEIGHT, fractal, Real::ONE, x, y).1 - y,
    );
    assert!(along_x.abs() < 0.05, "correlation {along_x}");
}

// ---- Continuity ------------------------------------------------------------------

/// The largest change of a function over a small step, as a multiple of
/// the step, over many positions and directions.
fn steepest(step: f64, mut function: impl FnMut([Real; 3]) -> Real) -> f64 {
    let mut rng = Rng::from_seed(55);
    let mut steepest = 0.0_f64;
    for _ in 0..SAMPLES / 4 {
        let at = position(&mut rng);
        let toward = [rng.unit(), rng.unit(), rng.unit()];
        let length = (toward[0] * toward[0] + toward[1] * toward[1] + toward[2] * toward[2]).sqrt();
        let scale = r(step) / length.max(r(1e-9));
        let next = [
            at[0] + toward[0] * scale,
            at[1] + toward[1] * scale,
            at[2] + toward[2] * scale,
        ];
        let change = (function(next) - function(at)).abs().get();
        steepest = steepest.max(change / step);
    }
    steepest
}

#[test]
fn neighbouring_samples_are_close() {
    const STEP: f64 = 1.0 / 4096.0;
    // Each bound is the steepest the function can be, with room to spare.
    let slope = steepest(STEP, |[x, y, _]| value_2d(SEED, HEIGHT, x, y));
    assert!(slope < 3.0, "value_2d changes by {slope} per unit");
    let slope = steepest(STEP, |[x, y, z]| value_3d(SEED, HEIGHT, x, y, z));
    assert!(slope < 4.0, "value_3d changes by {slope} per unit");
    let slope = steepest(STEP, |[x, y, _]| gradient_2d(SEED, HEIGHT, x, y));
    assert!(slope < 6.0, "gradient_2d changes by {slope} per unit");
    let slope = steepest(STEP, |[x, y, z]| gradient_3d(SEED, HEIGHT, x, y, z));
    assert!(slope < 6.0, "gradient_3d changes by {slope} per unit");

    // With a lacunarity of 2 and a gain of one half every octave is as
    // steep as the first, and the sum is divided by nearly 2.
    let fractal = Fractal::new(5);
    let slope = steepest(STEP, |[x, y, _]| fbm_2d(SEED, HEIGHT, fractal, x, y));
    assert!(slope < 12.0, "fbm_2d changes by {slope} per unit");
    let slope = steepest(STEP, |[x, y, z]| fbm_3d(SEED, HEIGHT, fractal, x, y, z));
    assert!(slope < 12.0, "fbm_3d changes by {slope} per unit");
    let slope = steepest(STEP, |[x, y, _]| ridged_2d(SEED, HEIGHT, fractal, x, y));
    assert!(slope < 24.0, "ridged_2d changes by {slope} per unit");
    let slope = steepest(STEP, |[x, y, _]| billow_2d(SEED, HEIGHT, fractal, x, y));
    assert!(slope < 12.0, "billow_2d changes by {slope} per unit");
}

#[test]
fn there_is_no_seam_where_lattice_cells_meet() {
    // Either side of a lattice line, a hair apart.
    const HAIR: f64 = 1e-9;
    const CLOSE: f64 = 1e-7;
    let fractal = Fractal::new(4);
    let mut rng = Rng::from_seed(77);
    for _ in 0..2000 {
        let line = Real::from(rng.range(-500, 500) as i32);
        let [_, y, z] = position(&mut rng);
        let (before, after) = (line - r(HAIR), line + r(HAIR));
        let pairs = [
            (
                value_2d(SEED, HEIGHT, before, y),
                value_2d(SEED, HEIGHT, after, y),
            ),
            (
                value_2d(SEED, HEIGHT, y, before),
                value_2d(SEED, HEIGHT, y, after),
            ),
            (
                value_3d(SEED, HEIGHT, y, z, before),
                value_3d(SEED, HEIGHT, y, z, after),
            ),
            (
                gradient_2d(SEED, HEIGHT, before, y),
                gradient_2d(SEED, HEIGHT, after, y),
            ),
            (
                gradient_3d(SEED, HEIGHT, y, before, z),
                gradient_3d(SEED, HEIGHT, y, after, z),
            ),
            (
                fbm_2d(SEED, HEIGHT, fractal, before, y),
                fbm_2d(SEED, HEIGHT, fractal, after, y),
            ),
            (
                ridged_3d(SEED, HEIGHT, fractal, before, y, z),
                ridged_3d(SEED, HEIGHT, fractal, after, y, z),
            ),
        ];
        for (index, (a, b)) in pairs.into_iter().enumerate() {
            assert!(
                (a - b).abs() < r(CLOSE),
                "pair {index} at {line}: {a} and {b}"
            );
        }
    }
}

#[test]
fn cellular_distances_never_change_faster_than_the_position() {
    // A distance to a point cannot change by more than the step taken.
    // This holds only if the two nearest points are found exactly.
    const STEP: f64 = 1.0 / 64.0;
    let slack = 1.0 + 1e-9;
    let slope = steepest(STEP, |[x, y, _]| cellular_2d(SEED, HEIGHT, x, y).nearest);
    assert!(slope <= slack, "nearest in the plane: {slope}");
    let slope = steepest(STEP, |[x, y, _]| cellular_2d(SEED, HEIGHT, x, y).second);
    assert!(slope <= slack, "second in the plane: {slope}");
    let slope = steepest(STEP, |[x, y, z]| cellular_3d(SEED, HEIGHT, x, y, z).nearest);
    assert!(slope <= slack, "nearest in space: {slope}");
    let slope = steepest(STEP, |[x, y, z]| cellular_3d(SEED, HEIGHT, x, y, z).second);
    assert!(slope <= slack, "second in space: {slope}");
}

// ---- Independence ------------------------------------------------------------------

#[test]
fn different_domains_and_seeds_are_unrelated() {
    let fractal = Fractal::new(4);
    type Sampler = fn(u64, HashDomain, [Real; 3]) -> Real;
    let samplers: [(&str, Sampler); 7] = [
        ("value_2d", |seed, domain, [x, y, _]| {
            value_2d(seed, domain, x, y)
        }),
        ("value_3d", |seed, domain, [x, y, z]| {
            value_3d(seed, domain, x, y, z)
        }),
        ("gradient_2d", |seed, domain, [x, y, _]| {
            gradient_2d(seed, domain, x, y)
        }),
        ("gradient_3d", |seed, domain, [x, y, z]| {
            gradient_3d(seed, domain, x, y, z)
        }),
        ("fbm_2d", |seed, domain, [x, y, _]| {
            fbm_2d(seed, domain, Fractal::new(4), x, y)
        }),
        ("cellular_2d", |seed, domain, [x, y, _]| {
            cellular_2d(seed, domain, x, y).nearest
        }),
        ("cellular_3d", |seed, domain, [x, y, z]| {
            cellular_3d(seed, domain, x, y, z).second
        }),
    ];
    for (name, sampler) in samplers {
        // With itself a function is perfectly related, which shows the
        // measure works.
        let same = correlation(
            |at| sampler(SEED, HEIGHT, at),
            |at| sampler(SEED, HEIGHT, at),
        );
        assert!(same > 0.999, "{name}: {same}");
        let domains = correlation(
            |at| sampler(SEED, HEIGHT, at),
            |at| sampler(SEED, MOISTURE, at),
        );
        assert!(domains.abs() < 0.03, "{name} across domains: {domains}");
        let seeds = correlation(
            |at| sampler(SEED, HEIGHT, at),
            |at| sampler(SEED + 1, HEIGHT, at),
        );
        assert!(seeds.abs() < 0.03, "{name} across seeds: {seeds}");
    }

    // Within one domain the kinds of noise are unrelated, and so are the
    // octaves of a sum: one octave says nothing about the next.
    let kinds = correlation(
        |[x, y, _]| value_2d(SEED, HEIGHT, x, y),
        |[x, y, _]| gradient_2d(SEED, HEIGHT, x, y),
    );
    assert!(kinds.abs() < 0.03, "value against gradient: {kinds}");
    let octaves = correlation(
        |[x, y, _]| fbm_2d(SEED, HEIGHT, Fractal::new(1), x, y),
        |[x, y, _]| {
            let two = fbm_2d(SEED, HEIGHT, Fractal::new(2), x, y);
            let one = fbm_2d(SEED, HEIGHT, Fractal::new(1), x, y);
            // The second octave alone: the sum of two is (a + b/2) / 1.5.
            two * r(1.5) - one
        },
    );
    assert!(
        octaves.abs() < 0.03,
        "first octave against second: {octaves}"
    );
    let _ = fractal;
}

#[test]
fn nearby_positions_are_related_and_distant_ones_are_not() {
    let near = correlation(
        |[x, y, _]| value_2d(SEED, HEIGHT, x, y),
        |[x, y, _]| value_2d(SEED, HEIGHT, x + r(0.1), y),
    );
    assert!(near > 0.9, "a tenth of a cell apart: {near}");
    let far = correlation(
        |[x, y, _]| value_2d(SEED, HEIGHT, x, y),
        |[x, y, _]| value_2d(SEED, HEIGHT, x + r(3.0), y),
    );
    assert!(far.abs() < 0.03, "three cells apart: {far}");
}

// ---- Positions no world has ---------------------------------------------------------

#[test]
fn any_position_gives_a_result_in_range() {
    let fractal = Fractal {
        octaves: 8,
        lacunarity: r(4.0),
        gain: r(0.5),
    };
    let extremes = [
        0.0,
        -0.0,
        1e-300,
        4_294_967_295.999,
        -4_294_967_296.0,
        9.007_199_254_740_992e15,
        1e300,
        -1e300,
        f64::MAX,
        f64::INFINITY,
        f64::NEG_INFINITY,
    ];
    for x in extremes {
        for y in extremes {
            let (x, y) = (r(x), r(y));
            let unit = |value: Real| value >= Real::ZERO && value <= Real::ONE;
            let signed = |value: Real| value >= r(-1.0) && value <= Real::ONE;
            assert!(unit(value_2d(SEED, HEIGHT, x, y)));
            assert!(unit(value_3d(SEED, HEIGHT, x, y, x)));
            assert!(signed(gradient_2d(SEED, HEIGHT, x, y)));
            assert!(signed(gradient_3d(SEED, HEIGHT, y, x, y)));
            assert!(signed(fbm_2d(SEED, HEIGHT, fractal, x, y)));
            assert!(signed(fbm_3d(SEED, HEIGHT, fractal, x, y, x)));
            assert!(unit(ridged_2d(SEED, HEIGHT, fractal, x, y)));
            assert!(unit(billow_3d(SEED, HEIGHT, fractal, x, y, y)));
            let found = cellular_2d(SEED, HEIGHT, x, y);
            assert!(found.nearest <= found.second && found.second < r(2.0));
            let found = cellular_3d(SEED, HEIGHT, x, y, x);
            assert!(found.nearest <= found.second && found.second < r(2.0));
            let _ = warp_2d(SEED, HEIGHT, fractal, r(1e300), x, y);
        }
    }
    // Fractals no author should write still give an answer in range.
    let odd = [
        (0, 0.0, 0.0),
        (9, -2.0, -0.5),
        (9, 1e300, 1e300),
        (9, f64::INFINITY, f64::INFINITY),
        (9, 0.5, 4.0),
    ];
    for (octaves, lacunarity, gain) in odd {
        let fractal = Fractal {
            octaves,
            lacunarity: r(lacunarity),
            gain: r(gain),
        };
        let value = fbm_2d(SEED, HEIGHT, fractal, r(3.3), r(4.4));
        assert!(
            value >= r(-1.0) && value <= Real::ONE,
            "{fractal:?}: {value}"
        );
        let value = ridged_3d(SEED, HEIGHT, fractal, r(3.3), r(4.4), r(5.5));
        assert!(
            value >= Real::ZERO && value <= Real::ONE,
            "{fractal:?}: {value}"
        );
    }
}

// ---- Descriptions from data -----------------------------------------------------------

/// The fields of the `temperate_island` example of 10 §9.1.
const ISLAND_HEIGHT: &str = r#"{ "noise": { "octaves": 5, "scale": 90 },
                                 "shape": "island", "range": [-12, 30] }"#;
const ISLAND_MOISTURE: &str = r#"{ "noise": { "octaves": 3, "scale": 40 } }"#;

/// A realm 512 tracts square, in tracts.
fn realm() -> Extent {
    Extent::new(Real::ZERO, Real::ZERO, r(512.0), r(512.0))
}

fn field(json: &str) -> FieldSpec {
    let spec: FieldSpec = serde_json::from_str(json).expect("the description is read");
    spec.validate().expect("the description is sound");
    spec
}

#[test]
fn the_examples_of_document_10_are_read() {
    let height = field(ISLAND_HEIGHT);
    assert_eq!(
        height,
        FieldSpec {
            noise: NoiseSpec {
                octaves: 5,
                scale: r(90.0),
                lacunarity: DEFAULT_LACUNARITY,
                gain: DEFAULT_GAIN,
            },
            shape: Shape::Island,
            range: [r(-12.0), r(30.0)],
            extent: None,
        }
    );
    let moisture = field(ISLAND_MOISTURE);
    assert_eq!(moisture.shape, Shape::Flat);
    assert_eq!(moisture.range, DEFAULT_RANGE);
    assert_eq!(moisture.noise.fractal(), Fractal::new(3));

    let full = field(
        r#"{ "noise": { "octaves": 2, "scale": 7.5, "lacunarity": 3, "gain": 0.25 },
             "shape": "ridged", "range": [100, 200] }"#,
    );
    assert_eq!(full.noise.lacunarity, r(3.0));
    assert_eq!(full.noise.gain, r(0.25));
    for (name, shape) in [
        ("island", Shape::Island),
        ("continent", Shape::Continent),
        ("flat", Shape::Flat),
        ("ridged", Shape::Ridged),
    ] {
        let json = format!(r#"{{ "noise": {{ "octaves": 1, "scale": 1 }}, "shape": "{name}" }}"#);
        assert_eq!(field(&json).shape, shape);
        assert_eq!(
            shape.needs_extent(),
            matches!(shape, Shape::Island | Shape::Continent)
        );
    }
}

#[test]
fn a_description_with_a_mistake_is_refused() {
    let refused = |json: &str| {
        serde_json::from_str::<FieldSpec>(json)
            .expect_err("the description is refused")
            .to_string()
    };
    // Keys that do not exist, at either level.
    assert!(
        refused(r#"{ "noise": { "octaves": 1, "scale": 1 }, "shpae": "island" }"#)
            .contains("unknown field `shpae`")
    );
    assert!(
        refused(r#"{ "noise": { "octaves": 1, "scale": 1, "octave": 2 } }"#)
            .contains("unknown field `octave`")
    );
    assert!(
        refused(r#"{ "noise": { "octaves": 1, "scale": 1 }, "extent": [0, 0, 1, 1] }"#)
            .contains("unknown field `extent`")
    );
    // Keys that must be there.
    assert!(refused(r#"{ "shape": "island" }"#).contains("missing field `noise`"));
    assert!(refused(r#"{ "noise": { "octaves": 1 } }"#).contains("missing field `scale`"));
    assert!(refused(r#"{ "noise": { "scale": 1 } }"#).contains("missing field `octaves`"));
    // Values of the wrong kind.
    assert!(
        refused(r#"{ "noise": { "octaves": 1, "scale": 1 }, "shape": "volcano" }"#)
            .contains("unknown variant `volcano`")
    );
    assert!(!refused(r#"{ "noise": { "octaves": -1, "scale": 1 } }"#).is_empty());
    assert!(!refused(r#"{ "noise": { "octaves": 1, "scale": 1 }, "range": [1] }"#).is_empty());
    assert!(
        !refused(r#"{ "noise": { "octaves": 1, "scale": 1 }, "range": [1, 2, 3] }"#).is_empty()
    );
}

#[test]
fn numbers_that_make_no_sense_are_reported_with_a_reason() {
    let base = field(ISLAND_HEIGHT);
    let with = |change: &dyn Fn(&mut FieldSpec)| {
        let mut spec = base.clone();
        change(&mut spec);
        spec.validate()
    };
    assert_eq!(with(&|_| {}), Ok(()));
    assert_eq!(
        with(&|spec| spec.noise.octaves = 0),
        Err(NoiseSpecError::Octaves(0))
    );
    assert_eq!(
        with(&|spec| spec.noise.octaves = MAX_OCTAVES + 1),
        Err(NoiseSpecError::Octaves(MAX_OCTAVES + 1))
    );
    assert_eq!(with(&|spec| spec.noise.octaves = MAX_OCTAVES), Ok(()));
    for scale in [0.0, -90.0, f64::INFINITY] {
        assert_eq!(
            with(&|spec| spec.noise.scale = r(scale)),
            Err(NoiseSpecError::Scale(r(scale)))
        );
    }
    for lacunarity in [0.5, -2.0, f64::INFINITY] {
        assert_eq!(
            with(&|spec| spec.noise.lacunarity = r(lacunarity)),
            Err(NoiseSpecError::Lacunarity(r(lacunarity)))
        );
    }
    assert_eq!(with(&|spec| spec.noise.lacunarity = Real::ONE), Ok(()));
    for gain in [0.0, -0.5, 1.5] {
        assert_eq!(
            with(&|spec| spec.noise.gain = r(gain)),
            Err(NoiseSpecError::Gain(r(gain)))
        );
    }
    assert_eq!(with(&|spec| spec.noise.gain = Real::ONE), Ok(()));
    assert_eq!(
        with(&|spec| spec.range = [r(30.0), r(-12.0)]),
        Err(NoiseSpecError::Range {
            low: r(30.0),
            high: r(-12.0)
        })
    );
    assert_eq!(
        with(&|spec| spec.range = [Real::ZERO, Real::INFINITY]),
        Err(NoiseSpecError::Range {
            low: Real::ZERO,
            high: Real::INFINITY
        })
    );
    assert_eq!(with(&|spec| spec.range = [r(5.0), r(5.0)]), Ok(()));

    assert_eq!(
        NoiseSpecError::Octaves(40).to_string(),
        "`octaves` is 40, but must be from 1 to 16"
    );
    assert_eq!(
        NoiseSpecError::Scale(r(-90.0)).to_string(),
        "`scale` is -90, but must be a finite number greater than zero"
    );
    assert_eq!(
        NoiseSpecError::Range {
            low: r(30.0),
            high: r(-12.0)
        }
        .to_string(),
        "`range` is [30, -12], but must be two finite numbers, the lower first"
    );
}

#[test]
fn a_flat_field_is_its_noise_stretched_over_its_range() {
    let moisture = field(ISLAND_MOISTURE);
    let scaled = field(r#"{ "noise": { "octaves": 3, "scale": 40 }, "range": [-50, 150] }"#);
    let (low, high, mean) = survey(|[x, y, _]| moisture.sample(SEED, MOISTURE, x, y));
    assert!(low >= Real::ZERO && high <= Real::ONE, "{low} to {high}");
    assert!(low < r(0.3) && high > r(0.7), "{low} to {high}");
    assert!((mean - r(0.5)).abs() < r(0.02), "mean {mean}");

    let mut rng = Rng::from_seed(8);
    for _ in 0..500 {
        let [x, y, _] = position(&mut rng);
        let unit = moisture.sample(SEED, MOISTURE, x, y);
        // The description's noise is the fractal sum at the position
        // divided by the scale.
        let sum = fbm_2d(SEED, MOISTURE, Fractal::new(3), x / r(40.0), y / r(40.0));
        assert_eq!(unit, r(0.5) + r(0.5) * sum);
        assert_eq!(unit, moisture.noise.sample(SEED, MOISTURE, x, y));
        assert_eq!(
            scaled.sample(SEED, MOISTURE, x, y),
            r(-50.0) + unit * r(200.0)
        );
        // An extent changes nothing for a shape that has no edges.
        assert_eq!(
            moisture
                .clone()
                .within(realm())
                .sample(SEED, MOISTURE, x, y),
            unit
        );
    }
}

#[test]
fn an_island_is_highest_near_the_middle_and_below_zero_toward_the_edges() {
    let height = field(ISLAND_HEIGHT).within(realm());
    let at = |x: f64, y: f64| height.sample(SEED, HEIGHT, r(x), r(y));

    // All along every edge, and beyond, it is at the bottom of its range.
    for step in 0..=64 {
        let along = f64::from(step) * 8.0;
        for value in [
            at(along, 0.0),
            at(along, 512.0),
            at(0.0, along),
            at(512.0, along),
        ] {
            assert_eq!(value, r(-12.0));
        }
        assert_eq!(at(along, -40.0), r(-12.0));
        assert_eq!(at(600.0, along), r(-12.0));
    }

    // The middle is in the upper half of the range, whatever the seed.
    for seed in 0..200 {
        let middle = height.sample(seed, HEIGHT, r(256.0), r(256.0));
        assert!(
            middle >= r(9.0) && middle <= r(30.0),
            "seed {seed}: {middle}"
        );
    }

    // Rings of positions round the middle get lower as they get wider,
    // and the outer ones are under water.
    let ring_mean = |reach: f64| {
        let mut total = Real::ZERO;
        let mut count = 0;
        for step in 0..256 {
            // A square ring: `reach` of the way from the middle to the edge.
            let along = f64::from(step) / 256.0 * 2.0 - 1.0;
            for (dx, dy) in [(along, -1.0), (along, 1.0), (-1.0, along), (1.0, along)] {
                total += at(256.0 + dx * reach * 256.0, 256.0 + dy * reach * 256.0);
                count += 1;
            }
        }
        total / Real::from(count)
    };
    let rings: Vec<Real> = [0.0, 0.2, 0.4, 0.6, 0.8, 0.95, 1.0]
        .into_iter()
        .map(ring_mean)
        .collect();
    for pair in rings.windows(2) {
        assert!(pair[0] > pair[1], "{rings:?}");
    }
    assert!(rings[0] > r(9.0), "{rings:?}");
    assert!(rings[4] < Real::ZERO, "{rings:?}");
    // At 0.95 of the way out the bump is at most 1 − 0.95², so nothing
    // on that ring is above −12 + 42 · 0.0975.
    assert!(rings[5] < r(-7.9), "{rings:?}");
    assert_eq!(rings[6], r(-12.0));

    // Over the whole realm: land in the middle, sea round it, nothing
    // outside the range, and the highest point well inside.
    let mut highest = (Real::NEG_INFINITY, 0.0, 0.0);
    let mut land = 0;
    let mut cells = 0;
    for ty in 0..=128 {
        for tx in 0..=128 {
            let (x, y) = (f64::from(tx) * 4.0, f64::from(ty) * 4.0);
            let value = at(x, y);
            assert!(value >= r(-12.0) && value <= r(30.0));
            if value > highest.0 {
                highest = (value, x, y);
            }
            cells += 1;
            if value > Real::ZERO {
                land += 1;
                // Land never reaches the outer tenth of the realm.
                let inset = x.min(512.0 - x).min(y).min(512.0 - y);
                assert!(inset > 25.0, "land at {x}, {y}");
            }
        }
    }
    assert!(highest.0 > r(15.0), "{highest:?}");
    assert!(
        (highest.1 - 256.0).abs() < 128.0 && (highest.2 - 256.0).abs() < 128.0,
        "{highest:?}"
    );
    let share = f64::from(land) / f64::from(cells);
    assert!((0.2..0.6).contains(&share), "land covers {share}");
}

#[test]
fn a_continent_follows_its_noise_inland_and_sinks_at_the_edges() {
    let land = field(
        r#"{ "noise": { "octaves": 4, "scale": 60 }, "shape": "continent", "range": [-12, 30] }"#,
    )
    .within(realm());
    let flat = field(r#"{ "noise": { "octaves": 4, "scale": 60 }, "range": [-12, 30] }"#);
    let at = |x: f64, y: f64| land.sample(SEED, HEIGHT, r(x), r(y));

    for step in 0..=64 {
        let along = f64::from(step) * 8.0;
        for value in [
            at(along, 0.0),
            at(along, 512.0),
            at(0.0, along),
            at(512.0, along),
        ] {
            assert_eq!(value, r(-12.0));
        }
    }
    // At the very middle it is the noise alone.
    assert_eq!(
        at(256.0, 256.0),
        flat.sample(SEED, HEIGHT, r(256.0), r(256.0))
    );
    // Across the inner half it stays within a tenth of the range of the
    // noise, and never above it.
    let mut inland = 0;
    let mut total = 0;
    for ty in 0..=64 {
        for tx in 0..=64 {
            let (x, y) = (128.0 + f64::from(tx) * 4.0, 128.0 + f64::from(ty) * 4.0);
            let plain = flat.sample(SEED, HEIGHT, r(x), r(y));
            let shaped = at(x, y);
            assert!(shaped <= plain);
            assert!(
                plain - shaped < r(4.2),
                "{plain} against {shaped} at {x}, {y}"
            );
            total += 1;
            if shaped > Real::ZERO {
                inland += 1;
            }
        }
    }
    // Most of the interior is land: far more than an island has.
    assert!(inland * 10 > total * 8, "{inland} of {total}");
}

#[test]
fn a_ridged_field_is_the_ridged_sum() {
    let ridges =
        field(r#"{ "noise": { "octaves": 4, "scale": 25 }, "shape": "ridged", "range": [0, 60] }"#);
    let (low, high, _) = survey(|[x, y, _]| ridges.sample(SEED, HEIGHT, x, y));
    assert!(low >= Real::ZERO && high <= r(60.0), "{low} to {high}");
    assert!(high - low > r(25.0), "{low} to {high}");
    let (x, y) = (r(310.0), r(-45.5));
    assert_eq!(
        ridges.sample(SEED, HEIGHT, x, y),
        ridged_2d(SEED, HEIGHT, Fractal::new(4), x / r(25.0), y / r(25.0)) * r(60.0)
    );
    assert_eq!(
        ridges.noise.sample_ridged(SEED, HEIGHT, x, y) * r(60.0),
        ridges.sample(SEED, HEIGHT, x, y)
    );
}

#[test]
fn two_fields_of_one_realm_are_unrelated() {
    let height = field(ISLAND_HEIGHT);
    let moisture = field(ISLAND_MOISTURE);
    let related = correlation(
        |[x, y, _]| height.noise.sample(SEED, HEIGHT, x * r(40.0), y * r(40.0)),
        |[x, y, _]| {
            moisture
                .noise
                .sample(SEED, MOISTURE, x * r(40.0), y * r(40.0))
        },
    );
    assert!(related.abs() < 0.03, "height against moisture: {related}");
    // The same description under two domains is two different fields.
    let twice = correlation(
        |[x, y, _]| moisture.sample(SEED, HEIGHT, x * r(40.0), y * r(40.0)),
        |[x, y, _]| moisture.sample(SEED, MOISTURE, x * r(40.0), y * r(40.0)),
    );
    assert!(twice.abs() < 0.03, "one description, two domains: {twice}");
}

#[test]
fn a_field_stays_in_its_range_whatever_it_is_given() {
    let shapes = ["flat", "island", "continent", "ridged"];
    let extents = [
        None,
        Some(realm()),
        // Extents no realm has: no width, inside out, endless.
        Some(Extent::new(r(5.0), r(5.0), r(5.0), r(5.0))),
        Some(Extent::new(r(10.0), r(10.0), r(-10.0), r(-10.0))),
        Some(Extent::new(
            Real::NEG_INFINITY,
            Real::NEG_INFINITY,
            Real::INFINITY,
            Real::INFINITY,
        )),
    ];
    let places = [
        0.0,
        5.0,
        256.0,
        -1e9,
        1e300,
        f64::INFINITY,
        f64::NEG_INFINITY,
    ];
    for shape in shapes {
        let json = format!(
            r#"{{ "noise": {{ "octaves": 3, "scale": 20 }}, "shape": "{shape}", "range": [-3, 8] }}"#
        );
        let spec = field(&json);
        for extent in extents {
            let mut spec = spec.clone();
            spec.extent = extent;
            for x in places {
                for y in places {
                    let value = spec.sample(SEED, HEIGHT, r(x), r(y));
                    assert!(
                        value >= r(-3.0) && value <= r(8.0),
                        "{shape} within {extent:?} at {x}, {y}: {value}"
                    );
                }
            }
        }
        // Numbers `validate` would refuse still give an answer in range.
        let mut broken = spec.clone();
        broken.noise.scale = Real::ZERO;
        broken.noise.octaves = u32::MAX;
        broken.noise.gain = r(-3.0);
        broken.noise.lacunarity = Real::INFINITY;
        assert!(broken.validate().is_err());
        let value = broken.sample(SEED, HEIGHT, r(12.0), r(34.0));
        assert!(value >= r(-3.0) && value <= r(8.0), "{shape}: {value}");
    }
    // A range written backwards is refused by `validate`, and sampling
    // it still stays between its two ends.
    let mut backwards = field(ISLAND_MOISTURE);
    backwards.range = [r(8.0), r(-3.0)];
    let value = backwards.sample(SEED, HEIGHT, r(1.0), r(2.0));
    assert!(value >= r(-3.0) && value <= r(8.0));
}

#[test]
fn the_extent_s_bump_is_one_in_the_middle_and_zero_at_the_edges() {
    let extent = Extent::new(r(-100.0), r(40.0), r(300.0), r(240.0));
    assert_eq!(extent.bump(r(100.0), r(140.0)), Real::ONE);
    for (x, y) in [
        (-100.0, 140.0),
        (300.0, 140.0),
        (100.0, 40.0),
        (100.0, 240.0),
    ] {
        assert_eq!(extent.bump(r(x), r(y)), Real::ZERO);
    }
    assert_eq!(extent.bump(r(-500.0), r(140.0)), Real::ZERO);
    assert_eq!(extent.bump(r(100.0), r(1e12)), Real::ZERO);
    // Half way to an edge along one axis: 1 − ½² = ¾.
    assert_eq!(extent.bump(r(200.0), r(140.0)), r(0.75));
    assert_eq!(extent.bump(r(0.0), r(190.0)), r(0.75 * 0.75));
    // It falls steadily from the middle outward.
    let mut last = Real::ONE;
    for step in 1..=100 {
        let now = extent.bump(r(100.0 + 2.0 * f64::from(step)), r(140.0));
        assert!(now < last);
        last = now;
    }
}
