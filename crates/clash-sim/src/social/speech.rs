//! Speech (15 §11): messages said aloud, carried by a signal, heard by
//! whoever perceives the signal and understands it.
//!
//! ```text
//! say { who, kind, to?, about?, event?, line?, ... }
//!   → a signal of the kind's signal kind (the `noise` kind if it names
//!     none) from the speaker's mouth, carrying a Message
//!   → each observer that perceives it (15 §6): the hook `understands`,
//!     then `message_heard`; to whom it was said, a report's beliefs and
//!     an order
//! ```
//!
//! **Reports.** A message of a kind that carries reports takes with it,
//! when it is said, the speaker's memories of the event type named, or
//! naming whom it takes `about` for, and its track of `about`. Whom it is
//! said to gains them as told (`Knowing::Told`), with the speaker's
//! confidence times the kind's `belief`, times where the hearer's
//! relation of the kind's `trust` toward the speaker stands in its range
//! if the kind names one; each marked with its **source**, the persona
//! the hearer took the speaker for (none if it did not make them out).
//!
//! **Lines.** A `lines` table gives what is said: a line whose condition
//! holds is picked, by weight, from the random stream `social`, when it
//! is said, and written for each hearer as reader (05 §4.6) when heard.
//!
//! **Exchanges** (a short back-and-forth as a joint action, 03 §8) are
//! not written yet: a joint action whose phases `say`. **Talking to someone**: [`talk_options`]
//! raises the hook `talk_options` for the application's talk screen.

use clash_core::{EntityId, Real, StateHash, StateHasher, impl_state_hash};
use clash_expr::Value;
use clash_world::{Cause, CauseKind, Location};
use serde::{Deserialize, Serialize};

use super::defs::{Carries, LinesDef, LinesId, MessageKindDef, MessageKindId, line_scope};
use super::order::{self, OrderPlan};
use super::reason as why;
use super::relation::{self, Toward};
use super::{SPEECH_STREAM, events, local_now, memories_of, share};
use crate::Sim;
use crate::bus::{EventPayload, FieldValue};
use crate::event::{Subject, Where};
use crate::host::Bound;
use crate::perception::{Knowing, Memory, PersonaId, SenseId, Signal, SignalKindId, belief};
use crate::reason::Reason;

/// The most memories one report carries: the latest that match.
pub const MAX_REPORT_MEMORIES: usize = 4;

/// What a speaker knew of where someone was, as a report carries it.
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub struct ReportTrack {
    /// Whom the speaker took them for.
    pub persona: PersonaId,
    /// Where it last knew them to be.
    pub loc: Option<Location>,
    /// How sure it was when it spoke.
    pub confidence: Real,
}

impl_state_hash!(ReportTrack {
    persona,
    loc,
    confidence
});

/// What a report carries (15 §11): memories and a track, as the speaker
/// held them when it spoke.
#[derive(Clone, PartialEq, Eq, Debug, Default, Serialize, Deserialize)]
pub struct Report {
    /// Memories, oldest first.
    pub memories: Vec<Memory>,
    /// Where someone was.
    pub track: Option<ReportTrack>,
}

impl_state_hash!(Report { memories, track });

impl Report {
    /// Whether it carries nothing.
    pub fn is_empty(&self) -> bool {
        self.memories.is_empty() && self.track.is_none()
    }
}

/// Something said (15 §11): carried by a signal to whoever perceives it.
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub struct Message {
    /// Its number, given once in the simulation's life.
    pub id: u64,
    /// Its kind.
    pub kind: MessageKindId,
    /// Who said it. Truth: a hearer is told whom it took them for only if
    /// it made them out (15 §6).
    pub speaker: EntityId,
    /// To whom it was said, if to someone.
    pub to: Option<EntityId>,
    /// To which group it was said, by key, if to a group.
    pub to_group: Option<String>,
    /// What it is about, if anything.
    pub about: Option<EntityId>,
    /// The line picked to say it with: the table and the line's place.
    pub line: Option<(LinesId, u16)>,
    /// The language it is said in, as written, for `understands`.
    pub language: Option<String>,
    /// What a report carries.
    pub report: Report,
    /// What an order says to do.
    pub order: Option<OrderPlan>,
}

