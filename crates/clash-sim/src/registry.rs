//! What the simulation offers to expressions (05 §5.3): the kinds of
//! subject, the properties, the functions and the events, entered in the
//! expression registry, and the getters, setters and function bodies
//! that stand behind them.
//!
//! The registry lives in the expression crate, which cannot name the
//! simulation's types, so each entry has two halves. The half the
//! compiler reads (names, types, documentation, whether it is safe for
//! belief, whether it can be set) goes into the [`Registry`]. The half
//! that runs (the getter, the setter, the body) stays here, in a
//! [`PropertyDef`] or a [`FunctionDef`]. [`Bindings`] joins the two for
//! one registry: for each id the registry gave out, what answers it.
//!
//! # For whoever loads mods
//!
//! Give the loader [`engine_registry`] with
//! [`Loader::expr_registry`](clash_data::Loader::expr_registry), so that
//! every expression in a mod is checked against the real kinds and
//! properties. [`register_defs`](crate::register_defs) also adds a
//! registry step, which enters whatever of the engine's a registry lacks
//! and the events mods define: a loader that was given no registry, or
//! a test's own, still ends with every entry of the simulation.
//!
//! # For a package that adds properties
//!
//! Write a function that returns the package's [`PropertyDef`]s (and
//! one for its [`FunctionDef`]s, if it has any), and add a line for it
//! in `engine_properties` (or `engine_functions`) in this file. A getter
//! is a plain function given the simulation to read, the subject and
//! the arguments:
//!
//! ```text
//! PropertyDef::new("entity", "vigour", "real?", "How much vigour it has left.", |sim, subject, _| {
//!     let Subject::Entity(id) = subject else { return Value::Null };
//!     sim.components().get::<Stats>(id).map_or(Value::Null, |stats| Value::from(stats.vigour()))
//! })
//! .settable(|sim, subject, value, cause| { /* through the package's own mutation path */ })
//! .changed_by(&["pool_changed"])
//! ```
//!
//! A kind the package serves is added to
//! [`SUBJECT_KIND_DOCS`](crate::subject::SUBJECT_KIND_DOCS) with its
//! handle. Everything a package registers is stated in its document
//! with its arguments and types, and a property that reads truth about
//! anything but the subject's owner is left unsafe for belief, which is
//! what a new `PropertyDef` is.
//!
//! # What is safe for belief
//!
//! Nothing, unless it is marked. What is marked: a subject's `kind`; the
//! coordinates, grid, tract and realm of a cell, and the coordinates and
//! realm of a tract, which are arithmetic on the reference itself; the
//! layer and the cell of an element; and the functions `dist`, `dir`,
//! `offset`, `now`, `setting` and `has_def`. Everything about entities,
//! and everything that reads a cell's contents, is truth: a mind
//! context may read it of its own subject and of nothing else.

use std::sync::Arc;

use clash_data::{Def, DefTypes, Defs, RegistryCx};
use clash_expr::{
    DefRef, DefTypeId, EventSpec, FunctionId, FunctionSpec, Handle, PropertyId, PropertySpec,
    Registry, Type, Value,
};
use clash_world::Cause;
use clash_world::defs::{
    AreaKindDef, AtmosphereDef, BlockDef, DamageTypeDef, FixtureDef, FloorDef, MaterialDef,
    RealmDef,
};

use crate::Sim;
use crate::bus::{EventDef, FieldType, FieldValue};
use crate::event::{CAUSE_SUBJECTS, EventInfo, KERNEL_EVENTS, SimEvent, Subject, WORLD_EVENTS};
use crate::host::Bound;
use crate::reason::Reason;
use crate::subject::Kinds;

/// Reads a property: the simulation, the subject and the property's
/// arguments. Gives null for whatever cannot be answered: a subject
/// that no longer exists, a cell that is not loaded, an argument of the
/// wrong sort.
pub type Getter = fn(&Sim, Subject, &[Value]) -> Value;

/// Sets a property, through the engine's own mutation path (01 R3), or
/// says why not.
pub type Setter = fn(&mut Sim, Subject, &Value, Cause) -> Result<(), Reason>;

/// Computes a function the simulation registered.
pub type Callee = fn(&Sim, &[Value]) -> Value;

