//! Tests for W16: fields (02 §16), with the review additions: reactions,
//! drift and exposure.
//!
//! `office_fire` and `quench` are the bodies of the test scripts of
//! those names. The scripts themselves are owed: the script runner does
//! not exist yet (W11).

mod common;
#[path = "common/wreck.rs"]
mod wreck;

use std::sync::Arc;

use clash_core::{Dur, EntityId, Hash128, Real, Tick, codec};
use clash_time::Timeline;
use clash_world::field::{FieldReacts, FieldRule, FieldRuleKind};
use clash_world::{
    AmbientOverride, AreaShape, AreaSpec, BlockState, Cause, CauseKind, CellBox, CellPos,
    ColumnBuilder, DamagePacket, Destruction, Drift, EventSink, ExposureRoute, FieldId, FloorState,
    Footprint, Layer, Occupant, OccupantKind, RawWorldDefs, WorldActivity, WorldDefs, WorldEvent,
};
use wreck::{World, at, someone, world_from};

/// The definitions of `mods/core`, with fields and floors for these
/// tests.
fn raw() -> RawWorldDefs {
    let mut raw = common::test_raw();
    common::add_json(
        &mut raw,
        "t",
        r#"[
            { "type": "material", "id": "ash", "density": 500, "toughness": 1.0e5,
              "resist": { "core:heat": { "threshold": "100 MPa" } } },
            { "type": "floor", "id": "char", "material": "ash", "thickness": "4 cm" },
            { "//": "A wooden floor that leaves its char behind.",
              "type": "floor", "id": "planks", "material": "core:wood", "thickness": "4 cm",
              "on_destroy": "char" },

            { "type": "field", "id": "gas", "step": "100 ms",
              "rules": [ { "rule": "diffuse", "rate": 0.2 } ] },
            { "type": "field", "id": "mist", "step": "100 ms",
              "rules": [ { "rule": "decay", "amount": 3 }, { "rule": "diffuse", "rate": 0.2 } ] },
            { "type": "field", "id": "haze", "step": "100 ms",
              "rules": [ { "rule": "decay", "amount": 1 } ] },
            { "type": "field", "id": "toxin", "step": "200 ms",
              "rules": [ { "rule": "expose", "pool": "toxin", "per_level": 0.02, "route": "breath" } ] },

            { "type": "field", "id": "acid", "step": "100 ms",
              "rules": [ { "rule": "convert", "with": "lye",
                           "uses": { "acid": 1, "lye": 2 }, "gives": { "salt": 3 } } ] },
            { "type": "field", "id": "lye", "step": "100 ms" },
            { "type": "field", "id": "salt", "step": "100 ms" },
            { "type": "field", "id": "acid2", "step": "100 ms" },
            { "type": "field", "id": "lye2", "step": "100 ms" },
            { "type": "field", "id": "salt2", "step": "100 ms" },
            { "type": "field_reaction", "id": "neutralise",
              "when": ["acid2", "lye2"],
              "uses": { "acid2": 1, "lye2": 2 }, "gives": { "salt2": 3 } }
        ]"#,
        "the fields of the field tests",
    );
    raw
}

fn defs() -> Arc<WorldDefs> {
    Arc::new(WorldDefs::build(raw()).unwrap_or_else(|errors| panic!("{errors:?}")))
}

fn field(w: &World, name: &str) -> FieldId {
    w.defs
        .fields()
        .resolve(name, None)
        .unwrap_or_else(|| panic!("no field {name}"))
}

fn add(w: &mut World, cell: CellPos, field: FieldId, amount: i16) {
    let loc = w.loc(cell.x, cell.y, cell.z);
    w.ctx()
        .edit(someone())
        .add_field(loc, field, amount)
        .unwrap();
}

fn level(w: &World, cell: CellPos, field: FieldId) -> u8 {
    w.cosmos
        .field(w.loc(cell.x, cell.y, cell.z), field)
        .unwrap()
}

/// Every cell of a box that holds a field, with its level.
fn held(w: &World, field: FieldId, low: CellPos, high: CellPos) -> Vec<(CellPos, u8)> {
    w.cosmos
        .fields_in_box(w.grid, CellBox::new(low, high))
        .unwrap()
        .into_iter()
        .filter(|sample| sample.field == field)
        .map(|sample| (sample.cell, sample.level))
        .collect()
}

fn total(w: &World, field: FieldId) -> u32 {
    held(w, field, at(0, 0, -8), at(63, 31, 40))
        .iter()
        .map(|(_, level)| u32::from(*level))
        .sum()
}

/// A rectangle of the ground storey laid with a floor.
fn lay(builder: &mut ColumnBuilder<'_>, floor: FloorState, x: (i32, i32), y: (i32, i32)) {
    for cy in y.0..=y.1 {
        for cx in x.0..=x.1 {
            builder.set_floor(at(cx, cy, 0), floor);
        }
    }
}

fn planks(defs: &WorldDefs) -> FloorState {
    defs.floor_state("t:planks", None).unwrap()
}

fn char_floor(defs: &WorldDefs) -> FloorState {
    defs.floor_state("t:char", None).unwrap()
}

fn fields_idle(w: &World) -> bool {
    w.cosmos.active_field_cells() == 0 && w.cosmos.field_state().steps().count() == 0
}

// ---- definitions ---------------------------------------------------

