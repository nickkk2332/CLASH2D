//! Taking a step (06 §11): [`step`], what it checks and in what order,
//! [`step_time`], and [`move_to`], which puts a mover somewhere it can
//! be.

use clash_core::Dir8;
use clash_core::{Dur, EntityId, Real};
use clash_world::{
    Cause, CellGeometry, Footing, Footprint, Location, OccupantFilter, OccupantFlags, Step,
    StepKind, distinct_ids,
};

use crate::Sim;
use crate::bus::{EventPayload, FieldValue};
use crate::entity::{Place, Presence};
use crate::event::{EventBody, Subject, Where};
use crate::reason::Reason;
use crate::sim::unknown_entity;

use super::defs::{MovementDef, MovementId};
use super::fall::{fall, in_flight};
use super::mover::Mover;
use super::rules::{MoverRules, StepFault, mover_rules};
use super::{PASSAGE_REQUESTED, STEP_REFUSED, YIELDS_FIELD, codes};

/// The ten directions as data writes them, in the order steps are
/// always listed (02 §3.3).
const DIRECTIONS: [(&str, StepKind); 10] = [
    ("north", StepKind::Level(Dir8::North)),
    ("north_east", StepKind::Level(Dir8::NorthEast)),
    ("east", StepKind::Level(Dir8::East)),
    ("south_east", StepKind::Level(Dir8::SouthEast)),
    ("south", StepKind::Level(Dir8::South)),
    ("south_west", StepKind::Level(Dir8::SouthWest)),
    ("west", StepKind::Level(Dir8::West)),
    ("north_west", StepKind::Level(Dir8::NorthWest)),
    ("up", StepKind::Up),
    ("down", StepKind::Down),
];

/// The step a direction names: `north`, `north_east`, `east`,
/// `south_east`, `south`, `south_west`, `west`, `north_west`, `up` or
/// `down`.
///
/// ```
/// use clash_sim::movement::{dir_name, parse_dir};
/// use clash_world::StepKind;
///
/// assert_eq!(parse_dir("up"), Some(StepKind::Up));
/// assert_eq!(parse_dir("upward"), None);
/// assert_eq!(dir_name(StepKind::Down), "down");
/// ```
pub fn parse_dir(dir: &str) -> Option<StepKind> {
    DIRECTIONS
        .iter()
        .find(|(name, _)| *name == dir)
        .map(|(_, kind)| *kind)
}

/// The name of a step's direction as data writes it.
pub fn dir_name(kind: StepKind) -> &'static str {
    DIRECTIONS
        .iter()
        .find(|(_, have)| *have == kind)
        .map_or("", |(name, _)| *name)
}

/// A step that was taken.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Stepped {
    /// Where the mover was.
    pub from: Location,
    /// Where it is.
    pub to: Location,
    /// Which way it went.
    pub kind: StepKind,
    /// How far, in metres between cell centres.
    pub length_m: Real,
    /// What the step cost, in metres of travel at the mover's best
    /// speed.
    pub cost_m: Real,
    /// How long the step takes, in the mover's own time. The verb takes
    /// no time itself: the action that called it waits this long.
    pub time: Dur,
    /// The way of moving that made the step.
    pub mode: MovementId,
    /// Who the mover changed places with, if it did.
    pub swapped: Option<EntityId>,
    /// Whether the mover stepped off into nothing and began to fall: the
    /// world carries it from there.
    pub fell: bool,
}

/// Where an entity stands in the world, with its height in the cell, or
/// the reason it does not.
pub(super) fn standing(sim: &Sim, entity: EntityId) -> Result<(Location, u16), Reason> {
    match sim.known(entity)?.place() {
        Place::At { loc, alt_mm } => Ok((loc, alt_mm)),
        Place::In(_) | Place::Stored => Err(Reason::new(
            codes::NOWHERE,
            format!("entity {entity} is not standing in the world"),
        )
        .with("entity", entity.to_string())),
    }
}

