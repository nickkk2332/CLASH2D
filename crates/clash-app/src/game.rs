//! The application, running: the scene, the game being played and the
//! loop that joins them (12 §3).
//!
//! # One pass of the loop
//!
//! [`Game::pass`] is one frame:
//!
//! 1. **Input.** What the frontend polled is handed to the scene, which
//!    runs the effects it asks for: screens open and close, commands
//!    are submitted, services are asked of the application.
//! 2. **Follow-up.** What the effects came to is acted on: a refusal or
//!    a failure gets a line in the log, a service that failed opens the
//!    error screen, a game that was started is shown, one whose play
//!    screen was left is ended.
//! 3. **The simulation.** While the player's character owes time
//!    ([`crate::shim`]), the simulation is advanced toward the moment
//!    it is free again, in steps of [`SIM_STEP`], until that moment, a
//!    decision, or the end of the pass's time slice. After each step
//!    the events are drained and shown ([`crate::present`]). Nothing
//!    else advances the simulation, and it is never advanced without
//!    its events reaching the scene in the same pass.
//! 4. **Decisions.** A decision that is waiting gets its screen
//!    (`Scene::sync_decision`).
//! 5. **The view.** If anything happened, the view model is fed with
//!    what the character sees now ([`crate::perception_shim`]).
//! 6. **The frame.** The scene is advanced by the real time elapsed,
//!    builds a frame of the frontend's size and the frontend presents
//!    it.
//!
//! The pass returns whether to go on: not once the window was closed,
//! an effect asked to quit, or the last screen was closed.
//!
//! The loop round it ([`run_frontend`]) measures real time, gives each
//! pass a slice of it to advance the simulation in and sleeps out the
//! rest of the frame. It never waits on the simulation: a pass that
//! runs out of its slice presents a frame and carries on in the next.
//!
//! # What a screen is told when it opens
//!
//! A screen that declares a variable named `creation_<field>` has it
//! set, when it opens, to that field of `app.creation`: the new-game
//! screen starts with the seed given on the command line.
//!
//! # The roles the application opens
//!
//! Beside those of 12 §10.7 the application opens the screen of the
//! role [`ROLE_PLAY`] when a game starts, and of [`ROLE_ERROR`], with
//! the parameter `text`, when a service fails. A profile that names no
//! `error` screen gets the line in the log alone.

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::rc::Rc;
use std::time::{Duration, Instant};

use clash_core::{Dur, Value};
use clash_expr::Value as ExprValue;
use clash_scene::control::ROLE_PLAY;
use clash_scene::ui::{PreferenceDef, ScreenDef, VarRef, to_var};
use clash_scene::view::{WorldGlyphs, WorldView};
use clash_scene::{
    Frame, Frontend, GlyphInputs, GlyphPart, InputEvent, KeymapDef, PREF_KEYMAP, PREF_THEME,
    RichText, Scene, Theme, UiOutcome,
};
use clash_sim::{Advance, AdvanceLimit};

use crate::app::{App, CREATION_NAME, CREATION_SEED, STATE_CURRENT, Shared};
use crate::boot::{self, Loaded};
use crate::host::GameHost;
use crate::perception_shim::{self, PREF_OMNISCIENT};
use crate::prefs::PrefStore;
use crate::present::present_events;
use crate::saves::{NoSaves, SaveBackend, SaveMeta};
use crate::shim;

/// The role of the screen that shows why something the player asked of
/// the application could not be done. It is opened with the parameter
/// `text`.
pub const ROLE_ERROR: &str = "error";

/// The log category of a command the simulation refused.
pub const LOG_REFUSAL: &str = "refusal";
/// The log category of an effect that could not be run.
pub const LOG_UI: &str = "ui";
/// The log category of what the application itself has to say.
pub const LOG_APP: &str = "app";

