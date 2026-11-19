//! Laying out a settlement: streets, blocks, lots, sites. See the
//! module documentation of [`crate::settlement`].

use std::collections::BTreeSet;

use clash_core::{Dir8, HashDomain, Real, Rng, Rot4, Turn, pos_hash, pos_rng};
use clash_world::dims::TRACT_XY;
use clash_world::{CellBox, CellPos, Placement, SiteDefId, TractPos};

use super::{Pattern, SettlementId, SettlementKind, SiteWhere};
use crate::map::{CompiledMap, MapId, turned_cell, turned_size};
use crate::place::{boxes_overlap, placement_box, world_map_id};
use crate::realize::{DroppedPlacement, StandardGenerator};
use crate::region::paths::across;
use crate::region::{PlannedSite, Seg};

/// The domain of how a settlement's streets are spaced and moved.
const STREET_DOMAIN: HashDomain = HashDomain::new("generator.settlement.street");
/// The domain of which map fills a lot.
const LOT_DOMAIN: HashDomain = HashDomain::new("generator.settlement.lot");
/// The domain of the seed a lot's placement is given.
const LOT_SEED_DOMAIN: HashDomain = HashDomain::new("generator.settlement.seed");
/// The domain of the order sites try their rows in, and their maps.
const SITE_DOMAIN: HashDomain = HashDomain::new("generator.settlement.site");

/// How far an organic street may move from where the grid would put
/// it, as a share of the spacing: one part in this many.
const ORGANIC_SWAY: i32 = 5;

/// One settlement the realm places: its definition, where, and its
/// seed.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct SettlementAt {
    /// The definition.
    pub def: SettlementId,
    /// Its north-west tract.
    pub at: TractPos,
    /// The seed of everything it leaves to chance.
    pub seed: u64,
}

/// A block of a settlement: the open land between its streets.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Block {
    /// The north-west cell, as `(x, y)`.
    pub min: (i32, i32),
    /// The south-east cell, included.
    pub max: (i32, i32),
    /// Its zone, by position in the definition's list.
    pub zone: usize,
}

/// What fills a lot.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum LotFill {
    /// A map of a zone, by its position in the definition's list.
    Map {
        /// The zone.
        zone: usize,
    },
    /// A site, by its position in the plan's list.
    Site {
        /// The site.
        site: usize,
    },
}

/// A lot: a map placed along the front of a block, facing its street.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Lot {
    /// The placement.
    pub placement: Placement,
    /// The box it fills.
    pub bounds: CellBox,
    /// What fills it.
    pub fill: LotFill,
    /// The way its front faces: towards its street.
    pub facing: Dir8,
}

/// A settlement laid out (10 §8.4): a pure function of the definitions,
/// its position and its seed.
#[derive(Clone, Debug)]
pub struct SettlementPlan {
    /// Its definition.
    pub def: SettlementId,
    /// The north-west and south-east cells of its extent, as `(x, y)`.
    pub extent: ((i32, i32), (i32, i32)),
    /// Its centre cell.
    pub centre: (i32, i32),
    /// Its streets.
    pub streets: Vec<Seg>,
    /// Its blocks.
    pub blocks: Vec<Block>,
    /// Its lots, sites' among them.
    pub lots: Vec<Lot>,
    /// Its sites: the `must_have` sites that could be placed.
    pub sites: Vec<PlannedSite>,
    /// The `must_have` sites that could not be placed.
    pub missing: Vec<SiteDefId>,
    /// Lots that met a placement the realm makes by hand.
    pub dropped: Vec<DroppedPlacement>,
}

impl SettlementPlan {
    /// Whether a column lies inside its extent.
    pub fn contains(&self, x: i32, y: i32) -> bool {
        let ((x0, y0), (x1, y1)) = self.extent;
        (x0..=x1).contains(&x) && (y0..=y1).contains(&y)
    }

    /// Whether a column lies within `margin` cells of its extent.
    pub fn near(&self, x0: i32, y0: i32, x1: i32, y1: i32, margin: i32) -> bool {
        let ((ex0, ey0), (ex1, ey1)) = self.extent;
        x1 >= ex0 - margin && x0 <= ex1 + margin && y1 >= ey0 - margin && y0 <= ey1 + margin
    }
}

/// A row of lots along one side of a block.
#[derive(Clone, Copy, Debug)]
struct Row {
    x0: i32,
    x1: i32,
    y0: i32,
    y1: i32,
    facing: Dir8,
    zone: usize,
    /// Where the next lot starts.
    cursor: i32,
    /// How far its block's centre is from the settlement's centre.
    distance: Real,
}

