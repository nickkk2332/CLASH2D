//! A console session: lines in, a host driven, output back.
//!
//! The [`Session`] is the part of the console that is the same in the
//! game overlay, the terminal viewer and a file. It binds each line to
//! its command, resolves the references in it, carries out the commands
//! that are the console's own through the operations of
//! [`ConsoleHost`], and hands every other command to the host. It keeps
//! what belongs to the session and not to the simulation: the names
//! given with `as=`, the states kept with `save`, the events and trace
//! records seen so far, and a transcript that can be saved as a script.
//!
//! | Command | Carried out by |
//! |---------|----------------|
//! | `help` | The command table |
//! | `use`, `gen` | [`ConsoleHost::start`], [`ConsoleHost::generate`] |
//! | `eval`, `assert` | [`ConsoleHost::eval`] |
//! | `advance`, `until` | [`ConsoleHost::advance`], [`ConsoleHost::advance_until`] |
//! | `hash` with no subsystem | [`ConsoleHost::state_hash`] |
//! | `save`, `load` | [`ConsoleHost::save`], [`ConsoleHost::load`], with the bytes kept in the session under the name |
//! | `expect` | Nobody: it is an expectation, which [`Session::check`] checks for the script runner |
//! | Every other command | [`ConsoleHost::execute`] |

use std::collections::BTreeMap;

use clash_core::{Dur, Real};
use clash_expr::Value;

use super::command::{ArgValue, BoundCommand, CommandSource, CommandTable, HostKind};
use super::diag::Diagnostic;
use super::host::{ConsoleHost, Expression, HostError, Output, Record, RecordSource, Refusal};
use super::script::Expectation;
use super::syntax::{Line, Reference, parse_line, print_line};

/// The reason code `until` is refused with when its maximum passes
/// before its condition holds.
pub const UNTIL_NOT_REACHED: &str = "until_not_reached";
/// The reason code `load` is refused with when nothing was saved under
/// the name.
pub const NO_SUCH_SAVE: &str = "no_such_save";

/// How a console line can end without being carried out.
#[derive(Clone, PartialEq, Debug, thiserror::Error)]
pub enum ConsoleError {
    /// The line does not parse, or does not fit its command.
    #[error("{}", .0.iter().map(ToString::to_string).collect::<Vec<_>>().join("; "))]
    Syntax(Vec<Diagnostic>),
    /// The simulation declined, with a reason. Nothing changed.
    #[error("refused ({0})")]
    Refused(Refusal),
    /// The command could not be carried out at all.
    #[error("{0}")]
    Failed(String),
}

impl From<HostError> for ConsoleError {
    fn from(error: HostError) -> ConsoleError {
        match error {
            HostError::Refused(refusal) => ConsoleError::Refused(refusal),
            HostError::Failed(message) => ConsoleError::Failed(message),
        }
    }
}

/// An expectation that did not hold: what was expected and what was
/// found, each in words.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Mismatch {
    /// What the script expected.
    pub expected: String,
    /// What there was instead.
    pub found: String,
}

/// A console session.
///
/// ```
/// use clash_sim::console::testing::FakeHost;
/// use clash_sim::console::{HostKind, Session};
///
/// let table = FakeHost::table();
/// let mut host = FakeHost::new();
/// let mut session = Session::new(HostKind::File);
/// session.run_line(&table, &mut host, "new apples value=3 as=a").unwrap();
/// session.run_line(&table, &mut host, "add @a 2").unwrap();
/// let output = session.run_line(&table, &mut host, "eval @a.value * 2").unwrap();
/// assert_eq!(output.text, "10");
/// ```
#[derive(Clone, Debug)]
pub struct Session {
    kind: HostKind,
    names: BTreeMap<String, Value>,
    saves: BTreeMap<String, Vec<u8>>,
    records: Vec<Record>,
    last_refusal: Option<Refusal>,
    transcript: Vec<String>,
}

impl Session {
    /// A new session in a host of this kind.
    pub fn new(kind: HostKind) -> Session {
        Session {
            kind,
            names: BTreeMap::new(),
            saves: BTreeMap::new(),
            records: Vec::new(),
            last_refusal: None,
            transcript: Vec::new(),
        }
    }