/// What a screen variable's name starts with for the application to
/// set it, when the screen opens, to a field of `app.creation`:
/// `creation_seed` is set to `app.creation.seed`.
pub const CREATION_VAR_PREFIX: &str = "creation_";

/// The name of the interface effect that asks a service of the
/// application.
const EFFECT_APP: &str = "app";

/// How much simulation time one call of `advance` is asked for, so that
/// a pass can stop between two of them when its slice is used.
pub const SIM_STEP: Dur = Dur::from_millis(100);

/// How long one frame lasts in a terminal.
pub const FRAME_TIME: Duration = Duration::from_millis(16);

/// How much of a frame may be spent advancing the simulation.
pub const SIM_SLICE: Duration = Duration::from_millis(8);

/// What the application is started with, beside the loaded profile.
pub struct BootOptions {
    /// The seed a new game is offered.
    pub seed: u64,
    /// The user's data directory, if there is one: where preferences
    /// and key bindings are kept.
    pub data_dir: Option<PathBuf>,
    /// What keeps saves.
    pub saves: Box<dyn SaveBackend>,
}

impl Default for BootOptions {
    fn default() -> BootOptions {
        BootOptions {
            seed: crate::cli::DEFAULT_SEED,
            data_dir: None,
            saves: Box::new(NoSaves),
        }
    }
}

/// The application, running. See the [module](self) documentation.
pub struct Game {
    scene: Scene<GameHost>,
    shared: Rc<RefCell<Shared>>,
    frame: Frame,
    /// The qualified id of the preference that switches the omniscient
    /// view on, if a loaded mod defines one.
    omniscient_pref: Option<String>,
    /// The screen a game is played on while one is.
    play_screen: Option<String>,
    /// The screen the profile boots into.
    boot_screen: String,
}

/// The part of a qualified id after its namespace.
fn bare(id: &str) -> &str {
    id.rsplit(':').next().unwrap_or(id)
}

impl Game {
    /// Starts the application on a loaded profile: makes the scene with
    /// the theme and the keymap the preferences name, gives back the
    /// player's stored preferences and key bindings, and opens the
    /// profile's boot screen. Nothing is drawn yet.
    ///
    /// A profile with no theme or no keymap preference is shown with
    /// the engine's plain theme and the keyboard's own navigation keys:
    /// the engine brings no content of its own (04 §14).
    pub fn boot(loaded: Loaded, options: BootOptions) -> Result<Game, String> {
        let ui = loaded.ui.clone();
        let boot_name = boot::boot_screen(&loaded)?;
        let boot_screen = ui.resolve::<ScreenDef>(&boot_name, "").ok_or_else(|| {
            format!("the profile's `boot_screen` names a screen `{boot_name}` that no mod defines")
        })?;

        let (store, store_problem) = match &options.data_dir {
            Some(dir) => PrefStore::open(dir),
            None => (PrefStore::in_memory(), None),
        };
        let profile = loaded.defs.profile();
        let profile_entry = clash_scene::AppEntry {
            id: profile
                .map(|profile| profile.id.clone())
                .unwrap_or_default(),
            name: profile
                .map(|profile| profile.name.clone())
                .unwrap_or_default(),
            description: String::new(),
            state: STATE_CURRENT.to_owned(),
        };
        let mut creation = BTreeMap::new();
        creation.insert(CREATION_NAME.to_owned(), ExprValue::text(""));
        creation.insert(
            CREATION_SEED.to_owned(),
            ExprValue::Int(i64::try_from(options.seed).unwrap_or(i64::MAX)),
        );
        let shared = Rc::new(RefCell::new(Shared {
            profile: profile_entry,
            profiles: crate::app::list_profiles(&loaded),
            mods: crate::app::list_mods(&loaded),
            starts: crate::app::list_starts(&loaded),
            creation,
            session: None,
            saves: options.saves,
            store,
            data_dir: options.data_dir,
            keymap: String::new(),
            started: false,
            loaded,
        }));

        // The theme and the keymap in force are the ones the preferences
        // of those names hold: the profile's choice, or the default.
        let defaults = ui.pref_defaults();
        let named = |bare_name: &str| {
            defaults
                .iter()
                .find(|(id, _)| bare(id.as_str()) == bare_name)
                .and_then(|(_, value)| match value {
                    Value::Str(id) => Some(id.clone()),
                    _ => None,
                })
        };
        let render = |diagnostics: clash_data::Diagnostics| diagnostics.render();
        let theme = match named(PREF_THEME) {
            Some(id) => ui.theme(&id).map_err(render)?,
            None => Theme::default(),
        };
        let keymap_id = named(PREF_KEYMAP);
        let keymap = match &keymap_id {
            Some(id) => ui.keymap(id).map_err(render)?.clone(),
            None => KeymapDef::fallback(),
        };
        let defs = shared.borrow().loaded.defs.clone();
        let host = GameHost::new(Rc::clone(&shared), defs);
        let scene = Scene::new(host, theme, keymap)
            .with_keymap_id(keymap_id.as_deref().unwrap_or(""))
            .with_defs(ui.clone())
            .with_app(Box::new(App::new(Rc::clone(&shared))))
            .running_effects();
        shared.borrow_mut().keymap = keymap_id.unwrap_or_default();

        let mut game = Game {
            scene,
            shared,
            frame: Frame::new(0, 0),
            omniscient_pref: ui.resolve::<PreferenceDef>(PREF_OMNISCIENT, ""),
            play_screen: None,
            boot_screen,
        };
        if let Some(problem) = store_problem {
            game.say(LOG_APP, &problem);
        }
        let warnings = game.shared.borrow().loaded.warnings.clone();
        for line in warnings.lines().filter(|line| !line.trim().is_empty()) {
            game.say(LOG_APP, line);
        }
        game.restore_choices();
        let boot_screen = game.boot_screen.clone();
        game.scene.open_screen(&boot_screen, BTreeMap::new())?;
        Ok(game)
    }

