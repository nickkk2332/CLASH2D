//! Tests for WG7 and WG8: region kinds and surveys (10 §8.1 to §8.3),
//! settlements (10 §8.4), and the people a town's buildings ask for,
//! with the town bundled with the `core` mod.
//!
//! The pictures of a block of the bundled town at a fixed seed are kept
//! as text in `tests/golden/town.txt`. Run the test with `BLESS=1` to
//! rewrite them, look at the difference, and say why in the change's
//! description.

mod common;

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;
use std::sync::Arc;

use clash_gen::place::{PlacedMap, gen_map_id};
use clash_gen::settlement::{LotFill, SettlementPlan};
use clash_gen::{GenCtx, RegionSurveyor, StandardGenerator};
use clash_world::dims::{REGION_CELLS, TRACT_XY};
use clash_world::{
    CellPos, GridId, Location, MoveProfile, PathLimits, RealmId, RealmInfo, RegionPos,
    RegionRecord, TractPos,
};

/// The realm definition of the bundled town.
const TOWN_REALM: &str = "core:town";

/// The seed the stored pictures are made with.
const GOLDEN_SEED: u64 = 1;

/// The environment variable that rewrites the stored pictures.
const BLESS_VAR: &str = "BLESS";

/// The north-west tract of the block of the town that is pictured: it
/// takes in the edge between the realm's two regions.
const GOLDEN_FIRST: TractPos = TractPos::new(63, 31);

/// How many tracts wide and deep the pictured block is.
const GOLDEN_SIDE: i32 = 2;

fn bundled() -> common::Built {
    common::bundled().unwrap_or_else(|error| panic!("mods/core does not build:\n{error}"))
}

/// The facts of a realm of a definition with a seed, as loading would
/// give them.
fn realm_info(built: &common::Built, realm: &str, seed: u64) -> RealmInfo {
    let def = built.realm(realm).unwrap();
    let kind = built.world.realms().get(def).unwrap();
    RealmInfo {
        id: RealmId::new(0),
        def,
        seed,
        base_grid: GridId::new(0),
        topology: kind.topology,
        below_fill: kind.below_fill,
        above_fill: kind.above_fill,
    }
}

/// A fresh generator of a realm definition: nothing worked out yet.
fn generator(built: &common::Built, realm: &str) -> StandardGenerator {
    StandardGenerator::of_realm(&built.generation, realm).unwrap()
}

fn survey(
    built: &common::Built,
    generator: &StandardGenerator,
    info: &RealmInfo,
    region: RegionPos,
) -> clash_world::RegionSurveyResult {
    generator.survey(&GenCtx::new(&built.world), info, region)
}

/// The test realm of two regions by one, of a region kind and with
/// settlements given as the text of generator parameters.
fn test_realm(params: &str) -> String {
    format!(
        r#"{{ "type": "realm", "id": "land", "size_regions": [2, 1], "wrap": [],
             "z_range": [-16, 48], "generator": "standard",
             "generator_params": {{ {params} }},
             "fill": {{ "below": "solid@granite", "above": null }},
             "strata": [ {{ "from_z": -16, "gravity": 9.81, "atmosphere": "air" }} ],
             "time_rate": 1.0 }}"#
    )
}

/// The rows at which lines of a record run east or west across a
/// column next to `edge`: each line with consecutive points at `inner`
/// and `edge` in one row, by kind and row.
fn rows_across(record: &RegionRecord, inner: i32, edge: i32) -> BTreeSet<(u16, i32)> {
    record
        .paths
        .iter()
        .flat_map(|line| {
            line.points.windows(2).filter_map(move |pair| {
                let (a, b) = (pair[0], pair[1]);
                let across =
                    a.y == b.y && ((a.x == inner && b.x == edge) || (a.x == edge && b.x == inner));
                across.then_some((line.kind.raw(), a.y))
            })
        })
        .collect()
}

// --- WG7: region kinds and surveys ---------------------------------------

