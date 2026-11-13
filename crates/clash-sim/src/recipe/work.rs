//! Doing the work (16 §8.2): starting a recipe, keeping its progress
//! and its one scheduled wake-up true to its rate, and completing it.
//!
//! A recipe under way is a [`Process`] on the entity that keeps it: the
//! station's device, or the worker for work by hand. Its progress is the
//! work done, in seconds at a rate of 1, recorded at a moment of the
//! own time of the entity whose clock it runs on (the worker while one
//! works, else the station). Between changes the work done is a straight
//! line, and the process has at most one wake-up: the earliest of its
//! completion at the present rate, the moment a store it draws on runs
//! dry, and its next step. Whenever anything its rate reads changes
//! (the station's mode, settings or stores, the worker stopping) the
//! work done is brought up to date at the old rate and the wake-up is
//! scheduled again for the new: invariant G5.
//!
//! The rate is the worker's (the recipe's `rate` of `self`, 1 if it has
//! none; a machine alone works at 1) times the station's speed now,
//! times the share of its `while` rates its stores give, times one if
//! its condition on the place holds and none if it does not. A recipe
//! whose rate reads anything that changes without telling (an
//! expression, a condition on the place) or that gives off something is
//! looked at again every `check_every`, or every `recipe.step`; one that
//! does not schedules nothing while its rate stands (invariant G7).

use clash_core::{Dur, EntityId, Real};
use clash_time::{Wake, WakeDomain, WakeId};
use clash_world::{Cause, Location};

use super::defs::{RecipeDef, RecipeId, WhileDef};
use super::supply::{self, Plan};
use super::{
    Process, RECIPE_COMPLETED, RECIPE_COMPLETING, RECIPE_STALLED, RECIPE_STARTED,
    RECIPE_STARTING, codes, recipe_def, recipe_name,
};
use crate::Sim;
use crate::bus::{EventPayload, FieldValue};
use crate::device::{self, Device};
use crate::event::{Subject, Where};
use crate::host::Bound;
use crate::interaction::Target;
use crate::later::ActionId;
use crate::reason::Reason;
use crate::substance;

/// The setting that says how often a process whose rate reads what may
/// change unannounced is looked at again (16 §8.2), in seconds.
pub const STEP_SETTING: &str = "recipe.step";
/// The default of [`STEP_SETTING`]: ten seconds.
pub const DEFAULT_STEP_S: i64 = 10;
/// The random stream recipes draw the chances of their outputs from.
pub const RECIPE_STREAM: &str = "recipes";
/// What the `arg` of a recipe's wake-up says it is.
pub(crate) const WAKE_RECIPE: u64 = 0x5245_4349;

/// Work left this small is done: a millionth of a second at rate 1.
const DONE_EPSILON: Real = Real::of(1.0e-6);

// ---- Where the work is -----------------------------------------------------------

/// Where a target is, as a cell.
fn cell_of(sim: &Sim, target: Target) -> Option<Location> {
    crate::world_verbs::position_of(sim, target.subject()).map(|(loc, _)| loc)
}

/// Whether two cells are the same or neighbours on one storey.
fn within_a_step(sim: &Sim, from: Location, to: Location) -> bool {
    sim.cosmos
        .delta(from, to)
        .is_some_and(|delta| delta.z == 0 && delta.x.abs() <= 1 && delta.y.abs() <= 1)
}

/// The place work on a recipe is done at: the station's cell, else the
/// worker's.
fn place_of(sim: &Sim, worker: Option<EntityId>, station: Option<Target>) -> Option<Location> {
    station
        .and_then(|station| cell_of(sim, station))
        .or_else(|| worker.and_then(|worker| sim.entities.location(worker)))
}

/// The values a recipe's expressions are bound to: the worker (or the
/// machine), the station, and the place.
fn bound(sim: &Sim, worker: Option<EntityId>, station: Option<EntityId>, here: Option<Location>) -> Vec<clash_expr::Value> {
    let worker = worker.or(station);
    vec![
        worker.map_or(clash_expr::Value::Null, |id| sim.subject_value(Subject::Entity(id))),
        station.map_or(clash_expr::Value::Null, |id| sim.subject_value(Subject::Entity(id))),
        here.map_or(clash_expr::Value::Null, |loc| sim.subject_value(Subject::Loc(loc))),
    ]
}