#[test]
fn the_fields_of_core_load_as_written() {
    let defs = common::test_defs();
    let fire = defs.fields().lookup("core:fire").unwrap();
    let smoke = defs.fields().lookup("core:smoke").unwrap();
    let water = defs.fields().lookup("core:water").unwrap();
    let def = defs.fields().get(fire).unwrap();
    assert_eq!(def.step, Dur::from_millis(500));
    let kinds: Vec<FieldRuleKind> = def.rules.iter().map(FieldRule::kind).collect();
    assert_eq!(
        kinds,
        [
            FieldRuleKind::Decay,
            FieldRuleKind::Spread,
            FieldRuleKind::Emit,
            FieldRuleKind::Damage,
            FieldRuleKind::Drift
        ]
    );
    assert!(matches!(
        &def.rules[2],
        FieldRule::Emit { field, amount: 12, from: 1 } if *field == smoke
    ));
    // It feeds on fuel, and water puts it out: water is named first.
    assert!(matches!(&def.reacts[0], FieldReacts::Material { set, .. } if set.tag() == "fuel"));
    assert!(matches!(
        &def.reacts[1],
        FieldReacts::With { other, first: false, use_self: 3, use_other: 1, .. } if *other == water
    ));
    assert!(def.live && !def.rests);
    assert!(def.has_tag("light_source"));
    assert_eq!(
        def.light.unwrap().at_level(255).unwrap().intensity,
        Real::of(1.5)
    );
    assert!(def.light.unwrap().at_level(19).is_none());

    // Water only moves: it may lie still. It passes a shock on.
    let def = defs.fields().get(water).unwrap();
    assert!(def.rests);
    assert_eq!(def.conducts.len(), 1);
    assert_eq!(def.conducts[0].share, Real::of(0.8));
    assert_eq!(defs.fields().get(smoke).unwrap().opaque_from, Some(160));
    assert_eq!(defs.field_reactions().len(), 3);
    // Wood is fuel by its tag, and flammable by its flag.
    let wood = defs.materials().lookup("core:wood").unwrap();
    assert!(matches!(
        &def.reacts[0],
        FieldReacts::With { first: true, .. }
    ));
    let fire_def = defs.fields().get(fire).unwrap();
    let FieldRule::Spread {
        needs: Some(needs), ..
    } = &fire_def.rules[1]
    else {
        panic!("fire spreads to what it needs");
    };
    assert!(needs.has(wood));
    assert!(!needs.has(defs.materials().lookup("core:concrete").unwrap()));
}

#[test]
fn a_mistake_in_a_field_is_reported_where_it_is() {
    let wrong = |json: &str| {
        let mut raw = common::test_raw();
        common::add_json(&mut raw, "w", json, "a field with a mistake");
        let errors = WorldDefs::build(raw).expect_err("the definitions should not build");
        errors
            .iter()
            .map(|error| {
                format!(
                    "{} {} {}: {}",
                    error.kind, error.id, error.path, error.message
                )
            })
            .collect::<Vec<_>>()
            .join("\n")
    };
    let found = wrong(
        r#"[ { "type": "field", "id": "f", "step": "1 s",
              "rules": [ { "rule": "decay", "amount": 2, "rate": 0.5 },
                         { "rule": "emit", "field": "nothing", "amount": 3 },
                         { "rule": "diffuse", "rate": 0.9 },
                         { "rule": "damage", "kind": "core:heat" } ] } ]"#,
    );
    assert!(
        found.contains("field w:f rules.0.rate: the rule `decay` has no `rate`"),
        "{found}"
    );
    assert!(
        found.contains("field w:f rules.1.field: there is no field `nothing`"),
        "{found}"
    );
    assert!(
        found.contains("field w:f rules.2.rate: must be more than 0 to 0.25"),
        "{found}"
    );
    assert!(
        found.contains("rules.3.energy_per_level: the rule `damage` needs"),
        "{found}"
    );

    let found = wrong(
        r#"[ { "type": "field", "id": "a", "step": 0.0 },
            { "type": "field", "id": "b", "step": "1 s" },
            { "type": "field_reaction", "id": "r1", "when": ["a"] },
            { "type": "field_reaction", "id": "r2", "when": ["a", "b"],
              "uses": { "core:fire": 1 } },
            { "type": "field_reaction", "id": "r3", "when": ["a", { "damage": "core:shock" }] },
            { "type": "field_reaction", "id": "r4", "when": ["a", "a"] } ]"#,
    );
    assert!(found.contains("field w:a step: must be"), "{found}");
    assert!(
        found.contains("field_reaction w:r1 when: a reaction is between two things"),
        "{found}"
    );
    assert!(
        found.contains("field_reaction w:r2 uses: a reaction uses only the fields"),
        "{found}"
    );
    assert!(
        found.contains("field_reaction w:r3 spreads: a reaction to damage needs"),
        "{found}"
    );
    assert!(
        found.contains("field_reaction w:r4 when: a field does not react with itself"),
        "{found}"
    );
}

// ---- storage and edits -----------------------------------------------

#[test]
fn a_field_is_added_read_and_taken_away() {
    let defs = defs();
    let mut w = world_from(Arc::clone(&defs), |_| {});
    let haze = field(&w, "t:haze");
    let spot = w.loc(5, 5, 0);
    let pristine = Hash128::of(&w.cosmos);
    let rev = |w: &World| {
        let view = w.cosmos.grid(w.grid).unwrap();
        view.column(spot.cell.tract())
            .unwrap()
            .chunk(0)
            .unwrap()
            .rev()
    };
    let before = rev(&w);

    assert_eq!(w.ctx().edit(someone()).add_field(spot, haze, 40), Ok(0));
    assert_eq!(w.cosmos.field(spot, haze), Ok(40));
    assert_eq!(w.cosmos.fields_at(spot).unwrap(), [(haze, 40)]);
    assert_ne!(rev(&w).fields, before.fields);
    assert_eq!(rev(&w).structure, before.structure);
    // It is active, and whoever put it there is responsible for it.
    let cell = w.cosmos.field_cell(spot, haze).unwrap().unwrap();
    assert!(cell.awake);
    assert_eq!(cell.by, someone().instigator);
    assert_eq!(w.cosmos.active_field_cells(), 1);
    assert_ne!(Hash128::of(&w.cosmos), pristine);

    // More of it is no news; the last of it going is.
    assert_eq!(
        w.ctx().edit(Cause::DEBUG).add_field(spot, haze, 300),
        Ok(40)
    );
    assert_eq!(w.cosmos.field(spot, haze), Ok(255));
    assert_eq!(
        w.ctx().edit(Cause::DEBUG).add_field(spot, haze, -100),
        Ok(255)
    );
    assert_eq!(w.ctx().edit(Cause::DEBUG).set_field(spot, haze, 0), Ok(155));
    assert_eq!(w.cosmos.field(spot, haze), Ok(0));
    let names: Vec<&str> = w.events.events().iter().map(WorldEvent::name).collect();
    assert_eq!(names, ["field_appeared", "field_vanished"]);
    assert!(matches!(
        w.events.events()[0],
        WorldEvent::FieldAppeared { loc, field, level: 40, cause, .. }
            if loc == spot && field == haze && cause == someone()
    ));
    assert_eq!(w.cosmos.field_state().tracts().count(), 0);

    // Refusals change nothing.
    let nowhere = w.loc(5, 5000, 0);
    assert!(
        w.ctx()
            .edit(Cause::DEBUG)
            .add_field(nowhere, haze, 1)
            .is_err()
    );
    let unknown = FieldId::new(999);
    assert!(
        w.ctx()
            .edit(Cause::DEBUG)
            .add_field(spot, unknown, 1)
            .is_err()
    );
    assert_eq!(w.cosmos.fields_at(spot).unwrap(), []);
}

