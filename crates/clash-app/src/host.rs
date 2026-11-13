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
//! | `pending_decision` | The front of the simulation's decision queue (03 §5.2): a free decision, a choice a rule's `ask` put, or a reaction to a window `ask_reaction` put, with its options and the time left in the character's own time |
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
//! | `command react { entity, window, action, params }` | `React`: the reaction chosen at the reaction prompt (03 §7.3) |
//! | `command pass { entity, window }` | `Pass`: the prompt declined, the blow let happen |
//! | `command use_ability { ability, action?, params? }` | `UseAbility`: the ability by its id or qualified id, the grant's action by its id or name, the parameters the player gave (08 §5) |
//! | `command set_mode { ability, mode, on }` | `SetMode`: a mode of an ability switched on or off (08 §5.4) |
//! | `command cancel {}` | With no `action`: `Cancel` of the character's newest action, which lets a held phase go |
//! | a `run` of a rule | Refused, with the reason: the `Ui` command is not built |
//!
//! A refusal is the simulation's reason, which the frame loop writes in
//! the log. A step refused because the way is closed is noted
//! ([`crate::session::Session::bumped`]), and the frame loop offers what
//! is there instead ([`crate::interact`]). A step toward a creature the
//! party sees standing that way is not submitted while
//! [`crate::session::Session::bump_creatures`] holds: it is noted
//! ([`crate::session::Session::bumped_into`]) and the frame loop offers
//! to attack it ([`crate::interact`]).
//!
//! # What a picker is offered
//!
//! `candidates` answers for a parameter of the kind `part` (07 §4.2):
//! the parts on the outside of the creature settled before it (the
//! `target`), while the character perceives it: the parts that are there
//! and sit inside no other, in the body's order. Anything else is left
//! to the scene: an entity picker offers what the view model holds.
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
/// The command that answers a window with a reaction.
pub const COMMAND_REACT: &str = clash_scene::control::COMMAND_REACT;
/// The command that declines to react to a window.
pub const COMMAND_PASS: &str = clash_scene::control::COMMAND_PASS;
/// The command that uses an ability.
pub const COMMAND_USE_ABILITY: &str = clash_scene::control::COMMAND_USE_ABILITY;
/// The command that switches a mode of an ability.
pub const COMMAND_SET_MODE: &str = "set_mode";

/// How the first entry of a part picker, no part at all, is shown: the
/// blow lands where the body offers (07 §4.2).
pub const PART_ANYWHERE: &str = "wherever it lands";

/// The kind of subject a part of a body is.
const KIND_PART: &str = "part";
/// The kind of subject an entity is.
const KIND_ENTITY: &str = "entity";

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

/// A length of time in seconds, for showing: the bar of a reaction
/// prompt. Never read by the simulation.
fn seconds(dur: Dur) -> f64 {
    const MICROS_PER_SECOND: f64 = 1_000_000.0;
    dur.as_micros() as f64 / MICROS_PER_SECOND
}

/// An argument of a command as a whole number, if it is one.
fn int_arg(command: &UiCommand, name: &str) -> Option<i64> {
    command.args.get(name).and_then(Value::as_int)
}

/// The ability a value names: its id, or its qualified id as text.
fn ability_named(sim: &clash_sim::Sim, value: &Value) -> Option<clash_sim::ability::AbilityId> {
    use clash_data::Def;
    match value {
        Value::Id(id)
            if Some(id.ty)
                == sim
                    .exprs()
                    .registry()
                    .def_type_id(clash_sim::ability::AbilityDef::TYPE) =>
        {
            clash_sim::ability::AbilityId::from_index(id.index as usize)
        }
        Value::Str(text) => clash_sim::ability::lookup(sim.defs(), text),
        _ => None,
    }
}

/// An action a value names: its id, or its qualified id or bare name as
/// text.
fn action_value_named(sim: &clash_sim::Sim, value: &Value) -> Option<ActionDefId> {
    use clash_data::Def;
    match value {
        Value::Id(id) if Some(id.ty) == sim.exprs().registry().def_type_id(ActionDef::TYPE) => {
            ActionDefId::from_index(id.index as usize)
        }
        Value::Str(text) => action_named(sim.defs(), text),
        _ => None,
    }
}

