//! Agents, residents, routines and calendars
//! (`docs/09-ai-and-simulation.md` §10, §14.2; `docs/06-entities-and-bodies.md`
//! §12): the part of the layered simulation a town needs to look alive.
//!
//! | Part | Is | Module |
//! |------|----|--------|
//! | [`CalendarDef`], [`calendar::time_of_day`] | The `calendar` type; a realm's day, its parts and seasons (L1) | [`calendar`] |
//! | [`RoutineDef`] | The `routine` type: the parts of a character's day (L2) | [`routine`] |
//! | [`Resident`], [`ResidentKey`], [`AgentState`], [`AgentLayer`] | Residents of settlements, agents, their wake-ups (E11, L1) | [`state`] |
//! | Properties, verbs, events, reasons | What rules and minds read and call | [`props`] |
//!
//! # Residents (06 §12.2; package E11)
//!
//! When a tract is first loaded, generation hands the simulation a spawn
//! request for each person its settlements, sites and maps ask for
//! (10 §11.2). One that names a template becomes a **resident**: a record
//! keyed on the realm, the cell it was asked for at (its **home**) and
//! which of those asked for there it is ([`ResidentKey`]), with a seed
//! drawn from the named stream `agent.resident` keyed on the same
//! (decision D-780). Its **individual** is made from the template with
//! that seed, so it has the same name and looks every time it is made
//! (invariant B7), anonymous until something makes it notable.
//!
//! | When | An anonymous individual | A notable one |
//! |------|-------------------------|---------------|
//! | Its tract unloads | Becomes nothing: the record stays, cold, with an [`AgentState`] | Is stored, out of the world, with an [`AgentState`]: an agent (`agent_stored`) |
//! | Its routine puts it in a tract that loads, or a wake-up finds it in a loaded one | Is made again from its seed, where its routine puts it | Comes back, where its routine puts it (`agent_loaded`) |
//! | It dies, or is removed | Its record is `gone` and is never made again | The same |
//!
//! What makes an anonymous individual notable (06 §12.2): a wound, a
//! variable set on it, a trait gained or lost, an item taken or dropped,
//! a relation it holds changing, an action shared with a controlled or
//! notable entity, and `make_notable`. `make_anonymous` makes it
//! anonymous again; an agent made anonymous becomes nothing at once.
//!
//! # Agents and wake-ups (09 §10.1; package L1)
//!
//! A resident that is not loaded keeps where it is, what it is doing and
//! when it next needs attention: the next part of its routine, or its
//! arrival somewhere. Every cold resident has exactly one entry in one
//! schedule, and the timeline has one wake-up for the earliest of them,
//! in the `Stepper` domain: an agent costs nothing between its wake-ups
//! (invariant N7). At a wake-up it is placed again by its routine; if
//! that place is loaded it comes into view.
//!
//! # Routines (09 §10.2 to §10.4; package L2)
//!
//! A mind names its routine (`routine` on `mind`), and `set_routine`
//! gives one to anyone. At each part of the day the routine says where
//! the character is meant to be, by role (home, a site by definition or
//! tag), what it is doing, and which goal it pursues there. That goal
//! joins the goals its mind may have and is scored with them, so a
//! disturbance a goal scores higher (a fight, a fire, a call for help)
//! overrides the routine, and the routine is taken up again after. Its
//! goals read where they are meant to be through the properties
//! `routine_place`, `routine_places`, `routine_doing` and
//! `routine_away`, which are the character's own.
//!
//! Placed while unloaded, a character is at its place, or, between the
//! start of a part and the time it takes to walk there at its routine's
//! `speed` over a way a little longer than the straight line, on the
//! straight line between, at the free cell nearest where it has got to.
//!
//! # Not built
//!
//! Populations as counts with presence tables (09 §13.1) beyond the
//! residents themselves; long goals and cold methods (09 §10.3: `aim` is
//! kept and not acted on); a loaded character walking out of view toward
//! a place that is not loaded (it stays, and its routine's goal is not
//! wanted until the place loads); folding agents into populations and
//! the regional cap (09 §10.5); `day_part_changed`; a calendar's own
//! length of day (the sky's is used, decision D-782).

pub mod calendar;
pub mod props;
pub mod routine;
pub mod state;

use clash_core::{Dur, EntityId, HashDomain, Real, Tick, pos_hash};
use clash_data::DefTypes;
use clash_time::{ClockId, Wake, WakeDomain, WakeId};
use clash_world::{
    Cause, CauseKind, CellFlags, CellPos, Location, OccupantFilter, RealmId, SpawnRequest, TractPos,
};

