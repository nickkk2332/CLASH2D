//! The bundled screens (12 §10.7; package U9): character creation,
//! the inventory and what can be done with a thing, the condition of a
//! character's body, look mode's side panel, the keys of play, and the
//! play screen's status column.
//!
//! Everything runs through the null frontend with scripted input, over
//! the fake host and the fake application of `clash_scene::testing`, the
//! screens being the `ui` mod's own under the roles of the repository's
//! default profile. The mods are loaded with the simulation's own types
//! and registry, so that every binding is compiled as the game compiles
//! it: the screens read the simulation's real properties
//! (`view.carried`, `view.parts`, `part.state`, `wound.kind`, ...), and
//! the fake host answers them from what each test says. The fixture mod
//! (`tests/fixtures/mods/person`) is someone shaped like a person: a body
//! of parts, two pools, two attributes, a template with a choice of
//! every kind, and the parts content puts into the screens' slots.
//!
//! The fake application makes characters through a real creation
//! session over the fixture's templates (the creation-session contract,
//! 12 §10.4). The test plays the application's own part where the
//! scene hands over: a game that was asked for closes every screen and
//! opens the one of the role `play`.
//!
//! The golden frames are written with `BLESS=1 cargo test -p clash-scene
//! --test screens`.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use clash_data::{Defs, Diagnostics};
use clash_expr::{Handle, Value};
use clash_scene::control::ROLE_PLAY;
use clash_scene::testing::{FakeApp, FakeHost};
use clash_scene::ui::{AppService, CommandKind};
use clash_scene::view::{CellContents, EntitySprite, LIGHT_FULL, SpriteId, SpriteKind, WorldView};
use clash_scene::{
    Frame, GlyphPart, InputEvent, KeyChord, NullFrontend, Scene, UiCommand, UiDefs, UiOutcome,
    assert_golden, check_defs,
};
use clash_sim::template::Selection;
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

/// The repository's default profile, with the fixture mod in place of
/// `core`: the roles are the ones the game starts with.
fn profile() -> String {
    let path = repository().join("profiles/default.json");
    let text = std::fs::read_to_string(&path).expect("the default profile");
    let mut profile: serde_json::Value = serde_json::from_str(&text).expect("a profile");
    profile["id"] = "screens".into();
    profile["mods"] = serde_json::json!(["ui", "person"]);
    profile.to_string()
}

/// The bundled mods and the fixture, loaded with every definition type
/// and the registry of the engine, as the game loads them.
fn load() -> Arc<Defs> {
    let types = clash_scene::engine_types();
    let defs = clash_scene::engine_loader(&types)
        .search_path(repository().join("mods"))
        .search_path(fixtures())
        .memory_profile("screens", &profile())
        .load_profile("screens")
        .into_result()
        .unwrap_or_else(|e| panic!("{}", e.render()));
    let found = check_defs(&defs);
    assert!(!found.has_errors(), "{}", found.render());
    defs
}

/// The cell the character stands in.
const MARA: (i32, i32) = (4, 3);
/// The cell a knife lies in, in sight.
const KNIFE_CELL: (i32, i32) = (7, 3);

fn at(x: i32, y: i32) -> Location {
    Location::new(GridId::new(0), CellPos::new(x, y, 0))
}

/// Numbers of the fake world's things.
mod thing {
    pub const MARA: u64 = 1;
    pub const STRANGER: u64 = 2;
    pub const KNIFE: u64 = 10;
    pub const COAT: u64 = 11;
    pub const PACK: u64 = 12;
    pub const ROPE: u64 = 13;
    pub const LOOSE_KNIFE: u64 = 14;
}

/// A headless game over the fixture: Mara (`@`) in a small room, with a
/// knife in her hand, a coat and a pack worn and a rope in the pack, an
/// open cut on her left arm, which is disabled by it; a stranger (`s`)
/// and a knife lying on the floor three cells east.
struct Run {
    scene: Scene<FakeHost>,
    front: NullFrontend,
    frame: Frame,
    host: FakeHost,
    app: FakeApp,
    _defs: Arc<Defs>,
}