    /// Parses a line and carries it out.
    pub fn run_line(
        &mut self,
        table: &CommandTable,
        host: &mut dyn ConsoleHost,
        source: &str,
    ) -> Result<Output, ConsoleError> {
        let line = parse_line(source).map_err(|error| ConsoleError::Syntax(vec![error]))?;
        self.run(table, host, &line)
    }

    /// Carries out a parsed line. A blank line or a comment does
    /// nothing.
    pub fn run(
        &mut self,
        table: &CommandTable,
        host: &mut dyn ConsoleHost,
        line: &Line,
    ) -> Result<Output, ConsoleError> {
        let Some(mut bound) = table.bind(line, self.kind).map_err(ConsoleError::Syntax)? else {
            if line.comment.is_some() {
                self.transcript.push(print_line(line));
            }
            return Ok(Output::none());
        };
        let result = self.dispatch(table, host, &mut bound);
        self.records.extend(host.drain_records());
        self.last_refusal = match &result {
            Err(ConsoleError::Refused(refusal)) => Some(refusal.clone()),
            Ok(_) => host.last_refusal(),
            Err(_) => None,
        };
        // The transcript holds what was carried out, and what was
        // refused together with the refusal, so that it replays as a
        // script that passes. A command that a file is not offered
        // (a break, a rewind) is left out: it could not be replayed.
        let replayable = table
            .get(&bound.name)
            .is_some_and(|spec| spec.hosts.offers(HostKind::File));
        match &result {
            _ if !replayable => {}
            Ok(_) => self.transcript.push(print_line(line)),
            Err(ConsoleError::Refused(refusal)) => {
                self.transcript.push(print_line(line));
                self.transcript
                    .push(format!("expect reason {}", refusal.code));
            }
            Err(_) => {}
        }
        result
    }

    fn dispatch(
        &mut self,
        table: &CommandTable,
        host: &mut dyn ConsoleHost,
        bound: &mut BoundCommand,
    ) -> Result<Output, ConsoleError> {
        if let Some(subject) = &mut bound.subject {
            self.resolve_arg(host, subject)?;
        }
        for (_, arg) in bound.args.iter_mut().chain(bound.fields.iter_mut()) {
            self.resolve_arg(host, arg)?;
        }
        let subject = match &bound.subject {
            Some(ArgValue::Value(value)) => Some(value.clone()),
            _ => None,
        };
        let missing = |name: &str| ConsoleError::Failed(format!("`{name}` has no value"));
        let builtin = bound.source == CommandSource::Builtin;
        let output = match bound.name.as_str() {
            "help" if builtin => Output::text(match bound.text("command") {
                Some(name) => table
                    .help(name)
                    .ok_or_else(|| ConsoleError::Failed(format!("there is no command `{name}`")))?,
                None => table.help_all(),
            }),
            "use" if builtin => {
                let profile = bound.word("profile").ok_or_else(|| missing("profile"))?;
                let seed = u64::try_from(bound.int("seed").unwrap_or(0))
                    .map_err(|_| ConsoleError::Failed("a seed is not negative".to_owned()))?;
                host.start(profile, seed)?
            }
            "gen" if builtin => {
                let kind = bound.word("kind").ok_or_else(|| missing("kind"))?;
                let size = u32::try_from(bound.int("size").unwrap_or(1))
                    .ok()
                    .filter(|size| *size >= 1)
                    .ok_or_else(|| ConsoleError::Failed("a size is at least 1".to_owned()))?;
                host.generate(kind, size)?
            }
            "eval" if builtin => {
                let expression = bound
                    .expr("expression")
                    .ok_or_else(|| missing("expression"))?;
                let value = host.eval(expression, subject.as_ref())?;
                Output::text(value.to_string()).with_subject(value)
            }
            "assert" if builtin => {
                let condition = bound
                    .expr("condition")
                    .ok_or_else(|| missing("condition"))?;
                match host.eval(condition, subject.as_ref())? {
                    Value::Bool(true) => Output::none(),
                    other => {
                        return Err(ConsoleError::Failed(format!(
                            "assertion failed: `{}` is {other}",
                            condition.written
                        )));
                    }
                }
            }
            "advance" if builtin => {
                host.advance(forward(bound.dur("time").ok_or_else(|| missing("time"))?)?)?;
                Output::none()
            }
            "until" if builtin => {
                let condition = bound
                    .expr("condition")
                    .ok_or_else(|| missing("condition"))?;
                let max = forward(bound.dur("max").ok_or_else(|| missing("max"))?)?;
                if !host.advance_until(condition, subject.as_ref(), max)? {
                    return Err(ConsoleError::Refused(
                        Refusal::new(
                            UNTIL_NOT_REACHED,
                            format!("`{}` did not come to hold within {max}", condition.written),
                        )
                        .with("max", max.as_secs_real()),
                    ));
                }
                Output::none()
            }
            "hash" if builtin && bound.arg("subsystem").is_none() => {
                Output::text(host.state_hash().to_string())
            }
            "save" if builtin => {
                let name = bound.word("name").ok_or_else(|| missing("name"))?;
                let bytes = host.save()?;
                let size = bytes.len();
                self.saves.insert(name.to_owned(), bytes);
                Output::text(format!("saved `{name}`, {size} bytes"))
            }
            "load" if builtin => {
                let name = bound.word("name").ok_or_else(|| missing("name"))?;
                let bytes = self.saves.get(name).ok_or_else(|| {
                    ConsoleError::Refused(
                        Refusal::new(NO_SUCH_SAVE, format!("nothing was saved as `{name}`"))
                            .with("name", name),
                    )
                })?;
                host.load(bytes)?;
                Output::text(format!("loaded `{name}`"))
            }
            "expect" if builtin => {
                return Err(ConsoleError::Failed(
                    "an `expect` line is an expectation of a test script, which the script runner \
                     checks; it is not a command"
                        .to_owned(),
                ));
            }
            _ => host.execute(bound)?,
        };
        if let Some(name) = &bound.result_name {
            match &output.subject {
                Some(value) => {
                    self.names.insert(name.clone(), value.clone());
                }
                None => {
                    return Err(ConsoleError::Failed(format!(
                        "`{}` gave nothing to name `{name}`",
                        bound.name
                    )));
                }
            }
        }
        Ok(output)
    }

