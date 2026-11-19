//! Typed loading, linking and the validation stages (04 §7, §8, §9):
//! packages F4 and P4.

mod common;

use clash_core::{DenseId, Dur, Real};
use clash_data::{Defs, Limits, Loader};
use common::*;

fn material<'a>(defs: &'a Defs, id: &str) -> &'a MaterialDef {
    let registry = defs.registry::<MaterialDef>();
    registry.get(
        registry
            .lookup(id)
            .unwrap_or_else(|| panic!("no material {id}")),
    )
}

fn action<'a>(defs: &'a Defs, id: &str) -> &'a ActionDef {
    let registry = defs.registry::<ActionDef>();
    registry.get(
        registry
            .lookup(id)
            .unwrap_or_else(|| panic!("no action {id}")),
    )
}

#[test]
fn a_definition_loads_into_its_struct() {
    let defs = clean(load_one(
        r#"[
          { "type": "damage_type", "id": "impact" },
          { "type": "damage_type", "id": "blast", "name": "Blast" },
          { "//": "Anything under a // key is a comment.",
            "type": "material", "id": "concrete",
            "density": 2400, "toughness": 1.0e6,
            "resist": {
              "impact": { "threshold": "20 MPa" },
              "blast":  { "threshold": "100 MPa", "factor": 0.5 }
            },
            "span": 4, "bearing": "30 MPa", "melts_at": "1200 C",
            "flammable": false, "tags": ["mineral", "poured", "mineral"],
            "name": "Concrete" },
          { "type": "block", "id": "wall", "material": "concrete", "height": 3.0, "fill": 0.2,
            "flags": ["opaque", "load_bearing"], "shape": "full", "size": [1, 3] },
          { "type": "block", "id": "ramp", "material": "m:concrete", "height": "150 cm",
            "shape": { "slope": { "rise": "50 cm" } } },
          { "type": "block", "id": "post", "material": "concrete", "height": 3,
            "shape": { "pillar": "20 cm" } }
        ]"#,
    ));
    let concrete = material(&defs, "m:concrete");
    assert_eq!(concrete.density, Real::of(2400.0));
    assert_eq!(concrete.toughness, Real::of(1.0e6));
    assert_eq!(concrete.span, 4);
    assert_eq!(concrete.bearing.unwrap().get(), Real::of(3.0e7));
    assert_eq!(concrete.melts_at.unwrap().get(), Real::of(1473.15));
    assert_eq!(concrete.name.as_ref().unwrap().raw(), "Concrete");

    // A map keyed by references.
    let damage = defs.registry::<DamageTypeDef>();
    let impact = clash_data::Ref::new(damage.lookup("m:impact").unwrap());
    let blast = clash_data::Ref::new(damage.lookup("m:blast").unwrap());
    assert_eq!(concrete.resist.len(), 2);
    assert_eq!(
        concrete.resist[&impact].threshold.unwrap().get(),
        Real::of(2.0e7)
    );
    assert_eq!(concrete.resist[&impact].factor, Real::ONE);
    assert_eq!(concrete.resist[&blast].factor, Real::of(0.5));

    // Tags are interned in sorted order and a repeat is ignored.
    assert_eq!(defs.tags().names(), ["mineral", "poured"]);
    assert_eq!(concrete.tags.len(), 2);
    assert!(
        concrete
            .tags
            .contains(defs.tags().lookup("poured").unwrap())
    );
    assert_eq!(
        defs.tags().name(defs.tags().lookup("mineral").unwrap()),
        "mineral"
    );
    assert!(defs.tags().lookup("wooden").is_none());

    // References resolve to dense ids, bare or qualified.
    let blocks = defs.registry::<BlockDef>();
    let wall = blocks.get(blocks.lookup("m:wall").unwrap());
    assert_eq!(
        defs.registry::<MaterialDef>().name(wall.material.id()),
        "m:concrete"
    );
    assert_eq!(wall.height.get(), Real::of(3.0));
    assert_eq!(wall.flags, [Flag::Opaque, Flag::LoadBearing]);
    assert_eq!(wall.shape, Some(Shape::Full));
    assert_eq!(wall.size, Some((1, 3)));
    let ramp = blocks.get(blocks.lookup("m:ramp").unwrap());
    assert_eq!(ramp.material, wall.material);
    assert_eq!(ramp.height.get(), Real::of(1.5));
    assert_eq!(
        ramp.shape,
        Some(Shape::Slope {
            rise: clash_data::Length(Real::of(0.5))
        })
    );
    let post = blocks.get(blocks.lookup("m:post").unwrap());
    assert_eq!(
        post.shape,
        Some(Shape::Pillar(clash_data::Length(Real::of(0.2))))
    );
}

