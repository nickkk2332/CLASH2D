//! Settlements (10 §8.4): streets, blocks between them, lots along the
//! blocks' fronts, and the maps and sites that fill the lots.
//!
//! ```json
//! { "type": "settlement", "id": "town",
//!   "size": [8, 8], "tract": "town_lot",
//!   "streets": { "pattern": "grid", "spacing": [26, 30], "path": "street",
//!                "main": 2, "main_path": "main_street" },
//!   "zones": [ { "zone": "centre", "to": 0.3, "lots": ["shop", "office"] },
//!              { "zone": "town", "to": 1.0, "lots": ["house"],
//!                "people": [ { "creature": "person", "count": [1, 3], "in": "home" } ] } ],
//!   "must_have": [ { "site": "town_square", "at": "centre" }, "town_hall" ] }
//! ```
//!
//! A settlement is laid out as a [`SettlementPlan`] by
//! [`StandardGenerator::settlement_plan`](crate::StandardGenerator::settlement_plan): a pure function of the definitions, the
//! settlement's position and its seed. Every region it reaches and
//! every tract near it works out the same plan, so a settlement that
//! spans two regions has the same streets on both sides of the edge.
//!
//! # The layout
//!
//! 1. **Streets.** Streets running east to west cross the whole extent,
//!    the first and the last along its north and south edges. Between
//!    two of them lies a row of blocks, cut by streets running north to
//!    south, the first and last along the west and east edges. With the
//!    `grid` pattern every row is cut at the same places and the
//!    spacing is even; with `organic` each row is cut at places of its
//!    own and every street is moved a little from where the grid would
//!    put it, so that junctions are staggered and blocks differ in size.
//!    Every street meets one that runs the whole width, so all streets
//!    join. Main roads are the middle street of each direction, drawn
//!    with the main path kind; with `organic` the main road north to
//!    south runs straight through every row.
//! 2. **Blocks and lots.** A block deep enough has a row of lots along
//!    its north side, fronting the street to the north, and one along
//!    its south side, fronting the street to the south; a shallow one
//!    has one row. A lot is as wide as the map that fills it.
//! 3. **Sites.** Each `must_have` site takes the start of a row: near
//!    the centre, near the edge, or anywhere, as it asks. A site takes
//!    the tracts its map covers, and a tract holds one site at most
//!    (02 §8.4), so a site is put where no other site's tracts are, and
//!    where its map lies in one region.
//! 4. **Filling.** Along each row, from its west end, a map is chosen
//!    by the positional hash among the maps whose tags the block's zone
//!    names and which fit what is left of the row, turned so that its
//!    `front` faces the street; then `gap` cells are left, and the next
//!    is chosen. A row ends when no map fits what is left.

mod def;
pub(crate) mod layout;

pub use def::{MustHaveDef, SettlementDef, SettlementId, StreetsDef, ZoneDef};
pub use layout::{Block, Lot, LotFill, SettlementPlan};

use clash_core::Real;
use clash_data::{Def, DefLabel, Ref};
use clash_world::dims::{REGION_CELLS, REGION_TRACTS};
use clash_world::{PathKindId, SiteDefId, TractDefId};

use crate::ground::Report;
use crate::map::{MapDefs, MapId};
use crate::region::{PathKind, People, SiteKind, build_people};

/// The fewest cells across a block, either way: anything smaller is
/// left as open ground.
pub const MIN_BLOCK: i32 = 4;

/// The most cells left between two buildings side by side.
pub const MAX_GAP: i32 = 8;

/// The most main roads a settlement has: one each way.
pub const MAX_MAIN: i64 = 2;

/// The patterns of streets that are built.
pub const PATTERN_GRID: &str = "grid";
/// See [`PATTERN_GRID`].
pub const PATTERN_ORGANIC: &str = "organic";
/// Patterns 10 §8.4 names that are not built.
const PATTERNS_LATER: [&str; 3] = ["radial", "linear", "scatter"];

/// Where a `must_have` site goes.
pub const AT_CENTRE: &str = "centre";
/// See [`AT_CENTRE`].
pub const AT_EDGE: &str = "edge";
/// See [`AT_CENTRE`].
pub const AT_ANYWHERE: &str = "anywhere";

