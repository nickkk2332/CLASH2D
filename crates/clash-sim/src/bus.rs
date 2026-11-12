//! Events and hooks: event types, handlers and delivery (05 §8).
//!
//! | | Notification | Hook |
//! |---|---|---|
//! | Says | Something happened | Something is about to happen |
//! | Delivered | At the next pump, in order of emission | At once, before the operation proceeds |
//! | Handlers can | React | Change the declared mutable fields, or cancel |
//!
//! Every event type is registered with its subjects, its fields and
//! whether it is a hook ([`EventType`]). The kernel registers its own
//! and the world's; a mod defines one with an `event` definition
//! ([`EventDef`]); a later package registers its own with
//! [`Sim::register_event`].
//!
//! # Handlers
//!
//! A handler is a function given the simulation and the [`Delivery`] of
//! one event. At this stage handlers are Rust closures; the rule runtime
//! (package K5) registers rules through the same two calls:
//!
//! - [`Sim::on`]: a global handler, run for every event of a type.
//! - [`Sim::on_subject`]: a handler attached to one subject, run only
//!   for events in which that subject takes part under the name the
//!   handler says (`as`, by default the event's first subject).
//!
//! Handlers are not state. A loaded simulation has none, and whoever
//! registered them registers them again.
//!
//! # Delivery order (05 §8.3)
//!
//! 1. Global handlers of the event's type.
//! 2. Handlers attached to each of the event's subjects, in the order
//!    the event declares its subjects.
//!
//! Within each group: higher `priority` first, then lower `order` (the
//! rule's id), then the order of registration. A handler may `consume`
//! the event; later handlers marked `skip_if_consumed` are then skipped.
//!
//! # Filters (05 §8.4)
//!
//! A handler's `filter` is a list of discrete fields and the values they
//! must have. The first entry is the handler's index key: an event looks
//! up only the handlers with no filter and those whose first entry
//! matches one of its own fields, so a thousand handlers filtered on
//! other values cost it nothing.

use std::collections::BTreeMap;
use std::sync::Arc;

use clash_core::{Real, StateHash, StateHasher, Value, define_id};
use clash_data::{Def, DefTypes, Defs, ValidateCx};
use clash_world::Cause;
use clash_world::defs::OrderedMap;
use serde::{Deserialize, Serialize};

use crate::Sim;
use crate::event::{EventBody, KERNEL_EVENTS, SimEvent, Subject, WORLD_EVENTS, Where};
use crate::reason::{Reason, codes};

/// How many hooks may be raised from inside one another (05 §13.2). A
/// hook raised any deeper is skipped, with a fault reported.
pub const MAX_HOOK_DEPTH: u8 = 8;

/// The kinds of subject an event may bind (05 §5.1).
pub const SUBJECT_KINDS: &[&str] = &[
    "entity",
    "loc",
    "element",
    "area",
    "site",
    "tract",
    "region",
    "realm",
    "subgrid",
    "network",
    "job",
    "ability",
    "group",
    "incident",
    "persona",
    "track",
    "memory",
    "case",
    "mission",
    "objective",
    "action",
    "part",
    "wound",
    "body",
    "window",
    "global",
];

/// The value of an event's field.
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum FieldValue {
    /// No value.
    Null,
    /// A plain value: a truth value, a number or a text.
    Plain(Value),
    /// A reference to something.
    Subject(Subject),
}

impl FieldValue {
    /// A text.
    pub fn text(text: impl AsRef<str>) -> FieldValue {
        FieldValue::Plain(Value::from(text.as_ref()))
    }

    /// The plain value, if it is one.
    pub const fn plain(&self) -> Option<&Value> {
        match self {
            FieldValue::Plain(value) => Some(value),
            _ => None,
        }
    }

    /// The number as a real, if this is an integer or a real.
    pub fn as_real(&self) -> Option<Real> {
        match self.plain()? {
            Value::Real(real) => Some(*real),
            Value::Int(int) => Some(Real::from_i64(*int)),
            _ => None,
        }
    }

    /// The name of the value's type: `null`, `bool`, `int`, `real`,
    /// `string`, or the kind of the subject.
    pub const fn type_name(&self) -> &'static str {
        match self {
            FieldValue::Null => "null",
            FieldValue::Plain(value) => value.type_name(),
            FieldValue::Subject(subject) => subject.kind(),
        }
    }

    /// A text that is the same for equal values and different for
    /// different ones: the key handlers are indexed by.
    fn key(&self) -> String {
        match self {
            FieldValue::Null => "n".to_owned(),
            FieldValue::Plain(Value::Bool(value)) => format!("b{value}"),
            FieldValue::Plain(Value::Int(value)) => format!("i{value}"),
            FieldValue::Plain(Value::Real(value)) => {
                format!("r{:016x}", value.get().to_bits())
            }
            FieldValue::Plain(Value::Str(value)) => format!("s{value}"),
            FieldValue::Plain(other) => format!("?{other}"),
            FieldValue::Subject(subject) => format!("@{subject:?}"),
        }
    }
}

