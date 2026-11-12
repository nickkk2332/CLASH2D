//! The `bench` subcommand (14 §6.3; packages W23, T8, Z1, TL5): the
//! benchmarks of the specifications' budget tables, each measured and
//! printed against its row of `budgets.toml`.
//!
//! ```text
//! cargo xtask bench            # every benchmark, in release, against its budget
//! cargo xtask bench --check    # the same, failing if one is over by more than its margin
//! cargo xtask bench light      # only those whose name holds `light`
//! ```
//!
//! A benchmark builds its own scene, from the repository's mods, and
//! times the operation of its row. Each is timed in several rounds and
//! the best round is kept: the machine is shared, and noise only ever
//! adds time. The figures are wall-clock time on whatever runs them; the
//! gate is for one machine's history, not a contract.
//!
//! A benchmark with no row in the file is printed as a figure with no
//! budget. A row whose benchmark does not exist is an error.

pub mod budgets;
mod cases;

use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::time::Instant;

use budgets::Budget;

use crate::data::{EXIT_FAILED, EXIT_OK, EXIT_USAGE, Output};

/// The usage text of the `bench` subcommand.
pub const USAGE: &str = "\
benchmarks:
  bench [--budgets <file>] [--check] [--list] [<filter>...]
        Run the benchmarks whose names hold one of the filters (all of
        them with none) and print each against its budget from the
        budgets file (budgets.toml at the root of the repository by
        default). With --check, exit 1 if one is over its limit by more
        than its margin. --list names the benchmarks and runs none.
        Build in release: `cargo xtask bench` does.
";

/// What a benchmark measured: nanoseconds of the operation its budget
/// row names, and a note on how.
#[derive(Clone, Debug, PartialEq)]
pub struct Measured {
    /// The figure, in nanoseconds, in the terms of the row.
    pub ns: f64,
    /// How it was measured, and other figures worth printing.
    pub note: String,
}

impl Measured {
    fn new(ns: f64, note: impl Into<String>) -> Measured {
        Measured {
            ns,
            note: note.into(),
        }
    }
}

/// One benchmark.
pub struct Bench {
    /// Its name, as `budgets.toml` gives it.
    pub name: &'static str,
    /// What it times.
    pub what: &'static str,
    /// Builds its scene and times it.
    pub run: fn(&mut cases::Kit) -> Result<Measured, String>,
}

/// The time of one call of `body`, in nanoseconds: the best of `rounds`
/// rounds of `per_round` calls each.
pub fn best_ns<R>(rounds: u32, per_round: u32, mut body: impl FnMut() -> R) -> f64 {
    let per_round = per_round.max(1);
    let mut best = f64::INFINITY;
    for _ in 0..rounds.max(1) {
        let started = Instant::now();
        for _ in 0..per_round {
            std::hint::black_box(body());
        }
        let each = started.elapsed().as_nanos() as f64 / f64::from(per_round);
        best = best.min(each);
    }
    best
}

/// A duration in nanoseconds as a person reads it.
pub fn show_ns(ns: f64) -> String {
    if !ns.is_finite() {
        return "—".to_owned();
    }
    if ns < 1.0e3 {
        format!("{ns:.1} ns")
    } else if ns < 1.0e6 {
        format!("{:.2} µs", ns / 1.0e3)
    } else if ns < 1.0e9 {
        format!("{:.2} ms", ns / 1.0e6)
    } else {
        format!("{:.2} s", ns / 1.0e9)
    }
}

/// The verdict on one measurement.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Verdict {
    /// At or under the limit.
    Within,
    /// Over the limit but within its margin.
    Margin,
    /// Over the limit by more than its margin.
    Over,
    /// No budget row names it.
    Unbudgeted,
}

/// How a measurement stands against its budget.
pub fn verdict(budget: Option<&Budget>, ns: f64) -> Verdict {
    match budget {
        None => Verdict::Unbudgeted,
        Some(budget) if ns <= budget.limit_ns => Verdict::Within,
        Some(budget) if ns <= budget.allowed_ns() => Verdict::Margin,
        Some(_) => Verdict::Over,
    }
}

/// The repository's `budgets.toml`.
pub fn default_budgets_path() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../budgets.toml")
}

/// What the command line asked for.
struct Request {
    budgets: PathBuf,
    check: bool,
    list: bool,
    filters: Vec<String>,
}

