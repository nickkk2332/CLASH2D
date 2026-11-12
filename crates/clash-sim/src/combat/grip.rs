//! Grips (07 §9): one creature holding another, for as long as the
//! action that took hold goes on.
//!
//! A grip is taken by the verb `seize`, run from one of the holder's
//! actions: usually the `on_land` of an attack with little or no energy,
//! so that the grab is resolved like a strike (reach, aim, defences)
//! before anything is held. The grip belongs to that action, the
//! **maintaining** action:
//!
//! - it ends when the action ends, for any reason (invariant C7);
//! - it ends earlier by `release_grip` or by a `throw` of what is held;
//!   one ended by anything but the maintaining action's own lists
//!   interrupts that action, so that a grip never outlives the action
//!   and the action never holds what is no longer held;
//! - it is lost, interrupting the action, when the held is no longer
//!   beside the holder: knocked away, launched, taken out of the world.
//!
//! While it lasts:
//!
//! - **Pinning.** A grip that `pins` a part disables the channel slots
//!   the part and the parts on it supply (03 §6.2), and gives them back
//!   when it ends: a pinned arm's actions are refused, with the reason
//!   the runtime gives for slots lacking.
//! - **Dragging.** When the holder steps, the held is moved into the
//!   cell the holder left. A step by a holder gripping something heavier
//!   than the grip's `drags` is refused (`combat.too_heavy`); a step by
//!   the held to anywhere not beside its holder is refused
//!   (`combat.gripped`): breaking free is an action of content's that
//!   ends in `release_grip`.
//! - **Using it.** An attack or a defence whose `with` is
//!   `{ "gripped": true }` strikes with, or puts in the way, the first
//!   creature the actor grips (07 §9.2): a human shield takes the blow
//!   on its body. `throw` launches a creature its thrower grips as a
//!   body in flight at `√(2 E / m)`.
//!
//! What is kept is plain state, saved and hashed with the simulation
//! once anything has been gripped.

use clash_core::{EntityId, Real, StateHash, StateHasher, impl_state_hash};
use clash_expr::Value;
use clash_world::Cause;
use serde::{Deserialize, Serialize};

use super::events::CombatEvent;
use super::strike::gap;
use crate::Sim;
use crate::action::ChannelId;
use crate::body::PartKey;
use crate::body::props::{part_handle, part_of_handle};
use crate::bus::{EventPayload, FieldValue};
use crate::entity::Place;
use crate::event::{EventBody, SimEvent, Subject, Where};
use crate::later::ActionId;
use crate::life::events::{field_handle, handle_field};
use crate::reason::{Reason, ReasonInfo};
use crate::registry::PropertyDef;
use crate::rule::VerbCall;
use crate::verb::VerbDef;
use crate::world_verbs::{arg, spec};

/// `seize` was run outside an action of the holder's: a grip belongs to
/// the action that keeps it.
pub const NO_ACTION: &str = "combat.no_action";
/// What was to be seized is not within the holder's reach.
pub const OUT_OF_REACH: &str = "combat.out_of_reach";
/// A handler of `seize_resolving` made a grip fail.
pub const SEIZE_FAILED: &str = "combat.seize_failed";
/// A creature held tried to step away from its holder.
pub const GRIPPED: &str = "combat.gripped";
/// A holder tried to step dragging more than it can.
pub const TOO_HEAVY: &str = "combat.too_heavy";
/// The held is no longer beside its holder: the grip is lost.
pub const GRIP_LOST: &str = "combat.grip_lost";
/// The held broke free (`release_grip` by the held).
pub const GRIP_BROKEN: &str = "combat.grip_broken";
/// The holder let go by something other than the maintaining action.
pub const GRIP_RELEASED: &str = "combat.grip_released";
/// A creature to be thrown is not one the thrower grips.
pub const NOT_GRIPPED: &str = "combat.not_gripped";

/// Raised when a grip is taken.
pub const SEIZED: &str = "seized";
/// Raised when a grip ends.
pub const GRIP_ENDED: &str = "grip_broken";
/// The hook raised before a grip is taken.
pub const SEIZE_RESOLVING: &str = "seize_resolving";

