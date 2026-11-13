//! What a definition type's fields are, read from the type itself
//! (04 §13): the fields of every struct it holds, their shapes, which
//! are required, the values of its enums, the units of its quantities
//! and the types its references name.
//!
//! A definition type is a plain struct that derives `Deserialize`, and
//! `serde` tells a deserialiser the names of a struct's fields, the
//! values of an enum and, through the reserved names of [`crate::de`],
//! what each field type of this crate is. [`describe`] drives the type's
//! own `Deserialize` with a deserialiser that answers every question
//! with a harmless value and writes down what it was asked. It does so
//! as many times as it takes to see every field, every value of every
//! enum, every kind of JSON a field that takes several accepts, and
//! which fields cannot be left out: a struct is read once with each of
//! its fields left out, and a field whose absence is an error is
//! required. Nothing is hand-listed: what the generated reference and
//! the schemas say is what the loader reads.
//!
//! Field documentation is not here: Rust doc comments do not reach a
//! running program. The tools join it on by the Rust type, which
//! [`Named::rust`] gives (decision D-801).

use std::any::type_name;
use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::fmt;

use serde::de::{
    self, DeserializeOwned, DeserializeSeed, Deserializer, EnumAccess, IntoDeserializer, MapAccess,
    SeqAccess, Unexpected, VariantAccess, Visitor,
};

use crate::de::{
    GATED_MAGIC, INLINE_MAGIC, KIND_MAGIC, LOCATED_MAGIC, MICROS_MAGIC, QUANTITY_MAGICS, REF_MAGIC,
    SITE_MAGIC, SITE_OR_PLAIN_MAGIC, TAG_MAGIC,
};
use crate::fields::Kind;
use crate::units::Dim;

/// How deeply the probe follows a type into itself before it stops.
const MAX_DEPTH: usize = 40;

/// How many times one type is read before the probe settles for what it
/// has seen. A type of the engine needs a few hundred at most.
const MAX_RUNS: usize = 50_000;

/// How many times a reading that stopped at a type whose kinds were not
/// yet known is made again.
const MAX_RETRIES: usize = 4;

/// The shape of a value, as a definition type reads it.
#[derive(Clone, PartialEq, Debug)]
pub enum Shape {
    /// Not seen: reading stopped before this value was reached.
    Unknown,
    /// `true` or `false`.
    Bool,
    /// A whole number.
    Int {
        /// Whether it may be negative.
        signed: bool,
    },
    /// A number.
    Real,
    /// One character, written as a string.
    Char,
    /// A string.
    String,
    /// A quantity of this dimension: a number in SI units, or a string
    /// with a unit (04 §7.1).
    Quantity(Dim),
    /// A duration, kept in whole microseconds: a number of seconds or a
    /// string with a unit of time.
    Duration,
    /// The id of a definition of the named type.
    Ref(&'static str),
    /// The id of a definition of the named type, or one written in
    /// place (04 §5).
    Inline(&'static str),
    /// A tag: a free-form string.
    Tag,
    /// A list of values of one shape.
    List(Box<Shape>),
    /// A list of a fixed length, each place of its own shape.
    Tuple(Vec<Shape>),
    /// An object whose keys and values each have one shape.
    Map(Box<Shape>, Box<Shape>),
    /// A value of the inner shape, or `null`.
    Optional(Box<Shape>),
    /// A struct, an enum or a field that takes several kinds of JSON,
    /// described in [`Shapes::named`] under this key.
    Named(String),
    /// A value of an experimental feature (04 §8.4).
    Gated(&'static str, Box<Shape>),
    /// Anything: the value is not looked at.
    Any,
    /// `null`.
    Unit,
}

/// A required or optional field of a struct.
#[derive(Clone, PartialEq, Debug)]
pub struct FieldShape {
    /// The key, as written in JSON.
    pub name: &'static str,
    /// Its shape.
    pub shape: Shape,
    /// Whether a definition must give it.
    pub required: bool,
    /// What the probe could not read past, if anything: the field's own
    /// syntax refuses a made-up value.
    pub note: Option<String>,
}

/// A struct: an object with known keys.
#[derive(Clone, PartialEq, Debug)]
pub struct StructShape {
    /// The fields, in the order the type declares them.
    pub fields: Vec<FieldShape>,
}

/// How a value of an enum is written.
#[derive(Clone, PartialEq, Debug)]
pub enum VariantForm {
    /// Not seen.
    Unknown,
    /// The string alone: `"open"`.
    Unit,
    /// An object with the value's name as its one key, and this shape
    /// as its value: `{ "fixed": 3 }`.
    Newtype(Shape),
    /// An object with the name as its one key, and a list as its value.
    Tuple(Vec<Shape>),
    /// An object with the name as its one key, and an object of these
    /// fields as its value. The key into [`Shapes::named`].
    Struct(String),
}

/// One value of an enum.
#[derive(Clone, PartialEq, Debug)]
pub struct VariantShape {
    /// The name, as written in JSON.
    pub name: &'static str,
    /// How it is written.
    pub form: VariantForm,
}

/// An enum: one of a fixed set of values.
#[derive(Clone, PartialEq, Debug)]
pub struct EnumShape {
    /// The values, in the order the type declares them.
    pub variants: Vec<VariantShape>,
}

/// How a field that takes several kinds of JSON is read.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Via {
    /// With [`by_kind`](crate::by_kind): the loader says what kind of
    /// value is there.
    ByKind,
    /// With [`by_site`](crate::by_site): the same, and the value keeps
    /// where it was written, to be compiled after loading (an
    /// expression, a condition, a text, a list of effects).
    BySite,
    /// By a visitor of the type's own, which takes whichever kinds it
    /// can.
    Any,
}

/// A kind of JSON value, as [`KindsShape`] lists them.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub enum JsonKind {
    /// A string.
    String,
    /// An object.
    Object,
    /// A whole number.
    Integer,
    /// Any number.
    Number,
    /// `true` or `false`.
    Bool,
    /// A list.
    List,
    /// `null`.
    Null,
}

impl JsonKind {
    /// Every kind, in the order they are tried.
    pub const ALL: [JsonKind; 7] = [
        JsonKind::String,
        JsonKind::Object,
        JsonKind::Integer,
        JsonKind::Number,
        JsonKind::Bool,
        JsonKind::List,
        JsonKind::Null,
    ];