#[test]
fn edges_agree_whichever_region_is_surveyed_first() {
    let built = bundled();
    let info = realm_info(&built, TOWN_REALM, 5);
    let (west, east) = (RegionPos::new(0, 0), RegionPos::new(1, 0));

    let first = generator(&built, TOWN_REALM);
    let west_first = survey(&built, &first, &info, west);
    let east_after = survey(&built, &first, &info, east);
    let second = generator(&built, TOWN_REALM);
    let east_first = survey(&built, &second, &info, east);
    let west_after = survey(&built, &second, &info, west);
    assert_eq!(
        west_first, west_after,
        "the west region depends on the order"
    );
    assert_eq!(
        east_first, east_after,
        "the east region depends on the order"
    );

    // Every street of the town that reaches the edge on one side goes
    // on across it on the other, at the same row. (A path between the
    // regions may run along the edge on one side; it is checked below.)
    let edge = REGION_CELLS;
    let streets: BTreeSet<u16> = first
        .settlement_plan(info.seed, 0)
        .unwrap()
        .streets
        .iter()
        .map(|seg| seg.kind.raw())
        .collect();
    let street_rows = |record: &RegionRecord, inner: i32, edge: i32| -> BTreeSet<(u16, i32)> {
        rows_across(record, inner, edge)
            .into_iter()
            .filter(|(kind, _)| streets.contains(kind))
            .collect()
    };
    let west_side = street_rows(&west_first.record, edge - 2, edge - 1);
    let east_side = street_rows(&east_first.record, edge + 1, edge);
    assert!(!west_side.is_empty(), "no street crosses the edge");
    assert_eq!(west_side, east_side);

    // Every path between the regions crosses where both sides say: the
    // west region's runs from its last column, the east region's from
    // its first, in the same rows and of the same kind.
    let ends = |region: RegionPos, column: i32| -> BTreeSet<(u16, i32)> {
        first
            .region_plan(&info, region)
            .paths
            .iter()
            .filter(|seg| seg.minor && seg.from.0 == column)
            .map(|seg| (seg.kind.raw(), seg.from.1))
            .collect()
    };
    let west_ends = ends(west, edge - 1);
    assert!(!west_ends.is_empty(), "no path crosses the edge");
    assert_eq!(west_ends, ends(east, edge));
    // And the cells laid meet: realised on both sides, the floor at a
    // crossing is the path's on each.
    let tracts: Vec<TractPos> = west_ends
        .iter()
        .flat_map(|(_, y)| {
            [
                CellPos::new(edge - 1, *y, 0).tract(),
                CellPos::new(edge, *y, 0).tract(),
            ]
        })
        .collect();
    let realised = common::realise(&built, TOWN_REALM, info.seed, &tracts).unwrap();
    for (pair, (_, y)) in realised.chunks(2).zip(&west_ends) {
        let west_floor = pair[0].column.cell(CellPos::new(edge - 1, *y, 0)).floor;
        let east_floor = pair[1].column.cell(CellPos::new(edge, *y, 0)).floor;
        assert!(!west_floor.is_none() && west_floor == east_floor, "row {y}");
    }

    // A lot or a site across the edge is listed alike on both sides.
    let across = |record: &RegionRecord| -> Vec<clash_world::Placement> {
        record
            .placements
            .iter()
            .filter(|placement| {
                let bounds = clash_gen::placement_box(built.generation.maps(), placement).unwrap();
                bounds.min.x < edge && bounds.max.x >= edge
            })
            .cloned()
            .collect()
    };
    assert_eq!(across(&west_first.record), across(&east_first.record));
}

#[test]
fn a_survey_is_pure() {
    let built = bundled();
    for seed in [0, 3, 11] {
        let info = realm_info(&built, TOWN_REALM, seed);
        let region = RegionPos::new(1, 0);
        let fresh = survey(&built, &generator(&built, TOWN_REALM), &info, region);
        // A generator that has worked out other things first.
        let busy = generator(&built, TOWN_REALM);
        let _ = survey(&built, &busy, &info, RegionPos::new(0, 0));
        let _ = busy.settlement_plans(seed + 1);
        assert_eq!(survey(&built, &busy, &info, region), fresh, "seed {seed}");
    }
}

