//! The process table: spawning, waking, sleeping, finishing,
//! force-resolving, moving between clocks, saving and hashing.

use clash_core::{Dur, Hash128, Real, StateHash, StateHasher, Tick};
use clash_time::{ClockId, Next, ProcessId, ProcessTable, TimeError, Timeline, Wake, WakeDomain};
use serde::{Deserialize, Serialize};

const U: ClockId = ClockId::UNIVERSAL;

/// A stand-in for a world process: something that expands step by step
/// and stops at a limit.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
struct Wave {
    radius: u32,
    limit: u32,
}

impl StateHash for Wave {
    fn state_hash(&self, hasher: &mut StateHasher) {
        hasher.write_u32(self.radius);
        hasher.write_u32(self.limit);
    }
}

impl Wave {
    fn new(limit: u32) -> Wave {
        Wave { radius: 0, limit }
    }

    /// One piece of work: grow by one, then sleep a second or finish.
    fn expand(&mut self) -> Next {
        self.radius += 1;
        if self.radius >= self.limit {
            Next::Done
        } else {
            Next::Sleep(Dur::SECOND)
        }
    }
}

fn at(secs: i64) -> Tick {
    Tick::ZERO + Dur::from_secs(secs)
}

/// Pops wake-ups up to `limit` and runs the processes they name. Returns
/// the id and tick of each wake.
fn run(
    timeline: &mut Timeline,
    waves: &mut ProcessTable<Wave>,
    limit: Tick,
) -> Vec<(ProcessId, Tick)> {
    let mut woken = Vec::new();
    while let Some((_, wake)) = timeline.pop(limit) {
        let id = waves
            .process_of(&wake)
            .expect("the wake-up names a process");
        waves.wake(timeline, id, Wave::expand).unwrap();
        woken.push((id, timeline.now()));
    }
    woken
}

#[test]
fn a_process_wakes_works_sleeps_and_finishes() {
    let mut timeline = Timeline::new();
    let mut waves = ProcessTable::new(WakeDomain::World);
    assert!(waves.is_empty());
    assert_eq!(waves.domain(), WakeDomain::World);

    let id = waves
        .spawn(&mut timeline, U, Dur::SECOND, Wave::new(3))
        .unwrap();
    assert_eq!(waves.len(), 1);
    assert!(waves.contains(id));
    assert_eq!(waves.get(id), Some(&Wave::new(3)));
    assert_eq!(waves.clock_of(id), Some(U));
    let first_wake = waves.wake_of(id).expect("a spawned process has a wake-up");
    assert_eq!(timeline.due(first_wake), Some(at(1)));
    assert_eq!(
        timeline.wake(first_wake),
        Some(Wake::new(WakeDomain::World, u64::from(id.raw()), 0))
    );

    // First wake: grows, asks to sleep.
    let (_, wake) = timeline.pop(Tick::MAX).unwrap();
    assert_eq!(waves.process_of(&wake), Some(id));
    assert_eq!(
        waves.wake(&mut timeline, id, Wave::expand),
        Ok(Some(Next::Sleep(Dur::SECOND)))
    );
    assert_eq!(waves.get(id).map(|wave| wave.radius), Some(1));
    let second_wake = waves.wake_of(id).unwrap();
    assert_ne!(second_wake, first_wake);
    assert_eq!(timeline.due(second_wake), Some(at(2)));

    // It runs to its end and is forgotten, leaving nothing scheduled.
    assert_eq!(
        run(&mut timeline, &mut waves, Tick::MAX),
        [(id, at(2)), (id, at(3))]
    );
    assert!(waves.is_empty());
    assert!(!waves.contains(id));
    assert_eq!(waves.get(id), None);
    assert_eq!(timeline.wake_count(), 0);
}

