//! Where a host's definitions come from: [`HostEnv`], shared by every
//! host a tool makes, so that a profile is read from disk once.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use clash_data::Defs;
use clash_gen::{GenDefs, Generators, RayonRunner};
use clash_world::{SerialRunner, WorldDefs};

/// A profile as a host uses it: the definitions, and what is built from
/// them once.
#[derive(Clone, Debug)]
pub(crate) struct Profile {
    /// The name the profile was asked for by.
    pub name: String,
    /// The loaded definitions.
    pub defs: Arc<Defs>,
    /// The world's tables.
    pub world: Arc<WorldDefs>,
    /// What tracts are generated from.
    pub generation: Arc<GenDefs>,
}

impl Profile {
    /// Builds what a host needs from loaded definitions.
    fn build(name: &str, defs: Arc<Defs>) -> Result<Profile, String> {
        let world = WorldDefs::from_defs(&defs).map_err(|diagnostics| diagnostics.render())?;
        let generation =
            GenDefs::from_defs(&defs, &world).map_err(|diagnostics| diagnostics.render())?;
        Ok(Profile {
            name: name.to_owned(),
            defs,
            world,
            generation,
        })
    }

    /// The generators bound to the profile's definitions, realising
    /// batches of tracts on this many threads: one is the calling
    /// thread, and `None` the process's shared pool. What is generated
    /// does not depend on it (01 R1).
    pub fn generators(&self, threads: Option<u32>) -> Result<Generators, String> {
        let mut registry = clash_gen::registry_with(&self.generation);
        match threads {
            None => {}
            Some(1) => registry.set_runner(Arc::new(SerialRunner)),
            Some(count) => {
                let runner =
                    RayonRunner::with_threads(count as usize).map_err(|error| error.to_string())?;
                registry.set_runner(Arc::new(runner));
            }
        }
        Ok(registry.bind(&self.world))
    }
}

/// Where hosts find their profiles: the directories mods are searched
/// for in, and the profiles already loaded.
///
/// Definitions never change once loaded (04 M4), so every host made
/// over one environment shares them: the test-script runner makes a
/// dozen hosts for each script and reads the mods once.
///
/// ```no_run
/// use std::sync::Arc;
/// use clash_save::host::{GameHost, HostEnv};
///
/// let env = Arc::new(HostEnv::new("mods"));
/// let host = GameHost::new(Arc::clone(&env));
/// # let _ = host;
/// ```
#[derive(Debug, Default)]
pub struct HostEnv {
    search: Vec<PathBuf>,
    loaded: Mutex<BTreeMap<String, Profile>>,
}

impl HostEnv {
    /// An environment that looks for mods in one directory, and for a
    /// profile by name in `profiles/` beside it (04 §3.3).
    pub fn new(mods: impl Into<PathBuf>) -> HostEnv {
        HostEnv {
            search: vec![mods.into()],
            loaded: Mutex::new(BTreeMap::new()),
        }
    }

    /// This environment, looking for mods in one more directory.
    #[must_use]
    pub fn search_path(mut self, dir: impl Into<PathBuf>) -> HostEnv {
        self.search.push(dir.into());
        self
    }

    /// The directories mods are searched for in.
    pub fn search_paths(&self) -> &[PathBuf] {
        &self.search
    }

    /// Gives the environment definitions that are already loaded, under
    /// the name `use` will ask for them by: for an application that has
    /// loaded its profile itself, and for tests with mods in memory.
    /// Fails, with the diagnostics, if the world's tables or the
    /// generation definitions do not build from them.
    pub fn provide(&self, name: &str, defs: Arc<Defs>) -> Result<(), String> {
        let profile = Profile::build(name, defs)?;
        if let Ok(mut loaded) = self.loaded.lock() {
            loaded.insert(name.to_owned(), profile);
        }
        Ok(())
    }

    /// The profile of a name: loaded already, or loaded now from the
    /// search directories. A name that is a file is read as a profile
    /// file. The error is the rendered diagnostics.
    pub(crate) fn profile(&self, name: &str) -> Result<Profile, String> {
        if let Ok(loaded) = self.loaded.lock()
            && let Some(profile) = loaded.get(name)
        {
            return Ok(profile.clone());
        }
        let types = clash_scene::engine_types();
        let mut loader = clash_scene::engine_loader(&types);
        for dir in &self.search {
            loader = loader.search_path(dir.clone());
        }
        let file = Path::new(name);
        let outcome = if file.is_file() {
            loader.load_profile_file(file)
        } else {
            loader.load_profile(name)
        };
        let defs = outcome
            .into_result()
            .map_err(|diagnostics| diagnostics.render())?;
        let profile = Profile::build(name, defs)?;
        if let Ok(mut loaded) = self.loaded.lock() {
            loaded.insert(name.to_owned(), profile.clone());
        }
        Ok(profile)
    }
}
