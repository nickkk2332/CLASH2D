//! Minimal devices (02 §5.5; 16 §4.1 to §4.3; package E12) against the
//! real simulation: lockers that cost nothing until used, promotion when
//! a state leaves the default or something goes inside, demotion when
//! they are plain again, contents held inside, and what a destroyed
//! locker spills.

mod common;

use clash_core::{EntityId, Real};
use clash_sim::component::Shape;
use clash_sim::device::{self, Device};
use clash_sim::item;
use clash_sim::{Place, Sim, SimEvent, Subject};
use clash_world::{Cause, CellPos, EnergyRelease, FixtureState, Footprint, Layer, Location};

use common::{GRID, at, creature, names, on, spawn};

/// A locker that becomes a device, and a crate whose device holds
/// little, as test content.
const DEFS: &str = r#"[
  { "type": "device", "id": "test_locker", "name": "locker",
    "holds": { "volume": 0.8, "mass": "300 kg" }, "tags": ["storage"] },
  { "type": "fixture", "id": "test_locker", "material": "steel",
    "height": 2.0, "fill": 0.1, "flags": ["opaque"],
    "states": { "closed": {}, "open": { "remove_flags": ["opaque"] } },
    "device": "test_locker",
    "glyph": { "ch": "]", "fg": "material" } },
  { "type": "fixture", "id": "plain_crate", "material": "wood",
    "height": 1.0, "fill": 0.3,
    "glyph": { "ch": "=", "fg": "material" } },
  { "type": "channel", "id": "arm", "name": "arm" },
  { "type": "item", "id": "test_coin", "name": "coin",
    "material": "steel", "mass": "0.01 kg", "length": "0.02 m",
    "tags": ["money"], "glyph": { "ch": "$", "fg": "yellow" } }
]"#;

fn sim() -> Sim {
    let defs = common::try_defs(DEFS).unwrap_or_else(|error| panic!("{error}"));
    common::sim_over(defs, 5)
}

/// A fixture state as data writes it.
fn fixture(sim: &Sim, written: &str) -> FixtureState {
    sim.truth()
        .cosmos()
        .defs()
        .fixture_state(written, None)
        .unwrap()
}

/// Puts a fixture, written as data writes a state, in a cell.
fn place_fixture(sim: &mut Sim, loc: Location, written: &str) {
    let state = fixture(sim, written);
    place_state(sim, loc, state);
}

fn place_state(sim: &mut Sim, loc: Location, state: FixtureState) {
    sim.world(|ctx| ctx.edit(Cause::DEBUG).set_fixture(loc, state))
        .unwrap();
}

fn delivered(sim: &mut Sim) -> Vec<SimEvent> {
    sim.pump();
    sim.drain_events()
}

fn device_count(sim: &Sim) -> usize {
    sim.components().store::<Device>().len()
}

fn coin(sim: &mut Sim, at: Location) -> EntityId {
    let def = item::def_named(sim, "core:test_coin").unwrap();
    item::create(sim, def, 1, Place::at(at), Cause::DEBUG).unwrap()
}

/// A person with a size: a body of 70 kg and a little under a tenth of
/// a cubic metre.
fn person(sim: &mut Sim, x: i32, y: i32) -> EntityId {
    let id = spawn(sim, creature(), on(x, y));
    let shape = Shape {
        mass: clash_data::Mass::new(Real::of(70.0)),
        volume: clash_data::Volume::new(Real::of(0.07)),
        height: clash_data::Length::new(Real::of(1.7)),
        footprint: Footprint::SINGLE,
        cross_section: Real::of(0.5),
    };
    sim.set_shape(id, shape, Cause::DEBUG).unwrap();
    id
}