/// The reason a mover cannot move at all.
pub(super) fn cannot_move(entity: EntityId) -> Reason {
    Reason::new(
        codes::CANNOT_MOVE,
        format!("entity {entity} has no way of moving that works now"),
    )
    .with("entity", entity.to_string())
}

/// The reason a mover the world is carrying cannot step.
fn flying(entity: EntityId) -> Reason {
    Reason::new(codes::IN_FLIGHT, format!("entity {entity} is in the air"))
        .with("entity", entity.to_string())
}

/// The reason for a fault the geometry of a step gave.
pub(super) fn fault_reason(fault: StepFault, entity: EntityId, step: &Step) -> Reason {
    let height = step.to.obstacle_m;
    match fault {
        StepFault::CannotMove => Reason::new(
            codes::CANNOT_MOVE,
            format!("entity {entity} has no way of moving that takes it there"),
        )
        .with("entity", entity.to_string()),
        StepFault::Blocked | StepFault::Avoided => {
            Reason::new(codes::BLOCKED, "something is in the way").with("height", height)
        }
        StepFault::Closed => Reason::new(codes::CLOSED, "the way is closed").with("height", height),
        StepFault::Corner => Reason::new(
            codes::CORNER,
            "something is in the way of cutting the corner",
        ),
        StepFault::TooSteep => {
            let way = if step.kind == StepKind::Up {
                "up"
            } else {
                "down"
            };
            Reason::new(codes::TOO_STEEP, format!("there is no way {way} here"))
        }
        StepFault::Liquid => Reason::new(codes::LIQUID, "that is liquid, and it cannot swim"),
        StepFault::NoFooting => {
            Reason::new(codes::NO_FOOTING, "there is nothing to stand on there")
        }
    }
}

/// Reports a refused step or placement, if the entity exists.
fn report_refusal(sim: &mut Sim, entity: EntityId, dir: &str, reason: &Reason, cause: Cause) {
    if sim.known(entity).is_err() {
        return;
    }
    let payload = EventPayload::new(STEP_REFUSED)
        .subject("self", Subject::Entity(entity))
        .with("dir", dir)
        .with("code", reason.code.as_str())
        .with("message", reason.message.as_str());
    sim.emit(Where::Entity(entity), cause, EventBody::Custom(payload));
}

/// The geometry of the cells a footprint's lowest storey fills when its
/// corner is at `corner`, the corner first. `None` if one cannot be
/// read.
fn feet(sim: &Sim, footprint: Footprint, corner: Location) -> Option<Vec<CellGeometry>> {
    let cosmos = sim.truth().cosmos();
    Footprint::new(footprint.x, footprint.y, 1)
        .cells(corner.cell)
        .cells()
        .map(|cell| {
            cosmos
                .neighbours(Location::new(corner.grid, cell))
                .ok()
                .map(|around| around.here)
        })
        .collect()
}

/// Whether something holds a mover up where its corner is: one of its
/// ways of moving lets it be in a cell of its lowest storey, or, for
/// something that does not move by itself, a cell of it has footing.
pub(super) fn held_up(sim: &Sim, rules: &MoverRules, footprint: Footprint, at: Location) -> bool {
    let Some(cells) = feet(sim, footprint, at) else {
        // What cannot be read cannot be fallen through.
        return true;
    };
    cells.iter().any(|cell| {
        if rules.can_move() {
            rules.can_stay(cell)
        } else {
            cell.footing != Footing::None
        }
    })
}

