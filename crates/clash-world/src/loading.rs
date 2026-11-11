//! Loading and unloading tracts (02 §9.4, §9.5).
//!
//! A tract is loaded at a safe point, by [`WorldCtx::load_tracts`]:
//!
//! 1. Its region is surveyed if it has not been, and so are the regions
//!    of its neighbours, whose records its realiser is shown.
//! 2. If the tract has been changed since it was generated, its column
//!    is read from the [`TractStore`]. Otherwise its generator realises
//!    it: a tract that was never changed is not stored (02 §11).
//! 3. The changes that were waiting for it are applied, in order,
//!    through `WorldEdit`.
//! 4. `TractLoaded` is emitted, and what the simulation is to put there
//!    is handed back.
//!
//! Realising is the costly step and is pure, so a batch of tracts is
//! realised together, in parallel if the [`Generators`] were given a
//! runner that does that, and then installed one by one in the order
//! asked. The world that results is the same for every order and every
//! thread count.
//!
//! Unloading ([`WorldCtx::unload_tract`]) is refused while anyone is in
//! the tract. Otherwise `TractUnloading` is emitted, a column that has
//! changed is written to the store, and the tract's record notes when.
//!
//! Not built yet, each with the package that owns it: stamps and total
//! devastation (W17), the catching up of fields (W16), and the checks
//! that no body or process is left in a tract (W14, W13).

use std::collections::BTreeSet;
use std::sync::Arc;

use clash_core::{EntityId, IVec2, Tick};
use thiserror::Error;

use crate::area::AreaId;
use crate::cause::{Cause, CauseKind};
use crate::cosmos::{Cosmos, RealmId};
use crate::defs::WorldDefs;
use crate::edit::WorldCtx;
use crate::error::WorldError;
use crate::event::WorldEvent;
use crate::generate::{
    GenCtx, GenError, Generators, RealizeJob, RealizedTract, RealmInfo, RegionSurveyResult,
    TractRecords3x3, realize_batch,
};
use crate::grid::Column;
use crate::pos::{GridId, Location, RegionPos, TractPos};
use crate::store::{StoreError, TractStore, decode_column, encode_column};
use crate::topology::{Edge, Topology};
use crate::tract::{
    CellEdit, PendingEdit, RegionRecord, SiteIdx, SpawnRequest, TractRecord, TractState,
};

/// What loading and unloading need besides the world: the generators,
/// and where changed tracts are kept.
pub struct TractIo<'a> {
    /// The generator of each realm definition.
    pub generators: &'a Generators,
    /// Where changed tracts are kept while they are not loaded.
    pub store: &'a mut dyn TractStore,
}

impl<'a> TractIo<'a> {
    /// The two together.
    pub fn new(generators: &'a Generators, store: &'a mut dyn TractStore) -> TractIo<'a> {
        TractIo { generators, store }
    }
}

impl core::fmt::Debug for TractIo<'_> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("TractIo")
            .field("generators", &self.generators)
            .finish_non_exhaustive()
    }
}

/// Why a tract could not be loaded or unloaded. Nothing was changed.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[non_exhaustive]
pub enum LoadError {
    /// The world refused: an unknown realm, a tract outside it.
    #[error(transparent)]
    World(#[from] WorldError),
    /// The realm has no generator that can be used.
    #[error(transparent)]
    Gen(#[from] GenError),
    /// The tract store failed, or what it holds for the tract is damaged.
    #[error(transparent)]
    Store(#[from] StoreError),
    /// The tract is loaded already.
    #[error("tract {tract} of realm {realm} is already loaded")]
    AlreadyLoaded {
        /// The realm.
        realm: RealmId,
        /// The tract.
        tract: TractPos,
    },
    /// The tract is not loaded.
    #[error("tract {tract} of realm {realm} is not loaded")]
    NotLoaded {
        /// The realm.
        realm: RealmId,
        /// The tract.
        tract: TractPos,
    },
    /// Someone is still in the tract. Every occupant is in loaded space
    /// (invariant W4), so the tract stays.
    #[error("tract {tract} of realm {realm} still holds occupant {occupant}")]
    Occupied {
        /// The realm.
        realm: RealmId,
        /// The tract.
        tract: TractPos,
        /// One of those in it: the one with the lowest id.
        occupant: EntityId,
    },
    /// The tract's record says its cells are stored, and the store has
    /// none for it.
    #[error("tract {tract} of realm {realm} was stored, and the store does not hold it")]
    NotStored {
        /// The realm.
        realm: RealmId,
        /// The tract.
        tract: TractPos,
    },
    /// Loading the tract would pass the most tracts that may be loaded.
    #[error(
        "tract {tract} of realm {realm} would be more than the {max} tracts that may be loaded"
    )]
    OverBudget {
        /// The realm.
        realm: RealmId,
        /// The tract.
        tract: TractPos,
        /// The most tracts that may be loaded.
        max: usize,
    },
}

/// A tract that has just been loaded, and what comes with it.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct LoadedTract {
    /// The realm.
    pub realm: RealmId,
    /// The tract, inside its realm.
    pub tract: TractPos,
    /// Whether this is the first time the tract has ever been loaded.
    pub first: bool,
    /// Whether its cells were read from the store, not generated.
    pub from_store: bool,
    /// What the simulation is to put there: its generator's requests,
    /// the first time, and those that were waiting for it to load.
    pub spawns: Vec<SpawnRequest>,
    /// The areas its generator asked for, made the first time.
    pub areas: Vec<AreaId>,
}

