//! Placing maps and applying them to the world (10 §4).
//!
//! A [`Placement`] is a record: this map, here, turned this way, with
//! these parameters and this seed. A survey writes placements; realising
//! a tract reads those that overlap it and writes its own window of each;
//! and the same record applies a map to the world as it is, through
//! `WorldEdit`.
//!
//! | Need | Use |
//! |------|-----|
//! | The box a placement fills, for the overlap test | [`placement_box`], [`boxes_overlap`] |
//! | A placement with its map resolved: cells, marks, areas, spawns in world coordinates | [`PlacedMap`] |
//! | Writing a map into loaded cells | [`apply_map`] |
//! | The same, leaving it as a pending edit where tracts are not loaded | [`place_map`] |
//! | Writing the maps that waited for a tract, once it has loaded | [`apply_pending_maps`] |
//!
//! # Where a map goes
//!
//! A placement's `origin` is where the map's anchor goes: the cell its
//! `anchor` names, or the north-west corner at level 0 of its turned
//! footprint (10 §3.5). The world position of a map cell is the origin,
//! less the anchor, plus the cell.
//!
//! # A window for each tract
//!
//! Resolving a map is a pure function of the placement, so every tract a
//! map covers resolves it again and writes the cells that fall inside it.
//! Nothing is read from a neighbouring tract, and the tracts of a large
//! map come out the same in every order (10 §14, X1).
//!
//! # Sitting on the ground
//!
//! | `sit` | When a tract is realised | Applied to the world as it is |
//! |-------|--------------------------|-------------------------------|
//! | `level` | The surface under the footprint is the map's ground level before the ground is filled, so the ground's own rules lay strata, a surface and ramps round it | Ground above the map's level 0 is cut away within the footprint and gaps below are filled with the block under the origin |
//! | `foundation` | Under every column the map draws, gaps are filled with the map's `foundation` state and ground above its level 0 is cut away | The same |
//! | `follow` | Each column of the map is shifted to its own ground height | The same |
//! | `none` | Nothing is done | Nothing is done |
//!
//! In the world as it is, the ground of a column is found near the
//! storey the placement names: the nearest storey, within
//! [`SIT_REACH_STOREYS`], whose cell stands on a block with a solid top.

use std::collections::BTreeMap;

use clash_core::{HashDomain, IVec3, VarMap, pos_hash};
use clash_world::dims::TRACT_XY;
use clash_world::{
    AreaId, AreaShape, AreaSpec, BlockState, Cause, CauseKind, CellBox, CellFlags, CellPos,
    CellView, ColumnBuilder, ElementState, FixtureState, FloorState, GridId, Layer, Location,
    PendingEdit, RealmId, SpawnRequest, TractPos, WorldCtx, WorldError,
};
pub use clash_world::{MapMode, PendingMap, Placement};
use thiserror::Error;

use crate::map::{
    MapDefs, MapId, MapShape, ResolveError, ResolvedCell, ResolvedMap, Sit, SpawnPlace, SpawnWho,
};

/// How far up and down from the storey a placement names the ground of
/// a column is looked for, when a map is applied to the world as it is.
pub const SIT_REACH_STOREYS: i32 = 8;

/// The domain of the choice of where each creature of a spawn goes.
const SPAWN_DOMAIN: HashDomain = HashDomain::new("gen.place.spawn");

/// The parameter of a spawn request that says what `what` names:
/// [`SPAWN_KIND_CREATURE`] or [`SPAWN_KIND_POPULATION`].
pub const SPAWN_PARAM_KIND: &str = "kind";
/// The parameter of a spawn request that names the map that asked for
/// it, by qualified id: a bare `what` is an id of that map's mod.
pub const SPAWN_PARAM_MAP: &str = "map";
/// A spawn request for one creature of a definition.
pub const SPAWN_KIND_CREATURE: &str = "creature";
/// A spawn request for one member of a population.
pub const SPAWN_KIND_POPULATION: &str = "population";