/// One creature holding another (07 §9.1).
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub struct Grip {
    /// Who holds.
    pub holder: EntityId,
    /// Who or what is held.
    pub held: EntityId,
    /// The part held, if a particular one.
    pub on: Option<PartKey>,
    /// The holder's action that maintains it.
    pub action: ActionId,
    /// Whether it pins the part held.
    pub pins: bool,
    /// The slots the pin disabled on the held, by channel, to give back.
    pub pinned: Vec<(ChannelId, u16)>,
    /// The most mass, in kilograms, the holder drags with it; `None`
    /// for any.
    pub drags: Option<Real>,
}

impl_state_hash!(Grip {
    holder,
    held,
    on,
    action,
    pins,
    pinned,
    drags
});

/// Every grip, in the order taken. Kept by the simulation; changed only
/// here.
#[derive(Clone, PartialEq, Eq, Default, Debug, Serialize, Deserialize)]
pub struct GripTable {
    grips: Vec<Grip>,
}

impl StateHash for GripTable {
    fn state_hash(&self, hasher: &mut StateHasher) {
        self.grips.state_hash(hasher);
    }
}

impl GripTable {
    /// Whether nothing is gripped: then nothing is hashed.
    pub fn is_empty(&self) -> bool {
        self.grips.is_empty()
    }

    /// Every grip, in the order taken.
    pub fn iter(&self) -> impl Iterator<Item = &Grip> {
        self.grips.iter()
    }

    /// The grips a holder has, in the order taken.
    pub fn of_holder(&self, holder: EntityId) -> impl Iterator<Item = &Grip> {
        self.grips.iter().filter(move |grip| grip.holder == holder)
    }

    /// The grips on a creature, in the order taken.
    pub fn on_held(&self, held: EntityId) -> impl Iterator<Item = &Grip> {
        self.grips.iter().filter(move |grip| grip.held == held)
    }

    /// The grips an action maintains.
    pub fn of_action(&self, action: ActionId) -> impl Iterator<Item = &Grip> {
        self.grips.iter().filter(move |grip| grip.action == action)
    }
}

/// What a holder grips, in the order taken.
pub fn gripped_by_holder(sim: &Sim, holder: EntityId) -> Vec<EntityId> {
    sim.grips.of_holder(holder).map(|grip| grip.held).collect()
}

/// Who grips a creature, the first to have taken hold.
pub fn holder_of(sim: &Sim, held: EntityId) -> Option<EntityId> {
    sim.grips.on_held(held).next().map(|grip| grip.holder)
}

/// How `seize` is asked to grip.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Seize {
    /// Who takes hold.
    pub holder: EntityId,
    /// Who or what is held.
    pub held: EntityId,
    /// The part held.
    pub on: Option<PartKey>,
    /// Whether it pins that part.
    pub pins: bool,
    /// The most mass the holder drags with it, in kilograms.
    pub drags: Option<Real>,
}

fn refuse(code: &'static str, message: impl Into<String>) -> Reason {
    Reason::new(code, message)
}

/// The action of the holder's whose list is running, if one is.
fn maintaining(sim: &Sim, holder: EntityId) -> Option<ActionId> {
    let action = sim.acting?;
    (sim.actions.get(action)?.actor == holder).then_some(action)
}

/// How far a holder reaches: its body's reach, or nothing beyond a
/// neighbour for one with no body.
fn reach_of(sim: &Sim, holder: EntityId) -> Real {
    match (sim.components().body.get(holder), sim.body_defs()) {
        (Some(body), Some(defs)) => body.reach(defs),
        _ => Real::ZERO,
    }
}

/// The slots of a channel a part and the parts on it supply, by channel.
fn part_slots(sim: &Sim, held: EntityId, part: PartKey) -> Vec<(ChannelId, u16)> {
    let (Some(body), Some(defs)) = (sim.components().body.get(held), sim.body_defs()) else {
        return Vec::new();
    };
    let mut keys = vec![part];
    let mut at = 0;
    while let Some(key) = keys.get(at).copied() {
        for child in body.parts().iter().filter(|child| {
            child.parent() == Some(key)
                && child.is_present()
                && child.relation() == crate::body::Relation::On
        }) {
            if !keys.contains(&child.key()) {
                keys.push(child.key());
            }
        }
        at += 1;
    }
    let mut slots: Vec<(ChannelId, u16)> = Vec::new();
    for key in keys {
        let Some(found) = body.part(key).filter(|found| found.is_working()) else {
            continue;
        };
        for (channel, count) in defs.channels_of(found.def()) {
            match slots.iter_mut().find(|(have, _)| *have == channel) {
                Some((_, total)) => *total = total.saturating_add(count),
                None => slots.push((channel, count)),
            }
        }
    }
    slots.sort_unstable();
    slots
}