/// Whether a mover wider or taller than a cell fits with its corner at
/// `corner`: no cell it would fill beyond the corner has anything in the
/// way that ordinary walking does not step over, and no floor lies
/// inside it above its lowest storey.
fn fits(sim: &Sim, footprint: Footprint, corner: Location) -> bool {
    if footprint == Footprint::SINGLE {
        return true;
    }
    let cosmos = sim.truth().cosmos();
    footprint.cells(corner.cell).cells().all(|cell| {
        if cell == corner.cell {
            return true;
        }
        let Ok(around) = cosmos.neighbours(Location::new(corner.grid, cell)) else {
            return false;
        };
        let inside_floor = cell.z > corner.cell.z && around.here.footing == Footing::Floor;
        !around.here.walk_blocked() && !inside_floor
    })
}

/// Who fills the cells a mover would enter and blocks movement on a
/// plane the mover is on, in order of cell and then id.
fn blockers(sim: &Sim, entity: EntityId, presence: &Presence, corner: Location) -> Vec<EntityId> {
    if !presence.flags.contains(OccupantFlags::BLOCKS_MOVE) {
        return Vec::new();
    }
    let filter = OccupantFilter::ANY
        .on(presence.plane)
        .with_flags(OccupantFlags::BLOCKS_MOVE)
        .except(entity);
    let cosmos = sim.truth().cosmos();
    let mut found = Vec::new();
    for cell in presence.footprint.cells(corner.cell).cells() {
        if let Ok(here) = cosmos.occupants_at(Location::new(corner.grid, cell), &filter) {
            found.extend(here);
        }
    }
    distinct_ids(&found)
}

/// The reason a cell is filled by someone who did not yield.
fn occupied(by: EntityId) -> Reason {
    Reason::new(codes::OCCUPIED, "someone is in the way").with("by", by.to_string())
}

/// Asks whoever fills a cell to let a mover by (the hook
/// `passage_requested`). True if a handler made it yield.
fn yields(sim: &mut Sim, mover: EntityId, other: EntityId, at: Location, cause: Cause) -> bool {
    let payload = EventPayload::new(PASSAGE_REQUESTED)
        .subject("self", Subject::Entity(mover))
        .subject("target", Subject::Entity(other))
        .subject("here", Subject::Loc(at))
        .with(YIELDS_FIELD, false);
    match sim.raise(Where::Entity(mover), cause, payload) {
        Ok(raised) => {
            !raised.is_cancelled() && raised.field(YIELDS_FIELD) == Some(&FieldValue::from(true))
        }
        Err(_) => false,
    }
}

/// Takes one step: moves an entity to the cell next to it in a
/// direction (06 §11). Returns what the step was, with the time it
/// takes, or the reason it was refused; a refusal also raises
/// `step_refused` and changes nothing.
///
/// Checked, in order, against the world as it is now:
///
/// 1. The mover stands in the world, the world is not carrying it as a
///    body, and it has a way of moving that works.
/// 2. There is a cell there: loaded, inside the realm.
/// 3. One of its ways of moving makes the step
///    ([`MoverRules::judge`]): nothing in the way, footing or whatever
///    else it needs, a connection between storeys, no corner cut.
/// 4. It fits, if it is wider or taller than a cell.
/// 5. Nobody who blocks movement fills the cell on a plane it shares. If
///    exactly one does and both fill one cell, the hook
///    `passage_requested` is raised, and if a handler makes that one
///    yield the two change places.
///
/// With `drop`, a step on the level into a cell with nothing to stand
/// on is made all the same, and the mover falls ([`fall`]): the world
/// carries it from there.
///
/// The mover is relocated through [`Sim::relocate`], which emits
/// `entity_relocated`. Nothing here takes time: the action that calls
/// the verb waits [`Stepped::time`], which is what [`step_time`] gave
/// it beforehand.
pub fn step(
    sim: &mut Sim,
    entity: EntityId,
    kind: StepKind,
    drop: bool,
    cause: Cause,
) -> Result<Stepped, Reason> {
    let done = try_step(sim, entity, kind, drop, cause);
    if let Err(reason) = &done {
        report_refusal(sim, entity, dir_name(kind), reason, cause);
    }
    done
}

