//! Play session: the last stand (13 §11), from the title, as a player
//! plays it. Three strangers come down a dark lane for the character,
//! one by one; a lamp burns by the door of a house, another inside, and
//! a club and bandages lie on its table (decision D-909). In the first
//! session the character gets inside, shuts the door, leaves and carries
//! on from the title (the game's one save), and waits: the stranger
//! opens the door and comes in. In the second the character stays on the
//! lane and fights, and the night ends the other way. Then the night is
//! played to its end: fighting at the door as an ordinary person and as
//! a brawler, shooting from inside as a marksman, and hidden in the dark
//! house with the lamp out until dawn. That dawn ends the game is tested
//! in `clash-sim`'s `tests/scenario.rs` (decision D-879).
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
fn inside_with_the_door_shut_they_come_in() {
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

    // A shut door does not keep them out: minds open doors on their way
    // (town life), and waiting is broken off when they reach her.
    for _ in 0..6 {
        if answer(&mut run) == "ui:summary" {
            break;
        }
        press(&mut run, "p");
    }
    let lines = log_text(&run);
    assert!(
        lines.iter().any(|line| line.starts_with("The person")),
        "they came in: {}\n{lines:?}",
        run.text()
    );
}

/// The marksman is the third of the playable characters.
const MARKSMAN: usize = 2;

/// The brawler is the second of the playable characters.
const BRAWLER: usize = 1;

/// Holds out where the character stands as a player would: the long
/// wait (an hour, broken off when a foe comes into sight or a blow
/// lands); with someone alive in sight, Tab and `f` at them, a minute
/// going by when the blow cannot be struck (out of reach); any blow
/// coming is met with a step back, a dodge or a step aside where one is
/// in time; wounds are bound with a bandage in hand, the club put down
/// for it. Until the game ends or `turns` turns have gone. Returns the
/// blows started.
fn hold_out(run: &mut Headless, turns: usize) -> usize {
    let mut blows = 0;
    let mut wounds = 0;
    for _ in 0..turns {
        match top(run).as_str() {
            "ui:summary" => break,
            "ui:reaction" => {
                react(run, &["Step back", "Dodge", "Step aside", "Parry", "Block"]);
            }
            "ui:play" => {
                // Wounded since the last bandage, as the log says: one
                // is put on while there is one in hand.
                let now = log_text(run)
                    .iter()
                    .filter(|line| line.as_str() == "Ada is wounded.")
                    .count();
                if now > wounds && run.text().contains("bandage") {
                    wounds = now;
                    // A hand free for it: the club put down first, and
                    // taken up again after.
                    let armed = run.text().contains("Holding a club");
                    if armed {
                        do_with_named(run, "club", "drop");
                        until_asked(run, 60);
                    }
                    if use_near(run, "Bandage your wounds") {
                        until_asked(run, 60);
                    }
                    if armed && top(run) == "ui:play" && use_near(run, "Pick up a club") {
                        until_asked(run, 60);
                    }
                    continue;
                }
                // Tab past whoever the log said died.
                press(run, "tab");
                for _ in 0..4 {
                    if !target_dead(run) {
                        break;
                    }
                    press(run, "tab");
                }
                if run.game().scene().target().is_some() && !target_dead(run) {
                    let before = commands(run);
                    press(run, "f");
                    if !["ui:play", "ui:reaction", "ui:summary"].contains(&top(run).as_str()) {
                        press(run, "escape");
                    }
                    if commands(run) == before && top(run) == "ui:play" {
                        // Out of reach: a minute, broken off by a blow.
                        press(run, "p");
                    } else {
                        blows += 1;
                    }
                } else {
                    press(run, "shift+p");
                }
            }
            other => panic!("`{other}` is on top:\n{}", run.text()),
        }
    }
    blows
}

/// How the night ended, from the log: `survived`, `lost`, or `on` while
/// it goes on.
fn outcome(run: &Headless) -> &'static str {
    let lines = log_text(run);
    if lines
        .iter()
        .any(|line| line == "The game has ended: survived.")
    {
        "survived"
    } else if lines.iter().any(|line| line == "The game has ended: lost.") {
        "lost"
    } else {
        "on"
    }
}