#[test]
fn site_counts_and_spacing_hold_over_many_seeds() {
    let built = common::with_core(&format!(
        r#"[
          {{ "type": "region_kind", "id": "farms",
             "tracts": [ {{ "tract": "core:pasture" }} ],
             "sites": [ {{ "site": "core:farmstead", "per_region": [3, 6], "apart": 5 }},
                        {{ "site": "core:farmstead", "per_region": 2, "apart": 3, "near": "road" }} ],
             "paths": {{ "path": "core:road", "per_edge": [1, 2] }} }},
          {}
        ]"#,
        test_realm(r#""regions": [ { "kind": "farms" } ]"#)
    ));
    for seed in 0..12 {
        let info = realm_info(&built, "t:land", seed);
        let generator = generator(&built, "t:land");
        for region in [RegionPos::new(0, 0), RegionPos::new(1, 0)] {
            let plan = generator.region_plan(&info, region);
            assert!(
                plan.shortfalls.is_empty(),
                "seed {seed}: {:?}",
                plan.shortfalls
            );
            let count = plan.sites.len();
            assert!((5..=8).contains(&count), "seed {seed}: {count} sites");
            // One site to a tract, every one inside the region.
            let mut tracts = BTreeSet::new();
            for site in &plan.sites {
                for tract in &site.tracts {
                    assert_eq!(tract.region(), region);
                    assert!(
                        tracts.insert((tract.x, tract.y)),
                        "two sites share {tract:?}"
                    );
                }
            }
            // Sites of one rule keep apart; those near a road are
            // within a tract of a path and not on it.
            let firsts: Vec<TractPos> = plan.sites.iter().map(|site| site.tracts[0]).collect();
            let (spaced, by_road) = firsts.split_at(count - 2);
            for (i, a) in spaced.iter().enumerate() {
                for b in &spaced[i + 1..] {
                    assert!((a.x - b.x).abs().max((a.y - b.y).abs()) > 5, "seed {seed}");
                }
            }
            for tract in by_road {
                // Within a tract of a path: some cell of a path's
                // footprint lies within a tract of the site's tract.
                let origin = tract.origin(0);
                let near = plan.paths.iter().any(|seg| {
                    let fp = built.generation.path_kind(seg.kind).unwrap().footprint();
                    let (x0, y0, x1, y1) = seg.reach(fp);
                    x1 >= origin.x - TRACT_XY
                        && x0 < origin.x + 2 * TRACT_XY
                        && y1 >= origin.y - TRACT_XY
                        && y0 < origin.y + 2 * TRACT_XY
                });
                assert!(near, "seed {seed}: a site by the road is far from it");
            }
            // The survey makes them sites of the world.
            let result = survey(&built, &generator, &info, region);
            assert_eq!(result.sites.len(), count);
            assert!(result.record.survey.kind.is_some());
        }
    }
}

#[test]
fn a_region_kind_gives_tracts_in_patches() {
    let built = bundled();
    let info = realm_info(&built, TOWN_REALM, 2);
    let generator = generator(&built, TOWN_REALM);
    let record = survey(&built, &generator, &info, RegionPos::new(1, 0)).record;
    let pasture = built.generation.lookup_tract("core:pasture").unwrap();
    let copse = built.generation.lookup_tract("core:copse").unwrap();
    let town_lot = built.generation.lookup_tract("core:town_lot").unwrap();
    let mut seen = BTreeMap::new();
    for record in record.tracts.iter() {
        *seen.entry(record.def).or_insert(0) += 1;
    }
    assert!(seen.contains_key(&pasture) && seen.contains_key(&copse));
    // The tracts of the village, and of the half of the town in this
    // region, are the settlements' own definition's.
    assert_eq!(seen.get(&town_lot), Some(&(16 + 32)));
    // Patches of four tracts a side share their definition.
    let region = RegionPos::new(1, 0);
    for index in 0..clash_world::dims::REGION_TRACT_COUNT {
        let tract = region.tract(index);
        let corner = TractPos::new(
            tract.x - tract.x.rem_euclid(4),
            tract.y - tract.y.rem_euclid(4),
        );
        let def = generator.tract_def(info.seed, tract);
        if def != town_lot && generator.tract_def(info.seed, corner) != town_lot {
            assert_eq!(def, generator.tract_def(info.seed, corner), "{tract:?}");
        }
    }
}

