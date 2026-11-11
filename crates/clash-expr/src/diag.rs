//! Diagnostics: what the compiler says about an expression.
//!
//! A [`Diagnostic`] has a stable [`Code`], a [`Severity`], a message, a
//! [`Span`] of bytes inside the expression's source string and, where one
//! can be offered, a suggestion. This crate knows nothing about files: the
//! data layer turns the span into a file, line and column (04 §8.2), and
//! [`offset_in_json_string`] does the part of that which depends on how
//! the string was written in JSON.

use core::fmt;
use core::ops::Range;

/// A range of bytes in an expression's source string.
///
/// ```
/// use clash_expr::Span;
///
/// let span = Span::new(4, 9);
/// assert_eq!(span.len(), 5);
/// assert_eq!(span.slice("the quick fox"), "quick");
/// assert_eq!(Span::new(0, 3).to(span), Span::new(0, 9));
/// ```
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug, Default)]
pub struct Span {
    /// The first byte.
    pub start: u32,
    /// One past the last byte.
    pub end: u32,
}

impl Span {
    /// The span from `start` up to but not including `end`.
    pub const fn new(start: u32, end: u32) -> Span {
        Span { start, end }
    }

    /// An empty span at one position.
    pub const fn at(position: u32) -> Span {
        Span {
            start: position,
            end: position,
        }
    }

    /// The number of bytes covered.
    pub const fn len(self) -> u32 {
        self.end.saturating_sub(self.start)
    }

    /// Whether the span covers nothing.
    pub const fn is_empty(self) -> bool {
        self.end <= self.start
    }

    /// The smallest span covering both this one and `other`.
    pub fn to(self, other: Span) -> Span {
        Span {
            start: self.start.min(other.start),
            end: self.end.max(other.end),
        }
    }

    /// The span as a range of byte positions.
    pub const fn range(self) -> Range<usize> {
        self.start as usize..self.end as usize
    }

    /// The text the span covers, or the empty text if the span does not
    /// lie inside `source` on character boundaries.
    pub fn slice(self, source: &str) -> &str {
        source.get(self.range()).unwrap_or("")
    }
}

/// How serious a diagnostic is (04 §8.2).
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub enum Severity {
    /// The expression cannot be used.
    Error,
    /// The expression compiles, and is probably not what was meant.
    Warning,
    /// Additional information.
    Note,
}

impl fmt::Display for Severity {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Severity::Error => "error",
            Severity::Warning => "warning",
            Severity::Note => "note",
        })
    }
}

