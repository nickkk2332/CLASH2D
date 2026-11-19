//! A JSON parser that keeps where everything was written.
//!
//! The loader cannot use an ordinary JSON library: every diagnostic must
//! point at the file, line and column of the offending value or key
//! (01 R6, 04 §8.2), and a value keeps its origin through patching and
//! inheritance (04 §6.5). So a parsed file is a tree of [`Node`]s, each
//! with a [`Span`], and each object member also keeps the span of its key.
//!
//! Differences from plain JSON, all from 04 §5:
//!
//! - A member whose key is `"//"` is a comment. It is dropped while
//!   parsing, anywhere, and may appear more than once in one object.
//! - The order of keys in an object is kept.
//! - The same key twice in one object is a mistake and is reported.

use std::fmt;

use crate::diag::{Code, code};
use crate::source::{FileId, Span, to_u32};
use crate::units::Dim;

/// The key that marks a comment (04 §5).
pub const COMMENT_KEY: &str = "//";

/// Limits that stop a malformed or hostile file from exhausting memory
/// (04 §15). Exceeding one is a diagnostic, never a crash.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Limits {
    /// The largest file read, in bytes.
    pub max_file_bytes: usize,
    /// The deepest nesting of lists and objects in a file.
    pub max_depth: usize,
    /// The most definitions one load may hold, inline ones included.
    pub max_definitions: usize,
}

impl Limits {
    /// The default largest file: 16 MiB.
    pub const DEFAULT_MAX_FILE_BYTES: usize = 16 * 1024 * 1024;
    /// The default deepest nesting.
    pub const DEFAULT_MAX_DEPTH: usize = 64;
    /// The default most definitions.
    pub const DEFAULT_MAX_DEFINITIONS: usize = 1_000_000;
}

impl Default for Limits {
    fn default() -> Limits {
        Limits {
            max_file_bytes: Limits::DEFAULT_MAX_FILE_BYTES,
            max_depth: Limits::DEFAULT_MAX_DEPTH,
            max_definitions: Limits::DEFAULT_MAX_DEFINITIONS,
        }
    }
}

/// A JSON number as written.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Num {
    /// A whole number that fits `i64`.
    Int(i64),
    /// A whole number above `i64::MAX` that fits `u64`.
    UInt(u64),
    /// Anything else. Never NaN or infinite.
    Float(f64),
}

impl Num {
    /// The number as an `f64`, rounded to the nearest representable value.
    pub fn as_f64(self) -> f64 {
        match self {
            Num::Int(value) => value as f64,
            Num::UInt(value) => value as f64,
            Num::Float(value) => value,
        }
    }
}

impl fmt::Display for Num {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Num::Int(value) => write!(f, "{value}"),
            Num::UInt(value) => write!(f, "{value}"),
            Num::Float(value) => {
                if value.fract() == 0.0 && value.abs() < 1e15 {
                    write!(f, "{value:.1}")
                } else {
                    write!(f, "{value}")
                }
            }
        }
    }
}

/// One member of an object: a key, where the key was written, and a value.
#[derive(Clone, Debug, PartialEq)]
pub struct Member {
    /// The key, with escapes decoded.
    pub key: String,
    /// Where the key was written, quotation marks included.
    pub key_span: Span,
    /// The value.
    pub value: Node,
}

/// What a [`Node`] holds.
#[derive(Clone, Debug, PartialEq)]
pub enum NodeKind {
    /// `null`.
    Null,
    /// `true` or `false`.
    Bool(bool),
    /// A number.
    Number(Num),
    /// A number in SI units that was written with a unit and then changed
    /// by `scale` or `add` (04 §6.1). It never comes from the parser. It
    /// keeps its dimension so that the field it lands in can still check
    /// it.
    Quantity {
        /// The magnitude in SI units.
        value: f64,
        /// What it measures.
        dim: Dim,
    },
    /// A string, with escapes decoded.
    String(String),
    /// A list.
    Array(Vec<Node>),
    /// An object, in the order its keys were written. Comments are gone.
    Object(Vec<Member>),
}

