//! [`ActEvent`]: what acting reports (03 §13): the events of actions,
//! plans and free decisions, and beside them the ones a rule produces
//! for a player (`message`, `choice_made`; 05 §10.6) and `tag_changed`.
//!
//! They are one variant of [`EventBody`](crate::EventBody), as the
//! world's events are, so that the packages that add events do not all
//! add arms to the same matches. Each is listed in [`ACT_EVENTS`] with
//! its subjects and fields; the two hooks of 03 §13, which handlers may
//! change or cancel, are in [`ACT_HOOKS`] and are raised as payloads.

use clash_core::{Dur, EntityId, Real, StateHash, StateHasher, Tick};
use clash_expr::{EventSpec, Registry, Type};
use serde::{Deserialize, Serialize};

use crate::bus::{EventType, FieldType, FieldValue};
use crate::decision::{ChoiceId, FreeReason};
use crate::entity::Control;
use crate::event::{CAUSE_SUBJECTS, EventInfo, Subject, control_name};
use crate::later::ActionId;
use crate::reason::Reason;

/// The hook raised before an action claims its channels (03 §13).
pub const ACTION_STARTING: &str = "action_starting";
/// The hook raised before a phase is scheduled (03 §13).
pub const PHASE_TIMING: &str = "phase_timing";

/// Something that happened to an action, a plan, a decision, a message
/// or a tag. Later packages add variants: match with a wildcard arm.
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum ActEvent {
    /// An action began: its channels are claimed and `on_start` is
    /// about to run.
    ActionStarted {
        /// Who acts.
        actor: EntityId,
        /// The running action.
        id: ActionId,
        /// The qualified id of its definition.
        action: String,
    },
    /// A phase of an action began (03 §6.4).
    PhaseEntered {
        /// Who acts.
        actor: EntityId,
        /// The running action.
        id: ActionId,
        /// The qualified id of its definition.
        action: String,
        /// The phase's id.
        phase: String,
        /// Whether the phase lasts until it is let go.
        held: bool,
        /// How long it will last, in local time of the actor's clock:
        /// its `time` over the tempo, or a held phase's `max`. `None`
        /// for a held phase with no `max`.
        duration: Option<Dur>,
        /// The universal tick it is due to end at, as the clocks now
        /// run. `None` where there is no duration, or the clock is
        /// stopped.
        due: Option<Tick>,
    },
    /// A phase of an action ended normally.
    PhaseExited {
        /// Who acts.
        actor: EntityId,
        /// The running action.
        id: ActionId,
        /// The qualified id of its definition.
        action: String,
        /// The phase's id.
        phase: String,
    },
    /// An action ran to its end: `on_finish` has run and its channels
    /// are free.
    ActionFinished {
        /// Who acted.
        actor: EntityId,
        /// The action that ran.
        id: ActionId,
        /// The qualified id of its definition.
        action: String,
    },
    /// The actor abandoned an action: `on_cancel` has run.
    ActionCancelled {
        /// Who acted.
        actor: EntityId,
        /// The action that ran.
        id: ActionId,
        /// The qualified id of its definition.
        action: String,
        /// The phase it was in.
        phase: String,
    },
    /// An action was ended against the actor's will: `on_interrupt` has
    /// run.
    ActionInterrupted {
        /// Who acted.
        actor: EntityId,
        /// The action that ran.
        id: ActionId,
        /// The qualified id of its definition.
        action: String,
        /// The phase it was in.
        phase: String,
        /// Why.
        reason: Reason,
    },
    /// An action ended because its actor left the simulation (03 §9):
    /// `on_abandon` has run.
    ActionAbandoned {
        /// Who acted.
        actor: EntityId,
        /// The action that ran.
        id: ActionId,
        /// The qualified id of its definition.
        action: String,
    },
    /// An action that was asked for did not start.
    ActionRefused {
        /// Who was to act.
        actor: EntityId,
        /// The qualified id of the action's definition, as it was asked
        /// for.
        action: String,
        /// Why not.
        reason: Reason,
    },
    /// An entity's plan was emptied (03 §5.3).
    PlanCleared {
        /// Whose.
        actor: EntityId,
        /// Why, if a step could not be started or a rule interrupted
        /// it.
        reason: Option<Reason>,
    },
    /// A phase that opens a reaction window began (03 §7.1). Windows
    /// are a later package's: nobody is offered it yet.
    WindowOpened {
        /// Whose action opened it.
        actor: EntityId,
        /// The running action.
        id: ActionId,
        /// The qualified id of its definition.
        action: String,
        /// The qualified id of the window's kind.
        kind: String,
        /// The id of the phase whose start closes it.
        closes: String,
        /// Those it is aimed at.
        aimed_at: Vec<EntityId>,
    },
    /// An entity that nobody controls directly is free to act and has
    /// nothing planned (03 §5.2): whatever decides for it is asked.
    DecisionNeeded {
        /// Who is free.
        entity: EntityId,
        /// Who decides for it: never `direct`.
        control: Control,
        /// Why it is free.
        why: FreeReason,
    },
    /// A rule's `message` (05 §10.6): a line for the log of whoever
    /// controls `to`, written for `to` as its reader.
    Message {
        /// Who it is for.
        to: EntityId,
        /// The line, rendered for that reader.
        text: String,
        /// How it is to be shown: a style name of the interface, or
        /// empty.
        style: String,
    },
    /// A choice put by a rule's `ask` was answered (05 §10.6).
    ChoiceMade {
        /// Who chose.
        who: EntityId,
        /// The choice.
        choice: ChoiceId,
        /// The position of the option taken.
        option: u32,
        /// Its label, as it was shown.
        label: String,
    },
    /// An entity was given a tag, or lost one.
    TagChanged {
        /// Who.
        entity: EntityId,
        /// The tag.
        tag: String,
        /// Whether it was added; false if it was taken away.
        added: bool,
    },
}