impl Run {
    /// The scene with nothing open.
    fn bare() -> Run {
        let defs = load();
        let describe = |diagnostics: Diagnostics| diagnostics.render();
        let ui = UiDefs::from_defs(&defs).unwrap_or_else(|e| panic!("{}", describe(e)));
        let registry = (**defs.expr_registry()).clone();
        let host = FakeHost::over(registry);
        let app = FakeApp::new().making(Arc::clone(&defs));
        let mut scene = ui
            .scene_over(host.clone(), "ui:default", "ui:default")
            .unwrap_or_else(|e| panic!("{}", describe(e)))
            .with_app(Box::new(app.clone()))
            .running_effects();

        // What the party perceives, put into the view model by hand.
        let world = WorldDefs::from_defs(&defs).unwrap_or_else(|e| panic!("{}", describe(e)));
        let glyphs = ui
            .world_glyphs(&world, scene.theme())
            .unwrap_or_else(|e| panic!("{}", describe(e)));
        let mut view = WorldView::new(glyphs, at(MARA.0, MARA.1));
        let floor = world.floor_state("person:boards", None).expect("boards");
        let wall = world.block_state("person:wall", None).expect("a wall");
        let door = world.block_state("person:door#open", None).expect("a door");
        for y in 0..7 {
            for x in 0..14 {
                let edge = x == 0 || y == 0 || x == 10 || y == 6;
                let block = match (x, y) {
                    (10, 3) => door,
                    _ if edge && x <= 10 => wall,
                    _ => BlockState::NONE,
                };
                let contents = CellContents {
                    floor,
                    block,
                    light: LIGHT_FULL,
                    ..CellContents::EMPTY
                };
                view.model_mut().observe_cell(at(x, y), contents);
                if x > 10 {
                    view.model_mut().unsee_cell(at(x, y));
                }
            }
        }
        let sprite = |(x, y): (i32, i32), kind, ch| EntitySprite {
            loc: at(x, y),
            kind,
            glyph: GlyphPart {
                ch: Some(ch),
                ..GlyphPart::default()
            },
            own: None,
            dress: None,
            facing: None,
            suspected: false,
        };
        let model = view.model_mut();
        model.upsert_sprite(
            SpriteId::new(thing::MARA),
            sprite(MARA, SpriteKind::Creature, '@'),
        );
        model.upsert_sprite(
            SpriteId::new(thing::LOOSE_KNIFE),
            sprite(KNIFE_CELL, SpriteKind::Object, '/'),
        );
        model.upsert_sprite(
            SpriteId::new(thing::STRANGER),
            sprite(KNIFE_CELL, SpriteKind::Creature, 's'),
        );
        scene.set_world(view);

        let run = Run {
            scene,
            front: NullFrontend::new(80, 24),
            frame: Frame::new(0, 0),
            host,
            app,
            _defs: defs,
        };
        run.person();
        run
    }

    /// The scene with the play screen open.
    fn new() -> Run {
        let mut run = Run::bare();
        let play = run.scene.role(ROLE_PLAY).expect("the play role").to_owned();
        run.scene.open_screen(&play, BTreeMap::new()).unwrap();
        run.press(&[]);
        run
    }

    fn entity(&self, number: u64) -> Handle {
        self.host.entity(number)
    }

    fn part(&self, number: u64) -> Handle {
        self.host.subject("part", number)
    }

