//! Movement against the real simulation, in the test city (06 §11): a
//! walker steps, climbs, is refused with reasons, takes the time its
//! speed gives on the clock where it is, follows a path through doors,
//! falls when the floor goes, lies down, and is the same after a save
//! and a load.
//!
//! **Until the action runtime runs the actions of
//! `mods/core/data/movement.json`**, the tests drive them by hand: a
//! wake-up handler here does what a phase of `core:step` and
//! `core:go_to` does (waits the time the phase's own `time` expression
//! gives, read from the loaded definition, then calls what the phase's
//! effect calls and follows its `next`). The tests marked *re-point*
//! are to be driven by `Command::Act` with the real actions once they
//! run; the others test the functions the verbs call and stay as they
//! are.

mod common;

use std::sync::Arc;

use clash_core::{Dur, EntityId, Real, Tick};
use clash_data::{Defs, Length, Mass, Volume};
use clash_expr::Value;
use clash_sim::action::{ActionDef, PhaseTime};
use clash_sim::component::{Physical, Shape};
use clash_sim::movement::{
    self, Mover, MovementDef, PostureDef, Stepped, codes, dir_name, parse_dir,
};
use clash_sim::{
    AdvanceLimit, Bound, DebugCommand, Handler, Place, Reason, Sim, SimEvent, Subject,
};
use clash_time::{Timeline, Wake, WakeDomain, WakeId};
use clash_world::dims::STOREY_M;
use clash_world::{
    AreaShape, AreaSpec, BlockState, Cause, CellBox, CellPos, Cosmos, FloorState, Footprint,
    GridId, Layer, Location, PathLimits, StepKind, WorldDefs,
};

use common::{creature, debug, item, names};

/// The base grid of the city.
const GRID: GridId = GridId::new(0);

/// The seed of the city: what `cargo run -p clash-gen --example
/// city_ascii -- 1 0` prints. The cells named below are read off it.
const CITY_SEED: u64 = 1;

/// The first block's building fills cells 4 to 14 each way, with a
/// sidewalk round it at 3 and 15 and streets beyond.
///
/// The outer door in its east wall.
const EAST_DOOR: (i32, i32) = (14, 5);
/// The west door of the building across the street to the east, which
/// fills cells 20 to 30.
const FAR_WEST_DOOR: (i32, i32) = (20, 12);
/// The east door of that building.
const FAR_EAST_DOOR: (i32, i32) = (30, 11);
/// The stairs of the first building, on the ground storey: they lead to
/// the cell above.
const STAIRS: (i32, i32) = (5, 5);

/// Definitions these tests add: ways of moving and postures that the
/// engine has never heard of.
const FIXTURES: &str = r#"[
  { "type": "variable", "id": "legs", "scope": "entity", "value": "real", "default": 2 },
  { "type": "movement", "id": "stump", "surface": "standable", "speed": "0.7 * self.var.legs" },
  { "type": "movement", "id": "flit", "surface": "open", "speed": 4 },
  { "type": "movement", "id": "crawl", "surface": "standable", "speed": 1.0 },
  { "type": "posture", "id": "low", "height": 0.5, "speed": 0.5, "look": "stooped" },
  { "type": "posture", "id": "flat", "height": 0.25, "speed": 0.5, "movement": ["crawl"] }
]"#;

/// The world content of `mods/core`, its movement file and the
/// fixtures.
fn defs() -> Arc<Defs> {
    let path = common::mods_dir().join("core/data/movement.json");
    let core = std::fs::read_to_string(&path).unwrap_or_else(|error| panic!("{}: {error}", path.display()));
    common::try_defs_with(&[
        ("data/movement.json", core.as_str()),
        ("data/test/movement.json", FIXTURES),
    ])
    .unwrap_or_else(|error| panic!("the movement definitions do not load:\n{error}"))
}

fn cell(x: i32, y: i32, z: i32) -> Location {
    Location::new(GRID, CellPos::new(x, y, z))
}

/// A simulation over the test city with a walker in it, the tracts
/// round it loaded.
fn city(at: Location) -> (Sim, EntityId) {
    let defs = defs();
    let world = WorldDefs::from_defs(&defs).unwrap_or_else(|d| panic!("{}", d.render()));
    let mut timeline = Timeline::new();
    let mut cosmos = Cosmos::new(Arc::clone(&world));
    let def = world.realms().lookup("core:test_city").unwrap();
    cosmos.create_realm(&mut timeline, def, CITY_SEED).unwrap();
    let mut sim = Sim::from_parts(defs, cosmos, timeline, 3);
    sim.set_wake_handler(WakeDomain::Stepper, phase_ends);
    debug(
        &mut sim,
        DebugCommand::CreateEntity {
            spec: creature(),
            place: Place::Stored,
        },
    )
    .unwrap();
    let walker = sim.truth().entities().ids().next().unwrap();
    debug(&mut sim, DebugCommand::StartSession { entity: walker, at }).unwrap();
    assert_eq!(sim.truth().place(walker), Some(Place::at(at)));
    heard(&mut sim);
    (sim, walker)
}

fn loc_of(sim: &Sim, entity: EntityId) -> Location {
    sim.truth().place(entity).and_then(Place::loc).unwrap()
}

fn cause(entity: EntityId) -> Cause {
    Cause::DEBUG.by(entity)
}

