//! Tests for package K1: the simulation skeleton, kernel entities,
//! places with containment, `relocate`, creation and removal, the state
//! hash, saving, loading and `reindex`, and what events carry.

mod common;

use clash_core::{Dur, EntityId, Tick};
use clash_sim::{
    Command, DebugCommand, EventBody, MAX_CONTAINMENT_DEPTH, Place, Presence, Sim, Subject, codes,
};
use clash_time::ClockId;
use clash_world::{
    Cause, CauseKind, Footprint, Layer, OccupantFilter, OccupantKind, WorldEvent, distinct_ids,
};
use common::{SIDE, at, creature, item, on, spawn};
use proptest::prelude::*;

/// What must hold of the occupant index after anything `relocate` does:
/// an entity is in the index exactly when it is `At` a cell, at that
/// cell and findable there; nothing else is in the index; and every
/// container's list of contents is what the places say.
fn assert_index_consistent(sim: &Sim) {
    let truth = sim.truth();
    let (entities, cosmos) = (truth.entities(), truth.cosmos());
    let mut in_world = 0;
    for entity in entities.iter() {
        let id = entity.id();
        match entity.place() {
            Place::At { loc, .. } => {
                in_world += 1;
                let record = cosmos
                    .occupant(id)
                    .unwrap_or_else(|| panic!("{id} is at {loc} but not in the index"));
                assert_eq!(record.loc, loc);
                let here = cosmos.occupants_at(loc, &OccupantFilter::ANY).unwrap();
                assert!(distinct_ids(&here).contains(&id));
                assert_eq!(truth.location(id), Some(loc));
            }
            Place::In(container) => {
                assert!(
                    cosmos.occupant(id).is_none(),
                    "{id} is contained and indexed"
                );
                assert!(entities.contents(container).contains(&id));
                // It is wherever its container is.
                assert_eq!(truth.location(id), truth.location(container));
            }
            Place::Stored => {
                assert!(cosmos.occupant(id).is_none(), "{id} is stored and indexed");
                assert_eq!(truth.location(id), None);
            }
        }
        for inside in entities.contents(id) {
            assert_eq!(truth.place(*inside), Some(Place::In(id)));
        }
        assert!(entities.depth(id).unwrap() <= MAX_CONTAINMENT_DEPTH);
    }
    assert_eq!(cosmos.occupant_count(), in_world);
}

/// One step of a random script against `relocate` and its neighbours.
#[derive(Clone, Debug)]
enum Op {
    Create { x: i32, y: i32, item: bool },
    Move { who: usize, x: i32, y: i32 },
    Put { who: usize, into: usize },
    Store { who: usize },
    Remove { who: usize },
    Wall { x: i32, y: i32, up: bool },
}

fn op() -> impl Strategy<Value = Op> {
    // A few cells beyond the loaded ground, so that some moves are
    // refused.
    let cell = -2..SIDE + 2;
    prop_oneof![
        2 => (cell.clone(), cell.clone(), any::<bool>())
            .prop_map(|(x, y, item)| Op::Create { x, y, item }),
        6 => (any::<usize>(), cell.clone(), cell.clone())
            .prop_map(|(who, x, y)| Op::Move { who, x, y }),
        4 => (any::<usize>(), any::<usize>()).prop_map(|(who, into)| Op::Put { who, into }),
        1 => any::<usize>().prop_map(|who| Op::Store { who }),
        1 => any::<usize>().prop_map(|who| Op::Remove { who }),
        2 => (cell.clone(), cell, any::<bool>()).prop_map(|(x, y, up)| Op::Wall { x, y, up }),
    ]
}

