//! Tests for WG3: ground (10 §5).

mod common;

use std::collections::BTreeSet;

use clash_core::Hash128;
use clash_gen::ascii::surface_storey;
use clash_world::dims::TRACT_XY;
use clash_world::{Cause, CellPos, Column, EngineFlag, Layer, Location, TractPos};

/// How deep below the surface the tests look for what a ground made.
const LOOK_DEPTH: i32 = 24;

fn realm(id: &str, params: &str) -> String {
    // The sea is the realm's own, not its generator's.
    let (sea, params) = match params.strip_prefix("SEA ") {
        Some(rest) => (r#""sea_level": 0,"#, rest),
        None => ("", params),
    };
    format!(
        r#"{{ "type": "realm", "id": "{id}", "size_regions": [1, 1], "wrap": [],
             "z_range": [-64, 48], "generator": "standard", {sea}
             "generator_params": {{ {params} }},
             "fill": {{ "below": "solid@granite", "above": null }},
             "strata": [ {{ "from_z": -64, "gravity": 9.81, "atmosphere": "air" }} ],
             "time_rate": 1.0 }}"#
    )
}

/// The world these tests stand on. `earth` is ground that anchors in
/// one state and not in the other; `water` is a still liquid.
fn defs() -> String {
    let hills = r#""height": { "noise": { "octaves": 2, "scale": 0.5 }, "range": [0, 7] }"#;
    [
        r#"{ "type": "block", "id": "earth", "material": "soil", "height": 3.0, "fill": 1.0,
             "flags": ["opaque", "load_bearing", "anchor"],
             "states": { "firm": {}, "loose": { "remove_flags": ["anchor"] } },
             "glyph": { "ch": ":" } }"#
            .to_owned(),
        r#"{ "type": "material", "id": "brine", "density": 1000, "toughness": 1.0 }"#.to_owned(),
        r#"{ "type": "block", "id": "water", "material": "brine", "height": 3.0, "fill": 1.0,
             "flags": ["liquid", "passable"], "glyph": { "ch": "~" } }"#
            .to_owned(),
        r#"{ "type": "map", "id": "shrub", "size": [3, 3], "sit": "none",
             "palette": [ { "b": { "fixture": "bush", "keep": ["floor", "block"] } } ],
             "levels": { "0": [ " b ", "bbb", " b " ] }, "anchor": [1, 1, 0] }"#
            .to_owned(),
        r#"{ "type": "ground", "id": "layered",
             "steps": [ { "strata": [ { "depth": 4, "block": "earth@soil", "anchor": false },
                                      { "block": "solid@granite", "anchor": true } ] },
                        { "surface": { "floor": "grass" } } ] }"#
            .to_owned(),
        r#"{ "type": "ground", "id": "riddled", "depth": 20,
             "steps": [ { "strata": [ { "depth": 3, "block": "earth", "anchor": false },
                                      { "depth": 6, "block": "solid@granite" },
                                      { "depth": 3, "block": "earth#firm", "anchor": false },
                                      { "block": "solid@granite" } ] },
                        { "surface": { "floor": "grass", "ramp": "ramp@soil" } },
                        { "veins": { "block": "solid@steel", "density": 0.04, "size": [3, 9] } },
                        { "pockets": { "block": "earth#loose", "below": -5, "density": 0.04,
                                       "size": [2, 5] } },
                        { "caves": { "below": -1, "density": 0.25, "width": [1, 4] } },
                        { "pockets": { "block": "earth#loose", "below": -2, "density": 0.04,
                                       "size": [2, 4] } } ] }"#
            .to_owned(),
        r#"{ "type": "ground", "id": "seabed",
             "steps": [ { "strata": [ { "block": "solid@granite" } ] },
                        { "surface": { "floor": "grass", "where": "land" } },
                        { "liquid": { "to": "sea_level", "block": "water" } } ] }"#
            .to_owned(),
        r#"{ "type": "ground", "id": "strewn", "params": { "rocks": 0.03 },
             "steps": [ { "strata": [ { "block": "solid@granite" } ] },
                        { "surface": { "floor": "grass" } },
                        { "scatter": { "fixture": "boulder", "density": "$rocks", "apart": 2 } },
                        { "scatter": { "fixture": "bush", "density": 0.1 } } ] }"#
            .to_owned(),
        r#"{ "type": "ground", "id": "shrubby",
             "steps": [ { "strata": [ { "block": "solid@granite" } ] },
                        { "surface": { "floor": "grass" } },
                        { "scatter": { "map": "shrub", "density": 0.02, "apart": 3 } } ] }"#
            .to_owned(),
        r#"{ "type": "tract", "id": "layered", "ground": "layered" }"#.to_owned(),
        r#"{ "type": "tract", "id": "riddled", "ground": "riddled" }"#.to_owned(),
        r#"{ "type": "tract", "id": "seabed", "ground": "seabed" }"#.to_owned(),
        r#"{ "type": "tract", "id": "strewn", "ground": "strewn" }"#.to_owned(),
        r#"{ "type": "tract", "id": "bare_rock", "ground": "strewn", "params": { "rocks": 0 } }"#
            .to_owned(),
        r#"{ "type": "tract", "id": "shrubby", "ground": "shrubby" }"#.to_owned(),
        realm("layered", r#""tracts": [ { "tract": "layered" } ]"#),
        realm("riddled_flat", r#""tracts": [ { "tract": "riddled" } ]"#),
        realm(
            "riddled",
            &format!(r#""tracts": [ {{ "tract": "riddled" }} ], {hills}"#),
        ),
        realm(
            "sea",
            r#"SEA "tracts": [ { "tract": "seabed" } ],
               "height": { "noise": { "octaves": 2, "scale": 2 }, "range": [-40, -30] }"#,
        ),
        realm(
            "shore",
            r#"SEA "tracts": [ { "tract": "seabed" } ],
               "height": { "noise": { "octaves": 2, "scale": 0.5 }, "range": [-4, 4] }"#,
        ),
        realm("strewn", r#""tracts": [ { "tract": "strewn" } ]"#),
        realm("bare_rock", r#""tracts": [ { "tract": "bare_rock" } ]"#),
        realm("shrubby", r#""tracts": [ { "tract": "shrubby" } ]"#),
    ]
    .join(",\n")
}

fn built() -> common::Built {
    common::with_core(&format!("[{}]", defs()))
}

/// The cell columns of a tract, in world coordinates.
fn cells_of(tract: TractPos) -> impl Iterator<Item = (i32, i32)> {
    let origin = tract.origin(0);
    (0..TRACT_XY).flat_map(move |y| (0..TRACT_XY).map(move |x| (origin.x + x, origin.y + y)))
}

#[test]
fn a_tunnel_through_anchoring_rock_stands_and_one_through_loose_ground_comes_down() {
    let mut live = common::Live::new(built(), "t:layered", 1);
    let tract = TractPos::new(0, 0);
    live.load(&[tract]);
    // The stratum that asked for a state that does not anchor got it.
    let loose = live
        .built
        .world
        .block_state("earth@soil#loose", None)
        .unwrap();
    assert_eq!(live.cell(10, 10, -1).block, loose);
    assert_eq!(live.cell(10, 10, -4).block, loose);
    let granite = live.built.world.block_state("solid@granite", None).unwrap();
    assert_eq!(live.cell(10, 10, -5).block, granite);
    assert!(live.unsupported(tract).is_empty(), "as generated it stands");

    let grid = live.grid;
    let dig = |live: &mut common::Live, y: i32, z: i32| -> Vec<CellPos> {
        let cells: Vec<CellPos> = (8..16).map(|x| CellPos::new(x, y, z)).collect();
        for cell in &cells {
            live.ctx()
                .edit(Cause::DEBUG)
                .set_block(Location::new(grid, *cell), clash_world::BlockState::NONE)
                .unwrap();
        }
        cells
    };

    // Through the rock, eight storeys down: nothing above needs it.
    let tunnel = dig(&mut live, 20, -8);
    let rock = live.cosmos.assess_stability(grid, &tunnel).unwrap();
    assert!(rock.is_stable(), "{:?}", rock.falling());

    // Through the loose ground: what lay on the tunnel has a span of
    // nothing, and comes down (02 §14).
    let tunnel = dig(&mut live, 10, -3);
    let soil = live.cosmos.assess_stability(grid, &tunnel).unwrap();
    assert!(!soil.is_stable());
    assert!(
        soil.falling()
            .iter()
            .any(|(cell, layer)| *cell == CellPos::new(10, 10, -2) && *layer == Layer::Block),
        "{:?}",
        soil.falling()
    );
    // The edits asked for the check themselves, as any edit does.
    assert!(!live.cosmos.stability_requests().is_empty());
}

#[test]
fn over_many_seeds_no_cave_lies_in_ground_that_does_not_anchor() {
    let built = built();
    let ground = built
        .generation
        .ground(built.generation.lookup_ground("t:riddled").unwrap())
        .unwrap();
    assert_eq!(
        ground.strata().iter().map(|s| s.anchor).collect::<Vec<_>>(),
        [false, true, false, true]
    );
    let steel = built.world.block_state("solid@steel", None).unwrap();
    let loose = built.world.block_state("earth#loose", None).unwrap();
    let anchors = |block: clash_world::BlockState| {
        built.world.block_info(block).flags.has(EngineFlag::Anchor)
    };
    let tracts = [
        TractPos::new(0, 0),
        TractPos::new(1, 0),
        TractPos::new(5, 9),
    ];
    let (mut carved, mut veined, mut pocketed) = (0u32, 0u32, 0u32);
    for seed in 0..10 {
        let realised = common::realise(&built, "t:riddled_flat", seed, &tracts).unwrap();
        for (tract, made) in tracts.iter().zip(&realised) {
            let column = &made.column;
            for (x, y) in cells_of(*tract) {
                for z in -LOOK_DEPTH..0 {
                    let block = column.cell(CellPos::new(x, y, z)).block;
                    let stratum = ground.stratum_at(-1 - z).unwrap();
                    if block.is_none() {
                        carved += 1;
                        assert!(
                            stratum.anchor,
                            "seed {seed}: a cave at {x}, {y}, {z} in ground that does not anchor"
                        );
                        // Nothing loose hangs over a void.
                        let over = column.cell(CellPos::new(x, y, z + 1));
                        assert!(over.block.is_none() || anchors(over.block), "{x}, {y}, {z}");
                        assert!(over.floor.is_none());
                    }
                    veined += u32::from(block == steel);
                    // A pocket is loose ground where a stratum that
                    // anchors was laid.
                    pocketed += u32::from(block == loose && stratum.anchor);
                }
            }
        }
    }
    assert!(carved > 2000, "{carved} cells of cave");
    assert!(veined > 500, "{veined} cells of vein");
    assert!(pocketed > 500, "{pocketed} cells of pocket");
}

#[test]
fn over_many_seeds_nothing_is_unsupported_when_a_tract_is_first_realised() {
    for seed in 0..8 {
        let mut live = common::Live::new(built(), "t:riddled", seed);
        let tracts = [TractPos::new(0, 0), TractPos::new(1, 0)];
        live.load(&tracts);
        for tract in tracts {
            let found = live.unsupported(tract);
            assert!(found.is_empty(), "seed {seed}, {tract:?}: {found:?}");
        }
        // The ground was worked: a cave, somewhere, under the hills.
        let view = live.cosmos.grid(live.grid).unwrap();
        let column = view.column(tracts[0]).unwrap();
        let hollow = cells_of(tracts[0]).any(|(x, y)| {
            let surface = surface_storey(column, CellPos::new(x, y, 0));
            (surface - LOOK_DEPTH..surface - 1)
                .any(|z| column.cell(CellPos::new(x, y, z)).block.is_none())
        });
        assert!(hollow, "seed {seed}");
    }
}

#[test]
fn a_cave_that_crosses_an_edge_is_the_same_from_both_sides() {
    let built = built();
    let tracts = common::block(3);
    let together = common::realise(&built, "t:riddled", 3, &tracts).unwrap();
    for (tract, made) in tracts.iter().zip(&together) {
        let alone = common::realise(&built, "t:riddled", 3, &[*tract]).unwrap();
        assert_eq!(Hash128::of(&alone[0].column), Hash128::of(&made.column));
    }
    // A void at the eastern edge of one tract goes on into the next:
    // caves are not cut off where tracts meet.
    let west = &together[0].column;
    let east = &together[1].column;
    let mut crossing = 0;
    for y in 0..TRACT_XY {
        for z in -LOOK_DEPTH..6 {
            let here = west.cell(CellPos::new(TRACT_XY - 1, y, z));
            let there = east.cell(CellPos::new(TRACT_XY, y, z));
            let under = west.cell(CellPos::new(TRACT_XY - 1, y, z - 1));
            if here.block.is_none()
                && there.block.is_none()
                && here.floor.is_none()
                && there.floor.is_none()
                && !under.block.is_none()
                && z < surface_storey(west, CellPos::new(TRACT_XY - 1, y, 0)) - 1
            {
                crossing += 1;
            }
        }
    }
    assert!(crossing > 0, "no cave crosses the edge at this seed");
}

#[test]
fn the_sea_fills_to_sea_level_as_uniform_chunks() {
    let built = built();
    let water = built.world.block_state("water", None).unwrap();
    let tract = TractPos::new(0, 0);
    let realised = common::realise(&built, "t:sea", 2, &[tract]).unwrap();
    let column = &realised[0].column;
    for (x, y) in cells_of(tract) {
        let floor = surface_storey(column, CellPos::new(x, y, 0));
        assert!((-40..=-30).contains(&floor));
        for z in floor..0 {
            assert_eq!(column.cell(CellPos::new(x, y, z)).block, water);
        }
        assert!(
            column.cell(CellPos::new(x, y, 0)).is_empty(),
            "air over the sea"
        );
        // No grass under the sea: the surface step was for land.
        assert!(column.cell(CellPos::new(x, y, floor)).floor.is_none());
    }
    // The chunks between the sea floor and the surface hold water and
    // nothing else, each as one value.
    let mut uniform = 0;
    for (chunk_z, chunk) in column.chunks() {
        let whole = !chunk.any_state(Layer::Block, |state| state != water.raw());
        if whole {
            assert!(
                chunk.uniform_flags().is_some(),
                "chunk {chunk_z} is stored cell by cell"
            );
            uniform += 1;
        }
    }
    assert!(uniform >= 3, "{uniform} chunks of nothing but sea");

    // On a shore the sea fills what is below it and leaves the land.
    let realised = common::realise(&built, "t:shore", 2, &[tract]).unwrap();
    let column = &realised[0].column;
    let grass = built.world.floor_state("grass", None).unwrap();
    let (mut wet, mut dry) = (0, 0);
    for (x, y) in cells_of(tract) {
        let floor = surface_storey(column, CellPos::new(x, y, 0));
        let top = column.cell(CellPos::new(x, y, floor));
        if floor < 0 {
            assert_eq!(top.block, water);
            assert!(top.floor.is_none());
            wet += 1;
        } else {
            assert!(top.block.is_none());
            assert_eq!(top.floor, grass);
            dry += 1;
        }
    }
    assert!(wet > 0 && dry > 0, "{wet} wet, {dry} dry");
}

/// The cells of a block of tracts that hold a fixture.
fn fixtures(
    tracts: &[TractPos],
    columns: &[&Column],
    fixture: clash_world::FixtureState,
) -> BTreeSet<(i32, i32)> {
    let mut found = BTreeSet::new();
    for (tract, column) in tracts.iter().zip(columns) {
        for (x, y) in cells_of(*tract) {
            if column.cell(CellPos::new(x, y, 0)).fixture == fixture {
                found.insert((x, y));
            }
        }
    }
    found
}

#[test]
fn scatter_respects_spacing_and_density_over_many_seeds() {
    let built = built();
    let boulder = built.world.fixture_state("boulder", None).unwrap();
    let bush = built.world.fixture_state("bush", None).unwrap();
    let tracts = common::block(3);
    let seeds = 12;
    let (mut boulders, mut bushes) = (0usize, 0usize);
    for seed in 0..seeds {
        let realised = common::realise(&built, "t:strewn", seed, &tracts).unwrap();
        let columns: Vec<&Column> = realised.iter().map(|tract| &tract.column).collect();
        let rocks = fixtures(&tracts, &columns, boulder);
        // Spacing: two cells must lie between any two, across the
        // edges of tracts as within them.
        for (x, y) in &rocks {
            for dy in -2..=2 {
                for dx in -2..=2 {
                    assert!(
                        (dx, dy) == (0, 0) || !rocks.contains(&(x + dx, y + dy)),
                        "seed {seed}: boulders at {x}, {y} and {}, {}",
                        x + dx,
                        y + dy
                    );
                }
            }
        }
        boulders += rocks.len();
        bushes += fixtures(&tracts, &columns, bush).len();
    }
    let cells = (seeds as usize * tracts.len() * (TRACT_XY * TRACT_XY) as usize) as f64;
    let share = boulders as f64 / cells;
    assert!((0.028..0.032).contains(&share), "boulders: {share}");
    // A bush is not put where a boulder stands, so a little under a
    // tenth of the cells hold one.
    let share = bushes as f64 / cells;
    assert!((0.088..0.102).contains(&share), "bushes: {share}");

    // A tract definition sets the ground's parameter: no rocks at all.
    let realised = common::realise(&built, "t:bare_rock", 1, &tracts).unwrap();
    let columns: Vec<&Column> = realised.iter().map(|tract| &tract.column).collect();
    assert!(fixtures(&tracts, &columns, boulder).is_empty());
    assert!(!fixtures(&tracts, &columns, bush).is_empty());
}

#[test]
fn a_scattered_map_is_whole_where_it_crosses_an_edge() {
    let built = built();
    let bush = built.world.fixture_state("bush", None).unwrap();
    let tracts = common::block(2);
    let realised = common::realise(&built, "t:shrubby", 6, &tracts).unwrap();
    let columns: Vec<&Column> = realised.iter().map(|tract| &tract.column).collect();
    let bushes = fixtures(&tracts, &columns, bush);
    let arms = [(0, -1), (1, 0), (0, 1), (-1, 0)];
    let is_centre = |x: i32, y: i32| {
        bushes.contains(&(x, y))
            && arms
                .iter()
                .all(|(dx, dy)| bushes.contains(&(x + dx, y + dy)))
    };
    let inner = 2..2 * TRACT_XY - 2;
    let (mut centres, mut straddling) = (0, 0);
    for (x, y) in &bushes {
        if !inner.contains(x) || !inner.contains(y) {
            continue;
        }
        // Every bush is the middle of a shrub or an arm of one.
        let whole = is_centre(*x, *y) || arms.iter().any(|(dx, dy)| is_centre(x + dx, y + dy));
        assert!(whole, "a piece of a shrub at {x}, {y}");
        if is_centre(*x, *y) {
            centres += 1;
            let edge = TRACT_XY - 1..=TRACT_XY;
            straddling += u32::from(edge.contains(x) || edge.contains(y));
        }
    }
    assert!(centres > 20, "{centres} shrubs");
    assert!(straddling > 0, "no shrub straddles an edge at this seed");
    // And each tract alone makes the part of each shrub that is its.
    for (tract, made) in tracts.iter().zip(&realised) {
        let alone = common::realise(&built, "t:shrubby", 6, &[*tract]).unwrap();
        assert_eq!(Hash128::of(&alone[0].column), Hash128::of(&made.column));
    }
}

#[test]
fn a_ground_that_cannot_hold_is_refused_where_it_is_written() {
    let bad = |ground: &str| -> String {
        let text = format!(
            r#"[ {{ "type": "block", "id": "earth", "material": "soil", "height": 3.0, "fill": 1.0,
                    "flags": ["opaque", "load_bearing", "anchor"] }},
                 {ground} ]"#
        );
        match common::try_with_core(&text) {
            Ok(_) => panic!("the ground builds, and was meant not to"),
            Err(error) => {
                assert!(error.contains("gen.json:"), "no place is given:\n{error}");
                error
            }
        }
    };
    let ground =
        |steps: &str| format!(r#"{{ "type": "ground", "id": "g", "steps": [ {steps} ] }}"#);

    let error = bad(&ground(r#"{ "strata": [ { "block": "window" } ] }"#));
    assert!(error.contains("is not `load_bearing`"), "{error}");

    let error = bad(&ground(
        r#"{ "strata": [ { "block": "earth", "anchor": false } ] }"#,
    ));
    assert!(
        error.contains("a state of `earth` that does not anchor"),
        "{error}"
    );

    let error = bad(&ground(
        r#"{ "strata": [ { "block": "earth" }, { "depth": 2, "block": "earth" } ] }"#,
    ));
    assert!(
        error.contains("only the last stratum may leave out `depth`"),
        "{error}"
    );

    let error = bad(&ground(
        r#"{ "surface": { "floor": "grass", "ramp": "wall" } }"#,
    ));
    assert!(error.contains("is not flagged `ramp`"), "{error}");

    let error = bad(&ground(
        r#"{ "liquid": { "to": "sea_level", "block": "wall" } }"#,
    ));
    assert!(error.contains("is not flagged `liquid`"), "{error}");

    let error = bad(&ground(
        r#"{ "liquid": { "to": "the brim", "block": "wall" } }"#,
    ));
    assert!(
        error.contains("`to` is `sea_level`, `plan` or a storey"),
        "{error}"
    );

    let error = bad(&ground(
        r#"{ "caves": { "density": 0.1, "width": [3, 1] } }"#,
    ));
    assert!(error.contains("`width` is `[low, high]`"), "{error}");

    let error = bad(&ground(r#"{ "caves": { "density": 1.5 } }"#));
    assert!(error.contains("a density is a share"), "{error}");

    let error = bad(&ground(r#"{ "caves": { "density": 0.1, "below": 2 } }"#));
    assert!(
        error.contains("counts storeys from the surface down"),
        "{error}"
    );

    let error = bad(&ground(r#"{ "scatter": { "density": 0.1 } }"#));
    assert!(
        error.contains("puts either a `map`, or elements"),
        "{error}"
    );

    let error = bad(&ground(
        r#"{ "scatter": { "fixture": "bush", "density": "$wet" } }"#,
    ));
    assert!(error.contains("wet"), "{error}");

    let error = bad(&ground(
        r#"{ "surface": { "floor": "grass" }, "caves": { "density": 0.1 } }"#,
    ));
    assert!(
        error.contains("a step is an object with one key"),
        "{error}"
    );

    let error = bad(&ground(
        r#"{ "surface": { "floor": "grass", "where": "hills" } }"#,
    ));
    assert!(error.contains("`where` is `land` or `sea`"), "{error}");

    let error = bad(&ground(r#"{ "float": { "block": "earth" } }"#));
    assert!(
        error.contains("the `float` step is not built yet"),
        "{error}"
    );

    let error = bad(&ground(r#"{ "dunes": { } }"#));
    assert!(error.contains("dunes"), "{error}");
}
