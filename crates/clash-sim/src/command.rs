//! Commands: the only way anything outside the simulation changes it
//! (03 §4.4, invariant T5), the input log, and replay (invariant T6).
//!
//! # Adding a command
//!
//! Add a variant to [`Command`] (or to [`DebugCommand`] for a tool's),
//! and an arm in `Sim::apply` that validates it against current state
//! and either applies it or returns a [`Reason`]. A refused command must
//! leave the state exactly as it was: it is not logged, so a replay does
//! not repeat it. That is all: accepted commands are logged and replayed
//! without the new variant doing anything more.

use clash_core::{Dur, EntityId, Tick};
use clash_time::Wake;
use clash_world::{Cause, ElementState, Layer, Location};
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::Sim;
use crate::advance::{Advance, AdvanceLimit};
use crate::entity::{Control, EntitySpec, Place};
use crate::reason::{Reason, codes};

/// Something the outside asks of the simulation (03 §4.4).
///
/// The commands a player's choices become (`Act`, `Wait`, `Cancel`,
/// `ClearPlan`, `React`, `Pass`, `Choose`, `Setting`, `Ui`) arrive with
/// the packages that can honour them: match with a wildcard arm.
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum Command {
    /// A tool's command.
    Debug(DebugCommand),
    /// Answers a choice put by a rule's `ask` (05 §10.6) with the
    /// position of an option, from zero. Refused if the entity is not
    /// waiting for that choice, or the option is not one it may take.
    Choose {
        /// Who chooses.
        entity: EntityId,
        /// The choice, as the decision named it.
        choice: crate::decision::ChoiceId,
        /// The position of the option taken.
        option: u32,
    },
    /// Starts an action, or puts it at the end of the entity's plan
    /// (03 §4.4, §6.6). Answers a free decision. Refused, with the
    /// reason, if the action cannot start now; a handler of
    /// `action_starting` that refuses it raises `action_refused`
    /// instead, the command having been taken.
    Act {
        /// Who acts: an entity under direct or delegated control.
        entity: EntityId,
        /// What it does.
        action: crate::action::ActionDefId,
        /// The parameters, by name; those left out take their defaults.
        params: crate::action::Params,
        /// Now, or after what is planned.
        how: crate::action::How,
    },
    /// Occupies an entity doing nothing for a length of its own time,
    /// then asks again (03 §5.2). Answers a free decision; a wait of no
    /// length answers and asks nothing more.
    Wait {
        /// Who waits.
        entity: EntityId,
        /// How long.
        dur: Dur,
    },
    /// Ends an action at the actor's wish (03 §6.4): a held phase is let
    /// go, and anything else is abandoned where the phase allows it.
    /// With `abandon`, a held phase is abandoned too.
    Cancel {
        /// Whose action.
        entity: EntityId,
        /// Which.
        action: crate::later::ActionId,
        /// Whether to give it up rather than let a held phase go.
        abandon: bool,
    },
    /// Empties an entity's plan (03 §5.3).
    ClearPlan {
        /// Whose.
        entity: EntityId,
    },
    /// Answers a window with a reaction (03 §7.3): starts `action` with
    /// `$window` bound, cancelling the entity's actions in phases that
    /// allow it to make room, and answers the `Reaction` decision it was
    /// asked for the window, if it was. Refused, with the reason, if the
    /// reaction cannot start: the window has closed, the entity did not
    /// notice it, the action does not answer it, or anything an action
    /// is refused for.
    React {
        /// Who reacts: an entity under direct or delegated control.
        entity: EntityId,
        /// The window, as the decision named it.
        window: crate::action::WindowId,
        /// The reaction.
        action: crate::action::ActionDefId,
        /// Its parameters, by name; those left out take their defaults.
        params: crate::action::Params,
    },
    /// Declines the `Reaction` decision an entity was asked for a window
    /// (03 §5.2). Refused if it was asked none.
    Pass {
        /// Who declines.
        entity: EntityId,
        /// The window, as the decision named it.
        window: crate::action::WindowId,
    },
    /// Uses an ability the entity holds (08 §5): starts the action one
    /// of its grants makes available, paying its cost and starting its
    /// cooldown. Refused as `Act` is, and with the ability's reason.
    UseAbility {
        /// Who.
        entity: EntityId,
        /// The ability.
        ability: crate::ability::AbilityId,
        /// The action of the grant; `None` for the ability's only one.
        action: Option<crate::action::ActionDefId>,
        /// Parameters of the action, by name. The grant's own stand in
        /// for any of the same name.
        params: crate::action::Params,
    },
    /// Switches a mode of an ability the entity holds on or off
    /// (08 §5.4).
    SetMode {
        /// Who.
        entity: EntityId,
        /// The ability.
        ability: crate::ability::AbilityId,
        /// The mode's id within the ability.
        mode: String,
        /// On, or off.
        on: bool,
    },
    /// Changes a setting (04 §10). Refused for a setting the scenario
    /// being played locks (`setting.locked`), and for a value its
    /// `setting` definition does not take (`setting.bad_value`). Raises
    /// `setting_changed`.
    Setting {
        /// The setting's key: a qualified id, or an engine setting's
        /// name.
        key: String,
        /// Its new value.
        value: clash_core::Value,
    },
    /// Begins a game from a scenario (13 §4, §12), in a simulation whose
    /// world was made for it: places the party, applies the scenario's
    /// settings and the answers to its options, puts its rules in force,
    /// runs `start.do` and raises `scenario_started`
    /// ([`scenario::begin`](crate::scenario::begin)).
    BeginScenario(crate::scenario::BeginScenario),
}

