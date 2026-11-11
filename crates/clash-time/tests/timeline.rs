//! The timeline and the clock tree: ordering, rates, parking, rehoming,
//! saving. These are the "Done when" items of work package F3 and the
//! invariants T1 to T3 of `docs/03-time-and-actions.md` section 14.

use clash_core::{Dur, Hash128, Real, Tick};
use clash_time::{ClockId, TimeError, Timeline, Wake, WakeDomain, WakeId};

const U: ClockId = ClockId::UNIVERSAL;

fn wake(id: u64) -> Wake {
    Wake::new(WakeDomain::Timer, id, 0)
}

fn secs(n: i64) -> Dur {
    Dur::from_secs(n)
}

fn at(n: i64) -> Tick {
    Tick::ZERO + Dur::from_secs(n)
}

fn r(x: f64) -> Real {
    Real::of(x)
}

/// Pops everything due by `limit` and returns the payload ids with the
/// ticks they fired at.
fn drain(timeline: &mut Timeline, limit: Tick) -> Vec<(u64, Tick)> {
    let mut fired = Vec::new();
    while let Some((_, wake)) = timeline.pop(limit) {
        fired.push((wake.id, timeline.now()));
    }
    fired
}

// ---- Ordering (T1) -------------------------------------------------------

#[test]
fn same_tick_wake_ups_pop_in_scheduling_order() {
    let mut timeline = Timeline::new();
    let ids: Vec<WakeId> = (0..10)
        .map(|n| timeline.schedule(U, secs(1), wake(n)).unwrap())
        .collect();
    assert!(ids.windows(2).all(|pair| pair[0] < pair[1]));

    for (n, expected) in ids.iter().enumerate() {
        let (id, payload) = timeline.pop(Tick::MAX).unwrap();
        assert_eq!(id, *expected);
        assert_eq!(payload, wake(n as u64));
        assert_eq!(timeline.now(), at(1));
    }
    assert_eq!(timeline.pop(Tick::MAX), None);
}

#[test]
fn wake_ups_pop_by_due_tick_then_by_sequence() {
    let mut timeline = Timeline::new();
    timeline.schedule(U, secs(3), wake(0)).unwrap();
    timeline.schedule(U, secs(1), wake(1)).unwrap();
    timeline.schedule(U, secs(2), wake(2)).unwrap();
    timeline.schedule(U, secs(1), wake(3)).unwrap();
    timeline.schedule(U, Dur::ZERO, wake(4)).unwrap();
    assert_eq!(
        drain(&mut timeline, Tick::MAX),
        [(4, at(0)), (1, at(1)), (3, at(1)), (2, at(2)), (0, at(3))]
    );
}

#[test]
fn same_tick_order_survives_rate_changes_and_different_clocks() {
    // Two clocks, three wake-ups that all come due at universal 8 s by
    // different routes. They must still pop in the order scheduled.
    let mut timeline = Timeline::new();
    let slow = timeline.create_clock(U, r(0.5)).unwrap();
    let a = timeline.schedule(slow, secs(4), wake(0)).unwrap(); // 4 / 0.5 = 8 s
    let b = timeline.schedule(U, secs(8), wake(1)).unwrap();
    let c = timeline.schedule(slow, secs(4), wake(2)).unwrap();

    timeline.advance_to(at(2));
    // Speed the slow clock up and put it back: remaining 3 s local each time.
    timeline.set_rate(slow, r(2.0)).unwrap();
    timeline.set_rate(slow, r(0.5)).unwrap();
    assert_eq!(timeline.due(a), Some(at(8)));
    assert_eq!(timeline.due(c), Some(at(8)));

    let popped: Vec<WakeId> = std::iter::from_fn(|| timeline.pop(Tick::MAX))
        .map(|(id, _)| id)
        .collect();
    assert_eq!(popped, [a, b, c]);
    assert_eq!(timeline.now(), at(8));
}

#[test]
fn a_wake_up_scheduled_for_now_runs_after_those_already_due_now() {
    let mut timeline = Timeline::new();
    timeline.schedule(U, secs(1), wake(0)).unwrap();
    assert_eq!(timeline.pop(Tick::MAX).unwrap().1, wake(0));
    // Scheduled from inside the handler, for the same tick.
    timeline.schedule(U, Dur::ZERO, wake(1)).unwrap();
    timeline.schedule(U, Dur(-5), wake(2)).unwrap();
    assert_eq!(drain(&mut timeline, Tick::MAX), [(1, at(1)), (2, at(1))]);
}

// ---- Popping and advancing -------------------------------------------------

