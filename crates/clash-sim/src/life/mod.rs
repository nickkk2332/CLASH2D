//! Life: what joins creatures to the running simulation
//! (`docs/06-entities-and-bodies.md`).
//!
//! Attributes, pools and tracks ([`stats`](mod@crate::stats)) and bodies,
//! hits and wounds ([`body`](mod@crate::body)) are functions over stores
//! that return what changed. This module and the wiring beside each of
//! those hold them in the simulation, call them when the kernel must,
//! and raise their events.
//!
//! | Need | Use |
//! |------|-----|
//! | To give an entity stats, and change them | `stats::bind`: [`Sim::attach_stats`], [`Sim::give_pool`], [`Sim::pool_add`], [`Sim::pool_rate`], [`Sim::set_attr_base`], [`Sim::add_modifier`], [`Sim::advance_track`], [`Sim::refresh_stats`] |
//! | To give an entity a body, and change it | `body::bind`: [`Sim::attach_body`], [`Sim::close_wound`], [`Sim::heal_part`], [`Sim::disable_part`], [`Sim::remove_part`] |
//! | To harm a body | [`Sim::hit`] with a [`Hit`](crate::body::Hit): the hook `hit_incoming`, the layers, the events, what exits and what came off |
//! | What the world did to an occupant | [`harm`]: `occupant_exposed` and `occupant_dosed` become hits and doses as the world's events are passed on |
//! | Minds, death and waking | [`mind`](crate::mind): [`Sim::die`], [`Sim::revive`], [`Sim::vital`], [`Sim::can_act`] |
//! | To make a creature from data | [`template`](crate::template): `Sim::spawn_template` |
//! | The events and hooks of all of it | [`events::LIFE_EVENTS`] |
//!
//! # What is not state
//!
//! [`Life`]: the tables worked out from the definitions
//! ([`StatDefs`], [`BodyDefs`], the templates') and a [`Runner`]. It is
//! made with the simulation and again after a load.
//!
//! # How a stat's expression is evaluated
//!
//! A function of `stats` is handed the store of stats and the timeline
//! to change, and a host that answers everything else an expression
//! reads. The host is the simulation itself, so while such a function
//! runs the simulation is taken apart: the stats' store and the timeline
//! are lent to the function, and the host reads a simulation that holds
//! an empty store of stats (the function answers stats from the store it
//! was lent) and a copy of the timeline's clocks with nothing scheduled
//! ([`Timeline::clocks_only`](clash_time::Timeline::clocks_only)), which tells the time on every clock
//! exactly. Both are put back when the function returns
//! (`Sim::with_stats`).
//!
//! The functions of `body` never evaluate an expression, so they are
//! handed the stores as they stand and a host that answers nothing
//! (`Sim::with_body`). When one of them reports that a function of the
//! body moved, the wiring works the stats out again afterwards.
//!
//! # Time
//!
//! Pools and bodies keep their own wake-ups, on their owner's clock, in
//! the `Timer` domain, told apart by tag bits of the wake-up's `arg`
//! ([`stats::POOL_WAKE_TAG`], [`body::BODY_WAKE_ARG`]). The rest of the
//! domain is free: [`handle_wake`] says whether a wake-up was one of
//! these, and a package with timers of its own handles the others.
//!
//! The kernel does not know these wake-ups as the entity's, so it
//! neither moves nor cancels them. Whenever it puts an entity on the
//! clock that now applies ([`Sim::relocate`], an area that came or went,
//! a personal clock given or taken) it calls `Sim::life_follow`, which
//! moves the entity's pools and body to that clock, takes them out of
//! time when the entity leaves the world and puts them back when it
//! returns. An entity that returns resumes where it stood: no time
//! passed for it while it was away (decision D-552).

pub(crate) mod args;
pub mod events;
pub mod harm;

use std::sync::Arc;

use clash_core::{Dur, EntityId, Rng};
use clash_data::{Defs, Runner};
use clash_expr::{DefRef, FunctionId, Handle, Host, PropertyId, RuntimeWarning, Value};
use clash_time::{ClockId, Wake, WakeId};
use clash_world::{Cause, CauseKind, WorldDefs};

use crate::Sim;
use crate::body::{self, BodyCx, BodyDefs, BodyError};
use crate::host::{Bound, SimHost};
use crate::mind::VitalDefs;
use crate::reason::{Reason, ReasonInfo};
use crate::stats::{self, StatDefs, StatsCx, StatsError};
use crate::template::TemplateDefs;

/// The name of the random stream hits draw from (06 §7.3): which part
/// an unaimed hit finds, which inner part is struck, whether a worn
/// layer is in the way.
pub const HIT_STREAM: &str = "hits";

