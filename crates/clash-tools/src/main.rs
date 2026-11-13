//! The CLASH2D command-line tools: the mod linter, the schema dump, the
//! headless runner for test scripts with golden hashes, the world viewer
//! and the benchmarks. This crate is outside the simulation and may read
//! the wall clock. Specified in `docs/14-tooling.md`.
//!
//! Built so far: the data commands of `docs/04-data-and-modding.md`
//! section 13 (`lint`, `explain`, `show`), in [`data`], with the
//! definition types of the world and of the interface registered;
//! `scenario run`, which runs test scripts against the simulation in
//! the four modes ([`scenario`]); `save inspect`, `save verify` and
//! `save list` ([`save`]); `replay` between two saves ([`replay`]); and
//! the terminal world viewer, `worldview` ([`worldview`]); and the
//! benchmarks against the budgets of the specifications, `bench`
//! ([`bench`](mod@bench)). `save`, `replay` and `worldview` drive the console's
//! host of `clash-save`. `schema` answers that
//! it is not built yet. The rest is filled in by
//! `docs/plans/tooling.md`.

use std::process::ExitCode;

use clash_data::DefTypes;

mod bench;
mod data;
mod reference;
mod refs;
mod replay;
mod save;
mod scaffold;
mod scenario;
mod schema;
mod typedocs;
mod worldview;

/// Every definition type the engine knows.
///
/// Each crate that owns definition types offers one function that
/// registers them; it is called here, so that the tools load mods with
/// exactly the types the game does. A crate that gains definition types
/// adds its line here.
pub fn engine_types() -> DefTypes {
    clash_scene::engine_types()
}

/// What each crate checks once the definitions have loaded: the mistakes
/// that only show when it builds its own tables from them (04 §8.1, the
/// build stage). `lint` runs them all. A crate that builds something
/// from definitions adds its check here.
pub const ENGINE_CHECKS: &[data::BuildCheck] = &[
    |defs| clash_world::check_defs(defs),
    |defs| clash_gen::check_defs(defs),
    clash_scene::check_defs,
];

