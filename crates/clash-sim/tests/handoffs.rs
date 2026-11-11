//! Tests for what the kernel joins together: components held by the
//! simulation (06 §3), the world's processes running under `advance`
//! (02 §14), tickets and the loading and unloading of tracts with the
//! entities in them (02 §9.4, §9.5, §10.2), and the events the kernel
//! raises for occupants the world reports on.

mod common;

use std::sync::Arc;

use clash_core::{EntityId, Hash128, Real, Tick, VarMap};
use clash_data::Mass;
use clash_sim::component::{ComponentKind, Identity, Physical, Shape, physical};
use clash_sim::{
    Advance, AdvanceLimit, Command, DebugCommand, EntitySpec, Place, Sim, SimEvent, Subject,
};
use clash_time::Timeline;
use clash_world::dims::TRACT_XY;
use clash_world::test_support::load_column;
use clash_world::{
    BlockState, Cause, CellPos, ColumnBuilder, Cosmos, GridId, Layer, Location, OccupantKind,
    PendingEdit, RealmId, SpawnRequest, TractPos, WorldActivity, WorldDefs,
};
use common::{GRID, creature, debug, entity_of, item, names, on, spawn};

fn shape(kg: f64) -> Shape {
    Shape {
        mass: Mass::new(Real::of(kg)),
        ..Shape::default()
    }
}

/// The mass the world's occupant record holds for an entity.
fn world_mass(sim: &Sim, id: EntityId) -> Option<Real> {
    sim.truth()
        .cosmos()
        .occupant(id)
        .map(|record| record.occupant.mass)
}

// ---- 1a: components ----------------------------------------------------

#[test]
fn the_simulation_holds_components_and_mass_reaches_the_world() {
    let mut sim = common::sim(1);
    let before = sim.hashes();
    let pack = spawn(&mut sim, item(), on(3, 3));
    let coin = spawn(&mut sim, item(), Place::In(pack));
    sim.components_mut()
        .identity
        .insert(pack, Identity::new(None));
    sim.set_shape(pack, shape(2.0), Cause::DEBUG).unwrap();
    sim.set_shape(coin, shape(0.5), Cause::DEBUG).unwrap();

    // The pack weighs what it holds too, in its component, in the
    // kernel's record and in the world's.
    let mass_of = |sim: &Sim, id| sim.components().get::<Physical>(id).unwrap().mass().get();
    assert_eq!(mass_of(&sim, pack), Real::of(2.5));
    assert_eq!(
        sim.truth().entity(pack).unwrap().presence().mass,
        Real::of(2.5)
    );
    assert_eq!(world_mass(&sim, pack), Some(Real::of(2.5)));
    assert_eq!(world_mass(&sim, coin), None, "the coin is inside the pack");
    assert_ne!(sim.hashes().components, before.components);
    assert_ne!(sim.state_hash(), before.total);

    // Out of the pack: both records follow.
    sim.relocate(coin, on(4, 4), Cause::DEBUG).unwrap();
    assert_eq!(world_mass(&sim, pack), Some(Real::of(2.0)));
    assert_eq!(world_mass(&sim, coin), Some(Real::of(0.5)));
    // Back in, and heavier while it is there.
    sim.relocate(coin, Place::In(pack), Cause::DEBUG).unwrap();
    sim.set_own_mass(coin, Mass::new(Real::of(1.5)), Cause::DEBUG)
        .unwrap();
    assert_eq!(world_mass(&sim, pack), Some(Real::of(3.5)));
    assert!(physical::mismatches(&sim.components().physical, sim.truth().entities()).is_empty());

    // A bad measure and an entity without the component are refused.
    let bad = sim.set_own_mass(coin, Mass::new(Real::of(-1.0)), Cause::DEBUG);
    assert_eq!(bad.unwrap_err().code, "physical.bad_measure");
    let bare = spawn(&mut sim, item(), on(5, 5));
    let none = sim.set_own_mass(bare, Mass::new(Real::ONE), Cause::DEBUG);
    assert_eq!(none.unwrap_err().code, "component.missing");

    // Saved and loaded, the components are the same state.
    let physical_hash = sim.component_hash(ComponentKind::Physical);
    let (loaded, unplaced) = common::save_and_load(&sim);
    assert!(unplaced.is_empty());
    assert_eq!(loaded.hashes(), sim.hashes());
    assert_eq!(
        loaded.component_hash(ComponentKind::Physical),
        physical_hash
    );
    assert_eq!(world_mass(&loaded, pack), Some(Real::of(3.5)));

    // Destroying the coin takes its components and lightens the pack.
    sim.remove_entity(coin, Cause::DEBUG).unwrap();
    assert!(sim.components().kinds_of(coin).is_empty());
    assert_eq!(world_mass(&sim, pack), Some(Real::of(2.0)));
    assert_eq!(mass_of(&sim, pack), Real::of(2.0));
    // Destroying the pack leaves nothing of it behind.
    sim.remove_entity(pack, Cause::DEBUG).unwrap();
    assert!(sim.components().kinds_of(pack).is_empty());
    assert!(
        sim.components()
            .orphans(|id| sim.truth().entities().contains(id))
            .is_empty()
    );
}

