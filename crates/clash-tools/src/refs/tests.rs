use std::collections::BTreeSet;
use std::path::PathBuf;
use std::sync::{Arc, OnceLock};

use super::*;

/// The repository's directory of mods.
fn mods_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../mods")
}

/// The bundled mods, loaded once for every test.
fn bundled() -> &'static Arc<Defs> {
    static DEFS: OnceLock<Arc<Defs>> = OnceLock::new();
    DEFS.get_or_init(|| {
        let types = crate::engine_types();
        let outcome = clash_scene::engine_loader(&types)
            .search_path(mods_dir())
            .load_all();
        outcome.defs.expect("the bundled mods load")
    })
}

/// What the engine's types read, worked out once.
fn engine_entries() -> &'static [TypeEntry] {
    &crate::reference::tests::catalog().types
}

/// Every reference of the bundled mods, found once.
fn all() -> &'static [Reference] {
    static ALL: OnceLock<Vec<Reference>> = OnceLock::new();
    ALL.get_or_init(|| references(bundled(), engine_entries()))
}

/// `file:line` with the file's path from the mods directory.
fn short(defs: &Defs, span: Span) -> String {
    let location = defs.sources().locate(span);
    let file = location
        .file
        .rsplit_once("/mods/")
        .map_or(location.file.as_str(), |(_, rest)| rest)
        .to_owned();
    format!("{file}:{}", location.line)
}

fn incoming<'a>(ty: &str, id: &str) -> Vec<&'a Reference> {
    all()
        .iter()
        .filter(|reference| reference.to_type == ty && reference.to_id == id)
        .collect()
}

