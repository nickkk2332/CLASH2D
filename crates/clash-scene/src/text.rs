//! Styled text: inline style tags (12 §4.3), wrapping and drawing.
//!
//! In any text, `<name>…</>` applies a theme style or a colour to what it
//! encloses, tags may nest, and `<<` is a literal angle bracket:
//!
//! ```
//! use clash_scene::{RichText, Theme};
//!
//! let theme = Theme::default();
//! let text = RichText::parse("<red>You are <bright_red>bleeding</>.</> 3 << 4", &theme).unwrap();
//! assert_eq!(text.plain(), "You are bleeding. 3 < 4");
//! assert!(RichText::parse("<octarine>no such colour</>", &theme).is_err());
//! ```
//!
//! An unknown name is an error, reported when the definition that holds
//! the text is loaded. Templates in braces (05 §4.6) are not handled
//! here: a text is styled after its templates have been filled in.

use thiserror::Error;

use crate::frame::{Canvas, CellStyle};
use crate::theme::{Style, Theme};

/// Why a text's inline style tags are wrong.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[non_exhaustive]
pub enum TextError {
    /// A tag names neither a theme style nor a colour.
    #[error("unknown style or colour `{name}` in a tag")]
    UnknownStyle {
        /// The name in the tag.
        name: String,
    },
    /// A `<` that does not begin a tag.
    #[error("`<` at character {at} does not begin a tag; write `<<` for a literal `<`")]
    StrayBracket {
        /// The position of the `<`, counted in characters from 0.
        at: usize,
    },
    /// A `</>` with no tag open.
    #[error("`</>` at character {at} closes nothing")]
    NothingToClose {
        /// The position of the `</>`, counted in characters from 0.
        at: usize,
    },
    /// A tag opened and never closed.
    #[error("the tag `<{name}>` is never closed with `</>`")]
    Unclosed {
        /// The name in the tag.
        name: String,
    },
    /// More different styles in one text than can be held.
    #[error("a text may use at most {MAX_STYLES} different styles")]
    TooManyStyles,
}

/// How many different combinations of styles one text may use.
pub const MAX_STYLES: usize = 255;

/// Text with its inline styles resolved: plain characters, and for each
/// one the style it is drawn in.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct RichText {
    chars: Vec<char>,
    /// For each character, an index into `styles`.
    style_of: Vec<u8>,
    /// The styles in use. Entry 0 changes nothing.
    styles: Vec<Style>,
}

/// One tag found in a text, for checking names without a theme.
enum Token {
    Char(char),
    Open(String),
    Close,
}

fn tokens(source: &str) -> Result<Vec<(usize, Token)>, TextError> {
    let chars: Vec<char> = source.chars().collect();
    let mut out = Vec::with_capacity(chars.len());
    let mut at = 0;
    while at < chars.len() {
        let ch = chars[at];
        if ch != '<' {
            out.push((at, Token::Char(ch)));
            at += 1;
            continue;
        }
        if chars.get(at + 1) == Some(&'<') {
            out.push((at, Token::Char('<')));
            at += 2;
            continue;
        }
        if chars.get(at + 1) == Some(&'/') && chars.get(at + 2) == Some(&'>') {
            out.push((at, Token::Close));
            at += 3;
            continue;
        }
        let name_len = chars[at + 1..]
            .iter()
            .take_while(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | ':' | '.' | '-'))
            .count();
        if name_len == 0 || chars.get(at + 1 + name_len) != Some(&'>') {
            return Err(TextError::StrayBracket { at });
        }
        let name: String = chars[at + 1..at + 1 + name_len].iter().collect();
        out.push((at, Token::Open(name)));
        at += name_len + 2;
    }
    Ok(out)
}

/// The names used by the tags of a text, in order, or what is wrong with
/// its tags. Needs no theme, so it can run before one is chosen.
pub fn tag_names(source: &str) -> Result<Vec<String>, TextError> {
    let mut names = Vec::new();
    let mut open: Vec<String> = Vec::new();
    for (at, token) in tokens(source)? {
        match token {
            Token::Char(_) => {}
            Token::Open(name) => {
                names.push(name.clone());
                open.push(name);
            }
            Token::Close => {
                open.pop().ok_or(TextError::NothingToClose { at })?;
            }
        }
    }
    match open.pop() {
        Some(name) => Err(TextError::Unclosed { name }),
        None => Ok(names),
    }
}

impl RichText {
    /// Text with no styles at all. Angle brackets are taken literally.
    pub fn plain_text(text: &str) -> RichText {
        let chars: Vec<char> = text.chars().collect();
        RichText {
            style_of: vec![0; chars.len()],
            chars,
            styles: vec![Style::default()],
        }
    }

