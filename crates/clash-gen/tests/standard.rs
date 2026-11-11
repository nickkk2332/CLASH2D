//! Tests for WG4: tract definitions and the `standard` generator
//! (10 §6), and the countryside bundled with the `core` mod.
//!
//! The pictures of two tracts of the countryside at a fixed seed are
//! kept as text in `tests/golden/countryside.txt`. Run the test with
//! `BLESS=1` to rewrite them, look at the difference, and say why in
//! the change's description.

mod common;

use std::fmt::Write as _;
use std::sync::Arc;

use clash_core::{Hash128, Rng};
use clash_gen::place::gen_map_id;
use clash_gen::{
    GenError, RayonRunner, SerialRunner, StandardGenerator, placement_box, realize_tracts,
};
use clash_world::dims::{REGION_TRACT_COUNT, TRACT_XY};
use clash_world::{CellPos, RegionPos, TractPos};

use common::COUNTRYSIDE_REALM;

/// The seed the stored pictures are made with.
const GOLDEN_SEED: u64 = 7;

/// The environment variable that rewrites the stored pictures.
const BLESS_VAR: &str = "BLESS";

/// The tract of the bundled countryside that holds a lane with houses.
const LANE: TractPos = TractPos::new(1, 1);

/// A tract of the bundled countryside that is a small wood.
const WOOD: TractPos = TractPos::new(0, 0);

const SEED: u64 = 0xC0FFEE;

fn bundled() -> common::Built {
    common::bundled().unwrap_or_else(|error| panic!("mods/core does not build:\n{error}"))
}

#[test]
fn realising_gives_the_same_tracts_in_any_order_and_on_any_thread_count() {
    let built = bundled();
    let def = built.realm(COUNTRYSIDE_REALM).unwrap();
    let tracts = common::block(4);
    let hashes = |threads: usize, order: &[TractPos]| -> Vec<(TractPos, Hash128)> {
        let mut registry = clash_gen::registry_with(&built.generation);
        if threads == 0 {
            registry.set_runner(Arc::new(SerialRunner));
        } else {
            registry.set_runner(Arc::new(RayonRunner::with_threads(threads).unwrap()));
        }
        let generators = registry.bind(&built.world);
        let realised = realize_tracts(&generators, def, SEED, order).unwrap();
        let mut hashes: Vec<(TractPos, Hash128)> = order
            .iter()
            .copied()
            .zip(realised.iter().map(|tract| {
                let mut hasher = clash_core::StateHasher::new();
                clash_core::StateHash::state_hash(&tract.column, &mut hasher);
                clash_core::StateHash::state_hash(&tract.spawns, &mut hasher);
                hasher.write_len(tract.areas.len());
                hasher.finish()
            }))
            .collect();
        hashes.sort_by_key(|(tract, _)| *tract);
        hashes
    };
    let straight = hashes(0, &tracts);
    assert!(straight.windows(2).any(|pair| pair[0].1 != pair[1].1));
    let mut shuffled = tracts.clone();
    Rng::from_seed(5).shuffle(&mut shuffled);
    assert_ne!(shuffled, tracts);
    assert_eq!(hashes(0, &shuffled), straight);
    for threads in [1, 2, 7] {
        assert_eq!(hashes(threads, &tracts), straight, "{threads} threads");
        assert_eq!(hashes(threads, &shuffled), straight, "{threads} threads");
    }
    for (tract, hash) in &straight {
        assert_eq!(hashes(0, &[*tract]), [(*tract, *hash)]);
    }
    // Another seed is another countryside.
    assert_ne!(hashes(0, &tracts), {
        let generators = built.generators();
        let other = realize_tracts(&generators, def, SEED + 1, &tracts).unwrap();
        tracts
            .iter()
            .copied()
            .zip(other.iter().map(|tract| Hash128::of(&tract.column)))
            .collect::<Vec<_>>()
    });
}

