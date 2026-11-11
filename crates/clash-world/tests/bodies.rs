//! Tests for W14: bodies in flight (02 §15).
//!
//! `three_buildings`, `time_stop_street` and `over_the_horizon` are the
//! bodies of the test scripts of those names. The scripts themselves are
//! owed: the script runner does not exist yet (W11).

mod common;
#[path = "common/wreck.rs"]
mod wreck;

use std::sync::Arc;

use clash_core::{Dur, EntityId, Hash128, Real, Tick, codec};
use clash_time::Timeline;
use clash_world::test_support::load_column;
use clash_world::{
    AmbientOverride, AreaShape, AreaSpec, BODY_MAX_STEP, BlockState, Body, BodyId, BodyOwner,
    CauseKind, CellBox, ColumnBuilder, Cosmos, DEBRIS_MAX_BODIES, DEBRIS_PER_ELEMENT, DamagePacket,
    DamageResult, Drift, Drive, EventSink, Footprint, GenCtx, GeneratorRegistry, GeneratorRequest,
    Layer, MemoryStore, Occupant, OccupantFlags, OccupantKind, PendingEdit, PlaneId, PlaneMask,
    Projectile, REST_SPEED, RealizedTract, RealmInfo, RegionPos, RegionRecord, RegionSurvey,
    RegionSurveyResult, RegionSurveyor, TicketReason, TractIo, TractPos, TractRealizer,
    TractRecord, TractRecords3x3, WorldActivity, WorldDefs, WorldError, WorldEvent, WorldGenerator,
};
use wreck::{World, at, someone, world_from};

fn r(value: f64) -> Real {
    Real::of(value)
}

fn v(x: f64, y: f64, z: f64) -> [Real; 3] {
    [r(x), r(y), r(z)]
}

/// The definitions of `mods/core`, with an airless realm that has
/// weight, and a fixture with a striking surface.
fn defs() -> Arc<WorldDefs> {
    let mut raw = common::test_raw();
    common::add_json(
        &mut raw,
        "t",
        r#"[
            { "type": "realm", "id": "airless",
              "size_regions": [1, 1], "wrap": [], "z_range": [-16, 400],
              "generator": "plain",
              "fill": { "below": "core:solid@core:granite", "above": null },
              "strata": [ { "from_z": -16, "gravity": 9.81, "atmosphere": null } ] },
            { "type": "fixture", "id": "spikes", "material": "core:steel",
              "height": 0.5, "fill": 0.05,
              "contact": { "surface": { "kind": "core:pierce", "area": 3.0e-5, "give": 0.003,
                                        "span": 0.02 },
                           "energy": 60 } },
            { "type": "material", "id": "ice", "density": 900, "toughness": 1.0e5,
              "friction": 0.05, "rubble": "core:rubble" },
            { "type": "floor", "id": "rink", "material": "ice", "thickness": 0.1 }
        ]"#,
        "the definitions of the body tests",
    );
    Arc::new(WorldDefs::build(raw).unwrap_or_else(|errors| panic!("{errors:?}")))
}

