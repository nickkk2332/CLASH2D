//! Resolving maps: what a character puts in a cell, parameters, chance,
//! nested maps, turning, and the invariants of 10 §14 that belong to
//! the map format.

mod common;

use std::collections::BTreeSet;

use clash_core::{
    Dir8, Hash128, IVec2, IVec3, Real, RngStreams, Rot4, Turn, Value, VarMap, Vertical,
};
use clash_gen::map::{
    BerthCell, ClearedLayer, EvalOrder, ItemSource, JointDir, MapDefs, MapShape, PortalTo,
    ResolveError, ResolvedCell, ResolvedMap, SpawnPlace, SpawnWho,
};
use clash_world::{BlockState, FixtureState, FloorState};
use common::build;
use proptest::prelude::*;

const SEED: u64 = 0x5eed;

fn no_params() -> VarMap {
    VarMap::default()
}

fn id(maps: &MapDefs, name: &str) -> clash_gen::map::MapId {
    maps.lookup(&format!("t:{name}"))
        .unwrap_or_else(|| panic!("no map `{name}`"))
}

fn resolve(maps: &MapDefs, name: &str, seed: u64) -> ResolvedMap {
    maps.resolve(id(maps, name), &no_params(), seed, Turn::IDENTITY)
        .unwrap_or_else(|error| panic!("{error}"))
        .unwrap_or_else(|| panic!("`{name}` is not placed"))
}

fn floor(maps: &MapDefs, text: &str) -> FloorState {
    maps.world().floor_state(text, Some("t")).unwrap()
}

fn block(maps: &MapDefs, text: &str) -> BlockState {
    maps.world().block_state(text, Some("t")).unwrap()
}

fn fixture(maps: &MapDefs, text: &str) -> FixtureState {
    maps.world().fixture_state(text, Some("t")).unwrap()
}

fn cell(x: i32, y: i32, z: i32) -> IVec3 {
    IVec3::new(x, y, z)
}

#[test]
fn a_character_replaces_a_cell_and_a_space_leaves_it() {
    let maps = build(
        r##"[
  { "type": "map", "id": "room", "size": [4, 3],
    "palette": [ { "W": "wall", ".": {}, "T": { "fixture": "table" },
                   "o": { "clear": ["floor"] },
                   "k": { "keep": ["floor", "block"], "fixture": "table" },
                   "d": "door#open", "p": "paving" } ],
    "default": { "floor": "boards" },
    "levels": { "0": [ "WWdW",
                       "W.T ",
                       "Wokp" ] } }
]"##,
    );
    let room = resolve(&maps, "room", SEED);
    assert_eq!(room.size, IVec2::new(4, 3));
    assert_eq!(room.level_range(), (0, 0));
    assert_eq!(maps.ascii(&room), "level 0\n##'#\n#.T\n#:T,\n");

    // A space: nothing is touched.
    assert_eq!(room.cell(3, 1, 0), ResolvedCell::LEAVE);
    // Outside the map nothing is touched either.
    assert_eq!(room.cell(4, 0, 0), ResolvedCell::LEAVE);
    assert_eq!(room.cell(0, 0, 1), ResolvedCell::LEAVE);
    // What is written, in order: every cell but the one left alone.
    let written: Vec<IVec3> = room.written_cells().map(|(at, _)| at).collect();
    assert_eq!(written.len(), 11);
    assert_eq!(written[0], cell(0, 0, 0));
    assert_eq!(written[6], cell(2, 1, 0));
    assert_eq!(written[7], cell(0, 2, 0));
    assert!(
        room.written_cells()
            .all(|(at, what)| room.cell(at.x, at.y, at.z) == what)
    );

    // `.` says nothing itself: the default's floor, and the other two
    // layers emptied. Everything but a space replaces.
    assert_eq!(
        room.cell(1, 1, 0),
        ResolvedCell {
            floor: Some(floor(&maps, "boards")),
            block: Some(BlockState::NONE),
            fixture: Some(FixtureState::NONE),
        }
    );
    // The short form found its layer, and the default lies under it.
    assert_eq!(
        room.cell(0, 0, 0),
        ResolvedCell {
            floor: Some(floor(&maps, "boards")),
            block: Some(block(&maps, "wall")),
            fixture: Some(FixtureState::NONE),
        }
    );
    assert_eq!(room.cell(2, 0, 0).block, Some(block(&maps, "door#open")));
    assert_eq!(room.cell(3, 2, 0).floor, Some(floor(&maps, "paving")));
    // `clear` empties a layer the default would have filled.
    assert_eq!(room.cell(1, 2, 0).floor, Some(FloorState::NONE));
    // `keep` leaves layers as the world has them.
    assert_eq!(
        room.cell(2, 2, 0),
        ResolvedCell {
            floor: None,
            block: None,
            fixture: Some(fixture(&maps, "table")),
        }
    );
}

#[test]
fn the_roof_preset_covers_what_the_storey_below_draws() {
    let maps = build(
        r##"[
  { "type": "map", "id": "shed", "size": [3, 2],
    "palette": [ { "W": "wall", ".": {} } ],
    "default": { "floor": "boards" },
    "roof": { "floor": "paving" },
    "levels": { "0": [ "W.W", "W  " ], "1": "roof", "2": "roof" } },
  { "type": "map", "id": "plain", "size": [2, 1],
    "palette": [ { ".": {} } ],
    "default": { "floor": "boards" },
    "levels": { "0": [ ". " ], "1": "roof" } }
]"##,
    );
    let shed = resolve(&maps, "shed", SEED);
    assert_eq!(
        maps.ascii(&shed),
        "level 0\n#.#\n#\nlevel 1\n,,,\n,\nlevel 2\n,,,\n,\n"
    );
    assert_eq!(shed.cell(1, 1, 1), ResolvedCell::LEAVE);
    assert_eq!(shed.cell(1, 0, 1).block, Some(BlockState::NONE));
    // With no `roof`, the roof is the default.
    let plain = resolve(&maps, "plain", SEED);
    assert_eq!(plain.cell(0, 0, 1).floor, Some(floor(&maps, "boards")));
    assert_eq!(plain.cell(1, 0, 1), ResolvedCell::LEAVE);
}

