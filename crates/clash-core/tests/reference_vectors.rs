//! Reference vectors: the cross-platform contract of `clash-core`.
//!
//! Every value here must come out bit for bit on every supported target
//! (x86-64 and aarch64) and with every toolchain. A failure in this file
//! means saves, replays and golden hashes made elsewhere will not match:
//! fix the cause, never the vector. Changing a vector on purpose changes
//! every generated world and every golden hash, and needs a decision entry.
//!
//! Where the vectors come from:
//!
//! - **The hasher.** MurmurHash3 `x64_128`, seed 0, computed by the
//!   independent `mmh3` implementation over the same bytes. The typed
//!   vectors hash the byte encodings that the `StateHash` documentation
//!   promises, built by hand.
//! - **The generator.** xoshiro256\*\* and SplitMix64, from an independent
//!   implementation of the published algorithms, itself checked against
//!   the authors' published outputs (repeated below).
//! - **The positional hash and stream seeds.** An independent
//!   implementation of the formula in the documentation of `pos_hash`.
//! - **`Real` functions.** This crate's `libm`-backed results on x86-64.
//!   Each was compared with arbitrary-precision arithmetic: the functions
//!   are within one unit in the last place of the true value, and the
//!   exact ones are exact.

use clash_core::{
    Dir8, Dur, EntityId, Hash128, HashDomain, IVec3, LookupMap, Real, Rng, RngStreams, Rot4,
    StateHasher, Tick, Turn, Value, VarMap, pos_hash,
};

fn real(bits: u64) -> Real {
    Real::new(f64::from_bits(bits)).expect("reference inputs are never NaN")
}

fn bits(value: Real) -> u64 {
    value.get().to_bits()
}

fn check_one(name: &str, function: fn(Real) -> Real, vectors: &[(u64, u64)]) {
    for &(input, expected) in vectors {
        let got = bits(function(real(input)));
        assert_eq!(
            got,
            expected,
            "{name}({:?}) gave {:#018x}, expected {:#018x}",
            f64::from_bits(input),
            got,
            expected
        );
    }
}

fn check_two(name: &str, function: fn(Real, Real) -> Real, vectors: &[(u64, u64, u64)]) {
    for &(a, b, expected) in vectors {
        let got = bits(function(real(a), real(b)));
        assert_eq!(
            got,
            expected,
            "{name}({:?}, {:?}) gave {:#018x}, expected {:#018x}",
            f64::from_bits(a),
            f64::from_bits(b),
            got,
            expected
        );
    }
}

#[test]
fn real_sqrt_and_cbrt() {
    check_one(
        "sqrt",
        Real::sqrt,
        &[
            (0x4000000000000000, 0x3ff6a09e667f3bcd), // sqrt(2.0)
            (0x3fe0000000000000, 0x3fe6a09e667f3bcd), // sqrt(0.5)
            (0x46c8a6e32246c99c, 0x435c162fdb89e9d4), // sqrt(1e+33)
            (0x01a56e1fc2f8f359, 0x20ca2fe76a3f9475), // sqrt(1e-300)
            (0x40fe240c9fbe76c9, 0x4075f5d3b16948f6), // sqrt(123456.789)
            (0x4022000000000000, 0x4008000000000000), // sqrt(9.0)
        ],
    );
    check_one(
        "cbrt",
        Real::cbrt,
        &[
            (0x4000000000000000, 0x3ff428a2f98d728b), // cbrt(2.0)
            (0xc03b000000000000, 0xc008000000000000), // cbrt(-27.0)
            (0x46c8a6e32246c99c, 0x42374876e8000000), // cbrt(1e+33)
            (0x3f50624dd2f1a9fc, 0x3fb999999999999a), // cbrt(0.001)
            (0x3fd3333333333333, 0x3fe56bfea66ef78d), // cbrt(0.3)
        ],
    );
}

