//! Areas: named volumes that mean something (02 §8.3). A vault, an
//! arena, a bubble of stopped time, a room that holds its air.
//!
//! An [`Area`] has a shape in one grid, tags, variables, and optionally a
//! kind, a clock of its own, an override of the ambient conditions inside
//! it and an anchor it moves with. Areas are found through a list for
//! each tract. An occupant is inside an area when any cell of its
//! footprint is, and the world reports its entering and leaving.
//!
//! | Need | Use |
//! |------|-----|
//! | Make, remove, move or reshape an area | [`WorldCtx::create_area`], [`WorldCtx::remove_area`], [`WorldCtx::move_area`], [`WorldCtx::set_area_shape`] |
//! | Make an area out of a closed space | [`WorldCtx::enclose`], [`Cosmos::enclosed_cells`] |
//! | Give an area a clock, or change its rate | [`WorldCtx::set_area_clock_rate`] |
//! | Change its tags, variables, override, priority or anchor | [`Cosmos::set_area_tag`], [`Cosmos::set_area_var`], [`Cosmos::set_area_ambient`], [`Cosmos::set_area_priority`], [`Cosmos::set_area_anchor`] |
//! | Read an area, or the areas at a cell | [`Cosmos::area`], [`Cosmos::areas`], [`Cosmos::areas_at`] |
//! | Who is inside | [`Cosmos::occupants_in_area`] |
//!
//! Where areas overlap, the one with the highest priority applies, and of
//! two with the same priority the one made first. That is decided
//! separately for gravity, for drift and for the clock, among the areas
//! that set each.

use std::collections::{BTreeMap, VecDeque};

use clash_core::{
    Dur, EntityId, IVec3, LookupMap, Real, StateHash, StateHasher, Value, VarKey, VarMap,
    define_id, sorted,
};
use clash_time::ClockId;
use serde::{Deserialize, Serialize};

use crate::cause::Cause;
use crate::cosmos::{Ambient, Cosmos, GridView};
use crate::defs::{AreaKindId, EdgeRule, EngineFlag, WorldDefs};
use crate::dims::{CHUNK_Z_BITS, STOREY_M};
use crate::edit::WorldCtx;
use crate::error::WorldError;
use crate::event::WorldEvent;
use crate::grid::Column;
use crate::occupancy::{OccupantFilter, Occupied};
use crate::pos::{CellBox, CellPos, GridId, Location, TractPos};
use crate::site::SiteId;
use crate::topology::{Extent, Topology};

define_id! {
    /// An area of the cosmos. Never reused.
    pub struct AreaId;
}

/// The most tracts one area may touch. Something larger is a realm, or a
/// change to a realm's clock.
pub const AREA_MAX_TRACTS: usize = 1 << 12;

/// The most cells an area may list one by one, and so the most an
/// enclosed space may hold.
pub const AREA_MAX_CELLS: u32 = 1 << 16;

/// The coverage at which a block stops whatever spreads from cell to
/// cell: fields (02 §16), and the fill that finds an enclosed space.
pub const FIELD_BLOCK_COVERAGE: Real = Real::of(0.95);

/// The cells an area covers, in its grid's coordinates.
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AreaShape {
    /// A box of cells.
    Box(CellBox),
    /// These cells, in order and without repeats. Make one with
    /// [`AreaShape::cells`].
    Cells(Vec<CellPos>),
    /// Every cell whose centre is within a distance of a cell's centre:
    /// metres, a storey counting three, the shorter way round on wrapped
    /// axes.
    Radius {
        /// The cell at the middle.
        centre: CellPos,
        /// The distance in metres.
        radius: Real,
    },
}

/// Whether a coordinate lies in `low..=high` along an axis, going round
/// from `low` on a wrapped one.
fn spans(extent: Extent, low: i32, high: i32, value: i32) -> bool {
    match extent {
        Extent::Wrapped(size) if size > 0 => {
            let size = i64::from(size);
            (i64::from(value) - i64::from(low)).rem_euclid(size) <= i64::from(high) - i64::from(low)
        }
        _ => low <= value && value <= high,
    }
}

impl AreaShape {
    /// The shape of exactly these cells, given in any order.
    pub fn cells(cells: impl IntoIterator<Item = CellPos>) -> AreaShape {
        AreaShape::Cells(sorted::from_unsorted(cells.into_iter().collect()))
    }

    /// Whether the shape covers a cell. The cell must be normalised.
    pub fn contains(&self, topology: &Topology, cell: CellPos) -> bool {
        match self {
            AreaShape::Box(cells) => {
                cells.min.z <= cell.z
                    && cell.z <= cells.max.z
                    && spans(topology.x, cells.min.x, cells.max.x, cell.x)
                    && spans(topology.y, cells.min.y, cells.max.y, cell.y)
            }
            AreaShape::Cells(cells) => sorted::contains(cells, &cell),
            AreaShape::Radius { centre, radius } => {
                let offset = topology.delta(*centre, cell);
                Real::from_i64(crate::pos::distance_sq_cells(offset)) <= *radius * *radius
            }
        }
    }

    /// A box that holds every cell of the shape, before any wrap. `None`
    /// for a shape of no cells.
    pub fn bounds(&self) -> Option<CellBox> {
        match self {
            AreaShape::Box(cells) => Some(*cells),
            AreaShape::Cells(cells) => {
                let first = *cells.first()?;
                Some(cells.iter().fold(CellBox::single(first), |bounds, cell| {
                    CellBox::new(
                        CellPos::new(
                            bounds.min.x.min(cell.x),
                            bounds.min.y.min(cell.y),
                            bounds.min.z.min(cell.z),
                        ),
                        CellPos::new(
                            bounds.max.x.max(cell.x),
                            bounds.max.y.max(cell.y),
                            bounds.max.z.max(cell.z),
                        ),
                    )
                }))
            }
            AreaShape::Radius { centre, radius } => {
                if *radius < Real::ZERO {
                    return None;
                }
                let reach = radius.floor().to_i32();
                let storeys = (*radius / STOREY_M).floor().to_i32();
                Some(CellBox::around(*centre, IVec3::new(reach, reach, storeys)))
            }
        }
    }

    /// The cell at the middle of the shape: of a box its middle, of a
    /// radius its centre, of listed cells the middle one of the list.
    pub fn centre(&self) -> Option<CellPos> {
        match self {
            AreaShape::Box(cells) => Some(CellPos::new(
                midpoint(cells.min.x, cells.max.x),
                midpoint(cells.min.y, cells.max.y),
                midpoint(cells.min.z, cells.max.z),
            )),
            AreaShape::Cells(cells) => cells.get(cells.len() / 2).copied(),
            AreaShape::Radius { centre, .. } => Some(*centre),
        }
    }

