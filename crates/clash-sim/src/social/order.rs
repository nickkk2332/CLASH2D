//! Orders (15 §10): what one tells another to do, said aloud, and kept
//! by whoever hears it and obeys.
//!
//! ```text
//! order { from, to, action, with?, until? }
//!   → said as a message of a kind that carries orders (15 §11)
//!   → heard by `to`: the hook `order_received` (may refuse, or decide `obeys`)
//!   → kept in `to`'s orders, and `order_given` raised; or `order_refused`
//! ```
//!
//! **Who is obeyed.** The one an order is given to obeys it when it holds
//! toward the one it takes for the giver a relation of a kind flagged
//! `orders` above its default (15 §10): a servant made with such a
//! relation toward its master. It is the hearer's relation, read through
//! the persona it heard, so a voice taken for the master's is obeyed.
//! Whether it obeys is still the hook's: a handler may set `obeys`, or
//! cancel with a reason.
//!
//! **Doing it.** The engine keeps the order; what the character does
//! about it is a goal of the AI (09 §8.1), or until then a rule on
//! `order_given` (the bundled `core:do_as_ordered` puts the action in
//! the plan).
//!
//! **Ending.** `cancel_order` withdraws one; `until` ends it by itself
//! (an order past its time is no longer standing, and is dropped when
//! the orders are next changed); and when the relation that made it one
//! to obey falls to its default, the orders kept from that giver are
//! withdrawn (`order_cancelled` with `how` `relation_lost`).

use clash_core::{Dur, EntityId, StateHash, StateHasher, Tick, impl_state_hash};
use clash_world::Cause;
use serde::{Deserialize, Serialize};

use super::reason as why;
use super::relation::{Toward, holds_flagged_toward};
use super::speech::{Message, Report};
use super::{events, mind, mind_mut, tidy};
use crate::Sim;
use crate::action::{ActionDefId, Params};
use crate::bus::{EventPayload, FieldValue};
use crate::event::{Subject, Where};
use crate::perception::PersonaId;
use crate::reason::Reason;

/// What an order tells someone to do: an action with its parameters, and
/// for how long the order stands.
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub struct OrderPlan {
    /// The action.
    pub action: ActionDefId,
    /// Its parameters, by name.
    pub params: Params,
    /// How long the order stands once received, in universal time; for
    /// good if `None`.
    pub until: Option<Dur>,
}

impl StateHash for OrderPlan {
    fn state_hash(&self, hasher: &mut StateHasher) {
        self.action.state_hash(hasher);
        self.params.state_hash(hasher);
        self.until.state_hash(hasher);
    }
}

/// An order kept by whoever obeys it (15 §10).
#[derive(Clone, PartialEq, Debug, Serialize, Deserialize)]
pub struct Order {
    /// Its number, given once in the simulation's life.
    pub id: u64,
    /// Whom the one who keeps it took for the giver.
    pub from: PersonaId,
    /// Who gave it. Truth: kept so that the giver can withdraw it; what
    /// the character decides by is `from`.
    pub giver: EntityId,
    /// What it says to do.
    pub plan: OrderPlan,
    /// When it was received, in universal time.
    pub received: Tick,
    /// When it stops standing, in universal time, if it does.
    pub ends: Option<Tick>,
}

impl_state_hash!(Order {
    id,
    from,
    giver,
    plan,
    received,
    ends
});

impl Order {
    /// Whether it still stands at a time.
    pub fn stands_at(&self, now: Tick) -> bool {
        self.ends.is_none_or(|ends| now < ends)
    }
}

/// The orders an entity keeps that still stand, in the order received.
pub fn standing_orders(sim: &Sim, who: EntityId) -> Vec<&Order> {
    let now = sim.now();
    mind(sim, who).map_or_else(Vec::new, |held| {
        held.orders
            .iter()
            .filter(|order| order.stands_at(now))
            .collect()
    })
}

/// Gives an order (`order`, 15 §10): says it aloud from `from` to `to` as
/// a message of a kind that carries orders. Returns how many heard it
/// said, as [`say`](super::say) does; whether it is obeyed is settled
/// when it is heard.
pub fn give_order(
    sim: &mut Sim,
    from: EntityId,
    args: super::SayArgs,
    plan: OrderPlan,
    cause: Cause,
) -> Result<u32, Reason> {
    let kind = args.kind;
    let carries = sim
        .defs
        .try_registry::<super::MessageKindDef>()
        .and_then(|registry| registry.try_get(kind))
        .map(|def| def.carries);
    if carries != Some(super::Carries::Order) {
        let name = sim
            .defs
            .try_registry::<super::MessageKindDef>()
            .and_then(|registry| {
                registry
                    .try_get(kind)
                    .map(|_| registry.name(kind).to_owned())
            })
            .unwrap_or_default();
        return Err(Reason::new(
            why::NOT_AN_ORDER,
            format!("the message kind `{name}` does not carry orders"),
        )
        .with("kind", name));
    }
    super::speech::say_with(sim, from, args, Report::default(), Some(plan), cause)
}