fn step(sim: &mut Sim, entity: EntityId, dir: &str) -> Result<Stepped, Reason> {
    movement::step(sim, entity, parse_dir(dir).unwrap(), false, cause(entity))
}

/// Gives an entity a size and a mass of its own.
fn shape(sim: &mut Sim, entity: EntityId, mass: f64, height: f64) {
    let shape = Shape {
        mass: Mass::new(Real::of(mass)),
        volume: Volume::new(Real::of(mass / 1000.0)),
        height: Length::new(Real::of(height)),
        footprint: Footprint::SINGLE,
        cross_section: Real::of(0.5),
    };
    sim.set_shape(entity, shape, Cause::DEBUG).unwrap();
}

fn set_block(sim: &mut Sim, at: Location, state: &str) {
    let state = sim.truth().cosmos().defs().block_state(state, None).unwrap();
    debug(
        sim,
        DebugCommand::SetElement {
            loc: at,
            layer: Layer::Block,
            state: state.raw(),
        },
    )
    .unwrap();
}

fn open_door(sim: &mut Sim, (x, y): (i32, i32)) {
    set_block(sim, cell(x, y, 0), "door#open");
}

fn property(sim: &Sim, entity: EntityId, name: &str, args: &[Value]) -> Value {
    sim.property_named(Subject::Entity(entity), name, args)
}

/// The events of a name among some, in order.
fn of_name<'a>(events: &'a [SimEvent], name: &str) -> Vec<&'a SimEvent> {
    events.iter().filter(|event| event.name() == name).collect()
}

fn text_field(event: &SimEvent, name: &str) -> String {
    event
        .body
        .field(name)
        .and_then(|value| value.plain().and_then(|plain| plain.as_str().map(str::to_owned)))
        .unwrap_or_default()
}

fn real_field(event: &SimEvent, name: &str) -> Real {
    event.body.field(name).and_then(|value| value.as_real()).unwrap()
}

// ---- driving the actions by hand ------------------------------------------

/// The argument of the wake-up that ends a phase of `core:go_to`.
const GO_TO: u64 = 100;

/// How long a phase of an action of `mods/core` lasts for an actor: its
/// own `time` expression, read from the loaded definition, with the
/// action's locals bound.
fn phase_time(sim: &Sim, action: &str, phase: &str, actor: EntityId, locals: &[Value]) -> Dur {
    let registry = sim.defs().registry::<ActionDef>();
    let def = registry.get(registry.lookup(action).unwrap());
    let phase = def.phase(def.phase_index(phase).unwrap()).unwrap();
    let PhaseTime::For(time) = &phase.time else {
        panic!("`{action}` holds its phase");
    };
    let mut bound = Bound::new().subject(sim.subject_value(Subject::Entity(actor)));
    for local in locals {
        bound = bound.local(local.clone());
    }
    let seconds = time.eval_on(sim.defs(), &mut sim.host(bound)).unwrap();
    Dur::from_secs_real(seconds)
}

/// Starts `core:step` for an actor: its one phase waits the step's time
/// and then takes it.
fn act_step(sim: &mut Sim, actor: EntityId, dir: &str) -> Dur {
    let time = phase_time(sim, "core:step", "stride", actor, &[Value::text(dir)]);
    let kind = parse_dir(dir).unwrap();
    let at = StepKind::ALL.iter().position(|have| *have == kind).unwrap();
    sim.schedule_for(actor, time, Wake::new(WakeDomain::Stepper, actor.to_bits(), at as u64))
        .unwrap();
    time
}

/// The `plan` phase of `core:go_to`: plans, then enters `stride` if a
/// path is left.
fn act_go_to(sim: &mut Sim, actor: EntityId, to: Location) {
    let _ = movement::plan_path(sim, actor, to, PathLimits::DEFAULT, cause(actor));
    enter_stride(sim, actor, to);
}

/// Enters the `stride` phase of `core:go_to` if its `next` says so.
fn enter_stride(sim: &mut Sim, actor: EntityId, to: Location) {
    if property(sim, actor, "path_left", &[]).as_int().unwrap_or(0) <= 0 {
        return;
    }
    let to = sim.subject_value(Subject::Loc(to));
    let time = phase_time(sim, "core:go_to", "stride", actor, &[to]);
    sim.schedule_for(actor, time, Wake::new(WakeDomain::Stepper, actor.to_bits(), GO_TO))
        .unwrap();
}

/// The end of a phase: what its `on_exit` calls, and its `next`.
fn phase_ends(sim: &mut Sim, _id: WakeId, wake: Wake) {
    let actor = EntityId::from_bits(wake.id);
    if wake.arg != GO_TO {
        let kind = StepKind::ALL[wake.arg as usize];
        let _ = movement::step(sim, actor, kind, false, cause(actor));
        return;
    }
    let to = sim
        .components()
        .get::<Mover>(actor)
        .and_then(Mover::journey)
        .map(|journey| journey.to());
    let _ = movement::follow_path(sim, actor, cause(actor));
    let Some(to) = to else {
        return;
    };
    if property(sim, actor, "path_stale", &[]) == Value::Bool(true) {
        act_go_to(sim, actor, to);
    } else {
        enter_stride(sim, actor, to);
    }
}

/// What has happened since it was last asked: whatever is waiting is
/// delivered first, as the end of a command or a wake-up does.
fn heard(sim: &mut Sim) -> Vec<SimEvent> {
    sim.pump();
    sim.drain_events()
}

