//! The test-script format (14 §7): a console session with expectations,
//! kept in a file.
//!
//! A script is console lines, one to a line of the file. A line whose
//! JSON is still open at its end goes on over the following lines until
//! the closing brace. Blank lines and comments are skipped. `assert` and
//! `expect` lines are expectations (14 §7.1); every other line is a
//! command, bound and run when the script is.
//!
//! | Line | Passes when |
//! |------|-------------|
//! | `assert <expression>` | The expression is true now |
//! | `expect event <type> <field>=<value>… [within=<time>]` | Such an event occurred, in that span before now if one is given, and at any time in the run otherwise |
//! | `expect trace <kind> <field>=<value>…` | Such a trace record was produced |
//! | `expect reason <code>` | The previous command was refused for that reason |
//! | `expect hash [<value>]` | The state hash is that value. Without a value it fails until `--bless` writes one in |
//! | `expect frame <file>` | The presented frame equals the stored one |

use clash_core::{Dur, Hash128};

use super::command::{ArgValue, CommandTable, HostKind, duration, field_value};
use super::diag::{Diagnostic, Span, char_floor, codes, span};
use super::host::Expression;
use super::syntax::{Line, MAX_LINE_LEN, Reference, ValueKind, parse_line, print_value};

/// A parsed test script.
///
/// ```
/// use clash_sim::console::{Expectation, StepKind, parse_script};
///
/// let script = parse_script(
///     "# a comment\n\
///      new a value=2\n\
///      do {\n  \"stop\": {}\n}\n\
///      assert \"@a.value == 2\"\n",
/// )
/// .unwrap();
/// assert_eq!(script.steps.len(), 3);
/// assert_eq!(script.steps[1].line, 3);
/// assert_eq!(script.steps[2].line, 6);
/// assert!(matches!(script.steps[2].kind, StepKind::Expect(Expectation::Assert { .. })));
/// ```
#[derive(Clone, PartialEq, Debug, Default)]
pub struct Script {
    /// The commands and expectations, in order.
    pub steps: Vec<Step>,
}

impl Script {
    /// How many of the steps are commands.
    pub fn commands(&self) -> usize {
        self.steps
            .iter()
            .filter(|step| matches!(step.kind, StepKind::Command(_)))
            .count()
    }
}

/// One line of a script that does something.
#[derive(Clone, PartialEq, Debug)]
pub struct Step {
    /// The line of the file it starts on, counting from one.
    pub line: usize,
    /// Where it starts in the file, in bytes.
    pub offset: usize,
    /// The line as written; several lines of the file where JSON runs
    /// over them.
    pub source: String,
    /// What it is.
    pub kind: StepKind,
}

/// A command or an expectation.
#[derive(Clone, PartialEq, Debug)]
pub enum StepKind {
    /// A console command.
    Command(Line),
    /// Something the script expects to hold at this point.
    Expect(Expectation),
}

/// What a script expects (14 §7.1).
#[derive(Clone, PartialEq, Debug)]
pub enum Expectation {
    /// `assert`: the expression is true now.
    Assert {
        /// The expression.
        condition: Expression,
        /// What `self` is, if the line began with `as <reference>`.
        subject: Option<Reference>,
    },
    /// `expect event`: such an event occurred.
    Event {
        /// The event's type.
        name: String,
        /// The fields it must have, with their values.
        fields: Vec<(String, ArgValue)>,
        /// How far back from now to look. The whole run if `None`.
        within: Option<Dur>,
    },
    /// `expect trace`: such a trace record was produced.
    Trace {
        /// The trace's kind.
        kind: String,
        /// The fields it must have, with their values.
        fields: Vec<(String, ArgValue)>,
    },
    /// `expect reason`: the previous command was refused for this
    /// reason.
    Reason {
        /// The reason's code.
        code: String,
    },
    /// `expect hash`: the state hash is this value.
    Hash {
        /// The value, if one is written.
        value: Option<Hash128>,
        /// Where the value is in the file, in bytes; or, if none is
        /// written, the empty span just after the word `hash`, where one
        /// would go.
        at: Span,
    },
    /// `expect frame`: the presented frame equals the one stored in this
    /// file, named relative to the script.
    Frame {
        /// The file.
        file: String,
    },
}

