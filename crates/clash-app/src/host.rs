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
//! | `submit` | [`command_for`]: the one place a `UiCommand` becomes something done to the simulation |
//! | `pending_decision` | The front of the simulation's decision queue; else what the shim says an idle character is asked |
//! | `cell_value`, `cell_of` | The simulation's `loc` subject for a cell, and back |
//! | `sprite_value`, `sprite_of` | The entity whose id is the sprite's number ([`crate::perception_shim`] numbers sprites so), and back |
//! | `candidates` | Nothing: an entity picker then offers what the view model holds |
//!
//! # The name of the player's character
//!
//! A character's name is its `Identity`, which templates give
//! ([`crate::session::make_player`]). Until they do, the built-in
//! `name` of the player's own character is answered here with what the
//! player typed on the new-game screen.

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;

use clash_core::{EntityId, Rng};
use clash_data::Defs;
use clash_expr::{DefRef, FunctionId, Handle, Host, PropertyId, RuntimeWarning, Value};
use clash_scene::view::SpriteId;
use clash_scene::{DecisionAsk, ParamAsk, PendingDecision, SceneHost, UiCommand};
use clash_sim::{DecisionKind, Subject};
use clash_world::Location;

use crate::app::Shared;
use crate::session::{Session, player_value};
use crate::shim;

/// The built-in that gives what the reader calls something (05 §4.6).
const FN_NAME: &str = "name";

/// Why a command is refused when no game is being played.
pub const NO_GAME: &str = "No game is being played.";

/// Does to the simulation what a command of the interface asks, or says
/// why not.
///
/// **A seam.** The simulation's `Command` holds debug commands only, so
/// everything goes to [`crate::shim`]. When `Command::Act`, `Wait`,
/// `React`, `Pass`, `Choose` and `Ui` exist, this function builds the
/// one the `UiCommand` names and submits it, and the shim is deleted.
pub fn command_for(session: &mut Session, command: &UiCommand) -> Result<(), String> {
    shim::submit(session, command)
}

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
        self.read(|session| match session.sim.pending_decision() {
            Some(request) => match request.kind {
                DecisionKind::Free { .. } => Some(PendingDecision {
                    entity: session.sim.subject_value(Subject::Entity(request.entity)),
                    kind: DecisionAsk::Free,
                }),
                // Reactions and choices come with the packages that
                // raise them (T5, K9).
                _ => None,
            },
            None => shim::idle_decision(session),
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
            // it, until it has an identity of its own.
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