#[test]
fn pop_respects_its_limit_and_leaves_time_alone_when_nothing_is_due() {
    let mut timeline = Timeline::new();
    timeline.schedule(U, secs(5), wake(0)).unwrap();
    assert_eq!(timeline.peek(), Some(at(5)));
    assert_eq!(timeline.pop(at(4)), None);
    assert_eq!(timeline.now(), Tick::ZERO);
    assert_eq!(timeline.pop(at(5)).unwrap().1, wake(0));
    assert_eq!(timeline.now(), at(5));
    assert_eq!(timeline.peek(), None);
    assert_eq!(timeline.pop(Tick::MAX), None);
    // An empty timeline does not run off to the end of time.
    assert_eq!(timeline.now(), at(5));
}

#[test]
fn advance_to_never_passes_a_wake_up_and_never_goes_back() {
    let mut timeline = Timeline::new();
    timeline.schedule(U, secs(5), wake(0)).unwrap();
    assert_eq!(timeline.advance_to(at(3)), at(3));
    assert_eq!(timeline.advance_to(at(9)), at(5));
    assert_eq!(timeline.advance_to(at(1)), at(5));
    assert_eq!(timeline.now(), at(5));
    timeline.pop(Tick::MAX).unwrap();
    assert_eq!(timeline.advance_to(at(9)), at(9));
    assert_eq!(timeline.local_now(U), Ok(secs(9)));
}

#[test]
fn cancel_and_queries() {
    let mut timeline = Timeline::new();
    let a = timeline.schedule(U, secs(2), wake(7)).unwrap();
    let b = timeline.schedule(U, secs(1), wake(8)).unwrap();
    assert_eq!(timeline.wake_count(), 2);
    assert_eq!(timeline.wake(a), Some(wake(7)));
    assert_eq!(timeline.clock_of(a), Some(U));
    assert_eq!(timeline.due(a), Some(at(2)));
    assert_eq!(timeline.wakes_on(U).collect::<Vec<_>>(), [a, b]);

    assert!(timeline.cancel(a));
    assert!(!timeline.cancel(a));
    assert_eq!(timeline.remaining(a), None);
    assert_eq!(timeline.wake(a), None);
    assert_eq!(timeline.due(a), None);
    assert!(!timeline.is_parked(a));
    assert_eq!(timeline.wakes_on(U).collect::<Vec<_>>(), [b]);
    // The cancelled wake-up is out of the queue: the next one due is b.
    assert_eq!(timeline.peek(), Some(at(1)));
    assert_eq!(drain(&mut timeline, Tick::MAX), [(8, at(1))]);
    assert_eq!(timeline.peek(), None);
    // Nothing holds time back at the cancelled wake-up's old due tick.
    assert_eq!(timeline.advance_to(at(5)), at(5));
    // Fired wake-ups are gone too.
    assert!(!timeline.cancel(b));
    assert_eq!(timeline.wake_count(), 0);
}

#[test]
fn cancelling_a_parked_wake_up_works() {
    let mut timeline = Timeline::new();
    let stopped = timeline.create_clock(U, Real::ZERO).unwrap();
    let id = timeline.schedule(stopped, secs(1), wake(0)).unwrap();
    assert!(timeline.is_parked(id));
    assert!(timeline.cancel(id));
    timeline.set_rate(stopped, Real::ONE).unwrap();
    assert_eq!(timeline.pop(Tick::MAX), None);
    assert_eq!(timeline.wake_count(), 0);
}

#[test]
fn local_time_is_whole_microseconds_rounded_down_and_loses_nothing() {
    let mut timeline = Timeline::new();
    let slow = timeline.create_clock(U, r(0.3)).unwrap();
    // 5 ticks at 0.3 is 1.5 µs of local time: one whole microsecond.
    timeline.advance_to(Tick(5));
    assert_eq!(timeline.local_now(slow), Ok(Dur(1)));
    // Bringing the clock up to date keeps the half: 6 more ticks add
    // 1.8 µs, making 3.3 in all. Had the half been dropped it would be 2.
    timeline.set_rate(slow, r(0.6)).unwrap();
    timeline.set_rate(slow, r(0.3)).unwrap();
    assert_eq!(timeline.local_now(slow), Ok(Dur(1)));
    timeline.advance_to(Tick(11));
    assert_eq!(timeline.local_now(slow), Ok(Dur(3)));
    // However often it is brought up to date, nothing is lost: by tick
    // 1,011, brought up to date at every tick, 303.3 µs have passed.
    for tick in 12..=1_011 {
        timeline.advance_to(Tick(tick));
        timeline.set_rate(slow, r(0.6)).unwrap();
        timeline.set_rate(slow, r(0.3)).unwrap();
    }
    assert_eq!(timeline.local_now(slow), Ok(Dur(303)));
}

// ---- Rates and parking (T2, T3) ---------------------------------------------

