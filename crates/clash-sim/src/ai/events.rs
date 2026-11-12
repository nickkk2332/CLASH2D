//! The events minds raise (09 §17). Each is an engine event listed in
//! [`KERNEL_EVENTS`](crate::KERNEL_EVENTS), raised as a payload of its
//! registered type, on the clock of the mind it is about.
//!
//! `method_chosen`, `step_started` and `option_chosen` are raised only
//! while something listens for them (09 §17): they let a rule follow a
//! decision as the inspector does.

use clash_core::{EntityId, Real};
use clash_expr::Value;
use clash_world::{Cause, CauseKind};

use super::defs::{GoalId, StepKind};
use super::think::goal_name;
use crate::Sim;
use crate::bus::{EventPayload, FieldValue};
use crate::event::{EventBody, Subject, Where};
use crate::reason::Reason;

/// `goal_adopted`: a mind took up a goal.
pub const GOAL_ADOPTED: &str = "goal_adopted";
/// `goal_done`: a goal held was done.
pub const GOAL_DONE: &str = "goal_done";
/// `goal_failed`: a goal held failed and was set aside.
pub const GOAL_FAILED: &str = "goal_failed";
/// `step_failed`: a step of a method failed.
pub const STEP_FAILED: &str = "step_failed";
/// `method_chosen`: a mind began to follow a method.
pub const METHOD_CHOSEN: &str = "method_chosen";
/// `step_started`: a mind began a step.
pub const STEP_STARTED: &str = "step_started";
/// `option_chosen`: a `choose` step started an action.
pub const OPTION_CHOSEN: &str = "option_chosen";
/// `said`: a character said something.
pub const SAID: &str = "said";
/// `route_blocked`: a path in use was found blocked by a change.
pub const ROUTE_BLOCKED: &str = "route_blocked";

/// The events of this package, by name.
pub const NAMES: &[&str] = &[
    GOAL_ADOPTED,
    GOAL_DONE,
    GOAL_FAILED,
    STEP_FAILED,
    METHOD_CHOSEN,
    STEP_STARTED,
    OPTION_CHOSEN,
    SAID,
    ROUTE_BLOCKED,
];

/// What a mind does is the mind's own doing.
fn cause(mind: EntityId) -> Cause {
    Cause {
        actor: Some(mind),
        instigator: Some(mind),
        kind: CauseKind::Action,
    }
}

fn raise(sim: &mut Sim, mind: EntityId, payload: EventPayload) {
    sim.emit(Where::Entity(mind), cause(mind), EventBody::Custom(payload));
}

fn about(name: &str, mind: EntityId) -> EventPayload {
    EventPayload::new(name).subject("self", Subject::Entity(mind))
}

/// A value of an expression as an event's field.
fn field(sim: &Sim, value: &Value) -> FieldValue {
    match value {
        Value::Null => FieldValue::Null,
        Value::Subject(_) => sim
            .subject_of(value)
            .map_or(FieldValue::Null, FieldValue::Subject),
        other => FieldValue::Plain(other.to_stored()),
    }
}

/// Raises `goal_adopted`.
pub(super) fn goal_adopted(
    sim: &mut Sim,
    mind: EntityId,
    goal: GoalId,
    about_value: &Value,
    score: Real,
    previous: Option<GoalId>,
) {
    let previous = previous.map_or(FieldValue::Null, |goal| {
        FieldValue::text(goal_name(sim, goal))
    });
    let payload = about(GOAL_ADOPTED, mind)
        .with("goal", FieldValue::text(goal_name(sim, goal)))
        .with("about", field(sim, about_value))
        .with("score", score)
        .with("previous", previous);
    raise(sim, mind, payload);
}

/// Raises `goal_done`.
pub(super) fn goal_done(sim: &mut Sim, mind: EntityId, goal: GoalId) {
    let payload = about(GOAL_DONE, mind).with("goal", FieldValue::text(goal_name(sim, goal)));
    raise(sim, mind, payload);
}

/// Raises `goal_failed`.
pub(super) fn goal_failed(sim: &mut Sim, mind: EntityId, goal: GoalId, reason: &Reason) {
    let payload = about(GOAL_FAILED, mind)
        .with("goal", FieldValue::text(goal_name(sim, goal)))
        .with("code", reason.code.as_str())
        .with("message", reason.message.as_str());
    raise(sim, mind, payload);
}

/// Raises `step_failed`.
pub(super) fn step_failed(
    sim: &mut Sim,
    mind: EntityId,
    goal: GoalId,
    step: Option<StepKind>,
    reason: &Reason,
) {
    let payload = about(STEP_FAILED, mind)
        .with("goal", FieldValue::text(goal_name(sim, goal)))
        .with(
            "step",
            step.map_or(FieldValue::Null, |step| FieldValue::text(step.name())),
        )
        .with("code", reason.code.as_str())
        .with("message", reason.message.as_str());
    raise(sim, mind, payload);
}

/// Raises `method_chosen`, if anything listens.
pub(super) fn method_chosen(sim: &mut Sim, mind: EntityId, goal: GoalId, method: usize) {
    if !sim.bus.has_listeners(METHOD_CHOSEN) {
        return;
    }
    let payload = about(METHOD_CHOSEN, mind)
        .with("goal", FieldValue::text(goal_name(sim, goal)))
        .with("method", i64::try_from(method).unwrap_or(i64::MAX));
    raise(sim, mind, payload);
}

/// Raises `step_started`, if anything listens.
pub(super) fn step_started(sim: &mut Sim, mind: EntityId, goal: GoalId, step: StepKind) {
    if !sim.bus.has_listeners(STEP_STARTED) {
        return;
    }
    let payload = about(STEP_STARTED, mind)
        .with("goal", FieldValue::text(goal_name(sim, goal)))
        .with("step", step.name());
    raise(sim, mind, payload);
}

/// Raises `option_chosen`, if anything listens.
pub(super) fn option_chosen(sim: &mut Sim, mind: EntityId, purpose: &str, action: &str) {
    if !sim.bus.has_listeners(OPTION_CHOSEN) {
        return;
    }
    let payload = about(OPTION_CHOSEN, mind)
        .with("purpose", purpose)
        .with("action", action);
    raise(sim, mind, payload);
}

/// Raises `said`.
pub(super) fn said(sim: &mut Sim, mind: EntityId, text: &str, volume: Real) {
    let payload = about(SAID, mind).with("text", text).with("volume", volume);
    raise(sim, mind, payload);
}
