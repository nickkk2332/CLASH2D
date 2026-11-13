//! The reasons the action system gives when it declines (14 §5).
//!
//! Anything the engine declines to do says why: a stable code, a
//! sentence, and the values involved. These are the reasons for the
//! refusals of 03 §6 to §8, written down with the definitions so that
//! the runtime packages refuse with them and nothing is refused without
//! one. They are data: nothing here decides anything.
//!
//! A code is an id in the engine's own namespace, beside the ids of the
//! `reason` definitions that mods write (14 §15). A sentence is a text
//! template (05 §4.6) written for a reader, in which `self` is whoever
//! was refused and each value is a local of that name; the package that
//! builds the `reason` definition type compiles them, and until then the
//! test below does.
//!
//! What is not here: the cost, cooldown and rank of a grant (08 §5.1)
//! are refused by the package that builds grants, with reasons of its
//! own.

/// One value a reason carries.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct ReasonValue {
    /// The name, read in the sentence as `$name`.
    pub name: &'static str,
    /// The type of the value, as the expression language writes types.
    pub ty: &'static str,
    /// What it is.
    pub doc: &'static str,
}

/// A reason the engine gives for declining (14 §5).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Reason {
    /// The stable code: an id in the `engine` namespace.
    pub code: &'static str,
    /// The sentence, as a text template for a reader.
    pub text: &'static str,
    /// The values involved, which the sentence may name.
    pub values: &'static [ReasonValue],
    /// When it is given, for the reference.
    pub doc: &'static str,
}

const fn value(name: &'static str, ty: &'static str, doc: &'static str) -> ReasonValue {
    ReasonValue { name, ty, doc }
}

const ACTION: ReasonValue = value("action", "id<action>", "The action that was asked for.");
const CHANNEL: ReasonValue = value("channel", "id<channel>", "The channel concerned.");
const CHANNEL_NAME: ReasonValue = value(
    "channel_name",
    "string",
    "What the channel is called: its `name`, or its id if it has none.",
);
const PHASE: ReasonValue = value("phase", "string", "The id of the phase the action is in.");
const PARAM: ReasonValue = value("param", "string", "The parameter's name.");
const ROLE: ReasonValue = value("role", "string", "The role's name.");
const KIND: ReasonValue = value("kind", "id<window_kind>", "The kind of the window.");

/// A parameter with no default was not given (03 §6.3).
pub const PARAMETER_MISSING: Reason = Reason {
    code: "engine:action_parameter_missing",
    text: "{cap(the(self))} must say which {$param}.",
    values: &[ACTION, PARAM],
    doc: "An action was asked for without a parameter that has no default.",
};

/// A parameter was given a value that is not of its type, or null.
pub const PARAMETER_INVALID: Reason = Reason {
    code: "engine:action_parameter_invalid",
    text: "That is not something {$param} can be.",
    values: &[
        ACTION,
        PARAM,
        value("expected", "string", "The type the parameter takes."),
    ],
    doc: "An action was asked for with a parameter whose value is null or of the wrong type.",
};

/// A list parameter was given too few or too many values (03 §6.3).
pub const LIST_LENGTH: Reason = Reason {
    code: "engine:action_list_length",
    text: "{cap($param)} takes from {$min} to {$max} and was given {$count}.",
    values: &[
        ACTION,
        PARAM,
        value("count", "int", "How many values were given."),
        value("min", "int", "The least the parameter takes."),
        value(
            "max",
            "int?",
            "The most it takes, if it states a greatest length.",
        ),
    ],
    doc: "A list parameter was given fewer values than its least or more than its greatest.",
};

/// The action's `requires` does not hold (03 §6.1). The sentence is the
/// action's `fail_text` where it has one.
pub const REQUIRES: Reason = Reason {
    code: "engine:action_requires",
    text: "{cap(the(self))} cannot do that now.",
    values: &[ACTION],
    doc: "An action's `requires` was false, or null, when the action was to start, or when a step of a plan came due. The action's `fail_text` is shown in place of this sentence.",
};

