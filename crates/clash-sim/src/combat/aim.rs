//! Aim (07 §4.2): where on its target a strike lands, or whether it
//! misses.
//!
//! Package C3 resolves an attack's `aim` by its contest. Until then a
//! strike lands on the part it is aimed at, or on a part the body picks
//! by exposure where none is (06 §7.1).

use clash_core::{EntityId, Real};

use crate::Sim;
use crate::body::{HitAt, PartKey};

/// Where a strike lands.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Aimed {
    /// On the target, at this: a part, or one the body picks by
    /// exposure. With the margin of the aim, where there was one.
    Lands(HitAt, Option<Real>),
    /// Nowhere: the margin was too low.
    Misses(Real),
}

/// Where a strike aimed at `part` of `target`, or at no part, lands.
pub fn aim(sim: &mut Sim, target: EntityId, part: Option<PartKey>) -> Aimed {
    Aimed::Lands(mark(sim, target, part), None)
}

/// The part aimed at, if the target still has it; else anywhere.
pub fn mark(sim: &Sim, target: EntityId, part: Option<PartKey>) -> HitAt {
    let present = |key: PartKey| {
        sim.components()
            .body
            .get(target)
            .and_then(|body| body.part(key))
            .is_some_and(crate::body::PartInst::is_present)
    };
    match part {
        Some(key) if present(key) => HitAt::Part(key),
        _ => HitAt::Any,
    }
}
