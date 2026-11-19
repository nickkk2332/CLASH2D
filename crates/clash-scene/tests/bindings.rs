//! Bindings, interface effects, the screen stack, preferences and slots
//! (12 §10.4 to §10.6; packages U6b and U8), through the mod loader and
//! against the fake host and application of `clash_scene::testing`.

use std::collections::BTreeMap;
use std::sync::Arc;

use clash_core::Value;
use clash_data::{DefTypes, Defs, Diagnostics, Loader};
use clash_expr::Value as ExprValue;
use clash_scene::testing::{FakeApp, FakeHost};
use clash_scene::ui::{AppService, Collect, CommandKind, VarRef};
use clash_scene::{
    Frame, InputEvent, Scene, UiCommand, UiDefs, UiOutcome, check_defs, register_defs,
};

/// A theme, a second theme, a keymap and the screens the `confirm` and
/// `pick` effects open.
const BASE: &str = r##"[
  { "type": "theme", "id": "plain",
    "colors": { "text": "#c8c8c8", "background": "#101010", "blood": "#b02020" },
    "styles": { "title": { "fg": "blood", "bold": true } } },
  { "type": "theme", "id": "night",
    "colors": { "text": "#8090c0", "background": "#000010", "blood": "#602020" },
    "styles": { "title": { "fg": "blood", "underline": true } } },
  { "type": "keymap", "id": "keys",
    "menu": { "up": "prev", "down": "next", "left": "left", "right": "right",
              "enter": "accept", "space": "toggle", "escape": "back",
              "tab": "focus_next", "shift+tab": "focus_prev", "backspace": "erase",
              "f2": "more", "f3": "other" } },
  { "type": "preference", "id": "theme", "kind": "text", "default": "base:plain",
    "name": "Theme", "doc": "The theme screens are drawn with." },
  { "type": "preference", "id": "keymap", "kind": "text", "default": "base:keys",
    "name": "Keys", "doc": "What each key means." },
  { "type": "screen", "id": "confirm",
    "params": { "text": "string", "yes": "string?", "no": "string?" },
    "popup": { "width": 24, "place": "centre", "dim": true },
    "layout": { "frame": true, "rows": [
      { "text": "{$text}" },
      { "cols": [
        { "button": { "id": "yes", "label": "{$yes ?? 'Yes'}", "on_activate": [ { "answer": true } ] } },
        { "button": { "id": "no", "label": "{$no ?? 'No'}", "on_activate": [ { "answer": false } ] } } ] } ] },
    "keys": { "back": [ { "answer": false } ] } },
  { "type": "screen", "id": "pick_list",
    "params": { "options": "list<any>", "title": "string?" },
    "popup": { "width": 24, "place": "centre" },
    "layout": { "frame": "{$title ?? 'Choose'}",
                "list": { "id": "options", "rows": "$options", "item": "{it.label}",
                          "on_activate": [ { "answer": "it.value" } ] } },
    "keys": { "back": [ { "close": {} } ] } }
]"##;

fn types() -> DefTypes {
    let mut types = DefTypes::new();
    clash_data::expr::register_defs(&mut types);
    register_defs(&mut types);
    types
}