    /// The name a reader knows the kind by.
    pub fn name(self) -> &'static str {
        match self {
            JsonKind::String => "string",
            JsonKind::Object => "object",
            JsonKind::Integer => "integer",
            JsonKind::Number => "number",
            JsonKind::Bool => "boolean",
            JsonKind::List => "list",
            JsonKind::Null => "null",
        }
    }

    /// The loader's [`Kind`] of a value of this kind.
    fn kind(self) -> Kind {
        match self {
            JsonKind::String => Kind::String,
            JsonKind::Object => Kind::Object,
            JsonKind::Integer | JsonKind::Number => Kind::Number,
            JsonKind::Bool => Kind::Bool,
            JsonKind::List => Kind::List,
            JsonKind::Null => Kind::Null,
        }
    }

    /// The name the loader feeds the kind under.
    fn loader_name(self) -> &'static str {
        match self.kind() {
            Kind::Null => "null",
            Kind::Bool => "bool",
            Kind::Number => "number",
            Kind::String => "string",
            Kind::List => "list",
            Kind::Object => "object",
        }
    }
}

/// A field type that takes several kinds of JSON, and what it makes of
/// each.
#[derive(Clone, PartialEq, Debug)]
pub struct KindsShape {
    /// How it is read.
    pub via: Via,
    /// The kinds it takes, each with the shape it reads that kind as
    /// ([`Shape::Any`] where it reads the value itself).
    pub accepted: Vec<(JsonKind, Shape)>,
    /// What the type says it takes, from a kind it refused.
    pub expected: Option<String>,
}

/// What [`Shapes::named`] holds under a key.
#[derive(Clone, PartialEq, Debug)]
pub enum NamedShape {
    /// A struct.
    Struct(StructShape),
    /// An enum.
    Enum(EnumShape),
    /// A field type that takes several kinds of JSON.
    Kinds(KindsShape),
}

/// A struct, an enum or a field type of several kinds, with the Rust
/// type it was read from.
#[derive(Clone, PartialEq, Debug)]
pub struct Named {
    /// The Rust type, as `std::any::type_name` writes it. Its path is
    /// what documentation is found by; it is not stable from one
    /// compiler to the next and is never printed.
    pub rust: String,
    /// The name a reader knows it by: the Rust type's own name without
    /// its path and generic arguments, or for a struct written as a
    /// value of an enum, `Enum.value`.
    pub name: String,
    /// What it is.
    pub shape: NamedShape,
}

/// Everything one type reaches: its own shape and every struct, enum
/// and field type of several kinds below it.
#[derive(Clone, PartialEq, Debug)]
pub struct Shapes {
    /// The type itself: for a definition type, a [`Shape::Named`]
    /// struct.
    pub root: Shape,
    /// Every named shape it reaches, by key.
    pub named: BTreeMap<String, Named>,
    /// Whether the probe stopped before it had seen everything.
    pub incomplete: bool,
    /// How many times the type was read to see all this.
    pub runs: usize,
}

impl Shapes {
    /// The struct under a key, if it is one.
    pub fn struct_shape(&self, key: &str) -> Option<&StructShape> {
        match &self.named.get(key)?.shape {
            NamedShape::Struct(shape) => Some(shape),
            _ => None,
        }
    }

    /// The struct the root names, if the root is a struct.
    pub fn root_struct(&self) -> Option<&StructShape> {
        match &self.root {
            Shape::Named(key) => self.struct_shape(key),
            _ => None,
        }
    }
}

/// Reads what `T` is made of. See the module's documentation.
pub fn describe<T: DeserializeOwned>() -> Shapes {
    let mut explorer = Explorer::default();
    let mut root = Shape::Unknown;
    let mut queue: VecDeque<Config> = VecDeque::new();
    let mut tried: BTreeSet<Config> = BTreeSet::new();
    let mut retries: BTreeMap<Config, usize> = BTreeMap::new();
    queue.push_back(Config::default());
    let mut runs = 0;
    let mut incomplete = false;
    while let Some(config) = queue.pop_front() {
        if !tried.insert(config.clone()) {
            continue;
        }
        runs += 1;
        if runs > MAX_RUNS {
            incomplete = true;
            break;
        }
        let run = RefCell::new(Run {
            config: &config,
            ex: &mut explorer,
            stack: Vec::new(),
            last: Shape::Unknown,
            next: Vec::new(),
            retry: false,
        });
        let _ = T::deserialize(Probe {
            run: &run,
            shallow: false,
        });
        let run = run.into_inner();
        if root == Shape::Unknown {
            root = run.last;
        }
        queue.extend(run.next);
        if run.retry {
            let count = retries.entry(config.clone()).or_insert(0_usize);
            if *count < MAX_RETRIES {
                *count += 1;
                tried.remove(&config);
                queue.push_back(config);
            }
        }
    }
    Shapes {
        root,
        named: explorer.named,
        incomplete,
        runs,
    }
}