impl From<bool> for FieldValue {
    fn from(value: bool) -> FieldValue {
        FieldValue::Plain(Value::Bool(value))
    }
}

impl From<i64> for FieldValue {
    fn from(value: i64) -> FieldValue {
        FieldValue::Plain(Value::Int(value))
    }
}

impl From<Real> for FieldValue {
    fn from(value: Real) -> FieldValue {
        FieldValue::Plain(Value::Real(value))
    }
}

impl From<&str> for FieldValue {
    fn from(value: &str) -> FieldValue {
        FieldValue::text(value)
    }
}

impl From<Subject> for FieldValue {
    fn from(subject: Subject) -> FieldValue {
        FieldValue::Subject(subject)
    }
}

impl StateHash for FieldValue {
    fn state_hash(&self, hasher: &mut StateHasher) {
        match self {
            FieldValue::Null => hasher.write_u8(0),
            FieldValue::Plain(value) => {
                hasher.write_u8(1);
                value.state_hash(hasher);
            }
            FieldValue::Subject(subject) => {
                hasher.write_u8(2);
                subject.state_hash(hasher);
            }
        }
    }
}

/// What an event of a registered type carries: the type's name, its
/// subjects and its fields, each in the order the type declares them.
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub struct EventPayload {
    /// The name of the event type.
    pub name: String,
    /// The subjects, by name.
    pub subjects: Vec<(String, Subject)>,
    /// The fields, by name.
    pub fields: Vec<(String, FieldValue)>,
}

impl EventPayload {
    /// A payload of an event type, with nothing in it yet.
    pub fn new(name: impl Into<String>) -> EventPayload {
        EventPayload {
            name: name.into(),
            subjects: Vec::new(),
            fields: Vec::new(),
        }
    }

    /// This payload with a subject.
    #[must_use]
    pub fn subject(mut self, name: impl Into<String>, subject: Subject) -> EventPayload {
        self.subjects.push((name.into(), subject));
        self
    }

    /// This payload with a field.
    #[must_use]
    pub fn with(mut self, name: impl Into<String>, value: impl Into<FieldValue>) -> EventPayload {
        self.fields.push((name.into(), value.into()));
        self
    }

    /// A field, by name.
    pub fn field(&self, name: &str) -> Option<&FieldValue> {
        self.fields
            .iter()
            .find(|(have, _)| have == name)
            .map(|(_, value)| value)
    }
}

impl StateHash for EventPayload {
    fn state_hash(&self, hasher: &mut StateHasher) {
        hasher.write_str(&self.name);
        hasher.write_len(self.subjects.len());
        for (name, subject) in &self.subjects {
            hasher.write_str(name);
            subject.state_hash(hasher);
        }
        hasher.write_len(self.fields.len());
        for (name, value) in &self.fields {
            hasher.write_str(name);
            value.state_hash(hasher);
        }
    }
}

/// The type of a field of an event that a mod or a later package
/// defines.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum FieldType {
    /// `bool`.
    Bool,
    /// `int`.
    Int,
    /// `real`. An integer is accepted and kept as it is.
    Real,
    /// `string`.
    Str,
    /// `any`: whatever is given.
    Any,
}

impl FieldType {
    /// The type with this name, as data writes it.
    pub fn parse(name: &str) -> Option<FieldType> {
        Some(match name {
            "bool" => FieldType::Bool,
            "int" => FieldType::Int,
            "real" | "number" => FieldType::Real,
            "string" => FieldType::Str,
            "any" => FieldType::Any,
            _ => return None,
        })
    }

    /// The name as data writes it.
    pub const fn name(self) -> &'static str {
        match self {
            FieldType::Bool => "bool",
            FieldType::Int => "int",
            FieldType::Real => "real",
            FieldType::Str => "string",
            FieldType::Any => "any",
        }
    }

    /// Whether a value may be put in a field of this type. Null is
    /// accepted everywhere: a field may be missing.
    fn admits(self, value: &FieldValue) -> bool {
        matches!(
            (self, value),
            (FieldType::Any, _)
                | (_, FieldValue::Null)
                | (FieldType::Bool, FieldValue::Plain(Value::Bool(_)))
                | (FieldType::Int, FieldValue::Plain(Value::Int(_)))
                | (
                    FieldType::Real,
                    FieldValue::Plain(Value::Real(_) | Value::Int(_))
                )
                | (FieldType::Str, FieldValue::Plain(Value::Str(_)))
        )
    }
}

/// A field an event type declares.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct FieldSpec {
    /// The field's name.
    pub name: String,
    /// The type as the reference writes it.
    pub ty: String,
    /// The type a raised event's value is checked against. `None` for a
    /// field of an engine event, whose Rust type already fixes it.
    pub check: Option<FieldType>,
    /// What it holds.
    pub doc: String,
}

