//! Perception against the real simulation in the test city (15 §3 to
//! §7, §13): signals and their models, delay, detection, percepts,
//! beliefs and what the party perceives. Entities are made by debug
//! commands; senses are attached through the package's own functions.

mod common;

use std::path::PathBuf;
use std::sync::Arc;

use clash_core::{Dur, EntityId, Real};
use clash_data::{Defs, Loader};
use clash_sim::perception::{
    self, PartyPerception, SenseId, Signal, SignalEvent, SignalKindId, signal::Spot,
};
use clash_sim::{AdvanceLimit, Control, DebugCommand, EntitySpec, Place, Sim, SimEvent, Subject};
use clash_time::{Timeline, Wake, WakeDomain, WakeId};
use clash_world::dims::TRACT_XY;
use clash_world::{
    BlockState, Cause, CellPos, Cosmos, GridId, Layer, Location, MoveRules, OccupantKind,
    PathLimits, TractPos, WorldDefs,
};

/// The tract the tests stand in: well inside the city.
const START: TractPos = TractPos::new(20, 20);

/// The base grid of the city.
const GRID: GridId = GridId::new(0);

/// The core content files of a directory of `mods/core/data`, as paths
/// within the mod and their text.
fn core_files(dir: &str) -> Vec<(String, String)> {
    let path = common::mods_dir().join("core/data").join(dir);
    let mut files: Vec<PathBuf> = std::fs::read_dir(&path)
        .unwrap_or_else(|error| panic!("{}: {error}", path.display()))
        .filter_map(|entry| entry.ok().map(|entry| entry.path()))
        .filter(|path| path.extension().is_some_and(|ext| ext == "json"))
        .collect();
    files.sort();
    files
        .iter()
        .map(|file| {
            let name = file.file_name().unwrap_or_default().to_string_lossy();
            (
                format!("data/{dir}/{name}"),
                std::fs::read_to_string(file).unwrap(),
            )
        })
        .collect()
}

/// `mods/core`'s world and perception content, with one more file.
fn try_load(extra: &str) -> Result<Arc<Defs>, String> {
    let types = common::types();
    let mut files = core_files("world");
    files.extend(core_files("perception"));
    files.push(("data/test/extra.json".to_owned(), extra.to_owned()));
    let files: Vec<(&str, &str)> = files
        .iter()
        .map(|(path, text)| (path.as_str(), text.as_str()))
        .collect();
    Loader::new(&types)
        .expr_registry(&clash_sim::engine_registry())
        .memory_mod("core", r#"{ "id": "core" }"#, &files)
        .load_mods(&["core"])
        .into_result()
        .map_err(|diagnostics| diagnostics.render())
}

fn load(extra: &str) -> Arc<Defs> {
    try_load(extra).unwrap_or_else(|error| panic!("the definitions do not load:\n{error}"))
}

/// What the tests add: a sense that smells smoke, and one that hears
/// keenly through a body function.
const EXTRA: &str = r#"[
  { "type": "signal_kind", "id": "smell", "model": "field", "field": "smoke" },
  { "type": "sense", "id": "nose", "signal": "smell", "range": 1, "threshold": 0.1 },
  { "type": "body_function", "id": "hearing" },
  { "type": "sense", "id": "ears", "signal": "sound", "range": 40,
    "threshold": 0.001, "function": "hearing" },
  { "type": "event_signal", "id": "building", "event": "element_changed",
    "signal": "sound", "strength": 5 }
]"#;

/// A cell of the street that runs east along the north edge of the
/// start tract, `dx` cells east of its corner.
fn street(dx: i32) -> Location {
    Location::new(
        GRID,
        CellPos::new(START.x * TRACT_XY + dx, START.y * TRACT_XY + 1, 0),
    )
}

/// A simulation over the test city with the tracts round the start
/// loaded and hot: a session is started for someone two tracts south,
/// out of every sense's range.
fn city(extra: &str) -> Sim {
    let defs = load(extra);
    let world = WorldDefs::from_defs(&defs).unwrap_or_else(|d| panic!("{}", d.render()));
    let mut timeline = Timeline::new();
    let mut cosmos = Cosmos::new(Arc::clone(&world));
    let def = world.realms().lookup("core:test_city").unwrap();
    cosmos.create_realm(&mut timeline, def, 11).unwrap();
    let mut sim = Sim::from_parts(defs, cosmos, timeline, 5);
    let anchor = create(&mut sim, common::creature(), Place::Stored);
    let far = Location::new(
        GRID,
        CellPos::new(START.x * TRACT_XY + 1, (START.y + 3) * TRACT_XY + 1, 0),
    );
    common::debug(
        &mut sim,
        DebugCommand::StartSession {
            entity: anchor,
            at: far,
        },
    )
    .unwrap();
    // The street the tests use is open and clear.
    for dx in 0..48 {
        let cell = sim.truth().cosmos().cell(street(dx)).unwrap();
        assert!(
            cell.block.is_none() && cell.fixture.is_none(),
            "street({dx}) is not clear"
        );
    }
    sim.drain_events();
    sim
}

/// Makes an entity by debug command.
fn create(sim: &mut Sim, spec: EntitySpec, place: Place) -> EntityId {
    let id = sim.truth().entities().next_id().unwrap();
    common::debug(sim, DebugCommand::CreateEntity { spec, place }).unwrap();
    id
}

/// A person standing in a cell.
fn person(sim: &mut Sim, at: Location) -> EntityId {
    create(sim, common::creature(), Place::at(at))
}

fn sense(sim: &Sim, name: &str) -> SenseId {
    sim.perception_defs().sense_named(name).unwrap()
}

fn kind(sim: &Sim, name: &str) -> SignalKindId {
    sim.perception_defs().kind_named(name).unwrap()
}

/// Gives an entity a sense, by name.
fn give(sim: &mut Sim, entity: EntityId, name: &str) {
    let sense = sense(sim, name);
    perception::attach_sense(sim, entity, sense).unwrap();
}

fn set_block(sim: &mut Sim, at: Location, block: &str) {
    let state = sim
        .truth()
        .cosmos()
        .defs()
        .block_state(block, None)
        .unwrap()
        .raw();
    common::debug(
        sim,
        DebugCommand::SetElement {
            loc: at,
            layer: Layer::Block,
            state,
        },
    )
    .unwrap();
}

