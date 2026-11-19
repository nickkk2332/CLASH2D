//! The `scenario` subcommand (14 §7.2): `scenario run <dir> [--bless]`
//! runs every test script under a directory, each in the four modes:
//! straight, twice, with a save and a load at each split point, and
//! with different thread counts.
//!
//! The scripts, the modes and `--bless` are built in
//! `clash_sim::console`. The host they run against is the simulation
//! itself: [`clash_save::host::GameHost`], which loads profiles from a
//! mods directory.
//!
//! # Markers
//!
//! A comment in the lines before a script's first command may say how
//! the script is to be run:
//!
//! | Marker | Means |
//! |--------|-------|
//! | `# host: fake` | The script tests the console itself and uses only the commands of `clash_sim::console::testing::FakeHost`: it is run against that host |
//! | `# known failure: <why>` | The script is the smallest one that shows a bug somebody still has to fix. It is expected to fail; it counts against the run only if it passes, which is the moment to take the marker off |
//!
//! | Exit code | Means |
//! |-----------|-------|
//! | 0 | Every script passed in every mode |
//! | 1 | A script failed or does not parse, or the directory could not be read or holds no script |
//! | 2 | The command line was not understood |

use std::path::{Path, PathBuf};
use std::sync::Arc;

use clash_save::host::{GameHost, HostEnv};
use clash_sim::console::testing::FakeHost;
use clash_sim::console::{
    BlessError, CommandTable, ConsoleHost, Report, Runner, SCRIPT_EXTENSION, ScriptOutcome,
    ScriptReport, parse_script, read_scripts,
};

use crate::data::{DEFAULT_MODS_DIR, EXIT_FAILED, EXIT_OK, EXIT_USAGE, Output};

/// The marker of a script that runs against the fake host.
pub const FAKE_HOST_MARKER: &str = "host: fake";

/// The marker of a script that is expected to fail.
pub const KNOWN_FAILURE_MARKER: &str = "known failure";

/// The usage text of the `scenario` subcommand.
pub const USAGE: &str = "\
test scripts:
  scenario run <dir> [--bless] [--mods <dir>]
        Run every .scn test script under <dir>, each straight, twice,
        with a save and a load at each split point and with different
        thread counts. --bless rewrites expected hashes and frames.
        Profiles and mods are looked for in --mods, else in `mods`
        beside <dir>, else in `mods`.
";

fn usage(message: &str) -> Output {
    Output {
        out: String::new(),
        err: format!("clash-tools: {message}\n\n{USAGE}"),
        code: EXIT_USAGE,
    }
}

/// Whether a script carries a marker: a comment holding it, before the
/// script's first command.
fn has_marker(text: &str, marker: &str) -> bool {
    text.lines()
        .map(str::trim)
        .take_while(|line| line.is_empty() || line.starts_with('#'))
        .any(|line| line.trim_start_matches('#').trim().starts_with(marker))
}

/// The directory mods are looked for in: the one given, else `mods`
/// beside the scripts' directory or above it, else `mods`.
pub fn mods_dir(given: Option<&str>, beside: &Path) -> PathBuf {
    if let Some(dir) = given {
        return PathBuf::from(dir);
    }
    let mut at = Some(beside);
    while let Some(dir) = at {
        let candidate = dir.join(DEFAULT_MODS_DIR);
        if candidate.is_dir() {
            return candidate;
        }
        at = dir.parent();
    }
    PathBuf::from(DEFAULT_MODS_DIR)
}

/// A file named relative to a script, for `expect frame`.
fn beside(script: &str, file: &str) -> PathBuf {
    Path::new(script)
        .parent()
        .unwrap_or(Path::new(""))
        .join(file)
}

/// What the scripts are run against.
struct Hosts {
    real_table: CommandTable,
    fake_table: CommandTable,
    env: Arc<HostEnv>,
}

impl Hosts {
    fn new(mods: PathBuf) -> Hosts {
        Hosts {
            real_table: GameHost::table(),
            fake_table: FakeHost::table(),
            env: Arc::new(HostEnv::new(mods)),
        }
    }

