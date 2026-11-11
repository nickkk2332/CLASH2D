//! The world's processes: scheduled activity that belongs to the world
//! and not to an entity (01 §5.1), and the one way a wake-up of the
//! world's reaches it.
//!
//! Every world process lives in one table, [`WorldActivity`], whose
//! wake-ups carry [`WakeDomain::World`]. The simulation kernel pops a
//! wake-up from the timeline and, if it is the world's, hands it to
//! [`WorldCtx::handle_wake`]:
//!
//! ```
//! use std::sync::Arc;
//! use clash_core::Tick;
//! use clash_time::{Timeline, WakeDomain};
//! use clash_world::{Cosmos, EventSink, RawWorldDefs, WorldCtx, WorldDefs};
//!
//! let defs = Arc::new(WorldDefs::build(RawWorldDefs::default()).unwrap());
//! let mut cosmos = Cosmos::new(defs);
//! let mut timeline = Timeline::new();
//! let mut events = EventSink::new();
//!
//! // Run everything that is due in the next second.
//! let until = Tick(1_000_000);
//! while let Some((_, wake)) = timeline.pop(until) {
//!     if wake.domain == WakeDomain::World {
//!         let mut ctx = WorldCtx::new(&mut cosmos, &mut timeline, &mut events);
//!         ctx.handle_wake(&wake).unwrap();
//!     }
//! }
//! timeline.advance_to(until);
//! ```
//!
//! A process is plain state. It is saved and hashed with the cosmos, so
//! a collapse in progress survives a save and resumes.

use clash_core::{StateHash, StateHasher};
use clash_time::{ClockId, Next, ProcessId, ProcessTable, Wake, WakeDomain};
use serde::{Deserialize, Serialize};

use crate::collapse::{CollapseState, FallingMass};
use crate::cosmos::Cosmos;
use crate::edit::WorldCtx;
use crate::error::WorldError;
use crate::field::{FieldState, FieldStep};

/// A stability check that is waiting out the settle delay on a clock
/// (02 §14.2). When it wakes, the checks asked for where that clock
/// applies are made together.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub struct SettleCheck {
    /// The clock the delay runs on.
    pub clock: ClockId,
}

clash_core::impl_state_hash!(SettleCheck { clock });

/// One process of the world. Later packages add their kinds: match with
/// a wildcard arm.
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum WorldProcess {
    /// Stability checks waiting to be made together (02 §14.2).
    Settle(SettleCheck),
    /// A mass of debris on its way down (02 §14.3).
    Fall(FallingMass),
    /// The fields of one clock and one step interval, stepped (02 §16).
    FieldStep(FieldStep),
}

impl StateHash for WorldProcess {
    fn state_hash(&self, hasher: &mut StateHasher) {
        match self {
            WorldProcess::Settle(check) => {
                hasher.write_u8(0);
                check.state_hash(hasher);
            }
            WorldProcess::Fall(mass) => {
                hasher.write_u8(1);
                mass.state_hash(hasher);
            }
            WorldProcess::FieldStep(step) => {
                hasher.write_u8(2);
                step.state_hash(hasher);
            }
        }
    }
}

/// What the world has under way: its processes, and the collapses they
/// belong to. State: it is hashed with the cosmos and must be saved with
/// it.
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub struct WorldActivity {
    /// Every process of the world, each with one wake-up on the
    /// timeline.
    pub(crate) processes: ProcessTable<WorldProcess>,
    /// The collapses in progress and the checks waiting.
    pub(crate) collapse: CollapseState,
    /// Where the fields are and which processes step them (02 §16).
    pub(crate) fields: FieldState,
}

impl WorldActivity {
    /// Nothing under way.
    pub fn new() -> WorldActivity {
        WorldActivity {
            processes: ProcessTable::new(WakeDomain::World),
            collapse: CollapseState::default(),
            fields: FieldState::new(),
        }
    }

    /// The world's processes.
    pub fn processes(&self) -> &ProcessTable<WorldProcess> {
        &self.processes
    }