/// Computes a function the simulation registered that draws at random,
/// from the stream it is handed: the rule stream, where draws are
/// allowed (05 §4.7).
pub type DrawingCallee = fn(&Sim, &[Value], &mut clash_core::Rng) -> Value;

/// An argument of a property or a function: its name, its type as the
/// language writes types, what it means, and whether it may be left
/// out.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct ArgDef {
    /// The name.
    pub name: &'static str,
    /// The type: `string`, `loc`, `id<area_kind>`.
    pub ty: &'static str,
    /// What it means.
    pub doc: &'static str,
    /// Whether a call may leave it out.
    pub optional: bool,
}

/// A property the simulation offers: what the registry is told about it
/// and what answers it.
#[derive(Clone, Debug)]
pub struct PropertyDef {
    /// The kind of subject that has it.
    pub kind: &'static str,
    /// The name after the dot.
    pub name: &'static str,
    /// Its arguments, if it takes any.
    pub args: Vec<ArgDef>,
    /// The type of its value, as the language writes types. Nullable if
    /// the property can be missing.
    pub ty: &'static str,
    /// What it is.
    pub doc: &'static str,
    /// Whether a character may read it of anything (05 §7.3).
    pub belief_safe: bool,
    /// Whether its value belongs to the same owner as the subject.
    pub yields_own: bool,
    /// The events that can change its value (05 §7.5).
    pub changed_by: &'static [&'static str],
    /// What reads it.
    pub get: Getter,
    /// What sets it, if it can be set.
    pub set: Option<Setter>,
}

impl PropertyDef {
    /// A property of a kind: it takes no arguments, is not safe for
    /// belief and cannot be set until said otherwise.
    pub fn new(
        kind: &'static str,
        name: &'static str,
        ty: &'static str,
        doc: &'static str,
        get: Getter,
    ) -> PropertyDef {
        PropertyDef {
            kind,
            name,
            args: Vec::new(),
            ty,
            doc,
            belief_safe: false,
            yields_own: false,
            changed_by: &[],
            get,
            set: None,
        }
    }

    /// This property with one more required argument.
    #[must_use]
    pub fn arg(mut self, name: &'static str, ty: &'static str, doc: &'static str) -> PropertyDef {
        self.args.push(ArgDef {
            name,
            ty,
            doc,
            optional: false,
        });
        self
    }

    /// This property, safe for belief: a character may read it of
    /// anything.
    #[must_use]
    pub fn safe(mut self) -> PropertyDef {
        self.belief_safe = true;
        self
    }

    /// This property, whose value belongs to the owner of what it was
    /// read from.
    #[must_use]
    pub fn own(mut self) -> PropertyDef {
        self.yields_own = true;
        self
    }

    /// This property, settable through `set`.
    #[must_use]
    pub fn settable(mut self, set: Setter) -> PropertyDef {
        self.set = Some(set);
        self
    }

    /// This property, changed by these events.
    #[must_use]
    pub fn changed_by(mut self, events: &'static [&'static str]) -> PropertyDef {
        self.changed_by = events;
        self
    }

    /// What the registry is told. `None` if one of its types names a
    /// kind or a definition type the registry does not have.
    fn spec(&self, registry: &Registry) -> Option<PropertySpec> {
        let kind = registry.kind_id(self.kind)?;
        let mut spec = PropertySpec::new(kind, self.name, registry.parse_type(self.ty)?)
            .doc(self.doc)
            .belief_safe(self.belief_safe)
            .yields_own(self.yields_own)
            .settable(self.set.is_some())
            .changed_by(self.changed_by.iter().copied());
        for arg in &self.args {
            let ty = registry.parse_type(arg.ty)?;
            spec = if arg.optional {
                spec.optional_param(arg.name, ty, arg.doc)
            } else {
                spec.param(arg.name, ty, arg.doc)
            };
        }
        Some(spec)
    }
}

/// A function the simulation offers.
#[derive(Clone, Debug)]
pub struct FunctionDef {
    /// The name, bare: `dist`.
    pub name: &'static str,
    /// Its parameters, in order.
    pub args: Vec<ArgDef>,
    /// The type of its result.
    pub returns: &'static str,
    /// What it computes.
    pub doc: &'static str,
    /// The heading it is listed under in the reference.
    pub group: &'static str,
    /// Whether a character may call it in a mind context.
    pub belief_safe: bool,
    /// The events that can change its result, if it may be used in a
    /// reactive expression.
    pub changed_by: Option<&'static [&'static str]>,
    /// What computes it.
    pub call: Callee,
    /// What computes it where it may draw at random, for a function that
    /// does. Such a function is refused in a pure context (05 §7.3);
    /// where nothing may be drawn, `call` answers instead.
    pub draws: Option<DrawingCallee>,
}

