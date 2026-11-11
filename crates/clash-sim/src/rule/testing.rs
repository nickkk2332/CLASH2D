//! [`FakeHost`]: a small deterministic host for tests of rules.
//!
//! It is a handful of numbered subjects with variables and properties
//! in maps, a few verbs that write down what they were called with, a
//! log of the events raised and a queue of notifications delivered by
//! [`FakeHost::pump`]. It implements [`RuleHost`] in full, so that
//! every effect and every limit can be tested before the simulation is
//! bound to the rule runtime, and so that later packages can test their
//! own rules against something small. None of it is engine content: it
//! exists only here.
//!
//! # What is registered
//!
//! | Sort | Entries |
//! |------|---------|
//! | Kinds | `entity`, `loc`, `site`, `global` |
//! | `entity` | `health` real (settable), `sealed` real (settable; the host refuses every write), `name` string, `loc` loc |
//! | `loc` | `site` site |
//! | Functions | the built-ins; `setting(name)`, which reads [`FakeHost::settings`]; `the(x)` and `name(x)`, which read the property `name` |
//! | Verbs | `note(text, to?)` with `text` direct; `hurt(who, amount, kind?)`, which lowers `health`; `refuse(why?)`, which always declines with the reason `fake.refused` |
//! | Events | `alarm_tripped` (subjects `here` loc, `who` entity; fields `loud` bool, `kind` string, `energy` real); `strike_incoming`, a hook (subject `target` entity; fields `energy` real, mutable, and `kind` string); `sim_started`, which only the host may raise |
//!
//! Events that the loaded mods define with `event` definitions are
//! there too, entered by the rule runtime's registry step.

use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::sync::Arc;

use clash_core::{Dur, Real, Rng};
use clash_expr::{
    DefRef, EventId, EventSpec, FunctionId, FunctionSpec, Handle, Host, KindId, KindSpec,
    PropertyId, PropertySpec, Registry, RuntimeWarning, Type, Value, VerbSpec,
};

use super::host::{FaultLimiter, Raise, Raised, RuleFault, RuleHost, RuleLimits, VerbCall};
use super::state::{Occurrence, RuleSet, RuleState};
use super::{EocId, GLOBAL_SUBJECT};
use crate::reason::Reason;

/// How many hooks the fake host delivers inside one another, as the
/// kernel's bus does (05 §13.2).
pub const MAX_HOOK_DEPTH: u8 = crate::bus::MAX_HOOK_DEPTH;

/// How many notifications one [`FakeHost::pump`] delivers before it
/// gives up, as the kernel's pump does.
pub const MAX_CASCADE: usize = 10_000;

/// The code of the reason the verb `refuse` declines with.
pub const REFUSED: &str = "fake.refused";
/// The code of the reason a write to `sealed` is refused with.
pub const SEALED: &str = "fake.sealed";

const ENTITY: &str = "entity";
const LOC: &str = "loc";
const SITE: &str = "site";

/// A host for tests. See the [module](self) documentation.
#[derive(Clone, Debug)]
pub struct FakeHost {
    rules: Arc<RuleSet>,
    entity: KindId,
    loc: KindId,
    site: KindId,
    global: KindId,
    /// What the rule runtime keeps between runs.
    pub state: RuleState,
    /// The limits runs are held to.
    pub limits: RuleLimits,
    /// The local time of every clock that has none of its own.
    pub clock: Dur,
    /// The local time of the clocks of particular subjects.
    pub clocks: BTreeMap<Handle, Dur>,
    /// What `setting(name)` reads.
    pub settings: BTreeMap<String, Value>,
    variables: BTreeMap<(Handle, String), Value>,
    properties: BTreeMap<(Handle, String), Value>,
    /// Every verb call, in order, as `name(arg, arg)`.
    pub verbs: Vec<String>,
    /// Every event raised, in order, as `name[subjects](fields)`.
    pub events: Vec<String>,
    /// The faults the limiter let through, in order.
    pub faults: Vec<RuleFault>,
    /// How many faults there were, counting those the limiter dropped.
    pub fault_count: u64,
    /// Every run-time warning from an expression, in order.
    pub warnings: Vec<RuntimeWarning>,
    /// The stream random choices draw from, if there is one.
    pub rng: Option<Rng>,
    pending: VecDeque<Occurrence>,
    hook_depth: u8,
    carried: BTreeMap<Handle, Vec<EocId>>,
    stored: BTreeSet<Handle>,
    limiter: FaultLimiter,
}