#[test]
fn a_world_loaded_in_any_order_hashes_the_same() {
    // The cells of every tract, and how many areas the world has. The
    // ids of the areas are given out in the order tracts first load,
    // so the hash of the whole cosmos is not compared.
    let tracts = common::block(3);
    let summary = |live: &common::Live, areas: usize| -> (Vec<Hash128>, usize) {
        let view = live.cosmos.grid(live.grid).unwrap();
        let cells = tracts
            .iter()
            .map(|tract| Hash128::of(view.column(*tract).expect("the tract is loaded")))
            .collect();
        (cells, areas)
    };
    let loaded = |order: &[TractPos]| {
        let mut live = common::Live::new(bundled(), COUNTRYSIDE_REALM, SEED);
        let first = live.load(order);
        assert!(first.iter().all(|tract| tract.first));
        let areas = first.iter().map(|tract| tract.areas.len()).sum::<usize>();
        // The homes and the shops of the lane are areas of the world.
        assert!(areas >= 6, "{areas} areas");
        summary(&live, areas)
    };
    let straight = loaded(&tracts);
    let mut shuffled = tracts.clone();
    Rng::from_seed(11).shuffle(&mut shuffled);
    assert_eq!(loaded(&shuffled), straight);
    // One at a time is the same again.
    let mut live = common::Live::new(bundled(), COUNTRYSIDE_REALM, SEED);
    let mut areas = 0;
    for tract in &shuffled {
        areas += live.load(&[*tract])[0].areas.len();
    }
    assert_eq!(summary(&live, areas), straight);
}

#[test]
fn a_survey_gives_each_tract_its_definition_and_keeps_the_placements() {
    let built = bundled();
    let generation = Arc::clone(&built.generation);
    let tract_def = |name: &str| generation.lookup_tract(name).unwrap();
    let (field, wood, lane) = (
        tract_def("core:field"),
        tract_def("core:wood"),
        tract_def("core:lane"),
    );
    let mut live = common::Live::new(built, COUNTRYSIDE_REALM, SEED);
    let realm = live.realm;
    let region = RegionPos::new(0, 0);
    assert_eq!(
        live.cosmos.survey_region(&live.generators, realm, region),
        Ok(true)
    );
    let record = live.cosmos.region(realm, region).unwrap().clone();

    // The layout of the realm definition, as written.
    let def_at = |x: i32, y: i32| record.tract(TractPos::new(x, y)).def;
    assert_eq!(
        [def_at(0, 0), def_at(1, 0), def_at(2, 0)],
        [wood, field, field]
    );
    assert_eq!(
        [def_at(0, 1), def_at(1, 1), def_at(3, 1)],
        [lane, lane, lane]
    );
    assert_eq!(
        [def_at(0, 2), def_at(1, 2), def_at(2, 2)],
        [field, wood, field]
    );
    // Beyond it, the weighted list: five parts field to three of wood,
    // in patches two tracts on a side.
    let (mut fields, mut woods) = (0, 0);
    for index in 0..REGION_TRACT_COUNT {
        let tract = region.tract(index);
        if tract.y < 4 {
            continue;
        }
        let def = record.tract(tract).def;
        assert!(def == field || def == wood, "{tract:?}");
        fields += u32::from(def == field);
        woods += u32::from(def == wood);
        let patch = TractPos::new(
            tract.x - tract.x.rem_euclid(2),
            tract.y - tract.y.rem_euclid(2),
        );
        assert_eq!(def, record.tract(patch).def, "{tract:?} and its patch");
    }
    let share = f64::from(fields) / f64::from(fields + woods);
    assert!(
        (0.55..0.70).contains(&share),
        "{share} of the land is field"
    );

    // The record holds what stands on the region (02 §9.1): exactly
    // what the realiser of each tract works out for itself.
    let generator = StandardGenerator::of_realm(&generation, COUNTRYSIDE_REALM).unwrap();
    let maps = generation.maps();
    let in_tract = |tract: TractPos| -> Vec<_> {
        record
            .placements
            .iter()
            .filter(|placement| placement.origin.tract() == tract)
            .cloned()
            .collect()
    };
    for tract in [LANE, TractPos::new(0, 1), TractPos::new(3, 1), WOOD] {
        let (written, dropped) = generator.placements_of(SEED, tract, record.tract(tract).def);
        assert!(dropped.is_empty());
        assert_eq!(in_tract(tract), written, "{tract:?}");
    }
    let on_lane = in_tract(LANE);
    assert!(
        (3..=5).contains(&on_lane.len()),
        "{} placements",
        on_lane.len()
    );
    assert_eq!(
        maps.name(gen_map_id(on_lane[0].map)),
        "core:lane_strip",
        "the lane itself is the first placement"
    );
    assert!(in_tract(WOOD).is_empty());
    for placement in &on_lane {
        let bounds = placement_box(maps, placement).unwrap();
        assert_eq!(
            bounds.tracts().collect::<Vec<_>>(),
            [LANE],
            "inside its tract"
        );
    }
    // A survey is a pure function of the seed and the position.
    let mut again = common::Live::new(bundled(), COUNTRYSIDE_REALM, SEED);
    let realm = again.realm;
    again
        .cosmos
        .survey_region(&again.generators, realm, region)
        .unwrap();
    assert_eq!(again.cosmos.region(realm, region), Some(&record));
}

