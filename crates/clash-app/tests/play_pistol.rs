//! Play session: the pistol in a fight (07 §11; 12 §9.3), from the title
//! in the market town, as a player plays it: a marksman made on the
//! creation screen with the pistol and spare rounds rather than the bow;
//! someone in the square shot at until the pistol is empty, the watch
//! coming for the one who shoots; the empty pistol refused with its
//! reason, reloaded from the backpack with `r`, and fired again.
//! `docs/playtest-beta.md`, the fifth round.

mod session;
mod town;

use clash_app::headless::Headless;
use clash_expr::Value;
use clash_sim::Subject;
use session::*;
use town::*;

/// How many rounds the firearm in the character's hand holds; -1 with
/// none in hand.
fn rounds(run: &Headless) -> i64 {
    run.game()
        .with_session(|session| {
            let pistol = session.sim.property_named(
                Subject::Entity(session.player),
                "wielding",
                &[Value::text("firearm")],
            );
            match session.sim.subject_of(&pistol) {
                Some(subject) => session.sim.property_named(subject, "rounds", &[]),
                None => Value::Null,
            }
        })
        .and_then(|value| value.as_int())
        .unwrap_or(-1)
}

/// From the title: the market town, F2, the marksman, and the weapon
/// chosen: the pistol. Begun in the square.
fn a_marksman_with_a_pistol(run: &mut Headless) {
    create(run, TOWN_LIFE, MARKSMAN, "Ada");
    let shown = run.text();
    assert!(shown.contains("Choices"), "{shown}");
    tab_to(run, "choices");
    // The weapon is the marksman's own choice, after the four every
    // person makes.
    for _ in 0..4 {
        press(run, "down");
    }
    press(run, "enter");
    assert_eq!(top(run), "ui:create_choice", "{}", run.text());
    let listed = run.text();
    assert!(
        listed.contains("A pistol, loaded, and spare rounds"),
        "{listed}"
    );
    press(run, "down");
    press(run, "enter");
    press(run, "escape");
    assert_eq!(top(run), "ui:create");
    let shown = run.text();
    assert!(
        shown.contains("A pistol, loaded, and spare rounds"),
        "{shown}"
    );
    tab_to(run, "begin");
    press(run, "enter");
    until_top(run, "ui:setup", 60);
    press(run, "down");
    press(run, "enter");
    until_top(run, "ui:intro", 60);
    press(run, "enter");
    settle(run);
    assert_eq!(top(run), "ui:play", "{}", run.text());
}

/// A step to a free cell beside the character, the first of south,
/// east, north and west it can take.
fn step_back(run: &mut Headless) {
    let at = player_loc(run);
    for key in ["j", "l", "k", "h"] {
        press(run, key);
        if top(run) == "ui:confirm" {
            press(run, "escape");
        }
        until_free(run, 30);
        if player_loc(run) != at {
            return;
        }
    }
}

/// Shoots at whoever is in sight with `s` until the pistol in hand is
/// empty or `keys` turns have gone, answering blows; with nobody in
/// sight, a second goes by. Returns the shots started.
fn shoot_until_empty(run: &mut Headless, keys: usize) -> usize {
    let start = acts_of(run, "core:shoot");
    for _ in 0..keys {
        if rounds(run) == 0 {
            break;
        }
        answer(run);
        if top(run) != "ui:play" {
            continue;
        }
        if run.game().scene().target().is_none() || target_dead(run) {
            press(run, "tab");
        }
        if run.game().scene().target().is_none() {
            press(run, ".");
            continue;
        }
        let before = commands(run);
        press(run, "s");
        if !["ui:play", "ui:reaction"].contains(&top(run).as_str()) {
            press(run, "escape");
        }
        if commands(run) == before && top(run) == "ui:play" {
            // Refused: with the target in the shooter's own cell there is
            // nowhere to aim (D-973), and a step back is taken.
            if target_loc(run).is_some() && target_loc(run) == player_loc(run) {
                step_back(run);
            } else {
                press(run, ".");
            }
        }
        until_free(run, 30);
    }
    acts_of(run, "core:shoot") - start
}

#[test]
fn the_pistol_fired_emptied_and_reloaded_in_a_fight() {
    let data = DataDir::new("pistol-fight");
    let mut run = boot_default(data.path(), 4);
    a_marksman_with_a_pistol(&mut run);
    let shown = run.text();
    assert!(shown.contains("Holding a pistol"), "{shown}");
    let loaded = rounds(&run);
    assert!(loaded > 0, "the pistol is loaded: {loaded}");
    eprintln!("loaded {loaded}, carried {:?}", carried(&run));

    // The first in sight is the watchman at his post in the middle of
    // the square, in the cell the game began her in: a shot at him is
    // nowhere to aim, and is refused with that reason, not a fault
    // (D-973).
    assert!(tab_until_someone(&mut run, 30), "nobody in the square");
    if target_loc(&run) == player_loc(&run) {
        let taken = commands(&run);
        press(&mut run, "s");
        assert_eq!(commands(&run), taken, "nothing was started");
        let lines = log_text(&run);
        assert_eq!(
            lines.last().map(String::as_str),
            Some("There is no loaded weapon in hand, or nowhere to aim."),
            "{lines:?}"
        );
        step_back(&mut run);
    }

    // Someone in the square shot at until the pistol is empty.
    let before = log_len(&run);
    let shots = shoot_until_empty(&mut run, 200);
    settle(&mut run);
    let fight = log_since(&run, before);
    eprintln!("{shots} shots: {fight:?}");
    assert!(
        !fight.iter().any(|line| line.starts_with("Fault")),
        "{fight:?}"
    );
    assert_eq!(rounds(&run), 0, "{fight:?}");
    assert_eq!(shots as i64, loaded, "{fight:?}");
    assert!(
        fight.iter().any(|line| line.starts_with("Ada shoots")),
        "{fight:?}"
    );

    // Empty: the shot is refused, with the reason.
    for _ in 0..20 {
        answer(&mut run);
        if top(&run) == "ui:play" {
            break;
        }
        run.pass();
    }
    if run.game().scene().target().is_none() {
        tab_until_someone(&mut run, 10);
    }
    let taken = commands(&run);
    press(&mut run, "s");
    if top(&run) != "ui:play" {
        press(&mut run, "escape");
    }
    assert_eq!(commands(&run), taken, "nothing was started");
    let lines = log_text(&run);
    assert!(
        lines
            .iter()
            .any(|line| line == "There is no loaded weapon in hand, or nowhere to aim."),
        "{lines:?}"
    );

    // Reloaded from the backpack, and fired again.
    let before = log_len(&run);
    for _ in 0..10 {
        answer(&mut run);
        if top(&run) == "ui:play" {
            press(&mut run, "r");
            until_free(&mut run, 60);
            if rounds(&run) > 0 {
                break;
            }
        }
        press(&mut run, ".");
    }
    settle(&mut run);
    let reloaded = log_since(&run, before);
    assert!(rounds(&run) > 0, "{reloaded:?}");
    assert!(
        reloaded.iter().any(|line| line == "Ada reloads."),
        "{reloaded:?}"
    );
    let full = rounds(&run);
    let again = shoot_until_empty(&mut run, 40).min(1);
    eprintln!("after reloading: {:?}", log_since(&run, before));
    assert!(again >= 1 || rounds(&run) < full, "fired again");
}