    /// The shape brought into a grid: a box's lowest corner and a radius's
    /// centre normalised, listed cells each normalised and put in order.
    /// Refused if the corner, the centre or a listed cell lies outside the
    /// grid, if a radius is not a finite distance of zero or more, or if
    /// more than [`AREA_MAX_CELLS`] cells are listed.
    fn settled(self, grid: GridId, topology: &Topology) -> Result<AreaShape, WorldError> {
        let inside = |cell: CellPos| {
            topology
                .normalize(cell)
                .map_err(|edge| WorldError::OutsideRealm {
                    loc: Location::new(grid, cell),
                    edge,
                })
        };
        match self {
            AreaShape::Box(cells) => {
                let min = inside(cells.min)?;
                Ok(AreaShape::Box(CellBox {
                    min,
                    max: cells.max.offset(min - cells.min),
                }))
            }
            AreaShape::Cells(cells) => {
                if cells.len() > AREA_MAX_CELLS as usize {
                    return Err(WorldError::TooLarge {
                        what: "the cells an area lists",
                        limit: u64::from(AREA_MAX_CELLS),
                    });
                }
                let cells = cells
                    .into_iter()
                    .map(inside)
                    .collect::<Result<Vec<_>, _>>()?;
                Ok(AreaShape::cells(cells))
            }
            AreaShape::Radius { centre, radius } => {
                if radius < Real::ZERO || !radius.is_finite() {
                    return Err(WorldError::BadValue {
                        what: "the radius of an area",
                        why: "it must be a finite distance of zero or more",
                    });
                }
                Ok(AreaShape::Radius {
                    centre: inside(centre)?,
                    radius,
                })
            }
        }
    }

    /// The shape moved by an offset, before it is settled again.
    fn shifted(&self, by: IVec3) -> AreaShape {
        match self {
            AreaShape::Box(cells) => AreaShape::Box(cells.offset(by)),
            AreaShape::Cells(cells) => {
                AreaShape::Cells(cells.iter().map(|cell| cell.offset(by)).collect())
            }
            AreaShape::Radius { centre, radius } => AreaShape::Radius {
                centre: centre.offset(by),
                radius: *radius,
            },
        }
    }

    /// The tracts the shape touches, normalised, in order. Refused if
    /// there are more than [`AREA_MAX_TRACTS`].
    fn tracts(&self, topology: &Topology) -> Result<Vec<TractPos>, WorldError> {
        let too_many = WorldError::TooLarge {
            what: "the tracts an area touches",
            limit: AREA_MAX_TRACTS as u64,
        };
        let tracts: Vec<TractPos> = match self {
            AreaShape::Cells(cells) => cells.iter().map(|cell| cell.tract()).collect(),
            AreaShape::Box(_) | AreaShape::Radius { .. } => {
                let Some(bounds) = self.bounds() else {
                    return Ok(Vec::new());
                };
                let (first, last) = (bounds.min.tract(), bounds.max.tract());
                let across = (i64::from(last.x) - i64::from(first.x) + 1) as u64;
                let down = (i64::from(last.y) - i64::from(first.y) + 1) as u64;
                if across.saturating_mul(down) > AREA_MAX_TRACTS as u64 {
                    return Err(too_many);
                }
                let z = bounds.min.z.clamp(topology.z_min, topology.z_max);
                bounds
                    .tracts()
                    .filter_map(|tract| topology.normalize(tract.origin(z)).ok())
                    .map(|origin| origin.tract())
                    .collect()
            }
        };
        let tracts = sorted::from_unsorted(tracts);
        if tracts.len() > AREA_MAX_TRACTS {
            return Err(too_many);
        }
        Ok(tracts)
    }
}

/// The whole number half way between two others, rounded down.
fn midpoint(low: i32, high: i32) -> i32 {
    ((i64::from(low) + i64::from(high)).div_euclid(2)) as i32
}

impl StateHash for AreaShape {
    fn state_hash(&self, hasher: &mut StateHasher) {
        match self {
            AreaShape::Box(cells) => {
                hasher.write_u8(0);
                cells.state_hash(hasher);
            }
            AreaShape::Cells(cells) => {
                hasher.write_u8(1);
                cells.state_hash(hasher);
            }
            AreaShape::Radius { centre, radius } => {
                hasher.write_u8(2);
                centre.state_hash(hasher);
                radius.state_hash(hasher);
            }
        }
    }
}

/// What an area moves with.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AreaAnchor {
    /// An entity. While it is an occupant, the area moves when it does.
    Entity(EntityId),
    /// A subgrid. Stored only, until subgrids are built.
    Subgrid(GridId),
}

impl StateHash for AreaAnchor {
    fn state_hash(&self, hasher: &mut StateHasher) {
        match self {
            AreaAnchor::Entity(entity) => {
                hasher.write_u8(0);
                entity.state_hash(hasher);
            }
            AreaAnchor::Subgrid(grid) => {
                hasher.write_u8(1);
                grid.state_hash(hasher);
            }
        }
    }
}

/// Gravity inside an area, in place of its stratum's.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GravityOverride {
    /// The stratum's gravity times this. Zero is weightlessness.
    Scale(Real),
    /// This acceleration, in m/s² east, south and up, whatever the
    /// stratum says.
    Vector([Real; 3]),
}

impl StateHash for GravityOverride {
    fn state_hash(&self, hasher: &mut StateHasher) {
        match self {
            GravityOverride::Scale(scale) => {
                hasher.write_u8(0);
                scale.state_hash(hasher);
            }
            GravityOverride::Vector(vector) => {
                hasher.write_u8(1);
                vector.state_hash(hasher);
            }
        }
    }
}

/// How the medium inside an area is moving (11 §4.7).
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Drift {
    /// A steady flow: a velocity in m/s east, south and up.
    Flow([Real; 3]),
    /// A whirl about the middle of the area.
    Vortex {
        /// How fast the medium goes round, in m/s.
        speed: Real,
        /// How fast it is drawn toward the middle, in m/s.
        inflow: Real,
        /// How fast it rises, in m/s.
        lift: Real,
    },
}

impl StateHash for Drift {
    fn state_hash(&self, hasher: &mut StateHasher) {
        match self {
            Drift::Flow(velocity) => {
                hasher.write_u8(0);
                velocity.state_hash(hasher);
            }
            Drift::Vortex {
                speed,
                inflow,
                lift,
            } => {
                hasher.write_u8(1);
                speed.state_hash(hasher);
                inflow.state_hash(hasher);
                lift.state_hash(hasher);
            }
        }
    }
}

