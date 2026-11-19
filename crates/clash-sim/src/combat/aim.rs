//! Aim (07 §4.2): where on its target a strike lands, or whether it
//! misses.
//!
//! An attack's `aim` names a contest, the attacker's value and the
//! target's. One margin is drawn from the rule stream:
//!
//! | Margin | Result |
//! |--------|--------|
//! | At or above 0 | Lands on the part aimed at; with none, on one the body picks by exposure (06 §7.1) |
//! | Below 0, within the contest's `near` | Lands on a part beside the one aimed at, chosen by exposed area; with none aimed at, or none beside it, on one the body picks |
//! | Lower | Misses |
//!
//! The parts **beside** a part are those joined to it on the outside: the
//! part it is `on` and the parts `on` it, that are there and not inside
//! another. Which of them is chosen is one more draw from the rule
//! stream, by exposed area (06 §7.1).
//!
//! A strike that is `sure`, or whose attack has no `aim`, does not aim:
//! it lands on its mark ([`mark`]).
//!
//! What the target's value is, is content. The bundled `melee` contest
//! compares the two by their ratio, and the bundled `evasion` is zero
//! for a target that does not perceive its attacker (15 §6): in a ratio
//! nothing is below anything, so such a target is always hit where it
//! was aimed at.

use clash_core::{EntityId, Real};

use super::contest::{ContestDef, ContestId};
use crate::Sim;
use crate::body::{HitAt, PartKey, Relation};
use crate::host::RULE_STREAM;

/// Where a strike lands.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Aimed {
    /// On the target, at this: a part, or one the body picks by
    /// exposure. With the margin of the aim, where there was one.
    Lands(HitAt, Option<Real>),
    /// Nowhere: the margin was too low.
    Misses(Real),
}

/// Where a strike aimed at `part` of `target`, or at no part, lands, by
/// a contest between the attacker's value `by` and the target's
/// `against` (07 §4.2). A contest that is not loaded is a pure
/// comparison of the two with nothing beside.
pub fn aim(
    sim: &mut Sim,
    target: EntityId,
    part: Option<PartKey>,
    contest: ContestId,
    by: Real,
    against: Real,
) -> Aimed {
    let defs = std::sync::Arc::clone(sim.defs());
    let def = defs
        .try_registry::<ContestDef>()
        .and_then(|registry| registry.try_get(contest));
    let (margin, near) = {
        let rng = sim.rng.stream(RULE_STREAM);
        match def {
            Some(def) => (def.margin(by, against, rng), def.near),
            None => (by - against, Real::ZERO),
        }
    };
    if margin >= Real::ZERO {
        return Aimed::Lands(mark(sim, target, part), Some(margin));
    }
    if margin >= -near {
        let at = match part {
            Some(part) => beside(sim, target, part).map_or(HitAt::Any, HitAt::Part),
            None => HitAt::Any,
        };
        return Aimed::Lands(at, Some(margin));
    }
    Aimed::Misses(margin)
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

/// A part beside `part` on the outside of `target`'s body, chosen by
/// exposed area with one draw from the rule stream. `None` where there
/// is none.
pub fn beside(sim: &mut Sim, target: EntityId, part: PartKey) -> Option<PartKey> {
    let defs = sim.life.body_defs().ok()?;
    let candidates: Vec<(PartKey, Real)> = {
        let body = sim.components().body.get(target)?;
        let aimed = body.part(part)?;
        let up = (aimed.relation() == Relation::On)
            .then_some(aimed.parent())
            .flatten();
        let down = body
            .children(part)
            .filter(|child| child.relation() == Relation::On)
            .map(crate::body::PartInst::key);
        up.into_iter()
            .chain(down)
            .filter_map(|key| {
                let inst = body.part(key)?;
                (inst.is_present() && body.is_exposed(key))
                    .then(|| (key, body.exposed_area(&defs, inst)))
            })
            .filter(|(_, area)| *area > Real::ZERO)
            .collect()
    };
    let total = candidates
        .iter()
        .fold(Real::ZERO, |sum, (_, area)| sum + *area);
    if total <= Real::ZERO {
        return None;
    }
    let mut at = sim.rng.stream(RULE_STREAM).unit() * total;
    for (key, area) in &candidates {
        if at < *area {
            return Some(*key);
        }
        at -= *area;
    }
    candidates.last().map(|(key, _)| *key)
}
