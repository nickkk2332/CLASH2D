//! Thinking (09 §3.3) and pursuing a goal by its methods (09 §4).
//!
//! # When a mind thinks
//!
//! A mind is **prompted** ([`prompt`]) when it is free and must be told
//! what to do (`decision_needed`), when it perceives something at least
//! as clear as its `wake_clarity`, when an event one of its goals names
//! in `wake_on` binds it, when it comes into the world or under nobody's
//! direct control, when a step it is pursuing ends, and when a rule
//! tells it to (`think`). With nothing to prompt it, it thinks when its
//! interval has passed: its `think`, stretched up to `stretch` times
//! while nothing changes, and staggered for each mind by a hash of its
//! entity so that minds made together do not all think together.
//!
//! Prompted minds think in a **batch**, when the events of the moment
//! have all been delivered ([`think_due`]): in order of entity, each
//! deciding from its own state and beliefs. What one mind does in a
//! batch reaches another only through events, which are delivered after
//! the batch; so whom a mind adopts does not depend on which other minds
//! thought before it (invariant N8). A mind thinks at most
//! `ai.max_thoughts_per_tick` times in one tick; a batch thinks for at
//! most `ai.max_minds_per_batch` minds, and the rest wait for the next.
//!
//! # Choosing
//!
//! Each goal the mind may have (its own and those its `adjust` entries
//! add) is given its `about`, then scored, with its `noise` drawn from
//! the mind's own stream. The best goal with a score above zero is
//! adopted, unless the goal held scores within its `commit` of it. The
//! goal held is also tested for `done` and `fail`; a goal that failed is
//! set aside for its `retry_after`.
//!
//! # Pursuing
//!
//! The first method of the goal whose `if` holds and that has not failed
//! in this pursuit is followed, a step at a time. A step that takes time
//! starts an action or a wait, the same ones the player could ask for
//! (invariant N2), and the mind goes on when it ends. A step that fails
//! raises `step_failed`, the method is given up and the next one is
//! tried; when none is left the goal fails.

use std::sync::Arc;

use clash_core::{Dur, EntityId, Real, Rng, Tick, Value as Stored};
use clash_data::expr::{Any, Number};
use clash_data::{Cond, Defs, Expr, Runner, Text};
use clash_expr::{DefRef, FunctionId, Handle, Host, PropertyId, RuntimeWarning, Value};
use clash_world::{Cause, CauseKind, Location};

use super::defs::{
    AboutKind, GoalDef, GoalId, MethodDef, MindDef, MindDefId, StanceWritten, Step, StepKind,
};
use super::state::{AboutHeld, Held, MemoryKey, MindState, Outcome, Pending, Stance};
use super::{events, nav, reason as why};
use crate::Sim;
use crate::action::ActionDefId;
use crate::action::def::ActionDef;
use crate::bus::FieldValue;
use crate::entity::Control;
use crate::event::Subject;
use crate::host::{Bound, SimHost};
use crate::perception::belief::{self, Memory};
use crate::perception::{PersonaId, SALIENCE_HALFLIFE_SETTING};
use crate::reason::Reason;

/// The setting that limits how many times one mind thinks in one tick.
pub const MAX_THOUGHTS_SETTING: &str = "ai.max_thoughts_per_tick";
/// The default of [`MAX_THOUGHTS_SETTING`].
pub const DEFAULT_MAX_THOUGHTS: i64 = 4;
/// The setting that limits how many minds think in one batch.
pub const MAX_BATCH_SETTING: &str = "ai.max_minds_per_batch";
/// The default of [`MAX_BATCH_SETTING`].
pub const DEFAULT_MAX_BATCH: i64 = 256;
/// The setting that limits how many steps one thought takes that take
/// no time.
pub const MAX_STEPS_SETTING: &str = "ai.max_steps_per_thought";
/// The default of [`MAX_STEPS_SETTING`].
pub const DEFAULT_MAX_STEPS: i64 = 16;

/// How long a mind that may not think again this tick waits, at least.
pub const DEFER: Dur = Dur::from_millis(1);

/// How much later a `go` step's target may have moved before the step
/// goes after it, as a share of how close is close enough: a target
/// that moved further than this is followed.
pub const RETARGET_M: f64 = 1.5;

/// Seconds a `look` step looks if it does not say.
pub const DEFAULT_LOOK_S: f64 = 2.0;

/// How far from where it stands a step that moves counts as there
/// already, in metres, beyond its `within`.
pub const ARRIVED_SLACK_M: f64 = 0.5;

/// The domain the seed of a mind's own stream is drawn from.
const MIND_STREAM: clash_core::HashDomain = clash_core::HashDomain::new("ai.mind");

// ---- reading -----------------------------------------------------------------

/// The mind definition an entity thinks by: the one `set_mind_def` gave
/// it, else its mind's profile. `None` for an entity with no mind, or
/// whose profile names no `mind` definition.
pub fn mind_def_of(sim: &Sim, entity: EntityId) -> Option<MindDefId> {
    let written = match sim
        .components
        .mind_state
        .get(entity)
        .and_then(MindState::def_override)
    {
        Some(name) => name,
        None => sim.mind(entity)?.profile()?,
    };
    sim.ai_defs.mind_named(written)
}

/// A mind definition by id.
pub fn mind_def(sim: &Sim, id: MindDefId) -> Option<&MindDef> {
    sim.defs.try_registry::<MindDef>()?.try_get(id)
}

/// A goal definition by id.
pub fn goal_def(sim: &Sim, id: GoalId) -> Option<&GoalDef> {
    sim.defs.try_registry::<GoalDef>()?.try_get(id)
}

/// A goal's qualified id.
pub fn goal_name(sim: &Sim, id: GoalId) -> String {
    sim.defs
        .try_registry::<GoalDef>()
        .filter(|registry| id.index() < registry.len())
        .map_or_else(|| id.to_string(), |registry| registry.name(id).to_owned())
}

/// The local time of an entity's clock, or zero for one on no clock.
pub fn local_now(sim: &Sim, entity: EntityId) -> Dur {
    sim.clock_of(entity)
        .and_then(|clock| sim.timeline.local_now(clock).ok())
        .unwrap_or(Dur::ZERO)
}

/// How much a memory matters to a mind now: its salience, halved for
/// every `mind.salience_halflife` since.
pub fn memory_salience(sim: &Sim, mind: EntityId, memory: &Memory) -> Real {
    let halflife = crate::perception::setting_real(sim, SALIENCE_HALFLIFE_SETTING)
        .filter(|value| value.is_finite() && *value > Real::ZERO)
        .unwrap_or(Real::of(belief::DEFAULT_SALIENCE_HALFLIFE_S));
    let age = (local_now(sim, mind) - memory.at).as_secs_real();
    if age <= Real::ZERO {
        return memory.salience;
    }
    memory.salience * Real::of(0.5).powf(age / halflife)
}

/// A mind's weights now: its definition's, with what each `adjust`
/// whose condition holds adds.
pub fn weights(sim: &Sim, mind: EntityId) -> Vec<(String, Real)> {
    let Some(def) = mind_def_of(sim, mind).and_then(|id| mind_def(sim, id)) else {
        return Vec::new();
    };
    let mut out: Vec<(String, Real)> = def
        .weights
        .iter()
        .map(|(name, weight)| (name.clone(), *weight))
        .collect();
    let me = sim.subject_value(Subject::Entity(mind));
    for adjust in &def.adjust {
        if !pure_holds(sim, &adjust.when, &me) {
            continue;
        }
        for (name, by) in &adjust.weights {
            match out.iter_mut().find(|(have, _)| have == name) {
                Some((_, weight)) => *weight += *by,
                None => out.push((name.clone(), *by)),
            }
        }
    }
    out
}

/// Whether a condition on the character itself holds, reading only.
fn pure_holds(sim: &Sim, cond: &Cond, me: &Value) -> bool {
    if let Some(constant) = cond.constant_value() {
        return constant;
    }
    let mut host = sim.host(Bound::new().subject(me.clone()));
    cond.holds_on(&sim.defs, &mut host)
}

/// How a mind stands toward an entity, as it perceives it: the stance it
/// took; else, if the mind has perceived it, what its relations say
/// (15 §9; decision D-762) and the tags the persona it takes the entity
/// for presents: hostile toward one it opposes by its relations (a kind
/// flagged `opposes` held above its default by more than any flagged
/// `sides_with`) or that presents a tag its mind's `hostile_to` names;
/// else feared toward one it holds a kind flagged `avoids` toward, or
/// that presents a tag of `fear_of`. `None` for neither.
///
/// While a mind's goals are weighed, or the conditions of its steps read,
/// an answer once worked out is kept and given again (D-922): the
/// simulation is only read meanwhile, so it is the same answer.
pub fn stance_toward(sim: &Sim, mind: EntityId, other: EntityId) -> Option<Stance> {
    if mind == other {
        return None;
    }
    if let Some(kept) = sim.ai.stances.get(mind, other) {
        return kept;
    }
    let stance = stance_now(sim, mind, other);
    sim.ai.stances.put(mind, other, stance);
    stance
}

