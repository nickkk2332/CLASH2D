//! Using an ability (08 §5.1 to §5.3, §7.9): availability with reasons,
//! costs from pools, from items and from something else that pays,
//! overdraw, cooldowns on the owner's own time, the hook
//! `ability_using`, and starting the granted action through the action
//! runtime.

use std::collections::BTreeMap;
use std::sync::Arc;

use clash_core::{Dur, EntityId, Real};
use clash_expr::Value;
use clash_world::Cause;

use super::ops::{inst_of, not_held, notify};
use super::{
    ABILITY_REFUSED, ABILITY_USED, ABILITY_USING, AbilityDef, AbilityGrantDef, AbilityId,
    AbilityInst, CostKind, Costs, Intent, State, codes, def_of, handle_value, name_of,
};
use crate::Sim;
use crate::action::{ActionDefId, Params};
use crate::bus::{EventPayload, FieldValue};
use crate::event::{Subject, Where};
use crate::host::Bound;
use crate::item::{Item, ItemDef};
use crate::later::ActionId;
use crate::reason::Reason;
use crate::stats::{PoolDef, PoolId, Stats};

/// One cost of a use, worked out: what pays it and how much.
#[derive(Clone, PartialEq, Debug)]
pub struct CostLine {
    /// The qualified id of the pool or the item.
    pub what: String,
    /// Who pays: the owner, or what the cost is drawn `from`. `None`
    /// when that names nothing.
    pub payer: Option<EntityId>,
    /// For a pool, how much; for items, how many.
    pub amount: Real,
    /// Whether it consumes items.
    pub item: bool,
    /// For a pool: the owner's pool a shortfall is made up from, and at
    /// what ratio.
    pub overdraw: Option<(String, Real)>,
}

/// A cost worked out against the simulation.
#[derive(Clone, Debug)]
pub(crate) struct Plan {
    name: String,
    kind: CostKind,
    payer: Option<EntityId>,
    amount: Real,
    count: u32,
    overdraw: Option<(PoolId, Real)>,
}

fn pool_name(sim: &Sim, pool: PoolId) -> String {
    sim.defs
        .try_registry::<PoolDef>()
        .filter(|registry| pool.index() < registry.len())
        .map_or_else(String::new, |registry| registry.name(pool).to_owned())
}

fn item_name(sim: &Sim, item: crate::item::ItemId) -> String {
    sim.defs
        .try_registry::<ItemDef>()
        .filter(|registry| item.index() < registry.len())
        .map_or_else(String::new, |registry| registry.name(item).to_owned())
}

/// Works out the costs of a grant or a mode for an instance: how much
/// each is, and who pays it. Refused for a cost that names nothing
/// loaded.
pub(crate) fn plan_costs(
    sim: &Sim,
    entity: EntityId,
    number: u64,
    costs: &Costs,
    kinds: &[CostKind],
) -> Result<Vec<Plan>, Reason> {
    let bound = Bound::new()
        .subject(sim.subject_value(Subject::Entity(entity)))
        .subject(handle_value(sim, number));
    let mut host = sim.host(bound);
    let mut plans = Vec::with_capacity(costs.len());
    for ((name, cost), kind) in costs.iter().zip(kinds.iter().copied()) {
        let (name, payer, amount) = match kind {
            CostKind::Unknown => {
                return Err(Reason::new(
                    codes::COST_UNKNOWN,
                    format!("the cost `{name}` names no pool or item that is loaded"),
                )
                .with("cost", name.as_str()));
            }
            CostKind::Item(item) => (item_name(sim, item), Some(entity), Real::ZERO),
            CostKind::Pool(pool) => {
                let amount = cost
                    .amount
                    .eval_on(&sim.defs, &mut host)
                    .filter(|amount| amount.is_finite() && *amount > Real::ZERO)
                    .unwrap_or(Real::ZERO);
                let payer = match &cost.from {
                    None => Some(entity),
                    Some(from) => {
                        let value =
                            from.value(&mut clash_data::Runner::new().on(&sim.defs, &mut host));
                        match sim.subject_of(&value) {
                            Some(Subject::Entity(payer))
                                if sim.entities.contains(payer)
                                    && sim
                                        .components
                                        .get::<Stats>(payer)
                                        .is_some_and(|stats| stats.has_pool(pool)) =>
                            {
                                Some(payer)
                            }
                            _ => None,
                        }
                    }
                };
                (pool_name(sim, pool), payer, amount)
            }
        };
        let overdraw = cost
            .overdraw
            .as_ref()
            .map(|overdraw| (overdraw.from.id(), overdraw.ratio));
        plans.push(Plan {
            name,
            kind,
            payer,
            amount,
            count: cost.count,
            overdraw,
        });
    }
    Ok(plans)
}