    /// What a reference names: a name given with `as=` in this session,
    /// or whatever the host says.
    pub fn resolve(
        &self,
        host: &mut dyn ConsoleHost,
        reference: &Reference,
    ) -> Result<Value, ConsoleError> {
        if let Reference::Named(name) = reference
            && let Some(value) = self.names.get(name)
        {
            return Ok(value.clone());
        }
        Ok(host.resolve(reference)?)
    }

    fn resolve_arg(
        &self,
        host: &mut dyn ConsoleHost,
        arg: &mut ArgValue,
    ) -> Result<(), ConsoleError> {
        match arg {
            ArgValue::Ref(reference) => *arg = ArgValue::Value(self.resolve(host, reference)?),
            ArgValue::Expr(expression) => self.resolve_expression(host, expression)?,
            _ => {}
        }
        Ok(())
    }

    fn resolve_expression(
        &self,
        host: &mut dyn ConsoleHost,
        expression: &mut Expression,
    ) -> Result<(), ConsoleError> {
        for local in &mut expression.locals {
            local.value = self.resolve(host, &local.reference)?;
        }
        Ok(())
    }

    /// The value an expectation's field must have.
    fn field_value(
        &self,
        host: &mut dyn ConsoleHost,
        arg: &ArgValue,
    ) -> Result<Value, ConsoleError> {
        Ok(match arg {
            ArgValue::Value(value) => value.clone(),
            ArgValue::Dur(time) => Value::Real(time.as_secs_real()),
            ArgValue::Word(word) => Value::text(word),
            ArgValue::Ref(reference) => self.resolve(host, reference)?,
            ArgValue::Json(json) => json_value(json),
            ArgValue::Expr(expression) => {
                let mut expression = expression.clone();
                self.resolve_expression(host, &mut expression)?;
                host.eval(&expression, None)?
            }
        })
    }