#[test]
fn real_exp_and_logarithms() {
    check_one(
        "exp",
        Real::exp,
        &[
            (0x3ff0000000000000, 0x4005bf0a8b14576a), // exp(1.0), 1 ulp from correctly rounded
            (0xbff0000000000000, 0x3fd78b56362cef38), // exp(-1.0)
            (0x3fe0000000000000, 0x3ffa61298e1e069c), // exp(0.5)
            (0x4049000000000000, 0x44719103e4080b45), // exp(50.0)
            (0xc085e00000000000, 0x00d14f2b0fb9307f), // exp(-700.0)
            (0x4086280000000000, 0x7fdd422d2be5dc9b), // exp(709.0)
            (0x3ddb7cdfd9d7bdbb, 0x3ff000000006df38), // exp(1e-10)
        ],
    );
    check_one(
        "ln",
        Real::ln,
        &[
            (0x4000000000000000, 0x3fe62e42fefa39ef), // ln(2.0)
            (0x4024000000000000, 0x40026bb1bbb55516), // ln(10.0)
            (0x3fb999999999999a, 0xc0026bb1bbb55515), // ln(0.1)
            (0x46c8a6e32246c99c, 0x4052ff0f4992ffbe), // ln(1e+33)
            (0x01a56e1fc2f8f359, 0xc085963447f87fb5), // ln(1e-300)
            (0x3feffffde7210be9, 0xbeb0c6f82d74d230), // ln(0.999999)
        ],
    );
    check_one(
        "log10",
        Real::log10,
        &[
            (0x4000000000000000, 0x3fd34413509f79ff), // log10(2.0)
            (0x46c8a6e32246c99c, 0x4040800000000000), // log10(1e+33)
            (0x3fd3333333333333, 0xbfe0bb6c34d81502), // log10(0.3)
            (0x40c81cd6c8b43958, 0x40105db618083a9e), // log10(12345.678)
            (0x3e7ad7f29abcaf48, 0xc01c000000000000), // log10(1e-07)
        ],
    );
}

#[test]
fn real_trigonometry() {
    check_one(
        "sin",
        Real::sin,
        &[
            (0x3fe0000000000000, 0x3fdeaee8744b05f0), // sin(0.5)
            (0x3ff0000000000000, 0x3feaed548f090cee), // sin(1.0)
            (0x4008000000000000, 0x3fc210386db6d55b), // sin(3.0)
            (0xc004000000000000, 0xbfe326af0dcfcab0), // sin(-2.5), 1 ulp from correctly rounded
            (0x4059000000000000, 0xbfe03425b78c4db8), // sin(100.0)
            (0x412e848000000000, 0xbfd6664b2568d867), // sin(1000000.0)
            (0x4480f0cf064dd592, 0xbfeb453ab76bf397), // sin(1e+22)
            (0x3e45798ee2308c3a, 0x3e45798ee2308c3a), // sin(1e-08)
        ],
    );
    check_one(
        "cos",
        Real::cos,
        &[
            (0x3fe0000000000000, 0x3fec1528065b7d50), // cos(0.5)
            (0x3ff0000000000000, 0x3fe14a280fb5068c), // cos(1.0)
            (0x4008000000000000, 0xbfefae04be85e5d2), // cos(3.0)
            (0xc004000000000000, 0xbfe9a2f7ef858b7d), // cos(-2.5)
            (0x4059000000000000, 0x3feb981dbf665fdf), // cos(100.0)
            (0x412e848000000000, 0x3fedf9df9906d32c), // cos(1000000.0)
            (0x4480f0cf064dd592, 0x3fe0be2cef01c8f4), // cos(1e+22)
            (0x3e45798ee2308c3a, 0x3ff0000000000000), // cos(1e-08)
        ],
    );
    check_one(
        "tan",
        Real::tan,
        &[
            (0x3fe0000000000000, 0x3fe17b4f5bf3474a), // tan(0.5)
            (0x3ff8000000000000, 0x402c33ed50b88777), // tan(1.5)
            (0xc004000000000000, 0x3fe7e79b4e00bb15), // tan(-2.5)
            (0x4059000000000000, 0xbfe2ca74d62b5d38), // tan(100.0)
            (0xc12e848000000000, 0x3fd7e9768ab734c0), // tan(-1000000.0)
        ],
    );
    check_two(
        "atan2",
        Real::atan2,
        &[
            (0x3ff0000000000000, 0x3ff0000000000000, 0x3fe921fb54442d18), // atan2(1.0, 1.0)
            (0x3ff0000000000000, 0xbff0000000000000, 0x4002d97c7f3321d2), // atan2(1.0, -1.0)
            (0xbfe0000000000000, 0x4000000000000000, 0xbfcf5b75f92c80dd), // atan2(-0.5, 2.0)
            (0x3ddb7cdfd9d7bdbb, 0xbff0000000000000, 0x400921fb5440bd7c), // atan2(1e-10, -1.0)
            (0x4008000000000000, 0x4415af1d78b58c40, 0x3be1b578c96db19b), // atan2(3.0, 1e+20)
            (0xbff0000000000000, 0x0000000000000000, 0xbff921fb54442d18), // atan2(-1.0, 0.0)
        ],
    );
}