/// A registered event type (05 §8.1): what the compiler will check rules
/// against and what the generated reference prints.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct EventType {
    /// The name as rules write it: `element_changed`.
    pub name: String,
    /// The subjects it binds, in delivery order: name and kind. A name
    /// ending in `?` is bound only when there is one.
    pub subjects: Vec<(String, String)>,
    /// Its fields.
    pub fields: Vec<FieldSpec>,
    /// Whether it is a hook: delivered at once, before the operation.
    pub hook: bool,
    /// For a hook: the fields a handler may change.
    pub mutable: Vec<String>,
    /// For a notification, what happened; for a hook, what it precedes
    /// and what a handler may change.
    pub doc: String,
    /// Whether events of this type are raised through [`Sim::raise`]
    /// with a payload, which is then checked against the type. False
    /// for the engine's own events, which are Rust values.
    pub raised: bool,
}

impl EventType {
    /// A notification with this name and nothing in it, to be raised
    /// with [`Sim::raise`].
    pub fn new(name: impl Into<String>) -> EventType {
        EventType {
            name: name.into(),
            subjects: Vec::new(),
            fields: Vec::new(),
            hook: false,
            mutable: Vec::new(),
            doc: String::new(),
            raised: true,
        }
    }

    /// This type with its documentation.
    #[must_use]
    pub fn doc(mut self, doc: impl Into<String>) -> EventType {
        self.doc = doc.into();
        self
    }

    /// This type with one more subject, of a kind of [`SUBJECT_KINDS`].
    #[must_use]
    pub fn subject(mut self, name: impl Into<String>, kind: impl Into<String>) -> EventType {
        self.subjects.push((name.into(), kind.into()));
        self
    }

    /// This type with one more field.
    #[must_use]
    pub fn field(
        mut self,
        name: impl Into<String>,
        ty: FieldType,
        doc: impl Into<String>,
    ) -> EventType {
        self.fields.push(FieldSpec {
            name: name.into(),
            ty: ty.name().to_owned(),
            check: Some(ty),
            doc: doc.into(),
        });
        self
    }

    /// This type as a hook whose handlers may change these fields.
    #[must_use]
    pub fn hook<S: Into<String>>(mut self, mutable: impl IntoIterator<Item = S>) -> EventType {
        self.hook = true;
        self.mutable = mutable.into_iter().map(Into::into).collect();
        self
    }

    /// The type of an engine event, from its table entry.
    fn of_engine(info: &crate::event::EventInfo) -> EventType {
        EventType {
            name: info.name.to_owned(),
            subjects: info
                .subjects
                .iter()
                .map(|(name, kind)| ((*name).to_owned(), (*kind).to_owned()))
                .collect(),
            fields: info
                .fields
                .iter()
                .map(|(name, ty, doc)| FieldSpec {
                    name: (*name).to_owned(),
                    ty: (*ty).to_owned(),
                    check: None,
                    doc: (*doc).to_owned(),
                })
                .collect(),
            hook: false,
            mutable: Vec::new(),
            doc: info.doc.to_owned(),
            raised: false,
        }
    }
}

define_id! {
    /// An `event` definition (05 §8.1).
    pub struct EventDefId;
}

/// The `event` definition type: an event a mod defines and raises with
/// `emit` (05 §8.1).
///
/// ```json
/// { "type": "event", "id": "ritual_completing",
///   "subjects": { "caster": "entity", "here": "loc" },
///   "fields": { "power": "real" },
///   "hook": true, "mutable": ["power"],
///   "doc": "A ritual circle is about to finish charging." }
/// ```
#[derive(Clone, PartialEq, Eq, Debug, Deserialize)]
pub struct EventDef {
    /// The subjects it binds, in delivery order: name and kind.
    #[serde(default)]
    pub subjects: OrderedMap<String>,
    /// Its fields: name and type (`bool`, `int`, `real`, `string`,
    /// `any`).
    #[serde(default)]
    pub fields: OrderedMap<String>,
    /// Whether it is a hook: delivered at once, and cancellable.
    #[serde(default)]
    pub hook: bool,
    /// For a hook: the fields a handler may change.
    #[serde(default)]
    pub mutable: Vec<String>,
    /// What it means, for the reference.
    #[serde(default)]
    pub doc: String,
}

impl Def for EventDef {
    type Id = EventDefId;
    const TYPE: &'static str = "event";
    const DOC: &'static str = "An event that a mod raises with `emit` and that rules listen for: its subjects, its fields, and whether it is a hook (05 §8.1).";

    fn validate(&self, cx: &mut ValidateCx<'_>) {
        for (name, kind) in &self.subjects.0 {
            if !SUBJECT_KINDS.contains(&kind.as_str()) {
                cx.error(format!(
                    "subject `{name}` has the unknown kind `{kind}`; the kinds are: {}",
                    SUBJECT_KINDS.join(", ")
                ));
            }
        }
        for (name, ty) in &self.fields.0 {
            if FieldType::parse(ty).is_none() {
                cx.error(format!(
                    "field `{name}` has the unknown type `{ty}`; a field is `bool`, `int`, `real`, `string` or `any`"
                ));
            }
            if self.subjects.0.iter().any(|(subject, _)| subject == name) {
                cx.error(format!("`{name}` is both a subject and a field"));
            }
        }
        for name in &self.mutable {
            if !self.fields.0.iter().any(|(field, _)| field == name) {
                cx.error(format!(
                    "`{name}` is listed as mutable but is not one of the event's fields"
                ));
            }
        }
        if !self.mutable.is_empty() && !self.hook {
            cx.error("only a hook has mutable fields: add `\"hook\": true`");
        }
    }
}

