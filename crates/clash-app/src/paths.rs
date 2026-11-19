//! Where things are on disk: the mods, and the user's data directory
//! (04 §3.1, 12 §3.1).
//!
//! # Mods
//!
//! With `--mods dir`, that directory. Without, the first of these that
//! exists: `mods` in the working directory, `mods` beside the
//! executable, and `mods` one and two directories above the executable
//! (where a build run from the repository finds the repository's).
//! Profiles are read from `profiles/` beside the mods directory, and
//! from the `profiles/` of each mod.
//!
//! A `mods` directory inside the user's data directory is searched
//! first, so that a copy there replaces an installed mod of the same
//! id (04 §3.1).
//!
//! # The user's data directory
//!
//! Where preferences and key bindings are kept. The first of:
//!
//! | Source | Directory |
//! |--------|-----------|
//! | `--data dir` | `dir` |
//! | `CLASH_DATA_DIR` | Its value |
//! | `XDG_DATA_HOME` | `$XDG_DATA_HOME/clash2d` |
//! | `APPDATA` (Windows) | `%APPDATA%\clash2d` |
//! | `HOME` on macOS | `$HOME/Library/Application Support/clash2d` |
//! | `HOME` | `$HOME/.local/share/clash2d` |
//!
//! With none of them there is nowhere to keep anything, and choices
//! last for the run.

use std::ffi::OsString;
use std::path::{Path, PathBuf};

/// The environment variable that names the user's data directory,
/// before any of the platform's own.
pub const DATA_DIR_VAR: &str = "CLASH_DATA_DIR";

/// The directory the game keeps its files in, inside the platform's
/// place for such things.
pub const APP_DIR: &str = "clash2d";

/// The directory mods are in, by name.
pub const MODS_DIR: &str = "mods";

/// The directory profiles are in, beside the mods directory.
pub const PROFILES_DIR: &str = "profiles";

/// How many directories above the executable a `mods` directory is
/// looked for in.
const EXE_PARENTS: usize = 3;

/// The user's data directory, from the environment as `env` reads it.
/// `None` if the environment names no place.
pub fn user_data_dir(env: &dyn Fn(&str) -> Option<OsString>) -> Option<PathBuf> {
    let set = |name: &str| {
        env(name)
            .filter(|value| !value.is_empty())
            .map(PathBuf::from)
    };
    if let Some(dir) = set(DATA_DIR_VAR) {
        return Some(dir);
    }
    if let Some(dir) = set("XDG_DATA_HOME") {
        return Some(dir.join(APP_DIR));
    }
    if cfg!(windows)
        && let Some(dir) = set("APPDATA")
    {
        return Some(dir.join(APP_DIR));
    }
    let home = set("HOME")?;
    Some(if cfg!(target_os = "macos") {
        home.join("Library")
            .join("Application Support")
            .join(APP_DIR)
    } else {
        home.join(".local").join("share").join(APP_DIR)
    })
}

/// The user's data directory, from the process's environment.
pub fn default_data_dir() -> Option<PathBuf> {
    user_data_dir(&|name| std::env::var_os(name))
}

/// The places a `mods` directory is looked for in, in order, given the
/// working directory and the executable's path.
pub fn mods_candidates(cwd: Option<&Path>, exe: Option<&Path>) -> Vec<PathBuf> {
    let mut candidates = Vec::new();
    if let Some(cwd) = cwd {
        candidates.push(cwd.join(MODS_DIR));
    }
    let mut dir = exe.and_then(Path::parent);
    for _ in 0..EXE_PARENTS {
        let Some(at) = dir else { break };
        candidates.push(at.join(MODS_DIR));
        dir = at.parent();
    }
    candidates
}

/// The mods directory: the one named, or the first candidate that
/// exists. The error says where it was looked for.
pub fn find_mods(explicit: Option<&Path>) -> Result<PathBuf, String> {
    if let Some(dir) = explicit {
        return if dir.is_dir() {
            Ok(dir.to_path_buf())
        } else {
            Err(format!(
                "there is no directory `{}` to read mods from",
                dir.display()
            ))
        };
    }
    let cwd = std::env::current_dir().ok();
    let exe = std::env::current_exe().ok();
    let candidates = mods_candidates(cwd.as_deref(), exe.as_deref());
    match candidates.iter().find(|dir| dir.is_dir()) {
        Some(dir) => Ok(dir.clone()),
        None => {
            let looked: Vec<String> = candidates
                .iter()
                .map(|dir| format!("`{}`", dir.display()))
                .collect();
            Err(format!(
                "no mods directory was found; looked for {}. Name one with `--mods dir`",
                looked.join(", ")
            ))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn env<'a>(vars: &'a [(&'a str, &'a str)]) -> impl Fn(&str) -> Option<OsString> + 'a {
        move |name| {
            vars.iter()
                .find(|(have, _)| *have == name)
                .map(|(_, value)| OsString::from(value))
        }
    }

    #[test]
    fn the_data_directory_is_the_override_then_the_platforms() {
        let both = [
            (DATA_DIR_VAR, "/tmp/mine"),
            ("XDG_DATA_HOME", "/xdg"),
            ("HOME", "/home/a"),
        ];
        assert_eq!(user_data_dir(&env(&both)), Some(PathBuf::from("/tmp/mine")));
        let xdg = [("XDG_DATA_HOME", "/xdg"), ("HOME", "/home/a")];
        assert_eq!(
            user_data_dir(&env(&xdg)),
            Some(PathBuf::from("/xdg/clash2d"))
        );
        // An empty value names nothing.
        let empty = [(DATA_DIR_VAR, ""), ("XDG_DATA_HOME", "/xdg")];
        assert_eq!(
            user_data_dir(&env(&empty)),
            Some(PathBuf::from("/xdg/clash2d"))
        );
        assert_eq!(user_data_dir(&env(&[])), None);
        if cfg!(all(unix, not(target_os = "macos"))) {
            let home = [("HOME", "/home/a")];
            assert_eq!(
                user_data_dir(&env(&home)),
                Some(PathBuf::from("/home/a/.local/share/clash2d"))
            );
        }
    }

    #[test]
    fn mods_are_looked_for_by_the_working_directory_then_the_executable() {
        let candidates = mods_candidates(
            Some(Path::new("/work")),
            Some(Path::new("/repo/target/debug/clash")),
        );
        assert_eq!(
            candidates,
            [
                PathBuf::from("/work/mods"),
                PathBuf::from("/repo/target/debug/mods"),
                PathBuf::from("/repo/target/mods"),
                PathBuf::from("/repo/mods"),
            ]
        );
        assert!(find_mods(Some(Path::new("/no/such/place"))).is_err());
    }
}
