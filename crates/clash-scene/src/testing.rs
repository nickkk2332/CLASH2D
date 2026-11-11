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
//! | `pool` | `value`, `max`, `frac` | `real` | no |
//! | `item` | `name` | `string` | yes |
//! | `item` | `weight` | `real` | no |
//!
//! Its world holds three entities: Mara (1) and Jonas (2), who are the
//! party, Mara being `view`, and a stranger (3) whom Mara has seen.
//!
//! [`FakeHost::plain`] is for a load with no simulation's registry, as
//! the gallery's: there `view` is `any`, and the host gives it as a map
//! with the same shape, `name`, `pool.blood.frac`, `items`.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use clash_core::{Real, Rng};
use clash_expr::{
    DefRef, FunctionId, Handle, Host, KindId, KindSpec, PropertyId, PropertySpec, Registry,
    RuntimeWarning, Type, Value,
};

use crate::app::{AppCurrent, AppEntry, AppList, AppRequest, AppState};
use crate::scene::{SceneHost, UiCommand};
use crate::ui::{AppService, KIND_ENTITY};

/// The kind of a thing an entity carries, in the fake registry.
pub const KIND_ITEM: &str = "item";
/// The kind of a reserve an entity has, in the fake registry.
pub const KIND_POOL: &str = "pool";

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
        };
        let host = FakeHost {
            world: Arc::new(Mutex::new(World {
                registry: Some(registry),
                kinds: Some(kinds),
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
            world: Arc::new(Mutex::new(World::default())),
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
}

impl SceneHost for FakeHost {
    type Command = UiCommand;
    type Refusal = String;

    fn submit(&mut self, command: UiCommand) -> Result<(), String> {
        let mut world = lock(&self.world);
        if world.refused.contains(&command.name) {
            return Err(format!("the fake host refuses `{}`", command.name));
        }
        world.log.push(command);
        Ok(())
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

    fn call(&mut self, _function: FunctionId, _args: &[Value]) -> Value {
        Value::Null
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