/// A pattern of streets.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Pattern {
    /// Straight streets at even spacing.
    Grid,
    /// Streets moved from the grid, with staggered junctions.
    Organic,
}

/// Where in a settlement a site goes.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum SiteWhere {
    /// Wherever the seed puts it.
    Anywhere,
    /// As near the centre as it fits.
    Centre,
    /// As far from the centre as it fits.
    Edge,
}

/// A ring of a settlement, made ready.
#[derive(Clone, Debug)]
pub struct Zone {
    pub(crate) name: String,
    pub(crate) to: Real,
    /// The maps that may fill its lots: each carries one of its tags and
    /// names a `front`.
    pub(crate) maps: Vec<MapId>,
    pub(crate) people: Vec<People>,
}

impl Zone {
    /// Its name.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// The maps that may fill its lots.
    pub fn maps(&self) -> &[MapId] {
        &self.maps
    }

    /// Who lives or works in each of its lots.
    pub fn people(&self) -> &[People] {
        &self.people
    }
}

/// A site every instance has.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct MustHave {
    /// The site.
    pub site: SiteDefId,
    /// Where it goes.
    pub at: SiteWhere,
}

/// A settlement definition made ready.
#[derive(Clone, Debug)]
pub struct SettlementKind {
    pub(crate) name: String,
    pub(crate) tags: Vec<String>,
    /// Width and depth in tracts.
    pub(crate) size: (i32, i32),
    pub(crate) tract: Option<TractDefId>,
    pub(crate) pattern: Pattern,
    pub(crate) spacing: (i32, i32),
    pub(crate) street: PathKindId,
    pub(crate) main: i64,
    pub(crate) main_path: PathKindId,
    pub(crate) gap: i32,
    pub(crate) zones: Vec<Zone>,
    pub(crate) must_have: Vec<MustHave>,
    pub(crate) home_mark: Option<String>,
}

impl SettlementKind {
    /// The definition's qualified id.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Its tags, sorted.
    pub fn tags(&self) -> &[String] {
        &self.tags
    }

    /// Its extent in tracts: width and depth.
    pub fn size(&self) -> (i32, i32) {
        self.size
    }

    /// The tract definition its tracts are given, if it names one.
    pub fn tract(&self) -> Option<TractDefId> {
        self.tract
    }

    /// Its zones, from the centre out.
    pub fn zones(&self) -> &[Zone] {
        &self.zones
    }

    /// The sites every instance has.
    pub fn must_have(&self) -> &[MustHave] {
        &self.must_have
    }

    /// The mark that counts as a home's bed, as written.
    pub fn home_mark(&self) -> Option<&str> {
        self.home_mark.as_deref()
    }
}

