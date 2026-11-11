//! Property tests: the algebra of vectors, rotations and turns, and the
//! promises of `Real`, the sorted helpers, the hasher and the generator.

use std::collections::BTreeSet;

use clash_core::{
    Dir8, Hash128, IVec2, IVec3, LookupMap, Real, Rng, Rot4, StateHasher, Turn, sorted,
};
use proptest::prelude::*;

fn ivec2() -> impl Strategy<Value = IVec2> {
    (any::<i32>(), any::<i32>()).prop_map(|(x, y)| IVec2::new(x, y))
}

/// Vectors small enough that lengths and products do not saturate.
fn small_ivec2() -> impl Strategy<Value = IVec2> {
    (-1_000_000..=1_000_000, -1_000_000..=1_000_000).prop_map(|(x, y)| IVec2::new(x, y))
}

fn ivec3() -> impl Strategy<Value = IVec3> {
    (any::<i32>(), any::<i32>(), any::<i32>()).prop_map(|(x, y, z)| IVec3::new(x, y, z))
}

fn rot4() -> impl Strategy<Value = Rot4> {
    (0..4_usize).prop_map(|index| Rot4::ALL[index])
}

fn turn() -> impl Strategy<Value = Turn> {
    (0..8_usize).prop_map(|index| Turn::ALL[index])
}

fn dir8() -> impl Strategy<Value = Dir8> {
    (0..8_usize).prop_map(|index| Dir8::ALL[index])
}

/// Any `f64` that is not NaN, including infinities, zeros and subnormals.
fn real() -> impl Strategy<Value = Real> {
    any::<f64>().prop_map(Real::of)
}

