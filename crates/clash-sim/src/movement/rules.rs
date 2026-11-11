//! [`MoverRules`]: what one mover can do, as the world's search and the
//! step check ask it (02 §22 `MoveRules`; 06 §11).
//!
//! The rules are worked out from the entity each time they are wanted
//! ([`mover_rules`]): its ways of moving, each with the speed its
//! expression gives now, what its posture allows and makes of that
//! speed, and the channel slots its body has working. They are plain
//! values: nothing here reads the world. Whoever holds them hands them
//! the geometry of a step, from truth when a step is taken and from
//! what the mover believes when a path is planned
//! ([`belief`](super::belief)).

use clash_core::{EntityId, Real};
use clash_world::query::PROFILE_OPEN_COST_M;
use clash_world::{
    CellFlags, CellGeometry, CellPos, Connection, Footing, MoveRules, OccupantKind, Step, StepKind,
};
use smallvec::SmallVec;

use crate::Sim;
use crate::action::Slots;
use crate::body::Body;
use crate::event::Subject;
use crate::host::Bound;

use super::defs::{MovementDef, MovementId, PostureDef, Surface};
use super::mover::Mover;

/// One usable way of moving of one mover, with the numbers a step is
/// priced by.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct ModeRule {
    /// The definition.
    pub id: MovementId,
    /// Where it takes the mover.
    pub surface: Surface,
    /// Metres for each second of the mover's own time, now, posture
    /// included. Above zero.
    pub speed: Real,
    /// The tallest thing stepped over, in metres.
    pub step_up: Real,
    /// How many times longer a step into a rough cell takes.
    pub rough: Real,
    /// How many times longer a step between storeys takes.
    pub climb: Real,
}

impl ModeRule {
    /// Whether something in a cell stands in this mode's way.
    fn in_way(&self, cell: &CellGeometry) -> bool {
        self.surface != Surface::Solid && cell.obstacle_m > self.step_up
    }

    /// Whether the mode lets its mover be in a cell, taking what is in
    /// the way as `blocked` says.
    fn can_be_in(&self, cell: &CellGeometry, blocked: bool) -> bool {
        if blocked {
            return false;
        }
        match self.surface {
            Surface::Standable => !cell.liquid && cell.footing != Footing::None,
            Surface::Climbable => cell.connection.is_some() || cell.footing == Footing::Connection,
            Surface::Liquid => cell.liquid,
            Surface::Open | Surface::Solid => true,
        }
    }

    /// Whether the mode makes a step, the cell entered being one it can
    /// be in.
    fn makes(&self, step: &Step) -> bool {
        let on_foot = |connection: Option<Connection>| {
            matches!(connection, Some(Connection::Ramp | Connection::Stairs))
        };
        match (self.surface, step.kind) {
            (Surface::Solid, _) => true,
            (Surface::Standable | Surface::Open, StepKind::Level(_)) => true,
            (Surface::Liquid, StepKind::Level(_)) => true,
            (Surface::Climbable, StepKind::Level(_)) => false,
            (Surface::Standable, StepKind::Up) => on_foot(step.from.connection),
            (Surface::Standable, StepKind::Down) => on_foot(step.to.connection),
            (Surface::Climbable, StepKind::Up) => step.from.connection.is_some(),
            (Surface::Climbable, StepKind::Down) => step.to.connection.is_some(),
            (Surface::Liquid, StepKind::Up | StepKind::Down) => step.from.liquid && step.to.liquid,
            (Surface::Open, StepKind::Up) => !step.to.flags.contains(CellFlags::HAS_FLOOR),
            (Surface::Open, StepKind::Down) => {
                !step.from.flags.contains(CellFlags::HAS_FLOOR)
                    && !step.to.flags.contains(CellFlags::SOLID_TOP)
            }
        }
    }
}

/// Why a mover cannot make a step: what the geometry says, before
/// anyone else in the cell is asked.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum StepFault {
    /// The mover has no way of moving that works now.
    CannotMove,
    /// Something stands in the way that will not stand aside.
    Blocked,
    /// What stands in the way could stand aside, and does not: a closed
    /// door.
    Closed,
    /// The step is diagonal and passes something that is in the way.
    Corner,
    /// There is no way up or down here for this mover.
    TooSteep,
    /// The cell is liquid and the mover does not swim.
    Liquid,
    /// There is nothing to stand on there.
    NoFooting,
    /// The mover has found by trying that it cannot enter the cell.
    Avoided,
}

