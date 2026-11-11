//! [`Scene`]: what stands between a frontend and the simulation (12 §3).
//!
//! The scene takes input, keeps the stack of screens, evaluates their
//! bindings, runs their effects and builds frames.
//!
//! # The seams
//!
//! | To | Through | It can |
//! |----|---------|--------|
//! | The simulation | [`SceneHost`] | `submit` a command; read `view` and `party` and whatever bindings read through them |
//! | The application | [`AppState`] | Read what `app` offers; ask for an engine service |
//!
//! The link to the simulation is deliberately narrow: the scene can
//! `submit` a command and can change nothing any other way (12 invariant
//! V1). What it reads, it reads through a [`clash_expr::Host`], which
//! has no way to write.
//!
//! The simulation's own command type is not known here. An effect builds
//! a [`UiCommand`], the generic form of `act`, `command` and `run`: a
//! name and arguments as values. A host whose command can be made from
//! one (`H::Command: From<UiCommand>`) is submitted to, and refuses what
//! it does not accept, with its reason.
//!
//! [`NoHost`] is the host of a scene with no simulation behind it:
//! tools, tests and the widget gallery. It refuses every command.
//!
//! # The stack
//!
//! Screens are kept on a stack. Input goes to the top one. A screen
//! that is a popup (12 §10.1) leaves the ones beneath it on show, so the
//! frame is drawn from the lowest screen that still shows upward, and a
//! screen that is closed gives way to the one beneath with its state as
//! it was.
//!
//! # Bindings
//!
//! Bindings are evaluated again before a frame is built whenever
//! something they may read has changed: the application says the
//! simulation has advanced ([`Scene::simulation_advanced`]), a screen
//! variable or a preference changed, or a screen was opened or closed.
//! Only the screens on show are evaluated, and of them only what is on
//! show.

use core::fmt;
use std::collections::BTreeMap;

use clash_core::{Rng, Value, VarMap};
use clash_data::Runner;
use clash_expr::{
    DefRef, FunctionId, Handle, Host, PropertyId, RuntimeWarning, Value as ExprValue,
};
use thiserror::Error;

use crate::app::{AppIds, AppReader, AppRequest, AppState, NoApp, pref_request};
use crate::defs::{UiDefs, namespace_of};
use crate::frame::{Frame, FrameCell};
use crate::frontend::Frontend;
use crate::input::{InputEvent, KeyChord, KeymapDef};
use crate::theme::Theme;
use crate::ui::{
    AppService, Args, BindScope, Binding, Collect, CommandKind, Evaluator, MessageLog,
    PreferenceDef, Prop, SLOT_APP, SLOT_IT, SLOT_PARTY, SLOT_SCREEN, SLOT_VIEW, Screen, ScreenDef,
    TextProp, UiData, UiEffect, UiEffects, UiEvent, ValueProp, VarRef, ViewPainter, from_var,
    to_var,
};
use crate::view::WorldView;

/// The role of the screen the `confirm` effect opens (12 §10.7).
pub const ROLE_CONFIRM: &str = "confirm";
/// The role of the screen the `pick` effect opens for a list.
pub const ROLE_PICK_LIST: &str = "picker.list";
/// The bare name of the preference that holds the theme in use.
pub const PREF_THEME: &str = "theme";
/// The bare name of the preference that holds the keymap in use.
pub const PREF_KEYMAP: &str = "keymap";

/// How much of its brightness a cell beneath a dimming popup keeps, in
/// parts of 256.
const DIM_KEEPS: u16 = 112;

/// A command an interface effect built (12 §10.5): the generic form of
/// the simulation's `Act`, of any other command, and of `Ui`, which runs
/// a `ui_callable` rule. The host turns it into its own command, or
/// refuses it.
#[derive(Clone, Debug, PartialEq)]
pub struct UiCommand {
    /// Which command: an action, a named command, or a rule.
    pub kind: CommandKind,
    /// The action, the command or the rule.
    pub name: String,
    /// The arguments, by name, as the effect's expressions gave them.
    pub args: BTreeMap<String, ExprValue>,
    /// For an action: whether the player is to be asked for its
    /// parameters (12 §9.3).
    pub collect: Collect,
}

impl fmt::Display for UiCommand {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} {}", self.kind.effect(), self.name)?;
        for (name, value) in &self.args {
            write!(f, " {name}={value}")?;
        }
        match self.collect {
            Collect::Given => Ok(()),
            Collect::Needed => f.write_str(" (ask for what is needed)"),
            Collect::All => f.write_str(" (ask for everything)"),
        }
    }
}

/// The scene's handle on the simulation: it can submit a command, and it
/// can read (12 invariant V1).
pub trait SceneHost {
    /// What is submitted: the simulation's `Command` (03 §4.4).
    type Command;
    /// Why a command was refused: the simulation's `CommandError`.
    type Refusal;

    /// Hands a command to the simulation. A refusal carries its reason.
    fn submit(&mut self, command: Self::Command) -> Result<(), Self::Refusal>;

    /// The active controlled character: what `view` is in a binding
    /// (12 §10.4). Null when there is none.
    fn view(&self) -> ExprValue {
        ExprValue::Null
    }

    /// All directly controlled characters, as a list: what `party` is.
    fn party(&self) -> ExprValue {
        ExprValue::list([])
    }

    /// What bindings are evaluated against: it answers the properties,
    /// variables and functions the simulation registered. It can only
    /// be read from. A host with no simulation answers everything with
    /// null.
    fn reader(&mut self) -> &mut dyn Host {
        // `NoReader` holds nothing, so this allocates nothing.
        Box::leak(Box::new(NoReader))
    }
}

/// What a scene with no simulation reads from: nothing. Every answer is
/// null.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct NoReader;

impl Host for NoReader {
    fn subject(&mut self, _slot: u16) -> ExprValue {
        ExprValue::Null
    }

    fn local(&mut self, _slot: u16) -> ExprValue {
        ExprValue::Null
    }

    fn event_field(&mut self, _slot: u16) -> ExprValue {
        ExprValue::Null
    }

