//! Tests for WG2: placing maps and applying them to the world (10 §4).

mod common;

use std::collections::BTreeMap;
use std::sync::Arc;

use clash_core::{Hash128, IVec3, Rng};
use clash_gen::ascii::surface_storey;
use clash_gen::map::MapId;
use clash_gen::place::{self, PlacedMap, placement};
use clash_gen::{
    MapMode, RayonRunner, SerialRunner, StandardGenerator, apply_map, apply_pending_maps,
    boxes_overlap, place_map, placement_box, realize_tracts,
};
use clash_world::{
    Cause, CauseKind, CellFlags, CellPos, Column, EngineFlag, Layer, Location, PendingEdit,
    RealizedTract, TractPos, WorldEvent,
};

/// The side of the map that spans four tracts, in cells.
const BIG: i32 = 30;

/// Where its north-west corner goes: near the corner four tracts share.
const BIG_AT: i32 = 20;

fn rows(side: i32, row: impl Fn(i32) -> String) -> String {
    (0..side)
        .map(|y| format!("\"{}\"", row(y)))
        .collect::<Vec<_>>()
        .join(", ")
}

/// The definitions these tests share: flat and hilly ground, maps of
/// every way of sitting, and realms that place them.
fn defs() -> String {
    let big = rows(BIG, |y| {
        if y == 0 || y == BIG - 1 {
            "#".repeat(BIG as usize)
        } else {
            format!("#....{}....#", "f".repeat((BIG - 10) as usize))
        }
    });
    let pad = rows(7, |_| ".".repeat(7));
    let realm = |id: &str, params: &str| {
        format!(
            r#"{{ "type": "realm", "id": "{id}", "size_regions": [1, 1], "wrap": [],
                 "z_range": [-16, 48], "generator": "standard",
                 "generator_params": {{ {params} }},
                 "fill": {{ "below": "solid@granite", "above": null }},
                 "strata": [ {{ "from_z": -16, "gravity": 9.81, "atmosphere": "air" }} ],
                 "time_rate": 1.0 }}"#
        )
    };
    let hill = r#""tracts": [ { "tract": "plain" } ],
                  "height": { "noise": { "octaves": 2, "scale": 0.4 }, "range": [0, 9] }"#;
    [
        r#"{ "type": "ground", "id": "bare",
             "steps": [ { "strata": [ { "depth": 2, "block": "solid@soil" },
                                      { "block": "solid@granite" } ] },
                        { "surface": { "floor": "grass", "ramp": "ramp@soil" } } ] }"#
            .to_owned(),
        r#"{ "type": "tract", "id": "plain", "ground": "bare" }"#.to_owned(),
        r#"{ "type": "tract", "id": "crowded", "ground": "bare",
             "place": [ { "map": "hut", "at": [2, 2] }, { "map": "hut", "at": [4, 4] },
                        { "map": "rug", "at": [3, 3] } ] }"#
            .to_owned(),
        format!(
            r#"{{ "type": "map", "id": "big", "size": [{BIG}, {BIG}], "sit": "none",
                 "palette": [ {{ "\#": "wall", ".": {{}},
                                "f": [ {{ "fixture": "table" }}, {{ "fixture": "chair" }}, {{}} ] }} ],
                 "default": {{ "floor": "slab" }},
                 "levels": {{ "0": [ {big} ], "1": "roof" }},
                 "place": [ {{ "map": "hut", "at": [12, 12, 0] }} ],
                 "areas": [ {{ "kind": "home", "name": "hall", "box": [[1, 1], [28, 28]] }} ],
                 "marks": {{ "centre": [15, 15, 0] }},
                 "spawns": [ {{ "creature": "rat", "count": 3, "in": "hall" }} ] }}"#
        )
        .replace("\\#", "#"),
        r#"{ "type": "map", "id": "rug", "size": [3, 3], "over": true, "sit": "none",
             "palette": [ { "r": { "floor": "pavement" } } ],
             "levels": { "0": [ "rrr", "r r", "rrr" ] } }"#
            .to_owned(),
        format!(
            r#"{{ "type": "map", "id": "pad_level", "size": [7, 7],
                 "palette": [ {{ ".": {{ "floor": "slab" }} }} ], "levels": {{ "0": [ {pad} ] }} }}"#
        ),
        format!(
            r#"{{ "type": "map", "id": "pad_found", "size": [7, 7],
                 "sit": "foundation", "foundation": "column",
                 "palette": [ {{ ".": {{ "floor": "slab" }} }} ], "levels": {{ "0": [ {pad} ] }} }}"#
        ),
        format!(
            r#"{{ "type": "map", "id": "pad_follow", "size": [7, 7], "sit": "follow",
                 "palette": [ {{ ".": {{ "floor": "slab" }} }} ], "levels": {{ "0": [ {pad} ] }} }}"#
        ),
        format!(
            r#"{{ "type": "map", "id": "pad_none", "size": [7, 7], "sit": "none",
                 "palette": [ {{ ".": {{ "floor": "slab" }} }} ], "levels": {{ "0": [ {pad} ] }} }}"#
        ),
        realm("flat_plain", r#""tracts": [ { "tract": "plain" } ]"#),
        realm(
            "flat_big",
            &format!(
                r#""tracts": [ {{ "tract": "plain" }} ],
                   "place": [ {{ "map": "big", "at": [{BIG_AT}, {BIG_AT}, 0], "seed": 77 }} ]"#
            ),
        ),
        realm(
            "flat_over",
            r#""tracts": [ { "tract": "plain" } ],
               "place": [ { "map": "hut", "at": [4, 4, 0], "seed": 5 },
                          { "map": "rug", "at": [5, 5, 0] } ]"#,
        ),
        realm("flat_crowded", r#""tracts": [ { "tract": "crowded" } ]"#),
        realm("hill_bare", hill),
        realm(
            "hill_placed",
            &format!(
                r#"{hill},
                   "place": [ {{ "map": "pad_level", "at": [2, 2] }},
                              {{ "map": "pad_found", "at": [12, 2] }},
                              {{ "map": "pad_follow", "at": [22, 2] }},
                              {{ "map": "pad_none", "at": [2, 14] }} ]"#
            ),
        ),
    ]
    .join(",\n")
    .to_string()
    .pipe(|body| format!("[{body}]"))
}

