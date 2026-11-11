//! Mods, profiles, load order, the `engine` namespace and lenient
//! loading (04 §3, §4.4, §8.3, §14): packages F4, P1 and P3.

mod common;

use std::path::{Path, PathBuf};

use clash_core::{Dur, Real};
use clash_data::{DefTypes, Defs, LoadOutcome, Loader};
use common::*;

fn material<'a>(defs: &'a Defs, id: &str) -> &'a MaterialDef {
    let registry = defs.registry::<MaterialDef>();
    registry.get(
        registry
            .lookup(id)
            .unwrap_or_else(|| panic!("no material {id}")),
    )
}

/// The on-disk fixtures by a path relative to the crate, which is where
/// tests run, so that the paths in diagnostics are the same everywhere.
fn fixture_mods(set: &str) -> PathBuf {
    Path::new("tests").join("fixtures").join(set).join("mods")
}

fn loaded_ids(defs: &Defs) -> Vec<&str> {
    defs.mods()
        .iter()
        .map(|one| one.manifest.id.as_str())
        .collect()
}

#[test]
fn a_fixture_mod_set_loads_from_disk() {
    let types = types();
    let outcome = Loader::new(&types)
        .search_path(fixture_mods("good"))
        .load_profile("default");
    assert_known_codes(&outcome.diagnostics);
    let defs = clean(outcome);

    assert_eq!(loaded_ids(&defs), ["core", "ashlands", "tweaks"]);
    assert_eq!(defs.mods()[1].manifest.name, "The Ashlands");
    assert_eq!(defs.mods()[1].root, "tests/fixtures/good/mods/ashlands");
    let profile = defs.profile().expect("loaded from a profile");
    assert_eq!(profile.id, "default");
    assert_eq!(profile.name, "Fixture game");
    assert_eq!(profile.boot_screen.as_ref().unwrap().0, "ashlands:title");
    assert_eq!(
        profile.settings.as_ref().unwrap().to_json(),
        r#"{"core:reaction_prompts":"threats"}"#
    );

    // Ids are namespaced, and numbered in sorted order per type.
    assert_eq!(
        defs.registry::<MaterialDef>().names(),
        [
            "engine:none",
            "ashlands:ash",
            "ashlands:cinder",
            "core:concrete",
            "core:granite"
        ]
    );
    assert_eq!(
        defs.registry::<BlockDef>().names(),
        ["ashlands:ash_heap", "core:solid", "core:wall"]
    );
    assert_eq!(
        defs.registry::<RuleDef>().names(),
        ["ashlands:sift#phases.sift.on_end"]
    );

    // Inheritance, patches from two mods, and a patch to a parent.
    let granite = material(&defs, "core:granite");
    assert_eq!(granite.density, Real::of(2600.0 * 1.1));
    assert_eq!(granite.bearing.unwrap().get(), Real::of(1.3e8));
    assert_eq!(granite.span, 4);
    assert_eq!(material(&defs, "core:concrete").density, Real::of(2400.0));
    let cinder = material(&defs, "ashlands:cinder");
    assert_eq!(cinder.density, Real::of(2600.0 * 1.1));
    assert_eq!(cinder.toughness, Real::of(1.0e6));
    assert_eq!(cinder.span, 3);
    let tags: Vec<&str> = cinder
        .tags
        .iter()
        .map(|tag| defs.tags().name(tag))
        .collect();
    assert_eq!(tags, ["burnt", "mineral"]);

    let blocks = defs.registry::<BlockDef>();
    let wall = blocks.get(blocks.lookup("core:wall").unwrap());
    assert_eq!(wall.fill, Real::of(0.25));
    assert_eq!(wall.height.get(), Real::of(2.5));
    let heap = blocks.get(blocks.lookup("ashlands:ash_heap").unwrap());
    assert_eq!(heap.height.get(), Real::of(0.5));
    assert_eq!(
        defs.registry::<MaterialDef>().name(heap.material.id()),
        "ashlands:ash"
    );

    let actions = defs.registry::<ActionDef>();
    let sift = actions.get(actions.lookup("ashlands:sift").unwrap());
    assert_eq!(sift.phases[1].time.dur(), Dur::from_secs(1));
    assert_eq!(sift.name.raw(), "Sift the ash");
    // A text remembers where it was written.
    let span = sift.name.span().unwrap();
    assert_eq!(
        defs.sources().locate(span).short(),
        "tests/fixtures/good/mods/ashlands/data/ash.json:19:45"
    );

    // The same files give the same ids and fingerprint on every load.
    let again = Loader::new(&types)
        .search_path(fixture_mods("good"))
        .load_profile("default")
        .defs
        .unwrap();
    assert_eq!(again.fingerprint(), defs.fingerprint());
    assert_eq!(
        again.registry::<MaterialDef>().names(),
        defs.registry::<MaterialDef>().names()
    );
}