impl FakeHost {
    /// The registry to give the loader: the kinds, properties,
    /// functions, verbs and events of the [module](self) documentation.
    ///
    /// # Panics
    ///
    /// Never: the names are distinct.
    pub fn registry() -> Registry {
        let mut registry = Registry::with_builtins();
        let mut kind = |name: &str, doc: &str| {
            registry
                .add_kind(KindSpec::new(name).doc(doc))
                .expect("the fake kinds have distinct names")
        };
        let entity = kind(ENTITY, "A numbered thing.");
        let loc = kind(LOC, "A numbered place.");
        let site = kind(SITE, "A numbered site.");
        let _global = kind(GLOBAL_SUBJECT, "Everything.");
        let subject = Type::Subject;
        let properties = [
            PropertySpec::new(entity, "health", Type::Real)
                .settable(true)
                .doc("How well it is."),
            PropertySpec::new(entity, "sealed", Type::Real)
                .settable(true)
                .doc("Something the host refuses to change."),
            PropertySpec::new(entity, "name", Type::Str).doc("What it is called."),
            PropertySpec::new(entity, "loc", subject(loc)).doc("Where it is."),
            PropertySpec::new(loc, "site", subject(site)).doc("The site the place is in."),
        ];
        for spec in properties {
            registry
                .add_property(spec)
                .expect("the fake properties have distinct names");
        }
        registry
            .add_function(
                FunctionSpec::new("setting", Type::Any)
                    .param("name", Type::Str, "The setting's name.")
                    .doc("What a setting is set to, or null."),
            )
            .expect("the fake function has a name of its own");
        let verbs = [
            VerbSpec::new("note")
                .arg("text", Type::Str, "What to write down.")
                .optional_arg("to", subject(entity), "For whom.")
                .direct("text")
                .doc("Writes a line down."),
            VerbSpec::new("hurt")
                .arg("who", subject(entity), "Whose health goes down.")
                .arg("amount", Type::Real, "By how much.")
                .optional_arg("kind", Type::Str, "The kind of harm.")
                .doc("Lowers health."),
            VerbSpec::new("refuse")
                .optional_arg("why", Type::Str, "What to say.")
                .doc("Always declines."),
        ];
        for spec in verbs {
            registry
                .add_verb(spec)
                .expect("the fake verbs have distinct names");
        }
        let events = [
            EventSpec::new("alarm_tripped")
                .subject("here", loc)
                .subject("who", entity)
                .field("loud", Type::Bool, "Whether it is loud.")
                .field("kind", Type::Str, "What set it off.")
                .field("energy", Type::Real, "How hard.")
                .doc("An alarm went off."),
            EventSpec::new("strike_incoming")
                .subject("target", entity)
                .field("energy", Type::Real, "Joules about to land.")
                .field("kind", Type::Str, "The kind of blow.")
                .hook(["energy"])
                .doc("Precedes a blow. A handler may change the energy."),
            EventSpec::new("sim_started").doc("A game began. Only the host raises it."),
        ];
        for spec in events {
            registry
                .add_event(spec)
                .expect("the fake events have distinct names");
        }
        registry
    }

