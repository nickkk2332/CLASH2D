//! Input, controls and pickers (12 §9; package U7): the `control`
//! definition and the keymap's contexts, `act` with `needed` and `all`,
//! one picker for each kind of parameter, the mouse, and the screens for
//! decisions.
//!
//! Everything runs through the null frontend with scripted input, over
//! the fake host and application of `clash_scene::testing`. The screens,
//! the keymap and the controls are the `ui` mod's own, under the roles of
//! the repository's default profile. The actions are a fixture mod's
//! (`tests/fixtures/mods/fix`), loaded as the simulation's own `action`
//! type: the scene knows none of them by name. The simulation's stats
//! are left out of the load, because the fake host's registry has an
//! `entity.pool` of its own.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use clash_data::{DefTypes, Defs, Diagnostics, Loader};
use clash_expr::Value;
use clash_scene::control::{
    COMMAND_CHOOSE, COMMAND_PASS, COMMAND_REACT, ControlDef, DecisionAsk, PendingDecision,
    ROLE_PLAY, ReactionOption,
};
use clash_scene::testing::{FakeApp, FakeHost};
use clash_scene::ui::{AppService, Collect, CommandKind};
use clash_scene::view::{CellContents, EntitySprite, LIGHT_FULL, SpriteId, SpriteKind, WorldView};
use clash_scene::{
    Frame, GlyphPart, InputEvent, Mods, MouseButton, MouseEvent, MouseKind, NullFrontend, Scene,
    UiCommand, UiDefs, UiOutcome, assert_golden, check_defs,
};
use clash_world::{BlockState, CellPos, GridId, Location, WorldDefs};

/// The fixed time step of scripted runs, in seconds.
const STEP: f64 = 1.0 / 60.0;

fn repository() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn fixtures() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/mods")
}

fn golden(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/golden")
        .join(format!("{name}.frame"))
}

fn types() -> DefTypes {
    let mut types = DefTypes::new();
    clash_data::expr::register_defs(&mut types);
    clash_world::register_defs(&mut types);
    clash_sim::action::register_defs(&mut types);
    clash_scene::register_defs(&mut types);
    types
}

/// The repository's default profile, with the fixture mod in place of
/// `core`: the roles are the ones the game starts with.
fn profile() -> String {
    let path = repository().join("profiles/default.json");
    let text = std::fs::read_to_string(&path).expect("the default profile");
    let mut profile: serde_json::Value = serde_json::from_str(&text).expect("a profile");
    profile["id"] = "test".into();
    profile["mods"] = serde_json::json!(["ui", "fix"]);
    profile.to_string()
}

fn load() -> Arc<Defs> {
    let describe = |diagnostics: Diagnostics| diagnostics.render();
    let types = types();
    let registry = clash_scene::testing::registry();
    let defs = Loader::new(&types)
        .expr_registry(&registry)
        .search_path(repository().join("mods"))
        .search_path(fixtures())
        .memory_profile("test", &profile())
        .load_profile("test")
        .into_result()
        .unwrap_or_else(|e| panic!("{}", describe(e)));
    let found = check_defs(&defs);
    assert!(!found.has_errors(), "{}", found.render());
    defs
}

/// A headless game: the play screen over a small room, with Mara (`@`,
/// who is `view`) in the middle, a stranger (`s`) three cells east and
/// Jonas (`j`) to the south-west. East of the room is a strip the party
/// remembers and does not see.
struct Run {
    scene: Scene<FakeHost>,
    front: NullFrontend,
    frame: Frame,
    host: FakeHost,
    app: FakeApp,
}

/// The cell Mara stands in.
const MARA: (i32, i32) = (4, 3);
/// The cell the stranger stands in.
const STRANGER: (i32, i32) = (7, 3);
/// The cell Jonas stands in.
const JONAS: (i32, i32) = (2, 5);

fn at(x: i32, y: i32) -> Location {
    Location::new(GridId::new(0), CellPos::new(x, y, 0))
}

impl Run {
    fn new() -> Run {
        let defs = load();
        let describe = |diagnostics: Diagnostics| diagnostics.render();
        let ui = UiDefs::from_defs(&defs).unwrap_or_else(|e| panic!("{}", describe(e)));
        let (host, app) = (FakeHost::new(), FakeApp::new());
        let mut scene = ui
            .scene_over(host.clone(), "ui:default", "ui:default")
            .unwrap_or_else(|e| panic!("{}", describe(e)))
            .with_app(Box::new(app.clone()))
            .running_effects();

        // What the party perceives, put into the view model by hand, as
        // perception will.
        let world = WorldDefs::from_defs(&defs).unwrap_or_else(|e| panic!("{}", describe(e)));
        let glyphs = ui
            .world_glyphs(&world, scene.theme())
            .unwrap_or_else(|e| panic!("{}", describe(e)));
        let mut view = WorldView::new(glyphs, at(MARA.0, MARA.1));
        let floor = world.floor_state("fix:boards", None).expect("boards");
        let wall = world.block_state("fix:wall", None).expect("a wall");
        for y in 0..7 {
            for x in 0..14 {
                let edge = x == 0 || y == 0 || x == 10 || y == 6;
                let contents = CellContents {
                    floor,
                    block: if edge && x <= 10 {
                        wall
                    } else {
                        BlockState::NONE
                    },
                    light: LIGHT_FULL,
                    ..CellContents::EMPTY
                };
                view.model_mut().observe_cell(at(x, y), contents);
                if x > 10 {
                    view.model_mut().unsee_cell(at(x, y));
                }
            }
        }
        for (number, (x, y), ch) in [(1, MARA, '@'), (2, JONAS, 'j'), (3, STRANGER, 's')] {
            let sprite = EntitySprite {
                loc: at(x, y),
                kind: SpriteKind::Creature,
                glyph: GlyphPart {
                    ch: Some(ch),
                    ..GlyphPart::default()
                },
                own: None,
                dress: None,
                facing: None,
                suspected: false,
            };
            view.model_mut()
                .upsert_sprite(SpriteId::new(number), sprite);
            host.place(host.entity(number), x, y, 0);
        }
        scene.set_world(view);

        let play = scene.role(ROLE_PLAY).expect("the play role").to_owned();
        scene.open_screen(&play, BTreeMap::new()).unwrap();
        let mut run = Run {
            scene,
            front: NullFrontend::new(80, 24),
            frame: Frame::new(0, 0),
            host,
            app,
        };
        run.press(&[]);
        run
    }