#[test]
fn a_ten_second_wait_at_half_rate_after_four_seconds_fires_at_sixteen() {
    let mut timeline = Timeline::new();
    let clock = timeline.create_clock(U, Real::ONE).unwrap();
    let id = timeline.schedule(clock, secs(10), wake(0)).unwrap();
    assert_eq!(timeline.due(id), Some(at(10)));

    timeline.advance_to(at(4));
    assert_eq!(timeline.remaining(id), Some(secs(6)));
    timeline.set_rate(clock, r(0.5)).unwrap();
    assert_eq!(timeline.remaining(id), Some(secs(6)));
    assert_eq!(timeline.due(id), Some(at(16)));

    assert_eq!(timeline.pop(at(15)), None);
    assert_eq!(drain(&mut timeline, Tick::MAX), [(0, at(16))]);
    assert_eq!(timeline.local_now(clock), Ok(secs(10)));
}

#[test]
fn at_rate_zero_a_wake_up_never_fires_and_resumes_with_its_time_intact() {
    let mut timeline = Timeline::new();
    let clock = timeline.create_clock(U, Real::ONE).unwrap();
    let id = timeline.schedule(clock, secs(10), wake(0)).unwrap();
    let marker = timeline.schedule(U, secs(1_000), wake(99)).unwrap();

    timeline.advance_to(at(4));
    timeline.set_rate(clock, Real::ZERO).unwrap();
    assert!(timeline.is_parked(id));
    assert_eq!(timeline.due(id), None);
    assert_eq!(timeline.remaining(id), Some(secs(6)));

    // A thousand seconds pass on the universal clock. Nothing fires on
    // the stopped one, and its local time does not move.
    assert_eq!(drain(&mut timeline, Tick::MAX), [(99, at(1_000))]);
    assert!(!timeline.cancel(marker));
    assert_eq!(timeline.pop(Tick::MAX), None);
    assert_eq!(timeline.peek(), None);
    assert_eq!(timeline.remaining(id), Some(secs(6)));
    assert_eq!(timeline.local_now(clock), Ok(secs(4)));

    // It resumes with six seconds remaining.
    timeline.set_rate(clock, Real::ONE).unwrap();
    assert!(!timeline.is_parked(id));
    assert_eq!(timeline.remaining(id), Some(secs(6)));
    assert_eq!(drain(&mut timeline, Tick::MAX), [(0, at(1_006))]);
}

#[test]
fn a_wake_up_scheduled_on_a_stopped_clock_waits_for_it_to_start() {
    let mut timeline = Timeline::new();
    let clock = timeline.create_clock(U, Real::ZERO).unwrap();
    let id = timeline.schedule(clock, secs(3), wake(0)).unwrap();
    assert!(timeline.is_parked(id));
    timeline.advance_to(at(50));
    assert_eq!(timeline.remaining(id), Some(secs(3)));
    timeline.set_rate(clock, r(3.0)).unwrap();
    assert_eq!(drain(&mut timeline, Tick::MAX), [(0, at(51))]);
}

#[test]
fn a_zero_wait_on_a_stopped_clock_is_still_due_now() {
    let mut timeline = Timeline::new();
    let clock = timeline.create_clock(U, Real::ZERO).unwrap();
    timeline.schedule(clock, Dur::ZERO, wake(0)).unwrap();
    assert_eq!(drain(&mut timeline, Tick::MAX), [(0, at(0))]);
}

#[test]
fn nested_clock_rates_multiply() {
    let mut timeline = Timeline::new();
    let realm = timeline.create_clock(U, r(4.0)).unwrap();
    let area = timeline.create_clock(realm, r(0.5)).unwrap();
    let person = timeline.create_clock(area, r(3.0)).unwrap();
    assert_eq!(timeline.effective_rate(realm), Some(r(4.0)));
    assert_eq!(timeline.effective_rate(area), Some(r(2.0)));
    assert_eq!(timeline.effective_rate(person), Some(r(6.0)));
    assert_eq!(timeline.rate(person), Some(r(3.0)));
    assert_eq!(timeline.parent(person), Some(area));

    // 60 s of the person's time takes 10 s of universal time.
    timeline.schedule(person, secs(60), wake(0)).unwrap();
    timeline.schedule(area, secs(60), wake(1)).unwrap();
    timeline.schedule(realm, secs(60), wake(2)).unwrap();
    assert_eq!(
        drain(&mut timeline, Tick::MAX),
        [(0, at(10)), (2, at(15)), (1, at(30))]
    );
    assert_eq!(timeline.local_now(realm), Ok(secs(120)));
    assert_eq!(timeline.local_now(area), Ok(secs(60)));
    assert_eq!(timeline.local_now(person), Ok(secs(180)));
}

