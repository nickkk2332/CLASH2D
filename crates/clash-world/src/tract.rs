//! Tract and region records: everything the engine knows about space
//! that is not loaded (02 §9.1, §9.2).
//!
//! A realm keeps a [`RegionRecord`] for each region that has been
//! surveyed, and a region record holds a [`TractRecord`] for each of its
//! tracts. A tract goes from unsurveyed to surveyed when its region is
//! surveyed, and to realised the first time it is loaded; independently
//! of that it is loaded or not.
//!
//! ```
//! use clash_world::{RegionRecord, RegionSurvey, TractPos, TractState};
//!
//! let region = RegionRecord::new(RegionSurvey::default());
//! let record = region.tract(TractPos::new(3, 70));
//! assert_eq!(record.state, TractState::Surveyed);
//! assert!(record.extra.is_none());
//! ```

use clash_core::{Hash128, Tick, Turn, VarMap, define_id, impl_state_hash};
use clash_core::{StateHash, StateHasher};
use serde::{Deserialize, Serialize};

use crate::defs::{ElementState, Layer};
use crate::dims::REGION_TRACT_COUNT;
use crate::pos::{CellPos, TractPos};
use crate::site::SiteId;

define_id! {
    /// A `tract` definition: what kind of place a tract is (10 §6). The
    /// definition type is document 10's; until it is built the world
    /// carries the bare id, and 0 is a tract with no definition.
    pub struct TractDefId(u16);
    /// A `devastation_kind` definition (02 §17.2). Built with the kinds
    /// of devastation (package W24).
    pub struct DevastationKindId(u16);
    /// A `map` definition (10 §3). The definition type is document 10's.
    pub struct MapId;
    /// A region kind (10 §8.1). The definition type is document 10's.
    pub struct RegionKindId(u16);
    /// A kind of path: road, river, wall (10 §8.5). The definition type
    /// is document 10's.
    pub struct PathKindId(u16);
}

/// A site's position in its region's list of sites.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug, Serialize, Deserialize)]
pub struct SiteIdx(pub u16);

impl StateHash for SiteIdx {
    fn state_hash(&self, hasher: &mut StateHasher) {
        hasher.write_u16(self.0);
    }
}

/// How far along its life a surveyed tract is (02 §9.2).
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TractState {
    /// Its region has been surveyed and it has never been loaded.
    #[default]
    Surveyed,
    /// It has been loaded at least once.
    Realized {
        /// Whether its cells are still exactly what generation gives. A
        /// pristine tract is not stored: it is generated again when it
        /// is next loaded (02 §11).
        pristine: bool,
    },
}

impl TractState {
    /// Whether the tract has ever been loaded.
    pub const fn is_realized(self) -> bool {
        matches!(self, TractState::Realized { .. })
    }

    /// Whether the tract's cells are kept in the tract store.
    pub const fn is_stored(self) -> bool {
        matches!(self, TractState::Realized { pristine: false })
    }
}

impl StateHash for TractState {
    fn state_hash(&self, hasher: &mut StateHasher) {
        let code: u8 = match self {
            TractState::Surveyed => 0,
            TractState::Realized { pristine: true } => 1,
            TractState::Realized { pristine: false } => 2,
        };
        hasher.write_u8(code);
    }
}

/// A recorded energy release waiting to be applied to unloaded tracts
/// (02 §17.1).
///
/// A typed placeholder: the release it carries joins it with the coarse
/// layer (package W17). The world stores stamps and does not apply them.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub struct Stamp {
    /// When the release happened.
    pub tick: Tick,
    /// Its place in the order of everything recorded, cosmos-wide.
    pub seq: u64,
}

impl_state_hash!(Stamp { tick, seq });

/// One explicit change to one element of a cell, waiting for its tract
/// to load.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub struct CellEdit {
    /// The cell, in its realm's base grid.
    pub cell: CellPos,
    /// Which of the cell's elements.
    pub layer: Layer,
    /// What it becomes.
    pub state: ElementState,
}

impl_state_hash!(CellEdit { cell, layer, state });