#[test]
fn dense_ids_are_assigned_in_sorted_order_of_qualified_id() {
    let data_b =
        r#"[ { "type": "damage_type", "id": "zap" }, { "type": "damage_type", "id": "cut" } ]"#;
    let data_a = r#"[ { "type": "damage_type", "id": "pierce" }, { "type": "damage_type", "id": "blast" },
                      { "type": "material", "id": "ash", "density": 600, "toughness": 2e4 } ]"#;
    let defs = clean(load(&[
        test_mod("zeta", "", &[("data/z.json", data_b)]),
        test_mod("alpha", "", &[("data/a.json", data_a)]),
    ]));
    let damage = defs.registry::<DamageTypeDef>();
    assert_eq!(
        damage.names(),
        ["alpha:blast", "alpha:pierce", "zeta:cut", "zeta:zap"]
    );
    let ids: Vec<usize> = damage.ids().map(DenseId::index).collect();
    assert_eq!(ids, [0, 1, 2, 3]);
    let listed: Vec<&str> = damage.iter().map(|(id, _)| damage.name(id)).collect();
    assert_eq!(listed, damage.names());
    assert_eq!(damage.len(), 4);
    assert!(!damage.is_empty());
    assert_eq!(damage.lookup("zeta:cut").map(DenseId::index), Some(2));
    assert_eq!(damage.lookup("cut"), None);
    assert_eq!(
        defs.lookup::<DamageTypeDef>("alpha:blast")
            .map(DenseId::index),
        Some(0)
    );
    assert_eq!(defs.ids_of("damage_type").unwrap().len(), 4);
    assert!(defs.ids_of("nonsense").is_none());

    // The engine's builtin takes dense id 0 of its type (04 §4.4).
    let materials = defs.registry::<MaterialDef>();
    assert_eq!(materials.names(), ["engine:none", "alpha:ash"]);
    assert_eq!(materials.lookup("engine:none").map(DenseId::index), Some(0));
    assert_eq!(
        defs.type_names(),
        [
            "action",
            "block",
            "damage_type",
            "gizmo",
            "material",
            "rule"
        ]
    );
}

#[test]
fn ids_do_not_depend_on_the_order_files_and_mods_are_given_in() {
    let a = (
        "data/a.json",
        r#"[ { "type": "material", "id": "ash", "density": 600, "toughness": 2e4, "tags": ["soft", "grey"] } ]"#,
    );
    let b = (
        "data/sub/b.json",
        r#"[ { "type": "material", "id": "bone", "density": 1900, "toughness": 1e5, "tags": ["hard"] } ]"#,
    );
    let c = (
        "data/c.json",
        r#"{ "type": "block", "id": "heap", "material": "ash", "height": 1 }"#,
    );
    let other = (
        "data/o.json",
        r#"[ { "type": "material", "id": "clay", "density": 1700, "toughness": 5e4 } ]"#,
    );
    let first = clean(load(&[
        test_mod("one", "", &[a, b, c]),
        test_mod("two", "", &[other]),
    ]));
    let second = clean(load(&[
        test_mod("two", "", &[other]),
        test_mod("one", "", &[c, b, a]),
    ]));
    assert_eq!(
        first.registry::<MaterialDef>().names(),
        second.registry::<MaterialDef>().names()
    );
    assert_eq!(first.tags(), second.tags());
    assert_eq!(first.fingerprint(), second.fingerprint());
    assert_eq!(
        first.registry::<MaterialDef>().names(),
        ["engine:none", "one:ash", "one:bone", "two:clay"]
    );
}

#[test]
fn every_mistake_in_a_definition_is_found_in_one_load() {
    let outcome = load_one(
        r#"[
  { "type": "damage_type", "id": "impact" },
  { "type": "material", "id": "ash",
    "density": "heavy",
    "thougness": 2.0e4,
    "span": 2.5,
    "bearing": "30 MPA",
    "melts_at": "80 kg",
    "flammable": "no",
    "tags": "mineral",
    "resist": { "impcat": { "threshold": true } },
    "rubble": "ruble" }
]"#,
    );
    assert!(outcome.defs.is_none());
    let text = outcome.diagnostics.render();
    for expected in [
        "error[E0201]: expected a number, found a string",
        "error[E0203]: unknown key \"thougness\"",
        "did you mean \"toughness\"?",
        "error[E0208]: this field holds a whole number, and 2.5 is not one",
        "error[E0204]: unknown unit \"MPA\"",
        "did you mean \"MPa\"?",
        "error[E0205]: \"80 kg\" is a mass, but this field holds a temperature",
        "use one of: K C",
        "error[E0201]: expected `true` or `false`, found a string",
        "error[E0201]: expected a list, found a string",
        "error[E0501]: no damage_type \"impcat\"",
        "did you mean \"impact\"?",
        "error[E0201]: expected a number in SI units or a string with a unit, found a boolean",
        "error[E0501]: no block \"ruble\"",
        "error[E0202]: missing field \"toughness\"",
        "= in: material m:ash",
        "= in: material m:ash, at resist.impcat.threshold",
    ] {
        assert!(text.contains(expected), "missing `{expected}` in:\n{text}");
    }
    assert_eq!(outcome.diagnostics.error_count(), 11);
    // Every one names a file, a line, a column and a definition (M3).
    for diagnostic in &outcome.diagnostics {
        let location = diagnostic.location.as_ref().expect("a location");
        assert_eq!(location.file, "mods/m/data/a.json");
        assert!(location.line >= 3 && location.column >= 1);
        assert_eq!(diagnostic.definition.as_ref().unwrap().id, "m:ash");
    }
    let unknown = only(&outcome, "E0203");
    let location = unknown.location.as_ref().unwrap();
    assert_eq!((location.line, location.column), (5, 5));
    assert_eq!(location.end_column - location.column, 11);
}

