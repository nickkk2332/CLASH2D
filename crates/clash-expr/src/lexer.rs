//! The lexer: source text to tokens, one at a time.
//!
//! The lexer is lazy. The parser asks for the next token from a byte
//! position, which is what lets a text template switch between literal
//! text and expressions without scanning anything twice.

use clash_core::Real;

use crate::ast::{Name, Number};
use crate::diag::{Diagnostic, Span, codes};

/// A token.
#[derive(Clone, PartialEq, Debug)]
pub(crate) enum Tok {
    /// A number.
    Number(Number),
    /// Letters written directly after a number: `ms` in `300ms`.
    Unit(String),
    /// A text in single quotes, with doubled quotes undone.
    Str(String),
    /// A name, with its namespace if it has one.
    Name(Name),
    Dollar,
    LParen,
    RParen,
    LBracket,
    RBracket,
    LBrace,
    RBrace,
    Comma,
    Dot,
    Colon,
    Plus,
    Minus,
    Star,
    Slash,
    Percent,
    Caret,
    EqEq,
    NotEq,
    Lt,
    Le,
    Gt,
    Ge,
    Coalesce,
    If,
    Then,
    Else,
    And,
    Or,
    Not,
    In,
    True,
    False,
    Null,
    /// The end of the source.
    End,
}

impl Tok {
    /// The token as a person would name it in a message.
    pub(crate) fn describe(&self) -> String {
        let fixed = match self {
            Tok::Number(_) => "a number",
            Tok::Unit(unit) => return format!("unit `{unit}`"),
            Tok::Str(_) => "a text",
            Tok::Name(name) => return format!("`{name}`"),
            Tok::Dollar => "`$`",
            Tok::LParen => "`(`",
            Tok::RParen => "`)`",
            Tok::LBracket => "`[`",
            Tok::RBracket => "`]`",
            Tok::LBrace => "`{`",
            Tok::RBrace => "`}`",
            Tok::Comma => "`,`",
            Tok::Dot => "`.`",
            Tok::Colon => "`:`",
            Tok::Plus => "`+`",
            Tok::Minus => "`-`",
            Tok::Star => "`*`",
            Tok::Slash => "`/`",
            Tok::Percent => "`%`",
            Tok::Caret => "`^`",
            Tok::EqEq => "`==`",
            Tok::NotEq => "`!=`",
            Tok::Lt => "`<`",
            Tok::Le => "`<=`",
            Tok::Gt => "`>`",
            Tok::Ge => "`>=`",
            Tok::Coalesce => "`??`",
            Tok::If => "`if`",
            Tok::Then => "`then`",
            Tok::Else => "`else`",
            Tok::And => "`and`",
            Tok::Or => "`or`",
            Tok::Not => "`not`",
            Tok::In => "`in`",
            Tok::True => "`true`",
            Tok::False => "`false`",
            Tok::Null => "`null`",
            Tok::End => "the end of the expression",
        };
        fixed.to_owned()
    }
}

/// A token and where it is.
#[derive(Clone, PartialEq, Debug)]
pub(crate) struct Token {
    pub(crate) tok: Tok,
    pub(crate) span: Span,
}

/// The words that are not names.
fn keyword(word: &str) -> Option<Tok> {
    Some(match word {
        "if" => Tok::If,
        "then" => Tok::Then,
        "else" => Tok::Else,
        "and" => Tok::And,
        "or" => Tok::Or,
        "not" => Tok::Not,
        "in" => Tok::In,
        "true" => Tok::True,
        "false" => Tok::False,
        "null" => Tok::Null,
        _ => return None,
    })
}

/// Whether a word is reserved by the expression language and so cannot be
/// the name of a subject, a local, a property or a function.
///
/// ```
/// use clash_expr::is_keyword;
///
/// assert!(is_keyword("then"));
/// assert!(!is_keyword("than"));
/// ```
pub fn is_keyword(word: &str) -> bool {
    keyword(word).is_some()
}

const fn is_ident_start(byte: u8) -> bool {
    byte.is_ascii_alphabetic() || byte == b'_'
}

const fn is_ident_continue(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || byte == b'_'
}

/// A span from byte positions, which never exceed `u32` because the
/// lexer refuses longer sources.
fn span(start: usize, end: usize) -> Span {
    Span::new(
        u32::try_from(start).unwrap_or(u32::MAX),
        u32::try_from(end).unwrap_or(u32::MAX),
    )
}

/// Reads tokens from a source text.
pub(crate) struct Lexer<'s> {
    source: &'s str,
    bytes: &'s [u8],
    /// Whether the previous token was a number, so that letters directly
    /// after it are a unit.
    after_number: bool,
}

