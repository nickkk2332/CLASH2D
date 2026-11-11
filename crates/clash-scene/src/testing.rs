//! A host and an application that are not the game: for tests of the
//! scene, and for the widget gallery, which shows bound widgets against
//! them.
//!
//! [`FakeHost`] stands where the simulation will: it says what `view`
//! and `party` are, answers what bindings read through them, and keeps
//! every command it is handed. [`FakeApp`] stands where the application
//! will: it has a few saves, profiles, mods and scenarios, and keeps
//! every service it is asked for.
//!
//! Both are handles on shared state: a clone kept by a test sees what
//! the scene's copy was handed, and can change the world behind it.
//!
//! # With and without kinds
//!
//! [`FakeHost::new`] comes with [`registry`], which a test gives to the
//! loader so that bindings are checked against it:
//!
//! | Kind | Property | Type | Safe for belief |
//! |------|----------|------|-----------------|
//! | `entity` | `name` | `string` | yes |
//! | `entity` | `health` | `real` | no |
//! | `entity` | `pool(name)` | `pool?`, its own | no |
//! | `entity` | `items` | `list<item>`, its own | no |
//! | `entity` | `seen` | `list<entity>` | yes |
//! | `entity` | `at` | `loc` | yes |
//! | `pool` | `value`, `max`, `frac` | `real` | no |
//! | `item` | `name` | `string` | yes |
//! | `item` | `weight` | `real` | no |
//!
//! and one more kind, `loc`, a cell, with the function `dist(a, b)`,
//! the distance in metres between two entities or cells as the host
//! places them ([`FakeHost::place`]); safe for belief. The built-in
//! `name(x)` is answered with the `name` of an entity or an item.
//!
//! The registry also holds the rest of the simulation's kinds of
//! subject and the kernel's and the world's events, as the simulation
//! registers them (`clash_sim::KERNEL_EVENTS`, `WORLD_EVENTS`), so that
//! `show` definitions compile against it as they do in the game.
//!
//! Its world holds three entities: Mara (1) and Jonas (2), who are the
//! party, Mara being `view`, and a stranger (3) whom Mara has seen.
//!
//! # Who is reading
//!
//! The reader helpers of 05 §4.6 are answered plainly. `name(x)` is the
//! `name` an entity or item was given, or `someone` for an entity with
//! none; `the(x)` and `a(x)` are the same, since the fake names carry
//! their own articles. Once a test says who reads
//! ([`FakeHost::read_as`]), that one is "you": `verb` agrees with it and
//! the pronouns are the second person's. Until then nobody is.
//!
//! # A stand-in for perception
//!
//! Perception is not built, so the fake host answers what playback asks
//! from what a test tells it ([`FakeHost::see`], [`FakeHost::look`]):
//!
//! - An event is **seen** if a cell it is about is seen in the picture
//!   playback has, or a thing it is about has a sprite in such a cell.
//! - Otherwise it is **heard** from that cell if the cell lies within
//!   [`FakeHost::set_hearing`] cells of where `view` is placed (12 by
//!   default), and else not perceived.
//! - What is seen in a cell is what the test said with `see`; a cell it
//!   did not is out of view.
//! - The thing with number `n` has the sprite with number `n`, looking
//!   as the test said with `look`; one it said nothing of was never
//!   perceived.
//!
//! # Where the simulation's own values come from
//!
//! The scene knows cells by their coordinates and perceived things by
//! the number of their sprite. The fake host turns a cell into a `loc`
//! ([`FakeHost::loc`]) and the sprite with number `n` into the entity
//! with number `n`, and back. It offers the candidates a test gives it
//! ([`FakeHost::offer`]) and the decision a test says is waiting
//! ([`FakeHost::ask`]), which the commands `react`, `pass` and `choose`
//! answer.
//!
//! [`FakeHost::plain`] is for a load with no simulation's registry, as
//! the gallery's: there `view` is `any`, and the host gives it as a map
//! with the same shape, `name`, `pool.blood.frac`, `items`.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use clash_core::{EntityId, Real, Rng, Tick};
use clash_expr::{
    DefRef, EventSpec, FunctionId, FunctionSpec, Handle, Host, KindId, KindSpec, PropertyId,
    PropertySpec, Registry, RuntimeWarning, Type, Value,
};
use clash_sim::{CAUSE_SUBJECTS, KERNEL_EVENTS, SimEvent, WORLD_EVENTS};
use clash_world::{CellPos, Location};

