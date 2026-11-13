//! The modding guide, `docs/modding-guide.md`, run as a test.
//!
//! The guide is read in order: a fenced block whose info string names a
//! path after its language (```` ```json mods/hearth/data/kitchen.json ````)
//! is a file, written there; each line of a `bash` block is a command. A
//! `clash-tools` command is run as the tools run it, against a copy of the
//! bundled mods in a directory of its own, with `mods` in its arguments
//! meaning that directory; it must succeed. The game itself
//! (`clash-app`) and the world viewer, which wait for a person, are not
//! run: the test checks that what they would be given exists. Every link
//! of the guide into the documents must lead to a file, and to an anchor
//! in it where it names one.

use std::path::{Path, PathBuf};

use crate::data::EXIT_OK;

/// The root of the repository.
fn repo() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// The guide's text.
fn guide() -> String {
    std::fs::read_to_string(repo().join("docs/modding-guide.md")).expect("the modding guide")
}

/// What the guide asks to be done, in order.
#[derive(Debug)]
enum Step {
    /// A file to write: its path from the root of the repository, and
    /// its text.
    File { path: String, text: String },
    /// A line of a shell block.
    Command(String),
}

/// The steps of a guide: its files and its commands, in order.
fn steps(guide: &str) -> Vec<Step> {
    let mut steps = Vec::new();
    let mut lines = guide.lines();
    while let Some(line) = lines.next() {
        let Some(info) = line.trim_start().strip_prefix("```") else {
            continue;
        };
        let mut words = info.split_whitespace();
        let language = words.next().unwrap_or("");
        let path = words.next();
        let mut body = String::new();
        for line in lines.by_ref() {
            if line.trim_start().starts_with("```") {
                break;
            }
            body.push_str(line);
            body.push('\n');
        }
        match (language, path) {
            (_, Some(path)) => steps.push(Step::File {
                path: path.to_owned(),
                text: body,
            }),
            ("bash", None) => steps.extend(
                body.lines()
                    .map(str::trim)
                    .filter(|line| !line.is_empty() && !line.starts_with('#'))
                    .map(|line| Step::Command(line.to_owned())),
            ),
            _ => {}
        }
    }
    steps
}

/// A command line split into words as a shell would split it: on spaces,
/// with `'…'` and `"…"` keeping what they hold together.
fn words(line: &str) -> Vec<String> {
    let mut words = Vec::new();
    let mut word = String::new();
    let mut in_word = false;
    let mut quote = None;
    for c in line.chars() {
        match quote {
            Some(close) if c == close => quote = None,
            Some(_) => word.push(c),
            None if c == '\'' || c == '"' => {
                quote = Some(c);
                in_word = true;
            }
            None if c.is_whitespace() => {
                if in_word {
                    words.push(std::mem::take(&mut word));
                    in_word = false;
                }
            }
            None => {
                word.push(c);
                in_word = true;
            }
        }
    }
    if in_word {
        words.push(word);
    }
    words
}

/// How the guide calls the tools.
const TOOLS: &str = "cargo run -p clash-tools -- ";
/// How the guide starts the game.
const GAME: &str = "cargo run -p clash-app -- ";
/// The commands that load mods, and so are given the copy's directory.
const LOADING: &[&str] = &["lint", "show", "find", "refs", "schema", "scenario"];

/// Copies a directory and everything in it.
fn copy_dir(from: &Path, to: &Path) {
    std::fs::create_dir_all(to).expect("a directory");
    for entry in std::fs::read_dir(from).expect("a directory to copy") {
        let entry = entry.expect("an entry");
        let target = to.join(entry.file_name());
        if entry.file_type().expect("a file type").is_dir() {
            copy_dir(&entry.path(), &target);
        } else {
            std::fs::copy(entry.path(), &target).expect("a copy");
        }
    }
}

/// Runs a `clash-tools` command of the guide in `base`, which stands for
/// the root of the repository. Gives the command's name.
fn run_tools(base: &Path, line: &str, rest: &str) -> String {
    let mods = base.join("mods");
    let mut args: Vec<String> = words(rest)
        .into_iter()
        .map(|word| {
            if word == "mods" || word.starts_with("mods/") {
                base.join(&word).to_string_lossy().into_owned()
            } else {
                word
            }
        })
        .collect();
    assert!(!args.is_empty(), "`{line}` names no command");
    let name = args.remove(0);
    if name == "init-mod" && !args.iter().any(|arg| arg == "--dir") {
        args.extend(["--dir".to_owned(), mods.to_string_lossy().into_owned()]);
    }
    if LOADING.contains(&name.as_str()) && !args.iter().any(|arg| arg == "--mods") {
        args.extend(["--mods".to_owned(), mods.to_string_lossy().into_owned()]);
    }
    let output = crate::command(&name, &args)
        .unwrap_or_else(|| panic!("`{line}`: `{name}` is not a command of the tools"));
    assert_eq!(
        output.code, EXIT_OK,
        "the guide's `{line}` failed:\n{}{}",
        output.out, output.err
    );
    name
}

