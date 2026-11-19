//! The events perception raises (15 §5.1, §6, §7.1): each is an engine
//! event listed in [`KERNEL_EVENTS`](crate::KERNEL_EVENTS), raised as a
//! payload of its registered type.
//!
//! Their cause names no actor: whoever made what was perceived is truth,
//! and a rule that listens for a percept is told only what the observer
//! made out (15 §6).

use clash_core::{EntityId, Real};
use clash_world::{Cause, CauseKind, Location};

use crate::Sim;
use crate::bus::{EventPayload, FieldValue};
use crate::event::{EventBody, Subject, Where};

/// `perceived`: an observer perceived something (15 §6).
pub const PERCEIVED: &str = "perceived";
/// `suspicion_raised`: awareness crossed a sense's `suspect`.
pub const SUSPICION_RAISED: &str = "suspicion_raised";
/// `detected`: awareness reached 1.
pub const DETECTED: &str = "detected";
/// `lost_track`: awareness of something detected fell to 0.
pub const LOST_TRACK: &str = "lost_track";
/// `memory_added`: a mind remembered something.
pub const MEMORY_ADDED: &str = "memory_added";
/// `memory_forgotten`: a mind forgot a memory to make room.
pub const MEMORY_FORGOTTEN: &str = "memory_forgotten";

/// The events this package raises, by name.
pub const NAMES: &[&str] = &[
    PERCEIVED,
    SUSPICION_RAISED,
    DETECTED,
    LOST_TRACK,
    MEMORY_ADDED,
    MEMORY_FORGOTTEN,
];

/// A percept of something whose awareness reached detection.
pub const PERCEPT_DETECTION: &str = "detection";
/// A percept of a signal that was given off.
pub const PERCEPT_SIGNAL: &str = "signal";

/// The cause every perception event carries: the engine's, with no actor.
fn cause() -> Cause {
    Cause::new(CauseKind::Process)
}

fn place(at: Option<Location>) -> FieldValue {
    at.map_or(FieldValue::Null, |loc| {
        FieldValue::Subject(Subject::Loc(loc))
    })
}

fn entity(id: Option<EntityId>) -> FieldValue {
    id.map_or(FieldValue::Null, |id| {
        FieldValue::Subject(Subject::Entity(id))
    })
}

fn raise(sim: &mut Sim, observer: EntityId, payload: EventPayload) {
    sim.emit(Where::Entity(observer), cause(), EventBody::Custom(payload));
}

/// What one percept is: what `perceived` carries.
#[derive(Clone, PartialEq, Debug)]
pub struct Percept {
    /// Who perceived it.
    pub observer: EntityId,
    /// What it appeared to be about, if the observer made that out.
    pub about: Option<EntityId>,
    /// [`PERCEPT_DETECTION`] or [`PERCEPT_SIGNAL`].
    pub percept: &'static str,
    /// The sense's qualified id.
    pub sense: String,
    /// The qualified id of the kind of signal that carried it.
    pub signal: String,
    /// The type of the event the signal carried, if it carried one.
    pub event: Option<String>,
    /// Whom the observer took the event's actor for, if it made them out.
    pub about_actor: Option<EntityId>,
    /// Where it seemed to come from.
    pub at: Option<Location>,
    /// How far over the threshold it was received: received over
    /// threshold.
    pub clarity: Real,
}

/// Raises `perceived`.
pub fn perceived(sim: &mut Sim, percept: Percept) {
    let mut payload =
        EventPayload::new(PERCEIVED).subject("self", Subject::Entity(percept.observer));
    if let Some(about) = percept.about {
        payload = payload.subject("about", Subject::Entity(about));
    }
    let payload = payload
        .with("percept", percept.percept)
        .with("sense", percept.sense.as_str())
        .with("signal", percept.signal.as_str())
        .with(
            "event",
            percept
                .event
                .as_deref()
                .map_or(FieldValue::Null, FieldValue::text),
        )
        .with("about_actor", entity(percept.about_actor))
        .with("where", place(percept.at))
        .with("clarity", percept.clarity);
    raise(sim, percept.observer, payload);
}

/// Raises `suspicion_raised`.
pub fn suspicion_raised(
    sim: &mut Sim,
    observer: EntityId,
    about: EntityId,
    sense: &str,
    level: Real,
) {
    let payload = EventPayload::new(SUSPICION_RAISED)
        .subject("self", Subject::Entity(observer))
        .subject("about", Subject::Entity(about))
        .with("sense", sense)
        .with("level", level);
    raise(sim, observer, payload);
}

/// Raises `detected`.
pub fn detected(
    sim: &mut Sim,
    observer: EntityId,
    about: EntityId,
    sense: &str,
    at: Option<Location>,
) {
    let payload = EventPayload::new(DETECTED)
        .subject("self", Subject::Entity(observer))
        .subject("about", Subject::Entity(about))
        .with("sense", sense)
        .with("where", place(at));
    raise(sim, observer, payload);
}

/// Raises `lost_track`.
pub fn lost_track(
    sim: &mut Sim,
    observer: EntityId,
    about: EntityId,
    sense: &str,
    last: Option<Location>,
) {
    let payload = EventPayload::new(LOST_TRACK)
        .subject("self", Subject::Entity(observer))
        .subject("about", Subject::Entity(about))
        .with("sense", sense)
        .with("where", place(last));
    raise(sim, observer, payload);
}

/// Raises `memory_added`.
pub fn memory_added(
    sim: &mut Sim,
    mind: EntityId,
    event: &str,
    at: Option<Location>,
    salience: Real,
) {
    let payload = EventPayload::new(MEMORY_ADDED)
        .subject("self", Subject::Entity(mind))
        .with("event", event)
        .with("where", place(at))
        .with("salience", salience);
    raise(sim, mind, payload);
}

/// Raises `memory_forgotten`.
pub fn memory_forgotten(sim: &mut Sim, mind: EntityId, event: &str, at: Option<Location>) {
    let payload = EventPayload::new(MEMORY_FORGOTTEN)
        .subject("self", Subject::Entity(mind))
        .with("event", event)
        .with("where", place(at));
    raise(sim, mind, payload);
}
