//! Play session: the last stand (13 §11), from the title, as a player
//! plays it. Three strangers with knives and lanterns come down a dark
//! lane for the character, one by one; a lamp burns by the door of a
//! house, another inside, a club and bandages lie on its table and a
//! wardrobe stands in it, and it has a back door (decisions D-909,
//! D-950, D-963). In the first session the character gets inside, shuts
//! the door, leaves and carries on from the title (the game's one save),
//! and waits: the stranger opens the door and comes in. In the second
//! the character stays on the lane and fights, and the night ends the
//! other way. Then the night is played to its end: fighting at the door
//! as an ordinary person and as a brawler, shooting from inside as a
//! marksman, hidden in the dark house with the lamp out, and in the
//! wardrobe. An ignored session plays the night every way at many seeds
//! (`the_night_played_many_ways`, `docs/playtest-beta.md`, the fifth
//! round). That dawn ends the game is tested in `clash-sim`'s
//! `tests/scenario.rs` (decision D-879).
//!
//! The golden frames are written with `BLESS=1 cargo test -p clash-app`.

mod session;

use clash_app::headless::Headless;
use clash_scene::assert_golden;
use clash_world::Location;
use session::*;

/// The last stand is the first game of the bundled list.
const LAST_STAND: usize = 0;

/// The seed the night in the dark, in the wardrobe and at the door as a
/// brawler are played at: one at which all three strangers come for her
/// (`docs/playtest-beta.md`, the fifth round, has the night at sixteen
/// seeds played every way; at some, nobody ever sees her and nobody
/// comes in).
const NIGHT_SEED: u64 = 1;

/// The seed the ordinary person's night at the door is played at: the
/// kept sessions' first seed, at which she kills all three.
const DOOR_SEED: u64 = 4;

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

/// The seed the marksman's night is played at: one at which the first
/// stranger comes into the light of the open door and can be shot at
/// before he reaches her (at seed 4, since the strangers carry lanterns
/// and the first comes from the east, D-963, he is on her before a shot
/// is loosed).
const MARKSMAN_SEED: u64 = 8;

/// The brawler is the second of the playable characters.
const BRAWLER: usize = 1;