// ---- rules ---------------------------------------------------------

#[test]
fn diffusion_conserves_the_total_apart_from_decay() {
    let defs = defs();
    let mut w = world_from(Arc::clone(&defs), |_| {});
    let gas = field(&w, "t:gas");
    add(&mut w, at(10, 10, 0), gas, 250);
    add(&mut w, at(11, 10, 0), gas, 200);
    for _ in 0..30 {
        w.run_for(Dur::from_millis(100));
        assert_eq!(total(&w, gas), 450);
    }
    // It has spread out, sideways and upward, and not into the rock.
    assert!(level(&w, at(10, 10, 0), gas) < 100);
    assert!(level(&w, at(8, 10, 0), gas) > 0);
    assert!(level(&w, at(10, 10, 1), gas) > 0);
    assert_eq!(level(&w, at(10, 10, -1), gas), 0);
    // Left alone it evens out as far as whole levels go, and then every
    // cell of it sleeps and nothing steps it.
    w.run_out();
    assert_eq!(total(&w, gas), 450);
    assert!(fields_idle(&w));
    assert!(w.cosmos.processes().is_idle());

    // With decay the total only falls, to nothing.
    let mist = field(&w, "t:mist");
    add(&mut w, at(20, 10, 0), mist, 250);
    let mut last = 250;
    while last > 0 {
        w.run_for(Dur::from_millis(100));
        let now = total(&w, mist);
        assert!(now < last, "{now} {last}");
        last = now;
    }
    w.run_out();
    assert_eq!(total(&w, mist), 0);
    assert!(w.cosmos.processes().is_idle());
}

/// A basin on the ground: a ring of wall round seven cells by three.
fn basin(builder: &mut ColumnBuilder<'_>, defs: &WorldDefs) {
    let wall = defs.block_state("core:wall", None).unwrap();
    for y in 4..=8 {
        for x in 4..=12 {
            if x == 4 || x == 12 || y == 4 || y == 8 {
                builder.set_block(at(x, y, 0), wall);
            }
        }
    }
}

#[test]
fn liquid_falls_and_levels_and_then_lies_still() {
    let defs = defs();
    let mut w = world_from(Arc::clone(&defs), |builder| basin(builder, &defs));
    let water = field(&w, "core:water");
    // Poured out three storeys up, it falls a storey a step.
    add(&mut w, at(8, 6, 3), water, 255);
    w.run_for(Dur::from_millis(250));
    assert_eq!(level(&w, at(8, 6, 3), water), 0);
    assert_eq!(level(&w, at(8, 6, 2), water), 255);
    w.run_out();

    // All of it lies in the basin, on the ground, nearly level.
    assert_eq!(total(&w, water), 255);
    let lying = held(&w, water, at(5, 5, 0), at(11, 7, 0));
    assert_eq!(lying.len(), 21);
    assert_eq!(
        lying
            .iter()
            .map(|(_, level)| u32::from(*level))
            .sum::<u32>(),
        255
    );
    for (cell, here) in &lying {
        for (dx, dy) in [(1, 0), (0, 1)] {
            let beside = at(cell.x + dx, cell.y + dy, 0);
            if let Some((_, there)) = lying.iter().find(|(other, _)| *other == beside) {
                assert!(here.abs_diff(*there) <= 3, "{cell:?} {here} {there}");
            }
        }
    }
    // It sleeps, and nothing steps it.
    assert!(fields_idle(&w));
    assert!(w.cosmos.processes().is_idle());
    let still = w.state_hash();
    w.run_for(Dur::from_secs(5));
    assert_eq!(w.state_hash().0, still.0);

    // A hole opened under it wakes it, and it runs down the hole.
    let (floor, pit) = (w.loc(8, 6, 0), w.loc(8, 6, -1));
    w.ctx()
        .edit(Cause::DEBUG)
        .set_floor(floor, FloorState::NONE)
        .unwrap();
    w.ctx()
        .edit(Cause::DEBUG)
        .set_block(pit, BlockState::NONE)
        .unwrap();
    assert!(w.cosmos.active_field_cells() > 0);
    w.run_out();
    assert_eq!(total(&w, water), 255);
    assert!(level(&w, at(8, 6, -1), water) > 80);
    assert_eq!(level(&w, at(8, 6, 0), water), 0);
    assert!(fields_idle(&w));
}

#[test]
fn fire_spreads_only_where_there_is_fuel_and_burns_out() {
    let defs = defs();
    let wood = planks(&defs);
    let mut w = world_from(Arc::clone(&defs), |builder| {
        lay(builder, wood, (4, 13), (4, 9))
    });
    let (fire, smoke) = (field(&w, "core:fire"), field(&w, "core:smoke"));
    let grass = w.floor("grass");
    let grass_before = w.count_floors(at(0, 0, 0), at(31, 31, 0), grass);
    assert_eq!(w.count_floors(at(0, 0, 0), at(31, 31, 0), wood), 60);

    add(&mut w, at(5, 5, 0), fire, 80);
    w.run_for(Dur::from_secs(3));
    // It feeds on the wood under it and grows.
    assert!(level(&w, at(5, 5, 0), fire) > 150);
    assert!(total(&w, smoke) > 0);
    w.run_out();

    // Every plank has burnt to what its definition leaves; nothing else
    // has changed; and the fire and its smoke are gone.
    assert_eq!(w.count_floors(at(0, 0, 0), at(31, 31, 0), wood), 0);
    assert_eq!(
        w.count_floors(at(4, 4, 0), at(13, 9, 0), char_floor(&defs)),
        60
    );
    assert_eq!(
        w.count_floors(at(0, 0, 0), at(31, 31, 0), grass),
        grass_before
    );
    assert_eq!(total(&w, fire), 0);
    assert_eq!(total(&w, smoke), 0);
    assert!(fields_idle(&w));
    assert!(w.cosmos.processes().is_idle());

    // Fire was never anywhere but on the wood.
    let mut burnt = 0;
    for event in w.events.events() {
        match event {
            WorldEvent::FieldAppeared { loc, field, .. } if *field == fire => {
                assert!((4..=13).contains(&loc.cell.x) && (4..=9).contains(&loc.cell.y));
                assert_eq!(loc.cell.z, 0);
            }
            WorldEvent::ElementDestroyed {
                layer, how, cause, ..
            } => {
                // What it destroyed traces to whoever lit it.
                assert_eq!(*layer, Layer::Floor);
                assert_eq!(*how, Destruction::Damage(w.damage_type("core:heat")));
                assert_eq!(cause.kind, CauseKind::Field);
                assert_eq!(cause.instigator, someone().instigator);
                burnt += 1;
            }
            _ => {}
        }
    }
    assert_eq!(burnt, 60);
}