#[test]
fn real_powers() {
    check_two(
        "powf",
        Real::powf,
        &[
            (0x4000000000000000, 0x3fe0000000000000, 0x3ff6a09e667f3bcd), // powf(2.0, 0.5)
            (0x4024000000000000, 0x4040800000000000, 0x46c8a6e32246c99c), // powf(10.0, 33.0)
            (0x3ff8000000000000, 0xc002000000000000, 0x3fd9b3d438ada293), // powf(1.5, -2.25)
            (0x3fd3333333333333, 0x401ecccccccccccd, 0x3f18ae7e5b4e6bae), // powf(0.3, 7.7), 1 ulp from correctly rounded
            (0x405edd2f1a9fbe77, 0x3f50624dd2f1a9fc, 0x3ff013c6001e2a2e), // powf(123.456, 0.001)
            (0xc000000000000000, 0x4008000000000000, 0xc020000000000000), // powf(-2.0, 3.0)
            (0x4000000000000000, 0xc090c80000000000, 0x0000000000000001), // powf(2.0, -1074.0)
        ],
    );
    // `powi` is repeated multiplication, so it is exact to these bits
    // everywhere but can be a few units in the last place from the true
    // power.
    let vectors: [(u64, i32, u64); 4] = [
        (0x3ff199999999999a, 10, 0x4004bffc0c03023d), // powi(1.1, 10)
        (0x3fefae147ae147ae, 100, 0x3fd76d12e9c2ff40), // powi(0.99, 100)
        (0x4008000000000000, -5, 0x3f70db20a88f4696), // powi(3.0, -5)
        (0x4000000000000000, 62, 0x43d0000000000000), // powi(2.0, 62)
    ];
    for (input, power, expected) in vectors {
        assert_eq!(
            bits(real(input).powi(power)),
            expected,
            "powi({:?}, {power})",
            f64::from_bits(input)
        );
    }
}

#[test]
fn real_rounding() {
    check_one(
        "floor",
        Real::floor,
        &[
            (0x4004000000000000, 0x4000000000000000), // floor(2.5)
            (0xc004000000000000, 0xc008000000000000), // floor(-2.5)
            (0x430c6bf526340004, 0x430c6bf526340000), // floor(1000000000000000.5)
        ],
    );
    check_one(
        "ceil",
        Real::ceil,
        &[
            (0x4004000000000000, 0x4008000000000000), // ceil(2.5)
            (0xc004000000000000, 0xc000000000000000), // ceil(-2.5)
            (0x430c6bf526340004, 0x430c6bf526340008), // ceil(1000000000000000.5)
        ],
    );
    check_one(
        "round",
        Real::round,
        &[
            (0x4004000000000000, 0x4008000000000000), // round(2.5)
            (0xc004000000000000, 0xc008000000000000), // round(-2.5)
            (0x3fdfffffffffffff, 0x0000000000000000), // round(0.49999999999999994)
        ],
    );
    check_one(
        "trunc",
        Real::trunc,
        &[
            (0x4007333333333333, 0x4000000000000000), // trunc(2.9)
            (0xc007333333333333, 0xc000000000000000), // trunc(-2.9)
        ],
    );
}