impl FunctionDef {
    /// A function with its result type. It takes no parameters and is
    /// not safe for belief until said otherwise.
    pub fn new(
        name: &'static str,
        returns: &'static str,
        group: &'static str,
        doc: &'static str,
        call: Callee,
    ) -> FunctionDef {
        FunctionDef {
            name,
            args: Vec::new(),
            returns,
            doc,
            group,
            belief_safe: false,
            changed_by: None,
            call,
            draws: None,
        }
    }

    /// This function, drawing at random through `draws` where a random
    /// stream is at hand. It is then refused in a pure context.
    #[must_use]
    pub fn drawing(mut self, draws: DrawingCallee) -> FunctionDef {
        self.draws = Some(draws);
        self
    }

    /// This function with one more required parameter.
    #[must_use]
    pub fn arg(mut self, name: &'static str, ty: &'static str, doc: &'static str) -> FunctionDef {
        self.args.push(ArgDef {
            name,
            ty,
            doc,
            optional: false,
        });
        self
    }

    /// This function with one more parameter that may be left out.
    #[must_use]
    pub fn optional(
        mut self,
        name: &'static str,
        ty: &'static str,
        doc: &'static str,
    ) -> FunctionDef {
        self.args.push(ArgDef {
            name,
            ty,
            doc,
            optional: true,
        });
        self
    }

    /// This function, safe for belief.
    #[must_use]
    pub fn safe(mut self) -> FunctionDef {
        self.belief_safe = true;
        self
    }

    /// This function, usable in reactive expressions and made stale by
    /// these events.
    #[must_use]
    pub fn changed_by(mut self, events: &'static [&'static str]) -> FunctionDef {
        self.changed_by = Some(events);
        self
    }

    fn spec(&self, registry: &Registry) -> Option<FunctionSpec> {
        let mut spec = FunctionSpec::new(self.name, registry.parse_type(self.returns)?)
            .doc(self.doc)
            .group(self.group)
            .belief_safe(self.belief_safe);
        spec.random = self.draws.is_some();
        if let Some(events) = self.changed_by {
            spec = spec.changed_by(events.iter().copied());
        }
        for arg in &self.args {
            let ty = registry.parse_type(arg.ty)?;
            spec = if arg.optional {
                spec.optional_param(arg.name, ty, arg.doc)
            } else {
                spec.param(arg.name, ty, arg.doc)
            };
        }
        Some(spec)
    }
}

/// Every property the simulation offers. A package that adds properties
/// adds its table here.
pub fn engine_properties() -> Vec<PropertyDef> {
    let mut properties = crate::props::properties();
    properties.extend(crate::item::properties());
    properties.extend(crate::movement::properties());
    properties.extend(crate::perception::props::properties());
    properties.extend(crate::device::properties());
    properties.extend(crate::interaction::properties());
    properties.extend(crate::rule::timer_properties());
    properties.extend(crate::query::properties());
    properties.extend(crate::traits::properties());
    properties.extend(crate::combat::ranged::properties());
    properties
}

/// Every function the simulation offers beyond those the expression
/// language computes itself. A package that adds functions adds its
/// table here.
pub fn engine_functions() -> Vec<FunctionDef> {
    let mut table = crate::props::functions();
    table.extend(crate::perception::props::functions());
    table.extend(crate::combat::functions());
    table
}

/// The definition types the simulation's properties name in their
/// types, each with its documentation.
fn engine_def_types() -> [(&'static str, &'static str); 8] {
    [
        (MaterialDef::TYPE, MaterialDef::DOC),
        (AtmosphereDef::TYPE, AtmosphereDef::DOC),
        (AreaKindDef::TYPE, AreaKindDef::DOC),
        (RealmDef::TYPE, RealmDef::DOC),
        (FloorDef::TYPE, FloorDef::DOC),
        (BlockDef::TYPE, BlockDef::DOC),
        (FixtureDef::TYPE, FixtureDef::DOC),
        (DamageTypeDef::TYPE, DamageTypeDef::DOC),
    ]
}