/// The world's id of a map: the same number as the loader's.
pub fn world_map_id(map: MapId) -> clash_world::MapId {
    clash_world::MapId::new(map.raw())
}

/// The loader's id of a map a record names.
pub fn gen_map_id(map: clash_world::MapId) -> MapId {
    MapId::new(map.raw())
}

/// A placement of a map with its anchor at a cell: not turned, no
/// parameters given, seed 0, order 0, no site. Change the fields as
/// needed.
pub fn placement(map: MapId, origin: CellPos) -> Placement {
    Placement {
        map: world_map_id(map),
        origin,
        turn: clash_core::Turn::IDENTITY,
        params: VarMap::new(),
        seed: 0,
        order: 0,
        site: None,
    }
}

/// The box a placement fills: its footprint and its storeys, after
/// turning (10 §4.1). `None` if the placement names no map.
///
/// A map that sits by `follow` moves each column to its own ground, so
/// its cells may lie above or below this box; the footprint is exact.
pub fn placement_box(maps: &MapDefs, placement: &Placement) -> Option<CellBox> {
    let map = maps.get(gen_map_id(placement.map))?;
    let size = crate::map::turned_size(map.size(), placement.turn);
    let (low, high) = map.level_range();
    let corner = crate::map::nested_origin(
        map.size(),
        map.anchor(),
        placement.turn,
        placement.origin.to_ivec3(),
    );
    Some(CellBox::new(
        CellPos::new(corner.x, corner.y, corner.z.saturating_add(low)),
        CellPos::new(
            corner.x.saturating_add(size.x - 1),
            corner.y.saturating_add(size.y - 1),
            corner.z.saturating_add(high),
        ),
    ))
}

/// Whether two boxes share a cell: the overlap test of 10 §4.1, in
/// three dimensions.
pub fn boxes_overlap(a: CellBox, b: CellBox) -> bool {
    a.intersection(b).is_some()
}

/// Why a map could not be placed.
#[derive(Clone, PartialEq, Eq, Debug, Error)]
pub enum PlaceError {
    /// The map could not be resolved for the placement.
    #[error(transparent)]
    Resolve(#[from] ResolveError),
    /// The world refused.
    #[error(transparent)]
    World(#[from] WorldError),
}

/// A placement with its map resolved: what the map puts where, in world
/// coordinates.
///
/// Everything a resolved map carries that the world cannot take yet
/// (items, conduits, fields, devices, links, berths, watched cells,
/// variables, decals, patterns, joints, portals, vehicles) is in
/// [`map`](PlacedMap::map) as the map format left it, in map
/// coordinates; [`world`](PlacedMap::world) turns a map cell into a
/// world cell.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct PlacedMap {
    /// The record.
    pub placement: Placement,
    /// The map, resolved for the record's parameters, seed and turn.
    pub map: ResolvedMap,
    /// How the map meets uneven ground.
    pub sit: Sit,
    /// The map's qualified id.
    pub name: String,
    /// Where the north-west corner of the turned map, at its level 0,
    /// goes.
    pub corner: CellPos,
}

impl PlacedMap {
    /// Resolves the map of a placement. `Ok(None)` if the map's `if` is
    /// false for the placement's parameters: it is not placed.
    pub fn resolve(maps: &MapDefs, placement: &Placement) -> Result<Option<PlacedMap>, PlaceError> {
        let id = gen_map_id(placement.map);
        let compiled = maps.get(id).ok_or(ResolveError::UnknownMap(id))?;
        let Some(map) = maps.resolve(id, &placement.params, placement.seed, placement.turn)? else {
            return Ok(None);
        };
        let anchor = map.anchor_cell();
        let origin = placement.origin;
        Ok(Some(PlacedMap {
            placement: placement.clone(),
            sit: compiled.sit(),
            name: compiled.name().to_owned(),
            corner: CellPos::new(
                origin.x.wrapping_sub(anchor.x),
                origin.y.wrapping_sub(anchor.y),
                origin.z.wrapping_sub(anchor.z),
            ),
            map,
        }))
    }

