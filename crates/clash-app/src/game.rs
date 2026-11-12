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
//! 3. **The simulation.** While the player's character waits on the
//!    simulation (`Session::waits_on_simulation`: no decision is asked
//!    of it and it is busy or unconscious), the simulation is advanced
//!    toward its next decision (03 §5.2): `advance(Work(n))` with
//!    [`SIM_WORK`] wake-ups at a time, which is `UntilDecision` in
//!    slices, until a decision is needed, nothing is left to happen,
//!    the pass's time slice is used or [`PASS_LIMIT`] of simulation time
//!    has passed. Nothing else advances the simulation.
//! 4. **Playback.** Everything the simulation reported in the pass, the
//!    commands' events and the advance's, is drained and handed to the
//!    scene as one batch (`Scene::push_events`), after what the party
//!    perceives as the batch ends has been taken from the simulation's
//!    perception ([`crate::perception::look`]), so that the host answers
//!    what the party perceived as of the end of the batch. What is due
//!    at the present tick is run even while a decision waits, so that
//!    a sense that sweeps at once (a character that has just stepped,
//!    one that has just been put in the world) has swept before the
//!    character is asked.
//!    A fault, and a `message` for the player, also get a line in the
//!    log at once. The scene is then advanced by the real time elapsed
//!    and plays what falls due.
//! 5. **The present.** Once playback has caught up: the view model is
//!    given what the party perceives that no event said
//!    ([`crate::perception::show`]), and in a debug build checked
//!    against a fresh one (`Scene::check_caught_up`); a decision that
//!    is waiting gets its screen (`Scene::sync_decision`).
//! 6. **The frame.** The scene builds a frame of the frontend's size and
//!    the frontend presents it.
//!
//! Input applies to the present: a key pressed while something is being
//! played jumps playback to its end before it is used (the scene does
//! this), and so does a click or pasted text (this loop does).
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
//! set, when it opens, to that field of the character being made, `name`
//! or `seed` ([`crate::creation`]): the new-game screen starts with the
//! seed given on the command line.
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

use clash_core::{Dur, EntityId, Value};
use clash_expr::Value as ExprValue;
use clash_scene::ControlDef;
use clash_scene::control::ROLE_PLAY;
use clash_scene::input::MouseKind;
use clash_scene::ui::{PreferenceDef, ScreenDef, VarRef, to_var};
use clash_scene::view::{SpriteId, WorldGlyphs, WorldView};
use clash_scene::{
    Frame, Frontend, GlyphInputs, GlyphPart, InputEvent, KeymapDef, PREF_KEYMAP, PREF_THEME,
    RichText, Scene, Theme, UiOutcome,
};
use clash_sim::action::ActEvent;
use clash_sim::reader::capitalised;
use clash_sim::{Advance, AdvanceLimit, EventBody, SimEvent};

use crate::app::{App, STATE_CURRENT, Shared};
use crate::boot::{self, Loaded};
use crate::creation::Creation;
use crate::host::GameHost;
use crate::interact;
use crate::perception::{self, PREF_OMNISCIENT};
use crate::prefs::PrefStore;
use crate::saves::{FileSaves, SaveBackend};

/// The role of the screen that shows why something the player asked of
/// the application could not be done. It is opened with the parameter
/// `text`.
pub const ROLE_ERROR: &str = "error";

/// The role of the screen that tells the player something that is not a
/// failure: what differs in a save that was loaded. It is opened with
/// the parameters `title` and `text`.
pub const ROLE_NOTICE: &str = "notice";

/// The log category of a command the simulation refused.
pub const LOG_REFUSAL: &str = "refusal";
/// The log category of an effect that could not be run.
pub const LOG_UI: &str = "ui";
/// The log category of what the application itself has to say.
pub const LOG_APP: &str = "app";
/// The log category of a fault the engine contained.
pub const LOG_FAULT: &str = "fault";
/// The log category of a `message` a rule sent the player (05 §10.6).
pub const LOG_MESSAGE: &str = "message";

