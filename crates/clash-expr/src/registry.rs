//! The registry (05 §5.3): every kind of subject, property, function,
//! event, verb, definition type and declared variable the compiler knows.
//!
//! The registry is plain data. The host fills it in at start-up with the
//! `add_` functions, each of which takes a specification built with a
//! few chained calls and gives back a dense id. The compiler checks
//! expressions against it; the generated reference and the inspector read
//! it back through the iterators, which go in order of name.
//!
//! What a property's getter or a function's body *does* is not here: the
//! host keeps that, keyed by the ids the registry hands out, and answers
//! through the [`Host`](crate::Host) trait when an expression runs.
//!
//! ```
//! use clash_expr::{FunctionSpec, KindSpec, PropertySpec, Registry, Type};
//!
//! let mut registry = Registry::with_builtins();
//! let entity = registry.add_kind(KindSpec::new("entity").doc("Any entity.")).unwrap();
//! let mass = registry
//!     .add_property(
//!         PropertySpec::new(entity, "mass", Type::Real)
//!             .doc("The entity's mass in kilograms.")
//!             .belief_safe(false),
//!     )
//!     .unwrap();
//! registry
//!     .add_function(
//!         FunctionSpec::new("dist", Type::Real)
//!             .param("a", Type::Subject(entity), "One end.")
//!             .param("b", Type::Subject(entity), "The other end.")
//!             .doc("The distance between two things, in metres."),
//!     )
//!     .unwrap();
//!
//! assert_eq!(registry.property_id(entity, "mass"), Some(mass));
//! assert!(registry.function_id("clamp").is_some());
//! assert_eq!(registry.type_name(&Type::Subject(entity)), "entity");
//! ```

use std::collections::BTreeMap;

use clash_core::DenseId;
use thiserror::Error;

use crate::builtins::{self, Builtin};
use crate::lexer::is_keyword;
use crate::types::Type;
use crate::value::{
    DefRef, DefTypeId, EventId, FunctionId, KindId, PropertyId, VariableId, VerbId,
};

/// Why something could not be registered.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[non_exhaustive]
pub enum RegistryError {
    /// The name is already taken by something of the same sort.
    #[error("{what} `{name}` is already registered")]
    Duplicate {
        /// What sort of thing: "kind", "function", …
        what: &'static str,
        /// The name.
        name: String,
    },
    /// The name is not one the language can write.
    #[error("`{name}` is not a usable name for a {what}: {why}")]
    BadName {
        /// What sort of thing.
        what: &'static str,
        /// The name.
        name: String,
        /// What is wrong with it.
        why: &'static str,
    },
    /// The specification names a kind that is not registered.
    #[error("{what} `{name}` refers to a kind that is not registered")]
    UnknownKind {
        /// What sort of thing.
        what: &'static str,
        /// The name of what was being registered.
        name: String,
    },
    /// A parameter that may be left out comes before one that may not.
    #[error("{what} `{name}` has a required parameter after an optional one")]
    ParameterOrder {
        /// What sort of thing.
        what: &'static str,
        /// The name.
        name: String,
    },
    /// There are more of these than their id can count.
    #[error("too many {what}s are registered")]
    TooMany {
        /// What sort of thing.
        what: &'static str,
    },
}

/// A parameter of a property or a function, an argument of a verb, or a
/// field of an event: a name, a type and a line of documentation.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct ParamSpec {
    /// The name.
    pub name: String,
    /// The type a value for it must have.
    pub ty: Type,
    /// Whether it may be left out. Optional parameters come last.
    pub optional: bool,
    /// What it means.
    pub doc: String,
}

impl ParamSpec {
    /// A required parameter.
    pub fn new(name: impl Into<String>, ty: Type, doc: impl Into<String>) -> ParamSpec {
        ParamSpec {
            name: name.into(),
            ty,
            optional: false,
            doc: doc.into(),
        }
    }

    /// A parameter that may be left out.
    pub fn optional(name: impl Into<String>, ty: Type, doc: impl Into<String>) -> ParamSpec {
        ParamSpec {
            optional: true,
            ..ParamSpec::new(name, ty, doc)
        }
    }
}

/// A kind of subject to register.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct KindSpec {
    /// The name rules write: `entity`, `loc`.
    pub name: String,
    /// What it refers to.
    pub doc: String,
}

impl KindSpec {
    /// A kind with this name.
    pub fn new(name: impl Into<String>) -> KindSpec {
        KindSpec {
            name: name.into(),
            doc: String::new(),
        }
    }

    /// Sets the documentation.
    #[must_use]
    pub fn doc(mut self, doc: impl Into<String>) -> KindSpec {
        self.doc = doc.into();
        self
    }
}

