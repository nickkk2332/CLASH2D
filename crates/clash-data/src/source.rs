//! Source files and positions in them.
//!
//! Every value the loader reads remembers where it was written as a
//! [`Span`]: a file and a range of bytes. A [`SourceMap`] holds the text of
//! every file read during one load and turns a span into a [`Location`]
//! with a line and a column, which is what a diagnostic shows (04 §8.2).
//!
//! Lines and columns start at one. A column counts characters, not bytes,
//! so a caret drawn under a column lines up in an editor.

use serde::{Deserialize, Serialize};

/// One file known to a [`SourceMap`].
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug, Serialize, Deserialize)]
pub struct FileId(pub u32);

/// A range of bytes in one source file: where a value was written.
///
/// `start` is the first byte and `end` is one past the last. For a JSON
/// string the span includes both quotation marks.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug, Serialize, Deserialize)]
pub struct Span {
    /// The file.
    pub file: FileId,
    /// Offset of the first byte.
    pub start: u32,
    /// Offset one past the last byte.
    pub end: u32,
}

impl Span {
    /// A span in `file` from `start` to `end`.
    pub const fn new(file: FileId, start: u32, end: u32) -> Span {
        Span { file, start, end }
    }

    /// An empty span at the start of `file`: used for whole-file problems.
    pub const fn file_start(file: FileId) -> Span {
        Span {
            file,
            start: 0,
            end: 0,
        }
    }
}

/// A span resolved for showing: file path, line, column and the text of
/// the line it starts on.
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub struct Location {
    /// The file's path as it was given to the loader.
    pub file: String,
    /// Line of the first character, from one.
    pub line: u32,
    /// Column of the first character, from one, in characters.
    pub column: u32,
    /// Line of the last character.
    pub end_line: u32,
    /// Column one past the last character.
    pub end_column: u32,
    /// Byte offset of the first character.
    pub start: u32,
    /// Byte offset one past the last character.
    pub end: u32,
    /// The text of the line the span starts on, with tabs turned into
    /// single spaces so that a caret under `column` lines up.
    pub source_line: String,
}

impl Location {
    /// `file:line:column`, the form a terminal or an editor can jump to.
    pub fn short(&self) -> String {
        format!("{}:{}:{}", self.file, self.line, self.column)
    }
}

#[derive(Debug, Clone)]
struct SourceFile {
    path: String,
    text: String,
    /// Byte offset of the start of each line.
    line_starts: Vec<u32>,
}

/// The text of every file read during one load.
///
/// Files are added in the order the loader reads them, which is a fixed
/// order (M1), so a [`FileId`] means the same file on every run over the
/// same mods.
#[derive(Debug, Clone, Default)]
pub struct SourceMap {
    files: Vec<SourceFile>,
}

impl SourceMap {
    /// An empty map.
    pub fn new() -> SourceMap {
        SourceMap::default()
    }

    /// Adds a file and returns its id. `path` is shown in diagnostics as
    /// given.
    pub fn add(&mut self, path: impl Into<String>, text: impl Into<String>) -> FileId {
        let text = text.into();
        let mut line_starts = vec![0_u32];
        for (offset, byte) in text.bytes().enumerate() {
            if byte == b'\n' {
                line_starts.push(to_u32(offset + 1));
            }
        }
        let id = FileId(to_u32(self.files.len()));
        self.files.push(SourceFile {
            path: path.into(),
            text,
            line_starts,
        });
        id
    }

    /// How many files the map holds.
    pub fn len(&self) -> usize {
        self.files.len()
    }

    /// Whether the map holds no files.
    pub fn is_empty(&self) -> bool {
        self.files.is_empty()
    }

    /// The path a file was added under, or `""` for an id from another map.
    pub fn path(&self, file: FileId) -> &str {
        self.files
            .get(file.0 as usize)
            .map_or("", |file| file.path.as_str())
    }

    /// The text of a file, or `""` for an id from another map.
    pub fn text(&self, file: FileId) -> &str {
        self.files
            .get(file.0 as usize)
            .map_or("", |file| file.text.as_str())
    }

    /// The first file added under `path`.
    pub fn file_by_path(&self, path: &str) -> Option<FileId> {
        self.files
            .iter()
            .position(|file| file.path == path)
            .map(|index| FileId(to_u32(index)))
    }

    /// The text a span covers, or `""` if the span is not inside its file.
    pub fn slice(&self, span: Span) -> &str {
        self.text(span.file)
            .get(span.start as usize..span.end as usize)
            .unwrap_or("")
    }

