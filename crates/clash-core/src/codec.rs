//! The engine's binary encoding: a compact serde format, and a framed,
//! compressed, checksummed **blob** for what is stored on disk.
//!
//! Every piece of simulation state implements serde. JSON is for people
//! and for mods; this is for saves and tract blobs (01 §11.1, 02 §11). It
//! is written here rather than taken from a crate so that the bytes are
//! the engine's own contract.
//!
//! # The encoding
//!
//! [`to_bytes`] and [`from_bytes`] turn a value into bytes and back. The
//! format does not describe itself: the bytes hold values only, and the
//! type being read says what they mean.
//!
//! | Value | Bytes |
//! |-------|-------|
//! | `bool` | One byte, `0` or `1` |
//! | `u8`, `i8` | One byte |
//! | `u16` to `u128`, lengths, `char` | LEB128: seven bits a byte, lowest first, the top bit set on every byte but the last, in the fewest bytes |
//! | `i16` to `i128` | Zig-zag (`0, -1, 1, -2, …` become `0, 1, 2, 3, …`), then LEB128 |
//! | `f32`, `f64`, so [`Real`](crate::Real) | The IEEE bits, little-endian: 4 or 8 bytes, exact |
//! | Text, bytes | The length in bytes, then the bytes |
//! | `Option` | A byte `0` for none; a byte `1` and the value for some |
//! | Unit, unit struct | Nothing |
//! | Struct, tuple, array | Each field in order, with no names and no count |
//! | Sequence, set | The number of items, then each item |
//! | Map | The number of entries, then each key and value |
//! | Enum | The variant's index as LEB128, then the variant's fields in order |
//!
//! A value has exactly one encoding, so equal values give equal bytes.
//!
//! # What a type must do to be encoded
//!
//! Derived `Serialize` and `Deserialize` work as they are. Because there
//! are no field names in the bytes, a few serde features cannot work and
//! are refused with an error rather than writing something unreadable:
//!
//! - `#[serde(skip_serializing_if = …)]`: every field must be written.
//!   (A field skipped in one direction only, with `skip_serializing` or
//!   `skip_deserializing`, cannot be detected and will not read back:
//!   use `#[serde(skip)]`, which leaves it out both ways.)
//! - `#[serde(flatten)]`, `#[serde(untagged)]`, `#[serde(tag = …)]` and
//!   anything else that reads through `deserialize_any`.
//! - A sequence or map that does not state its length up front.
//!
//! Reordering, adding or removing a field or a variant changes the bytes.
//! That is what a blob's version number is for.
//!
//! # Corrupt input
//!
//! Decoding never panics, never allocates more than a small multiple of
//! the input and never does more work than the input is long: every
//! length is checked against the bytes that remain before anything is
//! reserved, nesting is limited to [`MAX_DEPTH`], and bytes left over are
//! an error. Two consequences are part of the format, and hold in both
//! directions:
//!
//! - Every item of a sequence, and every entry of a map, takes at least
//!   one byte. An item that takes none (the items of a `Vec<()>`) cannot
//!   be bounded by the input, so it is refused.
//! - A value nested deeper than [`MAX_DEPTH`] is refused.
//!
//! # Blobs
//!
//! Anything written to disk goes in a frame made by [`encode_blob`]:
//!
//! | Offset | Size | Holds |
//! |--------|------|-------|
//! | 0 | 4 | The magic, [`BLOB_MAGIC`] |
//! | 4 | 4 | The kind: four bytes chosen by the owner of the data |
//! | 8 | 4 | The version of the body's layout, `u32` little-endian |
//! | 12 | 4 | The length of the body before compression, `u32` little-endian |
//! | 16 | 16 | The checksum, `u128` little-endian |
//! | 32 | … | The body, compressed as one LZ4 block |
//!
//! The checksum is the engine's [`StateHasher`] (MurmurHash3 `x64_128`,
//! seed 0) over the kind, the version as four little-endian bytes and the
//! uncompressed body, in that order. It catches damage; it does not
//! resist tampering.
//!
//! [`decode_blob`] checks all of it and reads the body. To choose how to
//! read a body by its version, use [`open_blob`], which checks the frame
//! and hands back the version and the body's bytes for [`from_bytes`].
//!
//! ```
//! use clash_core::codec::{decode_blob, encode_blob, open_blob, from_bytes};
//!
//! const NOTES: [u8; 4] = *b"NOTE";
//!
//! let blob = encode_blob(NOTES, 2, &("ash", 7_u32)).unwrap();
//! let (version, note): (u32, (String, u32)) = decode_blob(NOTES, &blob).unwrap();
//! assert_eq!((version, note), (2, ("ash".to_owned(), 7)));
//!
//! // Or in two steps, to pick a reader by version.
//! let (version, body) = open_blob(NOTES, &blob).unwrap();
//! let note: (String, u32) = match version {
//!     2 => from_bytes(&body).unwrap(),
//!     _ => unreachable!("older layouts would be migrated here"),
//! };
//! assert_eq!(note.1, 7);
//! ```
//!
//! The compressed bytes are not a contract: a later compressor may pack
//! the same body differently. Compare and hash bodies, never blobs.

use core::fmt;

use serde::de::{
    self, DeserializeOwned, DeserializeSeed, EnumAccess, MapAccess, SeqAccess, VariantAccess,
    Visitor,
};
use serde::ser::{self, Serialize};
use serde::{Deserialize, Deserializer, Serializer};
use thiserror::Error;

use crate::hash::{Hash128, StateHasher};

/// How deeply a value may be nested: a struct in a sequence in an option
/// is three levels. Deeper values are refused when encoding and when
/// decoding, so that corrupt input describing a recursive type cannot
/// exhaust the stack.
pub const MAX_DEPTH: usize = 128;

/// The first four bytes of every blob.
pub const BLOB_MAGIC: [u8; 4] = *b"C2DB";

/// The number of bytes in a blob before its compressed body.
pub const BLOB_HEADER_LEN: usize = 32;

/// The largest body a blob can hold, in bytes before compression: the
/// length is stored in 32 bits.
pub const MAX_BLOB_BODY: usize = u32::MAX as usize;

/// The most bytes one byte of an LZ4 block can stand for. A blob whose
/// header claims a longer body than its compressed bytes could hold is
/// refused before any memory is reserved for it.
const LZ4_MAX_EXPANSION: usize = 255;

