//! Tests for package T1: which clock a thing is on (03 §3.4), rehoming
//! on relocation, and the operations `set_time_rate`, `anchor_clock`
//! and `set_clock_rate`.

mod common;

use clash_core::{Dur, EntityId, Real, Tick};
use clash_sim::{Advance, AdvanceLimit, ClockTarget, EventBody, Place, Sim, codes};
use clash_time::{ClockId, WakeDomain, WakeId};
use clash_world::Cause;
use common::{at, box_area, creature, item, on, spawn, walk_wake};

const SECOND: i64 = 1_000_000;

fn secs(n: i64) -> Dur {
    Dur::from_secs(n)
}

/// Schedules a wake-up for an entity that does nothing when it fires.
fn wait(sim: &mut Sim, entity: EntityId, after: Dur) -> WakeId {
    sim.schedule_for(entity, after, walk_wake(entity, 0))
        .unwrap()
}

fn idle(_sim: &mut Sim, _id: WakeId, _wake: clash_time::Wake) {}

/// A simulation whose `Stepper` wake-ups do nothing.
fn quiet_sim(seed: u64) -> Sim {
    let mut sim = common::sim(seed);
    sim.set_wake_handler(WakeDomain::Stepper, idle);
    sim
}

fn to(sim: &mut Sim, tick: i64) {
    assert_eq!(
        sim.advance(AdvanceLimit::Until(Tick(tick))),
        Advance::Reached(Tick(tick))
    );
}

/// T1: entering and leaving an area with a clock rehomes an entity's
/// wake-ups with the remaining local time preserved.
#[test]
fn entering_and_leaving_a_clocked_area_keeps_remaining_local_time() {
    let mut sim = quiet_sim(1);
    let slow = box_area(&mut sim, (10, 10), (19, 19), Some(0.5), 0);
    let slow_clock = sim.truth().cosmos().area(slow).unwrap().clock.unwrap();
    let realm_clock = sim.location_clock(at(0, 0));
    assert_eq!(sim.location_clock(at(12, 12)), slow_clock);
    assert_ne!(realm_clock, slow_clock);

    let walker = spawn(&mut sim, creature(), on(2, 2));
    let wake = wait(&mut sim, walker, secs(10));
    assert_eq!(sim.truth().clock_of(walker), Some(realm_clock));
    assert_eq!(sim.timeline().due(wake), Some(Tick(10 * SECOND)));

    // Two seconds pass outside: eight are left.
    to(&mut sim, 2 * SECOND);
    sim.relocate(walker, on(12, 12), Cause::DEBUG).unwrap();
    assert_eq!(sim.timeline().clock_of(wake), Some(slow_clock));
    assert_eq!(sim.timeline().remaining(wake), Some(secs(8)));
    // Eight seconds of local time at half speed are sixteen of universal.
    assert_eq!(sim.timeline().due(wake), Some(Tick(18 * SECOND)));

    // Four universal seconds inside are two local: six are left.
    to(&mut sim, 6 * SECOND);
    assert_eq!(sim.timeline().remaining(wake), Some(secs(6)));
    sim.relocate(walker, on(30, 30), Cause::DEBUG).unwrap();
    assert_eq!(sim.timeline().clock_of(wake), Some(realm_clock));
    assert_eq!(sim.timeline().remaining(wake), Some(secs(6)));
    assert_eq!(sim.timeline().due(wake), Some(Tick(12 * SECOND)));

    // It fires then, and not before.
    to(&mut sim, 12 * SECOND - 1);
    assert_eq!(sim.steps(), 0);
    to(&mut sim, 12 * SECOND);
    assert_eq!(sim.steps(), 1);
    // The entity's own time: 2 s outside, 2 s inside, 6 s outside.
    assert_eq!(sim.truth().own_time(walker), secs(10));
}