    /// A host over the rules of a load made with
    /// [`FakeHost::registry`].
    ///
    /// # Panics
    ///
    /// Panics if the definitions were loaded with another registry: a
    /// mistake in a test.
    pub fn new(rules: Arc<RuleSet>) -> FakeHost {
        let registry = rules.defs().expr_registry().clone();
        let kind = |name: &str| {
            registry
                .kind_id(name)
                .expect("the definitions were loaded with `FakeHost::registry`")
        };
        FakeHost {
            entity: kind(ENTITY),
            loc: kind(LOC),
            site: kind(SITE),
            global: kind(GLOBAL_SUBJECT),
            rules,
            state: RuleState::default(),
            limits: RuleLimits::default(),
            clock: Dur::ZERO,
            clocks: BTreeMap::new(),
            settings: BTreeMap::new(),
            variables: BTreeMap::new(),
            properties: BTreeMap::new(),
            verbs: Vec::new(),
            events: Vec::new(),
            faults: Vec::new(),
            fault_count: 0,
            warnings: Vec::new(),
            rng: Some(Rng::from_seed(1)),
            pending: VecDeque::new(),
            hook_depth: 0,
            carried: BTreeMap::new(),
            stored: BTreeSet::new(),
            limiter: FaultLimiter::new(),
        }
    }

    /// The rules the host runs.
    pub fn rules(&self) -> &Arc<RuleSet> {
        &self.rules
    }

    /// The handle of the entity with this number.
    pub const fn entity(&self, number: u64) -> Handle {
        Handle::new(self.entity, number)
    }

    /// The handle of the place with this number.
    pub const fn place(&self, number: u64) -> Handle {
        Handle::new(self.loc, number)
    }

    /// The handle of the site with this number.
    pub const fn site(&self, number: u64) -> Handle {
        Handle::new(self.site, number)
    }

    /// The handle of the `global` subject.
    pub const fn global_handle(&self) -> Handle {
        Handle::new(self.global, 0)
    }

    /// Sets a variable of a subject, by qualified name.
    pub fn set_var(&mut self, subject: Handle, name: &str, value: impl Into<Value>) {
        self.variables
            .insert((subject, name.to_owned()), value.into());
    }

    /// A variable of a subject as it reads: what was set, or else its
    /// declared default.
    pub fn var(&mut self, subject: Handle, name: &str) -> Value {
        self.variable(subject, name)
    }

    /// Whether a variable of a subject has been set.
    pub fn has_var(&self, subject: Handle, name: &str) -> bool {
        self.variables.contains_key(&(subject, name.to_owned()))
    }

    /// Sets what a property of a subject reads as.
    pub fn set_prop(&mut self, subject: Handle, property: &str, value: impl Into<Value>) {
        self.properties
            .insert((subject, property.to_owned()), value.into());
    }

    /// What a property of a subject reads as.
    pub fn prop(&self, subject: Handle, property: &str) -> Value {
        self.properties
            .get(&(subject, property.to_owned()))
            .cloned()
            .unwrap_or_default()
    }

    /// Makes a subject carry a rule through its definition, as an area
    /// kind's `rules` would (05 §9.3).
    pub fn carry(&mut self, subject: Handle, rule: EocId) {
        self.carried.entry(subject).or_default().push(rule);
    }

    /// Takes a subject out of the simulation: its attached rules stop
    /// running (05 §13.5).
    pub fn store(&mut self, subject: Handle) {
        self.stored.insert(subject);
    }

    /// Brings a stored subject back.
    pub fn restore(&mut self, subject: Handle) {
        self.stored.remove(&subject);
    }