/// The centre lines of streets across a span: the first and the last
/// given, and between them as many as the spacing allows, evenly, or
/// moved by up to `sway` cells each. Moved lines that would leave a
/// gap below `least` are not moved.
fn lines(first: i32, last: i32, spacing: i32, sway: i32, least: i32, rng: &mut Rng) -> Vec<i32> {
    let span = last - first;
    let count = (span / spacing.max(1)).max(1);
    let even: Vec<i32> = (0..=count)
        .map(|k| first + (i64::from(span) * i64::from(k) / i64::from(count)) as i32)
        .collect();
    if sway <= 0 || even.len() <= 2 {
        return even;
    }
    let mut moved = even.clone();
    let last_index = moved.len() - 1;
    for line in moved.iter_mut().take(last_index).skip(1) {
        *line += rng.range(i64::from(-sway), i64::from(sway)) as i32;
    }
    if moved.windows(2).all(|pair| pair[1] - pair[0] >= least) {
        moved
    } else {
        even
    }
}

/// The turn that makes a map's front face a way, if it may be turned
/// so.
fn facing_turn(map: &CompiledMap, facing: Dir8) -> Option<Turn> {
    let front = map.front()?;
    let rot = Rot4::ALL
        .into_iter()
        .find(|rot| front.rotated(*rot) == facing)?;
    (rot == Rot4::R0 || map.may_turn()).then_some(Turn::new(rot, false))
}

/// Where a map's anchor goes so that its turned north-west corner is at
/// a cell, with its level 0 at a storey.
pub(crate) fn origin_at(map: &CompiledMap, turn: Turn, x: i32, y: i32, level: i32) -> CellPos {
    match map.anchor() {
        None => CellPos::new(x, y, level),
        Some(anchor) => {
            let cell = turned_cell(anchor.xy(), map.size(), turn);
            CellPos::new(x + cell.x, y + cell.y, level + anchor.z)
        }
    }
}

/// A map fitted to a row at its cursor.
struct Fitted {
    map: MapId,
    placement: Placement,
    bounds: CellBox,
    width: i32,
}

/// Puts a map at the start of what is left of a row, facing its street,
/// if it fits.
fn fit(generator: &StandardGenerator, seed: u64, row: &Row, map: MapId) -> Option<Fitted> {
    let maps = generator.defs().maps();
    let compiled = maps.get(map)?;
    let turn = facing_turn(compiled, row.facing)?;
    let size = turned_size(compiled.size(), turn);
    if size.x > row.x1 - row.cursor + 1 || size.y > row.y1 - row.y0 + 1 {
        return None;
    }
    let x = row.cursor;
    let y = if row.facing == Dir8::North {
        row.y0
    } else {
        row.y1 - size.y + 1
    };
    let flat = origin_at(compiled, turn, x, y, 0);
    let level = generator.base_level(seed, compiled.base(), flat.x, flat.y);
    let placement = Placement {
        map: world_map_id(map),
        origin: origin_at(compiled, turn, x, y, level),
        turn,
        params: clash_core::VarMap::new(),
        seed: pos_hash(seed, LOT_SEED_DOMAIN, x, y, 0, u64::from(map.raw())),
        order: 0,
        site: None,
    };
    let bounds = placement_box(maps, &placement)?;
    Some(Fitted {
        map,
        placement,
        bounds,
        width: size.x,
    })
}

