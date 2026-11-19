//! What the play sessions share (`tests/play_*.rs`): a game booted from
//! the title as a player starts one, keys pressed one at a time, and
//! reads of what the game holds for the sessions' assertions.
//!
//! The sessions are driven by keys alone. Where a session crosses a
//! town, the way is the one the character believes leads there
//! (`clash_sim::movement::believed_path`, the same search its own
//! `go_to` makes), turned into the direction keys a player would press
//! while reading the map; nothing is moved by a debug command.

#![allow(dead_code, reason = "each session uses some of these")]

use std::path::{Path, PathBuf};

use clash_app::boot;
use clash_app::cli::Options;
use clash_app::game::{BootOptions, Game};
use clash_app::headless::Headless;
use clash_core::Dir8;
use clash_core::EntityId;
use clash_expr::Value;
use clash_scene::AppList;
use clash_sim::Subject;
use clash_world::{CellPos, Location, PathLimits, StepKind};

pub fn manifest() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

pub fn mods_dir() -> PathBuf {
    manifest().join("../../mods")
}

pub fn golden(name: &str) -> PathBuf {
    manifest()
        .join("tests/golden")
        .join(format!("{name}.frame"))
}

/// A fresh data directory for one session, removed when it is dropped.
pub struct DataDir(pub PathBuf);

