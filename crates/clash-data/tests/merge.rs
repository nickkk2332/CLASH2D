//! Inheritance, patching and overriding (04 §4, §5, §6): packages F4 and
//! P2.

mod common;

use clash_core::{Dur, Real};
use clash_data::{Defs, Node};
use common::*;

/// A finished tree as compact JSON.
fn tree(defs: &Defs, type_name: &str, id: &str) -> String {
    defs.tree(type_name, id)
        .unwrap_or_else(|| panic!("no {type_name} {id}"))
        .tree
        .to_json()
}

fn material<'a>(defs: &'a Defs, id: &str) -> &'a MaterialDef {
    let registry = defs.registry::<MaterialDef>();
    registry.get(
        registry
            .lookup(id)
            .unwrap_or_else(|| panic!("no material {id}")),
    )
}

const STONES: &str = r#"[
  { "type": "material", "id": "base_stone", "abstract": true,
    "density": 2600, "toughness": 1.5e6, "tags": ["mineral"] },
  { "type": "material", "id": "granite", "copy_from": "base_stone",
    "toughness": 2.5e6, "extend": { "tags": ["igneous"] } },
  { "type": "material", "id": "pumice", "copy_from": "granite",
    "density": 600, "delete": { "tags": ["mineral"] } }
]"#;

#[test]
fn copy_from_is_the_parents_finished_definition_with_changes() {
    let defs = clean(load(&[test_mod("core", "", &[("data/a.json", STONES)])]));
    assert_eq!(
        tree(&defs, "material", "core:granite"),
        r#"{"density":2600,"toughness":2500000.0,"tags":["mineral","igneous"]}"#
    );
    assert_eq!(
        tree(&defs, "material", "core:pumice"),
        r#"{"density":600,"toughness":2500000.0,"tags":["igneous"]}"#
    );
    let granite = material(&defs, "core:granite");
    assert_eq!(granite.density, Real::of(2600.0));
    assert_eq!(granite.tags.len(), 2);

    // An abstract definition is finished, and is not a definition.
    assert!(
        defs.tree("material", "core:base_stone")
            .unwrap()
            .is_abstract
    );
    assert_eq!(
        defs.registry::<MaterialDef>().names(),
        ["engine:none", "core:granite", "core:pumice"]
    );
}

#[test]
fn a_parent_may_be_defined_later_or_in_another_mod() {
    let child = test_mod(
        "ash",
        r#""requires": ["core"]"#,
        &[(
            "data/a.json",
            r#"[ { "type": "material", "id": "slag", "copy_from": "late", "density": 3000 },
                { "type": "material", "id": "late", "copy_from": "granite", "tags": [] } ]"#,
        )],
    );
    let defs = clean(load(&[
        test_mod("core", "", &[("data/a.json", STONES)]),
        child,
    ]));
    assert_eq!(
        tree(&defs, "material", "ash:slag"),
        r#"{"density":3000,"toughness":2500000.0,"tags":[]}"#
    );
}

#[test]
fn patching_a_parent_changes_its_children() {
    let tweak = test_mod(
        "tweak",
        r#""after": ["core"]"#,
        &[(
            "data/a.json",
            r#"[ { "type": "material", "id": "core:base_stone", "patch": true,
                  "density": 2700, "extend": { "tags": ["heavy"] } } ]"#,
        )],
    );
    let defs = clean(load(&[
        test_mod("core", "", &[("data/a.json", STONES)]),
        tweak,
    ]));
    assert_eq!(material(&defs, "core:granite").density, Real::of(2700.0));
    assert_eq!(
        tree(&defs, "material", "core:granite"),
        r#"{"density":2700,"toughness":2500000.0,"tags":["mineral","heavy","igneous"]}"#
    );
    // A child that sets the field itself keeps its own value.
    assert_eq!(material(&defs, "core:pumice").density, Real::of(600.0));
}

