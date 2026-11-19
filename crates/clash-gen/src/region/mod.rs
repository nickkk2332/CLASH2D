//! Regions (10 §8.1 to §8.3): region kinds, the kinds of path, sites,
//! and what a survey decides for a region beyond its tract definitions:
//! the paths that cross its edges and the sites its kind asks for.
//!
//! ```json
//! { "type": "region_kind", "id": "farmland",
//!   "tracts": [ { "tract": "field", "weight": 4 }, { "tract": "copse", "weight": 1 } ],
//!   "patch_size": 4,
//!   "sites": [ { "site": "farmstead", "per_region": [3, 8], "apart": 6, "near": "road" } ],
//!   "paths": { "path": "road", "per_edge": [1, 2] } }
//! ```
//!
//! # Pure by position
//!
//! What a region holds is a [`RegionPlan`]: a pure function of the
//! realm's seed, the region's position and the definitions
//! ([`StandardGenerator::region_plan`](crate::StandardGenerator::region_plan)). Neighbours agree at their shared edge
//! without reading each other (invariant X3): a path crosses an edge at
//! points drawn from the positional hash of the edge itself, and both
//! regions work out the kinds of both sides the same way.
//!
//! The steps of 10 §8.2, as built:
//!
//! 1. The region's kind, from the realm generator's `regions` (no realm
//!    plan exists yet).
//! 2. A tract definition for each tract: in patches, by weight.
//! 3. Settlements: those the realm places by hand that reach the
//!    region (see [`crate::settlement`]).
//! 4. Paths crossing its edges, each from its crossing point to the
//!    region's hub.
//! 5. Sites, by count and constraint, after the paths so that `near:
//!    road` can be honoured.
//! 6. The record.

mod def;
pub(crate) mod paths;
pub(crate) mod plan;

pub use def::{
    MinorPathsDef, PathEdgeDef, PathFloorDef, PathKindDef, PeopleDef, RegionKindDef, RegionSiteDef,
    SiteDef, WeightedTractDef,
};
pub use paths::Seg;
pub use plan::{PlannedSite, RegionPlan, SiteShortfall};

use clash_core::Real;
use clash_data::{Def, DefLabel, Defs};
use clash_world::dims::{REGION_TRACT_COUNT, REGION_TRACTS};
use clash_world::{FloorState, PathKindId, SiteDefId, TractDefId};

use crate::ground::Report;
use crate::map::{MARK_PREFIX, MapDefs, MapId, MapPickDef, SpawnWho};
use crate::settlement::SettlementId;
use crate::tractdef::TractGlyph;

/// The widest a path's surface may be, in cells.
pub const MAX_PATH_WIDTH: i32 = 15;
/// The widest the strip along each side of a path may be, in cells.
pub const MAX_PATH_EDGE: i32 = 8;
/// The most paths that may cross one edge of a region.
pub const MAX_EDGE_PATHS: u32 = 8;
/// The most people one entry may ask for.
pub const MAX_PEOPLE: u32 = 50;

/// The names `near` may take that are built.
pub const NEAR_ROAD: &str = "road";
/// The middle of the region.
pub const NEAR_CENTRE: &str = "centre";
/// The outer tracts of the region.
pub const NEAR_EDGE: &str = "edge";
/// Named choices of 10 §13 that wait for realm plans.
const NEAR_LATER: [&str; 2] = ["coast", "river"];

/// A kind of path made ready.
#[derive(Clone, Debug)]
pub struct PathKind {
    pub(crate) name: String,
    pub(crate) tags: Vec<String>,
    /// Cells across its surface.
    pub(crate) width: i32,
    pub(crate) surface: FloorState,
    /// The strip along each side: its floor and how many cells across.
    pub(crate) edge: Option<(FloorState, i32)>,
    pub(crate) glyph: Option<TractGlyph>,
}

impl PathKind {
    /// The definition's qualified id.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Its tags, sorted.
    pub fn tags(&self) -> &[String] {
        &self.tags
    }

    /// How many cells across its surface is.
    pub fn width(&self) -> i32 {
        self.width
    }

    /// How many cells across it is with the strips along its sides.
    pub fn footprint(&self) -> i32 {
        self.width + 2 * self.edge.map_or(0, |(_, width)| width)
    }