#[test]
fn the_order_fields_are_put_down_in_does_not_matter() {
    let defs = defs();
    let wood = planks(&defs);
    let run = |order: &[usize]| {
        let mut w = world_from(Arc::clone(&defs), |builder| {
            lay(builder, wood, (2, 31), (4, 9))
        });
        let (fire, water, gas) = (
            field(&w, "core:fire"),
            field(&w, "core:water"),
            field(&w, "t:gas"),
        );
        // Fires in two tracts, water beside one, gas over the other.
        let puts = [
            (at(5, 5, 0), fire, 90),
            (at(30, 6, 0), fire, 120),
            (at(40, 5, 0), fire, 60),
            (at(6, 5, 0), water, 200),
            (at(40, 6, 1), gas, 255),
        ];
        for index in order {
            let (cell, field, amount) = puts[*index];
            add(&mut w, cell, field, amount);
        }
        w.events.clear();
        w.run_for(Dur::from_secs(12));
        // What the cells hold and what was reported. The processes are
        // numbered in the order they began, which is not a result.
        let view = w.cosmos.grid(w.grid).unwrap();
        let cells: Vec<Hash128> = view
            .loaded_tracts()
            .into_iter()
            .map(|tract| Hash128::of(view.column(tract).unwrap()))
            .collect();
        (cells, Hash128::of(&w.events.drain()))
    };
    let first = run(&[0, 1, 2, 3, 4]);
    assert_eq!(run(&[4, 3, 2, 1, 0]), first);
    assert_eq!(run(&[2, 4, 0, 3, 1]), first);
}

// ---- reactions: the body of `quench` --------------------------------

#[test]
fn quench() {
    let defs = defs();
    let wood = planks(&defs);
    let mut w = world_from(Arc::clone(&defs), |builder| {
        lay(builder, wood, (20, 22), (20, 22))
    });
    let (fire, water, steam) = (
        field(&w, "core:fire"),
        field(&w, "core:water"),
        field(&w, "core:steam"),
    );

    // One part of water puts out three of fire and leaves two of steam.
    // Here the water runs out first: 20 of it take 60 of the fire.
    add(&mut w, at(5, 5, 0), fire, 90);
    add(&mut w, at(5, 5, 0), water, 20);
    // And here the fire does: 30 of it take 10 of the water.
    add(&mut w, at(12, 12, 0), fire, 30);
    add(&mut w, at(12, 12, 0), water, 50);
    w.run_for(Dur::from_millis(250));
    assert_eq!(level(&w, at(5, 5, 0), water), 0);
    assert_eq!(level(&w, at(5, 5, 0), fire), 30);
    assert_eq!(level(&w, at(5, 5, 0), steam), 40);
    assert_eq!(level(&w, at(12, 12, 0), fire), 0);
    assert_eq!(level(&w, at(12, 12, 0), steam), 20);
    // The water that is left runs off as water does.
    assert_eq!(total(&w, water), 40);

    // A field meeting a tagged material reacts: fire on wood grows by
    // what the reaction gives, less its own decay; on grass it only
    // decays.
    add(&mut w, at(21, 21, 0), fire, 40);
    add(&mut w, at(25, 25, 0), fire, 40);
    w.run_until(Tick(500_000));
    assert_eq!(level(&w, at(21, 21, 0), fire), 40 + 30 - 12);
    assert_eq!(level(&w, at(25, 25, 0), fire), 40 - 12);

    // A field's own `convert` gives what the equivalent reaction gives.
    let name = |text: &str| field(&w, text);
    let (acid, lye, salt) = (name("t:acid"), name("t:lye"), name("t:salt"));
    let (acid2, lye2, salt2) = (name("t:acid2"), name("t:lye2"), name("t:salt2"));
    for (cell, sour, base) in [(at(3, 20, 0), acid, lye), (at(3, 24, 0), acid2, lye2)] {
        add(&mut w, cell, sour, 10);
        add(&mut w, cell, base, 30);
    }
    w.run_for(Dur::from_millis(100));
    for (cell, sour, base, made) in [
        (at(3, 20, 0), acid, lye, salt),
        (at(3, 24, 0), acid2, lye2, salt2),
    ] {
        assert_eq!(level(&w, cell, sour), 0);
        assert_eq!(level(&w, cell, base), 10);
        assert_eq!(level(&w, cell, made), 30);
    }
}