#[test]
fn every_field_that_names_steel_is_found_where_it_was_written() {
    let defs = bundled();
    let found = |kind: RefKind| -> BTreeSet<String> {
        incoming("material", "core:steel")
            .into_iter()
            .filter(|reference| reference.kind == kind)
            .map(|reference| {
                assert_eq!(reference.path, "material");
                short(defs, reference.span)
            })
            .collect()
    };
    let items = found(RefKind::Field);
    let mentions = found(RefKind::Mentions);
    // Items read their material as a reference; the world's elements read
    // it as a string, resolved when the world's tables are built (D-803).
    for place in ["core/data/locks.json:4", "core/data/items.json:33"] {
        assert!(items.contains(place), "{place}: {items:?}");
    }
    for place in [
        "core/data/world/usable.json:10",
        "core/data/world/lighting.json:33",
    ] {
        assert!(mentions.contains(place), "{place}: {mentions:?}");
    }
    // Between them, every line of the bundled mods that gives steel as a
    // material, and nothing else.
    assert!(items.is_disjoint(&mentions));
    let both: BTreeSet<String> = items.union(&mentions).cloned().collect();
    assert_eq!(both, written(r#""material":"steel""#), "{both:?}");
}

/// Every line of the bundled mods' files that holds `text`, spaces aside,
/// as `file:line` with the file's path from the mods directory.
fn written(text: &str) -> BTreeSet<String> {
    fn walk(dir: &std::path::Path, base: &std::path::Path, text: &str, out: &mut BTreeSet<String>) {
        let mut entries: Vec<_> = std::fs::read_dir(dir)
            .expect("a directory")
            .filter_map(Result::ok)
            .map(|entry| entry.path())
            .collect();
        entries.sort();
        for path in entries {
            if path.is_dir() {
                walk(&path, base, text, out);
            } else if path.extension().is_some_and(|ext| ext == "json") {
                let file = path
                    .strip_prefix(base)
                    .expect("inside the mods")
                    .components()
                    .map(|part| part.as_os_str().to_string_lossy().into_owned())
                    .collect::<Vec<_>>()
                    .join("/");
                let source = std::fs::read_to_string(&path).expect("a file");
                for (at, line) in source.lines().enumerate() {
                    let line: String = line.chars().filter(|c| !c.is_whitespace()).collect();
                    if line.contains(text) {
                        out.insert(format!("{file}:{}", at + 1));
                    }
                }
            }
        }
    }
    let mut out = BTreeSet::new();
    walk(&mods_dir(), &mods_dir(), text, &mut out);
    out
}

/// How many definitions refer to steel by a field, and how many name it
/// in plain text, as `refs` counts them.
fn steel_counts() -> (usize, usize) {
    let incoming = incoming("material", "core:steel");
    let mentions = incoming
        .iter()
        .filter(|reference| reference.kind == RefKind::Mentions)
        .count();
    (incoming.len() - mentions, mentions)
}

#[test]
fn copies_patches_map_keys_and_mentions_are_found() {
    let defs = bundled();
    // Inheritance, from the file, which the finished tree forgets.
    let copies = incoming("block", "core:door");
    assert!(
        copies
            .iter()
            .any(|reference| reference.kind == RefKind::Copies
                && reference.from_id == "core:lockable_door"
                && short(defs, reference.span) == "core/data/locks.json:9"),
        "{copies:?}"
    );
    // A patch, by the id it names.
    let patches = incoming("damage_type", "core:cut");
    assert!(
        patches
            .iter()
            .any(|reference| reference.kind == RefKind::Patches
                && short(defs, reference.span) == "core/data/world/severing.json:3"),
        "{patches:?}"
    );
    // A key of a map read by damage type, and a key of the world's,
    // read as a string.
    assert!(
        all()
            .iter()
            .any(|reference| reference.kind == RefKind::Field
                && reference.from_type == "item"
                && reference.path == "grip"
                && reference.to_id == "core:arm")
    );
    assert!(
        all()
            .iter()
            .any(|reference| reference.kind == RefKind::Mentions
                && reference.from_id == "core:leather"
                && reference.path == "resist"
                && reference.to_id == "core:impact")
    );
    // A rule attached by name in an effect is named in plain text.
    let mentions = incoming("eoc", "core:rest_when_tired");
    assert!(
        mentions
            .iter()
            .any(|reference| reference.kind == RefKind::Mentions
                && reference.from_id == "core:people_rest"),
        "{mentions:?}"
    );
    // A qualified id inside an expression.
    let named = incoming("ability", "core:firebolt");
    assert!(
        named
            .iter()
            .any(|reference| reference.kind == RefKind::Mentions && reference.path.contains("if")),
        "{named:?}"
    );
}

#[test]
fn every_string_read_as_a_reference_resolves_as_the_loader_resolved_it() {
    // The bundled mods load, so the loader resolved every reference in
    // them: the walk must type and resolve each the same way.
    let defs = bundled();
    let resolver = Resolver::new(defs);
    let mut checked = 0;
    for tree in defs.trees() {
        let Some(entry) = engine_entries().iter().find(|e| e.name == tree.type_name) else {
            continue;
        };
        for place in entries(&entry.shapes, &tree.tree) {
            let Reading::Ref(ty) = place.reading else {
                continue;
            };
            let text = match place.value {
                Value::Key(text) => text,
                Value::Node(node) => node.as_str().unwrap_or_default(),
            };
            assert!(
                resolver.resolve(ty, text, place.span).is_some(),
                "{} {} at `{}` ({}): \"{text}\" names no {ty}",
                tree.type_name,
                tree.id,
                place.path,
                short(defs, place.span)
            );
            checked += 1;
        }
    }
    assert!(checked > 200, "only {checked} references");
}

#[test]
fn a_definition_lists_what_it_refers_to_and_what_refers_to_it() {
    let defs = bundled();
    let knife = defs.tree("item", "core:knife").expect("the knife");
    let text = describe(defs, knife, all());
    assert!(text.starts_with("item core:knife  defined at "), "{text}");
    assert!(
        text.contains("material core:steel  at `material`"),
        "{text}"
    );
    assert!(
        text.contains("damage_type core:cut  at `surfaces.kind`"),
        "{text}"
    );
    assert!(text.contains("channel core:arm  at `grip`"), "{text}");
    let steel = defs.tree("material", "core:steel").expect("steel");
    let text = describe(defs, steel, all());
    let (sure, mentions) = steel_counts();
    assert!(sure >= 4 && mentions >= 2, "{sure} {mentions}");
    assert!(
        text.contains(&format!("referred to by ({sure}):")),
        "{text}"
    );
    assert!(text.contains("item core:knife, at `material`"), "{text}");
    assert!(
        text.contains(&format!("named in plain text by ({mentions})")),
        "{text}"
    );
    assert!(text.contains("fixture core:lamp, at `material`"), "{text}");
}

#[test]
fn conditions_are_read() {
    assert_eq!(
        parse_condition("density>=5000"),
        Ok(Condition::Compare("density".into(), Op::Ge, "5000".into()))
    );
    assert_eq!(
        parse_condition("resist.cut.threshold < 10 MPa"),
        Ok(Condition::Compare(
            "resist.cut.threshold".into(),
            Op::Lt,
            "10 MPa".into()
        ))
    );
    assert_eq!(
        parse_condition("name~'stick'"),
        Ok(Condition::Compare(
            "name".into(),
            Op::Contains,
            "stick".into()
        ))
    );
    assert_eq!(parse_condition("glyph"), Ok(Condition::Has("glyph".into())));
    assert_eq!(
        parse_condition("!glyph"),
        Ok(Condition::Lacks("glyph".into()))
    );
    assert!(parse_condition("==3").is_err());
    assert!(parse_condition("a b").is_err());
}

#[test]
fn find_compares_fields_quantities_and_references() {
    let defs = bundled();
    let ids = |type_name: Option<&str>, conditions: &[&str]| -> Vec<String> {
        let conditions: Vec<Condition> = conditions
            .iter()
            .map(|text| match text.strip_prefix("where=") {
                Some(text) => parse_condition(text).expect("a condition"),
                None => Condition::Text((*text).to_owned()),
            })
            .collect();
        find(defs, engine_entries(), type_name, &conditions)
            .into_iter()
            .map(|tree| tree.id.clone())
            .collect()
    };
    let heavy = ids(Some("material"), &["where=density>5000"]);
    assert!(heavy.contains(&"core:steel".to_owned()), "{heavy:?}");
    assert!(!heavy.contains(&"core:leather".to_owned()), "{heavy:?}");
    // A reference, bare or qualified; the bundled items grow, so what is
    // checked is that the two conditions part every item between them.
    let of_steel = ids(Some("item"), &["where=material==steel"]);
    assert!(of_steel.contains(&"core:knife".to_owned()), "{of_steel:?}");
    assert!(!of_steel.contains(&"core:club".to_owned()), "{of_steel:?}");
    assert_eq!(ids(Some("item"), &["where=material==core:steel"]), of_steel);
    let not_steel = ids(Some("item"), &["where=material!=steel"]);
    assert!(not_steel.iter().all(|id| !of_steel.contains(id)));
    assert_eq!(
        of_steel.len() + not_steel.len(),
        ids(Some("item"), &[]).len(),
        "{of_steel:?} {not_steel:?}"
    );
    // A quantity, in SI units whatever it was written in.
    let tough = ids(Some("material"), &["where=resist.impact.threshold>=2MPa"]);
    assert!(tough.contains(&"core:leather".to_owned()), "{tough:?}");
    assert!(
        !ids(Some("material"), &["where=resist.impact.threshold>2000000"])
            .contains(&"core:leather".to_owned())
    );
    // Keys of a map, and words.
    assert!(ids(Some("material"), &["where=resist~heat"]).contains(&"core:leather".to_owned()));
    let knives = ids(None, &["knife"]);
    assert!(knives.contains(&"core:knife".to_owned()), "{knives:?}");
    assert!(!knives.contains(&"core:club".to_owned()), "{knives:?}");
    assert!(ids(None, &["no definition says this"]).is_empty());
    assert!(ids(Some("item"), &["where=!glyph"]).len() < ids(Some("item"), &[]).len());
}

#[test]
fn the_commands_print_what_they_find() {
    let run = |words: &[&str]| {
        let mut args: Vec<String> = words[1..].iter().map(|word| (*word).to_owned()).collect();
        args.push("--mods".to_owned());
        args.push(mods_dir().to_string_lossy().into_owned());
        crate::data::run(&crate::engine_types(), &[], words[0], &args).expect("a data command")
    };
    let output = run(&["refs", "steel"]);
    assert_eq!(output.code, EXIT_OK, "{}", output.err);
    let (sure, _) = steel_counts();
    assert!(
        output.out.contains(&format!("referred to by ({sure}):")),
        "{}",
        output.out
    );
    let output = run(&["refs", "material", "steal"]);
    assert_eq!(output.code, EXIT_FAILED);
    assert!(
        output.err.contains("did you mean `core:steel`?"),
        "{}",
        output.err
    );
    let output = run(&["find", "item", "where=material==steel"]);
    assert_eq!(output.code, EXIT_OK, "{}", output.err);
    assert!(
        output.out.contains("item core:knife  \"knife\""),
        "{}",
        output.out
    );
    let listed = output
        .out
        .lines()
        .filter(|line| line.starts_with("item "))
        .count();
    assert!(
        output.out.ends_with(&format!("\n{listed} definitions\n")),
        "{}",
        output.out
    );
    let of_steel = incoming("material", "core:steel")
        .into_iter()
        .filter(|reference| reference.kind == RefKind::Field && reference.from_type == "item")
        .count();
    assert_eq!(listed, of_steel, "{}", output.out);
    let output = run(&["find", "iten"]);
    assert!(
        output.err.contains("did you mean `item`?"),
        "{}",
        output.err
    );
    let output = run(&["find", "item", "where=a b"]);
    assert_eq!(output.code, crate::data::EXIT_USAGE);
}