/// Runs until nothing is left to happen, and gives what happened.
fn run(sim: &mut Sim) -> Vec<SimEvent> {
    sim.advance(AdvanceLimit::UntilDecision);
    heard(sim)
}

// ---- steps -----------------------------------------------------------------

#[test]
fn a_walker_steps_each_way_at_the_cost_of_the_distance() {
    // The middle of a crossing: open street all round.
    let home = cell(17, 17, 0);
    let (mut sim, walker) = city(home);
    let speed = Real::of(1.4);
    assert_eq!(property(&sim, walker, "speed", &[]), Value::Real(speed));
    for kind in &StepKind::ALL[..8] {
        let dir = dir_name(*kind);
        let diagonal = dir.contains('_');
        let length = if diagonal { Real::of(2.0).sqrt() } else { Real::ONE };
        let time = movement::step_time(&sim, walker, *kind).unwrap();
        assert_eq!(time, Dur::from_secs_real(length / speed), "{dir}");
        assert_eq!(
            property(&sim, walker, "step_time", &[Value::text(dir)]),
            Value::Real(time.as_secs_real())
        );
        let stepped = step(&mut sim, walker, dir).unwrap();
        assert_eq!(stepped.length_m, length, "{dir}");
        assert_eq!(stepped.cost_m, length, "{dir}");
        assert_eq!(stepped.time, time);
        assert_eq!(stepped.from, home);
        assert_eq!(loc_of(&sim, walker), stepped.to);
        let by = kind.delta();
        assert_eq!(stepped.to, cell(17 + by.x, 17 + by.y, 0));
        // And back the way it came.
        sim.relocate(walker, Place::at(home), Cause::DEBUG).unwrap();
    }
    // On the street there is no way up or down.
    for dir in ["up", "down"] {
        let refused = step(&mut sim, walker, dir).unwrap_err();
        assert_eq!(refused.code, codes::TOO_STEEP, "{dir}");
    }
    let events = heard(&mut sim);
    assert_eq!(of_name(&events, "step_refused").len(), 2);
    assert!(names(&events).contains(&"entity_relocated"));
}

#[test]
fn a_walker_does_not_cut_a_corner() {
    // South of the building's south-east corner, on the sidewalk.
    let (mut sim, walker) = city(cell(14, 15, 0));
    assert_eq!(
        property(&sim, walker, "can_step", &[Value::text("north_east")]),
        Value::Bool(false)
    );
    let refused = step(&mut sim, walker, "north_east").unwrap_err();
    assert_eq!(refused.code, codes::CORNER);
    assert_eq!(loc_of(&sim, walker), cell(14, 15, 0));
    // Round the corner, a step at a time, is fine.
    step(&mut sim, walker, "east").unwrap();
    step(&mut sim, walker, "north").unwrap();
    assert_eq!(loc_of(&sim, walker), cell(15, 14, 0));
}

#[test]
fn a_walker_climbs_the_stairs_and_comes_down() {
    let (x, y) = STAIRS;
    let (mut sim, walker) = city(cell(x + 1, y, 0));
    // Beside the stairs there is no way up.
    assert_eq!(step(&mut sim, walker, "up").unwrap_err().code, codes::TOO_STEEP);
    step(&mut sim, walker, "west").unwrap();
    let up = step(&mut sim, walker, "up").unwrap();
    assert_eq!(up.to, cell(x, y, 1));
    assert_eq!(up.length_m, STOREY_M);
    assert_eq!(up.time, Dur::from_secs_real(STOREY_M / Real::of(1.4)));
    // Off the top of the stairs onto the floor of the storey, and back.
    step(&mut sim, walker, "south").unwrap();
    assert_eq!(loc_of(&sim, walker), cell(x, y + 1, 1));
    // From the floor there is no way down.
    assert_eq!(step(&mut sim, walker, "down").unwrap_err().code, codes::TOO_STEEP);
    step(&mut sim, walker, "north").unwrap();
    let down = step(&mut sim, walker, "down").unwrap();
    assert_eq!(down.to, cell(x, y, 0));
    assert!(!down.fell);
    // Nobody fell on the way.
    assert!(of_name(&heard(&mut sim), "fell").is_empty());
}

