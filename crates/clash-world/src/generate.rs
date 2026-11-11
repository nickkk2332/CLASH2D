//! The generation interfaces: how a region is surveyed and a tract is
//! realised (02 §9.3).
//!
//! The interfaces live here, in the world, because loading a tract
//! (02 §9.4) calls them and the world cannot depend on the crate that
//! implements them (01 §3). `clash-gen` implements them and re-exports
//! them.
//!
//! # Generation is pure
//!
//! A generator is given a [`GenCtx`] (the definitions and positional
//! randomness), a [`RealmInfo`] (plain facts about the realm), a position
//! and records: its own tract's and its eight neighbours'
//! ([`TractRecords3x3`]). None of these holds a cell, a grid or the
//! cosmos, so a realiser cannot read a neighbouring tract's cells: what
//! spans tracts is agreed in records at survey time, or follows from
//! position alone. That is what lets tracts be realised in any order and
//! in parallel and always come out the same (invariants W2 and W6).
//!
//! A generator is `'static`: it cannot borrow the world either. This
//! compiles:
//!
//! ```
//! use std::sync::Arc;
//! use clash_world::{
//!     ColumnBuilder, GenCtx, RealizedTract, RealmInfo, RegionPos, RegionRecord, RegionSurvey,
//!     RegionSurveyResult, RegionSurveyor, TractPos, TractRealizer, TractRecord, TractRecords3x3,
//!     WorldGenerator,
//! };
//!
//! #[derive(Debug)]
//! struct Empty;
//!
//! impl RegionSurveyor for Empty {
//!     fn survey(&self, _: &GenCtx<'_>, _: &RealmInfo, _: RegionPos) -> RegionSurveyResult {
//!         RegionSurveyResult::new(RegionRecord::new(RegionSurvey::default()))
//!     }
//! }
//!
//! impl TractRealizer for Empty {
//!     fn realize(
//!         &self, cx: &GenCtx<'_>, realm: &RealmInfo, _: TractPos, _: &TractRecord,
//!         _: &TractRecords3x3,
//!     ) -> RealizedTract {
//!         let column = ColumnBuilder::new(cx.defs(), realm.below_fill, realm.above_fill, 0);
//!         RealizedTract::new(column.build())
//!     }
//! }
//!
//! let generator: Arc<dyn WorldGenerator> = Arc::new(Empty);
//! ```
//!
//! and the same generator holding a reference to the world does not:
//!
//! ```compile_fail
//! use std::sync::Arc;
//! use clash_world::{
//!     ColumnBuilder, Cosmos, GenCtx, RealizedTract, RealmInfo, RegionPos, RegionRecord,
//!     RegionSurvey, RegionSurveyResult, RegionSurveyor, TractPos, TractRealizer, TractRecord,
//!     TractRecords3x3, WorldGenerator,
//! };
//!
//! #[derive(Debug)]
//! struct Peeker<'a>(&'a Cosmos);
//!
//! impl RegionSurveyor for Peeker<'_> {
//!     fn survey(&self, _: &GenCtx<'_>, _: &RealmInfo, _: RegionPos) -> RegionSurveyResult {
//!         RegionSurveyResult::new(RegionRecord::new(RegionSurvey::default()))
//!     }
//! }
//!
//! impl TractRealizer for Peeker<'_> {
//!     fn realize(
//!         &self, cx: &GenCtx<'_>, realm: &RealmInfo, _: TractPos, _: &TractRecord,
//!         _: &TractRecords3x3,
//!     ) -> RealizedTract {
//!         let column = ColumnBuilder::new(cx.defs(), realm.below_fill, realm.above_fill, 0);
//!         RealizedTract::new(column.build())
//!     }
//! }
//!
//! fn smuggle(cosmos: &Cosmos) -> Arc<dyn WorldGenerator> {
//!     Arc::new(Peeker(cosmos))
//! }
//! ```
//!
//! # Choosing a generator
//!
//! A realm definition names its generator (`"generator": "flat"`) and
//! may pass it parameters (`"generator_params": { ... }`). Generators
//! register a factory by name in a [`GeneratorRegistry`];
//! [`GeneratorRegistry::bind`] builds, for each realm definition, the
//! generator its name and parameters ask for. The result, a
//! [`Generators`], is what loading takes. A realm definition whose
//! generator is unknown or whose parameters are wrong is not an error
//! until a tract of such a realm is to be generated; then it is a
//! [`GenError`] naming the definition and the parameter.