/// An area that comes over an entity, or goes away from under it,
/// rehomes just as a move does; so does what the entity holds.
#[test]
fn an_area_made_moved_or_removed_over_an_entity_rehomes_it() {
    let mut sim = quiet_sim(2);
    let realm_clock = sim.location_clock(at(0, 0));
    let carrier = spawn(&mut sim, creature(), on(5, 5));
    let parcel = spawn(&mut sim, item(), Place::In(carrier));
    let own = wait(&mut sim, carrier, secs(10));
    let held = wait(&mut sim, parcel, secs(4));

    // Time stops where they stand: their wake-ups are parked.
    let stop = box_area(&mut sim, (0, 0), (9, 9), Some(0.0), 0);
    let stop_clock = sim.truth().cosmos().area(stop).unwrap().clock.unwrap();
    for wake in [own, held] {
        assert_eq!(sim.timeline().clock_of(wake), Some(stop_clock));
        assert!(sim.timeline().is_parked(wake));
    }
    assert_eq!(sim.truth().clock_of(parcel), Some(stop_clock));
    to(&mut sim, 20 * SECOND);
    assert_eq!(sim.steps(), 0, "nothing fires in stopped time (T3)");
    assert_eq!(sim.timeline().remaining(own), Some(secs(10)));

    // The area moves away: they are back on the realm's clock with the
    // local time they had left.
    sim.world(|ctx| ctx.move_area(stop, clash_core::IVec3::new(30, 0, 0), Cause::DEBUG))
        .unwrap();
    for (wake, left) in [(own, 10), (held, 4)] {
        assert_eq!(sim.timeline().clock_of(wake), Some(realm_clock));
        assert_eq!(sim.timeline().remaining(wake), Some(secs(left)));
    }
    to(&mut sim, 24 * SECOND);
    assert_eq!(sim.steps(), 1, "the parcel's wake-up fired");

    // It comes back and is then removed altogether.
    sim.world(|ctx| ctx.move_area(stop, clash_core::IVec3::new(-30, 0, 0), Cause::DEBUG))
        .unwrap();
    assert!(sim.timeline().is_parked(own));
    sim.world(|ctx| ctx.remove_area(stop, Cause::DEBUG))
        .unwrap();
    assert_eq!(sim.timeline().clock_of(own), Some(realm_clock));
    assert_eq!(sim.timeline().remaining(own), Some(secs(6)));
    assert_eq!(sim.timeline().due(own), Some(Tick(30 * SECOND)));
}

/// T1: an anchored entity is unaffected by the area it stands in.
#[test]
fn an_anchored_entity_is_unaffected_by_the_area_it_stands_in() {
    let mut sim = quiet_sim(3);
    let realm_clock = sim.location_clock(at(0, 0));
    let stop = box_area(&mut sim, (10, 10), (19, 19), Some(0.0), 0);
    let stop_clock = sim.truth().cosmos().area(stop).unwrap().clock.unwrap();
    let free = spawn(&mut sim, creature(), on(2, 2));
    let caught = spawn(&mut sim, creature(), on(3, 2));

    // Anchoring gives the entity a clock of its own at rate 1, pinned
    // under the realm's.
    sim.anchor_clock(free, Some(realm_clock), Cause::DEBUG)
        .unwrap();
    let personal = sim.truth().entity(free).unwrap().clock().unwrap();
    assert_eq!(
        sim.truth().entity(free).unwrap().anchor(),
        Some(realm_clock)
    );
    assert_eq!(sim.timeline().rate(personal), Some(Real::ONE));

    let free_wake = wait(&mut sim, free, secs(5));
    let caught_wake = wait(&mut sim, caught, secs(5));
    to(&mut sim, SECOND);
    sim.relocate(free, on(12, 12), Cause::DEBUG).unwrap();
    sim.relocate(caught, on(13, 12), Cause::DEBUG).unwrap();

    // One hangs in stopped time; the other keeps moving.
    assert_eq!(sim.timeline().clock_of(caught_wake), Some(stop_clock));
    assert!(sim.timeline().is_parked(caught_wake));
    assert_eq!(sim.timeline().parent(personal), Some(realm_clock));
    assert_eq!(sim.timeline().due(free_wake), Some(Tick(5 * SECOND)));
    to(&mut sim, 9 * SECOND);
    assert_eq!(sim.steps(), 1, "only the anchored one's wake-up fired");
    assert!(sim.timeline().wake(caught_wake).is_some());
    assert_eq!(sim.truth().own_time(free), secs(9));
    assert_eq!(sim.truth().own_time(caught), secs(1));

    // With the anchor lifted, its clock goes under the area's and stops.
    let again = wait(&mut sim, free, secs(5));
    sim.anchor_clock(free, None, Cause::DEBUG).unwrap();
    assert_eq!(sim.timeline().parent(personal), Some(stop_clock));
    assert!(sim.timeline().is_parked(again));

    // Refusals: an unknown clock, and a clock under itself.
    let hash = sim.state_hash();
    let unknown = sim
        .anchor_clock(free, Some(ClockId::new(999)), Cause::DEBUG)
        .unwrap_err();
    assert_eq!(unknown.code, codes::CLOCK_UNKNOWN);
    let looped = sim
        .anchor_clock(free, Some(personal), Cause::DEBUG)
        .unwrap_err();
    assert_eq!(looped.code, codes::CLOCK_LOOP);
    assert_eq!(sim.state_hash(), hash);
}