fn advance_by(sim: &mut Sim, secs: f64) {
    let until = sim.now() + Dur::from_secs_real(Real::of(secs));
    sim.advance(AdvanceLimit::Until(until));
}

fn named<'a>(events: &'a [SimEvent], name: &str) -> Vec<&'a SimEvent> {
    events.iter().filter(|event| event.name() == name).collect()
}

fn entity_of(event: &SimEvent, name: &str) -> Option<EntityId> {
    event.subject(name).and_then(|subject| subject.entity())
}

fn field_real(event: &SimEvent, name: &str) -> Option<Real> {
    event.body.field(name).and_then(|value| value.as_real())
}

fn close(a: Real, b: Real) -> bool {
    (a - b).abs() <= Real::of(1e-9) * b.abs().max(Real::ONE)
}

// ---- S2: the three models, by hand ------------------------------------

#[test]
fn the_line_model_matches_hand_computed_cases() {
    let mut sim = city(EXTRA);
    let sight = kind(&sim, "sight");
    let info = sim.perception_defs().kind(sight).unwrap().clone();
    let light = perception::signal::light_now(&sim, street(10));
    assert!(light > Real::ZERO, "the street is lit");
    let eye = Spot::point(street(0), 1500);
    let body = Spot {
        loc: street(10),
        alt_mm: 0,
        height_mm: 1700,
    };
    // In the open: strength × exposed × light / max(1, d)^1.
    let open = perception::signal::strength_at(&sim, &info, Real::ONE, body, eye);
    assert!(
        close(open, light / Real::of(10.0)),
        "{open} against {light}/10"
    );
    // Closer than a metre counts as a metre.
    let near = Spot {
        loc: street(0),
        ..body
    };
    let at_hand = perception::signal::strength_at(&sim, &info, Real::of(2.0), near, eye);
    assert!(close(at_hand, Real::of(2.0) * light));
    // A wall between hides the whole body.
    set_block(&mut sim, street(5), "wall");
    assert_eq!(
        perception::signal::strength_at(&sim, &info, Real::ONE, body, eye),
        Real::ZERO
    );
    // A window does not: glass is not opaque.
    set_block(&mut sim, street(5), "window");
    let through_glass = perception::signal::strength_at(&sim, &info, Real::ONE, body, eye);
    assert!(close(through_glass, light / Real::of(10.0)));
}

#[test]
fn the_through_model_matches_hand_computed_cases() {
    let mut sim = city(EXTRA);
    let sound = kind(&sim, "sound");
    let info = sim.perception_defs().kind(sound).unwrap().clone();
    let ear = Spot::point(street(10), 1500);
    let source = Spot::point(street(0), 1500);
    // In the open: strength / max(1, d)^2.
    let open = perception::signal::strength_at(&sim, &info, Real::of(10.0), source, ear);
    assert!(close(open, Real::of(10.0) / Real::of(100.0)), "{open}");
    // A concrete wall (`mineral`: 0.8) closing the whole cell.
    let wall = sim
        .truth()
        .cosmos()
        .defs()
        .block_state("wall", None)
        .unwrap();
    let coverage = sim.truth().cosmos().defs().block_info(wall).coverage;
    set_block(&mut sim, street(4), "wall");
    let walled = perception::signal::strength_at(&sim, &info, Real::of(10.0), source, ear);
    let expected = Real::of(0.1) * (Real::ONE - Real::of(0.8) * coverage);
    assert!(close(walled, expected), "{walled} against {expected}");
    // And a window of glass (0.5 by name, though glass is `mineral`).
    set_block(&mut sim, street(7), "window");
    let window = sim
        .truth()
        .cosmos()
        .defs()
        .block_state("window", None)
        .unwrap();
    let glass = sim.truth().cosmos().defs().block_info(window).coverage;
    let both = perception::signal::strength_at(&sim, &info, Real::of(10.0), source, ear);
    let expected = expected * (Real::ONE - Real::of(0.5) * glass);
    assert!(close(both, expected), "{both} against {expected}");
    // The same both ways.
    let back = perception::signal::strength_at(&sim, &info, Real::of(10.0), ear, source);
    assert!(close(back, both));
}

#[test]
fn the_field_model_reads_the_observers_own_cell() {
    let mut sim = city(EXTRA);
    let dog = person(&mut sim, street(20));
    give(&mut sim, dog, "nose");
    sim.drain_events();
    let smoke = sim
        .truth()
        .cosmos()
        .defs()
        .fields()
        .lookup("core:smoke")
        .unwrap();
    // A wisp in the cell beside: the nose reads its own cell, nothing.
    sim.world(|ctx| ctx.edit(Cause::DEBUG).set_field(street(21), smoke, 200))
        .unwrap();
    perception::search(&mut sim, dog, Real::ONE).unwrap();
    sim.pump();
    assert!(named(sim.undrained_events(), "perceived").is_empty());
    // Smoke in its own cell over a tenth of full is smelt, and seems to
    // come from the neighbour with more.
    sim.world(|ctx| ctx.edit(Cause::DEBUG).set_field(street(20), smoke, 60))
        .unwrap();
    perception::search(&mut sim, dog, Real::ONE).unwrap();
    sim.pump();
    let events = sim.drain_events();
    let perceived = named(&events, "perceived");
    assert_eq!(perceived.len(), 1);
    let clarity = field_real(perceived[0], "clarity").unwrap();
    assert!(close(
        clarity,
        Real::of(60.0) / Real::of(255.0) / Real::of(0.1)
    ));
    assert_eq!(
        perceived[0].body.field("where"),
        Some(clash_sim::FieldValue::Subject(Subject::Loc(street(21))))
    );
    // It is not perceived again while it stays over the threshold.
    perception::search(&mut sim, dog, Real::ONE).unwrap();
    sim.pump();
    assert!(named(sim.undrained_events(), "perceived").is_empty());
}

#[test]
fn a_field_signal_cannot_be_given_off() {
    let mut sim = city(EXTRA);
    let smell = kind(&sim, "smell");
    let refused = perception::emit_signal(
        &mut sim,
        Signal::new(smell, street(3), Real::ONE, Cause::DEBUG),
    )
    .unwrap_err();
    assert_eq!(refused.code, perception::reason::FIELD_NOT_SENT);
}

