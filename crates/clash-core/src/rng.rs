//! Deterministic randomness: the positional hash, the generator and the
//! named streams.
//!
//! The engine has exactly two sources of randomness (01 §4), and neither
//! reads the operating system, a thread-local generator or the clock:
//!
//! - **Positional**, for generation. [`pos_hash`] is a stateless hash of
//!   `(seed, domain, coordinates, salt)`. The result depends only on where
//!   and what is being generated, never on when or in what order, so a
//!   world comes out the same however it is explored. [`pos_rng`] seeds a
//!   local [`Rng`] from it when more than one number is needed.
//! - **Streams**, for simulation. [`RngStreams`] is a set of named
//!   generators that lives in simulation state and is saved with it. Each
//!   system draws from its own stream, so adding a draw in one system does
//!   not change what another sees.
//!
//! Every algorithm here is fixed, and the reference vectors in this
//! crate's tests pin each one down. Changing any of them changes every
//! generated world and every golden hash.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::Real;
use crate::error::RngError;
use crate::hash::{StateHash, StateHasher};

/// The odd constant nearest to 2⁶⁴ divided by the golden ratio. Adding it
/// repeatedly visits every 64-bit value once (the SplitMix64 increment).
const GOLDEN_GAMMA: u64 = 0x9e37_79b9_7f4a_7c15;

/// Separates the seeds of named streams from positional hashes that happen
/// to use the same seed and domain name. The ASCII bytes of "stream".
const STREAM_SALT: u64 = 0x7374_7265_616d;

/// 2⁻⁵³: the spacing of the 2⁵³ evenly spread values [`Rng::unit`] returns.
const UNIT_STEP: f64 = 1.0 / 9_007_199_254_740_992.0;

/// The SplitMix64 output function: a bijection on `u64` in which every
/// input bit affects every output bit.
const fn mix64(mut z: u64) -> u64 {
    z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    z ^ (z >> 31)
}

/// FNV-1a, 64 bits: turns a name into a number at compile time.
const fn fnv1a64(text: &str) -> u64 {
    let bytes = text.as_bytes();
    let mut hash = 0xcbf2_9ce4_8422_2325_u64;
    let mut at = 0;
    while at < bytes.len() {
        hash ^= bytes[at] as u64;
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
        at += 1;
    }
    hash
}

/// What a positional hash is for.
///
/// Two uses of [`pos_hash`] with different domains are unrelated even at
/// the same seed and position, so the choice of ore in a cell says nothing
/// about the choice of tree there. Make one constant per use, named for
/// it:
///
/// ```
/// use clash_core::{pos_hash, HashDomain};
///
/// const ORE: HashDomain = HashDomain::new("gen.ore");
/// const TREES: HashDomain = HashDomain::new("gen.trees");
///
/// let seed = 42;
/// assert_ne!(pos_hash(seed, ORE, 10, 20, 0, 0), pos_hash(seed, TREES, 10, 20, 0, 0));
/// ```
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub struct HashDomain(u64);

impl HashDomain {
    /// The domain with this name. The number is the 64-bit FNV-1a hash of
    /// the name's bytes, so a name always gives the same domain.
    pub const fn new(name: &str) -> HashDomain {
        HashDomain(fnv1a64(name))
    }

    /// The domain with this number, for a domain that is not a fixed name:
    /// a definition's id, for example.
    pub const fn from_raw(raw: u64) -> HashDomain {
        HashDomain(raw)
    }

    /// The domain's number.
    pub const fn raw(self) -> u64 {
        self.0
    }
}

