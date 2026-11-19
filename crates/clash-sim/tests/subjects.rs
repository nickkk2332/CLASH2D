//! Tests for K4: subjects, properties and variable storage (05 §5, §6).
//!
//! What the package is done by: every property the simulation offers is
//! registered, documented and read here; chains across kinds work;
//! setters go through the engine's mutation paths, shown by the events
//! they emit; a watched variable raises `var_changed` with old and new
//! values.

mod common;

use std::cell::RefCell;
use std::collections::BTreeSet;
use std::sync::Arc;

use clash_core::{EntityId, Real, Tick, VarMap};
use clash_data::{Defs, Runner};
use clash_expr::{
    Compiled, Context, Handle, Host, PlaceRoot, Registry, Type, Value, compile, compile_place,
};
use clash_sim::component::{Identity, Shape};
use clash_sim::{
    AdvanceLimit, Assign, Bound, Control, EventBody, FieldValue, Place, Sim, SimEvent, Subject,
    engine_functions, engine_properties, engine_registry,
};
use clash_time::Timeline;
use clash_world::{
    AreaId, Cause, CellPos, Cosmos, GridId, Layer, Location, RealmId, SiteDefId, SiteId, SiteSpec,
    TractPos, WorldDefs,
};
use common::{GRID, at, creature, item, names, on, spawn};

/// What the tests declare beside the world content of `mods/core`.
const DECLARED: &str = r#"[
  { "type": "area_kind", "id": "stopped_time", "clock_rate": 0, "priority": 100, "tags": ["eerie"] },
  { "type": "event", "id": "ritual_completed",
    "subjects": { "caster": "entity", "here": "loc" },
    "fields": { "power": "real", "school": "string" },
    "doc": "A ritual circle finished charging." },
  { "type": "variable", "id": "kills", "scope": "entity", "value": "int", "default": 0,
    "watch": true, "doc": "How many this one has killed." },
  { "type": "variable", "id": "nick", "scope": "entity", "value": "string",
    "doc": "What friends call it." },
  { "type": "variable", "id": "tally", "scope": "entity", "value": "map<int>",
    "doc": "Counts by what was counted." },
  { "type": "variable", "id": "seen", "scope": "entity", "value": "list<entity>", "default": [],
    "doc": "Who it has met." },
  { "type": "variable", "id": "sealed", "scope": "cell", "value": "bool", "default": false,
    "watch": true, "doc": "Whether the cell has been sealed." },
  { "type": "variable", "id": "heat", "scope": "area", "value": "real", "default": 1.5,
    "doc": "How hot the area runs." },
  { "type": "variable", "id": "alarm_raised", "scope": "site", "value": "bool", "default": false,
    "watch": true, "doc": "Set once the alarm has gone off." },
  { "type": "variable", "id": "visits", "scope": "tract", "value": "int", "default": 0,
    "doc": "How often the tract was visited." },
  { "type": "variable", "id": "season", "scope": "realm", "value": "string", "default": "spring",
    "watch": true, "doc": "The season of the realm." },
  { "type": "variable", "id": "day_of_reckoning", "scope": "global", "value": "int",
    "watch": true, "doc": "The day it all ends." },
  { "type": "function", "id": "ruin_at", "params": { "who": "entity" }, "returns": "int",
    "value": "$who.loc.tract.devastation",
    "doc": "How devastated the tract someone stands in is." },
  { "type": "function", "id": "season_of", "params": { "who": "entity" }, "returns": "string",
    "value": { "expr": "$who.loc.realm.var.season" },
    "doc": "The season where someone is." },
  { "type": "function", "id": "lucky", "params": {}, "returns": "int",
    "value": "rand_int(1, 6)", "doc": "A roll of a die." }
]"#;

fn defs() -> Arc<Defs> {
    common::try_defs(DECLARED).unwrap_or_else(|error| panic!("{error}"))
}

fn sim() -> Sim {
    common::sim_over(defs(), 9)
}

fn entity(id: EntityId) -> Subject {
    Subject::Entity(id)
}

fn real(value: f64) -> Value {
    Value::Real(Real::of(value))
}

/// Compiles an expression against the simulation's registry, in a scope
/// that binds `self` (an entity) and `here` (a cell), reading truth.
fn compiled(sim: &Sim, source: &str) -> Compiled {
    let outcome = compile(source, sim.exprs().registry(), &scope(sim), &Type::Any);
    match outcome.value {
        Some(code) if !outcome.has_errors() => code,
        _ => panic!("`{source}` does not compile: {:?}", outcome.diagnostics),
    }
}

fn scope(sim: &Sim) -> Context {
    let kinds = sim.exprs().kinds();
    Context::new()
        .subject("self", Type::Subject(kinds.entity))
        .subject("here", Type::Subject(kinds.loc))
        .namespace("core")
}

/// What the scope of [`compiled`] binds.
fn bound(sim: &Sim, me: EntityId, here: Location) -> Bound {
    Bound::new()
        .subject(sim.subject_value(entity(me)))
        .subject(sim.subject_value(Subject::Loc(here)))
}

fn eval(sim: &Sim, source: &str, me: EntityId, here: Location) -> Value {
    sim.eval(&compiled(sim, source), bound(sim, me, here))
}

/// Reads properties by name and remembers which were read, so that a
/// test can tell that none was left out.
struct Probe<'a> {
    sim: &'a Sim,
    read: RefCell<BTreeSet<(String, String)>>,
}

