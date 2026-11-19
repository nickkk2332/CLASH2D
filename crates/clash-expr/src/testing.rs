//! A small host for tests: a registry with a few kinds, properties,
//! functions, events, verbs, variables and definitions, and state to
//! evaluate against.
//!
//! It names things (`entity`, `glass`, `mass`) so that fixtures read like
//! rules. None of it is engine content: it exists only here, and other
//! crates' tests may use it until they have a simulation to test with.
//!
//! # What is registered
//!
//! | Sort | Entries |
//! |------|---------|
//! | Kinds | `entity`, `loc`, `site`, `part`, `global` |
//! | Definition types | `material` (`core:ash`, `core:glass`, `core:iron`, `ashlands:ash`, `ashlands:slag`), `block` (`core:glass`, `core:wall`) |
//! | `entity` | `mass` real, `health` real (settable), `name` string, `loc` loc, `target` entity?, `tags` list of string, `has_tag(tag)` bool, `material` id of material, `materials` list of those, `stats` map of real, `part(name)` part? (own) |
//! | `part` | `size` real, `wounds` int |
//! | `loc` | `x` int (safe), `site` site (safe), `material` id of material, `field(kind)` real |
//! | `site` | `name` string (safe), `danger` real? |
//! | Functions | the built-ins; `dist(a, b)` real (safe); `secret()` int (not safe); `roll(sides)` int (random); `core:double(x)`; `ashlands:fall_damage(height)`; `ashlands:double(x)` |
//! | Events | `damaged` (subjects `target`, `here`; fields `energy` real, `kind` string, `amount` int, `source` entity?), `damage_incoming` (a hook; `energy` may be changed) |
//! | Verbs | `damage`, `message` |
//! | Variables | `core:kills` int on entity (safe), `core:alarm_raised` bool on site, `core:mood` string? on entity, `ashlands:kills` int on entity, `core:day` int on global |
//!
//! # The context
//!
//! [`TestHost::context`] binds subjects `self` (entity, the character
//! whose mind it is), `target` (entity?), `here` (loc) and `global`;
//! locals `$power` real, `$n` int, `$x` any, `$xs` list of int, `$name`
//! string, `$m` map of real, `$flag` bool and `$maybe` real?; the event
//! `damaged`; and the namespace `core`.

use std::collections::BTreeMap;

use clash_core::{Real, Rng};

use crate::context::Context;
use crate::host::Host;
use crate::registry::{
    EventSpec, FunctionSpec, KindSpec, PropertySpec, Registry, VariableSpec, VerbSpec,
};
use crate::types::Type;
use crate::value::{DefRef, DefTypeId, EventId, FunctionId, Handle, KindId, PropertyId, Value};
use crate::warn::RuntimeWarning;

/// The slots of the subjects [`TestHost::context`] binds.
pub mod slot {
    /// `self`
    pub const SELF: u16 = 0;
    /// `target`
    pub const TARGET: u16 = 1;
    /// `here`
    pub const HERE: u16 = 2;
    /// `global`
    pub const GLOBAL: u16 = 3;

    /// `$power`
    pub const POWER: u16 = 0;
    /// `$n`
    pub const N: u16 = 1;
    /// `$x`
    pub const X: u16 = 2;
    /// `$xs`
    pub const XS: u16 = 3;
    /// `$name`
    pub const NAME: u16 = 4;
    /// `$m`
    pub const M: u16 = 5;
    /// `$flag`
    pub const FLAG: u16 = 6;
    /// `$maybe`
    pub const MAYBE: u16 = 7;

    /// `event.energy`
    pub const ENERGY: u16 = 0;
    /// `event.kind`
    pub const KIND: u16 = 1;
    /// `event.amount`
    pub const AMOUNT: u16 = 2;
    /// `event.source`
    pub const SOURCE: u16 = 3;
}

