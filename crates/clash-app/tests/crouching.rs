//! Crouching from the keyboard (12 §9.2; D-930): `shift+c` runs the
//! control `crouch`, which takes the bundled action `set_posture` to
//! crouched, the posture whose steps are soft; pressed again, the
//! character stands.

mod common;

use clash_sim::movement::PostureDef;

use common::*;

/// The name of the posture the player's character is in; empty for
/// none.
fn posture(run: &clash_app::headless::Headless) -> String {
    run.game()
        .with_session(|session| {
            let id = clash_sim::movement::posture_of(&session.sim, session.player)?;
            Some(
                session
                    .sim
                    .defs()
                    .registry::<PostureDef>()
                    .name(id)
                    .to_owned(),
            )
        })
        .flatten()
        .unwrap_or_default()
}

#[test]
fn shift_c_crouches_and_stands_again() {
    let data = data_dir("crouching");
    let mut run = boot_default(&data);
    begin(&mut run, 2, "Ada");
    settle(&mut run);
    // A character begins in no posture, which is standing as far as
    // anything reads it.
    assert_ne!(posture(&run), "core:crouched");

    script(&mut run, "shift+c\nwait 60\n");
    assert_eq!(posture(&run), "core:crouched", "{:?}", log(&run));

    script(&mut run, "shift+c\nwait 60\n");
    assert_eq!(posture(&run), "core:standing", "{:?}", log(&run));
    let _ = std::fs::remove_dir_all(&data);
}