/// The code of the fault reported when a stat's own expression raised a
/// run-time warning while it was worked out.
pub const EXPRESSION_WARNING: &str = "stats.expression_warning";

/// The reasons of the wiring itself, beside those of
/// [`stats::reason`], [`body::reason`],
/// [`mind::MIND_REASONS`](crate::mind::MIND_REASONS) and
/// [`template::TEMPLATE_REASONS`](crate::template::TEMPLATE_REASONS).
pub const LIFE_REASONS: &[ReasonInfo] = &[ReasonInfo {
    code: EXPRESSION_WARNING,
    doc: "A fault, not a refusal: an expression of an attribute or a pool raised a run-time warning while it was worked out. Values: `entity`, `count`, `warning`.",
}];

/// A host that answers nothing: for functions that never evaluate an
/// expression (`Sim::with_body`).
#[derive(Clone, Copy, Debug, Default)]
pub struct NullHost;

impl Host for NullHost {
    fn subject(&mut self, _slot: u16) -> Value {
        Value::Null
    }

    fn local(&mut self, _slot: u16) -> Value {
        Value::Null
    }

    fn event_field(&mut self, _slot: u16) -> Value {
        Value::Null
    }

    fn property(&mut self, _property: PropertyId, _subject: Handle, _args: &[Value]) -> Value {
        Value::Null
    }

    fn variable(&mut self, _subject: Handle, _name: &str) -> Value {
        Value::Null
    }

    fn call(&mut self, _function: FunctionId, _args: &[Value]) -> Value {
        Value::Null
    }

    fn rng(&mut self) -> Option<&mut Rng> {
        None
    }

    fn warn(&mut self, _warning: RuntimeWarning) {}

    fn def_name(&self, _id: DefRef) -> Option<&str> {
        None
    }
}

/// What the simulation keeps for living things beside its state: the
/// tables worked out from the definitions, and the machines stats'
/// expressions run on. Not state: made with the simulation and again
/// after a load.
#[derive(Clone, Debug)]
pub struct Life {
    /// The stats' tables. `None` if the definitions were loaded without
    /// the stats' definition types.
    stats: Option<Arc<StatDefs>>,
    /// The bodies' tables. `None` if the definitions were loaded without
    /// the bodies' definition types, or the stats'.
    body: Option<Arc<BodyDefs>>,
    /// What stops a creature, by its definitions (06 §8.3).
    vital: Arc<VitalDefs>,
    /// The templates' tables.
    #[allow(dead_code)] // Read once templates are built (06 §10).
    templates: Arc<TemplateDefs>,
    /// The qualified names of the variables a stat's expression reads
    /// directly.
    variables: Arc<std::collections::BTreeSet<String>>,
    /// The machines expressions run on.
    pub(crate) runner: Runner,
}

impl Life {
    /// The tables for a set of loaded definitions and the world's tables
    /// built from them.
    pub(crate) fn for_defs(defs: &Arc<Defs>, world: &Arc<WorldDefs>) -> Life {
        let stats = StatDefs::new(defs).ok().map(Arc::new);
        let body = stats
            .as_ref()
            .and_then(|_| BodyDefs::new(defs, world).ok())
            .map(Arc::new);
        let vital = Arc::new(VitalDefs::new(defs, stats.as_deref(), body.as_deref()));
        let templates = Arc::new(TemplateDefs::new(defs));
        let variables = Arc::new(
            stats
                .as_deref()
                .map(stats::bind::variables_read)
                .unwrap_or_default(),
        );
        Life {
            stats,
            body,
            vital,
            templates,
            variables,
            runner: Runner::new(),
        }
    }

    /// The stats' tables, or the refusal that they are not registered.
    pub(crate) fn stat_defs(&self) -> Result<Arc<StatDefs>, Reason> {
        self.stats.clone().ok_or_else(|| {
            stats_reason(&StatsError::NotRegistered {
                what: "the stats' definition types",
            })
        })
    }

    /// The bodies' tables, or the refusal that they are not registered.
    pub(crate) fn body_defs(&self) -> Result<Arc<BodyDefs>, Reason> {
        self.body.clone().ok_or_else(|| {
            body_reason(&BodyError::NotRegistered {
                what: "the bodies' definition types",
            })
        })
    }

    /// What stops a creature, by its definitions.
    pub(crate) fn vital(&self) -> &Arc<VitalDefs> {
        &self.vital
    }

    /// The templates' tables.
    #[allow(dead_code)] // Read once templates are built (06 §10).
    pub(crate) fn templates(&self) -> &Arc<TemplateDefs> {
        &self.templates
    }

    /// Whether a stat's expression reads a variable of this name
    /// directly.
    pub(crate) fn reads_variable(&self, name: &str) -> bool {
        self.variables.contains(name)
    }
}

