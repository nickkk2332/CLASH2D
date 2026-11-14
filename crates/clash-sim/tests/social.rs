//! The social layer against the real simulation in the test city
//! (15 §8 to §12): personas and disguises, relations, groups and their
//! knowledge, orders, and speech, with `mods/core`'s people, groups,
//! relation kinds, message kinds, lines and rules.

mod common;

use std::sync::{Arc, LazyLock};

use clash_core::{Dur, EntityId, Real};
use clash_data::Defs;
use clash_expr::{Type, Value};
use clash_sim::action::{ActionDef, How, Params};
use clash_sim::perception::{self, Knowing, PersonaId};
use clash_sim::reader::Known;
use clash_sim::social::{self, Stance, Toward};
use clash_sim::template::{CharacterSpec, Selection};
use clash_sim::{AdvanceLimit, Command, Control, DebugCommand, Place, Sim, SimEvent, Subject};
use clash_time::Timeline;
use clash_world::dims::TRACT_XY;
use clash_world::{Cause, CellPos, Cosmos, GridId, Location, TractPos, WorldDefs};

/// The tract the tests stand in: well inside the city.
const START: TractPos = TractPos::new(20, 20);

/// The base grid of the city.
const GRID: GridId = GridId::new(0);

/// What the tests add: a theft that is heard, a rule that lets those
/// tagged `sharp` see through every disguise, a rule by which the watch
/// turns on whoever it remembers stealing, an action that calls for
/// help, a talk option, and a hook that keeps one listener from
/// understanding anything.
const FIXTURES: &str = r#"[
  { "type": "window_kind", "id": "test_threat", "reach": 5 },
  { "type": "event", "id": "theft", "subjects": { "thief": "entity" },
    "doc": "Someone took what was not theirs." },
  { "type": "event_signal", "id": "theft_heard", "event": "core:theft",
    "signal": "sound", "strength": 1, "salience": 0.9, "about": "thief" },
  { "type": "eoc", "id": "sharp_eyes", "on": "identity_check",
    "if": "self.has_tag('sharp')",
    "do": [ { "set": { "event.holds": false } } ] },
  { "type": "eoc", "id": "watch_knows_the_thief", "on": "detected",
    "if": "self.in_group('core:watch') and self.remembers_actor(about, 'core:theft')",
    "do": [ { "set_relation": { "who": "self", "toward": "about", "kind": "hostile", "value": 1 } } ] },
  { "type": "action", "id": "shout_for_help",
    "phases": [ { "id": "shout", "time": 0,
      "do": [ { "say": { "who": "self", "kind": "help", "line": "help_calls" } } ] } ] },
  { "type": "eoc", "id": "offer_greeting", "on": "talk_options",
    "do": [ { "set": { "event.options": "[{'id': 'greet', 'text': 'Greet', 'kind': 'core:greet', 'lines': 'core:greetings'}]" } } ] },
  { "type": "eoc", "id": "deaf", "on": "understands",
    "if": "self.has_tag('deaf')",
    "do": [ { "set": { "event.understood": false } } ] }
]"#;

/// The JSON files of a directory of `mods/core/data`.
fn core_dir(sub: &str) -> Vec<(String, String)> {
    let dir = common::mods_dir().join("core/data").join(sub);
    let mut files: Vec<std::path::PathBuf> = std::fs::read_dir(&dir)
        .unwrap()
        .filter_map(|entry| entry.ok().map(|entry| entry.path()))
        .filter(|path| path.extension().is_some_and(|ext| ext == "json"))
        .collect();
    files.sort();
    files
        .iter()
        .map(|file| {
            let name = file.file_name().unwrap().to_string_lossy();
            (
                format!("data/{sub}/{name}"),
                std::fs::read_to_string(file).unwrap(),
            )
        })
        .collect()
}

/// `mods/core`'s world, life, perception, movement and social content,
/// and the fixtures, loaded once.
static DEFS: LazyLock<Arc<Defs>> = LazyLock::new(|| {
    let mut owned = core_dir("life");
    owned.extend(core_dir("perception"));
    owned.extend(core_dir("social"));
    let movement = common::mods_dir().join("core/data/movement.json");
    owned.push((
        "data/movement.json".to_owned(),
        std::fs::read_to_string(movement).unwrap(),
    ));
    owned.push((
        "data/test/channels.json".to_owned(),
        r#"[{ "type": "channel", "id": "arm", "name": "arm" }]"#.to_owned(),
    ));
    owned.push(("data/test/social.json".to_owned(), FIXTURES.to_owned()));
    let files: Vec<(&str, &str)> = owned
        .iter()
        .map(|(path, text)| (path.as_str(), text.as_str()))
        .collect();
    common::try_defs_with(&files).unwrap_or_else(|error| panic!("{error}"))
});

/// A cell of the street along the north edge of the start tract.
fn street(dx: i32) -> Location {
    Location::new(
        GRID,
        CellPos::new(START.x * TRACT_XY + dx, START.y * TRACT_XY + 1, 0),
    )
}

/// A cell three tracts south of the street, where the session is held:
/// out of sight and earshot of it.
fn far(dx: i32) -> Location {
    Location::new(
        GRID,
        CellPos::new(START.x * TRACT_XY + dx, (START.y + 3) * TRACT_XY + 1, 0),
    )
}

