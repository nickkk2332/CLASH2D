//! The `replay` subcommand (14 §8, as far as the kernel's input log
//! allows it now): `replay <from> <to>` loads the earlier save, replays
//! the commands the later one's input log holds beyond the earlier
//! one's, runs on to the later save's tick, and says whether the state
//! is the later save's, naming each subsystem whose hash differs.
//!
//! The input log carries no state hashes at intervals yet, so the
//! first tick at which two runs part cannot be found: a divergence is
//! seen at the end and named by subsystem. Changes the console makes
//! without a command (`damage`, `field`, `gen`) are not logged and so
//! are not replayed.

use std::fmt::Write as _;
use std::sync::Arc;

use clash_core::Tick;
use clash_save::{load_game, read_meta, replay_between};

use crate::data::{EXIT_FAILED, EXIT_OK, EXIT_USAGE, Output};
use crate::save::{failed, profile_of};

/// The usage text of the `replay` subcommand.
pub const USAGE: &str = "\
replay:
  replay <from> <to> [--mods <dir>]
        Load the save <from>, replay the commands the later save <to>
        accepted since, run on to its time and compare the state with
        it, subsystem by subsystem. Exits 1 if they differ.
";

fn usage(message: &str) -> Output {
    Output {
        out: String::new(),
        err: format!("clash-tools: {message}\n\n{USAGE}"),
        code: EXIT_USAGE,
    }
}

/// Runs the `replay` subcommand with the arguments after its name.
pub fn run(args: &[String]) -> Output {
    let mut files: Vec<&str> = Vec::new();
    let mut mods = None;
    let mut words = args.iter().map(String::as_str);
    while let Some(word) = words.next() {
        match word {
            "--mods" => match words.next() {
                Some(dir) => mods = Some(dir),
                None => return usage("`--mods` needs a directory"),
            },
            flag if flag.starts_with("--") => {
                return usage(&format!("`replay` has no option `{flag}`"));
            }
            other => files.push(other),
        }
    }
    let [from, to] = files.as_slice() else {
        return usage("`replay` takes two saves: the one to replay from and the one to replay to");
    };
    let meta = match read_meta(to) {
        Ok(meta) => meta,
        Err(error) => return failed(error),
    };
    let (defs, generators) = match profile_of(&meta, mods, to) {
        Ok(found) => found,
        Err(output) => return output,
    };
    let (mut start, _) = match load_game(from, Arc::clone(&defs), generators.clone()) {
        Ok(loaded) => loaded,
        Err(error) => return failed(format!("`{from}` does not load: {error}")),
    };
    let (end, _) = match load_game(to, Arc::clone(&defs), generators) {
        Ok(loaded) => loaded,
        Err(error) => return failed(format!("`{to}` does not load: {error}")),
    };
    let replayed = match replay_between(&mut start, &end) {
        Ok(replayed) => replayed,
        Err(error) => return failed(format!("`{from}` does not replay to `{to}`: {error}")),
    };
    let mut out = format!(
        "replayed {} commands to t={}\n",
        replayed.commands,
        replayed.tick.since(Tick::ZERO)
    );
    if replayed.same() {
        let _ = writeln!(out, "the same: every subsystem hashes as in `{to}`");
        return Output {
            out,
            err: String::new(),
            code: EXIT_OK,
        };
    }
    for ((name, expected), (_, found)) in replayed.expected.iter().zip(&replayed.found) {
        let mark = if expected == found { "same" } else { "DIFFERS" };
        let _ = writeln!(out, "{name:<10} {mark:<7} {expected} {found}");
    }
    Output {
        out,
        err: format!(
            "clash-tools: the replay differs from `{to}` in {}; the log carries no hashes at intervals, so the tick at which they parted is not known\n",
            replayed
                .differing()
                .iter()
                .filter(|name| **name != "total")
                .copied()
                .collect::<Vec<_>>()
                .join(", ")
        ),
        code: EXIT_FAILED,
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use clash_save::host::{GameHost, HostEnv};
    use clash_sim::console::{HostKind, Session};

    use super::*;

    fn args(words: &[&str]) -> Vec<String> {
        words.iter().map(|word| (*word).to_owned()).collect()
    }

    #[test]
    fn the_commands_between_two_saves_replay_and_a_change_outside_the_log_is_named() {
        let dir = std::env::temp_dir().join(format!("clash-tools-replay-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let mods = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../mods");
        let mods = mods.to_str().unwrap();
        let file = |name: &str| dir.join(name).to_string_lossy().into_owned();

        let env = Arc::new(HostEnv::new(mods));
        let table = GameHost::table();
        let mut host = GameHost::new(env);
        let mut session = Session::new(HostKind::File);
        let mut type_line = |host: &mut GameHost, line: &str| {
            session
                .run_line(&table, host, line)
                .unwrap_or_else(|error| panic!("{line}: {error}"));
        };
        type_line(&mut host, "use default seed=6");
        type_line(&mut host, "gen flat size=1");
        type_line(&mut host, "spawn creature at=@c:3,3,0 as=a");
        host.save_to(file("from.c2save"), "from", "").unwrap();
        type_line(&mut host, "advance 1s");
        type_line(&mut host, "move @a @c:9,9,0");
        type_line(&mut host, "set @c:5,5,0 block core:wall");
        type_line(&mut host, "advance 2s");
        host.save_to(file("to.c2save"), "to", "").unwrap();
        // A change the input log does not hold.
        type_line(&mut host, "field @c:7,7,0 core:smoke 50");
        host.save_to(file("off.c2save"), "off", "").unwrap();

        let output = run_replay(&file("from.c2save"), &file("to.c2save"), mods);
        assert_eq!(output.code, EXIT_OK, "{}{}", output.out, output.err);
        assert!(
            output.out.contains("replayed 2 commands to t=3s"),
            "{}",
            output.out
        );
        assert!(output.out.contains("the same"), "{}", output.out);

        let output = run_replay(&file("from.c2save"), &file("off.c2save"), mods);
        assert_eq!(output.code, EXIT_FAILED, "{}", output.out);
        assert!(output.out.contains("world      DIFFERS"), "{}", output.out);
        assert!(output.err.contains("differs from"), "{}", output.err);

        // Backwards, and saves of nothing.
        let output = run_replay(&file("to.c2save"), &file("from.c2save"), mods);
        assert_eq!(output.code, EXIT_FAILED);
        assert!(output.err.contains("before the save"), "{}", output.err);
        assert_eq!(super::run(&args(&["only-one.c2save"])).code, EXIT_USAGE);
        assert_eq!(
            super::run(&args(&["a.c2save", "b.c2save"])).code,
            EXIT_FAILED
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    fn run_replay(from: &str, to: &str, mods: &str) -> Output {
        run(&args(&[from, to, "--mods", mods]))
    }
}
