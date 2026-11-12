//! Windows (03 §7.1, §7.2): published opportunities with a deadline,
//! who is offered them, and who notices them in time.
//!
//! # Opening
//!
//! A phase with a `window` opens one when it starts; the window closes
//! when the action enters the phase its `closes` names, or when the
//! action ends without entering it. [`Sim::open_window`] opens one from
//! Rust for whatever has no phase to close it: it then states how long it
//! stays open.
//!
//! # Who is offered it
//!
//! In this order, never its own source, each once:
//!
//! 1. those it names (its `for`), in the order named;
//! 2. anyone within the kind's `reach` of the source or of someone it
//!    names who holds toward one of those a relation flagged
//!    `intervenes` (15 §9.1), as the persona it takes them for: answered
//!    by [`social::intervenes`](crate::social::intervenes), bound with
//!    every simulation, or by what a test supplies with
//!    [`Sim::set_intervenes`];
//! 3. any directly controlled character within that `reach`.
//!
//! The second and third are found among the entities within `reach`
//! metres, middle to middle, of the source and of those named, by id.
//!
//! # Noticing
//!
//! For each candidate, when the window opens: perception's
//! [`notices_source`](crate::perception::notices_source) and the kind's
//! `noticed_if`; then its `notice`, a delay in the candidate's own time,
//! on the candidate's clock. A delay of zero is noticed at once. When the
//! delay is over and the window is still open, `window_noticed` is
//! raised for the candidate. If the window closes first the wake-up is
//! cancelled and the candidate had no chance. A candidate on a clock
//! twice as fast notices in half the universal time.
//!
//! What a candidate does once it has noticed is for rules (and the AI,
//! 09 §6.3) listening for `window_noticed`: the verbs `react` and
//! `ask_reaction` ([`react`](super::react)). With nothing listening,
//! nothing happens.
//!
//! # What is kept
//!
//! [`WindowTable`]: every open window with its candidates, and the count
//! of strikes each defence in force has dealt with (07 §5.1 `handles`).
//! It is plain data, saved and hashed with the simulation.

use std::collections::BTreeMap;
use std::sync::Arc;

use clash_core::{Dur, EntityId, Real, StateHash, StateHasher, Tick, define_id, impl_state_hash};
use clash_data::{Def, Runner};
use clash_expr::{EventSpec, Handle, KindSpec, Registry, Type, Value};
use clash_time::{Wake, WakeDomain, WakeId};
use clash_world::{Cause, CauseKind, OccupantFilter};
use serde::{Deserialize, Serialize};

use super::def::{Next, WindowKindDef};
use super::ids::{PhaseIndex, WindowKindId};
use crate::Sim;
use crate::bus::{EventPayload, EventType, FieldType, FieldValue};
use crate::entity::Control;
use crate::event::{CAUSE_SUBJECTS, EventInfo, Subject, Where};
use crate::host::Bound;
use crate::later::ActionId;
use crate::reason::Reason;
use crate::registry::FunctionDef;

/// The name of the kind of subject a window is, in expressions.
pub const WINDOW_KIND: &str = "window";

/// What the `arg` of an `Action` wake-up says when it ends a candidate's
/// delay in noticing a window: its `id` is the window's number.
pub(crate) const WAKE_NOTICE: u64 = 3;
/// What the `arg` of an `Action` wake-up says when it closes a window
/// opened for a length of time: its `id` is the window's number.
pub(crate) const WAKE_WINDOW_TIMER: u64 = 4;

/// Raised for a candidate that noticed a window in time.
pub const WINDOW_NOTICED: &str = "window_noticed";
/// Raised when a window closes.
pub const WINDOW_CLOSED: &str = "window_closed";

/// The heading the functions of windows are listed under in the
/// reference.
const GROUP_WINDOWS: &str = "Windows";

define_id! {
    /// An open window (03 §7.1): never given twice in one simulation.
    pub struct WindowId(u64);
}

/// Answers whether one entity holds toward another a relation whose
/// kind is flagged `intervenes` (15 §9.1): [`social::intervenes`](crate::social::intervenes)
/// by default, bound with every simulation; a test may supply another
/// ([`Sim::set_intervenes`]).
pub type IntervenesFn = fn(&Sim, EntityId, EntityId) -> bool;

/// Why a candidate was offered a window (03 §7.1).
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Offered {
    /// The window names it.
    Named,
    /// It holds an `intervenes` relation toward someone the window
    /// names, and is within reach.
    Intervenes,
    /// It is directly controlled and within reach.
    Controlled,
}

impl StateHash for Offered {
    fn state_hash(&self, hasher: &mut StateHasher) {
        hasher.write_u8(*self as u8);
    }
}