#[test]
fn a_creature_without_a_working_function_does_not_receive() {
    let mut sim = city(EXTRA);
    let listener = person(&mut sim, street(10));
    give(&mut sim, listener, "ears");
    // With no body, the function is not asked: it hears.
    let sound = kind(&sim, "sound");
    let loud = Signal::new(sound, street(12), Real::of(5.0), Cause::DEBUG);
    let ears = sense(&sim, "ears");
    assert!(perception::percept::reading_of(&sim, &loud, listener, ears).is_some());
    // With a body that no part of provides the function, it does not:
    // by data, not by naming ears.
    let clock = sim.truth().clock_of(listener).unwrap();
    let body = clash_sim::body::Body::new(clash_sim::body::BodyPlanId::new(0), Real::ONE, clock);
    sim.components_mut().body.insert(listener, body);
    assert!(perception::percept::reading_of(&sim, &loud, listener, ears).is_none());
    perception::emit_signal(&mut sim, loud).unwrap();
    advance_by(&mut sim, 0.2);
    assert!(named(&sim.drain_events(), "perceived").is_empty());
}

// ---- S2: delay against a fast clock (test script `slow_sound`) -------

/// A wake-up handler that moves an entity one cell east each time,
/// `arg` more times, every millisecond of its own time.
fn sprinter(sim: &mut Sim, _id: WakeId, wake: Wake) {
    let entity = EntityId::from_bits(wake.id);
    let Some(Place::At { loc, .. }) = sim.truth().place(entity) else {
        return;
    };
    let to = Location::new(
        loc.grid,
        CellPos::new(loc.cell.x + 1, loc.cell.y, loc.cell.z),
    );
    let _ = sim.relocate(entity, Place::at(to), Cause::DEBUG.by(entity));
    if wake.arg > 0 {
        let _ = sim.schedule_for(
            entity,
            Dur::from_millis(10),
            Wake::new(WakeDomain::Stepper, wake.id, wake.arg - 1),
        );
    }
}

#[test]
fn slow_sound() {
    let mut sim = city(EXTRA);
    sim.set_wake_handler(WakeDomain::Stepper, sprinter);
    let listener = person(&mut sim, street(34));
    give(&mut sim, listener, "human_hearing");
    // A speedster beside the sound, on a clock a hundred times fast:
    // one cell each 10 ms of its time is 0.1 ms of everyone else's.
    let speedster = person(&mut sim, street(1));
    sim.set_time_rate(speedster, Some(Real::of(100.0)), Cause::DEBUG)
        .unwrap();
    sim.drain_events();
    let sound = kind(&sim, "sound");
    let start = sim.now();
    let emitted = perception::emit_signal(
        &mut sim,
        Signal::new(sound, street(0), Real::of(50.0), Cause::DEBUG).carrying(SignalEvent {
            name: "crash".to_owned(),
            actor: None,
            salience: 0.0.into_real(),
        }),
    )
    .unwrap();
    assert_eq!(emitted.received, 0, "nothing hears it at once");
    assert!(emitted.delayed >= 1);
    sim.schedule_for(
        speedster,
        Dur::ZERO,
        Wake::new(WakeDomain::Stepper, speedster.to_bits(), 32),
    )
    .unwrap();
    advance_by(&mut sim, 0.2);
    let events = sim.drain_events();
    // The speedster's footsteps are heard as well; the crash once.
    let heard: Vec<&SimEvent> = named(&events, "perceived")
        .into_iter()
        .filter(|event| {
            entity_of(event, "self") == Some(listener)
                && event.body.field("event").and_then(|v| v.plain().cloned())
                    == Some(clash_core::Value::from("crash"))
        })
        .collect();
    assert_eq!(heard.len(), 1, "the listener hears it once");
    // It arrives after distance over the speed of sound: 34 m at 343 m/s.
    let expected = Dur::from_secs_real(Real::of(34.0) / Real::of(343.0));
    let took = heard[0].tick.since(start);
    assert!(
        (took.as_micros() - expected.as_micros()).abs() <= 1,
        "{took:?} against {expected:?}"
    );
    // The speedster was there first.
    let arrived = events
        .iter()
        .find(|event| {
            event.name() == "entity_relocated"
                && entity_of(event, "self") == Some(speedster)
                && event.subject("here") == Some(Subject::Loc(street(33)))
        })
        .expect("the speedster reached the listener");
    assert!(arrived.tick < heard[0].tick);
}

trait IntoReal {
    fn into_real(self) -> Real;
}

impl IntoReal for f64 {
    fn into_real(self) -> Real {
        Real::of(self)
    }
}

// ---- S3: detection -----------------------------------------------------

/// A watcher facing east along the street with sight, and someone
/// standing `dx` cells east of it.
fn watcher_and_target(sim: &mut Sim, dx: i32) -> (EntityId, EntityId) {
    let watcher = person(sim, street(0));
    give(sim, watcher, "human_sight");
    perception::set_facing(sim, watcher, Real::of(90.0)).unwrap();
    let target = person(sim, street(dx));
    sim.drain_events();
    (watcher, target)
}

#[test]
fn a_strong_signal_detects_at_once() {
    let mut sim = city(EXTRA);
    let (watcher, target) = watcher_and_target(&mut sim, 5);
    // One interval holds exactly one sweep.
    advance_by(&mut sim, 0.5);
    let events = sim.drain_events();
    let detected = named(&events, "detected");
    assert_eq!(detected.len(), 1);
    assert_eq!(entity_of(detected[0], "self"), Some(watcher));
    assert_eq!(entity_of(detected[0], "about"), Some(target));
    assert!(perception::perceives(&sim, watcher, target));
    assert_eq!(
        perception::believed_loc(&sim, watcher, target),
        Some(street(5))
    );
}

#[test]
fn a_marginal_signal_takes_the_time_the_rates_imply() {
    let mut sim = city(EXTRA);
    let (watcher, target) = watcher_and_target(&mut sim, 40);
    let sight = sense(&sim, "human_sight");
    let light = perception::signal::light_now(&sim, street(40));
    // ratio = light / 40 / 0.02; each sweep adds 4 × (ratio − 1 + 0.1) × 0.5.
    let ratio = light / Real::of(40.0) / Real::of(0.02);
    assert!(ratio > Real::ONE && ratio < Real::of(1.5), "ratio {ratio}");
    let per_sweep = Real::of(4.0) * (ratio - Real::ONE + Real::of(0.1)) * Real::of(0.5);
    let sweeps = (Real::ONE / per_sweep).ceil().to_i32();
    assert!(sweeps > 1);
    let mut seen_after = None;
    for sweep in 1..=sweeps + 2 {
        advance_by(&mut sim, 0.5);
        if seen_after.is_none() && perception::perceives(&sim, watcher, target) {
            seen_after = Some(sweep);
        }
        if sweep < sweeps {
            let level = sim
                .components()
                .awareness
                .get(watcher)
                .and_then(|awareness| awareness.level(target, sight))
                .unwrap();
            assert!(close(level, per_sweep * Real::from(sweep)), "{level}");
        }
    }
    assert_eq!(seen_after, Some(sweeps));
}

