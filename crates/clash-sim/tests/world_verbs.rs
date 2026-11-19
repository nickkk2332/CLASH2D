//! The world's verbs (02 §23; package K8) against the real simulation:
//! each bound verb is called as the rule runtime calls it, with a cause
//! of its own, and the world event it produces is found in what the
//! simulation delivers, carrying that cause. Refusals carry reasons.

mod common;

use clash_core::{EntityId, Real};
use clash_expr::Value;
use clash_sim::rule::{RunView, VerbCall};
use clash_sim::{EventBody, Place, Reason, Sim, SimEvent, Subject};
use clash_world::{Cause, CauseKind, CellPos, Layer, Location, WorldEvent};

use common::{GRID, SIDE, at, creature, names, on, spawn};

/// The cause every verb in these tests is handed: a rule's doing.
fn rule_cause() -> Cause {
    Cause::new(CauseKind::Eoc)
}

/// An argument as a test writes it: a cell, an element or an entity,
/// made into a handle when the call is made, or a value as it is.
#[derive(Clone)]
enum A {
    Loc(i32, i32, i32),
    El(i32, i32, Layer),
    Ent(EntityId),
    V(Value),
}

impl A {
    fn value(&self, sim: &Sim) -> Value {
        match self {
            A::Loc(x, y, z) => {
                sim.subject_value(Subject::Loc(Location::new(GRID, CellPos::new(*x, *y, *z))))
            }
            A::El(x, y, layer) => sim.subject_value(Subject::Element(at(*x, *y), *layer)),
            A::Ent(id) => sim.subject_value(Subject::Entity(*id)),
            A::V(value) => value.clone(),
        }
    }
}

/// Calls a verb of the engine's table as the rule runtime will, with
/// its arguments by name; those left out are null.
fn call(sim: &mut Sim, name: &str, args: &[(&str, A)]) -> Result<Value, Reason> {
    let registry = clash_sim::engine_registry();
    let verb = registry
        .verb_id(name)
        .unwrap_or_else(|| panic!("no verb `{name}`"));
    let spec = &registry.verb(verb).unwrap().spec;
    for (given, _) in args {
        assert!(
            spec.args.iter().any(|arg| arg.name == *given),
            "`{name}` takes no argument `{given}`"
        );
    }
    let values: Vec<Value> = spec
        .args
        .iter()
        .map(|arg| {
            args.iter()
                .find(|(given, _)| *given == arg.name)
                .map_or(Value::Null, |(_, value)| value.value(sim))
        })
        .collect();
    let def = clash_sim::verb::engine_verbs()
        .into_iter()
        .find(|def| def.name == name)
        .unwrap();
    let call = VerbCall {
        verb,
        args: &values,
        by: RunView::new(&[], &[]),
    };
    (def.run)(sim, &call, rule_cause())
}

fn loc(x: i32, y: i32, z: i32) -> A {
    A::Loc(x, y, z)
}

fn element(x: i32, y: i32, layer: Layer) -> A {
    A::El(x, y, layer)
}

fn text(value: &str) -> A {
    A::V(Value::from(value))
}

fn int(value: i64) -> A {
    A::V(Value::Int(value))
}

fn real(value: f64) -> A {
    A::V(Value::Real(Real::of(value)))
}

fn list(values: &[f64]) -> A {
    A::V(Value::list(
        values.iter().map(|value| Value::Real(Real::of(*value))),
    ))
}

fn null() -> A {
    A::V(Value::Null)
}

fn returned_text(value: &str) -> Value {
    Value::from(value)
}

/// Delivers what is queued and gives what was delivered.
fn delivered(sim: &mut Sim) -> Vec<SimEvent> {
    sim.pump();
    sim.drain_events()
}

/// The first delivered event of a name, which must carry the rule's
/// cause.
fn found<'a>(events: &'a [SimEvent], name: &str) -> &'a SimEvent {
    let event = events
        .iter()
        .find(|event| event.name() == name)
        .unwrap_or_else(|| panic!("no `{name}` among {:?}", names(events)));
    assert_eq!(
        event.cause,
        rule_cause(),
        "`{name}` carries the verb's cause"
    );
    event
}

fn block_text(sim: &Sim, x: i32, y: i32) -> String {
    let cosmos = sim.truth().cosmos();
    let state = cosmos
        .grid(GRID)
        .unwrap()
        .element(at(x, y).cell, Layer::Block)
        .unwrap();
    cosmos.defs().state_text(Layer::Block, state)
}

