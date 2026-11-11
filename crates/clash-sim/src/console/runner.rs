//! Running test scripts (14 §7.2): each script in four ways, with the
//! same result required from all.
//!
//! | Mode | How | Checks |
//! |------|-----|--------|
//! | Straight | Once, on a fresh host | The expectations |
//! | Twice | Twice more in the same process, each on a fresh host | No hidden state: nothing outlives a host |
//! | Save and load | Once for each split point, saving after that command, loading into a fresh host and going on | Rule R7 |
//! | Threads | Once for each thread count | Rule R1 |
//!
//! "The same result" is exact: the straight run records the state hash
//! after every command, and every other run must pass every expectation
//! and give the same hash after every command. A failure names the
//! line, what was expected and what was found, and for the last three
//! modes which run it was: the split point or the thread count.
//!
//! A **split point** is the end of a command. A script of at most
//! [`MAX_SPLIT_POINTS`] commands is split after each of them; a longer
//! one at that many points spread evenly from its first command to its
//! last. Each split point is a run of its own, so that a failure can
//! name it.
//!
//! A command that is refused fails the script unless the next line is
//! `expect reason`.
//!
//! Nothing here reads a clock, and nothing but [`read_scripts`] and
//! [`run_dir`] reads a file: the rest takes file contents as arguments.

use std::fmt::Write as _;
use std::path::Path;

use clash_core::Hash128;

use super::command::{CommandTable, HostKind};
use super::host::ConsoleHost;
use super::script::{Expectation, Script, ScriptError, Step, StepKind, parse_script};
use super::session::{ConsoleError, Session};

/// The most split points the save-and-load mode uses for one script.
pub const MAX_SPLIT_POINTS: usize = 8;

/// The thread counts the threads mode runs with, unless told otherwise.
pub const DEFAULT_THREAD_COUNTS: [u32; 2] = [1, 4];

/// The extension of a script file.
pub const SCRIPT_EXTENSION: &str = "scn";

/// The four ways a script is run.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub enum Mode {
    /// Once.
    Straight,
    /// Twice more in the same process.
    Twice,
    /// With a save and a load at each split point.
    SaveLoad,
    /// With each thread count.
    Threads,
}

impl Mode {
    /// The four modes, in the order they run.
    pub const ALL: [Mode; 4] = [Mode::Straight, Mode::Twice, Mode::SaveLoad, Mode::Threads];

    /// The mode's name in a report.
    pub const fn name(self) -> &'static str {
        match self {
            Mode::Straight => "straight",
            Mode::Twice => "twice",
            Mode::SaveLoad => "save-and-load",
            Mode::Threads => "threads",
        }
    }
}

/// Why a run failed.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Failure {
    /// The line of the script, counting from one.
    pub line: usize,
    /// What was expected.
    pub expected: String,
    /// What was found.
    pub found: String,
    /// Which run of the mode it was, where the mode has several: "split
    /// after line 5", "4 threads", "second run".
    pub run: Option<String>,
}

/// How one mode went for one script.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum ModeOutcome {
    /// Every run of the mode passed.
    Passed,
    /// A run failed.
    Failed(Failure),
    /// The mode was not run, because the straight run failed and the
    /// others have nothing to compare with.
    Skipped,
}

/// How a script went: one outcome for each mode.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct ScriptResult {
    /// The outcomes, in the order of [`Mode::ALL`].
    pub modes: Vec<(Mode, ModeOutcome)>,
}

impl ScriptResult {
    /// Whether every mode passed.
    pub fn passed(&self) -> bool {
        self.modes
            .iter()
            .all(|(_, outcome)| *outcome == ModeOutcome::Passed)
    }

    /// The outcome of one mode.
    pub fn outcome(&self, mode: Mode) -> &ModeOutcome {
        self.modes
            .iter()
            .find(|(found, _)| *found == mode)
            .map_or(&ModeOutcome::Skipped, |(_, outcome)| outcome)
    }
}