/// A pool's level for an entity, zero for one it lacks.
fn level(sim: &Sim, entity: EntityId, pool: PoolId) -> Real {
    sim.components
        .get::<Stats>(entity)
        .and_then(|stats| stats.pool_level(&sim.timeline, pool))
        .unwrap_or(Real::ZERO)
}

/// The items of a definition an entity holds, anywhere about it, in
/// order, with their counts.
fn items_held(sim: &Sim, entity: EntityId, item: crate::item::ItemId) -> Vec<(EntityId, u32)> {
    sim.entities
        .contents_deep(entity)
        .into_iter()
        .filter_map(|inside| {
            let found = sim.components.get::<Item>(inside)?;
            (found.def() == Some(item)).then_some((inside, found.count()))
        })
        .collect()
}

fn short(entity: EntityId, pool: &str, wanted: Real, have: Real) -> Reason {
    let missing = wanted - have;
    Reason::new(
        codes::COST_SHORT,
        format!("it costs {wanted} of `{pool}`, and there is {have}: {missing} short"),
    )
    .with("entity", entity.to_string())
    .with("pool", pool)
    .with("wanted", wanted)
    .with("have", have)
    .with("short", missing)
}

/// Whether every cost can be met in full (invariant A4), or the reason
/// the first that cannot gives: which pool, how much is wanted, how
/// much is there and how much is short.
pub(crate) fn check_costs(sim: &Sim, entity: EntityId, plans: &[Plan]) -> Result<(), Reason> {
    // What each payer's pool is asked for in all, overdraw included.
    let mut asked: BTreeMap<(EntityId, PoolId), Real> = BTreeMap::new();
    for plan in plans {
        match plan.kind {
            CostKind::Unknown => {}
            CostKind::Item(item) => {
                let have: u32 = items_held(sim, entity, item)
                    .iter()
                    .map(|(_, count)| count)
                    .sum();
                if have < plan.count {
                    return Err(Reason::new(
                        codes::ITEMS_SHORT,
                        format!(
                            "it consumes {} of `{}`, and {have} are held",
                            plan.count, plan.name
                        ),
                    )
                    .with("item", plan.name.as_str())
                    .with("wanted", i64::from(plan.count))
                    .with("have", i64::from(have)));
                }
            }
            CostKind::Pool(pool) => {
                let (have, payer) = match plan.payer {
                    Some(payer) => {
                        let already = asked.get(&(payer, pool)).copied().unwrap_or(Real::ZERO);
                        (
                            (level(sim, payer, pool) - already).max(Real::ZERO),
                            Some(payer),
                        )
                    }
                    None => (Real::ZERO, None),
                };
                if have >= plan.amount {
                    if let Some(payer) = payer {
                        *asked.entry((payer, pool)).or_insert(Real::ZERO) += plan.amount;
                    }
                    continue;
                }
                let Some((from, ratio)) = plan.overdraw else {
                    if payer.is_none() {
                        return Err(Reason::new(
                            codes::COST_SOURCE_GONE,
                            format!(
                                "the cost of `{}` is drawn from something that is not there",
                                plan.name
                            ),
                        )
                        .with("pool", plan.name.as_str()));
                    }
                    return Err(short(
                        payer.unwrap_or(entity),
                        &plan.name,
                        plan.amount,
                        have,
                    ));
                };
                let missing = (plan.amount - have) * ratio;
                let already = asked.get(&(entity, from)).copied().unwrap_or(Real::ZERO);
                let reserve = (level(sim, entity, from) - already).max(Real::ZERO);
                if reserve < missing {
                    return Err(short(entity, &pool_name(sim, from), missing, reserve));
                }
                if let Some(payer) = payer {
                    *asked.entry((payer, pool)).or_insert(Real::ZERO) += have;
                }
                *asked.entry((entity, from)).or_insert(Real::ZERO) += missing;
            }
        }
    }
    Ok(())
}