fn bound_of(values: &[clash_expr::Value]) -> Bound {
    values
        .iter()
        .fold(Bound::new(), |bound, value| bound.subject(value.clone()))
}

/// Whether a station's definition carries one of a recipe's `at` tags.
fn station_fits(sim: &Sim, recipe: &RecipeDef, station: Target) -> bool {
    device::def_for(sim, station).is_some_and(|def| def.tags.iter().any(|tag| recipe.at.contains(tag)))
}

/// The running action of a worker whose phase is held: what its work
/// releases when the recipe completes.
pub(crate) fn held_action(sim: &Sim, worker: EntityId) -> Option<ActionId> {
    sim.actions_of(worker).iter().rev().copied().find(|id| {
        sim.actions.get(*id).is_some_and(|running| {
            crate::action::run::action_def(&sim.defs, running.def)
                .and_then(|def| def.phase(running.phase))
                .is_some_and(crate::action::def::Phase::is_held)
        })
    })
}

// ---- Whether it can be started -------------------------------------------------

/// What starting a recipe would do: resume one under way, or use what a
/// plan says.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Starting {
    /// It is under way where it would be kept: go on with it.
    Resume,
    /// Begin it, using what the plan says.
    Begin(Plan),
}

/// Whether a recipe can be started, or resumed, now: by a worker (or a
/// machine alone, with none), at a station or by hand. Against what the
/// worker believes is at hand ([`supply::belief`]). Changes nothing.
pub(crate) fn check(
    sim: &Sim,
    worker: Option<EntityId>,
    id: RecipeId,
    station: Option<Target>,
) -> Result<Starting, Reason> {
    let name = recipe_name(sim, id);
    let recipe = recipe_def(sim, id).ok_or_else(|| {
        Reason::new(codes::UNKNOWN, format!("there is no recipe {}", id.raw()))
            .with("recipe", i64::from(id.raw()))
    })?;
    if let Some(worker) = worker {
        sim.known(worker)?;
    }
    let station = if recipe.by_hand() { None } else { station };
    if !recipe.by_hand() {
        let Some(at) = station else {
            return Err(Reason::new(
                codes::NO_STATION,
                format!("it is made at {}", recipe.at.join(" or ")),
            )
            .with("recipe", name.as_str())
            .with("at", recipe.at.join(", ")));
        };
        if !station_fits(sim, recipe, at) {
            return Err(Reason::new(
                codes::NO_STATION,
                format!("that is not a place to make it: it is made at {}", recipe.at.join(" or ")),
            )
            .with("recipe", name.as_str())
            .with("at", recipe.at.join(", ")));
        }
        if let Some(worker) = worker {
            let reached = match (sim.entities.location(worker), cell_of(sim, at)) {
                (Some(from), Some(to)) => within_a_step(sim, from, to),
                _ => false,
            };
            if !reached {
                return Err(Reason::new(codes::OUT_OF_REACH, "that is out of reach")
                    .with("recipe", name.as_str()));
            }
        }
    }
    let device = station.and_then(|at| device::device_now(sim, at));
    let holder = device.or(if recipe.by_hand() { worker } else { None });
    let under_way = holder.and_then(|holder| sim.components.get::<Process>(holder));
    let resume = match under_way {
        Some(process) if process.recipe == id => {
            let busy = process.worker.is_some_and(|other| Some(other) != worker && process_action_runs(sim, process));
            if busy {
                return Err(Reason::new(codes::BUSY, "someone is working there")
                    .with("recipe", name.as_str()));
            }
            true
        }
        Some(process) if device.is_some() => {
            let other = recipe_name(sim, process.recipe);
            return Err(Reason::new(
                codes::BUSY,
                format!("{} is under way there", super::recipe_label(sim, process.recipe)),
            )
            .with("recipe", name.as_str())
            .with("other", other.as_str()));
        }
        _ => false,
    };
    let provided = super::know_how::provided_at(sim, station);
    for needed in &recipe.know_how {
        let known = worker.is_some_and(|worker| super::know_how::knows(sim, worker, needed.id()));
        if !known && !provided.contains(&needed.id()) {
            let what = super::know_how::know_how_label(sim, needed.id());
            return Err(Reason::new(
                codes::UNKNOWN_HOW,
                format!("it needs know-how of {what}"),
            )
            .with("recipe", name.as_str())
            .with("know_how", super::know_how::know_how_name(sim, needed.id())));
        }
    }
    if let Some(at) = station
        && device::speed_of(sim, at) <= Real::ZERO
    {
        let mode = device::mode_of(sim, at).unwrap_or_default();
        return Err(Reason::new(
            codes::STATION_IDLE,
            if mode.is_empty() {
                "it does not work now".to_owned()
            } else {
                format!("it does not work while it is {mode}")
            },
        )
        .with("recipe", name.as_str())
        .with("mode", mode.as_str()));
    }
    let here = place_of(sim, worker, station);
    let values = bound(sim, worker, device, here);
    let needs = {
        let mut host = sim.host(bound_of(&values));
        recipe.needs.holds_on(&sim.defs, &mut host)
    };
    if !needs {
        let text = recipe
            .fail_text
            .as_ref()
            .map(|text| {
                let mut host = sim.host(bound_of(&values));
                text.render_on(&sim.defs, &mut host)
            })
            .filter(|text| !text.is_empty())
            .unwrap_or_else(|| "that cannot be made now".to_owned());
        return Err(Reason::new(codes::NEEDS, text).with("recipe", name.as_str()));
    }
    if resume {
        return Ok(Starting::Resume);
    }
    let items = supply::belief::at_hand(sim, worker, device, here);
    let plan = supply::plan(sim, id, recipe, &items)?;
    // Room for what it pours out.
    // What its inputs are drawn from leaves room in it.
    let drawn = plan
        .draws
        .iter()
        .fold(Real::ZERO, |sum, (_, _, volume)| sum + *volume);
    let mut room = items
        .iter()
        .filter_map(|item| substance::room_in(sim, *item))
        .fold(drawn, |sum, room| sum + room);
    for output in &recipe.outputs {
        if output.substance.is_some() {
            let wanted = output.amount.map_or(Real::ZERO, |amount| amount.get());
            if wanted > room {
                return Err(Reason::new(
                    codes::NO_ROOM,
                    "there is nothing at hand with room to take what it gives",
                )
                .with("recipe", name.as_str()));
            }
            room -= wanted;
        }
    }
    Ok(Starting::Begin(plan))
}

