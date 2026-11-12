//! The definition types of regions as a mod writes them: `path_kind`
//! (10 §8.5), `site` (10 §11.1, the part generation needs) and
//! `region_kind` (10 §8.1).
//!
//! These structs are what the loader reads, with where each value was
//! written. [`GenDefs::from_defs`](crate::GenDefs::from_defs) builds
//! them against the world's tables and the maps, and reports each
//! mistake at the value that is wrong.

use std::collections::BTreeMap;

use clash_data::{Def, Ref, TagSet};
use clash_world::{PathKindId, RegionKindId, SiteDefId};
use serde::Deserialize;

use crate::map::{MapPickDef, Sited};
use crate::settlement::SettlementDef;
use crate::tractdef::{TractDef, TractGlyph};

/// A floor laid along a path: the floor state, as written.
#[derive(Clone, Debug, Deserialize)]
pub struct PathFloorDef {
    /// The floor state.
    pub floor: Sited<String>,
}

/// The strip laid along each side of a path, outside its surface.
#[derive(Clone, Debug, Deserialize)]
pub struct PathEdgeDef {
    /// The floor state of the strip.
    pub floor: Sited<String>,
    /// How many cells across each strip is, from 1 to 8.
    #[serde(default)]
    pub width: Option<Sited<i64>>,
}

/// What a path is made of (10 §8.5): a road, a track, a street.
#[derive(Debug, Deserialize)]
pub struct PathKindDef {
    /// Free-form tags.
    #[serde(default)]
    pub tags: TagSet,
    /// How many cells across its surface is, from 1 to 15.
    pub width: Sited<i64>,
    /// The floor laid along it.
    pub surface: PathFloorDef,
    /// A strip of another floor along each side: a pavement.
    #[serde(default)]
    pub edge: Option<PathEdgeDef>,
    /// The steepest the path follows the ground, in storeys per cell.
    /// Carried: crossings are not built.
    #[serde(default)]
    pub max_grade: Option<Sited<f64>>,
    /// What it does where it meets liquid, another path or a steep
    /// rise. Carried as written: crossings are not built.
    #[serde(default)]
    pub crossings: Option<Sited<serde_json::Value>>,
    /// For a watercourse, the speed its water moves along its line, in
    /// metres a second. Carried.
    #[serde(default)]
    pub current: Option<Sited<f64>>,
    /// How it is drawn on the region map.
    #[serde(default)]
    pub glyph: Option<TractGlyph>,
}

impl Def for PathKindDef {
    type Id = PathKindId;
    const TYPE: &'static str = "path_kind";
    const DOC: &'static str = "What a path is made of: how wide it is and the floor laid along \
                               it, with a strip of another floor along each side (10 §8.5).";
}

/// People a site, a lot or a zone asks to be put somewhere (10 §11.2):
/// `{ "creature": id, "count": [1, 3], "in": "home" }`.
#[derive(Clone, Debug, Deserialize)]
pub struct PeopleDef {
    /// What to make: the id of a template or a creature, as written.
    /// The simulation answers the request.
    pub creature: Sited<String>,
    /// How many: a number or `[low, high]`. One if left out.
    #[serde(default)]
    pub count: Option<Sited<serde_json::Value>>,
    /// The chance that any are made, from 0 to 1.
    #[serde(default)]
    pub chance: Option<Sited<f64>>,
    /// The name of an area of the map they go in.
    #[serde(rename = "in", default)]
    pub area: Option<Sited<String>>,
    /// In place of `in`: a mark of the map, `"mark:counter"`.
    #[serde(default)]
    pub at: Option<Sited<String>>,
    /// What they are there as: `resident`, `staff`. Carried for the
    /// simulation.
    #[serde(default)]
    pub role: Option<String>,
}