/// How many times the log said someone died.
fn deaths(run: &Headless) -> usize {
    log_text(run)
        .iter()
        .filter(|line| line.ends_with(" dies."))
        .count()
}

/// The night played inside, from the title, as an ordinary person or
/// as a playable character chosen ready-made: in through the door, what
/// lies on the table taken up (the club, for whoever has nothing in
/// hand, and the bandages), and held out at the door (`hold_out`).
fn night_inside(template: Option<usize>, name: &str) -> Headless {
    let data = DataDir::new(&format!("stand-night-{name}"));
    let mut run = boot_default(data.path(), 4);
    match template {
        Some(template) => {
            create(&mut run, LAST_STAND, template, "Ada");
            begin_created(&mut run);
        }
        None => begin(&mut run),
    }
    get_inside(&mut run);
    // The lamp in the room shows what lies on the table.
    let here = player_loc(&run).unwrap();
    let table = lying_near(&run, here, "core:bandage", 6).expect("bandages on the table");
    let beside = [(-1, 0), (0, 1), (1, 0), (0, -1)]
        .into_iter()
        .map(|(dx, dy)| Location::new(table.grid, table.cell + clash_core::IVec3::new(dx, dy, 0)))
        .find(|at| believed_way(&run, *at).is_ok())
        .unwrap_or(table);
    walk_to(&mut run, beside, 0, 12);
    press(&mut run, "e");
    let near = run.text();
    assert!(
        near.contains("Pick up a club") && near.contains("Pick up 2 bandages"),
        "{near}"
    );
    press(&mut run, "escape");
    for what in ["Pick up a club", "Pick up 2 bandages"] {
        let armed = run.text().contains("Holding a club");
        if !(armed && what.ends_with("club")) && use_near(&mut run, what) {
            until_asked(&mut run, 60);
        }
    }
    hold_out(&mut run, 1500);
    run
}

#[test]
fn fighting_at_the_door_an_ordinary_person_kills_the_first_and_falls() {
    let run = night_inside(None, "ordinary");
    // One stranger at a time: the first is beaten to death at the door,
    // and the wounds of the fight (blows stopped on the forearms cut
    // them) kill the one who stood there.
    assert_eq!(outcome(&run), "lost", "{:?}", log_text(&run));
    assert_eq!(deaths(&run), 1, "{:?}", log_text(&run));
    let lines = log_text(&run);
    assert!(lines.iter().any(|line| line == "Ada is wounded."));
    assert!(lines.iter().any(|line| line == "Ada falls senseless."));
}

#[test]
fn a_brawler_at_the_door_kills_all_three_and_bleeds_to_death() {
    let run = night_inside(Some(BRAWLER), "brawler");
    // The club wins every fight; nothing binds the wounds (the bandages
    // are never taken up: both hands hold a club, his own and the
    // table's; a bandage carried binds with both hands full, D-947) and
    // the night is lost after the third, at one in the morning.
    assert_eq!(deaths(&run), 3, "{:?}", log_text(&run));
    assert_eq!(outcome(&run), "lost", "{:?}", log_text(&run));
    assert!(time_of_day(&run) < 2.0 * 3600.0);
}

#[test]
fn hidden_in_the_dark_house_until_dawn() {
    let data = DataDir::new("stand-dark");
    let mut run = boot_default(data.path(), 4);
    begin(&mut run);
    get_inside(&mut run);
    // The lamp in the room put out: the house dark again.
    let here = player_loc(&run).unwrap();
    let lamp = (-4..=4)
        .flat_map(|dx| (-4..=4).map(move |dy| (dx, dy)))
        .map(|(dx, dy)| Location::new(here.grid, here.cell + clash_core::IVec3::new(dx, dy, 0)))
        .find(|at| {
            run.game()
                .with_session(|session| {
                    let cosmos = session.sim.truth().cosmos();
                    let cell = cosmos.cell(*at).ok()?;
                    let fixture = cosmos
                        .defs()
                        .state_text(clash_world::Layer::Fixture, cell.fixture.raw());
                    Some(fixture.contains("lamp"))
                })
                .flatten()
                .unwrap_or(false)
        })
        .expect("a lamp in the room");
    walk_to(&mut run, lamp, 1, 12);
    assert!(use_near(&mut run, "Switch"), "{}", run.text());
    until_asked(&mut run, 60);
    hold_out(&mut run, 400);
    // Unseen, unfound, and alive at the first light: the night is won.
    assert_eq!(outcome(&run), "survived", "{:?}", log_text(&run));
    assert_eq!(time_of_day(&run), 5.0 * 3600.0);
    let summary = run.text();
    assert!(
        summary.contains("Survived.") && summary.contains("[x] Hold out until dawn."),
        "{summary}"
    );
}