/// Whether the action a process's worker works it by is still running.
fn process_action_runs(sim: &Sim, process: &Process) -> bool {
    process.action.is_some_and(|action| sim.actions.get(action).is_some())
}

// ---- Starting ------------------------------------------------------------------

/// The subjects of this module's events.
fn payload(name: &'static str, actor: EntityId, station: Option<EntityId>, recipe: &str) -> EventPayload {
    let mut payload = EventPayload::new(name)
        .subject("self", Subject::Entity(actor))
        .with("recipe", FieldValue::text(recipe));
    if let Some(station) = station {
        payload = payload.subject("station", Subject::Entity(station));
    }
    payload
}

/// Starts a recipe, or resumes it where it is under way (16 §8.2): by a
/// worker, whose held action its completion releases, or by a machine
/// alone. Checks as [`check`] does against truth, promotes the station,
/// raises the hook `recipe_starting`, uses up the inputs of a new start,
/// raises `recipe_started` and schedules the work. Returns the entity
/// that keeps the process.
pub fn start(
    sim: &mut Sim,
    worker: Option<EntityId>,
    id: RecipeId,
    station: Option<Target>,
    cause: Cause,
) -> Result<EntityId, Reason> {
    let starting = check(sim, worker, id, station)?;
    let name = recipe_name(sim, id);
    let by_hand = recipe_def(sim, id).is_some_and(RecipeDef::by_hand);
    let station = if by_hand { None } else { station };
    let device = match station {
        Some(at) => Some(device::device_for(sim, at, cause)?),
        None => None,
    };
    let Some(holder) = device.or(worker) else {
        return Err(Reason::new(codes::NO_STATION, "nothing is there to work it")
            .with("recipe", name.as_str()));
    };
    let actor = worker.unwrap_or(holder);
    if sim.bus.has_listeners(RECIPE_STARTING) {
        let raised = sim.raise(Where::Entity(actor), cause, payload(RECIPE_STARTING, actor, device, &name))?;
        if let Some(reason) = raised.cancelled {
            if let Some(device) = device {
                device::settle(sim, device, cause);
            }
            return Err(reason);
        }
    }
    let resumed = matches!(starting, Starting::Resume);
    let mut process = match starting {
        Starting::Resume => {
            let mut process = sim
                .components
                .get::<Process>(holder)
                .cloned()
                .ok_or_else(|| Reason::new(codes::UNKNOWN, "the work is gone"))?;
            bring_to_now(sim, holder, &mut process);
            process
        }
        Starting::Begin(plan) => {
            // Work by hand left unfinished for another is given up.
            if let Some(old) = sim.components.remove::<Process>(holder)
                && let Some(wake) = old.wake
            {
                sim.cancel_wake(wake);
            }
            use_up(sim, &plan, cause)?;
            let needed = recipe_def(sim, id).map_or(Real::ZERO, |recipe| recipe.work.0.as_secs_real());
            Process {
                recipe: id,
                worker: None,
                action: None,
                station: device,
                done: Real::ZERO,
                needed,
                rate: Real::ZERO,
                supply: Real::ONE,
                mark: Dur::ZERO,
                clock: holder,
                wake: None,
                stalled: None,
                mends: plan.mends,
            }
        }
    };
    process.worker = worker;
    process.action = worker.and_then(|worker| held_action(sim, worker));
    process.clock = worker.unwrap_or(holder);
    process.mark = sim.own_time(process.clock);
    process.rate = Real::ZERO;
    process.stalled = None;
    if let Some(wake) = process.wake.take() {
        sim.cancel_wake(wake);
    }
    sim.components.insert(holder, process);
    let started = payload(RECIPE_STARTED, actor, device, &name)
        .with("resumed", resumed)
        .with("work", recipe_def(sim, id).map_or(Real::ZERO, |recipe| recipe.work.0.as_secs_real()));
    let _ = sim.raise(Where::Entity(actor), cause, started);
    emit(sim, holder, cause);
    refresh(sim, holder, cause);
    Ok(holder)
}