/// What a step costs a mover that can make it.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct StepPrice {
    /// The way of moving that makes the step soonest.
    pub mode: MovementId,
    /// Metres of travel at the mover's best speed: what the search
    /// adds up. Never below the step's own length.
    pub cost_m: Real,
    /// Seconds of the mover's own time the step takes.
    pub seconds: Real,
}

/// What one mover can do: the world's `MoveRules` for it (02 §22).
///
/// A step costs its length, times `rough` into a rough cell, times
/// `climb` between storeys, times the mover's best speed over the speed
/// of the way of moving that makes it: metres at the best speed, so
/// that the cheapest path is the quickest. The time a step takes is its
/// cost over the best speed.
#[derive(Clone, PartialEq, Debug)]
pub struct MoverRules {
    /// The ways of moving that work now, in the order they are tried.
    modes: SmallVec<[ModeRule; 4]>,
    /// The fastest of their speeds, or zero when there is none.
    best_speed: Real,
    /// Whether what could stand aside counts as passable, at
    /// `open_cost_m`: how a path is planned through a door that whoever
    /// follows it will open.
    through_openable: bool,
    /// What passing something that must first stand aside costs, as
    /// metres of travel.
    open_cost_m: Real,
    /// Cells the mover has learned it cannot enter.
    avoid: Vec<CellPos>,
}

impl MoverRules {
    /// Rules for a mover with these usable ways of moving.
    pub fn new(modes: impl IntoIterator<Item = ModeRule>) -> MoverRules {
        let modes: SmallVec<[ModeRule; 4]> = modes
            .into_iter()
            .filter(|mode| mode.speed.is_finite() && mode.speed > Real::ZERO)
            .collect();
        let best_speed = modes
            .iter()
            .map(|mode| mode.speed)
            .fold(Real::ZERO, Real::max);
        MoverRules {
            modes,
            best_speed,
            through_openable: false,
            open_cost_m: PROFILE_OPEN_COST_M,
            avoid: Vec::new(),
        }
    }

    /// These rules, counting what could stand aside (a closed door) as
    /// passable at a cost: for planning a path that whoever follows it
    /// will open its way along. A step is never taken under them.
    #[must_use]
    pub fn through_openable(mut self) -> MoverRules {
        self.through_openable = true;
        self
    }

    /// These rules with cells the mover will not enter.
    #[must_use]
    pub fn avoiding(mut self, cells: &[CellPos]) -> MoverRules {
        self.avoid.extend_from_slice(cells);
        self
    }

    /// The ways of moving that work now.
    pub fn modes(&self) -> &[ModeRule] {
        &self.modes
    }

    /// Whether the mover can move at all.
    pub fn can_move(&self) -> bool {
        !self.modes.is_empty()
    }

    /// The mover's best speed now, in metres for each second of its own
    /// time. Zero for one that cannot move.
    pub fn best_speed(&self) -> Real {
        self.best_speed
    }

    /// Whether a mode treats a cell as closed to it by what stands in
    /// the way.
    fn blocked(&self, mode: &ModeRule, cell: &CellGeometry) -> bool {
        mode.in_way(cell) && !(self.through_openable && cell.opens)
    }

    /// Whether one of the mover's ways of moving gets past what stands
    /// in a cell.
    pub fn can_enter(&self, cell: &CellGeometry) -> bool {
        self.modes.iter().any(|mode| !self.blocked(mode, cell))
    }

    /// Whether the mover could stay in a cell without falling: some way
    /// of moving of its lets it be there.
    pub fn can_stay(&self, cell: &CellGeometry) -> bool {
        self.modes
            .iter()
            .any(|mode| mode.can_be_in(cell, self.blocked(mode, cell)))
    }