/// A JSON value and where it was written.
///
/// After merging (04 §6) a tree is made of nodes from several files, and
/// each still carries its own span: that is how an error in an inherited
/// field points at the line that supplied it.
#[derive(Clone, Debug, PartialEq)]
pub struct Node {
    /// The value.
    pub kind: NodeKind,
    /// Where it was written.
    pub span: Span,
}

impl Node {
    /// A node of `kind` written at `span`.
    pub fn new(kind: NodeKind, span: Span) -> Node {
        Node { kind, span }
    }

    /// An empty object attributed to `span`.
    pub fn empty_object(span: Span) -> Node {
        Node::new(NodeKind::Object(Vec::new()), span)
    }

    /// What the value is, in words for a message: `a string`, `a list`.
    pub fn kind_name(&self) -> &'static str {
        match &self.kind {
            NodeKind::Null => "null",
            NodeKind::Bool(_) => "a boolean",
            NodeKind::Number(_) => "a number",
            NodeKind::Quantity { .. } => "a quantity with a unit",
            NodeKind::String(_) => "a string",
            NodeKind::Array(_) => "a list",
            NodeKind::Object(_) => "an object",
        }
    }

    /// The string, if this is one.
    pub fn as_str(&self) -> Option<&str> {
        match &self.kind {
            NodeKind::String(text) => Some(text),
            _ => None,
        }
    }

    /// The boolean, if this is one.
    pub fn as_bool(&self) -> Option<bool> {
        match &self.kind {
            NodeKind::Bool(value) => Some(*value),
            _ => None,
        }
    }

    /// The members, if this is an object.
    pub fn as_object(&self) -> Option<&[Member]> {
        match &self.kind {
            NodeKind::Object(members) => Some(members),
            _ => None,
        }
    }

    /// The members for changing, if this is an object.
    pub fn as_object_mut(&mut self) -> Option<&mut Vec<Member>> {
        match &mut self.kind {
            NodeKind::Object(members) => Some(members),
            _ => None,
        }
    }

    /// The elements, if this is a list.
    pub fn as_array(&self) -> Option<&[Node]> {
        match &self.kind {
            NodeKind::Array(items) => Some(items),
            _ => None,
        }
    }

    /// The value under `key`, if this is an object that has it.
    pub fn get(&self, key: &str) -> Option<&Node> {
        self.member(key).map(|member| &member.value)
    }

    /// The member under `key`, if this is an object that has it.
    pub fn member(&self, key: &str) -> Option<&Member> {
        self.as_object()?.iter().find(|member| member.key == key)
    }

    /// Whether two trees hold the same values, wherever they were written.
    /// Object members are compared in order, because order can carry
    /// meaning (04 §5).
    pub fn same_value(&self, other: &Node) -> bool {
        match (&self.kind, &other.kind) {
            (NodeKind::Null, NodeKind::Null) => true,
            (NodeKind::Bool(a), NodeKind::Bool(b)) => a == b,
            (NodeKind::Number(a), NodeKind::Number(b)) => a.as_f64() == b.as_f64(),
            (
                NodeKind::Quantity { value: a, dim: da },
                NodeKind::Quantity { value: b, dim: db },
            ) => a == b && da == db,
            (NodeKind::String(a), NodeKind::String(b)) => a == b,
            (NodeKind::Array(a), NodeKind::Array(b)) => {
                a.len() == b.len() && a.iter().zip(b).all(|(a, b)| a.same_value(b))
            }
            (NodeKind::Object(a), NodeKind::Object(b)) => {
                a.len() == b.len()
                    && a.iter()
                        .zip(b)
                        .all(|(a, b)| a.key == b.key && a.value.same_value(&b.value))
            }
            _ => false,
        }
    }

    /// The tree as compact JSON text. A [`NodeKind::Quantity`] is written
    /// as a string with its SI unit.
    pub fn to_json(&self) -> String {
        let mut out = String::new();
        self.write_json(&mut out);
        out
    }

    fn write_json(&self, out: &mut String) {
        match &self.kind {
            NodeKind::Null => out.push_str("null"),
            NodeKind::Bool(value) => out.push_str(if *value { "true" } else { "false" }),
            NodeKind::Number(number) => out.push_str(&number.to_string()),
            NodeKind::Quantity { value, dim } => {
                write_json_string(&format!("{value} {}", dim.si_unit()), out);
            }
            NodeKind::String(text) => write_json_string(text, out),
            NodeKind::Array(items) => {
                out.push('[');
                for (index, item) in items.iter().enumerate() {
                    if index > 0 {
                        out.push(',');
                    }
                    item.write_json(out);
                }
                out.push(']');
            }
            NodeKind::Object(members) => {
                out.push('{');
                for (index, member) in members.iter().enumerate() {
                    if index > 0 {
                        out.push(',');
                    }
                    write_json_string(&member.key, out);
                    out.push(':');
                    member.value.write_json(out);
                }
                out.push('}');
            }
        }
    }
}