/// The number of value bits in each byte of a LEB128 number.
const VARINT_BITS: u32 = 7;
/// The mask for the value bits of a LEB128 byte.
const VARINT_MASK: u8 = 0x7f;
/// The bit of a LEB128 byte that says another byte follows.
const VARINT_MORE: u8 = 0x80;

/// The byte that stands for `None`.
const TAG_NONE: u8 = 0;
/// The byte that comes before the value of a `Some`.
const TAG_SOME: u8 = 1;

/// Why a value could not be encoded or decoded.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[non_exhaustive]
pub enum CodecError {
    /// A type's own `Serialize` or `Deserialize` implementation refused:
    /// a `Real` that would be NaN, an enum variant that does not exist, a
    /// key that appears twice.
    #[error("{0}")]
    Custom(String),

    /// Encoding: a sequence or map did not state its length before its
    /// items. Collect it into a `Vec` or a `BTreeMap` first. A struct
    /// with `#[serde(flatten)]` also ends up here.
    #[error("a sequence or map must state its length before its items")]
    UnknownLength,

    /// Encoding: a sequence or map wrote a different number of items than
    /// it said it would.
    #[error("a sequence or map stated {stated} items but wrote {written}")]
    LengthMismatch {
        /// The number of items it promised.
        stated: usize,
        /// The number of items it wrote.
        written: usize,
    },

    /// An item of a sequence, or an entry of a map, takes no bytes, as
    /// the items of a `Vec<()>` do. The number of such items could not be
    /// checked against the input, so they are refused in both directions.
    #[error("an item of a sequence or map takes no bytes, so their number cannot be checked")]
    EmptyItem,

