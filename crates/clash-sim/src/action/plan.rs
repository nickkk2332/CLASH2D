//! Control, plans and free decisions (03 §5): who is asked what to do,
//! and when.
//!
//! # When an entity is free
//!
//! An entity is **free** when it is in the world, no foreground action
//! of its own is running, its plan is empty and it is not waiting
//! ([`Sim::is_free`]). It is asked what to do when it becomes free:
//!
//! | What happened | Why it is free |
//! |---------------|----------------|
//! | It came under direct control, or entered the world under it | `started` |
//! | A foreground action of its own finished, or was cancelled | `finished` |
//! | A foreground action of its own was interrupted; `interrupt_plan` | `interrupted` |
//! | A step of its plan could not start; the plan was cleared | `plan_empty`, with the reason |
//! | A wait it was told to make, or an idle, is over | `waited` |
//!
//! Before it is asked, its plan is consulted: the next step starts if
//! its channels are free, waits if they are busy, and if it cannot start
//! for another reason the plan is cleared (`plan_cleared`) and the
//! entity is asked with that reason.
//!
//! # Who is asked
//!
//! A **directly controlled** entity is asked through a `Free` decision
//! request ([`Sim::request_decision`]): [`advance`](Sim::advance) returns
//! it and time does not move until a command answers. `Act`, `Wait` and
//! `Choose` answer.
//!
//! **Anyone else** is not asked by the simulation: `decision_needed` is
//! raised for whatever decides for it, the AI layer (09) or a rule, which
//! answers by starting an action or a wait. If something listens for
//! `decision_needed`, an entity nothing gave anything to do idles for
//! `actions.idle_wait` of its own time and is asked again; if nothing
//! listens, nobody would answer, and it simply stays free. An entity
//! that enters the world without direct control is not asked: whatever
//! made it decides when it should start deciding.
//!
//! A **delegated** entity takes commands as a directly controlled one
//! does, and is never asked: `decision_needed` is raised for it.

use clash_core::{Dur, EntityId, Real};
use clash_time::{Wake, WakeDomain};
use clash_world::{Cause, CauseKind};

use super::event::ActEvent;
use super::ids::ActionDefId;
use super::reason as why;
use super::run::{
    ACTION_UNKNOWN, COMMAND_BAD_ARGUMENT, COMMAND_NOT_CONTROLLED, WAKE_WAIT, action_def,
};
use super::state::{How, Params, PlanStep, Waiting};
use crate::Sim;
use crate::decision::{DecisionKind, FreeReason};
use crate::entity::Control;
use crate::event::{EventBody, Where};
use crate::later::ActionId;
use crate::reason::Reason;

/// The setting that says how long an entity nothing gave anything to do
/// idles before it is asked again, in seconds of its own time (03 §5.2).
pub const IDLE_WAIT_SETTING: &str = "actions.idle_wait";

/// The default of [`IDLE_WAIT_SETTING`]: one second.
pub const DEFAULT_IDLE_WAIT: Dur = Dur::SECOND;

/// The name of the event raised for an entity nobody controls directly
/// that is free.
const DECISION_NEEDED: &str = "decision_needed";

impl Sim {
    /// Whether an entity is free to be asked what to do: in the world,
    /// with no foreground action of its own running, an empty plan, and
    /// not waiting (03 §5.2). An entity that is idling is free.
    pub fn is_free(&self, entity: EntityId) -> bool {
        let Some(record) = self.entities.get(entity) else {
            return false;
        };
        self.entities.location(entity).is_some()
            && record.plan.is_empty()
            && !self.is_acting(entity)
            && self.actions.wait_of(entity).is_none_or(|wait| wait.idle)
    }

    /// What follows a foreground action of an entity ending: the next
    /// step of its plan, or else it is free.
    pub(crate) fn after_action(&mut self, actor: EntityId, why: FreeReason, cause: Cause) {
        if !self.follow_plan(actor) {
            self.free(actor, why, cause);
        }
    }