#[test]
fn several_processes_interleave_in_time_order() {
    let mut timeline = Timeline::new();
    let mut waves = ProcessTable::new(WakeDomain::World);
    let slow = timeline.create_clock(U, Real::of(0.5)).unwrap();
    let a = waves
        .spawn(&mut timeline, U, Dur::SECOND, Wave::new(2))
        .unwrap();
    let b = waves
        .spawn(&mut timeline, slow, Dur::SECOND, Wave::new(2))
        .unwrap();
    let c = waves
        .spawn(&mut timeline, U, Dur::from_millis(500), Wave::new(1))
        .unwrap();
    assert!(a < b && b < c);
    assert_eq!(waves.ids().collect::<Vec<_>>(), [a, b, c]);

    // b's seconds take two universal seconds each. At 2 s both a and b
    // are due: b's wake-up was scheduled first (when b was spawned), a's
    // only when a went back to sleep at 1 s, so b runs first.
    assert_eq!(
        run(&mut timeline, &mut waves, Tick::MAX),
        [
            (c, Tick(500_000)),
            (a, at(1)),
            (b, at(2)),
            (a, at(2)),
            (b, at(4))
        ]
    );
    assert!(waves.is_empty());
}

#[test]
fn a_wake_up_for_another_domain_or_a_gone_process_is_not_ours() {
    let mut timeline = Timeline::new();
    let mut waves = ProcessTable::new(WakeDomain::World);
    let id = waves
        .spawn(&mut timeline, U, Dur::SECOND, Wave::new(1))
        .unwrap();
    let raw = u64::from(id.raw());

    assert_eq!(
        waves.process_of(&Wake::new(WakeDomain::World, raw, 0)),
        Some(id)
    );
    assert_eq!(
        waves.process_of(&Wake::new(WakeDomain::Timer, raw, 0)),
        None
    );
    assert_eq!(waves.process_of(&Wake::new(WakeDomain::World, 77, 0)), None);
    assert_eq!(
        waves.process_of(&Wake::new(WakeDomain::World, u64::MAX, 0)),
        None
    );

    // Waking a process that does not exist does nothing and says so.
    let ghost = ProcessId::new(77);
    assert_eq!(waves.wake(&mut timeline, ghost, Wave::expand), Ok(None));
    assert_eq!(waves.len(), 1);
}

#[test]
fn a_process_can_be_run_before_its_wake_up_fires() {
    let mut timeline = Timeline::new();
    let mut waves = ProcessTable::new(WakeDomain::World);
    let id = waves
        .spawn(&mut timeline, U, Dur::from_secs(10), Wave::new(5))
        .unwrap();
    // Run it now: the pending wake-up is cancelled and replaced.
    waves.wake(&mut timeline, id, Wave::expand).unwrap();
    assert_eq!(timeline.wake_count(), 1);
    assert_eq!(timeline.peek(), Some(at(1)));
    assert_eq!(waves.get(id).map(|wave| wave.radius), Some(1));
}

/// State that holds the table its processes must change: the reason for
/// `take` and `settle`.
struct World {
    energy_released: u32,
    waves: ProcessTable<Wave>,
}

impl World {
    fn wake(&mut self, timeline: &mut Timeline, id: ProcessId) {
        let Some(mut wave) = self.waves.take(timeline, id) else {
            return;
        };
        // While the process is out it keeps its place but has no state.
        assert!(self.waves.contains(id));
        assert_eq!(self.waves.get(id), None);
        assert_eq!(self.waves.wake_of(id), None);
        assert_eq!(self.waves.iter().count(), self.waves.len() - 1);

        let next = wave.expand();
        self.energy_released += wave.radius;
        self.waves.settle(timeline, id, wave, next).unwrap();
    }
}

#[test]
fn take_and_settle_run_a_process_against_its_owner() {
    let mut timeline = Timeline::new();
    let mut world = World {
        energy_released: 0,
        waves: ProcessTable::new(WakeDomain::World),
    };
    let id = world
        .waves
        .spawn(&mut timeline, U, Dur::SECOND, Wave::new(3))
        .unwrap();

    while let Some((_, wake)) = timeline.pop(Tick::MAX) {
        let id = world.waves.process_of(&wake).unwrap();
        world.wake(&mut timeline, id);
    }
    assert_eq!(world.energy_released, 1 + 2 + 3);
    assert!(world.waves.is_empty());
    assert_eq!(timeline.now(), at(3));
    // A wake for a finished process is ignored.
    world.wake(&mut timeline, id);
    assert_eq!(world.energy_released, 6);
}

