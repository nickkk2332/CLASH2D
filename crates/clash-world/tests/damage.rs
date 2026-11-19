//! Tests for W12: damage packets applied to the elements of cells, and
//! destruction with what it leaves (02 §12).
//!
//! `a_wall_is_punched_through` is the body of the test script
//! `wall_punch`. The script itself is owed: the script runner does not
//! exist yet (W11).

mod common;
#[path = "common/wreck.rs"]
mod wreck;

use clash_core::{EntityId, Real};
use clash_world::{
    BlockState, DamageOutcome, DamagePacket, DamageResult, Destruction, FixtureState, FloorState,
    Footprint, Layer, Occupant, OccupantKind, RUBBLE_CELL_UNITS, WorldDefs, WorldError, WorldEvent,
};
use wreck::{World, someone, world_from};

fn world() -> World {
    world_from(common::test_defs(), |_| {})
}

fn impact(w: &World, energy: f64, intensity: f64) -> DamagePacket {
    DamagePacket::new(
        w.damage_type("core:impact"),
        Real::of(energy),
        Real::of(intensity),
        someone(),
    )
}

/// A rubble fixture of a material holding so many units.
fn rubble(defs: &WorldDefs, material: &str, units: u16) -> FixtureState {
    let pile = defs.fixtures().lookup("core:rubble").unwrap();
    let material = defs.materials().lookup(material).unwrap();
    FixtureState::new(pile, material, units)
}

#[test]
fn a_wall_is_punched_through() {
    let mut w = world();
    let wall = w.block("wall");
    let here = w.loc(5, 5, 0);
    w.ctx().edit(someone()).set_block(here, wall).unwrap();
    w.events.clear();
    let before = w.timeline.wake_count();

    // The slow example of 02 §12.2: 16 kJ at 0.3 MPa. Nothing happens.
    let slow = impact(&w, 16.0e3, 0.3e6);
    let outcome = w.ctx().edit(someone()).damage(here, Layer::Block, slow);
    assert_eq!(outcome, Ok(DamageOutcome::UNAFFECTED));
    assert!(w.events.is_empty());
    assert_eq!(w.block_at(5, 5, 0), wall);

    // The fast one: 14.4 MJ at 290 MPa. The wall goes, 0.6 MJ is spent
    // and 13.8 MJ carries on.
    let fast = impact(&w, 14.4e6, 290.0e6);
    let outcome = w
        .ctx()
        .edit(someone())
        .damage(here, Layer::Block, fast)
        .unwrap();
    assert_eq!(outcome.result, DamageResult::Destroyed);
    assert!((outcome.absorbed.get() - 0.6e6).abs() < 1.0e-3);
    assert!((outcome.passed.get() - 13.8e6).abs() < 1.0e-3);

    // The wall is gone, and its 0.6 m³ of concrete lies where it stood.
    assert_eq!(w.block_at(5, 5, 0), BlockState::NONE);
    let pile = rubble(&w.defs, "core:concrete", 600);
    assert_eq!(w.fixture_at(5, 5, 0), pile);

    // One destruction event, with its cause, after the changes it sums.
    let tick = w.timeline.now();
    assert_eq!(
        w.events.drain(),
        [
            WorldEvent::ElementChanged {
                tick,
                loc: here,
                layer: Layer::Block,
                old: wall.raw(),
                new: BlockState::NONE.raw(),
                cause: someone(),
            },
            WorldEvent::ElementChanged {
                tick,
                loc: here,
                layer: Layer::Fixture,
                old: FixtureState::NONE.raw(),
                new: pile.raw(),
                cause: someone(),
            },
            WorldEvent::ElementDestroyed {
                tick,
                loc: here,
                layer: Layer::Block,
                old: wall.raw(),
                new: BlockState::NONE.raw(),
                how: Destruction::Damage(w.damage_type("core:impact")),
                absorbed: outcome.absorbed,
                passed: outcome.passed,
                cause: someone(),
            },
        ]
    );
    // And a stability check is asked for, with a wake-up to make it.
    assert!(w.cosmos.stability_requests().contains(here));
    assert_eq!(w.timeline.wake_count(), before + 1);
}

#[test]
fn ten_thousand_packets_below_the_threshold_change_nothing() {
    let mut w = world();
    let wall = w.block("wall");
    let here = w.loc(5, 5, 0);
    w.ctx().edit(someone()).set_block(here, wall).unwrap();
    w.run_out();
    w.events.clear();
    let before = w.state_hash();

    // Any amount of energy, at the threshold exactly and no more.
    let tap = impact(&w, 1.0e9, 20.0e6);
    for _ in 0..10_000 {
        let outcome = w.ctx().edit(someone()).damage(here, Layer::Block, tap);
        assert_eq!(outcome, Ok(DamageOutcome::UNAFFECTED));
    }
    assert!(w.events.is_empty());
    assert_eq!(w.block_at(5, 5, 0), wall);
    assert_eq!(w.damage_at(5, 5, 0, Layer::Block), 0);
    assert_eq!(w.state_hash(), before);
}

