//! Deterministic noise for world generation: smooth random functions of
//! position, and the typed description of a noise field that a `worldgen`
//! definition gives (10 §9.1).
//!
//! Everything here is a pure function of a seed, a [`HashDomain`] and a
//! position. Nothing is stored and nothing depends on the order of calls,
//! so a world comes out the same however it is explored (10 X4, X6). The
//! only randomness is [`pos_hash`], and the only arithmetic is that of
//! [`Real`], so the results are the same bits on every supported target.
//!
//! # What there is
//!
//! | Function | Gives | Range |
//! |----------|-------|-------|
//! | [`value_2d`], [`value_3d`] | Random values at whole positions, blended between | 0 to 1 |
//! | [`gradient_2d`], [`gradient_3d`] | Random slopes at whole positions, blended between; zero at every whole position | −1 to 1 |
//! | [`fbm_2d`], [`fbm_3d`] | A sum of gradient noise at rising frequency and falling strength | −1 to 1 |
//! | [`ridged_2d`], [`ridged_3d`] | The same sum folded into sharp crests | 0 to 1 |
//! | [`billow_2d`], [`billow_3d`] | The same sum folded into rounded mounds | 0 to 1 |
//! | [`cellular_2d`], [`cellular_3d`] | The distances to the two nearest scattered points, and which point is nearest | 0 to under 2 |
//! | [`warp_2d`], [`warp_3d`] | A position pushed about by noise, to sample something else at | within the strength given |
//!
//! Every range is a promise: results are clamped to it, and the tests
//! sample each function widely.
//!
//! # Seeds and domains
//!
//! Every function takes a seed and a domain. Two fields of one world use
//! the world's seed and different domains, and are then unrelated:
//!
//! ```
//! use clash_core::{HashDomain, Real};
//! use clash_gen::noise::value_2d;
//!
//! const HEIGHT: HashDomain = HashDomain::new("field.height");
//! const MOISTURE: HashDomain = HashDomain::new("field.moisture");
//!
//! let (x, y) = (Real::of(12.25), Real::of(-3.5));
//! let height = value_2d(7, HEIGHT, x, y);
//! assert_eq!(height, value_2d(7, HEIGHT, x, y));
//! assert_ne!(height, value_2d(7, MOISTURE, x, y));
//! assert_ne!(height, value_2d(8, HEIGHT, x, y));
//! ```
//!
//! Within one domain the kinds of noise are unrelated too: value noise,
//! gradient noise, the scattered points and each octave of a sum draw
//! from separate salts.
//!
//! # Positions
//!
//! A position is in **lattice units**: features of the plain functions are
//! about one unit across. Divide a position by the size wanted before
//! calling, as [`NoiseSpec`] does with its `scale`. The lattice repeats
//! every 2³² units along each axis, and any position, however large, gives
//! a result: nothing here panics.
//!
//! # Descriptions from data
//!
//! [`FieldSpec`] and [`NoiseSpec`] are what a `worldgen` definition writes
//! for a noise field (10 §9.1), read with serde:
//!
//! ```
//! use clash_core::{HashDomain, Real};
//! use clash_gen::noise::{Extent, FieldSpec};
//!
//! let json = r#"{ "noise": { "octaves": 5, "scale": 90 },
//!                 "shape": "island", "range": [-12, 30] }"#;
//! let height: FieldSpec = serde_json::from_str(json).unwrap();
//! height.validate().unwrap();
//!
//! // An island fills the extent it is given: here 512 by 512.
//! let size = Real::of(512.0);
//! let height = height.within(Extent::new(Real::ZERO, Real::ZERO, size, size));
//! let domain = HashDomain::new("field.height");
//! let middle = height.sample(7, domain, Real::of(256.0), Real::of(256.0));
//! let corner = height.sample(7, domain, Real::ZERO, Real::ZERO);
//! assert!(middle >= Real::of(9.0));
//! assert_eq!(corner, Real::of(-12.0));
//! ```

use clash_core::{HashDomain, Real, pos_hash};
use serde::Deserialize;
use thiserror::Error;

/// The most octaves a fractal sum adds. A sum asked for more adds this
/// many. At the usual gain of one half, the sixteenth octave is already
/// thirty thousand times weaker than the first.
pub const MAX_OCTAVES: u32 = 16;

/// How much finer each octave is than the one before, unless said.
pub const DEFAULT_LACUNARITY: Real = Real::of(2.0);

/// How much weaker each octave is than the one before, unless said.
pub const DEFAULT_GAIN: Real = Real::of(0.5);

/// The range a field is mapped to unless it gives one: zero to one.
pub const DEFAULT_RANGE: [Real; 2] = [Real::ZERO, Real::ONE];

/// The share of an island's height that comes from noise. The rest comes
/// from being near the middle, so the middle of an island is never lower
/// than this share below the top of its range.
pub const ISLAND_NOISE_SHARE: Real = Real::of(0.5);

const HALF: Real = Real::of(0.5);
const MINUS_ONE: Real = Real::of(-1.0);

/// 2⁻⁵³: turns the upper 53 bits of a hash into a number below one.
const UNIT_STEP: f64 = 1.0 / 9_007_199_254_740_992.0;
/// The bits of a hash that do not fit a `Real` exactly.
const UNIT_SPARE_BITS: u32 = 11;

/// The quintic blend `6t⁵ − 15t⁴ + 10t³` and its coefficients: flat at
/// both ends to the second derivative, so neighbouring lattice cells meet
/// without a crease.
const FADE_A: Real = Real::of(6.0);
const FADE_B: Real = Real::of(15.0);
const FADE_C: Real = Real::of(10.0);

// What each salt is for. A salt is a purpose in its upper half and an
// octave in its lower, so no two uses of one domain share a hash.
const PURPOSE_VALUE: u64 = 1;
const PURPOSE_GRADIENT: u64 = 2;
const PURPOSE_CELL_POINT: u64 = 3;
const PURPOSE_CELL_ID: u64 = 4;
const PURPOSE_WARP_X: u64 = 5;
const PURPOSE_WARP_Y: u64 = 6;
const PURPOSE_WARP_Z: u64 = 7;
/// Where the purpose sits in a salt.
const PURPOSE_SHIFT: u32 = u32::BITS;

