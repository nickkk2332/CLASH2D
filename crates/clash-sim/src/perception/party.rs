//! What the player perceives (15 §13): how each event was perceived by
//! the directly controlled characters, their map memory, and the frames
//! the application feeds to the view model.
//!
//! The party is every entity whose control is direct and that has
//! senses. Each time one of its senses of a kind that uses light sweeps,
//! the cells it sees are recorded in its [`MapMemory`] as they are, and
//! a [`PartyFrame`] is queued: the cells that came into view or changed,
//! those that left the view of the whole party, the entities in view,
//! and what was heard since the last frame. The application drains the
//! frames ([`drain_frames`]) and feeds them to the scene's view model;
//! [`snapshot`] gives a member's whole map memory for a fresh view.

use std::collections::{BTreeMap, VecDeque};

use clash_core::{EntityId, Hash128, Real, StateHash, StateHasher, Tick, impl_state_hash};
use clash_world::{
    BlockState, CellBox, CellPos, DecalId, FieldId, FixtureState, FloorState, FovSet, GridId,
    Location, OccupantFilter, SkyTime, TractPos, WorldEvent, distance_m,
};
use serde::{Deserialize, Serialize};

use super::OMNISCIENT_SETTING;
use super::defs::{SenseId, SignalKindId};
use super::percept;
use super::signal::{self, Signal};
use crate::entity::Control;
use crate::event::{EventBody, SimEvent, Subject};
use crate::{Place, Sim};

/// How many tags of delivered events are kept for [`perceived`] to
/// answer from. Older events are answered `Not`.
pub const PARTY_TAGS_KEPT: usize = 8192;

/// How many frames wait for the application before the oldest are
/// dropped; the application then takes a [`snapshot`].
pub const PARTY_FRAMES_KEPT: usize = 256;

/// How many slabs of map memory a member keeps when
/// `perception.map_slabs_max` is not set: a slab is one storey of one
/// tract.
pub const DEFAULT_MAP_SLABS: i64 = 1024;

/// The setting that bounds a member's map memory, in slabs.
pub const MAP_SLABS_SETTING: &str = "perception.map_slabs_max";

/// How many storeys above its own a member's view takes in: what is
/// drawn of the world is the storey one stands on and what one looks
/// down on (12 §6.1).
pub const PARTY_VIEW_UP: i32 = 1;

/// How many storeys below its own a member's view takes in.
pub const PARTY_VIEW_DOWN: i32 = 3;

/// Cells in a slab: one storey of one tract.
const SLAB_CELLS: usize = clash_world::dims::CHUNK_AREA;

/// Light as remembered: a level from 0 to 1 in this many steps.
const LIGHT_STEPS: Real = Real::of(255.0);

/// How the party perceived an event (15 §13).
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PartyPerception {
    /// Seen: where it happened was in view, or it happened to a member.
    Seen,
    /// Heard, and not seen: a signal it gave off reached a member's sense.
    Heard {
        /// Where it seemed to come from.
        from: Location,
    },
    /// Neither.
    Not,
}

impl StateHash for PartyPerception {
    fn state_hash(&self, hasher: &mut StateHasher) {
        match self {
            PartyPerception::Seen => hasher.write_u8(0),
            PartyPerception::Heard { from } => {
                hasher.write_u8(1);
                from.state_hash(hasher);
            }
            PartyPerception::Not => hasher.write_u8(2),
        }
    }
}

/// One cell as a member saw it.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub struct Remembered {
    /// Its floor.
    pub floor: FloorState,
    /// Its block.
    pub block: BlockState,
    /// Its fixture.
    pub fixture: FixtureState,
    /// The strongest field in it and its level.
    pub field: Option<(FieldId, u8)>,
    /// The most marked decal and its amount.
    pub decal: Option<(DecalId, u8)>,
    /// Whether it was open to the sky.
    pub open_sky: bool,
    /// How well lit it was, 0 to 255 for 0 to 1.
    pub light: u8,
    /// When it was seen.
    pub at: Tick,
}

impl_state_hash!(Remembered {
    floor,
    block,
    fixture,
    field,
    decal,
    open_sky,
    light,
    at
});

impl Remembered {
    /// Whether what is there is the same, whenever it was seen.
    fn same_as(&self, other: &Remembered) -> bool {
        Remembered {
            at: other.at,
            ..*self
        } == *other
    }
}