/// What an area changes about the ambient conditions inside it.
#[derive(Clone, Copy, PartialEq, Eq, Default, Debug, Serialize, Deserialize)]
pub struct AmbientOverride {
    /// Gravity, as a scale or a vector. `None` leaves the stratum's.
    pub gravity: Option<GravityOverride>,
    /// How the medium is moving. `None` sets nothing.
    pub drift: Option<Drift>,
}

impl StateHash for AmbientOverride {
    fn state_hash(&self, hasher: &mut StateHasher) {
        self.gravity.state_hash(hasher);
        self.drift.state_hash(hasher);
    }
}

/// The drift an area sets at a cell, as [`Cosmos::ambient`] reports it.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct AreaDrift {
    /// The area that sets it.
    pub area: AreaId,
    /// The flow or the vortex.
    pub drift: Drift,
    /// The middle of the area, which a vortex turns about.
    pub centre: CellPos,
}

/// How an area made by [`WorldCtx::enclose`] was made, and whether the
/// space still closes.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub struct Enclosure {
    /// The cell the space was filled from.
    pub from: CellPos,
    /// The most cells the space may hold.
    pub limit: u32,
    /// Whether the space no longer closes. A breached area keeps the
    /// shape it last had.
    pub breached: bool,
    /// A cell through which it leaks, while it is breached.
    pub leak: Option<CellPos>,
}

impl StateHash for Enclosure {
    fn state_hash(&self, hasher: &mut StateHasher) {
        self.from.state_hash(hasher);
        hasher.write_u32(self.limit);
        hasher.write_bool(self.breached);
        self.leak.state_hash(hasher);
    }
}

/// A named volume with tags, variables and optionally its own clock
/// (02 §8.3).
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub struct Area {
    /// The area's id.
    pub id: AreaId,
    /// The definition that supplied its defaults and carries its rules.
    pub kind: Option<AreaKindId>,
    /// The grid it is in.
    pub grid: GridId,
    /// The cells it covers.
    pub shape: AreaShape,
    /// What it moves with.
    pub anchor: Option<AreaAnchor>,
    /// What it changes about the ambient conditions inside it.
    pub ambient: Option<AmbientOverride>,
    /// For how long cell changes made inside it are kept (08 §7.4).
    /// Stored only, until the journal is built.
    pub journal: Option<Dur>,
    /// Its tags, in order.
    pub tags: Vec<String>,
    /// Its variables.
    pub vars: VarMap,
    /// Its clock. Things inside run on it.
    pub clock: Option<ClockId>,
    /// The site it belongs to.
    pub site: Option<SiteId>,
    /// Which area applies where several overlap: the highest.
    pub priority: i32,
    /// How it was made, if it was made by enclosing a space.
    pub enclosure: Option<Enclosure>,
    /// A source of light it carries at the middle of its shape, moving
    /// with it (15 §3.3). A negative intensity darkens.
    #[serde(default)]
    pub light: Option<crate::light::LightSource>,
}

impl Area {
    /// Whether the area carries a tag.
    pub fn has_tag(&self, tag: &str) -> bool {
        self.tags
            .binary_search_by(|have| have.as_str().cmp(tag))
            .is_ok()
    }

    /// Whether the area was made by enclosing a space that no longer
    /// closes.
    pub fn is_breached(&self) -> bool {
        self.enclosure.is_some_and(|enclosure| enclosure.breached)
    }
}

clash_core::impl_state_hash!(Area {
    id,
    kind,
    grid,
    shape,
    anchor,
    ambient,
    journal,
    tags,
    vars,
    clock,
    site,
    priority,
    enclosure,
    light
});

/// What to make an area from.
///
/// ```
/// use clash_core::Real;
/// use clash_world::{AreaShape, AreaSpec, CellPos, GridId};
///
/// let bubble = AreaSpec::new(
///     GridId::new(0),
///     AreaShape::Radius { centre: CellPos::new(40, 40, 0), radius: Real::of(6.0) },
/// );
/// assert!(bubble.kind.is_none());
/// ```
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub struct AreaSpec {
    /// The definition supplying default tags, variables, clock rate and
    /// priority.
    pub kind: Option<AreaKindId>,
    /// The grid it is in.
    pub grid: GridId,
    /// The cells it covers.
    pub shape: AreaShape,
    /// What it moves with.
    pub anchor: Option<AreaAnchor>,
    /// What it changes about the ambient conditions inside it.
    pub ambient: Option<AmbientOverride>,
    /// For how long cell changes made inside it are kept.
    pub journal: Option<Dur>,
    /// Tags, added to its kind's.
    pub tags: Vec<String>,
    /// Variables, laid over its kind's.
    pub vars: VarMap,
    /// The rate of a clock of its own, relative to its realm's. `None`
    /// takes its kind's, and with neither it has no clock.
    pub clock_rate: Option<Real>,
    /// Which area applies where several overlap. `None` takes its
    /// kind's, or zero.
    pub priority: Option<i32>,
    /// The site it belongs to.
    pub site: Option<SiteId>,
    /// A source of light it carries at the middle of its shape. `None`
    /// takes its kind's, if any.
    pub light: Option<crate::light::LightSource>,
}

impl AreaSpec {
    /// An area of a shape in a grid, with nothing else said about it.
    pub fn new(grid: GridId, shape: AreaShape) -> AreaSpec {
        AreaSpec {
            kind: None,
            grid,
            shape,
            anchor: None,
            ambient: None,
            journal: None,
            tags: Vec::new(),
            vars: VarMap::new(),
            clock_rate: None,
            priority: None,
            site: None,
            light: None,
        }
    }
}

/// Every area of the cosmos, with the lists that find them (02 §8.3).
///
/// The lists are derived from the areas: the areas touching each tract,
/// the areas anchored to each entity. Only the areas themselves are
/// state.
#[derive(Clone, PartialEq, Eq, Debug, Default, Serialize, Deserialize)]
pub struct AreaTable {
    areas: BTreeMap<AreaId, Area>,
    /// The areas whose shape touches each tract, in id order.
    by_tract: LookupMap<(GridId, TractPos), Vec<AreaId>>,
    /// The areas anchored to each entity, in id order.
    anchored: BTreeMap<EntityId, Vec<AreaId>>,
    /// How many areas were made by enclosing a space.
    enclosures: u32,
    next: u32,
    /// The areas that carry a light, in id order.
    #[serde(default)]
    lit: Vec<AreaId>,
}

impl AreaTable {
    /// A table of no areas.
    pub fn new() -> AreaTable {
        AreaTable::default()
    }