/// What the runner needs besides the script.
pub struct Runner<'a> {
    /// The commands the host has.
    pub table: &'a CommandTable,
    /// Makes a fresh host. Every run starts from one, and the
    /// save-and-load mode loads into one.
    pub factory: &'a dyn Fn() -> Box<dyn ConsoleHost>,
    /// Reads a file named relative to the script, for `expect frame`.
    pub files: &'a dyn Fn(&str) -> Option<String>,
    /// The thread counts of the threads mode.
    pub thread_counts: Vec<u32>,
}

impl<'a> Runner<'a> {
    /// A runner with the default thread counts and no files to read.
    pub fn new(
        table: &'a CommandTable,
        factory: &'a dyn Fn() -> Box<dyn ConsoleHost>,
    ) -> Runner<'a> {
        Runner {
            table,
            factory,
            files: &no_files,
            thread_counts: DEFAULT_THREAD_COUNTS.to_vec(),
        }
    }

    /// The same runner, reading files with this.
    #[must_use]
    pub fn files(mut self, files: &'a dyn Fn(&str) -> Option<String>) -> Runner<'a> {
        self.files = files;
        self
    }

    /// Runs a script in the four modes.
    pub fn run(&self, script: &Script) -> ScriptResult {
        let straight = self.once(script, &Plan::default());
        let Trail::Complete(hashes) = &straight.trail else {
            let failure = straight.failure.unwrap_or_else(|| Failure {
                line: 0,
                expected: "the script to run".to_owned(),
                found: "nothing".to_owned(),
                run: None,
            });
            return ScriptResult {
                modes: vec![
                    (Mode::Straight, ModeOutcome::Failed(failure)),
                    (Mode::Twice, ModeOutcome::Skipped),
                    (Mode::SaveLoad, ModeOutcome::Skipped),
                    (Mode::Threads, ModeOutcome::Skipped),
                ],
            };
        };
        let compare = |plan: Plan, run: String| -> Option<Failure> {
            let other = self.once(script, &plan);
            // Where the state first differs is more use than the
            // expectation that later fails because of it.
            let failure = match (first_difference(hashes, &other.trail), other.failure) {
                (Some(differs), Some(failed)) if differs.line <= failed.line => Some(differs),
                (differs, failed) => failed.or(differs),
            };
            failure.map(|failure| Failure {
                run: Some(run),
                ..failure
            })
        };
        let outcome = |failure: Option<Failure>| match failure {
            Some(failure) => ModeOutcome::Failed(failure),
            None => ModeOutcome::Passed,
        };
        let twice = compare(Plan::default(), "first repeat".to_owned())
            .or_else(|| compare(Plan::default(), "second repeat".to_owned()));
        let save_load = split_points(script).into_iter().find_map(|step| {
            compare(
                Plan {
                    split_after: Some(step),
                    ..Plan::default()
                },
                format!("split after line {}", script.steps[step].line),
            )
        });
        let threads = self.thread_counts.iter().find_map(|&count| {
            compare(
                Plan {
                    threads: Some(count),
                    ..Plan::default()
                },
                match count {
                    1 => "1 thread".to_owned(),
                    count => format!("{count} threads"),
                },
            )
        });
        ScriptResult {
            modes: vec![
                (Mode::Straight, ModeOutcome::Passed),
                (Mode::Twice, outcome(twice)),
                (Mode::SaveLoad, outcome(save_load)),
                (Mode::Threads, outcome(threads)),
            ],
        }
    }

    /// One run of a script.
    fn once(&self, script: &Script, plan: &Plan) -> Run {
        let mut run = Run::default();
        let mut hashes = Vec::new();
        let fail = |line: usize, expected: String, found: String| Failure {
            line,
            expected,
            found,
            run: None,
        };
        let fresh = |line: usize| -> Result<Box<dyn ConsoleHost>, Failure> {
            let mut host = (self.factory)();
            if let Some(threads) = plan.threads {
                host.set_threads(threads).map_err(|error| {
                    fail(
                        line,
                        format!("the host to run with {threads} threads"),
                        error.to_string(),
                    )
                })?;
            }
            Ok(host)
        };
        let first_line = script.steps.first().map_or(1, |step| step.line);
        let mut host = match fresh(first_line) {
            Ok(host) => host,
            Err(failure) => {
                run.failure = Some(failure);
                return run;
            }
        };
        let mut session = Session::new(HostKind::File);
        for (index, step) in script.steps.iter().enumerate() {
            let result = match &step.kind {
                StepKind::Command(line) => {
                    let refusal_expected = matches!(
                        script.steps.get(index + 1),
                        Some(Step {
                            kind: StepKind::Expect(Expectation::Reason { .. }),
                            ..
                        })
                    );
                    let carried = match session.run(self.table, host.as_mut(), line) {
                        Ok(_) => Ok(()),
                        Err(ConsoleError::Refused(_)) if refusal_expected => Ok(()),
                        Err(ConsoleError::Refused(refusal)) => Err(fail(
                            step.line,
                            "the command to be carried out".to_owned(),
                            format!("it was refused for `{refusal}`"),
                        )),
                        Err(ConsoleError::Syntax(errors)) => Err(fail(
                            step.line,
                            "a command the console knows, with arguments that fit".to_owned(),
                            errors
                                .iter()
                                .map(|error| match &error.suggestion {
                                    Some(suggestion) => format!("{error} ({suggestion})"),
                                    None => error.to_string(),
                                })
                                .collect::<Vec<_>>()
                                .join("; "),
                        )),
                        Err(ConsoleError::Failed(message)) => Err(fail(
                            step.line,
                            "the command to be carried out".to_owned(),
                            message,
                        )),
                    };
                    carried.and_then(|()| {
                        let hash = host.state_hash();
                        hashes.push((step.line, hash));
                        if plan.split_after != Some(index) {
                            return Ok(());
                        }
                        // Save, throw the host away, and go on in a
                        // fresh one from what was saved.
                        let saved = host.save().map_err(|error| {
                            fail(
                                step.line,
                                "the state to be saved".to_owned(),
                                error.to_string(),
                            )
                        })?;
                        host = fresh(step.line)?;
                        host.load(&saved).map_err(|error| {
                            fail(
                                step.line,
                                "the saved state to load into a fresh host".to_owned(),
                                error.to_string(),
                            )
                        })?;
                        let loaded = host.state_hash();
                        if loaded == hash {
                            Ok(())
                        } else {
                            Err(fail(
                                step.line,
                                format!("state hash {hash} after saving and loading, as before"),
                                loaded.to_string(),
                            ))
                        }
                    })
                }
                StepKind::Expect(Expectation::Hash { at, .. }) if plan.bless => {
                    run.blessed_hashes.push((*at, host.state_hash()));
                    Ok(())
                }
                StepKind::Expect(Expectation::Frame { file }) if plan.bless => host
                    .frame()
                    .map(|frame| run.blessed_frames.push((file.clone(), frame)))
                    .map_err(|error| {
                        fail(step.line, "a frame to store".to_owned(), error.to_string())
                    }),
                StepKind::Expect(expectation) => session
                    .check(host.as_mut(), expectation, self.files)
                    .map_err(|mismatch| fail(step.line, mismatch.expected, mismatch.found)),
            };
            if let Err(failure) = result {
                run.failure = Some(failure);
                run.trail = Trail::Partial(hashes);
                return run;
            }
        }
        run.trail = Trail::Complete(hashes);
        run
    }

    /// Runs a script once and rewrites the hashes it expects to the
    /// ones found, leaving every other byte of `text` alone; an `expect
    /// hash` with no value gets one. The frames it expects are returned
    /// to be written to their files. Other expectations are checked as
    /// usual, and a failure of one of them, or a script that does not
    /// parse, blesses nothing.
    pub fn bless(&self, text: &str) -> Result<Blessed, BlessError> {
        let script = parse_script(text).map_err(BlessError::Syntax)?;
        let run = self.once(
            &script,
            &Plan {
                bless: true,
                ..Plan::default()
            },
        );
        if let Some(failure) = run.failure {
            return Err(BlessError::Failed(failure));
        }
        let mut blessed = text.to_owned();
        let mut hashes = run.blessed_hashes;
        // From the end of the file back, so that earlier spans stay
        // where they are.
        hashes.sort_by_key(|(at, _)| core::cmp::Reverse(at.start));
        let mut changed = 0;
        for (at, hash) in hashes {
            let written = hash.to_string();
            let old = at.slice(text);
            if old == written {
                continue;
            }
            changed += 1;
            let new = if at.is_empty() {
                format!(" {written}")
            } else {
                written
            };
            blessed.replace_range(at.range(), &new);
        }
        let mut frames: Vec<(String, String)> = Vec::new();
        for (file, frame) in run.blessed_frames {
            // The last frame stored under a name wins.
            frames.retain(|(earlier, _)| *earlier != file);
            frames.push((file, frame));
        }
        frames.retain(|(file, frame)| (self.files)(file).as_ref() != Some(frame));
        Ok(Blessed {
            text: blessed,
            hashes_changed: changed,
            frames,
        })
    }

    /// Runs scripts given as their paths and contents, in the order
    /// given. `files` here reads a file named relative to a script,
    /// given the script's path first.
    pub fn run_all<'s>(
        table: &CommandTable,
        factory: &dyn Fn() -> Box<dyn ConsoleHost>,
        files: &dyn Fn(&str, &str) -> Option<String>,
        scripts: impl IntoIterator<Item = (&'s str, &'s str)>,
    ) -> Report {
        let mut report = Report::default();
        for (path, text) in scripts {
            let outcome = match parse_script(text) {
                Err(errors) => ScriptOutcome::Syntax(errors),
                Ok(script) => {
                    let read = |file: &str| files(path, file);
                    ScriptOutcome::Ran(Runner::new(table, factory).files(&read).run(&script))
                }
            };
            report.scripts.push(ScriptReport {
                path: path.to_owned(),
                outcome,
            });
        }
        report
    }
}