use crate::app::{AppCurrent, AppEntry, AppList, AppRequest, AppState};
use crate::control::{COMMAND_CHOOSE, COMMAND_PASS, COMMAND_REACT, ParamAsk, PendingDecision};
use crate::playback::Perceived;
use crate::scene::{SceneHost, UiCommand};
use crate::ui::{AppService, KIND_ENTITY};
use crate::view::{CellContents, EntitySprite, SpriteId, ViewModel};

/// The kind of a thing an entity carries, in the fake registry.
pub const KIND_ITEM: &str = "item";
/// The kind of a reserve an entity has, in the fake registry.
pub const KIND_POOL: &str = "pool";
/// The kind of a cell, in the fake registry.
pub const KIND_LOC: &str = "loc";
/// The kind of a reaction window, in the fake registry: an action that
/// answers one binds it.
pub const KIND_WINDOW: &str = "window";
/// The function that gives the distance between two things.
pub const FN_DIST: &str = "dist";
/// The built-in that gives what the reader calls something.
const FN_NAME: &str = "name";

/// What the fake host calls an entity that was given no name.
pub const UNNAMED: &str = "someone";

/// How far, in cells, the party hears what it does not see unless a
/// test says otherwise.
pub const HEARING_DEFAULT: i32 = 12;

/// How many bits of a fake `loc` handle hold each coordinate.
const LOC_BITS: u32 = 20;
/// Added to a coordinate so that it is stored as a positive number.
const LOC_BIAS: i64 = 1 << (LOC_BITS - 1);

/// A registry with the built-in functions and the kinds and properties
/// tabled in the [module](self) documentation.
pub fn registry() -> Registry {
    let mut registry = Registry::with_builtins();
    let mut kind = |name: &str, doc: &str| {
        registry
            .add_kind(KindSpec::new(name).doc(doc))
            .expect("the fake kinds have distinct names")
    };
    let entity = kind(KIND_ENTITY, "Any entity.");
    let item = kind(KIND_ITEM, "A thing carried.");
    let pool = kind(KIND_POOL, "A reserve: an amount against a most.");
    let loc = kind(KIND_LOC, "A cell of the world.");
    kind(KIND_WINDOW, "An open reaction window.");
    let specs = [
        PropertySpec::new(entity, "name", Type::Str)
            .belief_safe(true)
            .doc("What it is called by those who have met it."),
        PropertySpec::new(entity, "health", Type::Real).doc("How well it is, from 0 to 1."),
        PropertySpec::new(entity, "pool", Type::Subject(pool).nullable())
            .param("name", Type::Str, "The reserve's name.")
            .yields_own(true)
            .doc("One of its reserves."),
        PropertySpec::new(entity, "items", Type::list(Type::Subject(item)))
            .yields_own(true)
            .doc("What it carries."),
        PropertySpec::new(entity, "seen", Type::list(Type::Subject(entity)))
            .belief_safe(true)
            .doc("The entities it has perceived."),
        PropertySpec::new(entity, "at", Type::Subject(loc))
            .belief_safe(true)
            .doc("The cell it is believed to be in."),
        PropertySpec::new(pool, "value", Type::Real).doc("How much there is."),
        PropertySpec::new(pool, "max", Type::Real).doc("The most there can be."),
        PropertySpec::new(pool, "frac", Type::Real).doc("The share there is, from 0 to 1."),
        PropertySpec::new(item, "name", Type::Str)
            .belief_safe(true)
            .doc("What it is called."),
        PropertySpec::new(item, "weight", Type::Real).doc("Its mass in kilograms."),
    ];
    for spec in specs {
        registry
            .add_property(spec)
            .expect("the fake properties have distinct names");
    }
    registry
        .add_function(
            FunctionSpec::new(FN_DIST, Type::Real.nullable())
                .param("a", Type::Any, "An entity or a cell.")
                .param("b", Type::Any, "An entity or a cell.")
                .doc("The distance between two things, in metres. Null if either is nowhere."),
        )
        .expect("the fake registry has no `dist` yet");
    // The rest of the simulation's kinds and its events, entered as the
    // simulation enters them.
    clash_sim::Kinds::ensure(&mut registry).expect("the fake registry has room for the kinds");
    for info in KERNEL_EVENTS.iter().chain(WORLD_EVENTS) {
        let mut spec = EventSpec::new(info.name).doc(info.doc);
        for (name, kind) in info.subjects.iter().chain(CAUSE_SUBJECTS) {
            if let Some(kind) = registry.kind_id(kind) {
                spec = spec.subject(name.trim_end_matches('?'), kind);
            }
        }
        for (name, ty, doc) in info.fields {
            let ty = registry.parse_type(ty).unwrap_or(Type::Any);
            spec = spec.field(*name, ty, *doc);
        }
        registry
            .add_event(spec)
            .expect("the simulation's events have distinct names");
    }
    registry
}