    /// The codes of the faults reported, in order.
    pub fn fault_codes(&self) -> Vec<&'static str> {
        self.faults.iter().map(|fault| fault.code).collect()
    }

    /// The event type with this name.
    ///
    /// # Panics
    ///
    /// Panics if there is none: a mistake in a test.
    pub fn event(&self, name: &str) -> EventId {
        self.rules
            .defs()
            .expr_registry()
            .event_id(name)
            .unwrap_or_else(|| panic!("no event `{name}`"))
    }

    /// Raises an event as the engine would: a hook is delivered at once
    /// and a notification at the next [`pump`](FakeHost::pump).
    /// `subjects` and `fields` are in the order the type declares them.
    pub fn raise_event(&mut self, name: &str, subjects: &[Value], fields: &[Value]) -> Raised {
        let event = self.event(name);
        self.raise_checked(event, subjects, fields)
    }

    fn raise_checked(&mut self, event: EventId, subjects: &[Value], fields: &[Value]) -> Raised {
        let rules = self.rules.clone();
        let registry = rules.defs().expr_registry();
        let (name, hook) = registry.event(event).map_or(("?", false), |event| {
            (event.spec.name.as_str(), event.spec.hook)
        });
        let show = |values: &[Value]| {
            values
                .iter()
                .map(Value::to_string)
                .collect::<Vec<_>>()
                .join(", ")
        };
        self.events
            .push(format!("{name}[{}]({})", show(subjects), show(fields)));
        let occurrence = Occurrence::new(event, subjects.to_vec(), fields.to_vec());
        if !hook {
            self.pending.push_back(occurrence);
            return Raised {
                fields: fields.to_vec(),
                cancelled: None,
            };
        }
        if self.hook_depth >= MAX_HOOK_DEPTH {
            self.fault(RuleFault {
                code: crate::reason::codes::EVENTS_HOOK_DEPTH,
                message: format!(
                    "hook `{name}` was raised from inside {MAX_HOOK_DEPTH} others and was skipped"
                ),
                origin: 0,
                rule: None,
                refusal: None,
            });
            return Raised {
                fields: fields.to_vec(),
                cancelled: None,
            };
        }
        let mut occurrence = occurrence.as_hook();
        self.hook_depth += 1;
        rules.deliver(self, &mut occurrence);
        self.hook_depth -= 1;
        Raised {
            fields: occurrence.fields,
            cancelled: occurrence.cancelled,
        }
    }

    /// Delivers the notifications waiting, and those raised while they
    /// are delivered, in order. Returns how many were delivered. Past
    /// [`MAX_CASCADE`] the rest are dropped and that is reported, as
    /// the kernel's pump does.
    pub fn pump(&mut self) -> usize {
        let rules = self.rules.clone();
        let mut delivered = 0;
        while let Some(mut occurrence) = self.pending.pop_front() {
            if delivered >= MAX_CASCADE {
                let dropped = self.pending.len() + 1;
                self.pending.clear();
                self.fault(RuleFault {
                    code: crate::reason::codes::EVENTS_CASCADE_LIMIT,
                    message: format!(
                        "one pump delivered {MAX_CASCADE} events; {dropped} more were dropped"
                    ),
                    origin: 0,
                    rule: None,
                    refusal: None,
                });
                break;
            }
            rules.deliver(self, &mut occurrence);
            delivered += 1;
        }
        delivered
    }

    /// How many notifications are waiting for the next pump.
    pub fn pending(&self) -> usize {
        self.pending.len()
    }

    fn name_of(&self, value: &Value) -> String {
        value
            .as_subject()
            .and_then(|handle| self.properties.get(&(handle, "name".to_owned())))
            .and_then(Value::as_str)
            .map_or_else(|| "something".to_owned(), str::to_owned)
    }
}

impl Host for FakeHost {
    fn subject(&mut self, _slot: u16) -> Value {
        Value::Null
    }

    fn local(&mut self, _slot: u16) -> Value {
        Value::Null
    }

    fn event_field(&mut self, _slot: u16) -> Value {
        Value::Null
    }

    fn property(&mut self, property: PropertyId, subject: Handle, _args: &[Value]) -> Value {
        let rules = self.rules.clone();
        let Some(name) = rules
            .defs()
            .expr_registry()
            .property(property)
            .map(|found| found.spec.name.as_str())
        else {
            return Value::Null;
        };
        self.prop(subject, name)
    }

    fn variable(&mut self, subject: Handle, name: &str) -> Value {
        match self.variables.get(&(subject, name.to_owned())) {
            Some(value) => value.clone(),
            None => self.rules.defs().variables().default_of(name),
        }
    }

    fn call(&mut self, function: FunctionId, args: &[Value]) -> Value {
        let rules = self.rules.clone();
        let Some(name) = rules
            .defs()
            .expr_registry()
            .function(function)
            .map(|found| found.spec.name.as_str())
        else {
            return Value::Null;
        };
        let first = args.first().unwrap_or(&Value::Null);
        match name {
            "setting" => first
                .as_str()
                .and_then(|setting| self.settings.get(setting))
                .cloned()
                .unwrap_or_default(),
            "name" => Value::from(self.name_of(first)),
            "the" => Value::from(format!("the {}", self.name_of(first))),
            _ => Value::Null,
        }
    }