#[test]
fn repeated_packets_destroy_at_the_predicted_count() {
    let mut w = world();
    let wall = w.block("wall");
    let here = w.loc(5, 5, 0);
    w.ctx().edit(someone()).set_block(here, wall).unwrap();
    w.events.clear();

    // 60 kJ at twice the threshold is a tenth of the wall's 0.6 MJ:
    // 25.5 parts of 255, rounded up to 26. The packet that destroys is
    // the first that is at least what remains: the tenth.
    let blow = impact(&w, 60.0e3, 40.0e6);
    let mut packets = 0;
    let last = loop {
        packets += 1;
        let outcome = w
            .ctx()
            .edit(someone())
            .damage(here, Layer::Block, blow)
            .unwrap();
        if outcome.result == DamageResult::Destroyed {
            break outcome;
        }
        assert_eq!(outcome.result, DamageResult::Damaged);
        assert_eq!(outcome.absorbed, Real::of(60.0e3));
        assert_eq!(outcome.passed, Real::ZERO);
        assert_eq!(w.damage_at(5, 5, 0, Layer::Block), 26 * packets);
    };
    assert_eq!(packets, 10);
    // What remained was 21 parts of 255: the rest of the packet passes.
    let remained = 0.6e6 * 21.0 / 255.0;
    assert!((last.absorbed.get() - remained).abs() < 1.0e-6);
    assert!((last.passed.get() - (60.0e3 - remained)).abs() < 1.0e-6);
    assert_eq!(w.block_at(5, 5, 0), BlockState::NONE);

    let damaged = w
        .events
        .events()
        .iter()
        .filter(|event| matches!(event, WorldEvent::ElementDamaged { .. }))
        .count();
    let destroyed = w
        .events
        .events()
        .iter()
        .filter(|event| matches!(event, WorldEvent::ElementDestroyed { .. }))
        .count();
    assert_eq!((damaged, destroyed), (9, 1));
    // A new wall there starts whole.
    w.ctx().edit(someone()).set_block(here, wall).unwrap();
    assert_eq!(w.damage_at(5, 5, 0, Layer::Block), 0);
}

#[test]
fn where_there_is_nothing_everything_passes() {
    let mut w = world();
    let here = w.loc(5, 5, 0);
    let blow = impact(&w, 500.0, 1.0e9);
    let outcome = w.ctx().edit(someone()).damage(here, Layer::Block, blow);
    assert_eq!(outcome, Ok(DamageOutcome::through(Real::of(500.0))));
    assert!(w.events.is_empty());

    // A cell that is not loaded is refused.
    let far = w.loc(500, 5, 0);
    let refused = w.ctx().edit(someone()).damage(far, Layer::Block, blow);
    assert_eq!(refused, Err(WorldError::NotLoaded(far)));
}

#[test]
fn the_packet_carries_the_cause() {
    let mut w = world();
    let wall = w.block("wall");
    let here = w.loc(5, 5, 0);
    w.ctx().edit(someone()).set_block(here, wall).unwrap();
    w.events.clear();

    let thrower = EntityId::new(1, 0);
    let thrown = EntityId::new(2, 0);
    let cause = clash_world::Cause::new(clash_world::CauseKind::Process)
        .by(thrown)
        .instigated_by(thrower);
    let mut blow = impact(&w, 14.4e6, 290.0e6);
    blow.cause = cause;
    // Whatever the edit was begun for, the packet's cause is what the
    // destruction carries.
    w.ctx()
        .edit(clash_world::Cause::DEBUG)
        .damage(here, Layer::Block, blow)
        .unwrap();
    assert!(!w.events.is_empty());
    for event in w.events.events() {
        assert_eq!(event.cause(), cause, "{event:?}");
    }
}