/// What one reading of the type is told to do differently.
#[derive(Clone, Default, PartialEq, Eq, PartialOrd, Ord, Debug)]
struct Config {
    /// For an enum or a type of several kinds, by key: which value or
    /// kind to take. The first otherwise.
    choices: BTreeMap<String, usize>,
    /// A field of a struct to leave out, to see whether it is required.
    omit: Option<(String, &'static str)>,
    /// Fields of structs not to give, because no made-up value is good
    /// enough for them and what follows them is to be seen.
    skip: BTreeSet<(String, &'static str)>,
}

impl Config {
    fn choosing(&self, key: &str, choice: usize) -> Config {
        let mut config = self.clone();
        config.choices.insert(key.to_owned(), choice);
        config
    }

    fn omitting(&self, key: &str, field: &'static str) -> Config {
        let mut config = self.clone();
        config.omit = Some((key.to_owned(), field));
        config
    }

    fn skipping(&self, key: &str, field: &'static str) -> Config {
        let mut config = self.clone();
        config.skip.insert((key.to_owned(), field));
        config
    }
}

/// What is known so far, across readings.
#[derive(Default)]
struct Explorer {
    named: BTreeMap<String, Named>,
    /// Types whose choices have been queued.
    chosen: BTreeSet<String>,
    /// Structs whose fields have been queued to be left out.
    omitted: BTreeSet<String>,
    /// For a type of several kinds, the first kind found to be taken: a
    /// reading that has no choice for it takes that kind.
    preferred: BTreeMap<String, usize>,
    /// Types whose preferred choice was read without a fault: the
    /// choice a reading that has none of its own makes, so that what
    /// lies beyond the type is reached.
    settled: BTreeSet<String>,
}

/// One reading of the type.
struct Run<'c> {
    config: &'c Config,
    ex: &'c mut Explorer,
    /// The keys of the structs and enums being read, outermost first.
    stack: Vec<String>,
    /// The shape of the value last read.
    last: Shape,
    /// Readings to make next.
    next: Vec<Config>,
    /// Whether this reading stopped at a type before any kind it takes
    /// was known, and is to be made again.
    retry: bool,
}

/// What reading a value with the probe can end in.
#[derive(Clone, Debug)]
enum ProbeError {
    /// A required field was not given.
    Missing(&'static str),
    /// A value of the wrong kind: what the type says it expected.
    Rejected(String),
    /// A kind or a value the type refused, which this reading was made
    /// to find out: nothing to put right.
    Explored,
    /// The type refused a made-up value.
    Custom(String),
    /// The type goes into itself further than the probe follows.
    TooDeep,
}

impl fmt::Display for ProbeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ProbeError::Missing(field) => write!(f, "missing field `{field}`"),
            ProbeError::Rejected(expected) => write!(f, "expected {expected}"),
            ProbeError::Explored => f.write_str("explored"),
            ProbeError::Custom(message) => f.write_str(message),
            ProbeError::TooDeep => f.write_str("nested too deeply"),
        }
    }
}

impl std::error::Error for ProbeError {}

impl de::Error for ProbeError {
    fn custom<T: fmt::Display>(message: T) -> ProbeError {
        ProbeError::Custom(message.to_string())
    }

    fn missing_field(field: &'static str) -> ProbeError {
        ProbeError::Missing(field)
    }

    fn invalid_type(_unexpected: Unexpected<'_>, expected: &dyn de::Expected) -> ProbeError {
        ProbeError::Rejected(expected.to_string())
    }
}

/// The name a reader knows a Rust type by: its last path segment,
/// without generic arguments.
fn short_name(rust: &str) -> String {
    let base = rust.split('<').next().unwrap_or(rust);
    base.rsplit("::").next().unwrap_or(base).to_owned()
}

/// The deserialiser that writes down what it is asked.
#[derive(Clone, Copy)]
struct Probe<'r, 'c> {
    run: &'r RefCell<Run<'c>>,
    /// Inside a type that is already being read further out: give
    /// nothing that leads further in, and write nothing down.
    shallow: bool,
}

impl<'r, 'c> Probe<'r, 'c> {
    fn set_last(&self, shape: Shape) {
        self.run.borrow_mut().last = shape;
    }

    fn take_last(&self) -> Shape {
        std::mem::replace(&mut self.run.borrow_mut().last, Shape::Unknown)
    }

    /// Whether a type is being read already, or the reading has gone
    /// too deep: then it is read shallowly.
    fn enters(&self, key: &str) -> Result<bool, ProbeError> {
        let mut run = self.run.borrow_mut();
        if run.stack.len() >= MAX_DEPTH {
            run.last = Shape::Named(key.to_owned());
            return Err(ProbeError::TooDeep);
        }
        Ok(self.shallow || run.stack.iter().any(|open| open == key))
    }

    /// The choice this reading makes for a type with `count` choices,
    /// queueing the others the first time the type is met.
    fn choose(&self, key: &str, count: usize, shallow: bool) -> usize {
        let mut run = self.run.borrow_mut();
        if let Some(&choice) = run.config.choices.get(key) {
            return choice;
        }
        if !shallow && run.ex.chosen.insert(key.to_owned()) {
            let configs: Vec<Config> = (1..count)
                .map(|choice| run.config.choosing(key, choice))
                .collect();
            run.next.extend(configs);
        }
        run.ex.preferred.get(key).copied().unwrap_or(0)
    }

    fn record(&self, key: &str, rust: &str, name: String, make: impl FnOnce() -> NamedShape) {
        let mut run = self.run.borrow_mut();
        run.ex.named.entry(key.to_owned()).or_insert_with(|| Named {
            rust: rust.to_owned(),
            name,
            shape: make(),
        });
    }