/// T1: overlapping areas resolve by priority, then by id.
#[test]
fn overlapping_areas_resolve_by_priority_then_id() {
    let mut sim = quiet_sim(4);
    let realm_clock = sim.location_clock(at(0, 0));
    // Made in this order, so ids ascend.
    let first = box_area(&mut sim, (0, 0), (20, 20), Some(0.5), 0);
    let second = box_area(&mut sim, (10, 10), (30, 30), Some(2.0), 0);
    let strong = box_area(&mut sim, (15, 15), (40, 40), Some(4.0), 7);
    let plain = box_area(&mut sim, (0, 0), (40, 40), None, 99);
    assert!(first < second && second < strong && strong < plain);
    let clock = |sim: &Sim, area| sim.truth().cosmos().area(area).unwrap().clock;
    assert_eq!(clock(&sim, plain), None);

    // One area: its clock. An area without a clock does not count,
    // whatever its priority.
    assert_eq!(Some(sim.location_clock(at(5, 5))), clock(&sim, first));
    // Equal priority: the lower id.
    assert_eq!(Some(sim.location_clock(at(12, 12))), clock(&sim, first));
    assert_eq!(Some(sim.location_clock(at(25, 25))), clock(&sim, strong));
    assert_eq!(Some(sim.location_clock(at(22, 12))), clock(&sim, second));
    // Higher priority wins over a lower id.
    assert_eq!(Some(sim.location_clock(at(17, 17))), clock(&sim, strong));
    assert_eq!(sim.location_clock(at(50, 50)), realm_clock);

    // An entity walking through them is on each in turn.
    let walker = spawn(&mut sim, creature(), on(50, 50));
    let wake = wait(&mut sim, walker, secs(8));
    for (cell, area) in [
        ((5, 5), first),
        ((12, 12), first),
        ((22, 12), second),
        ((17, 17), strong),
    ] {
        sim.relocate(walker, on(cell.0, cell.1), Cause::DEBUG)
            .unwrap();
        assert_eq!(
            sim.timeline().clock_of(wake),
            clock(&sim, area),
            "at {cell:?}"
        );
        assert_eq!(sim.timeline().remaining(wake), Some(secs(8)));
    }
    // Raising the lower area's priority changes which applies; the world
    // does not report that, so whoever does it settles the clocks by
    // moving or by `set_clock_rate`.
    sim.set_clock_rate(ClockTarget::Area(strong), None, Cause::DEBUG)
        .unwrap();
    assert_eq!(sim.timeline().clock_of(wake), clock(&sim, first));
}