impl DataDir {
    pub fn new(name: &str) -> DataDir {
        let data = std::env::temp_dir().join(format!("clash-play-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&data);
        DataDir(data)
    }

    pub fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for DataDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// The bundled profile with the seed the new-game screen offers, its
/// title on show.
pub fn boot_default(data: &Path, seed: u64) -> Headless {
    let options = Options {
        mods: Some(mods_dir()),
        data: Some(data.to_path_buf()),
        seed,
        ..Options::default()
    };
    Headless::new(clash_app::boot(&options).unwrap_or_else(|problem| panic!("{problem}")))
}

/// The `armoury` fixture: the bundled mods and two games of its own,
/// `armoury:gifted` (someone with every power of the sampler) and
/// `armoury:range`, first in the list of games.
pub fn boot_armoury(data: &Path) -> Headless {
    let fixture = manifest().join("tests/fixtures/armoury");
    let search = [fixture.join("mods"), mods_dir()];
    let profile = fixture.join("profiles/armoury.json");
    let loaded = boot::load(&search, &profile.to_string_lossy())
        .unwrap_or_else(|problem| panic!("{problem}"));
    let game = Game::boot(
        loaded,
        BootOptions {
            data_dir: Some(data.to_path_buf()),
            ..BootOptions::default()
        },
    )
    .unwrap_or_else(|problem| panic!("{problem}"));
    Headless::new(game)
}

/// Runs lines of a headless script (12 §3.3); the game must still run.
pub fn script(run: &mut Headless, text: &str) {
    run.run_script(text, &mut std::io::sink())
        .unwrap_or_else(|problem| panic!("{problem}"));
    assert!(run.running(), "the game ended in the script");
}

/// Presses one key.
pub fn press(run: &mut Headless, key: &str) {
    run.key(key).unwrap_or_else(|problem| panic!("{problem}"));
    assert!(run.running(), "the game ended on `{key}`");
}

/// The id of the screen on top.
pub fn top(run: &Headless) -> String {
    run.game()
        .scene()
        .top()
        .map(|screen| screen.id().to_owned())
        .unwrap_or_default()
}

/// Passes until the screen on top is the one named, or panics.
pub fn until_top(run: &mut Headless, screen: &str, passes: u32) {
    for _ in 0..passes {
        if top(run) == screen {
            return;
        }
        run.pass();
    }
    panic!(
        "`{screen}` never came to the top; `{}` is there\n{}",
        top(run),
        run.text()
    );
}

/// Passes with no input until playback has nothing left to play.
pub fn settle(run: &mut Headless) {
    for _ in 0..900 {
        if run.game().scene().playback().is_idle() {
            return;
        }
        run.pass();
    }
    panic!("playback never came to rest");
}

/// From the title: a new game of the game at a place in the list, with
/// the character named. The list is in order of qualified id:
/// `last_stand`, `the_errand`, `town`, `town_life`, `wayside`.
pub fn new_game(run: &mut Headless, index: usize, name: &str) {
    assert_eq!(top(run), "ui:title");
    script(run, "enter\n");
    script(run, &format!("type {name}\n"));
    script(run, "tab\ntab\n");
    for _ in 0..index {
        script(run, "down\n");
    }
    script(run, "enter\n");
}

/// The log as (category, plain text) lines.
pub fn log(run: &Headless) -> Vec<(String, String)> {
    run.game()
        .scene()
        .log()
        .lines()
        .iter()
        .map(|line| (line.category.clone(), line.text.plain()))
        .collect()
}

/// The text of the log's lines.
pub fn log_text(run: &Headless) -> Vec<String> {
    log(run).into_iter().map(|(_, text)| text).collect()
}

/// How many lines the log holds now: to look only at what follows.
pub fn log_len(run: &Headless) -> usize {
    run.game().scene().log().lines().len()
}

/// The log's lines from a point on.
pub fn log_since(run: &Headless, from: usize) -> Vec<String> {
    log_text(run).into_iter().skip(from).collect()
}

pub fn player(run: &Headless) -> EntityId {
    run.game()
        .with_session(|session| session.player)
        .expect("a game is being played")
}

pub fn player_loc(run: &Headless) -> Option<Location> {
    run.game()
        .with_session(|session| session.player_loc())
        .flatten()
}

/// The simulation's universal time, in seconds.
pub fn now_s(run: &Headless) -> f64 {
    run.game()
        .with_session(|session| session.sim.now().as_micros() as f64 / 1e6)
        .expect("a game is being played")
}

/// The time of day where the character stands, in seconds since
/// midnight, as its sky tells it.
pub fn time_of_day(run: &Headless) -> f64 {
    run.game()
        .with_session(|session| {
            let at = session.player_loc()?;
            let value = session
                .sim
                .property_named(Subject::Loc(at), "time_of_day", &[]);
            value.as_number().map(|real| real.get())
        })
        .flatten()
        .expect("the character stands under a sky")
}

/// A global variable of the game.
pub fn global(run: &Headless, name: &str) -> Value {
    run.game()
        .with_session(|session| session.sim.var(Subject::Global, name))
        .expect("a game is being played")
}

/// The ids of `app.saves`.
pub fn saves(run: &Headless) -> Vec<String> {
    run.game()
        .scene()
        .app()
        .entries(AppList::Saves)
        .into_iter()
        .map(|entry| entry.id)
        .collect()
}

/// The state hash of the simulation.
pub fn state_hash(run: &Headless) -> clash_core::Hash128 {
    run.game()
        .with_session(|session| session.sim.state_hash())
        .expect("a game is being played")
}

/// How many commands the simulation has taken.
pub fn commands(run: &Headless) -> usize {
    run.game()
        .with_session(|session| session.sim.input_log().len())
        .unwrap_or_default()
}

/// How many times the simulation was told to start an action whose
/// qualified id is the one given, by `Act`.
pub fn acts_of(run: &Headless, action: &str) -> usize {
    run.game()
        .with_session(|session| {
            let actions = session
                .sim
                .defs()
                .try_registry::<clash_sim::action::ActionDef>()?;
            let count = session
                .sim
                .input_log()
                .entries()
                .iter()
                .filter(|entry| match &entry.command {
                    clash_sim::Command::Act { action: id, .. } => actions.name(*id) == action,
                    _ => false,
                })
                .count();
            Some(count)
        })
        .flatten()
        .unwrap_or_default()
}

/// Where the character perceives its target to be: the target's sprite
/// in the picture the player is shown.
pub fn target_loc(run: &Headless) -> Option<Location> {
    let scene = run.game().scene();
    let id = scene.target()?;
    scene.world()?.model().sprite(id).map(|sprite| sprite.loc)
}

/// Tab until someone is in sight and selected, a second going by
/// between tries, at most `tries` times. Returns whether someone is.
pub fn tab_until_someone(run: &mut Headless, tries: usize) -> bool {
    for _ in 0..tries {
        press(run, "tab");
        if run.game().scene().target().is_some() {
            return true;
        }
        press(run, ".");
    }
    false
}

/// Walks up to the target, following where the character sees it, a
/// step at a time, until it stands beside it or `max_keys` keys have
/// been pressed. Returns whether it is beside the target.
pub fn walk_up_to_target(run: &mut Headless, max_keys: usize) -> bool {
    for _ in 0..max_keys {
        // Lost from sight: Tab again for whoever is in sight now.
        if target_loc(run).is_none() {
            press(run, "tab");
        }
        let (Some(at), Some(to)) = (player_loc(run), target_loc(run)) else {
            press(run, ".");
            continue;
        };
        let delta = to.cell - at.cell;
        if delta.x.abs().max(delta.y.abs()) <= 1 && delta.z == 0 {
            return true;
        }
        if walk_to(run, to, 1, 1) == 0 || top(run) != "ui:play" {
            press(run, ".");
        }
    }
    false
}

/// The quick attack (`f`) at the target, as a player fights: a foe out
/// of reach is walked up to first, a step at a time along the way the
/// character believes leads there, at most `max_steps` keys, and struck
/// once beside it. A choice the attack asks for is put away. Returns
/// whether anything was done: a blow started, a step taken toward the
/// foe, or a prompt come up that the session must answer; false when
/// nothing could be struck and no way leads to the foe.
pub fn strike_target(run: &mut Headless, max_steps: usize) -> bool {
    let quick = |run: &mut Headless| {
        let before = commands(run);
        press(run, "f");
        if !["ui:play", "ui:reaction", "ui:summary"].contains(&top(run).as_str()) {
            press(run, "escape");
        }
        commands(run) != before || top(run) != "ui:play"
    };
    if quick(run) {
        return true;
    }
    // Out of reach: up to them, if the character knows a way there.
    let Some(to) = target_loc(run) else {
        return false;
    };
    let reachable = believed_way(run, to).is_ok()
        || StepKind::ALL[..8]
            .iter()
            .any(|kind| believed_way(run, Location::new(to.grid, to.cell + kind.delta())).is_ok());
    if !reachable {
        return false;
    }
    let before = player_loc(run);
    walk_up_to_target(run, max_steps);
    if top(run) != "ui:play" {
        return true;
    }
    if quick(run) {
        return true;
    }
    // Steps were taken toward them even if no blow was struck yet.
    player_loc(run) != before
}

/// Where a thing of a definition (`core:club`) lies loose within some
/// cells of a place, if one does: what a player sees lying about.
pub fn lying_near(run: &Headless, at: Location, def: &str, radius: i32) -> Option<Location> {
    run.game()
        .with_session(|session| {
            let sim = &session.sim;
            let items = sim.defs().try_registry::<clash_sim::item::ItemDef>()?;
            sim.truth()
                .entities()
                .iter()
                .filter(|entity| matches!(entity.place(), clash_sim::Place::At { .. }))
                .filter(|entity| {
                    sim.components()
                        .get::<clash_sim::item::Item>(entity.id())
                        .and_then(|item| item.def())
                        .is_some_and(|id| items.name(id) == def)
                })
                .filter_map(|entity| sim.truth().location(entity.id()))
                .find(|loc| {
                    loc.grid == at.grid
                        && loc.cell.z == at.cell.z
                        && (loc.cell.x - at.cell.x).abs() <= radius
                        && (loc.cell.y - at.cell.y).abs() <= radius
                })
        })
        .flatten()
}

/// Whether the target the character has selected is dead, as the log
/// told the player when it died.
pub fn target_dead(run: &Headless) -> bool {
    let Some(sprite) = run.game().scene().target() else {
        return false;
    };
    let entity = EntityId::from_bits(sprite.raw());
    run.game()
        .with_session(|session| session.sim.vital(entity))
        .flatten()
        == Some(clash_sim::mind::VitalState::Dead)
}

/// On the reaction prompt: the first of the reactions named, in the
/// order of preference given, that is offered and not too late; with
/// none, the blow is let happen. Returns what was chosen.
pub fn react(run: &mut Headless, preferred: &[&str]) -> String {
    let text = run.text();
    let rows: Vec<String> = text
        .lines()
        .skip_while(|line| !line.contains("Time  "))
        .skip(1)
        .take_while(|line| !line.contains("[ Let it happen ]"))
        .filter_map(|line| {
            let start = line.find('│')?;
            let rest = &line[start + '│'.len_utf8()..];
            let end = rest.find('│')?;
            Some(rest[..end].trim().trim_end_matches('█').trim().to_owned())
        })
        .collect();
    for want in preferred {
        if let Some(index) = rows
            .iter()
            .position(|row| row.starts_with(want) && !row.contains("too late"))
        {
            for _ in 0..index {
                press(run, "down");
            }
            press(run, "enter");
            return (*want).to_owned();
        }
    }
    press(run, "escape");
    "nothing".to_owned()
}

/// The key of the default keymap that steps one way.
pub fn key_of(kind: StepKind) -> &'static str {
    match kind {
        StepKind::Level(Dir8::North) => "k",
        StepKind::Level(Dir8::NorthEast) => "u",
        StepKind::Level(Dir8::East) => "l",
        StepKind::Level(Dir8::SouthEast) => "n",
        StepKind::Level(Dir8::South) => "j",
        StepKind::Level(Dir8::SouthWest) => "b",
        StepKind::Level(Dir8::West) => "h",
        StepKind::Level(Dir8::NorthWest) => "y",
        StepKind::Up => "<",
        StepKind::Down => ">",
    }
}

/// The cell a direction key steps into from a cell, on its storey.
fn cell_toward(from: CellPos, key: &str) -> Option<CellPos> {
    StepKind::ALL[..8]
        .iter()
        .map(|kind| from + kind.delta())
        .find(|to| key_between(from, *to) == Some(key))
}

/// The key that steps from one cell to the next, if they touch.
fn key_between(from: CellPos, to: CellPos) -> Option<&'static str> {
    let delta = to - from;
    StepKind::ALL
        .into_iter()
        .find(|kind| kind.delta() == delta)
        .map(key_of)
}

/// The keys of the way the character believes leads from where it
/// stands to a place: the path its own `go_to` would plan.
pub fn believed_way(run: &Headless, to: Location) -> Result<Vec<&'static str>, String> {
    believed_way_round(run, to, &[])
}

