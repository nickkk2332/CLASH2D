//! Tests of the component stores, the registry and the two core
//! components.
//!
//! The property tests drive the real thing with random scripts and
//! compare it with the simplest possible reference: for a store, a plain
//! list searched from the front; for `Physical`, masses summed again from
//! nothing by code that shares none with the module.

use std::collections::BTreeMap;

use clash_core::{EntityId, Hash128, Real, codec};
use clash_data::{Length, Mass, Volume};
use clash_expr::DefTypeId;
use clash_time::ClockId;
use clash_world::Footprint;
use proptest::prelude::*;

use super::physical::{self, Containment, MAX_CONTAINMENT_DEPTH, PhysicalError, Shape};
use super::{
    Component, ComponentBundle, ComponentKind, ComponentKinds, ComponentStore, Components, DefKey,
    Identity, PAGE_SLOTS, Physical, TemplateId,
};
use crate::body::{Body, BodyPlanId};
use crate::movement::Mover;
use crate::stats::Stats;

// ---- The store against a model ---------------------------------------------

/// The reference for a store: owners and values in dense order, with the
/// rules of 06 §3 and nothing else.
#[derive(Clone, Debug, Default, PartialEq)]
struct Model {
    dense: Vec<(EntityId, u32)>,
}

impl Model {
    fn get(&self, id: EntityId) -> Option<u32> {
        self.dense
            .iter()
            .find(|(owner, _)| *owner == id)
            .map(|(_, value)| *value)
    }

    fn insert(&mut self, id: EntityId, value: u32) -> Option<u32> {
        for entry in &mut self.dense {
            if entry.0.slot() == id.slot() {
                let before = entry.1;
                *entry = (id, value);
                return Some(before);
            }
        }
        self.dense.push((id, value));
        None
    }

    fn remove(&mut self, id: EntityId) -> Option<u32> {
        let at = self.dense.iter().position(|(owner, _)| *owner == id)?;
        Some(self.dense.swap_remove(at).1)
    }
}

/// One step of a script against a store.
#[derive(Clone, Copy, Debug)]
enum StoreOp {
    Insert(EntityId, u32),
    Remove(EntityId),
    Change(EntityId, u32),
    GetOrInsert(EntityId, u32),
    Clear,
}

/// Ids over a few slots, so that scripts collide: neighbours, both sides
/// of a page of the index, and slots far away.
fn any_id() -> impl Strategy<Value = EntityId> {
    let page = PAGE_SLOTS as u32;
    let slot = prop_oneof![
        6 => 0..10_u32,
        2 => (page - 2)..(page + 2),
        1 => Just(7 * page + 3),
        1 => Just(40 * page + 1),
    ];
    (slot, 0..3_u32).prop_map(|(slot, generation)| EntityId::new(slot, generation))
}

fn store_op() -> impl Strategy<Value = StoreOp> {
    prop_oneof![
        8 => (any_id(), any::<u32>()).prop_map(|(id, value)| StoreOp::Insert(id, value)),
        5 => any_id().prop_map(StoreOp::Remove),
        2 => (any_id(), any::<u32>()).prop_map(|(id, value)| StoreOp::Change(id, value)),
        2 => (any_id(), any::<u32>()).prop_map(|(id, value)| StoreOp::GetOrInsert(id, value)),
        1 => Just(StoreOp::Clear),
    ]
}

/// Applies one step to a store and to the model, and checks that they
/// answered alike.
fn apply(store: &mut ComponentStore<u32>, model: &mut Model, op: StoreOp) {
    match op {
        StoreOp::Insert(id, value) => {
            assert_eq!(store.insert(id, value), model.insert(id, value));
        }
        StoreOp::Remove(id) => {
            assert_eq!(store.remove(id), model.remove(id));
            assert!(!store.contains(id));
        }
        StoreOp::Change(id, value) => {
            let held = store.get_mut(id);
            let expected = model.dense.iter_mut().find(|(owner, _)| *owner == id);
            assert_eq!(held.is_some(), expected.is_some());
            if let (Some(held), Some(expected)) = (held, expected) {
                *held = value;
                expected.1 = value;
            }
        }
        StoreOp::GetOrInsert(id, value) => {
            let held = *store.get_or_insert_with(id, || value);
            if model.get(id).is_none() {
                model.insert(id, value);
            }
            assert_eq!(Some(held), model.get(id));
        }
        StoreOp::Clear => {
            store.clear();
            model.dense.clear();
        }
    }
}

/// Everything an observer can see of a store, against the model.
fn assert_matches(store: &ComponentStore<u32>, model: &Model) {
    assert_eq!(store.len(), model.dense.len());
    assert_eq!(store.is_empty(), model.dense.is_empty());
    let seen: Vec<(EntityId, u32)> = store.iter().map(|(owner, value)| (owner, *value)).collect();
    assert_eq!(seen, model.dense, "iteration is in dense order");
    let owners: Vec<EntityId> = model.dense.iter().map(|(owner, _)| *owner).collect();
    let values: Vec<u32> = model.dense.iter().map(|(_, value)| *value).collect();
    assert_eq!(store.owners(), owners);
    assert_eq!(store.values(), values);
    for &(owner, value) in &model.dense {
        assert_eq!(store.get(owner), Some(&value));
        assert!(store.contains(owner));
        // No other generation of the slot matches.
        for generation in 0..4 {
            let other = EntityId::new(owner.slot(), generation);
            if other != owner {
                assert_eq!(store.get(other), None);
                assert!(!store.contains(other));
            }
        }
    }
}

