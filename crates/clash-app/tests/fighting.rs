//! Fighting from the keyboard (07; 12 §9.2, §9.3), headless from the
//! title in the test town: Tab selects the nearest creature the
//! character sees as the target; `f` strikes it with the first blow that
//! can be made; shift+f lists the blows, greying those that cannot be
//! made with their reason, and asks for the target, the part and the
//! effort from the action's own parameters; walking into someone asks
//! before striking. Golden frames are kept of the target selected, the
//! blow struck, the list of blows, the part picker and the question.
//!
//! The golden frames are written with `BLESS=1 cargo test -p clash-app`.

mod common;

use clash_app::headless::Headless;
use clash_core::{EntityId, IVec3};
use clash_scene::assert_golden;
use clash_sim::template::{CharacterSpec, Selection};
use clash_sim::{Command, DebugCommand, Place};
use clash_world::Location;
use common::*;

/// The test town is the third game of the bundled list.
const TOWN: usize = 2;

/// Begins in the test town and puts someone called Brute two cells east
/// of the character, then lets them be seen.
fn begin_with_brute(name: &str) -> (Headless, std::path::PathBuf, EntityId, EntityId) {
    let data = data_dir(name);
    let mut run = boot_default(&data);
    begin(&mut run, TOWN, "Ada");
    let (hero, at) = player(&run);
    let brute = run
        .game()
        .with_session(|session| session.sim.truth().entities().next_id())
        .flatten()
        .unwrap();
    let spec = CharacterSpec::of(["core:person"])
        .with("core:person.name", Selection::Text("Brute".to_owned()));
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
    (run, data, hero, brute)
}

fn target_of(run: &Headless) -> Option<EntityId> {
    run.game()
        .scene()
        .target()
        .map(|sprite| EntityId::from_bits(sprite.raw()))
}

#[test]
fn tab_selects_the_target_and_f_strikes_it() {
    let (mut run, data, _, brute) = begin_with_brute("fight-quick");
    assert_eq!(target_of(&run), None, "nothing is selected at first");
    script(&mut run, "tab\n");
    assert_eq!(target_of(&run), Some(brute), "Tab selects who is in sight");
    // Named as the character knows them: a stranger, by what they look
    // like (05 §4.6; decision D-862).
    assert!(run.text().contains("Target The person"), "{}", run.text());
    assert_golden(golden("target_selected"), run.frame());

    // The quick blow: the first that can be made, at the target, with
    // the action's defaults.
    let before = commands(&run);
    script(&mut run, "f\n");
    assert!(
        commands(&run) > before,
        "a blow was started: {:?}",
        log(&run)
    );
    let (action, params) = last_act(&run);
    assert_eq!(
        action, "core:kick",
        "unarmed, the first blow by its slots and id"
    );
    assert_eq!(params, ["effort", "target"], "the effort is its default");
    script(&mut run, "wait 60\n");
    settle(&mut run);
    assert!(
        log(&run)
            .iter()
            .any(|(_, line)| line.contains("strikes the person")),
        "the blow is told: {:?}",
        log(&run)
    );
    assert_golden(golden("struck"), run.frame());
    let _ = std::fs::remove_dir_all(&data);
}

