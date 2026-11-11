//! The reasons the rule runtime gives when something goes wrong while a
//! rule runs (14 §5; 05 §13.2).
//!
//! Each is the code of a [`RuleFault`](super::RuleFault). A fault never
//! stops the simulation: the effect is skipped, or the run is ended,
//! and the fault is handed to the host, which reports it.

use crate::reason::ReasonInfo;

/// A run executed more effects and loop iterations than
/// `rules.max_steps`.
pub const STEP_LIMIT: &str = "rule.step_limit";
/// Rules called each other more deeply than `rules.max_call_depth`.
pub const CALL_DEPTH: &str = "rule.call_depth";
/// More rules were queued than `rules.max_queued`.
pub const QUEUE_LIMIT: &str = "rule.queue_limit";
/// An effect would have written a value larger than
/// `rules.max_value_size`.
pub const VALUE_TOO_LARGE: &str = "rule.value_too_large";
/// An effect would have written a value nested more deeply than
/// `rules.max_value_depth`.
pub const VALUE_TOO_DEEP: &str = "rule.value_too_deep";
/// An effect needed a subject and was given null or something else.
pub const NO_SUBJECT: &str = "rule.no_subject";
/// A key or an index of a place does not fit what is there.
pub const BAD_KEY: &str = "rule.bad_key";
/// A value had a type the effect does not accept.
pub const TYPE_MISMATCH: &str = "rule.type_mismatch";
/// The simulation refused a write.
pub const WRITE_REFUSED: &str = "rule.write_refused";
/// A verb declined.
pub const VERB_REFUSED: &str = "rule.verb_refused";
/// The simulation refused to raise an event.
pub const EMIT_REFUSED: &str = "rule.emit_refused";
/// A computed rule id names no rule.
pub const UNKNOWN_RULE: &str = "rule.unknown_rule";
/// A rule was called with an argument it does not take, without one it
/// needs, or with one of the wrong type.
pub const BAD_ARGUMENT: &str = "rule.bad_argument";
/// A rule that is not an attached rule was attached.
pub const NOT_ATTACHABLE: &str = "rule.not_attachable";
/// An effect that acts on a hook ran where there is none.
pub const NOT_A_HOOK: &str = "rule.not_a_hook";
/// A random choice was asked for and there is no stream to draw from.
pub const NO_RANDOM: &str = "rule.no_random";
/// A timer effect ran; timers are not built yet.
pub const TIMERS_UNAVAILABLE: &str = "rule.timers_unavailable";
/// A query was iterated; queries are not built yet.
pub const QUERIES_UNAVAILABLE: &str = "rule.queries_unavailable";

/// A verb ran that nothing in the engine carries out.
pub const VERB_UNBOUND: &str = "rule.verb_unbound";
/// An expression a rule evaluated gave a run-time warning (05 §4.5).
pub const EXPRESSION_WARNING: &str = "rule.expression_warning";

/// A choice was answered and the effects of its options are no longer
/// where the `ask` was written.
pub const CHOICE_LOST: &str = "rule.choice_lost";
/// `Choose` named an option the choice does not have, or one that may
/// not be taken.
pub const CHOICE_REFUSED: &str = "rule.choice_refused";
/// The simulation would not deliver a `message`.
pub const MESSAGE_REFUSED: &str = "rule.message_refused";

