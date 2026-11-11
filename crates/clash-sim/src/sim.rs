//! [`Sim`]: the state of the simulation and the kernel's basic
//! operations on it (03 §4.1).

use std::collections::VecDeque;
use std::sync::Arc;

use clash_core::codec::{self, CodecError};
use clash_core::{Dur, EntityId, Hash128, RngStreams, StateHash, StateHasher, Tick, Value, VarMap};
use clash_data::{Defs, Diagnostics};
use clash_time::{ClockId, Timeline};
use clash_world::{Cause, Cosmos, EventSink, Location, WorldCtx, WorldDefs, WorldEvent};
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::advance::{AppRequest, Handlers};
use crate::bus::Bus;
use crate::clock::WakeOwners;
use crate::command::InputLog;
use crate::decision::DecisionQueue;
use crate::entity::{Control, EntitySpec, EntityTable, KernelEntity, Place};
use crate::event::{EventBody, SimEvent, Where};
use crate::later::{ActionTable, RuleState, WindowTable};
use crate::reason::{Reason, codes};

/// The four bytes that mark a blob as the simulation's own state.
pub const SAVE_KIND: [u8; 4] = *b"CSIM";

/// The version of the layout [`Sim::save`] writes. Raise it whenever a
/// state type gains, loses or reorders a field.
pub const SAVE_VERSION: u32 = 1;

/// The whole simulation: a pure state machine that takes commands,
/// advances time and emits events (01 §1, 03 §4.1).
///
/// # For frontends, tools and tests
///
/// [`submit`](Sim::submit) a [`Command`](crate::Command),
/// [`advance`](Sim::advance), [`drain_events`](Sim::drain_events), read
/// through [`truth`](Sim::truth), compare [`state_hash`](Sim::state_hash),
/// [`save`](Sim::save) and [`load`](Sim::load). Nothing else changes the
/// simulation from outside (invariant T5).
///
/// # For the simulation's own packages
///
/// Everything else here is for code inside the simulation: the functions
/// that create, remove and [`relocate`](Sim::relocate) entities, schedule
/// wake-ups, assign clocks, raise events and ask for decisions. They are
/// public so that this crate's later packages and its tests can call
/// them, not so that a frontend may.
///
/// # What is state
///
/// Everything except the definitions, the wake-up handlers and the event
/// handlers, which are fixed when the simulation is set up and must be
/// set up the same way again after a load. The world's occupant index is
/// derived: [`reindex`](Sim::reindex) rebuilds it.
#[derive(Clone, Debug)]
pub struct Sim {
    pub(crate) defs: Arc<Defs>,
    pub(crate) cosmos: Cosmos,
    pub(crate) timeline: Timeline,
    pub(crate) entities: EntityTable,
    /// Every running action. A placeholder until package T3.
    pub actions: ActionTable,
    /// Every open window. A placeholder until package T5.
    pub windows: WindowTable,
    /// What the rule runtime keeps. A placeholder until package K5.
    pub rules: RuleState,
    pub(crate) settings: VarMap,
    /// The named random streams (01 §4). A system draws from a stream of
    /// its own name and from no other source of randomness.
    pub rng: RngStreams,
    /// What the world has reported and the kernel has not yet passed on.
    pub(crate) world_events: EventSink<WorldEvent>,
    /// Notifications waiting to be delivered (05 §8).
    pub(crate) events: VecDeque<SimEvent>,
    pub(crate) decisions: DecisionQueue,
    /// What rules have asked the application to do (13 §9.1).
    pub(crate) requests: VecDeque<AppRequest>,
    /// Delivered events, for presentation.
    pub(crate) out: Vec<SimEvent>,
    pub(crate) log: InputLog,
    /// Which entity each of its wake-ups belongs to.
    pub(crate) wakes: WakeOwners,
    /// How many wake-ups have been handled since the simulation began.
    pub(crate) steps: u64,
    /// Not state: who handles the wake-ups of each domain.
    pub(crate) handlers: Handlers,
    /// Not state: the event types and who listens for them.
    pub(crate) bus: Bus,
}