#[test]
fn set_lays_cell_specs_at_cells_and_over_boxes() {
    let maps = build(
        r##"[
  { "type": "map", "id": "hall", "size": [4, 2],
    "palette": [ { ".": {} } ],
    "default": { "floor": "boards" },
    "levels": { "0": [ "....", ".   " ], "1": [ "....", "...." ] },
    "marks": { "centre": [2, 0, 1] },
    "set": [ { "box": [[1, 0], [2, 1]], "level": [0, 1], "spec": { "floor": "paving" } },
             { "at": [3, 1, 0], "spec": "wall" },
             { "at": "mark:centre", "spec": { "fixture": "table" } } ] }
]"##,
    );
    let hall = resolve(&maps, "hall", SEED);
    assert_eq!(
        maps.ascii(&hall),
        "level 0\n.,,.\n.,,#\nlevel 1\n.,T.\n.,,.\n"
    );
    // A `set` over a cell no character drew draws it: the rest is
    // emptied.
    assert_eq!(hall.cell(3, 1, 0).floor, Some(FloorState::NONE));
    // Over a drawn cell it changes only what it states.
    assert_eq!(hall.cell(2, 0, 1).floor, Some(floor(&maps, "paving")));
}

#[test]
fn parameters_are_given_or_drawn_and_checked() {
    let maps = build(
        r##"[
  { "type": "map", "id": "hut", "size": [2, 1],
    "params": { "wall": { "pick": ["stone", "oak"] },
                "beds": { "range": [1, 3] },
                "wealth": 2,
                "rich": "$wealth > 2",
                "owner": { "type": "string" } },
    "if": "$wealth > 0",
    "palette": [ { "W": "wall@$wall",
                   "b": { "fixture": "table", "items": { "item": "bedding", "count": "$beds" } } } ],
    "levels": { "0": [ "Wb" ] } }
]"##,
    );
    let hut = id(&maps, "hut");
    let compiled = maps.get(hut).unwrap();
    let declared: Vec<(&str, &str, bool)> = compiled
        .params()
        .iter()
        .map(|param| (param.name.as_str(), param.ty.name(), param.is_required()))
        .collect();
    assert_eq!(
        declared,
        [
            ("wall", "string", false),
            ("beds", "int", false),
            ("wealth", "int", false),
            ("rich", "bool", false),
            ("owner", "string", true),
        ]
    );

    let mut given = VarMap::default();
    given.set("owner", "mara");
    // Drawn by the map's own rule: the same for the same seed, and
    // over many seeds every choice turns up.
    let mut walls = BTreeSet::new();
    let mut beds = BTreeSet::new();
    for seed in 0..40 {
        let once = maps
            .resolve(hut, &given, seed, Turn::IDENTITY)
            .unwrap()
            .unwrap();
        let again = maps
            .resolve(hut, &given, seed, Turn::IDENTITY)
            .unwrap()
            .unwrap();
        assert_eq!(once, again);
        walls.insert(once.cell(0, 0, 0).block.unwrap());
        beds.insert(once.items[0].count);
    }
    assert_eq!(
        walls,
        BTreeSet::from([block(&maps, "wall@stone"), block(&maps, "wall@oak")])
    );
    assert_eq!(beds, BTreeSet::from([1, 2, 3]));

    // Given by the placement.
    given.set("wall", "oak");
    given.set("beds", 7);
    let made = maps
        .resolve(hut, &given, SEED, Turn::IDENTITY)
        .unwrap()
        .unwrap();
    assert_eq!(made.cell(0, 0, 0).block, Some(block(&maps, "wall@oak")));
    assert_eq!(made.items[0].count, 7);
    assert_eq!(made.items[0].source, ItemSource::Item("bedding".to_owned()));

    // `if` false: the map is not placed.
    given.set("wealth", 0);
    assert_eq!(maps.resolve(hut, &given, SEED, Turn::IDENTITY), Ok(None));
    given.set("wealth", 2);

    // Refusals, each with its reason.
    let missing = maps
        .resolve(hut, &no_params(), SEED, Turn::IDENTITY)
        .unwrap_err();
    assert_eq!(
        missing.to_string(),
        "the map `t:hut` needs the parameter `owner`, and the placement gives none"
    );
    let mut unknown = given.clone();
    unknown.set("colour", "red");
    assert!(matches!(
        maps.resolve(hut, &unknown, SEED, Turn::IDENTITY),
        Err(ResolveError::UnknownParam { .. })
    ));
    let mut wrong = given.clone();
    wrong.set("beds", "many");
    let error = maps.resolve(hut, &wrong, SEED, Turn::IDENTITY).unwrap_err();
    assert_eq!(
        error.to_string(),
        "the parameter `beds` of the map `t:hut` is a int, and was given the string `many`"
    );
    let mut paper = given.clone();
    paper.set("wall", "paper");
    let error = maps.resolve(hut, &paper, SEED, Turn::IDENTITY).unwrap_err();
    assert!(
        error
            .to_string()
            .contains("the state `wall@paper` cannot be used"),
        "{error}"
    );
    assert!(matches!(
        maps.resolve(clash_gen::map::MapId::new(99), &given, SEED, Turn::IDENTITY),
        Err(ResolveError::UnknownMap(_))
    ));
}