/// A property to register (05 §5.3).
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct PropertySpec {
    /// The kind of subject that has it.
    pub kind: KindId,
    /// The name after the dot.
    pub name: String,
    /// The arguments it takes, if any: `here.field('fire')`.
    pub params: Vec<ParamSpec>,
    /// The type of its value. Make it nullable if the property can be
    /// missing.
    pub ty: Type,
    /// What it is.
    pub doc: String,
    /// Whether a character may read it of anything, in a mind context
    /// (05 §7.3). A property that is not safe may still be read of the
    /// character itself.
    pub belief_safe: bool,
    /// Whether its value belongs to the same owner as the subject it was
    /// read from, such as a part of a body: reading further from it in a
    /// mind context is then still reading the character's own state.
    pub yields_own: bool,
    /// Whether the `set` effect may assign it.
    pub settable: bool,
    /// The events that can change its value, which is what reactive
    /// expressions subscribe to (05 §7.5).
    pub changed_by: Vec<String>,
}

impl PropertySpec {
    /// A property of a kind, with its type. It takes no arguments, is not
    /// safe for belief and cannot be set until said otherwise.
    pub fn new(kind: KindId, name: impl Into<String>, ty: Type) -> PropertySpec {
        PropertySpec {
            kind,
            name: name.into(),
            params: Vec::new(),
            ty,
            doc: String::new(),
            belief_safe: false,
            yields_own: false,
            settable: false,
            changed_by: Vec::new(),
        }
    }

    /// Sets the documentation.
    #[must_use]
    pub fn doc(mut self, doc: impl Into<String>) -> PropertySpec {
        self.doc = doc.into();
        self
    }

    /// Adds a required argument.
    #[must_use]
    pub fn param(mut self, name: impl Into<String>, ty: Type, doc: impl Into<String>) -> Self {
        self.params.push(ParamSpec::new(name, ty, doc));
        self
    }

    /// Adds an argument that may be left out.
    #[must_use]
    pub fn optional_param(
        mut self,
        name: impl Into<String>,
        ty: Type,
        doc: impl Into<String>,
    ) -> Self {
        self.params.push(ParamSpec::optional(name, ty, doc));
        self
    }

    /// Says whether the property is safe for belief.
    #[must_use]
    pub fn belief_safe(mut self, safe: bool) -> PropertySpec {
        self.belief_safe = safe;
        self
    }

    /// Says whether the value belongs to the owner of what it was read
    /// from.
    #[must_use]
    pub fn yields_own(mut self, own: bool) -> PropertySpec {
        self.yields_own = own;
        self
    }

    /// Says whether the property can be set.
    #[must_use]
    pub fn settable(mut self, settable: bool) -> PropertySpec {
        self.settable = settable;
        self
    }

    /// Names the events that can change the property's value.
    #[must_use]
    pub fn changed_by<S: Into<String>>(mut self, events: impl IntoIterator<Item = S>) -> Self {
        self.changed_by = events.into_iter().map(Into::into).collect();
        self
    }
}

/// A function to register (05 §4.7, §12).
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct FunctionSpec {
    /// The name: bare for the engine's (`clamp`), qualified for a mod's
    /// (`ashlands:fall_damage`).
    pub name: String,
    /// The parameters, in order.
    pub params: Vec<ParamSpec>,
    /// Whether the last parameter may be given any number of times, one
    /// or more.
    pub variadic: bool,
    /// The type of the result.
    pub returns: Type,
    /// What it computes.
    pub doc: String,
    /// The heading it is listed under in the reference: "Maths", "Space".
    pub group: String,
    /// Whether a character may call it in a mind context (05 §7.3).
    pub belief_safe: bool,
    /// Whether it draws random numbers. A random function is refused in a
    /// pure context.
    pub random: bool,
    /// The events that can change its result for the same arguments, if
    /// it may be used in a reactive expression (05 §7.5). `None` means it
    /// may not.
    pub changed_by: Option<Vec<String>>,
}

impl FunctionSpec {
    /// A function with its result type. It takes no parameters, is pure
    /// and is safe for belief until said otherwise.
    pub fn new(name: impl Into<String>, returns: Type) -> FunctionSpec {
        FunctionSpec {
            name: name.into(),
            params: Vec::new(),
            variadic: false,
            returns,
            doc: String::new(),
            group: String::new(),
            belief_safe: true,
            random: false,
            changed_by: None,
        }
    }

    /// Sets the documentation.
    #[must_use]
    pub fn doc(mut self, doc: impl Into<String>) -> FunctionSpec {
        self.doc = doc.into();
        self
    }

    /// Sets the heading the function is listed under.
    #[must_use]
    pub fn group(mut self, group: impl Into<String>) -> FunctionSpec {
        self.group = group.into();
        self
    }

    /// Adds a required parameter.
    #[must_use]
    pub fn param(mut self, name: impl Into<String>, ty: Type, doc: impl Into<String>) -> Self {
        self.params.push(ParamSpec::new(name, ty, doc));
        self
    }

    /// Adds a parameter that may be left out.
    #[must_use]
    pub fn optional_param(
        mut self,
        name: impl Into<String>,
        ty: Type,
        doc: impl Into<String>,
    ) -> Self {
        self.params.push(ParamSpec::optional(name, ty, doc));
        self
    }

    /// Lets the last parameter be given more than once.
    #[must_use]
    pub fn variadic(mut self) -> FunctionSpec {
        self.variadic = true;
        self
    }

    /// Says whether the function is safe for belief.
    #[must_use]
    pub fn belief_safe(mut self, safe: bool) -> FunctionSpec {
        self.belief_safe = safe;
        self
    }