#[test]
fn a_shock_in_water_reaches_the_water_joined_to_it() {
    let defs = defs();
    let mut w = world_from(Arc::clone(&defs), |_| {});
    let water = field(&w, "core:water");
    // A run of water ten cells long, and a puddle apart from it.
    for x in 5..=14 {
        add(&mut w, at(x, 5, 0), water, 60);
    }
    add(&mut w, at(20, 5, 0), water, 60);
    let stand = |w: &mut World, slot: u32, x: i32| {
        let id = EntityId::new(slot, 0);
        let loc = w.loc(x, 5, 0);
        w.ctx()
            .place(
                Occupant::new(id, OccupantKind::Creature),
                loc,
                Footprint::SINGLE,
                Cause::DEBUG,
            )
            .unwrap();
        id
    };
    let (at_source, near, far, apart) = (
        stand(&mut w, 10, 5),
        stand(&mut w, 11, 8),
        stand(&mut w, 12, 13),
        stand(&mut w, 13, 20),
    );
    w.events.clear();

    let shock = w.damage_type("core:shock");
    let packet = DamagePacket::new(shock, Real::of(1000.0), Real::of(5.0e6), someone());
    let source = w.loc(5, 5, 0);
    // Six cells of reach along the run: the cells from 6 to 11.
    assert_eq!(w.ctx().edit(someone()).conduct(source, packet), Ok(6));
    let struck: Vec<(EntityId, Real, Real)> = w
        .events
        .events()
        .iter()
        .filter_map(|event| match event {
            WorldEvent::OccupantExposed {
                occupant,
                kind,
                energy,
                intensity,
                cause,
                ..
            } => {
                assert_eq!(*kind, shock);
                assert_eq!(*cause, someone());
                Some((*occupant, *energy, *intensity))
            }
            _ => None,
        })
        .collect();
    // Whoever stands in it within reach gets the stated share; not who
    // is where the packet arrived, beyond reach or in other water.
    assert_eq!(struck, [(near, Real::of(800.0), Real::of(4.0e6))]);
    let _ = (at_source, far, apart);
    // Another kind of damage is not passed on, and nor is one that
    // arrives where there is no water.
    let blow = DamagePacket::new(
        w.damage_type("core:impact"),
        Real::of(1000.0),
        Real::of(5.0e6),
        someone(),
    );
    assert_eq!(w.ctx().edit(someone()).conduct(source, blow), Ok(0));
    let dry = w.loc(5, 9, 0);
    assert_eq!(w.ctx().edit(someone()).conduct(dry, packet), Ok(0));
}

// ---- drift ------------------------------------------------------------

fn wind(w: &mut World, velocity: [f64; 3]) {
    let mut spec = AreaSpec::new(
        w.grid,
        AreaShape::Box(CellBox::new(at(0, 0, 0), at(63, 31, 30))),
    );
    spec.ambient = Some(AmbientOverride {
        gravity: None,
        drift: Some(Drift::Flow(velocity.map(Real::of))),
    });
    w.ctx().create_area(spec, Cause::DEBUG).unwrap();
}

#[test]
fn smoke_moves_downwind_and_fire_spreads_further_downwind() {
    let defs = defs();
    let wood = planks(&defs);
    let world = || {
        world_from(Arc::clone(&defs), |builder| {
            lay(builder, wood, (1, 30), (10, 10))
        })
    };
    let sides = |w: &World, field: FieldId, middle: i32| {
        let cells = held(w, field, at(0, 0, 0), at(63, 31, 30));
        let sum = |keep: &dyn Fn(i32) -> bool| -> u32 {
            cells
                .iter()
                .filter(|(cell, _)| keep(cell.x))
                .map(|(_, level)| u32::from(*level))
                .sum()
        };
        (sum(&|x| x < middle), sum(&|x| x > middle))
    };

    // In still air a puff of smoke spreads the same to both sides.
    let mut still = world();
    let smoke = field(&still, "core:smoke");
    add(&mut still, at(16, 20, 0), smoke, 240);
    still.run_for(Dur::from_secs(2));
    let (west, east) = sides(&still, smoke, 16);
    assert!(west > 0);
    assert_eq!(west, east);
    assert_eq!(still.cosmos.field_drift(still.loc(16, 20, 0)), Ok(None));

    // In a wind from the west it is carried east.
    let mut windy = world();
    wind(&mut windy, [1.0, 0.0, 0.0]);
    assert_eq!(
        windy.cosmos.field_drift(windy.loc(16, 20, 0)),
        Ok(Some([Real::ONE, Real::ZERO, Real::ZERO]))
    );
    add(&mut windy, at(16, 20, 0), smoke, 240);
    windy.run_for(Dur::from_secs(2));
    let (west, east) = sides(&windy, smoke, 16);
    assert!(east > 4 * west.max(1), "{west} {east}");

    // Fire on a strip of wood, lit in the middle: in the same time it
    // gets further before the wind than against it.
    let fire = field(&windy, "core:fire");
    let reach = |w: &World| {
        let burnt = char_floor(&w.defs);
        let touched: Vec<i32> = (1..=30)
            .filter(|x| {
                (9..=11).any(|y| {
                    level(w, at(*x, y, 0), fire) > 0 || w.floor_at(*x, y, 0).def() == burnt.def()
                })
            })
            .collect();
        (16 - touched[0], touched[touched.len() - 1] - 16)
    };
    let mut windy = world();
    wind(&mut windy, [3.0, 0.0, 0.0]);
    add(&mut windy, at(16, 10, 0), fire, 120);
    windy.run_for(Dur::from_secs(20));
    let (upwind, downwind) = reach(&windy);
    assert!(downwind > upwind, "{upwind} {downwind}");
    let mut still = world();
    add(&mut still, at(16, 10, 0), fire, 120);
    still.run_for(Dur::from_secs(20));
    let (west, east) = reach(&still);
    assert!(
        downwind > east && upwind < west,
        "{upwind} {downwind} {west} {east}"
    );
}

// ---- occupants ----------------------------------------------------------

