//! What the tests of placing, ground and the `standard` generator
//! share with the `tract_ascii` example: the whole of `mods/core`
//! loaded and built, small worlds written in place, and tracts as text.

#![allow(dead_code, reason = "each test file uses its own part of this")]

use std::sync::Arc;

use clash_data::{Defs, Loader};
use clash_gen::ascii::surface_text;
use clash_gen::{GenDefs, Generators, RealizedTract, realize_tracts};
use clash_world::{Column, RealmDefId, TractPos, WorldDefs};

/// The realm definition of the bundled countryside.
pub const COUNTRYSIDE_REALM: &str = "core:countryside";

/// Everything generation is built from, with the loaded definitions.
pub struct Built {
    /// The loaded definitions.
    pub defs: Arc<Defs>,
    /// The world's tables.
    pub world: Arc<WorldDefs>,
    /// The maps, grounds and tract definitions.
    pub generation: Arc<GenDefs>,
}

impl Built {
    /// The generators, the `standard` one among them, realising on the
    /// calling thread.
    pub fn generators(&self) -> Generators {
        let mut registry = clash_gen::registry_with(&self.generation);
        registry.set_runner(Arc::new(clash_gen::SerialRunner));
        registry.bind(&self.world)
    }

    /// A realm definition by its qualified id.
    pub fn realm(&self, id: &str) -> Result<RealmDefId, String> {
        self.world
            .realms()
            .lookup(id)
            .ok_or_else(|| format!("there is no realm definition `{id}`"))
    }
}

fn built_from(defs: Arc<Defs>) -> Result<Built, String> {
    let world = WorldDefs::from_defs(&defs).map_err(|diagnostics| diagnostics.render())?;
    let generation =
        GenDefs::from_defs(&defs, &world).map_err(|diagnostics| diagnostics.render())?;
    Ok(Built {
        defs,
        world,
        generation,
    })
}

/// The mod `core` of the repository, with the mods it requires, loaded
/// and built. Definitions of types this crate does not know are passed
/// over (decision D-450).
pub fn bundled() -> Result<Built, String> {
    let types = super::bundled_types();
    let defs = Loader::new(&types)
        .search_path(super::mods_dir())
        .only_known_types(true)
        .load_mod_with_requirements("core")
        .into_result()
        .map_err(|diagnostics| diagnostics.render())?;
    built_from(defs)
}

/// The world content of `mods/core` with one more mod laid over it,
/// given as the text of a JSON array: `data/gen.json` of a mod `t`.
pub fn try_with_core(extra: &str) -> Result<Built, String> {
    let types = super::bundled_types();
    let defs = Loader::new(&types)
        .search_path(super::mods_dir())
        .only_known_types(true)
        .memory_mod(
            super::MOD,
            r#"{ "id": "t", "requires": ["core"] }"#,
            &[("data/gen.json", extra)],
        )
        .load_mod_with_requirements(super::MOD)
        .into_result()
        .map_err(|diagnostics| diagnostics.render())?;
    built_from(defs)
}

/// [`try_with_core`] for definitions that must load and build.
pub fn with_core(extra: &str) -> Built {
    try_with_core(extra).unwrap_or_else(|error| panic!("the definitions do not build:\n{error}"))
}

/// A block of tracts `width` by `height` whose north-west tract is
/// `first`, row by row.
pub fn tract_block(first: TractPos, width: i32, height: i32) -> Vec<TractPos> {
    (0..height)
        .flat_map(|y| (0..width).map(move |x| TractPos::new(first.x + x, first.y + y)))
        .collect()
}

/// Realises tracts of a realm definition.
pub fn realise(
    built: &Built,
    realm: &str,
    seed: u64,
    tracts: &[TractPos],
) -> Result<Vec<RealizedTract>, String> {
    let generators = built.generators();
    realize_tracts(&generators, built.realm(realm)?, seed, tracts)
        .map_err(|error| error.to_string())
}

/// A block of tracts of a realm definition as text, seen along the
/// ground: what the `tract_ascii` example prints. `storey` counts up
/// from the surface of each cell column.
pub fn tract_text(
    built: &Built,
    realm: &str,
    seed: u64,
    tracts: &[TractPos],
    storey: i32,
) -> Result<String, String> {
    let realised = realise(built, realm, seed, tracts)?;
    let columns: Vec<(TractPos, &Column)> = tracts
        .iter()
        .copied()
        .zip(realised.iter().map(|tract| &tract.column))
        .collect();
    Ok(surface_text(&built.world, &columns, storey))
}