fn try_step(
    sim: &mut Sim,
    entity: EntityId,
    kind: StepKind,
    drop: bool,
    cause: Cause,
) -> Result<Stepped, Reason> {
    let (from, alt_mm) = standing(sim, entity)?;
    if in_flight(sim, entity) {
        return Err(flying(entity));
    }
    let presence = *sim.known(entity)?.presence();
    let rules = mover_rules(sim, entity);
    if !rules.can_move() {
        return Err(cannot_move(entity));
    }
    let around = sim
        .truth()
        .cosmos()
        .neighbours(from)
        .map_err(|error| Reason::from_world(&error))?;
    let Some(step) = around.step(kind) else {
        return Err(
            Reason::new(codes::EDGE, "there is nothing there to step into")
                .with("dir", dir_name(kind)),
        );
    };
    let (seconds, cost_m, mode) = match rules.judge(&step) {
        Ok(price) => (price.seconds, price.cost_m, price.mode),
        Err(StepFault::NoFooting) if drop => {
            let mode = rules
                .modes()
                .first()
                .map_or(MovementId::new(0), |mode| mode.id);
            (step.length_m / rules.best_speed(), step.length_m, mode)
        }
        Err(fault) => return Err(fault_reason(fault, entity, &step)),
    };
    let to = Location::new(from.grid, step.to.cell);
    if !fits(sim, presence.footprint, to) {
        return Err(Reason::new(
            codes::NO_ROOM,
            "there is not room for it there",
        ));
    }
    let mut swapped = None;
    match blockers(sim, entity, &presence, to).as_slice() {
        [] => {}
        [other] => {
            let other = *other;
            let single = presence.footprint == Footprint::SINGLE
                && sim.known(other)?.presence().footprint == Footprint::SINGLE;
            if !single || !yields(sim, entity, other, to, cause) {
                return Err(occupied(other));
            }
            swapped = Some(other);
        }
        [first, ..] => return Err(occupied(*first)),
    }
    sim.relocate(entity, Place::At { loc: to, alt_mm }, cause)?;
    if let Some(other) = swapped {
        let other_alt = match sim.known(other)?.place() {
            Place::At { alt_mm, .. } => alt_mm,
            Place::In(_) | Place::Stored => 0,
        };
        let back = Place::At {
            loc: from,
            alt_mm: other_alt,
        };
        if let Err(reason) = sim.relocate(other, back, cause) {
            // The one who yielded cannot stand where the mover stood:
            // the mover goes back, and nothing has changed.
            sim.relocate(entity, Place::At { loc: from, alt_mm }, cause)?;
            return Err(reason);
        }
    }
    let fell = !held_up(sim, &rules, presence.footprint, to) && fall(sim, entity, cause).is_some();
    Ok(Stepped {
        from,
        to,
        kind,
        length_m: step.length_m,
        cost_m,
        time: Dur::from_secs_real(seconds),
        mode,
        swapped,
        fell,
    })
}

/// How long a step in a direction would take an entity, in its own
/// time: the step's length, with what its way of moving makes of rough
/// ground and of climbing, over that way's speed now (06 §11). Judged
/// against the world as it is, without asking who is in the cell, and
/// refused with the reason the step itself would be refused for.
///
/// An action's phase waits this long before its effect takes the step:
/// `"time": "self.step_time($dir) ?? 0"`. The duration is local time
/// (03 §3.4), so the same step takes half as long, as the world counts
/// it, where the mover's clock runs twice as fast.
pub fn step_time(sim: &Sim, entity: EntityId, kind: StepKind) -> Result<Dur, Reason> {
    let (from, _) = standing(sim, entity)?;
    let rules = mover_rules(sim, entity);
    if !rules.can_move() {
        return Err(cannot_move(entity));
    }
    let step = sim
        .truth()
        .cosmos()
        .neighbours(from)
        .map_err(|error| Reason::from_world(&error))?
        .step(kind)
        .ok_or_else(|| {
            Reason::new(codes::EDGE, "there is nothing there to step into")
                .with("dir", dir_name(kind))
        })?;
    rules
        .judge(&step)
        .map(|price| Dur::from_secs_real(price.seconds))
        .map_err(|fault| fault_reason(fault, entity, &step))
}