#[test]
fn an_occupant_in_a_field_is_reported_dosed_once_a_step() {
    let defs = defs();
    let mut w = world_from(Arc::clone(&defs), |_| {});
    let toxin = field(&w, "t:toxin");
    let (one, big, item, away) = (
        EntityId::new(20, 0),
        EntityId::new(21, 0),
        EntityId::new(22, 0),
        EntityId::new(23, 0),
    );
    let place = |w: &mut World, id: EntityId, kind: OccupantKind, x: i32, wide: u8| {
        let loc = w.loc(x, 5, 0);
        w.ctx()
            .place(
                Occupant::new(id, kind),
                loc,
                Footprint::new(wide, 1, 1),
                Cause::DEBUG,
            )
            .unwrap();
    };
    place(&mut w, one, OccupantKind::Creature, 5, 1);
    place(&mut w, item, OccupantKind::Item, 5, 1);
    place(&mut w, big, OccupantKind::Creature, 8, 2);
    place(&mut w, away, OccupantKind::Creature, 15, 1);

    // The field comes to where they are: each is told once.
    add(&mut w, at(5, 5, 0), toxin, 100);
    add(&mut w, at(8, 5, 0), toxin, 100);
    add(&mut w, at(9, 5, 0), toxin, 150);
    let contacts: Vec<(EntityId, u8)> = w
        .events
        .drain()
        .iter()
        .filter_map(|event| match event {
            WorldEvent::FieldContact {
                occupant, level, ..
            } => Some((*occupant, *level)),
            _ => None,
        })
        .collect();
    assert_eq!(contacts, [(one, 100), (item, 100), (big, 100), (big, 150)]);

    // Each step of the field, each occupant in it is dosed with what
    // the level gives: the strongest cell it fills. The world applies
    // no route: the item is told like the creature.
    w.run_for(Dur::SECOND);
    let doses = w.events.drain();
    assert_eq!(doses.len(), 5 * 3);
    for (step, told) in doses.chunks(3).enumerate() {
        let tick = Tick(200_000 * (step as i64 + 1));
        let expect = [
            (one, 100u8, 2.0, 5),
            (big, 150, 3.0, 9),
            (item, 100, 2.0, 5),
        ];
        for (event, (who, at_level, dose, x)) in told.iter().zip(expect) {
            let WorldEvent::OccupantDosed {
                tick: when,
                occupant,
                loc,
                field,
                level,
                pool,
                amount,
                route,
                cause,
            } = event
            else {
                panic!("{event:?}");
            };
            assert_eq!(
                (*when, *occupant, *field, *level),
                (tick, who, toxin, at_level)
            );
            assert_eq!(*loc, w.loc(x, 5, 0));
            assert_eq!(pool, "toxin");
            assert_eq!(*amount, Real::of(dose));
            assert_eq!(*route, ExposureRoute::Breath);
            assert_eq!(cause.kind, CauseKind::Field);
            assert_eq!(cause.instigator, someone().instigator);
        }
    }

    // Someone who walks into it is told of the contact, and dosed from
    // the next step on; someone who leaves is dosed no more.
    let into = w.loc(5, 5, 0);
    w.ctx().relocate(away, into, Cause::DEBUG).unwrap();
    let out = w.loc(5, 9, 0);
    w.ctx().relocate(one, out, Cause::DEBUG).unwrap();
    assert!(matches!(
        w.events.drain()[..],
        [WorldEvent::FieldContact { occupant, field, level: 100, .. }]
            if occupant == away && field == toxin
    ));
    w.run_for(Dur::from_millis(200));
    let dosed: Vec<EntityId> = w
        .events
        .drain()
        .iter()
        .filter_map(|event| match event {
            WorldEvent::OccupantDosed { occupant, .. } => Some(*occupant),
            _ => None,
        })
        .collect();
    assert_eq!(dosed, [big, item, away]);
}

#[test]
fn fire_exposes_whoever_stands_in_it() {
    let defs = defs();
    let mut w = world_from(Arc::clone(&defs), |_| {});
    let fire = field(&w, "core:fire");
    let who = EntityId::new(30, 0);
    let spot = w.loc(5, 5, 0);
    w.ctx()
        .place(
            Occupant::new(who, OccupantKind::Creature),
            spot,
            Footprint::SINGLE,
            Cause::DEBUG,
        )
        .unwrap();
    add(&mut w, at(5, 5, 0), fire, 100);
    w.events.clear();
    w.run_for(Dur::from_millis(500));
    let heat = w.damage_type("core:heat");
    assert!(w.events.events().iter().any(|event| matches!(
        event,
        WorldEvent::OccupantExposed { occupant, loc, kind, energy, intensity, cause, .. }
            if *occupant == who && *loc == spot && *kind == heat
                && *energy == Real::of(1000.0) && *intensity == Real::of(1.0e6)
                && cause.kind == CauseKind::Field
    )));
}

// ---- clocks -------------------------------------------------------------

#[test]
fn a_fire_in_stopped_time_does_not_burn() {
    let defs = defs();
    let wood = planks(&defs);
    let mut w = world_from(Arc::clone(&defs), |builder| {
        lay(builder, wood, (4, 13), (4, 9))
    });
    let fire = field(&w, "core:fire");
    let mut spec = AreaSpec::new(
        w.grid,
        AreaShape::Box(CellBox::new(at(4, 4, 0), at(8, 9, 3))),
    );
    spec.clock_rate = Some(Real::ZERO);
    let bubble = w.ctx().create_area(spec, Cause::DEBUG).unwrap();

    // Lit inside the bubble it hangs; lit outside it burns.
    add(&mut w, at(5, 5, 0), fire, 80);
    add(&mut w, at(12, 8, 0), fire, 80);
    w.run_for(Dur::from_secs(4));
    assert_eq!(level(&w, at(5, 5, 0), fire), 80);
    assert_eq!(w.floor_at(5, 5, 0), wood);
    assert!(level(&w, at(12, 8, 0), fire) > 80);
    // What spreads into the bubble from outside hangs there too.
    w.run_for(Dur::from_secs(40));
    assert_eq!(level(&w, at(5, 5, 0), fire), 80);
    assert_eq!(w.count_floors(at(9, 4, 0), at(13, 9, 0), wood), 0);
    assert_eq!(w.count_floors(at(4, 4, 0), at(8, 9, 0), wood), 30);
    let hung = held(&w, fire, at(4, 4, 0), at(8, 9, 0));
    assert!(hung.len() > 1);

    // When time moves again, so does the fire.
    w.ctx()
        .set_area_clock_rate(bubble, Some(Real::ONE))
        .unwrap();
    w.run_for(Dur::from_millis(600));
    assert!(level(&w, at(5, 5, 0), fire) > 80);
    // And when the bubble goes altogether it burns out as any fire.
    w.ctx().remove_area(bubble, Cause::DEBUG).unwrap();
    w.run_out();
    assert_eq!(w.count_floors(at(4, 4, 0), at(13, 9, 0), wood), 0);
    assert_eq!(total(&w, fire), 0);
    assert!(fields_idle(&w));
    assert!(w.cosmos.processes().is_idle());
}

// ---- the budget -----------------------------------------------------------

