//! Kernel entities: [`EntityTable`], [`KernelEntity`], [`Place`] and what
//! the kernel keeps for each entity (03 §4.2).
//!
//! An entity is an id with a kernel record. Document 06's component
//! store attaches components to the same ids; the kernel fixes only what
//! time, actions and rules depend on.
//!
//! # What the table promises
//!
//! - Ids are generational. A slot is reused, lowest slot first, with its
//!   generation raised by one, so an id held from before no longer
//!   matches.
//! - [`EntityTable::iter`] and [`EntityTable::ids`] visit entities in id
//!   order, which is slot order. Nothing here iterates a hash map.
//! - Entities are created and removed only by
//!   [`Sim::create_entity`](crate::Sim::create_entity) and
//!   [`Sim::remove_entity`](crate::Sim::remove_entity), which emit
//!   `entity_created` and `entity_removed`. A component store follows
//!   those two events and needs nothing else to stay in step.
//!
//! # Who may change what
//!
//! | Field | Changed only by |
//! |-------|-----------------|
//! | `place` | [`Sim::relocate`](crate::Sim::relocate) (01 R3) |
//! | `presence` | [`Sim::set_presence`](crate::Sim::set_presence) |
//! | `control` | [`Sim::set_control`](crate::Sim::set_control) |
//! | `clock`, `anchor` | [`Sim::set_time_rate`](crate::Sim::set_time_rate), [`Sim::anchor_clock`](crate::Sim::anchor_clock) |
//! | `vars` | [`Sim::set_var`](crate::Sim::set_var), which holds a declared variable to its declaration and raises `var_changed` for a watched one. The field is open, for setting a scene up: a write that goes round `set_var` is checked by nothing and told to nobody |
//! | `tags`, `channels`, `actions`, `plan`, `attached` | Whoever holds the entity mutably, until their packages give them owners |
//!
//! `place` is a private field of [`KernelEntity`]. The only code that can
//! name it is this module and its one child module, `relocate`, which
//! holds every function that writes it. Code elsewhere cannot assign it:
//!
//! ```compile_fail
//! use clash_sim::{KernelEntity, Place};
//!
//! fn teleport(entity: &mut KernelEntity) {
//!     entity.place = Place::Stored; // error: field `place` is private
//! }
//! ```
//!
//! Nor can it replace a whole record with another, because an owned
//! `KernelEntity` never leaves this crate: there is no constructor, it is
//! not `Clone`, and the table hands out one `&mut` at a time, so two
//! records cannot be swapped.
//!
//! ```compile_fail
//! use clash_sim::KernelEntity;
//!
//! fn copy(entity: &KernelEntity) -> KernelEntity {
//!     entity.clone() // error: `KernelEntity` is not `Clone`
//! }
//! ```

mod relocate;

pub use relocate::Unplaced;

use std::collections::{BTreeMap, BTreeSet};

use clash_core::{Dur, EntityId, Real, StateHash, StateHasher, VarMap, impl_state_hash, sorted};
use clash_time::ClockId;
use clash_world::{Footprint, Location, Occupant, OccupantFlags, OccupantKind, PlaneMask};
use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::later::{ActionId, ChannelSlots, EocId, Plan};

/// How deeply things may be nested inside one another: a coin in a purse
/// in a pack on a rider on a mount is five. Deeper nesting is refused, so
/// that walking up to the outermost container is bounded.
pub const MAX_CONTAINMENT_DEPTH: usize = 64;

/// Where an entity is (01 §6, 03 §4.2).
///
/// An entity is in the world at a location, inside another entity, or
/// not in the world at all. Only an entity that is `At` a location is in
/// the world's occupant index.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Place {
    /// In the world, at a cell and a height above the cell's base.
    At {
        /// The cell: for an entity that fills several, the corner of its
        /// footprint with the lowest coordinates.
        loc: Location,
        /// Height above the base of the cell, in millimetres.
        alt_mm: u16,
    },
    /// Inside another entity: an item in a pack, a rider on a mount, a
    /// soul in a jar. It is wherever its container is.
    In(EntityId),
    /// Not in the world: cold, or saved with a tract that is not loaded.
    Stored,
}

impl Place {
    /// At a cell, standing on its base.
    pub const fn at(loc: Location) -> Place {
        Place::At { loc, alt_mm: 0 }
    }

    /// The cell, for an entity that is in the world itself.
    pub const fn loc(self) -> Option<Location> {
        match self {
            Place::At { loc, .. } => Some(loc),
            Place::In(_) | Place::Stored => None,
        }
    }

