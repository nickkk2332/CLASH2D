//! Prints a map of the bundled mods, resolved for a seed and a turn.
//!
//! ```text
//! cargo run -p clash-gen --example map_ascii -- <map id> [seed] [turn]
//! ```
//!
//! `map id` is qualified (`core:house`) or bare (`house`). `seed` is a
//! whole number, 1 if left out. `turn` is `0`, `90`, `180` or `270`,
//! with an `m` before it to mirror first (`m90`); not turned if left
//! out. With no arguments the maps are listed.
//!
//! The mods are read from the repository's `mods` directory. This crate
//! sits below the interface, so it loads the mod named by `CLASH_MOD`
//! (`core` if unset) with the mods it requires, not a whole profile: a
//! profile may list mods whose definition types only the crates above
//! know.

use std::process::ExitCode;

use clash_core::{Rot4, Turn, VarMap};
use clash_data::{DefTypes, Loader};
use clash_gen::map::MapDefs;
use clash_world::WorldDefs;

/// The seed used when none is given.
const DEFAULT_SEED: u64 = 1;

/// The mod loaded when `CLASH_MOD` names none.
const DEFAULT_MOD: &str = "core";

/// Degrees in a quarter-turn.
const QUARTER_DEGREES: i32 = 90;

fn parse_turn(text: &str) -> Option<Turn> {
    let (mirror, degrees) = match text.strip_prefix('m') {
        Some(rest) => (true, rest),
        None => (false, text),
    };
    let degrees: i32 = if degrees.is_empty() {
        0
    } else {
        degrees.parse().ok()?
    };
    (degrees % QUARTER_DEGREES == 0)
        .then(|| Turn::new(Rot4::from_quarter_turns(degrees / QUARTER_DEGREES), mirror))
}

fn run() -> Result<(), String> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    // The `function` and `variable` types are left out on purpose: the
    // bundled ones name kinds of subject that only the simulation, which
    // sits above this crate, registers (decision D-450).
    let mut types = DefTypes::new();
    clash_world::register_defs(&mut types);
    clash_gen::register_defs(&mut types);
    let mods = concat!(env!("CARGO_MANIFEST_DIR"), "/../../mods");
    let wanted = std::env::var("CLASH_MOD").unwrap_or_else(|_| DEFAULT_MOD.to_owned());
    let defs = Loader::new(&types)
        .search_path(mods)
        .only_known_types(true)
        .load_mod_with_requirements(&wanted)
        .into_result()
        .map_err(|diagnostics| diagnostics.render())?;
    let world = WorldDefs::from_defs(&defs).map_err(|diagnostics| diagnostics.render())?;
    let maps = MapDefs::from_defs(&defs, &world).map_err(|diagnostics| diagnostics.render())?;

    let Some(wanted) = args.first() else {
        println!("usage: map_ascii <map id> [seed] [turn]\n\nmaps:");
        for (_, map) in maps.iter() {
            let size = map.size();
            let (low, high) = map.level_range();
            println!(
                "  {}  {} by {}, levels {low} to {high}",
                map.name(),
                size.x,
                size.y
            );
        }
        return Ok(());
    };
    let id = maps
        .lookup(wanted)
        .or_else(|| {
            maps.iter()
                .find(|(_, map)| map.name().rsplit(':').next() == Some(wanted.as_str()))
                .map(|(id, _)| id)
        })
        .ok_or_else(|| format!("there is no map `{wanted}`; run with no arguments for a list"))?;
    let seed = match args.get(1) {
        Some(text) => text
            .parse::<u64>()
            .map_err(|_| format!("`{text}` is not a seed: a whole number"))?,
        None => DEFAULT_SEED,
    };
    let turn = match args.get(2) {
        Some(text) => parse_turn(text)
            .ok_or_else(|| format!("`{text}` is not a turn: 0, 90, 180, 270, or m before one"))?,
        None => Turn::IDENTITY,
    };

    let resolved = maps
        .resolve(id, &VarMap::default(), seed, turn)
        .map_err(|error| error.to_string())?;
    let Some(resolved) = resolved else {
        println!(
            "{}: its `if` is false for this seed; it is not placed",
            maps.name(id)
        );
        return Ok(());
    };
    println!(
        "{}  seed {seed}  turned {}{}  {} by {}",
        maps.name(id),
        turn.rot.degrees(),
        if turn.mirror { ", mirrored" } else { "" },
        resolved.size.x,
        resolved.size.y
    );
    print!("{}", maps.ascii(&resolved));
    for (name, cells) in &resolved.marks {
        let cells: Vec<String> = cells
            .iter()
            .map(|cell| format!("[{}, {}, {}]", cell.x, cell.y, cell.z))
            .collect();
        println!("mark {name}: {}", cells.join(" "));
    }
    for area in &resolved.areas {
        println!("area {}", area.name);
    }
    Ok(())
}

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(message) => {
            eprintln!("{message}");
            ExitCode::FAILURE
        }
    }
}
