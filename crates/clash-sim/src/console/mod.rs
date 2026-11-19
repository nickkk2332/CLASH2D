//! The console and the test-script runner (`docs/14-tooling.md`
//! sections 3 and 7).
//!
//! One language serves the console in the game, the console in the
//! terminal viewer and test scripts in files (14 §2, principle 4). This
//! module is that language and everything about it that does not need a
//! simulation: it is written against a trait, [`ConsoleHost`], which
//! the simulation implements.
//!
//! | Part | Is |
//! |------|----|
//! | [`parse_line`], [`print_line`], [`Line`] | The language of 14 §3.2: a line into a syntax tree with byte spans, and back in canonical form |
//! | [`CommandSpec`], [`CommandTable`], [`BoundCommand`] | Registration of commands, one derived from each verb; binding a line to a command; generated help |
//! | [`ConsoleHost`] | What carries commands out: the seam to the simulation |
//! | [`Session`] | A console: lines in, the host driven, names and a transcript kept |
//! | [`parse_script`], [`Script`], [`Expectation`] | The test-script format of 14 §7 |
//! | [`Runner`], [`run_dir`], [`Report`] | Running scripts in the four modes of 14 §7.2, blessing, reporting |
//! | [`testing::FakeHost`] | A small host for tests |
//!
//! ```
//! use clash_sim::console::testing::FakeHost;
//! use clash_sim::console::{ConsoleHost, Runner, parse_script};
//!
//! let table = FakeHost::table();
//! let fresh = || -> Box<dyn ConsoleHost> { Box::new(FakeHost::new()) };
//! let runner = Runner::new(&table, &fresh);
//!
//! // A script with a hash still to be written in.
//! let text = "new apples value=3 as=a\n\
//!             timer @a every=1s\n\
//!             advance 2s\n\
//!             assert \"@a.value == 5\"\n\
//!             expect hash\n";
//! let blessed = runner.bless(text).unwrap();
//! assert_eq!(blessed.hashes_changed, 1);
//!
//! // Straight, twice, with a save and a load at each split point, and
//! // with different thread counts.
//! let result = runner.run(&parse_script(&blessed.text).unwrap());
//! assert!(result.passed());
//! ```
//!
//! # What waits for the simulation
//!
//! Everything a command does. The simulation implements [`ConsoleHost`]
//! and adds its verbs to a [`CommandTable`] with
//! [`CommandTable::add_verbs`]; the game overlay and the terminal viewer
//! each drive a [`Session`]. The `console_command` definition type
//! (14 §3.4) is a [`CommandSpec`] whose source is
//! [`CommandSource::Rule`], added by whoever loads definitions.
//! `--paranoid` waits for the invariant checkers of 14 §7.3.

mod command;
mod diag;
mod host;
mod runner;
mod script;
mod session;
mod syntax;
pub mod testing;

pub use command::{
    ArgValue, BoundCommand, CommandSource, CommandSpec, CommandTable, HostKind, Hosts, ParamSpec,
    ParamType, TableError,
};
pub use diag::{Code, Diagnostic, Span, codes};
pub use host::{
    ConsoleHost, ExprLocal, Expression, HostError, HostResult, Output, Record, RecordSource,
    Refusal,
};
pub use runner::{
    BlessError, Blessed, DEFAULT_THREAD_COUNTS, Failure, MAX_SPLIT_POINTS, Mode, ModeOutcome,
    Report, Runner, SCRIPT_EXTENSION, ScriptOutcome, ScriptReport, ScriptResult, read_scripts,
    run_dir,
};
pub use script::{Expectation, Script, ScriptError, Step, StepKind, parse_script};
pub use session::{ConsoleError, Mismatch, NO_SUCH_SAVE, Session, UNTIL_NOT_REACHED};
pub use syntax::{
    Arg, Command, Comment, Key, Line, MAX_LINE_LEN, Number, Numeral, RESERVED_NAMES, RefAt,
    Reference, Value, ValueKind, is_reference_name, is_word, parse_line, parse_number, print_line,
    print_value,
};