impl StateHash for Message {
    fn state_hash(&self, hasher: &mut StateHasher) {
        hasher.write_u64(self.id);
        self.kind.state_hash(hasher);
        self.speaker.state_hash(hasher);
        self.to.state_hash(hasher);
        self.to_group.state_hash(hasher);
        self.about.state_hash(hasher);
        match self.line {
            Some((lines, at)) => {
                hasher.write_u8(1);
                lines.state_hash(hasher);
                hasher.write_u16(at);
            }
            None => hasher.write_u8(0),
        }
        self.language.state_hash(hasher);
        self.report.state_hash(hasher);
        self.order.state_hash(hasher);
    }
}

/// What `say` is to say and how.
#[derive(Clone, PartialEq, Debug)]
pub struct SayArgs {
    /// The kind of message.
    pub kind: MessageKindId,
    /// To whom, if to someone.
    pub to: Option<EntityId>,
    /// To which group, by key or definition id, if to a group.
    pub to_group: Option<String>,
    /// What it is about.
    pub about: Option<EntityId>,
    /// For a report: the event type of the memories it carries.
    pub event: Option<String>,
    /// The table of lines to say it with.
    pub lines: Option<LinesId>,
    /// The language it is said in.
    pub language: Option<String>,
    /// The kind of signal it is said with, if not the kind's own.
    pub signal: Option<SignalKindId>,
    /// How strong, if not the kind's own.
    pub strength: Option<Real>,
}

impl SayArgs {
    /// A message of a kind to nobody in particular, about nothing.
    pub fn new(kind: MessageKindId) -> SayArgs {
        SayArgs {
            kind,
            to: None,
            to_group: None,
            about: None,
            event: None,
            lines: None,
            language: None,
            signal: None,
            strength: None,
        }
    }
}

/// A message kind's definition, or the reason there is none.
pub(crate) fn kind_def(sim: &Sim, kind: MessageKindId) -> Result<&MessageKindDef, Reason> {
    sim.defs
        .try_registry::<MessageKindDef>()
        .and_then(|registry| registry.try_get(kind))
        .ok_or_else(|| {
            Reason::new(why::UNKNOWN_MESSAGE_KIND, "there is no such message kind")
                .with("kind", kind.index() as i64)
        })
}

/// Says something (`say`, 15 §11): a message of a kind, carried by a
/// signal from the speaker. A kind that carries reports takes with it
/// what the speaker knows of `about` and of `event`. Returns how many
/// observers the signal was sent to. Refused for a speaker out of the
/// world and a kind, a table of lines or a group that does not exist.
pub fn say(sim: &mut Sim, speaker: EntityId, args: SayArgs, cause: Cause) -> Result<u32, Reason> {
    let carries = kind_def(sim, args.kind)?.carries;
    let report = if carries == Carries::Report {
        report_of(sim, speaker, args.about, args.event.as_deref())
    } else {
        Report::default()
    };
    say_with(sim, speaker, args, report, None, cause)
}

/// What a speaker knows to report of someone and of an event type.
fn report_of(sim: &Sim, speaker: EntityId, about: Option<EntityId>, event: Option<&str>) -> Report {
    let persona = about.map(|about| super::persona_of(sim, speaker, about));
    let matching: Vec<&Memory> = memories_of(sim, speaker)
        .iter()
        .filter(|memory| event.is_none_or(|event| memory.event == event))
        .filter(|memory| {
            persona.is_none_or(|persona| {
                memory
                    .roles
                    .iter()
                    .any(|(_, played)| *played == Some(persona))
            })
        })
        .collect();
    let skip = matching.len().saturating_sub(MAX_REPORT_MEMORIES);
    let memories = matching.into_iter().skip(skip).cloned().collect();
    let track = about.and_then(|about| {
        let track = belief::track_of(sim, speaker, about)?;
        Some(ReportTrack {
            persona: track.persona(),
            loc: track.loc(),
            confidence: belief::confidence(sim, speaker, about).unwrap_or(Real::ZERO),
        })
    });
    Report { memories, track }
}