/// Where a candidate is with a window.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Notice {
    /// It can never notice: it does not perceive the source, or the
    /// kind's `noticed_if` was false.
    Never,
    /// Its delay in noticing is running: the wake-up that ends it.
    Pending(WakeId),
    /// The window closed before its delay was over.
    Late,
    /// It noticed, and `window_noticed` was raised.
    Noticed,
    /// It noticed and was asked how to answer: a `Reaction` decision is
    /// waiting.
    Asked,
    /// It answered with a reaction: the action it started.
    Reacted(ActionId),
    /// It was asked and declined (`Pass`).
    Passed,
}

impl Notice {
    /// Whether the candidate has noticed the window.
    pub fn noticed(self) -> bool {
        matches!(
            self,
            Notice::Noticed | Notice::Asked | Notice::Reacted(_) | Notice::Passed
        )
    }
}

impl StateHash for Notice {
    fn state_hash(&self, hasher: &mut StateHasher) {
        match self {
            Notice::Never => hasher.write_u8(0),
            Notice::Pending(wake) => {
                hasher.write_u8(1);
                wake.state_hash(hasher);
            }
            Notice::Late => hasher.write_u8(2),
            Notice::Noticed => hasher.write_u8(3),
            Notice::Asked => hasher.write_u8(4),
            Notice::Reacted(action) => {
                hasher.write_u8(5);
                action.state_hash(hasher);
            }
            Notice::Passed => hasher.write_u8(6),
        }
    }
}

/// One candidate of a window.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub struct Candidate {
    /// Who.
    pub entity: EntityId,
    /// Why it was offered the window.
    pub offered: Offered,
    /// How long it takes to notice, in its own time, as the kind's
    /// `notice` gave it when the window opened.
    pub delay: Dur,
    /// Where it is with the window.
    pub state: Notice,
}

impl_state_hash!(Candidate {
    entity,
    offered,
    delay,
    state
});

/// An open window (03 §7.1).
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub struct Window {
    /// Its kind.
    pub kind: WindowKindId,
    /// Whose action or rule opened it, if anyone's.
    pub source: Option<EntityId>,
    /// The action whose phase opened it, if one did.
    pub action: Option<ActionId>,
    /// The phase of that action whose start closes it.
    pub closes: Option<PhaseIndex>,
    /// Those it names, in the order named.
    pub aimed_at: Vec<EntityId>,
    /// The universal tick it opened at.
    pub opened: Tick,
    /// The universal tick it is expected to close at, as the clocks ran
    /// when it opened: the end of the phase that opened it, when that
    /// phase leads straight to the one that closes it, or the end of its
    /// length. `None` where that is not known.
    pub deadline: Option<Tick>,
    /// For a window opened for a length of time: the wake-up that
    /// closes it.
    pub timer: Option<WakeId>,
    /// Who it is offered to, in the order of 03 §7.1.
    pub candidates: Vec<Candidate>,
}

impl_state_hash!(Window {
    kind,
    source,
    action,
    closes,
    aimed_at,
    opened,
    deadline,
    timer,
    candidates
});

impl Window {
    /// The candidate that is this entity, if it is one.
    pub fn candidate(&self, entity: EntityId) -> Option<&Candidate> {
        self.candidates
            .iter()
            .find(|candidate| candidate.entity == entity)
    }

    fn candidate_mut(&mut self, entity: EntityId) -> Option<&mut Candidate> {
        self.candidates
            .iter_mut()
            .find(|candidate| candidate.entity == entity)
    }

    /// Whether the window names this entity.
    pub fn names(&self, entity: EntityId) -> bool {
        self.aimed_at.contains(&entity)
    }
}

/// How many strikes a defence in force has dealt with (07 §5.1
/// `handles`): the tick its phase began, and the count since.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub struct Handled {
    /// When the phase the defence is in force during began.
    pub since: Tick,
    /// How many strikes it has dealt with since.
    pub count: u32,
}

impl_state_hash!(Handled { since, count });

/// Every open window, and the tally of what defences in force have
/// dealt with (03 §7, 07 §5). Kept by the simulation as
/// [`Sim::windows`](crate::Sim); changed only by the runtime.
#[derive(Clone, PartialEq, Eq, Default, Debug, Serialize, Deserialize)]
pub struct WindowTable {
    /// The number the next window is given.
    next: u64,
    open: BTreeMap<WindowId, Window>,
    /// For each running action whose defence has dealt with a strike.
    handled: BTreeMap<ActionId, Handled>,
}

impl_state_hash!(WindowTable {
    next,
    open,
    handled
});

impl WindowTable {
    /// An open window.
    pub fn get(&self, id: WindowId) -> Option<&Window> {
        self.open.get(&id)
    }