    fn property(&mut self, _property: PropertyId, _subject: Handle, _args: &[ExprValue]) -> ExprValue {
        ExprValue::Null
    }

    fn variable(&mut self, _subject: Handle, _name: &str) -> ExprValue {
        ExprValue::Null
    }

    fn call(&mut self, _function: FunctionId, _args: &[ExprValue]) -> ExprValue {
        ExprValue::Null
    }

    fn rng(&mut self) -> Option<&mut Rng> {
        None
    }

    fn warn(&mut self, _warning: RuntimeWarning) {}
}

/// Why [`NoHost`] refuses a command: there is no simulation to give it
/// to.
#[derive(Clone, Debug, PartialEq, Eq, Error)]
#[error("there is no simulation to submit `{command}` to")]
pub struct NoSimulation {
    /// The command, as text.
    pub command: String,
}

/// The host of a scene with no simulation behind it. It refuses every
/// command, saying so.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct NoHost;

impl SceneHost for NoHost {
    type Command = UiCommand;
    type Refusal = NoSimulation;

    fn submit(&mut self, command: UiCommand) -> Result<(), NoSimulation> {
        Err(NoSimulation {
            command: command.to_string(),
        })
    }
}

/// What running an effect came to. The application reads these to know
/// what the player did; a test reads them to know what was handed over.
#[derive(Clone, Debug, PartialEq)]
pub enum UiOutcome {
    /// A screen was opened.
    Opened(String),
    /// A screen was closed.
    Closed(String),
    /// A screen variable or a preference was changed.
    Changed(VarRef),
    /// A command was built and the simulation accepted it.
    Submitted(UiCommand),
    /// A command was built and the simulation refused it.
    Refused {
        /// The command.
        command: UiCommand,
        /// Why.
        reason: String,
    },
    /// An engine service was carried out.
    Service(AppRequest),
    /// An effect could not be run.
    Failed {
        /// The effect.
        effect: &'static str,
        /// Why.
        reason: String,
    },
}

/// What one [`Scene::turn`] produced.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Turn {
    /// What the screens asked for in answer to the input.
    pub events: Vec<UiEvent>,
    /// What running their effects came to, if the scene runs them.
    pub outcomes: Vec<UiOutcome>,
    /// Whether the player asked to close the window, or to quit.
    pub close_requested: bool,
}

impl Turn {
    /// Whether any event asks for an effect with this name to be run.
    pub fn asks_for(&self, effect: &str) -> bool {
        self.events.iter().any(|event| event.asks_for(effect))
    }
}

/// The seed of the scene's own random stream. Presentation randomness
/// (particles, shake) never touches the simulation's streams.
const SCENE_SEED: u64 = 0x5ce4e;

/// What waits for the answer of a screen opened by `confirm` or `pick`.
#[derive(Clone, Debug)]
struct Pending {
    /// Where a pick's choice is stored. `None` for a confirm.
    into: Option<Option<VarRef>>,
    /// The effects that follow in the list that asked.
    rest: Vec<UiEffect>,
    /// What `it` was for that list.
    it: Option<ExprValue>,
}

/// The scene: the stack of screens, the theme and keymap in force, the
/// preferences, the message log, and the routes to the simulation and
/// the application.
///
/// # The only route to the simulation is `submit`
///
/// The host is private, and no method hands it out. This compiles:
///
/// ```
/// use clash_scene::{KeymapDef, Scene, SceneHost, Theme};
///
/// struct Counter(u32);
/// impl SceneHost for Counter {
///     type Command = u32;
///     type Refusal = String;
///     fn submit(&mut self, command: u32) -> Result<(), String> {
///         if command == 0 {
///             return Err("nothing to add".to_owned());
///         }
///         self.0 += command;
///         Ok(())
///     }
/// }
///
/// let mut scene = Scene::new(Counter(0), Theme::default(), KeymapDef::fallback());
/// assert_eq!(scene.submit(2), Ok(()));
/// assert_eq!(scene.submit(0), Err("nothing to add".to_owned()));
/// ```
///
/// and each of these does not, because it reaches for the host some other
/// way:
///
/// ```compile_fail
/// use clash_scene::{Scene, SceneHost};
/// fn reach<H: SceneHost>(scene: &mut Scene<H>) -> &mut H {
///     &mut scene.host
/// }
/// ```
///
/// ```compile_fail
/// use clash_scene::{Scene, SceneHost};
/// fn reach<H: SceneHost>(scene: &mut Scene<H>) -> &mut H {
///     scene.host_mut()
/// }
/// ```
///
/// ```compile_fail
/// use clash_scene::{Scene, SceneHost};
/// fn take<H: SceneHost>(scene: Scene<H>) -> H {
///     scene.into_host()
/// }
/// ```
///
/// ```compile_fail
/// use clash_scene::{Scene, SceneHost};
/// fn peek<H: SceneHost>(scene: &Scene<H>) -> &H {
///     scene.host()
/// }
/// ```
///
/// Once the simulation exists these tests must be joined by ones that
/// use the real handle: that a scene holding it cannot call `advance`,
/// `drain_events` or anything that takes the simulation mutably, and
/// that the view it is given is read-only.
pub struct Scene<H: SceneHost = NoHost> {
    host: H,
    app: Box<dyn AppState>,
    theme: Theme,
    keymap: KeymapDef,
    screens: Vec<Screen>,
    /// For each open screen, what waits for its answer.
    pending: Vec<Option<Pending>>,
    log: MessageLog,
    rng: Rng,
    elapsed: f64,
    world: Option<WorldView>,
    ui: Option<UiDefs>,
    app_ids: Option<AppIds>,
    /// The player's preferences, by qualified id.
    prefs: VarMap,
    /// The screen for each role (12 §10.7), by qualified id.
    roles: BTreeMap<String, String>,
    runner: Runner,
    /// Whether something bindings may read has changed since they were
    /// last evaluated.
    stale: bool,
    /// The grid of the last frame built.
    size: (u16, u16),
    run_effects: bool,
    quit: bool,
    /// How many run-time warnings the bindings have raised.
    warnings: u64,
}