/// Says a message with what it carries.
pub(crate) fn say_with(
    sim: &mut Sim,
    speaker: EntityId,
    args: SayArgs,
    report: Report,
    order: Option<OrderPlan>,
    cause: Cause,
) -> Result<u32, Reason> {
    sim.known(speaker)?;
    let def = kind_def(sim, args.kind)?;
    let signal_kind = args
        .signal
        .or_else(|| def.signal.as_ref().map(|signal| signal.id()))
        .or_else(|| sim.perception_defs.noise())
        .ok_or_else(|| no_voice(speaker))?;
    let strength = args.strength.unwrap_or(def.strength);
    let to_group = match &args.to_group {
        Some(name) => Some(super::group::require(sim, name)?),
        None => None,
    };
    let mouth = crate::perception::signal::eye_of(sim, speaker).ok_or_else(|| no_voice(speaker))?;
    let line = match args.lines {
        Some(lines) => pick_line(sim, lines, speaker, args.to, args.about)?,
        None => None,
    };
    sim.social.next_message += 1;
    let message = Message {
        id: sim.social.next_message,
        kind: args.kind,
        speaker,
        to: args.to,
        to_group,
        about: args.about,
        line,
        language: args.language,
        report,
        order,
    };
    let signal = Signal::new(signal_kind, mouth.loc, strength, cause)
        .at_height(mouth.alt_mm)
        .about(speaker)
        .saying(message);
    let emitted = crate::perception::emit_signal(sim, signal)?;
    Ok(emitted.received + emitted.delayed)
}

fn no_voice(speaker: EntityId) -> Reason {
    Reason::new(
        why::NO_VOICE,
        format!("{speaker} cannot be heard: it is not in the world, or nothing carries speech"),
    )
    .with("entity", speaker.to_string())
}

/// What a line is bound to: the speaker, whom it is said to and what it
/// is about, in the order of [`line_scope`].
fn line_bound(
    sim: &Sim,
    speaker: EntityId,
    to: Option<EntityId>,
    about: Option<EntityId>,
) -> Bound {
    let value = |entity: Option<EntityId>| {
        entity.map_or(Value::Null, |entity| {
            sim.subject_value(Subject::Entity(entity))
        })
    };
    Bound::new()
        .subject(value(Some(speaker)))
        .subject(value(to))
        .subject(value(about))
}

/// Picks a line of a table whose condition holds, by weight, from the
/// random stream `social`. `None` if none fits.
fn pick_line(
    sim: &mut Sim,
    lines: LinesId,
    speaker: EntityId,
    to: Option<EntityId>,
    about: Option<EntityId>,
) -> Result<Option<(LinesId, u16)>, Reason> {
    let defs = std::sync::Arc::clone(&sim.defs);
    let table = defs
        .try_registry::<LinesDef>()
        .and_then(|registry| registry.try_get(lines))
        .ok_or_else(|| {
            Reason::new(why::UNKNOWN_LINES, "there is no such table of lines")
                .with("lines", lines.index() as i64)
        })?;
    let bound = line_bound(sim, speaker, to, about);
    let _ = line_scope;
    let mut fitting: Vec<(u16, Real)> = Vec::new();
    for (at, line) in table.lines.iter().enumerate() {
        let holds = match line.condition.constant_value() {
            Some(constant) => constant,
            None => line
                .condition
                .compiled()
                .is_none_or(|code| matches!(sim.eval(code, bound.clone()), Value::Bool(true))),
        };
        if holds && line.weight > Real::ZERO {
            fitting.push((u16::try_from(at).unwrap_or(u16::MAX), line.weight));
        }
    }
    let total: Real = fitting.iter().map(|(_, weight)| *weight).sum();
    if fitting.is_empty() || total <= Real::ZERO {
        return Ok(None);
    }
    let mut left = sim.rng.stream(SPEECH_STREAM).unit() * total;
    for (at, weight) in &fitting {
        if left < *weight {
            return Ok(Some((lines, *at)));
        }
        left -= *weight;
    }
    Ok(fitting.last().map(|(at, _)| (lines, *at)))
}