/// Writes `text` as a JSON string literal.
pub(crate) fn write_json_string(text: &str, out: &mut String) {
    out.push('"');
    for ch in text.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            ch if (ch as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", ch as u32)),
            ch => out.push(ch),
        }
    }
    out.push('"');
}

/// Something wrong with a file's JSON, before it becomes a diagnostic.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct JsonProblem {
    /// The diagnostic code.
    pub code: Code,
    /// Where.
    pub span: Span,
    /// What, in the form of a diagnostic message.
    pub message: String,
    /// What to do about it, if the parser can tell.
    pub suggestion: Option<String>,
}

/// Parses one file.
///
/// Returns the tree, or `None` if the text is not JSON, and every problem
/// found. A tree may come back together with problems: a repeated key is
/// reported and the first value kept.
pub fn parse_json(file: FileId, text: &str, limits: &Limits) -> (Option<Node>, Vec<JsonProblem>) {
    let mut parser = Parser {
        file,
        text,
        bytes: text.as_bytes(),
        at: 0,
        depth: 0,
        limits,
        problems: Vec::new(),
    };
    if text.len() > limits.max_file_bytes {
        parser.problems.push(JsonProblem {
            code: code::FILE_TOO_LARGE,
            span: Span::file_start(file),
            message: format!(
                "file is {} bytes; the limit is {}",
                text.len(),
                limits.max_file_bytes
            ),
            suggestion: Some("split it into several files".to_owned()),
        });
        return (None, parser.problems);
    }
    // A byte order mark is not JSON, but editors add one.
    if text.starts_with('\u{feff}') {
        parser.at = '\u{feff}'.len_utf8();
    }
    let tree = parser.document();
    (tree.ok(), parser.problems)
}

struct Parser<'a> {
    file: FileId,
    text: &'a str,
    bytes: &'a [u8],
    at: usize,
    depth: usize,
    limits: &'a Limits,
    problems: Vec<JsonProblem>,
}

/// The parser stops at the first syntax error: what follows one cannot be
/// trusted. The error itself is already in `problems`.
struct Stop;

