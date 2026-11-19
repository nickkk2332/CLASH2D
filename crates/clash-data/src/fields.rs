//! Field types that need more than plain `serde` (04 §7.1).
//!
//! A definition type is a struct that derives `serde::Deserialize`. Its
//! fields are ordinary types (`bool`, integers, [`Real`], `String`,
//! `Vec`, `Option`, `BTreeMap`, enums) and, where JSON says more than a
//! literal, the types here:
//!
//! | Type | In JSON |
//! |------|---------|
//! | [`Length`], [`Mass`], [`Energy`], [`Pressure`], [`Speed`], [`Temperature`], [`Power`], [`Force`], [`Volume`] | A number in SI units, or a string with a unit: `"70 kPa"`, `"-5 C"` |
//! | [`Time`] | The same for a duration, kept as a [`Dur`]: `"250 ms"` |
//! | [`Rate<Q>`], [`Frequency`] | A quantity per time: `"0.012 kg/s"`, `"0.15 /h"` |
//! | [`Ref<T>`] | The id of a definition of type `T`, resolved to its dense id |
//! | [`Inline<T>`] | An id, or a definition of type `T` written in place |
//! | [`Tag`], [`TagSet`] | Free-form strings, interned |
//! | [`Text`] | Player-facing text, with `{expression}` templates |
//! | [`Expr<T>`](crate::Expr), [`Cond`](crate::Cond) | An expression, a condition (see [`crate::expr`]) |
//! | [`Located<T>`] | Any value, together with where it was written |
//! | a type read with [`by_site`] | A value that needs its place and its JSON path |
//! | [`Gated<F, T>`] | A field that is an experimental feature (04 §8.4) |
//! | a type read with [`by_kind`] | More than one shape: an id or an object, a number or a string |
//!
//! A unit of the wrong dimension for a field is an error that names both
//! dimensions.

use std::cmp::Ordering;
use std::fmt;
use std::hash::{Hash, Hasher};
use std::marker::PhantomData;

use clash_core::{DenseId, Dur, Real, StateHash, StateHasher};
use serde::de::{self, Deserialize, Deserializer, IgnoredAny, MapAccess, Visitor};
use serde::ser::{Serialize, Serializer};

use crate::de::{
    GATED_MAGIC, INLINE_MAGIC, KIND_FIELDS, KIND_MAGIC, LOCATED_FIELDS, LOCATED_MAGIC,
    MICROS_MAGIC, REF_MAGIC, SITE_FIELDS, SITE_MAGIC, SITE_OR_PLAIN_MAGIC, TAG_MAGIC,
};
use crate::defs::Def;
use crate::source::{FileId, Span};
use crate::units::Dim;

/// A physical quantity as a field type: a [`Real`] in SI units that knows
/// what it measures.
pub trait Quantity: Copy {
    /// What it measures.
    const DIM: Dim;
    /// The name it asks the loader for itself under.
    #[doc(hidden)]
    const MAGIC: &'static str;

    /// The quantity with this magnitude in SI units.
    fn from_real(value: Real) -> Self;

    /// The magnitude in SI units.
    fn real(self) -> Real;
}

/// A [`Quantity`] that can be had per time, as a [`Rate`].
pub trait HasRate: Quantity {
    /// The name its rate asks the loader for itself under.
    #[doc(hidden)]
    const RATE_MAGIC: &'static str;
}

struct RealVisitor;

impl<'de> Visitor<'de> for RealVisitor {
    type Value = Real;

    fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("a number in SI units or a string with a unit")
    }

    fn visit_f64<E: de::Error>(self, value: f64) -> Result<Real, E> {
        Real::new(value).map_err(E::custom)
    }

    fn visit_i64<E: de::Error>(self, value: i64) -> Result<Real, E> {
        Ok(Real::from_i64(value))
    }

    fn visit_u64<E: de::Error>(self, value: u64) -> Result<Real, E> {
        Ok(Real::of(value as f64))
    }

    fn visit_newtype_struct<D: Deserializer<'de>>(self, deserializer: D) -> Result<Real, D::Error> {
        // Any other format: the bare number it was written as.
        deserializer.deserialize_f64(RealVisitor)
    }
}