impl<H: SceneHost + fmt::Debug> fmt::Debug for Scene<H> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let screens: Vec<&str> = self.screens.iter().map(Screen::id).collect();
        f.debug_struct("Scene")
            .field("host", &self.host)
            .field("screens", &screens)
            .field("prefs", &self.prefs)
            .field("roles", &self.roles)
            .field("elapsed", &self.elapsed)
            .finish_non_exhaustive()
    }
}

/// Evaluates bindings for the scene: the compiled expressions, against
/// the game's reader, the application's state and the screen's own
/// names.
struct Reader<'a> {
    runner: &'a mut Runner,
    ui: &'a UiDefs,
    game: &'a mut dyn Host,
    view: ExprValue,
    party: ExprValue,
    app: Option<AppReader<'a>>,
    warnings: &'a mut u64,
}

/// What the machine asks while a binding runs.
struct BindHost<'r, 's> {
    game: &'r mut dyn Host,
    view: &'r ExprValue,
    party: &'r ExprValue,
    app: Option<&'r AppReader<'r>>,
    registry: &'r clash_expr::Registry,
    scope: &'s BindScope<'s>,
    warnings: &'r mut u64,
}

impl Host for BindHost<'_, '_> {
    fn subject(&mut self, slot: u16) -> ExprValue {
        match slot {
            SLOT_VIEW => self.view.clone(),
            SLOT_PARTY => self.party.clone(),
            SLOT_APP => self.app.map_or(ExprValue::Null, |app| app.ids.subject()),
            SLOT_SCREEN => ExprValue::map(
                self.scope
                    .vars
                    .iter()
                    .map(|(name, value)| (name.as_str(), from_var(value))),
            ),
            SLOT_IT => self.scope.it.cloned().unwrap_or_default(),
            _ => ExprValue::Null,
        }
    }

    fn local(&mut self, slot: u16) -> ExprValue {
        self.scope
            .params
            .get(usize::from(slot))
            .cloned()
            .unwrap_or_default()
    }

    fn event_field(&mut self, _slot: u16) -> ExprValue {
        ExprValue::Null
    }

    fn property(&mut self, property: PropertyId, subject: Handle, args: &[ExprValue]) -> ExprValue {
        match self.app {
            Some(app) if app.ids.owns(subject) => app.property(property, subject, args),
            _ => self.game.property(property, subject, args),
        }
    }

    fn variable(&mut self, subject: Handle, name: &str) -> ExprValue {
        self.game.variable(subject, name)
    }

    fn call(&mut self, function: FunctionId, args: &[ExprValue]) -> ExprValue {
        self.game.call(function, args)
    }

    fn rng(&mut self) -> Option<&mut Rng> {
        // Bindings are pure.
        None
    }

    fn warn(&mut self, _warning: RuntimeWarning) {
        *self.warnings += 1;
    }

    fn def_name(&self, id: DefRef) -> Option<&str> {
        self.registry.def_name(id)
    }
}

impl Reader<'_> {
    fn host<'r, 's>(&'r mut self, scope: &'s BindScope<'s>) -> (BindHost<'r, 's>, &'r mut Runner) {
        (
            BindHost {
                game: &mut *self.game,
                view: &self.view,
                party: &self.party,
                app: self.app.as_ref(),
                registry: self.ui.defs().exprs().registry(),
                scope,
                warnings: &mut *self.warnings,
            },
            &mut *self.runner,
        )
    }

    /// The value an effect gives: a literal, or an expression's.
    fn arg(&mut self, value: &ValueProp, scope: &BindScope<'_>) -> ExprValue {
        match value {
            Prop::Static(literal) => from_var(&literal.0),
            Prop::Bound(binding) => self.value(binding, scope).unwrap_or_default(),
        }
    }

    fn args(&mut self, args: &Args, scope: &BindScope<'_>) -> BTreeMap<String, ExprValue> {
        args.iter()
            .map(|(name, value)| (name.clone(), self.arg(value, scope)))
            .collect()
    }

    /// A text an effect gives.
    fn prop_text(&mut self, text: &TextProp, scope: &BindScope<'_>) -> String {
        match text {
            Prop::Static(text) => text.clone(),
            Prop::Bound(binding) => self
                .text(binding, scope)
                .unwrap_or_else(|| binding.source().to_owned()),
        }
    }
}

impl Evaluator for Reader<'_> {
    fn value(&mut self, binding: &Binding, scope: &BindScope<'_>) -> Option<ExprValue> {
        let code = binding.expr()?;
        let exprs = self.ui.defs().exprs();
        let (mut host, runner) = self.host(scope);
        Some(runner.over(exprs, &mut host).value(code))
    }

    fn text(&mut self, binding: &Binding, scope: &BindScope<'_>) -> Option<String> {
        let code = binding.template()?;
        let exprs = self.ui.defs().exprs();
        let (mut host, runner) = self.host(scope);
        Some(runner.over(exprs, &mut host).text(code))
    }
}

/// The scope of a screen's own names, with `it` as given.
fn scope_of<'a>(screen: &'a Screen, it: Option<&'a ExprValue>) -> BindScope<'a> {
    BindScope {
        screen: screen.id(),
        vars: screen.vars(),
        params: screen.params(),
        it,
    }
}

/// A scope with no screen behind it: what an effect is evaluated in once
/// its screen has closed.
fn no_screen<'a>(vars: &'a VarMap, it: Option<&'a ExprValue>) -> BindScope<'a> {
    BindScope {
        screen: "",
        vars,
        params: &[],
        it,
    }
}

impl<H: SceneHost> Scene<H> {
    /// A scene with no screens open, no definitions and no application.
    pub fn new(host: H, theme: Theme, keymap: KeymapDef) -> Scene<H> {
        Scene {
            host,
            app: Box::new(NoApp),
            theme,
            keymap,
            screens: Vec::new(),
            pending: Vec::new(),
            log: MessageLog::new(),
            rng: Rng::from_seed(SCENE_SEED),
            elapsed: 0.0,
            world: None,
            ui: None,
            app_ids: None,
            prefs: VarMap::new(),
            roles: BTreeMap::new(),
            runner: Runner::new(),
            stale: true,
            size: (0, 0),
            run_effects: false,
            quit: false,
            warnings: 0,
        }
    }

