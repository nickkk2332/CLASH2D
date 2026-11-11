//! The `save` subcommand (14 §12): `save inspect`, `save verify` and
//! `save list`.
//!
//! | Command | Does |
//! |---------|------|
//! | `save inspect <file>` | Prints what a save says about itself: name, summary, profile, mods and versions, fingerprint, seed, time, state hash |
//! | `save verify <file> [--mods <dir>]` | Loads the save with the profile it names, reads every tract in full and compares the stored hash with what loaded. The first thing wrong is named |
//! | `save list <dir>` | Lists the saves in a directory, as a load screen would |
//!
//! `save migrate` waits for migration entries (04 §12).

use std::fmt::Write as _;
use std::path::Path;
use std::sync::Arc;

use clash_core::Tick;
use clash_save::{SaveMeta, generators_for, list_saves, read_meta, verify};

use crate::data::{EXIT_FAILED, EXIT_OK, EXIT_USAGE, Output};
use crate::scenario::mods_dir;

/// The usage text of the `save` subcommand.
pub const USAGE: &str = "\
saves:
  save inspect <file>
        Print what a save says about itself, without loading it.
  save verify <file> [--mods <dir>]
        Load the save with the profile it names, read every tract and
        compare the stored state hash with what loaded. Exits 1 and
        names what is wrong if anything is.
  save list <dir>
        List the saves in a directory.
";

fn usage(message: &str) -> Output {
    Output {
        out: String::new(),
        err: format!("clash-tools: {message}\n\n{USAGE}"),
        code: EXIT_USAGE,
    }
}

pub(crate) fn failed(message: impl core::fmt::Display) -> Output {
    Output {
        out: String::new(),
        err: format!("clash-tools: {message}\n"),
        code: EXIT_FAILED,
    }
}

/// A save's metadata as lines of text.
fn describe(meta: &SaveMeta) -> String {
    let mut out = String::new();
    let _ = writeln!(out, "name         {}", meta.name);
    if !meta.summary.is_empty() {
        let _ = writeln!(out, "summary      {}", meta.summary);
    }
    let _ = writeln!(out, "format       {}", meta.format);
    let _ = writeln!(
        out,
        "profile      {}",
        if meta.profile.is_empty() {
            "(none)"
        } else {
            &meta.profile
        }
    );
    let mods: Vec<String> = meta
        .mods
        .iter()
        .map(|stamp| format!("{} {}", stamp.id, stamp.version))
        .collect();
    let _ = writeln!(out, "mods         {}", mods.join(", "));
    let _ = writeln!(out, "fingerprint  {}", meta.fingerprint);
    if !meta.experimental.is_empty() {
        let _ = writeln!(out, "experimental {}", meta.experimental.join(", "));
    }
    let _ = writeln!(out, "seed         {}", meta.seed);
    let _ = writeln!(out, "time         {}", meta.tick.since(Tick::ZERO));
    let _ = writeln!(out, "state hash   {}", meta.state_hash);
    out
}

/// The definitions a save names, loaded from the mods directory, with
/// the generators bound to them: what loading the save needs.
pub(crate) fn profile_of(
    meta: &SaveMeta,
    mods: Option<&str>,
    file: &str,
) -> Result<(Arc<clash_data::Defs>, clash_gen::Generators), Output> {
    let dir = mods_dir(mods, Path::new(file).parent().unwrap_or(Path::new("")));
    let types = crate::engine_types();
    let outcome = clash_scene::engine_loader(&types)
        .search_path(dir)
        .load_profile(&meta.profile);
    let defs = outcome.into_result().map_err(|diagnostics| {
        failed(format!(
            "the save's profile `{}` does not load:\n{}",
            meta.profile,
            diagnostics.render()
        ))
    })?;
    let generators = generators_for(&defs).map_err(failed)?;
    Ok((defs, generators))
}

