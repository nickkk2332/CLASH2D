//! Reactive expressions (05 §7.5): a pure expression whose value the
//! engine keeps current by re-evaluating it when, and only when,
//! something it read has changed.
//!
//! # At load
//!
//! `check_reactive_cond` refuses, at the field, a reactive condition
//! that reads anything nothing says how it changes: a property
//! registered with no `changed_by` events (unless it is one of the
//! constant accessors of [`CONSTANT_PROPERTIES`]), a function of the
//! host registered with none, a function of a mod (whose body may read
//! anything), a query, and a structure of `all`, `any` and `not`, which
//! is written instead as one expression with `and`, `or` and `not`. A
//! random function is refused by the field's purity. Variables may be
//! read: any write to one marks what read it stale.
//!
//! # While the simulation runs
//!
//! Each watched expression belongs to a [`Watcher`]: the `while` of an
//! ability instance or of a trait instance. When one is evaluated, a
//! recording host notes every property of every subject it read and
//! every variable; each property becomes a subscription to the events
//! that change it, for that subject. A subject that is not one of the
//! kernel's kinds is followed by the entity it belongs to: an ability
//! by its owner, an entity's attributes, pools and tracks by the
//! entity.
//!
//! The kernel hands every event, once delivered, to `event_delivered`,
//! which marks stale whatever subscribed to that event for one of its
//! subjects; `var_written` does the same for a variable. At the end of
//! the pump (03 §4.3) `settle` re-evaluates what is stale, once each,
//! in the order of [`Watcher`], and tells the owner of each whose value
//! changed. If telling makes others stale they are evaluated in another
//! round, at most `rules.max_reactive_rounds` rounds (default 8) in one
//! pump; what is still stale then is reported as a fault, with the
//! chain, and left for the next pump. Nothing is polled.
//!
//! The subscriptions are not state: they follow from the saved state,
//! and a simulation made or loaded builds them afresh at its first pump
//! (`rebuild`), so a watched value always equals a fresh evaluation
//! (invariant A6).

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use clash_core::EntityId;
use clash_data::{CompileCx, Cond, Runner};
use clash_expr::{
    DefRef, ExprKind, Fault, FunctionId, Handle, Host, PropertyId, QueryCall, Registry,
    RuntimeWarning, Value,
};
use clash_world::{Cause, CauseKind};

use super::defs::{AbilityDef, report_at};
use super::{Abilities, State};
use crate::Sim;
use crate::event::{SimEvent, Subject};
use crate::host::{Bound, SimHost};
use crate::reason::Reason;
use crate::traits::{TraitDef, TraitKey, Traits};

/// The setting that limits how many rounds of re-evaluation one pump
/// runs (05 §7.5).
pub const MAX_REACTIVE_ROUNDS_SETTING: &str = "rules.max_reactive_rounds";

/// The default of [`MAX_REACTIVE_ROUNDS_SETTING`].
pub const DEFAULT_MAX_REACTIVE_ROUNDS: i64 = 8;

/// The code of the fault reported when expressions are still stale
/// after the last round of a pump.
pub const REACTIVE_ROUNDS: &str = "reactive.rounds";

/// The code of the fault reported when a watched expression read
/// something nothing says how it changes.
pub const REACTIVE_UNWATCHABLE: &str = "reactive.unwatchable";

/// Properties that need no events: their value never changes for the
/// subject they are read of. By kind and name.
pub const CONSTANT_PROPERTIES: &[(&str, &str)] = &[
    ("entity", "attr"),
    ("entity", "pool"),
    ("entity", "track"),
    ("ability", "def"),
    ("ability", "id"),
    ("ability", "tags"),
];

/// The cause of what telling an owner does.
const REACTION: Cause = Cause::new(CauseKind::Process);

// ---- At load -------------------------------------------------------------------