/// Registers the simulation's definition types with the loader. Call it
/// beside the other crates' `register_defs` before loading mods.
pub fn register_defs(types: &mut DefTypes) {
    types.register::<EventDef>();
}

/// A handler's function: the simulation, and the event being delivered.
pub type HandlerFn = Arc<dyn Fn(&mut Sim, &mut Delivery<'_>) + Send + Sync>;

/// A handler to register: what a rule's `priority`, `filter`, `as` and
/// `skip_if_consumed` become (05 §9.1), and what to run.
#[derive(Clone)]
pub struct Handler {
    /// Order among handlers: higher first. Default 0.
    pub priority: i32,
    /// Order among handlers of one priority: lower first. The rule
    /// runtime passes the rule's id.
    pub order: u32,
    /// Discrete fields and the values they must have (05 §8.4).
    pub filter: Vec<(String, FieldValue)>,
    /// Not run if an earlier handler consumed the event.
    pub skip_if_consumed: bool,
    /// For an attached handler: which of the event's subjects its owner
    /// must be. `None`: the event's first subject.
    pub role: Option<String>,
    /// What to run.
    pub run: HandlerFn,
}

impl Handler {
    /// A handler that runs a function, with default priority, no filter.
    pub fn new(run: impl Fn(&mut Sim, &mut Delivery<'_>) + Send + Sync + 'static) -> Handler {
        Handler {
            priority: 0,
            order: 0,
            filter: Vec::new(),
            skip_if_consumed: false,
            role: None,
            run: Arc::new(run),
        }
    }

    /// This handler with a priority.
    #[must_use]
    pub fn priority(mut self, priority: i32) -> Handler {
        self.priority = priority;
        self
    }

    /// This handler with an order among those of its priority.
    #[must_use]
    pub fn order(mut self, order: u32) -> Handler {
        self.order = order;
        self
    }

    /// This handler with one more field its events must have a value in.
    #[must_use]
    pub fn filter(mut self, field: impl Into<String>, value: impl Into<FieldValue>) -> Handler {
        self.filter.push((field.into(), value.into()));
        self
    }

    /// This handler, skipped once an earlier one consumed the event.
    #[must_use]
    pub fn skip_if_consumed(mut self) -> Handler {
        self.skip_if_consumed = true;
        self
    }

    /// This handler, attached as a subject of this name.
    #[must_use]
    pub fn role(mut self, role: impl Into<String>) -> Handler {
        self.role = Some(role.into());
        self
    }
}

impl core::fmt::Debug for Handler {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Handler")
            .field("priority", &self.priority)
            .field("order", &self.order)
            .field("filter", &self.filter)
            .field("skip_if_consumed", &self.skip_if_consumed)
            .field("role", &self.role)
            .finish_non_exhaustive()
    }
}

define_id! {
    /// A registered handler, for taking it away again.
    pub struct HandlerId(u64);
}

/// A registered handler and where it stands among the others.
#[derive(Clone, Debug)]
struct Listener {
    id: HandlerId,
    handler: Handler,
}

impl Listener {
    /// What listeners are ordered by: priority, highest first; then
    /// order; then registration.
    fn rank(&self) -> (core::cmp::Reverse<i32>, u32, HandlerId) {
        (
            core::cmp::Reverse(self.handler.priority),
            self.handler.order,
            self.id,
        )
    }
}

/// The handlers of one event type under one owner (or none), indexed by
/// the first entry of their filters.
#[derive(Clone, Debug, Default)]
struct Listeners {
    /// Those with no filter.
    open: Vec<Listener>,
    /// Those with a filter, by the field of its first entry and then by
    /// the key of the value that field must have.
    keyed: BTreeMap<String, BTreeMap<String, Vec<Listener>>>,
}

impl Listeners {
    fn add(&mut self, listener: Listener) {
        let list = match listener.handler.filter.first() {
            None => &mut self.open,
            Some((field, value)) => self
                .keyed
                .entry(field.clone())
                .or_default()
                .entry(value.key())
                .or_default(),
        };
        let at = list.partition_point(|have| have.rank() <= listener.rank());
        list.insert(at, listener);
    }

    fn remove(&mut self, id: HandlerId) -> bool {
        let before = self.len();
        self.open.retain(|listener| listener.id != id);
        for by_value in self.keyed.values_mut() {
            for list in by_value.values_mut() {
                list.retain(|listener| listener.id != id);
            }
            by_value.retain(|_, list| !list.is_empty());
        }
        self.keyed.retain(|_, by_value| !by_value.is_empty());
        self.len() != before
    }

    fn len(&self) -> usize {
        self.open.len()
            + self
                .keyed
                .values()
                .flat_map(BTreeMap::values)
                .map(Vec::len)
                .sum::<usize>()
    }

    /// The listeners an event may concern, in delivery order: those with
    /// no filter and those indexed under a value the event has. The rest
    /// of each one's filter is still to be tested.
    fn candidates(&self, event: &SimEvent, looked_at: &mut usize) -> Vec<Listener> {
        let mut found: Vec<Listener> = self.open.clone();
        for (field, by_value) in &self.keyed {
            let Some(value) = event.body.field(field) else {
                continue;
            };
            if let Some(list) = by_value.get(&value.key()) {
                found.extend(list.iter().cloned());
            }
        }
        *looked_at += found.len();
        found.sort_by_key(Listener::rank);
        found
    }
}

/// The event types and who listens for them. Not state: set up when the
/// simulation is, and again after a load.
#[derive(Clone, Debug, Default)]
pub(crate) struct Bus {
    types: BTreeMap<String, EventType>,
    global: BTreeMap<String, Listeners>,
    attached: BTreeMap<(Subject, String), Listeners>,
    next_handler: u64,
    /// How many hooks are being delivered inside one another right now.
    hook_depth: u8,
    /// How many listeners deliveries have looked at: what the filter
    /// index saves is seen in this number.
    looked_at: usize,
}

impl Bus {
    /// A bus that knows the kernel's events, the world's, and the
    /// `event` definitions of the loaded mods.
    pub(crate) fn for_defs(defs: &Defs) -> Bus {
        let mut bus = Bus::default();
        for info in KERNEL_EVENTS.iter().chain(WORLD_EVENTS) {
            bus.types
                .insert(info.name.to_owned(), EventType::of_engine(info));
        }
        for info in crate::movement::MOVEMENT_EVENTS {
            bus.types
                .insert(info.name.to_owned(), EventType::of_engine(info));
        }
        for hook in crate::movement::hook_types() {
            bus.types.insert(hook.name.clone(), hook);
        }
        if let Some(registry) = defs.try_registry::<EventDef>() {
            for (id, def) in registry.iter() {
                let name = registry.name(id).to_owned();
                let mut ty = EventType::new(name.clone()).doc(def.doc.clone());
                for (subject, kind) in &def.subjects.0 {
                    ty = ty.subject(subject.clone(), kind.clone());
                }
                for (field, field_type) in &def.fields.0 {
                    let field_type = FieldType::parse(field_type).unwrap_or(FieldType::Any);
                    ty = ty.field(field.clone(), field_type, "");
                }
                if def.hook {
                    ty = ty.hook(def.mutable.iter().cloned());
                }
                bus.types.insert(name, ty);
            }
        }
        for info in crate::action::ACT_EVENTS {
            bus.types
                .insert(info.name.to_owned(), EventType::of_engine(info));
        }
        for ty in crate::action::hook_types() {
            bus.types.insert(ty.name.clone(), ty);
        }
        // The events of the packages that raise their own.
        for ty in crate::item::event_types() {
            bus.types.entry(ty.name.clone()).or_insert(ty);
        }
        // The events and hooks of living things (06 §13), unless a mod
        // has taken a name.
        for ty in crate::life::events::event_types() {
            bus.types.entry(ty.name.clone()).or_insert(ty);
        }
        for ty in crate::device::event_types() {
            bus.types.entry(ty.name.clone()).or_insert(ty);
        }
        for ty in crate::interaction::event_types() {
            bus.types.entry(ty.name.clone()).or_insert(ty);
        }
        for info in crate::traits::TRAIT_EVENTS {
            bus.types
                .entry(info.name.to_owned())
                .or_insert_with(|| EventType::of_engine(info));
        }
        for hook in crate::traits::hook_types() {
            bus.types.entry(hook.name.clone()).or_insert(hook);
        }
        for info in crate::reshape::EDIT_EVENTS {
            bus.types
                .entry(info.name.to_owned())
                .or_insert_with(|| EventType::of_engine(info));
        }
        for hook in crate::reshape::hook_types() {
            bus.types.entry(hook.name.clone()).or_insert(hook);
        }
        for info in crate::ability::ABILITY_EVENTS {
            bus.types
                .entry(info.name.to_owned())
                .or_insert_with(|| EventType::of_engine(info));
        }
        for hook in crate::ability::hook_types() {
            bus.types.entry(hook.name.clone()).or_insert(hook);
        }
        // The events and hooks of combat (07 §16), unless a mod has
        // taken a name.
        for ty in crate::combat::event_types() {
            bus.types.entry(ty.name.clone()).or_insert(ty);
        }
        // The events of windows (03 §13), unless a mod has taken a name.
        for ty in crate::action::windows::event_types() {
            bus.types.entry(ty.name.clone()).or_insert(ty);
        }
        // The events and hooks of the social layer (15 §15).
        for ty in crate::social::event_types() {
            bus.types.entry(ty.name.clone()).or_insert(ty);
        }
        bus
    }

    /// Whether anything listens for events of a type: a global handler,
    /// or any attached one. Whoever raises a hook that nothing listens
    /// for may skip it.
    pub(crate) fn has_listeners(&self, event: &str) -> bool {
        self.global
            .get(event)
            .is_some_and(|listeners| listeners.len() > 0)
            || self.attached.keys().any(|(_, name)| name == event)
    }
}

/// One event on its way through its handlers: what a handler is given.
#[derive(Debug)]
pub struct Delivery<'a> {
    event: &'a mut SimEvent,
    /// The fields a handler may change. Empty for a notification.
    mutable: &'a [String],
    hook: bool,
    owner: Option<Subject>,
    consumed: bool,
    cancelled: Option<Reason>,
    /// What handlers did that was refused, for the fault report.
    refused: Vec<Reason>,
}