#[test]
fn the_ground_is_laid_first_and_placements_over_it() {
    let built = bundled();
    let generator = StandardGenerator::of_realm(&built.generation, COUNTRYSIDE_REALM).unwrap();
    let maps = built.generation.maps();
    let realised = common::realise(&built, COUNTRYSIDE_REALM, SEED, &[LANE]).unwrap();
    let column = &realised[0].column;
    let world = &built.world;
    let grass = world.floor_state("grass", None).unwrap();
    let scattered: Vec<_> = ["tree", "bush", "boulder"]
        .iter()
        .map(|name| world.fixture_state(name, None).unwrap())
        .collect();
    let (written, _) = generator.placements_of(SEED, LANE, generator.tract_def(SEED, LANE));
    let mut houses = 0;
    for placement in &written {
        let name = maps.name(gen_map_id(placement.map));
        if name == "core:lane_strip" {
            continue;
        }
        houses += 1;
        // Invariant X5: inside a building's footprint, at its ground
        // storey, nothing of the ground shows: no grass, and nothing
        // the ground scattered.
        let bounds = placement_box(maps, placement).unwrap();
        let level = placement.origin.z;
        for y in bounds.min.y..=bounds.max.y {
            for x in bounds.min.x..=bounds.max.x {
                let cell = column.cell(CellPos::new(x, y, level));
                assert_ne!(cell.floor, grass, "{name} at {x}, {y}");
                assert!(!cell.floor.is_none(), "{name} at {x}, {y}");
                assert!(!scattered.contains(&cell.fixture), "{name} at {x}, {y}");
                // The ground was levelled under it and then built on.
                assert!(!column.cell(CellPos::new(x, y, level - 1)).block.is_none());
            }
        }
    }
    assert!(houses >= 2, "{houses} buildings on the lane");
    // Outside every footprint the ground is as the ground laid it.
    let free = (0..TRACT_XY).any(|x| {
        let origin = LANE.origin(0);
        let at = CellPos::new(origin.x + x, origin.y, 0);
        let surface = clash_gen::ascii::surface_storey(column, at);
        column.cell(CellPos::new(at.x, at.y, surface)).floor == grass
    });
    assert!(free);
    // What the maps ask for besides cells came with the tract.
    assert!(realised[0].areas.len() >= 2, "a home and a shop at least");
}

fn countryside_pictures(built: &common::Built) -> String {
    let mut out = String::new();
    for (tract, name, storey) in [(LANE, "lane", 0), (LANE, "lane", 1), (WOOD, "wood", 0)] {
        writeln!(
            out,
            "== {name}: tract {}, {} of {COUNTRYSIDE_REALM}, seed {GOLDEN_SEED}, storey {storey} ==",
            tract.x, tract.y
        )
        .unwrap();
        out.push_str(
            &common::tract_text(built, COUNTRYSIDE_REALM, GOLDEN_SEED, &[tract], storey).unwrap(),
        );
        out.push('\n');
    }
    out
}

