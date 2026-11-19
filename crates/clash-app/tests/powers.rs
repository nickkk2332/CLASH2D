//! Powers from the abilities screen (08 §5; 12 §10.7), headless from
//! the title in the `armoury` fixture's game of the gifted, whose
//! character has every power of the sampler: the screen lists each use
//! with its rank, cost and state from `ability::options`; Enter uses
//! one, asking for its parameters but those its grant supplies, as the
//! command `UseAbility`; a used one shows its cooldown; a number key
//! puts a use on a quick slot and the same key in play uses it; Enter
//! on a mode switches it with `SetMode`. Golden frames are kept of the
//! screen before and after a use.
//!
//! The golden frames are written with `BLESS=1 cargo test -p clash-app`.

mod common;

use clash_app::headless::Headless;
use clash_scene::assert_golden;
use clash_sim::Command;
use common::*;

/// The game of the gifted is the first of the fixture's list.
const GIFTED: usize = 0;

fn begin_gifted(name: &str) -> (Headless, std::path::PathBuf) {
    let data = data_dir(name);
    let mut run = boot_armoury(&data);
    begin(&mut run, GIFTED, "Ada");
    settle(&mut run);
    (run, data)
}

/// The qualified id of the ability the last `UseAbility` used, and the
/// names of the parameters it gave.
fn last_use(run: &Headless) -> (String, Vec<String>) {
    run.game()
        .with_session(|session| {
            let abilities = session
                .sim
                .defs()
                .try_registry::<clash_sim::ability::AbilityDef>()
                .unwrap();
            session
                .sim
                .input_log()
                .entries()
                .iter()
                .rev()
                .find_map(|entry| match &entry.command {
                    Command::UseAbility {
                        ability, params, ..
                    } => Some((
                        abilities.name(*ability).to_owned(),
                        params.iter().map(|(name, _)| name.clone()).collect(),
                    )),
                    _ => None,
                })
        })
        .flatten()
        .expect("an ability was used")
}

#[test]
fn a_power_is_used_from_the_abilities_screen_and_cools_down() {
    let (mut run, data) = begin_gifted("powers-use");
    script(&mut run, "a\n");
    assert_eq!(top(&run), "ui:abilities");
    let listed = run.text();
    for power in ["Dash", "Force push", "Fire bolt", "Heal", "Shield raised"] {
        assert!(listed.contains(power), "{power} is listed: {listed}");
    }
    assert!(listed.contains("25 stamina"), "{listed}");
    assert_golden(golden("abilities"), run.frame());

    // Dash takes nothing the player gives: its grant supplies the time.
    script(&mut run, "enter\n");
    assert_eq!(top(&run), "ui:play");
    let (ability, params) = last_use(&run);
    assert_eq!(ability, "core:dash");
    assert!(params.is_empty(), "{params:?}");
    script(&mut run, "wait 10\n");
    settle(&mut run);

    // Back on the screen it is cooling down.
    script(&mut run, "a\n");
    assert!(run.text().contains("cooling"), "{}", run.text());
    assert_golden(golden("abilities_cooling"), run.frame());

    // Force push asks where to push, and nothing else.
    script(&mut run, "down\nenter\n");
    assert_eq!(top(&run), "ui:pick_cell", "the place is asked for");
    script(&mut run, "l\nl\nenter\n");
    let (ability, params) = last_use(&run);
    assert_eq!(ability, "core:force_push");
    assert_eq!(params, ["to"]);
    let _ = std::fs::remove_dir_all(&data);
}

#[test]
fn a_quick_slot_and_a_mode() {
    let (mut run, data) = begin_gifted("powers-slot");
    // Heal, the fourth use, on the key 3; the neighbour as the target.
    script(&mut run, "a\ndown\ndown\ndown\n3\n");
    assert!(run.text().contains("3"), "{}", run.text());
    let slot = run
        .game()
        .scene()
        .pref("ui:slot_3")
        .cloned()
        .unwrap_or(clash_core::Value::Null);
    assert_eq!(slot, clash_core::Value::Str("core:heal".to_owned()));
    script(&mut run, "escape\ntab\n3\n");
    let (ability, params) = last_use(&run);
    assert_eq!(ability, "core:heal");
    assert_eq!(
        params,
        ["target"],
        "the target is given, the share is the grant's"
    );

    // The shield is a mode: Enter raises it.
    script(&mut run, "wait 30\na\ntab\nenter\n");
    let raised = run
        .game()
        .with_session(|session| {
            session
                .sim
                .input_log()
                .entries()
                .iter()
                .any(|entry| matches!(&entry.command, Command::SetMode { on: true, .. }))
        })
        .unwrap_or(false);
    assert!(raised, "the mode was switched on: {:?}", log(&run));
    let _ = std::fs::remove_dir_all(&data);
}
