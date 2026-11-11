//! [`ConsoleHost`]: the seam between the console and whatever carries
//! its commands out.
//!
//! The console parses a line, binds it to a command and resolves its
//! references; the host does what the command says. The simulation
//! implements this trait once, and the game overlay, the terminal viewer
//! and the test-script runner all drive it the same way (14 §2,
//! principle 4). A small host for tests is in
//! [`testing`](super::testing).

use core::fmt;

use clash_core::{Dur, Hash128, Tick};
use clash_expr::Value;

use super::command::BoundCommand;
use super::diag::{Diagnostic, codes, span};
use super::syntax::{Reference, parse_reference};

/// What a command that was carried out gives back.
#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct Output {
    /// What to print: nothing, a line, or several lines.
    pub text: String,
    /// What the command made or found, if it is one thing that can be
    /// named with `as=`: usually a subject.
    pub subject: Option<Value>,
}

impl Output {
    /// An output with nothing to print and nothing to name.
    pub fn none() -> Output {
        Output::default()
    }

    /// An output that prints this.
    pub fn text(text: impl Into<String>) -> Output {
        Output {
            text: text.into(),
            subject: None,
        }
    }

    /// The same output, giving this to be named with `as=`.
    #[must_use]
    pub fn with_subject(mut self, subject: impl Into<Value>) -> Output {
        self.subject = Some(subject.into());
        self
    }
}

/// Why the simulation declined to do something (14 §5): a stable code, a
/// sentence, and the values involved.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Refusal {
    /// The reason's code: the id of a `reason` definition or one of the
    /// engine's own.
    pub code: String,
    /// The sentence, already written out for a person.
    pub message: String,
    /// The values involved, by name.
    pub values: Vec<(String, Value)>,
}

impl Refusal {
    /// A refusal with no values.
    pub fn new(code: impl Into<String>, message: impl Into<String>) -> Refusal {
        Refusal {
            code: code.into(),
            message: message.into(),
            values: Vec::new(),
        }
    }

    /// The same refusal with one more value.
    #[must_use]
    pub fn with(mut self, name: impl Into<String>, value: impl Into<Value>) -> Refusal {
        self.values.push((name.into(), value.into()));
        self
    }
}

/// `code: message`.
impl fmt::Display for Refusal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.code, self.message)
    }
}

/// How a request to the host can end without being carried out.
#[derive(Clone, PartialEq, Eq, Debug, thiserror::Error)]
pub enum HostError {
    /// The simulation understood and declined, with a reason. Nothing
    /// changed.
    #[error("refused ({0})")]
    Refused(Refusal),
    /// The request could not be carried out at all: an expression that
    /// does not compile, a save that does not read, a command this host
    /// does not have. The message is for a person.
    #[error("{0}")]
    Failed(String),
}

impl HostError {
    /// A failure with this message.
    pub fn failed(message: impl Into<String>) -> HostError {
        HostError::Failed(message.into())
    }

    /// A refusal with this code and message.
    pub fn refused(code: impl Into<String>, message: impl Into<String>) -> HostError {
        HostError::Refused(Refusal::new(code, message))
    }
}

impl From<Refusal> for HostError {
    fn from(refusal: Refusal) -> HostError {
        HostError::Refused(refusal)
    }
}

/// What a host gives back.
pub type HostResult<T> = Result<T, HostError>;

/// Whether a record is an event or a trace record.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub enum RecordSource {
    /// An event of the simulation (05 §8), for `expect event`.
    Event,
    /// A trace record (14 §6.1), for `expect trace`.
    Trace,
}

/// An event or a trace record in the plain form expectations are
/// checked against: the name of its type or kind, when it happened, and
/// its fields by name.
///
/// A field that is an id is given as its qualified id, a text, because
/// only the host can name a dense id. A field that is a subject is given
/// as the same value [`ConsoleHost::resolve`] gives for a reference to
/// it.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Record {
    /// Event or trace.
    pub source: RecordSource,
    /// The event's type or the trace's kind: `strike_defended`, `strike`.
    pub name: String,
    /// When it happened.
    pub tick: Tick,
    /// Its fields, subjects included, by name.
    pub fields: Vec<(String, Value)>,
}

impl Record {
    /// An event record with no fields.
    pub fn event(name: impl Into<String>, tick: Tick) -> Record {
        Record {
            source: RecordSource::Event,
            name: name.into(),
            tick,
            fields: Vec::new(),
        }
    }

    /// A trace record with no fields.
    pub fn trace(kind: impl Into<String>, tick: Tick) -> Record {
        Record {
            source: RecordSource::Trace,
            ..Record::event(kind, tick)
        }
    }

    /// The same record with one more field.
    #[must_use]
    pub fn with(mut self, name: impl Into<String>, value: impl Into<Value>) -> Record {
        self.fields.push((name.into(), value.into()));
        self
    }