/// One storey of one tract of a member's map memory.
#[derive(Clone, PartialEq, Debug, Serialize, Deserialize)]
struct Slab {
    /// One bit for each cell that has been seen.
    known: Vec<u64>,
    cells: Vec<Remembered>,
    /// When a cell of it was last seen.
    touched: Tick,
}

impl_state_hash!(Slab {
    known,
    cells,
    touched
});

const EMPTY: Remembered = Remembered {
    floor: FloorState::NONE,
    block: BlockState::NONE,
    fixture: FixtureState::NONE,
    field: None,
    decal: None,
    open_sky: false,
    light: 0,
    at: Tick::ZERO,
};

impl Slab {
    fn new() -> Slab {
        Slab {
            known: vec![0; SLAB_CELLS / 64],
            cells: vec![EMPTY; SLAB_CELLS],
            touched: Tick::ZERO,
        }
    }

    fn index(cell: CellPos) -> usize {
        let x = cell.x.rem_euclid(clash_world::dims::TRACT_XY) as usize;
        let y = cell.y.rem_euclid(clash_world::dims::TRACT_XY) as usize;
        y * clash_world::dims::TRACT_XY as usize + x
    }

    fn get(&self, cell: CellPos) -> Option<&Remembered> {
        let at = Slab::index(cell);
        (self.known[at / 64] & (1 << (at % 64)) != 0).then(|| &self.cells[at])
    }
}

type SlabKey = (GridId, TractPos, i32);

/// What a directly controlled character has seen of places, as it was
/// when it last saw them (15 §13): the `MapMemory` component.
#[derive(Clone, PartialEq, Debug, Default, Serialize, Deserialize)]
pub struct MapMemory {
    slabs: BTreeMap<SlabKey, Slab>,
}

impl StateHash for MapMemory {
    fn state_hash(&self, hasher: &mut StateHasher) {
        self.slabs.state_hash(hasher);
    }
}

impl MapMemory {
    /// What was in a cell when it was last seen, if it was ever seen.
    pub fn cell(&self, loc: Location) -> Option<&Remembered> {
        let key = (loc.grid, loc.cell.tract(), loc.cell.z);
        self.slabs.get(&key)?.get(loc.cell)
    }

    /// How many cells are remembered.
    pub fn len(&self) -> usize {
        self.slabs
            .values()
            .map(|slab| {
                slab.known
                    .iter()
                    .map(|word| word.count_ones() as usize)
                    .sum::<usize>()
            })
            .sum()
    }

    /// Whether nothing is remembered.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Every remembered cell, by slab and then row by row.
    pub fn cells(&self) -> impl Iterator<Item = (Location, &Remembered)> + '_ {
        self.slabs.iter().flat_map(|((grid, tract, z), slab)| {
            let origin = tract.origin(*z);
            (0..SLAB_CELLS)
                .filter(move |at| slab.known[at / 64] & (1 << (at % 64)) != 0)
                .map(move |at| {
                    let side = clash_world::dims::TRACT_XY as usize;
                    let cell = CellPos::new(
                        origin.x + (at % side) as i32,
                        origin.y + (at / side) as i32,
                        *z,
                    );
                    (Location::new(*grid, cell), &slab.cells[at])
                })
        })
    }

    /// Records a cell as seen. Returns whether what is there differs
    /// from what was remembered, or nothing was.
    fn record(&mut self, loc: Location, seen: Remembered, max_slabs: usize) -> bool {
        let key = (loc.grid, loc.cell.tract(), loc.cell.z);
        if !self.slabs.contains_key(&key) && self.slabs.len() >= max_slabs {
            let oldest = self
                .slabs
                .iter()
                .min_by_key(|(key, slab)| (slab.touched, **key))
                .map(|(key, _)| *key);
            if let Some(oldest) = oldest {
                self.slabs.remove(&oldest);
            }
        }
        let slab = self.slabs.entry(key).or_insert_with(Slab::new);
        let at = Slab::index(loc.cell);
        let known = slab.known[at / 64] & (1 << (at % 64)) != 0;
        let changed = !known || !slab.cells[at].same_as(&seen);
        slab.known[at / 64] |= 1 << (at % 64);
        slab.cells[at] = seen;
        slab.touched = slab.touched.max(seen.at);
        changed
    }
}

/// A cell in a frame: where it is and what was seen there.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub struct SeenCell {
    /// The cell.
    pub loc: Location,
    /// What was there.
    pub seen: Remembered,
    /// Whether it is in view now, or only remembered.
    pub in_view: bool,
}

