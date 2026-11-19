//! What a character believes of the things it could interact with: the
//! seam between interactions and perception (01 R11; 05 §7.3).
//!
//! What a character is offered must come from what it has perceived.
//! Perception (document 15) is not built, so every function here reads
//! ground truth for now, as `movement::belief` does (decision D-574).
//! When perception lands, the bodies of these functions read the
//! character's beliefs instead, and nothing that calls them changes.
//! What is attempted is always tested against truth by
//! [`begin`](super::begin) and [`complete`](super::complete).

use clash_data::Cond;
use clash_expr::Value;
use clash_world::{ElementState, Layer, Location};

use super::{Reach, Target};
use crate::Sim;
use crate::host::Bound;

/// The element a character believes is on a layer of a cell, if any.
/// Truth until perception is built.
pub fn element(sim: &Sim, loc: Location, layer: Layer) -> Option<ElementState> {
    let state = sim
        .cosmos
        .grid(loc.grid)
        .ok()?
        .element(loc.cell, layer)
        .ok()?;
    (!state.is_none()).then_some(state)
}

/// Whether a character believes it reaches a target. Truth until
/// perception is built.
pub fn reaches(sim: &Sim, user: clash_core::EntityId, target: Target, reach: Reach) -> bool {
    super::reaches(sim, user, target, reach)
}

/// Whether a character believes an interaction's `if` holds, with its
/// subjects bound as `values`: evaluated against truth, reading only,
/// until perception is built.
pub fn holds(sim: &Sim, condition: &Cond, values: &[Value]) -> bool {
    let bound = values
        .iter()
        .fold(Bound::new(), |bound, value| bound.subject(value.clone()));
    let mut host = sim.host(bound);
    condition.holds_on(&sim.defs, &mut host)
}