    fn turn(&mut self, events: Vec<InputEvent>) -> Vec<UiOutcome> {
        self.front.script(events);
        self.scene
            .turn(&mut self.front, &mut self.frame, STEP)
            .outcomes
    }

    /// Presses keys, one turn of the loop each, and returns what the
    /// effects came to.
    fn press(&mut self, keys: &[&str]) -> Vec<UiOutcome> {
        if keys.is_empty() {
            return self.turn(Vec::new());
        }
        let mut outcomes = Vec::new();
        for key in keys {
            outcomes.extend(self.turn(vec![InputEvent::key(key).unwrap()]));
        }
        // What the last key changed is drawn by the next pass.
        outcomes.extend(self.turn(Vec::new()));
        outcomes
    }

    fn mouse(&mut self, kind: MouseKind, col: u16, row: u16) -> Vec<UiOutcome> {
        let event = InputEvent::Mouse(MouseEvent {
            kind,
            col,
            row,
            mods: Mods::empty(),
        });
        let mut outcomes = self.turn(vec![event]);
        outcomes.extend(self.turn(Vec::new()));
        outcomes
    }

    fn shown(&self) -> &Frame {
        self.front.last().unwrap()
    }

    fn top(&self) -> &str {
        self.scene.top().unwrap().id()
    }

    /// The row of the frame that holds `needle`.
    fn row_with(&self, needle: &str) -> String {
        let frame = self.shown();
        (0..frame.rows)
            .map(|row| frame.row_text(row))
            .find(|row| row.contains(needle))
            .unwrap_or_else(|| panic!("no row holds `{needle}` in:\n{}", frame.text()))
            .trim()
            .to_owned()
    }

    fn cursor(&self) -> Option<(i32, i32)> {
        let camera = self.scene.world().unwrap().camera();
        camera.cursor().map(|cell| (cell.x, cell.y))
    }

    fn entity(&self, number: u64) -> Value {
        Value::Subject(self.host.entity(number))
    }

    fn item(&self, number: u64) -> Value {
        Value::Subject(self.host.item(number))
    }
}

/// The one command among the outcomes that was submitted.
fn submitted(outcomes: &[UiOutcome]) -> &UiCommand {
    let found: Vec<&UiCommand> = outcomes
        .iter()
        .filter_map(|outcome| match outcome {
            UiOutcome::Submitted(command) => Some(command),
            _ => None,
        })
        .collect();
    assert_eq!(found.len(), 1, "one command: {outcomes:?}");
    found[0]
}

fn real(value: f64) -> Value {
    Value::Real(clash_core::Real::of(value))
}

fn args<const N: usize>(entries: [(&str, Value); N]) -> BTreeMap<String, Value> {
    entries
        .into_iter()
        .map(|(name, value)| (name.to_owned(), value))
        .collect()
}

// ---------------------------------------------------------------------
// The play screen, controls and the keymap
// ---------------------------------------------------------------------

#[test]
fn the_play_screen_shows_the_map_the_log_and_a_status_column_with_slots() {
    let mut run = Run::new();
    assert_eq!(run.top(), "ui:play");
    assert_eq!(run.scene.top().unwrap().context(), "play");
    let text = run.shown().text();
    assert!(text.contains("Mara"), "{text}");
    assert!(text.contains("One of 2."), "{text}");
    // The fixture's part is in the play screen's slot.
    assert!(run.row_with("Blood").contains("████"), "{text}");
    assert_eq!(
        run.scene.top().unwrap().slots().collect::<Vec<_>>(),
        ["play.bars", "play.side"]
    );
    assert_golden(golden("play_first"), run.shown());

    // A walking key starts the action the control names, with the
    // direction the control gives: the engine knows neither.
    for (key, dir) in [
        ("h", "west"),
        ("left", "west"),
        ("u", "north_east"),
        ("<", "up"),
    ] {
        let outcomes = run.press(&[key]);
        assert_eq!(
            *submitted(&outcomes),
            UiCommand {
                kind: CommandKind::Act,
                name: "fix:step".to_owned(),
                args: args([("dir", Value::text(dir))]),
                collect: Collect::Given,
            },
            "{key}"
        );
    }
    // Waiting is a command of the simulation, not an action.
    let outcomes = run.press(&["."]);
    let wait = submitted(&outcomes);
    assert_eq!(
        (wait.kind, wait.name.as_str()),
        (CommandKind::Command, "wait")
    );
    assert_eq!(wait.args, args([("dur", Value::Int(1))]));
    // A control that only runs effects, bound by its qualified name.
    let outcomes = run.press(&["s"]);
    assert_eq!(submitted(&outcomes).name, "shout");

    // The bundled screens open, titled, and Esc leaves each (the map
    // gives the camera back as it goes).
    for (key, screen, title) in [
        ("i", "ui:inventory", "Inventory"),
        ("c", "ui:condition", "Condition"),
        ("a", "ui:abilities", "Abilities"),
        ("m", "ui:map", "Map"),
        ("o", "ui:options", "Options"),
    ] {
        let outcomes = run.press(&[key]);
        assert_eq!(outcomes, [UiOutcome::Opened(screen.to_owned())]);
        assert!(run.shown().row_text(0).contains(title));
        assert_eq!(
            run.press(&["escape"]).last(),
            Some(&UiOutcome::Closed(screen.to_owned()))
        );
    }
    assert_eq!(run.top(), "ui:play");
}

