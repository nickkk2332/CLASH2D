//! Tests for W13: structural support, load and collapse (02 §14).
//!
//! `tower_pancake` and `weakened_columns` are the bodies of the test
//! scripts of those names. The scripts themselves are owed: the script
//! runner does not exist yet (W11).

mod common;
#[path = "common/wreck.rs"]
mod wreck;

use std::sync::Arc;

use clash_core::{Dur, EntityId, Hash128, Real, Tick, codec};
use clash_time::{Timeline, Wake, WakeDomain};
use clash_world::{
    AmbientOverride, AreaShape, AreaSpec, BlockState, Cause, CauseKind, CellBox, CellPos,
    ColumnBuilder, Cosmos, DamagePacket, DamageResult, Destruction, EventSink, FixtureState,
    FloorState, Footprint, Generators, GravityOverride, Layer, MemoryStore, Occupant, OccupantKind,
    SETTLE_DELAY, TractIo, TractStore, WorldDefs, WorldEvent,
};
use wreck::{World, at, build_tower, edge_cells, someone, world_from};

/// The definitions of `mods/core`, with a weak material for posts that
/// carry most of what they can, and one with no bearing at all.
fn defs() -> Arc<WorldDefs> {
    let mut raw = common::test_raw();
    common::add_json(
        &mut raw,
        "t",
        r#"[
            { "//": "A post of this, half a square metre across, bears 10 tonnes.",
              "type": "material", "id": "chalk", "density": 1000, "toughness": 1.0e5,
              "resist": { "impact": { "threshold": "1 MPa" } },
              "span": 4, "bearing": "196.2 kPa", "rubble": "rubble" },
            { "type": "block", "id": "post", "material": "chalk",
              "height": 3.0, "fill": 0.5, "flags": ["load_bearing"] },
            { "type": "material", "id": "adamant", "density": 1000, "toughness": 1.0e5,
              "span": 4 },
            { "type": "block", "id": "prop", "material": "adamant",
              "height": 3.0, "fill": 0.01, "flags": ["load_bearing"] },
            { "type": "material", "id": "brine", "density": 1000, "toughness": 1.0e3 },
            { "type": "block", "id": "pool", "material": "brine",
              "height": 3.0, "fill": 1.0, "flags": ["liquid"] }
        ]"#,
        "the materials of the collapse tests",
    );
    Arc::new(WorldDefs::build(raw).unwrap_or_else(|errors| panic!("{errors:?}")))
}

fn count_events(w: &World, wanted: impl Fn(&WorldEvent) -> bool) -> usize {
    w.events
        .events()
        .iter()
        .filter(|event| wanted(event))
        .count()
}

fn started(w: &World) -> usize {
    count_events(w, |event| {
        matches!(event, WorldEvent::CollapseStarted { .. })
    })
}

fn settled(w: &World) -> usize {
    count_events(w, |event| {
        matches!(event, WorldEvent::CollapseSettled { .. })
    })
}

/// Two walls so many cells apart, three cells long, and a slab across
/// them one storey up.
fn bridge(builder: &mut ColumnBuilder<'_>, defs: &WorldDefs, apart: i32) {
    let wall = defs.block_state("core:wall", None).unwrap();
    let slab = defs.floor_state("core:slab", None).unwrap();
    for y in 4..=6 {
        builder.set_block(at(2, y, 0), wall);
        builder.set_block(at(2 + apart, y, 0), wall);
        for x in 2..=2 + apart {
            builder.set_floor(at(x, y, 1), slab);
        }
    }
}

#[test]
fn a_slab_of_span_four_stands_across_six_cells() {
    let defs = common::test_defs();
    let mut w = world_from(Arc::clone(&defs), |builder| bridge(builder, &defs, 6));
    let slab = w.floor("slab");
    let middle = at(5, 5, 1);

    let found = w.cosmos.assess_stability(w.grid, &[middle]).unwrap();
    assert!(found.is_stable());
    // 21 slab cells and 6 wall cells.
    assert_eq!(found.elements(), 27);
    assert_eq!(found.passes(), 1);
    assert!(!found.truncated());
    // A cell over a wall rests on it; the rest reach it through their
    // neighbours, a step at a time.
    let over_wall = at(2, 5, 1);
    assert_eq!(found.distance(over_wall, Layer::Floor), Some(0));
    assert_eq!(
        found.carrier(over_wall, Layer::Floor),
        Some((at(2, 5, 0), Layer::Block))
    );
    for (x, steps) in [(3, 1), (4, 2), (5, 3), (6, 2), (7, 1), (8, 0)] {
        assert_eq!(found.distance(at(x, 5, 1), Layer::Floor), Some(steps));
    }
    let (carrier, layer) = found.carrier(at(3, 5, 1), Layer::Floor).unwrap();
    assert_eq!((carrier.x, carrier.z, layer), (2, 1, Layer::Floor));
    // The wall under it carries the slab it is the nearest support of.
    assert_eq!(found.supported(at(2, 5, 0), Layer::Block), Some(true));
    assert!(found.load(at(2, 5, 0), Layer::Block).unwrap() > Real::of(1440.0 + 480.0));
    // Grass on rock is not part of it: it is always supported.
    assert_eq!(found.supported(at(5, 5, 0), Layer::Floor), None);

    // A check made in earnest changes nothing.
    let middle_loc = w.loc(5, 5, 1);
    w.ctx().check_stability(middle_loc, someone()).unwrap();
    assert_eq!(w.cosmos.stability_requests().len(), 1);
    w.run_out();
    assert_eq!(w.timeline.now(), Tick(SETTLE_DELAY.as_micros()));
    assert!(w.events.is_empty());
    assert!(w.cosmos.stability_requests().is_empty());
    assert!(w.cosmos.processes().is_idle());
    assert_eq!(w.count_floors(at(0, 0, 1), at(20, 10, 1), slab), 21);
}