/// Loads `base` and then the mods given as `(id, text of data/ui.json)`,
/// against the fake host's registry.
fn try_load(mods: &[(&str, &str)]) -> Result<Arc<Defs>, Diagnostics> {
    let types = types();
    let registry = clash_scene::testing::registry();
    let mut loader = Loader::new(&types).expr_registry(&registry).memory_mod(
        "base",
        r#"{ "id": "base" }"#,
        &[("data/ui.json", BASE)],
    );
    let mut ids = vec!["base"];
    for (id, text) in mods {
        let manifest = format!(r#"{{ "id": "{id}" }}"#);
        loader = loader.memory_mod(id, &manifest, &[("data/ui.json", text)]);
        ids.push(id);
    }
    let defs = loader.load_mods(&ids).into_result()?;
    let found = check_defs(&defs);
    if found.has_errors() {
        return Err(found);
    }
    Ok(defs)
}

fn load(mods: &[(&str, &str)]) -> UiDefs {
    match try_load(mods) {
        Ok(defs) => UiDefs::from_defs(&defs).unwrap(),
        Err(diagnostics) => panic!("the mods do not load:\n{}", diagnostics.render()),
    }
}

/// What is wrong with mods that should not load, as rendered text.
fn problems(mods: &[(&str, &str)]) -> String {
    match try_load(mods) {
        Ok(_) => panic!("the mods should not load"),
        Err(diagnostics) => diagnostics.render(),
    }
}

/// A scene over the fake host and application with one screen open, and
/// handles on both.
struct Stage {
    scene: Scene<FakeHost>,
    host: FakeHost,
    app: FakeApp,
    frame: Frame,
}

impl Stage {
    fn new(ui: &UiDefs, screen: &str) -> Stage {
        Stage::with(ui, screen, BTreeMap::new())
    }

    fn with(ui: &UiDefs, screen: &str, params: BTreeMap<String, ExprValue>) -> Stage {
        let host = FakeHost::new();
        let app = FakeApp::new();
        let mut scene = ui
            .scene_over(host.clone(), "base:plain", "base:keys")
            .unwrap_or_else(|e| panic!("{}", e.render()))
            .with_app(Box::new(app.clone()));
        scene.set_role("confirm", "base:confirm");
        scene.set_role("picker.list", "base:pick_list");
        scene.open_screen(screen, params).unwrap();
        Stage {
            scene,
            host,
            app,
            frame: Frame::new(40, 10),
        }
    }

    fn draw(&mut self) -> &Frame {
        self.scene.build_frame(&mut self.frame);
        &self.frame
    }

    fn rows(&mut self) -> Vec<String> {
        let frame = self.draw();
        (0..frame.rows)
            .map(|row| frame.row_text(row).trim_end().to_owned())
            .collect()
    }

    fn press(&mut self, keys: &[&str]) -> Vec<UiOutcome> {
        let mut out = Vec::new();
        for key in keys {
            // Lay the screen out, as a frame between two key presses does.
            self.draw();
            out.extend(self.scene.handle(&InputEvent::key(key).unwrap()));
        }
        out
    }
}

fn screen(body: &str) -> String {
    format!(r#"[ {{ "type": "screen", "id": "s", {body} }} ]"#)
}

// ---------------------------------------------------------------------
// Binding
// ---------------------------------------------------------------------

#[test]
fn bound_widgets_show_what_the_host_says() {
    let ui = load(&[(
        "demo",
        &screen(
            r#""vars": { "shown": 2 },
               "layout": { "rows": [
                 { "text": "{view.name} carries {len(view.items)} things." },
                 { "width": 16, "bar": { "label": "Blood", "value": "view.pool('blood').value",
                                         "max": "view.pool('blood').max" } },
                 { "gauge": { "value": "view.pool('blood').value", "max": "view.pool('blood').max" } },
                 { "height": 2, "list": { "id": "items", "rows": "view.items",
                                          "item": "{it.name}, {it.weight:.1} kg" } },
                 { "if": "screen.shown > 5", "text": "hidden" },
                 { "text": "{len(party)} in the party, {party[1].name} second." } ] }"#,
        ),
    )]);
    let mut stage = Stage::new(&ui, "demo:s");
    assert_eq!(
        stage.rows()[..6],
        [
            "Mara carries 2 things.",
            "Blood ████████░░",
            "4/5",
            "Lantern, 1.5 kg",
            "Coil of rope, 2.0 kg",
            "2 in the party, Jonas second.",
        ]
    );
    assert_eq!(stage.scene.binding_warnings(), 0);

    // Nothing is evaluated again until the host says the simulation has
    // advanced.
    let blood = clash_expr::Handle::new(stage.host.registry().unwrap().kind_id("pool").unwrap(), 1);
    stage
        .host
        .set(blood, "value", ExprValue::Real(clash_core::Real::of(1.0)));
    assert_eq!(stage.rows()[2], "4/5", "not told yet");
    stage.scene.simulation_advanced();
    assert_eq!(stage.rows()[1], "Blood ██░░░░░░░░");
    assert_eq!(stage.rows()[2], "1/5");
}

#[test]
fn a_screen_variable_that_changes_is_seen_by_what_reads_it() {
    let ui = load(&[(
        "demo",
        &screen(
            r#""vars": { "tab": 0, "note": "none" },
               "layout": { "rows": [
                 { "text": "Tab {screen.tab + 1} of 3: {screen.note}" },
                 { "if": "screen.tab == 1", "text": "the second" } ] },
               "keys": { "more": [ { "set": { "screen.tab": "(screen.tab + 1) % 3",
                                              "screen.note": "'turned'" } } ] }"#,
        ),
    )]);
    let mut stage = Stage::new(&ui, "demo:s");
    assert_eq!(stage.rows()[0], "Tab 1 of 3: none");
    let out = stage.press(&["f2"]);
    assert_eq!(
        out,
        [
            UiOutcome::Changed(VarRef::Screen("note".into())),
            UiOutcome::Changed(VarRef::Screen("tab".into())),
        ]
    );
    assert_eq!(stage.rows()[..2], ["Tab 2 of 3: turned", "the second"]);
    stage.press(&["f2", "f2"]);
    assert_eq!(stage.rows()[..2], ["Tab 1 of 3: turned", ""]);
    assert_eq!(stage.scene.top().unwrap().var("tab"), Some(&Value::Int(0)));
}

#[test]
fn only_what_is_on_show_is_evaluated() {
    let ui = load(&[(
        "demo",
        &screen(
            r#""vars": { "page": 0, "open": false },
               "layout": { "rows": [
                 { "id": "pages", "tabs": { "selected": "page", "pages": [
                   { "title": "One", "content": { "text": "fine" } },
                   { "title": "Two", "content": { "gauge": { "value": "1 / (screen.page - 1)" } } } ] } },
                 { "if": "screen.open", "rows": [ { "gauge": { "value": "1 / (screen.page - 1)" } } ] } ] }"#,
        ),
    )]);
    let mut stage = Stage::new(&ui, "demo:s");
    stage.draw();
    // Both divisions would fault at page 1: neither is on show.
    assert_eq!(
        stage.scene.binding_warnings(),
        0,
        "a page that is not shown and a node whose `if` fails are not evaluated"
    );
    stage.press(&["right"]);
    assert_eq!(stage.scene.top().unwrap().var("page"), Some(&Value::Int(1)));
    stage.draw();
    assert_eq!(stage.scene.binding_warnings(), 1, "the shown page is");
}

#[test]
fn app_is_answered_by_the_scene_and_the_application() {
    let ui = load(&[(
        "demo",
        r#"[ { "type": "preference", "id": "numbers", "kind": "flag", "default": false,
               "name": "Show damage numbers", "doc": "Float the energy of each hit." },
             { "type": "screen", "id": "s",
               "layout": { "rows": [
                 { "text": "{app.size[0]} by {app.size[1]}" },
                 { "text": "{app.profile.name} / {app.scenario.name}" },
                 { "text": "save {app.can_save} load {app.can_load}" },
                 { "text": "numbers {app.pref('numbers')} theme {app.pref('base:theme')}" },
                 { "height": 2, "list": { "rows": "app.saves", "item": "{it.name}: {it.description}" } },
                 { "text": "{len(app.mods)} mods, {len(app.scenarios)} scenarios, {len(app.profiles)} profiles" },
                 { "text": "{app.mods[2].id} is {app.mods[2].state}" } ] } } ]"#,
    )]);
    let mut stage = Stage::new(&ui, "demo:s");
    assert_eq!(
        stage.rows()[..8],
        [
            "40 by 10",
            "CLASH2D / A walk",
            "save true load true",
            "numbers false theme base:plain",
            "Autumn: Day 12, the docks.",
            "First try: Day 2, the gate.",
            "3 mods, 2 scenarios, 2 profiles",
            "ashlands is disabled",
        ]
    );
    // A preference defined by a mod is readable as `app.pref` and
    // changes when it is set.
    stage.app.allow(false, true);
    stage
        .scene
        .set_pref("demo:numbers", Value::Bool(true))
        .unwrap();
    assert_eq!(stage.rows()[2], "save false load true");
    assert_eq!(stage.rows()[3], "numbers true theme base:plain");
    assert_eq!(
        stage.app.requests().last().unwrap().service,
        AppService::SetPref,
        "the application is told, so that it can keep the choice"
    );
    // The grid's size follows the frame.
    stage.frame = Frame::new(50, 12);
    assert_eq!(stage.rows()[0], "50 by 12");
}