    fn rng(&mut self) -> Option<&mut Rng> {
        self.rng.as_mut()
    }

    fn warn(&mut self, warning: RuntimeWarning) {
        self.warnings.push(warning);
    }

    fn def_name(&self, id: DefRef) -> Option<&str> {
        self.rules.defs().expr_registry().def_name(id)
    }
}

impl RuleHost for FakeHost {
    fn rule_state(&mut self) -> &mut RuleState {
        &mut self.state
    }

    fn rule_limits(&self) -> RuleLimits {
        self.limits
    }

    fn local_now(&mut self, owner: Option<Handle>) -> Dur {
        owner
            .and_then(|owner| self.clocks.get(&owner).copied())
            .unwrap_or(self.clock)
    }

    fn global(&mut self) -> Value {
        Value::Subject(self.global_handle())
    }

    fn simulated(&mut self, subject: Handle) -> bool {
        !self.stored.contains(&subject)
    }

    fn rules_of(&mut self, subject: Handle, out: &mut Vec<EocId>) {
        if let Some(rules) = self.carried.get(&subject) {
            out.extend_from_slice(rules);
        }
    }

    fn set_variable(&mut self, subject: Handle, name: &str, value: Value) -> Result<(), Reason> {
        self.variables.insert((subject, name.to_owned()), value);
        Ok(())
    }

    fn unset_variable(&mut self, subject: Handle, name: &str) -> Result<(), Reason> {
        self.variables.remove(&(subject, name.to_owned()));
        Ok(())
    }

    fn set_property(
        &mut self,
        property: PropertyId,
        subject: Handle,
        value: Value,
    ) -> Result<(), Reason> {
        let rules = self.rules.clone();
        let name = rules
            .defs()
            .expr_registry()
            .property(property)
            .map_or("?", |found| found.spec.name.as_str());
        if name == "sealed" {
            return Err(Reason::new(SEALED, "it is sealed"));
        }
        self.set_prop(subject, name, value);
        Ok(())
    }

    fn call_verb(&mut self, call: &VerbCall<'_>) -> Result<Value, Reason> {
        let rules = self.rules.clone();
        let name = rules
            .defs()
            .expr_registry()
            .verb(call.verb)
            .map_or("?", |found| found.spec.name.as_str());
        let shown: Vec<String> = call.args.iter().map(Value::to_string).collect();
        self.verbs.push(format!("{name}({})", shown.join(", ")));
        match name {
            "hurt" => {
                let who = call.args.first().and_then(Value::as_subject);
                let amount = call.args.get(1).and_then(Value::as_number);
                if let (Some(who), Some(amount)) = (who, amount) {
                    let health = self.prop(who, "health").as_number().unwrap_or(Real::ZERO);
                    self.set_prop(who, "health", Value::Real(health - amount));
                }
                Ok(Value::Null)
            }
            "refuse" => {
                let why = call
                    .args
                    .first()
                    .and_then(Value::as_str)
                    .unwrap_or("the fake host refuses");
                Err(Reason::new(REFUSED, why))
            }
            _ => Ok(Value::Null),
        }
    }

    fn raise(&mut self, raise: &Raise<'_>) -> Result<Raised, Reason> {
        let rules = self.rules.clone();
        let name = rules
            .defs()
            .expr_registry()
            .event(raise.event)
            .map_or("?", |event| event.spec.name.as_str());
        if name == "sim_started" {
            return Err(Reason::new(
                crate::reason::codes::EVENTS_BAD_PAYLOAD,
                "event `sim_started`: it is one of the engine's own events and is raised only by the engine",
            )
            .with("event", name));
        }
        Ok(self.raise_checked(raise.event, raise.subjects, raise.fields))
    }

    fn fault(&mut self, fault: RuleFault) {
        self.fault_count += 1;
        if self.limiter.admit(&fault).is_some() {
            self.faults.push(fault);
        }
    }
}