#[test]
fn a_step_is_refused_by_a_wall_a_closed_door_and_a_creature_each_with_its_reason() {
    // On the sidewalk east of the building, beside its wall.
    let (mut sim, walker) = city(cell(15, 9, 0));
    let wall = step(&mut sim, walker, "west").unwrap_err();
    assert_eq!(wall.code, codes::BLOCKED);
    assert!(wall.value("height").is_some());
    assert_eq!(
        property(&sim, walker, "step_fault", &[Value::text("west")]),
        Value::text("something is in the way")
    );

    // Beside the door, which is shut.
    let (door_x, door_y) = EAST_DOOR;
    sim.relocate(walker, Place::at(cell(door_x + 1, door_y, 0)), Cause::DEBUG)
        .unwrap();
    let door = step(&mut sim, walker, "west").unwrap_err();
    assert_eq!(door.code, codes::CLOSED);

    // Someone stands to the east.
    let other = common::spawn(&mut sim, creature(), Place::at(cell(door_x + 2, door_y, 0)));
    let taken = step(&mut sim, walker, "east").unwrap_err();
    assert_eq!(taken.code, codes::OCCUPIED);
    assert_eq!(taken.value("by"), Some(&clash_core::Value::from(other.to_string())));
    assert_eq!(loc_of(&sim, walker), cell(door_x + 1, door_y, 0));

    let events = heard(&mut sim);
    let refusals: Vec<String> = of_name(&events, "step_refused")
        .into_iter()
        .map(|event| text_field(event, "code"))
        .collect();
    assert_eq!(refusals, [codes::BLOCKED, codes::CLOSED, codes::OCCUPIED]);
    let last = of_name(&events, "step_refused")[2];
    assert_eq!(text_field(last, "dir"), "east");
    assert_eq!(last.subject("self"), Some(Subject::Entity(walker)));

    // Something that does not block movement is no obstacle: an item
    // lies there instead.
    sim.remove_entity(other, Cause::DEBUG).unwrap();
    common::spawn(&mut sim, item(), Place::at(cell(door_x + 2, door_y, 0)));
    step(&mut sim, walker, "east").unwrap();

    // Once the door is open the way in is too. The step is checked
    // when it is taken: shut again, it is refused again.
    sim.relocate(walker, Place::at(cell(door_x + 1, door_y, 0)), Cause::DEBUG)
        .unwrap();
    open_door(&mut sim, EAST_DOOR);
    assert_eq!(
        property(&sim, walker, "can_step", &[Value::text("west")]),
        Value::Bool(true)
    );
    set_block(&mut sim, cell(door_x, door_y, 0), "door#closed");
    assert_eq!(step(&mut sim, walker, "west").unwrap_err().code, codes::CLOSED);
    open_door(&mut sim, EAST_DOOR);
    step(&mut sim, walker, "west").unwrap();
    assert_eq!(loc_of(&sim, walker), cell(door_x, door_y, 0));
}

#[test]
fn a_creature_yields_only_where_a_rule_says_so() {
    let (mut sim, walker) = city(cell(17, 17, 0));
    let other = common::spawn(&mut sim, creature(), Place::at(cell(18, 17, 0)));
    assert_eq!(step(&mut sim, walker, "east").unwrap_err().code, codes::OCCUPIED);
    // A handler of the hook lets the walker by.
    sim.on(
        movement::PASSAGE_REQUESTED,
        Handler::new(|_, delivery| {
            delivery.set_field(movement::YIELDS_FIELD, true).unwrap();
        }),
    )
    .unwrap();
    let stepped = step(&mut sim, walker, "east").unwrap();
    assert_eq!(stepped.swapped, Some(other));
    assert_eq!(loc_of(&sim, walker), cell(18, 17, 0));
    assert_eq!(loc_of(&sim, other), cell(17, 17, 0));
    // Each still fills a cell of the world's index, and only one.
    for id in [walker, other] {
        let record = sim.truth().cosmos().occupant(id).unwrap();
        assert_eq!(record.loc, loc_of(&sim, id));
    }
}

// ---- time ------------------------------------------------------------------

/// *Re-point*: start `core:step` with `Command::Act`.
#[test]
fn a_step_takes_distance_over_speed_on_the_clock_where_the_walker_is() {
    let (mut sim, walker) = city(cell(17, 17, 0));
    let expected = Dur::from_secs_real(Real::ONE / Real::of(1.4));
    let started = sim.now();
    assert_eq!(act_step(&mut sim, walker, "east"), expected);
    // Nothing has moved yet: the step is taken as the phase ends.
    assert_eq!(loc_of(&sim, walker), cell(17, 17, 0));
    let events = run(&mut sim);
    assert_eq!(loc_of(&sim, walker), cell(18, 17, 0));
    assert_eq!(sim.now().since(started), expected);
    assert_eq!(of_name(&events, "entity_relocated").len(), 1);

    // Where the clock runs twice as fast the same step takes the same
    // time of the walker's own, and half as long as the world counts.
    let mut spec = AreaSpec::new(
        GRID,
        AreaShape::Box(CellBox::new(CellPos::new(16, 16, 0), CellPos::new(19, 19, 0))),
    );
    spec.clock_rate = Some(Real::of(2.0));
    sim.world(|ctx| ctx.create_area(spec, Cause::DEBUG)).unwrap();
    sim.pump();
    let started = sim.now();
    assert_eq!(act_step(&mut sim, walker, "west"), expected);
    run(&mut sim);
    assert_eq!(loc_of(&sim, walker), cell(17, 17, 0));
    let taken = sim.now().since(started);
    assert_eq!(taken, Dur::from_micros(expected.as_micros() / 2));
}

#[test]
fn a_load_slows_a_walker_by_its_speed_expression() {
    let (mut sim, walker) = city(cell(17, 17, 0));
    shape(&mut sim, walker, 70.0, 1.8);
    let light = movement::step_time(&sim, walker, StepKind::ALL[2]).unwrap();
    assert_eq!(light, Dur::from_secs_real(Real::ONE / Real::of(1.4)));
    // Sixty kilograms on its back: `walk` halves.
    let sack = common::spawn(&mut sim, item(), Place::Stored);
    shape(&mut sim, sack, 60.0, 0.5);
    sim.relocate(sack, Place::In(walker), Cause::DEBUG).unwrap();
    assert_eq!(property(&sim, walker, "speed", &[]), Value::Real(Real::of(0.7)));
    let laden = movement::step_time(&sim, walker, StepKind::ALL[2]).unwrap();
    assert_eq!(laden, Dur::from_secs_real(Real::ONE / Real::of(0.7)));
    let stepped = step(&mut sim, walker, "east").unwrap();
    assert_eq!(stepped.time, laden);
    // Put down, it is as quick as it was.
    sim.relocate(sack, Place::at(loc_of(&sim, walker)), Cause::DEBUG)
        .unwrap();
    assert_eq!(movement::step_time(&sim, walker, StepKind::ALL[2]).unwrap(), light);
}

