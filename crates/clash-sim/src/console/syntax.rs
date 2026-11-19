//! The console language (14 §3.2): a line of text into a syntax tree
//! with byte spans, and the tree back into text in canonical form.
//!
//! ```text
//! line     := ( 'as' reference )? command? comment?
//! command  := word argument*
//! argument := ( key '=' )? value
//! value    := number unit? | word | reference | '"' text '"' | json | raw
//! comment  := '#' anything
//! ```
//!
//! # Lexical rules
//!
//! - Arguments are separated by spaces or tabs. A `#` where an argument
//!   could start begins a comment that runs to the end of the line.
//! - `key=value` is a named argument when `key` is letters, digits and
//!   underscores, the `=` follows it directly and is not the start of
//!   `==`.
//! - A **number** is an optional sign, digits, an optional fraction and
//!   an optional exponent, followed directly by an optional unit of
//!   05 §4.7: `10`, `-3`, `0.2`, `1e9`, `400ms`, `5min`. With a fraction
//!   or an exponent it is a real, otherwise an integer.
//! - A **word** is an id or a keyword: `town_guard`, `core:glass`, `on`.
//! - A **reference** begins with `@`: `@here`, `@self`, `@sel`,
//!   `@c:x,y,z`, `@e:17` (or `@e:17v2` with a generation), `@kind:name`
//!   such as `@site:old_mill`, and `@name` for a name given with `as=`.
//! - A **quoted** value is between double quotes and stays on one line.
//!   Inside, `\"` is a double quote and `\\` a backslash; there are no
//!   other escapes. What it holds is an expression or a text, as the
//!   parameter it is given to decides.
//! - **JSON** begins with `{` and runs to the matching `}`, whatever is
//!   between them, line breaks included.
//! - Anything else is **raw** text, taken as written: an unquoted
//!   expression (`self.pool.blood.frac`, `dist(self, @sel)`), a file
//!   name, a hash. It ends at the first space outside round, square or
//!   curly brackets and outside the expression language's single-quoted
//!   strings, so `dist(self, @sel)` is one value.
//!
//! # Canonical form
//!
//! [`print_line`] writes a tree with one space between arguments,
//! numbers in their shortest form, JSON compact and comments as
//! `# text`. Parsing what it wrote gives the same tree.

use core::fmt;

use clash_core::Real;
use clash_expr::Unit;

use super::diag::{Diagnostic, Span, codes, span};

/// The longest line the console accepts, in bytes.
pub const MAX_LINE_LEN: usize = 65_536;

/// The words a reference may not be named, because they are references
/// already.
pub const RESERVED_NAMES: [&str; 5] = ["here", "self", "sel", "c", "e"];

/// A parsed console line.
///
/// ```
/// use clash_sim::console::{Reference, ValueKind, parse_line, print_line};
///
/// let line = parse_line("as @g1 act slash target=@self  effort=0.2 # go").unwrap();
/// assert_eq!(line.subject.as_ref().unwrap().reference, Reference::Named("g1".into()));
/// let command = line.command.as_ref().unwrap();
/// assert_eq!(command.name, "act");
/// assert_eq!(command.args[1].key.as_ref().unwrap().name, "target");
/// assert_eq!(command.args[1].value.kind, ValueKind::Ref(Reference::Active));
/// assert_eq!(print_line(&line), "as @g1 act slash target=@self effort=0.2 # go");
/// ```
#[derive(Clone, PartialEq, Debug, Default)]
pub struct Line {
    /// What `self` is bound to for this command: the `as <reference>`
    /// prefix.
    pub subject: Option<RefAt>,
    /// The command, unless the line is blank or only a comment.
    pub command: Option<Command>,
    /// The comment, if the line has one.
    pub comment: Option<Comment>,
}

impl Line {
    /// Whether the line has no command: it is blank or only a comment.
    pub const fn is_blank(&self) -> bool {
        self.command.is_none()
    }

    /// The same tree with every span set to nothing, for comparing two
    /// lines that were written with different spacing.
    #[must_use]
    pub fn without_spans(&self) -> Line {
        let mut line = self.clone();
        if let Some(subject) = &mut line.subject {
            subject.span = Span::default();
        }
        if let Some(comment) = &mut line.comment {
            comment.span = Span::default();
        }
        if let Some(command) = &mut line.command {
            command.name_span = Span::default();
            for arg in &mut command.args {
                if let Some(key) = &mut arg.key {
                    key.span = Span::default();
                }
                arg.value.span = Span::default();
            }
        }
        line
    }
}

/// A reference and where it was written.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct RefAt {
    /// The reference.
    pub reference: Reference,
    /// Where it is in the line.
    pub span: Span,
}

/// A comment: what follows `#`, without the spaces around it.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Comment {
    /// The text.
    pub text: String,
    /// Where it is in the line, from the `#`.
    pub span: Span,
}

/// A command name and its arguments.
#[derive(Clone, PartialEq, Debug)]
pub struct Command {
    /// The first word of the command. A command with a name of two
    /// words, such as `map export`, has its second word as the first
    /// argument; the command table joins them.
    pub name: String,
    /// Where the name is in the line.
    pub name_span: Span,
    /// The arguments, in the order written.
    pub args: Vec<Arg>,
}

/// One argument: positional, or named with `key=`.
#[derive(Clone, PartialEq, Debug)]
pub struct Arg {
    /// The key, for a named argument.
    pub key: Option<Key>,
    /// The value.
    pub value: Value,
}

/// The key of a named argument.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Key {
    /// The name before the `=`.
    pub name: String,
    /// Where it is in the line.
    pub span: Span,
}