#[test]
fn rubble_adds_up_and_a_full_cell_becomes_a_heap() {
    let mut w = world();
    let wall = w.block("wall");
    let heap = w.block("rubble_heap");
    let here = w.loc(5, 5, 0);
    let blow = impact(&w, 14.4e6, 290.0e6);

    // Each wall leaves 600 units. Four of them lie as a pile.
    for walls in 1..=4_u16 {
        w.ctx().edit(someone()).set_block(here, wall).unwrap();
        w.ctx()
            .edit(someone())
            .damage(here, Layer::Block, blow)
            .unwrap();
        let pile = rubble(&w.defs, "core:concrete", 600 * walls);
        assert_eq!(w.fixture_at(5, 5, 0), pile);
        assert_eq!(w.block_at(5, 5, 0), BlockState::NONE);
    }
    // The fifth fills the cell: the pile becomes the heap block.
    assert_eq!(u32::from(5 * 600_u16), RUBBLE_CELL_UNITS);
    w.ctx().edit(someone()).set_block(here, wall).unwrap();
    w.ctx()
        .edit(someone())
        .damage(here, Layer::Block, blow)
        .unwrap();
    assert_eq!(w.fixture_at(5, 5, 0), FixtureState::NONE);
    assert_eq!(w.block_at(5, 5, 0).def(), heap.def());
    assert_eq!(
        w.block_at(5, 5, 0).material(),
        w.defs.materials().lookup("core:concrete").unwrap()
    );
    // The heap weighs what the five walls did.
    let defs = &w.defs;
    let heap_mass = defs.bulk_mass(Layer::Block, w.block_at(5, 5, 0).raw());
    let walls_mass = Real::of(5.0) * defs.mass(Layer::Block, wall.raw());
    assert!((heap_mass - walls_mass).abs() < Real::of(1.0e-6));

    // More rubble goes on top of the heap.
    let above = w.loc(5, 5, 1);
    w.ctx().edit(someone()).set_block(above, wall).unwrap();
    w.ctx()
        .edit(someone())
        .damage(above, Layer::Block, blow)
        .unwrap();
    assert_eq!(w.fixture_at(5, 5, 1), rubble(&w.defs, "core:concrete", 600));

    // A heap that is destroyed leaves nothing: rubble makes no rubble.
    let crush = impact(&w, 1.0e9, 1.0e9);
    let outcome = w
        .ctx()
        .edit(someone())
        .damage(here, Layer::Block, crush)
        .unwrap();
    assert_eq!(outcome.result, DamageResult::Destroyed);
    assert_eq!(w.block_at(5, 5, 0), BlockState::NONE);
    assert_eq!(w.fixture_at(5, 5, 0), FixtureState::NONE);

    // The pile that lay on the heap has nothing under it now. The check
    // the destruction asked for finds that, and it comes down: through
    // the grass, onto the rock, with the grass's 50 units of soil.
    assert_eq!(w.fixture_at(5, 5, 1), rubble(&w.defs, "core:concrete", 600));
    w.run_out();
    assert_eq!(w.fixture_at(5, 5, 1), FixtureState::NONE);
    assert_eq!(w.floor_at(5, 5, 0), FloorState::NONE);
    assert_eq!(w.fixture_at(5, 5, 0), rubble(&w.defs, "core:concrete", 650));
    assert!(w.cosmos.processes().is_idle());
}

#[test]
fn rubble_comes_to_rest_on_the_nearest_cell_below_that_can_be_stood_in() {
    let mut w = world();
    let slab = w.floor("slab");
    let desk = w.fixture("desk");
    // A slab in mid-air, three storeys up, and a desk on the ground
    // under it.
    let up = w.loc(5, 5, 3);
    let ground = w.loc(5, 5, 0);
    w.ctx().edit(someone()).set_floor(up, slab).unwrap();
    w.ctx().edit(someone()).set_fixture(ground, desk).unwrap();
    assert_eq!(w.cosmos.resting_cell(up), Ok(up));

    // 250 kJ at 1 MPa against the slab's 200 kJ behind 300 kPa.
    let blow = impact(&w, 250.0e3, 1.0e6);
    let outcome = w
        .ctx()
        .edit(someone())
        .damage(up, Layer::Floor, blow)
        .unwrap();
    assert_eq!(outcome.result, DamageResult::Destroyed);
    assert!((outcome.passed.get() - 50.0e3).abs() < 1.0e-6);
    assert_eq!(w.floor_at(5, 5, 3), FloorState::NONE);
    assert_eq!(w.cosmos.resting_cell(up), Ok(ground));

    // Its 0.2 m³ lies on the ground, and the desk is under it: the
    // desk's 0.3 m³ has joined the pile.
    assert_eq!(w.fixture_at(5, 5, 3), FixtureState::NONE);
    assert_eq!(w.fixture_at(5, 5, 0), rubble(&w.defs, "core:concrete", 500));
}

