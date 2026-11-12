//! [`Sim::advance`]: the loop that moves time (03 §4.3).
//!
//! One iteration:
//!
//! 1. If the application has been asked for something, return it.
//! 2. Pop the next wake-up within the limit. While a decision is waiting
//!    for a directly controlled entity, that is only what is still due
//!    at the current tick. If there is none, return: the decision, if
//!    one is waiting.
//! 3. Dispatch it to the handler of its domain.
//! 4. Pump events: pass on what the world reported, then deliver queued
//!    events to their handlers until the queue is empty (05 §8).
//! 5. Safe point: reconcile tickets (02 §10.2), which loads and unloads
//!    tracts and moves the entities in them out of and into the world
//!    (see [`activity`](crate::activity)).
//!
//! Everything between two pops happens at one tick. `advance` never
//! waits and always returns (01 R5, invariant T7).
//!
//! # Adding a domain handler
//!
//! A wake-up carries a [`WakeDomain`], and the kernel hands it to that
//! domain's handler: a plain function `fn(&mut Sim, WakeId, Wake)`.
//! A package that schedules wake-ups writes one and registers it in
//! `Handlers::default` in this file, beside the others, so that every
//! simulation has it from the start:
//!
//! ```text
//! handlers.set(WakeDomain::Action, crate::action::on_wake);
//! ```
//!
//! A test, or a crate above this one, registers one on a simulation it
//! has made with [`Sim::set_wake_handler`]. Handlers are not state: a
//! loaded simulation has the defaults, and whatever was registered on
//! top must be registered again.
//!
//! The world's own wake-ups (collapse, bodies in flight, fields: the
//! `World` domain) go through the same loop. The world cannot see the
//! simulation, so its handler here is a few lines that hand the wake-up
//! to the world through [`Sim::world`]: see `world_wake`.

use std::collections::BTreeMap;

use clash_core::{StateHash, StateHasher, Tick};
use clash_time::{Wake, WakeDomain, WakeId};
use serde::{Deserialize, Serialize};

use crate::Sim;
use crate::decision::DecisionRequest;
use crate::event::{EventBody, SimEvent, Where};
use crate::reason::{Reason, codes};

/// The setting that limits how many events one pump may deliver
/// (03 §4.3).
pub const MAX_CASCADE_SETTING: &str = "rules.max_cascade";

/// The default of [`MAX_CASCADE_SETTING`].
pub const DEFAULT_MAX_CASCADE: i64 = 10_000;

/// How many wake-ups one call of [`Sim::advance`] handles at a single
/// tick before it reports a fault and yields (05 §13.2).
pub const MAX_WAKES_PER_TICK: u32 = 100_000;

/// How many event names the report of a cut-off cascade lists.
const CASCADE_TRACE_LEN: usize = 8;

/// How far [`Sim::advance`] may go.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum AdvanceLimit {
    /// Until a directly controlled entity must choose, or nothing is
    /// left to happen. Time does not move when nothing is scheduled.
    UntilDecision,
    /// Until this tick, or a decision before it. Time is moved on to the
    /// tick even if nothing is scheduled.
    Until(Tick),
    /// At most this many wake-ups, so that a frontend stays responsive.
    Work(u32),
}

/// Why [`Sim::advance`] returned.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Advance {
    /// A directly controlled entity must choose. Time will not move
    /// until a command answers.
    NeedInput(DecisionRequest),
    /// The limit was reached, or nothing is left to happen. The tick is
    /// the current time.
    Reached(Tick),
    /// The work budget is spent and more is due. Call again.
    Yielded,
    /// A rule asked the application for something (13 §9.1). Carry it
    /// out, now that the simulation is at a safe point, and call again.
    Request(AppRequest),
}

/// Something a rule asks the application to do to the simulation as a
/// whole (13 §9.1). The kernel carries requests from the rule that
/// raises one to whoever called `advance`; the effects that raise them
/// and the rest of the requests come with the scenario packages.
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum AppRequest {
    /// Make a save the player cannot make by hand.
    Checkpoint {
        /// The checkpoint's name.
        name: String,
    },
}

impl StateHash for AppRequest {
    fn state_hash(&self, hasher: &mut StateHasher) {
        match self {
            AppRequest::Checkpoint { name } => {
                hasher.write_u8(0);
                hasher.write_str(name);
            }
        }
    }
}

