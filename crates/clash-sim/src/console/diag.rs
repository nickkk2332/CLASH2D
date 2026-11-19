//! Diagnostics of the console: what is wrong with a line, where, and
//! what to do about it.
//!
//! A [`Diagnostic`] has a stable [`Code`], a message, a [`Span`] of bytes
//! of the line it is about and, where one can be offered, a suggestion.
//! The codes are the `14` group of 04 §8.2: `E140x` for the syntax of a
//! line, `E142x` for binding a line to a command, `E144x` for test
//! scripts.

use core::fmt;

pub use clash_expr::Span;

/// The stable code of a kind of console diagnostic, such as `E1421`.
/// Every code is one of the constants in [`codes`].
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub struct Code(&'static str);

impl Code {
    /// The code as text.
    pub const fn as_str(self) -> &'static str {
        self.0
    }

    /// What the code means, in one line, for the generated reference.
    pub fn explanation(self) -> &'static str {
        codes::ALL
            .iter()
            .find(|(code, _)| *code == self)
            .map_or("", |(_, text)| text)
    }
}

impl fmt::Display for Code {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.0)
    }
}

/// Every diagnostic code the console issues.
pub mod codes {
    use super::Code;

    /// A character where no value can start or continue.
    pub const UNEXPECTED_CHARACTER: Code = Code("E1401");
    /// A quote that is never closed.
    pub const UNCLOSED_QUOTE: Code = Code("E1402");
    /// A backslash before something other than `"` or `\`.
    pub const BAD_ESCAPE: Code = Code("E1403");
    /// JSON in braces whose closing brace is missing.
    pub const UNCLOSED_JSON: Code = Code("E1404");
    /// Braces that do not hold valid JSON.
    pub const BAD_JSON: Code = Code("E1405");
    /// A reference that is none of the forms of 14 §3.2.
    pub const BAD_REFERENCE: Code = Code("E1406");
    /// `key=` with no value after it.
    pub const MISSING_VALUE: Code = Code("E1407");
    /// A line that does not start with a command name.
    pub const MISSING_COMMAND: Code = Code("E1408");
    /// Two values with nothing between them.
    pub const MISSING_SEPARATOR: Code = Code("E1409");
    /// A line longer than the console accepts.
    pub const TOO_LONG: Code = Code("E1410");
    /// A bracket inside an unquoted value that is never closed.
    pub const UNCLOSED_BRACKET: Code = Code("E1411");

    /// No command has this name.
    pub const UNKNOWN_COMMAND: Code = Code("E1421");
    /// The command has no parameter with this name.
    pub const UNKNOWN_ARGUMENT: Code = Code("E1422");
    /// A required argument was not given.
    pub const MISSING_ARGUMENT: Code = Code("E1423");
    /// More positional arguments than the command has parameters.
    pub const TOO_MANY_ARGUMENTS: Code = Code("E1424");
    /// An argument given twice.
    pub const DUPLICATE_ARGUMENT: Code = Code("E1425");
    /// A value of the wrong type for its parameter.
    pub const WRONG_TYPE: Code = Code("E1426");
    /// The command exists and is not offered where it was typed.
    pub const NOT_OFFERED: Code = Code("E1427");
    /// An expression that does not parse.
    pub const BAD_EXPRESSION: Code = Code("E1428");
    /// A name given with `as=` that cannot be a name.
    pub const BAD_NAME: Code = Code("E1429");

    /// An `expect` line that is none of the forms of 14 §7.1.
    pub const BAD_EXPECTATION: Code = Code("E1441");
    /// A state hash that is not 32 hexadecimal digits.
    pub const BAD_HASH: Code = Code("E1442");