    /// Gives the scene back what the player chose in earlier runs:
    /// preferences a loaded mod still defines, and the bindings made in
    /// the keymap in force.
    fn restore_choices(&mut self) {
        let stored: Vec<(String, Value)> = self
            .shared
            .borrow()
            .store
            .prefs()
            .iter()
            .map(|(id, value)| (id.clone(), value.clone()))
            .collect();
        for (id, value) in stored {
            let known = self
                .scene
                .defs()
                .is_some_and(|ui| ui.get::<PreferenceDef>(&id).is_some());
            if !known || self.scene.pref(&id) == Some(&value) {
                continue;
            }
            match self.scene.set_pref(&id, value) {
                Ok(()) => self.pref_changed(&id),
                Err(reason) => self.say(LOG_APP, &reason),
            }
        }
        self.restore_bindings();
    }

    /// Gives the scene the stored bindings of the keymap in force.
    fn restore_bindings(&mut self) {
        let (keymap, bindings) = {
            let shared = self.shared.borrow();
            (shared.keymap.clone(), shared.store.bindings().to_vec())
        };
        for binding in bindings.iter().filter(|binding| binding.keymap == keymap) {
            if let Err(reason) = self
                .scene
                .rebind(&binding.context, &binding.key, &binding.name)
            {
                self.say(LOG_APP, &reason);
            }
        }
    }

    /// The scene: its screens, its log, its preferences.
    pub fn scene(&self) -> &Scene<GameHost> {
        &self.scene
    }

    /// The last frame built.
    pub fn frame(&self) -> &Frame {
        &self.frame
    }

    /// Whether a game is being played.
    pub fn playing(&self) -> bool {
        self.shared.borrow().session.is_some()
    }

    /// Reads the game being played, if one is: for tests and tools.
    pub fn with_session<R>(&self, read: impl FnOnce(&crate::session::Session) -> R) -> Option<R> {
        self.shared.borrow().session.as_ref().map(read)
    }