/// The test city with the tracts round the start loaded and hot.
fn city(seed: u64) -> Sim {
    let defs = Arc::clone(&DEFS);
    let world = WorldDefs::from_defs(&defs).unwrap_or_else(|d| panic!("{}", d.render()));
    let mut timeline = Timeline::new();
    let mut cosmos = Cosmos::new(Arc::clone(&world));
    let def = world.realms().lookup("core:test_city").unwrap();
    cosmos.create_realm(&mut timeline, def, 11).unwrap();
    let mut sim = Sim::from_parts(defs, cosmos, timeline, seed);
    let anchor = sim.truth().entities().next_id().unwrap();
    common::debug(
        &mut sim,
        DebugCommand::CreateEntity {
            spec: common::creature(),
            place: Place::Stored,
        },
    )
    .unwrap();
    common::debug(
        &mut sim,
        DebugCommand::StartSession {
            entity: anchor,
            at: far(1),
        },
    )
    .unwrap();
    sim.drain_events();
    sim
}

/// Makes a person from `core:person` and more templates, named, in a
/// street cell, by command.
fn person(sim: &mut Sim, name: &str, also: &[&str], at: Location) -> EntityId {
    let mut templates = vec!["core:person".to_owned()];
    templates.extend(also.iter().map(|template| format!("core:{template}")));
    let spec =
        CharacterSpec::of(templates).with("core:person.name", Selection::Text(name.to_owned()));
    let id = sim.truth().entities().next_id().unwrap();
    common::debug(
        sim,
        DebugCommand::Spawn {
            spec,
            place: Place::at(at),
        },
    )
    .unwrap();
    sim.pump();
    id
}

/// Gives an entity sight and hearing.
fn give_senses(sim: &mut Sim, who: EntityId) {
    for name in ["core:human_sight", "core:human_hearing"] {
        let sense = sim.perception_defs().sense_named(name).unwrap();
        perception::attach_sense(sim, who, sense).unwrap();
    }
}

/// Makes `observer` detect `target` by sight now.
fn see(sim: &mut Sim, observer: EntityId, target: EntityId) {
    let sight = sim
        .perception_defs()
        .sense_named("core:human_sight")
        .unwrap();
    perception::set_awareness(sim, observer, target, sight, Real::ONE).unwrap();
    assert!(perception::perceives(sim, observer, target));
}

fn advance_by(sim: &mut Sim, secs: f64) {
    let until = sim.now() + Dur::from_secs_real(Real::of(secs));
    sim.advance(AdvanceLimit::Until(until));
}

fn entity_value(sim: &Sim, who: EntityId) -> Value {
    sim.subject_value(Subject::Entity(who))
}

/// Runs a list of effects as `who`, written as JSON.
fn run_as(sim: &mut Sim, who: EntityId, json: &str) {
    let entity = sim.exprs().registry().kind_id("entity").unwrap();
    let effects = sim
        .compile_effects(json, &[("self", Type::Subject(entity))], &[])
        .unwrap_or_else(|error| panic!("{json}: {error}"));
    let me = entity_value(sim, who);
    let mut locals = Vec::new();
    sim.run_effects(
        &effects,
        &[me],
        &mut locals,
        Some(Subject::Entity(who)),
        Cause::DEBUG,
    );
    sim.pump();
}

/// Calls a verb with named arguments.
fn verb(sim: &mut Sim, name: &str, args: &[(&str, Value)]) -> Value {
    let result = sim
        .call_verb(name, args, &[], Cause::DEBUG)
        .unwrap_or_else(|reason| panic!("{name}: {reason}"));
    sim.pump();
    result
}

fn text(value: &str) -> Value {
    Value::from(value)
}

fn named<'a>(events: &'a [SimEvent], name: &str) -> Vec<&'a SimEvent> {
    events.iter().filter(|event| event.name() == name).collect()
}

fn faults(events: &[SimEvent]) -> Vec<String> {
    named(events, "fault")
        .iter()
        .map(|event| format!("{event:?}"))
        .collect()
}

fn kind(sim: &Sim, name: &str) -> social::RelationKindId {
    social::relation::kind_named(sim, name).unwrap()
}

fn rel(sim: &Sim, who: EntityId, kind_name: &str, toward: EntityId) -> Real {
    social::rel(sim, who, kind(sim, kind_name), toward).unwrap()
}

/// The persona `who`'s memory of an event names as its actor.
fn remembered_actor(sim: &Sim, who: EntityId, event: &str) -> Option<PersonaId> {
    sim.components()
        .beliefs
        .get(who)?
        .memories()
        .iter()
        .rev()
        .find(|memory| memory.event == event)?
        .roles
        .iter()
        .find(|(role, _)| role == "actor")
        .and_then(|(_, persona)| *persona)
}

/// A world with a trickster who wears a townsman's face, the townsman
/// far off, a witness who sees the trickster steal, and a guard of the
/// watch.
struct FrameJob {
    sim: Sim,
    trickster: EntityId,
    victim: EntityId,
    witness: EntityId,
}

fn frame_job(seed: u64) -> FrameJob {
    let mut sim = city(seed);
    let trickster = person(&mut sim, "Tess", &[], street(4));
    let victim = person(&mut sim, "Vic", &[], street(40));
    let witness = person(&mut sim, "Wes", &[], street(2));
    // Out of anyone's sight, the trickster takes the townsman's face.
    let copied = social::copy_persona(&mut sim, trickster, victim, None).unwrap();
    assert_eq!(copied, PersonaId::true_of(victim));
    give_senses(&mut sim, witness);
    see(&mut sim, witness, trickster);
    run_as(
        &mut sim,
        trickster,
        r#"[ { "emit": { "event": "core:theft", "thief": "self" } } ]"#,
    );
    advance_by(&mut sim, 0.2);
    FrameJob {
        sim,
        trickster,
        victim,
        witness,
    }
}

// ---- S6: personas ------------------------------------------------------------------