#[test]
fn ten_thousand_unused_lockers_create_no_entities() {
    let mut sim = sim();
    let closed = fixture(&sim, "test_locker#closed");
    let mut placed = 0;
    'fill: for z in 0..3 {
        for y in 0..common::SIDE {
            for x in 0..common::SIDE {
                place_state(&mut sim, Location::new(GRID, CellPos::new(x, y, z)), closed);
                placed += 1;
                if placed == 10_000 {
                    break 'fill;
                }
            }
        }
    }
    assert_eq!(placed, 10_000);
    delivered(&mut sim);
    assert_eq!(sim.truth().entities().len(), 0);
    assert_eq!(device_count(&sim), 0);
}

#[test]
fn a_locker_is_a_device_while_open_or_full_and_plain_again_after() {
    let mut sim = sim();
    let here = at(6, 6);
    place_fixture(&mut sim, here, "test_locker#closed");
    delivered(&mut sim);
    assert_eq!(device::device_at(&sim, here, Layer::Fixture), None);

    // Opened, its state leaves the default: it is promoted.
    place_fixture(&mut sim, here, "test_locker#open");
    let events = delivered(&mut sim);
    assert!(
        names(&events).contains(&"device_promoted"),
        "{:?}",
        names(&events)
    );
    let box_ = device::device_at(&sim, here, Layer::Fixture).unwrap();
    assert_eq!(sim.truth().place(box_), Some(Place::at(here)));
    let element = sim.property_named(Subject::Element(here, Layer::Fixture), "device", &[]);
    assert_eq!(element, sim.subject_value(Subject::Entity(box_)));
    let bound = sim.property_named(Subject::Entity(box_), "element", &[]);
    assert_eq!(
        bound,
        sim.subject_value(Subject::Element(here, Layer::Fixture))
    );
    let is_device = |sim: &Sim, id| sim.property_named(Subject::Entity(id), "is_device", &[]);
    assert_eq!(is_device(&sim, box_), clash_expr::Value::Bool(true));
    let plain = sim.property_named(Subject::Element(at(1, 1), Layer::Fixture), "device", &[]);
    assert_eq!(plain, clash_expr::Value::Null);

    // Something is put in; shut, it stays a device for what it holds.
    let c = coin(&mut sim, at(7, 6));
    item::put_in(&mut sim, c, box_, Cause::DEBUG).unwrap();
    assert_eq!(
        item::hold_of(&sim, c).map(|hold| hold.name()),
        Some("stowed")
    );
    place_fixture(&mut sim, here, "test_locker#closed");
    delivered(&mut sim);
    assert_eq!(device::device_at(&sim, here, Layer::Fixture), Some(box_));

    // Taken out, shut and empty: it is demoted.
    item::take_out(&mut sim, c, None, Cause::DEBUG).unwrap();
    let events = delivered(&mut sim);
    assert!(
        names(&events).contains(&"device_demoted"),
        "{:?}",
        names(&events)
    );
    assert_eq!(device::device_at(&sim, here, Layer::Fixture), None);
    assert!(!sim.truth().entities().contains(box_));
    assert_eq!(sim.truth().place(c), Some(Place::at(here)));

    // An element that names no device is never promoted.
    place_fixture(&mut sim, at(9, 9), "plain_crate");
    let refused = device::promote(&mut sim, at(9, 9), Layer::Fixture, Cause::DEBUG).unwrap_err();
    assert_eq!(refused.code, "device.none");
    let refused = device::promote(&mut sim, at(10, 9), Layer::Fixture, Cause::DEBUG).unwrap_err();
    assert_eq!(refused.code, "device.no_element");
}