/// The reason for a refusal by the stats (14 §5): its code, its
/// sentence and the values involved.
pub fn stats_reason(error: &StatsError) -> Reason {
    let mut reason = Reason::new(error.code(), error.to_string());
    for (name, value) in error.values() {
        reason = reason.with(name, value.to_stored());
    }
    reason
}

/// The reason for a refusal by the body model (14 §5).
pub fn body_reason(error: &BodyError) -> Reason {
    let mut reason = Reason::new(error.code(), error.to_string());
    for (name, value) in error.values() {
        reason = reason.with(name, value.to_stored());
    }
    reason
}

/// The cause of what a creature's own pools and wounds do as time
/// passes: a process, with nobody behind it.
pub(crate) const TIME_CAUSE: Cause = Cause::new(CauseKind::Process);

/// Handles a wake-up of the `Timer` domain if it is a pool's crossing or
/// a body's closing and healing, and says whether it was. A package with
/// timers of its own calls this first and handles the rest.
pub fn handle_wake(sim: &mut Sim, wake: Wake) -> bool {
    if stats::is_pool_wake(&wake) {
        let entity = EntityId::from_bits(wake.id);
        let done = sim.with_body(|cx, _| stats::on_wake(&mut cx.stats, wake));
        match done {
            Ok(Ok(changes)) => sim.raise_stat_changes(entity, changes, TIME_CAUSE),
            Ok(Err(error)) => sim.fault(stats_reason(&error)),
            Err(reason) => sim.fault(reason),
        }
        true
    } else if body::is_body_wake(&wake) {
        let entity = EntityId::from_bits(wake.id);
        let done = sim.with_body(|cx, _| body::on_wake(cx, wake));
        match done {
            Ok(Ok(changes)) => sim.raise_body_changes(entity, changes, TIME_CAUSE),
            Ok(Err(error)) => sim.fault(body_reason(&error)),
            Err(reason) => sim.fault(reason),
        }
        true
    } else {
        false
    }
}

/// The default handler of the `Timer` wake-up domain: pools and bodies.
/// A timer that is neither is reported as a fault, as any wake-up nobody
/// handles is.
pub(crate) fn timer_wake(sim: &mut Sim, _id: WakeId, wake: Wake) {
    if !handle_wake(sim, wake) {
        sim.fault(
            Reason::new(
                crate::reason::codes::ADVANCE_UNHANDLED_WAKE,
                "a timer was due that is neither a pool's nor a body's, and nothing else handles timers yet",
            )
            .with("domain", format!("{:?}", wake.domain))
            .with("id", wake.id as i64)
            .with("arg", wake.arg as i64),
        );
    }
}

impl Sim {
    /// The stats' tables for the loaded definitions, if their definition
    /// types were registered.
    pub fn stat_defs(&self) -> Option<&Arc<StatDefs>> {
        self.life.stats.as_ref()
    }

    /// The bodies' tables for the loaded definitions, if their
    /// definition types were registered.
    pub fn body_defs(&self) -> Option<&Arc<BodyDefs>> {
        self.life.body.as_ref()
    }

    /// Runs something that changes stats and may evaluate their
    /// expressions. See the [module documentation](self): while it runs,
    /// the host reads the simulation with the stats' store and the
    /// timeline lent out.
    ///
    /// Refused if the stats' definition types were not registered.
    pub(crate) fn with_stats<R>(
        &mut self,
        entity: EntityId,
        run: impl FnOnce(&mut StatsCx<'_, SimHost<'_>>) -> R,
    ) -> Result<R, Reason> {
        let defs = self.life.stat_defs()?;
        let mut store = core::mem::take(&mut self.components.stats);
        let view = self.timeline.clocks_only();
        let mut timeline = core::mem::replace(&mut self.timeline, view);
        let mut runner = core::mem::take(&mut self.life.runner);
        let (result, warnings) = {
            let mut host = self.host(Bound::new());
            let result = run(&mut StatsCx {
                stats: &mut store,
                defs: &defs,
                timeline: &mut timeline,
                host: &mut host,
                runner: &mut runner,
            });
            (result, host.take_warnings())
        };
        self.components.stats = store;
        self.timeline = timeline;
        self.life.runner = runner;
        if let Some(first) = warnings.first() {
            self.fault(
                Reason::new(
                    EXPRESSION_WARNING,
                    format!(
                        "working out the stats of {entity} raised {} warnings; the first: {}",
                        warnings.len(),
                        first.fault
                    ),
                )
                .with("entity", entity.to_string())
                .with("count", warnings.len() as i64)
                .with("warning", first.fault.to_string()),
            );
        }
        Ok(result)
    }

