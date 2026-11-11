//! [`Reason`]: why the simulation declined to do something (14 §5).
//!
//! Anything the engine declines to do returns a reason: a stable code, a
//! sentence, and the values involved. Reasons are what the player is
//! told, what a character's mind sees when a step fails, and what
//! `explain` prints. A refusal without a reason is a defect.
//!
//! # Adding a reason
//!
//! Declare its code and what it means in a table of [`ReasonInfo`], as
//! [`KERNEL_REASONS`] does, and build the reason with [`Reason::new`].
//! The generated reference reads every table, and a test in each package
//! checks that no code is used that its table does not list.
//!
//! ```
//! use clash_sim::Reason;
//!
//! let reason = Reason::new("entity.unknown", "there is no entity 7v0").with("entity", "7v0");
//! assert_eq!(reason.code, "entity.unknown");
//! assert_eq!(reason.to_string(), "there is no entity 7v0");
//! ```

use core::fmt;

use clash_core::{StateHash, StateHasher, Value};
use clash_time::TimeError;
use clash_world::WorldError;
use serde::{Deserialize, Serialize};

/// Why something was refused: a stable code, a sentence that can be
/// shown to the player, and the values involved (14 §5).
///
/// The code is what programs compare and what the reference lists. The
/// message is for people and may change. The values are the particulars
/// (which entity, which cell, by how much), by name, for a screen or a
/// rule that wants to say it its own way.
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub struct Reason {
    /// The stable code: a dotted name such as `entity.unknown`.
    pub code: String,
    /// One sentence, without a full stop, that can be shown as it is.
    pub message: String,
    /// The values involved, by name, in the order they were added.
    pub values: Vec<(String, Value)>,
}

impl Reason {
    /// A reason with a code and a sentence.
    pub fn new(code: impl Into<String>, message: impl Into<String>) -> Reason {
        Reason {
            code: code.into(),
            message: message.into(),
            values: Vec::new(),
        }
    }

    /// This reason with one more of the values involved.
    #[must_use]
    pub fn with(mut self, name: impl Into<String>, value: impl Into<Value>) -> Reason {
        self.values.push((name.into(), value.into()));
        self
    }

    /// One of the values involved, by name.
    pub fn value(&self, name: &str) -> Option<&Value> {
        self.values
            .iter()
            .find(|(have, _)| have == name)
            .map(|(_, value)| value)
    }

    /// The reason for a refusal by the world. The code says which
    /// refusal; the message is the world's own.
    pub fn from_world(error: &WorldError) -> Reason {
        let code = match error {
            WorldError::NotLoaded(_) => codes::WORLD_NOT_LOADED,
            WorldError::OutsideRealm { .. } => codes::WORLD_OUTSIDE_REALM,
            WorldError::UnknownGrid(_) => codes::WORLD_UNKNOWN_GRID,
            WorldError::UnknownArea(_) => codes::WORLD_UNKNOWN_AREA,
            WorldError::UnknownRealm(_) => codes::WORLD_UNKNOWN_REALM,
            WorldError::Time(error) => return Reason::from_time(error),
            _ => codes::WORLD_REFUSED,
        };
        Reason::new(code, error.to_string())
    }

    /// The reason for a refusal by the timeline.
    pub fn from_time(error: &TimeError) -> Reason {
        let code = match error {
            TimeError::UnknownClock(_) => codes::CLOCK_UNKNOWN,
            TimeError::BadRate(_) => codes::CLOCK_BAD_RATE,
            TimeError::ClockLoop { .. } => codes::CLOCK_LOOP,
            _ => codes::TIME_REFUSED,
        };
        Reason::new(code, error.to_string())
    }
}

impl fmt::Display for Reason {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl StateHash for Reason {
    fn state_hash(&self, hasher: &mut StateHasher) {
        hasher.write_str(&self.code);
        hasher.write_str(&self.message);
        hasher.write_len(self.values.len());
        for (name, value) in &self.values {
            hasher.write_str(name);
            value.state_hash(hasher);
        }
    }
}

/// One entry of a table of reasons: a code and what it means, for the
/// generated reference.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct ReasonInfo {
    /// The stable code.
    pub code: &'static str,
    /// What was refused, and what the values say.
    pub doc: &'static str,
}