#[test]
fn changing_a_parent_rate_reaches_every_clock_beneath_it() {
    let mut timeline = Timeline::new();
    let realm = timeline.create_clock(U, Real::ONE).unwrap();
    let area = timeline.create_clock(realm, Real::ONE).unwrap();
    let person = timeline.create_clock(area, r(2.0)).unwrap();
    let on_realm = timeline.schedule(realm, secs(10), wake(0)).unwrap();
    let on_person = timeline.schedule(person, secs(10), wake(1)).unwrap();
    let outside = timeline.schedule(U, secs(10), wake(2)).unwrap();

    timeline.advance_to(at(2));
    // Time stops in the realm. The speedster inside stops with it.
    timeline.set_rate(realm, Real::ZERO).unwrap();
    assert_eq!(timeline.effective_rate(person), Some(Real::ZERO));
    assert!(timeline.is_parked(on_realm));
    assert!(timeline.is_parked(on_person));
    assert_eq!(timeline.remaining(on_realm), Some(secs(8)));
    assert_eq!(timeline.remaining(on_person), Some(secs(6)));
    assert_eq!(timeline.due(outside), Some(at(10)));

    assert_eq!(drain(&mut timeline, at(100)), [(2, at(10))]);
    timeline.advance_to(at(100));
    timeline.set_rate(realm, r(0.5)).unwrap();
    // 8 s at 0.5 and 6 s at 0.5 × 2.
    assert_eq!(
        drain(&mut timeline, Tick::MAX),
        [(1, at(106)), (0, at(116))]
    );
}

#[test]
fn a_rate_change_preserves_the_remaining_local_time_of_everything_on_the_clock() {
    let mut timeline = Timeline::new();
    let clock = timeline.create_clock(U, r(1.5)).unwrap();
    let child = timeline.create_clock(clock, r(0.25)).unwrap();
    let waits = [
        (clock, 7),
        (clock, 1),
        (clock, 300),
        (clock, 42),
        (clock, 9_000),
        (child, 13),
        (child, 2_000),
    ];
    let ids: Vec<WakeId> = waits
        .into_iter()
        .map(|(on, ms)| {
            timeline
                .schedule(on, Dur::from_millis(ms), wake(ms as u64))
                .unwrap()
        })
        .collect();

    timeline.advance_to(Tick(333));
    for rate in [0.3, 7.0, 0.0, 1e-3, 365.0, 1.0] {
        let before: Vec<Option<Dur>> = ids.iter().map(|id| timeline.remaining(*id)).collect();
        timeline.set_rate(clock, r(rate)).unwrap();
        let after: Vec<Option<Dur>> = ids.iter().map(|id| timeline.remaining(*id)).collect();
        assert_eq!(before, after, "rate {rate}");
    }
    // Order on each clock is still the order of remaining time.
    let fired: Vec<u64> = drain(&mut timeline, Tick::MAX)
        .into_iter()
        .map(|(id, _)| id)
        .collect();
    let on_child = |id: &u64| [13, 2_000].contains(id);
    let from_clock: Vec<u64> = fired.iter().copied().filter(|id| !on_child(id)).collect();
    let from_child: Vec<u64> = fired.iter().copied().filter(on_child).collect();
    assert_eq!(from_clock, [1, 7, 42, 300, 9_000]);
    assert_eq!(from_child, [13, 2_000]);
}

#[test]
fn many_rate_changes_do_not_let_a_wake_up_drift() {
    // 1 s of local time on a clock that flips between two rates every
    // 7 ticks. Exact arithmetic says when it must fire; however many
    // changes there are, the wake-up fires within one tick of that.
    let mut timeline = Timeline::new();
    let clock = timeline.create_clock(U, r(0.3)).unwrap();
    let id = timeline.schedule(clock, secs(1), wake(0)).unwrap();

    let rates = [0.3, 0.7];
    let mut local = 0.0_f64; // local time passed so far, to well under a tick
    let mut changes = 0_u32;
    let fired_at = loop {
        // At a mean rate of 0.5 it takes about 286,000 steps of 7 ticks.
        assert!(changes < 400_000, "the wake-up never fired");
        let step_end = timeline.now() + Dur(7);
        if let Some((popped, _)) = timeline.pop(step_end) {
            assert_eq!(popped, id);
            break timeline.now();
        }
        timeline.advance_to(step_end);
        local += 7.0 * rates[(changes % 2) as usize];
        changes += 1;
        timeline
            .set_rate(clock, r(rates[(changes % 2) as usize]))
            .unwrap();
    };
    assert!(changes > 100_000, "only {changes} changes");

    // The exact firing time: where the remaining local time runs out at
    // the rate in force.
    let rate = rates[(changes % 2) as usize];
    let exact = (changes as f64) * 7.0 + (1_000_000.0 - local) / rate;
    let error = fired_at.as_micros() as f64 - exact;
    assert!(
        (-1e-3..=1.001).contains(&error),
        "fired at {fired_at}, exact {exact}, after {changes} changes"
    );
}