    /// What a step costs the mover, or why it cannot make it. Only the
    /// geometry is judged: who else is in the cell is for the step
    /// itself.
    pub fn judge(&self, step: &Step) -> Result<StepPrice, StepFault> {
        if self.modes.is_empty() {
            return Err(StepFault::CannotMove);
        }
        if self.avoid.contains(&step.to.cell) {
            return Err(StepFault::Avoided);
        }
        let mut best: Option<StepPrice> = None;
        for mode in &self.modes {
            if !mode.can_be_in(&step.to, self.blocked(mode, &step.to)) || !mode.makes(step) {
                continue;
            }
            if mode.surface != Surface::Solid
                && matches!(step.kind, StepKind::Level(dir) if dir.is_diagonal())
            {
                // What could be opened is still in the way of cutting
                // past it.
                let clear = step
                    .sides
                    .iter()
                    .all(|side| side.as_ref().is_some_and(|side| !mode.in_way(side)));
                if !clear {
                    continue;
                }
            }
            let mut metres = step.length_m;
            if step.to.rough {
                metres *= mode.rough;
            }
            if matches!(step.kind, StepKind::Up | StepKind::Down) {
                metres *= mode.climb;
            }
            let mut seconds = metres / mode.speed;
            if mode.in_way(&step.to) {
                // Only under `through_openable`: the time it takes to
                // make it stand aside.
                seconds += self.open_cost_m / mode.speed;
            }
            if best.is_none_or(|have| seconds < have.seconds) {
                best = Some(StepPrice {
                    mode: mode.id,
                    cost_m: (seconds * self.best_speed).max(step.length_m),
                    seconds,
                });
            }
        }
        best.ok_or_else(|| self.fault(step))
    }

    /// Why no way of moving makes a step, read from the geometry in a
    /// fixed order.
    fn fault(&self, step: &Step) -> StepFault {
        let between_storeys = matches!(step.kind, StepKind::Up | StepKind::Down);
        if between_storeys && !self.modes.iter().any(|mode| mode.makes(step)) {
            return StepFault::TooSteep;
        }
        let all_blocked = self.modes.iter().all(|mode| mode.in_way(&step.to));
        if all_blocked {
            return if step.to.opens {
                StepFault::Closed
            } else {
                StepFault::Blocked
            };
        }
        match step.kind {
            StepKind::Level(dir) => {
                let has = |surface: Surface| self.modes.iter().any(|mode| mode.surface == surface);
                if step.to.liquid && !has(Surface::Liquid) {
                    StepFault::Liquid
                } else if step.to.footing == Footing::None && !step.to.liquid {
                    StepFault::NoFooting
                } else if dir.is_diagonal() {
                    StepFault::Corner
                } else {
                    StepFault::CannotMove
                }
            }
            StepKind::Up | StepKind::Down => StepFault::TooSteep,
        }
    }
}

impl MoveRules for MoverRules {
    fn can_occupy(&self, cell: &CellGeometry) -> bool {
        !self.avoid.contains(&cell.cell) && self.can_stay(cell)
    }

    fn step_cost(&self, step: &Step) -> Option<Real> {
        self.judge(step).ok().map(|price| price.cost_m)
    }
}

/// The posture an entity is in, if it is in one that still exists.
pub(super) fn posture_def(sim: &Sim, entity: EntityId) -> Option<&PostureDef> {
    let posture = sim.components().get::<Mover>(entity)?.posture?;
    sim.defs().try_registry::<PostureDef>()?.try_get(posture)
}

/// Whether a body has the channel slots a way of moving needs, with
/// what its posture occupies taken off.
fn has_slots(body: &Body, movement: &MovementDef, posture: Option<&PostureDef>) -> bool {
    movement.uses().needs().iter().all(|need| {
        let have = body.slots_of(need.channel);
        let held = match posture.and_then(|posture| posture.occupies().need(need.channel)) {
            Some(Slots::All) => have,
            Some(Slots::Count(count)) => count.min(have),
            None => 0,
        };
        let free = have - held;
        match need.slots {
            Slots::All => have > 0 && free == have,
            Slots::Count(count) => free >= count,
        }
    })
}