/// The files a runner reads when it was given none.
fn no_files(_: &str) -> Option<String> {
    None
}

/// How one run is to be made.
#[derive(Clone, Copy, Default)]
struct Plan {
    /// The thread count to set, if any.
    threads: Option<u32>,
    /// The index of the step to save and load after, if any.
    split_after: Option<usize>,
    /// Whether hashes and frames are collected instead of checked.
    bless: bool,
}

/// The state hash after each command of a run, with the command's line.
#[derive(Clone, PartialEq, Eq, Debug)]
enum Trail {
    /// The run reached the end of the script.
    Complete(Vec<(usize, Hash128)>),
    /// The run stopped early.
    Partial(Vec<(usize, Hash128)>),
}

impl Default for Trail {
    fn default() -> Trail {
        Trail::Partial(Vec::new())
    }
}

/// What one run gave.
#[derive(Default)]
struct Run {
    trail: Trail,
    failure: Option<Failure>,
    blessed_hashes: Vec<(super::diag::Span, Hash128)>,
    blessed_frames: Vec<(String, String)>,
}

/// The first command after which a run's hash is not the straight
/// run's.
fn first_difference(straight: &[(usize, Hash128)], other: &Trail) -> Option<Failure> {
    let (Trail::Complete(other) | Trail::Partial(other)) = other;
    straight
        .iter()
        .zip(other)
        .find(|((_, expected), (_, found))| expected != found)
        .map(|((line, expected), (_, found))| Failure {
            line: *line,
            expected: format!("state hash {expected}, as in the straight run"),
            found: found.to_string(),
            run: None,
        })
}