#[test]
fn setting_elements_changes_the_cell_and_says_so() {
    let mut sim = common::sim(1);
    let here = loc(4, 4, 0);
    call(
        &mut sim,
        "set_block",
        &[("at", here.clone()), ("to", text("door"))],
    )
    .unwrap();
    let events = delivered(&mut sim);
    found(&events, "element_changed");
    assert_eq!(block_text(&sim, 4, 4), "core:door@core:wood#closed");

    call(
        &mut sim,
        "set_state",
        &[("of", element(4, 4, Layer::Block)), ("state", text("open"))],
    )
    .unwrap();
    found(&delivered(&mut sim), "element_changed");
    assert_eq!(block_text(&sim, 4, 4), "core:door@core:wood#open");

    call(
        &mut sim,
        "set_material",
        &[
            ("of", element(4, 4, Layer::Block)),
            ("material", text("core:steel")),
        ],
    )
    .unwrap();
    found(&delivered(&mut sim), "element_changed");
    assert_eq!(block_text(&sim, 4, 4), "core:door@core:steel#open");

    call(
        &mut sim,
        "set_fixture",
        &[("at", here.clone()), ("to", text("desk"))],
    )
    .unwrap();
    found(&delivered(&mut sim), "element_changed");
    call(
        &mut sim,
        "set_floor",
        &[("at", here.clone()), ("to", null())],
    )
    .unwrap();
    found(&delivered(&mut sim), "element_changed");
    call(&mut sim, "set_block", &[("at", here), ("to", null())]).unwrap();
    found(&delivered(&mut sim), "element_changed");
    assert_eq!(block_text(&sim, 4, 4), "engine:none");
}

#[test]
fn a_state_change_keeps_the_damage_an_element_had() {
    let mut sim = common::sim(1);
    call(
        &mut sim,
        "set_block",
        &[("at", loc(6, 6, 0)), ("to", text("door"))],
    )
    .unwrap();
    let door = element(6, 6, Layer::Block);
    let hit = call(
        &mut sim,
        "damage",
        &[
            ("at", door.clone()),
            ("kind", text("core:impact")),
            ("energy", real(50.0)),
            ("intensity", real(10.0e6)),
        ],
    )
    .unwrap();
    assert_eq!(hit, returned_text("damaged"));
    found(&delivered(&mut sim), "element_damaged");
    let damage =
        |sim: &Sim| sim.property_named(Subject::Element(at(6, 6), Layer::Block), "damage", &[]);
    let before = damage(&sim);
    assert!(before.as_int().unwrap() > 0);
    call(
        &mut sim,
        "set_state",
        &[("of", door), ("state", text("open"))],
    )
    .unwrap();
    assert_eq!(damage(&sim), before);
}

#[test]
fn refusals_carry_reasons_and_change_nothing() {
    let mut sim = common::sim(1);
    let before = sim.state_hash();
    // Not loaded: the tracts beyond the first two are not.
    let far = loc(SIDE + 40, 4, 0);
    let refused = call(&mut sim, "set_block", &[("at", far), ("to", text("wall"))]).unwrap_err();
    assert_eq!(refused.code, "world.not_loaded");
    // Out of the realm: its north edge is a barrier.
    let beyond = loc(4, -1, 0);
    let refused = call(
        &mut sim,
        "set_block",
        &[("at", beyond), ("to", text("wall"))],
    )
    .unwrap_err();
    assert_eq!(refused.code, "world.outside_realm");
    // An unknown state, a state the definition does not have, no element.
    let here = loc(4, 4, 0);
    let refused = call(
        &mut sim,
        "set_block",
        &[("at", here.clone()), ("to", text("nothing_like_it"))],
    )
    .unwrap_err();
    assert_eq!(refused.code, "world.unknown_state");
    let refused = call(
        &mut sim,
        "set_state",
        &[("of", element(4, 4, Layer::Block)), ("state", text("open"))],
    )
    .unwrap_err();
    assert_eq!(refused.code, "world.no_element");
    call(&mut sim, "set_block", &[("at", here), ("to", text("wall"))]).unwrap();
    let after_wall = sim.state_hash();
    let refused = call(
        &mut sim,
        "set_state",
        &[("of", element(4, 4, Layer::Block)), ("state", text("ajar"))],
    )
    .unwrap_err();
    assert_eq!(refused.code, "world.unknown_state");
    let refused = call(
        &mut sim,
        "damage",
        &[
            ("at", element(4, 4, Layer::Block)),
            ("kind", text("core:nothing")),
            ("energy", real(1.0)),
            ("intensity", real(1.0)),
        ],
    )
    .unwrap_err();
    assert_eq!(refused.code, "world.unknown_def");
    let refused = call(&mut sim, "set_block", &[("at", int(3)), ("to", null())]).unwrap_err();
    assert_eq!(refused.code, "world.bad_argument");
    assert_eq!(sim.state_hash(), after_wall);
    assert_ne!(before, after_wall);
}

