//! Play session: the last stand (13 §11), from the title, as a player
//! plays it. Three strangers come down a dark lane for the character;
//! one lamp burns by the door of a house. In the first session the
//! character gets inside, shuts the door, leaves and carries on from the
//! title (the game's one save), and waits out the night with the long
//! wait until dawn ends it. In the second the character stays on the
//! lane and fights, and the night ends the other way.
//!
//! The golden frames are written with `BLESS=1 cargo test -p clash-app`.

mod session;

use clash_app::headless::Headless;
use clash_scene::assert_golden;
use clash_world::Location;
use session::*;

/// The last stand is the first game of the bundled list.
const LAST_STAND: usize = 0;

/// From the title to the lane, past the introduction.
fn begin(run: &mut Headless) {
    new_game(run, LAST_STAND, "Ada");
    until_top(run, "ui:intro", 60);
    let intro = run.text();
    assert!(intro.contains("It is ten at night"), "{intro}");
    press(run, "enter");
    settle(run);
    assert_eq!(top(run), "ui:play");
}

/// The door the journal points at.
fn refuge(run: &Headless) -> Location {
    let value = global(run, "core:refuge");
    run.game()
        .with_session(|session| match session.sim.subject_of(&value) {
            Some(clash_sim::Subject::Loc(at)) => at,
            other => panic!("the refuge is no cell: {other:?}"),
        })
        .unwrap()
}

/// Answers whatever the game asks on top of play: a blow coming is met
/// with the first reaction offered. Returns what was on top.
fn answer(run: &mut Headless) -> String {
    let on_top = top(run);
    match on_top.as_str() {
        "ui:reaction" => press(run, "enter"),
        "ui:play" | "ui:summary" => {}
        other => panic!("`{other}` is on top:\n{}", run.text()),
    }
    on_top
}

/// Through the door and in, walking into it to open it, and the door
/// shut behind, answering any blow on the way with the first reaction.
fn get_inside(run: &mut Headless) {
    let door = refuge(run);
    for _ in 0..40 {
        if answer(run) == "ui:play" {
            if player_loc(run).unwrap().cell == door.cell {
                break;
            }
            walk_to(run, door, 0, 1);
        }
    }
    let inside = Location::new(door.grid, door.cell + clash_core::IVec3::new(0, -1, 0));
    for _ in 0..10 {
        answer(run);
        if player_loc(run).unwrap().cell == inside.cell {
            break;
        }
        press(run, "k");
    }
    assert_eq!(player_loc(run).unwrap().cell, inside.cell, "{}", run.text());
    assert!(use_near(run, "Close"), "{}", run.text());
    until_asked(run, 60);
}

#[test]
fn inside_with_the_door_shut_the_night_goes_by() {
    let data = DataDir::new("stand-held");
    let mut run = boot_default(data.path(), 4);
    begin(&mut run);
    let shown = run.text();
    assert!(shown.contains("Time 22:00"), "{shown}");
    // The lamp by the door lights the lane: the house's front is seen.
    assert!(shown.contains("###X##"), "{shown}");
    assert_golden(golden("play_stand_lane"), run.frame());

    press(&mut run, "shift+j");
    let journal = run.text();
    assert!(
        journal.contains("[ ] Get inside the house.  north-east, 3 m")
            && journal.contains("[ ] Hold out until dawn."),
        "{journal}"
    );
    press(&mut run, "escape");

    get_inside(&mut run);
    press(&mut run, "shift+j");
    let journal = run.text();
    assert!(journal.contains("[x] Get inside the house."), "{journal}");
    press(&mut run, "escape");

    // Left, and carried on: a game of one save keeps it as it is left.
    let hash = state_hash(&run);
    quit_to_title(&mut run);
    assert_eq!(saves(&run), ["ada"]);
    press_button(&mut run, "Continue");
    assert!(run.game().playing());
    assert_eq!(state_hash(&run), hash, "a load changes nothing");

    // Time goes by behind the shut door.
    let before = now_s(&run);
    for _ in 0..3 {
        press(&mut run, "w");
        answer(&mut run);
    }
    assert!(now_s(&run) - before > 60.0, "time goes by");
    assert_eq!(top(&run), "ui:play");
    assert!(run.game().playing());
}

/// The whole night: many minutes in a debug build, under a minute in
/// release (`docs/playtest-beta.md`), so it is run by hand:
/// `cargo test -p clash-app --release --test play_last_stand -- --ignored`.
#[test]
#[ignore = "eight hours of play: too slow for the check in a debug build"]
fn inside_with_the_door_shut_the_night_is_waited_out_until_dawn() {
    let data = DataDir::new("stand-dawn");
    let mut run = boot_default(data.path(), 4);
    begin(&mut run);
    get_inside(&mut run);
    for _ in 0..40 {
        if answer(&mut run) == "ui:summary" {
            break;
        }
        press(&mut run, "shift+w");
    }
    until_top(&mut run, "ui:summary", 60);
    let summary = run.text();
    assert!(
        summary.contains("Survived.") && summary.contains("[x] Hold out until dawn."),
        "{summary}"
    );
    press(&mut run, "enter");
    assert_eq!(top(&run), "ui:title");
    assert!(saves(&run).is_empty(), "an ending takes the one save");
}

#[test]
fn standing_on_the_lane_the_night_ends_the_other_way() {
    let data = DataDir::new("stand-lost");
    let mut run = boot_default(data.path(), 4);
    begin(&mut run);
    // Ada stays where she is and hits back at whoever comes; a blow
    // coming is let happen.
    for i in 0..400 {
        let on_top = top(&run);
        if on_top == "ui:summary" {
            break;
        }
        match on_top.as_str() {
            "ui:reaction" => press(&mut run, "escape"),
            "ui:play" => {
                let before = commands(&run);
                press(&mut run, "tab");
                press(&mut run, "f");
                if top(&run) != "ui:play" && top(&run) != "ui:reaction" {
                    press(&mut run, "escape");
                }
                if commands(&run) == before && top(&run) == "ui:play" {
                    // Nothing could be struck: a second goes by.
                    press(&mut run, ".");
                }
            }
            other => panic!("`{other}` is on top:\n{}", run.text()),
        }
        let _ = i;
    }
    // Three strangers with knives against someone with bare hands: the
    // night ends the other way.
    assert_eq!(top(&run), "ui:summary", "{}", run.text());
    let lines = log_text(&run);
    assert!(
        lines.iter().any(|line| line == "The game has ended: lost."),
        "{lines:?}"
    );
    assert!(
        lines.iter().any(|line| line.contains("strikes Ada")),
        "{lines:?}"
    );
    // Nothing the player is shown names an entity by its number.
    assert!(!lines.iter().any(|line| line.contains("0v0")), "{lines:?}");
    let summary = run.text();
    assert!(
        summary.contains("Lost.") && summary.contains("[ ] Hold out until dawn."),
        "{summary}"
    );
    assert_golden(golden("play_stand_lost"), run.frame());
    press(&mut run, "enter");
    assert_eq!(top(&run), "ui:title");
    assert!(saves(&run).is_empty(), "an ending takes the one save");
}
