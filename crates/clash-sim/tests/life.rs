//! Living things in the running simulation (06 §4 to §8, §10, §12):
//! people and animals spawned from the templates of `mods/core`, cut,
//! bleeding on the timeline, stopping by their definitions, hurt by a
//! collapse, shelved and restored, saved and loaded, and replayed.

mod common;

use std::sync::Arc;

use clash_core::{Dur, EntityId, Real, Tick};
use clash_expr::Value;
use clash_sim::body::props::part_handle;
use clash_sim::mind::VitalState;
use clash_sim::rule::{RunView, VerbCall};
use clash_sim::stats::{PoolId, TrackId};
use clash_sim::template::{CharacterSpec, CreationSession, Selection, resolve};
use clash_sim::verb::engine_verbs;
use clash_sim::{
    Advance, AdvanceLimit, Command, DebugCommand, Delivery, Handler, Place, Reason, Sim, SimEvent,
    Subject,
};
use clash_time::{Timeline, Wake, WakeDomain, WakeId};
use clash_world::dims::TRACT_XY;
use clash_world::test_support::load_column;
use clash_world::{
    BlockState, Cause, CellPos, ColumnBuilder, Cosmos, GridId, Layer, Location, RealmId, TractPos,
    WorldDefs,
};
use common::{GRID, debug, entity_of, names, on};

fn defs() -> Arc<clash_data::Defs> {
    common::life_defs("[]")
}

fn sim(seed: u64) -> Sim {
    common::sim_over(defs(), seed)
}

fn person(name: &str) -> CharacterSpec {
    CharacterSpec::of(["core:person"]).with("core:person.name", Selection::Text(name.to_owned()))
}

fn spawn(sim: &mut Sim, spec: &CharacterSpec, place: Place) -> EntityId {
    let made = sim
        .spawn_template(spec, place, Cause::DEBUG)
        .unwrap_or_else(|reason| panic!("{reason:?}"))
        .entity;
    sim.pump();
    made
}

fn pool(sim: &Sim, name: &str) -> PoolId {
    sim.stat_defs().unwrap().pool_id(name).unwrap()
}

fn level(sim: &Sim, who: EntityId, name: &str) -> f64 {
    let stats = sim.components().stats.get(who).unwrap();
    stats
        .pool_level(sim.timeline(), pool(sim, name))
        .unwrap()
        .get()
}

fn seconds(s: f64) -> Dur {
    Dur::from_micros((s * 1.0e6) as i64)
}

fn advance_to(sim: &mut Sim, tick: Tick) {
    assert_eq!(
        sim.advance(AdvanceLimit::Until(tick)),
        Advance::Reached(tick)
    );
}

/// Cuts a part with the reference blade (07 §3.2), through the `hit`
/// verb as a rule would call it.
fn cut(sim: &mut Sim, who: EntityId, part: &str, energy: f64) -> Value {
    let key = sim
        .components()
        .body
        .get(who)
        .unwrap()
        .part_named(part)
        .unwrap()
        .key();
    let kinds = sim.body_defs().unwrap().kinds();
    let (area, give) = (2.0e-5, 0.002);
    let args = [
        sim.subject_value(Subject::Entity(who)),
        Value::from("core:cut"),
        Value::Real(Real::of(energy)),
        Value::Real(Real::of(energy / (area * give))),
        Value::Subject(part_handle(kinds, who, key)),
        Value::Null,
        Value::Null,
        Value::Real(Real::of(area)),
        Value::Real(Real::of(0.9)),
        Value::Null,
    ];
    let outcome = call(sim, "hit", &args).unwrap();
    sim.pump();
    outcome
}

/// Calls one of the engine's verbs as the rule runtime does.
fn call(sim: &mut Sim, name: &str, args: &[Value]) -> Result<Value, clash_sim::Reason> {
    let verb = engine_verbs()
        .into_iter()
        .find(|verb| verb.name == name)
        .unwrap_or_else(|| panic!("no verb `{name}`"));
    let id = sim
        .exprs()
        .registry()
        .verb_id(name)
        .expect("the verb is registered");
    let call = VerbCall {
        verb: id,
        args,
        by: RunView::new(&[], &[]),
    };
    (verb.run)(sim, &call, Cause::DEBUG)
}

fn about(events: &[SimEvent], who: EntityId) -> Vec<&SimEvent> {
    events
        .iter()
        .filter(|event| entity_of(event, "self") == Some(who))
        .collect()
}

