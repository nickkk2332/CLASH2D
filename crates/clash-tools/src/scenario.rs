//! The `scenario` subcommand (14 §7.2): `scenario run <dir> [--bless]`
//! runs every test script under a directory.
//!
//! The scripts, the four run modes and `--bless` are built in
//! `clash_sim::console`, against a trait the simulation implements.
//! Until the simulation does, this command has no host to run a script
//! against. It still finds every script under the directory and checks
//! its syntax, reporting each mistake with file, line and column; then
//! it says that no simulation host is registered and exits with
//! [`EXIT_NO_HOST`], so that nobody takes "parsed" for "passed".
//!
//! | Exit code | Means |
//! |-----------|-------|
//! | 0 | Every script passed in every mode |
//! | 1 | A script failed or does not parse, or the directory could not be read or holds no script |
//! | 2 | The command line was not understood |
//! | 3 | The scripts parse, and there is no simulation host to run them |

use std::path::Path;

use clash_sim::console::{SCRIPT_EXTENSION, parse_script, read_scripts};

use crate::data::{EXIT_FAILED, EXIT_USAGE, Output};

/// The process exit code when the scripts parse and there is no
/// simulation host to run them against.
pub const EXIT_NO_HOST: u8 = 3;

/// The usage text of the `scenario` subcommand.
pub const USAGE: &str = "\
test scripts:
  scenario run <dir> [--bless]
        Run every .scn test script under <dir>, each straight, twice,
        with a save and a load at each split point and with different
        thread counts. --bless rewrites expected hashes and frames.
        No simulation host is registered yet: scripts are checked for
        syntax and not run, and the exit code is 3.
";

fn usage(message: &str) -> Output {
    Output {
        out: String::new(),
        err: format!("clash-tools: {message}\n\n{USAGE}"),
        code: EXIT_USAGE,
    }
}

/// Runs the `scenario` subcommand with the arguments after its name.
pub fn run(args: &[String]) -> Output {
    let mut dir = None;
    let mut bless = false;
    let mut words = args.iter().map(String::as_str);
    match words.next() {
        Some("run") => {}
        Some(other) => return usage(&format!("`scenario` has no subcommand `{other}`")),
        None => return usage("`scenario` needs a subcommand"),
    }
    for word in words {
        match word {
            "--bless" => bless = true,
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
    let mut out = String::new();
    let mut err = String::new();
    let mut broken = 0;
    for (path, text) in &scripts {
        match parse_script(text) {
            Ok(script) => {
                let commands = script.commands();
                out.push_str(&format!(
                    "{path}: parsed, {commands} commands and {} expectations\n",
                    script.steps.len() - commands
                ));
            }
            Err(errors) => {
                broken += 1;
                for error in errors {
                    err.push_str(&error.render(path));
                }
            }
        }
    }
    let total = scripts.len();
    let counted = if total == 1 {
        "1 script".to_owned()
    } else {
        format!("{total} scripts")
    };
    if broken > 0 {
        err.push_str(&format!(
            "clash-tools: {broken} of {counted} under `{dir}` do not parse\n"
        ));
        return Output {
            out,
            err,
            code: EXIT_FAILED,
        };
    }
    err.push_str(&format!(
        "clash-tools: no simulation host is registered, so the {counted} under `{dir}` were \
         checked for syntax and not {}\n",
        if bless { "blessed" } else { "run" }
    ));
    Output {
        out,
        err,
        code: EXIT_NO_HOST,
    }
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::PathBuf;

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

    #[test]
    fn the_repository_scripts_parse_and_are_not_run() {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../scenarios");
        let output = run(&args(&["run", dir.to_str().unwrap()]));
        assert_eq!(output.code, EXIT_NO_HOST, "{}", output.err);
        assert!(
            output.out.contains("counter.scn: parsed, "),
            "{}",
            output.out
        );
        assert!(
            output.err.contains("no simulation host is registered"),
            "{}",
            output.err
        );
        assert!(output.err.contains("not run"), "{}", output.err);
        let blessed = run(&args(&["run", dir.to_str().unwrap(), "--bless"]));
        assert_eq!(blessed.code, EXIT_NO_HOST);
        assert!(blessed.err.contains("not blessed"), "{}", blessed.err);
    }

    #[test]
    fn syntax_errors_are_reported_with_file_and_line() {
        let dir = scratch("scenario-syntax");
        fs::create_dir_all(dir.join("deep")).unwrap();
        fs::write(dir.join("good.scn"), "check\nexpect hash\n").unwrap();
        fs::write(
            dir.join("deep/bad.scn"),
            "check\nspawn \"town\nexpect hash 12\n",
        )
        .unwrap();
        fs::write(dir.join("notes.txt"), "not a script \"").unwrap();
        let output = run(&args(&["run", dir.to_str().unwrap()]));
        assert_eq!(output.code, EXIT_FAILED);
        assert!(
            output
                .out
                .contains("good.scn: parsed, 1 commands and 1 expectations"),
            "{}",
            output.out
        );
        assert!(
            output.err.contains("bad.scn:2:7: error[E1402]"),
            "{}",
            output.err
        );
        assert!(
            output.err.contains("bad.scn:3:13: error[E1442]"),
            "{}",
            output.err
        );
        assert!(output.err.contains("1 of 2 scripts"), "{}", output.err);
        assert!(!output.err.contains("notes.txt"));
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
        ] {
            let output = run(&args(words));
            assert_eq!(output.code, EXIT_USAGE, "{words:?}");
            assert!(
                output.err.contains("scenario run <dir> [--bless]"),
                "{words:?}"
            );
        }
    }
}