/// Plays on from where the character stands as a player with a bow
/// would: the long wait (an hour, broken off when a foe comes into
/// sight), and then Tab and `s` at whoever is in sight; any blow is met
/// with a step back, a dodge or a step aside where one is in time.
/// Until the game ends or `keys` turns have gone. Returns how many
/// shots were started.
fn shoot_it_out(run: &mut Headless, keys: usize) -> usize {
    let start = acts_of(run, "core:draw_and_loose");
    for _ in 0..keys {
        match top(run).as_str() {
            "ui:summary" => break,
            "ui:reaction" => {
                react(run, &["Step back", "Dodge", "Step aside", "Parry", "Block"]);
            }
            "ui:play" => {
                press(run, "tab");
                if run.game().scene().target().is_some() && !target_dead(run) {
                    let before = commands(run);
                    press(run, "s");
                    if !["ui:play", "ui:reaction", "ui:summary"].contains(&top(run).as_str()) {
                        press(run, "escape");
                    }
                    // Refused (a guard raised to block holds an arm until
                    // the block has run its course; no arrow left): a
                    // second goes by.
                    if commands(run) == before && top(run) == "ui:play" {
                        press(run, ".");
                    }
                } else {
                    press(run, "shift+p");
                }
            }
            other => panic!("`{other}` is on top:\n{}", run.text()),
        }
    }
    acts_of(run, "core:draw_and_loose") - start
}

#[test]
fn a_marksman_shoots_at_them_from_inside() {
    let data = DataDir::new("stand-marksman");
    let mut run = boot_default(data.path(), 4);
    // Chosen ready-made: a name, and nothing more.
    create(&mut run, LAST_STAND, MARKSMAN, "Ada");
    let shown = run.text();
    assert!(shown.contains("Ready-made"), "{shown}");
    begin_created(&mut run);
    let shown = run.text();
    assert!(shown.contains("Holding a bow"), "{shown}");
    // A bow is not loaded: each shot nocks an arrow from the quiver.
    press(&mut run, "r");
    assert_eq!(
        log_text(&run).last().map(String::as_str),
        Some("There is nothing in hand to load."),
        "{:?}",
        log_text(&run)
    );
    get_inside(&mut run);
    // The door open again onto the lit porch, and back against the far
    // wall, facing it.
    assert!(use_near(&mut run, "Open"), "{}", run.text());
    until_asked(&mut run, 60);
    press(&mut run, "k");
    let shots = shoot_it_out(&mut run, 120);
    let lines = log_text(&run);
    // A quiver of arrows into the first of them, who dies at the door.
    assert!(shots >= 10, "{shots} shots: {lines:?}");
    assert!(
        lines
            .iter()
            .any(|line| line == "Something strikes the person."),
        "{lines:?}"
    );
    assert!(deaths(&run) >= 1, "{lines:?}");
    // With a bow in hand the quick shot says the bow's reason, never the
    // pistol's (D-943).
    assert!(
        !lines
            .iter()
            .any(|line| line == "There is no loaded weapon in hand, or nowhere to aim."),
        "{lines:?}"
    );
    // Tab passes over the dead (D-944).
    if top(&run) == "ui:play" {
        for _ in 0..4 {
            press(&mut run, "tab");
            assert!(!target_dead(&run), "Tab chose the dead: {}", run.text());
        }
    }
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