fn real(value: f64) -> Value {
    Value::Real(Real::of(value))
}

/// The kinds of the fake registry.
#[derive(Clone, Copy, Debug)]
struct Kinds {
    entity: KindId,
    item: KindId,
    pool: KindId,
    loc: KindId,
}

#[derive(Debug, Default)]
struct World {
    registry: Option<Registry>,
    kinds: Option<Kinds>,
    /// What each property of each subject is, by the property's name.
    values: BTreeMap<(Handle, String), Value>,
    view: Value,
    party: Value,
    log: Vec<UiCommand>,
    refused: Vec<String>,
    /// Where each entity is.
    places: BTreeMap<Handle, CellPos>,
    /// What may be chosen for each parameter, by its name.
    offers: BTreeMap<String, Vec<Value>>,
    /// The decision that is waiting.
    decision: Option<PendingDecision>,
    /// What the party sees in each cell in view.
    sight: BTreeMap<Location, CellContents>,
    /// How the party perceives each thing it ever has, by number: `None`
    /// for one it does not perceive now.
    looks: BTreeMap<u64, Option<EntitySprite>>,
    /// Who reads text: "you".
    reader: Option<Handle>,
    /// How far the party hears, in cells.
    hearing: i32,
    /// Every moment playback told the host of, in order.
    moments: Vec<Option<Tick>>,
}

/// A cell as the payload of a fake `loc` handle.
fn pack(cell: CellPos) -> u64 {
    let part = |n: i32| ((i64::from(n) + LOC_BIAS) as u64) & ((1 << LOC_BITS) - 1);
    part(cell.x) | (part(cell.y) << LOC_BITS) | (part(cell.z) << (2 * LOC_BITS))
}

/// The cell a fake `loc` handle's payload stands for.
fn unpack(payload: u64) -> CellPos {
    let part = |shift: u32| (((payload >> shift) & ((1 << LOC_BITS) - 1)) as i64 - LOC_BIAS) as i32;
    CellPos::new(part(0), part(LOC_BITS), part(2 * LOC_BITS))
}

fn lock<T>(state: &Mutex<T>) -> MutexGuard<'_, T> {
    // A test that panicked while holding the lock has already failed.
    state.lock().unwrap_or_else(PoisonError::into_inner)
}

/// A host that is not the simulation. See the [module](self)
/// documentation. A clone shares the same world and the same log.
#[derive(Clone, Debug)]
pub struct FakeHost {
    world: Arc<Mutex<World>>,
}

impl Default for FakeHost {
    fn default() -> FakeHost {
        FakeHost::new()
    }
}

