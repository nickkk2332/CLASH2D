//! The second gallery run headless: the `ui` mod's screen of bound
//! widgets, effects, a popup, a preference and a slot, driven through
//! the null frontend with scripted input over the fake host and
//! application, with golden frames along the way (12 §12).

use std::path::PathBuf;

use clash_data::{DefTypes, Diagnostics, Loader};
use clash_scene::testing::{FakeApp, FakeHost};
use clash_scene::ui::{AppService, VarRef};
use clash_scene::{
    Frame, InputEvent, NullFrontend, ROLE_CONFIRM, ROLE_PICK_LIST, Scene, UiDefs, UiOutcome,
    assert_golden,
};

/// The fixed time step of scripted runs, in seconds.
const STEP: f64 = 1.0 / 60.0;

fn mods_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../mods")
}

fn golden(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/golden")
        .join(format!("{name}.frame"))
}

/// A headless run of the gallery as the example sets it up: scripted
/// input in, frames out, effects run.
struct Run {
    scene: Scene<FakeHost>,
    front: NullFrontend,
    frame: Frame,
    app: FakeApp,
    host: FakeHost,
}

impl Run {
    fn new() -> Run {
        let describe = |diagnostics: Diagnostics| diagnostics.render();
        let mut types = DefTypes::new();
        clash_world::register_defs(&mut types);
        clash_scene::register_defs(&mut types);
        let loaded = Loader::new(&types)
            .search_path(mods_dir())
            .load_mod_with_requirements("ui")
            .into_result()
            .unwrap_or_else(|e| panic!("{}", describe(e)));
        let defs = UiDefs::from_defs(&loaded).unwrap_or_else(|e| panic!("{}", describe(e)));
        let (host, app) = (FakeHost::plain(), FakeApp::new());
        let mut scene = defs
            .scene_over(host.clone(), "ui:default", "ui:default")
            .unwrap_or_else(|e| panic!("{}", describe(e)))
            .with_app(Box::new(app.clone()))
            .running_effects();
        scene.set_role(ROLE_CONFIRM, "ui:confirm");
        scene.set_role(ROLE_PICK_LIST, "ui:pick_list");
        scene.open_screen("ui:gallery", Default::default()).unwrap();
        let mut run = Run {
            scene,
            front: NullFrontend::new(80, 24),
            frame: Frame::new(0, 0),
            app,
            host,
        };
        run.press(&[]);
        run
    }

    /// Presses keys, one turn of the loop each, and returns what the
    /// effects came to and whether any turn asked to close.
    fn press(&mut self, keys: &[&str]) -> (Vec<UiOutcome>, bool) {
        let mut outcomes = Vec::new();
        let mut close = false;
        let mut turn = |run: &mut Run| {
            let turn = run.scene.turn(&mut run.front, &mut run.frame, STEP);
            outcomes.extend(turn.outcomes);
            close |= turn.close_requested;
        };
        if keys.is_empty() {
            turn(self);
        }
        for key in keys {
            self.front.script([InputEvent::key(key).unwrap()]);
            turn(self);
        }
        (outcomes, close)
    }

    fn shown(&self) -> &Frame {
        self.front.last().unwrap()
    }

    fn text(&self) -> String {
        self.shown().text()
    }

    fn focus(&self) -> Option<&str> {
        self.scene.top().unwrap().focus()
    }
}