/// What an entity can do now, by the definitions and its own state
/// (06 §11): truth about the mover itself.
///
/// Its ways of moving are those its [`Mover`] lists, or, for a
/// creature that lists none, every definition flagged `default`, in
/// order of id; less those its posture does not allow, those whose channel slots
/// its body lacks, and those whose `speed` comes to nothing. A posture's
/// `speed` multiplies each. An entity that is left with none cannot
/// move, and nothing here says why in terms of legs: the data does.
pub fn mover_rules(sim: &Sim, entity: EntityId) -> MoverRules {
    let Some(registry) = sim.defs().try_registry::<MovementDef>() else {
        return MoverRules::new([]);
    };
    let mover = sim.components().get::<Mover>(entity);
    let own = mover.map_or(&[][..], Mover::modes);
    let creature = sim
        .entities
        .get(entity)
        .is_some_and(|known| known.presence().kind == OccupantKind::Creature);
    let ids: Vec<MovementId> = if !own.is_empty() {
        own.to_vec()
    } else if creature {
        registry
            .iter()
            .filter(|(_, def)| def.default)
            .map(|(id, _)| id)
            .collect()
    } else {
        Vec::new()
    };
    let posture = posture_def(sim, entity);
    let body = sim.components().get::<Body>(entity);
    let bound = Bound::new().subject(sim.subject_value(Subject::Entity(entity)));
    let mut host = sim.host(bound);
    let mut modes: SmallVec<[ModeRule; 4]> = SmallVec::new();
    for id in ids {
        let Some(def) = registry.try_get(id) else {
            continue;
        };
        if posture.is_some_and(|posture| !posture.allows(id)) {
            continue;
        }
        if body.is_some_and(|body| !has_slots(body, def, posture)) {
            continue;
        }
        let Some(speed) = def.speed.eval_on(sim.defs(), &mut host) else {
            continue;
        };
        let speed = speed * posture.map_or(Real::ONE, |posture| posture.speed);
        modes.push(ModeRule {
            id,
            surface: def.surface,
            speed,
            step_up: def.step_up.get(),
            rough: def.rough,
            climb: def.climb,
        });
    }
    MoverRules::new(modes)
}

#[cfg(test)]
mod tests {
    //! Step legality against a table of geometry cases (plan E10): each
    //! row is a mover, a step built by hand, and what the rules make of
    //! it.

    use clash_core::{Dir8, Real};
    use clash_world::dims::STOREY_M;
    use clash_world::{CellFlags, CellGeometry, CellPos, Connection, Footing, Step, StepKind};

    use super::{ModeRule, MoverRules, StepFault};
    use crate::movement::{MovementId, Surface};

    const WALK: u16 = 0;
    const CLIMB: u16 = 1;
    const SWIM: u16 = 2;
    const FLY: u16 = 3;
    const BURROW: u16 = 4;

    fn mode(id: u16, surface: Surface, speed: f64, climb: f64) -> ModeRule {
        ModeRule {
            id: MovementId::new(id),
            surface,
            speed: Real::of(speed),
            step_up: Real::of(0.5),
            rough: Real::of(2.0),
            climb: Real::of(climb),
        }
    }

    fn walker() -> ModeRule {
        mode(WALK, Surface::Standable, 1.0, 1.0)
    }

    fn rules(modes: &[ModeRule]) -> MoverRules {
        MoverRules::new(modes.iter().copied())
    }

    /// Open floor, and the cells made from it.
    fn floor() -> CellGeometry {
        CellGeometry {
            cell: CellPos::new(0, 0, 0),
            flags: CellFlags::HAS_FLOOR,
            footing: Footing::Floor,
            obstacle_m: Real::ZERO,
            connection: None,
            liquid: false,
            rough: false,
            opens: false,
        }
    }

    fn with(change: impl FnOnce(&mut CellGeometry)) -> CellGeometry {
        let mut cell = floor();
        change(&mut cell);
        cell
    }

    fn wall() -> CellGeometry {
        with(|cell| {
            cell.flags |= CellFlags::WALK_BLOCKED;
            cell.obstacle_m = STOREY_M;
        })
    }

    fn door() -> CellGeometry {
        with(|cell| {
            cell.flags |= CellFlags::WALK_BLOCKED;
            cell.obstacle_m = Real::of(2.0);
            cell.opens = true;
        })
    }

    fn kerb() -> CellGeometry {
        with(|cell| cell.obstacle_m = Real::of(0.3))
    }

    fn hole() -> CellGeometry {
        with(|cell| {
            cell.flags = CellFlags::NONE;
            cell.footing = Footing::None;
        })
    }

    fn water() -> CellGeometry {
        with(|cell| cell.liquid = true)
    }

    fn rough() -> CellGeometry {
        with(|cell| cell.rough = true)
    }

    fn holds(connection: Connection) -> CellGeometry {
        with(|cell| cell.connection = Some(connection))
    }

    /// The cell above a ramp, stairs or ladder, which its top holds up.
    fn landing() -> CellGeometry {
        with(|cell| {
            cell.flags = CellFlags::NONE;
            cell.footing = Footing::Connection;
        })
    }