    /// Gives the scene the loaded definitions: from then on it can open
    /// screens by id, evaluate their bindings, run their effects and
    /// keep preferences, which start at their defaults and then at what
    /// the profile sets. The profile's `screens` name the screen for
    /// each role.
    #[must_use]
    pub fn with_defs(mut self, ui: UiDefs) -> Scene<H> {
        self.app_ids = AppIds::find(ui.defs().exprs().registry());
        self.prefs = ui.pref_defaults();
        self.roles = ui.profile_roles();
        self.ui = Some(ui);
        self.stale = true;
        self
    }

    /// Gives the scene the application's state: what `app` reads and
    /// what the `app` effect asks for.
    #[must_use]
    pub fn with_app(mut self, app: Box<dyn AppState>) -> Scene<H> {
        self.app = app;
        self.stale = true;
        self
    }

    /// Makes [`Scene::turn`] run the effects of the screens' events. A
    /// scene that does not, hands them back for its caller to run.
    #[must_use]
    pub fn running_effects(mut self) -> Scene<H> {
        self.run_effects = true;
        self
    }

    /// The loaded definitions, if the scene was given them.
    pub fn defs(&self) -> Option<&UiDefs> {
        self.ui.as_ref()
    }

    /// The application's state.
    pub fn app(&self) -> &dyn AppState {
        &*self.app
    }

    /// Gives the scene its view of the world: the view model, the camera
    /// and the glyph tables. From then on the `map_view` and
    /// `section_view` nodes of the screens draw it. Returns the view
    /// the scene had before.
    ///
    /// The view holds what was put into it and cannot read the
    /// simulation: see [`crate::view`].
    pub fn set_world(&mut self, world: WorldView) -> Option<WorldView> {
        self.world.replace(world)
    }

    /// Takes the scene's view of the world away. The views of a screen
    /// are then drawn as empty areas with their names.
    pub fn take_world(&mut self) -> Option<WorldView> {
        self.world.take()
    }

    /// The scene's view of the world, if it has one.
    pub fn world(&self) -> Option<&WorldView> {
        self.world.as_ref()
    }

    /// The scene's view of the world, to put what was perceived into its
    /// view model and to move its camera.
    pub fn world_mut(&mut self) -> Option<&mut WorldView> {
        self.world.as_mut()
    }

    /// Submits a command to the simulation: the scene's only way to
    /// change it.
    pub fn submit(&mut self, command: H::Command) -> Result<(), H::Refusal> {
        self.host.submit(command)
    }

    /// Tells the scene that the simulation has advanced, or that
    /// playback has reached a later moment: every binding on show is
    /// evaluated again before the next frame (12 §10.4).
    pub fn simulation_advanced(&mut self) {
        self.stale = true;
    }

    /// The theme in force.
    pub fn theme(&self) -> &Theme {
        &self.theme
    }

    /// The keymap in force.
    pub fn keymap(&self) -> &KeymapDef {
        &self.keymap
    }

    /// The message log.
    pub fn log(&self) -> &MessageLog {
        &self.log
    }

    /// The message log, to add lines to it.
    pub fn log_mut(&mut self) -> &mut MessageLog {
        &mut self.log
    }

    /// The scene's own random stream, for presentation only.
    pub fn rng(&mut self) -> &mut Rng {
        &mut self.rng
    }

    /// Whether an effect asked to quit.
    pub fn quit_requested(&self) -> bool {
        self.quit
    }

    /// How many run-time warnings bindings have raised: a null in
    /// arithmetic, a division by zero (05 §4.4, §4.5).
    pub fn binding_warnings(&self) -> u64 {
        self.warnings
    }

    /// Puts a screen on top of the stack.
    pub fn open(&mut self, mut screen: Screen) {
        self.push_prefs(&mut screen);
        self.screens.push(screen);
        self.pending.push(None);
        self.stale = true;
    }

    /// Takes the top screen off the stack. The one beneath is as it was
    /// left. Whatever waited for the closed screen's answer is dropped.
    pub fn close(&mut self) -> Option<Screen> {
        self.pending.pop();
        self.stale = true;
        self.screens.pop()
    }

    /// The open screens, bottom first.
    pub fn screens(&self) -> &[Screen] {
        &self.screens
    }

    /// The top screen: the one that receives input.
    pub fn top(&self) -> Option<&Screen> {
        self.screens.last()
    }

    /// The top screen, to change it.
    pub fn top_mut(&mut self) -> Option<&mut Screen> {
        self.stale = true;
        self.screens.last_mut()
    }

    /// The screen that answers a role (12 §10.7), by qualified id.
    pub fn role(&self, role: &str) -> Option<&str> {
        self.roles.get(role).map(String::as_str)
    }

    /// Names the screen that answers a role.
    pub fn set_role(&mut self, role: &str, screen: &str) {
        self.roles.insert(role.to_owned(), screen.to_owned());
    }

    /// The player's preferences, by qualified id.
    pub fn prefs(&self) -> &VarMap {
        &self.prefs
    }

    /// A preference's value.
    pub fn pref(&self, id: &str) -> Option<&Value> {
        self.prefs.get(id)
    }

    /// Opens the screen with this qualified id on top of the stack,
    /// with values for its parameters by name. A value for a parameter
    /// the screen does not have is an error, and so is a screen that
    /// cannot be made.
    pub fn open_screen(
        &mut self,
        id: &str,
        with: BTreeMap<String, ExprValue>,
    ) -> Result<(), String> {
        let Some(ui) = &self.ui else {
            return Err("the scene has no definitions to open a screen from".to_owned());
        };
        let mut screen = ui
            .screen(id, &self.theme)
            .map_err(|diagnostics| diagnostics.render())?;
        for (name, value) in with {
            if !screen.set_param(&name, value) {
                return Err(format!("the screen `{id}` has no parameter `{name}`"));
            }
        }
        self.open(screen);
        Ok(())
    }

