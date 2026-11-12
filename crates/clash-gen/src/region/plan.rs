//! What a survey decides for a region beyond its tract definitions: the
//! paths that cross its edges and the sites its kind asks for
//! (10 §8.2, §8.3). A pure function of the realm's seed, the region's
//! position and the definitions.

use clash_core::{HashDomain, Rot4, Turn, pos_hash, pos_rng};
use clash_world::dims::{REGION_CELLS, REGION_TRACT_COUNT, TRACT_XY};
use clash_world::{
    CellBox, CellPos, Placement, RealmInfo, RegionKindId, RegionPos, SiteDefId, TractPos,
};

use super::{NearRule, Seg};
use crate::map::turned_size;
use crate::place::{boxes_overlap, placement_box, world_map_id};
use crate::realize::StandardGenerator;

/// The domain of how many paths cross an edge, and where.
const EDGE_DOMAIN: HashDomain = HashDomain::new("generator.region.edge");
/// The domain of where a region's paths meet when it holds no
/// settlement.
const HUB_DOMAIN: HashDomain = HashDomain::new("generator.region.hub");
/// The domain of how many sites of a kind a region holds.
const SITE_COUNT_DOMAIN: HashDomain = HashDomain::new("generator.region.site_count");
/// The domain of the order tracts are tried for a site, and its map,
/// turn and seed.
const SITE_DOMAIN: HashDomain = HashDomain::new("generator.region.site");

/// How close to a corner of a region a path may cross its edge, in
/// cells: an eighth of the region's side.
const EDGE_MARGIN: i32 = REGION_CELLS / 8;

/// A site a survey places: from a region kind, or a settlement's
/// `must_have`.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct PlannedSite {
    /// Its definition.
    pub def: SiteDefId,
    /// Its map, placed.
    pub placement: Placement,
    /// The box the map fills.
    pub bounds: CellBox,
    /// The tracts it takes: every tract the map covers.
    pub tracts: Vec<TractPos>,
    /// The cell that stands for it: where its map's anchor goes.
    pub anchor: CellPos,
}

/// Fewer sites of a kind than a region kind asks for: there was not
/// room for more.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct SiteShortfall {
    /// The site.
    pub site: SiteDefId,
    /// How many were asked for.
    pub wanted: u32,
    /// How many were placed.
    pub placed: u32,
}

/// What a survey decides for one region beyond its tracts' definitions.
#[derive(Clone, Debug, Default)]
pub struct RegionPlan {
    /// Its kind, if the realm gives regions kinds.
    pub kind: Option<RegionKindId>,
    /// Where its paths meet: the centre of a settlement there, or a
    /// cell its seed chooses.
    pub hub: (i32, i32),
    /// The paths that cross its edges, each from where it crosses to
    /// the hub. A stretch inside a settlement is not laid.
    pub paths: Vec<Seg>,
    /// The sites its kind asks for.
    pub sites: Vec<PlannedSite>,
    /// Kinds of site of which fewer were placed than asked for.
    pub shortfalls: Vec<SiteShortfall>,
}

/// Whether a region is one of the realm's: inside its extent, and not
/// reached across a wrapped edge.
fn exists(realm: &RealmInfo, region: RegionPos) -> bool {
    let origin = region.origin().origin(realm.topology.z_min);
    realm
        .topology
        .normalize(origin)
        .is_ok_and(|cell| cell == origin)
}

/// The paths crossing the edge between two neighbouring regions, `a`
/// west or north of `b`: where each crosses, along the edge, and its
/// kind. Both regions work it out alike.
fn crossings(
    generator: &StandardGenerator,
    realm: &RealmInfo,
    a: RegionPos,
    b: RegionPos,
) -> Vec<(i32, clash_world::PathKindId)> {
    if !exists(realm, a) || !exists(realm, b) {
        return Vec::new();
    }
    let paths = |region: RegionPos| {
        generator
            .region_kind(realm.seed, region)
            .and_then(|kind| generator.defs().region_kind(kind))
            .and_then(|kind| kind.paths)
    };
    let (Some(west), Some(east)) = (paths(a), paths(b)) else {
        return Vec::new();
    };
    let across_x = b.x != a.x;
    let mut rng = pos_rng(realm.seed, EDGE_DOMAIN, b.x, b.y, i32::from(across_x), 0);
    let low = west.per_edge.0.min(east.per_edge.0);
    let high = west.per_edge.1.min(east.per_edge.1).max(low);
    let count = rng.range(i64::from(low), i64::from(high));
    // Along the edge: y for an edge between west and east, x for one
    // between north and south.
    let start = if across_x {
        b.origin().origin(0).y
    } else {
        b.origin().origin(0).x
    };
    (0..count)
        .map(|_| {
            let along = rng.range(
                i64::from(EDGE_MARGIN),
                i64::from(REGION_CELLS - EDGE_MARGIN - 1),
            ) as i32;
            (start + along, west.kind)
        })
        .collect()
}

