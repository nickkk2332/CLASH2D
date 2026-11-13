//! Play sessions in the market town (town life; 09 §3, 15 §9, §11),
//! from the title. A quarrel in the square: a word with someone, who
//! answers a stranger; a blow; and a wardrobe upstairs in a house by
//! the square to hide in. (Struck, the watchman of the square walked on,
//! and nobody came looking while she hid: `docs/playtest-beta.md`, the
//! second round.) And the guardhouse's armoury: its key from the
//! sergeant's desk, the locked door opened with it by walking into it,
//! and a weapon taken from a rack.
//!
//! Waiting in the populated town is slow until performance 2 lands:
//! these sessions are kept short.

mod session;

use clash_app::headless::Headless;
use clash_world::Location;
use session::*;

/// The market town is the fourth game of the bundled list.
const TOWN_LIFE: usize = 3;

/// From the title to the town square, past the setup and the
/// introduction.
fn begin_in_the_square(run: &mut Headless) {
    new_game(run, TOWN_LIFE, "Ada");
    until_top(run, "ui:setup", 60);
    press(run, "down");
    press(run, "enter");
    until_top(run, "ui:intro", 60);
    press(run, "enter");
    settle(run);
    assert_eq!(top(run), "ui:play");
}

/// The cells within a box round a place, on every storey from one below
/// to three above, whose block or fixture is named by a word, nearest
/// first.
fn near(run: &Headless, at: Location, word: &str, reach: i32) -> Vec<Location> {
    let mut found = run
        .game()
        .with_session(|session| {
            let cosmos = session.sim.truth().cosmos();
            let defs = cosmos.defs();
            let mut found = Vec::new();
            for dz in -1..=3 {
                for dy in -reach..=reach {
                    for dx in -reach..=reach {
                        let loc =
                            Location::new(at.grid, at.cell + clash_core::IVec3::new(dx, dy, dz));
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
            }
            found
        })
        .unwrap();
    found.sort_by_key(|loc| {
        let delta = loc.cell - at.cell;
        delta.x.abs().max(delta.y.abs()) + delta.z.abs() * 4
    });
    found
}

/// Answers a blow coming as a player would.
fn answer(run: &mut Headless) {
    if top(run) == "ui:reaction" {
        react(run, &["Step back", "Dodge", "Step aside", "Block"]);
    }
}

/// Says the first thing the talk screen offers to the target.
fn greet(run: &mut Headless) -> Vec<String> {
    press(run, "shift+t");
    assert_eq!(top(run), "ui:talk", "{}", run.text());
    let before = log_len(run);
    press(run, "enter");
    for _ in 0..3 {
        if top(run) == "ui:talk" {
            press(run, "escape");
        }
    }
    for _ in 0..4 {
        answer(run);
        press(run, ".");
    }
    log_since(run, before)
}

#[test]
fn a_word_a_blow_the_watch_and_a_wardrobe() {
    let data = DataDir::new("market-quarrel");
    let mut run = boot_default(data.path(), 4);
    begin_in_the_square(&mut run);

    // Someone in the square: up to them, and a greeting. Nobody here
    // knows the newcomer: they are greeted back as a stranger.
    assert!(tab_until_someone(&mut run, 30), "nobody in the square");
    assert!(walk_up_to_target(&mut run, 40), "{}", run.text());
    let said = greet(&mut run);
    assert!(
        said.iter().any(|line| line.starts_with("Ada speaks to")),
        "{said:?}"
    );
    assert!(said.iter().any(|line| line.contains(" says: ")), "{said:?}");

    // Struck, while they stay in reach.
    let before = log_len(&run);
    for _ in 0..6 {
        answer(&mut run);
        press(&mut run, "f");
        if !["ui:play", "ui:reaction"].contains(&top(&run).as_str()) {
            press(&mut run, "escape");
        }
        press(&mut run, ".");
    }
    let struck = log_since(&run, before);
    assert!(
        struck.iter().any(|line| line.starts_with("Ada strikes")),
        "{struck:?}"
    );

    // Half a minute, then upstairs to the nearest wardrobe, and in.
    for _ in 0..30 {
        answer(&mut run);
        press(&mut run, ".");
    }
    let at = player_loc(&run).unwrap();
    let wardrobe = *near(&run, at, "wardrobe", 30)
        .first()
        .expect("a wardrobe in a house by the square");
    walk_to(&mut run, wardrobe, 1, 120);
    answer(&mut run);
    assert!(use_near(&mut run, "Hide"), "{}", run.text());
    until_asked(&mut run, 60);
    let hidden = run
        .game()
        .with_session(|session| {
            let record = session.sim.truth().entities().get(session.player)?;
            Some(matches!(record.place(), clash_sim::Place::In(_)))
        })
        .flatten();
    assert_eq!(hidden, Some(true), "inside the wardrobe: {}", run.text());
    // Three minutes in the wardrobe: nobody came looking.
    for _ in 0..3 {
        answer(&mut run);
        press(&mut run, "p");
    }
    assert_eq!(top(&run), "ui:play");
}

/// The state name of the block in a cell (`locked`, `closed`, `open`).
fn block_at(run: &Headless, at: Location) -> String {
    run.game()
        .with_session(|session| {
            session.sim.property_named(
                clash_sim::Subject::Element(at, clash_world::Layer::Block),
                "state_name",
                &[],
            )
        })
        .and_then(|value| value.as_str().map(str::to_owned))
        .unwrap_or_default()
}

#[test]
fn the_armoury_opened_with_its_key_and_a_weapon_taken() {
    let data = DataDir::new("market-armoury");
    let mut run = boot_default(data.path(), 4);
    begin_in_the_square(&mut run);

    // To the guardhouse, and to the key on the sergeant's desk.
    let guardhouse = site_named(&run, "Guardhouse");
    walk_to(&mut run, guardhouse, 1, 200);
    let at = player_loc(&run).unwrap();
    let key = lying_near(&run, at, "core:key", 14).expect("the key lies in the guardhouse");
    walk_to(&mut run, key, 1, 40);
    press(&mut run, "g");
    if top(&run) == "ui:pick_list" {
        press(&mut run, "enter");
    }
    until_asked(&mut run, 60);
    assert!(run.text().contains("Holding a key"), "{}", run.text());

    // The armoury's door, locked: walking into it unlocks it with the
    // key in hand, and again opens it.
    let at = player_loc(&run).unwrap();
    let door = (-10..=10)
        .flat_map(|dy| (-10..=10).map(move |dx| (dx, dy)))
        .map(|(dx, dy)| Location::new(at.grid, at.cell + clash_core::IVec3::new(dx, dy, 0)))
        .find(|cell| block_at(&run, *cell) == "locked")
        .expect("the armoury's door is locked");
    walk_to(&mut run, door, 1, 30);
    let step = |run: &mut Headless| {
        let at = player_loc(run).unwrap();
        let delta = door.cell - at.cell;
        let key = match (delta.x.signum(), delta.y.signum()) {
            (0, -1) => "k",
            (0, 1) => "j",
            (-1, 0) => "h",
            (1, 0) => "l",
            (1, -1) => "u",
            (-1, -1) => "y",
            (1, 1) => "n",
            _ => "b",
        };
        press(run, key);
        until_asked(run, 60);
        settle(run);
    };
    step(&mut run);
    assert_eq!(
        block_at(&run, door),
        "closed",
        "unlocked: {:?}",
        log_text(&run)
    );
    step(&mut run);
    assert_eq!(block_at(&run, door), "open", "{:?}", log_text(&run));

    // In, and a weapon taken from the rack.
    walk_to(&mut run, door, 0, 10);
    let at = player_loc(&run).unwrap();
    let racks: Vec<Location> = (-4..=4)
        .flat_map(|dy| (-4..=4).map(move |dx| (dx, dy)))
        .map(|(dx, dy)| Location::new(at.grid, at.cell + clash_core::IVec3::new(dx, dy, 0)))
        .filter(|cell| {
            run.game()
                .with_session(|session| {
                    let cosmos = session.sim.truth().cosmos();
                    let fixture = cosmos.cell(*cell).ok()?.fixture;
                    Some(
                        cosmos
                            .defs()
                            .state_text(clash_world::Layer::Fixture, fixture.raw())
                            .contains("rack"),
                    )
                })
                .flatten()
                .unwrap_or(false)
        })
        .collect();
    let rack = *racks.first().expect("a rack in the armoury");
    walk_to(&mut run, rack, 1, 12);
    assert!(use_near(&mut run, "Take out"), "{}", run.text());
    until_asked(&mut run, 60);
    let held = run.text();
    assert!(
        ["club", "buckler", "bow", "knife"]
            .iter()
            .any(|weapon| held.contains(weapon)),
        "{held}"
    );
}