/// The stable code of a kind of diagnostic or run-time warning, such as
/// `X206`. Every code is one of the constants in [`codes`].
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub struct Code(pub(crate) &'static str);

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

/// Every diagnostic code this crate issues.
///
/// `X1..` are syntax errors, `X2..` name errors, `X3..` type errors,
/// `X4..` context errors and `X9..` run-time warnings.
pub mod codes {
    use super::Code;

    /// A character that cannot start any token.
    pub const UNEXPECTED_CHARACTER: Code = Code("X101");
    /// A string with no closing quote.
    pub const UNTERMINATED_STRING: Code = Code("X102");
    /// A number that is malformed or too large.
    pub const BAD_NUMBER: Code = Code("X103");
    /// A unit after a number that is not one of the simple units.
    pub const UNKNOWN_UNIT: Code = Code("X104");
    /// A token where the grammar expects something else.
    pub const UNEXPECTED_TOKEN: Code = Code("X105");
    /// The expression ends where more is needed.
    pub const UNEXPECTED_END: Code = Code("X106");
    /// Two comparisons in a row, such as `a < b < c`.
    pub const CHAINED_COMPARISON: Code = Code("X107");
    /// A brace in a text template that is not closed or not opened.
    pub const UNBALANCED_BRACE: Code = Code("X108");
    /// A format after the colon in a text template that is not `.N`.
    pub const BAD_FORMAT: Code = Code("X109");
    /// An expression that is too long or nested too deeply.
    pub const TOO_DEEP: Code = Code("X110");
    /// An expression with nothing in it.
    pub const EMPTY_EXPRESSION: Code = Code("X111");

    /// A name that is not a subject bound in this context.
    pub const UNKNOWN_NAME: Code = Code("X201");
    /// A `$name` that is not a local of this context.
    pub const UNKNOWN_LOCAL: Code = Code("X202");
    /// A property the subject's kind does not have.
    pub const UNKNOWN_PROPERTY: Code = Code("X203");
    /// A function that is not registered.
    pub const UNKNOWN_FUNCTION: Code = Code("X204");
    /// `event.field` where the context has no event or the event has no
    /// such field.
    pub const UNKNOWN_EVENT_FIELD: Code = Code("X205");
    /// A text compared with or given as an id that names no definition.
    pub const UNKNOWN_ID: Code = Code("X206");
    /// A variable that no definition declares.
    pub const UNDECLARED_VARIABLE: Code = Code("X207");
    /// A call of something that is neither a function nor a property.
    pub const NOT_CALLABLE: Code = Code("X208");
    /// A variable read on a kind of subject other than the one it is
    /// declared on.
    pub const VARIABLE_SCOPE: Code = Code("X209");
    /// A bare name found in more than one imported namespace.
    pub const AMBIGUOUS_NAME: Code = Code("X210");
    /// The target of an assignment that is not a local, an event field,
    /// a variable or a settable property.
    pub const NOT_ASSIGNABLE: Code = Code("X211");

    /// Operands whose types the operator does not accept.
    pub const OPERAND_TYPE: Code = Code("X301");
    /// The wrong number of arguments.
    pub const ARGUMENT_COUNT: Code = Code("X302");
    /// An argument of the wrong type.
    pub const ARGUMENT_TYPE: Code = Code("X303");
    /// A value that may be null used in arithmetic.
    pub const NULL_IN_ARITHMETIC: Code = Code("X304");
    /// The expression's type is not the one the field wants.
    pub const RESULT_TYPE: Code = Code("X305");
    /// A comparison between types that can never be equal or ordered.
    pub const UNRELATED_COMPARISON: Code = Code("X306");
    /// An index into something that is not a list or a map.
    pub const NOT_INDEXABLE: Code = Code("X307");
    /// A property read on a value that is not a subject or a map.
    pub const NO_PROPERTIES: Code = Code("X308");
    /// An operation on constants that fails every time, such as `1 / 0`.
    pub const ALWAYS_FAILS: Code = Code("X309");

    /// A random function where the context must be pure.
    pub const RANDOM_IN_PURE: Code = Code("X401");
    /// A read that is not safe for belief, in a mind context.
    pub const NOT_BELIEF_SAFE: Code = Code("X402");

    /// Run time: arithmetic met a null.
    pub const RUN_NULL_ARITHMETIC: Code = Code("X901");
    /// Run time: division by zero.
    pub const RUN_DIVISION_BY_ZERO: Code = Code("X902");
    /// Run time: a function was given a number outside its domain.
    pub const RUN_DOMAIN: Code = Code("X903");
    /// Run time: integer arithmetic overflowed.
    pub const RUN_OVERFLOW: Code = Code("X904");
    /// Run time: a value had a type the operation does not accept.
    pub const RUN_TYPE: Code = Code("X905");
    /// Run time: a list, a map or a text grew past the size limit.
    pub const RUN_TOO_LARGE: Code = Code("X906");
    /// Run time: a random function was called with no stream to draw
    /// from.
    pub const RUN_NO_RANDOM: Code = Code("X907");

    /// Every code with its one-line explanation, in order of code.
    pub const ALL: &[(Code, &str)] = &[
        (
            UNEXPECTED_CHARACTER,
            "a character that cannot start any token",
        ),
        (UNTERMINATED_STRING, "a string with no closing quote"),
        (BAD_NUMBER, "a number that is malformed or too large"),
        (
            UNKNOWN_UNIT,
            "a unit after a number that is not one of the simple units",
        ),
        (
            UNEXPECTED_TOKEN,
            "a token where the grammar expects something else",
        ),
        (UNEXPECTED_END, "the expression ends where more is needed"),
        (
            CHAINED_COMPARISON,
            "two comparisons in a row; comparisons do not chain",
        ),
        (
            UNBALANCED_BRACE,
            "a brace in a text template that is not closed or not opened",
        ),
        (
            BAD_FORMAT,
            "a number format in a text template that is not `.N`",
        ),
        (
            TOO_DEEP,
            "an expression that is too long or nested too deeply",
        ),
        (EMPTY_EXPRESSION, "an expression with nothing in it"),
        (
            UNKNOWN_NAME,
            "a name that is not a subject bound in this context",
        ),
        (
            UNKNOWN_LOCAL,
            "a `$name` that is not a local of this context",
        ),
        (
            UNKNOWN_PROPERTY,
            "a property the subject's kind does not have",
        ),
        (UNKNOWN_FUNCTION, "a function that is not registered"),
        (
            UNKNOWN_EVENT_FIELD,
            "an event field where there is no event or no such field",
        ),
        (
            UNKNOWN_ID,
            "a text used as an id that names no definition of that type",
        ),
        (
            UNDECLARED_VARIABLE,
            "a variable that no definition declares",
        ),
        (
            NOT_CALLABLE,
            "a call of something that is neither a function nor a property",
        ),
        (
            VARIABLE_SCOPE,
            "a variable read on a kind of subject it is not declared on",
        ),
        (
            AMBIGUOUS_NAME,
            "a bare name found in more than one imported namespace",
        ),
        (
            NOT_ASSIGNABLE,
            "the target of an assignment that cannot be assigned to",
        ),
        (
            OPERAND_TYPE,
            "operands whose types the operator does not accept",
        ),
        (ARGUMENT_COUNT, "the wrong number of arguments"),
        (ARGUMENT_TYPE, "an argument of the wrong type"),
        (
            NULL_IN_ARITHMETIC,
            "a value that may be null used in arithmetic",
        ),
        (
            RESULT_TYPE,
            "the expression's type is not the one the field wants",
        ),
        (
            UNRELATED_COMPARISON,
            "a comparison between types that can never be equal or ordered",
        ),
        (
            NOT_INDEXABLE,
            "an index into something that is not a list or a map",
        ),
        (
            NO_PROPERTIES,
            "a property read on a value that is not a subject or a map",
        ),
        (
            ALWAYS_FAILS,
            "an operation on constants that fails every time",
        ),
        (
            RANDOM_IN_PURE,
            "a random function where the context must be pure",
        ),
        (
            NOT_BELIEF_SAFE,
            "a read that is not safe for belief, in a mind context",
        ),
        (RUN_NULL_ARITHMETIC, "run time: arithmetic met a null"),
        (RUN_DIVISION_BY_ZERO, "run time: division by zero"),
        (
            RUN_DOMAIN,
            "run time: a function was given a number outside its domain",
        ),
        (RUN_OVERFLOW, "run time: integer arithmetic overflowed"),
        (
            RUN_TYPE,
            "run time: a value had a type the operation does not accept",
        ),
        (
            RUN_TOO_LARGE,
            "run time: a list, a map or a text grew past the size limit",
        ),
        (
            RUN_NO_RANDOM,
            "run time: a random function was called with no stream",
        ),
    ];
}

/// One thing the compiler has to say about an expression.
///
/// ```
/// use clash_expr::{Severity, parse_expr};
///
/// let error = parse_expr("1 + ").unwrap_err();
/// assert_eq!(error.severity, Severity::Error);
/// assert_eq!(error.code.as_str(), "X106");
/// assert_eq!(error.span.start, 4);
/// ```
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Diagnostic {
    /// The stable code.
    pub code: Code,
    /// How serious it is.
    pub severity: Severity,
    /// What is wrong, for a person: lower case, no full stop.
    pub message: String,
    /// Where, as bytes of the expression's source string.
    pub span: Span,
    /// What to do about it, if the compiler can say.
    pub suggestion: Option<String>,
}

impl Diagnostic {
    /// An error.
    pub fn error(code: Code, span: Span, message: impl Into<String>) -> Diagnostic {
        Diagnostic {
            code,
            severity: Severity::Error,
            message: message.into(),
            span,
            suggestion: None,
        }
    }

    /// A warning.
    pub fn warning(code: Code, span: Span, message: impl Into<String>) -> Diagnostic {
        Diagnostic {
            code,
            severity: Severity::Warning,
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

    /// Whether this is an error.
    pub fn is_error(&self) -> bool {
        self.severity == Severity::Error
    }
}

/// `error[X105]: message (suggestion)`, without a position: the caller
/// knows the file.
impl fmt::Display for Diagnostic {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}[{}]: {}", self.severity, self.code, self.message)?;
        if let Some(suggestion) = &self.suggestion {
            write!(f, " ({suggestion})")?;
        }
        Ok(())
    }
}

/// Where a byte of a decoded JSON string sits in the string as it was
/// written in the file.
///
/// `raw` is the string literal as written, starting at its opening quote.
/// `offset` is a byte position in the decoded text. The result is the byte
/// position in `raw` of the character that produced that byte: one more
/// than `offset` when nothing before it is escaped, further along when
/// escapes such as `\"` or `\u00e9` come first. An offset at or past the
/// end of the text gives the position of the closing quote.
///
/// This is what lets a syntax error inside an expression point at the
/// right column of the file.
///
/// ```
/// use clash_expr::offset_in_json_string;
///
/// // Written: "say \"hi\" + 1"   Decoded: say "hi" + 1
/// let raw = r#""say \"hi\" + 1""#;
/// assert_eq!(offset_in_json_string(raw, 0), 1);
/// assert_eq!(offset_in_json_string(raw, 9), 12);
/// ```
pub fn offset_in_json_string(raw: &str, offset: usize) -> usize {
    let bytes = raw.as_bytes();
    let mut at = usize::from(bytes.first() == Some(&b'"'));
    let mut decoded = 0_usize;
    while at < bytes.len() && bytes[at] != b'"' {
        let (raw_len, decoded_len) = if bytes[at] == b'\\' {
            json_escape_lengths(bytes, at)
        } else {
            let len = utf8_len(bytes[at]);
            (len, len)
        };
        if offset < decoded + decoded_len {
            return at;
        }
        decoded += decoded_len;
        at += raw_len;
    }
    at.min(bytes.len())
}

/// The length in the file and in the decoded text of the escape that
/// starts at `at`.
fn json_escape_lengths(bytes: &[u8], at: usize) -> (usize, usize) {
    if bytes.get(at + 1) != Some(&b'u') {
        return (2, 1);
    }
    let unit = hex4(bytes, at + 2);
    match unit {
        // A high surrogate followed by a low one is a single character of
        // four bytes.
        Some(0xD800..=0xDBFF)
            if bytes.get(at + 6) == Some(&b'\\')
                && bytes.get(at + 7) == Some(&b'u')
                && matches!(hex4(bytes, at + 8), Some(0xDC00..=0xDFFF)) =>
        {
            (12, 4)
        }
        Some(0x0000..=0x007F) => (6, 1),
        Some(0x0080..=0x07FF) => (6, 2),
        _ => (6, 3),
    }
}

/// Four hexadecimal digits starting at `at`, as a number.
fn hex4(bytes: &[u8], at: usize) -> Option<u32> {
    let digits = bytes.get(at..at + 4)?;
    let text = core::str::from_utf8(digits).ok()?;
    u32::from_str_radix(text, 16).ok()
}

/// The length of the UTF-8 character whose first byte is `first`.
const fn utf8_len(first: u8) -> usize {
    match first {
        0xF0..=0xFF => 4,
        0xE0..=0xEF => 3,
        0xC0..=0xDF => 2,
        _ => 1,
    }
}

/// The closest of `candidates` to `name`, if any is close enough to be
/// worth suggesting.
///
/// Closeness is edit distance: one for each character added, removed or
/// replaced, or pair of neighbours swapped. A candidate is offered when
/// its distance is no more than a third of the longer length, and at least
/// one edit is always allowed. Ties go to the candidate that comes first
/// in text order, so the answer does not depend on the order given.
///
/// ```
/// use clash_expr::closest_name;
///
/// let ids = ["core:glass", "core:grass", "core:iron"];
/// assert_eq!(closest_name("core:glas", ids), Some("core:glass"));
/// assert_eq!(closest_name("core:water", ids), None);
/// ```
pub fn closest_name<'a>(
    name: &str,
    candidates: impl IntoIterator<Item = &'a str>,
) -> Option<&'a str> {
    let mut best: Option<(usize, &'a str)> = None;
    for candidate in candidates {
        let longer = name.chars().count().max(candidate.chars().count());
        let allowed = (longer / 3).max(1);
        let Some(distance) = edit_distance(name, candidate, allowed) else {
            continue;
        };
        let better = match best {
            None => true,
            Some((best_distance, best_name)) => {
                distance < best_distance || (distance == best_distance && candidate < best_name)
            }
        };
        if better {
            best = Some((distance, candidate));
        }
    }
    best.map(|(_, candidate)| candidate)
}

