//! The game on the real systems (decisions D-652 to D-655): the
//! player's character is made from the `core:person` template, walks
//! through an open door with the bundled `core:step` action, opens a
//! shut one by walking into it (the bundled `core:interact`), and what happens
//! is played back over time, its lines reaching the log as they are
//! played: after the `wait` of a script, not before.

use std::path::PathBuf;

use clash_app::cli::Options;
use clash_app::headless::Headless;
use clash_core::{EntityId, Tick};
use clash_sim::action::ActionDef;
use clash_sim::component::Identity;
use clash_sim::mind::VitalState;
use clash_sim::{Command, DebugCommand, Place};
use clash_world::{CellPos, FloorState, Layer, Location};

fn mods_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../mods")
}

fn start(name: &str) -> Headless {
    let data = std::env::temp_dir().join(format!("clash-app-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&data);
    let options = Options {
        mods: Some(mods_dir()),
        data: Some(data),
        ..Options::default()
    };
    Headless::new(clash_app::boot(&options).unwrap_or_else(|problem| panic!("{problem}")))
}

fn press(run: &mut Headless, keys: &[&str]) {
    let going = run.keys(keys).unwrap_or_else(|problem| panic!("{problem}"));
    assert!(going, "the game ended while pressing {keys:?}");
}

fn script(run: &mut Headless, text: &str) {
    run.run_script(text, &mut std::io::sink())
        .unwrap_or_else(|problem| panic!("{problem}"));
}

/// The player's character, where it is and the simulation's time.
fn player(run: &Headless) -> (EntityId, Location, Tick) {
    run.game()
        .with_session(|session| {
            let at = session.player_loc().expect("the character is in the world");
            (session.player, at, session.sim.now())
        })
        .expect("a game is being played")
}

/// The lines of the log, as plain text, with their categories.
fn log(run: &Headless) -> Vec<(String, String)> {
    run.game()
        .scene()
        .log()
        .lines()
        .iter()
        .map(|line| (line.category.clone(), line.text.plain()))
        .collect()
}

/// How many commands of the game's input log start an action, by the
/// action's qualified id.
fn acts(run: &Headless, action: &str) -> usize {
    run.game()
        .with_session(|session| {
            let id = session.sim.defs().lookup::<ActionDef>(action).unwrap();
            session
                .sim
                .input_log()
                .entries()
                .iter()
                .filter(
                    |entry| matches!(&entry.command, Command::Act { action, .. } if *action == id),
                )
                .count()
        })
        .unwrap()
}

#[test]
fn a_person_walks_through_an_open_door_is_stopped_by_a_shut_one_and_falls_in_the_log() {
    let mut run = start("playing");
    press(&mut run, &["enter"]);
    assert!(run.type_text("Ada"));
    press(&mut run, &["tab", "tab", "enter"]);
    assert!(run.game().playing());

    // Made from the template: its name, a body, a mind awake, and the
    // template's look.
    let (ada, begin, began) = player(&run);
    run.game()
        .with_session(|session| {
            let sim = &session.sim;
            let identity = sim.components().get::<Identity>(ada).unwrap();
            assert_eq!(identity.name(), Some("Ada"));
            assert!(sim.components().body.contains(ada));
            assert_eq!(sim.vital(ada), Some(VitalState::Awake));
            assert_eq!(sim.glyph(ada).map(|glyph| glyph.ch.as_str()), Some("@"));
            // The player is asked what to do, and nothing moves until it
            // answers.
            assert!(sim.pending_decision().is_some());
        })
        .unwrap();

    // North through the open door of the house across the street, a
    // `core:step` at a time; each takes the step's time.
    press(&mut run, &["k", "k", "k", "k"]);
    let (_, inside, walked) = player(&run);
    assert_eq!(
        (inside.cell.x, inside.cell.y),
        (begin.cell.x, begin.cell.y - 4)
    );
    assert!(walked > began);
    assert_eq!(acts(&run, "core:step"), 4);

    // The floor of a room upstairs, taken from under the character as a
    // tool would: it falls. While the player has not answered, time
    // stands still and only what happened at once has been played.
    let above = Location::new(inside.grid, CellPos::new(10, 9, 1));
    run_tool(
        &mut run,
        Command::Debug(DebugCommand::MoveEntity {
            entity: ada,
            to: Place::at(above),
        }),
    );
    run_tool(
        &mut run,
        Command::Debug(DebugCommand::SetElement {
            loc: above,
            layer: Layer::Floor,
            state: FloorState::NONE.raw(),
        }),
    );
    let said = |run: &Headless, line: &str| {
        log(run)
            .iter()
            .any(|(category, text)| category == "event" && text == line)
    };
    assert!(said(&run, "Ada loses their footing."), "{:?}", log(&run));
    assert!(!said(&run, "Ada lands hard."));
    // The wait key lets a second pass: the fall is carried through, and
    // played back over time. Only after the script's `wait` has the
    // landing been played and its line written.
    press(&mut run, &["."]);
    let (_, landed, _) = player(&run);
    assert_eq!(landed, Location::new(inside.grid, CellPos::new(10, 9, 0)));
    assert!(!run.game().scene().playback_caught_up());
    assert!(!said(&run, "Ada lands hard."), "{:?}", log(&run));
    script(&mut run, "wait 60\n");
    assert!(run.game().scene().playback_caught_up());
    assert!(said(&run, "Ada lands hard."), "{:?}", log(&run));
    let lines: Vec<String> = log(&run).into_iter().map(|(_, text)| text).collect();
    let at = |line: &str| lines.iter().position(|have| have == line).unwrap();
    assert!(at("Ada loses their footing.") < at("Ada lands hard."));
    // The picture came to rest where the character is.
    let model = run.game().scene().world().unwrap().model();
    let drawn = model
        .sprite(clash_scene::view::SpriteId::new(ada.to_bits()))
        .map(|sprite| sprite.loc);
    assert_eq!(drawn, Some(landed));
    assert!(
        !log(&run).iter().any(|(category, _)| category == "fault"),
        "{:?}",
        log(&run)
    );

    // Leave, and begin on the lane, whose doors are shut.
    press(
        &mut run,
        &["escape", "down", "down", "down", "enter", "enter"],
    );
    assert!(!run.game().playing());
    press(&mut run, &["enter", "tab", "tab", "down", "enter"]);
    assert!(run.game().playing());
    let (_, begin, _) = player(&run);
    press(&mut run, &["j", "j", "j", "l", "l"]);
    let (_, at_door, then) = player(&run);
    assert_eq!(
        (at_door.cell.x, at_door.cell.y),
        (begin.cell.x + 2, begin.cell.y + 3)
    );
    let logged = run
        .game()
        .with_session(|session| session.sim.input_log().len())
        .unwrap();
    // Into the shut door: the step is refused by movement, and walking
    // into it does what the door offers by default instead (D-701): the
    // bundled `core:interact` opens it. Nothing moves, time passes for
    // the opening, and the one command logged is the interaction's.
    press(&mut run, &["j"]);
    let (_, still, now) = player(&run);
    assert_eq!(still, at_door);
    assert!(now > then, "opening took time");
    assert!(
        !log(&run).iter().any(|(category, _)| category == "refusal"),
        "{:?}",
        log(&run)
    );
    assert_eq!(
        run.game()
            .with_session(|session| session.sim.input_log().len())
            .unwrap(),
        logged + 1
    );
    assert_eq!(acts(&run, "core:interact"), 1);
}

#[test]
fn a_click_on_the_map_walks_there_with_go_to_and_playback_follows() {
    use clash_scene::{InputEvent, Mods, MouseButton, MouseEvent, MouseKind};
    let mut run = start("click");
    press(&mut run, &["enter", "tab", "tab", "enter"]);
    let (who, begin, began) = player(&run);
    // The character is drawn in the middle of the map view: column 26,
    // row 8 of the play screen. Three cells east along the street.
    assert_eq!(
        run.text().lines().nth(8).unwrap().chars().nth(26),
        Some('@')
    );
    run.input(InputEvent::Mouse(MouseEvent {
        kind: MouseKind::Down(MouseButton::Left),
        col: 29,
        row: 8,
        mods: Mods::empty(),
    }));
    let (_, there, then) = player(&run);
    assert_eq!(
        there.cell,
        begin.cell.offset(clash_core::IVec3::new(3, 0, 0))
    );
    assert!(then > began);
    assert_eq!(acts(&run, "core:go_to"), 1);
    // A walk of three steps is one stretch, played over time; once it
    // has been, the character is drawn where it stands.
    assert!(!run.game().scene().playback_caught_up());
    script(&mut run, "wait 60\n");
    let model = run.game().scene().world().unwrap().model();
    let drawn = model
        .sprite(clash_scene::view::SpriteId::new(who.to_bits()))
        .map(|sprite| sprite.loc);
    assert_eq!(drawn, Some(there));
    assert_eq!(
        run.text().lines().nth(8).unwrap().chars().nth(26),
        Some('@')
    );
}

/// Hands the game a command as a tool would, and makes a pass so that
/// what it caused is shown.
fn run_tool(run: &mut Headless, command: Command) {
    run.game_mut()
        .submit(command)
        .unwrap_or_else(|problem| panic!("{problem}"));
    run.pass();
}
