//! Modes (08 §5.4): states an ability is switched into, with a trait and
//! an upkeep that begin and end together (invariant A5), ending when the
//! upkeep runs a pool dry.
//!
//! Upkeep is a pool rate applied under the mode's own stats source
//! ([`mode_source`]), so the pool's level runs as a straight line and
//! nothing is checked per tick. The moment the first pool of the upkeep
//! comes to nothing is worked out from that line and scheduled once on
//! the owner's clock ([`mode_wake`]); it moves with the clock's rate.
//! Whatever else changes the owner's pools in a way the kernel hears of
//! (a threshold crossed, a trait, an attribute, a cost, a move into or
//! out of the world) has it worked out again ([`reschedule_all`]).

use std::sync::Arc;

use clash_core::{Dur, EntityId, MICROS_PER_SECOND, Real};
use clash_time::WakeId;
use clash_world::Cause;

use super::ops::{inst, inst_of, not_held, notify};
use super::{
    Abilities, AbilityDef, AbilityId, MODE_ENDED, MODE_STARTED, ModeOn, State, codes, def_of,
    handle_value, mode_giver, mode_source, mode_wake, name_of,
};
use crate::Sim;
use crate::bus::EventPayload;
use crate::event::Subject;
use crate::host::Bound;
use crate::reason::Reason;
use crate::stats::{PoolId, Stats};

fn mode_unknown(ability: &str, mode: &str) -> Reason {
    Reason::new(
        codes::MODE_UNKNOWN,
        format!("the ability `{ability}` has no mode `{mode}`"),
    )
    .with("ability", ability)
    .with("mode", mode)
}

/// The upkeep of a mode, worked out for an instance: each pool and what
/// it drains each second.
fn upkeep(sim: &Sim, entity: EntityId, number: u64, def: &AbilityDef, mode: usize) -> Vec<(PoolId, Real)> {
    let Some(mode) = def.modes.get(mode) else {
        return Vec::new();
    };
    let bound = Bound::new()
        .subject(sim.subject_value(Subject::Entity(entity)))
        .subject(handle_value(sim, number));
    let mut host = sim.host(bound);
    mode.upkeep
        .iter()
        .map(|(pool, rate)| {
            let rate = rate
                .eval_on(&sim.defs, &mut host)
                .filter(|rate| rate.is_finite() && *rate > Real::ZERO)
                .unwrap_or(Real::ZERO);
            (pool.id(), rate)
        })
        .collect()
}

