//! The reaction prompt in play (03 §7.3; 12 §3.2, §9.5), headless from
//! the title screen: someone beside the player's character swings at
//! them, the character notices the blow in time and the game stops on
//! the prompt with the reactions the character could make; choosing
//! one answers the window and time goes on, and letting it happen
//! answers it too. Without the prompt wired, the bundled rule that asks
//! the player would stop time on a decision nobody could see.

use std::path::{Path, PathBuf};

use clash_app::cli::Options;
use clash_app::headless::Headless;
use clash_core::{EntityId, IVec3, Tick};
use clash_sim::action::{ActionDef, How};
use clash_sim::template::{CharacterSpec, Selection};
use clash_sim::{Command, Control, DebugCommand, Place, Subject};
use clash_world::Location;

fn mods_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../mods")
}

fn data_dir(name: &str) -> PathBuf {
    let data = std::env::temp_dir().join(format!("clash-app-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&data);
    data
}

fn boot(data: &Path) -> Headless {
    let options = Options {
        mods: Some(mods_dir()),
        data: Some(data.to_path_buf()),
        ..Options::default()
    };
    Headless::new(clash_app::boot(&options).unwrap_or_else(|problem| panic!("{problem}")))
}

/// Runs lines of a headless script (12 §3.3).
fn script(run: &mut Headless, text: &str) {
    run.run_script(text, &mut std::io::sink())
        .unwrap_or_else(|problem| panic!("{problem}"));
    assert!(run.running(), "the game ended in the script");
}

fn player(run: &Headless) -> (EntityId, Location) {
    run.game()
        .with_session(|session| (session.player, session.player_loc().unwrap()))
        .expect("a game is being played")
}

fn top(run: &Headless) -> String {
    run.game()
        .scene()
        .top()
        .map(|screen| screen.id().to_owned())
        .unwrap_or_default()
}

fn now(run: &Headless) -> Tick {
    run.game()
        .with_session(|session| session.sim.now())
        .expect("a game is being played")
}

fn asked(run: &Headless) -> bool {
    run.game()
        .with_session(|session| session.sim.pending_decision())
        .flatten()
        .is_some_and(|request| matches!(request.kind, clash_sim::DecisionKind::Reaction { .. }))
}

/// Hands the game a command as a tool would.
fn tool(run: &mut Headless, command: Command) {
    run.game_mut()
        .submit(command)
        .unwrap_or_else(|problem| panic!("{problem}"));
}

/// Begins a game on the lane, walks the player's character a step so
/// that it faces a free cell, and puts a delegated person there, in
/// front of it; then lets a second pass so that the character sees
/// them. Gives both.
fn begin(name: &str) -> (Headless, PathBuf, EntityId, EntityId) {
    let data = data_dir(name);
    let mut run = boot(&data);
    script(
        &mut run,
        "enter\ntab\ntab\ndown\ndown\ndown\ndown\nenter\nwait 5\n",
    );
    let (hero, _) = player(&run);
    let mut placed = None;
    for (key, dx, dy) in [("l", 1, 0), ("h", -1, 0), ("j", 0, 1), ("k", 0, -1)] {
        let (_, before) = player(&run);
        script(&mut run, &format!("{key}\nwait 30\n"));
        let (_, at) = player(&run);
        if at == before {
            continue;
        }
        let id = run
            .game()
            .with_session(|session| session.sim.truth().entities().next_id())
            .flatten()
            .unwrap();
        let spec = CharacterSpec::of(["core:person"])
            .with("core:person.name", Selection::Text("Brute".to_owned()));
        let place = Place::at(Location::new(
            at.grid,
            at.cell.offset(IVec3::new(dx, dy, 0)),
        ));
        if run
            .game_mut()
            .submit(Command::Debug(DebugCommand::Spawn { spec, place }))
            .is_ok()
        {
            placed = Some(id);
            break;
        }
    }
    let brute = placed.expect("a free cell in front of the player");
    tool(
        &mut run,
        Command::Debug(DebugCommand::SetControl {
            entity: brute,
            control: Control::Delegated,
        }),
    );
    tool(
        &mut run,
        Command::Wait {
            entity: hero,
            dur: clash_core::Dur::from_secs(1),
        },
    );
    script(&mut run, "wait 30\n");
    let sees = run
        .game()
        .with_session(|session| clash_sim::perception::perceives(&session.sim, hero, brute))
        .unwrap();
    assert!(sees, "the player's character sees the brute in front of it");
    (run, data, hero, brute)
}

/// The brute punches the player's character.
fn punch(run: &mut Headless, brute: EntityId, hero: EntityId) {
    let (action, target) = run
        .game()
        .with_session(|session| {
            let action = session
                .sim
                .defs()
                .lookup::<ActionDef>("core:punch")
                .expect("the bundled punch");
            let target = session.sim.subject_value(Subject::Entity(hero)).to_stored();
            (action, target)
        })
        .unwrap();
    tool(
        run,
        Command::Act {
            entity: brute,
            action,
            params: vec![("target".to_owned(), target)],
            how: How::Start,
        },
    );
}

/// The player's character waits, as the wait key would have it, so
/// that time runs while the blow comes.
fn stand(run: &mut Headless, hero: EntityId) {
    tool(
        run,
        Command::Wait {
            entity: hero,
            dur: clash_core::Dur::from_secs(3),
        },
    );
}

/// Passes frames until the reaction prompt is on show.
fn until_prompt(run: &mut Headless) {
    for _ in 0..600 {
        if top(run).ends_with("reaction") {
            return;
        }
        run.pass();
    }
    let log: Vec<String> = run
        .game()
        .scene()
        .log()
        .lines()
        .iter()
        .map(|line| format!("{}: {}", line.category, line.text.plain()))
        .collect();
    let state = run
        .game()
        .with_session(|session| {
            format!(
                "now {:?}, pending {:?}, windows {:?}",
                session.sim.now(),
                session.sim.pending_decision(),
                session.sim.windows()
            )
        })
        .unwrap_or_default();
    panic!(
        "the reaction prompt never opened; on top: {}\n{state}\n{log:#?}",
        top(run)
    );
}

#[test]
fn a_blow_noticed_in_time_asks_and_a_reaction_chosen_answers() {
    let (mut run, data, hero, brute) = begin("react");
    punch(&mut run, brute, hero);
    stand(&mut run, hero);
    until_prompt(&mut run);
    assert!(asked(&run), "the simulation waits on the reaction");
    let asked_at = now(&run);

    // The first reaction offered, chosen from the list.
    script(&mut run, "enter\n");
    assert!(!asked(&run), "the reaction answered the window");
    assert!(!top(&run).ends_with("reaction"));
    script(&mut run, "wait 2\n");
    assert!(now(&run) > asked_at, "time goes on");
    let _ = std::fs::remove_dir_all(&data);
}

#[test]
fn letting_a_blow_happen_answers_the_window() {
    let (mut run, data, hero, brute) = begin("pass");
    punch(&mut run, brute, hero);
    stand(&mut run, hero);
    until_prompt(&mut run);
    let asked_at = now(&run);

    script(&mut run, "escape\n");
    assert!(!asked(&run), "passing answered the window");
    script(&mut run, "wait 2\n");
    assert!(now(&run) > asked_at, "time goes on");
    let _ = std::fs::remove_dir_all(&data);
}
