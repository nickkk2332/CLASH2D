//! Game scenarios in the application (13 §12, §9; 12 §10.7), headless
//! from the title screen: the errand begun from the new-game screen with
//! its introduction, its objective in the journal with a marker that
//! points the way, and its completion ending the game with the summary;
//! a scenario that offers a choice of place set up on its own screen;
//! and a game of one save whose save is gone once the party is gone and
//! the game has ended.

use std::path::{Path, PathBuf};

use clash_app::cli::Options;
use clash_app::headless::Headless;
use clash_scene::{AppList, assert_golden};
use clash_sim::{Command, DebugCommand, Subject};
use clash_world::{CellPos, Location};

fn repo() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn golden(name: &str) -> PathBuf {
    repo().join(format!("crates/clash-app/tests/golden/{name}.frame"))
}

fn data_dir(name: &str) -> PathBuf {
    let data = std::env::temp_dir().join(format!("clash-app-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&data);
    std::fs::create_dir_all(&data).unwrap();
    data
}

fn boot(data: &Path) -> Headless {
    let options = Options {
        mods: Some(repo().join("mods")),
        data: Some(data.to_path_buf()),
        seed: 4,
        ..Options::default()
    };
    Headless::new(clash_app::boot(&options).unwrap_or_else(|problem| panic!("{problem}")))
}

fn script(run: &mut Headless, text: &str) {
    run.run_script(text, &mut std::io::sink())
        .unwrap_or_else(|problem| panic!("{problem}"));
}

fn top(run: &Headless) -> String {
    run.game()
        .scene()
        .top()
        .map(|screen| screen.id().to_owned())
        .unwrap_or_default()
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

/// Passes until the screen on top is the one named, or panics.
fn until_top(run: &mut Headless, screen: &str) {
    for _ in 0..600 {
        if top(run) == screen {
            return;
        }
        run.pass();
    }
    panic!("`{screen}` never came to the top; `{}` is there", top(run));
}

#[test]
fn the_errand_is_begun_shown_in_the_journal_and_ends_with_its_summary() {
    let data = data_dir("errand");
    let mut run = boot(&data);
    // The second scenario: the errand. Its introduction comes first.
    script(&mut run, "enter\ntype Ada\ntab\ntab\ndown\nenter\n");
    assert!(run.game().playing());
    assert_eq!(
        run.game()
            .with_session(|session| session.scenario.id.clone())
            .unwrap(),
        "core:the_errand"
    );
    assert_eq!(top(&run), "ui:intro");
    assert_golden(golden("errand_intro"), run.frame());
    script(&mut run, "enter\nwait 30\n");
    assert_eq!(top(&run), "ui:play");

    // The journal: the objective, and which way the town hall is.
    script(&mut run, "shift+j\n");
    assert_eq!(top(&run), "ui:journal");
    let text = run.text();
    assert!(
        text.contains("[ ] Take the parcel to the town hall before nightfall.")
            && text.contains("south, 129 m"),
        "{text}"
    );
    assert_golden(golden("errand_journal"), run.frame());
    script(&mut run, "escape\n");
    assert_eq!(top(&run), "ui:play");

    // To the town hall, with the parcel in hand: a session at its door
    // loads the tracts round it and puts the character there.
    let (player, door) = run
        .game()
        .with_session(|session| {
            let sim = &session.sim;
            let hall = sim.var(Subject::Global, "core:errand_to");
            let Some(Subject::Site(site)) = sim.subject_of(&hall) else {
                panic!("no town hall was chosen: {hall:?}");
            };
            let site = sim.truth().cosmos().site(site).unwrap();
            let grid = sim.truth().cosmos().realm(site.realm).unwrap().base_grid;
            let top = sim.truth().cosmos().topology(grid).unwrap().z_max;
            (
                session.player,
                Location::new(grid, CellPos::new(site.anchor.x, site.anchor.y, top)),
            )
        })
        .unwrap();
    run.game_mut()
        .submit(Command::Debug(DebugCommand::StartSession {
            entity: player,
            at: door,
        }))
        .expect("the session starts at the town hall");
    until_top(&mut run, "ui:summary");
    let lines = log(&run);
    assert!(
        lines
            .iter()
            .any(|line| line == "The game has ended: delivered."),
        "{lines:?}"
    );
    let text = run.text();
    assert!(
        text.contains("Delivered.")
            && text.contains("[x] Take the parcel to the town hall before nightfall."),
        "{text}"
    );
    assert_golden(golden("errand_summary"), run.frame());
    assert!(
        !run.game().scene().app().can_save(),
        "an ended game is not saved"
    );

    // Back to the title, which ends the game.
    script(&mut run, "enter\n");
    assert_eq!(top(&run), "ui:title");
    assert!(!run.game().playing());
    let _ = std::fs::remove_dir_all(&data);
}

#[test]
fn a_scenario_with_a_choice_of_place_is_set_up_first() {
    let data = data_dir("setup");
    let mut run = boot(&data);
    // The fourth: the market town, with two places to begin.
    script(
        &mut run,
        "enter\ntype Ada\ntab\ntab\ndown\ndown\ndown\nenter\n",
    );
    assert!(!run.game().playing());
    assert_eq!(top(&run), "ui:setup");
    let text = run.text();
    assert!(
        text.contains("The north street") && text.contains("The town square"),
        "{text}"
    );
    assert_golden(golden("setup"), run.frame());
    // The square: a site of the town, chosen among those the realm has.
    script(&mut run, "down\nenter\n");
    assert!(run.game().playing(), "{:?}\n{}", log(&run), run.text());
    let site = run
        .game()
        .with_session(|session| {
            let game = session.sim.scenario().game.clone().unwrap();
            let site = game.site.expect("the game began at a site");
            session
                .sim
                .truth()
                .cosmos()
                .site(site)
                .unwrap()
                .name
                .clone()
        })
        .unwrap();
    assert_eq!(site, "Town square");
    assert_eq!(top(&run), "ui:intro");
    let _ = std::fs::remove_dir_all(&data);
}

/// The ids of `app.saves`.
fn saves(run: &Headless) -> Vec<String> {
    run.game()
        .scene()
        .app()
        .entries(AppList::Saves)
        .into_iter()
        .map(|entry| entry.id)
        .collect()
}

#[test]
fn permadeath_run_a_single_save_is_gone_once_the_game_ends() {
    let data = data_dir("permadeath");
    let mut run = boot(&data);
    // The first scenario: the last stand, of one save that ends when the
    // party is gone.
    script(&mut run, "enter\ntype Ada\ntab\ntab\nenter\n");
    assert_eq!(
        run.game()
            .with_session(|session| (session.scenario.id.clone(), session.saving.clone()))
            .unwrap(),
        ("core:last_stand".to_owned(), "single".to_owned())
    );
    script(&mut run, "enter\nwait 10\n");
    assert_eq!(top(&run), "ui:play");
    assert!(!run.game().scene().app().can_save());

    // Left, it is saved under the character's name; and carried on.
    script(&mut run, "escape\ndown\ndown\nenter\nenter\n");
    assert_eq!(top(&run), "ui:title");
    assert_eq!(saves(&run), ["ada"]);
    script(&mut run, "down\nenter\n");
    assert!(run.game().playing(), "{:?}\n{}", log(&run), run.text());
    assert_eq!(top(&run), "ui:play");

    // The party is gone: the game ends, and its one save with it.
    let player = run.game().with_session(|session| session.player).unwrap();
    run.game_mut()
        .submit(Command::Debug(DebugCommand::RemoveEntity {
            entity: player,
        }))
        .unwrap();
    until_top(&mut run, "ui:summary");
    let text = run.text();
    assert!(text.contains("Lost."), "{text}");
    assert!(saves(&run).is_empty(), "the save is gone");
    assert!(!data.join("saves/ada.c2save").exists());
    let lines = log(&run);
    assert!(
        lines
            .iter()
            .any(|line| line == "Its one save (ada) is gone."),
        "{lines:?}"
    );
    // Leaving saves nothing again.
    script(&mut run, "enter\n");
    assert_eq!(top(&run), "ui:title");
    assert!(saves(&run).is_empty());
    let _ = std::fs::remove_dir_all(&data);
}