/// The steps the save-and-load mode saves after: every command of a
/// short script, and [`MAX_SPLIT_POINTS`] spread evenly from the first
/// to the last of a long one.
fn split_points(script: &Script) -> Vec<usize> {
    let commands: Vec<usize> = script
        .steps
        .iter()
        .enumerate()
        .filter(|(_, step)| matches!(step.kind, StepKind::Command(_)))
        .map(|(index, _)| index)
        .collect();
    if commands.len() <= MAX_SPLIT_POINTS {
        return commands;
    }
    let last = commands.len() - 1;
    (0..MAX_SPLIT_POINTS)
        .map(|point| commands[point * last / (MAX_SPLIT_POINTS - 1)])
        .collect()
}

/// A script with its expected hashes rewritten.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Blessed {
    /// The script's text, changed only where a hash is written.
    pub text: String,
    /// How many hashes changed or were added.
    pub hashes_changed: usize,
    /// The frames that differ from what is stored: each file, named as
    /// the script names it, and what it should now hold.
    pub frames: Vec<(String, String)>,
}

/// Why a script could not be blessed.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum BlessError {
    /// The script does not parse.
    Syntax(Vec<ScriptError>),
    /// Something other than a hash or a frame failed.
    Failed(Failure),
}

/// How one script file went.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum ScriptOutcome {
    /// The file does not parse.
    Syntax(Vec<ScriptError>),
    /// The file was run.
    Ran(ScriptResult),
}