#[test]
fn the_precise_blow_asks_for_the_blow_the_part_and_the_effort() {
    let (mut run, data, _, _) = begin_with_brute("fight-precise");
    script(&mut run, "tab\nshift+f\n");
    assert_eq!(top(&run), "ui:pick_list", "the blows are listed");
    let listed = run.text();
    assert!(listed.contains("Punch"), "{listed}");
    // A blade's blows cannot be made with empty hands, and say why.
    assert!(
        listed.contains("Slash: Slash needs a blade in hand."),
        "{listed}"
    );
    assert_golden(golden("attack_choose"), run.frame());

    // Choosing a greyed one is refused with its reason.
    script(&mut run, "down\ndown\ndown\nenter\n");
    assert_eq!(top(&run), "ui:play");
    assert_eq!(
        log(&run).last().map(|(_, line)| line.as_str()),
        Some("Slash needs a blade in hand.")
    );

    // Punch: the target, filled in with the one selected; the part,
    // from the parts the character sees on them; the effort.
    script(&mut run, "shift+f\ndown\nenter\n");
    assert_eq!(top(&run), "ui:pick_entity");
    script(&mut run, "enter\n");
    assert_eq!(top(&run), "ui:pick_list", "the part is asked for");
    let parts = run.text();
    assert!(parts.contains("wherever it lands"), "{parts}");
    assert!(parts.contains("head"), "{parts}");
    assert_golden(golden("attack_part"), run.frame());
    let before = commands(&run);
    script(&mut run, "down\nenter\n");
    assert_eq!(top(&run), "ui:pick_number", "the effort is asked for");
    script(&mut run, "enter\n");
    assert!(
        commands(&run) > before,
        "the punch was started: {:?}",
        log(&run)
    );
    let (action, mut params) = last_act(&run);
    params.sort();
    assert_eq!(action, "core:punch");
    assert_eq!(params, ["effort", "part", "target"]);

    // Holds and takedowns are listed together, at the target.
    script(&mut run, "wait 60\nv\n");
    assert_eq!(top(&run), "ui:pick_list");
    let listed = run.text();
    assert!(listed.contains("Grab"), "{listed}");
    script(&mut run, "escape\n");
    assert_eq!(top(&run), "ui:play");
    let _ = std::fs::remove_dir_all(&data);
}

#[test]
fn the_second_button_offers_what_can_be_done_with_someone() {
    let (mut run, data, _, _) = begin_with_brute("fight-menu");
    // Brute stands two cells east of the character, which the map draws
    // at the middle of its view.
    let at = run
        .text()
        .lines()
        .enumerate()
        .find_map(|(row, line)| line.find("@.@").map(|col| (col + 2, row)))
        .expect("the two are drawn side by side");
    let click = |kind| {
        clash_scene::InputEvent::Mouse(clash_scene::input::MouseEvent {
            kind,
            col: u16::try_from(at.0).unwrap(),
            row: u16::try_from(at.1).unwrap(),
            mods: clash_scene::Mods::empty(),
        })
    };
    use clash_scene::input::{MouseButton, MouseKind};
    run.input(click(MouseKind::Down(MouseButton::Right)));
    run.input(click(MouseKind::Up(MouseButton::Right)));
    assert_eq!(top(&run), "ui:context_menu");
    let menu = run.text();
    for offer in [
        "The person",
        "Attack",
        "Shoot",
        "Talk",
        "Go there",
        "Look around",
    ] {
        assert!(menu.contains(offer), "{offer}: {menu}");
    }
    assert_golden(golden("context_menu"), run.frame());
    let taken = commands(&run);
    script(&mut run, "enter\n");
    assert!(
        commands(&run) > taken,
        "the blow was started: {:?}",
        log(&run)
    );
    assert_eq!(last_act(&run).0, "core:kick");
    assert_eq!(top(&run), "ui:play");
    let _ = std::fs::remove_dir_all(&data);
}

#[test]
fn walking_into_someone_asks_before_striking() {
    let (mut run, data, _, _) = begin_with_brute("fight-bump");
    // A step toward them is not taken: Brute is in the way, and is not
    // held hostile, so the player is asked.
    script(&mut run, "l\n");
    let (_, before) = player(&run);
    let taken = commands(&run);
    script(&mut run, "l\n");
    assert_eq!(top(&run), "ui:confirm", "{:?}", log(&run));
    assert_eq!(player(&run).1, before);
    assert_golden(golden("bump_ask"), run.frame());
    script(&mut run, "enter\n");
    assert!(commands(&run) > taken, "the blow was started");
    let (action, params) = last_act(&run);
    assert_eq!(action, "core:kick");
    assert_eq!(params, ["effort", "target"], "the effort is its default");
    let _ = std::fs::remove_dir_all(&data);
}