/// What a screen variable's name starts with for the application to
/// set it, when the screen opens, to a field of `app.creation`:
/// `creation_seed` is set to `app.creation.seed`.
pub const CREATION_VAR_PREFIX: &str = "creation_";

/// The name of the interface effect that asks a service of the
/// application.
const EFFECT_APP: &str = "app";

/// How many wake-ups one call of `advance` may handle, so that a pass
/// can stop between two calls when its slice is used.
pub const SIM_WORK: u32 = 256;

/// The most simulation time one pass advances by. A character that
/// waits a long while, or lies unconscious, is carried through it over
/// several passes, and a pass that has nothing to stop it (everyone
/// else busy for ever) still returns.
pub const PASS_LIMIT: Dur = Dur::from_secs(3_600);

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
            saves: Box::new(FileSaves),
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
        let creation = Creation::new(options.seed);
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
            warnings: Vec::new(),
            said: Vec::new(),
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
            if !known || !crate::prefs::kept(&id) || self.scene.pref(&id) == Some(&value) {
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

    /// Reads the loaded profile: its definitions, the interface's and
    /// the world's.
    pub fn with_loaded<R>(&self, read: impl FnOnce(&Loaded) -> R) -> R {
        read(&self.shared.borrow().loaded)
    }

    /// Reads the game being played, if one is: for tests and tools.
    pub fn with_session<R>(&self, read: impl FnOnce(&crate::session::Session) -> R) -> Option<R> {
        self.shared.borrow().session.as_ref().map(read)
    }

    /// Hands a command to the game being played, as the scene's host
    /// does, through `Sim::submit`: for tools, tests and a console. What
    /// it causes is shown by the next pass. Refused with the
    /// simulation's reason, or when no game is being played.
    pub fn submit(&mut self, command: clash_sim::Command) -> Result<(), String> {
        let mut shared = self.shared.borrow_mut();
        let session = shared
            .session
            .as_mut()
            .ok_or_else(|| crate::host::NO_GAME.to_owned())?;
        session
            .sim
            .submit(command)
            .map_err(|refusal| refusal.reason.message)?;
        session.stale_view = true;
        Ok(())
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

    /// The player's character's own glyph under the theme in force, if
    /// it has one.
    fn player_look(&self) -> Option<GlyphPart> {
        let shared = self.shared.borrow();
        let look = shared.session.as_ref()?.look.as_ref()?;
        Some(match look.compile(self.scene.theme()) {
            Ok(glyph) => glyph.part(&GlyphInputs::default()),
            // A colour this theme lacks: the character, in the theme's
            // own text colour.
            Err(_) => GlyphPart {
                ch: look.ch,
                ..GlyphPart::default()
            },
        })
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
    /// to that field, `name` or `seed`, of the character being made. A
    /// screen is told once, when it opens.
    fn fill_creation(&mut self) {
        let fields = self.shared.borrow().creation.fields();
        let Some(top) = self.scene.top_mut() else {
            return;
        };
        for (field, value) in fields {
            if let Some(value) = to_var(&value) {
                top.set_var(&format!("{CREATION_VAR_PREFIX}{field}"), value);
            }
        }
    }

    /// A preference's value, by its bare name, as text, if a loaded mod
    /// defines it.
    fn pref_text(&self, bare_name: &str) -> Option<String> {
        let id = self.scene.defs()?.resolve::<PreferenceDef>(bare_name, "")?;
        match self.scene.pref(&id)? {
            Value::Str(text) => Some(text.clone()),
            _ => None,
        }
    }

    /// The effects of a control, by its bare name, if a loaded mod
    /// defines it.
    fn control_effects(&self, bare_name: &str) -> Option<clash_scene::ui::UiEffects> {
        let ui = self.scene.defs()?;
        let id = ui.resolve::<ControlDef>(bare_name, "")?;
        ui.get::<ControlDef>(&id)
            .map(|control| control.effects.clone())
    }

    /// A step was refused because the way is closed: what is there is
    /// done, asked about, or why it cannot be done is told, as
    /// [`crate::interact`] says. `reason` is the step's refusal.
    fn bump(&mut self, dir: &str, reason: &str) {
        let found = {
            let shared = self.shared.borrow();
            let Some(session) = shared.session.as_ref() else {
                return;
            };
            interact::bumped(&session.sim, session.player, dir)
        };
        let mode = self.pref_text(interact::PREF_BUMP);
        match found {
            Ok(offer) if mode.as_deref() != Some(interact::BUMP_NEVER) => {
                let control = if mode.as_deref() == Some(interact::BUMP_ASK) {
                    interact::CONTROL_BUMP_ASK
                } else {
                    interact::CONTROL_BUMP
                };
                match self.control_effects(control) {
                    Some(effects) => {
                        let outcomes = self.scene.run(&effects, Some(offer));
                        self.follow_up(&outcomes);
                    }
                    None => self.say(LOG_REFUSAL, reason),
                }
            }
            Ok(_) => self.say(LOG_REFUSAL, reason),
            Err(why) => self.say(LOG_REFUSAL, why.as_deref().unwrap_or(reason)),
        }
    }

    /// Acts on what running effects came to. See the
    /// [module](self) documentation.
    fn follow_up(&mut self, outcomes: &[UiOutcome]) {
        for outcome in outcomes {
            match outcome {
                UiOutcome::Refused { reason, .. } => {
                    let bumped = self
                        .shared
                        .borrow_mut()
                        .session
                        .as_mut()
                        .and_then(|session| session.bumped.take());
                    match bumped {
                        Some(dir) => self.bump(&dir, reason),
                        None => self.say(LOG_REFUSAL, reason),
                    }
                }
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
        let said = std::mem::take(&mut self.shared.borrow_mut().said);
        for line in said {
            self.say(LOG_APP, &line);
        }
        let started = std::mem::take(&mut self.shared.borrow_mut().started);
        if started {
            self.show_game();
            let warnings = std::mem::take(&mut self.shared.borrow_mut().warnings);
            if !warnings.is_empty() {
                self.show_warnings(&warnings);
            }
        }
        self.end_if_left();
    }

    /// Tells the player what a load had to say (01 §11.2): a line in
    /// the log for each, and the screen of the role [`ROLE_NOTICE`]
    /// over the play screen with all of them, if the profile names one.
    fn show_warnings(&mut self, warnings: &[String]) {
        for line in warnings {
            self.say(LOG_APP, &format!("The save differs: {line}."));
        }
        let Some(screen) = self.scene.role(ROLE_NOTICE).map(str::to_owned) else {
            return;
        };
        let text = warnings
            .iter()
            .map(|line| format!("- {}", capitalised(line)))
            .collect::<Vec<_>>()
            .join("\n");
        let with = BTreeMap::from([
            (
                "title".to_owned(),
                ExprValue::text("This save was made with other mods"),
            ),
            ("text".to_owned(), ExprValue::text(text)),
        ]);
        if let Err(reason) = self.scene.open_screen(&screen, with) {
            self.say(LOG_UI, &reason);
        }
    }

    /// Shows a game that was just started or loaded: the screens that
    /// were open give way to the play screen, and the scene is given a
    /// view of the world to draw, empty until what the party remembers
    /// and sees is put in it. Nothing of a game shown before is left to
    /// play.
    fn show_game(&mut self) {
        self.scene.skip_playback();
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
        // The camera follows the character as playback moves it.
        let player = self
            .shared
            .borrow()
            .session
            .as_ref()
            .map(|session| SpriteId::new(session.player.to_bits()));
        self.scene.playback_mut().follow(player);
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

    /// Ends the game being played: what was still to be played is
    /// played, and the simulation is dropped and the scene's view of the
    /// world with it.
    fn end_game(&mut self) {
        self.scene.skip_playback();
        self.scene.playback_mut().follow(None);
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
            self.save_on_leaving();
            self.end_game();
        }
    }

    /// A game whose saving policy is `single` is saved as it is left
    /// (13 §9.2), under the character's name: its one save.
    fn save_on_leaving(&mut self) {
        let said = {
            let mut shared = self.shared.borrow_mut();
            let shared = &mut *shared;
            let dir = shared.saves_dir();
            let Some(session) = shared.session.as_mut() else {
                return;
            };
            if session.saving != crate::app::POLICY_SINGLE {
                return;
            }
            let name = session.default_save_name();
            let meta = session.save_meta();
            match shared
                .saves
                .save(dir.as_deref(), &mut session.sim, &meta, &name)
            {
                Ok(_) => format!("Saved as “{name}” on leaving."),
                Err(reason) => format!("The game was not saved on leaving: {reason}."),
            }
        };
        self.say(LOG_APP, &said);
    }

    /// Advances the simulation toward the player's next decision and
    /// hands the scene what it reported: see the [module](self)
    /// documentation, steps 3 and 4. `until` is when this pass's slice
    /// of real time ends; with none the simulation is advanced until the
    /// character is asked, or [`PASS_LIMIT`] has passed.
    fn advance_simulation(&mut self, until: Option<Instant>) {
        let look = self.player_look();
        let mut lines: Vec<(&str, String)> = Vec::new();
        let batch = {
            let mut shared = self.shared.borrow_mut();
            let shared = &mut *shared;
            let Some(session) = shared.session.as_mut() else {
                return;
            };
            // What the commands of this pass delivered comes first.
            let mut batch = session.sim.drain_events();
            let began = session.sim.now();
            // What is due now runs even while a decision waits: the
            // simulation runs it before it asks (03 §5.2).
            let mut due_now = session.due_now();
            while due_now || session.waits_on_simulation() {
                // Only what is due now, unless the character waits on
                // the simulation: nothing else moves time.
                let limit = if session.waits_on_simulation() {
                    AdvanceLimit::Work(SIM_WORK)
                } else {
                    AdvanceLimit::Until(session.sim.now())
                };
                due_now = false;
                let answer = session.sim.advance(limit);
                batch.extend(session.sim.drain_events());
                match answer {
                    // The character is asked, or nothing is left to
                    // happen: the pass has gone as far as it can.
                    Advance::NeedInput(_) | Advance::Reached(_) => break,
                    Advance::Yielded => {}
                    Advance::Request(clash_sim::AppRequest::Checkpoint { name }) => {
                        let dir = shared
                            .data_dir
                            .as_ref()
                            .map(|dir| dir.join(crate::saves::SAVES_DIR));
                        let saved = if session.saving == crate::app::POLICY_NONE {
                            Err("this game is not saved: its saving policy is `none`".to_owned())
                        } else {
                            let meta = session.save_meta();
                            shared
                                .saves
                                .save(dir.as_deref(), &mut session.sim, &meta, &name)
                        };
                        match saved {
                            Ok(_) => lines.push((LOG_APP, format!("Checkpoint “{name}” saved."))),
                            Err(reason) => lines.push((
                                LOG_APP,
                                format!("The checkpoint “{name}” was not saved: {reason}."),
                            )),
                        }
                    }
                    Advance::Request(_) => lines.push((
                        LOG_APP,
                        "A rule asked the application for something it cannot do yet.".to_owned(),
                    )),
                }
                let slice_used = until.is_some_and(|until| Instant::now() >= until);
                if slice_used || session.sim.now().since(began) >= PASS_LIMIT {
                    break;
                }
            }
            if !batch.is_empty() {
                session.stale_view = true;
            }
            lines.extend(batch.iter().filter_map(|event| told(event, session.player)));
            // What the party perceives as the batch ends: what the host
            // answers playback with.
            if session.stale_view {
                session.stale_view = false;
                perception::look(
                    &mut session.sim,
                    session.player,
                    look,
                    self.scene.theme(),
                    &mut session.percepts,
                );
            }
            batch
        };
        for (category, line) in lines {
            self.say(category, &line);
        }
        if !batch.is_empty() {
            self.scene.push_events(batch);
            self.scene.simulation_advanced();
        }
    }

    /// Whether the debugging view of the world is switched on.
    fn omniscient(&self) -> bool {
        self.omniscient_pref
            .as_deref()
            .and_then(|id| self.scene.pref(id))
            .is_some_and(|value| *value == Value::Bool(true))
    }

    /// Gives the scene's view of the world what the party perceives now
    /// that no event said, once playback has caught up, and in a debug
    /// build checks the picture against a fresh one (12 §5). With the
    /// omniscient view on, fills it from truth instead, and has the
    /// simulation call every event seen (`set_omniscient`).
    fn feed_view(&mut self) {
        let omniscient = self.omniscient();
        let look = self.player_look();
        let theme = self.scene.theme().clone();
        let mut problem = None;
        let fresh = {
            let mut shared = self.shared.borrow_mut();
            let Some(session) = shared.session.as_mut() else {
                return;
            };
            let Some(view) = self.scene.world_mut() else {
                return;
            };
            if omniscient {
                if !session.omniscient {
                    // Nothing seen before is mixed with truth.
                    view.model_mut().clear();
                    session.omniscient = true;
                    clash_sim::perception::set_omniscient(&mut session.sim, true);
                } else if !session.percepts.unshown {
                    return;
                }
                session.percepts.unshown = false;
                problem =
                    perception::feed_omniscient(&session.sim, session.player, look, &theme, view)
                        .err();
                None
            } else {
                if session.omniscient {
                    // Back to the party's own eyes: nothing learnt from
                    // truth is kept, and the picture is built again from
                    // what the party remembers and sees.
                    session.omniscient = false;
                    clash_sim::perception::set_omniscient(&mut session.sim, false);
                    session.percepts.rebuild = true;
                    session.percepts.unshown = true;
                }
                if !session.percepts.unshown {
                    return;
                }
                perception::show(&session.sim, session.player, view, &mut session.percepts);
                cfg!(debug_assertions)
                    .then(|| perception::fresh(&session.sim, session.player, &session.percepts))
            }
        };
        if let Some(reason) = problem {
            self.say(LOG_APP, &reason);
        }
        if let Some(fresh) = fresh
            && let Err(drift) = self.scene.check_caught_up(&fresh)
        {
            let line = format!(
                "The picture is not what the character knows: {} difference{} from a fresh view.",
                drift.differences.len(),
                if drift.differences.len() == 1 {
                    ""
                } else {
                    "s"
                }
            );
            self.say(LOG_FAULT, &line);
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
            // Input applies to the present. A key jumps playback to its
            // end itself (12 §7.1); a click and pasted text do here.
            if acts_on_present(&event) && !self.scene.playback_caught_up() {
                self.scene.skip_playback();
                self.feed_view();
            }
            outcomes.extend(self.scene.handle(&event));
        }
        self.follow_up(&outcomes);
        self.advance_simulation(until);
        self.scene.advance(seconds);
        if self.scene.playback_caught_up() {
            self.feed_view();
            // A decision's prompt opens once playback has caught up.
            let asked = self.scene.sync_decision();
            self.follow_up(&asked);
        }
        let caps = front.caps();
        if (self.frame.cols, self.frame.rows) != (caps.cols, caps.rows) {
            self.frame.resize(caps.cols, caps.rows);
        }
        self.scene.build_frame(&mut self.frame);
        front.present(&self.frame);
        !(closed || self.scene.quit_requested() || self.scene.top().is_none())
    }
}

/// Whether an input event acts on what is on screen, other than a key:
/// a press of a mouse button, or pasted text.
fn acts_on_present(event: &InputEvent) -> bool {
    match event {
        InputEvent::Mouse(mouse) => matches!(mouse.kind, MouseKind::Down(_)),
        InputEvent::Text(_) => true,
        _ => false,
    }
}

/// The line the application writes in the log at once for an event,
/// beside what playback writes: a fault the engine contained, and a
/// `message` a rule sent the player (12 §7.4).
fn told(event: &SimEvent, player: EntityId) -> Option<(&'static str, String)> {
    match &event.body {
        EventBody::Fault { reason } => Some((LOG_FAULT, format!("Fault: {reason}"))),
        EventBody::Act(ActEvent::Message { to, text, .. }) if *to == player => {
            Some((LOG_MESSAGE, text.clone()))
        }
        _ => None,
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
