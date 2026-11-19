//! The first walk (package U11), run headless with the bundled mods:
//! from the title screen, through the new-game screen, to a character
//! made from the `core:person` template standing in the generated town;
//! then through an open door into a house with the `core:step` action,
//! and up to a wall that stops it with the movement package's reason.
//! Golden frames (12 §12) are kept for each of those moments, taken
//! once playback has come to rest, and the view model is checked to
//! hold only what the character could see: inside the house the street
//! behind the wall is remembered, not seen.
//!
//! The golden frames are written with `BLESS=1 cargo test -p clash-app`.

use std::collections::BTreeSet;
use std::path::PathBuf;

use clash_app::cli::Options;
use clash_app::headless::Headless;
use clash_core::{IVec3, Tick};
use clash_scene::assert_golden;
use clash_world::{CellPos, Location};

/// The repository's `mods` directory.
fn mods_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../mods")
}

/// A data directory of the test's own, empty, so that no stored choice
/// of whoever runs the tests changes what is drawn.
fn data_dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("clash-app-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    dir
}

fn golden(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/golden")
        .join(format!("{name}.frame"))
}

/// The default profile, booted headless.
fn start(name: &str) -> Headless {
    let options = Options {
        mods: Some(mods_dir()),
        data: Some(data_dir(name)),
        ..Options::default()
    };
    let game = clash_app::boot(&options).unwrap_or_else(|problem| panic!("{problem}"));
    Headless::new(game)
}

fn press(run: &mut Headless, keys: &[&str]) {
    let going = run.keys(keys).unwrap_or_else(|problem| panic!("{problem}"));
    assert!(going, "the game ended while pressing {keys:?}");
}

/// Passes with no input until playback has nothing left to play: the
/// picture at rest, as a golden frame is taken of it.
fn settle(run: &mut Headless) {
    for _ in 0..600 {
        if run.game().scene().playback().is_idle() {
            return;
        }
        run.pass();
    }
    panic!("playback never came to rest");
}

/// Where the player's character is, and the simulation's time.
fn player(run: &Headless) -> (Location, Tick) {
    run.game()
        .with_session(|session| {
            let at = session.player_loc().expect("the character is in the world");
            (at, session.sim.now())
        })
        .expect("a game is being played")
}

/// The cells round a place that the view model knows, split into those
/// seen now and those remembered.
fn known_round(run: &Headless, at: Location) -> (BTreeSet<CellPos>, BTreeSet<CellPos>) {
    let model = run
        .game()
        .scene()
        .world()
        .expect("a world is shown")
        .model();
    let (mut seen, mut remembered) = (BTreeSet::new(), BTreeSet::new());
    for z in at.cell.z - 2..=at.cell.z + 2 {
        for y in at.cell.y - 48..=at.cell.y + 48 {
            for x in at.cell.x - 48..=at.cell.x + 48 {
                let cell = CellPos::new(x, y, z);
                match model.cell(Location::new(at.grid, cell)) {
                    Some(known) if known.seen() => seen.insert(cell),
                    Some(_) => remembered.insert(cell),
                    None => false,
                };
            }
        }
    }
    (seen, remembered)
}

/// The last line of the log, as plain text.
fn last_log(run: &Headless) -> String {
    run.game()
        .scene()
        .log()
        .lines()
        .last()
        .map(|line| line.text.plain())
        .unwrap_or_default()
}