proptest! {
    // ---- Vectors ---------------------------------------------------------

    #[test]
    fn vec2_addition_is_a_commutative_group(a in ivec2(), b in ivec2(), c in ivec2()) {
        prop_assert_eq!(a + b, b + a);
        prop_assert_eq!((a + b) + c, a + (b + c));
        prop_assert_eq!(a + IVec2::ZERO, a);
        prop_assert_eq!(a + (-a), IVec2::ZERO);
        prop_assert_eq!(a - b, a + (-b));
        prop_assert_eq!((a - b) + b, a);
    }

    #[test]
    fn vec3_addition_is_a_commutative_group(a in ivec3(), b in ivec3(), c in ivec3()) {
        prop_assert_eq!(a + b, b + a);
        prop_assert_eq!((a + b) + c, a + (b + c));
        prop_assert_eq!(a + IVec3::ZERO, a);
        prop_assert_eq!(a + (-a), IVec3::ZERO);
        prop_assert_eq!(a - b, a + (-b));
        prop_assert_eq!((a - b) + b, a);
    }

    #[test]
    fn scaling_distributes_over_addition(a in ivec2(), b in ivec2(), k in any::<i32>(), m in any::<i32>()) {
        prop_assert_eq!((a + b) * k, a * k + b * k);
        prop_assert_eq!(a * k.wrapping_add(m), a * k + a * m);
        prop_assert_eq!(a * 1, a);
        // Written as a call because clippy objects to `a * 0`.
        prop_assert_eq!(std::ops::Mul::mul(a, 0), IVec2::ZERO);
        prop_assert_eq!(a * -1, -a);
    }

    #[test]
    fn scaling_distributes_over_addition_in_3d(a in ivec3(), b in ivec3(), k in any::<i32>()) {
        prop_assert_eq!((a + b) * k, a * k + b * k);
        prop_assert_eq!(a * 1, a);
        prop_assert_eq!(a * -1, -a);
    }

    #[test]
    fn assign_operators_match_the_plain_ones(a in ivec3(), b in ivec3()) {
        let mut sum = a;
        sum += b;
        prop_assert_eq!(sum, a + b);
        let mut difference = a;
        difference -= b;
        prop_assert_eq!(difference, a - b);
        let mut flat = a.xy();
        flat += b.xy();
        prop_assert_eq!(flat, (a + b).xy());
    }

    #[test]
    fn dot_product_is_symmetric_and_bilinear(a in small_ivec2(), b in small_ivec2(), c in small_ivec2()) {
        prop_assert_eq!(a.dot(b), b.dot(a));
        prop_assert_eq!((a + b).dot(c), a.dot(c) + b.dot(c));
        prop_assert_eq!(a.dot(a), a.length_sq());
        prop_assert!(a.length_sq() >= 0);
    }

    #[test]
    fn norms_are_norms(a in small_ivec2(), b in small_ivec2()) {
        prop_assert!(a.chebyshev() <= a.manhattan());
        prop_assert!(a.manhattan() <= 2 * a.chebyshev());
        prop_assert_eq!(a.chebyshev() == 0, a == IVec2::ZERO);
        // The triangle inequality.
        prop_assert!((a + b).manhattan() <= a.manhattan() + b.manhattan());
        prop_assert!((a + b).chebyshev() <= a.chebyshev() + b.chebyshev());
        prop_assert_eq!((-a).manhattan(), a.manhattan());
        prop_assert_eq!((-a).chebyshev(), a.chebyshev());
    }

    #[test]
    fn extremes_never_panic(a in ivec3(), b in ivec3()) {
        let _ = (a.dot(b), a.length_sq(), a.manhattan(), a.chebyshev());
        let _ = (a.xy().dot(b.xy()), a.xy().length_sq());
        prop_assert!(a.chebyshev() >= 0);
        prop_assert!(a.length_sq() >= 0);
    }

    #[test]
    fn component_bounds_bracket_both_points(a in ivec3(), b in ivec3()) {
        let low = a.component_min(b);
        let high = a.component_max(b);
        for (l, h, p, q) in [
            (low.x, high.x, a.x, b.x),
            (low.y, high.y, a.y, b.y),
            (low.z, high.z, a.z, b.z),
        ] {
            prop_assert!(l <= p && p <= h);
            prop_assert!(l <= q && q <= h);
            prop_assert!(l == p || l == q);
            prop_assert!(h == p || h == q);
        }
        prop_assert_eq!(a.xy().component_min(b.xy()), low.xy());
        prop_assert_eq!(a.xy().component_max(b.xy()), high.xy());
    }

    #[test]
    fn splitting_and_joining_height_round_trips(a in ivec3()) {
        prop_assert_eq!(a.xy().with_z(a.z), a);
    }

    // ---- Rotations -------------------------------------------------------

    #[test]
    fn rotations_form_a_cyclic_group(a in rot4(), b in rot4(), c in rot4()) {
        prop_assert_eq!(a.then(b), b.then(a));
        prop_assert_eq!(a.then(b).then(c), a.then(b.then(c)));
        prop_assert_eq!(a.then(Rot4::R0), a);
        prop_assert_eq!(a.then(a.inverse()), Rot4::R0);
        prop_assert_eq!(a.then(a).then(a).then(a), Rot4::R0);
        prop_assert_eq!(Rot4::from_quarter_turns(a.quarter_turns()), a);
    }

    #[test]
    fn rotation_is_linear_and_keeps_length(a in rot4(), v in small_ivec2(), w in small_ivec2()) {
        prop_assert_eq!(a.apply(v + w), a.apply(v) + a.apply(w));
        prop_assert_eq!(a.apply(-v), -a.apply(v));
        prop_assert_eq!(a.apply(v).length_sq(), v.length_sq());
        prop_assert_eq!(a.apply(v).dot(a.apply(w)), v.dot(w));
        prop_assert_eq!(v.rotated(a), a.apply(v));
    }

    #[test]
    fn composing_rotations_composes_their_effect(a in rot4(), b in rot4(), v in ivec2()) {
        prop_assert_eq!(a.then(b).apply(v), b.apply(a.apply(v)));
        prop_assert_eq!(a.inverse().apply(a.apply(v)), v);
    }

    #[test]
    fn a_quarter_turn_is_perpendicular(v in small_ivec2()) {
        prop_assert_eq!(v.dot(Rot4::R90.apply(v)), 0);
        prop_assert_eq!(Rot4::R180.apply(v), -v);
    }

    #[test]
    fn any_count_of_quarter_turns_is_accepted(turns in any::<i32>(), v in small_ivec2()) {
        let rot = Rot4::from_quarter_turns(turns);
        prop_assert_eq!(rot, Rot4::from_quarter_turns(turns.rem_euclid(4)));
        prop_assert_eq!(rot.apply(v).length_sq(), v.length_sq());
    }

    #[test]
    fn rotating_in_3d_leaves_height_alone(a in rot4(), t in turn(), v in ivec3()) {
        prop_assert_eq!(v.rotated(a).z, v.z);
        prop_assert_eq!(v.rotated(a).xy(), a.apply(v.xy()));
        prop_assert_eq!(v.turned(t).z, v.z);
        prop_assert_eq!(v.turned(t).xy(), t.apply(v.xy()));
    }

    // ---- Turns: rotations with an optional mirror ------------------------

    #[test]
    fn turns_form_a_group(a in turn(), b in turn(), c in turn()) {
        prop_assert_eq!(a.then(b).then(c), a.then(b.then(c)));
        prop_assert_eq!(a.then(Turn::IDENTITY), a);
        prop_assert_eq!(Turn::IDENTITY.then(a), a);
        prop_assert_eq!(a.then(a.inverse()), Turn::IDENTITY);
        prop_assert_eq!(a.inverse().then(a), Turn::IDENTITY);
        prop_assert_eq!(a.inverse().inverse(), a);
        prop_assert_eq!(a.then(b).inverse(), b.inverse().then(a.inverse()));
    }

    #[test]
    fn composing_turns_composes_their_effect(a in turn(), b in turn(), v in ivec2()) {
        prop_assert_eq!(a.then(b).apply(v), b.apply(a.apply(v)));
        prop_assert_eq!(a.inverse().apply(a.apply(v)), v);
        prop_assert_eq!(v.turned(a), a.apply(v));
    }

    #[test]
    fn turning_is_linear_and_keeps_length(a in turn(), v in small_ivec2(), w in small_ivec2()) {
        prop_assert_eq!(a.apply(v + w), a.apply(v) + a.apply(w));
        prop_assert_eq!(a.apply(v).length_sq(), v.length_sq());
        prop_assert_eq!(a.apply(v).dot(a.apply(w)), v.dot(w));
    }

    #[test]
    fn a_mirror_reverses_the_sense_of_rotation(a in rot4(), v in small_ivec2(), w in small_ivec2()) {
        let mirror = Turn::new(Rot4::R0, true);
        // Mirror, rotate, mirror back: the same rotation the other way.
        prop_assert_eq!(
            mirror.then(Turn::from(a)).then(mirror),
            Turn::from(a.inverse())
        );
        // A turn without a mirror keeps handedness; one with a mirror flips it.
        let cross = |p: IVec2, q: IVec2| p.x as i64 * q.y as i64 - p.y as i64 * q.x as i64;
        prop_assert_eq!(cross(a.apply(v), a.apply(w)), cross(v, w));
        prop_assert_eq!(cross(mirror.apply(v), mirror.apply(w)), -cross(v, w));
    }

    #[test]
    fn a_pure_rotation_is_a_turn_without_a_mirror(a in rot4(), b in rot4(), v in ivec2()) {
        prop_assert_eq!(Turn::from(a).apply(v), a.apply(v));
        prop_assert_eq!(Turn::from(a).then(Turn::from(b)), Turn::from(a.then(b)));
        prop_assert_eq!(Turn::from(a).inverse(), Turn::from(a.inverse()));
    }

    // ---- Directions ------------------------------------------------------

    #[test]
    fn directions_turn_with_their_deltas(d in dir8(), a in rot4(), t in turn()) {
        prop_assert_eq!(d.rotated(a).delta(), a.apply(d.delta()));
        prop_assert_eq!(d.turned(t).delta(), t.apply(d.delta()));
        prop_assert_eq!(d.turned(t).turned(t.inverse()), d);
        prop_assert_eq!(d.rotated(a).is_diagonal(), d.is_diagonal());
        prop_assert_eq!(Dir8::from_delta(d.delta()), Some(d));
        prop_assert_eq!(d.opposite().delta(), -d.delta());
    }

    #[test]
    fn only_single_steps_are_directions(v in ivec2()) {
        let is_step = v.chebyshev() == 1;
        prop_assert_eq!(Dir8::from_delta(v).is_some(), is_step);
    }

    // ---- Real ------------------------------------------------------------

    #[test]
    fn real_arithmetic_never_makes_nan(a in real(), b in real()) {
        let results = [
            a + b, a - b, a * b, a / b, -a,
            a.sqrt(), a.cbrt(), a.exp(), a.ln(), a.log10(),
            a.sin(), a.cos(), a.tan(), a.atan2(b), a.powf(b), a.powi(3), a.powi(-2),
            a.floor(), a.ceil(), a.round(), a.trunc(), a.abs(), a.signum(),
            a.min(b), a.max(b), a.clamp(b, a),
        ];
        for value in results {
            prop_assert!(!value.get().is_nan());
        }
    }

    #[test]
    fn real_order_is_total_and_consistent(a in real(), b in real(), c in real()) {
        prop_assert_eq!(a.cmp(&b), b.cmp(&a).reverse());
        prop_assert_eq!(a.partial_cmp(&b), Some(a.cmp(&b)));
        prop_assert_eq!(a == b, a.cmp(&b) == std::cmp::Ordering::Equal);
        if a <= b && b <= c {
            prop_assert!(a <= c);
        }
        prop_assert!(a.min(b) <= a.max(b));
        prop_assert!(a.min(b) == a || a.min(b) == b);
        prop_assert!(a.max(b) == a || a.max(b) == b);
    }

    #[test]
    fn real_matches_ieee_where_ieee_is_defined(a in real(), b in real()) {
        let (x, y) = (a.get(), b.get());
        for (ours, ieee) in [(a + b, x + y), (a - b, x - y), (a * b, x * y), (a / b, x / y)] {
            if ieee.is_nan() {
                prop_assert_eq!(ours, Real::ZERO);
            } else {
                prop_assert_eq!(ours.get().to_bits(), ieee.to_bits());
            }
        }
    }

    #[test]
    fn real_try_functions_agree_with_the_plain_ones(a in real(), b in real()) {
        if let Ok(value) = a.try_div(b) {
            prop_assert_eq!(value, a / b);
        } else {
            prop_assert!(b == Real::ZERO || (!a.is_finite() && !b.is_finite()));
        }
        prop_assert_eq!(a.try_sqrt().is_ok(), a >= Real::ZERO);
        if let Ok(value) = a.try_sqrt() {
            prop_assert_eq!(value, a.sqrt());
        }
        prop_assert_eq!(a.try_ln().is_ok(), a > Real::ZERO);
        prop_assert_eq!(a.try_log10().is_ok(), a > Real::ZERO);
        if let Ok(value) = a.try_ln() {
            prop_assert_eq!(value, a.ln());
        }
        if let Ok(value) = a.try_powf(b) {
            prop_assert_eq!(value, a.powf(b));
        }
    }

    #[test]
    fn real_clamp_lands_in_range(a in real(), low in real(), high in real()) {
        let clamped = a.clamp(low, high);
        if low <= high {
            prop_assert!(low <= clamped && clamped <= high);
            if low <= a && a <= high {
                prop_assert_eq!(clamped, a);
            }
        } else {
            prop_assert_eq!(clamped, high);
        }
    }

    #[test]
    fn real_rounding_brackets_the_value(a in -1.0e15_f64..1.0e15) {
        let a = Real::of(a);
        prop_assert!(a.floor() <= a && a <= a.ceil());
        prop_assert!(a.ceil() - a.floor() <= Real::ONE);
        prop_assert!((a.round() - a).abs() <= Real::of(0.5));
        prop_assert!(a.trunc().abs() <= a.abs());
        prop_assert_eq!(a.floor().to_i64(), a.get().floor() as i64);
    }

    // ---- Sorted helpers --------------------------------------------------

    #[test]
    fn sorted_vec_behaves_like_a_set(operations in prop::collection::vec((any::<bool>(), 0..40_u8), 0..200)) {
        let mut items: Vec<u8> = Vec::new();
        let mut model: BTreeSet<u8> = BTreeSet::new();
        for (insert, value) in operations {
            if insert {
                prop_assert_eq!(sorted::insert(&mut items, value), model.insert(value));
            } else {
                prop_assert_eq!(sorted::remove(&mut items, &value), model.remove(&value));
            }
            prop_assert_eq!(sorted::contains(&items, &value), model.contains(&value));
            prop_assert!(sorted::is_sorted_set(&items));
        }
        prop_assert_eq!(items, model.into_iter().collect::<Vec<u8>>());
    }

    #[test]
    fn from_unsorted_sorts_and_removes_duplicates(items in prop::collection::vec(any::<u8>(), 0..100)) {
        let set: BTreeSet<u8> = items.iter().copied().collect();
        let made = sorted::from_unsorted(items);
        prop_assert!(sorted::is_sorted_set(&made));
        prop_assert_eq!(made, set.into_iter().collect::<Vec<u8>>());
    }

    // ---- LookupMap -------------------------------------------------------

    #[test]
    fn lookup_map_contents_do_not_depend_on_history(
        entries in prop::collection::btree_map(any::<u16>(), any::<u32>(), 0..60),
        extra in prop::collection::vec(any::<u16>(), 0..20),
        seed in any::<u64>(),
    ) {
        let in_order: LookupMap<u16, u32> = entries.iter().map(|(k, v)| (*k, *v)).collect();

        // The same contents reached another way: shuffled, with extra keys
        // inserted and removed again along the way.
        let mut pairs: Vec<(u16, u32)> = entries.iter().map(|(k, v)| (*k, *v)).collect();
        Rng::from_seed(seed).shuffle(&mut pairs);
        let mut other: LookupMap<u16, u32> = LookupMap::new();
        for key in &extra {
            other.insert(*key, 0);
        }
        for key in &extra {
            other.remove(key);
        }
        for (key, value) in pairs {
            other.insert(key, value);
        }

        prop_assert_eq!(&in_order, &other);
        prop_assert_eq!(in_order.len(), entries.len());
        prop_assert_eq!(Hash128::of(&in_order), Hash128::of(&other));
        prop_assert_eq!(in_order.sorted_keys(), entries.keys().copied().collect::<Vec<u16>>());
        prop_assert_eq!(
            other.into_sorted_vec(),
            entries.into_iter().collect::<Vec<(u16, u32)>>()
        );
    }

    // ---- Hasher ----------------------------------------------------------

    #[test]
    fn hasher_depends_on_the_bytes_not_on_how_they_arrive(
        data in prop::collection::vec(any::<u8>(), 0..300),
        cuts in prop::collection::vec(any::<prop::sample::Index>(), 0..6),
    ) {
        let mut whole = StateHasher::new();
        whole.write_bytes(&data);

        let mut points: Vec<usize> = cuts.iter().map(|cut| cut.index(data.len() + 1)).collect();
        points.sort_unstable();
        let mut pieces = StateHasher::new();
        let mut start = 0;
        for point in points {
            pieces.write_bytes(&data[start..point]);
            start = point;
        }
        pieces.write_bytes(&data[start..]);

        prop_assert_eq!(pieces.finish(), whole.finish());
    }

    #[test]
    fn hasher_notices_any_single_byte_change(
        data in prop::collection::vec(any::<u8>(), 1..200),
        at in any::<prop::sample::Index>(),
        flip in 1..=255_u8,
    ) {
        let mut changed = data.clone();
        let position = at.index(data.len());
        changed[position] ^= flip;
        let hash = |bytes: &[u8]| {
            let mut hasher = StateHasher::new();
            hasher.write_bytes(bytes);
            hasher.finish()
        };
        prop_assert_ne!(hash(&data), hash(&changed));
    }

    // ---- Generator -------------------------------------------------------

    #[test]
    fn rng_draws_stay_in_range(seed in any::<u64>(), bound in 1..=u64::MAX, low in any::<i64>(), high in any::<i64>()) {
        let mut rng = Rng::from_seed(seed);
        prop_assert!(rng.below(bound) < bound);
        let value = rng.range(low, high);
        if low <= high {
            prop_assert!(low <= value && value <= high);
        } else {
            prop_assert_eq!(value, low);
        }
        let unit = rng.unit();
        prop_assert!(unit >= Real::ZERO && unit < Real::ONE);
    }

    #[test]
    fn rng_shuffle_is_a_permutation(seed in any::<u64>(), length in 0..64_usize) {
        let mut items: Vec<usize> = (0..length).collect();
        Rng::from_seed(seed).shuffle(&mut items);
        items.sort_unstable();
        prop_assert_eq!(items, (0..length).collect::<Vec<usize>>());
    }

    #[test]
    fn rng_state_round_trip_continues_the_sequence(seed in any::<u64>(), skip in 0..50_usize) {
        let mut rng = Rng::from_seed(seed);
        for _ in 0..skip {
            rng.next_u64();
        }
        let mut restored = Rng::from_state(rng.state()).unwrap();
        for _ in 0..8 {
            prop_assert_eq!(restored.next_u64(), rng.next_u64());
        }
    }
}
