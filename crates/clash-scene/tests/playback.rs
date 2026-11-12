//! Playback (12 §7; package U5): the simulation's events played into the
//! picture and the log over real time, with the `show` and `anim`
//! definitions of the `ui` mod.
//!
//! Everything runs through the null frontend with a fixed time step,
//! over the fake host of `clash_scene::testing`, whose stand-in for
//! perception says what the party saw, heard or missed. The screens and
//! the shows are the `ui` mod's own; the room is the fixture mod's
//! floor and wall (`tests/fixtures/mods/fix`).
//!
//! Each test keeps a record of the truth beside the scene: what the
//! party sees in each cell and how it perceives each thing. The host is
//! told it as the simulation would have it once a batch has happened,
//! and [`Run::fresh`] makes from it the view model a fresh
//! initialisation would give, which caught-up playback must equal
//! (12 §5, invariant V4).

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use clash_core::{Dur, EntityId, Real, Tick, Value};
use clash_data::{DefTypes, Defs, Diagnostics, Loader};
use clash_scene::control::{DecisionAsk, PendingDecision};
use clash_scene::playback::{Importance, check_caught_up};
use clash_scene::testing::FakeHost;
use clash_scene::view::{
    CellContents, EntitySprite, LIGHT_FULL, SpriteId, SpriteKind, ViewModel, WorldView,
};
use clash_scene::{
    Frame, GlyphPart, InputEvent, NullFrontend, Scene, UiDefs, assert_golden, check_defs,
};
use clash_sim::{EventBody, Place, SimEvent};
use clash_time::ClockId;
use clash_world::{
    BlockState, BodyId, BodyOwner, Cause, CellPos, DamageTypeId, Destruction, ElementState,
    FloorState, GridId, Layer, Location, WorldDefs, WorldEvent,
};
use proptest::prelude::*;

/// The fixed time step of scripted runs, in seconds.
const STEP: f64 = 1.0 / 60.0;

/// When the batch of each test begins, in ticks.
const T: i64 = 5_000_000;

/// The things in the room, by number: Mara (`@`) is `view`, Jonas (`j`)
/// is the other of the party, and the stranger (`s`) is seen.
const MARA: u64 = 1;
const JONAS: u64 = 2;
const STRANGER: u64 = 3;

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

/// The repository's default profile over `ui`, the fixture mod and
/// whatever extra mods a test adds.
fn profile(extra: &[&str]) -> String {
    let path = repository().join("profiles/default.json");
    let text = std::fs::read_to_string(&path).expect("the default profile");
    let mut profile: serde_json::Value = serde_json::from_str(&text).expect("a profile");
    profile["id"] = "test".into();
    let mut mods = vec!["ui", "fix"];
    mods.extend(extra);
    profile["mods"] = serde_json::json!(mods);
    profile.to_string()
}