/// Applies a step through the simulation's own functions. Refusals are
/// part of the test: they must change nothing.
fn apply(sim: &mut Sim, op: &Op) {
    let ids: Vec<EntityId> = sim.truth().entities().ids().collect();
    let pick = |index: usize| (!ids.is_empty()).then(|| ids[index % ids.len()]);
    // Events waiting to be delivered are state: settle them first, so
    // that a refusal can be seen to change nothing.
    sim.pump();
    let before = sim.state_hash();
    let refused = match op {
        Op::Create {
            x,
            y,
            item: is_item,
        } => {
            let spec = if *is_item { item() } else { creature() };
            sim.spawn(spec, on(*x, *y), Cause::DEBUG).is_err()
        }
        Op::Move { who, x, y } => match pick(*who) {
            Some(id) => sim.relocate(id, on(*x, *y), Cause::DEBUG).is_err(),
            None => false,
        },
        Op::Put { who, into } => match (pick(*who), pick(*into)) {
            (Some(id), Some(container)) => sim
                .relocate(id, Place::In(container), Cause::DEBUG)
                .is_err(),
            _ => false,
        },
        Op::Store { who } => match pick(*who) {
            Some(id) => sim.relocate(id, Place::Stored, Cause::DEBUG).is_err(),
            None => false,
        },
        Op::Remove { who } => match pick(*who) {
            Some(id) => sim.remove_entity(id, Cause::DEBUG).is_err(),
            None => false,
        },
        Op::Wall { x, y, up } => {
            let state = if *up {
                common::wall(sim)
            } else {
                clash_world::BlockState::NONE
            };
            sim.world(|ctx| ctx.edit(Cause::DEBUG).set_block(at(*x, *y), state))
                .is_err()
        }
    };
    if refused {
        assert_eq!(
            sim.state_hash(),
            before,
            "a refusal changed the state: {op:?}"
        );
    }
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(48))]

    /// K1: `relocate` keeps the world's occupant index consistent under
    /// random moves, containment changes and removals, and under the
    /// world moving occupants out of the way of edits.
    #[test]
    fn relocate_keeps_the_occupant_index_consistent(ops in prop::collection::vec(op(), 1..60)) {
        let mut sim = common::sim(11);
        for (x, y) in [(3, 3), (4, 3), (10, 12)] {
            spawn(&mut sim, creature(), on(x, y));
        }
        assert_index_consistent(&sim);
        for op in &ops {
            apply(&mut sim, op);
            assert_index_consistent(&sim);
        }
        // And the index can be thrown away and rebuilt from the places.
        let hash = sim.state_hash();
        prop_assert!(sim.reindex().is_empty());
        assert_index_consistent(&sim);
        prop_assert_eq!(sim.state_hash(), hash);
    }
}

/// K1: a contained entity is absent from the index and reports its
/// container's location, however deeply it is nested and wherever the
/// container goes.
#[test]
fn a_contained_entity_is_absent_from_the_index_and_is_where_its_container_is() {
    let mut sim = common::sim(1);
    let mount = spawn(&mut sim, creature(), on(5, 5));
    let rider = spawn(&mut sim, creature(), on(6, 5));
    let coin = spawn(&mut sim, item(), on(7, 5));
    sim.relocate(coin, Place::In(rider), Cause::DEBUG).unwrap();
    sim.relocate(rider, Place::In(mount), Cause::DEBUG).unwrap();

    let truth = sim.truth();
    assert!(truth.cosmos().occupant(coin).is_none());
    assert!(truth.cosmos().occupant(rider).is_none());
    assert_eq!(truth.cosmos().occupant_count(), 1);
    assert_eq!(truth.location(coin), Some(at(5, 5)));
    assert_eq!(truth.entities().outermost(coin), Some(mount));
    assert_eq!(truth.contents(mount), [rider]);
    assert_eq!(truth.entities().contents_deep(mount), [rider, coin]);

    // The container moves; what it holds is wherever it is.
    sim.relocate(mount, on(20, 9), Cause::DEBUG).unwrap();
    assert_eq!(sim.truth().location(coin), Some(at(20, 9)));
    assert_eq!(sim.truth().place(coin), Some(Place::In(rider)));

    // Taken out, it is in the world itself again.
    sim.relocate(coin, on(21, 9), Cause::DEBUG).unwrap();
    assert_eq!(sim.truth().cosmos().occupant(coin).unwrap().loc, at(21, 9));
    assert_index_consistent(&sim);
}

