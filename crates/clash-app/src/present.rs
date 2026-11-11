//! Showing what the simulation reports (12 §7).
//!
//! **A seam.** Playback, which plays a stretch of events back over real
//! time with motion, animations and `show` definitions, is being built
//! beside this crate. For the first walk [`present_events`] does the
//! least that keeps the rules: every event the simulation delivered
//! reaches the scene before the next frame, the view model is brought
//! up to date at once, and each event that carries words gets one line
//! in the log.
//!
//! When playback lands, the frame loop hands it the drained events and
//! the real time elapsed in place of calling this function, and this
//! file is deleted.

use clash_scene::RichText;
use clash_scene::ui::MessageLog;
use clash_sim::{EventBody, SimEvent};

use crate::session::Session;

/// The log category of a fault the engine contained.
pub const LOG_FAULT: &str = "fault";
/// The log category of a line an event of a mod's own carries.
pub const LOG_EVENT: &str = "event";

/// The field of a mod's event that holds a line for the log.
const FIELD_TEXT: &str = "text";

/// The line an event has for the log, with its category, if it has
/// one. The kernel's events carry no words yet, but for a contained
/// fault; a mod's event may carry a `text`.
pub fn message_of(event: &SimEvent) -> Option<(&'static str, String)> {
    match &event.body {
        EventBody::Fault { reason } => Some((LOG_FAULT, format!("Fault: {reason}"))),
        EventBody::Custom(payload) => payload
            .fields
            .iter()
            .find(|(name, _)| name == FIELD_TEXT)
            .and_then(|(_, value)| value.plain())
            .and_then(|value| value.as_str())
            .map(|text| (LOG_EVENT, text.to_owned())),
        _ => None,
    }
}

/// Shows a batch of events the simulation delivered: the view of the
/// world is marked to be fed again before the next frame, and each
/// event with a message gets one line in the log. See the
/// [module](self) documentation.
pub fn present_events(events: &[SimEvent], session: &mut Session, log: &mut MessageLog) {
    for event in events {
        // Anything may have changed what is in sight.
        session.stale_view = true;
        if let Some((category, line)) = message_of(event) {
            log.push(category, RichText::plain_text(&line));
        }
    }
}