/// Something generation, or a deferred change, asks the simulation to
/// put into the world when a tract loads (02 §9.3).
///
/// The world carries it and gives it no meaning: what `what` names and
/// what the parameters say is for the simulation (06 §12).
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub struct SpawnRequest {
    /// What to make: the id of a definition, as written.
    pub what: String,
    /// Where, in the realm's base grid.
    pub at: CellPos,
    /// Whatever else the request says.
    pub params: VarMap,
}

impl_state_hash!(SpawnRequest { what, at, params });

/// A change to an unloaded tract, applied when the tract next loads
/// (02 §17.3).
///
/// Built so far: explicit cell changes, spawn requests and maps. The
/// other kinds of 02 §17.3 (a liquid level, devastation, a rule to run)
/// join it with the packages that give them meaning.
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum PendingEdit {
    /// A recorded energy release. Kept in the list until the coarse
    /// layer applies it (package W17).
    Stamp(Stamp),
    /// Explicit small changes, made through `WorldEdit` when the tract
    /// loads.
    Cells(Vec<CellEdit>),
    /// Something for the simulation to put there. Handed over with the
    /// loaded tract.
    Spawn(SpawnRequest),
    /// A map to write into the tract's cells (10 §4.3). The world cannot
    /// resolve a map: the edit stays in the list when the tract loads,
    /// until whoever holds the maps takes it with
    /// [`Cosmos::take_pending_maps`](crate::Cosmos::take_pending_maps)
    /// and applies it.
    Map(PendingMap),
    /// A body that left loaded space and came down here (02 §15.4). Kept
    /// in the list until the coarse layer plays it out (package W17).
    Impact(crate::body::PendingImpact),
}

impl StateHash for PendingEdit {
    fn state_hash(&self, hasher: &mut StateHasher) {
        match self {
            PendingEdit::Stamp(stamp) => {
                hasher.write_u8(0);
                stamp.state_hash(hasher);
            }
            PendingEdit::Cells(cells) => {
                hasher.write_u8(1);
                cells.state_hash(hasher);
            }
            PendingEdit::Spawn(spawn) => {
                hasher.write_u8(2);
                spawn.state_hash(hasher);
            }
            PendingEdit::Map(map) => {
                hasher.write_u8(3);
                map.state_hash(hasher);
            }
            PendingEdit::Impact(impact) => {
                hasher.write_u8(4);
                impact.state_hash(hasher);
            }
        }
    }
}

/// What few tracts need to remember, kept off to one side (02 §9.1).
#[derive(Clone, PartialEq, Eq, Debug, Default, Serialize, Deserialize)]
pub struct TractExtra {
    /// Changes waiting for the tract to load, applied in order.
    pub pending: Vec<PendingEdit>,
    /// Stamps up to this sequence number are already applied.
    pub stamp_watermark: u64,
    /// When the tract was last unloaded, in the local time of its
    /// realm's clock.
    pub last_loaded: Tick,
    /// Tract-scope variables.
    pub vars: VarMap,
    /// The fingerprint of the generator's inputs when the tract's cells
    /// were last generated: the definitions' content, the generator and
    /// its parameters, and the realm's seed (02 §11). `None` for a tract
    /// that has never been generated.
    pub generated: Option<Hash128>,
}

impl_state_hash!(TractExtra {
    pending,
    stamp_watermark,
    last_loaded,
    vars,
    generated
});

/// What the engine knows about one tract, loaded or not (02 §9.1).
#[derive(Clone, PartialEq, Eq, Debug, Default, Serialize, Deserialize)]
pub struct TractRecord {
    /// What kind of place this is.
    pub def: TractDefId,
    /// How its definition is turned: quarter-turns.
    pub rot: u8,
    /// Which variant of its definition it is.
    pub variant: u8,
    /// How far along its life it is.
    pub state: TractState,
    /// How devastated it is, 0 to 255 (02 §17.2).
    pub devastation: u8,
    /// The kind of whatever devastated it most.
    pub devastation_kind: Option<DevastationKindId>,
    /// The site it belongs to, as a position in its region's list.
    pub site: Option<SiteIdx>,
    /// What few tracts need to remember.
    pub extra: Option<Box<TractExtra>>,
}

impl Default for TractDefId {
    fn default() -> TractDefId {
        TractDefId::new(0)
    }
}

impl TractRecord {
    /// The record's extra part, made if it has none yet.
    pub fn extra_mut(&mut self) -> &mut TractExtra {
        self.extra.get_or_insert_with(Box::default)
    }