#[test]
fn look_mode_moves_a_free_cursor_and_says_what_is_known_of_the_place() {
    let mut run = Run::new();
    run.press(&["x"]);
    assert_eq!(run.top(), "ui:look");
    assert_eq!(run.cursor(), Some(MARA));
    assert_eq!(run.row_with("Look"), "Look  Mara. In sight, 0.0 m away.");
    run.press(&["l", "l", "l"]);
    assert_eq!(run.cursor(), Some(STRANGER));
    assert_eq!(
        run.row_with("Look"),
        "Look  A tall figure in red. In sight, 3.0 m away."
    );
    // Into the strip that is only remembered, and past what is known.
    run.press(&["l", "l", "l", "l", "l"]);
    assert_eq!(
        run.row_with("Look"),
        "Look  Nobody there. Remembered, 8.0 m away."
    );
    run.press(&["l", "l", "l"]);
    assert_eq!(
        run.row_with("Look"),
        "Look  Nobody there. Not known, 11.0 m away."
    );
    // Tab goes through what the party perceives, nearest first.
    run.press(&["tab"]);
    assert_eq!(run.cursor(), Some(JONAS));
    run.press(&["tab"]);
    assert_eq!(run.cursor(), Some(STRANGER));
    run.press(&["shift+tab"]);
    assert_eq!(run.cursor(), Some(JONAS));
    // Leaving gives the view back to the character.
    run.press(&["escape"]);
    assert_eq!(run.top(), "ui:play");
    assert_eq!(run.cursor(), None);
}

#[test]
fn rebinding_a_key_changes_what_it_does_and_leaves_controls_alone() {
    let mut run = Run::new();
    let controls = |run: &Run| -> Vec<ControlDef> {
        let ui = run.scene.defs().unwrap();
        ui.ids::<ControlDef>()
            .iter()
            .map(|id| ui.get::<ControlDef>(id).unwrap().clone())
            .collect()
    };
    let before = controls(&run);
    // `w` means nothing in play; `f` strikes.
    assert_eq!(run.press(&["w"]), []);
    assert_eq!(submitted(&run.press(&["f"])).name, "fix:strike");

    // The fixture's F5 asks the `app` service to give `w` the meaning
    // `strike`. It takes effect at once, and the application is told, so
    // that it can keep the binding.
    let outcomes = run.press(&["f5"]);
    assert!(
        matches!(&outcomes[..], [UiOutcome::Service(request)] if request.service == AppService::Rebind),
        "{outcomes:?}"
    );
    let kept = run.app.requests();
    assert_eq!(kept.len(), 1);
    assert_eq!(
        (
            kept[0].service,
            kept[0].text("context"),
            kept[0].text("key"),
            kept[0].text("name")
        ),
        (AppService::Rebind, Some("play"), Some("w"), Some("strike"))
    );
    assert_eq!(submitted(&run.press(&["w"])).name, "fix:strike");

    // The application gives stored bindings back through the scene: `f`
    // now walks west, and an empty name takes a key's meaning away.
    run.scene.rebind("play", "f", "west").unwrap();
    let step = run.press(&["f"]);
    assert_eq!(submitted(&step).name, "fix:step");
    assert_eq!(submitted(&step).args, args([("dir", Value::text("west"))]));
    run.scene.rebind("play", "h", "").unwrap();
    assert_eq!(run.press(&["h"]), []);
    assert!(run.scene.rebind("play", "hyper+h", "west").is_err());

    // No control changed: only what the keys mean.
    assert_eq!(controls(&run), before);
}

// ---------------------------------------------------------------------
// `act` with `needed` and `all`
// ---------------------------------------------------------------------

/// The strike as its defaults give it: at the first thing Mara has seen.
fn default_strike(run: &Run) -> BTreeMap<String, Value> {
    args([
        ("target", run.entity(3)),
        ("spot", run.host.loc(STRANGER.0, STRANGER.1, 0)),
        ("dir", Value::text("north")),
        ("force", real(2.5)),
        ("times", Value::Int(3)),
        ("style", Value::text("low")),
        ("with", run.item(1)),
        ("extra", Value::list([])),
    ])
}

#[test]
fn one_key_press_starts_an_action_from_its_defaults() {
    let mut run = Run::new();
    let outcomes = run.press(&["f"]);
    // No picker opened: the one outcome is the command, whole.
    assert_eq!(
        outcomes,
        [UiOutcome::Submitted(UiCommand {
            kind: CommandKind::Act,
            name: "fix:strike".to_owned(),
            args: default_strike(&run),
            collect: Collect::Given,
        })]
    );
    assert_eq!(run.top(), "ui:play");
    assert_eq!(run.host.commands().len(), 1);
}

