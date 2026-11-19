//! Takedowns (07 §10): a joint action (03 §8) with a forced role for the
//! victim, `sure` strikes, and a cutscene the application may play.
//!
//! Nothing here is a takedown. What makes one is content:
//!
//! - **Roles.** The attacker starts it; the victim's role is `forced`, so
//!   its participant is locked in without consent: its channels are held
//!   for the action's length, its own actions holding them give way, and
//!   it has no defence. Interrupting the attacker, or disabling the
//!   victim's slots, ends it for both.
//! - **`requires`** says what the attacker can see beforehand (it is the
//!   attacker's belief, 05 §7.3); **`requires_at_start`** says what is
//!   true when it begins: the bundled ones ask that the victim does not
//!   perceive the attacker, or is `helpless`. It is tested only when the
//!   attempt is made: an aware victim cannot be taken down, and the
//!   attempt is interrupted at once with the reason
//!   `engine:action_start_condition`.
//! - **`sure` strikes** skip aim (07 §3.4): the victim is held still. The
//!   strike is still a strike, delivered through what the victim wears
//!   and its body: a steel collar is a layer, and stops a blow it can.
//! - **`cutscene`** names a cutscene (12 §8.1). When an action that names
//!   one begins, after its `requires_at_start` held, `cutscene_cued` is
//!   raised with every participant: a cue for the application to show
//!   what the simulation then decides. It decides nothing.

use clash_core::EntityId;
use clash_expr::Value;
use clash_world::Cause;

use super::events::CombatEvent;
use crate::Sim;
use crate::bus::{EventPayload, FieldValue};
use crate::event::{Subject, Where};
use crate::later::ActionId;
use crate::registry::PropertyDef;

/// Raised when an action that names a cutscene begins.
pub const CUTSCENE_CUED: &str = "cutscene_cued";

/// Raises `cutscene_cued` for a running action that names a cutscene
/// (07 §10): the action runtime calls it as the action begins, once its
/// `requires_at_start` has held. Nothing is raised for one that names
/// none.
pub(crate) fn cue(sim: &mut Sim, id: ActionId, cause: Cause) {
    let Some(running) = sim.actions.get(id) else {
        return;
    };
    let (actor, def_id) = (running.actor, running.def);
    let defs = std::sync::Arc::clone(sim.defs());
    let Some(def) = crate::action::run::action_def(&defs, def_id) else {
        return;
    };
    let Some(cutscene) = def.cutscene.as_deref() else {
        return;
    };
    let mut participants: Vec<EntityId> = vec![actor];
    participants.extend(sim.actions.joined(id).iter().map(|joined| joined.entity));
    let list = Value::list(
        participants
            .iter()
            .map(|who| sim.subject_value(Subject::Entity(*who))),
    );
    let name = crate::action::run::action_name(&defs, def_id);
    let mut payload = EventPayload::new(CUTSCENE_CUED)
        .subject("self", Subject::Entity(actor))
        .with("cutscene", cutscene)
        .with("action", name.as_str())
        .with("id", i64::from(id.raw()))
        .with("participants", FieldValue::Plain(list.to_stored()));
    if let Some(other) = participants.get(1) {
        payload = payload.subject("other", Subject::Entity(*other));
    }
    if let Err(reason) = sim.raise(Where::Entity(actor), cause, payload) {
        sim.fault(reason);
    }
}

/// Whether an entity cannot resist (07 §10): its mind is not awake
/// (unconscious, or dead), or someone grips it with a pin.
pub fn helpless(sim: &Sim, entity: EntityId) -> bool {
    let mind_out = sim
        .vital(entity)
        .is_some_and(|state| state != crate::mind::VitalState::Awake);
    mind_out
        || sim
            .grips
            .on_held(entity)
            .any(|grip| grip.pins && !grip.pinned.is_empty())
}

/// The property `entity.helpless`.
pub fn properties() -> Vec<PropertyDef> {
    vec![
        PropertyDef::new(
            "entity",
            "helpless",
            "bool",
            "Whether it cannot resist (07 §10): its mind is not awake (unconscious, or dead), or someone grips it with a pin. What a takedown's `requires_at_start` reads, beside whether the victim perceives the attacker. Reads truth.",
            |sim, subject, _| {
                subject
                    .entity()
                    .map_or(Value::Null, |id| Value::Bool(helpless(sim, id)))
            },
        )
        .changed_by(&[
            "entity_unconscious",
            "entity_woke",
            "entity_died",
            "entity_revived",
            super::grip::SEIZED,
            super::grip::GRIP_ENDED,
        ]),
    ]
}

/// The event of takedowns.
pub const TAKEDOWN_EVENTS: &[CombatEvent] = &[CombatEvent {
    name: CUTSCENE_CUED,
    subjects: &[("self", "entity"), ("other?", "entity")],
    fields: &[
        (
            "cutscene",
            "string",
            "The cutscene the action names (12 §8.1), as written.",
        ),
        (
            "action",
            "string",
            "The qualified id of the action that began.",
        ),
        ("id", "int", "The number of the running action."),
        (
            "participants",
            "list<entity>",
            "Everyone taking part, in the order of the roles: whoever started it first.",
        ),
    ],
    hook: None,
    doc: "An action that names a cutscene began (07 §10, 12 §8.1), once its `requires_at_start` held: a cue for the application to show it. `self` started it; `other` is the participant of its second role, for a joint action. What happens is the simulation's to decide, as for any action.",
}];