/// [`believed_way`], round cells the walker has found someone standing
/// in.
fn believed_way_round(
    run: &Headless,
    to: Location,
    avoid: &[CellPos],
) -> Result<Vec<&'static str>, String> {
    run.game()
        .with_session(|session| {
            let from = session
                .player_loc()
                .ok_or("the character is not in the world")?;
            let path = clash_sim::movement::believed_path(
                &session.sim,
                session.player,
                from,
                to,
                avoid,
                PathLimits::DEFAULT,
            )
            .map_err(|fail| format!("no way from {from:?} to {to:?}: {fail:?}"))?;
            path.cells()
                .windows(2)
                .map(|pair| {
                    key_between(pair[0], pair[1]).ok_or_else(|| {
                        format!("the path jumps from {:?} to {:?}", pair[0], pair[1])
                    })
                })
                .collect()
        })
        .unwrap_or_else(|| Err("no game is being played".to_owned()))
}

/// Walks with the direction keys toward a place, a step at a time, the
/// way the character believes leads there, until it stands within
/// `near` cells of it (on its storey) or `max_keys` keys are pressed.
/// A step that does not move the character (a door opened by walking
/// into it) is tried again after the way is looked for afresh; someone
/// standing in the way is walked round, as a player would, for the next
/// few keys. Returns how many keys were pressed.
pub fn walk_to(run: &mut Headless, to: Location, near: i32, max_keys: usize) -> usize {
    /// How many keys a cell someone stood in is walked round for.
    const ROUND_FOR: usize = 6;
    let mut pressed = 0;
    // Cells someone stood in, and the key count until which they are
    // walked round.
    let mut round: Vec<(CellPos, usize)> = Vec::new();
    while pressed < max_keys {
        round.retain(|(_, until)| *until > pressed);
        let avoid: Vec<CellPos> = round.iter().map(|(cell, _)| *cell).collect();
        let at = player_loc(run).expect("the character is in the world");
        let delta = to.cell - at.cell;
        if delta.x.abs().max(delta.y.abs()) <= near && delta.z == 0 {
            return pressed;
        }
        // A place one cannot stand in (a shut door, a wall) is walked up
        // to: the way to the nearest cell beside it that one can.
        let way = believed_way_round(run, to, &avoid)
            .or_else(|_| believed_way(run, to))
            .or_else(|problem| {
                StepKind::ALL[..8]
                    .iter()
                    .filter_map(|kind| {
                        let beside = Location::new(to.grid, to.cell + kind.delta());
                        believed_way_round(run, beside, &avoid)
                            .or_else(|_| believed_way(run, beside))
                            .ok()
                    })
                    .min_by_key(Vec::len)
                    .ok_or(problem)
            });
        let way = way.unwrap_or_else(|problem| panic!("{problem}"));
        let Some(key) = way.first() else {
            return pressed;
        };
        press(run, key);
        pressed += 1;
        // Walking into someone in the way asks whether to strike them:
        // no, and the way is looked for round the cell they stand in.
        // Bumping into the same cell again while walking round it means
        // there is no way round (a doorway): wait a second for them to
        // move on, as a player would, rather than press keys that take
        // no time at all.
        if top(run) == "ui:confirm" {
            press(run, "escape");
            if let Some(cell) = cell_toward(at.cell, key) {
                if round.iter().any(|(stood, _)| *stood == cell) {
                    press(run, ".");
                }
                round.retain(|(stood, _)| *stood != cell);
                round.push((cell, pressed + ROUND_FOR));
            }
        }
        // Anything else on top of the play screen (a reaction prompt)
        // stops the walk for the session to answer.
        if top(run) != "ui:play" {
            return pressed;
        }
    }
    pressed
}

