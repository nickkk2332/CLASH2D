//! Components: what an entity is made of beyond the kernel entity
//! (`docs/06-entities-and-bodies.md` section 3).
//!
//! An entity is an id with components. The kernel entity of 03 §4.2
//! (place, tags, variables, control, clock, channels, plan) is the
//! kernel's. Everything else an entity has is a **component**: a plain
//! value of one of a fixed set of types, kept with every other component
//! of its type in a [`ComponentStore`] and found by [`EntityId`].
//!
//! | Part | Is |
//! |------|----|
//! | [`ComponentStore`] | One component type: dense storage, lookup by id, iteration in a saved order |
//! | [`Components`] | Every store behind one value: saved, loaded and hashed together |
//! | [`ComponentKind`], [`ComponentKinds`] | The component types by name, and a set of them |
//! | [`ComponentBundle`] | One entity's components taken out of the stores, or ready to go in |
//! | [`Component`] | What a component type implements; written by the list in this file |
//! | [`Identity`] | What a thing is: its definition, own name and description, templates applied |
//! | [`Physical`], [`Shape`] | How much of a thing there is, with the mass of what it holds derived and cached |
//! | [`Containment`] | Who is inside whom, as [`physical`] needs to be told it |
//!
//! Nothing here knows the simulation. The stores are keyed by id and are
//! told about containment through a trait, so they can be built, tested,
//! saved and hashed on their own.
//!
//! ```
//! use clash_core::{EntityId, Hash128, codec};
//! use clash_sim::component::{ComponentKind, Components, Identity};
//!
//! let mut components = Components::default();
//! let crate_ = EntityId::new(0, 0);
//! components.identity.insert(crate_, Identity::new(None));
//!
//! // By type, and by kind for code that only has a name.
//! assert!(components.get::<Identity>(crate_).is_some());
//! assert!(components.has(ComponentKind::Identity, crate_));
//! assert_eq!(ComponentKind::from_name("identity"), Some(ComponentKind::Identity));
//!
//! // Saved and loaded, it is the same state.
//! let bytes = codec::to_bytes(&components).unwrap();
//! let loaded: Components = codec::from_bytes(&bytes).unwrap();
//! assert_eq!(Hash128::of(&loaded), Hash128::of(&components));
//!
//! // Destroying the entity takes everything it had, in one call.
//! let removed = components.remove_entity(crate_);
//! assert!(removed.kinds().has(ComponentKind::Identity));
//! assert!(components.kinds_of(crate_).is_empty());
//! ```
//!
//! # Order, saving and hashing
//!
//! A store iterates in its dense order, which the operations on it decide
//! and nothing else (see [`ComponentStore`]). That order is state: it is
//! what a system walking the store sees, so it is saved, restored and
//! hashed as it is (06 invariant B8). A store is not re-sorted on saving,
//! because a run that saved and loaded would then walk entities in a
//! different order from one that did not (01 R7).
//!
//! [`Components`] serialises as its stores in the order of the list in
//! this file, each a sequence of `(owner, component)` pairs, and hashes
//! in the same order. [`Components::hash_of`] gives one store's hash, to
//! localise a divergence.
//!
//! # Adding a component type
//!
//! Content never adds component types (decision D-064); packages do. For
//! a component `Stats`:
//!
//! 1. Write the type in a module of its own: `component/stats.rs`, or a
//!    module elsewhere in the crate. Keep its fields private and change
//!    them only through that module's functions, which return what
//!    changed so that the caller can raise events (06 §3). Derive or
//!    write `Clone`, `PartialEq`, `Debug`, `Serialize`, `Deserialize` and
//!    [`StateHash`], within what `clash_core::codec` can read: no
//!    `flatten`, no untagged or internally tagged enums, no
//!    `skip_serializing_if`.
//! 2. Add one line to the `components!` list at the end of this file:
//!    `stats: Stats = "stats";`, with a `use` for the type if it lives
//!    in another module. Add it **at the end**: the position in the list
//!    is the store's place in a save and its [`ComponentKind`]'s number.
//!
//! That line gives the type a store (`components.stats`), a
//! `ComponentKind::Stats` named `"stats"`, a place in saves and in the
//! hash, a field in [`ComponentBundle`], and removal with the entity.
//! Nothing else in this module changes. A layout that changed means a new
//! version of whatever blob holds the components (01 §11.1).
//!
//! # What the kernel holds and calls
//!
//! The simulation holds one [`Components`] beside its entity table.
//!
//! | When | The kernel |
//! |------|-----------|
//! | An entity is created | Calls nothing. Whoever builds the entity inserts the components it is to have: `components.identity.insert(id, …)`, [`physical::set_shape`]. |
//! | An entity is destroyed | Calls [`Components::remove_entity`] before its slot can be given out again, then [`physical::refresh`] on the container it was in, if any. |
//! | An entity is stored and brought back (06 §12.3) | Keeps the [`ComponentBundle`] from `remove_entity` with the stored entity and hands it to [`Components::insert_bundle`] under the id it returns with, then calls [`physical::refresh`] on it. |
//! | `relocate` moves an entity into or out of a container | Calls [`physical::refresh`] on the old container and on the new one. |
//! | A mass changed ([`MassChange`]) for an entity in the world | Passes the new mass to the world's occupant record (02 §8). |
//! | Saving | Writes the [`Components`] as one section. |
//! | Loading | Reads it back, and may check [`Components::orphans`] against its entity table: a component whose owner is not alive is damage. |
//! | Hashing | Feeds the [`Components`] to the simulation's hasher, and offers [`Components::hash_of`] among its subsystem hashes. |
//!
//! The kernel implements [`Containment`] over its entities' places:
//! `container` is the `In` of an entity's place, and `contents` lists
//! the entities whose place is `In` it.
//!
//! An id whose slot has been given to a newer entity never finds the
//! older one's components, and never the newer one's either.
//!
//! # What is not built here
//!
//! - **Properties and events.** The expression registry is the kernel's.
//!   The functions that will back the properties of 06 §13 are here to
//!   be registered: `mass` ([`Physical::mass`]), `carried_mass`,
//!   `volume`, `height`, `name` ([`Identity::name`], falling back to the
//!   definition's) and `templates`. A generic "has this component" is
//!   [`Components::has`].
//! - **Definition ids in saves.** [`DefKey`] and [`TemplateId`] hold
//!   runtime ids. Writing them by qualified id through the save's id
//!   table (01 §7) belongs to the simulation's save path.
//! - **A change log.** 06 §3 asks for none. A module that changes a
//!   component returns what changed, and the caller raises the events
//!   that presentation follows.