    /// The world cell of a cell of the map. For a map that sits by
    /// `follow` this is where the cell would go on flat ground.
    pub fn world(&self, cell: IVec3) -> CellPos {
        self.corner.offset(cell)
    }

    /// The box the map fills.
    pub fn bounds(&self) -> CellBox {
        let (low, high) = self.map.level_range();
        CellBox::new(
            self.world(IVec3::new(0, 0, low)),
            self.world(IVec3::new(self.map.size.x - 1, self.map.size.y - 1, high)),
        )
    }

    /// The map's marks: named cells, in world coordinates (10 §11.1).
    /// They follow from the map and the placement alone, so they are
    /// known before any of the map is realised.
    pub fn marks(&self) -> BTreeMap<String, Vec<CellPos>> {
        self.map
            .marks
            .iter()
            .map(|(name, cells)| {
                (
                    name.clone(),
                    cells.iter().map(|cell| self.world(*cell)).collect(),
                )
            })
            .collect()
    }

    /// The areas the map asks for, each with its home: the cell whose
    /// tract makes the area. An area that spans tracts is made once, by
    /// the tract its lowest corner or first cell lies in.
    pub fn areas(&self, grid: GridId) -> Vec<(CellPos, AreaSpec)> {
        self.map
            .areas
            .iter()
            .filter_map(|area| {
                let (home, shape) = match &area.shape {
                    MapShape::Box { min, max } => {
                        let cells = CellBox::new(self.world(*min), self.world(*max));
                        (cells.min, AreaShape::Box(cells))
                    }
                    MapShape::Cells(cells) => {
                        let first = self.world(*cells.first()?);
                        (
                            first,
                            AreaShape::cells(cells.iter().map(|cell| self.world(*cell))),
                        )
                    }
                };
                let mut spec = AreaSpec::new(grid, shape);
                spec.kind = area.kind;
                spec.tags = area.tags.clone();
                spec.site = self.placement.site;
                Some((home, spec))
            })
            .collect()
    }

    /// Whether a cell of the map is somewhere a creature could be put:
    /// it has a floor and nothing in the way.
    fn open_floor(&self, cell: IVec3) -> bool {
        let drawn = self.map.cell(cell.x, cell.y, cell.z);
        drawn.floor.is_some_and(|floor| !floor.is_none())
            && drawn.block.is_none_or(|block| block.is_none())
            && drawn.fixture.is_none_or(|fixture| fixture.is_none())
    }