#[test]
fn what_cannot_move_by_its_definitions_does_not_move() {
    let (mut sim, walker) = city(cell(17, 17, 0));
    let stump = sim.defs().lookup::<MovementDef>("core:stump").unwrap();
    movement::set_modes(&mut sim, walker, &[stump]).unwrap();
    // Two legs, by the variable's default.
    assert_eq!(property(&sim, walker, "speed", &[]), Value::Real(Real::of(1.4)));
    step(&mut sim, walker, "east").unwrap();
    // The definition's speed comes to nothing: nothing in the engine
    // knows why.
    sim.set_var(
        Subject::Entity(walker),
        "core:legs",
        Value::Real(Real::ZERO),
        Cause::DEBUG,
    )
    .unwrap();
    assert_eq!(property(&sim, walker, "speed", &[]), Value::Null);
    assert_eq!(step(&mut sim, walker, "west").unwrap_err().code, codes::CANNOT_MOVE);
    assert_eq!(
        movement::step_time(&sim, walker, StepKind::ALL[6]).unwrap_err().code,
        codes::CANNOT_MOVE
    );
    assert_eq!(
        movement::plan_path(&mut sim, walker, cell(17, 20, 0), PathLimits::DEFAULT, Cause::DEBUG)
            .unwrap_err()
            .code,
        codes::CANNOT_MOVE
    );
    // Something that is no creature and lists no way of moving has
    // none: the defaults are for creatures.
    let crate_ = common::spawn(&mut sim, item(), Place::at(cell(17, 18, 0)));
    assert_eq!(step(&mut sim, crate_, "south").unwrap_err().code, codes::CANNOT_MOVE);
    // It can still be put somewhere it can lie.
    movement::move_to(&mut sim, crate_, cell(17, 19, 0), Cause::DEBUG).unwrap();
    assert_eq!(loc_of(&sim, crate_), cell(17, 19, 0));
    let (door_x, door_y) = EAST_DOOR;
    let refused = movement::move_to(&mut sim, crate_, cell(door_x, door_y, 0), Cause::DEBUG);
    assert_eq!(refused.unwrap_err().code, codes::CLOSED);
}

// ---- paths -----------------------------------------------------------------

/// *Re-point*: start `core:go_to` with `Command::Act`.
#[test]
fn go_to_reaches_a_room_across_the_street_through_open_doors() {
    // In the north-east room of the first building.
    let start = cell(12, 6, 0);
    let goal = cell(22, 12, 0);
    let (mut sim, walker) = city(start);
    open_door(&mut sim, EAST_DOOR);
    open_door(&mut sim, FAR_WEST_DOOR);
    let cost = movement::path_cost(&sim, walker, goal).unwrap();
    assert_eq!(
        property(&sim, walker, "path_cost", &[sim.subject_value(Subject::Loc(goal))]),
        Value::Real(cost)
    );
    let started = sim.now();
    act_go_to(&mut sim, walker, goal);
    let planned = property(&sim, walker, "path_left", &[]).as_int().unwrap();
    assert!(planned > 10);
    let events = run(&mut sim);
    assert_eq!(loc_of(&sim, walker), goal);
    // One step for each cell of the plan, none refused, and the journey
    // is over.
    assert_eq!(of_name(&events, "entity_relocated").len() as i64, planned);
    assert!(of_name(&events, "step_refused").is_empty());
    assert!(of_name(&events, "path_failed").is_empty());
    assert_eq!(property(&sim, walker, "path_left", &[]), Value::Null);
    // It took the path's cost over the walker's speed, to the tick each
    // step rounds to.
    let seconds = sim.now().since(started).as_secs_real();
    let expected = cost / Real::of(1.4);
    assert!((seconds - expected).abs() < Real::of(1e-4), "{seconds} against {expected}");
    // It went by the two doors.
    let through: Vec<Location> = of_name(&events, "entity_relocated")
        .iter()
        .filter_map(|event| match event.subject("here") {
            Some(Subject::Loc(loc)) => Some(loc),
            _ => None,
        })
        .collect();
    assert!(through.contains(&cell(EAST_DOOR.0, EAST_DOOR.1, 0)));
    assert!(through.contains(&cell(FAR_WEST_DOOR.0, FAR_WEST_DOOR.1, 0)));
}