impl FakeHost {
    /// A host over [`registry`], with the world tabled in the
    /// [module](self) documentation.
    pub fn new() -> FakeHost {
        let registry = registry();
        let kind = |name: &str| {
            registry
                .kind_id(name)
                .expect("the fake registry has its kinds")
        };
        let kinds = Kinds {
            entity: kind(KIND_ENTITY),
            item: kind(KIND_ITEM),
            pool: kind(KIND_POOL),
            loc: kind(KIND_LOC),
        };
        let host = FakeHost {
            world: Arc::new(Mutex::new(World {
                registry: Some(registry),
                kinds: Some(kinds),
                hearing: HEARING_DEFAULT,
                ..World::default()
            })),
        };
        let (mara, jonas, stranger) = (host.entity(1), host.entity(2), host.entity(3));
        let (lantern, rope) = (host.item(1), host.item(2));
        let blood = Handle::new(kinds.pool, 1);
        host.set(mara, "name", "Mara");
        host.set(mara, "health", real(0.75));
        host.set(mara, "pool", Value::map([("blood", Value::Subject(blood))]));
        host.set(
            mara,
            "items",
            Value::list([Value::Subject(lantern), Value::Subject(rope)]),
        );
        host.set(mara, "seen", Value::list([Value::Subject(stranger)]));
        host.set(blood, "value", real(4.0));
        host.set(blood, "max", real(5.0));
        host.set(blood, "frac", real(0.8));
        host.set(lantern, "name", "Lantern");
        host.set(lantern, "weight", real(1.5));
        host.set(rope, "name", "Coil of rope");
        host.set(rope, "weight", real(2.0));
        host.set(jonas, "name", "Jonas");
        host.set(jonas, "health", real(0.5));
        host.set(jonas, "items", Value::list([]));
        host.set(jonas, "seen", Value::list([]));
        host.set(stranger, "name", "a tall figure in red");
        host.set(stranger, "health", real(0.25));
        host.set(stranger, "items", Value::list([]));
        host.set_view(Value::Subject(mara));
        host.set_party(Value::list([Value::Subject(mara), Value::Subject(jonas)]));
        host
    }

    /// A host for a load with no simulation's registry: `view` is a map
    /// with a `name`, a `pool` of reserves each with `value`, `max` and
    /// `frac`, and `items`, each with a `name` and a `weight`; `party`
    /// is a list of two such maps.
    pub fn plain() -> FakeHost {
        let pool = |value: f64, max: f64| {
            Value::map([
                ("value", real(value)),
                ("max", real(max)),
                ("frac", real(value / max)),
            ])
        };
        let item = |name: &str, weight: f64| {
            Value::map([("name", Value::text(name)), ("weight", real(weight))])
        };
        let mara = Value::map([
            ("name", Value::text("Mara")),
            (
                "pool",
                Value::map([("blood", pool(4.0, 5.0)), ("breath", pool(9.0, 10.0))]),
            ),
            (
                "items",
                Value::list([
                    item("Lantern", 1.5),
                    item("Coil of rope", 2.0),
                    item("Brass key", 0.1),
                ]),
            ),
        ]);
        let jonas = Value::map([
            ("name", Value::text("Jonas")),
            ("pool", Value::map([("blood", pool(2.0, 5.0))])),
            ("items", Value::list([item("Crowbar", 3.0)])),
        ]);
        let host = FakeHost {
            world: Arc::new(Mutex::new(World {
                hearing: HEARING_DEFAULT,
                ..World::default()
            })),
        };
        host.set_view(mara.clone());
        host.set_party(Value::list([mara, jonas]));
        host
    }

    /// The registry the host answers by, if it has one: give a clone to
    /// the loader with `Loader::expr_registry`.
    pub fn registry(&self) -> Option<Registry> {
        lock(&self.world).registry.clone()
    }

    fn handle(&self, pick: fn(&Kinds) -> KindId, number: u64) -> Handle {
        let kind = lock(&self.world)
            .kinds
            .as_ref()
            .map(pick)
            .expect("a plain fake host has no kinds: its values are maps");
        Handle::new(kind, number)
    }

    /// The handle of the entity with this number.
    ///
    /// # Panics
    ///
    /// If the host is [`plain`](FakeHost::plain): a mistake in a test.
    pub fn entity(&self, number: u64) -> Handle {
        self.handle(|kinds| kinds.entity, number)
    }

    /// The handle of the item with this number.
    ///
    /// # Panics
    ///
    /// If the host is [`plain`](FakeHost::plain): a mistake in a test.
    pub fn item(&self, number: u64) -> Handle {
        self.handle(|kinds| kinds.item, number)
    }

    /// Says what a property of a subject is. A property that takes an
    /// argument is given a map from the argument, as text, to the value.
    pub fn set(&self, subject: Handle, property: &str, value: impl Into<Value>) {
        lock(&self.world)
            .values
            .insert((subject, property.to_owned()), value.into());
    }