    /// Reads a struct, or a value of an enum written as a struct: gives
    /// every field but those the reading leaves out, and writes down the
    /// shape of each.
    fn read_struct<'de, V: Visitor<'de>>(
        self,
        key: String,
        rust: &str,
        name: String,
        fields: &'static [&'static str],
        visitor: V,
    ) -> Result<V::Value, ProbeError> {
        let shallow = self.enters(&key)?;
        if shallow {
            self.run.borrow_mut().stack.push(key.clone());
            let result = visitor.visit_map(Fields {
                probe: Probe {
                    run: self.run,
                    shallow: true,
                },
                key: key.clone(),
                fields: fields.to_vec(),
                at: 0,
            });
            let mut run = self.run.borrow_mut();
            run.stack.pop();
            run.last = Shape::Named(key);
            return result;
        }
        self.record(&key, rust, name, || {
            NamedShape::Struct(StructShape {
                fields: fields
                    .iter()
                    .map(|&field| FieldShape {
                        name: field,
                        shape: Shape::Unknown,
                        required: false,
                        note: None,
                    })
                    .collect(),
            })
        });
        let given: Vec<&'static str> = {
            let mut run = self.run.borrow_mut();
            if run.config.omit.is_none() && run.ex.omitted.insert(key.clone()) {
                let configs: Vec<Config> = fields
                    .iter()
                    .map(|field| run.config.omitting(&key, field))
                    .collect();
                run.next.extend(configs);
            }
            let config = run.config;
            fields
                .iter()
                .copied()
                .filter(|field| {
                    config.omit.as_ref() != Some(&(key.clone(), *field))
                        && !config.skip.contains(&(key.clone(), *field))
                })
                .collect()
        };
        self.run.borrow_mut().stack.push(key.clone());
        let result = visitor.visit_map(Fields {
            probe: self,
            key: key.clone(),
            fields: given,
            at: 0,
        });
        let mut run = self.run.borrow_mut();
        run.stack.pop();
        if let Err(ProbeError::Missing(missing)) = &result
            && run.config.omit.as_ref() == Some(&(key.clone(), *missing))
            && let Some(Named {
                shape: NamedShape::Struct(shape),
                ..
            }) = run.ex.named.get_mut(&key)
            && let Some(field) = shape.fields.iter_mut().find(|field| field.name == *missing)
        {
            field.required = true;
        }
        run.last = Shape::Named(key);
        result
    }

    /// Reads a field type that takes several kinds of JSON: in each
    /// reading, one kind.
    fn read_kinds<'de, V: Visitor<'de>>(
        self,
        via: Via,
        visitor: V,
    ) -> Result<V::Value, ProbeError> {
        let rust = type_name::<V::Value>();
        // A visitor of a type's own may read one Rust type in several
        // ways (a number of metres, a number of pascals): what it
        // expects tells them apart.
        let key = match via {
            Via::Any => format!("{rust}#{}", (&visitor as &dyn de::Expected)),
            Via::ByKind | Via::BySite => rust.to_owned(),
        };
        let shallow = self.enters(&key)?;
        let explicit = self.run.borrow().config.choices.contains_key(&key);
        let unsettled = !explicit && !self.run.borrow().ex.settled.contains(&key);
        let choice = self.choose(&key, JsonKind::ALL.len(), shallow);
        let kind = JsonKind::ALL[choice.min(JsonKind::ALL.len() - 1)];
        if !shallow {
            self.record(&key, rust, short_name(rust), || {
                NamedShape::Kinds(KindsShape {
                    via,
                    accepted: Vec::new(),
                    expected: None,
                })
            });
        }
        self.run.borrow_mut().stack.push(key.clone());
        let inner = Probe {
            run: self.run,
            shallow,
        };
        let read = RefCell::new(None);
        let result = match via {
            Via::ByKind => visitor.visit_map(Entries {
                probe: inner,
                entries: &[("kind", Fed::Kind(kind)), ("value", Fed::Probe)],
                at: 0,
                read: &read,
            }),
            Via::BySite => visitor.visit_map(Entries {
                probe: inner,
                entries: &[
                    ("file", Fed::Zero),
                    ("start", Fed::Zero),
                    ("end", Fed::Zero),
                    ("path", Fed::Empty),
                    ("kind", Fed::Kind(kind)),
                    ("value", Fed::Probe),
                ],
                at: 0,
                read: &read,
            }),
            Via::Any => match kind {
                JsonKind::String => {
                    *read.borrow_mut() = Some(Shape::String);
                    visitor.visit_str("")
                }
                JsonKind::Object => {
                    *read.borrow_mut() =
                        Some(Shape::Map(Box::new(Shape::String), Box::new(Shape::Any)));
                    visitor.visit_map(Entries {
                        probe: inner,
                        entries: &[],
                        at: 0,
                        read: &read,
                    })
                }
                JsonKind::Integer => {
                    *read.borrow_mut() = Some(Shape::Int { signed: false });
                    visitor.visit_u64(0)
                }
                JsonKind::Number => {
                    *read.borrow_mut() = Some(Shape::Real);
                    visitor.visit_f64(0.0)
                }
                JsonKind::Bool => {
                    *read.borrow_mut() = Some(Shape::Bool);
                    visitor.visit_bool(false)
                }
                JsonKind::List => {
                    let mut elements = Elements {
                        probe: inner,
                        count: usize::from(!shallow),
                        shapes: Vec::new(),
                    };
                    let result = visitor.visit_seq(&mut elements);
                    let element = elements.shapes.into_iter().next().unwrap_or(Shape::Any);
                    *read.borrow_mut() = Some(Shape::List(Box::new(element)));
                    result
                }
                JsonKind::Null => {
                    *read.borrow_mut() = Some(Shape::Unit);
                    visitor.visit_unit()
                }
            },
        };
        let mut run = self.run.borrow_mut();
        run.stack.pop();
        if shallow {
            run.last = Shape::Named(key);
            return result.map_err(|error| match error {
                ProbeError::Rejected(_) => ProbeError::Explored,
                other => other,
            });
        }
        let read = read.into_inner();
        // A kind is refused when the type turns it away without reading
        // it. One it read and then refused was refused for what the
        // probe made up, `""` where the type wants `"all"`: the kind is
        // taken.
        let refused = match &result {
            Err(ProbeError::Rejected(expected)) if read.is_none() || via == Via::Any => {
                Some(expected.clone())
            }
            _ => None,
        };
        if result.is_err() && unsettled && !shallow {
            // Read again once a kind the type takes without a fault is
            // known, to see what lies beyond it.
            run.retry = true;
        }
        if result.is_ok() && !shallow && run.ex.settled.insert(key.clone()) {
            run.ex.preferred.insert(key.clone(), choice);
        }
        if let Some(Named {
            shape: NamedShape::Kinds(shape),
            ..
        }) = run.ex.named.get_mut(&key)
        {
            match refused {
                Some(expected) => {
                    if shape.expected.is_none() && !expected.contains("read by the mod loader") {
                        shape.expected = Some(expected);
                    }
                }
                None => {
                    let fits = match &read {
                        Some(shape) => fits(kind, shape),
                        None => true,
                    };
                    if fits && !shape.accepted.iter().any(|(known, _)| *known == kind) {
                        shape
                            .accepted
                            .push((kind, read.clone().unwrap_or(Shape::Any)));
                        shape.accepted.sort_by_key(|(kind, _)| *kind);
                    }
                    if fits {
                        run.ex.preferred.entry(key.clone()).or_insert(choice);
                    }
                }
            }
        }
        run.last = Shape::Named(key);
        result.map_err(|error| match error {
            // A kind refused, or a fault in a kind this reading chose to
            // see: what it was made to find out, not something to read
            // past.
            ProbeError::Rejected(_) => ProbeError::Explored,
            _ if explicit => ProbeError::Explored,
            other => other,
        })
    }
}