#[test]
fn frame_job_witnesses_remember_the_copied_persona_while_the_cause_names_the_actor() {
    let FrameJob {
        mut sim,
        trickster,
        victim,
        witness,
    } = frame_job(3);
    let events = sim.drain_events();
    assert!(faults(&events).is_empty(), "{:?}", faults(&events));
    // The world knows who did it.
    let theft = named(&events, "core:theft");
    assert_eq!(theft.len(), 1);
    assert_eq!(theft[0].cause.actor, Some(trickster));
    // The witness remembers the townsman doing it.
    assert_eq!(
        remembered_actor(&sim, witness, "core:theft"),
        Some(PersonaId::true_of(victim))
    );
    assert_eq!(
        social::persona_of(&sim, witness, trickster),
        PersonaId::true_of(victim)
    );
    // What the witness believes of the townsman is where the trickster
    // stood.
    assert_eq!(
        perception::believed_loc(&sim, witness, victim),
        Some(street(4))
    );
    // Its belief properties say the same of either.
    let blames = |sim: &Sim, x: EntityId| {
        social::persona_of(sim, witness, x) == PersonaId::true_of(victim)
            && remembered_actor(sim, witness, "core:theft")
                == Some(social::persona_of(sim, witness, x))
    };
    assert!(blames(&sim, victim));
    // The trickster drops the face out of sight: the witness, seeing it
    // again, does not blame it.
    social::drop_persona(&mut sim, trickster).unwrap();
    let sight = sim
        .perception_defs()
        .sense_named("core:human_sight")
        .unwrap();
    perception::set_awareness(&mut sim, witness, trickster, sight, Real::ZERO).unwrap();
    see(&mut sim, witness, trickster);
    assert_eq!(
        social::persona_of(&sim, witness, trickster),
        PersonaId::true_of(trickster)
    );
    assert!(!blames(&sim, trickster));
}

#[test]
fn seeing_through_a_disguise_links_the_personas_in_that_observer_only() {
    let mut sim = city(5);
    let masked = person(&mut sim, "Mara", &[], street(4));
    let sharp = person(&mut sim, "Sid", &[], street(2));
    let dull = person(&mut sim, "Dan", &[], street(6));
    run_as(
        &mut sim,
        sharp,
        r#"[ { "add_tag": { "to": "self", "tag": "sharp" } } ]"#,
    );
    let mask = social::create_persona(
        &mut sim,
        masked,
        social::PersonaSpec {
            name: Some("the Owl".to_owned()),
            ..social::PersonaSpec::default()
        },
    )
    .unwrap();
    social::assume_persona(&mut sim, masked, mask).unwrap();
    give_senses(&mut sim, sharp);
    give_senses(&mut sim, dull);
    sim.drain_events();
    see(&mut sim, sharp, masked);
    see(&mut sim, dull, masked);
    sim.pump();
    let events = sim.drain_events();
    let seen = named(&events, "disguise_seen_through");
    assert_eq!(seen.len(), 1, "only the sharp one sees through");
    assert_eq!(seen[0].subject("self"), Some(Subject::Entity(sharp)));
    assert_eq!(
        social::persona_of(&sim, sharp, masked),
        PersonaId::true_of(masked)
    );
    assert_eq!(social::persona_of(&sim, dull, masked), mask);
    // Linked for good in the sharp one: out of sight and back, the mask
    // fools it no more, and no check is asked again.
    let sight = sim
        .perception_defs()
        .sense_named("core:human_sight")
        .unwrap();
    for observer in [sharp, dull] {
        perception::set_awareness(&mut sim, observer, masked, sight, Real::ZERO).unwrap();
    }
    assert_eq!(
        social::persona_of(&sim, sharp, masked),
        PersonaId::true_of(masked)
    );
    assert_eq!(social::persona_of(&sim, dull, masked), mask);
    // Revealing it to the dull one links it there too.
    assert!(social::reveal_persona(&mut sim, dull, masked).unwrap());
    assert_eq!(
        social::persona_of(&sim, dull, masked),
        PersonaId::true_of(masked)
    );
}

#[test]
fn a_change_of_face_in_view_is_seen() {
    let mut sim = city(6);
    let masked = person(&mut sim, "Mara", &[], street(4));
    let watcher = person(&mut sim, "Wil", &[], street(2));
    give_senses(&mut sim, watcher);
    see(&mut sim, watcher, masked);
    let mask = social::create_persona(&mut sim, masked, social::PersonaSpec::default()).unwrap();
    social::assume_persona(&mut sim, masked, mask).unwrap();
    // It saw the mask go on: it still takes her for herself.
    assert_eq!(
        social::persona_of(&sim, watcher, masked),
        PersonaId::true_of(masked)
    );
    // Refused: someone else's persona.
    let other = social::create_persona(&mut sim, watcher, social::PersonaSpec::default()).unwrap();
    let refused = social::assume_persona(&mut sim, masked, other).unwrap_err();
    assert_eq!(refused.code, social::reason::NOT_OWN_PERSONA);
}