fn saved_and_loaded<T: serde::Serialize + serde::de::DeserializeOwned>(value: &T) -> T {
    let bytes = codec::to_bytes(value).expect("state encodes");
    codec::from_bytes(&bytes).expect("what was written reads back")
}

proptest! {
    /// Store operations match the model, step by step (E1, first item).
    #[test]
    fn a_store_matches_its_model(ops in prop::collection::vec(store_op(), 0..120)) {
        let mut store = ComponentStore::new();
        let mut model = Model::default();
        for op in ops {
            apply(&mut store, &mut model, op);
            prop_assert_eq!(store.len(), model.dense.len());
        }
        assert_matches(&store, &model);
    }

    /// Iteration order survives saving and loading, in the binary
    /// encoding and in JSON, and the loaded store goes on exactly as the
    /// one that was never saved (06 invariant B8, 01 R7).
    #[test]
    fn dense_order_survives_save_and_load(
        before in prop::collection::vec(store_op(), 0..80),
        after in prop::collection::vec(store_op(), 0..80),
    ) {
        let mut store = ComponentStore::new();
        let mut model = Model::default();
        for op in before {
            apply(&mut store, &mut model, op);
        }

        let mut loaded: ComponentStore<u32> = saved_and_loaded(&store);
        prop_assert_eq!(&loaded, &store);
        prop_assert_eq!(Hash128::of(&loaded), Hash128::of(&store));
        assert_matches(&loaded, &model);

        let json = serde_json::to_string(&store).expect("state writes as JSON");
        let from_json: ComponentStore<u32> = serde_json::from_str(&json).expect("and reads back");
        prop_assert_eq!(&from_json, &store);

        let mut loaded_model = model.clone();
        for op in after {
            apply(&mut store, &mut model, op);
            apply(&mut loaded, &mut loaded_model, op);
        }
        prop_assert_eq!(&loaded, &store);
        prop_assert_eq!(Hash128::of(&loaded), Hash128::of(&store));
        prop_assert_eq!(
            codec::to_bytes(&loaded).expect("state encodes"),
            codec::to_bytes(&store).expect("state encodes")
        );
    }

    /// Damaged bytes are refused or read; they never panic (01 R6).
    #[test]
    fn damaged_bytes_never_panic(bytes in prop::collection::vec(any::<u8>(), 0..64)) {
        let _ = codec::from_bytes::<ComponentStore<u32>>(&bytes);
        let _ = codec::from_bytes::<Components>(&bytes);
    }
}

#[test]
fn a_stale_id_never_matches() {
    let mut store = ComponentStore::new();
    let first = EntityId::new(3, 0);
    let second = first.next_generation();

    store.insert(first, "first");
    assert_eq!(store.get(second), None);
    assert_eq!(store.get_mut(second), None);
    assert!(!store.contains(second));
    assert_eq!(store.remove(second), None, "a stale id removes nothing");
    assert_eq!(store.get(first), Some(&"first"));

    // The entity is destroyed and its slot reused.
    assert_eq!(store.remove(first), Some("first"));
    assert_eq!(store.get(first), None);
    assert_eq!(store.get(second), None);
    store.insert(second, "second");
    assert_eq!(
        store.get(first),
        None,
        "the old id does not see the new owner"
    );
    assert_eq!(store.remove(first), None);
    assert_eq!(store.get(second), Some(&"second"));
    assert_eq!(store.len(), 1);
}

#[test]
fn the_last_slot_there_is_can_hold_a_component() {
    let mut store = ComponentStore::new();
    let last = EntityId::new(u32::MAX, u32::MAX);
    let first = EntityId::new(0, 0);
    store.insert(last, 'z');
    store.insert(first, 'a');
    assert_eq!(store.get(last), Some(&'z'));
    assert_eq!(store.get(EntityId::new(u32::MAX - 1, u32::MAX)), None);
    assert_eq!(store.get(EntityId::new(u32::MAX, 0)), None);
    let loaded: ComponentStore<char> = saved_and_loaded(&store);
    assert_eq!(loaded.owners(), &[last, first]);
    assert_eq!(store.remove(last), Some('z'));
    assert_eq!(store.owners(), &[first]);
    assert_eq!(store.get(first), Some(&'a'));
}

#[test]
fn a_slot_holds_one_component_whatever_the_generation() {
    let mut store = ComponentStore::new();
    let other = EntityId::new(9, 0);
    let old = EntityId::new(3, 0);
    store.insert(old, 1);
    store.insert(other, 2);

    // Not removed before its slot was reused: the newcomer replaces it,
    // in the same position.
    assert_eq!(store.insert(old.next_generation(), 3), Some(1));
    assert_eq!(store.owners(), &[old.next_generation(), other]);
    assert_eq!(store.values(), &[3, 2]);
    assert_eq!(store.get(old), None);
}