fn parse_args(args: &[String]) -> Result<Request, String> {
    let mut request = Request {
        budgets: default_budgets_path(),
        check: false,
        list: false,
        filters: Vec::new(),
    };
    let mut words = args.iter();
    while let Some(word) = words.next() {
        match word.as_str() {
            "--check" => request.check = true,
            "--list" => request.list = true,
            "--budgets" => {
                let path = words.next().ok_or("--budgets needs a file")?;
                request.budgets = PathBuf::from(path);
            }
            flag if flag.starts_with("--") => return Err(format!("unknown option `{flag}`")),
            filter => request.filters.push(filter.to_owned()),
        }
    }
    Ok(request)
}

/// Checks that every budget names a benchmark that exists.
pub fn unknown_benches(budgets: &[Budget]) -> Vec<String> {
    budgets
        .iter()
        .filter(|budget| {
            !cases::BENCHES
                .iter()
                .any(|bench| bench.name == budget.bench)
        })
        .map(|budget| {
            format!(
                "budgets.toml:{}: there is no benchmark `{}`",
                budget.line, budget.bench
            )
        })
        .collect()
}

/// Runs the `bench` subcommand.
pub fn run(args: &[String]) -> Output {
    let request = match parse_args(args) {
        Ok(request) => request,
        Err(message) => {
            return Output {
                out: String::new(),
                err: format!("clash-tools: {message}\n\n{USAGE}"),
                code: EXIT_USAGE,
            };
        }
    };
    let text = match std::fs::read_to_string(&request.budgets) {
        Ok(text) => text,
        Err(error) => {
            return Output {
                out: String::new(),
                err: format!(
                    "clash-tools: cannot read {}: {error}\n",
                    request.budgets.display()
                ),
                code: EXIT_FAILED,
            };
        }
    };
    let budgets = match budgets::parse(&text) {
        Ok(budgets) => budgets,
        Err(error) => {
            return Output {
                out: String::new(),
                err: format!("clash-tools: {error}\n"),
                code: EXIT_FAILED,
            };
        }
    };
    let unknown = unknown_benches(&budgets);
    if !unknown.is_empty() {
        return Output {
            out: String::new(),
            err: format!("{}\n", unknown.join("\n")),
            code: EXIT_FAILED,
        };
    }
    let chosen: Vec<&Bench> = cases::BENCHES
        .iter()
        .filter(|bench| {
            request.filters.is_empty()
                || request
                    .filters
                    .iter()
                    .any(|filter| bench.name.contains(filter.as_str()))
        })
        .collect();
    let budget_of = |name: &str| budgets.iter().find(|budget| budget.bench == name);
    if request.list {
        let mut out = String::new();
        for bench in &chosen {
            let row = budget_of(bench.name).map_or_else(
                || "no budget".to_owned(),
                |budget| {
                    format!(
                        "{} \"{}\" under {}",
                        budget.spec, budget.row, budget.limit_text
                    )
                },
            );
            let _ = writeln!(out, "{:<34} {}\n{:<34} {row}", bench.name, bench.what, "");
        }
        return Output {
            out,
            err: String::new(),
            code: EXIT_OK,
        };
    }
    if cfg!(debug_assertions) {
        eprintln!(
            "clash-tools: a debug build: the figures mean little; `cargo xtask bench` builds in release"
        );
    }

    let mut kit = cases::Kit::default();
    let mut over = Vec::new();
    let mut failed = Vec::new();
    println!(
        "{:<34} {:>12} {:>12} {:>7}  row",
        "benchmark", "measured", "limit", ""
    );
    for bench in chosen {
        let budget = budget_of(bench.name);
        match (bench.run)(&mut kit) {
            Ok(measured) => {
                let verdict = verdict(budget, measured.ns);
                let mark = match verdict {
                    Verdict::Within => "ok",
                    Verdict::Margin => "margin",
                    Verdict::Over => "OVER",
                    Verdict::Unbudgeted => "",
                };
                let (limit, row) = budget.map_or_else(
                    || ("—".to_owned(), "no budget".to_owned()),
                    |budget| {
                        (
                            budget.limit_text.clone(),
                            format!("{}: {}", budget.spec, budget.row),
                        )
                    },
                );
                println!(
                    "{:<34} {:>12} {:>12} {:>7}  {row}",
                    bench.name,
                    show_ns(measured.ns),
                    limit,
                    mark
                );
                if !measured.note.is_empty() {
                    println!("{:<34} {}", "", measured.note);
                }
                if verdict == Verdict::Over {
                    over.push(bench.name);
                }
            }
            Err(message) => {
                println!("{:<34} {:>12}  {message}", bench.name, "FAILED");
                failed.push(bench.name);
            }
        }
    }
    let mut out = String::new();
    if !failed.is_empty() {
        let _ = writeln!(out, "bench: could not run {}", failed.join(", "));
    }
    if over.is_empty() {
        let _ = writeln!(
            out,
            "bench: nothing over its budget by more than its margin"
        );
    } else {
        let _ = writeln!(
            out,
            "bench: over budget by more than the margin: {}",
            over.join(", ")
        );
    }
    let gate_fails = !failed.is_empty() || (request.check && !over.is_empty());
    Output {
        out,
        err: String::new(),
        code: if gate_fails { EXIT_FAILED } else { EXIT_OK },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The budget tables of the specifications that `budgets.toml`
    /// draws from: the rows of the table under a `## N. Budgets`
    /// heading, by their first column.
    fn spec_rows(spec: &str) -> Vec<String> {
        let (number, section) = spec.split_once(" §").expect("a spec is `NN §M`");
        let docs = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../docs");
        let file = std::fs::read_dir(&docs)
            .unwrap()
            .filter_map(Result::ok)
            .map(|entry| entry.path())
            .find(|path| {
                path.file_name()
                    .and_then(|name| name.to_str())
                    .is_some_and(|name| name.starts_with(&format!("{number}-")))
            })
            .unwrap_or_else(|| panic!("no document {number}"));
        let text = std::fs::read_to_string(&file).unwrap();
        let heading = format!("## {section}. Budgets");
        let start = text
            .find(&heading)
            .unwrap_or_else(|| panic!("{} has no `{heading}`", file.display()));
        text[start..]
            .lines()
            .skip(1)
            .take_while(|line| !line.starts_with("## "))
            .filter(|line| line.starts_with('|') && !line.starts_with("|--"))
            .filter_map(|line| line.split('|').nth(1).map(|cell| cell.trim().to_owned()))
            .collect()
    }

    #[test]
    fn every_budget_is_a_row_of_its_specification_and_has_a_benchmark() {
        let text = std::fs::read_to_string(default_budgets_path()).unwrap();
        let budgets = budgets::parse(&text).unwrap_or_else(|error| panic!("{error}"));
        assert!(!budgets.is_empty());
        assert_eq!(unknown_benches(&budgets), Vec::<String>::new());
        for budget in &budgets {
            let rows = spec_rows(&budget.spec);
            assert!(
                rows.contains(&budget.row),
                "budgets.toml:{}: `{}` is not a row of the table of {}: {rows:?}",
                budget.line,
                budget.row,
                budget.spec
            );
            assert!(budget.margin <= 1.0, "a margin over the whole limit");
        }
    }

    #[test]
    fn every_benchmark_has_a_distinct_name() {
        for (at, bench) in cases::BENCHES.iter().enumerate() {
            assert!(
                !cases::BENCHES[..at]
                    .iter()
                    .any(|other| other.name == bench.name),
                "{}",
                bench.name
            );
            assert!(!bench.what.is_empty());
        }
    }

    #[test]
    fn verdicts_follow_the_limit_and_the_margin() {
        let budget = Budget {
            bench: "x".to_owned(),
            spec: "02 §25".to_owned(),
            row: "y".to_owned(),
            limit_text: "10 ns".to_owned(),
            limit_ns: 10.0,
            margin: 0.5,
            line: 1,
        };
        assert_eq!(verdict(Some(&budget), 9.0), Verdict::Within);
        assert_eq!(verdict(Some(&budget), 10.0), Verdict::Within);
        assert_eq!(verdict(Some(&budget), 14.0), Verdict::Margin);
        assert_eq!(verdict(Some(&budget), 16.0), Verdict::Over);
        assert_eq!(verdict(None, 16.0), Verdict::Unbudgeted);
    }

    #[test]
    fn options_are_read_and_mistakes_refused() {
        let words = |list: &[&str]| list.iter().map(|w| (*w).to_owned()).collect::<Vec<_>>();
        let request = parse_args(&words(&["--check", "light", "field"])).unwrap();
        assert!(request.check && !request.list);
        assert_eq!(request.filters, ["light", "field"]);
        assert!(parse_args(&words(&["--budgets"])).is_err());
        assert!(parse_args(&words(&["--fast"])).is_err());
        let output = run(&words(&["--list"]));
        assert_eq!(output.code, EXIT_OK, "{}", output.err);
        for bench in cases::BENCHES {
            assert!(output.out.contains(bench.name));
        }
    }

    #[test]
    fn durations_are_shown_in_a_fitting_unit() {
        assert_eq!(show_ns(12.34), "12.3 ns");
        assert_eq!(show_ns(2_500.0), "2.50 µs");
        assert_eq!(show_ns(4_200_000.0), "4.20 ms");
        assert_eq!(show_ns(f64::INFINITY), "—");
    }
}