    /// The container, for an entity that is inside another.
    pub const fn container(self) -> Option<EntityId> {
        match self {
            Place::In(container) => Some(container),
            Place::At { .. } | Place::Stored => None,
        }
    }
}

impl StateHash for Place {
    fn state_hash(&self, hasher: &mut StateHasher) {
        match self {
            Place::At { loc, alt_mm } => {
                hasher.write_u8(0);
                loc.state_hash(hasher);
                hasher.write_u16(*alt_mm);
            }
            Place::In(container) => {
                hasher.write_u8(1);
                container.state_hash(hasher);
            }
            Place::Stored => hasher.write_u8(2),
        }
    }
}

/// Who decides for an entity (03 §5.1).
#[derive(
    Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default, Debug, Serialize, Deserialize,
)]
#[serde(rename_all = "snake_case")]
pub enum Control {
    /// The AI layer decides (09).
    #[default]
    None,
    /// The player decides. The simulation stops and asks.
    Direct,
    /// The AI layer decides, following orders the player may give.
    Delegated,
}

impl StateHash for Control {
    fn state_hash(&self, hasher: &mut StateHasher) {
        hasher.write_u8(*self as u8);
    }
}

/// An entity's tags: free-form strings that rules test.
///
/// They are kept as strings, in order, and not as the numbers the loader
/// interns for definitions: a rule may give an entity a tag that no
/// definition carries, and a save names tags by their strings (04 §7.1).
///
/// ```
/// use clash_sim::Tags;
///
/// let mut tags = Tags::new();
/// assert!(tags.insert("guard"));
/// assert!(!tags.insert("guard"));
/// assert!(tags.contains("guard"));
/// assert!(tags.remove("guard"));
/// assert!(tags.is_empty());
/// ```
#[derive(Clone, PartialEq, Eq, Default, Debug, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Tags(BTreeSet<String>);

impl Tags {
    /// No tags.
    pub const fn new() -> Tags {
        Tags(BTreeSet::new())
    }

    /// Whether the set holds a tag.
    pub fn contains(&self, tag: &str) -> bool {
        self.0.contains(tag)
    }

    /// Adds a tag. Returns whether it was new.
    pub fn insert(&mut self, tag: impl Into<String>) -> bool {
        self.0.insert(tag.into())
    }

    /// Removes a tag. Returns whether it was there.
    pub fn remove(&mut self, tag: &str) -> bool {
        self.0.remove(tag)
    }

    /// The tags, in alphabetical order.
    pub fn iter(&self) -> impl Iterator<Item = &str> {
        self.0.iter().map(String::as_str)
    }

    /// How many tags there are.
    pub fn len(&self) -> usize {
        self.0.len()
    }

    /// Whether there are none.
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

impl<S: Into<String>> FromIterator<S> for Tags {
    fn from_iter<I: IntoIterator<Item = S>>(iter: I) -> Tags {
        Tags(iter.into_iter().map(Into::into).collect())
    }
}

impl StateHash for Tags {
    fn state_hash(&self, hasher: &mut StateHasher) {
        self.0.state_hash(hasher);
    }
}

/// What the world is told about an entity that stands in it: its kind,
/// the planes it is on, its mass, what it does to others in its cells
/// and the box of cells it fills (02 §8.1).
///
/// The world neither saves nor hashes who occupies it, so the kernel
/// keeps this for every entity and places the entity again from it after
/// a load. Document 06's `Physical` component derives it from the body
/// and writes it with [`Sim::set_presence`](crate::Sim::set_presence).
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub struct Presence {
    /// What kind of thing it is, for queries.
    pub kind: OccupantKind,
    /// The planes it is on.
    pub plane: PlaneMask,
    /// Its mass in kilograms, with everything it holds.
    pub mass: Real,
    /// What it does to others in its cells.
    pub flags: OccupantFlags,
    /// The box of cells it fills.
    pub footprint: Footprint,
}

impl Presence {
    /// A presence of a kind: on the default plane, with no mass and no
    /// flags, filling one cell.
    pub const fn new(kind: OccupantKind) -> Presence {
        Presence {
            kind,
            plane: PlaneMask::DEFAULT,
            mass: Real::ZERO,
            flags: OccupantFlags::NONE,
            footprint: Footprint::SINGLE,
        }
    }