    /// The spawn requests of the map (06 §12), one for each creature,
    /// each at a cell. A spawn that names an area, or none, is given
    /// cells of open floor there, chosen by the placement's seed.
    pub fn spawns(&self) -> Vec<SpawnRequest> {
        let mut requests = Vec::new();
        for (index, spawn) in self.map.spawns.iter().enumerate() {
            let cells: Vec<IVec3> = match &spawn.place {
                SpawnPlace::Cell(cell) => vec![*cell],
                SpawnPlace::Area(name) => {
                    let all: Vec<IVec3> = self
                        .map
                        .areas
                        .iter()
                        .filter(|area| area.name == *name)
                        .flat_map(|area| shape_cells(&area.shape))
                        .collect();
                    let open: Vec<IVec3> = all
                        .iter()
                        .copied()
                        .filter(|cell| self.open_floor(*cell))
                        .collect();
                    if open.is_empty() { all } else { open }
                }
                SpawnPlace::Anywhere => self
                    .map
                    .written_cells()
                    .map(|(cell, _)| cell)
                    .filter(|cell| self.open_floor(*cell))
                    .collect(),
            };
            if cells.is_empty() {
                continue;
            }
            let (what, kind) = match &spawn.who {
                SpawnWho::Creature(id) => (id, SPAWN_KIND_CREATURE),
                SpawnWho::Population(id) => (id, SPAWN_KIND_POPULATION),
            };
            for copy in 0..spawn.count {
                let hash = pos_hash(
                    self.placement.seed,
                    SPAWN_DOMAIN,
                    i32::try_from(index).unwrap_or(i32::MAX),
                    i32::try_from(copy).unwrap_or(i32::MAX),
                    0,
                    0,
                );
                let at = (hash % cells.len() as u64) as usize;
                let Some(cell) = cells.get(at) else { continue };
                let mut params = VarMap::new();
                params.set(SPAWN_PARAM_KIND, kind);
                params.set(SPAWN_PARAM_MAP, self.name.as_str());
                requests.push(SpawnRequest {
                    what: what.clone(),
                    at: self.world(*cell),
                    params,
                });
            }
        }
        requests
    }
}

/// The cells of a shape of a resolved map.
fn shape_cells(shape: &MapShape) -> Vec<IVec3> {
    match shape {
        MapShape::Box { min, max } => (min.z..=max.z)
            .flat_map(|z| {
                (min.y..=max.y).flat_map(move |y| (min.x..=max.x).map(move |x| IVec3::new(x, y, z)))
            })
            .collect(),
        MapShape::Cells(cells) => cells.clone(),
    }
}

/// What a map is written onto: the cells of a tract being realised, or
/// the loaded cells of the world.
pub(crate) trait Canvas {
    /// Whether the cell column at `(x, y)` is this canvas's to write.
    fn covers(&self, x: i32, y: i32) -> bool;
    /// What a cell holds.
    fn cell(&self, at: CellPos) -> CellView;
    /// Sets one element of a cell.
    fn put(&mut self, at: CellPos, layer: Layer, state: ElementState);
    /// The ground height of a column: the lowest storey that is not
    /// ground, near the storey `near`.
    fn ground(&self, x: i32, y: i32, near: i32) -> i32;
    /// What fills a gap under a levelled map whose anchor is at
    /// `origin`, if anything does.
    fn level_fill(&self, _origin: CellPos) -> Option<BlockState> {
        None
    }
}

fn layer_of(view: CellView, layer: Layer) -> ElementState {
    match layer {
        Layer::Floor => view.floor.raw(),
        Layer::Block => view.block.raw(),
        Layer::Fixture => view.fixture.raw(),
    }
}

fn stated(cell: ResolvedCell, layer: Layer) -> Option<ElementState> {
    match layer {
        Layer::Floor => cell.floor.map(FloorState::raw),
        Layer::Block => cell.block.map(BlockState::raw),
        Layer::Fixture => cell.fixture.map(FixtureState::raw),
    }
}

/// Writes one cell of a map as the mode says.
fn write_cell(canvas: &mut dyn Canvas, at: CellPos, cell: ResolvedCell, mode: MapMode) {
    let have = canvas.cell(at);
    if mode == MapMode::OnlyEmpty && !have.is_empty() {
        return;
    }
    for layer in Layer::ALL {
        let Some(state) = stated(cell, layer) else {
            continue;
        };
        let there = layer_of(have, layer);
        let write = match mode {
            MapMode::Replace => there != state,
            MapMode::OnlyEmpty => !state.is_none(),
            MapMode::Add => !state.is_none() && there.is_none(),
        };
        if write {
            canvas.put(at, layer, state);
        }
    }
}

/// Empties a cell.
fn clear_cell(canvas: &mut dyn Canvas, at: CellPos) {
    let have = canvas.cell(at);
    for layer in Layer::ALL {
        if !layer_of(have, layer).is_none() {
            canvas.put(at, layer, ElementState::NONE);
        }
    }
}

/// Whether the map draws anything in each of its columns, row by row.
fn drawn_columns(map: &ResolvedMap) -> Vec<bool> {
    let mut drawn = vec![false; (map.size.x.max(0) * map.size.y.max(0)) as usize];
    for level in &map.levels {
        for (slot, cell) in drawn.iter_mut().zip(&level.cells) {
            *slot |= !cell.is_leave();
        }
    }
    drawn
}

/// Makes the ground ready for a map (10 §4.2, §6.1 step 2): levels it
/// under the footprint, or fills and cuts under the columns the map
/// draws. Only a map written in [`MapMode::Replace`] cuts.
pub(crate) fn prepare(canvas: &mut dyn Canvas, placed: &PlacedMap, mode: MapMode) {
    let fill = match placed.sit {
        Sit::None | Sit::Follow => return,
        Sit::Level => canvas.level_fill(placed.placement.origin),
        Sit::Foundation(state) => state,
    };
    let whole = placed.sit == Sit::Level;
    let drawn = drawn_columns(&placed.map);
    let level = placed.corner.z;
    let size = placed.map.size;
    for my in 0..size.y {
        for mx in 0..size.x {
            if !whole && !drawn[(my * size.x + mx) as usize] {
                continue;
            }
            let (x, y) = (
                placed.corner.x.wrapping_add(mx),
                placed.corner.y.wrapping_add(my),
            );
            if !canvas.covers(x, y) {
                continue;
            }
            let ground = canvas.ground(x, y, level);
            if ground > level && mode == MapMode::Replace {
                // The surface cell holds the ground's floor and whatever
                // was scattered on it: it goes with the ground under it.
                for z in level..=ground {
                    clear_cell(canvas, CellPos::new(x, y, z));
                }
            }
            let Some(fill) = fill else { continue };
            for z in ground..level {
                let at = CellPos::new(x, y, z);
                let have = canvas.cell(at);
                if mode == MapMode::Replace {
                    if !have.floor.is_none() && z > ground {
                        canvas.put(at, Layer::Floor, ElementState::NONE);
                    }
                    if !have.fixture.is_none() {
                        canvas.put(at, Layer::Fixture, ElementState::NONE);
                    }
                    if have.block != fill {
                        canvas.put(at, Layer::Block, fill.raw());
                    }
                } else if have.block.is_none() && (mode == MapMode::Add || have.is_empty()) {
                    canvas.put(at, Layer::Block, fill.raw());
                }
            }
        }
    }
}

/// Writes the cells of a map that fall on a canvas (10 §6.1 step 3).
pub(crate) fn draw(canvas: &mut dyn Canvas, placed: &PlacedMap, mode: MapMode) {
    let level = placed.corner.z;
    let size = placed.map.size;
    for my in 0..size.y {
        for mx in 0..size.x {
            let (x, y) = (
                placed.corner.x.wrapping_add(mx),
                placed.corner.y.wrapping_add(my),
            );
            if !canvas.covers(x, y) {
                continue;
            }
            let shift = if placed.sit == Sit::Follow {
                canvas.ground(x, y, level).saturating_sub(level)
            } else {
                0
            };
            let index = (my * size.x + mx) as usize;
            for storey in &placed.map.levels {
                let Some(cell) = storey.cells.get(index).filter(|cell| !cell.is_leave()) else {
                    continue;
                };
                let z = level.saturating_add(storey.z).saturating_add(shift);
                write_cell(canvas, CellPos::new(x, y, z), *cell, mode);
            }
        }
    }
}

/// The surface heights of a tract and a margin round it: for each cell
/// column, the lowest storey that is not ground.
#[derive(Clone, Debug)]
pub(crate) struct Heights {
    min_x: i32,
    min_y: i32,
    side: i32,
    values: Vec<i32>,
}

impl Heights {
    /// The heights of a tract and `margin` cells round it, each from
    /// `height(x, y)`.
    pub(crate) fn new(tract: TractPos, margin: i32, height: impl Fn(i32, i32) -> i32) -> Heights {
        let origin = tract.origin(0);
        let (min_x, min_y) = (origin.x - margin, origin.y - margin);
        let side = TRACT_XY + 2 * margin;
        let mut values = Vec::with_capacity((side * side) as usize);
        for y in 0..side {
            for x in 0..side {
                values.push(height(min_x + x, min_y + y));
            }
        }
        Heights {
            min_x,
            min_y,
            side,
            values,
        }
    }

