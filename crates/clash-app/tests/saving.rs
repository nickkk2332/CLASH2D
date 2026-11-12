//! Saving and loading (13 §9; 01 §11.2), headless from the title
//! screen: a game saved in the town from the pause menu, left, and
//! carried on from the title draws the same frame and has the same
//! state hash as when it was saved; the list of saves says which were
//! made with other mods; a save made with a mod that is no longer
//! loaded loads with what differs shown; and a start's saving policy
//! decides whether a game can be saved by hand, and saves a game of one
//! save as it is left.

use std::path::{Path, PathBuf};

use clash_app::cli::Options;
use clash_app::headless::Headless;
use clash_core::{Hash128, IVec3};
use clash_scene::{AppList, assert_golden, frame_to_text};
use clash_sim::{Command, DebugCommand};
use clash_world::{Layer, Location};

fn repo() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn data_dir(name: &str) -> PathBuf {
    let data = std::env::temp_dir().join(format!("clash-app-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&data);
    std::fs::create_dir_all(&data).unwrap();
    data
}

fn boot(data: &Path, profile: &str) -> Headless {
    let options = Options {
        profile: profile.to_owned(),
        mods: Some(repo().join("mods")),
        data: Some(data.to_path_buf()),
        ..Options::default()
    };
    Headless::new(clash_app::boot(&options).unwrap_or_else(|problem| panic!("{problem}")))
}

fn script(run: &mut Headless, text: &str) {
    run.run_script(text, &mut std::io::sink())
        .unwrap_or_else(|problem| panic!("{problem}"));
    assert!(run.running(), "the game ended in the script");
}

fn settle(run: &mut Headless) {
    for _ in 0..600 {
        if run.game().scene().playback().is_idle() {
            return;
        }
        run.pass();
    }
    panic!("playback never came to rest");
}

fn top(run: &Headless) -> String {
    run.game()
        .scene()
        .top()
        .map(|screen| screen.id().to_owned())
        .unwrap_or_default()
}

fn hash(run: &Headless) -> Hash128 {
    run.game()
        .with_session(|session| session.sim.state_hash())
        .expect("a game is being played")
}

fn log(run: &Headless) -> Vec<String> {
    run.game()
        .scene()
        .log()
        .lines()
        .iter()
        .map(|line| line.text.plain())
        .collect()
}

/// The ids and states of `app.saves`, newest first.
fn saves(run: &Headless) -> Vec<(String, String)> {
    run.game()
        .scene()
        .app()
        .entries(AppList::Saves)
        .into_iter()
        .map(|entry| (entry.id, entry.state))
        .collect()
}

/// From the pause menu: Save, under a name.
fn save_as(run: &mut Headless, name: &str) {
    script(run, "escape\n");
    assert_eq!(top(run), "ui:pause");
    script(run, &format!("down\nenter\ntype {name}\ntab\nenter\n"));
    assert_eq!(top(run), "ui:play", "the save closed the menus");
}

/// From the pause menu, back to the title.
fn quit_to_title(run: &mut Headless) {
    script(run, "escape\n");
    assert_eq!(top(run), "ui:pause");
    // Resume, Save, Load, Options, Quit to title; and yes.
    script(run, "down\ndown\ndown\ndown\nenter\nenter\n");
    assert_eq!(top(run), "ui:title");
    assert!(!run.game().playing());
}

#[test]
fn saved_left_and_carried_on_it_is_the_same_game() {
    let data = data_dir("save-town");
    let mut run = boot(&data, "default");
    // Ada, in the town; into the house across the street.
    script(
        &mut run,
        "enter\ntype Ada\ntab\ntab\nenter\nk\nk\nk\nk\nwait 60\n",
    );
    settle(&mut run);
    assert!(run.game().scene().app().can_save());

    save_as(&mut run, "In the house");
    settle(&mut run);
    assert_eq!(
        saves(&run),
        [("in-the-house".to_owned(), "enabled".to_owned())]
    );
    assert!(data.join("saves/in-the-house.c2save").is_file());
    assert_eq!(
        log(&run).last().map(String::as_str),
        Some("Saved as “In the house” (in-the-house).")
    );
    let frame = frame_to_text(run.frame());
    let before = hash(&run);

    // To the title, which now offers to carry on; and on.
    quit_to_title(&mut run);
    let title = run.text();
    assert!(
        title.contains("Continue") && title.contains("Load"),
        "{title}"
    );
    script(&mut run, "down\nenter\n");
    assert_eq!(top(&run), "ui:play");
    assert!(run.game().playing());
    assert!(
        run.game().scene().playback().is_idle(),
        "nothing is played back after a load"
    );
    assert_eq!(hash(&run), before, "the state hash is as it was saved");
    settle(&mut run);
    assert_eq!(frame_to_text(run.frame()), frame, "the frame is as it was");
    assert_golden(
        repo().join("crates/clash-app/tests/golden/carried_on.frame"),
        run.frame(),
    );

    // And it goes on: a step north.
    let at = run
        .game()
        .with_session(|session| session.player_loc().unwrap())
        .unwrap();
    script(&mut run, "k\nwait 60\n");
    let now = run
        .game()
        .with_session(|session| session.player_loc().unwrap())
        .unwrap();
    assert_eq!(now.cell, at.cell.offset(IVec3::new(0, -1, 0)));
    assert!(
        !log(&run)
            .iter()
            .any(|line| line.contains("not what the character knows"))
    );
    let _ = std::fs::remove_dir_all(&data);
}

/// A mod in the data directory that defines one block, and a profile
/// that loads it with the bundled ones.
fn with_extra_mod(data: &Path) -> PathBuf {
    let root = data.join("mods/extra");
    std::fs::create_dir_all(root.join("data")).unwrap();
    std::fs::write(
        root.join("mod.json"),
        r#"{ "id": "extra", "name": "Extra", "version": "1.2.0", "engine": ">=0.1", "requires": ["core"] }"#,
    )
    .unwrap();
    std::fs::write(
        root.join("data/crate.json"),
        r#"[ { "type": "block", "id": "crate", "material": "core:wood",
               "height": 1.0, "fill": 0.5, "flags": ["opaque"],
               "glyph": { "ch": "x", "fg": "material" } } ]"#,
    )
    .unwrap();
    let default = std::fs::read_to_string(repo().join("profiles/default.json")).unwrap();
    let mut profile: serde_json::Value = serde_json::from_str(&default).unwrap();
    profile["id"] = "with_extra".into();
    profile["mods"].as_array_mut().unwrap().push("extra".into());
    let path = data.join("with_extra.json");
    std::fs::write(&path, profile.to_string()).unwrap();
    path
}