    /// Encoding: a struct left a field out (`skip_serializing_if`). The
    /// bytes carry no field names, so every field must be written.
    #[error("field `{0}` was skipped, but every field must be written")]
    SkippedField(&'static str),

    /// The value is nested deeper than [`MAX_DEPTH`].
    #[error("value is nested more than {MAX_DEPTH} levels deep")]
    TooDeep,

    /// Encoding: the body is too large for a blob (see [`MAX_BLOB_BODY`]).
    #[error("a body of {0} bytes is too large for a blob")]
    BodyTooLarge(usize),

    /// [`to_writer`]'s writer failed.
    #[error("could not write: {message}")]
    Write {
        /// The kind of failure the writer reported.
        kind: std::io::ErrorKind,
        /// The writer's own message.
        message: String,
    },

    /// Decoding: the input ended before the value did.
    #[error("input ends early: {needed} more bytes are needed and {remaining} remain")]
    UnexpectedEnd {
        /// The number of bytes the next part of the value takes.
        needed: usize,
        /// The number of bytes that were left.
        remaining: usize,
    },

    /// Decoding: bytes were left over after the value.
    #[error("{0} bytes are left over after the value")]
    TrailingBytes(usize),

    /// Decoding: a text, a sequence or a map states a length greater than
    /// the number of bytes left, so it cannot be real.
    #[error("a length of {length} is more than the {remaining} bytes that remain")]
    LengthOutOfBounds {
        /// The length that was read.
        length: u64,
        /// The number of bytes that were left.
        remaining: usize,
    },

    /// Decoding: a LEB128 number is too large for its type, or is not
    /// written in the fewest bytes.
    #[error("a number is too large for its type or is not in its shortest form")]
    BadVarint,

    /// Decoding: a boolean was neither `0` nor `1`.
    #[error("`{0}` is not a boolean")]
    InvalidBool(u8),

    /// Decoding: an option's tag was neither `0` nor `1`.
    #[error("`{0}` is not an option tag")]
    InvalidOptionTag(u8),

    /// Decoding: a number is not a Unicode scalar value.
    #[error("`{0:#x}` is not a character")]
    InvalidChar(u32),

    /// Decoding: a text is not valid UTF-8.
    #[error("text is not valid UTF-8")]
    InvalidUtf8,

    /// Decoding: the type asked the format what comes next. The format
    /// does not know. The name is the serde call that was made.
    #[error("the encoding does not describe itself, so `{0}` cannot be answered")]
    NotSelfDescribing(&'static str),

    /// A blob does not start with [`BLOB_MAGIC`]: it is not a blob.
    #[error("not a blob: it starts with `{}`", tag_text(.found))]
    WrongMagic {
        /// The four bytes found where the magic belongs.
        found: [u8; 4],
    },

    /// A blob is of another kind than the one asked for.
    #[error("blob is of kind `{}`, not `{}`", tag_text(.found), tag_text(.expected))]
    WrongKind {
        /// The kind asked for.
        expected: [u8; 4],
        /// The kind the blob states.
        found: [u8; 4],
    },

    /// A blob's body is not as long as its header states.
    #[error("blob does not hold the {stated} bytes its header states")]
    WrongLength {
        /// The body length in the header, in bytes.
        stated: usize,
    },

    /// A blob's compressed body is not a valid LZ4 block.
    #[error("blob's compressed body is damaged")]
    BadCompression,

    /// A blob's kind, version or body does not match its checksum.
    #[error("blob's checksum is {stated} but its contents hash to {found}")]
    WrongChecksum {
        /// The checksum in the header.
        stated: Hash128,
        /// The checksum of what the blob holds.
        found: Hash128,
    },
}

/// A blob tag as text, with bytes that are not printable ASCII escaped.
fn tag_text(tag: &[u8; 4]) -> String {
    tag.escape_ascii().to_string()
}

impl ser::Error for CodecError {
    fn custom<T: fmt::Display>(message: T) -> CodecError {
        CodecError::Custom(message.to_string())
    }
}

impl de::Error for CodecError {
    fn custom<T: fmt::Display>(message: T) -> CodecError {
        CodecError::Custom(message.to_string())
    }
}

/// Encodes a value.
///
/// Equal values give equal bytes, on every platform.
///
/// ```
/// use clash_core::codec::{from_bytes, to_bytes};
///
/// let bytes = to_bytes(&(300_u32, -1_i32, "ab")).unwrap();
/// assert_eq!(bytes, [0xac, 0x02, 0x01, 2, b'a', b'b']);
/// let back: (u32, i32, String) = from_bytes(&bytes).unwrap();
/// assert_eq!(back, (300, -1, "ab".to_owned()));
/// ```
pub fn to_bytes<T: Serialize + ?Sized>(value: &T) -> Result<Vec<u8>, CodecError> {
    let mut encoder = Encoder {
        out: Vec::new(),
        depth: 0,
    };
    value.serialize(&mut encoder)?;
    Ok(encoder.out)
}

/// Encodes a value and writes the bytes to a writer.
///
/// The value is encoded in memory first and written in one call, so a
/// value that cannot be encoded writes nothing.
pub fn to_writer<W: std::io::Write, T: Serialize + ?Sized>(
    mut writer: W,
    value: &T,
) -> Result<(), CodecError> {
    let bytes = to_bytes(value)?;
    writer.write_all(&bytes).map_err(|error| CodecError::Write {
        kind: error.kind(),
        message: error.to_string(),
    })
}

/// Decodes a value from bytes made by [`to_bytes`].
///
/// The bytes must hold exactly one value of the type asked for: bytes
/// left over are [`CodecError::TrailingBytes`]. Corrupt or truncated
/// input is an error, never a panic.
pub fn from_bytes<'a, T: Deserialize<'a>>(bytes: &'a [u8]) -> Result<T, CodecError> {
    let mut decoder = Decoder {
        input: bytes,
        depth: 0,
    };
    let value = T::deserialize(&mut decoder)?;
    if decoder.input.is_empty() {
        Ok(value)
    } else {
        Err(CodecError::TrailingBytes(decoder.input.len()))
    }
}

/// What the fixed part of a blob says, before its body is read.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BlobHeader {
    /// The kind of data the blob holds.
    pub kind: [u8; 4],
    /// The version of the body's layout.
    pub version: u32,
    /// The length of the body in bytes, before compression.
    pub body_len: usize,
    /// The checksum of the kind, the version and the body.
    pub checksum: Hash128,
}

/// Encodes a value and frames it as a blob of the given kind and version.
///
/// See the [module documentation](self) for the frame.
pub fn encode_blob<T: Serialize + ?Sized>(
    kind: [u8; 4],
    version: u32,
    body: &T,
) -> Result<Vec<u8>, CodecError> {
    seal_blob(kind, version, &to_bytes(body)?)
}

/// Checks a blob's frame and decodes its body, returning the version the
/// blob was written with.
///
/// Use this when one layout is all the caller reads. When the layout
/// depends on the version, use [`open_blob`] and [`from_bytes`].
pub fn decode_blob<T: DeserializeOwned>(
    kind: [u8; 4],
    blob: &[u8],
) -> Result<(u32, T), CodecError> {
    let (version, body) = open_blob(kind, blob)?;
    Ok((version, from_bytes(&body)?))
}

/// Frames bytes that are already encoded as a blob: the header, then the
/// bytes compressed.
pub fn seal_blob(kind: [u8; 4], version: u32, body: &[u8]) -> Result<Vec<u8>, CodecError> {
    let Ok(body_len) = u32::try_from(body.len()) else {
        return Err(CodecError::BodyTooLarge(body.len()));
    };
    let compressed = lz4_flex::block::compress(body);
    let mut blob = Vec::with_capacity(BLOB_HEADER_LEN + compressed.len());
    blob.extend_from_slice(&BLOB_MAGIC);
    blob.extend_from_slice(&kind);
    blob.extend_from_slice(&version.to_le_bytes());
    blob.extend_from_slice(&body_len.to_le_bytes());
    blob.extend_from_slice(&blob_checksum(kind, version, body).0.to_le_bytes());
    blob.extend_from_slice(&compressed);
    Ok(blob)
}

/// Checks a blob's frame and returns its version and its body's bytes,
/// uncompressed and verified against the checksum.
///
/// Each thing that can be wrong with a frame is its own error: too short
/// to hold a header ([`CodecError::UnexpectedEnd`]), not a blob
/// ([`CodecError::WrongMagic`]), a blob of something else
/// ([`CodecError::WrongKind`]), a body of the wrong length
/// ([`CodecError::WrongLength`]), a body that does not decompress
/// ([`CodecError::BadCompression`]) and contents that do not match the
/// checksum ([`CodecError::WrongChecksum`]).
pub fn open_blob(kind: [u8; 4], blob: &[u8]) -> Result<(u32, Vec<u8>), CodecError> {
    let header = read_blob_header(blob)?;
    if header.kind != kind {
        return Err(CodecError::WrongKind {
            expected: kind,
            found: header.kind,
        });
    }
    let wrong_length = CodecError::WrongLength {
        stated: header.body_len,
    };
    let compressed = blob.get(BLOB_HEADER_LEN..).unwrap_or(&[]);
    // No block can expand to more than this, so a header that claims more
    // is wrong, and nothing larger is ever reserved.
    let most = compressed.len().saturating_mul(LZ4_MAX_EXPANSION);
    if header.body_len > most {
        return Err(wrong_length);
    }
    let mut body = vec![0_u8; header.body_len];
    match lz4_flex::block::decompress_into(compressed, &mut body) {
        Ok(written) if written == header.body_len => {}
        Ok(_) | Err(lz4_flex::block::DecompressError::OutputTooSmall { .. }) => {
            return Err(wrong_length);
        }
        Err(_) => return Err(CodecError::BadCompression),
    }
    let found = blob_checksum(header.kind, header.version, &body);
    if found != header.checksum {
        return Err(CodecError::WrongChecksum {
            stated: header.checksum,
            found,
        });
    }
    Ok((header.version, body))
}

/// Reads the fixed part of a blob without touching its body: enough to
/// say what a file is and which version wrote it.
///
/// Nothing but the magic is checked. The body may still be damaged.
pub fn read_blob_header(blob: &[u8]) -> Result<BlobHeader, CodecError> {
    let mut decoder = Decoder {
        input: blob,
        depth: 0,
    };
    if blob.len() < BLOB_HEADER_LEN {
        return Err(CodecError::UnexpectedEnd {
            needed: BLOB_HEADER_LEN,
            remaining: blob.len(),
        });
    }
    let magic: [u8; 4] = decoder.take_array()?;
    if magic != BLOB_MAGIC {
        return Err(CodecError::WrongMagic { found: magic });
    }
    let kind: [u8; 4] = decoder.take_array()?;
    let version = u32::from_le_bytes(decoder.take_array()?);
    let stated = u32::from_le_bytes(decoder.take_array()?);
    let checksum = Hash128(u128::from_le_bytes(decoder.take_array()?));
    Ok(BlobHeader {
        kind,
        version,
        // A `u32` always fits: the engine's targets are 64-bit (01 §4).
        body_len: usize::try_from(stated).unwrap_or(usize::MAX),
        checksum,
    })
}

/// The checksum of a blob: the kind, the version and the body.
fn blob_checksum(kind: [u8; 4], version: u32, body: &[u8]) -> Hash128 {
    let mut hasher = StateHasher::new();
    hasher.write_bytes(&kind);
    hasher.write_u32(version);
    hasher.write_bytes(body);
    hasher.finish()
}

/// Maps a signed number onto the unsigned ones so that small magnitudes
/// stay small: `0, -1, 1, -2, …` become `0, 1, 2, 3, …`.
const fn zigzag(value: i128) -> u128 {
    ((value << 1) ^ (value >> (i128::BITS - 1))) as u128
}

/// The inverse of [`zigzag`].
const fn unzigzag(value: u128) -> i128 {
    ((value >> 1) as i128) ^ -((value & 1) as i128)
}

// ---- Encoding -----------------------------------------------------------

/// Writes values into a buffer.
struct Encoder {
    out: Vec<u8>,
    /// How many compound values enclose what is being written.
    depth: usize,
}

impl Encoder {
    /// Writes an unsigned number as LEB128.
    fn put_varint(&mut self, mut value: u128) {
        loop {
            let low = (value as u8) & VARINT_MASK;
            value >>= VARINT_BITS;
            if value == 0 {
                self.out.push(low);
                return;
            }
            self.out.push(low | VARINT_MORE);
        }
    }