// ---------------------------------------------------------------------
// What a screen may know
// ---------------------------------------------------------------------

#[test]
fn reading_truth_about_someone_else_is_a_load_error_outside_debug_screens() {
    let body = r#""params": { "who": "entity" },
                  "layout": { "bar": { "value": "$who.health" } }"#;
    // In a debug screen the binding loads.
    let debug = format!(r#""context": "debug", {body}"#);
    let ui = load(&[("demo", &screen(&debug))]);
    let stranger = ExprValue::Subject(FakeHost::new().entity(3));
    let mut stage = Stage::with(
        &ui,
        "demo:s",
        BTreeMap::from([("who".to_owned(), stranger)]),
    );
    assert_eq!(
        stage.rows()[0],
        format!("{}{}", "█".repeat(10), "░".repeat(30)),
        "and shows the truth: a quarter"
    );

    // In an ordinary screen the same binding is a load error, at the
    // place inside the string.
    let text = problems(&[("demo", &screen(body))]);
    assert!(
        text.contains("`entity.health` is not safe for belief"),
        "{text}"
    );
    assert!(text.contains("error[E0662]"), "{text}");
    assert!(text.contains("--> mods/demo/data/ui.json:2:"), "{text}");
    assert!(text.contains("$who.health"), "{text}");
    assert!(
        text.contains("= in: screen demo:s, at layout.bar.value"),
        "{text}"
    );
}

#[test]
fn the_party_is_read_in_full_and_others_as_perceived() {
    // Everything here is the party's own, or safe for belief.
    let fine = r#""params": { "who": "entity", "member": "party_member" },
                  "layout": { "rows": [
                    { "bar": { "value": "view.health" } },
                    { "bar": { "value": "party[0].health" } },
                    { "bar": { "value": "$member.health" } },
                    { "gauge": { "value": "view.pool('blood').frac" } },
                    { "text": "{$who.name} is seen by {view.name}" },
                    { "list": { "rows": "party", "item": "{it.name}: {it.health}" } },
                    { "list": { "rows": "view.items", "item": "{it.name}: {it.weight}" } },
                    { "list": { "rows": "view.seen", "item": "{it.name}" } } ] }"#;
    load(&[("demo", &screen(fine))]);

    // Each of these reads what the party has not perceived.
    for (binding, what) in [
        (
            r#"{ "bar": { "value": "$who.health" } }"#,
            "`entity.health` is not safe for belief",
        ),
        (
            r#"{ "list": { "rows": "view.seen", "item": "{it.health}" } }"#,
            "`entity.health` is not safe for belief",
        ),
        (
            r#"{ "list": { "rows": "$who.items", "item": "{it.name}" } }"#,
            "`entity.items` is not safe for belief",
        ),
        (
            r#"{ "list": { "rows": "view.seen", "item": "{it.name}",
                           "on_activate": [ { "set": { "screen.n": "it.health" } } ] } }"#,
            "`entity.health` is not safe for belief",
        ),
    ] {
        let body = format!(
            r#""params": {{ "who": "entity" }}, "vars": {{ "n": 0 }}, "layout": {binding}"#
        );
        let text = problems(&[("demo", &screen(&body))]);
        assert!(text.contains(what), "{binding}:\n{text}");
        // The same loads where truth may be seen.
        let debug = format!(r#""context": "debug", {body}"#);
        load(&[("demo", &screen(&debug))]);
    }
}

#[test]
fn a_binding_that_is_wrong_is_reported_inside_its_string() {
    let text = problems(&[(
        "demo",
        r#"[ { "type": "screen", "id": "s",
  "vars": { "tab": 0 },
  "layout": { "rows": [
    { "text": "{vieww.name}" },
    { "bar": { "value": "view.helth" } },
    { "if": "view.name", "text": "x" },
    { "text": "{screen.tabb}" },
    { "text": "{$nobody}" },
    { "gauge": { "value": "rand(0, 1)" } } ] },
  "keys": { "more": [ { "set": { "screen.tab": "screen.tab +" } } ] } } ]"#,
    )]);
    for (needle, line) in [
        ("unknown name `vieww`", 4),
        ("`entity` has no property `helth`", 5),
        ("did you mean `health`?", 5),
        ("the screen has no variable `tabb`", 7),
        ("did you mean `screen.tab`?", 7),
        ("unknown local `$nobody`", 8),
        ("rand", 9),
    ] {
        assert!(text.contains(needle), "missing `{needle}` in:\n{text}");
        assert!(
            text.contains(&format!("--> mods/demo/data/ui.json:{line}:")),
            "nothing at line {line} in:\n{text}"
        );
    }
    // A condition must be a boolean, and a broken expression in an
    // effect is found too.
    assert!(text.contains("ui.json:6:"), "{text}");
    assert!(text.contains("ui.json:10:"), "{text}");
    assert!(text.contains("at keys.more"), "{text}");
    // The caret is under the name, not under the whole string.
    assert!(
        text.contains("\n  |                 ^^^^^\n") || text.contains("^^^^^ "),
        "{text}"
    );
}

// ---------------------------------------------------------------------
// Effects
// ---------------------------------------------------------------------