#[test]
fn removal_moves_the_last_component_into_the_gap() {
    let ids: Vec<EntityId> = (0..5).map(|slot| EntityId::new(slot, 0)).collect();
    let mut store = ComponentStore::new();
    for (value, &id) in ids.iter().enumerate() {
        store.insert(id, value);
    }
    assert_eq!(store.remove(ids[1]), Some(1));
    assert_eq!(store.owners(), &[ids[0], ids[4], ids[2], ids[3]]);
    assert_eq!(store.remove(ids[3]), Some(3));
    assert_eq!(store.owners(), &[ids[0], ids[4], ids[2]]);
    assert_eq!(store.get(ids[4]), Some(&4));

    let changed: Vec<usize> = store
        .iter_mut()
        .map(|(_, value)| {
            *value += 10;
            *value
        })
        .collect();
    assert_eq!(changed, [10, 14, 12]);
}

#[test]
fn hashing_agrees_between_built_and_loaded_and_follows_dense_order() {
    let a = EntityId::new(1, 0);
    let b = EntityId::new(2, 0);
    let c = EntityId::new(700_000, 2);

    let mut built = ComponentStore::new();
    built.insert(a, 10_u32);
    built.insert(b, 20);
    built.insert(c, 30);
    built.remove(a);
    let loaded: ComponentStore<u32> = saved_and_loaded(&built);
    assert_eq!(Hash128::of(&loaded), Hash128::of(&built));
    assert_eq!(loaded.owners(), &[c, b]);

    // The same contents reached another way, in another order, are
    // another state: iteration would differ.
    let mut other = ComponentStore::new();
    other.insert(b, 20_u32);
    other.insert(c, 30);
    assert_ne!(other, built);
    assert_ne!(Hash128::of(&other), Hash128::of(&built));

    // The same operations give the same state.
    let mut again = ComponentStore::new();
    again.insert(a, 10_u32);
    again.insert(b, 20);
    again.insert(c, 30);
    again.remove(a);
    assert_eq!(again, built);
    assert_eq!(Hash128::of(&again), Hash128::of(&built));
}

#[test]
fn a_save_with_two_components_for_one_slot_is_refused() {
    let twice = vec![(EntityId::new(4, 0), 1_u32), (EntityId::new(4, 1), 2)];
    let bytes = codec::to_bytes(&twice).expect("a list encodes");
    let error = codec::from_bytes::<ComponentStore<u32>>(&bytes).expect_err("refused");
    assert!(error.to_string().contains("slot 4"), "{error}");

    let once = vec![(EntityId::new(4, 0), 1_u32), (EntityId::new(5, 1), 2)];
    let bytes = codec::to_bytes(&once).expect("a list encodes");
    let store: ComponentStore<u32> = codec::from_bytes(&bytes).expect("read");
    assert_eq!(store.get(EntityId::new(5, 1)), Some(&2));
}

#[test]
fn a_component_that_takes_no_bytes_can_still_be_saved() {
    // A marker component: the owner beside it is what the codec counts.
    let mut marked: ComponentStore<()> = ComponentStore::new();
    marked.insert(EntityId::new(1, 0), ());
    marked.insert(EntityId::new(2, 0), ());
    assert_eq!(saved_and_loaded(&marked), marked);
}

// ---- The registry ----------------------------------------------------------

fn shape(mass: f64) -> Shape {
    Shape {
        mass: Mass::new(Real::of(mass)),
        volume: Volume::new(Real::of(0.07)),
        height: Length::new(Real::of(1.7)),
        footprint: Footprint::SINGLE,
        cross_section: Real::of(0.6),
    }
}

/// A full set of components, differing by `seed`.
fn bundle(seed: u32) -> ComponentBundle {
    let mut identity = Identity::new(Some(DefKey::new(DefTypeId::new(2), seed)));
    identity.rename(Some(format!("thing {seed}")));
    identity.apply_template(TemplateId::new(seed));
    ComponentBundle {
        identity: Some(identity),
        physical: Some(Physical::new(shape(f64::from(seed))).expect("a valid shape")),
        stats: Some(Stats::new(ClockId::new(seed))),
        body: Some(Body::new(
            BodyPlanId::new(seed as u16),
            Real::ONE,
            ClockId::new(seed),
        )),
        item: Some(crate::item::Item::sample(seed)),
        holds: Some(crate::item::Holds::default()),
        mover: Some(Mover::default()),
        mind: Some(crate::mind::Mind::new(Some(format!("profile {seed}")))),
        driver: Some(crate::mind::Driver {
            by: EntityId::new(seed + 1, 0),
        }),
        senses: Some(crate::perception::Senses::default()),
        signature: Some(crate::perception::Signature::default()),
        awareness: Some(crate::perception::Awareness::default()),
        beliefs: Some(crate::perception::Beliefs::default()),
        map_memory: Some(crate::perception::MapMemory::default()),
        mind_state: Some(crate::ai::MindState::sample(seed)),
    }
}