    /// The value of a field, if the record has it.
    pub fn field(&self, name: &str) -> Option<&Value> {
        self.fields
            .iter()
            .find(|(field, _)| field == name)
            .map(|(_, value)| value)
    }
}

/// `name field=value …`, as a trace line is written.
impl fmt::Display for Record {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} tick={}", self.name, self.tick.as_micros())?;
        for (name, value) in &self.fields {
            write!(f, " {name}={value}")?;
        }
        Ok(())
    }
}

/// A reference written inside an expression, and the local that stands
/// for it.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct ExprLocal {
    /// The local's name, without the `$`: `at0`.
    pub name: String,
    /// The reference as it was written.
    pub reference: Reference,
    /// What the reference names. Null until the console has resolved it,
    /// which it always has before a host sees the expression.
    pub value: Value,
}

/// An expression typed at the console, ready for the expression
/// language.
///
/// The console's expressions may name references (`"@b.wounds.count ==
/// 0"`), which the language of 05 §4 does not have. The console rewrites
/// each one to a local, `$at0`, `$at1` and so on, and hands the host the
/// rewritten source together with what each local holds. A host compiles
/// the source in a context that declares those locals and answers for
/// them when the expression runs.
///
/// ```
/// use clash_sim::console::{Expression, Reference};
///
/// let expression = Expression::parse("dist(@a, @b) < 3 and @a.name != '@b'").unwrap();
/// assert_eq!(expression.source, "dist($at0, $at1) < 3 and $at0.name != '@b'");
/// assert_eq!(expression.locals[1].reference, Reference::Named("b".into()));
/// assert_eq!(expression.written, "dist(@a, @b) < 3 and @a.name != '@b'");
/// ```
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Expression {
    /// The expression as it was typed.
    pub written: String,
    /// The expression in the language of 05 §4, references rewritten to
    /// locals.
    pub source: String,
    /// The locals, in the order their references first appear.
    pub locals: Vec<ExprLocal>,
}

impl Expression {
    /// Rewrites the references in an expression to locals and checks
    /// that the result parses. Spans in an error count from the start
    /// of `written`.
    pub fn parse(written: &str) -> Result<Expression, Diagnostic> {
        let bytes = written.as_bytes();
        let mut source = String::with_capacity(written.len());
        let mut locals: Vec<ExprLocal> = Vec::new();
        let mut at = 0;
        let mut copied = 0;
        while let Some(&byte) = bytes.get(at) {
            match byte {
                b'\'' => {
                    // A string of the expression language: skip it,
                    // doubled quotes included.
                    at += 1;
                    while let Some(&inner) = bytes.get(at) {
                        at += 1;
                        if inner == b'\'' {
                            if bytes.get(at) == Some(&b'\'') {
                                at += 1;
                            } else {
                                break;
                            }
                        }
                    }
                }
                b'@' => {
                    let (reference, used) = parse_reference(&written[at..])
                        .map_err(|error| error.shifted(u32::try_from(at).unwrap_or(u32::MAX)))?;
                    source.push_str(&written[copied..at]);
                    let index = locals
                        .iter()
                        .position(|local| local.reference == reference)
                        .unwrap_or_else(|| {
                            locals.push(ExprLocal {
                                name: format!("at{}", locals.len()),
                                reference,
                                value: Value::Null,
                            });
                            locals.len() - 1
                        });
                    source.push('$');
                    source.push_str(&locals[index].name);
                    at += used;
                    copied = at;
                }
                _ => at += 1,
            }
        }
        source.push_str(&written[copied..]);
        if let Err(error) = clash_expr::parse_expr(&source) {
            return Err(Diagnostic::new(
                codes::BAD_EXPRESSION,
                span(0, written.len()),
                format!("the expression does not parse: {}", error.message),
            )
            .with_suggestion_opt(error.suggestion));
        }
        Ok(Expression {
            written: written.to_owned(),
            source,
            locals,
        })
    }
}

/// What carries the console's commands out.
///
/// The simulation implements this. The console and the test-script
/// runner need nothing else of it, which is what lets them be built and
/// tested before it exists, against
/// [`FakeHost`](super::testing::FakeHost).
///
/// # What a host may rely on
///
/// - Every reference in a [`BoundCommand`] and in an [`Expression`] has
///   been resolved before the host sees it: the console asks
///   [`resolve`](ConsoleHost::resolve) for the forms of 14 §3.2 and
///   keeps the names given with `as=` itself.
/// - Arguments have been matched to the command's parameters, their
///   types checked as far as they can be without evaluating, and
///   defaults filled in.
///
/// # What the console relies on
///
/// - Nothing here panics on what a person typed, and nothing blocks. A
///   host that reaches a decision nobody is there to make takes the
///   default (03 §5.2) or refuses with a reason.
/// - A refusal changes nothing. Whatever is refused says why (14 §5).
/// - A command that changes the simulation goes through a `Debug`
///   command (03 §4.4), so that it is validated, logged and replayed
///   like any other (14 §3.3).
/// - [`load`](ConsoleHost::load) works on a host fresh from wherever
///   hosts come from, with nothing else done to it first: the runner
///   saves one host and loads into another to check rule R7.
pub trait ConsoleHost {
    /// `use`: starts again from a profile and a seed, discarding
    /// whatever state there was.
    fn start(&mut self, profile: &str, seed: u64) -> HostResult<Output>;