/// Switches a mode of an ability on (08 §5.4): what switching costs is
/// taken, its trait is given with the instance's mode as giver, its
/// upkeep applied as pool rates, its `on_start` run, and `mode_started`
/// raised.
///
/// Refused, changing nothing, for an ability not held, dormant, or
/// below the mode's `at_rank`; a mode that is not the ability's or is
/// on already; an upkeep pool the owner lacks or that is empty; a cost
/// that cannot be met; and with whatever reason a handler of
/// `trait_adding` refuses its trait for.
pub fn start_mode(
    sim: &mut Sim,
    entity: EntityId,
    def: AbilityId,
    mode_id: &str,
    cause: Cause,
) -> Result<(), Reason> {
    sim.known(entity)?;
    let defs = Arc::clone(&sim.defs);
    let ability = def_of(&defs, def).ok_or_else(|| super::ops::unknown(def.index().to_string()))?;
    let name = name_of(&defs, def);
    let current = inst_of(sim, entity, def)
        .filter(|inst| inst.is_held())
        .cloned()
        .ok_or_else(|| not_held(sim, entity, def))?;
    let number = current.number();
    let mode = ability
        .mode_index(mode_id)
        .ok_or_else(|| mode_unknown(&name, mode_id))?;
    if current.state() == State::Dormant {
        return Err(Reason::new(
            codes::DORMANT,
            format!("the ability `{name}` is dormant"),
        )
        .with("ability", name.as_str()));
    }
    let rank = super::rank(sim, entity, &current);
    if let Some(needed) = ability.modes[mode].at_rank
        && rank < needed
    {
        return Err(Reason::new(
            codes::RANK_TOO_LOW,
            format!("the mode `{mode_id}` of `{name}` needs rank {needed}, and it is at {rank}"),
        )
        .with("ability", name.as_str())
        .with("rank", i64::from(rank))
        .with("needed", i64::from(needed)));
    }
    if current.mode_is_on(mode) {
        return Err(Reason::new(
            codes::MODE_ON,
            format!("the mode `{mode_id}` of `{name}` is on already"),
        )
        .with("ability", name.as_str())
        .with("mode", mode_id));
    }
    let drains = upkeep(sim, entity, number, ability, mode);
    for (pool, _) in &drains {
        let level = sim
            .components
            .get::<Stats>(entity)
            .and_then(|stats| stats.pool_level(&sim.timeline, *pool));
        if level.is_none_or(|level| level <= Real::ZERO) {
            let pool_name = sim
                .defs
                .try_registry::<crate::stats::PoolDef>()
                .map_or_else(String::new, |registry| registry.name(*pool).to_owned());
            return Err(Reason::new(
                codes::UPKEEP,
                format!("the mode `{mode_id}` of `{name}` draws on `{pool_name}`, and there is none"),
            )
            .with("ability", name.as_str())
            .with("mode", mode_id)
            .with("pool", pool_name));
        }
    }
    let tables = Arc::clone(&sim.ability_rt.tables);
    let (trait_id, cost_kinds) = tables
        .modes
        .get(def.index())
        .and_then(|modes| modes.get(mode))
        .cloned()
        .unwrap_or((None, Vec::new()));
    let plans = super::using::plan_costs(
        sim,
        entity,
        number,
        &ability.modes[mode].cost,
        &cost_kinds,
    )?;
    super::using::check_costs(sim, entity, &plans)?;
    let index = u16::try_from(mode).unwrap_or(u16::MAX);
    let mut spent = super::using::pay_costs(sim, entity, &plans, cause);
    if let Some(trait_id) = trait_id
        && let Err(reason) =
            crate::traits::add_trait(sim, entity, trait_id, mode_giver(number, index), None, cause)
    {
        super::using::refund(sim, entity, &mut spent, cause);
        return Err(reason);
    }
    for (pool, rate) in &drains {
        if let Err(reason) =
            sim.pool_rate(entity, *pool, mode_source(number, index), Some(-*rate), cause)
        {
            sim.fault(reason);
        }
    }
    if let Some(inst) = sim
        .components
        .get_mut::<Abilities>(entity)
        .and_then(|abilities| abilities.by_number_mut(number))
    {
        inst.modes.push(ModeOn {
            mode: index,
            wake: None,
        });
    }
    run(sim, entity, number, &ability.modes[mode].on_start, cause);
    schedule(sim, entity, number, index);
    notify(
        sim,
        entity,
        EventPayload::new(MODE_STARTED)
            .with("ability", name.as_str())
            .with("mode", mode_id),
        cause,
    );
    Ok(())
}

/// Switches a mode off (08 §5.4): its trait and its upkeep go together,
/// its `on_end` runs and `mode_ended` is raised with `how` `ended`.
/// Refused, changing nothing, for an ability not held, a mode that is
/// not the ability's and one that is not on.
pub fn end_mode(
    sim: &mut Sim,
    entity: EntityId,
    def: AbilityId,
    mode_id: &str,
    cause: Cause,
) -> Result<(), Reason> {
    sim.known(entity)?;
    let defs = Arc::clone(&sim.defs);
    let ability = def_of(&defs, def).ok_or_else(|| super::ops::unknown(def.index().to_string()))?;
    let name = name_of(&defs, def);
    let current = inst_of(sim, entity, def)
        .filter(|inst| inst.is_held())
        .cloned()
        .ok_or_else(|| not_held(sim, entity, def))?;
    let mode = ability
        .mode_index(mode_id)
        .ok_or_else(|| mode_unknown(&name, mode_id))?;
    if !current.mode_is_on(mode) {
        return Err(Reason::new(
            codes::MODE_OFF,
            format!("the mode `{mode_id}` of `{name}` is not on"),
        )
        .with("ability", name.as_str())
        .with("mode", mode_id));
    }
    end_one(
        sim,
        entity,
        current.number(),
        u16::try_from(mode).unwrap_or(u16::MAX),
        "ended",
        cause,
    );
    Ok(())
}