trait Pipe: Sized {
    fn pipe<T>(self, f: impl FnOnce(Self) -> T) -> T {
        f(self)
    }
}
impl Pipe for String {}

fn built() -> common::Built {
    common::with_core(&defs())
}

fn map(built: &common::Built, name: &str) -> MapId {
    built
        .generation
        .maps()
        .lookup(name)
        .unwrap_or_else(|| panic!("there is no map `{name}`"))
}

fn columns<'a>(
    tracts: &[TractPos],
    realised: &'a [RealizedTract],
) -> BTreeMap<TractPos, &'a Column> {
    tracts
        .iter()
        .copied()
        .zip(realised.iter().map(|tract| &tract.column))
        .collect()
}

const SEED: u64 = 0x5EED;

#[test]
fn a_map_spanning_four_tracts_is_the_same_in_every_order_and_on_every_thread_count() {
    let built = built();
    let def = built.realm("t:flat_big").unwrap();
    let tracts = common::block(2);
    let generators = |threads: usize| {
        let mut registry = clash_gen::registry_with(&built.generation);
        if threads == 0 {
            registry.set_runner(Arc::new(SerialRunner));
        } else {
            registry.set_runner(Arc::new(RayonRunner::with_threads(threads).unwrap()));
        }
        registry.bind(&built.world)
    };
    let hashes = |threads: usize, order: &[TractPos]| -> Vec<(TractPos, Hash128)> {
        let realised = realize_tracts(&generators(threads), def, SEED, order).unwrap();
        let mut hashes: Vec<(TractPos, Hash128)> = order
            .iter()
            .copied()
            .zip(realised.iter().map(|tract| Hash128::of(&tract.column)))
            .collect();
        hashes.sort_by_key(|(tract, _)| *tract);
        hashes
    };
    let straight = hashes(0, &tracts);
    for shuffle in 0..6 {
        let mut order = tracts.clone();
        Rng::from_seed(shuffle).shuffle(&mut order);
        assert_eq!(hashes(0, &order), straight, "shuffle {shuffle}");
        for threads in [1, 2, 7] {
            assert_eq!(hashes(threads, &order), straight, "{threads} threads");
        }
    }
    // Each tract on its own is the tract it is in a batch.
    for (tract, hash) in &straight {
        assert_eq!(hashes(0, &[*tract]), [(*tract, *hash)]);
    }

    // And together the four windows are the map: every cell it writes
    // is in the world, in whichever tract it falls.
    let realised = realize_tracts(&generators(0), def, SEED, &tracts).unwrap();
    let columns = columns(&tracts, &realised);
    let mut record = placement(map(&built, "t:big"), CellPos::new(BIG_AT, BIG_AT, 0));
    record.seed = 77;
    let placed = PlacedMap::resolve(built.generation.maps(), &record)
        .unwrap()
        .unwrap();
    assert_eq!(
        placed.bounds().tracts().collect::<Vec<_>>(),
        tracts,
        "the map covers the four tracts"
    );
    let mut written = 0;
    let mut by_tract: BTreeMap<TractPos, u32> = BTreeMap::new();
    for (cell, drawn) in placed.map.written_cells() {
        let at = placed.world(cell);
        let have = columns[&at.tract()].cell(at);
        if let Some(floor) = drawn.floor {
            assert_eq!(have.floor, floor, "{at:?}");
        }
        if let Some(block) = drawn.block {
            assert_eq!(have.block, block, "{at:?}");
        }
        if let Some(fixture) = drawn.fixture {
            assert_eq!(have.fixture, fixture, "{at:?}");
        }
        written += 1;
        *by_tract.entry(at.tract()).or_default() += 1;
    }
    assert!(written > 1000);
    assert_eq!(by_tract.len(), 4, "every tract holds a window of it");
    // The nested hut straddles the corner the four tracts share.
    let door = placed.world(IVec3::new(12 + 3, 12 + 4, 0));
    let closed = built.world.block_state("door", None).unwrap();
    assert_eq!(columns[&door.tract()].cell(door).block, closed);

    // What the map asks for besides cells is handed on once: each
    // spawn by the tract its cell is in, the area by its home tract.
    let spawns: usize = realised.iter().map(|tract| tract.spawns.len()).sum();
    assert_eq!(spawns, 3);
    for tract in &realised {
        for spawn in &tract.spawns {
            assert_eq!(spawn.what, "rat");
            assert_eq!(
                spawn.params.get(place::SPAWN_PARAM_KIND),
                Some(&clash_core::Value::Str(place::SPAWN_KIND_CREATURE.into()))
            );
            assert!(placed.bounds().contains(spawn.at));
        }
    }
    let areas: Vec<usize> = realised.iter().map(|tract| tract.areas.len()).collect();
    // The big map's hall and the nested hut's home, each made once.
    assert_eq!(areas.iter().sum::<usize>(), 2, "{areas:?}");
    assert_eq!(
        placed.marks()["centre"],
        [CellPos::new(BIG_AT + 15, BIG_AT + 15, 0)]
    );
}