    /// Whether there are no areas.
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.areas.is_empty()
    }

    /// How many areas there are.
    pub fn len(&self) -> usize {
        self.areas.len()
    }

    /// An area by id.
    pub fn get(&self, id: AreaId) -> Option<&Area> {
        self.areas.get(&id)
    }

    /// Every area, in id order.
    pub fn iter(&self) -> impl Iterator<Item = &Area> + '_ {
        self.areas.values()
    }

    /// The areas whose shape touches a tract of a grid, in id order. An
    /// area listed here need not cover any particular cell of the tract.
    pub fn in_tract(&self, grid: GridId, tract: TractPos) -> &[AreaId] {
        self.by_tract.get(&(grid, tract)).map_or(&[], Vec::as_slice)
    }

    /// The areas that cover a cell of a grid, in id order. The cell must
    /// be normalised.
    pub fn at<'a>(
        &'a self,
        topology: &'a Topology,
        grid: GridId,
        cell: CellPos,
    ) -> impl Iterator<Item = &'a Area> + 'a {
        self.in_tract(grid, cell.tract())
            .iter()
            .filter_map(move |id| self.areas.get(id))
            .filter(move |area| area.shape.contains(topology, cell))
    }

    /// The areas anchored to an entity, in id order.
    pub fn anchored_to(&self, entity: EntityId) -> &[AreaId] {
        self.anchored.get(&entity).map_or(&[], Vec::as_slice)
    }

    /// The areas that carry a source of light, in id order.
    pub fn lit(&self) -> &[AreaId] {
        &self.lit
    }

    /// Replaces the kind of every area by what `turn` makes of it: how
    /// the ids of definitions are turned into save ids and back
    /// (02 §11).
    pub(crate) fn map_kinds(&mut self, turn: &mut dyn FnMut(AreaKindId) -> AreaKindId) {
        for area in self.areas.values_mut() {
            if let Some(kind) = area.kind.as_mut() {
                *kind = turn(*kind);
            }
        }
    }

    /// Whether any area was made by enclosing a space.
    pub(crate) fn has_enclosures(&self) -> bool {
        self.enclosures > 0
    }

    fn list(&mut self, id: AreaId, grid: GridId, tracts: &[TractPos]) {
        for tract in tracts {
            sorted::insert(
                self.by_tract.get_or_insert_with((grid, *tract), Vec::new),
                id,
            );
        }
    }

    fn unlist(&mut self, id: AreaId, grid: GridId, tracts: &[TractPos]) {
        for tract in tracts {
            let key = (grid, *tract);
            if let Some(list) = self.by_tract.get_mut(&key) {
                sorted::remove(list, &id);
                if list.is_empty() {
                    self.by_tract.remove(&key);
                }
            }
        }
    }

    fn set_anchor_list(&mut self, id: AreaId, old: Option<AreaAnchor>, new: Option<AreaAnchor>) {
        if let Some(AreaAnchor::Entity(entity)) = old
            && let Some(list) = self.anchored.get_mut(&entity)
        {
            sorted::remove(list, &id);
            if list.is_empty() {
                self.anchored.remove(&entity);
            }
        }
        if let Some(AreaAnchor::Entity(entity)) = new {
            sorted::insert(self.anchored.entry(entity).or_default(), id);
        }
    }

    /// Applies what the areas covering a cell say about its ambient
    /// conditions: for gravity, drift and the clock separately, the area
    /// with the highest priority among those that set it, and of equals
    /// the one made first.
    pub(crate) fn apply_ambient(
        &self,
        topology: &Topology,
        grid: GridId,
        cell: CellPos,
        ambient: &mut Ambient,
    ) {
        let mut gravity: Option<(i32, GravityOverride)> = None;
        let mut drift: Option<(i32, AreaDrift)> = None;
        let mut clock: Option<(i32, ClockId)> = None;
        // In id order, so a later area must be strictly higher to win.
        for area in self.at(topology, grid, cell) {
            let higher = |best: Option<i32>| best.is_none_or(|best| area.priority > best);
            if let Some(over) = area.ambient {
                if let Some(set) = over.gravity
                    && higher(gravity.map(|best| best.0))
                {
                    gravity = Some((area.priority, set));
                }
                if let Some(set) = over.drift
                    && higher(drift.map(|best| best.0))
                {
                    let centre = area.shape.centre().unwrap_or(cell);
                    drift = Some((
                        area.priority,
                        AreaDrift {
                            area: area.id,
                            drift: set,
                            centre,
                        },
                    ));
                }
            }
            if let Some(own) = area.clock
                && higher(clock.map(|best| best.0))
            {
                clock = Some((area.priority, own));
            }
        }
        match gravity {
            Some((_, GravityOverride::Scale(scale))) => {
                ambient.gravity *= scale;
                ambient.gravity_vector = [Real::ZERO, Real::ZERO, -ambient.gravity];
            }
            Some((_, GravityOverride::Vector(vector))) => {
                ambient.gravity = -vector[2];
                ambient.gravity_vector = vector;
            }
            None => {}
        }
        if let Some((_, set)) = drift {
            ambient.drift = Some(set);
        }
        if let Some((_, own)) = clock {
            ambient.clock = own;
        }
    }
}

/// Hashes the areas in id order and the next id. The lists that find
/// areas are derived and are not hashed.
impl StateHash for AreaTable {
    fn state_hash(&self, hasher: &mut StateHasher) {
        self.areas.state_hash(hasher);
        hasher.write_u32(self.next);
    }
}

/// The occupants with a cell inside a shape, in id order.
fn occupants_in_shape(
    view: &GridView<'_>,
    tracts: &[TractPos],
    shape: &AreaShape,
) -> Vec<EntityId> {
    let mut ids = Vec::new();
    let Some(bounds) = shape.bounds() else {
        return ids;
    };
    let topology = view.topology();
    for tract in tracts {
        let Some(column) = view.column(*tract).filter(|column| column.has_occupants()) else {
            continue;
        };
        let origin = tract.origin(0);
        for (chunk_z, index) in
            column.occupant_indices(bounds.min.z >> CHUNK_Z_BITS, bounds.max.z >> CHUNK_Z_BITS)
        {
            for entry in index.iter() {
                let cell = CellPos::new(
                    origin.x | entry.cell.lx(),
                    origin.y | entry.cell.ly(),
                    (chunk_z << CHUNK_Z_BITS) | entry.cell.lz(),
                );
                if shape.contains(topology, cell) {
                    sorted::insert(&mut ids, entry.id);
                }
            }
        }
    }
    ids
}

impl Cosmos {
    /// An area by id.
    pub fn area(&self, id: AreaId) -> Option<&Area> {
        self.areas.get(id)
    }

