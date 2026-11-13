//! Occupancy: the index from cells to whatever stands in them (02 §8.1,
//! §8.2).
//!
//! The world does not own entities. It knows each occupant by its
//! [`EntityId`], with the few things collisions and queries need: its
//! kind, the planes it is on, its mass, its flags and the box of cells it
//! fills. The index changes only through [`WorldCtx::place`],
//! [`WorldCtx::relocate`] and [`WorldCtx::remove`] (01 R3), which the
//! simulation calls from its single relocation function.
//!
//! | Need | Use |
//! |------|-----|
//! | Put something into the world, move it, take it out | [`WorldCtx::place`], [`WorldCtx::relocate`], [`WorldCtx::relocate_as`], [`WorldCtx::remove`] |
//! | Change what the world knows about it | [`WorldCtx::update_occupant`] |
//! | Where something is | [`Cosmos::occupant`] |
//! | Who is in a cell | [`Cosmos::occupants_at`], [`Cosmos::is_occupied`] |
//! | Who is in a box, near a point, along a line | [`Cosmos::occupants_in_box`], [`Cosmos::occupants_within`], [`Cosmos::occupants_along`] |
//!
//! Queries list `(cell, occupant)` pairs: an occupant that fills several
//! cells is listed once for each of them that the query covers. Results
//! come by cell, storey by storey and row by row, then by id; along a
//! line they come in the order the line passes through the cells.
//! [`distinct_ids`] reduces a result to one entry for each occupant.
//!
//! Every occupant is in loaded space (02 invariant W4), so a query never
//! needs anything loaded: a cell that is not loaded, or lies outside the
//! realm, holds no one.

use core::cmp::Reverse;
use std::collections::BinaryHeap;

use clash_core::{EntityId, IVec3, Real, StateHash, StateHasher, sorted};
use serde::{Deserialize, Serialize};

use crate::area::{AreaId, AreaTable};
use crate::cause::Cause;
use crate::cosmos::{Cosmos, GridView};
use crate::defs::PlaneId;
use crate::dims::{CHUNK_Z_BITS, TRACT_XY};
use crate::edit::WorldCtx;
use crate::error::WorldError;
use crate::event::WorldEvent;
use crate::iter::segment_cells;
use crate::pos::{CellBox, CellIdx, CellPos, GridId, Location, PointM, distance_sq_cells};
use crate::topology::Topology;

/// The most cells an occupant's footprint may measure along one side.
/// Anything larger should be a subgrid (02 §8.1).
pub const FOOTPRINT_MAX_SIDE: u8 = 16;

/// The most tracts one occupant query may cover.
pub const QUERY_MAX_TRACTS: u64 = 1 << 12;

/// The most cells a query along a line may pass through.
pub const QUERY_MAX_LINE_CELLS: i64 = 1 << 16;

/// A chunk holding more `(cell, occupant)` pairs than this is looked at
/// by [`Cosmos::occupant_ids_within_nearest_first`] a band of rows of
/// one storey at a time (D-920).
pub const NEAREST_SPLIT_FROM: usize = 48;

/// How many rows of cells a band of a chunk holds, for
/// [`Cosmos::occupant_ids_within_nearest_first`].
pub const NEAREST_BAND_ROWS: i32 = 4;

/// A set of planes: one bit for each (02 §8.2). Two occupants interact
/// only if their masks share a bit. Terrain is on every plane.
///
/// ```
/// use clash_world::{PlaneId, PlaneMask};
///
/// let astral = PlaneId::new(1);
/// let ghost = PlaneMask::of(astral);
/// assert!(!ghost.shares(PlaneMask::DEFAULT));
/// assert!(ghost.with(PlaneId::new(0)).shares(PlaneMask::DEFAULT));
/// ```
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug, Serialize, Deserialize)]
#[serde(transparent)]
pub struct PlaneMask(u32);

impl PlaneMask {
    /// No plane: something with this mask interacts with nothing.
    pub const NONE: PlaneMask = PlaneMask(0);

    /// The built-in plane, bit 0, which everything is on unless it says
    /// otherwise.
    pub const DEFAULT: PlaneMask = PlaneMask(1);

    /// Every plane.
    pub const ALL: PlaneMask = PlaneMask(u32::MAX);

    /// How many planes a mask can tell apart.
    pub const BITS: u32 = u32::BITS;

    /// The mask of one plane. A plane beyond the last bit gives no plane.
    pub const fn of(plane: PlaneId) -> PlaneMask {
        if (plane.raw() as u32) < Self::BITS {
            PlaneMask(1 << plane.raw())
        } else {
            PlaneMask::NONE
        }
    }

    /// The mask with these bits.
    pub const fn from_bits(bits: u32) -> PlaneMask {
        PlaneMask(bits)
    }

    /// The bits: bit `n` is the plane with id `n`.
    pub const fn bits(self) -> u32 {
        self.0
    }

    /// This mask with a plane added.
    pub const fn with(self, plane: PlaneId) -> PlaneMask {
        PlaneMask(self.0 | PlaneMask::of(plane).0)
    }

    /// This mask with a plane removed.
    pub const fn without(self, plane: PlaneId) -> PlaneMask {
        PlaneMask(self.0 & !PlaneMask::of(plane).0)
    }

    /// The planes of either mask.
    pub const fn union(self, other: PlaneMask) -> PlaneMask {
        PlaneMask(self.0 | other.0)
    }

    /// Whether a plane is in the mask.
    pub const fn has(self, plane: PlaneId) -> bool {
        self.0 & PlaneMask::of(plane).0 != 0
    }

    /// Whether the two masks share a plane: whether things with them
    /// interact.
    #[inline]
    pub const fn shares(self, other: PlaneMask) -> bool {
        self.0 & other.0 != 0
    }

    /// Whether the mask holds no plane.
    pub const fn is_empty(self) -> bool {
        self.0 == 0
    }
}

impl Default for PlaneMask {
    fn default() -> PlaneMask {
        PlaneMask::DEFAULT
    }
}

impl StateHash for PlaneMask {
    fn state_hash(&self, hasher: &mut StateHasher) {
        hasher.write_u32(self.0);
    }
}

/// What kind of thing an occupant is. The world gives the kinds no
/// behaviour: they exist so that a query can ask for one.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OccupantKind {
    /// Something with a body that acts.
    Creature,
    /// A thing lying in the world.
    Item,
    /// The entity behind an interactive element (02 §5.5).
    Device,
    /// Anything else.
    Other,
}