#[test]
fn over_the_budget_the_weakest_cells_decay_faster() {
    let defs = defs();
    let run = || {
        let mut w = world_from(Arc::clone(&defs), |_| {});
        let haze = field(&w, "t:haze");
        w.cosmos.set_field_max_active_cells(150);
        // A hundred strong cells and a hundred weak ones, ten at each
        // level from 20 to 29.
        for i in 0..100 {
            add(&mut w, at(i % 20, 2 + i / 20, 0), haze, 200);
            add(
                &mut w,
                at(i % 20, 12 + i / 20, 0),
                haze,
                20 + (i % 10) as i16,
            );
        }
        w.events.clear();
        assert_eq!(w.cosmos.active_field_cells(), 200);
        w.run_for(Dur::from_millis(100));
        (w, haze)
    };
    let (mut w, haze) = run();
    // Fifty too many: the weakest fifty, those at 24 and below, lose
    // sixteen levels on top of their own decay. The rest decay as ever.
    assert!(matches!(
        w.events.events()[..],
        [WorldEvent::FieldBudgetExceeded {
            active: 200,
            max: 150,
            ..
        }]
    ));
    assert!(w.cosmos.field_state().is_over_budget());
    assert_eq!(level(&w, at(0, 2, 0), haze), 199);
    assert_eq!(level(&w, at(0, 12, 0), haze), 20 - 17);
    assert_eq!(level(&w, at(4, 12, 0), haze), 24 - 17);
    assert_eq!(level(&w, at(5, 12, 0), haze), 25 - 1);
    // It is said once. At the next step the weakest fifty are gone,
    // and with them the excess.
    w.events.clear();
    w.run_for(Dur::from_millis(100));
    assert_eq!(w.cosmos.active_field_cells(), 150);
    assert!(
        w.events
            .events()
            .iter()
            .all(|event| event.name() == "field_vanished")
    );
    assert_eq!(w.events.len(), 50);
    w.run_for(Dur::from_millis(100));
    assert!(!w.cosmos.field_state().is_over_budget());
    assert_eq!(level(&w, at(5, 12, 0), haze), 25 - 3);

    // And it is the same every time.
    let (again, _) = run();
    let (mut first, _) = run();
    assert_eq!(first.state_hash(), again.state_hash());
    first.run_out();
    assert!(first.cosmos.processes().is_idle());
}

// ---- sight and light ---------------------------------------------------------

#[test]
fn fields_say_what_they_hide_and_what_light_they_give() {
    let defs = defs();
    let mut w = world_from(Arc::clone(&defs), |_| {});
    let (fire, smoke) = (field(&w, "core:fire"), field(&w, "core:smoke"));
    add(&mut w, at(5, 5, 0), smoke, 200);
    add(&mut w, at(6, 5, 0), smoke, 100);
    assert_eq!(w.cosmos.field_hides_sight(w.loc(5, 5, 0)), Ok(true));
    assert_eq!(w.cosmos.field_hides_sight(w.loc(6, 5, 0)), Ok(false));
    assert_eq!(w.cosmos.field_hides_sight(w.loc(7, 5, 0)), Ok(false));
    assert!(w.cosmos.field_hides_sight(w.loc(7, 5000, 0)).is_err());

    add(&mut w, at(10, 5, 0), fire, 255);
    add(&mut w, at(11, 5, 0), fire, 85);
    add(&mut w, at(12, 5, 0), fire, 10);
    let lights = w
        .cosmos
        .field_lights(w.grid, CellBox::new(at(0, 0, 0), at(31, 31, 0)))
        .unwrap();
    let listed: Vec<(i32, Real, Real)> = lights
        .iter()
        .map(|light| (light.at.cell.x, light.source.intensity, light.source.radius))
        .collect();
    assert_eq!(
        listed,
        [
            (10, Real::of(1.5), Real::of(10.0)),
            (
                11,
                Real::of(1.5) * (Real::of(85.0) / Real::of(255.0)),
                Real::of(10.0)
            )
        ]
    );
}

// ---- the body of `office_fire` -------------------------------------------------

/// Two closed storeys: walls round a room with a wooden floor, a slab
/// over it with a stairwell in one corner, a second room above and a
/// roof over that.
fn office(builder: &mut ColumnBuilder<'_>, defs: &WorldDefs) {
    let wall = defs.block_state("core:wall", None).unwrap();
    let slab = defs.floor_state("core:slab", None).unwrap();
    let stairs = defs.block_state("core:stairs", None).unwrap();
    let wood = planks(defs);
    for y in 4..=10 {
        for x in 4..=12 {
            let edge = x == 4 || x == 12 || y == 4 || y == 10;
            for z in 0..=1 {
                if edge {
                    builder.set_block(at(x, y, z), wall);
                }
            }
            if !edge {
                builder.set_floor(at(x, y, 0), wood);
            }
            if (x, y) != (11, 9) {
                builder.set_floor(at(x, y, 1), slab);
            }
            builder.set_floor(at(x, y, 2), slab);
        }
    }
    builder.set_block(at(11, 9, 0), stairs);
}

fn office_world(defs: &Arc<WorldDefs>) -> World {
    let mut w = world_from(Arc::clone(defs), |builder| office(builder, defs));
    let fire = field(&w, "core:fire");
    add(&mut w, at(6, 6, 0), fire, 80);
    w
}

