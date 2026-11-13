//! Play session: the errand (13 §11), from the title, as a player plays
//! it. A parcel is carried across the market town to its town hall
//! before nightfall: the introduction and the clock, the journal's way,
//! a walk through the streets by the direction keys, a save on the way,
//! a quit to the title and Continue, and the delivery's ending and
//! summary. A second session lets the afternoon go by with the long
//! wait until night falls and the errand fails.
//!
//! The golden frames are written with `BLESS=1 cargo test -p clash-app`.

mod session;

use clash_scene::assert_golden;
use session::*;

/// The errand is the second game of the bundled list.
const ERRAND: usize = 1;

/// From the title to the street, past the introduction.
fn begin(run: &mut clash_app::headless::Headless) {
    new_game(run, ERRAND, "Ada");
    until_top(run, "ui:intro", 60);
    let intro = run.text();
    assert!(intro.contains("It is two in the afternoon"), "{intro}");
    press(run, "enter");
    settle(run);
    assert_eq!(top(run), "ui:play");
}

#[test]
fn the_parcel_is_carried_across_town_with_a_save_on_the_way() {
    let data = DataDir::new("errand-delivered");
    let mut run = boot_default(data.path(), 4);
    begin(&mut run);
    let shown = run.text();
    // The clock the deadline is played against, and the parcel in hand.
    assert!(shown.contains("Time 14:00"), "{shown}");
    assert!(
        log_text(&run)
            .iter()
            .any(|line| line.starts_with("You have the parcel"))
    );

    // The journal says which way the town hall is.
    press(&mut run, "shift+j");
    assert_eq!(top(&run), "ui:journal");
    let journal = run.text();
    assert!(
        journal.contains("[ ] Take the parcel to the town hall before nightfall.")
            && journal.contains("south, 129 m"),
        "{journal}"
    );
    press(&mut run, "escape");

    // Half the way, by the streets.
    let hall = site_anchor(&run, "core:errand_to");
    let walked = walk_to(&mut run, hall, 1, 60);
    assert_eq!(walked, 60);
    assert_eq!(top(&run), "ui:play");
    let journal_now = {
        press(&mut run, "shift+j");
        let text = run.text();
        press(&mut run, "escape");
        text
    };
    assert!(
        !journal_now.contains("129 m"),
        "the hall is nearer: {journal_now}"
    );

    // Saved by name, Enter in the name field saving it.
    save_as(&mut run, "half way");
    assert_eq!(top(&run), "ui:play");
    assert!(
        log_text(&run)
            .iter()
            .any(|line| line == "Saved as “half way” (half-way)."),
        "{:?}",
        log_text(&run)
    );
    let saved_at = player_loc(&run);
    let saved_hash = state_hash(&run);

    // Left, and carried on from the title: the same game.
    quit_to_title(&mut run);
    assert!(!run.game().playing());
    assert_eq!(buttons(&run)[..2], ["New game", "Continue"]);
    press_button(&mut run, "Continue");
    assert!(run.game().playing());
    assert_eq!(top(&run), "ui:play");
    assert_eq!(player_loc(&run), saved_at);
    assert_eq!(state_hash(&run), saved_hash, "a load changes nothing");

    // The rest of the way: the parcel reaches the hall and the game ends.
    walk_to(&mut run, hall, 0, 200);
    until_top(&mut run, "ui:summary", 120);
    assert!(
        log_text(&run)
            .iter()
            .any(|line| line == "The game has ended: delivered."),
        "{:?}",
        log_text(&run)
    );
    let summary = run.text();
    assert!(
        summary.contains("Delivered.")
            && summary.contains("[x] Take the parcel to the town hall before nightfall."),
        "{summary}"
    );
    // Well before nightfall: a walk across town takes minutes.
    assert!(time_of_day(&run) < 15.0 * 3600.0);
    assert_golden(golden("play_errand_delivered"), run.frame());

    // Back to the title; an ended game is not saved.
    press(&mut run, "enter");
    assert_eq!(top(&run), "ui:title");
    assert_eq!(saves(&run), ["half-way"]);
}