impl OccupantKind {
    /// Every kind, in declaration order.
    pub const ALL: [OccupantKind; 4] = [
        OccupantKind::Creature,
        OccupantKind::Item,
        OccupantKind::Device,
        OccupantKind::Other,
    ];

    const fn bit(self) -> u8 {
        1 << self as u8
    }
}

impl StateHash for OccupantKind {
    fn state_hash(&self, hasher: &mut StateHasher) {
        hasher.write_u8(*self as u8);
    }
}

/// A set of occupant kinds, for filtering a query.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Serialize, Deserialize)]
#[serde(transparent)]
pub struct OccupantKinds(u8);

impl OccupantKinds {
    /// No kind.
    pub const NONE: OccupantKinds = OccupantKinds(0);

    /// Every kind.
    pub const ALL: OccupantKinds = OccupantKinds(
        OccupantKind::Creature.bit()
            | OccupantKind::Item.bit()
            | OccupantKind::Device.bit()
            | OccupantKind::Other.bit(),
    );

    /// The set of the given kinds.
    pub const fn of(kinds: &[OccupantKind]) -> OccupantKinds {
        let mut bits = 0;
        let mut at = 0;
        while at < kinds.len() {
            bits |= kinds[at].bit();
            at += 1;
        }
        OccupantKinds(bits)
    }

    /// Whether a kind is in the set.
    #[inline]
    pub const fn has(self, kind: OccupantKind) -> bool {
        self.0 & kind.bit() != 0
    }
}

impl Default for OccupantKinds {
    fn default() -> OccupantKinds {
        OccupantKinds::ALL
    }
}

/// What an occupant does to others in its cells.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(transparent)]
pub struct OccupantFlags(u8);

impl OccupantFlags {
    /// No flags.
    pub const NONE: OccupantFlags = OccupantFlags(0);

    /// Others cannot walk into its cells.
    pub const BLOCKS_MOVE: OccupantFlags = OccupantFlags(1 << 0);

    /// It cannot be seen past.
    pub const BLOCKS_SIGHT: OccupantFlags = OccupantFlags(1 << 1);

    /// Bodies in flight strike it.
    pub const COLLIDES: OccupantFlags = OccupantFlags(1 << 2);

    /// The flags with their names.
    pub const NAMED: [(OccupantFlags, &'static str); 3] = [
        (OccupantFlags::BLOCKS_MOVE, "blocks_move"),
        (OccupantFlags::BLOCKS_SIGHT, "blocks_sight"),
        (OccupantFlags::COLLIDES, "collides"),
    ];

    /// Whether every flag of `other` is set.
    #[inline]
    pub const fn contains(self, other: OccupantFlags) -> bool {
        self.0 & other.0 == other.0
    }

    /// The flags of either set.
    pub const fn union(self, other: OccupantFlags) -> OccupantFlags {
        OccupantFlags(self.0 | other.0)
    }

    /// Whether no flag is set.
    pub const fn is_empty(self) -> bool {
        self.0 == 0
    }

    /// The bits.
    pub const fn bits(self) -> u8 {
        self.0
    }
}

impl core::ops::BitOr for OccupantFlags {
    type Output = OccupantFlags;
    fn bitor(self, other: OccupantFlags) -> OccupantFlags {
        self.union(other)
    }
}

impl core::fmt::Debug for OccupantFlags {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        let names = OccupantFlags::NAMED
            .iter()
            .filter(|(flag, _)| self.contains(*flag))
            .map(|(_, name)| *name);
        f.debug_set().entries(names).finish()
    }
}

impl StateHash for OccupantFlags {
    fn state_hash(&self, hasher: &mut StateHasher) {
        hasher.write_u8(self.0);
    }
}

/// What the world knows about something that stands in it (02 §8.1).
///
/// ```
/// use clash_core::{EntityId, Real};
/// use clash_world::{Occupant, OccupantFlags, OccupantKind, PlaneMask};
///
/// let guard = Occupant::new(EntityId::new(7, 0), OccupantKind::Creature)
///     .with_mass(Real::of(80.0))
///     .with_flags(OccupantFlags::BLOCKS_MOVE | OccupantFlags::COLLIDES);
/// assert_eq!(guard.plane, PlaneMask::DEFAULT);
/// ```
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub struct Occupant {
    /// Whose entry this is. The simulation above owns what an entity is.
    pub id: EntityId,
    /// What kind of thing it is.
    pub kind: OccupantKind,
    /// The planes it is on.
    pub plane: PlaneMask,
    /// Its mass in kilograms, for collisions with bodies.
    pub mass: Real,
    /// What it does to others in its cells.
    pub flags: OccupantFlags,
}

impl Occupant {
    /// An occupant on the default plane, with no mass and no flags.
    pub const fn new(id: EntityId, kind: OccupantKind) -> Occupant {
        Occupant {
            id,
            kind,
            plane: PlaneMask::DEFAULT,
            mass: Real::ZERO,
            flags: OccupantFlags::NONE,
        }
    }

    /// This occupant on other planes.
    pub const fn on(mut self, plane: PlaneMask) -> Occupant {
        self.plane = plane;
        self
    }

    /// This occupant with a mass in kilograms.
    pub const fn with_mass(mut self, mass: Real) -> Occupant {
        self.mass = mass;
        self
    }

    /// This occupant with flags.
    pub const fn with_flags(mut self, flags: OccupantFlags) -> Occupant {
        self.flags = flags;
        self
    }
}

impl StateHash for Occupant {
    fn state_hash(&self, hasher: &mut StateHasher) {
        self.id.state_hash(hasher);
        self.kind.state_hash(hasher);
        self.plane.state_hash(hasher);
        self.mass.state_hash(hasher);
        self.flags.state_hash(hasher);
    }
}

/// The box of cells an occupant fills: so many cells east, south and up
/// from the cell it is at, which is the box's corner with the lowest
/// coordinates. Most occupants fill one cell.
///
/// ```
/// use clash_world::{CellPos, Footprint};
///
/// let cart = Footprint::new(2, 3, 1);
/// let cells = cart.cells(CellPos::new(10, 10, 0));
/// assert_eq!(cells.max, CellPos::new(11, 12, 0));
/// assert_eq!(cart.volume(), 6);
/// ```
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Serialize, Deserialize)]
pub struct Footprint {
    /// Cells along `x`. At least one.
    pub x: u8,
    /// Cells along `y`. At least one.
    pub y: u8,
    /// Storeys. At least one.
    pub z: u8,
}

impl Footprint {
    /// One cell.
    pub const SINGLE: Footprint = Footprint { x: 1, y: 1, z: 1 };