#[test]
fn the_precise_twin_asks_for_every_parameter_filled_in_with_its_default() {
    let mut run = Run::new();
    // Accepting every picker as it comes gives the same command as the
    // quick key: each was filled in with the default.
    let opened = run.press(&["shift+f"]);
    assert_eq!(opened, [UiOutcome::Opened("ui:pick_entity".to_owned())]);
    assert_eq!(run.cursor(), Some(STRANGER), "on the default target");
    assert_eq!(
        run.row_with("Strike: target"),
        "Strike: target  A tall figure in red (2 of 2), 3.0 m. In reach."
    );
    assert_golden(golden("pick_entity"), run.shown());
    let mut walked = vec![run.top().to_owned()];
    let mut outcomes = Vec::new();
    for key in [
        "enter", "enter", "enter", "enter", "enter", "enter", "enter",
    ] {
        outcomes = run.press(&[key]);
        walked.push(run.top().to_owned());
    }
    assert_eq!(
        walked,
        [
            "ui:pick_entity",
            "ui:pick_cell",
            "ui:pick_direction",
            "ui:pick_number",
            "ui:pick_number",
            "ui:pick_list",
            "ui:pick_list",
            "ui:pick_multi",
        ]
    );
    assert!(run.host.commands().is_empty(), "nothing until the last");
    assert_eq!(
        outcomes,
        [
            UiOutcome::Closed("ui:pick_list".to_owned()),
            UiOutcome::Opened("ui:pick_multi".to_owned())
        ]
    );
    assert_golden(golden("pick_multi"), run.shown());
    // The list is empty by default: Done ends it.
    let outcomes = run.press(&["ctrl+d"]);
    let command = submitted(&outcomes);
    assert_eq!(command.name, "fix:strike");
    assert_eq!(command.args, default_strike(&run));
    assert_eq!(run.top(), "ui:play");
    assert_eq!(run.cursor(), None, "the camera is the character's again");
}

#[test]
fn every_picker_takes_a_choice_of_its_own() {
    let mut run = Run::new();
    run.press(&["shift+f"]);
    // The entity picker cycles through what the party perceives.
    run.press(&["tab"]);
    assert_eq!(run.cursor(), Some(JONAS));
    assert_eq!(
        run.row_with("Strike: target"),
        "Strike: target  Jonas (1 of 2), 2.8 m. In reach."
    );
    run.press(&["enter"]);
    // The cell picker starts on the default, which follows the target
    // just chosen: where Jonas is believed to be. One west of that.
    assert_eq!(run.top(), "ui:pick_cell");
    assert_eq!(run.cursor(), Some(JONAS));
    run.press(&["h"]);
    assert_eq!(run.cursor(), Some((JONAS.0 - 1, JONAS.1)));
    run.press(&["enter"]);
    // The direction picker: one key.
    assert_golden(golden("pick_direction"), run.shown());
    run.press(&["l"]);
    // The number picker: type a real, Enter takes it, Enter accepts.
    assert!(run.row_with("Value").contains("Value  < 2.5 >"));
    run.press(&["4", ".", "2", "5"]);
    assert_golden(golden("pick_number"), run.shown());
    run.press(&["enter", "enter"]);
    // A whole number steps and stays whole.
    assert_eq!(run.top(), "ui:pick_number");
    run.press(&["right", "right", "enter"]);
    // A list the control gives: `low` is selected; `high` is above it.
    assert_eq!(run.top(), "ui:pick_list");
    run.press(&["up", "enter"]);
    // What Mara carries: the rope is the second.
    run.press(&["down", "enter"]);
    // Several of what she carries: both, and the list is full at two.
    assert_eq!(run.top(), "ui:pick_multi");
    run.press(&["enter", "down", "enter"]);
    assert!(run.row_with("chosen").contains("2 of at most 2 chosen."));
    let outcomes = run.press(&["ctrl+d"]);
    assert_eq!(
        submitted(&outcomes).args,
        args([
            ("target", run.entity(2)),
            ("spot", run.host.loc(JONAS.0 - 1, JONAS.1, 0)),
            ("dir", Value::text("east")),
            ("force", real(4.25)),
            ("times", Value::Int(5)),
            ("style", Value::text("high")),
            ("with", run.item(2)),
            ("extra", Value::list([run.item(1), run.item(2)])),
        ])
    );
}

#[test]
fn cancelling_a_picker_cancels_the_whole_command() {
    let mut run = Run::new();
    // Two parameters in, at the direction picker, Esc.
    run.press(&["shift+f", "enter", "enter"]);
    assert_eq!(run.top(), "ui:pick_direction");
    let outcomes = run.press(&["escape"]);
    assert_eq!(
        outcomes,
        [
            UiOutcome::Closed("ui:pick_direction".to_owned()),
            UiOutcome::Cancelled {
                command: "fix:strike".to_owned()
            }
        ]
    );
    assert_eq!(run.top(), "ui:play");
    assert!(run.host.commands().is_empty(), "nothing was submitted");
    // At the first picker too, and the cursor it took is given back.
    run.press(&["shift+f"]);
    assert_eq!(run.cursor(), Some(STRANGER));
    run.press(&["escape"]);
    assert_eq!((run.top(), run.cursor()), ("ui:play", None));
    assert!(run.host.commands().is_empty());
    // The key still works afterwards.
    assert_eq!(submitted(&run.press(&["f"])).name, "fix:strike");
}