/// `personas` (D-971): an entity's own, its true one first, then those
/// made for it in the order they were made, so that a rule can take up
/// the one it has just made; someone else's is never among them.
#[test]
fn an_entitys_own_personas_list_the_newest_last_for_a_rule_to_take_up() {
    let mut sim = city(6);
    let masked = person(&mut sim, "Mara", &[], street(4));
    let other = person(&mut sim, "Wil", &[], street(2));
    let theirs = social::create_persona(&mut sim, other, social::PersonaSpec::default()).unwrap();
    let personas = |sim: &Sim| {
        sim.property_named(Subject::Entity(masked), "personas", &[])
            .to_string()
    };
    assert_eq!(personas(&sim), format!("[{}]", PersonaId::true_of(masked)));
    // A rule makes a persona and takes up the newest of its own.
    for _ in 0..2 {
        run_as(
            &mut sim,
            masked,
            r#"[ { "create_persona": { "who": "self", "tags": ["hooded"] } },
                 { "assume_persona": { "who": "self", "persona": { "expr": "last(self.personas)" } } } ]"#,
        );
    }
    let own = social::own_personas(&sim, masked);
    assert_eq!(own.len(), 3, "{own:?}");
    assert_eq!(own[0], PersonaId::true_of(masked));
    assert!(own[1] < own[2] && !own.contains(&theirs));
    assert_eq!(
        personas(&sim),
        format!("[{}, {}, {}]", own[0], own[1], own[2])
    );
    assert_eq!(social::presenting(&sim, masked), own[2]);
    // Copying someone else's adds nothing of one's own.
    social::copy_persona(&mut sim, masked, other, None).unwrap();
    assert_eq!(social::own_personas(&sim, masked), own);
}

#[test]
fn the_reader_lens_names_the_known_and_describes_strangers() {
    let FrameJob {
        mut sim,
        trickster,
        victim,
        witness,
    } = frame_job(7);
    // A stranger is described by what it looks like.
    assert_eq!(
        social::social_lens(&sim, witness, victim),
        Some(Known::Common("person".to_owned()))
    );
    social::learn_name(&mut sim, witness, victim).unwrap();
    assert_eq!(
        social::social_lens(&sim, witness, victim),
        Some(Known::Proper("Vic".to_owned()))
    );
    // The trickster in the townsman's face is named as him.
    assert_eq!(
        social::social_lens(&sim, witness, trickster),
        Some(Known::Proper("Vic".to_owned()))
    );
    // One knows one's own name.
    assert_eq!(
        social::social_lens(&sim, trickster, trickster),
        Some(Known::Proper("Tess".to_owned()))
    );
}

// ---- S7: relations, groups and orders ------------------------------------------------

#[test]
fn a_member_uses_its_groups_relation_and_a_uniform_holds_until_seen_through() {
    let mut sim = city(11);
    let guard = person(&mut sim, "Gus", &["watchman"], street(2));
    let thug = person(&mut sim, "Hal", &["thug"], street(4));
    let townsman = person(&mut sim, "Tom", &[], street(6));
    let events = sim.drain_events();
    assert!(faults(&events).is_empty(), "{:?}", faults(&events));
    assert_eq!(
        named(&events, "group_joined").len(),
        5,
        "three townsfolk, a guard, a thug"
    );
    assert!(social::groups_of(&sim, guard).contains(&"core:watch".to_owned()));
    // The guard holds nothing of its own toward the thug: the watch's
    // relation toward the gang the thug openly belongs to stands in.
    assert_eq!(rel(&sim, guard, "hostile", thug), Real::ONE);
    assert_eq!(social::stance_to(&sim, guard, thug), Stance::Opposes);
    assert_eq!(rel(&sim, guard, "hostile", townsman), Real::ZERO);
    assert_eq!(social::stance_to(&sim, guard, townsman), Stance::SidesWith);
    // Its own relation comes first.
    let hostile = kind(&sim, "hostile");
    let thug_persona = social::persona_of(&sim, guard, thug);
    social::set_relation(
        &mut sim,
        guard,
        Toward::Persona(thug_persona),
        hostile,
        Real::of(0.25),
        Cause::DEBUG,
    )
    .unwrap();
    assert_eq!(rel(&sim, guard, "hostile", thug), Real::of(0.25));
    social::clear_relation(
        &mut sim,
        guard,
        Toward::Persona(thug_persona),
        None,
        Cause::DEBUG,
    )
    .unwrap();
    assert_eq!(rel(&sim, guard, "hostile", thug), Real::ONE);
    // The thug puts on the watch's uniform: the guard takes it for one
    // of its own.
    let uniform = social::create_persona(
        &mut sim,
        thug,
        social::PersonaSpec {
            claims: Some(vec!["watch".to_owned()]),
            ..social::PersonaSpec::default()
        },
    )
    .unwrap();
    social::assume_persona(&mut sim, thug, uniform).unwrap();
    assert_eq!(rel(&sim, guard, "hostile", thug), Real::ZERO);
    assert_eq!(rel(&sim, guard, "friendly", thug), Real::ONE);
    assert_eq!(social::stance_to(&sim, guard, thug), Stance::SidesWith);
    // Until the guard sees through it.
    social::reveal_persona(&mut sim, guard, thug).unwrap();
    assert_eq!(rel(&sim, guard, "hostile", thug), Real::ONE);
    assert_eq!(social::stance_to(&sim, guard, thug), Stance::Opposes);
}