    /// A box of this many cells along each axis. A size of zero counts
    /// as one.
    pub const fn new(x: u8, y: u8, z: u8) -> Footprint {
        Footprint {
            x: if x == 0 { 1 } else { x },
            y: if y == 0 { 1 } else { y },
            z: if z == 0 { 1 } else { z },
        }
    }

    /// The cells filled by an occupant at `at`, before any wrap.
    pub const fn cells(self, at: CellPos) -> CellBox {
        let size = Footprint::new(self.x, self.y, self.z);
        CellBox {
            min: at,
            max: at.offset(IVec3::new(
                size.x as i32 - 1,
                size.y as i32 - 1,
                size.z as i32 - 1,
            )),
        }
    }

    /// How many cells the box holds.
    pub const fn volume(self) -> u32 {
        let size = Footprint::new(self.x, self.y, self.z);
        size.x as u32 * size.y as u32 * size.z as u32
    }

    fn check(self) -> Result<Footprint, WorldError> {
        let size = Footprint::new(self.x, self.y, self.z);
        if size.x > FOOTPRINT_MAX_SIDE || size.y > FOOTPRINT_MAX_SIDE || size.z > FOOTPRINT_MAX_SIDE
        {
            return Err(WorldError::TooLarge {
                what: "the side of an occupant's footprint, in cells",
                limit: u64::from(FOOTPRINT_MAX_SIDE),
            });
        }
        Ok(size)
    }
}

impl Default for Footprint {
    fn default() -> Footprint {
        Footprint::SINGLE
    }
}

impl StateHash for Footprint {
    fn state_hash(&self, hasher: &mut StateHasher) {
        hasher.write_u8(self.x);
        hasher.write_u8(self.y);
        hasher.write_u8(self.z);
    }
}

/// Where an occupant is, and what the world knows about it.
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub struct OccupantRecord {
    /// The occupant.
    pub occupant: Occupant,
    /// The cell it is at: the corner of its footprint with the lowest
    /// coordinates, normalised.
    pub loc: Location,
    /// The box of cells it fills, from `loc`.
    pub footprint: Footprint,
    /// The areas it is inside, in id order. It is inside an area when any
    /// cell of its footprint is.
    pub areas: Vec<AreaId>,
}

/// One occupant in one cell of a chunk.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct OccupantEntry {
    /// The cell, inside its chunk.
    pub cell: CellIdx,
    /// Who.
    pub id: EntityId,
    /// What kind of thing it is.
    pub kind: OccupantKind,
    /// The planes it is on.
    pub plane: PlaneMask,
    /// What it does to others in the cell.
    pub flags: OccupantFlags,
}

/// A chunk's index from cells to occupants (02 §8.1): the occupants of
/// each cell, kept in order of cell and then id, so that what it lists
/// never depends on the order things arrived in.
///
/// It is rebuilt when a tract loads and is neither saved nor hashed: the
/// simulation above owns the entities and places them again.
#[derive(Clone, PartialEq, Eq, Default, Debug)]
pub struct OccupantIndex {
    /// The pairs, in order. `None` when there are none, as for most
    /// chunks: an empty index is one pointer wide.
    #[allow(
        clippy::box_collection,
        reason = "a chunk holds one of these and most chunks hold no one: boxed, an empty one is 8 bytes and not 24"
    )]
    entries: Option<Box<Vec<OccupantEntry>>>,
}

impl OccupantIndex {
    #[inline]
    fn entries(&self) -> &[OccupantEntry] {
        self.entries.as_deref().map_or(&[], Vec::as_slice)
    }

    /// Whether no cell of the chunk holds anyone.
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.entries.is_none()
    }

    /// How many `(cell, occupant)` pairs the chunk holds.
    pub fn len(&self) -> usize {
        self.entries().len()
    }

    /// The occupants of a cell, in id order.
    #[inline]
    pub fn at(&self, cell: CellIdx) -> &[OccupantEntry] {
        let entries = self.entries();
        let start = entries.partition_point(|entry| entry.cell < cell);
        let count = entries[start..].partition_point(|entry| entry.cell == cell);
        &entries[start..start + count]
    }

    /// Every `(cell, occupant)` pair, by cell and then by id.
    pub fn iter(&self) -> impl Iterator<Item = &OccupantEntry> + '_ {
        self.entries().iter()
    }

    /// The bytes the index holds on the heap.
    pub fn heap_bytes(&self) -> usize {
        self.entries.as_ref().map_or(0, |entries| {
            core::mem::size_of::<Vec<OccupantEntry>>()
                + entries.capacity() * core::mem::size_of::<OccupantEntry>()
        })
    }

    /// Puts an occupant into a cell, replacing what the index held for it
    /// there.
    pub(crate) fn insert(&mut self, entry: OccupantEntry) {
        let entries = self.entries.get_or_insert_with(Box::default);
        match entries.binary_search_by(|have| (have.cell, have.id).cmp(&(entry.cell, entry.id))) {
            Ok(at) => entries[at] = entry,
            Err(at) => entries.insert(at, entry),
        }
    }

    /// Takes an occupant out of a cell. Returns whether it was there.
    pub(crate) fn remove(&mut self, cell: CellIdx, id: EntityId) -> bool {
        let Some(entries) = self.entries.as_mut() else {
            return false;
        };
        let Ok(at) = entries.binary_search_by(|have| (have.cell, have.id).cmp(&(cell, id))) else {
            return false;
        };
        entries.remove(at);
        if entries.is_empty() {
            self.entries = None;
        }
        true
    }
}

/// An occupant found in a cell by a query.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Occupied {
    /// The cell, normalised.
    pub cell: CellPos,
    /// Who is in it.
    pub id: EntityId,
    /// What kind of thing it is.
    pub kind: OccupantKind,
    /// The planes it is on.
    pub plane: PlaneMask,
    /// What it does to others in the cell.
    pub flags: OccupantFlags,
}

/// The occupants of a query's result, each once, in the order they first
/// appear.
pub fn distinct_ids(found: &[Occupied]) -> Vec<EntityId> {
    let mut seen: Vec<EntityId> = Vec::new();
    let mut ids = Vec::new();
    for occupied in found {
        if sorted::insert(&mut seen, occupied.id) {
            ids.push(occupied.id);
        }
    }
    ids
}