/// Loads the `ui` mod and the fixture, with extra mods given as `(id,
/// text of data/extra.json)`, against the fake host's registry.
fn try_load(extra: &[(&str, &str)]) -> Result<Arc<Defs>, Diagnostics> {
    let types = types();
    let registry = clash_scene::testing::registry();
    let mut loader = Loader::new(&types)
        .expr_registry(&registry)
        .search_path(repository().join("mods"))
        .search_path(fixtures());
    for (id, text) in extra {
        let manifest = format!(r#"{{ "id": "{id}" }}"#);
        loader = loader.memory_mod(id, &manifest, &[("data/extra.json", text)]);
    }
    let ids: Vec<&str> = extra.iter().map(|(id, _)| *id).collect();
    let defs = loader
        .memory_profile("test", &profile(&ids))
        .load_profile("test")
        .into_result()?;
    let found = check_defs(&defs);
    if found.has_errors() {
        return Err(found);
    }
    Ok(defs)
}

fn at(x: i32, y: i32) -> Location {
    Location::new(GridId::new(0), CellPos::new(x, y, 0))
}

fn id(number: u64) -> EntityId {
    EntityId::from_bits(number)
}

fn creature(ch: char, (x, y): (i32, i32)) -> EntitySprite {
    EntitySprite {
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
    }
}

/// What the party knows of the world outside the scene: what it sees,
/// what it remembers and how it perceives each thing.
#[derive(Clone, Debug, Default)]
struct Truth {
    sight: BTreeMap<Location, CellContents>,
    memory: BTreeMap<Location, CellContents>,
    looks: BTreeMap<u64, Option<EntitySprite>>,
}

impl Truth {
    /// The view model a fresh initialisation gives: what is remembered,
    /// then what is seen and perceived now.
    fn model(&self) -> ViewModel {
        let mut model = ViewModel::new();
        for (loc, contents) in &self.memory {
            model.observe_cell(*loc, *contents);
            model.unsee_cell(*loc);
        }
        for (loc, contents) in &self.sight {
            model.observe_cell(*loc, *contents);
        }
        for (number, look) in &self.looks {
            if let Some(look) = look {
                model.upsert_sprite(SpriteId::new(*number), *look);
            }
        }
        model
    }
}

/// A headless game: the play screen over a room the party sees, with
/// a strip east of it that it remembers and does not see. The room's
/// walls are on its edges, from (0, 0) to (10, 6); the camera is on
/// (4, 3), which the map view draws at column 26 and row 8.
struct Run {
    scene: Scene<FakeHost>,
    front: NullFrontend,
    frame: Frame,
    host: FakeHost,
    truth: Truth,
    floor: FloorState,
    wall: BlockState,
}

/// Where Mara stands at first.
const MARA_AT: (i32, i32) = (4, 2);
const JONAS_AT: (i32, i32) = (2, 5);
const STRANGER_AT: (i32, i32) = (7, 3);

/// The column and row of the frame a cell of the room is drawn at.
fn screen((x, y): (i32, i32)) -> (i32, i32) {
    (22 + x, 5 + y)
}

impl Run {
    fn new() -> Run {
        Run::with(&[])
    }

    fn with(extra: &[(&str, &str)]) -> Run {
        let describe = |diagnostics: Diagnostics| diagnostics.render();
        let defs = try_load(extra).unwrap_or_else(|e| panic!("{}", describe(e)));
        let ui = UiDefs::from_defs(&defs).unwrap_or_else(|e| panic!("{}", describe(e)));
        assert!(!ui.shows(&clash_scene::Theme::default()).0.is_empty());
        let host = FakeHost::new();
        let mut scene = ui
            .scene_over(host.clone(), "ui:default", "ui:default")
            .unwrap_or_else(|e| panic!("{}", describe(e)))
            .running_effects();
        let world = WorldDefs::from_defs(&defs).unwrap_or_else(|e| panic!("{}", describe(e)));
        let glyphs = ui
            .world_glyphs(&world, scene.theme())
            .unwrap_or_else(|e| panic!("{}", describe(e)));
        let floor = world.floor_state("fix:boards", None).expect("boards");
        let wall = world.block_state("fix:wall", None).expect("a wall");

        let mut truth = Truth::default();
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
                if x > 10 {
                    truth.memory.insert(at(x, y), contents);
                } else {
                    truth.sight.insert(at(x, y), contents);
                    host.see(at(x, y), Some(contents));
                }
            }
        }
        for (number, ch, place) in [
            (MARA, '@', MARA_AT),
            (JONAS, 'j', JONAS_AT),
            (STRANGER, 's', STRANGER_AT),
        ] {
            let look = Some(creature(ch, place));
            truth.looks.insert(number, look);
            host.look(number, look);
            host.place(host.entity(number), place.0, place.1, 0);
        }
        let mut view = WorldView::new(glyphs, at(4, 3));
        *view.model_mut() = truth.model();
        scene.set_world(view);
        let play = scene.role("play").expect("the play role").to_owned();
        scene.open_screen(&play, BTreeMap::new()).unwrap();
        let mut run = Run {
            scene,
            front: NullFrontend::new(80, 24),
            frame: Frame::new(0, 0),
            host,
            truth,
            floor,
            wall,
        };
        run.turns(1);
        run
    }

    /// Runs turns of the loop with no input.
    fn turns(&mut self, count: usize) {
        for _ in 0..count {
            self.front.script(Vec::new());
            self.scene.turn(&mut self.front, &mut self.frame, STEP);
        }
    }

    /// Runs turns until playback has nothing left to play or finish.
    fn until_idle(&mut self) -> usize {
        let mut turns = 0;
        while !self.scene.playback().is_idle() {
            self.turns(1);
            turns += 1;
            assert!(turns < 600, "playback did not end in ten seconds");
        }
        // One more, to draw what the last turn left.
        self.turns(1);
        turns
    }

    fn press(&mut self, key: &str) {
        self.front.script(vec![InputEvent::key(key).unwrap()]);
        self.scene.turn(&mut self.front, &mut self.frame, STEP);
    }

    fn shown(&self) -> &Frame {
        self.front.last().unwrap()
    }

    /// The character drawn for a cell of the room.
    fn ch(&self, cell: (i32, i32)) -> char {
        let (col, row) = screen(cell);
        self.shown().get(col, row).unwrap().ch
    }

    fn lines(&self) -> Vec<String> {
        self.scene
            .log()
            .lines()
            .iter()
            .map(|line| line.shown())
            .collect()
    }

    /// The party now sees this in a cell.
    fn set_cell(&mut self, cell: (i32, i32), block: BlockState) {
        let contents = CellContents {
            floor: self.floor,
            block,
            light: LIGHT_FULL,
            ..CellContents::EMPTY
        };
        self.truth.sight.insert(at(cell.0, cell.1), contents);
        self.host.see(at(cell.0, cell.1), Some(contents));
    }

    /// The party now perceives a thing so, or knows of it and does not
    /// perceive it.
    fn set_look(&mut self, number: u64, look: Option<EntitySprite>) {
        self.truth.looks.insert(number, look);
        self.host.look(number, look);
        if let Some(look) = look {
            let cell = look.loc.cell;
            self.host
                .place(self.host.entity(number), cell.x, cell.y, cell.z);
        }
    }

    /// The view model a fresh initialisation would give now.
    fn fresh(&self) -> ViewModel {
        self.truth.model()
    }

    fn model(&self) -> &ViewModel {
        self.scene.world().unwrap().model()
    }

    fn check(&self) {
        if let Err(drift) = self.scene.check_caught_up(&self.fresh()) {
            panic!("{drift}");
        }
    }
}