#[test]
fn an_override_discards_earlier_patches_and_accepts_later_ones() {
    let core = test_mod(
        "core",
        "",
        &[(
            "data/a.json",
            r#"[ { "type": "material", "id": "stone", "density": 2600, "toughness": 1e6 },
                { "type": "material", "id": "brick", "density": 1900, "toughness": 5e5 },
                { "type": "block", "id": "wall", "material": "stone", "height": 3.0, "fill": 0.2 } ]"#,
        )],
    );
    let early = test_mod(
        "a_early",
        r#""requires": ["core"]"#,
        &[(
            "data/a.json",
            r#"[ { "type": "block", "id": "core:wall", "patch": true, "fill": 0.9, "flags": ["opaque"] } ]"#,
        )],
    );
    let replace = test_mod(
        "b_replace",
        r#""requires": ["core"], "after": ["a_early"]"#,
        &[(
            "data/a.json",
            r#"[ { "type": "block", "id": "core:wall", "override": true, "material": "core:brick", "height": 2.5 } ]"#,
        )],
    );
    let late = test_mod(
        "c_late",
        r#""requires": ["core"], "after": ["b_replace"]"#,
        &[(
            "data/a.json",
            r#"[ { "type": "block", "id": "core:wall", "patch": true, "height": 2.0 } ]"#,
        )],
    );
    let defs = clean(load(&[core, early, replace, late]));
    assert_eq!(
        tree(&defs, "block", "core:wall"),
        r#"{"material":"core:brick","height":2.0}"#
    );
    let blocks = defs.registry::<BlockDef>();
    let wall = blocks.get(blocks.lookup("core:wall").unwrap());
    assert_eq!(
        defs.registry::<MaterialDef>().name(wall.material.id()),
        "core:brick"
    );
    assert_eq!(wall.fill, Real::ZERO);
    assert!(wall.flags.is_empty());
}

const STEEL: &str = r#"[
  { "type": "damage_type", "id": "impact" },
  { "type": "damage_type", "id": "cut" },
  { "type": "damage_type", "id": "blast" },
  { "type": "material", "id": "steel", "density": 7800, "toughness": 1e7,
    "tags": ["metal", "hard"],
    "resist": { "impact": { "threshold": "200 MPa", "factor": 0.5 },
                "cut": { "threshold": "1 GPa" } },
    "span": 6, "bearing": "250 MPa" }
]"#;

#[test]
fn each_operation_does_what_the_table_says() {
    let patch = test_mod(
        "tweak",
        r#""requires": ["core"]"#,
        &[(
            "data/a.json",
            r#"[ { "type": "material", "id": "core:steel", "patch": true,
                  "flammable": true,
                  "extend": { "tags": ["shiny"], "resist": { "blast": { "threshold": "50 MPa" } } },
                  "delete": { "tags": ["hard"], "resist": ["cut"] },
                  "scale": { "density": 1.5, "resist.impact.threshold": 2.0, "span": 2 },
                  "add": { "span": 1, "bearing": "50 MPa", "toughness": 5 } } ]"#,
        )],
    );
    let defs = clean(load(&[
        test_mod("core", "", &[("data/a.json", STEEL)]),
        patch,
    ]));
    assert_eq!(
        tree(&defs, "material", "core:steel"),
        concat!(
            r#"{"density":11700.0,"toughness":10000005.0,"tags":["metal","shiny"],"#,
            r#""resist":{"impact":{"threshold":"400000000 Pa","factor":0.5},"blast":{"threshold":"50 MPa"}},"#,
            r#""span":13,"bearing":"300000000 Pa","flammable":true}"#
        )
    );
    let steel = material(&defs, "core:steel");
    assert_eq!(steel.density, Real::of(11_700.0));
    assert_eq!(steel.toughness, Real::of(10_000_005.0));
    // Scale first, then add: 6 × 2 + 1.
    assert_eq!(steel.span, 13);
    // A value written with a unit is converted before either.
    assert_eq!(steel.bearing.unwrap().get(), Real::of(3.0e8));
    let damage = defs.registry::<DamageTypeDef>();
    let impact = clash_data::Ref::new(damage.lookup("core:impact").unwrap());
    let cut = clash_data::Ref::new(damage.lookup("core:cut").unwrap());
    let blast = clash_data::Ref::new(damage.lookup("core:blast").unwrap());
    assert_eq!(
        steel.resist[&impact].threshold.unwrap().get(),
        Real::of(4.0e8)
    );
    assert_eq!(
        steel.resist[&blast].threshold.unwrap().get(),
        Real::of(5.0e7)
    );
    assert!(!steel.resist.contains_key(&cut));
    assert!(steel.flammable);
    let tags: Vec<&str> = steel.tags.iter().map(|tag| defs.tags().name(tag)).collect();
    assert_eq!(tags, ["metal", "shiny"]);
}

