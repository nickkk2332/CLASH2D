//! `budgets.toml` (14 §6.3): the rows of the specifications' budget
//! tables that have a benchmark, each with its limit and margin.
//!
//! The file is a small part of TOML, read here without a dependency:
//! comments from `#`, a `[[budget]]` header for each row, and
//! `key = value` lines whose value is a string in double quotes or a
//! number. Anything else is an error that names its line.
//!
//! ```toml
//! [[budget]]
//! bench = "perception.sight_30m"
//! spec = "15 §17"
//! row = "One observer-target evaluation, including line of sight at 30 m"
//! limit = "2 µs"
//! margin = 0.25
//! ```
//!
//! `limit` is a duration with one of the units `ns`, `µs` (or `us`),
//! `ms` and `s`. `margin` is a share of the limit: a measurement fails
//! the gate when it is over `limit × (1 + margin)`.

use std::fmt;

/// One row of a specification's budget table, with its benchmark.
#[derive(Clone, Debug, PartialEq)]
pub struct Budget {
    /// The benchmark that measures it, by name.
    pub bench: String,
    /// The specification and section whose table holds the row, as
    /// `15 §17`.
    pub spec: String,
    /// The row's first column, as the table words it.
    pub row: String,
    /// The limit as written.
    pub limit_text: String,
    /// The limit in nanoseconds.
    pub limit_ns: f64,
    /// The share of the limit a measurement may exceed it by before the
    /// gate fails.
    pub margin: f64,
    /// The line of the file the row's header is on.
    pub line: usize,
}

impl Budget {
    /// The most a measurement may be and still pass the gate.
    pub fn allowed_ns(&self) -> f64 {
        self.limit_ns * (1.0 + self.margin)
    }
}

/// A mistake in the file, with the line it is on.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BudgetError {
    /// The line, from 1.
    pub line: usize,
    /// What is wrong.
    pub message: String,
}

impl fmt::Display for BudgetError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "budgets.toml:{}: {}", self.line, self.message)
    }
}

/// The keys a `[[budget]]` holds; every one is needed.
const KEYS: [&str; 5] = ["bench", "spec", "row", "limit", "margin"];

/// Nanoseconds in each unit a limit may be written in.
const UNITS: [(&str, f64); 5] = [
    ("ns", 1.0),
    ("µs", 1.0e3),
    ("us", 1.0e3),
    ("ms", 1.0e6),
    ("s", 1.0e9),
];

/// A duration written as a number and a unit, in nanoseconds.
pub fn parse_duration(text: &str) -> Option<f64> {
    let text = text.trim();
    let split = text
        .find(|c: char| !(c.is_ascii_digit() || c == '.' || c == '_'))
        .unwrap_or(text.len());
    let (number, unit) = text.split_at(split);
    let number: f64 = number.replace('_', "").parse().ok()?;
    let scale = UNITS
        .iter()
        .find(|(name, _)| *name == unit.trim())
        .map(|(_, scale)| *scale)?;
    (number.is_finite() && number >= 0.0).then_some(number * scale)
}

/// A value: a string or a number.
enum Value {
    Text(String),
    Number(f64),
}

fn parse_value(text: &str) -> Result<Value, String> {
    let text = text.trim();
    if let Some(inner) = text.strip_prefix('"') {
        let Some(inner) = inner.strip_suffix('"') else {
            return Err("a string is not closed".to_owned());
        };
        if inner.contains('"') || inner.contains('\\') {
            return Err("escapes and quotes inside a string are not read".to_owned());
        }
        return Ok(Value::Text(inner.to_owned()));
    }
    text.replace('_', "")
        .parse::<f64>()
        .ok()
        .filter(|number| number.is_finite())
        .map(Value::Number)
        .ok_or_else(|| format!("`{text}` is neither a string nor a number"))
}

/// Strips a comment from a line, minding `#` inside a string.
fn strip_comment(line: &str) -> &str {
    let mut quoted = false;
    for (at, c) in line.char_indices() {
        match c {
            '"' => quoted = !quoted,
            '#' if !quoted => return &line[..at],
            _ => {}
        }
    }
    line
}

/// One `[[budget]]` being read.
#[derive(Default)]
struct Partial {
    line: usize,
    text: Vec<(String, String)>,
    margin: Option<f64>,
}

impl Partial {
    fn finish(self) -> Result<Budget, BudgetError> {
        let error = |message: String| BudgetError {
            line: self.line,
            message,
        };
        let get = |key: &str| {
            self.text
                .iter()
                .find(|(have, _)| have == key)
                .map(|(_, value)| value.clone())
                .ok_or_else(|| error(format!("the budget has no `{key}`")))
        };
        let limit_text = get("limit")?;
        let limit_ns = parse_duration(&limit_text).ok_or_else(|| {
            error(format!(
                "`{limit_text}` is not a duration in ns, µs, ms or s"
            ))
        })?;
        let margin = self
            .margin
            .ok_or_else(|| error("the budget has no `margin`".to_owned()))?;
        if margin < 0.0 {
            return Err(error("a margin is not below zero".to_owned()));
        }
        Ok(Budget {
            bench: get("bench")?,
            spec: get("spec")?,
            row: get("row")?,
            limit_text,
            limit_ns,
            margin,
            line: self.line,
        })
    }
}