#[test]
fn a_run_effect_reaches_submit_as_a_whole_command() {
    let ui = load(&[(
        "demo",
        &screen(
            r#""vars": { "times": 3 },
               "layout": { "list": { "id": "who", "rows": "party", "item": "{it.name}",
                 "on_activate": [
                   { "run": { "rule": "demo:ring_bell",
                              "args": { "by": "it", "times": "screen.times * 2", "loud": true } } },
                   { "act": { "action": "step", "params": { "dir": "'west'" } } },
                   { "act": { "action": "attack", "params": "needed" } },
                   { "command": "wait" } ] } }"#,
        ),
    )]);
    let run = || {
        let mut stage = Stage::new(&ui, "demo:s");
        let out = stage.press(&["down", "enter"]);
        (out, stage.host.commands())
    };
    let (out, commands) = run();
    let jonas = ExprValue::Subject(FakeHost::new().entity(2));
    let expected = [
        UiCommand {
            kind: CommandKind::Rule,
            name: "demo:ring_bell".into(),
            args: BTreeMap::from([
                ("by".to_owned(), jonas),
                ("loud".to_owned(), ExprValue::Bool(true)),
                ("times".to_owned(), ExprValue::Int(6)),
            ]),
            collect: Collect::Given,
        },
        UiCommand {
            kind: CommandKind::Act,
            name: "step".into(),
            args: BTreeMap::from([("dir".to_owned(), ExprValue::text("west"))]),
            collect: Collect::Given,
        },
        UiCommand {
            kind: CommandKind::Act,
            name: "attack".into(),
            args: BTreeMap::new(),
            collect: Collect::Needed,
        },
        UiCommand {
            kind: CommandKind::Command,
            name: "wait".into(),
            args: BTreeMap::new(),
            collect: Collect::Given,
        },
    ];
    // What the host was handed is the whole command, and what the
    // scene reports is what the host was handed.
    assert_eq!(commands, expected);
    let submitted: Vec<UiCommand> = out
        .iter()
        .filter_map(|outcome| match outcome {
            UiOutcome::Submitted(command) => Some(command.clone()),
            _ => None,
        })
        .collect();
    assert_eq!(submitted, expected);
    assert_eq!(
        commands[0].to_string(),
        "run demo:ring_bell by=entity#2 loud=true times=6".replace(
            "entity#2",
            &ExprValue::Subject(FakeHost::new().entity(2)).to_string()
        )
    );
    // The same input gives the same commands, every time.
    assert_eq!(run(), run());
}

#[test]
fn a_command_the_host_refuses_comes_back_with_its_reason() {
    let ui = load(&[(
        "demo",
        &screen(
            r#""layout": { "button": { "label": "Ring", "on_activate": [
                 { "run": "demo:ring_bell" }, { "command": "wait" } ] } }"#,
        ),
    )]);
    let mut stage = Stage::new(&ui, "demo:s");
    stage.host.refuse("demo:ring_bell");
    let out = stage.press(&["enter"]);
    assert!(
        matches!(&out[0], UiOutcome::Refused { command, reason }
            if command.name == "demo:ring_bell" && reason == "the fake host refuses `demo:ring_bell`"),
        "{out:?}"
    );
    assert!(matches!(&out[1], UiOutcome::Submitted(command) if command.name == "wait"));
    assert_eq!(stage.host.commands().len(), 1, "only what was accepted");
}

#[test]
fn an_effect_that_is_wrong_is_a_load_error_where_it_is_written() {
    let text = problems(&[(
        "demo",
        r#"[
  { "type": "screen", "id": "a",
    "layout": { "button": { "label": "a", "on_activate": [ { "cloze": {} } ] } } },
  { "type": "screen", "id": "b",
    "layout": { "button": { "label": "b", "on_activate": [ { "app": "explode" } ] } } },
  { "type": "screen", "id": "c",
    "layout": { "button": { "label": "c", "on_activate": [ { "app": { "load": {} } } ] } } },
  { "type": "screen", "id": "d",
    "keys": { "back": [ { "close": { "now": true } } ] }, "layout": { "spacer": {} } } ]"#,
    )]);
    for (needle, line) in [
        ("unknown effect `cloze`", 3),
        ("unknown service `explode`", 5),
        ("`load` needs the argument `save`", 7),
        ("now", 9),
    ] {
        assert!(text.contains(needle), "missing `{needle}` in:\n{text}");
        assert!(
            text.contains(&format!("--> mods/demo/data/ui.json:{line}:")),
            "nothing at line {line} in:\n{text}"
        );
    }
    // What the loader could not read hides the rest; mended, the rest
    // shows.
    let text = problems(&[(
        "demo",
        r#"[ { "type": "screen", "id": "s",
  "vars": { "n": 0 },
  "layout": { "rows": [
    { "button": { "label": "d", "on_activate": [ { "set": { "screen.m": 1 } } ] } },
    { "button": { "label": "e", "on_activate": [ { "set": { "screen.n": "view.nme" } } ] } } ] } } ]"#,
    )]);
    assert!(
        text.contains("the screen has no variable `m` to set"),
        "{text}"
    );
    assert!(text.contains("`entity` has no property `nme`"), "{text}");
    assert!(text.contains("ui.json:5:"), "{text}");
}

// ---------------------------------------------------------------------
// The screen stack
// ---------------------------------------------------------------------