    /// How it is drawn on the region map.
    pub fn glyph(&self) -> Option<&TractGlyph> {
        self.glyph.as_ref()
    }
}

/// Where people asked for are put inside a map.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum PeoplePlace {
    /// Open floor in the areas of this name, or anywhere in the map if
    /// it has none.
    Area(String),
    /// The first cell of this mark, or anywhere if the map lacks it.
    Mark(String),
    /// Open floor anywhere in the map.
    Anywhere,
}

/// People asked for, made ready.
#[derive(Clone, Debug)]
pub struct People {
    /// What `what` the spawn requests name.
    pub who: SpawnWho,
    /// The fewest and the most.
    pub count: (u32, u32),
    /// The chance that any are made.
    pub chance: Real,
    /// Where.
    pub place: PeoplePlace,
    /// What they are there as, as written.
    pub role: Option<String>,
}

/// A site definition made ready.
#[derive(Clone, Debug)]
pub struct SiteKind {
    pub(crate) name: String,
    pub(crate) label: String,
    pub(crate) tags: Vec<String>,
    /// The maps it may stand as, each supplying every role. Empty for a
    /// founded site.
    pub(crate) maps: Vec<MapId>,
    /// Its roles, each the name of a mark.
    pub(crate) roles: Vec<(String, String)>,
    pub(crate) staff: Vec<People>,
    pub(crate) founded: bool,
}

impl SiteKind {
    /// The definition's qualified id.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Its name as shown.
    pub fn label(&self) -> &str {
        &self.label
    }

    /// Its tags, sorted.
    pub fn tags(&self) -> &[String] {
        &self.tags
    }

    /// The maps it may stand as.
    pub fn maps(&self) -> &[MapId] {
        &self.maps
    }

    /// Its roles, each with the mark that supplies it.
    pub fn roles(&self) -> &[(String, String)] {
        &self.roles
    }

    /// Who works there.
    pub fn staff(&self) -> &[People] {
        &self.staff
    }

    /// Whether it is made in play and never by generation.
    pub fn founded(&self) -> bool {
        self.founded
    }
}

/// Where a region kind's sites go (10 §13).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum NearRule {
    /// Beside a path, not on it.
    Road,
    /// In the middle half of the region.
    Centre,
    /// In the outer tracts of the region.
    Edge,
}

/// One kind of site a region holds.
#[derive(Clone, Debug)]
pub struct SiteRule {
    /// The site.
    pub site: SiteDefId,
    /// The fewest and the most in each region.
    pub per_region: (u32, u32),
    /// Tracts that must lie between two of them.
    pub apart: i32,
    /// Where they go.
    pub near: Option<NearRule>,
}

/// The paths that cross a region kind's edges.
#[derive(Clone, Copy, Debug)]
pub struct MinorPaths {
    /// Their kind.
    pub kind: PathKindId,
    /// The fewest and the most across each edge.
    pub per_edge: (u32, u32),
}

/// A region kind made ready.
#[derive(Clone, Debug)]
pub struct RegionKind {
    pub(crate) name: String,
    pub(crate) tags: Vec<String>,
    pub(crate) tracts: Vec<(TractDefId, Real)>,
    pub(crate) total: Real,
    pub(crate) patch: i32,
    pub(crate) sites: Vec<SiteRule>,
    pub(crate) settlements: Vec<SettlementId>,
    pub(crate) paths: Option<MinorPaths>,
    pub(crate) weather: Option<String>,
    pub(crate) glyph: Option<TractGlyph>,
}

impl RegionKind {
    /// The definition's qualified id.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Its tags, sorted.
    pub fn tags(&self) -> &[String] {
        &self.tags
    }

    /// The sites it holds.
    pub fn sites(&self) -> &[SiteRule] {
        &self.sites
    }

    /// The settlements a realm plan may put in it.
    pub fn settlements(&self) -> &[SettlementId] {
        &self.settlements
    }

    /// The paths that cross its edges.
    pub fn paths(&self) -> Option<MinorPaths> {
        self.paths
    }

    /// Its climate, as written.
    pub fn weather(&self) -> Option<&str> {
        self.weather.as_deref()
    }