#[test]
fn title_to_town_through_a_door_to_a_wall() {
    let mut run = start("walk");

    // The title screen.
    assert_eq!(run.presented(), 1, "the boot screen is the first frame");
    assert_eq!(
        run.game().scene().top().map(|screen| screen.id()),
        Some("ui:title")
    );
    assert!(!run.game().playing());
    assert_golden(golden("title"), run.frame());

    // The new-game screen: a name, the seed, the list of starts.
    press(&mut run, &["enter"]);
    assert_eq!(
        run.game().scene().top().map(|screen| screen.id()),
        Some("ui:new_game")
    );
    assert!(run.type_text("Ada"));
    press(&mut run, &["tab", "tab"]);
    assert_golden(golden("new_game"), run.frame());

    // The town, as the character sees it from where it begins: the
    // third of the scenarios.
    press(&mut run, &["down", "down", "enter"]);
    assert!(run.game().playing(), "the first start began a game");
    assert_eq!(
        run.game().scene().top().map(|screen| screen.id()),
        Some("ui:play")
    );
    settle(&mut run);
    assert_golden(golden("first_play"), run.frame());
    let (begin, began) = player(&run);
    let (seen_first, remembered_first) = known_round(&run, begin);
    assert!(remembered_first.is_empty(), "nothing is remembered yet");
    // What is shown is what the field of view holds, and no more.
    let view_cells = run
        .game()
        .with_session(|session| clash_sim::perception::party::party_view(&session.sim).len())
        .unwrap();
    assert!(
        seen_first.len() <= view_cells,
        "only cells in view are known"
    );
    assert!(seen_first.contains(&begin.cell));
    assert!(
        run.text().contains("Ada"),
        "the character goes by the name it was given"
    );

    // North, through the open door of the house across the street.
    press(&mut run, &["k", "k", "k", "k"]);
    let (inside, walked) = player(&run);
    assert_eq!(
        (inside.cell.x, inside.cell.y),
        (begin.cell.x, begin.cell.y - 4),
        "four steps north"
    );
    assert!(walked > began, "each step took time");
    let (seen_inside, remembered_inside) = known_round(&run, inside);
    assert!(seen_inside.contains(&inside.cell));
    assert!(
        seen_inside.iter().any(|cell| !seen_first.contains(cell)),
        "the inside of the house is seen from in it"
    );
    // The street behind the wall went to memory: far along it to the
    // east and to the west, out of the door's line.
    for far in [
        begin.cell.offset(IVec3::new(16, 0, 0)),
        begin.cell.offset(IVec3::new(-8, 0, 0)),
    ] {
        assert!(seen_first.contains(&far), "{far} was seen from the street");
        assert!(
            remembered_inside.contains(&far),
            "{far} is remembered, not seen, from inside the house"
        );
    }
    settle(&mut run);
    assert_golden(golden("inside_house"), run.frame());

    // On north until a wall stops the character, with movement's
    // reason.
    let wall = "Something is in the way.";
    let mut steps = 0;
    while last_log(&run) != wall {
        press(&mut run, &["k"]);
        steps += 1;
        assert!(steps < 12, "no wall stopped the character");
    }
    let (stopped, stopped_at) = player(&run);
    press(&mut run, &["k"]);
    assert_eq!(
        player(&run),
        (stopped, stopped_at),
        "a refused step moves nothing and takes no time"
    );
    assert_eq!(last_log(&run), wall);
    settle(&mut run);
    assert_golden(golden("wall"), run.frame());
}

#[test]
fn a_game_is_left_from_the_pause_menu() {
    let mut run = start("pause");
    press(&mut run, &["enter", "tab", "tab", "down", "down", "enter"]);
    assert!(run.game().playing());
    // Esc pauses: Resume, Save (no save is there yet, so no Load),
    // Options, and the last button leaves, after asking.
    press(&mut run, &["escape"]);
    assert_eq!(
        run.game().scene().top().map(|screen| screen.id()),
        Some("ui:pause")
    );
    press(&mut run, &["down", "down", "down", "enter", "enter"]);
    assert!(!run.game().playing(), "the game is over");
    assert_eq!(
        run.game().scene().top().map(|screen| screen.id()),
        Some("ui:title")
    );
    // And the title's Quit ends the run.
    press(&mut run, &["down", "down"]);
    assert!(!run.keys(&["enter"]).unwrap(), "quit ends the run");
}

#[test]
fn a_shut_door_stops_the_character_who_will_not_open_doors() {
    // The preference `bump` at `never`: walking into a door is only a
    // step, which movement refuses. (Walking into it opens it otherwise:
    // `tests/interacting.rs`.)
    let data = data_dir("shut");
    std::fs::create_dir_all(&data).unwrap();
    std::fs::write(
        data.join("preferences.json"),
        r#"{ "preferences": { "ui:bump": "never" }, "bindings": [] }"#,
    )
    .unwrap();
    let options = Options {
        mods: Some(mods_dir()),
        data: Some(data.clone()),
        ..Options::default()
    };
    let mut run =
        Headless::new(clash_app::boot(&options).unwrap_or_else(|problem| panic!("{problem}")));
    // The second start: the lane, whose doors are shut.
    press(
        &mut run,
        &[
            "enter", "tab", "tab", "down", "down", "down", "down", "enter",
        ],
    );
    assert!(run.game().playing());
    let (begin, _) = player(&run);
    // Across the lane to the door of the house south of it.
    press(&mut run, &["j", "j", "j", "l", "l"]);
    let (at_door, then) = player(&run);
    assert_eq!(
        (at_door.cell.x, at_door.cell.y),
        (begin.cell.x + 2, begin.cell.y + 3)
    );
    press(&mut run, &["j"]);
    assert_eq!(last_log(&run), "The way is closed.");
    assert_eq!(player(&run), (at_door, then));
    let _ = std::fs::remove_dir_all(&data);
}