    /// Adds a line to the log.
    fn say(&mut self, category: &str, line: &str) {
        self.scene
            .log_mut()
            .push(category, RichText::plain_text(line));
    }

    /// Shows why something asked of the application could not be done:
    /// on the screen of the role `error`, if the profile names one.
    fn show_error(&mut self, text: &str) {
        let Some(screen) = self.scene.role(ROLE_ERROR).map(str::to_owned) else {
            return;
        };
        let with = BTreeMap::from([("text".to_owned(), ExprValue::text(text))]);
        if let Err(reason) = self.scene.open_screen(&screen, with) {
            self.say(LOG_UI, &reason);
        }
    }

    /// The player's character's own glyph under the theme in force.
    fn player_look(&self) -> GlyphPart {
        let shared = self.shared.borrow();
        let Some(session) = shared.session.as_ref() else {
            return GlyphPart::default();
        };
        match session.look.compile(self.scene.theme()) {
            Ok(glyph) => glyph.part(&GlyphInputs::default()),
            // A colour this theme lacks: the character, in the theme's
            // own text colour.
            Err(_) => GlyphPart {
                ch: session.look.ch,
                ..GlyphPart::default()
            },
        }
    }

    /// The glyph tables of the loaded world under the theme in force.
    fn world_glyphs(&self) -> Option<WorldGlyphs> {
        let shared = self.shared.borrow();
        let tables = shared.loaded.tables.as_ref().ok()?;
        Some(WorldGlyphs::build(&tables.world, self.scene.theme()).0)
    }

    /// A preference changed: the application acts on the theme (the
    /// world is drawn with it) and the keymap (stored bindings belong
    /// to one).
    fn pref_changed(&mut self, id: &str) {
        match bare(id) {
            PREF_THEME => {
                if let Some(glyphs) = self.world_glyphs()
                    && let Some(view) = self.scene.world_mut()
                {
                    view.set_glyphs(glyphs);
                }
            }
            PREF_KEYMAP => {
                if let Some(Value::Str(keymap)) = self.scene.pref(id).cloned() {
                    self.shared.borrow_mut().keymap = keymap;
                    self.restore_bindings();
                }
            }
            _ => {}
        }
        if let Some(session) = self.shared.borrow_mut().session.as_mut() {
            session.stale_view = true;
        }
    }

    /// Tells the screen on top what the player has chosen for the next
    /// game: each variable it declares named `creation_<field>` is set
    /// to that field of `app.creation`. A screen is told once, when it
    /// opens.
    fn fill_creation(&mut self) {
        let creation = self.shared.borrow().creation.clone();
        let Some(top) = self.scene.top_mut() else {
            return;
        };
        for (field, value) in creation {
            if let Some(value) = to_var(&value) {
                top.set_var(&format!("{CREATION_VAR_PREFIX}{field}"), value);
            }
        }
    }

    /// Acts on what running effects came to. See the
    /// [module](self) documentation.
    fn follow_up(&mut self, outcomes: &[UiOutcome]) {
        for outcome in outcomes {
            match outcome {
                UiOutcome::Refused { reason, .. } => self.say(LOG_REFUSAL, reason),
                UiOutcome::Failed { effect, reason } => {
                    self.say(LOG_UI, reason);
                    if *effect == EFFECT_APP {
                        self.show_error(reason);
                    }
                }
                UiOutcome::Changed(VarRef::Pref(id)) => self.pref_changed(id),
                UiOutcome::Opened(_) => self.fill_creation(),
                _ => {}
            }
        }
        let started = std::mem::take(&mut self.shared.borrow_mut().started);
        if started {
            self.show_game();
        }
        self.end_if_left();
    }