#[test]
fn every_missing_field_is_reported_together() {
    let outcome = load_one(
        r#"[ { "type": "material", "id": "ash" },
            { "type": "block", "id": "heap", "fill": 0.5 } ]"#,
    );
    assert_eq!(
        summary(&outcome),
        [
            ("E0202".to_owned(), "missing field \"density\"".to_owned()),
            ("E0202".to_owned(), "missing field \"toughness\"".to_owned()),
            ("E0202".to_owned(), "missing field \"material\"".to_owned()),
            ("E0202".to_owned(), "missing field \"height\"".to_owned()),
        ]
    );
    // The caret is on the opening brace of the object that lacks it.
    let first = &outcome.diagnostics.as_slice()[0];
    let location = first.location.as_ref().unwrap();
    assert_eq!(
        (location.line, location.column, location.end_column),
        (1, 3, 4)
    );
}

#[test]
fn a_unit_of_the_wrong_dimension_names_what_the_field_holds() {
    let outcome = load_one(
        r#"[ { "type": "action", "id": "punch", "name": "Punch",
              "cost": "3 s", "reach": "2 kg", "speed": "4 m", "drain": "5 kg",
              "power": "6 J", "every": "7 s", "dose": "2 m", "cooldown": "9 J",
              "phases": [ { "id": "windup", "time": "3 m/s" } ] } ]"#,
    );
    let messages: Vec<String> = outcome
        .diagnostics
        .iter()
        .map(|diagnostic| {
            format!(
                "{} / {}",
                diagnostic.message,
                diagnostic.suggestion.clone().unwrap_or_default()
            )
        })
        .collect();
    assert_eq!(
        messages,
        [
            "\"3 s\" is a time, but this field holds an energy / use one of: J kJ MJ GJ",
            "\"2 kg\" is a mass, but this field holds a length / use one of: mm cm m km",
            "\"4 m\" is a length, but this field holds a speed / use one of: mm/s cm/s m/s km/s (or /min, /h, /d)",
            "\"5 kg\" is a mass, but this field holds a mass rate / use one of: g/s kg/s t/s (or /min, /h, /d)",
            "\"6 J\" is an energy, but this field holds a power / use one of: W kW MW J/s kJ/s MJ/s GJ/s (or /min, /h, /d)",
            "\"7 s\" is a time, but this field holds a rate / use one of: /s (or /min, /h, /d)",
            "\"2 m\" is a length, but this field holds a volume / use one of: ml L",
            "\"9 J\" is an energy, but this field holds a time / use one of: us ms s min h d",
            "\"3 m/s\" is a speed, but this field holds a time / use one of: us ms s min h d",
        ]
    );
    assert!(outcome.diagnostics.iter().all(|d| d.code == "E0205"));
    assert_eq!(outcome.diagnostics.as_slice()[8].path, "phases.windup.time");
}

#[test]
fn every_quantity_type_reads_its_units() {
    let defs = clean(load_one(
        r#"[ { "type": "action", "id": "punch", "name": "Punch",
              "cost": "1.5 kJ", "reach": "80 cm", "speed": "18 km/h", "drain": "0.012 kg/s",
              "power": "2 kW", "every": "0.15 /h", "dose": "250 ml", "cooldown": "2 min",
              "phases": [ { "id": "windup", "time": "250 ms" },
                          { "id": "impact", "time": 0.05 },
                          { "id": "recovery", "time": "1.1 s" } ] },
            { "type": "action", "id": "wait", "name": "Wait", "cost": 12, "power": "3600 kJ/h" } ]"#,
    ));
    let punch = action(&defs, "m:punch");
    assert_eq!(punch.cost.unwrap().get(), Real::of(1500.0));
    assert_eq!(punch.reach.unwrap().get(), Real::of(0.8));
    assert_eq!(punch.speed.unwrap().get(), Real::of(5.0));
    assert_eq!(punch.drain.unwrap().get(), Real::of(0.012));
    assert_eq!(punch.power.unwrap().get(), Real::of(2000.0));
    assert_eq!(punch.every.unwrap().get(), Real::of(0.15 / 3600.0));
    assert_eq!(punch.dose.unwrap().get(), Real::of(0.00025));
    assert_eq!(punch.cooldown, Some(Dur::from_secs(120)));
    let times: Vec<Dur> = punch.phases.iter().map(|phase| phase.time.dur()).collect();
    assert_eq!(
        times,
        [
            Dur::from_millis(250),
            Dur::from_millis(50),
            Dur::from_micros(1_100_000)
        ]
    );
    let wait = action(&defs, "m:wait");
    assert_eq!(wait.cost.unwrap().get(), Real::of(12.0));
    assert_eq!(wait.power.unwrap().get(), Real::of(1000.0));
    assert_eq!(wait.cooldown, None);
}