use core::fmt;
use core::ops::RangeInclusive;
use std::collections::BTreeMap;
use std::sync::{Arc, OnceLock};

use clash_core::{HashDomain, Rng, pos_hash, pos_rng};
use thiserror::Error;

use crate::area::AreaSpec;
use crate::cosmos::RealmId;
use crate::defs::{
    BlockState, FixtureState, FloorState, GeneratorSpec, RealmDefId, RealmKind, StateRefError,
    WorldDefs,
};
use crate::grid::Column;
use crate::pos::{CellPos, GridId, RegionPos, TractPos};
use crate::site::SiteSpec;
use crate::topology::Topology;
use crate::tract::{RegionRecord, SpawnRequest, TractRecord};

/// What generation may use: the definitions and positional randomness,
/// and nothing else (02 §9.3).
#[derive(Clone, Copy, Debug)]
pub struct GenCtx<'a> {
    defs: &'a WorldDefs,
}

impl<'a> GenCtx<'a> {
    /// A context over a set of definitions.
    pub fn new(defs: &'a WorldDefs) -> GenCtx<'a> {
        GenCtx { defs }
    }

    /// The definitions.
    pub fn defs(&self) -> &'a WorldDefs {
        self.defs
    }

    /// A random number decided by where and what for, never by when or
    /// in what order: the positional hash of the realm's seed, a domain,
    /// a position and a salt (01 §4).
    pub fn hash(&self, realm: &RealmInfo, domain: HashDomain, at: CellPos, salt: u64) -> u64 {
        pos_hash(realm.seed, domain, at.x, at.y, at.z, salt)
    }

    /// A generator seeded by [`hash`](GenCtx::hash), for when one
    /// position needs several random numbers.
    pub fn rng(&self, realm: &RealmInfo, domain: HashDomain, at: CellPos, salt: u64) -> Rng {
        pos_rng(realm.seed, domain, at.x, at.y, at.z, salt)
    }
}

/// Plain facts about the realm being generated. It holds no cells.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct RealmInfo {
    /// The realm.
    pub id: RealmId,
    /// Its definition.
    pub def: RealmDefId,
    /// The seed its generation draws from.
    pub seed: u64,
    /// Its base grid: the grid of the areas a realised tract asks for.
    pub base_grid: GridId,
    /// Its extent, wrap and storeys.
    pub topology: Topology,
    /// What implicit cells hold below a tract's stored range.
    pub below_fill: BlockState,
    /// What implicit cells hold above a tract's stored range.
    pub above_fill: BlockState,
}

/// The records of a tract's eight neighbours, and its own in the middle.
/// A neighbour beyond an edge of the realm has none.
#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct TractRecords3x3 {
    /// Row by row from the north-west.
    records: [Option<TractRecord>; TractRecords3x3::COUNT],
}

impl TractRecords3x3 {
    /// How many tracts a neighbourhood holds.
    pub const COUNT: usize = 9;

    /// The offsets of the nine tracts, row by row from the north-west.
    pub const OFFSETS: [(i32, i32); TractRecords3x3::COUNT] = [
        (-1, -1),
        (0, -1),
        (1, -1),
        (-1, 0),
        (0, 0),
        (1, 0),
        (-1, 1),
        (0, 1),
        (1, 1),
    ];

    /// A neighbourhood from the record at each of [`OFFSETS`]
    /// (TractRecords3x3::OFFSETS), in that order.
    pub fn new(records: [Option<TractRecord>; TractRecords3x3::COUNT]) -> TractRecords3x3 {
        TractRecords3x3 { records }
    }