#[test]
fn a_person_is_made_from_its_template() {
    let mut sim = sim(1);
    sim.drain_events();
    let ada = spawn(&mut sim, &person("Ada"), on(10, 10));
    let told = sim.drain_events();
    let told_names = names(&told);
    assert_eq!(told_names.first(), Some(&"entity_created"));
    assert_eq!(told_names.last(), Some(&"entity_spawned"));
    assert!(told_names.contains(&"entity_relocated"));
    assert_eq!(
        told.iter().filter(|e| e.name() == "fault").count(),
        0,
        "{told_names:?}"
    );

    // A body of the plan, at its size, with its mass in the world.
    let body = sim.components().body.get(ada).unwrap();
    assert_eq!(body.parts().len(), 16, "fifteen parts and the brain");
    assert!(body.mass() > Real::of(70.0) && body.mass() < Real::of(80.0));
    let world_mass = sim.truth().cosmos().occupant(ada).unwrap().occupant.mass;
    assert_eq!(world_mass, body.mass());
    // Stats: every attribute, the two pools, full blood.
    let max = sim
        .components()
        .stats
        .get(ada)
        .unwrap()
        .pool_max(pool(&sim, "core:blood"))
        .unwrap();
    assert!((max.get() - body.mass().get() * 0.07).abs() < 1e-6);
    assert!((level(&sim, ada, "core:blood") - max.get()).abs() < 1e-9);
    // A derived attribute read what the body can do.
    let speed = sim.stat_defs().unwrap().attribute_id("core:speed").unwrap();
    assert_eq!(
        sim.components().stats.get(ada).unwrap().attr(speed),
        Some(Real::of(1.4))
    );
    // A mind, awake; a name; a glyph; tags; notable.
    assert_eq!(sim.vital(ada), Some(VitalState::Awake));
    assert!(sim.can_act(ada).is_ok());
    let identity = sim.components().identity.get(ada).unwrap();
    assert_eq!(identity.name(), Some("Ada"));
    assert_eq!(identity.templates().len(), 1);
    assert_eq!(sim.glyph(ada).unwrap().ch, "@");
    assert!(sim.truth().entity(ada).unwrap().tags.contains("person"));
    assert!(sim.is_notable(ada));

    // Properties are answered by the simulation.
    let value = sim.property_named(Subject::Entity(ada), "mass", &[]);
    assert!(matches!(value, Value::Real(_)));

    // An animal from its own template, with a plan of its own.
    let dog = spawn(&mut sim, &CharacterSpec::of(["dog"]), on(11, 10));
    let body = sim.components().body.get(dog).unwrap();
    assert!(body.part_named("fl_leg").is_some());
    assert_eq!(sim.glyph(dog).unwrap().ch, "d");
    let strength = sim
        .stat_defs()
        .unwrap()
        .attribute_id("core:strength")
        .unwrap();
    assert_eq!(
        sim.components().stats.get(dog).unwrap().attr(strength),
        Some(Real::of(6.0))
    );
}

#[test]
fn a_spec_that_does_not_resolve_is_refused_and_leaves_no_trace() {
    let mut sim = sim(1);
    let before = sim.state_hash();
    let refused = |sim: &mut Sim, spec: &CharacterSpec| {
        sim.spawn_template(spec, on(5, 5), Cause::DEBUG)
            .unwrap_err()
            .code
    };
    assert_eq!(
        refused(&mut sim, &CharacterSpec::of(["core:nobody"])),
        "unknown_template"
    );
    assert_eq!(refused(&mut sim, &CharacterSpec::default()), "no_templates");
    let spec = person("Ada").with("core:person.wings", Selection::Number(Real::ONE));
    assert_eq!(refused(&mut sim, &spec), "unknown_choice");
    // Each kind of choice refuses what it does not take.
    for (choice, selection) in [
        (
            "core:person.build",
            Selection::Options(vec!["huge".to_owned()]),
        ),
        ("core:person.build", Selection::Options(vec![])),
        ("core:person.build", Selection::Number(Real::ONE)),
        ("core:person.stature", Selection::Number(Real::of(2.0))),
        (
            "core:person.training",
            Selection::Options(vec!["runner".into(), "reader".into(), "runner".into()]),
        ),
        ("core:person.name", Selection::Text("x".repeat(40))),
    ] {
        let spec = person("Ada").with(choice, selection);
        assert_eq!(refused(&mut sim, &spec), "bad_selection", "{choice}");
    }
    assert_eq!(sim.state_hash(), before);
    // A place that cannot take it is the kernel's refusal.
    let far = Place::at(Location::new(GRID, CellPos::new(-50, -50, 0)));
    assert!(
        sim.spawn_template(&person("Ada"), far, Cause::DEBUG)
            .is_err()
    );
    assert_eq!(sim.state_hash(), before);
}