/// Uses up what a plan says: items, and substances from containers.
fn use_up(sim: &mut Sim, plan: &Plan, cause: Cause) -> Result<(), Reason> {
    for (item, count) in &plan.takes {
        crate::item::spend(sim, *item, *count, cause)?;
    }
    for (container, substance, volume) in &plan.draws {
        substance::draw(sim, *container, *substance, *volume);
    }
    Ok(())
}

// ---- Keeping it true -------------------------------------------------------------

/// The work done and the stores drawn on, brought up to now at the rate
/// recorded.
fn bring_to_now(sim: &mut Sim, holder: EntityId, process: &mut Process) {
    let now = sim.own_time(process.clock);
    let elapsed = (now - process.mark).as_secs_real().max(Real::ZERO);
    process.mark = now;
    if process.rate <= Real::ZERO || elapsed <= Real::ZERO {
        return;
    }
    process.done = (process.done + process.rate * elapsed).min(process.needed);
    let (Some(station), Some(WhileDef::Rates(rates))) = (
        process.station,
        recipe_def(sim, process.recipe).and_then(|recipe| recipe.during.as_ref()),
    ) else {
        return;
    };
    let draws: Vec<(String, Real)> = rates
        .iter()
        .map(|(resource, want)| (resource.clone(), *want * process.supply * elapsed))
        .collect();
    let _ = holder;
    for (resource, drawn) in draws {
        if let Some(level) = device::stored(sim, station, &resource) {
            device::record_store(sim, station, &resource, level - drawn);
        }
    }
}

/// What a store of a station has given a recipe under way since its
/// progress was last recorded: what property reads of the store take
/// off its recorded level.
pub(crate) fn drawn_since(sim: &Sim, station: EntityId, resource: &str) -> Real {
    let Some(process) = sim.components.get::<Process>(station) else {
        return Real::ZERO;
    };
    if process.rate <= Real::ZERO {
        return Real::ZERO;
    }
    let Some(WhileDef::Rates(rates)) = recipe_def(sim, process.recipe).and_then(|recipe| recipe.during.as_ref()) else {
        return Real::ZERO;
    };
    let Some(want) = rates.get(resource) else {
        return Real::ZERO;
    };
    let elapsed = (sim.own_time(process.clock) - process.mark)
        .as_secs_real()
        .max(Real::ZERO);
    *want * process.supply * elapsed
}

/// Why a process does not advance now, as `recipe_stalled` says it.
const WHY_STOPPED: &str = "stopped";
const WHY_STATION: &str = "station";
const WHY_SUPPLY: &str = "supply";
const WHY_CONDITION: &str = "condition";
const WHY_WORKER: &str = "worker";