    /// Writes a length or a count.
    fn put_len(&mut self, length: usize) {
        self.put_varint(length as u128);
    }

    /// Goes one level deeper, or refuses.
    fn enter(&mut self) -> Result<(), CodecError> {
        if self.depth >= MAX_DEPTH {
            return Err(CodecError::TooDeep);
        }
        self.depth += 1;
        Ok(())
    }

    /// Comes back up one level.
    fn leave(&mut self) {
        self.depth = self.depth.saturating_sub(1);
    }

    /// Starts a compound value with no count of its own.
    fn fields(&mut self) -> Result<Compound<'_>, CodecError> {
        self.enter()?;
        Ok(Compound {
            encoder: self,
            counted: None,
        })
    }

    /// Starts a sequence or map: writes the count and remembers it.
    fn items(&mut self, length: Option<usize>) -> Result<Compound<'_>, CodecError> {
        let stated = length.ok_or(CodecError::UnknownLength)?;
        self.enter()?;
        self.put_len(stated);
        let item_start = self.out.len();
        Ok(Compound {
            encoder: self,
            counted: Some(Counted {
                stated,
                written: 0,
                item_start,
            }),
        })
    }
}

/// The bookkeeping of a sequence or map being written.
struct Counted {
    /// The number of items promised.
    stated: usize,
    /// The number of items written so far.
    written: usize,
    /// Where in the buffer the item being written starts.
    item_start: usize,
}

/// A compound value being written: its fields or items go straight into
/// the encoder's buffer.
struct Compound<'a> {
    encoder: &'a mut Encoder,
    /// Set for a sequence or map, which has a count to keep.
    counted: Option<Counted>,
}

impl Compound<'_> {
    fn put<T: Serialize + ?Sized>(&mut self, value: &T) -> Result<(), CodecError> {
        value.serialize(&mut *self.encoder)
    }

    /// Begins an item of a sequence, or an entry of a map.
    fn begin_item(&mut self) {
        if let Some(counted) = &mut self.counted {
            counted.written += 1;
            counted.item_start = self.encoder.out.len();
        }
    }

    /// Ends an item or an entry: it must have written something, because
    /// the decoder bounds the number of items by the bytes that remain.
    fn end_item(&self) -> Result<(), CodecError> {
        match &self.counted {
            Some(counted) if self.encoder.out.len() == counted.item_start => {
                Err(CodecError::EmptyItem)
            }
            _ => Ok(()),
        }
    }

    fn finish(self) -> Result<(), CodecError> {
        self.encoder.leave();
        match self.counted {
            Some(counted) if counted.written != counted.stated => Err(CodecError::LengthMismatch {
                stated: counted.stated,
                written: counted.written,
            }),
            _ => Ok(()),
        }
    }
}

impl<'a> Serializer for &'a mut Encoder {
    type Ok = ();
    type Error = CodecError;
    type SerializeSeq = Compound<'a>;
    type SerializeTuple = Compound<'a>;
    type SerializeTupleStruct = Compound<'a>;
    type SerializeTupleVariant = Compound<'a>;
    type SerializeMap = Compound<'a>;
    type SerializeStruct = Compound<'a>;
    type SerializeStructVariant = Compound<'a>;

    fn is_human_readable(&self) -> bool {
        false
    }

    fn serialize_bool(self, value: bool) -> Result<(), CodecError> {
        self.out.push(u8::from(value));
        Ok(())
    }

    fn serialize_i8(self, value: i8) -> Result<(), CodecError> {
        self.out.extend_from_slice(&value.to_le_bytes());
        Ok(())
    }

    fn serialize_i16(self, value: i16) -> Result<(), CodecError> {
        self.serialize_i128(i128::from(value))
    }

    fn serialize_i32(self, value: i32) -> Result<(), CodecError> {
        self.serialize_i128(i128::from(value))
    }

    fn serialize_i64(self, value: i64) -> Result<(), CodecError> {
        self.serialize_i128(i128::from(value))
    }

    fn serialize_i128(self, value: i128) -> Result<(), CodecError> {
        self.put_varint(zigzag(value));
        Ok(())
    }

    fn serialize_u8(self, value: u8) -> Result<(), CodecError> {
        self.out.push(value);
        Ok(())
    }

    fn serialize_u16(self, value: u16) -> Result<(), CodecError> {
        self.serialize_u128(u128::from(value))
    }

    fn serialize_u32(self, value: u32) -> Result<(), CodecError> {
        self.serialize_u128(u128::from(value))
    }

    fn serialize_u64(self, value: u64) -> Result<(), CodecError> {
        self.serialize_u128(u128::from(value))
    }