    /// Every area, in id order.
    pub fn areas(&self) -> impl Iterator<Item = &Area> + '_ {
        self.areas.iter()
    }

    /// The table of areas.
    pub fn area_table(&self) -> &AreaTable {
        &self.areas
    }

    /// The areas that cover a cell, in id order. The cell must be inside
    /// its grid; its tract need not be loaded.
    pub fn areas_at(&self, loc: Location) -> Result<Vec<AreaId>, WorldError> {
        self.grid(loc.grid)?.areas_at(loc.cell)
    }

    /// The occupants inside an area that the filter lets through, by
    /// cell and then by id: each is listed for every one of its cells
    /// that the area covers.
    pub fn occupants_in_area(
        &self,
        id: AreaId,
        filter: &OccupantFilter,
    ) -> Result<Vec<Occupied>, WorldError> {
        let area = self.area(id).ok_or(WorldError::UnknownArea(id))?;
        let view = self.grid(area.grid)?;
        let Some(bounds) = area.shape.bounds() else {
            return Ok(Vec::new());
        };
        let topology = view.topology();
        let mut out = Vec::new();
        for tract in area.shape.tracts(topology)? {
            let Some(column) = view.column(tract).filter(|column| column.has_occupants()) else {
                continue;
            };
            let origin = tract.origin(0);
            for (chunk_z, index) in
                column.occupant_indices(bounds.min.z >> CHUNK_Z_BITS, bounds.max.z >> CHUNK_Z_BITS)
            {
                for entry in index.iter() {
                    let cell = CellPos::new(
                        origin.x | entry.cell.lx(),
                        origin.y | entry.cell.ly(),
                        (chunk_z << CHUNK_Z_BITS) | entry.cell.lz(),
                    );
                    if filter.admits(entry.id, entry.kind, entry.plane, entry.flags)
                        && area.shape.contains(topology, cell)
                    {
                        out.push(Occupied {
                            cell,
                            id: entry.id,
                            kind: entry.kind,
                            plane: entry.plane,
                            flags: entry.flags,
                        });
                    }
                }
            }
        }
        out.sort_by_key(|occupied| (occupied.cell, occupied.id));
        Ok(out)
    }

    /// Sets or clears a variable of an area and returns the value it had.
    pub fn set_area_var(
        &mut self,
        id: AreaId,
        key: VarKey,
        value: Option<Value>,
    ) -> Result<Option<Value>, WorldError> {
        let area = self
            .areas
            .areas
            .get_mut(&id)
            .ok_or(WorldError::UnknownArea(id))?;
        Ok(match value {
            Some(value) => area.vars.set(key, value),
            None => area.vars.remove(key.as_str()),
        })
    }

    /// Gives an area a tag or takes it away. Returns whether that changed
    /// anything.
    pub fn set_area_tag(&mut self, id: AreaId, tag: &str, on: bool) -> Result<bool, WorldError> {
        let area = self
            .areas
            .areas
            .get_mut(&id)
            .ok_or(WorldError::UnknownArea(id))?;
        Ok(if on {
            sorted::insert(&mut area.tags, tag.to_owned())
        } else {
            match area.tags.binary_search_by(|have| have.as_str().cmp(tag)) {
                Ok(at) => {
                    area.tags.remove(at);
                    true
                }
                Err(_) => false,
            }
        })
    }

    /// Sets what an area changes about the ambient conditions inside it.
    ///
    /// This asks for no stability check. A change of gravity made with
    /// [`WorldCtx::set_area_ambient`] does (02 §14.5).
    pub fn set_area_ambient(
        &mut self,
        id: AreaId,
        ambient: Option<AmbientOverride>,
    ) -> Result<(), WorldError> {
        let area = self
            .areas
            .areas
            .get_mut(&id)
            .ok_or(WorldError::UnknownArea(id))?;
        area.ambient = ambient;
        Ok(())
    }

    /// Sets which area applies where this one overlaps others.
    pub fn set_area_priority(&mut self, id: AreaId, priority: i32) -> Result<(), WorldError> {
        let area = self
            .areas
            .areas
            .get_mut(&id)
            .ok_or(WorldError::UnknownArea(id))?;
        area.priority = priority;
        Ok(())
    }

    /// Sets what an area moves with. The area does not move now: it moves
    /// when its anchor next does.
    pub fn set_area_anchor(
        &mut self,
        id: AreaId,
        anchor: Option<AreaAnchor>,
    ) -> Result<(), WorldError> {
        let area = self
            .areas
            .areas
            .get_mut(&id)
            .ok_or(WorldError::UnknownArea(id))?;
        let old = core::mem::replace(&mut area.anchor, anchor);
        self.areas.set_anchor_list(id, old, anchor);
        Ok(())
    }

    /// The cells of the closed space around a cell: every cell reached
    /// from it by steps north, east, south, west, up and down, in order
    /// and without repeats.
    ///
    /// The fill is stopped by what stops a field (02 §16). It does not
    /// enter a cell whose block covers [`FIELD_BLOCK_COVERAGE`] of it or
    /// more, and it passes between storeys only where the upper cell has
    /// no floor or a floor that is a grate. A barrier edge of the realm
    /// closes the space like a wall.
    ///
    /// Refused if the space does not close within `limit` cells
    /// ([`WorldError::NotEnclosed`]). The refusal names a cell through
    /// which it leaks: the cell from which the fill would cross a `void`
    /// edge or a boundary link; or else the first cell under open sky
    /// that the fill reached; or else the first cell past the limit.
    /// Also refused if the starting cell itself is covered
    /// ([`WorldError::NothingToEnclose`]), or the fill reaches a cell
    /// that is not loaded.
    pub fn enclosed_cells(&self, from: Location, limit: u32) -> Result<Vec<CellPos>, WorldError> {
        let view = self.grid(from.grid)?;
        let start = view.normalize(from.cell)?;
        let kind = self.realm_kind(view.realm());
        fill_space(&view, self.defs(), start, limit, |edge| {
            kind.is_some_and(|kind| matches!(kind.edge(edge), EdgeRule::Void | EdgeRule::Link(_)))
        })
    }
}

/// Whether whatever spreads from cell to cell can be in a cell.
fn admits(defs: &WorldDefs, column: &Column, cell: CellPos) -> bool {
    defs.block_info(column.block(cell)).coverage < FIELD_BLOCK_COVERAGE
}

/// Whether a cell's floor stops what would pass between it and the cell
/// below.
fn floor_stops(defs: &WorldDefs, column: &Column, cell: CellPos) -> bool {
    let floor = column.floor(cell);
    !floor.is_none() && !defs.floor_info(floor).flags.has(EngineFlag::Grate)
}