const STACK: &str = r#"[
  { "type": "screen", "id": "main",
    "vars": { "name": "Mara", "chosen": "nobody", "count": 0 },
    "layout": { "rows": [
      { "text": "Main: {screen.name}, chosen {screen.chosen}, count {screen.count}" },
      { "input": { "id": "name", "bind": "name" } },
      { "button": { "id": "open", "label": "Open",
                    "on_activate": [ { "open": { "screen": "detail", "with": { "who": "party[1]", "n": "screen.count + 1" } } } ] } },
      { "button": { "id": "quit", "label": "Quit",
                    "on_activate": [ { "confirm": { "text": "Leave {screen.name}?", "yes": "Leave" } },
                                     { "set": { "screen.count": "screen.count + 10" } },
                                     { "app": "quit" } ] } },
      { "button": { "id": "pick", "label": "Pick",
                    "on_activate": [ { "pick": { "from": "party", "item": "{it.name}", "title": "Who?" } },
                                     { "set": { "screen.chosen": "it.name" } },
                                     { "run": { "rule": "demo:choose", "args": { "who": "it" } } } ] } },
      { "button": { "id": "save", "label": "Save",
                    "on_activate": [ { "app": { "save": { "name": "screen.name" } } },
                                     { "app": { "delete_save": { "save": "'first'" } } },
                                     { "app": { "load": { "save": "'nothing'" } } },
                                     { "focus": "name" } ] } } ] },
    "keys": { "more": [ { "replace": "other" } ],
              "other": [ { "focus": "nowhere" }, { "open": "nowhere" } ] } },
  { "type": "screen", "id": "detail",
    "params": { "who": "party_member", "n": "int" },
    "popup": { "width": 20, "height": 4, "place": "bottom_right" },
    "layout": { "frame": "Detail", "rows": [
      { "text": "{$who.name} #{$n}" },
      { "text": "health {$who.health}" } ] },
    "keys": { "back": [ { "close": {} } ] } },
  { "type": "screen", "id": "other",
    "layout": { "text": "Other" },
    "keys": { "back": [ { "close": {} } ] } }
]"#;

#[test]
fn close_returns_to_the_screen_beneath_as_it_was_left() {
    let ui = load(&[("demo", STACK)]);
    let mut stage = Stage::new(&ui, "demo:main");
    // Leave some state behind: a typed name and the focus on a button.
    stage.press(&["x", "tab"]);
    assert_eq!(stage.scene.top().unwrap().focus(), Some("open"));
    assert_eq!(stage.rows()[0], "Main: Marax, chosen nobody, count 0");

    let out = stage.press(&["enter"]);
    assert_eq!(out, [UiOutcome::Opened("demo:detail".into())]);
    assert_eq!(stage.scene.screens().len(), 2);
    // The popup is drawn over the screen beneath, which is still shown.
    let rows = stage.rows();
    assert_eq!(rows[0], "Main: Marax, chosen nobody, count 0");
    assert_eq!(rows[6], "                    ┌ Detail ──────────┐");
    assert_eq!(rows[7], "                    │Jonas #1          │");
    assert_eq!(rows[8], "                    │health 0.5        │");
    assert_eq!(rows[9], "                    └──────────────────┘");
    // Input goes to the top screen: the popup has no `more` key.
    assert!(stage.press(&["f2"]).is_empty());

    let out = stage.press(&["escape"]);
    assert_eq!(out, [UiOutcome::Closed("demo:detail".into())]);
    let main = stage.scene.top().unwrap();
    assert_eq!(main.id(), "demo:main");
    assert_eq!(main.focus(), Some("open"), "the focus is where it was");
    assert_eq!(main.var("name"), Some(&Value::Str("Marax".into())));
    assert_eq!(stage.rows()[9], "", "and the popup is gone");

    // `replace` swaps the top screen; closing that leaves nothing.
    let out = stage.press(&["f2"]);
    assert_eq!(
        out,
        [
            UiOutcome::Closed("demo:main".into()),
            UiOutcome::Opened("demo:other".into())
        ]
    );
    assert_eq!(stage.scene.screens().len(), 1);
    assert_eq!(stage.rows()[0], "Other");
    stage.press(&["escape"]);
    assert!(stage.scene.top().is_none());
    assert_eq!(stage.rows()[0], "");
}

#[test]
fn confirm_asks_before_the_rest_of_the_list_runs() {
    let ui = load(&[("demo", STACK)]);
    let mut stage = Stage::new(&ui, "demo:main");
    stage.draw();
    assert!(stage.scene.top_mut().unwrap().set_focus("quit"));
    let out = stage.press(&["enter"]);
    assert_eq!(out, [UiOutcome::Opened("base:confirm".into())]);
    // The popup is in the middle, over the screen that asked.
    let rows = stage.rows();
    assert_eq!(rows[3], "[ Quit ]┌──────────────────────┐");
    assert_eq!(rows[4], "[ Pick ]│Leave Mara?           │");
    assert_eq!(rows[5], "[ Save ]│[ Leave ][ No ]       │");
    assert_eq!(rows[6], "        └──────────────────────┘");
    // What is beneath is dimmed: darker, and dark grey on a terminal.
    let frame = stage.draw().clone();
    let beneath = frame.get(0, 0).unwrap();
    let popup = frame.get(9, 4).unwrap();
    assert!(beneath.fg.rgb.r < popup.fg.rgb.r, "{beneath:?} {popup:?}");
    assert_eq!(beneath.fg.fallback, clash_scene::Ansi16::BrightBlack);

    // No: the popup closes and nothing else happens.
    let out = stage.press(&["right", "enter"]);
    assert_eq!(out, [UiOutcome::Closed("base:confirm".into())]);
    assert_eq!(
        stage.scene.top().unwrap().var("count"),
        Some(&Value::Int(0))
    );
    assert!(!stage.scene.quit_requested());
    assert!(stage.app.requests().is_empty());

    // Yes: the rest of the list runs, in the screen that asked.
    stage.press(&["enter"]);
    let out = stage.press(&["enter"]);
    assert_eq!(out.len(), 3, "{out:?}");
    assert_eq!(out[0], UiOutcome::Closed("base:confirm".into()));
    assert_eq!(out[1], UiOutcome::Changed(VarRef::Screen("count".into())));
    assert!(matches!(&out[2], UiOutcome::Service(request) if request.service == AppService::Quit));
    assert_eq!(
        stage.scene.top().unwrap().var("count"),
        Some(&Value::Int(10))
    );
    assert!(stage.scene.quit_requested());
    assert_eq!(stage.app.requests().len(), 1);

    // Escape answers no.
    stage.press(&["enter"]);
    assert_eq!(stage.scene.screens().len(), 2);
    let out = stage.press(&["escape"]);
    assert_eq!(out, [UiOutcome::Closed("base:confirm".into())]);
    assert_eq!(
        stage.scene.top().unwrap().var("count"),
        Some(&Value::Int(10))
    );
}