    /// Says what `view` is.
    pub fn set_view(&self, view: Value) {
        lock(&self.world).view = view;
    }

    /// Says what `party` is: a list.
    pub fn set_party(&self, party: Value) {
        lock(&self.world).party = party;
    }

    /// Makes the host refuse every command of this name from now on.
    pub fn refuse(&self, name: &str) {
        lock(&self.world).refused.push(name.to_owned());
    }

    /// Every command the host accepted, in the order it was handed
    /// them.
    pub fn commands(&self) -> Vec<UiCommand> {
        lock(&self.world).log.clone()
    }

    /// The value of a cell: a `loc`. Coordinates within half a million
    /// cells of the origin are told apart.
    ///
    /// # Panics
    ///
    /// If the host is [`plain`](FakeHost::plain): a mistake in a test.
    pub fn loc(&self, x: i32, y: i32, z: i32) -> Value {
        Value::Subject(self.handle(|kinds| kinds.loc, pack(CellPos::new(x, y, z))))
    }

    /// Says where an entity is, for `dist` and for its `at`.
    pub fn place(&self, entity: Handle, x: i32, y: i32, z: i32) {
        let at = self.loc(x, y, z);
        self.set(entity, "at", at);
        lock(&self.world)
            .places
            .insert(entity, CellPos::new(x, y, z));
    }

    /// Says what the acting character believes could be chosen for the
    /// parameter with this name, of any action.
    pub fn offer(&self, param: &str, values: Vec<Value>) {
        lock(&self.world).offers.insert(param.to_owned(), values);
    }

    /// Says which decision is waiting, or that none is. A `react`, a
    /// `pass` or a `choose` that the host accepts answers it.
    pub fn ask(&self, decision: Option<PendingDecision>) {
        lock(&self.world).decision = decision;
    }

    /// Says what the party sees in a cell now, or with `None` that the
    /// cell is out of view: what playback is told when an event that was
    /// seen changes the cell.
    pub fn see(&self, at: Location, contents: Option<CellContents>) {
        let mut world = lock(&self.world);
        match contents {
            Some(contents) => world.sight.insert(at, contents),
            None => world.sight.remove(&at),
        };
    }

    /// Says how the party perceives the thing with this number now, or
    /// with `None` that it knows of it and does not perceive it now. Its
    /// sprite has the same number.
    pub fn look(&self, number: u64, look: Option<EntitySprite>) {
        lock(&self.world).looks.insert(number, look);
    }

    /// Makes text read as it does to this entity: it is "you". `None`:
    /// nobody is.
    pub fn read_as(&self, reader: Option<Handle>) {
        lock(&self.world).reader = reader;
    }

    /// Says how far, in cells, the party hears what it does not see,
    /// from where `view` is placed.
    pub fn set_hearing(&self, cells: i32) {
        lock(&self.world).hearing = cells;
    }

    /// Every moment playback has told the host of, oldest first
    /// ([`SceneHost::as_of`]).
    pub fn moments(&self) -> Vec<Option<Tick>> {
        lock(&self.world).moments.clone()
    }
}

/// The verb in the third person singular: `hits`, `crashes`, `carries`.
fn third_person(word: &str) -> String {
    let before_last = word.chars().rev().nth(1);
    if word.ends_with('y') && !before_last.is_some_and(|ch| "aeiou".contains(ch)) {
        format!("{}ies", &word[..word.len() - 1])
    } else if ["s", "sh", "ch", "x", "z", "o"]
        .iter()
        .any(|end| word.ends_with(end))
    {
        format!("{word}es")
    } else {
        format!("{word}s")
    }
}

impl World {
    /// Where a value is: an entity's place, or the cell a `loc` is.
    fn cell_of(&self, value: &Value) -> Option<CellPos> {
        let handle = value.as_subject()?;
        match self.kinds {
            Some(kinds) if handle.kind == kinds.loc => Some(unpack(handle.payload)),
            _ => self.places.get(&handle).copied(),
        }
    }
}

impl SceneHost for FakeHost {
    type Command = UiCommand;
    type Refusal = String;