macro_rules! quantities {
    ($( $(#[$doc:meta])* $name:ident, $dim:expr, $magic:literal $(, rate $rate_magic:literal)?; )*) => {
        $(
            $(#[$doc])*
            ///
            /// In data: a number in SI units, or a string with a unit of
            /// this dimension (04 §7.1).
            #[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default, Debug)]
            pub struct $name(pub Real);

            impl $name {
                /// The quantity with this magnitude in SI units.
                pub const fn new(value: Real) -> $name {
                    $name(value)
                }

                /// The magnitude in SI units.
                pub const fn get(self) -> Real {
                    self.0
                }
            }

            impl Quantity for $name {
                const DIM: Dim = $dim;
                const MAGIC: &'static str = concat!("$clash_data::quantity::", $magic);

                fn from_real(value: Real) -> $name {
                    $name(value)
                }

                fn real(self) -> Real {
                    self.0
                }
            }

            $(
                impl HasRate for $name {
                    const RATE_MAGIC: &'static str = concat!("$clash_data::quantity::", $rate_magic);
                }
            )?

            impl From<$name> for Real {
                fn from(value: $name) -> Real {
                    value.0
                }
            }

            impl fmt::Display for $name {
                fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                    write!(f, "{} {}", self.0, <$name as Quantity>::DIM.si_unit())
                }
            }

            impl StateHash for $name {
                fn state_hash(&self, hasher: &mut StateHasher) {
                    self.0.state_hash(hasher);
                }
            }

            impl Serialize for $name {
                fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
                    self.0.serialize(serializer)
                }
            }

            impl<'de> Deserialize<'de> for $name {
                fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<$name, D::Error> {
                    deserializer
                        .deserialize_newtype_struct(<$name as Quantity>::MAGIC, RealVisitor)
                        .map($name)
                }
            }
        )*
    };
}

quantities! {
    /// A length, in metres.
    Length, Dim::LENGTH, "length", rate "speed";
    /// A volume, in cubic metres.
    Volume, Dim::VOLUME, "volume", rate "volume_rate";
    /// A mass, in kilograms.
    Mass, Dim::MASS, "mass", rate "mass_rate";
    /// An energy, in joules.
    Energy, Dim::ENERGY, "energy", rate "power";
    /// A force, in newtons.
    Force, Dim::FORCE, "force", rate "force_rate";
    /// A pressure, in pascals.
    Pressure, Dim::PRESSURE, "pressure", rate "pressure_rate";
    /// A temperature, in kelvin.
    Temperature, Dim::TEMPERATURE, "temperature", rate "temperature_rate";
    /// A speed, in metres per second.
    Speed, Dim::SPEED, "speed";
    /// A power, in watts.
    Power, Dim::POWER, "power";
    /// A plain rate, per second: how often, or what share per time.
    /// Written `"0.15 /h"`.
    Frequency, Dim::FREQUENCY, "frequency";
    /// A duration as a magnitude in seconds, for arithmetic with other
    /// quantities. A duration that schedules something is a [`Time`].
    Seconds, Dim::TIME, "time", rate "time_rate";
}

/// A quantity per time, in its SI unit per second: `Rate<Mass>` is
/// kilograms per second and is written `"0.012 kg/s"`.
///
/// Any unit of the right quantity followed by `/s`, `/min`, `/h` or `/d`
/// is accepted (04 §7.1). `Rate<Length>` is the same dimension as
/// [`Speed`], and `Rate<Energy>` as [`Power`].
pub struct Rate<Q: HasRate>(pub Real, PhantomData<fn() -> Q>);

impl<Q: HasRate> Rate<Q> {
    /// The rate with this magnitude in SI units per second.
    pub const fn new(value: Real) -> Rate<Q> {
        Rate(value, PhantomData)
    }

    /// The magnitude in SI units per second.
    pub const fn get(self) -> Real {
        self.0
    }

    /// What it measures.
    pub const DIM: Dim = Q::DIM.rate();
}

impl<Q: HasRate> Clone for Rate<Q> {
    fn clone(&self) -> Rate<Q> {
        *self
    }
}

impl<Q: HasRate> Copy for Rate<Q> {}

impl<Q: HasRate> PartialEq for Rate<Q> {
    fn eq(&self, other: &Rate<Q>) -> bool {
        self.0 == other.0
    }
}

impl<Q: HasRate> Eq for Rate<Q> {}

impl<Q: HasRate> PartialOrd for Rate<Q> {
    fn partial_cmp(&self, other: &Rate<Q>) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl<Q: HasRate> Ord for Rate<Q> {
    fn cmp(&self, other: &Rate<Q>) -> Ordering {
        self.0.cmp(&other.0)
    }
}

impl<Q: HasRate> Default for Rate<Q> {
    fn default() -> Rate<Q> {
        Rate::new(Real::ZERO)
    }
}

impl<Q: HasRate> fmt::Debug for Rate<Q> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Rate({:?})", self.0)
    }
}

impl<Q: HasRate> StateHash for Rate<Q> {
    fn state_hash(&self, hasher: &mut StateHasher) {
        self.0.state_hash(hasher);
    }
}

impl<Q: HasRate> Serialize for Rate<Q> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        self.0.serialize(serializer)
    }
}

impl<'de, Q: HasRate> Deserialize<'de> for Rate<Q> {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Rate<Q>, D::Error> {
        deserializer
            .deserialize_newtype_struct(Q::RATE_MAGIC, RealVisitor)
            .map(Rate::new)
    }
}

/// A duration as a field type: a [`Dur`] read from a number of seconds or
/// a string with a unit of time, `"250 ms"`, `"2 h"`.
///
/// `clash_core::Dur` itself reads as a bare count of microseconds, which
/// is right for saves and wrong for data. In a definition, write the
/// field as `Time`, or keep it a `Dur` and name this crate's [`dur`]
/// module: `#[serde(with = "clash_data::dur")]`.
///
/// Durations in data are local time (01 §7).
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default, Debug)]
pub struct Time(pub Dur);

impl Time {
    /// The duration.
    pub const fn dur(self) -> Dur {
        self.0
    }
}

impl From<Time> for Dur {
    fn from(value: Time) -> Dur {
        value.0
    }
}

impl From<Dur> for Time {
    fn from(value: Dur) -> Time {
        Time(value)
    }
}

impl fmt::Display for Time {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(&self.0, f)
    }
}

impl StateHash for Time {
    fn state_hash(&self, hasher: &mut StateHasher) {
        self.0.state_hash(hasher);
    }
}

impl Serialize for Time {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        self.0.serialize(serializer)
    }
}

struct MicrosVisitor;

impl<'de> Visitor<'de> for MicrosVisitor {
    type Value = Dur;

    fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("a number of seconds or a string with a unit of time")
    }

    fn visit_i64<E: de::Error>(self, value: i64) -> Result<Dur, E> {
        Ok(Dur::from_micros(value))
    }

    fn visit_u64<E: de::Error>(self, value: u64) -> Result<Dur, E> {
        i64::try_from(value)
            .map(Dur::from_micros)
            .map_err(|_| E::custom("this time is too long to represent"))
    }

    fn visit_newtype_struct<D: Deserializer<'de>>(self, deserializer: D) -> Result<Dur, D::Error> {
        // Any other format: the count of microseconds a `Dur` is saved as.
        Dur::deserialize(deserializer)
    }
}

impl<'de> Deserialize<'de> for Time {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Time, D::Error> {
        deserializer
            .deserialize_newtype_struct(MICROS_MAGIC, MicrosVisitor)
            .map(Time)
    }
}

/// For a field that stays a `clash_core::Dur`:
/// `#[serde(with = "clash_data::dur")]`. An optional one takes
/// `#[serde(default, with = "clash_data::dur::option")]`.
pub mod dur {
    use clash_core::Dur;
    use serde::{Deserialize, Deserializer, Serialize, Serializer};

    use super::Time;

    /// Reads a duration the way [`Time`] does.
    pub fn deserialize<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Dur, D::Error> {
        Time::deserialize(deserializer).map(|time| time.0)
    }

    /// Writes a duration the way `Dur` does.
    pub fn serialize<S: Serializer>(value: &Dur, serializer: S) -> Result<S::Ok, S::Error> {
        value.serialize(serializer)
    }

    /// The same for `Option<Dur>`.
    pub mod option {
        use clash_core::Dur;
        use serde::{Deserialize, Deserializer, Serialize, Serializer};

        use super::Time;

        /// Reads an optional duration the way [`Time`] does.
        pub fn deserialize<'de, D: Deserializer<'de>>(
            deserializer: D,
        ) -> Result<Option<Dur>, D::Error> {
            Option::<Time>::deserialize(deserializer).map(|time| time.map(|time| time.0))
        }

        /// Writes an optional duration the way `Dur` does.
        pub fn serialize<S: Serializer>(
            value: &Option<Dur>,
            serializer: S,
        ) -> Result<S::Ok, S::Error> {
            value.serialize(serializer)
        }
    }
}

/// Reads the dense index the loader answers a reference with.
struct IndexVisitor<I>(PhantomData<fn() -> I>, &'static str);

impl<'de, I: DenseId> Visitor<'de> for IndexVisitor<I> {
    type Value = I;

    fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}, read by the mod loader", self.1)
    }

    fn visit_u64<E: de::Error>(self, value: u64) -> Result<I, E> {
        usize::try_from(value)
            .ok()
            .and_then(I::from_index)
            .ok_or_else(|| E::custom("there are more of these than their id can count"))
    }

    fn visit_u32<E: de::Error>(self, value: u32) -> Result<I, E> {
        self.visit_u64(u64::from(value))
    }

    fn visit_i64<E: de::Error>(self, value: i64) -> Result<I, E> {
        match u64::try_from(value) {
            Ok(value) => self.visit_u64(value),
            Err(_) => Err(E::custom("an id cannot be negative")),
        }
    }

    fn visit_newtype_struct<D: Deserializer<'de>>(self, deserializer: D) -> Result<I, D::Error> {
        deserializer.deserialize_u64(self)
    }
}

/// A reference to a definition of type `T` (04 §7.1).
///
/// In data it is an id: a bare name, looked up in the namespace of the
/// mod that wrote it and then in that mod's imports, or a qualified
/// `namespace:name` (04 §4.3). It is resolved the moment it is read: a
/// `Ref` always holds the dense id of a definition that exists. A
/// dangling reference is a diagnostic at the exact position of the
/// string, with the closest existing ids as suggestions.
///
/// A `Ref` can be the key of a map: `BTreeMap<Ref<DamageType>, Resist>`
/// reads `{ "impact": { ... }, "blast": { ... } }`.
pub struct Ref<T: Def>(T::Id);

impl<T: Def> Ref<T> {
    const FIELDS: &'static [&'static str] = &[T::TYPE];

    /// A reference to the definition with this dense id.
    pub fn new(id: T::Id) -> Ref<T> {
        Ref(id)
    }

    /// The dense id of the definition referred to.
    pub fn id(self) -> T::Id {
        self.0
    }
}

impl<T: Def> Clone for Ref<T> {
    fn clone(&self) -> Ref<T> {
        *self
    }
}

impl<T: Def> Copy for Ref<T> {}

impl<T: Def> PartialEq for Ref<T> {
    fn eq(&self, other: &Ref<T>) -> bool {
        self.0 == other.0
    }
}

impl<T: Def> Eq for Ref<T> {}

impl<T: Def> PartialOrd for Ref<T> {
    fn partial_cmp(&self, other: &Ref<T>) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl<T: Def> Ord for Ref<T> {
    fn cmp(&self, other: &Ref<T>) -> Ordering {
        self.0.cmp(&other.0)
    }
}

impl<T: Def> Hash for Ref<T> {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.0.hash(state);
    }
}

impl<T: Def> fmt::Debug for Ref<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Ref<{}>({})", T::TYPE, self.0.index())
    }
}

impl<'de, T: Def> Deserialize<'de> for Ref<T> {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Ref<T>, D::Error> {
        deserializer
            .deserialize_struct(
                REF_MAGIC,
                Self::FIELDS,
                IndexVisitor::<T::Id>(PhantomData, "the id of a definition"),
            )
            .map(Ref)
    }
}