const fn salt(purpose: u64, octave: u32) -> u64 {
    (purpose << PURPOSE_SHIFT) | octave as u64
}

/// cos 45°, the two parts of a unit diagonal in the plane.
const DIAGONAL_2: f64 = core::f64::consts::FRAC_1_SQRT_2;
/// 1/√3, the three parts of a unit diagonal in space.
const DIAGONAL_3: f64 = 0.577_350_269_189_625_8;
/// cos 22.5°.
const COS_SIXTEENTH: f64 = 0.923_879_532_511_286_7;
/// sin 22.5°.
const SIN_SIXTEENTH: f64 = 0.382_683_432_365_089_8;

/// The number of hash bits that choose a gradient in the plane.
const GRADIENT_2D_BITS: u32 = 4;
/// The number of gradients in the plane.
const GRADIENT_2D_COUNT: usize = 1 << GRADIENT_2D_BITS;

const fn g2(x: f64, y: f64) -> [Real; 2] {
    [Real::of(x), Real::of(y)]
}

/// Sixteen unit vectors, evenly spaced round the circle.
const GRADIENTS_2D: [[Real; 2]; GRADIENT_2D_COUNT] = [
    g2(1.0, 0.0),
    g2(COS_SIXTEENTH, SIN_SIXTEENTH),
    g2(DIAGONAL_2, DIAGONAL_2),
    g2(SIN_SIXTEENTH, COS_SIXTEENTH),
    g2(0.0, 1.0),
    g2(-SIN_SIXTEENTH, COS_SIXTEENTH),
    g2(-DIAGONAL_2, DIAGONAL_2),
    g2(-COS_SIXTEENTH, SIN_SIXTEENTH),
    g2(-1.0, 0.0),
    g2(-COS_SIXTEENTH, -SIN_SIXTEENTH),
    g2(-DIAGONAL_2, -DIAGONAL_2),
    g2(-SIN_SIXTEENTH, -COS_SIXTEENTH),
    g2(0.0, -1.0),
    g2(SIN_SIXTEENTH, -COS_SIXTEENTH),
    g2(DIAGONAL_2, -DIAGONAL_2),
    g2(COS_SIXTEENTH, -SIN_SIXTEENTH),
];

/// The number of gradients in space: the directions from a lattice point
/// to its 26 neighbours.
const GRADIENT_3D_COUNT: usize = 26;
/// The offsets along one axis to a neighbour: −1, 0 and 1.
const NEIGHBOUR_STEPS: usize = 3;

/// The 26 directions from a lattice point to its neighbours, each at unit
/// length: 6 along an axis, 12 across a face and 8 through a corner.
const GRADIENTS_3D: [[Real; 3]; GRADIENT_3D_COUNT] = {
    let mut table = [[Real::ZERO; 3]; GRADIENT_3D_COUNT];
    let mut filled = 0;
    let mut code = 0;
    while code < NEIGHBOUR_STEPS * NEIGHBOUR_STEPS * NEIGHBOUR_STEPS {
        let x = (code % NEIGHBOUR_STEPS) as f64 - 1.0;
        let y = ((code / NEIGHBOUR_STEPS) % NEIGHBOUR_STEPS) as f64 - 1.0;
        let z = (code / (NEIGHBOUR_STEPS * NEIGHBOUR_STEPS)) as f64 - 1.0;
        let axes = (x * x + y * y + z * z) as u32;
        if axes != 0 {
            let length = match axes {
                1 => 1.0,
                2 => DIAGONAL_2,
                _ => DIAGONAL_3,
            };
            table[filled] = [
                Real::of(x * length),
                Real::of(y * length),
                Real::of(z * length),
            ];
            filled += 1;
        }
        code += 1;
    }
    table
};

/// What plain gradient noise in the plane is multiplied by to reach ±1.
/// With unit gradients it reaches ±√2⁄2, at the middle of a cell whose
/// four corners all slope toward it.
const GRADIENT_2D_SCALE: Real = Real::of(core::f64::consts::SQRT_2);
/// The same for space, where the reach is ±√3⁄2: this is 2⁄√3.
const GRADIENT_3D_SCALE: Real = Real::of(2.0 * DIAGONAL_3);

/// The number of hash bits that place a scattered point along each axis.
const POINT_BITS: u32 = 21;
/// 2⁻²¹: turns those bits into a position within a cell.
const POINT_STEP: f64 = 1.0 / 2_097_152.0;
/// The mask for one axis's bits.
const POINT_MASK: u64 = (1 << POINT_BITS) - 1;

/// How far the first search for scattered points looks: one cell each
/// way, which nearly always holds the two nearest.
const NEAR_RING: i32 = 1;
/// How far the second search looks when the first cannot be sure: two
/// cells each way, which always holds the two nearest.
const FAR_RING: i32 = 2;

/// A hash as a number from zero up to but not including one.
fn unit(hash: u64) -> Real {
    Real::of((hash >> UNIT_SPARE_BITS) as f64 * UNIT_STEP)
}

/// The lattice cell a coordinate is in, and how far into it: from zero up
/// to but not including one. Cells wrap round every 2³².
fn split(value: Real) -> (i32, Real) {
    let floor = value.floor();
    (floor.to_i64() as i32, value - floor)
}

/// The quintic blend: 0 at 0, 1 at 1, and flat at both.
fn fade(t: Real) -> Real {
    t * t * t * (t * (t * FADE_A - FADE_B) + FADE_C)
}

/// The value `t` of the way from `a` to `b`.
fn lerp(a: Real, b: Real, t: Real) -> Real {
    a + (b - a) * t
}

/// The random value at a lattice point.
fn lattice_value(seed: u64, domain: HashDomain, salt: u64, x: i32, y: i32, z: i32) -> Real {
    unit(pos_hash(seed, domain, x, y, z, salt))
}

