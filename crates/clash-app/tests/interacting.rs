//! Interactions in play (16 §3; 12 §3.2, §9.4), headless from the title
//! screen: walking into a shut door opens it and the character walks
//! through; a locked door refuses with its reason until the character
//! holds its key; with the preference `bump` at `ask` the player is
//! asked first; and the `interact` key lists what is near, by which the
//! character hides in a locker and comes out again. Golden frames are
//! kept of the opened door, the way through and the locked door.
//!
//! The golden frames are written with `BLESS=1 cargo test -p clash-app`.

use std::path::{Path, PathBuf};

use clash_app::cli::Options;
use clash_app::headless::Headless;
use clash_core::{EntityId, IVec3};
use clash_expr::Value;
use clash_scene::assert_golden;
use clash_sim::{Command, DebugCommand, Place, Subject};
use clash_world::{Layer, Location};

fn mods_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../mods")
}

fn golden(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/golden")
        .join(format!("{name}.frame"))
}

fn data_dir(name: &str) -> PathBuf {
    let data = std::env::temp_dir().join(format!("clash-app-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&data);
    data
}

fn boot(data: &Path) -> Headless {
    let options = Options {
        mods: Some(mods_dir()),
        data: Some(data.to_path_buf()),
        ..Options::default()
    };
    Headless::new(clash_app::boot(&options).unwrap_or_else(|problem| panic!("{problem}")))
}

/// Runs lines of a headless script (12 §3.3).
fn script(run: &mut Headless, text: &str) {
    run.run_script(text, &mut std::io::sink())
        .unwrap_or_else(|problem| panic!("{problem}"));
    assert!(run.running(), "the game ended in the script");
}

/// Passes with no input until playback has nothing left to play.
fn settle(run: &mut Headless) {
    for _ in 0..600 {
        if run.game().scene().playback().is_idle() {
            return;
        }
        run.pass();
    }
    panic!("playback never came to rest");
}

fn player(run: &Headless) -> (EntityId, Location) {
    run.game()
        .with_session(|session| (session.player, session.player_loc().unwrap()))
        .expect("a game is being played")
}

fn log(run: &Headless) -> Vec<(String, String)> {
    run.game()
        .scene()
        .log()
        .lines()
        .iter()
        .map(|line| (line.category.clone(), line.text.plain()))
        .collect()
}

fn top(run: &Headless) -> String {
    run.game()
        .scene()
        .top()
        .map(|screen| screen.id().to_owned())
        .unwrap_or_default()
}

/// The state of the block of a cell, by name.
fn block_state(run: &Headless, at: Location) -> String {
    run.game()
        .with_session(|session| {
            session
                .sim
                .property_named(Subject::Element(at, Layer::Block), "state_name", &[])
        })
        .and_then(|value| value.as_str().map(str::to_owned))
        .unwrap_or_default()
}

/// Hands the game a command as a tool would.
fn tool(run: &mut Headless, command: Command) {
    run.game_mut()
        .submit(command)
        .unwrap_or_else(|problem| panic!("{problem}"));
    run.pass();
}

/// Puts an element, written as a map writes it, into a cell.
fn put(run: &mut Headless, at: Location, layer: Layer, written: &str) {
    let state = run
        .game()
        .with_session(|session| {
            let world = session.sim.truth().cosmos().defs();
            match layer {
                Layer::Floor => world.floor_state(written, None).map(|s| s.raw()),
                Layer::Block => world.block_state(written, None).map(|s| s.raw()),
                Layer::Fixture => world.fixture_state(written, None).map(|s| s.raw()),
            }
        })
        .unwrap()
        .unwrap_or_else(|error| panic!("{written}: {error}"));
    tool(
        run,
        Command::Debug(DebugCommand::SetElement {
            loc: at,
            layer,
            state,
        }),
    );
}

/// Begins on the lane, whose doors are shut, and walks to the door of
/// the house south of it. Gives the cell of the door.
fn to_the_door(run: &mut Headless) -> Location {
    script(
        run,
        "enter\ntab\ntab\ndown\ndown\ndown\ndown\nenter\nj\nj\nj\nl\nl\nwait 60\n",
    );
    let (_, at) = player(run);
    Location::new(at.grid, at.cell.offset(IVec3::new(0, 1, 0)))
}

#[test]
fn walking_into_a_shut_door_opens_it_and_through_it_goes() {
    let data = data_dir("bump");
    let mut run = boot(&data);
    let door = to_the_door(&mut run);
    assert_eq!(block_state(&run, door), "closed");
    let (_, at_door) = player(&run);

    // Into the door: it is opened, not refused.
    script(&mut run, "j\nwait 60\n");
    assert_eq!(block_state(&run, door), "open");
    assert_eq!(player(&run).1, at_door, "opening is not a step");
    assert!(
        !log(&run).iter().any(|(category, _)| category == "refusal"),
        "{:?}",
        log(&run)
    );
    settle(&mut run);
    assert_golden(golden("door_opened"), run.frame());

    // And through it, into the house.
    script(&mut run, "j\nj\nwait 60\n");
    let (_, inside) = player(&run);
    assert_eq!(inside.cell, door.cell.offset(IVec3::new(0, 1, 0)));
    settle(&mut run);
    assert_golden(golden("through_door"), run.frame());
    assert!(!log(&run).iter().any(|(category, _)| category == "fault"));
    let _ = std::fs::remove_dir_all(&data);
}

#[test]
fn a_locked_door_says_why_until_its_key_is_in_hand() {
    let data = data_dir("locked");
    let mut run = boot(&data);
    let door = to_the_door(&mut run);
    put(&mut run, door, Layer::Block, "core:lockable_door#locked");
    settle(&mut run);
    let (hero, at_door) = player(&run);

    // Locked, and no key: the door's own reason, and nothing moves.
    script(&mut run, "j\nwait 60\n");
    assert_eq!(
        log(&run).last(),
        Some(&(
            "refusal".to_owned(),
            "It does not open. It is locked, and you have no key.".to_owned()
        ))
    );
    assert_eq!(block_state(&run, door), "locked");
    assert_eq!(player(&run).1, at_door);
    settle(&mut run);
    assert_golden(golden("door_locked"), run.frame());

    // With the key in a pocket: unlocked, then opened, then through.
    tool(
        &mut run,
        Command::Debug(DebugCommand::CreateItem {
            def: "core:key".to_owned(),
            count: 1,
            place: Place::In(hero),
        }),
    );
    script(&mut run, "j\nwait 60\n");
    assert_eq!(block_state(&run, door), "closed", "unlocked");
    script(&mut run, "j\nwait 60\n");
    assert_eq!(block_state(&run, door), "open");
    script(&mut run, "j\nj\nwait 60\n");
    assert_eq!(player(&run).1.cell, door.cell.offset(IVec3::new(0, 1, 0)));
    let _ = std::fs::remove_dir_all(&data);
}

#[test]
fn asked_first_the_door_opens_only_on_yes() {
    let data = data_dir("bump-ask");
    std::fs::create_dir_all(&data).unwrap();
    std::fs::write(
        data.join("preferences.json"),
        r#"{ "preferences": { "ui:bump": "ask" }, "bindings": [] }"#,
    )
    .unwrap();
    let mut run = boot(&data);
    let door = to_the_door(&mut run);

    // Asked, and told no: nothing.
    script(&mut run, "j\n");
    assert_eq!(top(&run), "ui:confirm");
    script(&mut run, "escape\nwait 60\n");
    assert_eq!(top(&run), "ui:play");
    assert_eq!(block_state(&run, door), "closed");

    // Asked, and told yes: opened.
    script(&mut run, "j\n");
    assert_eq!(top(&run), "ui:confirm");
    script(&mut run, "enter\nwait 60\n");
    assert_eq!(top(&run), "ui:play");
    assert_eq!(block_state(&run, door), "open");
    let _ = std::fs::remove_dir_all(&data);
}

/// The row of the `interact` screen that offers an interaction, by its
/// qualified id, and whether it can be done.
fn offer_row(run: &Headless, id: &str) -> (usize, bool) {
    run.game()
        .with_session(|session| {
            let offers =
                session
                    .sim
                    .property_named(Subject::Entity(session.player), "offers_near", &[]);
            offers
                .as_list()
                .unwrap()
                .iter()
                .enumerate()
                .find_map(|(row, offer)| {
                    let offer = offer.as_map()?;
                    (offer.get("id")?.as_str()? == id)
                        .then(|| (row, offer.get("available") == Some(&Value::Bool(true))))
                })
        })
        .flatten()
        .unwrap_or_else(|| panic!("nothing near offers {id}"))
}

/// Chooses an interaction from the `interact` screen.
fn choose(run: &mut Headless, id: &str) {
    script(run, "e\n");
    assert_eq!(top(run), "ui:interact");
    let (row, available) = offer_row(run, id);
    assert!(available, "{id} can be done");
    let downs = "down\n".repeat(row);
    script(run, &format!("{downs}enter\nwait 90\n"));
    assert_eq!(top(run), "ui:play");
}

#[test]
fn hiding_in_a_locker_and_coming_out() {
    let data = data_dir("locker");
    let mut run = boot(&data);
    script(&mut run, "enter\ntab\ntab\ndown\ndown\ndown\ndown\nenter\n");
    let (hero, begin) = player(&run);
    let beside = Location::new(begin.grid, begin.cell.offset(IVec3::new(1, 0, 0)));
    put(&mut run, beside, Layer::Fixture, "core:steel_locker");
    settle(&mut run);

    // The list of what is near offers to hide in it; done, the
    // character is inside it, out of sight.
    choose(&mut run, "core:hide_inside");
    let place = run
        .game()
        .with_session(|session| session.sim.truth().place(hero))
        .flatten();
    assert!(matches!(place, Some(Place::In(_))), "{place:?}");
    assert_eq!(player(&run).1, beside, "where the locker is");
    // Hidden, there is no hiding again; there is getting out.
    let hide_again = run
        .game()
        .with_session(|session| {
            session
                .sim
                .property_named(Subject::Entity(hero), "offers_near", &[])
                .as_list()
                .unwrap()
                .iter()
                .any(|offer| {
                    offer
                        .as_map()
                        .and_then(|offer| offer.get("id"))
                        .and_then(Value::as_str)
                        == Some("core:hide_inside")
                })
        })
        .unwrap();
    assert!(!hide_again);

    choose(&mut run, "core:get_out");
    let place = run
        .game()
        .with_session(|session| session.sim.truth().place(hero))
        .flatten();
    assert!(matches!(place, Some(Place::At { .. })), "{place:?}");
    assert!(!log(&run).iter().any(|(category, _)| category == "fault"));
    let _ = std::fs::remove_dir_all(&data);
}