/// The rate a process works at now, the share of its `while` its stores
/// give, and why it does not advance if it does not.
fn rate_now(sim: &Sim, holder: EntityId, process: &Process) -> (Real, Real, Option<&'static str>) {
    let Some(recipe) = recipe_def(sim, process.recipe) else {
        return (Real::ZERO, Real::ZERO, Some(WHY_STOPPED));
    };
    let here = place_of(sim, process.worker, process.station.map(Target::Entity))
        .or_else(|| sim.entities.location(holder));
    let values = bound(sim, process.worker, process.station, here);
    // Who works it.
    let worker_rate = match process.worker {
        Some(worker) if sim.entities.contains(worker) && process_action_runs(sim, process) => match &recipe.rate {
            Some(rate) => {
                let mut host = sim.host(bound_of(&values));
                rate.eval_on(&sim.defs, &mut host)
                    .filter(|rate| rate.is_finite())
                    .unwrap_or(Real::ZERO)
                    .max(Real::ZERO)
            }
            None => Real::ONE,
        },
        Some(_) => return (Real::ZERO, process.supply, Some(WHY_STOPPED)),
        None => {
            let alone = process.station.is_some_and(|station| device::auto_now(sim, station)) && recipe.auto;
            if alone {
                Real::ONE
            } else {
                return (Real::ZERO, process.supply, Some(WHY_STOPPED));
            }
        }
    };
    if worker_rate <= Real::ZERO {
        return (Real::ZERO, process.supply, Some(WHY_WORKER));
    }
    let speed = process.station.map_or(Real::ONE, |station| device::speed_now(sim, station));
    if speed <= Real::ZERO {
        return (Real::ZERO, process.supply, Some(WHY_STATION));
    }
    let mut supply = Real::ONE;
    match &recipe.during {
        Some(WhileDef::Rates(rates)) => {
            for (resource, want) in rates {
                if *want <= Real::ZERO {
                    continue;
                }
                let share = process.station.map_or(Real::ZERO, |station| {
                    let level = device::stored(sim, station, resource).unwrap_or(Real::ZERO);
                    if level <= Real::ZERO {
                        return Real::ZERO;
                    }
                    let limit = device::def_of(sim, station)
                        .and_then(|def| def.stores.get(resource))
                        .and_then(|store| store.rate);
                    limit.map_or(Real::ONE, |limit| (limit / *want).min(Real::ONE))
                });
                supply = supply.min(share);
            }
            if supply <= Real::ZERO {
                return (Real::ZERO, Real::ZERO, Some(WHY_SUPPLY));
            }
        }
        Some(WhileDef::Place(condition)) => {
            let holds = {
                let mut host = sim.host(bound_of(&values));
                condition.holds_on(&sim.defs, &mut host)
            };
            if !holds {
                return (Real::ZERO, Real::ONE, Some(WHY_CONDITION));
            }
        }
        None => {}
    }
    (worker_rate * speed * supply, supply, None)
}

/// Whether a process must be looked at again now and then: its rate or
/// its condition reads what may change without telling, or it gives
/// something off.
fn steps(recipe: &RecipeDef) -> bool {
    recipe.rate.is_some()
        || recipe.check_every.is_some()
        || recipe.emits.is_some()
        || matches!(recipe.during, Some(WhileDef::Place(_)))
}