#[test]
fn damage_and_destruction_are_reported_with_the_cause() {
    let mut sim = common::sim(1);
    call(
        &mut sim,
        "set_block",
        &[("at", loc(8, 8, 0)), ("to", text("wall"))],
    )
    .unwrap();
    delivered(&mut sim);
    let unaffected = call(
        &mut sim,
        "damage",
        &[
            ("at", loc(8, 8, 0)),
            ("layer", text("block")),
            ("kind", text("core:impact")),
            ("energy", real(1.0e3)),
            ("intensity", real(1.0e3)),
        ],
    )
    .unwrap();
    assert_eq!(unaffected, returned_text("unaffected"));
    let destroyed = call(
        &mut sim,
        "damage",
        &[
            ("at", loc(8, 8, 0)),
            ("kind", text("core:impact")),
            ("energy", real(1.0e9)),
            ("intensity", real(1.0e9)),
            ("dir", list(&[1.0, 0.0, 0.0])),
        ],
    )
    .unwrap();
    assert_eq!(destroyed, returned_text("destroyed"));
    let events = delivered(&mut sim);
    found(&events, "element_destroyed");
    assert_eq!(block_text(&sim, 8, 8), "engine:none");

    call(
        &mut sim,
        "set_block",
        &[("at", loc(9, 9, 0)), ("to", text("wall"))],
    )
    .unwrap();
    let gone = call(
        &mut sim,
        "destroy",
        &[
            ("of", element(9, 9, Layer::Block)),
            ("kind", text("core:heat")),
        ],
    )
    .unwrap();
    assert_eq!(gone, Value::Bool(true));
    let events = delivered(&mut sim);
    let event = found(&events, "element_destroyed");
    let EventBody::World(WorldEvent::ElementDestroyed { absorbed, .. }) = &event.body else {
        panic!("not a destruction");
    };
    assert_eq!(*absorbed, Real::ZERO);
    let again = call(
        &mut sim,
        "destroy",
        &[
            ("of", element(9, 9, Layer::Block)),
            ("kind", text("core:heat")),
        ],
    )
    .unwrap();
    assert_eq!(again, Value::Bool(false));

    call(&mut sim, "check_stability", &[("at", loc(9, 9, 0))]).unwrap();
}

#[test]
fn a_release_starts_and_runs_its_course() {
    let mut sim = common::sim(1);
    call(
        &mut sim,
        "release",
        &[
            ("at", loc(20, 20, 0)),
            ("kind", text("core:blast")),
            ("energy", real(8.0e5)),
        ],
    )
    .unwrap();
    found(&delivered(&mut sim), "release_started");
    let until = sim.now() + clash_core::Dur::from_secs(1);
    sim.advance(clash_sim::AdvanceLimit::Until(until));
    // What the front did is the release's doing: a process, as 02 §13
    // has it.
    let events = delivered(&mut sim);
    let done = events
        .iter()
        .find(|event| event.name() == "energy_released")
        .unwrap_or_else(|| panic!("no `energy_released` among {:?}", names(&events)));
    assert_eq!(done.cause.kind, CauseKind::Process);

    call(
        &mut sim,
        "release",
        &[
            ("at", loc(20, 20, 0)),
            ("kind", text("core:heat")),
            ("energy", real(1.0e5)),
            ("shape", text("cone")),
            ("dir", loc(30, 20, 0)),
            ("half_angle", real(0.4)),
        ],
    )
    .unwrap();
    found(&delivered(&mut sim), "release_started");
    let refused = call(
        &mut sim,
        "release",
        &[
            ("at", loc(20, 20, 0)),
            ("kind", text("core:heat")),
            ("energy", real(1.0e5)),
            ("shape", text("beam")),
        ],
    )
    .unwrap_err();
    assert_eq!(refused.code, "world.bad_argument");
    let refused = call(
        &mut sim,
        "release",
        &[
            ("at", loc(20, 20, 0)),
            ("kind", text("core:heat")),
            ("energy", real(-1.0)),
        ],
    )
    .unwrap_err();
    assert_eq!(refused.code, "world.refused");
}