/// The positional hash: a stateless, well-mixed function of a seed, a
/// domain, a cell position and a salt.
///
/// Use it for anything decided by *where* rather than *when*. `salt` tells
/// apart several independent choices at one position.
///
/// The algorithm is fixed. With `mix` the SplitMix64 output function and
/// `γ` = `0x9e3779b97f4a7c15`, all arithmetic wrapping:
///
/// ```text
/// h = mix(seed + γ)
/// h = mix((h ^ domain) + γ)
/// h = mix((h ^ (x as u32 | (y as u32) << 32)) + γ)
/// h = mix((h ^ (z as u32)) + γ)
/// h = mix((h ^ salt) + γ)
/// ```
pub const fn pos_hash(seed: u64, domain: HashDomain, x: i32, y: i32, z: i32, salt: u64) -> u64 {
    let column = (x as u32 as u64) | ((y as u32 as u64) << 32);
    let mut h = mix64(seed.wrapping_add(GOLDEN_GAMMA));
    h = mix64((h ^ domain.0).wrapping_add(GOLDEN_GAMMA));
    h = mix64((h ^ column).wrapping_add(GOLDEN_GAMMA));
    h = mix64((h ^ (z as u32 as u64)).wrapping_add(GOLDEN_GAMMA));
    mix64((h ^ salt).wrapping_add(GOLDEN_GAMMA))
}

/// A generator seeded by [`pos_hash`], for when one position needs several
/// random numbers.
///
/// ```
/// use clash_core::{pos_rng, HashDomain};
///
/// const LOOT: HashDomain = HashDomain::new("gen.loot");
///
/// let mut first = pos_rng(7, LOOT, 3, 4, 0, 0);
/// let mut again = pos_rng(7, LOOT, 3, 4, 0, 0);
/// assert_eq!(first.below(100), again.below(100));
/// ```
pub fn pos_rng(seed: u64, domain: HashDomain, x: i32, y: i32, z: i32, salt: u64) -> Rng {
    Rng::from_seed(pos_hash(seed, domain, x, y, z, salt))
}

/// A small, fast pseudo-random generator with a fixed algorithm.
///
/// The algorithm is xoshiro256\*\* by Blackman and Vigna, and
/// [`from_seed`](Rng::from_seed) fills its state with SplitMix64, as its
/// authors recommend. It is not for cryptography.
///
/// An `Rng` is deliberately not `Copy`: a copy would repeat the original's
/// numbers. Clone one only to look ahead.
///
/// ```
/// use clash_core::{Real, Rng};
///
/// let mut rng = Rng::from_seed(2024);
/// let roll = rng.range(1, 6);
/// assert!((1..=6).contains(&roll));
/// let share = rng.unit();
/// assert!(share >= Real::ZERO && share < Real::ONE);
/// ```
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
#[serde(try_from = "[u64; 4]", into = "[u64; 4]")]
pub struct Rng {
    state: [u64; 4],
}

impl Rng {
    /// A generator for a seed. Every seed is valid, and seeds that differ
    /// in a single bit give unrelated sequences.
    pub const fn from_seed(seed: u64) -> Rng {
        let mut state = [0_u64; 4];
        let mut counter = seed;
        let mut at = 0;
        while at < state.len() {
            counter = counter.wrapping_add(GOLDEN_GAMMA);
            state[at] = mix64(counter);
            at += 1;
        }
        Rng { state }
    }

    /// Restores a generator from a state taken by [`state`](Rng::state).
    /// A state of all zeros is refused: it would produce only zeros.
    pub const fn from_state(state: [u64; 4]) -> Result<Rng, RngError> {
        if state[0] == 0 && state[1] == 0 && state[2] == 0 && state[3] == 0 {
            Err(RngError::ZeroState)
        } else {
            Ok(Rng { state })
        }
    }

    /// The generator's whole state.
    pub const fn state(&self) -> [u64; 4] {
        self.state
    }

    /// The next 64 random bits.
    pub fn next_u64(&mut self) -> u64 {
        let [s0, s1, s2, s3] = &mut self.state;
        let result = s1.wrapping_mul(5).rotate_left(7).wrapping_mul(9);
        let shifted = *s1 << 17;
        *s2 ^= *s0;
        *s3 ^= *s1;
        *s1 ^= *s2;
        *s0 ^= *s3;
        *s2 ^= shifted;
        *s3 = s3.rotate_left(45);
        result
    }