/// Puts an entity in a cell where it can be, from wherever it is,
/// without a step's adjacency and without time: for rules that place
/// someone (06 §11). Returns where it was.
///
/// Refused, raising `step_refused` and changing nothing, if the world
/// is carrying the entity as a body, the cell cannot be read, something
/// stands in it that the entity does not step
/// over (for something with no way of moving: that ordinary walking
/// does not), it does not fit, or someone who blocks movement fills it
/// on a plane it shares. Nobody is asked to yield. If nothing holds it
/// up there it falls.
pub fn move_to(
    sim: &mut Sim,
    entity: EntityId,
    to: Location,
    cause: Cause,
) -> Result<Place, Reason> {
    let done = try_move_to(sim, entity, to, cause);
    if let Err(reason) = &done {
        report_refusal(sim, entity, "", reason, cause);
    }
    done
}

fn try_move_to(
    sim: &mut Sim,
    entity: EntityId,
    to: Location,
    cause: Cause,
) -> Result<Place, Reason> {
    let known = sim.known(entity)?;
    let (presence, was) = (*known.presence(), known.place());
    if in_flight(sim, entity) {
        return Err(flying(entity));
    }
    let rules = mover_rules(sim, entity);
    let here = sim
        .truth()
        .cosmos()
        .neighbours(to)
        .map_err(|error| Reason::from_world(&error))?
        .here;
    let to = Location::new(to.grid, here.cell);
    let enterable = if rules.can_move() {
        rules.can_enter(&here)
    } else {
        !here.walk_blocked()
    };
    if !enterable {
        let (code, message) = if here.opens {
            (codes::CLOSED, "the way is closed")
        } else {
            (codes::BLOCKED, "something is in the way")
        };
        return Err(Reason::new(code, message).with("height", here.obstacle_m));
    }
    if !fits(sim, presence.footprint, to) {
        return Err(Reason::new(
            codes::NO_ROOM,
            "there is not room for it there",
        ));
    }
    if let Some(first) = blockers(sim, entity, &presence, to).first() {
        return Err(occupied(*first));
    }
    let alt_mm = match was {
        Place::At { alt_mm, .. } => alt_mm,
        Place::In(_) | Place::Stored => 0,
    };
    sim.relocate(entity, Place::At { loc: to, alt_mm }, cause)?;
    if !held_up(sim, &rules, presence.footprint, to) {
        let _ = fall(sim, entity, cause);
    }
    Ok(was)
}

/// Says which ways of moving an entity has, in the order they are
/// tried, replacing those it had (06 §11). An empty list makes it a
/// plain mover again, which moves by the definitions flagged `default`
/// if it is a creature. Whoever builds a creature from its template
/// calls this.
///
/// Refused for an unknown entity and for an id no loaded definition
/// has.
pub fn set_modes(sim: &mut Sim, entity: EntityId, modes: &[MovementId]) -> Result<(), Reason> {
    if !sim.entities.contains(entity) {
        return Err(unknown_entity(entity));
    }
    let known = |id: MovementId| {
        sim.defs()
            .try_registry::<MovementDef>()
            .is_some_and(|registry| registry.try_get(id).is_some())
    };
    if let Some(unknown) = modes.iter().find(|id| !known(**id)) {
        return Err(Reason::new(
            codes::MOVEMENT_UNKNOWN,
            format!("there is no way of moving {unknown}"),
        ));
    }
    let mover = sim
        .components_mut()
        .store_mut::<Mover>()
        .get_or_insert_with(entity, Mover::default);
    mover.modes = modes.to_vec();
    Ok(())
}