    fn level(dir: Dir8, to: CellGeometry, sides: [CellGeometry; 2]) -> Step {
        let length_m = if dir.is_diagonal() {
            Real::of(2.0).sqrt()
        } else {
            Real::ONE
        };
        Step {
            kind: StepKind::Level(dir),
            from: floor(),
            to,
            sides: if dir.is_diagonal() {
                sides.map(Some)
            } else {
                [None, None]
            },
            length_m,
        }
    }

    fn east(to: CellGeometry) -> Step {
        level(Dir8::East, to, [floor(), floor()])
    }

    fn between(kind: StepKind, from: CellGeometry, to: CellGeometry) -> Step {
        Step {
            kind,
            from,
            to,
            sides: [None, None],
            length_m: STOREY_M,
        }
    }

    /// The way of moving that makes the step and how many seconds it
    /// takes, or why none does.
    type Outcome = Result<(u16, f64), StepFault>;

    #[test]
    fn step_legality_follows_the_table() {
        let climber = mode(CLIMB, Surface::Climbable, 0.5, 2.0);
        let swimmer = mode(SWIM, Surface::Liquid, 0.5, 1.0);
        let flier = mode(FLY, Surface::Open, 4.0, 1.0);
        let burrower = mode(BURROW, Surface::Solid, 0.1, 1.0);
        let sqrt2 = 2.0_f64.sqrt();
        let rows: Vec<(&str, MoverRules, Step, Outcome)> = vec![
            (
                "walk onto floor",
                rules(&[walker()]),
                east(floor()),
                Ok((WALK, 1.0)),
            ),
            (
                "walk on a diagonal",
                rules(&[walker()]),
                level(Dir8::NorthEast, floor(), [floor(), floor()]),
                Ok((WALK, sqrt2)),
            ),
            (
                "cut a corner past a wall",
                rules(&[walker()]),
                level(Dir8::NorthEast, floor(), [floor(), wall()]),
                Err(StepFault::Corner),
            ),
            (
                "cut a corner past a closed door",
                rules(&[walker()]),
                level(Dir8::NorthEast, floor(), [door(), floor()]),
                Err(StepFault::Corner),
            ),
            (
                "cut a corner past a kerb",
                rules(&[walker()]),
                level(Dir8::NorthEast, floor(), [kerb(), floor()]),
                Ok((WALK, sqrt2)),
            ),
            (
                "walk into a wall",
                rules(&[walker()]),
                east(wall()),
                Err(StepFault::Blocked),
            ),
            (
                "walk into a closed door",
                rules(&[walker()]),
                east(door()),
                Err(StepFault::Closed),
            ),
            (
                "step over a kerb",
                rules(&[walker()]),
                east(kerb()),
                Ok((WALK, 1.0)),
            ),
            (
                "walk into a hole",
                rules(&[walker()]),
                east(hole()),
                Err(StepFault::NoFooting),
            ),
            (
                "walk into water",
                rules(&[walker()]),
                east(water()),
                Err(StepFault::Liquid),
            ),
            (
                "walk onto rough ground",
                rules(&[walker()]),
                east(rough()),
                Ok((WALK, 2.0)),
            ),
            (
                "walk up stairs",
                rules(&[walker()]),
                between(StepKind::Up, holds(Connection::Stairs), landing()),
                Ok((WALK, 3.0)),
            ),
            (
                "walk down stairs",
                rules(&[walker()]),
                between(StepKind::Down, landing(), holds(Connection::Stairs)),
                Ok((WALK, 3.0)),
            ),
            (
                "walk up a ramp",
                rules(&[walker()]),
                between(StepKind::Up, holds(Connection::Ramp), landing()),
                Ok((WALK, 3.0)),
            ),
            (
                "walk up from a floor",
                rules(&[walker()]),
                between(StepKind::Up, floor(), floor()),
                Err(StepFault::TooSteep),
            ),
            (
                "walk up a ladder",
                rules(&[walker()]),
                between(StepKind::Up, holds(Connection::Ladder), landing()),
                Err(StepFault::TooSteep),
            ),
            (
                "climb a ladder",
                rules(&[walker(), climber]),
                between(StepKind::Up, holds(Connection::Ladder), landing()),
                Ok((CLIMB, 12.0)),
            ),
            (
                "climb on the level",
                rules(&[climber]),
                east(floor()),
                Err(StepFault::CannotMove),
            ),
            (
                "swim",
                rules(&[walker(), swimmer]),
                east(water()),
                Ok((SWIM, 2.0)),
            ),
            (
                "walk beside the water",
                rules(&[walker(), swimmer]),
                east(floor()),
                Ok((WALK, 1.0)),
            ),
            (
                "swim down",
                rules(&[swimmer]),
                between(StepKind::Down, water(), water()),
                Ok((SWIM, 6.0)),
            ),
            (
                "fly over a hole",
                rules(&[walker(), flier]),
                east(hole()),
                Ok((FLY, 0.25)),
            ),
            (
                "fly up through a floor",
                rules(&[flier]),
                between(StepKind::Up, hole(), floor()),
                Err(StepFault::TooSteep),
            ),
            (
                "fly up into the open",
                rules(&[flier]),
                between(StepKind::Up, hole(), hole()),
                Ok((FLY, 0.75)),
            ),
            (
                "fly down through one's floor",
                rules(&[flier]),
                between(StepKind::Down, floor(), hole()),
                Err(StepFault::TooSteep),
            ),
            (
                "fly into a wall",
                rules(&[flier]),
                east(wall()),
                Err(StepFault::Blocked),
            ),
            (
                "burrow into a wall",
                rules(&[burrower]),
                east(wall()),
                Ok((BURROW, 10.0)),
            ),
            (
                "burrow past a corner",
                rules(&[burrower]),
                level(Dir8::SouthWest, wall(), [wall(), wall()]),
                Ok((BURROW, sqrt2 * 10.0)),
            ),
            (
                "no way of moving",
                rules(&[]),
                east(floor()),
                Err(StepFault::CannotMove),
            ),
            (
                "a way of moving with no speed",
                rules(&[mode(WALK, Surface::Standable, 0.0, 1.0)]),
                east(floor()),
                Err(StepFault::CannotMove),
            ),
            (
                "plan through a closed door",
                rules(&[walker()]).through_openable(),
                east(door()),
                Ok((WALK, 3.0)),
            ),
            (
                "plan through a wall",
                rules(&[walker()]).through_openable(),
                east(wall()),
                Err(StepFault::Blocked),
            ),
            (
                "a cell learned to be closed",
                rules(&[walker()]).avoiding(&[floor().cell]),
                east(floor()),
                Err(StepFault::Avoided),
            ),
        ];
        for (name, rules, step, expected) in rows {
            let got = rules
                .judge(&step)
                .map(|price| (price.mode.index() as u16, price.seconds));
            match (got, expected) {
                (Ok((mode, seconds)), Ok((want_mode, want_seconds))) => {
                    assert_eq!(mode, want_mode, "{name}");
                    assert!(
                        (seconds - Real::of(want_seconds)).abs() < Real::of(1e-9),
                        "{name}: {seconds} seconds against {want_seconds}"
                    );
                }
                (got, expected) => assert_eq!(
                    got.map(|(mode, _)| mode),
                    expected.map(|(mode, _)| mode),
                    "{name}"
                ),
            }
        }
    }