#[test]
fn choices_shape_what_is_made_and_the_same_seed_makes_the_same_thing() {
    let defs = defs();
    // Each kind of choice, selected.
    let spec = person("Bram")
        .with("person.stature", Selection::Number(Real::of(1.1)))
        .with(
            "core:person.build",
            Selection::Options(vec!["heavy".to_owned()]),
        )
        .with(
            "core:person.training",
            Selection::Options(vec!["runner".to_owned(), "reader".to_owned()]),
        );
    let resolved = resolve(&defs, &spec).unwrap();
    assert_eq!(resolved.scale, Real::of(1.1));
    assert_eq!(resolved.name.as_deref(), Some("Bram"));
    assert_eq!(resolved.select.len(), 4);
    let mut sim = sim(2);
    let bram = spawn(&mut sim, &spec, on(4, 4));
    let stats = sim.components().stats.get(bram).unwrap();
    let defs_stats = sim.stat_defs().unwrap();
    let strength = defs_stats.attribute_id("core:strength").unwrap();
    assert_eq!(stats.attr(strength), Some(Real::of(12.0)));
    let athletics: TrackId = defs_stats.track_id("core:athletics").unwrap();
    assert_eq!(stats.track_level(athletics), 1);
    assert_eq!(
        sim.components().body.get(bram).unwrap().scale(),
        Real::of(1.1)
    );

    // Random selections are a function of the spec's seed alone
    // (invariant B4): two simulations, the same entity.
    let random = CharacterSpec::of(["core:person"]).random(77);
    let mut one = common::sim_over(Arc::clone(&defs), 3);
    let mut two = common::sim_over(Arc::clone(&defs), 9);
    let a = spawn(&mut one, &random, on(6, 6));
    let b = spawn(&mut two, &random, on(6, 6));
    assert_eq!(a, b);
    assert_eq!(one.hashes().components, two.hashes().components);
    assert_eq!(
        one.components().identity.get(a).unwrap().name(),
        two.components().identity.get(b).unwrap().name()
    );
    let other = resolve(&defs, &CharacterSpec::of(["core:person"]).random(78)).unwrap();
    let first = resolve(&defs, &random).unwrap();
    assert_eq!(first, resolve(&defs, &random).unwrap());
    assert!(first.select != other.select || first.name != other.name || first.scale != other.scale);
}

#[test]
fn a_creation_session_is_what_a_screen_edits() {
    let defs = defs();
    let mut session = CreationSession::new(&defs, ["core:person"]).unwrap();
    let choices = session.choices(&defs);
    let keys: Vec<&str> = choices.iter().map(|view| view.key.as_str()).collect();
    assert_eq!(
        keys,
        [
            "core:person.name",
            "core:person.stature",
            "core:person.build",
            "core:person.training"
        ]
    );
    assert!(choices.iter().all(|view| view.by_default));
    session
        .select(
            &defs,
            "core:person.training",
            Selection::Options(vec!["runner".into()]),
        )
        .unwrap();
    let training = session
        .choices(&defs)
        .into_iter()
        .find(|view| view.key == "core:person.training")
        .unwrap();
    assert_eq!(
        (training.spent, training.budget),
        (Real::of(2.0), Real::of(3.0))
    );
    // Over budget: refused, nothing changed.
    let over = session.select(
        &defs,
        "core:person.training",
        Selection::Options(vec!["runner".into(), "reader".into(), "nobody".into()]),
    );
    assert_eq!(over.unwrap_err().code, "bad_selection");
    assert_eq!(session.spec().select.len(), 1);
    assert_eq!(
        session
            .select(&defs, "core:person.wings", Selection::Number(Real::ONE))
            .unwrap_err()
            .code,
        "unknown_choice"
    );
    session
        .select(&defs, "core:person.name", Selection::Text("Cora".into()))
        .unwrap();
    let spec = session.finish(&defs).unwrap();
    assert_eq!(spec.select.len(), 4, "every choice, defaults filled in");
    // What the interface reads as `app.creation`.
    let Value::Map(view) = session.to_value(&defs) else {
        panic!("a map");
    };
    assert_eq!(view.get("complete"), Some(&Value::Bool(true)));
    let mut sim = sim(4);
    let cora = spawn(&mut sim, &spec, on(3, 3));
    assert_eq!(
        sim.components().identity.get(cora).unwrap().name(),
        Some("Cora")
    );
}