#[test]
fn pick_opens_a_picker_and_goes_on_with_the_choice() {
    let ui = load(&[("demo", STACK)]);
    let mut stage = Stage::new(&ui, "demo:main");
    stage.draw();
    assert!(stage.scene.top_mut().unwrap().set_focus("pick"));
    let out = stage.press(&["enter"]);
    assert_eq!(out, [UiOutcome::Opened("base:pick_list".into())]);
    let rows = stage.rows();
    assert_eq!(rows[3], "[ Quit ]┌ Who? ────────────────┐");
    assert_eq!(rows[4], "[ Pick ]│Mara                  │");
    assert_eq!(rows[5], "[ Save ]│Jonas                 │");

    let out = stage.press(&["down", "enter"]);
    let jonas = ExprValue::Subject(FakeHost::new().entity(2));
    assert_eq!(out.len(), 3, "{out:?}");
    assert_eq!(out[0], UiOutcome::Closed("base:pick_list".into()));
    assert_eq!(out[1], UiOutcome::Changed(VarRef::Screen("chosen".into())));
    assert!(
        matches!(&out[2], UiOutcome::Submitted(command)
            if command.name == "demo:choose" && command.args["who"] == jonas),
        "{out:?}"
    );
    assert_eq!(
        stage.rows()[0],
        "Main: Mara, chosen Jonas, count 0",
        "the rest of the list ran with the choice as `it`"
    );

    // Leaving the picker without choosing drops what was waiting.
    stage.press(&["enter"]);
    let out = stage.press(&["escape"]);
    assert_eq!(out, [UiOutcome::Closed("base:pick_list".into())]);
    assert_eq!(stage.host.commands().len(), 1);
}

#[test]
fn app_services_reach_the_application_and_failures_say_why() {
    let ui = load(&[("demo", STACK)]);
    let mut stage = Stage::new(&ui, "demo:main");
    stage.draw();
    assert!(stage.scene.top_mut().unwrap().set_focus("save"));
    let out = stage.press(&["enter"]);
    assert_eq!(out.len(), 3, "{out:?}");
    assert!(matches!(&out[0], UiOutcome::Service(request)
            if request.service == AppService::Save && request.text("name") == Some("Mara")));
    assert!(
        matches!(&out[1], UiOutcome::Service(request) if request.service == AppService::DeleteSave)
    );
    assert_eq!(
        out[2],
        UiOutcome::Failed {
            effect: "app",
            reason: "there is no save `nothing`".into()
        }
    );
    // `focus` moved the focus, and reports nothing when it works.
    assert_eq!(stage.scene.top().unwrap().focus(), Some("name"));
    let saves: Vec<String> = stage
        .app
        .requests()
        .iter()
        .map(|request| request.service.name().to_owned())
        .collect();
    assert_eq!(saves, ["save", "delete_save"]);

    // An effect that cannot be run says why and the list goes on.
    let out = stage.press(&["f3"]);
    assert_eq!(
        out,
        [
            UiOutcome::Failed {
                effect: "focus",
                reason: "no widget `nowhere` on show can take the focus".into()
            },
            UiOutcome::Failed {
                effect: "open",
                reason: "there is no screen `nowhere`".into()
            }
        ]
    );
}

// ---------------------------------------------------------------------
// Preferences and themes
// ---------------------------------------------------------------------

const OPTIONS: &str = r#"[
  { "type": "preference", "id": "numbers", "kind": "flag", "default": false, "name": "Numbers" },
  { "type": "preference", "id": "speed", "kind": "number", "default": 1, "min": 0, "max": 4, "name": "Speed" },
  { "type": "preference", "id": "cutscenes", "kind": "choice", "default": "all",
    "options": ["all", "major", "none"], "name": "Cutscenes" },
  { "type": "screen", "id": "options",
    "layout": { "rows": [
      { "text": "<title>Options</> speed {app.pref('speed')}" },
      { "toggle": { "id": "numbers", "bind": "pref.numbers", "label": "Numbers" } },
      { "number": { "id": "speed", "bind": "pref.speed", "min": 0, "max": 4 } },
      { "choice": { "id": "cutscenes", "bind": "pref.cutscenes", "options": ["all", "major", "none"] } },
      { "choice": { "id": "theme", "bind": "pref.theme", "options": ["base:plain", "base:night"] } },
      { "button": { "id": "broken", "label": "Break",
                    "on_activate": [ { "set": { "pref.theme": "'base:nowhere'" } },
                                     { "set": { "pref.speed": 9 } },
                                     { "app": { "set_pref": { "pref": "'demo:cutscenes'", "value": "'none'" } } },
                                     { "app": { "rebind": { "context": "'menu'", "key": "'f5'", "name": "'more'" } } } ] } } ] },
    "keys": { "more": [ { "open": "over" } ] } },
  { "type": "screen", "id": "over",
    "popup": { "width": 12, "height": 3, "place": "top_right" },
    "layout": { "frame": "Over", "text": "<title>popup</>" },
    "keys": { "back": [ { "close": {} } ] } }
]"#;