fn event(tick: i64, cause: Cause, body: EventBody) -> SimEvent {
    SimEvent {
        tick: Tick(tick),
        clock: ClockId::UNIVERSAL,
        local: Dur(tick),
        cause,
        body,
    }
}

fn moved(tick: i64, who: u64, from: (i32, i32), to: (i32, i32)) -> SimEvent {
    event(
        tick,
        Cause::DEBUG.by(id(who)),
        EventBody::EntityRelocated {
            entity: id(who),
            from: Place::at(at(from.0, from.1)),
            to: Place::at(at(to.0, to.1)),
        },
    )
}

fn destroyed(tick: i64, cell: (i32, i32), by: Option<u64>, how: Destruction) -> SimEvent {
    let cause = by.map_or(Cause::DEBUG, |who| Cause::DEBUG.by(id(who)));
    event(
        tick,
        cause,
        EventBody::World(WorldEvent::ElementDestroyed {
            tick: Tick(tick),
            loc: at(cell.0, cell.1),
            layer: Layer::Block,
            old: ElementState::NONE,
            new: ElementState::NONE,
            how,
            absorbed: Real::ZERO,
            passed: Real::ZERO,
            cause,
        }),
    )
}

/// A blow that breaks a wall.
fn broken(tick: i64, cell: (i32, i32), by: Option<u64>) -> SimEvent {
    destroyed(tick, cell, by, Destruction::Damage(DamageTypeId::new(0)))
}

/// Mara steps north, and three ticks later breaks the wall before her.
fn wall_punch(run: &mut Run) -> Vec<SimEvent> {
    run.set_look(MARA, Some(creature('@', (4, 1))));
    run.set_cell((4, 0), BlockState::NONE);
    vec![
        moved(T, MARA, MARA_AT, (4, 1)),
        broken(T + 3, (4, 0), Some(MARA)),
    ]
}

// ---------------------------------------------------------------------
// Playing a batch over time
// ---------------------------------------------------------------------