    fn serialize_u128(self, value: u128) -> Result<(), CodecError> {
        self.put_varint(value);
        Ok(())
    }

    fn serialize_f32(self, value: f32) -> Result<(), CodecError> {
        self.out.extend_from_slice(&value.to_le_bytes());
        Ok(())
    }

    fn serialize_f64(self, value: f64) -> Result<(), CodecError> {
        self.out.extend_from_slice(&value.to_le_bytes());
        Ok(())
    }

    fn serialize_char(self, value: char) -> Result<(), CodecError> {
        self.serialize_u32(u32::from(value))
    }

    fn serialize_str(self, value: &str) -> Result<(), CodecError> {
        self.serialize_bytes(value.as_bytes())
    }

    fn serialize_bytes(self, value: &[u8]) -> Result<(), CodecError> {
        self.put_len(value.len());
        self.out.extend_from_slice(value);
        Ok(())
    }

    fn serialize_none(self) -> Result<(), CodecError> {
        self.out.push(TAG_NONE);
        Ok(())
    }

    fn serialize_some<T: Serialize + ?Sized>(self, value: &T) -> Result<(), CodecError> {
        self.enter()?;
        self.out.push(TAG_SOME);
        value.serialize(&mut *self)?;
        self.leave();
        Ok(())
    }

    fn serialize_unit(self) -> Result<(), CodecError> {
        Ok(())
    }

    fn serialize_unit_struct(self, _name: &'static str) -> Result<(), CodecError> {
        Ok(())
    }

    fn serialize_unit_variant(
        self,
        _name: &'static str,
        index: u32,
        _variant: &'static str,
    ) -> Result<(), CodecError> {
        self.serialize_u32(index)
    }

    fn serialize_newtype_struct<T: Serialize + ?Sized>(
        self,
        _name: &'static str,
        value: &T,
    ) -> Result<(), CodecError> {
        self.enter()?;
        value.serialize(&mut *self)?;
        self.leave();
        Ok(())
    }

    fn serialize_newtype_variant<T: Serialize + ?Sized>(
        self,
        _name: &'static str,
        index: u32,
        _variant: &'static str,
        value: &T,
    ) -> Result<(), CodecError> {
        self.enter()?;
        self.put_varint(u128::from(index));
        value.serialize(&mut *self)?;
        self.leave();
        Ok(())
    }

    fn serialize_seq(self, length: Option<usize>) -> Result<Compound<'a>, CodecError> {
        self.items(length)
    }

    fn serialize_tuple(self, _length: usize) -> Result<Compound<'a>, CodecError> {
        self.fields()
    }

    fn serialize_tuple_struct(
        self,
        _name: &'static str,
        _length: usize,
    ) -> Result<Compound<'a>, CodecError> {
        self.fields()
    }

    fn serialize_tuple_variant(
        self,
        _name: &'static str,
        index: u32,
        _variant: &'static str,
        _length: usize,
    ) -> Result<Compound<'a>, CodecError> {
        self.put_varint(u128::from(index));
        self.fields()
    }

    fn serialize_map(self, length: Option<usize>) -> Result<Compound<'a>, CodecError> {
        self.items(length)
    }

    fn serialize_struct(
        self,
        _name: &'static str,
        _length: usize,
    ) -> Result<Compound<'a>, CodecError> {
        self.fields()
    }

    fn serialize_struct_variant(
        self,
        _name: &'static str,
        index: u32,
        _variant: &'static str,
        _length: usize,
    ) -> Result<Compound<'a>, CodecError> {
        self.put_varint(u128::from(index));
        self.fields()
    }
}

impl ser::SerializeSeq for Compound<'_> {
    type Ok = ();
    type Error = CodecError;

    fn serialize_element<T: Serialize + ?Sized>(&mut self, value: &T) -> Result<(), CodecError> {
        self.begin_item();
        self.put(value)?;
        self.end_item()
    }

    fn end(self) -> Result<(), CodecError> {
        self.finish()
    }
}

impl ser::SerializeTuple for Compound<'_> {
    type Ok = ();
    type Error = CodecError;

    fn serialize_element<T: Serialize + ?Sized>(&mut self, value: &T) -> Result<(), CodecError> {
        self.put(value)
    }

    fn end(self) -> Result<(), CodecError> {
        self.finish()
    }
}

impl ser::SerializeTupleStruct for Compound<'_> {
    type Ok = ();
    type Error = CodecError;

    fn serialize_field<T: Serialize + ?Sized>(&mut self, value: &T) -> Result<(), CodecError> {
        self.put(value)
    }

    fn end(self) -> Result<(), CodecError> {
        self.finish()
    }
}

impl ser::SerializeTupleVariant for Compound<'_> {
    type Ok = ();
    type Error = CodecError;

    fn serialize_field<T: Serialize + ?Sized>(&mut self, value: &T) -> Result<(), CodecError> {
        self.put(value)
    }

    fn end(self) -> Result<(), CodecError> {
        self.finish()
    }
}

impl ser::SerializeMap for Compound<'_> {
    type Ok = ();
    type Error = CodecError;

    fn serialize_key<T: Serialize + ?Sized>(&mut self, key: &T) -> Result<(), CodecError> {
        self.begin_item();
        self.put(key)
    }

    fn serialize_value<T: Serialize + ?Sized>(&mut self, value: &T) -> Result<(), CodecError> {
        self.put(value)?;
        self.end_item()
    }

    fn end(self) -> Result<(), CodecError> {
        self.finish()
    }
}

impl ser::SerializeStruct for Compound<'_> {
    type Ok = ();
    type Error = CodecError;

    fn serialize_field<T: Serialize + ?Sized>(
        &mut self,
        _key: &'static str,
        value: &T,
    ) -> Result<(), CodecError> {
        self.put(value)
    }

    fn skip_field(&mut self, key: &'static str) -> Result<(), CodecError> {
        Err(CodecError::SkippedField(key))
    }

    fn end(self) -> Result<(), CodecError> {
        self.finish()
    }
}

impl ser::SerializeStructVariant for Compound<'_> {
    type Ok = ();
    type Error = CodecError;

    fn serialize_field<T: Serialize + ?Sized>(
        &mut self,
        _key: &'static str,
        value: &T,
    ) -> Result<(), CodecError> {
        self.put(value)
    }

    fn skip_field(&mut self, key: &'static str) -> Result<(), CodecError> {
        Err(CodecError::SkippedField(key))
    }

    fn end(self) -> Result<(), CodecError> {
        self.finish()
    }
}