/// Which occupants a query wants.
///
/// ```
/// use clash_world::{OccupantFilter, OccupantFlags, OccupantKind, PlaneMask};
///
/// // Whatever stands in the way of someone on the default plane.
/// let in_the_way = OccupantFilter::ANY
///     .on(PlaneMask::DEFAULT)
///     .with_flags(OccupantFlags::BLOCKS_MOVE);
/// // Any creature, on whatever plane.
/// let creatures = OccupantFilter::ANY.of_kind(OccupantKind::Creature);
/// assert_ne!(in_the_way, creatures);
/// ```
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct OccupantFilter {
    /// Only occupants that share a plane with this mask. `None`: every
    /// occupant, whatever planes it is on.
    pub planes: Option<PlaneMask>,
    /// Only occupants of these kinds.
    pub kinds: OccupantKinds,
    /// Only occupants with every one of these flags.
    pub flags: OccupantFlags,
    /// Not this occupant: usually the one asking.
    pub except: Option<EntityId>,
}

impl OccupantFilter {
    /// Every occupant.
    pub const ANY: OccupantFilter = OccupantFilter {
        planes: None,
        kinds: OccupantKinds::ALL,
        flags: OccupantFlags::NONE,
        except: None,
    };

    /// Only occupants that share a plane with `planes`.
    pub const fn on(mut self, planes: PlaneMask) -> OccupantFilter {
        self.planes = Some(planes);
        self
    }

    /// Only occupants of one kind.
    pub const fn of_kind(mut self, kind: OccupantKind) -> OccupantFilter {
        self.kinds = OccupantKinds::of(&[kind]);
        self
    }

    /// Only occupants of these kinds.
    pub const fn of_kinds(mut self, kinds: OccupantKinds) -> OccupantFilter {
        self.kinds = kinds;
        self
    }

    /// Only occupants with every one of these flags.
    pub const fn with_flags(mut self, flags: OccupantFlags) -> OccupantFilter {
        self.flags = flags;
        self
    }

    /// Every occupant but this one.
    pub const fn except(mut self, id: EntityId) -> OccupantFilter {
        self.except = Some(id);
        self
    }

    /// Whether the filter lets an occupant through.
    #[inline]
    pub fn admits(
        &self,
        id: EntityId,
        kind: OccupantKind,
        plane: PlaneMask,
        flags: OccupantFlags,
    ) -> bool {
        self.planes.is_none_or(|planes| planes.shares(plane))
            && self.kinds.has(kind)
            && flags.contains(self.flags)
            && self.except != Some(id)
    }

    #[inline]
    fn admits_entry(&self, entry: &OccupantEntry) -> bool {
        self.admits(entry.id, entry.kind, entry.plane, entry.flags)
    }
}

impl Default for OccupantFilter {
    fn default() -> OccupantFilter {
        OccupantFilter::ANY
    }
}

fn found(cell: CellPos, entry: &OccupantEntry) -> Occupied {
    Occupied {
        cell,
        id: entry.id,
        kind: entry.kind,
        plane: entry.plane,
        flags: entry.flags,
    }
}

/// The cells of a footprint at a position, each normalised, or the
/// refusal for the first that lies outside the grid or is not loaded.
fn footprint_cells(
    view: &GridView<'_>,
    at: CellPos,
    footprint: Footprint,
) -> Result<Vec<CellPos>, WorldError> {
    let mut cells = Vec::with_capacity(footprint.volume() as usize);
    for cell in footprint.cells(at).cells() {
        let cell = view.normalize(cell)?;
        if view.column(cell.tract()).is_none() {
            return Err(WorldError::NotLoaded(Location::new(view.id(), cell)));
        }
        cells.push(cell);
    }
    Ok(cells)
}

/// The cells of a footprint that is already in the world. Cells that no
/// longer normalise are left out.
fn placed_cells(topology: &Topology, at: CellPos, footprint: Footprint) -> Vec<CellPos> {
    footprint
        .cells(at)
        .cells()
        .filter_map(|cell| topology.normalize(cell).ok())
        .collect()
}

/// The areas that hold any of these cells of a grid, in id order.
pub(crate) fn areas_holding(
    areas: &AreaTable,
    topology: &Topology,
    grid: GridId,
    cells: &[CellPos],
) -> Vec<AreaId> {
    let mut holding = Vec::new();
    if areas.is_empty() {
        return holding;
    }
    for cell in cells {
        for area in areas.at(topology, grid, *cell) {
            sorted::insert(&mut holding, area.id);
        }
    }
    holding
}

impl Cosmos {
    /// Where an occupant is and what the world knows about it, if it is
    /// in the world.
    pub fn occupant(&self, id: EntityId) -> Option<&OccupantRecord> {
        self.occupants.get(&id)
    }

    /// How many occupants are in the world.
    pub fn occupant_count(&self) -> usize {
        self.occupants.len()
    }

    /// Every occupant in the world, in id order.
    pub fn occupants(&self) -> Vec<&OccupantRecord> {
        self.occupants
            .sorted_entries()
            .into_iter()
            .map(|(_, record)| record)
            .collect()
    }

    /// The occupants of a cell that the filter lets through, in id order.
    /// A cell that is not loaded, or lies outside its grid, holds no one.
    pub fn occupants_at(
        &self,
        loc: Location,
        filter: &OccupantFilter,
    ) -> Result<Vec<Occupied>, WorldError> {
        let view = self.grid(loc.grid)?;
        let mut out = Vec::new();
        if let Ok(cell) = view.normalize(loc.cell)
            && let Some(column) = view.column(cell.tract())
            && let Some(index) = column.occupants_in(cell.z >> CHUNK_Z_BITS)
        {
            out.extend(
                index
                    .at(cell.idx())
                    .iter()
                    .filter(|entry| filter.admits_entry(entry))
                    .map(|entry| found(cell, entry)),
            );
        }
        Ok(out)
    }

    /// Whether a cell holds any occupant the filter lets through. This
    /// allocates nothing: it is the check for inner loops.
    pub fn is_occupied(&self, loc: Location, filter: &OccupantFilter) -> Result<bool, WorldError> {
        let view = self.grid(loc.grid)?;
        Ok(view
            .normalize(loc.cell)
            .ok()
            .and_then(|cell| {
                let index = view
                    .column(cell.tract())?
                    .occupants_in(cell.z >> CHUNK_Z_BITS)?;
                Some(
                    index
                        .at(cell.idx())
                        .iter()
                        .any(|entry| filter.admits_entry(entry)),
                )
            })
            .unwrap_or(false))
    }