#[test]
fn minds_take_their_stances_from_relations() {
    let mut sim = city(14);
    let guard = person(&mut sim, "Gus", &["guard", "watchman"], street(2));
    let townsman = person(&mut sim, "Tom", &[], street(3));
    let thug = person(&mut sim, "Hal", &["thug"], street(5));
    use clash_sim::ai::{Stance as AiStance, think::stance_toward};
    // Nothing is held toward one never perceived.
    assert_eq!(stance_toward(&sim, guard, thug), None);
    for observer in [guard, townsman] {
        give_senses(&mut sim, observer);
        see(&mut sim, observer, thug);
    }
    // The watch takes the gang for enemies; the townsfolk fear it.
    assert_eq!(stance_toward(&sim, guard, thug), Some(AiStance::Hostile));
    assert_eq!(stance_toward(&sim, townsman, thug), Some(AiStance::Feared));
    // A uniform of the watch, put on out of sight, changes both.
    let uniform = social::create_persona(
        &mut sim,
        thug,
        social::PersonaSpec {
            claims: Some(vec!["watch".to_owned()]),
            ..social::PersonaSpec::default()
        },
    )
    .unwrap();
    let sight = sim
        .perception_defs()
        .sense_named("core:human_sight")
        .unwrap();
    for observer in [guard, townsman] {
        perception::set_awareness(&mut sim, observer, thug, sight, Real::ZERO).unwrap();
    }
    social::assume_persona(&mut sim, thug, uniform).unwrap();
    for observer in [guard, townsman] {
        see(&mut sim, observer, thug);
    }
    assert_eq!(stance_toward(&sim, guard, thug), None);
    assert_eq!(stance_toward(&sim, townsman, thug), None);
}

#[test]
fn relations_fade_toward_their_default() {
    let mut sim = city(12);
    let a = person(&mut sim, "Ann", &[], street(2));
    let b = person(&mut sim, "Ben", &[], street(30));
    let fearful = kind(&sim, "fearful");
    let toward = Toward::Persona(social::persona_of(&sim, a, b));
    social::set_relation(&mut sim, a, toward, fearful, Real::of(0.8), Cause::DEBUG).unwrap();
    assert_eq!(rel(&sim, a, "fearful", b), Real::of(0.8));
    // 0.02 a minute: ten minutes later, 0.6.
    advance_by(&mut sim, 600.0);
    let now = rel(&sim, a, "fearful", b);
    assert!((now - Real::of(0.6)).abs() < Real::of(1e-6), "{now}");
    advance_by(&mut sim, 3600.0);
    assert_eq!(rel(&sim, a, "fearful", b), Real::ZERO);
}

#[test]
fn a_bodyguard_intervenes_for_whom_it_guards() {
    let mut sim = city(13);
    let guard = person(&mut sim, "Bo", &[], street(2));
    let ward = person(&mut sim, "Wen", &[], street(3));
    assert!(!social::intervenes(&sim, guard, ward));
    let guards = kind(&sim, "guards");
    let toward = Toward::Persona(social::persona_of(&sim, guard, ward));
    social::set_relation(&mut sim, guard, toward, guards, Real::ONE, Cause::DEBUG).unwrap();
    assert!(social::intervenes(&sim, guard, ward));
    assert!(!social::intervenes(&sim, ward, guard));
}

#[test]
fn a_bodyguard_is_offered_the_window_opened_against_whom_it_guards() {
    // Cleo threatens Ada. Bram, who guards her, is offered the window by
    // his relation; Dan, who stands as near and guards nobody, is not.
    let mut sim = city(16);
    let ada = person(&mut sim, "Ada", &[], street(2));
    let bram = person(&mut sim, "Bram", &[], street(4));
    let cleo = person(&mut sim, "Cleo", &[], street(3));
    let dan = person(&mut sim, "Dan", &[], street(1));
    let guards = kind(&sim, "guards");
    let toward = Toward::Persona(social::persona_of(&sim, bram, ada));
    social::set_relation(&mut sim, bram, toward, guards, Real::ONE, Cause::DEBUG).unwrap();
    let threat = sim
        .defs()
        .lookup::<clash_sim::action::WindowKindDef>("core:test_threat")
        .unwrap();
    let ty = sim.exprs().registry().def_type_id("window_kind").unwrap();
    let args = [
        (
            "kind",
            Value::Id(clash_expr::DefRef::new(ty, u32::from(threat.raw()))),
        ),
        ("source", entity_value(&sim, cleo)),
        ("for", Value::list([entity_value(&sim, ada)])),
        ("time", Value::Real(Real::of(2.0))),
    ];
    verb(&mut sim, "open_window", &args);
    let offered: Vec<(EntityId, clash_sim::action::Offered)> = sim
        .windows()
        .iter()
        .find(|(_, window)| window.source == Some(cleo))
        .map(|(_, window)| {
            window
                .candidates
                .iter()
                .map(|candidate| (candidate.entity, candidate.offered))
                .collect()
        })
        .unwrap();
    use clash_sim::action::Offered;
    assert_eq!(
        offered,
        [(ada, Offered::Named), (bram, Offered::Intervenes)]
    );
    assert!(!offered.iter().any(|(who, _)| *who == dan));
}

/// A master and a servant who hears, made loyal to the master.
fn master_and_servant(seed: u64, servant_at: i32) -> (Sim, EntityId, EntityId) {
    let mut sim = city(seed);
    let master = person(&mut sim, "Mab", &[], street(2));
    let servant = person(&mut sim, "Sol", &[], street(servant_at));
    give_senses(&mut sim, servant);
    let loyal = kind(&sim, "loyal");
    let toward = Toward::Persona(social::persona_of(&sim, servant, master));
    social::set_relation(&mut sim, servant, toward, loyal, Real::ONE, Cause::DEBUG).unwrap();
    sim.drain_events();
    (sim, master, servant)
}

fn order_step(sim: &mut Sim, master: EntityId, servant: EntityId) -> Value {
    let step = sim.defs().lookup::<ActionDef>("core:step").unwrap();
    let ty = sim.exprs().registry().def_type_id("action").unwrap();
    let action = Value::Id(clash_expr::DefRef::new(ty, step.raw()));
    let with = Value::map([("dir", Value::from("east"))]);
    let master_value = entity_value(sim, master);
    let servant_value = entity_value(sim, servant);
    verb(
        sim,
        "order",
        &[
            ("from", master_value),
            ("to", servant_value),
            ("action", action),
            ("with", with),
        ],
    )
}