#[test]
fn setting_the_same_rate_changes_nothing() {
    let mut timeline = Timeline::new();
    let clock = timeline.create_clock(U, r(0.3)).unwrap();
    timeline.schedule(clock, secs(1), wake(0)).unwrap();
    timeline.advance_to(Tick(12_345));
    let before = Hash128::of(&timeline);
    timeline.set_rate(clock, r(0.3)).unwrap();
    timeline.reparent(clock, U).unwrap();
    assert_eq!(Hash128::of(&timeline), before);
}

// ---- Reparenting, rehoming, destroying -------------------------------------

#[test]
fn reparenting_preserves_remaining_local_time() {
    let mut timeline = Timeline::new();
    let fast = timeline.create_clock(U, r(2.0)).unwrap();
    let slow = timeline.create_clock(U, r(0.5)).unwrap();
    let person = timeline.create_clock(fast, Real::ONE).unwrap();
    let id = timeline.schedule(person, secs(10), wake(0)).unwrap();
    assert_eq!(timeline.due(id), Some(at(5)));

    timeline.advance_to(at(2));
    assert_eq!(timeline.remaining(id), Some(secs(6)));
    timeline.reparent(person, slow).unwrap();
    assert_eq!(timeline.parent(person), Some(slow));
    assert_eq!(timeline.effective_rate(person), Some(r(0.5)));
    assert_eq!(timeline.remaining(id), Some(secs(6)));
    // The clock's own local time carries on from where it was.
    assert_eq!(timeline.local_now(person), Ok(secs(4)));
    assert_eq!(drain(&mut timeline, Tick::MAX), [(0, at(14))]);
    assert_eq!(timeline.local_now(person), Ok(secs(10)));
}

#[test]
fn reparenting_carries_the_whole_subtree() {
    let mut timeline = Timeline::new();
    let a = timeline.create_clock(U, r(2.0)).unwrap();
    let b = timeline.create_clock(U, r(0.5)).unwrap();
    let group = timeline.create_clock(a, Real::ONE).unwrap();
    let member = timeline.create_clock(group, r(4.0)).unwrap();
    let id = timeline.schedule(member, secs(16), wake(0)).unwrap();
    assert_eq!(timeline.due(id), Some(at(2)));
    timeline.advance_to(at(1));
    timeline.reparent(group, b).unwrap();
    assert_eq!(timeline.effective_rate(member), Some(r(2.0)));
    assert_eq!(timeline.remaining(id), Some(secs(8)));
    assert_eq!(drain(&mut timeline, Tick::MAX), [(0, at(5))]);
}

#[test]
fn a_clock_cannot_be_placed_under_itself() {
    let mut timeline = Timeline::new();
    let a = timeline.create_clock(U, Real::ONE).unwrap();
    let b = timeline.create_clock(a, Real::ONE).unwrap();
    let c = timeline.create_clock(b, Real::ONE).unwrap();
    assert_eq!(
        timeline.reparent(a, c),
        Err(TimeError::ClockLoop {
            clock: a,
            parent: c
        })
    );
    assert_eq!(
        timeline.reparent(a, a),
        Err(TimeError::ClockLoop {
            clock: a,
            parent: a
        })
    );
    assert_eq!(timeline.parent(a), Some(U));
    // Moving the other way is fine.
    timeline.reparent(c, a).unwrap();
    assert_eq!(timeline.parent(c), Some(a));
}

#[test]
fn rehoming_keeps_remaining_local_time() {
    let mut timeline = Timeline::new();
    let fast = timeline.create_clock(U, r(2.0)).unwrap();
    let slow = timeline.create_clock(U, r(0.25)).unwrap();
    let id = timeline.schedule(fast, secs(10), wake(0)).unwrap();
    let stays = timeline.schedule(fast, secs(10), wake(1)).unwrap();

    timeline.advance_to(at(3));
    assert_eq!(timeline.remaining(id), Some(secs(4)));
    timeline.rehome(id, slow).unwrap();
    assert_eq!(timeline.clock_of(id), Some(slow));
    assert_eq!(timeline.remaining(id), Some(secs(4)));
    assert_eq!(timeline.due(id), Some(at(19)));
    assert_eq!(timeline.wakes_on(fast).collect::<Vec<_>>(), [stays]);
    assert_eq!(timeline.wakes_on(slow).collect::<Vec<_>>(), [id]);

    // Onto a stopped clock and off again.
    let stopped = timeline.create_clock(U, Real::ZERO).unwrap();
    timeline.rehome(id, stopped).unwrap();
    assert!(timeline.is_parked(id));
    timeline.advance_to(at(4));
    timeline.rehome(id, U).unwrap();
    assert_eq!(timeline.remaining(id), Some(secs(4)));
    assert_eq!(drain(&mut timeline, Tick::MAX), [(1, at(5)), (0, at(8))]);
}