    /// Checks an expectation of a script (14 §7.1). `files` reads a
    /// file named relative to the script, for `expect frame`.
    pub fn check(
        &mut self,
        host: &mut dyn ConsoleHost,
        expectation: &Expectation,
        files: &dyn Fn(&str) -> Option<String>,
    ) -> Result<(), Mismatch> {
        self.records.extend(host.drain_records());
        let failed = |expected: String, error: ConsoleError| Mismatch {
            expected,
            found: error.to_string(),
        };
        match expectation {
            Expectation::Assert { condition, subject } => {
                let expected = format!("`{}` to be true", condition.written);
                let mut condition = condition.clone();
                let value = self
                    .resolve_expression(host, &mut condition)
                    .and_then(|()| {
                        subject
                            .as_ref()
                            .map(|reference| self.resolve(host, reference))
                            .transpose()
                    })
                    .and_then(|subject| Ok(host.eval(&condition, subject.as_ref())?));
                match value {
                    Ok(Value::Bool(true)) => Ok(()),
                    Ok(other) => Err(Mismatch {
                        expected,
                        found: other.to_string(),
                    }),
                    Err(error) => Err(failed(expected, error)),
                }
            }
            Expectation::Event {
                name,
                fields,
                within,
            } => self.check_record(host, RecordSource::Event, name, fields, *within),
            Expectation::Trace { kind, fields } => {
                self.check_record(host, RecordSource::Trace, kind, fields, None)
            }
            Expectation::Reason { code } => match &self.last_refusal {
                Some(refusal) if refusal.code == *code => Ok(()),
                other => Err(Mismatch {
                    expected: format!("the previous command to be refused for `{code}`"),
                    found: match other {
                        Some(refusal) => format!("it was refused for `{refusal}`"),
                        None => "it was not refused".to_owned(),
                    },
                }),
            },
            Expectation::Hash { value, .. } => {
                let found = host.state_hash();
                match value {
                    Some(expected) if *expected == found => Ok(()),
                    Some(expected) => Err(Mismatch {
                        expected: format!("state hash {expected}"),
                        found: found.to_string(),
                    }),
                    None => Err(Mismatch {
                        expected: "a state hash written in the script".to_owned(),
                        found: format!("none; the hash now is {found}, which --bless writes in"),
                    }),
                }
            }
            Expectation::Frame { file } => {
                let expected = format!("the frame stored in `{file}`");
                let found = host
                    .frame()
                    .map_err(|error| failed(expected.clone(), error.into()))?;
                match files(file) {
                    Some(stored) if stored == found => Ok(()),
                    Some(stored) => Err(Mismatch {
                        expected: format!("{expected}:\n{stored}"),
                        found: format!("this frame:\n{found}"),
                    }),
                    None => Err(Mismatch {
                        expected,
                        found: "no such file; --bless writes it".to_owned(),
                    }),
                }
            }
        }
    }

    fn check_record(
        &mut self,
        host: &mut dyn ConsoleHost,
        source: RecordSource,
        name: &str,
        fields: &[(String, ArgValue)],
        within: Option<Dur>,
    ) -> Result<(), Mismatch> {
        let what = match source {
            RecordSource::Event => "event",
            RecordSource::Trace => "trace record",
        };
        let mut expected = format!("a `{name}` {what}");
        let mut wanted = Vec::new();
        for (field, arg) in fields {
            expected.push_str(&format!(" with {field}={}", arg.shown()));
            match self.field_value(host, arg) {
                Ok(value) => wanted.push((field.as_str(), value)),
                Err(error) => {
                    return Err(Mismatch {
                        expected,
                        found: format!("`{field}` could not be worked out: {error}"),
                    });
                }
            }
        }
        let now = host.now();
        if let Some(within) = within {
            expected.push_str(&format!(" within the last {within}"));
        }
        let in_span =
            |record: &Record| within.is_none_or(|within| now.since(record.tick) <= within);
        let candidates: Vec<&Record> = self
            .records
            .iter()
            .filter(|record| record.source == source && record.name == name)
            .collect();
        let found = candidates.iter().any(|record| {
            in_span(record)
                && wanted.iter().all(|(field, value)| {
                    record
                        .field(field)
                        .is_some_and(|found| values_match(value, found))
                })
        });
        if found {
            return Ok(());
        }
        let found = if candidates.is_empty() {
            let mut seen: Vec<&str> = self
                .records
                .iter()
                .filter(|record| record.source == source)
                .map(|record| record.name.as_str())
                .collect();
            seen.sort_unstable();
            seen.dedup();
            if seen.is_empty() {
                format!("no {what} at all")
            } else {
                format!("no such {what}; there were: {}", seen.join(", "))
            }
        } else {
            let shown: Vec<String> = candidates
                .iter()
                .rev()
                .take(MISMATCH_RECORDS_SHOWN)
                .map(ToString::to_string)
                .collect();
            format!(
                "{} of that name, none matching; the latest: {}",
                candidates.len(),
                shown.join("; ")
            )
        };
        Err(Mismatch { expected, found })
    }