#[test]
fn scale_and_add_wait_for_the_final_value() {
    // Whatever order the mods load in, every scale applies before every
    // add, and both after the replacing of the field.
    let core = || test_mod("core", "", &[("data/a.json", STEEL)]);
    let adder = |extra: &'static str| {
        test_mod(
            "adder",
            extra,
            &[(
                "data/a.json",
                r#"[ { "type": "material", "id": "core:steel", "patch": true, "add": { "span": 1 } } ]"#,
            )],
        )
    };
    let scaler = |extra: &'static str| {
        test_mod(
            "scaler",
            extra,
            &[(
                "data/a.json",
                r#"[ { "type": "material", "id": "core:steel", "patch": true, "scale": { "span": 3 } } ]"#,
            )],
        )
    };
    let setter = test_mod(
        "setter",
        r#""after": ["core", "adder", "scaler"]"#,
        &[(
            "data/a.json",
            r#"[ { "type": "material", "id": "core:steel", "patch": true, "span": 4 } ]"#,
        )],
    );
    let one = clean(load(&[
        core(),
        adder(r#""after": ["core"]"#),
        scaler(r#""after": ["core", "adder"]"#),
    ]));
    let other = clean(load(&[
        core(),
        scaler(r#""after": ["core"]"#),
        adder(r#""after": ["core", "scaler"]"#),
    ]));
    assert_eq!(material(&one, "core:steel").span, 6 * 3 + 1);
    assert_eq!(material(&other, "core:steel").span, 6 * 3 + 1);
    let set_last = clean(load(&[
        core(),
        adder(r#""after": ["core"]"#),
        scaler(r#""after": ["core"]"#),
        setter,
    ]));
    assert_eq!(material(&set_last, "core:steel").span, 4 * 3 + 1);
}

#[test]
fn scale_and_add_apply_after_inheritance() {
    let defs = clean(load(&[
        test_mod(
            "core",
            "",
            &[(
                "data/a.json",
                r#"[ { "type": "material", "id": "iron", "density": 7000, "toughness": 1e7, "span": 4 },
                    { "type": "material", "id": "wrought", "copy_from": "iron", "scale": { "span": 2 } } ]"#,
            )],
        ),
        test_mod(
            "tweak",
            r#""after": ["core"]"#,
            &[(
                "data/a.json",
                r#"[ { "type": "material", "id": "core:iron", "patch": true, "add": { "span": 1 } } ]"#,
            )],
        ),
    ]));
    // The parent is finished first: 4 + 1. The child scales that.
    assert_eq!(material(&defs, "core:iron").span, 5);
    assert_eq!(material(&defs, "core:wrought").span, 10);
}

#[test]
fn scale_and_add_act_on_a_declared_default() {
    let outcome = load_one(
        r#"[ { "type": "material", "id": "foam", "density": 30, "toughness": 100,
              "scale": { "span": 4 }, "add": { "span": 2 } },
            { "type": "material", "id": "bad", "density": 30, "toughness": 100,
              "add": { "bearing": "5 MPa" }, "scale": { "melts_at": 2 } } ]"#,
    );
    // `span` is left out and the type declares its default as 1.
    assert_eq!(
        summary(&outcome),
        [
            (
                "E0404".to_owned(),
                "cannot add to `bearing`: it has no value here and the type declares no default for it".to_owned()
            ),
            (
                "E0403".to_owned(),
                "cannot scale `melts_at`: it has no value here and the type declares no default for it".to_owned()
            ),
        ]
    );
    let lenient = {
        let types = types();
        loader(
            &types,
            &[test_mod(
                "m",
                "",
                &[(
                    "data/a.json",
                    r#"[ { "type": "material", "id": "foam", "density": 30, "toughness": 100,
                          "scale": { "span": 4 }, "add": { "span": 2 } } ]"#,
                )],
            )],
        )
        .load_all()
    };
    assert_eq!(material(&clean(lenient), "m:foam").span, 6);
}

const PUNCH: &str = r#"[
  { "type": "action", "id": "punch", "name": "Punch",
    "phases": [ { "id": "windup", "time": "300 ms" },
                { "id": "impact", "time": "50 ms" },
                { "id": "recovery", "time": "400 ms" } ] }
]"#;

fn phases(defs: &Defs, id: &str) -> Vec<(String, Dur)> {
    let actions = defs.registry::<ActionDef>();
    actions
        .get(actions.lookup(id).unwrap())
        .phases
        .iter()
        .map(|phase| (phase.id.clone(), phase.time.dur()))
        .collect()
}

fn phase(id: &str, millis: i64) -> (String, Dur) {
    (id.to_owned(), Dur::from_millis(millis))
}