    /// The lowest and the highest surface of the tract whose
    /// north-west corner is `origin`.
    pub(crate) fn range(&self, origin: CellPos) -> (i32, i32) {
        let mut range: Option<(i32, i32)> = None;
        for dy in 0..TRACT_XY {
            for dx in 0..TRACT_XY {
                if let Some(height) = self.get(origin.x + dx, origin.y + dy) {
                    range = Some(match range {
                        Some((low, high)) => (low.min(height), high.max(height)),
                        None => (height, height),
                    });
                }
            }
        }
        range.unwrap_or((0, 0))
    }

    /// The lowest surface of the tract whose north-west corner is
    /// `origin`.
    pub(crate) fn lowest(&self, origin: CellPos) -> i32 {
        self.range(origin).0
    }

    /// The height at a column, if it is within the margin.
    pub(crate) fn get(&self, x: i32, y: i32) -> Option<i32> {
        let (dx, dy) = (x - self.min_x, y - self.min_y);
        if dx < 0 || dy < 0 || dx >= self.side || dy >= self.side {
            return None;
        }
        self.values.get((dy * self.side + dx) as usize).copied()
    }
}

/// The cells of one tract while it is realised.
pub(crate) struct TractCanvas<'a> {
    /// The column being built.
    pub(crate) builder: ColumnBuilder<'a>,
    /// The cell at the tract's north-west corner, at storey 0.
    pub(crate) origin: CellPos,
    /// The surface heights of the tract.
    pub(crate) heights: &'a Heights,
}