    /// This presence on other planes.
    #[must_use]
    pub const fn on(mut self, plane: PlaneMask) -> Presence {
        self.plane = plane;
        self
    }

    /// This presence with a mass in kilograms.
    #[must_use]
    pub const fn with_mass(mut self, mass: Real) -> Presence {
        self.mass = mass;
        self
    }

    /// This presence with flags.
    #[must_use]
    pub const fn with_flags(mut self, flags: OccupantFlags) -> Presence {
        self.flags = flags;
        self
    }

    /// This presence filling a box of cells.
    #[must_use]
    pub const fn with_footprint(mut self, footprint: Footprint) -> Presence {
        self.footprint = footprint;
        self
    }

    /// The world's record of an entity with this presence.
    pub const fn occupant(&self, id: EntityId) -> Occupant {
        Occupant {
            id,
            kind: self.kind,
            plane: self.plane,
            mass: self.mass,
            flags: self.flags,
        }
    }
}

impl Default for Presence {
    fn default() -> Presence {
        Presence::new(OccupantKind::Other)
    }
}

impl_state_hash!(Presence {
    kind,
    plane,
    mass,
    flags,
    footprint
});

/// What to make an entity from: the kernel's part of it. Document 06
/// adds components to the entity once it exists.
#[derive(Clone, PartialEq, Eq, Default, Debug, Serialize, Deserialize)]
pub struct EntitySpec {
    /// What the world is told about it.
    pub presence: Presence,
    /// Its tags.
    pub tags: Tags,
    /// Its variables.
    pub vars: VarMap,
    /// Who decides for it.
    pub control: Control,
}

impl EntitySpec {
    /// An entity of a kind, with nothing else said about it.
    pub fn new(kind: OccupantKind) -> EntitySpec {
        EntitySpec {
            presence: Presence::new(kind),
            ..EntitySpec::default()
        }
    }

    /// This entity with a presence.
    #[must_use]
    pub fn with_presence(mut self, presence: Presence) -> EntitySpec {
        self.presence = presence;
        self
    }

    /// This entity with one more tag.
    #[must_use]
    pub fn with_tag(mut self, tag: impl Into<String>) -> EntitySpec {
        self.tags.insert(tag);
        self
    }

    /// This entity under a control mode.
    #[must_use]
    pub fn with_control(mut self, control: Control) -> EntitySpec {
        self.control = control;
        self
    }
}

impl_state_hash!(EntitySpec {
    presence,
    tags,
    vars,
    control
});

/// An entity's own time: how much local time has passed for it, on
/// whatever clocks it has been on, while it was in the world.
///
/// `total` is its own time as of `mark`; since then it has been on the
/// marked clock, whose local time then is kept beside it. An entity that
/// is not in the world has no mark, and its own time stands still.
#[derive(Clone, Copy, PartialEq, Eq, Default, Debug, Serialize, Deserialize)]
pub(crate) struct OwnTime {
    /// Own time as of the mark.
    pub(crate) total: Dur,
    /// The clock the entity has been on since, and that clock's local
    /// time when it joined it.
    pub(crate) mark: Option<(ClockId, Dur)>,
    /// Own time when the entity last decided (03 §5.2).
    pub(crate) decided: Dur,
}

impl_state_hash!(OwnTime {
    total,
    mark,
    decided
});

/// The part of an entity that time, actions and rules depend on
/// (03 §4.2). Document 06 extends it with components and changes nothing
/// in it.
///
/// See the [module documentation](self) for who may change which field.
/// Every accessor here reads ground truth (01 R11): what a character
/// decides by must come from what it has perceived, not from this.
#[derive(PartialEq, Eq, Debug, Serialize, Deserialize)]
pub struct KernelEntity {
    id: EntityId,
    /// Private to this module and `relocate`: see the module
    /// documentation.
    place: Place,
    pub(crate) presence: Presence,
    pub(crate) control: Control,
    pub(crate) clock: Option<ClockId>,
    pub(crate) anchor: Option<ClockId>,
    pub(crate) time: OwnTime,
    /// Free-form strings that rules test.
    pub tags: Tags,
    /// Variables that content stores on the entity (05 §6). Rules read
    /// them with [`Sim::var`](crate::Sim::var), which knows the declared
    /// defaults, and change them with
    /// [`Sim::set_var`](crate::Sim::set_var).
    pub vars: VarMap,
    /// Its channel slots (03 §6.2).
    pub channels: ChannelSlots,
    /// Its running actions.
    pub actions: Vec<ActionId>,
    /// What it will do next (03 §5.3).
    pub plan: Plan,
    /// The rules attached to it (05 §9.3).
    pub attached: Vec<EocId>,
}

impl KernelEntity {
    /// A new entity, not yet in the world.
    fn new(id: EntityId, spec: EntitySpec) -> KernelEntity {
        KernelEntity {
            id,
            place: Place::Stored,
            presence: spec.presence,
            control: spec.control,
            clock: None,
            anchor: None,
            time: OwnTime::default(),
            tags: spec.tags,
            vars: spec.vars,
            channels: ChannelSlots::default(),
            actions: Vec::new(),
            plan: Plan::default(),
            attached: Vec::new(),
        }
    }