/// A definition of type `T` given by id or written in place (04 §5).
///
/// Where a definition is expected inside another (a rule inside an
/// action, a map piece inside a tract) it may be written inline in place
/// of an id. The inline definition is loaded as a definition of its own
/// and registered like any other; this field holds its dense id. Its id
/// is `<parent id>#<path>`, which cannot be written as a reference, so it
/// cannot be referred to from elsewhere. Given an `id` of its own, it is
/// defined under that id in the mod's namespace and can be.
pub struct Inline<T: Def>(T::Id);

impl<T: Def> Inline<T> {
    const FIELDS: &'static [&'static str] = &[T::TYPE];

    /// The dense id of the definition.
    pub fn id(self) -> T::Id {
        self.0
    }
}

impl<T: Def> Clone for Inline<T> {
    fn clone(&self) -> Inline<T> {
        *self
    }
}

impl<T: Def> Copy for Inline<T> {}

impl<T: Def> PartialEq for Inline<T> {
    fn eq(&self, other: &Inline<T>) -> bool {
        self.0 == other.0
    }
}

impl<T: Def> Eq for Inline<T> {}

impl<T: Def> fmt::Debug for Inline<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Inline<{}>({})", T::TYPE, self.0.index())
    }
}

impl<'de, T: Def> Deserialize<'de> for Inline<T> {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Inline<T>, D::Error> {
        deserializer
            .deserialize_struct(
                INLINE_MAGIC,
                Self::FIELDS,
                IndexVisitor::<T::Id>(PhantomData, "an id or a definition written in place"),
            )
            .map(Inline)
    }
}

/// A free-form string given to definitions and tested by rules, interned
/// to a number (04 §7.1).
///
/// The number is a position in the [`TagTable`](crate::TagTable) of the
/// [`Defs`](crate::Defs) it was loaded into: tags are numbered in sorted
/// order, so the same mods always give the same numbers. A tag from one
/// `Defs` means nothing in another; a save names tags by their strings.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Tag(u32);

impl Tag {
    /// The tag at a position in the tag table.
    pub const fn from_index(index: u32) -> Tag {
        Tag(index)
    }

    /// The tag's position in the tag table.
    pub const fn index(self) -> usize {
        self.0 as usize
    }
}

impl fmt::Debug for Tag {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Tag({})", self.0)
    }
}

struct TagVisitor;

impl<'de> Visitor<'de> for TagVisitor {
    type Value = Tag;

    fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("a tag, read by the mod loader")
    }

    fn visit_u32<E: de::Error>(self, value: u32) -> Result<Tag, E> {
        Ok(Tag(value))
    }

    fn visit_u64<E: de::Error>(self, value: u64) -> Result<Tag, E> {
        u32::try_from(value)
            .map(Tag)
            .map_err(|_| E::custom("there are more tags than can be counted"))
    }
}

impl<'de> Deserialize<'de> for Tag {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Tag, D::Error> {
        deserializer.deserialize_newtype_struct(TAG_MAGIC, TagVisitor)
    }
}

/// A set of [`Tag`]s. In data, a list of strings; a repeat is ignored.
#[derive(Clone, PartialEq, Eq, Hash, Default, Debug)]
pub struct TagSet {
    /// Sorted, without repeats.
    tags: Vec<Tag>,
}

impl TagSet {
    /// An empty set.
    pub const fn new() -> TagSet {
        TagSet { tags: Vec::new() }
    }

    /// Whether the set holds `tag`.
    pub fn contains(&self, tag: Tag) -> bool {
        self.tags.binary_search(&tag).is_ok()
    }

    /// Adds `tag`. Returns whether it was new.
    pub fn insert(&mut self, tag: Tag) -> bool {
        match self.tags.binary_search(&tag) {
            Ok(_) => false,
            Err(at) => {
                self.tags.insert(at, tag);
                true
            }
        }
    }

    /// Removes `tag`. Returns whether it was there.
    pub fn remove(&mut self, tag: Tag) -> bool {
        match self.tags.binary_search(&tag) {
            Ok(at) => {
                self.tags.remove(at);
                true
            }
            Err(_) => false,
        }
    }

    /// The tags, in order of their numbers.
    pub fn iter(&self) -> impl Iterator<Item = Tag> + '_ {
        self.tags.iter().copied()
    }

    /// How many tags the set holds.
    pub fn len(&self) -> usize {
        self.tags.len()
    }

    /// Whether the set is empty.
    pub fn is_empty(&self) -> bool {
        self.tags.is_empty()
    }
}

impl FromIterator<Tag> for TagSet {
    fn from_iter<I: IntoIterator<Item = Tag>>(iter: I) -> TagSet {
        let mut tags: Vec<Tag> = iter.into_iter().collect();
        tags.sort_unstable();
        tags.dedup();
        TagSet { tags }
    }
}

impl<'de> Deserialize<'de> for TagSet {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<TagSet, D::Error> {
        Vec::<Tag>::deserialize(deserializer).map(TagSet::from_iter)
    }
}

/// A value together with where it was written.
///
/// Use it for a field whose content is checked or compiled after
/// loading, so that the later error can point into the file: an
/// expression, a state reference, a name looked up in another
/// definition. [`SourceMap::span_in_string`](crate::SourceMap::span_in_string)
/// turns an offset inside a located string into a position in the file.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Located<T> {
    /// The value.
    pub value: T,
    /// Where it was written.
    pub span: Span,
}