    fn submit(&mut self, command: UiCommand) -> Result<(), String> {
        let mut world = lock(&self.world);
        if world.refused.contains(&command.name) {
            return Err(format!("the fake host refuses `{}`", command.name));
        }
        if [COMMAND_REACT, COMMAND_PASS, COMMAND_CHOOSE].contains(&command.name.as_str()) {
            world.decision = None;
        }
        world.log.push(command);
        Ok(())
    }

    fn pending_decision(&self) -> Option<PendingDecision> {
        lock(&self.world).decision.clone()
    }

    fn cell_value(&mut self, at: Location) -> Value {
        match lock(&self.world).kinds {
            Some(kinds) => Value::Subject(Handle::new(kinds.loc, pack(at.cell))),
            None => Value::map([
                ("x", Value::Int(i64::from(at.cell.x))),
                ("y", Value::Int(i64::from(at.cell.y))),
                ("z", Value::Int(i64::from(at.cell.z))),
            ]),
        }
    }

    fn cell_of(&mut self, value: &Value) -> Option<Location> {
        let world = lock(&self.world);
        let handle = value.as_subject()?;
        // The fake world has one grid.
        (handle.kind == world.kinds?.loc)
            .then(|| Location::new(clash_world::GridId::new(0), unpack(handle.payload)))
    }

    fn sprite_value(&mut self, sprite: SpriteId) -> Value {
        // The sprite with a number is the entity with that number.
        match lock(&self.world).kinds {
            Some(kinds) => Value::Subject(Handle::new(kinds.entity, sprite.raw())),
            None => Value::Null,
        }
    }

    fn sprite_of(&mut self, value: &Value) -> Option<SpriteId> {
        let entity = lock(&self.world).kinds?.entity;
        let handle = value.as_subject()?;
        (handle.kind == entity).then(|| SpriteId::new(handle.payload))
    }

    fn candidates(&mut self, ask: &ParamAsk<'_>) -> Option<Vec<Value>> {
        lock(&self.world).offers.get(ask.param).cloned()
    }

    fn perceived(&mut self, event: &SimEvent, picture: &ViewModel) -> Perceived {
        let subjects = event.subjects();
        let sprite_of = |entity: EntityId| {
            picture
                .sprite(SpriteId::new(entity.to_bits()))
                .map(|sprite| sprite.loc)
        };
        // The cells the event is about, its `here` first, and the cells
        // the things it is about are drawn in.
        let mut places: Vec<Location> = event
            .subject("here")
            .and_then(|s| s.loc())
            .into_iter()
            .collect();
        places.extend(subjects.iter().filter_map(|(_, subject)| subject.loc()));
        places.extend(
            subjects
                .iter()
                .filter_map(|(_, subject)| subject.entity().and_then(sprite_of)),
        );
        if places
            .iter()
            .any(|at| picture.cell(*at).is_some_and(|cell| cell.seen()))
        {
            return Perceived::Seen;
        }
        let world = lock(&self.world);
        let Some(from) = places.first().copied() else {
            return Perceived::Not;
        };
        let listener = world.cell_of(&world.view);
        let hearing = i64::from(world.hearing);
        match listener {
            Some(at) if at.z == from.cell.z => {
                let (dx, dy) = (i64::from(at.x - from.cell.x), i64::from(at.y - from.cell.y));
                if dx * dx + dy * dy <= hearing * hearing {
                    Perceived::Heard { from }
                } else {
                    Perceived::Not
                }
            }
            _ => Perceived::Not,
        }
    }

    fn seen_cell(&mut self, at: Location) -> Option<CellContents> {
        lock(&self.world).sight.get(&at).copied()
    }

    fn seen_sprite(&mut self, entity: EntityId) -> Option<(SpriteId, Option<EntitySprite>)> {
        let number = entity.to_bits();
        let look = *lock(&self.world).looks.get(&number)?;
        Some((SpriteId::new(number), look))
    }

    fn as_of(&mut self, moment: Option<Tick>) {
        lock(&self.world).moments.push(moment);
    }

    fn view(&self) -> Value {
        lock(&self.world).view.clone()
    }

    fn party(&self) -> Value {
        lock(&self.world).party.clone()
    }