    /// How it is drawn on the realm map.
    pub fn glyph(&self) -> Option<&TractGlyph> {
        self.glyph.as_ref()
    }

    /// The tract definition of a tract of a region of this kind: one of
    /// its list by weight, drawn from the positional hash of the
    /// tract's patch.
    pub(crate) fn tract_def(
        &self,
        seed: u64,
        domain: clash_core::HashDomain,
        x: i32,
        y: i32,
    ) -> TractDefId {
        if self.tracts.is_empty() || self.total <= Real::ZERO {
            return TractDefId::new(0);
        }
        let patch = (x.div_euclid(self.patch), y.div_euclid(self.patch));
        let mut left =
            clash_core::pos_rng(seed, domain, patch.0, patch.1, 1, 0).unit() * self.total;
        for (def, weight) in &self.tracts {
            if left < *weight {
                return *def;
            }
            left -= *weight;
        }
        self.tracts
            .last()
            .map_or(TractDefId::new(0), |(def, _)| *def)
    }
}

/// The namespace of a qualified id.
pub(crate) fn namespace_of(name: &str) -> &str {
    name.split_once(':').map_or("", |(namespace, _)| namespace)
}

/// Reads a count written as a number or `[low, high]`, each from 0 to
/// `most`.
pub(crate) fn read_count(
    report: &mut Report<'_>,
    label: &DefLabel,
    value: &crate::map::Sited<serde_json::Value>,
    most: u32,
) -> Option<(u32, u32)> {
    let in_range = |n: &serde_json::Value| {
        n.as_u64()
            .and_then(|n| u32::try_from(n).ok())
            .filter(|n| *n <= most)
    };
    let read = match &value.value {
        serde_json::Value::Array(pair) => match pair.as_slice() {
            [low, high] => in_range(low)
                .zip(in_range(high))
                .filter(|(low, high)| low <= high),
            _ => None,
        },
        number => in_range(number).map(|n| (n, n)),
    };
    if read.is_none() {
        report.error(
            label,
            &value.site,
            format!(
                "a count is a whole number or `[low, high]`, each from 0 to {most}, `low` first"
            ),
        );
    }
    read
}

/// Builds people asked for by a site, a zone or a lot.
pub(crate) fn build_people(
    report: &mut Report<'_>,
    label: &DefLabel,
    defs: &[crate::region::PeopleDef],
) -> Vec<People> {
    let mut people = Vec::new();
    for def in defs {
        let count = match &def.count {
            None => (1, 1),
            Some(count) => match read_count(report, label, count, MAX_PEOPLE) {
                Some(count) => count,
                None => continue,
            },
        };
        let chance = match &def.chance {
            None => Real::ONE,
            Some(chance) => match Real::new(chance.value) {
                Ok(chance) if (Real::ZERO..=Real::ONE).contains(&chance) => chance,
                _ => {
                    report.error(label, &chance.site, "a chance is from 0 to 1");
                    continue;
                }
            },
        };
        let place = match (&def.area, &def.at) {
            (Some(_), Some(at)) => {
                report.error(
                    label,
                    &at.site,
                    "people go `in` an area or `at` a mark, not both",
                );
                continue;
            }
            (Some(area), None) => PeoplePlace::Area(
                area.value
                    .strip_prefix(crate::map::AREA_PREFIX)
                    .unwrap_or(&area.value)
                    .to_owned(),
            ),
            (None, Some(at)) => match at.value.strip_prefix(MARK_PREFIX) {
                Some(mark) if !mark.is_empty() => PeoplePlace::Mark(mark.to_owned()),
                _ => {
                    report.error(label, &at.site, "`at` names a mark: `\"mark:name\"`");
                    continue;
                }
            },
            (None, None) => PeoplePlace::Anywhere,
        };
        if def.creature.value.is_empty() {
            report.error(label, &def.creature.site, "`creature` names what to make");
            continue;
        }
        people.push(People {
            who: SpawnWho::Creature(def.creature.value.clone()),
            count,
            chance,
            place,
            role: def.role.clone(),
        });
    }
    people
}

fn tags_of(defs: &Defs, tags: &clash_data::TagSet) -> Vec<String> {
    let mut names: Vec<String> = tags
        .iter()
        .map(|tag| defs.tags().name(tag).to_owned())
        .collect();
    names.sort();
    names
}