/// Lays out one settlement the realm places. `realm_seed` is the
/// realm's: the height field and the realm's own placements depend on
/// it.
pub(crate) fn lay_out(
    generator: &StandardGenerator,
    realm_seed: u64,
    at: &SettlementAt,
) -> Option<SettlementPlan> {
    let defs = generator.defs();
    let kind: &SettlementKind = defs.settlement(at.def)?;
    let seed = at.seed;
    let origin = at.at.origin(0);
    let (x0, y0) = (origin.x, origin.y);
    let (x1, y1) = (
        x0 + kind.size.0 * TRACT_XY - 1,
        y0 + kind.size.1 * TRACT_XY - 1,
    );
    let centre = ((x0 + x1) / 2, (y0 + y1) / 2);
    let footprint = |main: bool| {
        let path = if main { kind.main_path } else { kind.street };
        defs.path_kind(path).map_or(1, |path| path.footprint())
    };
    let ordinary = footprint(false);
    let widest = ordinary.max(footprint(true));
    let least = widest + super::MIN_BLOCK;
    let (low, high) = across(ordinary);
    let organic = kind.pattern == Pattern::Organic;

    // Streets running east to west.
    let mut rng = pos_rng(seed, STREET_DOMAIN, 0, 0, 0, 0);
    let spacing_y = rng.range(i64::from(kind.spacing.0), i64::from(kind.spacing.1)) as i32;
    let spacing_x = rng.range(i64::from(kind.spacing.0), i64::from(kind.spacing.1)) as i32;
    let sway = |spacing: i32| if organic { spacing / ORGANIC_SWAY } else { 0 };
    let rows_at = lines(
        y0 - low,
        y1 - high,
        spacing_y,
        sway(spacing_y),
        least,
        &mut rng,
    );
    let main_row = (kind.main >= 1 && rows_at.len() > 2).then_some(rows_at.len() / 2);
    // The lines north to south of the grid: every row's, or, with the
    // organic pattern, where its straight main road runs.
    let grid_columns = lines(x0 - low, x1 - high, spacing_x, 0, least, &mut rng);
    let main_column = (kind.main >= 2 && grid_columns.len() > 2).then_some(grid_columns.len() / 2);

    let mut streets = Vec::new();
    let row_footprint = |k: usize| footprint(main_row == Some(k));
    for (k, y) in rows_at.iter().enumerate() {
        let main = main_row == Some(k);
        streets.push(Seg {
            kind: if main { kind.main_path } else { kind.street },
            from: (x0, *y),
            to: (x1, *y),
            minor: false,
        });
    }

    let mut blocks = Vec::new();
    let mut rows = Vec::new();
    for k in 0..rows_at.len().saturating_sub(1) {
        let (top, bottom) = (rows_at[k], rows_at[k + 1]);
        // The columns of this row of blocks, and which is a main road.
        let (columns, main_at): (Vec<i32>, Option<i32>) = if organic {
            let mut row_rng = pos_rng(seed, STREET_DOMAIN, 1, k as i32, 0, 0);
            let spacing =
                row_rng.range(i64::from(kind.spacing.0), i64::from(kind.spacing.1)) as i32;
            let (west, east) = (x0 - low, x1 - high);
            match main_column.map(|index| grid_columns[index]) {
                Some(middle) => {
                    let mut all = lines(west, middle, spacing, sway(spacing), least, &mut row_rng);
                    let east_part =
                        lines(middle, east, spacing, sway(spacing), least, &mut row_rng);
                    all.extend(east_part.into_iter().skip(1));
                    (all, Some(middle))
                }
                None => (
                    lines(west, east, spacing, sway(spacing), least, &mut row_rng),
                    None,
                ),
            }
        } else {
            (
                grid_columns.clone(),
                main_column.map(|index| grid_columns[index]),
            )
        };
        let column_footprint = |x: i32| footprint(main_at == Some(x));
        for x in &columns {
            let main = main_at == Some(*x);
            streets.push(Seg {
                kind: if main { kind.main_path } else { kind.street },
                from: (*x, top),
                to: (*x, bottom),
                minor: false,
            });
        }
        let block_top = top + across(row_footprint(k)).1 + 1;
        let block_bottom = bottom + across(row_footprint(k + 1)).0 - 1;
        for pair in columns.windows(2) {
            let block_west = pair[0] + across(column_footprint(pair[0])).1 + 1;
            let block_east = pair[1] + across(column_footprint(pair[1])).0 - 1;
            if block_east - block_west + 1 < super::MIN_BLOCK
                || block_bottom - block_top + 1 < super::MIN_BLOCK
            {
                continue;
            }
            // The zone of the block, by how far its centre is from the
            // settlement's, as a share of the way to the edge.
            let (bx, by) = (
                (block_west + block_east) / 2,
                (block_top + block_bottom) / 2,
            );
            let share = |offset: i32, half: i32| {
                Real::from_i64(i64::from(offset.abs())) / Real::from_i64(i64::from(half.max(1)))
            };
            let distance = share(bx - centre.0, (x1 - x0 + 1) / 2)
                .max(share(by - centre.1, (y1 - y0 + 1) / 2));
            let zone = kind
                .zones
                .iter()
                .position(|zone| distance <= zone.to)
                .unwrap_or(kind.zones.len().saturating_sub(1));
            blocks.push(Block {
                min: (block_west, block_top),
                max: (block_east, block_bottom),
                zone,
            });
            // A row of lots along each side deep enough for the
            // shallowest map of the zone, or one along the north side.
            let depth = block_bottom - block_top + 1;
            let shallowest = kind.zones.get(zone).map_or(depth, |zone| {
                zone.maps
                    .iter()
                    .filter_map(|map| {
                        let compiled = defs.maps().get(*map)?;
                        let turn = facing_turn(compiled, Dir8::North)?;
                        Some(turned_size(compiled.size(), turn).y)
                    })
                    .min()
                    .unwrap_or(depth)
            });
            let row = |y0: i32, y1: i32, facing: Dir8| Row {
                x0: block_west,
                x1: block_east,
                y0,
                y1,
                facing,
                zone,
                cursor: block_west,
                distance,
            };
            if depth / 2 >= shallowest {
                let split = block_top + depth / 2;
                rows.push(row(block_top, split - 1, Dir8::North));
                rows.push(row(split, block_bottom, Dir8::South));
            } else {
                rows.push(row(block_top, block_bottom, Dir8::North));
            }
        }
    }

    // What the realm places by hand stands where it is: a lot that
    // meets it is dropped.
    let whole = CellBox::new(
        CellPos::new(x0, y0, i32::MIN),
        CellPos::new(x1, y1, i32::MAX),
    );
    let blocked: Vec<CellBox> = generator
        .authored_in(realm_seed, whole)
        .iter()
        .filter_map(|placement| placement_box(defs.maps(), placement))
        .collect();
    let meets = |bounds: CellBox| {
        blocked
            .iter()
            .copied()
            .find(|other| boxes_overlap(bounds, *other))
    };

    // The sites every instance has, each at the start of a row.
    let mut lots = Vec::new();
    let mut sites = Vec::new();
    let mut missing = Vec::new();
    let mut dropped = Vec::new();
    let mut claimed: BTreeSet<(i32, i32)> = BTreeSet::new();
    for (index, must) in kind.must_have.iter().enumerate() {
        let Some(site) = defs.site(must.site) else {
            continue;
        };
        let mut order: Vec<usize> = (0..rows.len()).collect();
        let hash = |row: usize| pos_hash(seed, SITE_DOMAIN, row as i32, 0, 0, index as u64);
        match must.at {
            SiteWhere::Centre => order.sort_by_key(|&row| (rows[row].distance, row)),
            SiteWhere::Edge => {
                order.sort_by_key(|&row| (core::cmp::Reverse(rows[row].distance), row))
            }
            SiteWhere::Anywhere => order.sort_by_key(|&row| (hash(row), row)),
        }
        let mut placed = false;
        'rows: for row_index in order {
            let row = rows[row_index];
            if row.cursor != row.x0 {
                continue;
            }
            let mut maps = site.maps.clone();
            maps.sort_by_key(|map| {
                pos_hash(seed, SITE_DOMAIN, map.raw() as i32, 1, 0, index as u64)
            });
            for map in maps {
                let Some(fitted) = fit(generator, realm_seed, &row, map) else {
                    continue;
                };
                let tracts: Vec<TractPos> = fitted.bounds.tracts().collect();
                let one_region =
                    fitted.bounds.min.tract().region() == fitted.bounds.max.tract().region();
                if !one_region
                    || tracts
                        .iter()
                        .any(|tract| claimed.contains(&(tract.x, tract.y)))
                    || meets(fitted.bounds).is_some()
                {
                    continue;
                }
                claimed.extend(tracts.iter().map(|tract| (tract.x, tract.y)));
                rows[row_index].cursor = row.cursor + fitted.width + kind.gap;
                let placement = Placement {
                    seed: pos_hash(seed, LOT_SEED_DOMAIN, row.cursor, row.y0, 1, index as u64),
                    ..fitted.placement
                };
                lots.push(Lot {
                    placement: placement.clone(),
                    bounds: fitted.bounds,
                    fill: LotFill::Site { site: sites.len() },
                    facing: row.facing,
                });
                sites.push(PlannedSite {
                    def: must.site,
                    anchor: placement.origin,
                    placement,
                    bounds: fitted.bounds,
                    tracts,
                });
                placed = true;
                break 'rows;
            }
        }
        if !placed {
            missing.push(must.site);
        }
    }

    // Every row filled from its cursor.
    for row in &mut rows {
        let Some(zone) = kind.zones.get(row.zone) else {
            continue;
        };
        loop {
            let fitting: Vec<Fitted> = zone
                .maps
                .iter()
                .filter_map(|map| fit(generator, realm_seed, row, *map))
                .collect();
            let mut pick = pos_rng(seed, LOT_DOMAIN, row.cursor, row.y0, 0, 0);
            let Some(chosen) = pick.pick(&fitting) else {
                break;
            };
            match meets(chosen.bounds) {
                Some(met) => dropped.push(DroppedPlacement {
                    tract: chosen.placement.origin.tract(),
                    placement: chosen.placement.clone(),
                    bounds: chosen.bounds,
                    met,
                }),
                None => lots.push(Lot {
                    placement: chosen.placement.clone(),
                    bounds: chosen.bounds,
                    fill: LotFill::Map { zone: row.zone },
                    facing: row.facing,
                }),
            }
            let _ = chosen.map;
            row.cursor += chosen.width + kind.gap;
        }
    }

    Some(SettlementPlan {
        def: at.def,
        extent: ((x0, y0), (x1, y1)),
        centre,
        streets,
        blocks,
        lots,
        sites,
        missing,
        dropped,
    })
}