/// A named place with identity (10 §11.1): what generation needs of it.
#[derive(Debug, Deserialize)]
pub struct SiteDef {
    /// Free-form tags.
    #[serde(default)]
    pub tags: TagSet,
    /// Its name, as shown. Its id if left out.
    #[serde(default)]
    pub name: Option<String>,
    /// The map that stands there: an id, or `{ "tag": … }`.
    #[serde(default)]
    pub map: Option<MapPickDef>,
    /// Named places inside it, each a mark of its map: `"mark:desk"`.
    #[serde(default)]
    pub roles: BTreeMap<String, Sited<String>>,
    /// Who works there.
    #[serde(default)]
    pub staff: Vec<PeopleDef>,
    /// Made in play, not by generation: it needs no map.
    #[serde(default)]
    pub founded: bool,
    /// The group or character that holds it. Carried for WG9.
    #[serde(default)]
    pub owner: Option<String>,
    /// Carried for WG9.
    #[serde(default)]
    pub stocks: Option<serde_json::Value>,
    /// Carried for WG9.
    #[serde(default)]
    pub supplies: Option<serde_json::Value>,
    /// Carried for WG9.
    #[serde(default)]
    pub incidents: Vec<String>,
    /// Carried: buildings are WG10's.
    #[serde(default)]
    pub building: Option<String>,
    /// Carried: assemblies are WG10's.
    #[serde(default)]
    pub assembly: Option<String>,
}

impl Def for SiteDef {
    type Id = SiteDefId;
    const TYPE: &'static str = "site";
    const DOC: &'static str = "A named place with identity: the map that stands there, its roles \
                               from the map's marks, and who works there (10 §11.1).";
}

/// A tract definition and how often a region of a kind is made of it.
#[derive(Clone, Debug, Deserialize)]
pub struct WeightedTractDef {
    /// The tract definition.
    pub tract: Ref<TractDef>,
    /// Its share, a number that is not negative. One if left out.
    #[serde(default)]
    pub weight: Option<Sited<f64>>,
}

/// One kind of site a region holds, with its count and constraints.
#[derive(Clone, Debug, Deserialize)]
pub struct RegionSiteDef {
    /// The site definition.
    pub site: Ref<SiteDef>,
    /// How many in each region: a number or `[low, high]`.
    pub per_region: Sited<serde_json::Value>,
    /// How many tracts must lie between two of them. None if left out.
    #[serde(default)]
    pub apart: Option<Sited<i64>>,
    /// Where they go: `road`, `centre` or `edge` (10 §13).
    #[serde(default)]
    pub near: Option<Sited<String>>,
}

/// Paths that cross the edges of regions of a kind (10 §8.3).
#[derive(Clone, Debug, Deserialize)]
pub struct MinorPathsDef {
    /// The kind of path.
    pub path: Ref<PathKindDef>,
    /// How many cross each edge: a number or `[low, high]`.
    #[serde(default)]
    pub per_edge: Option<Sited<serde_json::Value>>,
}

/// The recipe for surveying a region (10 §8.1).
#[derive(Debug, Deserialize)]
pub struct RegionKindDef {
    /// Free-form tags.
    #[serde(default)]
    pub tags: TagSet,
    /// The tract definitions its ground is made of, by weight.
    #[serde(default)]
    pub tracts: Vec<WeightedTractDef>,
    /// Tracts along the side of a patch, from 1 to 64. Not `patch`,
    /// which the loader keeps for merging (04 §6.3).
    #[serde(default)]
    pub patch_size: Option<Sited<i64>>,
    /// The sites it holds.
    #[serde(default)]
    pub sites: Vec<RegionSiteDef>,
    /// The settlements a realm plan may put in it.
    #[serde(default)]
    pub settlements: Vec<Ref<SettlementDef>>,
    /// The paths that cross its edges.
    #[serde(default)]
    pub paths: Option<MinorPathsDef>,
    /// Its climate (09 §14.1), by id as written. Carried.
    #[serde(default)]
    pub weather: Option<String>,
    /// People outside any site, by population and density. Carried.
    #[serde(default)]
    pub population: BTreeMap<String, f64>,
    /// How it is drawn on the realm map.
    #[serde(default)]
    pub glyph: Option<TractGlyph>,
}

impl Def for RegionKindDef {
    type Id = RegionKindId;
    const TYPE: &'static str = "region_kind";
    const DOC: &'static str = "The recipe for surveying a region: its tract definitions in \
                               patches, the sites it holds, the settlements it may hold and the \
                               paths that cross its edges (10 §8.1).";
}