/// What a change to a cell means for a fill passing through it: whether
/// the cell admits the fill, and whether its floor lets it through.
pub(crate) fn passage(defs: &WorldDefs, column: &Column, cell: CellPos) -> (bool, bool) {
    (admits(defs, column, cell), !floor_stops(defs, column, cell))
}

/// The steps a fill takes from a cell, in order: north, east, south,
/// west, up, down.
const FILL_STEPS: [IVec3; 6] = [
    IVec3::new(0, -1, 0),
    IVec3::new(1, 0, 0),
    IVec3::new(0, 1, 0),
    IVec3::new(-1, 0, 0),
    IVec3::new(0, 0, 1),
    IVec3::new(0, 0, -1),
];

fn fill_space(
    view: &GridView<'_>,
    defs: &WorldDefs,
    start: CellPos,
    limit: u32,
    edge_leaks: impl Fn(crate::topology::Edge) -> bool,
) -> Result<Vec<CellPos>, WorldError> {
    let grid = view.id();
    let at = |cell: CellPos| Location::new(grid, cell);
    let column_of = |cell: CellPos| {
        view.column(cell.tract())
            .ok_or(WorldError::NotLoaded(at(cell)))
    };
    let first = column_of(start)?;
    if !admits(defs, first, start) {
        return Err(WorldError::NothingToEnclose(at(start)));
    }
    if limit == 0 {
        return Err(WorldError::NotEnclosed { leak: at(start) });
    }
    let limit = limit as usize;
    let topology = view.topology();
    let mut seen: LookupMap<CellPos, ()> = LookupMap::new();
    let mut cells = vec![start];
    let mut queue = VecDeque::from([start]);
    seen.insert(start, ());
    let mut sky = (start.z >= first.height_at(start)).then_some(start);
    while let Some(cell) = queue.pop_front() {
        let here = column_of(cell)?;
        for step in FILL_STEPS {
            let next = match topology.normalize(cell.offset(step)) {
                Ok(next) => next,
                Err(edge) if edge_leaks(edge) => {
                    return Err(WorldError::NotEnclosed { leak: at(cell) });
                }
                // A barrier closes the space like a wall.
                Err(_) => continue,
            };
            if seen.contains_key(&next) {
                continue;
            }
            let there = if next.tract() == cell.tract() {
                here
            } else {
                column_of(next)?
            };
            // A floor lies at the base of its cell: going up it is the
            // upper cell's, going down this one's.
            if (step.z > 0 && floor_stops(defs, there, next))
                || (step.z < 0 && floor_stops(defs, here, cell))
                || !admits(defs, there, next)
            {
                continue;
            }
            if cells.len() >= limit {
                return Err(WorldError::NotEnclosed {
                    leak: at(sky.unwrap_or(next)),
                });
            }
            if sky.is_none() && next.z >= there.height_at(next) {
                sky = Some(next);
            }
            seen.insert(next, ());
            cells.push(next);
            queue.push_back(next);
        }
    }
    cells.sort_unstable();
    Ok(cells)
}