/// [`stance_toward`] worked out.
fn stance_now(sim: &Sim, mind: EntityId, other: EntityId) -> Option<Stance> {
    let persona = belief::persona_of(sim, mind, other);
    if let Some(stance) = sim
        .components
        .mind_state
        .get(mind)
        .and_then(|state| state.stance(persona))
    {
        return Some(stance);
    }
    belief::track_of(sim, mind, other)?;
    if crate::social::stance_to(sim, mind, other) == crate::social::Stance::Opposes {
        return Some(Stance::Hostile);
    }
    let def = mind_def_of(sim, mind).and_then(|id| mind_def(sim, id));
    // What the persona it takes the entity for presents (15 §8.1).
    let tags = crate::social::persona_tags(sim, persona);
    let tagged = |names: &[String]| names.iter().any(|tag| tags.contains(tag));
    if def.is_some_and(|def| tagged(&def.hostile_to)) {
        return Some(Stance::Hostile);
    }
    if crate::social::avoids(sim, mind, other) {
        return Some(Stance::Feared);
    }
    let def = def?;
    if def.hostile_to.iter().any(|tag| tags.contains(tag)) {
        Some(Stance::Hostile)
    } else if def.fear_of.iter().any(|tag| tags.contains(tag)) {
        Some(Stance::Feared)
    } else {
        None
    }
}

// ---- evaluating in the mind context -------------------------------------------

/// The simulation as a host for a mind's expressions: it reads as
/// [`SimHost`] does, and draws from the mind's own stream.
struct MindHost<'a, 'r> {
    inner: SimHost<'a>,
    rng: &'r mut Rng,
}

impl Host for MindHost<'_, '_> {
    fn subject(&mut self, slot: u16) -> Value {
        self.inner.subject(slot)
    }

    fn local(&mut self, slot: u16) -> Value {
        self.inner.local(slot)
    }

    fn event_field(&mut self, slot: u16) -> Value {
        self.inner.event_field(slot)
    }

    fn property(&mut self, property: PropertyId, subject: Handle, args: &[Value]) -> Value {
        self.inner.property(property, subject, args)
    }

    fn variable(&mut self, subject: Handle, name: &str) -> Value {
        self.inner.variable(subject, name)
    }

    fn call(&mut self, function: FunctionId, args: &[Value]) -> Value {
        self.inner.call(function, args)
    }

    fn rng(&mut self) -> Option<&mut Rng> {
        Some(self.rng)
    }

    fn warn(&mut self, warning: RuntimeWarning) {
        self.inner.warn(warning);
    }

    fn def_name(&self, id: DefRef) -> Option<&str> {
        self.inner.def_name(id)
    }
}

/// Evaluates a mind's expressions: `self` bound, `$about` or `it` as the
/// scope says, and the mind's own stream to draw from.
struct Eval<'s> {
    sim: &'s Sim,
    defs: Arc<Defs>,
    me: Value,
    runner: Runner,
    rng: Rng,
    warnings: Vec<RuntimeWarning>,
}

