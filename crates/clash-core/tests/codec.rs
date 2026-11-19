//! Tests of `clash_core::codec`: the binary encoding and the blob frame.
//!
//! The byte vectors here pin the format down. Changing one changes every
//! save and every stored tract, and needs a decision entry and a bump of
//! the versions of the blobs that hold the affected type.

use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::fmt::Debug;

use clash_core::codec::{
    BLOB_HEADER_LEN, BLOB_MAGIC, BlobHeader, MAX_DEPTH, decode_blob, encode_blob, from_bytes,
    open_blob, read_blob_header, seal_blob, to_bytes, to_writer,
};
use clash_core::{
    CodecError, Dir8, Dur, EntityId, Hash128, IVec2, IVec3, LookupMap, Real, Rng, RngStreams, Rot4,
    StateHasher, Tick, Turn, Value, VarKey, VarMap, Vertical, define_id,
};
use proptest::prelude::*;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

define_id! {
    /// A dense id of the default width.
    pub struct WideId;
    /// A dense id sixteen bits wide.
    pub struct NarrowId(u16);
}

/// Encodes, decodes, and checks that the value and the bytes both come
/// back. Returns the bytes.
fn round_trip<T>(value: &T) -> Vec<u8>
where
    T: Serialize + DeserializeOwned + PartialEq + Debug,
{
    let bytes = to_bytes(value).expect("the value encodes");
    let back: T = from_bytes(&bytes).expect("its bytes decode");
    assert_eq!(&back, value);
    assert_eq!(to_bytes(&back).expect("it encodes again"), bytes);
    bytes
}

