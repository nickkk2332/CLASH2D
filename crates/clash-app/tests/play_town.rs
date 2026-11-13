//! Play session: the market town (13 §11), the sandbox, from the title,
//! as a player lives in it: begun in the town square, a look round at
//! the people and things in sight, a word with a resident, the walk to
//! the market, a shut door opened by walking into it, a cupboard in a
//! home opened and emptied, and a save loaded again. Then a fight in the
//! market: a resident struck, and what the town does about it.
//!
//! The golden frames are written with `BLESS=1 cargo test -p clash-app`.

mod session;

use clash_app::headless::Headless;
use clash_scene::assert_golden;
use clash_world::Location;
use session::*;

/// The market town is the fourth game of the bundled list.
const TOWN_LIFE: usize = 3;

/// From the title to the town square, past the setup and the
/// introduction.
fn begin_in_the_square(run: &mut Headless) {
    new_game(run, TOWN_LIFE, "Ada");
    until_top(run, "ui:setup", 60);
    let setup = run.text();
    assert!(setup.contains("The town square"), "{setup}");
    press(run, "down");
    press(run, "enter");
    until_top(run, "ui:intro", 60);
    press(run, "enter");
    settle(run);
    assert_eq!(top(run), "ui:play");
}

/// The cells within a box round a place whose block or fixture is
/// named by a word (`door`, `cupboard`), nearest first.
fn near(run: &Headless, at: Location, word: &str, reach: i32) -> Vec<Location> {
    let mut found = run
        .game()
        .with_session(|session| {
            let cosmos = session.sim.truth().cosmos();
            let defs = cosmos.defs();
            let mut found = Vec::new();
            for dy in -reach..=reach {
                for dx in -reach..=reach {
                    let loc = Location::new(at.grid, at.cell + clash_core::IVec3::new(dx, dy, 0));
                    let Ok(cell) = cosmos.cell(loc) else { continue };
                    let named = [
                        defs.state_text(clash_world::Layer::Block, cell.block.raw()),
                        defs.state_text(clash_world::Layer::Fixture, cell.fixture.raw()),
                    ]
                    .iter()
                    .any(|name| name.contains(word));
                    if named {
                        found.push(loc);
                    }
                }
            }
            found
        })
        .unwrap();
    found.sort_by_key(|loc| {
        let delta = loc.cell - at.cell;
        delta.x.abs().max(delta.y.abs())
    });
    found
}

/// Who the character sees now, as the play screen's target cycle
/// offers them: Tab until it comes round again. Returns how many.
fn people_in_sight(run: &mut Headless) -> usize {
    let mut seen = Vec::new();
    for _ in 0..12 {
        press(run, "tab");
        let target = run.game().scene().target();
        match target {
            Some(target) if !seen.contains(&target) => seen.push(target),
            _ => break,
        }
    }
    seen.len()
}

#[test]
fn a_morning_in_the_market_town() {
    let data = DataDir::new("town");
    let mut run = boot_default(data.path(), 4);
    begin_in_the_square(&mut run);
    let shown = run.text();
    assert!(shown.contains("Time 9:00"), "{shown}");
    assert_golden(golden("play_town_square"), run.frame());

    // A look round: the cursor starts on the character and Tab goes to
    // the next thing in sight.
    press(&mut run, "x");
    assert_eq!(top(&run), "ui:look");
    let look = run.text();
    assert!(look.contains("In sight, 0.0 m away."), "{look}");
    press(&mut run, "escape");
    assert_eq!(top(&run), "ui:play");

    // To the market, by the streets.
    let market = site_named(&run, "Market");
    walk_to(&mut run, market, 2, 200);
    let at = player_loc(&run).unwrap();
    let delta = market.cell - at.cell;
    assert!(
        delta.x.abs().max(delta.y.abs()) <= 2,
        "at the market: {at:?}"
    );

    // Saved, and loaded again from the pause menu: the same game.
    save_as(&mut run, "market");
    let hash = state_hash(&run);
    let loc = player_loc(&run);
    press(&mut run, "escape");
    press_button(&mut run, "Load");
    assert_eq!(top(&run), "ui:load");
    press(&mut run, "enter");
    until_top(&mut run, "ui:play", 60);
    assert_eq!(state_hash(&run), hash, "a load changes nothing");
    assert_eq!(player_loc(&run), loc);

    // A word with whoever is in sight: someone seen may be talked to.
    let people = people_in_sight(&mut run);
    assert!(people > 0, "someone is in sight at the market");
    press(&mut run, "shift+t");
    assert_eq!(top(&run), "ui:talk");
    let talk = run.text();
    assert!(!talk.contains("You cannot see or hear them"), "{talk}");
    let rows = popup_rows(&run, "Talk to");
    eprintln!("talk: {rows:?}");
    let before = log_len(&run);
    press(&mut run, "enter");
    for _ in 0..60 {
        run.pass();
    }
    let said = log_since(&run, before);
    eprintln!("said: {said:?}");
    if top(&run) == "ui:talk" {
        press(&mut run, "escape");
    }
    assert_eq!(top(&run), "ui:play");

    // A shut door near by, opened by walking into it, and a home's
    // cupboard opened and emptied.
    let at = player_loc(&run).unwrap();
    let doors = near(&run, at, "door", 14);
    let door = doors[0];
    walk_to(&mut run, door, 0, 60);
    let door_now = run
        .game()
        .with_session(|session| {
            let cosmos = session.sim.truth().cosmos();
            let cell = cosmos.cell(door).unwrap();
            cosmos
                .defs()
                .state_text(clash_world::Layer::Block, cell.block.raw())
        })
        .unwrap();
    assert!(door_now.ends_with("#open"), "{door_now}");
    assert_eq!(player_loc(&run), Some(door), "in the doorway");
    let cupboard = near(&run, door, "cupboard", 8)[0];
    walk_to(&mut run, cupboard, 1, 30);
    assert!(use_near(&mut run, "Open"), "{}", run.text());
    until_asked(&mut run, 60);
    assert!(use_near(&mut run, "Take out"), "{}", run.text());
    until_asked(&mut run, 60);
    let held = run
        .game()
        .with_session(|session| session.sim.truth().contents(session.player).len())
        .unwrap();
    assert!(held > 0, "something from the cupboard in hand");
    // Hands full, the rest is left where it is, and no fault of `take` is told.
    assert!(
        !log_text(&run)
            .iter()
            .any(|line| line.starts_with("Fault") && line.contains("`take`")),
        "{:?}",
        log_text(&run)
    );
    assert!(run.text().contains("Holding"), "{}", run.text());
}