/// Brings a process up to date, completes it if its work is done, and
/// otherwise works out its rate and schedules its one wake-up again.
pub(crate) fn refresh(sim: &mut Sim, holder: EntityId, cause: Cause) {
    let Some(mut process) = sim.components.get::<Process>(holder).cloned() else {
        return;
    };
    bring_to_now(sim, holder, &mut process);
    if process.needed - process.done <= DONE_EPSILON {
        process.done = process.needed;
        sim.components.insert(holder, process);
        complete(sim, holder, cause);
        return;
    }
    let (rate, supply, why) = rate_now(sim, holder, &process);
    let was_running = process.rate > Real::ZERO;
    let newly_stalled = why.is_some() && (was_running || process.stalled.is_none());
    process.rate = rate;
    process.supply = supply;
    process.stalled = why.map(str::to_owned);
    if let Some(wake) = process.wake.take() {
        sim.cancel_wake(wake);
    }
    let defs = std::sync::Arc::clone(&sim.defs);
    let Some(recipe) = defs
        .try_registry::<RecipeDef>()
        .and_then(|registry| registry.try_get(process.recipe))
    else {
        sim.components.insert(holder, process);
        return;
    };
    // The earliest of: completion, a store running dry, the next step.
    let mut next: Option<Real> = None;
    let mut sooner = |secs: Real| {
        if secs.is_finite() && secs >= Real::ZERO {
            next = Some(next.map_or(secs, |have| have.min(secs)));
        }
    };
    if rate > Real::ZERO {
        sooner((process.needed - process.done) / rate);
        if let (Some(station), Some(WhileDef::Rates(rates))) = (process.station, &recipe.during) {
            for (resource, want) in rates {
                let draw = *want * supply;
                if draw > Real::ZERO {
                    let level = device::stored(sim, station, resource).unwrap_or(Real::ZERO);
                    sooner(level / draw);
                }
            }
        }
    }
    let stepping = steps(recipe) && (rate > Real::ZERO || matches!(recipe.during, Some(WhileDef::Place(_))) || process.worker.is_some());
    if stepping && why != Some(WHY_STOPPED) {
        let step = recipe.check_every.map_or_else(
            || {
                Dur::from_secs(sim.setting_int(STEP_SETTING, DEFAULT_STEP_S).max(1)).as_secs_real()
            },
            |every| every.0.as_secs_real(),
        );
        sooner(step);
    }
    if let Some(secs) = next {
        let after = Dur::from_secs_real(secs).max(Dur::from_micros(1));
        match sim.schedule_for(
            process.clock,
            after,
            Wake::new(WakeDomain::Stepper, holder.to_bits(), WAKE_RECIPE),
        ) {
            Ok(wake) => process.wake = Some(wake),
            Err(reason) => sim.fault(reason),
        }
    }
    let recipe_id = process.recipe;
    let (station, worker) = (process.station, process.worker);
    sim.components.insert(holder, process);
    if newly_stalled && let Some(why) = why {
        let name = recipe_name(sim, recipe_id);
        let actor = worker.unwrap_or(holder);
        let stalled = payload(RECIPE_STALLED, actor, station, &name).with("why", FieldValue::text(why));
        let _ = sim.raise(Where::Entity(actor), cause, stalled);
    }
}

/// What a process's wake-up does: gives off what it gives off, and
/// refreshes it. The handler of the `Stepper` domain's recipe wake-ups.
pub(crate) fn on_wake(sim: &mut Sim, wake_id: WakeId, wake: Wake) {
    let holder = EntityId::from_bits(wake.id);
    let ours = sim
        .components
        .get::<Process>(holder)
        .is_some_and(|process| process.wake == Some(wake_id));
    if !ours {
        return;
    }
    if let Some(process) = sim.components.get_mut::<Process>(holder) {
        process.wake = None;
    }
    let cause = Cause::new(clash_world::CauseKind::Process);
    let running = sim
        .components
        .get::<Process>(holder)
        .is_some_and(|process| process.rate > Real::ZERO);
    if running {
        emit(sim, holder, cause);
    }
    refresh(sim, holder, cause);
}

/// Gives off what a recipe under way gives off (16 §8.1 `emits`): the
/// field levels, at the place it is done.
fn emit(sim: &mut Sim, holder: EntityId, cause: Cause) {
    let Some(process) = sim.components.get::<Process>(holder) else {
        return;
    };
    let Some(emits) = recipe_def(sim, process.recipe).and_then(|recipe| recipe.emits.as_ref()) else {
        return;
    };
    let Some(here) = sim.entities.location(holder) else {
        return;
    };
    let owner = recipe_name(sim, process.recipe);
    let namespace = owner.split_once(':').map(|(namespace, _)| namespace.to_owned());
    let fields: Vec<(clash_world::FieldId, i16)> = emits
        .field
        .iter()
        .filter_map(|(name, amount)| {
            let field = sim.cosmos.defs().fields().resolve(name, namespace.as_deref())?;
            Some((field, i16::from(*amount)))
        })
        .collect();
    for (field, amount) in fields {
        if let Err(error) = sim.world(|ctx| ctx.edit(cause).add_field(here, field, amount)) {
            sim.fault(Reason::from_world(&error));
        }
    }
}