#[test]
fn across_ten_cells_the_middle_falls() {
    let defs = common::test_defs();
    let mut w = world_from(Arc::clone(&defs), |builder| bridge(builder, &defs, 10));
    let slab = w.floor("slab");

    let found = w.cosmos.assess_stability(w.grid, &[at(7, 5, 1)]).unwrap();
    assert!(!found.is_stable());
    assert!(found.crushed().is_empty());
    assert_eq!(
        found.falling(),
        [
            (at(7, 4, 1), Layer::Floor),
            (at(7, 5, 1), Layer::Floor),
            (at(7, 6, 1), Layer::Floor),
        ]
    );
    assert_eq!(found.supported(at(7, 5, 1), Layer::Floor), Some(false));
    assert_eq!(found.distance(at(6, 5, 1), Layer::Floor), Some(4));

    let middle_loc = w.loc(7, 5, 1);
    w.ctx().check_stability(middle_loc, someone()).unwrap();
    w.run_out();
    // The middle row is gone and nothing else.
    assert_eq!(w.count_floors(at(0, 0, 1), at(20, 10, 1), slab), 30);
    for y in 4..=6 {
        assert_eq!(w.floor_at(7, y, 1), FloorState::NONE);
        // Each cell's 0.2 m³ lies on the grass below, which it was too
        // light to break.
        let pile = w.fixture_at(7, y, 0);
        assert_eq!(w.defs.rubble_units(pile), Some(200));
        assert_eq!(w.floor_at(7, y, 0), w.floor("grass"));
    }
    assert_eq!((started(&w), settled(&w)), (1, 1));
    assert!(w.cosmos.processes().is_idle());
    // The collapse is a process's doing, on behalf of whoever asked.
    let fell: Vec<&WorldEvent> = w
        .events
        .events()
        .iter()
        .filter(|event| matches!(event, WorldEvent::ElementDestroyed { .. }))
        .collect();
    assert_eq!(fell.len(), 3);
    for event in fell {
        let WorldEvent::ElementDestroyed { how, cause, .. } = event else {
            unreachable!()
        };
        assert_eq!(*how, Destruction::Fell);
        assert_eq!(cause.kind, CauseKind::Process);
        assert_eq!(cause.instigator, someone().instigator);
    }
}

const CORNER: (i32, i32) = (4, 4);

#[test]
fn removing_one_interior_column_of_a_tower_fails_locally() {
    let defs = common::test_defs();
    let mut w = world_from(Arc::clone(&defs), |builder| {
        build_tower(builder, &defs, CORNER, 11, 10, true);
    });
    let (wall, slab, pillar) = (w.block("wall"), w.floor("slab"), w.block("pillar"));
    let (low, high) = (at(4, 4, 0), at(14, 14, 10));
    assert_eq!(w.count_blocks(low, high, wall), 400);
    assert_eq!(w.count_floors(at(4, 4, 1), high, slab), 1210);
    assert_eq!(w.count_blocks(low, high, pillar), 10);
    // It stands as built: the middle of each slab rests on the pillar.
    let found = w.cosmos.assess_stability(w.grid, &[at(9, 9, 3)]).unwrap();
    assert!(found.is_stable());
    assert_eq!(found.distance(at(9, 9, 4), Layer::Floor), Some(0));
    assert_eq!(found.distance(at(8, 9, 4), Layer::Floor), Some(1));

    // The pillar of the fourth storey is taken out.
    let gone = w.loc(9, 9, 3);
    w.ctx()
        .edit(someone())
        .set_block(gone, BlockState::NONE)
        .unwrap();
    w.events.clear();
    w.run_out();

    // The middle of every slab above was five cells from a wall, and
    // fell with the pillars over it; what fell went through the slabs
    // below. Nothing else moved.
    assert_eq!(w.count_blocks(low, high, wall), 400);
    assert_eq!(w.count_floors(at(4, 4, 1), high, slab), 1200);
    assert_eq!(w.count_blocks(low, high, pillar), 3);
    for z in 1..=10 {
        assert_eq!(w.floor_at(9, 9, z), FloorState::NONE, "storey {z}");
    }
    for z in 0..3 {
        assert_eq!(w.block_at(9, 9, z), pillar);
    }
    assert!(w.defs.rubble_units(w.fixture_at(9, 9, 0)).is_some());
    for event in w.events.events() {
        if let WorldEvent::ElementDestroyed { loc, .. } = event {
            assert_eq!((loc.cell.x, loc.cell.y), (9, 9), "{event:?}");
        }
    }
    assert_eq!(started(&w), settled(&w));
    assert!(started(&w) >= 1);
    assert!(w.cosmos.processes().is_idle());
}