/// What a use spent: pools by payer, and items consumed, for the event
/// and for a refund.
#[derive(Clone, Debug, Default)]
pub(crate) struct Spent {
    pools: Vec<(EntityId, PoolId, Real)>,
    /// By qualified id, what was spent of each, overdraw included.
    by_name: Vec<(String, Real)>,
}

/// Takes every cost, which [`check_costs`] has found can be met: from
/// each pool what it can give, the shortfall from the overdraw pool at
/// its ratio, and the items.
pub(crate) fn pay_costs(sim: &mut Sim, entity: EntityId, plans: &[Plan], cause: Cause) -> Spent {
    let mut spent = Spent::default();
    for plan in plans {
        match plan.kind {
            CostKind::Unknown => {}
            CostKind::Item(item) => {
                let mut left = plan.count;
                for (found, count) in items_held(sim, entity, item) {
                    if left == 0 {
                        break;
                    }
                    let take = count.min(left);
                    left -= take;
                    let gone = if take < count {
                        crate::item::split(sim, found, take, cause)
                    } else {
                        Ok(found)
                    };
                    match gone.and_then(|gone| crate::item::destroy(sim, gone, cause)) {
                        Ok(()) => {}
                        Err(reason) => sim.fault(reason),
                    }
                }
                spent.by_name.push((
                    plan.name.clone(),
                    Real::from_i64(i64::from(plan.count - left)),
                ));
            }
            CostKind::Pool(pool) => {
                let have = plan
                    .payer
                    .map_or(Real::ZERO, |payer| level(sim, payer, pool));
                let from_pool = have.min(plan.amount);
                if let Some(payer) = plan.payer
                    && from_pool > Real::ZERO
                {
                    match sim.pool_spend(payer, pool, from_pool, cause) {
                        Ok(()) => spent.pools.push((payer, pool, from_pool)),
                        Err(reason) => sim.fault(reason),
                    }
                }
                spent.by_name.push((plan.name.clone(), from_pool));
                if from_pool < plan.amount
                    && let Some((from, ratio)) = plan.overdraw
                {
                    let missing = (plan.amount - from_pool) * ratio;
                    match sim.pool_spend(entity, from, missing, cause) {
                        Ok(()) => spent.pools.push((entity, from, missing)),
                        Err(reason) => sim.fault(reason),
                    }
                    spent.by_name.push((pool_name(sim, from), missing));
                }
            }
        }
    }
    let payers: Vec<EntityId> = spent.pools.iter().map(|(payer, ..)| *payer).collect();
    for payer in payers {
        super::modes::reschedule_all(sim, payer);
    }
    spent
}

/// Gives back what [`pay_costs`] took from pools: a use that did not
/// start after all. Items consumed are not made again; they are taken
/// last, after everything that can refuse.
pub(crate) fn refund(sim: &mut Sim, _entity: EntityId, spent: &mut Spent, cause: Cause) {
    for (payer, pool, amount) in spent.pools.drain(..) {
        if let Err(reason) = sim.dose_pool(payer, pool, amount, cause) {
            sim.fault(reason);
        }
        super::modes::reschedule_all(sim, payer);
    }
}

// ---- Availability ---------------------------------------------------------------------------

/// The grant of an action an ability makes, by the action's id; with
/// none named, the ability's only grant.
fn find_grant<'a>(
    sim: &Sim,
    def: AbilityId,
    ability: &'a AbilityDef,
    action: Option<ActionDefId>,
) -> Result<(usize, &'a AbilityGrantDef), Reason> {
    let found = match action {
        Some(action) => ability
            .grants
            .iter()
            .enumerate()
            .find(|(_, grant)| grant.action.id() == action),
        None if ability.grants.len() == 1 => ability.grants.first().map(|grant| (0, grant)),
        None => None,
    };
    found.ok_or_else(|| {
        let name = name_of(&sim.defs, def);
        let action_name = action
            .map(|action| crate::action::run::action_name(&sim.defs, action))
            .unwrap_or_default();
        Reason::new(
            codes::NO_GRANT,
            if action_name.is_empty() {
                format!("the ability `{name}` grants several actions, and none was named")
            } else {
                format!("the ability `{name}` does not grant `{action_name}`")
            },
        )
        .with("ability", name)
        .with("action", action_name)
    })
}

