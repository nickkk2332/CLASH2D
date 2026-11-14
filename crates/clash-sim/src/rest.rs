//! Levels while the party rests (03 §11, 09 §9.5; decision D-932).
//!
//! While every character the player controls waits out a long wait (a
//! wait with at least `sim.rest_from` of its own time left) or lies
//! unconscious, and the setting `sim.rest_levels` is on, what is far
//! from the party is simulated with less detail, and nothing the party
//! perceives changes:
//!
//! - each member's own ticket (`Ticket::player`) shrinks to the tracts
//!   its senses reach, hot, with no warm margin and no unloading margin
//!   ([`Ticket::tight`]): the tracts beyond fall to cold, which is to
//!   say they unload, their residents stored as agents following their
//!   routines on the cold schedule (09 §10.6), and come back where
//!   their routines put them when the tracts load again;
//! - the warm stepper steps every `sim.rest_warm_step` instead of every
//!   `WARM_STEP` (03 §10): warm characters walk as far and think as
//!   their intervals allow, less often;
//! - no more than `sim.rest_max_hot_minds` (default none) are hot
//!   beside those the party sees or is involved with, who are hot
//!   whatever the cap (09 §9.4).
//!
//! The moment one member stops resting (its wait is over or broken off,
//! it acts, it wakes) the tickets are as they were at the next safe
//! point, and the tracts come back at that reconciliation; the stepper
//! keeps its widened step only until its next wake-up. Whether the
//! party rests is worked out from state at every safe point, never from
//! the wall clock, and the tickets are saved with the world, so a save
//! made in the middle of a long wait loads into the same levels.
//!
//! What a member perceives is unchanged: every tract within the range
//! of its senses stays loaded and hot, so whoever it could see or hear
//! is simulated as before (a warm character seen is promoted, D-891).

use clash_core::{Dur, EntityId, Real, Value};
use clash_world::dims::TRACT_XY;
use clash_world::{HOT_RADIUS, TicketAnchor, TicketId, TicketReason};

use crate::Sim;
use crate::mind::VitalState;

/// The setting that switches levels while the party rests on or off.
/// Default off: the bundled game scenarios switch it on.
pub const REST_LEVELS_SETTING: &str = "sim.rest_levels";

/// The setting that says how long a wait must still have to run, in
/// seconds of the waiting character's own time, for its character to
/// count as resting. Default [`DEFAULT_REST_FROM`].
pub const REST_FROM_SETTING: &str = "sim.rest_from";

/// The default of [`REST_FROM_SETTING`]: five minutes, so that a minute's
/// wait is waited at full detail.
pub const DEFAULT_REST_FROM: Dur = Dur::from_secs(300);

/// The setting that says how often the warm stepper steps while the
/// party rests, in seconds. Default [`DEFAULT_REST_WARM_STEP`].
pub const REST_WARM_STEP_SETTING: &str = "sim.rest_warm_step";

/// The default of [`REST_WARM_STEP_SETTING`]: the ordinary warm step.
pub const DEFAULT_REST_WARM_STEP: Dur = crate::ai::warm::WARM_STEP;

/// The setting that caps the characters kept hot while the party rests,
/// beside `sim.max_hot_minds`: those the party sees and those involved
/// with it are hot whatever the cap (09 §9.4). Default
/// [`DEFAULT_REST_MAX_HOT`].
pub const REST_MAX_HOT_SETTING: &str = "sim.rest_max_hot_minds";

/// The default of [`REST_MAX_HOT_SETTING`]: none beyond those the party
/// sees or is involved with.
pub const DEFAULT_REST_MAX_HOT: i64 = 0;

/// The longest warm step a setting may ask for: a minute.
pub const MAX_REST_WARM_STEP: Dur = Dur::from_secs(60);

/// The edge of a tract, in metres: cells are a metre across (02 §2).
const TRACT_M: i64 = TRACT_XY as i64;

/// The settings of resting levels, with their defaults, for the
/// reference.
pub fn settings() -> Vec<crate::registry::SettingInfo> {
    vec![
        crate::registry::SettingInfo {
            name: REST_LEVELS_SETTING,
            default: clash_expr::Value::Bool(false),
            doc: "Whether, while every character the player controls rests (waits a long wait or lies unconscious), the tracts beyond the reach of its senses fall to cold and the warm step widens (03 §11, D-932). Default false; the bundled game scenarios set it.",
        },
        crate::registry::SettingInfo {
            name: REST_FROM_SETTING,
            default: clash_expr::Value::Real(DEFAULT_REST_FROM.as_secs_real()),
            doc: "Seconds of its own time a character's wait must still have to run for it to count as resting (03 §11). Default 300.",
        },
        crate::registry::SettingInfo {
            name: REST_MAX_HOT_SETTING,
            default: clash_expr::Value::Int(DEFAULT_REST_MAX_HOT),
            doc: "How many characters with minds are hot while the party rests, beside those it sees or is involved with, who are hot whatever the cap (09 §9.4, D-932); never more than `sim.max_hot_minds`. Default 0.",
        },
        crate::registry::SettingInfo {
            name: REST_WARM_STEP_SETTING,
            default: clash_expr::Value::Real(DEFAULT_REST_WARM_STEP.as_secs_real()),
            doc: "Seconds between the warm stepper's steps while the party rests (03 §10, §11), from 1 to 60. Default 1.",
        },
    ]
}