/// Handles the wake-ups of one domain: the handle the wake-up had and
/// what it carried. The wake-up has already left the timeline.
pub type WakeHandler = fn(&mut Sim, WakeId, Wake);

/// Who handles the wake-ups of each domain. Not state.
#[derive(Clone, Debug)]
pub(crate) struct Handlers {
    by_domain: BTreeMap<WakeDomain, WakeHandler>,
}

impl Handlers {
    /// Sets the handler of a domain and returns the one it had.
    pub(crate) fn set(&mut self, domain: WakeDomain, handler: WakeHandler) -> Option<WakeHandler> {
        self.by_domain.insert(domain, handler)
    }
}

/// The handlers every simulation starts with. A package that schedules
/// wake-ups adds its handler here.
impl Default for Handlers {
    fn default() -> Handlers {
        let mut handlers = Handlers {
            by_domain: BTreeMap::new(),
        };
        handlers.set(WakeDomain::World, world_wake);
        handlers.set(WakeDomain::Action, crate::action::run::action_wake);
        // Pools and bodies keep their wake-ups in the `Timer` domain, and
        // so do traits' durations: the handler of traits asks
        // `life::handle_wake` first and handles the rest.
        handlers.set(WakeDomain::Timer, crate::traits::on_timer);
        handlers.set(WakeDomain::Perception, crate::perception::on_wake);
        handlers
    }
}

/// Handles a wake-up of the world's: a stability check, a step of a
/// falling mass, and whatever other process the world has (02 §14).
///
/// The world's one entry for them is `WorldCtx::handle_wake`. It is
/// called inside [`Sim::world`], so what the process reports reaches
/// the event queue as it returns and the kernel follows whatever it
/// did to occupants. A process the world could not carry on is reported
/// as a fault.
fn world_wake(sim: &mut Sim, _id: WakeId, wake: Wake) {
    if let Err(error) = sim.world(|ctx| ctx.handle_wake(&wake)) {
        sim.fault(Reason::from_world(&error));
    }
}

impl Sim {
    /// Registers the handler of a wake-up domain, replacing the one it
    /// had, which is returned. See the [module documentation](self).
    pub fn set_wake_handler(
        &mut self,
        domain: WakeDomain,
        handler: WakeHandler,
    ) -> Option<WakeHandler> {
        self.handlers.set(domain, handler)
    }

    /// Asks the application to do something to the simulation as a
    /// whole. [`advance`](Sim::advance) returns the request at the next
    /// safe point.
    pub fn request_app(&mut self, request: AppRequest) {
        self.requests.push_back(request);
    }

    /// Moves the simulation forward: pops wake-ups in order and runs
    /// them until the limit is reached, a directly controlled entity
    /// must choose, a rule asks the application for something, or the
    /// work budget is spent (03 §4.3).
    ///
    /// Calling it again carries on exactly where it stopped: a run made
    /// of many `Work(n)` calls handles the same wake-ups in the same
    /// order as one long call.
    pub fn advance(&mut self, limit: AdvanceLimit) -> Advance {
        // Whatever simulation code did between calls is delivered first.
        self.pump();
        let mut handled: u32 = 0;
        let mut at_tick: (Tick, u32) = (self.now(), 0);
        loop {
            if let Some(request) = self.requests.pop_front() {
                return Advance::Request(request);
            }
            // While a decision is waiting, time does not move (T8). What
            // is still due at this very tick runs first, so that every
            // request raised at one tick is there to be returned in
            // entity order (03 §5.2).
            let waiting = !self.decisions.is_empty();
            let until = match (limit, waiting) {
                (AdvanceLimit::Until(tick), false) => tick,
                (AdvanceLimit::Until(tick), true) => tick.min(self.now()),
                (_, true) => self.now(),
                (_, false) => Tick::MAX,
            };
            let more_due = self.timeline.peek().is_some_and(|due| due <= until);
            let spent = matches!(limit, AdvanceLimit::Work(budget) if handled >= budget);
            if spent && more_due {
                return Advance::Yielded;
            }
            let next = if spent {
                None
            } else {
                self.timeline.pop(until)
            };
            let Some((id, wake)) = next else {
                if let Some(request) = self.pending_decision() {
                    return Advance::NeedInput(request);
                }
                if let AdvanceLimit::Until(tick) = limit {
                    self.timeline.advance_to(tick);
                }
                return Advance::Reached(self.now());
            };
            handled += 1;
            self.steps += 1;
            self.wakes.remove(id);
            self.dispatch(id, wake);
            self.pump();
            self.safe_point();

            if self.now() == at_tick.0 {
                at_tick.1 += 1;
            } else {
                at_tick = (self.now(), 1);
            }
            if at_tick.1 >= MAX_WAKES_PER_TICK {
                self.fault(
                    Reason::new(
                        codes::ADVANCE_TICK_LIMIT,
                        format!(
                            "{MAX_WAKES_PER_TICK} wake-ups ran at tick {} without time moving",
                            self.now()
                        ),
                    )
                    .with("tick", self.now().as_micros())
                    .with("limit", i64::from(MAX_WAKES_PER_TICK)),
                );
                return Advance::Yielded;
            }
        }
    }