    fn reader(&mut self) -> &mut dyn Host {
        self
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

    fn property(&mut self, property: PropertyId, subject: Handle, args: &[Value]) -> Value {
        let world = lock(&self.world);
        let Some(name) = world
            .registry
            .as_ref()
            .and_then(|registry| registry.property(property))
            .map(|property| property.spec.name.clone())
        else {
            return Value::Null;
        };
        let value = world
            .values
            .get(&(subject, name))
            .cloned()
            .unwrap_or_default();
        // A property with an argument is kept as a map by the argument.
        match args.first().and_then(Value::as_str) {
            Some(key) => value
                .as_map()
                .and_then(|map| map.get(key))
                .cloned()
                .unwrap_or_default(),
            None => value,
        }
    }

    fn variable(&mut self, _subject: Handle, _name: &str) -> Value {
        Value::Null
    }

    fn call(&mut self, function: FunctionId, args: &[Value]) -> Value {
        let world = lock(&self.world);
        // A plain host has the built-ins and nothing more: `name` is the
        // one function the scene asks it for.
        let name = match &world.registry {
            Some(registry) => match registry.function(function) {
                Some(function) => function.spec.name.clone(),
                None => return Value::Null,
            },
            None => FN_NAME.to_owned(),
        };
        // What the reader calls something, and whether it is the reader.
        let called = |value: &Value| -> (Value, bool) {
            if world.reader.is_some() && value.as_subject() == world.reader {
                return (Value::text("you"), true);
            }
            let named = match value {
                Value::Subject(handle) => {
                    let named = world.values.get(&(*handle, FN_NAME.to_owned())).cloned();
                    let entity = world.kinds.is_some_and(|kinds| handle.kind == kinds.entity);
                    match named {
                        Some(named) => named,
                        None if entity => Value::text(UNNAMED),
                        None => Value::Null,
                    }
                }
                other => other
                    .as_map()
                    .and_then(|map| map.get(FN_NAME))
                    .cloned()
                    .unwrap_or_default(),
            };
            (named, false)
        };
        let person = |value: &Value, second: &str, third: &str| {
            Value::text(if called(value).1 { second } else { third })
        };
        match (name.as_str(), args) {
            (FN_NAME | "the" | "a", [value]) => called(value).0,
            ("they", [value]) => person(value, "you", "they"),
            ("them", [value]) => person(value, "you", "them"),
            ("their", [value]) => person(value, "your", "their"),
            ("themself", [value]) => person(value, "yourself", "themself"),
            ("verb", [value, word]) => {
                let word = word.as_str().unwrap_or_default();
                if called(value).1 {
                    Value::text(word)
                } else {
                    Value::text(third_person(word))
                }
            }
            (FN_DIST, [a, b]) => match (world.cell_of(a), world.cell_of(b)) {
                (Some(a), Some(b)) => {
                    let squared: f64 = [a.x - b.x, a.y - b.y, a.z - b.z]
                        .iter()
                        .map(|d| f64::from(*d) * f64::from(*d))
                        .sum();
                    real(squared.sqrt())
                }
                _ => Value::Null,
            },
            _ => Value::Null,
        }
    }

    fn rng(&mut self) -> Option<&mut Rng> {
        None
    }

    fn warn(&mut self, _warning: RuntimeWarning) {}

    fn def_name(&self, _id: DefRef) -> Option<&str> {
        None
    }
}

#[derive(Debug, Default)]
struct Shelf {
    saves: Vec<AppEntry>,
    profiles: Vec<AppEntry>,
    mods: Vec<AppEntry>,
    scenarios: Vec<AppEntry>,
    profile: Option<AppEntry>,
    scenario: Option<AppEntry>,
    can_save: bool,
    can_load: bool,
    requests: Vec<AppRequest>,
    refused: Vec<AppService>,
}

/// An application that is not one. See the [module](self)
/// documentation. A clone shares the same state and the same list of
/// requests.
#[derive(Clone, Debug)]
pub struct FakeApp {
    shelf: Arc<Mutex<Shelf>>,
}

impl Default for FakeApp {
    fn default() -> FakeApp {
        FakeApp::new()
    }
}

fn entry(id: &str, name: &str, description: &str, state: &str) -> AppEntry {
    AppEntry {
        id: id.to_owned(),
        name: name.to_owned(),
        description: description.to_owned(),
        state: state.to_owned(),
    }
}

impl FakeApp {
    /// An application with two saves, two profiles, three mods and two
    /// scenarios, in which saving and loading are allowed.
    pub fn new() -> FakeApp {
        let profiles = vec![
            entry("default", "CLASH2D", "The bundled game.", "current"),
            entry("ashlands_game", "The Ashlands", "A total conversion.", ""),
        ];
        let scenarios = vec![
            entry("core:walk", "A walk", "Nothing happens.", "current"),
            entry("core:siege", "The siege", "Hold the gate.", ""),
        ];
        FakeApp {
            shelf: Arc::new(Mutex::new(Shelf {
                saves: vec![
                    entry("autumn", "Autumn", "Day 12, the docks.", ""),
                    entry("first", "First try", "Day 2, the gate.", ""),
                ],
                profile: profiles.first().cloned(),
                scenario: scenarios.first().cloned(),
                profiles,
                mods: vec![
                    entry("core", "Core", "What the engine needs.", "enabled"),
                    entry("ui", "Interface", "The bundled screens.", "enabled"),
                    entry("ashlands", "The Ashlands", "Ash and embers.", "disabled"),
                ],
                scenarios,
                can_save: true,
                can_load: true,
                requests: Vec::new(),
                refused: Vec::new(),
            })),
        }
    }