/// A setting of seconds as a length of time.
fn seconds(sim: &Sim, key: &str) -> Option<Dur> {
    let secs = match sim.settings().get(key)? {
        Value::Real(value) => *value,
        Value::Int(value) => Real::from_i64(*value),
        _ => return None,
    };
    (secs.is_finite() && secs >= Real::ZERO).then(|| Dur::from_secs_real(secs))
}

/// Whether levels while resting are switched on.
pub fn enabled(sim: &Sim) -> bool {
    matches!(
        sim.settings().get(REST_LEVELS_SETTING),
        Some(Value::Bool(true))
    )
}

/// The warm step while the party rests.
pub fn rest_warm_step(sim: &Sim) -> Dur {
    seconds(sim, REST_WARM_STEP_SETTING)
        .unwrap_or(DEFAULT_REST_WARM_STEP)
        .clamp(crate::ai::warm::WARM_STEP, MAX_REST_WARM_STEP)
}

/// The cap on hot characters while the party rests.
pub fn rest_max_hot(sim: &Sim) -> usize {
    usize::try_from(
        sim.setting_int(REST_MAX_HOT_SETTING, DEFAULT_REST_MAX_HOT)
            .max(0),
    )
    .unwrap_or(usize::MAX)
}

/// The warm stepper's step now: widened while the party rests.
pub fn warm_step(sim: &Sim) -> Dur {
    if is_resting(sim) {
        rest_warm_step(sim)
    } else {
        crate::ai::warm::WARM_STEP
    }
}

/// Whether levels are lowered now: some character's own ticket is
/// tight. Read from the tickets, which are saved.
pub fn is_resting(sim: &Sim) -> bool {
    sim.cosmos
        .tickets()
        .iter()
        .any(|(_, ticket)| ticket.reason == TicketReason::Player && ticket.tight)
}

/// Whether a member of the party rests: it waits a wait with at least
/// `sim.rest_from` left, or its mind is stopped.
pub fn member_rests(sim: &Sim, member: EntityId) -> bool {
    if matches!(
        sim.vital(member),
        Some(VitalState::Unconscious | VitalState::Dead)
    ) {
        return true;
    }
    let from = seconds(sim, REST_FROM_SETTING).unwrap_or(DEFAULT_REST_FROM);
    sim.wait_left(member).is_some_and(|left| left >= from)
}

/// How many tracts round its own a member's senses reach: the longest
/// range of its senses, in whole tracts, rounded up.
fn sense_reach(sim: &Sim, member: EntityId) -> u8 {
    let longest = sim
        .components
        .senses
        .get(member)
        .map(|senses| {
            senses
                .slots()
                .iter()
                .filter_map(|slot| sim.perception_defs.sense(slot.sense()))
                .map(|def| def.range.get())
                .fold(Real::ZERO, Real::max)
        })
        .unwrap_or(Real::ZERO);
    let metres = longest.ceil().to_i64().max(0);
    let tracts = (metres + TRACT_M - 1) / TRACT_M;
    u8::try_from(tracts).unwrap_or(HOT_RADIUS).min(HOT_RADIUS)
}

impl Sim {
    /// Brings the party's own tickets into line with whether it rests
    /// (see the [module](self)): called at every safe point, before the
    /// world reconciles. Costs a look at the tickets while nothing
    /// changes.
    pub(crate) fn settle_rest(&mut self) {
        let on = enabled(self);
        let mut players: Vec<(TicketId, EntityId, u8, bool)> = Vec::new();
        for (id, ticket) in self.cosmos.tickets().iter() {
            if ticket.reason != TicketReason::Player {
                continue;
            }
            if let TicketAnchor::Entity(entity) = ticket.anchor {
                players.push((id, entity, ticket.radius, ticket.tight));
            }
        }
        if players.is_empty() || (!on && players.iter().all(|(_, _, _, tight)| !tight)) {
            return;
        }
        let resting = on
            && players.iter().all(|(_, entity, _, _)| {
                self.entities.location(*entity).is_some() && member_rests(self, *entity)
            });
        for (id, entity, radius, tight) in players {
            let (want_radius, want_tight) = if resting {
                (sense_reach(self, entity), true)
            } else {
                (HOT_RADIUS, false)
            };
            if (radius, tight) != (want_radius, want_tight) {
                let _ = self.cosmos.set_ticket_reach(id, want_radius, want_tight);
            }
        }
    }
}
