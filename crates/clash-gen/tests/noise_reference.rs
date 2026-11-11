//! Reference vectors for `clash_gen::noise`: the cross-platform contract
//! of the noise functions.
//!
//! Every value here must come out bit for bit on every supported target
//! and with every toolchain. A failure in this file means a world made
//! elsewhere from the same seed would differ: fix the cause, never the
//! vector. Changing a vector on purpose changes every generated world and
//! needs a decision entry.
//!
//! The vectors were produced by an independent implementation of the
//! formulas in the documentation of `clash_gen::noise` (in another
//! language, sharing no code with the engine, and finding the nearest
//! scattered points by searching a block of cells nine wide), and this
//! crate's results matched it on every one.

use clash_core::{HashDomain, Real};
use clash_gen::noise::{
    Fractal, billow_2d, cellular_2d, cellular_3d, fbm_2d, fbm_3d, gradient_2d, gradient_3d,
    ridged_2d, value_2d, value_3d,
};

const DOMAIN: HashDomain = HashDomain::new("test.reference");

/// The fractal the `billow` vectors use: nothing at its default.
const ODD: Fractal = Fractal {
    octaves: 3,
    lacunarity: Real::of(2.5),
    gain: Real::of(0.4),
};

/// The bits of each function's result at one position in the plane.
struct PlaneVector {
    seed: u64,
    /// The bits of `x` and `y`.
    at: [u64; 2],
    /// `value_2d`.
    value: u64,
    /// `gradient_2d`.
    gradient: u64,
    /// `fbm_2d` with five octaves.
    fbm: u64,
    /// `ridged_2d` with four octaves.
    ridged: u64,
    /// `billow_2d` with [`ODD`].
    billow: u64,
    /// `cellular_2d`: the nearest and second distances, and the cell.
    cellular: [u64; 3],
}

/// The bits of each function's result at one position in space.
struct SpaceVector {
    seed: u64,
    /// The bits of `x`, `y` and `z`.
    at: [u64; 3],
    /// `value_3d`.
    value: u64,
    /// `gradient_3d`.
    gradient: u64,
    /// `fbm_3d` with five octaves.
    fbm: u64,
    /// `cellular_3d`: the nearest and second distances, and the cell.
    cellular: [u64; 3],
}

fn real(bits: u64) -> Real {
    Real::new(f64::from_bits(bits)).expect("reference inputs are never NaN")
}

fn bits(value: Real) -> u64 {
    value.get().to_bits()
}

#[track_caller]
fn check(name: &str, seed: u64, at: &[u64], got: u64, expected: u64) {
    assert_eq!(
        got, expected,
        "{name} with seed {seed} at {at:x?}: got {got:#018x}, expected {expected:#018x}"
    );
}

#[test]
fn noise_in_the_plane_matches_the_reference() {
    assert_eq!(PLANE.len(), 24);
    for vector in PLANE {
        let seed = vector.seed;
        let (x, y) = (real(vector.at[0]), real(vector.at[1]));
        let at = &vector.at;
        check(
            "value_2d",
            seed,
            at,
            bits(value_2d(seed, DOMAIN, x, y)),
            vector.value,
        );
        check(
            "gradient_2d",
            seed,
            at,
            bits(gradient_2d(seed, DOMAIN, x, y)),
            vector.gradient,
        );
        let fbm = fbm_2d(seed, DOMAIN, Fractal::new(5), x, y);
        check("fbm_2d", seed, at, bits(fbm), vector.fbm);
        let ridged = ridged_2d(seed, DOMAIN, Fractal::new(4), x, y);
        check("ridged_2d", seed, at, bits(ridged), vector.ridged);
        let billow = billow_2d(seed, DOMAIN, ODD, x, y);
        check("billow_2d", seed, at, bits(billow), vector.billow);
        let found = cellular_2d(seed, DOMAIN, x, y);
        check(
            "cellular_2d nearest",
            seed,
            at,
            bits(found.nearest),
            vector.cellular[0],
        );
        check(
            "cellular_2d second",
            seed,
            at,
            bits(found.second),
            vector.cellular[1],
        );
        check("cellular_2d cell", seed, at, found.cell, vector.cellular[2]);
    }
}