#[test]
fn quantity_mistakes_say_what_to_write() {
    let outcome = load_one(
        r#"[ { "type": "action", "id": "a", "name": "A", "cost": "lots", "reach": "12",
              "speed": "3 m/week", "power": "3 W/s", "dose": "2 l" },
            { "type": "material", "id": "ash", "density": "600 kg", "toughness": 1 } ]"#,
    );
    let shown: Vec<(String, Option<String>)> = outcome
        .diagnostics
        .iter()
        .map(|d| (d.message.clone(), d.suggestion.clone()))
        .collect();
    assert_eq!(
        shown,
        [
            (
                "\"lots\" is not a quantity: a quantity starts with a number, as in \"250 ms\"".to_owned(),
                Some("this field holds an energy: one of J kJ MJ GJ".to_owned())
            ),
            (
                "\"12\" has no unit".to_owned(),
                Some("write a bare number in SI units without quotation marks, or add one of: mm cm m km".to_owned())
            ),
            (
                "\"3 m/week\" is not a quantity: a rate is per `s`, `min`, `h` or `d`, not per \"week\"".to_owned(),
                Some("this field holds a speed: one of mm/s cm/s m/s km/s (or /min, /h, /d)".to_owned())
            ),
            (
                "\"3 W/s\" is not a quantity: \"W\" is already a rate and cannot be per time again".to_owned(),
                Some("this field holds a power: one of W kW MW J/s kJ/s MJ/s GJ/s (or /min, /h, /d)".to_owned())
            ),
            (
                "unknown unit \"l\"".to_owned(),
                Some("did you mean \"L\"?".to_owned())
            ),
            (
                "expected a plain number, found the quantity \"600 kg\"".to_owned(),
                Some("this field takes no unit; write the number alone, without quotation marks".to_owned())
            ),
        ]
    );
}

#[test]
fn numbers_must_fit_their_fields() {
    let outcome = load_one(
        r#"[ { "type": "material", "id": "a", "density": 1, "toughness": 1, "span": -1 },
            { "type": "material", "id": "b", "density": 1, "toughness": 1, "span": 5000000000 },
            { "type": "material", "id": "c", "density": 1, "toughness": 1, "span": 3.0 },
            { "type": "block", "id": "d", "material": "a", "height": 1, "size": [1, 300] },
            { "type": "block", "id": "e", "material": "a", "height": 1, "size": [1, 2, 3] },
            { "type": "block", "id": "f", "material": "a", "height": 1, "size": [1] } ]"#,
    );
    assert_eq!(
        summary(&outcome),
        [
            (
                "E0208".to_owned(),
                "-1 does not fit: this field holds a whole number from 0 to 4294967295".to_owned()
            ),
            (
                "E0208".to_owned(),
                "5000000000 does not fit: this field holds a whole number from 0 to 4294967295"
                    .to_owned()
            ),
            (
                "E0208".to_owned(),
                "300 does not fit: this field holds a whole number from 0 to 255".to_owned()
            ),
            (
                "E0207".to_owned(),
                "this list has 3 entries, and only 2 belong here".to_owned()
            ),
            (
                "E0207".to_owned(),
                "invalid length 1, expected a tuple of size 2".to_owned()
            ),
        ]
    );
    assert_eq!(outcome.diagnostics.as_slice()[2].path, "size[1]");
}

#[test]
fn an_enum_takes_one_of_a_fixed_set_of_strings() {
    let outcome = load_one(
        r#"[ { "type": "material", "id": "a", "density": 1, "toughness": 1 },
            { "type": "block", "id": "b", "material": "a", "height": 1, "flags": ["opaque", "opaqe"] },
            { "type": "block", "id": "c", "material": "a", "height": 1, "shape": "slope" },
            { "type": "block", "id": "d", "material": "a", "height": 1, "shape": { "full": 3 } },
            { "type": "block", "id": "e", "material": "a", "height": 1, "shape": 7 } ]"#,
    );
    let unknown = only(&outcome, "E0206");
    assert_eq!(
        unknown.message,
        "\"opaqe\" is not one of the values allowed here"
    );
    assert_eq!(
        unknown.suggestion.as_deref(),
        Some("did you mean \"opaque\"?")
    );
    assert_eq!(
        unknown.notes[0].message,
        "allowed: \"opaque\", \"passable\", \"load_bearing\", \"anchor\""
    );
    assert_eq!(unknown.path, "flags[1]");
    let mismatches: Vec<&str> = outcome
        .diagnostics
        .iter()
        .filter(|d| d.code == "E0201")
        .map(|d| d.message.as_str())
        .collect();
    assert_eq!(
        mismatches,
        [
            "\"slope\" takes a value: write it as an object, { \"slope\": ... }",
            "\"full\" takes no value: write it as the string \"full\"",
            "expected a string or an object with one key, found a number",
        ]
    );
}