/// Something wrong with a script, with where it is in the file.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct ScriptError {
    /// The line of the file, counting from one.
    pub line: usize,
    /// The column, counting characters from one.
    pub column: usize,
    /// The script line the diagnostic is about, as written.
    pub source: String,
    /// What is wrong. Its span counts bytes of `source`.
    pub diagnostic: Diagnostic,
}

impl ScriptError {
    /// `path:line:column: error[E1402]: message`, then the line with
    /// carets under the place and the suggestion.
    pub fn render(&self, path: &str) -> String {
        format!(
            "{path}:{}:{}: {}",
            self.line,
            self.column,
            self.diagnostic.render(&self.source)
        )
    }
}

/// Parses a script. Every mistake in it is reported, each with its line
/// and column.
pub fn parse_script(text: &str) -> Result<Script, Vec<ScriptError>> {
    let table = CommandTable::builtin();
    let mut starts = Vec::new();
    let mut offset = 0;
    for piece in text.split('\n') {
        starts.push((offset, offset + piece.len()));
        offset += piece.len() + 1;
    }
    let mut steps = Vec::new();
    let mut errors = Vec::new();
    let mut index = 0;
    while let Some(&(start, mut end)) = starts.get(index) {
        let line_number = index + 1;
        // JSON left open at the end of a line goes on over the next.
        let parsed = loop {
            match parse_line(&text[start..end]) {
                Err(error)
                    if error.code == codes::UNCLOSED_JSON
                        && index + 1 < starts.len()
                        && end - start < MAX_LINE_LEN =>
                {
                    index += 1;
                    end = starts[index].1;
                }
                other => break other,
            }
        };
        index += 1;
        let source = &text[start..end];
        let mut fail = |diagnostic: Diagnostic| {
            let before = &source[..char_floor(source, diagnostic.span.start as usize)];
            let last_line = before.rfind('\n').map_or(0, |at| at + 1);
            errors.push(ScriptError {
                line: line_number + before.matches('\n').count(),
                column: before[last_line..].chars().count() + 1,
                source: source.to_owned(),
                diagnostic,
            });
        };
        match parsed {
            Err(error) => fail(error),
            Ok(line) if line.is_blank() => {}
            Ok(line) => match step_kind(&table, line, source, start) {
                Ok(kind) => steps.push(Step {
                    line: line_number,
                    offset: start,
                    source: source.to_owned(),
                    kind,
                }),
                Err(found) => found.into_iter().for_each(&mut fail),
            },
        }
    }
    if errors.is_empty() {
        Ok(Script { steps })
    } else {
        Err(errors)
    }
}