#[test]
fn an_element_becomes_its_on_destroy_state_and_leaves_what_it_lost() {
    let mut raw = common::test_raw();
    common::add_json(
        &mut raw,
        "t",
        r#"[
            { "type": "block", "id": "stump", "material": "core:concrete",
              "height": 1.0, "fill": 0.1 },
            { "type": "block", "id": "panel", "material": "core:concrete",
              "height": 3.0, "fill": 0.3, "flags": ["load_bearing"],
              "on_destroy": "stump" }
        ]"#,
        "an element with an on_destroy state",
    );
    let defs = std::sync::Arc::new(WorldDefs::build(raw).unwrap());
    let mut w = world_from(defs, |_| {});
    let panel = w.block("t:panel");
    let stump = w.block("t:stump");
    let here = w.loc(5, 5, 0);
    w.ctx().edit(someone()).set_block(here, panel).unwrap();
    w.events.clear();

    let blow = impact(&w, 14.4e6, 290.0e6);
    w.ctx()
        .edit(someone())
        .damage(here, Layer::Block, blow)
        .unwrap();
    // 0.9 m³ became 0.3 m³: the other 0.6 m³ is rubble.
    assert_eq!(w.block_at(5, 5, 0), stump);
    assert_eq!(w.fixture_at(5, 5, 0), rubble(&w.defs, "core:concrete", 600));
    let destroyed: Vec<&WorldEvent> = w
        .events
        .events()
        .iter()
        .filter(|event| matches!(event, WorldEvent::ElementDestroyed { .. }))
        .collect();
    assert_eq!(destroyed.len(), 1);
    assert!(matches!(
        destroyed[0],
        WorldEvent::ElementDestroyed { old, new, .. } if *old == panel.raw() && *new == stump.raw()
    ));
}

#[test]
fn whoever_stood_on_what_was_destroyed_is_told() {
    let mut w = world();
    let slab = w.floor("slab");
    let ramp = w.block("ramp");
    let on_slab = EntityId::new(10, 0);
    let on_ramp = EntityId::new(11, 0);
    let beside = EntityId::new(12, 0);
    let wide = EntityId::new(13, 0);

    // A slab in the air with someone on it; a ramp with someone at its
    // top, who stands on the ramp and not on any floor; someone on the
    // ground next to it all; and something two cells wide, half on a
    // second slab and half on a third.
    let (air, top, ground) = (w.loc(5, 5, 2), w.loc(8, 5, 1), w.loc(6, 5, 0));
    let (left, right) = (w.loc(10, 5, 2), w.loc(11, 5, 2));
    let mut ctx = w.ctx();
    let mut edit = ctx.edit(someone());
    edit.set_floor(air, slab).unwrap();
    edit.set_block(top.offset(clash_core::IVec3::new(0, 0, -1)), ramp)
        .unwrap();
    edit.set_floor(left, slab).unwrap();
    edit.set_floor(right, slab).unwrap();
    let creature = |id| Occupant::new(id, OccupantKind::Creature);
    ctx.place(creature(on_slab), air, Footprint::SINGLE, someone())
        .unwrap();
    ctx.place(creature(on_ramp), top, Footprint::SINGLE, someone())
        .unwrap();
    ctx.place(creature(beside), ground, Footprint::SINGLE, someone())
        .unwrap();
    ctx.place(creature(wide), left, Footprint::new(2, 1, 1), someone())
        .unwrap();
    w.events.clear();

    let unsupported = |w: &mut World| -> Vec<EntityId> {
        w.events
            .drain()
            .into_iter()
            .filter_map(|event| match event {
                WorldEvent::OccupantUnsupported { occupant, .. } => Some(occupant),
                _ => None,
            })
            .collect()
    };

    // The slab is destroyed under the one standing on it.
    let blow = impact(&w, 250.0e3, 1.0e6);
    w.ctx()
        .edit(someone())
        .damage(air, Layer::Floor, blow)
        .unwrap();
    assert_eq!(unsupported(&mut w), [on_slab]);

    // Half of what holds the wide one goes: it is still held. Then the
    // other half.
    w.ctx()
        .edit(someone())
        .damage(left, Layer::Floor, blow)
        .unwrap();
    assert_eq!(unsupported(&mut w), []);
    w.ctx()
        .edit(someone())
        .damage(right, Layer::Floor, blow)
        .unwrap();
    assert_eq!(unsupported(&mut w), [wide]);

    // The grass under the one on the ground is destroyed: the rock
    // below still holds them up.
    let scrape = impact(&w, 1.0e6, 1.0e9);
    let outcome = w
        .ctx()
        .edit(someone())
        .damage(ground, Layer::Floor, scrape)
        .unwrap();
    assert_eq!(outcome.result, DamageResult::Destroyed);
    assert_eq!(unsupported(&mut w), []);

    // The ramp is taken away from under the one at its top.
    let under = top.offset(clash_core::IVec3::new(0, 0, -1));
    w.ctx()
        .edit(someone())
        .set_block(under, BlockState::NONE)
        .unwrap();
    assert_eq!(unsupported(&mut w), [on_ramp]);
}