    /// The occupants in a box of cells that the filter lets through, by
    /// cell and then by id. The box may reach across a wrapped edge.
    ///
    /// Refused if the box covers more than [`QUERY_MAX_TRACTS`] tracts.
    pub fn occupants_in_box(
        &self,
        grid: GridId,
        cells: CellBox,
        filter: &OccupantFilter,
    ) -> Result<Vec<Occupied>, WorldError> {
        let view = self.grid(grid)?;
        let mut out = Vec::new();
        collect_box(&view, cells, filter, &mut out)?;
        out.sort_by_key(|occupied| (occupied.cell, occupied.id));
        out.dedup_by_key(|occupied| (occupied.cell, occupied.id));
        Ok(out)
    }

    /// The occupants within a distance of a cell that the filter lets
    /// through, by cell and then by id. The distance is in metres between
    /// cell centres, a storey counting three, the shorter way round on
    /// wrapped axes (02 §3.3). An occupant is listed for each of its
    /// cells that is in reach.
    pub fn occupants_within(
        &self,
        centre: Location,
        radius_m: Real,
        filter: &OccupantFilter,
    ) -> Result<Vec<Occupied>, WorldError> {
        let view = self.grid(centre.grid)?;
        if radius_m < Real::ZERO {
            return Ok(Vec::new());
        }
        let mut out = within(&view, centre, radius_m, filter)?;
        // An occupant has one entry for each cell it fills, so no two
        // entries share a cell and an id: the order of an unstable sort
        // is the order of a stable one.
        out.sort_unstable_by_key(|occupied| (occupied.cell, occupied.id));
        out.dedup_by_key(|occupied| (occupied.cell, occupied.id));
        Ok(out)
    }

    /// Who is within a distance of a cell that the filter lets through,
    /// each once, by id: the ids of [`occupants_within`](Self::occupants_within)
    /// without its order by cell, for a caller that wants only who. A
    /// sense's sweep asks this many times a second for every creature
    /// that perceives (decision D-875).
    pub fn occupant_ids_within(
        &self,
        centre: Location,
        radius_m: Real,
        filter: &OccupantFilter,
    ) -> Result<Vec<EntityId>, WorldError> {
        let view = self.grid(centre.grid)?;
        if radius_m < Real::ZERO {
            return Ok(Vec::new());
        }
        let mut ids: Vec<EntityId> = within(&view, centre, radius_m, filter)?
            .into_iter()
            .map(|occupied| occupied.id)
            .collect();
        ids.sort_unstable();
        ids.dedup();
        Ok(ids)
    }

    /// Who [`occupants_within`](Cosmos::occupants_within) finds, in no
    /// order and once for each of their cells in reach, added to `out`:
    /// for a caller that keeps only a few of them and puts those in order
    /// itself (decision D-897).
    pub fn occupant_ids_within_unordered(
        &self,
        centre: Location,
        radius_m: Real,
        filter: &OccupantFilter,
        out: &mut Vec<EntityId>,
    ) -> Result<(), WorldError> {
        let view = self.grid(centre.grid)?;
        if radius_m < Real::ZERO {
            return Ok(());
        }
        out.extend(
            within(&view, centre, radius_m, filter)?
                .into_iter()
                .map(|occupied| occupied.id),
        );
        Ok(())
    }