/// `set_time_rate` gives an entity a clock of its own: everything about
/// it, and whatever it holds, runs at that rate (03 §3.5).
#[test]
fn a_personal_clock_speeds_everything_about_an_entity() {
    let mut sim = quiet_sim(5);
    let realm_clock = sim.location_clock(at(0, 0));
    let speedster = spawn(&mut sim, creature(), on(4, 4));
    let watch = spawn(&mut sim, item(), Place::In(speedster));
    let bystander = spawn(&mut sim, creature(), on(5, 4));
    let fast = wait(&mut sim, speedster, secs(10));
    let ticking = wait(&mut sim, watch, secs(10));
    let slow = wait(&mut sim, bystander, secs(10));
    to(&mut sim, 2 * SECOND);

    sim.set_time_rate(speedster, Some(Real::of(4.0)), Cause::DEBUG)
        .unwrap();
    let personal = sim.truth().entity(speedster).unwrap().clock().unwrap();
    assert_eq!(sim.timeline().parent(personal), Some(realm_clock));
    // Eight seconds were left; at four times the rate they take two.
    for wake in [fast, ticking] {
        assert_eq!(sim.timeline().clock_of(wake), Some(personal));
        assert_eq!(sim.timeline().remaining(wake), Some(secs(8)));
        assert_eq!(sim.timeline().due(wake), Some(Tick(4 * SECOND)));
    }
    assert_eq!(sim.timeline().due(slow), Some(Tick(10 * SECOND)));
    assert_eq!(sim.truth().clock_of(watch), Some(personal));

    // What is put down leaves the fast clock; what is picked up joins it.
    sim.relocate(watch, on(4, 5), Cause::DEBUG).unwrap();
    assert_eq!(sim.timeline().clock_of(ticking), Some(realm_clock));
    assert_eq!(sim.timeline().due(ticking), Some(Tick(10 * SECOND)));
    sim.relocate(watch, Place::In(speedster), Cause::DEBUG)
        .unwrap();
    assert_eq!(sim.timeline().due(ticking), Some(Tick(4 * SECOND)));

    // A change of rate reschedules; the events say what changed.
    to(&mut sim, 3 * SECOND);
    sim.set_time_rate(speedster, Some(Real::of(1.0)), Cause::DEBUG)
        .unwrap();
    assert_eq!(sim.timeline().due(fast), Some(Tick(7 * SECOND)));
    sim.set_time_rate(speedster, None, Cause::DEBUG).unwrap();
    assert!(!sim.timeline().has_clock(personal));
    assert_eq!(sim.truth().entity(speedster).unwrap().clock(), None);
    for wake in [fast, ticking] {
        assert_eq!(sim.timeline().clock_of(wake), Some(realm_clock));
        assert_eq!(sim.timeline().due(wake), Some(Tick(7 * SECOND)));
    }
    // Own time: 2 s at rate 1, 1 s at rate 4, and it keeps counting.
    assert_eq!(sim.truth().own_time(speedster), secs(6));

    sim.pump();
    let rates: Vec<(Option<Real>, Option<Real>)> = sim
        .drain_events()
        .into_iter()
        .filter_map(|event| match event.body {
            EventBody::ClockRateChanged {
                entity, old, new, ..
            } => {
                assert_eq!(entity, Some(speedster));
                Some((old, new))
            }
            _ => None,
        })
        .collect();
    assert_eq!(
        rates,
        [
            (None, Some(Real::of(4.0))),
            (Some(Real::of(4.0)), Some(Real::ONE)),
            (Some(Real::ONE), None)
        ]
    );

    // Refusals carry reasons and change nothing.
    let hash = sim.state_hash();
    let bad = sim
        .set_time_rate(bystander, Some(Real::of(-1.0)), Cause::DEBUG)
        .unwrap_err();
    assert_eq!(bad.code, codes::CLOCK_BAD_RATE);
    let ghost = EntityId::new(77, 0);
    assert_eq!(
        sim.set_time_rate(ghost, Some(Real::ONE), Cause::DEBUG)
            .unwrap_err()
            .code,
        codes::ENTITY_UNKNOWN
    );
    assert_eq!(sim.state_hash(), hash);
}