#[test]
fn take_is_refused_twice_and_settle_needs_a_taken_process() {
    let mut timeline = Timeline::new();
    let mut waves = ProcessTable::new(WakeDomain::World);
    let id = waves
        .spawn(&mut timeline, U, Dur::SECOND, Wave::new(3))
        .unwrap();

    // Settling a process that was never taken is refused.
    assert_eq!(
        waves.settle(&mut timeline, id, Wave::new(9), Next::Done),
        Err(TimeError::UnknownProcess(id))
    );
    assert_eq!(waves.get(id), Some(&Wave::new(3)));

    let wave = waves.take(&mut timeline, id).unwrap();
    assert_eq!(timeline.wake_count(), 0);
    assert_eq!(waves.take(&mut timeline, id), None);
    // In-place waking is refused too while it is out.
    assert_eq!(waves.wake(&mut timeline, id, Wave::expand), Ok(None));

    waves
        .settle(&mut timeline, id, wave, Next::Sleep(Dur::from_secs(2)))
        .unwrap();
    assert_eq!(timeline.peek(), Some(at(2)));
    assert_eq!(waves.get(id), Some(&Wave::new(3)));

    // Unknown ids.
    let ghost = ProcessId::new(77);
    assert_eq!(waves.take(&mut timeline, ghost), None);
    assert_eq!(
        waves.settle(&mut timeline, ghost, Wave::new(1), Next::Done),
        Err(TimeError::UnknownProcess(ghost))
    );
}

#[test]
fn remove_force_resolves_a_process() {
    let mut timeline = Timeline::new();
    let mut waves = ProcessTable::new(WakeDomain::World);
    let doomed = waves
        .spawn(&mut timeline, U, Dur::SECOND, Wave::new(100))
        .unwrap();
    let other = waves
        .spawn(&mut timeline, U, Dur::SECOND, Wave::new(2))
        .unwrap();
    run(&mut timeline, &mut waves, at(1));

    // The owner takes the state and finishes it at once, in its own way.
    let mut wave = waves.remove(&mut timeline, doomed).unwrap();
    while wave.expand() != Next::Done {}
    assert_eq!(wave.radius, 100);

    // Nothing is left of it: no slot, no wake-up.
    assert!(!waves.contains(doomed));
    assert_eq!(waves.remove(&mut timeline, doomed), None);
    assert_eq!(timeline.wake_count(), 1);
    assert_eq!(run(&mut timeline, &mut waves, Tick::MAX), [(other, at(2))]);
}

#[test]
fn removing_a_process_while_it_is_out_makes_its_settle_fail() {
    let mut timeline = Timeline::new();
    let mut waves = ProcessTable::new(WakeDomain::World);
    let id = waves
        .spawn(&mut timeline, U, Dur::SECOND, Wave::new(3))
        .unwrap();
    let wave = waves.take(&mut timeline, id).unwrap();
    assert_eq!(waves.remove(&mut timeline, id), None);
    assert!(waves.is_empty());
    assert_eq!(
        waves.settle(&mut timeline, id, wave, Next::Sleep(Dur::SECOND)),
        Err(TimeError::UnknownProcess(id))
    );
    assert_eq!(timeline.wake_count(), 0);
}

#[test]
fn a_process_on_a_stopped_clock_waits_and_then_carries_on() {
    let mut timeline = Timeline::new();
    let mut waves = ProcessTable::new(WakeDomain::World);
    let field = timeline.create_clock(U, Real::ONE).unwrap();
    let id = waves
        .spawn(&mut timeline, field, Dur::SECOND, Wave::new(3))
        .unwrap();
    assert_eq!(run(&mut timeline, &mut waves, at(1)), [(id, at(1))]);

    // Time stops in the field: the wave hangs where it is.
    timeline.advance_to(Tick(1_500_000));
    timeline.set_rate(field, Real::ZERO).unwrap();
    assert!(run(&mut timeline, &mut waves, at(100)).is_empty());
    timeline.advance_to(at(100));
    assert_eq!(waves.get(id).map(|wave| wave.radius), Some(1));

    // It starts again with the half second it had left.
    timeline.set_rate(field, Real::ONE).unwrap();
    assert_eq!(
        run(&mut timeline, &mut waves, Tick::MAX),
        [(id, Tick(100_500_000)), (id, Tick(101_500_000))]
    );
}