#[test]
fn each_mode_of_apply_map_writes_what_it_says_and_every_change_is_an_edit_with_its_cause() {
    let hut_at = CellPos::new(5, 5, 0);
    let setup = || {
        let mut live = common::Live::new(built(), "t:flat_plain", SEED);
        live.load(&[TractPos::new(0, 0)]);
        // Something in the way: a boulder where the hut has open floor.
        let boulder = live.built.world.fixture_state("boulder", None).unwrap();
        let spot = Location::new(live.grid, CellPos::new(7, 7, 0));
        live.ctx()
            .edit(Cause::DEBUG)
            .set_fixture(spot, boulder)
            .unwrap();
        live.events.clear();
        live
    };
    let apply = |live: &mut common::Live, mode: MapMode| {
        let maps = Arc::clone(live.built.generation.maps());
        let mut record = placement(map(&live.built, "core:hut"), hut_at);
        record.seed = 3;
        let cause = Cause::new(CauseKind::Eoc);
        let grid = live.grid;
        let applied = apply_map(&mut live.ctx(), &maps, grid, &record, mode, cause).unwrap();
        // Invariant X8: the map changed the world only through edits,
        // each an `ElementChanged` with the cause given.
        let events = live.events.drain();
        assert_eq!(events.len() as u32, applied.changed, "{mode:?}");
        assert!(applied.changed > 0);
        for event in &events {
            match event {
                WorldEvent::ElementChanged { cause: by, .. } => assert_eq!(*by, cause),
                other => panic!("{mode:?} raised {}", other.name()),
            }
        }
        applied
    };
    let world = Arc::clone(&built().world);
    let grass = world.floor_state("grass", None).unwrap();
    let boards = world.floor_state("boards", None).unwrap();
    let roofing = world.floor_state("roofing", None).unwrap();
    let boulder = world.fixture_state("boulder", None).unwrap();
    let bed = world.fixture_state("bed", None).unwrap();

    // Replace: the map as drawn, clearing what was in its way.
    let mut live = setup();
    let applied = apply(&mut live, MapMode::Replace);
    assert!(applied.placed);
    assert!(!live.cell(5, 5, 0).block.is_none(), "a wall");
    assert_eq!(live.cell(6, 6, 0).floor, boards);
    assert_eq!(live.cell(6, 6, 0).fixture, bed);
    assert!(live.cell(7, 7, 0).fixture.is_none(), "the boulder is gone");
    assert_eq!(live.cell(7, 7, 1).floor, roofing);
    assert_eq!(
        live.cell(4, 5, 0).floor,
        grass,
        "outside it nothing changed"
    );
    assert_eq!(applied.marks["entrance"], [CellPos::new(8, 9, 0)]);
    assert_eq!(applied.areas.len(), 1);

    // Only empty: the ground storey holds grass, so only the roof,
    // drawn in empty air, is written.
    let mut live = setup();
    apply(&mut live, MapMode::OnlyEmpty);
    assert!(live.cell(5, 5, 0).block.is_none());
    assert_eq!(live.cell(6, 6, 0).floor, grass);
    assert_eq!(live.cell(7, 7, 0).fixture, boulder);
    assert_eq!(live.cell(7, 7, 1).floor, roofing);

    // Add: elements go where their layer is empty, and nothing is
    // cleared: the grass stays under the walls and the boulder stays.
    let mut live = setup();
    apply(&mut live, MapMode::Add);
    assert!(!live.cell(5, 5, 0).block.is_none(), "a wall");
    assert_eq!(live.cell(6, 6, 0).floor, grass);
    assert_eq!(live.cell(6, 6, 0).fixture, bed);
    assert_eq!(live.cell(7, 7, 0).fixture, boulder);
    assert_eq!(live.cell(7, 7, 1).floor, roofing);
}