/// Holds out where the character stands as a player would: the long
/// wait (an hour, broken off when a foe comes into sight or a blow
/// lands); with someone alive in sight, Tab and `f` at them, walking up
/// to them first while they are out of reach (`strike_target`), a
/// minute going by when there is no way to them; any blow
/// coming is met with a dodge, a step back or a step aside where one is
/// in time (a dodge first: holding the door, a step back lets them in;
/// until the prompt listed the reactions by `offer`, D-943, Step back
/// was off the end of its list, and a dodge was what was chosen); wounds
/// are bound with a bandage in hand, the club put down
/// for it. Until the game ends or `turns` turns have gone. Returns the
/// blows started.
fn hold_out(run: &mut Headless, turns: usize) -> usize {
    let mut blows = 0;
    let mut wounds = 0;
    // Whether the character fights with the club: one put down is taken
    // up again.
    let mut armed = run.text().contains("Holding a club");
    for _ in 0..turns {
        match top(run).as_str() {
            "ui:summary" => break,
            "ui:reaction" => {
                react(run, &["Dodge", "Step back", "Step aside", "Parry", "Block"]);
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
                    // Bound with what is in hand (a bandage needs no
                    // free hand, D-947); only if it cannot be is the
                    // club put down first, and taken up again after.
                    let before = log_text(run).len();
                    if use_near(run, "Bandage your wounds") {
                        until_asked(run, 60);
                    }
                    let bound = log_since(run, before)
                        .iter()
                        .any(|line| line == "Ada binds their wounds.");
                    if !bound && top(run) == "ui:play" && run.text().contains("Holding a club") {
                        do_with_named(run, "club", "drop");
                        // A blow coming while the things carried were
                        // open leaves them open: put away.
                        for _ in 0..3 {
                            if ["ui:inventory", "ui:item"].contains(&top(run).as_str()) {
                                press(run, "escape");
                            }
                        }
                        until_asked(run, 60);
                        if top(run) == "ui:play" && use_near(run, "Bandage your wounds") {
                            until_asked(run, 60);
                        }
                    }
                    continue;
                }
                // The club put down to bind a wound, taken up again.
                if armed && !run.text().contains("Holding a club") {
                    // Once: if it cannot be had back, she fights without.
                    armed = false;
                    if use_near(run, "Pick up a club") {
                        until_asked(run, 60);
                        armed = run.text().contains("Holding a club");
                        continue;
                    }
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
                    // Out of reach, walked up to first, as a player
                    // would; with no way there, a minute, broken off by
                    // a blow.
                    if strike_target(run, 4) {
                        blows += 1;
                    } else {
                        press(run, "p");
                    }
                } else {
                    press(run, "shift+p");
                }
            }
            // Left open when a blow came: closed, as a player would.
            "ui:inventory" | "ui:item" | "ui:interact" => press(run, "escape"),
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
fn night_inside(template: Option<usize>, name: &str, seed: u64) -> Headless {
    let data = DataDir::new(&format!("stand-night-{name}"));
    let mut run = boot_default(data.path(), seed);
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
    hold_out(&mut run, 3000);
    run
}

#[test]
fn fighting_at_the_door_an_ordinary_person_kills_them_and_lives() {
    let run = night_inside(None, "ordinary", DOOR_SEED);
    // The strangers carry lanterns and tell each other where she is
    // (D-960, D-962); the first comes from the east, the fighter walks
    // up to whoever is in sight, binds her wounds between, and steps
    // over the fallen (D-963, D-964). At this seed she kills each as he
    // comes and lives to dawn; at most seeds an ordinary person at the
    // door does not (`docs/playtest-beta.md`, the fifth round).
    assert_eq!(outcome(&run), "survived", "{:?}", log_text(&run));
    assert!(deaths(&run) >= 1, "{:?}", log_text(&run));
    let lines = log_text(&run);
    assert!(lines.iter().any(|line| line == "Ada is wounded."));
}

#[test]
fn a_brawler_at_the_door_kills_them_and_lives() {
    let run = night_inside(Some(BRAWLER), "brawler", NIGHT_SEED);
    // As the ordinary person's night, at another seed (the fifth round).
    assert_eq!(outcome(&run), "survived", "{:?}", log_text(&run));
    assert!(deaths(&run) >= 1, "{:?}", log_text(&run));
}

/// The nearest cell within four of the character whose fixture is
/// named by a word (`lamp`, `wardrobe`): what a player sees in the room.
fn fixture_near(run: &Headless, word: &str) -> Option<Location> {
    let here = player_loc(run).unwrap();
    let mut cells: Vec<(i32, i32)> = (-4..=4)
        .flat_map(|dx| (-4..=4).map(move |dy| (dx, dy)))
        .collect();
    cells.sort_by_key(|(dx, dy)| dx.abs().max(dy.abs()));
    cells
        .into_iter()
        .map(|(dx, dy)| Location::new(here.grid, here.cell + clash_core::IVec3::new(dx, dy, 0)))
        .find(|at| {
            run.game()
                .with_session(|session| {
                    let cosmos = session.sim.truth().cosmos();
                    let cell = cosmos.cell(*at).ok()?;
                    let fixture = cosmos
                        .defs()
                        .state_text(clash_world::Layer::Fixture, cell.fixture.raw());
                    Some(fixture.contains(word))
                })
                .flatten()
                .unwrap_or(false)
        })
}

/// The lamp in the room put out: the house dark again.
fn put_out_the_lamp(run: &mut Headless) {
    let lamp = fixture_near(run, "lamp").expect("a lamp in the room");
    walk_to(run, lamp, 1, 12);
    assert!(use_near(run, "Switch"), "{}", run.text());
    until_asked(run, 60);
}

/// Whether the character is inside something (a wardrobe).
fn hidden_inside(run: &Headless) -> bool {
    run.game()
        .with_session(|session| {
            session
                .sim
                .truth()
                .entity(session.player)
                .map(|entity| matches!(entity.place(), clash_sim::Place::In(_)))
        })
        .flatten()
        .unwrap_or(false)
}

#[test]
fn hidden_in_the_dark_house_until_dawn() {
    let data = DataDir::new("stand-dark");
    let mut run = boot_default(data.path(), NIGHT_SEED);
    begin(&mut run);
    get_inside(&mut run);
    take_up_what_is_on_the_table(&mut run);
    put_out_the_lamp(&mut run);
    hide_out(&mut run, 6000);
    // In the dark by the lamp put out, with the club and the bandages;
    // the strangers came in with their lanterns, one found her, and she
    // fought them off in the dark and is alive at the first light: the
    // night is won (at most seeds it is not: the fifth round).
    assert_eq!(outcome(&run), "survived", "{:?}", log_text(&run));
    assert_eq!(time_of_day(&run), 5.0 * 3600.0);
    let summary = run.text();
    assert!(
        summary.contains("Survived.") && summary.contains("[x] Hold out until dawn."),
        "{summary}"
    );
}

#[test]
fn hidden_in_the_wardrobe_the_stranger_searches_it() {
    let data = DataDir::new("stand-wardrobe");
    let mut run = boot_default(data.path(), NIGHT_SEED);
    begin(&mut run);
    get_inside(&mut run);
    put_out_the_lamp(&mut run);
    // The wardrobe further in the room (D-950), and in.
    let wardrobe = fixture_near(&run, "wardrobe").expect("a wardrobe in the house");
    walk_to(&mut run, wardrobe, 1, 12);
    assert!(use_near(&mut run, "Hide inside"), "{}", run.text());
    until_asked(&mut run, 60);
    assert!(hidden_inside(&run), "{}", run.text());
    // Waiting inside: the stranger comes believing her in the house,
    // searches the wardrobe and brings her out (D-950).
    for _ in 0..60 {
        if !hidden_inside(&run) || top(&run) != "ui:play" {
            break;
        }
        press(&mut run, "p");
    }
    assert!(!hidden_inside(&run), "never found: {:?}", log_text(&run));
    assert!(time_of_day(&run) > 20.0 * 3600.0, "found before dawn");
    hold_out(&mut run, 400);
    let lines = log_text(&run);
    assert!(
        lines.iter().any(|line| line.starts_with("The person")),
        "{lines:?}"
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
                react(run, &["Dodge", "Step back", "Step aside", "Parry", "Block"]);
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
            // Left open when a blow came: closed, as a player would.
            "ui:inventory" | "ui:item" | "ui:interact" => press(run, "escape"),
            other => panic!("`{other}` is on top:\n{}", run.text()),
        }
    }
    acts_of(run, "core:draw_and_loose") - start
}

#[test]
fn a_marksman_shoots_at_them_from_inside() {
    let data = DataDir::new("stand-marksman");
    let mut run = boot_default(data.path(), MARKSMAN_SEED);
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
            // Left open when a blow came: closed, as a player would.
            "ui:inventory" | "ui:item" | "ui:interact" => press(&mut run, "escape"),
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

// ---- the night played many ways (`docs/playtest-beta.md`, the fifth round) ----

/// Waits out the night where the character is, as a player hiding
/// would: the long wait while nobody is in sight, a minute at a time
/// while someone is (the long wait is broken off when a foe comes into
/// sight); found (a blow coming, a wound, or brought out of where it
/// hid), it fights as `hold_out` does. Until the game ends or `turns`
/// turns have gone. Returns whether it was found.
fn hide_out(run: &mut Headless, turns: usize) -> bool {
    let inside = hidden_inside(run);
    for _ in 0..turns {
        match top(run).as_str() {
            "ui:summary" => return false,
            "ui:reaction" => break,
            "ui:play" => {
                let hurt = log_text(run).iter().any(|line| line == "Ada is wounded.");
                if hurt || (inside && !hidden_inside(run)) {
                    break;
                }
                press(run, "tab");
                if run.game().scene().target().is_some() && !target_dead(run) {
                    press(run, "p");
                } else {
                    press(run, "shift+p");
                }
            }
            // Left open when a blow came: closed, as a player would.
            "ui:inventory" | "ui:item" | "ui:interact" => press(run, "escape"),
            other => panic!("`{other}` is on top:\n{}", run.text()),
        }
    }
    if top(run) == "ui:summary" {
        return false;
    }
    hold_out(run, turns);
    true
}

/// The other door of the house, if it has one: a door within eight
/// cells of the refuge on its storey, the farthest from it.
fn back_door(run: &Headless) -> Option<Location> {
    let door = refuge(run);
    let mut cells: Vec<(i32, i32)> = (-8..=8)
        .flat_map(|dx| (-8..=8).map(move |dy| (dx, dy)))
        .collect();
    cells.sort_by_key(|(dx, dy)| std::cmp::Reverse(dx * dx + dy * dy));
    cells
        .into_iter()
        .filter(|delta| *delta != (0, 0))
        .map(|(dx, dy)| Location::new(door.grid, door.cell + clash_core::IVec3::new(dx, dy, 0)))
        .find(|at| {
            run.game()
                .with_session(|session| {
                    let cosmos = session.sim.truth().cosmos();
                    let cell = cosmos.cell(*at).ok()?;
                    let block = cosmos
                        .defs()
                        .state_text(clash_world::Layer::Block, cell.block.raw());
                    Some(block.starts_with("core:door"))
                })
                .flatten()
                .unwrap_or(false)
        })
}

/// The bandages and the club on the table taken up, by the lamp.
fn take_up_what_is_on_the_table(run: &mut Headless) {
    let here = player_loc(run).unwrap();
    let Some(table) = lying_near(run, here, "core:bandage", 6) else {
        return;
    };
    let beside = [(-1, 0), (0, 1), (1, 0), (0, -1)]
        .into_iter()
        .map(|(dx, dy)| Location::new(table.grid, table.cell + clash_core::IVec3::new(dx, dy, 0)))
        .find(|at| believed_way(run, *at).is_ok())
        .unwrap_or(table);
    walk_to(run, beside, 0, 12);
    for what in ["Pick up a club", "Pick up 2 bandages"] {
        if top(run) != "ui:play" {
            return;
        }
        let armed = run.text().contains("Holding a club") || run.text().contains("Holding a bow");
        if !(armed && what.ends_with("club")) && use_near(run, what) {
            until_asked(run, 60);
        }
    }
}

/// Shoots at whoever is in sight while there are arrows, as
/// `shoot_it_out` does, and fights as `hold_out` does once a shot
/// cannot be loosed; waits the long wait with nobody in sight.
fn shoot_then_fight(run: &mut Headless, turns: usize) {
    for _ in 0..turns {
        match top(run).as_str() {
            "ui:summary" => return,
            "ui:reaction" => {
                react(run, &["Dodge", "Step back", "Step aside", "Parry", "Block"]);
            }
            "ui:play" => {
                press(run, "tab");
                if run.game().scene().target().is_some() && !target_dead(run) {
                    let before = commands(run);
                    press(run, "s");
                    if !["ui:play", "ui:reaction", "ui:summary"].contains(&top(run).as_str()) {
                        press(run, "escape");
                    }
                    if commands(run) == before && top(run) == "ui:play" && !strike_target(run, 4) {
                        press(run, ".");
                    }
                } else {
                    press(run, "shift+p");
                }
            }
            // Left open when a blow came: closed, as a player would.
            "ui:inventory" | "ui:item" | "ui:interact" => press(run, "escape"),
            other => panic!("`{other}` is on top:\n{}", run.text()),
        }
    }
}

/// One night played one way at one seed: the outcome, how many died,
/// whether the hider was found, and the time of day it ended.
fn play_night(way: &str, seed: u64) -> String {
    let data = DataDir::new(&format!("stand-{way}-{seed}"));
    let mut run = boot_default(data.path(), seed);
    match way {
        "brawler" => {
            create(&mut run, LAST_STAND, BRAWLER, "Ada");
            begin_created(&mut run);
        }
        "marksman" => {
            create(&mut run, LAST_STAND, MARKSMAN, "Ada");
            begin_created(&mut run);
        }
        _ => begin(&mut run),
    }
    get_inside(&mut run);
    take_up_what_is_on_the_table(&mut run);
    let mut found = false;
    // Set upon before there was time to hide or flee: fought out.
    let way = if top(&run) == "ui:play" {
        way
    } else {
        "ordinary"
    };
    match way {
        "dark" => {
            put_out_the_lamp(&mut run);
            found = hide_out(&mut run, 6000);
        }
        "wardrobe" => {
            put_out_the_lamp(&mut run);
            let wardrobe = fixture_near(&run, "wardrobe").expect("a wardrobe in the house");
            walk_to(&mut run, wardrobe, 1, 12);
            assert!(use_near(&mut run, "Hide inside"), "{}", run.text());
            until_asked(&mut run, 60);
            found = hide_out(&mut run, 3000);
        }
        "flee" => {
            let back = back_door(&run).expect("a back door");
            walk_to(&mut run, back, 0, 30);
            // Out, and on away from the lane into the dark, some thirty
            // metres, round what is in the way.
            let door = refuge(&run);
            let away = (back.cell.y - door.cell.y).signum();
            let far = Location::new(
                back.grid,
                back.cell + clash_core::IVec3::new(0, 30 * away, 0),
            );
            if top(&run) == "ui:play" {
                let reachable = believed_way(&run, far).is_ok();
                if reachable {
                    walk_to(&mut run, far, 3, 60);
                } else {
                    for _ in 0..30 {
                        if top(&run) != "ui:play" {
                            break;
                        }
                        press(&mut run, if away < 0 { "k" } else { "j" });
                    }
                }
            }
            found = hide_out(&mut run, 3000);
        }
        "marksman" => shoot_then_fight(&mut run, 3000),
        _ => {
            hold_out(&mut run, 3000);
        }
    }
    // For looking into a night: its log, and the lights carried.
    if let Ok(dir) = std::env::var("NIGHT_DUMP") {
        let door = refuge(&run);
        let lights = run
            .game()
            .with_session(|session| session.sim.truth().cosmos().area_lights(door.grid).len())
            .unwrap_or_default();
        let text = format!(
            "lights carried: {lights}\n{}\n{}",
            log_text(&run).join("\n"),
            run.text()
        );
        let _ = std::fs::write(format!("{dir}/night-{way}-{seed}.txt"), text);
    }
    format!(
        "{way} seed {seed}: {} deaths {} found {found} at {:.2} h",
        outcome(&run),
        deaths(&run),
        time_of_day(&run) / 3600.0
    )
}

/// The night played each way at each seed (`NIGHT_WAYS`: `dark`,
/// `wardrobe`, `ordinary`, `brawler`, `marksman`, `flee`, comma
/// separated; `NIGHT_SEEDS`: `1-8` or a list; `NIGHT_THREADS`, default
/// three), a line for each to standard error and to `NIGHT_LOG` if set.
/// Run in release: `cargo test --release -p clash-app --test
/// play_last_stand -- --ignored --nocapture the_night_played`.
#[test]
#[ignore = "plays dozens of nights: run by hand in release for the balance"]
fn the_night_played_many_ways() {
    let ways: Vec<String> = std::env::var("NIGHT_WAYS")
        .unwrap_or_else(|_| "dark,wardrobe,ordinary,brawler,marksman,flee".to_owned())
        .split(',')
        .map(str::to_owned)
        .collect();
    let seeds: Vec<u64> = match std::env::var("NIGHT_SEEDS") {
        Ok(text) if text.contains('-') => {
            let (a, b) = text.split_once('-').unwrap();
            (a.parse().unwrap()..=b.parse().unwrap()).collect()
        }
        Ok(text) => text.split(',').map(|seed| seed.parse().unwrap()).collect(),
        Err(_) => (1..=8).collect(),
    };
    let threads: usize = std::env::var("NIGHT_THREADS")
        .ok()
        .and_then(|text| text.parse().ok())
        .unwrap_or(3);
    let jobs: Vec<(String, u64)> = ways
        .iter()
        .flat_map(|way| seeds.iter().map(move |seed| (way.clone(), *seed)))
        .collect();
    let next = std::sync::atomic::AtomicUsize::new(0);
    let log = std::env::var("NIGHT_LOG").ok();
    let lock = std::sync::Mutex::new(());
    std::thread::scope(|scope| {
        for _ in 0..threads {
            scope.spawn(|| {
                loop {
                    let at = next.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                    let Some((way, seed)) = jobs.get(at) else {
                        break;
                    };
                    let started = std::time::Instant::now();
                    let line = std::panic::catch_unwind(|| play_night(way, *seed))
                        .unwrap_or_else(|_| format!("{way} seed {seed}: panicked"));
                    let line = format!("{line} ({:.0} s)", started.elapsed().as_secs_f64());
                    let _guard = lock.lock().unwrap();
                    eprintln!("{line}");
                    if let Some(path) = &log {
                        use std::io::Write as _;
                        if let Ok(mut file) = std::fs::OpenOptions::new()
                            .create(true)
                            .append(true)
                            .open(path)
                        {
                            let _ = writeln!(file, "{line}");
                        }
                    }
                }
            });
        }
    });
}
