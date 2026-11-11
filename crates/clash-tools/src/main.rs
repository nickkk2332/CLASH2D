//! The CLASH2D command-line tools: the mod linter, the schema dump, the
//! headless runner for test scripts with golden hashes, the world viewer
//! and the benchmarks. This crate is outside the simulation and may read
//! the wall clock. Specified in `docs/14-tooling.md`.
//!
//! Built so far: the data commands of `docs/04-data-and-modding.md`
//! section 13 (`lint`, `explain`, `show`), in [`data`]. `schema` answers
//! that it is not built yet. The rest is filled in by W11 of
//! `docs/plans/world-model.md` and by `docs/plans/tooling.md`.

use std::process::ExitCode;

use clash_data::DefTypes;

mod data;
mod scenario;

/// Every definition type the engine knows.
///
/// Each crate that owns definition types offers one function that
/// registers them; it is called here, so that the tools load mods with
/// exactly the types the game does. None is registered yet: the crates
/// that own them are being built.
pub fn engine_types() -> DefTypes {
    DefTypes::new()
}

fn usage() -> String {
    format!(
        "usage: clash-tools <command> [arguments]\n\n{}\n{}",
        data::USAGE,
        scenario::USAGE
    )
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let Some((command, rest)) = args.split_first() else {
        eprint!("{}", usage());
        return ExitCode::from(data::EXIT_USAGE);
    };
    if matches!(command.as_str(), "help" | "--help" | "-h") {
        print!("{}", usage());
        return ExitCode::SUCCESS;
    }
    if command == "scenario" {
        let output = scenario::run(rest);
        print!("{}", output.out);
        eprint!("{}", output.err);
        return ExitCode::from(output.code);
    }
    match data::run(&engine_types(), command, rest) {
        Some(output) => {
            print!("{}", output.out);
            eprint!("{}", output.err);
            ExitCode::from(output.code)
        }
        None => {
            eprintln!("clash-tools: unknown command `{command}`\n\n{}", usage());
            ExitCode::from(data::EXIT_USAGE)
        }
    }
}