/// The body of the test script `aftermath_edit`: a map left for tracts
/// that are not loaded is there when they load.
#[test]
fn a_pending_map_is_present_when_its_tract_loads() {
    let mut live = common::Live::new(built(), "t:flat_plain", SEED);
    let maps = Arc::clone(live.built.generation.maps());
    let (west, east) = (TractPos::new(0, 0), TractPos::new(1, 0));
    let realm = live.realm;
    // The map lies across the edge the two tracts share.
    let mut record = placement(map(&live.built, "t:big"), CellPos::new(BIG_AT, 1, 0));
    record.seed = 9;
    let cause = Cause::new(CauseKind::Eoc);

    // Nothing is surveyed yet: there is no record to leave it in.
    let applied = place_map(
        &mut live.ctx(),
        &maps,
        realm,
        &record,
        MapMode::Replace,
        cause,
    );
    let applied = applied.unwrap();
    assert_eq!((applied.changed, applied.pending.len()), (0, 0));
    assert_eq!(applied.unreached, [west, east]);

    live.cosmos
        .survey_region(&live.generators, realm, west.region())
        .unwrap();
    let applied = place_map(
        &mut live.ctx(),
        &maps,
        realm,
        &record,
        MapMode::Replace,
        cause,
    );
    let applied = applied.unwrap();
    assert_eq!(applied.changed, 0);
    assert_eq!(applied.pending, [west, east]);
    let pending = |live: &common::Live, tract: TractPos| {
        live.cosmos
            .tract_record(realm, tract)
            .unwrap()
            .pending()
            .iter()
            .filter(|edit| matches!(edit, PendingEdit::Map(_)))
            .count()
    };
    assert_eq!((pending(&live, west), pending(&live, east)), (1, 1));

    // The western tract loads: its window of the map is written, as
    // ordinary edits with the coarse cause, and the eastern still waits.
    live.load(&[west]);
    assert_eq!(pending(&live, west), 1, "the world cannot resolve a map");
    live.events.clear();
    let done = apply_pending_maps(&mut live.ctx(), &maps, realm, west);
    assert_eq!(done.len(), 1);
    assert!(done[0].placed && done[0].changed > 0);
    assert_eq!(pending(&live, west), 0);
    let events = live.events.drain();
    assert_eq!(events.len() as u32, done[0].changed);
    assert!(
        events
            .iter()
            .all(|event| event.cause().kind == CauseKind::Coarse)
    );
    assert!(!live.cell(BIG_AT, 1, 0).block.is_none(), "the west wall");
    assert_eq!(pending(&live, east), 1);
    // Applying again does nothing: the edit is gone.
    assert!(apply_pending_maps(&mut live.ctx(), &maps, realm, west).is_empty());

    live.load(&[east]);
    let rest = apply_pending_maps(&mut live.ctx(), &maps, realm, east);
    assert_eq!(rest.len(), 1);
    assert!(
        !live.cell(BIG_AT + BIG - 1, 1, 0).block.is_none(),
        "the east wall"
    );
    // Together the two windows are the whole map, and what it asks for
    // was handed over once.
    let placed = PlacedMap::resolve(&maps, &record).unwrap().unwrap();
    for (cell, drawn) in placed.map.written_cells() {
        let at = placed.world(cell);
        let have = live.cell(at.x, at.y, at.z);
        assert_eq!(Some(have.block), drawn.block.or(Some(have.block)), "{at:?}");
        assert_eq!(Some(have.floor), drawn.floor.or(Some(have.floor)), "{at:?}");
    }
    assert_eq!(done[0].spawns.len() + rest[0].spawns.len(), 3);
    assert_eq!(done[0].areas.len() + rest[0].areas.len(), 2);

    // A map put where one tract is loaded and the other is not: the
    // loaded part is written now, the rest waits.
    live.unload(east);
    let mut hut = placement(map(&live.built, "core:hut"), CellPos::new(29, 26, 0));
    hut.seed = 4;
    let applied = place_map(&mut live.ctx(), &maps, realm, &hut, MapMode::Replace, cause);
    let applied = applied.unwrap();
    assert!(applied.changed > 0);
    assert_eq!(applied.pending, [east]);
    assert!(!live.cell(29, 26, 0).block.is_none());
    live.load(&[east]);
    apply_pending_maps(&mut live.ctx(), &maps, realm, east);
    assert!(!live.cell(35, 26, 0).block.is_none(), "the hut's east wall");
}