// ---- Decoding -----------------------------------------------------------

/// Reads values from the front of a slice.
struct Decoder<'de> {
    /// What has not been read yet.
    input: &'de [u8],
    /// How many compound values enclose what is being read.
    depth: usize,
}

impl<'de> Decoder<'de> {
    /// Takes the next `count` bytes.
    fn take(&mut self, count: usize) -> Result<&'de [u8], CodecError> {
        match self.input.split_at_checked(count) {
            Some((head, rest)) => {
                self.input = rest;
                Ok(head)
            }
            None => Err(CodecError::UnexpectedEnd {
                needed: count,
                remaining: self.input.len(),
            }),
        }
    }

    /// Takes the next `N` bytes as an array.
    fn take_array<const N: usize>(&mut self) -> Result<[u8; N], CodecError> {
        let mut array = [0_u8; N];
        array.copy_from_slice(self.take(N)?);
        Ok(array)
    }

    /// Takes the next byte.
    fn take_byte(&mut self) -> Result<u8, CodecError> {
        let [byte] = self.take_array()?;
        Ok(byte)
    }

    /// Takes a LEB128 number of at most `bits` bits, in its shortest form.
    fn take_varint(&mut self, bits: u32) -> Result<u128, CodecError> {
        let mut value = 0_u128;
        let mut shift = 0_u32;
        loop {
            let byte = self.take_byte()?;
            let part = u128::from(byte & VARINT_MASK);
            if shift >= bits || (part << shift) >> shift != part {
                return Err(CodecError::BadVarint);
            }
            value |= part << shift;
            if byte & VARINT_MORE == 0 {
                // A last byte of zero adds nothing: a shorter form exists.
                if byte == 0 && shift != 0 {
                    return Err(CodecError::BadVarint);
                }
                break;
            }
            shift += VARINT_BITS;
        }
        if bits < u128::BITS && value >> bits != 0 {
            return Err(CodecError::BadVarint);
        }
        Ok(value)
    }

    /// Takes a zig-zag LEB128 number of at most `bits` bits.
    fn take_signed(&mut self, bits: u32) -> Result<i128, CodecError> {
        Ok(unzigzag(self.take_varint(bits)?))
    }

    /// Takes a length or a count, refusing one that the bytes left could
    /// not hold. This is what bounds every allocation and every loop.
    fn take_len(&mut self) -> Result<usize, CodecError> {
        let length = self.take_varint(u64::BITS)? as u64;
        match usize::try_from(length) {
            Ok(length) if length <= self.input.len() => Ok(length),
            _ => Err(CodecError::LengthOutOfBounds {
                length,
                remaining: self.input.len(),
            }),
        }
    }

    /// Takes a length and that many bytes.
    fn take_bytes(&mut self) -> Result<&'de [u8], CodecError> {
        let length = self.take_len()?;
        self.take(length)
    }

    /// Takes a text.
    fn take_str(&mut self) -> Result<&'de str, CodecError> {
        core::str::from_utf8(self.take_bytes()?).map_err(|_| CodecError::InvalidUtf8)
    }

    /// Goes one level deeper, or refuses.
    fn enter(&mut self) -> Result<(), CodecError> {
        if self.depth >= MAX_DEPTH {
            return Err(CodecError::TooDeep);
        }
        self.depth += 1;
        Ok(())
    }

    /// Comes back up one level.
    fn leave(&mut self) {
        self.depth = self.depth.saturating_sub(1);
    }

    /// Reads `count` values in a row for a visitor: the fields of a
    /// struct or tuple (`counted` false), or the items of a sequence
    /// whose count came from the input (`counted` true).
    fn visit_items<V: Visitor<'de>>(
        &mut self,
        count: usize,
        counted: bool,
        visitor: V,
    ) -> Result<V::Value, CodecError> {
        self.enter()?;
        let mut items = Items {
            item_start: self.input.len(),
            decoder: self,
            left: count,
            counted,
        };
        let value = visitor.visit_seq(&mut items)?;
        let unread = items.left;
        self.leave();
        if unread == 0 {
            Ok(value)
        } else {
            Err(de::Error::custom(format_args!(
                "{unread} of {count} items were not read"
            )))
        }
    }
}

macro_rules! decode_number {
    ($($method:ident => $visit:ident($take:ident, $int:ty);)*) => {
        $(
            fn $method<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, CodecError> {
                let wide = self.$take(<$int>::BITS)?;
                match <$int>::try_from(wide) {
                    Ok(value) => visitor.$visit(value),
                    Err(_) => Err(CodecError::BadVarint),
                }
            }
        )*
    };
}