/// A part as an event's field.
fn part_field(sim: &Sim, owner: EntityId, part: Option<PartKey>) -> FieldValue {
    match (part, sim.body_defs()) {
        (Some(key), Some(defs)) => handle_field(part_handle(defs.kinds(), owner, key)),
        _ => FieldValue::Null,
    }
}

/// Takes hold (07 §9.1): see the [module documentation](self). The grip
/// belongs to the holder's action whose list runs this.
///
/// Refused, having changed nothing, when no action of the holder's is
/// running the list (`combat.no_action`), when what is to be held is
/// the holder itself or not in the world (`combat.no_target`), when it
/// is out of the holder's reach (`combat.out_of_reach`), and when a
/// handler of `seize_resolving` cancels (`combat.seize_failed`).
pub fn seize(sim: &mut Sim, ask: &Seize, cause: Cause) -> Result<(), Reason> {
    let Seize {
        holder, held, on, ..
    } = *ask;
    sim.known(holder)?;
    let action = maintaining(sim, holder).ok_or_else(|| {
        refuse(
            NO_ACTION,
            "a grip is kept by the action that takes it, and `seize` was run outside one of the holder's",
        )
        .with("holder", holder.to_string())
    })?;
    if held == holder || sim.truth().location(held).is_none() {
        return Err(refuse(
            super::reason::NO_TARGET,
            "what is to be seized is not something else in the world",
        )
        .with("target", held.to_string()));
    }
    if on.is_some_and(|part| {
        sim.components()
            .body
            .get(held)
            .and_then(|body| body.part(part))
            .is_none_or(|found| !found.is_present())
    }) {
        return Err(refuse(
            super::reason::NOT_ITS_PART,
            format!("the part to be seized is not a part of {held}"),
        )
        .with("target", held.to_string()));
    }
    let reach = reach_of(sim, holder);
    if gap(sim, holder, held).is_none_or(|gap| gap > reach) {
        return Err(refuse(OUT_OF_REACH, format!("{held} is out of reach"))
            .with("target", held.to_string())
            .with("reach", reach));
    }
    if sim
        .grips
        .of_action(action)
        .any(|grip| grip.held == held)
    {
        return Ok(());
    }
    // The hook: a handler may make it fail, or change what it pins.
    let mut on = on;
    let mut pins = ask.pins;
    if sim.bus.has_listeners(SEIZE_RESOLVING) {
        let payload = EventPayload::new(SEIZE_RESOLVING)
            .subject("self", Subject::Entity(holder))
            .subject("target", Subject::Entity(held))
            .with("part", part_field(sim, held, on))
            .with("pins", pins);
        let raised = sim.raise(Where::Entity(holder), cause, payload)?;
        if let Some(why) = raised.cancelled {
            return Err(refuse(SEIZE_FAILED, why.message.clone()).with("target", held.to_string()));
        }
        if let Some(FieldValue::Plain(clash_core::Value::Bool(changed))) = raised.field("pins") {
            pins = *changed;
        }
        match field_handle(raised.field("part")) {
            Some(handle) => {
                on = part_of_handle(&sim.components().body, handle)
                    .filter(|(owner, _)| *owner == held)
                    .map(|(_, key)| key);
            }
            None => {
                if matches!(raised.field("part"), Some(FieldValue::Null)) {
                    on = None;
                }
            }
        }
    }
    let pinned = match (pins, on) {
        (true, Some(part)) => part_slots(sim, held, part),
        _ => Vec::new(),
    };
    if let Some(record) = sim.entity_mut(held) {
        for (channel, count) in &pinned {
            record.channels.change(*channel, i32::from(*count));
        }
    }
    sim.grips.grips.push(Grip {
        holder,
        held,
        on,
        action,
        pins,
        pinned: pinned.clone(),
        drags: ask.drags,
    });
    let payload = EventPayload::new(SEIZED)
        .subject("self", Subject::Entity(holder))
        .subject("target", Subject::Entity(held))
        .with("part", part_field(sim, held, on))
        .with("pins", pins)
        .with("action", i64::from(action.raw()));
    if let Err(reason) = sim.raise(Where::Entity(holder), cause, payload) {
        sim.fault(reason);
    }
    // What the pin took away interrupts what used it (03 §6.2).
    if !pinned.is_empty() {
        sim.check_slots(held, cause);
    }
    Ok(())
}

