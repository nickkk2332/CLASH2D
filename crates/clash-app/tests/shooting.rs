//! Shooting from the keyboard (07 §11; 12 §9.3), headless from the
//! title in the `armoury` fixture's range: a pistol in hand with two
//! rounds, six more in a backpack, and someone down the street. `s`
//! shoots the target with the first shot that can be made; once the
//! pistol is empty the shot is refused with the action's own reason;
//! `r` reloads from the backpack and the next shot is made. Shift+s
//! lists the shots and asks for every parameter: the target, the place
//! aimed at (filled in with where the character believes the target is,
//! with the line drawn to it), the height and the count.
//!
//! The golden frames are written with `BLESS=1 cargo test -p clash-app`.

mod common;

use clash_app::headless::Headless;
use clash_expr::Value;
use clash_scene::assert_golden;
use clash_sim::Subject;
use common::*;

/// The range is the second game of the fixture's list.
const RANGE: usize = 1;

/// How many rounds the pistol in the character's hand holds.
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

fn begin_range(name: &str) -> (Headless, std::path::PathBuf) {
    let data = data_dir(name);
    let mut run = boot_armoury(&data);
    begin(&mut run, RANGE, "Ada");
    settle(&mut run);
    (run, data)
}

#[test]
fn shooting_until_empty_then_reloading() {
    let (mut run, data) = begin_range("shoot");
    assert_eq!(rounds(&run), 2);
    script(&mut run, "tab\n");
    assert!(run.game().scene().target().is_some(), "someone to shoot at");

    // Two shots, each the plain one at the target.
    script(&mut run, "s\nwait 30\n");
    assert_eq!(last_act(&run).0, "core:shoot");
    script(&mut run, "s\nwait 30\n");
    assert_eq!(rounds(&run), 0, "both rounds fired");

    // Empty: the shot cannot be made, and the action says why.
    let taken = commands(&run);
    script(&mut run, "s\n");
    assert_eq!(commands(&run), taken, "nothing was started");
    assert_eq!(
        log(&run).last().map(|(_, line)| line.as_str()),
        Some("There is no loaded weapon in hand, or nowhere to aim.")
    );
    settle(&mut run);
    assert_golden(golden("empty_weapon"), run.frame());

    // Reloading from the backpack fills it, and the next shot is made.
    script(&mut run, "r\nwait 30\n");
    assert_eq!(last_act(&run).0, "core:reload");
    assert!(rounds(&run) > 0, "reloaded: {:?}", log(&run));
    script(&mut run, "s\nwait 30\n");
    assert_eq!(last_act(&run).0, "core:shoot");
    let _ = std::fs::remove_dir_all(&data);
}

#[test]
fn aiming_asks_for_the_place_with_the_line_drawn_to_it() {
    let (mut run, data) = begin_range("aim");
    script(&mut run, "tab\nshift+s\n");
    assert_eq!(top(&run), "ui:pick_list", "the shots are listed");
    let listed = run.text();
    assert!(listed.contains("Shoot"), "{listed}");
    assert!(
        listed.contains("Draw and loose: "),
        "a bow's shot is greyed: {listed}"
    );
    assert_golden(golden("aim_choose"), run.frame());

    // The plain shot: the target, filled in with the one selected; the
    // place, filled in with where the character believes they are.
    script(&mut run, "enter\n");
    assert_eq!(top(&run), "ui:pick_entity");
    script(&mut run, "enter\n");
    assert_eq!(top(&run), "ui:pick_cell", "the place aimed at is asked for");
    assert!(
        run.game()
            .scene()
            .world()
            .is_some_and(|world| world.line_shown()),
        "the line to the place is drawn"
    );
    assert_golden(golden("aim_cell"), run.frame());
    let before = commands(&run);
    script(&mut run, "enter\nenter\nenter\n");
    assert!(
        commands(&run) > before,
        "the shot was made: {:?}",
        log(&run)
    );
    let (action, mut params) = last_act(&run);
    params.sort();
    assert_eq!(action, "core:shoot");
    assert_eq!(params, ["at", "count", "height", "target"]);
    let _ = std::fs::remove_dir_all(&data);
}
