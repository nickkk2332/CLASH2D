//! The screens of play that are not fighting (12 §10.7), headless from
//! the title: the map, drawn from what the character remembers with the
//! journal's marker pinned; the journal, opened from play; and the keys,
//! with every control the mods define. Talking is `tests/talking.rs`.
//!
//! The golden frames are written with `BLESS=1 cargo test -p clash-app`.

mod common;

use clash_scene::assert_golden;
use common::*;

/// The errand is the second game of the bundled list; the town the
/// third.
const ERRAND: usize = 1;
const TOWN: usize = 2;

#[test]
fn the_map_shows_what_is_known_with_the_marker_pinned() {
    let data = data_dir("map");
    let mut run = boot_default(&data);
    begin(&mut run, ERRAND, "Ada");
    // The introduction, then the map.
    script(&mut run, "escape\n");
    settle(&mut run);
    script(&mut run, "m\n");
    assert_eq!(top(&run), "ui:map");
    let pins = run
        .game()
        .scene()
        .world()
        .map(|world| world.pins().len())
        .unwrap_or_default();
    assert_eq!(pins, 1, "the town hall's door is pinned");
    let shown = run.text();
    assert!(
        shown.contains(" m Take the parc"),
        "which way and how far: {shown}"
    );
    assert_golden(golden("errand_map"), run.frame());

    // The journal is a key away, and back.
    script(&mut run, "escape\nshift+j\n");
    assert_eq!(top(&run), "ui:journal");
    script(&mut run, "escape\n");
    assert_eq!(top(&run), "ui:play");
    let _ = std::fs::remove_dir_all(&data);
}

#[test]
fn the_keys_list_every_control() {
    let data = data_dir("keys");
    let mut run = boot_default(&data);
    begin(&mut run, TOWN, "Ada");
    settle(&mut run);
    script(&mut run, "?\n");
    assert_eq!(top(&run), "ui:help");
    let listed = run.text();
    // A control with no key is listed too.
    assert!(listed.contains("Attack who is in the way"), "{listed}");
    assert!(listed.contains("shift+f"), "{listed}");
    assert_golden(golden("help_controls"), run.frame());
    script(&mut run, "escape\n");
    assert_eq!(top(&run), "ui:play");
    let _ = std::fs::remove_dir_all(&data);
}