/// *Re-point*: start `core:go_to` with `Command::Act`.
#[test]
fn go_to_stops_with_a_reason_at_a_door_that_is_closed() {
    let start = cell(12, 6, 0);
    let goal = cell(22, 12, 0);
    let (mut sim, walker) = city(start);
    // Its own door is open; the doors across the street are shut. It
    // believes it will be able to open them, and plans through.
    open_door(&mut sim, EAST_DOOR);
    act_go_to(&mut sim, walker, goal);
    let events = run(&mut sim);
    // It walked to a door, found it shut, went round to the other, found
    // that shut, and gave up there with the reason.
    let refused = of_name(&events, "step_refused");
    assert_eq!(refused.len(), 2);
    assert!(refused.iter().all(|event| text_field(event, "code") == codes::CLOSED));
    let failed = of_name(&events, "path_failed");
    assert_eq!(failed.len(), 1);
    assert_eq!(text_field(failed[0], "code"), codes::NO_WAY);
    assert_eq!(failed[0].subject("here"), Some(Subject::Loc(goal)));
    let stands = loc_of(&sim, walker);
    let (east_x, east_y) = FAR_EAST_DOOR;
    assert_eq!(stands, cell(east_x + 1, east_y, 0));
    assert_eq!(property(&sim, walker, "path_left", &[]), Value::Null);
    assert_eq!(sim.truth().wakes_of(walker), []);

    // A door shut after the plan was made stops the step that would
    // pass it: the plan was made from what was believed, the step is
    // checked against what is.
    open_door(&mut sim, FAR_EAST_DOOR);
    let inside = cell(east_x - 2, east_y, 0);
    movement::plan_path(&mut sim, walker, inside, PathLimits::DEFAULT, cause(walker)).unwrap();
    set_block(&mut sim, cell(east_x, east_y, 0), "door#closed");
    let stopped = movement::follow_path(&mut sim, walker, cause(walker)).unwrap_err();
    assert_eq!(stopped.code, codes::CLOSED);
    assert_eq!(property(&sim, walker, "path_stale", &[]), Value::Bool(true));
    assert_eq!(
        movement::follow_path(&mut sim, walker, cause(walker)).unwrap_err().code,
        codes::OFF_PATH
    );
}

/// *Re-point*: start `core:go_to` with `Command::Act`.
#[test]
fn a_save_and_a_load_in_the_middle_of_a_path_change_nothing() {
    let start = cell(12, 6, 0);
    let goal = cell(22, 12, 0);
    let walk = |save_after: Option<u32>| {
        let (mut sim, walker) = city(start);
        shape(&mut sim, walker, 70.0, 1.8);
        open_door(&mut sim, EAST_DOOR);
        open_door(&mut sim, FAR_WEST_DOOR);
        act_go_to(&mut sim, walker, goal);
        if let Some(steps) = save_after {
            assert!(matches!(
                sim.advance(AdvanceLimit::Work(steps)),
                clash_sim::Advance::Yielded
            ));
            let before = sim.state_hash();
            let (mut loaded, unplaced) = common::save_and_load(&sim);
            assert_eq!(unplaced, []);
            eprintln!("{:#?}\n{:#?}", sim.hashes(), loaded.hashes());
            assert_eq!(loaded.state_hash(), before);
            loaded.set_wake_handler(WakeDomain::Stepper, phase_ends);
            sim = loaded;
            // In the middle of the path, with the rest of it kept.
            assert_ne!(loc_of(&sim, walker), goal);
            assert!(property(&sim, walker, "path_left", &[]).as_int().unwrap() > 0);
        }
        sim.advance(AdvanceLimit::UntilDecision);
        assert_eq!(loc_of(&sim, walker), goal);
        (sim.state_hash(), sim.now())
    };
    let straight = walk(None);
    assert_eq!(walk(Some(5)), straight);
    assert_eq!(walk(Some(11)), straight);
    assert!(straight.1 > Tick::ZERO);
}

// ---- falling ---------------------------------------------------------------

#[test]
fn a_walker_whose_floor_is_removed_falls_to_the_storey_below() {
    // On the floor of the first storey, over a room.
    let above = cell(10, 9, 1);
    let below = cell(10, 9, 0);
    let (mut sim, walker) = city(above);
    shape(&mut sim, walker, 70.0, 1.8);
    heard(&mut sim);
    debug(
        &mut sim,
        DebugCommand::SetElement {
            loc: above,
            layer: Layer::Floor,
            state: FloorState::NONE.raw(),
        },
    )
    .unwrap();
    assert_eq!(loc_of(&sim, walker), below);
    let events = heard(&mut sim);
    let order: Vec<&str> = names(&events)
        .into_iter()
        .filter(|name| {
            ["entity_unsupported", "fell", "entity_relocated", "landed"].contains(name)
        })
        .collect();
    assert_eq!(order, ["entity_unsupported", "fell", "entity_relocated", "landed"]);
    let fell = of_name(&events, "fell")[0];
    assert_eq!(fell.subject("here"), Some(Subject::Loc(above)));
    let landed = of_name(&events, "landed")[0];
    assert_eq!(landed.subject("self"), Some(Subject::Entity(walker)));
    assert_eq!(landed.subject("here"), Some(Subject::Loc(below)));
    let gravity = Real::of(9.81);
    assert_eq!(real_field(landed, "height"), STOREY_M);
    assert_eq!(real_field(landed, "energy"), Real::of(70.0) * gravity * STOREY_M);
    assert_eq!(real_field(landed, "speed"), (Real::of(2.0) * gravity * STOREY_M).sqrt());
}