/// Whether a property of a registry may be read reactively: it names
/// the events that change it, or it is constant, or it is a subject's
/// `kind`.
fn watchable_property(registry: &Registry, id: PropertyId) -> Result<(), String> {
    let Some(property) = registry.property(id) else {
        return Err("a property that is not registered".to_owned());
    };
    let spec = &property.spec;
    let kind = registry
        .kind(spec.kind)
        .map_or("?", |kind| kind.spec.name.as_str());
    let constant = spec.name == "kind"
        || CONSTANT_PROPERTIES
            .iter()
            .any(|(of, name)| *of == kind && *name == spec.name);
    if constant || !spec.changed_by.is_empty() {
        return Ok(());
    }
    Err(format!(
        "`{kind}.{}`, which no event is registered as changing",
        spec.name
    ))
}

/// The names called as functions in a syntax tree, in the order
/// written.
fn calls(expr: &clash_expr::Expr, out: &mut Vec<clash_expr::Name>) {
    match &expr.kind {
        ExprKind::Null
        | ExprKind::Bool(_)
        | ExprKind::Number { .. }
        | ExprKind::Str(_)
        | ExprKind::Name(_)
        | ExprKind::Local(_) => {}
        ExprKind::List(items) => items.iter().for_each(|item| calls(item, out)),
        ExprKind::Map(entries) => {
            for (key, value) in entries {
                calls(key, out);
                calls(value, out);
            }
        }
        ExprKind::Member { object, .. } => calls(object, out),
        ExprKind::Call { callee, args } => {
            if let ExprKind::Name(name) = &callee.kind {
                out.push(name.clone());
            } else {
                calls(callee, out);
            }
            args.iter().for_each(|arg| calls(arg, out));
        }
        ExprKind::Index { object, index } => {
            calls(object, out);
            calls(index, out);
        }
        ExprKind::Unary { operand, .. } => calls(operand, out),
        ExprKind::Binary { lhs, rhs, .. } => {
            calls(lhs, out);
            calls(rhs, out);
        }
        ExprKind::If {
            cond,
            then,
            otherwise,
        } => {
            calls(cond, out);
            calls(then, out);
            calls(otherwise, out);
        }
    }
}

/// What a reactive source reads that cannot be watched, as sentences:
/// properties with no events, functions with none, functions of mods.
pub fn unwatchable(
    registry: &Registry,
    source: &str,
    compiled: &clash_expr::Compiled,
) -> Vec<String> {
    let mut out = Vec::new();
    for id in compiled.properties_read() {
        if let Err(what) = watchable_property(registry, id) {
            out.push(what);
        }
    }
    let Ok(tree) = clash_expr::parse_expr(source) else {
        return out;
    };
    let mut names = Vec::new();
    calls(&tree, &mut names);
    for name in names {
        let written = match &name.namespace {
            Some(namespace) => format!("{namespace}:{}", name.name),
            None => name.name.clone(),
        };
        let function = (name.namespace.is_none())
            .then(|| registry.function_id(&name.name))
            .flatten()
            .and_then(|id| registry.function(id));
        match function {
            Some(function) if function.is_builtin() && !function.spec.random => {}
            Some(function) if function.spec.changed_by.is_some() => {}
            Some(_) => out.push(format!(
                "the function `{written}`, which no event is registered as changing"
            )),
            None => out.push(format!(
                "the function `{written}` of a mod, whose body may read anything"
            )),
        }
    }
    out
}

/// Checks a condition that is to be kept current (05 §7.5), reporting
/// at the field what it reads that cannot be watched. A constant
/// condition is always fine.
pub(crate) fn check_reactive_cond(condition: &Cond, cx: &mut CompileCx<'_>) {
    if condition.constant_value().is_some() {
        return;
    }
    let (Some(source), Some(compiled)) = (condition.source(), condition.compiled()) else {
        report_at(
            cx,
            condition.site(),
            "a condition kept current must be one expression: write `all`, `any` and `not` as `and`, `or` and `not`, and a query as a property",
        );
        return;
    };
    for what in unwatchable(cx.registry(), source, compiled) {
        report_at(
            cx,
            condition.site(),
            format!("a condition kept current (05 §7.5) cannot read {what}"),
        );
    }
}