#[test]
fn the_modding_guide_runs() {
    let guide = guide();
    let base = std::env::temp_dir().join(format!("clash-tools-guide-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&base);
    // The bundled mods, as the repository has them; a mod the guide makes
    // is left out, should someone have followed it in the repository.
    let made: Vec<String> = steps(&guide)
        .iter()
        .filter_map(|step| match step {
            Step::Command(line) => {
                let args = words(line.strip_prefix(TOOLS)?);
                (args.first().map(String::as_str) == Some("init-mod"))
                    .then(|| args.get(1).cloned())
                    .flatten()
            }
            Step::File { .. } => None,
        })
        .collect();
    for entry in std::fs::read_dir(repo().join("mods")).expect("the bundled mods") {
        let entry = entry.expect("an entry");
        let name = entry.file_name().to_string_lossy().into_owned();
        if entry.path().join("mod.json").is_file() && !made.contains(&name) {
            copy_dir(&entry.path(), &base.join("mods").join(&name));
        }
    }

    let mut ran = Vec::new();
    for step in steps(&guide) {
        match step {
            Step::File { path, text } => {
                let target = base.join(&path);
                std::fs::create_dir_all(target.parent().expect("a parent")).expect("a directory");
                std::fs::write(&target, text).expect("a file of the guide");
            }
            Step::Command(line) => {
                if let Some(rest) = line.strip_prefix(TOOLS) {
                    ran.push(run_tools(&base, &line, rest));
                } else if let Some(rest) = line.strip_prefix(GAME) {
                    // The game waits for a person. What it is given must
                    // be there: a profile, by name, in a mod.
                    let args = words(rest);
                    let at = args.iter().position(|arg| arg == "--profile");
                    let profile = at.and_then(|at| args.get(at + 1)).expect("a profile");
                    let found = std::fs::read_dir(base.join("mods"))
                        .expect("the mods")
                        .filter_map(Result::ok)
                        .any(|entry| {
                            entry
                                .path()
                                .join("profiles")
                                .join(format!("{profile}.json"))
                                .is_file()
                        });
                    assert!(found, "`{line}`: no mod has a profile `{profile}`");
                } else if let Some((feed, viewer)) = line.split_once(" | ") {
                    // The world viewer reads its commands from a person
                    // or a pipe; the file piped to it must be there.
                    assert_eq!(viewer.trim(), format!("{TOOLS}worldview"), "`{line}`");
                    let file = words(feed).pop().expect("a file");
                    assert!(base.join(&file).is_file(), "`{line}`: {file} is not there");
                } else {
                    panic!("the guide's `{line}` is not a command this test knows how to run");
                }
            }
        }
    }
    for needed in [
        "init-mod", "lint", "new", "show", "find", "refs", "scenario",
    ] {
        assert!(
            ran.iter().any(|name| name == needed),
            "the guide no longer runs `{needed}`"
        );
    }
    let _ = std::fs::remove_dir_all(&base);
}

/// The anchors a Markdown file offers: `<a id="…">` and its headings, as
/// GitHub names them.
fn anchors(text: &str) -> Vec<String> {
    let mut anchors = Vec::new();
    for line in text.lines() {
        let mut rest = line;
        while let Some(at) = rest.find("<a id=\"") {
            rest = &rest[at + 7..];
            if let Some(end) = rest.find('"') {
                anchors.push(rest[..end].to_owned());
            }
        }
        if let Some(heading) = line.strip_prefix('#') {
            let heading = heading.trim_start_matches('#').trim();
            let slug: String = heading
                .to_lowercase()
                .chars()
                .filter_map(|c| match c {
                    ' ' => Some('-'),
                    c if c.is_alphanumeric() || c == '-' || c == '_' => Some(c),
                    _ => None,
                })
                .collect();
            anchors.push(slug);
        }
    }
    anchors
}

#[test]
fn the_links_of_the_modding_guide_lead_somewhere() {
    let guide = guide();
    let docs = repo().join("docs");
    let mut broken = Vec::new();
    let mut count = 0;
    let mut rest = guide.as_str();
    while let Some(at) = rest.find("](") {
        rest = &rest[at + 2..];
        let Some(end) = rest.find(')') else { break };
        let target = &rest[..end];
        if target.contains("://") {
            continue;
        }
        count += 1;
        let (file, anchor) = match target.split_once('#') {
            Some((file, anchor)) => (file, Some(anchor)),
            None => (target, None),
        };
        let path = docs.join(file);
        match std::fs::read_to_string(&path) {
            Err(_) => broken.push(format!("{target}: no file")),
            Ok(text) => {
                if let Some(anchor) = anchor
                    && !anchors(&text).iter().any(|offered| offered == anchor)
                {
                    broken.push(format!("{target}: no anchor `{anchor}`"));
                }
            }
        }
    }
    assert!(count > 20, "the guide links into the reference");
    assert!(
        broken.is_empty(),
        "broken links:\n  {}",
        broken.join("\n  ")
    );
}

#[test]
fn a_guide_is_read_as_files_and_commands() {
    let text = "Some words.\n\n```json mods/a/data/x.json\n[1]\n```\n\n```bash\n# a comment\n\
                cargo run -p clash-tools -- find item \"where=mass>1\"\n```\n\n```json\n{}\n```\n";
    let steps = steps(text);
    assert_eq!(steps.len(), 2, "{steps:?}");
    assert!(
        matches!(&steps[0], Step::File { path, text } if path == "mods/a/data/x.json" && text == "[1]\n")
    );
    assert!(matches!(&steps[1], Step::Command(line) if line.ends_with("\"where=mass>1\"")));
    assert_eq!(
        words("find item \"where=mass>1\" 'a b' c"),
        ["find", "item", "where=mass>1", "a b", "c"]
    );
}