#[test]
fn a_save_made_with_a_mod_that_is_gone_says_so() {
    let data = data_dir("save-mods");
    let profile = with_extra_mod(&data);
    let mut run = boot(&data, profile.to_str().unwrap());
    script(&mut run, "enter\ntype Ada\ntab\ntab\nenter\n");
    // A crate of the extra mod beside the character.
    let beside = run
        .game()
        .with_session(|session| {
            let at = session.player_loc().unwrap();
            Location::new(at.grid, at.cell.offset(IVec3::new(1, 0, 0)))
        })
        .unwrap();
    let state = run
        .game()
        .with_session(|session| {
            session
                .sim
                .truth()
                .cosmos()
                .defs()
                .block_state("extra:crate", None)
                .map(|state| state.raw())
        })
        .unwrap()
        .unwrap();
    run.game_mut()
        .submit(Command::Debug(DebugCommand::SetElement {
            loc: beside,
            layer: Layer::Block,
            state,
        }))
        .unwrap();
    script(&mut run, "wait\n");
    save_as(&mut run, "crate");
    assert_eq!(saves(&run), [("crate".to_owned(), "enabled".to_owned())]);

    // The bundled profile alone: the save is listed as made with other
    // mods, and carried on with what differs told.
    let mut again = boot(&data, "default");
    assert_eq!(saves(&again), [("crate".to_owned(), "changed".to_owned())]);
    script(&mut again, "down\nenter\n");
    assert!(again.game().playing());
    assert_eq!(top(&again), "ui:notice", "what differs is shown");
    let text = again.text();
    assert!(text.contains("other mods"), "{text}");
    let lines = log(&again);
    assert!(
        lines.iter().any(|line| line
            == "The save differs: the save was made with the mod `extra` 1.2.0, which is not loaded."),
        "{lines:?}"
    );
    assert!(
        lines.iter().any(|line| line.contains("extra:crate")),
        "the block that is gone is named: {lines:?}"
    );
    script(&mut again, "enter\n");
    assert_eq!(top(&again), "ui:play");
    let _ = std::fs::remove_dir_all(&data);
}

