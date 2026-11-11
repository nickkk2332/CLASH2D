//! The rule runtime bound to the simulation (05 §13.1): [`SimRules`],
//! the simulation as a [`RuleHost`], and what a simulation sets up so
//! that rules answer its events.
//!
//! # How rules run against the simulation
//!
//! | Need | Use |
//! |------|-----|
//! | The rules of the definitions | [`Sim::rule_set`]: a [`RuleSet`], built when the simulation is set up and again after a load |
//! | A rule to answer an event | Nothing: every event type some rule listens for has one handler on the bus, which hands the event to [`RuleSet::deliver`] |
//! | To run a list of effects of another definition | [`Sim::run_effects`], with the values of the subjects and locals its scope bound and the [`Cause`] of whatever it changes |
//! | To call one rule | [`Sim::call_rule`] |
//! | A verb | A [`VerbDef`](crate::verb::VerbDef) in [`engine_verbs`](crate::verb::engine_verbs): found by the id the registry gave its name |
//!
//! # What a change made by a rule is caused by
//!
//! Every change a rule makes carries a [`Cause`]. Its kind is `eoc`, or
//! `action` for the effect lists of an action. Its actor is the entity
//! the rule is attached to; else the entity bound as `self` where the
//! effect is written; else the actor of the event being answered. Its
//! instigator is whoever is bound as `instigator`, else the instigator
//! of the event being answered, else the actor. An `emit` that names
//! `actor` or `instigator` itself is raised with those.
//!
//! # What is reported
//!
//! A fault of a rule, and a run-time warning of an expression a rule
//! evaluated, is reported as a `fault` event (03 §13), thinned by count
//! as 05 §4.5 says: a site reports its first three and then the
//! occurrences whose count is a power of two. The counts are not state.

use std::sync::Arc;

use clash_core::{Dur, EntityId, Rng, Tick};
use clash_data::Defs;
use clash_expr::{
    DefRef, FunctionId, Handle, Host, PropertyId, RuntimeWarning, Value, VerbId, WarningLimiter,
};
use clash_world::{Cause, CauseKind};

use super::host::{FaultLimiter, Raise, Raised, RuleFault, RuleHost, RuleLimits, RunView, VerbCall};
use super::interp::RunOutcome;
use super::state::{Occurrence, RuleSet, RuleState};
use super::{Effects, EocId, reason};
use crate::Sim;
use crate::bus::{Delivery, EventPayload, Handler};
use crate::event::{CAUSE_SUBJECTS, Subject, Where};
use crate::host::RULE_STREAM;
use crate::reason::Reason;
use crate::verb::{VerbFn, engine_verbs};

/// What the simulation sets up beside its state so that rules run: the
/// rules of its definitions, the verbs by id, and the counts that thin
/// reports. Not state: made with the simulation and again after a load.
#[derive(Clone, Debug)]
pub(crate) struct Runtime {
    /// The rules of the definitions.
    pub(crate) rules: Arc<RuleSet>,
    /// What carries out each verb, by the index of the id the registry
    /// gave it. `None` for a verb that is not the engine's.
    verbs: Vec<Option<VerbFn>>,
    faults: FaultLimiter,
    warnings: WarningLimiter,
    /// How many faults and warnings were reported, counting those that
    /// were thinned out.
    pub(crate) fault_count: u64,
}

impl Runtime {
    /// The runtime for a set of definitions.
    pub(crate) fn for_defs(defs: &Arc<Defs>) -> Runtime {
        let registry = defs.expr_registry();
        let mut verbs: Vec<Option<VerbFn>> = Vec::new();
        for verb in engine_verbs() {
            let Some(id) = registry.verb_id(verb.name) else {
                continue;
            };
            if verbs.len() <= id.index() {
                verbs.resize(id.index() + 1, None);
            }
            // Of two verbs of one name the first listed is the one the
            // registry holds.
            if let Some(slot) = verbs.get_mut(id.index())
                && slot.is_none()
            {
                *slot = Some(verb.run);
            }
        }
        Runtime {
            rules: Arc::new(RuleSet::new(Arc::clone(defs))),
            verbs,
            faults: FaultLimiter::new(),
            warnings: WarningLimiter::new(),
            fault_count: 0,
        }
    }