#[test]
fn references_resolve_in_the_writing_mod_then_its_imports() {
    let core = test_mod(
        "core",
        "",
        &[(
            "data/a.json",
            r#"[ { "type": "material", "id": "stone", "density": 2600, "toughness": 1e6 },
                { "type": "material", "id": "glass", "density": 2500, "toughness": 1e4 } ]"#,
        )],
    );
    let extra = test_mod(
        "extra",
        "",
        &[(
            "data/a.json",
            r#"[ { "type": "material", "id": "glass", "density": 1, "toughness": 1 },
                { "type": "material", "id": "amber", "density": 1, "toughness": 1 } ]"#,
        )],
    );
    let ash = test_mod(
        "ash",
        r#""requires": ["core", "extra"]"#,
        &[(
            "data/a.json",
            r#"[ { "type": "material", "id": "stone", "density": 1, "toughness": 1 },
                { "type": "block", "id": "own", "material": "stone", "height": 1 },
                { "type": "block", "id": "imported", "material": "amber", "height": 1 },
                { "type": "block", "id": "exact", "material": "core:stone", "height": 1 },
                { "type": "block", "id": "shadowed", "material": "glass", "height": 1 },
                { "type": "block", "id": "builtin", "material": "none", "height": 1 } ]"#,
        )],
    );
    let outcome = load(&[core, extra, ash]);
    let defs = outcome.defs.as_ref().expect("warnings do not stop a load");
    let blocks = defs.registry::<BlockDef>();
    let materials = defs.registry::<MaterialDef>();
    let made_of =
        |block: &str| materials.name(blocks.get(blocks.lookup(block).unwrap()).material.id());
    // The mod's own namespace first, then imports in order.
    assert_eq!(made_of("ash:own"), "ash:stone");
    assert_eq!(made_of("ash:imported"), "extra:amber");
    assert_eq!(made_of("ash:exact"), "core:stone");
    assert_eq!(made_of("ash:shadowed"), "core:glass");
    assert_eq!(made_of("ash:builtin"), "engine:none");
    // A later import with the same name is a warning that says to qualify,
    // whether the first match was the mod's own or an earlier import.
    let warnings: Vec<(&str, Option<&str>)> = outcome
        .diagnostics
        .iter()
        .map(|d| (d.message.as_str(), d.suggestion.as_deref()))
        .collect();
    assert_eq!(
        warnings,
        [
            (
                "\"stone\" means \"ash:stone\" here, and \"core:stone\" also exists",
                Some("write \"ash:stone\" to be clear")
            ),
            (
                "\"glass\" means \"core:glass\" here, and \"extra:glass\" also exists",
                Some("write \"core:glass\" to be clear")
            ),
        ]
    );
    assert!(outcome.diagnostics.iter().all(|d| d.code == "W0504"));
}

#[test]
fn imports_replace_the_default_of_requires() {
    let core = test_mod(
        "core",
        "",
        &[(
            "data/a.json",
            r#"[ { "type": "material", "id": "stone", "density": 1, "toughness": 1 } ]"#,
        )],
    );
    let ash = test_mod(
        "ash",
        r#""requires": ["core"], "imports": []"#,
        &[(
            "data/a.json",
            r#"[ { "type": "block", "id": "b", "material": "stone", "height": 1 } ]"#,
        )],
    );
    let outcome = load(&[core, ash]);
    let dangling = only(&outcome, "E0501");
    assert_eq!(dangling.message, "no material \"stone\"");
    assert_eq!(
        dangling.suggestion.as_deref(),
        Some("did you mean \"core:stone\"?")
    );
}

#[test]
fn a_field_may_take_more_than_one_shape() {
    let defs = clean(load_one(
        r#"[ { "type": "damage_type", "id": "heat" },
            { "type": "material", "id": "stone", "density": 1, "toughness": 1, "rubble": "gravel" },
            { "type": "material", "id": "ice", "density": 1, "toughness": 1,
              "rubble": { "heat": "puddle" } },
            { "type": "block", "id": "gravel", "material": "stone", "height": 1 },
            { "type": "block", "id": "puddle", "material": "ice", "height": 1 } ]"#,
    ));
    let blocks = defs.registry::<BlockDef>();
    let gravel = clash_data::Ref::new(blocks.lookup("m:gravel").unwrap());
    let puddle = clash_data::Ref::new(blocks.lookup("m:puddle").unwrap());
    let heat = clash_data::Ref::new(defs.lookup::<DamageTypeDef>("m:heat").unwrap());
    assert_eq!(material(&defs, "m:stone").rubble, Some(Rubble::One(gravel)));
    assert_eq!(
        material(&defs, "m:ice").rubble,
        Some(Rubble::ByDamage([(heat, puddle)].into_iter().collect()))
    );

    // Each shape reports its own mistakes where they are, and a shape
    // the field does not take is a value of the wrong kind.
    let outcome = load_one(
        "[ { \"type\": \"damage_type\", \"id\": \"heat\" },\n  { \"type\": \"material\", \"id\": \"a\", \"density\": 1, \"toughness\": 1, \"rubble\": 7 },\n  { \"type\": \"material\", \"id\": \"b\", \"density\": 1, \"toughness\": 1, \"rubble\": \"gravle\" },\n  { \"type\": \"material\", \"id\": \"c\", \"density\": 1, \"toughness\": 1,\n    \"rubble\": { \"heat\": \"pudle\", \"cold\": \"gravel\" } },\n  { \"type\": \"block\", \"id\": \"gravel\", \"material\": \"a\", \"height\": 1 } ]",
    );
    let shown: Vec<String> = outcome
        .diagnostics
        .iter()
        .map(|d| {
            format!(
                "{} {} {} [{}]",
                d.code,
                d.location.as_ref().unwrap().line,
                d.message,
                d.path
            )
        })
        .collect();
    assert_eq!(
        shown,
        [
            "E0201 2 expected a block id, or a block id for each damage type, found a number [rubble]",
            "E0501 3 no block \"gravle\" [rubble]",
            "E0501 5 no block \"pudle\" [rubble.heat]",
            "E0501 5 no damage_type \"cold\" [rubble.cold]",
        ]
    );
    assert_eq!(
        outcome.diagnostics.as_slice()[1].suggestion.as_deref(),
        Some("did you mean \"gravel\"?")
    );
}