    /// Shows a game that was just started or loaded: the screens that
    /// were open give way to the play screen, and the scene is given a
    /// view of the world to draw.
    fn show_game(&mut self) {
        let Some(play) = self.scene.role(ROLE_PLAY).map(str::to_owned) else {
            self.end_game();
            let reason = format!(
                "no screen answers the role `{ROLE_PLAY}`: the profile's `screens` names none, so the game cannot be shown"
            );
            self.say(LOG_APP, &reason);
            self.show_error(&reason);
            return;
        };
        let at = self
            .shared
            .borrow()
            .session
            .as_ref()
            .and_then(crate::session::Session::player_loc);
        let (Some(at), Some(glyphs)) = (at, self.world_glyphs()) else {
            self.end_game();
            return;
        };
        while self.scene.close().is_some() {}
        self.scene.set_world(WorldView::new(glyphs, at));
        if let Err(reason) = self.scene.open_screen(&play, BTreeMap::new()) {
            self.end_game();
            let boot_screen = self.boot_screen.clone();
            if let Err(reason) = self.scene.open_screen(&boot_screen, BTreeMap::new()) {
                self.say(LOG_UI, &reason);
            }
            self.say(LOG_APP, &reason);
            self.show_error(&reason);
            return;
        }
        self.play_screen = Some(play);
        if let Some(session) = self.shared.borrow_mut().session.as_mut() {
            session.shown = true;
            session.stale_view = true;
        }
    }

    /// Ends the game being played: the simulation is dropped and the
    /// scene's view of the world with it.
    fn end_game(&mut self) {
        self.shared.borrow_mut().session = None;
        self.scene.take_world();
        self.play_screen = None;
    }

    /// Ends the game if its play screen is no longer open: leaving the
    /// play screen is how a screen ends a game.
    fn end_if_left(&mut self) {
        let shown = self
            .shared
            .borrow()
            .session
            .as_ref()
            .is_some_and(|session| session.shown);
        let Some(play) = &self.play_screen else {
            return;
        };
        if shown
            && !self
                .scene
                .screens()
                .iter()
                .any(|screen| screen.id() == play)
        {
            self.end_game();
        }
    }

    /// Advances the simulation toward the moment the player's character
    /// is free again, showing its events as it goes. `until` is when
    /// this pass's slice of real time ends; with none the simulation is
    /// advanced until the character is free.
    fn advance_simulation(&mut self, until: Option<Instant>) {
        let mut shared = self.shared.borrow_mut();
        let shared = &mut *shared;
        let Some(session) = shared.session.as_mut() else {
            return;
        };
        let mut lines: Vec<(&str, String)> = Vec::new();
        loop {
            // What commands and the last step delivered.
            let events = session.sim.drain_events();
            present_events(&events, session, self.scene.log_mut());
            if let Some(line) = shim::next_route_step(session) {
                lines.push((LOG_REFUSAL, line));
            }
            let Some(target) = session.busy_until else {
                break;
            };
            let next = target.min(session.sim.now() + SIM_STEP);
            match session.sim.advance(AdvanceLimit::Until(next)) {
                // Time does not move until the decision is answered.
                Advance::NeedInput(_) => {
                    session.busy_until = None;
                    session.route.clear();
                }
                Advance::Reached(now) => {
                    if now >= target {
                        session.busy_until = None;
                    }
                }
                Advance::Yielded => {}
                Advance::Request(clash_sim::AppRequest::Checkpoint { name }) => {
                    let meta = SaveMeta {
                        start: session.start.id.clone(),
                        name: session.name.clone(),
                    };
                    let dir = shared
                        .data_dir
                        .as_ref()
                        .map(|dir| dir.join(crate::saves::SAVES_DIR));
                    if let Err(reason) =
                        shared
                            .saves
                            .save(dir.as_deref(), &mut session.sim, &meta, &name)
                    {
                        lines.push((
                            LOG_APP,
                            format!("The checkpoint `{name}` was not saved: {reason}."),
                        ));
                    }
                }
                Advance::Request(_) => lines.push((
                    LOG_APP,
                    "A rule asked the application for something it cannot do yet.".to_owned(),
                )),
            }
            if until.is_some_and(|until| Instant::now() >= until) {
                // The slice is used: what was delivered is shown, and
                // the rest waits for the next pass.
                let events = session.sim.drain_events();
                present_events(&events, session, self.scene.log_mut());
                break;
            }
        }
        for (category, line) in lines {
            self.scene
                .log_mut()
                .push(category, RichText::plain_text(&line));
        }
    }