/// Refuses parameters of an ability that read each other in a circle:
/// `ability.a` read by `b` and `ability.b` by `a`.
pub(crate) fn check_param_cycles(def: &AbilityDef, cx: &mut CompileCx<'_>) {
    let registry = cx.registry();
    let Some(kind) = registry.kind_id(super::ABILITY_KIND) else {
        return;
    };
    let names: Vec<&String> = def.params.keys().collect();
    // For each parameter, those of the same ability it reads directly.
    let mut reads: Vec<Vec<usize>> = Vec::with_capacity(names.len());
    for expr in def.params.values() {
        let mut direct = Vec::new();
        if let Some(code) = expr.compiled() {
            for id in code.properties_read() {
                let Some(property) = registry.property(id) else {
                    continue;
                };
                if property.spec.kind != kind {
                    continue;
                }
                if let Some(at) = names.iter().position(|name| **name == property.spec.name) {
                    direct.push(at);
                }
            }
        }
        reads.push(direct);
    }
    // A depth-first walk: a parameter met again on its own path reads
    // itself.
    fn walk(at: usize, reads: &[Vec<usize>], path: &mut Vec<usize>, done: &mut [bool]) -> bool {
        if path.contains(&at) {
            return true;
        }
        if done[at] {
            return false;
        }
        path.push(at);
        for next in &reads[at] {
            if walk(*next, reads, path, done) {
                return true;
            }
        }
        path.pop();
        done[at] = true;
        false
    }
    let mut done = vec![false; names.len()];
    for (start, name) in names.iter().enumerate() {
        let mut path = Vec::new();
        if walk(start, &reads, &mut path, &mut done) {
            let site = def.params[*name].site().clone();
            report_at(
                cx,
                &site,
                format!("the parameter `{name}` reads itself through the parameters it reads"),
            );
            return;
        }
    }
}

// ---- While running ---------------------------------------------------------------

/// What owns a watched expression, and is told when its value changes.
/// Watchers are evaluated in this order.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub enum Watcher {
    /// The `while` of an ability instance, by its owner and number.
    Ability {
        /// The owner.
        entity: EntityId,
        /// The instance's number.
        number: u64,
    },
    /// The `while` of a trait instance, by its holder and key.
    Trait {
        /// The holder.
        entity: EntityId,
        /// The instance's key.
        key: TraitKey,
    },
}

impl Watcher {
    /// Whose it is.
    pub fn entity(self) -> EntityId {
        match self {
            Watcher::Ability { entity, .. } | Watcher::Trait { entity, .. } => entity,
        }
    }
}

/// One thing a watched expression read.
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Debug)]
enum Read {
    /// An event that changes what was read, for a subject.
    Event(String, Subject),
    /// A variable of a subject.
    Var(Subject, String),
}

/// What is kept of one watched expression.
#[derive(Clone, Debug)]
struct Entry {
    /// Its value when last evaluated.
    value: bool,
    /// What it read then.
    reads: Vec<Read>,
}

/// The watched expressions of a simulation and what each subscribes
/// to. Not state: see the [module documentation](self).
#[derive(Clone, Debug, Default)]
pub struct Reactive {
    built: bool,
    entries: BTreeMap<Watcher, Entry>,
    by_event: BTreeMap<(String, Subject), BTreeSet<Watcher>>,
    by_var: BTreeMap<(Subject, String), BTreeSet<Watcher>>,
    stale: BTreeSet<Watcher>,
    /// How many rounds the current pump has run.
    rounds: i64,
    /// How many evaluations have been made since the simulation was
    /// made or loaded: for tests and the budget.
    evaluations: u64,
}

