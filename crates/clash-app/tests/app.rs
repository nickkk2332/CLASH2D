//! The application round a game (package U11): a total conversion boots
//! into its own screen with nothing of the engine's drawn first; leaving
//! gives the frontend back, which is what restores a terminal;
//! preferences and key bindings are kept in the user's data directory
//! and given back at the next start; and a development profile's
//! omniscient view shows truth and, switched off, leaves none of it
//! behind, while in any other profile its key does nothing.

use std::cell::Cell;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use clash_app::cli::Options;
use clash_app::game::{Game, run_frontend};
use clash_app::headless::Headless;
use clash_app::prefs::PREFS_FILE;
use clash_core::Value;
use clash_scene::{Caps, Frame, Frontend, InputEvent, NullFrontend, assert_golden};
use clash_world::{CellPos, Location};

fn manifest() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn fixture(name: &str) -> PathBuf {
    manifest().join("tests/fixtures").join(name)
}

/// A data directory of the test's own, emptied.
fn data_dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("clash-app-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    dir
}

/// Boots a profile from a mods directory, keeping choices in `data`.
fn boot(mods: &Path, profile: &str, data: &Path) -> Game {
    let options = Options {
        profile: profile.to_owned(),
        mods: Some(mods.to_path_buf()),
        data: Some(data.to_path_buf()),
        ..Options::default()
    };
    clash_app::boot(&options).unwrap_or_else(|problem| panic!("{problem}"))
}

fn bundled(profile: &str, data: &Path) -> Headless {
    Headless::new(boot(&manifest().join("../../mods"), profile, data))
}

fn press(run: &mut Headless, keys: &[&str]) {
    let going = run.keys(keys).unwrap_or_else(|problem| panic!("{problem}"));
    assert!(going, "the game ended while pressing {keys:?}");
}

fn top(run: &Headless) -> Option<String> {
    run.game()
        .scene()
        .top()
        .map(|screen| screen.id().to_owned())
}

fn player(run: &Headless) -> Location {
    run.game()
        .with_session(|session| session.player_loc())
        .flatten()
        .expect("the character is in the world")
}

#[test]
fn a_total_conversion_boots_into_its_own_screen() {
    let data = data_dir("conversion");
    let game = boot(&fixture("conversion/mods"), "other", &data);
    let mods: Vec<String> = game.with_loaded(|loaded| {
        loaded
            .defs
            .mods()
            .iter()
            .map(|loaded_mod| loaded_mod.manifest.id.clone())
            .collect()
    });
    assert_eq!(mods, ["other"], "neither bundled mod is loaded");
    let run = Headless::new(game);
    // The first frame presented is the profile's own boot screen.
    assert_eq!(run.presented(), 1);
    assert_eq!(top(&run).as_deref(), Some("other:front_door"));
    assert!(run.text().starts_with("Another game entirely."));
    assert_golden(
        manifest().join("tests/golden/conversion.frame"),
        run.frame(),
    );
}

/// A frontend that says when it is dropped: a terminal's frontend gives
/// the terminal back when it is.
struct Watched {
    inner: NullFrontend,
    dropped: Rc<Cell<bool>>,
}

impl Frontend for Watched {
    fn caps(&self) -> Caps {
        self.inner.caps()
    }

    fn poll(&mut self) -> Vec<InputEvent> {
        self.inner.poll()
    }

    fn present(&mut self, frame: &Frame) {
        self.inner.present(frame);
    }
}

impl Drop for Watched {
    fn drop(&mut self) {
        self.dropped.set(true);
    }
}

fn watched(keys: &[&str]) -> (Watched, Rc<Cell<bool>>) {
    let dropped = Rc::new(Cell::new(false));
    let mut inner = NullFrontend::new(80, 24);
    for key in keys {
        inner.script([InputEvent::key(key).unwrap()]);
    }
    let front = Watched {
        inner,
        dropped: Rc::clone(&dropped),
    };
    (front, dropped)
}

#[test]
fn quitting_gives_the_frontend_back() {
    let data = data_dir("quit");
    let mut game = boot(&manifest().join("../../mods"), "default", &data);
    // Esc on the title screen quits.
    let (front, dropped) = watched(&["escape"]);
    let result = run_frontend(&mut game, front, |_| None);
    assert_eq!(result, Ok(()));
    assert!(game.scene().quit_requested());
    assert!(
        dropped.get(),
        "the frontend was dropped, and with it the terminal restored"
    );

    // A frontend that breaks is given back too, and the loop says why.
    let mut game = boot(&manifest().join("../../mods"), "default", &data);
    let (front, dropped) = watched(&[]);
    let result = run_frontend(&mut game, front, |_| Some("the line went dead".to_owned()));
    assert_eq!(result, Err("the line went dead".to_owned()));
    assert!(dropped.get());
}