/// Why saved state could not be loaded.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[non_exhaustive]
pub enum LoadError {
    /// The blob is damaged, or is not the simulation's.
    #[error("the saved simulation could not be read: {0}")]
    Codec(#[from] CodecError),
    /// The blob was written with another layout.
    #[error(
        "the saved simulation has layout version {found}, and this build reads version {expected}"
    )]
    Version {
        /// The version the blob carries.
        found: u32,
        /// The version this build reads.
        expected: u32,
    },
}

/// The simulation's hashes: one for each part of its state, so that a
/// divergence can be localised (01 §4), and the total over all of them.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct SimHashes {
    /// The world: realms, grids, areas, sites.
    pub cosmos: Hash128,
    /// The timeline: clocks and wake-ups.
    pub timeline: Hash128,
    /// The entity table.
    pub entities: Hash128,
    /// Actions, windows and rule state.
    pub rules: Hash128,
    /// Settings.
    pub settings: Hash128,
    /// The random streams.
    pub rng: Hash128,
    /// The kernel's own: events and decisions waiting, application
    /// requests, who owns which wake-up, the count of wake-ups handled.
    pub kernel: Hash128,
    /// All of the above.
    pub total: Hash128,
}

/// What [`Sim::save`] writes, borrowed.
#[derive(Serialize)]
struct SavedRef<'a> {
    timeline: &'a Timeline,
    entities: &'a EntityTable,
    actions: &'a ActionTable,
    windows: &'a WindowTable,
    rules: &'a RuleState,
    settings: &'a VarMap,
    rng: &'a RngStreams,
    world_events: &'a EventSink<WorldEvent>,
    events: &'a VecDeque<SimEvent>,
    decisions: &'a DecisionQueue,
    requests: &'a VecDeque<AppRequest>,
    out: &'a Vec<SimEvent>,
    log: &'a InputLog,
    wakes: &'a WakeOwners,
    steps: u64,
}

/// What [`Sim::load`] reads: the same fields in the same order, owned.
#[derive(Deserialize)]
struct Saved {
    timeline: Timeline,
    entities: EntityTable,
    actions: ActionTable,
    windows: WindowTable,
    rules: RuleState,
    settings: VarMap,
    rng: RngStreams,
    world_events: EventSink<WorldEvent>,
    events: VecDeque<SimEvent>,
    decisions: DecisionQueue,
    requests: VecDeque<AppRequest>,
    out: Vec<SimEvent>,
    log: InputLog,
    wakes: WakeOwners,
    steps: u64,
}

impl Sim {
    /// A simulation over loaded definitions, with an empty world, at
    /// tick zero. Every random stream is seeded from `seed`.
    ///
    /// Fails with located diagnostics if the world's tables cannot be
    /// built from the definitions.
    pub fn new(defs: Arc<Defs>, seed: u64) -> Result<Sim, Diagnostics> {
        let world = WorldDefs::from_defs(&defs)?;
        Ok(Sim::from_parts(
            defs,
            Cosmos::new(world),
            Timeline::new(),
            seed,
        ))
    }

    /// A simulation over a world that has already been made: a cosmos
    /// and the timeline its clocks are on. This is how a generated or
    /// hand-built world is handed to the simulation.
    pub fn from_parts(defs: Arc<Defs>, cosmos: Cosmos, timeline: Timeline, seed: u64) -> Sim {
        let bus = Bus::for_defs(&defs);
        Sim {
            defs,
            cosmos,
            timeline,
            entities: EntityTable::new(),
            actions: ActionTable::default(),
            windows: WindowTable::default(),
            rules: RuleState::default(),
            settings: VarMap::new(),
            rng: RngStreams::new(seed),
            world_events: EventSink::new(),
            events: VecDeque::new(),
            decisions: DecisionQueue::default(),
            requests: VecDeque::new(),
            out: Vec::new(),
            log: InputLog::default(),
            wakes: WakeOwners::default(),
            steps: 0,
            handlers: Handlers::default(),
            bus,
        }
    }

