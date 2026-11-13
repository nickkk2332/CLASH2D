//! The CLASH2D application: what a person starts. It loads a profile,
//! shows its boot screen, starts a world, and runs the loop that hands
//! input to the scene, advances the simulation and presents frames
//! through a frontend. The binary `clash` is [`run`] and nothing more.
//! Specified in `docs/12-presentation-and-ui.md` section 3.
//!
//! This crate is outside the simulation (01 §2): it may read the wall
//! clock and the file system. It names no content: every screen, realm
//! and control it uses is named by the profile and its mods.
//!
//! # Where things are
//!
//! | Need | Use |
//! |------|-----|
//! | The command line | [`cli`] |
//! | Where mods and the user's data are | [`paths`] |
//! | A profile, loaded | [`boot::load`], a [`boot::Loaded`] |
//! | The application running: boot, one pass of the loop, the loop | [`game::Game`], [`game::run_frontend`] |
//! | What `app` reads and the `app` effect asks | [`app`] |
//! | The character being made: `app.creation` and the service `creation` | [`creation`] |
//! | A game being played | [`session`]: [`session::new_game`], [`session::spawn_player`] |
//! | The scene's handle on the simulation, and the commands it submits | [`host::GameHost`], [`host::command_for`] |
//! | Preferences and key bindings between runs | [`prefs`] |
//! | Saving and loading | [`saves`]: [`saves::SaveBackend`], [`saves::FileSaves`] over `clash-save` |
//! | What the party perceives, for the view model and playback | [`perception`] |
//! | Running from a script, without a terminal | [`headless`] |
//!
//! # Seams
//!
//! Nothing stands in for a package any more. A game begins from a game
//! scenario (13 §3, [`session`]); the simulation gives the party senses
//! as the game begins, where templates give none (15 §15).
//!
//! # Rules this crate keeps
//!
//! - The simulation is advanced only by [`game::Game::pass`], through
//!   `Sim::advance`, and changed only through `Sim::submit`, but for
//!   the debugging switch `perception.omniscient` and saving, which
//!   changes nothing hashed.
//! - Nothing an ordinary screen shows reads truth: the view model is
//!   changed by playback and by what the simulation's perception says
//!   the party sees and remembers ([`perception`]), and bindings read
//!   through [`host::GameHost`].
//! - Bad data is a located message, never a panic: before a scene
//!   exists it is printed and the program leaves with a failure; after,
//!   it is a line in the log and the screen of the role `error`.
//! - The terminal is given back however the program leaves.

pub mod app;
pub mod boot;
pub mod cli;
pub mod creation;
pub mod game;
pub mod headless;
pub mod host;
pub mod interact;
pub mod paths;
pub mod perception;
pub mod prefs;
pub mod saves;
pub mod session;

use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use clash_front_term::{TermError, TermFrontend};

use crate::cli::Options;
use crate::game::{BootOptions, Game};
use crate::headless::Headless;

/// The exit code of a command line that could not be read.
const EXIT_USAGE: u8 = 2;

/// Loads the profile the options name and boots the application on it.
/// A problem is returned as text to print: located diagnostics, or a
/// sentence.
pub fn boot(options: &Options) -> Result<Game, String> {
    let mods = paths::find_mods(options.mods.as_deref())?;
    let data_dir = options.data.clone().or_else(paths::default_data_dir);
    // A copy of a mod in the user's data directory replaces an
    // installed one of the same id (04 §3.1).
    let mut search: Vec<PathBuf> = Vec::new();
    if let Some(own) = data_dir.as_ref().map(|dir| dir.join(paths::MODS_DIR))
        && own.is_dir()
        && own != mods
    {
        search.push(own);
    }
    search.push(mods);
    let loaded = boot::load(&search, &options.profile)?;
    Game::boot(
        loaded,
        BootOptions {
            seed: options.seed,
            data_dir,
            ..BootOptions::default()
        },
    )
}

/// Reads a headless script: a file, or standard input for `-`.
fn read_script(path: &Path) -> Result<String, String> {
    if path == Path::new("-") {
        let mut text = String::new();
        std::io::stdin()
            .read_to_string(&mut text)
            .map_err(|error| {
                format!("the script could not be read from standard input: {error}")
            })?;
        return Ok(text);
    }
    std::fs::read_to_string(path)
        .map_err(|error| format!("the script `{}` could not be read: {error}", path.display()))
}

/// Runs the game in the terminal until the player leaves. Without a
/// terminal it says so and returns.
fn run_terminal(mut game: Game) -> Result<(), String> {
    let front = match TermFrontend::start(true) {
        Ok(front) => front,
        Err(TermError::NotATerminal) => {
            println!(
                "clash: not started, because standard input and output are not both a terminal."
            );
            println!(
                "The profile loaded and its boot screen `{}` is ready. Run clash in a terminal to play, or give it a script of key names with `--headless script`.",
                game.scene().top().map_or("", |screen| screen.id())
            );
            return Ok(());
        }
        Err(error) => return Err(error.to_string()),
    };
    game::run_frontend(&mut game, front, |front| {
        front
            .take_error()
            .map(|error| format!("the terminal failed: {error}"))
    })
}

/// The program: reads the arguments that follow its name, starts the
/// profile and runs it, in a terminal or from a script. Returns the
/// process's exit code: success, or failure with what went wrong
/// printed on standard error.
pub fn run(args: impl IntoIterator<Item = String>) -> ExitCode {
    let options = match cli::parse(args) {
        Ok(options) => options,
        Err(problem) => {
            eprintln!("clash: {problem}\n\n{}", cli::USAGE);
            return ExitCode::from(EXIT_USAGE);
        }
    };
    if options.help {
        print!("{}", cli::USAGE);
        return ExitCode::SUCCESS;
    }
    let result = boot(&options).and_then(|game| match &options.headless {
        Some(script) => {
            let script = read_script(script)?;
            let mut out = std::io::stdout().lock();
            Headless::new(game).run_script(&script, &mut out)
        }
        None => run_terminal(game),
    });
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(problem) => {
            eprintln!("clash: the game could not be run.\n{}", problem.trim_end());
            ExitCode::FAILURE
        }
    }
}