/// Parameters by name, from a map value.
fn params_of(value: Option<&Value>) -> clash_sim::action::Params {
    value
        .and_then(Value::as_map)
        .map(|map| {
            map.iter()
                .map(|(name, value)| (name.to_string(), value.to_stored()))
                .collect()
        })
        .unwrap_or_default()
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
                // With no number: the character's newest action, which
                // lets a held phase go.
                let newest = || {
                    session
                        .sim
                        .actions_of(player)
                        .last()
                        .map(|action| action.raw())
                };
                let action = match command.args.get("action") {
                    Some(_) => int_arg(command, "action").and_then(|n| u32::try_from(n).ok()),
                    None => newest(),
                }
                .ok_or("There is nothing being done to stop.")?;
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
            COMMAND_USE_ABILITY => {
                let sim = &session.sim;
                let ability = command
                    .args
                    .get("ability")
                    .and_then(|value| ability_named(sim, value))
                    .ok_or("`use_ability` needs the `ability` to use: no loaded mod defines it.")?;
                let action = match command.args.get("action") {
                    Some(value) if !value.is_null() => Some(
                        action_value_named(sim, value)
                            .ok_or("`use_ability` names an `action` no loaded mod defines.")?,
                    ),
                    _ => None,
                };
                Ok(Command::UseAbility {
                    entity: player,
                    ability,
                    action,
                    params: params_of(command.args.get("params")),
                })
            }
            COMMAND_SET_MODE => {
                let sim = &session.sim;
                let ability = command
                    .args
                    .get("ability")
                    .and_then(|value| ability_named(sim, value))
                    .ok_or("`set_mode` needs the `ability` whose mode it is.")?;
                let mode = command
                    .args
                    .get("mode")
                    .and_then(Value::as_str)
                    .ok_or("`set_mode` needs the `mode` to switch.")?
                    .to_owned();
                let on = command
                    .args
                    .get("on")
                    .and_then(Value::as_bool)
                    .ok_or("`set_mode` needs `on`, true or false.")?;
                Ok(Command::SetMode {
                    entity: player,
                    ability,
                    mode,
                    on,
                })
            }
            COMMAND_REACT | COMMAND_PASS => {
                let entity = match command.args.get("entity") {
                    Some(value) => match session.sim.subject_of(value) {
                        Some(Subject::Entity(entity)) => entity,
                        _ => {
                            return Err(format!(
                                "`{}` names nobody who could react.",
                                command.name
                            ));
                        }
                    },
                    None => player,
                };
                let window = command
                    .args
                    .get("window")
                    .and_then(|value| session.sim.window_of(value))
                    .ok_or_else(|| format!("`{}` needs the `window` it answers.", command.name))?;
                if command.name == COMMAND_PASS {
                    return Ok(Command::Pass { entity, window });
                }
                let name = command
                    .args
                    .get("action")
                    .and_then(Value::as_str)
                    .ok_or("`react` needs the `action` to answer with.")?;
                let action = action_named(session.sim.defs(), name).ok_or_else(|| {
                    format!("Nothing can do `{name}`: no loaded mod defines that action.")
                })?;
                let params = command
                    .args
                    .get("params")
                    .and_then(Value::as_map)
                    .map(|map| {
                        map.iter()
                            .map(|(name, value)| (name.to_string(), value.to_stored()))
                            .collect()
                    })
                    .unwrap_or_default();
                Ok(Command::React {
                    entity,
                    window,
                    action,
                    params,
                })
            }
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
    session.bumped_into = None;
    // A step toward someone the character sees there: the frame loop
    // offers to attack them (12 §3.7).
    if command.kind == CommandKind::Act
        && session.bump_creatures
        && let Some(dir) = command.args.get(ARG_DIR).and_then(Value::as_str)
        && let Some(who) = creature_that_way(session, dir)
    {
        session.bumped_into = Some((dir.to_owned(), who));
        return Err("Someone is in the way.".to_owned());
    }
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

/// The creature the party sees standing one step away in a direction,
/// as the view of the last batch holds it: never what it does not
/// perceive.
fn creature_that_way(session: &Session, dir: &str) -> Option<EntityId> {
    let kind = clash_sim::movement::parse_dir(dir)?;
    let from = session.player_loc()?;
    let delta = kind.delta();
    let cell = clash_world::CellPos::new(
        from.cell.x + delta.x,
        from.cell.y + delta.y,
        from.cell.z + delta.z,
    );
    let there = session
        .sim
        .truth()
        .cosmos()
        .normalize(Location::new(from.grid, cell))
        .ok()?;
    session
        .percepts
        .sight
        .sprites
        .iter()
        .find(|(id, sprite)| {
            sprite.loc == there
                && sprite.kind == clash_scene::view::SpriteKind::Creature
                && !sprite.suspected
                && id.raw() != session.player.to_bits()
        })
        .map(|(id, _)| EntityId::from_bits(id.raw()))
}

/// The parts on the outside of a creature the character perceives: the
/// parts that are there and sit inside no other, in the body's order.
fn outer_parts(sim: &clash_sim::Sim, viewer: EntityId, target: &Value) -> Option<Vec<Value>> {
    let Some(Subject::Entity(entity)) = sim.subject_of(target) else {
        return None;
    };
    if !clash_sim::perception::perceives(sim, viewer, entity) {
        return Some(Vec::new());
    }
    let registry = sim.exprs().registry();
    let Value::Subject(handle) = sim.subject_value(Subject::Entity(entity)) else {
        return None;
    };
    let parts = registry
        .kind_id(KIND_ENTITY)
        .and_then(|kind| registry.property_id(kind, "parts"))
        .map(|parts| sim.property(parts, handle, &[]))?;
    let parts = parts.as_list()?.to_vec();
    let inside = registry
        .kind_id(KIND_PART)
        .and_then(|kind| registry.property_id(kind, "inside"));
    let mut hidden: Vec<Value> = Vec::new();
    if let Some(inside) = inside {
        for part in &parts {
            if let Value::Subject(handle) = part
                && let Some(within) = sim.property(inside, *handle, &[]).as_list()
            {
                hidden.extend(within.iter().cloned());
            }
        }
    }
    // Each with its name in the body, as a row a picker shows.
    let name = registry
        .kind_id(KIND_PART)
        .and_then(|kind| registry.property_id(kind, "name"));
    Some(
        parts
            .into_iter()
            .filter(|part| !hidden.contains(part))
            .map(|part| {
                let label = match (&part, name) {
                    (Value::Subject(handle), Some(name)) => sim
                        .property(name, *handle, &[])
                        .as_str()
                        .map(|name| name.replace('_', " ")),
                    _ => None,
                };
                match label {
                    Some(label) => Value::map([("value", part), ("label", Value::text(label))]),
                    None => part,
                }
            })
            .collect(),
    )
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
                DecisionKind::Reaction {
                    window,
                    deadline,
                    options,
                } => {
                    let sim = &session.sim;
                    let actions = sim.defs().try_registry::<ActionDef>();
                    let time_left = sim
                        .window_time_left(window, request.entity)
                        .map_or(0.0, seconds);
                    // The whole window in the character's own time: its
                    // universal length at the rate the time left shows.
                    let time_total = match (deadline, sim.windows().get(window)) {
                        (Some(deadline), Some(open)) => {
                            let total = seconds(deadline.since(open.opened));
                            let left = seconds(deadline.since(sim.now()));
                            if left > 0.0 {
                                total * time_left / left
                            } else {
                                total
                            }
                        }
                        _ => time_left,
                    };
                    // What is happening, as the character perceives it:
                    // who does what. It noticed the window, so it
                    // perceives its source (15 §6).
                    let what = sim.windows().get(window).map_or_else(String::new, |open| {
                        let who = open.source.map(|source| {
                            let value = sim.subject_value(Subject::Entity(source));
                            sim.exprs()
                                .registry()
                                .function_id("the")
                                .map(|the| {
                                    sim.call_function_for(the, &[value], Some(request.entity))
                                })
                                .and_then(|name| name.as_str().map(clash_sim::reader::capitalised))
                                .unwrap_or_else(|| "Someone".to_owned())
                        });
                        let doing =
                            open.action
                                .and_then(|action| sim.actions.get(action))
                                .map(|running| {
                                    clash_sim::offer::action_label(sim, request.entity, running.def)
                                });
                        match (who, doing) {
                            (Some(who), Some(doing)) => format!("{who}: {doing}"),
                            (Some(who), None) => who,
                            (None, Some(doing)) => doing,
                            (None, None) => String::new(),
                        }
                    });
                    DecisionAsk::Reaction {
                        what,
                        window: sim.window_value(window),
                        options: options
                            .into_iter()
                            .map(|option| clash_scene::ReactionOption {
                                action: actions
                                    .map(|registry| registry.name(option.action).to_owned())
                                    .unwrap_or_default(),
                                label: option.label,
                                in_time: option.in_time,
                            })
                            .collect(),
                        time_left,
                        time_total,
                    }
                }
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

    fn candidates(&mut self, ask: &ParamAsk<'_>) -> Option<Vec<Value>> {
        // A part: those on the outside of the creature chosen before it.
        // Anything else: an entity picker offers what the view model
        // holds, which is what the character perceives.
        let registry = self.defs.exprs().registry();
        let is_kind = |name: &str| match ask.ty.non_null() {
            clash_expr::Type::Subject(kind) => registry.kind_id(name) == Some(*kind),
            _ => false,
        };
        if !is_kind(KIND_PART) {
            return None;
        }
        let entity_kind = registry.kind_id(KIND_ENTITY);
        let target = ask
            .settled
            .iter()
            .rev()
            .map(|(_, value)| value)
            .find(|value| {
                value
                    .as_subject()
                    .is_some_and(|handle| Some(handle.kind) == entity_kind)
            })?
            .clone();
        let parts = self
            .read(|session| outer_parts(&session.sim, session.player, &target))
            .flatten()?;
        // First, no part at all: the blow lands where the body offers.
        let anywhere = Value::map([
            ("value", Value::Null),
            ("label", Value::text(PART_ANYWHERE)),
        ]);
        Some(std::iter::once(anywhere).chain(parts).collect())
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