impl<'de> Deserializer<'de> for &mut Decoder<'de> {
    type Error = CodecError;

    fn is_human_readable(&self) -> bool {
        false
    }

    fn deserialize_any<V: Visitor<'de>>(self, _visitor: V) -> Result<V::Value, CodecError> {
        Err(CodecError::NotSelfDescribing("deserialize_any"))
    }

    fn deserialize_ignored_any<V: Visitor<'de>>(self, _visitor: V) -> Result<V::Value, CodecError> {
        Err(CodecError::NotSelfDescribing("deserialize_ignored_any"))
    }

    fn deserialize_identifier<V: Visitor<'de>>(self, _visitor: V) -> Result<V::Value, CodecError> {
        Err(CodecError::NotSelfDescribing("deserialize_identifier"))
    }

    fn deserialize_bool<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, CodecError> {
        match self.take_byte()? {
            0 => visitor.visit_bool(false),
            1 => visitor.visit_bool(true),
            other => Err(CodecError::InvalidBool(other)),
        }
    }

    fn deserialize_i8<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, CodecError> {
        visitor.visit_i8(i8::from_le_bytes(self.take_array()?))
    }

    fn deserialize_u8<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, CodecError> {
        visitor.visit_u8(self.take_byte()?)
    }

    decode_number! {
        deserialize_i16 => visit_i16(take_signed, i16);
        deserialize_i32 => visit_i32(take_signed, i32);
        deserialize_i64 => visit_i64(take_signed, i64);
        deserialize_i128 => visit_i128(take_signed, i128);
        deserialize_u16 => visit_u16(take_varint, u16);
        deserialize_u32 => visit_u32(take_varint, u32);
        deserialize_u64 => visit_u64(take_varint, u64);
        deserialize_u128 => visit_u128(take_varint, u128);
    }

    fn deserialize_f32<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, CodecError> {
        visitor.visit_f32(f32::from_le_bytes(self.take_array()?))
    }

    fn deserialize_f64<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, CodecError> {
        visitor.visit_f64(f64::from_le_bytes(self.take_array()?))
    }

    fn deserialize_char<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, CodecError> {
        let number = self.take_varint(u32::BITS)? as u32;
        match char::from_u32(number) {
            Some(character) => visitor.visit_char(character),
            None => Err(CodecError::InvalidChar(number)),
        }
    }

    fn deserialize_str<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, CodecError> {
        visitor.visit_borrowed_str(self.take_str()?)
    }

    fn deserialize_string<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, CodecError> {
        self.deserialize_str(visitor)
    }

    fn deserialize_bytes<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, CodecError> {
        visitor.visit_borrowed_bytes(self.take_bytes()?)
    }

    fn deserialize_byte_buf<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, CodecError> {
        self.deserialize_bytes(visitor)
    }

    fn deserialize_option<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, CodecError> {
        match self.take_byte()? {
            TAG_NONE => visitor.visit_none(),
            TAG_SOME => {
                self.enter()?;
                let value = visitor.visit_some(&mut *self)?;
                self.leave();
                Ok(value)
            }
            other => Err(CodecError::InvalidOptionTag(other)),
        }
    }

    fn deserialize_unit<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, CodecError> {
        visitor.visit_unit()
    }

    fn deserialize_unit_struct<V: Visitor<'de>>(
        self,
        _name: &'static str,
        visitor: V,
    ) -> Result<V::Value, CodecError> {
        visitor.visit_unit()
    }

    fn deserialize_newtype_struct<V: Visitor<'de>>(
        self,
        _name: &'static str,
        visitor: V,
    ) -> Result<V::Value, CodecError> {
        self.enter()?;
        let value = visitor.visit_newtype_struct(&mut *self)?;
        self.leave();
        Ok(value)
    }

    fn deserialize_seq<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, CodecError> {
        let count = self.take_len()?;
        self.visit_items(count, true, visitor)
    }

    fn deserialize_tuple<V: Visitor<'de>>(
        self,
        length: usize,
        visitor: V,
    ) -> Result<V::Value, CodecError> {
        self.visit_items(length, false, visitor)
    }

    fn deserialize_tuple_struct<V: Visitor<'de>>(
        self,
        _name: &'static str,
        length: usize,
        visitor: V,
    ) -> Result<V::Value, CodecError> {
        self.visit_items(length, false, visitor)
    }

    fn deserialize_map<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, CodecError> {
        let count = self.take_len()?;
        self.enter()?;
        let mut entries = Items {
            item_start: self.input.len(),
            decoder: self,
            left: count,
            counted: true,
        };
        let value = visitor.visit_map(&mut entries)?;
        let unread = entries.left;
        self.leave();
        if unread == 0 {
            Ok(value)
        } else {
            Err(de::Error::custom(format_args!(
                "{unread} of {count} entries were not read"
            )))
        }
    }

    fn deserialize_struct<V: Visitor<'de>>(
        self,
        _name: &'static str,
        fields: &'static [&'static str],
        visitor: V,
    ) -> Result<V::Value, CodecError> {
        self.visit_items(fields.len(), false, visitor)
    }

    fn deserialize_enum<V: Visitor<'de>>(
        self,
        _name: &'static str,
        _variants: &'static [&'static str],
        visitor: V,
    ) -> Result<V::Value, CodecError> {
        visitor.visit_enum(Variant { decoder: self })
    }
}

/// The items of a sequence, the fields of a struct or the entries of a
/// map, as they are read.
struct Items<'a, 'de> {
    decoder: &'a mut Decoder<'de>,
    /// How many are still to be read.
    left: usize,
    /// Whether the count came from the input, as a sequence's and a
    /// map's does. Each such item must take at least one byte, which is
    /// what keeps the work of decoding within the length of the input.
    counted: bool,
    /// How many bytes were unread when the current item began.
    item_start: usize,
}

impl Items<'_, '_> {
    /// Begins an item of a sequence, or an entry of a map.
    fn begin_item(&mut self) {
        self.left -= 1;
        self.item_start = self.decoder.input.len();
    }

    /// Ends an item or an entry: it must have read something.
    fn end_item(&self) -> Result<(), CodecError> {
        if self.counted && self.decoder.input.len() == self.item_start {
            Err(CodecError::EmptyItem)
        } else {
            Ok(())
        }
    }
}

impl<'de> SeqAccess<'de> for Items<'_, 'de> {
    type Error = CodecError;

    fn next_element_seed<T: DeserializeSeed<'de>>(
        &mut self,
        seed: T,
    ) -> Result<Option<T::Value>, CodecError> {
        if self.left == 0 {
            return Ok(None);
        }
        self.begin_item();
        let value = seed.deserialize(&mut *self.decoder)?;
        self.end_item()?;
        Ok(Some(value))
    }

    fn size_hint(&self) -> Option<usize> {
        Some(self.left)
    }
}

impl<'de> MapAccess<'de> for Items<'_, 'de> {
    type Error = CodecError;

    fn next_key_seed<K: DeserializeSeed<'de>>(
        &mut self,
        seed: K,
    ) -> Result<Option<K::Value>, CodecError> {
        if self.left == 0 {
            return Ok(None);
        }
        self.begin_item();
        seed.deserialize(&mut *self.decoder).map(Some)
    }

    fn next_value_seed<V: DeserializeSeed<'de>>(
        &mut self,
        seed: V,
    ) -> Result<V::Value, CodecError> {
        let value = seed.deserialize(&mut *self.decoder)?;
        self.end_item()?;
        Ok(value)
    }

    fn size_hint(&self) -> Option<usize> {
        Some(self.left)
    }
}

/// An enum being read: the variant's index, then its fields.
struct Variant<'a, 'de> {
    decoder: &'a mut Decoder<'de>,
}

impl<'de> EnumAccess<'de> for Variant<'_, 'de> {
    type Error = CodecError;
    type Variant = Self;