    /// A copy, for cloning a whole table. Kept private so that no owned
    /// record can be made outside this module.
    fn duplicate(&self) -> KernelEntity {
        KernelEntity {
            id: self.id,
            place: self.place,
            presence: self.presence,
            control: self.control,
            clock: self.clock,
            anchor: self.anchor,
            time: self.time,
            tags: self.tags.clone(),
            vars: self.vars.clone(),
            channels: self.channels.clone(),
            actions: self.actions.clone(),
            plan: self.plan.clone(),
            attached: self.attached.clone(),
        }
    }

    /// The entity's id.
    pub const fn id(&self) -> EntityId {
        self.id
    }

    /// Where the entity is. Truth.
    pub const fn place(&self) -> Place {
        self.place
    }

    /// What the world is told about it.
    pub const fn presence(&self) -> &Presence {
        &self.presence
    }

    /// Who decides for it.
    pub const fn control(&self) -> Control {
        self.control
    }

    /// Its personal clock, if it has one (03 §3.4).
    pub const fn clock(&self) -> Option<ClockId> {
        self.clock
    }

    /// The clock its personal clock is pinned under, if it is anchored.
    pub const fn anchor(&self) -> Option<ClockId> {
        self.anchor
    }
}

impl_state_hash!(KernelEntity {
    id,
    place,
    presence,
    control,
    clock,
    anchor,
    time,
    tags,
    vars,
    channels,
    actions,
    plan,
    attached
});

/// One slot of the table: how often it has been reused, and who is in it.
#[derive(PartialEq, Eq, Debug, Serialize, Deserialize)]
struct Slot {
    generation: u32,
    entity: Option<KernelEntity>,
}

impl_state_hash!(Slot { generation, entity });

/// Every entity's kernel record, by generational [`EntityId`] (06 §3).
///
/// The slots are the state. The list of free slots and the contents of
/// each container are derived from them and rebuilt on load.
#[derive(PartialEq, Eq, Debug, Default)]
pub struct EntityTable {
    slots: Vec<Slot>,
    /// Derived: the slots holding no entity.
    free: BTreeSet<u32>,
    /// Derived: what is directly inside each container, in id order.
    contents: BTreeMap<EntityId, Vec<EntityId>>,
}

impl EntityTable {
    /// An empty table.
    pub fn new() -> EntityTable {
        EntityTable::default()
    }

    /// A table over these slots, with what is derived worked out.
    fn from_slots(slots: Vec<Slot>) -> EntityTable {
        let mut free = BTreeSet::new();
        let mut contents: BTreeMap<EntityId, Vec<EntityId>> = BTreeMap::new();
        for (index, slot) in slots.iter().enumerate() {
            match &slot.entity {
                Some(entity) => {
                    if let Place::In(container) = entity.place {
                        sorted::insert(contents.entry(container).or_default(), entity.id);
                    }
                }
                None => {
                    free.insert(index as u32);
                }
            }
        }
        EntityTable {
            slots,
            free,
            contents,
        }
    }

    /// How many entities there are.
    pub fn len(&self) -> usize {
        self.slots.len() - self.free.len()
    }

    /// Whether there are none.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Whether an id names a living entity.
    pub fn contains(&self, id: EntityId) -> bool {
        self.get(id).is_some()
    }

    /// An entity's record. `None` for an id that was never given out or
    /// whose entity has been removed, even if its slot is in use again.
    pub fn get(&self, id: EntityId) -> Option<&KernelEntity> {
        self.slots
            .get(id.index())
            .and_then(|slot| slot.entity.as_ref())
            .filter(|entity| entity.id == id)
    }

