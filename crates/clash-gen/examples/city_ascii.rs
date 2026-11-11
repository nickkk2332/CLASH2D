//! Prints one storey of a block of the `test_city` generator as plain
//! text, so that a person or an agent can see the city.
//!
//! ```bash
//! cargo run -p clash-gen --example city_ascii -- [seed] [storey]
//! ```
//!
//! `seed` is the realm's seed (default 1) and `storey` counts up from
//! the ground storey (default 0). The block is two tracts by two, 64
//! cells on a side, of the realm definition `core:test_city`. Each cell
//! is the character of its topmost element's glyph: a fixture's if it
//! has one, else its block's, else its floor's. With the definitions of
//! `mods/core`: `.` street or indoor floor, `,` sidewalk, `#` wall, `=`
//! window, `+` door, `O` pillar, `>` stairs, `n` desk, `_` roof.

#[path = "../tests/common/mod.rs"]
mod common;

use std::process::ExitCode;

/// The seed used when none is given.
const DEFAULT_SEED: u64 = 1;

fn main() -> ExitCode {
    let mut args = std::env::args().skip(1);
    let seed = match args.next().map(|text| text.parse::<u64>()) {
        None => DEFAULT_SEED,
        Some(Ok(seed)) => seed,
        Some(Err(error)) => {
            eprintln!("the seed must be a whole number: {error}");
            return ExitCode::FAILURE;
        }
    };
    let storey = match args.next().map(|text| text.parse::<i32>()) {
        None => 0,
        Some(Ok(storey)) => storey,
        Some(Err(error)) => {
            eprintln!("the storey must be a whole number: {error}");
            return ExitCode::FAILURE;
        }
    };
    match common::city_text(seed, storey) {
        Ok(text) => {
            print!("{text}");
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("{error}");
            ExitCode::FAILURE
        }
    }
}