#[test]
fn a_parameter_drawn_by_an_expression_takes_its_type_from_it() {
    let maps = build(
        r##"[
  { "type": "map", "id": "post", "size": [1, 1],
    "params": { "roll": "rand_int(1, 6)",
                "wall": { "expr": "pick(['stone', 'oak'])", "type": "string" },
                "tall": "$roll > 3",
                "half": "$roll / 2" },
    "palette": [ { "W": "wall@$wall",
                   "t": { "items": { "item": "die", "count": "$roll" } } } ],
    "levels": { "0": [ "W" ] },
    "set": [ { "at": [0, 0, 0], "spec": { "items": { "item": "die", "count": "$roll" } } } ] }
]"##,
    );
    let post = id(&maps, "post");
    let types: Vec<(&str, &str)> = maps
        .get(post)
        .unwrap()
        .params()
        .iter()
        .map(|param| (param.name.as_str(), param.ty.name()))
        .collect();
    assert_eq!(
        types,
        [
            ("roll", "int"),
            ("wall", "string"),
            ("tall", "bool"),
            ("half", "real")
        ]
    );
    let mut walls = BTreeSet::new();
    let mut rolls = BTreeSet::new();
    for seed in 0..60 {
        let map = resolve(&maps, "post", seed);
        walls.insert(map.cell(0, 0, 0).block.unwrap());
        rolls.insert(map.items[0].count);
    }
    assert_eq!(walls.len(), 2);
    assert_eq!(rolls, BTreeSet::from([1, 2, 3, 4, 5, 6]));
}

#[test]
fn weighted_alternatives_are_chosen_for_each_cell_or_once() {
    let maps = build(
        r##"[
  { "type": "map", "id": "mixed", "size": [6, 3],
    "params": { "tables": { "default": 0.5 } },
    "palette": [ { "f": [ { "fixture": "table", "weight": 3 }, { "fixture": "mat" }, {} ],
                   "g": { "one_of": [ { "fixture": "table" }, { "fixture": "mat" }, {} ],
                          "once": true },
                   "n": [ { "fixture": "table", "weight": 0 }, { "fixture": "mat" } ],
                   "p": [ { "fixture": "table", "weight": "$tables" },
                          { "fixture": "mat", "weight": "1 - $tables" } ] } ],
    "default": { "floor": "boards" },
    "levels": { "0": [ "ffffff", "gggggg", "nnnppp" ] } }
]"##,
    );
    let mixed = id(&maps, "mixed");
    let row = |map: &ResolvedMap, y: i32, from: i32, to: i32| -> Vec<FixtureState> {
        (from..to)
            .map(|x| map.cell(x, y, 0).fixture.unwrap())
            .collect()
    };
    let table = fixture(&maps, "table");
    let mat = fixture(&maps, "mat");
    let mut per_cell = BTreeSet::new();
    let mut once = BTreeSet::new();
    let mut differs_within_a_row = false;
    for seed in 0..60 {
        let map = resolve(&maps, "mixed", seed);
        let cells = row(&map, 0, 0, 6);
        differs_within_a_row |= cells.iter().any(|cell| *cell != cells[0]);
        per_cell.extend(cells);
        // Once for the placement: every cell of the row is the same.
        let whole = row(&map, 1, 0, 6);
        assert!(whole.iter().all(|cell| *cell == whole[0]), "seed {seed}");
        once.insert(whole[0]);
        // A weight of nothing is never chosen.
        assert!(row(&map, 2, 0, 3).iter().all(|cell| *cell == mat));
    }
    let all = BTreeSet::from([table, mat, FixtureState::NONE]);
    assert_eq!(per_cell, all);
    assert_eq!(once, all);
    assert!(differs_within_a_row);

    // A weight may be an expression over the parameters.
    for (tables, expected) in [(1.0, table), (0.0, mat)] {
        let mut given = VarMap::default();
        given.set("tables", Real::of(tables));
        let map = maps
            .resolve(mixed, &given, SEED, Turn::IDENTITY)
            .unwrap()
            .unwrap();
        assert!(row(&map, 2, 3, 6).iter().all(|cell| *cell == expected));
    }
}

#[test]
fn a_nested_map_is_laid_in_turned_and_given_parameters() {
    let maps = build(
        r##"[
  { "type": "map", "id": "sign", "tags": ["sign"], "size": [2, 1],
    "params": { "post": { "type": "string" } },
    "palette": [ { "a": { "fixture": "arrow#north", "mark": "arrow" }, "p": "wall@$post" } ],
    "levels": { "0": [ "ap" ] } },
  { "type": "map", "id": "yard", "size": [4, 4],
    "params": { "wood": { "default": "oak" } },
    "palette": [ { ".": {} } ],
    "default": { "floor": "boards" },
    "levels": { "0": [ "....", "....", "....", "...." ] },
    "place": [ { "map": { "tag": "sign" }, "at": [1, 1, 0], "turn": 90,
                 "params": { "post": "$wood" } },
               { "map": "sign", "at": [0, 3, 0], "params": { "post": "'stone'" }, "chance": 0 } ] }
]"##,
    );
    let yard = resolve(&maps, "yard", SEED);
    // A quarter-turn clockwise: the arrow that pointed north points
    // east, and the post that was east of it is south of it.
    assert_eq!(maps.ascii(&yard), "level 0\n....\n.>..\n.#..\n....\n");
    assert_eq!(
        yard.cell(1, 1, 0).fixture,
        Some(fixture(&maps, "arrow#east"))
    );
    assert_eq!(yard.cell(1, 2, 0).block, Some(block(&maps, "wall@oak")));
    // The nested map stated no floor: the yard's shows through.
    assert_eq!(yard.cell(1, 1, 0).floor, Some(floor(&maps, "boards")));
    // Its marks are the yard's, where they landed.
    assert_eq!(yard.marks["arrow"], [cell(1, 1, 0)]);
    // A chance of nothing places nothing.
    assert_eq!(yard.cell(0, 3, 0).fixture, Some(FixtureState::NONE));
}