/// A line written for a hearer as reader (05 §4.6).
fn line_text(sim: &Sim, message: &Message, hearer: EntityId) -> Option<String> {
    let (lines, at) = message.line?;
    let defs = std::sync::Arc::clone(&sim.defs);
    let line = defs
        .try_registry::<LinesDef>()?
        .try_get(lines)?
        .lines
        .get(usize::from(at))?;
    let bound = line_bound(sim, message.speaker, message.to, message.about);
    let mut host = sim.host_for_reader(bound, hearer);
    Some(line.text.render_on(&defs, &mut host))
}

/// Whether a message was said to a hearer: to it, to a group it belongs
/// to, or to nobody in particular.
fn addressed(sim: &Sim, message: &Message, hearer: EntityId) -> bool {
    match (&message.to, &message.to_group) {
        (None, None) => true,
        (to, group) => {
            *to == Some(hearer)
                || group.as_ref().is_some_and(|key| {
                    sim.social
                        .groups
                        .get(key)
                        .is_some_and(|group| group.members.contains_key(&hearer))
                })
        }
    }
}

/// A hearer perceived a signal carrying a message (15 §11): if it
/// understands it, `message_heard` is raised; said to it, it gains what
/// a report carries and is given what an order says. `speaker` is the
/// speaker if the hearer made them out.
pub(crate) fn heard(
    sim: &mut Sim,
    hearer: EntityId,
    sense: SenseId,
    message: &Message,
    speaker: Option<EntityId>,
    at: Location,
    clarity: Real,
) {
    let _ = sense;
    if hearer == message.speaker {
        return;
    }
    let cause = Cause::new(CauseKind::Process);
    let Ok(def) = kind_def(sim, message.kind) else {
        return;
    };
    let (carries, belief_share, trust) =
        (def.carries, def.belief, def.trust.as_ref().map(|t| t.id()));
    let kind_name = sim
        .defs
        .try_registry::<MessageKindDef>()
        .map_or(String::new(), |registry| {
            registry.name(message.kind).to_owned()
        });
    if !understands(
        sim,
        hearer,
        speaker,
        &kind_name,
        message.language.as_deref(),
        cause,
    ) {
        return;
    }
    let to_it = addressed(sim, message, hearer);
    let text = line_text(sim, message, hearer);
    let mut payload =
        EventPayload::new(events::MESSAGE_HEARD).subject("self", Subject::Entity(hearer));
    if let Some(speaker) = speaker {
        payload = payload.subject("speaker", Subject::Entity(speaker));
    }
    if let Some(about) = message.about {
        payload = payload.subject("about", Subject::Entity(about));
    }
    if let Some(to) = message.to {
        payload = payload.subject("to", Subject::Entity(to));
    }
    let payload = payload
        .with("kind", kind_name.as_str())
        .with(
            "text",
            text.as_deref().map_or(FieldValue::Null, FieldValue::text),
        )
        .with("addressed", to_it)
        .with(
            "report",
            !message.report.is_empty() && to_it && carries == Carries::Report,
        )
        .with("clarity", clarity)
        .with("where", FieldValue::Subject(Subject::Loc(at)));
    if let Err(reason) = sim.raise(Where::Entity(hearer), cause, payload) {
        sim.fault(reason);
    }
    if !to_it {
        return;
    }
    match carries {
        Carries::Report if !message.report.is_empty() => {
            let source = speaker.map(|speaker| super::persona_of(sim, hearer, speaker));
            let trust_share = match (trust, source) {
                (Some(kind), Some(source)) => trust_share(sim, hearer, kind, source),
                (Some(_), None) => Real::ZERO,
                (None, _) => Real::ONE,
            };
            let factor = share(belief_share * trust_share);
            told(sim, hearer, &message.report, factor, source);
        }
        Carries::Order => {
            if let Some(plan) = &message.order {
                order::received(sim, hearer, message, speaker, plan, cause);
            }
        }
        _ => {}
    }
}

/// Where a hearer's relation of a kind toward a persona stands in the
/// kind's range, 0 to 1.
fn trust_share(
    sim: &Sim,
    hearer: EntityId,
    kind: super::RelationKindId,
    toward: PersonaId,
) -> Real {
    let Some(def) = relation::kind_def(sim, kind) else {
        return Real::ONE;
    };
    let (low, high) = def.bounds();
    let value = relation::rel_toward(sim, hearer, kind, &Toward::Persona(toward))
        .unwrap_or(def.default_value());
    share((value - low) / (high - low))
}

