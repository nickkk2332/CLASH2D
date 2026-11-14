//! What the player sees and hears is the simulation's perception
//! (15 §13; 12 §5, §7.5): a person behind a wall is not drawn until the
//! player's character sees them, and someone moving behind a wall is
//! heard, which puts a marker on the map and a line in the log, and no
//! picture of them.

use std::collections::BTreeSet;
use std::path::PathBuf;

use clash_app::cli::Options;
use clash_app::headless::Headless;
use clash_core::{EntityId, IVec3};
use clash_scene::view::SpriteId;
use clash_sim::template::CharacterSpec;
use clash_sim::{Command, DebugCommand, Place};
use clash_world::{CellPos, Location, MoveProfile, MoveRules};

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

/// The cells the party sees now, as the simulation's perception says.
fn in_view(run: &Headless) -> BTreeSet<Location> {
    run.game()
        .with_session(|session| {
            clash_sim::perception::party::party_view(&session.sim)
                .into_iter()
                .collect()
        })
        .unwrap()
}

/// Whether the view model draws a thing.
fn drawn(run: &Headless, who: EntityId) -> bool {
    run.game()
        .scene()
        .world()
        .unwrap()
        .model()
        .sprite(SpriteId::new(who.to_bits()))
        .is_some()
}

/// Whether a person could stand in a cell.
fn standable(run: &Headless, at: Location) -> bool {
    run.game()
        .with_session(|session| {
            let here = session.sim.truth().cosmos().neighbours(at).ok()?.here;
            let free = session
                .sim
                .truth()
                .cosmos()
                .occupants_at(at, &clash_world::OccupantFilter::ANY)
                .ok()?
                .is_empty();
            Some(free && MoveProfile::walker().can_occupy(&here))
        })
        .flatten()
        .unwrap_or(false)
}

fn tool(run: &mut Headless, command: Command) {
    run.game_mut()
        .submit(command)
        .unwrap_or_else(|problem| panic!("{problem}"));
    run.pass();
}

/// Makes a person from the bundled template at a cell, and gives back
/// who it is.
fn spawn_person(run: &mut Headless, at: Location) -> EntityId {
    let id = run
        .game()
        .with_session(|session| session.sim.truth().entities().next_id())
        .flatten()
        .unwrap();
    tool(
        run,
        Command::Debug(DebugCommand::Spawn {
            spec: CharacterSpec::of(["core:person"]),
            place: Place::at(at),
        }),
    );
    id
}

/// A cell inside the house across the street that the character sees
/// from inside and did not see from the street, found by walking in
/// once: the town is made from the same seed every time.
fn hidden_room_cell() -> (Location, Location) {
    let mut run = start("perceiving-scout");
    press(&mut run, &["enter", "tab", "tab", "down", "down", "enter"]);
    let seen_outside = in_view(&run);
    let (_, outside) = player(&run);
    press(&mut run, &["k", "k", "k", "k"]);
    let (_, inside) = player(&run);
    let seen_inside = in_view(&run);
    // The nearest such cell to the street, so that it is in earshot.
    let far = |loc: &Location| {
        let (dx, dy) = (loc.cell.x - outside.cell.x, loc.cell.y - outside.cell.y);
        (dx * dx + dy * dy, *loc)
    };
    let target = seen_inside
        .iter()
        .copied()
        .filter(|loc| loc.cell.z == inside.cell.z && loc.cell.y < inside.cell.y)
        .filter(|loc| !seen_outside.contains(loc) && *loc != inside)
        .filter(|loc| standable(&run, *loc))
        .min_by_key(far)
        .expect("a room the street does not see");
    (outside, target)
}

