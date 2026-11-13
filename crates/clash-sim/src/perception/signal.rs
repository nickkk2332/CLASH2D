//! Signals and how they travel (15 §3): the three models, delay, and
//! [`emit_signal`].
//!
//! | Model | Received |
//! |-------|----------|
//! | `line` | `strength × exposed × light / max(1, d)^falloff`, if a line of sight reaches the observer's eye |
//! | `through` | `strength / max(1, d)^falloff × Π (1 − block × coverage)` over the elements on the straight path |
//! | `field` | The level of the kind's field in the observer's cell, over 255 |
//!
//! `d` is the distance in metres between the two cells, a storey
//! counting three. `exposed` is the share of [`EXPOSURE_SAMPLES`] points
//! up the source's height that a line of sight from the eye reaches.
//! `light` is the light at the source's cell now (15 §3.3). `block` is
//! what the element's material holds back of the kind and `coverage`
//! the share of its cell's cross-section the element closes.

use std::collections::BTreeMap;
use std::sync::Arc;

use clash_core::{Dur, EntityId, Real, StateHash, StateHasher, impl_state_hash};
use clash_time::{Wake, WakeDomain, WakeId};
use clash_world::dims::STOREY_MM;
use clash_world::iter::segment_cells;
use clash_world::{
    Cause, CellPos, Cosmos, Layer, Location, OccupantFilter, PointM, SkyTime, distance_m,
    distinct_ids,
};
use serde::{Deserialize, Deserializer, Serialize, Serializer};

use super::defs::{KindInfo, PerceptionDefs, SenseId, SignalKindId, SignalModel};
use super::reason as why;
use super::{FLIGHT_ARG, percept};
use crate::reason::Reason;
use crate::{Place, Sim};

/// How many points up a source's height a line of sight is tried to,
/// for the share of it that is exposed (15 §3.1).
pub const EXPOSURE_SAMPLES: u16 = 4;

/// The height of an eye above the base of its cell, for an observer
/// whose body gives none: half a storey, as light is taken to travel at
/// (15 §3.3).
pub const EYE_MM: u16 = clash_world::light::LIGHT_EYE_MM;

/// The height of a source whose body gives none, in millimetres: what a
/// standing person is taken to be.
pub const SOURCE_HEIGHT_MM: u16 = 1700;

/// How far apart in metres the front of a delayed signal may be from an
/// observer and still be taken to have reached it: half a cell, so that
/// arithmetic on the front's place never makes it miss.
const FRONT_SLACK_M: Real = Real::of(0.5);

/// What a signal carries about the event that gave it off (15 §6).
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub struct SignalEvent {
    /// The event type's name.
    pub name: String,
    /// Who did it, from the event's cause: what an observer may take the
    /// actor for, if it perceives them.
    pub actor: Option<EntityId>,
    /// The salience a percept of it is remembered with; 0 is not
    /// remembered.
    pub salience: Real,
}

impl_state_hash!(SignalEvent {
    name,
    actor,
    salience
});

/// A signal (15 §3.2): of a kind, at a place, with a strength, about
/// something or nothing, made by a cause.
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub struct Signal {
    /// Its kind.
    pub kind: SignalKindId,
    /// The cell it comes from.
    pub at: Location,
    /// How high above the base of that cell, in millimetres.
    pub alt_mm: u16,
    /// How strong it is where it starts.
    pub strength: Real,
    /// What it is about: the entity that made it, if it was one.
    pub about: Option<EntityId>,
    /// The event it carries, if an event gave it off.
    pub event: Option<SignalEvent>,
    /// What made it. Truth: an observer is not told it.
    pub cause: Cause,
    /// What was said, if it is speech (15 §11): whoever perceives it
    /// hears the message.
    pub message: Option<Box<crate::social::Message>>,
}