    /// The line and column of a byte offset, both from one. An offset past
    /// the end of the file gives the position just after its last
    /// character.
    pub fn line_col(&self, file: FileId, offset: u32) -> (u32, u32) {
        let Some(source) = self.files.get(file.0 as usize) else {
            return (1, 1);
        };
        let offset = offset.min(to_u32(source.text.len()));
        let line_index = match source.line_starts.binary_search(&offset) {
            Ok(index) => index,
            Err(index) => index.saturating_sub(1),
        };
        let line_start = source.line_starts[line_index] as usize;
        let mut end = offset as usize;
        while end > line_start && !source.text.is_char_boundary(end) {
            end -= 1;
        }
        let column = source.text[line_start..end].chars().count();
        (to_u32(line_index + 1), to_u32(column + 1))
    }

    /// Resolves a span for showing.
    pub fn locate(&self, span: Span) -> Location {
        let (line, column) = self.line_col(span.file, span.start);
        let (end_line, end_column) = self.line_col(span.file, span.end.max(span.start));
        Location {
            file: self.path(span.file).to_owned(),
            line,
            column,
            end_line,
            end_column,
            start: span.start,
            end: span.end,
            source_line: self.line_text(span.file, line).replace('\t', " "),
        }
    }

    /// The text of a line, numbered from one, without its line ending.
    pub fn line_text(&self, file: FileId, line: u32) -> &str {
        let Some(source) = self.files.get(file.0 as usize) else {
            return "";
        };
        let Some(index) = (line as usize).checked_sub(1) else {
            return "";
        };
        let Some(&start) = source.line_starts.get(index) else {
            return "";
        };
        let end = source
            .line_starts
            .get(index + 1)
            .map_or(source.text.len(), |&next| next as usize);
        source.text[start as usize..end].trim_end_matches(['\n', '\r'])
    }

    /// Maps a range of bytes inside the *decoded* content of a JSON string
    /// back to the source.
    ///
    /// `string` is the span of a JSON string value or key, quotation marks
    /// included, as the parser recorded it. `from` and `to` are byte
    /// offsets into the string's content after its escapes were decoded,
    /// which is the text an expression compiler sees. The result is the
    /// span in the file that those bytes were written as, with every
    /// escape (`\n`, `\"`, `é`, a surrogate pair) accounted for.
    ///
    /// An offset that falls inside a character written as an escape maps
    /// to the start of the escape; an offset past the end maps to the
    /// closing quotation mark. If `string` does not cover a string literal
    /// (the value was computed, not written), it is returned unchanged.
    ///
    /// ```
    /// use clash_data::{parse_json, Limits, SourceMap};
    ///
    /// let mut sources = SourceMap::new();
    /// let file = sources.add("rule.json", r#"{ "when": "a \"b\" + c" }"#);
    /// let (tree, _) = parse_json(file, sources.text(file), &Limits::default());
    /// let when = tree.unwrap().get("when").unwrap().clone();
    /// assert_eq!(when.as_str(), Some(r#"a "b" + c"#));
    /// // The decoded text has `c` at byte 8; in the file it is further on.
    /// let span = sources.span_in_string(when.span, 8, 9);
    /// assert_eq!(sources.slice(span), "c");
    /// assert_eq!(sources.locate(span).column, 22);
    /// ```
    pub fn span_in_string(&self, string: Span, from: usize, to: usize) -> Span {
        let raw = self.slice(string);
        let bytes = raw.as_bytes();
        if bytes.len() < 2 || bytes[0] != b'"' || bytes[bytes.len() - 1] != b'"' {
            return string;
        }
        let to = to.max(from);
        let close = bytes.len() - 1;
        let mut raw_at = 1_usize;
        let mut decoded_at = 0_usize;
        let mut start = None;
        let mut end = None;
        while raw_at < close {
            let (raw_len, decoded_len) = unit_at(raw, raw_at, close);
            if start.is_none() && from < decoded_at + decoded_len {
                start = Some(raw_at);
            }
            if end.is_none() && to <= decoded_at {
                end = Some(raw_at);
            }
            raw_at += raw_len;
            decoded_at += decoded_len;
        }
        let start = start.unwrap_or(close);
        let end = end.unwrap_or(close).max(start);
        Span {
            file: string.file,
            start: string.start + to_u32(start),
            end: string.start + to_u32(end),
        }
    }

    /// Like [`span_in_string`](SourceMap::span_in_string), resolved for
    /// showing: the line and column of an error at bytes `from..to` of an
    /// expression held in the JSON string at `string`.
    pub fn locate_in_string(&self, string: Span, from: usize, to: usize) -> Location {
        self.locate(self.span_in_string(string, from, to))
    }
}

/// The length in the source and in the decoded text of the character or
/// escape that starts at `at` inside a string literal.
fn unit_at(raw: &str, at: usize, close: usize) -> (usize, usize) {
    let bytes = raw.as_bytes();
    if bytes[at] != b'\\' {
        let len = raw[at..].chars().next().map_or(1, char::len_utf8);
        return (len, len);
    }
    if at + 1 >= close {
        return (1, 1);
    }
    if bytes[at + 1] != b'u' {
        return (2, 1);
    }
    let Some(first) = hex4(raw, at + 2) else {
        return (2, 1);
    };
    if (0xD800..0xDC00).contains(&first)
        && raw.get(at + 6..at + 8) == Some("\\u")
        && let Some(second) = hex4(raw, at + 8)
        && (0xDC00..0xE000).contains(&second)
    {
        return (12, 4);
    }
    let decoded = char::from_u32(first).map_or(3, char::len_utf8);
    (6, decoded)
}