impl<'s> Eval<'s> {
    fn new(sim: &'s Sim, mind: EntityId, rng: Rng) -> Eval<'s> {
        Eval {
            sim,
            defs: Arc::clone(&sim.defs),
            me: sim.subject_value(Subject::Entity(mind)),
            runner: Runner::new(),
            rng,
            warnings: Vec::new(),
        }
    }

    fn with_about(&self, about: &Value) -> Bound {
        Bound::new().subject(self.me.clone()).local(about.clone())
    }

    fn with_it(&self, it: &Value) -> Bound {
        Bound::new().subject(self.me.clone()).subject(it.clone())
    }

    fn only_self(&self) -> Bound {
        Bound::new().subject(self.me.clone())
    }

    fn run<T>(&mut self, bound: Bound, work: impl FnOnce(&Defs, &mut MindHost<'_, '_>) -> T) -> T {
        let mut host = MindHost {
            inner: self.sim.host(bound),
            rng: &mut self.rng,
        };
        let out = work(&self.defs, &mut host);
        self.warnings.extend(host.inner.take_warnings());
        out
    }

    fn number(&mut self, expr: &Expr<Number>, bound: Bound) -> Option<Real> {
        if let Some(constant) = expr.constant() {
            return constant.as_number();
        }
        let runner = &mut self.runner;
        let mut host = MindHost {
            inner: self.sim.host(bound),
            rng: &mut self.rng,
        };
        let out = expr.eval(&mut runner.on(&self.defs, &mut host));
        self.warnings.extend(host.inner.take_warnings());
        out.filter(|value| value.is_finite())
    }

    fn value(&mut self, expr: &Expr<Any>, bound: Bound) -> Value {
        if let Some(constant) = expr.constant() {
            return constant.clone();
        }
        let runner = &mut self.runner;
        let mut host = MindHost {
            inner: self.sim.host(bound),
            rng: &mut self.rng,
        };
        let out = expr.value(&mut runner.on(&self.defs, &mut host));
        self.warnings.extend(host.inner.take_warnings());
        out
    }

    fn holds(&mut self, cond: &Cond, bound: Bound) -> bool {
        if let Some(constant) = cond.constant_value() {
            return constant;
        }
        let runner = &mut self.runner;
        let mut host = MindHost {
            inner: self.sim.host(bound),
            rng: &mut self.rng,
        };
        let out = cond.holds(&mut runner.on(&self.defs, &mut host));
        self.warnings.extend(host.inner.take_warnings());
        out
    }

    fn text(&mut self, text: &Text, bound: Bound) -> String {
        self.run(bound, |defs, host| text.render_on(defs, host))
    }
}

// ---- what a goal is about --------------------------------------------------------

/// A candidate for a goal's `about`: what the mind keeps, and the value
/// expressions read.
struct Bound2 {
    held: AboutHeld,
    value: Value,
}

/// The value of a memory as expressions read it (see
/// [`About`](super::defs::About)).
fn memory_value(sim: &Sim, mind: EntityId, memory: &Memory) -> Value {
    let mut map = clash_expr::Map::new();
    let age = (local_now(sim, mind) - memory.at)
        .as_secs_real()
        .max(Real::ZERO);
    map.insert("event".into(), Value::from(memory.event.as_str()));
    map.insert(
        "loc".into(),
        memory
            .loc
            .map_or(Value::Null, |loc| sim.subject_value(Subject::Loc(loc))),
    );
    map.insert("age".into(), Value::Real(age));
    map.insert(
        "salience".into(),
        Value::Real(memory_salience(sim, mind, memory)),
    );
    map.insert("confidence".into(), Value::Real(memory.confidence));
    let actor = memory
        .roles
        .iter()
        .find(|(role, _)| role == "actor")
        .and_then(|(_, persona)| *persona)
        .filter(|persona| sim.entities.contains(persona.entity()));
    map.insert(
        "actor".into(),
        actor.map_or(Value::Null, |persona| {
            sim.subject_value(Subject::Entity(persona.entity()))
        }),
    );
    let how = match memory.how {
        belief::Knowing::Perceived(kind) => sim.perception_defs.kind_name(kind).to_owned(),
        belief::Knowing::Told => "told".to_owned(),
        belief::Knowing::Inferred => "inferred".to_owned(),
        belief::Knowing::Carried => "carried".to_owned(),
    };
    map.insert("how".into(), Value::from(how.as_str()));
    Value::from(map)
}

fn memory_key(memory: &Memory) -> MemoryKey {
    MemoryKey {
        event: memory.event.clone(),
        loc: memory.loc,
        at: memory.at,
    }
}

/// The value a held `about` has now, if what it names is still believed.
fn held_value(sim: &Sim, mind: EntityId, about: &AboutHeld) -> Option<Value> {
    match about {
        AboutHeld::None => Some(Value::Null),
        AboutHeld::Track(persona) => {
            let entity = persona.entity();
            (sim.entities.contains(entity) && belief::track_of(sim, mind, entity).is_some())
                .then(|| sim.subject_value(Subject::Entity(entity)))
        }
        AboutHeld::Memory(key) => {
            let beliefs = sim.components.beliefs.get(mind)?;
            let memory = beliefs
                .memories()
                .iter()
                .find(|memory| memory_key(memory) == *key)?;
            Some(memory_value(sim, mind, memory))
        }
        AboutHeld::Place(loc) => Some(sim.subject_value(Subject::Loc(*loc))),
    }
}

/// Picks a goal's `about` (see [`About`](super::defs::About)). With
/// `prefer`, what the goal is already about is kept while it is still a
/// candidate. `None` if the goal has an `about` and nothing fits.
fn bind_about(
    ev: &mut Eval<'_>,
    mind: EntityId,
    state: &MindState,
    goal: &GoalDef,
    prefer: Option<&AboutHeld>,
) -> Option<Bound2> {
    let sim = ev.sim;
    match goal.about.kind() {
        AboutKind::None => Some(Bound2 {
            held: AboutHeld::None,
            value: Value::Null,
        }),
        AboutKind::Place => {
            let expr = goal.about.place.as_ref()?;
            let bound = ev.only_self();
            let value = ev.value(expr, bound);
            match sim.subject_of(&value) {
                Some(Subject::Loc(loc)) => Some(Bound2 {
                    held: AboutHeld::Place(loc),
                    value,
                }),
                _ => None,
            }
        }
        AboutKind::Track => {
            let pick = goal.about.track.as_ref()?;
            let beliefs = sim.components.beliefs.get(mind)?;
            let mut best: Option<(Real, Bound2)> = None;
            for track in beliefs.tracks() {
                let persona = track.persona();
                let entity = persona.entity();
                if entity == mind || !sim.entities.contains(entity) {
                    continue;
                }
                let settled = state
                    .settled_tracks
                    .iter()
                    .any(|(have, at)| *have == persona && *at == track.at());
                if settled {
                    continue;
                }
                let it = sim.subject_value(Subject::Entity(entity));
                if let Some(filter) = &pick.filter
                    && !ev.holds(filter, ev.with_it(&it))
                {
                    continue;
                }
                let candidate = Bound2 {
                    held: AboutHeld::Track(persona),
                    value: it.clone(),
                };
                if prefer == Some(&candidate.held) {
                    return Some(candidate);
                }
                let score = match &pick.best {
                    Some(best) => ev.number(best, ev.with_it(&it)).unwrap_or(Real::ZERO),
                    None => Real::ZERO,
                };
                if best.as_ref().is_none_or(|(have, _)| score > *have) {
                    best = Some((score, candidate));
                }
            }
            best.map(|(_, found)| found)
        }
        AboutKind::Memory => {
            let pick = goal.about.memory.as_ref()?;
            let beliefs = sim.components.beliefs.get(mind)?;
            let mut best: Option<(Real, Bound2)> = None;
            for memory in beliefs.memories() {
                if !pick.events.is_empty() && !pick.events.contains(&memory.event) {
                    continue;
                }
                let key = memory_key(memory);
                if state.settled_memories.contains(&key) {
                    continue;
                }
                let it = memory_value(sim, mind, memory);
                if let Some(filter) = &pick.filter
                    && !ev.holds(filter, ev.with_it(&it))
                {
                    continue;
                }
                let candidate = Bound2 {
                    held: AboutHeld::Memory(key),
                    value: it.clone(),
                };
                if prefer == Some(&candidate.held) {
                    return Some(candidate);
                }
                // Left out, the most recent: memories are oldest first,
                // and a later one at the same score wins.
                let score = match &pick.best {
                    Some(best) => ev.number(best, ev.with_it(&it)).unwrap_or(Real::ZERO),
                    None => Real::ZERO,
                };
                let better = match &best {
                    None => true,
                    Some((have, _)) if pick.best.is_none() => score >= *have,
                    Some((have, _)) => score > *have,
                };
                if better {
                    best = Some((score, candidate));
                }
            }
            best.map(|(_, found)| found)
        }
    }
}

// ---- choosing ----------------------------------------------------------------------

/// What a thought decided about goals.
enum Choice {
    /// Go on with the goal held.
    Keep,
    /// Take up this goal.
    Adopt {
        goal: GoalId,
        about: AboutHeld,
        score: Real,
    },
    /// Hold no goal.
    Drop,
}

/// What became of the goal held.
enum HeldStatus {
    /// It goes on, scoring this.
    Goes(Real),
    /// Its `done` holds.
    Done,
    /// Its `fail` holds.
    Failed,
    /// What it is about is no longer believed, or it is no longer one of
    /// the mind's goals.
    Gone,
}

/// The goals a mind may have now: its own, then those of each `adjust`
/// whose condition holds, each once.
fn goal_list(ev: &mut Eval<'_>, def: &MindDef, mind: EntityId) -> Vec<GoalId> {
    let mut out: Vec<GoalId> = def.goals.iter().map(|goal| goal.id()).collect();
    for adjust in &def.adjust {
        let bound = ev.only_self();
        if !ev.holds(&adjust.when, bound) {
            continue;
        }
        for goal in &adjust.goals {
            if !out.contains(&goal.id()) {
                out.push(goal.id());
            }
        }
    }
    // The goal its routine names for this part of the day (09 §10.2).
    if let Some(goal) = crate::agent::routine_goal(ev.sim, mind)
        && !out.contains(&goal)
    {
        out.push(goal);
    }
    out
}

/// Scores the mind's goals and decides what to pursue (09 §3.3).
fn choose(
    ev: &mut Eval<'_>,
    mind: EntityId,
    state: &MindState,
    def: &MindDef,
    now: Tick,
) -> (Choice, Option<HeldStatus>) {
    let sim = ev.sim;
    let held = state.held.as_ref();
    let mut best: Option<(Real, GoalId, AboutHeld)> = None;
    let mut status: Option<HeldStatus> = held.map(|_| HeldStatus::Gone);
    for goal_id in goal_list(ev, def, mind) {
        let Some(goal) = goal_def(sim, goal_id) else {
            continue;
        };
        let is_held = held.is_some_and(|held| held.goal == goal_id);
        let penalised = state
            .penalties
            .iter()
            .any(|(have, until)| *have == goal_id && *until > now);
        if penalised && !is_held {
            continue;
        }
        let prefer = held.filter(|_| is_held).map(|held| &held.about);
        let Some(about) = bind_about(ev, mind, state, goal, prefer) else {
            continue;
        };
        let bound = ev.with_about(&about.value);
        let mut score = ev.number(&goal.score, bound).unwrap_or(Real::ZERO);
        if goal.noise > Real::ZERO {
            score += goal.noise * ev.rng.unit();
        }
        if is_held {
            let bound = ev.with_about(&about.value);
            let done = goal.done.as_ref().is_some_and(|done| ev.holds(done, bound));
            let bound = ev.with_about(&about.value);
            let failed = goal.fail.as_ref().is_some_and(|fail| ev.holds(fail, bound));
            status = Some(if done {
                HeldStatus::Done
            } else if failed {
                HeldStatus::Failed
            } else if prefer != Some(&about.held) {
                // What it was about is gone; the goal may start again
                // about something else.
                HeldStatus::Gone
            } else {
                HeldStatus::Goes(score)
            });
            if done || failed {
                continue;
            }
        }
        if score > Real::ZERO && best.as_ref().is_none_or(|(have, _, _)| score > *have) {
            best = Some((score, goal_id, about.held));
        }
    }
    let choice = match (held, &status) {
        (Some(held), Some(HeldStatus::Goes(score))) if held.forced || *score > Real::ZERO => {
            let commit = goal_def(sim, held.goal).map_or(Real::ZERO, |goal| goal.commit);
            match best {
                Some((best_score, goal, about))
                    if !held.forced
                        && goal != held.goal
                        && best_score > *score * (Real::ONE + commit) =>
                {
                    Choice::Adopt {
                        goal,
                        about,
                        score: best_score,
                    }
                }
                _ => Choice::Keep,
            }
        }
        (Some(held), Some(HeldStatus::Gone)) if held.forced => Choice::Keep,
        _ => match best {
            Some((score, goal, about)) => Choice::Adopt { goal, about, score },
            None => Choice::Drop,
        },
    };
    (choice, status)
}

// ---- the batch -------------------------------------------------------------------

/// Prompts a mind to think at the next batch (see the [module](self)).
/// Nothing is done for an entity whose mind names no `mind` definition.
pub fn prompt(sim: &mut Sim, entity: EntityId) {
    if mind_def_of(sim, entity).is_some() {
        sim.ai.due.insert(entity);
    }
}

/// Thinks for every mind prompted, and every mind whose interval has
/// passed, in order of entity (see the [module](self)); first rechecks
/// routes in use that the world changed under. Returns whether anything
/// was done, so that the events it raised are delivered.
pub fn think_due(sim: &mut Sim) -> bool {
    let routes = !sim.ai.changed.is_empty() || sim.ai.all_changed;
    if routes {
        nav::recheck_routes(sim);
    }
    let now = sim.now();
    while let Some(&(at, entity)) = sim.ai.schedule.first() {
        if at > now {
            break;
        }
        sim.ai.schedule.pop_first();
        if let Some(state) = sim.components.mind_state.get_mut(entity)
            && state.next_think == Some(at)
        {
            state.next_think = None;
            sim.ai.due.insert(entity);
        }
    }
    if sim.ai.due.is_empty() {
        return routes;
    }
    let limit = sim.setting_int(MAX_BATCH_SETTING, DEFAULT_MAX_BATCH).max(1) as usize;
    let mut batch = Vec::new();
    while batch.len() < limit {
        let Some(entity) = sim.ai.due.pop_first() else {
            break;
        };
        // A warm mind thinks at the warm stepper's next step (09 §9.1).
        if super::warm::defer(sim, entity) {
            continue;
        }
        batch.push(entity);
    }
    for entity in batch {
        think(sim, entity);
    }
    true
}

/// Thinks for some minds in the order given, as one batch would: for
/// tests of invariant N8 and for tools. Each is taken off the minds due.
pub fn think_in_order(sim: &mut Sim, minds: &[EntityId]) {
    for &mind in minds {
        sim.ai.due.remove(&mind);
        think(sim, mind);
    }
}

/// The cause of what a mind does.
pub(super) fn cause_of(mind: EntityId) -> Cause {
    Cause {
        actor: Some(mind),
        instigator: Some(mind),
        kind: CauseKind::Action,
    }
}

/// A fraction from 0 to 1 that is the same for an entity in every run:
/// what staggers minds made together.
fn stagger(entity: EntityId) -> Real {
    let bits = entity.to_bits();
    let hash = clash_core::pos_hash(0, MIND_STREAM, bits as i32, (bits >> 32) as i32, 0, 1);
    Real::from_i64((hash >> 11) as i64) / Real::from_i64(1_i64 << 53)
}

/// Makes the state of a mind that has none, by a definition and, where
/// one is named for it, under that name; and gives a character with
/// nothing to perceive with the senses the definition names. Every way
/// a mind's state is first made comes here, so that a mind given a
/// stance, a goal or a home before it first thinks perceives as any
/// other does (decision D-901).
pub(super) fn begin_state(sim: &mut Sim, mind: EntityId, def: &MindDef, named: Option<&str>) {
    if sim.components.mind_state.contains(mind) {
        return;
    }
    let mut state = new_state(sim, mind, def);
    state.def = named.map(str::to_owned);
    sim.components.mind_state.insert(mind, state);
    if !sim.components.senses.contains(mind) {
        for name in &def.senses {
            if let Some(sense) = sim.perception_defs.sense_named(name) {
                let _ = crate::perception::attach_sense(sim, mind, sense);
            }
        }
    }
}

/// The state of a mind that thinks for the first time.
fn new_state(sim: &Sim, mind: EntityId, def: &MindDef) -> MindState {
    let bits = mind.to_bits();
    let seed = clash_core::pos_hash(
        sim.rng.seed(),
        MIND_STREAM,
        bits as i32,
        (bits >> 32) as i32,
        0,
        0,
    );
    MindState::new(
        Rng::from_seed(seed),
        sim.entities.location(mind),
        def.think.dur(),
    )
}

/// Whether an entity's mind may decide for it now: it is in the world,
/// awake, and nobody controls it directly.
fn may_decide(sim: &Sim, mind: EntityId) -> bool {
    let Some(record) = sim.entities.get(mind) else {
        return false;
    };
    record.control() != Control::Direct
        && sim.entities.location(mind).is_some()
        && sim.mind(mind).is_some_and(crate::mind::Mind::is_active)
}

/// One thought of one mind (see the [module](self)).
pub(super) fn think(sim: &mut Sim, mind: EntityId) {
    let Some(def_id) = mind_def_of(sim, mind) else {
        return;
    };
    if !may_decide(sim, mind) {
        return;
    }
    // Levels (09 §9): the stepper starts when it may be needed; a warm
    // mind that must open a door is promoted, a warm one starting no
    // action; one hot again goes on with the step it walked in bulk.
    super::warm::notice(sim);
    if super::warm::is_warm(sim, mind) && door_met(sim, mind) {
        super::warm::promote(sim, mind, super::warm::Why::Acting);
        sim.ai.due.remove(&mind);
    }
    if !super::warm::is_warm(sim, mind) && super::warm::is_walking(sim, mind) {
        resume_moving(sim, mind);
    }
    let defs = Arc::clone(&sim.defs);
    let Some(def) = defs
        .try_registry::<MindDef>()
        .and_then(|r| r.try_get(def_id))
    else {
        return;
    };
    let now = sim.now();
    begin_state(sim, mind, def, None);
    let max = sim
        .setting_int(MAX_THOUGHTS_SETTING, DEFAULT_MAX_THOUGHTS)
        .max(1) as u32;
    {
        let Some(state) = sim.components.mind_state.get_mut(mind) else {
            return;
        };
        if state.thought_at != now {
            state.thought_at = now;
            state.thoughts = 0;
        }
        if state.thoughts >= max {
            schedule(sim, mind, now + DEFER);
            return;
        }
        state.thoughts += 1;
        state.penalties.retain(|(_, until)| *until > now);
    }

    // What came of the step under way, now that it may have ended.
    settle_step(sim, mind);

    // Choose, reading only.
    let Some(state) = sim.components.mind_state.get(mind).cloned() else {
        return;
    };
    // While goals are weighed the simulation is only read: how the mind
    // stands toward others is worked out once (D-922).
    sim.ai.stances.begin();
    let mut ev = Eval::new(sim, mind, state.rng.clone());
    let (choice, status) = choose(&mut ev, mind, &state, def, now);
    let (rng, warnings) = (ev.rng, ev.warnings);
    sim.ai.stances.end();
    if let Some(state) = sim.components.mind_state.get_mut(mind) {
        state.rng = rng;
    }
    for warning in warnings {
        sim.report_warning(warning);
    }

    let mut changed = false;
    if let (Some(held), Some(status)) = (&state.held, status) {
        match status {
            HeldStatus::Done => {
                events::goal_done(sim, mind, held.goal);
                set_held(sim, mind, None);
                changed = true;
            }
            HeldStatus::Failed => {
                let reason = Reason::new(why::GOAL_FAILED, "the goal's `fail` held")
                    .with("goal", goal_name(sim, held.goal).as_str());
                fail_goal(sim, mind, held.goal, reason);
                changed = true;
            }
            HeldStatus::Gone | HeldStatus::Goes(_) => {}
        }
    }
    match choice {
        Choice::Keep => {}
        Choice::Adopt { goal, about, score } => {
            adopt(sim, mind, goal, about, score, false);
            changed = true;
        }
        Choice::Drop => {
            if sim
                .components
                .mind_state
                .get(mind)
                .is_some_and(|state| state.held.is_some())
            {
                set_held(sim, mind, None);
                changed = true;
            }
        }
    }
    let progressed = pursue(sim, mind);
    rest(sim, mind, def, changed || progressed);
}

/// Sets the time a mind next thinks with nothing to prompt it.
fn schedule(sim: &mut Sim, mind: EntityId, at: Tick) {
    let Some(state) = sim.components.mind_state.get_mut(mind) else {
        return;
    };
    if let Some(old) = state.next_think.replace(at) {
        sim.ai.schedule.remove(&(old, mind));
    }
    sim.ai.schedule.insert((at, mind));
}

/// What a mind does after a thought: one that is free and has nothing
/// to do waits its interval; one that is busy thinks again when its
/// interval has passed. A thought that changed nothing stretches the
/// interval, up to `stretch` times the mind's `think`.
fn rest(sim: &mut Sim, mind: EntityId, def: &MindDef, changed: bool) {
    let base = def.think.dur();
    let most = Dur::from_micros(base.as_micros().saturating_mul(def.stretch.max(1)));
    let interval = {
        let Some(state) = sim.components.mind_state.get_mut(mind) else {
            return;
        };
        state.interval = if changed {
            base
        } else {
            Dur::from_micros(state.interval.as_micros().saturating_mul(2)).min(most)
        };
        state.interval
    };
    // Staggered: a quarter of an interval at most, the same for an
    // entity every time.
    let extra = Dur::from_secs_real(interval.as_secs_real() * stagger(mind) / Real::of(4.0));
    let wait = interval + extra;
    if sim.is_free(mind) && sim.actions.wait_of(mind).is_none() {
        let _ = sim.wait(mind, wait, cause_of(mind));
        let rest = sim.actions.wait_of(mind).and_then(|wait| wait.wake);
        if let Some(state) = sim.components.mind_state.get_mut(mind) {
            state.rest = rest;
        }
    }
    let rate = sim
        .clock_of(mind)
        .and_then(|clock| sim.timeline.effective_rate(clock))
        .unwrap_or(Real::ONE);
    if rate > Real::ZERO {
        let universal = Dur::from_secs_real(wait.as_secs_real() / rate);
        let at = sim.now() + universal.max(DEFER);
        schedule(sim, mind, at);
    }
}

/// Replaces the goal a mind holds.
fn set_held(sim: &mut Sim, mind: EntityId, held: Option<Held>) {
    if let Some(state) = sim.components.mind_state.get_mut(mind) {
        state.held = held;
    }
}

/// A mind takes up a goal: `goal_adopted` is raised, and if the goal
/// interrupts, what the character is doing is cancelled where it can be.
pub(super) fn adopt(
    sim: &mut Sim,
    mind: EntityId,
    goal: GoalId,
    about: AboutHeld,
    score: Real,
    forced: bool,
) {
    let previous = sim
        .components
        .mind_state
        .get(mind)
        .and_then(|state| state.held.as_ref())
        .map(|held| held.goal);
    // What the mind itself set going: the action of the step under way,
    // and a wait of a step or a rest.
    let (own_action, own_wait) =
        sim.components
            .mind_state
            .get(mind)
            .map_or((None, None), |state| {
                let pending = state.held.as_ref().and_then(|held| held.pending.as_ref());
                let action = pending.and_then(|pending| pending.action);
                let waits = pending.is_some_and(|pending| {
                    pending.action.is_none()
                        && matches!(
                            pending.kind(),
                            StepKind::Wait | StepKind::Look | StepKind::KeepNear
                        )
                });
                let current = sim.actions.wait_of(mind).and_then(|wait| wait.wake);
                let wait = current.filter(|wake| waits || state.rest == Some(*wake));
                (action, wait)
            });
    set_held(
        sim,
        mind,
        Some(Held {
            goal,
            about: about.clone(),
            score,
            forced,
            method: None,
            failed: Vec::new(),
            step: 0,
            pending: None,
        }),
    );
    let value = held_value(sim, mind, &about).unwrap_or(Value::Null);
    events::goal_adopted(sim, mind, goal, &value, score, previous);
    let interrupts = forced || goal_def(sim, goal).is_some_and(|goal| goal.interrupts);
    if interrupts {
        stop_doing(sim, mind, own_action, own_wait.is_some());
    }
}

/// Cancels what the mind itself set going, where the action's phase
/// allows it, and ends its own wait. What a command or a rule set the
/// character doing runs on: the goal is pursued when it ends (09 §4:
/// reactions written as rules and goals coexist).
fn stop_doing(sim: &mut Sim, mind: EntityId, action: Option<crate::later::ActionId>, wait: bool) {
    let cause = cause_of(mind);
    if let Some(id) = action
        && sim.actions.get(id).is_some()
    {
        // An action in a phase that cannot be cancelled runs on.
        let _ = sim.cancel_action(mind, id, true, cause);
    }
    if wait {
        let _ = sim.wait(mind, Dur::ZERO, cause);
    }
}

/// A goal failed: `goal_failed` is raised and it is set aside for its
/// `retry_after`.
fn fail_goal(sim: &mut Sim, mind: EntityId, goal: GoalId, reason: Reason) {
    let until = goal_def(sim, goal).map_or(sim.now(), |def| {
        let rate = sim
            .clock_of(mind)
            .and_then(|clock| sim.timeline.effective_rate(clock))
            .filter(|rate| *rate > Real::ZERO)
            .unwrap_or(Real::ONE);
        sim.now() + Dur::from_secs_real(def.retry_after.dur().as_secs_real() / rate)
    });
    events::goal_failed(sim, mind, goal, &reason);
    if let Some(state) = sim.components.mind_state.get_mut(mind) {
        state.penalties.retain(|(have, _)| *have != goal);
        state.penalties.push((goal, until));
        state.held = None;
        state.last_failure = Some(reason);
    }
}

// ---- pursuing ---------------------------------------------------------------------

/// The steps of a method of a goal, and its `if`.
fn method_of<'d>(
    defs: &'d Defs,
    goal: &'d GoalDef,
    at: usize,
) -> Option<(Option<&'d Cond>, &'d [Step])> {
    let entry = goal.methods.get(at)?;
    match entry.method {
        Some(method) => {
            let def = defs.try_registry::<MethodDef>()?.try_get(method.id())?;
            // Both conditions must hold: the entry's and the method's.
            let when = entry.when.as_ref().or(def.when.as_ref());
            Some((when, &def.steps))
        }
        None => Some((entry.when.as_ref(), &entry.steps)),
    }
}