impl clash_core::StateHash for Signal {
    fn state_hash(&self, hasher: &mut clash_core::StateHasher) {
        self.kind.state_hash(hasher);
        self.at.state_hash(hasher);
        self.alt_mm.state_hash(hasher);
        self.strength.state_hash(hasher);
        self.about.state_hash(hasher);
        self.event.state_hash(hasher);
        self.cause.state_hash(hasher);
        // Hashed only when there is one, so that a signal that is not
        // speech hashes as it did before speech.
        if let Some(message) = &self.message {
            message.state_hash(hasher);
        }
    }
}

impl Signal {
    /// A signal of a kind at a cell, on its floor, about nothing, carrying
    /// no event.
    pub fn new(kind: SignalKindId, at: Location, strength: Real, cause: Cause) -> Signal {
        Signal {
            kind,
            at,
            alt_mm: 0,
            strength,
            about: None,
            event: None,
            cause,
            message: None,
        }
    }

    /// This signal about an entity.
    #[must_use]
    pub fn about(mut self, entity: EntityId) -> Signal {
        self.about = Some(entity);
        self
    }

    /// This signal from a height in its cell.
    #[must_use]
    pub fn at_height(mut self, alt_mm: u16) -> Signal {
        self.alt_mm = alt_mm;
        self
    }

    /// This signal carrying an event.
    #[must_use]
    pub fn carrying(mut self, event: SignalEvent) -> Signal {
        self.event = Some(event);
        self
    }

    /// This signal carrying something said (15 §11).
    #[must_use]
    pub fn saying(mut self, message: crate::social::Message) -> Signal {
        self.message = Some(Box::new(message));
        self
    }
}

/// A delayed signal on its way to one observer.
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub(super) struct Flight {
    pub(super) id: u64,
    pub(super) signal: Signal,
    pub(super) observer: EntityId,
    pub(super) sense: SenseId,
    /// The local time of `clock` when it was given off.
    pub(super) emitted: Dur,
    /// The clock of where it was given off: the medium's.
    pub(super) clock: clash_time::ClockId,
    /// Its speed in metres a second of that clock.
    pub(super) speed: Real,
    pub(super) wake: Option<WakeId>,
}

impl_state_hash!(Flight {
    id,
    signal,
    observer,
    sense,
    emitted,
    clock,
    speed,
    wake
});

/// The delayed signals on their way, by id. Saved and hashed as the
/// list of them in order of id; kept by id so that one arriving is
/// taken out without moving the rest (D-733).
#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub(super) struct Flights(BTreeMap<u64, Flight>);

impl Flights {
    /// How many there are.
    pub(super) fn len(&self) -> usize {
        self.0.len()
    }

    /// Adds one, with an id above every one already here.
    fn push(&mut self, flight: Flight) {
        self.0.insert(flight.id, flight);
    }

    /// Takes one out by its id.
    fn take(&mut self, id: u64) -> Option<Flight> {
        self.0.remove(&id)
    }

    /// The one with the highest id.
    fn last_mut(&mut self) -> Option<&mut Flight> {
        self.0.values_mut().next_back()
    }
}

impl StateHash for Flights {
    fn state_hash(&self, hasher: &mut StateHasher) {
        hasher.write_len(self.0.len());
        for flight in self.0.values() {
            flight.state_hash(hasher);
        }
    }
}

impl Serialize for Flights {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_seq(self.0.values())
    }
}

impl<'de> Deserialize<'de> for Flights {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Flights, D::Error> {
        let list = Vec::<Flight>::deserialize(deserializer)?;
        Ok(Flights(
            list.into_iter().map(|flight| (flight.id, flight)).collect(),
        ))
    }
}

/// What emitting a signal came to.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct Emitted {
    /// Observers who received it at once.
    pub received: u32,
    /// Observers it is on its way to.
    pub delayed: u32,
}

/// Where an eye or a source is: a cell, a height above its base, and for a
/// source how tall it is.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Spot {
    /// The cell.
    pub loc: Location,
    /// Millimetres above its base.
    pub alt_mm: u16,
    /// For a source, how tall it is in millimetres; 0 for a point.
    pub height_mm: u16,
}