#[test]
fn fields_come_and_go_by_verb() {
    let mut sim = common::sim(1);
    let here = loc(5, 5, 0);
    let old = call(
        &mut sim,
        "add_field",
        &[
            ("at", here.clone()),
            ("field", text("core:smoke")),
            ("amount", int(100)),
        ],
    )
    .unwrap();
    assert_eq!(old, Value::Int(0));
    found(&delivered(&mut sim), "field_appeared");
    let old = call(
        &mut sim,
        "set_field",
        &[
            ("at", here),
            ("field", text("core:smoke")),
            ("level", int(0)),
        ],
    )
    .unwrap();
    assert_eq!(old, Value::Int(100));
    found(&delivered(&mut sim), "field_vanished");
}

#[test]
fn an_entity_is_thrown_pushed_and_caught() {
    let mut sim = common::sim(1);
    let crate_ = spawn(
        &mut sim,
        creature().with_presence(
            clash_sim::Presence::new(clash_world::OccupantKind::Creature).with_mass(Real::of(50.0)),
        ),
        on(10, 10),
    );
    let thrown = A::Ent(crate_);
    call(
        &mut sim,
        "launch",
        &[("what", thrown.clone()), ("vel", list(&[3.0, 0.0, 4.0]))],
    )
    .unwrap();
    found(&delivered(&mut sim), "body_launched");
    assert!(sim.truth().cosmos().body_of(crate_).is_some());
    let refused = call(
        &mut sim,
        "launch",
        &[("what", thrown.clone()), ("vel", list(&[1.0, 0.0, 0.0]))],
    )
    .unwrap_err();
    assert_eq!(refused.code, "world.refused");
    call(
        &mut sim,
        "impulse",
        &[("what", thrown.clone()), ("j", list(&[0.0, 50.0, 0.0]))],
    )
    .unwrap();
    let body = sim.truth().cosmos().body_of(crate_).unwrap();
    assert!(sim.truth().cosmos().body(body).unwrap().vel[1] > Real::ZERO);
    call(&mut sim, "arrest", &[("what", thrown.clone())]).unwrap();
    assert!(sim.truth().cosmos().body_of(crate_).is_none());
    let refused = call(&mut sim, "arrest", &[("what", thrown)]).unwrap_err();
    assert_eq!(refused.code, "world.not_in_world");

    let stored = spawn(&mut sim, creature(), Place::Stored);
    let refused = call(
        &mut sim,
        "launch",
        &[("what", A::Ent(stored)), ("vel", list(&[1.0, 0.0, 0.0]))],
    )
    .unwrap_err();
    assert_eq!(refused.code, "world.not_in_world");
}

#[test]
fn areas_are_made_enclosed_and_removed() {
    let mut sim = common::sim(1);
    let walker = spawn(&mut sim, creature(), on(12, 12));
    delivered(&mut sim);
    let area = call(
        &mut sim,
        "create_area",
        &[
            ("at", loc(10, 10, 0)),
            ("to", loc(14, 14, 0)),
            ("kind", text("core:slow_time")),
            ("tags", A::V(Value::list([Value::from("ward")]))),
        ],
    )
    .unwrap();
    let events = delivered(&mut sim);
    let entered = found(&events, "area_entered");
    assert_eq!(entered.subject("target"), Some(Subject::Entity(walker)));
    let Some(Subject::Area(id)) = sim.subject_of(&area) else {
        panic!("not an area: {area:?}");
    };
    assert!(sim.truth().cosmos().area(id).unwrap().has_tag("ward"));
    call(&mut sim, "remove_area", &[("area", A::V(area))]).unwrap();
    found(&delivered(&mut sim), "area_exited");

    // A room of walls: three by three inside, with a roof of floor over
    // it, encloses; the open ground does not.
    let refused = call(
        &mut sim,
        "enclose",
        &[("from", loc(30, 30, 0)), ("limit", int(500))],
    )
    .unwrap_err();
    assert_eq!(refused.code, "world.refused");
    for x in 19..=23 {
        for y in 19..=23 {
            let inside = (20..=22).contains(&x) && (20..=22).contains(&y);
            if !inside {
                call(
                    &mut sim,
                    "set_block",
                    &[("at", loc(x, y, 0)), ("to", text("wall"))],
                )
                .unwrap();
            }
            call(
                &mut sim,
                "set_floor",
                &[("at", loc(x, y, 1)), ("to", text("slab"))],
            )
            .unwrap();
        }
    }
    let room = call(
        &mut sim,
        "enclose",
        &[("from", loc(21, 21, 0)), ("limit", int(500))],
    )
    .unwrap();
    assert!(matches!(sim.subject_of(&room), Some(Subject::Area(_))));
}