/// Runs a mode's effects with the owner as `self` and the instance as
/// `ability`.
fn run(sim: &mut Sim, entity: EntityId, number: u64, effects: &crate::rule::Effects, cause: Cause) {
    if effects.is_empty() {
        return;
    }
    let subjects = [
        sim.subject_value(Subject::Entity(entity)),
        handle_value(sim, number),
    ];
    let mut locals = Vec::new();
    sim.run_effects(
        effects,
        &subjects,
        &mut locals,
        Some(Subject::Entity(entity)),
        cause,
    );
}

/// Ends one mode that is on: its wake-up is cancelled, its upkeep and
/// its trait taken away together, its `on_end` run and `mode_ended`
/// raised with `how`.
pub(super) fn end_one(sim: &mut Sim, entity: EntityId, number: u64, mode: u16, how: &str, cause: Cause) {
    let Some(current) = inst(sim, entity, number).cloned() else {
        return;
    };
    let Some(on) = current.modes.iter().find(|on| on.mode == mode).copied() else {
        return;
    };
    if let Some(wake) = on.wake {
        sim.cancel_wake(wake);
    }
    if let Some(inst) = sim
        .components
        .get_mut::<Abilities>(entity)
        .and_then(|abilities| abilities.by_number_mut(number))
    {
        inst.modes.retain(|have| have.mode != mode);
    }
    let defs = Arc::clone(&sim.defs);
    let Some(ability) = def_of(&defs, current.def()) else {
        return;
    };
    let Some(mode_def) = ability.modes.get(usize::from(mode)) else {
        return;
    };
    for pool in mode_def.upkeep.keys() {
        let _ = sim.pool_rate(entity, pool.id(), mode_source(number, mode), None, cause);
    }
    crate::traits::withdraw(sim, entity, mode_giver(number, mode), cause);
    run(sim, entity, number, &mode_def.on_end, cause);
    let name = name_of(&defs, current.def());
    notify(
        sim,
        entity,
        EventPayload::new(MODE_ENDED)
            .with("ability", name.as_str())
            .with("mode", mode_def.id.value.as_str())
            .with("how", how),
        cause,
    );
}

/// Ends every mode of an instance that is on, with `how`.
pub(super) fn end_all(sim: &mut Sim, entity: EntityId, number: u64, how: &str, cause: Cause) {
    let modes: Vec<u16> = inst(sim, entity, number)
        .map(|inst| inst.modes.iter().map(|on| on.mode).collect())
        .unwrap_or_default();
    for mode in modes {
        end_one(sim, entity, number, mode, how, cause);
    }
}

/// Ends the modes of an instance whose `at_rank` is now above its rank.
pub(super) fn end_above_rank(sim: &mut Sim, entity: EntityId, number: u64, cause: Cause) {
    let Some(current) = inst(sim, entity, number).cloned() else {
        return;
    };
    let rank = super::rank(sim, entity, &current);
    let defs = Arc::clone(&sim.defs);
    let Some(ability) = def_of(&defs, current.def()) else {
        return;
    };
    for on in &current.modes {
        let above = ability
            .modes
            .get(usize::from(on.mode))
            .and_then(|mode| mode.at_rank)
            .is_some_and(|needed| rank < needed);
        if above {
            end_one(sim, entity, number, on.mode, "rank", cause);
        }
    }
}