pub mod identity;
pub mod physical;
mod store;
#[cfg(test)]
mod tests;

use core::fmt;

use clash_core::{EntityId, Hash128, StateHash, StateHasher, impl_state_hash};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

pub use identity::{DefKey, Identity, TemplateId};
pub use physical::{
    Containment, MAX_CONTAINMENT_DEPTH, MassChange, MassChanges, Physical, PhysicalError, Shape,
};
pub use store::{ComponentStore, PAGE_SLOTS};

use crate::stats::Stats;

/// A type of component: one of the fixed set the engine defines (06 §3).
///
/// Implemented by the `components!` list in this module for each type it
/// names, and by nothing else.
pub trait Component:
    Sized + Clone + PartialEq + fmt::Debug + StateHash + Serialize + DeserializeOwned + 'static
{
    /// Which component type this is.
    const KIND: ComponentKind;

    /// This type's store.
    fn store(components: &Components) -> &ComponentStore<Self>;

    /// This type's store, to change.
    fn store_mut(components: &mut Components) -> &mut ComponentStore<Self>;
}

/// A set of component types.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub struct ComponentKinds(u32);

impl ComponentKinds {
    /// No component types.
    pub const NONE: ComponentKinds = ComponentKinds(0);

    /// The set of these types.
    pub const fn of(kinds: &[ComponentKind]) -> ComponentKinds {
        let mut bits = 0;
        let mut at = 0;
        while at < kinds.len() {
            bits |= kinds[at].bit();
            at += 1;
        }
        ComponentKinds(bits)
    }

    /// Whether the set holds `kind`.
    pub const fn has(self, kind: ComponentKind) -> bool {
        self.0 & kind.bit() != 0
    }

    /// Adds `kind` to the set.
    pub const fn insert(&mut self, kind: ComponentKind) {
        self.0 |= kind.bit();
    }

    /// Whether the set is empty.
    pub const fn is_empty(self) -> bool {
        self.0 == 0
    }

    /// How many types the set holds.
    pub const fn len(self) -> usize {
        self.0.count_ones() as usize
    }