/// A host for tests. See the [module](self) documentation.
#[derive(Clone, Debug)]
pub struct TestHost {
    pub(crate) registry: Registry,
    /// The kind `entity`.
    pub entity: KindId,
    /// The kind `loc`.
    pub loc: KindId,
    /// The kind `site`.
    pub site: KindId,
    /// The kind `part`.
    pub part: KindId,
    /// The kind `global`.
    pub global: KindId,
    /// The definition type `material`.
    pub material: DefTypeId,
    /// The definition type `block`.
    pub block: DefTypeId,
    /// The event `damaged`.
    pub damaged: EventId,
    /// The values of the bound subjects, by slot.
    pub subjects: Vec<Value>,
    /// The values of the locals, by slot.
    pub locals: Vec<Value>,
    /// The values of the event's fields, by slot.
    pub event: Vec<Value>,
    /// Property values: by subject and by the property as it would be
    /// written, such as `mass` or `field(fire)`.
    pub(crate) properties: BTreeMap<(Handle, String), Value>,
    /// Variable values, by subject and qualified name.
    pub(crate) variables: BTreeMap<(Handle, String), Value>,
    /// The stream random functions draw from, if there is one.
    pub rng: Option<Rng>,
    /// Every run-time warning raised, in order.
    pub warnings: Vec<RuntimeWarning>,
    /// Every call the machine made to a host function or a property, in
    /// order, as text. For checking what was evaluated and in what
    /// order.
    pub log: Vec<String>,
}

/// The numbers the test definitions are known by. They are deliberately
/// not their positions in order of id, to show that nothing assumes so.
const MATERIALS: &[(&str, u32)] = &[
    ("core:ash", 11),
    ("core:glass", 12),
    ("core:iron", 13),
    ("ashlands:ash", 21),
    ("ashlands:slag", 22),
];
const BLOCKS: &[(&str, u32)] = &[("core:glass", 1), ("core:wall", 2)];

impl Default for TestHost {
    fn default() -> TestHost {
        TestHost::new()
    }
}