enum LocatedField {
    File,
    Start,
    End,
    Value,
    Other,
}

impl<'de> Deserialize<'de> for LocatedField {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<LocatedField, D::Error> {
        struct FieldVisitor;

        impl Visitor<'_> for FieldVisitor {
            type Value = LocatedField;

            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str("a field name")
            }

            fn visit_str<E: de::Error>(self, value: &str) -> Result<LocatedField, E> {
                Ok(match value {
                    "file" => LocatedField::File,
                    "start" => LocatedField::Start,
                    "end" => LocatedField::End,
                    "value" => LocatedField::Value,
                    _ => LocatedField::Other,
                })
            }
        }

        deserializer.deserialize_identifier(FieldVisitor)
    }
}

struct LocatedVisitor<T>(PhantomData<fn() -> T>);

impl<'de, T: Deserialize<'de>> Visitor<'de> for LocatedVisitor<T> {
    type Value = Located<T>;

    fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("a value and its position")
    }

    fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Located<T>, A::Error> {
        let mut file = None;
        let mut start = None;
        let mut end = None;
        let mut value = None;
        while let Some(field) = map.next_key::<LocatedField>()? {
            match field {
                LocatedField::File => file = Some(map.next_value::<u32>()?),
                LocatedField::Start => start = Some(map.next_value::<u32>()?),
                LocatedField::End => end = Some(map.next_value::<u32>()?),
                LocatedField::Value => value = Some(map.next_value::<T>()?),
                LocatedField::Other => {
                    map.next_value::<IgnoredAny>()?;
                }
            }
        }
        let missing = |field| <A::Error as de::Error>::missing_field(field);
        Ok(Located {
            value: value.ok_or_else(|| missing("value"))?,
            span: Span::new(
                FileId(file.ok_or_else(|| missing("file"))?),
                start.ok_or_else(|| missing("start"))?,
                end.ok_or_else(|| missing("end"))?,
            ),
        })
    }
}

impl<'de, T: Deserialize<'de>> Deserialize<'de> for Located<T> {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Located<T>, D::Error> {
        deserializer.deserialize_struct(LOCATED_MAGIC, LOCATED_FIELDS, LocatedVisitor(PhantomData))
    }
}

/// Where a value was written: its place in a file and its JSON path in
/// the definition that holds it (04 §8.2).
///
/// A field that is compiled after loading keeps one, so that what the
/// compiler finds can be reported with file, line, column, definition
/// and path. Read one with [`by_site`].
#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct Site {
    /// Where the value was written, if it was read from a file.
    pub span: Option<Span>,
    /// The JSON path inside the definition: `phases.windup.damage`.
    /// Empty at the definition's top level.
    pub path: String,
}

impl Site {
    /// The site of a value inside this one, under `key`.
    #[must_use]
    pub fn child(&self, key: &str) -> Site {
        let path = if self.path.is_empty() {
            key.to_owned()
        } else {
            format!("{}.{key}", self.path)
        };
        Site {
            span: self.span,
            path,
        }
    }
}

/// A field type that needs to know where it was written and at what
/// JSON path, as well as what kind of JSON it was given. Implement this
/// and let `Deserialize` call [`by_site`]. The expression fields
/// ([`Expr`](crate::Expr), [`Cond`](crate::Cond), [`Text`]) are read
/// this way.
pub trait FromSite<'de>: Sized {
    /// Builds the value, given where it is and what kind of JSON is
    /// there.
    fn from_site<A: MapAccess<'de>>(
        site: Site,
        kind: Kind,
        value: KindValue<'_, A>,
    ) -> Result<Self, A::Error>;
}

struct BySiteVisitor<T>(PhantomData<fn() -> T>);

impl<'de, T: FromSite<'de>> Visitor<'de> for BySiteVisitor<T> {
    type Value = T;

    fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("a value read by the mod loader")
    }

    fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<T, A::Error> {
        let missing =
            || <A::Error as de::Error>::custom("this field can only be read by the mod loader");
        map.next_key::<IgnoredAny>()?.ok_or_else(missing)?;
        let file = map.next_value::<u32>()?;
        map.next_key::<IgnoredAny>()?.ok_or_else(missing)?;
        let start = map.next_value::<u32>()?;
        map.next_key::<IgnoredAny>()?.ok_or_else(missing)?;
        let end = map.next_value::<u32>()?;
        map.next_key::<IgnoredAny>()?.ok_or_else(missing)?;
        let path = map.next_value::<String>()?;
        map.next_key::<IgnoredAny>()?.ok_or_else(missing)?;
        let kind = map.next_value::<Kind>()?;
        map.next_key::<IgnoredAny>()?.ok_or_else(missing)?;
        let site = Site {
            span: Some(Span::new(FileId(file), start, end)),
            path,
        };
        T::from_site(site, kind, KindValue { map: &mut map })
    }
}

/// Reads a field that needs its [`Site`], keeping positions. Like
/// [`by_kind`], with the place and the JSON path as well.
pub fn by_site<'de, T: FromSite<'de>, D: Deserializer<'de>>(
    deserializer: D,
) -> Result<T, D::Error> {
    deserializer.deserialize_struct(SITE_MAGIC, SITE_FIELDS, BySiteVisitor(PhantomData))
}

struct SiteOrPlainVisitor<T, P>(P, PhantomData<fn() -> T>);