#[test]
fn a_profile_is_found_by_file_by_name_and_inside_a_mod() {
    let types = types();
    let by_file = Loader::new(&types)
        .search_path(fixture_mods("good"))
        .load_profile_file(&Path::new("tests/fixtures/good/profiles").join("default.json"));
    assert_eq!(loaded_ids(&clean(by_file)), ["core", "ashlands", "tweaks"]);

    // Inside a mod: `mods/<mod>/profiles/<name>.json`.
    let inside = Loader::new(&types)
        .search_path(fixture_mods("good"))
        .load_profile("ashlands_game");
    let defs = clean(inside);
    assert_eq!(loaded_ids(&defs), ["core", "ashlands"]);
    assert_eq!(defs.profile().unwrap().name, "The Ashlands");
    // Without the third mod its patches are not applied.
    assert_eq!(material(&defs, "core:granite").density, Real::of(2600.0));

    let missing = Loader::new(&types)
        .search_path(fixture_mods("good"))
        .load_profile("nonesuch");
    assert!(missing.defs.is_none());
    let diagnostic = only(&missing, "E0008");
    assert_eq!(
        diagnostic.message,
        "no profile named \"nonesuch\" was found"
    );
    assert_eq!(
        diagnostic.notes[0].message,
        "looked for `tests/fixtures/good/profiles/nonesuch.json`"
    );

    let unreadable = Loader::new(&types).load_profile_file(Path::new("tests/fixtures/none.json"));
    assert!(
        only(&unreadable, "E0008")
            .message
            .starts_with("cannot read the profile")
    );
}

#[test]
fn a_profile_with_zero_mods_builds_empty_valid_defs() {
    let types = types();
    for outcome in [
        Loader::new(&types).load_mods(&[]),
        Loader::new(&types).load_all(),
        Loader::new(&types)
            .search_path(fixture_mods("good"))
            .load_profile("bare"),
        Loader::new(&types)
            .load_profile_text("profiles/empty.json", r#"{ "id": "empty", "mods": [] }"#),
    ] {
        let defs = clean(outcome);
        assert!(defs.mods().is_empty());
        assert!(defs.registry::<BlockDef>().is_empty());
        assert!(defs.registry::<ActionDef>().is_empty());
        assert!(defs.tags().is_empty());
        assert!(defs.trees().is_empty());
        assert!(defs.experimental_features().is_empty());
        // The engine's own definitions are there with no mods at all:
        // dense id 0 of the type is `engine:none` (04 §4.4, M7).
        let materials = defs.registry::<MaterialDef>();
        assert_eq!(materials.names(), ["engine:none"]);
        let none = materials.lookup("engine:none").unwrap();
        assert_eq!(none.index(), 0);
        assert_eq!(materials.get(none).density, Real::ONE);
        assert!(!materials.is_placeholder(none));
    }
    // With no definition types at all, too.
    let nothing = DefTypes::new();
    let defs = clean(Loader::new(&nothing).load_mods(&[]));
    assert!(defs.type_names().is_empty());
    assert!(defs.try_registry::<MaterialDef>().is_none());
}

#[test]
fn a_total_conversion_lists_none_of_the_bundled_mods() {
    let types = types();
    let outcome = loader(
        &types,
        &[
            test_mod(
                "core",
                "",
                &[(
                    "data/a.json",
                    r#"[ { "type": "damage_type", "id": "impact" } ]"#,
                )],
            ),
            test_mod(
                "my_game",
                "",
                &[(
                    "data/a.json",
                    r#"[ { "type": "damage_type", "id": "doom" } ]"#,
                )],
            ),
        ],
    )
    .load_profile_text(
        "mods/my_game/profiles/my_game.json",
        r#"{ "id": "my_game", "name": "My Game", "mods": ["my_game"],
            "boot_screen": "my_game:title",
            "screens": { "pause": "my_game:pause" },
            "preferences": { "tiles": false } }"#,
    );
    let defs = clean(outcome);
    // Nothing of the bundled mod is loaded: only what the profile lists.
    assert_eq!(loaded_ids(&defs), ["my_game"]);
    assert_eq!(defs.registry::<DamageTypeDef>().names(), ["my_game:doom"]);
    let profile = defs.profile().unwrap();
    assert_eq!(profile.boot_screen.as_ref().unwrap().0, "my_game:title");
    assert_eq!(
        profile.screens.as_ref().unwrap().to_json(),
        r#"{"pause":"my_game:pause"}"#
    );
    assert_eq!(
        profile.preferences.as_ref().unwrap().to_json(),
        r#"{"tiles":false}"#
    );
}