use crate::Sim;
use crate::action::ActEvent;
use crate::ai::GoalId;
use crate::bus::EventPayload;
use crate::entity::{Control, Place};
use crate::event::{EventBody, SimEvent, Subject, Where};
use crate::reason::ReasonInfo;
use crate::template::CharacterSpec;

pub use calendar::{CalendarDef, CalendarId, Day};
pub use props::{AGENT_EVENTS, properties, verbs};
pub use routine::{Pick, PlaceSel, PlaceSpec, RoutineDef, RoutineEntry, RoutineId, Whom};
pub use state::{Activity, AgentLayer, AgentPlace, AgentState, Resident, ResidentKey};

/// The domain a resident's seed is drawn from (invariant B7).
pub const RESIDENT_DOMAIN: HashDomain = HashDomain::new("agent.resident");

/// The domain the seed of a character that is not a resident is drawn
/// from, for its picks and spots.
pub const CHARACTER_DOMAIN: HashDomain = HashDomain::new("agent.character");

/// What marks a wake-up of the `Stepper` domain as the agents' own.
pub const WAKE_AGENTS: u64 = 0x4147_454E;

/// How far round where its routine puts it an individual is made or
/// brought back, at most, in cells: the nearest free cell it could stand
/// in.
pub const SPOT_RADIUS: i32 = 6;

/// The word an agent with no routine is doing.
pub const WAITING: &str = "waiting";

/// The stable codes of the reasons of this module.
pub mod reason {
    /// `make_anonymous` was given something that is no individual made
    /// from templates. Values: `entity`.
    pub const NOT_AN_INDIVIDUAL: &str = "agent.not_an_individual";
    /// `set_routine` named no routine that is loaded. Values: `routine`.
    pub const UNKNOWN_ROUTINE: &str = "agent.unknown_routine";
}

/// Every reason of this module, for the generated reference.
pub const REASONS: &[ReasonInfo] = &[
    ReasonInfo {
        code: reason::NOT_AN_INDIVIDUAL,
        doc: "`make_anonymous` was given something that is not an individual made from templates (it has no identity to make anonymous). Values: `entity`.",
    },
    ReasonInfo {
        code: reason::UNKNOWN_ROUTINE,
        doc: "`set_routine` named no `routine` that is loaded. Values: `routine`.",
    },
];

/// Registers the definition types of this module. Called from the
/// crate's `register_defs`.
pub fn register_defs(types: &mut DefTypes) {
    types.register::<CalendarDef>().register::<RoutineDef>();
}

/// What the agents' layer does is a process of the simulation's own.
const AGENT_CAUSE: Cause = Cause::new(CauseKind::Coarse);

/// The social layer's event of a relation a mind holds changing
/// (15 §9.1).
const RELATION_CHANGED: &str = "relation_changed";

impl Sim {
    /// The residents, agents and their schedule (09 §10).
    pub fn agents(&self) -> &AgentLayer {
        &self.agents
    }
}

// ---- reading a routine ---------------------------------------------------

/// Whose routine it is, for an entity: a resident's home and seed, or for
/// anyone else the home its mind keeps (else where it is) and a seed of
/// its own.
pub fn whom_of(sim: &Sim, entity: EntityId) -> Option<Whom> {
    if let Some((key, resident)) = sim.agents.resident_of(entity) {
        let grid = sim.cosmos.realm(key.realm)?.base_grid;
        return Some(Whom {
            realm: key.realm,
            home: Location::new(grid, key.home),
            seed: resident.seed,
        });
    }
    let here = sim.entities.location(entity)?;
    let realm = sim.cosmos.realm_of(here.grid)?;
    let home = sim
        .components
        .mind_state
        .get(entity)
        .and_then(crate::ai::MindState::home)
        .unwrap_or(here);
    let bits = entity.to_bits();
    let seed = pos_hash(
        sim.rng.seed(),
        CHARACTER_DOMAIN,
        bits as i32,
        (bits >> 32) as i32,
        0,
        0,
    );
    Some(Whom { realm, home, seed })
}

/// A resident's home, for the individual that stands for it.
pub fn resident_home(sim: &Sim, entity: EntityId) -> Option<Location> {
    let (key, _) = sim.agents.resident_of(entity)?;
    let grid = sim.cosmos.realm(key.realm)?.base_grid;
    Some(Location::new(grid, key.home))
}