/// K1: nothing outside `relocate` can change `place`.
///
/// This is a privacy argument, checked by the compiler. `place` is a
/// private field of `KernelEntity`, declared in `clash_sim::entity`, so
/// only that module and its one child, `entity::relocate`, can name it;
/// the parent writes it once, as `Stored`, when an entity is made, and
/// every other write is in `relocate.rs`. The two `compile_fail`
/// examples in the documentation of `clash_sim::entity` show that code
/// outside can neither assign the field nor make a second record to
/// overwrite one with. What is left to check here is that what the rest
/// of the crate offers does not move anything.
#[test]
fn nothing_but_relocate_changes_a_place() {
    let mut sim = common::sim(2);
    let guard = spawn(&mut sim, creature(), on(8, 8));
    let place = sim.truth().place(guard);

    // Everything that can be changed through `entity_mut` is elsewhere.
    let entity = sim.entity_mut(guard).unwrap();
    entity.tags.insert("alert");
    entity.vars.set("core:kills", 3);
    assert_eq!(sim.truth().place(guard), place);

    // Changing what the world is told about it does not move it.
    let wider = Presence::new(OccupantKind::Creature).with_footprint(Footprint::new(2, 1, 1));
    sim.set_presence(guard, wider, Cause::DEBUG).unwrap();
    assert_eq!(sim.truth().place(guard), place);
    assert_eq!(
        sim.truth().cosmos().occupant(guard).unwrap().footprint,
        Footprint::new(2, 1, 1)
    );
    // A footprint that does not fit where it stands is refused.
    let hash = sim.state_hash();
    let huge = Presence::new(OccupantKind::Creature).with_footprint(Footprint::new(16, 16, 1));
    let edge = spawn(&mut sim, creature(), on(SIDE - 3, SIDE - 3));
    sim.pump();
    let before = sim.state_hash();
    assert_ne!(before, hash);
    let refused = sim.set_presence(edge, huge, Cause::DEBUG).unwrap_err();
    assert_eq!(refused.code, codes::WORLD_NOT_LOADED);
    assert_eq!(sim.state_hash(), before);
    // A refused move leaves it where it was.
    let refused = sim
        .relocate(guard, on(SIDE + 40, 0), Cause::DEBUG)
        .unwrap_err();
    assert_eq!(refused.code, codes::WORLD_NOT_LOADED);
    assert_eq!(sim.truth().place(guard), place);
}

#[test]
fn containment_refuses_loops_and_the_unknown_with_reasons() {
    let mut sim = common::sim(3);
    let pack = spawn(&mut sim, item(), on(2, 2));
    let purse = spawn(&mut sim, item(), Place::In(pack));
    let coin = spawn(&mut sim, item(), Place::In(purse));
    let hash = sim.state_hash();

    let own = sim
        .relocate(pack, Place::In(pack), Cause::DEBUG)
        .unwrap_err();
    assert_eq!(own.code, codes::RELOCATE_INSIDE_ITSELF);
    let looped = sim
        .relocate(pack, Place::In(coin), Cause::DEBUG)
        .unwrap_err();
    assert_eq!(looped.code, codes::RELOCATE_INSIDE_ITSELF);
    let ghost = EntityId::new(99, 0);
    let unknown = sim
        .relocate(coin, Place::In(ghost), Cause::DEBUG)
        .unwrap_err();
    assert_eq!(unknown.code, codes::ENTITY_UNKNOWN);
    assert_eq!(
        sim.relocate(ghost, Place::Stored, Cause::DEBUG)
            .unwrap_err()
            .code,
        codes::ENTITY_UNKNOWN
    );
    assert_eq!(sim.state_hash(), hash, "refusals change nothing");
    assert!(!own.message.is_empty());

    // Nesting is bounded.
    let mut inner = coin;
    let mut deepest = None;
    for _ in 0..MAX_CONTAINMENT_DEPTH + 2 {
        match sim.spawn(item(), Place::In(inner), Cause::DEBUG) {
            Ok(id) => inner = id,
            Err(reason) => {
                deepest = Some(reason);
                break;
            }
        }
    }
    assert_eq!(deepest.unwrap().code, codes::RELOCATE_TOO_DEEP);
}