fn order_fixture() -> Vec<TestMod> {
    vec![
        test_mod(
            "core",
            "",
            &[(
                "data/a.json",
                r#"[ { "type": "material", "id": "stone", "density": 1, "toughness": 1 } ]"#,
            )],
        ),
        test_mod(
            "a",
            r#""after": ["core"]"#,
            &[(
                "data/a.json",
                r#"[ { "type": "material", "id": "core:stone", "patch": true, "density": 10 } ]"#,
            )],
        ),
        test_mod(
            "b",
            r#""after": ["core"]"#,
            &[(
                "data/a.json",
                r#"[ { "type": "material", "id": "core:stone", "patch": true, "density": 20 } ]"#,
            )],
        ),
    ]
}

fn load_listed(mods: &[TestMod], listed: &[&str]) -> LoadOutcome {
    let types = types();
    let list: Vec<String> = listed.iter().map(|id| format!("\"{id}\"")).collect();
    let profile = format!("{{ \"id\": \"p\", \"mods\": [{}] }}", list.join(", "));
    let outcome = loader(&types, mods).load_profile_text("profiles/p.json", &profile);
    assert_known_codes(&outcome.diagnostics);
    outcome
}

#[test]
fn ties_in_load_order_are_broken_by_position_in_the_profile() {
    let mods = order_fixture();
    // The two patches have no order between them: the profile decides,
    // and the later one wins.
    let a_then_b = load_listed(&mods, &["core", "a", "b"]);
    let defs = a_then_b.defs.as_ref().unwrap();
    assert_eq!(loaded_ids(defs), ["core", "a", "b"]);
    assert_eq!(material(defs, "core:stone").density, Real::of(20.0));
    assert_eq!(
        only(&a_then_b, "W0410").message,
        "mods \"a\" and \"b\" both replace `density` of material core:stone; \"b\" wins"
    );

    let b_then_a = load_listed(&mods, &["core", "b", "a"]);
    let defs = b_then_a.defs.as_ref().unwrap();
    assert_eq!(loaded_ids(defs), ["core", "b", "a"]);
    assert_eq!(material(defs, "core:stone").density, Real::of(10.0));
    assert_eq!(
        only(&b_then_a, "W0410").message,
        "mods \"b\" and \"a\" both replace `density` of material core:stone; \"a\" wins"
    );

    // What must come first does, wherever the profile lists it.
    let core_last = load_listed(&mods, &["b", "a", "core"]);
    assert_eq!(
        loaded_ids(core_last.defs.as_ref().unwrap()),
        ["core", "b", "a"]
    );

    // With no profile, ties are broken by id.
    let by_id = load(&order_fixture());
    assert_eq!(loaded_ids(by_id.defs.as_ref().unwrap()), ["core", "a", "b"]);
}