#[test]
fn one_bad_cut() {
    let mut sim = sim(5);
    let ada = spawn(&mut sim, &person("Ada"), on(10, 10));
    sim.drain_events();
    let blood = pool(&sim, "core:blood");
    let max = sim
        .components()
        .stats
        .get(ada)
        .unwrap()
        .pool_max(blood)
        .unwrap()
        .get();

    let outcome = cut(&mut sim, ada, "l_arm", 30.0);
    let Value::Map(outcome) = outcome else {
        panic!("a map")
    };
    assert_eq!(outcome.get("cancelled"), Some(&Value::Bool(false)));
    let told = sim.drain_events();
    let told_names = names(&told);
    // The hook first, then what the hit did, in order.
    let at = |name: &str| {
        told_names
            .iter()
            .position(|n| *n == name)
            .unwrap_or_else(|| panic!("no {name} in {told_names:?}"))
    };
    assert!(at("hit_landed") < at("wound_inflicted"));
    assert!(at("part_disabled") < at("slots_lost"));
    assert!(told_names.contains(&"function_changed") || !told_names.contains(&"attr_changed"));
    assert_eq!(
        told.iter().filter(|e| e.name() == "fault").count(),
        0,
        "{told_names:?}"
    );
    let body = sim.components().body.get(ada).unwrap();
    assert!(!body.part_named("l_arm").unwrap().is_working());
    assert_eq!(body.wounds().len(), 1);

    // It drains the blood on the timeline: the first mark is crossed at
    // the tick the line says.
    let rate = sim
        .components()
        .stats
        .get(ada)
        .unwrap()
        .pool_rate(blood)
        .unwrap()
        .get();
    assert!(rate < 0.0);
    let start = sim.now();
    let expected = start.0 + ((max - 0.8 * max) / -rate * 1.0e6).ceil() as i64;
    advance_to(&mut sim, start + seconds(200.0));
    let told = sim.drain_events();
    let crossed = told
        .iter()
        .find(|event| event.name() == "pool_crossed")
        .expect("the blood crossed a mark");
    assert_eq!(entity_of(crossed, "self"), Some(ada));
    assert_eq!(
        crossed.body.field("threshold").unwrap().as_real(),
        Some(Real::of(0.8))
    );
    assert!(
        (crossed.tick.0 - expected).abs() <= 1,
        "{} against {expected}",
        crossed.tick.0
    );
}

#[test]
fn hooks_may_soften_or_cancel_a_hit_and_refuse_a_death() {
    let mut sim = sim(10);
    let ada = spawn(&mut sim, &person("Ada"), on(10, 10));
    // A charm that takes almost all of a blow's energy.
    sim.on(
        "hit_incoming",
        Handler::new(|_sim: &mut Sim, delivery: &mut Delivery<'_>| {
            delivery.set_field("energy", Real::of(0.01)).unwrap();
        }),
    )
    .unwrap();
    let outcome = cut(&mut sim, ada, "l_arm", 30.0);
    let Value::Map(outcome) = outcome else {
        panic!("a map")
    };
    assert_eq!(outcome.get("absorbed"), Some(&Value::Real(Real::of(0.01))));
    assert!(sim.components().body.get(ada).unwrap().wounds().is_empty());

    // A ward that turns blows aside altogether, and one against death.
    let mut sim = sim_with_wards();
    let ada = spawn(&mut sim, &person("Ada"), on(10, 10));
    let outcome = cut(&mut sim, ada, "l_arm", 30.0);
    let Value::Map(outcome) = outcome else {
        panic!("a map")
    };
    assert_eq!(outcome.get("cancelled"), Some(&Value::Bool(true)));
    assert!(sim.components().body.get(ada).unwrap().wounds().is_empty());
    let refused = sim.die(ada, Cause::DEBUG).unwrap_err();
    assert_eq!(refused.code, "test.warded");
    assert_eq!(sim.vital(ada), Some(VitalState::Awake));
}

fn sim_with_wards() -> Sim {
    let mut sim = sim(11);
    for hook in ["hit_incoming", "entity_dying"] {
        sim.on(
            hook,
            Handler::new(|_sim: &mut Sim, delivery: &mut Delivery<'_>| {
                delivery
                    .cancel(Reason::new("test.warded", "warded"))
                    .unwrap();
            }),
        )
        .unwrap();
    }
    sim
}

#[test]
fn enough_loss_stops_a_person_and_then_ends_them() {
    let mut sim = sim(6);
    let ada = spawn(&mut sim, &person("Ada"), on(10, 10));
    cut(&mut sim, ada, "l_arm", 30.0);
    sim.drain_events();
    advance_to(&mut sim, Tick(1_000_000_000));
    let told = sim.drain_events();
    let mine: Vec<&str> = about(&told, ada).iter().map(|event| event.name()).collect();
    let at = |name: &str| {
        mine.iter()
            .position(|n| *n == name)
            .unwrap_or_else(|| panic!("no {name} in {mine:?}"))
    };
    // By the pool's own stops: unconscious below 0.6, dead below 0.4.
    assert!(at("pool_crossed") < at("entity_unconscious"));
    assert!(at("entity_unconscious") < at("entity_died"));
    assert_eq!(mine[at("entity_died") + 1], "mind_released");
    assert_eq!(sim.vital(ada), Some(VitalState::Dead));
    assert_eq!(sim.can_act(ada).unwrap_err().code, "entity_dead");
    // The body remains, with its wound.
    assert!(sim.truth().entities().contains(ada));
    assert_eq!(sim.components().body.get(ada).unwrap().wounds().len(), 1);
    // Revived, it stops again at once: it still lacks what it needs.
    sim.revive(ada, Cause::DEBUG).unwrap();
    assert_eq!(sim.vital(ada), Some(VitalState::Dead));
    assert_eq!(told.iter().filter(|e| e.name() == "fault").count(), 0);
}