    /// Marks the function as drawing random numbers.
    #[must_use]
    pub fn random(mut self) -> FunctionSpec {
        self.random = true;
        self
    }

    /// Names the events that can change the function's result, which
    /// allows it in reactive expressions.
    #[must_use]
    pub fn changed_by<S: Into<String>>(mut self, events: impl IntoIterator<Item = S>) -> Self {
        self.changed_by = Some(events.into_iter().map(Into::into).collect());
        self
    }
}

/// An event type to register (05 §8.1).
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct EventSpec {
    /// The name in data: `element_destroyed`.
    pub name: String,
    /// The subjects it binds, in the order handlers are found by
    /// (05 §8.3): name and kind.
    pub subjects: Vec<(String, KindId)>,
    /// Its fields, read as `event.name`.
    pub fields: Vec<ParamSpec>,
    /// Whether it is a hook: delivered at once, before the operation.
    pub hook: bool,
    /// For a hook: the fields a handler may change.
    pub mutable: Vec<String>,
    /// For a notification, what happened; for a hook, what it precedes.
    pub doc: String,
}

impl EventSpec {
    /// A notification with this name and nothing in it.
    pub fn new(name: impl Into<String>) -> EventSpec {
        EventSpec {
            name: name.into(),
            subjects: Vec::new(),
            fields: Vec::new(),
            hook: false,
            mutable: Vec::new(),
            doc: String::new(),
        }
    }

    /// Sets the documentation.
    #[must_use]
    pub fn doc(mut self, doc: impl Into<String>) -> EventSpec {
        self.doc = doc.into();
        self
    }

    /// Adds a subject the event binds.
    #[must_use]
    pub fn subject(mut self, name: impl Into<String>, kind: KindId) -> EventSpec {
        self.subjects.push((name.into(), kind));
        self
    }

    /// Adds a field.
    #[must_use]
    pub fn field(mut self, name: impl Into<String>, ty: Type, doc: impl Into<String>) -> Self {
        self.fields.push(ParamSpec::new(name, ty, doc));
        self
    }

    /// Makes the event a hook whose handlers may change these fields.
    #[must_use]
    pub fn hook<S: Into<String>>(mut self, mutable: impl IntoIterator<Item = S>) -> EventSpec {
        self.hook = true;
        self.mutable = mutable.into_iter().map(Into::into).collect();
        self
    }
}

/// A verb to register (05 §10.1): the name of an effect and its
/// arguments. This crate keeps the description; the rule runtime compiles
/// and runs effects.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct VerbSpec {
    /// The key an effect object is written with: `damage`.
    pub name: String,
    /// Its arguments.
    pub args: Vec<ParamSpec>,
    /// The one argument that may be given directly as the verb's value,
    /// if there is an obvious one: `{ "message": "…" }`.
    pub direct: Option<String>,
    /// What the verb does.
    pub doc: String,
}

impl VerbSpec {
    /// A verb with this name and no arguments.
    pub fn new(name: impl Into<String>) -> VerbSpec {
        VerbSpec {
            name: name.into(),
            args: Vec::new(),
            direct: None,
            doc: String::new(),
        }
    }

    /// Sets the documentation.
    #[must_use]
    pub fn doc(mut self, doc: impl Into<String>) -> VerbSpec {
        self.doc = doc.into();
        self
    }

    /// Adds a required argument.
    #[must_use]
    pub fn arg(mut self, name: impl Into<String>, ty: Type, doc: impl Into<String>) -> VerbSpec {
        self.args.push(ParamSpec::new(name, ty, doc));
        self
    }

    /// Adds an argument that may be left out.
    #[must_use]
    pub fn optional_arg(
        mut self,
        name: impl Into<String>,
        ty: Type,
        doc: impl Into<String>,
    ) -> VerbSpec {
        self.args.push(ParamSpec::optional(name, ty, doc));
        self
    }

    /// Names the argument that may be given directly.
    #[must_use]
    pub fn direct(mut self, arg: impl Into<String>) -> VerbSpec {
        self.direct = Some(arg.into());
        self
    }
}

/// A declared variable (05 §6).
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct VariableSpec {
    /// The qualified name: `core:alarm_raised`.
    pub name: String,
    /// The kind of subject it lives on.
    pub kind: KindId,
    /// The type of its value. A declared variable that was never set
    /// reads as its default, so it is nullable only if declared so.
    pub ty: Type,
    /// What it is for.
    pub doc: String,
    /// Whether a character may read it of anything, in a mind context.
    pub belief_safe: bool,
}

impl VariableSpec {
    /// A variable on a kind of subject.
    pub fn new(name: impl Into<String>, kind: KindId, ty: Type) -> VariableSpec {
        VariableSpec {
            name: name.into(),
            kind,
            ty,
            doc: String::new(),
            belief_safe: false,
        }
    }

    /// Sets the documentation.
    #[must_use]
    pub fn doc(mut self, doc: impl Into<String>) -> VariableSpec {
        self.doc = doc.into();
        self
    }