/// A hearer gains what a report carries, as told, at `factor` of the
/// speaker's confidence, marked with its source.
fn told(sim: &mut Sim, hearer: EntityId, report: &Report, factor: Real, source: Option<PersonaId>) {
    let now = local_now(sim, hearer);
    for memory in &report.memories {
        let told = Memory {
            at: now,
            how: Knowing::Told,
            confidence: share(memory.confidence * factor),
            source,
            ..memory.clone()
        };
        let _ = belief::remember(sim, hearer, told);
    }
    if let Some(track) = &report.track {
        belief::put_persona_track(
            sim,
            hearer,
            track.persona,
            track.loc,
            Knowing::Told,
            track.confidence * factor,
            source,
        );
    }
}

/// The hook `understands`: whether a hearer understands what it heard.
/// With no rule answering, it does.
fn understands(
    sim: &mut Sim,
    hearer: EntityId,
    speaker: Option<EntityId>,
    kind: &str,
    language: Option<&str>,
    cause: Cause,
) -> bool {
    if !sim.bus.has_listeners(events::UNDERSTANDS) {
        return true;
    }
    let mut payload =
        EventPayload::new(events::UNDERSTANDS).subject("self", Subject::Entity(hearer));
    if let Some(speaker) = speaker {
        payload = payload.subject("speaker", Subject::Entity(speaker));
    }
    let payload = payload
        .with("kind", kind)
        .with(
            "language",
            language.map_or(FieldValue::Null, FieldValue::text),
        )
        .with("understood", true);
    match sim.raise(Where::Entity(hearer), cause, payload) {
        Ok(raised) if raised.cancelled.is_some() => false,
        Ok(raised) => raised
            .field("understood")
            .and_then(|value| value.plain())
            .and_then(clash_core::Value::as_bool)
            .unwrap_or(true),
        Err(reason) => {
            sim.fault(reason);
            true
        }
    }
}

/// Something the player can say to someone (15 §11): what the
/// application's talk screen offers.
#[derive(Clone, PartialEq, Debug)]
pub struct TalkOption {
    /// Its id, as the rule that offered it wrote it.
    pub id: String,
    /// What it reads as.
    pub text: String,
    /// The message kind it says, by id, if it says one.
    pub kind: Option<String>,
    /// The table of lines it says it with, by id, if any.
    pub lines: Option<String>,
}

/// What one can say to another (15 §11): the hook `talk_options` is
/// raised with an empty list of `options`, which rules add to (each a map
/// with `id`, `text` and, if it says something, `kind` and `lines`), and
/// what they left is given back in order. The application offers them
/// through `ask` and says the one chosen with `say`.
pub fn talk_options(sim: &mut Sim, who: EntityId, to: EntityId) -> Result<Vec<TalkOption>, Reason> {
    sim.known(who)?;
    sim.known(to)?;
    if !sim.bus.has_listeners(events::TALK_OPTIONS) {
        return Ok(Vec::new());
    }
    let payload = EventPayload::new(events::TALK_OPTIONS)
        .subject("self", Subject::Entity(who))
        .subject("to", Subject::Entity(to))
        .with(
            "options",
            FieldValue::Plain(clash_core::Value::List(Vec::new())),
        );
    let raised = sim.raise(Where::Entity(who), Cause::new(CauseKind::Process), payload)?;
    if let Some(reason) = raised.cancelled {
        return Err(reason);
    }
    let Some(clash_core::Value::List(items)) = raised.field("options").and_then(FieldValue::plain)
    else {
        return Ok(Vec::new());
    };
    let text = |map: &std::collections::BTreeMap<String, clash_core::Value>, key: &str| {
        map.get(key)
            .and_then(clash_core::Value::as_str)
            .map(str::to_owned)
    };
    Ok(items
        .iter()
        .filter_map(|item| match item {
            clash_core::Value::Map(map) => Some(TalkOption {
                id: text(map, "id").unwrap_or_default(),
                text: text(map, "text").unwrap_or_default(),
                kind: text(map, "kind"),
                lines: text(map, "lines"),
            }),
            _ => None,
        })
        .collect())
}