#[test]
fn kinds_have_distinct_names_in_order() {
    assert_eq!(ComponentKind::ALL.len(), 15);
    for (index, &kind) in ComponentKind::ALL.iter().enumerate() {
        assert_eq!(kind.index(), index);
        assert_eq!(ComponentKind::from_name(kind.name()), Some(kind));
        assert_eq!(kind.to_string(), kind.name());
    }
    assert_eq!(ComponentKind::Identity.name(), "identity");
    assert_eq!(ComponentKind::Physical.name(), "physical");
    assert_eq!(ComponentKind::Stats.name(), "stats");
    assert_eq!(ComponentKind::Body.name(), "body");
    assert_eq!(ComponentKind::Mover.name(), "mover");
    assert_eq!(ComponentKind::from_name("hit_points"), None);
    assert_eq!(<Identity as Component>::KIND, ComponentKind::Identity);
    assert_eq!(<Physical as Component>::KIND, ComponentKind::Physical);

    let all = ComponentKinds::of(ComponentKind::ALL);
    assert_eq!(all.len(), 15);
    assert_eq!(all.iter().collect::<Vec<_>>(), ComponentKind::ALL);
    assert_eq!(
        format!("{all:?}"),
        r#"{"identity", "physical", "stats", "body", "item", "holds", "mover", "mind", "driver", "senses", "signature", "awareness", "beliefs", "map_memory", "mind_state"}"#
    );
    assert!(ComponentKinds::NONE.is_empty());
}

#[test]
fn removing_an_entity_clears_every_store_and_reports_what_went() {
    let mut components = Components::new();
    let gone = EntityId::new(5, 1);
    let stays = EntityId::new(6, 0);
    let bare = EntityId::new(7, 0);
    assert!(components.insert_bundle(gone, bundle(1)).is_empty());
    assert!(components.insert_bundle(stays, bundle(2)).is_empty());
    components.insert(bare, Identity::new(None));

    for &kind in ComponentKind::ALL {
        assert!(components.has(kind, gone));
        assert!(!components.has(kind, gone.next_generation()));
    }
    assert_eq!(
        components.kinds_of(bare),
        ComponentKinds::of(&[ComponentKind::Identity])
    );
    assert_eq!(components.count(ComponentKind::Identity), 3);
    assert_eq!(components.count(ComponentKind::Physical), 2);

    let removed = components.remove_entity(gone);
    assert_eq!(removed, bundle(1));
    assert_eq!(removed.kinds(), ComponentKinds::of(ComponentKind::ALL));
    for &kind in ComponentKind::ALL {
        assert!(!components.has(kind, gone), "{kind} is gone");
        assert!(
            components.has(kind, stays),
            "{kind} of another entity stays"
        );
    }
    assert!(components.kinds_of(gone).is_empty());
    assert!(components.get::<Identity>(gone).is_none());
    assert!(components.get::<Physical>(gone).is_none());

    // Removing again, or removing an entity that had nothing, reports
    // nothing.
    assert!(components.remove_entity(gone).is_empty());
    assert!(components.remove_entity(EntityId::new(99, 0)).is_empty());

    let partly = components.remove_entity(bare);
    assert_eq!(
        partly.kinds(),
        ComponentKinds::of(&[ComponentKind::Identity])
    );
}

#[test]
fn components_are_reached_by_type_and_by_kind() {
    let mut components = Components::new();
    let id = EntityId::new(1, 0);
    assert_eq!(components.insert(id, Identity::new(None)), None);
    components
        .get_mut::<Identity>(id)
        .expect("just inserted")
        .rename(Some("Ash".to_owned()));
    assert_eq!(
        components.identity.get(id).and_then(Identity::name),
        Some("Ash")
    );
    assert_eq!(components.store::<Identity>().len(), 1);
    assert_eq!(components.owners_of(ComponentKind::Identity), &[id]);
    assert!(components.store_mut::<Physical>().is_empty());
    assert!(components.remove::<Identity>(id).is_some());
    assert!(!components.has(ComponentKind::Identity, id));
}

#[test]
fn all_stores_save_load_and_hash_together() {
    let mut components = Components::new();
    for slot in 0..6 {
        components.insert_bundle(EntityId::new(slot, slot % 2), bundle(slot));
    }
    components.remove_entity(EntityId::new(1, 1));
    components.physical.remove(EntityId::new(4, 0));

    let loaded: Components = saved_and_loaded(&components);
    assert_eq!(loaded, components);
    assert_eq!(Hash128::of(&loaded), Hash128::of(&components));
    for &kind in ComponentKind::ALL {
        assert_eq!(loaded.hash_of(kind), components.hash_of(kind));
        assert_eq!(loaded.owners_of(kind), components.owners_of(kind));
    }
    assert_ne!(
        components.hash_of(ComponentKind::Identity),
        components.hash_of(ComponentKind::Physical)
    );

    let json = serde_json::to_string(&components).expect("state writes as JSON");
    let from_json: Components = serde_json::from_str(&json).expect("and reads back");
    assert_eq!(from_json, components);

    // A change in one store shows in that store's hash and in the whole.
    let mut changed = components.clone();
    changed
        .identity
        .get_mut(EntityId::new(0, 0))
        .expect("held")
        .rename(None);
    assert_ne!(Hash128::of(&changed), Hash128::of(&components));
    assert_ne!(
        changed.hash_of(ComponentKind::Identity),
        components.hash_of(ComponentKind::Identity)
    );
    assert_eq!(
        changed.hash_of(ComponentKind::Physical),
        components.hash_of(ComponentKind::Physical)
    );

    // A stored entity's bundle is state too.
    let stored = bundle(9);
    assert_eq!(saved_and_loaded(&stored), stored);
    assert_ne!(Hash128::of(&stored), Hash128::of(&bundle(8)));
}