impl Delivery<'_> {
    /// The event being delivered, with whatever earlier handlers changed.
    pub fn event(&self) -> &SimEvent {
        self.event
    }

    /// For an attached handler, the subject it is attached to: `self` in
    /// a rule (05 §9.3). `None` for a global handler.
    pub fn owner(&self) -> Option<Subject> {
        self.owner
    }

    /// Whether this is a hook: something about to happen, which may be
    /// changed or cancelled.
    pub fn is_hook(&self) -> bool {
        self.hook
    }

    /// One of the event's fields.
    pub fn field(&self, name: &str) -> Option<FieldValue> {
        self.event.body.field(name)
    }

    /// Marks the event consumed: later handlers marked
    /// `skip_if_consumed` are skipped (05 §8.3).
    pub fn consume(&mut self) {
        self.consumed = true;
    }

    /// Whether an earlier handler consumed the event.
    pub fn is_consumed(&self) -> bool {
        self.consumed
    }

    /// Changes a field of a hook: the `modify` effect (05 §8.2). The
    /// operation that raised the hook goes on with the changed value.
    ///
    /// Refused for a notification, for a field the hook does not declare
    /// mutable, and for a value of the wrong type.
    pub fn set_field(&mut self, name: &str, value: impl Into<FieldValue>) -> Result<(), Reason> {
        let event = self.event.name().to_owned();
        let fixed = || {
            Reason::new(
                codes::EVENTS_FIELD_FIXED,
                format!("handlers of `{event}` may not change `{name}`"),
            )
            .with("event", event.as_str())
            .with("field", name)
        };
        if !self.hook || !self.mutable.iter().any(|field| field == name) {
            let reason = fixed();
            self.refused.push(reason.clone());
            return Err(reason);
        }
        let EventBody::Custom(payload) = &mut self.event.body else {
            let reason = fixed();
            self.refused.push(reason.clone());
            return Err(reason);
        };
        let value = value.into();
        match payload.fields.iter_mut().find(|(have, _)| have == name) {
            Some((_, slot)) => *slot = value,
            None => payload.fields.push((name.to_owned(), value)),
        }
        Ok(())
    }

    /// Cancels a hook: what it preceded will not happen, and no later
    /// handler runs. The `cancel_event` effect (05 §8.2). The reason is
    /// what whoever raised the hook reports.
    ///
    /// Refused for a notification: what has happened cannot be undone.
    pub fn cancel(&mut self, reason: Reason) -> Result<(), Reason> {
        if !self.hook {
            let refusal = Reason::new(
                codes::EVENTS_FIELD_FIXED,
                format!(
                    "`{}` is a notification: it has happened and cannot be cancelled",
                    self.event.name()
                ),
            )
            .with("event", self.event.name());
            self.refused.push(refusal.clone());
            return Err(refusal);
        }
        self.cancelled = Some(reason);
        Ok(())
    }
}