fn value_plane(seed: u64, domain: HashDomain, salt: u64, x: Real, y: Real) -> Real {
    let (x0, fx) = split(x);
    let (y0, fy) = split(y);
    let (x1, y1) = (x0.wrapping_add(1), y0.wrapping_add(1));
    let at = |x: i32, y: i32| lattice_value(seed, domain, salt, x, y, 0);
    let u = fade(fx);
    let near = lerp(at(x0, y0), at(x1, y0), u);
    let far = lerp(at(x0, y1), at(x1, y1), u);
    lerp(near, far, fade(fy)).clamp(Real::ZERO, Real::ONE)
}

fn value_space(seed: u64, domain: HashDomain, salt: u64, x: Real, y: Real, z: Real) -> Real {
    let (x0, fx) = split(x);
    let (y0, fy) = split(y);
    let (z0, fz) = split(z);
    let (x1, y1, z1) = (x0.wrapping_add(1), y0.wrapping_add(1), z0.wrapping_add(1));
    let at = |x: i32, y: i32, z: i32| lattice_value(seed, domain, salt, x, y, z);
    let (u, v) = (fade(fx), fade(fy));
    let layer = |z: i32| {
        let near = lerp(at(x0, y0, z), at(x1, y0, z), u);
        let far = lerp(at(x0, y1, z), at(x1, y1, z), u);
        lerp(near, far, v)
    };
    lerp(layer(z0), layer(z1), fade(fz)).clamp(Real::ZERO, Real::ONE)
}

fn gradient_plane(seed: u64, domain: HashDomain, salt: u64, x: Real, y: Real) -> Real {
    let (x0, fx) = split(x);
    let (y0, fy) = split(y);
    let (x1, y1) = (x0.wrapping_add(1), y0.wrapping_add(1));
    // The slope at a corner, read at an offset from that corner.
    let at = |x: i32, y: i32, dx: Real, dy: Real| {
        let hash = pos_hash(seed, domain, x, y, 0, salt);
        let [gx, gy] = GRADIENTS_2D[(hash >> (u64::BITS - GRADIENT_2D_BITS)) as usize];
        gx * dx + gy * dy
    };
    let (bx, by) = (fx - Real::ONE, fy - Real::ONE);
    let u = fade(fx);
    let near = lerp(at(x0, y0, fx, fy), at(x1, y0, bx, fy), u);
    let far = lerp(at(x0, y1, fx, by), at(x1, y1, bx, by), u);
    (lerp(near, far, fade(fy)) * GRADIENT_2D_SCALE).clamp(MINUS_ONE, Real::ONE)
}

fn gradient_space(seed: u64, domain: HashDomain, salt: u64, x: Real, y: Real, z: Real) -> Real {
    let (x0, fx) = split(x);
    let (y0, fy) = split(y);
    let (z0, fz) = split(z);
    let (x1, y1, z1) = (x0.wrapping_add(1), y0.wrapping_add(1), z0.wrapping_add(1));
    let at = |x: i32, y: i32, z: i32, dx: Real, dy: Real, dz: Real| {
        let hash = pos_hash(seed, domain, x, y, z, salt);
        let [gx, gy, gz] = GRADIENTS_3D[(hash % GRADIENT_3D_COUNT as u64) as usize];
        gx * dx + gy * dy + gz * dz
    };
    let (bx, by, bz) = (fx - Real::ONE, fy - Real::ONE, fz - Real::ONE);
    let (u, v) = (fade(fx), fade(fy));
    let layer = |z: i32, dz: Real| {
        let near = lerp(at(x0, y0, z, fx, fy, dz), at(x1, y0, z, bx, fy, dz), u);
        let far = lerp(at(x0, y1, z, fx, by, dz), at(x1, y1, z, bx, by, dz), u);
        lerp(near, far, v)
    };
    (lerp(layer(z0, fz), layer(z1, bz), fade(fz)) * GRADIENT_3D_SCALE).clamp(MINUS_ONE, Real::ONE)
}

/// Value noise in the plane, from 0 to 1.
///
/// Each whole position has a random value, uniform from 0 to 1, and the
/// result blends the four around the position with the quintic
/// `6t⁵ − 15t⁴ + 10t³`. At a whole position it is that position's value.
/// It is the slice of [`value_3d`] at `z = 0`.
pub fn value_2d(seed: u64, domain: HashDomain, x: Real, y: Real) -> Real {
    value_plane(seed, domain, salt(PURPOSE_VALUE, 0), x, y)
}

/// Value noise in space, from 0 to 1: [`value_2d`] with a third axis,
/// blending the eight values around the position.
pub fn value_3d(seed: u64, domain: HashDomain, x: Real, y: Real, z: Real) -> Real {
    value_space(seed, domain, salt(PURPOSE_VALUE, 0), x, y, z)
}

/// Gradient noise in the plane, from −1 to 1.
///
/// Each whole position has a random slope, one of sixteen unit vectors
/// evenly spaced round the circle. Each of the four positions around the
/// point contributes its slope times the offset from it, and the four are
/// blended with the quintic. The result is zero at every whole position,
/// so it has no flat patches the size of a cell as value noise has. It is
/// multiplied by √2 so that its reach is exactly ±1.
pub fn gradient_2d(seed: u64, domain: HashDomain, x: Real, y: Real) -> Real {
    gradient_plane(seed, domain, salt(PURPOSE_GRADIENT, 0), x, y)
}

/// Gradient noise in space, from −1 to 1: [`gradient_2d`] with a third
/// axis. The slopes are the 26 unit directions from a lattice point to
/// its neighbours, and the result is multiplied by 2⁄√3.
pub fn gradient_3d(seed: u64, domain: HashDomain, x: Real, y: Real, z: Real) -> Real {
    gradient_space(seed, domain, salt(PURPOSE_GRADIENT, 0), x, y, z)
}