#[test]
fn a_dangling_reference_points_at_the_string() {
    let outcome = load_one(
        "[ { \"type\": \"material\", \"id\": \"granite\", \"density\": 1, \"toughness\": 1 },\n  { \"type\": \"block\", \"id\": \"wall\", \"height\": 1,\n    \"material\": \"granit\" },\n  { \"type\": \"block\", \"id\": \"odd\", \"height\": 1, \"material\": \"Not An Id\" },\n  { \"type\": \"block\", \"id\": \"num\", \"height\": 1, \"material\": 4 } ]",
    );
    let dangling = only(&outcome, "E0501");
    assert_eq!(dangling.message, "no material \"granit\"");
    assert_eq!(
        dangling.suggestion.as_deref(),
        Some("did you mean \"granite\"?")
    );
    let location = dangling.location.as_ref().unwrap();
    assert_eq!((location.line, location.column), (3, 17));
    assert_eq!(location.end_column - location.column, 8);
    assert_eq!(dangling.path, "material");
    assert_eq!(
        only(&outcome, "E0503").message,
        "\"Not An Id\" is not an id"
    );
    assert_eq!(
        only(&outcome, "E0201").message,
        "expected an id, found a number"
    );
}

#[test]
fn inline_definitions_get_stable_ids_and_cannot_be_referenced() {
    let data = r#"[
      { "type": "rule", "id": "shared", "when": "true" },
      { "type": "action", "id": "punch", "name": "Punch",
        "phases": [
          { "id": "windup", "time": "250 ms" },
          { "id": "impact", "time": "50 ms", "on_end": { "when": "hit", "then": ["damage"] } }
        ],
        "rules": [
          "shared",
          { "when": "always" },
          { "id": "named", "when": "sometimes" }
        ] },
      { "type": "action", "id": "kick", "name": "Kick", "rules": ["named"] }
    ]"#;
    let defs = clean(load_one(data));
    let rules = defs.registry::<RuleDef>();
    assert_eq!(
        rules.names(),
        [
            "m:named",
            "m:punch#phases.impact.on_end",
            "m:punch#rules[1]",
            "m:shared"
        ]
    );
    let punch = action(&defs, "m:punch");
    let on_end = punch.phases[1].on_end.unwrap().id();
    assert_eq!(rules.name(on_end), "m:punch#phases.impact.on_end");
    assert_eq!(rules.get(on_end).when.value, "hit");
    assert_eq!(rules.get(on_end).then[0].value, "damage");
    let listed: Vec<&str> = punch
        .rules
        .iter()
        .map(|rule| rules.name(rule.id()))
        .collect();
    assert_eq!(listed, ["m:shared", "m:punch#rules[1]", "m:named"]);
    // One given an id of its own can be referred to.
    let kick = action(&defs, "m:kick");
    assert_eq!(rules.name(kick.rules[0].id()), "m:named");
    // The inline ones are finished trees of their own, marked as such.
    assert!(defs.tree("rule", "m:punch#rules[1]").unwrap().inline);
    assert!(!defs.tree("rule", "m:shared").unwrap().inline);

    // The ids are the same on every load.
    let again = clean(load_one(data));
    assert_eq!(again.registry::<RuleDef>().names(), rules.names());

    // A generated id cannot be written as a reference.
    let outcome = load_one(
        r#"[ { "type": "action", "id": "punch", "name": "P", "rules": [ { "when": "x" } ] },
            { "type": "action", "id": "kick", "name": "K", "rules": ["m:punch#rules[0]"] } ]"#,
    );
    let refused = only(&outcome, "E0503");
    assert_eq!(refused.message, "\"m:punch#rules[0]\" is not an id");
    assert_eq!(
        refused.suggestion.as_deref(),
        Some("an inline definition cannot be referred to; give it an `id` of its own")
    );
}

#[test]
fn mistakes_inside_an_inline_definition_are_located_in_it() {
    let outcome = load_one(
        r#"[ { "type": "action", "id": "punch", "name": "P",
              "rules": [ { "when": "a", "thne": [] }, { "type": "action", "when": "b" }, 5,
                         { "id": "core:taken", "when": "c" } ] } ]"#,
    );
    let unknown = only(&outcome, "E0203");
    assert_eq!(unknown.definition.as_ref().unwrap().id, "m:punch#rules[0]");
    assert_eq!(unknown.definition.as_ref().unwrap().type_name, "rule");
    assert_eq!(
        unknown.suggestion.as_deref(),
        Some("did you mean \"then\"?")
    );
    let messages: Vec<&str> = outcome
        .diagnostics
        .iter()
        .filter(|d| d.code != "E0203")
        .map(|d| d.message.as_str())
        .collect();
    assert_eq!(
        messages,
        [
            "a definition written in place here is a rule",
            "expected an id or a definition written in place, found a number",
            "the id of a definition written in place is a bare name in the mod's own namespace",
        ]
    );
}

