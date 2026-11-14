//! Long waits a player can stop (12 §3.2, D-931): `shift+p` waits an
//! hour, carried over many passes of the loop; the clock in the status
//! column moves on pass by pass; any key breaks the wait off through
//! the simulation's own command (`Interrupt`), and the player is asked
//! at once.

mod common;

use std::time::Instant;

use clash_core::Tick;
use clash_scene::InputEvent;
use clash_sim::Command;

use common::*;

/// The simulation's time and whether the player's character is in a
/// wait it was told to make.
fn waiting(run: &clash_app::headless::Headless) -> (Tick, bool, bool) {
    run.game()
        .with_session(|session| {
            (
                session.sim.now(),
                session.sim.is_waiting(session.player),
                session.sim.pending_decision().is_some(),
            )
        })
        .expect("a game is being played")
}

/// The time the status column shows, as its line reads.
fn clock(run: &clash_app::headless::Headless) -> String {
    run.text()
        .lines()
        .find_map(|line| {
            let at = line.find("Time ")?;
            Some(line[at..].split_whitespace().take(2).collect::<Vec<_>>().join(" "))
        })
        .unwrap_or_default()
}

#[test]
fn a_long_wait_moves_the_clock_and_any_key_breaks_it_off() {
    let data = data_dir("waiting");
    let mut run = boot_default(&data);
    begin(&mut run, 2, "Ada");
    settle(&mut run);
    let (began, waits, asked) = waiting(&run);
    assert!(!waits && asked, "the character is asked what it does");
    let shown = clock(&run);
    assert!(shown.starts_with("Time "), "the status column has a clock: {shown:?}");

    // An hour's wait, the simulation advanced a slice at a time as the
    // terminal's loop does: a pass whose slice is already used advances
    // it by one call.
    let key = InputEvent::key("shift+p").unwrap();
    assert!(run.input_until(key, Instant::now()));
    let (_, waits, _) = waiting(&run);
    assert!(waits, "shift+p waits");
    let mut seen = vec![clock(&run)];
    let mut last = waiting(&run).0;
    for _ in 0..40 {
        assert!(run.pass_until(Instant::now()));
        let (now, waits, asked) = waiting(&run);
        assert!(waits && !asked, "the wait goes on over many passes");
        assert!(now >= last, "time does not go back");
        last = now;
        let shown = clock(&run);
        if seen.last() != Some(&shown) {
            seen.push(shown);
        }
    }
    assert!(last > began, "time passed");
    assert!(
        seen.len() >= 3,
        "the clock in the status column moved on as the wait went on: {seen:?}"
    );

    // Any key breaks it off: the command is the simulation's own, so
    // the input log replays it, and the key does nothing else.
    let commands_before = commands(&run);
    let stopped_at = waiting(&run).0;
    assert!(run.input_until(InputEvent::key("i").unwrap(), Instant::now()));
    assert_eq!(commands(&run), commands_before + 1);
    let player = run.game().with_session(|session| session.player).unwrap();
    assert_eq!(last_command(&run), Command::Interrupt { entity: player });
    let (now, waits, asked) = waiting(&run);
    assert!(!waits && asked, "the wait is over and the player is asked");
    assert_eq!(now, stopped_at, "no more time passed");
    assert_eq!(top(&run), "ui:play", "the key opened nothing");
    for _ in 0..5 {
        assert!(run.pass_until(Instant::now()));
    }
    assert_eq!(waiting(&run).0, stopped_at, "time stands still while the player is asked");
    let _ = std::fs::remove_dir_all(&data);
}