    /// Reads a text with inline style tags, resolving names in `theme`.
    pub fn parse(source: &str, theme: &Theme) -> Result<RichText, TextError> {
        let mut text = RichText {
            chars: Vec::with_capacity(source.len()),
            style_of: Vec::with_capacity(source.len()),
            styles: vec![Style::default()],
        };
        // The stack of open tags: each entry is the name and the index of
        // the combined style in force inside it.
        let mut open: Vec<(String, u8)> = Vec::new();
        for (at, token) in tokens(source)? {
            match token {
                Token::Char(ch) => {
                    text.chars.push(ch);
                    text.style_of
                        .push(open.last().map_or(0, |(_, index)| *index));
                }
                Token::Open(name) => {
                    let style = theme
                        .style(&name)
                        .ok_or_else(|| TextError::UnknownStyle { name: name.clone() })?;
                    let outer =
                        text.styles[usize::from(open.last().map_or(0, |(_, index)| *index))];
                    let combined = Style {
                        fg: style.fg.or(outer.fg),
                        bg: style.bg.or(outer.bg),
                        set: (outer.set - style.clear) | style.set,
                        clear: (outer.clear - style.set) | style.clear,
                    };
                    let index = match text.styles.iter().position(|s| *s == combined) {
                        Some(index) => index,
                        None => {
                            if text.styles.len() >= MAX_STYLES {
                                return Err(TextError::TooManyStyles);
                            }
                            text.styles.push(combined);
                            text.styles.len() - 1
                        }
                    };
                    open.push((name, index as u8));
                }
                Token::Close => {
                    open.pop().ok_or(TextError::NothingToClose { at })?;
                }
            }
        }
        match open.pop() {
            Some((name, _)) => Err(TextError::Unclosed { name }),
            None => Ok(text),
        }
    }

    /// Gives the text a capital first letter, as a finished line has
    /// (05 §4.6). A first character that is not a letter with one
    /// capital form is left as it is.
    pub fn capitalize_first(&mut self) {
        if let Some(first) = self.chars.first_mut() {
            let mut upper = first.to_uppercase();
            if let (Some(capital), None) = (upper.next(), upper.next()) {
                *first = capital;
            }
        }
    }

    /// How many characters the text has.
    pub fn len(&self) -> usize {
        self.chars.len()
    }

    /// Whether the text has no characters.
    pub fn is_empty(&self) -> bool {
        self.chars.is_empty()
    }

    /// The characters without their styles.
    pub fn plain(&self) -> String {
        self.chars.iter().collect()
    }

    /// The characters.
    pub fn chars(&self) -> &[char] {
        &self.chars
    }

    /// The style of the character at an index, over `base`.
    pub fn style_at(&self, index: usize, base: CellStyle) -> CellStyle {
        match self.style_of.get(index) {
            Some(style) => self.styles[usize::from(*style)].over(base),
            None => base,
        }
    }

    /// The length of the longest line, in characters.
    pub fn width(&self) -> usize {
        self.chars
            .split(|c| *c == '\n')
            .map(<[char]>::len)
            .max()
            .unwrap_or(0)
    }

    /// Breaks the text into lines no wider than `width`, at spaces where
    /// it can and inside a word where it must, and at every line break in
    /// the text. Each line is a range of character indices, without the
    /// space or line break it was broken at. `out` is cleared first, so a
    /// caller can keep one buffer.
    pub fn wrap(&self, width: usize, out: &mut Vec<(usize, usize)>) {
        out.clear();
        self.wrap_each(width, |line| out.push(line));
    }

    /// [`RichText::wrap`] without a buffer: `each` is given every line in
    /// order, as a range of character indices.
    pub fn wrap_each(&self, width: usize, mut each: impl FnMut((usize, usize))) {
        let width = width.max(1);
        let mut line_start = 0;
        loop {
            let line_end = self.chars[line_start..]
                .iter()
                .position(|c| *c == '\n')
                .map_or(self.chars.len(), |offset| line_start + offset);
            let mut start = line_start;
            loop {
                if line_end - start <= width {
                    each((start, line_end));
                    break;
                }
                // The last space at which the line still fits.
                let limit = start + width;
                let break_at = (start + 1..=limit).rev().find(|i| self.chars[*i] == ' ');
                match break_at {
                    Some(space) => {
                        each((start, space));
                        start = space + 1;
                    }
                    None => {
                        each((start, limit));
                        start = limit;
                    }
                }
            }
            if line_end >= self.chars.len() {
                break;
            }
            line_start = line_end + 1;
        }
    }

    /// Draws the characters `from..to` rightward from a cell, over
    /// `base`, and returns how many cells they cover.
    pub fn draw(
        &self,
        canvas: &mut Canvas<'_>,
        x: i32,
        y: i32,
        (from, to): (usize, usize),
        base: CellStyle,
    ) -> i32 {
        let to = to.min(self.chars.len());
        let mut column = x;
        for index in from..to {
            canvas.put(column, y, self.chars[index], self.style_at(index, base));
            column += 1;
        }
        column - x
    }