    /// The record of the tract `dx` east and `dy` south of the middle,
    /// each from −1 to 1. `None` beyond an edge of the realm, and for an
    /// offset outside the neighbourhood.
    pub fn get(&self, dx: i32, dy: i32) -> Option<&TractRecord> {
        if !(-1..=1).contains(&dx) || !(-1..=1).contains(&dy) {
            return None;
        }
        let at = usize::try_from((dy + 1) * 3 + dx + 1).ok()?;
        self.records.get(at)?.as_ref()
    }
}

/// A portal a realised tract asks for (02 §19). A typed placeholder:
/// portals are package W19's.
#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct PortalSpec {}

/// A subgrid a realised tract asks for (02 §18). A typed placeholder:
/// subgrids are package W20a's.
#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct SubgridSpec {}

/// What surveying a region gives: its record, with a record for each of
/// its tracts, and the sites to make in it.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct RegionSurveyResult {
    /// The region's record.
    pub record: RegionRecord,
    /// The sites the region holds. The world makes them and lists them
    /// in the record.
    pub sites: Vec<SiteSpec>,
}

impl RegionSurveyResult {
    /// A survey with a record and no sites.
    pub fn new(record: RegionRecord) -> RegionSurveyResult {
        RegionSurveyResult {
            record,
            sites: Vec::new(),
        }
    }
}

/// What realising a tract gives (02 §9.3).
#[derive(Clone, Debug)]
pub struct RealizedTract {
    /// The tract's cells: its chunks and its two fills, built with a
    /// [`ColumnBuilder`](crate::ColumnBuilder).
    pub column: Column,
    /// What the simulation should put there. Handed over when the tract
    /// is first loaded.
    pub spawns: Vec<SpawnRequest>,
    /// Areas to make when the tract is first loaded.
    pub areas: Vec<AreaSpec>,
    /// Portals to open. Carried until portals are built.
    pub portals: Vec<PortalSpec>,
    /// Subgrids to place. Carried until subgrids are built.
    pub subgrids: Vec<SubgridSpec>,
}

impl RealizedTract {
    /// A realised tract with these cells and nothing else.
    pub fn new(column: Column) -> RealizedTract {
        RealizedTract {
            column,
            spawns: Vec::new(),
            areas: Vec::new(),
            portals: Vec::new(),
            subgrids: Vec::new(),
        }
    }
}

/// Lays out a region: a record for each of its tracts, and its sites.
pub trait RegionSurveyor {
    /// Surveys a region. A pure function of the definitions, the realm's
    /// seed and the position.
    fn survey(&self, cx: &GenCtx<'_>, realm: &RealmInfo, pos: RegionPos) -> RegionSurveyResult;
}

/// Turns a tract's record into cells.
pub trait TractRealizer {
    /// Realises a tract. A pure function of the definitions, the realm's
    /// seed, the position and the records: the tract's own and its
    /// neighbours'. It sees no cell of any other tract.
    fn realize(
        &self,
        cx: &GenCtx<'_>,
        realm: &RealmInfo,
        pos: TractPos,
        rec: &TractRecord,
        around: &TractRecords3x3,
    ) -> RealizedTract;
}

/// A generator of worlds: it surveys regions and realises tracts, can be
/// shared between threads, and borrows nothing. Implemented for
/// everything that qualifies.
pub trait WorldGenerator:
    RegionSurveyor + TractRealizer + Send + Sync + fmt::Debug + 'static
{
}

impl<T> WorldGenerator for T where
    T: RegionSurveyor + TractRealizer + Send + Sync + fmt::Debug + 'static
{
}

/// Why a realm's generator could not be had. Each names the realm
/// definition, and a parameter by its path within the definition.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[non_exhaustive]
pub enum GenError {
    /// The realm definition names no generator.
    #[error("realm definition `{realm}` names no generator, so its tracts cannot be generated")]
    NoGenerator {
        /// The realm definition's qualified id.
        realm: String,
    },
    /// No generator is registered under the name the definition gives.
    #[error("realm definition `{realm}`, `generator`: there is no generator `{name}`")]
    UnknownGenerator {
        /// The realm definition's qualified id.
        realm: String,
        /// The name it gives.
        name: String,
    },
    /// The definition asks for a realm derived from another (02 §21),
    /// which is not built yet.
    #[error("realm definition `{realm}`, `generator`: derived realms are not built yet")]
    Derived {
        /// The realm definition's qualified id.
        realm: String,
    },
    /// A parameter is missing, of the wrong kind, out of range or names
    /// something that does not exist.
    #[error("realm definition `{realm}`, `{path}`: {message}")]
    BadParam {
        /// The realm definition's qualified id.
        realm: String,
        /// The value's path within the definition: `generator_params.wall`.
        path: String,
        /// What is wrong with it.
        message: String,
    },
}

/// The key of a realm definition that holds its generator's parameters.
const PARAMS_KEY: &str = "generator_params";

/// What a generator's factory is given: the realm definition it is to
/// generate for, with helpers that read its parameters and report what
/// is wrong with them.
#[derive(Clone, Copy, Debug)]
pub struct GeneratorRequest<'a> {
    /// The definitions.
    pub defs: &'a WorldDefs,
    /// The realm definition.
    pub realm_def: RealmDefId,
    /// Its qualified id.
    pub realm_name: &'a str,
    /// What it says.
    pub kind: &'a RealmKind,
}