#[test]
fn what_is_behind_the_watcher_or_in_the_dark_is_not_seen() {
    let mut sim = city(EXTRA);
    let (watcher, target) = watcher_and_target(&mut sim, 5);
    perception::set_facing(&mut sim, watcher, Real::of(270.0)).unwrap();
    advance_by(&mut sim, 2.0);
    assert!(perception::awareness_of(&sim, watcher, target).is_none());
    // Facing it, it is seen.
    perception::face_toward(&mut sim, watcher, street(5)).unwrap();
    advance_by(&mut sim, 0.5);
    assert!(perception::perceives(&sim, watcher, target));
}

/// The awareness of every observer of every target, by their places in
/// a list, for comparing runs that made them in other orders.
fn awareness_by_role(sim: &Sim, people: &[EntityId]) -> Vec<(usize, usize, Real)> {
    let mut out = Vec::new();
    for (i, observer) in people.iter().enumerate() {
        for (j, target) in people.iter().enumerate() {
            if let Some(level) = perception::awareness_of(sim, *observer, *target) {
                out.push((i, j, level));
            }
        }
    }
    out
}

#[test]
fn detection_does_not_depend_on_evaluation_order() {
    // Invariant P4: the same people, made and given senses in two
    // orders, come to the same awareness of each other.
    let spots = [0, 6, 12, 25, 41];
    let run = |order: &[usize]| {
        let mut sim = city(EXTRA);
        let mut people = vec![EntityId::new(0, 0); spots.len()];
        for &at in order {
            people[at] = person(&mut sim, street(spots[at]));
        }
        for &at in order.iter().rev() {
            give(&mut sim, people[at], "human_sight");
            give(&mut sim, people[at], "human_hearing");
            let facing = if at % 2 == 0 { 90.0 } else { 270.0 };
            perception::set_facing(&mut sim, people[at], Real::of(facing)).unwrap();
        }
        advance_by(&mut sim, 3.0);
        awareness_by_role(&sim, &people)
    };
    let forward = run(&[0, 1, 2, 3, 4]);
    let backward = run(&[4, 3, 2, 1, 0]);
    let shuffled = run(&[2, 0, 4, 1, 3]);
    assert!(!forward.is_empty());
    assert_eq!(forward, backward);
    assert_eq!(forward, shuffled);
}

#[test]
fn the_locker() {
    let mut sim = city(EXTRA);
    let searcher = person(&mut sim, street(0));
    give(&mut sim, searcher, "human_sight");
    give(&mut sim, searcher, "human_hearing");
    perception::set_facing(&mut sim, searcher, Real::of(90.0)).unwrap();
    let hider = person(&mut sim, street(8));
    let locker = create(
        &mut sim,
        EntitySpec::new(OccupantKind::Item),
        Place::at(street(9)),
    );
    sim.drain_events();
    advance_by(&mut sim, 1.0);
    assert!(
        perception::perceives(&sim, searcher, hider),
        "seen in the open"
    );
    // The hider steps into the locker: not seen any more.
    common::debug(
        &mut sim,
        DebugCommand::MoveEntity {
            entity: hider,
            to: Place::In(locker),
        },
    )
    .unwrap();
    let sight = sense(&sim, "human_sight");
    let hearing = sense(&sim, "human_hearing");
    advance_by(&mut sim, 5.0);
    let events = sim.drain_events();
    let lost = named(&events, "lost_track");
    assert!(!lost.is_empty(), "the searcher loses it");
    assert!(!perception::perceives(&sim, searcher, hider));
    let awareness = sim.components().awareness.get(searcher);
    assert!(awareness.and_then(|a| a.level(hider, sight)).is_none());
    // The track ends where it was last perceived.
    let last = perception::believed_loc(&sim, searcher, hider).unwrap();
    assert!(last == street(8) || last == street(9), "{last:?}");
    // It makes a noise: heard, through the locker's walls, where the
    // locker is.
    let sound = kind(&sim, "sound");
    perception::emit_signal(
        &mut sim,
        Signal::new(sound, street(9), Real::of(0.5), Cause::DEBUG.by(hider)).about(hider),
    )
    .unwrap();
    advance_by(&mut sim, 0.1);
    let events = sim.drain_events();
    let heard: Vec<&SimEvent> = named(&events, "perceived")
        .into_iter()
        .filter(|event| {
            entity_of(event, "self") == Some(searcher)
                && event.body.field("percept").and_then(|v| v.plain().cloned())
                    == Some(clash_core::Value::from("signal"))
        })
        .collect();
    assert_eq!(heard.len(), 1);
    assert_eq!(
        heard[0]
            .body
            .field("signal")
            .and_then(|v| v.plain().cloned()),
        Some(clash_core::Value::from("core:sound"))
    );
    assert!(
        sim.components()
            .awareness
            .get(searcher)
            .and_then(|a| a.level(hider, hearing))
            .is_some()
    );
    assert_eq!(
        perception::believed_loc(&sim, searcher, hider),
        Some(street(9))
    );
    // Still not seen.
    advance_by(&mut sim, 1.0);
    assert!(
        sim.components()
            .awareness
            .get(searcher)
            .and_then(|a| a.level(hider, sight))
            .is_none()
    );
}

// ---- S4: percepts --------------------------------------------------------

