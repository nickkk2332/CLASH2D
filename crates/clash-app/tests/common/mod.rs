//! What the headless tests of play share: booting the bundled profile
//! or the `armoury` fixture, running script lines, waiting for playback
//! and reading what the game holds.

#![allow(dead_code, reason = "each test file uses some of these")]

use std::path::{Path, PathBuf};

use clash_app::boot;
use clash_app::cli::Options;
use clash_app::game::{BootOptions, Game};
use clash_app::headless::Headless;
use clash_core::EntityId;
use clash_sim::Command;
use clash_world::Location;

pub fn manifest() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

pub fn mods_dir() -> PathBuf {
    manifest().join("../../mods")
}

pub fn golden(name: &str) -> PathBuf {
    manifest()
        .join("tests/golden")
        .join(format!("{name}.frame"))
}

/// A fresh data directory for one test.
pub fn data_dir(name: &str) -> PathBuf {
    let data = std::env::temp_dir().join(format!("clash-app-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&data);
    data
}

/// The bundled profile, its boot screen on show.
pub fn boot_default(data: &Path) -> Headless {
    let options = Options {
        mods: Some(mods_dir()),
        data: Some(data.to_path_buf()),
        ..Options::default()
    };
    Headless::new(clash_app::boot(&options).unwrap_or_else(|problem| panic!("{problem}")))
}

/// The `armoury` fixture: the bundled mods and two games of its own,
/// `armoury:gifted` and `armoury:range`, first in the list of games.
pub fn boot_armoury(data: &Path) -> Headless {
    let fixture = manifest().join("tests/fixtures/armoury");
    let search = [fixture.join("mods"), mods_dir()];
    let profile = fixture.join("profiles/armoury.json");
    let loaded = boot::load(&search, &profile.to_string_lossy())
        .unwrap_or_else(|problem| panic!("{problem}"));
    let game = Game::boot(
        loaded,
        BootOptions {
            data_dir: Some(data.to_path_buf()),
            ..BootOptions::default()
        },
    )
    .unwrap_or_else(|problem| panic!("{problem}"));
    Headless::new(game)
}

/// Runs lines of a headless script (12 §3.3).
pub fn script(run: &mut Headless, text: &str) {
    run.run_script(text, &mut std::io::sink())
        .unwrap_or_else(|problem| panic!("{problem}"));
    assert!(run.running(), "the game ended in the script");
}

/// From the title: a new game of the scenario at a place in the list,
/// with the character named.
pub fn begin(run: &mut Headless, index: usize, name: &str) {
    script(run, "enter\n");
    script(run, &format!("type {name}\n"));
    script(run, "tab\ntab\n");
    for _ in 0..index {
        script(run, "down\n");
    }
    script(run, "enter\nwait 5\n");
    assert!(run.game().playing(), "a game began");
}

/// Passes with no input until playback has nothing left to play.
pub fn settle(run: &mut Headless) {
    for _ in 0..900 {
        if run.game().scene().playback().is_idle() {
            return;
        }
        run.pass();
    }
    panic!("playback never came to rest");
}

pub fn player(run: &Headless) -> (EntityId, Location) {
    run.game()
        .with_session(|session| (session.player, session.player_loc().unwrap()))
        .expect("a game is being played")
}

pub fn log(run: &Headless) -> Vec<(String, String)> {
    run.game()
        .scene()
        .log()
        .lines()
        .iter()
        .map(|line| (line.category.clone(), line.text.plain()))
        .collect()
}

pub fn top(run: &Headless) -> String {
    run.game()
        .scene()
        .top()
        .map(|screen| screen.id().to_owned())
        .unwrap_or_default()
}

/// Hands the game a command as a tool would.
pub fn tool(run: &mut Headless, command: Command) {
    run.game_mut()
        .submit(command)
        .unwrap_or_else(|problem| panic!("{problem}"));
}

/// How many commands the simulation has taken.
pub fn commands(run: &Headless) -> usize {
    run.game()
        .with_session(|session| session.sim.input_log().len())
        .unwrap_or_default()
}

/// The last command the simulation took.
pub fn last_command(run: &Headless) -> Command {
    run.game()
        .with_session(|session| {
            session
                .sim
                .input_log()
                .entries()
                .last()
                .map(|entry| entry.command.clone())
        })
        .flatten()
        .expect("a command was taken")
}

/// The qualified id of an action, and the names of the parameters a
/// command gave it: the last `Act` or `UseAbility` the simulation took.
pub fn last_act(run: &Headless) -> (String, Vec<String>) {
    run.game()
        .with_session(|session| {
            let actions = session
                .sim
                .defs()
                .try_registry::<clash_sim::action::ActionDef>()
                .unwrap();
            session
                .sim
                .input_log()
                .entries()
                .iter()
                .rev()
                .find_map(|entry| match &entry.command {
                    Command::Act { action, params, .. } => Some((
                        actions.name(*action).to_owned(),
                        params.iter().map(|(name, _)| name.clone()).collect(),
                    )),
                    Command::UseAbility { action, params, .. } => Some((
                        action
                            .map(|action| actions.name(action).to_owned())
                            .unwrap_or_default(),
                        params.iter().map(|(name, _)| name.clone()).collect(),
                    )),
                    _ => None,
                })
        })
        .flatten()
        .expect("an action was started")
}
