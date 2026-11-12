//! The scene's handle on the simulation (12 §3, invariant V1).
//!
//! [`GameHost`] is what the scene is given as its `SceneHost`: it can
//! submit a command, and it can read. It holds no simulation itself. It
//! is a handle on the application's shared state ([`crate::app`]), in
//! which the game being played lives, and borrows it for the length of
//! each call. With no game being played it reads nothing and refuses
//! every command.
//!
//! | The scene asks | The host answers |
//! |----------------|------------------|
//! | `view`, `party` | The player's character, as the simulation's value for it |
//! | `reader` | Itself, as a `clash_expr::Host` that reads the simulation and writes nothing. Bindings of ordinary screens were compiled in the mind context of the party (05 §7.3), so what they can ask for is already held to what the character may know. |
//! | `submit` | [`command_for`]: the one place a `UiCommand` becomes a command of the simulation |
//! | `pending_decision` | The front of the simulation's decision queue (03 §5.2): a free decision, or a choice a rule's `ask` put |
//! | `perceived` | How the party perceived the event: the simulation's tag of it (`Sim::party_perceived`, 15 §13) |
//! | `seen_cell`, `seen_sprite` | What is in a cell of the party's view and how a thing in view is perceived, as the batch ended ([`crate::perception`]) |
//! | `as_of` | Nothing: the simulation keeps no history, so bindings are answered as of the present (D-605) |
//! | `cell_value`, `cell_of` | The simulation's `loc` subject for a cell, and back |
//! | `sprite_value`, `sprite_of` | The entity whose id is the sprite's number ([`crate::perception`] numbers sprites so), and back |
//! | `candidates` | Nothing: an entity picker then offers what the view model holds |
//!
//! # The commands
//!
//! | The interface submits | The simulation is given |
//! |-----------------------|-------------------------|
//! | `act` of an action, with its parameters | `Act` with `start`, for the player's character: the action by its qualified id (a bare name means the first action of that name in order of qualified id), the parameters by name as the scene assembled them |
//! | `command wait { dur }` | `Wait` for `dur` seconds of the character's own time |
//! | `command cancel { action, abandon? }` | `Cancel` of one of the character's running actions, by its number |
//! | `command clear_plan` | `ClearPlan` |
//! | `command choose { entity, choice, option }` | `Choose`: the answer to a choice prompt (12 §9.5) |
//! | `command react`, `command pass`, a `run` of a rule | Refused, with the reason: windows (03 §7) and the `Ui` command are not built |
//!
//! A refusal is the simulation's reason, which the frame loop writes in
//! the log. A step refused because the way is closed is noted
//! ([`crate::session::Session::bumped`]), and the frame loop offers what
//! is there instead ([`crate::interact`]).
//!
//! # The name of the player's character
//!
//! A character's name is its `Identity`, which its templates give. Where
//! none of them has a choice that sets one, the built-in `name` of the
//! player's own character is answered here with what the player typed
//! on the new-game screen.

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;

use clash_core::{Dur, EntityId, Rng, Tick};
use clash_data::Defs;
use clash_expr::{DefRef, FunctionId, Handle, Host, PropertyId, RuntimeWarning, Value};
use clash_scene::ui::CommandKind;
use clash_scene::view::{CellContents, EntitySprite, SpriteId, ViewModel};
use clash_scene::{DecisionAsk, ParamAsk, PendingDecision, Perceived, SceneHost, UiCommand};
use clash_sim::action::{ActionDef, ActionDefId, How};
use clash_sim::decision::ChoiceId;
use clash_sim::{ActionId, Command, DecisionKind, SimEvent, Subject};
use clash_world::Location;

use crate::app::Shared;
use crate::perception;
use crate::session::{Session, player_value};

/// The built-in that gives what the reader calls something (05 §4.6).
const FN_NAME: &str = "name";

/// Why a command is refused when no game is being played.
pub const NO_GAME: &str = "No game is being played.";