/// An order was heard by the one it was given to (15 §10).
pub(crate) fn received(
    sim: &mut Sim,
    hearer: EntityId,
    message: &Message,
    giver: Option<EntityId>,
    plan: &OrderPlan,
    cause: Cause,
) {
    let action = action_value(sim, plan.action);
    let Some(giver) = giver else {
        let reason = Reason::new(
            why::GIVER_UNKNOWN,
            "an order was heard from someone not made out",
        )
        .with("entity", hearer.to_string());
        events::order_refused(sim, hearer, None, action, &reason, cause);
        return;
    };
    let from = super::persona_of(sim, hearer, giver);
    let obeys = holds_flagged_toward(sim, hearer, &Toward::Persona(from), |def| def.orders);
    let payload = EventPayload::new(events::ORDER_RECEIVED)
        .subject("self", Subject::Entity(hearer))
        .subject("giver", Subject::Entity(giver))
        .with("action", FieldValue::Plain(action.clone()))
        .with(
            "with",
            FieldValue::Plain(clash_core::Value::Map(
                plan.params.iter().cloned().collect(),
            )),
        )
        .with("obeys", obeys);
    let (obeys, refused) = match sim.raise(Where::Entity(hearer), cause, payload) {
        Ok(raised) => match raised.cancelled {
            Some(reason) => (false, Some(reason)),
            None => (
                raised
                    .field("obeys")
                    .and_then(|value| value.plain())
                    .and_then(clash_core::Value::as_bool)
                    .unwrap_or(obeys),
                None,
            ),
        },
        Err(reason) => {
            sim.fault(reason);
            (obeys, None)
        }
    };
    if !obeys {
        let reason = refused.unwrap_or_else(|| {
            Reason::new(
                why::NOT_OBEYED,
                "the hearer holds nothing toward the giver that makes an order one to obey",
            )
            .with("entity", hearer.to_string())
        });
        events::order_refused(sim, hearer, Some(giver), action, &reason, cause);
        return;
    }
    prune(sim, hearer, cause);
    sim.social.next_order += 1;
    let now = sim.now();
    let order = Order {
        id: sim.social.next_order,
        from,
        giver,
        plan: plan.clone(),
        received: now,
        ends: plan.until.map(|until| now + until),
    };
    let _ = message;
    mind_mut(sim, hearer).orders.push(order.clone());
    events::order_given(sim, hearer, &order, action, cause);
}

/// The action of an order as a value: the definition's id.
fn action_value(sim: &Sim, action: ActionDefId) -> clash_core::Value {
    match sim.exprs().registry().def_type_id("action") {
        Some(ty) => clash_core::Value::Id {
            ty: ty.raw(),
            index: action.raw(),
        },
        None => clash_core::Value::Null,
    }
}

/// Drops the orders of an entity that no longer stand, with
/// `order_cancelled` and `how` `expired`.
fn prune(sim: &mut Sim, who: EntityId, cause: Cause) {
    let now = sim.now();
    let Some(held) = sim.social.minds.get_mut(&who) else {
        return;
    };
    let (kept, gone): (Vec<Order>, Vec<Order>) = held
        .orders
        .drain(..)
        .partition(|order| order.stands_at(now));
    held.orders = kept;
    for order in gone {
        events::order_cancelled(sim, who, order.id, "expired", cause);
    }
    tidy(sim, who);
}

/// Withdraws an order an entity keeps, or all of them (`cancel_order`).
/// Raises `order_cancelled` with `how` `cancelled` for each. Returns how
/// many went. Refused for an order it does not keep.
pub fn cancel_order(
    sim: &mut Sim,
    who: EntityId,
    order: Option<u64>,
    cause: Cause,
) -> Result<usize, Reason> {
    sim.known(who)?;
    let ids: Vec<u64> = mind(sim, who)
        .map(|held| {
            held.orders
                .iter()
                .map(|kept| kept.id)
                .filter(|id| order.is_none_or(|order| order == *id))
                .collect()
        })
        .unwrap_or_default();
    if let (Some(order), true) = (order, ids.is_empty()) {
        return Err(
            Reason::new(why::UNKNOWN_ORDER, format!("{who} keeps no order {order}"))
                .with("entity", who.to_string())
                .with("order", i64::try_from(order).unwrap_or(i64::MAX)),
        );
    }
    withdraw(sim, who, &ids, "cancelled", cause);
    Ok(ids.len())
}

/// Withdraws the orders of these numbers.
fn withdraw(sim: &mut Sim, who: EntityId, ids: &[u64], how: &str, cause: Cause) {
    if let Some(held) = sim.social.minds.get_mut(&who) {
        held.orders.retain(|order| !ids.contains(&order.id));
    }
    tidy(sim, who);
    for id in ids {
        events::order_cancelled(sim, who, *id, how, cause);
    }
}

/// A relation flagged `orders` that `who` held toward someone fell to its
/// default: the orders it keeps from them that it no longer has reason to
/// obey are withdrawn, with `how` `relation_lost`.
pub(crate) fn relation_lost(sim: &mut Sim, who: EntityId, toward: &Toward, cause: Cause) {
    let ids: Vec<u64> = mind(sim, who)
        .map(|held| {
            held.orders
                .iter()
                .filter(|order| match toward {
                    Toward::Persona(persona) => order.from == *persona,
                    Toward::Group(key) => super::claims_of(sim, order.from).contains(key),
                })
                .filter(|order| {
                    !holds_flagged_toward(sim, who, &Toward::Persona(order.from), |def| def.orders)
                })
                .map(|order| order.id)
                .collect()
        })
        .unwrap_or_default();
    withdraw(sim, who, &ids, "relation_lost", cause);
}

/// The number of an order as a value rules read.
pub(crate) fn id_value(id: u64) -> i64 {
    i64::try_from(id).unwrap_or(i64::MAX)
}