#[test]
fn orphans_are_components_without_a_living_owner() {
    let mut components = Components::new();
    let alive = EntityId::new(1, 0);
    let dead = EntityId::new(2, 0);
    components.insert_bundle(alive, bundle(1));
    components.insert_bundle(dead, bundle(2));
    assert!(components.orphans(|_| true).is_empty());
    assert_eq!(
        components.orphans(|id| id == alive),
        [
            (ComponentKind::Identity, dead),
            (ComponentKind::Physical, dead),
            (ComponentKind::Stats, dead),
            (ComponentKind::Body, dead),
            (ComponentKind::Item, dead),
            (ComponentKind::Holds, dead),
            (ComponentKind::Mover, dead),
            (ComponentKind::Mind, dead),
            (ComponentKind::Driver, dead),
            (ComponentKind::Senses, dead),
            (ComponentKind::Signature, dead),
            (ComponentKind::Awareness, dead),
            (ComponentKind::Beliefs, dead),
            (ComponentKind::MapMemory, dead),
            (ComponentKind::MindState, dead)
        ]
    );
}

// ---- Identity ----------------------------------------------------------------

#[test]
fn identity_keeps_what_a_thing_is() {
    let def = DefKey::new(DefTypeId::new(4), 17);
    let mut identity = Identity::new(Some(def));
    assert_eq!(identity.def(), Some(def));
    assert_eq!(clash_expr::DefRef::from(def).index, 17);
    assert_eq!(DefKey::from(clash_expr::DefRef::from(def)), def);
    assert_eq!(identity.name(), None, "it goes by its definition's name");
    assert_eq!(identity.description(), None);

    assert_eq!(identity.rename(Some("Maren".to_owned())), None);
    assert_eq!(
        identity
            .rename(Some("Maren the Tall".to_owned()))
            .as_deref(),
        Some("Maren")
    );
    assert_eq!(identity.set_description(Some("A guard.".to_owned())), None);
    identity.apply_template(TemplateId::new(3));
    identity.apply_template(TemplateId::new(1));
    assert_eq!(
        identity.templates(),
        &[TemplateId::new(3), TemplateId::new(1)]
    );
    assert!(identity.has_template(TemplateId::new(1)));
    assert!(!identity.has_template(TemplateId::new(2)));
    let other = DefKey::new(DefTypeId::new(4), 18);
    assert_eq!(identity.set_def(Some(other)), Some(def));

    let loaded: Identity = saved_and_loaded(&identity);
    assert_eq!(loaded, identity);
    assert_eq!(Hash128::of(&loaded), Hash128::of(&identity));
    assert_ne!(
        Hash128::of(&identity),
        Hash128::of(&Identity::new(Some(other)))
    );
    assert_eq!(identity.rename(None).as_deref(), Some("Maren the Tall"));
    assert_ne!(Hash128::of(&loaded), Hash128::of(&identity));
}

// ---- Physical ----------------------------------------------------------------

/// The containment relation for tests: who is directly inside whom.
#[derive(Clone, Debug, Default)]
struct Places {
    inside: BTreeMap<EntityId, EntityId>,
    /// Lists contents last first, to show the order does not matter.
    backwards: bool,
}

impl Places {
    fn put(&mut self, what: EntityId, into: Option<EntityId>) -> Option<EntityId> {
        match into {
            Some(container) => self.inside.insert(what, container),
            None => self.inside.remove(&what),
        }
    }

    /// Whether putting `what` into `into` would make a thing hold itself.
    fn would_loop(&self, what: EntityId, into: EntityId) -> bool {
        let mut at = Some(into);
        while let Some(entity) = at {
            if entity == what {
                return true;
            }
            at = self.inside.get(&entity).copied();
        }
        false
    }
}

impl Containment for Places {
    fn container(&self, id: EntityId) -> Option<EntityId> {
        self.inside.get(&id).copied()
    }

    fn contents(&self, id: EntityId, visit: &mut dyn FnMut(EntityId)) {
        let held = self
            .inside
            .iter()
            .filter(|(_, container)| **container == id)
            .map(|(what, _)| *what);
        if self.backwards {
            held.rev().for_each(visit);
        } else {
            held.for_each(visit);
        }
    }
}

/// The reference for masses: own masses and nothing cached, summed with
/// plain floats.
fn naive_mass(own: &BTreeMap<EntityId, f64>, places: &Places, id: EntityId) -> f64 {
    let mut carried = 0.0;
    for (&what, &container) in &places.inside {
        if container == id {
            carried += naive_mass(own, places, what);
        }
    }
    match own.get(&id) {
        Some(mass) => mass + carried,
        None => carried,
    }
}

fn entity(index: usize) -> EntityId {
    EntityId::new(index as u32, 0)
}

/// How many entities the mass scripts play with.
const CAST: usize = 9;

/// Masses whose sums depend on the order of adding, so that a cache kept
/// by adding and taking away differences would drift.
const MASSES: [f64; 9] = [0.0, 0.1, 0.2, 0.3, 1.0e-9, 0.7, 70.0, 1.0e12, 3.3];