/// The routine a mind profile names, by the profile's name.
fn routine_of_profile<'s>(sim: &'s Sim, profile: &str) -> Option<&'s str> {
    let id = sim.ai_defs.mind_named(profile)?;
    crate::ai::think::mind_def(sim, id)?.routine.as_deref()
}

/// The routine an entity keeps: the one `set_routine` gave it, else the
/// one the mind it thinks by names.
pub fn routine_of(sim: &Sim, entity: EntityId) -> Option<(RoutineId, &RoutineDef)> {
    let written = match sim.agents.routines.get(&entity) {
        Some(name) => name.as_str(),
        None => {
            let id = crate::ai::mind_def_of(sim, entity)?;
            crate::ai::think::mind_def(sim, id)?.routine.as_deref()?
        }
    };
    routine::routine_named(sim, written)
}

/// The routine a resident keeps: its individual's, or for one with none,
/// the one the mind its template names keeps.
fn resident_routine<'s>(sim: &'s Sim, resident: &Resident) -> Option<(RoutineId, &'s RoutineDef)> {
    if let Some(entity) = resident.entity {
        return routine_of(sim, entity);
    }
    let template = crate::template::template_by_name(&sim.defs, &resident.what)?;
    let def = sim.life.templates().get(template)?;
    match def.mind.as_ref()? {
        crate::template::MindField::Profile(profile) => {
            routine::routine_named(sim, routine_of_profile(sim, profile)?)
        }
        crate::template::MindField::Has(_) => None,
    }
}

/// Where a character's routine has it now: which part of the day, the
/// entry, and the reckoned time that part began.
pub fn current_entry<'s>(
    sim: &'s Sim,
    def: &'s RoutineDef,
    whom: &Whom,
) -> Option<(usize, &'s RoutineEntry, Dur)> {
    let day = calendar::realm_day(sim, whom.realm);
    let local = calendar::local_now(sim, whom.realm)?;
    let (at, began) = routine::entry_at(def, day, calendar::reckoned(day, local), whom.seed)?;
    Some((at, def.day.get(at)?, began))
}

/// The goal an entity's routine has it pursue now, if it names one.
pub fn routine_goal(sim: &Sim, entity: EntityId) -> Option<GoalId> {
    let (_, def) = routine_of(sim, entity)?;
    let whom = whom_of(sim, entity)?;
    let (_, entry, _) = current_entry(sim, def, &whom)?;
    entry.goal.as_ref().map(|goal| goal.id())
}

/// Every place an entity's routine has it be now, each its own spot, in
/// order: what `routine_places` reads.
pub fn routine_places(sim: &Sim, entity: EntityId) -> Vec<Location> {
    let Some((_, def)) = routine_of(sim, entity) else {
        return Vec::new();
    };
    let Some(whom) = whom_of(sim, entity) else {
        return Vec::new();
    };
    let Some((_, entry, _)) = current_entry(sim, def, &whom) else {
        return Vec::new();
    };
    let mut places = routine::entry_places(sim, &whom, entry);
    if places.is_empty() {
        places.push(whom.home);
    }
    places
}

/// Where an entity's routine has it be now: of the places its part names,
/// the one it keeps to now (on a `beat`, the one it has come to), else
/// its home. `None` if it keeps no routine.
pub fn routine_place(sim: &Sim, entity: EntityId) -> Option<Location> {
    let (_, def) = routine_of(sim, entity)?;
    let whom = whom_of(sim, entity)?;
    let (_, entry, began) = current_entry(sim, def, &whom)?;
    let day = calendar::realm_day(sim, whom.realm);
    let now = calendar::reckoned(day, calendar::local_now(sim, whom.realm)?);
    let places = routine::entry_places(sim, &whom, entry);
    Some(routine::beat_place(&places, entry, day, now - began).unwrap_or(whom.home))
}

/// The tick a reckoned time of a realm's clock is at, at the rate the
/// clock runs now: in the past or the future.
fn tick_of_reckoned(sim: &Sim, realm: RealmId, day: Day, reckoned: Dur) -> Option<Tick> {
    let clock = sim.cosmos.realm(realm)?.clock;
    let local_now = sim.timeline.local_now(clock).ok()?;
    let rate = sim.timeline.effective_rate(clock)?;
    if rate <= Real::ZERO {
        return None;
    }
    let local = reckoned - day.starts_at;
    let apart = local - local_now;
    let universal = if rate == Real::ONE {
        apart
    } else {
        Dur::from_secs_real(apart.as_secs_real() / rate)
    };
    Some(sim.now() + universal)
}