#[test]
fn minion_orders_a_servant_obeys_while_the_relation_holds_and_must_hear() {
    let (mut sim, master, servant) = master_and_servant(17, 4);
    order_step(&mut sim, master, servant);
    advance_by(&mut sim, 3.0);
    let events = sim.drain_events();
    assert!(faults(&events).is_empty(), "{:?}", faults(&events));
    assert_eq!(named(&events, "order_given").len(), 1);
    assert_eq!(social::standing_orders(&sim, servant).len(), 1);
    assert_eq!(
        sim.truth().location(servant),
        Some(street(5)),
        "it stepped east"
    );
    // The relation is taken away: the order is withdrawn, and the next
    // is refused.
    let toward = Toward::Persona(social::persona_of(&sim, servant, master));
    social::clear_relation(&mut sim, servant, toward, None, Cause::DEBUG).unwrap();
    sim.pump();
    let events = sim.drain_events();
    let cancelled = named(&events, "order_cancelled");
    assert_eq!(cancelled.len(), 1);
    assert!(social::standing_orders(&sim, servant).is_empty());
    order_step(&mut sim, master, servant);
    advance_by(&mut sim, 3.0);
    let events = sim.drain_events();
    assert_eq!(named(&events, "order_given").len(), 0);
    let refused = named(&events, "order_refused");
    assert_eq!(refused.len(), 1);
    assert_eq!(
        refused[0]
            .body
            .field("reason")
            .and_then(|value| value.plain().cloned()),
        Some(clash_core::Value::from(social::reason::NOT_OBEYED))
    );
    assert_eq!(sim.truth().location(servant), Some(street(5)), "it stayed");
}

#[test]
fn an_order_out_of_earshot_is_not_received() {
    // Forty-four metres: beyond the reach of hearing.
    let (mut sim, master, servant) = master_and_servant(19, 46);
    let sent = order_step(&mut sim, master, servant);
    assert_eq!(sent, Value::Int(0));
    advance_by(&mut sim, 3.0);
    let events = sim.drain_events();
    assert!(named(&events, "order_given").is_empty());
    assert!(named(&events, "order_refused").is_empty());
    assert_eq!(sim.truth().location(servant), Some(street(46)));
}

// ---- S8: speech ----------------------------------------------------------------------

#[test]
fn a_report_gives_the_hearer_the_memory_at_reduced_confidence_with_its_source() {
    let FrameJob {
        mut sim,
        victim,
        witness,
        ..
    } = frame_job(23);
    let guard = person(&mut sim, "Gus", &["watchman"], street(1));
    give_senses(&mut sim, guard);
    // A third guard, three blocks off, sees and hears nothing of it.
    let other = person(&mut sim, "Ora", &["watchman"], far(3));
    give_senses(&mut sim, other);
    sim.drain_events();
    let args = [
        ("who", entity_value(&sim, witness)),
        ("kind", text("report")),
        ("to", entity_value(&sim, guard)),
        ("event", text("core:theft")),
        ("about", entity_value(&sim, victim)),
    ];
    verb(&mut sim, "say", &args);
    advance_by(&mut sim, 0.5);
    let events = sim.drain_events();
    assert!(faults(&events).is_empty(), "{:?}", faults(&events));
    let heard = named(&events, "message_heard");
    assert!(
        heard
            .iter()
            .any(|event| event.subject("self") == Some(Subject::Entity(guard)))
    );
    let memory = sim
        .components()
        .beliefs
        .get(guard)
        .unwrap()
        .memories()
        .iter()
        .find(|memory| memory.event == "core:theft")
        .cloned()
        .unwrap();
    assert_eq!(memory.how, Knowing::Told);
    assert_eq!(memory.confidence, Real::of(0.5));
    assert_eq!(memory.source, Some(PersonaId::true_of(witness)));
    assert_eq!(
        memory.roles,
        vec![("actor".to_owned(), Some(PersonaId::true_of(victim)))]
    );
    // The guard stands where it sees the trickster in the townsman's
    // face for itself: its track of the townsman is its own.
    assert!(perception::track_of(&sim, guard, victim).is_some());
    // The guard passed it to the watch: the far guard, a third member
    // who heard nothing, was told it at once.
    assert_eq!(
        remembered_actor(&sim, other, "core:theft"),
        Some(PersonaId::true_of(victim))
    );
    let track = perception::track_of(&sim, other, victim).unwrap();
    assert_eq!(track.how(), Knowing::Told);
    assert_eq!(track.source(), Some(social::persona_of(&sim, other, guard)));
    assert_eq!(track.loc(), Some(street(4)));
    // And uses it: seeing the townsman, it turns on him; seeing the true
    // thief, it does not.
    see(&mut sim, other, victim);
    advance_by(&mut sim, 0.1);
    assert!(rel(&sim, other, "hostile", victim) > Real::of(0.99));
}