/// How a fractal sum is built: how many layers of noise, how much finer
/// each is than the last, and how much weaker.
///
/// ```
/// use clash_core::Real;
/// use clash_gen::noise::Fractal;
///
/// let rough = Fractal { gain: Real::of(0.7), ..Fractal::new(6) };
/// assert_eq!(rough.lacunarity, Real::of(2.0));
/// ```
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Fractal {
    /// The number of layers. Zero counts as one, and more than
    /// [`MAX_OCTAVES`] as [`MAX_OCTAVES`].
    pub octaves: u32,
    /// What the frequency is multiplied by from one layer to the next.
    pub lacunarity: Real,
    /// What the strength is multiplied by from one layer to the next.
    pub gain: Real,
}

impl Fractal {
    /// A sum of this many octaves with the usual lacunarity of 2 and gain
    /// of one half.
    pub const fn new(octaves: u32) -> Fractal {
        Fractal {
            octaves,
            lacunarity: DEFAULT_LACUNARITY,
            gain: DEFAULT_GAIN,
        }
    }

    /// Adds the layers: `layer(octave, frequency)` is each layer's noise,
    /// and the result is their weighted mean, so it stays in the range of
    /// one layer.
    fn sum(&self, mut layer: impl FnMut(u32, Real) -> Real) -> Real {
        let mut total = Real::ZERO;
        let mut weight = Real::ZERO;
        let mut strength = Real::ONE;
        let mut frequency = Real::ONE;
        for octave in 0..self.octaves.clamp(1, MAX_OCTAVES) {
            total += strength * layer(octave, frequency);
            weight += strength.abs();
            strength *= self.gain;
            frequency *= self.lacunarity;
        }
        total / weight
    }
}

/// What a fractal sum does to each layer before adding it.
#[derive(Clone, Copy)]
enum Fold {
    /// Nothing: the layer as it is, from −1 to 1.
    Plain,
    /// `(1 − |n|)²`: a sharp crest wherever the layer crosses zero.
    Ridged,
    /// `|n|`: a rounded mound between each pair of crossings.
    Billow,
}

impl Fold {
    fn apply(self, noise: Real) -> Real {
        match self {
            Fold::Plain => noise,
            Fold::Ridged => {
                let crest = Real::ONE - noise.abs();
                crest * crest
            }
            Fold::Billow => noise.abs(),
        }
    }

    fn clamp(self, total: Real) -> Real {
        match self {
            Fold::Plain => total.clamp(MINUS_ONE, Real::ONE),
            Fold::Ridged | Fold::Billow => total.clamp(Real::ZERO, Real::ONE),
        }
    }
}

struct Layers {
    seed: u64,
    domain: HashDomain,
    fractal: Fractal,
    /// Which lattice of slopes the layers read: plain noise, or one of
    /// the warp axes.
    purpose: u64,
    fold: Fold,
}

impl Layers {
    fn plane(&self, x: Real, y: Real) -> Real {
        let total = self.fractal.sum(|octave, frequency| {
            let salt = salt(self.purpose, octave);
            let noise = gradient_plane(self.seed, self.domain, salt, x * frequency, y * frequency);
            self.fold.apply(noise)
        });
        self.fold.clamp(total)
    }

    fn space(&self, x: Real, y: Real, z: Real) -> Real {
        let total = self.fractal.sum(|octave, frequency| {
            let salt = salt(self.purpose, octave);
            let noise = gradient_space(
                self.seed,
                self.domain,
                salt,
                x * frequency,
                y * frequency,
                z * frequency,
            );
            self.fold.apply(noise)
        });
        self.fold.clamp(total)
    }
}

fn layers(seed: u64, domain: HashDomain, fractal: Fractal, purpose: u64, fold: Fold) -> Layers {
    Layers {
        seed,
        domain,
        fractal,
        purpose,
        fold,
    }
}

/// A fractal sum of gradient noise in the plane, from −1 to 1.
///
/// Octave `i` is [`gradient_2d`] on a lattice of its own, read at the
/// position times `lacunarity^i` and weighted by `gain^i`. The result is
/// the sum divided by the sum of the weights, so it keeps the range of
/// one octave. Most of it lies near zero: the more octaves, the rarer the
/// ends of the range.
pub fn fbm_2d(seed: u64, domain: HashDomain, fractal: Fractal, x: Real, y: Real) -> Real {
    layers(seed, domain, fractal, PURPOSE_GRADIENT, Fold::Plain).plane(x, y)
}

/// A fractal sum of gradient noise in space, from −1 to 1. See
/// [`fbm_2d`].
pub fn fbm_3d(seed: u64, domain: HashDomain, fractal: Fractal, x: Real, y: Real, z: Real) -> Real {
    layers(seed, domain, fractal, PURPOSE_GRADIENT, Fold::Plain).space(x, y, z)
}

/// Ridged noise in the plane, from 0 to 1: sharp crests with broad
/// valleys between, as mountain ridges and veins have.
///
/// It is [`fbm_2d`] with each octave `n` replaced by `(1 − |n|)²` before
/// it is added, so every line where an octave crosses zero becomes a
/// crest of height 1.
pub fn ridged_2d(seed: u64, domain: HashDomain, fractal: Fractal, x: Real, y: Real) -> Real {
    layers(seed, domain, fractal, PURPOSE_GRADIENT, Fold::Ridged).plane(x, y)
}

/// Ridged noise in space, from 0 to 1. See [`ridged_2d`].
pub fn ridged_3d(
    seed: u64,
    domain: HashDomain,
    fractal: Fractal,
    x: Real,
    y: Real,
    z: Real,
) -> Real {
    layers(seed, domain, fractal, PURPOSE_GRADIENT, Fold::Ridged).space(x, y, z)
}

/// Billowed noise in the plane, from 0 to 1: rounded mounds with sharp
/// creases between, as clouds and dunes have.
///
/// It is [`fbm_2d`] with each octave `n` replaced by `|n|` before it is
/// added.
pub fn billow_2d(seed: u64, domain: HashDomain, fractal: Fractal, x: Real, y: Real) -> Real {
    layers(seed, domain, fractal, PURPOSE_GRADIENT, Fold::Billow).plane(x, y)
}