/// Whether a value of a kind could be read as a shape: a type that
/// reads a number as a string is refused by the loader, whatever the
/// probe made of it.
fn fits(kind: JsonKind, shape: &Shape) -> bool {
    match shape {
        Shape::Unknown | Shape::Any | Shape::Named(_) => true,
        Shape::Optional(inner) => kind == JsonKind::Null || fits(kind, inner),
        Shape::Gated(_, inner) => fits(kind, inner),
        Shape::Bool => kind == JsonKind::Bool,
        Shape::Int { .. } => kind == JsonKind::Integer,
        Shape::Real => matches!(kind, JsonKind::Integer | JsonKind::Number),
        Shape::Quantity(_) | Shape::Duration => {
            matches!(
                kind,
                JsonKind::Integer | JsonKind::Number | JsonKind::String
            )
        }
        Shape::Char | Shape::String | Shape::Ref(_) | Shape::Tag => kind == JsonKind::String,
        Shape::Inline(_) => matches!(kind, JsonKind::String | JsonKind::Object),
        Shape::List(_) | Shape::Tuple(_) => kind == JsonKind::List,
        Shape::Map(..) => kind == JsonKind::Object,
        Shape::Unit => kind == JsonKind::Null,
    }
}

/// What [`Entries`] feeds under a key.
#[derive(Clone, Copy)]
enum Fed {
    Zero,
    Empty,
    Kind(JsonKind),
    Probe,
}

/// Feeds a fixed set of entries: the reserved structs of the loader.
struct Entries<'r, 'c, 'a> {
    probe: Probe<'r, 'c>,
    entries: &'a [(&'static str, Fed)],
    at: usize,
    read: &'a RefCell<Option<Shape>>,
}

impl<'de> MapAccess<'de> for Entries<'_, '_, '_> {
    type Error = ProbeError;

    fn next_key_seed<K: DeserializeSeed<'de>>(
        &mut self,
        seed: K,
    ) -> Result<Option<K::Value>, ProbeError> {
        match self.entries.get(self.at) {
            Some((key, _)) => seed
                .deserialize(IntoDeserializer::<ProbeError>::into_deserializer(*key))
                .map(Some),
            None => Ok(None),
        }
    }

    fn next_value_seed<V: DeserializeSeed<'de>>(
        &mut self,
        seed: V,
    ) -> Result<V::Value, ProbeError> {
        let Some(&(_, fed)) = self.entries.get(self.at) else {
            return Err(ProbeError::Custom("no value".to_owned()));
        };
        self.at += 1;
        match fed {
            Fed::Zero => seed.deserialize(IntoDeserializer::<ProbeError>::into_deserializer(0_u32)),
            Fed::Empty => seed.deserialize(IntoDeserializer::<ProbeError>::into_deserializer("")),
            Fed::Kind(kind) => seed.deserialize(IntoDeserializer::<ProbeError>::into_deserializer(
                kind.loader_name(),
            )),
            Fed::Probe => {
                let result = seed.deserialize(self.probe);
                *self.read.borrow_mut() = Some(self.probe.take_last());
                result
            }
        }
    }
}

/// Feeds the fields of a struct, one at a time.
struct Fields<'r, 'c> {
    probe: Probe<'r, 'c>,
    key: String,
    fields: Vec<&'static str>,
    at: usize,
}

impl<'de> MapAccess<'de> for Fields<'_, '_> {
    type Error = ProbeError;

    fn next_key_seed<K: DeserializeSeed<'de>>(
        &mut self,
        seed: K,
    ) -> Result<Option<K::Value>, ProbeError> {
        match self.fields.get(self.at) {
            Some(field) => seed
                .deserialize(IntoDeserializer::<ProbeError>::into_deserializer(*field))
                .map(Some),
            None => Ok(None),
        }
    }

    fn next_value_seed<V: DeserializeSeed<'de>>(
        &mut self,
        seed: V,
    ) -> Result<V::Value, ProbeError> {
        let Some(&field) = self.fields.get(self.at) else {
            return Err(ProbeError::Custom("no value".to_owned()));
        };
        self.at += 1;
        let result = seed.deserialize(self.probe);
        let shape = self.probe.take_last();
        if self.probe.shallow {
            return result;
        }
        let mut run = self.probe.run.borrow_mut();
        let failed = match &result {
            Err(ProbeError::Explored) | Ok(_) => None,
            // A field this reading leaves out on purpose, further in:
            // what it was made to find out.
            Err(ProbeError::Missing(missing))
                if run
                    .config
                    .omit
                    .as_ref()
                    .is_some_and(|(_, omitted)| omitted == missing) =>
            {
                None
            }
            Err(error) => Some(error.to_string()),
        };
        if let Some(Named {
            shape: NamedShape::Struct(record),
            ..
        }) = run.ex.named.get_mut(&self.key)
            && let Some(record) = record.fields.iter_mut().find(|known| known.name == field)
        {
            if record.shape == Shape::Unknown || matches!(record.shape, Shape::Named(_)) {
                record.shape = shape;
            }
            if let Some(message) = &failed
                && record.note.is_none()
            {
                record.note = Some(message.clone());
            }
        }
        if failed.is_some() {
            let config = run.config.skipping(&self.key, field);
            run.next.push(config);
        }
        result
    }
}

/// Feeds the elements of a list.
struct Elements<'r, 'c> {
    probe: Probe<'r, 'c>,
    count: usize,
    shapes: Vec<Shape>,
}

impl<'de> SeqAccess<'de> for Elements<'_, '_> {
    type Error = ProbeError;

    fn next_element_seed<T: DeserializeSeed<'de>>(
        &mut self,
        seed: T,
    ) -> Result<Option<T::Value>, ProbeError> {
        if self.shapes.len() >= self.count {
            return Ok(None);
        }
        let result = seed.deserialize(self.probe);
        self.shapes.push(self.probe.take_last());
        result.map(Some)
    }
}

/// Feeds one entry of a map.
struct Entry<'r, 'c> {
    probe: Probe<'r, 'c>,
    given: bool,
    key: Shape,
    value: Shape,
}