/// Something in view, and where.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub struct SeenEntity {
    /// What it appeared to be: until personas, the entity.
    pub entity: EntityId,
    /// Its cell.
    pub loc: Location,
    /// How high it is in its cell, in millimetres.
    pub alt_mm: u16,
}

/// Something a member heard.
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub struct HeardSignal {
    /// Who heard it.
    pub by: EntityId,
    /// Where it seemed to come from.
    pub from: Location,
    /// Its kind.
    pub kind: SignalKindId,
    /// How far over the threshold it was received.
    pub clarity: Real,
    /// The event it carried, if any.
    pub event: Option<String>,
}

/// What the party perceived at one observation (15 §13): what the
/// application feeds to the view model.
#[derive(Clone, PartialEq, Debug, Default, Serialize, Deserialize)]
pub struct PartyFrame {
    /// When.
    pub tick: Tick,
    /// Cells that came into view or changed since last seen, with what
    /// is there now.
    pub seen: Vec<SeenCell>,
    /// Cells that left the view of the whole party: what was there stays
    /// remembered.
    pub unseen: Vec<Location>,
    /// Everything in view, with where it is.
    pub entities: Vec<SeenEntity>,
    /// What was heard since the last frame.
    pub heard: Vec<HeardSignal>,
}

/// What perception keeps about the party that belongs to no one entity.
#[derive(Clone, PartialEq, Debug, Default, Serialize, Deserialize)]
pub(super) struct PartyState {
    /// For each member, the cells in its view at its last observation,
    /// sorted.
    views: BTreeMap<EntityId, Vec<Location>>,
    /// Heard since the last frame.
    heard: Vec<HeardSignal>,
    /// Frames waiting for the application. Output: not hashed.
    frames: VecDeque<PartyFrame>,
    /// Whether frames were dropped since the application last drained.
    dropped: bool,
    /// How each delivered event was perceived, by the event's hash,
    /// newest last. Output: not hashed.
    tags: VecDeque<(Hash128, PartyPerception)>,
}

impl StateHash for PartyState {
    fn state_hash(&self, hasher: &mut StateHasher) {
        self.views.state_hash(hasher);
    }
}

impl PartyState {
    fn in_view(&self, loc: Location) -> bool {
        self.views
            .values()
            .any(|view| view.binary_search(&loc).is_ok())
    }
}

/// Whether an entity is of the party: directly controlled, with senses.
pub fn is_member(sim: &Sim, entity: EntityId) -> bool {
    sim.components.senses.contains(entity)
        && sim
            .entities
            .get(entity)
            .is_some_and(|record| record.control() == Control::Direct)
}

/// Whether the omniscient switch is on: every event is seen
/// (`perception.omniscient`, for debug profiles).
pub fn is_omniscient(sim: &Sim) -> bool {
    sim.settings()
        .get(OMNISCIENT_SETTING)
        .and_then(clash_core::Value::as_bool)
        .unwrap_or(false)
}

/// Turns the omniscient switch on or off (15 §13). A setting, and so
/// state: a debug profile sets it.
pub fn set_omniscient(sim: &mut Sim, on: bool) {
    sim.set_setting(OMNISCIENT_SETTING, on);
}

/// After a member's sense of a kind that uses light swept: what it sees
/// is recorded in its map memory, and a frame is queued.
pub(super) fn after_sweep(sim: &mut Sim, member: EntityId, sense: SenseId) {
    if !is_member(sim, member) {
        return;
    }
    let Some(view) = view_of(sim, member, sense) else {
        return;
    };
    let max_slabs = sim.setting_int(MAP_SLABS_SETTING, DEFAULT_MAP_SLABS).max(1) as usize;
    let memory = sim
        .components
        .map_memory
        .get_or_insert_with(member, MapMemory::default);
    let mut seen = Vec::new();
    let mut cells: Vec<Location> = Vec::with_capacity(view.len());
    for (loc, remembered) in view {
        if memory.record(loc, remembered, max_slabs) {
            seen.push(SeenCell {
                loc,
                seen: remembered,
                in_view: true,
            });
        }
        cells.push(loc);
    }
    cells.sort_unstable();
    cells.dedup();
    let before: Vec<Location> = union(&sim.perception.party.views);
    sim.perception.party.views.insert(member, cells.clone());
    let after = union(&sim.perception.party.views);
    let unseen: Vec<Location> = before
        .iter()
        .filter(|loc| after.binary_search(loc).is_err())
        .copied()
        .collect();
    // A cell new to the party's view is reported even if this member
    // remembered it as it is.
    for loc in &cells {
        if before.binary_search(loc).is_err()
            && !seen.iter().any(|cell| cell.loc == *loc)
            && let Some(remembered) = sim
                .components
                .map_memory
                .get(member)
                .and_then(|memory| memory.cell(*loc))
        {
            seen.push(SeenCell {
                loc: *loc,
                seen: *remembered,
                in_view: true,
            });
        }
    }
    seen.sort_by_key(|cell| cell.loc);
    let entities = entities_in(sim, &after);
    let heard = core::mem::take(&mut sim.perception.party.heard);
    let frame = PartyFrame {
        tick: sim.now(),
        seen,
        unseen,
        entities,
        heard,
    };
    let party = &mut sim.perception.party;
    party.frames.push_back(frame);
    while party.frames.len() > PARTY_FRAMES_KEPT {
        party.frames.pop_front();
        party.dropped = true;
    }
}