    /// What carries out a verb, if it is one of the engine's.
    fn verb(&self, id: VerbId) -> Option<VerbFn> {
        self.verbs.get(id.index()).copied().flatten()
    }
}

/// The simulation as the rule runtime's host: reads ground truth, and
/// writes through the simulation's own functions (05 E1, E2).
///
/// Made by [`Sim::rule_host`]. One is made for each event delivered and
/// each list of effects run; it holds the cause of whatever the run
/// changes.
#[derive(Debug)]
pub struct SimRules<'a> {
    sim: &'a mut Sim,
    rules: Arc<RuleSet>,
    /// What the run's changes are caused by, before the rule and its
    /// owner are known.
    base: Cause,
    /// What the next write is caused by: the base, with what the rule
    /// that is writing adds.
    writes: Cause,
    /// Who a text is being rendered for (05 §4.6).
    reader: Option<EntityId>,
}

impl<'a> SimRules<'a> {
    /// The simulation underneath.
    pub fn sim(&mut self) -> &mut Sim {
        self.sim
    }

    fn named(&self, handle: Handle) -> Option<Subject> {
        self.sim.exprs.subject(handle)
    }

    fn entity_of(&self, value: Option<&Value>) -> Option<EntityId> {
        self.sim
            .subject_of(value?)
            .and_then(|subject| subject.entity())
            .filter(|id| self.sim.entities.contains(*id))
    }

    /// The cause of what an effect does, given what is running (see the
    /// [module documentation](self)).
    pub fn cause(&self, by: &RunView<'_>) -> Cause {
        let owner = by
            .owner
            .and_then(|handle| self.named(handle))
            .and_then(|subject| subject.entity());
        let actor = owner
            .or_else(|| self.entity_of(by.subject(super::OWNER_SUBJECT)))
            .or(self.base.actor);
        let instigator = self
            .entity_of(by.subject("instigator"))
            .or(self.base.instigator)
            .or(actor);
        // What a rule does is a rule's doing, whoever called it.
        let kind = if by.rule.is_some() {
            CauseKind::Eoc
        } else {
            self.base.kind
        };
        Cause {
            actor,
            instigator,
            kind,
        }
    }

    /// Where an event a rule raises happened: with the first entity it
    /// binds, else at the first cell, else nowhere in particular.
    fn place_of(&self, subjects: &[(String, Subject)]) -> Where {
        let entity = subjects.iter().find_map(|(_, subject)| subject.entity());
        let loc = subjects.iter().find_map(|(_, subject)| subject.loc());
        match (entity, loc) {
            (Some(entity), _) => Where::Entity(entity),
            (None, Some(loc)) => Where::Loc(loc),
            (None, None) => Where::Global,
        }
    }
}

impl Host for SimRules<'_> {
    // The interpreter answers these three from the run in progress.
    fn subject(&mut self, _slot: u16) -> Value {
        Value::Null
    }

    fn local(&mut self, _slot: u16) -> Value {
        Value::Null
    }

    fn event_field(&mut self, _slot: u16) -> Value {
        Value::Null
    }

    fn property(&mut self, property: PropertyId, subject: Handle, args: &[Value]) -> Value {
        self.sim.property(property, subject, args)
    }

    fn variable(&mut self, subject: Handle, name: &str) -> Value {
        match self.named(subject) {
            Some(subject) => self.sim.var(subject, name),
            None => Value::Null,
        }
    }

    fn call(&mut self, function: FunctionId, args: &[Value]) -> Value {
        self.sim.call_function_for(function, args, self.reader)
    }

    fn rng(&mut self) -> Option<&mut Rng> {
        Some(self.sim.rng.stream(RULE_STREAM))
    }

    fn warn(&mut self, warning: RuntimeWarning) {
        self.sim.rt.fault_count += 1;
        let Some(count) = self.sim.rt.warnings.admit(&warning) else {
            return;
        };
        let defs = Arc::clone(self.rules.defs());
        let mut reason = Reason::new(reason::EXPRESSION_WARNING, warning.fault.to_string())
            .with("count", i64::try_from(count).unwrap_or(i64::MAX));
        if let Some(origin) = defs.exprs().origin(warning.origin) {
            reason = reason
                .with("definition", origin.definition.to_string())
                .with("path", origin.path.as_str());
        }
        self.sim.fault(reason);
    }

    fn def_name(&self, id: DefRef) -> Option<&str> {
        self.sim.exprs.registry().def_name(id)
    }
}