    /// Says whether the variable is safe for belief.
    #[must_use]
    pub fn belief_safe(mut self, safe: bool) -> VariableSpec {
        self.belief_safe = safe;
        self
    }
}

/// A registered kind of subject.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Kind {
    /// Its id.
    pub id: KindId,
    /// What was registered.
    pub spec: KindSpec,
}

/// A registered property.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Property {
    /// Its id.
    pub id: PropertyId,
    /// What was registered.
    pub spec: PropertySpec,
}

/// A registered function.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Function {
    /// Its id.
    pub id: FunctionId,
    /// What was registered.
    pub spec: FunctionSpec,
    /// The built-in that computes it, if this crate does; otherwise the
    /// host is asked.
    pub(crate) builtin: Option<Builtin>,
}

impl Function {
    /// Whether this crate computes the function itself. If not, the
    /// machine calls [`Host::call`](crate::Host::call) for it.
    pub const fn is_builtin(&self) -> bool {
        self.builtin.is_some()
    }

    /// The fewest arguments a call may give.
    pub fn min_args(&self) -> usize {
        self.spec.params.iter().filter(|p| !p.optional).count()
    }

    /// The most arguments a call may give, or `None` if there is no
    /// limit.
    pub fn max_args(&self) -> Option<usize> {
        (!self.spec.variadic).then_some(self.spec.params.len())
    }
}

/// A registered event type.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Event {
    /// Its id.
    pub id: EventId,
    /// What was registered.
    pub spec: EventSpec,
}

/// A registered verb.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Verb {
    /// Its id.
    pub id: VerbId,
    /// What was registered.
    pub spec: VerbSpec,
}

/// A declared variable.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Variable {
    /// Its id.
    pub id: VariableId,
    /// What was declared.
    pub spec: VariableSpec,
}

/// A registered type of definition and the ids of that type.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct DefType {
    /// Its id.
    pub id: DefTypeId,
    /// The name of the type: `material`.
    pub name: String,
    /// What definitions of the type are.
    pub doc: String,
    /// The known ids, in order of qualified id, each with the host's
    /// number for it.
    ids: Vec<(String, u32)>,
    /// The same ids by number.
    by_index: BTreeMap<u32, usize>,
}

impl DefType {
    /// The qualified ids of this type with their numbers, in order of
    /// id.
    pub fn ids(&self) -> impl Iterator<Item = (&str, u32)> {
        self.ids.iter().map(|(name, index)| (name.as_str(), *index))
    }

    fn find(&self, qualified: &str) -> Option<u32> {
        self.ids
            .binary_search_by(|(name, _)| name.as_str().cmp(qualified))
            .ok()
            .map(|at| self.ids[at].1)
    }
}

/// The namespace a name is written in and the namespaces it imports
/// (04 §4.3): what a bare name is looked up in.
#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct Scope {
    /// The mod's own namespace, if the expression belongs to a mod.
    pub namespace: Option<String>,
    /// The namespaces the mod imports, in order.
    pub imports: Vec<String>,
}

impl Scope {
    /// A scope with a namespace of its own and no imports.
    pub fn of(namespace: impl Into<String>) -> Scope {
        Scope {
            namespace: Some(namespace.into()),
            imports: Vec::new(),
        }
    }

    /// The namespaces to try for a bare name, in order.
    pub fn namespaces(&self) -> impl Iterator<Item = &str> {
        self.namespace
            .iter()
            .chain(self.imports.iter())
            .map(String::as_str)
    }
}

/// What a name resolved to.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Resolved<T> {
    /// What was found.
    pub found: T,
    /// The qualified name it was found under.
    pub name: String,
    /// For a bare name: a later import that also has it, which makes the
    /// reference worth qualifying (04 §4.3).
    pub also_in: Option<String>,
}

/// Everything the compiler knows. See the [module](self) documentation.
#[derive(Clone, Debug, Default)]
pub struct Registry {
    kinds: Vec<Kind>,
    kind_names: BTreeMap<String, KindId>,
    properties: Vec<Property>,
    property_names: BTreeMap<(KindId, String), PropertyId>,
    functions: Vec<Function>,
    function_names: BTreeMap<String, FunctionId>,
    events: Vec<Event>,
    event_names: BTreeMap<String, EventId>,
    verbs: Vec<Verb>,
    verb_names: BTreeMap<String, VerbId>,
    variables: Vec<Variable>,
    variable_names: BTreeMap<String, VariableId>,
    def_types: Vec<DefType>,
    def_type_names: BTreeMap<String, DefTypeId>,
}

/// Whether `name` is `ident` or, where allowed, `ident:ident`.
fn check_name(what: &'static str, name: &str, qualified_ok: bool) -> Result<(), RegistryError> {
    let bad = |why| RegistryError::BadName {
        what,
        name: name.to_owned(),
        why,
    };
    let is_ident = |part: &str| {
        let mut bytes = part.bytes();
        bytes
            .next()
            .is_some_and(|first| first.is_ascii_alphabetic() || first == b'_')
            && bytes.all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
    };
    let mut parts = name.split(':');
    let first = parts.next().unwrap_or("");
    let second = parts.next();
    if parts.next().is_some() || (second.is_some() && !qualified_ok) {
        return Err(bad("it has too many colons"));
    }
    for part in [Some(first), second].into_iter().flatten() {
        if !is_ident(part) {
            return Err(bad(
                "a name is letters, digits and `_`, not starting with a digit",
            ));
        }
    }
    if second.is_none() && is_keyword(first) {
        return Err(bad("it is a reserved word"));
    }
    Ok(())
}