/// The rows of the list a popup shows under its title, as written: the
/// lines between the line holding the title and the frame's foot.
pub fn popup_rows(run: &Headless, title: &str) -> Vec<String> {
    let text = run.text();
    let mut rows = Vec::new();
    let mut inside = false;
    for line in text.lines() {
        if line.contains(title) {
            inside = true;
            continue;
        }
        if inside {
            let Some(start) = line.find('│') else { break };
            let rest = &line[start + '│'.len_utf8()..];
            let Some(end) = rest.find('│') else { break };
            let row = rest[..end].trim().to_owned();
            if row.is_empty() {
                break;
            }
            rows.push(row);
        }
    }
    rows
}

/// The buttons on show, `[ Label ]`, top to bottom and left to right.
pub fn buttons(run: &Headless) -> Vec<String> {
    let mut found = Vec::new();
    for line in run.text().lines() {
        let mut rest = line;
        while let Some(open) = rest.find("[ ") {
            let after = &rest[open + 2..];
            let Some(close) = after.find(" ]") else { break };
            found.push(after[..close].to_owned());
            rest = &after[close + 2..];
        }
    }
    found
}

/// On a screen of buttons whose focus starts on the first: moves down
/// to the one labelled so and presses Enter.
pub fn press_button(run: &mut Headless, label: &str) {
    let shown = buttons(run);
    let index = shown
        .iter()
        .position(|have| have == label)
        .unwrap_or_else(|| panic!("no button `{label}` among {shown:?}\n{}", run.text()));
    for _ in 0..index {
        press(run, "down");
    }
    press(run, "enter");
}