/// The edit distance between two texts, or `None` if it is more than
/// `limit`.
fn edit_distance(a: &str, b: &str, limit: usize) -> Option<usize> {
    let a: Vec<char> = a.chars().collect();
    let b: Vec<char> = b.chars().collect();
    if a.len().abs_diff(b.len()) > limit {
        return None;
    }
    // Rows of the classic table, with one more row kept for swaps.
    let mut before: Vec<usize> = vec![0; b.len() + 1];
    let mut previous: Vec<usize> = (0..=b.len()).collect();
    let mut current: Vec<usize> = vec![0; b.len() + 1];
    for i in 1..=a.len() {
        current[0] = i;
        for j in 1..=b.len() {
            let replace = previous[j - 1] + usize::from(a[i - 1] != b[j - 1]);
            let mut cost = replace.min(previous[j] + 1).min(current[j - 1] + 1);
            if i > 1 && j > 1 && a[i - 1] == b[j - 2] && a[i - 2] == b[j - 1] {
                cost = cost.min(before[j - 2] + 1);
            }
            current[j] = cost;
        }
        core::mem::swap(&mut before, &mut previous);
        core::mem::swap(&mut previous, &mut current);
    }
    let distance = previous[b.len()];
    (distance <= limit).then_some(distance)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spans_join_and_slice() {
        let a = Span::new(2, 5);
        let b = Span::new(7, 9);
        assert_eq!(a.to(b), Span::new(2, 9));
        assert_eq!(b.to(a), Span::new(2, 9));
        assert_eq!(a.slice("0123456789"), "234");
        assert_eq!(Span::new(8, 20).slice("short"), "");
        assert!(Span::at(3).is_empty());
        assert_eq!(Span::at(3).len(), 0);
        assert_eq!(a.range(), 2..5);
    }

    #[test]
    fn every_code_is_listed_once_in_order_with_an_explanation() {
        let listed: Vec<&str> = codes::ALL.iter().map(|(code, _)| code.as_str()).collect();
        let mut sorted = listed.clone();
        sorted.sort_unstable();
        sorted.dedup();
        assert_eq!(listed, sorted);
        for (code, text) in codes::ALL {
            assert!(!text.is_empty());
            assert_eq!(code.explanation(), *text);
        }
        assert_eq!(Code("X000").explanation(), "");
    }

    #[test]
    fn a_diagnostic_displays_its_code_and_suggestion() {
        let plain = Diagnostic::error(codes::UNKNOWN_NAME, Span::at(0), "unknown name `slef`");
        assert_eq!(plain.to_string(), "error[X201]: unknown name `slef`");
        let hinted = plain.clone().with_suggestion("did you mean `self`?");
        assert_eq!(
            hinted.to_string(),
            "error[X201]: unknown name `slef` (did you mean `self`?)"
        );
        assert!(hinted.is_error());
        let warning = Diagnostic::warning(codes::NULL_IN_ARITHMETIC, Span::at(0), "may be null");
        assert!(!warning.is_error());
        assert_eq!(warning.to_string(), "warning[X304]: may be null");
        assert_eq!(plain.with_suggestion_opt(None).suggestion, None);
    }

    #[test]
    fn json_offsets_skip_escapes() {
        // No escapes: one past the opening quote.
        assert_eq!(offset_in_json_string(r#""a + b""#, 0), 1);
        assert_eq!(offset_in_json_string(r#""a + b""#, 4), 5);
        // Past the end: the closing quote.
        assert_eq!(offset_in_json_string(r#""a + b""#, 5), 6);
        assert_eq!(offset_in_json_string(r#""a + b""#, 99), 6);
        // A two-byte escape is one decoded byte.
        let raw = r#""x\ty""#;
        assert_eq!(offset_in_json_string(raw, 1), 2);
        assert_eq!(offset_in_json_string(raw, 2), 4);
        // An escape for a two-byte character is two decoded bytes; a pair
        // of surrogates is four.
        let raw = r#""\u00e9z""#;
        assert_eq!(offset_in_json_string(raw, 0), 1);
        assert_eq!(offset_in_json_string(raw, 1), 1);
        assert_eq!(offset_in_json_string(raw, 2), 7);
        let raw = r#""\ud83d\ude00z""#;
        assert_eq!(offset_in_json_string(raw, 3), 1);
        assert_eq!(offset_in_json_string(raw, 4), 13);
        // An escape for an ASCII character is one byte; for a character of
        // the wider plane, three.
        assert_eq!(offset_in_json_string(r#""\u0041z""#, 1), 7);
        assert_eq!(offset_in_json_string(r#""\u20acz""#, 3), 7);
        // Characters written directly keep their own length.
        assert_eq!(offset_in_json_string("\"é+\"", 2), 3);
        // Nothing to walk.
        assert_eq!(offset_in_json_string("", 0), 0);
    }

    #[test]
    fn closest_name_prefers_fewer_edits_then_text_order() {
        let names = ["glass", "grass", "brass", "iron"];
        assert_eq!(closest_name("glas", names), Some("glass"));
        assert_eq!(closest_name("gass", names), Some("glass"));
        assert_eq!(closest_name("rgass", names), Some("grass"));
        assert_eq!(closest_name("xrass", names), Some("brass"));
        assert_eq!(closest_name("zzzzz", names), None);
        assert_eq!(closest_name("irno", names), Some("iron"));
        assert_eq!(closest_name("x", ["y"]), Some("y"));
        assert_eq!(closest_name("abc", core::iter::empty()), None);
        // The same answer whatever order the candidates come in.
        assert_eq!(closest_name("xrass", ["grass", "brass"]), Some("brass"));
        assert_eq!(closest_name("xrass", ["brass", "grass"]), Some("brass"));
    }
}