/// What a line with a command is: an expectation if it is `assert` or
/// `expect`, a command otherwise.
fn step_kind(
    table: &CommandTable,
    line: Line,
    source: &str,
    offset: usize,
) -> Result<StepKind, Vec<Diagnostic>> {
    let Some(command) = &line.command else {
        return Ok(StepKind::Command(line));
    };
    let bad = |at: Span, message: &str| {
        vec![
            Diagnostic::new(codes::BAD_EXPECTATION, at, message).with_suggestion(
                "an expectation is `expect event <type> <fields> within=<time>`, `expect trace <kind> <fields>`, `expect reason <code>`, `expect hash <value>` or `expect frame <file>`",
            ),
        ]
    };
    match command.name.as_str() {
        "assert" => {
            let bound = table.bind(&line, HostKind::File)?;
            let condition = bound
                .as_ref()
                .and_then(|bound| bound.expr("condition"))
                .cloned()
                .ok_or_else(|| bad(command.name_span, "`assert` needs an expression"))?;
            Ok(StepKind::Expect(Expectation::Assert {
                condition,
                subject: line.subject.map(|subject| subject.reference),
            }))
        }
        "expect" => {
            // An expectation is read from the line itself and not
            // through the command table, so that an event's fields may
            // have any names.
            let mut positional = command
                .args
                .iter()
                .filter(|arg| arg.key.is_none())
                .map(|arg| &arg.value);
            let what = match positional.next() {
                Some(value) => match &value.kind {
                    ValueKind::Word(word) => word.as_str(),
                    _ => return Err(bad(value.span, "this is not a kind of expectation")),
                },
                None => {
                    return Err(bad(
                        command.name_span,
                        "`expect` needs to be told what is expected",
                    ));
                }
            };
            // The value after the kind, as it was written.
            let written = positional.next();
            if let Some(extra) = positional.next() {
                return Err(bad(
                    extra.span,
                    "an expectation names one thing; a field is written `field=value`",
                ));
            }
            let read_fields = || {
                let mut fields = Vec::new();
                let mut errors = Vec::new();
                for arg in &command.args {
                    let Some(key) = &arg.key else { continue };
                    if key.name == "within" {
                        continue;
                    }
                    match field_value(&arg.value) {
                        Ok(value) => fields.push((key.name.clone(), value)),
                        Err(error) => errors.push(error),
                    }
                }
                if errors.is_empty() {
                    Ok(fields)
                } else {
                    Err(errors)
                }
            };
            let name = |missing: &str| {
                written
                    .map(|value| match &value.kind {
                        ValueKind::Quoted(text) => text.clone(),
                        other => print_value(other),
                    })
                    .ok_or_else(|| bad(command.name_span, missing))
            };
            let no_fields = |kind: &str| match command.args.iter().find_map(|arg| arg.key.as_ref())
            {
                Some(key) => Err(bad(
                    key.span,
                    &format!("`expect {kind}` takes no `{}=`", key.name),
                )),
                None => Ok(()),
            };
            let expectation = match what {
                "event" => {
                    let mut within = None;
                    if let Some(arg) = command
                        .args
                        .iter()
                        .find(|arg| arg.key.as_ref().is_some_and(|key| key.name == "within"))
                    {
                        within = Some(duration("within", &arg.value).map_err(|error| vec![error])?);
                    }
                    Expectation::Event {
                        name: name("`expect event` needs the event's type")?,
                        fields: read_fields()?,
                        within,
                    }
                }
                "trace" => {
                    if let Some(arg) = command
                        .args
                        .iter()
                        .find(|arg| arg.key.as_ref().is_some_and(|key| key.name == "within"))
                    {
                        return Err(bad(
                            arg.value.span,
                            "`within=` belongs to `expect event`; a trace record is looked for in the whole run",
                        ));
                    }
                    Expectation::Trace {
                        kind: name("`expect trace` needs the trace's kind")?,
                        fields: read_fields()?,
                    }
                }
                "reason" => {
                    no_fields("reason")?;
                    Expectation::Reason {
                        code: name("`expect reason` needs the reason's code")?,
                    }
                }
                "hash" => {
                    no_fields("hash")?;
                    match written {
                        Some(value) => {
                            let text = value.span.slice(source);
                            let hash = text.parse::<Hash128>().map_err(|_| {
                                vec![
                                    Diagnostic::new(
                                        codes::BAD_HASH,
                                        value.span,
                                        format!("`{text}` is not a state hash"),
                                    )
                                    .with_suggestion(
                                        "a state hash is 32 hexadecimal digits; leave it out and run with --bless to have it written",
                                    ),
                                ]
                            })?;
                            Expectation::Hash {
                                value: Some(hash),
                                at: span(
                                    offset + value.span.start as usize,
                                    offset + value.span.end as usize,
                                ),
                            }
                        }
                        None => {
                            let after = command
                                .args
                                .first()
                                .map_or(command.name_span.end, |arg| arg.value.span.end)
                                as usize;
                            Expectation::Hash {
                                value: None,
                                at: span(offset + after, offset + after),
                            }
                        }
                    }
                }
                "frame" => {
                    no_fields("frame")?;
                    Expectation::Frame {
                        file: name("`expect frame` needs the file the frame is stored in")?,
                    }
                }
                other => {
                    return Err(bad(
                        command.name_span,
                        &format!("`{other}` is not a kind of expectation"),
                    ));
                }
            };
            Ok(StepKind::Expect(expectation))
        }
        _ => Ok(StepKind::Command(line)),
    }
}