    /// Every code with its one-line explanation.
    pub const ALL: &[(Code, &str)] = &[
        (
            UNEXPECTED_CHARACTER,
            "a character appears where no value can start or continue",
        ),
        (UNCLOSED_QUOTE, "a quote is opened and never closed"),
        (
            BAD_ESCAPE,
            "inside double quotes a backslash may only come before a double quote or another backslash",
        ),
        (
            UNCLOSED_JSON,
            "JSON in braces runs to its closing brace, which is missing",
        ),
        (BAD_JSON, "what the braces hold is not valid JSON"),
        (
            BAD_REFERENCE,
            "a reference is @here, @self, @sel, @c:x,y,z, @e:id, @kind:name or a name given with as=",
        ),
        (MISSING_VALUE, "`key=` must be followed directly by a value"),
        (
            MISSING_COMMAND,
            "a line starts with a command name, after an optional `as <reference>`",
        ),
        (MISSING_SEPARATOR, "two values need a space between them"),
        (TOO_LONG, "a console line is at most 65,536 bytes"),
        (
            UNCLOSED_BRACKET,
            "a bracket opened inside an unquoted value is never closed",
        ),
        (UNKNOWN_COMMAND, "no command has this name"),
        (
            UNKNOWN_ARGUMENT,
            "the command has no parameter with this name",
        ),
        (MISSING_ARGUMENT, "a required argument was not given"),
        (
            TOO_MANY_ARGUMENTS,
            "more positional arguments were given than the command takes",
        ),
        (DUPLICATE_ARGUMENT, "an argument was given more than once"),
        (WRONG_TYPE, "a value is not of the type its parameter takes"),
        (
            NOT_OFFERED,
            "the command is not offered by the host it was typed in",
        ),
        (BAD_EXPRESSION, "an expression does not parse"),
        (
            BAD_NAME,
            "a name given with as= is letters, digits and underscores, and not a reserved reference",
        ),
        (
            BAD_EXPECTATION,
            "an expectation is `expect event`, `expect trace`, `expect reason`, `expect hash` or `expect frame`",
        ),
        (BAD_HASH, "a state hash is 32 hexadecimal digits"),
    ];
}

/// One thing wrong with a console line.
///
/// ```
/// use clash_sim::console::{codes, parse_line};
///
/// let error = parse_line("spawn \"town").unwrap_err();
/// assert_eq!(error.code, codes::UNCLOSED_QUOTE);
/// assert_eq!(error.span.start, 6);
/// assert!(error.render("spawn \"town").contains("^"));
/// ```
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Diagnostic {
    /// The stable code.
    pub code: Code,
    /// What is wrong, for a person: lower case, no full stop.
    pub message: String,
    /// Where, as bytes of the line.
    pub span: Span,
    /// What to do about it, if the console can say.
    pub suggestion: Option<String>,
}

impl Diagnostic {
    /// A diagnostic with no suggestion.
    pub fn new(code: Code, span: Span, message: impl Into<String>) -> Diagnostic {
        Diagnostic {
            code,
            message: message.into(),
            span,
            suggestion: None,
        }
    }

    /// The same diagnostic with a suggestion.
    #[must_use]
    pub fn with_suggestion(mut self, suggestion: impl Into<String>) -> Diagnostic {
        self.suggestion = Some(suggestion.into());
        self
    }

    /// The same diagnostic with a suggestion, if there is one.
    #[must_use]
    pub fn with_suggestion_opt(mut self, suggestion: Option<String>) -> Diagnostic {
        self.suggestion = suggestion;
        self
    }

    /// The same diagnostic moved `by` bytes further along: for a
    /// diagnostic about a part of a line, found by parsing the part.
    #[must_use]
    pub fn shifted(mut self, by: u32) -> Diagnostic {
        self.span = Span::new(
            self.span.start.saturating_add(by),
            self.span.end.saturating_add(by),
        );
        self
    }