fn next_id<I: DenseId>(len: usize, what: &'static str) -> Result<I, RegistryError> {
    I::from_index(len).ok_or(RegistryError::TooMany { what })
}

fn check_param_order(
    what: &'static str,
    name: &str,
    params: &[ParamSpec],
) -> Result<(), RegistryError> {
    let first_optional = params.iter().position(|param| param.optional);
    let last_required = params.iter().rposition(|param| !param.optional);
    match (first_optional, last_required) {
        (Some(optional), Some(required)) if optional < required => {
            Err(RegistryError::ParameterOrder {
                what,
                name: name.to_owned(),
            })
        }
        _ => Ok(()),
    }
}

impl Registry {
    /// An empty registry: no kinds, no functions, nothing.
    pub fn new() -> Registry {
        Registry::default()
    }

    /// A registry that holds the built-in functions of 05 §4.7 that
    /// belong to no subsystem: maths, random, lists, maps and text, and
    /// types. The text helpers for a reader (05 §4.6) and `exists` are
    /// registered with their signatures and answered by the host.
    pub fn with_builtins() -> Registry {
        let mut registry = Registry::new();
        builtins::register(&mut registry);
        registry
    }

    // ---- registering -------------------------------------------------

    /// Registers a kind of subject.
    pub fn add_kind(&mut self, spec: KindSpec) -> Result<KindId, RegistryError> {
        check_name("kind", &spec.name, false)?;
        if self.kind_names.contains_key(&spec.name) {
            return Err(RegistryError::Duplicate {
                what: "kind",
                name: spec.name,
            });
        }
        let id: KindId = next_id(self.kinds.len(), "kind")?;
        self.kind_names.insert(spec.name.clone(), id);
        self.kinds.push(Kind { id, spec });
        Ok(id)
    }

    /// Registers a property of a kind of subject.
    ///
    /// `var` cannot be a property: `subject.var.name` reads a variable.
    pub fn add_property(&mut self, spec: PropertySpec) -> Result<PropertyId, RegistryError> {
        check_name("property", &spec.name, true)?;
        if spec.name == "var" {
            return Err(RegistryError::BadName {
                what: "property",
                name: spec.name,
                why: "`var` reads variables",
            });
        }
        if self.kind(spec.kind).is_none() {
            return Err(RegistryError::UnknownKind {
                what: "property",
                name: spec.name,
            });
        }
        check_param_order("property", &spec.name, &spec.params)?;
        let key = (spec.kind, spec.name.clone());
        if self.property_names.contains_key(&key) {
            return Err(RegistryError::Duplicate {
                what: "property",
                name: spec.name,
            });
        }
        let id: PropertyId = next_id(self.properties.len(), "property")?;
        self.property_names.insert(key, id);
        self.properties.push(Property { id, spec });
        Ok(id)
    }

    /// Registers a function the host computes.
    pub fn add_function(&mut self, spec: FunctionSpec) -> Result<FunctionId, RegistryError> {
        self.add_function_impl(spec, None)
    }

    pub(crate) fn add_function_impl(
        &mut self,
        spec: FunctionSpec,
        builtin: Option<Builtin>,
    ) -> Result<FunctionId, RegistryError> {
        check_name("function", &spec.name, true)?;
        check_param_order("function", &spec.name, &spec.params)?;
        if spec.variadic && spec.params.last().is_none_or(|param| param.optional) {
            return Err(RegistryError::ParameterOrder {
                what: "function",
                name: spec.name,
            });
        }
        if self.function_names.contains_key(&spec.name) {
            return Err(RegistryError::Duplicate {
                what: "function",
                name: spec.name,
            });
        }
        let id: FunctionId = next_id(self.functions.len(), "function")?;
        self.function_names.insert(spec.name.clone(), id);
        self.functions.push(Function { id, spec, builtin });
        Ok(id)
    }

    /// Changes whether a registered function is random and whether it
    /// is safe for belief. Both are inferred for a function defined in
    /// data (05 §12), from a body that can only be read once every
    /// function it might call is registered: the data layer registers
    /// them all, works the two out and sets them here before anything is
    /// compiled against them. Does nothing for an id that is not
    /// registered.
    ///
    /// ```
    /// use clash_expr::{Context, FunctionSpec, Registry, Type, compile};
    ///
    /// let mut registry = Registry::with_builtins();
    /// let lucky = registry
    ///     .add_function(FunctionSpec::new("core:lucky", Type::Int))
    ///     .unwrap();
    /// let pure = Context::new().pure();
    /// assert!(compile("core:lucky()", &registry, &pure, &Type::Int).value.is_some());
    /// // Its body turned out to draw at random.
    /// registry.set_function_traits(lucky, true, true);
    /// let outcome = compile("core:lucky()", &registry, &pure, &Type::Int);
    /// assert_eq!(outcome.diagnostics[0].code.as_str(), "X401");
    /// ```
    pub fn set_function_traits(&mut self, id: FunctionId, random: bool, belief_safe: bool) {
        if let Some(function) = self.functions.get_mut(id.index()) {
            function.spec.random = random;
            function.spec.belief_safe = belief_safe;
        }
    }