#[test]
fn preferences_are_kept_by_the_scene_and_edited_by_widgets() {
    let ui = load(&[("demo", OPTIONS)]);
    let mut stage = Stage::new(&ui, "demo:options");
    assert_eq!(stage.scene.pref("demo:numbers"), Some(&Value::Bool(false)));
    assert_eq!(stage.scene.pref("demo:speed"), Some(&Value::Int(1)));
    assert_eq!(
        stage.scene.pref("base:theme"),
        Some(&Value::Str("base:plain".into()))
    );
    assert_eq!(
        stage.rows()[..4],
        ["Options speed 1", "[ ] Numbers", "< 1 >", "< all >"]
    );

    // A widget bound to a preference changes it in the scene.
    let out = stage.press(&["space", "tab", "right", "right", "tab", "right"]);
    assert_eq!(
        out,
        [
            UiOutcome::Changed(VarRef::Pref("demo:numbers".into())),
            UiOutcome::Changed(VarRef::Pref("demo:speed".into())),
            UiOutcome::Changed(VarRef::Pref("demo:speed".into())),
            UiOutcome::Changed(VarRef::Pref("demo:cutscenes".into())),
        ]
    );
    assert_eq!(stage.scene.pref("demo:numbers"), Some(&Value::Bool(true)));
    assert_eq!(stage.scene.pref("demo:speed"), Some(&Value::Int(3)));
    assert_eq!(
        stage.scene.pref("demo:cutscenes"),
        Some(&Value::Str("major".into()))
    );
    assert_eq!(
        stage.rows()[..4],
        ["Options speed 3", "[x] Numbers", "< 3 >", "< major >"]
    );

    // What a preference cannot hold is refused, with the reason, and
    // the preference keeps its value.
    assert!(stage.scene.top_mut().unwrap().set_focus("broken"));
    let out = stage.press(&["enter"]);
    assert_eq!(out.len(), 4, "{out:?}");
    assert!(
        matches!(&out[0], UiOutcome::Failed { effect: "set", reason }
            if reason.contains("there is no theme `base:nowhere`")),
        "{out:?}"
    );
    assert!(
        matches!(&out[1], UiOutcome::Failed { effect: "set", reason }
            if reason == "the preference `demo:speed` cannot be 9: 9 is above the most it may be"),
        "{out:?}"
    );
    assert!(
        matches!(&out[2], UiOutcome::Changed(VarRef::Pref(id)) if id == "demo:cutscenes")
            || matches!(&out[2], UiOutcome::Service(_)),
        "{out:?}"
    );
    assert!(
        matches!(&out[3], UiOutcome::Service(request) if request.service == AppService::Rebind)
    );
    assert_eq!(
        stage.scene.pref("base:theme"),
        Some(&Value::Str("base:plain".into()))
    );
    assert_eq!(stage.scene.pref("demo:speed"), Some(&Value::Int(3)));
    assert_eq!(
        stage.scene.pref("demo:cutscenes"),
        Some(&Value::Str("none".into()))
    );
    assert_eq!(
        stage.rows()[3],
        "< none >",
        "the widget shows the new value"
    );
    assert_eq!(
        stage.scene.set_pref("demo:numbers", Value::Int(1)),
        Err("the preference `demo:numbers` cannot be 1: it holds true or false, not int".into())
    );
    assert!(stage.scene.set_pref("demo:nothing", Value::Int(1)).is_err());

    // A key the player rebound means what it was bound to.
    let out = stage.press(&["f5"]);
    assert_eq!(out, [UiOutcome::Opened("demo:over".into())]);
}

#[test]
fn switching_theme_restyles_every_open_screen() {
    let ui = load(&[("demo", OPTIONS)]);
    let mut stage = Stage::new(&ui, "demo:options");
    // Two screens open, the popup over the options, and some state.
    stage.press(&["tab", "right", "f2"]);
    assert_eq!(stage.scene.screens().len(), 2);
    let before = stage.draw().clone();
    let plain = ui.theme("base:plain").unwrap();
    let night = ui.theme("base:night").unwrap();
    let title = |frame: &Frame, col: i32, row: i32| frame.get(col, row).unwrap().fg;
    assert_eq!(title(&before, 0, 0), plain.color("blood").unwrap());
    assert_eq!(title(&before, 29, 1), plain.color("blood").unwrap());

    stage
        .scene
        .set_pref("base:theme", Value::Str("base:night".into()))
        .unwrap();
    let after = stage.draw().clone();
    for row in (0..10).filter(|row| *row != 4) {
        assert_eq!(
            after.row_text(row),
            before.row_text(row),
            "the same characters"
        );
    }
    assert_eq!(after.row_text(4).trim_end(), "< base:night >");
    // Both screens are drawn in the new theme: text, background and the
    // `title` style of each.
    assert_eq!(title(&after, 0, 0), night.color("blood").unwrap());
    assert_eq!(title(&after, 29, 1), night.color("blood").unwrap());
    assert_eq!(
        after.get(0, 5).unwrap().bg,
        night.color("background").unwrap()
    );
    assert_eq!(after.get(12, 0).unwrap().fg, night.color("text").unwrap());
    assert!(
        after
            .get(0, 0)
            .unwrap()
            .attrs
            .contains(clash_scene::Attrs::UNDERLINE)
    );
    // What the screens held is kept: the popup is still on top, and the
    // one beneath has its focus and its values.
    assert_eq!(stage.scene.top().unwrap().id(), "demo:over");
    stage.press(&["escape"]);
    assert_eq!(stage.scene.top().unwrap().focus(), Some("speed"));
    assert_eq!(stage.rows()[2], "< 2 >");

    // A choice bound to the theme preference does the same.
    stage.press(&["tab", "tab", "right"]);
    assert_eq!(
        stage.scene.pref("base:theme"),
        Some(&Value::Str("base:plain".into()))
    );
    assert_eq!(title(stage.draw(), 0, 0), plain.color("blood").unwrap());
    assert_eq!(stage.scene.top().unwrap().focus(), Some("theme"));
}

// ---------------------------------------------------------------------
// Slots
// ---------------------------------------------------------------------

const HUD: &str = r#"[
  { "type": "screen", "id": "hud",
    "vars": { "tab": 0 },
    "layout": { "id": "root", "rows": [
      { "id": "top", "text": "Status" },
      { "slot": "hud.bars" },
      { "id": "bottom", "text": "End" } ] } }
]"#;