impl Spot {
    /// A point at a height in a cell.
    pub const fn point(loc: Location, alt_mm: u16) -> Spot {
        Spot {
            loc,
            alt_mm,
            height_mm: 0,
        }
    }
}

/// `max(1, d)^falloff`, the share a signal keeps over `d` metres being
/// its inverse.
pub fn spread(distance_m: Real, falloff: Real) -> Real {
    let d = distance_m.max(Real::ONE);
    let whole = falloff.round();
    if whole == falloff && (Real::ZERO..=Real::of(16.0)).contains(&falloff) {
        d.powi(whole.to_i32())
    } else {
        d.powf(falloff)
    }
}

/// The distance in metres between two cells, a storey counting three,
/// the shorter way round on wrapped axes. `None` across grids.
pub fn cell_distance(cosmos: &Cosmos, a: Location, b: Location) -> Option<Real> {
    cosmos.delta(a, b).map(distance_m)
}

/// The share of a source that a line of sight from an eye reaches: of
/// [`EXPOSURE_SAMPLES`] points spread up its height, how many the eye
/// sees (15 §3.1). A point source is seen or not.
pub fn exposure(cosmos: &Cosmos, eye: Spot, source: Spot) -> Real {
    let samples = exposure_heights(source);
    let seen = cosmos
        .sight_to_heights(eye.loc, eye.alt_mm, source.loc, &samples)
        .count_ones();
    Real::from(seen as u16) / Real::from(samples.len() as u16)
}

/// The heights above the base of its cell that a source is looked at,
/// for [`exposure`]: one for a point, [`EXPOSURE_SAMPLES`] spread up a
/// body.
fn exposure_heights(source: Spot) -> Vec<u16> {
    let samples = if source.height_mm == 0 {
        1
    } else {
        EXPOSURE_SAMPLES
    };
    (0..samples)
        .map(|sample| {
            let up = if source.height_mm == 0 {
                0
            } else {
                u32::from(source.height_mm) * (2 * u32::from(sample) + 1) / (2 * u32::from(samples))
            };
            (u32::from(source.alt_mm) + up).min(u32::from(STOREY_MM - 1)) as u16
        })
        .collect()
}

/// [`exposure`] as it was first written, one line of sight for each
/// height: what the faster one is tested against.
#[cfg(test)]
pub(super) fn exposure_reference(cosmos: &Cosmos, eye: Spot, source: Spot) -> Real {
    let samples = if source.height_mm == 0 {
        1
    } else {
        EXPOSURE_SAMPLES
    };
    let mut seen = 0u16;
    for sample in 0..samples {
        let up = if source.height_mm == 0 {
            0
        } else {
            u32::from(source.height_mm) * (2 * u32::from(sample) + 1) / (2 * u32::from(samples))
        };
        let alt = (u32::from(source.alt_mm) + up).min(u32::from(STOREY_MM - 1)) as u16;
        if cosmos
            .line_of_sight_between(eye.loc, eye.alt_mm, source.loc, alt)
            .unwrap_or(false)
        {
            seen += 1;
        }
    }
    Real::from(seen) / Real::from(samples)
}

/// Whether a field hides the straight line between two points: one of
/// the cells strictly between them holds a field at or above its
/// `opaque_from` (D-508).
pub fn fields_hide(cosmos: &Cosmos, a: PointM, b: PointM) -> bool {
    if a.grid != b.grid || cosmos.field_state().tracts().next().is_none() {
        return false;
    }
    let first = CellPos::containing(a.pos);
    let last = CellPos::containing(b.pos);
    segment_cells(a.pos, b.pos)
        .filter(|cell| *cell != first && *cell != last)
        .any(|cell| {
            cosmos
                .field_hides_sight(Location::new(a.grid, cell))
                .unwrap_or(false)
        })
}