// --- WG8: settlements ----------------------------------------------------

/// Whether a cell lies on a street of a plan: within the footprint of
/// one of its street lines.
fn on_street(built: &common::Built, plan: &SettlementPlan, x: i32, y: i32) -> bool {
    plan.streets.iter().any(|seg| {
        let footprint = built.generation.path_kind(seg.kind).unwrap().footprint();
        seg.near(footprint, x, y, 0)
    })
}

/// Checks a plan: every lot inside the extent, none overlapping another
/// or a street, every one fronting a street, and every street joined to
/// every other.
fn check_plan(built: &common::Built, plan: &SettlementPlan, what: &str) {
    assert!(!plan.lots.is_empty(), "{what}: no lots");
    for (i, lot) in plan.lots.iter().enumerate() {
        let b = lot.bounds;
        assert!(
            plan.contains(b.min.x, b.min.y) && plan.contains(b.max.x, b.max.y),
            "{what}"
        );
        for other in &plan.lots[i + 1..] {
            assert!(
                !clash_gen::boxes_overlap(b, other.bounds),
                "{what}: two lots overlap"
            );
        }
        // The map fits its lot: nothing of it lies on a street.
        for (x, y) in [
            (b.min.x, b.min.y),
            (b.max.x, b.max.y),
            (b.min.x, b.max.y),
            (b.max.x, b.min.y),
        ] {
            assert!(
                !on_street(built, plan, x, y),
                "{what}: a lot lies on a street"
            );
        }
        // It fronts a street: the row beyond its front edge is street.
        let compiled = built
            .generation
            .maps()
            .get(gen_map_id(lot.placement.map))
            .unwrap();
        let front = compiled.front().unwrap().rotated(lot.placement.turn.rot);
        assert_eq!(front, lot.facing, "{what}: a map does not face its street");
        let beyond = match front {
            clash_core::Dir8::North => b.min.y - 1,
            _ => b.max.y + 1,
        };
        for x in b.min.x..=b.max.x {
            assert!(
                on_street(built, plan, x, beyond),
                "{what}: a lot does not front a street"
            );
        }
    }
    // Every street cell joins every other.
    let ((x0, y0), (x1, y1)) = plan.extent;
    let mut street: BTreeSet<(i32, i32)> = BTreeSet::new();
    for y in y0..=y1 {
        for x in x0..=x1 {
            if on_street(built, plan, x, y) {
                street.insert((x, y));
            }
        }
    }
    let start = *street.iter().next().unwrap();
    let mut seen = BTreeSet::from([start]);
    let mut todo = vec![start];
    while let Some((x, y)) = todo.pop() {
        for next in [(x + 1, y), (x - 1, y), (x, y + 1), (x, y - 1)] {
            if street.contains(&next) && seen.insert(next) {
                todo.push(next);
            }
        }
    }
    assert_eq!(
        seen.len(),
        street.len(),
        "{what}: the streets are not all joined"
    );
}

#[test]
fn every_lot_fits_and_fronts_a_street_over_many_seeds() {
    let built = bundled();
    for seed in 0..6 {
        let generator = generator(&built, TOWN_REALM);
        let plans = generator.settlement_plans(seed);
        assert_eq!(plans.len(), 2);
        for plan in &plans {
            check_plan(&built, plan, &format!("seed {seed}"));
        }
    }
}