/// Whether both an entry's own `if` and its method's hold.
fn method_allowed(
    ev: &mut Eval<'_>,
    defs: &Defs,
    goal: &GoalDef,
    at: usize,
    about: &Value,
) -> bool {
    let Some(entry) = goal.methods.get(at) else {
        return false;
    };
    if let Some(when) = &entry.when
        && !ev.holds(when, ev.with_about(about))
    {
        return false;
    }
    if let Some(method) = entry.method
        && let Some(def) = defs
            .try_registry::<MethodDef>()
            .and_then(|r| r.try_get(method.id()))
        && let Some(when) = &def.when
        && !ev.holds(when, ev.with_about(about))
    {
        return false;
    }
    true
}

/// Looks at the step under way. One whose action or wait has ended is
/// done or failed; one that follows someone who has moved is started
/// again toward where they now are; one that has come close enough
/// stops. Returns nothing: what it decided is in the pending step.
fn settle_step(sim: &mut Sim, mind: EntityId) {
    let Some(pending) = sim
        .components
        .mind_state
        .get(mind)
        .and_then(|state| state.held.as_ref())
        .and_then(|held| held.pending.clone())
    else {
        return;
    };
    if pending.outcome != Outcome::Running {
        return;
    }
    let outcome = match pending.kind() {
        // A closed way met on the step: opening it comes first.
        StepKind::Go | StepKind::KeepNear | StepKind::KeepFrom
            if let Some(door) = super::doors::settle(sim, mind) =>
        {
            door
        }
        StepKind::Go | StepKind::KeepNear | StepKind::KeepFrom => {
            let arrived = pending.target.is_some_and(|target| {
                nav::distance_to(sim, mind, target)
                    .is_some_and(|d| d <= pending.within + Real::of(ARRIVED_SLACK_M))
            });
            let acting = pending
                .action
                .is_some_and(|id| sim.actions.get(id).is_some());
            // A warm mind walking the step in bulk is under way too.
            let running = acting || super::warm::is_walking(sim, mind);
            if arrived {
                if acting && let Some(id) = pending.action {
                    let _ = sim.cancel_action(mind, id, true, cause_of(mind));
                }
                super::warm::stop_walk(sim, mind);
                Some(Outcome::Done)
            } else if running {
                None
            } else {
                Some(Outcome::Failed(
                    Reason::new(why::NOT_ARRIVED, "it did not get there")
                        .with("entity", mind.to_string()),
                ))
            }
        }
        StepKind::Do | StepKind::Choose => {
            let running = pending
                .action
                .is_some_and(|id| sim.actions.get(id).is_some());
            (!running).then_some(Outcome::Done)
        }
        _ => {
            let waiting = sim.actions.wait_of(mind).is_some_and(|wait| !wait.idle);
            (!waiting).then_some(Outcome::Done)
        }
    };
    if let Some(outcome) = outcome
        && let Some(pending) = sim
            .components
            .mind_state
            .get_mut(mind)
            .and_then(|state| state.held.as_mut())
            .and_then(|held| held.pending.as_mut())
        && pending.outcome == Outcome::Running
    {
        pending.outcome = outcome;
    }
}