// ---- 1b, 1d: the world's wake-ups ---------------------------------------

const CORNER: (i32, i32) = (4, 4);
const SIDE: i32 = 7;
const STOREYS: i32 = 10;

/// A tower in a column being built: walls round the edge of a square
/// footprint on every storey and a slab over the whole footprint at the
/// top of each.
fn build_tower(builder: &mut ColumnBuilder<'_>, defs: &WorldDefs) {
    let wall = defs.block_state("core:wall", None).unwrap();
    let slab = defs.floor_state("core:slab", None).unwrap();
    for z in 0..=STOREYS {
        for y in 0..SIDE {
            for x in 0..SIDE {
                let cell = CellPos::new(CORNER.0 + x, CORNER.1 + y, z);
                if z >= 1 {
                    builder.set_floor(cell, slab);
                }
                let edge = x == 0 || y == 0 || x == SIDE - 1 || y == SIDE - 1;
                if z < STOREYS && edge {
                    builder.set_block(cell, wall);
                }
            }
        }
    }
}

/// A simulation over flat ground with a ten-storey tower on it, someone
/// on its roof and someone on its ground floor.
fn tower_sim() -> (Sim, EntityId, EntityId) {
    let defs = common::defs();
    let world = WorldDefs::from_defs(&defs).unwrap_or_else(|d| panic!("{}", d.render()));
    let mut timeline = Timeline::new();
    let mut cosmos = Cosmos::new(Arc::clone(&world));
    let def = world.realms().lookup("core:testland").unwrap();
    cosmos.create_realm(&mut timeline, def, 7).unwrap();
    let rock = world.block_state("core:solid", None).unwrap();
    let grass = world.floor_state("core:grass", None).unwrap();
    for (index, tract) in [TractPos::new(0, 0), TractPos::new(1, 0)]
        .into_iter()
        .enumerate()
    {
        let mut builder = ColumnBuilder::new(&world, rock, BlockState::NONE, 0);
        builder.fill_floor(0, grass);
        if index == 0 {
            build_tower(&mut builder, &world);
        }
        load_column(&mut cosmos, GRID, tract, builder.build()).unwrap();
    }
    let mut sim = Sim::from_parts(defs, cosmos, timeline, 3);
    let middle = (CORNER.0 + SIDE / 2, CORNER.1 + SIDE / 2);
    let roof = Location::new(GRID, CellPos::new(middle.0, middle.1, STOREYS));
    let above = spawn(&mut sim, creature(), Place::at(roof));
    let below = spawn(&mut sim, creature(), on(middle.0, middle.1));
    sim.drain_events();
    (sim, above, below)
}

/// Takes out every wall of the sixth storey, by debug command.
fn knock_out_a_storey(sim: &mut Sim) {
    for y in 0..SIDE {
        for x in 0..SIDE {
            if x == 0 || y == 0 || x == SIDE - 1 || y == SIDE - 1 {
                let loc = Location::new(GRID, CellPos::new(CORNER.0 + x, CORNER.1 + y, 5));
                debug(
                    sim,
                    DebugCommand::SetElement {
                        loc,
                        layer: Layer::Block,
                        state: BlockState::NONE.raw(),
                    },
                )
                .unwrap();
            }
        }
    }
}