#[test]
fn unconsciousness_is_refused_a_command_and_minds_move() {
    let mut sim = sim(7);
    let ada = spawn(&mut sim, &person("Ada"), on(10, 10));
    let dog = spawn(&mut sim, &CharacterSpec::of(["core:dog"]), on(12, 10));
    cut(&mut sim, ada, "l_arm", 30.0);
    // Past the first stop and short of the second.
    advance_to(&mut sim, Tick(330_000_000));
    assert_eq!(sim.vital(ada), Some(VitalState::Unconscious));
    assert_eq!(sim.can_act(ada).unwrap_err().code, "entity_unconscious");
    // A driver acts for a body whose mind does not.
    sim.set_driver(ada, Some(dog), Cause::DEBUG).unwrap();
    assert!(sim.can_act(ada).is_ok());
    sim.set_driver(ada, None, Cause::DEBUG).unwrap();
    // The mind moves with what is seated in it: pain goes, blood stays.
    let blood = pool(&sim, "core:blood");
    let ada_pain = level(&sim, ada, "core:pain");
    let dog_pain = level(&sim, dog, "core:pain");
    let ada_blood = level(&sim, ada, "core:blood");
    sim.set_control(ada, clash_sim::Control::Delegated, Cause::DEBUG)
        .unwrap();
    sim.transfer_mind(ada, dog, true, Cause::DEBUG).unwrap();
    sim.pump();
    // Control goes with the mind (06 §8.2).
    assert_eq!(
        sim.truth().entity(dog).unwrap().control(),
        clash_sim::Control::Delegated
    );
    assert_eq!(
        sim.truth().entity(ada).unwrap().control(),
        clash_sim::Control::None
    );
    assert!(sim.components().stats.get(ada).unwrap().has_pool(blood));
    assert_eq!(
        level(&sim, ada, "core:blood"),
        ada_blood,
        "blood stays with the body"
    );
    assert_eq!(
        level(&sim, dog, "core:pain"),
        ada_pain,
        "pain goes with the mind"
    );
    assert_eq!(level(&sim, ada, "core:pain"), dog_pain);
    // What stopped the mind was its body's: in a whole body it wakes,
    // and the mind that came the other way stops.
    assert_eq!(sim.vital(dog), Some(VitalState::Awake));
    assert_eq!(sim.vital(ada), Some(VitalState::Unconscious));
    let told = names(&sim.drain_events())
        .iter()
        .map(|n| n.to_string())
        .collect::<Vec<_>>();
    assert!(told.contains(&"mind_transferred".to_owned()), "{told:?}");
    assert!(told.contains(&"entity_woke".to_owned()), "{told:?}");
    // A cancelled `die` is the hook's; one with no mind is refused.
    let bare = sim
        .spawn(common::creature(), on(20, 20), Cause::DEBUG)
        .unwrap();
    assert_eq!(sim.die(bare, Cause::DEBUG).unwrap_err().code, "no_mind");
    assert_eq!(sim.can_act(bare).unwrap_err().code, "no_mind");
}

// ---- A collapse -------------------------------------------------------------

const CORNER: (i32, i32) = (4, 4);
const SIDE: i32 = 5;
const STOREYS: i32 = 6;

fn tower_sim() -> (Sim, EntityId) {
    let defs = defs();
    let world = WorldDefs::from_defs(&defs).unwrap_or_else(|d| panic!("{}", d.render()));
    let mut timeline = Timeline::new();
    let mut cosmos = Cosmos::new(Arc::clone(&world));
    let def = world.realms().lookup("core:testland").unwrap();
    cosmos.create_realm(&mut timeline, def, 7).unwrap();
    let rock = world.block_state("core:solid", None).unwrap();
    let grass = world.floor_state("core:grass", None).unwrap();
    let wall = world.block_state("core:wall", None).unwrap();
    let slab = world.floor_state("core:slab", None).unwrap();
    for (index, tract) in [TractPos::new(0, 0), TractPos::new(1, 0)]
        .into_iter()
        .enumerate()
    {
        let mut builder = ColumnBuilder::new(&world, rock, BlockState::NONE, 0);
        builder.fill_floor(0, grass);
        if index == 0 {
            for z in 0..=STOREYS {
                for y in 0..SIDE {
                    for x in 0..SIDE {
                        let cell = CellPos::new(CORNER.0 + x, CORNER.1 + y, z);
                        if z >= 1 {
                            builder.set_floor(cell, slab);
                        }
                        let edge = x == 0 || y == 0 || x == SIDE - 1 || y == SIDE - 1;
                        if z < STOREYS && edge {
                            builder.set_block(cell, wall);
                        }
                    }
                }
            }
        }
        load_column(&mut cosmos, GRID, tract, builder.build()).unwrap();
    }
    let mut sim = Sim::from_parts(defs, cosmos, timeline, 3);
    let middle = (CORNER.0 + SIDE / 2, CORNER.1 + SIDE / 2);
    let below = spawn(&mut sim, &person("Esme"), on(middle.0, middle.1));
    sim.drain_events();
    (sim, below)
}