#[test]
fn real_operators_are_ieee_exact() {
    // One example of each operator whose rounding is not trivial.
    let tenth = Real::of(0.1);
    let fifth = Real::of(0.2);
    assert_eq!(bits(tenth + fifth), 0x3fd3333333333334); // 0.30000000000000004
    assert_eq!(bits(Real::of(0.3) - tenth), 0x3fc9999999999999); // 0.19999999999999998
    assert_eq!(bits(tenth * Real::of(3.0)), 0x3fd3333333333334);
    assert_eq!(bits(Real::ONE / Real::of(3.0)), 0x3fd5555555555555);
    assert_eq!(bits(Real::of(1e33) * Real::of(1e-33)), 0x3ff0000000000000);
}

#[test]
fn prng_published_xoshiro256_starstar_vector() {
    // From the authors' reference implementation, state {1, 2, 3, 4}.
    let mut rng = Rng::from_state([1, 2, 3, 4]).unwrap();
    let expected: [u64; 10] = [
        11520,
        0,
        1509978240,
        1215971899390074240,
        1216172134540287360,
        607988272756665600,
        16172922978634559625,
        8476171486693032832,
        10595114339597558777,
        2904607092377533576,
    ];
    for value in expected {
        assert_eq!(rng.next_u64(), value);
    }
}

#[test]
fn prng_seeding_and_output() {
    // Rng::from_seed(seed).state(): four SplitMix64 outputs.
    let states: [(u64, [u64; 4]); 2] = [
        (
            0x0,
            [
                0xe220a8397b1dcdaf,
                0x6e789e6aa1b965f4,
                0x06c45d188009454f,
                0xf88bb8a8724c81ec,
            ],
        ),
        (
            0x2a,
            [
                0xbdd732262feb6e95,
                0x28efe333b266f103,
                0x47526757130f9f52,
                0x581ce1ff0e4ae394,
            ],
        ),
    ];
    for (seed, state) in states {
        assert_eq!(Rng::from_seed(seed).state(), state, "seed {seed:#x}");
    }

    // The first six outputs for a seed.
    let outputs: [(u64, [u64; 6]); 5] = [
        (
            0x0,
            [
                0x99ec5f36cb75f2b4,
                0xbf6e1f784956452a,
                0x1a5f849d4933e6e0,
                0x6aa594f1262d2d2c,
                0xbba5ad4a1f842e59,
                0xffef8375d9ebcaca,
            ],
        ),
        (
            0x1,
            [
                0xb3f2af6d0fc710c5,
                0x853b559647364cea,
                0x92f89756082a4514,
                0x642e1c7bc266a3a7,
                0xb27a48e29a233673,
                0x24c123126ffda722,
            ],
        ),
        (
            0x2a,
            [
                0x15780b2e0c2ec716,
                0x6104d9866d113a7e,
                0xae17533239e499a1,
                0xecb8ad4703b360a1,
                0xfde6dc7fe2ec5e64,
                0xc50da53101795238,
            ],
        ),
        (
            0xdeadbeef,
            [
                0xc5555444a74d7e83,
                0x65c30d37b4b16e38,
                0x54f773200a4efa23,
                0x429aed75fb958af7,
                0xfb0e1dd69c255b2e,
                0x9d6d02ec58814a27,
            ],
        ),
        (
            0xffffffffffffffff,
            [
                0x8f5520d52a7ead08,
                0xc476a018caa1802d,
                0x81de31c0d260469e,
                0xbf658d7e065f3c2f,
                0x913593fda1bca32a,
                0xbb535e93941ba525,
            ],
        ),
    ];
    for (seed, expected) in outputs {
        let mut rng = Rng::from_seed(seed);
        for value in expected {
            assert_eq!(rng.next_u64(), value, "seed {seed:#x}");
        }
    }

    let mut rng = Rng::from_seed(0x2a);
    assert_eq!(rng.next_u32(), 0x15780b2e);
}