    /// Whether nothing is under way: no process is waiting and no
    /// collapse is in progress.
    pub fn is_idle(&self) -> bool {
        self.processes.is_empty() && self.collapse.is_idle()
    }
}

impl Default for WorldActivity {
    fn default() -> WorldActivity {
        WorldActivity::new()
    }
}

impl StateHash for WorldActivity {
    fn state_hash(&self, hasher: &mut StateHasher) {
        self.processes.state_hash(hasher);
        self.collapse.state_hash(hasher);
        self.fields.state_hash(hasher);
    }
}

impl Cosmos {
    /// What the world has under way: its processes and the collapses in
    /// progress. Whoever saves the cosmos saves this with it, and the
    /// timeline that holds the processes' wake-ups.
    pub fn processes(&self) -> &WorldActivity {
        &self.activity
    }

    /// Puts back what the world had under way when it was saved. The
    /// timeline that was saved with it holds the wake-ups.
    pub fn restore_activity(&mut self, activity: WorldActivity) {
        self.activity = activity;
    }
}

impl WorldCtx<'_> {
    /// Handles a wake-up that the timeline has popped. This is the one
    /// way the world's scheduled activity runs: the kernel calls it for
    /// every wake-up whose domain is [`WakeDomain::World`], with the
    /// timeline's time already at the wake-up's tick.
    ///
    /// Returns whether the wake-up was for a process of the world. One
    /// that is not, or whose process has gone, does nothing.
    pub fn handle_wake(&mut self, wake: &Wake) -> Result<bool, WorldError> {
        let table = &mut self.cosmos.activity.processes;
        let Some(id) = table.process_of(wake) else {
            return Ok(false);
        };
        let Some(process) = table.take(self.timeline, id) else {
            return Ok(false);
        };
        let (process, next) = match process {
            WorldProcess::Settle(check) => {
                self.run_settle(check.clock);
                (WorldProcess::Settle(check), Next::Done)
            }
            WorldProcess::Fall(mass) => {
                let (mass, next) = self.run_fall(mass);
                (WorldProcess::Fall(mass), next)
            }
            WorldProcess::FieldStep(step) => {
                let next = self.run_field_step(step);
                (WorldProcess::FieldStep(step), next)
            }
        };
        if let (WorldProcess::Fall(mass), Next::Sleep(_)) = (&process, next) {
            // It goes on falling, on the clock of where it now is.
            let clock = self.cosmos.ambient(mass.at).map(|ambient| ambient.clock);
            if let Ok(clock) = clock {
                self.keep_clock(id, clock);
            }
        }
        self.cosmos
            .activity
            .processes
            .settle(self.timeline, id, process, next)?;
        Ok(true)
    }

    /// Moves a process to a clock, if it is not on it already.
    fn keep_clock(&mut self, id: ProcessId, clock: ClockId) {
        let table = &mut self.cosmos.activity.processes;
        if table.clock_of(id) != Some(clock) {
            // A clock that has gone leaves the process where it is.
            let _ = table.rehome(self.timeline, id, clock);
        }
    }

    /// Puts every process that is somewhere on the clock of where it
    /// is. Called when an area's clock comes, goes or moves, so that
    /// what is falling inside stopped time hangs, and resumes
    /// (01 §5.2).
    pub(crate) fn sync_process_clocks(&mut self) {
        if self.cosmos.activity.processes.is_empty() {
            return;
        }
        let placed: Vec<(ProcessId, ClockId)> = self
            .cosmos
            .activity
            .processes
            .iter()
            .filter_map(|(id, process)| match process {
                WorldProcess::Fall(mass) => Some((id, mass.at)),
                _ => None,
            })
            .filter_map(|(id, at)| Some((id, self.cosmos.ambient(at).ok()?.clock)))
            .collect();
        for (id, clock) in placed {
            self.keep_clock(id, clock);
        }
        // Fields follow the clock of where they are too (02 §16).
        self.resume_fields();
    }
}