#[test]
fn noise_in_space_matches_the_reference() {
    assert_eq!(SPACE.len(), 20);
    for vector in SPACE {
        let seed = vector.seed;
        let (x, y, z) = (real(vector.at[0]), real(vector.at[1]), real(vector.at[2]));
        let at = &vector.at;
        check(
            "value_3d",
            seed,
            at,
            bits(value_3d(seed, DOMAIN, x, y, z)),
            vector.value,
        );
        check(
            "gradient_3d",
            seed,
            at,
            bits(gradient_3d(seed, DOMAIN, x, y, z)),
            vector.gradient,
        );
        let fbm = fbm_3d(seed, DOMAIN, Fractal::new(5), x, y, z);
        check("fbm_3d", seed, at, bits(fbm), vector.fbm);
        let found = cellular_3d(seed, DOMAIN, x, y, z);
        check(
            "cellular_3d nearest",
            seed,
            at,
            bits(found.nearest),
            vector.cellular[0],
        );
        check(
            "cellular_3d second",
            seed,
            at,
            bits(found.second),
            vector.cellular[1],
        );
        check("cellular_3d cell", seed, at, found.cell, vector.cellular[2]);
    }
}

const PLANE: &[PlaneVector] = &[
    PlaneVector {
        seed: 0,
        at: [0x0000_0000_0000_0000, 0x0000_0000_0000_0000],
        value: 0x3fe8_6fa3_0431_a81c,
        gradient: 0x0000_0000_0000_0000,
        fbm: 0x0000_0000_0000_0000,
        ridged: 0x3ff0_0000_0000_0000,
        billow: 0x0000_0000_0000_0000,
        cellular: [
            0x3fd0_d795_d608_d2a8,
            0x3fe4_7e6f_caa0_cf1b,
            0x873b_e055_176e_57ac,
        ],
    },
    PlaneVector {
        seed: 0,
        at: [0x3fe0_0000_0000_0000, 0x3fe0_0000_0000_0000],
        value: 0x3fe9_34f0_5893_2364,
        gradient: 0x3fe2_73d7_48a2_787f,
        fbm: 0x3fd3_0c39_112b_d739,
        ridged: 0x3fe1_fe03_b83d_ff77,
        billow: 0x3fde_f56d_e9d7_6ba2,
        cellular: [
            0x3fb3_7c93_3897_a8af,
            0x3fe1_457e_d3e9_689a,
            0xc30f_96c6_7e11_89fa,
        ],
    },
    PlaneVector {
        seed: 0,
        at: [0x3ff4_0000_0000_0000, 0xc00e_0000_0000_0000],
        value: 0x3fd6_5bf2_b0d2_e576,
        gradient: 0x3fd3_2722_8aa4_efba,
        fbm: 0x3fbe_9a5a_acc2_55d8,
        ridged: 0x3fe5_2924_52e5_e56d,
        billow: 0x3fd3_916b_5445_1b55,
        cellular: [
            0x3fd3_7368_06db_d06c,
            0x3fea_348e_08f7_ae6d,
            0x3d8f_d0f5_19d5_e65e,
        ],
    },
    PlaneVector {
        seed: 0,
        at: [0xc059_0666_6666_6666, 0x406f_5ccc_cccc_cccd],
        value: 0x3fe8_380a_7a28_7e2a,
        gradient: 0xbfc7_4384_2e58_421e,
        fbm: 0xbfc0_17f9_fde9_53db,
        ridged: 0x3fe7_b39a_592b_5b6c,
        billow: 0x3fc7_35f4_6049_44d9,
        cellular: [
            0x3fe0_7130_477d_035b,
            0x3fe1_2507_8a4c_4371,
            0x76eb_62c8_d8f8_0214,
        ],
    },
    PlaneVector {
        seed: 0,
        at: [0x412e_8480_9999_999a, 0xc12e_8481_6666_6666],
        value: 0x3fde_e73d_1a76_877e,
        gradient: 0xbfd0_d13b_72be_ad4d,
        fbm: 0xbfbb_e58a_3395_3ceb,
        ridged: 0x3fe3_587f_ddeb_e012,
        billow: 0x3fd1_01f4_f6bc_44c0,
        cellular: [
            0x3fdf_28a2_2772_a234,
            0x3feb_a3c1_1217_2565,
            0xc7e7_ef1e_64e4_8016,
        ],
    },
    PlaneVector {
        seed: 0,
        at: [0x40c8_1cd6_c8b4_3958, 0x3f50_624d_d2f1_a9fc],
        value: 0x3fdb_680d_536a_f082,
        gradient: 0xbfc1_d294_f53a_e97e,
        fbm: 0xbfb0_d27d_6b51_6376,
        ridged: 0x3fe7_2438_74df_cb76,
        billow: 0x3fcc_09cb_580d_d8cb,
        cellular: [
            0x3fe2_e9b9_dbcb_5d56,
            0x3fe6_a69e_616e_92cd,
            0x77f9_60a6_62bc_bd81,
        ],
    },
    PlaneVector {
        seed: 1,
        at: [0x0000_0000_0000_0000, 0x0000_0000_0000_0000],
        value: 0x3fc4_6fe9_742b_52f0,
        gradient: 0x0000_0000_0000_0000,
        fbm: 0x0000_0000_0000_0000,
        ridged: 0x3ff0_0000_0000_0000,
        billow: 0x0000_0000_0000_0000,
        cellular: [
            0x3fb7_a422_f0c8_3b0f,
            0x3fc8_5fd3_a1ec_f453,
            0x80e1_2939_e192_ebeb,
        ],
    },
    PlaneVector {
        seed: 1,
        at: [0x3fe0_0000_0000_0000, 0x3fe0_0000_0000_0000],
        value: 0x3fe0_19cc_a75f_c936,
        gradient: 0xbfcd_906b_cf32_8d47,
        fbm: 0xbfbe_8490_51c0_91d6,
        ridged: 0x3fe9_06d4_d2fd_2c64,
        billow: 0x3fc9_c4a4_d154_597f,
        cellular: [
            0x3fdc_115f_e0dd_1b4d,
            0x3fe2_e192_64b2_aa27,
            0x92df_8f90_cbe9_1e69,
        ],
    },
    PlaneVector {
        seed: 1,
        at: [0x3ff4_0000_0000_0000, 0xc00e_0000_0000_0000],
        value: 0x3fc6_fbca_4dc3_929d,
        gradient: 0xbfda_31aa_b86b_e3e5,
        fbm: 0xbfc0_fda4_8251_9c69,
        ridged: 0x3fe0_7c9d_4f75_cf5c,
        billow: 0x3fd7_3e9c_06e5_e3ad,
        cellular: [
            0x3fe9_0b94_f52b_191b,
            0x3fed_4717_48c4_2c44,
            0xc8e4_7da0_ce95_a519,
        ],
    },
    PlaneVector {
        seed: 1,
        at: [0xc059_0666_6666_6666, 0x406f_5ccc_cccc_cccd],
        value: 0x3fe7_37d8_366f_7fb6,
        gradient: 0xbfc6_0ccc_106e_5f02,
        fbm: 0x3fa1_fd80_b123_6d2a,
        ridged: 0x3fe3_eacd_0f8b_a0a2,
        billow: 0x3fcb_7cc6_c31a_a835,
        cellular: [
            0x3fd4_02ad_a725_edb2,
            0x3fe6_7429_609c_4952,
            0xd5d4_2bcc_2c32_73b5,
        ],
    },
    PlaneVector {
        seed: 1,
        at: [0x412e_8480_9999_999a, 0xc12e_8481_6666_6666],
        value: 0x3fda_6113_7d7d_2cdb,
        gradient: 0x3fd8_ef79_b405_3242,
        fbm: 0x3fc5_8a29_7147_c268,
        ridged: 0x3fe1_fa68_1e89_4f54,
        billow: 0x3fd9_bccb_73a6_8948,
        cellular: [
            0x3fdd_ea76_2b53_c28e,
            0x3fde_f36c_1989_b67d,
            0xb89d_4bcc_0e1d_5283,
        ],
    },
    PlaneVector {
        seed: 1,
        at: [0x40c8_1cd6_c8b4_3958, 0x3f50_624d_d2f1_a9fc],
        value: 0x3faa_fefb_66bd_3716,
        gradient: 0xbfc0_8ad3_7254_39bb,
        fbm: 0xbfa9_f98a_828b_61f4,
        ridged: 0x3fe7_c135_d0cd_c0c2,
        billow: 0x3fc4_1141_64d5_0b82,
        cellular: [
            0x3fe2_505a_ac14_dc21,
            0x3fe8_2484_68c9_d24e,
            0x2fc8_ef6d_a4b3_447f,
        ],
    },
    PlaneVector {
        seed: 0xdeadbeef,
        at: [0x0000_0000_0000_0000, 0x0000_0000_0000_0000],
        value: 0x3fe7_fbeb_9d87_cadf,
        gradient: 0x0000_0000_0000_0000,
        fbm: 0x0000_0000_0000_0000,
        ridged: 0x3ff0_0000_0000_0000,
        billow: 0x0000_0000_0000_0000,
        cellular: [
            0x3fd8_9120_fef3_29ab,
            0x3fe0_a0fc_294c_78a0,
            0xa516_7b78_de3e_3615,
        ],
    },
    PlaneVector {
        seed: 0xdeadbeef,
        at: [0x3fe0_0000_0000_0000, 0x3fe0_0000_0000_0000],
        value: 0x3fe7_0be6_7965_420e,
        gradient: 0x3fd1_6fc7_dceb_483f,
        fbm: 0x3fc1_ffc6_0d55_f7ff,
        ridged: 0x3fe7_f79f_61ee_1a5c,
        billow: 0x3fc9_f2dc_0c44_f235,
        cellular: [
            0x3fb5_c7b3_dd55_78fa,
            0x3fe8_ed5f_cbd4_d563,
            0xcd76_3cda_9cdf_ac58,
        ],
    },
    PlaneVector {
        seed: 0xdeadbeef,
        at: [0x3ff4_0000_0000_0000, 0xc00e_0000_0000_0000],
        value: 0x3fcf_974e_39f5_9b20,
        gradient: 0x3fc3_58f4_b4b7_8995,
        fbm: 0xbf64_a839_5982_24a5,
        ridged: 0x3fe6_bc4e_7e6c_b13f,
        billow: 0x3fc0_0ec0_a3f4_323e,
        cellular: [
            0x3fb0_c27d_2317_2cb6,
            0x3ff0_f827_c13d_579e,
            0x3ab4_ffa7_c74d_c1b5,
        ],
    },
    PlaneVector {
        seed: 0xdeadbeef,
        at: [0xc059_0666_6666_6666, 0x406f_5ccc_cccc_cccd],
        value: 0x3faa_a57d_766c_0a40,
        gradient: 0x3fb3_ba5a_eac5_e2a2,
        fbm: 0x3fc0_00aa_4b7e_1be5,
        ridged: 0x3fe6_cc33_6202_fe4b,
        billow: 0x3fc6_300b_b243_94da,
        cellular: [
            0x3fd3_b562_b202_5211,
            0x3fe3_dc49_0e01_ab19,
            0x709e_09af_7306_943e,
        ],
    },
    PlaneVector {
        seed: 0xdeadbeef,
        at: [0x412e_8480_9999_999a, 0xc12e_8481_6666_6666],
        value: 0x3fe6_3aa9_1574_fb16,
        gradient: 0x3fb1_ef97_2fd7_cded,
        fbm: 0x3fbe_be15_d957_927a,
        ridged: 0x3fe8_5263_ef61_2bf3,
        billow: 0x3fb9_1d55_090a_943f,
        cellular: [
            0x3fd7_9cdd_bfe6_be91,
            0x3fe0_7934_2cf7_5fea,
            0x6ea7_2fa8_7195_9275,
        ],
    },
    PlaneVector {
        seed: 0xdeadbeef,
        at: [0x40c8_1cd6_c8b4_3958, 0x3f50_624d_d2f1_a9fc],
        value: 0x3fe0_bcf8_eefe_5a44,
        gradient: 0xbfdb_a195_89db_dc80,
        fbm: 0xbfd3_d987_4fce_c08e,
        ridged: 0x3fda_947d_4462_3eb1,
        billow: 0x3fd7_5196_b4a5_7e89,
        cellular: [
            0x3fe2_6335_06c4_7a87,
            0x3fe3_a03e_e0c3_3ac4,
            0xf5de_6cc9_6e6c_2965,
        ],
    },
    PlaneVector {
        seed: u64::MAX,
        at: [0x0000_0000_0000_0000, 0x0000_0000_0000_0000],
        value: 0x3fe0_2151_115f_8b72,
        gradient: 0x0000_0000_0000_0000,
        fbm: 0x0000_0000_0000_0000,
        ridged: 0x3ff0_0000_0000_0000,
        billow: 0x0000_0000_0000_0000,
        cellular: [
            0x3fd4_fc33_8bca_513a,
            0x3fdb_04f2_446c_47fa,
            0x683e_9b82_e16e_7841,
        ],
    },
    PlaneVector {
        seed: u64::MAX,
        at: [0x3fe0_0000_0000_0000, 0x3fe0_0000_0000_0000],
        value: 0x3fd4_8cec_a854_1e6c,
        gradient: 0x3fdd_906b_cf32_8d47,
        fbm: 0x3fce_8490_51c0_91d6,
        ridged: 0x3fe3_dfd1_0fcb_634e,
        billow: 0x3fd8_3749_9911_1447,
        cellular: [
            0x3fb7_858c_5ae8_d650,
            0x3fea_9ab2_914e_63ee,
            0x1c35_c912_8d0e_3086,
        ],
    },
    PlaneVector {
        seed: u64::MAX,
        at: [0x3ff4_0000_0000_0000, 0xc00e_0000_0000_0000],
        value: 0x3fe3_9d4f_d53b_29b4,
        gradient: 0xbfe1_8e04_22ff_e240,
        fbm: 0xbfd8_dafd_5458_e276,
        ridged: 0x3fd9_bdeb_f8b4_63b7,
        billow: 0x3fe1_7111_6c2a_f03b,
        cellular: [
            0x3fdd_87dd_2650_58a8,
            0x3fed_c237_a840_c708,
            0xd8d2_ca6a_6426_5f15,
        ],
    },
    PlaneVector {
        seed: u64::MAX,
        at: [0xc059_0666_6666_6666, 0x406f_5ccc_cccc_cccd],
        value: 0x3fde_be68_204a_23da,
        gradient: 0x3fc1_ee9e_f27e_104e,
        fbm: 0xbf91_84c1_9ec3_9303,
        ridged: 0x3fe7_2ba2_53ad_8483,
        billow: 0x3fc5_1e48_7f0e_4352,
        cellular: [
            0x3fee_1741_1694_f1b8,
            0x3fee_b29a_6c97_ea12,
            0x1206_6da4_aef1_d5ca,
        ],
    },
    PlaneVector {
        seed: u64::MAX,
        at: [0x412e_8480_9999_999a, 0xc12e_8481_6666_6666],
        value: 0x3feb_d9da_202a_7af9,
        gradient: 0xbfe2_9965_0e33_eb8a,
        fbm: 0xbfd0_8382_8d8d_73ec,
        ridged: 0x3fd8_550f_e1bb_9d97,
        billow: 0x3fdb_07ae_837b_82eb,
        cellular: [
            0x3fdb_42a0_09b9_19c0,
            0x3fdc_23e6_bbe0_1b59,
            0x5486_8514_cead_14b7,
        ],
    },
    PlaneVector {
        seed: u64::MAX,
        at: [0x40c8_1cd6_c8b4_3958, 0x3f50_624d_d2f1_a9fc],
        value: 0x3fd2_d23b_b291_9eb4,
        gradient: 0x3fc2_2992_d44a_8267,
        fbm: 0x3f6f_2ee9_bbe5_2a47,
        ridged: 0x3fe4_a60a_d790_f585,
        billow: 0x3fcb_21a7_3f39_3f13,
        cellular: [
            0x3fe1_0341_f3ad_bb0c,
            0x3fec_2893_91d1_cdc0,
            0x5139_ec71_98d6_a195,
        ],
    },
];