/// What a cold resident's routine makes of it now (09 §10.4): where it
/// is, what it is doing and when it next needs attention. One with no
/// routine waits where it last was, or at home.
fn plan_now(sim: &Sim, key: ResidentKey, resident: &Resident) -> Option<AgentState> {
    let grid = sim.cosmos.realm(key.realm)?.base_grid;
    let whom = Whom {
        realm: key.realm,
        home: Location::new(grid, key.home),
        seed: resident.seed,
    };
    let now = sim.now();
    let Some((_, def)) = resident_routine(sim, resident) else {
        let loc = resident
            .agent
            .as_ref()
            .map_or(whom.home, |agent| match agent.at {
                AgentPlace::At { loc } => loc,
                AgentPlace::Travelling { to, .. } => to,
            });
        return Some(AgentState {
            at: AgentPlace::At { loc },
            doing: Activity {
                what: WAITING.to_owned(),
                since: now,
                until: None,
            },
            aim: resident.agent.as_ref().and_then(|agent| agent.aim.clone()),
            wake: None,
        });
    };
    let day = calendar::realm_day(sim, key.realm);
    let local = calendar::local_now(sim, key.realm)?;
    let (at, began) = routine::entry_at(def, day, calendar::reckoned(day, local), resident.seed)?;
    let entry = def.day.get(at)?;
    // The part before, the one it set out from.
    let previous = routine::entry_at(def, day, began - Dur::from_micros(1), resident.seed)
        .and_then(|(before, _)| def.day.get(before))
        .unwrap_or(entry);
    let now_reckoned = calendar::reckoned(day, local);
    let to = routine::entry_place(sim, &whom, entry, day, now_reckoned - began);
    let from = routine::entry_place(sim, &whom, previous, day, Dur::ZERO);
    let next = routine::next_start(def, day, began, resident.seed).unwrap_or(began + day.length);
    let since = tick_of_reckoned(sim, key.realm, day, began).unwrap_or(now);
    let next_tick = tick_of_reckoned(sim, key.realm, day, next).filter(|tick| *tick > now);
    let arrives = since + routine::travel_time(sim, def, from, to);
    let place = if arrives > now && from != to {
        AgentPlace::Travelling {
            from,
            to,
            since,
            until: arrives,
        }
    } else {
        AgentPlace::At { loc: to }
    };
    let wake = match place {
        AgentPlace::Travelling { until, .. } => {
            Some(next_tick.map_or(until, |next| next.min(until)))
        }
        AgentPlace::At { .. } => next_tick,
    };
    Some(AgentState {
        at: place,
        doing: Activity {
            what: entry.doing.clone(),
            since,
            until: next_tick,
        },
        aim: resident.agent.as_ref().and_then(|agent| agent.aim.clone()),
        wake,
    })
}

/// Where an agent is now: its place, or the point it has got to on its
/// way.
pub fn position_now(sim: &Sim, state: &AgentState) -> Location {
    match state.at {
        AgentPlace::At { loc } => loc,
        AgentPlace::Travelling {
            from,
            to,
            since,
            until,
        } => {
            let total = until.since(since).as_micros().max(1);
            let gone = sim.now().since(since).as_micros().clamp(0, total);
            let share = Real::from_i64(gone) / Real::from_i64(total);
            let Some(by) = sim.cosmos.delta(from, to) else {
                return to;
            };
            let along = |part: i32| {
                (Real::from_i64(i64::from(part)) * share)
                    .round()
                    .to_i64()
                    .clamp(i64::from(i32::MIN), i64::from(i32::MAX)) as i32
            };
            Location::new(
                from.grid,
                CellPos::new(
                    from.cell.x + along(by.x),
                    from.cell.y + along(by.y),
                    from.cell.z + along(by.z),
                ),
            )
        }
    }
}

// ---- making and storing individuals ----------------------------------------