    /// The types in the set, in the order of [`ComponentKind::ALL`].
    pub fn iter(self) -> impl Iterator<Item = ComponentKind> {
        ComponentKind::ALL
            .iter()
            .copied()
            .filter(move |kind| self.has(*kind))
    }
}

impl fmt::Debug for ComponentKinds {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_set()
            .entries(self.iter().map(ComponentKind::name))
            .finish()
    }
}

impl StateHash for ComponentKinds {
    fn state_hash(&self, hasher: &mut StateHasher) {
        hasher.write_u32(self.0);
    }
}

impl StateHash for ComponentKind {
    fn state_hash(&self, hasher: &mut StateHasher) {
        hasher.write_u8(*self as u8);
    }
}

impl fmt::Display for ComponentKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}

impl ComponentKind {
    /// The kind's position in [`ComponentKind::ALL`].
    pub const fn index(self) -> usize {
        self as usize
    }

    /// The kind's bit in a [`ComponentKinds`].
    const fn bit(self) -> u32 {
        1 << (self as u32)
    }
}

impl Components {
    /// No components at all.
    pub fn new() -> Components {
        Components::default()
    }

    /// The store of component type `C`.
    pub fn store<C: Component>(&self) -> &ComponentStore<C> {
        C::store(self)
    }

    /// The store of component type `C`, to change.
    pub fn store_mut<C: Component>(&mut self) -> &mut ComponentStore<C> {
        C::store_mut(self)
    }

    /// `id`'s component of type `C`, if it has one.
    pub fn get<C: Component>(&self, id: EntityId) -> Option<&C> {
        C::store(self).get(id)
    }

    /// `id`'s component of type `C`, to change in place, if it has one.
    pub fn get_mut<C: Component>(&mut self, id: EntityId) -> Option<&mut C> {
        C::store_mut(self).get_mut(id)
    }

    /// Gives `id` a component, returning the one of that type its slot
    /// held before. See [`ComponentStore::insert`].
    pub fn insert<C: Component>(&mut self, id: EntityId, component: C) -> Option<C> {
        C::store_mut(self).insert(id, component)
    }

    /// Takes `id`'s component of type `C` away.
    pub fn remove<C: Component>(&mut self, id: EntityId) -> Option<C> {
        C::store_mut(self).remove(id)
    }

    /// The component types `id` has.
    pub fn kinds_of(&self, id: EntityId) -> ComponentKinds {
        let mut kinds = ComponentKinds::NONE;
        for &kind in ComponentKind::ALL {
            if self.has(kind, id) {
                kinds.insert(kind);
            }
        }
        kinds
    }

    /// Every component whose owner `alive` does not vouch for, by kind
    /// and then in dense order. Empty when the stores agree with the
    /// entity table.
    pub fn orphans(&self, alive: impl Fn(EntityId) -> bool) -> Vec<(ComponentKind, EntityId)> {
        let mut orphans = Vec::new();
        for &kind in ComponentKind::ALL {
            for &owner in self.owners_of(kind) {
                if !alive(owner) {
                    orphans.push((kind, owner));
                }
            }
        }
        orphans
    }
}