impl<'a> Probe<'a> {
    fn new(sim: &'a Sim) -> Probe<'a> {
        Probe {
            sim,
            read: RefCell::new(BTreeSet::new()),
        }
    }

    fn get(&self, subject: Subject, name: &str) -> Value {
        self.call(subject, name, &[])
    }

    fn call(&self, subject: Subject, name: &str, args: &[Value]) -> Value {
        assert!(
            self.sim.exprs().property_id(subject.kind(), name).is_some(),
            "`{}.{name}` is not registered",
            subject.kind()
        );
        self.read
            .borrow_mut()
            .insert((subject.kind().to_owned(), name.to_owned()));
        self.sim.property_named(subject, name, args)
    }

    fn value(&self, subject: Subject) -> Value {
        self.sim.subject_value(subject)
    }
}

// ---- the registry ---------------------------------------------------------

#[test]
fn everything_the_simulation_offers_is_registered_and_documented() {
    let registry = engine_registry();
    for kind in [
        "entity", "loc", "element", "area", "site", "tract", "realm", "global",
    ] {
        let id = registry.kind_id(kind).unwrap_or_else(|| panic!("{kind}"));
        assert!(!registry.kind(id).unwrap().spec.doc.is_empty(), "{kind}");
    }
    let properties = engine_properties();
    assert!(properties.len() > 90, "{}", properties.len());
    // The events of combat and of living things are entered by their
    // own registry steps, which run once the bodies' step has registered
    // the kinds their fields name; a property may say one of them
    // changes it.
    let mut events = engine_registry();
    clash_sim::combat::events::register(&mut events);
    clash_sim::life::events::register(&mut events);
    for property in &properties {
        let kind = registry.kind_id(property.kind).unwrap();
        let id = registry
            .property_id(kind, property.name)
            .unwrap_or_else(|| panic!("{}.{} is not registered", property.kind, property.name));
        let spec = &registry.property(id).unwrap().spec;
        let label = format!("{}.{}", property.kind, property.name);
        assert!(spec.doc.len() > 10, "{label} has no documentation");
        assert_eq!(spec.settable, property.set.is_some(), "{label}");
        assert_eq!(spec.belief_safe, property.belief_safe, "{label}");
        assert_eq!(spec.params.len(), property.args.len(), "{label}");
        for param in &spec.params {
            assert!(!param.doc.is_empty(), "{label}({})", param.name);
        }
        // Every event a property says changes it is an event.
        for event in &spec.changed_by {
            assert!(events.event_id(event).is_some(), "{label}: {event}");
        }
    }
    for function in engine_functions() {
        let id = registry
            .function_id(function.name)
            .unwrap_or_else(|| panic!("{}", function.name));
        let spec = &registry.function(id).unwrap().spec;
        assert!(spec.doc.len() > 10, "{}", function.name);
        assert!(!spec.group.is_empty(), "{}", function.name);
        for param in &spec.params {
            assert!(!param.doc.is_empty(), "{}({})", function.name, param.name);
        }
    }
    // The functions of 05 §4.7 that the simulation answers.
    for name in [
        "dist",
        "dir",
        "offset",
        "los",
        "now",
        "local_now",
        "setting",
        "has_def",
        "exists",
        "light",
    ] {
        assert!(registry.function_id(name).is_some(), "{name}");
    }

    // Signatures read as the reference prints them.
    let signature = |kind: &str, name: &str| {
        let id = registry
            .property_id(registry.kind_id(kind).unwrap(), name)
            .unwrap();
        registry.property_signature(registry.property(id).unwrap())
    };
    assert_eq!(signature("entity", "loc"), "entity.loc -> loc?");
    assert_eq!(
        signature("loc", "in_area"),
        "loc.in_area(kind: id<area_kind>) -> bool"
    );
    assert_eq!(
        signature("element", "material"),
        "element.material -> id<material>?"
    );
    assert_eq!(
        signature("tract", "devastation"),
        "tract.devastation -> int?"
    );
    let dist = registry
        .function(registry.function_id("dist").unwrap())
        .unwrap();
    assert_eq!(registry.signature(dist), "dist(a: any, b: any) -> real?");

    // Registering again changes nothing: the step runs on every load.
    let mut again = registry.clone();
    clash_sim::register_engine(&mut again);
    assert_eq!(again.properties().count(), registry.properties().count());
    assert_eq!(again.functions().count(), registry.functions().count());
    assert_eq!(again.events().count(), registry.events().count());
}

#[test]
fn only_what_is_marked_is_safe_for_belief() {
    let registry = engine_registry();
    let safe: BTreeSet<String> = registry
        .properties()
        .filter(|property| property.spec.belief_safe)
        .map(|property| {
            let kind = &registry.kind(property.spec.kind).unwrap().spec.name;
            format!("{kind}.{}", property.spec.name)
        })
        .collect();
    let expected: BTreeSet<String> = [
        "area.kind",
        "element.kind",
        "element.layer",
        "element.loc",
        "entity.kind",
        "global.kind",
        "global.now",
        "loc.grid",
        "loc.kind",
        "loc.realm",
        // The hour where one stands, as the sky shows it (D-872).
        "loc.time_of_day",
        "loc.tract",
        "loc.x",
        "loc.y",
        "loc.z",
        "realm.day",
        "realm.day_length",
        "realm.day_part",
        "realm.kind",
        "realm.season",
        "realm.time_of_day",
        "site.kind",
        "tract.kind",
        "tract.loc",
        "tract.realm",
        "tract.x",
        "tract.y",
    ]
    .into_iter()
    .map(str::to_owned)
    .collect();
    assert_eq!(safe, expected);
    for (name, is_safe) in [
        ("dist", true),
        ("dir", true),
        ("offset", true),
        ("now", true),
        ("setting", true),
        ("has_def", true),
        ("los", false),
        ("light", false),
        ("local_now", false),
        ("exists", false),
    ] {
        let function = registry
            .function(registry.function_id(name).unwrap())
            .unwrap();
        assert_eq!(function.spec.belief_safe, is_safe, "{name}");
    }
}

#[test]
fn a_mind_may_read_itself_and_nothing_else_that_is_truth() {
    let sim = sim();
    let kinds = sim.exprs().kinds();
    let mind = Context::new()
        .own_subject("self", Type::Subject(kinds.entity))
        .subject("target", Type::Subject(kinds.entity))
        .namespace("core")
        .belief();
    let registry = sim.exprs().registry();
    let check = |source: &str| {
        let outcome = compile(source, registry, &mind, &Type::Any);
        outcome
            .errors()
            .map(|error| error.code.as_str().to_owned())
            .collect::<Vec<_>>()
    };
    // Its own state, and arithmetic on what it read of itself.
    assert_eq!(check("self.loc"), [] as [&str; 0]);
    assert_eq!(check("self.mass + self.var.kills"), [] as [&str; 0]);
    assert_eq!(check("self.loc.x + self.loc.tract.x"), [] as [&str; 0]);
    assert_eq!(check("dist(self, target)"), [] as [&str; 0]);
    // Truth about another, about the world it stands in, and about
    // what cannot be seen from the handle alone.
    assert_eq!(check("target.loc"), ["X402"]);
    assert_eq!(check("target.var.kills"), ["X402"]);
    assert_eq!(check("target.control"), ["X402"]);
    assert_eq!(check("self.loc.block"), ["X402"]);
    assert_eq!(check("self.loc.standable"), ["X402"]);
    assert_eq!(check("self.loc.occupants"), ["X402"]);
    assert_eq!(check("self.loc.tract.devastation"), ["X402"]);
    assert_eq!(check("los(self, target)"), ["X402"]);
    assert_eq!(check("exists(target)"), ["X402"]);
    // The same reads are fine where truth is read.
    let world = scope(&sim);
    for source in ["self.loc.block.material", "los(self, here)"] {
        assert!(!compile(source, registry, &world, &Type::Any).has_errors());
    }
}

// ---- entities --------------------------------------------------------------

#[test]
fn an_entity_answers_for_itself() {
    let mut sim = sim();
    let guard = spawn(&mut sim, creature().with_tag("guard"), on(3, 4));
    let pack = spawn(&mut sim, item(), Place::In(guard));
    let coin = spawn(&mut sim, item().with_tag("shiny"), Place::In(pack));
    let ghost = sim.create_entity(creature(), Cause::DEBUG).unwrap();
    sim.components_mut().identity.insert(guard, {
        let mut identity = Identity::new(None);
        identity.rename(Some("Oda".to_owned()));
        identity.set_description(Some("A tired guard.".to_owned()));
        identity
    });
    let shape = |kg: f64| Shape {
        mass: clash_data::Mass::new(Real::of(kg)),
        volume: clash_data::Volume::new(Real::of(0.07)),
        height: clash_data::Length::new(Real::of(1.8)),
        ..Shape::default()
    };
    sim.set_shape(guard, shape(70.0), Cause::DEBUG).unwrap();
    sim.set_shape(coin, shape(0.5), Cause::DEBUG).unwrap();
    sim.set_time_rate(guard, Some(Real::of(2.0)), Cause::DEBUG)
        .unwrap();
    sim.advance(AdvanceLimit::Until(Tick(3_000_000)));

    let probe = Probe::new(&sim);
    let (g, c) = (entity(guard), entity(coin));
    assert_eq!(probe.get(g, "kind"), Value::from("entity"));
    assert_eq!(probe.get(g, "loc"), probe.value(Subject::Loc(at(3, 4))));
    // What is carried is where its carrier is.
    assert_eq!(probe.get(c, "loc"), probe.value(Subject::Loc(at(3, 4))));
    assert_eq!(probe.get(c, "container"), probe.value(entity(pack)));
    assert_eq!(probe.get(g, "container"), Value::Null);
    assert_eq!(probe.get(c, "outermost"), probe.value(g));
    assert_eq!(
        probe.get(g, "contents"),
        Value::list([probe.value(entity(pack))])
    );
    assert_eq!(probe.get(c, "in_world"), Value::Bool(true));
    assert_eq!(probe.get(entity(ghost), "in_world"), Value::Bool(false));
    assert_eq!(probe.get(entity(ghost), "loc"), Value::Null);
    assert_eq!(probe.get(g, "altitude"), real(0.0));
    assert_eq!(probe.get(c, "altitude"), Value::Null);
    assert_eq!(probe.get(g, "occupant_kind"), Value::from("creature"));
    assert_eq!(probe.get(c, "occupant_kind"), Value::from("item"));
    assert_eq!(probe.get(g, "tags"), Value::list([Value::from("guard")]));
    assert_eq!(
        probe.call(g, "has_tag", &[Value::from("guard")]),
        Value::Bool(true)
    );
    assert_eq!(
        probe.call(g, "has_tag", &[Value::from("shiny")]),
        Value::Bool(false)
    );
    assert_eq!(probe.get(g, "def"), Value::Null);
    assert_eq!(probe.get(g, "name"), Value::from("Oda"));
    assert_eq!(probe.get(c, "name"), Value::Null);
    assert_eq!(probe.get(g, "description"), Value::from("A tired guard."));
    assert_eq!(
        probe.call(g, "has_component", &[Value::from("physical")]),
        Value::Bool(true)
    );
    assert_eq!(
        probe.call(entity(pack), "has_component", &[Value::from("physical")]),
        Value::Bool(false)
    );
    assert_eq!(probe.get(g, "mass"), real(70.5));
    assert_eq!(probe.get(g, "own_mass"), real(70.0));
    assert_eq!(probe.get(g, "carried_mass"), real(0.5));
    assert_eq!(probe.get(entity(pack), "own_mass"), Value::Null);
    assert_eq!(probe.get(g, "volume"), real(0.07));
    assert_eq!(probe.get(g, "height"), real(1.8));
    assert_eq!(probe.get(g, "control"), Value::from("none"));
    assert_eq!(probe.get(g, "time_rate"), real(2.0));
    assert_eq!(probe.get(c, "time_rate"), Value::Null);
    // Three seconds passed outside: six for the guard and what it holds.
    assert_eq!(probe.get(g, "clock_rate"), real(2.0));
    assert_eq!(probe.get(c, "clock_rate"), real(2.0));
    assert_eq!(probe.get(g, "local_time"), real(6.0));
    assert_eq!(probe.get(g, "own_time"), real(6.0));
    assert_eq!(probe.get(entity(ghost), "clock_rate"), Value::Null);
    // Movement's, of something with no way of moving: no `movement`
    // definition is loaded here (`tests/movement.rs` has the rest).
    assert_eq!(probe.get(g, "posture"), Value::Null);
    assert_eq!(probe.get(g, "speed"), Value::Null);
    let east = [Value::from("east")];
    assert_eq!(probe.call(g, "can_step", &east), Value::Bool(false));
    assert!(probe.call(g, "step_fault", &east).as_str().is_some());
    assert_eq!(probe.call(g, "step_time", &east), Value::Null);
    let next_door = [probe.value(Subject::Loc(at(4, 4)))];
    assert_eq!(probe.call(g, "can_reach", &next_door), Value::Bool(false));
    assert_eq!(probe.call(g, "path_cost", &next_door), Value::Null);
    assert_eq!(probe.get(g, "path_left"), Value::Null);
    assert_eq!(probe.get(g, "path_next"), Value::Null);
    assert_eq!(probe.get(g, "path_stale"), Value::Bool(false));

    // Perception's beliefs, of what has never been perceived: tests of
    // perception read them as they change.
    let other = probe.value(c);
    for name in [
        "believed_loc",
        "last_seen",
        "last_seen_at",
        "confidence",
        "awareness",
        "seen_how",
    ] {
        assert_eq!(
            probe.call(g, name, std::slice::from_ref(&other)),
            Value::Null,
            "{name}"
        );
    }
    for name in ["knows", "perceives"] {
        assert_eq!(
            probe.call(g, name, std::slice::from_ref(&other)),
            Value::Bool(false),
            "{name}"
        );
    }
    assert_eq!(probe.get(g, "facing"), Value::Null);
    assert_eq!(
        probe.call(g, "remembers", std::slice::from_ref(&other)),
        Value::Bool(false)
    );
    assert_eq!(
        probe.call(
            g,
            "could_perceive",
            &[other.clone(), Value::from("no_such_kind")]
        ),
        Value::Null
    );
    assert_eq!(
        probe.call(g, "knows_element", &[other, Value::from("no_such_layer")]),
        Value::Null
    );
    // A timer it does not have (`tests/timers.rs` sets them).
    assert_eq!(probe.call(g, "timer", &[Value::from("none")]), Value::Null);

    // Everything registered for an entity was read.
    let read = probe.read.into_inner();
    // Those of items are read in `tests/items.rs`, those of devices in
    // `tests/devices.rs`, those of interactions in
    // `tests/interactions.rs`, those of traits in `tests/traits.rs`,
    // those of defences in `tests/reactions.rs`, those of abilities in
    // `tests/abilities.rs`, those of minds in `tests/ai.rs`, those of
    // ranged combat in `tests/ranged.rs`, those of scenarios in
    // `tests/scenario.rs`, those of grips and takedowns in
    // `tests/grips.rs`, those of recipes and substances in
    // `tests/making.rs`, those of the social layer in
    // `tests/social.rs`, those of what a character could do in
    // `tests/offer.rs`, and those of residents in `tests/agents.rs`.
    let of_items: Vec<&str> = clash_sim::item::properties()
        .iter()
        .chain(clash_sim::recipe::properties().iter())
        .chain(clash_sim::substance::properties().iter())
        .chain(clash_sim::device::properties().iter())
        .chain(clash_sim::interaction::properties().iter())
        .chain(clash_sim::traits::properties().iter())
        .chain(clash_sim::combat::defence::properties().iter())
        .chain(clash_sim::ability::properties().iter())
        .chain(clash_sim::ai::properties().iter())
        .chain(clash_sim::combat::ranged::properties().iter())
        .chain(clash_sim::scenario::journal::properties().iter())
        .chain(clash_sim::combat::grip::properties().iter())
        .chain(clash_sim::combat::takedown::properties().iter())
        .chain(clash_sim::social::properties().iter())
        .chain(clash_sim::offer::properties().iter())
        .chain(clash_sim::agent::properties().iter())
        // `entity.vital` is read awake, unconscious and dead by the test
        // script `scenarios/ai/sleeper.scn`.
        .chain(clash_sim::mind::properties().iter())
        .map(|property| property.name)
        .collect();
    for property in engine_properties() {
        if property.kind == "entity" && !of_items.contains(&property.name) {
            assert!(
                read.contains(&("entity".to_owned(), property.name.to_owned())),
                "entity.{} is not tested",
                property.name
            );
        }
    }

    // A handle outlives what it names and then reads as nothing.
    let handle = sim.subject_value(entity(coin));
    let loc = compiled(&sim, "self.loc");
    assert!(
        !sim.eval(&loc, Bound::new().subject(handle.clone()))
            .is_null()
    );
    assert_eq!(
        eval(&sim, "exists(self)", coin, at(0, 0)),
        Value::Bool(true)
    );
    sim.remove_entity(coin, Cause::DEBUG).unwrap();
    assert_eq!(
        sim.eval(&loc, Bound::new().subject(handle.clone())),
        Value::Null
    );
    assert_eq!(sim.property_named(entity(coin), "mass", &[]), Value::Null);
    let exists = compiled(&sim, "exists(self)");
    assert_eq!(
        sim.eval(&exists, Bound::new().subject(handle.clone())),
        Value::Bool(false)
    );
    // The slot's next tenant is not found through the old handle.
    let tenant = spawn(&mut sim, item(), on(1, 1));
    assert_eq!(tenant.slot(), coin.slot());
    assert_eq!(sim.eval(&loc, Bound::new().subject(handle)), Value::Null);
}

// ---- the world -------------------------------------------------------------

/// A site over the two western tracts of the test ground.
fn make_site(sim: &mut Sim) -> SiteId {
    let realm = common::realm(sim);
    sim.world(|ctx| {
        ctx.cosmos.create_site(SiteSpec {
            def: SiteDefId::new(0),
            name: "The Vault".to_owned(),
            realm,
            tracts: vec![TractPos::new(0, 0), TractPos::new(0, 1)],
            anchor: CellPos::new(5, 5, 0),
            vars: VarMap::new(),
            tags: vec!["bank".to_owned()],
        })
    })
    .unwrap()
}

/// An area of the kind `core:stopped_time` over cells (2, 2) to (6, 6).
fn make_area(sim: &mut Sim, site: Option<SiteId>) -> AreaId {
    let kind = sim
        .truth()
        .cosmos()
        .defs()
        .area_kinds()
        .lookup("core:stopped_time")
        .unwrap();
    let shape = clash_world::AreaShape::Box(clash_world::CellBox::new(
        CellPos::new(2, 2, 0),
        CellPos::new(6, 6, 0),
    ));
    let mut spec = clash_world::AreaSpec::new(GRID, shape);
    spec.kind = Some(kind);
    spec.site = site;
    sim.world(|ctx| ctx.create_area(spec, Cause::DEBUG))
        .unwrap()
}

#[test]
fn the_world_answers_for_cells_elements_areas_sites_tracts_and_realms() {
    let mut sim = sim();
    let realm = common::realm(&sim);
    let site = make_site(&mut sim);
    let area = make_area(&mut sim, Some(site));
    let inside = spawn(&mut sim, creature(), on(4, 4));
    let outside = spawn(&mut sim, creature(), on(20, 20));
    let wall = common::wall(&sim);
    let wall_at = at(10, 10);
    sim.world(|ctx| ctx.edit(Cause::DEBUG).set_block(wall_at, wall))
        .unwrap();
    sim.advance(AdvanceLimit::Until(Tick(7_200_000_000)));

    let probe = Probe::new(&sim);
    let cell = |x, y| Subject::Loc(at(x, y));

    // ---- a cell
    let here = cell(4, 4);
    assert_eq!(probe.get(here, "kind"), Value::from("loc"));
    assert_eq!(probe.get(here, "x"), Value::Int(4));
    assert_eq!(probe.get(here, "y"), Value::Int(4));
    assert_eq!(probe.get(here, "z"), Value::Int(0));
    assert_eq!(probe.get(here, "grid"), Value::Int(0));
    assert_eq!(probe.get(here, "realm"), probe.value(Subject::Realm(realm)));
    assert_eq!(
        probe.get(cell(40, 4), "tract"),
        probe.value(Subject::Tract(realm, TractPos::new(1, 0)))
    );
    assert_eq!(probe.get(here, "site"), probe.value(Subject::Site(site)));
    assert_eq!(probe.get(cell(40, 4), "site"), Value::Null);
    assert_eq!(
        probe.get(here, "areas"),
        Value::list([probe.value(Subject::Area(area))])
    );
    assert_eq!(probe.get(cell(20, 20), "areas"), Value::list([]));
    let kind_id = sim
        .exprs()
        .registry()
        .def_ref(
            sim.exprs().def_types().area_kind.unwrap(),
            "core:stopped_time",
        )
        .unwrap();
    assert_eq!(
        probe.call(here, "in_area", &[Value::Id(kind_id)]),
        Value::Bool(true)
    );
    assert_eq!(
        probe.call(cell(20, 20), "in_area", &[Value::Id(kind_id)]),
        Value::Bool(false)
    );
    assert_eq!(
        probe.call(here, "in_area_tagged", &[Value::from("eerie")]),
        Value::Bool(true)
    );
    assert_eq!(
        probe.call(here, "in_area_tagged", &[Value::from("cosy")]),
        Value::Bool(false)
    );
    let floor = Subject::Element(at(4, 4), Layer::Floor);
    let block = Subject::Element(wall_at, Layer::Block);
    assert_eq!(probe.get(here, "floor"), probe.value(floor));
    assert_eq!(probe.get(here, "block"), Value::Null);
    assert_eq!(probe.get(here, "fixture"), Value::Null);
    assert_eq!(
        probe.get(Subject::Loc(wall_at), "block"),
        probe.value(block)
    );
    assert_eq!(probe.get(here, "is_loaded"), Value::Bool(true));
    assert_eq!(probe.get(cell(999, 4), "is_loaded"), Value::Bool(false));
    assert_eq!(probe.get(here, "standable"), Value::Bool(true));
    assert_eq!(probe.get(cell(999, 4), "standable"), Value::Null);
    assert_eq!(probe.get(here, "is_open"), Value::Bool(false));
    assert_eq!(probe.get(here, "obstacle_height"), real(0.0));
    assert_eq!(
        probe.get(Subject::Loc(wall_at), "obstacle_height"),
        real(3.0)
    );
    assert_eq!(probe.get(here, "under_open_sky"), Value::Bool(true));
    assert_eq!(
        probe.call(here, "has_flag", &[Value::from("has_floor")]),
        Value::Bool(true)
    );
    assert_eq!(
        probe.call(
            Subject::Loc(wall_at),
            "has_flag",
            &[Value::from("walk_blocked")]
        ),
        Value::Bool(true)
    );
    assert_eq!(
        probe.call(here, "has_flag", &[Value::from("walk_blocked")]),
        Value::Bool(false)
    );
    assert_eq!(
        probe.call(here, "has_flag", &[Value::from("nonsense")]),
        Value::Null
    );
    assert_eq!(probe.get(here, "tags"), Value::list([Value::from("plant")]));
    assert_eq!(
        probe.call(here, "has_tag", &[Value::from("plant")]),
        Value::Bool(true)
    );
    // Two hours after a midnight start it is dark; the light is read.
    let light = probe.get(cell(20, 20), "light");
    assert!(light.as_number().is_some(), "{light}");
    assert_eq!(probe.get(here, "gravity"), real(9.81));
    let air = probe.get(here, "atmosphere");
    assert_eq!(
        sim.exprs().registry().def_name(air.as_id().unwrap()),
        Some("core:air")
    );
    // The area stops time; outside it two hours have passed.
    let outside_time = probe.get(cell(20, 20), "time_of_day");
    assert!(outside_time.as_number().is_some(), "{outside_time}");
    assert_eq!(
        probe.get(here, "occupants"),
        Value::list([probe.value(entity(inside))])
    );
    assert_eq!(probe.get(here, "clock_rate"), real(0.0));
    assert_eq!(probe.get(cell(20, 20), "clock_rate"), real(1.0));
    assert_eq!(probe.get(here, "local_time"), real(0.0));
    assert_eq!(probe.get(cell(20, 20), "local_time"), real(7200.0));

    // ---- an element
    assert_eq!(probe.get(block, "kind"), Value::from("element"));
    assert_eq!(probe.get(block, "layer"), Value::from("block"));
    assert_eq!(probe.get(block, "loc"), probe.value(Subject::Loc(wall_at)));
    let def = probe.get(block, "def").as_id().unwrap();
    assert_eq!(sim.exprs().registry().def_name(def), Some("core:wall"));
    assert_eq!(probe.get(block, "def_id"), Value::from("core:wall"));
    let material = probe.get(block, "material").as_id().unwrap();
    assert_eq!(
        sim.exprs().registry().def_name(material),
        Some("core:concrete")
    );
    assert_eq!(probe.get(block, "state"), Value::Int(0));
    assert_eq!(probe.get(block, "state_name"), Value::Null);
    assert_eq!(probe.get(block, "damage"), Value::Int(0));
    assert_eq!(probe.get(block, "height"), real(3.0));
    assert_eq!(
        probe.call(block, "has_flag", &[Value::from("opaque")]),
        Value::Bool(true)
    );
    assert_eq!(
        probe.call(block, "has_flag", &[Value::from("liquid")]),
        Value::Bool(false)
    );
    assert_eq!(
        probe.get(floor, "tags"),
        Value::list([Value::from("plant")])
    );
    assert_eq!(
        probe.call(floor, "has_tag", &[Value::from("plant")]),
        Value::Bool(true)
    );
    // An element that is not there reads as nothing.
    let nothing = Subject::Element(at(4, 4), Layer::Fixture);
    assert_eq!(probe.get(nothing, "def"), Value::Null);
    assert_eq!(probe.get(nothing, "tags"), Value::Null);

    // ---- an area
    let a = Subject::Area(area);
    assert_eq!(probe.get(a, "kind"), Value::from("area"));
    assert_eq!(probe.get(a, "def"), Value::Id(kind_id));
    assert_eq!(probe.get(a, "tags"), Value::list([Value::from("eerie")]));
    assert_eq!(
        probe.call(a, "has_tag", &[Value::from("eerie")]),
        Value::Bool(true)
    );
    assert_eq!(probe.get(a, "site"), probe.value(Subject::Site(site)));
    assert_eq!(probe.get(a, "priority"), Value::Int(100));
    assert_eq!(probe.get(a, "grid"), Value::Int(0));
    assert_eq!(probe.get(a, "loc"), probe.value(cell(4, 4)));
    assert_eq!(
        probe.call(a, "contains", &[probe.value(entity(inside))]),
        Value::Bool(true)
    );
    assert_eq!(
        probe.call(a, "contains", &[probe.value(entity(outside))]),
        Value::Bool(false)
    );
    assert_eq!(
        probe.get(a, "occupants"),
        Value::list([probe.value(entity(inside))])
    );
    assert_eq!(probe.get(a, "breached"), Value::Bool(false));
    assert_eq!(probe.get(a, "clock_rate"), real(0.0));
    assert_eq!(probe.get(a, "local_time"), real(0.0));
    assert_eq!(
        probe.get(Subject::Area(AreaId::new(77)), "priority"),
        Value::Null
    );

    // ---- a site
    let s = Subject::Site(site);
    assert_eq!(probe.get(s, "kind"), Value::from("site"));
    assert_eq!(probe.get(s, "name"), Value::from("The Vault"));
    assert_eq!(probe.get(s, "tags"), Value::list([Value::from("bank")]));
    assert_eq!(
        probe.call(s, "has_tag", &[Value::from("bank")]),
        Value::Bool(true)
    );
    assert_eq!(probe.get(s, "realm"), probe.value(Subject::Realm(realm)));
    assert_eq!(probe.get(s, "loc"), probe.value(cell(5, 5)));
    assert_eq!(
        probe.get(s, "tracts"),
        Value::list([
            probe.value(Subject::Tract(realm, TractPos::new(0, 0))),
            probe.value(Subject::Tract(realm, TractPos::new(0, 1))),
        ])
    );

    // ---- a tract
    let t = Subject::Tract(realm, TractPos::new(1, 0));
    assert_eq!(probe.get(t, "kind"), Value::from("tract"));
    assert_eq!(probe.get(t, "realm"), probe.value(Subject::Realm(realm)));
    assert_eq!(probe.get(t, "x"), Value::Int(1));
    assert_eq!(probe.get(t, "y"), Value::Int(0));
    assert_eq!(probe.get(t, "loc"), probe.value(cell(32, 0)));
    assert_eq!(probe.get(t, "loaded"), Value::Bool(true));
    assert_eq!(
        probe.get(Subject::Tract(realm, TractPos::new(9, 0)), "loaded"),
        Value::Bool(false)
    );
    // The test ground was put there by hand: no ticket keeps it and no
    // region was surveyed.
    assert_eq!(probe.get(t, "activity"), Value::Null);
    assert_eq!(probe.get(t, "surveyed"), Value::Bool(false));
    assert_eq!(probe.get(t, "devastation"), Value::Null);
    assert_eq!(
        probe.get(Subject::Tract(realm, TractPos::new(0, 1)), "site"),
        probe.value(s)
    );
    assert_eq!(probe.get(t, "site"), Value::Null);

    // ---- a realm, and the simulation
    let r = Subject::Realm(realm);
    assert_eq!(probe.get(r, "kind"), Value::from("realm"));
    let def = probe.get(r, "def").as_id().unwrap();
    assert_eq!(sim.exprs().registry().def_name(def), Some("core:testland"));
    assert_eq!(probe.get(r, "grid"), Value::Int(0));
    assert_eq!(probe.get(r, "clock_rate"), real(1.0));
    assert_eq!(probe.get(r, "local_time"), real(7200.0));
    assert_eq!(
        probe.get(Subject::Realm(RealmId::new(9)), "grid"),
        Value::Null
    );
    assert_eq!(probe.get(Subject::Global, "kind"), Value::from("global"));
    assert_eq!(probe.get(Subject::Global, "now"), real(7200.0));

    // ---- what a cell holds, and timers (`tests/timers.rs` sets them)
    assert_eq!(
        probe.call(here, "field", &[Value::from("core:fire")]),
        Value::Int(0)
    );
    assert_eq!(
        probe.call(here, "field", &[Value::from("no_such_field")]),
        Value::Null
    );
    for owner in [here, a, s, r, Subject::Global] {
        assert_eq!(
            probe.call(owner, "timer", &[Value::from("none")]),
            Value::Null
        );
    }

    // Every property that is not an entity's was read, but those of
    // devices, which `tests/devices.rs` reads, furnishing's, which
    // `tests/furnish.rs` reads, and a realm's calendar, which
    // `tests/agents.rs` reads.
    let read = probe.read.into_inner();
    let elsewhere: Vec<(&str, &str)> = clash_sim::device::properties()
        .iter()
        .chain(clash_sim::interaction::properties().iter())
        .chain(clash_sim::furnish::properties().iter())
        .chain(clash_sim::agent::properties().iter())
        .map(|property| (property.kind, property.name))
        .collect();
    for property in engine_properties() {
        if property.kind != "entity" && !elsewhere.contains(&(property.kind, property.name)) {
            assert!(
                read.contains(&(property.kind.to_owned(), property.name.to_owned())),
                "{}.{} is not tested",
                property.kind,
                property.name
            );
        }
    }
}

#[test]
fn functions_of_space_time_and_data_answer_from_the_simulation() {
    let mut sim = sim();
    let me = spawn(&mut sim, creature(), on(3, 4));
    let other = spawn(&mut sim, creature(), on(6, 8));
    let wall = common::wall(&sim);
    for y in 0..12 {
        sim.world(|ctx| ctx.edit(Cause::DEBUG).set_block(at(5, y), wall))
            .unwrap();
    }
    sim.set_setting("core:difficulty", 3);
    sim.advance(AdvanceLimit::Until(Tick(1_500_000)));
    let here = at(6, 8);
    let ask = |source: &str| eval(&sim, source, me, here);

    assert_eq!(ask("dist(self, here)"), real(5.0));
    assert_eq!(ask("dist(self.loc, self)"), real(0.0));
    assert_eq!(
        ask("dir(self, here)"),
        Value::list([Value::Int(1), Value::Int(1), Value::Int(0)])
    );
    assert_eq!(
        ask("offset(self.loc, 3, 4)"),
        sim.subject_value(Subject::Loc(here))
    );
    assert_eq!(ask("offset(here, 0, 0, 1).z"), Value::Int(1));
    assert_eq!(ask("offset(here, 0, -100)"), Value::Null, "beyond the edge");
    // A wall stands between them; nothing stands between a cell and
    // the one beside it.
    assert_eq!(ask("los(self, here)"), Value::Bool(false));
    assert_eq!(ask("los(self, offset(self.loc, 1, 0))"), Value::Bool(true));
    assert_eq!(ask("light(here) == here.light"), Value::Bool(true));
    assert_eq!(ask("now()"), real(1.5));
    assert_eq!(ask("local_now(self)"), real(1.5));
    assert_eq!(ask("local_now(here.realm)"), real(1.5));
    assert_eq!(ask("setting('core:difficulty')"), Value::Int(3));
    assert_eq!(ask("setting('core:nothing')"), Value::Null);
    assert_eq!(ask("has_def('block', 'core:wall')"), Value::Bool(true));
    assert_eq!(ask("has_def('block', 'core:cheese')"), Value::Bool(false));
    assert_eq!(ask("exists(self) and exists(here)"), Value::Bool(true));
    // The reader's helpers answer plainly until there is a reader.
    assert_eq!(ask("name('door')"), Value::from("door"));
    assert_eq!(ask("the('door')"), Value::from("the door"));
    assert_eq!(ask("count(2, 'door')"), Value::from("2 doors"));
    let _ = other;

    // A random function has nothing to draw from where only reading is
    // allowed, and draws from the rule stream where it may.
    let roll = compiled(&sim, "rand_int(1, 6)");
    let mut host = sim.host(bound(&sim, me, here));
    assert_eq!(
        Runner::new().on(sim.defs(), &mut host).value(&roll),
        Value::Null
    );
    assert_eq!(host.take_warnings().len(), 1);
    let before = sim.hashes().rng;
    let defs = Arc::clone(sim.defs());
    let bound = bound(&sim, me, here);
    let rolled = sim.with_random(bound, |host| Runner::new().on(&defs, host).value(&roll));
    assert!((1..=6).contains(&rolled.as_int().unwrap()));
    assert_ne!(sim.hashes().rng, before, "the draw is state");
}

// ---- chains ----------------------------------------------------------------

/// A simulation over a generated city with someone in it.
fn city() -> (Sim, EntityId, RealmId) {
    let defs = defs();
    let world = WorldDefs::from_defs(&defs).unwrap_or_else(|d| panic!("{}", d.render()));
    let mut timeline = Timeline::new();
    let mut cosmos = Cosmos::new(Arc::clone(&world));
    let def = world.realms().lookup("core:test_city").unwrap();
    let realm = cosmos.create_realm(&mut timeline, def, 11).unwrap();
    let mut sim = Sim::from_parts(defs, cosmos, timeline, 5);
    let walker = sim.create_entity(creature(), Cause::DEBUG).unwrap();
    let start = Location::new(GridId::new(0), CellPos::new(641, 641, 0));
    sim.start_session(walker, start, Cause::DEBUG).unwrap();
    sim.drain_events();
    (sim, walker, realm)
}

#[test]
fn chains_cross_kinds_in_expressions_and_in_functions_mods_define() {
    let (mut sim, walker, realm) = city();
    let here = sim.truth().location(walker).unwrap();
    let tract = Subject::Tract(realm, here.cell.tract());

    // entity -> loc -> tract -> a property of the tract's record.
    assert_eq!(
        eval(&sim, "self.loc.tract.devastation", walker, here),
        Value::Int(0)
    );
    assert_eq!(
        eval(&sim, "self.loc.tract.activity", walker, here),
        Value::from("hot")
    );
    assert_eq!(
        eval(&sim, "self.loc.tract.surveyed", walker, here),
        Value::Bool(true)
    );
    assert_eq!(
        eval(
            &sim,
            "self.loc.tract.realm.def == 'test_city'",
            walker,
            here
        ),
        Value::Bool(true)
    );
    // entity -> loc -> element -> an id compared with a text checked at
    // load: a misspelling is an error then.
    assert_eq!(
        eval(
            &sim,
            "here.floor.material == here.floor.material",
            walker,
            here
        ),
        Value::Bool(true)
    );
    let misspelt = compile(
        "here.floor.material == 'granit'",
        sim.exprs().registry(),
        &scope(&sim),
        &Type::Any,
    );
    assert!(misspelt.has_errors());
    assert_eq!(
        misspelt.diagnostics[0].suggestion.as_deref(),
        Some("did you mean 'granite'?")
    );
    // Variables at the end of a chain, with their declared defaults.
    assert_eq!(
        eval(&sim, "self.loc.realm.var.season", walker, here),
        Value::from("spring")
    );
    assert_eq!(
        eval(&sim, "self.loc.tract.var.visits + 1", walker, here),
        Value::Int(1)
    );
    sim.set_var(tract, "core:visits", Value::Int(4), Cause::DEBUG)
        .unwrap();
    assert_eq!(
        eval(&sim, "self.loc.tract.var.visits + 1", walker, here),
        Value::Int(5)
    );

    // The same chains inside functions that a mod defines, compiled when
    // the mod loaded and run through the data layer's evaluator.
    let me = sim.subject_value(entity(walker));
    let mut host = sim.host(Bound::new());
    let mut runner = Runner::new();
    let mut evaluator = runner.on(sim.defs(), &mut host);
    assert_eq!(
        evaluator.call_named("core:ruin_at", core::slice::from_ref(&me)),
        Value::Int(0)
    );
    assert_eq!(
        evaluator.call_named("core:season_of", core::slice::from_ref(&me)),
        Value::from("spring")
    );
    // What a function draws at random is worked out from its body.
    let functions = sim.defs().functions();
    let lucky = functions
        .iter()
        .find(|(_, f)| f.name == "core:lucky")
        .unwrap()
        .1;
    assert!(lucky.random);
    let ruin = functions
        .iter()
        .find(|(_, f)| f.name == "core:ruin_at")
        .unwrap()
        .1;
    assert!(!ruin.random);
    assert!(
        !ruin.belief_safe,
        "it reads truth about whoever it is given"
    );
}

// ---- setters ---------------------------------------------------------------

fn set(sim: &mut Sim, subject: Subject, name: &str, value: Value) -> Vec<SimEvent> {
    sim.pump();
    sim.drain_events();
    sim.set_property_named(subject, name, &value, Cause::DEBUG)
        .unwrap_or_else(|reason| panic!("{}.{name}: {reason}", subject.kind()));
    sim.pump();
    sim.drain_events()
}

#[test]
fn setters_go_through_the_engines_mutation_paths() {
    let mut sim = sim();
    let realm = common::realm(&sim);
    let site = make_site(&mut sim);
    let area = make_area(&mut sim, None);
    let mover = spawn(&mut sim, creature(), on(20, 20));
    let chest = spawn(&mut sim, item(), on(21, 20));
    sim.set_shape(mover, Shape::default(), Cause::DEBUG)
        .unwrap();
    let m = entity(mover);

    // loc: a relocation, with the area's events before it.
    let to = sim.subject_value(Subject::Loc(at(4, 4)));
    let told = set(&mut sim, m, "loc", to);
    assert_eq!(names(&told), ["area_entered", "entity_relocated"]);
    assert_eq!(sim.truth().place(mover), Some(on(4, 4)));
    assert_eq!(
        sim.truth()
            .cosmos()
            .occupant(mover)
            .map(|record| record.loc),
        Some(at(4, 4))
    );
    // container: into the chest and out again, to where the chest is.
    let into = sim.subject_value(entity(chest));
    let told = set(&mut sim, m, "container", into);
    assert_eq!(names(&told), ["area_exited", "entity_relocated"]);
    assert_eq!(sim.truth().place(mover), Some(Place::In(chest)));
    let told = set(&mut sim, m, "container", Value::Null);
    assert_eq!(names(&told), ["entity_relocated"]);
    assert_eq!(sim.truth().place(mover), Some(on(21, 20)));
    // loc = null: out of the world.
    let told = set(&mut sim, m, "loc", Value::Null);
    assert_eq!(names(&told), ["entity_relocated"]);
    assert_eq!(sim.truth().place(mover), Some(Place::Stored));
    let back = sim.subject_value(Subject::Loc(at(20, 20)));
    set(&mut sim, m, "loc", back);

    // control: an entity with nothing to do that comes under direct
    // control is asked what to do (03 §5.2).
    let told = set(&mut sim, m, "control", Value::from("direct"));
    assert_eq!(names(&told), ["control_changed", "decision_requested"]);
    assert_eq!(
        sim.truth().entity(mover).unwrap().control(),
        Control::Direct
    );
    // time_rate: a personal clock, and its removal.
    let told = set(&mut sim, m, "time_rate", real(3.0));
    assert_eq!(names(&told), ["clock_rate_changed"]);
    assert!(sim.truth().entity(mover).unwrap().clock().is_some());
    let told = set(&mut sim, m, "time_rate", Value::Null);
    assert_eq!(names(&told), ["clock_rate_changed"]);
    assert!(sim.truth().entity(mover).unwrap().clock().is_none());
    // own_mass reaches the world's record.
    set(&mut sim, m, "own_mass", Value::Int(80));
    let record = sim.truth().cosmos().occupant(mover).unwrap();
    assert_eq!(record.occupant.mass, Real::of(80.0));
    // An area's clock and a realm's.
    let told = set(&mut sim, Subject::Area(area), "clock_rate", real(0.5));
    assert_eq!(names(&told), ["clock_rate_changed"]);
    let told = set(&mut sim, Subject::Realm(realm), "clock_rate", real(2.0));
    assert_eq!(names(&told), ["clock_rate_changed"]);
    assert_eq!(
        sim.property_named(Subject::Realm(realm), "clock_rate", &[]),
        real(2.0)
    );
    // A site's name.
    set(
        &mut sim,
        Subject::Site(site),
        "name",
        Value::from("The Bank"),
    );
    assert_eq!(sim.truth().cosmos().site(site).unwrap().name, "The Bank");

    // Every settable property was set.
    // Those of items are set in `tests/items.rs`.
    let of_items: Vec<&str> = clash_sim::item::properties()
        .iter()
        .map(|property| property.name)
        .collect();
    let settable: Vec<String> = engine_properties()
        .iter()
        .filter(|property| property.set.is_some() && !of_items.contains(&property.name))
        .map(|property| format!("{}.{}", property.kind, property.name))
        .collect();
    assert_eq!(
        settable,
        [
            "entity.loc",
            "entity.container",
            "entity.own_mass",
            "entity.control",
            "entity.time_rate",
            "area.clock_rate",
            "site.name",
            "realm.clock_rate",
        ]
    );

    // Refusals carry reasons, and change nothing.
    let hash = sim.state_hash();
    let refuse = |sim: &mut Sim, subject: Subject, name: &str, value: Value| {
        sim.set_property_named(subject, name, &value, Cause::DEBUG)
            .unwrap_err()
            .code
    };
    assert_eq!(
        refuse(&mut sim, m, "control", Value::from("puppet")),
        "write.bad_value"
    );
    assert_eq!(refuse(&mut sim, m, "loc", Value::Int(3)), "write.bad_value");
    let far = sim.subject_value(Subject::Loc(at(999, 999)));
    assert_eq!(refuse(&mut sim, m, "loc", far), "world.not_loaded");
    let itself = sim.subject_value(m);
    assert_eq!(
        refuse(&mut sim, m, "container", itself),
        "relocate.inside_itself"
    );
    assert_eq!(
        refuse(&mut sim, m, "time_rate", real(-1.0)),
        "clock.bad_rate"
    );
    assert_eq!(
        refuse(&mut sim, m, "own_mass", real(-1.0)),
        "physical.bad_measure"
    );
    assert_eq!(refuse(&mut sim, m, "mass", real(1.0)), "write.not_settable");
    assert_eq!(
        refuse(&mut sim, m, "nonsense", real(1.0)),
        "write.not_settable"
    );
    assert_eq!(
        refuse(&mut sim, Subject::Realm(realm), "clock_rate", Value::Null),
        "clock.bad_rate"
    );
    let gone = entity(EntityId::new(500, 0));
    assert_eq!(
        refuse(&mut sim, gone, "control", Value::from("none")),
        "subject.gone"
    );
    assert_eq!(sim.state_hash(), hash);
}

// ---- variables -------------------------------------------------------------

/// The `var_changed` events among some, as (scope, name, old, new).
fn changes(events: &[SimEvent]) -> Vec<(String, String, FieldValue, FieldValue)> {
    events
        .iter()
        .filter_map(|event| match &event.body {
            EventBody::VarChanged {
                owner,
                name,
                old,
                new,
            } => Some((
                owner.kind().to_owned(),
                name.clone(),
                old.clone(),
                new.clone(),
            )),
            _ => None,
        })
        .collect()
}

#[test]
fn variables_live_on_every_scope_with_defaults_types_and_watches() {
    let (mut sim, walker, realm) = city();
    let here = sim.truth().location(walker).unwrap();
    let site = sim
        .world(|ctx| {
            ctx.cosmos.create_site(SiteSpec {
                def: SiteDefId::new(0),
                name: "Depot".to_owned(),
                realm,
                tracts: vec![TractPos::new(3, 3)],
                anchor: CellPos::new(100, 100, 0),
                vars: VarMap::new(),
                tags: Vec::new(),
            })
        })
        .unwrap();
    let shape = clash_world::AreaShape::Box(clash_world::CellBox::new(here.cell, here.cell));
    let area = sim
        .world(|ctx| ctx.create_area(clash_world::AreaSpec::new(here.grid, shape), Cause::DEBUG))
        .unwrap();
    let tract = here.cell.tract();
    let scopes = [
        (entity(walker), "core:kills", Value::Int(0), Value::Int(3)),
        (
            Subject::Loc(here),
            "core:sealed",
            Value::Bool(false),
            Value::Bool(true),
        ),
        (Subject::Area(area), "core:heat", real(1.5), real(40.0)),
        (
            Subject::Site(site),
            "core:alarm_raised",
            Value::Bool(false),
            Value::Bool(true),
        ),
        (
            Subject::Tract(realm, tract),
            "core:visits",
            Value::Int(0),
            Value::Int(7),
        ),
        (
            Subject::Realm(realm),
            "core:season",
            Value::from("spring"),
            Value::from("winter"),
        ),
        (
            Subject::Global,
            "core:day_of_reckoning",
            Value::Null,
            Value::Int(99),
        ),
    ];
    let empty = sim.hashes();
    sim.drain_events();
    for (subject, name, default, value) in &scopes {
        // Never set: the declared default.
        assert_eq!(&sim.var(*subject, name), default, "{name}");
        let before = sim
            .set_var(*subject, name, value.clone(), Cause::DEBUG)
            .unwrap();
        assert_eq!(&before, default, "{name}");
        assert_eq!(&sim.var(*subject, name), value, "{name}");
    }
    sim.pump();
    let told = sim.drain_events();
    // A cell's variable is written through the world, which says so.
    assert_eq!(
        told.iter()
            .filter(|e| e.name() == "cell_var_changed")
            .count(),
        1
    );
    // The watched ones raised `var_changed` with old and new; the area's
    // and the tract's are not watched.
    let plain = |value: &Value| sim.field_of(value);
    assert_eq!(
        changes(&told),
        [
            (
                "entity",
                "core:kills",
                plain(&Value::Int(0)),
                plain(&Value::Int(3))
            ),
            (
                "loc",
                "core:sealed",
                plain(&Value::Bool(false)),
                plain(&Value::Bool(true))
            ),
            (
                "site",
                "core:alarm_raised",
                plain(&Value::Bool(false)),
                plain(&Value::Bool(true))
            ),
            (
                "realm",
                "core:season",
                plain(&Value::from("spring")),
                plain(&Value::from("winter"))
            ),
            (
                "global",
                "core:day_of_reckoning",
                FieldValue::Null,
                plain(&Value::Int(99))
            ),
        ]
        .map(|(scope, name, old, new)| (scope.to_owned(), name.to_owned(), old, new))
    );
    // Each binds what the variable lives on under the name of its kind.
    let bound_as = |name: &str| {
        told.iter()
            .find(|event| matches!(&event.body, EventBody::VarChanged { name: n, .. } if n == name))
            .map(|event| {
                event
                    .subjects()
                    .iter()
                    .map(|(name, _)| (*name).to_owned())
                    .collect::<Vec<_>>()
            })
            .unwrap()
    };
    assert_eq!(bound_as("core:kills"), ["self"]);
    assert_eq!(bound_as("core:sealed"), ["here"]);
    assert_eq!(bound_as("core:alarm_raised"), ["site"]);
    assert_eq!(bound_as("core:season"), ["realm"]);
    assert_eq!(bound_as("core:day_of_reckoning"), [] as [&str; 0]);

    // Variables are state: in the entity table and in the world.
    let set = sim.hashes();
    assert_ne!(set.entities, empty.entities);
    assert_ne!(set.cosmos, empty.cosmos);
    // Set to the same value, nothing is raised.
    sim.set_var(entity(walker), "core:kills", Value::Int(3), Cause::DEBUG)
        .unwrap();
    sim.pump();
    assert!(changes(&sim.drain_events()).is_empty());

    // They are read at the end of chains, and survive a save.
    assert_eq!(
        eval(&sim, "self.var.kills * 2", walker, here),
        Value::Int(6)
    );
    assert_eq!(
        eval(
            &sim,
            "here.var.sealed and here.realm.var.season == 'winter'",
            walker,
            here
        ),
        Value::Bool(true)
    );
    let global = compile(
        "world.var.day_of_reckoning",
        sim.exprs().registry(),
        &Context::new()
            .subject("world", Type::Subject(sim.exprs().kinds().global))
            .namespace("core"),
        &Type::Any,
    )
    .into_result()
    .unwrap();
    let bind_world = Bound::new().subject(sim.subject_value(Subject::Global));
    assert_eq!(sim.eval(&global, bind_world), Value::Int(99));
    let blob = sim.save().unwrap();
    let cosmos = sim.truth().cosmos().clone();
    let (generators, store) = (sim.generators().clone(), sim.store().clone_store());
    let mut loaded = Sim::load(Arc::clone(sim.defs()), cosmos, &blob)
        .unwrap()
        .with_io(generators, store);
    assert!(loaded.reindex().is_empty());
    assert_eq!(loaded.hashes(), sim.hashes());
    for (subject, name, _, value) in &scopes {
        assert_eq!(&loaded.var(*subject, name), value, "{name}");
    }

    // Unset, they read as their defaults again, and say so.
    for (subject, name, default, _) in &scopes {
        sim.set_var(*subject, name, Value::Null, Cause::DEBUG)
            .unwrap();
        assert_eq!(&sim.var(*subject, name), default, "{name}");
    }
    sim.pump();
    assert_eq!(changes(&sim.drain_events()).len(), 5);
    assert_eq!(sim.hashes().entities, empty.entities);

    // Typed access: a declared variable is held to its declaration.
    let code = |sim: &mut Sim, subject: Subject, name: &str, value: Value| {
        sim.set_var(subject, name, value, Cause::DEBUG)
            .unwrap_err()
            .code
    };
    let me = entity(walker);
    assert_eq!(
        code(&mut sim, me, "core:kills", Value::from("many")),
        "vars.wrong_type"
    );
    assert_eq!(
        code(&mut sim, me, "core:kills", real(1.5)),
        "vars.wrong_type"
    );
    assert_eq!(
        code(&mut sim, me, "core:season", Value::from("x")),
        "vars.wrong_scope"
    );
    assert_eq!(
        code(&mut sim, Subject::Global, "core:kills", Value::Int(1)),
        "vars.wrong_scope"
    );
    let element = Subject::Element(here, Layer::Floor);
    assert_eq!(
        code(&mut sim, element, "core:x", Value::Int(1)),
        "vars.wrong_scope"
    );
    let gone = entity(EntityId::new(900, 0));
    assert_eq!(
        code(&mut sim, gone, "core:kills", Value::Int(1)),
        "subject.gone"
    );
    let unloaded = Subject::Loc(Location::new(here.grid, CellPos::new(1, 1, 0)));
    assert_eq!(
        code(&mut sim, unloaded, "core:sealed", Value::Bool(true)),
        "world.not_loaded"
    );
    let mut deep = Value::list([]);
    for _ in 0..40 {
        deep = Value::list([deep]);
    }
    assert_eq!(code(&mut sim, me, "core:anything", deep), "vars.too_deep");
    // An int serves where a real is declared; an undeclared variable
    // holds whatever it is given.
    sim.set_var(
        Subject::Area(area),
        "core:heat",
        Value::Int(3),
        Cause::DEBUG,
    )
    .unwrap();
    sim.set_var(
        me,
        "core:scratch",
        Value::list([Value::Int(1), Value::Null]),
        Cause::DEBUG,
    )
    .unwrap();
    assert_eq!(
        sim.var(me, "core:scratch"),
        Value::list([Value::Int(1), Value::Null])
    );
    // Nothing is read of what does not exist, a default included.
    assert_eq!(sim.var(gone, "core:kills"), Value::Null);
}

#[test]
fn places_are_assigned_with_keys_and_added_to() {
    let mut sim = sim();
    let me = spawn(&mut sim, creature(), on(3, 4));
    let friend = spawn(&mut sim, creature(), on(5, 4));
    let here = at(3, 4);
    let context = scope(&sim).local("who", Type::Subject(sim.exprs().kinds().entity));
    let place = |sim: &Sim, source: &str| {
        compile_place(source, sim.exprs().registry(), &context)
            .into_result()
            .unwrap_or_else(|errors| panic!("`{source}`: {errors:?}"))
    };
    let bind = |sim: &Sim| bound(sim, me, here).local(sim.subject_value(entity(friend)));
    let assign = |sim: &mut Sim, source: &str, how: Assign, value: Value| {
        let (place, bound) = (place(sim, source), bind(sim));
        sim.assign(&place, how, value, &bound, Cause::DEBUG)
    };
    let me_subject = entity(me);

    // A variable, set and added to.
    assign(&mut sim, "self.var.kills", Assign::Set, Value::Int(2)).unwrap();
    assign(&mut sim, "self.var.kills", Assign::Add, Value::Int(3)).unwrap();
    assert_eq!(sim.var(me_subject, "core:kills"), Value::Int(5));
    // Added to its default where it was never set.
    assign(&mut sim, "$who.var.kills", Assign::Add, Value::Int(1)).unwrap();
    assert_eq!(sim.var(entity(friend), "core:kills"), Value::Int(1));
    // A key into nothing makes a map; an entry is added to; an entry set
    // to null is removed.
    assign(
        &mut sim,
        "self.var.tally['ash']",
        Assign::Set,
        Value::Int(3),
    )
    .unwrap();
    assign(
        &mut sim,
        "self.var.tally['iron']",
        Assign::Add,
        Value::Int(1),
    )
    .unwrap();
    assign(
        &mut sim,
        "self.var.tally['ash']",
        Assign::Add,
        Value::Int(4),
    )
    .unwrap();
    assert_eq!(
        sim.var(me_subject, "core:tally"),
        Value::map([("ash", Value::Int(7)), ("iron", Value::Int(1))])
    );
    assign(&mut sim, "self.var.tally['iron']", Assign::Set, Value::Null).unwrap();
    assert_eq!(
        sim.var(me_subject, "core:tally"),
        Value::map([("ash", Value::Int(7))])
    );
    assert_eq!(
        eval(&sim, "self.var.tally['ash'] + 1", me, here),
        Value::Int(8)
    );
    // A list is appended to, and an element of it replaced.
    let friend_value = sim.subject_value(entity(friend));
    assign(&mut sim, "self.var.seen", Assign::Add, friend_value.clone()).unwrap();
    assign(&mut sim, "self.var.seen", Assign::Add, friend_value.clone()).unwrap();
    assert_eq!(eval(&sim, "len(self.var.seen)", me, here), Value::Int(2));
    let me_value = sim.subject_value(me_subject);
    assign(&mut sim, "self.var.seen[0]", Assign::Set, me_value.clone()).unwrap();
    assert_eq!(
        sim.var(me_subject, "core:seen"),
        Value::list([me_value, friend_value.clone()])
    );
    // What a stored handle names can be read through it.
    assert_eq!(
        eval(&sim, "self.var.seen[1].loc.x", me, here),
        Value::Int(5)
    );

    // A settable property: the write is a relocation.
    sim.pump();
    sim.drain_events();
    let to = sim.subject_value(Subject::Loc(at(8, 8)));
    assign(&mut sim, "$who.loc", Assign::Set, to).unwrap();
    sim.pump();
    assert_eq!(names(&sim.drain_events()), ["entity_relocated"]);
    assert_eq!(sim.truth().place(friend), Some(on(8, 8)));
    assert!(matches!(
        place(&sim, "$who.loc").root(),
        PlaceRoot::Property(_)
    ));

    // Refusals.
    let hash = sim.state_hash();
    let code = |result: Result<(), clash_sim::Reason>| result.unwrap_err().code;
    assert_eq!(
        code(assign(
            &mut sim,
            "self.var.seen[5]",
            Assign::Set,
            Value::Null
        )),
        "write.bad_key"
    );
    assert_eq!(
        code(assign(
            &mut sim,
            "self.var.kills",
            Assign::Add,
            Value::from("x")
        )),
        "write.cannot_add"
    );
    assert_eq!(
        code(assign(
            &mut sim,
            "self.var.tally['ash']",
            Assign::Set,
            Value::from("x")
        )),
        "vars.wrong_type"
    );
    assert_eq!(
        code(assign(&mut sim, "$who", Assign::Set, Value::Int(1))),
        "write.not_a_subject"
    );
    sim.remove_entity(friend, Cause::DEBUG).unwrap();
    let hash_after = sim.state_hash();
    assert_ne!(hash, hash_after);
    assert_eq!(
        code(assign(
            &mut sim,
            "$who.var.kills",
            Assign::Set,
            Value::Int(1)
        )),
        "subject.gone"
    );
    assert_eq!(sim.state_hash(), hash_after);
    // What cannot be assigned to is refused when it is compiled.
    assert!(compile_place("self.mass", sim.exprs().registry(), &context).has_errors());
}

// ---- events ----------------------------------------------------------------

#[test]
fn the_simulations_events_are_event_types_of_the_expression_language() {
    let mut sim = sim();
    let registry = Arc::clone(sim.exprs().registry());
    let kinds = *sim.exprs().kinds();
    // Every event type the simulation knows is in the registry, with
    // its subjects of their kinds and its fields.
    for ty in sim.event_types() {
        let id = registry
            .event_id(&ty.name)
            .unwrap_or_else(|| panic!("`{}` is not in the expression registry", ty.name));
        let spec = &registry.event(id).unwrap().spec;
        assert!(!spec.doc.is_empty(), "{}", ty.name);
        for (name, kind) in &ty.subjects {
            let name = name.trim_end_matches('?');
            let found = spec.subjects.iter().find(|(have, _)| have == name);
            let found = found.unwrap_or_else(|| panic!("`{}` binds no `{name}`", ty.name));
            assert_eq!(registry.kind(found.1).unwrap().spec.name, *kind);
        }
        for field in &ty.fields {
            assert!(
                spec.fields.iter().any(|have| have.name == field.name),
                "`{}` has no field `{}`",
                ty.name,
                field.name
            );
        }
        // The cause's subjects come after its own.
        let last: Vec<&str> = spec
            .subjects
            .iter()
            .rev()
            .take(2)
            .map(|(name, _)| name.as_str())
            .collect();
        assert_eq!(last, ["instigator", "actor"], "{}", ty.name);
    }
    let spec = |name: &str| {
        &registry
            .event(registry.event_id(name).unwrap())
            .unwrap()
            .spec
    };
    assert_eq!(
        spec("entity_relocated").subjects[0],
        ("self".to_owned(), kinds.entity)
    );
    assert_eq!(
        spec("entity_relocated").subjects[1],
        ("here".to_owned(), kinds.loc)
    );
    assert_eq!(spec("element_damaged").fields[1].ty, Type::Int);
    assert_eq!(
        spec("clock_rate_changed").fields[1].ty,
        Type::Real.nullable()
    );
    // An event a mod defines, by its qualified id.
    let ritual = spec("core:ritual_completed");
    assert_eq!(ritual.subjects[0], ("caster".to_owned(), kinds.entity));
    assert_eq!(ritual.fields[0].name, "power");
    assert_eq!(ritual.fields[0].ty, Type::Real);

    // An expression in an event's scope type-checks against it, and is
    // evaluated against what the event binds.
    let event_scope = |name: &str| {
        let event = registry.event(registry.event_id(name).unwrap()).unwrap();
        let mut context = Context::new().event(event.id).namespace("core");
        for (subject, kind) in &event.spec.subjects {
            context.add_subject(subject.clone(), Type::Subject(*kind), false);
        }
        context
    };
    let relocated = event_scope("entity_relocated");
    let code = compile(
        "self.loc == here and here.x > 2",
        &registry,
        &relocated,
        &Type::Bool,
    )
    .into_result()
    .unwrap();
    assert!(compile("event.energy", &registry, &relocated, &Type::Any).has_errors());
    let mover = spawn(&mut sim, creature(), on(1, 1));
    sim.pump();
    sim.drain_events();
    sim.relocate(mover, on(7, 3), Cause::DEBUG).unwrap();
    sim.pump();
    let told = sim.drain_events();
    let event = told
        .iter()
        .find(|e| e.name() == "entity_relocated")
        .unwrap();
    let bound = sim.exprs().bind_event(event).unwrap();
    assert_eq!(bound.subjects.len(), 4, "self, here, actor, instigator");
    assert_eq!(sim.eval(&code, bound), Value::Bool(true));

    // The fields of a watched variable's event.
    let changed = event_scope("var_changed");
    let code = compile(
        "event.name == 'core:kills' and event.new > event.old and self.var.kills == event.new",
        &registry,
        &changed,
        &Type::Bool,
    )
    .into_result()
    .unwrap();
    sim.set_var(entity(mover), "core:kills", Value::Int(2), Cause::DEBUG)
        .unwrap();
    sim.pump();
    let told = sim.drain_events();
    let event = told.iter().find(|e| e.name() == "var_changed").unwrap();
    let bound = sim.exprs().bind_event(event).unwrap();
    assert_eq!(
        sim.eval(&code, bound.clone()),
        Value::Bool(true),
        "{bound:?}"
    );
}

#[test]
fn the_host_answers_null_for_what_it_does_not_know() {
    let sim = sim();
    let mut host = sim.host(Bound::new().subject(Value::Int(1)));
    assert_eq!(host.subject(0), Value::Int(1));
    assert_eq!(host.subject(5), Value::Null);
    assert_eq!(host.local(0), Value::Null);
    assert_eq!(host.event_field(0), Value::Null);
    let stranger = Handle::new(clash_expr::KindId::new(900), 4);
    let any_property = sim.exprs().property_id("entity", "loc").unwrap();
    assert_eq!(host.property(any_property, stranger, &[]), Value::Null);
    assert_eq!(host.variable(stranger, "core:kills"), Value::Null);
    assert!(host.rng().is_none());
    let registry: &Registry = sim.exprs().registry();
    assert!(registry.kinds().count() >= 8);
}