/// `set_clock_rate` changes an area's or a realm's clock, and gives or
/// takes an area's clock away with whoever is inside following.
#[test]
fn set_clock_rate_changes_area_and_realm_clocks() {
    let mut sim = quiet_sim(6);
    let realm = common::realm(&sim);
    let realm_clock = sim.location_clock(at(0, 0));
    let room = box_area(&mut sim, (0, 0), (9, 9), None, 0);
    let inside = spawn(&mut sim, creature(), on(3, 3));
    let outside = spawn(&mut sim, creature(), on(30, 30));
    let in_wake = wait(&mut sim, inside, secs(10));
    let out_wake = wait(&mut sim, outside, secs(10));

    // The room is given a clock: a year inside per day outside, nearly.
    sim.set_clock_rate(ClockTarget::Area(room), Some(Real::of(10.0)), Cause::DEBUG)
        .unwrap();
    let room_clock = sim.truth().cosmos().area(room).unwrap().clock.unwrap();
    assert_eq!(sim.timeline().clock_of(in_wake), Some(room_clock));
    assert_eq!(sim.timeline().due(in_wake), Some(Tick(SECOND)));
    assert_eq!(sim.timeline().due(out_wake), Some(Tick(10 * SECOND)));

    // Its rate changes: the same clock, rescheduled.
    sim.set_clock_rate(ClockTarget::Area(room), Some(Real::of(0.0)), Cause::DEBUG)
        .unwrap();
    assert!(sim.timeline().is_parked(in_wake));
    to(&mut sim, 3 * SECOND);

    // Its clock is taken away: back to the realm's, nothing lost.
    sim.set_clock_rate(ClockTarget::Area(room), None, Cause::DEBUG)
        .unwrap();
    assert_eq!(sim.truth().cosmos().area(room).unwrap().clock, None);
    assert_eq!(sim.timeline().clock_of(in_wake), Some(realm_clock));
    assert_eq!(sim.timeline().remaining(in_wake), Some(secs(10)));
    assert_eq!(
        sim.truth().own_time(inside),
        Dur::ZERO,
        "no time passed for it"
    );
    assert_eq!(sim.truth().own_time(outside), secs(3));

    // The realm's clock: everything in the realm follows.
    sim.set_clock_rate(ClockTarget::Realm(realm), Some(Real::of(2.0)), Cause::DEBUG)
        .unwrap();
    assert_eq!(sim.timeline().due(in_wake), Some(Tick(8 * SECOND)));
    assert_eq!(
        sim.timeline().due(out_wake),
        Some(Tick(3 * SECOND + 3_500_000))
    );

    sim.pump();
    let changes = sim
        .drain_events()
        .into_iter()
        .filter(|event| event.name() == "clock_rate_changed")
        .count();
    assert_eq!(changes, 4);

    // Refusals.
    let hash = sim.state_hash();
    let none = sim
        .set_clock_rate(ClockTarget::Realm(realm), None, Cause::DEBUG)
        .unwrap_err();
    assert_eq!(none.code, codes::CLOCK_BAD_RATE);
    let unknown = sim
        .set_clock_rate(
            ClockTarget::Area(clash_world::AreaId::new(404)),
            Some(Real::ONE),
            Cause::DEBUG,
        )
        .unwrap_err();
    assert_eq!(unknown.code, codes::WORLD_UNKNOWN_AREA);
    assert_eq!(sim.state_hash(), hash);
}

/// Clock assignment survives a save and a load: the loaded simulation
/// has the same wake-ups on the same clocks and goes on the same way.
#[test]
fn clock_assignment_survives_save_and_load() {
    let mut sim = common::walking_sim(7);
    let slow = box_area(&mut sim, (10, 10), (19, 19), Some(0.5), 0);
    let realm_clock = sim.location_clock(at(0, 0));
    let a = spawn(&mut sim, creature(), on(12, 12));
    let b = spawn(&mut sim, creature(), on(2, 2));
    sim.anchor_clock(b, Some(realm_clock), Cause::DEBUG)
        .unwrap();
    sim.set_time_rate(b, Some(Real::of(3.0)), Cause::DEBUG)
        .unwrap();
    for id in [a, b] {
        sim.schedule_for(id, secs(1), walk_wake(id, 20)).unwrap();
    }
    to(&mut sim, 4 * SECOND);

    let (mut loaded, unplaced) = common::save_and_load(&sim);
    assert!(unplaced.is_empty());
    loaded.set_wake_handler(WakeDomain::Stepper, common::walker);
    assert_eq!(loaded.hashes(), sim.hashes());
    assert_eq!(loaded.truth().clock_of(a), sim.truth().clock_of(a));
    assert_eq!(loaded.truth().wakes_of(b), sim.truth().wakes_of(b));
    for target in [&mut sim, &mut loaded] {
        target
            .set_clock_rate(ClockTarget::Area(slow), Some(Real::of(0.25)), Cause::DEBUG)
            .unwrap();
        to(target, 30 * SECOND);
    }
    assert_eq!(loaded.hashes(), sim.hashes());
    assert_eq!(loaded.drain_events(), sim.drain_events());
}