/// What a step came to when it was begun.
enum Begun {
    /// It is over: go on to the next.
    Done,
    /// It takes time: wait for it.
    Under(Pending),
    /// It could not be done.
    Failed(Reason),
}

/// Pursues the goal held: the step under way, then the next steps, as
/// far as steps that take no time go. Returns whether anything changed.
fn pursue(sim: &mut Sim, mind: EntityId) -> bool {
    let max_steps = sim.setting_int(MAX_STEPS_SETTING, DEFAULT_MAX_STEPS).max(1) as usize;
    let defs = Arc::clone(&sim.defs);
    let mut changed = false;
    for _ in 0..max_steps {
        let Some((held, rng, rest)) = sim
            .components
            .mind_state
            .get(mind)
            .and_then(|state| Some((state.held.clone()?, state.rng.clone(), state.rest)))
        else {
            return changed;
        };
        let Some(goal) = defs
            .try_registry::<GoalDef>()
            .and_then(|r| r.try_get(held.goal))
        else {
            set_held(sim, mind, None);
            return true;
        };
        let about = held_value(sim, mind, &held.about).unwrap_or(Value::Null);

        // The step under way.
        if let Some(pending) = &held.pending {
            match &pending.outcome {
                Outcome::Running => {
                    if let Some(tries) = super::doors::opened(sim, mind) {
                        restart(sim, mind, goal, &held, &about, tries);
                        changed = true;
                        continue;
                    }
                    if retarget(sim, mind, goal, &held, pending, &about) {
                        changed = true;
                        continue;
                    }
                    if until_holds(sim, mind, goal, &held, &about) {
                        let _ = sim.wait(mind, Dur::ZERO, cause_of(mind));
                        next_step(sim, mind);
                        changed = true;
                        continue;
                    }
                    return changed;
                }
                Outcome::Done if pending.kind() == StepKind::KeepNear => {
                    // Staying near never ends by itself: it is begun
                    // again, toward where they are now.
                    if let Some(held) = sim
                        .components
                        .mind_state
                        .get_mut(mind)
                        .and_then(|state| state.held.as_mut())
                    {
                        held.pending = None;
                    }
                }
                Outcome::Done => {
                    next_step(sim, mind);
                    changed = true;
                    continue;
                }
                Outcome::Failed(reason) => {
                    let reason = reason.clone();
                    step_failed(sim, mind, &held, reason);
                    changed = true;
                    continue;
                }
            }
        }

        // Nothing under way: a rest the mind took ends, and the
        // character must be free to be given the next step.
        if let Some(rest) = rest
            && sim.actions.wait_of(mind).and_then(|wait| wait.wake) == Some(rest)
        {
            let _ = sim.wait(mind, Dur::ZERO, cause_of(mind));
            if let Some(state) = sim.components.mind_state.get_mut(mind) {
                state.rest = None;
            }
        }
        if !sim.is_free(mind) && sim.actions.wait_of(mind).is_none_or(|wait| !wait.idle) {
            return changed;
        }

        // The method.
        let method = match held.method {
            Some(method) => usize::from(method),
            None => {
                let mut ev = Eval::new(sim, mind, rng.clone());
                let found = (0..goal.methods.len()).find(|at| {
                    !held.failed.contains(&(*at as u16))
                        && method_allowed(&mut ev, &defs, goal, *at, &about)
                });
                let rng = ev.rng;
                if let Some(state) = sim.components.mind_state.get_mut(mind) {
                    state.rng = rng;
                }
                let Some(found) = found else {
                    let reason = Reason::new(why::NO_METHOD, "no method of the goal can be used")
                        .with("goal", goal_name(sim, held.goal).as_str());
                    fail_goal(sim, mind, held.goal, reason);
                    return true;
                };
                if let Some(held) = sim
                    .components
                    .mind_state
                    .get_mut(mind)
                    .and_then(|state| state.held.as_mut())
                {
                    held.method = Some(found as u16);
                    held.step = 0;
                }
                events::method_chosen(sim, mind, held.goal, found);
                found
            }
        };
        let Some((_, steps)) = method_of(&defs, goal, method) else {
            set_held(sim, mind, None);
            return true;
        };
        let Some(step) = steps.get(usize::from(held.step)) else {
            // The method is over. The goal is done if its `done` holds;
            // else the next thought takes it up again, after a rest.
            let done = goal.done.as_ref().is_some_and(|done| {
                let mut ev = Eval::new(sim, mind, rng.clone());
                ev.holds(done, ev.with_about(&about))
            });
            if done {
                events::goal_done(sim, mind, held.goal);
                set_held(sim, mind, None);
            } else if let Some(held) = sim
                .components
                .mind_state
                .get_mut(mind)
                .and_then(|state| state.held.as_mut())
            {
                held.method = None;
                held.step = 0;
                held.failed.clear();
            }
            return true;
        };
        changed = true;
        let skip = step.when.as_ref().is_some_and(|when| {
            let mut ev = Eval::new(sim, mind, rng.clone());
            !ev.holds(when, ev.with_about(&about))
        });
        if skip {
            next_step(sim, mind);
            continue;
        }
        events::step_started(sim, mind, held.goal, step.kind());
        match begin(sim, mind, step, &held, &about) {
            Begun::Done => next_step(sim, mind),
            Begun::Under(pending) => {
                if let Some(held) = sim
                    .components
                    .mind_state
                    .get_mut(mind)
                    .and_then(|state| state.held.as_mut())
                {
                    held.pending = Some(pending);
                }
                return true;
            }
            Begun::Failed(reason) => {
                let held = sim
                    .components
                    .mind_state
                    .get(mind)
                    .and_then(|state| state.held.clone());
                if let Some(held) = held {
                    step_failed(sim, mind, &held, reason);
                }
            }
        }
    }
    // Steps that take no time ran out the allowance: the rest wait for
    // the next thought.
    changed
}

