//! Following a path (06 §11): [`plan_path`] plans one from what the
//! mover believes and keeps it in its [`Mover`]; [`follow_path`] takes
//! its next step, checked against truth like any other.
//!
//! The `go_to` action is a loop over the two: plan, step, step, and
//! plan again when a step is refused. A refused step teaches the mover
//! that it cannot enter that cell, and the next plan goes round it; when
//! no plan is left the journey ends with the reason.

use clash_core::{EntityId, Real};
use clash_world::{Cause, Location, PathFail, PathLimits, StepKind};

use crate::Sim;
use crate::bus::EventPayload;
use crate::event::{EventBody, Subject, Where};
use crate::reason::Reason;

use super::belief::{believed_path, believed_rules};
use super::mover::{Journey, Mover};
use super::step::{Stepped, cannot_move, standing, step};
use super::{PATH_FAILED, codes};

/// A path that was planned.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Planned {
    /// How many steps it takes. Zero for a mover that is already there.
    pub steps: usize,
    /// What it costs, in metres of travel at the mover's best speed.
    pub cost_m: Real,
}

/// The reason no path was found.
fn fail_reason(fail: &PathFail) -> Reason {
    match fail {
        PathFail::World(error) => Reason::from_world(error),
        PathFail::OtherGrid { .. } => Reason::new(codes::OTHER_GRID, fail.to_string()),
        PathFail::GoalBlocked(_) => Reason::new(codes::GOAL_BLOCKED, "it cannot be there"),
        PathFail::NoRoute { .. } => Reason::new(codes::NO_WAY, "there is no way there"),
        PathFail::TooFar { max_cost_m, .. } => {
            Reason::new(codes::TOO_FAR, "that is too far").with("max_cost", *max_cost_m)
        }
        // A search that gave up found no way either: how far it looked
        // is carried for whoever wants to say so.
        PathFail::BudgetSpent { max_nodes, .. } => {
            Reason::new(codes::NO_WAY, "there is no way there that it can find")
                .with("searched", i64::from(*max_nodes))
        }
    }
}

/// The path a mover believes leads to a cell, with the cells it has
/// learned to avoid on a journey there.
fn search(
    sim: &Sim,
    entity: EntityId,
    to: Location,
    limits: PathLimits,
) -> Result<(clash_world::Path, Vec<clash_world::CellPos>), Reason> {
    let (from, _) = standing(sim, entity)?;
    if !believed_rules(sim, entity).can_move() {
        return Err(cannot_move(entity));
    }
    // What it learned on the way to the same place still holds.
    let avoid = sim
        .components()
        .get::<Mover>(entity)
        .and_then(Mover::journey)
        .filter(|journey| sim.truth().cosmos().normalize(to).ok() == Some(journey.to))
        .map(|journey| journey.avoid.clone())
        .unwrap_or_default();
    let path =
        believed_path(sim, entity, from, to, &avoid, limits).map_err(|fail| fail_reason(&fail))?;
    Ok((path, avoid))
}

/// Plans a path for an entity from where it stands to a cell, from
/// what it believes (01 R11; [`belief`](super::belief)), and keeps it
/// as the path the entity is following. Returns how long the path is.
///
/// The path goes through what the mover expects to be able to open, at
/// a cost, and round the cells an earlier step of a journey to the same
/// place was refused at. A mover that is already there follows no path.
///
/// Refused with the reason there is no path: the mover cannot move or
/// is not standing in the world, it cannot be at the goal, no way
/// leads there, or none was found within `limits`. A refusal forgets
/// whatever path the entity was following and raises `path_failed`.
pub fn plan_path(
    sim: &mut Sim,
    entity: EntityId,
    to: Location,
    limits: PathLimits,
    cause: Cause,
) -> Result<Planned, Reason> {
    match search(sim, entity, to, limits) {
        Ok((path, avoid)) => {
            let planned = Planned {
                steps: path.steps(),
                cost_m: path.cost_m(),
            };
            let journey = (planned.steps > 0).then(|| Journey {
                to: path.goal().unwrap_or(to),
                cells: path.cells().iter().skip(1).copied().collect(),
                at: 0,
                avoid,
                stale: false,
            });
            sim.components_mut()
                .store_mut::<Mover>()
                .get_or_insert_with(entity, Mover::default)
                .journey = journey;
            Ok(planned)
        }
        Err(reason) => {
            if sim.known(entity).is_ok() {
                clear_path(sim, entity);
                let goal = sim.truth().cosmos().normalize(to).ok();
                let mut payload =
                    EventPayload::new(PATH_FAILED).subject("self", Subject::Entity(entity));
                if let Some(goal) = goal {
                    payload = payload.subject("here", Subject::Loc(goal));
                }
                let payload = payload
                    .with("code", reason.code.as_str())
                    .with("message", reason.message.as_str());
                sim.emit(Where::Entity(entity), cause, EventBody::Custom(payload));
            }
            Err(reason)
        }
    }
}