#[test]
fn lists_of_objects_with_ids_are_matched_by_id() {
    let patch = test_mod(
        "tweak",
        r#""requires": ["core"]"#,
        &[(
            "data/a.json",
            r#"[ { "type": "action", "id": "core:punch", "patch": true,
                  "phases": [ { "id": "windup", "time": "200 ms" },
                              { "id": "feint", "time": "100 ms", "before": "impact" },
                              { "id": "gloat", "time": "1 s", "after": "recovery" },
                              { "id": "tail", "time": "10 ms" } ],
                  "scale": { "phases.recovery.time": 0.5 },
                  "delete": { "phases": ["impact"] } },
                { "type": "action", "id": "jab", "copy_from": "core:punch", "name": "Jab",
                  "phases": [ { "id": "windup", "time": "100 ms" } ],
                  "extend": { "phases": [ { "id": "snap", "time": "20 ms" } ] } } ]"#,
        )],
    );
    let defs = clean(load(&[
        test_mod("core", "", &[("data/a.json", PUNCH)]),
        patch,
    ]));
    // One phase is changed without restating the rest; new entries go
    // where they say, or at the end; `delete` takes entry ids; a dotted
    // path steps through an entry by its id.
    assert_eq!(
        phases(&defs, "core:punch"),
        [
            phase("windup", 200),
            phase("feint", 100),
            phase("recovery", 200),
            phase("gloat", 1000),
            phase("tail", 10),
        ]
    );
    // The same matching applies through `copy_from` and `extend`.
    assert_eq!(
        phases(&defs, "tweak:jab"),
        [
            phase("windup", 100),
            phase("feint", 100),
            phase("recovery", 200),
            phase("gloat", 1000),
            phase("tail", 10),
            phase("snap", 20),
        ]
    );
    let actions = defs.registry::<ActionDef>();
    assert_eq!(
        actions.get(actions.lookup("tweak:jab").unwrap()).name.raw(),
        "Jab"
    );
}

#[test]
fn a_plain_list_or_scalar_replaces_and_an_object_merges() {
    let patch = test_mod(
        "tweak",
        r#""requires": ["core"]"#,
        &[(
            "data/a.json",
            r#"[ { "type": "material", "id": "core:steel", "patch": true,
                  "tags": ["new"], "density": 1, "bearing": null,
                  "resist": { "impact": { "factor": 0.25 }, "blast": {} },
                  "name": "Steel" } ]"#,
        )],
    );
    let defs = clean(load(&[
        test_mod("core", "", &[("data/a.json", STEEL)]),
        patch,
    ]));
    // Keys keep the order they were written in, and keys added by a
    // patch come after those already there (04 §5).
    assert_eq!(
        tree(&defs, "material", "core:steel"),
        concat!(
            r#"{"density":1,"toughness":10000000.0,"tags":["new"],"#,
            r#""resist":{"impact":{"threshold":"200 MPa","factor":0.25},"cut":{"threshold":"1 GPa"},"blast":{}},"#,
            r#""span":6,"bearing":null,"name":"Steel"}"#
        )
    );
    assert!(material(&defs, "core:steel").bearing.is_none());
}

#[test]
fn each_merge_mistake_is_an_error_that_says_where() {
    let outcome = load_one(
        r#"[
  { "type": "damage_type", "id": "impact" },
  { "type": "material", "id": "steel", "density": 7800, "toughness": 1e7,
    "tags": ["metal"], "name": "Steel",
    "resist": { "impact": { "threshold": "200 MPa" } } },
  { "type": "material", "id": "steel", "patch": true,
    "delete": { "tags": ["metla"], "resist": ["cut"], "density": [1], "name": "x" },
    "extend": { "tags": { "a": 1 }, "density": [1], "name": 4, "resist.impact.threshold.deep": [1] },
    "scale": { "name": 2, "density": "2", "tags.metal": 2, "resist.cut.factor": 2 },
    "add": { "name": 1, "density": [1], "resist.impact.threshold": "3 kg" } },
  { "type": "action", "id": "punch", "name": "P",
    "phases": [ { "id": "windup", "time": 1 } ] },
  { "type": "action", "id": "punch", "patch": true,
    "phases": [ { "id": "feint", "time": 1, "before": "impcat" } ],
    "delete": { "phases": ["recovery"] } }
]"#,
    );
    assert!(outcome.defs.is_none());
    let shown: Vec<String> = outcome
        .diagnostics
        .iter()
        .map(|d| {
            format!(
                "{} {}:{} {}",
                d.code,
                d.location.as_ref().unwrap().line,
                d.location.as_ref().unwrap().column,
                d.message
            )
        })
        .collect();
    assert_eq!(
        shown,
        [
            "E0402 7:26 cannot delete \"metla\" from `tags`: it is not there",
            "E0402 7:47 cannot delete key \"cut\" from `resist`: it is not there",
            "E0402 7:55 cannot delete from `density`: it is a number, not a list or a map",
            "E0402 7:79 `delete` takes a list of what to remove from `name`, but this is a string",
            "E0401 8:25 cannot extend `tags` with an object: it is a list",
            "E0401 8:48 cannot extend `density` with a list: it is a number",
            "E0401 8:61 `extend` takes a list to append or a map to merge into `name`, but this is a number",
            "E0408 8:64 cannot extend `resist.impact.threshold.deep`: `resist.impact.threshold` is a string, which a path cannot step into",
            "E0403 9:16 cannot scale `name`: it is a string, not a number",
            "E0403 9:38 `scale` multiplies `density` by a number, but this is a string",
            "E0408 9:43 cannot scale `tags.metal`: `tags` is a list, which a path cannot step into",
            "E0408 9:60 cannot scale `resist.cut.factor`: `resist` has no `cut`",
            "E0404 10:14 cannot add to `name`: it is a string, not a number",
            "E0404 10:36 `add` adds a number or a quantity with a unit to `density`, but this is [1]",
            "E0404 10:68 cannot add a mass to `resist.impact.threshold`, which is a pressure",
            "E0409 14:55 no entry \"impcat\" to put \"feint\" before",
            "E0402 15:28 cannot delete \"recovery\" from `phases`: it is not there",
        ]
    );
    let first = &outcome.diagnostics.as_slice()[0];
    assert_eq!(first.suggestion.as_deref(), Some("did you mean \"metal\"?"));
    assert_eq!(first.path, "tags");
    assert_eq!(
        first.definition.as_ref().unwrap().to_string(),
        "material m:steel"
    );
    assert_eq!(
        outcome.diagnostics.as_slice()[14].path,
        "resist.impact.threshold"
    );
}