/// An action listed as what could be done at someone has no parameter
/// or role that takes them (12 §9.3, decision D-904). The sentence names
/// the action.
pub const NOT_AIMED: Reason = Reason {
    code: "engine:action_not_aimed",
    text: "That is not aimed at someone.",
    values: &[ACTION],
    doc: "`actions_with` was given a target, and the action has no parameter or role named `target` to take it: it cannot be done at them (a blow at a place, `Smash`, when someone is the target). The sentence names the action.",
};

/// The action needs footing and the actor has nothing under it
/// (03 §6.1).
pub const NO_FOOTING: Reason = Reason {
    code: "engine:action_no_footing",
    text: "{cap(the(self))} {verb(self, 'have')} nothing to stand on.",
    values: &[ACTION],
    doc: "An action whose `footing` is true was asked of an actor with nothing under it.",
};

/// Slots of a channel exist and too few of them are free (03 §6.2).
pub const CHANNEL_BUSY: Reason = Reason {
    code: "engine:action_channel_busy",
    text: "{cap(the(self))} {verb(self, 'need')} {count($needed, $channel_name)} free and {verb(self, 'have')} {$free}.",
    values: &[
        ACTION,
        CHANNEL,
        CHANNEL_NAME,
        value(
            "needed",
            "int",
            "How many free slots the action needs; for `all`, every enabled slot.",
        ),
        value("free", "int", "How many enabled slots are free."),
        value(
            "holders",
            "list<id<action>>",
            "The actions holding the rest.",
        ),
    ],
    doc: "An action needs more free slots of a channel than its participant has free: other actions hold them.",
};

/// The participant has too few enabled slots of a channel, free or not
/// (03 §6.2).
pub const CHANNEL_LACKING: Reason = Reason {
    code: "engine:action_channel_lacking",
    text: "{cap(the(self))} {verb(self, 'need')} {count($needed, $channel_name)} and {verb(self, 'have')} {$enabled}.",
    values: &[
        ACTION,
        CHANNEL,
        CHANNEL_NAME,
        value(
            "needed",
            "int",
            "How many slots the action needs; for `all`, at least one.",
        ),
        value(
            "enabled",
            "int",
            "How many slots of the channel the participant has that are not disabled.",
        ),
    ],
    doc: "An action needs more slots of a channel than its participant has enabled at all: a limb is missing, bound or broken.",
};

/// The slot a `part` parameter names cannot be the one used (03 §6.2).
pub const SLOT_UNAVAILABLE: Reason = Reason {
    code: "engine:action_slot_unavailable",
    text: "{cap(the(self))} cannot use that for this.",
    values: &[
        ACTION,
        CHANNEL,
        PARAM,
        value("state", "string", "`held`, `disabled` or `not_a_slot`."),
    ],
    doc: "A parameter named the slot an action is to use, and that slot is held by another action, disabled, or not a slot of a channel the action uses.",
};

/// A handler of `ActionStarting` cancelled the action (03 §13). The
/// handler's own reason is carried in its place where it gave one.
pub const VETOED: Reason = Reason {
    code: "engine:action_vetoed",
    text: "Something stops {the(self)}.",
    values: &[
        ACTION,
        value("rule", "string", "The id of the rule that cancelled it."),
    ],
    doc: "A handler of the `ActionStarting` hook cancelled the action without naming a reason of its own.",
};

/// The action's `requires_at_start` did not hold: it began and was
/// interrupted at once (03 §6.1).
pub const START_CONDITION: Reason = Reason {
    code: "engine:action_start_condition",
    text: "{cap(the(self))} cannot go through with it.",
    values: &[ACTION],
    doc: "An action's `requires_at_start` was false, or null, when the action began. The action is interrupted and its `on_interrupt` runs.",
};

/// A `Cancel` command met a phase that does not allow it (03 §6.4).
pub const CANNOT_CANCEL: Reason = Reason {
    code: "engine:action_cannot_cancel",
    text: "{cap(the(self))} {verb(self, 'be')} committed.",
    values: &[ACTION, PHASE],
    doc: "The actor tried to abandon an action in a phase whose `cancel` is false.",
};