/// A tract's position brought inside a grid's shape: wrapped on wrapped
/// axes, or the edge it lies beyond.
pub(crate) fn tract_in(topology: &Topology, tract: TractPos) -> Result<TractPos, Edge> {
    topology
        .normalize(tract.origin(topology.z_min))
        .map(|cell| cell.tract())
}

impl TractRecord {
    /// The record as its survey wrote it, with how far along its life
    /// the tract is left out. This is what a realiser is shown, so that
    /// what it makes cannot depend on which tracts happen to have been
    /// loaded before (invariant W6).
    pub fn as_surveyed(&self) -> TractRecord {
        TractRecord {
            def: self.def,
            rot: self.rot,
            variant: self.variant,
            state: TractState::Surveyed,
            devastation: self.devastation,
            devastation_kind: self.devastation_kind,
            site: self.site,
            extra: None,
        }
    }
}

impl Cosmos {
    /// The plain facts about a realm that its generator is given.
    pub fn realm_info(&self, realm: RealmId) -> Result<RealmInfo, WorldError> {
        let found = self.realm(realm).ok_or(WorldError::UnknownRealm(realm))?;
        let kind = self
            .defs()
            .realms()
            .get(found.def)
            .ok_or(WorldError::UnknownRealmDef(found.def))?;
        Ok(RealmInfo {
            id: found.id,
            def: found.def,
            seed: found.seed,
            base_grid: found.base_grid,
            topology: kind.topology,
            below_fill: kind.below_fill,
            above_fill: kind.above_fill,
        })
    }

    /// A tract's position brought inside its realm: wrapped on wrapped
    /// axes, or an error naming the edge it lies beyond.
    pub fn normalize_tract(&self, realm: RealmId, tract: TractPos) -> Result<TractPos, WorldError> {
        let info = self.realm_info(realm)?;
        tract_in(&info.topology, tract).map_err(|edge| WorldError::OutsideRealm {
            loc: Location::new(info.base_grid, tract.origin(info.topology.z_min)),
            edge,
        })
    }

    /// Whether a tract of a realm is loaded.
    pub fn is_tract_loaded(&self, realm: RealmId, tract: TractPos) -> bool {
        self.loaded_column(realm, tract).is_some()
    }

    /// The column of a loaded tract of a realm.
    fn loaded_column(&self, realm: RealmId, tract: TractPos) -> Option<&Column> {
        let tract = self.normalize_tract(realm, tract).ok()?;
        let grid = self.realm(realm)?.base_grid;
        self.grid(grid).ok()?.column(tract)
    }

    /// The loaded tracts of a realm, row by row.
    pub fn loaded_tracts(&self, realm: RealmId) -> Vec<TractPos> {
        self.realm(realm)
            .and_then(|realm| self.grid(realm.base_grid).ok())
            .map(|view| view.loaded_tracts())
            .unwrap_or_default()
    }

    /// The record of a region, if it has been surveyed.
    pub fn region(&self, realm: RealmId, region: RegionPos) -> Option<&RegionRecord> {
        self.realm(realm)?.regions.get(&region)
    }

    /// The record of a tract, if its region has been surveyed.
    pub fn tract_record(&self, realm: RealmId, tract: TractPos) -> Option<&TractRecord> {
        let tract = self.normalize_tract(realm, tract).ok()?;
        Some(self.region(realm, tract.region())?.tract(tract))
    }