/// Every cell in any member's view, sorted, each once.
fn union(views: &BTreeMap<EntityId, Vec<Location>>) -> Vec<Location> {
    let mut all: Vec<Location> = views.values().flatten().copied().collect();
    all.sort_unstable();
    all.dedup();
    all
}

/// The cells a member's sense sees now, with what is in each: those in
/// its field of view within the sense's range and arc where a creature
/// standing in the cell would be seen by the light there.
fn view_of(sim: &Sim, member: EntityId, sense: SenseId) -> Option<Vec<(Location, Remembered)>> {
    let defs = &sim.perception_defs;
    let def = defs.sense(sense)?;
    let kind_id = defs.sense_kind(sense)?;
    let kind = defs.kind(kind_id)?;
    let eye = signal::eye_of(sim, member)?;
    let threshold = sim
        .components
        .senses
        .get(member)?
        .slot(sense)?
        .threshold()?;
    let facing = sim.components.senses.get(member)?.facing();
    let mut fov = FovSet::new();
    sim.cosmos
        .field_of_view(eye.loc, eye.alt_mm, def.range.get(), &mut fov)
        .ok()?;
    let (low, high) = fov.storeys()?;
    let (low, high) = (low.max(-PARTY_VIEW_DOWN), high.min(PARTY_VIEW_UP));
    if low > high {
        return Some(Vec::new());
    }
    let reach = def
        .range
        .get()
        .ceil()
        .to_i32()
        .min(clash_world::query::FOV_MAX_REACH);
    let origin = eye.loc.cell;
    let bounds = CellBox::new(
        CellPos::new(origin.x - reach, origin.y - reach, origin.z + low),
        CellPos::new(origin.x + reach, origin.y + reach, origin.z + high),
    );
    let time = SkyTime::now(&sim.cosmos, &sim.timeline, eye.loc.grid).ok()?;
    let mut light = Vec::new();
    sim.cosmos
        .fill_light(eye.loc.grid, bounds, time, &[], &mut light)
        .ok()?;
    let side = (2 * reach + 1) as usize;
    let strength = if kind.standing > Real::ZERO {
        kind.standing
    } else {
        Real::ONE
    };
    let topology = *sim.cosmos.topology(eye.loc.grid).ok()?;
    let any_fields = sim.cosmos.field_state().tracts().next().is_some();
    let mut out = Vec::new();
    // The storeys a member keeps alone: the air of the open sky above
    // is not walked through.
    for offset in fov.offsets_between(low, high) {
        if offset.z < low || offset.z > high {
            continue;
        }
        let raw = origin.offset(offset);
        let index = ((raw.z - bounds.min.z) as usize * side + (raw.y - bounds.min.y) as usize)
            * side
            + (raw.x - bounds.min.x) as usize;
        let level = light.get(index).copied().unwrap_or(Real::ZERO);
        let lit = if kind.uses_light { level } else { Real::ONE };
        let distance = distance_m(offset);
        if strength * lit / signal::spread(distance, kind.falloff) < threshold {
            continue;
        }
        let direction = if offset.x == 0 && offset.y == 0 {
            None
        } else {
            let radians = Real::from(offset.x).atan2(-Real::from(offset.y));
            Some(super::senses::normalize_deg(
                radians * Real::of(180.0) / Real::of(core::f64::consts::PI),
            ))
        };
        if !super::senses::within_arc(facing, def.arc, direction) {
            continue;
        }
        let Ok(cell) = topology.normalize(raw) else {
            continue;
        };
        let loc = Location::new(eye.loc.grid, cell);
        let Ok(states) = sim.cosmos.cell(loc) else {
            continue;
        };
        let field = if any_fields {
            sim.cosmos.fields_at(loc).ok().and_then(|fields| {
                fields
                    .into_iter()
                    .max_by_key(|(id, level)| (*level, core::cmp::Reverse(*id)))
            })
        } else {
            None
        };
        let decal = sim
            .cosmos
            .decals(loc)
            .ok()
            .and_then(|decals| decals.iter().max_by_key(|decal| decal.amount))
            .map(|decal| (decal.kind, decal.amount));
        out.push((
            loc,
            Remembered {
                floor: states.floor,
                block: states.block,
                fixture: states.fixture,
                field,
                decal,
                open_sky: sim
                    .cosmos
                    .static_light(loc)
                    .is_ok_and(|light| light.sky_steps == 0),
                light: (level.clamp(Real::ZERO, Real::ONE) * LIGHT_STEPS)
                    .round()
                    .to_i32() as u8,
                at: sim.now(),
            },
        ));
    }
    Some(out)
}