#[test]
fn prng_derived_draws() {
    // below: two draws for each bound in turn.
    let mut rng = Rng::from_seed(7);
    let bounds: [u64; 8] = [
        6,
        100,
        1099511627776,
        9223372036854788153,
        3,
        18446744073709551615,
        1,
        10,
    ];
    let expected: [u64; 16] = [
        4,
        1,
        83,
        98,
        1089462398078,
        959625094070,
        560339031174819607,
        3723535483949831687,
        0,
        1,
        13500401043614375895,
        17320858093524191696,
        0,
        0,
        5,
        2,
    ];
    let got: Vec<u64> = bounds
        .iter()
        .flat_map(|&bound| [bound, bound])
        .map(|bound| rng.below(bound))
        .collect();
    assert_eq!(got, expected);

    // range: two draws for each range in turn.
    let mut rng = Rng::from_seed(8);
    let ranges: [(i64, i64); 6] = [
        (1, 6),
        (-10, 10),
        (i64::MIN, i64::MAX),
        (0, 0),
        (5, 1),
        (-(1 << 62), 1 << 62),
    ];
    let expected: [i64; 12] = [
        5,
        4,
        2,
        9,
        7302792631413820773,
        4088574419011006665,
        0,
        0,
        5,
        5,
        3939011841777761744,
        2228039904992974602,
    ];
    let got: Vec<i64> = ranges
        .iter()
        .flat_map(|&range| [range, range])
        .map(|(low, high)| rng.range(low, high))
        .collect();
    assert_eq!(got, expected);

    // unit: the exact bits of four draws.
    let mut rng = Rng::from_seed(9);
    let expected: [u64; 4] = [
        0x3f6529dd9ec33400,
        0x3fd01866e17454be,
        0x3fc0f485e418402c,
        0x3fe7723b945dbdc9,
    ];
    for value in expected {
        assert_eq!(bits(rng.unit()), value);
    }

    // shuffle.
    let mut rng = Rng::from_seed(10);
    let mut items: Vec<u32> = (0..10).collect();
    rng.shuffle(&mut items);
    assert_eq!(items, [5, 4, 6, 8, 1, 2, 7, 0, 3, 9]);
}

#[test]
fn hash_domain_names_are_fnv1a_64() {
    let vectors: [(&str, u64); 5] = [
        ("", 0xcbf29ce484222325),
        ("a", 0xaf63dc4c8601ec8c),
        ("foobar", 0x85944171f73967e8),
        ("gen.ore", 0x677ce60867ed3f17),
        ("clash.worldgen.tract", 0x82ea135da554dc62),
    ];
    for (name, expected) in vectors {
        assert_eq!(HashDomain::new(name).raw(), expected, "{name:?}");
    }
}

