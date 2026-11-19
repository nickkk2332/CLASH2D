//! Typed ids: [`define_id!`](crate::define_id) for dense ids, the
//! [`DenseId`] trait they share, and the generational [`EntityId`].

use core::fmt;
use core::hash::Hash;

use serde::{Deserialize, Serialize};

use crate::hash::{StateHash, StateHasher};

/// An id that is a position in a dense table.
///
/// Every type made by [`define_id!`](crate::define_id) implements this, so
/// that containers indexed by an id can be written once for all of them.
pub trait DenseId: Copy + Eq + Ord + Hash + fmt::Debug {
    /// The id for a position in a table, or `None` if the position is too
    /// large for the id's width.
    fn from_index(index: usize) -> Option<Self>;

    /// The id's position in a table.
    fn index(self) -> usize;
}

/// Defines one or more typed dense ids.
///
/// A dense id is a small number that stands for one thing of one kind: a
/// definition, a grid, a clock. Giving each kind its own type stops a block
/// id from being used where a material id is wanted.
///
/// ```
/// use clash_core::{define_id, DenseId};
///
/// define_id! {
///     /// A kind of terrain.
///     pub struct TerrainId;
///     /// A slot in a chunk's palette. Sixteen bits are enough.
///     pub struct PaletteIx(u16);
/// }
///
/// let grass = TerrainId::new(3);
/// assert_eq!(grass.raw(), 3);
/// assert_eq!(grass.index(), 3);
/// assert_eq!(TerrainId::from_index(3), Some(grass));
/// assert_eq!(PaletteIx::from_index(70_000), None);
/// assert_eq!(format!("{grass:?}"), "TerrainId(3)");
/// ```
///
/// The number is a `u32` unless another unsigned width is given in
/// brackets. Each id is `Copy`, ordered by its number, hashable, has
/// [`StateHash`](crate::StateHash) and [`DenseId`], and serialises as its
/// bare number.
///
/// Two different ids do not mix:
///
/// ```compile_fail
/// use clash_core::define_id;
///
/// define_id! {
///     /// One kind.
///     pub struct BlockId;
///     /// Another kind.
///     pub struct MaterialId;
/// }
///
/// let block: BlockId = MaterialId::new(1);
/// ```
#[macro_export]
macro_rules! define_id {
    (@one [$(#[$meta:meta])*] [$vis:vis] $name:ident $repr:ty) => {
        $(#[$meta])*
        #[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
        $vis struct $name($repr);

        #[allow(dead_code)]
        impl $name {
            /// The id with this number.
            #[inline]
            pub const fn new(raw: $repr) -> Self {
                Self(raw)
            }

            /// The id's number.
            #[inline]
            pub const fn raw(self) -> $repr {
                self.0
            }

            /// The id's position in a dense table.
            #[inline]
            pub const fn index(self) -> usize {
                self.0 as usize
            }

            /// The id for a position in a dense table, or `None` if the
            /// position is too large for the id's width.
            #[inline]
            pub fn from_index(index: usize) -> ::core::option::Option<Self> {
                match <$repr as ::core::convert::TryFrom<usize>>::try_from(index) {
                    ::core::result::Result::Ok(raw) => ::core::option::Option::Some(Self(raw)),
                    ::core::result::Result::Err(_) => ::core::option::Option::None,
                }
            }
        }

        impl $crate::DenseId for $name {
            #[inline]
            fn from_index(index: usize) -> ::core::option::Option<Self> {
                $name::from_index(index)
            }

            #[inline]
            fn index(self) -> usize {
                $name::index(self)
            }
        }

        impl ::core::fmt::Debug for $name {
            fn fmt(&self, f: &mut ::core::fmt::Formatter<'_>) -> ::core::fmt::Result {
                write!(f, "{}({})", stringify!($name), self.0)
            }
        }

        impl ::core::fmt::Display for $name {
            fn fmt(&self, f: &mut ::core::fmt::Formatter<'_>) -> ::core::fmt::Result {
                ::core::fmt::Display::fmt(&self.0, f)
            }
        }

        impl $crate::StateHash for $name {
            fn state_hash(&self, hasher: &mut $crate::StateHasher) {
                $crate::StateHash::state_hash(&self.0, hasher);
            }
        }

        impl $crate::__private::serde::Serialize for $name {
            fn serialize<S: $crate::__private::serde::Serializer>(
                &self,
                serializer: S,
            ) -> ::core::result::Result<S::Ok, S::Error> {
                $crate::__private::serde::Serialize::serialize(&self.0, serializer)
            }
        }

        impl<'de> $crate::__private::serde::Deserialize<'de> for $name {
            fn deserialize<D: $crate::__private::serde::Deserializer<'de>>(
                deserializer: D,
            ) -> ::core::result::Result<Self, D::Error> {
                <$repr as $crate::__private::serde::Deserialize<'de>>::deserialize(deserializer)
                    .map(Self)
            }
        }
    };
    () => {};
    ($(#[$meta:meta])* $vis:vis struct $name:ident; $($rest:tt)*) => {
        $crate::define_id!(@one [$(#[$meta])*] [$vis] $name u32);
        $crate::define_id!($($rest)*);
    };
    ($(#[$meta:meta])* $vis:vis struct $name:ident($repr:ty); $($rest:tt)*) => {
        $crate::define_id!(@one [$(#[$meta])*] [$vis] $name $repr);
        $crate::define_id!($($rest)*);
    };
}

/// The identity of one entity: a slot in the entity table and the
/// generation of that slot.
///
/// When an entity is destroyed its slot can be given to a new entity, with
/// the generation raised by one. An `EntityId` held from before then no
/// longer matches, so a stale reference is noticed instead of silently
/// meaning someone else (01 §6).
///
/// Ids order by slot, then by generation.
///
/// ```
/// use clash_core::EntityId;
///
/// let first = EntityId::new(7, 0);
/// let reused = first.next_generation();
/// assert_eq!(reused.slot(), 7);
/// assert_eq!(reused.generation(), 1);
/// assert_ne!(first, reused);
/// assert_eq!(EntityId::from_bits(first.to_bits()), first);
/// assert_eq!(reused.to_string(), "7v1");
/// ```
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct EntityId {
    slot: u32,
    generation: u32,
}

impl EntityId {
    /// The id for a slot at a generation.
    pub const fn new(slot: u32, generation: u32) -> EntityId {
        EntityId { slot, generation }
    }

    /// The slot in the entity table.
    pub const fn slot(self) -> u32 {
        self.slot
    }

    /// The slot as a position in a dense table.
    pub const fn index(self) -> usize {
        self.slot as usize
    }

    /// How many times the slot had been reused when this id was made.
    pub const fn generation(self) -> u32 {
        self.generation
    }

    /// The id for the next entity to use the same slot. The generation
    /// wraps round after `u32::MAX` reuses.
    pub const fn next_generation(self) -> EntityId {
        EntityId {
            slot: self.slot,
            generation: self.generation.wrapping_add(1),
        }
    }

    /// Packs the id into one number: the generation in the upper 32 bits
    /// and the slot in the lower. Useful where an id must travel as a plain
    /// `u64`, such as in a wake-up's payload.
    pub const fn to_bits(self) -> u64 {
        ((self.generation as u64) << 32) | self.slot as u64
    }

    /// Unpacks an id packed by [`to_bits`](EntityId::to_bits).
    pub const fn from_bits(bits: u64) -> EntityId {
        EntityId {
            slot: bits as u32,
            generation: (bits >> 32) as u32,
        }
    }
}

/// Written as `EntityId(7v1)`: slot 7, generation 1.
impl fmt::Debug for EntityId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "EntityId({}v{})", self.slot, self.generation)
    }
}

/// Written as `7v1`: slot 7, generation 1.
impl fmt::Display for EntityId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}v{}", self.slot, self.generation)
    }
}

impl StateHash for EntityId {
    fn state_hash(&self, hasher: &mut StateHasher) {
        hasher.write_u32(self.slot);
        hasher.write_u32(self.generation);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Hash128;

    define_id! {
        /// A test id of the default width.
        struct WideId;
        /// A test id sixteen bits wide.
        struct NarrowId(u16);
        /// A test id sixty-four bits wide.
        struct HugeId(u64);
    }

    #[test]
    fn dense_ids_convert_to_and_from_indices() {
        let id = WideId::new(41);
        assert_eq!(id.raw(), 41_u32);
        assert_eq!(id.index(), 41);
        assert_eq!(WideId::from_index(41), Some(id));
        assert_eq!(NarrowId::from_index(65_535), Some(NarrowId::new(65_535)));
        assert_eq!(NarrowId::from_index(65_536), None);
        assert_eq!(HugeId::new(1 << 40).index(), 1 << 40);
        assert_eq!(<WideId as DenseId>::from_index(2), Some(WideId::new(2)));
        assert_eq!(DenseId::index(NarrowId::new(9)), 9);
    }

    #[test]
    fn dense_ids_order_format_and_hash_by_number() {
        assert!(WideId::new(1) < WideId::new(2));
        assert_eq!(format!("{:?}", NarrowId::new(5)), "NarrowId(5)");
        assert_eq!(WideId::new(5).to_string(), "5");
        assert_eq!(Hash128::of(&WideId::new(5)), Hash128::of(&5_u32));
        assert_eq!(Hash128::of(&NarrowId::new(5)), Hash128::of(&5_u16));
        assert_eq!(Hash128::of(&HugeId::new(5)), Hash128::of(&5_u64));
    }

    #[test]
    fn dense_ids_serialise_as_bare_numbers() {
        assert_eq!(serde_json::to_string(&WideId::new(12)).unwrap(), "12");
        assert_eq!(
            serde_json::from_str::<WideId>("12").unwrap(),
            WideId::new(12)
        );
        assert!(serde_json::from_str::<NarrowId>("70000").is_err());
        assert!(serde_json::from_str::<WideId>("-1").is_err());
    }

    #[test]
    fn entity_ids_pack_and_unpack() {
        let id = EntityId::new(0xdead_beef, 0x0123_4567);
        assert_eq!(id.to_bits(), 0x0123_4567_dead_beef);
        assert_eq!(EntityId::from_bits(id.to_bits()), id);
        assert_eq!(id.slot(), 0xdead_beef);
        assert_eq!(id.index(), 0xdead_beef);
        assert_eq!(id.generation(), 0x0123_4567);
    }

    #[test]
    fn entity_generations_tell_reuses_apart() {
        let first = EntityId::new(3, 0);
        let second = first.next_generation();
        assert_ne!(first, second);
        assert_eq!(second, EntityId::new(3, 1));
        assert_eq!(
            EntityId::new(3, u32::MAX).next_generation(),
            EntityId::new(3, 0)
        );
        // Slot first, then generation.
        assert!(EntityId::new(2, 9) < EntityId::new(3, 0));
        assert!(first < second);
        assert_eq!(format!("{second:?}"), "EntityId(3v1)");
        assert_eq!(second.to_string(), "3v1");
    }

    #[test]
    fn entity_ids_hash_and_serialise() {
        let id = EntityId::new(3, 1);
        assert_eq!(Hash128::of(&id), Hash128::of(&(3_u32, 1_u32)));
        let json = serde_json::to_string(&id).unwrap();
        assert_eq!(json, r#"{"slot":3,"generation":1}"#);
        assert_eq!(serde_json::from_str::<EntityId>(&json).unwrap(), id);
    }
}