    /// Who [`occupants_within`](Cosmos::occupants_within) finds, a block
    /// of cells at a time, the nearest blocks first (decision D-920):
    /// `visit` is given the least [`distance_sq_cells`] any cell of the
    /// block lies from the centre, the shorter way round, and for each of
    /// the block's occupants in reach, once for each of its cells in
    /// reach and in no order, that cell's distance and the occupant's id;
    /// it returns whether to go on. Blocks come in order of their least
    /// distance, so a caller keeping only the nearest few may stop at the
    /// first block farther than the farthest it keeps. A block is the
    /// part of a chunk the query covers, or for a chunk holding many, a
    /// band of [`NEAREST_BAND_ROWS`] rows of one storey of it. Every
    /// occupant `occupant_ids_within_unordered` finds is given to `visit`
    /// unless it stops first.
    pub fn occupant_ids_within_nearest_first(
        &self,
        centre: Location,
        radius_m: Real,
        filter: &OccupantFilter,
        mut visit: impl FnMut(i64, &[(i64, EntityId)]) -> bool,
    ) -> Result<(), WorldError> {
        let view = self.grid(centre.grid)?;
        if radius_m < Real::ZERO {
            return Ok(());
        }
        let reach = radius_m.ceil().to_i32();
        let storeys = (radius_m / crate::dims::STOREY_M).ceil().to_i32();
        let cells = CellBox::around(centre.cell, IVec3::new(reach, reach, storeys));
        let topology = view.topology();
        let z_lo = cells.min.z.max(topology.z_min);
        let z_hi = cells.max.z.min(topology.z_max);
        if z_lo > z_hi {
            return Ok(());
        }
        let (first, last) = (cells.min.tract(), cells.max.tract());
        let across = (i64::from(last.x) - i64::from(first.x) + 1) as u64;
        let down = (i64::from(last.y) - i64::from(first.y) + 1) as u64;
        if across.saturating_mul(down) > QUERY_MAX_TRACTS {
            return Err(WorldError::TooLarge {
                what: "the tracts an occupant query covers",
                limit: QUERY_MAX_TRACTS,
            });
        }
        // The centre brought round on wrapped axes, as the blocks' cells
        // are.
        let wrap = |extent: crate::topology::Extent, value: i32| match extent {
            crate::topology::Extent::Wrapped(size) if size > 0 => value.rem_euclid(size),
            _ => value,
        };
        let at = CellPos::new(
            wrap(topology.x, centre.cell.x),
            wrap(topology.y, centre.cell.y),
            centre.cell.z,
        );
        // The least distance from the centre to any cell of a box of
        // cells of the grid, in its own (normalised) coordinates, the
        // shorter way round: along each axis zero if the run holds the
        // centre, else from the nearer end (on a wrapped axis, a run that
        // does not hold a point is nearest it at one of its ends).
        let least_of = |x: (i32, i32), y: (i32, i32), z: (i32, i32)| -> i64 {
            let along = |lo: i32, hi: i32, at: i32, delta: &dyn Fn(i32) -> i32| -> i32 {
                if lo <= at && at <= hi {
                    0
                } else {
                    delta(lo).unsigned_abs().min(delta(hi).unsigned_abs()) as i32
                }
            };
            let dx = along(x.0, x.1, at.x, &|x| {
                topology.delta(at, CellPos::new(x, at.y, at.z)).x
            });
            let dy = along(y.0, y.1, at.y, &|y| {
                topology.delta(at, CellPos::new(at.x, y, at.z)).y
            });
            let dz = along(z.0, z.1, at.z, &|z| z.wrapping_sub(at.z));
            distance_sq_cells(IVec3::new(dx, dy, dz))
        };
        /// A block waiting to be looked at: cells of one chunk, its
        /// tract's normalised origin and the part of the tract the query
        /// covers, and the chunk's entries for those cells.
        #[derive(Clone, Copy)]
        struct Block<'a> {
            base: CellPos,
            chunk_z: i32,
            local: (i32, i32, i32, i32),
            entries: &'a [OccupantEntry],
            whole: bool,
        }
        let mut blocks: Vec<Block<'_>> = Vec::new();
        // Least distance first, then the order blocks were made in.
        let mut waiting: BinaryHeap<Reverse<(i64, usize)>> = BinaryHeap::new();
        for tract in cells.tracts() {
            let origin = tract.origin(z_lo);
            let Ok(base) = topology.normalize(origin) else {
                continue;
            };
            let Some(column) = view.column(base.tract()) else {
                continue;
            };
            if !column.has_occupants() {
                continue;
            }
            let x_lo = cells.min.x.max(origin.x) - origin.x;
            let x_hi = cells.max.x.min(origin.x.saturating_add(TRACT_XY - 1)) - origin.x;
            let y_lo = cells.min.y.max(origin.y) - origin.y;
            let y_hi = cells.max.y.min(origin.y.saturating_add(TRACT_XY - 1)) - origin.y;
            for (chunk_z, index) in
                column.occupant_indices(z_lo >> CHUNK_Z_BITS, z_hi >> CHUNK_Z_BITS)
            {
                let low = z_lo.max(chunk_z << CHUNK_Z_BITS);
                let high = z_hi.min((chunk_z << CHUNK_Z_BITS) | crate::dims::CHUNK_Z_MASK);
                let least = least_of(
                    (base.x.wrapping_add(x_lo), base.x.wrapping_add(x_hi)),
                    (base.y.wrapping_add(y_lo), base.y.wrapping_add(y_hi)),
                    (low, high),
                );
                waiting.push(Reverse((least, blocks.len())));
                blocks.push(Block {
                    base,
                    chunk_z,
                    local: (x_lo, x_hi, y_lo, y_hi),
                    entries: index.entries(),
                    whole: true,
                });
            }
        }
        let reach_sq = radius_m * radius_m;
        let mut found: Vec<(i64, EntityId)> = Vec::new();
        while let Some(Reverse((least, at_block))) = waiting.pop() {
            let Block {
                base,
                chunk_z,
                local: (x_lo, x_hi, y_lo, y_hi),
                entries,
                whole,
            } = blocks[at_block];
            if whole && entries.len() > NEAREST_SPLIT_FROM {
                // Many in one chunk: its bands of rows of each storey,
                // each a run of its entries (in order of cell: storey,
                // row, column), are looked at in their own turn.
                let low = (z_lo - (chunk_z << CHUNK_Z_BITS)).max(0);
                let high = (z_hi - (chunk_z << CHUNK_Z_BITS)).min(crate::dims::CHUNK_Z_MASK);
                for lz in low..=high {
                    let mut row = y_lo - y_lo % NEAREST_BAND_ROWS;
                    while row <= y_hi {
                        let (from, to) = (row.max(y_lo), (row + NEAREST_BAND_ROWS - 1).min(y_hi));
                        let first = CellIdx::new(0, from as u32, lz as u32).raw();
                        let after = u32::from(CellIdx::new(0, to as u32, lz as u32).raw())
                            + crate::dims::CHUNK_XY as u32;
                        let start = entries.partition_point(|entry| entry.cell.raw() < first);
                        let end = entries
                            .partition_point(|entry| u32::from(entry.cell.raw()) < after);
                        if start < end {
                            let z = (chunk_z << CHUNK_Z_BITS) | lz;
                            let least = least_of(
                                (base.x.wrapping_add(x_lo), base.x.wrapping_add(x_hi)),
                                (base.y.wrapping_add(from), base.y.wrapping_add(to)),
                                (z, z),
                            );
                            waiting.push(Reverse((least, blocks.len())));
                            blocks.push(Block {
                                base,
                                chunk_z,
                                local: (x_lo, x_hi, from, to),
                                entries: &entries[start..end],
                                whole: false,
                            });
                        }
                        row += NEAREST_BAND_ROWS;
                    }
                }
                continue;
            }
            found.clear();
            for entry in entries {
                let (x, y) = (entry.cell.lx(), entry.cell.ly());
                let z = (chunk_z << CHUNK_Z_BITS) | entry.cell.lz();
                if x >= x_lo
                    && x <= x_hi
                    && y >= y_lo
                    && y <= y_hi
                    && z >= z_lo
                    && z <= z_hi
                    && filter.admits_entry(entry)
                {
                    let cell = CellPos::new(base.x.wrapping_add(x), base.y.wrapping_add(y), z);
                    let distance = distance_sq_cells(topology.delta(at, cell));
                    if Real::from_i64(distance) <= reach_sq {
                        found.push((distance, entry.id));
                    }
                }
            }
            if !visit(least, &found) {
                break;
            }
        }
        Ok(())
    }

    /// The occupants in the cells a straight line passes through, from a
    /// point to another point of the same grid, in the order the line
    /// reaches the cells and by id within a cell. The cells are those of
    /// [`segment_cells`].
    ///
    /// Refused if the line passes through more than
    /// [`QUERY_MAX_LINE_CELLS`] cells.
    pub fn occupants_along(
        &self,
        from: PointM,
        to: [Real; 3],
        filter: &OccupantFilter,
    ) -> Result<Vec<Occupied>, WorldError> {
        let view = self.grid(from.grid)?;
        let too_long = WorldError::TooLarge {
            what: "the cells a line query passes through",
            limit: QUERY_MAX_LINE_CELLS as u64,
        };
        if from.pos.iter().chain(to.iter()).any(|at| !at.is_finite()) {
            return Err(too_long);
        }
        let steps = (CellPos::containing(to) - CellPos::containing(from.pos)).manhattan();
        if steps >= QUERY_MAX_LINE_CELLS {
            return Err(too_long);
        }
        let mut out = Vec::new();
        for cell in segment_cells(from.pos, to) {
            let Ok(cell) = view.normalize(cell) else {
                continue;
            };
            let Some(index) = view
                .column(cell.tract())
                .and_then(|column| column.occupants_in(cell.z >> CHUNK_Z_BITS))
            else {
                continue;
            };
            out.extend(
                index
                    .at(cell.idx())
                    .iter()
                    .filter(|entry| filter.admits_entry(entry))
                    .map(|entry| found(cell, entry)),
            );
        }
        Ok(out)
    }

    /// Takes every occupant with a cell in a tract out of the index,
    /// without events. For the functions that replace or drop a whole
    /// column, so that no record is left pointing at cells that are gone.
    pub(crate) fn purge_occupants(&mut self, grid: GridId, tract: crate::pos::TractPos) {
        let ids: Vec<EntityId> = match self
            .grid(grid)
            .ok()
            .and_then(|view| view.column(tract))
            .filter(|column| column.has_occupants())
        {
            Some(column) => {
                let mut ids = Vec::new();
                for (_, index) in column.occupant_indices(i32::MIN, i32::MAX) {
                    for entry in index.iter() {
                        sorted::insert(&mut ids, entry.id);
                    }
                }
                ids
            }
            None => return,
        };
        for id in ids {
            self.take_occupant(id);
        }
    }

    /// Takes an occupant's entries out of every cell it fills and returns
    /// its record. Emits nothing.
    fn take_occupant(&mut self, id: EntityId) -> Option<OccupantRecord> {
        let record = self.occupants.remove(&id)?;
        if let Ok(parts) = self.grid_mut_occupants(record.loc.grid) {
            for cell in placed_cells(&parts.topology, record.loc.cell, record.footprint) {
                if let Some(column) = parts.grid.column_mut(cell.tract()) {
                    column.occupant_remove(cell, id);
                }
            }
        }
        Some(record)
    }

    /// Writes an occupant's entry into each of these cells of a grid.
    fn index_occupant(&mut self, grid: GridId, cells: &[CellPos], occupant: &Occupant) {
        let Ok(parts) = self.grid_mut_occupants(grid) else {
            return;
        };
        for cell in cells {
            if let Some(column) = parts.grid.column_mut(cell.tract()) {
                column.occupant_insert(
                    *cell,
                    OccupantEntry {
                        cell: cell.idx(),
                        id: occupant.id,
                        kind: occupant.kind,
                        plane: occupant.plane,
                        flags: occupant.flags,
                    },
                );
            }
        }
    }
}