impl Reactive {
    /// How many expressions are watched.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Whether nothing is watched.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Whether a watcher is watched.
    pub fn watches(&self, watcher: Watcher) -> bool {
        self.entries.contains_key(&watcher)
    }

    /// The value a watcher's expression had when last evaluated.
    pub fn value(&self, watcher: Watcher) -> Option<bool> {
        self.entries.get(&watcher).map(|entry| entry.value)
    }

    /// How many evaluations have been made since the simulation was
    /// made or loaded.
    pub fn evaluations(&self) -> u64 {
        self.evaluations
    }

    /// What is stale now.
    pub fn stale(&self) -> impl Iterator<Item = &Watcher> {
        self.stale.iter()
    }

    fn unsubscribe(&mut self, watcher: Watcher) {
        let Some(entry) = self.entries.remove(&watcher) else {
            return;
        };
        for read in entry.reads {
            match read {
                Read::Event(event, subject) => {
                    let key = (event, subject);
                    if let Some(set) = self.by_event.get_mut(&key) {
                        set.remove(&watcher);
                        if set.is_empty() {
                            self.by_event.remove(&key);
                        }
                    }
                }
                Read::Var(subject, name) => {
                    let key = (subject, name);
                    if let Some(set) = self.by_var.get_mut(&key) {
                        set.remove(&watcher);
                        if set.is_empty() {
                            self.by_var.remove(&key);
                        }
                    }
                }
            }
        }
    }

    fn subscribe(&mut self, watcher: Watcher, value: bool, reads: Vec<Read>) {
        self.unsubscribe(watcher);
        for read in &reads {
            match read {
                Read::Event(event, subject) => {
                    self.by_event
                        .entry((event.clone(), *subject))
                        .or_default()
                        .insert(watcher);
                }
                Read::Var(subject, name) => {
                    self.by_var
                        .entry((*subject, name.clone()))
                        .or_default()
                        .insert(watcher);
                }
            }
        }
        self.entries.insert(watcher, Entry { value, reads });
    }
}

/// The host a watched expression is evaluated against: the
/// simulation's, reading only, noting everything read.
struct Recorder<'a> {
    inner: SimHost<'a>,
    sim: &'a Sim,
    reads: BTreeSet<Read>,
    /// The first thing read that cannot be watched, if any.
    unwatchable: Option<String>,
}

impl Recorder<'_> {
    fn refuse(&mut self, what: String) {
        if self.unwatchable.is_none() {
            self.unwatchable = Some(what);
        }
    }

    fn note_property(&mut self, property: PropertyId, subject: Handle) {
        let registry = self.sim.exprs.registry();
        if let Err(what) = watchable_property(registry, property) {
            self.refuse(what);
            return;
        }
        let Some(spec) = registry.property(property).map(|found| &found.spec) else {
            return;
        };
        if spec.changed_by.is_empty() {
            // Constant for its subject.
            return;
        }
        let Some(key) = subject_key(self.sim, subject) else {
            self.refuse(format!(
                "a property `{}` of a subject that cannot be followed",
                spec.name
            ));
            return;
        };
        for event in &spec.changed_by {
            self.reads.insert(Read::Event(event.clone(), key));
        }
    }
}

/// The subject events about a handle are raised for: the kernel's own
/// subjects as they are; an ability by its owner; an entity's
/// attributes, pools and tracks by the entity.
fn subject_key(sim: &Sim, handle: Handle) -> Option<Subject> {
    if let Some(subject) = sim.exprs.subject(handle) {
        return Some(subject);
    }
    let registry = sim.exprs.registry();
    let name = registry.kind(handle.kind)?.spec.name.as_str();
    match name {
        super::ABILITY_KIND => super::owner_of(sim, handle.payload).map(Subject::Entity),
        crate::stats::props::ATTRS_KIND
        | crate::stats::props::POOLS_KIND
        | crate::stats::props::TRACKS_KIND => {
            Some(Subject::Entity(EntityId::from_bits(handle.payload)))
        }
        _ => None,
    }
}