/// Goes on to the next step of the method in hand.
fn next_step(sim: &mut Sim, mind: EntityId) {
    if let Some(held) = sim
        .components
        .mind_state
        .get_mut(mind)
        .and_then(|state| state.held.as_mut())
    {
        held.step = held.step.saturating_add(1);
        held.pending = None;
    }
}

/// A step failed: `step_failed` is raised and the method is given up.
fn step_failed(sim: &mut Sim, mind: EntityId, held: &Held, reason: Reason) {
    let kind = held
        .pending
        .as_ref()
        .map(Pending::kind)
        .or_else(|| current_step_kind(sim, held));
    events::step_failed(sim, mind, held.goal, kind, &reason);
    if let Some(state) = sim.components.mind_state.get_mut(mind) {
        state.last_failure = Some(reason);
        if let Some(held) = state.held.as_mut() {
            if let Some(method) = held.method.take() {
                held.failed.push(method);
            }
            held.step = 0;
            held.pending = None;
        }
    }
}

/// The kind of the step a held goal is at.
pub(super) fn current_step_kind(sim: &Sim, held: &Held) -> Option<StepKind> {
    let goal = goal_def(sim, held.goal)?;
    let (_, steps) = method_of(&sim.defs, goal, usize::from(held.method?))?;
    steps.get(usize::from(held.step)).map(Step::kind)
}

/// The step a held goal is at.
fn current_step<'d>(defs: &'d Defs, goal: &'d GoalDef, held: &Held) -> Option<&'d Step> {
    let (_, steps) = method_of(defs, goal, usize::from(held.method?))?;
    steps.get(usize::from(held.step))
}

/// For a `wait` step with `until`: whether it holds.
fn until_holds(sim: &mut Sim, mind: EntityId, goal: &GoalDef, held: &Held, about: &Value) -> bool {
    let defs = Arc::clone(&sim.defs);
    let Some(until) = current_step(&defs, goal, held)
        .and_then(|step| step.wait.as_ref())
        .and_then(|wait| wait.until.as_ref())
    else {
        return false;
    };
    let Some(rng) = sim
        .components
        .mind_state
        .get(mind)
        .map(|state| state.rng.clone())
    else {
        return false;
    };
    let mut ev = Eval::new(sim, mind, rng);
    let holds = ev.holds(until, ev.with_about(about));
    let rng = ev.rng;
    if let Some(state) = sim.components.mind_state.get_mut(mind) {
        state.rng = rng;
    }
    holds
}

/// For a step that moves toward someone: if they are now believed to be
/// further from where the step is going than it allows, the step is
/// begun again toward where they are. Returns whether it was.
fn retarget(
    sim: &mut Sim,
    mind: EntityId,
    goal: &GoalDef,
    held: &Held,
    pending: &Pending,
    about: &Value,
) -> bool {
    if !matches!(pending.kind(), StepKind::Go | StepKind::KeepNear) {
        return false;
    }
    let (Some(whom), Some(target)) = (pending.whom, pending.target) else {
        return false;
    };
    let Some(now_at) = nav::believed_place(sim, mind, whom) else {
        return false;
    };
    let moved = sim
        .cosmos
        .delta(target, now_at)
        .map_or(Real::INFINITY, nav::metres);
    if moved <= pending.within.max(Real::of(RETARGET_M)) {
        return false;
    }
    let defs = Arc::clone(&sim.defs);
    let Some(step) = current_step(&defs, goal, held) else {
        return false;
    };
    if let Some(id) = pending.action {
        let _ = sim.cancel_action(mind, id, true, cause_of(mind));
    }
    match begin(sim, mind, step, held, about) {
        Begun::Under(pending) => {
            if let Some(held) = sim
                .components
                .mind_state
                .get_mut(mind)
                .and_then(|state| state.held.as_mut())
            {
                held.pending = Some(pending);
            }
        }
        Begun::Done => next_step(sim, mind),
        Begun::Failed(reason) => {
            if let Some(held) = sim
                .components
                .mind_state
                .get(mind)
                .and_then(|state| state.held.clone())
            {
                step_failed(sim, mind, &held, reason);
            }
        }
    }
    true
}

/// Begins the step in hand again, once the door it met is open, keeping
/// count of the doors it has opened.
fn restart(sim: &mut Sim, mind: EntityId, goal: &GoalDef, held: &Held, about: &Value, tries: u8) {
    let defs = Arc::clone(&sim.defs);
    let Some(step) = current_step(&defs, goal, held) else {
        next_step(sim, mind);
        return;
    };
    match begin(sim, mind, step, held, about) {
        Begun::Under(mut pending) => {
            pending.door = super::state::Door::Walking { tries };
            if let Some(held) = sim
                .components
                .mind_state
                .get_mut(mind)
                .and_then(|state| state.held.as_mut())
            {
                held.pending = Some(pending);
            }
        }
        Begun::Done => next_step(sim, mind),
        Begun::Failed(reason) => {
            if let Some(held) = sim
                .components
                .mind_state
                .get(mind)
                .and_then(|state| state.held.clone())
            {
                step_failed(sim, mind, &held, reason);
            }
        }
    }
}

// ---- the steps ----------------------------------------------------------------------

/// What a value names as a place: a cell, an element's cell, or where
/// an entity is believed to be; and the entity, if it named one.
fn place_of(sim: &Sim, mind: EntityId, value: &Value) -> Option<(Location, Option<EntityId>)> {
    match sim.subject_of(value)? {
        Subject::Loc(loc) | Subject::Element(loc, _) => Some((loc, None)),
        Subject::Entity(entity) => Some((nav::believed_place(sim, mind, entity)?, Some(entity))),
        _ => None,
    }
}

fn no_target(step: StepKind) -> Reason {
    Reason::new(
        why::NO_TARGET,
        format!(
            "the `{}` step's target names no place it knows of",
            step.name()
        ),
    )
    .with("step", step.name())
}