#[test]
fn what_can_stay_in_the_air_does_not_fall_and_a_step_off_an_edge_needs_to_be_meant() {
    let above = cell(10, 9, 1);
    let (mut sim, walker) = city(above);
    let hole = cell(11, 9, 1);
    debug(
        &mut sim,
        DebugCommand::SetElement {
            loc: hole,
            layer: Layer::Floor,
            state: FloorState::NONE.raw(),
        },
    )
    .unwrap();
    // A walker will not step into the hole.
    assert_eq!(step(&mut sim, walker, "east").unwrap_err().code, codes::NO_FOOTING);
    assert_eq!(loc_of(&sim, walker), above);
    // A flier steps over it and stays; the floor under it can go too.
    let flit = sim.defs().lookup::<MovementDef>("core:flit").unwrap();
    let flier = common::spawn(&mut sim, creature(), Place::at(cell(12, 9, 1)));
    movement::set_modes(&mut sim, flier, &[flit]).unwrap();
    let over = step(&mut sim, flier, "west").unwrap();
    assert_eq!(over.to, hole);
    assert!(!over.fell);
    assert_eq!(over.time, Dur::from_secs_real(Real::ONE / Real::of(4.0)));
    // Down through the hole and up again, where a walker has no way.
    assert_eq!(step(&mut sim, flier, "down").unwrap().to, cell(11, 9, 0));
    assert_eq!(step(&mut sim, flier, "up").unwrap().to, hole);
    step(&mut sim, flier, "east").unwrap();
    heard(&mut sim);
    // Meaning to drop, the walker steps off, and falls.
    let dropped = movement::step(&mut sim, walker, StepKind::ALL[2], true, cause(walker)).unwrap();
    assert!(dropped.fell);
    assert_eq!(loc_of(&sim, walker), cell(11, 9, 0));
    let events = heard(&mut sim);
    assert_eq!(of_name(&events, "landed").len(), 1);
    // The flier's own floor is taken away: it stays where it is.
    debug(
        &mut sim,
        DebugCommand::SetElement {
            loc: cell(12, 9, 1),
            layer: Layer::Floor,
            state: FloorState::NONE.raw(),
        },
    )
    .unwrap();
    assert_eq!(loc_of(&sim, flier), cell(12, 9, 1));
    let events = heard(&mut sim);
    assert_eq!(of_name(&events, "entity_unsupported").len(), 1);
    assert!(of_name(&events, "fell").is_empty());
}

// ---- postures --------------------------------------------------------------

#[test]
fn a_posture_scales_height_and_speed_and_says_which_ways_of_moving_are_allowed() {
    let (mut sim, walker) = city(cell(17, 17, 0));
    shape(&mut sim, walker, 70.0, 1.8);
    let height = |sim: &Sim| sim.components().get::<Physical>(walker).unwrap().height().get();
    let low = sim.defs().lookup::<PostureDef>("core:low").unwrap();
    let flat = sim.defs().lookup::<PostureDef>("core:flat").unwrap();
    let prone = sim.defs().lookup::<PostureDef>("core:prone").unwrap();
    assert_eq!(property(&sim, walker, "posture", &[]), Value::Null);

    // Half the height, half the speed.
    assert_eq!(movement::set_posture(&mut sim, walker, Some(low), Cause::DEBUG), Ok(None));
    assert_eq!(height(&sim), Real::of(0.9));
    assert_eq!(property(&sim, walker, "height", &[]), Value::Real(Real::of(0.9)));
    assert_eq!(property(&sim, walker, "speed", &[]), Value::Real(Real::of(0.7)));
    assert_eq!(movement::posture_of(&sim, walker), Some(low));
    assert!(matches!(property(&sim, walker, "posture", &[]), Value::Id(_)));
    let events = heard(&mut sim);
    let changed = of_name(&events, "posture_changed");
    assert_eq!(changed.len(), 1);
    assert_eq!(text_field(changed[0], "from"), "");
    assert_eq!(text_field(changed[0], "to"), "core:low");
    assert_eq!(text_field(changed[0], "look"), "stooped");
    // The same posture again changes nothing and says nothing.
    assert_eq!(movement::set_posture(&mut sim, walker, Some(low), Cause::DEBUG), Ok(Some(low)));
    assert!(heard(&mut sim).is_empty());

    // The bundled prone posture: a fifth of the height.
    movement::set_posture(&mut sim, walker, Some(prone), Cause::DEBUG).unwrap();
    assert!((height(&sim) - Real::of(0.36)).abs() < Real::of(1e-12));

    // A posture that allows only crawling: a walker that only walks
    // cannot move in it, and one that also crawls crawls, at the
    // posture's share of that speed.
    movement::set_posture(&mut sim, walker, Some(flat), Cause::DEBUG).unwrap();
    assert_eq!(property(&sim, walker, "speed", &[]), Value::Null);
    assert_eq!(step(&mut sim, walker, "east").unwrap_err().code, codes::CANNOT_MOVE);
    let walk = sim.defs().lookup::<MovementDef>("core:walk").unwrap();
    let crawl = sim.defs().lookup::<MovementDef>("core:crawl").unwrap();
    movement::set_modes(&mut sim, walker, &[walk, crawl]).unwrap();
    assert_eq!(property(&sim, walker, "speed", &[]), Value::Real(Real::of(0.5)));
    assert_eq!(step(&mut sim, walker, "east").unwrap().mode, crawl);

    // Standing again, it is as tall and as fast as it was.
    movement::set_posture(&mut sim, walker, None, Cause::DEBUG).unwrap();
    assert!((height(&sim) - Real::of(1.8)).abs() < Real::of(1e-12));
    assert_eq!(property(&sim, walker, "speed", &[]), Value::Real(Real::of(1.4)));
    assert_eq!(step(&mut sim, walker, "west").unwrap().mode, walk);

    // A rule may forbid a change of posture: the hook cancels it.
    sim.on(
        movement::POSTURE_CHANGING,
        Handler::new(|_, delivery| {
            delivery
                .cancel(Reason::new("test.pinned", "it is held upright"))
                .unwrap();
        }),
    )
    .unwrap();
    let refused = movement::set_posture(&mut sim, walker, Some(low), Cause::DEBUG).unwrap_err();
    assert_eq!(refused.code, "test.pinned");
    assert_eq!(movement::posture_of(&sim, walker), None);
}