    /// Runs `with` on a runner over the host a script asks for.
    fn runner<R>(&self, path: &str, text: &str, with: impl FnOnce(Runner<'_>) -> R) -> R {
        let read = |file: &str| std::fs::read_to_string(beside(path, file)).ok();
        if has_marker(text, FAKE_HOST_MARKER) {
            let fresh = || -> Box<dyn ConsoleHost> { Box::new(FakeHost::new()) };
            with(Runner::new(&self.fake_table, &fresh).files(&read))
        } else {
            let fresh =
                || -> Box<dyn ConsoleHost> { Box::new(GameHost::new(Arc::clone(&self.env))) };
            with(Runner::new(&self.real_table, &fresh).files(&read))
        }
    }
}

/// Runs the `scenario` subcommand with the arguments after its name.
pub fn run(args: &[String]) -> Output {
    let mut dir = None;
    let mut mods = None;
    let mut bless = false;
    let mut words = args.iter().map(String::as_str);
    match words.next() {
        Some("run") => {}
        Some(other) => return usage(&format!("`scenario` has no subcommand `{other}`")),
        None => return usage("`scenario` needs a subcommand"),
    }
    while let Some(word) = words.next() {
        match word {
            "--bless" => bless = true,
            "--mods" => match words.next() {
                Some(given) => mods = Some(given),
                None => return usage("`--mods` needs a directory"),
            },
            flag if flag.starts_with("--") => {
                return usage(&format!("`scenario run` has no option `{flag}`"));
            }
            path if dir.is_none() => dir = Some(path),
            extra => {
                return usage(&format!(
                    "`scenario run` takes one directory, and `{extra}` is a second"
                ));
            }
        }
    }
    let Some(dir) = dir else {
        return usage("`scenario run` needs a directory");
    };
    let failed = |message: String| Output {
        out: String::new(),
        err: format!("clash-tools: {message}\n"),
        code: EXIT_FAILED,
    };
    let scripts = match read_scripts(Path::new(dir)) {
        Ok(scripts) => scripts,
        Err(error) => return failed(format!("cannot read the scripts under `{dir}`: {error}")),
    };
    if scripts.is_empty() {
        return failed(format!(
            "there is no .{SCRIPT_EXTENSION} script under `{dir}`"
        ));
    }
    let hosts = Hosts::new(mods_dir(mods, Path::new(dir)));
    if bless {
        bless_all(&hosts, &scripts)
    } else {
        run_all(&hosts, &scripts)
    }
}

/// Runs every script in the four modes and reports.
fn run_all(hosts: &Hosts, scripts: &[(String, String)]) -> Output {
    let mut report = Report::default();
    let mut known: Vec<String> = Vec::new();
    let mut err = String::new();
    for (path, text) in scripts {
        let outcome = match parse_script(text) {
            Err(errors) => ScriptOutcome::Syntax(errors),
            Ok(script) => {
                ScriptOutcome::Ran(hosts.runner(path, text, |runner| runner.run(&script)))
            }
        };
        let entry = ScriptReport {
            path: path.clone(),
            outcome,
        };
        if has_marker(text, KNOWN_FAILURE_MARKER) {
            if entry.passed() {
                err.push_str(&format!(
                    "clash-tools: {path} is marked `# {KNOWN_FAILURE_MARKER}` and passes in every \
                     mode: what it shows is mended, so take the marker off and move it to where \
                     it belongs\n"
                ));
                // It counts against the run: a marker that says
                // otherwise than the truth is a mistake.
                report.scripts.push(ScriptReport {
                    path: path.clone(),
                    outcome: ScriptOutcome::Syntax(Vec::new()),
                });
            } else {
                known.push(
                    Report {
                        scripts: vec![entry],
                    }
                    .render(),
                );
            }
            continue;
        }
        report.scripts.push(entry);
    }
    let mut out = report.render();
    for shown in &known {
        // The script's own lines, without the line of totals.
        let lines: Vec<&str> = shown.lines().collect();
        let body = &lines[..lines.len().saturating_sub(1)];
        out.push_str(&format!(
            "known failure, as expected:\n{}\n",
            body.join("\n")
        ));
    }
    if !known.is_empty() {
        out.push_str(&format!(
            "{} known failure{} still failing\n",
            known.len(),
            if known.len() == 1 { "" } else { "s" }
        ));
    }
    let code = if report.passed() {
        EXIT_OK
    } else {
        EXIT_FAILED
    };
    Output { out, err, code }
}

/// Runs every script once and writes the hashes and frames found into
/// the files. A script that fails for another reason is left alone.
fn bless_all(hosts: &Hosts, scripts: &[(String, String)]) -> Output {
    let mut out = String::new();
    let mut err = String::new();
    let mut failed = 0;
    for (path, text) in scripts {
        if has_marker(text, KNOWN_FAILURE_MARKER) {
            out.push_str(&format!("{path}: a known failure, not blessed\n"));
            continue;
        }
        match hosts.runner(path, text, |runner| runner.bless(text)) {
            Ok(blessed) => {
                let mut written = Ok(());
                if blessed.text != *text {
                    written = std::fs::write(path, &blessed.text);
                }
                for (file, frame) in &blessed.frames {
                    written = written.and_then(|()| std::fs::write(beside(path, file), frame));
                }
                match written {
                    Ok(()) => out.push_str(&format!(
                        "{path}: {} hashes and {} frames written\n",
                        blessed.hashes_changed,
                        blessed.frames.len()
                    )),
                    Err(error) => {
                        failed += 1;
                        err.push_str(&format!("clash-tools: cannot write `{path}`: {error}\n"));
                    }
                }
            }
            Err(BlessError::Syntax(errors)) => {
                failed += 1;
                for error in errors {
                    err.push_str(&error.render(path));
                }
            }
            Err(BlessError::Failed(failure)) => {
                failed += 1;
                err.push_str(&format!(
                    "{path}:{}: not blessed: expected {}, found {}\n",
                    failure.line, failure.expected, failure.found
                ));
            }
        }
    }
    if failed > 0 {
        err.push_str(&format!(
            "clash-tools: {failed} of {} scripts could not be blessed\n",
            scripts.len()
        ));
    }
    Output {
        out,
        err,
        code: if failed == 0 { EXIT_OK } else { EXIT_FAILED },
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::*;

    fn args(words: &[&str]) -> Vec<String> {
        words.iter().map(|word| (*word).to_owned()).collect()
    }

    /// A fresh directory of this test's own under the system's
    /// temporary directory.
    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("clash-tools-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// The repository's `mods` directory.
    fn repo_mods() -> String {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../mods")
            .to_string_lossy()
            .into_owned()
    }

    #[test]
    fn the_repository_scripts_pass_in_every_mode() {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../scenarios");
        let output = run(&args(&["run", dir.to_str().unwrap()]));
        assert_eq!(output.code, EXIT_OK, "{}\n{}", output.out, output.err);
        // The console's own script runs against the fake host, and the
        // world's against the simulation.
        assert!(output.out.contains("counter.scn"), "{}", output.out);
        assert!(output.out.contains("wall_punch.scn"), "{}", output.out);
        assert!(
            output.out.contains("save-and-load passed"),
            "{}",
            output.out
        );
        assert!(!output.out.contains("FAILED") || output.out.contains("known failure"));
    }

    #[test]
    fn a_script_runs_against_the_simulation_and_is_blessed() {
        let dir = scratch("scenario-real");
        let script = dir.join("flat.scn");
        fs::write(
            &script,
            "use default seed=3\ngen flat size=1\nspawn creature as=a\n\
             move @a to @c:3,3,0\nassert \"@a.loc.x == 3\"\nexpect hash\n",
        )
        .unwrap();
        let mods = repo_mods();
        let run_it = || run(&args(&["run", dir.to_str().unwrap(), "--mods", &mods]));
        // No hash yet: it fails, saying what the hash is.
        let output = run_it();
        assert_eq!(output.code, EXIT_FAILED, "{}", output.out);
        assert!(output.out.contains("--bless writes in"), "{}", output.out);
        // Blessed, it passes in every mode.
        let blessed = run(&args(&[
            "run",
            dir.to_str().unwrap(),
            "--bless",
            "--mods",
            &mods,
        ]));
        assert_eq!(blessed.code, EXIT_OK, "{}", blessed.err);
        assert!(blessed.out.contains("1 hashes"), "{}", blessed.out);
        let output = run_it();
        assert_eq!(output.code, EXIT_OK, "{}\n{}", output.out, output.err);
        assert!(output.out.contains("1 script, 1 passed, 0 failed"));
        for mode in ["straight", "twice", "save-and-load", "threads"] {
            assert!(output.out.contains(&format!("{mode:<13} passed")), "{mode}");
        }

        // A known failure counts only when it passes.
        fs::write(
            dir.join("broken.scn"),
            "# known failure: one is not two\nuse default\nassert \"1 == 2\"\n",
        )
        .unwrap();
        let output = run_it();
        assert_eq!(output.code, EXIT_OK, "{}\n{}", output.out, output.err);
        assert!(output.out.contains("known failure, as expected"));
        fs::write(
            dir.join("broken.scn"),
            "# known failure: no longer\nuse default\nassert \"1 == 1\"\n",
        )
        .unwrap();
        let output = run_it();
        assert_eq!(output.code, EXIT_FAILED);
        assert!(output.err.contains("take the marker off"), "{}", output.err);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn syntax_errors_are_reported_with_file_and_line() {
        let dir = scratch("scenario-syntax");
        fs::create_dir_all(dir.join("deep")).unwrap();
        fs::write(dir.join("good.scn"), "# host: fake\nnew a\n").unwrap();
        fs::write(
            dir.join("deep/bad.scn"),
            "check\nspawn \"town\nexpect hash 12\n",
        )
        .unwrap();
        fs::write(dir.join("notes.txt"), "not a script \"").unwrap();
        let output = run(&args(&["run", dir.to_str().unwrap()]));
        assert_eq!(output.code, EXIT_FAILED);
        assert!(
            output.out.contains("bad.scn:2:7: error[E1402]"),
            "{}",
            output.out
        );
        assert!(
            output.out.contains("bad.scn:3:13: error[E1442]"),
            "{}",
            output.out
        );
        assert!(output.out.contains("2 scripts, 1 passed, 1 failed"));
        assert!(!output.out.contains("notes.txt"));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_directory_with_no_scripts_or_that_is_missing_fails() {
        let dir = scratch("scenario-empty");
        let output = run(&args(&["run", dir.to_str().unwrap()]));
        assert_eq!(output.code, EXIT_FAILED);
        assert!(
            output.err.contains("there is no .scn script"),
            "{}",
            output.err
        );
        let output = run(&args(&["run", dir.join("missing").to_str().unwrap()]));
        assert_eq!(output.code, EXIT_FAILED);
        assert!(output.err.contains("cannot read"), "{}", output.err);
    }

    #[test]
    fn a_command_line_that_is_not_understood_is_a_usage_error() {
        for words in [
            &[][..],
            &["walk"],
            &["run"],
            &["run", "a", "b"],
            &["run", "a", "--paranoid"],
            &["run", "a", "--mods"],
        ] {
            let output = run(&args(words));
            assert_eq!(output.code, EXIT_USAGE, "{words:?}");
            assert!(
                output.err.contains("scenario run <dir> [--bless]"),
                "{words:?}"
            );
        }
    }

    #[test]
    fn markers_are_read_from_the_comments_before_the_first_command() {
        assert!(has_marker("# host: fake\nnew a\n", FAKE_HOST_MARKER));
        assert!(has_marker(
            "# A script.\n\n#   known failure: because\ncheck\n",
            KNOWN_FAILURE_MARKER
        ));
        assert!(!has_marker("check\n# host: fake\n", FAKE_HOST_MARKER));
        assert!(!has_marker("# the host: fake one\n", FAKE_HOST_MARKER));
    }
}