impl<'de, T: FromSite<'de>, P: Visitor<'de, Value = T>> Visitor<'de> for SiteOrPlainVisitor<T, P> {
    type Value = T;

    fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.expecting(f)
    }

    fn visit_map<A: MapAccess<'de>>(self, map: A) -> Result<T, A::Error> {
        BySiteVisitor(PhantomData).visit_map(map)
    }

    fn visit_newtype_struct<D: Deserializer<'de>>(self, deserializer: D) -> Result<T, D::Error> {
        deserializer.deserialize_any(self.0)
    }
}

/// Reads a field with its [`Site`] under the mod loader, and with
/// `plain` under any other deserialiser.
///
/// [`by_site`] reads only under the loader. A type that is also read
/// from plain JSON, by a unit test or a tool with `serde_json`, uses
/// this in its place: the loader hands [`FromSite::from_site`] the place,
/// the JSON path and the kind of value as it does for `by_site`, and any
/// other deserialiser hands the value itself to `plain`, which builds
/// the type without a site. What was read without a site cannot be
/// reported at a place, so such a value is for tests and tools only.
pub fn by_site_or_plain<'de, T, D, P>(deserializer: D, plain: P) -> Result<T, D::Error>
where
    T: FromSite<'de>,
    D: Deserializer<'de>,
    P: Visitor<'de, Value = T>,
{
    deserializer
        .deserialize_newtype_struct(SITE_OR_PLAIN_MAGIC, SiteOrPlainVisitor(plain, PhantomData))
}

/// Player-facing text (04 §7.1).
///
/// All player-facing strings use `Text`, so that a translation layer can
/// be added later without touching definitions. A text may contain
/// `{expression}` templates (05 §4.6).
///
/// A text holds the string as written, where it was written and, once
/// its definition has compiled it, the compiled template. A definition
/// type compiles its texts in [`Def::compile`], saying what is in scope
/// there:
///
/// ```ignore
/// fn compile(&mut self, cx: &mut CompileCx<'_>) {
///     let scope = Scope::new().subject("self", "entity");
///     cx.compile(&mut self.log, &scope);
/// }
/// ```
///
/// and renders one with [`Text::render`]. A text that was never compiled
/// renders as it was written.
#[derive(Clone, Debug, Default)]
pub struct Text {
    pub(crate) raw: String,
    pub(crate) site: Site,
    pub(crate) compiled: Option<clash_expr::CompiledText>,
}

impl Text {
    /// A text that was not read from a file.
    pub fn plain(raw: impl Into<String>) -> Text {
        Text {
            raw: raw.into(),
            site: Site::default(),
            compiled: None,
        }
    }

    /// The string as written, templates and all.
    pub fn raw(&self) -> &str {
        &self.raw
    }

    /// Where the string was written, if it was read from a file.
    pub fn span(&self) -> Option<Span> {
        self.site.span
    }

    /// Where the string was written and at what JSON path.
    pub fn site(&self) -> &Site {
        &self.site
    }

    /// Whether the string holds a `{`, and so may hold a template.
    pub fn may_have_templates(&self) -> bool {
        self.raw.contains('{')
    }

    /// The compiled template, once the definition has compiled it.
    pub fn compiled(&self) -> Option<&clash_expr::CompiledText> {
        self.compiled.as_ref()
    }
}

/// Two texts are equal when they were written the same in the same
/// place. Whether one has been compiled yet does not count.
impl PartialEq for Text {
    fn eq(&self, other: &Text) -> bool {
        self.raw == other.raw && self.site.span == other.site.span
    }
}

impl Eq for Text {}

impl fmt::Display for Text {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.raw)
    }
}

impl<'de> FromSite<'de> for Text {
    fn from_site<A: MapAccess<'de>>(
        site: Site,
        _kind: Kind,
        value: KindValue<'_, A>,
    ) -> Result<Text, A::Error> {
        // Whatever the kind, it is read as a string, so that a value of
        // another kind is reported as any wrong value is.
        Ok(Text {
            raw: value.read::<String>()?,
            site,
            compiled: None,
        })
    }
}

impl<'de> Deserialize<'de> for Text {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Text, D::Error> {
        by_site(deserializer)
    }
}

/// What kind of JSON value a field was given, for a field that takes
/// more than one shape. See [`by_kind`].
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Kind {
    /// `null`.
    Null,
    /// `true` or `false`.
    Bool,
    /// A number.
    Number,
    /// A string.
    String,
    /// A list.
    List,
    /// An object.
    Object,
}

impl Kind {
    /// The error for a kind the field does not take. `expected` says
    /// what it does take: `"a block id, or a map of them"`. It is
    /// reported as a value of the wrong kind, at the value.
    pub fn unexpected<E: de::Error>(self, expected: &'static str) -> E {
        let found = match self {
            Kind::Null => de::Unexpected::Unit,
            Kind::Bool => de::Unexpected::Bool(false),
            Kind::Number => de::Unexpected::Float(0.0),
            Kind::String => de::Unexpected::Str(""),
            Kind::List => de::Unexpected::Seq,
            Kind::Object => de::Unexpected::Map,
        };
        E::invalid_type(found, &expected)
    }
}

impl<'de> Deserialize<'de> for Kind {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Kind, D::Error> {
        struct KindVisitor;

        impl Visitor<'_> for KindVisitor {
            type Value = Kind;

            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str("the kind of a value")
            }