#[test]
fn pos_hash_vectors() {
    // ((seed, domain, x, y, z, salt), hash)
    type Case = ((u64, u64, i32, i32, i32, u64), u64);
    let vectors: [Case; 11] = [
        ((0x0, 0x0, 0, 0, 0, 0x0), 0x78ae5a9a6b5fd45e),
        ((0x1, 0x0, 0, 0, 0, 0x0), 0x348c3d1af8bec8e5),
        ((0x0, 0x1, 0, 0, 0, 0x0), 0xf9ac6e337c2f0f1e),
        ((0x0, 0x0, 1, 0, 0, 0x0), 0x3ceea767acc0e20d),
        ((0x0, 0x0, 0, 1, 0, 0x0), 0xcbb14bca9f2edd44),
        ((0x0, 0x0, 0, 0, 1, 0x0), 0x2d6527ba78af243f),
        ((0x0, 0x0, 0, 0, 0, 0x1), 0xa5d3758b95b92366),
        (
            (0x2a, 0x677ce60867ed3f17, 10, 20, 3, 0x7),
            0x37944b62f1f1269f,
        ),
        (
            (0x2a, 0x677ce60867ed3f17, -10, -20, -3, 0x7),
            0xac98640bf78b74b9,
        ),
        (
            (
                0xffffffffffffffff,
                0xffffffffffffffff,
                -2147483648,
                2147483647,
                -1,
                0xffffffffffffffff,
            ),
            0xfa05a25b70140ca5,
        ),
        (
            (
                0x123456789abcdef,
                0x82ea135da554dc62,
                1000000,
                -1000000,
                255,
                0xfedcba9876543210,
            ),
            0x8a07342755e02faa,
        ),
    ];
    for ((seed, domain, x, y, z, salt), expected) in vectors {
        let got = pos_hash(seed, HashDomain::from_raw(domain), x, y, z, salt);
        assert_eq!(
            got, expected,
            "pos_hash({seed:#x}, {domain:#x}, {x}, {y}, {z}, {salt:#x})"
        );
    }
    // A named domain is its FNV-1a number.
    assert_eq!(
        pos_hash(0x2a, HashDomain::new("gen.ore"), 10, 20, 3, 0x7),
        0x37944b62f1f1269f
    );
}

#[test]
fn stream_seed_vectors() {
    // The first two draws of RngStreams::new(seed).stream(name).
    let vectors: [((u64, &str), [u64; 2]); 4] = [
        ((0x0, "rules"), [0xcb3c6a542e20e758, 0xffdf123c443a884b]),
        ((0x63, "combat"), [0x9b50f28d51b6bc1d, 0x96965a3dcc9f022b]),
        ((0x63, "weather"), [0x630a38dbfc861162, 0xd11da86c5cf08385]),
        (
            (0xffffffffffffffff, ""),
            [0xe34965e187b89f03, 0x63f001af8a60d5be],
        ),
    ];
    for ((seed, name), expected) in vectors {
        let mut streams = RngStreams::new(seed);
        let stream = streams.stream(name);
        assert_eq!(
            [stream.next_u64(), stream.next_u64()],
            expected,
            "seed {seed:#x}, stream {name:?}"
        );
    }
}

fn counting(length: usize) -> Vec<u8> {
    (0..length).map(|n| n as u8).collect()
}

fn hash_bytes(bytes: &[u8]) -> String {
    let mut hasher = StateHasher::new();
    hasher.write_bytes(bytes);
    hasher.finish().to_string()
}

#[test]
fn hasher_is_murmur3_x64_128() {
    let vectors: Vec<(Vec<u8>, &str)> = vec![
        (b"".to_vec(), "00000000000000000000000000000000"),
        (b"a".to_vec(), "85555565f6597889e6b53a48510e895a"),
        (b"hello".to_vec(), "cbd8a7b341bd9b025b1e906a48ae1d19"),
        (
            b"The quick brown fox jumps over the lazy dog".to_vec(),
            "e34bbc7bbc071b6c7a433ca9c49a9347",
        ),
        (counting(15), "47231598fd4925e9cd846dee88c67de9"),
        (counting(16), "444924b591903f30ab906456762fe845"),
        (counting(17), "5c76f40f9fe7c20ec15f026b9edaa824"),
        (counting(31), "053dd3e1a32cd0949ee59aefb4005490"),
        (counting(32), "c66d9022b62f500f1c050a6e34c31151"),
        (counting(33), "7d41281bfaba461255ac8073a7d6a30b"),
        (counting(256), "1c99c313dc6f12b970d6077fab34cc1e"),
        (vec![0x00], "4610abe56eff5cb551622daa78f83583"),
        (vec![0x00; 16], "4bbd1bf27da918d6b465a9eccd791cb6"),
        (vec![0xff; 47], "9dd45b94fe1934777fa198f5a7b3cbc7"),
    ];
    for (bytes, expected) in vectors {
        assert_eq!(hash_bytes(&bytes), expected, "{} bytes", bytes.len());
    }
}