#[test]
fn the_second_gallery_shows_bindings_effects_a_popup_a_preference_and_a_slot() {
    let mut run = Run::new();
    assert!(run.text().contains("Widget gallery"));

    // F2 opens the second gallery over the first, with the active
    // character as its parameter.
    let (out, _) = run.press(&["f2"]);
    assert_eq!(out, [UiOutcome::Opened("ui:gallery_more".into())]);
    assert_golden(golden("gallery_more"), run.shown());
    let text = run.text();
    for line in [
        "A grid of 80 by 24.",
        "Mara carries 3 things.",
        "Lantern, 1.5 kg",
        "Breath 9/10",
        "2 in the party.",
        "3 mods. Saving allowed: true.",
        "Interface (enabled)",
        "Picked: nothing yet.",
        "This line was put into the slot by a ui_part.",
    ] {
        assert!(text.contains(line), "missing `{line}` in:\n{text}");
    }
    assert!(!text.contains("High."));
    assert!(!text.contains("while the level is 5 or more"));

    // A number is typed; what reads the variable follows it, and so do
    // a node's `if` and the condition of a part in the slot.
    assert_eq!(run.focus(), Some("items"));
    let (out, _) = run.press(&["tab", "8"]);
    assert!(out.is_empty(), "nothing changes until the number is taken");
    assert!(run.text().contains("Level < 8_ >"));
    let (out, _) = run.press(&["enter"]);
    assert_eq!(out, [UiOutcome::Changed(VarRef::Screen("level".into()))]);
    assert_golden(golden("gallery_more_level"), run.shown());
    let text = run.text();
    assert!(text.contains("Level < 8 >"), "{text}");
    assert!(text.contains("High."), "{text}");
    assert!(text.contains("while the level is 5 or more"), "{text}");

    // `pick` opens the picker over the screen, dimmed beneath; the rest
    // of the button's effects run with the choice.
    run.press(&["tab", "tab", "tab"]);
    assert_eq!(run.focus(), Some("pick"));
    let (out, _) = run.press(&["enter"]);
    assert_eq!(out, [UiOutcome::Opened("ui:pick_list".into())]);
    assert_golden(golden("gallery_more_pick"), run.shown());
    assert!(run.text().contains("Which thing?"));
    let (out, _) = run.press(&["down", "enter"]);
    assert_eq!(
        out,
        [
            UiOutcome::Closed("ui:pick_list".into()),
            UiOutcome::Changed(VarRef::Screen("picked".into()))
        ]
    );
    assert!(run.text().contains("Picked: Coil of rope."));
    assert_eq!(run.focus(), Some("pick"), "the screen beneath is as it was");

    // The theme preference restyles every open screen.
    run.press(&["shift+tab"]);
    assert_eq!(run.focus(), Some("theme"));
    let before = run.shown().clone();
    let (out, _) = run.press(&["right"]);
    assert_eq!(out, [UiOutcome::Changed(VarRef::Pref("ui:theme".into()))]);
    assert_eq!(
        run.scene.pref("ui:theme").and_then(|theme| theme.as_str()),
        Some("ui:night")
    );
    assert_golden(golden("gallery_more_night"), run.shown());
    let after = run.shown().clone();
    assert_ne!(after.get(0, 0).unwrap().fg, before.get(0, 0).unwrap().fg);
    assert_ne!(
        after.get(40, 23).unwrap().bg,
        before.get(40, 23).unwrap().bg
    );
    assert!(after.text().contains("Theme < Night >"));
    assert!(after.text().contains('╔'), "the night theme's borders");
    assert_eq!(
        run.app.requests().last().map(|request| request.service),
        Some(AppService::SetPref),
        "the application is told, to keep the choice"
    );

    // `confirm` asks in a popup. No leaves everything as it was.
    run.press(&["tab", "tab"]);
    assert_eq!(run.focus(), Some("quit"));
    let (out, _) = run.press(&["enter"]);
    assert_eq!(out, [UiOutcome::Opened("ui:confirm".into())]);
    assert_golden(golden("gallery_more_confirm"), run.shown());
    assert!(run.text().contains("Leave the gallery at level 8?"));
    assert_eq!(run.scene.screens().len(), 3);
    let (out, close) = run.press(&["right", "enter"]);
    assert_eq!(out, [UiOutcome::Closed("ui:confirm".into())]);
    assert!(!close);

    // Esc returns to the first gallery, which kept its state, in the
    // new theme.
    let (out, _) = run.press(&["escape"]);
    assert_eq!(out, [UiOutcome::Closed("ui:gallery_more".into())]);
    assert_eq!(run.scene.top().unwrap().id(), "ui:gallery");
    assert!(run.text().contains("Widget gallery"));
    assert!(run.text().contains('╔'));

    // Yes runs the rest: the application is asked to quit.
    run.press(&["f2", "tab", "tab", "tab", "tab", "tab", "enter"]);
    assert_eq!(run.scene.top().unwrap().id(), "ui:confirm");
    let (out, close) = run.press(&["enter"]);
    assert_eq!(out.len(), 2, "{out:?}");
    assert!(matches!(&out[1], UiOutcome::Service(request) if request.service == AppService::Quit));
    assert!(close);
    assert!(run.scene.quit_requested());
    // Nothing was submitted to the game: the gallery only shows.
    assert!(run.host.commands().is_empty());
}

#[test]
fn the_second_gallery_is_reproducible() {
    let script = [
        "f2", "tab", "right", "right", "right", "tab", "tab", "right", "tab", "enter", "down",
        "enter", "tab", "enter", "escape",
    ];
    let frames = || {
        let mut run = Run::new();
        run.press(&script);
        run.front.frames().to_vec()
    };
    assert_eq!(
        frames(),
        frames(),
        "the same script gives the same frames (12 invariant V8)"
    );
}
