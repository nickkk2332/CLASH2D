//! The maps bundled with the `core` mod: that they build, that they
//! resolve under every turn, and what they look like.
//!
//! The picture of each map at a fixed seed is kept as text in
//! `tests/golden/maps.txt`. Run the test with `BLESS=1` to rewrite it,
//! look at the difference, and say why in the change's description.

use std::fmt::Write as _;
use std::path::PathBuf;
use std::sync::Arc;

use clash_core::{Rot4, Turn, VarMap};
use clash_data::{DefTypes, Defs, Loader};
use clash_gen::map::{MapDefs, MapId};
use clash_world::WorldDefs;

/// The mod whose maps these are.
const MOD: &str = "core";

/// The seed the stored pictures are made with.
const GOLDEN_SEED: u64 = 7;

/// The environment variable that rewrites the stored pictures.
const BLESS_VAR: &str = "BLESS";

/// The maps pictured, by their names within the mod.
const PICTURED: [&str; 3] = ["hut", "house", "corner_shop"];

fn repo(path: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(path)
}

fn bundled() -> (Arc<Defs>, Arc<MapDefs>) {
    let mut types = DefTypes::new();
    clash_data::expr::register_defs(&mut types);
    clash_world::register_defs(&mut types);
    clash_gen::register_defs(&mut types);
    let outcome = Loader::new(&types)
        .search_path(repo("mods"))
        .load_mod_with_requirements(MOD);
    assert!(
        outcome.diagnostics.is_empty(),
        "the mod has something to say:\n{}",
        outcome.diagnostics.render()
    );
    let defs = outcome.defs.expect("the mod loads");
    let world = WorldDefs::from_defs(&defs)
        .unwrap_or_else(|diagnostics| panic!("{}", diagnostics.render()));
    let maps = MapDefs::from_defs(&defs, &world)
        .unwrap_or_else(|diagnostics| panic!("{}", diagnostics.render()));
    (defs, maps)
}

fn id(maps: &MapDefs, name: &str) -> MapId {
    maps.lookup(&format!("{MOD}:{name}"))
        .unwrap_or_else(|| panic!("the mod has no map `{name}`"))
}

/// A map as text: its picture, then its marks and areas.
fn picture(maps: &MapDefs, name: &str, seed: u64, turn: Turn) -> String {
    let map = maps
        .resolve(id(maps, name), &VarMap::default(), seed, turn)
        .unwrap_or_else(|error| panic!("{error}"))
        .unwrap_or_else(|| panic!("`{name}` is not placed"));
    let mut out = String::new();
    writeln!(
        out,
        "== {name}, seed {seed}, turned {}{} ==",
        turn.rot.degrees(),
        if turn.mirror { ", mirrored" } else { "" }
    )
    .unwrap();
    out.push_str(&maps.ascii(&map));
    for (mark, cells) in &map.marks {
        let cells: Vec<String> = cells
            .iter()
            .map(|cell| format!("[{}, {}, {}]", cell.x, cell.y, cell.z))
            .collect();
        writeln!(out, "mark {mark}: {}", cells.join(" ")).unwrap();
    }
    for area in &map.areas {
        writeln!(out, "area {}", area.name).unwrap();
    }
    out
}

#[test]
fn the_bundled_maps_look_as_they_did() {
    let (_, maps) = bundled();
    let mut now = String::new();
    for name in PICTURED {
        now.push_str(&picture(&maps, name, GOLDEN_SEED, Turn::IDENTITY));
        now.push('\n');
    }
    // One of them turned and mirrored, so that a change to turning
    // shows here too.
    now.push_str(&picture(
        &maps,
        "house",
        GOLDEN_SEED,
        Turn::new(Rot4::R90, true),
    ));

    let path = repo("crates/clash-gen/tests/golden/maps.txt");
    if std::env::var_os(BLESS_VAR).is_some_and(|value| value == "1") {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, &now).unwrap();
        return;
    }
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
            "the bundled maps differ from {} at line {line}\n  stored: {:?}\n  now:    {:?}\n\
             run with {BLESS_VAR}=1 to accept the change, and say why in the change's \
             description",
            path.display(),
            stored.lines().nth(line - 1).unwrap_or(""),
            now.lines().nth(line - 1).unwrap_or(""),
        );
    }
}

#[test]
fn every_bundled_map_resolves_under_every_turn() {
    let (_, maps) = bundled();
    assert!(maps.len() >= PICTURED.len());
    for (id, map) in maps.iter() {
        for seed in 0..6 {
            let plain = maps
                .resolve(id, &VarMap::default(), seed, Turn::IDENTITY)
                .unwrap_or_else(|error| panic!("{}: {error}", map.name()))
                .unwrap_or_else(|| panic!("{} is not placed", map.name()));
            assert!(
                plain
                    .levels
                    .iter()
                    .flat_map(|level| level.cells.iter())
                    .any(|cell| !cell.is_leave()),
                "{} draws nothing",
                map.name()
            );
            for turn in Turn::ALL {
                let turned = maps
                    .resolve(id, &VarMap::default(), seed, turn)
                    .unwrap_or_else(|error| panic!("{}: {error}", map.name()))
                    .unwrap();
                assert_eq!(turned, plain.turned(turn, &maps), "{} {turn:?}", map.name());
            }
        }
    }
}

#[test]
fn the_shop_places_the_hut_turned_and_hands_it_its_wall() {
    let (_, maps) = bundled();
    let world = maps.world();
    let shop = maps
        .resolve(
            id(&maps, "corner_shop"),
            &VarMap::default(),
            GOLDEN_SEED,
            Turn::IDENTITY,
        )
        .unwrap()
        .unwrap();
    // The hut is 7 by 5; turned a quarter it is 5 by 7, with its
    // north-west corner at [9, 1]. Its door, in the middle of its south
    // side as drawn, is in the middle of its west side.
    let door = shop.cell(9, 4, 0).block.unwrap();
    assert_eq!(world.blocks().name(door.def()), "core:door");
    // The shop gave the hut its own wall, where the hut alone would
    // have picked one.
    let wall = shop.cell(13, 1, 0).block.unwrap();
    assert_eq!(world.blocks().name(wall.def()), "core:wall");
    assert_eq!(world.materials().name(wall.material()), "core:concrete");
    // The hut's bed is one of the shop's marks, and its roof is on the
    // shop's upper storey.
    assert_eq!(shop.marks["bed"].len(), 1);
    assert!(
        shop.cell(11, 4, 1)
            .floor
            .is_some_and(|floor| !floor.is_none())
    );

    // The two-storey house: a flight of stairs below an opening.
    let house = maps
        .resolve(
            id(&maps, "house"),
            &VarMap::default(),
            GOLDEN_SEED,
            Turn::IDENTITY,
        )
        .unwrap()
        .unwrap();
    assert_eq!(house.level_range(), (0, 2));
    let stairs = house.cell(7, 3, 0).fixture.unwrap();
    assert_eq!(world.fixtures().name(stairs.def()), "core:flight");
    let above = house.cell(7, 3, 1);
    assert!(above.floor.is_some_and(|floor| floor.is_none()));
    assert_eq!(house.marks["bed"].len(), 3);
}