/// Billowed noise in space, from 0 to 1. See [`billow_2d`].
pub fn billow_3d(
    seed: u64,
    domain: HashDomain,
    fractal: Fractal,
    x: Real,
    y: Real,
    z: Real,
) -> Real {
    layers(seed, domain, fractal, PURPOSE_GRADIENT, Fold::Billow).space(x, y, z)
}

/// A position in the plane pushed about by noise: sample another function
/// at the result and its features bend and swirl instead of following the
/// lattice.
///
/// Each coordinate is moved by `strength` times a fractal sum of its own
/// (as [`fbm_2d`], on separate lattices), so it moves by at most
/// `strength` in lattice units.
///
/// ```
/// use clash_core::{HashDomain, Real};
/// use clash_gen::noise::{warp_2d, ridged_2d, Fractal};
///
/// const VEINS: HashDomain = HashDomain::new("gen.veins");
///
/// let (x, y) = (Real::of(3.25), Real::of(8.5));
/// let (wx, wy) = warp_2d(1, VEINS, Fractal::new(2), Real::of(0.75), x, y);
/// assert!((wx - x).abs() <= Real::of(0.75) && (wy - y).abs() <= Real::of(0.75));
/// let vein = ridged_2d(1, VEINS, Fractal::new(4), wx, wy);
/// assert!(vein >= Real::ZERO && vein <= Real::ONE);
/// ```
pub fn warp_2d(
    seed: u64,
    domain: HashDomain,
    fractal: Fractal,
    strength: Real,
    x: Real,
    y: Real,
) -> (Real, Real) {
    let push = |purpose: u64| layers(seed, domain, fractal, purpose, Fold::Plain).plane(x, y);
    (
        x + strength * push(PURPOSE_WARP_X),
        y + strength * push(PURPOSE_WARP_Y),
    )
}

/// A position in space pushed about by noise. See [`warp_2d`].
pub fn warp_3d(
    seed: u64,
    domain: HashDomain,
    fractal: Fractal,
    strength: Real,
    x: Real,
    y: Real,
    z: Real,
) -> (Real, Real, Real) {
    let push = |purpose: u64| layers(seed, domain, fractal, purpose, Fold::Plain).space(x, y, z);
    (
        x + strength * push(PURPOSE_WARP_X),
        y + strength * push(PURPOSE_WARP_Y),
        z + strength * push(PURPOSE_WARP_Z),
    )
}

/// What [`cellular_2d`] and [`cellular_3d`] find at a position.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Cellular {
    /// The distance to the nearest scattered point, in lattice units.
    pub nearest: Real,
    /// The distance to the second nearest. Never less than `nearest`, and
    /// always under 2. `second - nearest` is zero on the borders between
    /// patches and grows toward their middles.
    pub second: Real,
    /// A hash that names the nearest point: the same everywhere in that
    /// point's patch, and unrelated between patches. Use it to choose
    /// what the patch is.
    pub cell: u64,
}

/// The two nearest points found so far, by squared distance.
struct Nearest {
    nearest_sq: Real,
    second_sq: Real,
    cell: [i32; 3],
}

impl Nearest {
    fn offer(&mut self, distance_sq: Real, cell: [i32; 3]) {
        if distance_sq < self.nearest_sq {
            self.second_sq = self.nearest_sq;
            self.nearest_sq = distance_sq;
            self.cell = cell;
        } else if distance_sq < self.second_sq {
            self.second_sq = distance_sq;
        }
    }
}

/// Where the scattered point of a lattice cell lies within it, along each
/// axis: from zero up to but not including one.
fn cell_point(seed: u64, domain: HashDomain, cell: [i32; 3]) -> [Real; 3] {
    let [x, y, z] = cell;
    let hash = pos_hash(seed, domain, x, y, z, salt(PURPOSE_CELL_POINT, 0));
    let part = |index: u32| {
        let bits = (hash >> (index * POINT_BITS)) & POINT_MASK;
        Real::of(bits as f64 * POINT_STEP)
    };
    [part(0), part(1), part(2)]
}

/// Looks at the points of every cell within `ring` cells of the one the
/// position is in. In the plane (`volume` false) only the cells of that
/// plane are looked at and the third axis is ignored.
fn scan(
    seed: u64,
    domain: HashDomain,
    cell: [i32; 3],
    within: [Real; 3],
    ring: i32,
    volume: bool,
) -> Nearest {
    let mut found = Nearest {
        nearest_sq: Real::INFINITY,
        second_sq: Real::INFINITY,
        cell,
    };
    let depth = if volume { ring } else { 0 };
    for dz in -depth..=depth {
        for dy in -ring..=ring {
            for dx in -ring..=ring {
                let other = [
                    cell[0].wrapping_add(dx),
                    cell[1].wrapping_add(dy),
                    cell[2].wrapping_add(dz),
                ];
                let point = cell_point(seed, domain, other);
                let x = Real::from(dx) + point[0] - within[0];
                let y = Real::from(dy) + point[1] - within[1];
                let z = if volume {
                    Real::from(dz) + point[2] - within[2]
                } else {
                    Real::ZERO
                };
                found.offer(x * x + y * y + z * z, other);
            }
        }
    }
    found
}

fn cellular(
    seed: u64,
    domain: HashDomain,
    cell: [i32; 3],
    within: [Real; 3],
    volume: bool,
) -> Cellular {
    // How near the position is to a face of its own cell. Every point
    // outside the near ring is at least one cell further away than that.
    let mut margin = Real::ONE;
    let axes = if volume { 3 } else { 2 };
    for part in &within[..axes] {
        margin = margin.min(*part).min(Real::ONE - *part);
    }
    let sure = Real::ONE + margin;
    let mut found = scan(seed, domain, cell, within, NEAR_RING, volume);
    if found.second_sq > sure * sure {
        // A point beyond the near ring could be nearer than the second
        // one found. None beyond the far ring can be: the two nearest are
        // always under two cells away.
        found = scan(seed, domain, cell, within, FAR_RING, volume);
    }
    let [x, y, z] = found.cell;
    Cellular {
        nearest: found.nearest_sq.sqrt(),
        second: found.second_sq.sqrt(),
        cell: pos_hash(seed, domain, x, y, z, salt(PURPOSE_CELL_ID, 0)),
    }
}