    /// Hands a popped wake-up to the handler of its domain.
    fn dispatch(&mut self, id: WakeId, wake: Wake) {
        match self.handlers.by_domain.get(&wake.domain).copied() {
            Some(handler) => handler(self, id, wake),
            None => self.fault(
                Reason::new(
                    codes::ADVANCE_UNHANDLED_WAKE,
                    format!(
                        "a wake-up of domain {:?} was due and nothing handles that domain",
                        wake.domain
                    ),
                )
                .with("domain", format!("{:?}", wake.domain))
                .with("id", wake.id as i64)
                .with("arg", wake.arg as i64),
            ),
        }
    }

    /// Reports a fault the engine contained. It goes straight to the
    /// output: it is not delivered to handlers, so that a fault in
    /// delivering events cannot raise more of them.
    pub(crate) fn fault(&mut self, reason: Reason) {
        let event = self.stamped(
            Where::Global,
            clash_world::Cause::new(clash_world::CauseKind::Process),
            EventBody::Fault { reason },
        );
        self.out.push(event);
    }

    /// Pumps events (03 §4.3 step 4): passes on what the world reported
    /// and delivers queued events, oldest first, to their handlers and
    /// then to the output, until the queue is empty. Events raised while
    /// handling one join the back of the queue.
    ///
    /// If one pump delivers more than `rules.max_cascade` events, the
    /// rest are dropped and the chain is reported with a trace of the
    /// last events delivered. Content cannot hang the engine.
    pub fn pump(&mut self) {
        let limit = self
            .setting_int(MAX_CASCADE_SETTING, DEFAULT_MAX_CASCADE)
            .max(1) as u64;
        let mut delivered: u64 = 0;
        let mut trace: Vec<String> = Vec::new();
        loop {
            self.absorb_world();
            let Some(mut event) = self.events.pop_front() else {
                return;
            };
            if delivered >= limit {
                let dropped = 1 + self.events.len();
                self.events.clear();
                self.fault(
                    Reason::new(
                        codes::EVENTS_CASCADE_LIMIT,
                        format!(
                            "more than {limit} events were delivered in one pump; {dropped} were dropped after: {}",
                            trace.join(" > ")
                        ),
                    )
                    .with("limit", limit as i64)
                    .with("dropped", dropped as i64)
                    .with("trace", trace.join(" > ")),
                );
                return;
            }
            delivered += 1;
            if trace.len() == CASCADE_TRACE_LEN {
                trace.remove(0);
            }
            trace.push(event.name().to_owned());
            self.deliver(&mut event);
            // What the engine's own packages do about it, once rules
            // have had their say.
            crate::movement::on_event(self, &event);
            crate::perception::observe_event(self, &event);
            crate::device::on_event(self, &event);
            self.out.push(event);
        }
    }

    /// The safe point between two wake-ups (03 §4.3 step 5): where
    /// loading, unloading and saving are allowed. Tickets are
    /// reconciled with what is loaded (02 §10.2), and what that
    /// reported is delivered before anything else runs.
    pub(crate) fn safe_point(&mut self) {
        self.reconcile_activity();
        if !self.events.is_empty() {
            self.pump();
        }
    }

    /// The events waiting to be delivered: empty between calls.
    pub fn queued_events(&self) -> impl Iterator<Item = &SimEvent> {
        self.events.iter()
    }
}
