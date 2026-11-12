//! What a mover believes about where it can go (01 R11): the one seam
//! between a mind's movement and the world.
//!
//! A path is planned from what the mover has perceived, and a step is
//! checked against truth when it is taken (06 §11). Everything a mind
//! decides by in this package reads the world through the first three
//! functions here and through nothing else:
//!
//! | Function | Answers | Read by |
//! |----------|---------|---------|
//! | [`believed_rules`] | What the mover takes itself to be able to do | Every other function here |
//! | [`believed_step`] | The geometry of a step from where it stands | `entity.can_step`, `entity.step_fault`, `entity.can_reach`, [`believed_step_time`] |
//! | [`believed_path`] | A way from where it stands to a cell | [`plan_path`](super::plan_path), `entity.path_cost` |
//! | [`believed_step_time`] | How long it expects a step to take | `entity.step_time`, and so the phases of the `step` and `go_to` actions |
//!
//! **Until perception is built (document 15) the first three read
//! truth.** [`believed_path`] now plans through the perception
//! package's believed rules (decision D-619): static geometry is common
//! knowledge, hidden elements count only once noted, unseen cells cost
//! more for a mover with map memory.
//! That is the stand-in, and it is here and nowhere else: the
//! perception package replaces the bodies of these functions with reads
//! of what the entity has seen and remembers, and nothing that calls
//! them changes. What a mover has learned by trying, the cells of its
//! journey that refused it, is already belief and is passed in by the
//! caller.

use clash_core::{Dur, EntityId};
use clash_world::{CellPos, Location, Path, PathFail, PathLimits, Step, StepKind};

use crate::Sim;

use super::rules::{MoverRules, mover_rules};

/// What a mover takes itself to be able to do: its own ways of moving
/// and their speeds, which it knows. A mind may read this of itself.
pub fn believed_rules(sim: &Sim, entity: EntityId) -> MoverRules {
    // Stand-in: a mover's own abilities are its to know, so truth about
    // the mover itself is what it believes.
    mover_rules(sim, entity)
}

/// The step a mover believes lies in a direction from where it stands:
/// the geometry of the cell it is in, the one it would enter and, for a
/// diagonal, the two it would pass. `None` if it is not standing in the
/// world or knows of no cell there.
pub fn believed_step(sim: &Sim, entity: EntityId, kind: StepKind) -> Option<Step> {
    // Stand-in: reads the world as it is.
    let loc = sim.truth().place(entity)?.loc()?;
    sim.truth().cosmos().neighbours(loc).ok()?.step(kind)
}

/// The way a mover believes leads from a cell to another: the cheapest
/// under its rules, planned through what it expects to be able to open
/// and round the cells it has learned it cannot enter.
pub fn believed_path(
    sim: &Sim,
    entity: EntityId,
    from: Location,
    to: Location,
    avoid: &[CellPos],
    limits: PathLimits,
) -> Result<Path, PathFail> {
    let rules = believed_rules(sim, entity)
        .through_openable()
        .avoiding(avoid);
    // Static geometry is common knowledge (15 §7.3): the search walks the
    // world's layout, judged by what the mover knows of it: something
    // hidden it has not noted does not open, and a mover with map memory
    // keeps to what it has seen where it can (decision D-619).
    let rules = crate::perception::believed_move_rules_with(sim, entity, rules).on_grid(from.grid);
    sim.truth().cosmos().find_path(from, to, &rules, limits)
}

/// How long a mover expects a step in a direction to take, in its own
/// time: what its rules make of the step it believes lies there
/// ([`MoverRules::judge`]). `None` if it believes it cannot make it.
///
/// An action's phase waits this long (`self.step_time(dir)`), and the
/// step is then checked against truth as it is taken. Until perception
/// is built it is what [`step_time`](super::step_time) gives.
pub fn believed_step_time(sim: &Sim, entity: EntityId, kind: StepKind) -> Option<Dur> {
    let step = believed_step(sim, entity, kind)?;
    let price = believed_rules(sim, entity).judge(&step).ok()?;
    Some(Dur::from_secs_real(price.seconds))
}
