//! The `settlement` definition type as a mod writes it (10 §8.4).

use clash_core::define_id;
use clash_data::{Def, FromSite, Kind, KindValue, Ref, Site, TagSet, by_site};
use serde::de::MapAccess;
use serde::{Deserialize, Deserializer};

use crate::map::Sited;
use crate::region::{PathKindDef, PeopleDef, SiteDef};
use crate::tractdef::TractDef;

define_id! {
    /// A `settlement` definition.
    pub struct SettlementId(u16);
}

/// How a settlement's streets are laid out.
#[derive(Clone, Debug, Deserialize)]
pub struct StreetsDef {
    /// `grid` or `organic`.
    pub pattern: Sited<String>,
    /// `[low, high]`: cells from one street to the next.
    pub spacing: Sited<Vec<i64>>,
    /// The kind of path its streets are.
    pub path: Ref<PathKindDef>,
    /// How many main roads cross it through its centre: 0, 1 (east to
    /// west) or 2 (and north to south).
    #[serde(default)]
    pub main: Option<Sited<i64>>,
    /// The kind of path its main roads are. Its `path` if left out.
    #[serde(default)]
    pub main_path: Option<Ref<PathKindDef>>,
}

/// A ring of a settlement by distance from its centre.
#[derive(Clone, Debug, Deserialize)]
pub struct ZoneDef {
    /// Its name.
    pub zone: String,
    /// How far out it reaches, as a share of the way from the centre to
    /// the edge, from 0 to 1.
    pub to: Sited<f64>,
    /// The tags of the maps that may fill its lots.
    pub lots: Sited<Vec<String>>,
    /// Who lives or works in each of its lots.
    #[serde(default)]
    pub people: Vec<PeopleDef>,
}

/// A site every instance of a settlement has.
#[derive(Clone, Debug)]
pub struct MustHaveDef {
    /// The site definition.
    pub site: Ref<SiteDef>,
    /// Where: `centre`, `edge` or `anywhere`.
    pub at: Option<Sited<String>>,
}

/// A site every instance has, written as an object: the site and
/// where it goes.
#[derive(Deserialize)]
struct MustHaveObject {
    /// The site definition every instance has. Required.
    site: Ref<SiteDef>,
    /// Where: `centre`, `edge` or `anywhere`. Left out, anywhere.
    #[serde(default)]
    at: Option<Sited<String>>,
}

impl<'de> FromSite<'de> for MustHaveDef {
    fn from_site<A: MapAccess<'de>>(
        _site: Site,
        kind: Kind,
        value: KindValue<'_, A>,
    ) -> Result<MustHaveDef, A::Error> {
        match kind {
            Kind::String => Ok(MustHaveDef {
                site: value.read::<Ref<SiteDef>>()?,
                at: None,
            }),
            Kind::Object => {
                let object = value.read::<MustHaveObject>()?;
                Ok(MustHaveDef {
                    site: object.site,
                    at: object.at,
                })
            }
            other => Err(other.unexpected("a site id or `{ \"site\": id, \"at\": where }`")),
        }
    }
}

impl<'de> Deserialize<'de> for MustHaveDef {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<MustHaveDef, D::Error> {
        by_site(deserializer)
    }
}

/// A town, a village, a camp: streets, lots, and what fills them
/// (10 §8.4).
#[derive(Debug, Deserialize)]
pub struct SettlementDef {
    /// Free-form tags.
    #[serde(default)]
    pub tags: TagSet,
    /// `[width, depth]`: its extent in tracts, each from 1 to 64.
    pub size: Sited<Vec<i64>>,
    /// The tract definition its tracts are given. The region's choice
    /// if left out.
    #[serde(default)]
    pub tract: Option<Ref<TractDef>>,
    /// Its streets.
    pub streets: StreetsDef,
    /// Cells left between two buildings side by side, from 0 to 8.
    #[serde(default)]
    pub gap: Option<Sited<i64>>,
    /// Its rings, from the centre out.
    pub zones: Vec<ZoneDef>,
    /// Sites every instance has.
    #[serde(default)]
    pub must_have: Vec<MustHaveDef>,
    /// A wall round it. Carried: walls are not built.
    #[serde(default)]
    pub wall: Option<Sited<serde_json::Value>>,
    /// The groups that exist here (10 §11.3). Carried for WG9.
    #[serde(default)]
    pub groups: Option<serde_json::Value>,
    /// How many people live here. Carried for WG9.
    #[serde(default)]
    pub population: Option<serde_json::Value>,
    /// The mark that counts as a home's bed (10 §13). Carried for WG9.
    #[serde(default)]
    pub home_mark: Option<String>,
}

impl Def for SettlementDef {
    type Id = SettlementId;
    const TYPE: &'static str = "settlement";
    const DOC: &'static str = "A town, a village or a camp: a pattern of streets, the lots \
                               between them filled by maps chosen by zone, and the sites every \
                               instance has (10 §8.4).";
}