impl<'s> Lexer<'s> {
    pub(crate) fn new(source: &'s str) -> Lexer<'s> {
        Lexer {
            source,
            bytes: source.as_bytes(),
            after_number: false,
        }
    }

    pub(crate) fn source(&self) -> &'s str {
        self.source
    }

    fn byte(&self, at: usize) -> Option<u8> {
        self.bytes.get(at).copied()
    }

    /// The end of the run of identifier characters starting at `at`.
    fn ident_end(&self, mut at: usize) -> usize {
        while self.byte(at).is_some_and(is_ident_continue) {
            at += 1;
        }
        at
    }

    /// The token that starts at or after byte `from`, skipping whitespace.
    pub(crate) fn next(&mut self, from: usize) -> Result<Token, Diagnostic> {
        let mut at = from;
        while self.byte(at).is_some_and(|byte| byte.is_ascii_whitespace()) {
            at += 1;
        }
        let adjacent_to_number = self.after_number && at == from;
        self.after_number = false;
        let Some(first) = self.byte(at) else {
            return Ok(Token {
                tok: Tok::End,
                span: span(self.bytes.len(), self.bytes.len()),
            });
        };
        if first.is_ascii_digit() {
            self.after_number = true;
            return self.number(at);
        }
        if is_ident_start(first) {
            let end = self.ident_end(at);
            let word = &self.source[at..end];
            if adjacent_to_number {
                return Ok(Token {
                    tok: Tok::Unit(word.to_owned()),
                    span: span(at, end),
                });
            }
            // `namespace:name` is one token when nothing separates its
            // parts. A colon followed by anything else is a colon.
            if self.byte(end) == Some(b':') && self.byte(end + 1).is_some_and(is_ident_start) {
                let name_end = self.ident_end(end + 1);
                return Ok(Token {
                    tok: Tok::Name(Name::qualified(word, &self.source[end + 1..name_end])),
                    span: span(at, name_end),
                });
            }
            let tok = keyword(word).unwrap_or_else(|| Tok::Name(Name::bare(word)));
            return Ok(Token {
                tok,
                span: span(at, end),
            });
        }
        if first == b'\'' {
            return self.string(at);
        }
        let second = self.byte(at + 1);
        let (tok, len) = match (first, second) {
            (b'=', Some(b'=')) => (Tok::EqEq, 2),
            (b'!', Some(b'=')) => (Tok::NotEq, 2),
            (b'<', Some(b'=')) => (Tok::Le, 2),
            (b'>', Some(b'=')) => (Tok::Ge, 2),
            (b'?', Some(b'?')) => (Tok::Coalesce, 2),
            (b'<', _) => (Tok::Lt, 1),
            (b'>', _) => (Tok::Gt, 1),
            (b'$', _) => (Tok::Dollar, 1),
            (b'(', _) => (Tok::LParen, 1),
            (b')', _) => (Tok::RParen, 1),
            (b'[', _) => (Tok::LBracket, 1),
            (b']', _) => (Tok::RBracket, 1),
            (b'{', _) => (Tok::LBrace, 1),
            (b'}', _) => (Tok::RBrace, 1),
            (b',', _) => (Tok::Comma, 1),
            (b'.', _) => (Tok::Dot, 1),
            (b':', _) => (Tok::Colon, 1),
            (b'+', _) => (Tok::Plus, 1),
            (b'-', _) => (Tok::Minus, 1),
            (b'*', _) => (Tok::Star, 1),
            (b'/', _) => (Tok::Slash, 1),
            (b'%', _) => (Tok::Percent, 1),
            (b'^', _) => (Tok::Caret, 1),
            _ => return Err(self.unexpected_character(at)),
        };
        Ok(Token {
            tok,
            span: span(at, at + len),
        })
    }

    fn unexpected_character(&self, at: usize) -> Diagnostic {
        let character = self
            .source
            .get(at..)
            .and_then(|rest| rest.chars().next())
            .unwrap_or(' ');
        let here = span(at, at + character.len_utf8());
        let error = Diagnostic::error(
            codes::UNEXPECTED_CHARACTER,
            here,
            format!("unexpected character `{character}`"),
        );
        match character {
            '"' => error.with_suggestion("texts are written in single quotes: 'ash'"),
            '=' => error.with_suggestion("equality is written `==`"),
            '!' => error.with_suggestion("negation is written `not`"),
            '&' => error.with_suggestion("write `and`"),
            '|' => error.with_suggestion("write `or`"),
            '?' => error.with_suggestion("the null-coalescing operator is written `??`"),
            _ => error,
        }
    }