#[derive(Clone, Copy, Debug)]
enum MassOp {
    /// Give an entity a shape, making its `Physical` if it has none.
    Shape(usize, usize),
    /// Change an entity's own mass.
    OwnMass(usize, usize),
    /// Put an entity inside another, or out into the world.
    Move(usize, Option<usize>),
    /// Destroy an entity: what it held is left where it was.
    Destroy(usize),
    /// Take an entity's `Physical` away while it stays where it is.
    Strip(usize),
    /// List contents the other way round from now on.
    Flip,
}

fn mass_op() -> impl Strategy<Value = MassOp> {
    let who = || 0..CAST;
    let mass = || 0..MASSES.len();
    prop_oneof![
        5 => (who(), mass()).prop_map(|(who, mass)| MassOp::Shape(who, mass)),
        4 => (who(), mass()).prop_map(|(who, mass)| MassOp::OwnMass(who, mass)),
        8 => (who(), prop::option::weighted(0.8, who())).prop_map(|(who, into)| MassOp::Move(who, into)),
        1 => who().prop_map(MassOp::Destroy),
        1 => who().prop_map(MassOp::Strip),
        1 => Just(MassOp::Flip),
    ]
}

/// The cache against the module's own recomputation and against the
/// naive reference, for every entity.
fn assert_masses(store: &ComponentStore<Physical>, places: &Places, own: &BTreeMap<EntityId, f64>) {
    let mismatches = physical::mismatches(store, places);
    assert!(
        mismatches.is_empty(),
        "cache and recomputation differ for {mismatches:?}"
    );
    assert_eq!(store.len(), own.len());
    for (id, held) in store.iter() {
        let expected = naive_mass(own, places, id);
        assert_eq!(
            held.mass().get().get().to_bits(),
            expected.to_bits(),
            "mass of {id}"
        );
        assert_eq!(held.own_mass().get().get().to_bits(), own[&id].to_bits());
        assert_eq!(
            held.mass().get().get().to_bits(),
            (held.own_mass().get() + held.carried_mass().get())
                .get()
                .to_bits()
        );
        assert_eq!(physical::recompute(store, places, id), Ok(held.mass()));
    }
}

proptest! {
    /// After any sequence of changes to shapes and to who holds whom,
    /// every cached mass is bit for bit the from-scratch sum (E1, second
    /// item; 06 invariant B3), and each call reports exactly the masses
    /// it changed.
    #[test]
    fn physical_equals_recomputation(ops in prop::collection::vec(mass_op(), 0..90)) {
        let mut store: ComponentStore<Physical> = ComponentStore::new();
        let mut places = Places::default();
        let mut own: BTreeMap<EntityId, f64> = BTreeMap::new();

        for op in ops {
            let before: Vec<f64> =
                (0..CAST).map(|index| naive_mass(&own, &places, entity(index))).collect();
            let mut reported = None;
            match op {
                MassOp::Shape(who, mass) => {
                    let given = shape(MASSES[mass]);
                    let changes = physical::set_shape(&mut store, &places, entity(who), given)
                        .expect("a valid shape in a sound relation");
                    own.insert(entity(who), MASSES[mass]);
                    reported = Some(changes);
                }
                MassOp::OwnMass(who, mass) => {
                    let result = physical::set_own_mass(
                        &mut store,
                        &places,
                        entity(who),
                        Mass::new(Real::of(MASSES[mass])),
                    );
                    match own.get_mut(&entity(who)) {
                        Some(own_mass) => {
                            *own_mass = MASSES[mass];
                            reported = Some(result.expect("it has a physical component"));
                        }
                        None => {
                            let refused = PhysicalError::NoComponent { entity: entity(who) };
                            prop_assert_eq!(result, Err(refused));
                        }
                    }
                }
                MassOp::Move(who, into) => {
                    let into = into.map(entity);
                    if into.is_none_or(|into| !places.would_loop(entity(who), into)) {
                        let from = places.put(entity(who), into);
                        // What the kernel's relocation does.
                        for container in [from, into].into_iter().flatten() {
                            physical::refresh(&mut store, &places, container).expect("a sound relation");
                        }
                    }
                }
                MassOp::Destroy(who) => {
                    let container = places.put(entity(who), None);
                    let held: Vec<EntityId> = places
                        .inside
                        .iter()
                        .filter(|(_, inside)| **inside == entity(who))
                        .map(|(what, _)| *what)
                        .collect();
                    for what in held {
                        places.put(what, container);
                    }
                    store.remove(entity(who));
                    own.remove(&entity(who));
                    if let Some(container) = container {
                        physical::refresh(&mut store, &places, container).expect("a sound relation");
                    }
                }
                MassOp::Strip(who) => {
                    store.remove(entity(who));
                    own.remove(&entity(who));
                    physical::refresh(&mut store, &places, entity(who)).expect("a sound relation");
                }
                MassOp::Flip => places.backwards = !places.backwards,
            }
            assert_masses(&store, &places, &own);

            if let Some(changes) = reported {
                let mut expected = Vec::new();
                for (index, &before) in before.iter().enumerate() {
                    let after = naive_mass(&own, &places, entity(index));
                    if store.contains(entity(index)) && before != after {
                        expected.push((entity(index), before.to_bits(), after.to_bits()));
                    }
                }
                let mut got: Vec<(EntityId, u64, u64)> = changes
                    .iter()
                    .map(|change| {
                        (
                            change.entity,
                            change.before.get().get().to_bits(),
                            change.after.get().get().to_bits(),
                        )
                    })
                    .collect();
                got.sort_unstable();
                prop_assert_eq!(got, expected);
            }
        }

        // The cache is state: it is saved, and a loaded store agrees.
        let loaded: ComponentStore<Physical> = saved_and_loaded(&store);
        prop_assert_eq!(&loaded, &store);
        prop_assert_eq!(Hash128::of(&loaded), Hash128::of(&store));
        assert_masses(&loaded, &places, &own);
    }
}

