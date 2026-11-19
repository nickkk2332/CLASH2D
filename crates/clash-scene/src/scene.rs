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
//! name and arguments as values. An `act` that asks for its parameters
//! is put together with pickers first ([`crate::control`]), and a
//! decision the simulation is waiting for is answered with one
//! ([`Scene::sync_decision`]). A host whose command can be made from
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

use clash_core::{EntityId, Rng, Tick, Value, VarMap};
use clash_data::Runner;
use clash_expr::{
    DefRef, FunctionId, Handle, Host, PropertyId, RuntimeWarning, Value as ExprValue,
};
use thiserror::Error;

use clash_sim::SimEvent;
use clash_world::Location;

use crate::app::{AppIds, AppReader, AppRequest, AppState, NoApp, pref_request};
use crate::control::{Collecting, ParamAsk, PendingDecision};
use crate::defs::{UiDefs, namespace_of};
use crate::frame::{Frame, FrameCell};
use crate::frontend::Frontend;
use crate::input::{CONTEXT_MENU, InputEvent, KeyChord, KeymapDef};
use crate::playback::{NAV_SKIP, Perceived, Playback, Shows};
use crate::theme::Theme;
use crate::ui::{
    AppService, Args, BindScope, Binding, CameraEffect, Collect, CommandKind, Evaluator,
    MessageLog, PreferenceDef, Prop, SLOT_APP, SLOT_IT, SLOT_PARTY, SLOT_SCREEN, SLOT_VIEW, Screen,
    ScreenDef, TextProp, UiData, UiEffect, UiEffects, UiEvent, ValueProp, VarRef, ViewPainter,
    from_var, to_var,
};
use crate::view::{CellContents, EntitySprite, SpriteId, ViewModel, WorldView};

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
    /// For an action: whether the player was still to be asked for its
    /// parameters (12 §9.3). The scene asks before it submits, so this
    /// is [`Collect::Given`] in every command it has put together; it
    /// is anything else only where the scene had no definition of the
    /// action to read, and hands the command over as it was written.
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

    /// The decision a directly controlled character is being asked for
    /// (03 §5.2), if one is waiting: the one the simulation returned
    /// from `advance`. The scene opens the screen for its kind and
    /// submits the answer as a command (12 §9.5).
    fn pending_decision(&self) -> Option<PendingDecision> {
        None
    }

    /// The value that stands for a cell in expressions and commands: a
    /// `loc`. The scene knows cells by their coordinates and the
    /// simulation by handles only it can make. Null where there is no
    /// simulation.
    fn cell_value(&mut self, _at: Location) -> ExprValue {
        ExprValue::Null
    }

    /// The cell a `loc` value stands for: the other way round from
    /// [`SceneHost::cell_value`]. A cell picker starts its cursor on the
    /// cell its parameter is filled in with.
    fn cell_of(&mut self, _value: &ExprValue) -> Option<Location> {
        None
    }

    /// The value that stands for a perceived character or object: what
    /// the party knows as the sprite with this number in the view
    /// model. Null if the number means nothing to the host.
    fn sprite_value(&mut self, _sprite: SpriteId) -> ExprValue {
        ExprValue::Null
    }

    /// The sprite the view model knows a perceived thing by, if the
    /// party perceives it somewhere: the other way round from
    /// [`SceneHost::sprite_value`].
    fn sprite_of(&mut self, _value: &ExprValue) -> Option<SpriteId> {
        None
    }

    /// What the acting character believes could be chosen for a
    /// parameter of an action (12 §9.3): the entries of a picker.
    /// `None` if the host has nothing to say, in which case an entity
    /// picker offers what the view model holds and a list picker is
    /// empty. Never truth: only what that character has perceived.
    fn candidates(&mut self, _ask: &ParamAsk<'_>) -> Option<Vec<ExprValue>> {
        None
    }

    /// How the directly controlled characters perceived an event
    /// (15 §13): seen, only heard, or not at all. Playback shows only
    /// what was perceived, in the manner it was perceived (12 §5).
    /// `picture` is the view model as playback has it, for a host that
    /// decides by what is in view. A host that says nothing shows
    /// nothing.
    fn perceived(&mut self, _event: &SimEvent, _picture: &ViewModel) -> Perceived {
        Perceived::Not
    }

    /// What the party sees in a cell now: asked when an event that was
    /// seen changes it. `None` if the cell is not in view, in which
    /// case what is known of it stays as it is.
    fn seen_cell(&mut self, _at: Location) -> Option<CellContents> {
        None
    }

    /// How the party perceives an entity: the number its sprite is
    /// known by in the view model, and how it looks now, or `None` in
    /// place of the look if it is not perceived now. `None` altogether
    /// for an entity the party has never perceived, which playback then
    /// leaves alone. Asked when an event that was seen is about the
    /// entity.
    fn seen_sprite(&mut self, _entity: EntityId) -> Option<(SpriteId, Option<EntitySprite>)> {
        None
    }

    /// Tells the host the moment playback has reached, before bindings
    /// are evaluated: the instant before the next event to be played,
    /// or `None` once playback has caught up with the simulation. A
    /// host that can answer its reader as of a moment does, so that a
    /// bar does not drop before the blow is seen (12 §10.4); one that
    /// cannot answers as of the present.
    fn as_of(&mut self, _moment: Option<Tick>) {}
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

    fn property(
        &mut self,
        _property: PropertyId,
        _subject: Handle,
        _args: &[ExprValue],
    ) -> ExprValue {
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
    /// A command that was being put together was abandoned: a picker
    /// asking for one of its parameters was closed. Nothing was
    /// submitted.
    Cancelled {
        /// The action.
        command: String,
    },
    /// The cursor or the view of the world was moved.
    Camera(CameraEffect),
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

/// What a screen that was opened to ask something is asking for.
#[derive(Clone, Debug)]
pub(crate) enum Wait {
    /// A `confirm`: yes or no.
    Confirm,
    /// A `pick`: a choice, and where it is stored.
    Pick(Option<VarRef>),
    /// Which action an `act` is to start.
    Action,
    /// A parameter of the command being put together.
    Param,
    /// The answer to a decision the simulation asked for.
    Decision,
}

/// What waits for the answer of a screen opened to ask something.
#[derive(Clone, Debug)]
pub(crate) struct Pending {
    /// What is being asked.
    pub(crate) wait: Wait,
    /// The effects that follow in the list that asked.
    pub(crate) rest: Vec<UiEffect>,
    /// What `it` was for that list.
    pub(crate) it: Option<ExprValue>,
}

impl Pending {
    /// Something asked with no effects waiting behind it.
    pub(crate) fn bare(wait: Wait) -> Pending {
        Pending {
            wait,
            rest: Vec::new(),
            it: None,
        }
    }
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
    pub(crate) host: H,
    pub(crate) app: Box<dyn AppState>,
    pub(crate) theme: Theme,
    pub(crate) keymap: KeymapDef,
    /// The namespace of the keymap in force: where a bare control name
    /// is looked up first.
    pub(crate) keymap_ns: String,
    pub(crate) screens: Vec<Screen>,
    /// For each open screen, what waits for its answer.
    pub(crate) pending: Vec<Option<Pending>>,
    /// Counts every screen opened and closed, so that a list of effects
    /// can tell that the stack changed beneath it.
    pub(crate) stack_gen: u64,
    pub(crate) log: MessageLog,
    pub(crate) rng: Rng,
    pub(crate) elapsed: f64,
    pub(crate) world: Option<WorldView>,
    pub(crate) ui: Option<UiDefs>,
    pub(crate) app_ids: Option<AppIds>,
    /// The player's preferences, by qualified id.
    pub(crate) prefs: VarMap,
    /// The screen for each role (12 §10.7), by qualified id.
    pub(crate) roles: BTreeMap<String, String>,
    pub(crate) runner: Runner,
    /// Whether something bindings may read has changed since they were
    /// last evaluated.
    pub(crate) stale: bool,
    /// The grid of the last frame built.
    pub(crate) size: (u16, u16),
    pub(crate) run_effects: bool,
    pub(crate) quit: bool,
    /// How many run-time warnings the bindings have raised.
    pub(crate) warnings: u64,
    /// The command being put together by pickers, if one is (12 §9.3).
    pub(crate) collecting: Option<Collecting>,
    /// The decision the scene has shown, so that it is shown once
    /// (12 §9.5).
    pub(crate) deciding: Option<PendingDecision>,
    /// The queue of what happened, played over real time (12 §7).
    pub(crate) playback: Playback,
    /// The `show` definitions under the theme in force.
    pub(crate) shows: Shows,
    /// The qualified id of the preference that scales playback.
    pub(crate) pref_speed: Option<String>,
    /// The qualified id of the preference that says which animations
    /// play.
    pub(crate) pref_effects: Option<String>,
    /// The creature selected as the target (12 §9.2): what `app.target`
    /// reads while the party still perceives it.
    pub(crate) target: Option<SpriteId>,
    /// For each show whose `heard` asks for a quiet, when something it
    /// is about was last heard (12 §7.4, decision D-945).
    pub(crate) heard_last: BTreeMap<crate::playback::ShowId, clash_core::Tick>,
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
pub(crate) struct Reader<'a> {
    runner: &'a mut Runner,
    ui: &'a UiDefs,
    theme: &'a Theme,
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
    pub(crate) fn arg(&mut self, value: &ValueProp, scope: &BindScope<'_>) -> ExprValue {
        match value {
            Prop::Static(literal) => from_var(&literal.0),
            Prop::Bound(binding) => self.value(binding, scope).unwrap_or_default(),
        }
    }

    pub(crate) fn args(
        &mut self,
        args: &Args,
        scope: &BindScope<'_>,
    ) -> BTreeMap<String, ExprValue> {
        args.iter()
            .map(|(name, value)| (name.clone(), self.arg(value, scope)))
            .collect()
    }

    /// The value of an expression an effect holds: null if it was never
    /// compiled.
    pub(crate) fn bound(&mut self, binding: &Binding, scope: &BindScope<'_>) -> ExprValue {
        self.value(binding, scope).unwrap_or_default()
    }

    /// What the reader calls a value: the built-in `name` (05 §4.6),
    /// which the host answers. Empty if the host has no name for it.
    pub(crate) fn name_of(&mut self, value: &ExprValue) -> String {
        let exprs = self.ui.defs().exprs();
        let none = VarMap::new();
        let scope = no_screen(&none, None);
        let (mut host, runner) = self.host(&scope);
        match runner
            .over(exprs, &mut host)
            .call_named("name", core::slice::from_ref(value))
        {
            ExprValue::Null => String::new(),
            name => name.to_string(),
        }
    }

    /// A text an effect gives.
    pub(crate) fn prop_text(&mut self, text: &TextProp, scope: &BindScope<'_>) -> String {
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
            keymap_ns: String::new(),
            screens: Vec::new(),
            pending: Vec::new(),
            stack_gen: 0,
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
            collecting: None,
            deciding: None,
            playback: Playback::new(),
            shows: Shows::new(),
            pref_speed: None,
            pref_effects: None,
            target: None,
            heard_last: BTreeMap::new(),
        }
    }

    /// The creature selected as the target with the `camera` effect's
    /// `target`, while the party perceives it as one: in the view model,
    /// not only suspected, and not a body the party sees lying dead,
    /// which is drawn as a thing (decision D-944). `None` otherwise.
    pub fn target(&self) -> Option<SpriteId> {
        let id = self.target?;
        let sprite = self.world.as_ref()?.model().sprite(id)?;
        (!sprite.suspected && sprite.kind == crate::view::SpriteKind::Creature).then_some(id)
    }

    /// Selects a creature as the target, or none.
    pub fn set_target(&mut self, target: Option<SpriteId>) {
        self.target = target;
        self.stale = true;
    }

    /// Says which `keymap` definition the keymap in force is, by its
    /// qualified id: a bare control name is looked up in its namespace
    /// first (12 §9.2).
    #[must_use]
    pub fn with_keymap_id(mut self, id: &str) -> Scene<H> {
        self.keymap_ns = namespace_of(id).to_owned();
        self
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
        self.ready_playback();
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
        self.stack_gen += 1;
        self.stale = true;
    }

    /// Takes the top screen off the stack. The one beneath is as it was
    /// left. Whatever waited for the closed screen's answer is dropped:
    /// if it was a picker asking for a parameter, the whole command is
    /// abandoned and nothing is submitted (12 §9.3).
    pub fn close(&mut self) -> Option<Screen> {
        self.close_top().0
    }

    /// Closes the top screen, and names the command that was abandoned
    /// with it, if one was.
    pub(crate) fn close_top(&mut self) -> (Option<Screen>, Option<String>) {
        let waiting = self.pending.pop().flatten();
        let screen = self.pop_screen();
        let abandoned = match waiting.map(|pending| pending.wait) {
            Some(Wait::Param | Wait::Action) => self.abandon_command(),
            Some(Wait::Decision) => {
                // Still to be answered: it is shown again.
                self.deciding = None;
                None
            }
            _ => None,
        };
        (screen, abandoned)
    }

    /// Takes the top screen off the stack and nothing more. The caller
    /// has dealt with what waited for it.
    pub(crate) fn pop_screen(&mut self) -> Option<Screen> {
        self.stack_gen += 1;
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
    /// with values by name for its parameters and for its variables: a
    /// variable given a value starts with it, so that a screen that
    /// edits something starts from what it edits (12 §10.5). A value
    /// for a name the screen has as neither is an error, and so is one
    /// a variable cannot hold and a screen that cannot be made.
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
            let var = screen.var(&name).cloned();
            if let Some(held) = &var {
                match to_var(&value) {
                    Some(start) if same_kind(held, &start) => {
                        screen.set_var(&name, start);
                    }
                    _ => {
                        return Err(format!(
                            "the variable `{name}` of the screen `{id}` cannot start as {value}"
                        ));
                    }
                }
            }
            let param = screen.set_param(&name, value);
            if var.is_none() && !param {
                return Err(format!(
                    "the screen `{id}` has no parameter or variable `{name}`"
                ));
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
                self.keymap_ns = namespace_of(keymap).to_owned();
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
        self.ready_playback();
        self.stale = true;
        Ok(())
    }

    /// Passes input to the top screen and returns what it asks for,
    /// without running any effect: see [`Scene::handle`].
    ///
    /// While anything is being played back, a key press first jumps
    /// playback to its end (12 §7.1). If the keymap in force gives some
    /// key the name `skip`, only that key does, and it does nothing
    /// else.
    pub fn input(&mut self, event: &InputEvent) -> Vec<UiEvent> {
        if let InputEvent::Key(chord) = event
            && !self.playback.is_idle()
        {
            let context = self.screens.last().map_or(CONTEXT_MENU, Screen::context);
            let named = |context: &str| self.keymap.name(context, *chord) == Some(NAV_SKIP);
            let skips = named(context) || named(CONTEXT_MENU);
            let bound = self
                .keymap
                .contexts
                .values()
                .any(|keys| keys.values().any(|name| name == NAV_SKIP));
            if skips || !bound {
                self.skip_playback();
            }
            if skips {
                return Vec::new();
            }
        }
        let Scene {
            screens,
            keymap,
            keymap_ns,
            ui,
            stale,
            ..
        } = self;
        match screens.last_mut() {
            Some(screen) => {
                // A name nothing on the screen used may be a control's.
                let controls = |name: &str| ui.as_ref().and_then(|ui| ui.control(name, keymap_ns));
                let events = screen.input_with(event, keymap, &controls);
                if !events.is_empty() {
                    *stale = true;
                }
                events
            }
            None => Vec::new(),
        }
    }

    /// Gives a key a meaning in a context of the keymap in force, at
    /// once (12 §9.2): `name` is what the key is to mean there, a
    /// control's name or one of the toolkit's, and an empty `name` takes
    /// the key's meaning away. Controls are untouched. The `rebind`
    /// service does this and then tells the application, which keeps
    /// the player's bindings between runs and gives them back through
    /// this function when it starts.
    pub fn rebind(&mut self, context: &str, key: &str, name: &str) -> Result<(), String> {
        let chord = KeyChord::parse(key).map_err(|error| error.to_string())?;
        if context.is_empty() {
            return Err("`rebind` needs the context the key is bound in".to_owned());
        }
        let keys = self.keymap.contexts.entry(context.to_owned()).or_default();
        if name.is_empty() {
            keys.remove(&chord);
        } else {
            keys.insert(chord, name.to_owned());
        }
        Ok(())
    }

    /// Advances the scene by real time, in seconds. The application
    /// measures it; tests pass a fixed step (12 §12). Playback plays
    /// what falls due in that time into the picture of the world and
    /// the log (12 §7): see [`crate::playback`].
    pub fn advance(&mut self, seconds: f64) {
        if seconds.is_finite() && seconds > 0.0 {
            self.elapsed += seconds;
        }
        let speed = self.playback_speed();
        let played = self
            .playback
            .advance(seconds, speed, self.world.as_mut(), &mut self.log);
        if played.changed || played.events > 0 {
            // The moment bindings on the view show has moved on.
            self.stale = true;
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
    pub(crate) fn with_reader<R>(
        &mut self,
        f: impl FnOnce(&mut Reader<'_>, &mut Vec<Screen>) -> R,
    ) -> Option<R> {
        let Scene {
            host,
            app,
            theme,
            screens,
            ui,
            app_ids,
            prefs,
            runner,
            size,
            warnings,
            keymap,
            keymap_ns,
            target,
            world,
            ..
        } = self;
        let ui = ui.as_ref()?;
        let (view, party) = (host.view(), host.party());
        // What `app.target` reads: the selected creature while the
        // party perceives it.
        let target_value = target
            .filter(|id| {
                world
                    .as_ref()
                    .and_then(|world| world.model().sprite(*id))
                    .is_some_and(|sprite| !sprite.suspected)
            })
            .map_or(ExprValue::Null, |id| host.sprite_value(id));
        let app = app_ids.as_ref().map(|ids| AppReader {
            target: target_value,
            ids,
            state: &**app,
            registry: ui.defs().exprs().registry(),
            prefs,
            size: *size,
            keymap,
            keymap_ns,
            ui,
        });
        let mut reader = Reader {
            runner,
            ui,
            theme,
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
        // Bindings on the view show the moment playback has reached.
        self.host.as_of(self.playback.moment());
        let first = self.lowest_shown();
        self.with_reader(|reader, screens| {
            let theme = reader.theme;
            for screen in screens.iter_mut().skip(first) {
                screen.refresh(reader, theme);
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
        if self.stale {
            // The simulation may have moved what the cursor is over.
            self.push_status();
        }
        // The selected target is marked where the party perceives it.
        let marked = self.target().and_then(|id| {
            let world = self.world.as_ref()?;
            Some(world.model().sprite(id)?.loc)
        });
        if let Some(world) = &mut self.world {
            world.set_marked(marked);
        }
        self.refresh();
        frame.clear(FrameCell::blank(self.theme.base()));
        // What playback is in the middle of showing: drawn over the
        // views of the world and given to the frame.
        let effects = self.playback.render(self.world.as_mut());
        let data = UiData {
            log: Some(&self.log),
            views: self.world.as_ref().map(|world| world as &dyn ViewPainter),
        };
        let first = self.lowest_shown();
        for screen in self.screens.iter_mut().skip(first) {
            if screen.popup().is_some_and(|popup| popup.dim) {
                dim(frame);
            }
            // A sprite of a screen beneath does not show through this
            // one.
            let beneath = frame.sprites.len();
            screen.draw(frame, &data);
            if beneath > 0 {
                let area = screen.area();
                let mut kept = 0;
                for index in 0..beneath {
                    let (x, y) = frame.sprites[index].cell();
                    if !area.contains(x, y) {
                        frame.sprites.swap(kept, index);
                        kept += 1;
                    }
                }
                frame.sprites.drain(kept..beneath);
            }
        }
        frame.fx.shake = effects.shake;
        frame.fx.flash = effects.flash;
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
                    match var {
                        VarRef::Pref(key) => self.pref_edited(key, &mut out),
                        VarRef::Screen(_) => out.push(UiOutcome::Changed(var.clone())),
                    }
                }
                UiEvent::Control { effects, .. } => self.run_into(effects, None, &mut out),
                UiEvent::ViewPick {
                    kind,
                    zoom,
                    area,
                    at,
                    second,
                    effects,
                    ..
                } => self.view_picked(*kind, *zoom, *area, *at, *second, effects, &mut out),
                UiEvent::ViewWheel { notches } => {
                    let by = CameraEffect {
                        zoom: i32::from(notches.signum()),
                        ..CameraEffect::default()
                    };
                    if self.move_camera(&by).is_ok() {
                        out.push(UiOutcome::Camera(by));
                    }
                }
            }
        }
        self.push_status();
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

    pub(crate) fn run_into(
        &mut self,
        effects: &UiEffects,
        it: Option<ExprValue>,
        out: &mut Vec<UiOutcome>,
    ) {
        let origin = self.screens.len().checked_sub(1);
        self.run_list(&effects.0, it, origin, out);
    }

    /// Works out a value in the screen that asked, or with no screen if
    /// it has closed.
    pub(crate) fn eval<R: Default>(
        &mut self,
        origin: Option<usize>,
        it: Option<&ExprValue>,
        f: impl FnOnce(&mut Reader<'_>, &BindScope<'_>) -> R,
    ) -> R {
        let none = VarMap::new();
        self.with_reader(
            |reader, screens| match origin.and_then(|at| screens.get(at)) {
                Some(screen) => f(reader, &scope_of(screen, it)),
                None => f(reader, &no_screen(&none, it)),
            },
        )
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

    /// Opens the screen for a role as [`Scene::open_role`] does, giving
    /// it only those of `with` that it declares as parameters: a picker
    /// or a prompt takes what it has a use for.
    pub(crate) fn open_role_with(
        &mut self,
        role: &str,
        with: BTreeMap<String, ExprValue>,
        pending: Option<Pending>,
    ) -> Result<String, String> {
        let Some(id) = self.roles.get(role).cloned() else {
            return Err(format!(
                "no screen answers the role `{role}`: the profile's `screens` names none"
            ));
        };
        let Some(ui) = &self.ui else {
            return Err("the scene has no definitions to open a screen from".to_owned());
        };
        let mut screen = ui
            .screen(&id, &self.theme)
            .map_err(|diagnostics| diagnostics.render())?;
        for (name, value) in with {
            // A parameter the screen does not declare is not its concern.
            screen.set_param(&name, value);
        }
        self.open(screen);
        if let Some(slot) = self.pending.last_mut() {
            *slot = pending;
        }
        Ok(id)
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

    pub(crate) fn run_list(
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
                    let (closed, abandoned) = self.close_top();
                    if let Some(closed) = closed {
                        out.push(UiOutcome::Closed(closed.id().to_owned()));
                    }
                    if let Some(command) = abandoned {
                        out.push(UiOutcome::Cancelled { command });
                    }
                    if origin.is_some_and(|at| at >= self.screens.len()) {
                        origin = None;
                    }
                }
                UiEffect::Set(entries) => {
                    for (name, value) in entries {
                        let value = self.eval(origin, it.as_ref(), |r, scope| r.arg(value, scope));
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
                        wait: Wait::Confirm,
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
                        wait: Wait::Pick(pick.into.as_deref().map(VarRef::parse)),
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
                    let asked = self
                        .pending
                        .last()
                        .and_then(Option::as_ref)
                        .map(|pending| pending.wait.clone());
                    // An answer to a picker of a command being put
                    // together, or to a decision, is the scene's to
                    // take: the screen may stay open.
                    let stack = self.stack_gen;
                    match asked {
                        Some(Wait::Param) => self.param_answered(answer, out),
                        Some(Wait::Action) => self.action_answered(answer, out),
                        Some(Wait::Decision) => self.decision_answered(answer, out),
                        _ => {
                            let waiting = self.pending.last_mut().and_then(Option::take);
                            if let Some(closed) = self.close() {
                                out.push(UiOutcome::Closed(closed.id().to_owned()));
                            }
                            let beneath = self.screens.len().checked_sub(1);
                            match waiting {
                                // A confirm: go on only on yes.
                                Some(Pending {
                                    wait: Wait::Confirm,
                                    rest,
                                    it,
                                }) => {
                                    if answer.as_bool().unwrap_or(false) {
                                        self.run_list(&rest, it, beneath, out);
                                    }
                                }
                                // A pick: store the choice and go on
                                // with it.
                                Some(Pending {
                                    wait: Wait::Pick(into),
                                    rest,
                                    ..
                                }) if !answer.is_null() => {
                                    if let Some(var) = into {
                                        match self.assign(&var, &answer, beneath) {
                                            Ok(var) => out.push(UiOutcome::Changed(var)),
                                            Err(reason) => out.push(UiOutcome::Failed {
                                                effect: "pick",
                                                reason,
                                            }),
                                        }
                                    }
                                    self.run_list(&rest, Some(answer), beneath, out);
                                }
                                _ => {}
                            }
                        }
                    }
                    if self.stack_gen != stack {
                        origin = None;
                    }
                }
                UiEffect::Submit(command)
                    if command.kind == CommandKind::Act
                        && (command.collect != Collect::Given || command.choose.is_some()) =>
                {
                    // The player is asked: what follows in the list
                    // waits for the command to be submitted.
                    let rest = effects[index + 1..].to_vec();
                    self.begin_act(command, it.clone(), origin, rest, out);
                    return;
                }
                UiEffect::Submit(command) => {
                    let args =
                        self.eval(origin, it.as_ref(), |r, scope| r.args(&command.args, scope));
                    let name = match command.kind {
                        // An action is known to the simulation by its
                        // qualified id, where the scene can tell it.
                        CommandKind::Act => self
                            .action_id(&command.name, origin)
                            .unwrap_or_else(|| command.name.clone()),
                        _ => command.name.clone(),
                    };
                    let command = UiCommand {
                        kind: command.kind,
                        name,
                        args,
                        collect: command.collect,
                    };
                    self.submit_command(command, out);
                }
                UiEffect::Camera(camera) => match self.move_camera(camera) {
                    Ok(()) => out.push(UiOutcome::Camera(*camera)),
                    Err(reason) => out.push(failed(reason)),
                },
                UiEffect::Picker(picker) => {
                    let value = match picker {
                        crate::ui::PickerEffect::Toggle(value) => {
                            self.eval(origin, it.as_ref(), |r, scope| r.arg(value, scope))
                        }
                        _ => ExprValue::Null,
                    };
                    let stack = self.stack_gen;
                    self.picker_effect(picker, value, out);
                    if self.stack_gen != stack {
                        origin = None;
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

    /// Hands a finished command to the simulation and reports what came
    /// of it. Returns whether it was accepted.
    pub(crate) fn submit_command(&mut self, command: UiCommand, out: &mut Vec<UiOutcome>) -> bool {
        match self.host.submit(H::Command::from(command.clone())) {
            Ok(()) => {
                out.push(UiOutcome::Submitted(command));
                true
            }
            Err(refusal) => {
                out.push(UiOutcome::Refused {
                    command,
                    reason: refusal.to_string(),
                });
                false
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
                    return Err(format!("the screen that would hold `{name}` is closed"));
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
                // At once; the application is then told, to keep it.
                self.rebind(context, key, name)?;
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
        if self.run_effects && self.playback.is_caught_up() {
            // A decision the simulation is waiting for gets its screen
            // once playback has reached it (12 §9.5).
            turn.outcomes.extend(self.sync_decision());
        }
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

/// Whether a screen variable that holds one value may be given another:
/// a flag for a flag, a number for a number, a text for a text.
fn same_kind(held: &Value, given: &Value) -> bool {
    matches!(
        (held, given),
        (Value::Bool(_), Value::Bool(_))
            | (
                Value::Int(_) | Value::Real(_),
                Value::Int(_) | Value::Real(_)
            )
            | (Value::Str(_), Value::Str(_))
    )
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