#[test]
fn every_must_have_site_is_present_and_made_a_site_of_the_world() {
    let built = bundled();
    let square = built
        .generation
        .resolve_site("town_square", Some("core"))
        .unwrap();
    let hall = built
        .generation
        .resolve_site("town_hall", Some("core"))
        .unwrap();
    // The trades every market town has, after the square and the hall
    // (`gen/trades.json`).
    let trades: Vec<_> = [
        "market",
        "bakery",
        "tavern",
        "guardhouse",
        "workshop",
        "healer",
    ]
    .into_iter()
    .map(|name| built.generation.resolve_site(name, Some("core")).unwrap())
    .collect();
    let mut wanted = vec![square, hall];
    wanted.extend(trades);
    for seed in 0..8 {
        let generator = generator(&built, TOWN_REALM);
        let plan = generator.settlement_plan(seed, 0).unwrap();
        assert!(plan.missing.is_empty(), "seed {seed}: {:?}", plan.missing);
        let defs: Vec<_> = plan.sites.iter().map(|site| site.def).collect();
        assert_eq!(defs, wanted, "seed {seed}");
        // A tract holds one site at most.
        let tracts: Vec<_> = plan
            .sites
            .iter()
            .flat_map(|site| site.tracts.clone())
            .collect();
        let unique: BTreeSet<_> = tracts.iter().map(|tract| (tract.x, tract.y)).collect();
        assert_eq!(tracts.len(), unique.len());
        // The square is near the centre.
        let centre = plan.centre;
        let anchor = plan.sites[0].anchor;
        assert!(
            (anchor.x - centre.0).abs() < 4 * TRACT_XY
                && (anchor.y - centre.1).abs() < 4 * TRACT_XY
        );
        // The regions they lie in make them.
        let info = realm_info(&built, TOWN_REALM, seed);
        let mut made = Vec::new();
        for region in [RegionPos::new(0, 0), RegionPos::new(1, 0)] {
            made.extend(
                survey(&built, &generator, &info, region)
                    .sites
                    .into_iter()
                    .map(|site| site.def),
            );
            assert!(generator.survey_report(&info, region).missing.is_empty());
        }
        assert!(
            made.contains(&square) && made.contains(&hall),
            "seed {seed}"
        );
    }
}

#[test]
fn the_organic_pattern_staggers_its_streets_and_still_joins_them() {
    let built = common::with_core(&format!(
        r#"[
          {{ "type": "settlement", "id": "hamlet", "size": [6, 5],
             "streets": {{ "pattern": "organic", "spacing": [18, 30], "path": "core:street",
                           "main": 2, "main_path": "core:main_street" }},
             "zones": [ {{ "zone": "all", "to": 1.0, "lots": ["house", "shop"] }} ],
             "must_have": [ "core:town_hall" ] }},
          {}
        ]"#,
        test_realm(r#""settlements": [ { "settlement": "hamlet", "at": [61, 10] } ]"#)
    ));
    let mut widths = BTreeSet::new();
    for seed in 0..6 {
        let generator = generator(&built, "t:land");
        let plan = generator.settlement_plan(seed, 0).unwrap();
        check_plan(&built, &plan, &format!("organic, seed {seed}"));
        assert!(plan.missing.is_empty());
        for block in &plan.blocks {
            widths.insert(block.max.0 - block.min.0);
        }
        // Streets north to south do not all line up from row to row.
        let mut columns: BTreeMap<(i32, i32), BTreeSet<i32>> = BTreeMap::new();
        for seg in plan.streets.iter().filter(|seg| !seg.runs_east()) {
            columns
                .entry((seg.from.1, seg.to.1))
                .or_default()
                .insert(seg.from.0);
        }
        let rows: Vec<&BTreeSet<i32>> = columns.values().collect();
        assert!(
            rows.windows(2).any(|pair| pair[0] != pair[1]),
            "seed {seed}: a plain grid"
        );
    }
    assert!(widths.len() > 4, "the blocks are all alike: {widths:?}");
}