impl<'de> MapAccess<'de> for Entry<'_, '_> {
    type Error = ProbeError;

    fn next_key_seed<K: DeserializeSeed<'de>>(
        &mut self,
        seed: K,
    ) -> Result<Option<K::Value>, ProbeError> {
        if self.given {
            return Ok(None);
        }
        self.given = true;
        let result = seed.deserialize(self.probe);
        self.key = self.probe.take_last();
        result.map(Some)
    }

    fn next_value_seed<V: DeserializeSeed<'de>>(
        &mut self,
        seed: V,
    ) -> Result<V::Value, ProbeError> {
        let result = seed.deserialize(self.probe);
        self.value = self.probe.take_last();
        result
    }
}

/// Feeds one value of an enum.
struct Variant<'r, 'c> {
    probe: Probe<'r, 'c>,
    key: String,
    rust: String,
    enum_name: String,
    name: &'static str,
}

impl<'de, 'r, 'c> EnumAccess<'de> for Variant<'r, 'c> {
    type Error = ProbeError;
    type Variant = Variant<'r, 'c>;

    fn variant_seed<S: DeserializeSeed<'de>>(
        self,
        seed: S,
    ) -> Result<(S::Value, Variant<'r, 'c>), ProbeError> {
        let value =
            seed.deserialize(IntoDeserializer::<ProbeError>::into_deserializer(self.name))?;
        Ok((value, self))
    }
}

impl Variant<'_, '_> {
    fn set_form(&self, form: VariantForm) {
        if self.probe.shallow {
            return;
        }
        let mut run = self.probe.run.borrow_mut();
        if let Some(Named {
            shape: NamedShape::Enum(shape),
            ..
        }) = run.ex.named.get_mut(&self.key)
            && let Some(variant) = shape
                .variants
                .iter_mut()
                .find(|variant| variant.name == self.name)
            && matches!(variant.form, VariantForm::Unknown)
        {
            variant.form = form;
        }
    }
}

impl<'de> VariantAccess<'de> for Variant<'_, '_> {
    type Error = ProbeError;

    fn unit_variant(self) -> Result<(), ProbeError> {
        self.set_form(VariantForm::Unit);
        Ok(())
    }

    fn newtype_variant_seed<T: DeserializeSeed<'de>>(
        self,
        seed: T,
    ) -> Result<T::Value, ProbeError> {
        let result = seed.deserialize(self.probe);
        let shape = self.probe.take_last();
        self.set_form(VariantForm::Newtype(shape));
        result
    }

    fn tuple_variant<V: Visitor<'de>>(
        self,
        len: usize,
        visitor: V,
    ) -> Result<V::Value, ProbeError> {
        let mut elements = Elements {
            probe: self.probe,
            count: if self.probe.shallow { 0 } else { len },
            shapes: Vec::new(),
        };
        let result = visitor.visit_seq(&mut elements);
        self.set_form(VariantForm::Tuple(elements.shapes));
        result
    }

    fn struct_variant<V: Visitor<'de>>(
        self,
        fields: &'static [&'static str],
        visitor: V,
    ) -> Result<V::Value, ProbeError> {
        let key = format!("{}::{}", self.key, self.name);
        let name = format!("{}.{}", self.enum_name, self.name);
        self.set_form(VariantForm::Struct(key.clone()));
        let rust = format!("{}::{}", self.rust, self.name);
        self.probe.read_struct(key, &rust, name, fields, visitor)
    }
}

macro_rules! probe_number {
    ($($method:ident, $visit:ident, $zero:expr, $shape:expr;)*) => {
        $(
            fn $method<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, ProbeError> {
                self.set_last($shape);
                let result = visitor.$visit($zero);
                self.set_last($shape);
                result
            }
        )*
    };
}

impl<'de> Deserializer<'de> for Probe<'_, '_> {
    type Error = ProbeError;

    fn deserialize_any<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, ProbeError> {
        self.read_kinds(Via::Any, visitor)
    }

    fn deserialize_bool<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, ProbeError> {
        let result = visitor.visit_bool(false);
        self.set_last(Shape::Bool);
        result
    }

    probe_number! {
        deserialize_i8, visit_i8, 0, Shape::Int { signed: true };
        deserialize_i16, visit_i16, 0, Shape::Int { signed: true };
        deserialize_i32, visit_i32, 0, Shape::Int { signed: true };
        deserialize_i64, visit_i64, 0, Shape::Int { signed: true };
        deserialize_u8, visit_u8, 0, Shape::Int { signed: false };
        deserialize_u16, visit_u16, 0, Shape::Int { signed: false };
        deserialize_u32, visit_u32, 0, Shape::Int { signed: false };
        deserialize_u64, visit_u64, 0, Shape::Int { signed: false };
        deserialize_f32, visit_f32, 0.0, Shape::Real;
        deserialize_f64, visit_f64, 0.0, Shape::Real;
    }

    fn deserialize_char<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, ProbeError> {
        let result = visitor.visit_char(' ');
        self.set_last(Shape::Char);
        result
    }

    fn deserialize_str<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, ProbeError> {
        let result = visitor.visit_str("");
        self.set_last(Shape::String);
        result
    }

    fn deserialize_string<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, ProbeError> {
        self.deserialize_str(visitor)
    }

    fn deserialize_bytes<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, ProbeError> {
        self.deserialize_str(visitor)
    }

    fn deserialize_byte_buf<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, ProbeError> {
        self.deserialize_str(visitor)
    }

    fn deserialize_option<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, ProbeError> {
        if self.shallow {
            let result = visitor.visit_none();
            self.set_last(Shape::Optional(Box::new(Shape::Unknown)));
            return result;
        }
        let result = visitor.visit_some(self);
        let inner = self.take_last();
        self.set_last(Shape::Optional(Box::new(inner)));
        result
    }

    fn deserialize_unit<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, ProbeError> {
        let result = visitor.visit_unit();
        self.set_last(Shape::Unit);
        result
    }