#[test]
fn the_bundled_countryside_looks_as_it_did_and_stands() {
    let built = bundled();
    let now = countryside_pictures(&built);
    let path = common::mods_dir().join("../crates/clash-gen/tests/golden/countryside.txt");
    if std::env::var_os(BLESS_VAR).is_some_and(|value| value == "1") {
        std::fs::write(&path, &now).unwrap();
    } else {
        let stored = std::fs::read_to_string(&path).unwrap_or_else(|_| {
            panic!(
                "no stored pictures at {}: run the test with {BLESS_VAR}=1 to write them, then \
                 check them by eye",
                path.display()
            )
        });
        if stored != now {
            let line = stored
                .lines()
                .zip(now.lines())
                .position(|(stored, now)| stored != now)
                .map_or_else(
                    || stored.lines().count().min(now.lines().count()) + 1,
                    |index| index + 1,
                );
            panic!(
                "the countryside differs from {} at line {line}\n  stored: {:?}\n  now:    {:?}\n\
                 run with {BLESS_VAR}=1 to accept the change, and say why in the change's \
                 description",
                path.display(),
                stored.lines().nth(line - 1).unwrap_or(""),
                now.lines().nth(line - 1).unwrap_or(""),
            );
        }
    }

    // Every placed building must stand: nothing in either tract is
    // unsupported or crushed as generated (02 §14; 10 X9).
    let mut live = common::Live::new(built, COUNTRYSIDE_REALM, GOLDEN_SEED);
    live.load(&[LANE, WOOD]);
    for tract in [LANE, WOOD] {
        let found = live.unsupported(tract);
        assert!(found.is_empty(), "{tract:?} does not stand: {found:?}");
    }
}

#[test]
fn every_lane_of_the_countryside_stands_over_many_seeds() {
    for seed in 0..6 {
        let mut live = common::Live::new(bundled(), COUNTRYSIDE_REALM, seed);
        let lanes = [TractPos::new(0, 1), LANE, TractPos::new(2, 1)];
        live.load(&lanes);
        for tract in lanes {
            let found = live.unsupported(tract);
            assert!(found.is_empty(), "seed {seed}, {tract:?}: {found:?}");
        }
    }
}

fn realm(id: &str, params: &str) -> String {
    format!(
        r#"{{ "type": "realm", "id": "{id}", "size_regions": [1, 1], "wrap": [],
             "z_range": [-16, 48], "generator": "standard",
             "generator_params": {{ {params} }},
             "fill": {{ "below": "solid@granite", "above": null }},
             "strata": [ {{ "from_z": -16, "gravity": 9.81, "atmosphere": "air" }} ],
             "time_rate": 1.0 }}"#
    )
}