/// A value and where it was written.
#[derive(Clone, PartialEq, Debug)]
pub struct Value {
    /// What it is.
    pub kind: ValueKind,
    /// Where it is in the line.
    pub span: Span,
}

/// The forms a value takes (14 §3.2).
#[derive(Clone, PartialEq, Debug)]
pub enum ValueKind {
    /// A number, with a unit if one was written: `10`, `400ms`.
    Number(Number),
    /// An id or a keyword: `town_guard`, `core:glass`, `on`.
    Word(String),
    /// A reference: `@here`.
    Ref(Reference),
    /// What was between double quotes, with the escapes undone.
    Quoted(String),
    /// JSON in braces.
    Json(serde_json::Value),
    /// Unquoted text that is none of the others, as written.
    Raw(String),
}

/// A number as written: the numeral and the unit after it. The unit has
/// not been applied.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Number {
    /// The numeral.
    pub value: Numeral,
    /// The unit, if one follows.
    pub unit: Option<Unit>,
}

impl Number {
    /// The number in SI units, as a real.
    pub fn si(self) -> Real {
        let plain = match self.value {
            Numeral::Int(value) => Real::from_i64(value),
            Numeral::Real(value) => value,
        };
        self.unit.map_or(plain, |unit| unit.apply(plain))
    }
}

/// A numeral: an integer, or a real when it has a fraction or an
/// exponent.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Numeral {
    /// A whole number.
    Int(i64),
    /// A real number. Always finite.
    Real(Real),
}

/// What a `@` names (14 §3.2).
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub enum Reference {
    /// `@here`: the cursor's cell.
    Here,
    /// `@self`: the active character.
    Active,
    /// `@sel`: the last selection.
    Selection,
    /// `@c:x,y,z`: a cell.
    Cell {
        /// East.
        x: i32,
        /// South.
        y: i32,
        /// The storey.
        z: i32,
    },
    /// `@e:17` or `@e:17v2`: an entity by its slot and, if given, its
    /// generation.
    Entity {
        /// The slot.
        slot: u32,
        /// The generation, if it was written.
        generation: Option<u32>,
    },
    /// `@kind:name`: something of a kind the host knows, by name, such
    /// as `@site:old_mill`.
    Of {
        /// The kind: `site`.
        kind: String,
        /// The name, which may carry a namespace: `core:old_mill`.
        name: String,
    },
    /// `@name`: a name given with `as=`.
    Named(String),
}

impl fmt::Display for Reference {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Reference::Here => f.write_str("@here"),
            Reference::Active => f.write_str("@self"),
            Reference::Selection => f.write_str("@sel"),
            Reference::Cell { x, y, z } => write!(f, "@c:{x},{y},{z}"),
            Reference::Entity {
                slot,
                generation: None,
            } => write!(f, "@e:{slot}"),
            Reference::Entity {
                slot,
                generation: Some(generation),
            } => write!(f, "@e:{slot}v{generation}"),
            Reference::Of { kind, name } => write!(f, "@{kind}:{name}"),
            Reference::Named(name) => write!(f, "@{name}"),
        }
    }
}

const fn is_space(byte: u8) -> bool {
    matches!(byte, b' ' | b'\t' | b'\n' | b'\r')
}

const fn is_ident_start(byte: u8) -> bool {
    byte.is_ascii_alphabetic() || byte == b'_'
}

const fn is_ident_char(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || byte == b'_'
}

/// Where the identifier starting at `start` ends. `start` itself if no
/// identifier starts there.
fn ident_end(bytes: &[u8], start: usize) -> usize {
    if !bytes.get(start).copied().is_some_and(is_ident_start) {
        return start;
    }
    let mut end = start + 1;
    while bytes.get(end).copied().is_some_and(is_ident_char) {
        end += 1;
    }
    end
}

/// Where the run of identifier characters (digits included from the
/// first) starting at `start` ends.
fn name_end(bytes: &[u8], start: usize) -> usize {
    let mut end = start;
    while bytes.get(end).copied().is_some_and(is_ident_char) {
        end += 1;
    }
    end
}

/// The text, if all of it is one identifier: letters, digits and
/// underscores, not starting with a digit.
pub(crate) fn ident_only(text: &str) -> Option<&str> {
    let end = ident_end(text.as_bytes(), 0);
    (end > 0 && end == text.len()).then_some(text)
}

/// Whether a text is a word: `ident` or `ident:ident`.
pub fn is_word(text: &str) -> bool {
    let bytes = text.as_bytes();
    let first = ident_end(bytes, 0);
    if first == 0 {
        return false;
    }
    if first == bytes.len() {
        return true;
    }
    if bytes[first] != b':' {
        return false;
    }
    let second = ident_end(bytes, first + 1);
    second > first + 1 && second == bytes.len()
}

/// Whether a text can be the name of a reference given with `as=`: an
/// identifier that is not one of [`RESERVED_NAMES`].
pub fn is_reference_name(text: &str) -> bool {
    ident_only(text).is_some() && !RESERVED_NAMES.contains(&text)
}