/// Reads the budgets file. Every benchmark is named once.
pub fn parse(text: &str) -> Result<Vec<Budget>, BudgetError> {
    let mut budgets = Vec::new();
    let mut current: Option<Partial> = None;
    for (index, raw) in text.lines().enumerate() {
        let line = index + 1;
        let error = |message: String| BudgetError { line, message };
        let content = strip_comment(raw).trim();
        if content.is_empty() {
            continue;
        }
        if content == "[[budget]]" {
            if let Some(done) = current.take() {
                budgets.push(done.finish()?);
            }
            current = Some(Partial {
                line,
                ..Partial::default()
            });
            continue;
        }
        let Some((key, value)) = content.split_once('=') else {
            return Err(error(format!("`{content}` is not `key = value`")));
        };
        let key = key.trim();
        let Some(partial) = current.as_mut() else {
            return Err(error("a key before the first `[[budget]]`".to_owned()));
        };
        if !KEYS.contains(&key) {
            return Err(error(format!(
                "`{key}` is not a key of a budget; the keys are {}",
                KEYS.join(", ")
            )));
        }
        if partial.text.iter().any(|(have, _)| have == key)
            || (key == "margin" && partial.margin.is_some())
        {
            return Err(error(format!("`{key}` is given twice")));
        }
        match (key, parse_value(value).map_err(error)?) {
            ("margin", Value::Number(margin)) => partial.margin = Some(margin),
            ("margin", Value::Text(_)) => return Err(error("a margin is a number".to_owned())),
            (_, Value::Text(text)) => partial.text.push((key.to_owned(), text)),
            (_, Value::Number(_)) => return Err(error(format!("`{key}` is a string"))),
        }
    }
    if let Some(done) = current.take() {
        budgets.push(done.finish()?);
    }
    for (at, budget) in budgets.iter().enumerate() {
        if budgets[..at]
            .iter()
            .any(|other| other.bench == budget.bench)
        {
            return Err(BudgetError {
                line: budget.line,
                message: format!("the benchmark `{}` has two budgets", budget.bench),
            });
        }
    }
    Ok(budgets)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn durations_are_read_in_every_unit() {
        assert_eq!(parse_duration("30 ns"), Some(30.0));
        assert_eq!(parse_duration("2 µs"), Some(2000.0));
        assert_eq!(parse_duration("2us"), Some(2000.0));
        assert_eq!(parse_duration("1.5 ms"), Some(1.5e6));
        assert_eq!(parse_duration("1 s"), Some(1.0e9));
        assert_eq!(parse_duration("50_000 ns"), Some(5.0e4));
        assert_eq!(parse_duration("2 hours"), None);
        assert_eq!(parse_duration("ms"), None);
    }

    #[test]
    fn a_file_of_two_budgets_is_read() {
        let text = r#"
# A comment.
[[budget]]
bench = "a.one"   # the first
spec = "02 §25"
row = "Cell flag read"
limit = "10 ns"
margin = 0.5

[[budget]]
bench = "a.two"
spec = "15 §17"
row = "One # in a row"
limit = "2 µs"
margin = 0
"#;
        let budgets = parse(text).unwrap();
        assert_eq!(budgets.len(), 2);
        assert_eq!(budgets[0].bench, "a.one");
        assert_eq!(budgets[0].limit_ns, 10.0);
        assert_eq!(budgets[0].allowed_ns(), 15.0);
        assert_eq!(budgets[0].line, 3);
        assert_eq!(budgets[1].row, "One # in a row");
        assert_eq!(budgets[1].allowed_ns(), 2000.0);
    }

    #[test]
    fn mistakes_name_their_line() {
        let missing = "[[budget]]\nbench = \"a\"\nspec = \"x\"\nrow = \"y\"\nlimit = \"1 ms\"\n";
        assert_eq!(parse(missing).unwrap_err().line, 1);
        let unknown = "[[budget]]\nbench = \"a\"\nlimt = \"1 ms\"\n";
        let error = parse(unknown).unwrap_err();
        assert_eq!(error.line, 3);
        assert!(error.message.contains("limt"), "{error}");
        let twice = "[[budget]]\nbench = \"a\"\nbench = \"b\"\n";
        assert_eq!(parse(twice).unwrap_err().line, 3);
        let stray = "bench = \"a\"\n";
        assert_eq!(parse(stray).unwrap_err().line, 1);
        let bad_limit =
            "[[budget]]\nbench = \"a\"\nspec = \"x\"\nrow = \"y\"\nlimit = \"soon\"\nmargin = 1\n";
        assert!(parse(bad_limit).unwrap_err().message.contains("soon"));
        let row =
            "[[budget]]\nbench = \"a\"\nspec = \"x\"\nrow = \"y\"\nlimit = \"1 ms\"\nmargin = 1\n";
        let both = format!("{row}{row}");
        let error = parse(&both).unwrap_err();
        assert_eq!(error.line, 7);
        assert!(error.message.contains("two budgets"));
    }
}