#[test]
fn a_type_finishes_and_validates_its_own_definitions() {
    let outcome = load_one(
        "[ { \"type\": \"material\", \"id\": \"void\", \"density\": 0, \"toughness\": 1, \"span\": 99 },\n  { \"type\": \"material\", \"id\": \"ice\", \"density\": 900, \"toughness\": 1, \"name\": \"Ice\" },\n  { \"type\": \"material\", \"id\": \"frost\", \"density\": 900, \"toughness\": 1, \"name\": \"Ice\" },\n  { \"type\": \"rule\", \"id\": \"melt\", \"when\": \"heat \\\"x\\\" @ 3\", \"target\": \"ice@wet\" },\n  { \"type\": \"rule\", \"id\": \"lost\", \"when\": \"true\", \"target\": \"steam@dry\" } ]",
    );
    // Semantic: the type's own `validate`.
    let invalid = only(&outcome, "E0701");
    assert_eq!(invalid.message, "`density` must be above zero");
    assert_eq!(invalid.definition.as_ref().unwrap().id, "m:void");
    assert_eq!(invalid.location.as_ref().unwrap().column, 31);
    // Cross: a whole-set check.
    let warnings: Vec<&str> = outcome
        .diagnostics
        .iter()
        .filter(|d| d.code == "W0702")
        .map(|d| d.message.as_str())
        .collect();
    assert_eq!(
        warnings,
        [
            "a `span` above 64 cells is unusual",
            "m:ice has the same name as m:frost"
        ]
    );
    // Finish: an error inside a string is placed inside the string, with
    // its escapes accounted for.
    let inside = only(&outcome, "E0207");
    assert_eq!(inside.message, "unexpected `@` in an expression");
    let location = inside.location.as_ref().unwrap();
    assert_eq!(location.line, 4);
    assert_eq!(
        &location.source_line[location.column as usize - 1..][..1],
        "@"
    );
    assert_eq!(location.end_column - location.column, 1);
    // Finish: a reference in the type's own syntax, resolved and reported
    // like any other.
    let dangling = only(&outcome, "E0501");
    assert_eq!(dangling.message, "no material \"steam\"");
    let location = dangling.location.as_ref().unwrap();
    assert_eq!(
        &location.source_line[location.column as usize - 1..][..5],
        "steam"
    );
    assert_eq!(location.end_column - location.column, 5);
    assert!(outcome.defs.is_none());
}

#[test]
fn a_finished_reference_in_a_types_own_syntax_resolves() {
    let defs = clean(load_one(
        r#"[ { "type": "material", "id": "ice", "density": 900, "toughness": 1 },
            { "type": "rule", "id": "melt", "when": "hot", "target": "ice@wet" } ]"#,
    ));
    let rules = defs.registry::<RuleDef>();
    let melt = rules.get(rules.lookup("m:melt").unwrap());
    assert_eq!(
        melt.target_material,
        defs.registry::<MaterialDef>().lookup("m:ice")
    );
}

#[test]
fn the_fingerprint_is_stable_and_follows_the_finished_trees() {
    let base = r#"[ { "type": "material", "id": "ash", "density": 600, "toughness": 2e4, "tags": ["soft"] },
                   { "type": "block", "id": "heap", "material": "ash", "height": 1 } ]"#;
    let first = clean(load_one(base));
    let second = clean(load_one(base));
    assert_eq!(first.fingerprint(), second.fingerprint());

    // Where a value sits in the file does not matter.
    let moved = clean(load_one(
        &base.replace("[ {", "[\n\n   {").replace(", \"", ",\n  \""),
    ));
    assert_eq!(moved.fingerprint(), first.fingerprint());

    // Any change to any finished tree does.
    for changed in [
        base.replace("600", "601"),
        base.replace("\"soft\"", "\"hard\""),
        base.replace("\"height\": 1", "\"height\": 1.5"),
        base.replace("\"id\": \"heap\"", "\"id\": \"pile\""),
        base.replace(", \"tags\": [\"soft\"]", ""),
        base.replace(
            "\"density\": 600, \"toughness\": 2e4",
            "\"toughness\": 2e4, \"density\": 600",
        ),
    ] {
        let other = clean(load_one(&changed));
        assert_ne!(other.fingerprint(), first.fingerprint(), "{changed}");
    }

    // A patch from another mod changes the finished tree, and so the
    // fingerprint.
    let patched = clean(load(&[
        test_mod("m", "", &[("data/a.json", base)]),
        test_mod(
            "n",
            r#""after": ["m"]"#,
            &[(
                "data/a.json",
                r#"[ { "type": "material", "id": "m:ash", "patch": true, "density": 700 } ]"#,
            )],
        ),
    ]));
    assert_ne!(patched.fingerprint(), first.fingerprint());
    // No mods at all still have a fingerprint, and it is another one.
    let types = types();
    let empty = Loader::new(&types).load_mods(&[]).defs.unwrap();
    assert_ne!(empty.fingerprint(), first.fingerprint());
    assert_eq!(first.fingerprint().to_string().len(), 32);
}