#[test]
fn a_scaled_quantity_still_has_its_dimension_checked() {
    let outcome = load_one(
        r#"[ { "type": "material", "id": "odd", "density": 1, "toughness": 1,
              "melts_at": "20 MPa", "scale": { "melts_at": 2 } } ]"#,
    );
    let wrong = only(&outcome, "E0205");
    assert_eq!(
        wrong.message,
        "this value is a pressure, but this field holds a temperature"
    );
}

#[test]
fn structure_mistakes_are_each_reported() {
    let core = test_mod(
        "core",
        "",
        &[(
            "data/a.json",
            r#"[ { "type": "material", "id": "stone", "density": 1, "toughness": 1 } ]"#,
        )],
    );
    let bad = test_mod(
        "bad",
        r#""requires": ["core"]"#,
        &[(
            "data/a.json",
            r#"[
  { "id": "no_type" },
  { "type": "material" },
  { "type": "materail", "id": "x" },
  { "type": "material", "id": "Bad Id", "density": 1, "toughness": 1 },
  { "type": "material", "id": "twice", "density": 1, "toughness": 1 },
  { "type": "material", "id": "twice", "density": 2, "toughness": 2 },
  { "type": "material", "id": "core:intruder", "density": 1, "toughness": 1 },
  { "type": "material", "id": "engine:none", "patch": true, "density": 5 },
  { "type": "material", "id": "core:stoen", "patch": true, "density": 5 },
  { "type": "material", "id": "core:absent", "patch": true, "optional": true, "density": 5 },
  { "type": "material", "id": "core:nothing", "override": true, "density": 1, "toughness": 1 },
  { "type": "material", "id": "both", "patch": true, "override": true },
  { "type": "material", "id": "flags", "abstract": "yes", "optional": true, "copy_from": 3,
    "extend": [], "if_mod": "core", "density": 1, "toughness": 1 },
  { "type": 5, "id": "x" },
  { "type": "material", "id": 5 },
  7,
  { "type": "material", "id": "gated", "if_mod": 3 }
]"#,
        )],
    );
    let outcome = load(&[core, bad]);
    let shown: Vec<String> = outcome
        .diagnostics
        .iter()
        .map(|d| {
            format!(
                "{} {} {}",
                d.code,
                d.location.as_ref().unwrap().line,
                d.message
            )
        })
        .collect();
    assert_eq!(
        shown,
        [
            "E0301 2 this definition has no `type`",
            "E0302 3 this material has no `id`",
            "E0303 4 unknown definition type \"materail\"",
            "E0304 5 \"Bad Id\" is not a valid id",
            "E0305 7 material \"bad:twice\" is already defined",
            "E0306 8 mod \"bad\" cannot define \"core:intruder\": that id is in the namespace of \"core\"",
            "E0306 9 the namespace \"engine\" is reserved for the engine",
            "E0307 10 nothing to patch: material \"core:stoen\" is not defined before this",
            "E0308 12 nothing to override: material \"core:nothing\" is not defined before this",
            "E0310 13 a definition cannot be both a patch and an override",
            "E0310 14 `abstract` is `true` or `false`, but this is a string",
            "E0310 14 `optional` only means something together with `\"patch\": true`",
            "E0310 14 `copy_from` is an id, but this is a number",
            "E0310 15 `extend` is an object whose keys are field paths, but this is a list",
            "E0301 16 `type` must be a string, but this is a number",
            "E0302 17 `id` must be a string, but this is a number",
            "E0107 18 a definition is an object, but this is a number",
            "E0310 19 `if_mod` is a mod id or a list of mod ids, but this is a number",
        ]
    );
    let by_code = |code: &str| {
        outcome
            .diagnostics
            .iter()
            .find(|d| d.code == code)
            .unwrap_or_else(|| panic!("no {code}"))
    };
    assert_eq!(
        by_code("E0303").suggestion.as_deref(),
        Some("did you mean \"material\"?")
    );
    // A duplicate says where the first definition is.
    let duplicate = by_code("E0305");
    assert_eq!(duplicate.notes[0].message, "first defined here");
    assert_eq!(duplicate.notes[0].location.as_ref().unwrap().line, 6);
    assert_eq!(
        by_code("E0307").suggestion.as_deref(),
        Some("did you mean \"core:stone\"?")
    );
}