/// Stops the work a worker was doing on a process, keeping its progress
/// (16 §8.2: interrupted work is not lost): what the end of the
/// worker's action does.
pub(crate) fn stop(sim: &mut Sim, holder: EntityId, cause: Cause) {
    let Some(mut process) = sim.components.get::<Process>(holder).cloned() else {
        return;
    };
    bring_to_now(sim, holder, &mut process);
    process.action = None;
    process.worker = None;
    let is_device = sim.components.get::<Device>(holder).is_some();
    if is_device {
        process.clock = holder;
    }
    process.mark = sim.own_time(process.clock);
    sim.components.insert(holder, process);
    refresh(sim, holder, cause);
    if is_device {
        auto_check(sim, holder, cause);
    }
}

// ---- Completing ------------------------------------------------------------------

/// Completes a process (16 §8.2): the hook `recipe_completing`, whose
/// handlers may spoil it; its outputs, what it mends and its `result`
/// unless spoiled; `recipe_completed`; the worker's held action is let
/// go; a machine may start its next recipe.
pub(crate) fn complete(sim: &mut Sim, holder: EntityId, cause: Cause) {
    let Some(process) = sim.components.remove::<Process>(holder) else {
        return;
    };
    if let Some(wake) = process.wake {
        sim.cancel_wake(wake);
    }
    let name = recipe_name(sim, process.recipe);
    let actor = process.worker.unwrap_or(holder);
    let mut spoiled = false;
    if sim.bus.has_listeners(RECIPE_COMPLETING) {
        let completing = payload(RECIPE_COMPLETING, actor, process.station, &name).with("spoiled", false);
        match sim.raise(Where::Entity(actor), cause, completing) {
            Ok(raised) => {
                spoiled = raised.cancelled.is_some()
                    || raised
                        .field("spoiled")
                        .and_then(FieldValue::plain)
                        .and_then(clash_core::Value::as_bool)
                        .unwrap_or(false);
            }
            Err(reason) => sim.fault(reason),
        }
    }
    if !spoiled {
        give_outputs(sim, &process, holder, cause);
    }
    let completed = payload(RECIPE_COMPLETED, actor, process.station, &name).with("spoiled", spoiled);
    let _ = sim.raise(Where::Entity(actor), cause, completed);
    if let (Some(worker), Some(action)) = (process.worker, process.action)
        && sim.actions.get(action).is_some_and(|running| running.actor == worker)
    {
        let _ = sim.cancel_action(worker, action, false, cause);
    }
    if sim.components.get::<Device>(holder).is_some() {
        device::settle(sim, holder, cause);
        if sim.entities.contains(holder) {
            auto_check(sim, holder, cause);
        }
    }
}

/// Makes what a recipe gives: its items, its substances, what it mends,
/// and its `result`.
fn give_outputs(sim: &mut Sim, process: &Process, holder: EntityId, cause: Cause) {
    let defs = std::sync::Arc::clone(&sim.defs);
    let Some(recipe) = defs
        .try_registry::<RecipeDef>()
        .and_then(|registry| registry.try_get(process.recipe))
    else {
        return;
    };
    let here = place_of(sim, process.worker, process.station.map(Target::Entity))
        .or_else(|| sim.entities.location(holder));
    for output in &recipe.outputs {
        if output.chance < Real::ONE && !sim.rng.stream(super::work::RECIPE_STREAM).chance(output.chance) {
            continue;
        }
        if let Some(item) = output.item {
            make_item(sim, item.id(), output.count.max(1), process, here, cause);
        }
        if let (Some(substance), Some(amount)) = (output.substance, output.amount) {
            let items = supply::at_hand(sim, process.worker, process.station, here);
            let mut left = amount.get();
            for container in items {
                if left <= Real::ZERO {
                    break;
                }
                if substance::room_in(sim, container).is_none_or(|room| room <= Real::ZERO) {
                    continue;
                }
                let went = substance::put(sim, container, &[(substance.id(), left)]);
                left -= went.iter().fold(Real::ZERO, |sum, (_, volume)| sum + *volume);
            }
        }
    }
    if let (Some(repairs), Some(item)) = (&recipe.repairs, process.mends)
        && sim.entities.contains(item)
        && let Err(reason) = crate::item::repair(sim, item, repairs.share, cause)
    {
        sim.fault(reason);
    }
    if !recipe.result.is_empty() {
        let values = bound(sim, process.worker, process.station, here);
        let mut locals = Vec::new();
        sim.run_effects(
            &recipe.result,
            &values,
            &mut locals,
            Some(Subject::Entity(process.worker.unwrap_or(holder))),
            cause,
        );
    }
}