/// Reads a text as a number with an optional unit, if all of it is one.
pub fn parse_number(text: &str) -> Option<Number> {
    let bytes = text.as_bytes();
    let digits = |mut at: usize| {
        while bytes.get(at).is_some_and(u8::is_ascii_digit) {
            at += 1;
        }
        at
    };
    let mut at = usize::from(matches!(bytes.first(), Some(b'+' | b'-')));
    let whole = digits(at);
    if whole == at {
        return None;
    }
    at = whole;
    let mut real = false;
    if bytes.get(at) == Some(&b'.') {
        let fraction = digits(at + 1);
        if fraction > at + 1 {
            real = true;
            at = fraction;
        }
    }
    if matches!(bytes.get(at), Some(b'e' | b'E')) {
        let sign = at + 1 + usize::from(matches!(bytes.get(at + 1), Some(b'+' | b'-')));
        let exponent = digits(sign);
        if exponent > sign {
            real = true;
            at = exponent;
        }
    }
    let unit = match &text[at..] {
        "" => None,
        symbol => Some(Unit::from_symbol(symbol)?),
    };
    let numeral = &text[..at];
    let value = if real {
        let value: f64 = numeral.parse().ok()?;
        if !value.is_finite() {
            return None;
        }
        Numeral::Real(Real::new(value).ok()?)
    } else {
        Numeral::Int(numeral.parse().ok()?)
    };
    Some(Number { value, unit })
}

/// The forms a reference takes, for messages.
const REFERENCE_FORMS: &str =
    "a reference is @here, @self, @sel, @c:x,y,z, @e:id, @kind:name or a name given with as=";

/// Reads the reference at the start of `text`, which begins with `@`.
/// Gives the reference and how many bytes it takes. Spans in an error
/// count from the start of `text`.
pub(crate) fn parse_reference(text: &str) -> Result<(Reference, usize), Diagnostic> {
    let bytes = text.as_bytes();
    let bad = |end: usize, message: &str| {
        Diagnostic::new(codes::BAD_REFERENCE, span(0, end.max(1)), message)
            .with_suggestion(REFERENCE_FORMS)
    };
    let name_stop = ident_end(bytes, 1);
    if name_stop == 1 {
        return Err(bad(1, "`@` must be followed by a name"));
    }
    let name = &text[1..name_stop];
    if bytes.get(name_stop) != Some(&b':') {
        let reference = match name {
            "here" => Reference::Here,
            "self" => Reference::Active,
            "sel" => Reference::Selection,
            other => Reference::Named(other.to_owned()),
        };
        return Ok((reference, name_stop));
    }
    let body = name_stop + 1;
    match name {
        "c" => {
            let mut at = body;
            let mut parts = [0_i32; 3];
            for (index, part) in parts.iter_mut().enumerate() {
                if index > 0 {
                    if bytes.get(at) != Some(&b',') {
                        return Err(bad(at, "a cell reference is `@c:x,y,z`"));
                    }
                    at += 1;
                }
                let start = at;
                at += usize::from(bytes.get(at) == Some(&b'-'));
                while bytes.get(at).is_some_and(u8::is_ascii_digit) {
                    at += 1;
                }
                *part = text[start..at].parse().map_err(|_| {
                    bad(
                        at,
                        "a cell reference is `@c:x,y,z` with whole numbers that fit 32 bits",
                    )
                })?;
            }
            let [x, y, z] = parts;
            Ok((Reference::Cell { x, y, z }, at))
        }
        "e" => {
            let number = |start: usize| {
                let mut at = start;
                while bytes.get(at).is_some_and(u8::is_ascii_digit) {
                    at += 1;
                }
                text[start..at]
                    .parse::<u32>()
                    .map(|value| (value, at))
                    .map_err(|_| bad(at, "an entity reference is `@e:17` or `@e:17v2`"))
            };
            let (slot, mut at) = number(body)?;
            let mut generation = None;
            if bytes.get(at) == Some(&b'v') && bytes.get(at + 1).is_some_and(u8::is_ascii_digit) {
                let (value, end) = number(at + 1)?;
                generation = Some(value);
                at = end;
            }
            Ok((Reference::Entity { slot, generation }, at))
        }
        kind => {
            let mut at = name_end(bytes, body);
            if at == body {
                return Err(bad(at, &format!("`@{kind}:` must be followed by a name")));
            }
            if bytes.get(at) == Some(&b':') {
                let second = name_end(bytes, at + 1);
                if second > at + 1 {
                    at = second;
                }
            }
            Ok((
                Reference::Of {
                    kind: kind.to_owned(),
                    name: text[body..at].to_owned(),
                },
                at,
            ))
        }
    }
}

/// Parses one console line.
///
/// The line may be blank, or only a comment. An error says where it is
/// and, where the console can tell, what to write instead; nothing a
/// person can type makes this panic.
///
/// ```
/// use clash_sim::console::{ValueKind, codes, parse_line};
///
/// let line = parse_line("until \"party.count == 0\" max=5min").unwrap();
/// let command = line.command.unwrap();
/// assert_eq!(command.args[0].value.kind, ValueKind::Quoted("party.count == 0".into()));
/// assert_eq!(command.args[1].value.span.slice("until \"party.count == 0\" max=5min"), "5min");
///
/// assert_eq!(parse_line("spawn at=").unwrap_err().code, codes::MISSING_VALUE);
/// ```
pub fn parse_line(source: &str) -> Result<Line, Diagnostic> {
    if source.len() > MAX_LINE_LEN {
        return Err(Diagnostic::new(
            codes::TOO_LONG,
            span(0, 0),
            format!(
                "the line is {} bytes long; a console line is at most {MAX_LINE_LEN}",
                source.len()
            ),
        ));
    }
    Parser {
        source,
        bytes: source.as_bytes(),
        at: 0,
    }
    .line()
}

struct Parser<'s> {
    source: &'s str,
    bytes: &'s [u8],
    at: usize,
}