/// The name of a reason an entity is free, as data writes it.
pub const fn free_name(why: &FreeReason) -> &'static str {
    match why {
        FreeReason::Started => "started",
        FreeReason::Finished(_) => "finished",
        FreeReason::Interrupted(_) => "interrupted",
        FreeReason::PlanEmpty(_) => "plan_empty",
        FreeReason::Waited => "waited",
    }
}

fn number(value: u64) -> FieldValue {
    FieldValue::from(i64::try_from(value).unwrap_or(i64::MAX))
}

impl ActEvent {
    /// The name of the event as rules write it.
    pub const fn name(&self) -> &'static str {
        match self {
            ActEvent::ActionStarted { .. } => "action_started",
            ActEvent::PhaseEntered { .. } => "phase_entered",
            ActEvent::PhaseExited { .. } => "phase_exited",
            ActEvent::ActionFinished { .. } => "action_finished",
            ActEvent::ActionCancelled { .. } => "action_cancelled",
            ActEvent::ActionInterrupted { .. } => "action_interrupted",
            ActEvent::ActionAbandoned { .. } => "action_abandoned",
            ActEvent::ActionRefused { .. } => "action_refused",
            ActEvent::PlanCleared { .. } => "plan_cleared",
            ActEvent::WindowOpened { .. } => "window_opened",
            ActEvent::DecisionNeeded { .. } => "decision_needed",
            ActEvent::Message { .. } => "message",
            ActEvent::ChoiceMade { .. } => "choice_made",
            ActEvent::TagChanged { .. } => "tag_changed",
        }
    }

    /// Who the event is about: bound as `self`.
    pub const fn entity(&self) -> EntityId {
        match self {
            ActEvent::ActionStarted { actor, .. }
            | ActEvent::PhaseEntered { actor, .. }
            | ActEvent::PhaseExited { actor, .. }
            | ActEvent::ActionFinished { actor, .. }
            | ActEvent::ActionCancelled { actor, .. }
            | ActEvent::ActionInterrupted { actor, .. }
            | ActEvent::ActionAbandoned { actor, .. }
            | ActEvent::ActionRefused { actor, .. }
            | ActEvent::PlanCleared { actor, .. }
            | ActEvent::WindowOpened { actor, .. } => *actor,
            ActEvent::DecisionNeeded { entity, .. } | ActEvent::TagChanged { entity, .. } => {
                *entity
            }
            ActEvent::Message { to, .. } => *to,
            ActEvent::ChoiceMade { who, .. } => *who,
        }
    }

    /// The subjects of what happened, by name: always `self`.
    pub fn subjects(&self) -> Vec<(&str, Subject)> {
        vec![("self", Subject::Entity(self.entity()))]
    }

    /// The running action the event is about, if it is about one.
    pub const fn action_id(&self) -> Option<ActionId> {
        match self {
            ActEvent::ActionStarted { id, .. }
            | ActEvent::PhaseEntered { id, .. }
            | ActEvent::PhaseExited { id, .. }
            | ActEvent::ActionFinished { id, .. }
            | ActEvent::ActionCancelled { id, .. }
            | ActEvent::ActionInterrupted { id, .. }
            | ActEvent::ActionAbandoned { id, .. }
            | ActEvent::WindowOpened { id, .. } => Some(*id),
            _ => None,
        }
    }

    /// One of the event's fields, by name: what a rule's `filter` tests
    /// and `event.name` reads.
    pub fn field(&self, name: &str) -> Option<FieldValue> {
        if name == "id" {
            return self.action_id().map(|id| number(u64::from(id.raw())));
        }
        match self {
            ActEvent::ActionStarted { action, .. }
            | ActEvent::ActionFinished { action, .. }
            | ActEvent::ActionAbandoned { action, .. } => match name {
                "action" => Some(FieldValue::text(action)),
                _ => None,
            },
            ActEvent::PhaseEntered {
                action,
                phase,
                held,
                duration,
                due,
                ..
            } => match name {
                "action" => Some(FieldValue::text(action)),
                "phase" => Some(FieldValue::text(phase)),
                "held" => Some(FieldValue::from(*held)),
                "duration" => duration.map(|dur| FieldValue::from(dur.as_secs_real())),
                "due" => due.map(|tick| FieldValue::from(tick.as_micros())),
                _ => None,
            },
            ActEvent::PhaseExited { action, phase, .. }
            | ActEvent::ActionCancelled { action, phase, .. } => match name {
                "action" => Some(FieldValue::text(action)),
                "phase" => Some(FieldValue::text(phase)),
                _ => None,
            },
            ActEvent::ActionInterrupted {
                action,
                phase,
                reason,
                ..
            } => match name {
                "action" => Some(FieldValue::text(action)),
                "phase" => Some(FieldValue::text(phase)),
                "code" => Some(FieldValue::text(&reason.code)),
                "message" => Some(FieldValue::text(&reason.message)),
                _ => None,
            },
            ActEvent::ActionRefused { action, reason, .. } => match name {
                "action" => Some(FieldValue::text(action)),
                "code" => Some(FieldValue::text(&reason.code)),
                "message" => Some(FieldValue::text(&reason.message)),
                _ => None,
            },
            ActEvent::PlanCleared { reason, .. } => match name {
                "code" => reason.as_ref().map(|reason| FieldValue::text(&reason.code)),
                "message" => reason
                    .as_ref()
                    .map(|reason| FieldValue::text(&reason.message)),
                _ => None,
            },
            ActEvent::WindowOpened {
                action,
                kind,
                closes,
                aimed_at,
                ..
            } => match name {
                "action" => Some(FieldValue::text(action)),
                "kind" => Some(FieldValue::text(kind)),
                "closes" => Some(FieldValue::text(closes)),
                "count" => Some(number(aimed_at.len() as u64)),
                _ => None,
            },
            ActEvent::DecisionNeeded { control, why, .. } => match name {
                "control" => Some(FieldValue::text(control_name(*control))),
                "why" => Some(FieldValue::text(free_name(why))),
                _ => None,
            },
            ActEvent::Message { text, style, .. } => match name {
                "text" => Some(FieldValue::text(text)),
                "style" => Some(FieldValue::text(style)),
                _ => None,
            },
            ActEvent::ChoiceMade {
                choice,
                option,
                label,
                ..
            } => match name {
                "choice" => Some(number(choice.raw())),
                "option" => Some(FieldValue::from(i64::from(*option))),
                "label" => Some(FieldValue::text(label)),
                _ => None,
            },
            ActEvent::TagChanged { tag, added, .. } => match name {
                "tag" => Some(FieldValue::text(tag)),
                "added" => Some(FieldValue::from(*added)),
                _ => None,
            },
        }
    }
}