/// A world with one realm of a realm definition and nothing loaded,
/// with what loading tracts into it needs.
pub struct Live {
    /// The definitions.
    pub built: Built,
    /// The timeline.
    pub timeline: clash_time::Timeline,
    /// The world.
    pub cosmos: clash_world::Cosmos,
    /// Where the world's events go.
    pub events: clash_world::EventSink<clash_world::WorldEvent>,
    /// The realm.
    pub realm: clash_world::RealmId,
    /// Its base grid.
    pub grid: clash_world::GridId,
    /// Where changed tracts are kept.
    pub store: clash_world::MemoryStore,
    /// The generators.
    pub generators: Generators,
}

impl Live {
    /// A world with a realm of this definition and seed.
    pub fn new(built: Built, realm_def: &str, seed: u64) -> Live {
        let mut timeline = clash_time::Timeline::new();
        let mut cosmos = clash_world::Cosmos::new(Arc::clone(&built.world));
        let def = built.realm(realm_def).unwrap();
        let realm = cosmos.create_realm(&mut timeline, def, seed).unwrap();
        let grid = cosmos.realm(realm).unwrap().base_grid;
        let generators = built.generators();
        Live {
            built,
            timeline,
            cosmos,
            events: clash_world::EventSink::new(),
            realm,
            grid,
            store: clash_world::MemoryStore::new(),
            generators,
        }
    }

    /// The context edits are made through.
    pub fn ctx(&mut self) -> clash_world::WorldCtx<'_> {
        clash_world::WorldCtx::new(&mut self.cosmos, &mut self.timeline, &mut self.events)
    }

    /// Loads tracts, which must load.
    pub fn load(&mut self, tracts: &[TractPos]) -> Vec<clash_world::LoadedTract> {
        let mut io = clash_world::TractIo::new(&self.generators, &mut self.store);
        let mut ctx =
            clash_world::WorldCtx::new(&mut self.cosmos, &mut self.timeline, &mut self.events);
        let wanted: Vec<_> = tracts.iter().map(|tract| (self.realm, *tract)).collect();
        ctx.load_tracts(&mut io, &wanted)
            .into_iter()
            .map(|outcome| outcome.unwrap_or_else(|error| panic!("{error}")))
            .collect()
    }

    /// Unloads a tract, which must unload.
    pub fn unload(&mut self, tract: TractPos) {
        let mut io = clash_world::TractIo::new(&self.generators, &mut self.store);
        let mut ctx =
            clash_world::WorldCtx::new(&mut self.cosmos, &mut self.timeline, &mut self.events);
        ctx.unload_tract(&mut io, self.realm, tract)
            .unwrap_or_else(|error| panic!("{error}"));
    }

    /// What a cell of the realm holds.
    pub fn cell(&self, x: i32, y: i32, z: i32) -> clash_world::CellView {
        self.cosmos
            .cell(clash_world::Location::new(
                self.grid,
                clash_world::CellPos::new(x, y, z),
            ))
            .unwrap_or_else(|error| panic!("{error}"))
    }

    /// Everything in a loaded tract that nothing holds up, or that is
    /// crushed by what it carries (02 §14): empty when the tract
    /// stands.
    pub fn unsupported(&self, tract: TractPos) -> Vec<String> {
        use clash_world::dims::{CHUNK_Z_BITS, TRACT_XY};
        let view = self.cosmos.grid(self.grid).unwrap();
        let column = view.column(tract).expect("the tract is loaded");
        let Some((low, high)) = column.stored_range() else {
            return Vec::new();
        };
        let origin = tract.origin(0);
        let mut cells = Vec::new();
        for z in (low << CHUNK_Z_BITS)..((high + 1) << CHUNK_Z_BITS) {
            for y in 0..TRACT_XY {
                for x in 0..TRACT_XY {
                    let cell = clash_world::CellPos::new(origin.x + x, origin.y + y, z);
                    if !column.cell(cell).is_empty() {
                        cells.push(cell);
                    }
                }
            }
        }
        let assessment = self
            .cosmos
            .assess_stability(self.grid, &cells)
            .unwrap_or_else(|error| panic!("{error}"));
        assert!(!assessment.truncated(), "the check ran out of room");
        let mut found: Vec<String> = assessment
            .falling()
            .iter()
            .map(|(cell, layer)| format!("the {} at {cell:?} falls", layer.name()))
            .collect();
        found.extend(
            assessment
                .crushed()
                .iter()
                .map(|cell| format!("the block at {cell:?} is crushed")),
        );
        found
    }
}