#[cfg(test)]
mod tests {
    use clash_expr::Value;

    use super::*;

    const HASH: &str = "0123456789abcdef0123456789abcdef";

    fn expectation(source: &str) -> Expectation {
        let script = parse_script(source).unwrap_or_else(|errors| panic!("{source}: {errors:?}"));
        assert_eq!(script.steps.len(), 1, "{source}");
        match &script.steps[0].kind {
            StepKind::Expect(expectation) => expectation.clone(),
            StepKind::Command(_) => panic!("{source} is a command"),
        }
    }

    fn error(source: &str) -> ScriptError {
        let mut errors = parse_script(source).unwrap_err();
        assert_eq!(errors.len(), 1, "{source}: {errors:?}");
        errors.remove(0)
    }

    #[test]
    fn the_script_of_the_specification_parses() {
        // 14 §7, with a whole hash.
        let text = format!(
            "# scenarios/combat/parry_in_time.scn\n\
             use profile=test_core seed=7\n\
             gen flat size=1\n\
             spawn duellist at @c:10,10,0 as=a\n\
             spawn duellist at @c:11,10,0 as=b\n\
             as @b add_trait stance_guarded\n\
             as @a act slash target=@b\n\
             advance 400ms\n\
             assert \"@b.wounds.count == 0\"\n\
             expect event strike_defended target=@b within=400ms\n\
             expect trace strike turned=162 landed=18\n\
             expect hash {HASH}\n"
        );
        let script = parse_script(&text).unwrap();
        assert_eq!(script.steps.len(), 11);
        assert_eq!(script.commands(), 7);
        let lines: Vec<usize> = script.steps.iter().map(|step| step.line).collect();
        assert_eq!(lines, (2..=12).collect::<Vec<_>>());
        assert_eq!(script.steps[0].source, "use profile=test_core seed=7");
        assert_eq!(&text[script.steps[3].offset..][..5], "spawn");
    }

    #[test]
    fn every_expectation() {
        match expectation("as @a assert @b.wounds.count == 0") {
            Expectation::Assert { condition, subject } => {
                assert_eq!(condition.source, "$at0.wounds.count == 0");
                assert_eq!(subject, Some(Reference::Named("a".into())));
            }
            other => panic!("{other:?}"),
        }
        assert_eq!(
            expectation("expect event strike_defended target=@b turned=162 within=400ms"),
            Expectation::Event {
                name: "strike_defended".into(),
                fields: vec![
                    ("target".into(), ArgValue::Ref(Reference::Named("b".into()))),
                    ("turned".into(), ArgValue::Value(Value::Int(162))),
                ],
                within: Some(Dur::from_millis(400)),
            }
        );
        assert_eq!(
            expectation("expect event core:alarm"),
            Expectation::Event {
                name: "core:alarm".into(),
                fields: vec![],
                within: None
            }
        );
        // Fields may have any names, those of the command's own
        // parameters included.
        assert_eq!(
            expectation("expect trace strike kind=cut name=a what=b"),
            Expectation::Trace {
                kind: "strike".into(),
                fields: vec![
                    ("kind".into(), ArgValue::Word("cut".into())),
                    ("name".into(), ArgValue::Word("a".into())),
                    ("what".into(), ArgValue::Word("b".into())),
                ],
            }
        );
        assert_eq!(
            expectation("expect reason no_fuel"),
            Expectation::Reason {
                code: "no_fuel".into()
            }
        );
        assert_eq!(
            expectation(&format!("  expect hash {HASH}")),
            Expectation::Hash {
                value: Some(HASH.parse().unwrap()),
                at: Span::new(14, 46),
            }
        );
        assert_eq!(
            expectation("expect hash # not yet"),
            Expectation::Hash {
                value: None,
                at: Span::new(11, 11),
            }
        );
        assert_eq!(
            expectation("expect frame frames/menu.txt"),
            Expectation::Frame {
                file: "frames/menu.txt".into()
            }
        );
        assert_eq!(
            expectation("expect frame \"my frames/menu.txt\""),
            Expectation::Frame {
                file: "my frames/menu.txt".into()
            }
        );
    }