/// Hashes the event's name and then what it carries, in the order
/// declared.
impl StateHash for ActEvent {
    fn state_hash(&self, hasher: &mut StateHasher) {
        hasher.write_str(self.name());
        self.entity().state_hash(hasher);
        self.action_id().state_hash(hasher);
        match self {
            ActEvent::ActionStarted { action, .. }
            | ActEvent::ActionFinished { action, .. }
            | ActEvent::ActionAbandoned { action, .. } => hasher.write_str(action),
            ActEvent::PhaseEntered {
                action,
                phase,
                held,
                duration,
                due,
                ..
            } => {
                hasher.write_str(action);
                hasher.write_str(phase);
                held.state_hash(hasher);
                duration.state_hash(hasher);
                due.state_hash(hasher);
            }
            ActEvent::PhaseExited { action, phase, .. }
            | ActEvent::ActionCancelled { action, phase, .. } => {
                hasher.write_str(action);
                hasher.write_str(phase);
            }
            ActEvent::ActionInterrupted {
                action,
                phase,
                reason,
                ..
            } => {
                hasher.write_str(action);
                hasher.write_str(phase);
                reason.state_hash(hasher);
            }
            ActEvent::ActionRefused { action, reason, .. } => {
                hasher.write_str(action);
                reason.state_hash(hasher);
            }
            ActEvent::PlanCleared { reason, .. } => reason.state_hash(hasher),
            ActEvent::WindowOpened {
                action,
                kind,
                closes,
                aimed_at,
                ..
            } => {
                hasher.write_str(action);
                hasher.write_str(kind);
                hasher.write_str(closes);
                aimed_at.state_hash(hasher);
            }
            ActEvent::DecisionNeeded { control, why, .. } => {
                control.state_hash(hasher);
                why.state_hash(hasher);
            }
            ActEvent::Message { text, style, .. } => {
                hasher.write_str(text);
                hasher.write_str(style);
            }
            ActEvent::ChoiceMade {
                choice,
                option,
                label,
                ..
            } => {
                choice.state_hash(hasher);
                hasher.write_u32(*option);
                hasher.write_str(label);
            }
            ActEvent::TagChanged { tag, added, .. } => {
                hasher.write_str(tag);
                added.state_hash(hasher);
            }
        }
    }
}