    /// Every open window, oldest first.
    pub fn iter(&self) -> impl Iterator<Item = (WindowId, &Window)> {
        self.open.iter().map(|(id, window)| (*id, window))
    }

    /// How many windows are open.
    pub fn len(&self) -> usize {
        self.open.len()
    }

    /// Whether none is.
    pub fn is_empty(&self) -> bool {
        self.open.is_empty()
    }

    /// How many strikes the defence of a running action has dealt with
    /// since its phase, which began at `since`, began.
    pub fn handled(&self, action: ActionId, since: Tick) -> u32 {
        self.handled
            .get(&action)
            .filter(|handled| handled.since == since)
            .map_or(0, |handled| handled.count)
    }

    pub(crate) fn count_handled(&mut self, action: ActionId, since: Tick) {
        let entry = self
            .handled
            .entry(action)
            .or_insert(Handled { since, count: 0 });
        if entry.since != since {
            *entry = Handled { since, count: 0 };
        }
        entry.count = entry.count.saturating_add(1);
    }

    pub(crate) fn forget_action(&mut self, action: ActionId) {
        self.handled.remove(&action);
    }

    pub(crate) fn get_mut(&mut self, id: WindowId) -> Option<&mut Window> {
        self.open.get_mut(&id)
    }

    fn insert(&mut self, window: Window) -> Option<WindowId> {
        let id = WindowId::new(self.next);
        self.next = self.next.checked_add(1)?;
        self.open.insert(id, window);
        Some(id)
    }

    fn remove(&mut self, id: WindowId) -> Option<Window> {
        self.open.remove(&id)
    }

    /// The windows a running action opened, oldest first.
    fn of_action(&self, action: ActionId) -> Vec<WindowId> {
        self.open
            .iter()
            .filter(|(_, window)| window.action == Some(action))
            .map(|(id, _)| *id)
            .collect()
    }
}

/// A window to open from Rust ([`Sim::open_window`]): one with no phase
/// to close it, such as a shot's (07 §11.2).
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct OpenWindow {
    /// Its kind.
    pub kind: WindowKindId,
    /// Whose doing it is, if anyone's.
    pub source: Option<EntityId>,
    /// Those it is aimed at.
    pub aimed_at: Vec<EntityId>,
    /// How long it stays open, in universal time.
    pub open_for: Dur,
}

impl Sim {
    /// The windows that are open, and the tally of defences (03 §7).
    pub fn windows(&self) -> &WindowTable {
        &self.windows
    }

    /// Supplies the answer to whether one entity holds an `intervenes`
    /// relation toward another (15 §9.1), in place of the social layer's. Not
    /// state: a load binds the social layer's again. `None` answers no for everyone.
    pub fn set_intervenes(&mut self, answer: Option<IntervenesFn>) {
        self.intervenes = answer;
    }

    /// A window as expressions hold it: a handle of the kind `window`.
    /// Null where the registry has no such kind.
    pub fn window_value(&self, id: WindowId) -> Value {
        self.exprs
            .registry()
            .kind_id(WINDOW_KIND)
            .map_or(Value::Null, |kind| {
                Value::Subject(Handle::new(kind, id.raw()))
            })
    }

    /// The window a value names, if it is a handle of the kind `window`.
    /// Whether it is still open is [`WindowTable::get`].
    pub fn window_of(&self, value: &Value) -> Option<WindowId> {
        let handle = value.as_subject()?;
        (Some(handle.kind) == self.exprs.registry().kind_id(WINDOW_KIND))
            .then(|| WindowId::new(handle.payload))
    }

    /// How long an entity has, in its own time, before a window is
    /// expected to close. `None` for a window that is not open or whose
    /// deadline is not known.
    pub fn window_time_left(&self, id: WindowId, entity: EntityId) -> Option<Dur> {
        let deadline = self.windows.get(id)?.deadline?;
        let universal = deadline.since(self.now()).max(Dur::ZERO);
        Some(self.to_own_time(entity, universal))
    }

    /// A length of universal time as an entity's own: multiplied by the
    /// rate of the clock it is on.
    pub(crate) fn to_own_time(&self, entity: EntityId, universal: Dur) -> Dur {
        let rate = self
            .clock_of(entity)
            .and_then(|clock| self.timeline.effective_rate(clock))
            .unwrap_or(Real::ONE);
        Dur::from_secs_real(universal.as_secs_real() * rate)
    }

    /// A length of an entity's own time as universal time: divided by
    /// the rate of the clock it is on. `None` on a clock that is
    /// stopped.
    pub(crate) fn own_to_universal(&self, entity: EntityId, own: Dur) -> Option<Dur> {
        let rate = self
            .clock_of(entity)
            .and_then(|clock| self.timeline.effective_rate(clock))
            .unwrap_or(Real::ONE);
        (rate > Real::ZERO).then(|| Dur::from_secs_real(own.as_secs_real() / rate))
    }