/// The surface of each cell column of a tract with nothing on it.
fn terrain(column: &Column) -> impl Fn(i32, i32) -> i32 + '_ {
    |x, y| surface_storey(column, CellPos::new(x, y, 0))
}

fn pad_columns(x: i32, y: i32) -> impl Iterator<Item = (i32, i32)> {
    (0..7).flat_map(move |dy| (0..7).map(move |dx| (x + dx, y + dy)))
}

#[test]
fn a_map_on_a_slope_is_levelled_founded_or_followed_as_it_asks() {
    let built = built();
    let tract = [TractPos::new(0, 0)];
    let bare = common::realise(&built, "t:hill_bare", SEED, &tract).unwrap();
    let placed = common::realise(&built, "t:hill_placed", SEED, &tract).unwrap();
    let (bare, placed) = (&bare[0].column, &placed[0].column);
    let ground = terrain(bare);
    let slab = built.world.floor_state("slab", None).unwrap();
    let pier = built.world.block_state("column", None).unwrap();
    let solid = |column: &Column, x: i32, y: i32, z: i32| {
        column
            .flags(CellPos::new(x, y, z))
            .contains(CellFlags::SOLID_TOP)
    };
    let slope = |x: i32, y: i32| {
        let heights: Vec<i32> = pad_columns(x, y).map(|(x, y)| ground(x, y)).collect();
        heights.iter().max().unwrap() - heights.iter().min().unwrap()
    };
    for (x, y) in [(2, 2), (12, 2), (22, 2), (2, 14)] {
        assert!(
            slope(x, y) > 0,
            "the ground under the pad at {x}, {y} is flat"
        );
    }

    // Level: the ground under the footprint is at the height of the
    // origin, with the ground's own strata under it and its ramps
    // round it.
    let level = ground(2, 2);
    for (x, y) in pad_columns(2, 2) {
        let at = CellPos::new(x, y, level);
        assert_eq!(placed.cell(at).floor, slab, "{at:?}");
        assert!(solid(placed, x, y, level - 1), "ground under {at:?}");
        assert!((level..level + 12).all(|z| placed.cell(CellPos::new(x, y, z)).block.is_none()));
    }
    let soil = built.world.block_state("solid@soil", None).unwrap();
    assert_eq!(placed.cell(CellPos::new(5, 5, level - 1)).block, soil);
    let surface = terrain(placed);
    let mut ramps = 0;
    for y in 1..31 {
        for x in 1..31 {
            let own = surface(x, y);
            let steps_up = [(0, -1), (1, 0), (0, 1), (-1, 0)]
                .into_iter()
                .any(|(dx, dy)| surface(x + dx, y + dy) == own + 1);
            let block = placed.cell(CellPos::new(x, y, own)).block;
            let ramp = built.world.block_info(block).flags.has(EngineFlag::Ramp);
            // The founded pad's fill and the pads' own cells aside, a
            // ramp stands exactly where the surface steps up a storey.
            let on_pad = [(12, 2), (22, 2), (2, 14)]
                .into_iter()
                .any(|(px, py)| (px - 1..px + 8).contains(&x) && (py - 1..py + 8).contains(&y));
            if !on_pad {
                assert_eq!(ramp, steps_up, "at {x}, {y}");
                ramps += u32::from(ramp);
            }
        }
    }
    assert!(ramps > 0);

    // Foundation: the ground is left; gaps under the map are filled
    // with the state it names, and ground above its floor is cut away.
    let level = ground(12, 2);
    let (mut filled, mut cut) = (0, 0);
    for (x, y) in pad_columns(12, 2) {
        let own = ground(x, y);
        assert_eq!(placed.cell(CellPos::new(x, y, level)).floor, slab);
        for z in own..level {
            assert_eq!(
                placed.cell(CellPos::new(x, y, z)).block,
                pier,
                "{x}, {y}, {z}"
            );
            filled += 1;
        }
        for z in level..own.max(level) + 2 {
            assert!(placed.cell(CellPos::new(x, y, z)).block.is_none());
            cut += u32::from(z < own);
        }
        assert!(solid(placed, x, y, own.min(level) - 1));
    }
    assert!(filled + cut > 0);

    // Follow: each column at its own height.
    for (x, y) in pad_columns(22, 2) {
        let at = CellPos::new(x, y, ground(x, y));
        assert_eq!(placed.cell(at).floor, slab, "{at:?}");
    }

    // None: where the placement says, whatever the ground does.
    let level = ground(2, 14);
    for (x, y) in pad_columns(2, 14) {
        let at = CellPos::new(x, y, level);
        assert_eq!(placed.cell(at).floor, slab, "{at:?}");
        assert!(placed.cell(at).block.is_none());
        if ground(x, y) > level + 1 {
            assert!(
                solid(placed, x, y, level + 1),
                "the ground over {at:?} is left"
            );
        }
    }
}

