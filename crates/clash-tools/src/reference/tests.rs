use std::path::PathBuf;
use std::sync::OnceLock;

use super::*;

/// The reference of the engine, gathered once for every test.
pub(crate) fn catalog() -> &'static Catalog {
    static CATALOG: OnceLock<Catalog> = OnceLock::new();
    CATALOG.get_or_init(|| {
        Catalog::gather(&crate::engine_types()).unwrap_or_else(|diagnostics| {
            panic!("the engine's own definitions do not load:\n{diagnostics}")
        })
    })
}

/// The committed reference.
fn committed() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../docs/reference")
}

#[test]
fn every_registered_item_is_documented() {
    let missing = catalog().undocumented();
    assert!(
        missing.is_empty(),
        "{} registered items have no documentation. Document each where it is registered: a \
         definition type in its `Def::DOC`, a field or a value in a doc comment on it, a \
         property, function, event, verb or query in its registration, a setting or a reason \
         in its table.\n  {}",
        missing.len(),
        missing.join("\n  ")
    );
}

#[test]
fn the_committed_reference_is_up_to_date() {
    let pages = markdown::pages(catalog());
    let dir = committed();
    let mut stale = Vec::new();
    for (path, page) in &pages {
        match std::fs::read_to_string(dir.join(path)) {
            Ok(text) if text == *page => {}
            Ok(_) => stale.push(format!("{path} differs")),
            Err(_) => stale.push(format!("{path} is missing")),
        }
    }
    for path in existing_pages(&dir) {
        if !pages.contains_key(&path) {
            stale.push(format!("{path} is no longer written"));
        }
    }
    assert!(
        stale.is_empty(),
        "the committed reference in docs/reference is out of date; run `{}` and commit what it \
         changes:\n  {}",
        markdown::COMMAND,
        stale.join("\n  ")
    );
}

#[test]
fn every_definition_type_has_a_page_with_its_fields() {
    let catalog = catalog();
    let pages = markdown::pages(catalog);
    for entry in &catalog.types {
        let page = pages
            .get(&format!("types/{}.md", entry.name))
            .unwrap_or_else(|| panic!("no page for `{}`", entry.name));
        assert!(
            page.contains(&format!("# `{}`", entry.name)),
            "{}",
            entry.name
        );
        assert!(
            !entry.shapes.incomplete,
            "{} was not read to the end",
            entry.name
        );
        if let Some(root) = entry.shapes.root_struct() {
            for field in &root.fields {
                assert!(
                    page.contains(&format!("- `{}` (", field.name)),
                    "`{}` has no line for `{}`",
                    entry.name,
                    field.name
                );
            }
        }
    }
}

#[test]
fn fields_say_what_they_take() {
    let catalog = catalog();
    let pages = markdown::pages(catalog);
    // A reference, a quantity, a required field and an enum's values.
    let part = &pages["types/body_part.md"];
    assert!(
        part.contains("- `mass` ([mass](../units.md#mass) in `kg`; **required**)"),
        "{part}"
    );
    assert!(
        part.contains("object, id of a [`body_function`](body_function.md) → number"),
        "{part}"
    );
    let action = &pages["types/action.md"];
    assert!(
        action.contains("- `phases` (list of object [`Phase`](#phase); **required**)"),
        "{action}"
    );
    assert!(
        action.contains("[expression of number](../values.md#expressions)"),
        "{action}"
    );
    assert!(
        action.contains("[condition](../values.md#conditions)"),
        "{action}"
    );
    assert!(
        action.contains("[list of effects](../values.md#effects)"),
        "{action}"
    );
    // Hooks say what a handler may change.
    let hooks = &pages["hooks.md"];
    assert!(hooks.contains("A handler may change"), "{hooks}");
    // Functions say whether they are safe for belief.
    let functions = &pages["functions.md"];
    assert!(
        functions
            .contains("`clamp(x: number, low: number, high: number) -> number` · safe for belief"),
        "{functions}"
    );
}

#[test]
fn the_reference_is_the_same_every_time() {
    let again = Catalog::gather(&crate::engine_types()).expect("the engine loads");
    assert_eq!(markdown::pages(catalog()), markdown::pages(&again));
    assert_eq!(json::document(catalog()), json::document(&again));
}

#[test]
fn the_json_reference_reads_back_as_json() {
    let text = json::document(catalog());
    let mut sources = clash_data::SourceMap::new();
    let file = sources.add("reference.json", &text);
    let (tree, problems) = clash_data::parse_json(
        file,
        sources.text(file),
        &clash_data::Limits {
            max_file_bytes: usize::MAX,
            ..clash_data::Limits::default()
        },
    );
    assert!(problems.is_empty(), "{problems:?}");
    let tree = tree.expect("a tree");
    let types = tree
        .get("types")
        .and_then(|types| types.as_array())
        .expect("types");
    assert_eq!(types.len(), catalog().types.len());
}