            fn visit_str<E: de::Error>(self, value: &str) -> Result<Kind, E> {
                Ok(match value {
                    "null" => Kind::Null,
                    "bool" => Kind::Bool,
                    "number" => Kind::Number,
                    "string" => Kind::String,
                    "list" => Kind::List,
                    "object" => Kind::Object,
                    other => return Err(E::custom(format!("unknown kind `{other}`"))),
                })
            }
        }

        deserializer.deserialize_str(KindVisitor)
    }
}

/// The value [`FromKind::from_kind`] was told the kind of, waiting to be
/// read as whatever type that kind calls for.
pub struct KindValue<'m, A> {
    map: &'m mut A,
}

impl<'de, A: MapAccess<'de>> KindValue<'_, A> {
    /// Reads the value as an `X`. Mistakes inside it are reported where
    /// they are, as for any field.
    pub fn read<X: Deserialize<'de>>(self) -> Result<X, A::Error> {
        self.map.next_value::<X>()
    }
}

/// A field type that takes more than one JSON shape: an id or an object,
/// a number or a string. Implement this and let `Deserialize` call
/// [`by_kind`].
pub trait FromKind<'de>: Sized {
    /// Builds the value, given what kind of JSON is there.
    fn from_kind<A: MapAccess<'de>>(kind: Kind, value: KindValue<'_, A>) -> Result<Self, A::Error>;
}

struct ByKindVisitor<T>(PhantomData<fn() -> T>);

impl<'de, T: FromKind<'de>> Visitor<'de> for ByKindVisitor<T> {
    type Value = T;

    fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("a value read by the mod loader")
    }

    fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<T, A::Error> {
        let missing =
            || <A::Error as de::Error>::custom("this field can only be read by the mod loader");
        map.next_key::<IgnoredAny>()?.ok_or_else(missing)?;
        let kind = map.next_value::<Kind>()?;
        map.next_key::<IgnoredAny>()?.ok_or_else(missing)?;
        T::from_kind(kind, KindValue { map: &mut map })
    }
}

/// Reads a field that takes more than one JSON shape, keeping positions.
///
/// `serde`'s `untagged` cannot be used in a definition (see
/// [`crate::de`]). This is what to use in its place: the loader says
/// what kind of value is there, and the type reads it as the type that
/// kind calls for.
///
/// ```
/// use std::collections::BTreeMap;
/// use clash_data::{by_kind, FromKind, Kind, KindValue};
/// use serde::de::{Deserialize, Deserializer, MapAccess};
///
/// /// A name, or names by language.
/// #[derive(Debug, PartialEq)]
/// enum Label {
///     One(String),
///     Many(BTreeMap<String, String>),
/// }
///
/// impl<'de> FromKind<'de> for Label {
///     fn from_kind<A: MapAccess<'de>>(
///         kind: Kind,
///         value: KindValue<'_, A>,
///     ) -> Result<Label, A::Error> {
///         match kind {
///             Kind::String => value.read().map(Label::One),
///             Kind::Object => value.read().map(Label::Many),
///             other => Err(other.unexpected("a name, or names by language")),
///         }
///     }
/// }
///
/// impl<'de> Deserialize<'de> for Label {
///     fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Label, D::Error> {
///         by_kind(deserializer)
///     }
/// }
///
/// # use clash_data::{from_node, parse_json, Diagnostics, Limits, SourceMap};
/// # let mut sources = SourceMap::new();
/// # let file = sources.add("l.json", r#"["Ash", { "fr": "Cendre" }, 7]"#);
/// # let (tree, _) = parse_json(file, sources.text(file), &Limits::default());
/// # let tree = tree.unwrap();
/// # let items = tree.as_array().unwrap();
/// # let mut diagnostics = Diagnostics::new();
/// # let one: Option<Label> = from_node(&items[0], &sources, &mut diagnostics, None);
/// # assert_eq!(one, Some(Label::One("Ash".to_owned())));
/// # let many: Option<Label> = from_node(&items[1], &sources, &mut diagnostics, None);
/// # assert!(matches!(many, Some(Label::Many(_))));
/// # let bad: Option<Label> = from_node(&items[2], &sources, &mut diagnostics, None);
/// # assert!(bad.is_none());
/// # assert_eq!(
/// #     diagnostics.as_slice()[0].message,
/// #     "expected a name, or names by language, found a number"
/// # );
/// ```
pub fn by_kind<'de, T: FromKind<'de>, D: Deserializer<'de>>(
    deserializer: D,
) -> Result<T, D::Error> {
    deserializer.deserialize_struct(KIND_MAGIC, KIND_FIELDS, ByKindVisitor(PhantomData))
}

/// Names an experimental feature (04 §8.4) for [`Gated`].
pub trait Feature {
    /// The feature's name, as a mod writes it in the `experimental` list
    /// of its `mod.json`.
    const NAME: &'static str;
}

/// A field that is an experimental feature (04 §8.4).
///
/// Write the field as `Option<Gated<MyFeature, T>>` with
/// `#[serde(default)]`. A definition that gives the field uses the
/// feature: that is a load error unless the mod that wrote the value
/// names the feature in its `mod.json`.
pub struct Gated<F: Feature, T>(pub T, PhantomData<fn() -> F>);

impl<F: Feature, T> Gated<F, T> {
    const FIELDS: &'static [&'static str] = &[F::NAME];

    /// The value.
    pub fn into_inner(self) -> T {
        self.0
    }

    /// The value.
    pub fn get(&self) -> &T {
        &self.0
    }
}

impl<F: Feature, T: Clone> Clone for Gated<F, T> {
    fn clone(&self) -> Gated<F, T> {
        Gated(self.0.clone(), PhantomData)
    }
}