/// Cellular noise in the plane: the distances from a position to the two
/// nearest of a scatter of points, and which point is nearest.
///
/// There is one point in every lattice cell, placed anywhere within it.
/// The plane divides into irregular patches, one round each point: use
/// [`Cellular::cell`] to give each patch a kind, `nearest` for a value
/// that falls to zero at each point, and `second - nearest` for one that
/// falls to zero along the borders. The distances are exact, and so are
/// continuous everywhere.
///
/// ```
/// use clash_core::{HashDomain, Real};
/// use clash_gen::noise::cellular_2d;
///
/// const PATCHES: HashDomain = HashDomain::new("gen.patches");
///
/// let here = cellular_2d(3, PATCHES, Real::of(10.3), Real::of(4.8));
/// assert!(here.nearest <= here.second && here.second < Real::of(2.0));
/// let kind = here.cell % 3;
/// assert!(kind < 3);
/// ```
pub fn cellular_2d(seed: u64, domain: HashDomain, x: Real, y: Real) -> Cellular {
    let (cx, fx) = split(x);
    let (cy, fy) = split(y);
    cellular(seed, domain, [cx, cy, 0], [fx, fy, Real::ZERO], false)
}

/// Cellular noise in space: [`cellular_2d`] with a third axis, one point
/// in every lattice cube.
pub fn cellular_3d(seed: u64, domain: HashDomain, x: Real, y: Real, z: Real) -> Cellular {
    let (cx, fx) = split(x);
    let (cy, fy) = split(y);
    let (cz, fz) = split(z);
    cellular(seed, domain, [cx, cy, cz], [fx, fy, fz], true)
}

// ---- Descriptions from data ----------------------------------------------

/// Why a noise description cannot be used.
///
/// Sampling never fails: it treats a bad number as the nearest usable
/// one. This is for whoever loads the description, to tell the author.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[non_exhaustive]
pub enum NoiseSpecError {
    /// `octaves` is zero or more than [`MAX_OCTAVES`].
    #[error("`octaves` is {0}, but must be from 1 to {MAX_OCTAVES}")]
    Octaves(u32),
    /// `scale` is zero, negative or infinite.
    #[error("`scale` is {0}, but must be a finite number greater than zero")]
    Scale(Real),
    /// `lacunarity` is less than one or infinite.
    #[error("`lacunarity` is {0}, but must be a finite number of at least 1")]
    Lacunarity(Real),
    /// `gain` is zero, negative or more than one.
    #[error("`gain` is {0}, but must be greater than zero and at most 1")]
    Gain(Real),
    /// `range` is infinite, or its upper end is below its lower.
    #[error("`range` is [{low}, {high}], but must be two finite numbers, the lower first")]
    Range {
        /// The first number given.
        low: Real,
        /// The second number given.
        high: Real,
    },
}

fn default_lacunarity() -> Real {
    DEFAULT_LACUNARITY
}

fn default_gain() -> Real {
    DEFAULT_GAIN
}

fn default_range() -> [Real; 2] {
    DEFAULT_RANGE
}

/// The `noise` of a field, as a `worldgen` definition writes it
/// (10 §9.1): `{ "octaves": 5, "scale": 90 }`.
///
/// | Key | Meaning | Default |
/// |-----|---------|---------|
/// | `octaves` | How many layers are added, from 1 to [`MAX_OCTAVES`] | required |
/// | `scale` | The distance across the broadest features, in the units positions are given in | required |
/// | `lacunarity` | How much finer each layer is than the last | 2 |
/// | `gain` | How much weaker each layer is than the last | 0.5 |
#[derive(Deserialize, Debug, Clone, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct NoiseSpec {
    /// How many layers are added.
    pub octaves: u32,
    /// The distance across the broadest features.
    pub scale: Real,
    /// How much finer each layer is than the last.
    #[serde(default = "default_lacunarity")]
    pub lacunarity: Real,
    /// How much weaker each layer is than the last.
    #[serde(default = "default_gain")]
    pub gain: Real,
}

impl NoiseSpec {
    /// The fractal sum this describes.
    pub const fn fractal(&self) -> Fractal {
        Fractal {
            octaves: self.octaves,
            lacunarity: self.lacunarity,
            gain: self.gain,
        }
    }

    /// The noise at a position, from 0 to 1: [`fbm_2d`] at the position
    /// divided by `scale`, moved from ±1 to the range 0 to 1. Its mean is
    /// one half and most of it lies near that.
    pub fn sample(&self, seed: u64, domain: HashDomain, x: Real, y: Real) -> Real {
        let noise = fbm_2d(seed, domain, self.fractal(), x / self.scale, y / self.scale);
        (HALF + HALF * noise).clamp(Real::ZERO, Real::ONE)
    }

    /// The noise at a position folded into crests, from 0 to 1:
    /// [`ridged_2d`] at the position divided by `scale`.
    pub fn sample_ridged(&self, seed: u64, domain: HashDomain, x: Real, y: Real) -> Real {
        ridged_2d(seed, domain, self.fractal(), x / self.scale, y / self.scale)
    }

    /// Checks the numbers, for the loader to report to the author.
    pub fn validate(&self) -> Result<(), NoiseSpecError> {
        if self.octaves == 0 || self.octaves > MAX_OCTAVES {
            return Err(NoiseSpecError::Octaves(self.octaves));
        }
        if !(self.scale.is_finite() && self.scale > Real::ZERO) {
            return Err(NoiseSpecError::Scale(self.scale));
        }
        if !(self.lacunarity.is_finite() && self.lacunarity >= Real::ONE) {
            return Err(NoiseSpecError::Lacunarity(self.lacunarity));
        }
        if !(self.gain > Real::ZERO && self.gain <= Real::ONE) {
            return Err(NoiseSpecError::Gain(self.gain));
        }
        Ok(())
    }
}