#[test]
fn a_collapse_hurts_whoever_stands_under_it_with_its_own_energy() {
    let (mut sim, below) = tower_sim();
    // Every wall of the second storey goes.
    for y in 0..SIDE {
        for x in 0..SIDE {
            if x == 0 || y == 0 || x == SIDE - 1 || y == SIDE - 1 {
                let loc = Location::new(GRID, CellPos::new(CORNER.0 + x, CORNER.1 + y, 1));
                debug(
                    &mut sim,
                    DebugCommand::SetElement {
                        loc,
                        layer: Layer::Block,
                        state: BlockState::NONE.raw(),
                    },
                )
                .unwrap();
            }
        }
    }
    sim.advance(AdvanceLimit::UntilDecision);
    let told = sim.drain_events();
    let exposed: Vec<&SimEvent> = told
        .iter()
        .filter(|event| event.name() == "entity_exposed" && entity_of(event, "self") == Some(below))
        .collect();
    assert!(!exposed.is_empty(), "{:?}", names(&told));
    let delivered: f64 = exposed
        .iter()
        .map(|event| event.body.field("energy").unwrap().as_real().unwrap().get())
        .sum();
    // What the body's parts took came from that energy and no more.
    let landed = about(&told, below)
        .into_iter()
        .filter(|event| event.name() == "hit_landed")
        .collect::<Vec<_>>();
    assert!(!landed.is_empty(), "the collapse reached the body");
    let absorbed: f64 = landed
        .iter()
        .map(|event| {
            event
                .body
                .field("absorbed")
                .unwrap()
                .as_real()
                .unwrap()
                .get()
        })
        .sum();
    assert!(
        absorbed > 0.0 && absorbed <= delivered + 1e-6,
        "{absorbed} of {delivered}"
    );
    // Each hit followed the report of what caused it.
    let first_exposed = told
        .iter()
        .position(|e| core::ptr::eq(e, exposed[0]))
        .unwrap();
    let first_hit = told.iter().position(|e| e.name() == "hit_landed").unwrap();
    assert!(first_exposed < first_hit);
    assert_eq!(
        told.iter().filter(|e| e.name() == "fault").count(),
        0,
        "{:?}",
        names(&told)
    );
}

// ---- Coming and going -----------------------------------------------------

const START_TRACT: TractPos = TractPos::new(20, 20);

fn city_cell(tract: TractPos, dx: i32, dy: i32) -> Location {
    Location::new(
        GridId::new(0),
        CellPos::new(tract.x * TRACT_XY + dx, tract.y * TRACT_XY + dy, 0),
    )
}

fn city_sim() -> (Sim, RealmId) {
    let defs = defs();
    let world = WorldDefs::from_defs(&defs).unwrap_or_else(|d| panic!("{}", d.render()));
    let mut timeline = Timeline::new();
    let mut cosmos = Cosmos::new(Arc::clone(&world));
    let def = world.realms().lookup("core:test_city").unwrap();
    let realm = cosmos.create_realm(&mut timeline, def, 11).unwrap();
    (Sim::from_parts(defs, cosmos, timeline, 5), realm)
}

fn step(sim: &mut Sim, walker: EntityId, by: i32) {
    let Some(Place::At { loc, .. }) = sim.truth().place(walker) else {
        panic!("the walker is not in the world");
    };
    let to = Location::new(
        loc.grid,
        CellPos::new(loc.cell.x + by * TRACT_XY, loc.cell.y, loc.cell.z),
    );
    debug(
        sim,
        DebugCommand::MoveEntity {
            entity: walker,
            to: Place::at(to),
        },
    )
    .unwrap();
}

/// Each part's state and the damage of its layers.
fn damage_of(sim: &Sim, who: EntityId) -> Vec<(String, String, Vec<u8>)> {
    let body = sim.components().body.get(who).unwrap();
    body.parts()
        .iter()
        .map(|part| {
            (
                part.name().to_owned(),
                part.state().name().to_owned(),
                part.layers().iter().map(|layer| layer.damage()).collect(),
            )
        })
        .collect()
}