#[test]
fn a_wall_broken_three_ticks_after_a_step_is_drawn_until_its_event_is_played() {
    let mut run = Run::new();
    run.host.read_as(Some(run.host.entity(MARA)));
    assert_eq!(run.ch(MARA_AT), '@');
    assert_eq!(run.ch((4, 0)), '#');

    let batch = wall_punch(&mut run);
    run.scene.push_events(batch);
    assert!(!run.scene.playback_caught_up());
    assert_eq!(run.scene.playback().pending(), 2);

    // The step is played first: Mara glides north and nothing is
    // written. The wall, long gone in the simulation, is still there.
    run.turns(1);
    assert_golden(golden("playback_step_begins"), run.shown());
    assert_eq!(run.ch((4, 0)), '#');
    assert_eq!(
        run.ch(MARA_AT),
        '.',
        "the gliding sprite is drawn between cells"
    );
    assert!(run.lines().is_empty(), "a step writes no line");
    assert_eq!(run.shown().sprites.len(), 1);
    run.turns(3);
    assert_golden(golden("playback_step_glides"), run.shown());
    let glide = run.shown().sprites[0];
    assert_eq!(glide.ch, '@');
    assert!(
        (6.0..7.0).contains(&glide.y),
        "on its way up a row: {}",
        glide.y
    );

    // The stretch takes 350 ms however few ticks it covered: at 333 ms
    // the wall stands, and by 367 ms it has been broken.
    run.turns(16);
    assert_eq!(run.ch((4, 0)), '#');
    assert_eq!(run.ch((4, 1)), '@');
    assert!(run.model().cell(at(4, 0)).unwrap().block == run.wall);
    run.turns(2);
    assert!(run.scene.playback_caught_up());
    assert_eq!(run.model().cell(at(4, 0)).unwrap().block, BlockState::NONE);
    assert_eq!(run.lines(), ["You break the wall."]);
    assert_eq!(run.ch((4, 0)), '*', "the burst over the broken wall");
    assert!(run.shown().fx.shake > 0.0);
    assert_golden(golden("playback_wall_breaks"), run.shown());

    run.until_idle();
    assert_eq!(run.ch((4, 0)), '.');
    assert_eq!(run.shown().fx.shake, 0.0);
    assert_golden(golden("playback_wall_done"), run.shown());
    run.check();
}

#[test]
fn the_line_is_worded_for_whoever_reads_it() {
    let mut said = Vec::new();
    for (reader, actor) in [
        (Some(MARA), Some(MARA)),
        (Some(JONAS), Some(MARA)),
        (Some(MARA), Some(9)),
        (None, None),
    ] {
        let mut run = Run::new();
        run.host
            .read_as(reader.map(|reader| run.host.entity(reader)));
        run.set_cell((10, 3), BlockState::NONE);
        run.scene.push_events([broken(T, (10, 3), actor)]);
        run.until_idle();
        said.extend(run.lines());
        run.check();
    }
    assert_eq!(
        said,
        [
            "You break the wall.",
            "Mara breaks the wall.",
            "Someone breaks the wall.",
            "A wall breaks apart.",
        ]
    );
}

#[test]
fn what_is_heard_behind_a_wall_is_a_marker_and_a_line_and_changes_no_cell() {
    let mut run = Run::new();
    run.host.read_as(Some(run.host.entity(MARA)));
    let behind = (12, 3);
    let before = *run.model().cell(at(behind.0, behind.1)).unwrap();
    assert!(!before.seen());
    // Something breaks in the strip the party remembers and does not
    // see, within hearing; and far away, out of it.
    run.scene.push_events([
        broken(T, behind, Some(STRANGER)),
        broken(T + 10, (60, 3), Some(STRANGER)),
    ]);
    assert_eq!(run.scene.playback().pending(), 1, "the far one is dropped");
    run.turns(1);
    assert_eq!(run.lines(), ["Something breaks nearby."]);
    let heard: Vec<_> = run.model().heard().copied().collect();
    assert_eq!(heard.len(), 1);
    assert_eq!(heard[0].loc, at(behind.0, behind.1));
    assert_eq!(heard[0].ch, Some('!'));
    assert_eq!(run.ch(behind), '!');
    assert_eq!(*run.model().cell(at(behind.0, behind.1)).unwrap(), before);
    assert_golden(golden("playback_heard"), run.shown());
    run.check();

    // The marker is shown for a second and a half.
    run.turns(80);
    assert_eq!(run.model().heard().count(), 1);
    run.turns(15);
    assert_eq!(run.model().heard().count(), 0);
    assert_eq!(run.ch(behind), '.');
}

#[test]
fn instant_playback_applies_a_whole_batch_at_once_and_draws_what_slow_playback_ends_with() {
    let batch = |run: &mut Run| {
        let mut batch = wall_punch(run);
        run.set_look(STRANGER, Some(creature('s', (8, 4))));
        batch.push(moved(T + 900_000, STRANGER, STRANGER_AT, (8, 4)));
        batch
    };
    let mut slow = Run::new();
    let events = batch(&mut slow);
    slow.scene.push_events(events);
    let turns = slow.until_idle();
    assert!(turns > 20, "{turns}");

    let mut instant = Run::new();
    instant
        .scene
        .set_pref("ui:playback_speed", Value::Int(0))
        .unwrap();
    let events = batch(&mut instant);
    instant.scene.push_events(events);
    instant.turns(1);
    assert!(instant.scene.playback().is_idle());
    assert_eq!(instant.lines(), slow.lines());
    assert_eq!(instant.shown(), slow.shown());
    assert!(instant.shown().sprites.is_empty());
    instant.check();
    slow.check();
}