    /// Runs something that changes bodies, or stats without evaluating
    /// anything, over the stores as they stand, with the stream hits
    /// draw from.
    ///
    /// Refused if the bodies' definition types were not registered.
    pub(crate) fn with_body<R>(
        &mut self,
        run: impl FnOnce(&mut BodyCx<'_, NullHost>, &mut Rng) -> R,
    ) -> Result<R, Reason> {
        let stat_defs = self.life.stat_defs()?;
        let body_defs = self.life.body_defs()?;
        let mut host = NullHost;
        let mut cx = BodyCx {
            bodies: &mut self.components.body,
            defs: &body_defs,
            physical: &mut self.components.physical,
            containment: &self.entities,
            stats: StatsCx {
                stats: &mut self.components.stats,
                defs: &stat_defs,
                timeline: &mut self.timeline,
                host: &mut host,
                runner: &mut self.life.runner,
            },
        };
        let rng = self.rng.stream(HIT_STREAM);
        Ok(run(&mut cx, rng))
    }

    /// A property of the stats or of bodies, if `property` is one of
    /// theirs: what [`Sim::property`] forwards (05 §5.3).
    pub(crate) fn life_property(
        &self,
        property: PropertyId,
        subject: Handle,
        args: &[Value],
    ) -> Option<Value> {
        if let Some(defs) = &self.life.stats
            && let Some(value) = stats::property(
                &self.components.stats,
                defs,
                &self.timeline,
                property,
                subject,
                args,
            )
        {
            return Some(value);
        }
        let defs = self.life.body.as_ref()?;
        body::property(
            &self.components.body,
            defs,
            &self.timeline,
            property,
            subject,
            args,
        )
    }

    /// The clock an entity's pools and wounds are to run on: the clock
    /// it is on, if it is in the world.
    pub(crate) fn life_clock(&self, entity: EntityId) -> Option<ClockId> {
        self.entities
            .location(entity)
            .and_then(|_| self.clock_of(entity))
    }

    /// Brings an entity's pools and body into step with where it now is:
    /// onto the clock that applies, out of time if it has left the
    /// world, back into time if it has returned. The kernel calls this
    /// whenever it puts an entity on the clock that now applies. It does
    /// nothing for an entity with neither stats nor a body.
    pub(crate) fn life_follow(&mut self, entity: EntityId) {
        let stats = self
            .components
            .stats
            .get(entity)
            .map(|stats| (stats.is_suspended(), stats.clock()));
        let body = self
            .components
            .body
            .get(entity)
            .map(|body| (body.is_suspended(), body.clock()));
        if stats.is_none() && body.is_none() {
            return;
        }
        let target = self.life_clock(entity);
        // What to do about one of the two, given whether it stands
        // still and the clock it is on.
        #[derive(Clone, Copy)]
        enum Step {
            Nothing,
            Suspend,
            Resume(ClockId),
            Move(ClockId),
        }
        let step = |held: Option<(bool, ClockId)>| match (held, target) {
            (Some((false, _)), None) => Step::Suspend,
            (Some((true, _)), Some(clock)) => Step::Resume(clock),
            (Some((false, on)), Some(clock)) if on != clock => Step::Move(clock),
            _ => Step::Nothing,
        };
        let (stats_step, body_step) = (step(stats), step(body));
        if matches!((stats_step, body_step), (Step::Nothing, Step::Nothing)) {
            return;
        }
        let done = self.with_body(|cx, _| {
            let stat_changes = match stats_step {
                Step::Nothing => Ok(Vec::new()),
                Step::Suspend => stats::suspend(&mut cx.stats, entity),
                Step::Resume(clock) => stats::resume(&mut cx.stats, entity, clock, Dur::ZERO),
                Step::Move(clock) => stats::set_clock(&mut cx.stats, entity, clock),
            };
            let body_changes = match body_step {
                Step::Nothing => Ok(Vec::new()),
                Step::Suspend => body::suspend(cx, entity),
                Step::Resume(clock) => body::resume(cx, entity, clock, Dur::ZERO),
                Step::Move(clock) => body::set_clock(cx, entity, clock),
            };
            (stat_changes, body_changes)
        });
        match done {
            Ok((stat_changes, body_changes)) => {
                match stat_changes {
                    Ok(changes) => self.raise_stat_changes(entity, changes, TIME_CAUSE),
                    Err(error) => self.fault(stats_reason(&error)),
                }
                match body_changes {
                    Ok(changes) => self.raise_body_changes(entity, changes, TIME_CAUSE),
                    Err(error) => self.fault(body_reason(&error)),
                }
            }
            Err(reason) => self.fault(reason),
        }
    }

    /// Sets up what is not state for living things: called when a
    /// simulation is made and again after a load.
    pub(crate) fn life_for(defs: &Arc<Defs>, world: &Arc<WorldDefs>) -> Life {
        Life::for_defs(defs, world)
    }
}