impl<'a> GeneratorRequest<'a> {
    /// The namespace of the realm definition, in which bare names among
    /// its parameters are looked up first.
    fn namespace(&self) -> Option<&'a str> {
        self.realm_name
            .split_once(':')
            .map(|(namespace, _)| namespace)
    }

    /// A mistake in a parameter.
    pub fn error(&self, param: &str, message: impl Into<String>) -> GenError {
        GenError::BadParam {
            realm: self.realm_name.to_owned(),
            path: format!("{PARAMS_KEY}.{param}"),
            message: message.into(),
        }
    }

    fn value(&self, param: &str) -> Option<&'a serde_json::Value> {
        self.kind.generator_params.get(param)
    }

    /// A whole number within a range, or `default` if the parameter is
    /// not given.
    pub fn int(
        &self,
        param: &str,
        default: i64,
        range: RangeInclusive<i64>,
    ) -> Result<i64, GenError> {
        let Some(value) = self.value(param) else {
            return Ok(default);
        };
        self.int_in(param, value, &range)
    }

    fn int_in(
        &self,
        param: &str,
        value: &serde_json::Value,
        range: &RangeInclusive<i64>,
    ) -> Result<i64, GenError> {
        match value.as_i64() {
            Some(number) if range.contains(&number) => Ok(number),
            Some(number) => Err(self.error(
                param,
                format!(
                    "must be from {} to {}, not {number}",
                    range.start(),
                    range.end()
                ),
            )),
            None => Err(self.error(param, "must be a whole number")),
        }
    }

    /// Two whole numbers within a range, the first no greater than the
    /// second, written as a list; or `default` if the parameter is not
    /// given.
    pub fn int_pair(
        &self,
        param: &str,
        default: (i64, i64),
        range: RangeInclusive<i64>,
    ) -> Result<(i64, i64), GenError> {
        let Some(value) = self.value(param) else {
            return Ok(default);
        };
        let Some([low, high]) = value.as_array().map(Vec::as_slice) else {
            return Err(self.error(param, "must be a list of two whole numbers"));
        };
        let low = self.int_in(param, low, &range)?;
        let high = self.int_in(param, high, &range)?;
        if low > high {
            return Err(self.error(param, "the first number must not be above the second"));
        }
        Ok((low, high))
    }

    fn state_text(&self, param: &str, what: &str) -> Result<&'a str, GenError> {
        match self.value(param) {
            Some(serde_json::Value::String(text)) => Ok(text),
            Some(_) => Err(self.error(param, format!("must be the state of a {what}, as text"))),
            None => Err(self.error(
                param,
                format!("the generator needs this parameter: the state of a {what}"),
            )),
        }
    }

    fn state_error(&self, param: &str, error: StateRefError) -> GenError {
        self.error(param, error.to_string())
    }

    /// The floor state a required parameter names.
    pub fn floor(&self, param: &str) -> Result<FloorState, GenError> {
        let text = self.state_text(param, "floor")?;
        self.defs
            .floor_state(text, self.namespace())
            .map_err(|error| self.state_error(param, error))
    }

    /// The block state a required parameter names.
    pub fn block(&self, param: &str) -> Result<BlockState, GenError> {
        let text = self.state_text(param, "block")?;
        self.defs
            .block_state(text, self.namespace())
            .map_err(|error| self.state_error(param, error))
    }

    /// The fixture state a required parameter names.
    pub fn fixture(&self, param: &str) -> Result<FixtureState, GenError> {
        let text = self.state_text(param, "fixture")?;
        self.defs
            .fixture_state(text, self.namespace())
            .map_err(|error| self.state_error(param, error))
    }

    /// Refuses a parameter the generator does not know: a misspelt name
    /// must not be silently ignored.
    pub fn only(&self, known: &[&str]) -> Result<(), GenError> {
        match self
            .kind
            .generator_params
            .keys()
            .find(|key| !known.contains(&key.as_str()))
        {
            Some(unknown) => Err(self.error(
                unknown,
                format!(
                    "the generator has no such parameter; it takes: {}",
                    known.join(", ")
                ),
            )),
            None => Ok(()),
        }
    }
}