/// The share of a signal of a kind that passes what stands on the
/// straight line between two points: `Π (1 − block × coverage)` over
/// each block and fixture in the cells strictly between them, and each
/// floor the line passes through between storeys, which closes the whole
/// of its cell. Space that is not loaded passes nothing.
pub fn passing(cosmos: &Cosmos, kind: &KindInfo, a: PointM, b: PointM) -> Real {
    let Ok(view) = cosmos.grid(a.grid) else {
        return Real::ZERO;
    };
    if a.grid != b.grid {
        return Real::ZERO;
    }
    // Always walked from the same end, so that the answer is the same
    // both ways.
    let key = |p: [Real; 3]| (p[2], p[1], p[0]);
    let (a, b) = if key(a.pos) <= key(b.pos) {
        (a.pos, b.pos)
    } else {
        (b.pos, a.pos)
    };
    let defs = cosmos.defs();
    let first = CellPos::containing(a);
    let last = CellPos::containing(b);
    let mut pass = Real::ONE;
    let mut prev: Option<(i32, clash_world::CellView)> = None;
    // The column of the cell before: the cells of a line are nearly
    // always in the same tract.
    let mut held: Option<(clash_world::TractPos, &clash_world::Column)> = None;
    for cell in segment_cells(a, b) {
        let Ok(norm) = view.normalize(cell) else {
            return Real::ZERO;
        };
        let column = match held {
            Some((tract, column)) if tract == norm.tract() => column,
            _ => {
                let Some(column) = view.column(norm.tract()) else {
                    return Real::ZERO;
                };
                held = Some((norm.tract(), column));
                column
            }
        };
        let states = column.cell(norm);
        if let Some((prev_z, prev_states)) = prev
            && cell.z != prev_z
        {
            let upper = if cell.z > prev_z { states } else { prev_states };
            if !upper.floor.is_none() {
                let floor = upper.floor.raw();
                pass *= Real::ONE - kind.block_of(floor.material.index());
            }
        }
        if cell != first && cell != last {
            for (layer, state) in [
                (Layer::Block, states.block.raw()),
                (Layer::Fixture, states.fixture.raw()),
            ] {
                if state.is_none() {
                    continue;
                }
                let coverage = defs
                    .info(layer, state)
                    .coverage
                    .clamp(Real::ZERO, Real::ONE);
                pass *= Real::ONE - kind.block_of(state.material.index()) * coverage;
            }
        }
        if pass <= Real::ZERO {
            return Real::ZERO;
        }
        prev = Some((cell.z, states));
    }
    pass
}

/// The light at a cell now, by the sky of its realm and the fixed
/// sources (15 §3.3). Zero where it cannot be read.
pub fn light_now(sim: &Sim, loc: Location) -> Real {
    SkyTime::now(&sim.cosmos, &sim.timeline, loc.grid)
        .and_then(|time| sim.cosmos.light(loc, time))
        .unwrap_or(Real::ZERO)
}

/// What reaches an eye of a signal of a kind and a strength from a
/// source, by the kind's model, before any sense or arc is considered.
/// Zero across grids, through space that is not loaded, and for the
/// `field` model, which is read and not sent.
pub fn strength_at(sim: &Sim, kind: &KindInfo, strength: Real, source: Spot, eye: Spot) -> Real {
    if strength <= Real::ZERO {
        return Real::ZERO;
    }
    strength_lit(sim, kind, strength, source, eye, || {
        source_light(sim, kind, source)
    })
}

/// The light at a source as [`strength_at`] and [`bound_at`] take it: the
/// light at its cell now for a kind of the `line` model that uses light,
/// and 1 for any other.
pub(super) fn source_light(sim: &Sim, kind: &KindInfo, source: Spot) -> Real {
    if kind.model == SignalModel::Line && kind.uses_light {
        light_now(sim, source.loc)
    } else {
        Real::ONE
    }
}