impl TractCanvas<'_> {
    /// The surface height of a column of the tract or its margin.
    pub(crate) fn height(&self, x: i32, y: i32) -> i32 {
        self.heights.get(x, y).unwrap_or(0)
    }
}

impl Canvas for TractCanvas<'_> {
    fn covers(&self, x: i32, y: i32) -> bool {
        let (dx, dy) = (x.wrapping_sub(self.origin.x), y.wrapping_sub(self.origin.y));
        (0..TRACT_XY).contains(&dx) && (0..TRACT_XY).contains(&dy)
    }

    fn cell(&self, at: CellPos) -> CellView {
        self.builder.column().cell(at)
    }

    fn put(&mut self, at: CellPos, layer: Layer, state: ElementState) {
        match layer {
            Layer::Floor => self.builder.set_floor(at, FloorState::from_raw(state)),
            Layer::Block => self.builder.set_block(at, BlockState::from_raw(state)),
            Layer::Fixture => self.builder.set_fixture(at, FixtureState::from_raw(state)),
        };
    }

    fn ground(&self, x: i32, y: i32, near: i32) -> i32 {
        self.heights.get(x, y).unwrap_or(near)
    }
}

/// The loaded cells of the world, written through `WorldEdit`.
struct LiveCanvas<'a, 'b> {
    ctx: &'a mut WorldCtx<'b>,
    grid: GridId,
    cause: Cause,
    /// A storey inside the grid, for asking whether a column is loaded.
    storey: i32,
    /// The one tract written, or every loaded tract.
    only: Option<TractPos>,
    changed: u32,
    refused: u32,
}

impl LiveCanvas<'_, '_> {
    fn loaded_tract(&self, x: i32, y: i32, z: i32) -> Option<TractPos> {
        let at = self
            .ctx
            .cosmos
            .normalize(Location::new(self.grid, CellPos::new(x, y, z)))
            .ok()?;
        self.ctx.cosmos.is_loaded(at).then(|| at.cell.tract())
    }

    fn solid_top(&self, at: CellPos) -> bool {
        self.ctx
            .cosmos
            .flags(Location::new(self.grid, at))
            .is_ok_and(|flags| flags.contains(CellFlags::SOLID_TOP))
    }
}

impl Canvas for LiveCanvas<'_, '_> {
    fn covers(&self, x: i32, y: i32) -> bool {
        match (self.loaded_tract(x, y, self.storey), self.only) {
            (Some(tract), Some(only)) => tract == only,
            (Some(_), None) => true,
            (None, _) => false,
        }
    }