    /// Starts the next step of an entity's plan if it can (03 §5.3).
    /// Returns whether the plan settled what comes next: a step started,
    /// waits for its channels or for the action running, or could not
    /// start and the entity was asked with the reason. `false` if the
    /// plan is empty.
    pub(crate) fn follow_plan(&mut self, actor: EntityId) -> bool {
        let Some(step) = self
            .entities
            .get(actor)
            .and_then(|record| record.plan.front().cloned())
        else {
            return false;
        };
        if self.is_acting(actor) || self.entities.location(actor).is_none() {
            return true;
        }
        let cause = Cause {
            actor: Some(actor),
            instigator: Some(actor),
            kind: CauseKind::Action,
        };
        let streams = self.rng.clone();
        let refused = match self.prepare(actor, step.action, &step.params, false) {
            Ok(prepared) => {
                if let Some(record) = self.entities.get_mut(actor) {
                    record.plan.pop();
                }
                match self.begin(prepared, cause) {
                    Ok(_) => return true,
                    Err(reason) => reason,
                }
            }
            Err(reason) => {
                self.rng = streams;
                if reason.code == why::CHANNEL_BUSY.code {
                    // A step whose channels are busy waits for them.
                    return true;
                }
                if let Some(record) = self.entities.get_mut(actor) {
                    record.plan.pop();
                }
                let action = super::run::action_name(&self.defs, step.action);
                self.emit(
                    Where::Entity(actor),
                    cause,
                    EventBody::Act(ActEvent::ActionRefused {
                        actor,
                        action,
                        reason: reason.clone(),
                    }),
                );
                reason
            }
        };
        self.clear_plan(actor, Some(refused.clone()), cause);
        self.free(actor, FreeReason::PlanEmpty(Some(refused)), cause);
        true
    }

    /// Empties an entity's plan and raises `plan_cleared` if it held
    /// anything, or if there is a reason to give.
    pub fn clear_plan(&mut self, entity: EntityId, reason: Option<Reason>, cause: Cause) {
        let cleared = self
            .entities
            .get_mut(entity)
            .map_or(0, |record| record.plan.clear());
        if cleared > 0 || reason.is_some() {
            self.emit(
                Where::Entity(entity),
                cause,
                EventBody::Act(ActEvent::PlanCleared {
                    actor: entity,
                    reason,
                }),
            );
        }
    }

    /// Asks whoever decides for a free entity what it does next: a
    /// directly controlled one through a `Free` request, anyone else by
    /// `decision_needed` (see the [module documentation](self)). Nothing
    /// is asked of an entity that is not free, nor of one whose mind
    /// has stopped and that nobody drives ([`Sim::life_lets_act`]): it
    /// is asked when it wakes or is revived.
    pub(crate) fn free(&mut self, entity: EntityId, why: FreeReason, cause: Cause) {
        if !self.is_free(entity) {
            return;
        }
        // A stopped mind is asked nothing: it is asked when it wakes or
        // is revived (06 §8.4).
        if self.life_lets_act(entity).is_err() {
            return;
        }
        let Some(control) = self.entities.get(entity).map(|record| record.control()) else {
            return;
        };
        if control == Control::Direct {
            if !self.decisions.has_free(entity) {
                let _ = self.request_decision(entity, DecisionKind::Free { why }, cause);
            }
            return;
        }
        self.emit(
            Where::Entity(entity),
            cause,
            EventBody::Act(ActEvent::DecisionNeeded {
                entity,
                control,
                why,
            }),
        );
        if self.bus.has_listeners(DECISION_NEEDED) && self.actions.wait_of(entity).is_none() {
            let wait = match self.settings.get(IDLE_WAIT_SETTING) {
                Some(clash_core::Value::Real(secs)) => Dur::from_secs_real((*secs).max(Real::ZERO)),
                Some(clash_core::Value::Int(secs)) => Dur::from_secs((*secs).max(0)),
                _ => DEFAULT_IDLE_WAIT,
            };
            if wait > Dur::ZERO
                && let Ok(wake) = self.schedule_for(
                    entity,
                    wait,
                    Wake::new(WakeDomain::Action, entity.to_bits(), WAKE_WAIT),
                )
            {
                self.actions.set_wait(
                    entity,
                    Waiting {
                        wake: Some(wake),
                        idle: true,
                    },
                );
            }
        }
    }