    fn deserialize_unit_struct<V: Visitor<'de>>(
        self,
        _name: &'static str,
        visitor: V,
    ) -> Result<V::Value, ProbeError> {
        self.deserialize_unit(visitor)
    }

    fn deserialize_newtype_struct<V: Visitor<'de>>(
        self,
        name: &'static str,
        visitor: V,
    ) -> Result<V::Value, ProbeError> {
        if name == TAG_MAGIC {
            let result = visitor.visit_u32(0);
            self.set_last(Shape::Tag);
            return result;
        }
        if name == MICROS_MAGIC {
            let result = visitor.visit_i64(0);
            self.set_last(Shape::Duration);
            return result;
        }
        if let Some(&(_, dim)) = QUANTITY_MAGICS.iter().find(|(magic, _)| *magic == name) {
            let result = visitor.visit_f64(0.0);
            self.set_last(Shape::Quantity(dim));
            return result;
        }
        if name == SITE_OR_PLAIN_MAGIC {
            return self.read_kinds(Via::BySite, visitor);
        }
        // Any other newtype is its inner value.
        visitor.visit_newtype_struct(self)
    }

    fn deserialize_seq<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, ProbeError> {
        let mut elements = Elements {
            probe: self,
            count: usize::from(!self.shallow),
            shapes: Vec::new(),
        };
        let result = visitor.visit_seq(&mut elements);
        let element = elements.shapes.into_iter().next().unwrap_or(Shape::Unknown);
        self.set_last(Shape::List(Box::new(element)));
        result
    }

    fn deserialize_tuple<V: Visitor<'de>>(
        self,
        len: usize,
        visitor: V,
    ) -> Result<V::Value, ProbeError> {
        let mut elements = Elements {
            probe: self,
            count: len,
            shapes: Vec::new(),
        };
        let result = visitor.visit_seq(&mut elements);
        self.set_last(Shape::Tuple(elements.shapes));
        result
    }

    fn deserialize_tuple_struct<V: Visitor<'de>>(
        self,
        _name: &'static str,
        len: usize,
        visitor: V,
    ) -> Result<V::Value, ProbeError> {
        self.deserialize_tuple(len, visitor)
    }

    fn deserialize_map<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, ProbeError> {
        let mut entry = Entry {
            probe: self,
            given: self.shallow,
            key: Shape::Unknown,
            value: Shape::Unknown,
        };
        let result = visitor.visit_map(&mut entry);
        self.set_last(Shape::Map(Box::new(entry.key), Box::new(entry.value)));
        result
    }

    fn deserialize_struct<V: Visitor<'de>>(
        self,
        name: &'static str,
        fields: &'static [&'static str],
        visitor: V,
    ) -> Result<V::Value, ProbeError> {
        let first = fields.first().copied().unwrap_or("");
        match name {
            REF_MAGIC => {
                let result = visitor.visit_u32(0);
                self.set_last(Shape::Ref(first));
                result
            }
            INLINE_MAGIC => {
                let result = visitor.visit_u32(0);
                self.set_last(Shape::Inline(first));
                result
            }
            GATED_MAGIC => {
                let result = visitor.visit_newtype_struct(self);
                let inner = self.take_last();
                self.set_last(Shape::Gated(first, Box::new(inner)));
                result
            }
            LOCATED_MAGIC => {
                let read = RefCell::new(None);
                let result = visitor.visit_map(Entries {
                    probe: self,
                    entries: &[
                        ("file", Fed::Zero),
                        ("start", Fed::Zero),
                        ("end", Fed::Zero),
                        ("value", Fed::Probe),
                    ],
                    at: 0,
                    read: &read,
                });
                self.set_last(read.into_inner().unwrap_or(Shape::Unknown));
                result
            }
            KIND_MAGIC => self.read_kinds(Via::ByKind, visitor),
            SITE_MAGIC => self.read_kinds(Via::BySite, visitor),
            _ => {
                let rust = type_name::<V::Value>();
                self.read_struct(rust.to_owned(), rust, short_name(rust), fields, visitor)
            }
        }
    }

    fn deserialize_enum<V: Visitor<'de>>(
        self,
        _name: &'static str,
        variants: &'static [&'static str],
        visitor: V,
    ) -> Result<V::Value, ProbeError> {
        let rust = type_name::<V::Value>();
        let key = rust.to_owned();
        let shallow = self.enters(&key)?;
        if variants.is_empty() {
            self.set_last(Shape::Named(key));
            return Err(ProbeError::Custom("an enum with no values".to_owned()));
        }
        let explicit = self.run.borrow().config.choices.contains_key(&key);
        let unsettled = !explicit && !self.run.borrow().ex.settled.contains(&key);
        let choice = self
            .choose(&key, variants.len(), shallow)
            .min(variants.len() - 1);
        let enum_name = short_name(rust);
        if !shallow {
            self.record(&key, rust, enum_name.clone(), || {
                NamedShape::Enum(EnumShape {
                    variants: variants
                        .iter()
                        .map(|&name| VariantShape {
                            name,
                            form: VariantForm::Unknown,
                        })
                        .collect(),
                })
            });
        }
        self.run.borrow_mut().stack.push(key.clone());
        let result = visitor.visit_enum(Variant {
            probe: Probe {
                run: self.run,
                shallow,
            },
            key: key.clone(),
            rust: rust.to_owned(),
            enum_name,
            name: variants[choice],
        });
        let mut run = self.run.borrow_mut();
        run.stack.pop();
        if !shallow {
            if result.is_ok() && run.ex.settled.insert(key.clone()) {
                run.ex.preferred.insert(key.clone(), choice);
            }
            if result.is_err() && unsettled {
                run.retry = true;
            }
        }
        run.last = Shape::Named(key);
        result.map_err(|error| match error {
            _ if explicit => ProbeError::Explored,
            other => other,
        })
    }

    fn deserialize_identifier<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, ProbeError> {
        self.deserialize_str(visitor)
    }

    fn deserialize_ignored_any<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, ProbeError> {
        let result = visitor.visit_unit();
        self.set_last(Shape::Any);
        result
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use clash_core::{Real, define_id};
    use serde::Deserialize;

    use super::*;
    use crate::{Def, FromKind, KindValue, Length, Ref, TagSet, Text, Time, by_kind};

    define_id! {
        struct ThingId(u16);
    }

    #[derive(Deserialize)]
    #[allow(dead_code)]
    struct ThingDef {
        mass: Real,
    }

    impl Def for ThingDef {
        type Id = ThingId;
        const TYPE: &'static str = "thing";
    }

    #[derive(Deserialize)]
    #[serde(rename_all = "snake_case")]
    #[allow(dead_code)]
    enum Mode {
        Open,
        Shut,
        Ajar(Real),
        Slide { by: Length },
    }

    /// One thing, or several by name.
    #[allow(dead_code)]
    enum Pick {
        One(Ref<ThingDef>),
        Many(BTreeMap<String, Ref<ThingDef>>),
    }

    impl<'de> FromKind<'de> for Pick {
        fn from_kind<A: MapAccess<'de>>(
            kind: Kind,
            value: KindValue<'_, A>,
        ) -> Result<Pick, A::Error> {
            match kind {
                Kind::String => value.read().map(Pick::One),
                Kind::Object => value.read().map(Pick::Many),
                other => Err(other.unexpected("a thing, or things by name")),
            }
        }
    }

    impl<'de> Deserialize<'de> for Pick {
        fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Pick, D::Error> {
            by_kind(deserializer)
        }
    }

    #[derive(Deserialize)]
    #[allow(dead_code)]
    struct Inner {
        id: String,
        #[serde(default)]
        next: Vec<Inner>,
    }

    #[derive(Deserialize)]
    #[allow(dead_code)]
    struct Sample {
        height: Length,
        #[serde(default)]
        wait: Option<Time>,
        thing: Ref<ThingDef>,
        #[serde(default)]
        tags: TagSet,
        #[serde(default)]
        name: Option<Text>,
        mode: Mode,
        #[serde(default)]
        pick: Option<Pick>,
        #[serde(default)]
        steps: Vec<Inner>,
        #[serde(default)]
        size: Option<(u8, u8)>,
    }

    fn field<'a>(shape: &'a StructShape, name: &str) -> &'a FieldShape {
        shape
            .fields
            .iter()
            .find(|field| field.name == name)
            .unwrap_or_else(|| panic!("no field `{name}`"))
    }

    #[test]
    fn a_struct_is_read_field_by_field_with_what_each_takes() {
        let shapes = describe::<Sample>();
        assert!(!shapes.incomplete);
        let root = shapes.root_struct().expect("a struct");
        let names: Vec<&str> = root.fields.iter().map(|field| field.name).collect();
        assert_eq!(
            names,
            [
                "height", "wait", "thing", "tags", "name", "mode", "pick", "steps", "size"
            ]
        );
        assert_eq!(field(root, "height").shape, Shape::Quantity(Dim::LENGTH));
        assert_eq!(
            field(root, "wait").shape,
            Shape::Optional(Box::new(Shape::Duration))
        );
        assert_eq!(field(root, "thing").shape, Shape::Ref("thing"));
        assert_eq!(field(root, "tags").shape, Shape::List(Box::new(Shape::Tag)));
        assert_eq!(
            field(root, "size").shape,
            Shape::Optional(Box::new(Shape::Tuple(vec![
                Shape::Int { signed: false },
                Shape::Int { signed: false }
            ])))
        );
        // Required are exactly the fields with neither a default nor an
        // `Option`.
        let required: Vec<&str> = root
            .fields
            .iter()
            .filter(|field| field.required)
            .map(|field| field.name)
            .collect();
        assert_eq!(required, ["height", "thing", "mode"]);
    }

    #[test]
    fn every_value_of_an_enum_is_seen_with_its_form() {
        let shapes = describe::<Sample>();
        let Some(NamedShape::Enum(mode)) = shapes
            .named
            .values()
            .find(|named| named.name == "Mode")
            .map(|named| &named.shape)
        else {
            panic!("the enum is described");
        };
        let forms: Vec<(&str, &VariantForm)> = mode
            .variants
            .iter()
            .map(|variant| (variant.name, &variant.form))
            .collect();
        assert_eq!(forms[0], ("open", &VariantForm::Unit));
        assert_eq!(forms[1], ("shut", &VariantForm::Unit));
        assert_eq!(forms[2], ("ajar", &VariantForm::Newtype(Shape::Real)));
        let VariantForm::Struct(key) = forms[3].1 else {
            panic!("a struct value");
        };
        let slide = shapes.struct_shape(key).expect("its fields");
        assert_eq!(field(slide, "by").shape, Shape::Quantity(Dim::LENGTH));
        assert!(field(slide, "by").required);
    }

    #[test]
    fn a_field_of_several_kinds_says_which_it_takes_and_what_it_expects() {
        let shapes = describe::<Sample>();
        let Some(NamedShape::Kinds(pick)) = shapes
            .named
            .values()
            .find(|named| named.name == "Pick")
            .map(|named| &named.shape)
        else {
            panic!("the field type is described");
        };
        assert_eq!(pick.via, Via::ByKind);
        assert_eq!(pick.expected.as_deref(), Some("a thing, or things by name"));
        let kinds: Vec<JsonKind> = pick.accepted.iter().map(|(kind, _)| *kind).collect();
        assert_eq!(kinds, [JsonKind::String, JsonKind::Object]);
        assert_eq!(pick.accepted[0].1, Shape::Ref("thing"));
        // A text is read with its place, and only from a string.
        let Some(NamedShape::Kinds(text)) = shapes
            .named
            .values()
            .find(|named| named.name == "Text")
            .map(|named| &named.shape)
        else {
            panic!("a text is described");
        };
        assert_eq!(text.via, Via::BySite);
        assert_eq!(text.accepted, [(JsonKind::String, Shape::String)]);
    }

    #[test]
    fn a_type_that_holds_itself_is_read_to_an_end() {
        let shapes = describe::<Sample>();
        let inner = shapes
            .named
            .values()
            .find(|named| named.name == "Inner")
            .expect("the inner struct");
        let NamedShape::Struct(inner) = &inner.shape else {
            panic!("a struct");
        };
        assert!(field(inner, "id").required);
        assert_eq!(
            field(inner, "next").shape,
            Shape::List(Box::new(Shape::Named(type_name::<Inner>().to_owned())))
        );
    }

    #[test]
    fn the_same_type_is_described_the_same_every_time() {
        assert_eq!(describe::<Sample>(), describe::<Sample>());
    }
}