    /// The definitions the simulation runs on.
    pub fn defs(&self) -> &Arc<Defs> {
        &self.defs
    }

    /// The current universal time.
    pub fn now(&self) -> Tick {
        self.timeline.now()
    }

    /// The timeline, to read: clocks, rates, what is scheduled.
    pub fn timeline(&self) -> &Timeline {
        &self.timeline
    }

    /// How many wake-ups have been handled since the simulation began.
    pub fn steps(&self) -> u64 {
        self.steps
    }

    /// Ground truth, to read. See [`Truth`].
    pub fn truth(&self) -> Truth<'_> {
        Truth { sim: self }
    }

    /// The world, to change, for simulation code: rule verbs, action
    /// effects, world processes. Cells change only through the
    /// [`WorldEdit`](clash_world::WorldEdit) that the context hands out.
    ///
    /// When the closure returns, whatever the world reported is passed
    /// on to the event queue and the kernel is brought into step with
    /// it: an occupant the world moved out of the way of an edit has its
    /// place follow, and whoever an area came over or left is put on the
    /// clock that now applies.
    ///
    /// Never call `place`, `relocate` or `remove` through it: an entity's
    /// position changes only through [`Sim::relocate`], which calls them.
    pub fn world<R>(&mut self, change: impl FnOnce(&mut WorldCtx<'_>) -> R) -> R {
        let result = change(&mut self.world_ctx());
        self.absorb_world();
        result
    }

    /// The world, to change, for the kernel itself. Whoever uses it must
    /// see that `absorb_world` follows before anything reads an entity's
    /// place: [`Sim::world`] does both.
    pub(crate) fn world_ctx(&mut self) -> WorldCtx<'_> {
        WorldCtx::new(&mut self.cosmos, &mut self.timeline, &mut self.world_events)
    }

    /// The settings rules can read (04 §10).
    pub fn settings(&self) -> &VarMap {
        &self.settings
    }

    /// Sets a setting. Settings are simulation state and change only by
    /// command (decision D-055): this is for the command that package P6
    /// builds, and for setting a scene in a test.
    pub fn set_setting(&mut self, key: &str, value: impl Into<Value>) -> Option<Value> {
        self.settings.set(key, value)
    }

    /// An integer setting, or `default` if it is unset or not an integer.
    pub(crate) fn setting_int(&self, key: &str, default: i64) -> i64 {
        self.settings
            .get(key)
            .and_then(Value::as_int)
            .unwrap_or(default)
    }

    /// An entity's record, or the reason there is none.
    pub(crate) fn known(&self, id: EntityId) -> Result<&KernelEntity, Reason> {
        self.entities.get(id).ok_or_else(|| unknown_entity(id))
    }

    /// An entity's record, to change its open fields: tags, variables,
    /// channels, actions, plan, attached rules.
    pub fn entity_mut(&mut self, id: EntityId) -> Option<&mut KernelEntity> {
        self.entities.get_mut(id)
    }

    // ---- Entities --------------------------------------------------------

    /// Makes an entity. It is not yet anywhere: put it somewhere with
    /// [`relocate`](Sim::relocate). Emits `entity_created`.
    pub fn create_entity(&mut self, spec: EntitySpec, cause: Cause) -> Result<EntityId, Reason> {
        let id = self.entities.create(spec).ok_or_else(|| {
            Reason::new(
                codes::ENTITY_IDS_EXHAUSTED,
                "every entity slot has been used",
            )
        })?;
        self.emit(
            Where::Global,
            cause,
            EventBody::EntityCreated { entity: id },
        );
        Ok(id)
    }

    /// Makes an entity and puts it somewhere. If it cannot be put there
    /// nothing is made, and the reason is the one `relocate` gave.
    pub fn spawn(
        &mut self,
        spec: EntitySpec,
        place: Place,
        cause: Cause,
    ) -> Result<EntityId, Reason> {
        // Whether it can be there is found out before anything is made,
        // so that a refusal leaves no trace.
        let place = self.check_place(None, spec.presence.footprint, place)?;
        let id = self.create_entity(spec, cause)?;
        if place != Place::Stored {
            self.relocate(id, place, cause)?;
        }
        Ok(id)
    }

    /// Removes an entity for good. Emits `entity_removed`.
    ///
    /// What it held is first put where it was: at its cell, in its own
    /// container, or out of the world. It is then taken out of the world,
    /// its wake-ups are cancelled and its personal clock destroyed
    /// (invariant T9), and any decision it was asked for is forgotten.
    pub fn remove_entity(&mut self, id: EntityId, cause: Cause) -> Result<(), Reason> {
        let place = self.known(id)?.place();
        for inside in self.entities.contents(id).to_vec() {
            if self.relocate(inside, place, cause).is_err() {
                // What cannot stand where its container stood leaves the
                // world rather than being lost inside nothing.
                self.relocate(inside, Place::Stored, cause)?;
            }
        }
        self.relocate(id, Place::Stored, cause)?;
        self.drop_wakes(id);
        self.decisions.forget(id);
        if let Some(clock) = self.entities.get(id).and_then(KernelEntity::clock) {
            // A clock that is already gone needs no destroying.
            let _ = self.timeline.destroy_clock(clock);
        }
        self.emit(
            Where::Global,
            cause,
            EventBody::EntityRemoved { entity: id },
        );
        self.entities.free(id);
        Ok(())
    }

    /// Changes who decides for an entity (03 §5.1). Emits
    /// `control_changed` if that changes anything. An entity that stops
    /// being directly controlled is no longer waiting for any decision.
    pub fn set_control(
        &mut self,
        id: EntityId,
        control: Control,
        cause: Cause,
    ) -> Result<Control, Reason> {
        let old = self.known(id)?.control();
        if old == control {
            return Ok(old);
        }
        if let Some(entity) = self.entities.get_mut(id) {
            entity.control = control;
        }
        if control != Control::Direct {
            self.decisions.forget(id);
        }
        self.emit(
            Where::Entity(id),
            cause,
            EventBody::ControlChanged {
                entity: id,
                old,
                new: control,
            },
        );
        Ok(old)
    }

    // ---- Events ----------------------------------------------------------

    /// Reports that something happened. The event joins the back of the
    /// queue and is delivered to handlers, and then to presentation, at
    /// the next pump (03 §4.3): after the wake-up or command being
    /// handled has finished.
    ///
    /// The event is stamped with the universal tick and with the local
    /// time of the clock that `on` names. Whatever the world has
    /// reported since the last event is passed on first, so that events
    /// keep the order they happened in.
    pub fn emit(&mut self, on: Where, cause: Cause, body: EventBody) {
        self.absorb_world();
        let event = self.stamped(on, cause, body);
        self.events.push_back(event);
    }

    /// An event stamped with the time, here and on the clock of `on`.
    pub(crate) fn stamped(&self, on: Where, cause: Cause, body: EventBody) -> SimEvent {
        let clock = match on {
            Where::Entity(id) => self.clock_of(id).unwrap_or(ClockId::UNIVERSAL),
            Where::Loc(loc) => self.location_clock(loc),
            Where::Clock(clock) => clock,
            Where::Global => ClockId::UNIVERSAL,
        };
        let tick = self.timeline.now();
        let (clock, local) = match self.timeline.local_now(clock) {
            Ok(local) => (clock, local),
            // A clock that has gone: universal time is always there.
            Err(_) => (ClockId::UNIVERSAL, tick.since(Tick::ZERO)),
        };
        SimEvent {
            tick,
            clock,
            local,
            cause,
            body,
        }
    }

    /// Passes on what the world has reported, in order, and keeps the
    /// kernel in step with it: an occupant the world moved out of the
    /// way of an edit is followed, and whoever an area came over or left
    /// is put on the clock that now applies.
    pub(crate) fn absorb_world(&mut self) {
        if self.world_events.is_empty() {
            return;
        }
        for event in self.world_events.drain() {
            let mut relocated = None;
            match &event {
                WorldEvent::OccupantDisplaced {
                    occupant,
                    to,
                    cause,
                    ..
                } => {
                    if let Some(from) = self.follow_displacement(*occupant, *to) {
                        relocated = Some((*occupant, from, *cause));
                    }
                }
                WorldEvent::AreaEntered { occupant, .. }
                | WorldEvent::AreaExited { occupant, .. } => self.sync_clocks_under(*occupant),
                _ => {}
            }
            let (cause, loc) = (event.cause(), event.location());
            let stamped = self.stamped(Where::Loc(loc), cause, EventBody::World(event));
            self.events.push_back(stamped);
            if let Some((entity, from, cause)) = relocated {
                let to = self
                    .entities
                    .get(entity)
                    .map_or(Place::Stored, KernelEntity::place);
                let event = self.stamped(
                    Where::Entity(entity),
                    cause,
                    EventBody::EntityRelocated { entity, from, to },
                );
                self.events.push_back(event);
            }
        }
    }

    /// Takes the events delivered since the last call, oldest first, for
    /// presentation (01 §5.3).
    pub fn drain_events(&mut self) -> Vec<SimEvent> {
        core::mem::take(&mut self.out)
    }

    /// The events delivered and not yet drained.
    pub fn undrained_events(&self) -> &[SimEvent] {
        &self.out
    }

    // ---- Hashing, saving and loading -------------------------------------

    /// The hash of the whole simulation state (01 §4). Two simulations
    /// with the same hash will behave the same from here on.
    ///
    /// The output waiting to be drained and the input log are records of
    /// what has happened, not state that decides what will: they are not
    /// hashed, and draining them changes nothing.
    pub fn state_hash(&self) -> Hash128 {
        self.hashes().total
    }

    /// The hash of each part of the state, and the total.
    pub fn hashes(&self) -> SimHashes {
        let cosmos = Hash128::of(&self.cosmos);
        let timeline = Hash128::of(&self.timeline);
        let entities = Hash128::of(&self.entities);
        let rules = Hash128::of(&(&self.actions, &self.windows, &self.rules));
        let settings = Hash128::of(&self.settings);
        let rng = Hash128::of(&self.rng);
        let mut kernel = StateHasher::new();
        self.world_events.state_hash(&mut kernel);
        self.events.state_hash(&mut kernel);
        self.decisions.state_hash(&mut kernel);
        self.requests.state_hash(&mut kernel);
        self.wakes.state_hash(&mut kernel);
        kernel.write_u64(self.steps);
        let kernel = kernel.finish();
        let total = Hash128::of(&[cosmos, timeline, entities, rules, settings, rng, kernel]);
        SimHashes {
            cosmos,
            timeline,
            entities,
            rules,
            settings,
            rng,
            kernel,
            total,
        }
    }

    /// The simulation's own state as a blob: everything but the
    /// definitions and the world, which have stores of their own
    /// (02 §11). Save at a safe point: between two calls of
    /// [`advance`](Sim::advance).
    pub fn save(&self) -> Result<Vec<u8>, CodecError> {
        codec::encode_blob(
            SAVE_KIND,
            SAVE_VERSION,
            &SavedRef {
                timeline: &self.timeline,
                entities: &self.entities,
                actions: &self.actions,
                windows: &self.windows,
                rules: &self.rules,
                settings: &self.settings,
                rng: &self.rng,
                world_events: &self.world_events,
                events: &self.events,
                decisions: &self.decisions,
                requests: &self.requests,
                out: &self.out,
                log: &self.log,
                wakes: &self.wakes,
                steps: self.steps,
            },
        )
    }

    /// A simulation from a blob written by [`save`](Sim::save), over the
    /// definitions and the world it was saved with.
    ///
    /// The world does not store who occupies it. Once the cosmos holds
    /// its loaded tracts, call [`reindex`](Sim::reindex) to place every
    /// entity again. Wake-up handlers and event handlers are not state:
    /// register them again as they were.
    pub fn load(defs: Arc<Defs>, cosmos: Cosmos, blob: &[u8]) -> Result<Sim, LoadError> {
        let (version, body) = codec::open_blob(SAVE_KIND, blob)?;
        if version != SAVE_VERSION {
            return Err(LoadError::Version {
                found: version,
                expected: SAVE_VERSION,
            });
        }
        let saved: Saved = codec::from_bytes(&body)?;
        let bus = Bus::for_defs(&defs);
        Ok(Sim {
            defs,
            cosmos,
            timeline: saved.timeline,
            entities: saved.entities,
            actions: saved.actions,
            windows: saved.windows,
            rules: saved.rules,
            settings: saved.settings,
            rng: saved.rng,
            world_events: saved.world_events,
            events: saved.events,
            decisions: saved.decisions,
            requests: saved.requests,
            out: saved.out,
            log: saved.log,
            wakes: saved.wakes,
            steps: saved.steps,
            handlers: Handlers::default(),
            bus,
        })
    }
}