/// The nearest cell to `near`, on its storey and within [`SPOT_RADIUS`]
/// cells, that is loaded, can be stood in, holds nothing in the way and
/// has nothing in it that blocks the way: `near` itself if it is such a cell. Ties go to the
/// first in order of row, then column.
pub fn free_spot(sim: &Sim, near: Location) -> Option<Location> {
    let fits = |loc: Location| {
        let loc = sim.cosmos.normalize(loc).ok()?;
        if !sim.cosmos.is_loaded(loc) || !sim.cosmos.standable(loc).ok()? {
            return None;
        }
        if sim
            .cosmos
            .flags(loc)
            .ok()?
            .contains(CellFlags::WALK_BLOCKED)
        {
            return None;
        }
        let empty = sim
            .cosmos
            .occupants_at(
                loc,
                &OccupantFilter::ANY.with_flags(clash_world::OccupantFlags::BLOCKS_MOVE),
            )
            .is_ok_and(|found| found.is_empty());
        empty.then_some(loc)
    };
    if let Some(found) = fits(near) {
        return Some(found);
    }
    for ring in 1..=SPOT_RADIUS {
        for dy in -ring..=ring {
            for dx in -ring..=ring {
                if dx.abs() != ring && dy.abs() != ring {
                    continue;
                }
                let at = Location::new(
                    near.grid,
                    CellPos::new(near.cell.x + dx, near.cell.y + dy, near.cell.z),
                );
                if let Some(found) = fits(at) {
                    return Some(found);
                }
            }
        }
    }
    None
}

/// The seed of a resident: the named stream `agent.resident`, keyed on
/// its realm, its home and its index (invariant B7).
pub fn resident_seed(sim: &Sim, key: ResidentKey) -> u64 {
    pos_hash(
        sim.rng.seed(),
        RESIDENT_DOMAIN,
        key.home.x,
        key.home.y,
        key.home.z,
        (u64::from(key.realm.raw()) << 16) | u64::from(key.index),
    )
}

/// Answers a loaded tract's request for a person (06 §12.2; 10 §11.2):
/// a request that names a template becomes a resident, keyed on where
/// it was asked for, and its individual is made where its routine puts
/// it now if that is loaded. Returns whether the request was taken.
pub(crate) fn take_spawn_request(sim: &mut Sim, loc: Location, request: &SpawnRequest) -> bool {
    if crate::template::template_by_name(&sim.defs, &request.what).is_none() {
        return false;
    }
    let Some(realm) = sim.cosmos.realm_of(loc.grid) else {
        return false;
    };
    let index = sim
        .agents
        .residents
        .range(
            ResidentKey {
                realm,
                home: loc.cell,
                index: 0,
            }..=ResidentKey {
                realm,
                home: loc.cell,
                index: u16::MAX,
            },
        )
        .count();
    let key = ResidentKey {
        realm,
        home: loc.cell,
        index: u16::try_from(index).unwrap_or(u16::MAX),
    };
    let text = |name: &str| {
        request
            .params
            .get(name)
            .and_then(clash_core::Value::as_str)
            .map(str::to_owned)
    };
    let resident = Resident {
        what: request.what.clone(),
        map: text(clash_gen::place::SPAWN_PARAM_MAP),
        seed: resident_seed(sim, key),
        entity: None,
        agent: None,
        gone: false,
    };
    sim.agents.residents.insert(key, resident);
    place_cold(sim, key);
    true
}

/// Adds a resident of a realm whose home is a cell, made from a template
/// by id as written, as a request generation hands over would, whether
/// or not the cell is loaded: it comes into view where its routine puts
/// it if that is loaded, and is otherwise an agent from the start. For
/// rules' and tools' residents, and for tests of the cold layer. `None`
/// if no template answers to `what` or the realm has no grid.
pub fn add_resident(
    sim: &mut Sim,
    realm: RealmId,
    home: CellPos,
    what: &str,
) -> Option<ResidentKey> {
    let grid = sim.cosmos.realm(realm)?.base_grid;
    let request = SpawnRequest {
        what: what.to_owned(),
        at: home,
        params: clash_core::VarMap::new(),
    };
    let before = sim.agents.residents.len();
    if !take_spawn_request(sim, Location::new(grid, home), &request) {
        return None;
    }
    debug_assert_eq!(sim.agents.residents.len(), before + 1);
    sim.agents
        .residents
        .range(
            ResidentKey {
                realm,
                home,
                index: 0,
            }..=ResidentKey {
                realm,
                home,
                index: u16::MAX,
            },
        )
        .next_back()
        .map(|(key, _)| *key)
}