impl Host for Recorder<'_> {
    fn subject(&mut self, slot: u16) -> Value {
        self.inner.subject(slot)
    }

    fn local(&mut self, slot: u16) -> Value {
        self.inner.local(slot)
    }

    fn event_field(&mut self, slot: u16) -> Value {
        self.inner.event_field(slot)
    }

    fn property(&mut self, property: PropertyId, subject: Handle, args: &[Value]) -> Value {
        self.note_property(property, subject);
        self.inner.property(property, subject, args)
    }

    fn variable(&mut self, subject: Handle, name: &str) -> Value {
        match subject_key(self.sim, subject) {
            Some(key) => {
                self.reads.insert(Read::Var(key, name.to_owned()));
            }
            None => self.refuse(format!(
                "the variable `{name}` of a subject that cannot be followed"
            )),
        }
        self.inner.variable(subject, name)
    }

    fn call(&mut self, function: FunctionId, args: &[Value]) -> Value {
        let registry = self.sim.exprs.registry();
        match registry.function(function) {
            Some(found) => match &found.spec.changed_by {
                Some(events) => {
                    for arg in args {
                        if let Value::Subject(handle) = arg
                            && let Some(key) = subject_key(self.sim, *handle)
                        {
                            for event in events {
                                self.reads.insert(Read::Event(event.clone(), key));
                            }
                        }
                    }
                }
                None => {
                    let name = found.spec.name.clone();
                    self.refuse(format!("the function `{name}`"));
                }
            },
            None => self.refuse("a function that is not registered".to_owned()),
        }
        self.inner.call(function, args)
    }

    fn rng(&mut self) -> Option<&mut clash_core::Rng> {
        None
    }

    fn warn(&mut self, warning: RuntimeWarning) {
        self.inner.warn(warning);
    }

    fn def_name(&self, id: DefRef) -> Option<&str> {
        self.inner.def_name(id)
    }

    fn query(&mut self, call: &QueryCall<'_>, out: &mut Vec<Value>) -> Result<(), Fault> {
        self.refuse("a query".to_owned());
        self.inner.query(call, out)
    }
}

/// What evaluating a watcher's expression gave.
struct Evaluated {
    value: bool,
    reads: Vec<Read>,
    unwatchable: Option<String>,
}

/// Evaluates a condition with the given subjects bound, noting what it
/// read.
fn evaluate(sim: &Sim, condition: &Cond, subjects: Vec<Value>) -> Evaluated {
    let bound = Bound {
        subjects,
        locals: Vec::new(),
        fields: Vec::new(),
    };
    let mut recorder = Recorder {
        inner: sim.host(bound),
        sim,
        reads: BTreeSet::new(),
        unwatchable: None,
    };
    let value = condition.holds(&mut Runner::new().on(&sim.defs, &mut recorder));
    Evaluated {
        value,
        reads: recorder.reads.into_iter().collect(),
        unwatchable: recorder.unwatchable,
    }
}

/// Evaluates what a watcher watches, if it still exists and has a
/// condition.
fn evaluate_watcher(sim: &Sim, watcher: Watcher) -> Option<Evaluated> {
    let defs = Arc::clone(&sim.defs);
    match watcher {
        Watcher::Ability { entity, number } => {
            let inst = sim.components.get::<Abilities>(entity)?.by_number(number)?;
            let def = defs.try_registry::<AbilityDef>()?.try_get(inst.def())?;
            let condition = def.condition.as_ref()?;
            let subjects = vec![
                sim.subject_value(Subject::Entity(entity)),
                super::handle_value(sim, number),
            ];
            Some(evaluate(sim, condition, subjects))
        }
        Watcher::Trait { entity, key } => {
            let inst = sim.components.get::<Traits>(entity)?.get(key)?;
            let def = defs.try_registry::<TraitDef>()?.try_get(inst.def())?;
            let condition = def.condition.as_ref()?;
            let subjects = vec![sim.subject_value(Subject::Entity(entity))];
            Some(evaluate(sim, condition, subjects))
        }
    }
}