#[test]
fn removing_every_ground_storey_support_brings_the_whole_tower_down() {
    let defs = common::test_defs();
    let mut w = world_from(Arc::clone(&defs), |builder| {
        build_tower(builder, &defs, CORNER, 7, 6, false);
    });
    let (wall, slab) = (w.block("wall"), w.floor("slab"));
    let (low, high) = (at(4, 4, 0), at(10, 10, 6));
    assert_eq!(w.count_blocks(low, high, wall), 24 * 6);
    assert_eq!(w.count_floors(low, high, slab), 49 * 6);

    for cell in edge_cells(CORNER, 7, 0) {
        let loc = w.loc(cell.x, cell.y, cell.z);
        w.ctx()
            .edit(someone())
            .set_block(loc, BlockState::NONE)
            .unwrap();
    }
    w.events.clear();
    w.run_out();

    assert_eq!(w.count_blocks(low, high, wall), 0);
    assert_eq!(w.count_floors(low, high, slab), 0);
    assert!(w.cosmos.processes().is_idle());
    assert_eq!((started(&w), settled(&w)), (1, 1));

    // The collapse is traced to whoever took the walls out, and sums
    // what it brought down: five storeys of wall and six slabs, and the
    // grass that what fell went through.
    let summed = w.events.events().iter().find_map(|event| match event {
        WorldEvent::CollapseSettled { tallies, cause, .. } => Some((tallies.clone(), *cause)),
        _ => None,
    });
    let (tallies, cause) = summed.unwrap();
    assert_eq!(cause.kind, CauseKind::Process);
    assert_eq!(cause.instigator, someone().instigator);
    assert_eq!(tallies.len(), 1);
    assert_eq!(tallies[0].instigator, someone().instigator);
    let elements = 24 * 5 + 49 * 6;
    let mass = 24.0 * 5.0 * 1440.0 + 49.0 * 6.0 * 480.0;
    assert!(tallies[0].elements >= elements);
    assert!(tallies[0].elements <= elements + 49);
    assert!(tallies[0].mass.get() >= mass - 1.0e-6);
    assert!(tallies[0].mass.get() <= mass + 49.0 * 75.0 + 1.0e-6);
    assert_eq!(tallies[0].exposed, 0);
}

/// A column of wall three storeys high with a slab reaching four cells
/// from its top.
fn cantilever(builder: &mut ColumnBuilder<'_>, defs: &WorldDefs) {
    let wall = defs.block_state("core:wall", None).unwrap();
    let slab = defs.floor_state("core:slab", None).unwrap();
    for z in 0..3 {
        builder.set_block(at(10, 10, z), wall);
    }
    for x in 10..=14 {
        builder.set_floor(at(x, 10, 3), slab);
    }
}

#[test]
fn someone_moved_out_from_under_a_falling_mass_is_not_exposed() {
    let defs = common::test_defs();
    let mut w = world_from(Arc::clone(&defs), |builder| cantilever(builder, &defs));
    let found = w.cosmos.assess_stability(w.grid, &[at(10, 10, 3)]).unwrap();
    assert!(found.is_stable());
    assert_eq!(found.distance(at(14, 10, 3), Layer::Floor), Some(4));

    let (quick, slow, above) = (
        EntityId::new(20, 0),
        EntityId::new(21, 0),
        EntityId::new(22, 0),
    );
    let creature = |id| Occupant::new(id, OccupantKind::Creature);
    let (under_a, under_b, on_top) = (w.loc(12, 10, 0), w.loc(13, 10, 0), w.loc(14, 10, 3));
    let mut ctx = w.ctx();
    ctx.place(creature(quick), under_a, Footprint::SINGLE, someone())
        .unwrap();
    ctx.place(creature(slow), under_b, Footprint::SINGLE, someone())
        .unwrap();
    ctx.place(creature(above), on_top, Footprint::SINGLE, someone())
        .unwrap();

    // The top of the wall goes, and with it everything the slab had.
    let top = w.loc(10, 10, 2);
    w.ctx()
        .edit(someone())
        .set_block(top, BlockState::NONE)
        .unwrap();
    w.events.clear();

    // Half a second later the slab is on its way down: nine metres take
    // about 1.35 s. One of the two below steps aside.
    w.run_until(Tick(500_000));
    assert_eq!(w.cosmos.falling_masses().count(), 5);
    let aside = w.loc(12, 14, 0);
    w.ctx().relocate(quick, aside, someone()).unwrap();
    w.run_out();
    assert_eq!(w.cosmos.falling_masses().count(), 0);

    let mut exposed = Vec::new();
    let mut unsupported = Vec::new();
    for event in w.events.events() {
        match event {
            WorldEvent::OccupantExposed {
                occupant,
                loc,
                kind,
                energy,
                intensity,
                tick,
                ..
            } => {
                exposed.push(*occupant);
                assert_eq!(*loc, under_b);
                assert_eq!(*kind, w.damage_type("core:impact"));
                // 480 kg from nine metres up, over a square metre and
                // a tenth of a metre.
                let joules = 480.0 * 9.81 * 9.0;
                assert!((energy.get() - joules).abs() < 1.0e-6);
                assert!((intensity.get() - joules * 10.0).abs() < 1.0e-5);
                assert!(*tick > Tick(1_300_000));
            }
            WorldEvent::OccupantUnsupported {
                occupant,
                loc,
                tick,
                ..
            } => {
                unsupported.push(*occupant);
                assert_eq!(*loc, on_top);
                assert_eq!(*tick, Tick(SETTLE_DELAY.as_micros()));
            }
            _ => {}
        }
    }
    assert_eq!(exposed, [slow]);
    assert_eq!(unsupported, [above]);
    let tally = w.events.events().iter().find_map(|event| match event {
        WorldEvent::CollapseSettled { tallies, .. } => Some(tallies[0]),
        _ => None,
    });
    assert_eq!(tally.unwrap().exposed, 1);
}