fn count(events: &[SimEvent], name: &str) -> usize {
    events.iter().filter(|event| event.name() == name).count()
}

#[test]
fn a_tower_comes_down_under_advance_and_a_save_in_the_middle_changes_nothing() {
    // Straight through.
    let (mut straight, above, below) = tower_sim();
    knock_out_a_storey(&mut straight);
    assert!(matches!(
        straight.advance(AdvanceLimit::UntilDecision),
        Advance::Reached(_)
    ));
    assert!(straight.now() > Tick(2_000_000), "it took seconds");
    assert!(straight.truth().cosmos().processes().is_idle());
    let told = straight.drain_events();
    assert!(count(&told, "collapse_started") >= 1);
    assert_eq!(
        count(&told, "collapse_started"),
        count(&told, "collapse_settled")
    );
    assert!(count(&told, "element_destroyed") > 100);
    assert_eq!(count(&told, "fault"), 0, "{:?}", names(&told));

    // 1d: the one on the roof lost its footing, and the one on the
    // ground floor was under what landed. Each is told as an event
    // about the entity, after the world's own.
    let unsupported: Vec<&SimEvent> = told
        .iter()
        .filter(|event| event.name() == "entity_unsupported")
        .collect();
    assert!(
        unsupported
            .iter()
            .any(|event| entity_of(event, "self") == Some(above))
    );
    let exposed = told
        .iter()
        .find(|event| event.name() == "entity_exposed" && entity_of(event, "self") == Some(below))
        .expect("the one below was struck");
    assert_eq!(
        exposed
            .body
            .field("kind")
            .unwrap()
            .plain()
            .unwrap()
            .as_str(),
        Some("core:impact")
    );
    assert!(exposed.body.field("energy").unwrap().as_real().unwrap() > Real::ZERO);
    assert!(exposed.body.field("intensity").unwrap().as_real().unwrap() > Real::ZERO);
    assert!(matches!(exposed.subject("here"), Some(Subject::Loc(_))));
    let at = told
        .iter()
        .position(|event| core::ptr::eq(event, exposed))
        .unwrap();
    assert_eq!(told[at - 1].name(), "occupant_exposed");
    // The kernel moved neither: that is the movement package's.
    assert!(matches!(
        straight.truth().place(above),
        Some(Place::At { .. })
    ));

    // The same, saved while masses are in flight.
    let (mut first, _, _) = tower_sim();
    knock_out_a_storey(&mut first);
    assert_eq!(
        first.advance(AdvanceLimit::Until(Tick(1_000_000))),
        Advance::Reached(Tick(1_000_000))
    );
    assert!(first.truth().cosmos().falling_masses().count() > 0);
    assert!(first.truth().cosmos().collapses().count() > 0);

    let blob = first.save().unwrap();
    // The cells are copied: the cosmos has no store of its own yet
    // (world-model W10). What it has under way is taken out of the
    // copy, so that it can only come back from the blob.
    let mut cosmos = first.truth().cosmos().clone();
    cosmos.restore_activity(WorldActivity::new());
    assert_ne!(Hash128::of(&cosmos), first.hashes().cosmos);
    let mut second = Sim::load(Arc::clone(first.defs()), cosmos, &blob).unwrap();
    assert!(second.reindex().is_empty());
    assert_eq!(second.hashes(), first.hashes());
    assert!(matches!(
        second.advance(AdvanceLimit::UntilDecision),
        Advance::Reached(_)
    ));

    assert_eq!(second.hashes(), straight.hashes());
    // What was reported is the same too: what was waiting to be drained
    // at the save came with it.
    let again = second.drain_events();
    assert_eq!(names(&again), names(&told));
    let hashes = |events: &[SimEvent]| Hash128::of(&events.to_vec());
    assert_eq!(hashes(&again), hashes(&told));
}

// ---- 1c: activity --------------------------------------------------------

/// Where the walk starts: a street corner well inside the city.
const START_TRACT: TractPos = TractPos::new(20, 20);

fn city_cell(tract: TractPos, dx: i32, dy: i32) -> Location {
    Location::new(
        GridId::new(0),
        CellPos::new(tract.x * TRACT_XY + dx, tract.y * TRACT_XY + dy, 0),
    )
}