impl Parser<'_> {
    fn line(mut self) -> Result<Line, Diagnostic> {
        let mut args: Vec<Arg> = Vec::new();
        let mut comment = None;
        loop {
            while self.bytes.get(self.at).copied().is_some_and(is_space) {
                self.at += 1;
            }
            let Some(&byte) = self.bytes.get(self.at) else {
                break;
            };
            if byte == b'#' {
                comment = Some(Comment {
                    text: self.source[self.at + 1..].trim().to_owned(),
                    span: span(self.at, self.source.len()),
                });
                break;
            }
            args.push(self.arg()?);
            if let Some(&next) = self.bytes.get(self.at)
                && !is_space(next)
            {
                return Err(Diagnostic::new(
                    codes::MISSING_SEPARATOR,
                    span(self.at, self.at + 1),
                    "a value ends here and another begins with nothing between them",
                )
                .with_suggestion("put a space between two values"));
            }
        }
        let mut args = args.into_iter();
        let Some(first) = args.next() else {
            return Ok(Line {
                subject: None,
                command: None,
                comment,
            });
        };
        let (mut name, mut name_span) = command_name(first)?;
        let mut rest: Vec<Arg> = args.collect();
        let mut subject = None;
        if name == "as"
            && let Some(Arg {
                key: None,
                value:
                    Value {
                        kind: ValueKind::Ref(reference),
                        span: at,
                    },
            }) = rest.first()
        {
            subject = Some(RefAt {
                reference: reference.clone(),
                span: *at,
            });
            let end = at.end as usize;
            rest.remove(0);
            if rest.is_empty() {
                return Err(Diagnostic::new(
                    codes::MISSING_COMMAND,
                    span(end, end),
                    "`as <reference>` must be followed by a command",
                )
                .with_suggestion("for example `as @g1 act slash`"));
            }
            (name, name_span) = command_name(rest.remove(0))?;
        }
        Ok(Line {
            subject,
            command: Some(Command {
                name,
                name_span,
                args: rest,
            }),
            comment,
        })
    }

    fn arg(&mut self) -> Result<Arg, Diagnostic> {
        let start = self.at;
        let key_end = ident_end(self.bytes, start);
        if key_end > start
            && self.bytes.get(key_end) == Some(&b'=')
            && self.bytes.get(key_end + 1) != Some(&b'=')
        {
            let key = Key {
                name: self.source[start..key_end].to_owned(),
                span: span(start, key_end),
            };
            self.at = key_end + 1;
            match self.bytes.get(self.at) {
                Some(&byte) if !is_space(byte) && byte != b'#' => {}
                _ => {
                    return Err(Diagnostic::new(
                        codes::MISSING_VALUE,
                        span(start, self.at),
                        format!("`{}=` has no value", key.name),
                    )
                    .with_suggestion("write the value directly after the `=`, with no space"));
                }
            }
            let value = self.value()?;
            return Ok(Arg {
                key: Some(key),
                value,
            });
        }
        Ok(Arg {
            key: None,
            value: self.value()?,
        })
    }

    fn value(&mut self) -> Result<Value, Diagnostic> {
        match self.bytes[self.at] {
            b'"' => self.quoted(),
            b'{' => self.json(),
            _ => self.bare(),
        }
    }

    fn quoted(&mut self) -> Result<Value, Diagnostic> {
        let start = self.at;
        let unclosed = |end: usize| {
            Diagnostic::new(
                codes::UNCLOSED_QUOTE,
                span(start, end),
                "the quote opened here is never closed",
            )
            .with_suggestion("close it with `\"`; a quoted value stays on one line")
        };
        self.at += 1;
        let mut text = String::new();
        loop {
            let Some(character) = self.source[self.at..].chars().next() else {
                return Err(unclosed(self.source.len()));
            };
            match character {
                '"' => {
                    self.at += 1;
                    break;
                }
                '\n' => return Err(unclosed(self.at)),
                '\\' => match self.bytes.get(self.at + 1) {
                    Some(&escaped @ (b'"' | b'\\')) => {
                        text.push(char::from(escaped));
                        self.at += 2;
                    }
                    _ => {
                        return Err(Diagnostic::new(
                            codes::BAD_ESCAPE,
                            span(self.at, self.at + 1),
                            "a backslash inside double quotes must come before `\"` or `\\`",
                        )
                        .with_suggestion("write `\\\\` for a backslash"));
                    }
                },
                other => {
                    text.push(other);
                    self.at += other.len_utf8();
                }
            }
        }
        Ok(Value {
            kind: ValueKind::Quoted(text),
            span: span(start, self.at),
        })
    }

    fn json(&mut self) -> Result<Value, Diagnostic> {
        let start = self.at;
        let mut depth = 0_usize;
        let mut in_string = false;
        let mut at = start;
        let mut end = None;
        while let Some(&byte) = self.bytes.get(at) {
            if in_string {
                match byte {
                    b'\\' => at += 1,
                    b'"' => in_string = false,
                    _ => {}
                }
            } else {
                match byte {
                    b'"' => in_string = true,
                    b'{' => depth += 1,
                    b'}' => {
                        depth -= 1;
                        if depth == 0 {
                            end = Some(at + 1);
                            break;
                        }
                    }
                    _ => {}
                }
            }
            at += 1;
        }
        let Some(end) = end else {
            return Err(Diagnostic::new(
                codes::UNCLOSED_JSON,
                span(start, start + 1),
                "this `{` is never closed",
            )
            .with_suggestion(
                "JSON runs to its closing brace; in a script it may go on over the following lines",
            ));
        };
        let text = &self.source[start..end];
        match serde_json::from_str::<serde_json::Value>(text) {
            Ok(value) => {
                self.at = end;
                Ok(Value {
                    kind: ValueKind::Json(value),
                    span: span(start, end),
                })
            }
            Err(error) => {
                // serde_json counts lines from one and columns in bytes
                // from one.
                let line_start: usize = text
                    .split_inclusive('\n')
                    .take(error.line().saturating_sub(1))
                    .map(str::len)
                    .sum();
                let offset = (line_start + error.column().saturating_sub(1)).min(text.len() - 1);
                let message = error.to_string();
                let message = message
                    .rfind(" at line ")
                    .map_or(message.as_str(), |cut| &message[..cut]);
                Err(Diagnostic::new(
                    codes::BAD_JSON,
                    span(start + offset, start + offset + 1),
                    format!("the braces do not hold valid JSON: {message}"),
                )
                .with_suggestion(
                    "JSON uses double quotes; an expression with braces goes inside double quotes",
                ))
            }
        }
    }

    fn bare(&mut self) -> Result<Value, Diagnostic> {
        let start = self.at;
        let mut open: Vec<(u8, usize)> = Vec::new();
        while let Some(&byte) = self.bytes.get(self.at) {
            if is_space(byte) && open.is_empty() {
                break;
            }
            match byte {
                b'\'' => {
                    let quote = self.at;
                    self.at += 1;
                    loop {
                        match self.bytes.get(self.at) {
                            None => {
                                return Err(Diagnostic::new(
                                    codes::UNCLOSED_QUOTE,
                                    span(quote, self.source.len()),
                                    "the quote opened here is never closed",
                                )
                                .with_suggestion(
                                    "an expression's strings are in single quotes; a quote inside one is written twice",
                                ));
                            }
                            Some(b'\'') if self.bytes.get(self.at + 1) == Some(&b'\'') => {
                                self.at += 2;
                            }
                            Some(b'\'') => break,
                            Some(_) => self.at += 1,
                        }
                    }
                }
                b'(' | b'[' | b'{' => open.push((byte, self.at)),
                b')' | b']' | b'}' => {
                    let wanted = match byte {
                        b')' => b'(',
                        b']' => b'[',
                        _ => b'{',
                    };
                    match open.pop() {
                        Some((opened, _)) if opened == wanted => {}
                        Some((opened, _)) => {
                            return Err(Diagnostic::new(
                                codes::UNEXPECTED_CHARACTER,
                                span(self.at, self.at + 1),
                                format!(
                                    "`{}` cannot close the `{}` that is open",
                                    char::from(byte),
                                    char::from(opened)
                                ),
                            ));
                        }
                        None => {
                            return Err(Diagnostic::new(
                                codes::UNEXPECTED_CHARACTER,
                                span(self.at, self.at + 1),
                                format!("nothing is open for this `{}` to close", char::from(byte)),
                            ));
                        }
                    }
                }
                b'"' => {
                    return Err(Diagnostic::new(
                        codes::UNEXPECTED_CHARACTER,
                        span(self.at, self.at + 1),
                        "a double quote cannot appear inside an unquoted value",
                    )
                    .with_suggestion(
                        "put the whole value in double quotes and use single quotes for strings inside it",
                    ));
                }
                _ => {}
            }
            self.at += 1;
        }
        if let Some(&(opened, at)) = open.last() {
            return Err(Diagnostic::new(
                codes::UNCLOSED_BRACKET,
                span(at, at + 1),
                format!("this `{}` is never closed", char::from(opened)),
            ));
        }
        let text = &self.source[start..self.at];
        let at = span(start, self.at);
        let kind = if let Some(number) = parse_number(text) {
            ValueKind::Number(number)
        } else if is_word(text) {
            ValueKind::Word(text.to_owned())
        } else if text.starts_with('@') {
            let (reference, used) =
                parse_reference(text).map_err(|error| error.shifted(at.start))?;
            match text.as_bytes().get(used) {
                None => ValueKind::Ref(reference),
                // A reference with more after it is part of an unquoted
                // expression: `@a.value`, `@a==@b`.
                Some(&next) if next != b':' && !is_ident_char(next) => {
                    ValueKind::Raw(text.to_owned())
                }
                Some(_) => {
                    return Err(Diagnostic::new(
                        codes::BAD_REFERENCE,
                        at,
                        format!("`{text}` is not a reference"),
                    )
                    .with_suggestion(REFERENCE_FORMS));
                }
            }
        } else {
            ValueKind::Raw(text.to_owned())
        };
        Ok(Value { kind, span: at })
    }
}