    fn tract_record_mut(&mut self, realm: RealmId, tract: TractPos) -> Option<&mut TractRecord> {
        let region = self.realm_mut(realm)?.regions.get_mut(&tract.region())?;
        Some(region.tract_mut(tract))
    }

    /// Surveys a region if it has not been: asks the realm's generator
    /// for its record, makes its sites and keeps the record. Returns
    /// whether it was surveyed now. A pure function of the definitions,
    /// the realm's seed and the position, so regions may be surveyed in
    /// any order.
    pub fn survey_region(
        &mut self,
        generators: &Generators,
        realm: RealmId,
        region: RegionPos,
    ) -> Result<bool, LoadError> {
        let info = self.realm_info(realm)?;
        // The region must be one of the realm's own.
        let origin = self.normalize_tract(realm, region.origin())?;
        let region = origin.region();
        if self.region(realm, region).is_some() {
            return Ok(false);
        }
        let generator = generators.of(info.def)?;
        let RegionSurveyResult { mut record, sites } = {
            let defs = Arc::clone(self.defs());
            generator.survey(&GenCtx::new(&defs), &info, region)
        };
        for mut spec in sites {
            spec.realm = realm;
            let tracts = spec.tracts.clone();
            let Ok(index) = u16::try_from(record.sites.len()) else {
                break;
            };
            // A site that cannot be made, because one of its tracts has
            // a site already, is left out.
            if let Ok(site) = self.create_site(spec) {
                record.sites.push(site);
                for tract in tracts {
                    if tract.region() == region {
                        record.tract_mut(tract).site = Some(SiteIdx(index));
                    }
                }
            }
        }
        if let Some(found) = self.realm_mut(realm) {
            found.regions.insert(region, record);
        }
        Ok(true)
    }

    /// Adds a change to a tract that is not loaded, to be applied when
    /// it next loads (02 §17.3).
    ///
    /// Refused for a tract that is loaded (edit it), a tract whose
    /// region has not been surveyed, and cell changes that name a cell
    /// outside the tract.
    pub fn add_pending_edit(
        &mut self,
        realm: RealmId,
        tract: TractPos,
        edit: PendingEdit,
    ) -> Result<(), WorldError> {
        let tract = self.normalize_tract(realm, tract)?;
        let grid = self.realm_info(realm)?.base_grid;
        if self.is_tract_loaded(realm, tract) {
            return Err(WorldError::BadValue {
                what: "a pending edit",
                why: "its tract is loaded, where changes are made through WorldEdit",
            });
        }
        let edit = match edit {
            PendingEdit::Cells(cells) => {
                let mut placed = Vec::with_capacity(cells.len());
                for change in cells {
                    let cell = self.normalize(Location::new(grid, change.cell))?.cell;
                    if cell.tract() != tract {
                        return Err(WorldError::BadValue {
                            what: "a pending edit",
                            why: "it names a cell outside the tract it is given to",
                        });
                    }
                    placed.push(CellEdit { cell, ..change });
                }
                PendingEdit::Cells(placed)
            }
            other => other,
        };
        let record = self
            .tract_record_mut(realm, tract)
            .ok_or(WorldError::NotSurveyed { realm, tract })?;
        record.extra_mut().pending.push(edit);
        Ok(())
    }
}

/// Where the cells of a tract being loaded come from.
enum Source {
    /// The tract store.
    Stored,
    /// The realiser: the result of the job at this position in the
    /// batch.
    Realized(usize),
}

/// A tract that is going to be loaded.
struct Planned {
    realm: RealmId,
    grid: GridId,
    tract: TractPos,
    source: Source,
}