/// Lays out what a region holds besides its tracts' definitions.
pub(crate) fn plan_region(
    generator: &StandardGenerator,
    realm: &RealmInfo,
    region: RegionPos,
) -> RegionPlan {
    let seed = realm.seed;
    let kind_id = generator.region_kind(seed, region);
    let first = region.origin().origin(0);
    let (rx0, ry0) = (first.x, first.y);
    let (rx1, ry1) = (rx0 + REGION_CELLS - 1, ry0 + REGION_CELLS - 1);

    // Where the paths meet: the centre of the first settlement whose
    // centre lies in the region, or a cell in its middle half.
    let hub = generator
        .settlement_plans(seed)
        .into_iter()
        .map(|plan| plan.centre)
        .find(|(x, y)| (rx0..=rx1).contains(x) && (ry0..=ry1).contains(y))
        .unwrap_or_else(|| {
            let mut rng = pos_rng(seed, HUB_DOMAIN, region.x, region.y, 0, 0);
            let quarter = REGION_CELLS / 4;
            (
                rx0 + quarter + rng.range(0, i64::from(2 * quarter - 1)) as i32,
                ry0 + quarter + rng.range(0, i64::from(2 * quarter - 1)) as i32,
            )
        });

    // The paths: from each crossing of each of the four edges to the
    // hub, first straight in from the edge, then along to it.
    let mut paths = Vec::new();
    let north = RegionPos::new(region.x, region.y - 1);
    let south = RegionPos::new(region.x, region.y + 1);
    let west = RegionPos::new(region.x - 1, region.y);
    let east = RegionPos::new(region.x + 1, region.y);
    let mut leg = |from: (i32, i32), bend: (i32, i32), kind| {
        paths.push(Seg {
            kind,
            from,
            to: bend,
            minor: true,
        });
        paths.push(Seg {
            kind,
            from: bend,
            to: hub,
            minor: true,
        });
    };
    for (x, kind) in crossings(generator, realm, north, region) {
        leg((x, ry0), (x, hub.1), kind);
    }
    for (x, kind) in crossings(generator, realm, region, south) {
        leg((x, ry1), (x, hub.1), kind);
    }
    for (y, kind) in crossings(generator, realm, west, region) {
        leg((rx0, y), (hub.0, y), kind);
    }
    for (y, kind) in crossings(generator, realm, region, east) {
        leg((rx1, y), (hub.0, y), kind);
    }

    let mut plan = RegionPlan {
        kind: kind_id,
        hub,
        paths,
        sites: Vec::new(),
        shortfalls: Vec::new(),
    };
    let Some(kind) = kind_id.and_then(|id| generator.defs().region_kind(id)) else {
        return plan;
    };

    // The sites, kind by kind.
    let settlements = generator.settlement_plans(seed);
    let footprint_of = |seg: &Seg| {
        generator
            .defs()
            .path_kind(seg.kind)
            .map_or(1, |path| path.footprint())
    };
    let near_path = |x0: i32, y0: i32, x1: i32, y1: i32, margin: i32| {
        plan.paths.iter().any(|seg| {
            let (px0, py0, px1, py1) = seg.reach(footprint_of(seg));
            px1 >= x0 - margin && px0 <= x1 + margin && py1 >= y0 - margin && py0 <= y1 + margin
        })
    };
    let mut sites: Vec<PlannedSite> = Vec::new();
    let mut shortfalls = Vec::new();
    for (index, rule) in kind.sites.iter().enumerate() {
        let Some(site) = generator.defs().site(rule.site) else {
            continue;
        };
        let wanted = pos_rng(seed, SITE_COUNT_DOMAIN, region.x, region.y, 0, index as u64)
            .range(i64::from(rule.per_region.0), i64::from(rule.per_region.1))
            as u32;
        let mut order: Vec<TractPos> = (0..REGION_TRACT_COUNT).map(|at| region.tract(at)).collect();
        order.sort_by_key(|tract| {
            (
                pos_hash(seed, SITE_DOMAIN, tract.x, tract.y, 0, index as u64),
                tract.y,
                tract.x,
            )
        });
        let mut placed: Vec<TractPos> = Vec::new();
        for tract in order {
            if placed.len() as u32 >= wanted {
                break;
            }
            let origin = tract.origin(0);
            let (tx0, ty0) = (origin.x, origin.y);
            let (tx1, ty1) = (tx0 + TRACT_XY - 1, ty0 + TRACT_XY - 1);
            // Not in a settlement, nor in a tract beside one.
            if settlements
                .iter()
                .any(|plan| plan.near(tx0, ty0, tx1, ty1, TRACT_XY))
            {
                continue;
            }
            if sites.iter().any(|other| other.tracts.contains(&tract))
                || placed.iter().any(|other| {
                    (other.x - tract.x).abs().max((other.y - tract.y).abs()) <= rule.apart
                })
            {
                continue;
            }
            let local = (tract.x - region.origin().x, tract.y - region.origin().y);
            let side = clash_world::dims::REGION_TRACTS;
            let ring = local
                .0
                .min(local.1)
                .min(side - 1 - local.0)
                .min(side - 1 - local.1);
            let fits_near = match rule.near {
                None => true,
                Some(NearRule::Centre) => ring >= side / 4,
                Some(NearRule::Edge) => ring < side / 8,
                Some(NearRule::Road) => near_path(tx0, ty0, tx1, ty1, TRACT_XY),
            };
            // Never on a path.
            if !fits_near || near_path(tx0, ty0, tx1, ty1, 1) {
                continue;
            }
            let mut rng = pos_rng(seed, SITE_DOMAIN, tract.x, tract.y, 1, index as u64);
            let Some(&map) = rng.pick(&site.maps) else {
                continue;
            };
            let Some(compiled) = generator.defs().maps().get(map) else {
                continue;
            };
            let rot = if compiled.may_turn() {
                Rot4::from_quarter_turns(rng.range(0, 3) as i32)
            } else {
                Rot4::R0
            };
            let turn = Turn::new(rot, false);
            let size = turned_size(compiled.size(), turn);
            if size.x > TRACT_XY || size.y > TRACT_XY {
                continue;
            }
            let (cx, cy) = (tx0 + (TRACT_XY - size.x) / 2, ty0 + (TRACT_XY - size.y) / 2);
            let flat = crate::settlement::layout::origin_at(compiled, turn, cx, cy, 0);
            let level = generator.base_level(seed, compiled.base(), flat.x, flat.y);
            let placement = Placement {
                map: world_map_id(map),
                origin: crate::settlement::layout::origin_at(compiled, turn, cx, cy, level),
                turn,
                params: clash_core::VarMap::new(),
                seed: pos_hash(seed, SITE_DOMAIN, tract.x, tract.y, 2, index as u64),
                order: 0,
                site: None,
            };
            let Some(bounds) = placement_box(generator.defs().maps(), &placement) else {
                continue;
            };
            // Nothing the tract's definition or the realm places may be
            // met (invariant X10).
            let own = generator
                .placements_of(seed, tract, generator.tract_def(seed, tract))
                .0;
            let whole = CellBox::new(
                CellPos::new(tx0, ty0, i32::MIN),
                CellPos::new(tx1, ty1, i32::MAX),
            );
            let meets = own
                .iter()
                .chain(generator.authored_in(seed, whole).iter())
                .filter_map(|other| placement_box(generator.defs().maps(), other))
                .any(|other| boxes_overlap(bounds, other));
            if meets {
                continue;
            }
            placed.push(tract);
            sites.push(PlannedSite {
                def: rule.site,
                anchor: placement.origin,
                tracts: bounds.tracts().collect(),
                placement,
                bounds,
            });
        }
        if (placed.len() as u32) < wanted {
            shortfalls.push(SiteShortfall {
                site: rule.site,
                wanted,
                placed: placed.len() as u32,
            });
        }
    }
    plan.sites = sites;
    plan.shortfalls = shortfalls;
    plan
}