/// The type of an engine event's field, from how its table writes it.
/// What is not a type of the language (`place`, `decision`) is `any`.
fn field_type(registry: &Registry, written: &str) -> Type {
    registry.parse_type(written).unwrap_or(Type::Any)
}

/// Enters one of the engine's event types, unless an event of its name
/// is there already.
fn mirror_event(registry: &mut Registry, info: &EventInfo) {
    if registry.event_id(info.name).is_some() {
        return;
    }
    let mut spec = EventSpec::new(info.name).doc(info.doc);
    for (name, kind) in info.subjects.iter().chain(CAUSE_SUBJECTS) {
        if let Some(kind) = registry.kind_id(kind) {
            spec = spec.subject(name.trim_end_matches('?'), kind);
        }
    }
    for (name, ty, doc) in info.fields {
        spec = spec.field(*name, field_type(registry, ty), *doc);
    }
    let _ = registry.add_event(spec);
}

/// Enters in a registry whatever of the simulation's it lacks: the
/// kinds of subject, the definition types its properties name, every
/// property and function, and the kernel's and the world's events.
/// What is there already under the same name is left as it is, so this
/// may be called on any registry, any number of times.
pub fn register_engine(registry: &mut Registry) {
    if Kinds::ensure(registry).is_none() {
        return;
    }
    for (name, doc) in engine_def_types() {
        if registry.def_type_id(name).is_none() {
            let _ = registry.add_def_type(name, doc);
        }
    }
    crate::item::register_types(registry);
    crate::movement::register_types(registry);
    crate::world_verbs::register_types(registry);
    crate::device::register_types(registry);
    crate::interaction::register_types(registry);
    crate::traits::register_types(registry);
    crate::reshape::register_types(registry);
    crate::combat::register_types(registry);
    for property in engine_properties() {
        let known = registry
            .kind_id(property.kind)
            .is_some_and(|kind| registry.property_id(kind, property.name).is_some());
        if !known && let Some(spec) = property.spec(registry) {
            let _ = registry.add_property(spec);
        }
    }
    for function in engine_functions() {
        if registry.function_id(function.name).is_none()
            && let Some(spec) = function.spec(registry)
        {
            let _ = registry.add_function(spec);
        }
    }
    crate::verb::register_engine_verbs(registry);
    crate::query::register_queries(registry);
    for info in KERNEL_EVENTS.iter().chain(WORLD_EVENTS) {
        mirror_event(registry, info);
    }
    for info in crate::action::ACT_EVENTS {
        mirror_event(registry, info);
    }
    crate::action::register_hooks(registry);
    crate::item::register_events(registry);
    for info in crate::movement::MOVEMENT_EVENTS {
        mirror_event(registry, info);
    }
    crate::device::register_events(registry);
    crate::interaction::register_events(registry);
    for info in crate::traits::TRAIT_EVENTS {
        mirror_event(registry, info);
    }
    for info in crate::reshape::EDIT_EVENTS {
        mirror_event(registry, info);
    }
}

/// The registry of everything the simulation offers so far: the
/// built-in functions of the expression language, and every kind,
/// property, function and event of the engine (05 §5.3). Whoever loads
/// mods hands it to the loader.
pub fn engine_registry() -> Registry {
    let mut registry = Registry::with_builtins();
    register_engine(&mut registry);
    registry
}