/// The reason for an id that names no entity.
pub(crate) fn unknown_entity(id: EntityId) -> Reason {
    Reason::new(codes::ENTITY_UNKNOWN, format!("there is no entity {id}"))
        .with("entity", id.to_string())
}

/// Ground truth, to read: what is really where (01 R11).
///
/// Truth is for the engine, physics, saves, the inspector, scoring and
/// tests. What a character decides by must come from what that character
/// has perceived or been told, never from here: nothing reached from a
/// goal, a method, a tactic, an action's defaults or an ordinary screen
/// may hold a `Truth` (05 §7.3). Keeping every such read behind this one
/// type is what lets the mind context be kept away from them.
#[derive(Clone, Copy, Debug)]
pub struct Truth<'a> {
    sim: &'a Sim,
}

impl<'a> Truth<'a> {
    /// The world.
    pub fn cosmos(self) -> &'a Cosmos {
        &self.sim.cosmos
    }

    /// Every entity's kernel record.
    pub fn entities(self) -> &'a EntityTable {
        &self.sim.entities
    }

    /// One entity's kernel record.
    pub fn entity(self, id: EntityId) -> Option<&'a KernelEntity> {
        self.sim.entities.get(id)
    }

    /// Where an entity is.
    pub fn place(self, id: EntityId) -> Option<Place> {
        self.sim.entities.get(id).map(KernelEntity::place)
    }

    /// The cell an entity is in: its own, or that of the container that
    /// holds whatever it is nested in. `None` if it is not in the world.
    pub fn location(self, id: EntityId) -> Option<Location> {
        self.sim.entities.location(id)
    }

    /// What is directly inside an entity, in id order.
    pub fn contents(self, id: EntityId) -> &'a [EntityId] {
        self.sim.entities.contents(id)
    }

    /// The clock an entity is on (03 §3.4). `None` if it is not in the
    /// world and has no clock of its own.
    pub fn clock_of(self, id: EntityId) -> Option<ClockId> {
        self.sim.clock_of(id)
    }

    /// How much local time has passed for an entity, on whatever clocks
    /// it has been on, while it was in the world.
    pub fn own_time(self, id: EntityId) -> Dur {
        self.sim.own_time(id)
    }

    /// The wake-ups scheduled for an entity, in the order they were
    /// scheduled.
    pub fn wakes_of(self, id: EntityId) -> Vec<clash_time::WakeId> {
        self.sim.wakes.of(id)
    }
}
