//! The workspace's task runner.
//!
//! `cargo xtask check` is the gate every change must pass (see `CLAUDE.md`).
//! It runs, in order:
//!
//! 1. `cargo fmt --all --check`
//! 2. `cargo clippy --workspace --all-targets` with warnings denied
//! 3. `cargo test --workspace`
//! 4. `cargo doc --workspace --no-deps` with warnings denied
//!
//! Every step runs even when an earlier one fails, so one run shows
//! everything that is wrong. `cargo xtask <step>` runs a single step by name
//! (`fmt`, `clippy`, `test`, `doc`).
//!
//! The check that the literals for chunk, tract and region sizes appear only
//! in `clash_world::dims` joins this list with work package W1, which creates
//! that module.

use std::env;
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};
use std::time::{Duration, Instant};

/// One step of `cargo xtask check`: a cargo command and the environment it
/// needs.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Step {
    /// The name used on the command line and in the summary.
    name: &'static str,
    /// Arguments passed to cargo.
    args: &'static [&'static str],
    /// Environment variables set for the command.
    env: &'static [(&'static str, &'static str)],
}

/// The steps of `cargo xtask check`, in the order they run.
const STEPS: &[Step] = &[
    Step {
        name: "fmt",
        args: &["fmt", "--all", "--check"],
        env: &[],
    },
    Step {
        name: "clippy",
        args: &[
            "clippy",
            "--workspace",
            "--all-targets",
            "--",
            "-D",
            "warnings",
        ],
        env: &[],
    },
    Step {
        name: "test",
        args: &["test", "--workspace"],
        env: &[],
    },
    Step {
        name: "doc",
        args: &["doc", "--workspace", "--no-deps"],
        env: &[("RUSTDOCFLAGS", "-D warnings")],
    },
];

/// What the command line asked for.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Request {
    /// Run these steps and summarise.
    Run(Vec<&'static Step>),
    /// Print the usage text.
    Help,
    /// The arguments were not understood.
    Unknown(String),
}

/// Works out what to do from the arguments after `cargo xtask`.
fn parse(args: &[String]) -> Request {
    match args {
        [] => Request::Help,
        [only] => match only.as_str() {
            "check" => Request::Run(STEPS.iter().collect()),
            "help" | "--help" | "-h" => Request::Help,
            name => match STEPS.iter().find(|step| step.name == name) {
                Some(step) => Request::Run(vec![step]),
                None => Request::Unknown(name.to_owned()),
            },
        },
        [_, extra, ..] => Request::Unknown(extra.clone()),
    }
}

/// The usage text.
fn usage() -> String {
    let mut text = String::from(
        "usage: cargo xtask <command>\n\ncommands:\n  check    run every step below, in order\n",
    );
    for step in STEPS {
        text.push_str(&format!("  {:<8} {}\n", step.name, command_line(step)));
    }
    text
}

/// The command a step runs, as a person would type it.
fn command_line(step: &Step) -> String {
    let mut line = String::new();
    for (key, value) in step.env {
        line.push_str(&format!("{key}=\"{value}\" "));
    }
    line.push_str("cargo ");
    line.push_str(&step.args.join(" "));
    line
}

/// The workspace root: the directory that holds this crate's directory.
fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from("."))
}

/// Runs one step and reports whether it passed and how long it took.
fn run(step: &Step, root: &Path) -> (bool, Duration) {
    println!("\nxtask: {} — {}", step.name, command_line(step));
    let cargo = env::var_os("CARGO").unwrap_or_else(|| OsString::from("cargo"));
    let started = Instant::now();
    let status = Command::new(cargo)
        .args(step.args)
        .envs(step.env.iter().copied())
        .current_dir(root)
        .status();
    let passed = match status {
        Ok(status) => status.success(),
        Err(error) => {
            eprintln!("xtask: could not run cargo: {error}");
            false
        }
    };
    (passed, started.elapsed())
}

fn main() -> ExitCode {
    let args: Vec<String> = env::args().skip(1).collect();
    let steps = match parse(&args) {
        Request::Run(steps) => steps,
        Request::Help => {
            print!("{}", usage());
            return ExitCode::SUCCESS;
        }
        Request::Unknown(word) => {
            eprintln!("xtask: unknown command `{word}`\n\n{}", usage());
            return ExitCode::from(2);
        }
    };

    let root = workspace_root();
    let results: Vec<(&Step, bool, Duration)> = steps
        .into_iter()
        .map(|step| {
            let (passed, took) = run(step, &root);
            (step, passed, took)
        })
        .collect();

    println!("\nxtask: summary");
    for (step, passed, took) in &results {
        let verdict = if *passed { "ok  " } else { "FAIL" };
        println!(
            "  {verdict}  {:<7} {:>7.1}s  {}",
            step.name,
            took.as_secs_f64(),
            command_line(step)
        );
    }
    let failed = results.iter().filter(|(_, passed, _)| !passed).count();
    if failed == 0 {
        println!("xtask: all {} steps passed", results.len());
        ExitCode::SUCCESS
    } else {
        println!("xtask: {failed} of {} steps failed", results.len());
        ExitCode::FAILURE
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(words: &[&str]) -> Vec<String> {
        words.iter().map(|word| (*word).to_owned()).collect()
    }

    #[test]
    fn check_runs_the_four_gates_in_order() {
        let Request::Run(steps) = parse(&args(&["check"])) else {
            panic!("`check` must run steps");
        };
        let names: Vec<&str> = steps.iter().map(|step| step.name).collect();
        assert_eq!(names, ["fmt", "clippy", "test", "doc"]);
    }

    #[test]
    fn a_step_name_runs_that_step_alone() {
        for step in STEPS {
            assert_eq!(parse(&args(&[step.name])), Request::Run(vec![step]));
        }
    }

    #[test]
    fn warnings_are_denied_where_the_gate_says_so() {
        let clippy = STEPS.iter().find(|step| step.name == "clippy").unwrap();
        assert!(clippy.args.ends_with(&["--", "-D", "warnings"]));
        assert!(clippy.args.contains(&"--all-targets"));
        let doc = STEPS.iter().find(|step| step.name == "doc").unwrap();
        assert_eq!(doc.env, [("RUSTDOCFLAGS", "-D warnings")]);
        assert!(doc.args.contains(&"--no-deps"));
    }

    #[test]
    fn help_and_mistakes_are_told_apart() {
        assert_eq!(parse(&args(&[])), Request::Help);
        assert_eq!(parse(&args(&["--help"])), Request::Help);
        assert_eq!(
            parse(&args(&["chekc"])),
            Request::Unknown("chekc".to_owned())
        );
        assert_eq!(
            parse(&args(&["check", "now"])),
            Request::Unknown("now".to_owned())
        );
    }

    #[test]
    fn the_usage_text_names_every_step() {
        let text = usage();
        assert!(text.contains("check"));
        for step in STEPS {
            assert!(text.contains(step.name));
        }
    }
}