impl WorldCtx<'_> {
    /// Loads one tract. See [`load_tracts`](WorldCtx::load_tracts).
    pub fn load_tract(
        &mut self,
        io: &mut TractIo<'_>,
        realm: RealmId,
        tract: TractPos,
    ) -> Result<LoadedTract, LoadError> {
        match self.load_tracts(io, &[(realm, tract)]).pop() {
            Some(outcome) => outcome,
            None => Err(LoadError::NotLoaded { realm, tract }),
        }
    }

    /// Loads tracts (02 §9.4), and says for each, in the order given,
    /// what was loaded or why it was not. Call it at a safe point.
    ///
    /// The tracts that need generating are realised together, then each
    /// tract is installed in the order given: its column joins the grid,
    /// the first time its areas are made, the changes waiting for it
    /// are applied through `WorldEdit`, and `TractLoaded` is emitted. A
    /// tract that is loaded already, or named twice, is refused.
    pub fn load_tracts(
        &mut self,
        io: &mut TractIo<'_>,
        tracts: &[(RealmId, TractPos)],
    ) -> Vec<Result<LoadedTract, LoadError>> {
        let defs = Arc::clone(self.cosmos.defs());
        let mut jobs: Vec<RealizeJob> = Vec::new();
        let mut claimed: BTreeSet<(RealmId, TractPos)> = BTreeSet::new();
        let plans: Vec<Result<Planned, LoadError>> = tracts
            .iter()
            .map(|&(realm, tract)| self.plan(io, realm, tract, &mut claimed, &mut jobs))
            .collect();
        let mut realised: Vec<Option<RealizedTract>> =
            realize_batch(&defs, io.generators.runner(), &jobs)
                .into_iter()
                .map(Some)
                .collect();
        plans
            .into_iter()
            .map(|plan| self.install(io, &defs, plan?, &mut realised))
            .collect()
    }

    /// Decides where a tract's cells will come from, surveying what
    /// that needs, and queues its realisation if it is to be generated.
    fn plan(
        &mut self,
        io: &mut TractIo<'_>,
        realm: RealmId,
        tract: TractPos,
        claimed: &mut BTreeSet<(RealmId, TractPos)>,
        jobs: &mut Vec<RealizeJob>,
    ) -> Result<Planned, LoadError> {
        let info = self.cosmos.realm_info(realm)?;
        let tract = self.cosmos.normalize_tract(realm, tract)?;
        if self.cosmos.is_tract_loaded(realm, tract) || claimed.contains(&(realm, tract)) {
            return Err(LoadError::AlreadyLoaded { realm, tract });
        }
        self.cosmos
            .survey_region(io.generators, realm, tract.region())?;
        let record = self
            .cosmos
            .tract_record(realm, tract)
            .ok_or(WorldError::NotSurveyed { realm, tract })?;
        let source = if record.state.is_stored() {
            Source::Stored
        } else {
            let rec = record.as_surveyed();
            let generator = Arc::clone(io.generators.of(info.def)?);
            let mut around: [Option<TractRecord>; TractRecords3x3::COUNT] = Default::default();
            for (slot, (dx, dy)) in around.iter_mut().zip(TractRecords3x3::OFFSETS) {
                let near = tract.offset(IVec2::new(dx, dy));
                let Ok(near) = tract_in(&info.topology, near) else {
                    continue;
                };
                self.cosmos
                    .survey_region(io.generators, realm, near.region())?;
                *slot = self
                    .cosmos
                    .tract_record(realm, near)
                    .map(TractRecord::as_surveyed);
            }
            jobs.push(RealizeJob {
                generator,
                realm: info,
                pos: tract,
                rec,
                around: TractRecords3x3::new(around),
            });
            Source::Realized(jobs.len() - 1)
        };
        claimed.insert((realm, tract));
        Ok(Planned {
            realm,
            grid: info.base_grid,
            tract,
            source,
        })
    }

    /// Makes a planned tract part of the world.
    fn install(
        &mut self,
        io: &mut TractIo<'_>,
        defs: &WorldDefs,
        plan: Planned,
        realised: &mut [Option<RealizedTract>],
    ) -> Result<LoadedTract, LoadError> {
        let Planned {
            realm,
            grid,
            tract,
            source,
        } = plan;
        let from_store = matches!(source, Source::Stored);
        let (column, fresh) =
            match source {
                Source::Stored => {
                    let blob = io
                        .store
                        .get(realm, tract)?
                        .ok_or(LoadError::NotStored { realm, tract })?;
                    (decode_column(defs, &blob)?, None)
                }
                Source::Realized(at) => {
                    let made = realised.get_mut(at).and_then(Option::take).ok_or(
                        WorldError::BadValue {
                            what: "a batch of tracts",
                            why: "fewer tracts were realised than were asked for",
                        },
                    )?;
                    let RealizedTract {
                        column,
                        spawns,
                        areas,
                        ..
                    } = made;
                    (column, Some((spawns, areas)))
                }
            };
        let tick = self.timeline.now();
        self.cosmos.insert_column(grid, tract, column)?;
        self.cosmos.tickets.note_loaded(realm, tract);
        let mut loaded = LoadedTract {
            realm,
            tract,
            first: false,
            from_store,
            spawns: Vec::new(),
            areas: Vec::new(),
        };
        let mut pending = Vec::new();
        if let Some(record) = self.cosmos.tract_record_mut(realm, tract) {
            loaded.first = !record.state.is_realized();
            if loaded.first {
                record.state = TractState::Realized { pristine: true };
            }
            if let Some(extra) = record.extra.as_mut() {
                pending = core::mem::take(&mut extra.pending);
            }
        }
        // What a generator asks for besides cells is made once, the
        // first time: areas are the cosmos's and outlive the loading.
        if let (true, Some((spawns, areas))) = (loaded.first, fresh) {
            loaded.spawns = spawns;
            for spec in areas {
                if let Ok(area) = self.create_area(spec, Cause::GENERATOR) {
                    loaded.areas.push(area);
                }
            }
        }
        // The changes that were waiting, in order (02 §9.4 step 3).
        let mut waiting = Vec::new();
        for edit in pending {
            match edit {
                PendingEdit::Cells(cells) => {
                    let mut edit = self.edit(Cause::new(CauseKind::Coarse));
                    for change in cells {
                        // A change the world refuses now is dropped: the
                        // place it was meant for is not there.
                        let at = Location::new(grid, change.cell);
                        let _ = edit.set_element(at, change.layer, change.state);
                    }
                }
                PendingEdit::Spawn(spawn) => loaded.spawns.push(spawn),
                // Stamps wait for the coarse layer (W17).
                stamp @ PendingEdit::Stamp(_) => waiting.push(stamp),
            }
        }
        if !waiting.is_empty()
            && let Some(record) = self.cosmos.tract_record_mut(realm, tract)
        {
            record.extra_mut().pending = waiting;
        }
        self.events.emit(WorldEvent::TractLoaded {
            tick,
            realm,
            loc: Location::new(grid, tract.origin(0)),
            tract,
            first: loaded.first,
        });
        Ok(loaded)
    }

    /// Unloads a tract (02 §9.5). Call it at a safe point.
    ///
    /// Refused, changing nothing, while any occupant has a cell in the
    /// tract: the simulation takes its entities out first. Otherwise
    /// `TractUnloading` is emitted, the column is written to the store
    /// if it has changed since it was loaded, and the tract's record
    /// notes the local time.
    pub fn unload_tract(
        &mut self,
        io: &mut TractIo<'_>,
        realm: RealmId,
        tract: TractPos,
    ) -> Result<(), LoadError> {
        self.unload(io, realm, tract, true)
    }

    /// Unloads a tract, announcing it or not: the reconciliation of
    /// tickets has announced it already for a tract it had to wait for.
    pub(crate) fn unload(
        &mut self,
        io: &mut TractIo<'_>,
        realm: RealmId,
        tract: TractPos,
        announce: bool,
    ) -> Result<(), LoadError> {
        let info = self.cosmos.realm_info(realm)?;
        let tract = self.cosmos.normalize_tract(realm, tract)?;
        let column = self
            .cosmos
            .loaded_column(realm, tract)
            .ok_or(LoadError::NotLoaded { realm, tract })?;
        if let Some(occupant) = first_occupant(column) {
            return Err(LoadError::Occupied {
                realm,
                tract,
                occupant,
            });
        }
        let surveyed = self.cosmos.tract_record(realm, tract).is_some();
        let blob = if surveyed && column.is_dirty() {
            Some(encode_column(column)?)
        } else {
            None
        };
        let clock = self
            .cosmos
            .realm(realm)
            .ok_or(WorldError::UnknownRealm(realm))?
            .clock;
        let local = self.timeline.local_now(clock).map_err(WorldError::from)?;
        // Nothing below can fail.
        if announce {
            self.events.emit(WorldEvent::TractUnloading {
                tick: self.timeline.now(),
                realm,
                loc: Location::new(info.base_grid, tract.origin(0)),
                tract,
            });
        }
        if let Some(blob) = &blob {
            io.store.put(realm, tract, blob);
        }
        if let Some(record) = self.cosmos.tract_record_mut(realm, tract) {
            if blob.is_some() {
                record.state = TractState::Realized { pristine: false };
            }
            if record.state.is_realized() {
                record.extra_mut().last_loaded = Tick(local.as_micros());
            }
        }
        self.cosmos.remove_column(info.base_grid, tract);
        Ok(())
    }
}

/// The occupant with the lowest id that has a cell in a column.
pub(crate) fn first_occupant(column: &Column) -> Option<EntityId> {
    if !column.has_occupants() {
        return None;
    }
    column
        .occupant_indices(i32::MIN, i32::MAX)
        .flat_map(|(_, index)| index.iter().map(|entry| entry.id))
        .min()
}