    /// Whether the debugging view of the world is switched on.
    fn omniscient(&self) -> bool {
        self.omniscient_pref
            .as_deref()
            .and_then(|id| self.scene.pref(id))
            .is_some_and(|value| *value == Value::Bool(true))
    }

    /// Feeds the scene's view of the world, if the simulation has moved
    /// since it was last fed.
    fn feed_view(&mut self) {
        let omniscient = self.omniscient();
        let look = self.player_look();
        let mut problem = None;
        {
            let mut shared = self.shared.borrow_mut();
            let Some(session) = shared.session.as_mut() else {
                return;
            };
            if !session.stale_view && session.omniscient == omniscient {
                return;
            }
            let Some(view) = self.scene.world_mut() else {
                return;
            };
            let fed = if omniscient {
                perception_shim::feed_omniscient(session, view, look)
            } else {
                perception_shim::feed(session, view, look)
            };
            session.stale_view = false;
            if let Err(reason) = fed {
                problem = Some(reason);
            }
        }
        if let Some(reason) = problem {
            self.say(LOG_APP, &reason);
        }
        // Bindings on the view are evaluated again before the frame.
        self.scene.simulation_advanced();
    }

    /// One pass of the loop: see the [module](self) documentation.
    /// `seconds` is the real time since the last pass, and `until` when
    /// this pass must stop advancing the simulation; with none it is
    /// advanced until the player's character is free, which is what a
    /// test with a fixed time step wants. Returns whether to go on.
    pub fn pass(&mut self, front: &mut dyn Frontend, seconds: f64, until: Option<Instant>) -> bool {
        let mut closed = false;
        let mut outcomes = Vec::new();
        for event in front.poll() {
            closed |= event == InputEvent::Close;
            outcomes.extend(self.scene.handle(&event));
        }
        self.follow_up(&outcomes);
        self.advance_simulation(until);
        let asked = self.scene.sync_decision();
        self.follow_up(&asked);
        self.feed_view();
        self.scene.advance(seconds);
        let caps = front.caps();
        if (self.frame.cols, self.frame.rows) != (caps.cols, caps.rows) {
            self.frame.resize(caps.cols, caps.rows);
        }
        self.scene.build_frame(&mut self.frame);
        front.present(&self.frame);
        !(closed || self.scene.quit_requested() || self.scene.top().is_none())
    }
}

/// Runs the loop on a frontend until the player leaves: each pass is
/// given the real time since the last and a slice of the frame to
/// advance the simulation in, and the rest of the frame is slept out.
/// After each pass `failed` is asked whether the frontend has broken;
/// if it says how, the loop ends with that.
///
/// The frontend is taken whole and dropped before this returns, however
/// it returns: for a terminal, that is what gives the terminal back.
pub fn run_frontend<F: Frontend>(
    game: &mut Game,
    mut front: F,
    mut failed: impl FnMut(&mut F) -> Option<String>,
) -> Result<(), String> {
    let mut last = Instant::now();
    let result = loop {
        let began = Instant::now();
        let seconds = began.duration_since(last).as_secs_f64();
        last = began;
        let go_on = game.pass(&mut front, seconds, Some(began + SIM_SLICE));
        if let Some(problem) = failed(&mut front) {
            break Err(problem);
        }
        if !go_on {
            break Ok(());
        }
        if let Some(rest) = FRAME_TIME.checked_sub(began.elapsed()) {
            std::thread::sleep(rest);
        }
    };
    drop(front);
    result
}