#[test]
fn needed_asks_only_for_what_has_no_default() {
    let mut run = Run::new();
    // `what` has no default: a list of what the control offers. `at`
    // has none: the cell picker. `force` has one and is not asked for.
    assert_eq!(
        run.press(&["t"]),
        [UiOutcome::Opened("ui:pick_list".to_owned())]
    );
    assert!(run.shown().text().contains("Throw: what"));
    assert!(run.shown().text().contains("Coil of rope"));
    run.press(&["down", "enter"]);
    assert_eq!(run.top(), "ui:pick_cell");
    run.press(&["j"]);
    let outcomes = run.press(&["enter"]);
    assert_eq!(
        *submitted(&outcomes),
        UiCommand {
            kind: CommandKind::Act,
            name: "fix:throw".to_owned(),
            args: args([
                ("what", run.item(2)),
                ("at", run.host.loc(MARA.0, MARA.1 + 1, 0)),
                ("force", real(1.5)),
            ]),
            collect: Collect::Given,
        }
    );
}

#[test]
fn an_action_may_be_picked_before_its_parameters() {
    let mut run = Run::new();
    assert_eq!(
        run.press(&["shift+a"]),
        [UiOutcome::Opened("ui:pick_list".to_owned())]
    );
    assert!(run.shown().text().contains("fix:throw"));
    // The second is the throw, which then asks for what it needs.
    run.press(&["down", "enter"]);
    assert!(run.shown().text().contains("Throw: what"));
    run.press(&["enter", "enter"]);
    assert_eq!(run.host.commands().len(), 1);
    assert_eq!(run.host.commands()[0].name, "fix:throw");
    // The first is the strike, which needs nothing asked.
    let outcomes = run.press(&["shift+a", "enter"]);
    assert_eq!(submitted(&outcomes).name, "fix:strike");
    // Leaving the list of actions abandons the command.
    run.press(&["shift+a", "escape"]);
    assert_eq!(run.host.commands().len(), 2);
    assert_eq!(run.top(), "ui:play");
}

// ---------------------------------------------------------------------
// The cell picker
// ---------------------------------------------------------------------

#[test]
fn the_cell_picker_moves_shows_reach_and_sight_and_returns_the_cell() {
    let mut run = Run::new();
    run.press(&["t", "enter"]);
    assert_eq!(run.top(), "ui:pick_cell");
    assert_eq!(run.cursor(), Some(MARA));
    assert_eq!(
        run.row_with("Throw: at"),
        "Throw: at  In sight, 0.0 m. In reach."
    );
    // Five east: still in the room, in reach, with the check's advice.
    run.press(&["l", "l", "l", "l", "l"]);
    assert_eq!(run.cursor(), Some((MARA.0 + 5, MARA.1)));
    assert_eq!(
        run.row_with("Throw: at"),
        "Throw: at  In sight, 5.0 m. In reach.  A long throw."
    );
    // Three more: past the wall, where the party only remembers, and
    // further than the action reaches, with the action's own words.
    run.press(&["l", "l", "l"]);
    assert_eq!(
        run.row_with("Throw: at"),
        "Throw: at  Remembered, 8.0 m. Out of reach. Too far to throw. A long throw."
    );
    // The line from Mara to the cursor is drawn across the cells between.
    assert_golden(golden("pick_cell_far"), run.shown());
    // Diagonals and the other way; then the cell is returned.
    run.press(&["y", "h", "h", "h"]);
    assert_eq!(run.cursor(), Some((MARA.0 + 4, MARA.1 - 1)));
    let outcomes = run.press(&["enter"]);
    assert_eq!(
        submitted(&outcomes).args["at"],
        run.host.loc(MARA.0 + 4, MARA.1 - 1, 0)
    );
    assert_eq!(run.cursor(), None);
}

#[test]
fn an_answer_the_parameter_cannot_hold_is_refused_and_the_picker_stays() {
    let mut run = Run::new();
    run.press(&["shift+f"]);
    // One cell west of the stranger nobody stands: there is nothing to
    // choose there.
    run.press(&["h"]);
    assert!(
        run.row_with("Strike: target")
            .contains("Nobody there (0 of 2)")
    );
    let outcomes = run.press(&["enter"]);
    assert_eq!(
        outcomes,
        [UiOutcome::Failed {
            effect: "answer",
            reason: "`target` cannot be nothing".to_owned()
        }]
    );
    assert_eq!(run.top(), "ui:pick_entity");
    assert!(run.host.commands().is_empty());
}

// ---------------------------------------------------------------------
// The mouse
// ---------------------------------------------------------------------

/// The cell of the frame that shows a cell of the world, in a map view
/// with its top left at the frame's and of this size, centred on
/// `centre`.
fn on_screen(cell: (i32, i32), centre: (i32, i32), size: (i32, i32)) -> (u16, u16) {
    let (mid_col, mid_row) = ((size.0 - 1) / 2, (size.1 - 1) / 2);
    (
        (mid_col + cell.0 - centre.0) as u16,
        (mid_row + cell.1 - centre.1) as u16,
    )
}