#[test]
fn an_experimental_feature_is_refused_without_opting_in() {
    let data = r#"[
      { "type": "rule", "id": "undo", "when": "true", "then": ["message", "rewind"] },
      { "type": "material", "id": "neon", "density": 1, "toughness": 1, "glow": "red" },
      { "type": "gizmo", "id": "widget" }
    ]"#;
    let refused = load(&[test_mod("m", "", &[("data/a.json", data)])]);
    assert!(refused.defs.is_none());
    let messages: Vec<&str> = refused
        .diagnostics
        .iter()
        .map(|d| d.message.as_str())
        .collect();
    assert_eq!(
        messages,
        [
            "this uses the experimental feature \"world_rewind\", which mod \"m\" has not opted in to",
            "this uses the experimental feature \"coloured_light\", which mod \"m\" has not opted in to",
            "this uses the experimental feature \"gizmos\", which mod \"m\" has not opted in to",
        ]
    );
    assert!(refused.diagnostics.iter().all(|d| d.code == "E0802"));
    let verb = &refused.diagnostics.as_slice()[0];
    assert_eq!(
        verb.suggestion.as_deref(),
        Some("add \"world_rewind\" to `experimental` in the mod.json of \"m\"")
    );
    let location = verb.location.as_ref().unwrap();
    assert_eq!(
        &location.source_line[location.column as usize - 1..][..8],
        "\"rewind\""
    );
    assert_eq!(refused.diagnostics.as_slice()[1].path, "glow");

    // Opted in: accepted, with a warning by name at every load.
    let accepted = load(&[test_mod(
        "m",
        r#""experimental": ["world_rewind", "coloured_light", "gizmos"]"#,
        &[("data/a.json", data)],
    )]);
    let defs = accepted.defs.as_ref().expect("opted in");
    let warnings: Vec<&str> = accepted
        .diagnostics
        .iter()
        .map(|d| d.message.as_str())
        .collect();
    assert_eq!(
        warnings,
        [
            "mod \"m\" relies on the experimental feature \"world_rewind\", which may change or be removed",
            "mod \"m\" relies on the experimental feature \"coloured_light\", which may change or be removed",
            "mod \"m\" relies on the experimental feature \"gizmos\", which may change or be removed",
        ]
    );
    assert!(accepted.diagnostics.iter().all(|d| d.code == "W0803"));
    // The list a save records.
    assert_eq!(
        defs.experimental_features(),
        ["coloured_light", "gizmos", "world_rewind"]
    );
    let neon = material(defs, "m:neon");
    assert_eq!(neon.glow.as_ref().unwrap().get(), "red");

    // Opting in is per mod: the mod that wrote the value must name it.
    let other = load(&[
        test_mod("m", r#""experimental": ["gizmos"]"#, &[]),
        test_mod(
            "n",
            "",
            &[("data/a.json", r#"[ { "type": "gizmo", "id": "w" } ]"#)],
        ),
    ]);
    assert_eq!(
        only(&other, "E0802").message,
        "this uses the experimental feature \"gizmos\", which mod \"n\" has not opted in to"
    );
}

#[test]
fn safety_limits_are_diagnostics() {
    let types = types();
    let many: Vec<String> = (0..300)
        .map(|n| format!("{{ \"type\": \"damage_type\", \"id\": \"d{n}\" }}"))
        .collect();
    let many = format!("[{}]", many.join(","));
    let mods = [test_mod("m", "", &[("data/a.json", &many)])];

    // A definition count over its limit.
    let limits = Limits {
        max_definitions: 100,
        ..Limits::default()
    };
    let outcome = loader(&types, &mods).limits(limits).load_all();
    assert!(outcome.defs.is_none());
    assert_eq!(
        only(&outcome, "E0105").message,
        "the mods hold more than 100 definitions, which is the limit"
    );

    // A file over the size limit.
    let limits = Limits {
        max_file_bytes: 1000,
        ..Limits::default()
    };
    let outcome = loader(&types, &mods).limits(limits).load_all();
    assert!(outcome.defs.is_none());
    assert!(
        only(&outcome, "E0103")
            .message
            .ends_with("bytes; the limit is 1000")
    );

    // Nesting over the depth limit.
    let deep = format!(
        "[{{ \"type\": \"damage_type\", \"id\": \"d\", \"x\": {}1{} }}]",
        "[".repeat(80),
        "]".repeat(80)
    );
    let outcome = load_one(&deep);
    assert_eq!(
        only(&outcome, "E0104").message,
        "lists and objects are nested more than 64 deep"
    );

    // More definitions of a type than its dense id can count.
    let gizmos: Vec<String> = (0..257)
        .map(|n| format!("{{ \"type\": \"gizmo\", \"id\": \"g{n}\" }}"))
        .collect();
    let outcome = load(&[test_mod(
        "m",
        r#""experimental": ["gizmos"]"#,
        &[("data/a.json", &format!("[{}]", gizmos.join(",")))],
    )]);
    assert!(outcome.defs.is_none());
    assert_eq!(
        only(&outcome, "E0105").message,
        "there are 257 definitions of type `gizmo`, more than its id can count"
    );
}

#[test]
fn defs_are_shared_across_threads() {
    fn assert_send_sync<T: Send + Sync>() {}
    assert_send_sync::<Defs>();
    assert_send_sync::<std::sync::Arc<Defs>>();

    let defs = clean(load_one(
        r#"[ { "type": "material", "id": "ash", "density": 600, "toughness": 2e4 } ]"#,
    ));
    let shared = std::sync::Arc::clone(&defs);
    let density = std::thread::spawn(move || material(&shared, "m:ash").density)
        .join()
        .unwrap();
    assert_eq!(density, Real::of(600.0));
    assert!(defs.try_registry::<MaterialDef>().is_some());
    assert!(format!("{defs:?}").contains("fingerprint"));
}