    /// The diagnostic with the line it is about and a row of carets
    /// under the place:
    ///
    /// ```text
    /// error[E1402]: the quote opened here is never closed
    ///   spawn "town
    ///         ^^^^^
    ///   help: close it with `"`
    /// ```
    pub fn render(&self, source: &str) -> String {
        let mut out = format!("{self}\n");
        // Only the physical line the span starts on is shown.
        let start = char_floor(source, self.span.start as usize);
        let line_start = source[..start].rfind('\n').map_or(0, |at| at + 1);
        let line_end = source[start..]
            .find('\n')
            .map_or(source.len(), |at| start + at);
        let line = source.get(line_start..line_end).unwrap_or("");
        let before = source
            .get(line_start..start)
            .map_or(0, |text| text.chars().count());
        let end = (self.span.end as usize).clamp(start, line_end);
        let width = source
            .get(start..end)
            .map_or(1, |text| text.chars().count())
            .max(1);
        out.push_str(&format!("  {}\n", line.trim_end()));
        out.push_str(&format!("  {}{}\n", " ".repeat(before), "^".repeat(width)));
        if let Some(suggestion) = &self.suggestion {
            out.push_str(&format!("  help: {suggestion}\n"));
        }
        out
    }
}

/// `error[E1421]: message`, without a position: the caller knows the
/// line.
impl fmt::Display for Diagnostic {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "error[{}]: {}", self.code, self.message)
    }
}

/// The nearest position at or before `at` that is inside `source` and
/// not in the middle of a character, so that slicing there is safe
/// whatever a span says.
pub(crate) fn char_floor(source: &str, at: usize) -> usize {
    let mut at = at.min(source.len());
    while !source.is_char_boundary(at) {
        at -= 1;
    }
    at
}

/// "did you mean `x`?" for the candidate closest to `name`, if one is
/// close.
pub(crate) fn did_you_mean<'a>(
    name: &str,
    candidates: impl IntoIterator<Item = &'a str>,
) -> Option<String> {
    clash_expr::closest_name(name, candidates).map(|found| format!("did you mean `{found}`?"))
}

/// The span from `start` up to `end`, as byte positions of a line. A
/// line is short enough for both to fit (see
/// [`MAX_LINE_LEN`](super::MAX_LINE_LEN)).
pub(crate) fn span(start: usize, end: usize) -> Span {
    Span::new(
        u32::try_from(start).unwrap_or(u32::MAX),
        u32::try_from(end).unwrap_or(u32::MAX),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_code_is_listed_once_with_an_explanation() {
        let mut seen = std::collections::BTreeSet::new();
        for (code, text) in codes::ALL {
            assert!(seen.insert(code.as_str()), "{code} is listed twice");
            assert!(!text.is_empty());
            assert_eq!(code.explanation(), *text);
            assert!(code.as_str().starts_with("E14"));
        }
    }

    #[test]
    fn rendering_points_at_the_place() {
        let diagnostic = Diagnostic::new(codes::UNKNOWN_COMMAND, Span::new(2, 5), "no such thing")
            .with_suggestion("try `help`");
        let text = diagnostic.render("  abc def");
        assert_eq!(
            text,
            "error[E1421]: no such thing\n    abc def\n    ^^^\n  help: try `help`\n"
        );
    }

    #[test]
    fn rendering_survives_a_span_that_is_not_on_a_character() {
        let source = "x \u{e9}\u{e9}";
        for start in 0..8 {
            for end in start..8 {
                let diagnostic = Diagnostic::new(codes::BAD_JSON, Span::new(start, end), "bad");
                assert!(diagnostic.render(source).contains("^"));
            }
        }
        assert_eq!(char_floor(source, 3), 2);
        assert_eq!(char_floor(source, 99), source.len());
    }

    #[test]
    fn rendering_shows_the_line_of_a_span_in_later_lines() {
        let diagnostic = Diagnostic::new(codes::BAD_JSON, Span::new(8, 9), "bad");
        let text = diagnostic.render("do {\n  \"a\" 1 }");
        assert!(text.contains("  \"a\" 1 }\n"), "{text}");
        assert!(text.contains("\n     ^\n"), "{text}");
    }
}