/// Works out where a cold resident is now: brings it into view if that
/// is loaded, else keeps its agent state and schedules its wake-up.
fn place_cold(sim: &mut Sim, key: ResidentKey) {
    let Some(resident) = sim.agents.residents.get(&key).cloned() else {
        return;
    };
    if resident.gone {
        return;
    }
    let Some(state) = plan_now(sim, key, &resident) else {
        return;
    };
    let at = position_now(sim, &state);
    if sim.cosmos.is_loaded(at) && bring(sim, key, at, &state).is_some() {
        return;
    }
    set_cold(sim, key, state);
}

/// Keeps a resident cold with an agent state, scheduling its wake-up.
fn set_cold(sim: &mut Sim, key: ResidentKey, state: AgentState) {
    unschedule(sim, key);
    if let Some(wake) = state.wake {
        sim.agents.schedule.insert((wake, key));
    }
    if let Some(resident) = sim.agents.residents.get_mut(&key) {
        resident.agent = Some(state);
    }
    sync_wake(sim);
}

/// Takes a resident's wake-up out of the schedule.
fn unschedule(sim: &mut Sim, key: ResidentKey) {
    if let Some(wake) = sim
        .agents
        .residents
        .get(&key)
        .and_then(|resident| resident.agent.as_ref())
        .and_then(|agent| agent.wake)
    {
        sim.agents.schedule.remove(&(wake, key));
    }
}

/// Keeps the one wake-up on the timeline at the earliest of the
/// schedule.
fn sync_wake(sim: &mut Sim) {
    let earliest = sim.agents.schedule.first().map(|(tick, _)| *tick);
    if sim.agents.wake.map(|(tick, _)| tick) == earliest {
        return;
    }
    if let Some((_, id)) = sim.agents.wake.take() {
        sim.cancel_wake(id);
    }
    let Some(at) = earliest else {
        return;
    };
    let after = at.since(sim.now()).max(Dur::ZERO);
    if let Ok(id) = sim.schedule_on(
        ClockId::UNIVERSAL,
        after,
        Wake::new(WakeDomain::Stepper, 0, WAKE_AGENTS),
    ) {
        sim.agents.wake = Some((sim.now() + after, id));
    }
}

/// Brings a resident into view near a place (09 §10.4): its stored
/// individual comes back, or a new anonymous one is made from its seed.
/// Its mind thinks as it comes into the world. Returns the individual,
/// or `None` if there was nowhere free to put it.
fn bring(sim: &mut Sim, key: ResidentKey, near: Location, state: &AgentState) -> Option<EntityId> {
    let spot = free_spot(sim, near)?;
    let resident = sim.agents.residents.get(&key)?.clone();
    let entity = match resident.entity {
        Some(entity) if sim.entities.contains(entity) => {
            sim.relocate(entity, Place::at(spot), AGENT_CAUSE).ok()?;
            let payload = EventPayload::new(props::AGENT_LOADED)
                .subject("self", Subject::Entity(entity))
                .with("doing", state.doing.what.as_str());
            sim.emit(
                Where::Entity(entity),
                AGENT_CAUSE,
                EventBody::Custom(payload),
            );
            entity
        }
        _ => {
            let spec = CharacterSpec::of([resident.what.clone()]).random(resident.seed);
            let spawned = match sim.spawn_anonymous(&spec, Place::at(spot), AGENT_CAUSE) {
                Ok(spawned) => spawned,
                Err(reason) => {
                    sim.fault(reason.with("what", resident.what.as_str()));
                    return None;
                }
            };
            spawned.entity
        }
    };
    unschedule(sim, key);
    if let Some(resident) = sim.agents.residents.get_mut(&key) {
        resident.entity = Some(entity);
        resident.agent = None;
    }
    sim.agents.by_entity.insert(entity, key);
    sync_wake(sim);
    Some(entity)
}

/// The kernel's call when a tract is about to unload, before it shelves
/// what stands there (09 §10.4): each resident's individual standing in
/// it becomes an agent. An anonymous one becomes nothing, its record
/// cold; a notable one is stored out of the world.
pub(crate) fn tract_unloading(sim: &mut Sim, realm: RealmId, tract: TractPos) {
    if sim.agents.by_entity.is_empty() {
        return;
    }
    let Some(grid) = sim.cosmos.realm(realm).map(|realm| realm.base_grid) else {
        return;
    };
    let mut standing: Vec<EntityId> = Vec::new();
    if let Ok(view) = sim.cosmos.grid(grid)
        && let Some(column) = view.column(tract)
    {
        for (_, chunk) in column.chunks() {
            for entry in chunk.occupants().iter() {
                if sim.agents.by_entity.contains_key(&entry.id) {
                    clash_core::sorted::insert(&mut standing, entry.id);
                }
            }
        }
    }
    for entity in standing {
        store(sim, entity);
    }
}