#[test]
fn shelving_and_restoring_a_wounded_person_changes_nothing() {
    const FAR: i32 = 11;
    let (mut sim, _realm) = city_sim();
    debug(
        &mut sim,
        DebugCommand::CreateEntity {
            spec: common::creature(),
            place: Place::Stored,
        },
    )
    .unwrap();
    let walker = sim.truth().entities().ids().next().unwrap();
    let start = city_cell(START_TRACT, 1, 1);
    debug(
        &mut sim,
        DebugCommand::StartSession {
            entity: walker,
            at: start,
        },
    )
    .unwrap();
    // A wounded person in the street, by command.
    let stands = city_cell(START_TRACT, 2, 1);
    debug(
        &mut sim,
        DebugCommand::Spawn {
            spec: person("Finn"),
            place: Place::at(stands),
        },
    )
    .unwrap();
    let finn = sim.truth().entities().ids().last().unwrap();
    assert!(sim.components().body.contains(finn));
    cut(&mut sim, finn, "l_arm", 30.0);
    let until = sim.now() + seconds(20.0);
    advance_to(&mut sim, until);
    let before = level(&sim, finn, "core:blood");
    let body_before = damage_of(&sim, finn);
    let wounds_before = sim.components().body.get(finn).unwrap().wounds().len();

    for _ in 0..FAR {
        step(&mut sim, walker, 1);
    }
    assert_eq!(
        sim.truth().place(finn),
        Some(Place::Stored),
        "shelved with its tract"
    );
    assert!(sim.components().stats.get(finn).unwrap().is_suspended());
    assert!(sim.components().body.get(finn).unwrap().is_suspended());
    // Time passes elsewhere; none of it for the shelved.
    let until = sim.now() + seconds(600.0);
    advance_to(&mut sim, until);
    for _ in 0..FAR {
        step(&mut sim, walker, -1);
    }
    assert_eq!(sim.truth().place(finn), Some(Place::at(stands)));
    assert!(!sim.components().stats.get(finn).unwrap().is_suspended());
    assert_eq!(level(&sim, finn, "core:blood"), before);
    assert_eq!(damage_of(&sim, finn), body_before);
    assert_eq!(
        sim.components().body.get(finn).unwrap().wounds().len(),
        wounds_before
    );
    assert_eq!(sim.vital(finn), Some(VitalState::Awake));
    // And it goes on bleeding from where it stood.
    let until = sim.now() + seconds(20.0);
    advance_to(&mut sim, until);
    assert!(level(&sim, finn, "core:blood") < before);
    let told = sim.drain_events();
    assert_eq!(
        told.iter().filter(|e| e.name() == "fault").count(),
        0,
        "{:?}",
        names(&told)
    );
}

#[test]
fn a_save_in_the_middle_changes_nothing() {
    let run = |save_at: Option<Tick>| {
        let mut sim = sim(8);
        let ada = spawn(&mut sim, &person("Ada"), on(10, 10));
        cut(&mut sim, ada, "l_arm", 30.0);
        sim.drain_events();
        let mut told = Vec::new();
        if let Some(at) = save_at {
            advance_to(&mut sim, at);
            told.extend(sim.drain_events());
            let (loaded, unplaced) = common::save_and_load(&sim);
            assert!(unplaced.is_empty());
            assert_eq!(loaded.hashes(), sim.hashes());
            sim = loaded;
        }
        advance_to(&mut sim, Tick(500_000_000));
        told.extend(sim.drain_events());
        let told: Vec<(String, Tick)> = told
            .iter()
            .map(|event| (event.name().to_owned(), event.tick))
            .collect();
        (sim.state_hash(), told)
    };
    let straight = run(None);
    assert_eq!(run(Some(Tick(100_000_000))), straight);
    assert_eq!(run(Some(Tick(300_000_000))), straight);
}

/// A test handler: cuts the arm of the entity the wake-up names.
fn cutter(sim: &mut Sim, _id: WakeId, wake: Wake) {
    let who = EntityId::from_bits(wake.id);
    cut(sim, who, "r_arm", 30.0);
}

#[test]
fn a_replay_reproduces_the_hash() {
    let session = |sim: &mut Sim| {
        sim.set_wake_handler(WakeDomain::Stepper, cutter);
    };
    let mut first = sim(9);
    session(&mut first);
    let spec = CharacterSpec::of(["core:person"]).random(5);
    first
        .submit(Command::Debug(DebugCommand::Spawn {
            spec,
            place: on(8, 8),
        }))
        .unwrap();
    let who = first.truth().entities().ids().last().unwrap();
    first
        .submit(Command::Debug(DebugCommand::Schedule {
            entity: Some(who),
            after: seconds(2.0),
            wake: Wake::new(WakeDomain::Stepper, who.to_bits(), 0),
        }))
        .unwrap();
    advance_to(&mut first, Tick(400_000_000));
    let log = first.take_input_log();

    let mut second = sim(9);
    session(&mut second);
    second.replay(&log).unwrap();
    advance_to(&mut second, Tick(400_000_000));
    assert_eq!(second.state_hash(), first.state_hash());
    assert_eq!(second.vital(who), first.vital(who));
}