#[test]
fn office_fire() {
    let defs = defs();
    let mut straight = office_world(&defs);
    let (fire, smoke) = (
        field(&straight, "core:fire"),
        field(&straight, "core:smoke"),
    );
    let wood = planks(&defs);

    // Twenty seconds in, the room is full of smoke, and it has risen
    // through the stairwell into the room above. None is outside.
    straight.run_until(Tick(20_000_000));
    let below = held(&straight, smoke, at(5, 5, 0), at(11, 9, 0));
    assert_eq!(below.len(), 35, "{below:?}");
    assert!(level(&straight, at(11, 9, 1), smoke) > 0);
    let above = held(&straight, smoke, at(5, 5, 1), at(11, 9, 1));
    assert!(above.len() >= 6, "{above:?}");
    let everywhere = held(&straight, smoke, at(0, 0, -8), at(63, 31, 40));
    assert_eq!(everywhere.len(), below.len() + above.len());
    assert!(total(&straight, fire) > 0);
    // Thick smoke hides what is behind it.
    assert!(below.iter().any(|(cell, _)| {
        straight
            .cosmos
            .field_hides_sight(straight.loc(cell.x, cell.y, cell.z))
            .unwrap()
    }));

    // It burns out: the floor is gone to char, the walls stand, and in
    // the end the smoke clears.
    straight.run_until(Tick(60_000_000));
    let after_a_minute = straight.state_hash();
    straight.run_out();
    assert_eq!(straight.count_floors(at(4, 4, 0), at(12, 10, 0), wood), 0);
    assert_eq!(
        straight.count_blocks(at(4, 4, 0), at(12, 10, 1), straight.block("wall")),
        2 * 28
    );
    assert_eq!(total(&straight, fire) + total(&straight, smoke), 0);
    assert!(straight.cosmos.processes().is_idle());
    assert!(fields_idle(&straight));

    // The same fire stepped in a hundred small advances, and in
    // advances that fall between its steps.
    let mut sliced = office_world(&defs);
    for _ in 0..100 {
        sliced.run_for(Dur::from_millis(600));
    }
    assert_eq!(sliced.state_hash(), after_a_minute);
    let mut odd = office_world(&defs);
    for _ in 0..(60_000_000 / 37_500) {
        odd.run_for(Dur::from_micros(37_500));
    }
    assert_eq!(odd.state_hash(), after_a_minute);

    // The same, saved in mid-fire and restored.
    let mut first = office_world(&defs);
    first.run_until(Tick(7_300_000));
    assert!(first.cosmos.active_field_cells() > 20);
    let saved_timeline = codec::to_bytes(&first.timeline).unwrap();
    let saved_activity = codec::to_bytes(first.cosmos.processes()).unwrap();
    let timeline: Timeline = codec::from_bytes(&saved_timeline).unwrap();
    let activity: WorldActivity = codec::from_bytes(&saved_activity).unwrap();
    assert_eq!(&activity, first.cosmos.processes());
    // The cells are written as a tract is stored, fields and all.
    let view = first.cosmos.grid(first.grid).unwrap();
    let column = view.column(clash_world::TractPos::new(0, 0)).unwrap();
    let mut ids = clash_world::SaveIds::new(&defs);
    let blob = clash_world::encode_tract(&defs, &mut ids, Some(column), &[]).unwrap();
    let column = clash_world::decode_tract(&defs, &ids, &blob)
        .unwrap()
        .cells
        .unwrap();
    let mut cosmos = first.cosmos.clone();
    cosmos.restore_activity(WorldActivity::new());
    clash_world::test_support::load_column(
        &mut cosmos,
        first.grid,
        clash_world::TractPos::new(0, 0),
        column,
    )
    .unwrap();
    cosmos.restore_activity(activity);
    let mut second = World {
        defs: Arc::clone(&first.defs),
        timeline,
        cosmos,
        events: EventSink::new(),
        grid: first.grid,
    };
    assert_eq!(second.state_hash(), first.state_hash());
    second.run_until(Tick(60_000_000));
    assert_eq!(second.state_hash(), after_a_minute);
    second.run_out();
    assert_eq!(second.state_hash(), straight.state_hash());
    let mut told = first.events.drain();
    told.extend(second.events.drain());
    assert_eq!(Hash128::of(&told), Hash128::of(&straight.events.drain()));
}

// ---- timing ----------------------------------------------------------------------

#[test]
#[ignore = "timing; run by hand in release mode"]
#[allow(
    clippy::disallowed_types,
    clippy::disallowed_methods,
    reason = "a timing test reads the wall clock; nothing in the simulation does"
)]
fn timing_of_a_building_fire() {
    let defs = defs();
    let wood = planks(&defs);
    // Six storeys of wooden floor sixty cells by twenty-eight, open to
    // each other at one end, alight in a dozen places on every storey.
    let mut timeline = Timeline::new();
    let mut cosmos = clash_world::Cosmos::new(Arc::clone(&defs));
    let def = defs.realms().lookup("core:testland").unwrap();
    let realm = cosmos.create_realm(&mut timeline, def, 7).unwrap();
    let grid = cosmos.realm(realm).unwrap().base_grid;
    let rock = defs.block_state("core:solid", None).unwrap();
    for tract in [
        clash_world::TractPos::new(0, 0),
        clash_world::TractPos::new(1, 0),
    ] {
        let mut builder = ColumnBuilder::new(&defs, rock, BlockState::NONE, 0);
        for z in 0..6 {
            for y in 2..30 {
                for x in 0..32 {
                    if z == 0 || (tract.x, x) != (0, 2) {
                        builder.set_floor(at(x, y, z), wood);
                    }
                }
            }
        }
        clash_world::test_support::load_column(&mut cosmos, grid, tract, builder.build()).unwrap();
    }
    let mut w = World {
        defs: Arc::clone(&defs),
        timeline,
        cosmos,
        events: EventSink::new(),
        grid,
    };
    let fire = field(&w, "core:fire");
    for z in 0..6 {
        for i in 0..12 {
            add(
                &mut w,
                at(4 + 5 * i, 4 + (7 * i + 3 * z) % 24, z),
                fire,
                120,
            );
        }
    }

    let mut steps = 0u32;
    let mut most = 0u64;
    let mut cell_steps = 0u64;
    let mut slowest = 0.0f64;
    let began = std::time::Instant::now();
    while w.cosmos.active_field_cells() > 0 && steps < 400 {
        let active = w.cosmos.active_field_cells();
        let one = std::time::Instant::now();
        w.run_for(Dur::from_millis(500));
        slowest = slowest.max(one.elapsed().as_secs_f64());
        w.events.clear();
        most = most.max(active);
        cell_steps += active;
        steps += 1;
    }
    let took = began.elapsed().as_secs_f64();
    println!(
        "a building fire: {steps} steps of fire and of smoke in {:.1} ms; at most {most} active \
         cells; {:.0} ns for each active cell each step; the slowest step {:.2} ms",
        took * 1.0e3,
        took * 1.0e9 / cell_steps.max(1) as f64,
        slowest * 1.0e3
    );
    assert!(most > 2000, "{most}");
}