    /// The events and trace records seen so far, oldest first.
    pub fn records(&self) -> &[Record] {
        &self.records
    }

    /// Why the previous command was refused, if it was.
    pub const fn last_refusal(&self) -> Option<&Refusal> {
        self.last_refusal.as_ref()
    }

    /// What a name given with `as=` names.
    pub fn name(&self, name: &str) -> Option<&Value> {
        self.names.get(name)
    }

    /// The session so far as a script: each line that was carried out,
    /// in canonical form, and each one that was refused followed by an
    /// `expect reason`. Lines that failed, and commands that are not
    /// offered in files, are left out. Run against a fresh host it
    /// gives the same state.
    pub fn transcript(&self) -> String {
        let mut out = String::new();
        for line in &self.transcript {
            out.push_str(line);
            out.push('\n');
        }
        out
    }
}

/// How many near misses a failed `expect event` shows.
const MISMATCH_RECORDS_SHOWN: usize = 3;

/// A time to advance by, which must not be negative.
fn forward(time: Dur) -> Result<Dur, ConsoleError> {
    if time.is_negative() {
        Err(ConsoleError::Failed(format!(
            "time only goes forward, and {time} is negative; `rewind` goes back"
        )))
    } else {
        Ok(time)
    }
}

/// Whether a field has the value an expectation wants: numbers by
/// value, an integer equal to the same real, everything else exactly.
fn values_match(wanted: &Value, found: &Value) -> bool {
    match (wanted.as_number(), found.as_number()) {
        (Some(wanted), Some(found)) => wanted == found,
        _ => wanted == found,
    }
}