/// [`strength_at`] with the light at the source given, worked out only
/// if it is wanted: a sweep that has read it for [`bound_at`] does not
/// read it twice.
pub(super) fn strength_lit(
    sim: &Sim,
    kind: &KindInfo,
    strength: Real,
    source: Spot,
    eye: Spot,
    light: impl FnOnce() -> Real,
) -> Real {
    if strength <= Real::ZERO {
        return Real::ZERO;
    }
    let Some(distance) = cell_distance(&sim.cosmos, source.loc, eye.loc) else {
        return Real::ZERO;
    };
    let falloff = spread(distance, kind.falloff);
    match kind.model {
        SignalModel::Line => {
            let exposed = exposure(&sim.cosmos, eye, source);
            if exposed <= Real::ZERO {
                return Real::ZERO;
            }
            let middle = source.alt_mm.saturating_add(source.height_mm / 2);
            if fields_hide(
                &sim.cosmos,
                eye.loc.point_at(eye.alt_mm),
                source.loc.point_at(middle),
            ) {
                return Real::ZERO;
            }
            strength * exposed * light() / falloff
        }
        SignalModel::Through => {
            let pass = passing(
                &sim.cosmos,
                kind,
                source.loc.point_at(source.alt_mm),
                eye.loc.point_at(eye.alt_mm),
            );
            strength * pass / falloff
        }
        SignalModel::Field => Real::ZERO,
    }
}

/// The most of a signal that could reach an eye from a source with
/// nothing in the way: [`strength_at`] with the source fully exposed and
/// nothing holding it back. Never less than what does arrive.
pub fn bound_at(sim: &Sim, kind: &KindInfo, strength: Real, source: Spot, eye: Spot) -> Real {
    bound_lit(
        sim,
        kind,
        strength,
        source,
        eye,
        source_light(sim, kind, source),
    )
}

/// [`bound_at`] with the light at the source given, as
/// [`source_light`] gives it.
pub(super) fn bound_lit(
    sim: &Sim,
    kind: &KindInfo,
    strength: Real,
    source: Spot,
    eye: Spot,
    light: Real,
) -> Real {
    let Some(distance) = cell_distance(&sim.cosmos, source.loc, eye.loc) else {
        return Real::ZERO;
    };
    match kind.model {
        SignalModel::Field => Real::ZERO,
        SignalModel::Line | SignalModel::Through => {
            strength * light / spread(distance, kind.falloff)
        }
    }
}

/// Where an entity's eye is, if it is in the world: the cell of what it
/// is in, at its height there and up its body, or at [`EYE_MM`] for one
/// whose body gives no height.
pub fn eye_of(sim: &Sim, entity: EntityId) -> Option<Spot> {
    let top = sim.entities.outermost(entity)?;
    let Place::At { loc, alt_mm } = sim.entities.get(top)?.place() else {
        return None;
    };
    let height = height_mm(sim, entity);
    let eye = if height == 0 {
        EYE_MM
    } else {
        // Nine tenths of the way up.
        (u32::from(height) * 9 / 10) as u16
    };
    Some(Spot::point(
        loc,
        (u32::from(alt_mm) + u32::from(eye)).min(u32::from(STOREY_MM - 1)) as u16,
    ))
}

/// Where an entity is as a source of signals: its cell, its height there
/// and how tall it is.
pub fn source_of(sim: &Sim, entity: EntityId) -> Option<Spot> {
    let top = sim.entities.outermost(entity)?;
    let Place::At { loc, alt_mm } = sim.entities.get(top)?.place() else {
        return None;
    };
    let height = match height_mm(sim, entity) {
        0 => SOURCE_HEIGHT_MM,
        height => height,
    };
    Some(Spot {
        loc,
        alt_mm,
        height_mm: height,
    })
}

/// How tall an entity is, in millimetres, by its `Physical`; 0 if it has
/// none.
fn height_mm(sim: &Sim, entity: EntityId) -> u16 {
    sim.components.physical.get(entity).map_or(0, |physical| {
        let mm = (physical.height().get() * Real::of(1000.0)).round();
        mm.clamp(Real::ZERO, Real::from(STOREY_MM - 1)).to_i32() as u16
    })
}

