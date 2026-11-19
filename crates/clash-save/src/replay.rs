//! Replaying the commands between two saves (14 §8.1, as far as the
//! kernel's input log allows it now), and the state hash by subsystem.

use clash_core::{Hash128, Tick};
use clash_sim::{AdvanceLimit, InputLog, LogEntry, Reason, Sim};
use serde::Serialize;
use thiserror::Error;

/// The parts of a simulation's state hash, by the name the console's
/// `hash <subsystem>` and the replay report use, the total last.
pub fn subsystem_hashes(sim: &Sim) -> [(&'static str, Hash128); 9] {
    let hashes = sim.hashes();
    [
        ("world", hashes.cosmos),
        ("timeline", hashes.timeline),
        ("entities", hashes.entities),
        ("components", hashes.components),
        ("rules", hashes.rules),
        ("settings", hashes.settings),
        ("rng", hashes.rng),
        ("kernel", hashes.kernel),
        ("total", hashes.total),
    ]
}

/// Why the commands between two saves could not be replayed.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[non_exhaustive]
pub enum ReplayError {
    /// The later save's input log does not begin with the earlier
    /// one's: the log was taken between them, or they are of different
    /// games.
    #[error(
        "the later save's input log does not follow the earlier one's: the first holds {earlier} commands, and the second's first {earlier} are not the same"
    )]
    NotFollowing {
        /// How many commands the earlier save's log holds.
        earlier: usize,
    },
    /// The later save is not later.
    #[error("the save to replay to is at {to}, before the save to replay from, at {from}")]
    Backwards {
        /// The tick of the save replayed from.
        from: Tick,
        /// The tick of the save replayed to.
        to: Tick,
    },
    /// A command could not be replayed where it was accepted.
    #[error("{0}")]
    Diverged(Reason),
    /// The commands left over could not be handed to the kernel.
    #[error("the input log could not be rebuilt: {0}")]
    Log(String),
}

/// What replaying found.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Replayed {
    /// How many commands were replayed.
    pub commands: usize,
    /// The tick replayed to: that of the later save.
    pub tick: Tick,
    /// The later save's hash, by subsystem.
    pub expected: [(&'static str, Hash128); 9],
    /// The replayed state's hash, by subsystem.
    pub found: [(&'static str, Hash128); 9],
}

impl Replayed {
    /// Whether the replay ended in the state the later save holds.
    pub fn same(&self) -> bool {
        self.expected == self.found
    }

    /// The subsystems whose hash differs, the total last.
    pub fn differing(&self) -> Vec<&'static str> {
        self.expected
            .iter()
            .zip(&self.found)
            .filter(|(expected, found)| expected.1 != found.1)
            .map(|(expected, _)| expected.0)
            .collect()
    }
}

/// The input log with entries of its own: the kernel's has no way to be
/// built outside it, so the entries pass through its serialised form.
#[derive(Serialize)]
struct Entries<'a> {
    entries: &'a [LogEntry],
}

/// Replays onto `from`, the state of an earlier save, the commands the
/// state of a later save `to` accepted since, then runs it on to the
/// later save's tick, and compares the two states subsystem by
/// subsystem (14 §8.2).
///
/// The kernel's input log carries no state hashes at intervals yet
/// (14 §8.1), so a divergence is found at the end and named by
/// subsystem, not at the first tick where it arose. Changes the console
/// makes without a command (`damage`, `field`, `gen`) are not in the
/// log and are not replayed: a session that made them differs.
pub fn replay_between(from: &mut Sim, to: &Sim) -> Result<Replayed, ReplayError> {
    if to.now() < from.now() {
        return Err(ReplayError::Backwards {
            from: from.now(),
            to: to.now(),
        });
    }
    let earlier = from.input_log().entries();
    let later = to.input_log().entries();
    if later.len() < earlier.len() || later[..earlier.len()] != *earlier {
        return Err(ReplayError::NotFollowing {
            earlier: earlier.len(),
        });
    }
    let rest = &later[earlier.len()..];
    let json = serde_json::to_value(Entries { entries: rest })
        .map_err(|error| ReplayError::Log(error.to_string()))?;
    let log: InputLog =
        serde_json::from_value(json).map_err(|error| ReplayError::Log(error.to_string()))?;
    from.replay(&log).map_err(ReplayError::Diverged)?;
    // On to where the later save was made: the wake-ups it had handled,
    // then its tick.
    while from.steps() < to.steps() {
        let before = from.steps();
        let budget = u32::try_from(to.steps() - before).unwrap_or(u32::MAX);
        from.advance(AdvanceLimit::Work(budget));
        if from.steps() == before {
            break;
        }
    }
    if from.now() < to.now() {
        from.advance(AdvanceLimit::Until(to.now()));
    }
    Ok(Replayed {
        commands: rest.len(),
        tick: to.now(),
        expected: subsystem_hashes(to),
        found: subsystem_hashes(from),
    })
}