#[test]
fn a_patch_must_come_after_what_it_patches() {
    // "early" sorts before "late" and declares no order, so it loads
    // first and its patch finds nothing yet.
    let early = test_mod(
        "early",
        "",
        &[(
            "data/a.json",
            r#"[ { "type": "material", "id": "late:stone", "patch": true, "density": 5 } ]"#,
        )],
    );
    let late = test_mod(
        "late",
        "",
        &[(
            "data/a.json",
            r#"[ { "type": "material", "id": "stone", "density": 1, "toughness": 1 } ]"#,
        )],
    );
    let outcome = load(&[early, late]);
    let misplaced = only(&outcome, "E0307");
    assert_eq!(
        misplaced.suggestion.as_deref(),
        Some("list the defining mod in `requires` or `after` so that it loads first")
    );
    assert_eq!(misplaced.notes[0].message, "it is defined later, here");
    assert_eq!(
        misplaced.notes[0].location.as_ref().unwrap().file,
        "mods/late/data/a.json"
    );
}

#[test]
fn inheritance_mistakes_are_each_reported() {
    let outcome = load_one(
        r#"[
  { "type": "material", "id": "a", "copy_from": "b", "density": 1 },
  { "type": "material", "id": "b", "copy_from": "c" },
  { "type": "material", "id": "c", "copy_from": "a" },
  { "type": "material", "id": "selfish", "copy_from": "selfish" },
  { "type": "material", "id": "orphan", "copy_from": "granit" },
  { "type": "material", "id": "granite", "density": 1, "toughness": 1 },
  { "type": "block", "id": "slab", "material": "granite", "height": 1 },
  { "type": "material", "id": "confused", "copy_from": "slab" },
  { "type": "material", "id": "child_of_broken", "copy_from": "orphan" }
]"#,
    );
    let shown: Vec<String> = outcome
        .diagnostics
        .iter()
        .map(|d| {
            format!(
                "{} {} {}",
                d.code,
                d.location.as_ref().unwrap().line,
                d.message
            )
        })
        .collect();
    assert_eq!(
        shown,
        [
            "E0405 4 definitions inherit from each other in a circle: m:a -> m:b -> m:c -> m:a",
            "E0405 5 definitions inherit from each other in a circle: m:selfish -> m:selfish",
            "E0406 6 no material \"granit\" to copy from",
            "E0407 9 \"m:slab\" is a block, and a material can only copy from a material",
        ]
    );
    let orphan = &outcome.diagnostics.as_slice()[2];
    assert_eq!(
        orphan.suggestion.as_deref(),
        Some("did you mean \"granite\"?")
    );
    assert_eq!(orphan.path, "copy_from");
    assert_eq!(orphan.location.as_ref().unwrap().column, 54);
    // Each broken definition is reported once, and what inherits from a
    // broken one is not reported again.
    assert_eq!(outcome.diagnostics.len(), 4);
}

#[test]
fn an_abstract_definition_is_only_for_copy_from() {
    let outcome = load_one(
        r#"[ { "type": "material", "id": "base", "abstract": true, "density": 1 },
            { "type": "material", "id": "real", "copy_from": "base", "toughness": 1 },
            { "type": "material", "id": "unused", "abstract": true, "thougness": 1 },
            { "type": "block", "id": "b", "material": "base", "height": 1 } ]"#,
    );
    // Never instantiated: an abstract definition need not be complete,
    // and is not read as its type at all.
    let refused = only(&outcome, "E0502");
    assert_eq!(
        refused.message,
        "material \"m:base\" is abstract and cannot be referred to"
    );
    let unused = only(&outcome, "W0801");
    assert_eq!(
        unused.message,
        "abstract material \"m:unused\" is never copied from"
    );
    assert_eq!(outcome.diagnostics.len(), 2);

    // `abstract` is not inherited: the child is a real definition.
    let defs = clean(load_one(
        r#"[ { "type": "material", "id": "base", "abstract": true, "density": 1 },
            { "type": "material", "id": "real", "copy_from": "base", "toughness": 1 } ]"#,
    ));
    assert_eq!(
        defs.registry::<MaterialDef>().names(),
        ["engine:none", "m:real"]
    );
    assert_eq!(
        tree(&defs, "material", "m:real"),
        r#"{"density":1,"toughness":1}"#
    );
}