/// The command that lets time pass.
pub const COMMAND_WAIT: &str = "wait";
/// The command that ends one of the character's actions.
pub const COMMAND_CANCEL: &str = "cancel";
/// The command that empties the character's plan.
pub const COMMAND_CLEAR_PLAN: &str = "clear_plan";
/// The command that answers a choice.
pub const COMMAND_CHOOSE: &str = clash_scene::control::COMMAND_CHOOSE;

/// The part of a qualified id after its namespace.
fn bare(name: &str) -> &str {
    name.rsplit(':').next().unwrap_or(name)
}

/// An action definition by the name a command gives: its qualified id,
/// or a bare name, which means the first action of that name in order
/// of qualified id (12 §9.2).
pub fn action_named(defs: &Defs, name: &str) -> Option<ActionDefId> {
    let actions = defs.try_registry::<ActionDef>()?;
    if let Some(id) = actions.lookup(name) {
        return Some(id);
    }
    if name.contains(':') {
        return None;
    }
    let qualified = actions
        .names()
        .iter()
        .filter(|qualified| bare(qualified) == name)
        .min()?;
    actions.lookup(qualified)
}

/// An argument of a command as a whole number, if it is one.
fn int_arg(command: &UiCommand, name: &str) -> Option<i64> {
    command.args.get(name).and_then(Value::as_int)
}

/// The command of the simulation a command of the interface asks for,
/// or why there is none. See the [module](self) documentation.
fn build(session: &Session, command: &UiCommand) -> Result<Command, String> {
    let player = session.player;
    match command.kind {
        CommandKind::Act => {
            let action = action_named(session.sim.defs(), &command.name).ok_or_else(|| {
                format!(
                    "Nothing can do `{}`: no loaded mod defines that action.",
                    command.name
                )
            })?;
            let params = command
                .args
                .iter()
                .map(|(name, value)| (name.clone(), value.to_stored()))
                .collect();
            Ok(Command::Act {
                entity: player,
                action,
                params,
                how: How::Start,
            })
        }
        CommandKind::Command => match command.name.as_str() {
            COMMAND_WAIT => {
                let seconds = command
                    .args
                    .get("dur")
                    .and_then(Value::as_number)
                    .ok_or("`wait` needs `dur`, how long to wait in seconds.")?;
                Ok(Command::Wait {
                    entity: player,
                    dur: Dur::from_secs_real(seconds),
                })
            }
            COMMAND_CANCEL => {
                let action = int_arg(command, "action")
                    .and_then(|number| u32::try_from(number).ok())
                    .ok_or("`cancel` needs the number of the `action` to end.")?;
                let abandon = command
                    .args
                    .get("abandon")
                    .is_some_and(|value| *value == Value::Bool(true));
                Ok(Command::Cancel {
                    entity: player,
                    action: ActionId::new(action),
                    abandon,
                })
            }
            COMMAND_CLEAR_PLAN => Ok(Command::ClearPlan { entity: player }),
            COMMAND_CHOOSE => {
                let entity = match command.args.get("entity") {
                    Some(value) => match session.sim.subject_of(value) {
                        Some(Subject::Entity(entity)) => entity,
                        _ => return Err("`choose` names nobody who could choose.".to_owned()),
                    },
                    None => player,
                };
                let choice =
                    int_arg(command, "choice").ok_or("`choose` needs the `choice` it answers.")?;
                let option = int_arg(command, "option")
                    .and_then(|option| u32::try_from(option).ok())
                    .ok_or("`choose` needs the number of the `option` taken.")?;
                Ok(Command::Choose {
                    entity,
                    choice: ChoiceId::new(choice.unsigned_abs()),
                    option,
                })
            }
            other => Err(format!(
                "The simulation has no command `{other}` that this game can give yet."
            )),
        },
        CommandKind::Rule => Err(format!(
            "The rule `{}` cannot be run from a screen: the simulation has no `Ui` command yet.",
            command.name
        )),
    }
}