impl TestHost {
    /// A host with the registry described in the [module](self)
    /// documentation, and state in which `self` is entity 1, `target` is
    /// entity 2, `here` is location 100 and nothing else is set.
    pub fn new() -> TestHost {
        let mut registry = Registry::with_builtins();
        let mut kind = |name: &str, doc: &str| {
            registry
                .add_kind(KindSpec::new(name).doc(doc))
                .expect("the test kinds have distinct names")
        };
        let entity = kind("entity", "Any entity.");
        let loc = kind("loc", "A cell.");
        let site = kind("site", "A site.");
        let part = kind("part", "A part of a body.");
        let global = kind("global", "The whole simulation.");
        let material = registry
            .add_def_type("material", "What things are made of.")
            .expect("the test definition types have distinct names");
        let block = registry
            .add_def_type("block", "What fills a cell.")
            .expect("the test definition types have distinct names");
        registry.set_def_ids(material, MATERIALS.iter().copied());
        registry.set_def_ids(block, BLOCKS.iter().copied());

        let subject = Type::Subject;
        let properties = [
            PropertySpec::new(entity, "mass", Type::Real).doc("Mass in kilograms."),
            PropertySpec::new(entity, "health", Type::Real)
                .doc("Health, for tests of settable properties.")
                .settable(true)
                .changed_by(["damaged"]),
            PropertySpec::new(entity, "name", Type::Str).doc("The true name."),
            PropertySpec::new(entity, "loc", subject(loc)).doc("Where it is."),
            PropertySpec::new(entity, "target", subject(entity).nullable())
                .doc("What it is aiming at, if anything."),
            PropertySpec::new(entity, "tags", Type::list(Type::Str)).doc("Its tags."),
            PropertySpec::new(entity, "has_tag", Type::Bool)
                .param("tag", Type::Str, "The tag to look for.")
                .doc("Whether it has a tag."),
            PropertySpec::new(entity, "material", Type::Id(material)).doc("What it is made of."),
            PropertySpec::new(entity, "materials", Type::list(Type::Id(material)))
                .doc("Everything it is made of."),
            PropertySpec::new(entity, "stats", Type::map(Type::Real)).doc("Numbers by name."),
            PropertySpec::new(entity, "part", subject(part).nullable())
                .param("name", Type::Str, "The part's name.")
                .yields_own(true)
                .doc("A part of its body."),
            PropertySpec::new(part, "size", Type::Real).doc("How large the part is."),
            PropertySpec::new(part, "wounds", Type::Int).doc("How many wounds it has."),
            PropertySpec::new(loc, "x", Type::Int)
                .belief_safe(true)
                .doc("The cell's x coordinate."),
            PropertySpec::new(loc, "site", subject(site))
                .belief_safe(true)
                .doc("The site the cell is in."),
            PropertySpec::new(loc, "material", Type::Id(material))
                .doc("What the cell's block is made of."),
            PropertySpec::new(loc, "field", Type::Real)
                .param("kind", Type::Str, "The field's name.")
                .doc("How much of a field is in the cell."),
            PropertySpec::new(site, "name", Type::Str)
                .belief_safe(true)
                .doc("The site's name."),
            PropertySpec::new(site, "danger", Type::Real.nullable())
                .doc("How dangerous it is, if anyone has said."),
        ];
        for spec in properties {
            registry
                .add_property(spec)
                .expect("the test properties have distinct names");
        }

        let functions = [
            FunctionSpec::new("dist", Type::Real)
                .group("Space")
                .param("a", subject(loc), "One place.")
                .param("b", subject(loc), "Another.")
                .changed_by(["relocated"])
                .doc("The distance between two places."),
            FunctionSpec::new("secret", Type::Int)
                .group("Test")
                .belief_safe(false)
                .doc("Something no character may know."),
            FunctionSpec::new("roll", Type::Int)
                .group("Test")
                .random()
                .param("sides", Type::Int, "How many sides the die has.")
                .doc("A die roll, drawn by the host."),
            FunctionSpec::new("core:double", Type::Number)
                .group("Test")
                .param("x", Type::Number, "A number.")
                .doc("Twice a number."),
            FunctionSpec::new("ashlands:double", Type::Number)
                .group("Test")
                .param("x", Type::Number, "A number.")
                .doc("Twice a number, from another mod."),
            FunctionSpec::new("ashlands:fall_damage", Type::Real)
                .group("Test")
                .param("height", Type::Real, "Metres fallen.")
                .optional_param("mass", Type::Real, "Kilograms falling; 70 if left out.")
                .doc("The energy of a fall."),
        ];
        for spec in functions {
            registry
                .add_function(spec)
                .expect("the test functions have distinct names");
        }

        // A kind of query this host does not answer: one written in a
        // field compiles, and is refused as unavailable when it runs.
        registry
            .add_query(
                crate::QuerySpec::new("entities", subject(entity))
                    .arg("around", Type::Any, "The centre.")
                    .arg("radius", Type::Real, "How far, in metres.")
                    .doc("Entities near a place."),
            )
            .expect("the test queries have distinct names");

        let damaged = registry
            .add_event(
                EventSpec::new("damaged")
                    .subject("target", entity)
                    .subject("here", loc)
                    .field("energy", Type::Real, "Joules delivered.")
                    .field("kind", Type::Str, "The kind of damage.")
                    .field("amount", Type::Int, "How many times.")
                    .field(
                        "source",
                        subject(entity).nullable(),
                        "Who did it, if known.",
                    )
                    .doc("Something was damaged."),
            )
            .expect("the test events have distinct names");
        registry
            .add_event(
                EventSpec::new("damage_incoming")
                    .subject("target", entity)
                    .field("energy", Type::Real, "Joules about to be delivered.")
                    .hook(["energy"])
                    .doc("Precedes damage to an entity. A handler may change the energy."),
            )
            .expect("the test events have distinct names");

        let verbs = [
            VerbSpec::new("damage")
                .arg("at", subject(loc), "Where.")
                .arg("energy", Type::Real, "Joules.")
                .optional_arg("kind", Type::Str, "The kind of damage.")
                .doc("Damages what is in a cell."),
            VerbSpec::new("message")
                .arg("text", Type::Str, "The line to show.")
                .optional_arg("to", subject(entity), "Whose log it goes in.")
                .direct("text")
                .doc("Puts a line in a message log."),
        ];
        for spec in verbs {
            registry
                .add_verb(spec)
                .expect("the test verbs have distinct names");
        }

        let variables = [
            VariableSpec::new("core:kills", entity, Type::Int)
                .belief_safe(true)
                .doc("How many it has killed."),
            VariableSpec::new("core:alarm_raised", site, Type::Bool)
                .doc("Whether the alarm has gone off."),
            VariableSpec::new("core:mood", entity, Type::Str.nullable()).doc("Its mood, if any."),
            VariableSpec::new("ashlands:kills", entity, Type::Int)
                .doc("The same name in another mod."),
            VariableSpec::new("core:day", global, Type::Int).doc("The day."),
        ];
        for spec in variables {
            registry
                .add_variable(spec)
                .expect("the test variables have distinct names");
        }

        let mut host = TestHost {
            registry,
            entity,
            loc,
            site,
            part,
            global,
            material,
            block,
            damaged,
            subjects: Vec::new(),
            locals: vec![Value::Null; 8],
            event: vec![Value::Null; 4],
            properties: BTreeMap::new(),
            variables: BTreeMap::new(),
            rng: Some(Rng::from_seed(1)),
            warnings: Vec::new(),
            log: Vec::new(),
        };
        host.subjects = vec![
            Value::Subject(host.entity(1)),
            Value::Subject(host.entity(2)),
            Value::Subject(host.place(100)),
            Value::Subject(Handle::new(global, 0)),
        ];
        host
    }