    /// Registers an event type.
    pub fn add_event(&mut self, spec: EventSpec) -> Result<EventId, RegistryError> {
        check_name("event", &spec.name, true)?;
        if spec
            .subjects
            .iter()
            .any(|(_, kind)| self.kind(*kind).is_none())
        {
            return Err(RegistryError::UnknownKind {
                what: "event",
                name: spec.name,
            });
        }
        if self.event_names.contains_key(&spec.name) {
            return Err(RegistryError::Duplicate {
                what: "event",
                name: spec.name,
            });
        }
        let id: EventId = next_id(self.events.len(), "event")?;
        self.event_names.insert(spec.name.clone(), id);
        self.events.push(Event { id, spec });
        Ok(id)
    }

    /// Registers a verb.
    pub fn add_verb(&mut self, spec: VerbSpec) -> Result<VerbId, RegistryError> {
        check_name("verb", &spec.name, true)?;
        if self.verb_names.contains_key(&spec.name) {
            return Err(RegistryError::Duplicate {
                what: "verb",
                name: spec.name,
            });
        }
        let id: VerbId = next_id(self.verbs.len(), "verb")?;
        self.verb_names.insert(spec.name.clone(), id);
        self.verbs.push(Verb { id, spec });
        Ok(id)
    }

    /// Declares a variable.
    pub fn add_variable(&mut self, spec: VariableSpec) -> Result<VariableId, RegistryError> {
        check_name("variable", &spec.name, true)?;
        if self.kind(spec.kind).is_none() {
            return Err(RegistryError::UnknownKind {
                what: "variable",
                name: spec.name,
            });
        }
        if self.variable_names.contains_key(&spec.name) {
            return Err(RegistryError::Duplicate {
                what: "variable",
                name: spec.name,
            });
        }
        let id: VariableId = next_id(self.variables.len(), "variable")?;
        self.variable_names.insert(spec.name.clone(), id);
        self.variables.push(Variable { id, spec });
        Ok(id)
    }

    /// Registers a type of definition, with no ids yet.
    pub fn add_def_type(
        &mut self,
        name: impl Into<String>,
        doc: impl Into<String>,
    ) -> Result<DefTypeId, RegistryError> {
        let name = name.into();
        check_name("definition type", &name, false)?;
        if self.def_type_names.contains_key(&name) {
            return Err(RegistryError::Duplicate {
                what: "definition type",
                name,
            });
        }
        let id: DefTypeId = next_id(self.def_types.len(), "definition type")?;
        self.def_type_names.insert(name.clone(), id);
        self.def_types.push(DefType {
            id,
            name,
            doc: doc.into(),
            ids: Vec::new(),
            by_index: BTreeMap::new(),
        });
        Ok(id)
    }

    /// Replaces the known ids of a definition type: each qualified id
    /// with the number the host knows it by. Called when a set of mods
    /// has been linked (04 §7.2), before their expressions are compiled.
    /// An id given twice keeps its last number. Does nothing for a type
    /// that is not registered.
    pub fn set_def_ids<S: Into<String>>(
        &mut self,
        ty: DefTypeId,
        ids: impl IntoIterator<Item = (S, u32)>,
    ) {
        let Some(def_type) = self.def_types.get_mut(ty.index()) else {
            return;
        };
        let by_name: BTreeMap<String, u32> = ids
            .into_iter()
            .map(|(name, index)| (name.into(), index))
            .collect();
        def_type.ids = by_name.into_iter().collect();
        def_type.by_index = def_type
            .ids
            .iter()
            .enumerate()
            .map(|(at, (_, index))| (*index, at))
            .collect();
    }

    // ---- looking up ---------------------------------------------------

    /// The kind with this name.
    pub fn kind_id(&self, name: &str) -> Option<KindId> {
        self.kind_names.get(name).copied()
    }

    /// A registered kind.
    pub fn kind(&self, id: KindId) -> Option<&Kind> {
        self.kinds.get(id.index())
    }

    /// The property of a kind with this name.
    pub fn property_id(&self, kind: KindId, name: &str) -> Option<PropertyId> {
        self.property_names.get(&(kind, name.to_owned())).copied()
    }

    /// A registered property.
    pub fn property(&self, id: PropertyId) -> Option<&Property> {
        self.properties.get(id.index())
    }

    /// The function registered under exactly this name.
    pub fn function_id(&self, name: &str) -> Option<FunctionId> {
        self.function_names.get(name).copied()
    }

    /// A registered function.
    pub fn function(&self, id: FunctionId) -> Option<&Function> {
        self.functions.get(id.index())
    }

    /// The event type with this name.
    pub fn event_id(&self, name: &str) -> Option<EventId> {
        self.event_names.get(name).copied()
    }