#[test]
fn a_map_with_an_anchor_is_placed_by_it() {
    let maps = build(
        r##"[
  { "type": "map", "id": "gate", "size": [3, 1], "anchor": [1, 0, 0],
    "palette": [ { "W": "wall", "d": "door" } ],
    "levels": { "0": [ "WdW" ] } },
  { "type": "map", "id": "field", "size": [5, 5],
    "palette": [ { ".": {} } ],
    "default": { "floor": "boards" },
    "levels": { "0": [ ".....", ".....", ".....", ".....", "....." ] },
    "place": [ { "map": "gate", "at": [2, 0, 0] },
               { "map": "gate", "at": [4, 3, 0], "turn": 90 } ] }
]"##,
    );
    let field = resolve(&maps, "field", SEED);
    // The door, the anchor, is on the cell named, however it is turned.
    assert_eq!(
        maps.ascii(&field),
        "level 0\n.#+#.\n.....\n....#\n....+\n....#\n"
    );
    let gate = resolve(&maps, "gate", SEED);
    assert_eq!(gate.anchor_cell(), cell(1, 0, 0));
    let turned = gate.turned(Turn::from(Rot4::R90), &maps);
    assert_eq!(turned.anchor_cell(), cell(0, 1, 0));
    // A map that names no anchor is placed by its north-west corner,
    // as turned.
    assert_eq!(field.anchor, None);
    assert_eq!(
        field.turned(Turn::from(Rot4::R180), &maps).anchor_cell(),
        cell(0, 0, 0)
    );
}

#[test]
fn maps_are_scattered_through_an_area_with_spacing() {
    let maps = build(
        r##"[
  { "type": "map", "id": "rug", "size": [1, 1],
    "palette": [ { "r": { "fixture": "mat" } } ],
    "levels": { "0": [ "r" ] } },
  { "type": "map", "id": "hall", "size": [8, 8],
    "palette": [ { ".": {} } ],
    "default": { "floor": "boards" },
    "levels": { "0": [ "........", "........", "........", "........",
                       "........", "........", "........", "........" ] },
    "areas": [ { "kind": "room", "name": "middle", "box": [[1, 1], [6, 6]] } ],
    "place": [ { "map": "rug", "in": "area:middle", "count": 4, "apart": 1 } ] }
]"##,
    );
    let mat = fixture(&maps, "mat");
    let mut layouts = BTreeSet::new();
    for seed in 0..20 {
        let hall = resolve(&maps, "hall", seed);
        let rugs: Vec<IVec2> = (0..8)
            .flat_map(|y| (0..8).map(move |x| IVec2::new(x, y)))
            .filter(|at| hall.cell(at.x, at.y, 0).fixture == Some(mat))
            .collect();
        assert_eq!(rugs.len(), 4, "seed {seed}");
        for (index, rug) in rugs.iter().enumerate() {
            assert!((1..=6).contains(&rug.x) && (1..=6).contains(&rug.y));
            // The floor under a rug is still the hall's.
            assert_eq!(
                hall.cell(rug.x, rug.y, 0).floor,
                Some(floor(&maps, "boards"))
            );
            for other in &rugs[index + 1..] {
                let apart = IVec2::new(rug.x - other.x, rug.y - other.y).chebyshev();
                assert!(apart > 1, "seed {seed}: {rug:?} and {other:?}");
            }
        }
        layouts.insert(rugs);
    }
    assert!(layouts.len() > 10, "the seed should decide where they go");
}

/// A map that says a little of everything a map can say.
const EVERYTHING: &str = r##"[
  { "type": "map", "id": "everything", "size": [4, 3], "front": "south",
    "palette": [ { ".": {},
      "s": { "fixture": "table", "spawn": { "creature": "rat", "count": 2 },
             "items": { "table": "scraps", "count": [3, 3] } },
      "d": { "block": "door", "device": { "name": "gate", "config": { "locked": true } },
             "watched": true },
      "l": { "fixture": "table", "device": "lever" },
      "b": { "berth": { "name": "bay", "kind": "parking" } },
      "a": { "approach": "bay" },
      "m": { "mark": ["post", "corner"], "area": "room" },
      "v": { "var": { "depth": 3 }, "decal": "stain", "conduits": ["pipe"],
             "field": { "smoke": 0.25 }, "clear": ["items"] },
      "t": { "fixture_tag": "lamp", "any": true } } ],
    "default": { "floor": "boards" },
    "levels": { "0": [ "sdlb",
                       "mm.a",
                       "vtm." ] },
    "marks": { "entrance": [1, 0, 0] },
    "areas": [ { "kind": "room", "name": "all", "level": 0, "box": [[0, 0], [3, 2]],
                 "tags": ["whole"] } ],
    "spawns": [ { "population": "householders", "count": [2, 2], "in": "all" },
                { "creature": "cat", "at": "mark:entrance" } ],
    "wiring": { "palette": { "=": ["cable"] },
                "levels": { "0": [ " == ", "    ", "    " ] },
                "links": [ { "from": "lever", "to": "gate", "resource": "control" } ] },
    "joints": [ { "id": "front", "at": [1, 2, 0], "dir": "south", "kind": "doorway" },
                { "id": "hatch", "at": [3, 0, 0], "dir": "up", "kind": "hatch" } ],
    "portals": [ { "from": "mark:entrance", "to": "mark:post" },
                 { "from": "mark:post", "to": "cellar_door" } ],
    "vehicles": [ { "vehicle": "cart", "at": [3, 2, 0], "facing": "north" } ],
    "base": "sea_level", "sit": "foundation", "foundation": "wall", "over": true }
]"##;