#[test]
fn a_falling_mass_inside_stopped_time_hangs_and_resumes() {
    let defs = common::test_defs();
    let mut w = world_from(Arc::clone(&defs), |builder| cantilever(builder, &defs));
    let top = w.loc(10, 10, 2);
    w.ctx()
        .edit(someone())
        .set_block(top, BlockState::NONE)
        .unwrap();

    // The slab comes away after the settle delay. Thirty milliseconds
    // into its fall, time stops round it.
    w.run_until(Tick(80_000));
    assert_eq!(w.cosmos.falling_masses().count(), 5);
    let mut spec = AreaSpec::new(
        w.grid,
        AreaShape::Box(CellBox::new(at(8, 8, 0), at(16, 12, 4))),
    );
    spec.clock_rate = Some(Real::ZERO);
    let bubble = w.ctx().create_area(spec, someone()).unwrap();
    w.events.clear();

    // Ten seconds pass outside. Inside, nothing lands and nothing is
    // checked.
    w.run_for(Dur::from_secs(10));
    assert_eq!(w.cosmos.falling_masses().count(), 5);
    assert!(w.events.is_empty());
    assert_eq!(w.cosmos.collapses().count(), 1);
    for x in 10..=14 {
        assert_eq!(w.fixture_at(x, 10, 0), FixtureState::NONE);
    }

    // Time starts again, and the fall takes what it had left.
    w.ctx()
        .set_area_clock_rate(bubble, Some(Real::ONE))
        .unwrap();
    w.run_out();
    assert_eq!(w.cosmos.falling_masses().count(), 0);
    assert!(w.cosmos.processes().is_idle());
    let fall = Dur::from_secs_real((Real::of(2.0 * 9.0) / Real::of(9.81)).sqrt());
    let expected = Tick(SETTLE_DELAY.as_micros()) + fall + Dur::from_secs(10);
    let landed = w
        .events
        .events()
        .iter()
        .find_map(|event| match event {
            WorldEvent::ElementChanged {
                tick,
                layer: Layer::Fixture,
                ..
            } => Some(*tick),
            _ => None,
        })
        .unwrap();
    assert!((landed - expected).as_micros().abs() <= 2, "{landed}");
    for x in 10..=14 {
        assert!(w.defs.rubble_units(w.fixture_at(x, 10, 0)).is_some());
    }
    assert_eq!(settled(&w), 1);
}

/// A pillar at (10, 10) with a slab reaching four cells each way from
/// its top: 81 cells of 480 kg.
fn canopy(builder: &mut ColumnBuilder<'_>, defs: &WorldDefs, pillar: &str) {
    let pillar = defs.block_state(pillar, None).unwrap();
    let slab = defs.floor_state("core:slab", None).unwrap();
    builder.set_block(at(10, 10, 0), pillar);
    for y in 6..=14 {
        for x in 6..=14 {
            builder.set_floor(at(x, y, 1), slab);
        }
    }
}

const CANOPY_LOW: CellPos = CellPos::new(6, 6, 1);
const CANOPY_HIGH: CellPos = CellPos::new(14, 14, 1);

#[test]
fn a_pillar_is_crushed_when_damaged_to_the_fraction_its_load_predicts() {
    let defs = common::test_defs();
    let mut w = world_from(Arc::clone(&defs), |builder| {
        canopy(builder, &defs, "core:pillar@core:brick");
    });
    let slab = w.floor("slab");
    let pillar = w.block("pillar@brick");
    let here = at(10, 10, 0);
    let found = w.cosmos.assess_stability(w.grid, &[here]).unwrap();
    assert!(found.is_stable());

    // It carries the 81 cells of slab and its own 0.9 m³ of brick.
    let load = found.load(here, Layer::Block).unwrap();
    assert!((load.get() - (81.0 * 480.0 + 0.9 * 1900.0)).abs() < 1.0e-6);
    // 02 §14.5: crushed when load × gravity / area exceeds
    // bearing × (1 − damage / 255). The area is 0.3 m², the bearing of
    // brick 15 MPa: it bears its load eleven times over, and gives way
    // at the first damage for which the inequality holds.
    let stress = load.get() * 9.81 / 0.3;
    let needed = (0..=254_u8)
        .find(|damage| stress > 15.0e6 * (1.0 - f64::from(*damage) / 255.0))
        .unwrap();
    assert_eq!(needed, 233);

    let loc = w.loc(10, 10, 0);
    w.ctx()
        .edit(someone())
        .set_damage(loc, Layer::Block, needed - 1)
        .unwrap();
    w.run_out();
    assert_eq!(w.block_at(10, 10, 0), pillar);
    assert_eq!(w.count_floors(CANOPY_LOW, CANOPY_HIGH, slab), 81);
    assert_eq!(started(&w), 0);

    w.events.clear();
    w.ctx()
        .edit(someone())
        .set_damage(loc, Layer::Block, needed)
        .unwrap();
    w.run_out();
    assert_eq!(w.block_at(10, 10, 0), BlockState::NONE);
    assert_eq!(w.count_floors(CANOPY_LOW, CANOPY_HIGH, slab), 0);
    let crushed: Vec<&WorldEvent> = w
        .events
        .events()
        .iter()
        .filter(|event| {
            matches!(
                event,
                WorldEvent::ElementDestroyed {
                    how: Destruction::Crushed,
                    ..
                }
            )
        })
        .collect();
    assert_eq!(crushed.len(), 1);
    assert_eq!(crushed[0].location(), loc);
    assert_eq!(crushed[0].cause().kind, CauseKind::Process);
    assert_eq!(crushed[0].cause().instigator, someone().instigator);
    assert_eq!((started(&w), settled(&w)), (1, 1));
}