#[test]
fn preferences_and_bindings_are_kept_between_runs() {
    let data = data_dir("prefs");
    let mut run = bundled("default", &data);
    let theme = |run: &Headless| run.game().scene().pref("ui:theme").cloned();
    assert_eq!(theme(&run), Some(Value::Str("ui:default".to_owned())));

    // Options, from the title: the theme to the next, and `w` bound to
    // `north` in play.
    press(&mut run, &["down", "enter"]);
    assert_eq!(top(&run).as_deref(), Some("ui:options"));
    press(&mut run, &["right"]);
    assert_eq!(theme(&run), Some(Value::Str("ui:night".to_owned())));
    // Past the playback speed, the cutscenes, the `ui` mod's own preference
    // of walking into doors and the context.
    press(&mut run, &["tab", "tab", "tab", "tab", "tab"]);
    assert!(run.type_text("w"));
    press(&mut run, &["tab"]);
    assert!(run.type_text("north"));
    press(&mut run, &["tab", "enter"]);
    assert!(run.text().contains("Bound w."), "{}", run.text());

    let stored = std::fs::read_to_string(data.join(PREFS_FILE)).expect("the choices were written");
    let stored: serde_json::Value = serde_json::from_str(&stored).unwrap();
    assert_eq!(stored["preferences"]["ui:theme"], "ui:night");
    assert_eq!(
        stored["bindings"][0],
        serde_json::json!({ "keymap": "ui:default", "context": "play", "key": "w", "name": "north" })
    );

    // The next run starts with them.
    let mut again = bundled("default", &data);
    assert_eq!(theme(&again), Some(Value::Str("ui:night".to_owned())));
    press(&mut again, &["enter", "tab", "tab", "enter"]);
    let before = player(&again);
    press(&mut again, &["w"]);
    let after = player(&again);
    assert_eq!(
        (after.cell.x, after.cell.y),
        (before.cell.x, before.cell.y - 1),
        "`w` steps north, as bound in the earlier run"
    );
    let _ = std::fs::remove_dir_all(&data);
}

/// The cells round a place the view model knows on its storey, each
/// with whether it is seen now.
fn known_cells(run: &Headless, at: Location) -> Vec<(CellPos, bool)> {
    let model = run
        .game()
        .scene()
        .world()
        .expect("a world is shown")
        .model();
    let mut known = Vec::new();
    for y in at.cell.y - 40..=at.cell.y + 40 {
        for x in at.cell.x - 40..=at.cell.x + 40 {
            let cell = CellPos::new(x, y, at.cell.z);
            if let Some(seen) = model.cell(Location::new(at.grid, cell)) {
                known.push((cell, seen.seen()));
            }
        }
    }
    known
}

/// How many cells round a place the view model knows on its storey.
fn known_count(run: &Headless, at: Location) -> usize {
    known_cells(run, at).len()
}

/// Whether the simulation calls every event seen.
fn sim_omniscient(run: &Headless) -> bool {
    run.game()
        .with_session(|session| clash_sim::perception::party::is_omniscient(&session.sim))
        .unwrap()
}

#[test]
fn the_development_profile_can_see_everything() {
    let data = data_dir("omniscient");
    let mut run = bundled("dev", &data);
    press(&mut run, &["enter", "tab", "tab", "enter"]);
    // Into the house: the street is now remembered.
    press(&mut run, &["k", "k", "k", "k"]);
    let at = player(&run);
    let before = known_cells(&run, at);
    let believed = before.len();
    assert!(
        before.iter().any(|(_, seen)| !seen),
        "the street behind the wall is remembered"
    );

    // F12 shows what is really there: more than the character knows.
    // It is not kept for the next run.
    press(&mut run, &["f12"]);
    assert!(
        !data.join(PREFS_FILE).exists(),
        "the omniscient view was not kept"
    );
    assert_eq!(
        run.game().scene().pref("ui:omniscient"),
        Some(&Value::Bool(true))
    );
    let truth = known_count(&run, at);
    assert!(
        truth > believed,
        "{truth} cells from truth, {believed} believed"
    );
    // The simulation calls every event seen while it is on.
    assert!(sim_omniscient(&run));

    // And again goes back to the character's eyes, with nothing learnt
    // from truth kept: the picture is what the character remembers and
    // sees, as it was before.
    press(&mut run, &["f12"]);
    assert_eq!(
        run.game().scene().pref("ui:omniscient"),
        Some(&Value::Bool(false))
    );
    assert!(!sim_omniscient(&run));
    assert_eq!(known_cells(&run, at), before);
    let _ = std::fs::remove_dir_all(&data);
}

#[test]
fn outside_a_development_profile_f12_sees_nothing_more() {
    let data = data_dir("not-omniscient");
    let mut run = bundled("default", &data);
    press(&mut run, &["enter", "tab", "tab", "enter"]);
    let at = player(&run);
    let believed = known_count(&run, at);
    let before = run.text();
    press(&mut run, &["f12"]);
    assert_eq!(
        run.game().scene().pref("ui:omniscient"),
        Some(&Value::Bool(false))
    );
    assert_eq!(known_count(&run, at), believed);
    assert_eq!(run.text(), before);
    let _ = std::fs::remove_dir_all(&data);
}