#[test]
fn destroying_a_clock_rehomes_its_wake_ups_and_children_to_the_parent() {
    let mut timeline = Timeline::new();
    let realm = timeline.create_clock(U, r(2.0)).unwrap();
    let bubble = timeline.create_clock(realm, r(0.5)).unwrap();
    let inside = timeline.create_clock(bubble, r(3.0)).unwrap();
    let on_bubble = timeline.schedule(bubble, secs(10), wake(0)).unwrap();
    let on_inside = timeline.schedule(inside, secs(30), wake(1)).unwrap();
    assert_eq!(timeline.due(on_bubble), Some(at(10)));
    assert_eq!(timeline.due(on_inside), Some(at(10)));

    timeline.advance_to(at(4));
    timeline.destroy_clock(bubble).unwrap();
    assert!(!timeline.has_clock(bubble));
    assert_eq!(timeline.clocks().collect::<Vec<_>>(), [U, realm, inside]);

    // The wake-up moved to the realm clock with 6 s of local time left.
    assert_eq!(timeline.clock_of(on_bubble), Some(realm));
    assert_eq!(timeline.remaining(on_bubble), Some(secs(6)));
    assert_eq!(timeline.due(on_bubble), Some(at(7)));

    // The child hangs from the realm now, at its own rate: 2 × 3.
    assert_eq!(timeline.parent(inside), Some(realm));
    assert_eq!(timeline.effective_rate(inside), Some(r(6.0)));
    assert_eq!(timeline.remaining(on_inside), Some(secs(18)));
    assert_eq!(timeline.due(on_inside), Some(at(7)));
    assert_eq!(timeline.local_now(inside), Ok(secs(12)));

    assert_eq!(drain(&mut timeline, Tick::MAX), [(0, at(7)), (1, at(7))]);
    assert_eq!(
        timeline.schedule(bubble, secs(1), wake(2)),
        Err(TimeError::UnknownClock(bubble))
    );
}

#[test]
fn clock_ids_are_not_reused() {
    let mut timeline = Timeline::new();
    let a = timeline.create_clock(U, Real::ONE).unwrap();
    timeline.destroy_clock(a).unwrap();
    let b = timeline.create_clock(U, Real::ONE).unwrap();
    assert_ne!(a, b);
    assert!(!timeline.has_clock(a));
    assert_eq!(timeline.local_now(a), Err(TimeError::UnknownClock(a)));
}

// ---- Refusals ------------------------------------------------------------------

#[test]
fn refused_operations_say_why_and_change_nothing() {
    let mut timeline = Timeline::new();
    let clock = timeline.create_clock(U, Real::ONE).unwrap();
    let id = timeline.schedule(clock, secs(1), wake(0)).unwrap();
    let ghost = ClockId::new(999);
    let before = Hash128::of(&timeline);

    assert_eq!(
        timeline.schedule(ghost, secs(1), wake(1)),
        Err(TimeError::UnknownClock(ghost))
    );
    assert_eq!(
        timeline.create_clock(ghost, Real::ONE),
        Err(TimeError::UnknownClock(ghost))
    );
    assert_eq!(
        timeline.create_clock(U, r(-1.0)),
        Err(TimeError::BadRate(r(-1.0)))
    );
    assert_eq!(
        timeline.set_rate(clock, Real::INFINITY),
        Err(TimeError::BadRate(Real::INFINITY))
    );
    assert_eq!(
        timeline.set_rate(ghost, Real::ONE),
        Err(TimeError::UnknownClock(ghost))
    );
    assert_eq!(timeline.set_rate(U, r(2.0)), Err(TimeError::UniversalClock));
    assert_eq!(timeline.reparent(U, clock), Err(TimeError::UniversalClock));
    assert_eq!(
        timeline.reparent(clock, ghost),
        Err(TimeError::UnknownClock(ghost))
    );
    assert_eq!(timeline.destroy_clock(U), Err(TimeError::UniversalClock));
    assert_eq!(
        timeline.destroy_clock(ghost),
        Err(TimeError::UnknownClock(ghost))
    );
    assert_eq!(
        timeline.rehome(id, ghost),
        Err(TimeError::UnknownClock(ghost))
    );
    assert_eq!(
        timeline.rehome(WakeId::new(999), clock),
        Err(TimeError::UnknownWake(WakeId::new(999)))
    );
    assert_eq!(
        timeline.local_now(ghost),
        Err(TimeError::UnknownClock(ghost))
    );

    assert_eq!(Hash128::of(&timeline), before);
    assert_eq!(
        TimeError::UnknownClock(ghost).to_string(),
        "unknown clock `999`"
    );
    assert_eq!(
        TimeError::BadRate(r(-1.0)).to_string(),
        "a clock's rate must be zero or more and finite, but `-1` was given"
    );
}