#[test]
fn a_solid_column_bears_about_three_thousand_tonnes() {
    // 02 §14.5: a solid concrete pillar one metre square bears about
    // 3,000 tonnes, and a slab reaching four cells each way from it
    // weighs about 39 tonnes a storey.
    let defs = common::test_defs();
    let w = world_from(Arc::clone(&defs), |builder| {
        canopy(builder, &defs, "core:column");
    });
    let found = w.cosmos.assess_stability(w.grid, &[at(10, 10, 0)]).unwrap();
    assert!(found.is_stable());
    let load = found.load(at(10, 10, 0), Layer::Block).unwrap().get();
    let slab = load - 3.0 * 2400.0;
    assert!((slab - 38_880.0).abs() < 1.0e-6);
    let bears = 30.0e6 * 1.0 / 9.81;
    assert!((3.0e6..3.1e6).contains(&bears));
    assert!(bears / slab > 70.0);
}

/// Posts at every second cell of a row, three storeys of them, with a
/// strip of slab over each storey.
fn frame(builder: &mut ColumnBuilder<'_>, defs: &WorldDefs) {
    let post = defs.block_state("t:post", None).unwrap();
    let slab = defs.floor_state("core:slab", None).unwrap();
    for z in 0..3 {
        for x in [2, 4, 6, 8, 10] {
            builder.set_block(at(x, 5, z), post);
        }
        for x in 2..=10 {
            builder.set_floor(at(x, 5, z + 1), slab);
        }
    }
}

const FRAME_LOW: CellPos = CellPos::new(2, 5, 0);
const FRAME_HIGH: CellPos = CellPos::new(10, 5, 3);

fn crushed(w: &World) -> usize {
    count_events(w, |event| {
        matches!(
            event,
            WorldEvent::ElementDestroyed {
                how: Destruction::Crushed,
                ..
            }
        )
    })
}

#[test]
fn removing_every_second_support_overloads_those_that_remain() {
    let defs = defs();
    let mut w = world_from(Arc::clone(&defs), |builder| frame(builder, &defs));
    let (post, slab) = (w.block("t:post"), w.floor("slab"));
    assert_eq!(w.count_blocks(FRAME_LOW, FRAME_HIGH, post), 15);
    assert_eq!(w.count_floors(FRAME_LOW, FRAME_HIGH, slab), 27);

    // As built it stands. A ground post carries three of its own kind
    // at 1.5 tonnes and two cells of slab on each storey: 7.38 tonnes
    // of the 10 it can bear.
    let found = w.cosmos.assess_stability(w.grid, &[at(2, 5, 0)]).unwrap();
    assert!(found.is_stable());
    assert_eq!(found.elements(), 42);
    for x in [2, 4, 6, 8] {
        let load = found.load(at(x, 5, 0), Layer::Block).unwrap();
        assert!((load.get() - 7380.0).abs() < 1.0e-6, "{x}: {load}");
    }
    let last = found.load(at(10, 5, 0), Layer::Block).unwrap();
    assert!((last.get() - 5940.0).abs() < 1.0e-6);
    // A post stands on the post below it, and a slab cell between posts
    // hangs from the cell beside it.
    assert_eq!(
        found.carrier(at(4, 5, 1), Layer::Block),
        Some((at(4, 5, 0), Layer::Block))
    );
    assert_eq!(
        found.carrier(at(3, 5, 1), Layer::Floor),
        Some((at(2, 5, 1), Layer::Floor))
    );

    // Two ground posts are taken out, and nothing else is touched.
    for x in [4, 8] {
        let loc = w.loc(x, 5, 0);
        w.ctx()
            .edit(someone())
            .set_block(loc, BlockState::NONE)
            .unwrap();
    }
    // What they carried hangs from the slab now, and the posts beside
    // them carry it: more than they can.
    let found = w.cosmos.assess_stability(w.grid, &[at(4, 5, 0)]).unwrap();
    // Post 2 now carries 12.78 tonnes and post 6 13.26: they go first.
    // Then everything hangs from the last column, by way of the slabs:
    // its ground post and the post that all of it hangs through go too.
    assert_eq!(
        found.crushed(),
        [at(2, 5, 0), at(6, 5, 0), at(10, 5, 0), at(8, 5, 1)],
    );
    assert_eq!(found.passes(), 3);
    assert_eq!(found.falling().len(), 42 - 2 - 4);
    w.events.clear();
    w.run_out();

    assert_eq!(w.count_blocks(FRAME_LOW, FRAME_HIGH, post), 0);
    assert_eq!(w.count_floors(FRAME_LOW, FRAME_HIGH, slab), 0);
    assert_eq!(crushed(&w), 4);
    assert_eq!(started(&w), settled(&w));
    assert!(w.cosmos.processes().is_idle());
}

fn gravity(scale: f64) -> Option<AmbientOverride> {
    Some(AmbientOverride {
        gravity: Some(GravityOverride::Scale(Real::of(scale))),
        drift: None,
    })
}