/// A mod in the data directory with two starts of the town, one that
/// cannot be saved and one that has a single save, and a profile that
/// loads it with the bundled ones.
fn with_policies(data: &Path) -> PathBuf {
    let root = data.join("mods/policies");
    std::fs::create_dir_all(root.join("data")).unwrap();
    std::fs::write(
        root.join("mod.json"),
        r#"{ "id": "policies", "name": "Policies", "version": "0.1.0", "engine": ">=0.1", "requires": ["core"] }"#,
    )
    .unwrap();
    std::fs::write(
        root.join("data/starts.json"),
        r#"[ { "type": "start", "id": "a_sitting", "name": "One sitting", "realm": "core:open_town",
               "at": [9, 17], "template": "core:person", "saving": "none" },
             { "type": "start", "id": "b_one_life", "name": "One life", "realm": "core:open_town",
               "at": [9, 17], "template": "core:person", "saving": "single",
               "senses": ["core:human_sight"] } ]"#,
    )
    .unwrap();
    let default = std::fs::read_to_string(repo().join("profiles/default.json")).unwrap();
    let mut profile: serde_json::Value = serde_json::from_str(&default).unwrap();
    profile["id"] = "policies".into();
    profile["mods"]
        .as_array_mut()
        .unwrap()
        .push("policies".into());
    let path = data.join("policies.json");
    std::fs::write(&path, profile.to_string()).unwrap();
    path
}

#[test]
fn a_start_says_how_its_game_is_saved() {
    let data = data_dir("save-policy");
    let profile = with_policies(&data);
    let mut run = boot(&data, profile.to_str().unwrap());

    // One sitting: no saving by hand, and nothing saved on leaving.
    script(
        &mut run,
        "enter\ntype Ada\ntab\ntab\ndown\ndown\ndown\nenter\n",
    );
    assert_eq!(
        run.game().with_session(|session| session.saving.clone()),
        Some("none".to_owned())
    );
    assert!(!run.game().scene().app().can_save());
    script(&mut run, "escape\n");
    assert!(!run.text().contains("Save"), "{}", run.text());
    // Resume, Options, Quit to title; and yes.
    script(&mut run, "down\ndown\nenter\nenter\n");
    assert_eq!(top(&run), "ui:title");
    assert!(saves(&run).is_empty());

    // One life: no saving by hand, and its one save is written on
    // leaving, under the character's name. Its character sees and does
    // not hear: the start names its senses.
    // The new-game screen keeps the name given before.
    script(&mut run, "enter\ntab\ntab\ndown\ndown\ndown\ndown\nenter\n");
    assert_eq!(
        run.game().with_session(|session| session.saving.clone()),
        Some("single".to_owned())
    );
    let senses = run
        .game()
        .with_session(|session| {
            session
                .sim
                .components()
                .senses
                .get(session.player)
                .map(|senses| senses.slots().len())
        })
        .flatten();
    assert_eq!(senses, Some(1));
    assert!(!run.game().scene().app().can_save());
    script(&mut run, "escape\ndown\ndown\nenter\nenter\n");
    assert_eq!(top(&run), "ui:title");
    assert_eq!(saves(&run), [("ada".to_owned(), "enabled".to_owned())]);
    let _ = std::fs::remove_dir_all(&data);
}