const ACTION: (&str, &str, &str) = (
    "action",
    "string",
    "The qualified id of the action's definition.",
);
const ID: (&str, &str, &str) = (
    "id",
    "int",
    "The number of the running action, which no other action of this simulation ever has.",
);
const PHASE: (&str, &str, &str) = ("phase", "string", "The id of the phase.");
const CODE: (&str, &str, &str) = ("code", "string", "The stable code of the reason.");
const MESSAGE: (&str, &str, &str) = ("message", "string", "The reason, in a sentence.");
const ACTOR: &[(&str, &str)] = &[("self", "entity")];

/// The events of [`ActEvent`], for the registry and the generated
/// reference. Every one binds who it is about as `self`, and
/// [`CAUSE_SUBJECTS`] beside.
pub const ACT_EVENTS: &[EventInfo] = &[
    EventInfo {
        name: "action_started",
        subjects: ACTOR,
        fields: &[ACTION, ID],
        doc: "An action began: its channels are claimed and its `on_start` is about to run (03 §6.6).",
    },
    EventInfo {
        name: "phase_entered",
        subjects: ACTOR,
        fields: &[
            ACTION,
            ID,
            PHASE,
            (
                "held",
                "bool",
                "Whether the phase lasts until it is let go.",
            ),
            (
                "duration",
                "real?",
                "How long it will last, in seconds of the actor's own clock; null for a held phase with no `max`.",
            ),
            (
                "due",
                "int?",
                "The universal tick it is due to end at as the clocks now run, in microseconds; null where there is no duration or the clock is stopped.",
            ),
        ],
        doc: "A phase of an action began. The event's own stamp is the start; `duration` and `due` are the scheduled end (03 §12).",
    },
    EventInfo {
        name: "phase_exited",
        subjects: ACTOR,
        fields: &[ACTION, ID, PHASE],
        doc: "A phase of an action ended normally: its `on_exit` has run.",
    },
    EventInfo {
        name: "action_finished",
        subjects: ACTOR,
        fields: &[ACTION, ID],
        doc: "An action ran to its end: `on_finish` has run and its channels are free.",
    },
    EventInfo {
        name: "action_cancelled",
        subjects: ACTOR,
        fields: &[ACTION, ID, PHASE],
        doc: "The actor abandoned an action in a phase that allows it: `on_cancel` has run and its channels are free.",
    },
    EventInfo {
        name: "action_interrupted",
        subjects: ACTOR,
        fields: &[ACTION, ID, PHASE, CODE, MESSAGE],
        doc: "An action was ended against the actor's will, by the `interrupt` effect, a lost channel slot, a failed `requires_at_start` or the transition limit: `on_interrupt` has run.",
    },
    EventInfo {
        name: "action_abandoned",
        subjects: ACTOR,
        fields: &[ACTION, ID],
        doc: "An action ended because its actor left the world or was removed (03 §9): `on_abandon` has run.",
    },
    EventInfo {
        name: "action_refused",
        subjects: ACTOR,
        fields: &[ACTION, CODE, MESSAGE],
        doc: "An action that an effect or a plan asked for did not start: a failed `requires`, slots lacking, a veto. A command refused before any handler ran is told its reason directly and raises nothing; one that a handler of `action_starting` refused raises it.",
    },
    EventInfo {
        name: "plan_cleared",
        subjects: ACTOR,
        fields: &[
            (
                "code",
                "string?",
                "The code of the reason, if a step could not be started or a rule interrupted the plan.",
            ),
            ("message", "string?", "That reason, in a sentence."),
        ],
        doc: "An entity's plan was emptied: by `ClearPlan`, by `clear_plan` or `interrupt_plan`, or because a step's `requires` failed when it came due (03 §5.3).",
    },
    EventInfo {
        name: "window_opened",
        subjects: ACTOR,
        fields: &[
            ACTION,
            ID,
            ("kind", "string", "The qualified id of the window's kind."),
            (
                "closes",
                "string",
                "The id of the phase whose start closes it.",
            ),
            ("count", "int", "How many it is aimed at."),
        ],
        doc: "A phase that opens a reaction window began (03 §7.1). Until windows are built nobody is offered it and the action carries on.",
    },
    EventInfo {
        name: "decision_needed",
        subjects: ACTOR,
        fields: &[
            (
                "control",
                "string",
                "Who decides for it: `none` or `delegated`.",
            ),
            (
                "why",
                "string",
                "Why it is free: `started`, `finished`, `interrupted`, `plan_empty` or `waited`.",
            ),
        ],
        doc: "An entity that is not directly controlled is free to act and has nothing planned (03 §5.2). Whatever decides for it answers by starting an action or a wait; if nothing does, it idles and is asked again.",
    },
    EventInfo {
        name: "message",
        subjects: ACTOR,
        fields: &[
            (
                "text",
                "string",
                "The line, rendered for `self` as its reader.",
            ),
            (
                "style",
                "string",
                "How it is to be shown: a style name of the interface, or empty.",
            ),
        ],
        doc: "A rule's `message` (05 §10.6): a line for the log of whoever controls `self`.",
    },
    EventInfo {
        name: "choice_made",
        subjects: ACTOR,
        fields: &[
            ("choice", "int", "The number of the choice."),
            (
                "option",
                "int",
                "The position of the option taken, from zero.",
            ),
            ("label", "string", "Its label, as it was shown."),
        ],
        doc: "A choice put by a rule's `ask` was answered (05 §10.6): by the `Choose` command, or at once for an entity that is not directly controlled. The option's effects run next.",
    },
    EventInfo {
        name: "tag_changed",
        subjects: ACTOR,
        fields: &[
            ("tag", "string", "The tag."),
            (
                "added",
                "bool",
                "Whether it was added; false if it was taken away.",
            ),
        ],
        doc: "An entity was given a tag it did not have, or lost one it had (`add_tag`, `remove_tag`).",
    },
];

