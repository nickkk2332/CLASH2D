//! Talking (15 §11; 12 §10.7; decision D-799), headless from the title in
//! the test town: Tab selects someone in sight, shift+t gathers what the
//! character could say to them (the rules on `talk_options`, raised on a
//! copy of the simulation, so that asking changes nothing) and lists it;
//! Enter says the one chosen with the action `say`, which anyone in
//! earshot hears; the one greeted greets back, and the log has both.
//! Golden frames are kept of the options and of the reply in the log.
//!
//! The golden frames are written with `BLESS=1 cargo test -p clash-app`.

mod common;

use clash_core::IVec3;
use clash_scene::assert_golden;
use clash_sim::template::{CharacterSpec, Selection};
use clash_sim::{Command, DebugCommand, Place};
use clash_world::Location;
use common::*;

/// The test town is the third game of the bundled list.
const TOWN: usize = 2;

#[test]
fn a_greeting_is_chosen_from_what_can_be_said_and_answered_in_the_log() {
    let data = data_dir("talk");
    let mut run = boot_default(&data);
    begin(&mut run, TOWN, "Ada");
    let (hero, at) = player(&run);

    // Nobody chosen: the screen says how to choose someone.
    script(&mut run, "shift+t\n");
    assert_eq!(top(&run), "ui:talk");
    assert!(run.text().contains("Nobody is chosen"), "{}", run.text());
    script(&mut run, "escape\n");

    // Bram stands two cells east, and is seen.
    let spec = CharacterSpec::of(["core:person"])
        .with("core:person.name", Selection::Text("Bram".to_owned()));
    tool(
        &mut run,
        Command::Debug(DebugCommand::Spawn {
            spec,
            place: Place::at(Location::new(at.grid, at.cell.offset(IVec3::new(2, 0, 0)))),
        }),
    );
    tool(
        &mut run,
        Command::Wait {
            entity: hero,
            dur: clash_core::Dur::from_millis(500),
        },
    );
    script(&mut run, "wait 30\n");
    settle(&mut run);
    script(&mut run, "tab\n");
    // Named as the character knows them (05 §4.6; decision D-862).
    assert!(run.text().contains("Target The person"), "{}", run.text());

    // What can be said to Bram, gathered without changing the game.
    let hash = run
        .game()
        .with_session(|session| session.sim.state_hash())
        .unwrap();
    let before = commands(&run);
    script(&mut run, "shift+t\n");
    assert_eq!(top(&run), "ui:talk");
    assert_eq!(
        run.game()
            .with_session(|session| session.sim.state_hash())
            .unwrap(),
        hash,
        "asking what can be said changes nothing"
    );
    assert_eq!(commands(&run), before);
    let shown = run.text();
    assert!(shown.contains("Talk to the person"), "{shown}");
    assert!(shown.contains("Greet"), "{shown}");
    assert!(shown.contains("Tell them to stay back"), "{shown}");
    assert_golden(golden("talk_options"), run.frame());

    // The greeting is said with `say`, and Bram greets back.
    script(&mut run, "enter\n");
    assert_eq!(top(&run), "ui:play");
    let (action, params) = last_act(&run);
    assert_eq!(action, "core:say");
    for name in ["to", "kind", "lines"] {
        assert!(params.contains(&name.to_owned()), "{params:?}");
    }
    script(&mut run, "wait 60\n");
    settle(&mut run);
    // Ada's words were worded for Bram, so the log says only that she
    // spoke; Bram's are worded for Ada, who has not learned his name.
    let lines: Vec<String> = log(&run).into_iter().map(|(_, text)| text).collect();
    assert!(
        lines.iter().any(|line| line == "Ada speaks to the person."),
        "what the character said: {lines:?}"
    );
    let replies: Vec<&String> = lines
        .iter()
        .filter(|line| {
            ["Good day", "Morning", "raises a hand"]
                .iter()
                .any(|words| line.contains(words))
        })
        .collect();
    assert_eq!(replies.len(), 1, "one greeting back: {lines:?}");
    assert_golden(golden("talk_reply"), run.frame());
    let _ = std::fs::remove_dir_all(&data);
}