    /// The next 32 random bits: the upper half of
    /// [`next_u64`](Rng::next_u64).
    pub fn next_u32(&mut self) -> u32 {
        (self.next_u64() >> 32) as u32
    }

    /// A whole number from `0` up to but not including `bound`, each
    /// equally likely. A `bound` of zero gives zero and draws nothing.
    ///
    /// Uses Lemire's multiply-and-reject method, so it usually costs one
    /// draw and occasionally more.
    pub fn below(&mut self, bound: u64) -> u64 {
        if bound == 0 {
            return 0;
        }
        let mut product = u128::from(self.next_u64()) * u128::from(bound);
        if (product as u64) < bound {
            let threshold = bound.wrapping_neg() % bound;
            while (product as u64) < threshold {
                product = u128::from(self.next_u64()) * u128::from(bound);
            }
        }
        (product >> 64) as u64
    }

    /// A whole number from `low` to `high`, both included, each equally
    /// likely. If `high` is less than `low` the result is `low` and
    /// nothing is drawn.
    pub fn range(&mut self, low: i64, high: i64) -> i64 {
        if high < low {
            return low;
        }
        let span = (high.wrapping_sub(low) as u64).wrapping_add(1);
        if span == 0 {
            // The whole of i64: every bit pattern is in range.
            return self.next_u64() as i64;
        }
        low.wrapping_add(self.below(span) as i64)
    }

    /// A number from zero up to but not including one, one of 2⁵³ evenly
    /// spread values. Always one draw.
    pub fn unit(&mut self) -> Real {
        Real::of((self.next_u64() >> 11) as f64 * UNIT_STEP)
    }

    /// `true` with the given probability. Zero or less is never; one or
    /// more is always. Always one draw, whatever the probability, so the
    /// stream stays in step.
    pub fn chance(&mut self, probability: Real) -> bool {
        self.unit() < probability
    }

    /// One item of a slice, each equally likely, or `None` if the slice is
    /// empty.
    pub fn pick<'a, T>(&mut self, items: &'a [T]) -> Option<&'a T> {
        if items.is_empty() {
            None
        } else {
            items.get(self.below(items.len() as u64) as usize)
        }
    }

    /// Puts a slice in random order, every order equally likely (the
    /// Fisher–Yates shuffle, from the last item down).
    pub fn shuffle<T>(&mut self, items: &mut [T]) {
        for last in (1..items.len()).rev() {
            let other = self.below(last as u64 + 1) as usize;
            items.swap(last, other);
        }
    }

    /// A new generator seeded from this one's next number. Use it to hand
    /// a job its own randomness without letting the job disturb this
    /// sequence further.
    pub fn fork(&mut self) -> Rng {
        Rng::from_seed(self.next_u64())
    }
}

impl TryFrom<[u64; 4]> for Rng {
    type Error = RngError;
    fn try_from(state: [u64; 4]) -> Result<Rng, RngError> {
        Rng::from_state(state)
    }
}

impl From<Rng> for [u64; 4] {
    fn from(rng: Rng) -> [u64; 4] {
        rng.state
    }
}

impl StateHash for Rng {
    fn state_hash(&self, hasher: &mut StateHasher) {
        self.state.state_hash(hasher);
    }
}

/// The simulation's named random streams.
///
/// A stream is an [`Rng`] with a name. It is created the first time it is
/// asked for, seeded from the set's seed and the stream's name alone, so
/// what a stream produces does not depend on which other streams exist or
/// on the order they were first used. The whole set is part of simulation
/// state: it is saved, loaded and hashed.
///
/// ```
/// use clash_core::RngStreams;
///
/// let mut streams = RngStreams::new(99);
/// let hit = streams.stream("combat").below(20);
///
/// // Another system drawing from its own stream changes nothing here.
/// let mut other = RngStreams::new(99);
/// other.stream("weather").next_u64();
/// assert_eq!(other.stream("combat").below(20), hit);
/// ```
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub struct RngStreams {
    seed: u64,
    streams: BTreeMap<String, Rng>,
}