    /// An entity's record, to change. What can be changed through it is
    /// what the [module documentation](self) lists as open.
    pub fn get_mut(&mut self, id: EntityId) -> Option<&mut KernelEntity> {
        self.slots
            .get_mut(id.index())
            .and_then(|slot| slot.entity.as_mut())
            .filter(|entity| entity.id == id)
    }

    /// Every entity, in id order.
    pub fn iter(&self) -> impl Iterator<Item = &KernelEntity> + '_ {
        self.slots.iter().filter_map(|slot| slot.entity.as_ref())
    }

    /// Every entity's id, in order.
    pub fn ids(&self) -> impl Iterator<Item = EntityId> + '_ {
        self.iter().map(|entity| entity.id)
    }

    /// The id the next entity created will have: the lowest free slot at
    /// its next generation, or a new slot. `None` when every slot is in
    /// use.
    pub fn next_id(&self) -> Option<EntityId> {
        match self.free.first() {
            Some(slot) => {
                let generation = self.slots.get(*slot as usize)?.generation;
                Some(EntityId::new(*slot, generation))
            }
            None => u32::try_from(self.slots.len())
                .ok()
                .map(|slot| EntityId::new(slot, 0)),
        }
    }

    /// What is directly inside a container, in id order. Truth.
    pub fn contents(&self, container: EntityId) -> &[EntityId] {
        self.contents.get(&container).map_or(&[], Vec::as_slice)
    }

    /// Everything inside a container at any depth, parents before what
    /// they hold, and those of one container in id order. Truth.
    pub fn contents_deep(&self, container: EntityId) -> Vec<EntityId> {
        let mut found = Vec::new();
        let mut stack: Vec<EntityId> = self.contents(container).iter().rev().copied().collect();
        while let Some(id) = stack.pop() {
            found.push(id);
            stack.extend(self.contents(id).iter().rev());
        }
        found
    }

    /// The entity that is in the world on behalf of this one: itself, or
    /// the container that everything it is nested in is inside. `None`
    /// for an unknown entity or a chain that leads nowhere. Truth.
    pub fn outermost(&self, id: EntityId) -> Option<EntityId> {
        let mut at = id;
        for _ in 0..=MAX_CONTAINMENT_DEPTH {
            match self.get(at)?.place {
                Place::In(container) => at = container,
                Place::At { .. } | Place::Stored => return Some(at),
            }
        }
        None
    }

    /// How many containers an entity is nested in: zero for one that is
    /// in the world itself or stored.
    pub fn depth(&self, id: EntityId) -> Option<usize> {
        let mut at = id;
        for depth in 0..=MAX_CONTAINMENT_DEPTH {
            match self.get(at)?.place {
                Place::In(container) => at = container,
                Place::At { .. } | Place::Stored => return Some(depth),
            }
        }
        None
    }

    /// Where an entity is in the world: its own cell, or that of its
    /// outermost container. `None` if it is not in the world. Truth.
    pub fn location(&self, id: EntityId) -> Option<Location> {
        self.get(self.outermost(id)?)?.place.loc()
    }

    /// Makes an entity, not yet in the world, and returns its id. `None`
    /// when every slot is in use.
    pub(crate) fn create(&mut self, spec: EntitySpec) -> Option<EntityId> {
        let id = self.next_id()?;
        let entity = KernelEntity::new(id, spec);
        match self.free.pop_first() {
            Some(slot) => self.slots.get_mut(slot as usize)?.entity = Some(entity),
            None => self.slots.push(Slot {
                generation: 0,
                entity: Some(entity),
            }),
        }
        Some(id)
    }

    /// Forgets an entity and makes its slot ready for another, a
    /// generation on. The caller has taken it out of the world and
    /// emptied it first.
    pub(crate) fn free(&mut self, id: EntityId) -> bool {
        if !self.contains(id) {
            return false;
        }
        self.set_place(id, Place::Stored);
        self.contents.remove(&id);
        if let Some(slot) = self.slots.get_mut(id.index()) {
            slot.entity = None;
            slot.generation = id.next_generation().generation();
            self.free.insert(id.slot());
        }
        true
    }

    /// Writes an entity's place and keeps the lists of contents in step.
    /// Private: reachable only from this module and `relocate`.
    fn set_place(&mut self, id: EntityId, place: Place) {
        let Some(entity) = self.get_mut(id) else {
            return;
        };
        let old = core::mem::replace(&mut entity.place, place);
        if let Place::In(container) = old
            && let Some(inside) = self.contents.get_mut(&container)
        {
            sorted::remove(inside, &id);
            if inside.is_empty() {
                self.contents.remove(&container);
            }
        }
        if let Place::In(container) = place {
            sorted::insert(self.contents.entry(container).or_default(), id);
        }
    }
}