#[test]
fn repeated_lines_are_folded_with_a_count() {
    let mut run = Run::new();
    let mut batch = Vec::new();
    for x in 1..=3 {
        run.set_cell((x, 6), BlockState::NONE);
        batch.push(destroyed(T + i64::from(x), (x, 6), None, Destruction::Fell));
    }
    run.scene.push_events(batch);
    run.until_idle();
    assert_eq!(run.lines(), ["Debris crashes down. (x3)"]);
    let log = run.scene.log();
    assert_eq!(log.lines()[0].count, 3);
    assert_eq!(log.lines()[0].tick, Some(Tick(T + 3)));
    let frame = run.shown();
    assert!(
        (0..frame.rows).any(|row| frame.row_text(row).contains("Debris crashes down. (x3)")),
        "{}",
        frame.text()
    );
    run.check();
}

#[test]
fn a_key_skips_to_the_end_unless_the_keymap_names_one_for_it() {
    let mut run = Run::new();
    let batch = wall_punch(&mut run);
    run.scene.push_events(batch);
    run.turns(1);
    assert!(!run.scene.playback_caught_up());
    // No key is named `skip`, so any key skips, and then does what it
    // does: this one nothing.
    run.press("f11");
    assert!(run.scene.playback().is_idle());
    assert_eq!(run.lines().len(), 1);
    assert_eq!(run.ch((4, 0)), '.');
    assert_eq!(run.ch((4, 1)), '@');
    run.check();

    // Once a key is named `skip`, only it skips, and it does nothing
    // else.
    run.scene.rebind("play", "f9", "skip").unwrap();
    run.set_look(MARA, Some(creature('@', (4, 2))));
    run.scene.push_events([moved(T + 10, MARA, (4, 1), (4, 2))]);
    run.turns(1);
    assert!(!run.scene.playback().is_idle());
    run.press("f11");
    assert!(!run.scene.playback().is_idle(), "not the skip key");
    run.press("f9");
    assert!(run.scene.playback().is_idle());
    assert_eq!(run.scene.top().unwrap().id(), "ui:play");
    run.check();

    // And the application may skip by itself.
    run.set_look(MARA, Some(creature('@', (4, 3))));
    run.scene.push_events([moved(T + 20, MARA, (4, 2), (4, 3))]);
    run.scene.skip_playback();
    assert!(run.scene.playback().is_idle());
    run.check();
}

#[test]
fn bindings_show_the_moment_playback_has_reached() {
    let mut run = Run::new();
    let batch = wall_punch(&mut run);
    run.scene.push_events(batch);
    assert_eq!(run.scene.playback_moment(), Some(Tick(T - 1)));
    run.turns(1);
    // The step is played: the host is asked as of just before the blow.
    assert_eq!(run.scene.playback_moment(), Some(Tick(T + 2)));
    assert_eq!(run.host.moments().last(), Some(&Some(Tick(T + 2))));
    run.until_idle();
    assert_eq!(run.scene.playback_moment(), None);
    assert_eq!(run.host.moments().last(), Some(&None));
}

#[test]
fn a_decision_is_asked_for_once_playback_reaches_it() {
    let mut run = Run::new();
    let batch = wall_punch(&mut run);
    run.scene.push_events(batch);
    run.host.ask(Some(PendingDecision {
        entity: clash_expr::Value::Subject(run.host.entity(MARA)),
        kind: DecisionAsk::Choice {
            choice: clash_expr::Value::Int(4),
            prompt: "The way north is open.".to_owned(),
            options: vec!["Go through".to_owned(), "Wait".to_owned()],
        },
    }));
    run.turns(1);
    assert_eq!(run.scene.top().unwrap().id(), "ui:play", "not yet");
    run.until_idle();
    assert_eq!(run.scene.top().unwrap().id(), "ui:choice");
    assert!(run.shown().text().contains("The way north is open."));
}

#[test]
fn the_player_may_turn_the_effects_off_and_keep_the_lines() {
    let mut run = Run::new();
    run.host.read_as(Some(run.host.entity(MARA)));
    run.scene
        .set_pref("ui:effects", Value::Str("none".into()))
        .unwrap();
    let batch = wall_punch(&mut run);
    run.scene.push_events(batch);
    run.turns(1);
    // No glide: Mara is in her new cell at once.
    assert!(run.shown().sprites.is_empty());
    assert_eq!(run.ch((4, 1)), '@');
    // The stretch takes no time when nothing in it is animated but the
    // line: what shows comes in one group after another, at once.
    run.until_idle();
    assert_eq!(run.lines(), ["You break the wall."]);
    assert_eq!(run.ch((4, 0)), '.');
    run.check();
}