/// Evaluates what a step needs with the mind's own stream, keeping what
/// it drew.
fn with_eval<T>(sim: &mut Sim, mind: EntityId, work: impl FnOnce(&mut Eval<'_>) -> T) -> T {
    let rng = sim
        .components
        .mind_state
        .get(mind)
        .map(|state| state.rng.clone())
        .unwrap_or_else(|| Rng::from_seed(0));
    sim.ai.stances.begin();
    let mut ev = Eval::new(sim, mind, rng);
    let out = work(&mut ev);
    let (rng, warnings) = (ev.rng, ev.warnings);
    sim.ai.stances.end();
    if let Some(state) = sim.components.mind_state.get_mut(mind) {
        state.rng = rng;
    }
    for warning in warnings {
        sim.report_warning(warning);
    }
    out
}

/// Begins a step.
fn begin(sim: &mut Sim, mind: EntityId, step: &Step, held: &Held, about: &Value) -> Begun {
    let kind = step.kind();
    match kind {
        StepKind::Go => {
            let Some(go) = &step.go else {
                return Begun::Done;
            };
            let (to, within) = with_eval(sim, mind, |ev| {
                let to = ev.value(&go.to, ev.with_about(about));
                let within = go
                    .within
                    .as_ref()
                    .and_then(|within| ev.number(within, ev.with_about(about)))
                    .unwrap_or(Real::ZERO)
                    .max(Real::ZERO);
                (to, within)
            });
            let Some((target, whom)) = place_of(sim, mind, &to) else {
                return Begun::Failed(no_target(kind));
            };
            go_toward(sim, mind, kind, target, whom, within)
        }
        StepKind::KeepNear => {
            let Some(keep) = &step.keep_near else {
                return Begun::Done;
            };
            let to = with_eval(sim, mind, |ev| ev.value(&keep.who, ev.with_about(about)));
            let Some((target, whom)) = place_of(sim, mind, &to) else {
                return Begun::Failed(no_target(kind));
            };
            if nav::distance_to(sim, mind, target)
                .is_some_and(|d| d <= keep.within + Real::of(ARRIVED_SLACK_M))
            {
                // Near enough: stay, and look again at the next thought.
                let mut pending = Pending::new(kind);
                pending.target = Some(target);
                pending.within = keep.within;
                pending.whom = whom;
                let interval = sim
                    .components
                    .mind_state
                    .get(mind)
                    .map_or(Dur::SECOND, |state| state.interval);
                return match sim.wait(mind, interval, cause_of(mind)) {
                    Ok(()) => Begun::Under(pending),
                    Err(reason) => Begun::Failed(reason),
                };
            }
            go_toward(sim, mind, kind, target, whom, keep.within)
        }
        StepKind::KeepFrom => {
            let Some(keep) = &step.keep_from else {
                return Begun::Done;
            };
            let from = with_eval(sim, mind, |ev| ev.value(&keep.from, ev.with_about(about)));
            let Some((danger, _)) = place_of(sim, mind, &from) else {
                return Begun::Failed(no_target(kind));
            };
            let Some(spot) = nav::away_from(sim, mind, danger, keep.distance) else {
                return Begun::Failed(
                    Reason::new(
                        why::NO_WAY_OUT,
                        "there is nowhere it knows of to get away to",
                    )
                    .with("entity", mind.to_string()),
                );
            };
            go_toward(sim, mind, kind, spot, None, Real::ZERO)
        }
        StepKind::Do => {
            let Some(act) = &step.act else {
                return Begun::Done;
            };
            // A warm character starts no action (03 §10): it is promoted
            // to act.
            super::warm::promote(sim, mind, super::warm::Why::Acting);
            sim.ai.due.remove(&mind);
            let params = with_eval(sim, mind, |ev| {
                act.params
                    .iter()
                    .map(|(name, expr)| {
                        (
                            name.clone(),
                            ev.value(expr, ev.with_about(about)).to_stored(),
                        )
                    })
                    .collect::<Vec<(String, Stored)>>()
            });
            start(sim, mind, kind, act.action.id(), &params)
        }
        StepKind::Choose => {
            let Some(choose) = &step.choose else {
                return Begun::Done;
            };
            super::warm::promote(sim, mind, super::warm::Why::Acting);
            sim.ai.due.remove(&mind);
            let target = choose
                .target
                .as_ref()
                .map(|target| with_eval(sim, mind, |ev| ev.value(target, ev.with_about(about))));
            choose_action(sim, mind, &choose.purpose, target)
        }
        StepKind::Wait => {
            let Some(wait) = &step.wait else {
                return Begun::Done;
            };
            let time = wait.time.as_ref().map(|time| {
                with_eval(sim, mind, |ev| ev.number(time, ev.with_about(about)))
                    .unwrap_or(Real::ZERO)
            });
            // An `until` with no `time` is looked at whenever the mind
            // thinks, which it does at its interval.
            let secs = time.unwrap_or_else(|| {
                sim.components
                    .mind_state
                    .get(mind)
                    .map_or(Real::ONE, |state| state.interval.as_secs_real())
            });
            wait_for(sim, mind, kind, secs)
        }
        StepKind::Look => {
            let Some(look) = &step.look else {
                return Begun::Done;
            };
            if let Some(at) = &look.at {
                let at = with_eval(sim, mind, |ev| ev.value(at, ev.with_about(about)));
                if let Some((loc, _)) = place_of(sim, mind, &at) {
                    // A mind with no senses has nothing to turn.
                    let _ = crate::perception::face_toward(sim, mind, loc);
                }
            }
            let secs = look
                .time
                .as_ref()
                .and_then(|time| with_eval(sim, mind, |ev| ev.number(time, ev.with_about(about))))
                .unwrap_or(Real::of(DEFAULT_LOOK_S));
            wait_for(sim, mind, kind, secs)
        }
        StepKind::Say => {
            let Some(say) = &step.say else {
                return Begun::Done;
            };
            let (text, volume) = with_eval(sim, mind, |ev| {
                let text = ev.text(&say.text, ev.with_about(about));
                let volume = say
                    .volume
                    .as_ref()
                    .and_then(|volume| ev.number(volume, ev.with_about(about)))
                    .unwrap_or(Real::ONE);
                (text, volume)
            });
            events::said(sim, mind, &text, volume);
            // Said as a message of a kind too (15 §11), where one is
            // named and the social layer knows it.
            if let Some(kind) = say.kind.as_deref().and_then(|kind| message_kind(sim, kind))
                && let Err(reason) =
                    crate::social::say(sim, mind, crate::social::SayArgs::new(kind), cause_of(mind))
            {
                sim.fault(reason);
            }
            Begun::Done
        }
        StepKind::Set => {
            let Some(set) = &step.set else {
                return Begun::Done;
            };
            let value = with_eval(sim, mind, |ev| ev.value(&set.value, ev.with_about(about)));
            match sim.set_var(Subject::Entity(mind), &set.var, value, cause_of(mind)) {
                Ok(_) => Begun::Done,
                Err(reason) => Begun::Failed(reason),
            }
        }
        StepKind::Settle => {
            settle_about(sim, mind, &held.about);
            Begun::Done
        }
        StepKind::Stance => {
            let Some(stance) = &step.stance else {
                return Begun::Done;
            };
            let of = with_eval(sim, mind, |ev| ev.value(&stance.of, ev.with_about(about)));
            let Some(Subject::Entity(other)) = sim.subject_of(&of) else {
                return Begun::Failed(no_target(kind));
            };
            let toward = match stance.toward {
                StanceWritten::Hostile => Some(Stance::Hostile),
                StanceWritten::Feared => Some(Stance::Feared),
                StanceWritten::None => None,
            };
            set_stance(sim, mind, other, toward);
            Begun::Done
        }
    }
}

/// A message kind by id as written, qualified or bare.
fn message_kind(sim: &Sim, name: &str) -> Option<crate::social::MessageKindId> {
    let registry = sim.defs.try_registry::<crate::social::MessageKindDef>()?;
    registry.lookup(name).or_else(|| {
        if name.contains(':') {
            return None;
        }
        registry
            .names()
            .iter()
            .filter(|qualified| qualified.rsplit(':').next() == Some(name))
            .min()
            .and_then(|qualified| registry.lookup(qualified))
    })
}

/// Starts the mind's `go_action` toward a cell, or finds it is there.
fn go_toward(
    sim: &mut Sim,
    mind: EntityId,
    kind: StepKind,
    target: Location,
    whom: Option<EntityId>,
    within: Real,
) -> Begun {
    let mut pending = Pending::new(kind);
    pending.target = Some(target);
    pending.within = within;
    pending.whom = whom;
    if nav::distance_to(sim, mind, target).is_some_and(|d| d <= within + Real::of(ARRIVED_SLACK_M))
    {
        return Begun::Done;
    }
    let Some(action) = mind_def_of(sim, mind)
        .and_then(|id| mind_def(sim, id))
        .and_then(|def| def.go_action)
        .map(|action| action.id())
    else {
        return Begun::Failed(
            Reason::new(why::NO_GO_ACTION, "its mind names no `go_action`")
                .with("entity", mind.to_string()),
        );
    };
    // Somewhere it believes it can stand: the cell itself, or the
    // nearest that is. A step that need not reach the cell exactly is
    // judged by how near it comes to the cell; one that must, by how
    // near it comes to where it can stand.
    let cell = nav::nearest_standable(sim, mind, target).unwrap_or(target);
    if within <= Real::ZERO {
        pending.target = Some(cell);
    }
    // A way it cannot find within a mind's limits is none it sets out on
    // (D-896).
    let limits = nav::way_limits(sim, mind, cell);
    if super::warm::is_warm(sim, mind) {
        return walk_toward(sim, mind, pending, cell, limits);
    }
    if !nav::way_within(sim, mind, cell, limits) {
        return Begun::Failed(
            Reason::new(why::NOT_ARRIVED, "it did not get there").with("entity", mind.to_string()),
        );
    }
    let to = sim.subject_value(Subject::Loc(cell)).to_stored();
    match start(sim, mind, kind, action, &[("to".to_owned(), to)]) {
        Begun::Under(started) => {
            pending.action = started.action;
            Begun::Under(pending)
        }
        other => other,
    }
}

/// A warm mind's step that moves (09 §9.1): its path is planned from
/// what it believes, as its `go_action` would plan it, and walked in
/// bulk by the warm stepper ([`warm`](super::warm)). A path that cannot
/// be planned is a step that did not get there, as for a hot mind.
fn walk_toward(
    sim: &mut Sim,
    mind: EntityId,
    pending: Pending,
    cell: Location,
    limits: clash_world::PathLimits,
) -> Begun {
    // Whatever wait it was idling in ends, as starting an action ends it.
    if sim.actions.wait_of(mind).is_some() {
        let _ = sim.wait(mind, Dur::ZERO, cause_of(mind));
    }
    let not_arrived = || {
        Begun::Failed(
            Reason::new(why::NOT_ARRIVED, "it did not get there").with("entity", mind.to_string()),
        )
    };
    let planned = crate::movement::plan_path(sim, mind, cell, limits, cause_of(mind));
    match planned {
        Ok(planned) if planned.steps > 0 => {
            let step = sim
                .components
                .mind_state
                .get(mind)
                .and_then(MindState::held)
                .map_or(0, |held| held.step);
            if let Some(target) = pending.target {
                super::warm::start_walk(sim, mind, target, step);
            }
            Begun::Under(pending)
        }
        _ => not_arrived(),
    }
}

/// Whether a mind's step under way met a closed way it is to open.
fn door_met(sim: &Sim, mind: EntityId) -> bool {
    sim.components
        .mind_state
        .get(mind)
        .and_then(MindState::held)
        .and_then(|held| held.pending.as_ref())
        .is_some_and(|pending| matches!(pending.door, super::state::Door::Met { .. }))
}

/// A mind hot again that walked a step in bulk (09 §9.2): the step is
/// begun again as its `go_action`, toward the same place. One that met
/// a door is begun again once the door is open ([`doors`](super::doors)).
fn resume_moving(sim: &mut Sim, mind: EntityId) {
    super::warm::stop_walk(sim, mind);
    let Some(pending) = sim
        .components
        .mind_state
        .get(mind)
        .and_then(MindState::held)
        .and_then(|held| held.pending.clone())
        .filter(|pending| pending.outcome == Outcome::Running && pending.action.is_none())
        .filter(|pending| pending.door == super::state::Door::None)
    else {
        return;
    };
    let Some(target) = pending.target else {
        return;
    };
    let begun = go_toward(
        sim,
        mind,
        pending.kind(),
        target,
        pending.whom,
        pending.within,
    );
    let Some(held) = sim
        .components
        .mind_state
        .get_mut(mind)
        .and_then(|state| state.held.as_mut())
    else {
        return;
    };
    match begun {
        Begun::Under(begun) => held.pending = Some(begun),
        Begun::Done => {
            if let Some(pending) = held.pending.as_mut() {
                pending.outcome = Outcome::Done;
            }
        }
        Begun::Failed(reason) => {
            if let Some(pending) = held.pending.as_mut() {
                pending.outcome = Outcome::Failed(reason);
            }
        }
    }
}

/// A mind lets go of the action of its step under way, so that the
/// action's ending is not the step's: what a warm mind does before it
/// ends the action it walked with ([`warm`](super::warm)).
pub(super) fn let_go(sim: &mut Sim, mind: EntityId) {
    if let Some(pending) = sim
        .components
        .mind_state
        .get_mut(mind)
        .and_then(|state| state.held.as_mut())
        .and_then(|held| held.pending.as_mut())
    {
        pending.action = None;
    }
}

/// Undoes [`let_go`]: the action is the step's again.
pub(super) fn take_back(sim: &mut Sim, mind: EntityId, id: crate::later::ActionId) {
    if let Some(pending) = sim
        .components
        .mind_state
        .get_mut(mind)
        .and_then(|state| state.held.as_mut())
        .and_then(|held| held.pending.as_mut())
    {
        pending.action = Some(id);
    }
}

/// Starts an action as a step.
fn start(
    sim: &mut Sim,
    mind: EntityId,
    kind: StepKind,
    action: ActionDefId,
    params: &[(String, Stored)],
) -> Begun {
    // Whatever wait it was idling in ends.
    if sim.actions.wait_of(mind).is_some() {
        let _ = sim.wait(mind, Dur::ZERO, cause_of(mind));
    }
    match sim.start_action(mind, action, &params.to_vec(), cause_of(mind)) {
        Ok(Some(id)) => {
            let mut pending = Pending::new(kind);
            pending.action = Some(id);
            // An action that ended at once (a phase of no length) is
            // over already.
            if sim.actions.get(id).is_none() {
                pending.outcome = Outcome::Done;
            }
            Begun::Under(pending)
        }
        Ok(None) => Begun::Failed(
            Reason::new(why::REFUSED, "a handler refused the action").with(
                "action",
                crate::action::run::action_name(&sim.defs, action).as_str(),
            ),
        ),
        Err(reason) => Begun::Failed(reason),
    }
}

/// The `choose` step: the first action tagged with the purpose that can
/// start, given the target as its parameter `target` if it has one.
fn choose_action(sim: &mut Sim, mind: EntityId, purpose: &str, target: Option<Value>) -> Begun {
    let defs = Arc::clone(&sim.defs);
    let Some(tag) = defs.tags().lookup(purpose) else {
        return Begun::Failed(no_option(purpose));
    };
    let Some(registry) = defs.try_registry::<ActionDef>() else {
        return Begun::Failed(no_option(purpose));
    };
    let mut last: Option<Reason> = None;
    for (id, def) in registry.iter() {
        if !def.tags.contains(tag) {
            continue;
        }
        let mut params: Vec<(String, Stored)> = Vec::new();
        if let Some(target) = &target
            && (def.param("target").is_some()
                || def.roles().iter().any(|role| role.name.value == "target"))
        {
            params.push(("target".to_owned(), target.to_stored()));
        }
        match start(sim, mind, StepKind::Choose, id, &params) {
            Begun::Failed(reason) => last = Some(reason),
            other => {
                events::option_chosen(sim, mind, purpose, registry.name(id));
                return other;
            }
        }
    }
    Begun::Failed(last.unwrap_or_else(|| no_option(purpose)))
}

fn no_option(purpose: &str) -> Reason {
    Reason::new(
        why::NO_OPTION,
        format!("no action that serves `{purpose}` can be started"),
    )
    .with("purpose", purpose)
}

/// Waits as a step.
fn wait_for(sim: &mut Sim, mind: EntityId, kind: StepKind, secs: Real) -> Begun {
    let secs = if secs.is_finite() {
        secs.max(Real::ZERO)
    } else {
        Real::ZERO
    };
    if secs <= Real::ZERO {
        return Begun::Done;
    }
    match sim.wait(mind, Dur::from_secs_real(secs), cause_of(mind)) {
        Ok(()) => Begun::Under(Pending::new(kind)),
        Err(reason) => Begun::Failed(reason),
    }
}

/// Marks what a goal is about settled (the `settle` step).
pub(super) fn settle_about(sim: &mut Sim, mind: EntityId, about: &AboutHeld) {
    let track_at = match about {
        AboutHeld::Track(persona) => {
            belief::track_of(sim, mind, persona.entity()).map(belief::Track::at)
        }
        _ => None,
    };
    let Some(state) = sim.components.mind_state.get_mut(mind) else {
        return;
    };
    match about {
        AboutHeld::Track(persona) => {
            state.settled_tracks.retain(|(have, _)| have != persona);
            if let Some(at) = track_at {
                state.settled_tracks.push((*persona, at));
                state.settled_tracks.sort();
            }
        }
        AboutHeld::Memory(key) => {
            if !state.settled_memories.contains(key) {
                state.settled_memories.push(key.clone());
                while state.settled_memories.len() > super::MAX_SETTLED {
                    state.settled_memories.remove(0);
                }
            }
        }
        AboutHeld::None | AboutHeld::Place(_) => {}
    }
}

/// Sets a mind's stance toward an entity, as it perceives it.
pub fn set_stance(sim: &mut Sim, mind: EntityId, other: EntityId, stance: Option<Stance>) {
    let persona: PersonaId = belief::persona_of(sim, mind, other);
    if !sim.components.mind_state.contains(mind) {
        let defs = Arc::clone(&sim.defs);
        let Some(def) = mind_def_of(sim, mind)
            .and_then(|id| defs.try_registry::<MindDef>().and_then(|r| r.try_get(id)))
        else {
            return;
        };
        begin_state(sim, mind, def, None);
    }
    if let Some(state) = sim.components.mind_state.get_mut(mind) {
        state.set_stance(persona, stance);
    }
}

/// A step's action ended: what came of it is kept for the next thought.
pub(super) fn action_ended(
    sim: &mut Sim,
    actor: EntityId,
    id: crate::later::ActionId,
    outcome: Outcome,
) {
    // The opening of a door met on the way: the step goes on.
    if super::doors::action_ended(sim, actor, id) {
        return;
    }
    let Some(pending) = sim
        .components
        .mind_state
        .get_mut(actor)
        .and_then(|state| state.held.as_mut())
        .and_then(|held| held.pending.as_mut())
    else {
        return;
    };
    if pending.action != Some(id) || pending.outcome != Outcome::Running {
        return;
    }
    // A step that moves is judged by where it ended, at the next
    // thought; any other is what its action came to.
    let moves = matches!(
        pending.kind(),
        StepKind::Go | StepKind::KeepNear | StepKind::KeepFrom
    );
    if moves && outcome == Outcome::Done {
        return;
    }
    pending.outcome = outcome;
    sim.ai.due.insert(actor);
}

/// A field of an event as plain text, if it has one.
pub(super) fn text_field(field: Option<FieldValue>) -> Option<String> {
    match field? {
        FieldValue::Plain(Stored::Str(text)) => Some(text),
        _ => None,
    }
}