/// Makes a generator for a realm definition, or says what is wrong with
/// the definition's parameters.
pub type GeneratorFactory = fn(&GeneratorRequest<'_>) -> Result<Arc<dyn WorldGenerator>, GenError>;

/// Runs the pieces of a batch of pure work: one after another, or on
/// several threads.
///
/// `run` must call `job` exactly once for each number below `count`, and
/// return when every call has returned. The order and the threads are
/// its own business: whoever gives it work makes the result independent
/// of both (01 §4).
pub trait BatchRunner: Send + Sync + fmt::Debug {
    /// Calls `job(i)` for every `i` below `count`.
    fn run(&self, count: usize, job: &(dyn Fn(usize) + Sync));
}

/// Runs a batch on the calling thread, in order.
#[derive(Clone, Copy, Debug, Default)]
pub struct SerialRunner;

impl BatchRunner for SerialRunner {
    fn run(&self, count: usize, job: &(dyn Fn(usize) + Sync)) {
        for index in 0..count {
            job(index);
        }
    }
}

/// The generators there are, by name.
#[derive(Clone, Debug)]
pub struct GeneratorRegistry {
    factories: BTreeMap<String, GeneratorFactory>,
    runner: Arc<dyn BatchRunner>,
}

impl Default for GeneratorRegistry {
    fn default() -> GeneratorRegistry {
        GeneratorRegistry::new()
    }
}

impl GeneratorRegistry {
    /// A registry of no generators, which realises on the calling
    /// thread.
    pub fn new() -> GeneratorRegistry {
        GeneratorRegistry {
            factories: BTreeMap::new(),
            runner: Arc::new(SerialRunner),
        }
    }

    /// Registers a generator under the name realm definitions give it
    /// by. Returns the factory the name had before, if any.
    pub fn register(&mut self, name: &str, factory: GeneratorFactory) -> Option<GeneratorFactory> {
        self.factories.insert(name.to_owned(), factory)
    }

    /// The names registered, in order.
    pub fn names(&self) -> impl Iterator<Item = &str> + '_ {
        self.factories.keys().map(String::as_str)
    }

    /// Sets how batches of tracts are realised.
    pub fn set_runner(&mut self, runner: Arc<dyn BatchRunner>) {
        self.runner = runner;
    }

    /// Builds the generator of every realm definition.
    pub fn bind(&self, defs: &Arc<WorldDefs>) -> Generators {
        let by_def = defs
            .realms()
            .iter()
            .map(|(id, kind)| {
                let realm_name = defs.realms().name(id);
                let name = match &kind.generator {
                    Some(GeneratorSpec::Named(name)) => name.0.as_str(),
                    Some(GeneratorSpec::Derived(_)) => {
                        return Err(GenError::Derived {
                            realm: realm_name.to_owned(),
                        });
                    }
                    None => {
                        return Err(GenError::NoGenerator {
                            realm: realm_name.to_owned(),
                        });
                    }
                };
                let factory =
                    self.factories
                        .get(name)
                        .ok_or_else(|| GenError::UnknownGenerator {
                            realm: realm_name.to_owned(),
                            name: name.to_owned(),
                        })?;
                factory(&GeneratorRequest {
                    defs,
                    realm_def: id,
                    realm_name,
                    kind,
                })
            })
            .collect();
        Generators {
            defs: Arc::clone(defs),
            by_def,
            runner: Arc::clone(&self.runner),
        }
    }
}