/// A species of the tests' own, with choices of every kind, one inside
/// an option.
const SPECIES: &str = r#"[
  { "type": "template", "id": "tinker", "kind": "species", "name": "Tinker",
    "body": "humanoid", "pools": ["blood"], "mind": "careful",
    "tags": ["tinker"], "items": ["core:toolkit"], "traits": ["core:steady_hands"],
    "variables": { "guild": "'bellows'" },
    "choices": [
      { "id": "craft", "kind": "pick", "default": ["smith"],
        "options": [
          { "id": "smith", "traits": ["core:burn_scars"],
            "choices": [ { "id": "metal", "kind": "pick", "options": [ { "id": "iron" }, { "id": "bronze" } ] } ] },
          { "id": "weaver", "items": ["core:loom"] } ] },
      { "id": "spares", "kind": "assemble", "budget": 2, "parts": ["arm", "hand"] },
      { "id": "motto", "kind": "text", "sets": "description", "default": "measure twice" }
    ] }
]"#;

#[test]
fn every_kind_of_choice_takes_what_it_should_and_refuses_the_rest() {
    let defs = common::life_defs(SPECIES);
    let spec = CharacterSpec::of(["tinker"])
        .with(
            "tinker.craft.smith.metal",
            Selection::Options(vec!["bronze".into()]),
        )
        .with(
            "core:tinker.spares",
            Selection::Options(vec!["arm".into(), "core:hand".into()]),
        );
    let resolved = resolve(&defs, &spec).unwrap();
    assert_eq!(resolved.assembled.len(), 2);
    assert_eq!(resolved.traits, ["core:steady_hands", "core:burn_scars"]);
    assert_eq!(resolved.items, ["core:toolkit"]);
    assert_eq!(resolved.description.as_deref(), Some("measure twice"));
    assert_eq!(resolved.mind, Some(Some("careful".to_owned())));
    assert!(
        resolved
            .select
            .contains_key("core:tinker.craft.smith.metal")
    );
    // A nested choice is closed while its option is not chosen.
    let weaver = CharacterSpec::of(["tinker"])
        .with(
            "core:tinker.craft",
            Selection::Options(vec!["weaver".into()]),
        )
        .with(
            "core:tinker.craft.smith.metal",
            Selection::Options(vec!["iron".into()]),
        );
    assert_eq!(
        resolve(&defs, &weaver).unwrap_err().code(),
        "unknown_choice"
    );
    // Over budget, and a part it does not allow.
    for parts in [vec!["arm", "arm", "hand"], vec!["leg"]] {
        let spec = CharacterSpec::of(["tinker"]).with(
            "core:tinker.spares",
            Selection::Options(parts.into_iter().map(str::to_owned).collect()),
        );
        assert_eq!(resolve(&defs, &spec).unwrap_err().code(), "bad_selection");
    }
    // Made, it is told to whoever gives traits and items.
    let mut sim = common::sim_over(defs, 12);
    let made = sim.spawn_template(&spec, on(2, 2), Cause::DEBUG).unwrap();
    assert_eq!(made.items, ["core:toolkit"]);
    sim.pump();
    let told = sim.drain_events();
    let spawned = told
        .iter()
        .find(|event| event.name() == "entity_spawned")
        .unwrap();
    assert!(spawned.body.field("traits").is_some());
    assert_eq!(sim.mind(made.entity).unwrap().profile(), Some("careful"));
    assert_eq!(
        sim.var(Subject::Entity(made.entity), "guild"),
        Value::from("bellows")
    );
}

#[test]
fn thrown_into_a_wall_a_person_is_hurt_by_the_impact() {
    use clash_world::{Body, BodyOwner};
    let mut sim = sim(13);
    let wall = common::wall(&sim);
    for y in 8..13 {
        debug(
            &mut sim,
            DebugCommand::SetElement {
                loc: Location::new(GRID, CellPos::new(14, y, 0)),
                layer: Layer::Block,
                state: wall.raw(),
            },
        )
        .unwrap();
    }
    let ada = spawn(&mut sim, &person("Ada"), on(10, 10));
    sim.drain_events();
    let mass = sim.components().body.get(ada).unwrap().mass();
    let r = Real::of;
    let body = Body::of_mass(
        BodyOwner::Entity(ada),
        GRID,
        [r(10.5), r(10.5), r(1.0)],
        [r(20.0), r(0.0), r(0.0)],
        mass,
        Cause::DEBUG,
    );
    sim.world(|ctx| ctx.launch(body)).unwrap();
    advance_to(&mut sim, Tick(3_000_000));
    let told = sim.drain_events();
    let all = names(&told);
    let impact = all.iter().position(|name| *name == "body_impact");
    assert!(impact.is_some(), "{all:?}");
    let landed = told
        .iter()
        .position(|event| event.name() == "hit_landed" && entity_of(event, "self") == Some(ada));
    assert!(landed.is_some_and(|at| at > impact.unwrap()), "{all:?}");
    assert_eq!(
        told.iter().filter(|e| e.name() == "fault").count(),
        0,
        "{all:?}"
    );
}