    /// Says who Mara is: what she holds, wears and carries, her body,
    /// her wound, her reserves and her attributes.
    fn person(&self) {
        let host = &self.host;
        let subject = |handle: Handle| Value::Subject(handle);
        let things =
            |numbers: &[u64]| Value::list(numbers.iter().map(|n| subject(host.entity(*n))));
        let mara = host.entity(thing::MARA);
        host.set_view(subject(mara));
        host.set_party(Value::list([subject(mara)]));
        host.set(mara, "name", "Mara");
        host.place(mara, MARA.0, MARA.1, 0);

        // What she has.
        host.set(
            mara,
            "held",
            Value::map([
                ("grasp", things(&[thing::KNIFE])),
                ("worn", things(&[thing::COAT, thing::PACK])),
            ]),
        );
        host.set(
            mara,
            "carried",
            things(&[thing::KNIFE, thing::COAT, thing::PACK, thing::ROPE]),
        );
        host.set(mara, "carried_mass", Value::Real(clash_core::Real::of(5.2)));
        for (number, name, hold, holder) in [
            (thing::KNIFE, "knife", "grasp", thing::MARA),
            (thing::COAT, "coat", "worn", thing::MARA),
            (thing::PACK, "pack", "worn", thing::MARA),
            (thing::ROPE, "rope", "stowed", thing::PACK),
        ] {
            let item = host.entity(number);
            host.set(item, "name", name);
            host.set(item, "hold", hold);
            host.set(item, "holder", subject(host.entity(holder)));
        }
        let pack = host.entity(thing::PACK);
        host.set(
            pack,
            "capacity_mass",
            Value::Real(clash_core::Real::of(20.0)),
        );
        host.set(pack, "carried_mass", Value::Real(clash_core::Real::of(2.0)));
        host.set(host.entity(thing::LOOSE_KNIFE), "name", "a knife");
        host.set(host.entity(thing::STRANGER), "name", "a stranger");

        // Her body: a torso with a head, two arms with hands and two legs
        // on it, and a heart in it.
        let parts = [
            (1, "torso", None, "whole", 1.0),
            (2, "head", Some(1), "whole", 1.0),
            (3, "l_arm", Some(1), "disabled", 0.42),
            (4, "l_hand", Some(3), "whole", 1.0),
            (5, "r_arm", Some(1), "whole", 1.0),
            (6, "r_hand", Some(5), "whole", 1.0),
            (7, "l_leg", Some(1), "whole", 1.0),
            (8, "r_leg", Some(1), "whole", 1.0),
            (9, "heart", Some(1), "whole", 1.0),
        ];
        let wound = host.subject("wound", 1);
        for (number, name, parent, state, condition) in parts {
            let part = self.part(number);
            host.set(part, "name", name);
            host.set(part, "state", state);
            host.set(
                part,
                "condition",
                Value::Real(clash_core::Real::of(condition)),
            );
            host.set(
                part,
                "parent",
                parent.map_or(Value::Null, |p| subject(self.part(p))),
            );
            let on: Vec<Value> = parts
                .iter()
                .filter(|(n, _, p, ..)| *p == Some(number) && *n != 9)
                .map(|(n, ..)| subject(self.part(*n)))
                .collect();
            host.set(part, "attached", Value::list(on));
            let inside = if number == 1 {
                vec![subject(self.part(9))]
            } else {
                Vec::new()
            };
            host.set(part, "inside", Value::list(inside));
            let wounds = if number == 3 {
                vec![subject(wound)]
            } else {
                Vec::new()
            };
            host.set(part, "wounds", Value::list(wounds));
            host.set(part, "owner", subject(mara));
        }
        host.set(
            mara,
            "parts",
            Value::list(parts.iter().map(|(n, ..)| subject(self.part(*n)))),
        );
        host.set(
            mara,
            "worn_on",
            Value::map([
                ("torso", things(&[thing::COAT, thing::PACK])),
                ("l_arm", things(&[thing::COAT])),
                ("r_arm", things(&[thing::COAT])),
            ]),
        );
        let registry = host.registry().expect("a registry");
        let cut = registry
            .def_type_id("damage_type")
            .and_then(|ty| registry.def_ref(ty, "person:cut"))
            .expect("the fixture's damage type");
        host.set(wound, "part", subject(self.part(3)));
        host.set(wound, "kind", Value::Id(cut));
        host.set(wound, "state", "open");
        host.set(wound, "depth", 2);
        host.set(wound, "stump", false);
        host.set(mara, "wounds", Value::list([subject(wound)]));

        // Her reserves and attributes.
        let pools = host.subject("pools", 1);
        host.set(mara, "pool", subject(pools));
        host.set(pools, "blood", Value::Real(clash_core::Real::of(3.5)));
        host.set(pools, "pain", Value::Real(clash_core::Real::of(30.0)));
        host.set(
            pools,
            "frac",
            Value::map([
                ("person:blood", Value::Real(clash_core::Real::of(0.7))),
                ("person:pain", Value::Real(clash_core::Real::of(0.3))),
            ]),
        );
        let attrs = host.subject("attrs", 1);
        host.set(mara, "attr", subject(attrs));
        host.set(attrs, "strength", Value::Real(clash_core::Real::of(12.0)));
        host.set(attrs, "wits", Value::Real(clash_core::Real::of(10.0)));
    }