/// The codes of the kernel's own reasons. Each is listed in
/// [`KERNEL_REASONS`].
pub mod codes {
    /// No entity has this id.
    pub const ENTITY_UNKNOWN: &str = "entity.unknown";
    /// The entity is not in the world.
    pub const ENTITY_STORED: &str = "entity.stored";
    /// Every entity slot has been used.
    pub const ENTITY_IDS_EXHAUSTED: &str = "entity.ids_exhausted";
    /// An entity cannot be put inside itself or its own contents.
    pub const RELOCATE_INSIDE_ITSELF: &str = "relocate.inside_itself";
    /// Things are nested too deeply.
    pub const RELOCATE_TOO_DEEP: &str = "relocate.too_deep";
    /// The cell's tract is not loaded.
    pub const WORLD_NOT_LOADED: &str = "world.not_loaded";
    /// The cell lies beyond an edge of its realm.
    pub const WORLD_OUTSIDE_REALM: &str = "world.outside_realm";
    /// No grid has this id.
    pub const WORLD_UNKNOWN_GRID: &str = "world.unknown_grid";
    /// No area has this id.
    pub const WORLD_UNKNOWN_AREA: &str = "world.unknown_area";
    /// No realm has this id.
    pub const WORLD_UNKNOWN_REALM: &str = "world.unknown_realm";
    /// The world refused for another reason.
    pub const WORLD_REFUSED: &str = "world.refused";
    /// No clock has this id.
    pub const CLOCK_UNKNOWN: &str = "clock.unknown";
    /// A clock's rate must be zero or more, and finite.
    pub const CLOCK_BAD_RATE: &str = "clock.bad_rate";
    /// The clock would be placed under itself.
    pub const CLOCK_LOOP: &str = "clock.loop";
    /// The timeline refused for another reason.
    pub const TIME_REFUSED: &str = "time.refused";
    /// A wake-up was due and nothing handles its domain.
    pub const ADVANCE_UNHANDLED_WAKE: &str = "advance.unhandled_wake";
    /// More wake-ups ran at one tick than the limit allows.
    pub const ADVANCE_TICK_LIMIT: &str = "advance.tick_limit";
    /// One pump delivered more events than the limit allows.
    pub const EVENTS_CASCADE_LIMIT: &str = "events.cascade_limit";
    /// Hooks were raised from inside hooks more deeply than the limit.
    pub const EVENTS_HOOK_DEPTH: &str = "events.hook_depth";
    /// No event type has this name.
    pub const EVENTS_UNKNOWN_TYPE: &str = "events.unknown_type";
    /// An event was raised without a subject or field its type declares,
    /// or with one of the wrong kind.
    pub const EVENTS_BAD_PAYLOAD: &str = "events.bad_payload";
    /// A handler tried to change a field its hook does not let it change.
    pub const EVENTS_FIELD_FIXED: &str = "events.field_fixed";
    /// A hook was cancelled by a handler that gave no reason of its own.
    pub const EVENTS_CANCELLED: &str = "events.cancelled";
    /// The entity is not waiting for a decision.
    pub const DECISION_NONE_PENDING: &str = "decision.none_pending";
    /// A replayed log no longer matches the simulation.
    pub const REPLAY_DIVERGED: &str = "replay.diverged";
}