    /// Copies the player's preferences into the widgets of a screen
    /// that are bound to them.
    fn push_prefs(&self, screen: &mut Screen) {
        let Some(ui) = &self.ui else { return };
        let namespace = namespace_of(screen.id()).to_owned();
        let bound: Vec<String> = screen
            .prefs()
            .iter()
            .map(|(key, _)| key.as_str().to_owned())
            .collect();
        for key in bound {
            if let Some(id) = ui.resolve::<PreferenceDef>(&key, &namespace)
                && let Some(value) = self.prefs.get(&id)
            {
                screen.set_pref(&key, value.clone());
            }
        }
    }

    /// Changes a preference (12 §10.4). The value must be one the
    /// preference can hold. Changing the preference named `theme`
    /// restyles every open screen, and changing the one named `keymap`
    /// changes what the keys mean; if the theme or the keymap cannot be
    /// used, the preference keeps its value and the reason is returned.
    /// The application is told, so that it can keep the choice.
    pub fn set_pref(&mut self, id: &str, value: Value) -> Result<(), String> {
        let Some(ui) = &self.ui else {
            return Err("the scene has no definitions, and so no preferences".to_owned());
        };
        let Some(def) = ui.get::<PreferenceDef>(id) else {
            return Err(format!("there is no preference `{id}`"));
        };
        if let Some(reason) = def.refuses(&value) {
            return Err(format!("the preference `{id}` cannot be {value}: {reason}"));
        }
        let bare = id.rsplit(':').next().unwrap_or(id);
        match (bare, &value) {
            (PREF_THEME, Value::Str(theme)) => {
                let theme = theme.clone();
                self.set_theme(&theme)?;
            }
            (PREF_KEYMAP, Value::Str(keymap)) => {
                let Some(ui) = &self.ui else { return Ok(()) };
                self.keymap = ui
                    .keymap(keymap)
                    .map_err(|diagnostics| diagnostics.render())?
                    .clone();
            }
            _ => {}
        }
        self.prefs.set(id, value.clone());
        let screens = core::mem::take(&mut self.screens);
        self.screens = screens
            .into_iter()
            .map(|mut screen| {
                self.push_prefs(&mut screen);
                screen
            })
            .collect();
        self.stale = true;
        self.app.service(&pref_request(id, &value))
    }

    /// Changes the theme in force to the one with this qualified id and
    /// restyles every open screen: each is made again from its
    /// definition and keeps its variables, focus and selections. If the
    /// theme cannot be built, or a screen cannot be made with it,
    /// nothing changes.
    pub fn set_theme(&mut self, id: &str) -> Result<(), String> {
        let Some(ui) = &self.ui else {
            return Err("the scene has no definitions to take a theme from".to_owned());
        };
        let theme = ui.theme(id).map_err(|diagnostics| diagnostics.render())?;
        let mut restyled = Vec::with_capacity(self.screens.len());
        for old in &self.screens {
            // A screen that was not made from a definition stays as it is.
            if ui.get::<ScreenDef>(old.id()).is_none() {
                restyled.push(old.clone());
                continue;
            }
            let mut screen = ui
                .screen(old.id(), &theme)
                .map_err(|diagnostics| diagnostics.render())?;
            screen.adopt(old);
            restyled.push(screen);
        }
        self.screens = restyled;
        self.theme = theme;
        self.stale = true;
        Ok(())
    }

    /// Passes input to the top screen and returns what it asks for,
    /// without running any effect: see [`Scene::handle`].
    pub fn input(&mut self, event: &InputEvent) -> Vec<UiEvent> {
        match self.screens.last_mut() {
            Some(screen) => {
                let events = screen.input(event, &self.keymap);
                if !events.is_empty() {
                    self.stale = true;
                }
                events
            }
            None => Vec::new(),
        }
    }

    /// Advances the scene by real time, in seconds. The application
    /// measures it; tests pass a fixed step (12 §12).
    pub fn advance(&mut self, seconds: f64) {
        if seconds.is_finite() && seconds > 0.0 {
            self.elapsed += seconds;
        }
    }

    /// Real time the scene has been advanced by, in seconds.
    pub fn elapsed(&self) -> f64 {
        self.elapsed
    }

    /// The first screen of the stack that is on show: the top one, or,
    /// while the screens above it are popups, one further down.
    fn lowest_shown(&self) -> usize {
        let mut first = self.screens.len().saturating_sub(1);
        while first > 0 && self.screens[first].popup().is_some() {
            first -= 1;
        }
        first
    }