    /// Ends the wait an entity is in, if it is waiting: what was due at
    /// its end is not. Returns whether it was waiting. `interrupt_plan`
    /// calls it, so that a rule can break off a long wait (decision
    /// D-871).
    pub fn end_wait(&mut self, entity: EntityId) -> bool {
        let Some(old) = self.actions.take_wait(entity) else {
            return false;
        };
        if let Some(wake) = old.wake {
            self.cancel_wake(wake);
        }
        true
    }

    /// Occupies an entity doing nothing for a length of its own time
    /// (`Wait`, the `wait` effect). When it is over the entity is free
    /// and is asked again. A wait of no length answers without asking
    /// again: the entity is free and nothing is due.
    pub fn wait(&mut self, entity: EntityId, dur: Dur, _cause: Cause) -> Result<(), Reason> {
        self.known(entity)?;
        if dur < Dur::ZERO {
            return Err(
                Reason::new(COMMAND_BAD_ARGUMENT, "a wait cannot last less than no time")
                    .with("entity", entity.to_string())
                    .with("argument", "dur"),
            );
        }
        if let Some(old) = self.actions.take_wait(entity)
            && let Some(wake) = old.wake
        {
            self.cancel_wake(wake);
        }
        if dur > Dur::ZERO {
            let wake = self.schedule_for(
                entity,
                dur,
                Wake::new(WakeDomain::Action, entity.to_bits(), WAKE_WAIT),
            )?;
            self.actions.set_wait(
                entity,
                Waiting {
                    wake: Some(wake),
                    idle: false,
                },
            );
        }
        Ok(())
    }

    /// Puts an action at the end of an entity's plan, and starts it if
    /// nothing is in the way (03 §5.3).
    pub fn enqueue_action(
        &mut self,
        entity: EntityId,
        action: ActionDefId,
        params: Params,
        _cause: Cause,
    ) -> Result<(), Reason> {
        self.known(entity)?;
        if action_def(&self.defs, action).is_none() {
            return Err(Reason::new(
                ACTION_UNKNOWN,
                format!("there is no action definition {action}"),
            )
            .with("action", i64::from(action.raw())));
        }
        if let Some(record) = self.entities.get_mut(entity) {
            record.plan.push(PlanStep { action, params });
        }
        self.follow_plan(entity);
        Ok(())
    }

    /// Follows a change of who decides for an entity (03 §5.1): one that
    /// is free is asked, as one that has just started.
    pub(crate) fn control_followed(&mut self, entity: EntityId, cause: Cause) {
        self.free(entity, FreeReason::Started, cause);
    }

    /// Follows an entity entering the world: its plan goes on, and one
    /// under direct control with nothing planned is asked.
    pub(crate) fn entered_world(&mut self, entity: EntityId, cause: Cause) {
        if self.follow_plan(entity) {
            return;
        }
        let direct = self
            .entities
            .get(entity)
            .is_some_and(|record| record.control() == Control::Direct);
        if direct {
            self.free(entity, FreeReason::Started, cause);
        }
    }

    /// Whether commands may address an entity: it is under `Direct` or
    /// `Delegated` control (03 §4.4).
    fn controlled(&self, entity: EntityId) -> Result<(), Reason> {
        let control = self.known(entity)?.control();
        if control == Control::None {
            return Err(Reason::new(
                COMMAND_NOT_CONTROLLED,
                format!("nobody controls entity {entity}, so it takes no commands"),
            )
            .with("entity", entity.to_string()));
        }
        Ok(())
    }

    /// Takes a free decision an entity is waiting for as answered.
    fn answer_free(&mut self, entity: EntityId) {
        self.answer_which(entity, |kind| matches!(kind, DecisionKind::Free { .. }));
    }