impl<F: Feature, T: fmt::Debug> fmt::Debug for Gated<F, T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Gated<{}>({:?})", F::NAME, self.0)
    }
}

impl<F: Feature, T: PartialEq> PartialEq for Gated<F, T> {
    fn eq(&self, other: &Gated<F, T>) -> bool {
        self.0 == other.0
    }
}

struct GatedVisitor<F, T>(PhantomData<fn() -> (F, T)>);

impl<'de, F: Feature, T: Deserialize<'de>> Visitor<'de> for GatedVisitor<F, T> {
    type Value = Gated<F, T>;

    fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "a value of the experimental feature `{}`", F::NAME)
    }

    fn visit_newtype_struct<D: Deserializer<'de>>(
        self,
        deserializer: D,
    ) -> Result<Gated<F, T>, D::Error> {
        T::deserialize(deserializer).map(|value| Gated(value, PhantomData))
    }
}

impl<'de, F: Feature, T: Deserialize<'de>> Deserialize<'de> for Gated<F, T> {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Gated<F, T>, D::Error> {
        deserializer.deserialize_struct(GATED_MAGIC, Self::FIELDS, GatedVisitor(PhantomData))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quantities_know_what_they_measure() {
        assert_eq!(Length::DIM, Dim::LENGTH);
        assert_eq!(<Rate<Mass>>::DIM, Dim::MASS.rate());
        assert_eq!(<Rate<Length>>::DIM, Speed::DIM);
        assert_eq!(<Rate<Energy>>::DIM, Power::DIM);
        assert_eq!(Length::new(Real::of(2.5)).get(), Real::of(2.5));
        assert_eq!(Real::from(Mass(Real::of(80.0))), Real::of(80.0));
        assert_eq!(Pressure(Real::of(7.0e4)).to_string(), "70000 Pa");
        assert_eq!(Rate::<Mass>::default().get(), Real::ZERO);
        // Every name a quantity asks under is one the loader answers.
        let magics: Vec<&str> = crate::de::QUANTITY_MAGICS
            .iter()
            .map(|(magic, _)| *magic)
            .collect();
        for (magic, dim) in [
            (Length::MAGIC, Length::DIM),
            (Volume::MAGIC, Volume::DIM),
            (Mass::MAGIC, Mass::DIM),
            (Energy::MAGIC, Energy::DIM),
            (Force::MAGIC, Force::DIM),
            (Pressure::MAGIC, Pressure::DIM),
            (Temperature::MAGIC, Temperature::DIM),
            (Speed::MAGIC, Speed::DIM),
            (Power::MAGIC, Power::DIM),
            (Frequency::MAGIC, Frequency::DIM),
            (Seconds::MAGIC, Seconds::DIM),
            (Length::RATE_MAGIC, Dim::SPEED),
            (Volume::RATE_MAGIC, Dim::VOLUME.rate()),
            (Mass::RATE_MAGIC, Dim::MASS.rate()),
            (Energy::RATE_MAGIC, Dim::POWER),
            (Force::RATE_MAGIC, Dim::FORCE.rate()),
            (Pressure::RATE_MAGIC, Dim::PRESSURE.rate()),
            (Temperature::RATE_MAGIC, Dim::TEMPERATURE.rate()),
            (Seconds::RATE_MAGIC, Dim::TIME.rate()),
        ] {
            assert!(magics.contains(&magic), "{magic} is not answered");
            let answered = crate::de::QUANTITY_MAGICS
                .iter()
                .find(|(name, _)| *name == magic)
                .map(|(_, dim)| *dim);
            assert_eq!(answered, Some(dim), "{magic}");
        }
    }

    #[test]
    fn quantities_read_bare_numbers_from_other_formats() {
        // A save or a tool reads the SI magnitude back as a plain number.
        let length: Length = serde_json::from_str("2.5").unwrap();
        assert_eq!(length, Length(Real::of(2.5)));
        assert_eq!(serde_json::to_string(&length).unwrap(), "2.5");
        let rate: Rate<Mass> = serde_json::from_str("3").unwrap();
        assert_eq!(rate.get(), Real::of(3.0));
        let time: Time = serde_json::from_str("250000").unwrap();
        assert_eq!(time.dur(), Dur::from_millis(250));
        assert_eq!(serde_json::to_string(&time).unwrap(), "250000");
    }

    #[test]
    fn tag_sets_are_sorted_and_without_repeats() {
        let mut set: TagSet = [Tag(5), Tag(1), Tag(5), Tag(3)].into_iter().collect();
        assert_eq!(set.len(), 3);
        assert!(set.contains(Tag(3)));
        assert!(!set.contains(Tag(2)));
        assert!(set.insert(Tag(2)));
        assert!(!set.insert(Tag(2)));
        assert!(set.remove(Tag(5)));
        assert!(!set.remove(Tag(5)));
        let order: Vec<usize> = set.iter().map(Tag::index).collect();
        assert_eq!(order, [1, 2, 3]);
        assert!(TagSet::new().is_empty());
    }

    #[test]
    fn text_keeps_the_raw_string() {
        let text = Text::plain("You hit {target.name}.");
        assert_eq!(text.raw(), "You hit {target.name}.");
        assert_eq!(text.to_string(), "You hit {target.name}.");
        assert!(text.may_have_templates());
        assert!(text.span().is_none());
        assert!(!Text::plain("Plain.").may_have_templates());
    }
}