/// The observers whose senses could receive a signal of a kind given off
/// at a cell: every entity within the kind's reach that has a sense of
/// it, those inside containers included, by id and then by sense.
pub(super) fn listeners(
    sim: &Sim,
    defs: &PerceptionDefs,
    at: Location,
    kind: SignalKindId,
) -> Vec<(EntityId, SenseId)> {
    let Some(info) = defs.kind(kind) else {
        return Vec::new();
    };
    let Ok(found) = sim
        .cosmos
        .occupants_within(at, info.reach, &OccupantFilter::ANY)
    else {
        return Vec::new();
    };
    let mut ids = distinct_ids(&found);
    let inside: Vec<EntityId> = ids
        .iter()
        .flat_map(|id| sim.entities.contents_deep(*id))
        .collect();
    ids.extend(inside);
    ids.sort_unstable();
    ids.dedup();
    let mut out = Vec::new();
    for id in ids {
        let Some(senses) = sim.components.senses.get(id) else {
            continue;
        };
        for slot in senses.slots() {
            if defs.sense_kind(slot.sense()) == Some(kind) {
                out.push((id, slot.sense()));
            }
        }
    }
    out
}

/// The speed a delayed signal of a kind travels at from a cell: its own,
/// or the speed of sound of the atmosphere there. `None` in vacuum.
fn speed_from(sim: &Sim, kind: &KindInfo, at: Location) -> Option<Real> {
    if let Some(speed) = kind.speed {
        return Some(speed);
    }
    let ambient = sim.cosmos.ambient(at).ok()?;
    let atmosphere = sim.cosmos.defs().atmospheres().get(ambient.atmosphere?)?;
    (atmosphere.sound_speed > Real::ZERO).then_some(atmosphere.sound_speed)
}

/// Gives off a signal (15 §3.2): each observer within reach with a sense
/// of its kind receives it, at once or, for a kind with `delay`, when its
/// front reaches them. What each receives is worked out from where it
/// stands when the signal reaches it.
///
/// Refused for a kind the definitions do not have, a kind of the `field`
/// model (a field is read, not sent), a strength that is negative or not
/// finite, and a cell outside its grid. A signal given off in space that
/// is not loaded, or a delayed one in vacuum, reaches nobody.
pub fn emit_signal(sim: &mut Sim, signal: Signal) -> Result<Emitted, Reason> {
    let defs = Arc::clone(&sim.perception_defs);
    let kind = defs.kind(signal.kind).ok_or_else(|| {
        Reason::new(why::UNKNOWN_KIND, "there is no such signal kind")
            .with("kind", signal.kind.index() as i64)
    })?;
    if kind.model == SignalModel::Field {
        return Err(Reason::new(
            why::FIELD_NOT_SENT,
            format!(
                "`{}` is read from a field and cannot be given off",
                defs.kind_name(signal.kind)
            ),
        )
        .with("kind", defs.kind_name(signal.kind)));
    }
    if !signal.strength.is_finite() || signal.strength < Real::ZERO {
        return Err(Reason::new(
            why::BAD_VALUE,
            "a signal's strength is a finite number of zero or more",
        )
        .with("value", signal.strength));
    }
    let at = sim
        .cosmos
        .normalize(signal.at)
        .map_err(|error| Reason::from_world(&error))?;
    let signal = Signal { at, ..signal };
    let mut emitted = Emitted::default();
    if !sim.cosmos.is_loaded(at) {
        return Ok(emitted);
    }
    let listeners = listeners(sim, &defs, at, signal.kind);
    if kind.delay {
        let Some(speed) = speed_from(sim, kind, at) else {
            return Ok(emitted);
        };
        let clock = sim.location_clock(at);
        for (observer, sense) in listeners {
            if signal.about == Some(observer) {
                continue;
            }
            let Some(eye) = eye_of(sim, observer) else {
                continue;
            };
            let Some(distance) = cell_distance(&sim.cosmos, at, eye.loc) else {
                continue;
            };
            launch(sim, &signal, observer, sense, clock, speed, distance);
            emitted.delayed += 1;
        }
    } else {
        for (observer, sense) in listeners {
            if signal.about == Some(observer) {
                continue;
            }
            if percept::receive(sim, &signal, observer, sense) {
                emitted.received += 1;
            }
        }
    }
    Ok(emitted)
}