    /// A registered event type.
    pub fn event(&self, id: EventId) -> Option<&Event> {
        self.events.get(id.index())
    }

    /// The verb with this name.
    pub fn verb_id(&self, name: &str) -> Option<VerbId> {
        self.verb_names.get(name).copied()
    }

    /// A registered verb.
    pub fn verb(&self, id: VerbId) -> Option<&Verb> {
        self.verbs.get(id.index())
    }

    /// The variable declared under exactly this qualified name.
    pub fn variable_id(&self, name: &str) -> Option<VariableId> {
        self.variable_names.get(name).copied()
    }

    /// A declared variable.
    pub fn variable(&self, id: VariableId) -> Option<&Variable> {
        self.variables.get(id.index())
    }

    /// The definition type with this name.
    pub fn def_type_id(&self, name: &str) -> Option<DefTypeId> {
        self.def_type_names.get(name).copied()
    }

    /// A registered definition type.
    pub fn def_type(&self, id: DefTypeId) -> Option<&DefType> {
        self.def_types.get(id.index())
    }

    /// The definition of a type with exactly this qualified id.
    pub fn def_ref(&self, ty: DefTypeId, qualified: &str) -> Option<DefRef> {
        let index = self.def_type(ty)?.find(qualified)?;
        Some(DefRef::new(ty, index))
    }

    /// The qualified id of a definition.
    pub fn def_name(&self, id: DefRef) -> Option<&str> {
        let def_type = self.def_type(id.ty)?;
        let at = *def_type.by_index.get(&id.index)?;
        def_type.ids.get(at).map(|(name, _)| name.as_str())
    }

    // ---- resolving bare names (04 §4.3) --------------------------------

    /// Looks `name` up among `known` the way references resolve: a
    /// qualified name exactly; a bare one in the scope's own namespace,
    /// then in each import in order, then as it stands.
    fn resolve_in<T: Copy>(
        name: &str,
        scope: &Scope,
        known: impl Fn(&str) -> Option<T>,
    ) -> Option<Resolved<T>> {
        if name.contains(':') {
            return known(name).map(|found| Resolved {
                found,
                name: name.to_owned(),
                also_in: None,
            });
        }
        let mut first: Option<Resolved<T>> = None;
        for namespace in scope.namespaces() {
            let qualified = format!("{namespace}:{name}");
            if let Some(found) = known(&qualified) {
                match &mut first {
                    None => {
                        first = Some(Resolved {
                            found,
                            name: qualified,
                            also_in: None,
                        });
                    }
                    Some(resolved) => {
                        if resolved.also_in.is_none() && resolved.name != qualified {
                            resolved.also_in = Some(namespace.to_owned());
                        }
                    }
                }
            }
        }
        first.or_else(|| {
            known(name).map(|found| Resolved {
                found,
                name: name.to_owned(),
                also_in: None,
            })
        })
    }

    /// Resolves an id of a definition type as written in a mod.
    pub fn resolve_def(
        &self,
        ty: DefTypeId,
        name: &str,
        scope: &Scope,
    ) -> Option<Resolved<DefRef>> {
        Registry::resolve_in(name, scope, |qualified| self.def_ref(ty, qualified))
    }

    /// Resolves a function name as written in a mod. A bare name is the
    /// engine's function of that name if there is one, and otherwise
    /// resolves like an id.
    pub fn resolve_function(&self, name: &str, scope: &Scope) -> Option<Resolved<FunctionId>> {
        if !name.contains(':')
            && let Some(found) = self.function_id(name)
        {
            return Some(Resolved {
                found,
                name: name.to_owned(),
                also_in: None,
            });
        }
        Registry::resolve_in(name, scope, |qualified| self.function_id(qualified))
    }

    /// Resolves a variable name as written in a mod.
    pub fn resolve_variable(&self, name: &str, scope: &Scope) -> Option<Resolved<VariableId>> {
        Registry::resolve_in(name, scope, |qualified| self.variable_id(qualified))
    }