    /// Every service the application carried out, in order.
    pub fn requests(&self) -> Vec<AppRequest> {
        lock(&self.shelf).requests.clone()
    }

    /// Says whether the saving policy allows saving and loading now.
    pub fn allow(&self, save: bool, load: bool) {
        let mut shelf = lock(&self.shelf);
        shelf.can_save = save;
        shelf.can_load = load;
    }

    /// Makes the application refuse a service from now on.
    pub fn refuse(&self, service: AppService) {
        lock(&self.shelf).refused.push(service);
    }
}

impl AppState for FakeApp {
    fn entries(&self, list: AppList) -> Vec<AppEntry> {
        let shelf = lock(&self.shelf);
        match list {
            AppList::Saves => shelf.saves.clone(),
            AppList::Profiles => shelf.profiles.clone(),
            AppList::Mods => shelf.mods.clone(),
            AppList::Scenarios => shelf.scenarios.clone(),
        }
    }

    fn current(&self, which: AppCurrent) -> Option<AppEntry> {
        let shelf = lock(&self.shelf);
        match which {
            AppCurrent::Profile => shelf.profile.clone(),
            AppCurrent::Scenario => shelf.scenario.clone(),
        }
    }

    fn can_save(&self) -> bool {
        lock(&self.shelf).can_save
    }

    fn can_load(&self) -> bool {
        lock(&self.shelf).can_load
    }

    fn service(&mut self, request: &AppRequest) -> Result<(), String> {
        let mut shelf = lock(&self.shelf);
        if shelf.refused.contains(&request.service) {
            return Err(format!(
                "the fake application refuses `{}`",
                request.service.name()
            ));
        }
        match request.service {
            AppService::Save => {
                if !shelf.can_save {
                    return Err("the scenario does not allow saving now".to_owned());
                }
                let name = request.text("name").unwrap_or("Quick save").to_owned();
                let id = name.to_lowercase().replace(' ', "_");
                shelf.saves.insert(0, entry(&id, &name, "Just now.", ""));
            }
            AppService::Load => {
                if !shelf.can_load {
                    return Err("the scenario does not allow loading now".to_owned());
                }
                let wanted = request.text("save").unwrap_or("");
                if !shelf.saves.iter().any(|save| save.id == wanted) {
                    return Err(format!("there is no save `{wanted}`"));
                }
            }
            AppService::DeleteSave => {
                let wanted = request.text("save").unwrap_or("").to_owned();
                let before = shelf.saves.len();
                shelf.saves.retain(|save| save.id != wanted);
                if shelf.saves.len() == before {
                    return Err(format!("there is no save `{wanted}`"));
                }
            }
            _ => {}
        }
        shelf.requests.push(request.clone());
        Ok(())
    }
}