    /// The changes waiting for the tract to load.
    pub fn pending(&self) -> &[PendingEdit] {
        self.extra.as_ref().map_or(&[], |extra| &extra.pending)
    }
}

impl_state_hash!(TractRecord {
    def,
    rot,
    variant,
    state,
    devastation,
    devastation_kind,
    site,
    extra
});

/// What a survey found a region to be: its kind and its field values
/// (10 §8).
///
/// A typed placeholder for document 10's survey. The world stores it.
#[derive(Clone, PartialEq, Eq, Debug, Default, Serialize, Deserialize)]
pub struct RegionSurvey {
    /// The region's kind, if the generator gives regions kinds.
    pub kind: Option<RegionKindId>,
    /// The values of the generator's fields at the region.
    pub fields: VarMap,
}

impl_state_hash!(RegionSurvey { kind, fields });

/// A map put into the world: this map, here, turned this way, with these
/// parameters and this seed (10 §4.1).
///
/// A record. A survey writes placements; realising a tract reads those
/// that overlap it. The world stores them; maps are document 10's.
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub struct Placement {
    /// The map.
    pub map: MapId,
    /// Where the map's north-west corner at level 0 goes.
    pub origin: CellPos,
    /// Quarter-turns and mirror.
    pub turn: Turn,
    /// The map's parameters for this placement.
    pub params: VarMap,
    /// The seed of whatever the map leaves to chance.
    pub seed: u64,
    /// Later placements overwrite earlier where they overlap.
    pub order: u16,
    /// The site it belongs to.
    pub site: Option<SiteId>,
}

impl_state_hash!(Placement {
    map,
    origin,
    turn,
    params,
    seed,
    order,
    site
});

/// How a map is written over what the world holds (10 §4.3).
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MapMode {
    /// Every layer the map states is written, emptying where it states
    /// nothing.
    #[default]
    Replace,
    /// A cell of the map is written only where the world's cell holds
    /// nothing at all: no floor, no block and no fixture.
    OnlyEmpty,
    /// Each element the map states is put only where that layer of the
    /// world's cell is empty. Nothing is ever cleared.
    Add,
}

impl StateHash for MapMode {
    fn state_hash(&self, hasher: &mut StateHasher) {
        hasher.write_u8(match self {
            MapMode::Replace => 0,
            MapMode::OnlyEmpty => 1,
            MapMode::Add => 2,
        });
    }
}

/// A map waiting to be written into a tract that is not loaded
/// (02 §17.3; 10 §4.3): the placement, and how it is written over what
/// is there. A map that covers several tracts waits in each of them, and
/// each takes its own window of it when it loads.
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub struct PendingMap {
    /// The map, where, turned how, with which parameters and seed.
    pub placement: Placement,
    /// How it is written over what is there.
    pub mode: MapMode,
}

impl_state_hash!(PendingMap { placement, mode });

/// A road, river, wall or rail crossing a region (10 §8.3).
///
/// A typed placeholder for document 10's lines. The world stores it.
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub struct PathLine {
    /// What kind of line it is.
    pub kind: PathKindId,
    /// The cells it runs through, in order.
    pub points: Vec<CellPos>,
}

impl_state_hash!(PathLine { kind, points });

/// What the engine knows about one surveyed region (02 §9.1).
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub struct RegionRecord {
    /// The region's kind and field values.
    pub survey: RegionSurvey,
    /// A record for each tract, row by row (see
    /// [`TractPos::index_in_region`]).
    #[serde(with = "tract_table")]
    pub tracts: Box<[TractRecord; REGION_TRACT_COUNT]>,
    /// The maps placed in this region.
    pub placements: Vec<Placement>,
    /// The lines crossing it.
    pub paths: Vec<PathLine>,
    /// Region-wide stamps.
    pub stamps: Vec<Stamp>,
    /// Its sites. A tract's [`SiteIdx`] is a position in this list.
    pub sites: Vec<SiteId>,
    /// Region-scope variables.
    pub vars: VarMap,
}

/// A table of tract records, every one as surveyed and nothing else
/// said.
fn blank_tracts() -> Box<[TractRecord; REGION_TRACT_COUNT]> {
    Box::new(core::array::from_fn(|_| TractRecord::default()))
}