/// The phase may be abandoned only for an action with certain tags, and
/// what was asked for has none of them (03 §6.4).
pub const CANCEL_ONLY_FOR: Reason = Reason {
    code: "engine:action_cancel_only_for",
    text: "{cap(the(self))} cannot break off for that.",
    values: &[
        ACTION,
        PHASE,
        value(
            "tags",
            "list<string>",
            "The tags the phase may be abandoned for.",
        ),
    ],
    doc: "The actor tried to abandon an action in a phase whose `cancel` lists tags, for something that carries none of them.",
};

/// More zero-duration phase changes in one tick than
/// `rules.max_transitions` allows: the action is ended (03 §6.4).
pub const TRANSITION_LIMIT: Reason = Reason {
    code: "engine:action_transition_limit",
    text: "{cap(the(self))} {verb(self, 'lose')} the thread.",
    values: &[
        ACTION,
        PHASE,
        value("limit", "int", "The value of `rules.max_transitions`."),
    ],
    doc: "An action changed phase without time passing more often than `rules.max_transitions` allows, and was ended with an error. A loop of phases of no length is the usual cause.",
};

/// A slot the action held was disabled, which interrupts it (03 §6.2).
pub const CHANNEL_DISABLED: Reason = Reason {
    code: "engine:action_channel_disabled",
    text: "{cap(the(self))} {verb(self, 'lose')} the use of {their(self)} {$channel_name}.",
    values: &[ACTION, CHANNEL, CHANNEL_NAME],
    doc: "A channel slot an action was using was disabled: the action is interrupted.",
};

/// The action does not list the window's kind in `reaction_to`
/// (03 §7.3).
pub const NOT_A_REACTION: Reason = Reason {
    code: "engine:reaction_not_listed",
    text: "That is no answer to this.",
    values: &[ACTION, KIND],
    doc: "An action was offered as a reaction to a window whose kind its `reaction_to` does not list.",
};

/// The window is aimed at someone else and the action does not allow
/// `on_behalf` (03 §7.1).
pub const NOT_ON_BEHALF: Reason = Reason {
    code: "engine:reaction_not_on_behalf",
    text: "{cap(the(self))} cannot answer for someone else that way.",
    values: &[ACTION, KIND],
    doc: "A reaction whose `on_behalf` is false was offered to a window that does not name the reactor.",
};

/// The reactor's own actions hold the slots the reaction needs and are
/// in phases that do not allow cancelling (03 §7.3).
pub const REACTION_CANNOT_CANCEL: Reason = Reason {
    code: "engine:reaction_cannot_cancel",
    text: "{cap(the(self))} {verb(self, 'be')} committed to something else.",
    values: &[
        ACTION,
        CHANNEL,
        value("holder", "id<action>", "The action holding the slot."),
        PHASE,
    ],
    doc: "A reaction needs slots held by the reactor's current actions, and one of those is in a phase that cannot be cancelled to make room.",
};

/// The candidate never noticed the window (03 §7.2).
pub const WINDOW_NOT_NOTICED: Reason = Reason {
    code: "engine:window_not_noticed",
    text: "{cap(the(self))} never {verb(self, 'see')} it coming.",
    values: &[KIND],
    doc: "A window's `noticed_if` was false for the candidate, or the candidate was no candidate at all.",
};

/// The window closed before the candidate's notice elapsed (03 §7.2).
pub const WINDOW_NOTICED_LATE: Reason = Reason {
    code: "engine:window_noticed_late",
    text: "{cap(the(self))} {verb(self, 'be')} too slow.",
    values: &[
        KIND,
        value("notice", "real", "Seconds the candidate needed to notice."),
        value("open", "real", "Seconds the window stayed open."),
    ],
    doc: "A window closed before the candidate's `notice` delay had passed on the candidate's clock.",
};

/// The window has already closed (03 §7.1).
pub const WINDOW_CLOSED: Reason = Reason {
    code: "engine:window_closed",
    text: "The moment has passed.",
    values: &[KIND],
    doc: "A reaction was asked for to a window that has closed.",
};

/// A role of a joint action has nobody in it (03 §8).
pub const ROLE_UNFILLED: Reason = Reason {
    code: "engine:role_unfilled",
    text: "{cap(the(self))} must say who.",
    values: &[ACTION, ROLE],
    doc: "A joint action was asked for without a participant for one of its roles, or with one who no longer exists.",
};