fn hex4(raw: &str, at: usize) -> Option<u32> {
    let digits = raw.get(at..at + 4)?;
    u32::from_str_radix(digits, 16).ok()
}

/// Narrows a length to `u32`, saturating. Files are limited to far less
/// than 4 GiB before they are read ([`Limits`](crate::Limits)).
pub(crate) fn to_u32(value: usize) -> u32 {
    u32::try_from(value).unwrap_or(u32::MAX)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lines_and_columns_count_from_one_in_characters() {
        let mut sources = SourceMap::new();
        let file = sources.add("a.json", "ab\ncdé f\n\nlast");
        assert_eq!(sources.line_col(file, 0), (1, 1));
        assert_eq!(sources.line_col(file, 2), (1, 3));
        assert_eq!(sources.line_col(file, 3), (2, 1));
        // `é` is two bytes and one column.
        assert_eq!(sources.line_col(file, 8), (2, 5));
        assert_eq!(sources.line_col(file, 10), (3, 1));
        assert_eq!(sources.line_col(file, 11), (4, 1));
        assert_eq!(sources.line_col(file, 999), (4, 5));
        assert_eq!(sources.line_text(file, 2), "cdé f");
        assert_eq!(sources.line_text(file, 3), "");
        assert_eq!(sources.line_text(file, 9), "");
    }

    #[test]
    fn a_location_carries_the_line_it_starts_on() {
        let mut sources = SourceMap::new();
        let file = sources.add("mods/x/data/a.json", "[\n\t{ \"id\": 1 }\n]");
        let location = sources.locate(Span::new(file, 5, 9));
        assert_eq!(location.short(), "mods/x/data/a.json:2:4");
        assert_eq!(location.source_line, " { \"id\": 1 }");
        assert_eq!((location.end_line, location.end_column), (2, 8));
        assert_eq!(sources.file_by_path("mods/x/data/a.json"), Some(file));
        assert_eq!(sources.file_by_path("nope"), None);
    }

    fn string_span(sources: &SourceMap, file: FileId) -> Span {
        let text = sources.text(file);
        let start = text.find('"').unwrap();
        let end = text.rfind('"').unwrap() + 1;
        Span::new(file, to_u32(start), to_u32(end))
    }

    #[test]
    fn offsets_in_a_plain_string_shift_by_the_opening_quote() {
        let mut sources = SourceMap::new();
        let file = sources.add("e.json", r#"  "hp > 3"  "#);
        let string = string_span(&sources, file);
        let span = sources.span_in_string(string, 3, 4);
        assert_eq!(sources.slice(span), ">");
        assert_eq!(sources.locate(span).column, 7);
    }

    #[test]
    fn offsets_account_for_escapes() {
        let mut sources = SourceMap::new();
        // Decoded: `a\n"é😀" x` where é is written é and the emoji
        // as a surrogate pair.
        let file = sources.add("e.json", r#""a\n\"é😀\" x""#);
        let string = string_span(&sources, file);
        let decoded = "a\n\"é😀\" x";
        let x = decoded.find('x').unwrap();
        let span = sources.span_in_string(string, x, x + 1);
        assert_eq!(sources.slice(span), "x");
        // The emoji: four decoded bytes written as twelve.
        let emoji = decoded.find('😀').unwrap();
        let span = sources.span_in_string(string, emoji, emoji + 4);
        assert_eq!(sources.slice(span), r"😀");
        // An offset inside it maps to the start of the escape.
        let inside = sources.span_in_string(string, emoji + 2, emoji + 2);
        assert_eq!(inside.start, span.start);
        // The newline escape.
        let span = sources.span_in_string(string, 1, 2);
        assert_eq!(sources.slice(span), r"\n");
    }

    #[test]
    fn offsets_past_the_end_point_at_the_closing_quote() {
        let mut sources = SourceMap::new();
        let file = sources.add("e.json", r#""abc""#);
        let string = string_span(&sources, file);
        let span = sources.span_in_string(string, 3, 3);
        assert_eq!((span.start, span.end), (4, 4));
        let span = sources.span_in_string(string, 50, 60);
        assert_eq!((span.start, span.end), (4, 4));
    }

    #[test]
    fn a_span_that_is_not_a_string_is_returned_unchanged() {
        let mut sources = SourceMap::new();
        let file = sources.add("e.json", "12345");
        let span = Span::new(file, 0, 5);
        assert_eq!(sources.span_in_string(span, 1, 2), span);
    }
}