/// What came of raising an event.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Raised {
    /// The event's subjects and fields. For a hook, as the handlers left
    /// them: the operation uses these.
    pub payload: EventPayload,
    /// For a hook that a handler cancelled, why. The operation must not
    /// go ahead.
    pub cancelled: Option<Reason>,
}

impl Raised {
    /// Whether a handler cancelled the hook.
    pub fn is_cancelled(&self) -> bool {
        self.cancelled.is_some()
    }

    /// A field, as the handlers left it.
    pub fn field(&self, name: &str) -> Option<&FieldValue> {
        self.payload.field(name)
    }
}

impl Sim {
    /// Registers an event type (05 §8.1). For a package that raises
    /// events or hooks of its own; a mod's come from its `event`
    /// definitions. Refused if the name is taken, a subject's kind is
    /// unknown, or a field listed as mutable is not declared.
    pub fn register_event(&mut self, ty: EventType) -> Result<(), Reason> {
        let bad = |why: String| {
            Reason::new(codes::EVENTS_BAD_PAYLOAD, why).with("event", ty.name.as_str())
        };
        if self.bus.types.contains_key(&ty.name) {
            return Err(bad(format!(
                "an event named `{}` is already registered",
                ty.name
            )));
        }
        for (name, kind) in &ty.subjects {
            if !SUBJECT_KINDS.contains(&kind.as_str()) {
                return Err(bad(format!(
                    "subject `{name}` has the unknown kind `{kind}`"
                )));
            }
        }
        for name in &ty.mutable {
            if !ty.fields.iter().any(|field| field.name == *name) {
                return Err(bad(format!("mutable field `{name}` is not declared")));
            }
        }
        self.bus.types.insert(ty.name.clone(), ty);
        Ok(())
    }