impl RuleHost for SimRules<'_> {
    fn rule_state(&mut self) -> &mut RuleState {
        &mut self.sim.rules
    }

    fn rule_limits(&self) -> RuleLimits {
        let defaults = RuleLimits::default();
        let setting = |key: &str, default: usize| {
            let default = i64::try_from(default).unwrap_or(i64::MAX);
            usize::try_from(self.sim.setting_int(key, default).max(1)).unwrap_or(usize::MAX)
        };
        RuleLimits {
            max_steps: setting("rules.max_steps", defaults.max_steps as usize) as u64,
            max_call_depth: setting("rules.max_call_depth", defaults.max_call_depth),
            max_queued: setting("rules.max_queued", defaults.max_queued),
            max_value_size: setting("rules.max_value_size", defaults.max_value_size),
            max_value_depth: setting("rules.max_value_depth", defaults.max_value_depth),
        }
    }

    fn local_now(&mut self, owner: Option<Handle>) -> Dur {
        let universal = self.sim.now().since(Tick::ZERO);
        let clock = match owner.and_then(|handle| self.named(handle)) {
            Some(Subject::Entity(id)) => self.sim.clock_of(id),
            Some(Subject::Loc(loc) | Subject::Element(loc, _)) => {
                Some(self.sim.location_clock(loc))
            }
            Some(Subject::Area(id)) => self.sim.cosmos.area(id).and_then(|area| area.clock),
            Some(Subject::Realm(id)) => self.sim.cosmos.realm(id).map(|realm| realm.clock),
            _ => None,
        };
        clock
            .and_then(|clock| self.sim.timeline.local_now(clock).ok())
            .unwrap_or(universal)
    }

    fn global(&mut self) -> Value {
        self.sim.subject_value(Subject::Global)
    }

    fn simulated(&mut self, subject: Handle) -> bool {
        match self.named(subject) {
            // An entity that is not in the world is stored (05 §13.5).
            Some(Subject::Entity(id)) => self.sim.entities.location(id).is_some(),
            Some(subject) => self.sim.exists(subject),
            None => false,
        }
    }

    fn writing(&mut self, by: &RunView<'_>) {
        self.writes = self.cause(by);
    }

    fn set_variable(&mut self, subject: Handle, name: &str, value: Value) -> Result<(), Reason> {
        let subject = self.named(subject).ok_or_else(not_a_subject)?;
        self.sim
            .set_var(subject, name, value, self.writes)
            .map(|_| ())
    }

    fn unset_variable(&mut self, subject: Handle, name: &str) -> Result<(), Reason> {
        let subject = self.named(subject).ok_or_else(not_a_subject)?;
        self.sim
            .set_var(subject, name, Value::Null, self.writes)
            .map(|_| ())
    }

    fn set_property(
        &mut self,
        property: PropertyId,
        subject: Handle,
        value: Value,
    ) -> Result<(), Reason> {
        let subject = self.named(subject).ok_or_else(not_a_subject)?;
        self.sim
            .set_property(property, subject, &value, self.writes)
    }

    fn call_verb(&mut self, call: &VerbCall<'_>) -> Result<Value, Reason> {
        let Some(run) = self.sim.rt.verb(call.verb) else {
            let name = self
                .rules
                .defs()
                .expr_registry()
                .verb(call.verb)
                .map_or("?", |verb| verb.spec.name.as_str());
            return Err(Reason::new(
                reason::VERB_UNBOUND,
                format!("nothing in the engine carries out `{name}`"),
            )
            .with("verb", name));
        };
        let cause = self.cause(&call.by);
        run(self.sim, call, cause)
    }

    fn raise(&mut self, raise: &Raise<'_>) -> Result<Raised, Reason> {
        let defs = Arc::clone(self.rules.defs());
        let registry = defs.expr_registry();
        let Some(spec) = registry.event(raise.event).map(|event| &event.spec) else {
            return Err(Reason::new(
                crate::reason::codes::EVENTS_UNKNOWN_TYPE,
                "there is no such event type",
            ));
        };
        let mut cause = self.cause(&raise.by);
        let mut payload = EventPayload::new(spec.name.clone());
        for ((name, _), value) in spec.subjects.iter().zip(raise.subjects) {
            let from_cause = CAUSE_SUBJECTS
                .iter()
                .any(|(cause, _)| cause.trim_end_matches('?') == name);
            if from_cause {
                // Who acted and who is behind it come from the cause: a
                // rule that names them says who they are.
                if let Some(entity) = self.entity_of(Some(value)) {
                    if name == "actor" {
                        cause.actor = Some(entity);
                    } else {
                        cause.instigator = Some(entity);
                    }
                }
                continue;
            }
            if let Some(subject) = self.sim.subject_of(value) {
                payload = payload.subject(name.clone(), subject);
            }
        }
        for (field, value) in spec.fields.iter().zip(raise.fields) {
            payload = payload.with(field.name.clone(), self.sim.field_of(value));
        }
        let on = self.place_of(&payload.subjects);
        let raised = self.sim.raise(on, cause, payload)?;
        let fields = spec
            .fields
            .iter()
            .map(|field| {
                raised
                    .field(&field.name)
                    .map_or(Value::Null, |value| self.sim.exprs.field_value(value))
            })
            .collect();
        Ok(Raised {
            fields,
            cancelled: raised.cancelled,
        })
    }

    fn fault(&mut self, fault: RuleFault) {
        self.sim.rt.fault_count += 1;
        if let Some(count) = self.sim.rt.faults.admit(&fault) {
            let reason = self
                .rules
                .fault_reason(&fault)
                .with("count", i64::try_from(count).unwrap_or(i64::MAX));
            self.sim.fault(reason);
        }
    }
}