/// Does to the simulation what a command of the interface asks, or says
/// why not: the simulation's own reason, worded for the player. See the
/// [module](self) documentation.
pub fn command_for(session: &mut Session, command: &UiCommand) -> Result<(), String> {
    let built = build(session, command)?;
    session.bumped = None;
    if let Err(refusal) = session.sim.submit(built) {
        // Walking into something that could stand aside and does not:
        // the frame loop offers what is there (12 §3.2).
        if let Some(dir) = command.args.get(ARG_DIR).and_then(Value::as_str)
            && (refusal.reason.code == clash_sim::movement::codes::CLOSED
                || closed_way(&session.sim, session.player, dir))
        {
            session.bumped = Some(dir.to_owned());
        }
        return Err(refusal.reason.message);
    }
    session.stale_view = true;
    Ok(())
}

/// Whether a step a way is one the mover believes is closed: what
/// stands there could stand aside and does not (`move.closed`). A step
/// action's `requires` refuses with its `fail_text`, which does not say
/// so by its code.
fn closed_way(sim: &clash_sim::Sim, who: EntityId, dir: &str) -> bool {
    use clash_sim::movement;
    let Some(kind) = movement::parse_dir(dir) else {
        return false;
    };
    let Some(step) = movement::believed_step(sim, who, kind) else {
        return false;
    };
    matches!(
        movement::believed_rules(sim, who).judge(&step),
        Err(movement::StepFault::Closed)
    )
}

/// The argument of a step that says which way: what a bump is into.
pub const ARG_DIR: &str = "dir";

/// The scene's handle on the simulation. See the [module](self)
/// documentation.
#[derive(Clone)]
pub struct GameHost {
    shared: Rc<RefCell<Shared>>,
    /// The loaded definitions: names of definitions are answered from
    /// them without borrowing the shared state.
    defs: Arc<Defs>,
    /// How many run-time warnings evaluations have raised.
    warnings: u64,
}

impl GameHost {
    /// A host over the application's shared state.
    pub fn new(shared: Rc<RefCell<Shared>>, defs: Arc<Defs>) -> GameHost {
        GameHost {
            shared,
            defs,
            warnings: 0,
        }
    }

    /// Reads the game being played, if one is.
    fn read<R>(&self, read: impl FnOnce(&Session) -> R) -> Option<R> {
        self.shared.borrow().session.as_ref().map(read)
    }
}

impl SceneHost for GameHost {
    type Command = UiCommand;
    type Refusal = String;

    fn submit(&mut self, command: UiCommand) -> Result<(), String> {
        let mut shared = self.shared.borrow_mut();
        match shared.session.as_mut() {
            Some(session) => command_for(session, &command),
            None => Err(NO_GAME.to_owned()),
        }
    }

    fn view(&self) -> Value {
        self.read(player_value).unwrap_or_default()
    }

    fn party(&self) -> Value {
        Value::list(self.read(player_value))
    }

    fn reader(&mut self) -> &mut dyn Host {
        self
    }

    fn pending_decision(&self) -> Option<PendingDecision> {
        self.read(|session| {
            let request = session.sim.pending_decision()?;
            let entity = session.sim.subject_value(Subject::Entity(request.entity));
            let kind = match request.kind {
                DecisionKind::Free { .. } => DecisionAsk::Free,
                DecisionKind::Choice {
                    choice,
                    prompt,
                    options,
                } => DecisionAsk::Choice {
                    choice: Value::Int(i64::try_from(choice.raw()).unwrap_or(i64::MAX)),
                    prompt,
                    options: options.into_iter().map(|option| option.label).collect(),
                },
                // Reactions come with the package that opens windows to
                // them (T5).
                _ => return None,
            };
            Some(PendingDecision { entity, kind })
        })
        .flatten()
    }