fn over_canopy(w: &World) -> AreaSpec {
    AreaSpec::new(
        w.grid,
        AreaShape::Box(CellBox::new(at(4, 4, 0), at(16, 16, 3))),
    )
}

#[test]
fn raised_gravity_crushes_what_stood() {
    let defs = common::test_defs();
    let mut w = world_from(Arc::clone(&defs), |builder| {
        canopy(builder, &defs, "core:pillar@core:brick");
    });
    let slab = w.floor("slab");
    let pillar = w.block("pillar@brick");

    // Eleven times its weight the pillar still bears: 14.6 of 15 MPa.
    let mut spec = over_canopy(&w);
    spec.ambient = gravity(11.0);
    let heavy = w.ctx().create_area(spec, someone()).unwrap();
    assert!(!w.cosmos.stability_requests().is_empty());
    w.run_out();
    assert_eq!(w.block_at(10, 10, 0), pillar);
    assert_eq!(w.count_floors(CANOPY_LOW, CANOPY_HIGH, slab), 81);
    assert_eq!(started(&w), 0);

    // Twelve times is too much.
    w.ctx()
        .set_area_ambient(heavy, gravity(12.0), someone())
        .unwrap();
    w.run_out();
    assert_eq!(w.block_at(10, 10, 0), BlockState::NONE);
    assert_eq!(w.count_floors(CANOPY_LOW, CANOPY_HIGH, slab), 0);
    assert_eq!(crushed(&w), 1);
    assert_eq!((started(&w), settled(&w)), (1, 1));
}

#[test]
fn nothing_is_crushed_and_nothing_falls_without_weight() {
    let defs = common::test_defs();
    let mut w = world_from(Arc::clone(&defs), |builder| {
        canopy(builder, &defs, "core:pillar@core:brick");
    });
    let slab = w.floor("slab");
    let pillar = w.block("pillar@brick");
    let mut spec = over_canopy(&w);
    spec.ambient = gravity(0.0);
    let weightless = w.ctx().create_area(spec, someone()).unwrap();
    w.run_out();

    // Damaged almost to nothing, the pillar still stands.
    let loc = w.loc(10, 10, 0);
    w.ctx()
        .edit(someone())
        .set_damage(loc, Layer::Block, 254)
        .unwrap();
    w.run_out();
    assert_eq!(w.block_at(10, 10, 0), pillar);
    // Taken away altogether, what it held stays where it is.
    w.ctx()
        .edit(someone())
        .set_block(loc, BlockState::NONE)
        .unwrap();
    w.run_out();
    assert_eq!(w.count_floors(CANOPY_LOW, CANOPY_HIGH, slab), 81);
    assert_eq!(started(&w), 0);
    assert_eq!(crushed(&w), 0);
    assert!(w.cosmos.processes().is_idle());

    // Weight comes back, and it comes down.
    w.ctx()
        .set_area_ambient(weightless, None, someone())
        .unwrap();
    w.run_out();
    assert_eq!(w.count_floors(CANOPY_LOW, CANOPY_HIGH, slab), 0);
    assert_eq!((started(&w), settled(&w)), (1, 1));
}

#[test]
fn a_material_with_no_bearing_is_never_crushed() {
    let defs = defs();
    let mut w = world_from(Arc::clone(&defs), |builder| {
        canopy(builder, &defs, "t:prop");
    });
    let slab = w.floor("slab");
    let prop = w.block("t:prop");
    // A hundredth of a square metre under 39 tonnes, damaged almost to
    // nothing, at a hundred times the weight.
    let loc = w.loc(10, 10, 0);
    w.ctx()
        .edit(someone())
        .set_damage(loc, Layer::Block, 254)
        .unwrap();
    let mut spec = over_canopy(&w);
    spec.ambient = gravity(100.0);
    w.ctx().create_area(spec, someone()).unwrap();
    let found = w.cosmos.assess_stability(w.grid, &[at(10, 10, 0)]).unwrap();
    assert!(found.is_stable());
    assert!(found.load(at(10, 10, 0), Layer::Block).unwrap() > Real::of(38_880.0));
    w.run_out();
    assert_eq!(w.block_at(10, 10, 0), prop);
    assert_eq!(w.count_floors(CANOPY_LOW, CANOPY_HIGH, slab), 81);
    assert_eq!((started(&w), crushed(&w)), (0, 0));
}

#[test]
fn a_mass_keeps_falling_when_what_it_was_to_land_on_goes() {
    let defs = common::test_defs();
    let mut w = world_from(Arc::clone(&defs), |builder| {
        cantilever(builder, &defs);
        // A second slab a storey up from the ground, under the end of
        // the first.
        let slab = defs.floor_state("core:slab", None).unwrap();
        builder.set_floor(at(14, 10, 1), slab);
    });
    let top = w.loc(10, 10, 2);
    w.ctx()
        .edit(someone())
        .set_block(top, BlockState::NONE)
        .unwrap();
    w.run_until(Tick(500_000));
    let bound_for: Vec<i32> = w.cosmos.falling_masses().map(|mass| mass.land_z).collect();
    assert_eq!(bound_for, [0, 0, 0, 0, 1]);

    // The lower slab is taken away while the upper is in the air.
    let lower = w.loc(14, 10, 1);
    w.ctx()
        .edit(someone())
        .set_floor(lower, FloorState::NONE)
        .unwrap();
    w.run_out();
    // It went on down to the ground, through the grass, and came to
    // rest there with the grass's 50 units of soil.
    assert_eq!(w.fixture_at(14, 10, 1), FixtureState::NONE);
    assert_eq!(w.defs.rubble_units(w.fixture_at(14, 10, 0)), Some(250));
    assert!(w.cosmos.processes().is_idle());
}