/// Builds a settlement definition.
pub(crate) fn build_settlement(
    report: &mut Report<'_>,
    maps: &MapDefs,
    paths: &[PathKind],
    sites: &[SiteKind],
    name: &str,
    def: &SettlementDef,
) -> SettlementKind {
    let label = DefLabel::new(SettlementDef::TYPE, name);
    let size = match def.size.value.as_slice() {
        [w, h]
            if (1..=i64::from(REGION_TRACTS)).contains(w)
                && (1..=i64::from(REGION_TRACTS)).contains(h) =>
        {
            (*w as i32, *h as i32)
        }
        _ => {
            report.error(
                &label,
                &def.size.site,
                format!("`size` is `[width, depth]` in tracts, each from 1 to {REGION_TRACTS}"),
            );
            (1, 1)
        }
    };
    let pattern = match def.streets.pattern.value.as_str() {
        PATTERN_GRID => Pattern::Grid,
        PATTERN_ORGANIC => Pattern::Organic,
        later if PATTERNS_LATER.contains(&later) => {
            report.error(
                &label,
                &def.streets.pattern.site,
                format!("the pattern `{later}` is not built yet; `grid` and `organic` are"),
            );
            Pattern::Grid
        }
        other => {
            report.error(
                &label,
                &def.streets.pattern.site,
                format!("a pattern of streets is `grid` or `organic`, not `{other}`"),
            );
            Pattern::Grid
        }
    };
    let street = def.streets.path.id();
    let main_path = def.streets.main_path.map_or(street, Ref::id);
    let footprint = |kind: PathKindId| paths.get(kind.index()).map_or(1, PathKind::footprint);
    let widest = footprint(street).max(footprint(main_path));
    let spacing = match def.streets.spacing.value.as_slice() {
        [low, high]
            if *low >= i64::from(widest + MIN_BLOCK)
                && low <= high
                && *high <= i64::from(REGION_CELLS) =>
        {
            (*low as i32, *high as i32)
        }
        _ => {
            report.error(
                &label,
                &def.streets.spacing.site,
                format!(
                    "`spacing` is `[low, high]` cells from one street to the next, `low` first: \
                     at least {} here (the widest street and a block of {MIN_BLOCK}), and at \
                     most {REGION_CELLS}",
                    widest + MIN_BLOCK
                ),
            );
            (widest + MIN_BLOCK, widest + MIN_BLOCK)
        }
    };
    let main = match &def.streets.main {
        None => 0,
        Some(main) if (0..=MAX_MAIN).contains(&main.value) => main.value,
        Some(main) => {
            report.error(
                &label,
                &main.site,
                format!("`main` is from 0 to {MAX_MAIN}: none, east to west, and north to south"),
            );
            0
        }
    };
    let gap = match &def.gap {
        None => 1,
        Some(gap) => match i32::try_from(gap.value) {
            Ok(gap) if (0..=MAX_GAP).contains(&gap) => gap,
            _ => {
                report.error(
                    &label,
                    &gap.site,
                    format!("`gap` is from 0 to {MAX_GAP} cells"),
                );
                1
            }
        },
    };
    let mut zones = Vec::new();
    let mut last = Real::ZERO;
    for zone in &def.zones {
        let to = match Real::new(zone.to.value) {
            Ok(to) if to > last && to <= Real::ONE => to,
            _ => {
                report.error(
                    &label,
                    &zone.to.site,
                    "`to` is above the zone before it and at most 1: a share of the way from \
                     the centre to the edge",
                );
                continue;
            }
        };
        last = to;
        let eligible: Vec<MapId> = maps
            .iter()
            .filter(|(_, map)| {
                map.front().is_some() && map.tags().iter().any(|tag| zone.lots.value.contains(tag))
            })
            .map(|(id, _)| id)
            .collect();
        if eligible.is_empty() {
            report.error(
                &label,
                &zone.lots.site,
                "no map carrying any of these tags names a `front`, so nothing can fill a lot",
            );
        }
        zones.push(Zone {
            name: zone.zone.clone(),
            to,
            maps: eligible,
            people: build_people(report, &label, &zone.people),
        });
    }
    if let Some(zone) = def.zones.last()
        && last < Real::ONE
    {
        report.error(
            &label,
            &zone.to.site,
            "the last zone reaches the edge: its `to` is 1",
        );
    }
    let mut must_have = Vec::new();
    for entry in &def.must_have {
        let site = entry.site.id();
        let at = match &entry.at {
            None => SiteWhere::Anywhere,
            Some(at) => match at.value.as_str() {
                AT_CENTRE => SiteWhere::Centre,
                AT_EDGE => SiteWhere::Edge,
                AT_ANYWHERE => SiteWhere::Anywhere,
                other => {
                    report.error(
                        &label,
                        &at.site,
                        format!("`at` is `centre`, `edge` or `anywhere`, not `{other}`"),
                    );
                    continue;
                }
            },
        };
        if sites
            .get(site.index())
            .is_some_and(|kind| kind.founded() || kind.maps().is_empty())
        {
            if let Some(at) = &entry.at {
                report.error(
                    &label,
                    &at.site,
                    "this site has no map to place: a founded site is made in play",
                );
            }
            continue;
        }
        must_have.push(MustHave { site, at });
    }
    if let Some(wall) = &def.wall {
        report.error(
            &label,
            &wall.site,
            "walls round a settlement are not built yet: they wait for assemblies (WG10)",
        );
    }
    let mut tags: Vec<String> = def
        .tags
        .iter()
        .map(|tag| report.defs.tags().name(tag).to_owned())
        .collect();
    tags.sort();
    SettlementKind {
        name: name.to_owned(),
        tags,
        size,
        tract: def.tract.map(Ref::id),
        pattern,
        spacing,
        street,
        main,
        main_path,
        gap,
        zones,
        must_have,
        home_mark: def.home_mark.clone(),
    }
}