    fn cell(&self, at: CellPos) -> CellView {
        self.ctx
            .cosmos
            .cell(Location::new(self.grid, at))
            .unwrap_or_default()
    }

    fn put(&mut self, at: CellPos, layer: Layer, state: ElementState) {
        let outcome =
            self.ctx
                .edit(self.cause)
                .set_element(Location::new(self.grid, at), layer, state);
        match outcome {
            Ok(old) if old != state => self.changed += 1,
            Ok(_) => {}
            Err(_) => self.refused += 1,
        }
    }

    fn ground(&self, x: i32, y: i32, near: i32) -> i32 {
        let stands = |z: i32| {
            self.solid_top(CellPos::new(x, y, z - 1)) && !self.solid_top(CellPos::new(x, y, z))
        };
        if stands(near) {
            return near;
        }
        for step in 1..=SIT_REACH_STOREYS {
            for z in [near - step, near + step] {
                if stands(z) {
                    return z;
                }
            }
        }
        near
    }

    fn level_fill(&self, origin: CellPos) -> Option<BlockState> {
        let under = origin.below();
        self.solid_top(under).then(|| self.cell(under).block)
    }
}

/// What applying a map did.
#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct MapApplied {
    /// Whether the map was placed at all: `false` if its `if` was false
    /// for the placement's parameters.
    pub placed: bool,
    /// The box the map fills.
    pub bounds: Option<CellBox>,
    /// How many elements were changed. Each was an ordinary edit with
    /// the cause given, and emitted `ElementChanged`.
    pub changed: u32,
    /// How many changes the world refused: cells beyond an edge, or too
    /// far from the rest of their tract to store.
    pub refused: u32,
    /// What the simulation is to put there (06 §12): the map's spawns
    /// whose cells are in the tracts written.
    pub spawns: Vec<SpawnRequest>,
    /// The areas made: those of the map whose home is in the tracts
    /// written.
    pub areas: Vec<AreaId>,
    /// The map's marks, in world coordinates.
    pub marks: BTreeMap<String, Vec<CellPos>>,
    /// The tracts the map was left waiting for, as a pending edit.
    pub pending: Vec<TractPos>,
    /// The tracts the map covers that are neither loaded nor surveyed,
    /// where it could not be left waiting.
    pub unreached: Vec<TractPos>,
}

fn apply_window(
    ctx: &mut WorldCtx<'_>,
    maps: &MapDefs,
    grid: GridId,
    placement: &Placement,
    mode: MapMode,
    cause: Cause,
    only: Option<TractPos>,
) -> Result<MapApplied, PlaceError> {
    let storey = ctx.cosmos.topology(grid)?.z_min;
    let Some(placed) = PlacedMap::resolve(maps, placement)? else {
        return Ok(MapApplied::default());
    };
    let mut canvas = LiveCanvas {
        ctx,
        grid,
        cause,
        storey,
        only,
        changed: 0,
        refused: 0,
    };
    prepare(&mut canvas, &placed, mode);
    draw(&mut canvas, &placed, mode);
    let written = |canvas: &LiveCanvas<'_, '_>, cell: CellPos| canvas.covers(cell.x, cell.y);
    let spawns = placed
        .spawns()
        .into_iter()
        .filter(|spawn| written(&canvas, spawn.at))
        .collect();
    let (changed, refused) = (canvas.changed, canvas.refused);
    let specs: Vec<AreaSpec> = placed
        .areas(grid)
        .into_iter()
        .filter(|(home, _)| written(&canvas, *home))
        .map(|(_, spec)| spec)
        .collect();
    let areas = specs
        .into_iter()
        .filter_map(|spec| ctx.create_area(spec, cause).ok())
        .collect();
    Ok(MapApplied {
        placed: true,
        bounds: Some(placed.bounds()),
        changed,
        refused,
        spawns,
        areas,
        marks: placed.marks(),
        pending: Vec::new(),
        unreached: Vec::new(),
    })
}