/// A simulation over an empty `core:test_city` realm. Nothing is loaded.
fn city_sim() -> (Sim, RealmId) {
    let defs = common::defs();
    let world = WorldDefs::from_defs(&defs).unwrap_or_else(|d| panic!("{}", d.render()));
    let mut timeline = Timeline::new();
    let mut cosmos = Cosmos::new(Arc::clone(&world));
    let def = world.realms().lookup("core:test_city").unwrap();
    let realm = cosmos.create_realm(&mut timeline, def, 11).unwrap();
    (Sim::from_parts(defs, cosmos, timeline, 5), realm)
}

fn loaded(sim: &Sim, realm: RealmId, tract: TractPos) -> bool {
    sim.truth().cosmos().is_tract_loaded(realm, tract)
}

/// Moves the walker one tract east or west, by debug command.
fn step(sim: &mut Sim, walker: EntityId, by: i32) {
    let Some(Place::At { loc, .. }) = sim.truth().place(walker) else {
        panic!("the walker is not in the world");
    };
    let to = Location::new(
        loc.grid,
        CellPos::new(loc.cell.x + by * TRACT_XY, loc.cell.y, loc.cell.z),
    );
    debug(
        sim,
        DebugCommand::MoveEntity {
            entity: walker,
            to: Place::at(to),
        },
    )
    .unwrap();
}

/// The whole walk: a session starts, something is left behind, the
/// walker goes `far` tracts east and comes back. `save_at` names the
/// steps after which the simulation is saved and loaded again.
fn walk(save_at: &[usize]) -> (Sim, Vec<String>) {
    const FAR: i32 = 11;
    let (mut sim, realm) = city_sim();
    let mut told: Vec<String> = Vec::new();
    let mut hear = |sim: &mut Sim| {
        told.extend(
            sim.drain_events()
                .iter()
                .map(|event| event.name().to_owned()),
        );
    };

    let start = city_cell(START_TRACT, 1, 1);
    assert!(!loaded(&sim, realm, START_TRACT));
    // The walker exists before it is anywhere.
    debug(
        &mut sim,
        DebugCommand::CreateEntity {
            spec: creature(),
            place: Place::Stored,
        },
    )
    .unwrap();
    let walker = sim.truth().entities().ids().next().unwrap();
    debug(
        &mut sim,
        DebugCommand::StartSession {
            entity: walker,
            at: start,
        },
    )
    .unwrap();
    assert_eq!(sim.truth().place(walker), Some(Place::at(start)));
    // Hot for six tracts round it and warm for two more: seventeen
    // tracts a side.
    assert_eq!(sim.truth().cosmos().loaded_tracts(realm).len(), 17 * 17);
    assert_eq!(sim.truth().cosmos().tickets().len(), 1);

    // Something stays behind, beside a wall that was not there before.
    let idle_at = city_cell(START_TRACT, 2, 1);
    debug(
        &mut sim,
        DebugCommand::CreateEntity {
            spec: EntitySpec::new(OccupantKind::Item).with_tag("left_behind"),
            place: Place::at(idle_at),
        },
    )
    .unwrap();
    let idle = sim.truth().entities().ids().nth(1).unwrap();
    let wall = common::wall(&sim);
    let wall_at = city_cell(START_TRACT, 1, 2);
    debug(
        &mut sim,
        DebugCommand::SetElement {
            loc: wall_at,
            layer: Layer::Block,
            state: wall.raw(),
        },
    )
    .unwrap();
    // And something is asked for in a tract that is not loaded yet.
    let ahead = TractPos::new(START_TRACT.x + 12, START_TRACT.y);
    let request = SpawnRequest {
        what: "core:pigeon".to_owned(),
        at: city_cell(ahead, 3, 3).cell,
        params: VarMap::new(),
    };
    sim.world(|ctx| {
        ctx.cosmos
            .add_pending_edit(realm, ahead, PendingEdit::Spawn(request))
    })
    .unwrap();
    hear(&mut sim);

    let mut steps_taken = 0;
    let maybe_save = |sim: &mut Sim, steps_taken: usize| {
        if !save_at.contains(&steps_taken) {
            return;
        }
        let blob = sim.save().unwrap();
        // The world as its own store would hand it back (W10 is not
        // built: the cosmos is copied), and the tract store with it.
        let cosmos = sim.truth().cosmos().clone();
        let (generators, store) = (sim.generators().clone(), sim.store().clone_store());
        let before = sim.hashes();
        let mut loaded = Sim::load(Arc::clone(sim.defs()), cosmos, &blob)
            .unwrap()
            .with_io(generators, store);
        assert!(loaded.reindex().is_empty());
        assert_eq!(loaded.hashes(), before);
        *sim = loaded;
    };

    // East.
    for hop in 1..=FAR {
        step(&mut sim, walker, 1);
        steps_taken += 1;
        hear(&mut sim);
        let here = TractPos::new(START_TRACT.x + hop, START_TRACT.y);
        // Loaded ahead as far as the ticket reaches, and no further.
        assert!(loaded(&sim, realm, TractPos::new(here.x + 8, here.y)));
        assert!(!loaded(&sim, realm, TractPos::new(here.x + 10, here.y)));
        // Unloaded behind, a tract beyond the reach.
        assert!(loaded(&sim, realm, TractPos::new(here.x - 9, here.y)));
        assert!(!loaded(&sim, realm, TractPos::new(here.x - 10, here.y)));
        maybe_save(&mut sim, steps_taken);
    }

    // What was left behind left the world with its tract, and waits.
    assert!(!loaded(&sim, realm, START_TRACT));
    assert_eq!(sim.truth().place(idle), Some(Place::Stored));
    let (_, waits_for, shelved) = sim.shelf().of(idle).expect("the idle one is shelved");
    assert_eq!((waits_for, shelved.loc), (START_TRACT, idle_at));
    assert!(sim.truth().cosmos().occupant(idle).is_none());
    // The tract was changed, so it was stored.
    assert!(sim.store().get(realm, START_TRACT).unwrap().is_some());

    // West again.
    for _ in 1..=FAR {
        step(&mut sim, walker, -1);
        steps_taken += 1;
        hear(&mut sim);
        maybe_save(&mut sim, steps_taken);
    }
    assert_eq!(sim.truth().place(walker), Some(Place::at(start)));
    assert_eq!(sim.truth().place(idle), Some(Place::at(idle_at)));
    assert!(sim.shelf().is_empty());
    assert!(
        sim.truth()
            .entity(idle)
            .unwrap()
            .tags
            .contains("left_behind")
    );
    assert_eq!(
        sim.truth().cosmos().cell(wall_at).unwrap().block.def(),
        wall.def()
    );
    assert!(sim.truth().cosmos().loaded_tracts(realm).len() >= 17 * 17);
    hear(&mut sim);
    (sim, told)
}