/// The kernel's reasons, for the generated reference.
pub const KERNEL_REASONS: &[ReasonInfo] = &[
    ReasonInfo {
        code: codes::ENTITY_UNKNOWN,
        doc: "No entity has this id: it was never created, or it has been removed. Values: `entity`.",
    },
    ReasonInfo {
        code: codes::ENTITY_STORED,
        doc: "The entity is not in the world, so nothing can be scheduled for it. Values: `entity`.",
    },
    ReasonInfo {
        code: codes::ENTITY_IDS_EXHAUSTED,
        doc: "Every entity slot has been used.",
    },
    ReasonInfo {
        code: codes::RELOCATE_INSIDE_ITSELF,
        doc: "An entity cannot be put inside itself or inside something it contains. Values: `entity`, `container`.",
    },
    ReasonInfo {
        code: codes::RELOCATE_TOO_DEEP,
        doc: "Things are nested more deeply than the engine allows. Values: `entity`, `limit`.",
    },
    ReasonInfo {
        code: codes::WORLD_NOT_LOADED,
        doc: "The cell's tract is not loaded.",
    },
    ReasonInfo {
        code: codes::WORLD_OUTSIDE_REALM,
        doc: "The cell lies beyond an edge of its realm.",
    },
    ReasonInfo {
        code: codes::WORLD_UNKNOWN_GRID,
        doc: "No grid has this id.",
    },
    ReasonInfo {
        code: codes::WORLD_UNKNOWN_AREA,
        doc: "No area has this id.",
    },
    ReasonInfo {
        code: codes::WORLD_UNKNOWN_REALM,
        doc: "No realm has this id.",
    },
    ReasonInfo {
        code: codes::WORLD_REFUSED,
        doc: "The world refused the change; the message says why.",
    },
    ReasonInfo {
        code: codes::CLOCK_UNKNOWN,
        doc: "No clock has this id: it was never created, or it has been destroyed.",
    },
    ReasonInfo {
        code: codes::CLOCK_BAD_RATE,
        doc: "A clock's rate must be zero or more, and finite.",
    },
    ReasonInfo {
        code: codes::CLOCK_LOOP,
        doc: "The clock would be placed under itself or one of its descendants.",
    },
    ReasonInfo {
        code: codes::TIME_REFUSED,
        doc: "The timeline refused; the message says why.",
    },
    ReasonInfo {
        code: codes::ADVANCE_UNHANDLED_WAKE,
        doc: "A wake-up was due and nothing handles its domain. It was dropped. Values: `domain`, `id`, `arg`.",
    },
    ReasonInfo {
        code: codes::ADVANCE_TICK_LIMIT,
        doc: "More wake-ups ran at one tick than the limit allows; the advance yielded. Values: `tick`, `limit`.",
    },
    ReasonInfo {
        code: codes::EVENTS_CASCADE_LIMIT,
        doc: "One pump delivered more events than `rules.max_cascade`; the rest were dropped. Values: `limit`, `dropped`, `trace`.",
    },
    ReasonInfo {
        code: codes::EVENTS_HOOK_DEPTH,
        doc: "A hook was raised from inside too many others and was skipped. Values: `event`, `limit`.",
    },
    ReasonInfo {
        code: codes::EVENTS_UNKNOWN_TYPE,
        doc: "No event type has this name. Values: `event`.",
    },
    ReasonInfo {
        code: codes::EVENTS_BAD_PAYLOAD,
        doc: "An event was raised without a subject or field its type declares, with one it does not declare, or with one of the wrong kind. Values: `event`, `name`.",
    },
    ReasonInfo {
        code: codes::EVENTS_FIELD_FIXED,
        doc: "A handler tried to change a field that its hook does not declare mutable. Values: `event`, `field`.",
    },
    ReasonInfo {
        code: codes::EVENTS_CANCELLED,
        doc: "A hook was cancelled by a handler that gave no reason of its own. Values: `event`.",
    },
    ReasonInfo {
        code: codes::DECISION_NONE_PENDING,
        doc: "The entity is not waiting for a decision. Values: `entity`.",
    },
    ReasonInfo {
        code: codes::REPLAY_DIVERGED,
        doc: "A replayed input log no longer matches the simulation: an entry could not be reached or was refused. Values: `entry`.",
    },
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_kernel_code_is_listed_once_and_documented() {
        let mut seen: Vec<&str> = Vec::new();
        for info in KERNEL_REASONS {
            assert!(!info.doc.is_empty(), "{} has no documentation", info.code);
            assert!(!seen.contains(&info.code), "{} is listed twice", info.code);
            seen.push(info.code);
        }
    }

    #[test]
    fn a_reason_carries_its_values_and_reads_as_its_message() {
        let reason = Reason::new(codes::ENTITY_UNKNOWN, "there is no entity 3v1")
            .with("entity", "3v1")
            .with("tries", 2);
        assert_eq!(reason.value("tries"), Some(&Value::Int(2)));
        assert_eq!(reason.value("missing"), None);
        assert_eq!(reason.to_string(), "there is no entity 3v1");
    }

    #[test]
    fn world_and_time_refusals_get_codes_of_their_own() {
        let unknown = clash_time::ClockId::new(99);
        let time = TimeError::UnknownClock(unknown);
        assert_eq!(Reason::from_time(&time).code, codes::CLOCK_UNKNOWN);
        assert_eq!(
            Reason::from_world(&WorldError::Time(time)).code,
            codes::CLOCK_UNKNOWN
        );
        let grid = clash_world::GridId::new(4);
        assert_eq!(
            Reason::from_world(&WorldError::UnknownGrid(grid)).code,
            codes::WORLD_UNKNOWN_GRID
        );
    }
}