/// One of the hooks the action system raises (03 §13).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct HookInfo {
    /// What the registry and the reference are told.
    pub info: EventInfo,
    /// The fields a handler may change.
    pub mutable: &'static [&'static str],
}

/// The hooks of 03 §13. Each is delivered at once, before what it
/// precedes; a handler may change the fields listed, or cancel.
pub const ACT_HOOKS: &[HookInfo] = &[
    HookInfo {
        info: EventInfo {
            name: ACTION_STARTING,
            subjects: ACTOR,
            fields: &[
                ACTION,
                (
                    "params",
                    "any",
                    "The action's parameters by name, as a map, defaults filled in. Mutable: a value a handler puts there is held to the parameter's type, and one that does not fit refuses the action.",
                ),
            ],
            doc: "Runs before an action claims its channels and enters its first phase. A handler may change its parameters, or cancel with a reason, which is what whoever asked is told; with none, `engine:action_vetoed`.",
        },
        mutable: &["params"],
    },
    HookInfo {
        info: EventInfo {
            name: PHASE_TIMING,
            subjects: ACTOR,
            fields: &[
                ACTION,
                ID,
                PHASE,
                (
                    "duration",
                    "real",
                    "How long the phase is to last, in seconds of the actor's own clock, after tempo. Mutable; a value below zero counts as zero.",
                ),
            ],
            doc: "Runs before a phase that has a duration is scheduled. A handler may change the duration. Cancelling it changes nothing: the phase is scheduled as it was.",
        },
        mutable: &["duration"],
    },
];