#[test]
fn sound_cues_are_handed_on_and_played_by_nobody() {
    let mut run = Run::new();
    let batch = wall_punch(&mut run);
    run.scene.push_events(batch);
    run.until_idle();
    let cues = run.scene.take_sound_cues();
    let names: Vec<&str> = cues.iter().map(|cue| cue.name.as_str()).collect();
    assert_eq!(names, ["step", "break"]);
    assert_eq!(cues[1].at, Some(at(4, 0)));
    assert!(!cues[1].heard);
    assert!(run.scene.take_sound_cues().is_empty());
}

#[test]
fn someone_thrown_goes_flying_and_lands_where_the_party_sees_them() {
    let mut run = Run::new();
    run.host.read_as(Some(run.host.entity(MARA)));
    let jonas = id(JONAS);
    let body = BodyId::new(1);
    let owner = BodyOwner::Entity(jonas);
    let zero = [Real::ZERO; 3];
    run.set_look(JONAS, Some(creature('j', (6, 5))));
    run.scene.push_events([
        event(
            T,
            Cause::DEBUG,
            EventBody::World(WorldEvent::BodyLaunched {
                tick: Tick(T),
                body,
                owner,
                loc: at(JONAS_AT.0, JONAS_AT.1),
                pos: zero,
                vel: zero,
                mass: Real::ONE,
                cause: Cause::DEBUG,
            }),
        ),
        event(
            T + 400_000,
            Cause::DEBUG,
            EventBody::World(WorldEvent::BodyLanded {
                tick: Tick(T + 400_000),
                body,
                owner,
                loc: at(6, 5),
                pos: zero,
                cause: Cause::DEBUG,
            }),
        ),
    ]);
    run.turns(1);
    assert_eq!(run.lines(), ["Jonas goes flying."]);
    // Dust where he was, and his sprite still there until he lands.
    assert_eq!(run.ch(JONAS_AT), ':');
    assert_eq!(
        run.model().sprite(SpriteId::new(JONAS)).unwrap().loc,
        at(JONAS_AT.0, JONAS_AT.1)
    );
    run.until_idle();
    assert_eq!(run.lines(), ["Jonas goes flying.", "Jonas lands hard."]);
    assert_eq!(run.ch((6, 5)), 'j');
    run.check();
}

/// A test mod's shows: a bolt that flies from whoever broke a wall to
/// it, and a line about the stranger's moves written for the log's
/// filter by importance.
const BOLT: &str = r#"[
  { "type": "anim", "id": "bolt", "time": "200 ms", "travel": true,
    "frames": [ { "ch": "-", "fg": "amber" }, { "at": 0.5, "ch": "=", "fg": "white" } ] },
  { "type": "show", "id": "bolt", "on": "element_destroyed",
    "filter": { "how": "damage" }, "if": "not is_null(actor)",
    "anim": "bolt", "at": "actor", "to": "here", "min_time": "200 ms", "importance": "major" }
]"#;

#[test]
fn a_projectile_is_drawn_along_its_path_over_frames() {
    let mut run = Run::with(&[("bolt", BOLT)]);
    // The stranger, three cells from the east wall, breaks it.
    run.set_cell((10, 3), BlockState::NONE);
    run.scene.push_events([broken(T, (10, 3), Some(STRANGER))]);
    run.turns(1);
    let path = |run: &Run| {
        run.shown()
            .sprites
            .iter()
            .find(|sprite| sprite.layer == 3)
            .map(|sprite| (sprite.ch, sprite.x))
    };
    let (ch, start) = path(&run).expect("the bolt");
    assert_eq!(ch, '-');
    run.turns(7);
    assert_golden(golden("playback_bolt"), run.shown());
    let (ch, middle) = path(&run).expect("the bolt");
    assert_eq!(ch, '=');
    assert!(middle > start + 1.0, "{start} then {middle}");
    run.until_idle();
    assert!(path(&run).is_none());
    run.check();
}

// ---------------------------------------------------------------------
// Bad data
// ---------------------------------------------------------------------