/// How much of the owner's own time is left of a grant's cooldown.
fn cooldown_left(sim: &Sim, entity: EntityId, inst: &AbilityInst, action: ActionDefId) -> Dur {
    let now = sim.own_time(entity);
    inst.cooldowns()
        .iter()
        .find(|cooldown| cooldown.action == action)
        .map_or(Dur::ZERO, |cooldown| {
            (cooldown.ready_at - now).max(Dur::ZERO)
        })
}

/// What a use is, once the instance and the grant are found.
struct Found<'a> {
    inst: AbilityInst,
    ability: &'a AbilityDef,
    index: usize,
    grant: &'a AbilityGrantDef,
    name: String,
}

/// Finds the instance and the grant, and checks everything but the
/// cost: held, active, rank, cooldown, condition.
fn usable<'a>(
    sim: &Sim,
    defs: &'a clash_data::Defs,
    entity: EntityId,
    def: AbilityId,
    action: Option<ActionDefId>,
) -> Result<Found<'a>, Reason> {
    sim.known(entity)?;
    let registry = defs
        .try_registry::<AbilityDef>()
        .ok_or_else(super::ops::not_registered)?;
    let ability = registry
        .try_get(def)
        .ok_or_else(|| super::ops::unknown(def.index().to_string()))?;
    let name = registry.name(def).to_owned();
    let inst = inst_of(sim, entity, def)
        .filter(|inst| inst.is_held())
        .cloned()
        .ok_or_else(|| not_held(sim, entity, def))?;
    let (index, grant) = find_grant(sim, def, ability, action)?;
    if inst.state() == State::Dormant {
        return Err(
            Reason::new(codes::DORMANT, format!("the ability `{name}` is dormant"))
                .with("ability", name.as_str()),
        );
    }
    let rank = super::rank(sim, entity, &inst);
    if let Some(needed) = grant.at_rank
        && rank < needed
    {
        return Err(Reason::new(
            codes::RANK_TOO_LOW,
            format!("`{name}` needs rank {needed} for this, and it is at {rank}"),
        )
        .with("ability", name.as_str())
        .with("rank", i64::from(rank))
        .with("needed", i64::from(needed)));
    }
    let action_id = grant.action.id();
    let left = cooldown_left(sim, entity, &inst, action_id);
    if left > Dur::ZERO {
        let action_name = crate::action::run::action_name(defs, action_id);
        let seconds = left.as_secs_real();
        return Err(Reason::new(
            codes::COOLING_DOWN,
            format!("`{action_name}` of `{name}` is ready again in {seconds} s"),
        )
        .with("ability", name.as_str())
        .with("action", action_name)
        .with("left", seconds));
    }
    if let Some(condition) = &grant.condition {
        let bound = Bound::new()
            .subject(sim.subject_value(Subject::Entity(entity)))
            .subject(handle_value(sim, inst.number()));
        let mut host = sim.host_for_reader(bound, entity);
        if !condition.holds_on(defs, &mut host) {
            let action_name = crate::action::run::action_name(defs, action_id);
            return Err(Reason::new(
                codes::CONDITION,
                format!("`{action_name}` of `{name}` cannot be used now"),
            )
            .with("ability", name.as_str())
            .with("action", action_name));
        }
    }
    Ok(Found {
        inst,
        ability,
        index,
        grant,
        name,
    })
}

/// Whether an entity can use the grant of an action an ability makes
/// now, cost included (08 §5.1): `Ok` if it can, or the reason it
/// cannot. Changes nothing. What the abilities screen and the AI ask.
/// It reads the owner's own state (its pools, its cooldowns, its
/// ability) and a condition written in its mind; whoever a cost is
/// drawn `from` is read as it is.
pub fn available(
    sim: &Sim,
    entity: EntityId,
    def: AbilityId,
    action: Option<ActionDefId>,
) -> Result<(), Reason> {
    let defs = Arc::clone(&sim.defs);
    let found = usable(sim, &defs, entity, def, action)?;
    let tables = Arc::clone(&sim.ability_rt.tables);
    let kinds = tables
        .grant_costs
        .get(def.index())
        .and_then(|grants| grants.get(found.index))
        .cloned()
        .unwrap_or_default();
    let plans = plan_costs(sim, entity, found.inst.number(), &found.grant.cost, &kinds)?;
    check_costs(sim, entity, &plans)
}