    /// Opens the window of a phase that has just been entered (03 §7.1),
    /// once `window_opened` has been raised: works out who is offered it
    /// and starts each one's delay in noticing it.
    pub(crate) fn open_phase_window(
        &mut self,
        action: ActionId,
        kind: WindowKindId,
        closes: PhaseIndex,
        aimed_at: Vec<EntityId>,
    ) -> Option<WindowId> {
        let running = self.actions.get(action)?;
        let (actor, def_id, phase, due) = (running.actor, running.def, running.phase, running.due);
        let defs = Arc::clone(&self.defs);
        let leads_there = super::run::action_def(&defs, def_id)
            .and_then(|def| def.phase(phase))
            .is_some_and(|phase| phase.exits().any(|next| next == Next::Phase(closes)));
        let window = Window {
            kind,
            source: Some(actor),
            action: Some(action),
            closes: Some(closes),
            aimed_at,
            opened: self.now(),
            deadline: if leads_there { due } else { None },
            timer: None,
            candidates: Vec::new(),
        };
        self.begin_window(window)
    }

    /// Opens a window that no phase closes (03 §7.1, the `open_window`
    /// effect): it closes when its length is over. Its candidates are
    /// worked out and their delays started as for a phase's window.
    /// `window_opened` is the phase's event, and is not raised.
    pub fn open_window(&mut self, open: OpenWindow) -> Result<WindowId, Reason> {
        if self
            .defs
            .try_registry::<WindowKindDef>()
            .and_then(|registry| registry.try_get(open.kind))
            .is_none()
        {
            return Err(Reason::new(
                WINDOW_UNKNOWN_KIND,
                format!("there is no window kind {}", open.kind),
            )
            .with("kind", i64::from(open.kind.raw())));
        }
        let now = self.now();
        let window = Window {
            kind: open.kind,
            source: open.source,
            action: None,
            closes: None,
            aimed_at: open.aimed_at,
            opened: now,
            deadline: Some(Tick(
                now.as_micros().saturating_add(open.open_for.as_micros()),
            )),
            timer: None,
            candidates: Vec::new(),
        };
        let id = self.begin_window(window).ok_or_else(|| {
            Reason::new(
                WINDOW_UNKNOWN_KIND,
                "every number a window can have has been given out",
            )
        })?;
        if self.windows.get(id).is_some() {
            match self.schedule_on(
                clash_time::ClockId::UNIVERSAL,
                open.open_for,
                Wake::new(WakeDomain::Action, id.raw(), WAKE_WINDOW_TIMER),
            ) {
                Ok(wake) => {
                    if let Some(window) = self.windows.get_mut(id) {
                        window.timer = Some(wake);
                    }
                }
                Err(reason) => self.fault(reason),
            }
        }
        Ok(id)
    }

    /// Enters a window in the table, works out its candidates and starts
    /// their delays.
    fn begin_window(&mut self, mut window: Window) -> Option<WindowId> {
        window.candidates = self
            .offered(&window)
            .into_iter()
            .map(|(entity, offered)| Candidate {
                entity,
                offered,
                delay: Dur::ZERO,
                state: Notice::Never,
            })
            .collect();
        let id = self.windows.insert(window)?;
        let candidates: Vec<EntityId> = self
            .windows
            .get(id)
            .map(|window| window.candidates.iter().map(|c| c.entity).collect())
            .unwrap_or_default();
        for candidate in candidates {
            if self.windows.get(id).is_none() {
                break;
            }
            self.start_noticing(id, candidate);
        }
        Some(id)
    }