impl Clone for EntityTable {
    fn clone(&self) -> EntityTable {
        EntityTable {
            slots: self
                .slots
                .iter()
                .map(|slot| Slot {
                    generation: slot.generation,
                    entity: slot.entity.as_ref().map(KernelEntity::duplicate),
                })
                .collect(),
            free: self.free.clone(),
            contents: self.contents.clone(),
        }
    }
}

/// Hashes the slots in order: each one's generation and whoever is in it.
impl StateHash for EntityTable {
    fn state_hash(&self, hasher: &mut StateHasher) {
        self.slots.state_hash(hasher);
    }
}

impl Serialize for EntityTable {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        self.slots.serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for EntityTable {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<EntityTable, D::Error> {
        Vec::<Slot>::deserialize(deserializer).map(EntityTable::from_slots)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use clash_core::{Hash128, codec};

    fn spec() -> EntitySpec {
        EntitySpec::new(OccupantKind::Creature)
    }

    #[test]
    fn ids_are_generational_and_slots_are_reused_lowest_first() {
        let mut table = EntityTable::new();
        let a = table.create(spec()).unwrap();
        let b = table.create(spec()).unwrap();
        let c = table.create(spec()).unwrap();
        assert_eq!(
            [a, b, c],
            [
                EntityId::new(0, 0),
                EntityId::new(1, 0),
                EntityId::new(2, 0)
            ]
        );
        assert!(table.free(b));
        assert!(table.free(a));
        assert!(!table.free(a), "an entity is removed once");
        assert_eq!(table.len(), 1);
        assert!(!table.contains(a));

        // The lowest free slot first, a generation on.
        assert_eq!(table.next_id(), Some(EntityId::new(0, 1)));
        let again = table.create(spec()).unwrap();
        assert_eq!(again, EntityId::new(0, 1));
        assert!(table.get(a).is_none(), "the old id no longer matches");
        assert!(table.get(again).is_some());
        assert_eq!(table.create(spec()), Some(EntityId::new(1, 1)));
        assert_eq!(table.create(spec()), Some(EntityId::new(3, 0)));
    }

    #[test]
    fn entities_are_visited_in_id_order() {
        let mut table = EntityTable::new();
        let ids: Vec<EntityId> = (0..6).map(|_| table.create(spec()).unwrap()).collect();
        table.free(ids[4]);
        table.free(ids[1]);
        let late = table.create(spec()).unwrap();
        assert_eq!(late.slot(), 1);
        let visited: Vec<EntityId> = table.ids().collect();
        let mut sorted = visited.clone();
        sorted.sort();
        assert_eq!(visited, sorted);
        assert_eq!(visited.len(), 5);
    }

    #[test]
    fn a_table_survives_the_codec_with_what_is_derived_rebuilt() {
        let mut table = EntityTable::new();
        let pack = table.create(spec()).unwrap();
        let gone = table.create(spec()).unwrap();
        let coin = table.create(spec().with_tag("shiny")).unwrap();
        table.set_place(coin, Place::In(pack));
        table.free(gone);

        let bytes = codec::to_bytes(&table).unwrap();
        let back: EntityTable = codec::from_bytes(&bytes).unwrap();
        assert_eq!(back, table);
        assert_eq!(Hash128::of(&back), Hash128::of(&table));
        assert_eq!(back.contents(pack), [coin]);
        assert_eq!(back.next_id(), Some(EntityId::new(1, 1)));
        assert_eq!(table.clone(), table);
    }

    #[test]
    fn containment_is_walked_to_the_outermost() {
        let mut table = EntityTable::new();
        let mount = table.create(spec()).unwrap();
        let rider = table.create(spec()).unwrap();
        let pack = table.create(spec()).unwrap();
        let coin = table.create(spec()).unwrap();
        table.set_place(rider, Place::In(mount));
        table.set_place(pack, Place::In(rider));
        table.set_place(coin, Place::In(pack));
        assert_eq!(table.outermost(coin), Some(mount));
        assert_eq!(table.depth(coin), Some(3));
        assert_eq!(table.contents_deep(mount), [rider, pack, coin]);
        assert_eq!(table.location(coin), None, "the mount is stored");
        table.free(pack);
        assert_eq!(table.contents(rider), [] as [EntityId; 0]);
    }
}