/// The generator of each realm definition, built: what loading a tract
/// takes (02 §9.4). Made by [`GeneratorRegistry::bind`].
#[derive(Clone, Debug)]
pub struct Generators {
    defs: Arc<WorldDefs>,
    /// By realm definition id.
    by_def: Vec<Result<Arc<dyn WorldGenerator>, GenError>>,
    runner: Arc<dyn BatchRunner>,
}

impl Generators {
    /// No generators at all: every realm definition is refused as
    /// naming a generator that does not exist. For a world whose tracts
    /// are all put there by hand.
    pub fn none(defs: &Arc<WorldDefs>) -> Generators {
        GeneratorRegistry::new().bind(defs)
    }

    /// The definitions the generators were built from.
    pub fn defs(&self) -> &Arc<WorldDefs> {
        &self.defs
    }

    /// The generator of a realm definition, or why there is none.
    pub fn of(&self, def: RealmDefId) -> Result<&Arc<dyn WorldGenerator>, GenError> {
        match self.by_def.get(def.index()) {
            Some(Ok(generator)) => Ok(generator),
            Some(Err(error)) => Err(error.clone()),
            None => Err(GenError::NoGenerator {
                realm: def.to_string(),
            }),
        }
    }

    /// What is wrong with the parameters realm definitions give their
    /// generators. A definition that names no generator, or one that is
    /// not registered, is not listed: it is refused when used.
    pub fn param_errors(&self) -> impl Iterator<Item = &GenError> + '_ {
        self.by_def
            .iter()
            .filter_map(|bound| bound.as_ref().err())
            .filter(|error| matches!(error, GenError::BadParam { .. }))
    }

    /// How batches of tracts are realised.
    pub fn runner(&self) -> &dyn BatchRunner {
        self.runner.as_ref()
    }
}

/// One tract to realise: everything its realiser is given.
#[derive(Clone, Debug)]
pub struct RealizeJob {
    /// The generator of the tract's realm.
    pub generator: Arc<dyn WorldGenerator>,
    /// The realm.
    pub realm: RealmInfo,
    /// The tract.
    pub pos: TractPos,
    /// Its record.
    pub rec: TractRecord,
    /// Its neighbours' records.
    pub around: TractRecords3x3,
}

impl RealizeJob {
    fn run(&self, defs: &WorldDefs) -> RealizedTract {
        let cx = GenCtx::new(defs);
        self.generator
            .realize(&cx, &self.realm, self.pos, &self.rec, &self.around)
    }
}