/// Ends the grips that match, giving back what they pinned and raising
/// `grip_broken` with how; the maintaining action of each is
/// interrupted with `interrupt` unless it is the one whose list is
/// running. Returns how many ended.
fn end_where(
    sim: &mut Sim,
    matches: impl Fn(&Grip) -> bool,
    how: &str,
    interrupt: Option<&'static str>,
    cause: Cause,
) -> usize {
    let (ended, kept): (Vec<Grip>, Vec<Grip>) =
        std::mem::take(&mut sim.grips.grips).into_iter().partition(|grip| matches(grip));
    sim.grips.grips = kept;
    for grip in &ended {
        if let Some(record) = sim.entity_mut(grip.held) {
            for (channel, count) in &grip.pinned {
                record.channels.change(*channel, -i32::from(*count));
            }
        }
        let mut payload = EventPayload::new(GRIP_ENDED)
            .subject("self", Subject::Entity(grip.holder))
            .with("part", part_field(sim, grip.held, grip.on))
            .with("how", how);
        if sim.truth().entity(grip.held).is_some() {
            payload = payload.subject("target", Subject::Entity(grip.held));
        }
        if sim.truth().entity(grip.holder).is_some()
            && let Err(reason) = sim.raise(Where::Entity(grip.holder), cause, payload)
        {
            sim.fault(reason);
        }
    }
    if let Some(code) = interrupt {
        for grip in &ended {
            if sim.acting == Some(grip.action) || sim.actions.get(grip.action).is_none() {
                continue;
            }
            let why = refuse(code, format!("the grip on {} ended", grip.held))
                .with("target", grip.held.to_string());
            sim.interrupt_action(grip.action, why, cause);
        }
    }
    ended.len()
}

/// Ends the grips an action maintains: what the action runtime calls as
/// any action ends (invariant C7). Nothing is raised for an action that
/// kept none.
pub(crate) fn action_ended(sim: &mut Sim, action: ActionId) {
    if sim.grips.of_action(action).next().is_none() {
        return;
    }
    let cause = Cause::new(clash_world::CauseKind::Action);
    end_where(sim, |grip| grip.action == action, "ended", None, cause);
}

/// Lets go (07 §9.1, `release_grip`): ends every grip `who` holds or is
/// held by, only those with `other` if it is given. A grip ended by
/// anything but its maintaining action interrupts that action: with
/// `combat.grip_broken` when the held broke free, `combat.grip_released`
/// when the holder let go. Returns how many ended.
pub fn release_grip(sim: &mut Sim, who: EntityId, other: Option<EntityId>, cause: Cause) -> usize {
    let as_holder = end_where(
        sim,
        |grip| grip.holder == who && other.is_none_or(|other| grip.held == other),
        "released",
        Some(GRIP_RELEASED),
        cause,
    );
    let as_held = end_where(
        sim,
        |grip| grip.held == who && other.is_none_or(|other| grip.holder == other),
        "broken",
        Some(GRIP_BROKEN),
        cause,
    );
    as_holder + as_held
}

/// Ends the grip of a holder on a creature it throws: the grip ends with
/// how `thrown`. Refused for a creature the thrower does not grip
/// (`combat.not_gripped`).
pub(crate) fn let_fly(sim: &mut Sim, thrower: EntityId, held: EntityId, cause: Cause) -> Result<(), Reason> {
    if !sim
        .grips
        .of_holder(thrower)
        .any(|grip| grip.held == held)
    {
        return Err(refuse(NOT_GRIPPED, format!("{thrower} does not grip {held}"))
            .with("who", thrower.to_string())
            .with("what", held.to_string()));
    }
    end_where(
        sim,
        |grip| grip.holder == thrower && grip.held == held,
        "thrown",
        Some(GRIP_RELEASED),
        cause,
    );
    Ok(())
}

/// Whether a creature grips another.
pub fn grips(sim: &Sim, holder: EntityId, held: EntityId) -> bool {
    sim.grips.of_holder(holder).any(|grip| grip.held == held)
}

/// Whether two cells are beside each other on one level, or the same.
fn beside(sim: &Sim, a: clash_world::Location, b: clash_world::Location) -> bool {
    sim.truth()
        .cosmos()
        .delta(a, b)
        .is_some_and(|delta| delta.x.abs() <= 1 && delta.y.abs() <= 1 && delta.z == 0)
}

