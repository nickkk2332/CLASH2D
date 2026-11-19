//! [`SaveMeta`]: what a save says about itself, read without loading it.

use clash_core::{Hash128, Tick};
use clash_data::Defs;
use serde::{Deserialize, Serialize};

/// The version of the save format this build writes and reads: which
/// sections a save holds and what its metadata is. Each section's blob
/// has a version of its own beside this (01 §11.1).
pub const FORMAT_VERSION: u32 = 1;

/// One mod a save was made with.
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub struct ModStamp {
    /// The mod's id.
    pub id: String,
    /// The mod's version, as its manifest writes it.
    pub version: String,
}

/// What a save says about itself (01 §11): enough for a load screen to
/// list it, and for a loader to say what has changed since it was made.
///
/// Whoever saves fills in what only they know with [`SaveMeta::new`] and
/// the builder functions: a name, a summary, the seed, and bytes of
/// their own. [`save_game`](crate::save_game) fills in the rest from
/// the simulation: the profile and its mods, the fingerprint of the
/// definitions, the tick, the experimental features in use and the
/// state hash.
///
/// ```
/// use clash_save::SaveMeta;
///
/// let meta = SaveMeta::new("The long night").summary("Day 3, the mill").seed(7);
/// assert_eq!(meta.name, "The long night");
/// assert_eq!(meta.seed, 7);
/// ```
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub struct SaveMeta {
    /// The version of the save format. Filled in by saving.
    pub format: u32,
    /// The name shown on a load screen.
    pub name: String,
    /// A line or two shown beside the name: where and when.
    pub summary: String,
    /// The id of the profile the game was started from; empty if the
    /// definitions were not loaded through a profile. Filled in by
    /// saving.
    pub profile: String,
    /// The mods the definitions came from, in load order. Filled in by
    /// saving.
    pub mods: Vec<ModStamp>,
    /// The content fingerprint of the definitions (04 §9). Filled in by
    /// saving.
    pub fingerprint: Hash128,
    /// The seed the simulation was started with.
    pub seed: u64,
    /// The universal time of the save. Filled in by saving.
    pub tick: Tick,
    /// The experimental features the mods opted in to (04 §8.4). Filled
    /// in by saving.
    pub experimental: Vec<String>,
    /// The simulation's state hash when it was saved. Filled in by
    /// saving; `save verify` compares it with what loads.
    pub state_hash: Hash128,
    /// Bytes of whoever saved: what the application or a tool keeps
    /// beside the simulation, such as where its camera was. The save
    /// neither reads nor drops them.
    pub app: Vec<u8>,
}

impl SaveMeta {
    /// Metadata with a name and nothing else said.
    pub fn new(name: impl Into<String>) -> SaveMeta {
        SaveMeta {
            format: FORMAT_VERSION,
            name: name.into(),
            summary: String::new(),
            profile: String::new(),
            mods: Vec::new(),
            fingerprint: Hash128::default(),
            seed: 0,
            tick: Tick::ZERO,
            experimental: Vec::new(),
            state_hash: Hash128::default(),
            app: Vec::new(),
        }
    }

    /// This metadata with a summary.
    #[must_use]
    pub fn summary(mut self, summary: impl Into<String>) -> SaveMeta {
        self.summary = summary.into();
        self
    }

    /// This metadata with the simulation's seed.
    #[must_use]
    pub fn seed(mut self, seed: u64) -> SaveMeta {
        self.seed = seed;
        self
    }

    /// This metadata with bytes of the saver's own.
    #[must_use]
    pub fn app(mut self, bytes: Vec<u8>) -> SaveMeta {
        self.app = bytes;
        self
    }

    /// Fills in what the definitions say: the profile, the mods with
    /// their versions, the fingerprint and the experimental features.
    pub(crate) fn stamp_defs(&mut self, defs: &Defs) {
        self.profile = defs
            .profile()
            .map(|profile| profile.id.clone())
            .unwrap_or_default();
        self.mods = mod_stamps(defs);
        self.fingerprint = defs.fingerprint();
        self.experimental = defs.experimental_features().to_vec();
    }

    /// What differs between the definitions this save was made with and
    /// the ones given: mods that are missing, new or at another
    /// version, and a fingerprint that no longer matches. Empty when
    /// nothing differs.
    pub fn differences(&self, defs: &Defs) -> Vec<String> {
        let mut found = Vec::new();
        let now = mod_stamps(defs);
        for saved in &self.mods {
            match now.iter().find(|have| have.id == saved.id) {
                None => found.push(format!(
                    "the save was made with the mod `{}` {}, which is not loaded",
                    saved.id, saved.version
                )),
                Some(have) if have.version != saved.version => found.push(format!(
                    "the mod `{}` was at version {} when the save was made and is at {} now",
                    saved.id, saved.version, have.version
                )),
                Some(_) => {}
            }
        }
        for have in &now {
            if !self.mods.iter().any(|saved| saved.id == have.id) {
                found.push(format!(
                    "the mod `{}` {} is loaded and the save was made without it",
                    have.id, have.version
                ));
            }
        }
        if found.is_empty() && self.fingerprint != defs.fingerprint() {
            found.push(format!(
                "the definitions have changed since the save was made (fingerprint {} then, {} now)",
                self.fingerprint,
                defs.fingerprint()
            ));
        }
        found
    }
}

/// The mods of a load, with their versions, in load order.
fn mod_stamps(defs: &Defs) -> Vec<ModStamp> {
    defs.mods()
        .iter()
        .map(|loaded| ModStamp {
            id: loaded.manifest.id.clone(),
            version: loaded.manifest.version.to_string(),
        })
        .collect()
}