/// Takes the next step of the path an entity is following
/// ([`step`]), checked against the world as it is now.
///
/// Refused if the entity follows no path, if the path must be planned
/// again (`move.off_path`), and with the reason the step itself was
/// refused for. A refused step marks the path stale and teaches the
/// mover to avoid the cell it could not enter, so that
/// [`plan_path`] to the same place goes round it. Entering the last
/// cell ends the journey.
pub fn follow_path(sim: &mut Sim, entity: EntityId, cause: Cause) -> Result<Stepped, Reason> {
    let (from, _) = standing(sim, entity)?;
    let journey = sim
        .components()
        .get::<Mover>(entity)
        .and_then(Mover::journey);
    let Some(next) = journey.and_then(Journey::next) else {
        clear_path(sim, entity);
        return Err(Reason::new(
            codes::NO_PATH,
            format!("entity {entity} is following no path"),
        ));
    };
    let stale = journey.is_some_and(Journey::is_stale);
    let off_path = || Reason::new(codes::OFF_PATH, "its path must be planned again");
    if stale {
        return Err(off_path());
    }
    let delta = sim
        .truth()
        .cosmos()
        .delta(from, Location::new(from.grid, next));
    let kind = StepKind::ALL
        .into_iter()
        .find(|kind| Some(kind.delta()) == delta);
    let Some(kind) = kind else {
        mark_stale(sim, entity, None);
        return Err(off_path());
    };
    match step(sim, entity, kind, false, cause) {
        Ok(stepped) => {
            // A step that ended in a fall has already ended the journey.
            if let Some(mover) = sim.components_mut().get_mut::<Mover>(entity)
                && let Some(journey) = &mut mover.journey
            {
                journey.at += 1;
                if journey.next().is_none() {
                    mover.journey = None;
                }
            }
            Ok(stepped)
        }
        Err(reason) => {
            mark_stale(sim, entity, Some(next));
            Err(reason)
        }
    }
}

/// Marks an entity's path as one that must be planned again, and adds
/// a cell to those it has learned to avoid.
fn mark_stale(sim: &mut Sim, entity: EntityId, avoid: Option<clash_world::CellPos>) {
    if let Some(mover) = sim.components_mut().get_mut::<Mover>(entity)
        && let Some(journey) = &mut mover.journey
    {
        journey.stale = true;
        if let Some(cell) = avoid
            && !journey.avoid.contains(&cell)
        {
            journey.avoid.push(cell);
        }
    }
}

/// Forgets the path an entity is following, with what it learned on
/// the way. Returns whether it was following one.
pub fn clear_path(sim: &mut Sim, entity: EntityId) -> bool {
    sim.components_mut()
        .get_mut::<Mover>(entity)
        .and_then(|mover| mover.journey.take())
        .is_some()
}

/// What the way from where an entity stands to a cell costs, as the
/// entity believes it to be: metres of travel at its best speed
/// (09 §6.4). `None` if it believes there is no way.
pub fn path_cost(sim: &Sim, entity: EntityId, to: Location) -> Option<Real> {
    let (from, _) = standing(sim, entity).ok()?;
    believed_path(sim, entity, from, to, &[], PathLimits::DEFAULT)
        .ok()
        .map(|path| path.cost_m())
}