/// Makes an output item: into the station's holds where it fits, else
/// where the station stands; by hand into the worker's grasp where it
/// can take it, else where the worker stands.
fn make_item(
    sim: &mut Sim,
    def: crate::item::ItemId,
    count: u32,
    process: &Process,
    here: Option<Location>,
    cause: Cause,
) {
    let Some(here) = here else {
        return;
    };
    let stacks = sim
        .defs
        .try_registry::<crate::item::ItemDef>()
        .and_then(|registry| registry.try_get(def))
        .is_some_and(|def| def.stack);
    let (batch, times) = if stacks { (count, 1) } else { (1, count) };
    for _ in 0..times {
        let made = match crate::item::create(sim, def, batch, crate::entity::Place::at(here), cause) {
            Ok(made) => made,
            Err(reason) => {
                sim.fault(reason);
                return;
            }
        };
        match (process.station, process.worker) {
            (Some(station), _) => {
                let _ = crate::item::put_in(sim, made, station, cause);
            }
            (None, Some(worker)) => {
                let _ = crate::item::take(sim, worker, made, cause);
            }
            _ => {}
        }
    }
}

// ---- Machines ------------------------------------------------------------------

/// What a machine with `auto` does when something about it changes
/// (16 §8.2): with nothing under way, it starts the first of its recipes
/// that it can, from what is in its own holds.
pub(crate) fn auto_check(sim: &mut Sim, device: EntityId, cause: Cause) {
    if !device::auto_now(sim, device) || sim.components.get::<Process>(device).is_some() {
        return;
    }
    let Some(def) = device::def_of(sim, device) else {
        return;
    };
    let candidates: Vec<RecipeId> = if def.recipes.is_empty() {
        sim.defs
            .try_registry::<RecipeDef>()
            .map(|registry| {
                registry
                    .iter()
                    .filter(|(_, recipe)| {
                        recipe.auto && !recipe.by_hand() && def.tags.iter().any(|tag| recipe.at.contains(tag))
                    })
                    .map(|(id, _)| id)
                    .collect()
            })
            .unwrap_or_default()
    } else {
        def.recipes.iter().map(|recipe| recipe.id()).collect()
    };
    for id in candidates {
        let auto = recipe_def(sim, id).is_some_and(|recipe| recipe.auto);
        if auto && check(sim, None, id, Some(Target::Entity(device))).is_ok() {
            if let Err(reason) = start(sim, None, id, Some(Target::Entity(device)), cause) {
                sim.fault(reason);
            }
            return;
        }
    }
}

/// What a station's change of mode, settings or stores does to work at
/// it: what is under way is refreshed at its new rate; with nothing
/// under way, a machine may start something.
pub(crate) fn station_changed(sim: &mut Sim, device: EntityId, cause: Cause) {
    if sim.components.get::<Process>(device).is_some() {
        refresh(sim, device, cause);
    } else {
        auto_check(sim, device, cause);
    }
}

/// Records the work done at a station and what its stores gave, up to
/// now, without working its rate out again: before a store is changed.
pub(crate) fn bring_up_to_date(sim: &mut Sim, device: EntityId, _cause: Cause) {
    let Some(mut process) = sim.components.get::<Process>(device).cloned() else {
        return;
    };
    bring_to_now(sim, device, &mut process);
    sim.components.insert(device, process);
}

/// Every entity that keeps a process, in dense order.
pub(crate) fn holders(sim: &Sim) -> Vec<EntityId> {
    sim.components.store::<Process>().owners().to_vec()
}

/// What the end of an action does: a process its worker worked by it
/// stops, keeping its progress.
pub(crate) fn action_ended(sim: &mut Sim, actor: EntityId, action: ActionId, cause: Cause) {
    for holder in holders(sim) {
        let ours = sim
            .components
            .get::<Process>(holder)
            .is_some_and(|process| process.worker == Some(actor) && process.action == Some(action));
        if ours {
            stop(sim, holder, cause);
        }
    }
}

/// Whether an item lies in a device: what putting something into a
/// machine may start.
pub(crate) fn put_into(sim: &mut Sim, device: EntityId, cause: Cause) {
    if sim.components.get::<Device>(device).is_some() {
        auto_check(sim, device, cause);
    }
}