#[test]
fn what_the_world_cannot_take_yet_is_carried_as_data() {
    let maps = build(EVERYTHING);
    let compiled = maps.get(id(&maps, "everything")).unwrap();
    assert_eq!(compiled.base(), clash_gen::map::Base::SeaLevel);
    assert_eq!(
        compiled.sit(),
        clash_gen::map::Sit::Foundation(Some(block(&maps, "wall")))
    );
    assert!(compiled.over() && compiled.may_turn());
    assert_eq!(compiled.front(), Some(Dir8::South));
    // Marks that are the same in every placement are known before any.
    assert_eq!(
        compiled.marks()["post"],
        [cell(0, 1, 0), cell(1, 1, 0), cell(2, 2, 0)]
    );

    let map = resolve(&maps, "everything", SEED);
    assert_eq!(map.front, Some(Dir8::South));
    assert_eq!(
        map.marks["corner"],
        [cell(0, 1, 0), cell(1, 1, 0), cell(2, 2, 0)]
    );
    assert_eq!(map.marks["entrance"], [cell(1, 0, 0)]);

    // Areas: the box, and the cells that touch as one area each.
    let shapes: Vec<(&str, &MapShape)> = map
        .areas
        .iter()
        .map(|area| (area.name.as_str(), &area.shape))
        .collect();
    assert_eq!(
        shapes,
        [
            (
                "all",
                &MapShape::Box {
                    min: cell(0, 0, 0),
                    max: cell(3, 2, 0)
                }
            ),
            ("room", &MapShape::Cells(vec![cell(0, 1, 0), cell(1, 1, 0)])),
            ("room", &MapShape::Cells(vec![cell(2, 2, 0)])),
        ]
    );
    assert_eq!(map.areas[0].tags, ["whole"]);
    assert!(map.areas.iter().all(|area| area.kind.is_some()));

    let spawns: Vec<(&SpawnWho, u32, &SpawnPlace)> = map
        .spawns
        .iter()
        .map(|spawn| (&spawn.who, spawn.count, &spawn.place))
        .collect();
    assert_eq!(
        spawns,
        [
            (
                &SpawnWho::Creature("rat".to_owned()),
                2,
                &SpawnPlace::Cell(cell(0, 0, 0))
            ),
            (
                &SpawnWho::Creature("cat".to_owned()),
                1,
                &SpawnPlace::Cell(cell(1, 0, 0))
            ),
            (
                &SpawnWho::Population("householders".to_owned()),
                2,
                &SpawnPlace::Area("all".to_owned())
            ),
        ]
    );
    assert_eq!(map.items.len(), 1);
    assert_eq!(map.items[0].source, ItemSource::Table("scraps".to_owned()));
    assert_eq!((map.items[0].at, map.items[0].count), (cell(0, 0, 0), 3));

    assert_eq!(
        map.conduits,
        [
            (cell(1, 0, 0), "cable".to_owned()),
            (cell(2, 0, 0), "cable".to_owned()),
            (cell(0, 2, 0), "pipe".to_owned()),
        ]
    );
    assert_eq!(map.links.len(), 1);
    assert_eq!(
        (map.links[0].from.as_str(), map.links[0].to.as_str()),
        ("lever", "gate")
    );
    assert_eq!(map.links[0].resource.as_deref(), Some("control"));
    let devices: Vec<(IVec3, Option<&str>)> = map
        .devices
        .iter()
        .map(|(at, device)| (*at, device.name.as_deref()))
        .collect();
    assert_eq!(
        devices,
        [
            (cell(1, 0, 0), Some("gate")),
            (cell(2, 0, 0), Some("lever"))
        ]
    );
    assert_eq!(
        map.devices[0].1.config,
        [("locked".to_owned(), Value::Bool(true))]
    );
    assert_eq!(map.watched, [cell(1, 0, 0)]);
    assert_eq!(
        map.berths,
        [
            BerthCell {
                at: cell(3, 0, 0),
                name: "bay".to_owned(),
                kind: "parking".to_owned(),
                approach: false
            },
            BerthCell {
                at: cell(3, 1, 0),
                name: "bay".to_owned(),
                kind: String::new(),
                approach: true
            },
        ]
    );
    assert_eq!(
        map.vars,
        [(cell(0, 2, 0), "depth".to_owned(), Value::Int(3))]
    );
    assert_eq!(map.decals, [(cell(0, 2, 0), "stain".to_owned())]);
    assert_eq!(
        map.fields,
        [(cell(0, 2, 0), "smoke".to_owned(), Real::of(0.25))]
    );
    assert_eq!(map.cleared, [(cell(0, 2, 0), ClearedLayer::Items)]);
    assert_eq!(map.patterns.len(), 1);
    assert_eq!(map.patterns[0].0, cell(1, 2, 0));
    assert_eq!(map.patterns[0].1.fixture_tag.as_deref(), Some("lamp"));
    assert!(map.patterns[0].1.any);

    let joints: Vec<(Option<&str>, IVec3, JointDir)> = map
        .joints
        .iter()
        .map(|joint| (joint.id.as_deref(), joint.at, joint.dir))
        .collect();
    assert_eq!(
        joints,
        [
            (
                Some("hatch"),
                cell(3, 0, 0),
                JointDir::Vertical(Vertical::Up)
            ),
            (Some("front"), cell(1, 2, 0), JointDir::Side(Dir8::South)),
        ]
    );
    assert_eq!(map.portals.len(), 2);
    assert_eq!(map.portals[0].to, PortalTo::Mark("post".to_owned()));
    assert_eq!(map.portals[1].to, PortalTo::Entry("cellar_door".to_owned()));
    assert_eq!(map.vehicles.len(), 1);
    assert_eq!(map.vehicles[0].facing, Some(Dir8::North));
}