/// JSON as a value of the expression language.
fn json_value(json: &serde_json::Value) -> Value {
    match json {
        serde_json::Value::Null => Value::Null,
        serde_json::Value::Bool(value) => Value::Bool(*value),
        serde_json::Value::Number(number) => match number.as_i64() {
            Some(whole) => Value::Int(whole),
            None => Value::Real(Real::of(number.as_f64().unwrap_or(0.0))),
        },
        serde_json::Value::String(text) => Value::text(text),
        serde_json::Value::Array(items) => Value::list(items.iter().map(json_value)),
        serde_json::Value::Object(entries) => {
            Value::map(entries.iter().map(|(key, value)| (key, json_value(value))))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::console::testing::FakeHost;
    use crate::console::{StepKind, codes, parse_script};

    struct Console {
        table: CommandTable,
        host: FakeHost,
        session: Session,
    }

    impl Console {
        fn new() -> Console {
            Console {
                table: FakeHost::table(),
                host: FakeHost::new(),
                session: Session::new(HostKind::File),
            }
        }

        fn run(&mut self, source: &str) -> Result<Output, ConsoleError> {
            self.session.run_line(&self.table, &mut self.host, source)
        }

        fn ok(&mut self, source: &str) -> String {
            self.run(source)
                .unwrap_or_else(|error| panic!("{source}: {error}"))
                .text
        }

        fn expect(&mut self, source: &str) -> Result<(), Mismatch> {
            let script = parse_script(source).unwrap_or_else(|errors| panic!("{errors:?}"));
            let StepKind::Expect(expectation) = &script.steps[0].kind else {
                panic!("{source} is not an expectation")
            };
            self.session
                .check(&mut self.host, expectation, &|file: &str| {
                    (file == "frames/two.txt")
                        .then(|| "fake host  profile=fake seed=7  t=0s\na = 1\nb = 5\n".to_owned())
                })
        }
    }

    #[test]
    fn names_given_with_as_are_kept_by_the_session() {
        let mut console = Console::new();
        console.ok("new apples value=3 as=a");
        console.ok("new pears as=p");
        console.ok("add @a 4");
        assert_eq!(console.ok("eval @a.value + @p.value"), "7");
        assert_eq!(console.ok("as @a eval self.value"), "7");
        assert!(console.session.name("a").is_some());
        // A name can be given to what `eval` finds.
        console.ok("eval cell('pears') as=q");
        console.ok("set @q 9");
        assert_eq!(console.ok("eval @p.value"), "9");
        // And a command that gives nothing cannot be named.
        let error = console.run("advance 1s as=x").unwrap_err();
        assert!(
            matches!(error, ConsoleError::Failed(message) if message.contains("nothing to name"))
        );
    }

    #[test]
    fn syntax_and_binding_errors_stop_a_line() {
        let mut console = Console::new();
        let ConsoleError::Syntax(errors) = console.run("new \"a").unwrap_err() else {
            panic!()
        };
        assert_eq!(errors[0].code, codes::UNCLOSED_QUOTE);
        let ConsoleError::Syntax(errors) = console.run("nw a").unwrap_err() else {
            panic!()
        };
        assert_eq!(errors[0].code, codes::UNKNOWN_COMMAND);
        assert_eq!(errors[0].suggestion.as_deref(), Some("did you mean `new`?"));
        assert_eq!(console.session.transcript(), "");
    }

    #[test]
    fn refusals_carry_reasons_and_are_remembered() {
        let mut console = Console::new();
        console.ok("new a");
        console.ok("lock @a");
        let ConsoleError::Refused(refusal) = console.run("add @a").unwrap_err() else {
            panic!()
        };
        assert_eq!(refusal.code, "cell_locked");
        assert_eq!(console.session.last_refusal().unwrap().code, "cell_locked");
        assert_eq!(console.expect("expect reason cell_locked"), Ok(()));
        let mismatch = console.expect("expect reason no_fuel").unwrap_err();
        assert_eq!(
            mismatch.expected,
            "the previous command to be refused for `no_fuel`"
        );
        assert!(mismatch.found.contains("cell_locked"), "{}", mismatch.found);
        console.ok("check");
        assert_eq!(
            console
                .expect("expect reason cell_locked")
                .unwrap_err()
                .found,
            "it was not refused"
        );
        // A reference to nothing is a refusal too.
        let ConsoleError::Refused(refusal) = console.run("add @nobody").unwrap_err() else {
            panic!()
        };
        assert_eq!(refusal.code, "unknown_name");
    }

    #[test]
    fn time_events_and_traces() {
        let mut console = Console::new();
        console.ok("new a as=a");
        console.ok("timer @a every=1s");
        console.ok("advance 2500ms");
        assert_eq!(console.ok("eval @a.value"), "2");
        assert_eq!(console.ok("eval clock()"), "2.5");
        assert_eq!(
            console.expect("expect event timer_fired cell=@a value=2"),
            Ok(())
        );
        assert_eq!(
            console.expect("expect event timer_fired value=2 within=1s"),
            Ok(())
        );
        assert_eq!(
            console.expect("expect event timer_fired value=\"1 + 1\""),
            Ok(())
        );
        assert_eq!(console.expect("expect event timer_fired name=a"), Ok(()));
        assert_eq!(console.expect("expect trace timer cell=@a"), Ok(()));
        let mismatch = console
            .expect("expect event timer_fired value=1 within=1s")
            .unwrap_err();
        assert_eq!(
            mismatch.expected,
            "a `timer_fired` event with value=1 within the last 1s"
        );
        assert!(
            mismatch.found.starts_with("2 of that name, none matching"),
            "{}",
            mismatch.found
        );
        let mismatch = console.expect("expect event explosion").unwrap_err();
        assert!(
            mismatch
                .found
                .contains("there were: cell_made, timer_fired"),
            "{}",
            mismatch.found
        );
        assert!(
            console
                .expect("expect trace strike")
                .unwrap_err()
                .found
                .contains("timer")
        );

        console.ok("until \"@a.value == 5\" max=10s");
        assert_eq!(console.ok("eval clock()"), "5");
        let ConsoleError::Refused(refusal) =
            console.run("until @a.value == 99 max 2s").unwrap_err()
        else {
            panic!()
        };
        assert_eq!(refusal.code, UNTIL_NOT_REACHED);
        assert_eq!(console.ok("eval clock()"), "7");
        assert!(matches!(
            console.run("advance -1s"),
            Err(ConsoleError::Failed(_))
        ));
    }

    #[test]
    fn assertions_hashes_and_frames() {
        let mut console = Console::new();
        console.ok("use fake seed=7");
        console.ok("new a value=1");
        console.ok("new b value=5");
        assert_eq!(console.expect("assert \"@a.value < @b.value\""), Ok(()));
        assert_eq!(console.expect("as @b assert self.value == 5"), Ok(()));
        let mismatch = console.expect("assert @a.value == 2").unwrap_err();
        assert_eq!(mismatch.expected, "`@a.value == 2` to be true");
        assert_eq!(mismatch.found, "false");
        assert!(
            console
                .expect("assert @a.value")
                .unwrap_err()
                .found
                .contains('1')
        );
        assert!(
            console
                .expect("assert @zz.value == 1")
                .unwrap_err()
                .found
                .contains("unknown_name")
        );
        assert!(matches!(
            console.run("assert @a.value == 2"),
            Err(ConsoleError::Failed(_))
        ));
        console.ok("assert @a.value == 1");

        let hash = console.ok("hash");
        assert_eq!(hash.len(), 32);
        assert_eq!(console.expect(&format!("expect hash {hash}")), Ok(()));
        let other = "0".repeat(32);
        let mismatch = console.expect(&format!("expect hash {other}")).unwrap_err();
        assert_eq!(mismatch.expected, format!("state hash {other}"));
        assert_eq!(mismatch.found, hash);
        assert!(
            console
                .expect("expect hash")
                .unwrap_err()
                .found
                .contains("--bless")
        );

        assert_eq!(console.expect("expect frame frames/two.txt"), Ok(()));
        assert!(
            console
                .expect("expect frame frames/none.txt")
                .unwrap_err()
                .found
                .contains("no such file")
        );
        console.ok("add @b");
        let mismatch = console.expect("expect frame frames/two.txt").unwrap_err();
        assert!(mismatch.expected.contains("b = 5") && mismatch.found.contains("b = 6"));
    }

    #[test]
    fn save_and_load_by_name() {
        let mut console = Console::new();
        console.ok("new a value=1 as=a");
        let before = console.ok("hash");
        console.ok("save before");
        console.ok("add @a 10");
        assert_ne!(console.ok("hash"), before);
        console.ok("load before");
        assert_eq!(console.ok("hash"), before);
        assert_eq!(console.ok("eval @a.value"), "1");
        let ConsoleError::Refused(refusal) = console.run("load never").unwrap_err() else {
            panic!()
        };
        assert_eq!(refusal.code, NO_SUCH_SAVE);
    }

    #[test]
    fn help_comes_from_the_table() {
        let mut console = Console::new();
        assert!(console.ok("help").contains("advance <time>"));
        assert!(
            console
                .ok("help timer")
                .starts_with("timer <cell> <every> [times]\n")
        );
        assert!(matches!(
            console.run("help nothing"),
            Err(ConsoleError::Failed(_))
        ));
    }

    #[test]
    fn a_session_saved_as_a_file_replays_to_the_same_hash() {
        // TL1: "A console session saved as a file replays to the same
        // hash."
        let mut console = Console::new();
        for line in [
            "use fake seed=3   # start",
            "gen flat size=2",
            "new a value=2 as=a",
            "timer @a every=250ms times=6",
            "advance   1s",
            "lock @c:0,0,0",
            "add @c:0,0,0",
            "# a refusal above",
            "until \"@a.value == 8\" max=1min",
            "do { \"anything\": { \"n\": [1, 2] } }",
        ] {
            let _ = console.run(line);
        }
        let _ = console.run("nonsense that is not logged");
        let hash = console.ok("hash");
        let file = console.session.transcript();
        assert!(
            file.contains("add @c:0,0,0\nexpect reason cell_locked\n"),
            "{file}"
        );
        assert!(file.contains("use fake seed=3 # start\n"), "{file}");
        assert!(!file.contains("nonsense"));

        let script = parse_script(&file).unwrap();
        let mut replay = Console::new();
        for step in &script.steps {
            match &step.kind {
                StepKind::Command(line) => {
                    let _ = replay.session.run(&replay.table, &mut replay.host, line);
                }
                StepKind::Expect(expectation) => {
                    replay
                        .session
                        .check(&mut replay.host, expectation, &|_| None)
                        .unwrap();
                }
            }
        }
        assert_eq!(replay.ok("hash"), hash);
        // What the replay logs is the file again.
        let commands: String = file
            .lines()
            .filter(|line| !line.starts_with('#'))
            .map(|line| format!("{line}\n"))
            .collect();
        assert_eq!(replay.session.transcript(), format!("{commands}hash\n"));
    }

    #[test]
    fn json_becomes_values() {
        let json: serde_json::Value =
            serde_json::from_str("{\"a\": [1, 2.5, \"x\", true, null]}").unwrap();
        assert_eq!(
            json_value(&json).to_string(),
            "{a: [1, 2.5, x, true, null]}"
        );
    }
}
