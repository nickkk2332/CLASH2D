//! [`ComponentStore`]: one component type, kept densely and found by
//! [`EntityId`] (06 §3).

use core::fmt;

use clash_core::{EntityId, StateHash, StateHasher};
use serde::de::Error as _;
use serde::ser::SerializeSeq;
use serde::{Deserialize, Deserializer, Serialize, Serializer};

/// How many entity slots one page of a store's index covers.
///
/// The index from slot to dense position is kept in pages so that a store
/// held by a few entities among very many costs a few pages and not one
/// number for every entity in the simulation, and so that an id with a
/// huge slot read from a damaged save cannot ask for a huge allocation.
pub const PAGE_SLOTS: usize = 1024;

/// One page of the index: the dense position of each of its slots. An
/// entry means something only if the owner at that position has the
/// slot, so a page starts as zeros and is never cleaned.
type Page = Box<[u32; PAGE_SLOTS]>;

/// Every component of one type, kept densely and found by [`EntityId`].
///
/// This is the sparse set of 06 §3. The components sit side by side in
/// one list, the **dense order**, with the id of each one's owner beside
/// it. A paged index turns an entity's slot into a position in that list.
///
/// ```
/// use clash_core::EntityId;
/// use clash_sim::component::ComponentStore;
///
/// let mut names: ComponentStore<String> = ComponentStore::new();
/// let ash = EntityId::new(4, 0);
/// let oak = EntityId::new(9, 0);
/// names.insert(ash, "ash".to_owned());
/// names.insert(oak, "oak".to_owned());
/// assert_eq!(names.get(ash).map(String::as_str), Some("ash"));
/// assert_eq!(names.len(), 2);
///
/// // The slot is reused by a newer entity: the old id finds nothing.
/// assert_eq!(names.remove(ash), Some("ash".to_owned()));
/// let reused = ash.next_generation();
/// names.insert(reused, "elm".to_owned());
/// assert_eq!(names.get(ash), None);
/// assert!(names.contains(reused));
/// ```
///
/// # One component for each slot
///
/// A store holds at most one component for an entity slot, whatever the
/// generation. [`insert`](ComponentStore::insert) for an id replaces what
/// the slot held and hands it back. An id of another generation than the
/// owner's never matches: `get`, `get_mut`, `contains` and `remove` treat
/// it as absent. Whoever destroys an entity removes its components before
/// the slot is given out again (see the [module](super) documentation),
/// so a component of an older generation is never left to be replaced.
///
/// # Order
///
/// Iteration is in dense order, and dense order is part of the state
/// (06 invariant B8). It is decided by the operations alone:
///
/// - a new component goes to the end;
/// - a replaced component keeps its position;
/// - a removed component's position is taken by the last one.
///
/// The store is saved as its components in dense order, so a loaded
/// store iterates exactly as the saved one did, and [`StateHash`] feeds
/// the same order. Two stores that hold the same components in different
/// orders are different states: what iterates them would visit entities
/// in a different order from then on. The index is rebuilt on loading and
/// is neither saved nor hashed.
///
/// Nothing can be inserted or removed during iteration. A system that
/// must do so copies [`owners`](ComponentStore::owners) first.
#[derive(Clone)]
pub struct ComponentStore<C> {
    /// The components, in dense order.
    dense: Vec<C>,
    /// The owner of each component: `owners[i]` owns `dense[i]`. No two
    /// owners share a slot.
    owners: Vec<EntityId>,
    /// Entity slot to position in `dense`, in pages of [`PAGE_SLOTS`].
    sparse: Vec<Option<Page>>,
}

impl<C> ComponentStore<C> {
    /// An empty store.
    pub const fn new() -> ComponentStore<C> {
        ComponentStore {
            dense: Vec::new(),
            owners: Vec::new(),
            sparse: Vec::new(),
        }
    }

    /// How many components the store holds.
    pub fn len(&self) -> usize {
        self.dense.len()
    }

    /// Whether the store holds nothing.
    pub fn is_empty(&self) -> bool {
        self.dense.is_empty()
    }

    /// The dense position the index gives for a slot, which may be out of
    /// date or never have been written.
    fn indexed(&self, slot: usize) -> Option<usize> {
        let page = self.sparse.get(slot / PAGE_SLOTS)?.as_ref()?;
        Some(page[slot % PAGE_SLOTS] as usize)
    }

    /// The dense position of the component in the same slot as `id`,
    /// whatever its generation.
    fn position_of_slot(&self, id: EntityId) -> Option<usize> {
        let position = self.indexed(id.index())?;
        let owner = self.owners.get(position)?;
        (owner.slot() == id.slot()).then_some(position)
    }

    /// The dense position of `id`'s component.
    fn position(&self, id: EntityId) -> Option<usize> {
        let position = self.indexed(id.index())?;
        (self.owners.get(position) == Some(&id)).then_some(position)
    }

    /// Points the index for `id`'s slot at a dense position.
    fn index(&mut self, id: EntityId, position: usize) {
        let slot = id.index();
        let page = slot / PAGE_SLOTS;
        if self.sparse.len() <= page {
            self.sparse.resize_with(page + 1, || None);
        }
        let page = self.sparse[page].get_or_insert_with(|| Box::new([0; PAGE_SLOTS]));
        // Owners have distinct slots and a slot is a `u32`, so a position
        // is always below 2^32.
        page[slot % PAGE_SLOTS] = position as u32;
    }