/// Creation and removal are events a component store can follow, in
/// order, and removal leaves nothing behind (invariant T9).
#[test]
fn creation_and_removal_are_reported_and_removal_leaves_nothing_behind() {
    let mut sim = common::walking_sim(4);
    let chest = spawn(&mut sim, item(), on(4, 4));
    let gem = spawn(&mut sim, item(), Place::In(chest));
    let wake = sim
        .schedule_for(chest, Dur::from_secs(5), common::walk_wake(chest, 0))
        .unwrap();
    sim.set_time_rate(chest, Some(clash_core::Real::of(2.0)), Cause::DEBUG)
        .unwrap();
    let clock = sim.truth().entity(chest).unwrap().clock().unwrap();
    sim.pump();
    let made = sim.drain_events();
    assert_eq!(
        common::names(&made),
        [
            "entity_created",
            "entity_relocated",
            "entity_created",
            "entity_relocated",
            "clock_rate_changed"
        ]
    );
    assert_eq!(common::entity_of(&made[0], "self"), Some(chest));
    assert_eq!(common::entity_of(&made[2], "self"), Some(gem));

    sim.remove_entity(chest, Cause::DEBUG).unwrap();
    sim.pump();
    let gone = sim.drain_events();
    // What it held is put where it was, then it leaves and is removed.
    assert_eq!(
        common::names(&gone),
        ["entity_relocated", "entity_relocated", "entity_removed"]
    );
    assert_eq!(sim.truth().place(gem), Some(on(4, 4)));
    assert!(sim.truth().entity(chest).is_none());
    assert!(sim.truth().cosmos().occupant(chest).is_none());
    assert!(
        sim.timeline().wake(wake).is_none(),
        "its wake-up is cancelled"
    );
    assert!(!sim.timeline().has_clock(clock), "its clock is destroyed");
    assert_eq!(sim.timeline().wake_count(), 0);

    // The slot is reused a generation on, and the old id stays dead.
    let next = spawn(&mut sim, item(), Place::Stored);
    assert_eq!((next.slot(), next.generation()), (chest.slot(), 1));
    assert!(sim.truth().entity(chest).is_none());
    assert_eq!(
        sim.remove_entity(chest, Cause::DEBUG).unwrap_err().code,
        codes::ENTITY_UNKNOWN
    );
}

/// An entity that leaves the world has no queued wake-up (T9), and
/// neither has anything inside it.
#[test]
fn leaving_the_world_cancels_wake_ups() {
    let mut sim = common::walking_sim(5);
    let cart = spawn(&mut sim, creature(), on(9, 9));
    let rat = spawn(&mut sim, creature(), Place::In(cart));
    let other = spawn(&mut sim, creature(), on(1, 1));
    for id in [cart, rat, other] {
        sim.schedule_for(id, Dur::from_secs(1), common::walk_wake(id, 0))
            .unwrap();
    }
    assert_eq!(sim.timeline().wake_count(), 3);
    sim.relocate(cart, Place::Stored, Cause::DEBUG).unwrap();
    assert_eq!(sim.timeline().wake_count(), 1);
    assert_eq!(sim.truth().wakes_of(other).len(), 1);
    // Nothing can be scheduled for an entity that is not in the world,
    // even one with a clock of its own.
    let refused = sim
        .schedule_for(rat, Dur::from_secs(1), common::walk_wake(rat, 0))
        .unwrap_err();
    assert_eq!(refused.code, codes::ENTITY_STORED);
    sim.set_time_rate(other, Some(clash_core::Real::of(2.0)), Cause::DEBUG)
        .unwrap();
    assert_eq!(
        sim.timeline().wake_count(),
        1,
        "its wake-up moved to its own clock"
    );
    sim.relocate(other, Place::Stored, Cause::DEBUG).unwrap();
    assert_eq!(sim.timeline().wake_count(), 0);
    assert!(sim.truth().clock_of(other).is_some());
    let refused = sim
        .schedule_for(other, Dur::from_secs(1), common::walk_wake(other, 0))
        .unwrap_err();
    assert_eq!(refused.code, codes::ENTITY_STORED);
}