    fn turn(&mut self, events: Vec<InputEvent>) -> Vec<UiOutcome> {
        self.front.script(events);
        let outcomes = self
            .scene
            .turn(&mut self.front, &mut self.frame, STEP)
            .outcomes;
        self.follow_up(&outcomes);
        outcomes
    }

    /// The application's part: a service that failed shows why; a game
    /// that was asked for closes every screen and opens the play screen,
    /// with the character the player made, if one was finished.
    fn follow_up(&mut self, outcomes: &[UiOutcome]) {
        // A service that failed opens the screen of the role `error` with
        // the reason, as the application does.
        for outcome in outcomes {
            if let UiOutcome::Failed {
                effect: "app",
                reason,
            } = outcome
            {
                let error = self.scene.role("error").expect("the error role").to_owned();
                let with = BTreeMap::from([("text".to_owned(), Value::text(reason))]);
                self.scene.open_screen(&error, with).unwrap();
            }
        }
        let started = outcomes.iter().any(|outcome| {
            matches!(outcome, UiOutcome::Service(request) if request.service == AppService::NewGame)
        });
        if !started {
            return;
        }
        if let Some(made) = self.app.finished() {
            let mara = self.entity(thing::MARA);
            self.host.set(mara, "name", made.name.as_str());
        }
        while self.scene.close().is_some() {}
        let play = self
            .scene
            .role(ROLE_PLAY)
            .expect("the play role")
            .to_owned();
        self.scene.open_screen(&play, BTreeMap::new()).unwrap();
    }

    /// Presses keys, one turn of the loop each, and returns what the
    /// effects came to.
    fn press(&mut self, keys: &[&str]) -> Vec<UiOutcome> {
        let mut outcomes = Vec::new();
        for key in keys {
            outcomes.extend(self.turn(vec![InputEvent::key(key).unwrap()]));
        }
        // What the last key changed is drawn by the next pass.
        outcomes.extend(self.turn(Vec::new()));
        outcomes
    }

    /// Types a text, a key for each character.
    fn type_text(&mut self, text: &str) {
        for ch in text.chars() {
            self.turn(vec![InputEvent::Key(KeyChord::char(ch))]);
        }
        self.turn(Vec::new());
    }

    fn shown(&self) -> &Frame {
        self.front.last().unwrap()
    }