/// Runs the `save` subcommand with the arguments after its name.
pub fn run(args: &[String]) -> Output {
    let mut positional: Vec<&str> = Vec::new();
    let mut mods = None;
    let mut words = args.iter().map(String::as_str);
    while let Some(word) = words.next() {
        match word {
            "--mods" => match words.next() {
                Some(dir) => mods = Some(dir),
                None => return usage("`--mods` needs a directory"),
            },
            flag if flag.starts_with("--") => {
                return usage(&format!("`save` has no option `{flag}`"));
            }
            other => positional.push(other),
        }
    }
    match positional.as_slice() {
        ["inspect", file] => match read_meta(file) {
            Ok(meta) => Output {
                out: describe(&meta),
                err: String::new(),
                code: EXIT_OK,
            },
            Err(error) => failed(error),
        },
        ["verify", file] => {
            let meta = match read_meta(file) {
                Ok(meta) => meta,
                Err(error) => return failed(error),
            };
            let (defs, generators) = match profile_of(&meta, mods, file) {
                Ok(found) => found,
                Err(output) => return output,
            };
            match verify(file, Arc::clone(&defs), generators) {
                Ok(verified) => {
                    let mut out = describe(&verified.meta);
                    let _ = writeln!(
                        out,
                        "verified: {} sections and {} tracts read, and what loads hashes to the stored hash",
                        verified.sections, verified.tracts
                    );
                    for warning in &verified.warnings {
                        let _ = writeln!(out, "warning: {warning}");
                    }
                    Output {
                        out,
                        err: String::new(),
                        code: EXIT_OK,
                    }
                }
                Err(error) => failed(format!("`{file}` does not verify: {error}")),
            }
        }
        ["list", dir] => {
            let mut out = String::new();
            for entry in list_saves(dir) {
                match &entry.meta {
                    Ok(meta) => {
                        let _ = writeln!(
                            out,
                            "{}  {}  {}  t={}",
                            entry.path.display(),
                            meta.name,
                            meta.summary,
                            meta.tick.since(Tick::ZERO)
                        );
                    }
                    Err(error) => {
                        let _ = writeln!(out, "{}  unreadable: {error}", entry.path.display());
                    }
                }
            }
            Output {
                out,
                err: String::new(),
                code: EXIT_OK,
            }
        }
        ["migrate", ..] => failed("`save migrate` is not built: it waits for migration entries"),
        [] => usage("`save` needs a subcommand"),
        _ => usage("`save` takes `inspect <file>`, `verify <file>` or `list <dir>`"),
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use clash_save::host::{GameHost, HostEnv};
    use clash_sim::console::{HostKind, Session};
    use clash_world::{RealmId, RedbStore, TractPos, TractStore};

    use super::*;

    fn args(words: &[&str]) -> Vec<String> {
        words.iter().map(|word| (*word).to_owned()).collect()
    }

    fn repo_mods() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../mods")
    }

    #[test]
    fn a_save_is_inspected_verified_listed_and_a_damaged_tract_is_named() {
        let dir = std::env::temp_dir().join(format!("clash-tools-save-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("game.c2save");
        let path = file.to_str().unwrap();
        let mods = repo_mods();
        let mods = mods.to_str().unwrap();

        // A small game made at the console, saved to a file.
        let env = Arc::new(HostEnv::new(mods));
        let table = GameHost::table();
        let mut host = GameHost::new(env);
        let mut session = Session::new(HostKind::File);
        for line in [
            "use default seed=4",
            "gen flat size=1",
            "set @c:3,3,0 block core:wall",
            "spawn item at @c:4,4,0",
        ] {
            session
                .run_line(&table, &mut host, line)
                .unwrap_or_else(|error| panic!("{line}: {error}"));
        }
        host.save_to(&file, "A small game", "one wall").unwrap();

        let output = run(&args(&["inspect", path]));
        assert_eq!(output.code, EXIT_OK, "{}", output.err);
        assert!(output.out.contains("name         A small game"));
        assert!(output.out.contains("profile      default"));
        assert!(output.out.contains("seed         4"));

        let output = run(&args(&["verify", path, "--mods", mods]));
        assert_eq!(output.code, EXIT_OK, "{}", output.err);
        assert!(output.out.contains("verified: "), "{}", output.out);

        let output = run(&args(&["list", dir.to_str().unwrap()]));
        assert!(
            output.out.contains("A small game  one wall"),
            "{}",
            output.out
        );

        // A tract written over: `verify` names it.
        {
            let mut store = RedbStore::open(&file).unwrap();
            let (realm, tract) = (RealmId::new(0), TractPos::new(0, 0));
            let mut blob = store.get(realm, tract).unwrap().expect("the changed tract");
            let middle = blob.len() / 2;
            blob[middle] ^= 0x55;
            store.put(realm, tract, &blob);
            store.commit().unwrap();
        }
        let output = run(&args(&["verify", path, "--mods", mods]));
        assert_eq!(output.code, EXIT_FAILED);
        assert!(
            output.err.contains("tract (0, 0) of realm 0") || output.err.contains("tract"),
            "{}",
            output.err
        );

        assert_eq!(run(&args(&["inspect", "nowhere.c2save"])).code, EXIT_FAILED);
        assert_eq!(run(&args(&[])).code, EXIT_USAGE);
        assert_eq!(run(&args(&["verify"])).code, EXIT_USAGE);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