/// Review addition to K2: every event carries the universal tick, the
/// local time of its clock, its subjects and its cause.
#[test]
fn every_event_carries_tick_local_time_subjects_and_cause() {
    let mut sim = common::sim(6);
    let slow = common::box_area(&mut sim, (10, 10), (14, 14), Some(0.5), 0);
    let slow_clock = sim.truth().cosmos().area(slow).unwrap().clock.unwrap();
    let thrower = spawn(&mut sim, creature(), on(2, 2));
    let rock = spawn(&mut sim, item(), on(3, 2));
    sim.advance(clash_sim::AdvanceLimit::Until(Tick(4_000_000)));
    sim.drain_events();

    let cause = Cause::new(CauseKind::Action)
        .by(rock)
        .instigated_by(thrower);
    sim.relocate(rock, on(12, 12), cause).unwrap();
    sim.pump();
    let events = sim.drain_events();
    assert_eq!(common::names(&events), ["area_entered", "entity_relocated"]);
    for event in &events {
        assert_eq!(event.tick, Tick(4_000_000));
        assert_eq!(event.cause, cause);
        // The area's clock runs at half speed and was made at tick 0.
        assert_eq!(event.clock, slow_clock);
        assert_eq!(event.local, Dur(2_000_000));
        assert_eq!(event.subject("actor"), Some(Subject::Entity(rock)));
        assert_eq!(event.subject("instigator"), Some(Subject::Entity(thrower)));
    }
    assert_eq!(events[0].subject("area"), Some(Subject::Area(slow)));
    assert_eq!(events[0].subject("target"), Some(Subject::Entity(rock)));
    assert_eq!(events[1].subject("self"), Some(Subject::Entity(rock)));
    assert_eq!(events[1].subject("here"), Some(Subject::Loc(at(12, 12))));
    assert_eq!(
        events[1].body,
        EventBody::EntityRelocated {
            entity: rock,
            from: on(3, 2),
            to: on(12, 12)
        }
    );

    // An event with no place of its own is on universal time.
    let made = spawn(&mut sim, item(), Place::Stored);
    sim.pump();
    let events = sim.drain_events();
    assert_eq!(events[0].clock, ClockId::UNIVERSAL);
    assert_eq!(events[0].local, Dur(4_000_000));
    assert_eq!(common::entity_of(&events[0], "self"), Some(made));
}

/// The world moves an occupant out of the way of an edit (02 §23); the
/// kernel follows, so that place and index still agree.
#[test]
fn an_occupant_the_world_displaces_is_followed() {
    let mut sim = common::sim(7);
    let guard = spawn(&mut sim, creature(), on(6, 6));
    sim.pump();
    sim.drain_events();
    let wall = common::wall(&sim);
    sim.submit(Command::Debug(DebugCommand::SetElement {
        loc: at(6, 6),
        layer: Layer::Block,
        state: wall.raw(),
    }))
    .unwrap();

    let place = sim.truth().place(guard).unwrap();
    assert_ne!(place, on(6, 6), "it was moved out of the wall");
    assert_eq!(
        sim.truth().cosmos().occupant(guard).unwrap().loc,
        place.loc().unwrap()
    );
    assert_index_consistent(&sim);
    let events = sim.drain_events();
    assert_eq!(
        common::names(&events),
        ["element_changed", "occupant_displaced", "entity_relocated"]
    );
    assert!(matches!(
        events[1].body,
        EventBody::World(WorldEvent::OccupantDisplaced { occupant, .. }) if occupant == guard
    ));
    assert_eq!(
        events[2].body,
        EventBody::EntityRelocated {
            entity: guard,
            from: on(6, 6),
            to: place
        }
    );
}

/// The world does not save occupants: after a save and a load the
/// simulation places its entities again, and the hash and every
/// occupant query are what they were.
#[test]
fn save_load_reindex_gives_the_same_hash_and_the_same_occupants() {
    let mut sim = common::walking_sim(8);
    common::box_area(&mut sim, (0, 0), (9, 9), Some(0.25), 5);
    let mut ids = Vec::new();
    for (x, y) in [(1, 1), (2, 7), (30, 30), (40, 3), (8, 8)] {
        ids.push(spawn(&mut sim, creature(), on(x, y)));
    }
    let wide = Presence::new(OccupantKind::Creature).with_footprint(Footprint::new(3, 2, 1));
    let cart = spawn(&mut sim, creature().with_presence(wide), on(20, 20));
    let load = spawn(&mut sim, item().with_tag("heavy"), Place::In(cart));
    spawn(&mut sim, item(), Place::Stored);
    for id in &ids {
        sim.schedule_for(*id, Dur::from_millis(300), common::walk_wake(*id, 6))
            .unwrap();
    }
    sim.set_time_rate(ids[0], Some(clash_core::Real::of(3.0)), Cause::DEBUG)
        .unwrap();
    sim.advance(clash_sim::AdvanceLimit::Until(Tick(1_000_000)));

    let (mut loaded, unplaced) = common::save_and_load(&sim);
    assert!(unplaced.is_empty(), "{unplaced:?}");
    assert_eq!(loaded.hashes(), sim.hashes());
    assert_eq!(loaded.truth().entities(), sim.truth().entities());
    assert_eq!(loaded.undrained_events(), sim.undrained_events());
    assert_eq!(loaded.truth().contents(cart), [load]);

    // The same occupants, by every kind of query.
    let (was, is) = (sim.truth().cosmos(), loaded.truth().cosmos());
    assert_eq!(is.occupant_count(), was.occupant_count());
    for record in was.occupants() {
        assert_eq!(is.occupant(record.occupant.id), Some(record));
    }
    let everything = clash_world::CellBox::new(at(0, 0).cell, at(SIDE - 1, SIDE - 1).cell);
    assert_eq!(
        is.occupants_in_box(common::GRID, everything, &OccupantFilter::ANY)
            .unwrap(),
        was.occupants_in_box(common::GRID, everything, &OccupantFilter::ANY)
            .unwrap()
    );
    assert_index_consistent(&loaded);

    // And from here on the two behave the same.
    loaded.set_wake_handler(clash_time::WakeDomain::Stepper, common::walker);
    let end = clash_sim::AdvanceLimit::Until(Tick(9_000_000));
    assert_eq!(loaded.advance(end), sim.advance(end));
    assert_eq!(loaded.state_hash(), sim.state_hash());
    assert_eq!(loaded.drain_events(), sim.drain_events());
}