impl Parser<'_> {
    fn span(&self, start: usize, end: usize) -> Span {
        Span::new(self.file, to_u32(start), to_u32(end))
    }

    fn fail<T>(&mut self, start: usize, end: usize, message: impl Into<String>) -> Result<T, Stop> {
        self.fail_with(start, end, message, None)
    }

    fn fail_with<T>(
        &mut self,
        start: usize,
        end: usize,
        message: impl Into<String>,
        suggestion: Option<&str>,
    ) -> Result<T, Stop> {
        self.problems.push(JsonProblem {
            code: code::MALFORMED_JSON,
            span: self.span(start, end),
            message: message.into(),
            suggestion: suggestion.map(str::to_owned),
        });
        Err(Stop)
    }

    fn skip_whitespace(&mut self) {
        while let Some(byte) = self.bytes.get(self.at) {
            if matches!(byte, b' ' | b'\t' | b'\n' | b'\r') {
                self.at += 1;
            } else {
                break;
            }
        }
    }

    fn peek(&self) -> Option<u8> {
        self.bytes.get(self.at).copied()
    }

    /// The character at the cursor, for a message.
    fn found(&self) -> String {
        match self.text[self.at..].chars().next() {
            Some(ch) => format!("`{ch}`"),
            None => "the end of the file".to_owned(),
        }
    }

    /// One past the character at the cursor.
    fn after_char(&self) -> usize {
        self.at
            + self.text[self.at..]
                .chars()
                .next()
                .map_or(0, char::len_utf8)
    }

    fn document(&mut self) -> Result<Node, Stop> {
        self.skip_whitespace();
        if self.at >= self.bytes.len() {
            return self.fail(
                self.at,
                self.at,
                "file is empty; expected a list of definitions",
            );
        }
        let node = self.value()?;
        self.skip_whitespace();
        if self.at < self.bytes.len() {
            let message = format!("expected the end of the file, found {}", self.found());
            return self.fail_with(
                self.at,
                self.after_char(),
                message,
                Some("a data file holds one list or one object"),
            );
        }
        Ok(node)
    }

    fn value(&mut self) -> Result<Node, Stop> {
        self.skip_whitespace();
        let start = self.at;
        match self.peek() {
            None => self.fail(start, start, "expected a value, found the end of the file"),
            Some(b'{') => self.object(),
            Some(b'[') => self.array(),
            Some(b'"') => {
                let text = self.string()?;
                Ok(Node::new(NodeKind::String(text), self.span(start, self.at)))
            }
            Some(b't') => self.word("true", NodeKind::Bool(true)),
            Some(b'f') => self.word("false", NodeKind::Bool(false)),
            Some(b'n') => self.word("null", NodeKind::Null),
            Some(b'-' | b'0'..=b'9') => self.number(),
            Some(b'/') => self.fail_with(
                start,
                self.after_char(),
                "comments are not JSON",
                Some("write a comment as a member with the key \"//\""),
            ),
            Some(b'\'') => self.fail_with(
                start,
                self.after_char(),
                "strings take double quotation marks",
                Some("use \" in place of '"),
            ),
            Some(_) => {
                let message = format!("expected a value, found {}", self.found());
                self.fail(start, self.after_char(), message)
            }
        }
    }

    fn word(&mut self, word: &str, kind: NodeKind) -> Result<Node, Stop> {
        let start = self.at;
        if self.text[start..].starts_with(word) {
            self.at += word.len();
            return Ok(Node::new(kind, self.span(start, self.at)));
        }
        let mut end = start;
        while self.bytes.get(end).is_some_and(u8::is_ascii_alphanumeric) {
            end += 1;
        }
        let found = &self.text[start..end];
        self.fail(
            start,
            end.max(start + 1),
            format!("expected a value, found `{found}`"),
        )
    }

    fn enter(&mut self, start: usize) -> Result<(), Stop> {
        self.depth += 1;
        if self.depth > self.limits.max_depth {
            self.problems.push(JsonProblem {
                code: code::NESTING_TOO_DEEP,
                span: self.span(start, start + 1),
                message: format!(
                    "lists and objects are nested more than {} deep",
                    self.limits.max_depth
                ),
                suggestion: None,
            });
            return Err(Stop);
        }
        Ok(())
    }

    fn array(&mut self) -> Result<Node, Stop> {
        let start = self.at;
        self.enter(start)?;
        self.at += 1;
        let mut items = Vec::new();
        self.skip_whitespace();
        if self.peek() == Some(b']') {
            self.at += 1;
        } else {
            loop {
                items.push(self.value()?);
                self.skip_whitespace();
                match self.peek() {
                    Some(b',') => {
                        let comma = self.at;
                        self.at += 1;
                        self.skip_whitespace();
                        if self.peek() == Some(b']') {
                            return self.fail_with(
                                comma,
                                comma + 1,
                                "a comma must be followed by another value",
                                Some("remove the trailing comma"),
                            );
                        }
                    }
                    Some(b']') => {
                        self.at += 1;
                        break;
                    }
                    None => {
                        return self.fail_with(
                            start,
                            start + 1,
                            "this list is never closed",
                            Some("add `]`"),
                        );
                    }
                    Some(_) => {
                        let message = format!("expected `,` or `]`, found {}", self.found());
                        return self.fail(self.at, self.after_char(), message);
                    }
                }
            }
        }
        self.depth -= 1;
        Ok(Node::new(NodeKind::Array(items), self.span(start, self.at)))
    }

    fn object(&mut self) -> Result<Node, Stop> {
        let start = self.at;
        self.enter(start)?;
        self.at += 1;
        let mut members: Vec<Member> = Vec::new();
        self.skip_whitespace();
        if self.peek() == Some(b'}') {
            self.at += 1;
        } else {
            loop {
                self.skip_whitespace();
                let key_start = self.at;
                if self.peek() != Some(b'"') {
                    if self.peek().is_none() {
                        return self.fail_with(
                            start,
                            start + 1,
                            "this object is never closed",
                            Some("add `}`"),
                        );
                    }
                    let message =
                        format!("expected a key in quotation marks, found {}", self.found());
                    return self.fail(key_start, self.after_char(), message);
                }
                let key = self.string()?;
                let key_span = self.span(key_start, self.at);
                self.skip_whitespace();
                if self.peek() != Some(b':') {
                    let message = format!("expected `:` after the key, found {}", self.found());
                    return self.fail(self.at, self.after_char().max(self.at + 1), message);
                }
                self.at += 1;
                let value = self.value()?;
                if key == COMMENT_KEY {
                    // A comment: ignored anywhere (04 §5).
                } else if members.iter().any(|member| member.key == key) {
                    self.problems.push(JsonProblem {
                        code: code::DUPLICATE_KEY,
                        span: key_span,
                        message: format!("key \"{key}\" appears twice in one object"),
                        suggestion: Some("remove one; the first is the one kept".to_owned()),
                    });
                } else {
                    members.push(Member {
                        key,
                        key_span,
                        value,
                    });
                }
                self.skip_whitespace();
                match self.peek() {
                    Some(b',') => {
                        let comma = self.at;
                        self.at += 1;
                        self.skip_whitespace();
                        if self.peek() == Some(b'}') {
                            return self.fail_with(
                                comma,
                                comma + 1,
                                "a comma must be followed by another member",
                                Some("remove the trailing comma"),
                            );
                        }
                    }
                    Some(b'}') => {
                        self.at += 1;
                        break;
                    }
                    None => {
                        return self.fail_with(
                            start,
                            start + 1,
                            "this object is never closed",
                            Some("add `}`"),
                        );
                    }
                    Some(_) => {
                        let message = format!("expected `,` or `}}`, found {}", self.found());
                        return self.fail(self.at, self.after_char(), message);
                    }
                }
            }
        }
        self.depth -= 1;
        Ok(Node::new(
            NodeKind::Object(members),
            self.span(start, self.at),
        ))
    }

    fn string(&mut self) -> Result<String, Stop> {
        let start = self.at;
        self.at += 1;
        let mut out = String::new();
        loop {
            let Some(byte) = self.peek() else {
                return self.fail_with(
                    start,
                    start + 1,
                    "this string is never closed",
                    Some("add `\"`"),
                );
            };
            match byte {
                b'"' => {
                    self.at += 1;
                    return Ok(out);
                }
                b'\\' => {
                    let escape = self.at;
                    self.at += 1;
                    let Some(kind) = self.peek() else {
                        return self.fail(start, start + 1, "this string is never closed");
                    };
                    self.at += 1;
                    match kind {
                        b'"' => out.push('"'),
                        b'\\' => out.push('\\'),
                        b'/' => out.push('/'),
                        b'b' => out.push('\u{8}'),
                        b'f' => out.push('\u{c}'),
                        b'n' => out.push('\n'),
                        b'r' => out.push('\r'),
                        b't' => out.push('\t'),
                        b'u' => out.push(self.unicode_escape(escape)?),
                        _ => {
                            self.at = escape + 1;
                            let end = self.after_char();
                            return self.fail_with(
                                escape,
                                end,
                                "unknown escape in a string",
                                Some("the escapes are \\\" \\\\ \\/ \\b \\f \\n \\r \\t \\uXXXX"),
                            );
                        }
                    }
                }
                b'\n' | b'\r' => {
                    return self.fail_with(
                        start,
                        start + 1,
                        "this string is never closed before the end of the line",
                        Some("add `\"`, or write a line break as \\n"),
                    );
                }
                byte if byte < 0x20 => {
                    return self.fail_with(
                        self.at,
                        self.at + 1,
                        "a control character must be written as an escape",
                        Some("write a tab as \\t"),
                    );
                }
                _ => {
                    // A run of ordinary characters. Continuation bytes of
                    // UTF-8 are all above 0x7f, so scanning bytes is safe.
                    let run = self.at;
                    while let Some(&byte) = self.bytes.get(self.at) {
                        if byte == b'"' || byte == b'\\' || byte < 0x20 {
                            break;
                        }
                        self.at += 1;
                    }
                    out.push_str(&self.text[run..self.at]);
                }
            }
        }
    }

    fn hex4(&mut self) -> Option<u32> {
        let digits = self.text.get(self.at..self.at + 4)?;
        if !digits.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            return None;
        }
        let value = u32::from_str_radix(digits, 16).ok()?;
        self.at += 4;
        Some(value)
    }

    fn unicode_escape(&mut self, escape: usize) -> Result<char, Stop> {
        let Some(first) = self.hex4() else {
            return self.fail(
                escape,
                (escape + 6).min(self.bytes.len()),
                "\\u must be followed by four hexadecimal digits",
            );
        };
        if (0xD800..0xDC00).contains(&first) {
            if self.text[self.at..].starts_with("\\u") {
                self.at += 2;
                if let Some(second) = self.hex4()
                    && (0xDC00..0xE000).contains(&second)
                {
                    let scalar = 0x10000 + ((first - 0xD800) << 10) + (second - 0xDC00);
                    if let Some(ch) = char::from_u32(scalar) {
                        return Ok(ch);
                    }
                }
            }
            return self.fail(
                escape,
                self.at.max(escape + 6).min(self.bytes.len()),
                "this \\u escape is half of a pair and the other half is missing",
            );
        }
        match char::from_u32(first) {
            Some(ch) => Ok(ch),
            None => self.fail(
                escape,
                escape + 6,
                "this \\u escape is half of a pair and the other half is missing",
            ),
        }
    }

    fn number(&mut self) -> Result<Node, Stop> {
        let start = self.at;
        if self.peek() == Some(b'-') {
            self.at += 1;
        }
        let digits_start = self.at;
        while self.peek().is_some_and(|byte| byte.is_ascii_digit()) {
            self.at += 1;
        }
        if self.at == digits_start {
            return self.fail(start, self.at.max(start + 1), "expected digits after `-`");
        }
        if self.bytes[digits_start] == b'0' && self.at - digits_start > 1 {
            return self.fail_with(
                start,
                self.at,
                "a number may not start with a zero",
                Some("remove the leading zeros"),
            );
        }
        let mut whole = true;
        if self.peek() == Some(b'.') {
            whole = false;
            self.at += 1;
            let fraction_start = self.at;
            while self.peek().is_some_and(|byte| byte.is_ascii_digit()) {
                self.at += 1;
            }
            if self.at == fraction_start {
                return self.fail_with(
                    start,
                    self.at,
                    "expected digits after the decimal point",
                    Some("write `1.0`, not `1.`"),
                );
            }
        }
        if matches!(self.peek(), Some(b'e' | b'E')) {
            whole = false;
            self.at += 1;
            if matches!(self.peek(), Some(b'+' | b'-')) {
                self.at += 1;
            }
            let exponent_start = self.at;
            while self.peek().is_some_and(|byte| byte.is_ascii_digit()) {
                self.at += 1;
            }
            if self.at == exponent_start {
                return self.fail(start, self.at, "expected digits in the exponent");
            }
        }
        if self
            .peek()
            .is_some_and(|byte| byte.is_ascii_alphanumeric() || byte == b'.' || byte == b'_')
        {
            let mut end = self.at;
            while self
                .bytes
                .get(end)
                .is_some_and(|byte| byte.is_ascii_alphanumeric() || *byte == b'.' || *byte == b'_')
            {
                end += 1;
            }
            return self.fail_with(
                start,
                end,
                format!("`{}` is not a number", &self.text[start..end]),
                Some("a quantity with a unit is a string: \"250 ms\""),
            );
        }
        let literal = &self.text[start..self.at];
        let number = if whole {
            if let Ok(value) = literal.parse::<i64>() {
                Some(Num::Int(value))
            } else if let Ok(value) = literal.parse::<u64>() {
                Some(Num::UInt(value))
            } else {
                None
            }
        } else {
            None
        };
        let number = match number {
            Some(number) => number,
            None => match literal.parse::<f64>() {
                Ok(value) if value.is_finite() => Num::Float(value),
                _ => {
                    return self.fail(start, self.at, "this number is too large to represent");
                }
            },
        };
        Ok(Node::new(
            NodeKind::Number(number),
            self.span(start, self.at),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::source::SourceMap;

    fn parse(text: &str) -> (Option<Node>, Vec<JsonProblem>, SourceMap) {
        let mut sources = SourceMap::new();
        let file = sources.add("t.json", text);
        let (tree, problems) = parse_json(file, text, &Limits::default());
        (tree, problems, sources)
    }

    fn ok(text: &str) -> Node {
        let (tree, problems, _) = parse(text);
        assert_eq!(problems, Vec::new(), "{text}");
        tree.expect("a tree")
    }

    fn error(text: &str) -> (String, String) {
        let (tree, problems, sources) = parse(text);
        assert!(tree.is_none(), "{text} should not parse");
        assert_eq!(problems.len(), 1, "{text}: {problems:?}");
        let problem = &problems[0];
        (
            problem.message.clone(),
            sources.slice(problem.span).to_owned(),
        )
    }

    #[test]
    fn scalars_parse_to_their_kinds() {
        assert_eq!(ok("null").kind, NodeKind::Null);
        assert_eq!(ok(" true ").kind, NodeKind::Bool(true));
        assert_eq!(ok("false").kind, NodeKind::Bool(false));
        assert_eq!(ok("12").kind, NodeKind::Number(Num::Int(12)));
        assert_eq!(ok("-0").kind, NodeKind::Number(Num::Int(0)));
        assert_eq!(
            ok("18446744073709551615").kind,
            NodeKind::Number(Num::UInt(u64::MAX))
        );
        assert_eq!(ok("2.0e4").kind, NodeKind::Number(Num::Float(2.0e4)));
        assert_eq!(ok("-1.5E-3").kind, NodeKind::Number(Num::Float(-1.5e-3)));
        assert_eq!(
            ok("123456789012345678901234567890").kind,
            NodeKind::Number(Num::Float(1.234_567_890_123_456_8e29))
        );
        assert_eq!(
            ok(r#""a\tb\u00e9\ud83d\ude00\/""#).as_str(),
            Some("a\tbé😀/")
        );
    }

    #[test]
    fn every_value_and_key_keeps_its_span() {
        let text = "[\n  { \"id\": \"ash\", \"density\": 600 }\n]";
        let (tree, problems, sources) = parse(text);
        assert!(problems.is_empty());
        let tree = tree.unwrap();
        let object = &tree.as_array().unwrap()[0];
        let density = object.member("density").unwrap();
        assert_eq!(sources.slice(density.key_span), "\"density\"");
        assert_eq!(sources.slice(density.value.span), "600");
        let location = sources.locate(density.key_span);
        assert_eq!((location.line, location.column), (2, 18));
        assert_eq!(sources.slice(object.get("id").unwrap().span), "\"ash\"");
        assert_eq!(sources.slice(object.span), &text[4..text.len() - 2]);
    }

    #[test]
    fn comments_are_dropped_anywhere_and_key_order_is_kept() {
        let tree = ok(r#"{ "//": "one", "b": 1, "//": ["two"], "a": { "//": 3, "z": 2 } }"#);
        let keys: Vec<&str> = tree
            .as_object()
            .unwrap()
            .iter()
            .map(|member| member.key.as_str())
            .collect();
        assert_eq!(keys, ["b", "a"]);
        assert_eq!(tree.get("a").unwrap().as_object().unwrap().len(), 1);
    }

    #[test]
    fn a_repeated_key_is_reported_and_the_first_kept() {
        let (tree, problems, sources) = parse(r#"{ "a": 1, "a": 2 }"#);
        assert_eq!(problems.len(), 1);
        assert_eq!(problems[0].code, code::DUPLICATE_KEY);
        assert_eq!(sources.locate(problems[0].span).column, 11);
        let tree = tree.unwrap();
        assert_eq!(tree.get("a").unwrap().kind, NodeKind::Number(Num::Int(1)));
    }

    #[test]
    fn syntax_errors_say_what_and_where() {
        assert_eq!(
            error("[1, 2,]"),
            (
                "a comma must be followed by another value".to_owned(),
                ",".to_owned()
            )
        );
        assert_eq!(error("{ \"a\": 1 ").0, "this object is never closed");
        assert_eq!(error("[1 2]").0, "expected `,` or `]`, found `2`");
        assert_eq!(
            error("{ a: 1 }").0,
            "expected a key in quotation marks, found `a`"
        );
        assert_eq!(
            error("{ \"a\" 1 }").0,
            "expected `:` after the key, found `1`"
        );
        assert_eq!(error("// hi\n[]").0, "comments are not JSON");
        assert_eq!(error("['a']").0, "strings take double quotation marks");
        assert_eq!(
            error("[tru]"),
            ("expected a value, found `tru`".to_owned(), "tru".to_owned())
        );
        assert_eq!(error("\"abc").0, "this string is never closed");
        assert_eq!(error("\"a\\qb\"").0, "unknown escape in a string");
        assert!(error("\"\\ud83d\"").0.contains("half of a pair"));
        assert_eq!(error("01").0, "a number may not start with a zero");
        assert_eq!(error("1.").0, "expected digits after the decimal point");
        assert_eq!(
            error("250ms"),
            ("`250ms` is not a number".to_owned(), "250ms".to_owned())
        );
        assert_eq!(error("1e999").0, "this number is too large to represent");
        assert_eq!(error("[] []").0, "expected the end of the file, found `[`");
        assert_eq!(error("").0, "file is empty; expected a list of definitions");
        assert_eq!(
            error("\"a\tb\"").0,
            "a control character must be written as an escape"
        );
    }

    #[test]
    fn limits_are_diagnostics_not_crashes() {
        let mut sources = SourceMap::new();
        let deep = "[".repeat(100_000);
        let file = sources.add("deep.json", deep.as_str());
        let (tree, problems) = parse_json(file, &deep, &Limits::default());
        assert!(tree.is_none());
        assert_eq!(problems[0].code, code::NESTING_TOO_DEEP);

        let limits = Limits {
            max_file_bytes: 8,
            ..Limits::default()
        };
        let (tree, problems) = parse_json(file, "[1, 2, 3, 4]", &limits);
        assert!(tree.is_none());
        assert_eq!(problems[0].code, code::FILE_TOO_LARGE);
    }

    #[test]
    fn a_tree_writes_back_as_json() {
        let text = r#"{"a":[1,2.5,"x\ny",null,true],"b":{"c":-3}}"#;
        assert_eq!(ok(text).to_json(), text);
        assert_eq!(ok("2.0e4").to_json(), "20000.0");
        assert!(ok(text).same_value(&ok(&text.replace(":", " : "))));
        assert!(!ok("[1, 2]").same_value(&ok("[2, 1]")));
        assert!(ok("1").same_value(&ok("1.0")));
    }

    #[test]
    fn a_byte_order_mark_is_skipped() {
        assert_eq!(ok("\u{feff}[1]").as_array().unwrap().len(), 1);
    }

    #[test]
    fn mangled_input_is_reported_and_never_panics() {
        // Bad data is an error message, not a crash (01 R6). Every
        // truncation of a document, and the document with each of a set
        // of awkward characters put in at each position, must come back
        // as a tree or as located problems.
        let document = "[ { \"//\": \"é😀\", \"id\": \"a\\u00e9\\ud83d\\ude00\\n\", \"n\": [1, -2.5e-3, true, null], \"o\": {} } ]";
        let mut inputs: Vec<String> = Vec::new();
        for (cut, _) in document.char_indices() {
            inputs.push(document[..cut].to_owned());
            for awkward in [
                "\"", "\\", "\\u", "\\ud83d", "é", "😀", "\n", "\t", "{", "]", ",", "-", "e",
                "\u{0}",
            ] {
                inputs.push(format!("{}{awkward}{}", &document[..cut], &document[cut..]));
            }
        }
        for input in &inputs {
            let (tree, problems, sources) = parse(input);
            assert!(
                tree.is_some() || !problems.is_empty(),
                "{input:?} gave neither a tree nor a problem"
            );
            for problem in &problems {
                // A position can always be shown.
                let location = sources.locate(problem.span);
                assert!(location.line >= 1 && location.column >= 1, "{input:?}");
                assert!(!problem.message.is_empty());
            }
        }
        assert!(inputs.len() > 1000);
    }
}