/// Builds a kind of path against the world's tables.
pub(crate) fn build_path_kind(report: &mut Report<'_>, name: &str, def: &PathKindDef) -> PathKind {
    let label = DefLabel::new(PathKindDef::TYPE, name);
    let namespace = namespace_of(name);
    let width = match i32::try_from(def.width.value) {
        Ok(width) if (1..=MAX_PATH_WIDTH).contains(&width) => width,
        _ => {
            report.error(
                &label,
                &def.width.site,
                format!("`width` is from 1 to {MAX_PATH_WIDTH} cells"),
            );
            1
        }
    };
    let surface = report
        .floor(&label, namespace, &def.surface.floor)
        .unwrap_or(FloorState::NONE);
    let edge = def.edge.as_ref().and_then(|edge| {
        let floor = report.floor(&label, namespace, &edge.floor)?;
        let width = match &edge.width {
            None => 1,
            Some(width) => match i32::try_from(width.value) {
                Ok(width) if (1..=MAX_PATH_EDGE).contains(&width) => width,
                _ => {
                    report.error(
                        &label,
                        &width.site,
                        format!("an edge is from 1 to {MAX_PATH_EDGE} cells across"),
                    );
                    return None;
                }
            },
        };
        Some((floor, width))
    });
    PathKind {
        name: name.to_owned(),
        tags: tags_of(report.defs, &def.tags),
        width,
        surface,
        edge,
        glyph: def.glyph.clone(),
    }
}

/// The marks a map supplies in every placement.
fn supplies(maps: &MapDefs, map: MapId, mark: &str) -> bool {
    maps.get(map)
        .is_some_and(|compiled| compiled.marks().contains_key(mark))
}

/// Builds a site definition against the maps (invariant X7).
pub(crate) fn build_site(
    report: &mut Report<'_>,
    maps: &MapDefs,
    name: &str,
    def: &SiteDef,
) -> SiteKind {
    let label = DefLabel::new(SiteDef::TYPE, name);
    let mut roles = Vec::new();
    for (role, mark) in &def.roles {
        match mark.value.strip_prefix(MARK_PREFIX) {
            Some(mark) if !mark.is_empty() => roles.push((role.clone(), mark.to_owned())),
            _ => report.error(
                &label,
                &mark.site,
                format!("the role `{role}` names a mark of the site's map: `\"mark:name\"`"),
            ),
        }
    }
    let mut eligible = Vec::new();
    match &def.map {
        None if !def.founded => {
            if let Some((_, mark)) = def.roles.iter().next() {
                report.error(
                    &label,
                    &mark.site,
                    "a site made by generation names its `map`; one made in play says \
                     `\"founded\": true`",
                );
            }
        }
        None => {}
        Some(MapPickDef::Id(map, site)) => {
            let map = map.id();
            let mut fits = true;
            for (role, mark) in &roles {
                if !supplies(maps, map, mark) {
                    fits = false;
                    report.error(
                        &label,
                        site,
                        format!(
                            "the map `{}` has no mark `{mark}` for the role `{role}` (invariant \
                             X7): a role is a mark that is the same in every placement",
                            maps.name(map)
                        ),
                    );
                }
            }
            if maps
                .get(map)
                .is_some_and(|compiled| compiled.front().is_none())
            {
                fits = false;
                report.error(
                    &label,
                    site,
                    format!(
                        "the map `{}` names no `front`: a site's map is turned to face its street",
                        maps.name(map)
                    ),
                );
            }
            if fits {
                eligible.push(map);
            }
        }
        Some(MapPickDef::Tag(tag, site)) => {
            eligible = maps
                .with_tag(tag)
                .filter(|map| {
                    maps.get(*map)
                        .is_some_and(|compiled| compiled.front().is_some())
                        && roles.iter().all(|(_, mark)| supplies(maps, *map, mark))
                })
                .collect();
            if eligible.is_empty() {
                report.error(
                    &label,
                    site,
                    format!(
                        "no map carrying the tag `{tag}` names a `front` and supplies every role \
                         (invariant X7)"
                    ),
                );
            }
        }
    }
    if def.founded
        && let Some(MapPickDef::Id(_, site) | MapPickDef::Tag(_, site)) = &def.map
    {
        report.error(
            &label,
            site,
            "a founded site is made in play and has no map",
        );
    }
    SiteKind {
        name: name.to_owned(),
        label: def
            .name
            .clone()
            .unwrap_or_else(|| name.rsplit(':').next().unwrap_or(name).to_owned()),
        tags: tags_of(report.defs, &def.tags),
        maps: eligible,
        roles,
        staff: build_people(report, &label, &def.staff),
        founded: def.founded,
    }
}