    /// Who a window is offered to (03 §7.1), in order: those it names,
    /// then by id those within reach who intervene for one of them, then
    /// by id the directly controlled within reach. Never its source.
    fn offered(&self, window: &Window) -> Vec<(EntityId, Offered)> {
        let mut out: Vec<(EntityId, Offered)> = Vec::new();
        let present = |id: EntityId| self.entities.location(id).is_some();
        for named in &window.aimed_at {
            if Some(*named) != window.source
                && present(*named)
                && !out.iter().any(|(have, _)| have == named)
            {
                out.push((*named, Offered::Named));
            }
        }
        let reach = self
            .defs
            .try_registry::<WindowKindDef>()
            .and_then(|registry| registry.try_get(window.kind))
            .map_or(Real::ZERO, |kind| kind.reach.get());
        let centres: Vec<EntityId> = window
            .source
            .into_iter()
            .chain(window.aimed_at.iter().copied())
            .filter(|id| present(*id))
            .collect();
        let mut near: Vec<EntityId> = Vec::new();
        for centre in &centres {
            let Some(loc) = self.entities.location(*centre) else {
                continue;
            };
            let found = self
                .cosmos
                .occupants_within(loc, reach + clash_world::dims::CELL_M, &OccupantFilter::ANY)
                .unwrap_or_default();
            for id in clash_world::distinct_ids(&found) {
                let within = crate::combat::strike::offset(self, *centre, id)
                    .is_some_and(|v| (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt() <= reach);
                if within && !near.contains(&id) {
                    near.push(id);
                }
            }
        }
        near.sort_unstable();
        let taken = |out: &[(EntityId, Offered)], id: EntityId| {
            Some(id) == window.source || out.iter().any(|(have, _)| *have == id)
        };
        if let Some(intervenes) = self.intervenes {
            for id in &near {
                if taken(&out, *id) {
                    continue;
                }
                if window
                    .aimed_at
                    .iter()
                    .any(|named| intervenes(self, *id, *named))
                {
                    out.push((*id, Offered::Intervenes));
                }
            }
        }
        for id in near {
            if taken(&out, id) {
                continue;
            }
            let direct = self
                .entities
                .get(id)
                .is_some_and(|record| record.control() == Control::Direct);
            if direct {
                out.push((id, Offered::Controlled));
            }
        }
        out
    }

    /// Evaluates whether a candidate can notice a window and how long it
    /// takes (03 §7.2), and starts the wait on its clock, or notices it
    /// at once.
    fn start_noticing(&mut self, id: WindowId, candidate: EntityId) {
        let Some(window) = self.windows.get(id) else {
            return;
        };
        let (kind, source) = (window.kind, window.source);
        let defs = Arc::clone(&self.defs);
        let Some(kind_def) = defs
            .try_registry::<WindowKindDef>()
            .and_then(|registry| registry.try_get(kind))
        else {
            return;
        };
        if !crate::perception::notices_source(self, candidate, source) {
            return;
        }
        let bound = Bound {
            subjects: vec![
                self.subject_value(Subject::Entity(candidate)),
                source.map_or(Value::Null, |source| {
                    self.subject_value(Subject::Entity(source))
                }),
                self.window_value(id),
            ],
            locals: Vec::new(),
            fields: Vec::new(),
        };
        let ((can, delay), warnings) = self.with_random(bound, |host| {
            let mut runner = Runner::new();
            let can = kind_def.noticed_if.holds(&mut runner.on(&defs, host));
            let delay = kind_def.notice.eval(&mut runner.on(&defs, host));
            ((can, delay), host.take_warnings())
        });
        for warning in warnings {
            self.report_warning(warning);
        }
        if !can {
            return;
        }
        let secs = match delay {
            Some(secs) if secs.is_finite() && secs >= Real::ZERO => secs,
            _ => {
                self.fault(
                    Reason::new(
                        super::run::ACTION_BAD_TIME,
                        "a window kind's `notice` came out as no length of time",
                    )
                    .with("kind", kind_name(&defs, kind).as_str()),
                );
                Real::ZERO
            }
        };
        let delay = Dur::from_secs_real(secs);
        if let Some(window) = self.windows.get_mut(id)
            && let Some(entry) = window.candidate_mut(candidate)
        {
            entry.delay = delay;
        }
        if delay <= Dur::ZERO {
            self.notice(id, candidate);
            return;
        }
        match self.schedule_for(
            candidate,
            delay,
            Wake::new(WakeDomain::Action, id.raw(), WAKE_NOTICE),
        ) {
            Ok(wake) => {
                if let Some(window) = self.windows.get_mut(id)
                    && let Some(entry) = window.candidate_mut(candidate)
                {
                    entry.state = Notice::Pending(wake);
                }
            }
            Err(reason) => self.fault(reason),
        }
    }

    /// A candidate notices a window: `window_noticed` is raised for it.
    fn notice(&mut self, id: WindowId, candidate: EntityId) {
        let Some(window) = self.windows.get_mut(id) else {
            return;
        };
        let Some(entry) = window.candidate_mut(candidate) else {
            return;
        };
        entry.state = Notice::Noticed;
        let window = window.clone();
        let defs = Arc::clone(&self.defs);
        let action = window
            .action
            .and_then(|action| self.actions.get(action))
            .map(|running| super::run::action_name(&defs, running.def));
        let left = self.window_time_left(id, candidate);
        let mut payload = EventPayload::new(WINDOW_NOTICED)
            .subject("self", Subject::Entity(candidate))
            .with("window", value_field(&self.window_value(id)))
            .with("kind", kind_name(&defs, window.kind).as_str())
            .with(
                "action",
                action.map_or(FieldValue::Null, |name| FieldValue::text(&name)),
            )
            .with("named", window.names(candidate))
            .with(
                "time_left",
                left.map_or(FieldValue::Null, |left| {
                    FieldValue::from(left.as_secs_real())
                }),
            );
        if let Some(source) = window.source {
            payload = payload.subject("source", Subject::Entity(source));
        }
        let cause = Cause {
            actor: window.source,
            instigator: window.source,
            kind: CauseKind::Process,
        };
        if let Err(reason) = self.raise(Where::Entity(candidate), cause, payload) {
            self.fault(reason);
        }
    }

    /// Closes a window (03 §7.1): what is still waiting to notice it never
    /// does, a reaction a player was being asked for is no longer asked
    /// for, and `window_closed` is raised.
    pub fn close_window(&mut self, id: WindowId) -> bool {
        let Some(mut window) = self.windows.remove(id) else {
            return false;
        };
        if let Some(timer) = window.timer.take() {
            self.cancel_wake(timer);
        }
        for candidate in &mut window.candidates {
            match candidate.state {
                Notice::Pending(wake) => {
                    self.cancel_wake(wake);
                    candidate.state = Notice::Late;
                }
                Notice::Asked => {
                    self.decisions.take(candidate.entity, |kind| {
                        matches!(kind, crate::decision::DecisionKind::Reaction { window, .. } if *window == id)
                    });
                }
                _ => {}
            }
        }
        let defs = Arc::clone(&self.defs);
        let count = |pick: fn(&Notice) -> bool| {
            i64::try_from(
                window
                    .candidates
                    .iter()
                    .filter(|candidate| pick(&candidate.state))
                    .count(),
            )
            .unwrap_or(i64::MAX)
        };
        let action = window
            .action
            .and_then(|action| self.actions.get(action))
            .map(|running| super::run::action_name(&defs, running.def));
        let mut payload = EventPayload::new(WINDOW_CLOSED)
            .with("window", value_field(&self.window_value(id)))
            .with("kind", kind_name(&defs, window.kind).as_str())
            .with(
                "action",
                action.map_or(FieldValue::Null, |name| FieldValue::text(&name)),
            )
            .with("offered", count(|_| true))
            .with("noticed", count(|state| state.noticed()))
            .with(
                "reacted",
                count(|state| matches!(state, Notice::Reacted(_))),
            );
        let on = match window.source {
            Some(source) => {
                payload = payload.subject("self", Subject::Entity(source));
                Where::Entity(source)
            }
            None => Where::Global,
        };
        let cause = Cause {
            actor: window.source,
            instigator: window.source,
            kind: CauseKind::Process,
        };
        if let Err(reason) = self.raise(on, cause, payload) {
            self.fault(reason);
        }
        true
    }

    /// Closes the windows of an action whose deadline is the start of
    /// this phase. Called as the phase is entered.
    pub(crate) fn close_windows_at(&mut self, action: ActionId, phase: PhaseIndex) {
        for id in self.windows.of_action(action) {
            if self
                .windows
                .get(id)
                .is_some_and(|window| window.closes == Some(phase))
            {
                self.close_window(id);
            }
        }
    }

    /// Closes every window of an action that is ending, and forgets what
    /// its defence dealt with.
    pub(crate) fn close_windows_of(&mut self, action: ActionId) {
        for id in self.windows.of_action(action) {
            self.close_window(id);
        }
        self.windows.forget_action(action);
    }

    /// Marks a candidate as having been asked, or as having answered.
    pub(crate) fn set_notice(&mut self, id: WindowId, entity: EntityId, state: Notice) {
        if let Some(window) = self.windows.get_mut(id)
            && let Some(entry) = window.candidate_mut(entity)
        {
            entry.state = state;
        }
    }
}

/// The handler of the window wake-ups of the `Action` domain: a
/// candidate's delay in noticing is over, or a window's length is.
pub(crate) fn window_wake(sim: &mut Sim, wake_id: WakeId, wake: Wake) {
    let id = WindowId::new(wake.id);
    match wake.arg {
        WAKE_NOTICE => {
            let Some(candidate) = sim.windows.get(id).and_then(|window| {
                window
                    .candidates
                    .iter()
                    .find(|candidate| candidate.state == Notice::Pending(wake_id))
                    .map(|candidate| candidate.entity)
            }) else {
                return;
            };
            sim.notice(id, candidate);
        }
        WAKE_WINDOW_TIMER
            if sim
                .windows
                .get(id)
                .is_some_and(|window| window.timer == Some(wake_id)) =>
        {
            if let Some(window) = sim.windows.get_mut(id) {
                window.timer = None;
            }
            sim.close_window(id);
        }
        _ => {}
    }
}

/// A window kind's qualified id.
pub(crate) fn kind_name(defs: &clash_data::Defs, kind: WindowKindId) -> String {
    defs.try_registry::<WindowKindDef>()
        .filter(|registry| kind.index() < registry.len())
        .map_or_else(
            || kind.to_string(),
            |registry| registry.name(kind).to_owned(),
        )
}

/// A value of the expression language as an event's field.
fn value_field(value: &Value) -> FieldValue {
    match value {
        Value::Null => FieldValue::Null,
        other => FieldValue::Plain(other.to_stored()),
    }
}

/// No window kind has the id that was given.
pub const WINDOW_UNKNOWN_KIND: &str = "window.unknown_kind";

/// The reasons of windows that are not the action system's own
/// ([`reason`](super::reason) has those), for the generated reference.
pub const WINDOW_REASONS: &[crate::reason::ReasonInfo] = &[crate::reason::ReasonInfo {
    code: WINDOW_UNKNOWN_KIND,
    doc: "A window was to be opened with a kind that is not loaded, or every number a window can have has been given out. Values: `kind`.",
}];

const WINDOW_FIELD: (&str, &str, &str) = (
    "window",
    "window",
    "The window: what `react` and `ask_reaction` are given.",
);
const KIND_FIELD: (&str, &str, &str) = ("kind", "string", "The qualified id of its kind.");
const ACTION_FIELD: (&str, &str, &str) = (
    "action",
    "string?",
    "The qualified id of the action whose phase opened it; null for a window no action opened.",
);

/// The events of windows (03 §13), raised as payloads.
pub const WINDOW_EVENTS: &[EventInfo] = &[
    EventInfo {
        name: WINDOW_NOTICED,
        subjects: &[("self", "entity"), ("source?", "entity")],
        fields: &[
            WINDOW_FIELD,
            KIND_FIELD,
            ACTION_FIELD,
            (
                "named",
                "bool",
                "Whether the window names `self`: false for one offered for someone else.",
            ),
            (
                "time_left",
                "real?",
                "Seconds of `self`'s own time before the window is expected to close; null where that is not known.",
            ),
        ],
        doc: "`self` noticed a window opened by `source` before it closed (03 §7.2). A rule may answer with `react` or `ask_reaction`; with nothing listening, nothing happens.",
    },
    EventInfo {
        name: WINDOW_CLOSED,
        subjects: &[("self?", "entity")],
        fields: &[
            WINDOW_FIELD,
            KIND_FIELD,
            ACTION_FIELD,
            ("offered", "int", "How many candidates it was offered to."),
            ("noticed", "int", "How many of them noticed it in time."),
            ("reacted", "int", "How many answered it with a reaction."),
        ],
        doc: "A window closed (03 §7.1): the phase that is its deadline began, its action ended, or its length was over. `self` is whose action or rule opened it. A candidate still waiting to notice it never does.",
    },
];

/// The type a raised field is checked against, from how it is written.
fn field_type(written: &str) -> FieldType {
    match written.trim_end_matches('?') {
        "real" => FieldType::Real,
        "int" => FieldType::Int,
        "bool" => FieldType::Bool,
        "string" => FieldType::Str,
        _ => FieldType::Any,
    }
}

/// The bus's type for each event of [`WINDOW_EVENTS`].
pub(crate) fn event_types() -> Vec<EventType> {
    WINDOW_EVENTS
        .iter()
        .map(|info| {
            let mut ty = EventType::new(info.name).doc(info.doc);
            for (name, kind) in info.subjects {
                ty = ty.subject(*name, *kind);
            }
            for (name, written, doc) in info.fields {
                ty = ty.field(*name, field_type(written), *doc);
            }
            ty
        })
        .collect()
}

/// Enters in a registry the kind `window`, unless it is there: called
/// before the engine's functions and events, which name it.
pub fn register_types(registry: &mut Registry) {
    if registry.def_type_id(WindowKindDef::TYPE).is_none() {
        let _ = registry.add_def_type(WindowKindDef::TYPE, WindowKindDef::DOC);
    }
    if registry.kind_id(WINDOW_KIND).is_none() {
        let _ = registry.add_kind(
            KindSpec::new(WINDOW_KIND)
                .doc("A reaction window (03 §7.1): an opportunity with a deadline that those who notice it may answer. A handle that outlives its window reads as closed."),
        );
    }
}

/// Enters in a registry the events of [`WINDOW_EVENTS`], leaving alone
/// whatever is there already.
pub fn register_events(registry: &mut Registry) {
    for info in WINDOW_EVENTS {
        if registry.event_id(info.name).is_some() {
            continue;
        }
        let mut spec = EventSpec::new(info.name).doc(info.doc);
        for (name, kind) in info.subjects.iter().chain(CAUSE_SUBJECTS) {
            if let Some(kind) = registry.kind_id(kind) {
                spec = spec.subject(name.trim_end_matches('?'), kind);
            }
        }
        for (name, written, doc) in info.fields {
            let ty = registry.parse_type(written).unwrap_or(Type::Any);
            spec = spec.field(*name, ty, *doc);
        }
        let _ = registry.add_event(spec);
    }
}

/// The open window a value names, if it names one.
fn open_window_of<'a>(sim: &'a Sim, value: Option<&Value>) -> Option<(WindowId, &'a Window)> {
    let id = sim.window_of(value?)?;
    sim.windows.get(id).map(|window| (id, window))
}