#[test]
fn a_settlement_across_two_regions_has_continuous_streets() {
    let built = bundled();
    let generator = generator(&built, TOWN_REALM);
    let plan = generator.settlement_plan(GOLDEN_SEED, 0).unwrap();
    let ((x0, _), (x1, _)) = plan.extent;
    assert!(
        x0 < REGION_CELLS && x1 >= REGION_CELLS,
        "the town does not span the edge"
    );
    // Realised on both sides of the edge, the floors meet: a street on
    // one side of the edge is a street on the other.
    let tracts: Vec<TractPos> = (28..36)
        .flat_map(|y| [TractPos::new(63, y), TractPos::new(64, y)])
        .collect();
    let realised = common::realise(&built, TOWN_REALM, GOLDEN_SEED, &tracts).unwrap();
    let mut street_rows = 0;
    for (pair, tract) in realised.chunks(2).zip(tracts.chunks(2)) {
        let origin = tract[0].origin(0);
        for dy in 0..TRACT_XY {
            let y = origin.y + dy;
            let west = pair[0]
                .column
                .cell(CellPos::new(REGION_CELLS - 1, y, 0))
                .floor;
            let east = pair[1].column.cell(CellPos::new(REGION_CELLS, y, 0)).floor;
            let streets = on_street(&built, &plan, REGION_CELLS - 1, y);
            assert_eq!(streets, on_street(&built, &plan, REGION_CELLS, y));
            if streets {
                street_rows += 1;
                assert_eq!(west, east, "row {y}: the street changes at the edge");
            }
        }
    }
    assert!(
        street_rows >= 9,
        "only {street_rows} rows of street cross the edge"
    );
}

#[test]
fn every_entrance_is_reachable_from_every_other_on_foot() {
    let built = bundled();
    let generator = generator(&built, TOWN_REALM);
    let seed = GOLDEN_SEED;
    let plan = generator.settlement_plan(seed, 0).unwrap();
    let maps = Arc::clone(built.generation.maps());
    let first = TractPos::new(62, 30);
    let tracts = common::tract_block(first, 4, 4);
    let mut live = common::Live::new(built, TOWN_REALM, seed);
    live.load(&tracts);
    // The entrances of every lot wholly inside the loaded block, away
    // from its edge.
    let (lx0, ly0) = (first.origin(0).x + 2, first.origin(0).y + 2);
    let (lx1, ly1) = (lx0 + 4 * TRACT_XY - 5, ly0 + 4 * TRACT_XY - 5);
    let mut entrances = Vec::new();
    for lot in &plan.lots {
        let b = lot.bounds;
        if b.min.x < lx0 || b.min.y < ly0 || b.max.x > lx1 || b.max.y > ly1 {
            continue;
        }
        let placed = PlacedMap::resolve(&maps, &lot.placement).unwrap().unwrap();
        entrances.extend(
            placed
                .marks()
                .get("entrance")
                .into_iter()
                .flatten()
                .copied(),
        );
    }
    assert!(entrances.len() >= 20, "only {} entrances", entrances.len());
    let walker = MoveProfile {
        opens: true,
        ..MoveProfile::walker()
    };
    let limits = PathLimits {
        max_nodes: 400_000,
        ..PathLimits::DEFAULT
    };
    let from = Location::new(live.grid, entrances[0]);
    for to in &entrances[1..] {
        let found = live
            .cosmos
            .find_path(from, Location::new(live.grid, *to), &walker, limits);
        assert!(
            found.is_ok(),
            "no way from {:?} to {to:?}: {found:?}",
            entrances[0]
        );
    }
}