/// Builds a region kind.
pub(crate) fn build_region_kind(
    report: &mut Report<'_>,
    sites: &[SiteKind],
    name: &str,
    def: &RegionKindDef,
) -> RegionKind {
    let label = DefLabel::new(RegionKindDef::TYPE, name);
    let mut tracts = Vec::new();
    for entry in &def.tracts {
        let weight = match &entry.weight {
            None => Real::ONE,
            Some(weight) => match Real::new(weight.value) {
                Ok(weight) if weight >= Real::ZERO && weight.is_finite() => weight,
                _ => {
                    report.error(
                        &label,
                        &weight.site,
                        "a weight is a number that is not negative",
                    );
                    continue;
                }
            },
        };
        tracts.push((entry.tract.id(), weight));
    }
    let total = tracts
        .iter()
        .fold(Real::ZERO, |sum, (_, weight)| sum + *weight);
    let patch = match &def.patch_size {
        None => 1,
        Some(patch) => match i32::try_from(patch.value) {
            Ok(patch) if (1..=REGION_TRACTS).contains(&patch) => patch,
            _ => {
                report.error(
                    &label,
                    &patch.site,
                    format!("`patch_size` is from 1 to {REGION_TRACTS} tracts"),
                );
                1
            }
        },
    };
    let mut rules = Vec::new();
    for entry in &def.sites {
        let site = entry.site.id();
        if sites.get(site.index()).is_some_and(SiteKind::founded) {
            report.error(
                &label,
                &entry.per_region.site,
                "a founded site is made in play and cannot be placed by a survey",
            );
            continue;
        }
        let Some(per_region) = read_count(
            report,
            &label,
            &entry.per_region,
            u32::try_from(REGION_TRACT_COUNT).unwrap_or(u32::MAX),
        ) else {
            continue;
        };
        let apart = match &entry.apart {
            None => 0,
            Some(apart) => match i32::try_from(apart.value) {
                Ok(apart) if (0..=REGION_TRACTS).contains(&apart) => apart,
                _ => {
                    report.error(
                        &label,
                        &apart.site,
                        format!("`apart` is from 0 to {REGION_TRACTS} tracts"),
                    );
                    continue;
                }
            },
        };
        let near = match &entry.near {
            None => None,
            Some(near) => match near.value.as_str() {
                NEAR_ROAD => Some(NearRule::Road),
                NEAR_CENTRE => Some(NearRule::Centre),
                NEAR_EDGE => Some(NearRule::Edge),
                later if NEAR_LATER.contains(&later) => {
                    report.error(
                        &label,
                        &near.site,
                        format!(
                            "`near: {later}` waits for realm plans, which give coasts and rivers \
                             (WG6); `road`, `centre` and `edge` are built"
                        ),
                    );
                    continue;
                }
                other => {
                    report.error(
                        &label,
                        &near.site,
                        format!("`near` is `road`, `centre` or `edge`, not `{other}`"),
                    );
                    continue;
                }
            },
        };
        rules.push(SiteRule {
            site,
            per_region,
            apart,
            near,
        });
    }
    let paths = def.paths.as_ref().and_then(|paths| {
        let per_edge = match &paths.per_edge {
            None => (1, 1),
            Some(count) => read_count(report, &label, count, MAX_EDGE_PATHS)?,
        };
        Some(MinorPaths {
            kind: paths.path.id(),
            per_edge,
        })
    });
    RegionKind {
        name: name.to_owned(),
        tags: tags_of(report.defs, &def.tags),
        tracts,
        total,
        patch,
        sites: rules,
        settlements: def
            .settlements
            .iter()
            .map(|settlement| settlement.id())
            .collect(),
        paths,
        weather: def.weather.clone(),
        glyph: def.glyph.clone(),
    }
}