#[test]
fn heard_not_seen() {
    // A wall breaks round a corner: the guard hears it, cannot see who
    // did it, and the percept carries what a rule needs to send it to
    // look.
    let mut sim = city(EXTRA);
    let guard = person(&mut sim, street(0));
    give(&mut sim, guard, "human_sight");
    give(&mut sim, guard, "human_hearing");
    perception::set_facing(&mut sim, guard, Real::of(90.0)).unwrap();
    // A wall, then a window, then a vandal: out of sight.
    set_block(&mut sim, street(4), "wall");
    set_block(&mut sim, street(8), "window");
    let vandal = person(&mut sim, street(9));
    advance_by(&mut sim, 1.0);
    assert!(!perception::perceives(&sim, guard, vandal));
    sim.drain_events();
    // The vandal breaks the window.
    let impact = sim
        .truth()
        .cosmos()
        .defs()
        .damage_types()
        .lookup("core:impact")
        .unwrap();
    let packet = clash_world::DamagePacket::new(
        impact,
        Real::of(1.0e6),
        Real::of(1.0e9),
        Cause::DEBUG.by(vandal),
    );
    sim.world(|ctx| {
        ctx.edit(Cause::DEBUG.by(vandal))
            .damage(street(8), Layer::Block, packet)
    })
    .unwrap();
    advance_by(&mut sim, 0.2);
    let events = sim.drain_events();
    assert_eq!(named(&events, "element_destroyed").len(), 1);
    let percept = named(&events, "perceived")
        .into_iter()
        .find(|event| {
            entity_of(event, "self") == Some(guard)
                && event.body.field("event").and_then(|v| v.plain().cloned())
                    == Some(clash_core::Value::from("element_destroyed"))
        })
        .expect("the guard perceived the wall breaking");
    // What a rule on `perceived` filters and reads.
    assert_eq!(
        percept
            .body
            .field("signal")
            .and_then(|v| v.plain().cloned()),
        Some(clash_core::Value::from("core:sound"))
    );
    assert_eq!(
        percept.body.field("sense").and_then(|v| v.plain().cloned()),
        Some(clash_core::Value::from("core:human_hearing"))
    );
    assert_eq!(
        percept.body.field("where"),
        Some(clash_sim::FieldValue::Subject(Subject::Loc(street(8))))
    );
    assert!(field_real(percept, "clarity").unwrap() > Real::ONE);
    // Whoever did it was not made out; the cause carries no actor.
    assert_eq!(
        percept.body.field("about_actor"),
        Some(clash_sim::FieldValue::Null)
    );
    assert_eq!(percept.subject("actor"), None);
    // The guard remembers it.
    let memories = sim.components().beliefs.get(guard).unwrap().memories();
    assert!(
        memories
            .iter()
            .any(|memory| memory.event == "element_destroyed")
    );
}

/// The rule of `heard_not_seen`, as it can be written today: it sends
/// the character to look by noting where (the action it would enqueue is
/// the actions package's). The fixture under `tests/fixtures` holds it
/// as 15 §12 writes it.
const HEARD_NOT_SEEN_RULE: &str = r#"[
  { "type": "variable", "id": "investigate_at", "scope": "entity", "value": "loc" },
  { "type": "eoc", "id": "investigate_sounds", "on": "perceived", "attached": true,
    "filter": { "event": "element_destroyed", "signal": "core:sound" },
    "if": "event.clarity > 1 and event.about_actor == null",
    "do": [ { "set": { "self.var.investigate_at": "event.where" } } ] }
]"#;

#[test]
fn the_rule_of_heard_not_seen_loads_and_reads_only_belief() {
    // It loads, and the linter has nothing to say of it: it reads the
    // percept and nothing true about anyone.
    let types = common::types();
    let mut files = core_files("world");
    files.extend(core_files("perception"));
    files.push((
        "data/test/extra.json".to_owned(),
        HEARD_NOT_SEEN_RULE.to_owned(),
    ));
    let files: Vec<(&str, &str)> = files
        .iter()
        .map(|(path, text)| (path.as_str(), text.as_str()))
        .collect();
    let outcome = Loader::new(&types)
        .expr_registry(&clash_sim::engine_registry())
        .memory_mod("core", r#"{ "id": "core" }"#, &files)
        .load_mods(&["core"]);
    let rendered = outcome.diagnostics.render();
    assert!(outcome.defs.is_some(), "{rendered}");
    assert!(!rendered.contains("decides what it does"), "{rendered}");
    let fixture = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/perception/heard_not_seen.json"),
    )
    .unwrap();
    assert!(fixture.contains("\"perceived\"") && fixture.contains("investigate"));
}

#[test]
fn a_window_is_noticed_only_by_who_perceives_its_source() {
    let mut sim = city(EXTRA);
    let (watcher, target) = watcher_and_target(&mut sim, 5);
    let blind = person(&mut sim, street(3));
    assert!(!perception::notices_source(&sim, watcher, Some(target)));
    advance_by(&mut sim, 0.5);
    assert!(perception::notices_source(&sim, watcher, Some(target)));
    assert!(!perception::notices_source(&sim, blind, Some(target)));
    assert!(perception::notices_source(&sim, blind, None));
    assert!(perception::notices_source(&sim, target, Some(target)));
}

#[test]
fn could_perceive_answers_for_the_observer_now() {
    let mut sim = city(EXTRA);
    let (watcher, _) = watcher_and_target(&mut sim, 5);
    let sight = kind(&sim, "sight");
    assert!(perception::could_perceive(
        &sim,
        watcher,
        street(10),
        0,
        sight
    ));
    // Behind it, no.
    let behind = Location::new(
        GRID,
        CellPos::new(street(0).cell.x - 5, street(0).cell.y, 0),
    );
    assert!(!perception::could_perceive(&sim, watcher, behind, 0, sight));
    // Through a wall, no.
    set_block(&mut sim, street(7), "wall");
    assert!(!perception::could_perceive(
        &sim,
        watcher,
        street(10),
        0,
        sight
    ));
}

// ---- S5: beliefs -------------------------------------------------------