/// One way an entity can use an ability now or later: a grant of an
/// ability it holds.
#[derive(Clone, PartialEq, Debug)]
pub struct UseOption {
    /// The ability.
    pub ability: AbilityId,
    /// The instance's number.
    pub number: u64,
    /// The action the grant starts.
    pub action: ActionDefId,
    /// What it is for, if the grant says.
    pub intent: Option<Intent>,
    /// What it costs now.
    pub cost: Vec<CostLine>,
    /// How much of the owner's own time is left of its cooldown.
    pub cooldown_left: Dur,
    /// Why it cannot be used now; `None` if it can.
    pub unavailable: Option<Reason>,
}

/// Every grant of every ability an entity holds, in the order the
/// abilities were gained and the grants written, with what each costs
/// and whether it can be used now: what an abilities screen lists and
/// what an AI chooses among (08 §11).
pub fn options(sim: &Sim, entity: EntityId) -> Vec<UseOption> {
    let defs = Arc::clone(&sim.defs);
    let tables = Arc::clone(&sim.ability_rt.tables);
    let Some(abilities) = sim.components.get::<super::Abilities>(entity) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for inst in abilities.instances().iter().filter(|inst| inst.is_held()) {
        let Some(ability) = def_of(&defs, inst.def()) else {
            continue;
        };
        for (index, grant) in ability.grants.iter().enumerate() {
            let kinds = tables
                .grant_costs
                .get(inst.def().index())
                .and_then(|grants| grants.get(index))
                .cloned()
                .unwrap_or_default();
            let plans = plan_costs(sim, entity, inst.number(), &grant.cost, &kinds);
            let cost = plans
                .as_ref()
                .map(|plans| {
                    plans
                        .iter()
                        .map(|plan| CostLine {
                            what: plan.name.clone(),
                            payer: plan.payer,
                            amount: match plan.kind {
                                CostKind::Item(_) => Real::from_i64(i64::from(plan.count)),
                                CostKind::Pool(_) | CostKind::Unknown => plan.amount,
                            },
                            item: matches!(plan.kind, CostKind::Item(_)),
                            overdraw: plan
                                .overdraw
                                .map(|(pool, ratio)| (pool_name(sim, pool), ratio)),
                        })
                        .collect()
                })
                .unwrap_or_default();
            out.push(UseOption {
                ability: inst.def(),
                number: inst.number(),
                action: grant.action.id(),
                intent: grant.intent,
                cost,
                cooldown_left: cooldown_left(sim, entity, inst, grant.action.id()),
                unavailable: available(sim, entity, inst.def(), Some(grant.action.id())).err(),
            });
        }
    }
    out
}

// ---- Using ----------------------------------------------------------------------------------

/// The value of a map of names to numbers, as an event's field holds it.
fn cost_field(sim: &Sim, costs: &[(String, Real)]) -> FieldValue {
    let map = Value::map(
        costs
            .iter()
            .map(|(name, amount)| (name.clone(), Value::Real(*amount))),
    );
    sim.field_of(&map)
}

/// Raises `ability_refused` for a use that was refused after something
/// may have changed, or that a rule asked for.
pub(crate) fn refused(
    sim: &mut Sim,
    entity: EntityId,
    ability: &str,
    action: &str,
    reason: &Reason,
    cause: Cause,
) {
    notify(
        sim,
        entity,
        EventPayload::new(ABILITY_REFUSED)
            .with("ability", ability)
            .with("action", action)
            .with("reason", reason.code.as_str())
            .with("message", reason.message.as_str()),
        cause,
    );
}

