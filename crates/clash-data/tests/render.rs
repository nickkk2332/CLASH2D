//! How diagnostics look (04 §8.2): package P1, over the error classes of
//! package F4's fixture.

mod common;

use std::path::Path;

use clash_data::{Diagnostics, Loader, Severity};
use common::*;

fn broken() -> clash_data::LoadOutcome {
    let types = types();
    // A path relative to the crate, so that the paths shown are the same
    // on every machine.
    let outcome = Loader::new(&types)
        .search_path(
            Path::new("tests")
                .join("fixtures")
                .join("broken")
                .join("mods"),
        )
        .load_all();
    assert_known_codes(&outcome.diagnostics);
    outcome
}

#[test]
fn each_error_class_renders_with_a_caret_under_the_right_characters() {
    let outcome = broken();
    assert!(outcome.defs.is_none());
    let rendered = outcome.diagnostics.render();
    let expected = std::fs::read_to_string(
        Path::new("tests")
            .join("fixtures")
            .join("broken")
            .join("expected.txt"),
    )
    .expect("the expected rendering is committed beside the fixture");
    assert_eq!(
        rendered, expected,
        "\n--- rendered ---\n{rendered}\n--- expected ---\n{expected}"
    );
}

#[test]
fn every_caret_sits_under_the_text_the_diagnostic_is_about() {
    let outcome = broken();
    // What each diagnostic's caret must underline, by its code.
    let under: &[(&str, &str)] = &[
        ("E0203", "\"thougness\""),
        ("E0202", "{"),
        ("E0204", "\"30 MPA\""),
        ("E0501", "\"ashh\""),
        ("E0405", "\"loop_a\""),
        ("E0305", "\"slag\""),
        ("E0201", "\"0.2\""),
    ];
    let mut seen = Vec::new();
    for diagnostic in &outcome.diagnostics {
        let location = diagnostic.location.as_ref().expect("a location");
        let (_, expected) = under
            .iter()
            .find(|(code, _)| *code == diagnostic.code)
            .unwrap_or_else(|| panic!("unexpected {}", diagnostic.code));
        let line: Vec<char> = location.source_line.chars().collect();
        let start = location.column as usize - 1;
        let end = location.end_column as usize - 1;
        let underlined: String = line[start..end].iter().collect();
        assert_eq!(&underlined, expected, "{}", diagnostic.code);

        // And the rendered caret line has exactly that many carets,
        // starting under that column.
        let rendered = diagnostic.render();
        let caret_line = rendered
            .lines()
            .find(|line| line.contains('^'))
            .expect("a caret line");
        let source_line = rendered
            .lines()
            .find(|line| line.starts_with(&format!("{} | ", location.line)))
            .expect("the source line");
        // Both lines have the same gutter, so the caret is under the
        // character at the same position in the line above it.
        let caret_at = caret_line.chars().position(|ch| ch == '^').unwrap();
        let carets = caret_line.chars().filter(|ch| *ch == '^').count();
        let above: String = source_line.chars().skip(caret_at).take(carets).collect();
        assert_eq!(&above, expected, "{}", diagnostic.code);
        seen.push(diagnostic.code.as_str());
    }
    // Every class of the fixture is there: unknown key, bad unit, missing
    // reference, inheritance cycle, duplicate id, type mismatch.
    for (code, _) in under {
        assert!(seen.contains(code), "no {code} in {seen:?}");
    }
}

#[test]
fn every_diagnostic_names_file_line_column_and_definition() {
    let outcome = broken();
    for diagnostic in &outcome.diagnostics {
        let location = diagnostic.location.as_ref().expect("a location");
        assert_eq!(
            location.file,
            "tests/fixtures/broken/mods/broken/data/errors.json"
        );
        assert!(location.line > 1);
        assert!(location.column > 1);
        let definition = diagnostic.definition.as_ref().expect("a definition");
        assert!(definition.id.starts_with("broken:"));
        assert_eq!(diagnostic.severity, Severity::Error);
    }
}

#[test]
fn the_json_form_round_trips_and_renders_the_same() {
    let outcome = broken();
    let json = outcome.diagnostics.to_json();
    let back = Diagnostics::from_json(&json).expect("the JSON form reads back");
    assert_eq!(back, outcome.diagnostics);
    assert_eq!(back.render(), outcome.diagnostics.render());
    assert_eq!(back.to_json(), json);
}