#[test]
fn a_still_liquid_is_not_structure() {
    let defs = defs();
    let mut w = world_from(Arc::clone(&defs), |builder| {
        // A tank of liquid two storeys deep, with a wall beside it.
        let pool = defs.block_state("t:pool", None).unwrap();
        let wall = defs.block_state("core:wall", None).unwrap();
        for z in 0..2 {
            for y in 20..=22 {
                for x in 20..=22 {
                    builder.set_block(at(x, y, z), pool);
                }
            }
        }
        builder.set_block(at(19, 21, 0), wall);
    });
    let pool = w.block("t:pool");
    // The wall goes. The liquid beside it stands on nothing that bears,
    // and reaches nowhere: as structure it would fall. It is not looked
    // at.
    let beside = w.loc(19, 21, 0);
    w.ctx()
        .edit(someone())
        .set_block(beside, BlockState::NONE)
        .unwrap();
    let found = w.cosmos.assess_stability(w.grid, &[at(19, 21, 0)]).unwrap();
    assert!(found.is_stable());
    assert_eq!(found.elements(), 0);
    w.run_out();
    assert_eq!(w.count_blocks(at(20, 20, 0), at(22, 22, 1), pool), 18);
    assert_eq!(started(&w), 0);
}

#[test]
fn a_wake_up_that_is_not_the_worlds_is_left_alone() {
    let mut w = world_from(common::test_defs(), |_| {});
    let foreign = Wake::new(WakeDomain::Action, 0, 0);
    assert_eq!(w.ctx().handle_wake(&foreign), Ok(false));
    // And one for a process that has gone.
    let stale = Wake::new(WakeDomain::World, 99, 0);
    assert_eq!(w.ctx().handle_wake(&stale), Ok(false));
    assert!(w.events.is_empty());
}

// ---- tower_pancake -------------------------------------------------

/// A ten-storey tower, seven cells square.
fn pancake_world() -> World {
    let defs = common::test_defs();
    world_from(Arc::clone(&defs), |builder| {
        build_tower(builder, &defs, CORNER, 7, 10, false);
    })
}

/// Takes out every wall of the sixth storey.
fn knock_out_a_storey(w: &mut World) {
    for cell in edge_cells(CORNER, 7, 5) {
        let loc = w.loc(cell.x, cell.y, cell.z);
        w.ctx()
            .edit(someone())
            .set_block(loc, BlockState::NONE)
            .unwrap();
    }
}

#[test]
fn tower_pancake() {
    // Straight through.
    let mut straight = pancake_world();
    knock_out_a_storey(&mut straight);
    straight.run_out();

    let (wall, slab) = (straight.block("wall"), straight.floor("slab"));
    let (low, high) = (at(4, 4, 0), at(10, 10, 10));
    // The storeys above came down on the sixth floor. Round the edge
    // they went through it, and through every floor below; and each
    // floor, with its edge gone, had nothing to hang from and followed.
    // What is left is five storeys of wall round a heap.
    assert_eq!(straight.count_floors(low, high, slab), 0);
    assert_eq!(straight.count_blocks(low, at(10, 10, 4), wall), 24 * 5);
    assert_eq!(straight.count_blocks(at(4, 4, 5), high, wall), 0);
    assert!(straight.cosmos.processes().is_idle());
    assert!(straight.cosmos.stability_requests().is_empty());
    assert_eq!(started(&straight), settled(&straight));
    // It went on for seconds, floor by floor, and nobody scripted it.
    assert!(straight.timeline.now() > Tick(2_000_000));
    let floors_broken = count_events(&straight, |event| {
        matches!(
            event,
            WorldEvent::ElementDestroyed {
                how: Destruction::Damage(_),
                layer: Layer::Floor,
                ..
            }
        )
    });
    assert!(floors_broken >= 24 * 5, "{floors_broken}");

    // The same, saved in the middle and restored.
    let mut first = pancake_world();
    knock_out_a_storey(&mut first);
    first.run_until(Tick(1_000_000));
    assert!(first.cosmos.falling_masses().count() > 0);
    assert!(first.cosmos.collapses().count() > 0);

    // A real save: the world into a store, and the timeline beside it
    // as the simulation's own section (02 §11).
    let mut store = MemoryStore::new();
    let report = first.cosmos.save_world(&mut store).unwrap();
    assert_eq!((report.loaded_tracts, report.tract_blobs), (2, 2));
    store.put_section("sim.timeline", &codec::to_bytes(&first.timeline).unwrap());
    store.commit().unwrap();
    let saved_timeline = store.section("sim.timeline").unwrap().unwrap();
    let timeline: Timeline = codec::from_bytes(&saved_timeline).unwrap();
    let generators = Generators::none(&first.defs);
    let cosmos =
        Cosmos::load_world(&first.defs, &mut TractIo::new(&generators, &mut store)).unwrap();
    assert_eq!(cosmos.processes(), first.cosmos.processes());
    assert!(!cosmos.processes().is_idle());
    let mut second = World {
        defs: Arc::clone(&first.defs),
        timeline,
        cosmos,
        events: EventSink::new(),
        grid: first.grid,
    };
    assert_eq!(second.state_hash(), first.state_hash());
    second.run_out();

    assert_eq!(second.state_hash(), straight.state_hash());
    let mut told = first.events.drain();
    told.extend(second.events.drain());
    assert_eq!(Hash128::of(&told), Hash128::of(&straight.events.drain()));
}