#[test]
fn a_map_applied_to_a_slope_in_the_world_sits_as_it_asks() {
    let mut live = common::Live::new(built(), "t:hill_bare", SEED);
    let tract = TractPos::new(0, 0);
    live.load(&[tract]);
    let maps = Arc::clone(live.built.generation.maps());
    let before = live
        .cosmos
        .grid(live.grid)
        .unwrap()
        .column(tract)
        .unwrap()
        .clone();
    let ground = terrain(&before);
    let slab = live.built.world.floor_state("slab", None).unwrap();
    let pier = live.built.world.block_state("column", None).unwrap();
    let grid = live.grid;
    let put = |live: &mut common::Live, name: &str, x: i32, y: i32| {
        let record = placement(map(&live.built, name), CellPos::new(x, y, ground(x, y)));
        apply_map(
            &mut live.ctx(),
            &maps,
            grid,
            &record,
            MapMode::Replace,
            Cause::DEBUG,
        )
        .unwrap()
    };

    let level = ground(2, 2);
    put(&mut live, "t:pad_level", 2, 2);
    for (x, y) in pad_columns(2, 2) {
        assert_eq!(live.cell(x, y, level).floor, slab);
        assert!(!live.cell(x, y, level - 1).block.is_none(), "filled under");
        assert!((level..level + 12).all(|z| live.cell(x, y, z).block.is_none()));
    }

    let level = ground(12, 2);
    put(&mut live, "t:pad_found", 12, 2);
    for (x, y) in pad_columns(12, 2) {
        assert_eq!(live.cell(x, y, level).floor, slab);
        for z in ground(x, y)..level {
            assert_eq!(live.cell(x, y, z).block, pier);
        }
        assert!((level..level + 12).all(|z| live.cell(x, y, z).block.is_none()));
    }

    put(&mut live, "t:pad_follow", 22, 2);
    for (x, y) in pad_columns(22, 2) {
        assert_eq!(live.cell(x, y, ground(x, y)).floor, slab);
    }
    assert!(
        live.unsupported(tract).is_empty(),
        "{:?}",
        live.unsupported(tract)
    );
}