/// The entities standing in cells of a view, by id: all but those given
/// no signature of the kinds that see.
fn entities_in(sim: &Sim, view: &[Location]) -> Vec<SeenEntity> {
    let mut out: Vec<SeenEntity> = Vec::new();
    let mut boxes: BTreeMap<GridId, CellBox> = BTreeMap::new();
    for loc in view {
        let bounds = boxes
            .entry(loc.grid)
            .or_insert(CellBox::new(loc.cell, loc.cell));
        bounds.min = CellPos::new(
            bounds.min.x.min(loc.cell.x),
            bounds.min.y.min(loc.cell.y),
            bounds.min.z.min(loc.cell.z),
        );
        bounds.max = CellPos::new(
            bounds.max.x.max(loc.cell.x),
            bounds.max.y.max(loc.cell.y),
            bounds.max.z.max(loc.cell.z),
        );
    }
    for (grid, bounds) in boxes {
        let Ok(found) = sim
            .cosmos
            .occupants_in_box(grid, bounds, &OccupantFilter::ANY)
        else {
            continue;
        };
        for occupied in found {
            let loc = Location::new(grid, occupied.cell);
            if view.binary_search(&loc).is_err()
                || out.iter().any(|seen| seen.entity == occupied.id)
            {
                continue;
            }
            let invisible = sim
                .components
                .signature
                .get(occupied.id)
                .is_some_and(|signature| {
                    !signature.entries().is_empty()
                        && signature.entries().iter().all(|(kind, strength)| {
                            *strength <= Real::ZERO
                                && sim
                                    .perception_defs
                                    .kind(*kind)
                                    .is_some_and(|info| info.uses_light)
                        })
                });
            if invisible {
                continue;
            }
            let Some(Place::At { loc, alt_mm }) = sim.entities.get(occupied.id).map(|e| e.place())
            else {
                continue;
            };
            out.push(SeenEntity {
                entity: occupied.id,
                loc,
                alt_mm,
            });
        }
    }
    out.sort_by_key(|seen| seen.entity);
    out
}

/// A member heard a signal: it goes into the next frame.
pub(super) fn heard(sim: &mut Sim, observer: EntityId, signal: &Signal, clarity: Real) {
    if !is_member(sim, observer) {
        return;
    }
    sim.perception.party.heard.push(HeardSignal {
        by: observer,
        from: signal.at,
        kind: signal.kind,
        clarity,
        event: signal.event.as_ref().map(|event| event.name.clone()),
    });
}

/// Where an event happened, if it says: its cell (`here`), the cell of
/// the entity it is about (`self`), or another cell it names.
fn event_where(sim: &Sim, event: &SimEvent) -> Option<Location> {
    if let EventBody::World(world) = &event.body {
        return Some(world_where(world));
    }
    let subjects = event.subjects();
    if let Some(loc) = subjects
        .iter()
        .find(|(name, _)| *name == "here")
        .and_then(|(_, subject)| subject.loc())
    {
        return Some(loc);
    }
    for (_, subject) in &subjects {
        match subject {
            Subject::Entity(id) => {
                if let Some(loc) = sim.entities.location(*id) {
                    return Some(loc);
                }
            }
            other => {
                if let Some(loc) = other.loc() {
                    return Some(loc);
                }
            }
        }
    }
    None
}