    /// The `Act` command (03 §4.4).
    pub(crate) fn command_act(
        &mut self,
        entity: EntityId,
        action: ActionDefId,
        params: Params,
        how: How,
    ) -> Result<(), Reason> {
        self.controlled(entity)?;
        // Refused with life's reason for one whose mind has stopped,
        // whether it would start now or be planned (06 §8.4).
        self.life_lets_act(entity)?;
        let cause = Cause {
            actor: Some(entity),
            instigator: Some(entity),
            kind: CauseKind::Action,
        };
        match how {
            How::Start => {
                let streams = self.rng.clone();
                let prepared = match self.prepare(entity, action, &params, true) {
                    Ok(prepared) => prepared,
                    Err(reason) => {
                        self.rng = streams;
                        return Err(reason);
                    }
                };
                self.answer_free(entity);
                // A handler of `action_starting` that refuses it raises
                // `action_refused`: the command was taken.
                let _ = self.begin(prepared, cause);
            }
            How::Enqueue => {
                self.answer_free(entity);
                self.enqueue_action(entity, action, params, cause)?;
            }
        }
        Ok(())
    }

    /// The `Wait` command.
    pub(crate) fn command_wait(&mut self, entity: EntityId, dur: Dur) -> Result<(), Reason> {
        self.controlled(entity)?;
        let cause = Cause {
            actor: Some(entity),
            instigator: Some(entity),
            kind: CauseKind::Action,
        };
        self.wait(entity, dur, cause)?;
        self.answer_free(entity);
        Ok(())
    }

    /// The `Cancel` command.
    pub(crate) fn command_cancel(
        &mut self,
        entity: EntityId,
        action: ActionId,
        abandon: bool,
    ) -> Result<(), Reason> {
        self.controlled(entity)?;
        let cause = Cause {
            actor: Some(entity),
            instigator: Some(entity),
            kind: CauseKind::Action,
        };
        self.cancel_action(entity, action, abandon, cause)
    }

    /// The `Interrupt` command (D-931): `interrupt_plan` for an entity
    /// under direct or delegated control.
    pub(crate) fn command_interrupt(&mut self, entity: EntityId) -> Result<(), Reason> {
        self.controlled(entity)?;
        let cause = Cause {
            actor: Some(entity),
            instigator: Some(entity),
            kind: CauseKind::Action,
        };
        self.interrupt_plan(entity, cause)
    }

    /// Breaks off what an entity planned and the wait it is in, and asks
    /// whoever decides for it what it does next (03 §5.3): its plan is
    /// emptied, its wait ended (decision D-871), and a directly
    /// controlled entity is asked with a `Free` request, anyone else by
    /// `decision_needed`. An action that is running runs on.
    pub fn interrupt_plan(&mut self, who: EntityId, cause: Cause) -> Result<(), Reason> {
        self.known(who)?;
        self.clear_plan(who, None, cause);
        // A wait is broken off too: what interrupts a plan interrupts
        // waiting for an hour (decision D-871).
        self.end_wait(who);
        let control = self
            .entities
            .get(who)
            .map_or(Control::None, |record| record.control());
        let why = FreeReason::Interrupted(cause);
        if control == Control::Direct {
            if !self.decisions.has_free(who) {
                self.request_decision(who, DecisionKind::Free { why }, cause)?;
            }
        } else {
            self.emit(
                Where::Entity(who),
                cause,
                EventBody::Act(ActEvent::DecisionNeeded {
                    entity: who,
                    control,
                    why,
                }),
            );
        }
        Ok(())
    }

    /// Whether an entity is in a wait it was told to make (`Wait`, the
    /// `wait` effect), not idling because nothing decided for it.
    pub fn is_waiting(&self, entity: EntityId) -> bool {
        self.actions.wait_of(entity).is_some_and(|wait| !wait.idle)
    }

    /// The `ClearPlan` command.
    pub(crate) fn command_clear_plan(&mut self, entity: EntityId) -> Result<(), Reason> {
        self.controlled(entity)?;
        let cause = Cause {
            actor: Some(entity),
            instigator: Some(entity),
            kind: CauseKind::Action,
        };
        self.clear_plan(entity, None, cause);
        Ok(())
    }
}