/// Takes a resident's individual out of the world as an agent.
fn store(sim: &mut Sim, entity: EntityId) {
    let Some(key) = sim.agents.by_entity.get(&entity).copied() else {
        return;
    };
    let Some(resident) = sim.agents.residents.get(&key).cloned() else {
        return;
    };
    let aim = sim
        .components
        .mind_state
        .get(entity)
        .and_then(crate::ai::MindState::held)
        .map(|held| crate::ai::think::goal_name(sim, held.goal))
        .filter(|goal| {
            routine_goal(sim, entity).map(|own| crate::ai::think::goal_name(sim, own))
                != Some(goal.clone())
        });
    let here = sim.entities.location(entity);
    let mut state = plan_now(sim, key, &resident).unwrap_or(AgentState {
        at: AgentPlace::At {
            loc: here.unwrap_or_else(|| Location::new(clash_world::GridId::new(0), key.home)),
        },
        doing: Activity {
            what: WAITING.to_owned(),
            since: sim.now(),
            until: None,
        },
        aim: None,
        wake: None,
    });
    state.aim = aim;
    if resident_routine(sim, &resident).is_none()
        && let Some(here) = here
    {
        // With no routine it waits where it was.
        state.at = AgentPlace::At { loc: here };
    }
    sim.agents.by_entity.remove(&entity);
    let anonymous = !sim.is_notable(entity);
    if anonymous {
        if let Some(resident) = sim.agents.residents.get_mut(&key) {
            resident.entity = None;
        }
        if let Err(reason) = sim.remove_entity(entity, AGENT_CAUSE) {
            sim.fault(reason);
        }
    } else {
        if let Err(reason) = sim.relocate(entity, Place::Stored, AGENT_CAUSE) {
            sim.fault(reason);
            sim.agents.by_entity.insert(entity, key);
            return;
        }
        sim.agents.by_entity.insert(entity, key);
        let mut payload = EventPayload::new(props::AGENT_STORED)
            .subject("self", Subject::Entity(entity))
            .with("doing", state.doing.what.as_str());
        payload = match &state.aim {
            Some(aim) => payload.with("aim", aim.as_str()),
            None => payload.with("aim", crate::bus::FieldValue::Null),
        };
        sim.emit(Where::Global, AGENT_CAUSE, EventBody::Custom(payload));
    }
    set_cold(sim, key, state);
}

/// The kernel's call when a tract has loaded and what was shelved under
/// it is back (09 §10.4): every cold resident of its realm whose place,
/// or either end of whose way, is in it is placed by its routine, and
/// comes into view if that is loaded.
pub(crate) fn tract_loaded(sim: &mut Sim, realm: RealmId, tract: TractPos) {
    let touched: Vec<ResidentKey> = sim
        .agents
        .residents
        .iter()
        .filter(|(key, resident)| key.realm == realm && !resident.gone)
        .filter(|(_, resident)| {
            resident.agent.as_ref().is_some_and(|agent| match agent.at {
                AgentPlace::At { loc } => loc.cell.tract() == tract,
                AgentPlace::Travelling { from, to, .. } => {
                    from.cell.tract() == tract || to.cell.tract() == tract
                }
            })
        })
        .map(|(key, _)| *key)
        .collect();
    for key in touched {
        place_cold(sim, key);
    }
}

/// Handles a wake-up of the `Stepper` domain: the agents' own, or a
/// recipe's (16 §8.2).
pub(crate) fn on_stepper(sim: &mut Sim, id: WakeId, wake: Wake) {
    if wake.arg != WAKE_AGENTS {
        crate::recipe::on_wake(sim, id, wake);
        return;
    }
    if sim.agents.wake.is_some_and(|(_, have)| have == id) {
        sim.agents.wake = None;
    }
    let now = sim.now();
    let mut due = Vec::new();
    while let Some(&(at, key)) = sim.agents.schedule.first() {
        if at > now {
            break;
        }
        sim.agents.schedule.pop_first();
        due.push(key);
    }
    for key in due {
        if let Some(agent) = sim
            .agents
            .residents
            .get_mut(&key)
            .and_then(|resident| resident.agent.as_mut())
        {
            // Taken off the schedule above.
            agent.wake = None;
        }
        place_cold(sim, key);
    }
    sync_wake(sim);
}