    fn cell_value(&mut self, at: Location) -> Value {
        self.read(|session| session.sim.subject_value(Subject::Loc(at)))
            .unwrap_or_default()
    }

    fn cell_of(&mut self, value: &Value) -> Option<Location> {
        self.read(|session| match session.sim.subject_of(value) {
            Some(Subject::Loc(loc)) => Some(loc),
            _ => None,
        })
        .flatten()
    }

    fn sprite_value(&mut self, sprite: SpriteId) -> Value {
        let entity = Subject::Entity(EntityId::from_bits(sprite.raw()));
        self.read(|session| {
            if session.sim.exists(entity) {
                session.sim.subject_value(entity)
            } else {
                Value::Null
            }
        })
        .unwrap_or_default()
    }

    fn sprite_of(&mut self, value: &Value) -> Option<SpriteId> {
        self.read(|session| match session.sim.subject_of(value) {
            Some(Subject::Entity(id)) => Some(SpriteId::new(id.to_bits())),
            _ => None,
        })
        .flatten()
    }

    fn candidates(&mut self, _ask: &ParamAsk<'_>) -> Option<Vec<Value>> {
        // Nothing to say yet: an entity picker offers what the view
        // model holds, which is what the character perceives.
        None
    }

    fn perceived(&mut self, event: &SimEvent, _picture: &ViewModel) -> Perceived {
        self.read(|session| perception::perceived(&session.sim, event))
            .unwrap_or_default()
    }

    fn seen_cell(&mut self, at: Location) -> Option<CellContents> {
        self.read(|session| perception::seen_cell(&session.percepts, at))
            .flatten()
    }

    fn seen_sprite(&mut self, entity: EntityId) -> Option<(SpriteId, Option<EntitySprite>)> {
        self.read(|session| perception::seen_sprite(&session.percepts, entity))
            .flatten()
    }

    fn as_of(&mut self, _moment: Option<Tick>) {
        // The simulation keeps no history to answer from: bindings show
        // the present (decision D-605).
    }
}

impl Host for GameHost {
    fn subject(&mut self, _slot: u16) -> Value {
        Value::Null
    }

    fn local(&mut self, _slot: u16) -> Value {
        Value::Null
    }

    fn event_field(&mut self, _slot: u16) -> Value {
        Value::Null
    }

    fn property(&mut self, property: PropertyId, subject: Handle, args: &[Value]) -> Value {
        self.read(|session| session.sim.property(property, subject, args))
            .unwrap_or_default()
    }

    fn variable(&mut self, subject: Handle, name: &str) -> Value {
        self.read(|session| match session.sim.exprs().subject(subject) {
            Some(subject) => session.sim.var(subject, name),
            None => Value::Null,
        })
        .unwrap_or_default()
    }

    fn call(&mut self, function: FunctionId, args: &[Value]) -> Value {
        let is_name = self
            .defs
            .exprs()
            .registry()
            .function(function)
            .is_some_and(|found| found.spec.name == FN_NAME);
        self.read(|session| {
            // The player's own character goes by what the player called
            // it, while it has no name of its own.
            if is_name && !session.name.is_empty() && args.first() == Some(&player_value(session)) {
                let own = session.sim.call_function(function, args);
                let unnamed = session
                    .sim
                    .components()
                    .get::<clash_sim::component::Identity>(session.player)
                    .and_then(clash_sim::component::Identity::name)
                    .is_none();
                return if unnamed {
                    Value::text(&session.name)
                } else {
                    own
                };
            }
            session.sim.call_function(function, args)
        })
        .unwrap_or_default()
    }

    fn rng(&mut self) -> Option<&mut Rng> {
        // Bindings are pure: nothing on a screen draws at random.
        None
    }

    fn warn(&mut self, _warning: RuntimeWarning) {
        self.warnings += 1;
    }

    fn def_name(&self, id: DefRef) -> Option<&str> {
        self.defs.exprs().registry().def_name(id)
    }
}