    /// `gen`: builds a small world of a kind the host knows, for a test
    /// to happen in. `size` is in the kind's own measure and is at least
    /// one.
    fn generate(&mut self, kind: &str, size: u32) -> HostResult<Output>;

    /// Carries out a command that is neither one of the console's own
    /// nor one of the other operations of this trait: a verb, an engine
    /// function, a mod's command, or a built-in such as `inspect`.
    fn execute(&mut self, command: &BoundCommand) -> HostResult<Output>;

    /// What a reference names. The console asks for `@here`, `@self`,
    /// `@sel`, `@c:x,y,z`, `@e:id` and `@kind:name`, and for a `@name`
    /// that was not given with `as=` in this session.
    fn resolve(&mut self, reference: &Reference) -> HostResult<Value>;

    /// Evaluates an expression. `subject` is what `self` is bound to;
    /// `None` leaves it to the host, which binds the active character if
    /// there is one. The expression is truth-context: the console is a
    /// debug tool (14 §4.4).
    fn eval(&mut self, expression: &Expression, subject: Option<&Value>) -> HostResult<Value>;

    /// `advance`: lets this much time pass.
    fn advance(&mut self, time: Dur) -> HostResult<()>;

    /// `until`: lets time pass until the condition holds, checking it
    /// between events, or until `max` has passed. Gives whether the
    /// condition came to hold.
    fn advance_until(
        &mut self,
        condition: &Expression,
        subject: Option<&Value>,
        max: Dur,
    ) -> HostResult<bool>;

    /// The time now.
    fn now(&self) -> Tick;

    /// The hash of the whole state (01 §4).
    fn state_hash(&self) -> Hash128;

    /// The whole state as bytes.
    fn save(&mut self) -> HostResult<Vec<u8>>;

    /// Replaces the whole state with one that was saved.
    fn load(&mut self, bytes: &[u8]) -> HostResult<()>;

    /// The events and trace records produced since this was last
    /// called, oldest first. Trace records are produced for whatever
    /// tracing is switched on; a host driven by a test script switches
    /// on what `expect trace` needs.
    fn drain_records(&mut self) -> Vec<Record>;

    /// Why the most recent command was refused, or `None` if it was
    /// not. The simulation may accept a command and refuse what it asked
    /// for a moment later (an action that cannot start when its turn
    /// comes), which is why the console asks instead of remembering.
    fn last_refusal(&self) -> Option<Refusal>;

    /// The frame as it would be presented now, as text (12 §12), for
    /// `expect frame`.
    fn frame(&mut self) -> HostResult<String>;

    /// Sets how many threads the host may use. The state must not
    /// depend on it (rule R1).
    fn set_threads(&mut self, threads: u32) -> HostResult<()>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn references_in_strings_are_left_alone() {
        let expression = Expression::parse("'a@b' + '''@c'").unwrap();
        assert_eq!(expression.source, "'a@b' + '''@c'");
        assert!(expression.locals.is_empty());
    }

    #[test]
    fn every_reference_form_becomes_a_local() {
        let expression =
            Expression::parse("[@here, @self, @sel, @c:1,2,3, @e:4, @site:mill, @g1, @here]")
                .unwrap();
        assert_eq!(
            expression.source,
            "[$at0, $at1, $at2, $at3, $at4, $at5, $at6, $at0]"
        );
        assert_eq!(expression.locals.len(), 7);
        assert_eq!(
            expression.locals[3].reference,
            Reference::Cell { x: 1, y: 2, z: 3 }
        );
    }

    #[test]
    fn a_bad_expression_is_a_diagnostic() {
        let error = Expression::parse("1 +").unwrap_err();
        assert_eq!(error.code, codes::BAD_EXPRESSION);
        assert!(error.message.contains("does not parse"));
        let error = Expression::parse("1 + @").unwrap_err();
        assert_eq!(error.code, codes::BAD_REFERENCE);
        assert_eq!(error.span.start, 4);
    }

    #[test]
    fn the_trait_is_object_safe() {
        fn takes(_: &mut dyn ConsoleHost) {}
        let _ = takes;
    }

    #[test]
    fn records_print_as_trace_lines() {
        let record = Record::trace("strike", Tick(12))
            .with("turned", 162)
            .with("landed", 18);
        assert_eq!(record.to_string(), "strike tick=12 turned=162 landed=18");
        assert_eq!(record.field("landed"), Some(&Value::Int(18)));
        assert_eq!(record.field("missing"), None);
    }
}
