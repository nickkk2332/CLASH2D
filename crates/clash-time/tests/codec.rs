//! The timeline and the process table through the binary encoding of
//! `clash_core::codec`: what a save holds must come back exactly.

use clash_core::codec::{decode_blob, encode_blob, from_bytes, to_bytes};
use clash_core::{CodecError, Dur, Hash128, Real, Tick};
use clash_time::{ClockId, Next, ProcessTable, Timeline, Wake, WakeDomain, WakeId};
use proptest::prelude::*;

const KIND: [u8; 4] = *b"TIME";

/// A timeline with a tree of clocks at different rates, a stopped clock
/// with parked wake-ups, and time already passed.
fn busy_timeline() -> Timeline {
    let mut timeline = Timeline::new();
    let slow = timeline
        .create_clock(ClockId::UNIVERSAL, Real::of(0.3))
        .unwrap();
    let fast = timeline.create_clock(slow, Real::of(7.0)).unwrap();
    let stopped = timeline
        .create_clock(ClockId::UNIVERSAL, Real::ZERO)
        .unwrap();
    for (index, clock) in [ClockId::UNIVERSAL, slow, fast, stopped, fast, slow]
        .into_iter()
        .enumerate()
    {
        let wake = Wake::new(WakeDomain::Timer, index as u64, 100 + index as u64);
        timeline
            .schedule(clock, Dur::from_millis(250 * (index as i64 + 1)), wake)
            .unwrap();
    }
    timeline.pop(Tick(400_000));
    timeline.advance_to(Tick(777_777));
    timeline.set_rate(slow, Real::of(1.7)).unwrap();
    timeline
}

/// Pops everything that will ever fire.
fn drain(timeline: &mut Timeline) -> Vec<(Tick, WakeId, Wake)> {
    let mut fired = Vec::new();
    while let Some((id, wake)) = timeline.pop(Tick::MAX) {
        fired.push((timeline.now(), id, wake));
    }
    fired
}

#[test]
fn a_timeline_round_trips_through_the_codec() {
    let original = busy_timeline();
    let bytes = to_bytes(&original).unwrap();
    let mut loaded: Timeline = from_bytes(&bytes).unwrap();

    assert_eq!(loaded, original);
    assert_eq!(Hash128::of(&loaded), Hash128::of(&original));
    assert_eq!(to_bytes(&loaded).unwrap(), bytes);

    // The binary form and the JSON form hold the same timeline.
    let json = serde_json::to_string(&original).unwrap();
    let from_json: Timeline = serde_json::from_str(&json).unwrap();
    assert_eq!(from_json, loaded);
    assert!(bytes.len() * 3 < json.len(), "{} bytes", bytes.len());

    // And it goes on to do exactly what the original does.
    let mut original = original;
    let fired = drain(&mut loaded);
    assert!(!fired.is_empty());
    assert_eq!(fired, drain(&mut original));
    assert_eq!(Hash128::of(&loaded), Hash128::of(&original));
}

#[test]
fn an_empty_timeline_round_trips() {
    let empty = Timeline::new();
    let bytes = to_bytes(&empty).unwrap();
    assert_eq!(from_bytes::<Timeline>(&bytes), Ok(empty));
}

#[test]
fn a_timeline_round_trips_in_a_blob() {
    let original = busy_timeline();
    let blob = encode_blob(KIND, 1, &original).unwrap();
    assert_eq!(decode_blob::<Timeline>(KIND, &blob), Ok((1, original)));
}

#[test]
fn a_timeline_that_could_not_exist_is_refused() {
    // The timeline checks what it is given, whichever format carried it:
    // here there are no clocks at all, not even the universal one.
    let no_clocks = to_bytes(&(Tick(0), 0_u64, 1_u32, Vec::<u8>::new(), Vec::<u8>::new())).unwrap();
    assert_eq!(
        from_bytes::<Timeline>(&no_clocks),
        Err(CodecError::Custom(
            "the universal clock is missing".to_owned()
        ))
    );
    // Cut short anywhere, it is an error and never a panic.
    let good = to_bytes(&busy_timeline()).unwrap();
    for length in 0..good.len() {
        assert!(from_bytes::<Timeline>(&good[..length]).is_err());
    }
}

#[test]
fn a_process_table_round_trips_through_the_codec() {
    let mut timeline = Timeline::new();
    let mut table: ProcessTable<(u32, String)> = ProcessTable::new(WakeDomain::World);
    let first = table
        .spawn(
            &mut timeline,
            ClockId::UNIVERSAL,
            Dur::SECOND,
            (3, "fire".to_owned()),
        )
        .unwrap();
    table
        .spawn(
            &mut timeline,
            ClockId::UNIVERSAL,
            Dur::from_secs(2),
            (1, "blast".to_owned()),
        )
        .unwrap();
    timeline.pop(Tick::MAX);
    table
        .wake(&mut timeline, first, |state: &mut (u32, String)| {
            state.0 -= 1;
            Next::Sleep(Dur::SECOND)
        })
        .unwrap();

    let bytes = to_bytes(&(&timeline, &table)).unwrap();
    let (loaded_timeline, loaded_table): (Timeline, ProcessTable<(u32, String)>) =
        from_bytes(&bytes).unwrap();
    assert_eq!(loaded_timeline, timeline);
    assert_eq!(loaded_table, table);
    assert_eq!(loaded_table.get(first), Some(&(2, "fire".to_owned())));
    assert_eq!(to_bytes(&(&loaded_timeline, &loaded_table)).unwrap(), bytes);
}

proptest! {
    #[test]
    fn any_timeline_round_trips(
        rates in prop::collection::vec(0_u32..40, 0..6),
        wakes in prop::collection::vec((0_usize..8, 0_i64..5_000_000, any::<u64>()), 0..24),
        stop in 0_i64..6_000_000,
    ) {
        let mut timeline = Timeline::new();
        let mut clocks = vec![ClockId::UNIVERSAL];
        for (index, rate) in rates.iter().enumerate() {
            let parent = clocks[index / 2];
            let rate = Real::from(*rate) / Real::from(8_u8);
            clocks.push(timeline.create_clock(parent, rate).unwrap());
        }
        for (clock, delay, arg) in wakes {
            let clock = clocks[clock % clocks.len()];
            let wake = Wake::new(WakeDomain::Action, arg, !arg);
            timeline.schedule(clock, Dur(delay), wake).unwrap();
        }
        while timeline.pop(Tick(stop)).is_some() {}
        timeline.advance_to(Tick(stop));

        let bytes = to_bytes(&timeline).unwrap();
        let mut loaded: Timeline = from_bytes(&bytes).unwrap();
        prop_assert_eq!(&loaded, &timeline);
        prop_assert_eq!(Hash128::of(&loaded), Hash128::of(&timeline));
        prop_assert_eq!(to_bytes(&loaded).unwrap(), bytes);
        prop_assert_eq!(drain(&mut loaded), drain(&mut timeline));
    }
}