    /// The registry.
    pub const fn registry(&self) -> &Registry {
        &self.registry
    }

    /// The registry, to add to.
    pub const fn registry_mut(&mut self) -> &mut Registry {
        &mut self.registry
    }

    /// The context described in the [module](self) documentation: random
    /// functions allowed, truth read.
    pub fn context(&self) -> Context {
        Context::new()
            .own_subject("self", Type::Subject(self.entity))
            .subject("target", Type::Subject(self.entity).nullable())
            .subject("here", Type::Subject(self.loc))
            .subject("global", Type::Subject(self.global))
            .local("power", Type::Real)
            .local("n", Type::Int)
            .local("x", Type::Any)
            .local("xs", Type::list(Type::Int))
            .local("name", Type::Str)
            .local("m", Type::map(Type::Real))
            .local("flag", Type::Bool)
            .local("maybe", Type::Real.nullable())
            .event(self.damaged)
            .namespace("core")
    }

    /// The handle of the entity with this number.
    pub const fn entity(&self, number: u64) -> Handle {
        Handle::new(self.entity, number)
    }

    /// The handle of the location with this number.
    pub const fn place(&self, number: u64) -> Handle {
        Handle::new(self.loc, number)
    }

    /// The handle of the site with this number.
    pub const fn site(&self, number: u64) -> Handle {
        Handle::new(self.site, number)
    }

    /// The handle of the part with this number.
    pub const fn part(&self, number: u64) -> Handle {
        Handle::new(self.part, number)
    }

    /// The id value of a material, by qualified id.
    ///
    /// # Panics
    ///
    /// Panics if there is no such material: a mistake in a test.
    pub fn material(&self, qualified: &str) -> Value {
        Value::Id(
            self.registry
                .def_ref(self.material, qualified)
                .expect("a test names a material that is registered"),
        )
    }

    /// Sets what a property of a subject reads as. `property` is the
    /// property as written, with its arguments if it takes any: `mass`,
    /// `field(fire)`, `part(arm)`.
    pub fn set(&mut self, subject: Handle, property: &str, value: impl Into<Value>) {
        self.properties
            .insert((subject, property.to_owned()), value.into());
    }