/// The reason for a handle that names nothing of the simulation's.
fn not_a_subject() -> Reason {
    Reason::new(
        crate::reason::codes::SUBJECT_GONE,
        "what this is to be written to is not something the simulation keeps",
    )
}

/// Hands one event to the rules that answer it, and writes back what
/// they did to it: the handler the simulation registers for every event
/// type some rule listens for.
fn deliver(sim: &mut Sim, delivery: &mut Delivery<'_>) {
    let rules = Arc::clone(&sim.rt.rules);
    let defs = Arc::clone(rules.defs());
    let registry = defs.expr_registry();
    let event = delivery.event();
    let Some(id) = registry.event_id(event.name()) else {
        return;
    };
    let Some(bound) = sim.exprs.bind_event(event) else {
        return;
    };
    let cause = Cause {
        kind: CauseKind::Eoc,
        ..event.cause
    };
    let mut occurrence = Occurrence::new(id, bound.subjects, bound.fields);
    if delivery.is_hook() {
        occurrence = occurrence.as_hook();
    }
    occurrence.consumed = delivery.is_consumed();
    rules.deliver(&mut sim.rule_host(cause), &mut occurrence);

    if let Some(spec) = registry.event(id).map(|event| &event.spec) {
        for slot in occurrence.changed() {
            let (Some(field), Some(value)) = (
                spec.fields.get(usize::from(*slot)),
                occurrence.fields.get(usize::from(*slot)),
            ) else {
                continue;
            };
            // A refusal is kept by the delivery and reported as a fault.
            let _ = delivery.set_field(&field.name, sim.field_of(value));
        }
    }
    if occurrence.consumed {
        delivery.consume();
    }
    if let Some(reason) = occurrence.cancelled {
        let _ = delivery.cancel(reason);
    }
}