/// The id of the widget with the focus on the screen on top.
pub fn focus(run: &Headless) -> Option<String> {
    run.game()
        .scene()
        .top()
        .and_then(|screen| screen.focus().map(str::to_owned))
}

/// Moves down on the screen on top until the widget of that id has the
/// focus, or panics.
pub fn down_to(run: &mut Headless, id: &str) {
    for _ in 0..24 {
        if focus(run).as_deref() == Some(id) {
            return;
        }
        press(run, "down");
    }
    panic!("`{id}` was never reached on {}:\n{}", top(run), run.text());
}

/// Presses Tab on the screen on top until the widget of that id has
/// the focus, or panics.
pub fn tab_to(run: &mut Headless, id: &str) {
    for _ in 0..24 {
        if focus(run).as_deref() == Some(id) {
            return;
        }
        press(run, "tab");
    }
    panic!(
        "`{id}` never came into focus on {}:\n{}",
        top(run),
        run.text()
    );
}

/// From the title: the new-game screen with the name typed and the game
/// at a place in the list selected, F2, and the template at a place in
/// that list begun on the creation screen.
pub fn create(run: &mut Headless, game: usize, template: usize, name: &str) {
    assert_eq!(top(run), "ui:title");
    script(run, "enter\n");
    script(run, &format!("type {name}\n"));
    script(run, "tab\ntab\n");
    for _ in 0..game {
        script(run, "down\n");
    }
    press(run, "f2");
    assert_eq!(top(run), "ui:create", "{}", run.text());
    for _ in 0..template {
        press(run, "down");
    }
    press(run, "enter");
}