/// What a step by an entity to a cell must allow, by its grips
/// (07 §9.1): one held goes nowhere its holder is not beside
/// (`combat.gripped`), and one gripping drags no more than it can
/// (`combat.too_heavy`). Movement asks this before it moves anyone.
pub fn may_step(sim: &Sim, entity: EntityId, to: clash_world::Location) -> Result<(), Reason> {
    for grip in sim.grips.on_held(entity) {
        let Some(at) = sim.truth().location(grip.holder) else {
            continue;
        };
        if !beside(sim, at, to) {
            return Err(refuse(
                GRIPPED,
                format!(
                    "{entity} is held by {} and cannot move away without breaking the grip",
                    grip.holder
                ),
            )
            .with("holder", grip.holder.to_string()));
        }
    }
    for grip in sim.grips.of_holder(entity) {
        let Some(most) = grip.drags else {
            continue;
        };
        let mass = super::push::mass_of(sim, grip.held);
        if mass > most {
            return Err(refuse(
                TOO_HEAVY,
                format!("{entity} cannot drag {} with it", grip.held),
            )
            .with("what", grip.held.to_string())
            .with("mass", mass)
            .with("most", most));
        }
    }
    Ok(())
}

/// The kernel hands every event to grips once its handlers have run:
/// a holder that stepped drags what it holds into the cell it left, and
/// a grip whose held is no longer beside its holder, or either of whom
/// has left the world or been launched, is lost.
pub(crate) fn on_event(sim: &mut Sim, event: &SimEvent) {
    if sim.grips.is_empty() {
        return;
    }
    match &event.body {
        EventBody::EntityRelocated { entity, from, to } => {
            let entity = *entity;
            let (from, to) = (*from, *to);
            // The holder moved: what it holds follows into where it was.
            if let (Place::At { loc: left, .. }, Place::At { .. }) = (from, to) {
                let held: Vec<EntityId> = sim
                    .grips
                    .of_holder(entity)
                    .map(|grip| grip.held)
                    .collect();
                for held in held {
                    let Some(at) = sim.truth().location(held) else {
                        continue;
                    };
                    let Some(now) = sim.truth().location(entity) else {
                        continue;
                    };
                    if beside(sim, at, now) {
                        continue;
                    }
                    if crate::movement::move_to(sim, held, left, event.cause).is_err() {
                        lose(sim, |grip| grip.holder == entity && grip.held == held, event.cause);
                    }
                }
            }
            check_beside(sim, entity, event.cause);
        }
        _ if event.name() == super::events::LAUNCHED => {
            if let Some(Subject::Entity(entity)) = event.subject("self") {
                lose(
                    sim,
                    |grip| grip.holder == entity || grip.held == entity,
                    event.cause,
                );
            }
        }
        EventBody::EntityRemoved { entity } => {
            let entity = *entity;
            lose(sim, |grip| grip.holder == entity || grip.held == entity, event.cause);
        }
        _ => {}
    }
}

/// Loses the grips of `entity`, as holder or held, whose two are no
/// longer beside each other or in the world.
fn check_beside(sim: &mut Sim, entity: EntityId, cause: Cause) {
    let apart: Vec<(EntityId, EntityId)> = sim
        .grips
        .iter()
        .filter(|grip| grip.holder == entity || grip.held == entity)
        .filter(|grip| {
            match (
                sim.truth().location(grip.holder),
                sim.truth().location(grip.held),
            ) {
                (Some(a), Some(b)) => !beside(sim, a, b),
                _ => true,
            }
        })
        .map(|grip| (grip.holder, grip.held))
        .collect();
    for (holder, held) in apart {
        lose(sim, |grip| grip.holder == holder && grip.held == held, cause);
    }
}

/// Loses grips: they end with how `lost`, and their actions are
/// interrupted with `combat.grip_lost`.
fn lose(sim: &mut Sim, matches: impl Fn(&Grip) -> bool, cause: Cause) {
    end_where(sim, matches, "lost", Some(GRIP_LOST), cause);
}

// ---- What rules call, read and listen for ---------------------------------------

fn bad_argument(verb: &str, name: &str, wanted: &str) -> Reason {
    Reason::new(
        super::ranged::reason::BAD_ARGUMENT,
        format!("`{verb}`: the argument `{name}` must be {wanted}"),
    )
    .with("verb", verb)
    .with("arg", name)
}