const ADDS: &str = r#"[
  { "type": "ui_part", "id": "mana", "into": "hud:hud.bars", "order": 20,
    "widget": { "text": "Mana of {view.name}" } },
  { "type": "ui_part", "id": "armour", "into": "hud:hud.bars", "order": 10,
    "if": "view.health > 0.5",
    "widget": { "id": "armour", "text": "Armour" } },
  { "type": "ui_part", "id": "zeal", "into": "hud:hud.bars", "order": 20,
    "if": "screen.tab == 1",
    "widget": { "if": "len(party) > 1", "text": "Zeal" } },
  { "type": "ui_part", "id": "elsewhere", "into": "hud:map.corner", "required": false,
    "widget": { "text": "Never shown" } }
]"#;

#[test]
fn a_part_from_another_mod_appears_in_a_slot_in_order_while_its_condition_holds() {
    let ui = load(&[("hud", HUD), ("adds", ADDS)]);
    let mut stage = Stage::new(&ui, "hud:hud");
    assert_eq!(
        stage.scene.top().unwrap().slots().collect::<Vec<_>>(),
        ["hud.bars"]
    );
    // By order, and among equals by id; `zeal` waits for its condition.
    assert_eq!(
        stage.rows()[..4],
        ["Status", "Armour", "Mana of Mara", "End"]
    );

    // A part's condition is a binding like any other: it follows the
    // game and the screen's variables.
    let mara = stage.host.entity(1);
    stage
        .host
        .set(mara, "health", ExprValue::Real(clash_core::Real::of(0.25)));
    stage.scene.simulation_advanced();
    assert_eq!(stage.rows()[..3], ["Status", "Mana of Mara", "End"]);
    assert!(stage.scene.top_mut().unwrap().set_var("tab", Value::Int(1)));
    assert_eq!(stage.rows()[..4], ["Status", "Mana of Mara", "Zeal", "End"]);

    // Without the second mod the slot is empty and takes no room.
    let alone = load(&[("hud", HUD)]);
    let mut stage = Stage::new(&alone, "hud:hud");
    assert_eq!(stage.rows()[..2], ["Status", "End"]);
}

#[test]
fn a_part_aimed_at_a_slot_that_is_missing_is_a_load_error_unless_it_need_not_be_there() {
    let text = problems(&[
        ("hud", HUD),
        (
            "adds",
            r#"[
  { "type": "ui_part", "id": "lost", "into": "hud:hud.barz",
    "widget": { "text": "Lost" } },
  { "type": "ui_part", "id": "fine", "into": "hud:nowhere", "required": false,
    "widget": { "text": "Fine" } },
  { "type": "ui_part", "id": "nosy", "into": "hud:hud.bars",
    "widget": { "text": "{view.seen[0].health}" } } ]"#,
        ),
    ]);
    assert!(
        text.contains("no screen places a slot `hud:hud.barz`"),
        "{text}"
    );
    assert!(text.contains("did you mean `hud:hud.bars`?"), "{text}");
    assert!(text.contains("--> mods/adds/data/ui.json:2:"), "{text}");
    assert!(text.contains("= in: ui_part adds:lost, at into"), "{text}");
    assert!(!text.contains("hud:nowhere"), "{text}");
    // A part is held to what an ordinary screen may know.
    assert!(
        text.contains("`entity.health` is not safe for belief"),
        "{text}"
    );
    assert!(
        text.contains("= in: ui_part adds:nosy, at widget.text"),
        "{text}"
    );
}

// ---------------------------------------------------------------------
// Widgets
// ---------------------------------------------------------------------

#[test]
fn a_number_can_be_typed() {
    let ui = load(&[(
        "demo",
        &screen(
            r#""vars": { "n": 5, "r": 0.5, "max": 50 },
               "layout": { "rows": [
                 { "number": { "id": "n", "bind": "n", "min": 0, "max": "screen.max" } },
                 { "number": { "id": "r", "bind": "r" } },
                 { "text": "n is {screen.n}, r is {screen.r}" } ] }"#,
        ),
    )]);
    let mut stage = Stage::new(&ui, "demo:s");
    assert_eq!(stage.rows()[..3], ["< 5 >", "< 0.5 >", "n is 5, r is 0.5"]);

    // Digits are kept until the number is taken; nothing changes yet.
    let out = stage.press(&["4", "2"]);
    assert!(out.is_empty());
    assert_eq!(stage.rows()[0], "< 42_ >");
    assert_eq!(stage.scene.top().unwrap().var("n"), Some(&Value::Int(5)));
    // Erase takes the last digit back; accept takes the number.
    stage.press(&["backspace", "7"]);
    assert_eq!(stage.rows()[0], "< 47_ >");
    let out = stage.press(&["enter"]);
    assert_eq!(out, [UiOutcome::Changed(VarRef::Screen("n".into()))]);
    assert_eq!(
        stage.rows()[..3],
        ["< 47 >", "< 0.5 >", "n is 47, r is 0.5"]
    );

    // A number past its bounds is brought within them, and moving on
    // takes what was typed as well. The bound is itself a binding.
    let out = stage.press(&["9", "9", "tab"]);
    assert_eq!(out, [UiOutcome::Changed(VarRef::Screen("n".into()))]);
    assert_eq!(stage.scene.top().unwrap().var("n"), Some(&Value::Int(50)));
    assert_eq!(stage.scene.top().unwrap().focus(), Some("r"));

    // A whole number takes no point; one that is not whole does, once.
    stage.press(&["-", "1", ".", "2", ".", "5", "enter"]);
    assert_eq!(
        stage.rows()[..3],
        ["< 50 >", "< -1.2 >", "n is 50, r is -1.25"]
    );
    stage.press(&["shift+tab", ".", "-", "3", "enter"]);
    assert_eq!(stage.rows()[0], "< 0 >", "minus three is below the least");
    // A step takes what was typed first, then steps.
    stage.press(&["7", "right"]);
    assert_eq!(stage.rows()[0], "< 8 >");
}