#[test]
fn a_person_behind_a_wall_is_not_drawn_until_seen() {
    let (outside, target) = hidden_room_cell();
    let mut run = start("perceiving");
    press(&mut run, &["enter", "tab", "tab", "down", "down", "enter"]);
    assert_eq!(player(&run).1, outside, "the town is the same town");
    let someone = spawn_person(&mut run, target);
    settle(&mut run);
    assert!(
        !in_view(&run).contains(&target),
        "the room is behind a wall"
    );
    assert!(!drawn(&run, someone), "nobody is drawn who is not seen");

    // Into the house: there they are.
    press(&mut run, &["k", "k", "k", "k"]);
    settle(&mut run);
    assert!(in_view(&run).contains(&target));
    assert!(drawn(&run, someone), "someone in view is drawn");
    let sprite = run
        .game()
        .scene()
        .world()
        .unwrap()
        .model()
        .sprite(SpriteId::new(someone.to_bits()))
        .copied()
        .unwrap();
    assert_eq!(sprite.loc, target);
    assert_eq!(
        sprite.glyph.ch,
        Some('@'),
        "drawn as the person they look like"
    );

    // Back out to the street: they are out of sight, and leave the
    // picture.
    press(&mut run, &["j", "j", "j", "j"]);
    settle(&mut run);
    assert!(!drawn(&run, someone));
}

#[test]
fn someone_moving_behind_a_wall_is_heard_not_seen() {
    // The lane, whose doors are shut: up to the door of the house south
    // of it, with someone in the room behind the door.
    let mut run = start("hearing");
    press(
        &mut run,
        &[
            "enter", "tab", "tab", "down", "down", "down", "down", "enter",
        ],
    );
    press(&mut run, &["j", "j", "j", "l", "l"]);
    let (_, at_door) = player(&run);
    let near =
        |dx: i32, dy: i32| Location::new(at_door.grid, at_door.cell.offset(IVec3::new(dx, dy, 0)));
    let hidden =
        |run: &Headless, loc: &Location| standable(run, *loc) && !in_view(run).contains(loc);
    let target = [near(0, 2), near(-1, 2), near(1, 2), near(0, 3)]
        .into_iter()
        .find(|loc| hidden(&run, loc))
        .expect("a room behind the door");
    let someone = spawn_person(&mut run, target);
    settle(&mut run);
    assert!(!drawn(&run, someone), "behind a shut door is not seen");
    // They move to the next free cell of the room, unseen.
    let next = [
        IVec3::new(1, 0, 0),
        IVec3::new(-1, 0, 0),
        IVec3::new(0, 1, 0),
    ]
    .into_iter()
    .map(|step| Location::new(target.grid, target.cell.offset(step)))
    .find(|loc| hidden(&run, loc))
    .expect("room to move in");
    // Being put there was heard too: let its marker go first.
    for _ in 0..120 {
        run.pass();
    }
    assert_eq!(
        run.game().scene().world().unwrap().model().heard().count(),
        0
    );
    let lines = run.game().scene().log().lines().len();
    tool(
        &mut run,
        Command::Debug(DebugCommand::MoveEntity {
            entity: someone,
            to: Place::at(next),
        }),
    );
    let model = run.game().scene().world().unwrap().model();
    let markers: Vec<CellPos> = model.heard().map(|marker| marker.loc.cell).collect();
    assert_eq!(markers, [next.cell], "a marker where it was heard");
    assert!(!drawn(&run, someone), "heard is not seen");
    // Heard within twenty seconds of being put there, the step is marked
    // and not told again (decision D-945): the last line is still the
    // footsteps heard then. That it is told again after a quiet is
    // `clash-scene`'s test of the show's `quiet`.
    let log = run.game().scene().log().lines();
    let last = log.last().unwrap();
    assert_eq!(last.text.plain(), "Footsteps, somewhere close.");
    assert_eq!(log.len(), lines, "no line within the quiet");
    // The marker is shown for a while and then goes.
    for _ in 0..120 {
        run.pass();
    }
    assert_eq!(
        run.game().scene().world().unwrap().model().heard().count(),
        0
    );
}