    /// Runs `f` with something to evaluate bindings with and the
    /// screens. `None` if the scene has no definitions.
    fn with_reader<R>(&mut self, f: impl FnOnce(&mut Reader<'_>, &mut Vec<Screen>) -> R) -> Option<R> {
        let Scene {
            host,
            app,
            screens,
            ui,
            app_ids,
            prefs,
            runner,
            size,
            warnings,
            ..
        } = self;
        let ui = ui.as_ref()?;
        let (view, party) = (host.view(), host.party());
        let app = app_ids.as_ref().map(|ids| AppReader {
            ids,
            state: &**app,
            registry: ui.defs().exprs().registry(),
            prefs,
            size: *size,
        });
        let mut reader = Reader {
            runner,
            ui,
            game: host.reader(),
            view,
            party,
            app,
            warnings,
        };
        Some(f(&mut reader, screens))
    }

    /// Evaluates the bindings of every screen on show, if anything they
    /// may read has changed.
    fn refresh(&mut self) {
        if !self.stale {
            return;
        }
        self.stale = false;
        let first = self.lowest_shown();
        let theme = self.theme.clone();
        self.with_reader(|reader, screens| {
            for screen in screens.iter_mut().skip(first) {
                screen.refresh(reader, &theme);
            }
        });
    }

    /// Builds the frame: every screen on show, the lowest first, or an
    /// empty frame in the theme's colours when no screen is open. What
    /// is beneath a popup that asks for it is dimmed. The frame keeps
    /// its size and its storage.
    pub fn build_frame(&mut self, frame: &mut Frame) {
        if self.size != (frame.cols, frame.rows) {
            self.size = (frame.cols, frame.rows);
            self.stale = true;
        }
        self.refresh();
        frame.clear(FrameCell::blank(self.theme.base()));
        let data = UiData {
            log: Some(&self.log),
            views: self.world.as_ref().map(|world| world as &dyn ViewPainter),
        };
        let first = self.lowest_shown();
        for screen in self.screens.iter_mut().skip(first) {
            if screen.popup().is_some_and(|popup| popup.dim) {
                dim(frame);
            }
            screen.draw(frame, &data);
        }
    }
}

/// Dims everything drawn so far: each colour keeps a share of its
/// brightness, and on sixteen colours a character falls back to dark
/// grey (as a remembered cell does, 12 §6.1).
fn dim(frame: &mut Frame) {
    let scale = |channel: u8| ((u16::from(channel) * DIM_KEEPS) >> 8) as u8;
    for cell in &mut frame.cells {
        for color in [&mut cell.fg, &mut cell.bg] {
            color.rgb.r = scale(color.rgb.r);
            color.rgb.g = scale(color.rgb.g);
            color.rgb.b = scale(color.rgb.b);
        }
        cell.fg.fallback = crate::color::Ansi16::BrightBlack;
        cell.attrs.remove(crate::frame::Attrs::BOLD);
    }
}

impl<H> Scene<H>
where
    H: SceneHost,
    H::Command: From<UiCommand>,
    H::Refusal: fmt::Display,
{
    /// Passes input to the top screen and runs the effects it asks for
    /// (12 §10.5). Returns what they came to.
    pub fn handle(&mut self, event: &InputEvent) -> Vec<UiOutcome> {
        let events = self.input(event);
        self.run_events(&events)
    }

    /// Runs the effects of what a screen asked for.
    pub fn run_events(&mut self, events: &[UiEvent]) -> Vec<UiOutcome> {
        let mut out = Vec::new();
        for event in events {
            match event {
                UiEvent::Key { effects, .. } => self.run_into(effects, None, &mut out),
                UiEvent::Widget { effects, it, .. } => {
                    self.run_into(effects, it.clone(), &mut out);
                }
                UiEvent::Changed { var } => {
                    self.stale = true;
                    if let VarRef::Pref(key) = var {
                        self.pref_edited(key, &mut out);
                    }
                }
            }
        }
        out
    }

    /// A widget of the top screen changed the preference it is bound
    /// to: the scene takes the new value.
    fn pref_edited(&mut self, key: &str, out: &mut Vec<UiOutcome>) {
        let Some(top) = self.screens.last() else {
            return;
        };
        let namespace = namespace_of(top.id()).to_owned();
        let Some(value) = top.prefs().get(key).cloned() else {
            return;
        };
        let id = self
            .ui
            .as_ref()
            .and_then(|ui| ui.resolve::<PreferenceDef>(key, &namespace));
        let result = match id {
            Some(id) => self.set_pref(&id, value).map(|()| id),
            None => Err(format!("there is no preference `{key}`")),
        };
        match result {
            Ok(id) => out.push(UiOutcome::Changed(VarRef::Pref(id))),
            Err(reason) => {
                // The widget shows what the preference still is.
                let screens = core::mem::take(&mut self.screens);
                self.screens = screens
                    .into_iter()
                    .map(|mut screen| {
                        self.push_prefs(&mut screen);
                        screen
                    })
                    .collect();
                out.push(UiOutcome::Failed {
                    effect: "set",
                    reason,
                });
            }
        }
    }

    /// Runs a list of effects as the top screen's, with `it` as given.
    pub fn run(&mut self, effects: &UiEffects, it: Option<ExprValue>) -> Vec<UiOutcome> {
        let mut out = Vec::new();
        self.run_into(effects, it, &mut out);
        out
    }

    fn run_into(&mut self, effects: &UiEffects, it: Option<ExprValue>, out: &mut Vec<UiOutcome>) {
        let origin = self.screens.len().checked_sub(1);
        self.run_list(&effects.0, it, origin, out);
    }

    /// Works out a value in the screen that asked, or with no screen if
    /// it has closed.
    fn eval<R: Default>(
        &mut self,
        origin: Option<usize>,
        it: Option<&ExprValue>,
        f: impl FnOnce(&mut Reader<'_>, &BindScope<'_>) -> R,
    ) -> R {
        let none = VarMap::new();
        self.with_reader(|reader, screens| match origin.and_then(|at| screens.get(at)) {
            Some(screen) => f(reader, &scope_of(screen, it)),
            None => f(reader, &no_screen(&none, it)),
        })
        .unwrap_or_default()
    }

    /// The qualified id of a screen an effect names, seen from the
    /// screen that asks.
    fn screen_id(&self, name: &str, origin: Option<usize>) -> Result<String, String> {
        let namespace = origin
            .and_then(|at| self.screens.get(at))
            .map_or("", |screen| namespace_of(screen.id()));
        self.ui
            .as_ref()
            .and_then(|ui| ui.resolve::<ScreenDef>(name, namespace))
            .ok_or_else(|| format!("there is no screen `{name}`"))
    }

    /// Opens the screen for a role, with a waiting list of effects.
    fn open_role(
        &mut self,
        role: &str,
        with: BTreeMap<String, ExprValue>,
        pending: Pending,
    ) -> Result<String, String> {
        let Some(id) = self.roles.get(role).cloned() else {
            return Err(format!(
                "no screen answers the role `{role}`: the profile's `screens` names none"
            ));
        };
        self.open_screen(&id, with)?;
        if let Some(slot) = self.pending.last_mut() {
            *slot = Some(pending);
        }
        Ok(id)
    }

    fn run_list(
        &mut self,
        effects: &[UiEffect],
        it: Option<ExprValue>,
        mut origin: Option<usize>,
        out: &mut Vec<UiOutcome>,
    ) {
        self.stale = true;
        for (index, effect) in effects.iter().enumerate() {
            let failed = |reason: String| UiOutcome::Failed {
                effect: effect.name(),
                reason,
            };
            match effect {
                UiEffect::Open(open) | UiEffect::Replace(open) => {
                    let with = self.eval(origin, it.as_ref(), |r, scope| r.args(&open.with, scope));
                    let id = match self.screen_id(&open.screen, origin) {
                        Ok(id) => id,
                        Err(reason) => {
                            out.push(failed(reason));
                            continue;
                        }
                    };
                    if matches!(effect, UiEffect::Replace(_)) {
                        if let Some(closed) = self.close() {
                            out.push(UiOutcome::Closed(closed.id().to_owned()));
                        }
                        if origin.is_some_and(|at| at >= self.screens.len()) {
                            origin = None;
                        }
                    }
                    match self.open_screen(&id, with) {
                        Ok(()) => out.push(UiOutcome::Opened(id)),
                        Err(reason) => out.push(failed(reason)),
                    }
                }
                UiEffect::Close => {
                    if let Some(closed) = self.close() {
                        out.push(UiOutcome::Closed(closed.id().to_owned()));
                    }
                    if origin.is_some_and(|at| at >= self.screens.len()) {
                        origin = None;
                    }
                }
                UiEffect::Set(entries) => {
                    for (name, value) in entries {
                        let value =
                            self.eval(origin, it.as_ref(), |r, scope| r.arg(value, scope));
                        match self.assign(&VarRef::parse(name), &value, origin) {
                            Ok(var) => out.push(UiOutcome::Changed(var)),
                            Err(reason) => out.push(failed(reason)),
                        }
                    }
                }
                UiEffect::Focus(id) => {
                    let moved = origin
                        .and_then(|at| self.screens.get_mut(at))
                        .is_some_and(|screen| screen.set_focus(id));
                    if !moved {
                        out.push(failed(format!(
                            "no widget `{id}` on show can take the focus"
                        )));
                    }
                }
                UiEffect::Confirm(confirm) => {
                    let with = self.eval(origin, it.as_ref(), |r, scope| {
                        let mut with = BTreeMap::new();
                        let text = r.prop_text(&confirm.text, scope);
                        with.insert("text".to_owned(), ExprValue::text(text));
                        for (name, label) in [("yes", &confirm.yes), ("no", &confirm.no)] {
                            if let Some(label) = label {
                                let label = r.prop_text(label, scope);
                                with.insert(name.to_owned(), ExprValue::text(label));
                            }
                        }
                        with
                    });
                    let pending = Pending {
                        into: None,
                        rest: effects[index + 1..].to_vec(),
                        it: it.clone(),
                    };
                    match self.open_role(ROLE_CONFIRM, with, pending) {
                        Ok(id) => out.push(UiOutcome::Opened(id)),
                        Err(reason) => out.push(failed(reason)),
                    }
                    // Whatever follows waits for the answer.
                    return;
                }
                UiEffect::Pick(pick) => {
                    let with = self.eval(origin, it.as_ref(), |r, scope| {
                        let from = r.value(&pick.from, scope).unwrap_or_default();
                        let options: Vec<ExprValue> = from
                            .as_list()
                            .unwrap_or(&[])
                            .iter()
                            .map(|entry| {
                                let row = BindScope {
                                    it: Some(entry),
                                    ..*scope
                                };
                                let label = match &pick.item {
                                    Some(item) => r.prop_text(item, &row),
                                    None => entry.to_string(),
                                };
                                ExprValue::map([
                                    ("label", ExprValue::text(label)),
                                    ("value", entry.clone()),
                                ])
                            })
                            .collect();
                        let mut with = BTreeMap::new();
                        with.insert("options".to_owned(), ExprValue::list(options));
                        if let Some(title) = &pick.title {
                            let title = r.prop_text(title, scope);
                            with.insert("title".to_owned(), ExprValue::text(title));
                        }
                        with
                    });
                    let pending = Pending {
                        into: Some(pick.into.as_deref().map(VarRef::parse)),
                        rest: effects[index + 1..].to_vec(),
                        it: it.clone(),
                    };
                    match self.open_role(ROLE_PICK_LIST, with, pending) {
                        Ok(id) => out.push(UiOutcome::Opened(id)),
                        Err(reason) => out.push(failed(reason)),
                    }
                    return;
                }
                UiEffect::Answer(value) => {
                    let answer = self.eval(origin, it.as_ref(), |r, scope| r.arg(value, scope));
                    let waiting = self.pending.last_mut().and_then(Option::take);
                    if let Some(closed) = self.close() {
                        out.push(UiOutcome::Closed(closed.id().to_owned()));
                    }
                    let beneath = self.screens.len().checked_sub(1);
                    origin = None;
                    let Some(waiting) = waiting else { continue };
                    match waiting.into {
                        // A confirm: go on only on yes.
                        None => {
                            if answer.as_bool().unwrap_or(false) {
                                self.run_list(&waiting.rest, waiting.it, beneath, out);
                            }
                        }
                        // A pick: store the choice and go on with it.
                        Some(into) => {
                            if answer.is_null() {
                                continue;
                            }
                            if let Some(var) = into {
                                match self.assign(&var, &answer, beneath) {
                                    Ok(var) => out.push(UiOutcome::Changed(var)),
                                    Err(reason) => out.push(UiOutcome::Failed {
                                        effect: "pick",
                                        reason,
                                    }),
                                }
                            }
                            self.run_list(&waiting.rest, Some(answer), beneath, out);
                        }
                    }
                }
                UiEffect::Submit(command) => {
                    let args =
                        self.eval(origin, it.as_ref(), |r, scope| r.args(&command.args, scope));
                    let command = UiCommand {
                        kind: command.kind,
                        name: command.name.clone(),
                        args,
                        collect: command.collect,
                    };
                    match self.host.submit(H::Command::from(command.clone())) {
                        Ok(()) => out.push(UiOutcome::Submitted(command)),
                        Err(refusal) => out.push(UiOutcome::Refused {
                            command,
                            reason: refusal.to_string(),
                        }),
                    }
                }
                UiEffect::App(app) => {
                    let args = self.eval(origin, it.as_ref(), |r, scope| r.args(&app.args, scope));
                    let request = AppRequest {
                        service: app.service,
                        args,
                    };
                    match self.service(&request) {
                        Ok(()) => out.push(UiOutcome::Service(request)),
                        Err(reason) => out.push(failed(reason)),
                    }
                }
            }
        }
    }

    /// Stores a value in a screen variable of the screen that asked, or
    /// in a preference.
    fn assign(
        &mut self,
        var: &VarRef,
        value: &ExprValue,
        origin: Option<usize>,
    ) -> Result<VarRef, String> {
        let Some(held) = to_var(value) else {
            return Err(format!(
                "a variable holds true, false, a number or a text, and this is {value}"
            ));
        };
        match var {
            VarRef::Screen(name) => {
                let Some(screen) = origin.and_then(|at| self.screens.get_mut(at)) else {
                    return Err(format!(
                        "the screen that would hold `{name}` is closed"
                    ));
                };
                if !screen.set_var(name, held) {
                    return Err(format!("the screen has no variable `{name}`"));
                }
                self.stale = true;
                Ok(var.clone())
            }
            VarRef::Pref(key) => {
                let namespace = origin
                    .and_then(|at| self.screens.get(at))
                    .map_or("", |screen| namespace_of(screen.id()))
                    .to_owned();
                let id = self
                    .ui
                    .as_ref()
                    .and_then(|ui| ui.resolve::<PreferenceDef>(key, &namespace))
                    .ok_or_else(|| format!("there is no preference `{key}`"))?;
                self.set_pref(&id, held)?;
                Ok(VarRef::Pref(id))
            }
        }
    }

    /// Carries out an engine service: the scene's own part, then the
    /// application's.
    fn service(&mut self, request: &AppRequest) -> Result<(), String> {
        match request.service {
            AppService::SetPref => {
                let id = request
                    .text("pref")
                    .ok_or("`set_pref` needs the preference's id as a text")?
                    .to_owned();
                let id = self
                    .ui
                    .as_ref()
                    .and_then(|ui| ui.resolve::<PreferenceDef>(&id, ""))
                    .ok_or_else(|| format!("there is no preference `{id}`"))?;
                let value = request
                    .args
                    .get("value")
                    .and_then(to_var)
                    .ok_or("a preference holds true, false, a number or a text")?;
                // `set_pref` tells the application itself.
                return self.set_pref(&id, value);
            }
            AppService::Rebind => {
                let (Some(context), Some(key), Some(name)) = (
                    request.text("context"),
                    request.text("key"),
                    request.text("name"),
                ) else {
                    return Err("`rebind` needs `context`, `key` and `name` as texts".to_owned());
                };
                let chord = KeyChord::parse(key).map_err(|error| error.to_string())?;
                self.keymap
                    .contexts
                    .entry(context.to_owned())
                    .or_default()
                    .insert(chord, name.to_owned());
            }
            AppService::Quit => self.quit = true,
            _ => {}
        }
        self.app.service(request)
    }

    /// One pass of the application loop of 12 §3, without a simulation:
    /// takes the frontend's input, advances the scene by `seconds` of real
    /// time, builds a frame of the frontend's size and presents it.
    /// Returns what the input asked for and, if the scene runs effects
    /// ([`Scene::running_effects`]), what they came to.
    pub fn turn(&mut self, front: &mut dyn Frontend, frame: &mut Frame, seconds: f64) -> Turn {
        let mut turn = Turn::default();
        for event in front.poll() {
            if event == InputEvent::Close {
                turn.close_requested = true;
            }
            let events = self.input(&event);
            if self.run_effects {
                turn.outcomes.extend(self.run_events(&events));
            }
            turn.events.extend(events);
        }
        turn.close_requested |= self.quit;
        self.advance(seconds);
        let caps = front.caps();
        if (frame.cols, frame.rows) != (caps.cols, caps.rows) {
            frame.resize(caps.cols, caps.rows);
        }
        self.build_frame(frame);
        front.present(frame);
        turn
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_scene_with_no_screens_builds_an_empty_frame_in_the_theme() {
        let mut scene: Scene = Scene::new(NoHost, Theme::default(), KeymapDef::fallback());
        let mut frame = Frame::new(4, 2);
        frame.fx.shake = 1.0;
        scene.build_frame(&mut frame);
        assert_eq!(frame.text(), "    \n    \n");
        assert_eq!(frame.cells[0].style(), Theme::default().base());
        assert_eq!(frame.fx.shake, 0.0);
        assert!(scene.input(&InputEvent::Close).is_empty());
        assert!(scene.close().is_none());
    }

    #[test]
    fn time_only_moves_forward() {
        let mut scene: Scene = Scene::new(NoHost, Theme::default(), KeymapDef::fallback());
        scene.advance(0.25);
        scene.advance(-1.0);
        scene.advance(f64::NAN);
        scene.advance(0.5);
        assert_eq!(scene.elapsed(), 0.75);
        let first = scene.rng().next_u64();
        let mut again: Scene = Scene::new(NoHost, Theme::default(), KeymapDef::fallback());
        assert_eq!(
            again.rng().next_u64(),
            first,
            "the scene's stream is seeded the same every run"
        );
    }

    #[test]
    fn a_scene_with_no_simulation_refuses_commands_with_a_reason() {
        let mut scene: Scene = Scene::new(NoHost, Theme::default(), KeymapDef::fallback());
        let command = UiCommand {
            kind: CommandKind::Rule,
            name: "ashlands:ring_bell".to_owned(),
            args: BTreeMap::from([("times".to_owned(), ExprValue::Int(3))]),
            collect: Collect::Given,
        };
        let refusal = scene.submit(command).unwrap_err();
        assert_eq!(
            refusal.to_string(),
            "there is no simulation to submit `run ashlands:ring_bell times=3` to"
        );
    }
}