/// The same, and also through JSON: a type must work with both.
fn round_trip_both<T>(value: &T) -> Vec<u8>
where
    T: Serialize + DeserializeOwned + PartialEq + Debug,
{
    let json = serde_json::to_string(value).expect("the value is written as JSON");
    let back: T = serde_json::from_str(&json).expect("its JSON is read");
    assert_eq!(&back, value);
    round_trip(value)
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
struct Inner {
    id: u32,
    name: String,
    weight: Real,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
struct Marker;

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
struct Wrapper(i64);

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
struct Pair(u8, String);

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
enum Shape {
    Unit,
    Newtype(u32),
    Tuple(i8, String),
    Struct { a: bool, b: Option<Real> },
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
struct Outer {
    inner: Inner,
    list: Vec<Inner>,
    next: Option<Box<Outer>>,
    shapes: BTreeMap<String, Shape>,
    tuple: (u8, i16, char),
    marker: Marker,
    wrapper: Wrapper,
    pair: Pair,
    array: [u16; 3],
    bytes: Vec<u8>,
    set: BTreeSet<i32>,
    queue: VecDeque<bool>,
    nothing: (),
}

fn inner(id: u32) -> Inner {
    Inner {
        id,
        name: format!("thing {id}"),
        weight: Real::of(f64::from(id) * 1.5),
    }
}

fn outer(depth: u32) -> Outer {
    Outer {
        inner: inner(depth),
        list: (0..depth).map(inner).collect(),
        next: (depth > 0).then(|| Box::new(outer(depth - 1))),
        shapes: [
            ("unit".to_owned(), Shape::Unit),
            ("newtype".to_owned(), Shape::Newtype(70_000)),
            ("tuple".to_owned(), Shape::Tuple(-3, "three".to_owned())),
            (
                "struct".to_owned(),
                Shape::Struct {
                    a: true,
                    b: Some(Real::of(-0.25)),
                },
            ),
        ]
        .into_iter()
        .collect(),
        tuple: (255, -32_768, 'é'),
        marker: Marker,
        wrapper: Wrapper(i64::MIN),
        pair: Pair(9, "nine".to_owned()),
        array: [0, 300, u16::MAX],
        bytes: vec![0, 1, 254, 255],
        set: [-5, 0, 5].into_iter().collect(),
        queue: [true, false, true].into_iter().collect(),
        nothing: (),
    }
}

// ---- The format, byte for byte -------------------------------------------

#[test]
fn primitives_have_the_documented_bytes() {
    assert_eq!(round_trip(&false), [0]);
    assert_eq!(round_trip(&true), [1]);

    assert_eq!(round_trip(&0x7f_u8), [0x7f]);
    assert_eq!(round_trip(&0xff_u8), [0xff]);
    assert_eq!(round_trip(&-1_i8), [0xff]);
    assert_eq!(round_trip(&i8::MIN), [0x80]);

    assert_eq!(round_trip(&0_u16), [0]);
    assert_eq!(round_trip(&300_u16), [0xac, 0x02]);
    assert_eq!(round_trip(&u16::MAX), [0xff, 0xff, 0x03]);
    assert_eq!(round_trip(&127_u32), [0x7f]);
    assert_eq!(round_trip(&128_u32), [0x80, 0x01]);
    assert_eq!(round_trip(&u32::MAX), [0xff, 0xff, 0xff, 0xff, 0x0f]);
    let mut most = vec![0xff; 9];
    most.push(0x01);
    assert_eq!(round_trip(&u64::MAX), most);
    assert_eq!(round_trip(&u128::MAX).len(), 19);
    assert_eq!(round_trip(&7_usize), [7]);

    assert_eq!(round_trip(&0_i32), [0]);
    assert_eq!(round_trip(&-1_i32), [1]);
    assert_eq!(round_trip(&1_i32), [2]);
    assert_eq!(round_trip(&-2_i16), [3]);
    assert_eq!(round_trip(&i16::MIN), [0xff, 0xff, 0x03]);
    assert_eq!(round_trip(&i16::MAX), [0xfe, 0xff, 0x03]);
    assert_eq!(round_trip(&i64::MIN), most);
    assert_eq!(round_trip(&i128::MIN).len(), 19);
    assert_eq!(round_trip(&i128::MAX).len(), 19);
    assert_eq!(round_trip(&-7_isize), [13]);

    assert_eq!(round_trip(&1.0_f64), [0, 0, 0, 0, 0, 0, 0xf0, 0x3f]);
    assert_eq!(round_trip(&1.5_f32), [0, 0, 0xc0, 0x3f]);

    assert_eq!(round_trip(&'a'), [0x61]);
    assert_eq!(round_trip(&'é'), [0xe9, 0x01]);
    assert_eq!(round_trip(&char::MAX), [0xff, 0xff, 0x43]);

    assert_eq!(round_trip(&String::new()), [0]);
    assert_eq!(round_trip(&"hé".to_owned()), [3, b'h', 0xc3, 0xa9]);

    assert_eq!(round_trip(&()), [0_u8; 0]);
    assert_eq!(round_trip(&None::<u8>), [0]);
    assert_eq!(round_trip(&Some(7_u8)), [1, 7]);
    assert_eq!(round_trip(&Some(None::<u8>)), [1, 0]);
}

#[test]
fn compound_values_have_the_documented_bytes() {
    assert_eq!(round_trip(&(1_u8, 2_u8)), [1, 2]);
    assert_eq!(round_trip(&[5_u8, 6, 7]), [5, 6, 7]);
    assert_eq!(round_trip(&vec![1_u16, 2, 300]), [3, 1, 2, 0xac, 0x02]);
    assert_eq!(round_trip(&Vec::<u64>::new()), [0]);

    let map: BTreeMap<String, u8> = [("b".to_owned(), 2), ("a".to_owned(), 1)]
        .into_iter()
        .collect();
    assert_eq!(round_trip(&map), [2, 1, b'a', 1, 1, b'b', 2]);
    let set: BTreeSet<i32> = [2, -1].into_iter().collect();
    assert_eq!(round_trip(&set), [2, 1, 4]);

    assert_eq!(round_trip(&Marker), [0_u8; 0]);
    assert_eq!(round_trip(&Wrapper(-1)), [1]);
    assert_eq!(round_trip(&Pair(9, "x".to_owned())), [9, 1, b'x']);
    let thing = Inner {
        id: 1,
        name: "n".to_owned(),
        weight: Real::ZERO,
    };
    assert_eq!(round_trip(&thing), [1, 1, b'n', 0, 0, 0, 0, 0, 0, 0, 0]);
}

#[test]
fn every_shape_of_variant_has_the_documented_bytes() {
    assert_eq!(round_trip(&Shape::Unit), [0]);
    assert_eq!(round_trip(&Shape::Newtype(5)), [1, 5]);
    assert_eq!(
        round_trip(&Shape::Tuple(-1, "x".to_owned())),
        [2, 0xff, 1, b'x']
    );
    assert_eq!(round_trip(&Shape::Struct { a: true, b: None }), [3, 1, 0]);
    let with_real = Shape::Struct {
        a: false,
        b: Some(Real::ONE),
    };
    assert_eq!(
        round_trip(&with_real),
        [3, 0, 1, 0, 0, 0, 0, 0, 0, 0xf0, 0x3f]
    );
    // A variant that does not exist is refused by the type, not guessed.
    assert!(matches!(
        from_bytes::<Shape>(&[4]),
        Err(CodecError::Custom(_))
    ));
}

#[test]
fn nested_structures_round_trip() {
    let value = outer(3);
    let bytes = round_trip_both(&value);
    // No field names and no padding: far smaller than the JSON.
    let json = serde_json::to_string(&value).unwrap();
    assert!(bytes.len() * 2 < json.len(), "{} bytes", bytes.len());
}

#[test]
fn borrowed_text_and_bytes_are_read_in_place() {
    let bytes = to_bytes(&("ash", 3_u8)).unwrap();
    let (text, number): (&str, u8) = from_bytes(&bytes).unwrap();
    assert_eq!((text, number), ("ash", 3));
}

// ---- Real ----------------------------------------------------------------

#[test]
fn reals_round_trip_bit_for_bit() {
    let edge_cases = [
        0.0,
        -0.0,
        f64::INFINITY,
        f64::NEG_INFINITY,
        f64::MAX,
        f64::MIN,
        f64::MIN_POSITIVE,
        // Subnormals: the smallest of all, and the largest.
        5e-324,
        -5e-324,
        f64::MIN_POSITIVE / 2.0,
        f64::from_bits(0x000f_ffff_ffff_ffff),
        f64::EPSILON,
        0.1,
        -1.5e33,
        core::f64::consts::PI,
    ];
    for raw in edge_cases {
        let value = Real::new(raw).unwrap();
        let bytes = to_bytes(&value).unwrap();
        assert_eq!(bytes, raw.to_le_bytes());
        let back: Real = from_bytes(&bytes).unwrap();
        assert_eq!(back.get().to_bits(), raw.to_bits(), "{raw:e}");
    }
}

#[test]
fn a_real_that_would_be_nan_is_refused() {
    let bytes = f64::NAN.to_le_bytes();
    assert!(from_bytes::<f64>(&bytes).unwrap().is_nan());
    assert_eq!(
        from_bytes::<Real>(&bytes),
        Err(CodecError::Custom("value is not a number".to_owned()))
    );
}

// ---- Every type of clash-core ----------------------------------------------

#[test]
fn vectors_directions_and_turns_round_trip() {
    assert_eq!(round_trip_both(&IVec2::new(1, -2)), [2, 3]);
    assert_eq!(round_trip_both(&IVec3::new(1, -2, 3)), [2, 3, 6]);
    assert_eq!(
        round_trip_both(&IVec3::new(i32::MIN, i32::MAX, 0)).len(),
        11
    );
    for (index, direction) in Dir8::ALL.into_iter().enumerate() {
        assert_eq!(round_trip_both(&direction), [index as u8]);
    }
    assert_eq!(round_trip_both(&Vertical::Up), [0]);
    assert_eq!(round_trip_both(&Vertical::Down), [1]);
    for (index, rotation) in Rot4::ALL.into_iter().enumerate() {
        assert_eq!(round_trip_both(&rotation), [index as u8]);
    }
    for turn in Turn::ALL {
        let bytes = round_trip_both(&turn);
        assert_eq!(bytes.len(), 2);
    }
    assert_eq!(round_trip_both(&Turn::new(Rot4::R180, true)), [2, 1]);
}

#[test]
fn times_and_ids_round_trip() {
    assert_eq!(round_trip_both(&Tick(42)), [84]);
    assert_eq!(round_trip_both(&Dur(-7)), [13]);
    round_trip_both(&Tick::MAX);
    round_trip_both(&Dur(i64::MIN));

    assert_eq!(round_trip_both(&EntityId::new(7, 1)), [7, 1]);
    round_trip_both(&EntityId::new(u32::MAX, u32::MAX));

    assert_eq!(round_trip_both(&WideId::new(300)), [0xac, 0x02]);
    assert_eq!(round_trip_both(&NarrowId::new(5)), [5]);
    round_trip_both(&NarrowId::new(u16::MAX));
    // An id too large for its width is refused, as it is from JSON.
    assert_eq!(
        from_bytes::<NarrowId>(&to_bytes(&70_000_u32).unwrap()),
        Err(CodecError::BadVarint)
    );
}

#[test]
fn values_and_variables_round_trip() {
    assert_eq!(round_trip_both(&Value::from(true)), [0, 1]);
    assert_eq!(round_trip_both(&Value::from(3)), [1, 6]);
    assert_eq!(
        round_trip_both(&Value::from(Real::of(1.5))),
        [2, 0, 0, 0, 0, 0, 0, 0xf8, 0x3f]
    );
    assert_eq!(round_trip_both(&Value::from("x")), [3, 1, b'x']);
    assert_eq!(round_trip_both(&VarKey::new("m:a")), [3, b'm', b':', b'a']);

    let mut vars = VarMap::new();
    assert_eq!(round_trip_both(&vars), [0]);
    vars.set("core:kills", 2);
    vars.set("core:alarm_raised", true);
    vars.set("core:heat", Real::of(310.5));
    vars.set("core:name", "ash");
    let bytes = round_trip_both(&vars);
    assert_eq!(bytes[0], 4);
}

#[test]
fn lookup_maps_round_trip_in_key_order() {
    let forward: LookupMap<u32, String> = (0..50).map(|key| (key, format!("v{key}"))).collect();
    let backward: LookupMap<u32, String> =
        (0..50).rev().map(|key| (key, format!("v{key}"))).collect();
    let bytes = round_trip_both(&forward);
    // The bytes do not depend on how the map was built.
    assert_eq!(to_bytes(&backward).unwrap(), bytes);

    let small: LookupMap<u8, bool> = [(2, true), (1, false)].into_iter().collect();
    assert_eq!(round_trip_both(&small), [2, 1, 0, 2, 1]);
    assert_eq!(round_trip_both(&LookupMap::<u8, u8>::new()), [0]);
    // A key that appears twice is refused by the map itself.
    assert!(matches!(
        from_bytes::<LookupMap<u8, bool>>(&[2, 1, 0, 1, 1]),
        Err(CodecError::Custom(_))
    ));
}

#[test]
fn generators_and_streams_round_trip() {
    let mut rng = Rng::from_seed(2024);
    rng.next_u64();
    let bytes = round_trip_both(&rng);
    assert_eq!(bytes, to_bytes(&rng.state()).unwrap());
    let mut restored: Rng = from_bytes(&bytes).unwrap();
    assert_eq!(restored.next_u64(), rng.next_u64());
    // The state a generator can never be in is refused.
    assert!(matches!(
        from_bytes::<Rng>(&[0, 0, 0, 0]),
        Err(CodecError::Custom(_))
    ));

    let mut streams = RngStreams::new(13);
    round_trip_both(&streams);
    streams.stream("rules").next_u64();
    streams.stream("combat").below(6);
    let bytes = round_trip_both(&streams);
    let mut loaded: RngStreams = from_bytes(&bytes).unwrap();
    assert_eq!(Hash128::of(&loaded), Hash128::of(&streams));
    assert_eq!(
        loaded.stream("rules").next_u64(),
        streams.stream("rules").next_u64()
    );
}

#[test]
fn hashes_round_trip() {
    let hash = Hash128(0x0123_4567_89ab_cdef_0011_2233_4455_6677);
    let bytes = round_trip_both(&hash);
    // A hash is its 32 hexadecimal digits, as in JSON.
    assert_eq!(bytes[0], 32);
    assert_eq!(&bytes[1..], b"0123456789abcdef0011223344556677");
    round_trip_both(&Hash128::default());
    round_trip_both(&Hash128(u128::MAX));
}

// ---- Determinism -----------------------------------------------------------

#[test]
fn the_same_value_always_gives_the_same_bytes() {
    let first = to_bytes(&outer(4)).unwrap();
    for _ in 0..4 {
        assert_eq!(to_bytes(&outer(4)).unwrap(), first);
    }
    // And the bytes are these, on every platform and in every build.
    assert_eq!(first.len(), 725);
    assert_eq!(
        Hash128::of(&first).to_string(),
        "9f11e6b33cf085a4b6005d243467e9be"
    );
    let small = to_bytes(&(inner(2), Shape::Newtype(9), Some(-4_i64))).unwrap();
    assert_eq!(
        small,
        [
            2, 7, b't', b'h', b'i', b'n', b'g', b' ', b'2', 0, 0, 0, 0, 0, 0, 0x08, 0x40, 1, 9, 1,
            7
        ]
    );
}

#[test]
fn writing_to_a_writer_gives_the_same_bytes() {
    let value = outer(2);
    let mut written = Vec::new();
    to_writer(&mut written, &value).unwrap();
    assert_eq!(written, to_bytes(&value).unwrap());

    // A writer that fails is reported, not unwrapped.
    let mut too_small = [0_u8; 4];
    let error = to_writer(&mut too_small[..], &value).unwrap_err();
    assert!(matches!(
        error,
        CodecError::Write {
            kind: std::io::ErrorKind::WriteZero,
            ..
        }
    ));
    assert!(error.to_string().starts_with("could not write: "));
}

// ---- What cannot be encoded ------------------------------------------------

#[test]
fn what_the_format_cannot_hold_is_refused_with_a_reason() {
    #[derive(Serialize, Deserialize, Debug, PartialEq)]
    struct Skips {
        id: u32,
        #[serde(skip_serializing_if = "Option::is_none", default)]
        note: Option<String>,
    }
    let skipped = Skips { id: 1, note: None };
    assert_eq!(to_bytes(&skipped), Err(CodecError::SkippedField("note")));

    #[derive(Serialize, Deserialize, Debug, PartialEq)]
    struct Flattens {
        id: u32,
        #[serde(flatten)]
        rest: BTreeMap<String, u32>,
    }
    let flattened = Flattens {
        id: 1,
        rest: BTreeMap::new(),
    };
    assert_eq!(to_bytes(&flattened), Err(CodecError::UnknownLength));

    #[derive(Serialize, Deserialize, Debug, PartialEq)]
    #[serde(untagged)]
    enum Untagged {
        Number(u32),
        Text(String),
    }
    let bytes = to_bytes(&Untagged::Number(3)).unwrap();
    assert_eq!(
        from_bytes::<Untagged>(&bytes),
        Err(CodecError::NotSelfDescribing("deserialize_any"))
    );
    assert_eq!(
        from_bytes::<serde_json::Value>(&bytes),
        Err(CodecError::NotSelfDescribing("deserialize_any"))
    );
    assert_eq!(
        from_bytes::<serde::de::IgnoredAny>(&bytes),
        Err(CodecError::NotSelfDescribing("deserialize_ignored_any"))
    );
}

#[test]
fn a_sequence_must_state_its_length_and_keep_to_it() {
    struct NoLength;
    impl Serialize for NoLength {
        fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
            serializer.collect_seq((0..5_u8).filter(|number| number % 2 == 0))
        }
    }
    assert_eq!(to_bytes(&NoLength), Err(CodecError::UnknownLength));

    struct Short;
    impl Serialize for Short {
        fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
            use serde::ser::SerializeSeq;
            let mut sequence = serializer.serialize_seq(Some(3))?;
            sequence.serialize_element(&1_u8)?;
            sequence.serialize_element(&2_u8)?;
            sequence.end()
        }
    }
    assert_eq!(
        to_bytes(&Short),
        Err(CodecError::LengthMismatch {
            stated: 3,
            written: 2
        })
    );
}

#[test]
fn items_that_take_no_bytes_are_refused_in_both_directions() {
    assert_eq!(to_bytes(&vec![(); 3]), Err(CodecError::EmptyItem));
    assert_eq!(to_bytes(&vec![Marker, Marker]), Err(CodecError::EmptyItem));
    assert_eq!(to_bytes(&vec![[0_u8; 0]]), Err(CodecError::EmptyItem));
    let empty_entries: BTreeMap<(), ()> = [((), ())].into_iter().collect();
    assert_eq!(to_bytes(&empty_entries), Err(CodecError::EmptyItem));
    // With no items there is nothing to refuse.
    assert_eq!(round_trip(&Vec::<()>::new()), [0]);
    // A map's keys take bytes even when its values do not.
    let map: BTreeMap<u8, ()> = [(1, ()), (2, ())].into_iter().collect();
    assert_eq!(round_trip(&map), [2, 1, 2]);
    // Fields are another matter: their number comes from the type.
    assert_eq!(round_trip(&((), 7_u8, Marker, [(); 4])), [7]);

    // The decoder would not have read such a sequence either. Its count
    // is more than the bytes that follow,
    assert_eq!(
        from_bytes::<Vec<()>>(&[3]),
        Err(CodecError::LengthOutOfBounds {
            length: 3,
            remaining: 0
        })
    );
    // or, where other bytes follow, its first item reads nothing.
    assert_eq!(
        from_bytes::<(Vec<()>, u8)>(&[1, 9]),
        Err(CodecError::EmptyItem)
    );
    assert_eq!(
        from_bytes::<BTreeMap<(), ()>>(&[1, 9]),
        Err(CodecError::EmptyItem)
    );
}

#[test]
fn decoding_never_does_more_work_than_the_input_is_long() {
    // Sequences of sequences of nothing, each claiming as many items as
    // there are bytes left: if the inner loops ran, a megabyte of this
    // would take 10^11 steps. Each stops at its first item.
    let mut input = Vec::new();
    for _ in 0..200_000 {
        // A count of 100000, in three bytes.
        input.extend([0xa0, 0x8d, 0x06]);
    }
    assert_eq!(
        from_bytes::<Vec<Vec<()>>>(&input),
        Err(CodecError::EmptyItem)
    );
    assert_eq!(
        from_bytes::<Vec<Vec<Vec<Marker>>>>(&input),
        Err(CodecError::EmptyItem)
    );
    assert_eq!(
        from_bytes::<Vec<BTreeMap<(), Vec<()>>>>(&input),
        Err(CodecError::EmptyItem)
    );
}

#[derive(Serialize, Deserialize, Debug, PartialEq)]
enum Chain {
    End,
    Link(Box<Chain>),
}

fn chain(links: usize) -> Chain {
    let mut chain = Chain::End;
    for _ in 0..links {
        chain = Chain::Link(Box::new(chain));
    }
    chain
}

#[test]
fn nesting_is_limited_in_both_directions() {
    let deepest = chain(MAX_DEPTH);
    let bytes = round_trip(&deepest);
    assert_eq!(bytes.len(), MAX_DEPTH + 1);

    assert_eq!(to_bytes(&chain(MAX_DEPTH + 1)), Err(CodecError::TooDeep));

    // Corrupt input that describes a chain a million links long is
    // refused long before the stack is in danger.
    let endless = vec![1_u8; 1_000_000];
    assert_eq!(from_bytes::<Chain>(&endless), Err(CodecError::TooDeep));
    let mut too_deep = vec![1_u8; MAX_DEPTH + 1];
    too_deep.push(0);
    assert_eq!(from_bytes::<Chain>(&too_deep), Err(CodecError::TooDeep));
}

// ---- Corrupt input ----------------------------------------------------------

#[test]
fn damaged_input_is_an_error_that_says_what_is_wrong() {
    assert_eq!(
        from_bytes::<u32>(&[]),
        Err(CodecError::UnexpectedEnd {
            needed: 1,
            remaining: 0
        })
    );
    assert_eq!(
        from_bytes::<f64>(&[1, 2, 3]),
        Err(CodecError::UnexpectedEnd {
            needed: 8,
            remaining: 3
        })
    );
    assert_eq!(from_bytes::<u8>(&[1, 2]), Err(CodecError::TrailingBytes(1)));
    assert_eq!(from_bytes::<()>(&[9, 9]), Err(CodecError::TrailingBytes(2)));
    assert_eq!(from_bytes::<bool>(&[2]), Err(CodecError::InvalidBool(2)));
    assert_eq!(
        from_bytes::<Option<u8>>(&[2, 0]),
        Err(CodecError::InvalidOptionTag(2))
    );
    // A surrogate is not a character, and neither is anything past the
    // end of Unicode.
    assert_eq!(
        from_bytes::<char>(&to_bytes(&0xd800_u32).unwrap()),
        Err(CodecError::InvalidChar(0xd800))
    );
    assert_eq!(
        from_bytes::<char>(&to_bytes(&0x11_0000_u32).unwrap()),
        Err(CodecError::InvalidChar(0x11_0000))
    );
    assert_eq!(
        from_bytes::<String>(&[2, 0xff, 0xfe]),
        Err(CodecError::InvalidUtf8)
    );
    assert_eq!(from_bytes::<u16>(&[0x80, 0x00]), Err(CodecError::BadVarint));
    assert_eq!(
        from_bytes::<u16>(&[0xff, 0xff, 0x04]),
        Err(CodecError::BadVarint)
    );
    // A text that claims more bytes than there are.
    assert_eq!(
        from_bytes::<String>(&[200, 1, b'a']),
        Err(CodecError::LengthOutOfBounds {
            length: 200,
            remaining: 1
        })
    );
}

#[test]
fn a_huge_count_reserves_nothing() {
    // Each of these claims about 2^63 items in ten bytes of input. If any
    // of them reserved what it claims, the test would not survive.
    let mut huge = vec![0xff; 8];
    huge.push(0x7f);
    assert!(matches!(
        from_bytes::<Vec<u64>>(&huge),
        Err(CodecError::LengthOutOfBounds { .. })
    ));
    assert!(matches!(
        from_bytes::<String>(&huge),
        Err(CodecError::LengthOutOfBounds { .. })
    ));
    assert!(matches!(
        from_bytes::<BTreeMap<u8, u8>>(&huge),
        Err(CodecError::LengthOutOfBounds { .. })
    ));
    assert!(matches!(
        from_bytes::<LookupMap<u8, u8>>(&huge),
        Err(CodecError::LengthOutOfBounds { .. })
    ));
    assert!(matches!(
        from_bytes::<Vec<Vec<String>>>(&huge),
        Err(CodecError::LengthOutOfBounds { .. })
    ));
}

// ---- Blobs -------------------------------------------------------------------

const KIND: [u8; 4] = *b"TEST";
const OTHER_KIND: [u8; 4] = *b"ELSE";

/// A body with enough repetition to compress.
fn blob_body() -> Vec<Inner> {
    (0..200).map(|index| inner(index % 7)).collect()
}

#[test]
fn a_blob_round_trips_and_is_smaller_than_its_body() {
    let body = blob_body();
    let blob = encode_blob(KIND, 3, &body).unwrap();
    let (version, back): (u32, Vec<Inner>) = decode_blob(KIND, &blob).unwrap();
    assert_eq!(version, 3);
    assert_eq!(back, body);
    let plain = to_bytes(&body).unwrap();
    assert!(blob.len() < plain.len() / 2, "{} bytes", blob.len());

    // In two steps, for a caller that picks a reader by version.
    let (version, bytes) = open_blob(KIND, &blob).unwrap();
    assert_eq!(version, 3);
    assert_eq!(bytes, plain);
    assert_eq!(seal_blob(KIND, 3, &plain).unwrap(), blob);
}

#[test]
fn a_blob_has_the_documented_frame() {
    let body = to_bytes(&blob_body()).unwrap();
    let blob = seal_blob(KIND, 0x0102_0304, &body).unwrap();
    assert_eq!(&blob[0..4], b"C2DB");
    assert_eq!(blob[0..4], BLOB_MAGIC);
    assert_eq!(blob[4..8], KIND);
    assert_eq!(blob[8..12], [4, 3, 2, 1]);
    assert_eq!(blob[12..16], (body.len() as u32).to_le_bytes());

    // The checksum is the engine's hasher over kind, version and body.
    let mut hasher = StateHasher::new();
    hasher.write_bytes(&KIND);
    hasher.write_u32(0x0102_0304);
    hasher.write_bytes(&body);
    let checksum = hasher.finish();
    assert_eq!(blob[16..32], checksum.0.to_le_bytes());

    // The rest is one LZ4 block.
    let mut unpacked = vec![0_u8; body.len()];
    let written = lz4_flex::block::decompress_into(&blob[BLOB_HEADER_LEN..], &mut unpacked);
    assert_eq!(written.ok(), Some(body.len()));
    assert_eq!(unpacked, body);

    assert_eq!(
        read_blob_header(&blob),
        Ok(BlobHeader {
            kind: KIND,
            version: 0x0102_0304,
            body_len: body.len(),
            checksum,
        })
    );
}

#[test]
fn a_blob_of_nothing_and_a_blob_of_noise_both_work() {
    let empty = encode_blob(KIND, 1, &()).unwrap();
    assert_eq!(empty.len(), BLOB_HEADER_LEN + 1);
    assert_eq!(decode_blob::<()>(KIND, &empty), Ok((1, ())));

    // Bytes that do not compress still round trip.
    let mut rng = Rng::from_seed(5);
    let noise: Vec<u64> = (0..500).map(|_| rng.next_u64()).collect();
    let blob = encode_blob(KIND, u32::MAX, &noise).unwrap();
    assert_eq!(decode_blob::<Vec<u64>>(KIND, &blob), Ok((u32::MAX, noise)));
}

#[test]
fn each_way_a_blob_can_be_wrong_is_its_own_error() {
    let body = blob_body();
    let good = encode_blob(KIND, 3, &body).unwrap();
    let open = |blob: &[u8]| open_blob(KIND, blob);

    // Too short to hold a header.
    for length in [0, 1, BLOB_HEADER_LEN - 1] {
        assert_eq!(
            open(&good[..length]),
            Err(CodecError::UnexpectedEnd {
                needed: BLOB_HEADER_LEN,
                remaining: length
            })
        );
    }

    // Not a blob.
    let mut blob = good.clone();
    blob[0] = b'X';
    assert_eq!(open(&blob), Err(CodecError::WrongMagic { found: *b"X2DB" }));

    // A blob of something else.
    assert_eq!(
        open_blob(OTHER_KIND, &good),
        Err(CodecError::WrongKind {
            expected: OTHER_KIND,
            found: KIND
        })
    );
    assert!(decode_blob::<Vec<Inner>>(OTHER_KIND, &good).is_err());

    // A length that is not the body's: one more, one less, and absurd.
    let stated = u32::from_le_bytes([good[12], good[13], good[14], good[15]]);
    for wrong in [stated + 1, stated - 1, 0, u32::MAX] {
        let mut blob = good.clone();
        blob[12..16].copy_from_slice(&wrong.to_le_bytes());
        assert_eq!(
            open(&blob),
            Err(CodecError::WrongLength {
                stated: wrong as usize
            }),
            "stated {wrong}"
        );
    }

    // A header with no body after it.
    assert_eq!(
        open(&good[..BLOB_HEADER_LEN]),
        Err(CodecError::WrongLength {
            stated: stated as usize
        })
    );

    // A body that is not an LZ4 block: a copy from before the start.
    let mut blob = good[..BLOB_HEADER_LEN].to_vec();
    blob.extend([0x00, 0x05, 0x00]);
    blob[12..16].copy_from_slice(&64_u32.to_le_bytes());
    assert_eq!(open(&blob), Err(CodecError::BadCompression));

    // A checksum that does not match: damage to the checksum itself, to
    // the version, and to the body.
    for at in [16, 31, 8] {
        let mut blob = good.clone();
        blob[at] ^= 0x01;
        assert!(
            matches!(open(&blob), Err(CodecError::WrongChecksum { .. })),
            "byte {at}"
        );
    }
    let plain = to_bytes(&body).unwrap();
    let mut changed = plain.clone();
    changed[10] ^= 0x40;
    let mut blob = seal_blob(KIND, 3, &changed).unwrap();
    blob[16..32].copy_from_slice(&good[16..32]);
    let error = open(&blob).unwrap_err();
    let CodecError::WrongChecksum { stated, found } = &error else {
        panic!("expected a checksum error, got {error:?}");
    };
    assert_ne!(stated, found);
    assert!(error.to_string().contains(&stated.to_string()));

    // A sound frame around a body of another type.
    assert!(decode_blob::<Vec<Outer>>(KIND, &good).is_err());
    let padded = seal_blob(KIND, 3, &[plain.as_slice(), &[0]].concat()).unwrap();
    assert_eq!(
        decode_blob::<Vec<Inner>>(KIND, &padded),
        Err(CodecError::TrailingBytes(1))
    );
}

#[test]
fn error_messages_are_for_a_person() {
    let messages = [
        CodecError::UnknownLength.to_string(),
        CodecError::TooDeep.to_string(),
        CodecError::TrailingBytes(3).to_string(),
        CodecError::InvalidChar(0xd800).to_string(),
        CodecError::WrongMagic { found: *b"PNG\n" }.to_string(),
        CodecError::WrongLength { stated: 9 }.to_string(),
        CodecError::NotSelfDescribing("deserialize_any").to_string(),
    ];
    assert_eq!(
        messages,
        [
            "a sequence or map must state its length before its items",
            "value is nested more than 128 levels deep",
            "3 bytes are left over after the value",
            "`0xd800` is not a character",
            "not a blob: it starts with `PNG\\n`",
            "blob does not hold the 9 bytes its header states",
            "the encoding does not describe itself, so `deserialize_any` cannot be answered",
        ]
    );
}

// ---- Properties ---------------------------------------------------------------

/// Random nested data that uses every part of the format.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
enum Tree {
    Leaf,
    Flag(bool),
    Small(i8, u8),
    Int(i64),
    Wide(u128, i128),
    Real(Real),
    Single(f32),
    Letter(char),
    Text(String),
    Bytes(Vec<u8>),
    Maybe(Option<Box<Tree>>),
    List(Vec<Tree>),
    Map(BTreeMap<String, Tree>),
    Pair(Box<Tree>, Box<Tree>),
    Record {
        id: u32,
        at: (i16, i32),
        child: Box<Tree>,
    },
}

fn real() -> impl Strategy<Value = Real> {
    any::<u64>().prop_map(|bits| Real::of(f64::from_bits(bits)))
}

fn single() -> impl Strategy<Value = f32> {
    any::<u32>()
        .prop_map(f32::from_bits)
        .prop_map(|value| if value.is_nan() { 0.0 } else { value })
}

fn tree() -> impl Strategy<Value = Tree> {
    let leaf = prop_oneof![
        Just(Tree::Leaf),
        any::<bool>().prop_map(Tree::Flag),
        (any::<i8>(), any::<u8>()).prop_map(|(a, b)| Tree::Small(a, b)),
        any::<i64>().prop_map(Tree::Int),
        (any::<u128>(), any::<i128>()).prop_map(|(a, b)| Tree::Wide(a, b)),
        real().prop_map(Tree::Real),
        single().prop_map(Tree::Single),
        any::<char>().prop_map(Tree::Letter),
        ".{0,12}".prop_map(Tree::Text),
        prop::collection::vec(any::<u8>(), 0..24).prop_map(Tree::Bytes),
    ];
    leaf.prop_recursive(5, 96, 6, |inner| {
        prop_oneof![
            prop::option::of(inner.clone()).prop_map(|child| Tree::Maybe(child.map(Box::new))),
            prop::collection::vec(inner.clone(), 0..6).prop_map(Tree::List),
            prop::collection::btree_map(".{0,6}", inner.clone(), 0..6).prop_map(Tree::Map),
            (inner.clone(), inner.clone()).prop_map(|(a, b)| Tree::Pair(Box::new(a), Box::new(b))),
            (any::<u32>(), any::<i16>(), any::<i32>(), inner).prop_map(|(id, x, y, child)| {
                Tree::Record {
                    id,
                    at: (x, y),
                    child: Box::new(child),
                }
            }),
        ]
    })
}

/// Whether two trees are the same to the last bit: `==` on a float calls
/// the two zeros equal, and the encoding must not.
fn same_bits(a: &Tree, b: &Tree) -> bool {
    to_bytes(a).ok() == to_bytes(b).ok()
}

proptest! {
    #[test]
    fn decoding_undoes_encoding(value in tree()) {
        let bytes = to_bytes(&value).unwrap();
        let back: Tree = from_bytes(&bytes).unwrap();
        prop_assert_eq!(&back, &value);
        prop_assert!(same_bits(&back, &value));
        // One value, one encoding.
        prop_assert_eq!(to_bytes(&back).unwrap(), bytes);
    }

    #[test]
    fn integers_of_every_width_round_trip(
        a in any::<u16>(), b in any::<u32>(), c in any::<u64>(), d in any::<u128>(),
        e in any::<i16>(), f in any::<i32>(), g in any::<i64>(), h in any::<i128>(),
    ) {
        let value = (a, b, c, d, e, f, g, h);
        let bytes = to_bytes(&value).unwrap();
        prop_assert_eq!(from_bytes::<(u16, u32, u64, u128, i16, i32, i64, i128)>(&bytes), Ok(value));
    }

    #[test]
    fn every_truncation_of_an_encoding_is_an_error(value in tree()) {
        let bytes = to_bytes(&value).unwrap();
        for length in 0..bytes.len() {
            prop_assert!(from_bytes::<Tree>(&bytes[..length]).is_err(), "cut at {}", length);
        }
    }

    #[test]
    fn random_bytes_never_panic(bytes in prop::collection::vec(any::<u8>(), 0..256)) {
        // Whatever these return, they return.
        let _ = from_bytes::<Tree>(&bytes);
        let _ = from_bytes::<Outer>(&bytes);
        let _ = from_bytes::<Vec<Vec<String>>>(&bytes);
        let _ = from_bytes::<BTreeMap<String, Vec<Real>>>(&bytes);
        let _ = from_bytes::<LookupMap<u32, Value>>(&bytes);
        let _ = from_bytes::<RngStreams>(&bytes);
        let _ = from_bytes::<VarMap>(&bytes);
        let _ = open_blob(KIND, &bytes);
        let _ = read_blob_header(&bytes);
    }

    #[test]
    fn a_damaged_encoding_never_panics(
        value in tree(),
        at in any::<prop::sample::Index>(),
        change in 1..=255_u8,
    ) {
        let mut bytes = to_bytes(&value).unwrap();
        let index = at.index(bytes.len());
        bytes[index] ^= change;
        // It may decode to some other tree or fail. If it decodes, what
        // it decoded encodes again.
        if let Ok(other) = from_bytes::<Tree>(&bytes) {
            prop_assert!(to_bytes(&other).is_ok());
        }
    }

    #[test]
    fn a_damaged_blob_is_refused_or_harmless(
        value in tree(),
        version in any::<u32>(),
        at in any::<prop::sample::Index>(),
        change in 1..=255_u8,
        cut in any::<prop::sample::Index>(),
    ) {
        let good = encode_blob(KIND, version, &value).unwrap();
        prop_assert!(same_bits(&decode_blob::<Tree>(KIND, &good).unwrap().1, &value));

        // One byte changed anywhere: an error, or (when the change was to
        // compressed bytes that unpack to the same body) the same value.
        let mut blob = good.clone();
        let index = at.index(blob.len());
        blob[index] ^= change;
        match decode_blob::<Tree>(KIND, &blob) {
            Err(_) => {}
            Ok((read_version, read)) => {
                prop_assert_eq!(read_version, version);
                prop_assert!(same_bits(&read, &value));
            }
        }

        // Cut short anywhere: always an error.
        let length = cut.index(good.len());
        prop_assert!(decode_blob::<Tree>(KIND, &good[..length]).is_err());
    }

    #[test]
    fn a_blob_with_a_random_body_never_panics(
        payload in prop::collection::vec(any::<u8>(), 0..200),
        stated in 0..4096_u32,
    ) {
        let mut blob = seal_blob(KIND, 1, &[]).unwrap();
        blob.truncate(BLOB_HEADER_LEN);
        blob[12..16].copy_from_slice(&stated.to_le_bytes());
        blob.extend(payload);
        prop_assert!(open_blob(KIND, &blob).is_err() || stated == 0);
    }
}