fn usage() -> String {
    format!(
        "usage: clash-tools <command> [arguments]\n\n{}\n{}\n{}\n{}\n{}\n{}",
        data::USAGE,
        scenario::USAGE,
        save::USAGE,
        replay::USAGE,
        worldview::USAGE,
        bench::USAGE
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
    let own = match command.as_str() {
        "scenario" => Some(scenario::run(rest)),
        "save" => Some(save::run(rest)),
        "worldview" => Some(worldview::run(rest)),
        "replay" => Some(replay::run(rest)),
        "bench" => Some(bench::run(rest)),
        _ => None,
    };
    if let Some(output) = own {
        print!("{}", output.out);
        eprint!("{}", output.err);
        return ExitCode::from(output.code);
    }
    match data::run(&engine_types(), ENGINE_CHECKS, command, rest) {
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

#[cfg(test)]
mod tests {
    use std::path::PathBuf;
    use std::sync::Arc;

    use clash_data::Defs;
    use clash_scene::UiDefs;
    use clash_time::Timeline;
    use clash_world::defs::{BlockDef, RealmDef};
    use clash_world::test_support::load_flat;
    use clash_world::{CellPos, Cosmos, Location, TractPos, WorldDefs};

    use super::*;

    /// The repository's `mods` directory.
    fn mods_dir() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../mods")
    }

    /// Loads the default profile of the repository, as the game will.
    fn default_profile() -> Arc<Defs> {
        let types = engine_types();
        let outcome = clash_scene::engine_loader(&types)
            .search_path(mods_dir())
            .load_profile("default");
        assert!(
            outcome.diagnostics.is_empty(),
            "the default profile has something to say:\n{}",
            outcome.diagnostics.render()
        );
        outcome.defs.expect("the default profile loads")
    }

    #[test]
    fn from_the_default_profile_to_a_cell_of_a_world() {
        let defs = default_profile();
        let mods: Vec<&str> = defs
            .mods()
            .iter()
            .map(|loaded| loaded.manifest.id.as_str())
            .collect();
        assert_eq!(mods, ["core", "ui", "core_ui"]);
        assert_eq!(
            defs.profile().map(|profile| profile.id.as_str()),
            Some("default")
        );

        // The world's tables, with the loader's dense ids.
        let world = WorldDefs::from_defs(&defs).unwrap_or_else(|diagnostics| {
            panic!("the world's tables do not build:\n{}", diagnostics.render())
        });
        let wall = defs.lookup::<BlockDef>("core:wall").expect("a wall");
        assert_eq!(world.blocks().lookup("core:wall"), Some(wall));

        // A realm made from a loaded definition, ground loaded into it,
        // and a cell read back.
        let mut timeline = Timeline::new();
        let mut cosmos = Cosmos::new(Arc::clone(&world));
        let realm_def = defs.lookup::<RealmDef>("core:testland").expect("a realm");
        let realm = cosmos
            .create_realm(&mut timeline, realm_def, 1)
            .expect("the realm is made");
        let grid = cosmos.realm(realm).expect("the realm").base_grid;
        let rock = world.block_state("core:solid", None).expect("rock");
        let grass = world.floor_state("core:grass", None).expect("grass");
        load_flat(&mut cosmos, grid, [TractPos::new(0, 0)], rock, grass, 0).expect("ground");
        let below = cosmos
            .cell(Location::new(grid, CellPos::new(5, 5, -1)))
            .expect("a loaded cell");
        assert_eq!(below.block, rock);
        let surface = cosmos
            .cell(Location::new(grid, CellPos::new(5, 5, 0)))
            .expect("a loaded cell");
        assert_eq!(surface.floor, grass);
        assert!(surface.block.is_none());

        // The interface from the same definitions: a theme and a screen.
        let ui = UiDefs::from_defs(&defs).unwrap_or_else(|diagnostics| {
            panic!("the interface does not load:\n{}", diagnostics.render())
        });
        let mut scene = ui
            .scene("ui:default", "ui:default")
            .unwrap_or_else(|diagnostics| panic!("{}", diagnostics.render()));
        let screen = ui
            .screen("ui:gallery", scene.theme())
            .unwrap_or_else(|diagnostics| panic!("{}", diagnostics.render()));
        scene.open(screen);
        assert!(scene.top().is_some());
    }

    #[test]
    fn the_engines_checks_pass_on_the_default_profile() {
        let defs = default_profile();
        for check in ENGINE_CHECKS {
            let diagnostics = check(&defs);
            assert!(diagnostics.is_empty(), "{}", diagnostics.render());
        }
    }

    #[test]
    fn mods_are_checked_against_the_simulations_own_registry() {
        // The bundled content names real kinds and properties, and a
        // variable of its own at the end of a chain.
        let defs = default_profile();
        let registry = defs.expr_registry();
        for kind in ["entity", "loc", "tract", "global"] {
            assert!(registry.kind_id(kind).is_some(), "{kind}");
        }
        assert!(registry.function_id("core:well_trodden").is_some());
        assert!(defs.variables().get("core:visits").is_some());
        let well_trodden = defs
            .functions()
            .iter()
            .find(|(_, function)| function.name == "core:well_trodden")
            .map(|(_, function)| function)
            .expect("the function is loaded");
        assert!(well_trodden.is_compiled());
        assert!(!well_trodden.belief_safe, "it reads truth about someone");

        // A property that does not exist is a located load error.
        let types = engine_types();
        let files = [(
            "data/broken.json",
            r#"[ { "type": "function", "id": "lost", "params": { "who": "entity" },
                   "returns": "bool", "value": "$who.locc.standable" } ]"#,
        )];
        let outcome = clash_scene::engine_loader(&types)
            .memory_mod("broken", r#"{ "id": "broken" }"#, &files)
            .load_mods(&["broken"]);
        assert!(outcome.has_errors());
        let rendered = outcome.diagnostics.render();
        assert!(rendered.contains("locc"), "{rendered}");
        assert!(rendered.contains("data/broken.json"), "{rendered}");
    }

    #[test]
    fn every_engine_type_has_documentation() {
        for info in engine_types().infos() {
            assert!(!info.doc.is_empty(), "`{}` has no documentation", info.name);
        }
    }
}