/// Adds to `out` the occupants in a box of cells of one grid that the
/// filter lets through, in no particular order and, where the box is
/// wider than a wrapped grid, possibly more than once.
///
/// This relies on a grid's horizontal extents being whole tracts, which
/// they are: a realm's are whole regions and a subgrid's are unbounded.
/// The occupants the filter lets through within a distance of a cell,
/// in no order: [`Cosmos::occupants_within`] before it sorts.
fn within(
    view: &GridView<'_>,
    centre: Location,
    radius_m: Real,
    filter: &OccupantFilter,
) -> Result<Vec<Occupied>, WorldError> {
    let reach = radius_m.ceil().to_i32();
    let storeys = (radius_m / crate::dims::STOREY_M).ceil().to_i32();
    let cells = CellBox::around(centre.cell, IVec3::new(reach, reach, storeys));
    let mut out = Vec::new();
    collect_box(view, cells, filter, &mut out)?;
    let topology = view.topology();
    let reach_sq = radius_m * radius_m;
    out.retain(|occupied| {
        let offset = topology.delta(centre.cell, occupied.cell);
        Real::from_i64(distance_sq_cells(offset)) <= reach_sq
    });
    Ok(out)
}

fn collect_box(
    view: &GridView<'_>,
    cells: CellBox,
    filter: &OccupantFilter,
    out: &mut Vec<Occupied>,
) -> Result<(), WorldError> {
    let topology = view.topology();
    let z_lo = cells.min.z.max(topology.z_min);
    let z_hi = cells.max.z.min(topology.z_max);
    if z_lo > z_hi {
        return Ok(());
    }
    let (first, last) = (cells.min.tract(), cells.max.tract());
    let across = (i64::from(last.x) - i64::from(first.x) + 1) as u64;
    let down = (i64::from(last.y) - i64::from(first.y) + 1) as u64;
    if across.saturating_mul(down) > QUERY_MAX_TRACTS {
        return Err(WorldError::TooLarge {
            what: "the tracts an occupant query covers",
            limit: QUERY_MAX_TRACTS,
        });
    }
    for tract in cells.tracts() {
        let origin = tract.origin(z_lo);
        let Ok(base) = topology.normalize(origin) else {
            continue;
        };
        let Some(column) = view.column(base.tract()) else {
            continue;
        };
        if !column.has_occupants() {
            continue;
        }
        // The part of the box inside this tract, as local coordinates.
        let x_lo = cells.min.x.max(origin.x) - origin.x;
        let x_hi = cells.max.x.min(origin.x.saturating_add(TRACT_XY - 1)) - origin.x;
        let y_lo = cells.min.y.max(origin.y) - origin.y;
        let y_hi = cells.max.y.min(origin.y.saturating_add(TRACT_XY - 1)) - origin.y;
        for (chunk_z, index) in column.occupant_indices(z_lo >> CHUNK_Z_BITS, z_hi >> CHUNK_Z_BITS)
        {
            for entry in index.iter() {
                let (x, y) = (entry.cell.lx(), entry.cell.ly());
                let z = (chunk_z << CHUNK_Z_BITS) | entry.cell.lz();
                if x >= x_lo
                    && x <= x_hi
                    && y >= y_lo
                    && y <= y_hi
                    && z >= z_lo
                    && z <= z_hi
                    && filter.admits_entry(entry)
                {
                    let cell = CellPos::new(base.x.wrapping_add(x), base.y.wrapping_add(y), z);
                    out.push(found(cell, entry));
                }
            }
        }
    }
    Ok(())
}