/// What a tool or a test may do to the simulation directly. These are
/// commands like any other: validated, logged and replayed (14 §2).
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum DebugCommand {
    /// Makes an entity and puts it somewhere. Its id is the one
    /// [`EntityTable::next_id`](crate::EntityTable::next_id) gave before
    /// the command, and `entity_created` reports it.
    CreateEntity {
        /// What to make it from.
        spec: EntitySpec,
        /// Where to put it.
        place: Place,
    },
    /// Moves an entity: to a cell, into another entity, out of the world.
    MoveEntity {
        /// Who.
        entity: EntityId,
        /// Where to.
        to: Place,
    },
    /// Removes an entity for good.
    RemoveEntity {
        /// Who.
        entity: EntityId,
    },
    /// Sets the floor, block or fixture of a cell.
    SetElement {
        /// The cell.
        loc: Location,
        /// Which of its elements.
        layer: Layer,
        /// What to put there.
        state: ElementState,
    },
    /// Changes who decides for an entity.
    SetControl {
        /// Whose.
        entity: EntityId,
        /// The new mode.
        control: Control,
    },
    /// Starts a session for an entity at a place that may not be loaded
    /// yet: loads the tracts round it, puts the entity there and holds
    /// the ticket of a controlled character for it
    /// ([`Sim::start_session`]).
    StartSession {
        /// Who.
        entity: EntityId,
        /// Where.
        at: Location,
    },
    /// Schedules a wake-up: for an entity, on the entity's clock, or
    /// with none, on universal time. For poking a domain's handler.
    Schedule {
        /// Whose wake-up it is, if anyone's.
        entity: Option<EntityId>,
        /// How much local time from now.
        after: Dur,
        /// What the handler is given.
        wake: Wake,
    },
    /// Makes `count` items of a definition, as one stack, and puts them
    /// somewhere ([`item::create`](crate::item::create)). Its id is the
    /// one [`EntityTable::next_id`](crate::EntityTable::next_id) gave
    /// before the command, and `entity_created` reports it.
    CreateItem {
        /// The `item` definition, by qualified id: `core:knife`.
        def: String,
        /// How many. More than one needs a definition that stacks.
        count: u32,
        /// Where to put it.
        place: Place,
    },
    /// Makes an entity from templates and puts it somewhere (06 §10.3).
    /// Its id is the one
    /// [`EntityTable::next_id`](crate::EntityTable::next_id) gave before
    /// the command, and `entity_spawned` reports it.
    Spawn {
        /// The templates and what was selected within them.
        spec: crate::template::CharacterSpec,
        /// Where to put it.
        place: Place,
    },
}