#[test]
fn everything_a_map_carries_turns_with_it() {
    let maps = build(EVERYTHING);
    let map = resolve(&maps, "everything", SEED);
    // A quarter-turn clockwise of a map 4 wide and 3 deep: the cell
    // [x, y] goes to [2 - y, x].
    let quarter = Turn::from(Rot4::R90);
    let turned = map.turned(quarter, &maps);
    assert_eq!(turned.size, IVec2::new(3, 4));
    assert_eq!(turned.front, Some(Dir8::West));
    assert_eq!(turned.marks["entrance"], [cell(2, 1, 0)]);
    assert_eq!(turned.watched, [cell(2, 1, 0)]);
    assert_eq!(turned.items[0].at, cell(2, 0, 0));
    assert_eq!(turned.vehicles[0].at, cell(0, 3, 0));
    assert_eq!(turned.vehicles[0].facing, Some(Dir8::East));
    let front = turned
        .joints
        .iter()
        .find(|joint| joint.id.as_deref() == Some("front"))
        .unwrap();
    assert_eq!(
        (front.at, front.dir),
        (cell(0, 1, 0), JointDir::Side(Dir8::West))
    );
    let hatch = turned
        .joints
        .iter()
        .find(|joint| joint.id.as_deref() == Some("hatch"))
        .unwrap();
    assert_eq!(hatch.dir, JointDir::Vertical(Vertical::Up));
    assert!(turned.areas.contains(&clash_gen::map::ResolvedArea {
        shape: MapShape::Box {
            min: cell(0, 0, 0),
            max: cell(2, 3, 0)
        },
        ..map.areas[0].clone()
    }));
    assert_eq!(
        turned
            .berths
            .iter()
            .map(|berth| berth.at)
            .collect::<Vec<_>>(),
        [cell(1, 3, 0), cell(2, 3, 0)]
    );
    // Asking for the map turned is the same as turning it afterwards.
    let asked = maps
        .resolve(id(&maps, "everything"), &no_params(), SEED, quarter)
        .unwrap()
        .unwrap();
    assert_eq!(asked, turned);
}

#[test]
fn a_state_with_a_facing_turns_and_any_other_state_does_not() {
    let maps = build(
        r##"[
  { "type": "map", "id": "compass", "size": [3, 1],
    "palette": [ { "a": { "fixture": "arrow#east" }, "l": { "fixture": "lamp#lit_east" },
                   "d": "door#open" } ],
    "levels": { "0": [ "ald" ] } },
  { "type": "map", "id": "fixed", "size": [1, 1], "turn": false,
    "palette": [ { "a": { "fixture": "arrow#east" } } ],
    "levels": { "0": [ "a" ] } }
]"##,
    );
    let compass = resolve(&maps, "compass", SEED);
    let arrow_after = |turn: Turn| {
        let turned = compass.turned(turn, &maps);
        let arrow = turned.levels[0]
            .cells
            .iter()
            .filter_map(|cell| cell.fixture)
            .find(|state| maps.world().fixtures().name(state.def()) == "t:arrow")
            .unwrap();
        maps.world()
            .state_text(clash_world::Layer::Fixture, arrow.raw())
    };
    assert_eq!(arrow_after(Turn::IDENTITY), "t:arrow@t:oak#east");
    assert_eq!(arrow_after(Turn::from(Rot4::R90)), "t:arrow@t:oak#south");
    assert_eq!(arrow_after(Turn::from(Rot4::R180)), "t:arrow@t:oak#west");
    assert_eq!(arrow_after(Turn::from(Rot4::R270)), "t:arrow@t:oak#north");
    // A mirror swaps east and west and leaves north and south.
    assert_eq!(arrow_after(Turn::new(Rot4::R0, true)), "t:arrow@t:oak#west");
    assert_eq!(
        arrow_after(Turn::new(Rot4::R90, true)),
        "t:arrow@t:oak#north"
    );

    // The lamp names two directions and has no state for the other
    // two, so it has no facing; a door's `open` names none.
    let lamp = fixture(&maps, "lamp#lit_east");
    let door = block(&maps, "door#open");
    for turn in Turn::ALL {
        let turned = compass.turned(turn, &maps);
        let cells = &turned.levels[0].cells;
        assert!(
            cells.iter().any(|cell| cell.fixture == Some(lamp)),
            "{turn:?}"
        );
        assert!(
            cells.iter().any(|cell| cell.block == Some(door)),
            "{turn:?}"
        );
    }

    // A map that says `"turn": false` is not turned.
    let fixed = id(&maps, "fixed");
    assert_eq!(
        maps.resolve(fixed, &no_params(), SEED, Turn::from(Rot4::R90))
            .unwrap_err()
            .to_string(),
        "the map `t:fixed` may not be turned"
    );
    assert!(
        maps.resolve(fixed, &no_params(), SEED, Turn::IDENTITY)
            .is_ok()
    );
}