    fn variant_seed<V: DeserializeSeed<'de>>(
        self,
        seed: V,
    ) -> Result<(V::Value, Self), CodecError> {
        let index = self.decoder.take_varint(u32::BITS)? as u32;
        let which = seed.deserialize(de::value::U32Deserializer::<CodecError>::new(index))?;
        Ok((which, self))
    }
}

impl<'de> VariantAccess<'de> for Variant<'_, 'de> {
    type Error = CodecError;

    fn unit_variant(self) -> Result<(), CodecError> {
        Ok(())
    }

    fn newtype_variant_seed<T: DeserializeSeed<'de>>(
        self,
        seed: T,
    ) -> Result<T::Value, CodecError> {
        self.decoder.enter()?;
        let value = seed.deserialize(&mut *self.decoder)?;
        self.decoder.leave();
        Ok(value)
    }

    fn tuple_variant<V: Visitor<'de>>(
        self,
        length: usize,
        visitor: V,
    ) -> Result<V::Value, CodecError> {
        self.decoder.visit_items(length, false, visitor)
    }

    fn struct_variant<V: Visitor<'de>>(
        self,
        fields: &'static [&'static str],
        visitor: V,
    ) -> Result<V::Value, CodecError> {
        self.decoder.visit_items(fields.len(), false, visitor)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn decoder(bytes: &[u8]) -> Decoder<'_> {
        Decoder {
            input: bytes,
            depth: 0,
        }
    }

    fn varint(value: u128) -> Vec<u8> {
        let mut encoder = Encoder {
            out: Vec::new(),
            depth: 0,
        };
        encoder.put_varint(value);
        encoder.out
    }

    #[test]
    fn varints_are_leb128() {
        assert_eq!(varint(0), [0x00]);
        assert_eq!(varint(1), [0x01]);
        assert_eq!(varint(127), [0x7f]);
        assert_eq!(varint(128), [0x80, 0x01]);
        assert_eq!(varint(300), [0xac, 0x02]);
        assert_eq!(varint(624_485), [0xe5, 0x8e, 0x26]);
        assert_eq!(varint(u128::from(u64::MAX)).len(), 10);
        assert_eq!(varint(u128::MAX).len(), 19);
    }

    #[test]
    fn varints_round_trip_at_every_width() {
        for bits in [16, 32, 64, 128] {
            let top = if bits == 128 {
                u128::MAX
            } else {
                (1_u128 << bits) - 1
            };
            for value in [0, 1, 127, 128, 255, 256, top / 3, top - 1, top] {
                let bytes = varint(value);
                let mut reader = decoder(&bytes);
                assert_eq!(
                    reader.take_varint(bits),
                    Ok(value),
                    "{value} in {bits} bits"
                );
                assert!(reader.input.is_empty());
            }
        }
    }

    #[test]
    fn varints_that_do_not_fit_are_refused() {
        // 2^16 does not fit sixteen bits.
        assert_eq!(
            decoder(&varint(1 << 16)).take_varint(16),
            Err(CodecError::BadVarint)
        );
        // Four bytes are more than sixteen bits can need.
        assert_eq!(
            decoder(&[0x80, 0x80, 0x80, 0x01]).take_varint(16),
            Err(CodecError::BadVarint)
        );
        // Twenty bytes are more than any number needs.
        let mut endless = vec![0xff; 19];
        endless.push(0x01);
        assert_eq!(
            decoder(&endless).take_varint(128),
            Err(CodecError::BadVarint)
        );
        // The nineteenth byte of a 128-bit number holds two bits.
        let mut wide = vec![0xff; 18];
        wide.push(0x04);
        assert_eq!(decoder(&wide).take_varint(128), Err(CodecError::BadVarint));
        // And one that never ends, ends with the input.
        assert_eq!(
            decoder(&[0x80, 0x80]).take_varint(64),
            Err(CodecError::UnexpectedEnd {
                needed: 1,
                remaining: 0
            })
        );
    }

    #[test]
    fn varints_longer_than_they_need_to_be_are_refused() {
        assert_eq!(
            decoder(&[0x80, 0x00]).take_varint(64),
            Err(CodecError::BadVarint)
        );
        assert_eq!(
            decoder(&[0xff, 0x80, 0x00]).take_varint(64),
            Err(CodecError::BadVarint)
        );
        // A lone zero is the shortest form of zero.
        assert_eq!(decoder(&[0x00]).take_varint(64), Ok(0));
    }

    #[test]
    fn zigzag_keeps_small_magnitudes_small() {
        let pairs: [(i128, u128); 7] = [
            (0, 0),
            (-1, 1),
            (1, 2),
            (-2, 3),
            (2, 4),
            (i128::from(i64::MAX), u128::from(u64::MAX) - 1),
            (i128::from(i64::MIN), u128::from(u64::MAX)),
        ];
        for (signed, unsigned) in pairs {
            assert_eq!(zigzag(signed), unsigned);
            assert_eq!(unzigzag(unsigned), signed);
        }
        assert_eq!(unzigzag(zigzag(i128::MIN)), i128::MIN);
        assert_eq!(unzigzag(zigzag(i128::MAX)), i128::MAX);
        assert_eq!(zigzag(i128::MIN), u128::MAX);
    }

    #[test]
    fn lengths_are_checked_against_what_remains() {
        // A length of five with four bytes after it.
        assert_eq!(
            decoder(&[5, 1, 2, 3, 4]).take_len(),
            Err(CodecError::LengthOutOfBounds {
                length: 5,
                remaining: 4
            })
        );
        assert_eq!(decoder(&[4, 1, 2, 3, 4]).take_len(), Ok(4));
        // A length no machine could hold.
        let huge = varint(u128::from(u64::MAX));
        assert_eq!(
            decoder(&huge).take_len(),
            Err(CodecError::LengthOutOfBounds {
                length: u64::MAX,
                remaining: 0
            })
        );
    }

    #[test]
    fn tags_are_shown_readably() {
        assert_eq!(tag_text(b"TRCT"), "TRCT");
        assert_eq!(tag_text(&[0, b'a', 0xff, b'"']), "\\x00a\\xff\\\"");
        let error = CodecError::WrongKind {
            expected: *b"TRCT",
            found: *b"SAVE",
        };
        assert_eq!(error.to_string(), "blob is of kind `SAVE`, not `TRCT`");
    }
}