#[test]
fn if_mod_and_unless_mod_include_an_object_by_the_profile() {
    let data = r#"[
      { "type": "material", "id": "stone", "density": 1, "toughness": 1 },
      { "type": "block", "id": "drift", "material": "stone", "height": 1, "if_mod": "weather" },
      { "type": "block", "id": "dry", "material": "stone", "height": 1, "unless_mod": "weather" },
      { "type": "block", "id": "both", "material": "stone", "height": 1, "if_mod": ["weather", "m"] },
      { "type": "no_such_type", "id": "skipped", "if_mod": "not_loaded" }
    ]"#;
    let without = clean(load(&[test_mod("m", "", &[("data/a.json", data)])]));
    assert_eq!(without.registry::<BlockDef>().names(), ["m:dry"]);
    let with = clean(load(&[
        test_mod("m", "", &[("data/a.json", data)]),
        test_mod("weather", "", &[]),
    ]));
    assert_eq!(with.registry::<BlockDef>().names(), ["m:both", "m:drift"]);
}

fn dispute(
    first_extra: &'static str,
    second_extra: &'static str,
    first: &'static str,
    second: &'static str,
) -> clash_data::LoadOutcome {
    load(&[
        test_mod("core", "", &[("data/a.json", STEEL)]),
        test_mod("first", first_extra, &[("data/a.json", first)]),
        test_mod("second", second_extra, &[("data/a.json", second)]),
    ])
}

#[test]
fn two_mods_replacing_one_field_is_a_warning_unless_declared() {
    let set_a = r#"[ { "type": "material", "id": "core:steel", "patch": true, "toughness": 2e7,
                       "resist": { "impact": { "factor": 0.1 } } } ]"#;
    let set_b = r#"[ { "type": "material", "id": "core:steel", "patch": true, "toughness": 3e7,
                       "resist": { "impact": { "factor": 0.9 } } } ]"#;

    // Neither knows of the other: the later wins, with a warning naming
    // both, the field and the winner.
    let outcome = dispute(r#""after": ["core"]"#, r#""after": ["core"]"#, set_a, set_b);
    let defs = outcome
        .defs
        .as_ref()
        .expect("a warning does not stop a load");
    assert_eq!(material(defs, "core:steel").toughness, Real::of(3.0e7));
    let messages: Vec<&str> = outcome
        .diagnostics
        .iter()
        .map(|d| d.message.as_str())
        .collect();
    assert_eq!(
        messages,
        [
            "mods \"first\" and \"second\" both replace `toughness` of material core:steel; \"second\" wins",
            "mods \"first\" and \"second\" both replace `resist.impact.factor` of material core:steel; \"second\" wins",
        ]
    );
    let warning = &outcome.diagnostics.as_slice()[0];
    assert_eq!(warning.code, "W0410");
    assert_eq!(
        warning.location.as_ref().unwrap().file,
        "mods/second/data/a.json"
    );
    assert_eq!(warning.notes[0].message, "\"first\" set it here");
    assert_eq!(
        warning.notes[0].location.as_ref().unwrap().file,
        "mods/first/data/a.json"
    );
    assert_eq!(
        warning.suggestion.as_deref(),
        Some("if \"second\" means to win, list \"first\" in its `requires` or `after`")
    );

    // The later lists the earlier in `after` or `requires`: deliberate.
    for knows in [
        r#""after": ["core", "first"]"#,
        r#""requires": ["core", "first"]"#,
    ] {
        let outcome = dispute(r#""after": ["core"]"#, knows, set_a, set_b);
        assert_eq!(outcome.diagnostics.render(), "");
        assert_eq!(
            material(&outcome.defs.unwrap(), "core:steel").toughness,
            Real::of(3.0e7)
        );
    }

    // The same value from both is no dispute.
    let outcome = dispute(r#""after": ["core"]"#, r#""after": ["core"]"#, set_a, set_a);
    assert_eq!(outcome.diagnostics.render(), "");

    // Two mods that scale, add to or extend the same field compose.
    let grow_a = r#"[ { "type": "material", "id": "core:steel", "patch": true,
                        "scale": { "toughness": 2 }, "add": { "span": 1 }, "extend": { "tags": ["a"] } } ]"#;
    let grow_b = r#"[ { "type": "material", "id": "core:steel", "patch": true,
                        "scale": { "toughness": 3 }, "add": { "span": 2 }, "extend": { "tags": ["b"] } } ]"#;
    let outcome = dispute(
        r#""after": ["core"]"#,
        r#""after": ["core"]"#,
        grow_a,
        grow_b,
    );
    assert_eq!(outcome.diagnostics.render(), "");
    let defs = outcome.defs.unwrap();
    let steel = material(&defs, "core:steel");
    assert_eq!(steel.toughness, Real::of(6.0e7));
    assert_eq!(steel.span, 9);
    assert_eq!(steel.tags.len(), 4);
}