#[test]
fn mass_is_own_mass_plus_everything_held() {
    let (rider, pack, purse, coin) = (entity(0), entity(1), entity(2), entity(3));
    let mut store = ComponentStore::new();
    let mut places = Places::default();
    let kg = |mass: f64| Mass::new(Real::of(mass));

    for (id, mass) in [(rider, 70.0), (pack, 2.0), (purse, 0.25), (coin, 0.125)] {
        let changes = physical::set_shape(&mut store, &places, id, shape(mass)).expect("valid");
        assert_eq!(changes.len(), 1);
        assert_eq!(
            (changes[0].entity, changes[0].before, changes[0].after),
            (id, kg(0.0), kg(mass))
        );
    }

    places.put(coin, Some(purse));
    physical::refresh(&mut store, &places, purse).expect("sound");
    places.put(purse, Some(pack));
    physical::refresh(&mut store, &places, pack).expect("sound");
    places.put(pack, Some(rider));
    let changes = physical::refresh(&mut store, &places, rider).expect("sound");
    assert_eq!(changes.len(), 1);
    assert_eq!(
        (changes[0].before, changes[0].after),
        (kg(70.0), kg(72.375))
    );

    let held = |store: &ComponentStore<Physical>, id| *store.get(id).expect("physical");
    assert_eq!(held(&store, rider).mass(), kg(72.375));
    assert_eq!(held(&store, rider).own_mass(), kg(70.0));
    assert_eq!(held(&store, rider).carried_mass(), kg(2.375));
    assert_eq!(held(&store, pack).mass(), kg(2.375));
    assert_eq!(held(&store, coin).carried_mass(), kg(0.0));

    // A change deep inside reaches every container, innermost first.
    let changes = physical::set_own_mass(&mut store, &places, coin, kg(0.5)).expect("held");
    let entities: Vec<EntityId> = changes.iter().map(|change| change.entity).collect();
    assert_eq!(entities, [coin, purse, pack, rider]);
    assert_eq!(changes[3].after, kg(72.75));

    // A change of size alone changes no mass.
    let taller = Shape {
        height: Length::new(Real::of(2.0)),
        ..held(&store, rider).shape()
    };
    let changes = physical::set_shape(&mut store, &places, rider, taller).expect("valid");
    assert!(changes.is_empty());
    let rider_now = held(&store, rider);
    assert_eq!(rider_now.height(), Length::new(Real::of(2.0)));
    assert_eq!(rider_now.mass(), kg(72.75));
    assert_eq!(rider_now.volume(), Volume::new(Real::of(0.07)));
    assert_eq!(rider_now.footprint(), Footprint::SINGLE);
    assert_eq!(rider_now.cross_section(), Real::of(0.6));
}

#[test]
fn a_thing_without_physical_passes_on_what_it_holds() {
    let (cart, idea, sack) = (entity(0), entity(1), entity(2));
    let mut store = ComponentStore::new();
    let mut places = Places::default();
    physical::set_shape(&mut store, &places, cart, shape(100.0)).expect("valid");
    places.put(idea, Some(cart));
    places.put(sack, Some(idea));

    // The sack is given its shape while inside something weightless.
    let changes = physical::set_shape(&mut store, &places, sack, shape(8.0)).expect("valid");
    let entities: Vec<EntityId> = changes.iter().map(|change| change.entity).collect();
    assert_eq!(entities, [sack, cart]);
    assert_eq!(
        store.get(cart).expect("physical").mass(),
        Mass::new(Real::of(108.0))
    );
    assert_eq!(
        physical::recompute(&store, &places, idea),
        Ok(Mass::new(Real::of(8.0)))
    );
    assert!(!store.contains(idea));

    // Given a shape itself, it reports the mass it already passed on.
    let changes = physical::set_shape(&mut store, &places, idea, shape(1.0)).expect("valid");
    assert_eq!(changes[0].before, Mass::new(Real::of(8.0)));
    assert_eq!(changes[0].after, Mass::new(Real::of(9.0)));
    assert_eq!(
        store.get(cart).expect("physical").mass(),
        Mass::new(Real::of(109.0))
    );
}