/// The large form a field takes over the place it fills (10 §9.1, §13).
///
/// Below, `u` is the field's noise from 0 to 1 ([`NoiseSpec::sample`])
/// and `b` is the [`Extent::bump`] of the place: 1 at its middle and 0
/// all along its edges. Each shape gives a number `s` from 0 to 1, which
/// the field's `range` then stretches.
#[derive(Deserialize, Debug, Clone, Copy, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum Shape {
    /// No large form: `s = u`. This is what a field without a `shape`
    /// has.
    #[default]
    Flat,
    /// High in the middle and at the bottom of the range all along the
    /// edges: `s = b · (1 + u) / 2`. The middle is never below the middle
    /// of the range.
    Island,
    /// The noise over most of the place, falling to the bottom of the
    /// range only close to the edges: `s = u · ⁴√b`.
    Continent,
    /// Crests and valleys with no large form: `s` is the ridged sum
    /// ([`NoiseSpec::sample_ridged`]).
    Ridged,
}

impl Shape {
    /// Whether the shape depends on where the edges of the place are, and
    /// so needs an [`Extent`].
    pub const fn needs_extent(self) -> bool {
        matches!(self, Shape::Island | Shape::Continent)
    }
}

/// The rectangle a shaped field fills: usually the whole of its realm.
/// In the same units as the positions the field is sampled at.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Extent {
    /// The western edge.
    pub min_x: Real,
    /// The northern edge.
    pub min_y: Real,
    /// The eastern edge.
    pub max_x: Real,
    /// The southern edge.
    pub max_y: Real,
}

impl Extent {
    /// The rectangle between two corners.
    pub const fn new(min_x: Real, min_y: Real, max_x: Real, max_y: Real) -> Extent {
        Extent {
            min_x,
            min_y,
            max_x,
            max_y,
        }
    }

    /// How far inside the rectangle a position is: 1 at the middle, 0
    /// along every edge and outside.
    ///
    /// With `nx` and `ny` the position's offsets from the middle as
    /// fractions of the way to the edge, from −1 to 1, it is
    /// `(1 − nx²)(1 − ny²)`.
    pub fn bump(&self, x: Real, y: Real) -> Real {
        let along = |at: Real, min: Real, max: Real| {
            let offset = ((at - min) + (at - max)) / (max - min);
            let offset = offset.clamp(MINUS_ONE, Real::ONE);
            Real::ONE - offset * offset
        };
        along(x, self.min_x, self.max_x) * along(y, self.min_y, self.max_y)
    }
}

/// A noise field, as a `worldgen` definition writes it (10 §9.1):
/// `{ "noise": { "octaves": 5, "scale": 90 }, "shape": "island", "range": [-12, 30] }`.
///
/// | Key | Meaning | Default |
/// |-----|---------|---------|
/// | `noise` | The noise: a [`NoiseSpec`] | required |
/// | `shape` | The large form: a [`Shape`] | `flat` |
/// | `range` | The lowest and highest values the field takes | `[0, 1]` |
///
/// The value at a position is `low + s · (high − low)`, where `s` is what
/// the shape makes of the noise there.
///
/// The extent is not written in the definition. Whoever binds the field
/// to a realm gives it with [`within`](FieldSpec::within).
#[derive(Deserialize, Debug, Clone, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct FieldSpec {
    /// The noise.
    pub noise: NoiseSpec,
    /// The large form.
    #[serde(default)]
    pub shape: Shape,
    /// The lowest and highest values the field takes.
    #[serde(default = "default_range")]
    pub range: [Real; 2],
    /// The place the shape fills. Set by [`within`](FieldSpec::within),
    /// never read from data.
    #[serde(skip)]
    pub extent: Option<Extent>,
}

impl FieldSpec {
    /// The same field, filling this extent. An `island` or `continent`
    /// needs one; the other shapes ignore it.
    pub fn within(mut self, extent: Extent) -> FieldSpec {
        self.extent = Some(extent);
        self
    }

    /// The field's value at a position, always within `range`.
    ///
    /// The seed is the realm's and the domain is the field's own, so that
    /// two fields of one realm are unrelated. An `island` or `continent`
    /// that was given no extent is sampled as though every position were
    /// the middle; [`validate`](FieldSpec::validate) cannot see that, so
    /// the loader checks [`Shape::needs_extent`].
    pub fn sample(&self, seed: u64, domain: HashDomain, x: Real, y: Real) -> Real {
        let bump = || match &self.extent {
            Some(extent) => extent.bump(x, y),
            None => Real::ONE,
        };
        let shaped = match self.shape {
            Shape::Flat => self.noise.sample(seed, domain, x, y),
            Shape::Island => {
                let noise = self.noise.sample(seed, domain, x, y);
                bump() * (Real::ONE - ISLAND_NOISE_SHARE + ISLAND_NOISE_SHARE * noise)
            }
            Shape::Continent => {
                let noise = self.noise.sample(seed, domain, x, y);
                noise * bump().sqrt().sqrt()
            }
            Shape::Ridged => self.noise.sample_ridged(seed, domain, x, y),
        };
        let [low, high] = self.range;
        let shaped = shaped.clamp(Real::ZERO, Real::ONE);
        (low + shaped * (high - low)).clamp(low.min(high), low.max(high))
    }