/// An entity whose cell is gone when the index is rebuilt leaves the
/// world, with a reason, instead of being lost.
#[test]
fn reindex_reports_whoever_cannot_be_placed() {
    let mut sim = common::sim(9);
    let near = spawn(&mut sim, creature(), on(3, 3));
    let far = spawn(&mut sim, creature(), on(SIDE - 2, SIDE - 2));
    let blob = sim.save().unwrap();
    let mut cosmos = sim.truth().cosmos().clone();
    clash_world::test_support::unload_column(
        &mut cosmos,
        common::GRID,
        clash_world::TractPos::new(1, 1),
    )
    .unwrap();
    let mut loaded = Sim::load(std::sync::Arc::clone(sim.defs()), cosmos, &blob).unwrap();
    let unplaced = loaded.reindex();
    assert_eq!(unplaced.len(), 1);
    assert_eq!(unplaced[0].entity, far);
    assert_eq!(unplaced[0].reason.code, codes::WORLD_NOT_LOADED);
    assert_eq!(loaded.truth().place(far), Some(Place::Stored));
    assert_eq!(loaded.truth().place(near), Some(on(3, 3)));
    assert_index_consistent(&loaded);
}

#[test]
fn the_hash_tells_states_apart_and_ignores_what_is_only_a_record() {
    let mut a = common::sim(10);
    let mut b = common::sim(10);
    assert_eq!(a.state_hash(), b.state_hash());
    let id = spawn(&mut a, creature(), on(1, 1));
    assert_ne!(a.state_hash(), b.state_hash());
    assert_ne!(a.hashes().entities, b.hashes().entities);
    assert_eq!(
        a.hashes().cosmos,
        b.hashes().cosmos,
        "occupants are not world state"
    );
    spawn(&mut b, creature(), on(1, 1));
    assert_eq!(a.state_hash(), b.state_hash());

    // Draining the output changes nothing; neither does a different seed
    // until a stream is drawn from.
    a.pump();
    b.pump();
    assert!(!a.drain_events().is_empty());
    assert_eq!(a.state_hash(), b.state_hash());
    a.entity_mut(id).unwrap().tags.insert("marked");
    assert_ne!(a.state_hash(), b.state_hash());
    assert_ne!(common::sim(1).state_hash(), common::sim(2).state_hash());
}

/// A blob that is not the simulation's, or is of another layout, is
/// refused and never panics.
#[test]
fn loading_refuses_what_it_cannot_read() {
    let sim = common::sim(12);
    let cosmos = sim.truth().cosmos().clone();
    let defs = std::sync::Arc::clone(sim.defs());
    assert!(Sim::load(defs.clone(), cosmos.clone(), b"not a blob").is_err());
    let other = clash_core::codec::encode_blob(clash_sim::SAVE_KIND, 999, &0_u8).unwrap();
    assert!(matches!(
        Sim::load(defs.clone(), cosmos.clone(), &other),
        Err(clash_sim::LoadError::Version { found: 999, .. })
    ));
    let mut blob = sim.save().unwrap();
    let last = blob.len() - 1;
    blob[last] ^= 0x40;
    assert!(Sim::load(defs, cosmos, &blob).is_err());
}