impl WorldCtx<'_> {
    /// Makes an area. Its tags are its kind's and the spec's, its
    /// variables its kind's with the spec's laid over them. If the spec
    /// or the kind gives a clock rate, the area gets a clock of its own
    /// under its realm's.
    ///
    /// Emits `AreaEntered` for every occupant inside it, in id order.
    ///
    /// Refused if the grid, the kind or the site is unknown, the shape
    /// lies outside the grid or touches more than [`AREA_MAX_TRACTS`]
    /// tracts, or the clock rate is not zero or more.
    pub fn create_area(&mut self, spec: AreaSpec, cause: Cause) -> Result<AreaId, WorldError> {
        self.make_area(spec, None, cause)
    }

    fn make_area(
        &mut self,
        spec: AreaSpec,
        enclosure: Option<Enclosure>,
        cause: Cause,
    ) -> Result<AreaId, WorldError> {
        let view = self.cosmos.grid(spec.grid)?;
        let topology = *view.topology();
        let realm_clock = view.clock();
        let defs = std::sync::Arc::clone(self.cosmos.defs());
        let kind = match spec.kind {
            Some(id) => Some(
                defs.area_kinds()
                    .get(id)
                    .ok_or(WorldError::UnknownAreaKind(id))?,
            ),
            None => None,
        };
        if let Some(site) = spec.site
            && self.cosmos.site(site).is_none()
        {
            return Err(WorldError::UnknownSite(site));
        }
        let shape = spec.shape.settled(spec.grid, &topology)?;
        let tracts = shape.tracts(&topology)?;
        let next = self
            .cosmos
            .areas
            .next
            .checked_add(1)
            .ok_or(WorldError::IdsExhausted("area"))?;
        let clock = match spec.clock_rate.or(kind.and_then(|kind| kind.clock_rate)) {
            Some(rate) => Some(self.timeline.create_clock(realm_clock, rate)?),
            None => None,
        };

        let mut tags = kind.map(|kind| kind.tags.clone()).unwrap_or_default();
        for tag in spec.tags {
            sorted::insert(&mut tags, tag);
        }
        let mut vars = kind.map(|kind| kind.vars.clone()).unwrap_or_default();
        for (key, value) in spec.vars.iter() {
            vars.set(key.clone(), value.clone());
        }
        let id = AreaId::new(self.cosmos.areas.next);
        // The kind's gravity, unless the area sets its own.
        let ambient = match (spec.ambient, kind.and_then(|kind| kind.gravity)) {
            (Some(mut own), Some(gravity)) => {
                own.gravity = own.gravity.or(Some(gravity));
                Some(own)
            }
            (None, Some(gravity)) => Some(AmbientOverride {
                gravity: Some(gravity),
                drift: None,
            }),
            (own, None) => own,
        };
        let light = spec.light.or(kind.and_then(|kind| kind.light));
        let area = Area {
            id,
            kind: spec.kind,
            grid: spec.grid,
            shape,
            anchor: spec.anchor,
            ambient,
            journal: spec.journal,
            tags,
            vars,
            clock,
            site: spec.site,
            priority: spec
                .priority
                .or(kind.map(|kind| kind.priority))
                .unwrap_or(0),
            enclosure,
            light,
        };

        let table = &mut self.cosmos.areas;
        table.next = next;
        table.list(id, area.grid, &tracts);
        table.set_anchor_list(id, None, area.anchor);
        if area.light.is_some() {
            sorted::insert(&mut table.lit, id);
        }
        if enclosure.is_some() {
            table.enclosures += 1;
        }
        let (grid, weighs, timed) = (
            area.grid,
            area.ambient.is_some_and(|over| over.gravity.is_some()),
            area.clock.is_some(),
        );
        let covers = (weighs || timed).then(|| area.shape.clone());
        table.areas.insert(id, area);
        self.settle_members(id, &[], cause);
        if let Some(shape) = covers {
            // What stands in new gravity is checked, and what is falling
            // inside a new clock goes onto it.
            self.area_changed(grid, &shape, weighs, timed, cause);
        }
        Ok(id)
    }

    /// Removes an area and returns it. Its clock, if it had one, is
    /// destroyed: whatever was waiting on it goes back to the clock
    /// above.
    ///
    /// Emits `AreaExited` for every occupant that was inside it, in id
    /// order.
    pub fn remove_area(&mut self, id: AreaId, cause: Cause) -> Result<Area, WorldError> {
        let area = self.cosmos.area(id).ok_or(WorldError::UnknownArea(id))?;
        let grid = area.grid;
        let view = self.cosmos.grid(grid)?;
        let tracts = area.shape.tracts(view.topology())?;
        let inside = occupants_in_shape(&view, &tracts, &area.shape);

        let table = &mut self.cosmos.areas;
        let Some(area) = table.areas.remove(&id) else {
            return Err(WorldError::UnknownArea(id));
        };
        table.unlist(id, grid, &tracts);
        table.set_anchor_list(id, area.anchor, None);
        sorted::remove(&mut table.lit, &id);
        if area.enclosure.is_some() {
            table.enclosures = table.enclosures.saturating_sub(1);
        }
        let tick = self.timeline.now();
        for occupant in inside {
            let Some(record) = self.cosmos.occupants.get_mut(&occupant) else {
                continue;
            };
            if sorted::remove(&mut record.areas, &id) {
                self.events.emit(WorldEvent::AreaExited {
                    tick,
                    area: id,
                    occupant,
                    loc: record.loc,
                    clock: area.clock,
                    cause,
                });
            }
        }
        if let Some(clock) = area.clock {
            // A clock that is already gone needs no destroying.
            let _ = self.timeline.destroy_clock(clock);
        }
        let weighs = area.ambient.is_some_and(|over| over.gravity.is_some());
        if weighs || area.clock.is_some() {
            self.area_changed(grid, &area.shape, weighs, area.clock.is_some(), cause);
        }
        Ok(area)
    }

    /// Moves an area by an offset within its grid. What it now covers
    /// and no longer covers is worked out, and `AreaExited` and then
    /// `AreaEntered` are emitted for the occupants concerned, each in id
    /// order, as when an occupant moves.
    ///
    /// Refused, changing nothing, if a box's corner, a radius's centre
    /// or a listed cell would leave the grid.
    pub fn move_area(&mut self, id: AreaId, by: IVec3, cause: Cause) -> Result<(), WorldError> {
        let area = self.cosmos.area(id).ok_or(WorldError::UnknownArea(id))?;
        let (grid, shape) = (area.grid, area.shape.shifted(by));
        self.reshape_area(id, grid, shape, cause)
    }

    /// Gives an area another shape in its grid. Events are emitted as
    /// for [`move_area`](WorldCtx::move_area).
    pub fn set_area_shape(
        &mut self,
        id: AreaId,
        shape: AreaShape,
        cause: Cause,
    ) -> Result<(), WorldError> {
        let grid = self
            .cosmos
            .area(id)
            .ok_or(WorldError::UnknownArea(id))?
            .grid;
        self.reshape_area(id, grid, shape, cause)
    }

    /// Moves an area along with its anchor: by an offset, into the grid
    /// the anchor is now in.
    pub(crate) fn carry_area(
        &mut self,
        id: AreaId,
        grid: GridId,
        by: IVec3,
        cause: Cause,
    ) -> Result<(), WorldError> {
        let area = self.cosmos.area(id).ok_or(WorldError::UnknownArea(id))?;
        let shape = area.shape.shifted(by);
        self.reshape_area(id, grid, shape, cause)
    }

    fn reshape_area(
        &mut self,
        id: AreaId,
        grid: GridId,
        shape: AreaShape,
        cause: Cause,
    ) -> Result<(), WorldError> {
        let area = self.cosmos.area(id).ok_or(WorldError::UnknownArea(id))?;
        let old_grid = area.grid;
        let new_view = self.cosmos.grid(grid)?;
        let shape = shape.settled(grid, new_view.topology())?;
        if grid == old_grid && shape == area.shape {
            return Ok(());
        }
        let new_tracts = shape.tracts(new_view.topology())?;
        let old_view = self.cosmos.grid(old_grid)?;
        let old_tracts = area.shape.tracts(old_view.topology())?;
        let before = occupants_in_shape(&old_view, &old_tracts, &area.shape);
        let (weighs, timed) = (
            area.ambient.is_some_and(|over| over.gravity.is_some()),
            area.clock.is_some(),
        );
        let covered = (weighs || timed).then(|| (area.shape.clone(), shape.clone()));

        let table = &mut self.cosmos.areas;
        table.unlist(id, old_grid, &old_tracts);
        table.list(id, grid, &new_tracts);
        if let Some(area) = table.areas.get_mut(&id) {
            area.grid = grid;
            area.shape = shape;
        }
        self.settle_members(id, &before, cause);
        if let Some((old_shape, new_shape)) = covered {
            // Gravity and the clock changed where it was and where it is.
            self.area_changed(old_grid, &old_shape, weighs, timed, cause);
            self.area_changed(grid, &new_shape, weighs, false, cause);
        }
        Ok(())
    }

    /// Brings the record of who is inside an area up to date after the
    /// area was made or changed, and emits the events. `before` is who
    /// had a cell in its shape before the change.
    fn settle_members(&mut self, id: AreaId, before: &[EntityId], cause: Cause) {
        let Some(area) = self.cosmos.areas.get(id) else {
            return;
        };
        let clock = area.clock;
        let Ok(view) = self.cosmos.grid(area.grid) else {
            return;
        };
        let Ok(tracts) = area.shape.tracts(view.topology()) else {
            return;
        };
        let after = occupants_in_shape(&view, &tracts, &area.shape);
        let tick = self.timeline.now();
        // Those who left, then those who came, each in id order. What
        // each occupant's record says decides: one already listed as
        // inside has not entered.
        for occupant in before.iter().filter(|who| !sorted::contains(&after, who)) {
            let Some(record) = self.cosmos.occupants.get_mut(occupant) else {
                continue;
            };
            if sorted::remove(&mut record.areas, &id) {
                self.events.emit(WorldEvent::AreaExited {
                    tick,
                    area: id,
                    occupant: *occupant,
                    loc: record.loc,
                    clock,
                    cause,
                });
            }
        }
        for occupant in &after {
            let Some(record) = self.cosmos.occupants.get_mut(occupant) else {
                continue;
            };
            if sorted::insert(&mut record.areas, id) {
                self.events.emit(WorldEvent::AreaEntered {
                    tick,
                    area: id,
                    occupant: *occupant,
                    loc: record.loc,
                    clock,
                    cause,
                });
            }
        }
    }

    /// Sets the rate of an area's clock relative to its realm's, giving
    /// it a clock if it has none. `None` takes its clock away: whatever
    /// was waiting on it goes back to the clock above.
    ///
    /// Occupants inside are not told. Whoever schedules for them asks
    /// [`Cosmos::ambient`] which clock applies.
    pub fn set_area_clock_rate(
        &mut self,
        id: AreaId,
        rate: Option<Real>,
    ) -> Result<(), WorldError> {
        let area = self.cosmos.area(id).ok_or(WorldError::UnknownArea(id))?;
        let realm_clock = self.cosmos.grid(area.grid)?.clock();
        let clock = match (area.clock, rate) {
            (Some(clock), Some(rate)) => {
                self.timeline.set_rate(clock, rate)?;
                Some(clock)
            }
            (None, Some(rate)) => Some(self.timeline.create_clock(realm_clock, rate)?),
            (Some(clock), None) => {
                self.timeline.destroy_clock(clock)?;
                None
            }
            (None, None) => None,
        };
        if let Some(area) = self.cosmos.areas.areas.get_mut(&id) {
            area.clock = clock;
        }
        // What is falling inside goes onto the clock that now applies.
        self.sync_process_clocks();
        Ok(())
    }

    /// Makes an area out of the closed space around a cell
    /// (see [`Cosmos::enclosed_cells`]): an area of the given kind over
    /// exactly the cells of the space.
    ///
    /// The area remembers how it was made. When an element inside it or
    /// on its edge changes in a way that matters to the fill, its shape
    /// is worked out again. If the space no longer closes, the area keeps
    /// its last shape, is marked breached and `AreaBreached` is emitted;
    /// when it closes again the mark is cleared and `AreaClosed` is
    /// emitted.
    ///
    /// Refused as [`Cosmos::enclosed_cells`] is, and if `limit` is more
    /// than [`AREA_MAX_CELLS`].
    pub fn enclose(
        &mut self,
        from: Location,
        kind: Option<AreaKindId>,
        limit: u32,
        cause: Cause,
    ) -> Result<AreaId, WorldError> {
        if limit > AREA_MAX_CELLS {
            return Err(WorldError::TooLarge {
                what: "the cells an enclosed space may hold",
                limit: u64::from(AREA_MAX_CELLS),
            });
        }
        let cells = self.cosmos.enclosed_cells(from, limit)?;
        let start = self.cosmos.normalize(from)?.cell;
        let mut spec = AreaSpec::new(from.grid, AreaShape::Cells(cells));
        spec.kind = kind;
        self.make_area(
            spec,
            Some(Enclosure {
                from: start,
                limit,
                breached: false,
                leak: None,
            }),
            cause,
        )
    }

    /// Works out again the enclosed areas that a change to a cell may
    /// have opened or closed. `opened` says the change only opened the
    /// cell: nothing that was open to the fill was closed by it.
    pub(crate) fn recheck_enclosures(&mut self, loc: Location, opened: bool, cause: Cause) {
        let Ok(view) = self.cosmos.grid(loc.grid) else {
            return;
        };
        let topology = view.topology();
        // The areas that hold the cell, or a cell beside, above or below
        // it: a wall is on the edge of the room it closes.
        let mut touched: Vec<AreaId> = Vec::new();
        let around = core::iter::once(loc.cell).chain(
            FILL_STEPS
                .iter()
                .filter_map(|step| topology.normalize(loc.cell.offset(*step)).ok()),
        );
        for cell in around {
            for area in self.cosmos.areas.at(topology, loc.grid, cell) {
                if area.enclosure.is_some() {
                    sorted::insert(&mut touched, area.id);
                }
            }
        }
        for id in touched {
            self.recheck_enclosure(id, opened, cause);
        }
    }

    fn recheck_enclosure(&mut self, id: AreaId, opened: bool, cause: Cause) {
        let Some(area) = self.cosmos.area(id) else {
            return;
        };
        let Some(enclosure) = area.enclosure else {
            return;
        };
        if opened && enclosure.breached {
            // Opening a space that is already open cannot close it.
            return;
        }
        let grid = area.grid;
        let Ok(view) = self.cosmos.grid(grid) else {
            return;
        };
        let defs = self.cosmos.defs();
        // Fill from where the space was first filled from, or, if that
        // cell has been walled up, from the first cell of the area that
        // is still open.
        let open = |cell: &CellPos| {
            view.column(cell.tract())
                .is_some_and(|column| admits(defs, column, *cell))
        };
        let start = if open(&enclosure.from) {
            Some(enclosure.from)
        } else {
            match &area.shape {
                AreaShape::Cells(cells) => cells.iter().copied().find(|cell| open(cell)),
                _ => None,
            }
        };
        let filled = match start {
            Some(start) => self
                .cosmos
                .enclosed_cells(Location::new(grid, start), enclosure.limit),
            None => Err(WorldError::NotEnclosed {
                leak: Location::new(grid, enclosure.from),
            }),
        };
        let tick = self.timeline.now();
        match filled {
            Ok(cells) => {
                if let Some(area) = self.cosmos.areas.areas.get_mut(&id)
                    && let Some(enclosure) = area.enclosure.as_mut()
                {
                    enclosure.breached = false;
                    enclosure.leak = None;
                }
                // The fill's own refusals cannot arise for cells it has
                // just walked.
                let _ = self.reshape_area(id, grid, AreaShape::Cells(cells), cause);
                if enclosure.breached {
                    self.events.emit(WorldEvent::AreaClosed {
                        tick,
                        area: id,
                        loc: Location::new(grid, enclosure.from),
                        cause,
                    });
                }
            }
            Err(WorldError::NotEnclosed { leak }) => {
                if let Some(area) = self.cosmos.areas.areas.get_mut(&id)
                    && let Some(enclosure) = area.enclosure.as_mut()
                {
                    enclosure.breached = true;
                    enclosure.leak = Some(leak.cell);
                }
                if !enclosure.breached {
                    self.events.emit(WorldEvent::AreaBreached {
                        tick,
                        area: id,
                        leak,
                        cause,
                    });
                }
            }
            // Part of the space is not loaded: there is no telling, so
            // the area is left as it was.
            Err(_) => {}
        }
    }
}