/// Uses an ability (08 §5): starts the action one of its grants makes
/// available, through the action runtime.
///
/// 1. The instance is found and checked: held, not dormant, the grant's
///    `at_rank`, its cooldown, its `if`, and that every cost can be met.
/// 2. The parameters are those given, with the grant's `params` worked
///    out for the instance standing in for any of the same name.
/// 3. The hook `ability_using`, if anything listens: a handler may
///    change the parameters and the cost for this use, or cancel. The
///    cost is checked again.
/// 4. The action is prepared: everything the action runtime checks.
/// 5. The cost is taken in full (invariant A4), the cooldown starts on
///    the owner's own time, the action starts and `ability_used` is
///    raised.
///
/// `Err` is a refusal before anything changed. `Ok(None)` is a use
/// refused after the hook ran (`ability_refused` was raised, nothing
/// was spent), or an action a handler of `action_starting` refused
/// (what was spent is given back). `Ok(Some(id))` is the action that
/// started.
pub fn use_ability(
    sim: &mut Sim,
    entity: EntityId,
    def: AbilityId,
    action: Option<ActionDefId>,
    params: &Params,
    cause: Cause,
) -> Result<Option<ActionId>, Reason> {
    let defs = Arc::clone(&sim.defs);
    let found = usable(sim, &defs, entity, def, action)?;
    let number = found.inst.number();
    let action_id = found.grant.action.id();
    let action_name = crate::action::run::action_name(&defs, action_id);
    let tables = Arc::clone(&sim.ability_rt.tables);
    let kinds = tables
        .grant_costs
        .get(def.index())
        .and_then(|grants| grants.get(found.index))
        .cloned()
        .unwrap_or_default();
    let mut plans = plan_costs(sim, entity, number, &found.grant.cost, &kinds)?;
    check_costs(sim, entity, &plans)?;

    // The parameters: what was given, then the grant's own.
    let mut values: Vec<(String, Value)> = params
        .iter()
        .map(|(name, value)| (name.clone(), Value::from(value)))
        .collect();
    if !found.grant.params.is_empty() {
        let bound = Bound::new()
            .subject(sim.subject_value(Subject::Entity(entity)))
            .subject(handle_value(sim, number));
        let mut host = sim.host(bound);
        let mut runner = clash_data::Runner::new();
        for (name, expr) in &found.grant.params {
            let value = expr.value(&mut runner.on(&defs, &mut host));
            values.retain(|(have, _)| have != name);
            values.push((name.clone(), value));
        }
    }

    // The hook.
    if sim.bus.has_listeners(ABILITY_USING) {
        let params_value = Value::map(values.iter().cloned());
        let cost_value = Value::map(
            plans
                .iter()
                .filter(|plan| !matches!(plan.kind, CostKind::Unknown))
                .map(|plan| {
                    let amount = match plan.kind {
                        CostKind::Item(_) => Real::from_i64(i64::from(plan.count)),
                        CostKind::Pool(_) | CostKind::Unknown => plan.amount,
                    };
                    (plan.name.clone(), Value::Real(amount))
                }),
        );
        let payload = EventPayload::new(ABILITY_USING)
            .subject("self", Subject::Entity(entity))
            .with("ability", found.name.as_str())
            .with("action", action_name.as_str())
            .with("params", sim.field_of(&params_value))
            .with("cost", sim.field_of(&cost_value));
        let raised = match sim.raise(Where::Entity(entity), cause, payload) {
            Ok(raised) => raised,
            Err(reason) => {
                refused(sim, entity, &found.name, &action_name, &reason, cause);
                return Ok(None);
            }
        };
        if let Some(reason) = raised.cancelled {
            refused(sim, entity, &found.name, &action_name, &reason, cause);
            return Ok(None);
        }
        if let Some(changed) = raised.field("params") {
            let changed = sim.exprs.field_value(changed);
            if let Some(map) = changed.as_map() {
                values = map
                    .iter()
                    .map(|(name, value)| (name.to_string(), value.clone()))
                    .collect();
            }
        }
        if let Some(changed) = raised.field("cost") {
            let changed = sim.exprs.field_value(changed);
            if let Some(map) = changed.as_map() {
                for plan in &mut plans {
                    if let Some(amount) = map.get(plan.name.as_str()).and_then(Value::as_number) {
                        let amount = if amount.is_finite() {
                            amount.max(Real::ZERO)
                        } else {
                            Real::ZERO
                        };
                        match plan.kind {
                            CostKind::Item(_) => {
                                plan.count = u32::try_from(amount.to_i64()).unwrap_or(u32::MAX);
                            }
                            CostKind::Pool(_) | CostKind::Unknown => plan.amount = amount,
                        }
                    }
                }
            }
        }
        if let Err(reason) = check_costs(sim, entity, &plans) {
            refused(sim, entity, &found.name, &action_name, &reason, cause);
            return Ok(None);
        }
    }

    // The action, prepared by the action runtime.
    let mut given = Params::new();
    for (name, value) in &values {
        given.push((name.clone(), value.to_stored()));
    }
    let streams = sim.rng.clone();
    let prepared = match sim.prepare(entity, action_id, &given, true) {
        Ok(prepared) => prepared,
        Err(reason) => {
            sim.rng = streams;
            if sim.bus.has_listeners(ABILITY_USING) {
                refused(sim, entity, &found.name, &action_name, &reason, cause);
                return Ok(None);
            }
            return Err(reason);
        }
    };

    // The cost, the cooldown, the start.
    let mut spent = pay_costs(sim, entity, &plans, cause);
    let cooldown = found.grant.cooldown.as_ref().and_then(|cooldown| {
        let bound = Bound::new()
            .subject(sim.subject_value(Subject::Entity(entity)))
            .subject(handle_value(sim, number));
        let mut host = sim.host(bound);
        cooldown
            .eval_on(&defs, &mut host)
            .filter(|seconds| seconds.is_finite() && *seconds > Real::ZERO)
    });
    let previous = found
        .inst
        .cooldowns()
        .iter()
        .find(|have| have.action == action_id)
        .copied();
    if let Some(seconds) = cooldown {
        let ready_at = sim.own_time(entity) + Dur::from_secs_real(seconds);
        if let Some(inst) = sim
            .components
            .get_mut::<super::Abilities>(entity)
            .and_then(|abilities| abilities.by_number_mut(number))
        {
            inst.cooldowns.retain(|have| have.action != action_id);
            inst.cooldowns.push(super::Cooldown {
                action: action_id,
                ready_at,
            });
        }
    }
    match sim.begin(prepared, cause) {
        Ok(started) => {
            notify(
                sim,
                entity,
                EventPayload::new(ABILITY_USED)
                    .with("ability", found.name.as_str())
                    .with("action", action_name.as_str())
                    .with("cost", cost_field(sim, &spent.by_name)),
                cause,
            );
            let _ = &found.ability;
            Ok(Some(started))
        }
        Err(_) => {
            // `action_refused` was raised: nothing is spent after all.
            refund(sim, entity, &mut spent, cause);
            if let Some(inst) = sim
                .components
                .get_mut::<super::Abilities>(entity)
                .and_then(|abilities| abilities.by_number_mut(number))
            {
                inst.cooldowns.retain(|have| have.action != action_id);
                if let Some(previous) = previous {
                    inst.cooldowns.push(previous);
                }
            }
            Ok(None)
        }
    }
}