/// A command was refused. The state is as it was.
#[derive(Clone, PartialEq, Eq, Debug, Error)]
#[error("{reason}")]
pub struct CommandError {
    /// Why: which check failed, in a form that can be shown to the
    /// player.
    pub reason: Reason,
}

impl From<Reason> for CommandError {
    fn from(reason: Reason) -> CommandError {
        CommandError { reason }
    }
}

/// One accepted command, and exactly when it was accepted.
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub struct LogEntry {
    /// The universal time at which it was accepted.
    pub tick: Tick,
    /// How many wake-ups the simulation had handled by then. With the
    /// tick, this fixes the command's place among the wake-ups of its
    /// tick, so that a session that yielded in the middle of a tick
    /// replays exactly.
    pub step: u64,
    /// The command.
    pub command: Command,
}

/// Every command accepted since the log was last taken, in order
/// (03 §4.4). A save and the log that followed it replay to the same
/// state (invariant T6).
#[derive(Clone, PartialEq, Eq, Default, Debug, Serialize, Deserialize)]
pub struct InputLog {
    entries: Vec<LogEntry>,
}

impl InputLog {
    /// The entries, oldest first.
    pub fn entries(&self) -> &[LogEntry] {
        &self.entries
    }

    /// How many commands were accepted.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Whether none was.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

impl Sim {
    /// Validates a command against the current state and either applies
    /// it or refuses it with a reason (03 §4.4). An accepted command is
    /// appended to the input log with its tick, and the events it caused
    /// are delivered before this returns. A refused command changes
    /// nothing and is not logged.
    pub fn submit(&mut self, command: Command) -> Result<(), CommandError> {
        // Deliver whatever is waiting first, so that the command sees
        // settled state and its own events follow in order.
        self.pump();
        let entry = LogEntry {
            tick: self.now(),
            step: self.steps,
            command: command.clone(),
        };
        self.apply(command)?;
        self.log.entries.push(entry);
        self.pump();
        // An accepted command leaves the simulation at a safe point: a
        // ticket that follows what the command moved is reconciled now.
        self.safe_point();
        Ok(())
    }

    /// Applies a command, or says why not. A refusal leaves the state as
    /// it was.
    fn apply(&mut self, command: Command) -> Result<(), Reason> {
        match command {
            Command::Debug(debug) => self.apply_debug(debug),
            Command::Choose {
                entity,
                choice,
                option,
            } => self.choose(entity, choice, option),
            Command::Act {
                entity,
                action,
                params,
                how,
            } => self.command_act(entity, action, params, how),
            Command::Wait { entity, dur } => self.command_wait(entity, dur),
            Command::Cancel {
                entity,
                action,
                abandon,
            } => self.command_cancel(entity, action, abandon),
            Command::ClearPlan { entity } => self.command_clear_plan(entity),
            Command::React {
                entity,
                window,
                action,
                params,
            } => self.command_react(entity, window, action, params),
            Command::Pass { entity, window } => self.command_pass(entity, window),
            Command::UseAbility {
                entity,
                ability,
                action,
                params,
            } => self.command_use_ability(entity, ability, action, &params),
            Command::SetMode {
                entity,
                ability,
                mode,
                on,
            } => self.command_set_mode(entity, ability, &mode, on),
            Command::Setting { key, value } => crate::scenario::setting_command(self, &key, value),
            Command::BeginScenario(begin) => crate::scenario::begin(self, begin),
        }
    }