#[test]
fn a_show_that_is_wrong_is_reported_where_it_is_written() {
    let bad = r#"[
      { "type": "anim", "id": "bolt", "time": "200 ms", "travel": true, "frames": [ { "ch": "-" } ] },
      { "type": "show", "id": "a", "on": "element_destroied" },
      { "type": "show", "id": "b", "on": "element_destroyed", "change": [ { "cell": "hear" } ] },
      { "type": "show", "id": "c", "on": "element_destroyed", "filter": { "layr": "block" } },
      { "type": "show", "id": "d", "on": "element_destroyed", "anim": "bust" },
      { "type": "show", "id": "e", "on": "element_destroyed", "anim": "bolt" },
      { "type": "show", "id": "f", "on": "element_destroyed", "flash": "octarine" },
      { "type": "show", "id": "g", "on": "element_destroyed", "log": "{name(here.block)}" },
      { "type": "show", "id": "h", "on": "element_destroyed", "heard": { "marker": "!!" } }
    ]"#;
    let found = match try_load(&[("bad", bad)]) {
        Ok(_) => panic!("the mod should not load"),
        Err(diagnostics) => diagnostics.render(),
    };
    for expected in [
        "there is no event `element_destroied`",
        "did you mean `element_destroyed`?",
        "the event `element_destroyed` binds no subject `hear`",
        "the event `element_destroyed` has no field `layr`",
        "there is no anim `bust`",
        "this show does not say where `to`",
        "data/extra.json",
    ] {
        assert!(found.contains(expected), "`{expected}` in:\n{found}");
    }
    // What the loader finds stops the load before the checks after it
    // run: those are found once the rest is right.
    let later = r#"[
      { "type": "show", "id": "f", "on": "element_destroyed", "flash": "octarine" },
      { "type": "show", "id": "h", "on": "element_destroyed", "heard": { "marker": "!!" } },
      { "type": "show", "id": "g", "on": "element_destroyed", "log": "{name(here.block)}" }
    ]"#;
    let found = match try_load(&[("later", later)]) {
        Ok(_) => panic!("the mod should not load"),
        Err(diagnostics) => diagnostics.render(),
    };
    for expected in ["expected a character", "`loc.block` is not safe for belief"] {
        assert!(found.contains(expected), "`{expected}` in:\n{found}");
    }
    let colour =
        r#"[ { "type": "show", "id": "f", "on": "element_destroyed", "flash": "octarine" } ]"#;
    let found = match try_load(&[("colour", colour)]) {
        Ok(_) => panic!("the mod should not load"),
        Err(diagnostics) => diagnostics.render(),
    };
    assert!(found.contains("unknown colour `octarine`"), "{found}");
    let sound =
        r#"[ { "type": "show", "id": "s", "on": "element_destroyed", "sound": "a bang!" } ]"#;
    let found = match try_load(&[("sound", sound)]) {
        Ok(_) => panic!("the mod should not load"),
        Err(diagnostics) => diagnostics.render(),
    };
    assert!(found.contains("is not a sound cue's name"), "{found}");
}

// ---------------------------------------------------------------------
// Caught-up playback equals a fresh view (invariant V4)
// ---------------------------------------------------------------------

/// One thing that can happen in the room.
#[derive(Clone, Debug)]
enum Happening {
    /// A cell of the room gains or loses its wall.
    Wall { cell: (i32, i32), stands: bool },
    /// A thing walks to a cell, which may be out of sight.
    Walk { who: u64, to: (i32, i32) },
    /// A new thing comes into the room.
    Arrive { who: u64, to: (i32, i32) },
    /// A thing ceases to exist.
    Vanish { who: u64 },
    /// Something breaks behind the east wall, in the strip the party
    /// only remembers.
    Behind { cell: (i32, i32) },
}

fn happening() -> impl Strategy<Value = Happening> {
    let cell = (1..10i32, 1..6i32);
    prop_oneof![
        (cell.clone(), any::<bool>()).prop_map(|(cell, stands)| Happening::Wall { cell, stands }),
        (1..4u64, (1..13i32, 1..6i32)).prop_map(|(who, to)| Happening::Walk { who, to }),
        (4..7u64, cell.clone()).prop_map(|(who, to)| Happening::Arrive { who, to }),
        (2..7u64).prop_map(|who| Happening::Vanish { who }),
        (11..14i32, 0..7i32).prop_map(|(x, y)| Happening::Behind { cell: (x, y) }),
    ]
}

/// The gaps between events, in ticks: none, a few, a second, an hour.
fn gap() -> impl Strategy<Value = i64> {
    prop_oneof![
        Just(0i64),
        Just(1),
        Just(3),
        Just(250_000),
        Just(1_000_000),
        Just(3_600_000_000),
    ]
}

