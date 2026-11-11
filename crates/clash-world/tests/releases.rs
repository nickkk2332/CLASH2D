//! Tests for W15: energy releases (02 §13).
//!
//! `car_bomb_street` and `beam_through_city` are the bodies of the test
//! scripts of those names. The scripts themselves are owed: the script
//! runner does not exist yet (W11).

mod common;
#[path = "common/wreck.rs"]
mod wreck;

use std::sync::Arc;

use clash_core::{EntityId, Hash128, Real, Tick, codec};
use clash_time::Timeline;
use clash_world::test_support::load_column;
use clash_world::{
    BlockState, BodyOwner, CauseKind, ColumnBuilder, Cosmos, DamageResult, EnergyRelease,
    EventSink, Footprint, Layer, Location, Occupant, OccupantFlags, OccupantKind, ReleaseShape,
    TractPos, WORLD_MIN_INTENSITY, WorldActivity, WorldDefs, WorldError, WorldEvent,
};
use wreck::{World, at, build_tower, someone};

fn r(value: f64) -> Real {
    Real::of(value)
}

/// A world of `core:testland`: rock with grass on it in the tracts of a
/// square so many tracts a side, and whatever `build` adds to each.
fn world_of(
    defs: &Arc<WorldDefs>,
    side: i32,
    mut build: impl FnMut(TractPos, &mut ColumnBuilder<'_>),
) -> World {
    let mut timeline = Timeline::new();
    let mut cosmos = Cosmos::new(Arc::clone(defs));
    let def = defs.realms().lookup("core:testland").unwrap();
    let realm = cosmos.create_realm(&mut timeline, def, 7).unwrap();
    let grid = cosmos.realm(realm).unwrap().base_grid;
    let rock = defs.block_state("core:solid", None).unwrap();
    let grass = defs.floor_state("core:grass", None).unwrap();
    for y in 0..side {
        for x in 0..side {
            let tract = TractPos::new(x, y);
            let mut builder = ColumnBuilder::new(defs, rock, BlockState::NONE, 0);
            builder.fill_floor(0, grass);
            build(tract, &mut builder);
            load_column(&mut cosmos, grid, tract, builder.build()).unwrap();
        }
    }
    World {
        defs: Arc::clone(defs),
        timeline,
        cosmos,
        events: EventSink::new(),
        grid,
    }
}

/// The intensity 02 §13.1 gives at a distance, with nothing in the way.
fn open_intensity(energy: f64, distance: f64) -> f64 {
    let r = distance.max(0.5);
    energy / (4.0 / 3.0 * std::f64::consts::PI * r * r * r)
}

fn blast(w: &World, x: i32, y: i32, z: i32, energy: f64) -> EnergyRelease {
    EnergyRelease::sphere(
        w.loc(x, y, z),
        w.damage_type("core:blast"),
        r(energy),
        someone(),
    )
}

/// Puts someone in a cell. With no mass they are exposed and never
/// thrown.
fn stand(w: &mut World, id: u32, x: i32, y: i32, z: i32, mass: f64) -> EntityId {
    let who = EntityId::new(id, 0);
    let loc = w.loc(x, y, z);
    let occupant = Occupant::new(who, OccupantKind::Creature)
        .with_mass(r(mass))
        .with_flags(OccupantFlags::COLLIDES);
    w.ctx()
        .place(occupant, loc, Footprint::SINGLE, someone())
        .unwrap();
    who
}

/// Every exposure of an occupant: when, the intensity and the energy.
fn exposures(w: &World, who: EntityId) -> Vec<(Tick, f64, f64)> {
    w.events
        .events()
        .iter()
        .filter_map(|event| match event {
            WorldEvent::OccupantExposed {
                occupant,
                tick,
                intensity,
                energy,
                ..
            } if *occupant == who => Some((*tick, intensity.get(), energy.get())),
            _ => None,
        })
        .collect()
}

/// What `EnergyReleased` said of each release that has ended.
struct Summary {
    energy: f64,
    absorbed: f64,
    reach: f64,
    cells: u32,
    cut_off: bool,
    elements: u32,
    mass: f64,
    exposed: u32,
    thrown: u32,
    instigator: Option<EntityId>,
}

fn summaries(w: &World) -> Vec<Summary> {
    w.events
        .events()
        .iter()
        .filter_map(|event| match event {
            WorldEvent::EnergyReleased {
                energy,
                absorbed,
                reach,
                cells,
                cut_off,
                tallies,
                cause,
                ..
            } => {
                assert_eq!(cause.kind, CauseKind::Process);
                assert_eq!(tallies.len(), 1);
                let tally = tallies[0];
                assert_eq!(tally.absorbed, *absorbed);
                Some(Summary {
                    energy: energy.get(),
                    absorbed: absorbed.get(),
                    reach: reach.get(),
                    cells: *cells,
                    cut_off: *cut_off,
                    elements: tally.elements,
                    mass: tally.mass.get(),
                    exposed: tally.exposed,
                    thrown: tally.thrown,
                    instigator: tally.instigator,
                })
            }
            _ => None,
        })
        .collect()
}

/// Every element a blast destroyed, by what the element events say:
/// where, which, what it took and what passed. What fragments and
/// falls broke afterwards is not the blast's own doing.
fn destroyed(w: &World) -> Vec<(Location, Layer, f64, f64)> {
    let blast = w.damage_type("core:blast");
    w.events
        .events()
        .iter()
        .filter_map(|event| match event {
            WorldEvent::ElementDestroyed {
                loc,
                layer,
                absorbed,
                passed,
                how: clash_world::Destruction::Damage(kind),
                ..
            } if *kind == blast => Some((*loc, *layer, absorbed.get(), passed.get())),
            _ => None,
        })
        .collect()
}

// ---- The intensity law -----------------------------------------------------

#[test]
fn a_blast_in_the_open_falls_off_with_the_cube_of_distance() {
    let defs = common::test_defs();
    let mut w = world_of(&defs, 1, |_, _| {});
    // High in the air, where nothing is in the way: a grenade's worth.
    let energy = 8.0e5;
    let one = stand(&mut w, 20, 11, 10, 20, 0.0);
    let two = stand(&mut w, 21, 10, 12, 20, 0.0);
    let three = stand(&mut w, 22, 7, 10, 20, 0.0);
    let four = stand(&mut w, 23, 10, 6, 20, 0.0);
    let above = stand(&mut w, 24, 10, 10, 21, 0.0);
    let spec = blast(&w, 10, 10, 20, energy);
    w.ctx().release(spec).unwrap();
    // It is announced at once, and nothing has happened yet.
    assert!(matches!(
        w.events.events(),
        [WorldEvent::ReleaseStarted { energy: let_go, reach, .. }]
            if let_go.get() == energy && (reach.get() - 3.37).abs() < 0.01
    ));
    assert_eq!(w.cosmos.releases().count(), 1);
    w.run_out();

    for (who, distance) in [(one, 1.0), (two, 2.0), (three, 3.0), (above, 3.0)] {
        let felt = exposures(&w, who);
        assert_eq!(felt.len(), 1, "at {distance} m");
        let (tick, intensity, carried) = felt[0];
        let expected = open_intensity(energy, distance);
        assert!(
            (intensity - expected).abs() < expected * 1.0e-9,
            "{intensity}"
        );
        // The energy the wave carries through a cell of three cubic
        // metres.
        assert!((carried - 3.0 * expected).abs() < expected * 1.0e-8);
        // The front moves at the speed of sound: a shell a wake.
        let shell = (1.0e6 / 343.0_f64).round() as i64;
        assert_eq!(tick.as_micros(), shell * distance as i64);
    }
    // Twice as far, an eighth as much.
    let near = exposures(&w, one)[0].1;
    let far = exposures(&w, two)[0].1;
    assert!((near / far - 8.0).abs() < 1.0e-9);
    // Beyond where it falls to 5 kPa it is not felt at all.
    assert!(open_intensity(energy, 4.0) < WORLD_MIN_INTENSITY.get());
    assert!(exposures(&w, four).is_empty());

    let told = summaries(&w);
    assert_eq!(told.len(), 1);
    let sum = &told[0];
    assert_eq!(
        (sum.energy, sum.absorbed, sum.cut_off),
        (energy, 0.0, false)
    );
    assert!((sum.reach - 3.37).abs() < 0.01);
    assert_eq!((sum.exposed, sum.thrown, sum.elements), (4, 0, 0));
    assert_eq!(sum.instigator, someone().instigator);
    assert!(sum.cells > 40 && sum.cells < 120, "{}", sum.cells);
    assert!(w.cosmos.processes().is_idle());
}

#[test]
fn someone_who_leaves_before_the_front_arrives_is_not_exposed() {
    let defs = common::test_defs();
    let mut w = world_of(&defs, 1, |_, _| {});
    let quick = stand(&mut w, 20, 13, 10, 20, 0.0);
    let slow = stand(&mut w, 21, 10, 13, 20, 0.0);
    let spec = blast(&w, 10, 10, 20, 8.0e5);
    w.ctx().release(spec).unwrap();
    // The front takes nine milliseconds to cover three metres.
    w.run_until(Tick(4_000));
    assert!(exposures(&w, quick).is_empty());
    let away = w.loc(20, 10, 20);
    w.ctx().relocate(quick, away, someone()).unwrap();
    w.run_out();
    assert!(exposures(&w, quick).is_empty());
    assert_eq!(exposures(&w, slow).len(), 1);
}

#[test]
fn walls_are_affected_out_to_the_radius_the_table_gives() {
    // A car bomb's worth in the open: 02 §13.1 puts the example wall's
    // threshold of 70 kPa at 19 m.
    let defs = common::test_defs();
    let wall = defs.block_state("core:wall", None).unwrap();
    let energy = 2.0e9;
    let centre = (48, 48);
    // Single wall cells round it, none behind another.
    let mut placed = Vec::new();
    for (step, distance) in (12..=26).enumerate() {
        let angle = r(step as f64 * 0.41);
        let x = (r(f64::from(distance)) * angle.cos()).round().to_i32();
        let y = (r(f64::from(distance)) * angle.sin()).round().to_i32();
        placed.push((centre.0 + x, centre.1 + y));
    }
    let mut w = world_of(&defs, 3, |tract, builder| {
        // Bare rock: no turf for the blast to throw about.
        builder.fill_floor(0, clash_world::FloorState::NONE);
        for (x, y) in &placed {
            if at(*x, *y, 0).tract() == tract {
                builder.set_block(at(*x, *y, 0), wall);
            }
        }
    });
    let spec = blast(&w, centre.0, centre.1, 0, energy);
    w.ctx().release(spec).unwrap();
    w.run_out();

    let mut furthest_marked = 0.0_f64;
    let mut nearest_whole = f64::INFINITY;
    for (x, y) in &placed {
        let (dx, dy) = (f64::from(x - centre.0), f64::from(y - centre.1));
        let distance = (dx * dx + dy * dy).sqrt();
        let marked = w.block_at(*x, *y, 0) != wall || w.damage_at(*x, *y, 0, Layer::Block) > 0;
        if marked {
            furthest_marked = furthest_marked.max(distance);
        } else {
            nearest_whole = nearest_whole.min(distance);
        }
        // Clear of the edge by a cell, there is no doubt either way.
        if distance < 18.0 {
            assert!(marked, "the wall at {distance} m");
        }
        if distance > 20.0 {
            assert!(!marked, "the wall at {distance} m");
        }
    }
    assert!((furthest_marked - 19.0).abs() <= 1.0, "{furthest_marked}");
    assert!((nearest_whole - 19.0).abs() <= 1.5, "{nearest_whole}");

    // Contract 1: no more is absorbed than was released.
    let told = summaries(&w);
    assert_eq!(told.len(), 1);
    assert!(told[0].absorbed > 0.0 && told[0].absorbed <= energy);
    assert!(!told[0].cut_off);
    assert!((told[0].reach - 45.7).abs() < 0.1);
}

#[test]
fn no_more_is_absorbed_than_was_released() {
    // A small charge with glass all round it. Each pane would take more
    // than its share: together they are offered more than there is.
    let defs = common::test_defs();
    let window = defs.block_state("core:window", None).unwrap();
    let mut w = world_of(&defs, 1, |_, builder| {
        for y in 9..=11 {
            for x in 9..=11 {
                if (x, y) != (10, 10) {
                    builder.set_block(at(x, y, 4), window);
                }
            }
        }
    });
    let energy = 5.0e4;
    let spec = blast(&w, 10, 10, 4, energy);
    w.ctx().release(spec).unwrap();
    w.run_out();
    let broken = destroyed(&w);
    assert!(broken.len() >= 4, "{}", broken.len());
    let offered: f64 = broken
        .iter()
        .map(|(_, _, absorbed, passed)| absorbed + passed)
        .sum();
    let taken: f64 = broken.iter().map(|(_, _, absorbed, _)| *absorbed).sum();
    assert!(offered > energy, "{offered}");
    assert!(taken <= energy + 1.0e-6, "{taken}");
    let told = summaries(&w);
    assert!(told[0].absorbed <= energy + 1.0e-6);
    assert!(told[0].absorbed >= taken - 1.0e-6);
}

// ---- Shadowing -------------------------------------------------------------

/// A wall three cells wide, three metres east of where the charge goes,
/// and someone three metres behind it.
fn behind_a_wall(defs: &Arc<WorldDefs>, walled: bool) -> (World, EntityId) {
    let wall = defs.block_state("core:wall", None).unwrap();
    let mut w = world_of(defs, 1, |_, builder| {
        if walled {
            // Up from the ground, so that it stands.
            for z in 0..=4 {
                for y in 9..=11 {
                    builder.set_block(at(13, y, z), wall);
                }
            }
        }
    });
    let who = stand(&mut w, 20, 16, 10, 4, 0.0);
    (w, who)
}

#[test]
fn a_wall_shields_what_is_behind_it_until_it_is_destroyed() {
    let defs = common::test_defs();
    let wall = defs.block_state("core:wall", None).unwrap();

    // 10 MJ: enough to mark the wall, not to bring it down.
    let weak = 1.0e7;
    let (mut open, exposed) = behind_a_wall(&defs, false);
    let spec = blast(&open, 10, 10, 4, weak);
    open.ctx().release(spec).unwrap();
    open.run_out();
    let felt = exposures(&open, exposed);
    assert_eq!(felt.len(), 1);
    assert!((felt[0].1 - open_intensity(weak, 6.0)).abs() < 1.0);

    let (mut w, sheltered) = behind_a_wall(&defs, true);
    let spec = blast(&w, 10, 10, 4, weak);
    w.ctx().release(spec).unwrap();
    w.run_out();
    // The wall stands, damaged, and nothing reached whoever is behind
    // it (contract 2).
    assert_eq!(w.block_at(13, 10, 4), wall);
    assert!(w.damage_at(13, 10, 4, Layer::Block) > 0);
    assert!(exposures(&w, sheltered).is_empty());
    assert_eq!(summaries(&w)[0].exposed, 0);

    // 1 GJ: the wall goes, and what it did not take passes.
    let strong = 1.0e9;
    let (mut w, behind) = behind_a_wall(&defs, true);
    let spec = blast(&w, 10, 10, 4, strong);
    w.ctx().release(spec).unwrap();
    w.run_out();
    assert_eq!(w.block_at(13, 10, 4), BlockState::NONE);
    let middle = w.loc(13, 10, 4);
    let (_, _, absorbed, passed) = *destroyed(&w)
        .iter()
        .find(|(loc, layer, _, _)| *loc == middle && *layer == Layer::Block)
        .unwrap();
    let got_through = passed / (absorbed + passed);
    assert!(got_through > 0.95 && got_through < 0.98, "{got_through}");
    let felt = exposures(&w, behind);
    assert_eq!(felt.len(), 1);
    let expected = open_intensity(strong, 6.0) * got_through;
    assert!((felt[0].1 - expected).abs() < expected * 1.0e-6, "{felt:?}");
    // The wall's fragments flew away from the blast.
    let thrown = w.events.events().iter().any(|event| {
        matches!(event, WorldEvent::BodyLaunched { owner: BodyOwner::Debris(_), vel, .. }
            if vel[0] > Real::ZERO)
    });
    assert!(thrown);
    let told = summaries(&w);
    assert!(told[0].elements >= 3);
    assert!(told[0].mass >= 3.0 * 1440.0);
    assert!(told[0].absorbed <= strong);
}

// ---- Confinement -----------------------------------------------------------

/// A room three cells square inside walls of steel, under a slab. With
/// `door`, one cell of its east wall is an open door.
fn bunker(defs: &Arc<WorldDefs>, room: bool, door: bool) -> World {
    let steel = defs.materials().lookup("core:steel").unwrap();
    let wall = defs.block_state("core:wall", None).unwrap();
    let wall = BlockState::new(wall.def(), steel, 0);
    let slab = defs.floor_state("core:slab", None).unwrap();
    let open = defs.block_state("core:door#open", None).unwrap();
    world_of(defs, 1, |_, builder| {
        if !room {
            return;
        }
        for y in 8..=12 {
            for x in 8..=12 {
                let edge = x == 8 || x == 12 || y == 8 || y == 12;
                if edge {
                    builder.set_block(at(x, y, 0), wall);
                }
                builder.set_floor(at(x, y, 1), slab);
            }
        }
        if door {
            builder.set_block(at(12, 10, 0), open);
        }
    })
}

#[test]
fn a_blast_in_a_closed_room_is_stronger_than_in_the_open_and_vents_through_a_door() {
    let defs = common::test_defs();
    let energy = 2.3e6;
    let run = |room: bool, door: bool| {
        let mut w = bunker(&defs, room, door);
        let inside = stand(&mut w, 20, 11, 10, 0, 0.0);
        let outside = stand(&mut w, 21, 14, 10, 0, 0.0);
        let spec = blast(&w, 10, 10, 0, energy);
        w.ctx().release(spec).unwrap();
        w.run_out();
        (exposures(&w, inside), exposures(&w, outside), w)
    };

    // In the open: once, with what the distance gives.
    let (inside, outside, _) = run(false, false);
    assert_eq!(inside.len(), 1);
    let direct = inside[0].1;
    let open = open_intensity(energy, 1.0);
    assert!(direct <= open && direct > open * 0.95, "{direct} of {open}");
    assert_eq!(outside.len(), 1);

    // In the closed room: the same, and then what the walls turned
    // back, which fills the room evenly.
    let (inside, outside, w) = run(true, false);
    assert_eq!(inside.len(), 2);
    assert!((inside[0].1 - direct).abs() < direct * 1.0e-6);
    let held = inside[1].1;
    // Nearly all of it, over the room's nine cells of three cubic
    // metres.
    assert!(
        held > 0.9 * energy / 27.0 && held <= energy / 27.0,
        "{held}"
    );
    assert!(inside[1].0 > inside[0].0);
    assert!(inside[0].1 + inside[1].1 > 1.1 * direct);
    // Nothing got out.
    assert!(outside.is_empty());
    // What it held pressed on the walls, which took a mark from it.
    assert!(w.damage_at(12, 9, 0, Layer::Block) > 0);
    assert!(summaries(&w)[0].absorbed <= energy);

    // With the door open the room holds nothing: inside it is as in the
    // open, and the blast is felt outside the door.
    let (inside, outside, _) = run(true, true);
    assert_eq!(inside.len(), 1);
    assert!((inside[0].1 - direct).abs() < direct * 1.0e-6);
    assert_eq!(outside.len(), 1);
    let through = outside[0].1;
    // Nearly what the open gives at four metres: the doorway is a cell wide.
    let open = open_intensity(energy, 4.0);
    assert!(
        through <= open && through > open * 0.8,
        "{through} of {open}"
    );
}

// ---- Occupants ---------------------------------------------------------------

#[test]
fn occupants_in_reach_are_exposed_and_thrown_where_the_wave_moves_them_enough() {
    let defs = common::test_defs();
    let mut w = world_of(&defs, 1, |_, _| {});
    let energy = 1.0e8;
    let near = stand(&mut w, 20, 13, 10, 0, 80.0);
    let far = stand(&mut w, 21, 10, 18, 0, 80.0);
    let unweighed = stand(&mut w, 22, 7, 10, 0, 0.0);
    let spec = blast(&w, 10, 10, 0, energy);
    w.ctx().release(spec).unwrap();
    w.run_until(Tick(100_000));

    // Each is exposed once, with the kind and the doing of the release.
    for who in [near, far, unweighed] {
        assert_eq!(exposures(&w, who).len(), 1);
    }
    let blast_kind = w.damage_type("core:blast");
    assert!(w.events.events().iter().all(|event| match event {
        WorldEvent::OccupantExposed { kind, cause, .. } =>
            *kind == blast_kind
                && cause.kind == CauseKind::Process
                && cause.instigator == someone().instigator,
        _ => true,
    }));
    // Three metres off the wave is at nearly 0.9 MPa. What it holds in
    // a slab of their cross-section a centimetre deep moves 80 kg at
    // over 6 m/s: thrown, straight away from the blast.
    let (_, intensity, carried) = exposures(&w, near)[0];
    // The ground takes a little of what was heading down and out.
    let open = open_intensity(energy, 3.0);
    assert!(
        intensity <= open && intensity > open * 0.94,
        "{intensity} of {open}"
    );
    assert!((carried - 3.0 * intensity).abs() < 1.0e-6 * carried);
    let area = r(0.08).cbrt().powi(2).get();
    let speed = (2.0 * intensity * area * 0.01 / 80.0).sqrt();
    assert!(speed > 6.0 && speed < 6.6, "{speed}");
    let launched: Vec<_> = w
        .events
        .events()
        .iter()
        .filter_map(|event| match event {
            WorldEvent::BodyLaunched {
                owner: BodyOwner::Entity(who),
                vel,
                mass,
                cause,
                ..
            } => Some((*who, *vel, *mass, *cause)),
            _ => None,
        })
        .collect();
    assert_eq!(launched.len(), 1);
    let (who, vel, mass, cause) = launched[0];
    assert_eq!((who, mass), (near, r(80.0)));
    assert!((vel[0].get() - speed).abs() < 1.0e-9 && vel[1] == Real::ZERO);
    assert_eq!(cause.instigator, someone().instigator);
    // Eight metres off the wave moves them at 1.5 m/s: not thrown. And
    // what has no mass the world knows of is not moved at all.
    assert!(w.cosmos.body_of(far).is_none());
    assert_eq!(w.cosmos.occupant(far).unwrap().loc.cell, at(10, 18, 0));
    assert_eq!(w.cosmos.occupant(unweighed).unwrap().loc.cell, at(7, 10, 0));
    let told = summaries(&w);
    assert_eq!((told[0].exposed, told[0].thrown), (3, 1));

    // They fly, come down and are back in the simulation's hands, east
    // of where they stood.
    w.run_out();
    let landed = w.events.events().iter().find_map(|event| match event {
        WorldEvent::BodyLanded {
            owner: BodyOwner::Entity(who),
            loc,
            ..
        } if *who == near => Some(loc.cell),
        _ => None,
    });
    let landed = landed.expect("they came down");
    assert!(landed.x > 13 && landed.y == 10, "{landed:?}");
    assert_eq!(w.cosmos.occupant(near).unwrap().loc.cell, landed);
    assert!(w.cosmos.processes().is_idle());
}

// ---- The budget --------------------------------------------------------------

#[test]
fn a_release_over_budget_is_cut_off_and_says_so() {
    let defs = common::test_defs();
    let run = || {
        let mut w = world_of(&defs, 1, |_, _| {});
        w.cosmos.set_release_max_cells(500);
        let near = stand(&mut w, 20, 15, 10, 20, 0.0);
        let far = stand(&mut w, 21, 10, 20, 20, 0.0);
        let spec = blast(&w, 10, 10, 20, 2.0e9);
        w.ctx().release(spec).unwrap();
        w.run_out();
        (w, near, far)
    };
    let (w, near, far) = run();
    let told = summaries(&w);
    assert_eq!(told.len(), 1);
    // Seven shells hold 480 cells and the eighth would pass 500: it is
    // cut off whole, at seven metres of its forty-six.
    assert!(told[0].cut_off);
    assert_eq!(told[0].reach, 7.0);
    assert!(
        told[0].cells <= 500 && told[0].cells > 400,
        "{}",
        told[0].cells
    );
    assert_eq!(exposures(&w, near).len(), 1);
    assert!(exposures(&w, far).is_empty());
    assert!(w.cosmos.processes().is_idle());
    // The same every time.
    let (again, _, _) = run();
    assert_eq!(again.state_hash(), w.state_hash());
    assert_eq!(
        Hash128::of(&again.events.events().to_vec()),
        Hash128::of(&w.events.events().to_vec())
    );
    assert_eq!(w.cosmos.release_max_cells(), 500);
}

// ---- Refusals ------------------------------------------------------------------

#[test]
fn what_makes_no_sense_is_refused_with_a_reason() {
    let defs = common::test_defs();
    let mut w = world_of(&defs, 1, |_, _| {});
    let good = blast(&w, 10, 10, 0, 1.0e5);
    let refused = |w: &mut World, spec: EnergyRelease| w.ctx().release(spec).unwrap_err();
    for energy in [0.0, -5.0, f64::INFINITY] {
        let spec = EnergyRelease {
            energy: r(energy),
            ..good
        };
        assert!(matches!(refused(&mut w, spec), WorldError::BadValue { .. }));
    }
    let nowhere = EnergyRelease {
        origin: w.loc(10, 500, 0),
        ..good
    };
    assert!(matches!(refused(&mut w, nowhere), WorldError::NotLoaded(_)));
    let still = EnergyRelease {
        speed: Some(Real::ZERO),
        ..good
    };
    assert!(matches!(
        refused(&mut w, still),
        WorldError::BadValue { .. }
    ));
    let aimless = EnergyRelease {
        shape: ReleaseShape::Cone {
            dir: [Real::ZERO; 3],
            half_angle: r(0.5),
        },
        ..good
    };
    assert!(matches!(
        refused(&mut w, aimless),
        WorldError::BadValue { .. }
    ));
    let thin = EnergyRelease {
        shape: ReleaseShape::Beam {
            dir: [r(1.0), Real::ZERO, Real::ZERO],
            radius: Real::ZERO,
            range: r(10.0),
            burst: false,
        },
        ..good
    };
    assert!(matches!(refused(&mut w, thin), WorldError::BadValue { .. }));
    assert!(w.events.is_empty());
    assert!(w.cosmos.processes().is_idle());
}

// ---- Cones and beams -----------------------------------------------------------

#[test]
fn a_cone_reaches_only_what_it_points_at_and_reaches_it_harder() {
    let defs = common::test_defs();
    let mut w = world_of(&defs, 1, |_, _| {});
    let ahead = stand(&mut w, 20, 13, 10, 20, 0.0);
    let behind = stand(&mut w, 21, 7, 10, 20, 0.0);
    let beside = stand(&mut w, 22, 10, 13, 20, 0.0);
    let energy = 8.0e5;
    let mut spec = blast(&w, 10, 10, 20, energy);
    // A quarter of a turn wide: an eighth and a bit of the sphere.
    let half_angle = r(std::f64::consts::FRAC_PI_4);
    spec.shape = ReleaseShape::Cone {
        dir: [r(1.0), Real::ZERO, Real::ZERO],
        half_angle,
    };
    w.ctx().release(spec).unwrap();
    w.run_out();
    let felt = exposures(&w, ahead);
    assert_eq!(felt.len(), 1);
    let share = (1.0 - half_angle.cos().get()) / 2.0;
    let expected = open_intensity(energy, 3.0) / share;
    assert!((felt[0].1 - expected).abs() < expected * 1.0e-9);
    assert!(exposures(&w, behind).is_empty());
    assert!(exposures(&w, beside).is_empty());
}

#[test]
fn beam_through_city() {
    let defs = common::test_defs();
    let wall = defs.block_state("core:wall", None).unwrap();
    let steel = defs.materials().lookup("core:steel").unwrap();
    let column = defs.block_state("core:column", None).unwrap();
    let pier = BlockState::new(column.def(), steel, 0);
    let mut w = world_of(&defs, 1, |_, builder| {
        // Up from the ground, so that they stand.
        for z in 0..=4 {
            for x in [8, 12] {
                for y in 9..=11 {
                    builder.set_block(at(x, y, z), wall);
                }
            }
            builder.set_block(at(18, 10, z), pier);
        }
    });
    let in_the_way = stand(&mut w, 20, 15, 10, 4, 0.0);
    let beside = stand(&mut w, 21, 15, 12, 4, 0.0);
    let sheltered = stand(&mut w, 22, 26, 10, 4, 0.0);
    let energy = 5.0e6;
    let mut spec = blast(&w, 4, 10, 4, energy);
    spec.shape = ReleaseShape::Beam {
        dir: [r(1.0), Real::ZERO, Real::ZERO],
        radius: r(0.5),
        range: r(40.0),
        burst: true,
    };
    w.ctx().release(spec).unwrap();
    w.run_out();

    // It went through both walls, on its axis and nowhere else, paying
    // for each from what it had.
    for x in [8, 12] {
        assert_eq!(w.block_at(x, 10, 4), BlockState::NONE);
        assert_eq!(w.block_at(x, 9, 4), wall);
        assert_eq!(w.block_at(x, 11, 4), wall);
    }
    let broken = destroyed(&w);
    let walls: Vec<_> = broken
        .iter()
        .filter(|(_, layer, _, _)| *layer == Layer::Block)
        .collect();
    assert_eq!(walls.len(), 2);
    // E / (π r² · 1 m), with E what is left after the first wall.
    let first = walls[0].2;
    let felt = exposures(&w, in_the_way);
    assert_eq!(felt.len(), 1);
    let left_then = energy - first - walls[1].2;
    let expected = left_then / (std::f64::consts::PI * 0.25);
    assert!((felt[0].1 - expected).abs() < expected * 1.0e-9, "{felt:?}");
    assert!(exposures(&w, beside).is_empty());
    // The steel pier stopped it. What it had left went off there as a
    // blast: felt by those round the pier, not by whoever it shelters.
    assert_eq!(w.block_at(18, 10, 4), pier);
    let told = summaries(&w);
    assert_eq!(told.len(), 2);
    let (beam, burst) = (&told[0], &told[1]);
    assert_eq!(beam.energy, energy);
    assert_eq!(beam.reach, 15.0);
    assert_eq!(beam.elements, 2);
    assert!((beam.absorbed - (first + walls[1].2)).abs() < 1.0e-6);
    assert!(
        (burst.energy - left_then).abs() < 1.0e-6,
        "{}",
        burst.energy
    );
    assert!(beam.absorbed + burst.absorbed <= energy);
    assert_eq!(exposures(&w, in_the_way).len(), 1);
    assert!(exposures(&w, sheltered).is_empty());
    let started = w
        .events
        .events()
        .iter()
        .filter(|event| matches!(event, WorldEvent::ReleaseStarted { .. }))
        .count();
    assert_eq!(started, 2);
}

// ---- A charge against a building ---------------------------------------------

/// A small house of two storeys beside a street, a car bomb's little
/// brother against its wall, and people about.
fn street(defs: &Arc<WorldDefs>) -> World {
    let mut w = world_of(defs, 2, |tract, builder| {
        if tract == TractPos::new(0, 0) {
            build_tower(builder, defs, (12, 8), 7, 2, false);
        }
    });
    stand(&mut w, 20, 8, 11, 0, 80.0);
    stand(&mut w, 21, 15, 11, 0, 80.0);
    stand(&mut w, 22, 14, 11, 1, 80.0);
    let spec = blast(&w, 10, 11, 0, 1.0e8);
    w.ctx().release(spec).unwrap();
    w
}

#[test]
fn car_bomb_street() {
    let defs = common::test_defs();
    let wall = defs.block_state("core:wall", None).unwrap();
    let mut straight = street(&defs);
    straight.run_out();

    // The wall facing the charge is down, on both storeys.
    let w = &straight;
    for z in 0..2 {
        assert_eq!(w.block_at(12, 11, z), BlockState::NONE, "storey {z}");
    }
    // The far wall, sheltered by the house itself, stands.
    assert_eq!(w.block_at(18, 11, 0), wall);
    let broken = destroyed(w);
    let by_blast = w
        .events
        .events()
        .iter()
        .filter(|event| {
            matches!(event, WorldEvent::ElementDestroyed { how: clash_world::Destruction::Damage(kind), .. }
                if *kind == w.damage_type("core:blast"))
        })
        .count();
    assert!(by_blast >= 6, "{by_blast}");
    // Its fragments flew, away from the charge, and came to rest.
    let debris: Vec<_> = w
        .events
        .events()
        .iter()
        .filter_map(|event| match event {
            WorldEvent::BodyLaunched {
                owner: BodyOwner::Debris(_),
                loc,
                vel,
                ..
            } => Some((*loc, *vel)),
            _ => None,
        })
        .collect();
    assert!(debris.len() >= 6, "{}", debris.len());
    assert!(
        debris
            .iter()
            .filter(|(loc, _)| loc.cell.x == 12)
            .all(|(_, vel)| vel[0] > Real::ZERO)
    );
    assert_eq!(w.cosmos.debris_in_flight(), 0);
    // Whatever gave way with its wall gone has come to rest.
    let collapses = w
        .events
        .events()
        .iter()
        .filter(|event| matches!(event, WorldEvent::CollapseSettled { .. }))
        .count();
    let begun = w
        .events
        .events()
        .iter()
        .filter(|event| matches!(event, WorldEvent::CollapseStarted { .. }))
        .count();
    assert_eq!(collapses, begun);
    // The one in the street was exposed and thrown back. The one behind
    // the far side of the house was sheltered from the front.
    let (street_side, indoors) = (EntityId::new(20, 0), EntityId::new(21, 0));
    assert!(!exposures(w, street_side).is_empty());
    assert!(w.events.events().iter().any(|event| {
        matches!(event, WorldEvent::BodyLaunched { owner: BodyOwner::Entity(who), vel, .. }
            if *who == street_side && vel[0] < Real::ZERO)
    }));
    assert!(!exposures(w, indoors).is_empty());
    // The account of it: who did it, what it broke, and no more taken
    // than was let go (contract 1, invariant W8).
    let told = summaries(w);
    assert_eq!(told.len(), 1);
    let sum = &told[0];
    assert_eq!(sum.instigator, someone().instigator);
    assert_eq!(sum.elements as usize, by_blast);
    assert!(sum.mass > 6.0 * 480.0);
    assert!(sum.exposed >= 2 && sum.thrown >= 1);
    assert!(sum.absorbed <= sum.energy);
    let taken: f64 = broken.iter().map(|(_, _, absorbed, _)| *absorbed).sum();
    assert!(taken <= sum.energy);
    assert!(w.cosmos.processes().is_idle());
    assert!(w.cosmos.stability_requests().is_empty());

    // The same, saved while the front is still spreading.
    let end = straight.timeline.now();
    let mut first = street(&defs);
    first.run_until(Tick(12_000));
    assert_eq!(first.cosmos.releases().count(), 1);
    assert!(first.cosmos.bodies().count() > 0);
    let saved_timeline = codec::to_bytes(&first.timeline).unwrap();
    let saved_activity = codec::to_bytes(first.cosmos.processes()).unwrap();
    let timeline: Timeline = codec::from_bytes(&saved_timeline).unwrap();
    let activity: WorldActivity = codec::from_bytes(&saved_activity).unwrap();
    assert_eq!(&activity, first.cosmos.processes());
    // The cells are copied: tract blobs are the persistence package's.
    let mut cosmos = first.cosmos.clone();
    cosmos.restore_activity(WorldActivity::new());
    assert_ne!(Hash128::of(&cosmos), Hash128::of(&first.cosmos));
    cosmos.restore_activity(activity);
    let mut second = World {
        defs: Arc::clone(&first.defs),
        timeline,
        cosmos,
        events: EventSink::new(),
        grid: first.grid,
    };
    assert_eq!(second.state_hash(), first.state_hash());
    second.run_until(end);
    assert_eq!(second.state_hash(), straight.state_hash());
    let mut told = first.events.drain();
    told.extend(second.events.drain());
    assert_eq!(Hash128::of(&told), Hash128::of(&straight.events.drain()));
}

// ---- Cost ----------------------------------------------------------------------

/// Prints what a blast in a city block costs. Run it with
/// `cargo test -p clash-world --release --test releases -- --ignored --nocapture`.
#[test]
#[ignore = "a timing, not a check"]
#[allow(
    clippy::disallowed_types,
    clippy::disallowed_methods,
    reason = "a timing test reads the wall clock; nothing in the simulation does"
)]
fn timing_of_a_blast_in_a_city_block() {
    let defs = common::test_defs();
    // Nine buildings of five storeys, eleven cells square, on a grid of
    // streets five cells wide: a block of 96 m.
    let block = |defs: &Arc<WorldDefs>| {
        world_of(defs, 3, |tract, builder| {
            let origin = tract.origin(0);
            for by in 0..2 {
                for bx in 0..2 {
                    let corner = (origin.x + 3 + bx * 16, origin.y + 3 + by * 16);
                    build_tower(builder, defs, corner, 11, 5, true);
                }
            }
        })
    };
    let mut w = block(&defs);
    let spec = blast(&w, 49, 49, 0, 2.0e9);
    let began = std::time::Instant::now();
    w.ctx().release(spec).unwrap();
    // The front alone: until the release has ended.
    while w.cosmos.releases().count() > 0 {
        let Some(due) = w.timeline.peek() else { break };
        w.run_until(due);
    }
    let front = began.elapsed().as_secs_f64();
    let told = summaries(&w);
    let sum = &told[0];
    println!(
        "a 2 GJ blast in a city block: the front {:.1} ms over {} cells ({:.0} ns a cell), {} elements destroyed, {:.0} t; {} fragments thrown",
        front * 1.0e3,
        sum.cells,
        front * 1.0e9 / f64::from(sum.cells),
        sum.elements,
        sum.mass / 1000.0,
        w.events
            .events()
            .iter()
            .filter(|event| matches!(event, WorldEvent::BodyLaunched { .. }))
            .count(),
    );
    w.events.clear();
    let began = std::time::Instant::now();
    w.run_out();
    println!(
        "what followed (collapse and debris): {:.1} ms",
        began.elapsed().as_secs_f64() * 1.0e3
    );
    assert!(w.cosmos.processes().is_idle());
    let _ = DamageResult::Destroyed;
}