    /// The names to offer in place of a misspelt one: every known
    /// qualified name, and, for those the scope can see, the bare name
    /// too.
    pub(crate) fn candidates<'a>(
        names: impl Iterator<Item = &'a str>,
        scope: &Scope,
    ) -> Vec<&'a str> {
        let mut out = Vec::new();
        for name in names {
            out.push(name);
            if let Some((namespace, bare)) = name.split_once(':')
                && scope.namespaces().any(|visible| visible == namespace)
            {
                out.push(bare);
            }
        }
        out
    }

    // ---- reading everything back, in order of name ---------------------

    /// Every kind, in order of name.
    pub fn kinds(&self) -> impl Iterator<Item = &Kind> {
        self.kind_names
            .values()
            .filter_map(|id| self.kinds.get(id.index()))
    }

    /// Every property, in order of kind name and then property name.
    pub fn properties(&self) -> impl Iterator<Item = &Property> {
        self.kinds().flat_map(|kind| self.properties_of(kind.id))
    }

    /// The properties of one kind, in order of name.
    pub fn properties_of(&self, kind: KindId) -> impl Iterator<Item = &Property> {
        self.property_names
            .range((kind, String::new())..)
            .take_while(move |((of, _), _)| *of == kind)
            .filter_map(|(_, id)| self.properties.get(id.index()))
    }

    /// Every function, in order of name.
    pub fn functions(&self) -> impl Iterator<Item = &Function> {
        self.function_names
            .values()
            .filter_map(|id| self.functions.get(id.index()))
    }

    /// Every event type, in order of name.
    pub fn events(&self) -> impl Iterator<Item = &Event> {
        self.event_names
            .values()
            .filter_map(|id| self.events.get(id.index()))
    }

    /// Every verb, in order of name.
    pub fn verbs(&self) -> impl Iterator<Item = &Verb> {
        self.verb_names
            .values()
            .filter_map(|id| self.verbs.get(id.index()))
    }

    /// Every declared variable, in order of qualified name.
    pub fn variables(&self) -> impl Iterator<Item = &Variable> {
        self.variable_names
            .values()
            .filter_map(|id| self.variables.get(id.index()))
    }

    /// Every definition type, in order of name.
    pub fn def_types(&self) -> impl Iterator<Item = &DefType> {
        self.def_type_names
            .values()
            .filter_map(|id| self.def_types.get(id.index()))
    }

    // ---- types as text --------------------------------------------------

    /// A type as the language writes it: `real`, `entity`,
    /// `id<material>`, `list<int>`, `map<real>`, `real?`.
    pub fn type_name(&self, ty: &Type) -> String {
        match ty {
            Type::Any => "any".to_owned(),
            Type::Null => "null".to_owned(),
            Type::Bool => "bool".to_owned(),
            Type::Int => "int".to_owned(),
            Type::Real => "real".to_owned(),
            Type::Number => "number".to_owned(),
            Type::Str => "string".to_owned(),
            Type::Id(id) => match self.def_type(*id) {
                Some(def_type) => format!("id<{}>", def_type.name),
                None => "id<?>".to_owned(),
            },
            Type::Subject(id) => match self.kind(*id) {
                Some(kind) => kind.spec.name.clone(),
                None => "subject<?>".to_owned(),
            },
            Type::List(element) => format!("list<{}>", self.type_name(element)),
            Type::Map(value) => format!("map<{}>", self.type_name(value)),
            Type::Nullable(inner) => format!("{}?", self.type_name(inner)),
        }
    }

    /// Reads a type written as [`type_name`](Registry::type_name) writes
    /// it. A bare `list` or `map` holds `any`. Gives `None` for a name
    /// that is neither a built-in type nor a registered kind, or for an
    /// `id<…>` of a definition type that is not registered.
    pub fn parse_type(&self, text: &str) -> Option<Type> {
        let text = text.trim();
        if let Some(inner) = text.strip_suffix('?') {
            return self.parse_type(inner).map(Type::nullable);
        }
        let generic = |prefix: &str| {
            text.strip_prefix(prefix)
                .and_then(|rest| rest.trim_start().strip_prefix('<'))
                .and_then(|rest| rest.strip_suffix('>'))
        };
        if let Some(inner) = generic("list") {
            return self.parse_type(inner).map(Type::list);
        }
        if let Some(inner) = generic("map") {
            return self.parse_type(inner).map(Type::map);
        }
        if let Some(inner) = generic("id") {
            return self.def_type_id(inner.trim()).map(Type::Id);
        }
        Some(match text {
            "any" => Type::Any,
            "null" => Type::Null,
            "bool" => Type::Bool,
            "int" => Type::Int,
            "real" => Type::Real,
            "number" => Type::Number,
            "string" => Type::Str,
            "list" => Type::list(Type::Any),
            "map" => Type::map(Type::Any),
            kind => Type::Subject(self.kind_id(kind)?),
        })
    }

    /// A function's signature as the reference prints it:
    /// `clamp(x: number, low: number, high: number) -> number`.
    pub fn signature(&self, function: &Function) -> String {
        let mut out = format!("{}(", function.spec.name);
        self.write_params(&mut out, &function.spec.params, function.spec.variadic);
        out.push_str(") -> ");
        out.push_str(&self.type_name(&function.spec.returns));
        out
    }

    /// A property's signature as the reference prints it:
    /// `loc.field(kind: id<field>) -> real`.
    pub fn property_signature(&self, property: &Property) -> String {
        let kind = self
            .kind(property.spec.kind)
            .map_or("?", |kind| kind.spec.name.as_str());
        let mut out = format!("{kind}.{}", property.spec.name);
        if !property.spec.params.is_empty() {
            out.push('(');
            self.write_params(&mut out, &property.spec.params, false);
            out.push(')');
        }
        out.push_str(" -> ");
        out.push_str(&self.type_name(&property.spec.ty));
        out
    }

    fn write_params(&self, out: &mut String, params: &[ParamSpec], variadic: bool) {
        for (index, param) in params.iter().enumerate() {
            if index > 0 {
                out.push_str(", ");
            }
            out.push_str(&param.name);
            if param.optional {
                out.push('?');
            }
            out.push_str(": ");
            out.push_str(&self.type_name(&param.ty));
            if variadic && index + 1 == params.len() {
                out.push_str("...");
            }
        }
    }
}