#[test]
fn a_town_has_its_residents_and_staff() {
    let built = bundled();
    let tracts = common::tract_block(TractPos::new(62, 30), 4, 4);
    let realised = common::realise(&built, TOWN_REALM, GOLDEN_SEED, &tracts).unwrap();
    let again = common::realise(&built, TOWN_REALM, GOLDEN_SEED, &tracts).unwrap();
    let spawns: Vec<_> = realised
        .iter()
        .flat_map(|tract| tract.spawns.clone())
        .collect();
    let spawns_again: Vec<_> = again
        .iter()
        .flat_map(|tract| tract.spawns.clone())
        .collect();
    assert_eq!(spawns, spawns_again, "the same town has the same people");
    let people = spawns.iter().filter(|spawn| spawn.what == "person").count();
    assert!(people >= 20, "only {people} people");
    // Each is handed over by the tract its cell is in.
    for (realised, tract) in realised.iter().zip(&tracts) {
        for spawn in &realised.spawns {
            assert_eq!(spawn.at.tract(), *tract);
        }
    }
    // The clerks of the town hall are at its desks.
    let generator = generator(&built, TOWN_REALM);
    let plan = generator.settlement_plan(GOLDEN_SEED, 0).unwrap();
    let hall = plan
        .lots
        .iter()
        .find(|lot| matches!(lot.fill, LotFill::Site { site: 1 }))
        .unwrap();
    let placed = PlacedMap::resolve(built.generation.maps(), &hall.placement)
        .unwrap()
        .unwrap();
    let desks: Vec<CellPos> = placed.marks().get("desk").cloned().unwrap_or_default();
    let all: Vec<_> = common::realise(
        &built,
        TOWN_REALM,
        GOLDEN_SEED,
        &hall.bounds.tracts().collect::<Vec<_>>(),
    )
    .unwrap()
    .into_iter()
    .flat_map(|tract| tract.spawns)
    .collect();
    assert!(
        all.iter().any(|spawn| desks.first() == Some(&spawn.at)),
        "no clerk at a desk"
    );
}

// --- Looking at it -------------------------------------------------------

fn town_pictures(built: &common::Built) -> String {
    let mut out = String::new();
    let tracts = common::tract_block(GOLDEN_FIRST, GOLDEN_SIDE, GOLDEN_SIDE);
    for storey in [0, 1] {
        writeln!(
            out,
            "== {TOWN_REALM}, seed {GOLDEN_SEED}, tracts {}, {} to {}, {}, storey {storey} ==",
            GOLDEN_FIRST.x,
            GOLDEN_FIRST.y,
            GOLDEN_FIRST.x + GOLDEN_SIDE - 1,
            GOLDEN_FIRST.y + GOLDEN_SIDE - 1,
        )
        .unwrap();
        out.push_str(&common::tract_text(built, TOWN_REALM, GOLDEN_SEED, &tracts, storey).unwrap());
        out.push('\n');
    }
    out
}

#[test]
fn the_bundled_town_looks_as_it_did_and_stands() {
    let built = bundled();
    let now = town_pictures(&built);
    let path = common::mods_dir().join("../crates/clash-gen/tests/golden/town.txt");
    if std::env::var_os(BLESS_VAR).is_some_and(|value| value == "1") {
        std::fs::write(&path, &now).unwrap();
    } else {
        let stored = std::fs::read_to_string(&path).unwrap_or_else(|_| {
            panic!(
                "no stored pictures at {}: run the test with {BLESS_VAR}=1 to write them",
                path.display()
            )
        });
        if stored != now {
            let line = stored
                .lines()
                .zip(now.lines())
                .position(|(stored, now)| stored != now)
                .map_or(0, |index| index + 1);
            panic!(
                "the town differs from {} at line {line}: run with {BLESS_VAR}=1 to accept \
                 the change, and say why in the change's description",
                path.display()
            );
        }
    }
    // Every placed building stands: nothing is unsupported or crushed.
    let tracts = common::tract_block(GOLDEN_FIRST, GOLDEN_SIDE, GOLDEN_SIDE);
    let mut live = common::Live::new(built, TOWN_REALM, GOLDEN_SEED);
    live.load(&tracts);
    for tract in tracts {
        let found = live.unsupported(tract);
        assert!(found.is_empty(), "{tract:?} does not stand: {found:?}");
    }
}