    #[test]
    fn a_step_costs_metres_at_the_best_speed() {
        // A slow swimmer that walks: two seconds of swimming are two
        // metres at its walking pace.
        let both = rules(&[walker(), mode(SWIM, Surface::Liquid, 0.5, 1.0)]);
        assert_eq!(both.best_speed(), Real::ONE);
        assert_eq!(both.judge(&east(water())).unwrap().cost_m, Real::of(2.0));
        // A fast flier pays never less than the length of the step.
        let fast = rules(&[mode(FLY, Surface::Open, 4.0, 1.0)]);
        assert_eq!(fast.judge(&east(hole())).unwrap().cost_m, Real::ONE);
    }

    #[test]
    fn where_a_mover_can_stay() {
        let walker = rules(&[walker()]);
        let flier = rules(&[mode(FLY, Surface::Open, 4.0, 1.0)]);
        let climber = rules(&[mode(CLIMB, Surface::Climbable, 0.5, 1.0)]);
        assert!(walker.can_stay(&floor()) && walker.can_stay(&landing()));
        assert!(!walker.can_stay(&hole()) && !walker.can_stay(&water()));
        assert!(!walker.can_stay(&wall()) && !walker.can_stay(&door()));
        assert!(flier.can_stay(&hole()) && !flier.can_stay(&wall()));
        assert!(climber.can_stay(&holds(Connection::Ladder)) && climber.can_stay(&landing()));
        assert!(!climber.can_stay(&floor()));
        assert!(!rules(&[]).can_stay(&floor()));
    }
}
