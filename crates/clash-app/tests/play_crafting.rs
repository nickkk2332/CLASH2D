//! Play session: making things (16 §8), from the title, as a player
//! makes them: in the market town's bakery, wood from its rack burnt in
//! its oven, the oven lit, its cupboard of stores opened, and bread
//! baked from what is at hand, two minutes of work in one press of a
//! key, the loaf then taken out of the oven. What else the beta's
//! recipes want (cloth for a bandage, nightshade and know-how for a
//! poison) is told in `docs/playtest-beta.md`.

mod session;

use clash_app::headless::Headless;
use clash_world::Location;
use session::*;

/// The market town is the fourth game of the bundled list.
const TOWN_LIFE: usize = 3;

/// The cells round a place whose block or fixture is named by a word.
fn find_near(run: &Headless, at: Location, word: &str) -> Vec<Location> {
    run.game()
        .with_session(|session| {
            let cosmos = session.sim.truth().cosmos();
            let defs = cosmos.defs();
            let mut found = Vec::new();
            for dy in -14..=14 {
                for dx in -14..=14 {
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
        .unwrap()
}

/// How many things the character holds.
fn holding(run: &Headless) -> usize {
    run.game()
        .with_session(|session| session.sim.truth().contents(session.player).len())
        .unwrap_or_default()
}

#[test]
fn bread_is_baked_in_the_bakery_oven() {
    let data = DataDir::new("bread");
    let mut run = boot_default(data.path(), 4);
    new_game(&mut run, TOWN_LIFE, "Ada");
    until_top(&mut run, "ui:setup", 60);
    press(&mut run, "enter");
    until_top(&mut run, "ui:intro", 60);
    press(&mut run, "enter");
    settle(&mut run);

    // To the bakery, by the streets, and in by its door.
    let bakery = site_named(&run, "Bakery");
    let oven = find_near(&run, bakery, "oven")[0];
    // Wood from the rack at the far end of the room.
    let by_wood = Location::new(oven.grid, oven.cell + clash_core::IVec3::new(-5, -1, 0));
    walk_to(&mut run, by_wood, 0, 300);
    assert_eq!(player_loc(&run), Some(by_wood));
    assert!(use_near(&mut run, "Take out"));
    until_asked(&mut run, 60);
    assert!(holding(&run) > 0, "wood in hand");

    // At the oven: the wood burnt in it, the fire lit, the stores opened.
    let by_oven = Location::new(oven.grid, oven.cell + clash_core::IVec3::new(-1, -1, 0));
    walk_to(&mut run, by_oven, 0, 30);
    press(&mut run, "shift+m");
    assert_eq!(top(&run), "ui:make");
    let make = run.text();
    assert!(make.contains("Bread  south-east"), "{make}");
    press(&mut run, "escape");
    for what in ["Stoke", "Light", "Open"] {
        assert!(use_near(&mut run, what), "{what}: {}", run.text());
        until_asked(&mut run, 60);
    }

    // Bread: two minutes of work in one press of a key.
    press(&mut run, "shift+m");
    let rows = popup_rows(&run, "Make");
    let bread = rows
        .iter()
        .position(|row| row.starts_with("Bread"))
        .unwrap_or_else(|| panic!("no bread in {rows:?}"));
    for _ in 0..bread {
        press(&mut run, "down");
    }
    let before = now_s(&run);
    press(&mut run, "enter");
    until_asked(&mut run, 600);
    assert!(
        now_s(&run) - before >= 120.0,
        "the work took its two minutes"
    );
    let lines = log_text(&run);
    assert!(
        lines
            .iter()
            .any(|line| line.starts_with("Ada finishes the work: what it gave is in ")),
        "{lines:?}"
    );

    // The loaf out of the oven.
    assert!(use_near(&mut run, "Take out"), "{}", run.text());
    until_asked(&mut run, 60);
    assert!(holding(&run) > 0, "the loaf in hand");
}