impl RegionRecord {
    /// A surveyed region in which every tract has the default record:
    /// no definition, never loaded, untouched.
    pub fn new(survey: RegionSurvey) -> RegionRecord {
        RegionRecord {
            survey,
            tracts: blank_tracts(),
            placements: Vec::new(),
            paths: Vec::new(),
            stamps: Vec::new(),
            sites: Vec::new(),
            vars: VarMap::new(),
        }
    }

    /// The record of a tract of this region. Only the tract's position
    /// inside the region matters.
    pub fn tract(&self, tract: TractPos) -> &TractRecord {
        &self.tracts[tract.index_in_region()]
    }

    /// The record of a tract of this region, to change.
    pub fn tract_mut(&mut self, tract: TractPos) -> &mut TractRecord {
        &mut self.tracts[tract.index_in_region()]
    }
}

impl StateHash for RegionRecord {
    fn state_hash(&self, hasher: &mut StateHasher) {
        self.survey.state_hash(hasher);
        for record in self.tracts.iter() {
            record.state_hash(hasher);
        }
        self.placements.state_hash(hasher);
        self.paths.state_hash(hasher);
        self.stamps.state_hash(hasher);
        self.sites.state_hash(hasher);
        self.vars.state_hash(hasher);
    }
}

/// A region's table of tract records as a list: `serde` has no arrays of
/// this length.
mod tract_table {
    use serde::de::Error;
    use serde::{Deserialize, Deserializer, Serialize, Serializer};

    use super::TractRecord;
    use crate::dims::REGION_TRACT_COUNT;

    pub(super) fn serialize<S: Serializer>(
        table: &[TractRecord; REGION_TRACT_COUNT],
        serializer: S,
    ) -> Result<S::Ok, S::Error> {
        table.as_slice().serialize(serializer)
    }

    pub(super) fn deserialize<'de, D: Deserializer<'de>>(
        deserializer: D,
    ) -> Result<Box<[TractRecord; REGION_TRACT_COUNT]>, D::Error> {
        let records = Vec::<TractRecord>::deserialize(deserializer)?;
        let found = records.len();
        records.into_boxed_slice().try_into().map_err(|_| {
            D::Error::custom(format!(
                "a region has {REGION_TRACT_COUNT} tract records, but {found} were given"
            ))
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use clash_core::codec;

    #[test]
    fn a_tract_record_is_small() {
        // 02 §9.1: a surveyed region is its tracts' records and little
        // else, so a record must stay a few words.
        assert!(core::mem::size_of::<TractRecord>() <= 24);
    }

    #[test]
    fn a_region_record_round_trips_through_the_codec() {
        let mut region = RegionRecord::new(RegionSurvey::default());
        let at = TractPos::new(5, 9);
        let record = region.tract_mut(at);
        record.state = TractState::Realized { pristine: false };
        record
            .extra_mut()
            .pending
            .push(PendingEdit::Cells(vec![CellEdit {
                cell: CellPos::new(1, 2, 3),
                layer: Layer::Block,
                state: ElementState::NONE,
            }]));
        record.extra_mut().last_loaded = Tick(77);
        let bytes = codec::to_bytes(&region).unwrap();
        let back: RegionRecord = codec::from_bytes(&bytes).unwrap();
        assert_eq!(back, region);
        assert_eq!(Hash128::of(&back), Hash128::of(&region));
        assert_ne!(
            Hash128::of(&region),
            Hash128::of(&RegionRecord::new(RegionSurvey::default()))
        );
        // A table of the wrong length is refused, not indexed.
        let short = codec::to_bytes(&(
            RegionSurvey::default(),
            vec![TractRecord::default(); 3],
            Vec::<Placement>::new(),
            Vec::<PathLine>::new(),
            Vec::<Stamp>::new(),
            Vec::<SiteId>::new(),
            VarMap::new(),
        ))
        .unwrap();
        assert!(codec::from_bytes::<RegionRecord>(&short).is_err());
    }

    #[test]
    fn only_a_changed_tract_is_stored() {
        assert!(!TractState::Surveyed.is_realized());
        assert!(!TractState::Realized { pristine: true }.is_stored());
        assert!(TractState::Realized { pristine: false }.is_stored());
    }
}