impl Sim {
    /// The rules of the simulation's definitions (05 §9). Not state.
    pub fn rule_set(&self) -> &Arc<RuleSet> {
        &self.rt.rules
    }

    /// The simulation as the rule runtime's host, for one run: whatever
    /// the run changes is caused by `cause`, with the actor and the
    /// instigator the running rule adds (see the
    /// [module documentation](self)).
    pub fn rule_host(&mut self, cause: Cause) -> SimRules<'_> {
        let rules = Arc::clone(&self.rt.rules);
        SimRules {
            sim: self,
            rules,
            base: cause,
            writes: cause,
            reader: None,
        }
    }

    /// Runs a list of effects that belongs to another definition
    /// against the simulation (05 §10): an action's `on_start`, a
    /// phase's `do`.
    ///
    /// `subjects` and `locals` are the values of what the list's scope
    /// bound, in the order it bound them; `set` and `add` change the
    /// locals in place. `owner` is what the list belongs to, for the
    /// cause of what it does. A list that was never compiled does
    /// nothing.
    pub fn run_effects(
        &mut self,
        effects: &Effects,
        subjects: &[Value],
        locals: &mut Vec<Value>,
        owner: Option<Subject>,
        cause: Cause,
    ) -> RunOutcome {
        let rules = Arc::clone(&self.rt.rules);
        let owner = owner.and_then(|owner| self.exprs.kinds().handle(owner));
        rules.run_effects(&mut self.rule_host(cause), effects, subjects, locals, owner)
    }

    /// Calls one rule from outside any rule (05 §9.5): for a console, a
    /// screen, a test. `subjects` are the subjects the rule is given, by
    /// name, and `args` its parameters, by name. What comes back is
    /// what the rule returned; it is refused if there is no such rule
    /// or the arguments do not fit its parameters.
    pub fn call_rule(
        &mut self,
        rule: EocId,
        subjects: &[(&str, Value)],
        args: &[(&str, Value)],
        cause: Cause,
    ) -> Result<Value, Reason> {
        let rules = Arc::clone(&self.rt.rules);
        rules.call(&mut self.rule_host(cause), rule, subjects, args)
    }

    /// How many faults and run-time warnings rules have raised since
    /// the simulation was set up, counting those thinned out of the
    /// report. Not state.
    pub fn rule_fault_count(&self) -> u64 {
        self.rt.fault_count
    }

    /// Registers what makes rules answer events: one handler for each
    /// event type some rule listens for. Called when the simulation is
    /// set up and again after a load; handlers are not state.
    pub(crate) fn bind_rules(&mut self) {
        let rules = Arc::clone(&self.rt.rules);
        let defs = Arc::clone(rules.defs());
        let names: Vec<String> = defs
            .expr_registry()
            .events()
            .filter(|event| {
                defs.expr_registry()
                    .event_id(&event.spec.name)
                    .is_some_and(|id| rules.listens(id))
            })
            .map(|event| event.spec.name.clone())
            .collect();
        for name in names {
            // An event type the bus does not know is never raised.
            let _ = self.on(&name, Handler::new(deliver));
        }
    }
}

impl Sim {
    /// Ends what an entity that is about to be removed was part of and
    /// forgets what rules kept for it: the rules attached to it and
    /// when rules last ran for it (05 §9.3).
    pub(crate) fn release_entity(&mut self, id: EntityId, _cause: Cause) {
        if let Some(handle) = self.exprs.kinds().handle(Subject::Entity(id)) {
            self.rules.forget(handle);
        }
    }
}