// ---- what happens to individuals ---------------------------------------------

/// What the kernel calls for every event it delivers, once rules have
/// had their say: a resident's individual that dies or is removed leaves
/// its record gone; and what 06 §12.2 lists makes an anonymous individual
/// notable.
pub fn on_event(sim: &mut Sim, event: &SimEvent) {
    match &event.body {
        EventBody::EntityRemoved { entity } => gone(sim, *entity),
        EventBody::Act(ActEvent::ActionStarted { actor, id, .. }) => {
            shared_action(sim, *actor, *id, event.cause);
        }
        EventBody::Custom(payload) => {
            let subject = |name: &str| event.subject(name).and_then(|subject| subject.entity());
            match payload.name.as_str() {
                "entity_died" => {
                    if let Some(entity) = subject("self") {
                        gone(sim, entity);
                    }
                }
                crate::traits::TRAIT_ADDED | crate::traits::TRAIT_REMOVED => {
                    if let Some(entity) = subject("self") {
                        sim.make_notable(entity, event.cause);
                    }
                }
                crate::item::ITEM_TAKEN | crate::item::ITEM_DROPPED => {
                    if let Some(holder) = subject("holder") {
                        sim.make_notable(holder, event.cause);
                    }
                }
                RELATION_CHANGED => {
                    if let Some(entity) = subject("self") {
                        sim.make_notable(entity, event.cause);
                    }
                }
                _ => {}
            }
        }
        _ => {}
    }
}

/// A resident's individual died or was removed: its record is gone, and
/// it is never made again.
fn gone(sim: &mut Sim, entity: EntityId) {
    let Some(key) = sim.agents.by_entity.remove(&entity) else {
        return;
    };
    unschedule(sim, key);
    if let Some(resident) = sim.agents.residents.get_mut(&key) {
        resident.gone = true;
        resident.entity = None;
        resident.agent = None;
    }
    sync_wake(sim);
}

/// An action began: an anonymous individual taking part in it with a
/// controlled or notable entity becomes notable (06 §12.2). Taking part
/// is being its actor or an entity among its parameters.
fn shared_action(sim: &mut Sim, actor: EntityId, id: crate::later::ActionId, cause: Cause) {
    let Some(running) = sim.actions.get(id) else {
        return;
    };
    let mut cast: Vec<EntityId> = vec![actor];
    for value in &running.locals {
        if let Some(Subject::Entity(entity)) = sim.subject_of(&clash_expr::Value::from(value))
            && !cast.contains(&entity)
        {
            cast.push(entity);
        }
    }
    if cast.len() < 2 {
        return;
    }
    let weighty = |sim: &Sim, entity: EntityId| {
        sim.entities
            .get(entity)
            .is_some_and(|record| record.control() != Control::None)
            || (sim.components.identity.contains(entity) && sim.is_notable(entity))
    };
    let anchors: Vec<EntityId> = cast
        .iter()
        .copied()
        .filter(|entity| weighty(sim, *entity))
        .collect();
    for entity in cast {
        if anchors.iter().any(|anchor| *anchor != entity) {
            sim.make_notable(entity, cause);
        }
    }
}

/// Makes a notable individual anonymous again (06 §12.2), raising
/// `became_anonymous`. An agent made anonymous becomes nothing: its
/// resident is made afresh when it next comes into view.
pub fn make_anonymous(
    sim: &mut Sim,
    entity: EntityId,
    cause: Cause,
) -> Result<(), crate::reason::Reason> {
    let Some(identity) = sim.components.identity.get_mut(entity) else {
        return Err(crate::reason::Reason::new(
            reason::NOT_AN_INDIVIDUAL,
            format!("{entity} is not an individual made from templates"),
        )
        .with("entity", entity.to_string()));
    };
    if identity.set_anonymous(true) {
        return Ok(());
    }
    let payload =
        EventPayload::new(props::BECAME_ANONYMOUS).subject("self", Subject::Entity(entity));
    sim.emit(Where::Entity(entity), cause, EventBody::Custom(payload));
    let stored = sim
        .entities
        .get(entity)
        .is_some_and(|record| record.place() == Place::Stored);
    if stored && let Some(key) = sim.agents.by_entity.remove(&entity) {
        if let Some(resident) = sim.agents.residents.get_mut(&key) {
            resident.entity = None;
        }
        if let Err(reason) = sim.remove_entity(entity, cause) {
            sim.fault(reason);
        }
    }
    Ok(())
}