/// On the creation screen: Begin, then the setup and the introduction
/// that follow, to play.
pub fn begin_created(run: &mut Headless) {
    tab_to(run, "begin");
    press(run, "enter");
    until_top_any(run, &["ui:setup", "ui:intro", "ui:play"], 60);
    if top(run) == "ui:setup" {
        press(run, "enter");
        until_top_any(run, &["ui:intro", "ui:play"], 60);
    }
    if top(run) == "ui:intro" {
        press(run, "enter");
    }
    settle(run);
    assert_eq!(top(run), "ui:play", "{}", run.text());
}

/// Passes until one of the screens named is on top, or panics.
pub fn until_top_any(run: &mut Headless, screens: &[&str], passes: u32) {
    for _ in 0..passes {
        if screens.contains(&top(run).as_str()) {
            return;
        }
        run.pass();
    }
    panic!(
        "none of {screens:?} came to the top; `{}` is there\n{}",
        top(run),
        run.text()
    );
}

/// From play: the inventory, Enter on the row of what the character
/// carries at a place in its list, and the button of that id in the
/// thing's popup. Back in play once the action is chosen.
pub fn do_with_carried(run: &mut Headless, row: usize, button: &str) {
    press(run, "i");
    assert_eq!(top(run), "ui:inventory", "{}", run.text());
    for _ in 0..row {
        press(run, "down");
    }
    press(run, "enter");
    assert_eq!(top(run), "ui:item", "{}", run.text());
    down_to(run, button);
    press(run, "enter");
    if top(run) == "ui:inventory" {
        press(run, "escape");
    }
}

/// From play: the inventory, and the button of that id in the popup of
/// the first thing carried whose row begins with `name` (`club`). Back
/// in play once the action is chosen. Returns whether there was one.
pub fn do_with_named(run: &mut Headless, name: &str, button: &str) -> bool {
    press(run, "i");
    assert_eq!(top(run), "ui:inventory", "{}", run.text());
    let rows: Vec<String> = run
        .text()
        .lines()
        .skip_while(|line| !line.contains("Carried"))
        .skip(1)
        .take_while(|line| !line.contains('└'))
        .filter_map(|line| {
            let inner = line.split('│').nth(2)?;
            Some(inner.trim().to_owned())
        })
        .filter(|row| !row.is_empty())
        .collect();
    let Some(row) = rows.iter().position(|row| row.starts_with(name)) else {
        press(run, "escape");
        return false;
    };
    for _ in 0..row {
        press(run, "down");
    }
    press(run, "enter");
    assert_eq!(top(run), "ui:item", "{}", run.text());
    down_to(run, button);
    press(run, "enter");
    if top(run) == "ui:inventory" {
        press(run, "escape");
    }
    true
}