fn entity_arg(sim: &Sim, call: &VerbCall<'_>, index: usize) -> Option<EntityId> {
    match sim.subject_of(arg(call, index)) {
        Some(Subject::Entity(id)) => Some(id),
        _ => None,
    }
}

fn self_of(sim: &Sim, call: &VerbCall<'_>) -> Option<EntityId> {
    call.by
        .subject("self")
        .and_then(|value| sim.subject_of(value))
        .and_then(|subject| subject.entity())
}

fn seize_verb(sim: &mut Sim, call: &VerbCall<'_>, cause: Cause) -> Result<Value, Reason> {
    const VERB: &str = "seize";
    let holder = entity_arg(sim, call, 0)
        .or_else(|| self_of(sim, call))
        .ok_or_else(|| bad_argument(VERB, "who", "an entity"))?;
    let held =
        entity_arg(sim, call, 1).ok_or_else(|| bad_argument(VERB, "target", "an entity"))?;
    let on = arg(call, 2)
        .as_subject()
        .and_then(|handle| part_of_handle(&sim.components().body, handle))
        .filter(|(owner, _)| *owner == held)
        .map(|(_, key)| key);
    let pins = arg(call, 3).as_bool().unwrap_or(false);
    let drags = match arg(call, 4) {
        Value::Null => None,
        value => Some(
            value
                .as_number()
                .filter(|most| most.is_finite() && *most >= Real::ZERO)
                .ok_or_else(|| bad_argument(VERB, "drags", "a mass in kilograms, zero or more"))?,
        ),
    };
    seize(
        sim,
        &Seize {
            holder,
            held,
            on,
            pins,
            drags,
        },
        cause,
    )?;
    Ok(Value::Bool(true))
}

fn release_verb(sim: &mut Sim, call: &VerbCall<'_>, cause: Cause) -> Result<Value, Reason> {
    const VERB: &str = "release_grip";
    let who = entity_arg(sim, call, 0)
        .or_else(|| self_of(sim, call))
        .ok_or_else(|| bad_argument(VERB, "who", "an entity"))?;
    let other = entity_arg(sim, call, 1);
    let ended = release_grip(sim, who, other, cause);
    Ok(Value::Int(i64::try_from(ended).unwrap_or(i64::MAX)))
}

/// The verbs of grips: `seize` and `release_grip`.
pub fn verbs() -> Vec<VerbDef> {
    vec![
        VerbDef {
            name: "seize",
            build: |registry| {
                spec(
                    registry,
                    "seize",
                    "Takes hold of a creature or a thing (07 §9.1), for as long as the holder's action whose list runs this goes on: usually an attack's `on_land`, so that the grab is resolved like a strike (reach, aim, defences) first. With `pins`, the channel slots of the part held and the parts on it are disabled until the grip ends. While it lasts the held goes nowhere its holder is not beside, and is dragged into the cell the holder leaves when the holder steps. Raises the hook `seize_resolving`, then `seized`. Returns true. Refused with `combat.no_action`, `combat.no_target`, `combat.not_its_part`, `combat.out_of_reach` and `combat.seize_failed`, having changed nothing.",
                    &[
                        ("who", "entity", true, "Who takes hold. Default `self`."),
                        ("target", "entity", false, "What is held."),
                        ("part", "part", true, "The part held, if a particular one."),
                        ("pins", "bool", true, "Whether the part held is pinned: its channel slots are disabled while the grip lasts. Default false."),
                        ("drags", "real", true, "The most mass, in kilograms, the holder drags with it when it steps; a step with more is refused. Default: any."),
                    ],
                )
            },
            run: seize_verb,
        },
        VerbDef {
            name: "release_grip",
            build: |registry| {
                spec(
                    registry,
                    "release_grip",
                    "Ends the grips `who` holds or is held by (07 §9.1), only those with `other` if it is given, giving back what they pinned; raises `grip_broken` for each. A grip ended other than by its maintaining action's own lists interrupts that action: `combat.grip_broken` when the held broke free, `combat.grip_released` when the holder let go. What a break-free action ends in. Returns how many ended.",
                    &[
                        ("who", "entity", true, "The holder or the held. Default `self`."),
                        ("other", "entity", true, "Only the grips with this one."),
                    ],
                )
            },
            run: release_verb,
        },
    ]
}