impl WorldCtx<'_> {
    /// Puts an occupant into the world at a cell, filling a box of cells
    /// from it. Every cell of the box must be inside the grid and loaded.
    ///
    /// Emits `AreaEntered` for each area it starts inside.
    ///
    /// Refused, changing nothing, if the occupant is already in the world
    /// ([`WorldError::AlreadyPlaced`]), the footprint is larger than
    /// [`FOOTPRINT_MAX_SIDE`] on a side, or a cell of it is outside the
    /// grid or not loaded.
    pub fn place(
        &mut self,
        occupant: Occupant,
        at: Location,
        footprint: Footprint,
        cause: Cause,
    ) -> Result<(), WorldError> {
        if self.cosmos.occupants.contains_key(&occupant.id) {
            return Err(WorldError::AlreadyPlaced(occupant.id));
        }
        let footprint = footprint.check()?;
        let view = self.cosmos.grid(at.grid)?;
        let cells = footprint_cells(&view, at.cell, footprint)?;
        let loc = Location::new(at.grid, view.normalize(at.cell)?);
        let areas = areas_holding(&self.cosmos.areas, view.topology(), at.grid, &cells);

        self.cosmos.index_occupant(at.grid, &cells, &occupant);
        let tick = self.timeline.now();
        for area in &areas {
            self.events.emit(WorldEvent::AreaEntered {
                tick,
                area: *area,
                occupant: occupant.id,
                loc,
                clock: self.cosmos.areas.get(*area).and_then(|area| area.clock),
                cause,
            });
        }
        self.cosmos.occupants.insert(
            occupant.id,
            OccupantRecord {
                occupant,
                loc,
                footprint,
                areas,
            },
        );
        // Whatever fields it now stands in have met it (02 §16).
        self.fields_met(occupant.id, at.grid, &cells, None, cause);
        Ok(())
    }

    /// Moves an occupant to another cell, of the same grid or another,
    /// keeping its footprint. This is the only way an occupant's position
    /// changes (01 R3). Returns where it was.
    ///
    /// Areas anchored to the occupant move with it. Emits `AreaExited`
    /// for each area it is no longer inside, then `AreaEntered` for each
    /// it now is, each in id order; and the same for whoever an anchored
    /// area moved over or away from. A move to where it already is does
    /// nothing at all.
    ///
    /// Refused, changing nothing, if the occupant is not in the world
    /// ([`WorldError::UnknownOccupant`]) or a cell of its footprint at
    /// the new position is outside the grid or not loaded.
    pub fn relocate(
        &mut self,
        id: EntityId,
        to: Location,
        cause: Cause,
    ) -> Result<Location, WorldError> {
        let footprint = self
            .cosmos
            .occupant(id)
            .ok_or(WorldError::UnknownOccupant(id))?
            .footprint;
        self.relocate_as(id, to, footprint, cause)
    }

    /// Moves an occupant to another cell and gives it another footprint:
    /// for something that turns, grows or lies down. Otherwise as
    /// [`relocate`](WorldCtx::relocate).
    pub fn relocate_as(
        &mut self,
        id: EntityId,
        to: Location,
        footprint: Footprint,
        cause: Cause,
    ) -> Result<Location, WorldError> {
        let record = self
            .cosmos
            .occupant(id)
            .ok_or(WorldError::UnknownOccupant(id))?;
        let from = record.loc;
        let occupant = record.occupant;
        let filled = record.footprint;
        let footprint = footprint.check()?;
        let view = self.cosmos.grid(to.grid)?;
        let cells = footprint_cells(&view, to.cell, footprint)?;
        let to = Location::new(to.grid, view.normalize(to.cell)?);
        if to == from && footprint == record.footprint {
            return Ok(from);
        }
        // How far it moves, for the areas that move with it.
        let by = if to.grid == from.grid {
            view.topology().delta(from.cell, to.cell)
        } else {
            to.cell - from.cell
        };

        let Some(mut record) = self.cosmos.take_occupant(id) else {
            return Err(WorldError::UnknownOccupant(id));
        };
        self.cosmos.index_occupant(to.grid, &cells, &occupant);
        record.loc = to;
        record.footprint = footprint;
        self.cosmos.occupants.insert(id, record);

        // Its own areas first, so that it is found inside them where it
        // now is and nothing is reported for them.
        let anchored: Vec<AreaId> = self.cosmos.areas.anchored_to(id).to_vec();
        for area in anchored {
            // An area that cannot follow stays where it is.
            let _ = self.carry_area(area, to.grid, by, cause);
        }

        let tick = self.timeline.now();
        let topology = *self.cosmos.grid(to.grid)?.topology();
        let now = areas_holding(&self.cosmos.areas, &topology, to.grid, &cells);
        let before = match self.cosmos.occupants.get_mut(&id) {
            Some(record) => core::mem::replace(&mut record.areas, now.clone()),
            None => Vec::new(),
        };
        for area in before.iter().filter(|area| !sorted::contains(&now, area)) {
            self.events.emit(WorldEvent::AreaExited {
                tick,
                area: *area,
                occupant: id,
                loc: to,
                clock: self.cosmos.areas.get(*area).and_then(|area| area.clock),
                cause,
            });
        }
        for area in now.iter().filter(|area| !sorted::contains(&before, area)) {
            self.events.emit(WorldEvent::AreaEntered {
                tick,
                area: *area,
                occupant: id,
                loc: to,
                clock: self.cosmos.areas.get(*area).and_then(|area| area.clock),
                cause,
            });
        }
        // Whatever fields it has moved into have met it (02 §16).
        self.fields_met(id, to.grid, &cells, Some((from, filled)), cause);
        Ok(from)
    }

    /// Takes an occupant out of the world and returns what the world
    /// knew about it. Emits `AreaExited` for each area it was inside.
    /// Areas anchored to it stay where they are.
    ///
    /// Refused if the occupant is not in the world
    /// ([`WorldError::UnknownOccupant`]).
    pub fn remove(&mut self, id: EntityId, cause: Cause) -> Result<OccupantRecord, WorldError> {
        let record = self
            .cosmos
            .take_occupant(id)
            .ok_or(WorldError::UnknownOccupant(id))?;
        let tick = self.timeline.now();
        for area in &record.areas {
            self.events.emit(WorldEvent::AreaExited {
                tick,
                area: *area,
                occupant: id,
                loc: record.loc,
                clock: self.cosmos.areas.get(*area).and_then(|area| area.clock),
                cause,
            });
        }
        Ok(record)
    }

    /// Replaces what the world knows about an occupant that is in it: its
    /// kind, planes, mass and flags. Its position does not change, and
    /// nothing is emitted.
    ///
    /// Refused if the occupant is not in the world
    /// ([`WorldError::UnknownOccupant`]).
    pub fn update_occupant(&mut self, occupant: Occupant) -> Result<(), WorldError> {
        let record = self
            .cosmos
            .occupants
            .get_mut(&occupant.id)
            .ok_or(WorldError::UnknownOccupant(occupant.id))?;
        if record.occupant == occupant {
            return Ok(());
        }
        record.occupant = occupant;
        let (loc, footprint) = (record.loc, record.footprint);
        let topology = *self.cosmos.grid(loc.grid)?.topology();
        let cells = placed_cells(&topology, loc.cell, footprint);
        self.cosmos.index_occupant(loc.grid, &cells, &occupant);
        Ok(())
    }
}