/// Starts watching, or watches afresh, and gives the value now. `None`
/// if there is nothing to watch: the owner is gone, it has no
/// condition, or (for a trait) its condition reads what cannot be
/// watched, which the traits package then works out as it always has.
pub(crate) fn watch(sim: &mut Sim, watcher: Watcher) -> Option<bool> {
    let Some(evaluated) = evaluate_watcher(sim, watcher) else {
        sim.ability_rt.reactive.unsubscribe(watcher);
        return None;
    };
    sim.ability_rt.reactive.evaluations += 1;
    if let Some(what) = evaluated.unwatchable {
        sim.ability_rt.reactive.unsubscribe(watcher);
        if let Watcher::Ability { .. } = watcher {
            // Loading refuses this; content reached it some other way.
            sim.fault(
                Reason::new(
                    REACTIVE_UNWATCHABLE,
                    format!("a condition kept current read {what}, so it is worked out only when its ability changes"),
                )
                .with("entity", watcher.entity().to_string()),
            );
            return Some(evaluated.value);
        }
        return None;
    }
    sim.ability_rt
        .reactive
        .subscribe(watcher, evaluated.value, evaluated.reads);
    Some(evaluated.value)
}

/// Stops watching.
pub(crate) fn unwatch(sim: &mut Sim, watcher: Watcher) {
    let reactive = &mut sim.ability_rt.reactive;
    reactive.unsubscribe(watcher);
    reactive.stale.remove(&watcher);
}

/// Stops watching everything of an entity: it has been removed.
pub(crate) fn forget_entity(sim: &mut Sim, entity: EntityId) {
    let gone: Vec<Watcher> = sim
        .ability_rt
        .reactive
        .entries
        .keys()
        .filter(|watcher| watcher.entity() == entity)
        .copied()
        .collect();
    for watcher in gone {
        unwatch(sim, watcher);
    }
}

/// Watches afresh the `while` of every trait instance of an entity that
/// has one: what the kernel calls when the entity's traits changed.
pub(crate) fn rewatch_traits(sim: &mut Sim, entity: EntityId) {
    let old: Vec<Watcher> = sim
        .ability_rt
        .reactive
        .entries
        .keys()
        .filter(|watcher| matches!(watcher, Watcher::Trait { entity: of, .. } if *of == entity))
        .copied()
        .collect();
    for watcher in old {
        unwatch(sim, watcher);
    }
    let defs = Arc::clone(&sim.defs);
    let Some(registry) = defs.try_registry::<TraitDef>() else {
        return;
    };
    let keys: Vec<TraitKey> = sim
        .components
        .get::<Traits>(entity)
        .map(|traits| {
            traits
                .instances()
                .iter()
                .filter(|inst| {
                    registry
                        .try_get(inst.def())
                        .is_some_and(|def| def.condition.is_some())
                })
                .map(crate::traits::TraitInst::key)
                .collect()
        })
        .unwrap_or_default();
    for key in keys {
        watch(sim, Watcher::Trait { entity, key });
    }
}

/// Marks stale whatever subscribed to a delivered event for one of its
/// subjects. The kernel calls this for every event it delivers.
pub(crate) fn event_delivered(sim: &mut Sim, event: &SimEvent) {
    let reactive = &mut sim.ability_rt.reactive;
    if reactive.by_event.is_empty() {
        return;
    }
    let name = event.name();
    for (_, subject) in event.body.subjects() {
        if let Some(watchers) = reactive.by_event.get(&(name.to_owned(), subject)) {
            reactive.stale.extend(watchers.iter().copied());
        }
    }
}