#[test]
fn tracts_follow_a_walker_and_what_is_left_behind_comes_back() {
    let (straight, told) = walk(&[]);
    let count = |name: &str| told.iter().filter(|have| *have == name).count();
    assert!(count("tract_loaded") > 17 * 17 + 17 * 11);
    assert!(count("tract_unloading") > 17 * 10);
    assert_eq!(count("spawn_requested"), 1);
    assert_eq!(count("fault"), 0);
    // The walker's own moves, and the idle one's two.
    assert_eq!(count("entity_relocated"), 1 + 1 + 22 + 2);

    // Saved and loaded on the way out, at the far end and on the way
    // back, the simulation ends in the same state.
    let (saved, told_again) = walk(&[3, 11, 17]);
    assert_eq!(saved.hashes(), straight.hashes());
    assert_eq!(told_again, told);
}

#[test]
fn a_session_is_refused_where_there_is_no_place() {
    let (mut sim, realm) = city_sim();
    let walker = sim.create_entity(creature(), Cause::DEBUG).unwrap();
    let outside = Location::new(GridId::new(0), CellPos::new(-5, 0, 0));
    let refused = sim.submit(Command::Debug(DebugCommand::StartSession {
        entity: walker,
        at: outside,
    }));
    assert_eq!(refused.unwrap_err().reason.code, "world.outside_realm");
    assert!(sim.truth().cosmos().tickets().is_empty());
    assert!(sim.truth().cosmos().loaded_tracts(realm).is_empty());
    assert!(sim.input_log().is_empty());
}
