//! The widget gallery: every container and widget of the interface
//! toolkit on one screen, in a terminal.
//!
//! ```text
//! cargo run -p clash-front-term --example gallery
//! ```
//!
//! Tab and Shift-Tab move the focus, the arrow keys act on whatever has
//! it, and `q` or Esc quits. F2 opens a second screen that shows what
//! the first cannot: widgets bound to expressions, effects that are run,
//! a popup that asks, a preference that switches the theme and a slot
//! filled by a `ui_part`. The screens, the themes and the keymap are
//! those of the `ui` mod, read by the mod loader from the `mods`
//! directory beside the crates; a different directory of mods may be
//! given as the first argument.
//!
//! There is no game behind it. What the bound widgets show comes from
//! the fake host and the fake application of `clash_scene::testing`.
//!
//! Without a terminal (input or output redirected) the example checks
//! that the definitions load, prints the first frame as plain text and
//! exits successfully.

use std::path::PathBuf;
use std::process::ExitCode;
use std::time::{Duration, Instant};

use clash_data::{Diagnostics, Loader};
use clash_front_term::{TermError, TermFrontend};
use clash_scene::testing::{FakeApp, FakeHost};
use clash_scene::{Frame, NullFrontend, ROLE_CONFIRM, ROLE_PICK_LIST, RichText, Scene, UiDefs};

/// How long the loop rests between frames.
const FRAME_TIME: Duration = Duration::from_millis(16);
/// The grid of the frame printed when there is no terminal.
const HEADLESS_SIZE: (u16, u16) = (80, 24);

/// The mod that holds the gallery.
const GALLERY_MOD: &str = "ui";
/// The theme the gallery is shown with.
const GALLERY_THEME: &str = "ui:default";
/// The keymap the gallery is used with.
const GALLERY_KEYMAP: &str = "ui:default";
/// The gallery's screen.
const GALLERY_SCREEN: &str = "ui:gallery";
/// The screen the `confirm` effect opens.
const CONFIRM_SCREEN: &str = "ui:confirm";
/// The screen the `pick` effect opens for a list.
const PICK_SCREEN: &str = "ui:pick_list";

fn mods_dir() -> PathBuf {
    match std::env::args_os().nth(1) {
        Some(dir) => PathBuf::from(dir),
        None => PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../mods"),
    }
}

fn describe(diagnostics: Diagnostics) -> String {
    diagnostics.render()
}

/// Builds the scene with the gallery screen open and a few lines in the
/// log for the `log` widget to show.
fn gallery() -> Result<Scene<FakeHost>, String> {
    // The types of every crate whose definitions the mod, or a mod it
    // requires, may hold.
    let types = clash_scene::engine_types();
    let loaded = Loader::new(&types)
        .search_path(mods_dir())
        .load_mod_with_requirements(GALLERY_MOD)
        .into_result()
        .map_err(describe)?;
    let defs = UiDefs::from_defs(&loaded).map_err(describe)?;
    // No simulation was registered with the loader, so `view` is `any`
    // and the plain fake host answers it with a map.
    let mut scene = defs
        .scene_over(FakeHost::plain(), GALLERY_THEME, GALLERY_KEYMAP)
        .map_err(describe)?
        .with_app(Box::new(FakeApp::new()))
        .running_effects();
    scene.set_role(ROLE_CONFIRM, CONFIRM_SCREEN);
    scene.set_role(ROLE_PICK_LIST, PICK_SCREEN);
    scene.open_screen(GALLERY_SCREEN, Default::default())?;
    for n in 1..=12 {
        let line = format!("Log line {n}.");
        scene.log_mut().push("demo", RichText::plain_text(&line));
    }
    Ok(scene)
}

fn run() -> Result<(), String> {
    let mut scene = gallery()?;
    let mut frame = Frame::new(0, 0);
    let mut front = match TermFrontend::start(true) {
        Ok(front) => front,
        Err(TermError::NotATerminal) => {
            let mut null = NullFrontend::new(HEADLESS_SIZE.0, HEADLESS_SIZE.1);
            scene.turn(&mut null, &mut frame, 0.0);
            print!("{}", frame.text());
            println!(
                "gallery: not started, because standard input and output are not both a terminal."
            );
            println!(
                "The definitions loaded and the first frame is shown above. Run it in a terminal to use it."
            );
            return Ok(());
        }
        Err(error) => return Err(error.to_string()),
    };
    let mut last = Instant::now();
    loop {
        let now = Instant::now();
        let seconds = now.duration_since(last).as_secs_f64();
        last = now;
        let turn = scene.turn(&mut front, &mut frame, seconds);
        // The scene runs the screens' effects. It is over when the
        // window is closed, an effect asks to quit, or the last screen
        // has been closed.
        if turn.close_requested || scene.top().is_none() {
            return Ok(());
        }
        std::thread::sleep(FRAME_TIME);
    }
}

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(message) => {
            eprintln!("gallery: {message}");
            ExitCode::FAILURE
        }
    }
}