#[test]
fn bad_measures_are_refused_with_a_reason() {
    let mut store = ComponentStore::new();
    let places = Places::default();
    let id = entity(0);
    physical::set_shape(&mut store, &places, id, shape(5.0)).expect("valid");
    let before = store.clone();

    for bad in [-1.0, f64::INFINITY, f64::NEG_INFINITY] {
        let error = physical::set_own_mass(&mut store, &places, id, Mass::new(Real::of(bad)))
            .expect_err("refused");
        assert_eq!(
            error,
            PhysicalError::BadMeasure {
                measure: "mass",
                value: Real::of(bad)
            }
        );
        assert!(error.to_string().contains("mass"), "{error}");
        assert_eq!(store, before, "a refusal changes nothing");
    }
    let flat = Shape {
        volume: Volume::new(Real::of(-0.5)),
        ..shape(1.0)
    };
    assert!(matches!(
        physical::set_shape(&mut store, &places, entity(1), flat),
        Err(PhysicalError::BadMeasure {
            measure: "volume",
            ..
        })
    ));
    assert!(!store.contains(entity(1)));
    assert!(Physical::new(flat).is_err());

    let held = store.get_mut(id).expect("physical");
    assert!(held.set_height(Length::new(Real::of(-2.0))).is_err());
    assert!(held.set_volume(Volume::new(Real::INFINITY)).is_err());
    assert!(held.set_cross_section(Real::of(-0.1)).is_err());
    assert_eq!(store, before);

    let held = store.get_mut(id).expect("physical");
    held.set_height(Length::new(Real::of(0.4))).expect("valid");
    held.set_volume(Volume::new(Real::of(0.01))).expect("valid");
    held.set_cross_section(Real::of(0.2)).expect("valid");
    held.set_footprint(Footprint::new(2, 1, 1));
    assert_eq!(held.shape().footprint, Footprint::new(2, 1, 1));
    assert_eq!(
        held.mass(),
        Mass::new(Real::of(5.0)),
        "size does not change mass"
    );

    assert_eq!(
        physical::set_own_mass(&mut store, &places, entity(7), Mass::new(Real::ONE)),
        Err(PhysicalError::NoComponent { entity: entity(7) })
    );
}

#[test]
fn a_thing_that_holds_itself_is_refused_and_nothing_changes() {
    let (a, b, c) = (entity(0), entity(1), entity(2));
    let mut store = ComponentStore::new();
    let mut places = Places::default();
    for id in [a, b] {
        physical::set_shape(&mut store, &places, id, shape(1.0)).expect("valid");
    }
    // A relation no relocation would make.
    places.put(a, Some(b));
    places.put(b, Some(a));
    places.put(c, Some(a));
    let before = store.clone();

    assert!(matches!(
        physical::refresh(&mut store, &places, a),
        Err(PhysicalError::TooDeep { .. })
    ));
    assert!(matches!(
        physical::set_own_mass(&mut store, &places, a, Mass::new(Real::of(2.0))),
        Err(PhysicalError::TooDeep { .. })
    ));
    assert!(matches!(
        physical::set_shape(&mut store, &places, c, shape(3.0)),
        Err(PhysicalError::TooDeep { .. })
    ));
    assert!(matches!(
        physical::recompute(&store, &places, a),
        Err(PhysicalError::TooDeep { .. })
    ));
    assert!(matches!(
        physical::rebuild(&mut store, &places),
        Err(PhysicalError::TooDeep { .. })
    ));
    assert_eq!(store, before, "a refusal changes nothing");
    assert!(!store.contains(c));
    assert_eq!(physical::mismatches(&store, &places), [a, b]);
}

#[test]
fn containment_may_be_as_deep_as_the_limit_and_no_deeper() {
    let mut store = ComponentStore::new();
    let mut places = Places::default();
    // A chain: each entity inside the next.
    for index in 0..=MAX_CONTAINMENT_DEPTH + 1 {
        physical::set_shape(&mut store, &places, entity(index), shape(1.0)).expect("valid");
    }
    for index in 0..MAX_CONTAINMENT_DEPTH {
        places.put(entity(index), Some(entity(index + 1)));
    }
    // The innermost is exactly the limit deep.
    physical::rebuild(&mut store, &places).expect("within the limit");
    let outermost = store.get(entity(MAX_CONTAINMENT_DEPTH)).expect("physical");
    assert_eq!(
        outermost.mass(),
        Mass::new(Real::of(MAX_CONTAINMENT_DEPTH as f64 + 1.0))
    );
    physical::set_own_mass(&mut store, &places, entity(0), Mass::new(Real::of(2.0)))
        .expect("within the limit");
    assert!(physical::mismatches(&store, &places).is_empty());

    // One more level is refused.
    places.put(
        entity(MAX_CONTAINMENT_DEPTH),
        Some(entity(MAX_CONTAINMENT_DEPTH + 1)),
    );
    assert!(matches!(
        physical::refresh(&mut store, &places, entity(0)),
        Err(PhysicalError::TooDeep { .. })
    ));
}

#[test]
fn rebuild_repairs_a_store_put_together_by_hand() {
    let (pack, tin) = (entity(0), entity(1));
    let mut components = Components::new();
    let mut places = Places::default();
    // Inserted directly, as a returning bundle is: nobody told the pack.
    components.insert(pack, Physical::new(shape(2.0)).expect("valid"));
    components.insert(tin, Physical::new(shape(0.4)).expect("valid"));
    places.put(tin, Some(pack));
    assert_eq!(physical::mismatches(&components.physical, &places), [pack]);

    let mut refreshed = components.clone();
    physical::refresh(&mut refreshed.physical, &places, tin).expect("sound");
    physical::rebuild(&mut components.physical, &places).expect("sound");
    assert!(physical::mismatches(&components.physical, &places).is_empty());
    assert_eq!(components, refreshed);
    assert_eq!(
        components.physical.get(pack).expect("physical").mass(),
        Mass::new(Real::of(2.4))
    );
}