/// The command name an argument is, if it can be one.
fn command_name(arg: Arg) -> Result<(String, Span), Diagnostic> {
    match arg {
        Arg {
            key: None,
            value:
                Value {
                    kind: ValueKind::Word(name),
                    span,
                },
        } => Ok((name, span)),
        Arg { key, value } => {
            let at = key.map_or(value.span, |key| key.span.to(value.span));
            Err(Diagnostic::new(
                codes::MISSING_COMMAND,
                at,
                "a line starts with the name of a command",
            )
            .with_suggestion("`help` lists the commands"))
        }
    }
}

/// Writes a value in canonical form.
pub fn print_value(value: &ValueKind) -> String {
    match value {
        ValueKind::Number(number) => {
            let numeral = match number.value {
                Numeral::Int(value) => value.to_string(),
                // The debug form of a float is the shortest that reads
                // back the same, and always has a point or an exponent.
                Numeral::Real(value) => format!("{:?}", value.get()),
            };
            match number.unit {
                Some(unit) => format!("{numeral}{}", unit.symbol()),
                None => numeral,
            }
        }
        ValueKind::Word(text) | ValueKind::Raw(text) => text.clone(),
        ValueKind::Ref(reference) => reference.to_string(),
        ValueKind::Quoted(text) => {
            let mut out = String::with_capacity(text.len() + 2);
            out.push('"');
            for character in text.chars() {
                if matches!(character, '"' | '\\') {
                    out.push('\\');
                }
                out.push(character);
            }
            out.push('"');
            out
        }
        ValueKind::Json(value) => value.to_string(),
    }
}