const SPACE: &[SpaceVector] = &[
    SpaceVector {
        seed: 0,
        at: [
            0x0000_0000_0000_0000,
            0x0000_0000_0000_0000,
            0x0000_0000_0000_0000,
        ],
        value: 0x3fe8_6fa3_0431_a81c,
        gradient: 0x0000_0000_0000_0000,
        fbm: 0x0000_0000_0000_0000,
        cellular: [
            0x3fdd_6299_14aa_34de,
            0x3fe5_92f6_e27a_c88c,
            0x6688_eda1_e31f_6cdb,
        ],
    },
    SpaceVector {
        seed: 0,
        at: [
            0x3fe0_0000_0000_0000,
            0x3fe0_0000_0000_0000,
            0x3fe0_0000_0000_0000,
        ],
        value: 0x3fe3_65de_ff7b_1d8b,
        gradient: 0x3f96_dd70_7e29_11d0,
        fbm: 0x3f87_9a42_92bf_0a21,
        cellular: [
            0x3fd3_25e7_a194_0edb,
            0x3fe4_3a78_1810_1a77,
            0xc30f_96c6_7e11_89fa,
        ],
    },
    SpaceVector {
        seed: 0,
        at: [
            0x3ff4_0000_0000_0000,
            0xc00e_0000_0000_0000,
            0x4001_0000_0000_0000,
        ],
        value: 0x3fe6_43c3_48b3_d631,
        gradient: 0xbfa0_f7f2_1000_98aa,
        fbm: 0x3fa8_b409_97c9_936f,
        cellular: [
            0x3fe3_a7a4_db6e_ee63,
            0x3fe6_7195_6e0d_cea4,
            0x9bd9_ebd3_e6fb_c8cf,
        ],
    },
    SpaceVector {
        seed: 0,
        at: [
            0xc059_0666_6666_6666,
            0x406f_5ccc_cccc_cccd,
            0xc01d_3333_3333_3333,
        ],
        value: 0x3fd1_6241_f3b7_f21c,
        gradient: 0x3fd0_41c3_e71f_efa1,
        fbm: 0x3fb9_8fb5_94ff_b614,
        cellular: [
            0x3fd6_5da4_a6f8_8005,
            0x3fe3_eadb_2426_2229,
            0xa6f7_d0a4_1429_743a,
        ],
    },
    SpaceVector {
        seed: 0,
        at: [
            0x40c8_1cd6_c8b4_3958,
            0x3f50_624d_d2f1_a9fc,
            0x4058_ff5c_28f5_c28f,
        ],
        value: 0x3fba_534b_29ed_c098,
        gradient: 0x3fdc_e222_6587_4f5f,
        fbm: 0x3fd3_0ca5_b91f_152f,
        cellular: [
            0x3fdf_9026_a44d_c8b3,
            0x3fe3_4a1c_1597_4d9b,
            0x15e0_7894_2459_2b0d,
        ],
    },
    SpaceVector {
        seed: 1,
        at: [
            0x0000_0000_0000_0000,
            0x0000_0000_0000_0000,
            0x0000_0000_0000_0000,
        ],
        value: 0x3fc4_6fe9_742b_52f0,
        gradient: 0x0000_0000_0000_0000,
        fbm: 0x0000_0000_0000_0000,
        cellular: [
            0x3fd2_9901_1391_4f4b,
            0x3fd3_68e1_ae03_0174,
            0x80e1_2939_e192_ebeb,
        ],
    },
    SpaceVector {
        seed: 1,
        at: [
            0x3fe0_0000_0000_0000,
            0x3fe0_0000_0000_0000,
            0x3fe0_0000_0000_0000,
        ],
        value: 0x3fdc_de5f_3f7b_e213,
        gradient: 0xbfbd_2451_f03a_ddc8,
        fbm: 0xbfae_14f9_be2c_3fc6,
        cellular: [
            0x3fde_6e4a_5b45_d9ee,
            0x3fe4_af34_8743_1ffd,
            0x92df_8f90_cbe9_1e69,
        ],
    },
    SpaceVector {
        seed: 1,
        at: [
            0x3ff4_0000_0000_0000,
            0xc00e_0000_0000_0000,
            0x4001_0000_0000_0000,
        ],
        value: 0x3fe1_3ad1_b8af_0d17,
        gradient: 0xbfc2_18d0_e741_8b45,
        fbm: 0xbfc5_f060_16a6_462e,
        cellular: [
            0x3fd7_7047_3441_10ec,
            0x3fdf_0971_b876_1e45,
            0xcc65_f102_fcc8_adae,
        ],
    },
    SpaceVector {
        seed: 1,
        at: [
            0xc059_0666_6666_6666,
            0x406f_5ccc_cccc_cccd,
            0xc01d_3333_3333_3333,
        ],
        value: 0x3fe9_36cf_dd2d_cb0b,
        gradient: 0x3fac_2542_1ef6_6ee6,
        fbm: 0x3fc1_8542_e642_884b,
        cellular: [
            0x3fe7_5259_e503_4c79,
            0x3fe7_71be_a50f_3976,
            0x3dfa_926e_68f6_5aba,
        ],
    },
    SpaceVector {
        seed: 1,
        at: [
            0x40c8_1cd6_c8b4_3958,
            0x3f50_624d_d2f1_a9fc,
            0x4058_ff5c_28f5_c28f,
        ],
        value: 0x3fe6_b235_b5c2_33f6,
        gradient: 0x3fb8_8769_7ee8_d65e,
        fbm: 0x3fc1_4165_8644_6f4f,
        cellular: [
            0x3fe2_1172_5923_8f14,
            0x3fe2_3bcf_4419_1c5e,
            0x1815_a353_89ae_1580,
        ],
    },
    SpaceVector {
        seed: 0xdeadbeef,
        at: [
            0x0000_0000_0000_0000,
            0x0000_0000_0000_0000,
            0x0000_0000_0000_0000,
        ],
        value: 0x3fe7_fbeb_9d87_cadf,
        gradient: 0x0000_0000_0000_0000,
        fbm: 0x0000_0000_0000_0000,
        cellular: [
            0x3fe5_bd79_bfd8_b182,
            0x3feb_e9fe_4500_9759,
            0xbb21_e526_fdd8_c561,
        ],
    },
    SpaceVector {
        seed: 0xdeadbeef,
        at: [
            0x3fe0_0000_0000_0000,
            0x3fe0_0000_0000_0000,
            0x3fe0_0000_0000_0000,
        ],
        value: 0x3fe3_85d9_b970_3edc,
        gradient: 0xbfd1_32da_06ad_b5bb,
        fbm: 0xbfc1_c0e1_0f26_ed24,
        cellular: [
            0x3fc4_5808_0829_abc7,
            0x3feb_c637_f29e_5503,
            0xcd76_3cda_9cdf_ac58,
        ],
    },
    SpaceVector {
        seed: 0xdeadbeef,
        at: [
            0x3ff4_0000_0000_0000,
            0xc00e_0000_0000_0000,
            0x4001_0000_0000_0000,
        ],
        value: 0x3fcc_4330_783b_e5b0,
        gradient: 0x3fd2_7ba2_ce23_2520,
        fbm: 0x3fcb_42ca_e197_40d6,
        cellular: [
            0x3fc9_aa40_85ea_45c3,
            0x3fe0_277e_90c5_87e2,
            0x737c_3fe5_5c84_9019,
        ],
    },
    SpaceVector {
        seed: 0xdeadbeef,
        at: [
            0xc059_0666_6666_6666,
            0x406f_5ccc_cccc_cccd,
            0xc01d_3333_3333_3333,
        ],
        value: 0x3fe2_f9ea_4af2_9d7a,
        gradient: 0xbfd5_d2f4_d07b_f27e,
        fbm: 0xbfcc_c9f5_c8b9_84fd,
        cellular: [
            0x3fda_6d1d_6f73_cadb,
            0x3fdd_23dc_1bc6_0492,
            0x2554_d197_da97_9f92,
        ],
    },
    SpaceVector {
        seed: 0xdeadbeef,
        at: [
            0x40c8_1cd6_c8b4_3958,
            0x3f50_624d_d2f1_a9fc,
            0x4058_ff5c_28f5_c28f,
        ],
        value: 0x3fb4_83a3_6b9f_61e8,
        gradient: 0xbfbb_1fe4_631c_c70a,
        fbm: 0x3f51_2257_3f05_f742,
        cellular: [
            0x3fe2_4089_6b5a_9cf6,
            0x3fe2_e055_8e78_e13d,
            0xe04d_5377_957a_11c8,
        ],
    },
    SpaceVector {
        seed: u64::MAX,
        at: [
            0x0000_0000_0000_0000,
            0x0000_0000_0000_0000,
            0x0000_0000_0000_0000,
        ],
        value: 0x3fe0_2151_115f_8b72,
        gradient: 0x0000_0000_0000_0000,
        fbm: 0x0000_0000_0000_0000,
        cellular: [
            0x3fe0_d2ed_32be_a996,
            0x3fe1_6e43_2365_b511,
            0x683e_9b82_e16e_7841,
        ],
    },
    SpaceVector {
        seed: u64::MAX,
        at: [
            0x3fe0_0000_0000_0000,
            0x3fe0_0000_0000_0000,
            0x3fe0_0000_0000_0000,
        ],
        value: 0x3fdb_7d20_e2a9_69d3,
        gradient: 0xbf43_f338_34c7_a864,
        fbm: 0xbf34_97f7_f46b_0067,
        cellular: [
            0x3fdc_8b81_7424_d132,
            0x3fe9_bb91_1899_c4fe,
            0x1c35_c912_8d0e_3086,
        ],
    },
    SpaceVector {
        seed: u64::MAX,
        at: [
            0x3ff4_0000_0000_0000,
            0xc00e_0000_0000_0000,
            0x4001_0000_0000_0000,
        ],
        value: 0x3fe0_f4ac_8bd5_bf8b,
        gradient: 0xbfd1_2bfe_bc4d_133f,
        fbm: 0xbfcc_535a_2729_c3b9,
        cellular: [
            0x3fe1_03fa_c53a_f708,
            0x3fe1_ed1e_3b79_4e51,
            0x7434_2f77_2245_0201,
        ],
    },
    SpaceVector {
        seed: u64::MAX,
        at: [
            0xc059_0666_6666_6666,
            0x406f_5ccc_cccc_cccd,
            0xc01d_3333_3333_3333,
        ],
        value: 0x3fe4_a8b3_10c5_57c3,
        gradient: 0xbfc3_43a3_5f50_bb43,
        fbm: 0xbfc1_18c8_7e32_99a2,
        cellular: [
            0x3fdc_eb66_11f3_f8c7,
            0x3fe7_bea3_b793_6a49,
            0xa736_fecc_01e3_908f,
        ],
    },
    SpaceVector {
        seed: u64::MAX,
        at: [
            0x40c8_1cd6_c8b4_3958,
            0x3f50_624d_d2f1_a9fc,
            0x4058_ff5c_28f5_c28f,
        ],
        value: 0x3fe0_5cfa_ce4f_8ad7,
        gradient: 0xbfb4_5aa3_a61d_014b,
        fbm: 0x3f98_73dc_222c_cde3,
        cellular: [
            0x3fe7_d117_cd7e_cdba,
            0x3fed_1a70_2c88_d695,
            0x4724_cae2_9fac_3e06,
        ],
    },
];