impl Sim {
    /// The `UseAbility` command: the controlled entity uses an ability
    /// ([`use_ability`]). Refused as `Act` is for an entity nobody
    /// controls or whose mind has stopped, and as `use_ability` is. A
    /// use refused after the hook ran was taken: `ability_refused` was
    /// raised.
    pub(crate) fn command_use_ability(
        &mut self,
        entity: EntityId,
        ability: AbilityId,
        action: Option<ActionDefId>,
        params: &Params,
    ) -> Result<(), Reason> {
        self.commanded(entity)?;
        let cause = Cause {
            actor: Some(entity),
            instigator: Some(entity),
            kind: clash_world::CauseKind::Action,
        };
        use_ability(self, entity, ability, action, params, cause)?;
        self.answer_which(entity, |kind| {
            matches!(kind, crate::decision::DecisionKind::Free { .. })
        });
        Ok(())
    }

    /// The `SetMode` command: the controlled entity switches a mode of
    /// an ability on or off ([`start_mode`](super::start_mode),
    /// [`end_mode`](super::end_mode)).
    pub(crate) fn command_set_mode(
        &mut self,
        entity: EntityId,
        ability: AbilityId,
        mode: &str,
        on: bool,
    ) -> Result<(), Reason> {
        self.commanded(entity)?;
        let cause = Cause {
            actor: Some(entity),
            instigator: Some(entity),
            kind: clash_world::CauseKind::Action,
        };
        if on {
            super::start_mode(self, entity, ability, mode, cause)
        } else {
            super::end_mode(self, entity, ability, mode, cause)
        }
    }

    /// Whether commands may address an entity: someone controls it and
    /// its mind lets it act.
    fn commanded(&self, entity: EntityId) -> Result<(), Reason> {
        let control = self.known(entity)?.control();
        if control == crate::entity::Control::None {
            return Err(Reason::new(
                crate::action::run::COMMAND_NOT_CONTROLLED,
                format!("nobody controls entity {entity}, so it takes no commands"),
            )
            .with("entity", entity.to_string()));
        }
        self.life_lets_act(entity)
    }
}