#[test]
fn belief_properties_are_null_for_the_never_perceived() {
    let mut sim = city(EXTRA);
    let (watcher, target) = watcher_and_target(&mut sim, 5);
    let ask = |sim: &Sim, name: &str| {
        let x = sim.subject_value(Subject::Entity(target));
        sim.property_named(Subject::Entity(watcher), name, &[x])
    };
    use clash_expr::Value;
    for name in [
        "believed_loc",
        "last_seen",
        "last_seen_at",
        "confidence",
        "awareness",
        "seen_how",
    ] {
        assert_eq!(ask(&sim, name), Value::Null, "{name}");
    }
    assert_eq!(ask(&sim, "knows"), Value::Bool(false));
    assert_eq!(ask(&sim, "perceives"), Value::Bool(false));
    advance_by(&mut sim, 0.5);
    assert_eq!(ask(&sim, "knows"), Value::Bool(true));
    assert_eq!(ask(&sim, "perceives"), Value::Bool(true));
    assert_eq!(
        ask(&sim, "believed_loc"),
        sim.subject_value(Subject::Loc(street(5)))
    );
    assert_eq!(ask(&sim, "seen_how"), Value::from("core:sight"));
    // Full when perceived, a little less a moment later.
    let fresh = ask(&sim, "confidence").as_number().unwrap();
    assert!(fresh > Real::of(0.99) && fresh <= Real::ONE, "{fresh}");
    // Time passes and the target walks off unseen: the track goes stale.
    perception::set_facing(&mut sim, watcher, Real::of(270.0)).unwrap();
    common::debug(
        &mut sim,
        DebugCommand::MoveEntity {
            entity: target,
            to: Place::at(street(30)),
        },
    )
    .unwrap();
    advance_by(&mut sim, 60.0);
    assert_eq!(
        ask(&sim, "believed_loc"),
        sim.subject_value(Subject::Loc(street(5)))
    );
    let confidence = ask(&sim, "confidence").as_number().unwrap();
    assert!(confidence < Real::of(0.6), "{confidence}");
    assert!(ask(&sim, "last_seen").as_number().unwrap() > Real::of(59.0));
}

#[test]
fn belief_properties_compile_in_a_mind_context_and_truth_does_not() {
    let registry = clash_sim::engine_registry();
    let entity = registry.kind_id("entity").unwrap();
    let context = clash_expr::Context::new()
        .own_subject("self", clash_expr::Type::Subject(entity))
        .subject("target", clash_expr::Type::Subject(entity))
        .belief();
    let ok = |source: &str| {
        clash_expr::compile(source, &registry, &context, &clash_expr::Type::Any)
            .diagnostics
            .iter()
            .all(|diagnostic| diagnostic.severity != clash_expr::Severity::Error)
    };
    assert!(ok("self.believed_loc(target)"));
    assert!(ok("self.knows(target) and self.confidence(target) > 0.5"));
    assert!(ok("self.perceives(target)"));
    assert!(ok("self.believed_loc(target).x"));
    // Truth about another, refused.
    assert!(!ok("target.loc"));
    assert!(!ok("self.believed_loc(target).light"));
    // Another mind's beliefs are not this one's to read.
    assert!(!ok("target.believed_loc(self)"));
}

#[test]
fn the_linter_flags_an_attached_rule_reading_another_entitys_place() {
    let flagged = r#"[
      { "type": "eoc", "id": "stalker", "on": "perceived", "attached": true,
        "if": "about != null and about.loc != null",
        "do": [] }
    ]"#;
    let types = common::types();
    let mut files = core_files("world");
    files.extend(core_files("perception"));
    files.push(("data/test/extra.json".to_owned(), flagged.to_owned()));
    let files: Vec<(&str, &str)> = files
        .iter()
        .map(|(path, text)| (path.as_str(), text.as_str()))
        .collect();
    let outcome = Loader::new(&types)
        .expr_registry(&clash_sim::engine_registry())
        .memory_mod("core", r#"{ "id": "core" }"#, &files)
        .load_mods(&["core"]);
    let rendered = outcome.diagnostics.render();
    assert!(
        rendered.contains("decides what it does") && rendered.contains("stalker"),
        "{rendered}"
    );
    // A rule that reads what it believes is not flagged.
    let fine = HEARD_NOT_SEEN_RULE.replace(
        "event.about_actor == null",
        "event.about_actor == null and not self.perceives(about ?? self) and self.believed_loc(about ?? self) == null",
    );
    let defs = try_load(&fine);
    assert!(defs.is_ok(), "{defs:?}");
}

#[test]
fn hidden_elements_are_not_known_until_noted() {
    let extra = r#"[
          { "type": "block", "id": "secret_door", "copy_from": "door", "tags": ["hidden"] }
        ]"#;
    let mut sim = city(extra);
    let mind = person(&mut sim, street(0));
    give(&mut sim, mind, "human_sight");
    // A wall across the street with a secret door in it.
    let row = street(6);
    for dy in -1..=1 {
        let at = Location::new(GRID, CellPos::new(row.cell.x, row.cell.y + dy, 0));
        set_block(&mut sim, at, "wall");
    }
    set_block(&mut sim, row, "secret_door");
    assert!(!perception::paths::knows_element(
        &sim,
        mind,
        row,
        Layer::Block
    ));
    let geometry = sim.truth().cosmos().neighbours(street(5)).unwrap();
    let east = geometry
        .neighbour(clash_world::StepKind::Level(clash_core::Dir8::East))
        .unwrap();
    let rules = perception::believed_move_rules(&sim, mind);
    assert!(!rules.can_occupy(&east), "the secret door is a wall to it");
    // Plain rules that open doors pass it.
    let walker = clash_world::MoveProfile {
        opens: true,
        ..clash_world::MoveProfile::walker()
    };
    assert!(walker.can_occupy(&east));
    perception::note_element(&mut sim, mind, row, Layer::Block).unwrap();
    assert!(perception::paths::knows_element(
        &sim,
        mind,
        row,
        Layer::Block
    ));
    let rules = perception::believed_move_rules(&sim, mind);
    assert!(rules.can_occupy(&east));
    let _ = PathLimits::DEFAULT;
}