#[test]
fn a_tract_with_no_definition_is_the_realm_s_fill_up_to_the_surface() {
    let built = common::with_core(&format!("[{}]", realm("empty", r#""surface_z": 3"#)));
    let realised = common::realise(&built, "t:empty", 1, &[TractPos::new(4, 4)]).unwrap();
    let column = &realised[0].column;
    let granite = built.world.block_state("solid@granite", None).unwrap();
    let at = TractPos::new(4, 4).origin(0);
    assert_eq!(column.cell(CellPos::new(at.x, at.y, 2)).block, granite);
    assert!(column.cell(CellPos::new(at.x, at.y, 3)).is_empty());
    assert_eq!(column.stored_range().map(|(low, high)| high - low), Some(0));
}

#[test]
fn a_realm_s_parameters_are_checked_when_the_generators_are_bound() {
    let ground = r#"{ "type": "ground", "id": "g",
                      "steps": [ { "strata": [ { "block": "solid@granite" } ] } ] }"#;
    let tract = r#"{ "type": "tract", "id": "plain", "ground": "g" }"#;
    let built = common::with_core(&format!(
        "[{ground}, {tract}, {}, {}, {}, {}, {}, {}]",
        realm("good", r#""tracts": [ { "tract": "plain", "weight": 2 } ]"#),
        realm("no_tract", r#""tracts": [ { "tract": "plian" } ]"#),
        realm("no_param", r#""tract": [ ]"#),
        realm(
            "bad_field",
            r#""height": { "noise": { "octaves": 0, "scale": 3 } }"#
        ),
        realm(
            "bad_legend",
            r#""layout": { "rows": [ "ab" ], "legend": { "a": "plain" } }"#
        ),
        realm(
            "bad_place",
            r#""place": [ { "map": "hut", "at": [1, 2], "turn": 45 } ]"#
        ),
    ));
    let of = |name: &str| StandardGenerator::of_realm(&built.generation, name);
    assert!(of("t:good").is_ok());
    let refused = |name: &str, path: &str, says: &str| match of(name) {
        Err(GenError::BadParam {
            path: at, message, ..
        }) => {
            assert_eq!(at, path, "{name}");
            assert!(message.contains(says), "{name}: {message}");
        }
        other => panic!("{name}: {other:?}"),
    };
    refused(
        "t:no_tract",
        "generator_params.tracts",
        "no tract definition `plian`",
    );
    refused("t:no_param", "generator_params.tract", "no such parameter");
    refused("t:bad_field", "generator_params.height", "octaves");
    refused(
        "t:bad_legend",
        "generator_params.layout",
        "`b` is not in the legend",
    );
    refused("t:bad_place", "generator_params.place", "degrees clockwise");

    // The linter's build stage says the same (04 §8.1).
    let diagnostics = clash_gen::check_defs(&built.defs);
    let rendered = diagnostics.render();
    assert_eq!(diagnostics.len(), 5, "{rendered}");
    assert!(
        rendered.contains("realm definition `t:no_tract`"),
        "{rendered}"
    );

    // Bound without the generation definitions, the generator says how
    // to get it.
    let unbound = clash_gen::generators(&built.world);
    let error = unbound.of(built.realm("t:good").unwrap()).unwrap_err();
    assert!(error.to_string().contains("generators_with"), "{error}");
}

#[test]
fn a_ground_that_would_crush_itself_in_a_realm_is_refused_for_that_realm() {
    let defs = format!(
        r#"[ {{ "type": "block", "id": "earth", "material": "soil", "height": 3.0, "fill": 1.0,
                "flags": ["opaque", "load_bearing"] }},
             {{ "type": "ground", "id": "deep",
                "steps": [ {{ "strata": [ {{ "depth": 12, "block": "earth" }},
                                          {{ "block": "solid@granite" }} ] }} ] }},
             {{ "type": "ground", "id": "thin",
                "steps": [ {{ "strata": [ {{ "depth": 3, "block": "earth" }},
                                          {{ "depth": 2, "block": "solid@granite" }},
                                          {{ "depth": 3, "block": "earth" }},
                                          {{ "block": "solid@granite" }} ] }} ] }},
             {{ "type": "tract", "id": "deep", "ground": "deep" }},
             {{ "type": "tract", "id": "thin", "ground": "thin" }},
             {}, {} ]"#,
        realm("deep", r#""tracts": [ { "tract": "deep" } ]"#),
        realm("thin", r#""tracts": [ { "tract": "thin" } ]"#),
    );
    let built = common::with_core(&defs);
    // Twelve storeys of loose soil press more on the lowest than soil
    // bears; two runs of three, with rock between, do not.
    match StandardGenerator::of_realm(&built.generation, "t:deep") {
        Err(GenError::BadParam { message, .. }) => {
            assert!(message.contains("crushed by its own weight"), "{message}");
        }
        other => panic!("{other:?}"),
    }
    assert!(StandardGenerator::of_realm(&built.generation, "t:thin").is_ok());
    let mut live = common::Live::new(built, "t:thin", 1);
    live.load(&[TractPos::new(0, 0)]);
    assert!(live.unsupported(TractPos::new(0, 0)).is_empty());
}

#[test]
fn a_tract_definition_that_cannot_be_realised_is_refused_where_it_is_written() {
    let bad = |tract: &str| -> String {
        let text = format!(
            r#"[ {{ "type": "ground", "id": "g", "params": {{ "wet": 0.5, "name": "fen" }},
                    "steps": [ {{ "strata": [ {{ "block": "solid@granite" }} ] }} ] }},
                 {{ "type": "map", "id": "fixed", "size": [2, 1], "turn": false,
                    "palette": [ {{ "s": {{ "floor": "slab" }} }} ], "levels": {{ "0": [ "ss" ] }} }},
                 {{ "type": "tract", "id": "bad", "ground": "g", {tract} }} ]"#
        );
        match common::try_with_core(&text) {
            Ok(_) => panic!("the tract builds, and was meant not to"),
            Err(error) => {
                assert!(error.contains("gen.json:"), "no place is given:\n{error}");
                error
            }
        }
    };
    let error = bad(r#""place": [ { "map": "house", "at": [28, 3] } ]"#);
    assert!(error.contains("does not fit inside the tract"), "{error}");
    let error = bad(r#""place": [ { "map": "house", "at": [40, 3] } ]"#);
    assert!(error.contains("a cell of the tract"), "{error}");
    let error = bad(r#""place": [ { "map": "lane_strip", "at": [0, 3], "facing": "north" } ]"#);
    assert!(error.contains("names no `front`"), "{error}");
    let error = bad(r#""place": [ { "map": "fixed", "at": [0, 3], "turn": 90 } ]"#);
    assert!(error.contains("may not be turned"), "{error}");
    let error = bad(r#""place": [ { "map": { "tag": "castle" }, "at": [0, 3] } ]"#);
    assert!(error.contains("no map carries this tag"), "{error}");
    let error =
        bad(r#""place": [ { "map": "hut", "at": [3, 3], "params": { "wall": "cheese" } } ]"#);
    assert!(error.contains("cheese"), "{error}");
    let error = bad(r#""place": [ { "map": "hut", "at": [3, 3], "chance": 1.5 } ]"#);
    assert!(error.contains("a chance is from 0 to 1"), "{error}");
    let error = bad(r#""params": { "dry": 1 }"#);
    assert!(
        error.contains("has no parameter `dry`; it has: wet, name"),
        "{error}"
    );
    let error = bad(r#""params": { "wet": "'very'" }"#);
    assert!(error.contains("is a real"), "{error}");
    let error = bad(r#""ruin": "scorched""#);
    assert!(error.contains("`ruin` is an object"), "{error}");
    let error = bad(r#""ground": "g2""#);
    assert!(error.contains("g2"), "{error}");

    // What is right loads: a whole number for a real, a text for a
    // text, an expression, and a ruin carried as written.
    let good = common::with_core(
        r#"[ { "type": "ground", "id": "g", "params": { "wet": 0.5, "name": "fen" },
               "steps": [ { "strata": [ { "block": "solid@granite" } ] } ] },
             { "type": "tract", "id": "ok", "ground": "g", "tags": ["wet"],
               "params": { "wet": "1 / 4", "name": "'bog'" },
               "ruin": { "transform": "scorched" }, "bands": ["overland", "air"],
               "glyph": { "ch": "~", "fg": "blue" } } ]"#,
    );
    let kind = good
        .generation
        .tract(good.generation.lookup_tract("t:ok").unwrap())
        .unwrap();
    assert_eq!(kind.tags(), ["wet"]);
    assert_eq!(kind.bands(), ["overland", "air"]);
    assert_eq!(
        kind.glyph().and_then(|glyph| glyph.ch.as_deref()),
        Some("~")
    );
    assert!(kind.ruin().is_some());
    // Dense id 0 is the tract that has no definition.
    assert_eq!(
        good.generation.lookup_tract("engine:none"),
        Some(clash_world::TractDefId::new(0))
    );
}