/// From play: saves the game under a name through the pause menu,
/// typing the name and pressing Enter in it.
pub fn save_as(run: &mut Headless, name: &str) {
    press(run, "escape");
    assert_eq!(top(run), "ui:pause");
    press_button(run, "Save");
    assert_eq!(top(run), "ui:save_game");
    run.type_text(name);
    press(run, "enter");
}

/// From play: leaves to the title through the pause menu.
pub fn quit_to_title(run: &mut Headless) {
    press(run, "escape");
    assert_eq!(top(run), "ui:pause");
    press_button(run, "Quit to title");
    assert_eq!(top(run), "ui:confirm");
    press(run, "enter");
    assert_eq!(top(run), "ui:title");
}

/// From play: `e`, then the first thing near that can be done whose
/// row begins with `name` (and says no reason it cannot), by moving
/// down to it and pressing Enter. Returns whether there was one; with
/// none the list is closed again.
pub fn use_near(run: &mut Headless, name: &str) -> bool {
    press(run, "e");
    assert_eq!(top(run), "ui:interact");
    let rows = popup_rows(run, "Use what is near");
    let Some(index) = rows
        .iter()
        .position(|row| row.starts_with(name) && !row.contains(':'))
    else {
        press(run, "escape");
        return false;
    };
    // Two lists: what lies loose (`Pick up …`), when anything does, and
    // then what can be done. Up and Down move within a list and Tab
    // between them, so a row of the second is reached by Tab first and
    // counted from the second's top (pressing Down from the first list's
    // top never leaves it, and Enter picked up what lay there instead).
    let loose = rows
        .iter()
        .take_while(|row| row.starts_with("Pick up "))
        .count();
    let index = if index >= loose && loose > 0 {
        press(run, "tab");
        index - loose
    } else {
        index
    };
    for _ in 0..index {
        press(run, "down");
    }
    press(run, "enter");
    true
}

/// Passes until the character is asked again (a long action is over),
/// at most `passes`.
pub fn until_asked(run: &mut Headless, passes: u32) {
    for _ in 0..passes {
        let free = run
            .game()
            .with_session(|session| session.sim.decisions().has_free(session.player))
            .unwrap_or(true);
        if free {
            return;
        }
        run.pass();
    }
}

/// The anchor of the first site of the realm whose name is the one
/// given (`Bakery`, `Town hall`): where a sign over its door would be.
pub fn site_named(run: &Headless, name: &str) -> Location {
    run.game()
        .with_session(|session| {
            let cosmos = session.sim.truth().cosmos();
            let site = cosmos
                .sites()
                .find(|site| site.name == name)
                .unwrap_or_else(|| panic!("the town has no {name}"));
            let grid = cosmos.realm(site.realm).unwrap().base_grid;
            Location::new(grid, site.anchor)
        })
        .expect("a game is being played")
}

/// A cell of the realm a site's anchor names, on the storey a walker
/// stands at in the street (the anchor's own).
pub fn site_anchor(run: &Headless, site_var: &str) -> Location {
    run.game()
        .with_session(|session| {
            let sim = &session.sim;
            let value = sim.var(Subject::Global, site_var);
            let Some(Subject::Site(site)) = sim.subject_of(&value) else {
                panic!("`{site_var}` names no site: {value:?}");
            };
            let site = sim.truth().cosmos().site(site).unwrap();
            let grid = sim.truth().cosmos().realm(site.realm).unwrap().base_grid;
            Location::new(grid, site.anchor)
        })
        .expect("a game is being played")
}
