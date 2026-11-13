//! The creation session as a screen drives it (12 §10.4, decision
//! D-653): `app.creation` is null until a session begins, then the
//! session's map with its template, name, seed, completeness and
//! whether it is finished; the service `creation` begins, selects,
//! names, seeds and finishes, and refuses a selection a choice does not
//! take with a reason shown on the error screen; `new_game` makes the
//! finished character, and the session is over once the game begins.
//!
//! The screen is the fixture mod `maker` under
//! `tests/fixtures/creation`, a button for each op, loaded beside the
//! bundled mods.

use std::path::PathBuf;

use clash_app::boot;
use clash_app::game::{BootOptions, Game};
use clash_app::headless::Headless;
use clash_core::Real;
use clash_expr::Value;
use clash_sim::component::Identity;

fn manifest() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn press(run: &mut Headless, keys: &[&str]) {
    let going = run.keys(keys).unwrap_or_else(|problem| panic!("{problem}"));
    assert!(going, "the game ended while pressing {keys:?}");
}

fn top(run: &Headless) -> Option<String> {
    run.game()
        .scene()
        .top()
        .map(|screen| screen.id().to_owned())
}

/// The first line of the frame: what the screen reads of
/// `app.creation`.
fn reads(run: &Headless) -> String {
    run.text()
        .lines()
        .next()
        .unwrap_or_default()
        .trim()
        .to_owned()
}

#[test]
fn a_screen_makes_a_character_through_the_creation_session() {
    let fixture = manifest().join("tests/fixtures/creation");
    let search = [fixture.join("mods"), manifest().join("../../mods")];
    let profile = fixture.join("profiles/maker.json");
    let loaded = boot::load(&search, &profile.to_string_lossy())
        .unwrap_or_else(|problem| panic!("{problem}"));
    let game = Game::boot(
        loaded,
        BootOptions {
            seed: 3,
            ..BootOptions::default()
        },
    )
    .unwrap_or_else(|problem| panic!("{problem}"));
    let mut run = Headless::new(game);
    assert_eq!(top(&run).as_deref(), Some("maker:maker"));
    // Nothing is being made: `app.creation` is null.
    assert_eq!(run.game().scene().app().creation(), Value::Null);
    assert!(
        reads(&run).starts_with("Template none; name none; seed -1;"),
        "{}",
        reads(&run)
    );

    // Begin: the person, every choice at its default, the seed given on
    // the command line.
    press(&mut run, &["enter"]);
    assert_eq!(
        reads(&run),
        "Template core:person; name ; seed 3; complete true; finished false."
    );
    let creation = run.game().scene().app().creation();
    let choices = creation
        .as_map()
        .and_then(|map| map.get("choices"))
        .and_then(Value::as_list)
        .expect("the open choices are listed");
    let keys: Vec<&str> = choices
        .iter()
        .filter_map(|choice| choice.as_map()?.get("key")?.as_str())
        .collect();
    assert_eq!(
        keys,
        [
            "core:person.name",
            "core:person.stature",
            "core:person.build",
            "core:person.training"
        ]
    );

    // A build that is there, then one that is not: refused, with the
    // reason on the error screen, and nothing changed.
    press(&mut run, &["down", "enter"]);
    press(&mut run, &["down", "enter"]);
    assert_eq!(top(&run).as_deref(), Some("ui:error"));
    let said = run.text();
    assert!(said.contains("giant"), "{said}");
    press(&mut run, &["escape"]);
    assert_eq!(top(&run).as_deref(), Some("maker:maker"));
    let build = |run: &Headless| {
        let creation = run.game().scene().app().creation();
        creation
            .as_map()
            .and_then(|map| map.get("choices"))
            .and_then(Value::as_list)
            .and_then(|choices| {
                choices
                    .iter()
                    .find(|choice| {
                        choice
                            .as_map()
                            .and_then(|map| map.get("key"))
                            .and_then(Value::as_str)
                            == Some("core:person.build")
                    })
                    .and_then(|choice| choice.as_map()?.get("value").cloned())
            })
    };
    assert_eq!(build(&run), Some(Value::list([Value::text("heavy")])));

    // A name, a seed, and done.
    press(
        &mut run,
        &["down", "enter", "down", "enter", "down", "enter"],
    );
    assert_eq!(
        reads(&run),
        "Template core:person; name Bea; seed 7; complete true; finished true."
    );

    // The game makes the character finished, and the session is over.
    press(&mut run, &["down", "enter"]);
    assert!(run.game().playing());
    assert_eq!(top(&run).as_deref(), Some("ui:play"));
    assert_eq!(run.game().scene().app().creation(), Value::Null);
    run.game()
        .with_session(|session| {
            let sim = &session.sim;
            let identity = sim.components().get::<Identity>(session.player).unwrap();
            assert_eq!(identity.name(), Some("Bea"));
            let strength = sim
                .stat_defs()
                .unwrap()
                .attribute_id("core:strength")
                .unwrap();
            let stats = sim.components().stats.get(session.player).unwrap();
            assert_eq!(stats.attr(strength), Some(Real::of(12.0)));
        })
        .unwrap();
}

#[test]
fn a_scenario_that_names_no_template_there_is_is_a_located_error() {
    let dir = std::env::temp_dir().join(format!("clash-app-no-template-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let data = dir.join("mods/elsewhere/data");
    std::fs::create_dir_all(&data).unwrap();
    std::fs::write(
        dir.join("mods/elsewhere/mod.json"),
        r#"{ "id": "elsewhere", "name": "Elsewhere", "version": "0.1.0", "engine": ">=0.1", "requires": [] }"#,
    )
    .unwrap();
    std::fs::write(
        data.join("scenarios.json"),
        r#"[ { "type": "scenario", "id": "nowhere", "worlds": [ { "realm": "core:countryside" } ],
               "start": { "characters": [ { "template": "nobody" } ], "at": [ { "cell": [1, 1] } ] } } ]"#,
    )
    .unwrap();
    let profile = dir.join("elsewhere.json");
    std::fs::write(
        &profile,
        r#"{ "id": "elsewhere", "mods": ["core", "ui", "elsewhere"], "boot_screen": "ui:title" }"#,
    )
    .unwrap();
    let search = [dir.join("mods"), manifest().join("../../mods")];
    let problem = boot::load(&search, &profile.to_string_lossy()).unwrap_err();
    assert!(
        problem.contains("there is no template `elsewhere:nobody`"),
        "{problem}"
    );
    assert!(problem.contains("scenarios.json"), "{problem}");
    let _ = std::fs::remove_dir_all(&dir);
}