/// Realises a batch of tracts and returns the results in the order of
/// the jobs, whatever order and threads the runner used. Each job is a
/// pure function of what it holds, and each result goes into a place of
/// its own, so the outcome does not depend on scheduling (01 §4).
pub fn realize_batch(
    defs: &WorldDefs,
    runner: &dyn BatchRunner,
    jobs: &[RealizeJob],
) -> Vec<RealizedTract> {
    let slots: Vec<OnceLock<RealizedTract>> = jobs.iter().map(|_| OnceLock::new()).collect();
    runner.run(jobs.len(), &|index| {
        if let (Some(job), Some(slot)) = (jobs.get(index), slots.get(index)) {
            slot.get_or_init(|| job.run(defs));
        }
    });
    slots
        .into_iter()
        .zip(jobs)
        // A runner that left a job out does not leave a hole: the job is
        // done here.
        .map(|(slot, job)| slot.into_inner().unwrap_or_else(|| job.run(defs)))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::defs::RawWorldDefs;

    #[test]
    fn a_neighbourhood_is_read_by_offset() {
        let mut records: [Option<TractRecord>; TractRecords3x3::COUNT] = Default::default();
        for (at, (dx, dy)) in TractRecords3x3::OFFSETS.iter().enumerate() {
            // Mark each record with its place; leave the south-east out,
            // as beyond an edge.
            if (*dx, *dy) != (1, 1) {
                records[at] = Some(TractRecord {
                    variant: at as u8,
                    ..TractRecord::default()
                });
            }
        }
        let around = TractRecords3x3::new(records);
        assert_eq!(around.get(0, 0).map(|rec| rec.variant), Some(4));
        assert_eq!(around.get(-1, -1).map(|rec| rec.variant), Some(0));
        assert_eq!(around.get(1, 0).map(|rec| rec.variant), Some(5));
        assert_eq!(around.get(1, 1), None);
        assert_eq!(around.get(2, 0), None);
        assert_eq!(around.get(0, -2), None);
    }

    #[test]
    fn a_realm_without_a_generator_is_refused_by_name() {
        let defs = Arc::new(WorldDefs::build(RawWorldDefs::default()).unwrap());
        let generators = Generators::none(&defs);
        let missing = generators.of(RealmDefId::new(3)).err();
        assert!(matches!(missing, Some(GenError::NoGenerator { .. })));
        assert_eq!(generators.param_errors().count(), 0);
    }

    /// A runner that does its jobs backwards, and one that does none.
    #[derive(Debug)]
    struct Backwards;
    impl BatchRunner for Backwards {
        fn run(&self, count: usize, job: &(dyn Fn(usize) + Sync)) {
            for index in (0..count).rev() {
                job(index);
            }
        }
    }
    #[derive(Debug)]
    struct Idle;
    impl BatchRunner for Idle {
        fn run(&self, _count: usize, _job: &(dyn Fn(usize) + Sync)) {}
    }

    #[derive(Debug)]
    struct Stripes;
    impl RegionSurveyor for Stripes {
        fn survey(&self, _: &GenCtx<'_>, _: &RealmInfo, _: RegionPos) -> RegionSurveyResult {
            RegionSurveyResult::new(RegionRecord::new(Default::default()))
        }
    }
    impl TractRealizer for Stripes {
        fn realize(
            &self,
            cx: &GenCtx<'_>,
            realm: &RealmInfo,
            pos: TractPos,
            _: &TractRecord,
            _: &TractRecords3x3,
        ) -> RealizedTract {
            // The surface height tells the tracts apart.
            let column = crate::ColumnBuilder::new(
                cx.defs(),
                realm.below_fill,
                realm.above_fill,
                pos.x * crate::dims::CHUNK_Z,
            );
            RealizedTract::new(column.build())
        }
    }

    #[test]
    fn a_batch_comes_back_in_job_order_whatever_the_runner_does() {
        let defs = WorldDefs::build(RawWorldDefs::default()).unwrap();
        let generator: Arc<dyn WorldGenerator> = Arc::new(Stripes);
        let realm = RealmInfo {
            id: RealmId::new(0),
            def: RealmDefId::new(0),
            seed: 1,
            base_grid: GridId::new(0),
            topology: Topology::unbounded(),
            // With no definitions there is nothing to fill with: a state
            // that names none reads as nothing and still hashes as
            // itself, which tells where the fills meet.
            below_fill: BlockState::from_raw(crate::ElementState {
                def: 1,
                ..crate::ElementState::NONE
            }),
            above_fill: BlockState::NONE,
        };
        let jobs: Vec<RealizeJob> = (0..5)
            .map(|x| RealizeJob {
                generator: Arc::clone(&generator),
                realm,
                pos: TractPos::new(x, 0),
                rec: TractRecord::default(),
                around: TractRecords3x3::default(),
            })
            .collect();
        let hashes = |runner: &dyn BatchRunner| -> Vec<clash_core::Hash128> {
            realize_batch(&defs, runner, &jobs)
                .iter()
                .map(|tract| clash_core::Hash128::of(&tract.column))
                .collect()
        };
        let straight = hashes(&SerialRunner);
        assert_eq!(straight.len(), jobs.len());
        assert_ne!(straight[0], straight[1]);
        assert_eq!(hashes(&Backwards), straight);
        assert_eq!(hashes(&Idle), straight);
    }
}
