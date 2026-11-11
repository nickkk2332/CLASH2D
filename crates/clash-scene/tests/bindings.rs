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
    let mut loader = Loader::new(&types)
        .expr_registry(&registry)
        .memory_mod("base", r#"{ "id": "base" }"#, &[("data/ui.json", BASE)]);
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
    let blood = clash_expr::Handle::new(
        stage.host.registry().unwrap().kind_id("pool").unwrap(),
        1,
    );
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
    assert_eq!(
        stage.rows()[..2],
        ["Tab 2 of 3: turned", "the second"]
    );
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
    assert!(text.contains("\n  |                 ^^^^^\n") || text.contains("^^^^^ "), "{text}");
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
        "run demo:ring_bell by=entity#2 loud=true times=6"
            .replace("entity#2", &ExprValue::Subject(FakeHost::new().entity(2)).to_string())
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