/// The functions of windows (03 §13).
pub fn functions() -> Vec<FunctionDef> {
    vec![
        FunctionDef::new(
            "window_open",
            "bool",
            GROUP_WINDOWS,
            "Whether a window is still open (03 §7.1).",
            |sim, args| Value::Bool(open_window_of(sim, args.first()).is_some()),
        )
        .arg("window", "window?", "The window.")
        .safe(),
        FunctionDef::new(
            "window_source",
            "entity?",
            GROUP_WINDOWS,
            "Whose action or rule opened a window that is still open; null for one that is closed or has no source. Safe for belief: a window is offered only to those who perceive its source (03 §7.2).",
            |sim, args| {
                open_window_of(sim, args.first())
                    .and_then(|(_, window)| window.source)
                    .map_or(Value::Null, |source| {
                        sim.subject_value(Subject::Entity(source))
                    })
            },
        )
        .arg("window", "window?", "The window.")
        .safe(),
        FunctionDef::new(
            "window_kind_of",
            "id<window_kind>?",
            GROUP_WINDOWS,
            "The kind of a window that is still open.",
            |sim, args| {
                let Some((_, window)) = open_window_of(sim, args.first()) else {
                    return Value::Null;
                };
                sim.defs()
                    .expr_registry()
                    .def_type_id(WindowKindDef::TYPE)
                    .map_or(Value::Null, |ty| {
                        Value::Id(clash_expr::DefRef::new(ty, u32::from(window.kind.raw())))
                    })
            },
        )
        .arg("window", "window?", "The window.")
        .safe(),
        FunctionDef::new(
            "window_time_left",
            "real?",
            GROUP_WINDOWS,
            "Seconds of `who`'s own time before a window is expected to close: when the phase that opened it ends, where that phase leads straight to the one that closes it, or when its length is over. Null for a window that is closed or whose deadline is not known.",
            |sim, args| {
                let (Some((id, _)), Some(Subject::Entity(who))) = (
                    open_window_of(sim, args.first()),
                    args.get(1).and_then(|value| sim.subject_of(value)),
                ) else {
                    return Value::Null;
                };
                sim.window_time_left(id, who)
                    .map_or(Value::Null, |left| Value::Real(left.as_secs_real()))
            },
        )
        .arg("window", "window?", "The window.")
        .arg("who", "entity", "Whose own time it is counted in."),
        FunctionDef::new(
            "away_from",
            "string?",
            GROUP_WINDOWS,
            "The step that leads straight away from `from` for `who`: `north`, `north_east`, `east`, `south_east`, `south`, `south_west`, `west` or `north_west`, by the signs of the shortest offset on the level. Null for two in one column, or either nowhere. What a reaction that leaves steps by (07 §5.2). Safe for belief, as `dir` is.",
            |sim, args| {
                let place = |value: Option<&Value>| match value.and_then(|value| sim.subject_of(value)) {
                    Some(Subject::Entity(id)) => sim.entities.location(id),
                    Some(Subject::Loc(loc)) => Some(loc),
                    _ => None,
                };
                let (Some(who), Some(from)) = (place(args.first()), place(args.get(1))) else {
                    return Value::Null;
                };
                let Some(delta) = sim.cosmos.delta(from, who) else {
                    return Value::Null;
                };
                let name = match (delta.x.signum(), delta.y.signum()) {
                    (0, -1) => "north",
                    (1, -1) => "north_east",
                    (1, 0) => "east",
                    (1, 1) => "south_east",
                    (0, 1) => "south",
                    (-1, 1) => "south_west",
                    (-1, 0) => "west",
                    (-1, -1) => "north_west",
                    _ => return Value::Null,
                };
                Value::from(name)
            },
        )
        .arg("who", "any", "Who steps: an entity or a cell.")
        .arg("from", "any", "What it steps away from: an entity or a cell.")
        .safe(),
    ]
}

/// Whether a wake-up of the `Action` domain is one of the windows'.
pub(crate) fn is_window_wake(arg: u64) -> bool {
    matches!(arg, WAKE_NOTICE | WAKE_WINDOW_TIMER)
}