fn world_where(event: &WorldEvent) -> Location {
    event.location()
}

/// Tags a delivered event with how the party perceived it, given the
/// signals it gave off.
pub(super) fn tag(sim: &mut Sim, event: &SimEvent, signals: &[Signal]) {
    if sim.perception.party.views.is_empty() && !is_omniscient(sim) {
        return;
    }
    let tag = tag_of(sim, event, signals);
    let party = &mut sim.perception.party;
    party.tags.push_back((Hash128::of(event), tag));
    while party.tags.len() > PARTY_TAGS_KEPT {
        party.tags.pop_front();
    }
}

fn tag_of(sim: &Sim, event: &SimEvent, signals: &[Signal]) -> PartyPerception {
    if is_omniscient(sim) {
        return PartyPerception::Seen;
    }
    let party = &sim.perception.party;
    let members: Vec<EntityId> = party
        .views
        .keys()
        .copied()
        .filter(|member| is_member(sim, *member))
        .collect();
    let concerns_member = event
        .subjects()
        .iter()
        .any(|(_, subject)| subject.entity().is_some_and(|id| members.contains(&id)));
    if concerns_member {
        return PartyPerception::Seen;
    }
    if let Some(loc) = event_where(sim, event)
        && party.in_view(loc)
    {
        return PartyPerception::Seen;
    }
    for signal in signals {
        for member in &members {
            let Some(senses) = sim.components.senses.get(*member) else {
                continue;
            };
            let heard = senses.slots().iter().any(|slot| {
                percept::reading_of(sim, signal, *member, slot.sense())
                    .is_some_and(|clarity| clarity >= Real::ONE)
            });
            if heard {
                return PartyPerception::Heard { from: signal.at };
            }
        }
    }
    PartyPerception::Not
}

/// How the party perceived an event that was delivered (15 §13): what
/// playback asks to show only what the party perceived. An event the
/// party could not have perceived, one delivered before any member
/// looked, and one older than the last [`PARTY_TAGS_KEPT`], is `Not`;
/// with the omniscient switch on, every event is `Seen`.
pub fn perceived(sim: &Sim, event: &SimEvent) -> PartyPerception {
    if is_omniscient(sim) {
        return PartyPerception::Seen;
    }
    let key = Hash128::of(event);
    sim.perception
        .party
        .tags
        .iter()
        .rev()
        .find(|(have, _)| *have == key)
        .map_or(PartyPerception::Not, |(_, tag)| *tag)
}

/// Takes the frames queued since the last call, oldest first, and
/// whether some were dropped before they could be taken: if so, take a
/// [`snapshot`] as well.
pub fn drain_frames(sim: &mut Sim) -> (Vec<PartyFrame>, bool) {
    let party = &mut sim.perception.party;
    let dropped = core::mem::take(&mut party.dropped);
    (party.frames.drain(..).collect(), dropped)
}

/// Everything a member remembers, as one frame: each remembered cell,
/// marked as in view or not, and the entities in view now. For filling a
/// fresh view model.
pub fn snapshot(sim: &Sim, member: EntityId) -> PartyFrame {
    let party = &sim.perception.party;
    let seen: Vec<SeenCell> = sim
        .components
        .map_memory
        .get(member)
        .map(|memory| {
            memory
                .cells()
                .map(|(loc, remembered)| SeenCell {
                    loc,
                    seen: *remembered,
                    in_view: party.in_view(loc),
                })
                .collect()
        })
        .unwrap_or_default();
    PartyFrame {
        tick: sim.now(),
        seen,
        unseen: Vec::new(),
        entities: entities_in(sim, &union(&party.views)),
        heard: Vec::new(),
    }
}

/// A member's map memory, if it has one.
pub fn map_memory(sim: &Sim, member: EntityId) -> Option<&MapMemory> {
    sim.components.map_memory.get(member)
}

/// The cells in view of the whole party at the members' last
/// observations, sorted.
pub fn party_view(sim: &Sim) -> Vec<Location> {
    union(&sim.perception.party.views)
}

/// A member is no longer of the party: its view is forgotten.
pub(super) fn left(sim: &mut Sim, entity: EntityId) {
    sim.perception.party.views.remove(&entity);
}