/// The local time on the owner's clock until the first pool of a
/// mode's upkeep comes to nothing, if one is draining: zero for one that
/// is empty already.
fn time_to_empty(sim: &Sim, entity: EntityId, pools: &[PoolId]) -> Option<Dur> {
    let stats = sim.components.get::<Stats>(entity)?;
    let mut soonest: Option<Dur> = None;
    for pool in pools {
        let (Some(level), Some(net)) = (stats.pool_level(&sim.timeline, *pool), stats.pool_rate(*pool))
        else {
            continue;
        };
        let after = if level <= Real::ZERO {
            Dur::ZERO
        } else if net < Real::ZERO {
            let micros = (level / -net * Real::from_i64(MICROS_PER_SECOND)).ceil();
            if !micros.is_finite() || micros > Real::from_i64(i64::MAX / 2) {
                continue;
            }
            Dur(micros.to_i64())
        } else {
            continue;
        };
        soonest = Some(soonest.map_or(after, |have| have.min(after)));
    }
    soonest
}

/// Schedules the wake-up of a mode that is on for the moment its upkeep
/// runs a pool dry, replacing the one it had. None is scheduled while
/// the owner is out of the world, or nothing is draining.
fn schedule(sim: &mut Sim, entity: EntityId, number: u64, mode: u16) {
    let Some(current) = inst(sim, entity, number).cloned() else {
        return;
    };
    let Some(on) = current.modes.iter().find(|on| on.mode == mode).copied() else {
        return;
    };
    if let Some(wake) = on.wake {
        sim.cancel_wake(wake);
    }
    let pools: Vec<PoolId> = def_of(&sim.defs, current.def())
        .and_then(|def| def.modes.get(usize::from(mode)))
        .map(|mode| mode.upkeep.keys().map(|pool| pool.id()).collect())
        .unwrap_or_default();
    let wake = time_to_empty(sim, entity, &pools).and_then(|after| {
        sim.schedule_for(entity, after, mode_wake(entity, number, mode))
            .ok()
    });
    if let Some(inst) = sim
        .components
        .get_mut::<Abilities>(entity)
        .and_then(|abilities| abilities.by_number_mut(number))
        && let Some(on) = inst.modes.iter_mut().find(|on| on.mode == mode)
    {
        on.wake = wake;
    }
}

/// Works out again when each mode of an entity that is on runs dry:
/// what the kernel asks when something may have moved its pools.
pub(crate) fn reschedule_all(sim: &mut Sim, entity: EntityId) {
    let on: Vec<(u64, u16)> = sim
        .components
        .get::<Abilities>(entity)
        .map(|abilities| {
            abilities
                .instances()
                .iter()
                .flat_map(|inst| inst.modes.iter().map(move |on| (inst.number(), on.mode)))
                .collect()
        })
        .unwrap_or_default();
    for (number, mode) in on {
        schedule(sim, entity, number, mode);
    }
}

/// A mode's wake-up popped: if a pool of its upkeep is dry the mode
/// ends (`how` `empty`); otherwise its pools moved since it was
/// scheduled, and it is scheduled again.
pub(super) fn on_wake(sim: &mut Sim, id: WakeId, entity: EntityId, number: u64, mode: u16) {
    let Some(current) = inst(sim, entity, number).cloned() else {
        return;
    };
    let current_wake = current
        .modes
        .iter()
        .find(|on| on.mode == mode)
        .and_then(|on| on.wake);
    if current_wake != Some(id) {
        // Moved or cancelled since: stale.
        return;
    }
    if let Some(inst) = sim
        .components
        .get_mut::<Abilities>(entity)
        .and_then(|abilities| abilities.by_number_mut(number))
        && let Some(on) = inst.modes.iter_mut().find(|on| on.mode == mode)
    {
        on.wake = None;
    }
    let pools: Vec<PoolId> = def_of(&sim.defs, current.def())
        .and_then(|def| def.modes.get(usize::from(mode)))
        .map(|mode| mode.upkeep.keys().map(|pool| pool.id()).collect())
        .unwrap_or_default();
    let dry = sim.components.get::<Stats>(entity).is_some_and(|stats| {
        pools.iter().any(|pool| {
            stats
                .pool_level(&sim.timeline, *pool)
                .is_none_or(|level| level <= Real::ZERO)
        })
    });
    if dry {
        end_one(sim, entity, number, mode, "empty", Cause::new(clash_world::CauseKind::Process));
    } else {
        schedule(sim, entity, number, mode);
    }
}