#[test]
fn tracks_and_memories_are_bounded() {
    let mut sim = city(EXTRA);
    sim.set_setting("mind.max_tracks", 3_i64);
    sim.set_setting("mind.max_memories", 2_i64);
    sim.set_setting("track_capabilities", 2_i64);
    let mind = person(&mut sim, street(0));
    let others: Vec<EntityId> = (1..6).map(|dx| person(&mut sim, street(dx))).collect();
    for (i, other) in others.iter().enumerate() {
        let confidence = Real::of(0.1 * (i + 1) as f64);
        perception::add_track(
            &mut sim,
            mind,
            *other,
            Some(street(i as i32 + 1)),
            perception::Knowing::Told,
            confidence,
        )
        .unwrap();
    }
    let beliefs = sim.components().beliefs.get(mind).unwrap();
    assert_eq!(beliefs.tracks().len(), 3);
    // The least confident went first.
    assert!(perception::track_of(&sim, mind, others[0]).is_none());
    assert!(perception::track_of(&sim, mind, others[4]).is_some());
    for (what, result) in [
        ("core:punch", 1.0),
        ("core:kick", 3.0),
        ("core:bite", 2.0),
        ("core:punch", 5.0),
    ] {
        perception::observe_capability(&mut sim, mind, others[4], what, Real::of(result), false)
            .unwrap();
    }
    let track = perception::track_of(&sim, mind, others[4]).unwrap();
    let kept: Vec<(&str, Real)> = track
        .capabilities()
        .iter()
        .map(|capability| (capability.what.as_str(), capability.best))
        .collect();
    assert_eq!(
        kept,
        vec![("core:punch", Real::of(5.0)), ("core:kick", Real::of(3.0))]
    );
    for (event, salience) in [("a", 0.9), ("b", 0.1), ("c", 0.5)] {
        perception::remember(
            &mut sim,
            mind,
            perception::Memory {
                event: event.to_owned(),
                loc: None,
                at: Dur::ZERO,
                roles: Vec::new(),
                how: perception::Knowing::Inferred,
                confidence: Real::ONE,
                salience: Real::of(salience),
                source: None,
            },
        )
        .unwrap();
    }
    let memories: Vec<&str> = sim
        .components()
        .beliefs
        .get(mind)
        .unwrap()
        .memories()
        .iter()
        .map(|memory| memory.event.as_str())
        .collect();
    assert_eq!(memories, vec!["a", "c"]);
    sim.pump();
    assert_eq!(named(sim.undrained_events(), "memory_forgotten").len(), 1);
}

// ---- S9: what the party perceives -----------------------------------

/// A party member facing east with sight and hearing.
fn member(sim: &mut Sim, at: Location) -> EntityId {
    let me = person(sim, at);
    give(sim, me, "human_sight");
    give(sim, me, "human_hearing");
    perception::set_facing(sim, me, Real::of(90.0)).unwrap();
    common::debug(
        sim,
        DebugCommand::SetControl {
            entity: me,
            control: Control::Direct,
        },
    )
    .unwrap();
    me
}

/// Moves time on with a directly controlled member in the world: each
/// time it is asked what to do, it waits out the rest.
fn party_advance(sim: &mut Sim, me: EntityId, secs: f64) {
    let until = sim.now() + Dur::from_secs_real(Real::of(secs));
    while let clash_sim::Advance::NeedInput(_) = sim.advance(AdvanceLimit::Until(until)) {
        let left = until.since(sim.now()).max(Dur::from_millis(1));
        sim.submit(clash_sim::Command::Wait {
            entity: me,
            dur: left,
        })
        .unwrap();
    }
}

#[test]
fn events_are_tagged_by_how_the_party_perceived_them() {
    let mut sim = city(EXTRA);
    let me = member(&mut sim, street(0));
    party_advance(&mut sim, me, 0.6);
    sim.drain_events();
    // In view: a wall built ten cells along the street.
    set_block(&mut sim, street(10), "wall");
    // Out of sight and earshot: a wall built in a room two blocks off.
    let far = Location::new(
        GRID,
        CellPos::new(street(0).cell.x - 40, street(0).cell.y + 40, 0),
    );
    set_block(&mut sim, far, "wall");
    party_advance(&mut sim, me, 0.1);
    let events = sim.drain_events();
    let changed: Vec<&SimEvent> = named(&events, "element_changed");
    assert_eq!(changed.len(), 2);
    let near = changed
        .iter()
        .find(|event| event.subject("here") == Some(Subject::Loc(street(10))))
        .unwrap();
    let away = changed
        .iter()
        .find(|event| event.subject("here") == Some(Subject::Loc(far)))
        .unwrap();
    assert_eq!(sim.party_perceived(near), PartyPerception::Seen);
    assert_eq!(sim.party_perceived(away), PartyPerception::Not);
    // Heard, not seen: building behind its back.
    let behind = Location::new(
        GRID,
        CellPos::new(street(0).cell.x - 4, street(0).cell.y, 0),
    );
    set_block(&mut sim, behind, "wall");
    party_advance(&mut sim, me, 0.1);
    let events = sim.drain_events();
    let built = named(&events, "element_changed");
    assert_eq!(built.len(), 1);
    assert_eq!(
        sim.party_perceived(built[0]),
        PartyPerception::Heard { from: behind }
    );
    assert!(perception::party::is_member(&sim, me));
    // The omniscient switch sees everything.
    perception::set_omniscient(&mut sim, true);
    assert_eq!(sim.party_perceived(away), PartyPerception::Seen);
}

#[test]
fn map_memory_records_what_was_there_when_last_seen() {
    let mut sim = city(EXTRA);
    let me = member(&mut sim, street(0));
    set_block(&mut sim, street(12), "wall");
    party_advance(&mut sim, me, 0.6);
    let (frames, dropped) = perception::drain_frames(&mut sim);
    assert!(!dropped);
    assert!(!frames.is_empty());
    let wall = sim
        .truth()
        .cosmos()
        .defs()
        .block_state("wall", None)
        .unwrap();
    let memory = perception::party::map_memory(&sim, me).unwrap();
    assert_eq!(memory.cell(street(12)).map(|cell| cell.block), Some(wall));
    assert!(
        frames
            .iter()
            .any(|frame| frame.seen.iter().any(|cell| cell.loc == street(12)))
    );
    // It turns its back; the wall is taken away.
    perception::set_facing(&mut sim, me, Real::of(270.0)).unwrap();
    party_advance(&mut sim, me, 0.6);
    let (frames, _) = perception::drain_frames(&mut sim);
    assert!(
        frames
            .iter()
            .any(|frame| frame.unseen.contains(&street(12)))
    );
    let none = BlockState::NONE;
    common::debug(
        &mut sim,
        DebugCommand::SetElement {
            loc: street(12),
            layer: Layer::Block,
            state: none.raw(),
        },
    )
    .unwrap();
    party_advance(&mut sim, me, 0.6);
    // Remembered as it was, not as it is.
    let memory = perception::party::map_memory(&sim, me).unwrap();
    assert_eq!(memory.cell(street(12)).map(|cell| cell.block), Some(wall));
    assert!(
        sim.truth()
            .cosmos()
            .cell(street(12))
            .unwrap()
            .block
            .is_none()
    );
    // Looking again, it learns.
    perception::set_facing(&mut sim, me, Real::of(90.0)).unwrap();
    party_advance(&mut sim, me, 0.6);
    let memory = perception::party::map_memory(&sim, me).unwrap();
    assert_eq!(memory.cell(street(12)).map(|cell| cell.block), Some(none));
    let snapshot = perception::snapshot(&sim, me);
    assert!(
        snapshot
            .seen
            .iter()
            .any(|cell| cell.loc == street(12) && cell.in_view)
    );
}