    /// A number: digits, then optionally a point and digits, then
    /// optionally an exponent.
    fn number(&self, start: usize) -> Result<Token, Diagnostic> {
        let digits_end = |mut at: usize| {
            while self.byte(at).is_some_and(|byte| byte.is_ascii_digit()) {
                at += 1;
            }
            at
        };
        let mut end = digits_end(start);
        let mut is_real = false;
        if self.byte(end) == Some(b'.') && self.byte(end + 1).is_some_and(|b| b.is_ascii_digit()) {
            is_real = true;
            end = digits_end(end + 1);
        }
        if matches!(self.byte(end), Some(b'e' | b'E')) {
            let mut exponent = end + 1;
            if matches!(self.byte(exponent), Some(b'+' | b'-')) {
                exponent += 1;
            }
            if self
                .byte(exponent)
                .is_some_and(|byte| byte.is_ascii_digit())
            {
                is_real = true;
                end = digits_end(exponent);
            }
        }
        let text = &self.source[start..end];
        let here = span(start, end);
        let number = if is_real {
            match text.parse::<f64>() {
                Ok(value) if value.is_finite() => Number::Real(Real::of(value)),
                _ => {
                    return Err(Diagnostic::error(
                        codes::BAD_NUMBER,
                        here,
                        format!("the number `{text}` is too large"),
                    ));
                }
            }
        } else {
            match text.parse::<i64>() {
                Ok(value) => Number::Int(value),
                Err(_) => {
                    return Err(Diagnostic::error(
                        codes::BAD_NUMBER,
                        here,
                        format!("the integer `{text}` is too large"),
                    )
                    .with_suggestion("write it with an exponent to make it a real: 1e19"));
                }
            }
        };
        Ok(Token {
            tok: Tok::Number(number),
            span: here,
        })
    }