/// Whether an event is one of the engine's own that content may not
/// raise with `emit`.
pub fn is_engine_event(name: &str) -> bool {
    ACT_HOOKS.iter().any(|hook| hook.info.name == name)
}

/// The type of a field, from how a table writes it.
fn field_type(written: &str) -> FieldType {
    match written.trim_end_matches('?') {
        "bool" => FieldType::Bool,
        "int" => FieldType::Int,
        "real" => FieldType::Real,
        "string" => FieldType::Str,
        _ => FieldType::Any,
    }
}

/// The hooks as the bus keeps event types: raised as payloads.
pub(crate) fn hook_types() -> Vec<EventType> {
    ACT_HOOKS
        .iter()
        .map(|hook| {
            let mut ty = EventType::new(hook.info.name).doc(hook.info.doc);
            for (name, kind) in hook.info.subjects {
                ty = ty.subject(*name, *kind);
            }
            for (name, written, doc) in hook.info.fields {
                ty = ty.field(*name, field_type(written), *doc);
            }
            ty.hook(hook.mutable.iter().copied())
        })
        .collect()
}

/// Enters the hooks in an expression registry, so that a rule may
/// listen for one. Those already there are left alone.
pub(crate) fn register_hooks(registry: &mut Registry) {
    for hook in ACT_HOOKS {
        if registry.event_id(hook.info.name).is_some() {
            continue;
        }
        let mut spec = EventSpec::new(hook.info.name).doc(hook.info.doc);
        for (name, kind) in hook.info.subjects.iter().chain(CAUSE_SUBJECTS) {
            if let Some(kind) = registry.kind_id(kind) {
                spec = spec.subject(name.trim_end_matches('?'), kind);
            }
        }
        for (name, written, doc) in hook.info.fields {
            let ty = registry.parse_type(written).unwrap_or(Type::Any);
            spec = spec.field(*name, ty, *doc);
        }
        let _ = registry.add_event(spec.hook(hook.mutable.iter().copied()));
    }
}