#[test]
fn a_call_for_help_is_heard_by_the_watch() {
    let mut sim = city(29);
    let townsman = person(&mut sim, "Tom", &[], street(2));
    let guard = person(&mut sim, "Gus", &["watchman"], street(20));
    let thug = person(&mut sim, "Hal", &["thug"], street(3));
    give_senses(&mut sim, guard);
    sim.drain_events();
    assert_eq!(social::stance_to(&sim, guard, thug), Stance::Opposes);
    let call = sim
        .defs()
        .lookup::<ActionDef>("core:shout_for_help")
        .unwrap();
    common::debug(
        &mut sim,
        DebugCommand::SetControl {
            entity: townsman,
            control: Control::Delegated,
        },
    )
    .unwrap();
    sim.submit(Command::Act {
        entity: townsman,
        action: call,
        params: Params::new(),
        how: How::Start,
    })
    .unwrap();
    advance_by(&mut sim, 1.0);
    let events = sim.drain_events();
    assert!(faults(&events).is_empty(), "{:?}", faults(&events));
    let heard: Vec<&SimEvent> = named(&events, "message_heard")
        .into_iter()
        .filter(|event| event.subject("self") == Some(Subject::Entity(guard)))
        .collect();
    assert_eq!(heard.len(), 1);
    let text = heard[0]
        .body
        .field("text")
        .and_then(|value| {
            value
                .plain()
                .and_then(clash_core::Value::as_str)
                .map(str::to_owned)
        })
        .unwrap();
    // Written for the guard, who does not know the townsman's name.
    assert!(text.starts_with("The person"), "{text}");
    // The bundled rule: the guard remembers it, where it came from.
    let memory = sim
        .components()
        .beliefs
        .get(guard)
        .unwrap()
        .memories()
        .iter()
        .find(|memory| memory.event == "core:help_called")
        .cloned()
        .unwrap();
    assert_eq!(memory.loc, Some(street(2)));
}

#[test]
fn one_who_does_not_understand_hears_nothing() {
    let mut sim = city(31);
    let speaker = person(&mut sim, "Sam", &[], street(2));
    let deaf = person(&mut sim, "Del", &[], street(4));
    let hearing = person(&mut sim, "Hep", &[], street(5));
    give_senses(&mut sim, deaf);
    give_senses(&mut sim, hearing);
    run_as(
        &mut sim,
        deaf,
        r#"[ { "add_tag": { "to": "self", "tag": "deaf" } } ]"#,
    );
    sim.drain_events();
    let args = [
        ("who", entity_value(&sim, speaker)),
        ("kind", text("greet")),
    ];
    verb(&mut sim, "say", &args);
    advance_by(&mut sim, 0.5);
    let events = sim.drain_events();
    let heard: Vec<Option<Subject>> = named(&events, "message_heard")
        .iter()
        .map(|event| event.subject("self"))
        .collect();
    assert_eq!(heard, vec![Some(Subject::Entity(hearing))]);
}

#[test]
fn talk_options_are_gathered_from_rules() {
    let mut sim = city(37);
    let a = person(&mut sim, "Ann", &[], street(2));
    let b = person(&mut sim, "Ben", &[], street(3));
    let options = social::talk_options(&mut sim, a, b).unwrap();
    assert_eq!(options.len(), 1);
    assert_eq!(options[0].id, "greet");
    assert_eq!(options[0].kind.as_deref(), Some("core:greet"));
    assert_eq!(options[0].lines.as_deref(), Some("core:greetings"));
}

// ---- Every property answers --------------------------------------------------------

/// Evaluates an expression with `self` and `x` bound.
fn read(sim: &Sim, me: EntityId, x: EntityId, source: &str) -> Value {
    let registry = sim.defs().expr_registry();
    let entity = registry.kind_id("entity").unwrap();
    let context = clash_expr::Context::new()
        .subject("self", Type::Subject(entity))
        .subject("x", Type::Subject(entity));
    let code = clash_expr::compile(source, registry, &context, &Type::Any)
        .into_result()
        .unwrap_or_else(|error| panic!("{source}: {error:?}"));
    sim.eval(
        &code,
        clash_sim::Bound::new()
            .subject(entity_value(sim, me))
            .subject(entity_value(sim, x)),
    )
}