/// A world in a realm of `defs`: rock below storey 0 with grass on it
/// in each of the tracts named, and whatever `build` adds.
fn world_in(
    defs: &Arc<WorldDefs>,
    realm: &str,
    tracts: &[(i32, i32)],
    mut build: impl FnMut(TractPos, &mut ColumnBuilder<'_>),
) -> World {
    let mut timeline = Timeline::new();
    let mut cosmos = Cosmos::new(Arc::clone(defs));
    let def = defs.realms().lookup(realm).unwrap();
    let realm = cosmos.create_realm(&mut timeline, def, 7).unwrap();
    let grid = cosmos.realm(realm).unwrap().base_grid;
    let rock = defs.block_state("core:solid", None).unwrap();
    let grass = defs.floor_state("core:grass", None).unwrap();
    for (x, y) in tracts {
        let tract = TractPos::new(*x, *y);
        let mut builder = ColumnBuilder::new(defs, rock, BlockState::NONE, 0);
        builder.fill_floor(0, grass);
        build(tract, &mut builder);
        load_column(&mut cosmos, grid, tract, builder.build()).unwrap();
    }
    World {
        defs: Arc::clone(defs),
        timeline,
        cosmos,
        events: EventSink::new(),
        grid,
    }
}

fn thrown() -> EntityId {
    EntityId::new(9, 0)
}

/// Something of a mass, thrown by the hero.
fn lump(w: &World, pos: [Real; 3], vel: [Real; 3], mass: f64) -> Body {
    Body::of_mass(
        BodyOwner::Entity(thrown()),
        w.grid,
        pos,
        vel,
        r(mass),
        someone(),
    )
}

fn events_where(w: &World, wanted: impl Fn(&WorldEvent) -> bool) -> Vec<&WorldEvent> {
    w.events
        .events()
        .iter()
        .filter(|event| wanted(event))
        .collect()
}

fn impacts(w: &World) -> Vec<&WorldEvent> {
    events_where(w, |event| matches!(event, WorldEvent::BodyImpact { .. }))
}

fn landings(w: &World) -> Vec<&WorldEvent> {
    events_where(w, |event| matches!(event, WorldEvent::BodyLanded { .. }))
}

/// Where and when a body first struck a floor.
fn first_floor_impact(w: &World) -> ([Real; 3], Tick) {
    impacts(w)
        .iter()
        .find_map(|event| match event {
            WorldEvent::BodyImpact {
                layer: Layer::Floor,
                pos,
                tick,
                ..
            } => Some((*pos, *tick)),
            _ => None,
        })
        .expect("it struck a floor")
}

/// Where the one who was thrown came to rest.
fn landed_at(w: &World) -> [Real; 3] {
    landings(w)
        .iter()
        .find_map(|event| match event {
            WorldEvent::BodyLanded {
                owner: BodyOwner::Entity(_),
                pos,
                ..
            } => Some(*pos),
            _ => None,
        })
        .expect("it landed")
}

fn close(a: Real, b: f64, within: f64) -> bool {
    (a.get() - b).abs() <= within
}

// ---- Flight ------------------------------------------------------------

/// A lump thrown up and across where there is weight and no air.
fn throw_in_vacuum(defs: &Arc<WorldDefs>) -> World {
    let mut w = world_in(defs, "t:airless", &[(0, 0), (1, 0)], |_, _| {});
    let body = lump(&w, v(5.5, 16.5, 1.5), v(12.0, 0.0, 9.0), 2.0);
    w.ctx().launch(body).unwrap();
    w
}

#[test]
fn a_thrown_body_lands_where_the_parabola_says_however_time_is_sliced() {
    let defs = defs();
    let mut straight = throw_in_vacuum(&defs);
    straight.run_out();

    // z(t) = 1.5 + 9 t − ½ g t² meets the ground at t, 12 m/s across.
    let g = 9.81_f64;
    let t = (9.0 + (81.0 + 2.0 * g * 1.5).sqrt()) / g;
    let (pos, tick) = first_floor_impact(&straight);
    assert!(close(pos[0], 5.5 + 12.0 * t, 0.01), "{pos:?} at {t}");
    assert!(close(pos[1], 16.5, 1.0e-9));
    assert!(close(pos[2], 0.0, 1.0e-9));
    // It is dealt with at the first wake after it struck: within a step.
    let late = tick.as_micros() - (t * 1.0e6) as i64;
    assert!((0..=BODY_MAX_STEP.as_micros()).contains(&late), "{late}");

    // It does not bounce: it slides, held back by the ground, and stops
    // v² / 2μg further on.
    let rest = landed_at(&straight);
    let slide = 12.0 * 12.0 / (2.0 * 0.5 * g);
    assert!(close(rest[0], 5.5 + 12.0 * t + slide, 0.1), "{rest:?}");
    assert_eq!(straight.cosmos.bodies().count(), 0);
    assert!(straight.cosmos.processes().is_idle());
    assert!(straight.cosmos.tickets().is_empty());

    // The same flight, with time advanced in slices of seven
    // milliseconds: nothing differs.
    let mut sliced = throw_in_vacuum(&defs);
    while !sliced.cosmos.processes().is_idle() {
        sliced.run_for(Dur::from_millis(7));
    }
    let end = Tick(10_000_000);
    straight.run_until(end);
    sliced.run_until(end);
    assert_eq!(sliced.state_hash(), straight.state_hash());
    assert_eq!(
        Hash128::of(&sliced.events.drain()),
        Hash128::of(&straight.events.drain())
    );
}

#[test]
fn in_air_a_falling_body_reaches_its_terminal_speed() {
    let defs = common::test_defs();
    let mut w = world_from(Arc::clone(&defs), |_| {});
    let mut body = lump(&w, v(10.5, 10.5, 240.0), v(0.0, 0.0, 0.0), 2.0);
    body.area = r(0.05);
    body.drag = r(1.0);
    let id = w.ctx().launch(body).unwrap();
    w.run_until(Tick(9_000_000));
    // ½ ρ C A v² = m g.
    let terminal = (2.0 * 2.0 * 9.81 / (1.225 * 0.05_f64)).sqrt();
    let falling = w.cosmos.body(id).unwrap();
    assert!(
        close(-falling.vel[2], terminal, terminal * 0.01),
        "{falling:?}"
    );
    assert!(falling.pos[2] > r(30.0) && falling.pos[2] < r(80.0));
}

#[test]
fn a_surface_holds_back_what_slides_on_it_by_its_friction() {
    let defs = defs();
    let rink = defs.floor_state("t:rink", None).unwrap();
    let slide_on = |ice: bool| {
        let mut w = world_in(&defs, "t:airless", &[(0, 0), (1, 0)], |_, builder| {
            if ice {
                builder.fill_floor(0, rink);
            }
        });
        // Set down already moving, a hand's breadth up.
        let body = lump(&w, v(5.5, 16.5, 0.05), v(6.0, 0.0, 0.0), 2.0);
        w.ctx().launch(body).unwrap();
        w.run_out();
        landed_at(&w)[0].get() - 5.5
    };
    // v² / 2μg: grass states no friction and takes the default, a half.
    let (grass, ice) = (slide_on(false), slide_on(true));
    assert!((grass - 36.0 / 9.81).abs() < 0.7, "{grass}");
    assert!((ice - 36.0 / (2.0 * 0.05 * 9.81)).abs() < 1.5, "{ice}");
}

// ---- Impacts -------------------------------------------------------------

/// Three walls across the way east, four cells apart.
fn three_walls(defs: &Arc<WorldDefs>) -> World {
    let wall = defs.block_state("core:wall", None).unwrap();
    world_from(Arc::clone(defs), |builder| {
        for x in [10, 14, 18] {
            for y in 14..=18 {
                builder.set_block(at(x, y, 0), wall);
            }
        }
    })
}

/// Puts the one who is thrown in the world and throws them east.
fn throw_someone(w: &mut World, speed: f64) -> BodyId {
    let start = w.loc(5, 16, 0);
    let who = Occupant::new(thrown(), OccupantKind::Creature)
        .with_mass(r(80.0))
        .with_flags(OccupantFlags::COLLIDES);
    w.ctx()
        .place(who, start, Footprint::SINGLE, someone())
        .unwrap();
    let body = lump(w, v(5.5, 16.5, 1.5), v(speed, 0.0, 0.0), 80.0);
    w.ctx().launch(body).unwrap()
}

#[test]
fn three_buildings() {
    let defs = common::test_defs();
    let wall = defs.block_state("core:wall", None).unwrap();

    // At 600 m/s: 14.4 MJ, far above what a wall can take.
    let mut w = three_walls(&defs);
    let id = throw_someone(&mut w, 600.0);
    w.run_until(Tick(40_000));

    for x in [10, 14, 18] {
        assert_eq!(w.block_at(x, 16, 0), BlockState::NONE, "the wall at {x}");
        assert_eq!(w.block_at(x, 15, 0), wall);
    }
    let struck = impacts(&w);
    assert_eq!(struck.len(), 3);
    let start = 0.5 * 80.0 * 600.0 * 600.0;
    let mut taken = 0.0;
    let mut last = start;
    for event in &struck {
        let WorldEvent::BodyImpact {
            loc,
            energy,
            result,
            cause,
            layer,
            ..
        } = event
        else {
            unreachable!()
        };
        assert_eq!((*result, *layer), (DamageResult::Destroyed, Layer::Block));
        // The wall's destruction is the thrown one's doing, on behalf of
        // whoever threw them (02 §7.3).
        assert_eq!(cause.actor, Some(thrown()));
        assert_eq!(cause.instigator, someone().instigator);
        assert_eq!(cause.kind, CauseKind::Process);
        let destroyed = w.events.events().iter().find_map(|event| match event {
            WorldEvent::ElementDestroyed {
                loc: there,
                absorbed,
                passed,
                cause: by,
                ..
            } if there == loc => Some((*absorbed, *passed, *by)),
            _ => None,
        });
        let (absorbed, passed, by) = destroyed.unwrap();
        assert_eq!(by, *cause);
        // What the wall took and what went on are what was delivered,
        // and the body lost exactly what did not go on with it (W8).
        let delivered = (absorbed + passed).get();
        assert!(delivered <= last, "{delivered} after {last}");
        assert!(((*energy + passed).get() - delivered).abs() < 1.0e-6 * delivered);
        // The wall's 0.6 MJ, and a tenth of the rest in its fragments.
        assert!((absorbed.get() - (0.6e6 + 0.1 * (delivered - 0.6e6))).abs() < 1.0);
        taken += absorbed.get();
        last = passed.get();
    }
    let flying = w.cosmos.body(id).unwrap();
    assert!(flying.pos[0] > r(18.0));
    assert!(flying.kinetic_energy().get() <= last);
    assert!(flying.kinetic_energy().get() + taken <= start);
    // 14.4 MJ less three walls and their fragments leaves about 9 MJ.
    assert!(flying.speed() > r(440.0) && flying.speed() < r(480.0));
    // They are carried along: where the body is, they are.
    assert_eq!(w.cosmos.body_of(thrown()), Some(id));
    let carried = w.cosmos.occupant(thrown()).unwrap().loc;
    assert_eq!(carried.cell, flying.cell());
    let moves = events_where(
        &w,
        |event| matches!(event, WorldEvent::OccupantDisplaced { occupant, .. } if *occupant == thrown()),
    );
    assert!(moves.len() >= 13, "{}", moves.len());
    // Fragments of the walls fly on after them.
    assert!(w.cosmos.debris_in_flight() > 0);

    // At 20 m/s: 16 kJ, spread too thin to mark the wall. They stop
    // against it and drop.
    let mut w = three_walls(&defs);
    throw_someone(&mut w, 20.0);
    w.run_out();
    assert_eq!(w.block_at(10, 16, 0), wall);
    assert_eq!(w.damage_at(10, 16, 0, Layer::Block), 0);
    let struck = impacts(&w);
    let WorldEvent::BodyImpact {
        loc,
        energy,
        result,
        layer,
        ..
    } = struck[0]
    else {
        unreachable!()
    };
    assert_eq!((loc.cell, *layer), (at(10, 16, 0), Layer::Block));
    assert_eq!(*result, DamageResult::Unaffected);
    assert!(close(*energy, 16.0e3, 16.0e3 * 0.03), "{energy:?}");
    let rest = landed_at(&w);
    assert!(rest[0] < r(10.0) && rest[0] > r(9.9));
    assert_eq!(w.cosmos.occupant(thrown()).unwrap().loc.cell, at(9, 16, 0));
    assert_eq!(w.cosmos.body_of(thrown()), None);
    assert!(w.cosmos.processes().is_idle());
}

#[test]
fn a_body_through_a_window_breaks_it_and_goes_on_with_what_passed() {
    let defs = common::test_defs();
    let window = defs.block_state("core:window", None).unwrap();
    let mut w = world_from(Arc::clone(&defs), |builder| {
        builder.set_block(at(10, 16, 0), window);
    });
    // 5 kg at 40 m/s: 4 kJ against a pane that takes 3 kJ.
    let body = lump(&w, v(8.5, 16.5, 1.5), v(40.0, 0.0, 0.0), 5.0);
    let id = w.ctx().launch(body).unwrap();
    w.run_until(Tick(60_000));

    assert_eq!(w.block_at(10, 16, 0), BlockState::NONE);
    let broke = w.events.events().iter().find_map(|event| match event {
        WorldEvent::ElementDestroyed {
            absorbed, passed, ..
        } => Some((absorbed.get(), passed.get())),
        _ => None,
    });
    let (absorbed, passed) = broke.unwrap();
    let delivered = absorbed + passed;
    // A little less than it was thrown with: the air took some.
    assert!(delivered < 4000.0 && delivered > 3900.0, "{delivered}");
    assert!(
        (passed - 0.9 * (delivered - 3000.0)).abs() < 1.0e-6,
        "{passed}"
    );
    let WorldEvent::BodyImpact { energy, .. } = impacts(&w)[0] else {
        unreachable!()
    };
    assert!((energy.get() - (delivered - passed)).abs() < 1.0e-6);
    // It flies on, slower: with the energy that passed.
    let flying = w.cosmos.body(id).unwrap();
    assert!(flying.pos[0] > r(10.0));
    assert!(
        close(flying.vel[0], (2.0 * passed / 5.0).sqrt(), 0.2),
        "{flying:?}"
    );
}

#[test]
fn what_closes_part_of_a_cell_is_struck_by_its_share() {
    // A pillar closes half its cell. Thin things thrown at it from many
    // places strike it about half the time, and each the same way every
    // time it is tried.
    let defs = common::test_defs();
    let pillar = defs.block_state("core:pillar", None).unwrap();
    let run = || {
        let mut w = world_from(Arc::clone(&defs), |builder| {
            for y in 2..30 {
                builder.set_block(at(10, y, 0), pillar);
            }
        });
        let mut ids = Vec::new();
        for shot in 0..200 {
            let y = 2.5 + f64::from(shot % 28);
            let mut body = lump(&w, v(8.5, y, 1.5), v(30.0, 0.0, 0.0), 0.01);
            body.owner = BodyOwner::Projectile(Projectile {
                kind: w.damage_type("core:pierce"),
                area: r(1.0e-4),
                give: r(0.01),
                tag: shot as u64,
            });
            body.area = r(1.0e-4);
            ids.push(w.ctx().launch(body).unwrap());
        }
        w.run_until(Tick(100_000));
        let struck: Vec<u64> = impacts(&w)
            .iter()
            .filter_map(|event| match event {
                WorldEvent::BodyImpact {
                    owner: BodyOwner::Projectile(projectile),
                    layer: Layer::Block,
                    ..
                } => Some(projectile.tag),
                _ => None,
            })
            .collect();
        struck
    };
    let struck = run();
    assert!(struck.len() > 70 && struck.len() < 130, "{}", struck.len());
    assert_eq!(run(), struck);
}

#[test]
fn a_body_strikes_whoever_is_in_its_way_on_its_planes() {
    let defs = common::test_defs();
    let mut w = world_from(Arc::clone(&defs), |_| {});
    let bystander = EntityId::new(30, 0);
    let ghost = EntityId::new(31, 0);
    let astral = PlaneMask::of(PlaneId::new(5));
    let (near, far) = (w.loc(12, 16, 0), w.loc(11, 16, 0));
    let standing = |id| {
        Occupant::new(id, OccupantKind::Creature)
            .with_mass(r(80.0))
            .with_flags(OccupantFlags::COLLIDES)
    };
    let mut ctx = w.ctx();
    ctx.place(standing(bystander), near, Footprint::SINGLE, someone())
        .unwrap();
    ctx.place(
        standing(ghost).on(astral),
        far,
        Footprint::SINGLE,
        someone(),
    )
    .unwrap();
    let body = lump(&w, v(8.5, 16.5, 1.0), v(10.0, 0.0, 2.0), 80.0);
    let id = w.ctx().launch(body).unwrap();
    w.run_until(Tick(400_000));

    let hits = events_where(&w, |event| {
        matches!(event, WorldEvent::BodyHitOccupant { .. })
    });
    assert_eq!(hits.len(), 1);
    let WorldEvent::BodyHitOccupant {
        occupant,
        loc,
        packet,
        vel,
        push,
        pos,
        ..
    } = hits[0]
    else {
        unreachable!()
    };
    // The one on another plane was passed through.
    assert_eq!((*occupant, loc.cell), (bystander, at(12, 16, 0)));
    assert_eq!(packet.kind, w.damage_type("core:impact"));
    let speed_sq: f64 = vel.iter().map(|part| part.get() * part.get()).sum();
    assert!((packet.energy.get() - 0.5 * 80.0 * speed_sq).abs() < 1.0e-6);
    // Spread over the body's cross-section and stopping depth.
    let area = r(0.08).cbrt().powi(2).get();
    assert!((packet.intensity.get() - packet.energy.get() / (area * 0.1)).abs() < 1.0);
    assert!(pos[2] > r(0.5) && pos[2] < r(1.5));
    // Equal masses and no bounce: each goes on at half the speed.
    for axis in 0..3 {
        assert!((push[axis].get() - vel[axis].get() / 2.0).abs() < 1.0e-9);
    }
    // The world moved neither of those struck: that is the simulation's.
    assert_eq!(w.cosmos.occupant(bystander).unwrap().loc, near);
    assert!(w.cosmos.body(id).unwrap().vel[0] < r(5.01));

    // A bullet strikes with its own surface.
    w.events.clear();
    let mut bullet = lump(&w, v(8.5, 16.5, 1.2), v(400.0, 0.0, 0.0), 0.03);
    bullet.owner = BodyOwner::Projectile(Projectile {
        kind: w.damage_type("core:pierce"),
        area: r(2.4e-4),
        give: r(0.01),
        tag: 77,
    });
    bullet.area = r(2.4e-4);
    bullet.drag = r(0.5);
    w.ctx().launch(bullet).unwrap();
    w.run_for(Dur::from_millis(20));
    let hits = events_where(&w, |event| {
        matches!(
            event,
            WorldEvent::BodyHitOccupant {
                owner: BodyOwner::Projectile(_),
                ..
            }
        )
    });
    let WorldEvent::BodyHitOccupant { packet, owner, .. } = hits[0] else {
        unreachable!()
    };
    assert!(matches!(owner, BodyOwner::Projectile(projectile) if projectile.tag == 77));
    assert_eq!(packet.kind, w.damage_type("core:pierce"));
    assert!((packet.intensity.get() - packet.energy.get() / 2.4e-6).abs() < 1.0);
    assert!(packet.energy.get() > 2300.0 && packet.energy.get() <= 2400.0);
}

#[test]
fn an_elements_contact_surface_is_reported_with_the_impact() {
    let defs = defs();
    let spikes = defs.fixture_state("t:spikes", None).unwrap();
    let mut w = world_in(&defs, "core:testland", &[(0, 0)], |_, builder| {
        builder.set_fixture(at(10, 16, 0), spikes);
    });
    let body = lump(&w, v(8.5, 16.5, 0.3), v(6.0, 0.0, 0.0), 80.0);
    w.ctx().launch(body).unwrap();
    w.run_out();
    let on_spikes = impacts(&w).iter().find_map(|event| match event {
        WorldEvent::BodyImpact {
            layer: Layer::Fixture,
            loc,
            contact,
            ..
        } => Some((loc.cell, *contact)),
        _ => None,
    });
    let (cell, contact) = on_spikes.expect("it met the spikes");
    assert_eq!(cell, at(10, 16, 0));
    let contact = contact.expect("the spikes have a surface");
    assert_eq!(contact.kind, w.damage_type("core:pierce"));
    assert_eq!(
        (contact.area, contact.give, contact.energy),
        (r(3.0e-5), r(0.003), r(60.0))
    );
    // The ground has none.
    assert!(impacts(&w).iter().any(|event| matches!(
        event,
        WorldEvent::BodyImpact {
            layer: Layer::Floor,
            contact: None,
            ..
        }
    )));
}

// ---- Debris --------------------------------------------------------------

#[test]
fn debris_from_a_destroyed_wall_flies_away_from_the_blow() {
    let defs = common::test_defs();
    let wall = defs.block_state("core:wall", None).unwrap();
    let mut w = world_from(Arc::clone(&defs), |builder| {
        builder.set_block(at(10, 16, 0), wall);
    });
    let impact = w.damage_type("core:impact");
    let (target, east) = (w.loc(10, 16, 0), v(1.0, 0.0, 0.0));
    // 0.7 MJ, concentrated: the wall takes its 0.6 MJ and 0.1 MJ is over.
    let packet = DamagePacket::new(impact, r(0.7e6), r(100.0e6), someone());
    let outcome = w
        .ctx()
        .edit(someone())
        .damage_along(target, Layer::Block, packet, Some(east))
        .unwrap();
    assert_eq!(outcome.result, DamageResult::Destroyed);
    // A tenth of what was over went into the fragments.
    assert!(close(outcome.passed, 0.09e6, 1.0e-6));
    assert!(close(outcome.absorbed, 0.61e6, 1.0e-6));

    let thrown: Vec<(Real, [Real; 3])> = w
        .events
        .events()
        .iter()
        .filter_map(|event| match event {
            WorldEvent::BodyLaunched {
                owner: BodyOwner::Debris(_),
                mass,
                vel,
                ..
            } => Some((*mass, *vel)),
            _ => None,
        })
        .collect();
    assert_eq!(thrown.len(), DEBRIS_PER_ELEMENT as usize);
    assert_eq!(w.cosmos.debris_in_flight(), DEBRIS_PER_ELEMENT);
    let mut energy = 0.0;
    for (mass, vel) in &thrown {
        // A quarter of the wall's 1440 kg between them, going east.
        assert!(close(*mass, 120.0, 1.0e-9));
        assert!(vel[0] > r(3.0), "{vel:?}");
        let speed_sq: f64 = vel.iter().map(|part| part.get() * part.get()).sum();
        energy += 0.5 * mass.get() * speed_sq;
    }
    assert!((energy - 0.01e6).abs() < 1.0e-6, "{energy}");
    // No two go quite the same way.
    assert_ne!(thrown[0].1, thrown[1].1);

    w.run_out();
    assert_eq!(w.cosmos.debris_in_flight(), 0);
    let fell = landings(&w);
    assert_eq!(fell.len(), 3);
    for event in fell {
        let WorldEvent::BodyLanded { loc, .. } = event else {
            unreachable!()
        };
        assert!(loc.cell.x > 10, "{loc:?}");
    }
    // Matter is kept: the wall's 600 litres lie where it stood and where
    // its fragments came to rest.
    let mut litres = 0;
    for y in 0..32 {
        for x in 0..64 {
            litres += w.defs.rubble_units(w.fixture_at(x, y, 0)).unwrap_or(0);
        }
    }
    assert_eq!(litres, 600);
    assert_eq!(w.defs.rubble_units(w.fixture_at(10, 16, 0)), Some(450));

    // A blow with no direction throws nothing.
    let mut w = world_from(Arc::clone(&defs), |builder| {
        builder.set_block(at(10, 16, 0), wall);
    });
    let outcome = w
        .ctx()
        .edit(someone())
        .damage(target, Layer::Block, packet)
        .unwrap();
    assert!(close(outcome.passed, 0.1e6, 1.0e-6));
    assert_eq!(w.cosmos.bodies().count(), 0);
    assert_eq!(w.defs.rubble_units(w.fixture_at(10, 16, 0)), Some(600));
}

#[test]
fn no_more_debris_flies_than_the_budget_allows() {
    let defs = common::test_defs();
    let wall = defs.block_state("core:wall", None).unwrap();
    let mut w = world_from(Arc::clone(&defs), |builder| {
        for y in 1..31 {
            for x in [10, 12, 14, 16] {
                builder.set_block(at(x, y, 0), wall);
            }
        }
    });
    let impact = w.damage_type("core:impact");
    let packet = DamagePacket::new(impact, r(0.7e6), r(100.0e6), someone());
    let mut whole = 0;
    for y in 1..31 {
        for x in [10, 12, 14, 16] {
            let target = w.loc(x, y, 0);
            w.ctx()
                .edit(someone())
                .damage_along(target, Layer::Block, packet, Some(v(0.0, 0.0, 1.0)))
                .unwrap();
            assert!(w.cosmos.debris_in_flight() <= DEBRIS_MAX_BODIES);
            if w.defs.rubble_units(w.fixture_at(x, y, 0)) == Some(600) {
                whole += 1;
            }
        }
    }
    // 120 walls would throw 360 fragments. The first 85 throw three
    // each and the next the one that is left; the rest are left whole.
    assert_eq!(w.cosmos.debris_in_flight(), DEBRIS_MAX_BODIES);
    assert_eq!(whole, 120 - 86);
}

// ---- Clocks, wind, drives ----------------------------------------------

/// A lump thrown east across a street.
fn throw_down_the_street(w: &mut World) -> BodyId {
    let body = lump(w, v(8.5, 16.5, 1.5), v(10.0, 0.0, 6.0), 2.0);
    w.ctx().launch(body).unwrap()
}

#[test]
fn time_stop_street() {
    let defs = common::test_defs();
    // As it would go with nothing in the way.
    let mut open = world_from(Arc::clone(&defs), |_| {});
    throw_down_the_street(&mut open);
    open.run_out();
    let lands = landed_at(&open);
    let took = open.timeline.now();

    // The same throw, into a place where time has stopped.
    let mut w = world_from(Arc::clone(&defs), |_| {});
    let mut bubble = AreaSpec::new(
        w.grid,
        AreaShape::Box(CellBox::new(at(20, 10, 0), at(24, 22, 3))),
    );
    bubble.clock_rate = Some(Real::ZERO);
    let area = w.ctx().create_area(bubble, someone()).unwrap();
    let id = throw_down_the_street(&mut w);
    w.run_until(Tick(3_000_000));
    let hung = *w.cosmos.body(id).unwrap();
    // It is just inside, in the air, with its speed.
    assert!(hung.pos[0] >= r(20.0) && hung.pos[0] < r(21.0), "{hung:?}");
    assert!(hung.pos[2] > r(1.0));
    assert!(hung.speed() > r(8.0));
    w.run_until(Tick(60_000_000));
    assert_eq!(*w.cosmos.body(id).unwrap(), hung);
    assert!(landings(&w).is_empty());

    // Time moves again: it goes on as if nothing had happened, and
    // comes down exactly where it would have.
    w.ctx().set_area_clock_rate(area, Some(Real::ONE)).unwrap();
    w.run_out();
    assert_eq!(landed_at(&w), lands);
    assert!(w.timeline.now() > took + Dur::from_secs(58));
    assert!(w.cosmos.processes().is_idle());

    // Time stopped round a body already in flight holds it too.
    let mut w = world_from(Arc::clone(&defs), |_| {});
    let id = throw_down_the_street(&mut w);
    w.run_until(Tick(500_000));
    let mut bubble = AreaSpec::new(
        w.grid,
        AreaShape::Box(CellBox::new(at(0, 0, 0), at(40, 31, 5))),
    );
    bubble.clock_rate = Some(Real::ZERO);
    let area = w.ctx().create_area(bubble, someone()).unwrap();
    let hung = *w.cosmos.body(id).unwrap();
    w.run_until(Tick(30_000_000));
    assert_eq!(*w.cosmos.body(id).unwrap(), hung);
    w.ctx().remove_area(area, someone()).unwrap();
    w.run_out();
    assert_eq!(landed_at(&w), lands);
}

#[test]
fn a_light_body_thrown_across_a_wind_lands_downwind() {
    let defs = common::test_defs();
    let throw_light = |w: &mut World| {
        let mut body = lump(w, v(8.5, 10.5, 1.5), v(10.0, 0.0, 4.0), 0.05);
        body.area = r(0.01);
        w.ctx().launch(body).unwrap();
        w.run_out();
        first_floor_impact(w).0
    };
    let mut still = world_from(Arc::clone(&defs), |_| {});
    let calm = throw_light(&mut still);
    assert!(close(calm[1], 10.5, 1.0e-9));

    // The same, with the air moving south at 6 m/s: drift is supplied
    // by an area until the medium is built (11 §4.7).
    let mut windy = world_from(Arc::clone(&defs), |_| {});
    let mut wind = AreaSpec::new(
        windy.grid,
        AreaShape::Box(CellBox::new(at(0, 0, 0), at(63, 31, 10))),
    );
    wind.ambient = Some(AmbientOverride {
        gravity: None,
        drift: Some(Drift::Flow(v(0.0, 6.0, 0.0))),
    });
    windy.ctx().create_area(wind, someone()).unwrap();
    let blown = throw_light(&mut windy);
    assert!(blown[1] > calm[1] + r(1.0), "{blown:?} against {calm:?}");
    // The wind is across the throw: it goes about as far.
    assert!((blown[0] - calm[0]).get().abs() < 0.5);

    // A heavy thing is hardly moved by it.
    let mut windy = world_from(Arc::clone(&defs), |_| {});
    let mut wind = AreaSpec::new(
        windy.grid,
        AreaShape::Box(CellBox::new(at(0, 0, 0), at(63, 31, 10))),
    );
    wind.ambient = Some(AmbientOverride {
        gravity: None,
        drift: Some(Drift::Flow(v(0.0, 6.0, 0.0))),
    });
    windy.ctx().create_area(wind, someone()).unwrap();
    let body = lump(&windy, v(8.5, 10.5, 1.5), v(10.0, 0.0, 4.0), 80.0);
    windy.ctx().launch(body).unwrap();
    windy.run_out();
    let heavy = first_floor_impact(&windy).0;
    assert!(heavy[1] < r(10.6), "{heavy:?}");
}

#[test]
fn a_driven_body_gains_speed_for_its_burn_and_turns_no_faster_than_it_can() {
    // High over testland there is neither air nor weight.
    let defs = common::test_defs();
    let mut w = world_from(Arc::clone(&defs), |_| {});
    let mut rocket = lump(&w, v(2.5, 16.5, 480.5), v(10.0, 0.0, 0.0), 6.0);
    rocket.area = r(0.008);
    rocket.drag = r(0.3);
    rocket.drive = Some(Drive {
        accel: r(20.0),
        burn: r(1.0),
        turn: r(0.5),
    });
    let id = w.ctx().launch(rocket).unwrap();

    w.run_until(Tick(500_000));
    let speed = w.cosmos.body(id).unwrap().speed();
    assert!(close(speed, 20.0, 1.0), "{speed:?}");
    w.run_until(Tick(1_200_000));
    let burnt = *w.cosmos.body(id).unwrap();
    // 10 m/s and a second at 20 m/s²; with no point to steer for it has
    // flown straight.
    assert!(close(burnt.speed(), 30.0, 1.0e-9));
    assert_eq!((burnt.vel[1], burnt.vel[2]), (Real::ZERO, Real::ZERO));
    assert_eq!(burnt.drive.unwrap().burn, Real::ZERO);

    // Told to turn for a point due south, it turns at half a radian a
    // second and keeps its speed.
    let south = [burnt.pos[0], burnt.pos[1] + r(1.0e6), burnt.pos[2]];
    w.ctx().steer(id, Some(south)).unwrap();
    w.run_until(Tick(2_000_000));
    let turning = *w.cosmos.body(id).unwrap();
    let heading = turning.vel[1].atan2(turning.vel[0]).get();
    assert!(heading > 0.37 && heading <= 0.4 + 1.0e-9, "{heading}");
    assert!(close(turning.speed(), 30.0, 1.0e-9));

    // Told to fly straight again, it does.
    w.ctx().steer(id, None).unwrap();
    let straight = w.cosmos.body(id).unwrap().vel;
    w.run_until(Tick(2_300_000));
    assert_eq!(w.cosmos.body(id).unwrap().vel, straight);

    // Caught, it is a body no longer.
    let caught = w.ctx().arrest(id).unwrap();
    assert_eq!(caught.vel, straight);
    assert_eq!(w.cosmos.bodies().count(), 0);
    assert!(w.cosmos.processes().is_idle());
    assert_eq!(w.ctx().arrest(id), Err(WorldError::UnknownBody(id)));
    assert!(events_where(&w, |event| matches!(event, WorldEvent::BodyLanded { .. })).is_empty());
}

#[test]
fn a_push_changes_a_bodys_motion_as_it_is_now() {
    let defs = common::test_defs();
    let mut w = world_from(Arc::clone(&defs), |_| {});
    // Where there is no weight: a lump drifting east.
    let body = lump(&w, v(2.5, 16.5, 480.5), v(10.0, 0.0, 0.0), 2.0);
    let id = w.ctx().launch(body).unwrap();
    w.run_until(Tick(1_005_000));
    w.ctx().impulse(id, v(0.0, 20.0, -4.0)).unwrap();
    let pushed = *w.cosmos.body(id).unwrap();
    // It was brought up to now first: ten metres and a little.
    assert!(close(pushed.pos[0], 2.5 + 10.05, 1.0e-6), "{pushed:?}");
    assert_eq!(pushed.vel, v(10.0, 10.0, -2.0));
    w.run_until(Tick(2_005_000));
    let later = *w.cosmos.body(id).unwrap();
    assert!(close(later.pos[1], 16.5 + 10.0, 0.3), "{later:?}");

    // What is refused, and why.
    assert!(
        w.ctx()
            .impulse(id, [Real::INFINITY, r(0.0), r(0.0)])
            .is_err()
    );
    let gone = BodyId::new(99);
    assert_eq!(
        w.ctx().impulse(gone, v(1.0, 0.0, 0.0)),
        Err(WorldError::UnknownBody(gone))
    );
    let mut weightless = lump(&w, v(2.5, 16.5, 1.5), v(1.0, 0.0, 0.0), 2.0);
    weightless.mass = Real::ZERO;
    assert!(matches!(
        w.ctx().launch(weightless),
        Err(WorldError::BadValue { .. })
    ));
    let again = lump(&w, v(2.5, 16.5, 1.5), v(1.0, 0.0, 0.0), 2.0);
    assert!(matches!(
        w.ctx().launch(again),
        Err(WorldError::BadValue { .. })
    ));
    let mut nowhere = lump(&w, v(2.5, 200.5, 1.5), v(1.0, 0.0, 0.0), 2.0);
    nowhere.owner = BodyOwner::Entity(EntityId::new(50, 0));
    assert!(matches!(
        w.ctx().launch(nowhere),
        Err(WorldError::NotLoaded(_))
    ));

    // Something hardly moving, where nothing pulls on it, has stopped.
    let mut adrift = lump(&w, v(2.5, 10.5, 480.5), v(0.1, 0.0, 0.0), 2.0);
    adrift.owner = BodyOwner::Entity(EntityId::new(51, 0));
    assert!(adrift.speed() < REST_SPEED);
    w.ctx().launch(adrift).unwrap();
    w.events.clear();
    w.run_for(Dur::from_millis(50));
    assert_eq!(landings(&w).len(), 1);
}

// ---- Saving ----------------------------------------------------------------

#[test]
fn a_save_in_mid_flight_changes_nothing() {
    let defs = common::test_defs();
    let end = Tick(4_000_000);
    let begin = || {
        let mut w = three_walls(&defs);
        throw_someone(&mut w, 600.0);
        // And something slower, to be in the air for a while.
        let mut slow = lump(&w, v(8.5, 8.5, 1.5), v(6.0, 2.0, 9.0), 3.0);
        slow.owner = BodyOwner::Entity(EntityId::new(40, 0));
        w.ctx().launch(slow).unwrap();
        w
    };
    let mut straight = begin();
    straight.run_until(end);

    let mut first = begin();
    first.run_until(Tick(15_000));
    // Someone going through walls, fragments of the walls, and a lump.
    assert!(first.cosmos.bodies().count() >= 4);
    assert!(first.cosmos.debris_in_flight() > 0);

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

// ---- Leaving loaded space ------------------------------------------------

/// Flat ground: rock with grass on it, everywhere.
#[derive(Debug)]
struct Plain {
    rock: BlockState,
    grass: clash_world::FloorState,
}

impl Plain {
    fn factory(
        request: &GeneratorRequest<'_>,
    ) -> Result<Arc<dyn WorldGenerator>, clash_world::GenError> {
        Ok(Arc::new(Plain {
            rock: request.defs.block_state("core:solid", None).unwrap(),
            grass: request.defs.floor_state("core:grass", None).unwrap(),
        }))
    }
}

impl RegionSurveyor for Plain {
    fn survey(&self, _: &GenCtx<'_>, _: &RealmInfo, _: RegionPos) -> RegionSurveyResult {
        RegionSurveyResult::new(RegionRecord::new(RegionSurvey::default()))
    }
}

impl TractRealizer for Plain {
    fn realize(
        &self,
        cx: &GenCtx<'_>,
        _: &RealmInfo,
        _: TractPos,
        _: &TractRecord,
        _: &TractRecords3x3,
    ) -> RealizedTract {
        let mut column = ColumnBuilder::new(cx.defs(), self.rock, BlockState::NONE, 0);
        column.fill_floor(0, self.grass);
        RealizedTract::new(column.build())
    }
}

#[test]
fn over_the_horizon() {
    let defs = defs();
    let mut registry = GeneratorRegistry::new();
    registry.register("plain", Plain::factory);
    let generators = registry.bind(&defs);
    let mut store = MemoryStore::new();

    // Two tracts where the gun stands, and one far down range.
    let mut w = world_in(&defs, "t:airless", &[], |_, _| {});
    let realm = w.cosmos.realm_of(w.grid).unwrap();
    let load = |w: &mut World, store: &mut MemoryStore, tracts: &[(i32, i32)]| {
        let wanted: Vec<_> = tracts
            .iter()
            .map(|(x, y)| (realm, TractPos::new(*x, *y)))
            .collect();
        let mut io = TractIo::new(&generators, store);
        for loaded in w.ctx().load_tracts(&mut io, &wanted) {
            loaded.unwrap();
        }
    };
    load(&mut w, &mut store, &[(0, 0), (1, 0), (5, 0)]);
    w.events.clear();

    // A shell fired across twenty tracts.
    let g = 9.81_f64;
    let t = (40.0 + (1600.0 + 2.0 * g * 1.5).sqrt()) / g;
    let mut shell = lump(&w, v(16.5, 16.5, 1.5), v(80.0, 0.0, 40.0), 20.0);
    shell.owner = BodyOwner::Projectile(Projectile {
        kind: w.damage_type("core:impact"),
        area: r(0.01),
        give: r(0.05),
        tag: 1,
    });
    let id = w.ctx().launch(shell).unwrap();
    w.run_until(Tick(500_000));
    // Its ticket goes ahead of it, into what is not loaded.
    let held: Vec<_> = w.cosmos.tickets().iter().collect();
    assert_eq!(held.len(), 1);
    assert_eq!(held[0].1.reason, TicketReason::Body);
    assert_eq!(
        w.cosmos.tickets().centre(held[0].0),
        Some(TractPos::new(2, 0))
    );
    w.run_out();

    // It left, and was finished by arithmetic: where and when it comes
    // down is what its flight gives.
    let left = events_where(&w, |event| {
        matches!(event, WorldEvent::BodyLeftLoaded { .. })
    });
    // On its way it passes high over the loaded tract down range: for
    // that stretch it is a body again, with nothing up there to meet.
    assert_eq!(left.len(), 2);
    assert!(matches!(
        left[0],
        WorldEvent::BodyLeftLoaded { loc, lands: Some(over), returns: true, .. }
            if loc.cell.x == 63 && over.cell.x == 160
    ));
    let WorldEvent::BodyLeftLoaded {
        body,
        loc,
        lands,
        at: when,
        returns,
        energy,
        ..
    } = left[1]
    else {
        unreachable!()
    };
    assert_eq!((*body, *returns), (id, false));
    assert_eq!(loc.cell.x, 191);
    let x = 16.5 + 80.0 * t;
    let lands = lands.expect("it comes down");
    assert_eq!(lands.cell, at(x.floor() as i32, 16, 0));
    assert_eq!(lands.cell.tract(), TractPos::new(20, 0));
    let late = when.unwrap().as_micros() - (t * 1.0e6) as i64;
    assert!(late.abs() <= 1000, "{late}");
    let arriving = 0.5 * 20.0 * (6400.0 + 1600.0) + 20.0 * g * 1.5;
    assert!((energy.get() - arriving).abs() < arriving * 1.0e-5);
    // None of the twenty tracts was loaded for it, it holds no ticket
    // now, and it is a body no longer.
    assert_eq!(w.cosmos.loaded_tracts(realm).len(), 3);
    assert!(w.cosmos.tickets().is_empty());
    assert_eq!(w.cosmos.bodies().count(), 0);
    assert!(w.cosmos.processes().is_idle());

    // Its impact waits with the tract it came down in, and is found
    // there when the tract is loaded.
    let waiting = |w: &World| {
        let record = w.cosmos.tract_record(realm, TractPos::new(20, 0)).unwrap();
        record
            .pending()
            .iter()
            .filter_map(|edit| match edit {
                PendingEdit::Impact(impact) => Some(*impact),
                _ => None,
            })
            .collect::<Vec<_>>()
    };
    let before = waiting(&w);
    assert_eq!(before.len(), 1);
    assert_eq!(before[0].cell, lands.cell);
    assert_eq!(Some(before[0].tick), *when);
    assert_eq!(before[0].energy, *energy);
    assert_eq!(before[0].mass, r(20.0));
    assert!(matches!(before[0].owner, BodyOwner::Projectile(shell) if shell.tag == 1));
    assert_eq!(before[0].cause.instigator, someone().instigator);
    load(&mut w, &mut store, &[(20, 0)]);
    assert_eq!(waiting(&w), before);

    // One that comes down where a tract is loaded lands in full.
    w.events.clear();
    let t = (80.0 + (6400.0 + 2.0 * g * 1.5).sqrt()) / g;
    let lob = lump(&w, v(16.5, 16.5, 1.5), v(9.9, 0.0, 80.0), 20.0);
    let launched = w.timeline.now();
    let id = w.ctx().launch(lob).unwrap();
    w.run_until(launched + Dur::from_secs(8));
    let back = events_where(&w, |event| {
        matches!(event, WorldEvent::BodyLeftLoaded { .. })
    });
    let WorldEvent::BodyLeftLoaded {
        lands,
        returns,
        at: when,
        ..
    } = back[0]
    else {
        unreachable!()
    };
    // It comes back in at the edge of the loaded tract, and is still a
    // body: beyond reach until it arrives.
    assert!(*returns);
    assert_eq!(lands.unwrap().cell.x, 160);
    assert!(when.unwrap() > launched + Dur::from_secs(14));
    assert_eq!(w.cosmos.body_of(thrown()), Some(id));
    assert!(w.cosmos.body_flight(id).unwrap().away);
    assert!(w.ctx().impulse(id, v(1.0, 0.0, 0.0)).is_err());
    w.run_out();
    let (pos, tick) = first_floor_impact(&w);
    assert!(close(pos[0], 16.5 + 9.9 * t, 0.01), "{pos:?}");
    let late = (tick - launched).as_micros() - (t * 1.0e6) as i64;
    assert!((0..=BODY_MAX_STEP.as_micros()).contains(&late), "{late}");
    // It came down hard enough to break the turf, stopped on the rock
    // under it, and slid to rest.
    assert!(
        events_where(&w, |event| {
            matches!(event, WorldEvent::ElementDestroyed { loc, layer: Layer::Floor, .. }
            if loc.cell.tract() == TractPos::new(5, 0))
        })
        .len()
            == 1
    );
    let rest = landed_at(&w)[0].get();
    assert!(
        (rest - (16.5 + 9.9 * t + 9.9 * 9.9 / g)).abs() < 0.2,
        "{rest}"
    );
    assert_eq!(w.cosmos.bodies().count(), 0);
}

// ---- Cost ------------------------------------------------------------------

/// Prints what a projectile's flight costs. Run it with
/// `cargo test -p clash-world --release --test bodies -- --ignored --nocapture`.
#[test]
#[ignore = "a timing, not a check"]
#[allow(
    clippy::disallowed_types,
    clippy::disallowed_methods,
    reason = "a timing test reads the wall clock; nothing in the simulation does"
)]
fn timing_of_a_projectile_across_thirty_metres() {
    let defs = common::test_defs();
    let mut w = world_from(Arc::clone(&defs), |_| {});
    let pierce = w.damage_type("core:pierce");
    let shots = 2000_u32;
    let started = std::time::Instant::now();
    let mut wakes = 0_u64;
    for shot in 0..shots {
        let mut bullet = lump(&w, v(2.5, 16.5, 1.5), v(900.0, 0.0, 0.0), 0.01);
        bullet.owner = BodyOwner::Projectile(Projectile {
            kind: pierce,
            area: r(5.0e-5),
            give: r(0.01),
            tag: u64::from(shot),
        });
        bullet.area = r(5.0e-5);
        bullet.drag = r(0.3);
        let id = w.ctx().launch(bullet).unwrap();
        // Thirty metres at 900 m/s.
        let until = w.timeline.now() + Dur::from_micros(33_400);
        while let Some((_, wake)) = w.timeline.pop(until) {
            w.ctx().handle_wake(&wake).unwrap();
            wakes += 1;
        }
        w.timeline.advance_to(until);
        let flown = w.ctx().arrest(id).unwrap();
        assert!(flown.pos[0] > r(31.0), "{flown:?}");
        w.events.clear();
    }
    let took = started.elapsed();
    println!(
        "a projectile across 30 m: {:.2} µs a flight, {:.0} ns a step ({} steps a flight)",
        took.as_secs_f64() * 1.0e6 / f64::from(shots),
        took.as_secs_f64() * 1.0e9 / wakes as f64,
        wakes / u64::from(shots)
    );
}