/// The loader's registry step for this crate (04 §7.3): enters whatever
/// of the engine's the registry lacks, and then the events that mods
/// define, so that an `on`-style scope and `event.field` type-check.
fn registry_step(cx: &mut RegistryCx<'_>) {
    register_engine(cx.registry());
    let defs = cx.defs();
    let events: Vec<(String, &EventDef)> = defs
        .iter::<EventDef>()
        .filter_map(|(id, def)| Some((defs.name::<EventDef>(id)?.to_owned(), def)))
        .collect();
    let registry = cx.registry();
    for (name, def) in events {
        if registry.event_id(&name).is_some() {
            continue;
        }
        let mut spec = EventSpec::new(name).doc(def.doc.clone());
        // A subject of a kind the engine does not serve yet is not
        // bound in expressions; the definition is still good.
        let subjects = def.subjects.0.iter().map(|(name, kind)| (&**name, &**kind));
        for (subject, kind) in subjects.chain(CAUSE_SUBJECTS.iter().copied()) {
            if let Some(kind) = registry.kind_id(kind) {
                spec = spec.subject(subject.trim_end_matches('?'), kind);
            }
        }
        for (field, ty) in &def.fields.0 {
            let ty = match FieldType::parse(ty) {
                Some(FieldType::Bool) => Type::Bool,
                Some(FieldType::Int) => Type::Int,
                Some(FieldType::Real) => Type::Real,
                Some(FieldType::Str) => Type::Str,
                Some(FieldType::Any) | None => Type::Any,
            };
            // As declared: a rule that emits the event gives every field
            // (05 §10.5), so a listener need not guard against null.
            spec = spec.field(field.clone(), ty, "");
        }
        if def.hook {
            spec = spec.hook(def.mutable.iter().cloned());
        }
        let _ = registry.add_event(spec);
    }
}

/// Registers this module's registry step with the loader.
pub(crate) fn register_defs(types: &mut DefTypes) {
    types.add_registry_step(registry_step);
}

/// The definition types the simulation's properties give ids of.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct EngineDefTypes {
    /// `material`.
    pub material: Option<DefTypeId>,
    /// `atmosphere`.
    pub atmosphere: Option<DefTypeId>,
    /// `area_kind`.
    pub area_kind: Option<DefTypeId>,
    /// `realm`.
    pub realm: Option<DefTypeId>,
    /// `floor`.
    pub floor: Option<DefTypeId>,
    /// `block`.
    pub block: Option<DefTypeId>,
    /// `fixture`.
    pub fixture: Option<DefTypeId>,
}

impl EngineDefTypes {
    fn of(registry: &Registry) -> EngineDefTypes {
        EngineDefTypes {
            material: registry.def_type_id(MaterialDef::TYPE),
            atmosphere: registry.def_type_id(AtmosphereDef::TYPE),
            area_kind: registry.def_type_id(AreaKindDef::TYPE),
            realm: registry.def_type_id(RealmDef::TYPE),
            floor: registry.def_type_id(FloorDef::TYPE),
            block: registry.def_type_id(BlockDef::TYPE),
            fixture: registry.def_type_id(FixtureDef::TYPE),
        }
    }

    /// A definition of a type as a value: its id, or null if the type
    /// is not in the registry.
    pub fn id(ty: Option<DefTypeId>, index: usize) -> Value {
        match (ty, u32::try_from(index)) {
            (Some(ty), Ok(index)) => Value::Id(DefRef::new(ty, index)),
            _ => Value::Null,
        }
    }
}

/// The simulation's entries of one registry, joined to what answers
/// them: for each property id and function id the registry gave out,
/// the getter, setter or body behind it. Not state: made with the
/// simulation and again after a load.
#[derive(Clone, Debug)]
pub struct Bindings {
    registry: Arc<Registry>,
    kinds: Kinds,
    def_types: EngineDefTypes,
    properties: Vec<PropertyDef>,
    /// For each property id of the registry, the position in
    /// `properties` plus one; zero for a property that is not the
    /// simulation's.
    by_property: Vec<u32>,
    functions: Vec<FunctionDef>,
    /// The same for function ids.
    by_function: Vec<u32>,
}

impl Bindings {
    /// The bindings for a set of loaded definitions: over the registry
    /// their expressions were compiled against. If that registry lacks
    /// anything of the simulation's (the loader was given no registry
    /// and this crate's types were not registered), a copy with the
    /// rest entered is used: ids already given out stay as they are.
    pub fn for_defs(defs: &Defs) -> Bindings {
        Bindings::for_registry(defs.expr_registry())
    }