    /// Sets a variable of a subject, by qualified name.
    pub fn set_var(&mut self, subject: Handle, name: &str, value: impl Into<Value>) {
        self.variables
            .insert((subject, name.to_owned()), value.into());
    }

    /// Sets a local, by slot.
    pub fn set_local(&mut self, slot: u16, value: impl Into<Value>) {
        if let Some(local) = self.locals.get_mut(usize::from(slot)) {
            *local = value.into();
        }
    }

    /// Sets a field of the event, by slot.
    pub fn set_event(&mut self, slot: u16, value: impl Into<Value>) {
        if let Some(field) = self.event.get_mut(usize::from(slot)) {
            *field = value.into();
        }
    }

    /// Sets a bound subject, by slot.
    pub fn set_subject(&mut self, slot: u16, value: impl Into<Value>) {
        if let Some(subject) = self.subjects.get_mut(usize::from(slot)) {
            *subject = value.into();
        }
    }

    /// A real, for brevity in tests.
    pub const fn real(value: f64) -> Value {
        Value::Real(Real::of(value))
    }

    /// Whether a value is the reader of text: the subject bound as
    /// `self`.
    fn is_reader(&self, value: &Value) -> bool {
        self.subjects.first() == Some(value)
    }

    /// What the reader calls something: "you", its `name` property, or
    /// "something".
    fn name_for_reader(&self, value: &Value) -> String {
        if self.is_reader(value) {
            return "you".to_owned();
        }
        let named = value
            .as_subject()
            .and_then(|handle| self.properties.get(&(handle, "name".to_owned())))
            .and_then(Value::as_str);
        match (named, value) {
            (Some(name), _) => name.to_owned(),
            (None, Value::Str(text)) => text.to_string(),
            _ => "something".to_owned(),
        }
    }

    fn reader_text(&self, function: &str, args: &[Value]) -> Option<String> {
        let first = args.first().unwrap_or(&Value::Null);
        let word = args.get(1).and_then(Value::as_str).unwrap_or("");
        let you = self.is_reader(first);
        let pronoun = |second: &str, third: &str| Some(if you { second } else { third }.to_owned());
        match function {
            "name" => Some(self.name_for_reader(first)),
            "the" if you => Some("you".to_owned()),
            "the" => Some(format!("the {}", self.name_for_reader(first))),
            "a" if you => Some("you".to_owned()),
            "a" => Some(format!("a {}", self.name_for_reader(first))),
            "they" => pronoun("you", "they"),
            "them" => pronoun("you", "them"),
            "their" => pronoun("your", "their"),
            "themself" => pronoun("yourself", "themself"),
            "verb" if you => Some(word.to_owned()),
            "verb" => Some(format!("{word}s")),
            "count" => {
                let one = first.as_number() == Some(Real::ONE);
                Some(format!("{first} {word}{}", if one { "" } else { "s" }))
            }
            "plural" => Some(format!("{}s", first.as_str().unwrap_or(""))),
            "list" => {
                let names: Vec<String> = first
                    .as_list()
                    .unwrap_or(&[])
                    .iter()
                    .map(|item| self.name_for_reader(item))
                    .collect();
                Some(match names.split_last() {
                    None => String::new(),
                    Some((only, [])) => only.clone(),
                    Some((last, rest)) => format!("{} and {last}", rest.join(", ")),
                })
            }
            _ => None,
        }
    }
}

/// Standard gravity, for the test function `ashlands:fall_damage`.
const STANDARD_GRAVITY: Real = Real::of(9.806_65);
/// The mass `ashlands:fall_damage` assumes when it is given none.
const DEFAULT_FALL_MASS: Real = Real::of(70.0);

impl Host for TestHost {
    fn subject(&mut self, slot: u16) -> Value {
        self.subjects
            .get(usize::from(slot))
            .cloned()
            .unwrap_or_default()
    }