/// A map in which nearly every cell is a matter of chance, drawn by
/// random functions in expressions as well as by weights.
const DICE: &str = r##"[
  { "type": "map", "id": "crumb", "size": [1, 1],
    "params": { "n": { "type": "int" } },
    "palette": [ { "c": { "items": { "item": "crumb", "count": "$n" } } } ],
    "levels": { "0": [ "c" ] } },
  { "type": "map", "id": "dice", "size": [6, 6],
    "params": { "roll": "rand_int(1, 1000000)", "luck": { "range": [0.2, 0.8] } },
    "palette": [ { "r": { "fixture": "table",
                          "items": { "item": "coin", "chance": "rand(0, 1)",
                                     "count": "rand_int(1, 9) + $roll % 5" },
                          "spawn": { "creature": "rat", "chance": "$luck" } },
                   "f": [ { "fixture": "table" }, { "fixture": "mat" }, { "fixture": "arrow#north" }, {} ],
                   ".": { "area": "room" } } ],
    "default": { "floor": "boards" },
    "levels": { "0": [ "rrrrrr", "ffffff", "rfrfrf", "......", "frfrfr", "rrffrr" ] },
    "place": [ { "map": "crumb", "in": "room", "count": [1, 4],
                 "params": { "n": "pick([1, 2, 3, 4, 5, 6, 7, 8])" } } ] }
]"##;

#[test]
fn the_same_seed_gives_the_same_map_and_another_seed_another() {
    let maps = build(DICE);
    let dice = id(&maps, "dice");
    let mut seen = Vec::new();
    for seed in 0..12 {
        let once = maps
            .resolve(dice, &no_params(), seed, Turn::IDENTITY)
            .unwrap()
            .unwrap();
        let again = maps
            .resolve(dice, &no_params(), seed, Turn::IDENTITY)
            .unwrap()
            .unwrap();
        assert_eq!(once, again, "seed {seed}");
        assert!(!seen.contains(&once), "seed {seed} repeats an earlier map");
        seen.push(once);
    }
    // The compiled maps are shared: resolving on another thread, in
    // another order, gives the same maps.
    let shared = std::sync::Arc::clone(&maps);
    let elsewhere = std::thread::spawn(move || {
        (0..12)
            .rev()
            .map(|seed| {
                shared
                    .resolve(dice, &VarMap::default(), seed, Turn::IDENTITY)
                    .unwrap()
                    .unwrap()
            })
            .collect::<Vec<_>>()
    })
    .join()
    .unwrap();
    assert!(elsewhere.iter().rev().eq(seen.iter()));
}

#[test]
fn the_order_cells_are_visited_in_decides_nothing() {
    let maps = build(DICE);
    let dice = id(&maps, "dice");
    for seed in 0..8 {
        for turn in [Turn::IDENTITY, Turn::new(Rot4::R270, true)] {
            let forward = maps.resolve(dice, &no_params(), seed, turn).unwrap();
            for order in [
                EvalOrder::Reverse,
                EvalOrder::Shuffled(1),
                EvalOrder::Shuffled(0xfeed),
            ] {
                let other = maps
                    .resolve_in_order(dice, &no_params(), seed, turn, order)
                    .unwrap();
                assert_eq!(forward, other, "seed {seed}, {order:?}");
            }
        }
    }
}

#[test]
fn a_random_function_in_a_map_draws_from_the_placement_and_not_from_a_stream() {
    let maps = build(DICE);
    let dice = id(&maps, "dice");
    // The simulation's streams, as a simulation holds them. Resolving
    // takes no generator: there is no way to hand it one of these.
    let mut streams = RngStreams::new(7);
    streams.stream("rules").next_u64();
    streams.stream("combat").next_u64();
    let before = Hash128::of(&streams);

    let first = maps
        .resolve(dice, &no_params(), SEED, Turn::IDENTITY)
        .unwrap()
        .unwrap();
    assert_eq!(
        Hash128::of(&streams),
        before,
        "resolving drew from a stream"
    );

    // Whatever the simulation draws in between, the map is the same:
    // its random functions depend on the seed and the cell alone.
    for _ in 0..100 {
        streams.stream("rules").next_u64();
    }
    let second = maps
        .resolve(dice, &no_params(), SEED, Turn::IDENTITY)
        .unwrap()
        .unwrap();
    assert_eq!(first, second);
    // The random functions did decide something: over the cells of one
    // placement the counts they gave differ, and so do the chances.
    let counts: BTreeSet<u32> = first.items.iter().map(|items| items.count).collect();
    assert!(counts.len() > 2, "{counts:?}");
    let with_coins = first
        .items
        .iter()
        .filter(|items| items.source == ItemSource::Item("coin".to_owned()))
        .count();
    assert!(with_coins > 0 && with_coins < 17, "{with_coins}");
}

/// The characters a generated map is drawn with: a space, and one for
/// each kind of thing a cell spec can say.
const PALETTE_CHARS: [char; 9] = [' ', '.', 'W', 'a', 'f', 'm', 'r', 'k', 'd'];