#[test]
fn requires_and_present_after_come_first() {
    let mods = [
        test_mod("zeta", "", &[]),
        test_mod(
            "beta",
            r#""requires": ["zeta"], "after": ["omega", "absent"]"#,
            &[],
        ),
        test_mod("omega", "", &[]),
        test_mod(
            "alpha",
            r#""requires": [{ "id": "beta", "version": ">=0" }]"#,
            &[],
        ),
    ];
    let defs = clean(load(&mods));
    assert_eq!(loaded_ids(&defs), ["omega", "zeta", "beta", "alpha"]);
    // `after` names a mod that is not loaded: no error, no effect.
    let defs = clean(load_listed(&mods, &["alpha", "beta", "zeta"]));
    assert_eq!(loaded_ids(&defs), ["zeta", "beta", "alpha"]);
}

#[test]
fn a_dependency_cycle_names_the_mods_in_it() {
    let mods = [
        test_mod("a", r#""requires": ["b"]"#, &[]),
        test_mod("b", r#""after": ["c"]"#, &[]),
        test_mod("c", r#""requires": ["a"]"#, &[]),
        test_mod("d", r#""requires": ["a"]"#, &[]),
        test_mod("free", "", &[]),
    ];
    let outcome = load(&mods);
    assert!(outcome.defs.is_none());
    let cycle = only(&outcome, "E0007");
    assert_eq!(
        cycle.message,
        "mods depend on each other in a circle: a -> b -> c -> a"
    );
    assert_eq!(cycle.location.as_ref().unwrap().file, "mods/a/mod.json");
    assert_eq!(outcome.diagnostics.len(), 1);
}

#[test]
fn what_a_profile_needs_must_be_there() {
    let mods = [
        test_mod("core", r#""version": "0.2""#, &[]),
        test_mod("needy", r#""requires": ["core", "ghost"]"#, &[]),
        test_mod(
            "picky",
            r#""requires": [{ "id": "core", "version": ">=1.0" }], "engine": ">=99""#,
            &[],
        ),
        test_mod("rival", r#""conflicts": ["core"]"#, &[]),
        test_mod("loner", r#""requires": ["core"]"#, &[]),
    ];
    let outcome = load_listed(&mods, &["core", "needy", "picky", "rival", "coer"]);
    assert_eq!(
        summary(&outcome),
        [
            (
                "E0004".to_owned(),
                "mod \"needy\" requires \"ghost\", which is not installed".to_owned()
            ),
            (
                "E0005".to_owned(),
                format!(
                    "mod \"picky\" asks for engine >=99, and this is engine {}",
                    clash_data::ENGINE_VERSION
                )
            ),
            (
                "E0005".to_owned(),
                "mod \"picky\" requires \"core\" >=1.0, and the one present is 0.2".to_owned()
            ),
            (
                "E0006".to_owned(),
                "mod \"rival\" may not be loaded alongside \"core\"".to_owned()
            ),
            (
                "E0004".to_owned(),
                "profile \"p\" lists mod \"coer\", which was not found".to_owned()
            ),
        ]
    );
    let missing = &outcome.diagnostics.as_slice()[4];
    assert_eq!(
        missing.suggestion.as_deref(),
        Some("did you mean \"core\"?")
    );
    assert_eq!(missing.location.as_ref().unwrap().file, "profiles/p.json");

    // A required mod that is installed but not listed is not added
    // silently: the profile is the whole list.
    let outcome = load_listed(&mods, &["loner"]);
    let unlisted = only(&outcome, "E0004");
    assert_eq!(
        unlisted.message,
        "mod \"loner\" requires \"core\", which is not in the profile"
    );
    assert_eq!(
        unlisted.suggestion.as_deref(),
        Some("add \"core\" to the profile's `mods`")
    );
}

#[test]
fn a_bad_profile_is_reported() {
    let types = types();
    let load_text = |text: &str| Loader::new(&types).load_profile_text("profiles/p.json", text);
    let outcome = load_text(r#"{ "id": "p", "mods": ["a", "a"], "boot": "x", "name": 3 }"#);
    let messages: Vec<&str> = outcome
        .diagnostics
        .iter()
        .map(|d| d.message.as_str())
        .collect();
    assert_eq!(
        messages,
        [
            "profile \"p\" lists mod \"a\", which was not found",
            "mod \"a\" is listed twice in the profile",
            "unknown key \"boot\" in a profile",
            "`name` in a profile must be a string, but this is a number",
        ]
    );
    assert_eq!(
        outcome.diagnostics.as_slice()[2].suggestion.as_deref(),
        Some("did you mean \"boot_screen\"?")
    );
    assert_eq!(
        only(&load_text(r#"{ "mods": [] }"#), "E0001").message,
        "a profile has no `id`"
    );
    assert_eq!(
        only(&load_text(r#"{ "id": "p" }"#), "E0001").message,
        "a profile has no `mods`"
    );
    assert_eq!(
        only(&load_text("[1, 2,]"), "E0101").message,
        "a comma must be followed by another value"
    );
}

#[test]
fn the_first_search_directory_to_hold_a_mod_supplies_it() {
    let types = types();
    let outcome = Loader::new(&types)
        .search_path(fixture_mods("user"))
        .search_path(fixture_mods("good"))
        .load_mods(&["core"]);
    let shadowed = only(&outcome, "W0011");
    assert_eq!(
        shadowed.message,
        "mod \"core\" in `tests/fixtures/good/mods/core` is hidden by the one in `tests/fixtures/user/mods/core`"
    );
    let defs = outcome.defs.as_ref().unwrap();
    assert_eq!(material(defs, "core:granite").density, Real::of(9999.0));
    assert_eq!(defs.mods()[0].manifest.name, "A working copy of core");

    // Mods in memory come before any directory.
    let outcome = loader(
        &types,
        &[test_mod(
            "core",
            "",
            &[(
                "data/a.json",
                r#"[ { "type": "material", "id": "granite", "density": 7, "toughness": 1 } ]"#,
            )],
        )],
    )
    .search_path(fixture_mods("good"))
    .load_mods(&["core"]);
    assert_eq!(
        material(outcome.defs.as_ref().unwrap(), "core:granite").density,
        Real::of(7.0)
    );
}

#[test]
fn two_directories_with_one_mod_id_are_an_error() {
    let types = types();
    let outcome = Loader::new(&types)
        .search_path(fixture_mods("dupes"))
        .load_all();
    assert!(outcome.defs.is_none());
    let duplicate = only(&outcome, "E0003");
    assert_eq!(
        duplicate.message,
        "mod \"same\" is declared by two directories: `tests/fixtures/dupes/mods/one` and `tests/fixtures/dupes/mods/two`"
    );
    // A directory not named after its mod is a warning.
    let misnamed: Vec<&str> = outcome
        .diagnostics
        .iter()
        .filter(|d| d.code == "W0010")
        .map(|d| d.message.as_str())
        .collect();
    assert_eq!(
        misnamed,
        [
            "mod \"same\" is in a directory named `one`",
            "mod \"same\" is in a directory named `two`"
        ]
    );
}

#[test]
fn a_directory_that_cannot_be_read_is_a_diagnostic() {
    let types = types();
    let outcome = Loader::new(&types)
        .search_path("tests/fixtures/no_such_directory")
        .load_all();
    assert!(
        only(&outcome, "E0106")
            .message
            .starts_with("cannot read the mods directory `tests/fixtures/no_such_directory`")
    );
    assert!(outcome.defs.is_none());
}

#[test]
fn one_mod_is_loaded_with_what_it_requires() {
    let types = types();
    let dir = fixture_mods("good").join("tweaks");
    let defs = clean(Loader::new(&types).load_mod_dir(&dir));
    assert_eq!(loaded_ids(&defs), ["core", "ashlands", "tweaks"]);
    assert!(defs.profile().is_none());

    let defs = clean(Loader::new(&types).load_mod_dir(&fixture_mods("good").join("core")));
    assert_eq!(loaded_ids(&defs), ["core"]);

    let defs = clean(
        Loader::new(&types)
            .search_path(fixture_mods("good"))
            .load_mod_with_requirements("ashlands"),
    );
    assert_eq!(loaded_ids(&defs), ["core", "ashlands"]);

    let outcome = Loader::new(&types)
        .search_path(fixture_mods("good"))
        .load_mod_with_requirements("ashland");
    let missing = only(&outcome, "E0004");
    assert_eq!(missing.message, "mod \"ashland\" was not found");
    assert_eq!(
        missing.suggestion.as_deref(),
        Some("did you mean \"ashlands\"?")
    );

    let outcome = Loader::new(&types).load_mod_dir(Path::new("tests/fixtures/good/mods/nothing"));
    assert!(outcome.defs.is_none());
    assert!(
        only(&outcome, "E0106")
            .message
            .contains("mods/nothing/mod.json")
    );
}

const HALF_BROKEN: &str = r#"[
  { "type": "damage_type", "id": "impact" },
  { "type": "material", "id": "good", "density": 100, "toughness": 1 },
  { "type": "material", "id": "typo", "density": 100, "thougness": 1 },
  { "type": "material", "id": "loop", "copy_from": "loop" },
  { "type": "block", "id": "uses_typo", "material": "typo", "height": 1 }
]"#;

#[test]
fn lenient_loading_replaces_each_broken_definition_with_a_placeholder() {
    let types = types();
    let mods = [test_mod("m", "", &[("data/a.json", HALF_BROKEN)])];

    // Errors prevent a profile from starting.
    let strict = loader(&types, &mods).load_all();
    assert!(strict.has_errors());
    assert!(strict.defs.is_none());
    assert!(strict.into_result().is_err());

    // Lenient: the same errors are reported, and loading carries on.
    let lenient = loader(&types, &mods).lenient(true).load_all();
    assert_known_codes(&lenient.diagnostics);
    assert!(lenient.has_errors());
    let codes: Vec<&str> = lenient
        .diagnostics
        .iter()
        .map(|d| d.code.as_str())
        .collect();
    assert_eq!(codes, ["E0202", "N0901", "E0203", "N0901", "E0405"]);
    let note = &lenient.diagnostics.as_slice()[1];
    assert_eq!(
        note.message,
        "material m:typo is broken and was replaced by a placeholder"
    );
    assert_eq!(note.severity, clash_data::Severity::Note);
    let defs = lenient.defs.expect("lenient loading gives definitions");
    let materials = defs.registry::<MaterialDef>();
    assert_eq!(
        materials.names(),
        ["engine:none", "m:good", "m:loop", "m:typo"]
    );
    let typo = materials.lookup("m:typo").unwrap();
    assert!(materials.is_placeholder(typo));
    assert!(materials.is_placeholder(materials.lookup("m:loop").unwrap()));
    assert!(!materials.is_placeholder(materials.lookup("m:good").unwrap()));
    // The placeholder is inert, and what refers to it still resolves.
    assert_eq!(materials.get(typo).density, Real::ONE);
    let blocks = defs.registry::<BlockDef>();
    let uses = blocks.get(blocks.lookup("m:uses_typo").unwrap());
    assert_eq!(uses.material.id(), typo);
    assert_eq!(material(&defs, "m:good").density, Real::of(100.0));
}

#[test]
fn lenient_loading_stops_at_a_type_with_no_placeholder() {
    let types = types();
    let mods = [test_mod(
        "m",
        "",
        &[(
            "data/a.json",
            r#"[ { "type": "block", "id": "b", "height": 1 } ]"#,
        )],
    )];
    let outcome = loader(&types, &mods).lenient(true).load_all();
    assert!(outcome.defs.is_none());
    assert_eq!(
        only(&outcome, "E0902").message,
        "block m:b is broken, and the type `block` has no placeholder to stand in for it"
    );
}

#[test]
fn bundled_mods_name_no_experimental_feature() {
    let bundled = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("mods");
    // No definition types are needed to read the manifests.
    let nothing = DefTypes::new();
    let mut names = Vec::new();
    for entry in std::fs::read_dir(&bundled).expect("the repository has a mods directory") {
        let entry = entry.unwrap();
        let manifest = entry.path().join("mod.json");
        if !manifest.is_file() {
            continue;
        }
        let text = std::fs::read_to_string(&manifest).unwrap();
        let name = entry.file_name().to_string_lossy().into_owned();
        let outcome = Loader::new(&nothing)
            .memory_mod(&name, &text, &[])
            .load_all();
        assert_eq!(outcome.diagnostics.render(), "", "{name}");
        let defs = outcome.defs.unwrap();
        assert!(
            defs.mods()[0].manifest.experimental.is_empty(),
            "bundled mod `{name}` names an experimental feature (04 §8.4)"
        );
        assert!(defs.experimental_features().is_empty());
        names.push(name);
    }
    assert!(names.contains(&"core".to_owned()));
}