/// A participant declined a role that is not forced, or did not answer
/// by the end of the first phase (03 §8).
pub const ROLE_DECLINED: Reason = Reason {
    code: "engine:role_declined",
    text: "{cap(the($who))} will not.",
    values: &[
        ACTION,
        ROLE,
        value("who", "entity", "The participant who declined."),
        value("answered", "bool", "False if the offer ran out unanswered."),
    ],
    doc: "A role that is not forced was offered and declined, or was not answered before the action's first phase ended.",
};

/// A participant left a joint action, which ends it for all (03 §8).
pub const PARTICIPANT_LEFT: Reason = Reason {
    code: "engine:role_participant_left",
    text: "{cap(the($who))} {verb($who, 'be')} out of it.",
    values: &[
        ACTION,
        ROLE,
        value(
            "who",
            "entity",
            "The participant who was interrupted or disabled.",
        ),
    ],
    doc: "A participant of a joint action was interrupted, or lost a slot the action held: the action ends for everyone in it.",
};

/// Every reason of the action system, in the order of this module.
pub const REASONS: &[Reason] = &[
    PARAMETER_MISSING,
    PARAMETER_INVALID,
    LIST_LENGTH,
    REQUIRES,
    NOT_AIMED,
    NO_FOOTING,
    CHANNEL_BUSY,
    CHANNEL_LACKING,
    SLOT_UNAVAILABLE,
    VETOED,
    START_CONDITION,
    CANNOT_CANCEL,
    CANCEL_ONLY_FOR,
    TRANSITION_LIMIT,
    CHANNEL_DISABLED,
    NOT_A_REACTION,
    NOT_ON_BEHALF,
    REACTION_CANNOT_CANCEL,
    WINDOW_NOT_NOTICED,
    WINDOW_NOTICED_LATE,
    WINDOW_CLOSED,
    ROLE_UNFILLED,
    ROLE_DECLINED,
    PARTICIPANT_LEFT,
];

/// The reason with this code, if it is one of the action system's.
pub fn find(code: &str) -> Option<&'static Reason> {
    REASONS.iter().find(|reason| reason.code == code)
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use clash_data::{ENGINE_NAMESPACE, is_id_part};
    use clash_expr::{Context, KindSpec, Registry, Type, compile_template};

    use super::*;

    #[test]
    fn every_reason_has_a_code_of_its_own_in_the_engines_namespace() {
        let mut seen = BTreeSet::new();
        for reason in REASONS {
            let (namespace, name) = reason
                .code
                .split_once(':')
                .unwrap_or_else(|| panic!("`{}` has no namespace", reason.code));
            assert_eq!(namespace, ENGINE_NAMESPACE, "{}", reason.code);
            assert!(is_id_part(name), "{}", reason.code);
            assert!(seen.insert(reason.code), "`{}` is given twice", reason.code);
            assert_eq!(find(reason.code), Some(reason));
            assert!(!reason.doc.is_empty(), "{}", reason.code);
            for value in reason.values {
                assert!(is_id_part(value.name), "{}: {}", reason.code, value.name);
                assert!(!value.doc.is_empty(), "{}: {}", reason.code, value.name);
            }
        }
        assert!(find("engine:no_such_reason").is_none());
    }

    /// Every sentence is a template the expression language accepts,
    /// with `self` the one refused and the reason's values as locals.
    #[test]
    fn every_sentence_compiles_for_a_reader() {
        let mut registry = Registry::with_builtins();
        let entity = registry.add_kind(KindSpec::new("entity")).unwrap();
        for name in ["action", "channel", "window_kind"] {
            registry.add_def_type(name, "").unwrap();
        }
        for reason in REASONS {
            let mut context = Context::new().subject("self", Type::Subject(entity));
            for value in reason.values {
                let ty = registry
                    .parse_type(value.ty)
                    .unwrap_or_else(|| panic!("{}: `{}` is not a type", reason.code, value.ty));
                context = context.local(value.name, ty);
            }
            let outcome = compile_template(reason.text, &registry, &context);
            assert!(
                outcome.value.is_some() && outcome.diagnostics.is_empty(),
                "{}: {:?}",
                reason.code,
                outcome.diagnostics
            );
        }
    }
}