    /// Whether `id` has a component here.
    pub fn contains(&self, id: EntityId) -> bool {
        self.position(id).is_some()
    }

    /// `id`'s component, if it has one.
    pub fn get(&self, id: EntityId) -> Option<&C> {
        self.position(id)
            .and_then(|position| self.dense.get(position))
    }

    /// `id`'s component, to change in place, if it has one.
    pub fn get_mut(&mut self, id: EntityId) -> Option<&mut C> {
        self.position(id)
            .and_then(|position| self.dense.get_mut(position))
    }

    /// Gives `id` a component, and says where it went and what the slot
    /// held before.
    fn place(&mut self, id: EntityId, component: C) -> (usize, Option<C>) {
        if let Some(position) = self.position_of_slot(id) {
            self.owners[position] = id;
            let before = core::mem::replace(&mut self.dense[position], component);
            return (position, Some(before));
        }
        let position = self.dense.len();
        self.dense.push(component);
        self.owners.push(id);
        self.index(id, position);
        (position, None)
    }

    /// Gives `id` a component. Returns the component its slot held
    /// before, if any, which keeps its position; a new one goes to the
    /// end of the dense order.
    pub fn insert(&mut self, id: EntityId, component: C) -> Option<C> {
        self.place(id, component).1
    }

    /// `id`'s component, made by `make` and inserted if it has none.
    pub fn get_or_insert_with(&mut self, id: EntityId, make: impl FnOnce() -> C) -> &mut C {
        let position = match self.position(id) {
            Some(position) => position,
            None => self.place(id, make()).0,
        };
        &mut self.dense[position]
    }

    /// Takes `id`'s component away. The last component in dense order
    /// takes its position.
    pub fn remove(&mut self, id: EntityId) -> Option<C> {
        let position = self.position(id)?;
        let component = self.dense.swap_remove(position);
        self.owners.swap_remove(position);
        if let Some(moved) = self.owners.get(position).copied() {
            self.index(moved, position);
        }
        Some(component)
    }

    /// Removes every component.
    pub fn clear(&mut self) {
        self.dense.clear();
        self.owners.clear();
        self.sparse.clear();
    }

    /// The owners, in dense order.
    pub fn owners(&self) -> &[EntityId] {
        &self.owners
    }

    /// The components, in dense order: `values()[i]` belongs to
    /// `owners()[i]`.
    pub fn values(&self) -> &[C] {
        &self.dense
    }

    /// Every owner with its component, in dense order.
    pub fn iter(&self) -> impl ExactSizeIterator<Item = (EntityId, &C)> + DoubleEndedIterator {
        self.owners.iter().copied().zip(self.dense.iter())
    }

    /// Every owner with its component to change in place, in dense order.
    pub fn iter_mut(
        &mut self,
    ) -> impl ExactSizeIterator<Item = (EntityId, &mut C)> + DoubleEndedIterator {
        self.owners.iter().copied().zip(self.dense.iter_mut())
    }

    /// Builds a store from owners and components in dense order, as they
    /// were saved. Refuses two components for one slot.
    fn from_dense(entries: Vec<(EntityId, C)>) -> Result<ComponentStore<C>, DuplicateSlot> {
        let mut store = ComponentStore {
            dense: Vec::with_capacity(entries.len()),
            owners: Vec::with_capacity(entries.len()),
            sparse: Vec::new(),
        };
        for (id, component) in entries {
            if store.position_of_slot(id).is_some() {
                return Err(DuplicateSlot(id));
            }
            store.insert(id, component);
        }
        Ok(store)
    }
}

/// A saved store named two components for one entity slot.
struct DuplicateSlot(EntityId);

impl fmt::Display for DuplicateSlot {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "a component store holds two components for entity slot {}",
            self.0.slot()
        )
    }
}

impl<C> Default for ComponentStore<C> {
    fn default() -> ComponentStore<C> {
        ComponentStore::new()
    }
}

/// Equal when they hold the same components for the same owners in the
/// same dense order.
impl<C: PartialEq> PartialEq for ComponentStore<C> {
    fn eq(&self, other: &ComponentStore<C>) -> bool {
        self.owners == other.owners && self.dense == other.dense
    }
}

impl<C: Eq> Eq for ComponentStore<C> {}

impl<C: fmt::Debug> fmt::Debug for ComponentStore<C> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_map().entries(self.iter()).finish()
    }
}

/// The number of components, then each owner and its component in dense
/// order.
impl<C: StateHash> StateHash for ComponentStore<C> {
    fn state_hash(&self, hasher: &mut StateHasher) {
        hasher.write_len(self.len());
        for (owner, component) in self.iter() {
            owner.state_hash(hasher);
            component.state_hash(hasher);
        }
    }
}

/// A sequence of `(owner, component)` pairs in dense order.
impl<C: Serialize> Serialize for ComponentStore<C> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut entries = serializer.serialize_seq(Some(self.len()))?;
        for entry in self.iter() {
            entries.serialize_element(&entry)?;
        }
        entries.end()
    }
}

/// Reads what [`Serialize`] wrote, keeping the dense order. Two
/// components for one entity slot are refused.
impl<'de, C: Deserialize<'de>> Deserialize<'de> for ComponentStore<C> {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<ComponentStore<C>, D::Error> {
        let entries = Vec::<(EntityId, C)>::deserialize(deserializer)?;
        ComponentStore::from_dense(entries).map_err(D::Error::custom)
    }
}