    /// Checks the numbers, for the loader to report to the author.
    pub fn validate(&self) -> Result<(), NoiseSpecError> {
        self.noise.validate()?;
        let [low, high] = self.range;
        if !(low.is_finite() && high.is_finite() && low <= high) {
            return Err(NoiseSpecError::Range { low, high });
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_blend_is_flat_at_both_ends() {
        assert_eq!(fade(Real::ZERO), Real::ZERO);
        assert_eq!(fade(Real::ONE), Real::ONE);
        assert_eq!(fade(HALF), HALF);
        let near = Real::of(1e-3);
        assert!(fade(near) < Real::of(1e-8));
        assert!(Real::ONE - fade(Real::ONE - near) < Real::of(1e-8));
        // It never turns back.
        let mut last = Real::ZERO;
        for step in 1..=1000 {
            let now = fade(Real::from(step) / Real::of(1000.0));
            assert!(now >= last);
            last = now;
        }
    }

    #[test]
    fn a_coordinate_splits_into_a_cell_and_a_fraction() {
        assert_eq!(split(Real::of(2.75)), (2, Real::of(0.75)));
        assert_eq!(split(Real::of(-2.75)), (-3, Real::of(0.25)));
        assert_eq!(split(Real::ZERO), (0, Real::ZERO));
        assert_eq!(split(Real::of(-0.0)), (0, Real::ZERO));
        // Cells wrap round every 2^32, and nothing is too large to split.
        assert_eq!(split(Real::of(4_294_967_296.5)), (0, HALF));
        assert_eq!(split(Real::of(2_147_483_648.0)).0, i32::MIN);
        let (_, fraction) = split(Real::INFINITY);
        assert_eq!(fraction, Real::ZERO);
        let (_, fraction) = split(Real::of(-1e300));
        assert_eq!(fraction, Real::ZERO);
    }

    #[test]
    fn the_gradients_are_unit_vectors_that_cancel() {
        let close = |a: Real, b: f64| (a.get() - b).abs() < 1e-15;
        let mut sum = [Real::ZERO; 2];
        for [x, y] in GRADIENTS_2D {
            assert!(close(x * x + y * y, 1.0));
            sum[0] += x;
            sum[1] += y;
        }
        assert!(close(sum[0], 0.0) && close(sum[1], 0.0));

        let mut sum = [Real::ZERO; 3];
        let mut kinds = [0; 3];
        for [x, y, z] in GRADIENTS_3D {
            assert!(close(x * x + y * y + z * z, 1.0));
            let axes = [x, y, z].iter().filter(|part| **part != Real::ZERO).count();
            kinds[axes - 1] += 1;
            sum[0] += x;
            sum[1] += y;
            sum[2] += z;
        }
        assert_eq!(kinds, [6, 12, 8]);
        assert!(sum.iter().all(|part| close(*part, 0.0)));
        // No direction appears twice.
        for (index, a) in GRADIENTS_3D.iter().enumerate() {
            for b in &GRADIENTS_3D[index + 1..] {
                assert_ne!(a, b);
            }
        }
    }

    #[test]
    fn a_scattered_point_lies_within_its_cell() {
        let domain = HashDomain::new("test.points");
        for index in 0..2000 {
            let point = cell_point(9, domain, [index, -index, index * 7]);
            for part in point {
                assert!(part >= Real::ZERO && part < Real::ONE);
            }
        }
    }

    #[test]
    fn the_two_nearest_points_are_found_exactly() {
        // The quick search must agree with a search far wider than any
        // point could matter, to the last bit, in the plane and in space.
        const WIDE_RING: i32 = 4;
        let domain = HashDomain::new("test.exact");
        let mut rng = clash_core::Rng::from_seed(41);
        for round in 0..4000 {
            let volume = round % 2 == 0;
            let cell = [
                rng.range(-50, 50) as i32,
                rng.range(-50, 50) as i32,
                if volume { rng.range(-50, 50) as i32 } else { 0 },
            ];
            let within = [
                rng.unit(),
                rng.unit(),
                if volume { rng.unit() } else { Real::ZERO },
            ];
            let quick = cellular(5, domain, cell, within, volume);
            let wide = scan(5, domain, cell, within, WIDE_RING, volume);
            assert_eq!(quick.nearest, wide.nearest_sq.sqrt(), "round {round}");
            assert_eq!(quick.second, wide.second_sq.sqrt(), "round {round}");
            let [x, y, z] = wide.cell;
            let id = pos_hash(5, domain, x, y, z, salt(PURPOSE_CELL_ID, 0));
            assert_eq!(quick.cell, id, "round {round}");
        }
    }

    #[test]
    fn the_wider_search_is_needed_now_and_then() {
        // Two positions, found by trying several hundred thousand, whose
        // second nearest point lies outside the near ring. About one
        // position in twenty thousand is like this.
        let domain = HashDomain::new("test.exact");
        let cases = [
            (
                [36, -43, 0],
                [0.700_490_068_645_281_7, 0.036_192_526_284_800_96, 0.0],
                false,
            ),
            (
                [26, 39, -21],
                [
                    0.987_678_560_487_68,
                    0.078_263_667_162_809_08,
                    0.914_935_597_667_612_5,
                ],
                true,
            ),
        ];
        for (cell, within, volume) in cases {
            let within = within.map(Real::of);
            let near = scan(5, domain, cell, within, NEAR_RING, volume);
            let far = scan(5, domain, cell, within, FAR_RING, volume);
            assert!(far.second_sq < near.second_sq);
            assert_eq!(far.nearest_sq, near.nearest_sq);
            let found = cellular(5, domain, cell, within, volume);
            assert_eq!(found.second, far.second_sq.sqrt());
        }
    }

    #[test]
    fn salts_keep_purposes_and_octaves_apart() {
        let mut seen = Vec::new();
        for purpose in [
            PURPOSE_VALUE,
            PURPOSE_GRADIENT,
            PURPOSE_CELL_POINT,
            PURPOSE_CELL_ID,
            PURPOSE_WARP_X,
            PURPOSE_WARP_Y,
            PURPOSE_WARP_Z,
        ] {
            for octave in 0..MAX_OCTAVES {
                seen.push(salt(purpose, octave));
            }
        }
        let total = seen.len();
        seen.sort_unstable();
        seen.dedup();
        assert_eq!(seen.len(), total);
    }
}