/// Writes a map into the loaded cells of a grid through `WorldEdit`
/// (10 §4.3): the verb `apply_map`.
///
/// Every change is an ordinary edit with `cause`: it emits
/// `ElementChanged`, asks for stability checks and moves occupants like
/// any other (invariant X8). Cells of the map in tracts that are not
/// loaded are left alone; [`place_map`] leaves the map waiting for
/// them. Putting an element in place asks for no stability check
/// (02 §7.2): whoever builds asks, with `WorldCtx::check_stability`.
///
/// | `mode` | Writes |
/// |--------|--------|
/// | [`MapMode::Replace`] | Every layer the map states, emptying where it states nothing |
/// | [`MapMode::OnlyEmpty`] | A cell of the map only where the world's cell holds nothing at all |
/// | [`MapMode::Add`] | Each element only where that layer of the world's cell is empty; never clears |
///
/// Refused if the grid is unknown or the map cannot be resolved for the
/// placement's parameters.
pub fn apply_map(
    ctx: &mut WorldCtx<'_>,
    maps: &MapDefs,
    grid: GridId,
    placement: &Placement,
    mode: MapMode,
    cause: Cause,
) -> Result<MapApplied, PlaceError> {
    apply_window(ctx, maps, grid, placement, mode, cause, None)
}

/// Puts a map into a realm whether or not its tracts are loaded
/// (10 §4.3): loaded cells are written now, as [`apply_map`] does, and
/// for each tract the map covers that is not loaded it is left as a
/// pending edit, applied when the tract next loads
/// ([`apply_pending_maps`]).
///
/// A tract that is neither loaded nor surveyed cannot hold a pending
/// edit: it is listed in [`MapApplied::unreached`].
pub fn place_map(
    ctx: &mut WorldCtx<'_>,
    maps: &MapDefs,
    realm: RealmId,
    placement: &Placement,
    mode: MapMode,
    cause: Cause,
) -> Result<MapApplied, PlaceError> {
    let grid = ctx.cosmos.realm_info(realm)?.base_grid;
    let mut applied = apply_map(ctx, maps, grid, placement, mode, cause)?;
    let Some(bounds) = applied.bounds else {
        return Ok(applied);
    };
    for tract in bounds.tracts() {
        let Ok(tract) = ctx.cosmos.normalize_tract(realm, tract) else {
            continue;
        };
        if ctx.cosmos.is_tract_loaded(realm, tract) || applied.pending.contains(&tract) {
            continue;
        }
        let edit = PendingEdit::Map(PendingMap {
            placement: placement.clone(),
            mode,
        });
        match ctx.cosmos.add_pending_edit(realm, tract, edit) {
            Ok(()) => applied.pending.push(tract),
            Err(_) => applied.unreached.push(tract),
        }
    }
    Ok(applied)
}

/// Writes the maps that were waiting for a tract, in the order they
/// were left, each clipped to the tract (02 §9.4 step 3; 10 §4.3).
/// Call it for each tract that has just loaded. The changes carry the
/// coarse cause, like every pending change.
///
/// A map that can no longer be resolved is dropped, and reported in
/// the result as not placed.
pub fn apply_pending_maps(
    ctx: &mut WorldCtx<'_>,
    maps: &MapDefs,
    realm: RealmId,
    tract: TractPos,
) -> Vec<MapApplied> {
    let Ok(info) = ctx.cosmos.realm_info(realm) else {
        return Vec::new();
    };
    let Ok(tract) = ctx.cosmos.normalize_tract(realm, tract) else {
        return Vec::new();
    };
    if !ctx.cosmos.is_tract_loaded(realm, tract) {
        return Vec::new();
    }
    let cause = Cause::new(CauseKind::Coarse);
    ctx.cosmos
        .take_pending_maps(realm, tract)
        .into_iter()
        .map(|pending| {
            apply_window(
                ctx,
                maps,
                info.base_grid,
                &pending.placement,
                pending.mode,
                cause,
                Some(tract),
            )
            .unwrap_or_default()
        })
        .collect()
}