/// The rule runtime's reasons, for the generated reference.
pub const RULE_REASONS: &[ReasonInfo] = &[
    ReasonInfo {
        code: STEP_LIMIT,
        doc: "A run executed more effects and loop iterations than `rules.max_steps`, counting those of the rules it called. The run was ended; what it had done stays done. Values: `rule`, `path`, `limit`.",
    },
    ReasonInfo {
        code: CALL_DEPTH,
        doc: "Rules called each other more deeply than `rules.max_call_depth`. The call was refused and gave null; the caller carried on. Values: `rule`, `path`, `limit`.",
    },
    ReasonInfo {
        code: QUEUE_LIMIT,
        doc: "More rules were queued than `rules.max_queued` before the queue was emptied. The rule was not queued. Values: `rule`, `path`, `limit`.",
    },
    ReasonInfo {
        code: VALUE_TOO_LARGE,
        doc: "An effect would have written a list or a map of more elements, or a text of more bytes, than `rules.max_value_size`. Nothing was written. Values: `rule`, `path`, `limit`.",
    },
    ReasonInfo {
        code: VALUE_TOO_DEEP,
        doc: "An effect would have written a value whose lists and maps nest more deeply than `rules.max_value_depth`. Nothing was written. Values: `rule`, `path`, `limit`.",
    },
    ReasonInfo {
        code: NO_SUBJECT,
        doc: "An effect needed a subject (whose variable to set, what to attach a rule to) and was given null or a value that is not a subject. The effect was skipped. Values: `rule`, `path`.",
    },
    ReasonInfo {
        code: BAD_KEY,
        doc: "A key of a place does not fit what is there: a key of something that is neither a map nor a list, an index outside a list, a key that is not a text or an id. Nothing was written. Values: `rule`, `path`.",
    },
    ReasonInfo {
        code: TYPE_MISMATCH,
        doc: "A value had a type the effect does not accept, where the type was not known at load: `add` to something that is neither a number nor a list, a `repeat` count that is not an int, a weight that is not a number, a list to iterate that is not a list, a returned value that is not of the rule's `returns`. The effect was skipped. Values: `rule`, `path`.",
    },
    ReasonInfo {
        code: WRITE_REFUSED,
        doc: "The simulation refused to set or remove a variable or a property. Values: `rule`, `path`, `reason` (the code of the simulation's reason).",
    },
    ReasonInfo {
        code: VERB_REFUSED,
        doc: "A verb declined. The effect did nothing and the rule carried on. Values: `rule`, `path`, `verb`, `reason` (the code of the verb's reason).",
    },
    ReasonInfo {
        code: EMIT_REFUSED,
        doc: "The simulation refused to raise an event: it is one of the engine's own, or what was given does not match its type. Values: `rule`, `path`, `reason`.",
    },
    ReasonInfo {
        code: UNKNOWN_RULE,
        doc: "An expression that should give the id of a rule gave something else, or the id of a rule that did not load. The call was refused. Values: `rule`, `path`.",
    },
    ReasonInfo {
        code: BAD_ARGUMENT,
        doc: "A rule was called with a parameter it does not have, without one that has no default, or with a value of the wrong type. The call was refused and gave null. Values: `rule`, `path`, `parameter`.",
    },
    ReasonInfo {
        code: NOT_ATTACHABLE,
        doc: "`attach_rule` was given a rule that is not an attached rule: one that listens for no event, or one that runs for every event of its type. Nothing was attached. Values: `rule`, `path`.",
    },
    ReasonInfo {
        code: NOT_A_HOOK,
        doc: "`modify` or `cancel_event` ran where the event being handled is not a hook. Nothing was changed. Values: `rule`, `path`.",
    },
    ReasonInfo {
        code: NO_RANDOM,
        doc: "`weighted` had to choose at random and the simulation has no stream to draw from. No entry was run. Values: `rule`, `path`.",
    },
    ReasonInfo {
        code: TIMERS_UNAVAILABLE,
        doc: "`after`, `every` or `cancel_timer` ran. Timers are read and kept and not yet built: nothing was scheduled. Values: `rule`, `path`.",
    },
    ReasonInfo {
        code: QUERIES_UNAVAILABLE,
        doc: "`for_each` was given a query. Queries are read and kept and not yet built: the body did not run. Values: `rule`, `path`.",
    },
    ReasonInfo {
        code: CHOICE_LOST,
        doc: "A choice put by `ask` was answered, and the `ask` is no longer where it was written when the choice was put: the definitions changed under a save. The option's effects did not run. Values: `definition`, `path`.",
    },
    ReasonInfo {
        code: CHOICE_REFUSED,
        doc: "The `Choose` command named an option the choice does not have, or one whose `if` did not hold when the choice was put. The choice is still waiting. Values: `choice`, `option`.",
    },
    ReasonInfo {
        code: MESSAGE_REFUSED,
        doc: "A `message` was for something that is not an entity, or the simulation would not deliver it. Nothing was shown. Values: `rule`, `path`.",
    },
    ReasonInfo {
        code: VERB_UNBOUND,
        doc: "An effect called a verb that is in the registry and that nothing in the engine carries out: a verb of a package that is not built into this simulation. Nothing was done. Values: `verb`.",
    },
    ReasonInfo {
        code: EXPRESSION_WARNING,
        doc: "An expression that a rule or a list of effects evaluated gave a run-time warning (05 §4.5): a division by zero, a value of the wrong type. The operation gave null and the rule carried on. Values: `definition`, `path`, `count`.",
    },
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_rule_code_is_listed_once_and_documented() {
        let mut seen: Vec<&str> = Vec::new();
        for info in RULE_REASONS {
            assert!(
                info.code.starts_with("rule."),
                "{} is not a rule code",
                info.code
            );
            assert!(info.doc.len() > 40, "{} needs documenting", info.code);
            assert!(!seen.contains(&info.code), "{} is listed twice", info.code);
            seen.push(info.code);
        }
        let used = [
            STEP_LIMIT,
            CALL_DEPTH,
            QUEUE_LIMIT,
            VALUE_TOO_LARGE,
            VALUE_TOO_DEEP,
            NO_SUBJECT,
            BAD_KEY,
            TYPE_MISMATCH,
            WRITE_REFUSED,
            VERB_REFUSED,
            EMIT_REFUSED,
            UNKNOWN_RULE,
            BAD_ARGUMENT,
            NOT_ATTACHABLE,
            NOT_A_HOOK,
            NO_RANDOM,
            TIMERS_UNAVAILABLE,
            QUERIES_UNAVAILABLE,
            VERB_UNBOUND,
            EXPRESSION_WARNING,
            CHOICE_LOST,
            CHOICE_REFUSED,
            MESSAGE_REFUSED,
        ];
        for code in used {
            assert!(seen.contains(&code), "{code} is used and not listed");
        }
        assert_eq!(used.len(), seen.len());
    }
}