#[test]
fn a_mouse_click_on_the_map_moves_the_cursor_and_then_chooses_the_cell() {
    let mut run = Run::new();
    run.press(&["t", "enter"]);
    assert_eq!(run.top(), "ui:pick_cell");
    // The picker's map fills the frame but for its two status rows.
    let size = (80, 22);
    let wanted = (MARA.0 + 2, MARA.1 - 1);
    // A click away from the cursor moves it there, and the screen says
    // what is known of the place.
    let (col, row) = on_screen(wanted, MARA, size);
    let outcomes = run.mouse(MouseKind::Down(MouseButton::Left), col, row);
    assert!(run.host.commands().is_empty(), "{outcomes:?}");
    assert_eq!(run.cursor(), Some(wanted));
    assert_eq!(
        run.row_with("Throw: at"),
        "Throw: at  In sight, 2.2 m. In reach."
    );
    // The view follows the cursor, so the cell is now in the middle. A
    // click on the cursor chooses it.
    let (col, row) = on_screen(wanted, wanted, size);
    let outcomes = run.mouse(MouseKind::Down(MouseButton::Left), col, row);
    assert_eq!(
        submitted(&outcomes).args["at"],
        run.host.loc(wanted.0, wanted.1, 0)
    );
    assert_eq!(run.top(), "ui:play");
}

#[test]
fn a_click_on_the_play_screen_runs_the_click_controls_and_the_wheel_zooms() {
    let mut run = Run::new();
    // The play screen's map is left of the status column and above the
    // log, with no cursor on show: a click picks at once and runs the
    // control `click`, whose action no loaded mod defines. The host is
    // handed the name as the control wrote it, with the place.
    let size = (54, 17);
    let (col, row) = on_screen(STRANGER, MARA, size);
    let outcomes = run.mouse(MouseKind::Down(MouseButton::Left), col, row);
    assert_eq!(
        *submitted(&outcomes),
        UiCommand {
            kind: CommandKind::Act,
            name: "go_to".to_owned(),
            args: args([("to", run.host.loc(STRANGER.0, STRANGER.1, 0))]),
            collect: Collect::Given,
        }
    );
    // The second button opens what the profile names for it.
    let outcomes = run.mouse(MouseKind::Down(MouseButton::Right), col, row);
    assert_eq!(outcomes, [UiOutcome::Opened("ui:context_menu".to_owned())]);
    run.press(&["escape"]);
    // The wheel over the map zooms it.
    assert_eq!(run.scene.world().unwrap().camera().zoom(), 1);
    run.mouse(MouseKind::Wheel(1), col, row);
    assert_eq!(run.scene.world().unwrap().camera().zoom(), 2);
    run.mouse(MouseKind::Wheel(-1), col, row);
    assert_eq!(run.scene.world().unwrap().camera().zoom(), 1);
}

// ---------------------------------------------------------------------
// Decisions
// ---------------------------------------------------------------------

fn reaction(run: &Run) -> PendingDecision {
    PendingDecision {
        entity: run.entity(1),
        kind: DecisionAsk::Reaction {
            window: Value::Int(7),
            options: vec![
                ReactionOption {
                    action: "fix:dodge".to_owned(),
                    label: "Dodge".to_owned(),
                    in_time: true,
                },
                ReactionOption {
                    action: "fix:parry".to_owned(),
                    label: "Parry".to_owned(),
                    in_time: false,
                },
            ],
            time_left: 0.25,
            time_total: 0.4,
            what: "The fixture: Punch".to_owned(),
        },
    }
}

#[test]
fn a_reaction_prompt_appears_for_a_pending_decision_and_the_response_is_submitted() {
    let mut run = Run::new();
    // Nothing is waiting: nothing is shown.
    assert_eq!(run.press(&[]), []);
    run.host.ask(Some(reaction(&run)));
    assert_eq!(
        run.press(&[]),
        [UiOutcome::Opened("ui:reaction".to_owned())]
    );
    assert_eq!(run.scene.decision(), Some(&reaction(&run)));
    let text = run.shown().text();
    assert!(
        text.contains("Mara can react, with 0.2 s to spare."),
        "{text}"
    );
    assert!(
        text.contains("The fixture: Punch"),
        "what is happening: {text}"
    );
    assert!(text.contains("Parry (too late)"), "{text}");
    assert_golden(golden("reaction_prompt"), run.shown());
    // It is shown once, however many passes go by.
    assert_eq!(run.press(&[]), []);

    // The first response: its one parameter has a default, so it goes
    // at once, as `react` with the window and the action.
    let outcomes = run.press(&["enter"]);
    assert_eq!(
        *submitted(&outcomes),
        UiCommand {
            kind: CommandKind::Command,
            name: COMMAND_REACT.to_owned(),
            args: args([
                ("entity", run.entity(1)),
                ("window", Value::Int(7)),
                ("action", Value::text("fix:dodge")),
                ("params", Value::map([("dir", Value::text("south"))])),
            ]),
            collect: Collect::Given,
        }
    );
    assert_eq!(run.top(), "ui:play");
    assert_eq!(run.scene.decision(), None);

    // The second response needs to be told something first: a picker.
    run.host.ask(Some(reaction(&run)));
    run.press(&[]);
    assert_eq!(
        run.press(&["down", "enter"]),
        [
            UiOutcome::Closed("ui:reaction".to_owned()),
            UiOutcome::Opened("ui:pick_list".to_owned())
        ]
    );
    assert!(run.shown().text().contains("Parry: high"));
    let outcomes = run.press(&["enter"]);
    let react = submitted(&outcomes);
    assert_eq!(react.args["action"], Value::text("fix:parry"));
    assert_eq!(
        react.args["params"],
        Value::map([("high", Value::Bool(true))])
    );

    // Esc lets it happen: `pass`.
    run.host.ask(Some(reaction(&run)));
    run.press(&[]);
    let outcomes = run.press(&["escape"]);
    assert_eq!(
        *submitted(&outcomes),
        UiCommand {
            kind: CommandKind::Command,
            name: COMMAND_PASS.to_owned(),
            args: args([("entity", run.entity(1)), ("window", Value::Int(7))]),
            collect: Collect::Given,
        }
    );
    assert_eq!(run.host.commands().len(), 3);
}