/// The properties of grips: `entity.gripping` and `entity.gripped_by`.
pub fn properties() -> Vec<PropertyDef> {
    vec![
        PropertyDef::new(
            "entity",
            "gripping",
            "entity?",
            "The first creature or thing it grips, in the order taken (07 §9.1); null for none. Readable of oneself in a mind context.",
            |sim, subject, _| {
                subject
                    .entity()
                    .and_then(|id| gripped_by_holder(sim, id).first().copied())
                    .map_or(Value::Null, |id| sim.subject_value(Subject::Entity(id)))
            },
        )
        .own()
        .changed_by(&[SEIZED, GRIP_ENDED]),
        PropertyDef::new(
            "entity",
            "gripped_by",
            "entity?",
            "Who grips it, the first to have taken hold (07 §9.1); null for no one. Readable of oneself in a mind context.",
            |sim, subject, _| {
                subject
                    .entity()
                    .and_then(|id| holder_of(sim, id))
                    .map_or(Value::Null, |id| sim.subject_value(Subject::Entity(id)))
            },
        )
        .own()
        .changed_by(&[SEIZED, GRIP_ENDED]),
    ]
}

/// The events and the hook of grips.
pub const GRIP_EVENTS: &[CombatEvent] = &[
    CombatEvent {
        name: SEIZE_RESOLVING,
        subjects: &[("self", "entity"), ("target", "entity")],
        fields: &[
            ("part", "part?", "The part to be held; null for none in particular."),
            ("pins", "bool", "Whether it pins that part."),
        ],
        hook: Some(&["part", "pins"]),
        doc: "Runs before `self` takes hold of `target` (07 §9.1). A handler may change the part held and whether it is pinned, or cancel: the grip fails (`combat.seize_failed`) and nothing is held.",
    },
    CombatEvent {
        name: SEIZED,
        subjects: &[("self", "entity"), ("target", "entity")],
        fields: &[
            ("part", "part?", "The part held; null for none in particular."),
            ("pins", "bool", "Whether it pins that part."),
            ("action", "int", "The number of the holder's action that maintains it."),
        ],
        hook: None,
        doc: "`self` took hold of `target` (07 §9.1). The grip lasts as long as the action that took it.",
    },
    CombatEvent {
        name: GRIP_ENDED,
        subjects: &[("self", "entity"), ("target?", "entity")],
        fields: &[
            ("part", "part?", "The part that was held."),
            (
                "how",
                "string",
                "`ended`: its action ended; `released`: the holder let go; `broken`: the held broke free; `thrown`: the held was thrown; `lost`: they came apart.",
            ),
        ],
        hook: None,
        doc: "A grip of `self` on `target` ended (07 §9.1), and what it pinned works again.",
    },
];

/// The reasons of grips, for the generated reference.
pub const GRIP_REASONS: &[ReasonInfo] = &[
    ReasonInfo {
        code: NO_ACTION,
        doc: "`seize` was run outside an action of the holder's: a grip is kept by the action that takes it (07 §9.1). Values: `holder`.",
    },
    ReasonInfo {
        code: OUT_OF_REACH,
        doc: "What was to be seized is further from the holder than its body reaches. Values: `target`, `reach`.",
    },
    ReasonInfo {
        code: SEIZE_FAILED,
        doc: "A handler of `seize_resolving` made the grip fail. Values: `target`.",
    },
    ReasonInfo {
        code: GRIPPED,
        doc: "A step by a creature held would take it away from its holder: it cannot move away without breaking the grip (07 §9.1). Values: `holder`.",
    },
    ReasonInfo {
        code: TOO_HEAVY,
        doc: "A step by a holder would drag something heavier than its grip's `drags`. Values: `what`, `mass`, `most`.",
    },
    ReasonInfo {
        code: GRIP_LOST,
        doc: "A grip's two came apart (one was knocked away, launched, or left the world): the action that kept it is interrupted. Values: `target`.",
    },
    ReasonInfo {
        code: GRIP_BROKEN,
        doc: "The held broke free (`release_grip` by the held): the action that kept the grip is interrupted. Values: `target`.",
    },
    ReasonInfo {
        code: GRIP_RELEASED,
        doc: "The holder let go by something other than the action that kept the grip: that action is interrupted. Values: `target`.",
    },
    ReasonInfo {
        code: NOT_GRIPPED,
        doc: "A creature to be thrown is not one the thrower grips. Values: `who`, `what`.",
    },
];