    fn apply_debug(&mut self, command: DebugCommand) -> Result<(), Reason> {
        let cause = Cause::DEBUG;
        match command {
            DebugCommand::CreateEntity { spec, place } => {
                self.spawn(spec, place, cause).map(|_| ())
            }
            DebugCommand::Spawn { spec, place } => {
                self.spawn_template(&spec, place, cause).map(|_| ())
            }
            DebugCommand::MoveEntity { entity, to } => self.relocate(entity, to, cause).map(|_| ()),
            DebugCommand::RemoveEntity { entity } => self.remove_entity(entity, cause),
            DebugCommand::SetElement { loc, layer, state } => self
                .world(|ctx| ctx.edit(cause).set_element(loc, layer, state))
                .map(|_| ())
                .map_err(|error| Reason::from_world(&error)),
            DebugCommand::SetControl { entity, control } => {
                self.set_control(entity, control, cause).map(|_| ())
            }
            DebugCommand::StartSession { entity, at } => {
                // What can be refused without a trace is refused here.
                // Past this point tracts have been surveyed and loaded,
                // so a failure is reported, not refused.
                self.check_session(entity, at)?;
                if let Err(reason) = self.start_session(entity, at, cause) {
                    self.fault(reason);
                }
                Ok(())
            }
            DebugCommand::Schedule {
                entity,
                after,
                wake,
            } => match entity {
                Some(entity) => self.schedule_for(entity, after, wake).map(|_| ()),
                None => self
                    .schedule_on(clash_time::ClockId::UNIVERSAL, after, wake)
                    .map(|_| ()),
            },
            DebugCommand::CreateItem { def, count, place } => {
                let def = crate::item::def_named(self, &def)?;
                crate::item::create(self, def, count, place, cause).map(|_| ())
            }
        }
    }

    /// The commands accepted since the log was last taken.
    pub fn input_log(&self) -> &InputLog {
        &self.log
    }

    /// Takes the input log, leaving an empty one. Whoever saves takes
    /// the log at the same moment: a save and the log that follows it
    /// are what replay needs.
    pub fn take_input_log(&mut self) -> InputLog {
        core::mem::take(&mut self.log)
    }

    /// Replays an input log: for each entry, advances to exactly where
    /// the command was accepted and submits it. Run on a simulation set
    /// up as the logged one was (the same definitions, seed, world and
    /// handlers, or a load of the save the log follows), it ends in the
    /// state the logged session was in when its last command was
    /// accepted (invariant T6).
    ///
    /// Stops at the first entry that cannot be reached or is refused,
    /// with a reason that names it: the session has diverged.
    pub fn replay(&mut self, log: &InputLog) -> Result<(), Reason> {
        for (index, entry) in log.entries.iter().enumerate() {
            let diverged = |why: String| {
                Reason::new(
                    codes::REPLAY_DIVERGED,
                    format!("replay diverged at entry {index}: {why}"),
                )
                .with("entry", index as i64)
            };
            while self.steps < entry.step {
                let (before, budget) = (
                    self.steps,
                    u32::try_from(entry.step - self.steps).unwrap_or(u32::MAX),
                );
                let outcome = self.advance(AdvanceLimit::Work(budget));
                if self.steps > before || matches!(outcome, Advance::Request(_)) {
                    continue;
                }
                return Err(diverged(match outcome {
                    Advance::NeedInput(request) => format!(
                        "entity {} is waiting for a decision that the log does not answer",
                        request.entity
                    ),
                    _ => format!(
                        "nothing is left to happen at tick {}, after {} wake-ups, but the command was accepted after {}",
                        self.now(),
                        self.steps,
                        entry.step
                    ),
                }));
            }
            if self.steps != entry.step {
                return Err(diverged(format!(
                    "{} wake-ups have been handled, but the command was accepted after {}",
                    self.steps, entry.step
                )));
            }
            self.timeline.advance_to(entry.tick);
            if self.now() != entry.tick {
                return Err(diverged(format!(
                    "the simulation is at tick {}, but the command was accepted at tick {}",
                    self.now(),
                    entry.tick
                )));
            }
            self.submit(entry.command.clone())
                .map_err(|error| diverged(format!("the command was refused: {error}")))?;
        }
        Ok(())
    }
}