/// Writes a line in canonical form: one space between arguments, numbers
/// in their shortest form, JSON compact, the comment as `# text`.
/// Parsing the result gives the same tree, spans apart.
pub fn print_line(line: &Line) -> String {
    let mut parts: Vec<String> = Vec::new();
    if let Some(subject) = &line.subject {
        parts.push("as".to_owned());
        parts.push(subject.reference.to_string());
    }
    if let Some(command) = &line.command {
        parts.push(command.name.clone());
        for arg in &command.args {
            let value = print_value(&arg.value.kind);
            parts.push(match &arg.key {
                Some(key) => format!("{}={value}", key.name),
                None => value,
            });
        }
    }
    if let Some(comment) = &line.comment {
        parts.push(if comment.text.is_empty() {
            "#".to_owned()
        } else {
            format!("# {}", comment.text)
        });
    }
    parts.join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn command(source: &str) -> Command {
        parse_line(source)
            .unwrap_or_else(|error| panic!("{source}: {error}"))
            .command
            .unwrap_or_else(|| panic!("{source}: no command"))
    }

    fn value(source: &str) -> ValueKind {
        let command = command(&format!("x {source}"));
        assert_eq!(command.args.len(), 1, "{source}");
        command.args[0].value.kind.clone()
    }

    fn int(value: i64) -> ValueKind {
        ValueKind::Number(Number {
            value: Numeral::Int(value),
            unit: None,
        })
    }

    fn real(value: f64, unit: Option<&str>) -> ValueKind {
        ValueKind::Number(Number {
            value: Numeral::Real(Real::of(value)),
            unit: unit.map(|symbol| Unit::from_symbol(symbol).unwrap()),
        })
    }

    #[test]
    fn every_line_of_the_specification_parses() {
        // 14 §3.2 and §7.
        let lines = [
            "release at @here energy=1e9 kind=blast",
            "spawn town_guard at @c:120,44,0 as=g1",
            "as @g1 act slash target=@self effort=0.2",
            "eval self.pool.blood.frac",
            "do { \"add_trait\": { \"to\": \"self\", \"trait\": \"adrenaline\" } }",
            "select entities around=@self radius=10 where=\"it.has_tag('guard')\"",
            "watch dist(self, @sel)",
            "trace on rule vault_alarm",
            "explain action slash @self",
            "advance 2s",
            "until \"party.count == 0\" max=5min",
            "save before_fight",
            "check",
            "use profile=test_core seed=7",
            "gen flat size=1",
            "as @b add_trait stance_guarded",
            "assert \"@b.wounds.count == 0\"",
            "expect event strike_defended target=@b within=400ms",
            "expect trace strike turned=162 landed=18",
            "expect hash 9c41f0aa9c41f0aa9c41f0aa9c41f0aa",
        ];
        for source in lines {
            let line = parse_line(source).unwrap_or_else(|error| panic!("{source}: {error}"));
            let printed = print_line(&line);
            let again = parse_line(&printed).unwrap();
            assert_eq!(again.without_spans(), line.without_spans(), "{source}");
            assert_eq!(print_line(&again), printed, "{source}");
        }
    }

    #[test]
    fn numbers_with_and_without_units() {
        assert_eq!(value("10"), int(10));
        assert_eq!(value("-3"), int(-3));
        assert_eq!(value("+3"), int(3));
        assert_eq!(value("0.2"), real(0.2, None));
        assert_eq!(value("1e9"), real(1e9, None));
        assert_eq!(value("1.5e-3"), real(1.5e-3, None));
        assert_eq!(value("2.5kPa"), real(2.5, Some("kPa")));
        assert_eq!(
            value("400ms"),
            ValueKind::Number(Number {
                value: Numeral::Int(400),
                unit: Unit::from_symbol("ms"),
            })
        );
        let ValueKind::Number(number) = value("400ms") else {
            panic!()
        };
        assert_eq!(number.si(), Real::of(0.4));
        let ValueKind::Number(number) = value("5min") else {
            panic!()
        };
        assert_eq!(number.si(), Real::of(300.0));
        // Not numbers: an unknown unit, an integer too large, a lone
        // sign, a hash that starts with digits.
        for raw in [
            "2sec",
            "99999999999999999999",
            "-",
            "1.",
            "9c41f0aa",
            "1e999",
        ] {
            assert_eq!(value(raw), ValueKind::Raw(raw.to_owned()), "{raw}");
        }
    }

    #[test]
    fn words_and_ids() {
        assert_eq!(value("town_guard"), ValueKind::Word("town_guard".into()));
        assert_eq!(value("core:glass"), ValueKind::Word("core:glass".into()));
        assert_eq!(value("_x1"), ValueKind::Word("_x1".into()));
        assert_eq!(value("a:b:c"), ValueKind::Raw("a:b:c".into()));
        assert_eq!(value("a:"), ValueKind::Raw("a:".into()));
        assert!(is_word("on") && !is_word("") && !is_word("1a") && !is_word("a.b"));
    }

    #[test]
    fn references_in_every_form() {
        let reference = |source: &str| match value(source) {
            ValueKind::Ref(reference) => reference,
            other => panic!("{source}: {other:?}"),
        };
        assert_eq!(reference("@here"), Reference::Here);
        assert_eq!(reference("@self"), Reference::Active);
        assert_eq!(reference("@sel"), Reference::Selection);
        assert_eq!(
            reference("@c:120,44,0"),
            Reference::Cell {
                x: 120,
                y: 44,
                z: 0
            }
        );
        assert_eq!(
            reference("@c:-1,-2,-3"),
            Reference::Cell {
                x: -1,
                y: -2,
                z: -3
            }
        );
        assert_eq!(
            reference("@e:17"),
            Reference::Entity {
                slot: 17,
                generation: None
            }
        );
        assert_eq!(
            reference("@e:17v2"),
            Reference::Entity {
                slot: 17,
                generation: Some(2)
            }
        );
        assert_eq!(
            reference("@site:old_mill"),
            Reference::Of {
                kind: "site".into(),
                name: "old_mill".into()
            }
        );
        assert_eq!(
            reference("@site:core:old_mill"),
            Reference::Of {
                kind: "site".into(),
                name: "core:old_mill".into()
            }
        );
        assert_eq!(reference("@g1"), Reference::Named("g1".into()));
        for source in [
            "@here",
            "@self",
            "@sel",
            "@c:1,-2,3",
            "@e:4",
            "@e:4v9",
            "@site:a",
            "@area:core:b",
            "@g1",
        ] {
            assert_eq!(reference(source).to_string(), source);
        }
    }

    #[test]
    fn malformed_references_are_errors_with_the_forms() {
        for source in [
            "x @",
            "x @1",
            "x @c:1,2",
            "x @c:a,b,c",
            "x @e:x",
            "x @site:",
            "x @c:1,2,99999999999",
        ] {
            let error = parse_line(source).unwrap_err();
            assert_eq!(error.code, codes::BAD_REFERENCE, "{source}");
            assert!(error.suggestion.as_deref().unwrap().contains("@c:x,y,z"));
            assert!(error.span.start >= 2, "{source}: {:?}", error.span);
        }
    }

    #[test]
    fn quoted_values_and_their_escapes() {
        assert_eq!(
            value("\"it.has_tag('guard')\""),
            ValueKind::Quoted("it.has_tag('guard')".into())
        );
        assert_eq!(
            value(r#""a \"b\" \\ c""#),
            ValueKind::Quoted(r#"a "b" \ c"#.into())
        );
        assert_eq!(value("\"\""), ValueKind::Quoted(String::new()));
        assert_eq!(
            value("\"# not a comment\""),
            ValueKind::Quoted("# not a comment".into())
        );
        assert_eq!(
            print_value(&ValueKind::Quoted(r#"a "b" \ c"#.into())),
            r#""a \"b\" \\ c""#
        );
        assert_eq!(
            parse_line("x \"abc").unwrap_err().code,
            codes::UNCLOSED_QUOTE
        );
        assert_eq!(
            parse_line("x \"a\nb\"").unwrap_err().code,
            codes::UNCLOSED_QUOTE
        );
        let error = parse_line(r#"x "a\nb""#).unwrap_err();
        assert_eq!(error.code, codes::BAD_ESCAPE);
        assert_eq!(error.span, Span::new(4, 5));
        assert_eq!(
            parse_line("x \"a\"\"b\"").unwrap_err().code,
            codes::MISSING_SEPARATOR
        );
    }

    #[test]
    fn json_runs_to_its_closing_brace() {
        let source = "do { \"add_trait\": { \"to\": \"self\", \"n\": [1, 2.5, \"}\"] } } # after";
        let line = parse_line(source).unwrap();
        let command = line.command.as_ref().unwrap();
        let ValueKind::Json(json) = &command.args[0].value.kind else {
            panic!()
        };
        assert_eq!(json["add_trait"]["n"][2], "}");
        assert_eq!(line.comment.as_ref().unwrap().text, "after");
        assert_eq!(
            print_line(&line),
            "do {\"add_trait\":{\"n\":[1,2.5,\"}\"],\"to\":\"self\"}} # after"
        );
        // Over several lines, as a script may write it.
        let line = parse_line("do {\n  \"a\": 1,\n  \"b\": \"x\\\"}\"\n} more").unwrap();
        let command = line.command.unwrap();
        assert!(matches!(command.args[0].value.kind, ValueKind::Json(_)));
        assert_eq!(command.args[1].value.kind, ValueKind::Word("more".into()));
        // As a named argument.
        let command = super::tests::command("run rule with={\"n\": 3}");
        assert_eq!(command.args[1].key.as_ref().unwrap().name, "with");

        let error = parse_line("do { \"a\": 1").unwrap_err();
        assert_eq!(error.code, codes::UNCLOSED_JSON);
        assert_eq!(error.span, Span::new(3, 4));
        let error = parse_line("do { 'a': 1 }").unwrap_err();
        assert_eq!(error.code, codes::BAD_JSON);
        assert_eq!(error.span, Span::new(5, 6));
        assert!(error.message.contains("valid JSON"), "{}", error.message);
        assert!(!error.message.contains("line 1"), "{}", error.message);
        let error = parse_line("do {\n \"a\": 1,\n \"b\" 2 }").unwrap_err();
        assert_eq!(error.code, codes::BAD_JSON);
        assert_eq!(error.span, Span::new(19, 20));
        assert_eq!(
            parse_line("do {}x").unwrap_err().code,
            codes::MISSING_SEPARATOR
        );
    }

    #[test]
    fn raw_values_keep_brackets_and_strings_together() {
        assert_eq!(
            value("self.pool.blood.frac"),
            ValueKind::Raw("self.pool.blood.frac".into())
        );
        assert_eq!(
            value("dist(self, @sel)"),
            ValueKind::Raw("dist(self, @sel)".into())
        );
        assert_eq!(value("'two  words'"), ValueKind::Raw("'two  words'".into()));
        assert_eq!(value("'it''s here'"), ValueKind::Raw("'it''s here'".into()));
        assert_eq!(value("xs[0 + 1]"), ValueKind::Raw("xs[0 + 1]".into()));
        assert_eq!(value("f({'a': 1})"), ValueKind::Raw("f({'a': 1})".into()));
        assert_eq!(value("@a.value"), ValueKind::Raw("@a.value".into()));
        assert_eq!(value("@a==@b"), ValueKind::Raw("@a==@b".into()));
        assert_eq!(
            value("frames/menu.txt"),
            ValueKind::Raw("frames/menu.txt".into())
        );
        let command = command("eval 1 + 2 == 3");
        let kinds: Vec<String> = command
            .args
            .iter()
            .map(|arg| print_value(&arg.value.kind))
            .collect();
        assert_eq!(kinds, ["1", "+", "2", "==", "3"]);
        assert!(command.args.iter().all(|arg| arg.key.is_none()));

        assert_eq!(
            parse_line("x f(1, 2").unwrap_err().code,
            codes::UNCLOSED_BRACKET
        );
        assert_eq!(
            parse_line("x f(1]").unwrap_err().code,
            codes::UNEXPECTED_CHARACTER
        );
        assert_eq!(
            parse_line("x a)").unwrap_err().code,
            codes::UNEXPECTED_CHARACTER
        );
        assert_eq!(
            parse_line("x a\"b\"").unwrap_err().code,
            codes::UNEXPECTED_CHARACTER
        );
        assert_eq!(
            parse_line("x 'abc").unwrap_err().code,
            codes::UNCLOSED_QUOTE
        );
    }

    #[test]
    fn named_arguments() {
        let command = command("spawn town_guard at=@here as=g1 n=2 where=\"a == b\" x==1 y<=2");
        let keys: Vec<Option<&str>> = command
            .args
            .iter()
            .map(|arg| arg.key.as_ref().map(|key| key.name.as_str()))
            .collect();
        assert_eq!(
            keys,
            [
                None,
                Some("at"),
                Some("as"),
                Some("n"),
                Some("where"),
                None,
                None
            ]
        );
        assert_eq!(command.args[5].value.kind, ValueKind::Raw("x==1".into()));
        let error = parse_line("spawn at= @here").unwrap_err();
        assert_eq!(error.code, codes::MISSING_VALUE);
        assert_eq!(error.span, Span::new(6, 9));
        assert_eq!(
            parse_line("spawn at=#c").unwrap_err().code,
            codes::MISSING_VALUE
        );
    }

    #[test]
    fn the_as_prefix() {
        let line = parse_line("as @b add_trait stance_guarded").unwrap();
        assert_eq!(
            line.subject.as_ref().unwrap().reference,
            Reference::Named("b".into())
        );
        assert_eq!(line.subject.as_ref().unwrap().span, Span::new(3, 5));
        let command = line.command.unwrap();
        assert_eq!(command.name, "add_trait");
        assert_eq!(command.name_span, Span::new(6, 15));
        assert_eq!(command.args.len(), 1);

        let error = parse_line("as @b").unwrap_err();
        assert_eq!(error.code, codes::MISSING_COMMAND);
        assert_eq!(error.span, Span::new(5, 5));
        assert_eq!(
            parse_line("as @b 12").unwrap_err().code,
            codes::MISSING_COMMAND
        );
        // Without a reference, `as` is an ordinary command name.
        assert_eq!(command_of("as x").name, "as");
    }

    fn command_of(source: &str) -> Command {
        command(source)
    }

    #[test]
    fn comments_and_blank_lines() {
        for blank in ["", "   ", "\t", " \r"] {
            let line = parse_line(blank).unwrap();
            assert!(line.is_blank() && line.comment.is_none(), "{blank:?}");
            assert_eq!(print_line(&line), "");
        }
        let line = parse_line("  #  scenarios/combat/parry.scn  ").unwrap();
        assert!(line.is_blank());
        assert_eq!(
            line.comment.as_ref().unwrap().text,
            "scenarios/combat/parry.scn"
        );
        assert_eq!(line.comment.as_ref().unwrap().span, Span::new(2, 33));
        assert_eq!(print_line(&line), "# scenarios/combat/parry.scn");
        let line = parse_line("check #").unwrap();
        assert_eq!(print_line(&line), "check #");
        // A `#` inside a value is not a comment.
        let command = command("x a#b");
        assert_eq!(command.args[0].value.kind, ValueKind::Raw("a#b".into()));
    }

    #[test]
    fn a_line_must_start_with_a_command_name() {
        for source in ["12 x", "@here", "\"quoted\"", "{} x", "a=1", "a.b c"] {
            let error = parse_line(source).unwrap_err();
            assert_eq!(error.code, codes::MISSING_COMMAND, "{source}");
            assert_eq!(error.span.start, 0, "{source}");
        }
    }

    #[test]
    fn a_line_that_is_too_long_is_refused() {
        let long = format!("x {}", "a".repeat(MAX_LINE_LEN));
        assert_eq!(parse_line(&long).unwrap_err().code, codes::TOO_LONG);
    }

    #[test]
    fn spans_cover_what_was_written() {
        let source = "spawn  town_guard at=@c:1,2,3   n=2kg";
        let command = command(source);
        assert_eq!(command.name_span.slice(source), "spawn");
        assert_eq!(command.args[0].value.span.slice(source), "town_guard");
        assert_eq!(
            command.args[1].key.as_ref().unwrap().span.slice(source),
            "at"
        );
        assert_eq!(command.args[1].value.span.slice(source), "@c:1,2,3");
        assert_eq!(command.args[2].value.span.slice(source), "2kg");
    }

    #[test]
    fn names_for_references() {
        assert!(is_reference_name("g1") && is_reference_name("_a"));
        for bad in ["", "1a", "here", "self", "sel", "c", "e", "a:b", "a b"] {
            assert!(!is_reference_name(bad), "{bad}");
        }
    }
}
