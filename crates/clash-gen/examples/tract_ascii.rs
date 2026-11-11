//! Prints realised tracts of a realm of the bundled mods as plain text,
//! seen along the ground, so that a person or an agent can look at what
//! the `standard` generator makes.
//!
//! ```bash
//! cargo run -p clash-gen --example tract_ascii -- <realm id> [seed] [tract x] [tract y] [storey] [width] [height]
//! ```
//!
//! `realm id` is qualified (`core:countryside`) or bare. `seed` is the
//! realm's seed (default 1). `tract x` and `tract y` name the
//! north-west tract printed (default 0 and 0), and `width` and `height`
//! how many tracts east and south of it are printed with it (default 2
//! and 1: 64 columns by 32 rows). `storey` counts up from the surface
//! of each cell column (default 0): the ground rolls, so storey 0 is
//! what stands on the ground everywhere, and 1 is the storey over that.
//!
//! Each cell is the character of its topmost element's glyph: a
//! fixture's if it has one, else its block's, else its floor's. With
//! the definitions of `mods/core`: `"` grass, `Y` tree, `*` bush, `o`
//! boulder, `<` ramp, `.` paving and indoor floor, `#` wall, `=`
//! window, `+` door, `~` roofing.
//!
//! With no arguments the realm definitions the generator can make are
//! listed.

#[path = "../tests/common/mod.rs"]
mod common;

use std::process::ExitCode;

use clash_world::TractPos;

/// The seed used when none is given.
const DEFAULT_SEED: u64 = 1;

/// How many tracts wide and deep the block printed is when not said.
const DEFAULT_BLOCK: (i32, i32) = (2, 1);

/// The most tracts along a side of the block printed.
const MAX_BLOCK_SIDE: i32 = 8;

fn number<T: std::str::FromStr>(arg: Option<String>, what: &str, default: T) -> Result<T, String> {
    match arg {
        None => Ok(default),
        Some(text) => text
            .parse()
            .map_err(|_| format!("{what} must be a whole number, and `{text}` is not")),
    }
}

fn run() -> Result<(), String> {
    let mut args = std::env::args().skip(1);
    let built = common::bundled()?;
    let Some(wanted) = args.next() else {
        println!(
            "usage: tract_ascii <realm id> [seed] [tract x] [tract y] [storey] [width] [height]\n\n\
             realms of the `{}` generator:",
            clash_gen::STANDARD_GENERATOR
        );
        let generators = built.generators();
        for (id, _) in built.world.realms().iter() {
            let standard = generators
                .of(id)
                .is_ok_and(|generator| format!("{generator:?}").starts_with("StandardGenerator"));
            if standard {
                println!("  {}", built.world.realms().name(id));
            }
        }
        return Ok(());
    };
    let realm = if wanted.contains(':') {
        wanted
    } else {
        format!("core:{wanted}")
    };
    let seed = number(args.next(), "the seed", DEFAULT_SEED)?;
    let x = number(args.next(), "the tract's x", 0)?;
    let y = number(args.next(), "the tract's y", 0)?;
    let storey = number(args.next(), "the storey", 0)?;
    let width: i32 = number(args.next(), "the width", DEFAULT_BLOCK.0)?;
    let height: i32 = number(args.next(), "the height", DEFAULT_BLOCK.1)?;
    if !(1..=MAX_BLOCK_SIDE).contains(&width) || !(1..=MAX_BLOCK_SIDE).contains(&height) {
        return Err(format!(
            "the width and the height are from 1 to {MAX_BLOCK_SIDE} tracts"
        ));
    }
    let tracts = common::tract_block(TractPos::new(x, y), width, height);
    print!(
        "{}",
        common::tract_text(&built, &realm, seed, &tracts, storey)?
    );
    Ok(())
}

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("{error}");
            ExitCode::FAILURE
        }
    }
}