    fn local(&mut self, slot: u16) -> Value {
        self.locals
            .get(usize::from(slot))
            .cloned()
            .unwrap_or_default()
    }

    fn event_field(&mut self, slot: u16) -> Value {
        self.event
            .get(usize::from(slot))
            .cloned()
            .unwrap_or_default()
    }

    fn property(&mut self, property: PropertyId, subject: Handle, args: &[Value]) -> Value {
        let Some(spec) = self.registry.property(property).map(|found| &found.spec) else {
            return Value::Null;
        };
        let mut key = spec.name.clone();
        if !args.is_empty() {
            let args: Vec<String> = args.iter().map(Value::to_string).collect();
            key = format!("{key}({})", args.join(","));
        }
        self.log.push(format!("{}.{key}", Value::Subject(subject)));
        if spec.name == "has_tag" {
            let tags = self.properties.get(&(subject, "tags".to_owned()));
            let wanted = args.first().unwrap_or(&Value::Null);
            let has = tags
                .and_then(Value::as_list)
                .is_some_and(|tags| tags.contains(wanted));
            return Value::Bool(has);
        }
        self.properties
            .get(&(subject, key))
            .cloned()
            .unwrap_or_default()
    }

    fn variable(&mut self, subject: Handle, name: &str) -> Value {
        self.log
            .push(format!("{}.var.{name}", Value::Subject(subject)));
        if let Some(value) = self.variables.get(&(subject, name.to_owned())) {
            return value.clone();
        }
        // A declared variable that was never set reads as its default.
        let declared = self
            .registry
            .variable_id(name)
            .and_then(|id| self.registry.variable(id));
        match declared.map(|variable| &variable.spec.ty) {
            Some(Type::Int) => Value::Int(0),
            Some(Type::Bool) => Value::Bool(false),
            _ => Value::Null,
        }
    }

    fn call(&mut self, function: FunctionId, args: &[Value]) -> Value {
        let Some(name) = self
            .registry
            .function(function)
            .map(|found| found.spec.name.clone())
        else {
            return Value::Null;
        };
        let shown: Vec<String> = args.iter().map(Value::to_string).collect();
        self.log.push(format!("{name}({})", shown.join(",")));
        let first = args.first().unwrap_or(&Value::Null);
        match name.as_str() {
            "dist" => match (first.as_subject(), args.get(1).and_then(Value::as_subject)) {
                (Some(a), Some(b)) => {
                    let apart = a.payload.abs_diff(b.payload);
                    Value::Real(Real::from_i64(i64::try_from(apart).unwrap_or(i64::MAX)))
                }
                _ => Value::Null,
            },
            "secret" => Value::Int(42),
            "roll" => match (first.as_int(), self.rng.as_mut()) {
                (Some(sides), Some(rng)) => Value::Int(rng.range(1, sides)),
                _ => Value::Null,
            },
            "core:double" | "ashlands:double" => match first {
                Value::Int(int) => int.checked_mul(2).map_or(Value::Null, Value::Int),
                Value::Real(real) => Value::Real(*real * Real::of(2.0)),
                _ => Value::Null,
            },
            "ashlands:fall_damage" => match first.as_number() {
                Some(height) => {
                    let mass = args
                        .get(1)
                        .and_then(Value::as_number)
                        .unwrap_or(DEFAULT_FALL_MASS);
                    Value::Real(mass * STANDARD_GRAVITY * height)
                }
                None => Value::Null,
            },
            // Entities numbered below a thousand exist; the rest are
            // stale references.
            "exists" => Value::Bool(first.as_subject().is_some_and(|h| h.payload < 1000)),
            other => self
                .reader_text(other, args)
                .map_or(Value::Null, Value::from),
        }
    }

    fn rng(&mut self) -> Option<&mut Rng> {
        self.rng.as_mut()
    }

    fn warn(&mut self, warning: RuntimeWarning) {
        self.warnings.push(warning);
    }

    fn def_name(&self, id: DefRef) -> Option<&str> {
        self.registry.def_name(id)
    }
}