    /// The bindings over a registry. See [`for_defs`](Bindings::for_defs).
    pub fn for_registry(registry: &Arc<Registry>) -> Bindings {
        let (properties, functions) = (engine_properties(), engine_functions());
        let complete = Kinds::of(registry).is_some_and(|_| {
            properties.iter().all(|property| {
                registry
                    .kind_id(property.kind)
                    .is_some_and(|kind| registry.property_id(kind, property.name).is_some())
            }) && functions
                .iter()
                .all(|function| registry.function_id(function.name).is_some())
        });
        let registry = if complete {
            Arc::clone(registry)
        } else {
            let mut wider = (**registry).clone();
            register_engine(&mut wider);
            Arc::new(wider)
        };
        // After `register_engine` the kinds are there. A registry too
        // full to take them leaves every handle unreadable: the ids
        // below then name no kind of the registry.
        let none = clash_expr::KindId::new(u16::MAX);
        let kinds = Kinds::of(&registry).unwrap_or(Kinds {
            entity: none,
            loc: none,
            element: none,
            area: none,
            site: none,
            tract: none,
            realm: none,
            global: none,
        });
        let mut by_property = Vec::new();
        for (at, property) in properties.iter().enumerate() {
            let id = registry
                .kind_id(property.kind)
                .and_then(|kind| registry.property_id(kind, property.name));
            if let Some(id) = id {
                index(&mut by_property, id.index(), at);
            }
        }
        let mut by_function = Vec::new();
        for (at, function) in functions.iter().enumerate() {
            if let Some(id) = registry.function_id(function.name) {
                index(&mut by_function, id.index(), at);
            }
        }
        Bindings {
            def_types: EngineDefTypes::of(&registry),
            registry,
            kinds,
            properties,
            by_property,
            functions,
            by_function,
        }
    }

    /// The registry these bindings are over.
    pub fn registry(&self) -> &Arc<Registry> {
        &self.registry
    }

    /// The simulation's kinds of subject in that registry.
    pub fn kinds(&self) -> &Kinds {
        &self.kinds
    }

    /// The definition types the properties give ids of.
    pub fn def_types(&self) -> &EngineDefTypes {
        &self.def_types
    }

    /// What answers a property, if it is one of the simulation's.
    pub fn property(&self, id: PropertyId) -> Option<&PropertyDef> {
        let at = *self.by_property.get(id.index())?;
        self.properties.get((at as usize).checked_sub(1)?)
    }

    /// The id of a property of a kind, by name.
    pub fn property_id(&self, kind: &str, name: &str) -> Option<PropertyId> {
        self.registry
            .property_id(self.registry.kind_id(kind)?, name)
    }

    /// What computes a function, if it is one of the simulation's.
    pub fn function(&self, id: FunctionId) -> Option<&FunctionDef> {
        let at = *self.by_function.get(id.index())?;
        self.functions.get((at as usize).checked_sub(1)?)
    }

    /// The handle of a subject as a value, or null where it has none.
    pub fn value(&self, subject: Subject) -> Value {
        self.kinds.value(subject)
    }

    /// What a handle names, if it is one of the simulation's subjects.
    pub fn subject(&self, handle: Handle) -> Option<Subject> {
        self.kinds.subject(handle)
    }

    /// The value of an event's field as an expression reads it.
    pub fn field_value(&self, field: &FieldValue) -> Value {
        match field {
            FieldValue::Null => Value::Null,
            FieldValue::Plain(value) => Value::from(value),
            FieldValue::Subject(subject) => self.value(*subject),
        }
    }

    /// What an event binds for a rule that listens for it: its subjects
    /// and its fields, each in the slot the event's registration gives
    /// it, which is the slot an expression compiled in the event's
    /// scope asks for. A subject the event did not bind, and a field it
    /// does not carry, is null. `None` for an event whose type is not in
    /// the registry.
    pub fn bind_event(&self, event: &SimEvent) -> Option<Bound> {
        let registered = self.registry.event(self.registry.event_id(event.name())?)?;
        let mut bound = Bound::new();
        for (name, _) in &registered.spec.subjects {
            let value = event
                .subject(name)
                .map_or(Value::Null, |subject| self.value(subject));
            bound = bound.subject(value);
        }
        for field in &registered.spec.fields {
            let value = event
                .body
                .field(&field.name)
                .map_or(Value::Null, |value| self.field_value(&value));
            bound = bound.field(value);
        }
        Some(bound)
    }
}

/// Records that the id at `slot` is answered by the entry at `at`.
fn index(table: &mut Vec<u32>, slot: usize, at: usize) {
    if table.len() <= slot {
        table.resize(slot + 1, 0);
    }
    if let (Some(entry), Ok(at)) = (table.get_mut(slot), u32::try_from(at + 1)) {
        *entry = at;
    }
}