// ---- weakened_columns ----------------------------------------------

#[test]
fn weakened_columns() {
    let defs = defs();
    let mut w = world_from(Arc::clone(&defs), |builder| frame(builder, &defs));
    let (post, slab) = (w.block("t:post"), w.floor("slab"));

    // Each ground post takes one blow that it survives: 50 kJ against
    // its 150 kJ, a third of what it can take.
    let blow = DamagePacket::new(
        w.damage_type("core:impact"),
        Real::of(50.0e3),
        Real::of(10.0e6),
        someone(),
    );
    for x in [2, 4, 6, 8, 10] {
        let loc = w.loc(x, 5, 0);
        let outcome = w
            .ctx()
            .edit(Cause::DEBUG)
            .damage(loc, Layer::Block, blow)
            .unwrap();
        assert_eq!(outcome.result, DamageResult::Damaged);
        assert_eq!(w.damage_at(x, 5, 0, Layer::Block), 85);
    }
    assert_eq!(w.count_blocks(FRAME_LOW, FRAME_HIGH, post), 15);

    // A post that bore 7.38 of 10 tonnes whole bears 6.67 now. No blow
    // destroyed anything; the load does.
    let found = w.cosmos.assess_stability(w.grid, &[at(2, 5, 0)]).unwrap();
    // Four give way at once. The fifth bore less and holds for a
    // moment, until everything hangs from its column.
    assert_eq!(
        found.crushed(),
        [
            at(2, 5, 0),
            at(4, 5, 0),
            at(6, 5, 0),
            at(8, 5, 0),
            at(10, 5, 0),
            at(8, 5, 1)
        ]
    );
    w.events.clear();
    w.run_out();
    assert_eq!(w.count_blocks(FRAME_LOW, FRAME_HIGH, post), 0);
    assert_eq!(w.count_floors(FRAME_LOW, FRAME_HIGH, slab), 0);
    assert_eq!(crushed(&w), 6);
    assert!(w.cosmos.processes().is_idle());
    // The collapse is traced to whoever struck the posts.
    let cause = w
        .events
        .events()
        .iter()
        .find_map(|event| match event {
            WorldEvent::CollapseStarted { cause, .. } => Some(*cause),
            _ => None,
        })
        .unwrap();
    assert_eq!(cause.kind, CauseKind::Process);
    assert_eq!(cause.instigator, someone().instigator);
    assert_eq!(cause.actor, None);
}

// ---- timing ----------------------------------------------------------

/// A ten-storey building thirty cells square: walls round the edge and
/// pillars every seven cells inside.
fn office(builder: &mut ColumnBuilder<'_>, defs: &WorldDefs) {
    build_tower(builder, defs, (1, 1), 30, 10, false);
    let pillar = defs.block_state("core:pillar", None).unwrap();
    for z in 0..10 {
        for y in [8, 15, 22] {
            for x in [8, 15, 22] {
                builder.set_block(at(x, y, z), pillar);
            }
        }
    }
}

/// Not a benchmark with a budget: rough figures, printed. Run with
/// `cargo test --release -p clash-world --test collapse -- --ignored --nocapture`.
#[test]
#[ignore = "timing; run by hand in release mode"]
#[allow(
    clippy::disallowed_types,
    clippy::disallowed_methods,
    reason = "a timing test reads the wall clock; nothing in the simulation does"
)]
fn timing_of_a_check_and_of_a_collapse() {
    let defs = common::test_defs();
    let mut w = world_from(Arc::clone(&defs), |builder| office(builder, &defs));

    let mut best = f64::INFINITY;
    let mut elements = 0;
    for _ in 0..10 {
        let began = std::time::Instant::now();
        let found = w.cosmos.assess_stability(w.grid, &[at(8, 8, 0)]).unwrap();
        best = best.min(began.elapsed().as_secs_f64());
        assert!(found.is_stable());
        elements = found.elements();
    }
    println!(
        "one check of a standing ten-storey building, {elements} elements: {:.3} ms",
        best * 1.0e3
    );

    // Every support of the ground storey goes at once.
    let mut supports = edge_cells((1, 1), 30, 0);
    for y in [8, 15, 22] {
        for x in [8, 15, 22] {
            supports.push(at(x, y, 0));
        }
    }
    for cell in supports {
        let loc = w.loc(cell.x, cell.y, cell.z);
        w.ctx()
            .edit(someone())
            .set_block(loc, BlockState::NONE)
            .unwrap();
    }
    let began = std::time::Instant::now();
    w.run_out();
    let took = began.elapsed().as_secs_f64();
    let fell = count_events(&w, |event| {
        matches!(event, WorldEvent::ElementDestroyed { .. })
    });
    println!(
        "the collapse of the whole building, {fell} elements destroyed, {} events, {:.1} simulated seconds: {:.1} ms",
        w.events.len(),
        w.timeline.now().as_micros() as f64 / 1.0e6,
        took * 1.0e3
    );
    assert!(w.cosmos.processes().is_idle());
    assert_eq!(
        w.count_floors(at(1, 1, 1), at(30, 30, 10), w.floor("slab")),
        0
    );
}
