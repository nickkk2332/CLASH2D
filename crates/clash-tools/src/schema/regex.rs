//! A small regular expression matcher for the patterns the schemas use:
//! literals, `.`, classes `[a-z+-]`, `\s`, `\d` and escaped
//! punctuation, groups with `|`, the quantifiers `?`, `*` and `+`, and
//! the anchors `^` and `$`. Matching searches the text, as JSON Schema's
//! `pattern` does. For the tests' validator only.

/// One element of a pattern.
#[derive(Debug, Clone)]
enum Atom {
    Char(char),
    Any,
    Class(Vec<(char, char)>, bool),
    Group(Vec<Vec<Item>>),
    Start,
    End,
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum Repeat {
    One,
    Optional,
    Star,
    Plus,
}

#[derive(Debug, Clone)]
struct Item {
    atom: Atom,
    repeat: Repeat,
}

/// A compiled pattern.
#[derive(Debug, Clone)]
pub struct Regex {
    alternatives: Vec<Vec<Item>>,
}

impl Regex {
    /// Compiles a pattern, or says why it cannot.
    pub fn new(pattern: &str) -> Result<Regex, String> {
        let chars: Vec<char> = pattern.chars().collect();
        let mut at = 0;
        let alternatives = parse_alternatives(&chars, &mut at)?;
        if at != chars.len() {
            return Err(format!("unbalanced `)` at {at} in `{pattern}`"));
        }
        Ok(Regex { alternatives })
    }

    /// Whether the pattern matches somewhere in `text`.
    pub fn is_match(&self, text: &str) -> bool {
        let chars: Vec<char> = text.chars().collect();
        (0..=chars.len())
            .any(|start| match_alternatives(&self.alternatives, &chars, start, &|_| true))
    }
}

fn parse_alternatives(chars: &[char], at: &mut usize) -> Result<Vec<Vec<Item>>, String> {
    let mut alternatives = vec![Vec::new()];
    while *at < chars.len() {
        let c = chars[*at];
        match c {
            ')' => break,
            '|' => {
                *at += 1;
                alternatives.push(Vec::new());
                continue;
            }
            _ => {}
        }
        let atom = match c {
            '(' => {
                *at += 1;
                let inner = parse_alternatives(chars, at)?;
                if chars.get(*at) != Some(&')') {
                    return Err("an unclosed `(`".to_owned());
                }
                *at += 1;
                Atom::Group(inner)
            }
            '[' => {
                *at += 1;
                let negated = chars.get(*at) == Some(&'^');
                if negated {
                    *at += 1;
                }
                let mut ranges = Vec::new();
                while *at < chars.len() && chars[*at] != ']' {
                    let mut low = chars[*at];
                    if low == '\\' {
                        *at += 1;
                        low = *chars.get(*at).ok_or("a `\\` at the end")?;
                    }
                    *at += 1;
                    if chars.get(*at) == Some(&'-') && chars.get(*at + 1).is_some_and(|c| *c != ']')
                    {
                        let high = chars[*at + 1];
                        *at += 2;
                        ranges.push((low, high));
                    } else {
                        ranges.push((low, low));
                    }
                }
                if chars.get(*at) != Some(&']') {
                    return Err("an unclosed `[`".to_owned());
                }
                *at += 1;
                Atom::Class(ranges, negated)
            }
            '\\' => {
                *at += 1;
                let escaped = *chars.get(*at).ok_or("a `\\` at the end")?;
                *at += 1;
                match escaped {
                    's' => Atom::Class(
                        vec![(' ', ' '), ('\t', '\t'), ('\n', '\n'), ('\r', '\r')],
                        false,
                    ),
                    'd' => Atom::Class(vec![('0', '9')], false),
                    other => Atom::Char(other),
                }
            }
            '.' => {
                *at += 1;
                Atom::Any
            }
            '^' => {
                *at += 1;
                Atom::Start
            }
            '$' => {
                *at += 1;
                Atom::End
            }
            other => {
                *at += 1;
                Atom::Char(other)
            }
        };
        let repeat = match chars.get(*at) {
            Some('?') => Repeat::Optional,
            Some('*') => Repeat::Star,
            Some('+') => Repeat::Plus,
            _ => Repeat::One,
        };
        if repeat != Repeat::One {
            *at += 1;
        }
        if let Some(sequence) = alternatives.last_mut() {
            sequence.push(Item { atom, repeat });
        }
    }
    Ok(alternatives)
}

fn match_alternatives(
    alternatives: &[Vec<Item>],
    text: &[char],
    at: usize,
    then: &dyn Fn(usize) -> bool,
) -> bool {
    alternatives
        .iter()
        .any(|sequence| match_sequence(sequence, text, at, then))
}

fn match_sequence(items: &[Item], text: &[char], at: usize, then: &dyn Fn(usize) -> bool) -> bool {
    let Some((first, rest)) = items.split_first() else {
        return then(at);
    };
    let next = |end: usize| match_sequence(rest, text, end, then);
    match first.repeat {
        Repeat::One => match_atom(&first.atom, text, at, &next),
        Repeat::Optional => match_atom(&first.atom, text, at, &next) || next(at),
        Repeat::Star => match_star(&first.atom, text, at, &next),
        Repeat::Plus => match_atom(&first.atom, text, at, &|end| {
            match_star(&first.atom, text, end, &next)
        }),
    }
}

fn match_star(atom: &Atom, text: &[char], at: usize, then: &dyn Fn(usize) -> bool) -> bool {
    match_atom(atom, text, at, &|end| {
        end > at && match_star(atom, text, end, then)
    }) || then(at)
}

fn match_atom(atom: &Atom, text: &[char], at: usize, then: &dyn Fn(usize) -> bool) -> bool {
    match atom {
        Atom::Start => at == 0 && then(at),
        Atom::End => at == text.len() && then(at),
        Atom::Group(alternatives) => match_alternatives(alternatives, text, at, then),
        Atom::Char(c) => text.get(at) == Some(c) && then(at + 1),
        Atom::Any => at < text.len() && then(at + 1),
        Atom::Class(ranges, negated) => match text.get(at) {
            Some(c) => {
                let inside = ranges.iter().any(|(low, high)| low <= c && c <= high);
                inside != *negated && then(at + 1)
            }
            None => false,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn patterns_match_as_json_schema_does() {
        let regex = Regex::new(
            r"^\s*[+-]?([0-9]+(\.[0-9]+)?|\.[0-9]+)([eE][+-]?[0-9]+)?\s*(Pa|kPa|MPa|GPa)\s*$",
        )
        .expect("compiles");
        for good in ["30 MPa", "30MPa", "-1.5e3 kPa", " .5 Pa ", "2e4 GPa"] {
            assert!(regex.is_match(good), "{good}");
        }
        for bad in ["30 MPA", "30", "MPa", "30 MPa x", "1. Pa"] {
            assert!(!regex.is_match(bad), "{bad}");
        }
        // Unanchored, a pattern is searched for.
        assert!(Regex::new("b+c").expect("compiles").is_match("abbbcd"));
        assert!(Regex::new("a(").is_err());
    }
}