impl RngStreams {
    /// A set of streams for a seed, with no stream yet created.
    pub const fn new(seed: u64) -> RngStreams {
        RngStreams {
            seed,
            streams: BTreeMap::new(),
        }
    }

    /// The seed the set was made with.
    pub const fn seed(&self) -> u64 {
        self.seed
    }

    /// The stream with this name, created if this is its first use.
    ///
    /// A new stream's generator is
    /// `Rng::from_seed(pos_hash(seed, HashDomain::new(name), 0, 0, 0, s))`
    /// where `s` is a fixed salt that keeps streams apart from positional
    /// hashes.
    pub fn stream(&mut self, name: &str) -> &mut Rng {
        if !self.streams.contains_key(name) {
            let seed = pos_hash(self.seed, HashDomain::new(name), 0, 0, 0, STREAM_SALT);
            self.streams.insert(name.to_owned(), Rng::from_seed(seed));
        }
        self.streams
            .get_mut(name)
            .expect("the stream was inserted just above")
    }

    /// The names of the streams created so far, in alphabetical order.
    pub fn names(&self) -> impl Iterator<Item = &str> {
        self.streams.keys().map(String::as_str)
    }
}

impl StateHash for RngStreams {
    fn state_hash(&self, hasher: &mut StateHasher) {
        self.seed.state_hash(hasher);
        self.streams.state_hash(hasher);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Hash128;

    const DOMAIN: HashDomain = HashDomain::new("test.domain");

    #[test]
    fn every_input_of_the_positional_hash_matters() {
        let base = pos_hash(1, DOMAIN, 2, 3, 4, 5);
        assert_eq!(base, pos_hash(1, DOMAIN, 2, 3, 4, 5));
        let changed = [
            pos_hash(2, DOMAIN, 2, 3, 4, 5),
            pos_hash(1, HashDomain::new("test.other"), 2, 3, 4, 5),
            pos_hash(1, DOMAIN, 3, 3, 4, 5),
            pos_hash(1, DOMAIN, 2, 4, 4, 5),
            pos_hash(1, DOMAIN, 2, 3, 5, 5),
            pos_hash(1, DOMAIN, 2, 3, 4, 6),
            // Swapped coordinates are different places.
            pos_hash(1, DOMAIN, 3, 2, 4, 5),
            pos_hash(1, DOMAIN, 4, 3, 2, 5),
            pos_hash(1, DOMAIN, -2, 3, 4, 5),
        ];
        for (index, value) in changed.iter().enumerate() {
            assert_ne!(*value, base, "case {index}");
        }
    }

    #[test]
    fn neighbouring_cells_are_unrelated() {
        // Moving one cell should flip about half of the 64 output bits.
        let mut flipped = 0_u32;
        let mut pairs = 0_u32;
        for y in -16..16 {
            for x in -16..16 {
                let here = pos_hash(77, DOMAIN, x, y, 0, 0);
                for (dx, dy, dz) in [(1, 0, 0), (0, 1, 0), (0, 0, 1)] {
                    let there = pos_hash(77, DOMAIN, x + dx, y + dy, dz, 0);
                    flipped += (here ^ there).count_ones();
                    pairs += 1;
                }
            }
        }
        let mean = f64::from(flipped) / f64::from(pairs);
        assert!((31.0..33.0).contains(&mean), "mean flipped bits {mean}");
    }

    #[test]
    fn positional_hashes_do_not_collide_over_a_region() {
        let mut seen = Vec::new();
        for z in 0..4 {
            for y in -32..32 {
                for x in -32..32 {
                    seen.push(pos_hash(5, DOMAIN, x, y, z, 0));
                }
            }
        }
        let total = seen.len();
        seen.sort_unstable();
        seen.dedup();
        assert_eq!(seen.len(), total);
    }

    #[test]
    fn domains_are_named_or_numbered() {
        assert_eq!(HashDomain::new("a"), HashDomain::new("a"));
        assert_ne!(HashDomain::new("a"), HashDomain::new("b"));
        assert_eq!(HashDomain::from_raw(17).raw(), 17);
        assert_eq!(HashDomain::new("a").raw(), 0xaf63_dc4c_8601_ec8c);
    }

    #[test]
    fn the_same_seed_gives_the_same_sequence() {
        let mut a = Rng::from_seed(123);
        let mut b = Rng::from_seed(123);
        let mut c = Rng::from_seed(124);
        let from_a: Vec<u64> = (0..16).map(|_| a.next_u64()).collect();
        let from_b: Vec<u64> = (0..16).map(|_| b.next_u64()).collect();
        let from_c: Vec<u64> = (0..16).map(|_| c.next_u64()).collect();
        assert_eq!(from_a, from_b);
        assert_ne!(from_a, from_c);
    }

    #[test]
    fn below_stays_under_its_bound_and_reaches_every_value() {
        let mut rng = Rng::from_seed(1);
        assert_eq!(rng.below(0), 0);
        assert_eq!(rng.below(1), 0);
        let mut counts = [0_u32; 6];
        for _ in 0..6_000 {
            counts[rng.below(6) as usize] += 1;
        }
        for count in counts {
            assert!((850..1150).contains(&count), "counts {counts:?}");
        }
        for _ in 0..1_000 {
            assert!(rng.below(u64::MAX) < u64::MAX);
            assert!(rng.below((1 << 63) + 1) <= 1 << 63);
        }
    }

    #[test]
    fn below_zero_draws_nothing() {
        let mut rng = Rng::from_seed(8);
        let before = rng.clone();
        assert_eq!(rng.below(0), 0);
        assert_eq!(rng, before);
    }

    #[test]
    fn range_includes_both_ends() {
        let mut rng = Rng::from_seed(2);
        let mut seen = [false; 5];
        for _ in 0..500 {
            let value = rng.range(-2, 2);
            assert!((-2..=2).contains(&value));
            seen[(value + 2) as usize] = true;
        }
        assert_eq!(seen, [true; 5]);
        assert_eq!(rng.range(7, 7), 7);
        assert_eq!(rng.range(7, 3), 7);
        // The widest ranges do not overflow.
        let _ = rng.range(i64::MIN, i64::MAX);
        let wide = rng.range(i64::MIN, i64::MAX - 1);
        assert!(wide < i64::MAX);
        let high = rng.range(i64::MAX - 1, i64::MAX);
        assert!(high >= i64::MAX - 1);
    }

    #[test]
    fn unit_is_in_the_half_open_interval() {
        let mut rng = Rng::from_seed(3);
        let mut total = Real::ZERO;
        for _ in 0..10_000 {
            let value = rng.unit();
            assert!(value >= Real::ZERO && value < Real::ONE);
            total += value;
        }
        let mean = total.get() / 10_000.0;
        assert!((0.48..0.52).contains(&mean), "mean {mean}");
    }

    #[test]
    fn chance_respects_certainty_and_always_draws_once() {
        let mut rng = Rng::from_seed(4);
        for _ in 0..200 {
            assert!(rng.chance(Real::ONE));
            assert!(!rng.chance(Real::ZERO));
            assert!(rng.chance(Real::of(2.0)));
            assert!(!rng.chance(Real::of(-1.0)));
        }
        let mut hits = 0;
        for _ in 0..10_000 {
            if rng.chance(Real::of(0.25)) {
                hits += 1;
            }
        }
        assert!((2_300..2_700).contains(&hits), "hits {hits}");

        let mut a = Rng::from_seed(5);
        let mut b = Rng::from_seed(5);
        a.chance(Real::ZERO);
        b.next_u64();
        assert_eq!(a, b);
    }

    #[test]
    fn pick_and_shuffle() {
        let mut rng = Rng::from_seed(6);
        assert_eq!(rng.pick::<u8>(&[]), None);
        assert_eq!(rng.pick(&[9]), Some(&9));
        let items = [1_usize, 2, 3, 4];
        let mut seen = [false; 4];
        for _ in 0..200 {
            let item = *rng.pick(&items).unwrap();
            seen[item - 1] = true;
        }
        assert_eq!(seen, [true; 4]);

        let mut deck: Vec<u32> = (0..52).collect();
        rng.shuffle(&mut deck);
        assert_ne!(deck, (0..52).collect::<Vec<u32>>());
        let mut sorted = deck.clone();
        sorted.sort_unstable();
        assert_eq!(sorted, (0..52).collect::<Vec<u32>>());

        let mut empty: [u8; 0] = [];
        rng.shuffle(&mut empty);
        let mut single = [1];
        rng.shuffle(&mut single);
        assert_eq!(single, [1]);
    }

    #[test]
    fn a_fork_is_its_own_sequence() {
        let mut parent = Rng::from_seed(9);
        let mut child = parent.fork();
        let mut twin = Rng::from_seed(9);
        twin.next_u64();
        // The parent moved on by exactly one draw.
        assert_eq!(parent, twin);
        assert_ne!(child.next_u64(), parent.next_u64());
    }

    #[test]
    fn state_round_trips_and_zero_state_is_refused() {
        let mut rng = Rng::from_seed(10);
        rng.next_u64();
        let mut restored = Rng::from_state(rng.state()).unwrap();
        assert_eq!(restored.next_u64(), rng.next_u64());
        assert_eq!(Rng::from_state([0; 4]), Err(RngError::ZeroState));

        let json = serde_json::to_string(&rng).unwrap();
        let mut loaded: Rng = serde_json::from_str(&json).unwrap();
        assert_eq!(loaded, rng);
        assert_eq!(loaded.next_u64(), rng.next_u64());
        assert!(serde_json::from_str::<Rng>("[0,0,0,0]").is_err());
        assert_eq!(Hash128::of(&rng), Hash128::of(&rng.state()));
    }

    #[test]
    fn streams_do_not_depend_on_each_other() {
        let mut alone = RngStreams::new(11);
        let expected: Vec<u64> = (0..4).map(|_| alone.stream("b").next_u64()).collect();

        let mut busy = RngStreams::new(11);
        busy.stream("a").next_u64();
        busy.stream("c").next_u64();
        let mut got = vec![busy.stream("b").next_u64()];
        busy.stream("a").next_u64();
        got.extend((0..3).map(|_| busy.stream("b").next_u64()));
        assert_eq!(got, expected);

        let mut other_seed = RngStreams::new(12);
        assert_ne!(other_seed.stream("b").next_u64(), expected[0]);
        assert_ne!(alone.stream("a").next_u64(), alone.stream("c").next_u64());
        assert_eq!(busy.names().collect::<Vec<_>>(), ["a", "b", "c"]);
        assert_eq!(busy.seed(), 11);
    }

    #[test]
    fn streams_save_load_and_hash() {
        let mut streams = RngStreams::new(13);
        streams.stream("rules").next_u64();
        streams.stream("combat").below(6);

        let json = serde_json::to_string(&streams).unwrap();
        let mut loaded: RngStreams = serde_json::from_str(&json).unwrap();
        assert_eq!(loaded, streams);
        assert_eq!(Hash128::of(&loaded), Hash128::of(&streams));
        assert_eq!(
            loaded.stream("rules").next_u64(),
            streams.stream("rules").next_u64()
        );

        // A draw changes the hash; so does a stream coming into being.
        let before = Hash128::of(&streams);
        streams.stream("combat").next_u64();
        let after_draw = Hash128::of(&streams);
        assert_ne!(after_draw, before);
        streams.stream("new");
        assert_ne!(Hash128::of(&streams), after_draw);
    }
}