fn hash_of<T: clash_core::StateHash>(value: T) -> String {
    Hash128::of(&value).to_string()
}

#[test]
fn state_hash_encodings_of_primitives_and_containers() {
    assert_eq!(hash_of(()), "00000000000000000000000000000000");
    assert_eq!(hash_of(true), "7ace5c908374fe16778867e4430e6785");
    assert_eq!(hash_of(7_u8), "c427909d8972bd175c15702a45b199dd");
    assert_eq!(hash_of(0x0102_0304_u32), "6b1f5992f69e9e548ee80d9aef7458b5");
    assert_eq!(hash_of(-1_i64), "a0e4b27a1abaed73692112c96b4a46af");
    assert_eq!(hash_of(5_usize), "0fd4c5f69b6c771b008a790318490b16");
    assert_eq!(hash_of("two"), "ecf44a008f0e4f45cec7ce378695bb45");
    assert_eq!(hash_of((1_u32, "two")), "c4155a0d591f59f606412c99d8f9a0c8");
    assert_eq!(
        hash_of(vec![1_u32, 2, 3]),
        "7c7ffbae1c3f600702b62dd916614398"
    );
    assert_eq!(hash_of([1_u32, 2, 3]), "50cf51c5c3695cb54db388a324d73541");
    assert_eq!(hash_of(Some(9_u8)), "ce0a4c08eeef895aa08c5b653f16a746");
    assert_eq!(hash_of(None::<u8>), "4610abe56eff5cb551622daa78f83583");
}

#[test]
fn state_hash_encodings_of_core_types() {
    assert_eq!(hash_of(Real::of(1.5)), "f262c8fe30b0f8b39a5ead32c511ccb4");
    assert_eq!(hash_of(Real::of(-0.0)), "01159dfeb45932278bdef8b0ec4fe0b6");
    assert_eq!(
        hash_of(Tick(16_000_000)),
        "05aa6a5c62a6cc296de2e4a35c50add5"
    );
    assert_eq!(hash_of(Dur(-1)), "a0e4b27a1abaed73692112c96b4a46af");
    assert_eq!(
        hash_of(IVec3::new(1, -2, 3)),
        "562870d58c939944e7de50d19d51ce9f"
    );
    assert_eq!(hash_of(Dir8::SouthWest), "3a7d969fbc368cf8d070b9cd738a1b64");
    assert_eq!(
        hash_of(Turn::new(Rot4::R270, true)),
        "b73e42bb654cee53750aa06ed9cd1bac"
    );
    assert_eq!(
        hash_of(EntityId::new(7, 2)),
        "9e8def6545ab999f82e7389175b6cefb"
    );
    assert_eq!(hash_of(Value::Int(3)), "8dc17b2750a8b667babe45cb4c26df62");
    assert_eq!(
        hash_of(Value::Str("ash".to_owned())),
        "e86017b874d59546c927c42fe6e2f4a0"
    );
    assert_eq!(
        hash_of(Value::Real(Real::of(2.5))),
        "dcd7671a0377735667e495f657f65456"
    );
    assert_eq!(
        hash_of(Value::Bool(true)),
        "7cb3f5c58dab264cf03c01326ec2e044"
    );

    let mut vars = VarMap::new();
    vars.set("m:b", true);
    vars.set("m:a", 1_i64);
    assert_eq!(hash_of(&vars), "1c7f821fa57aed62304cc512f669ffbb");

    let mut lookup: LookupMap<u32, u32> = LookupMap::new();
    lookup.insert(2, 20);
    lookup.insert(1, 10);
    assert_eq!(hash_of(&lookup), "7abf56062da4a203849cbeb51235ece4");

    assert_eq!(
        hash_of(Rng::from_seed(42)),
        "df36d5e3814652c2bbf6394fc39eb4b9"
    );

    let mut streams = RngStreams::new(99);
    streams.stream("weather");
    streams.stream("combat");
    assert_eq!(hash_of(&streams), "0c2fb754c3fb4897a7aec40570df1fa6");
}