/// A field that holds seconds, as a duration: null and what is not a
/// number give `None`, and a negative number no time at all.
pub(crate) fn duration_of(field: Option<&FieldValue>) -> Option<Dur> {
    let secs = field?.as_real()?;
    Some(Dur::from_secs_real(secs.max(Real::ZERO)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_event_is_in_the_table_with_its_fields() {
        let entity = EntityId::new(1, 0);
        let id = ActionId::new(3);
        let reason = Reason::new("x", "y");
        let events = [
            ActEvent::ActionStarted {
                actor: entity,
                id,
                action: "m:a".to_owned(),
            },
            ActEvent::PhaseEntered {
                actor: entity,
                id,
                action: "m:a".to_owned(),
                phase: "windup".to_owned(),
                held: false,
                duration: Some(Dur::from_millis(300)),
                due: Some(Tick(300_000)),
            },
            ActEvent::PhaseExited {
                actor: entity,
                id,
                action: "m:a".to_owned(),
                phase: "windup".to_owned(),
            },
            ActEvent::ActionFinished {
                actor: entity,
                id,
                action: "m:a".to_owned(),
            },
            ActEvent::ActionCancelled {
                actor: entity,
                id,
                action: "m:a".to_owned(),
                phase: "windup".to_owned(),
            },
            ActEvent::ActionInterrupted {
                actor: entity,
                id,
                action: "m:a".to_owned(),
                phase: "windup".to_owned(),
                reason: reason.clone(),
            },
            ActEvent::ActionAbandoned {
                actor: entity,
                id,
                action: "m:a".to_owned(),
            },
            ActEvent::ActionRefused {
                actor: entity,
                action: "m:a".to_owned(),
                reason: reason.clone(),
            },
            ActEvent::PlanCleared {
                actor: entity,
                reason: Some(reason),
            },
            ActEvent::WindowOpened {
                actor: entity,
                id,
                action: "m:a".to_owned(),
                kind: "m:strike".to_owned(),
                closes: "impact".to_owned(),
                aimed_at: vec![entity],
            },
            ActEvent::DecisionNeeded {
                entity,
                control: Control::None,
                why: FreeReason::Started,
            },
            ActEvent::Message {
                to: entity,
                text: "You wait.".to_owned(),
                style: String::new(),
            },
            ActEvent::ChoiceMade {
                who: entity,
                choice: ChoiceId::new(1),
                option: 0,
                label: "Yes".to_owned(),
            },
            ActEvent::TagChanged {
                entity,
                tag: "guard".to_owned(),
                added: true,
            },
        ];
        assert_eq!(events.len(), ACT_EVENTS.len());
        for event in events {
            let info = ACT_EVENTS
                .iter()
                .find(|info| info.name == event.name())
                .unwrap_or_else(|| panic!("`{}` is not in ACT_EVENTS", event.name()));
            assert!(!info.doc.is_empty());
            for (field, _, doc) in info.fields {
                assert!(!doc.is_empty());
                assert!(
                    event.field(field).is_some(),
                    "`{}` lists `{field}` but it cannot be read",
                    info.name
                );
            }
            assert_eq!(event.subjects(), [("self", Subject::Entity(entity))]);
        }
    }

    #[test]
    fn the_hooks_are_types_of_the_bus_and_of_the_registry() {
        let types = hook_types();
        assert_eq!(types.len(), ACT_HOOKS.len());
        assert!(types.iter().all(|ty| ty.hook && !ty.mutable.is_empty()));
        let mut registry = Registry::with_builtins();
        crate::subject::Kinds::ensure(&mut registry).unwrap();
        register_hooks(&mut registry);
        register_hooks(&mut registry);
        let starting = registry
            .event(registry.event_id(ACTION_STARTING).unwrap())
            .unwrap();
        assert!(starting.spec.hook);
        assert_eq!(starting.spec.mutable, ["params"]);
        assert!(is_engine_event(PHASE_TIMING) && !is_engine_event("action_started"));
    }
}