/// One script file in a report.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct ScriptReport {
    /// The file's path, as it was given.
    pub path: String,
    /// How it went.
    pub outcome: ScriptOutcome,
}

impl ScriptReport {
    /// Whether the script parsed and passed in every mode.
    pub fn passed(&self) -> bool {
        matches!(&self.outcome, ScriptOutcome::Ran(result) if result.passed())
    }
}

/// How a set of scripts went.
#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct Report {
    /// The scripts, in the order they were run.
    pub scripts: Vec<ScriptReport>,
}

impl Report {
    /// Whether every script passed.
    pub fn passed(&self) -> bool {
        self.scripts.iter().all(ScriptReport::passed)
    }

    /// How many scripts did not pass.
    pub fn failed(&self) -> usize {
        self.scripts
            .iter()
            .filter(|script| !script.passed())
            .count()
    }

    /// The report as plain text: each script with its four modes, each
    /// failure with its line, what was expected and what was found, and
    /// a last line of totals.
    pub fn render(&self) -> String {
        let mut out = String::new();
        for script in &self.scripts {
            let _ = writeln!(out, "{}", script.path);
            match &script.outcome {
                ScriptOutcome::Syntax(errors) => {
                    for error in errors {
                        for line in error.render(&script.path).lines() {
                            let _ = writeln!(out, "  {line}");
                        }
                    }
                }
                ScriptOutcome::Ran(result) => {
                    for (mode, outcome) in &result.modes {
                        let name = mode.name();
                        match outcome {
                            ModeOutcome::Passed => {
                                let _ = writeln!(out, "  {name:<13} passed");
                            }
                            ModeOutcome::Skipped => {
                                let _ = writeln!(out, "  {name:<13} not run");
                            }
                            ModeOutcome::Failed(failure) => {
                                let _ = write!(out, "  {name:<13} FAILED at line {}", failure.line);
                                if let Some(run) = &failure.run {
                                    let _ = write!(out, " ({run})");
                                }
                                let _ = writeln!(out);
                                let _ =
                                    writeln!(out, "    expected: {}", indent(&failure.expected));
                                let _ = writeln!(out, "    found:    {}", indent(&failure.found));
                            }
                        }
                    }
                }
            }
        }
        let total = self.scripts.len();
        let failed = self.failed();
        let _ = writeln!(
            out,
            "{total} script{}, {} passed, {failed} failed",
            if total == 1 { "" } else { "s" },
            total - failed
        );
        out
    }
}

/// A text of several lines with the later ones indented under the
/// first.
fn indent(text: &str) -> String {
    text.trim_end().replace('\n', "\n              ")
}

/// Every script under a directory, as its path and its contents, in
/// order of path. This is the runner's one walk of the file system.
pub fn read_scripts(dir: &Path) -> std::io::Result<Vec<(String, String)>> {
    let mut found = Vec::new();
    let mut pending = vec![dir.to_path_buf()];
    while let Some(next) = pending.pop() {
        for entry in std::fs::read_dir(&next)? {
            let path = entry?.path();
            if path.is_dir() {
                pending.push(path);
            } else if path
                .extension()
                .is_some_and(|extension| extension == SCRIPT_EXTENSION)
            {
                let text = std::fs::read_to_string(&path)?;
                found.push((path.to_string_lossy().into_owned(), text));
            }
        }
    }
    found.sort();
    Ok(found)
}