    fn text(&self) -> String {
        self.shown().text()
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

    /// The choice of the creation session with this key, as
    /// `app.creation` gives it.
    fn choice(&self, key: &str) -> Value {
        let creation = self.scene_app_creation();
        creation
            .as_map()
            .and_then(|map| map.get("choices"))
            .and_then(Value::as_list)
            .and_then(|choices| {
                choices.iter().find(|choice| {
                    choice
                        .as_map()
                        .and_then(|map| map.get("key"))
                        .and_then(Value::as_str)
                        == Some(key)
                })
            })
            .cloned()
            .unwrap_or_else(|| panic!("no choice `{key}` is open"))
    }

    fn scene_app_creation(&self) -> Value {
        use clash_scene::AppState;
        self.app.creation()
    }

    /// The value selected for a choice.
    fn selected(&self, key: &str) -> Value {
        self.choice(key)
            .as_map()
            .and_then(|map| map.get("value"))
            .cloned()
            .unwrap_or_default()
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

fn texts(items: &[&str]) -> Value {
    Value::list(items.iter().map(|item| Value::text(*item)))
}

// ---------------------------------------------------------------------
// Character creation
// ---------------------------------------------------------------------

#[test]
fn scripted_input_goes_from_the_title_to_a_made_character_in_play() {
    let mut run = Run::bare();
    run.scene.open_screen("ui:title", BTreeMap::new()).unwrap();
    run.press(&[]);
    assert_eq!(run.top(), "ui:title");

    // The new-game screen; F2 makes the character first, for the start
    // that is selected.
    run.press(&["enter"]);
    assert_eq!(run.top(), "ui:new_game");
    run.press(&["tab", "tab", "down", "f2"]);
    assert_eq!(run.top(), "ui:create");

    // Nothing is being made yet: what to make is chosen first.
    assert!(run.text().contains("What to make"), "{}", run.text());
    assert!(run.row_with("Person").contains("species"));
    assert!(run.row_with("Shade").contains("spirit"));
    assert_golden(golden("create_begin"), run.shown());
    run.press(&["enter"]);
    let begun = run.app.requests();
    assert_eq!(begun.last().map(|r| r.text("op")), Some(Some("begin")));
    assert_eq!(
        begun.last().unwrap().text("template"),
        Some("person:person")
    );

    // The session's first frame: every choice of the template, with what
    // is chosen now, and the character is complete with its defaults.
    assert!(run.text().contains("Made from Person"), "{}", run.text());
    assert!(run.row_with("Build").contains("average"));
    assert!(run.text().contains("Ready to begin."), "{}", run.text());
    assert_golden(golden("create_first"), run.shown());

    // A name: the focus is on it.
    assert_eq!(run.scene.top().unwrap().focus(), Some("name"));
    run.type_text("Ada");
    let named = run.app.requests();
    assert_eq!(named.last().unwrap().text("op"), Some("name"));
    assert_eq!(
        named.last().unwrap().args.get("value"),
        Some(&Value::text("Ada"))
    );

    // A heavier build, from the choices list: Tab to it, down to Build.
    run.press(&["tab", "tab", "tab"]);
    assert_eq!(run.scene.top().unwrap().focus(), Some("choices"));
    run.press(&["down", "down", "enter"]);
    assert_eq!(run.top(), "ui:create_choice");
    run.press(&["down", "down", "enter"]);
    assert_eq!(run.selected("person:person.build"), texts(&["heavy"]));
    run.press(&["escape"]);
    assert_eq!(run.top(), "ui:create");

    // Begin: the character is finished and the game asked for at the
    // start the screen was opened for.
    run.press(&["tab", "tab"]);
    let focus = run.scene.top().unwrap().focus().map(str::to_owned);
    assert_eq!(focus.as_deref(), Some("begin"));
    let outcomes = run.press(&["enter"]);
    let services: Vec<AppService> = outcomes
        .iter()
        .filter_map(|outcome| match outcome {
            UiOutcome::Service(request) => Some(request.service),
            _ => None,
        })
        .collect();
    assert_eq!(services, [AppService::Creation, AppService::NewGame]);
    let asked = run.app.requests();
    assert_eq!(
        asked.last().unwrap().text("scenario"),
        Some("core:siege"),
        "the start that was selected"
    );
    let made = run.app.finished().expect("a character was finished");
    assert_eq!(made.name, "Ada");
    assert_eq!(made.spec.templates, ["person:person"]);
    assert_eq!(
        made.spec.select.get("person:person.build"),
        Some(&Selection::Options(vec!["heavy".to_owned()]))
    );

    // In play, as the character made.
    assert_eq!(run.top(), "ui:play");
    assert!(run.row_with("Ada").starts_with("│Ada") || run.text().contains("Ada"));
}

#[test]
fn a_template_with_a_choice_of_every_kind_is_made_on_the_one_screen() {
    let mut run = Run::bare();
    run.scene.open_screen("ui:create", BTreeMap::new()).unwrap();
    run.press(&[]);
    run.press(&["enter"]);
    assert!(run.text().contains("Name"), "{}", run.text());
    run.press(&["tab", "tab", "tab"]);
    let focus = run.scene.top().unwrap().focus().map(str::to_owned);
    assert_eq!(focus.as_deref(), Some("choices"));

    // `text`: a line to type, started from what it holds.
    run.press(&["enter"]);
    assert_eq!(run.top(), "ui:create_choice");
    run.type_text("Ada");
    assert_golden(golden("create_text"), run.shown());
    run.press(&["enter"]);
    assert_eq!(run.top(), "ui:create");
    assert_eq!(run.selected("person:person.name"), Value::text("Ada"));
    assert!(run.text().contains("││Name  Ada"), "{}", run.text());

    // `range`: a number within its bounds, stepped by a tenth of them.
    run.press(&["down", "enter"]);
    assert!(run.text().contains("From 0.8 to 1.2."), "{}", run.text());
    run.press(&["right", "right", "enter"]);
    let stature = run.selected("person:person.stature");
    assert_eq!(
        stature.as_number().map(|n| (n.get() * 100.0).round()),
        Some(108.0)
    );

    // `pick`: one of three; Enter puts the new one in place of the old.
    run.press(&["down", "enter"]);
    assert_golden(golden("create_pick"), run.shown());
    run.press(&["enter"]);
    assert_eq!(run.selected("person:person.build"), texts(&["slight"]));
    run.press(&["escape"]);

    // `points`: options taken and put back within a budget; the one
    // that costs more than is left is refused with the reason.
    run.press(&["down", "enter"]);
    run.press(&["down", "enter"]);
    assert_eq!(run.selected("person:person.training"), texts(&["reader"]));
    assert!(run.text().contains("2 of 3 points left."), "{}", run.text());
    run.press(&["up", "enter"]);
    assert_eq!(
        run.selected("person:person.training"),
        texts(&["reader", "runner"])
    );
    run.press(&["escape"]);
    // The option taken opens a choice of its own, right after its own.
    assert!(run.row_with("Script").contains("plain"), "{}", run.text());

    // The nested `pick`.
    run.press(&["down", "enter", "down", "enter", "escape"]);
    assert_eq!(
        run.selected("person:person.training.reader.script"),
        texts(&["court"])
    );

    // `assemble`: parts added one at a time within a budget, and taken
    // back with Backspace.
    run.press(&["down", "enter"]);
    run.press(&["enter", "enter", "down", "enter"]);
    assert_eq!(
        run.selected("person:person.extras"),
        texts(&["person:tail", "person:tail", "person:horn"])
    );
    assert_golden(golden("create_assemble"), run.shown());
    run.press(&["up", "backspace"]);
    assert_eq!(
        run.selected("person:person.extras"),
        texts(&["person:horn"])
    );
    run.press(&["escape"]);

    // What the templates and options chosen give.
    assert!(run.row_with("Strength").contains('8'), "{}", run.text());
    assert!(run.row_with("Wits").contains("12"), "{}", run.text());
    assert_golden(golden("create_done"), run.shown());
}

#[test]
fn a_selection_the_session_refuses_is_shown_with_its_reason() {
    let mut run = Run::bare();
    run.scene.open_screen("ui:create", BTreeMap::new()).unwrap();
    run.press(&[]);
    run.press(&["enter", "tab", "tab", "tab"]);
    // Extras: three parts are its budget; a fourth is refused, and the
    // application's reason is shown over the screen.
    run.press(&["down", "down", "down", "down", "enter"]);
    assert!(run.text().contains("0 of 3 parts."), "{}", run.text());
    run.press(&["enter", "enter", "enter"]);
    assert_eq!(
        run.selected("person:person.extras"),
        texts(&["person:tail", "person:tail", "person:tail"])
    );
    let outcomes = run.press(&["enter"]);
    assert!(
        outcomes.iter().any(|outcome| matches!(
            outcome,
            UiOutcome::Failed { effect: "app", reason } if reason.contains("budget")
        )),
        "{outcomes:?}"
    );
    assert_eq!(run.top(), "ui:error");
    assert!(run.text().contains("budget of 3"), "{}", run.text());
    run.press(&["escape"]);
    assert_eq!(run.top(), "ui:create_choice");
    assert_eq!(
        run.selected("person:person.extras"),
        texts(&["person:tail", "person:tail", "person:tail"]),
        "a refused selection changes nothing"
    );
}

// ---------------------------------------------------------------------
// The inventory and the condition
// ---------------------------------------------------------------------

#[test]
fn the_inventory_lists_what_is_held_worn_and_carried_and_offers_what_can_be_done() {
    let mut run = Run::new();
    run.press(&["i"]);
    assert_eq!(run.top(), "ui:inventory");
    assert!(run.row_with("knife  in hand").starts_with("││"));
    assert!(run.row_with("coat  worn").starts_with("││"));
    assert!(
        run.row_with("pack  worn").contains("2 of 20 kg"),
        "{}",
        run.text()
    );
    assert!(run.row_with("  rope  in pack").starts_with("││"));
    assert!(run.row_with("l_arm").contains("coat"));
    assert!(run.text().contains("Carrying 5.2 kg."), "{}", run.text());
    assert_golden(golden("inventory"), run.shown());

    // Enter on the knife: what can be done with something held in hand.
    run.press(&["enter"]);
    assert_eq!(run.top(), "ui:item");
    let text = run.text();
    for offered in ["Drop", "Wear", "Stow", "Put in...", "Give..."] {
        assert!(text.contains(offered), "`{offered}` in:\n{text}");
    }
    for not_offered in ["Take off", "Take out"] {
        assert!(!text.contains(not_offered), "`{not_offered}` in:\n{text}");
    }
    assert_golden(golden("inventory_item"), run.shown());
    let outcomes = run.press(&["enter"]);
    let command = submitted(&outcomes);
    assert_eq!(
        (command.kind, command.name.as_str()),
        (CommandKind::Act, "drop")
    );
    assert_eq!(
        command.args.get("what"),
        Some(&Value::Subject(run.entity(thing::KNIFE)))
    );
    assert_eq!(
        run.top(),
        "ui:inventory",
        "the popup closed once it was done"
    );

    // The rope, in the pack: taking it out is offered, stowing is not.
    run.press(&["down", "down", "down", "enter"]);
    let text = run.text();
    assert!(
        text.contains("Take out") && !text.contains("Stow"),
        "{text}"
    );
    let outcomes = run.press(&["down", "down", "enter"]);
    assert!(
        matches!(&outcomes[..], [UiOutcome::Opened(id)] if id == "ui:pick_list"),
        "putting in asks which container first: {outcomes:?}"
    );
    run.press(&["escape", "escape"]);
    assert_eq!(run.top(), "ui:inventory");
}

#[test]
fn the_condition_shows_the_body_as_a_tree_its_wounds_reserves_and_attributes() {
    let mut run = Run::new();
    run.press(&["c"]);
    assert_eq!(run.top(), "ui:condition");
    assert!(run.row_with("l_arm").contains("42%"));
    assert!(run.row_with("l_arm").contains("disabled, 1 wound"));
    assert!(run.row_with("heart").contains("100%"));
    assert!(
        run.row_with("l_arm: person:cut").contains("open, 2 deep"),
        "{}",
        run.text()
    );
    assert!(run.row_with("Blood").contains("███"));
    assert!(run.row_with("Pain").contains("███"));
    assert!(run.row_with("Strength 12").contains("Wits 10"));
    assert_golden(golden("condition"), run.shown());
    // The tree's branches close and open.
    run.press(&["left"]);
    assert!(!run.text().contains("l_hand"), "{}", run.text());
    run.press(&["right"]);
    assert!(run.text().contains("l_hand"), "{}", run.text());
}

// ---------------------------------------------------------------------
// Look, the keys, the play screen
// ---------------------------------------------------------------------

#[test]
fn look_mode_says_what_the_cell_holds_as_the_party_perceives_it() {
    let mut run = Run::new();
    run.press(&["x"]);
    assert_eq!(run.top(), "ui:look");
    assert!(run.text().contains("Boards."), "{}", run.text());
    // Over to where the stranger stands on a knife.
    run.press(&["l", "l", "l"]);
    assert!(run.row_with("A stranger and a").contains('│'));
    assert!(run.text().contains("knife."), "{}", run.text());
    assert_golden(golden("look_panel"), run.shown());
    // The open door, by its state.
    run.press(&["l", "l", "l"]);
    assert!(run.text().contains("Door, open; boards."), "{}", run.text());
    // A cell nobody has seen.
    run.press(&["k", "k", "k", "k", "k"]);
    assert!(run.text().contains("Nothing known."), "{}", run.text());
    assert!(run.text().contains("Nothing seen."), "{}", run.text());
    run.press(&["escape"]);
    assert_eq!(run.top(), "ui:play");
}

#[test]
fn the_keys_of_play_are_listed_from_the_keymap_in_force() {
    let mut run = Run::new();
    run.press(&["?"]);
    assert!(run.row_with("Inventory").contains('i'));
    // The walking controls come last, by what they are called.
    run.press(&["end"]);
    assert_eq!(run.top(), "ui:help");
    assert!(
        run.row_with("Walk north").contains("k, up"),
        "{}",
        run.text()
    );
    assert_golden(golden("help"), run.shown());
    run.press(&["escape"]);
    // A key the player binds shows as it is bound.
    run.scene.rebind("play", "w", "north").unwrap();
    run.press(&["f1", "end"]);
    assert_eq!(run.top(), "ui:help");
    let north = run.row_with("Walk north");
    assert!(north.contains('k') && north.contains('w'), "{}", run.text());
}

#[test]
fn the_status_column_shows_the_character_its_reserves_and_what_it_holds() {
    let run = Run::new();
    assert!(run.row_with("Mara").starts_with('│'), "{}", run.text());
    assert!(run.row_with("Blood").contains("███"));
    assert!(run.row_with("Holding").contains("knife"));
    assert!(run.text().contains("x look  i things  c body"));
    assert!(run.text().contains("? keys  Esc menu"));
    assert_eq!(
        run.scene.top().unwrap().slots().collect::<Vec<_>>(),
        ["play.bars", "play.side"]
    );
    assert_golden(golden("play_status"), run.shown());
}

#[test]
fn every_bundled_screen_is_reached_from_play_by_its_key_and_left_by_escape() {
    let mut run = Run::new();
    for (key, screen) in [
        ("i", "ui:inventory"),
        ("c", "ui:condition"),
        ("x", "ui:look"),
        ("?", "ui:help"),
        ("f1", "ui:help"),
        ("a", "ui:abilities"),
        ("m", "ui:map"),
        ("shift+j", "ui:journal"),
        ("o", "ui:options"),
        ("escape", "ui:pause"),
    ] {
        let outcomes = run.press(&[key]);
        assert!(
            outcomes.contains(&UiOutcome::Opened(screen.to_owned())),
            "`{key}`: {outcomes:?}"
        );
        assert_eq!(run.top(), screen);
        run.press(&["escape"]);
        assert_eq!(run.top(), "ui:play", "Esc leaves {screen}");
    }
    // And from the inventory, the item popup and back.
    run.press(&["i", "enter"]);
    assert_eq!(run.top(), "ui:item");
    run.press(&["escape", "escape"]);
    assert_eq!(run.top(), "ui:play");
}

#[test]
fn a_screen_opened_with_values_for_its_variables_starts_from_them() {
    let mut run = Run::bare();
    let with = |pairs: &[(&str, Value)]| -> BTreeMap<String, Value> {
        pairs
            .iter()
            .map(|(name, value)| ((*name).to_owned(), value.clone()))
            .collect()
    };
    run.scene
        .open_screen(
            "ui:create_choice",
            with(&[("index", Value::Int(0)), ("words", Value::text("Ada"))]),
        )
        .unwrap();
    let top = run.scene.top().unwrap();
    assert_eq!(
        top.var("words"),
        Some(&clash_core::Value::Str("Ada".to_owned()))
    );
    // A variable is given only what it can hold, and a name the screen
    // has as neither a parameter nor a variable is a mistake.
    let wrong = run
        .scene
        .open_screen("ui:create_choice", with(&[("words", Value::Int(3))]))
        .unwrap_err();
    assert!(wrong.contains("cannot start as 3"), "{wrong}");
    let unknown = run
        .scene
        .open_screen("ui:create_choice", with(&[("colour", Value::text("red"))]))
        .unwrap_err();
    assert!(
        unknown.contains("no parameter or variable `colour`"),
        "{unknown}"
    );
}