    /// A registered event type, by name.
    pub fn event_type(&self, name: &str) -> Option<&EventType> {
        self.bus.types.get(name)
    }

    /// Every registered event type, in order of name, for the generated
    /// reference.
    pub fn event_types(&self) -> impl Iterator<Item = &EventType> {
        self.bus.types.values()
    }

    /// Registers a global handler: run for every event of this type
    /// (05 §9.2). Refused for an event type that is not registered.
    pub fn on(&mut self, event: &str, handler: Handler) -> Result<HandlerId, Reason> {
        self.listen(None, event, handler)
    }

    /// Registers a handler attached to one subject: run only for events
    /// of this type in which the subject takes part under the name the
    /// handler's `role` says, by default the event's first subject
    /// (05 §9.3). Refused for an event type that is not registered.
    pub fn on_subject(
        &mut self,
        owner: Subject,
        event: &str,
        handler: Handler,
    ) -> Result<HandlerId, Reason> {
        self.listen(Some(owner), event, handler)
    }

    fn listen(
        &mut self,
        owner: Option<Subject>,
        event: &str,
        handler: Handler,
    ) -> Result<HandlerId, Reason> {
        if !self.bus.types.contains_key(event) {
            return Err(unknown_event(event));
        }
        let id = HandlerId::new(self.bus.next_handler);
        self.bus.next_handler += 1;
        let listener = Listener { id, handler };
        match owner {
            None => self
                .bus
                .global
                .entry(event.to_owned())
                .or_default()
                .add(listener),
            Some(owner) => self
                .bus
                .attached
                .entry((owner, event.to_owned()))
                .or_default()
                .add(listener),
        }
        Ok(id)
    }

    /// Takes a handler away. Returns whether it was registered.
    pub fn remove_handler(&mut self, id: HandlerId) -> bool {
        let mut removed = false;
        for listeners in self.bus.global.values_mut() {
            removed |= listeners.remove(id);
        }
        for listeners in self.bus.attached.values_mut() {
            removed |= listeners.remove(id);
        }
        self.bus.attached.retain(|_, listeners| listeners.len() > 0);
        removed
    }

    /// How many handlers deliveries have looked at since the simulation
    /// was set up. A measure for tests and the profiler of what the
    /// filter index saves.
    pub fn handlers_looked_at(&self) -> usize {
        self.bus.looked_at
    }

    /// Raises an event of a registered type: the `emit` effect (05 §8.1).
    ///
    /// A notification joins the queue and is delivered at the next pump;
    /// what comes back is the payload as given. A hook is delivered at
    /// once: what comes back holds the fields as the handlers left them
    /// and, if one cancelled it, the reason. The caller then goes ahead
    /// with the changed values, or does not go ahead.
    ///
    /// Refused if the type is not registered, is one of the engine's own
    /// (those are raised by the engine), or the payload does not match
    /// the type: a declared subject missing or of the wrong kind,
    /// something undeclared, a field of the wrong type. A declared field
    /// left out is null.
    pub fn raise(
        &mut self,
        on: Where,
        cause: Cause,
        mut payload: EventPayload,
    ) -> Result<Raised, Reason> {
        let ty = self
            .bus
            .types
            .get(&payload.name)
            .ok_or_else(|| unknown_event(&payload.name))?;
        check_payload(ty, &mut payload)?;
        let (hook, mutable) = (ty.hook, ty.mutable.clone());
        if !hook {
            self.emit(on, cause, EventBody::Custom(payload.clone()));
            return Ok(Raised {
                payload,
                cancelled: None,
            });
        }
        if self.bus.hook_depth >= MAX_HOOK_DEPTH {
            self.fault(
                Reason::new(
                    codes::EVENTS_HOOK_DEPTH,
                    format!(
                        "hook `{}` was raised from inside {MAX_HOOK_DEPTH} others and was skipped",
                        payload.name
                    ),
                )
                .with("event", payload.name.as_str())
                .with("limit", i64::from(MAX_HOOK_DEPTH)),
            );
            return Ok(Raised {
                payload,
                cancelled: None,
            });
        }
        // What the world reported before the hook comes before it.
        self.absorb_world();
        let mut event = self.stamped(on, cause, EventBody::Custom(payload));
        self.bus.hook_depth += 1;
        let cancelled = self.run_handlers(&mut event, true, &mutable);
        self.bus.hook_depth -= 1;
        let tick = event.tick;
        let payload = match event.body {
            EventBody::Custom(payload) => payload,
            // Handlers cannot replace a body, only change its fields.
            other => EventPayload::new(other.name()),
        };
        if let Some(reason) = &cancelled {
            let mut notice = self.stamped(
                on,
                cause,
                EventBody::EventCancelled {
                    event: payload.name.clone(),
                    reason: reason.clone(),
                },
            );
            notice.tick = tick;
            self.events.push_back(notice);
        }
        Ok(Raised { payload, cancelled })
    }

    /// Delivers a notification to its handlers (05 §8.3).
    pub(crate) fn deliver(&mut self, event: &mut SimEvent) {
        self.run_handlers(event, false, &[]);
    }