#[test]
fn a_choice_is_answered_with_its_number_and_a_free_decision_is_the_play_screens() {
    let mut run = Run::new();
    let choice = PendingDecision {
        entity: run.entity(1),
        kind: DecisionAsk::Choice {
            choice: Value::Int(4),
            prompt: "The door is locked.".to_owned(),
            options: vec!["Force it".to_owned(), "Leave it".to_owned()],
        },
    };
    run.host.ask(Some(choice));
    assert_eq!(run.press(&[]), [UiOutcome::Opened("ui:choice".to_owned())]);
    assert!(run.shown().text().contains("The door is locked."));
    // A choice must be made: Esc does nothing.
    assert_eq!(run.press(&["escape"]), []);
    assert_eq!(run.top(), "ui:choice");
    let outcomes = run.press(&["down", "enter"]);
    assert_eq!(
        *submitted(&outcomes),
        UiCommand {
            kind: CommandKind::Command,
            name: COMMAND_CHOOSE.to_owned(),
            args: args([
                ("entity", run.entity(1)),
                ("choice", Value::Int(4)),
                ("option", Value::Int(1)),
            ]),
            collect: Collect::Given,
        }
    );
    assert_eq!(run.top(), "ui:play");

    // A free decision opens nothing: the play screen is already there,
    // and whatever the player does next answers it.
    let free = PendingDecision {
        entity: run.entity(1),
        kind: DecisionAsk::Free,
    };
    run.host.ask(Some(free.clone()));
    assert_eq!(run.press(&[]), []);
    assert_eq!(run.scene.decision(), Some(&free));
    assert_eq!(run.scene.screens().len(), 1);
    // A decision the host refuses the answer to is asked again.
    run.host.ask(Some(reaction(&run)));
    run.press(&[]);
    run.host.refuse(COMMAND_PASS);
    let outcomes = run.press(&["escape"]);
    assert!(
        outcomes
            .iter()
            .any(|outcome| matches!(outcome, UiOutcome::Refused { .. })),
        "{outcomes:?}"
    );
    assert_eq!(run.top(), "ui:reaction", "shown again");
}

// ---------------------------------------------------------------------
// Belief, and what is wrong with data
// ---------------------------------------------------------------------

#[test]
fn a_picker_offers_only_what_the_view_model_and_the_host_say() {
    let mut run = Run::new();
    // The stranger slips out of sight: perception takes the sprite away.
    // Nothing else changed, and the picker no longer offers them.
    run.scene
        .world_mut()
        .unwrap()
        .model_mut()
        .remove_sprite(SpriteId::new(3));
    run.press(&["shift+f"]);
    assert_eq!(run.cursor(), Some(JONAS), "the only one perceived");
    assert!(run.row_with("Strike: target").contains("(1 of 1)"));
    run.press(&["escape"]);
    // What the host says the character believes is there to be chosen
    // is what a list offers, and nothing else.
    run.host.offer("what", vec![run.item(2)]);
    let defs = run.scene.defs().unwrap().clone();
    assert!(defs.control("fix:throw", "").is_some());
    // The control gives `from` for `what`, so the host is not asked.
    run.press(&["t"]);
    assert!(run.shown().text().contains("Lantern"));
    run.press(&["escape"]);
    assert!(run.host.commands().is_empty());
}

/// Loads the `ui` mod with one more mod written here, and returns what
/// is wrong with it.
fn problems(text: &str) -> String {
    let types = types();
    let registry = clash_scene::testing::registry();
    let outcome = Loader::new(&types)
        .expr_registry(&registry)
        .search_path(repository().join("mods"))
        .memory_mod("bad", r#"{ "id": "bad" }"#, &[("data/bad.json", text)])
        .load_mods(&["ui", "bad"]);
    assert!(outcome.has_errors(), "the mod should not load");
    outcome.diagnostics.render()
}

#[test]
fn a_wrong_control_is_a_located_diagnostic() {
    let text = problems(
        r#"[
  { "type": "control", "id": "a", "do": { "opn": "inventory" } },
  { "type": "control", "id": "b", "do": { "act": { "action": "step", "pick": { "dir": "direction" } } } },
  { "type": "control", "id": "c", "do": { "act": { "action": "step", "params": { "dir": "wset" } } } },
  { "type": "control", "id": "d", "does": [] },
  { "type": "control", "id": "e", "do": { "camera": { "mov": [1, 0] } } },
  { "type": "control", "id": "f", "do": { "picker": "finish" } },
  { "type": "keymap", "id": "keys", "play": { "hyper+x": "a" } }
]"#,
    );
    for needle in [
        "unknown effect `opn`",
        "`pick` and `from` say how parameters are asked for",
        "wset",
        "does",
        "mov",
        "finish",
        "hyper+x",
        "--> mods/bad/data/bad.json:2:",
        "--> mods/bad/data/bad.json:3:",
        "--> mods/bad/data/bad.json:4:",
        "--> mods/bad/data/bad.json:5:",
        "--> mods/bad/data/bad.json:6:",
        "--> mods/bad/data/bad.json:7:",
        "--> mods/bad/data/bad.json:8:",
    ] {
        assert!(text.contains(needle), "missing `{needle}` in:\n{text}");
    }
}