    #[test]
    fn a_hash_of_digits_alone_is_still_a_hash() {
        let digits = "12345678901234567890123456789012";
        assert!(matches!(
            expectation(&format!("expect hash {digits}")),
            Expectation::Hash { value: Some(hash), .. } if hash.to_string() == digits
        ));
        let exponent = "123456789012345678901234567890e1";
        assert!(matches!(
            expectation(&format!("expect hash {exponent}")),
            Expectation::Hash { value: Some(hash), .. } if hash.to_string() == exponent
        ));
    }

    #[test]
    fn errors_carry_line_and_column() {
        let found = error("check\n\n  spawn \"town\nadvance 1s\n");
        assert_eq!((found.line, found.column), (3, 9));
        assert_eq!(found.diagnostic.code, codes::UNCLOSED_QUOTE);
        let rendered = found.render("scenarios/a.scn");
        assert!(
            rendered.starts_with(
                "scenarios/a.scn:3:9: error[E1402]: the quote opened here is never closed\n"
            ),
            "{rendered}"
        );
        assert!(
            rendered.contains("    spawn \"town\n          ^^^^^\n"),
            "{rendered}"
        );

        // Inside JSON that runs over lines.
        let found = error("check\ndo {\n  \"a\": 1,\n  \"b\" 2\n}\ncheck\n");
        assert_eq!(found.diagnostic.code, codes::BAD_JSON);
        assert_eq!(found.line, 4);
        // JSON that never closes is reported where it opens.
        let found = error("check\ndo {\n  \"a\": 1\ncheck\n");
        assert_eq!(found.diagnostic.code, codes::UNCLOSED_JSON);
        assert_eq!((found.line, found.column), (2, 4));

        let all = parse_script("a \"\nb )\nexpect hash zz\n").unwrap_err();
        let lines: Vec<usize> = all.iter().map(|error| error.line).collect();
        assert_eq!(lines, [1, 2, 3]);
    }

    #[test]
    fn malformed_expectations() {
        for (source, code) in [
            ("expect", codes::BAD_EXPECTATION),
            ("expect nothing x", codes::BAD_EXPECTATION),
            ("expect 12", codes::BAD_EXPECTATION),
            ("expect event a b", codes::BAD_EXPECTATION),
            ("expect trace a within=1s", codes::BAD_EXPECTATION),
            (
                "expect event a f={\"x\": 1} g=\"1 +\"",
                codes::BAD_EXPRESSION,
            ),
            ("expect event", codes::BAD_EXPECTATION),
            ("expect trace", codes::BAD_EXPECTATION),
            ("expect reason", codes::BAD_EXPECTATION),
            ("expect reason x y=1", codes::BAD_EXPECTATION),
            ("expect frame", codes::BAD_EXPECTATION),
            ("expect hash 9c41f0aa", codes::BAD_HASH),
            (
                "expect hash 0123456789abcdef0123456789abcdeg",
                codes::BAD_HASH,
            ),
            ("expect event x within=3kg", codes::WRONG_TYPE),
            ("expect event x within=soon", codes::WRONG_TYPE),
            ("assert", codes::MISSING_ARGUMENT),
            ("assert 1 +", codes::BAD_EXPRESSION),
        ] {
            assert_eq!(error(source).diagnostic.code, code, "{source}");
        }
        let found = error("expect hash 9c41f0aa");
        assert_eq!(found.column, 13);
        assert!(found.diagnostic.suggestion.unwrap().contains("--bless"));
    }

    #[test]
    fn line_endings_of_either_kind() {
        let script = parse_script("check\r\nexpect reason x\r\n\r\n# c\r\n").unwrap();
        assert_eq!(script.steps.len(), 2);
        assert_eq!(script.steps[1].line, 2);
    }
}