// ---- the verbs and the data ------------------------------------------------

#[test]
fn the_bundled_actions_load_and_call_verbs_the_engine_has() {
    let defs = defs();
    let registry = defs.registry::<ActionDef>();
    for id in ["core:step", "core:go_to", "core:set_posture"] {
        assert!(registry.lookup(id).is_some(), "{id}");
    }
    let exprs = defs.expr_registry();
    for verb in movement::verbs() {
        assert!(exprs.verb_id(verb.name).is_some(), "{}", verb.name);
    }
}

#[test]
fn the_verbs_do_what_the_functions_do() {
    use clash_sim::rule::{RunView, VerbCall};

    let (mut sim, walker) = city(cell(17, 17, 0));
    let run = |sim: &mut Sim, name: &str, args: &[Value]| {
        let verb = movement::verbs()
            .into_iter()
            .find(|verb| verb.name == name)
            .unwrap();
        let call = VerbCall {
            verb: sim.defs().expr_registry().verb_id(name).unwrap(),
            args,
            by: RunView::new(&[], &[]),
        };
        (verb.run)(sim, &call, Cause::DEBUG)
    };
    let who = sim.subject_value(Subject::Entity(walker));
    let result = run(&mut sim, "step", &[who.clone(), Value::text("east"), Value::Null]).unwrap();
    assert_eq!(loc_of(&sim, walker), cell(18, 17, 0));
    let result = result.as_map().unwrap().clone();
    assert_eq!(result.get("distance"), Some(&Value::Real(Real::ONE)));
    let refused = run(&mut sim, "step", &[who.clone(), Value::text("sideways"), Value::Null]);
    assert_eq!(refused.unwrap_err().code, codes::BAD_DIRECTION);
    let refused = run(&mut sim, "step", &[Value::Null, Value::text("east"), Value::Null]);
    assert_eq!(refused.unwrap_err().code, codes::BAD_ARGUMENT);

    let to = sim.subject_value(Subject::Loc(cell(17, 20, 0)));
    run(&mut sim, "plan_path", &[who.clone(), to.clone(), Value::Null]).unwrap();
    assert_eq!(property(&sim, walker, "path_left", &[]), Value::Int(3));
    assert_eq!(property(&sim, walker, "path_next", &[]), Value::text("south_west"));
    run(&mut sim, "follow_path", std::slice::from_ref(&who)).unwrap();
    assert_eq!(loc_of(&sim, walker), cell(17, 18, 0));
    run(&mut sim, "clear_path", std::slice::from_ref(&who)).unwrap();
    assert_eq!(property(&sim, walker, "path_left", &[]), Value::Null);
    let too_far = run(&mut sim, "plan_path", &[who.clone(), to.clone(), Value::Real(Real::ONE)]);
    assert_eq!(too_far.unwrap_err().code, codes::TOO_FAR);

    run(&mut sim, "move_to", &[who.clone(), to]).unwrap();
    assert_eq!(loc_of(&sim, walker), cell(17, 20, 0));
    run(&mut sim, "set_posture", &[who.clone(), Value::text("core:prone")]).unwrap();
    assert!(movement::posture_of(&sim, walker).is_some());
    run(&mut sim, "set_posture", &[who.clone(), Value::Null]).unwrap();
    assert_eq!(movement::posture_of(&sim, walker), None);
    let none = run(&mut sim, "set_posture", &[who.clone(), Value::text("core:sprawled")]);
    assert_eq!(none.unwrap_err().code, codes::POSTURE_UNKNOWN);
    let modes = Value::list([Value::text("core:flit")]);
    run(&mut sim, "set_movement", &[who, modes]).unwrap();
    assert_eq!(property(&sim, walker, "speed", &[]), Value::Real(Real::of(4.0)));
}

#[test]
fn an_unloaded_or_missing_mover_is_refused_with_a_reason() {
    let (mut sim, walker) = city(cell(17, 17, 0));
    let stored = common::spawn(&mut sim, creature(), Place::Stored);
    assert_eq!(step(&mut sim, stored, "east").unwrap_err().code, codes::NOWHERE);
    sim.remove_entity(stored, Cause::DEBUG).unwrap();
    assert_eq!(
        step(&mut sim, stored, "east").unwrap_err().code,
        clash_sim::codes::ENTITY_UNKNOWN
    );
    assert_eq!(
        movement::follow_path(&mut sim, walker, Cause::DEBUG).unwrap_err().code,
        codes::NO_PATH
    );
    assert_eq!(BlockState::NONE.raw(), FloorState::NONE.raw());
}