#[test]
fn every_building_of_the_town_stands_over_several_seeds() {
    for seed in [2, 9] {
        let built = bundled();
        let generator = generator(&built, TOWN_REALM);
        let plan = generator.settlement_plan(seed, 0).unwrap();
        // The tracts round its centre, where every kind of lot is.
        let (cx, cy) = plan.centre;
        let first = TractPos::new(cx / TRACT_XY - 1, cy / TRACT_XY - 1);
        let tracts = common::tract_block(first, 3, 3);
        let mut live = common::Live::new(built, TOWN_REALM, seed);
        live.load(&tracts);
        for tract in tracts {
            let found = live.unsupported(tract);
            assert!(found.is_empty(), "seed {seed}, {tract:?}: {found:?}");
        }
    }
}

// --- Mistakes in the data ------------------------------------------------

fn errors(extra: &str) -> String {
    match common::try_with_core(extra) {
        Ok(_) => panic!("the definitions build, and were meant not to"),
        Err(error) => error,
    }
}

#[test]
fn mistakes_in_regions_and_settlements_are_located() {
    let cases = [
        (
            r#"[ { "type": "region_kind", "id": "k", "sites": [ { "site": "core:farmstead", "per_region": 1, "near": "coast" } ] } ]"#,
            "waits for realm plans",
        ),
        (
            r#"[ { "type": "region_kind", "id": "k", "patch_size": 99 } ]"#,
            "`patch_size` is from 1",
        ),
        (
            r#"[ { "type": "site", "id": "s", "map": "core:hut", "roles": { "desk": "mark:desk" } } ]"#,
            "has no mark `desk`",
        ),
        (
            r#"[ { "type": "path_kind", "id": "p", "width": 40, "surface": { "floor": "core:asphalt" } } ]"#,
            "`width` is from 1",
        ),
        (
            r#"[ { "type": "settlement", "id": "s", "size": [2, 2],
                   "streets": { "pattern": "radial", "spacing": [20, 30], "path": "core:street" },
                   "zones": [ { "zone": "z", "to": 1.0, "lots": ["house"] } ] } ]"#,
            "is not built yet",
        ),
        (
            r#"[ { "type": "settlement", "id": "s", "size": [2, 2],
                   "streets": { "pattern": "grid", "spacing": [5, 30], "path": "core:street" },
                   "zones": [ { "zone": "z", "to": 1.0, "lots": ["house"] } ] } ]"#,
            "`spacing` is",
        ),
        (
            r#"[ { "type": "settlement", "id": "s", "size": [2, 2],
                   "streets": { "pattern": "grid", "spacing": [20, 30], "path": "core:street" },
                   "zones": [ { "zone": "z", "to": 1.0, "lots": ["no_such_tag"] } ] } ]"#,
            "nothing can fill a lot",
        ),
    ];
    for (text, needle) in cases {
        let error = errors(text);
        assert!(error.contains(needle), "{needle:?} not in:\n{error}");
        assert!(error.contains("gen.json:"), "not located:\n{error}");
    }
}

#[test]
fn a_wrong_realm_parameter_is_refused_with_its_path() {
    let built = common::with_core(&format!(
        "[ {} ]",
        test_realm(
            r#""settlements": [ { "settlement": "core:town", "at": [60, 60] } ],
                "regions": [ { "kind": "core:town_country" } ]"#
        )
    ));
    let error = StandardGenerator::of_realm(&built.generation, "t:land").unwrap_err();
    let text = error.to_string();
    assert!(text.contains("generator_params.settlements"), "{text}");
    assert!(text.contains("beyond the realm"), "{text}");
}