#[test]
fn negative_zero_is_a_rate_of_zero() {
    let mut timeline = Timeline::new();
    let clock = timeline.create_clock(U, r(-0.0)).unwrap();
    let id = timeline.schedule(clock, secs(1), wake(0)).unwrap();
    assert!(timeline.is_parked(id));
    timeline.set_rate(clock, Real::ONE).unwrap();
    timeline.set_rate(clock, r(-0.0)).unwrap();
    assert!(timeline.is_parked(id));
    assert_eq!(timeline.rate(clock), Some(Real::ZERO));
}

// ---- Extremes ---------------------------------------------------------------------

#[test]
fn waits_too_long_to_represent_saturate_instead_of_overflowing() {
    let mut timeline = Timeline::new();
    let crawl = timeline.create_clock(U, r(1e-12)).unwrap();
    let never = timeline.schedule(crawl, Dur::MAX, wake(0)).unwrap();
    let forever = timeline.schedule(U, Dur::MAX, wake(1)).unwrap();
    assert_eq!(timeline.due(never), Some(Tick::MAX));
    assert_eq!(timeline.due(forever), Some(Tick::MAX));
    assert_eq!(timeline.remaining(forever), Some(Dur::MAX));
    timeline.advance_to(at(1_000_000));
    timeline.set_rate(crawl, r(1e12)).unwrap();
    assert!(timeline.due(never).is_some());
}

#[test]
fn a_century_at_rate_one_is_exact_to_the_tick() {
    let mut timeline = Timeline::new();
    let realm = timeline.create_clock(U, Real::ONE).unwrap();
    let century = Dur::DAY * 36_525;
    let id = timeline.schedule(realm, century + Dur(1), wake(0)).unwrap();
    timeline.advance_to(Tick::ZERO + century);
    assert_eq!(timeline.local_now(realm), Ok(century));
    assert_eq!(timeline.remaining(id), Some(Dur(1)));
    assert_eq!(
        drain(&mut timeline, Tick::MAX),
        [(0, Tick::ZERO + century + Dur(1))]
    );
}

// ---- Saving and hashing -------------------------------------------------------------

/// A timeline in the middle of something: several clocks, one stopped,
/// parked wake-ups, fractional local time, a clock created and destroyed.
/// Two of its wake-ups have fired; ten wait on live clocks or parked, and
/// one more was inherited from the destroyed clock.
fn busy_timeline() -> Timeline {
    let mut timeline = Timeline::new();
    let realm = timeline.create_clock(U, r(0.3)).unwrap();
    let area = timeline.create_clock(realm, r(7.0)).unwrap();
    let stopped = timeline.create_clock(U, Real::ZERO).unwrap();
    for n in 0..12_u64 {
        let clock = [U, realm, area, stopped][(n % 4) as usize];
        let after = Dur::from_millis(((n * 37) % 11 + 1) as i64);
        timeline.schedule(clock, after, wake(n)).unwrap();
    }
    assert_eq!(drain(&mut timeline, Tick(2_500)).len(), 2);
    timeline.advance_to(Tick(2_500));
    timeline.set_rate(realm, r(1.7)).unwrap();
    let doomed = timeline.create_clock(area, r(2.0)).unwrap();
    timeline.schedule(doomed, Dur(55_000), wake(100)).unwrap();
    timeline.advance_to(Tick(2_513));
    assert_eq!(timeline.now(), Tick(2_513));
    timeline.destroy_clock(doomed).unwrap();
    assert_eq!(timeline.wake_count(), 11);
    timeline
}

#[test]
fn saving_and_loading_mid_run_preserves_pop_order() {
    let mut original = busy_timeline();
    let json = serde_json::to_string(&original).unwrap();
    let mut loaded: Timeline = serde_json::from_str(&json).unwrap();

    assert_eq!(loaded, original);
    assert_eq!(Hash128::of(&loaded), Hash128::of(&original));
    assert_eq!(serde_json::to_string(&loaded).unwrap(), json);

    // Both carry on identically: same pops at the same ticks, through a
    // further rate change, and new ids continue from the same place.
    let stopped = ClockId::new(3);
    for timeline in [&mut original, &mut loaded] {
        timeline.schedule(U, Dur(5), wake(200)).unwrap();
        timeline.set_rate(stopped, r(0.01)).unwrap();
    }
    let from_original = drain(&mut original, Tick::MAX);
    let from_loaded = drain(&mut loaded, Tick::MAX);
    assert_eq!(from_original, from_loaded);
    assert_eq!(from_original.len(), 12);
    assert_eq!(loaded, original);
    assert_eq!(Hash128::of(&loaded), Hash128::of(&original));
    assert_eq!(
        original.schedule(U, Dur::ZERO, wake(0)),
        loaded.schedule(U, Dur::ZERO, wake(0))
    );
}

