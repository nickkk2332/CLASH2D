//! Realising tracts without a world to put them in, for tools, tests and
//! anything that only wants to look at what a generator makes.

use std::collections::BTreeMap;
use std::sync::Arc;

use clash_world::{
    GenCtx, GenError, Generators, GridId, RealizeJob, RealizedTract, RealmDefId, RealmId,
    RealmInfo, RegionPos, RegionRecord, TractPos, TractRecord, TractRecords3x3, realize_batch,
};

/// Realises tracts of a realm definition, as a realm made from it with
/// this seed would have them, and returns them in the order asked.
///
/// Nothing is loaded and no cosmos is needed: the regions the tracts
/// and their neighbours lie in are surveyed, each tract is realised
/// from the records, and the batch runs on the generators' runner. The
/// result is the same for every order of `tracts` and every thread
/// count (invariant W6). Positions are taken as given: a tract beyond
/// an edge of the realm is realised as if the realm went on, and only
/// its neighbours beyond an edge are left out.
///
/// Refused when the realm definition is unknown or has no generator
/// that can be used.
pub fn realize_tracts(
    generators: &Generators,
    realm_def: RealmDefId,
    seed: u64,
    tracts: &[TractPos],
) -> Result<Vec<RealizedTract>, GenError> {
    let defs = Arc::clone(generators.defs());
    let generator = Arc::clone(generators.of(realm_def)?);
    let kind = defs
        .realms()
        .get(realm_def)
        .ok_or_else(|| GenError::NoGenerator {
            realm: realm_def.to_string(),
        })?;
    let realm = RealmInfo {
        id: RealmId::new(0),
        def: realm_def,
        seed,
        base_grid: GridId::new(0),
        topology: kind.topology,
        below_fill: kind.below_fill,
        above_fill: kind.above_fill,
    };
    let cx = GenCtx::new(&defs);
    let mut regions: BTreeMap<RegionPos, RegionRecord> = BTreeMap::new();
    let mut record = |tract: TractPos| -> TractRecord {
        regions
            .entry(tract.region())
            .or_insert_with(|| generator.survey(&cx, &realm, tract.region()).record)
            .tract(tract)
            .as_surveyed()
    };
    let inside = |tract: TractPos| -> Option<TractPos> {
        let origin = tract.origin(realm.topology.z_min);
        realm
            .topology
            .normalize(origin)
            .ok()
            .map(|cell| cell.tract())
    };
    let jobs: Vec<RealizeJob> = tracts
        .iter()
        .map(|&pos| {
            let mut around: [Option<TractRecord>; TractRecords3x3::COUNT] = Default::default();
            for (slot, (dx, dy)) in around.iter_mut().zip(TractRecords3x3::OFFSETS) {
                let near = TractPos::new(pos.x.wrapping_add(dx), pos.y.wrapping_add(dy));
                *slot = if (dx, dy) == (0, 0) {
                    Some(record(pos))
                } else {
                    inside(near).map(&mut record)
                };
            }
            RealizeJob {
                generator: Arc::clone(&generator),
                realm,
                pos,
                rec: record(pos),
                around: TractRecords3x3::new(around),
            }
        })
        .collect();
    Ok(realize_batch(&defs, generators.runner(), &jobs))
}