/// Finds every script under a directory and runs each in the four
/// modes, in order of path. A frame is read from beside its script.
pub fn run_dir(
    dir: &Path,
    table: &CommandTable,
    factory: &dyn Fn() -> Box<dyn ConsoleHost>,
) -> std::io::Result<Report> {
    let scripts = read_scripts(dir)?;
    let beside = |script: &str, file: &str| {
        let path = Path::new(script)
            .parent()
            .unwrap_or(Path::new(""))
            .join(file);
        std::fs::read_to_string(path).ok()
    };
    Ok(Runner::run_all(
        table,
        factory,
        &beside,
        scripts
            .iter()
            .map(|(path, text)| (path.as_str(), text.as_str())),
    ))
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;
    use std::rc::Rc;

    use super::*;
    use crate::console::testing::{FakeHost, Flaws};

    const SCRIPT: &str = "\
# A counter that ticks.
use fake seed=7
new a value=1 as=a
new b as=b
timer @a every=250ms times=8
advance 1s
assert \"@a.value == 5\"
lock @b
add @b
expect reason cell_locked
until \"@a.value == 9\" max=10s
expect event timer_fired cell=@a value=9 within=1s
add @a 100
expect hash
";

    fn sound() -> Box<dyn ConsoleHost> {
        Box::new(FakeHost::new())
    }

    fn run_with(text: &str, factory: &dyn Fn() -> Box<dyn ConsoleHost>) -> ScriptResult {
        let table = FakeHost::table();
        let script = parse_script(text).unwrap_or_else(|errors| panic!("{errors:?}"));
        Runner::new(&table, factory).run(&script)
    }

    fn blessed(text: &str) -> String {
        let table = FakeHost::table();
        Runner::new(&table, &sound).bless(text).unwrap().text
    }

    #[test]
    fn a_sound_host_passes_all_four_modes() {
        let result = run_with(&blessed(SCRIPT), &sound);
        assert!(result.passed(), "{result:?}");
        assert_eq!(result.modes.len(), 4);
        for mode in Mode::ALL {
            assert_eq!(result.outcome(mode), &ModeOutcome::Passed);
        }
    }

    #[test]
    fn a_failed_expectation_names_its_line_and_skips_the_other_modes() {
        let result = run_with(SCRIPT, &sound);
        let ModeOutcome::Failed(failure) = result.outcome(Mode::Straight) else {
            panic!("{result:?}")
        };
        assert_eq!(failure.line, 14);
        assert_eq!(failure.expected, "a state hash written in the script");
        assert_eq!(result.outcome(Mode::Twice), &ModeOutcome::Skipped);
        assert_eq!(result.outcome(Mode::Threads), &ModeOutcome::Skipped);

        let text = SCRIPT.replace("@a.value == 5", "@a.value == 6");
        let ModeOutcome::Failed(failure) = run_with(&text, &sound).outcome(Mode::Straight).clone()
        else {
            panic!()
        };
        assert_eq!(
            (
                failure.line,
                failure.expected.as_str(),
                failure.found.as_str()
            ),
            (7, "`@a.value == 6` to be true", "false")
        );
    }

    #[test]
    fn a_refusal_nobody_expected_fails_the_script() {
        let text = SCRIPT.replace("expect reason cell_locked\n", "");
        let ModeOutcome::Failed(failure) = run_with(&text, &sound).outcome(Mode::Straight).clone()
        else {
            panic!()
        };
        assert_eq!(failure.line, 9);
        assert!(failure.found.contains("cell_locked"), "{}", failure.found);
        // So does a command the console does not know, with the reason.
        let ModeOutcome::Failed(failure) =
            run_with("ad @a\n", &sound).outcome(Mode::Straight).clone()
        else {
            panic!()
        };
        assert!(failure.found.contains("E1421") && failure.found.contains("did you mean `add`?"));
    }

    #[test]
    fn a_save_that_loses_state_fails_under_save_and_load_naming_the_split() {
        // TL6: "A script that passes straight and fails under
        // save-and-load is reported as such, naming the split point."
        let flawed = || -> Box<dyn ConsoleHost> {
            Box::new(FakeHost::with_flaws(Flaws {
                save_forgets_timers: true,
                ..Flaws::default()
            }))
        };
        let result = run_with(&blessed(SCRIPT), &flawed);
        assert_eq!(result.outcome(Mode::Straight), &ModeOutcome::Passed);
        assert_eq!(result.outcome(Mode::Twice), &ModeOutcome::Passed);
        assert_eq!(result.outcome(Mode::Threads), &ModeOutcome::Passed);
        let ModeOutcome::Failed(failure) = result.outcome(Mode::SaveLoad) else {
            panic!("{result:?}")
        };
        // Nothing is lost until there is a timer to lose: the first
        // split that fails is the one after the `timer` line.
        assert_eq!(failure.run.as_deref(), Some("split after line 5"));
        assert_eq!(failure.line, 5);
        assert!(
            failure.expected.contains("after saving and loading"),
            "{}",
            failure.expected
        );

        let report = Report {
            scripts: vec![ScriptReport {
                path: "a.scn".to_owned(),
                outcome: ScriptOutcome::Ran(result),
            }],
        };
        let text = report.render();
        assert!(text.contains("  straight      passed\n"), "{text}");
        assert!(
            text.contains("  save-and-load FAILED at line 5 (split after line 5)\n"),
            "{text}"
        );
        assert!(text.ends_with("1 script, 0 passed, 1 failed\n"), "{text}");
        assert!(!report.passed());
    }

    #[test]
    fn state_that_outlives_a_host_fails_the_twice_mode() {
        // Each host made by this factory shares one counter, as a
        // static in the simulation would be shared.
        let leak = Rc::new(Cell::new(0));
        let leaky = || -> Box<dyn ConsoleHost> {
            Box::new(FakeHost::with_flaws(Flaws {
                leak: Some(Rc::clone(&leak)),
                ..Flaws::default()
            }))
        };
        let table = FakeHost::table();
        let text = "new a\nnew b\nexpect hash\n";
        let blessed = Runner::new(&table, &leaky).bless(text).unwrap().text;
        leak.set(0);
        let result = run_with(&blessed, &leaky);
        assert_eq!(result.outcome(Mode::Straight), &ModeOutcome::Passed);
        let ModeOutcome::Failed(failure) = result.outcome(Mode::Twice) else {
            panic!("{result:?}")
        };
        assert_eq!(failure.run.as_deref(), Some("first repeat"));
        assert_eq!(failure.line, 1);
        assert!(failure.expected.contains("as in the straight run"));
    }

    #[test]
    fn a_result_that_depends_on_the_thread_count_fails_the_threads_mode() {
        let racy = || -> Box<dyn ConsoleHost> {
            Box::new(FakeHost::with_flaws(Flaws {
                threads_matter: true,
                ..Flaws::default()
            }))
        };
        let result = run_with(&blessed(SCRIPT), &racy);
        assert_eq!(result.outcome(Mode::Straight), &ModeOutcome::Passed);
        assert_eq!(result.outcome(Mode::SaveLoad), &ModeOutcome::Passed);
        let ModeOutcome::Failed(failure) = result.outcome(Mode::Threads) else {
            panic!("{result:?}")
        };
        assert_eq!(failure.run.as_deref(), Some("4 threads"));
        assert_eq!(failure.line, 6);
    }

    #[test]
    fn bless_changes_hashes_and_nothing_else() {
        let table = FakeHost::table();
        let runner = Runner::new(&table, &sound);
        let stale = "0".repeat(32);
        let text = format!(
            "  new a   value=1 as=a  # odd  spacing\r\n\
             expect hash   {stale}  # was this\r\n\
             add @a\n\
             expect hash # none yet\n\
             \t add @a 2\n\
             expect hash"
        );
        let first = runner.bless(&text).unwrap();
        assert_eq!(first.hashes_changed, 3);
        // Everything but the hashes is byte for byte what it was.
        let hashes: Vec<&str> = first
            .text
            .split(|c: char| !c.is_ascii_hexdigit())
            .filter(|word| word.len() == 32)
            .collect();
        assert_eq!(hashes.len(), 3);
        let expected = format!(
            "  new a   value=1 as=a  # odd  spacing\r\n\
             expect hash   {}  # was this\r\n\
             add @a\n\
             expect hash {} # none yet\n\
             \t add @a 2\n\
             expect hash {}",
            hashes[0], hashes[1], hashes[2]
        );
        assert_eq!(first.text, expected);
        assert!(hashes[0] != stale && hashes[0] != hashes[1] && hashes[1] != hashes[2]);
        // Blessing again changes nothing, and the result passes.
        let second = runner.bless(&first.text).unwrap();
        assert_eq!(
            (second.text.as_str(), second.hashes_changed),
            (first.text.as_str(), 0)
        );
        assert!(runner.run(&parse_script(&first.text).unwrap()).passed());
    }

    #[test]
    fn bless_refuses_a_script_that_fails_otherwise() {
        let table = FakeHost::table();
        let runner = Runner::new(&table, &sound);
        let BlessError::Failed(failure) = runner
            .bless("new a as=a\nassert @a.value == 1\nexpect hash\n")
            .unwrap_err()
        else {
            panic!()
        };
        assert_eq!(failure.line, 2);
        assert!(
            matches!(runner.bless("new \"a\n"), Err(BlessError::Syntax(errors)) if errors[0].line == 1)
        );
    }

    #[test]
    fn frames_are_compared_and_blessed() {
        let table = FakeHost::table();
        let text = "use fake seed=7\nnew a value=1\nexpect frame frames/a.txt\n";
        let stored = "fake host  profile=fake seed=7  t=0s\na = 1\n";
        let good = |file: &str| (file == "frames/a.txt").then(|| stored.to_owned());
        let stale = |file: &str| (file == "frames/a.txt").then(|| "something else\n".to_owned());
        let script = parse_script(text).unwrap();
        assert!(
            Runner::new(&table, &sound)
                .files(&good)
                .run(&script)
                .passed()
        );
        let result = Runner::new(&table, &sound).files(&stale).run(&script);
        let ModeOutcome::Failed(failure) = result.outcome(Mode::Straight) else {
            panic!()
        };
        assert_eq!(failure.line, 3);
        assert!(failure.found.contains("a = 1"));
        assert!(!Runner::new(&table, &sound).run(&script).passed());

        let blessed = Runner::new(&table, &sound)
            .files(&stale)
            .bless(text)
            .unwrap();
        assert_eq!(blessed.text, text);
        assert_eq!(
            blessed.frames,
            [("frames/a.txt".to_owned(), stored.to_owned())]
        );
        assert!(
            Runner::new(&table, &sound)
                .files(&good)
                .bless(text)
                .unwrap()
                .frames
                .is_empty()
        );
    }

    #[test]
    fn split_points_cover_short_scripts_and_spread_over_long_ones() {
        let short = parse_script("check\ncheck\nexpect hash\ncheck\n").unwrap();
        assert_eq!(split_points(&short), [0, 1, 3]);
        let long = parse_script(&"check\n".repeat(30)).unwrap();
        let points = split_points(&long);
        assert_eq!(points.len(), MAX_SPLIT_POINTS);
        assert_eq!((points[0], points[MAX_SPLIT_POINTS - 1]), (0, 29));
        assert!(points.windows(2).all(|pair| pair[0] < pair[1]));
        assert!(split_points(&Script::default()).is_empty());
    }

    #[test]
    fn many_scripts_make_a_report() {
        let table = FakeHost::table();
        let good = blessed("new a\nexpect hash\n");
        let scripts = [
            ("a/good.scn", good.as_str()),
            ("b/bad.scn", "new a as=a\nassert @a.value == 3\n"),
            ("c/broken.scn", "new \"a\n"),
        ];
        let report = Runner::run_all(&table, &sound, &|_, _| None, scripts);
        assert_eq!(report.failed(), 2);
        assert!(report.scripts[0].passed());
        let text = report.render();
        let expected = "\
a/good.scn
  straight      passed
  twice         passed
  save-and-load passed
  threads       passed
b/bad.scn
  straight      FAILED at line 2
    expected: `@a.value == 3` to be true
    found:    false
  twice         not run
  save-and-load not run
  threads       not run
c/broken.scn
  c/broken.scn:1:5: error[E1402]: the quote opened here is never closed
    new \"a
        ^^
    help: close it with `\"`; a quoted value stays on one line
3 scripts, 1 passed, 2 failed
";
        assert_eq!(text, expected);
    }
}