// ---- Save and load -------------------------------------------------------

#[test]
fn save_and_load_are_invisible_to_perception() {
    let build = || {
        let mut sim = city(EXTRA);
        let me = member(&mut sim, street(0));
        let other = person(&mut sim, street(9));
        give(&mut sim, other, "human_hearing");
        let sound = kind(&sim, "sound");
        perception::emit_signal(
            &mut sim,
            Signal::new(sound, street(30), Real::of(30.0), Cause::DEBUG),
        )
        .unwrap();
        let _ = me;
        sim
    };
    let mut straight = build();
    advance_by(&mut straight, 3.0);
    let mut first = build();
    advance_by(&mut first, 0.05);
    // A delayed signal is in flight.
    assert!(first.perception().in_flight() > 0);
    let blob = first.save().unwrap();
    let cosmos = first.truth().cosmos().clone();
    let mut loaded = Sim::load(Arc::clone(first.defs()), cosmos, &blob).unwrap();
    loaded.reindex();
    assert_eq!(loaded.state_hash(), first.state_hash());
    advance_by(&mut loaded, 2.95);
    assert_eq!(loaded.now(), straight.now());
    assert_eq!(loaded.hashes(), straight.hashes());
}

#[test]
#[ignore = "a timing, not a check"]
#[allow(
    clippy::disallowed_types,
    clippy::disallowed_methods,
    reason = "a timing test reads the wall clock; nothing in the simulation does"
)]
fn timing_of_perception_in_a_city_block() {
    // Run with `cargo test -p clash-sim --release --test perception --
    // --ignored --nocapture timing`. 15 §17's budgets, measured.
    let mut sim = city(EXTRA);
    let sight = kind(&sim, "sight");
    let info = sim.perception_defs().kind(sight).unwrap().clone();

    // One observer-target evaluation of sight at 30 m.
    let eye = Spot::point(street(0), 1500);
    let body = Spot {
        loc: street(30),
        alt_mm: 0,
        height_mm: 1700,
    };
    let rounds = 20_000_u32;
    let started = std::time::Instant::now();
    let mut sum = Real::ZERO;
    for _ in 0..rounds {
        sum += perception::signal::strength_at(&sim, &info, Real::ONE, body, eye);
    }
    let each = started.elapsed() / rounds;
    println!("one evaluation of sight at 30 m: {each:?} ({sum})");

    // Fifty creatures in a block: two rows of a street, all in range of
    // each other, with sight and hearing, walking up and down.
    sim.set_wake_handler(WakeDomain::Stepper, pacer);
    let mut crowd = Vec::new();
    for i in 0..50 {
        let at = Location::new(
            GRID,
            CellPos::new(
                street(0).cell.x + (i % 25) * 2,
                street(0).cell.y + i / 25,
                0,
            ),
        );
        let one = person(&mut sim, at);
        give(&mut sim, one, "human_sight");
        give(&mut sim, one, "human_hearing");
        let facing = if i % 2 == 0 { 90.0 } else { 270.0 };
        perception::set_facing(&mut sim, one, Real::of(facing)).unwrap();
        sim.schedule_for(
            one,
            Dur::from_millis(i64::from(i) * 20),
            Wake::new(WakeDomain::Stepper, one.to_bits(), 0),
        )
        .unwrap();
        crowd.push(one);
    }
    advance_by(&mut sim, 1.0);
    sim.drain_events();
    let seconds = 10.0;
    let started = std::time::Instant::now();
    advance_by(&mut sim, seconds);
    let took = started.elapsed();
    let events = sim.drain_events();
    println!(
        "fifty creatures, one simulated second: {:?}; {} perceived, {} detected in {seconds} s",
        took / seconds as u32,
        named(&events, "perceived").len(),
        named(&events, "detected").len()
    );

    // A signal to fifty observers in range: given off, and received.
    let sound = kind(&sim, "sound");
    let started = std::time::Instant::now();
    let emitted = perception::emit_signal(
        &mut sim,
        Signal::new(sound, street(24), Real::of(5.0), Cause::DEBUG),
    )
    .unwrap();
    let given = started.elapsed();
    sim.set_wake_handler(WakeDomain::Stepper, |_, _, _| {});
    let started = std::time::Instant::now();
    advance_by(&mut sim, 0.2);
    println!(
        "a signal to {} observers: given off in {given:?}, received in {:?}",
        emitted.delayed,
        started.elapsed()
    );

    // A party member's sweep: its view, map memory and a frame.
    let me = member(&mut sim, street(40));
    let started = std::time::Instant::now();
    perception::search(&mut sim, me, Real::ONE).unwrap();
    println!("a party member's search: {:?}", started.elapsed());
    let memory = perception::party::map_memory(&sim, me).unwrap();
    let words = crowd.len();
    println!(
        "its map memory: {} cells; awareness entries of one of the crowd: {}",
        memory.len(),
        sim.components()
            .awareness
            .get(crowd[words / 2])
            .map_or(0, |awareness| awareness.entries().len())
    );
}

/// A wake-up handler that walks an entity a cell east and back, a step
/// every 700 ms of its own time.
fn pacer(sim: &mut Sim, _id: WakeId, wake: Wake) {
    let entity = EntityId::from_bits(wake.id);
    let Some(Place::At { loc, .. }) = sim.truth().place(entity) else {
        return;
    };
    let dx = if wake.arg.is_multiple_of(2) { 1 } else { -1 };
    let to = Location::new(
        loc.grid,
        CellPos::new(loc.cell.x + dx, loc.cell.y, loc.cell.z),
    );
    let _ = sim.relocate(entity, Place::at(to), Cause::DEBUG.by(entity));
    let _ = sim.schedule_for(
        entity,
        Dur::from_millis(700),
        Wake::new(WakeDomain::Stepper, wake.id, wake.arg + 1),
    );
}