#[test]
fn rehoming_moves_the_process_and_its_wake_up() {
    let mut timeline = Timeline::new();
    let mut waves = ProcessTable::new(WakeDomain::World);
    let slow = timeline.create_clock(U, Real::of(0.25)).unwrap();
    let id = waves
        .spawn(&mut timeline, U, Dur::from_secs(4), Wave::new(3))
        .unwrap();

    timeline.advance_to(at(2));
    waves.rehome(&mut timeline, id, slow).unwrap();
    assert_eq!(waves.clock_of(id), Some(slow));
    let wake = waves.wake_of(id).unwrap();
    assert_eq!(timeline.clock_of(wake), Some(slow));
    assert_eq!(timeline.remaining(wake), Some(Dur::from_secs(2)));

    // 2 s left at a quarter speed, then each later second takes four.
    assert_eq!(
        run(&mut timeline, &mut waves, Tick::MAX),
        [(id, at(10)), (id, at(14)), (id, at(18))]
    );

    let ghost_clock = ClockId::new(99);
    let ghost = ProcessId::new(77);
    assert_eq!(
        waves.rehome(&mut timeline, ghost, slow),
        Err(TimeError::UnknownProcess(ghost))
    );
    let again = waves
        .spawn(&mut timeline, U, Dur::SECOND, Wave::new(3))
        .unwrap();
    assert_eq!(
        waves.rehome(&mut timeline, again, ghost_clock),
        Err(TimeError::UnknownClock(ghost_clock))
    );
    assert_eq!(waves.clock_of(again), Some(U));
}

#[test]
fn spawning_on_an_unknown_clock_is_refused_and_uses_no_id() {
    let mut timeline = Timeline::new();
    let mut waves = ProcessTable::new(WakeDomain::World);
    let ghost = ClockId::new(99);
    assert_eq!(
        waves.spawn(&mut timeline, ghost, Dur::SECOND, Wave::new(1)),
        Err(TimeError::UnknownClock(ghost))
    );
    assert!(waves.is_empty());
    let id = waves
        .spawn(&mut timeline, U, Dur::SECOND, Wave::new(1))
        .unwrap();
    assert_eq!(id, ProcessId::new(0));
}

#[test]
fn a_process_whose_clock_was_destroyed_under_it_is_left_for_its_owner() {
    let mut timeline = Timeline::new();
    let mut waves = ProcessTable::new(WakeDomain::World);
    let bubble = timeline.create_clock(U, Real::ONE).unwrap();
    let id = waves
        .spawn(&mut timeline, bubble, Dur::SECOND, Wave::new(5))
        .unwrap();

    // The owner should have rehomed the process first. The timeline keeps
    // the wake-up alive on the parent clock, so the process still wakes.
    timeline.destroy_clock(bubble).unwrap();
    let (_, wake) = timeline.pop(Tick::MAX).unwrap();
    assert_eq!(waves.process_of(&wake), Some(id));

    // But it cannot sleep again on a clock that is gone. The error says
    // so, and the process stays, unscheduled, with its work done.
    assert_eq!(
        waves.wake(&mut timeline, id, Wave::expand),
        Err(TimeError::UnknownClock(bubble))
    );
    assert_eq!(waves.get(id).map(|wave| wave.radius), Some(1));
    assert_eq!(waves.wake_of(id), None);
    assert_eq!(timeline.wake_count(), 0);

    // The owner can put it on a clock that exists and run it again, or
    // remove it and resolve it.
    waves.rehome(&mut timeline, id, U).unwrap();
    waves.wake(&mut timeline, id, Wave::expand).unwrap();
    assert_eq!(waves.get(id).map(|wave| wave.radius), Some(2));
    assert!(waves.wake_of(id).is_some());
    assert_eq!(
        waves.remove(&mut timeline, id).map(|wave| wave.radius),
        Some(2)
    );
    assert_eq!(timeline.wake_count(), 0);
}