    /// Runs an event's handlers in delivery order: global ones, then
    /// those attached to each of its subjects in declared order. Returns
    /// the reason a hook was cancelled, if one was.
    fn run_handlers(
        &mut self,
        event: &mut SimEvent,
        hook: bool,
        mutable: &[String],
    ) -> Option<Reason> {
        let name = event.name().to_owned();
        let mut looked_at = 0;
        // Who is to be called is settled before anyone is: a handler
        // that registers another does not change this delivery.
        let mut calls: Vec<(Option<Subject>, Listener)> = Vec::new();
        if let Some(listeners) = self.bus.global.get(&name) {
            calls.extend(
                listeners
                    .candidates(event, &mut looked_at)
                    .into_iter()
                    .map(|listener| (None, listener)),
            );
        }
        if !self.bus.attached.is_empty() {
            let subjects: Vec<(String, Subject)> = event
                .subjects()
                .into_iter()
                .map(|(name, subject)| (name.to_owned(), subject))
                .collect();
            let first = subjects.first().map(|(name, _)| name.clone());
            for (role, subject) in &subjects {
                let Some(listeners) = self.bus.attached.get(&(*subject, name.clone())) else {
                    continue;
                };
                for listener in listeners.candidates(event, &mut looked_at) {
                    let wanted = listener.handler.role.as_ref().or(first.as_ref());
                    if wanted == Some(role) {
                        calls.push((Some(*subject), listener));
                    }
                }
            }
        }
        self.bus.looked_at += looked_at;
        if calls.is_empty() {
            return None;
        }

        let mut delivery = Delivery {
            event,
            mutable,
            hook,
            owner: None,
            consumed: false,
            cancelled: None,
            refused: Vec::new(),
        };
        for (owner, listener) in calls {
            let handler = &listener.handler;
            if handler.skip_if_consumed && delivery.consumed {
                continue;
            }
            // The whole filter, against the event as it now is.
            let passes = handler
                .filter
                .iter()
                .all(|(field, value)| delivery.event.body.field(field).as_ref() == Some(value));
            if !passes {
                continue;
            }
            delivery.owner = owner;
            (handler.run)(self, &mut delivery);
            if delivery.cancelled.is_some() {
                break;
            }
        }
        let Delivery {
            cancelled, refused, ..
        } = delivery;
        for reason in refused {
            self.fault(reason);
        }
        cancelled
    }
}

/// The reason for an event type nobody registered.
fn unknown_event(name: &str) -> Reason {
    Reason::new(
        codes::EVENTS_UNKNOWN_TYPE,
        format!("there is no event type `{name}`"),
    )
    .with("event", name)
}

/// Checks a payload against its type and puts its subjects and fields
/// in declared order, with a null for each declared field left out.
fn check_payload(ty: &EventType, payload: &mut EventPayload) -> Result<(), Reason> {
    let bad = |name: &str, why: String| {
        Reason::new(
            codes::EVENTS_BAD_PAYLOAD,
            format!("event `{}`: {why}", ty.name),
        )
        .with("event", ty.name.as_str())
        .with("name", name)
    };
    if !ty.raised {
        return Err(bad(
            "",
            "it is one of the engine's own events and is raised only by the engine".to_owned(),
        ));
    }
    let mut subjects = Vec::with_capacity(ty.subjects.len());
    for (declared, kind) in &ty.subjects {
        let optional = declared.ends_with('?');
        let name = declared.trim_end_matches('?');
        match payload.subjects.iter().find(|(have, _)| have == name) {
            Some((_, subject)) if subject.kind() == kind => {
                subjects.push((name.to_owned(), *subject));
            }
            Some((_, subject)) => {
                return Err(bad(
                    name,
                    format!(
                        "subject `{name}` is a {kind}, but a {} was given",
                        subject.kind()
                    ),
                ));
            }
            None if optional => {}
            None => return Err(bad(name, format!("subject `{name}` is missing"))),
        }
    }
    if let Some((extra, _)) = payload.subjects.iter().find(|(have, _)| {
        !ty.subjects
            .iter()
            .any(|(declared, _)| declared.trim_end_matches('?') == have)
    }) {
        return Err(bad(extra, format!("it declares no subject `{extra}`")));
    }
    let mut fields = Vec::with_capacity(ty.fields.len());
    for spec in &ty.fields {
        let value = payload
            .field(&spec.name)
            .cloned()
            .unwrap_or(FieldValue::Null);
        if let Some(check) = spec.check
            && !check.admits(&value)
        {
            return Err(bad(
                &spec.name,
                format!(
                    "field `{}` is a {}, but a {} was given",
                    spec.name,
                    check.name(),
                    value.type_name()
                ),
            ));
        }
        fields.push((spec.name.clone(), value));
    }
    if let Some((extra, _)) = payload
        .fields
        .iter()
        .find(|(have, _)| !ty.fields.iter().any(|spec| spec.name == *have))
    {
        return Err(bad(extra, format!("it declares no field `{extra}`")));
    }
    payload.subjects = subjects;
    payload.fields = fields;
    Ok(())
}