#[test]
fn a_later_placement_overwrites_an_earlier_one_only_where_it_draws() {
    let built = built();
    let tract = [TractPos::new(0, 0)];
    let realised = common::realise(&built, "t:flat_over", SEED, &tract).unwrap();
    let column = &realised[0].column;
    let boards = built.world.floor_state("boards", None).unwrap();
    let paving = built.world.floor_state("pavement", None).unwrap();
    // The rug's ring lies over the hut's floor; its middle, which it
    // does not draw, keeps what the hut put there.
    assert_eq!(column.cell(CellPos::new(5, 5, 0)).floor, paving);
    assert_eq!(column.cell(CellPos::new(7, 7, 0)).floor, paving);
    assert_eq!(column.cell(CellPos::new(6, 6, 0)).floor, boards);
    assert_eq!(column.cell(CellPos::new(8, 6, 0)).floor, boards);
    // A cell the rug draws is the rug's on every layer: the hut's bed
    // at its north-west corner is gone.
    assert!(column.cell(CellPos::new(5, 5, 0)).fixture.is_none());
}

#[test]
fn a_survey_drops_a_placement_that_meets_an_earlier_one_unless_its_map_may_overlap() {
    let built = built();
    let generator = StandardGenerator::of_realm(&built.generation, "t:flat_crowded").unwrap();
    let tract = TractPos::new(2, 3);
    let def = generator.tract_def(SEED, tract);
    let (written, dropped) = generator.placements_of(SEED, tract, def);
    let maps = built.generation.maps();
    // The second hut meets the first and is dropped; the rug says
    // `over` and stays.
    assert_eq!(written.len(), 2);
    assert_eq!(dropped.len(), 1);
    assert_eq!(dropped[0].tract, tract);
    assert!(boxes_overlap(dropped[0].bounds, dropped[0].met));
    assert_eq!(
        Some(dropped[0].met),
        placement_box(maps, &written[0]),
        "it met the first hut"
    );
    assert_eq!(place::gen_map_id(written[1].map), map(&built, "t:rug"));
    assert!(!generator.dropped_in(SEED, tract.region()).is_empty());

    // Invariant X10 over a whole surveyed region: no two placements
    // overlap unless the later one's map says it may.
    let mut live = common::Live::new(built, "t:flat_crowded", SEED);
    let realm = live.realm;
    live.cosmos
        .survey_region(&live.generators, realm, tract.region())
        .unwrap();
    let record = live.cosmos.region(realm, tract.region()).unwrap();
    let maps = live.built.generation.maps();
    let boxes: Vec<_> = record
        .placements
        .iter()
        .map(|placement| {
            let over = maps
                .get(place::gen_map_id(placement.map))
                .is_some_and(|map| map.over());
            (placement_box(maps, placement).unwrap(), over)
        })
        .collect();
    assert_eq!(boxes.len(), 2 * clash_world::dims::REGION_TRACT_COUNT);
    for (index, (bounds, over)) in boxes.iter().enumerate().take(200) {
        for (earlier, _) in &boxes[..index] {
            assert!(*over || !boxes_overlap(*bounds, *earlier));
        }
    }
    let _ = Layer::Block;
}