#[test]
fn an_action_no_mod_defines_is_the_hosts_to_refuse() {
    // The `ui` mod alone: its walking controls name `step`, which
    // nothing defines. With no definition to read, the command is
    // handed to the host as it was written, and the host says why it
    // will not have it.
    let types = types();
    let defs = Loader::new(&types)
        .search_path(repository().join("mods"))
        .memory_mod(
            "more",
            r#"{ "id": "more" }"#,
            &[(
                "data/more.json",
                r#"[ { "type": "control", "id": "leap", "do": { "act": { "action": "leap", "params": "needed" } } },
                     { "type": "keymap", "id": "ui:default", "patch": true, "play": { "z": "leap" } } ]"#,
            )],
        )
        .load_mods(&["ui", "more"])
        .into_result()
        .unwrap_or_else(|e| panic!("{}", e.render()));
    let ui = UiDefs::from_defs(&defs).unwrap();
    let host = FakeHost::plain();
    host.refuse("leap");
    let mut scene = ui
        .scene_over(host.clone(), "ui:default", "ui:default")
        .unwrap()
        .running_effects();
    scene.open_screen("ui:play", BTreeMap::new()).unwrap();
    let mut frame = Frame::new(80, 24);
    scene.build_frame(&mut frame);
    assert_eq!(
        scene.handle(&InputEvent::key("z").unwrap()),
        [UiOutcome::Refused {
            command: UiCommand {
                kind: CommandKind::Act,
                name: "leap".to_owned(),
                args: BTreeMap::new(),
                collect: Collect::Needed,
            },
            reason: "the fake host refuses `leap`".to_owned()
        }]
    );
    let outcomes = scene.handle(&InputEvent::key("h").unwrap());
    assert_eq!(submitted(&outcomes).name, "step");
    assert!(frame.text().contains("Mara"));
}

/// The one service among the outcomes that was asked for.
fn service(outcomes: &[UiOutcome]) -> &clash_scene::AppRequest {
    let found: Vec<&clash_scene::AppRequest> = outcomes
        .iter()
        .filter_map(|outcome| match outcome {
            UiOutcome::Service(request) => Some(request),
            _ => None,
        })
        .collect();
    assert_eq!(found.len(), 1, "one service: {outcomes:?}");
    found[0]
}

#[test]
fn talking_asks_the_application_what_can_be_said_and_says_the_one_chosen() {
    let mut run = Run::new();
    // With nobody selected the application is asked with nothing, and
    // the screen says how to choose someone.
    let asked = service(&run.press(&["shift+t"])).clone();
    assert_eq!(asked.service, AppService::Talk);
    assert!(asked.args.get("to").is_none_or(|to| *to == Value::Null));
    assert_eq!(run.top(), "ui:talk");
    assert!(run.row_with("Nobody is chosen").contains("Tab"));
    run.press(&["escape"]);

    // Someone selected: what the application gathered is listed, and
    // the one chosen is said to them with the action `say`.
    // In this harness the target cycle may pick anyone in sight: the
    // request says whom.
    run.press(&["tab"]);
    let option = |id: &str, text: &str, kind: Value, lines: Value| {
        Value::map([
            ("id", Value::from(id)),
            ("text", Value::from(text)),
            ("says", Value::Bool(kind != Value::Null)),
            ("kind", kind),
            ("lines", lines),
        ])
    };
    let gathered = |to: Value| {
        Value::map([
            ("to", to),
            (
                "options",
                Value::list([
                    option(
                        "greet",
                        "Greet",
                        Value::from("fix:greet"),
                        Value::from("fix:greetings"),
                    ),
                    option("stare", "Stare", Value::Null, Value::Null),
                ]),
            ),
            ("why", Value::from("")),
        ])
    };
    let asked = service(&run.press(&["shift+t"])).clone();
    let who = asked.args.get("to").cloned().unwrap_or(Value::Null);
    assert_ne!(who, Value::Null);
    // What the application gathers for them as it carries the service
    // out, before the screen opens.
    run.press(&["escape"]);
    run.app.set_talk(gathered(who.clone()));
    run.press(&["shift+t"]);
    assert!(run.row_with("Greet").contains("Greet"));
    assert!(run.row_with("Stare").contains("says nothing"));
    let said = run.press(&["enter"]);
    let command = submitted(&said);
    assert_eq!(command.name, "say");
    assert_eq!(command.args.get("to"), Some(&who));
    assert_eq!(command.args.get("kind"), Some(&Value::from("fix:greet")));
    assert_eq!(
        command.args.get("lines"),
        Some(&Value::from("fix:greetings"))
    );
    assert_eq!(run.top(), "ui:play");

    // Nothing can be said: the application's reason is shown.
    run.app.set_talk(Value::map([
        ("to", who),
        ("options", Value::list([])),
        ("why", Value::from("You cannot see or hear them.")),
    ]));
    run.press(&["shift+t"]);
    assert!(run.row_with("cannot see or hear").contains("You cannot"));
    assert!(run.row_with("There is nothing to say").contains("nothing"));
}