#[test]
fn someone_hiding_inside_is_out_of_the_world_until_they_come_out() {
    let mut sim = sim();
    let here = at(6, 6);
    place_fixture(&mut sim, here, "test_locker#closed");
    let hider = person(&mut sim, 6, 7);
    let box_ = device::promote(&mut sim, here, Layer::Fixture, Cause::DEBUG).unwrap();
    item::put_in(&mut sim, hider, box_, Cause::DEBUG).unwrap();
    assert_eq!(sim.truth().place(hider), Some(Place::In(box_)));
    assert_eq!(
        item::hold_of(&sim, hider).map(|hold| hold.name()),
        Some("inside")
    );
    assert!(sim.truth().cosmos().occupant(hider).is_none());
    // The device holds the room it says: a second person does not fit.
    let other = person(&mut sim, 5, 7);
    let big = Shape {
        mass: clash_data::Mass::new(Real::of(90.0)),
        volume: clash_data::Volume::new(Real::of(0.9)),
        height: clash_data::Length::new(Real::of(1.9)),
        footprint: Footprint::SINGLE,
        cross_section: Real::of(0.6),
    };
    sim.set_shape(other, big, Cause::DEBUG).unwrap();
    let refused = item::put_in(&mut sim, other, box_, Cause::DEBUG).unwrap_err();
    assert_eq!(refused.code, "item.does_not_fit");
    delivered(&mut sim);

    item::take_out(&mut sim, hider, None, Cause::DEBUG).unwrap();
    delivered(&mut sim);
    assert_eq!(sim.truth().place(hider), Some(Place::at(here)));
    assert!(sim.truth().cosmos().occupant(hider).is_some());
    assert_eq!(device_count(&sim), 0);
}

#[test]
fn a_destroyed_locker_spills_what_it_held_and_exposes_who_hid_there() {
    let mut sim = sim();
    let here = at(6, 6);
    place_fixture(&mut sim, here, "test_locker#closed");
    let hider = person(&mut sim, 6, 7);
    let c = coin(&mut sim, at(6, 7));
    let box_ = device::promote(&mut sim, here, Layer::Fixture, Cause::DEBUG).unwrap();
    item::put_in(&mut sim, c, box_, Cause::DEBUG).unwrap();
    item::put_in(&mut sim, hider, box_, Cause::DEBUG).unwrap();
    delivered(&mut sim);

    // A blast beside it: the world destroys the fixture, and the
    // simulation deals with the device through the world's events.
    let blast = sim
        .truth()
        .cosmos()
        .defs()
        .damage_types()
        .lookup("core:blast")
        .unwrap();
    let release = EnergyRelease::sphere(here, blast, Real::of(5.0e7), Cause::DEBUG);
    sim.world(|ctx| ctx.release(release)).unwrap();
    let until = sim.now() + clash_core::Dur::from_millis(200);
    sim.advance(clash_sim::AdvanceLimit::Until(until));
    let events = delivered(&mut sim);
    let seen = names(&events);
    let destroyed = seen.iter().position(|name| *name == "element_destroyed");
    let spilled = seen.iter().position(|name| *name == "device_spilled");
    assert!(destroyed.is_some() && spilled.is_some(), "{seen:?}");
    assert!(destroyed < spilled);
    assert!(!sim.truth().entities().contains(box_));
    for thing in [hider, c] {
        assert!(
            matches!(sim.truth().place(thing), Some(Place::At { .. })),
            "{thing} is not out in the world"
        );
    }
    assert!(sim.truth().cosmos().occupant(hider).is_some());
}

#[test]
fn a_device_and_what_it_holds_survive_a_save() {
    let mut sim = sim();
    let here = at(6, 6);
    place_fixture(&mut sim, here, "test_locker#open");
    delivered(&mut sim);
    let box_ = device::device_at(&sim, here, Layer::Fixture).unwrap();
    let hider = person(&mut sim, 6, 7);
    item::put_in(&mut sim, hider, box_, Cause::DEBUG).unwrap();
    delivered(&mut sim);

    let (mut loaded, unplaced) = common::save_and_load(&sim);
    assert!(unplaced.is_empty());
    assert_eq!(loaded.state_hash(), sim.state_hash());
    assert_eq!(device::device_at(&loaded, here, Layer::Fixture), Some(box_));
    for run in [&mut sim, &mut loaded] {
        place_fixture(run, here, "test_locker#closed");
        item::take_out(run, hider, None, Cause::DEBUG).unwrap();
        delivered(run);
    }
    assert_eq!(loaded.state_hash(), sim.state_hash());
    assert_eq!(device::device_at(&loaded, here, Layer::Fixture), None);
}