/// Declares the component types: for each, the field of [`Components`]
/// that holds its store, the type, and its name. Generates
/// [`ComponentKind`], [`Components`], [`ComponentBundle`] and each
/// type's [`Component`] implementation.
macro_rules! components {
    ($( $(#[$doc:meta])* $field:ident: $ty:ident = $name:literal; )+) => {
        /// The component types, in the order their stores are saved and
        /// hashed.
        ///
        /// This is how code that has only a name, such as the expression
        /// layer, asks about a component: [`ComponentKind::from_name`],
        /// then [`Components::has`].
        #[derive(
            Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug, Serialize, Deserialize,
        )]
        #[repr(u8)]
        pub enum ComponentKind {
            $( $(#[$doc])* $ty, )+
        }

        impl ComponentKind {
            /// Every component type, in order.
            pub const ALL: &'static [ComponentKind] = &[ $( ComponentKind::$ty, )+ ];

            /// The type's name, in `snake_case`.
            pub const fn name(self) -> &'static str {
                match self {
                    $( ComponentKind::$ty => $name, )+
                }
            }

            /// The type with this name.
            pub fn from_name(name: &str) -> Option<ComponentKind> {
                match name {
                    $( $name => Some(ComponentKind::$ty), )+
                    _ => None,
                }
            }
        }

        /// Every component of every entity: one [`ComponentStore`] for
        /// each component type.
        ///
        /// The stores are public fields, so that two can be borrowed at
        /// once: `components.physical` changed while
        /// `components.identity` is read. What may be done to a
        /// component is still up to its own module, whose fields are
        /// private.
        ///
        /// Serialises as its stores in order, and hashes in the same
        /// order.
        #[derive(Clone, PartialEq, Debug, Default, Serialize, Deserialize)]
        pub struct Components {
            $( $(#[$doc])* pub $field: ComponentStore<$ty>, )+
        }

        impl_state_hash!(Components { $( $field, )+ });

        /// One entity's components, out of the stores: what
        /// [`Components::remove_entity`] removed, or what
        /// [`Components::insert_bundle`] is to put in. A stored entity
        /// keeps its components as one of these (06 §12.3).
        #[derive(Clone, PartialEq, Debug, Default, Serialize, Deserialize)]
        pub struct ComponentBundle {
            $( $(#[$doc])* pub $field: Option<$ty>, )+
        }

        impl_state_hash!(ComponentBundle { $( $field, )+ });

        impl ComponentBundle {
            /// The component types the bundle holds.
            pub fn kinds(&self) -> ComponentKinds {
                let mut kinds = ComponentKinds::NONE;
                $(
                    if self.$field.is_some() {
                        kinds.insert(ComponentKind::$ty);
                    }
                )+
                kinds
            }

            /// Whether the bundle holds nothing.
            pub fn is_empty(&self) -> bool {
                self.kinds().is_empty()
            }
        }

        $(
            impl Component for $ty {
                const KIND: ComponentKind = ComponentKind::$ty;

                fn store(components: &Components) -> &ComponentStore<$ty> {
                    &components.$field
                }

                fn store_mut(components: &mut Components) -> &mut ComponentStore<$ty> {
                    &mut components.$field
                }
            }
        )+

        impl Components {
            /// Whether `id` has a component of this kind.
            pub fn has(&self, kind: ComponentKind, id: EntityId) -> bool {
                match kind {
                    $( ComponentKind::$ty => self.$field.contains(id), )+
                }
            }

            /// How many components of this kind there are.
            pub fn count(&self, kind: ComponentKind) -> usize {
                match kind {
                    $( ComponentKind::$ty => self.$field.len(), )+
                }
            }

            /// The owners of the components of this kind, in dense
            /// order.
            pub fn owners_of(&self, kind: ComponentKind) -> &[EntityId] {
                match kind {
                    $( ComponentKind::$ty => self.$field.owners(), )+
                }
            }

            /// The state hash of one kind's store, to find which store
            /// two simulations differ in.
            pub fn hash_of(&self, kind: ComponentKind) -> Hash128 {
                match kind {
                    $( ComponentKind::$ty => Hash128::of(&self.$field), )+
                }
            }

            /// Takes every component `id` has out of every store, and
            /// returns them. Afterwards `id` has no components.
            ///
            /// The kernel calls this when it destroys or stores an
            /// entity, before the entity's slot can be given out again.
            pub fn remove_entity(&mut self, id: EntityId) -> ComponentBundle {
                ComponentBundle {
                    $( $field: self.$field.remove(id), )+
                }
            }

            /// Gives `id` every component in `bundle`, and returns what
            /// each replaced. A type the bundle does not hold is left as
            /// it is.
            pub fn insert_bundle(
                &mut self,
                id: EntityId,
                bundle: ComponentBundle,
            ) -> ComponentBundle {
                ComponentBundle {
                    $(
                        $field: match bundle.$field {
                            Some(component) => self.$field.insert(id, component),
                            None => None,
                        },
                    )+
                }
            }
        }
    };
}

components! {
    /// What a thing is: its definition, own name and description, and
    /// the templates applied to it. On everything.
    identity: Identity = "identity";
    /// How much of a thing there is: mass, volume, height, footprint,
    /// cross-section. On anything in the world.
    physical: Physical = "physical";
    /// Attributes, pools and tracks. On creatures and some items.
    stats: Stats = "stats";
}

// A `ComponentKinds` has one bit for each kind.
const _: () = assert!(ComponentKind::ALL.len() <= u32::BITS as usize);