/// The file and line a value of a finished tree was last written at.
fn origin(defs: &Defs, node: &Node) -> String {
    let location = defs.sources().locate(node.span);
    format!("{}:{}", location.file, location.line)
}

#[test]
fn every_value_of_a_finished_tree_remembers_where_it_came_from() {
    let core = test_mod(
        "core",
        "",
        &[(
            "data/stone.json",
            "[\n  { \"type\": \"material\", \"id\": \"base\", \"abstract\": true,\n    \"density\": 2600,\n    \"toughness\": 1.5e6,\n    \"tags\": [\"mineral\"] }\n]",
        )],
    );
    let ash = test_mod(
        "ash",
        r#""requires": ["core"]"#,
        &[(
            "data/ash.json",
            "[\n  { \"type\": \"material\", \"id\": \"cinder\", \"copy_from\": \"base\",\n    \"toughness\": 9.0e5,\n    \"extend\": { \"tags\": [\"burnt\"] } }\n]",
        )],
    );
    let tweak = test_mod(
        "tweak",
        r#""requires": ["core", "ash"]"#,
        &[(
            "data/tweak.json",
            "[\n  { \"type\": \"material\", \"id\": \"ash:cinder\", \"patch\": true,\n    \"span\": 3,\n    \"scale\": { \"density\": 0.5 } }\n]",
        )],
    );
    let defs = clean(load(&[core, ash, tweak]));
    let cinder = &defs.tree("material", "ash:cinder").unwrap().tree;
    assert_eq!(
        cinder.to_json(),
        r#"{"density":1300.0,"toughness":900000.0,"tags":["mineral","burnt"],"span":3}"#
    );
    // Inherited and then scaled: the scale is what set the value last.
    assert_eq!(
        origin(&defs, cinder.get("density").unwrap()),
        "mods/tweak/data/tweak.json:4"
    );
    // Set by the child.
    assert_eq!(
        origin(&defs, cinder.get("toughness").unwrap()),
        "mods/ash/data/ash.json:3"
    );
    // A list built from two files: each entry keeps its own origin.
    let tags = cinder.get("tags").unwrap().as_array().unwrap();
    assert_eq!(origin(&defs, &tags[0]), "mods/core/data/stone.json:5");
    assert_eq!(origin(&defs, &tags[1]), "mods/ash/data/ash.json:4");
    // Added by the patch.
    assert_eq!(
        origin(&defs, cinder.get("span").unwrap()),
        "mods/tweak/data/tweak.json:3"
    );
}

#[test]
fn an_error_in_an_inherited_field_points_at_the_line_that_supplied_it() {
    let outcome = load_one(
        "[\n  { \"type\": \"material\", \"id\": \"base\", \"abstract\": true,\n    \"density\": \"heavy\" },\n  { \"type\": \"material\", \"id\": \"child\", \"copy_from\": \"base\", \"toughness\": 1 }\n]",
    );
    let mismatch = only(&outcome, "E0201");
    assert_eq!(mismatch.definition.as_ref().unwrap().id, "m:child");
    let location = mismatch.location.as_ref().unwrap();
    assert_eq!((location.line, location.column), (3, 16));
}

#[test]
fn a_bare_reference_keeps_the_namespace_of_the_file_that_wrote_it() {
    // "wall" is copied into another mod; its bare "stone" still means
    // the stone of the mod that wrote it (04 §6.1).
    let core = test_mod(
        "core",
        "",
        &[(
            "data/a.json",
            r#"[ { "type": "material", "id": "stone", "density": 1, "toughness": 1 },
                { "type": "block", "id": "wall", "material": "stone", "height": 3 } ]"#,
        )],
    );
    let ash = test_mod(
        "ash",
        r#""requires": ["core"]"#,
        &[(
            "data/a.json",
            r#"[ { "type": "material", "id": "stone", "density": 2, "toughness": 2 },
                { "type": "block", "id": "low_wall", "copy_from": "wall", "height": 1 },
                { "type": "block", "id": "own_wall", "copy_from": "wall", "material": "stone" } ]"#,
        )],
    );
    let outcome = load(&[core, ash]);
    let defs = outcome.defs.as_ref().unwrap();
    let blocks = defs.registry::<BlockDef>();
    let materials = defs.registry::<MaterialDef>();
    let made_of =
        |block: &str| materials.name(blocks.get(blocks.lookup(block).unwrap()).material.id());
    assert_eq!(made_of("ash:low_wall"), "core:stone");
    assert_eq!(made_of("ash:own_wall"), "ash:stone");
}
