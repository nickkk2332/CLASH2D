//! The binary `clash` as a person or a script starts it (package U11):
//! without a terminal it says so and leaves with success; a script on
//! standard input drives it headless and prints its frames; a mistake in
//! a script names its line; a profile with a broken mod shows the
//! located error and leaves with failure; a command line it cannot read
//! prints how it is used.

use std::io::Write;
use std::path::PathBuf;
use std::process::{Command, Output, Stdio};

fn manifest() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn data_dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("clash-bin-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    dir
}

/// Runs `clash` with arguments and standard input, in the repository,
/// with a data directory of the test's own.
fn clash(name: &str, args: &[&str], input: &str) -> Output {
    let mut child = Command::new(env!("CARGO_BIN_EXE_clash"))
        .args(args)
        .current_dir(manifest().join("../.."))
        .env("CLASH_DATA_DIR", data_dir(name))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("clash starts");
    // A process that leaves before reading its input closes the pipe;
    // that is not this test's concern.
    let _ = child
        .stdin
        .take()
        .expect("standard input is piped")
        .write_all(input.as_bytes());
    child.wait_with_output().expect("clash runs")
}

fn text(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).into_owned()
}

#[test]
fn without_a_terminal_it_says_so_and_succeeds() {
    let out = clash("no-terminal", &[], "");
    assert!(out.status.success(), "{}", text(&out.stderr));
    let said = text(&out.stdout);
    assert!(said.contains("not started"), "{said}");
    assert!(
        said.contains("ui:title"),
        "the boot screen is named: {said}"
    );
    assert!(said.contains("--headless"), "{said}");
}

#[test]
fn a_script_on_standard_input_drives_it() {
    let out = clash(
        "script",
        &["--headless", "-", "--seed", "7"],
        "# the title, then the new-game screen\nframe\nenter\nwait 2\nframe\n",
    );
    assert!(out.status.success(), "{}", text(&out.stderr));
    let printed = text(&out.stdout);
    let frames: Vec<&str> = printed.split("--- frame ---\n").skip(1).collect();
    assert_eq!(frames.len(), 2, "{printed}");
    assert!(frames[0].contains("C L A S H 2 D"));
    assert!(frames[1].contains("New game"));
    assert!(
        frames[1].contains("< 7 >"),
        "the seed given is offered: {}",
        frames[1]
    );
    assert_eq!(frames[1].lines().count(), 24);
}

#[test]
fn a_mistake_in_a_script_names_its_line() {
    let out = clash(
        "bad-script",
        &["--headless", "-"],
        "enter\njump over the moon\n",
    );
    assert!(!out.status.success());
    let said = text(&out.stderr);
    assert!(said.contains("line 2"), "{said}");
    assert!(said.contains("jump over the moon"), "{said}");
}

#[test]
fn a_broken_mod_is_a_located_error_and_a_failure() {
    let mods = manifest().join("tests/fixtures/broken/mods");
    let out = clash(
        "broken",
        &[
            "--mods",
            mods.to_str().unwrap(),
            "--profile",
            "broken",
            "--headless",
            "-",
        ],
        "frame\n",
    );
    assert_eq!(out.status.code(), Some(1));
    assert!(
        out.stdout.is_empty(),
        "nothing was drawn: {}",
        text(&out.stdout)
    );
    let said = text(&out.stderr);
    assert!(said.contains("unknown key \"colour_of_the_sky\""), "{said}");
    assert!(
        said.contains("screens.json:5:5"),
        "file, line and column: {said}"
    );
    assert!(said.contains("in: screen bad:front_door"), "{said}");
}

#[test]
fn a_command_line_it_cannot_read_prints_its_usage() {
    let out = clash("usage", &["--fast"], "");
    assert_eq!(out.status.code(), Some(2));
    let said = text(&out.stderr);
    assert!(
        said.contains("`--fast` is not an option of clash"),
        "{said}"
    );
    assert!(said.contains("usage: clash"), "{said}");

    let out = clash("help", &["--help"], "");
    assert!(out.status.success());
    assert!(text(&out.stdout).starts_with("usage: clash"));
}