/// Marks stale whatever read a variable that has been written. The
/// kernel calls this from `Sim::set_var`.
pub(crate) fn var_written(sim: &mut Sim, subject: Subject, name: &str) {
    let reactive = &mut sim.ability_rt.reactive;
    if let Some(watchers) = reactive.by_var.get(&(subject, name.to_owned())) {
        reactive.stale.extend(watchers.iter().copied());
    }
}

/// Tells a watcher's owner that its value changed.
fn tell(sim: &mut Sim, watcher: Watcher, value: bool) {
    match watcher {
        Watcher::Ability { entity, number } => {
            super::ops::set_dormant(sim, entity, number, !value, REACTION);
        }
        Watcher::Trait { entity, .. } => {
            crate::traits::reconsider(sim, entity, REACTION);
            super::ops::sync_trait_sources(sim, entity, REACTION);
        }
    }
}

/// Builds the subscriptions from the state: every ability instance and
/// every trait instance with a `while`. An owner whose value differs
/// from what its state says is told, which a consistent state never
/// needs.
pub(crate) fn rebuild(sim: &mut Sim) {
    sim.ability_rt.reactive = Reactive {
        built: true,
        ..Reactive::default()
    };
    let with_abilities: Vec<EntityId> = sim
        .components
        .store::<Abilities>()
        .iter()
        .map(|(entity, _)| entity)
        .collect();
    for entity in with_abilities {
        let numbers: Vec<(u64, State)> = sim
            .components
            .get::<Abilities>(entity)
            .map(|abilities| {
                abilities
                    .instances()
                    .iter()
                    .map(|inst| (inst.number(), inst.state()))
                    .collect()
            })
            .unwrap_or_default();
        for (number, state) in numbers {
            let watcher = Watcher::Ability { entity, number };
            if let Some(value) = watch(sim, watcher)
                && value != (state == State::Active)
            {
                tell(sim, watcher, value);
            }
        }
    }
    let with_traits: Vec<EntityId> = sim
        .components
        .store::<Traits>()
        .iter()
        .map(|(entity, _)| entity)
        .collect();
    for entity in with_traits {
        rewatch_traits(sim, entity);
    }
}

/// One round of re-evaluation at the end of a pump (05 §7.5). Returns
/// whether the pump should go on: something was raised, or more is
/// stale and another round is allowed.
pub(crate) fn settle(sim: &mut Sim) -> bool {
    if !sim.ability_rt.reactive.built {
        rebuild(sim);
    }
    if sim.ability_rt.reactive.stale.is_empty() {
        sim.ability_rt.reactive.rounds = 0;
        return !sim.events.is_empty();
    }
    let limit = sim
        .setting_int(MAX_REACTIVE_ROUNDS_SETTING, DEFAULT_MAX_REACTIVE_ROUNDS)
        .max(1);
    if sim.ability_rt.reactive.rounds >= limit {
        let chain: Vec<String> = sim
            .ability_rt
            .reactive
            .stale
            .iter()
            .map(|watcher| format!("{watcher:?}"))
            .collect();
        sim.ability_rt.reactive.rounds = 0;
        sim.fault(
            Reason::new(
                REACTIVE_ROUNDS,
                format!(
                    "expressions kept current were still stale after {limit} rounds in one pump, and wait for the next: {}",
                    chain.join(" > ")
                ),
            )
            .with("limit", limit)
            .with("stale", chain.join(" > ")),
        );
        return false;
    }
    sim.ability_rt.reactive.rounds += 1;
    let stale = core::mem::take(&mut sim.ability_rt.reactive.stale);
    for watcher in stale {
        let before = sim.ability_rt.reactive.value(watcher);
        let Some(value) = watch(sim, watcher) else {
            continue;
        };
        if before != Some(value) {
            tell(sim, watcher, value);
        }
    }
    !sim.events.is_empty() || !sim.ability_rt.reactive.stale.is_empty()
}