#[test]
fn the_state_hash_follows_the_state() {
    let base = busy_timeline();
    let hash = Hash128::of(&base);
    assert_eq!(Hash128::of(&busy_timeline()), hash);
    assert_eq!(Hash128::of(&base.clone()), hash);

    let mut moved = base.clone();
    moved.advance_to(moved.now() + Dur(1));
    assert_ne!(Hash128::of(&moved), hash);

    let mut scheduled = base.clone();
    let id = scheduled.schedule(U, secs(1), wake(0)).unwrap();
    assert_ne!(Hash128::of(&scheduled), hash);
    // Cancelling does not restore the hash: the sequence counter moved on,
    // and that is state, because it decides future ids and tie-breaks.
    scheduled.cancel(id);
    assert_ne!(Hash128::of(&scheduled), hash);

    let mut slowed = base.clone();
    slowed.set_rate(ClockId::new(1), r(0.31)).unwrap();
    assert_ne!(Hash128::of(&slowed), hash);

    let mut payload = base.clone();
    let first = payload.wakes_on(U).next().unwrap();
    payload.rehome(first, ClockId::new(1)).unwrap();
    assert_ne!(Hash128::of(&payload), hash);
}

#[test]
fn a_new_timeline_is_empty_at_tick_zero() {
    let timeline = Timeline::default();
    assert_eq!(timeline, Timeline::new());
    assert_eq!(timeline.now(), Tick::ZERO);
    assert_eq!(timeline.peek(), None);
    assert_eq!(timeline.wake_count(), 0);
    assert_eq!(timeline.clocks().collect::<Vec<_>>(), [U]);
    assert_eq!(timeline.rate(U), Some(Real::ONE));
    assert_eq!(timeline.effective_rate(U), Some(Real::ONE));
    assert_eq!(timeline.parent(U), None);
    assert!(timeline.has_clock(U));
}

#[test]
fn corrupt_saves_are_refused_with_a_reason() {
    let good = serde_json::to_value(busy_timeline()).unwrap();
    assert!(serde_json::from_value::<Timeline>(good.clone()).is_ok());

    let break_it = |edit: &dyn Fn(&mut serde_json::Value)| -> String {
        let mut bad = good.clone();
        edit(&mut bad);
        serde_json::from_value::<Timeline>(bad)
            .expect_err("a corrupt save must be refused")
            .to_string()
    };

    let message = break_it(&|save| save["clocks"][1]["parent"] = 77.into());
    assert!(message.contains("unknown parent"), "{message}");
    let message = break_it(&|save| save["clocks"][1]["parent"] = 1.into());
    assert!(message.contains("do not form a tree"), "{message}");
    let message = break_it(&|save| save["clocks"][1]["rate"] = (-2.0).into());
    assert!(message.contains("rate must be zero or more"), "{message}");
    let message = break_it(&|save| save["clocks"][0]["rate"] = 2.0.into());
    assert!(message.contains("universal clock's rate"), "{message}");
    let message = break_it(&|save| save["clocks"][1]["frac"] = 1.5.into());
    assert!(message.contains("bad fraction"), "{message}");
    let message = break_it(&|save| {
        let first = save["clocks"][1].clone();
        save["clocks"].as_array_mut().unwrap().push(first);
    });
    assert!(message.contains("appears twice"), "{message}");
    let message = break_it(&|save| {
        save["clocks"].as_array_mut().unwrap().remove(0);
    });
    assert!(
        message.contains("wrong kind of parent") || message.contains("missing"),
        "{message}"
    );
    let message = break_it(&|save| save["wakes"][0]["clock"] = 77.into());
    assert!(message.contains("unknown clock"), "{message}");
    let message = break_it(&|save| save["wakes"][0]["remaining"] = (-1.0).into());
    assert!(message.contains("negative time left"), "{message}");
    let message = break_it(&|save| save["wakes"][0]["due"] = 0.into());
    assert!(message.contains("out of step"), "{message}");
    let message = break_it(&|save| save["next_wake"] = 0.into());
    assert!(message.contains("beyond the next wake-up id"), "{message}");
    let message = break_it(&|save| save["next_clock"] = 1.into());
    assert!(message.contains("beyond the next clock id"), "{message}");
}