#[test]
fn ids_are_not_reused_and_iteration_is_in_id_order() {
    let mut timeline = Timeline::new();
    let mut waves = ProcessTable::new(WakeDomain::World);
    let a = waves
        .spawn(&mut timeline, U, Dur::SECOND, Wave::new(1))
        .unwrap();
    let b = waves
        .spawn(&mut timeline, U, Dur::SECOND, Wave::new(2))
        .unwrap();
    waves.remove(&mut timeline, a);
    let c = waves
        .spawn(&mut timeline, U, Dur::SECOND, Wave::new(3))
        .unwrap();
    assert_ne!(c, a);
    assert!(b < c);
    let limits: Vec<(ProcessId, u32)> = waves.iter().map(|(id, wave)| (id, wave.limit)).collect();
    assert_eq!(limits, [(b, 2), (c, 3)]);

    if let Some(wave) = waves.get_mut(b) {
        wave.limit = 20;
    }
    assert_eq!(waves.get(b).map(|wave| wave.limit), Some(20));
}

#[test]
fn the_table_saves_loads_and_hashes_with_the_timeline() {
    let mut timeline = Timeline::new();
    let mut waves = ProcessTable::new(WakeDomain::World);
    let slow = timeline.create_clock(U, Real::of(0.3)).unwrap();
    waves
        .spawn(&mut timeline, U, Dur::SECOND, Wave::new(4))
        .unwrap();
    waves
        .spawn(&mut timeline, slow, Dur::from_millis(700), Wave::new(3))
        .unwrap();
    let gone = waves
        .spawn(&mut timeline, U, Dur::SECOND, Wave::new(9))
        .unwrap();
    run(&mut timeline, &mut waves, at(2));
    waves.remove(&mut timeline, gone);

    let saved = serde_json::to_string(&(&timeline, &waves)).unwrap();
    let (mut loaded_timeline, mut loaded_waves): (Timeline, ProcessTable<Wave>) =
        serde_json::from_str(&saved).unwrap();
    assert_eq!(loaded_waves, waves);
    assert_eq!(Hash128::of(&loaded_waves), Hash128::of(&waves));
    assert_eq!(Hash128::of(&loaded_timeline), Hash128::of(&timeline));

    // Both carry on to the same end by the same steps, and new processes
    // get the same ids.
    let original_run = run(&mut timeline, &mut waves, Tick::MAX);
    let loaded_run = run(&mut loaded_timeline, &mut loaded_waves, Tick::MAX);
    assert_eq!(original_run, loaded_run);
    assert!(original_run.len() >= 4);
    assert_eq!(
        waves.spawn(&mut timeline, U, Dur::SECOND, Wave::new(1)),
        loaded_waves.spawn(&mut loaded_timeline, U, Dur::SECOND, Wave::new(1))
    );
}

#[test]
fn the_hash_follows_the_processes() {
    let mut timeline = Timeline::new();
    let mut waves = ProcessTable::new(WakeDomain::World);
    let empty = Hash128::of(&waves);
    assert_ne!(
        empty,
        Hash128::of(&ProcessTable::<Wave>::new(WakeDomain::Timer))
    );

    let id = waves
        .spawn(&mut timeline, U, Dur::SECOND, Wave::new(4))
        .unwrap();
    let spawned = Hash128::of(&waves);
    assert_ne!(spawned, empty);

    if let Some(wave) = waves.get_mut(id) {
        wave.radius = 2;
    }
    let grown = Hash128::of(&waves);
    assert_ne!(grown, spawned);

    let slow = timeline.create_clock(U, Real::of(0.5)).unwrap();
    waves.rehome(&mut timeline, id, slow).unwrap();
    let moved = Hash128::of(&waves);
    assert_ne!(moved, grown);

    // Removing it does not bring back the empty hash: the id counter is
    // state, because it decides the next process's id.
    waves.remove(&mut timeline, id);
    assert_ne!(Hash128::of(&waves), empty);
    assert!(waves.is_empty());
}