    /// Draws the whole text on one row, cut off after `max` cells.
    pub fn draw_line(
        &self,
        canvas: &mut Canvas<'_>,
        x: i32,
        y: i32,
        max: i32,
        base: CellStyle,
    ) -> i32 {
        let end = self.chars.len().min(usize::try_from(max).unwrap_or(0));
        self.draw(canvas, x, y, (0, end), base)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::color::{Ansi16, Color};
    use crate::frame::{Attrs, Frame};
    use crate::theme::ThemeDef;
    use std::collections::BTreeMap;

    fn theme() -> Theme {
        let def: ThemeDef = serde_json::from_str(
            r##"{ "colors": { "blood": "#b02020" },
                 "styles": { "title": { "fg": "yellow", "bold": true }, "soft": { "bold": false, "bg": "blue" } } }"##,
        )
        .unwrap();
        Theme::build(&def, &BTreeMap::new()).unwrap()
    }

    #[test]
    fn tags_style_what_they_enclose_and_nest() {
        let theme = theme();
        let text = RichText::parse("a<title>b<blood>c</>d<soft>e</></>f", &theme).unwrap();
        assert_eq!(text.plain(), "abcdef");
        let base = CellStyle::default();
        let style = |i| text.style_at(i, base);
        assert_eq!(style(0), base);
        assert_eq!(style(1).fg, Color::ansi(Ansi16::Yellow));
        assert!(style(1).attrs.contains(Attrs::BOLD));
        assert_eq!(style(2).fg, theme.color("blood").unwrap());
        assert!(
            style(2).attrs.contains(Attrs::BOLD),
            "the outer style still applies"
        );
        assert_eq!(style(3), style(1));
        assert_eq!(style(4).bg, Color::ansi(Ansi16::Blue));
        assert!(
            !style(4).attrs.contains(Attrs::BOLD),
            "an inner style can turn bold off"
        );
        assert_eq!(style(5), base);
    }

    #[test]
    fn a_doubled_bracket_is_literal() {
        let text = RichText::parse("1 << 2 > 0 <<red>", &Theme::default()).unwrap();
        assert_eq!(text.plain(), "1 < 2 > 0 <red>");
    }

    #[test]
    fn bad_tags_are_errors() {
        let theme = Theme::default();
        let parse = |s: &str| RichText::parse(s, &theme).unwrap_err();
        assert_eq!(
            parse("<octarine>x</>"),
            TextError::UnknownStyle {
                name: "octarine".into()
            }
        );
        assert_eq!(parse("a < b"), TextError::StrayBracket { at: 2 });
        assert_eq!(parse("x<red"), TextError::StrayBracket { at: 1 });
        assert_eq!(parse("x</>"), TextError::NothingToClose { at: 1 });
        assert_eq!(parse("<red>x"), TextError::Unclosed { name: "red".into() });
        assert_eq!(
            tag_names("<a>x<b>y</></>"),
            Ok(vec!["a".to_owned(), "b".to_owned()])
        );
        assert_eq!(
            tag_names("<a>x"),
            Err(TextError::Unclosed { name: "a".into() })
        );
    }

    fn wrapped(source: &str, width: usize) -> Vec<String> {
        let text = RichText::plain_text(source);
        let mut lines = Vec::new();
        text.wrap(width, &mut lines);
        lines
            .into_iter()
            .map(|(from, to)| text.chars()[from..to].iter().collect())
            .collect()
    }

    #[test]
    fn wrapping_breaks_at_spaces_then_inside_words() {
        assert_eq!(
            wrapped("the quick brown fox", 10),
            ["the quick", "brown fox"]
        );
        assert_eq!(
            wrapped("the quick brown fox", 9),
            ["the quick", "brown fox"]
        );
        assert_eq!(
            wrapped("the quick brown fox", 5),
            ["the", "quick", "brown", "fox"]
        );
        assert_eq!(wrapped("abcdefghij", 4), ["abcd", "efgh", "ij"]);
        assert_eq!(wrapped("one\n\ntwo", 10), ["one", "", "two"]);
        assert_eq!(wrapped("", 10), [""]);
        assert_eq!(wrapped("ab", 0), ["a", "b"]);
        assert_eq!(RichText::plain_text("ab\ncdef\ng").width(), 4);
    }

    #[test]
    fn drawing_puts_each_character_in_its_style() {
        let theme = theme();
        let text = RichText::parse("ab<blood>cd</>", &theme).unwrap();
        let mut frame = Frame::new(6, 1);
        let covered = text.draw_line(&mut Canvas::new(&mut frame), 1, 0, 3, CellStyle::default());
        assert_eq!(covered, 3);
        assert_eq!(frame.row_text(0), " abc  ");
        assert_eq!(frame.get(3, 0).unwrap().fg, theme.color("blood").unwrap());
        assert_eq!(frame.get(1, 0).unwrap().fg, CellStyle::default().fg);
    }
}