/// A map of the given size drawn with the characters chosen, with a
/// little of everything else a map carries.
fn generated(width: usize, depth: usize, levels: usize, picks: &[usize]) -> String {
    let mut storeys = Vec::new();
    for z in 0..levels {
        let rows: Vec<String> = (0..depth)
            .map(|y| {
                let row: String = (0..width)
                    .map(|x| {
                        let pick = picks[(z * depth + y) * width + x];
                        PALETTE_CHARS[pick % PALETTE_CHARS.len()]
                    })
                    .collect();
                format!("\"{row}\"")
            })
            .collect();
        storeys.push(format!("\"{z}\": [ {} ]", rows.join(", ")));
    }
    let (east, south) = (width - 1, depth - 1);
    format!(
        r##"[
  {{ "type": "map", "id": "random", "size": [{width}, {depth}], "front": "east",
    "palette": [ {{ ".": {{}}, "W": "wall",
      "a": {{ "fixture": "arrow#north" }},
      "f": [ {{ "fixture": "table", "weight": 2 }}, {{ "fixture": "arrow#east" }}, {{}} ],
      "m": {{ "mark": "spot", "area": "room" }},
      "r": {{ "fixture": "table",
             "items": {{ "item": "coin", "count": [0, 3], "chance": 0.5 }},
             "spawn": {{ "creature": "rat", "chance": "rand(0, 1)" }} }},
      "k": {{ "keep": ["floor"], "block": "door#open" }},
      "d": {{ "device": "lever", "watched": true, "berth": "bay", "var": {{ "n": 1 }},
             "decal": "stain", "conduits": ["wire"], "field": {{ "smoke": 0.5 }} }} }} ],
    "default": {{ "floor": "boards" }},
    "levels": {{ {} }},
    "marks": {{ "origin": [0, 0, 0] }},
    "anchor": [{east}, {south}, 0],
    "areas": [ {{ "kind": "room", "name": "whole", "box": [[0, 0], [{east}, {south}]] }} ],
    "joints": [ {{ "at": [0, 0, 0], "dir": "north", "kind": "doorway" }},
                {{ "at": [{east}, {south}, 0], "dir": "east", "kind": "doorway" }} ],
    "vehicles": [ {{ "vehicle": "cart", "at": [{east}, 0, 0], "facing": "north_east" }} ] }}
]"##,
        storeys.join(", ")
    )
}

proptest! {
    #![proptest_config(ProptestConfig {
        cases: 48,
        failure_persistence: None,
        ..ProptestConfig::default()
    })]

    /// Over random small maps: four quarter-turns and two mirrors give
    /// the map back, turning composes, asking for a turn is turning
    /// afterwards, and the order of evaluation decides nothing.
    #[test]
    fn turning_and_resolving_keep_their_laws(
        width in 1_usize..6,
        depth in 1_usize..6,
        levels in 1_usize..3,
        picks in proptest::collection::vec(0_usize..PALETTE_CHARS.len(), 75),
        seed in any::<u64>(),
        first in 0_usize..8,
        second in 0_usize..8,
        shuffle in any::<u64>(),
    ) {
        let maps = build(&generated(width, depth, levels, &picks));
        let random = id(&maps, "random");
        let map = maps.resolve(random, &no_params(), seed, Turn::IDENTITY).unwrap().unwrap();

        // Turned four times it is itself.
        let quarter = Turn::from(Rot4::R90);
        let mut turned = map.clone();
        for _ in 0..4 {
            turned = turned.turned(quarter, &maps);
        }
        prop_assert_eq!(&turned, &map);
        // Mirrored twice it is itself.
        let mirror = Turn::new(Rot4::R0, true);
        prop_assert_eq!(&map.turned(mirror, &maps).turned(mirror, &maps), &map);
        // Not turned, it is itself.
        prop_assert_eq!(&map.turned(Turn::IDENTITY, &maps), &map);

        // One turn after another is the two together, and a turn and
        // its inverse undo each other.
        let (a, b) = (Turn::ALL[first], Turn::ALL[second]);
        let one_by_one = map.turned(a, &maps).turned(b, &maps);
        prop_assert_eq!(&one_by_one, &map.turned(a.then(b), &maps));
        prop_assert_eq!(&map.turned(a, &maps).turned(a.inverse(), &maps), &map);

        // Asking for the map turned is turning it afterwards.
        let asked = maps.resolve(random, &no_params(), seed, a).unwrap().unwrap();
        prop_assert_eq!(&asked, &map.turned(a, &maps));

        // The same seed, the same map, in every order of evaluation.
        for order in [EvalOrder::Forward, EvalOrder::Reverse, EvalOrder::Shuffled(shuffle)] {
            let again = maps
                .resolve_in_order(random, &no_params(), seed, Turn::IDENTITY, order)
                .unwrap()
                .unwrap();
            prop_assert_eq!(&again, &map);
        }

        // What a turn keeps: how many cells are drawn, and how much of
        // everything is carried.
        let drawn = |map: &ResolvedMap| {
            map.levels
                .iter()
                .flat_map(|level| level.cells.iter())
                .filter(|cell| !cell.is_leave())
                .count()
        };
        let other = map.turned(a, &maps);
        prop_assert_eq!(drawn(&other), drawn(&map));
        prop_assert_eq!(other.items.len(), map.items.len());
        prop_assert_eq!(other.spawns.len(), map.spawns.len());
        prop_assert_eq!(other.areas.len(), map.areas.len());
        prop_assert_eq!(other.marks.len(), map.marks.len());
    }
}