#[test]
fn every_property_of_the_social_layer_answers() {
    let FrameJob {
        mut sim,
        trickster,
        victim,
        witness,
    } = frame_job(47);
    let guard = person(&mut sim, "Gus", &["watchman"], street(1));
    give_senses(&mut sim, guard);
    let args = [
        ("who", entity_value(&sim, witness)),
        ("kind", text("report")),
        ("to", entity_value(&sim, guard)),
        ("event", text("core:theft")),
    ];
    verb(&mut sim, "say", &args);
    advance_by(&mut sim, 0.5);
    let (master, servant) = (guard, witness);
    let loyal = kind(&sim, "loyal");
    let toward = Toward::Persona(social::persona_of(&sim, servant, master));
    social::set_relation(&mut sim, servant, toward, loyal, Real::ONE, Cause::DEBUG).unwrap();
    order_step(&mut sim, master, servant);
    advance_by(&mut sim, 0.2);
    let victim_persona = Value::from(PersonaId::true_of(victim).to_string().as_str());
    let witness_persona = Value::from(PersonaId::true_of(witness).to_string().as_str());
    // Personas.
    assert_eq!(read(&sim, victim, victim, "self.persona"), victim_persona);
    assert_eq!(
        read(&sim, trickster, victim, "self.presenting"),
        victim_persona
    );
    assert_eq!(
        read(&sim, witness, trickster, "self.persona_of(x)"),
        victim_persona
    );
    assert_eq!(
        read(&sim, witness, trickster, "self.knows_name(x)"),
        Value::Bool(false)
    );
    // Its own: the trickster's copy of the victim's face is not one.
    assert_eq!(
        read(&sim, trickster, trickster, "self.personas"),
        Value::list([Value::from(
            PersonaId::true_of(trickster).to_string().as_str()
        )])
    );
    // Relations and groups.
    assert_eq!(
        read(&sim, witness, guard, "self.rel('loyal', x)"),
        Value::Real(Real::ONE)
    );
    assert_eq!(
        read(&sim, witness, guard, "self.rel('no_such_kind', x)"),
        Value::Null
    );
    assert_eq!(
        read(&sim, guard, guard, "self.rel('friendly', 'core:watch')"),
        Value::Real(Real::ONE)
    );
    assert_eq!(
        read(&sim, witness, guard, "self.stance_to(x)"),
        Value::from("sides_with")
    );
    assert_eq!(
        read(&sim, guard, guard, "self.in_group('watch')"),
        Value::Bool(true)
    );
    assert_eq!(
        read(&sim, guard, guard, "self.role_in('core:watch')"),
        Value::from("guard")
    );
    assert_eq!(
        read(&sim, witness, witness, "self.role_in('core:watch')"),
        Value::Null
    );
    // Orders.
    assert_eq!(
        read(&sim, witness, witness, "self.standing_orders"),
        Value::Int(1)
    );
    // Memories and their sources.
    assert_eq!(
        read(
            &sim,
            witness,
            trickster,
            "self.remembers_actor(x, 'core:theft')"
        ),
        Value::Bool(true)
    );
    assert_eq!(
        read(&sim, guard, victim, "self.remembers_actor(x, '')"),
        Value::Bool(true)
    );
    assert_eq!(
        read(&sim, guard, guard, "self.memory_confidence('core:theft')"),
        Value::Real(Real::of(0.5))
    );
    assert_eq!(
        read(
            &sim,
            witness,
            witness,
            "self.memory_confidence('core:theft')"
        ),
        Value::Real(Real::ONE)
    );
    assert_eq!(
        read(&sim, guard, guard, "self.memory_source('core:theft')"),
        witness_persona
    );
    assert_eq!(
        read(&sim, witness, witness, "self.memory_source('core:theft')"),
        Value::Null
    );
    assert_eq!(
        read(&sim, guard, guard, "self.told_of('core:theft')"),
        Value::Bool(true)
    );
    assert_eq!(
        read(&sim, witness, witness, "self.told_of('core:theft')"),
        Value::Bool(false)
    );
    // The function.
    assert_eq!(
        read(&sim, witness, guard, "intervenes(self, x)"),
        Value::Bool(true)
    );
    assert_eq!(
        read(&sim, guard, witness, "intervenes(self, x)"),
        Value::Bool(false)
    );
    // Every property of the layer was read above.
    let read_here = [
        "persona",
        "presenting",
        "personas",
        "persona_of",
        "knows_name",
        "rel",
        "stance_to",
        "in_group",
        "role_in",
        "standing_orders",
        "remembers_actor",
        "memory_confidence",
        "memory_source",
        "told_of",
    ];
    for property in social::properties() {
        assert!(
            read_here.contains(&property.name),
            "{} is not read here",
            property.name
        );
    }
}

// ---- Save, load and replay ----------------------------------------------------------

#[test]
fn saving_and_loading_in_the_middle_changes_nothing() {
    let run = |split: bool| {
        let FrameJob {
            mut sim,
            witness,
            victim,
            ..
        } = frame_job(41);
        let guard = person(&mut sim, "Gus", &["watchman"], street(1));
        give_senses(&mut sim, guard);
        let (master, servant) = (
            person(&mut sim, "Mab", &[], street(10)),
            person(&mut sim, "Sol", &[], street(12)),
        );
        give_senses(&mut sim, servant);
        let loyal = kind(&sim, "loyal");
        let toward = Toward::Persona(social::persona_of(&sim, servant, master));
        social::set_relation(&mut sim, servant, toward, loyal, Real::ONE, Cause::DEBUG).unwrap();
        if split {
            let blob = sim.save().unwrap();
            let cosmos = sim.truth().cosmos().clone();
            let mut loaded = Sim::load(Arc::clone(sim.defs()), cosmos, &blob).unwrap();
            assert!(loaded.reindex().is_empty());
            assert_eq!(loaded.hashes(), sim.hashes());
            sim = loaded;
        }
        let args = [
            ("who", entity_value(&sim, witness)),
            ("kind", text("report")),
            ("to", entity_value(&sim, guard)),
            ("about", entity_value(&sim, victim)),
        ];
        verb(&mut sim, "say", &args);
        order_step(&mut sim, master, servant);
        advance_by(&mut sim, 3.0);
        sim.state_hash()
    };
    assert_eq!(run(false), run(true));
}

#[test]
fn a_replay_of_the_commands_reproduces_the_hash() {
    let setup = || {
        let mut sim = city(43);
        let townsman = person(&mut sim, "Tom", &[], street(2));
        let guard = person(&mut sim, "Gus", &["watchman"], street(20));
        give_senses(&mut sim, guard);
        common::debug(
            &mut sim,
            DebugCommand::SetControl {
                entity: townsman,
                control: Control::Delegated,
            },
        )
        .unwrap();
        (sim, townsman)
    };
    let (mut first, townsman) = setup();
    // The log from here on is what is replayed, on a world set up the
    // same way.
    let _ = first.take_input_log();
    let call = first
        .defs()
        .lookup::<ActionDef>("core:shout_for_help")
        .unwrap();
    first
        .submit(Command::Act {
            entity: townsman,
            action: call,
            params: Params::new(),
            how: How::Start,
        })
        .unwrap();
    advance_by(&mut first, 2.0);
    let end = first.now();
    let log = first.input_log().clone();
    assert_eq!(log.len(), 1);
    let (mut second, _) = setup();
    let _ = second.take_input_log();
    second.replay(&log).unwrap();
    second.advance(AdvanceLimit::Until(end));
    assert_eq!(second.state_hash(), first.state_hash());
}
