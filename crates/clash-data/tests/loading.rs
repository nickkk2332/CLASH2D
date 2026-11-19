//! Loader options that decide what is read at all (04 §9): passing over
//! definitions of types that were not registered.

mod common;

use clash_data::code;
use common::*;

/// One mod with a definition of a registered type and one of a type the
/// test registers nothing for.
fn mixed() -> [TestMod; 1] {
    [test_mod(
        "m",
        "",
        &[(
            "data/a.json",
            r#"[
              { "type": "damage_type", "id": "heat" },
              { "type": "hologram", "id": "sign", "glow": 3 }
            ]"#,
        )],
    )]
}

#[test]
fn an_unknown_type_is_a_mistake_unless_the_loader_is_told_otherwise() {
    let types = types();
    let outcome = loader(&types, &mixed()).load_all();
    assert_eq!(outcome.diagnostics.with_code(code::UNKNOWN_TYPE).len(), 1);
    assert!(outcome.defs.is_none());
}

#[test]
fn only_known_types_passes_over_the_rest_without_a_word() {
    let types = types();
    let outcome = loader(&types, &mixed()).only_known_types(true).load_all();
    assert!(
        outcome.diagnostics.is_empty(),
        "{}",
        outcome.diagnostics.render()
    );
    let defs = outcome.defs.expect("the mod loads");
    assert!(defs.lookup::<DamageTypeDef>("m:heat").is_some());
    assert_eq!(defs.registry::<DamageTypeDef>().len(), 1);
}

#[test]
fn a_reference_to_what_was_passed_over_is_still_an_unknown_reference() {
    let types = types();
    let mods = [test_mod(
        "m",
        "",
        &[(
            "data/a.json",
            r#"[
              { "type": "hologram", "id": "stone" },
              { "type": "block", "id": "wall", "material": "stone" }
            ]"#,
        )],
    )];
    let outcome = loader(&types, &mods).only_known_types(true).load_all();
    assert!(outcome.diagnostics.has_errors());
    assert_eq!(outcome.diagnostics.with_code(code::UNKNOWN_TYPE).len(), 0);
}