    /// A text in single quotes. A doubled quote inside it is one quote.
    fn string(&self, start: usize) -> Result<Token, Diagnostic> {
        let mut text = String::new();
        let mut at = start + 1;
        let mut run = at;
        loop {
            match self.byte(at) {
                None => {
                    return Err(Diagnostic::error(
                        codes::UNTERMINATED_STRING,
                        span(start, self.bytes.len()),
                        "this text has no closing quote",
                    )
                    .with_suggestion("a quote inside a text is written twice: 'it''s'"));
                }
                Some(b'\'') => {
                    text.push_str(&self.source[run..at]);
                    if self.byte(at + 1) == Some(b'\'') {
                        text.push('\'');
                        at += 2;
                        run = at;
                    } else {
                        return Ok(Token {
                            tok: Tok::Str(text),
                            span: span(start, at + 1),
                        });
                    }
                }
                Some(_) => at += 1,
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lex(source: &str) -> Result<Vec<Tok>, Diagnostic> {
        let mut lexer = Lexer::new(source);
        let mut tokens = Vec::new();
        let mut at = 0;
        loop {
            let token = lexer.next(at)?;
            at = token.span.end as usize;
            if token.tok == Tok::End {
                return Ok(tokens);
            }
            tokens.push(token.tok);
        }
    }

    fn name(text: &str) -> Tok {
        Tok::Name(Name::bare(text))
    }

    #[test]
    fn punctuation_and_operators() {
        let tokens = lex("( ) [ ] { } , . : + - * / % ^ == != < <= > >= ?? $").unwrap();
        assert_eq!(
            tokens,
            [
                Tok::LParen,
                Tok::RParen,
                Tok::LBracket,
                Tok::RBracket,
                Tok::LBrace,
                Tok::RBrace,
                Tok::Comma,
                Tok::Dot,
                Tok::Colon,
                Tok::Plus,
                Tok::Minus,
                Tok::Star,
                Tok::Slash,
                Tok::Percent,
                Tok::Caret,
                Tok::EqEq,
                Tok::NotEq,
                Tok::Lt,
                Tok::Le,
                Tok::Gt,
                Tok::Ge,
                Tok::Coalesce,
                Tok::Dollar,
            ]
        );
    }

    #[test]
    fn keywords_and_names() {
        let tokens = lex("if then else and or not in true false null self _x9 Abc").unwrap();
        assert_eq!(
            tokens,
            [
                Tok::If,
                Tok::Then,
                Tok::Else,
                Tok::And,
                Tok::Or,
                Tok::Not,
                Tok::In,
                Tok::True,
                Tok::False,
                Tok::Null,
                name("self"),
                name("_x9"),
                name("Abc"),
            ]
        );
        assert!(is_keyword("then"));
        assert!(!is_keyword("than"));
    }

    #[test]
    fn a_colon_joins_a_namespace_only_when_nothing_separates_them() {
        assert_eq!(
            lex("ashlands:fall_damage").unwrap(),
            [Tok::Name(Name::qualified("ashlands", "fall_damage"))]
        );
        assert_eq!(lex("a : b").unwrap(), [name("a"), Tok::Colon, name("b")]);
        assert_eq!(lex("a: b").unwrap(), [name("a"), Tok::Colon, name("b")]);
        assert_eq!(
            lex("a:.0").unwrap(),
            [name("a"), Tok::Colon, Tok::Dot, Tok::Number(Number::Int(0))]
        );
        assert_eq!(
            lex("a:1").unwrap(),
            [name("a"), Tok::Colon, Tok::Number(Number::Int(1))]
        );
    }

    #[test]
    fn numbers() {
        let real = |value: f64| Tok::Number(Number::Real(Real::of(value)));
        let int = |value: i64| Tok::Number(Number::Int(value));
        assert_eq!(lex("42").unwrap(), [int(42)]);
        assert_eq!(lex("0").unwrap(), [int(0)]);
        assert_eq!(lex("9223372036854775807").unwrap(), [int(i64::MAX)]);
        assert_eq!(lex("3.5").unwrap(), [real(3.5)]);
        assert_eq!(lex("1e6").unwrap(), [real(1e6)]);
        assert_eq!(lex("1E6").unwrap(), [real(1e6)]);
        assert_eq!(lex("2.5e-3").unwrap(), [real(2.5e-3)]);
        assert_eq!(lex("1e+2").unwrap(), [real(100.0)]);
        // A point with no digit after it is not part of the number.
        assert_eq!(lex("3.x").unwrap(), [int(3), Tok::Dot, name("x")]);
        // An `e` with no digits after it is not an exponent.
        assert_eq!(lex("2e").unwrap(), [int(2), Tok::Unit("e".to_owned())]);
        assert_eq!(
            lex("2e+").unwrap(),
            [int(2), Tok::Unit("e".to_owned()), Tok::Plus]
        );
    }

    #[test]
    fn numbers_that_do_not_fit_are_errors() {
        let error = lex("9223372036854775808").unwrap_err();
        assert_eq!(error.code, codes::BAD_NUMBER);
        assert_eq!(error.span, Span::new(0, 19));
        assert!(error.suggestion.is_some());
        let error = lex("1 + 1e999").unwrap_err();
        assert_eq!(error.code, codes::BAD_NUMBER);
        assert_eq!(error.span, Span::new(4, 9));
    }

    #[test]
    fn letters_directly_after_a_number_are_a_unit() {
        assert_eq!(
            lex("300ms").unwrap(),
            [Tok::Number(Number::Int(300)), Tok::Unit("ms".to_owned())]
        );
        assert_eq!(
            lex("1.5kPa").unwrap(),
            [
                Tok::Number(Number::Real(Real::of(1.5))),
                Tok::Unit("kPa".to_owned())
            ]
        );
        // With a space between them the letters are a name.
        assert_eq!(
            lex("300 ms").unwrap(),
            [Tok::Number(Number::Int(300)), name("ms")]
        );
    }

    #[test]
    fn strings() {
        assert_eq!(lex("'ash'").unwrap(), [Tok::Str("ash".to_owned())]);
        assert_eq!(lex("''").unwrap(), [Tok::Str(String::new())]);
        assert_eq!(lex("'it''s'").unwrap(), [Tok::Str("it's".to_owned())]);
        assert_eq!(lex("''''").unwrap(), [Tok::Str("'".to_owned())]);
        assert_eq!(
            lex("'a {b} \"c\" é'").unwrap(),
            [Tok::Str("a {b} \"c\" é".to_owned())]
        );
        let error = lex("1 + 'ash").unwrap_err();
        assert_eq!(error.code, codes::UNTERMINATED_STRING);
        assert_eq!(error.span, Span::new(4, 8));
    }

    #[test]
    fn unexpected_characters_are_positioned_and_explained() {
        for (source, at, hint) in [
            ("a = b", 2, true),
            ("a \"b\"", 2, true),
            ("!a", 0, true),
            ("a & b", 2, true),
            ("a | b", 2, true),
            ("a ? b", 2, true),
            ("a # b", 2, false),
            ("a + é", 4, false),
        ] {
            let error = lex(source).unwrap_err();
            assert_eq!(error.code, codes::UNEXPECTED_CHARACTER, "{source}");
            assert_eq!(error.span.start, at, "{source}");
            assert_eq!(error.suggestion.is_some(), hint, "{source}");
        }
        // The span covers the whole character, not one byte of it.
        assert_eq!(lex("é").unwrap_err().span, Span::new(0, 2));
    }

    #[test]
    fn every_token_can_be_described() {
        for token in lex("1 1ms 'a' a $ ( ) [ ] { } , . : + - * / % ^ == != < <= > >= ?? if then else and or not in true false null").unwrap() {
            assert!(!token.describe().is_empty());
        }
        assert_eq!(Tok::End.describe(), "the end of the expression");
        assert_eq!(name("self").describe(), "`self`");
    }
}