/// Sends a delayed signal on its way to one observer `distance` metres
/// off.
fn launch(
    sim: &mut Sim,
    signal: &Signal,
    observer: EntityId,
    sense: SenseId,
    clock: clash_time::ClockId,
    speed: Real,
    distance: Real,
) {
    let id = sim.perception.next_flight;
    sim.perception.next_flight += 1;
    let after = Dur::from_secs_real(distance / speed);
    let wake = sim
        .schedule_on(
            clock,
            after,
            Wake::new(WakeDomain::Perception, id, FLIGHT_ARG),
        )
        .ok();
    if wake.is_none() {
        return;
    }
    sim.perception.flights.push(Flight {
        id,
        signal: signal.clone(),
        observer,
        sense,
        emitted: sim.timeline.local_now(clock).unwrap_or(Dur::ZERO),
        clock,
        speed,
        wake,
    });
}

/// A delayed signal's front has reached where its observer stood: if the
/// observer is there still, or nearer, it receives the signal; if it has
/// moved further off, the front goes on after it, unless it is now out
/// of the sense's range.
pub(super) fn arrive(sim: &mut Sim, id: u64) {
    let Some(flight) = sim.perception.flights.take(id) else {
        return;
    };
    let Some(eye) = eye_of(sim, flight.observer) else {
        return;
    };
    let Some(distance) = cell_distance(&sim.cosmos, flight.signal.at, eye.loc) else {
        return;
    };
    let elapsed = sim
        .timeline
        .local_now(flight.clock)
        .map_or(Real::ZERO, |now| (now - flight.emitted).as_secs_real());
    let front = flight.speed * elapsed;
    if distance <= front + FRONT_SLACK_M {
        percept::receive(sim, &flight.signal, flight.observer, flight.sense);
        return;
    }
    let range = sim
        .perception_defs
        .sense(flight.sense)
        .map_or(Real::ZERO, |def| def.range.get());
    if distance > range {
        return;
    }
    launch(
        sim,
        &flight.signal,
        flight.observer,
        flight.sense,
        flight.clock,
        flight.speed,
        distance - front,
    );
    // The front has come `front` metres already: what is left is from
    // there, so the new flight is timed from now with the same start.
    if let Some(last) = sim.perception.flights.last_mut() {
        last.emitted = flight.emitted;
    }
}

/// Every observer that a sweep of `sense` by `observer` considers, and
/// the share of a standing signature of each it receives: the entities
/// within the sense's range, those inside containers too where the kind
/// passes containers, by id.
pub(super) fn candidates(
    sim: &Sim,
    observer: EntityId,
    at: Location,
    range: Real,
) -> Vec<EntityId> {
    let Ok(mut ids) =
        sim.cosmos
            .occupant_ids_within(at, range, &OccupantFilter::ANY.except(observer))
    else {
        return Vec::new();
    };
    let inside: Vec<EntityId> = ids
        .iter()
        .flat_map(|id| sim.entities.contents_deep(*id))
        .collect();
    ids.extend(inside);
    ids.retain(|id| *id != observer);
    ids.sort_unstable();
    ids.dedup();
    ids
}

/// A wake-up of this package that is a delayed signal's arrival.
pub(super) fn is_flight(wake: &Wake) -> bool {
    wake.domain == WakeDomain::Perception && wake.arg == FLIGHT_ARG
}