/// Makes the events of what happened and changes the truth to match.
fn simulate(run: &mut Run, happenings: &[(Happening, i64)], tick: &mut i64) -> Vec<SimEvent> {
    let mut events = Vec::new();
    for (happening, gap) in happenings {
        *tick += gap;
        let tick = *tick;
        match *happening {
            Happening::Wall { cell, stands } => {
                let block = if stands { run.wall } else { BlockState::NONE };
                run.set_cell(cell, block);
                let event = if stands {
                    event(
                        tick,
                        Cause::DEBUG,
                        EventBody::World(WorldEvent::ElementChanged {
                            tick: Tick(tick),
                            loc: at(cell.0, cell.1),
                            layer: Layer::Block,
                            old: ElementState::NONE,
                            new: ElementState::NONE,
                            cause: Cause::DEBUG,
                        }),
                    )
                } else {
                    broken(tick, cell, None)
                };
                events.push(event);
            }
            Happening::Walk { who, to } => {
                let Some(Some(was)) = run.truth.looks.get(&who).copied() else {
                    continue;
                };
                let from = (was.loc.cell.x, was.loc.cell.y);
                // Past the east wall it walks out of sight.
                let look = (to.0 <= 10).then_some(EntitySprite {
                    loc: at(to.0, to.1),
                    ..was
                });
                run.set_look(who, look);
                events.push(moved(tick, who, from, to));
            }
            Happening::Arrive { who, to } => {
                if run.truth.looks.contains_key(&who) {
                    continue;
                }
                let ch = char::from(b'a' + who as u8);
                run.set_look(who, Some(creature(ch, to)));
                events.push(event(
                    tick,
                    Cause::DEBUG,
                    EventBody::EntityCreated { entity: id(who) },
                ));
                events.push(event(
                    tick,
                    Cause::DEBUG,
                    EventBody::EntityRelocated {
                        entity: id(who),
                        from: Place::Stored,
                        to: Place::at(at(to.0, to.1)),
                    },
                ));
            }
            Happening::Vanish { who } => {
                let Some(Some(_)) = run.truth.looks.get(&who).copied() else {
                    continue;
                };
                // Known still, by the number of its sprite; perceived no
                // more.
                run.set_look(who, None);
                events.push(event(
                    tick,
                    Cause::DEBUG,
                    EventBody::EntityRemoved { entity: id(who) },
                ));
            }
            Happening::Behind { cell } => {
                events.push(broken(tick, cell, Some(STRANGER)));
            }
        }
    }
    events
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(48))]

    /// Whatever happens, in batches played at any pace, once playback
    /// has caught up the picture is what a fresh view would be.
    #[test]
    fn caught_up_playback_equals_a_fresh_view(
        batches in prop::collection::vec(prop::collection::vec((happening(), gap()), 0..12), 1..4),
        steps in prop::collection::vec(prop_oneof![Just(0.0), Just(STEP), Just(0.1), Just(0.7)], 1..8),
        speed in prop_oneof![Just(0i64), Just(1), Just(4)],
    ) {
        let mut run = Run::new();
        run.scene.set_pref("ui:playback_speed", Value::Int(speed)).unwrap();
        let mut tick = T;
        for batch in &batches {
            let events = simulate(&mut run, batch, &mut tick);
            run.scene.push_events(events);
            // Played at uneven steps; skipped if it has not caught up by
            // then.
            for seconds in &steps {
                run.scene.advance(*seconds);
            }
            if !run.scene.playback_caught_up() {
                let mut turns = 0;
                while !run.scene.playback_caught_up() {
                    run.scene.advance(STEP);
                    turns += 1;
                    prop_assert!(turns < 1200, "playback never caught up");
                }
            }
            let fresh = run.fresh();
            if let Err(drift) = check_caught_up(run.model(), &fresh) {
                prop_assert!(false, "{}", drift);
            }
        }
    }
}

#[test]
fn the_check_finds_a_picture_that_drifted() {
    let mut run = Run::new();
    // The simulation says the wall is gone, and nothing was played.
    run.set_cell((4, 0), BlockState::NONE);
    let drift = run.scene.check_caught_up(&run.fresh()).unwrap_err();
    assert_eq!(drift.differences.len(), 1);
    assert!(!drift.more);
    let text = drift.to_string();
    assert!(text.contains("cell (4, 0, 0)"), "{text}");
    // An importance is ordered: a log can keep only what matters.
    assert!(Importance::Minor < Importance::Major);
}
