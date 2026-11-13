//! Detail levels of minds against the real simulation in the test city
//! (09 §9, 03 §10; packages I8 and T7; decisions D-890 to D-899): the
//! attention cap and its ranking, dwell, warm stepping, promotion by
//! involvement and by the party's sight, and what the party perceives.

mod common;

use std::path::PathBuf;
use std::sync::Arc;

use clash_core::{Dur, EntityId, Real, Tick, Value as Stored};
use clash_data::{Defs, Loader};
use clash_sim::ai::warm;
use clash_sim::template::CharacterSpec;
use clash_sim::{
    Advance, AdvanceLimit, Command, Control, DebugCommand, Place, Sim, SimEvent, Subject,
};
use clash_time::Timeline;
use clash_world::dims::TRACT_XY;
use clash_world::{Cause, CellPos, Cosmos, GridId, Location, TractPos, WorldDefs};
use proptest::prelude::*;

/// The tract the tests stand in: well inside the city.
const START: TractPos = TractPos::new(20, 20);

/// The base grid of the city.
const GRID: GridId = GridId::new(0);

/// Minds for these tests, beside the bundled ones: one that stands
/// still, one that walks to and fro by its home, an errand a test sends
/// a mind on, and an action that names someone.
const EXTRA: &str = r#"[
  { "type": "goal", "id": "idle", "score": 0.1,
    "methods": [ { "steps": [ { "wait": { "time": "3 s" } } ] } ] },
  { "type": "mind", "id": "idler", "goals": ["idle"], "think": "1 s", "go_action": "go_to",
    "senses": ["human_sight", "human_hearing"] },
  { "type": "template", "id": "idler", "kind": "role", "name": "Idler", "mind": "idler" },

  { "type": "goal", "id": "pace", "score": 0.2, "about": { "place": "self.home" },
    "methods": [ { "steps": [
      { "go": { "to": "offset($about, 5, 0) ?? $about" } }, { "wait": { "time": "1 s" } },
      { "go": { "to": "$about" } }, { "wait": { "time": "1 s" } } ] } ] },
  { "type": "mind", "id": "pacer", "goals": ["pace"], "think": "1 s", "go_action": "go_to",
    "senses": ["human_sight", "human_hearing"] },
  { "type": "template", "id": "pacer", "kind": "role", "name": "Pacer", "mind": "pacer" },

  { "type": "goal", "id": "errand", "score": 0, "about": { "place": "self.loc" },
    "methods": [ { "steps": [ { "go": { "to": "$about", "within": 0 } } ] } ],
    "done": "(dist(self, $about) ?? 99) < 1" },

  { "type": "action", "id": "poke", "name": "Poke", "params": { "who": "entity" },
    "phases": [ { "id": "reach", "time": "1 s" } ] }
]"#;

/// The content files of a directory of `mods/core/data`.
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

/// `mods/core/data/items.json` without its colours, which are the
/// interface's definition type.
fn items() -> String {
    let path = common::mods_dir().join("core/data/items.json");
    let text = std::fs::read_to_string(path).unwrap();
    let all: Vec<serde_json::Value> = serde_json::from_str(&text).unwrap();
    let kept: Vec<serde_json::Value> = all
        .into_iter()
        .filter(|def| def.get("type").and_then(serde_json::Value::as_str) != Some("color"))
        .collect();
    serde_json::to_string_pretty(&kept).unwrap()
}

/// `mods/core`'s world, perception, living things, movement, items,
/// fighting and minds, with the minds of these tests.
fn defs() -> Arc<Defs> {
    let types = common::types();
    let mut files = core_files("world");
    for dir in ["perception", "life", "combat", "ai"] {
        files.extend(core_files(dir));
    }
    files.push((
        "data/movement.json".to_owned(),
        std::fs::read_to_string(common::mods_dir().join("core/data/movement.json")).unwrap(),
    ));
    files.push(("data/items.json".to_owned(), items()));
    files.push(("data/test/extra.json".to_owned(), EXTRA.to_owned()));
    let files: Vec<(&str, &str)> = files
        .iter()
        .map(|(path, text)| (path.as_str(), text.as_str()))
        .collect();
    Loader::new(&types)
        .expr_registry(&clash_sim::engine_registry())
        .memory_mod("core", r#"{ "id": "core" }"#, &files)
        .load_mods(&["core"])
        .into_result()
        .unwrap_or_else(|diagnostics| panic!("the definitions do not load:\n{}", diagnostics.render()))
}

/// A cell of the street that runs east along the north edge of the
/// start tract, `dy` rows down.
fn street(dx: i32, dy: i32) -> Location {
    Location::new(
        GRID,
        CellPos::new(START.x * TRACT_XY + dx, START.y * TRACT_XY + 1 + dy, 0),
    )
}

/// A cell of the street three tracts south: out of sight and earshot of
/// the first.
fn far_street(dx: i32) -> Location {
    street(dx, 3 * TRACT_XY)
}

/// The test city with the tracts round the start loaded and hot, the
/// session held by a creature standing in the far street: the anchor
/// attention measures from.
fn city(seed: u64, cap: i64) -> (Sim, EntityId) {
    let defs = defs();
    let world = WorldDefs::from_defs(&defs).unwrap_or_else(|d| panic!("{}", d.render()));
    let mut timeline = Timeline::new();
    let mut cosmos = Cosmos::new(Arc::clone(&world));
    let def = world.realms().lookup("core:test_city").unwrap();
    cosmos.create_realm(&mut timeline, def, 11).unwrap();
    let mut sim = Sim::from_parts(defs, cosmos, timeline, seed);
    sim.set_setting(warm::MAX_HOT_SETTING, cap);
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
            at: far_street(1),
        },
    )
    .unwrap();
    common::debug(
        &mut sim,
        DebugCommand::SetControl {
            entity: anchor,
            control: Control::Direct,
        },
    )
    .unwrap();
    sim.drain_events();
    (sim, anchor)
}

/// Makes a character from templates, standing in a cell.
fn spawn(sim: &mut Sim, templates: &[&str], at: Location) -> EntityId {
    let id = sim.truth().entities().next_id().unwrap();
    common::debug(
        sim,
        DebugCommand::Spawn {
            spec: CharacterSpec::of(templates.iter().copied()),
            place: Place::at(at),
        },
    )
    .unwrap();
    id
}

/// Lets time pass; the character the player plays waits whenever it is
/// asked what to do.
fn advance_by(sim: &mut Sim, secs: f64) {
    let until = sim.now() + Dur::from_secs_real(Real::of(secs));
    loop {
        match sim.advance(AdvanceLimit::Until(until)) {
            Advance::NeedInput(request) => {
                let dur = until.since(sim.now());
                if dur <= Dur::ZERO {
                    return;
                }
                sim.submit(Command::Wait {
                    entity: request.entity,
                    dur,
                })
                .unwrap();
            }
            Advance::Reached(_) => return,
            _ => {}
        }
    }
}

/// Gives a character senses and puts it under direct control, facing a
/// way: a member of the party (15 §13).
fn party_member(sim: &mut Sim, who: EntityId, facing: f64) {
    for sense in ["core:human_sight", "core:human_hearing"] {
        let id = sim.perception_defs().sense_named(sense).unwrap();
        clash_sim::perception::attach_sense(sim, who, id).unwrap();
    }
    clash_sim::perception::set_facing(sim, who, Real::of(facing)).unwrap();
    common::debug(
        sim,
        DebugCommand::SetControl {
            entity: who,
            control: Control::Direct,
        },
    )
    .unwrap();
}

fn hot(sim: &Sim, who: EntityId) -> bool {
    !warm::is_warm(sim, who)
}

/// The level changes of a character among events drained: the tick of
/// each and whether it was a promotion.
fn changes(events: &[SimEvent], who: EntityId) -> Vec<(Tick, bool)> {
    events
        .iter()
        .filter(|event| common::entity_of(event, "self") == Some(who))
        .filter_map(|event| match event.name() {
            warm::ENTITY_PROMOTED => Some((event.tick, true)),
            warm::ENTITY_DEMOTED => Some((event.tick, false)),
            _ => None,
        })
        .collect()
}

/// Calls a verb of the engine as a rule would.
fn verb(sim: &mut Sim, name: &str, args: &[clash_expr::Value]) {
    let def = clash_sim::verb::engine_verbs()
        .into_iter()
        .find(|verb| verb.name == name)
        .unwrap_or_else(|| panic!("no verb {name}"));
    let id = sim.exprs().registry().verb_id(name).unwrap();
    let call = clash_sim::rule::VerbCall {
        verb: id,
        args,
        by: clash_sim::rule::RunView::new(&[], &[]),
    };
    (def.run)(sim, &call, Cause::DEBUG).unwrap_or_else(|reason| panic!("{name}: {reason}"));
    sim.pump();
}

// ---- the cap -------------------------------------------------------------------------

#[test]
fn with_more_minds_than_the_cap_exactly_the_cap_are_hot_and_they_rank_highest() {
    let (mut sim, anchor) = city(31, 3);
    let mut people = Vec::new();
    for x in [30, 2, 22, 6, 26, 10, 18, 14] {
        people.push((x, spawn(&mut sim, &["core:person", "core:idler"], street(x, 0))));
    }
    advance_by(&mut sim, 8.0);
    let census = warm::census(&sim);
    assert_eq!((census.hot, census.warm), (3, 5), "{census:?}");
    // The nearest to the character the player plays (the anchor stands
    // in the far street, below `x = 1`).
    let anchor_at = sim.truth().location(anchor).unwrap();
    let mut by_distance: Vec<(i64, EntityId)> = people
        .iter()
        .map(|(_, who)| {
            let at = sim.truth().location(*who).unwrap();
            let delta = sim.truth().cosmos().delta(anchor_at, at).unwrap();
            (clash_world::distance_sq_cells(delta), *who)
        })
        .collect();
    by_distance.sort();
    for (rank, (_, who)) in by_distance.iter().enumerate() {
        assert_eq!(hot(&sim, *who), rank < 3, "{who} ranked {rank}");
    }
}

#[test]
fn a_character_at_the_boundary_does_not_change_level_more_often_than_the_dwell() {
    let (mut sim, anchor) = city(37, 1);
    let a = spawn(&mut sim, &["core:person", "core:idler"], street(8, 0));
    let b = spawn(&mut sim, &["core:person", "core:idler"], street(12, 0));
    advance_by(&mut sim, 3.0);
    sim.drain_events();
    // The anchor paces between the two, so that which is nearest changes
    // every second.
    let mut events = Vec::new();
    for second in 0..30 {
        let x = if second % 2 == 0 { 7 } else { 13 };
        sim.relocate(anchor, Place::at(street(x, 6)), Cause::DEBUG)
            .unwrap();
        advance_by(&mut sim, 1.0);
        events.extend(sim.drain_events());
    }
    let mut changed = 0;
    for who in [a, b] {
        let seen = changes(&events, who);
        changed += seen.len();
        for pair in seen.windows(2) {
            let apart = pair[1].0.since(pair[0].0);
            assert!(
                apart >= warm::DWELL,
                "{who} changed level twice {apart:?} apart: {seen:?}"
            );
        }
    }
    assert!(changed >= 4, "the two changed places under the cap");
    // Exactly the cap is hot whenever the dwell allows.
    assert_eq!(warm::census(&sim).hot, 1);
}

// ---- moving between levels ----------------------------------------------------------

#[test]
fn a_warm_character_promoted_to_hot_thinks_at_once_and_continues_its_purpose() {
    let (mut sim, _) = city(41, 0);
    let walker = spawn(&mut sim, &["core:person", "core:idler"], street(2, 0));
    advance_by(&mut sim, 3.0);
    assert!(warm::is_warm(&sim, walker), "with no room under the cap it is warm");
    // An errand to the far end of the street.
    let goal = street(34, 0);
    let who = sim.subject_value(Subject::Entity(walker));
    let to = sim.subject_value(Subject::Loc(goal));
    verb(
        &mut sim,
        "set_goal",
        &[who, clash_expr::Value::from("core:errand"), to],
    );
    advance_by(&mut sim, 5.0);
    let x = sim.truth().location(walker).unwrap().cell.x;
    assert!(warm::is_warm(&sim, walker));
    assert!(
        x >= street(4, 0).cell.x,
        "it walked in bulk while warm: at {x}"
    );
    assert!(warm::is_walking(&sim, walker));
    sim.drain_events();
    // Room under the cap: it is promoted at the next warm step.
    sim.set_setting(warm::MAX_HOT_SETTING, 10);
    advance_by(&mut sim, 1.5);
    let events = sim.drain_events();
    let promoted = changes(&events, walker);
    assert_eq!(promoted.len(), 1, "{promoted:?}");
    let (at, up) = promoted[0];
    assert!(up);

    // It thought at once, and went on toward the same place with its
    // `go_action`.
    let started: Vec<&SimEvent> = events
        .iter()
        .filter(|event| event.name() == "action_started")
        .filter(|event| common::entity_of(event, "self") == Some(walker))
        .collect();
    assert!(
        started.iter().any(|event| event.tick == at),
        "it set out again at the tick it was promoted: {:?}",
        started.iter().map(|event| event.tick).collect::<Vec<_>>()
    );
    assert!(!warm::is_walking(&sim, walker));
    let running = sim
        .actions
        .iter()
        .find(|(_, action)| action.actor == walker)
        .map(|(_, action)| action.locals.clone())
        .expect("it walks");
    let target = sim.subject_value(Subject::Loc(goal)).to_stored();
    assert!(running.contains(&target), "{running:?}");
    advance_by(&mut sim, 40.0);
    let at = sim.truth().location(walker).unwrap();
    assert_eq!(at, goal, "it got there");
}

#[test]
fn a_warm_character_named_by_an_action_is_promoted_before_it_starts() {
    let (mut sim, _) = city(43, 0);
    let target = spawn(&mut sim, &["core:person", "core:idler"], street(6, 0));
    let actor = common::spawn(&mut sim, common::creature(), Place::at(street(5, 0)));
    common::debug(
        &mut sim,
        DebugCommand::SetControl {
            entity: actor,
            control: Control::Delegated,
        },
    )
    .unwrap();
    advance_by(&mut sim, 3.0);
    assert!(warm::is_warm(&sim, target));
    sim.drain_events();
    let poke = sim
        .defs()
        .lookup::<clash_sim::action::def::ActionDef>("core:poke")
        .unwrap();
    sim.submit(Command::Act {
        entity: actor,
        action: poke,
        params: vec![("who".to_owned(), sim.subject_value(Subject::Entity(target)).to_stored())],
        how: clash_sim::action::How::Start,
    })
    .unwrap();
    sim.pump();
    let events = sim.drain_events();
    let names: Vec<&str> = events
        .iter()
        .filter(|event| {
            matches!(event.name(), warm::ENTITY_PROMOTED | "action_started")
        })
        .map(SimEvent::name)
        .collect();
    assert_eq!(names, [warm::ENTITY_PROMOTED, "action_started"]);
    assert!(hot(&sim, target));
}

// ---- what the party perceives --------------------------------------------------------

/// Every character with a mind standing where the party sees is hot.
fn seen_are_hot(sim: &Sim) -> Result<(), String> {
    let view = clash_sim::perception::party::party_view(sim);
    for who in sim.truth().entities().ids() {
        if sim.mind_state(who).is_none() {
            continue;
        }
        let Some(at) = sim.truth().location(who) else {
            continue;
        };
        if view.binary_search(&at).is_ok() && warm::is_warm(sim, who) {
            return Err(format!("{who} stands in view at {at:?} and is warm"));
        }
    }
    Ok(())
}

/// The party's view of the characters it sees: at each frame, who stands
/// where.
fn frames_of(sim: &mut Sim) -> Vec<(Tick, Vec<(EntityId, Location)>)> {
    let (frames, _) = clash_sim::perception::drain_frames(sim);
    frames
        .into_iter()
        .map(|frame| {
            (
                frame.tick,
                frame
                    .entities
                    .iter()
                    .map(|seen| (seen.entity, seen.loc))
                    .collect(),
            )
        })
        .collect()
}

/// A scene: the party looking east along the street, pacers walking to
/// and fro in its view, and wanderers in the far street out of sight and
/// earshot, under a cap.
fn scene(seed: u64, cap: i64, near: &[i32], far: &[i32]) -> (Sim, EntityId) {
    let (mut sim, anchor) = city(seed, cap);
    sim.relocate(anchor, Place::at(street(0, 1)), Cause::DEBUG)
        .unwrap();
    party_member(&mut sim, anchor, 90.0);
    for x in near {
        spawn(&mut sim, &["core:person", "core:pacer"], street(*x, 1));
    }
    for x in far {
        spawn(&mut sim, &["core:person"], far_street(*x));
    }
    (sim, anchor)
}

#[test]
fn the_party_sees_only_hot_characters_and_sees_them_as_it_would_were_all_hot() {
    let near = [4, 9, 15];
    let far = [3, 8, 13, 18, 23];
    let (mut capped, _) = scene(47, 0, &near, &far);
    let (mut free, _) = scene(47, 1000, &near, &far);
    let mut seen_capped = Vec::new();
    let mut seen_free = Vec::new();
    for _ in 0..120 {
        advance_by(&mut capped, 0.1);
        advance_by(&mut free, 0.1);
        seen_are_hot(&capped).unwrap();
        seen_capped.extend(frames_of(&mut capped));
        seen_free.extend(frames_of(&mut free));
    }
    assert!(warm::census(&capped).warm >= far.len(), "the far ones are warm");
    assert_eq!(warm::census(&free).warm, 0);
    assert!(seen_capped.iter().any(|(_, seen)| seen.len() > 3));
    assert_eq!(seen_capped, seen_free);
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(6))]

    /// Whatever the cap and wherever people stand, a character's level
    /// never changes what the party perceives of the characters in its
    /// view: those it sees are hot, and it sees them as it would were
    /// everyone hot (09 §9.3).
    #[test]
    fn a_characters_level_never_changes_what_the_party_perceives_in_view(
        seed in 1_u64..1000,
        cap in 0_i64..4,
        near in prop::collection::btree_set(3_i32..24, 1..4),
        far in prop::collection::btree_set(0_i32..40, 0..6),
    ) {
        let near: Vec<i32> = near.into_iter().collect();
        let far: Vec<i32> = far.into_iter().collect();
        let (mut capped, _) = scene(seed, cap, &near, &far);
        let (mut free, _) = scene(seed, 1000, &near, &far);
        let mut seen_capped = Vec::new();
        let mut seen_free = Vec::new();
        for _ in 0..60 {
            advance_by(&mut capped, 0.1);
            advance_by(&mut free, 0.1);
            prop_assert!(seen_are_hot(&capped).is_ok(), "{:?}", seen_are_hot(&capped));
            seen_capped.extend(frames_of(&mut capped));
            seen_free.extend(frames_of(&mut free));
        }
        prop_assert_eq!(seen_capped, seen_free);
    }
}

// ---- saving and replaying -----------------------------------------------------------

/// Townsfolk going about, more than the cap.
fn busy(seed: u64) -> Sim {
    let (mut sim, _) = city(seed, 2);
    for x in [2, 7, 12, 17, 22, 27] {
        spawn(&mut sim, &["core:person"], street(x, 0));
    }
    sim
}

#[test]
fn levels_run_the_same_twice_and_through_a_save() {
    let mut once = busy(53);
    let mut twice = busy(53);
    for sim in [&mut once, &mut twice] {
        advance_by(sim, 10.0);
        advance_by(sim, 10.0);
    }
    assert!(once.ai().levels().stepping(), "the stepper runs");
    assert!(warm::census(&once).warm > 0);
    assert_eq!(once.state_hash(), twice.state_hash());
    let mut split = busy(53);
    advance_by(&mut split, 10.0);
    let blob = split.save().unwrap();
    let cosmos = split.truth().cosmos().clone();
    let mut loaded = Sim::load(Arc::clone(split.defs()), cosmos, &blob).unwrap();
    assert!(loaded.reindex().is_empty());
    assert_eq!(loaded.hashes(), split.hashes());
    advance_by(&mut loaded, 10.0);
    assert_eq!(once.hashes(), loaded.hashes());
}

#[test]
fn nobody_is_warm_and_nothing_is_kept_while_the_minds_fit_under_the_cap() {
    let (mut sim, _) = city(59, 150);
    for x in [2, 7, 12] {
        spawn(&mut sim, &["core:person"], street(x, 0));
    }
    advance_by(&mut sim, 10.0);
    assert!(sim.ai().levels().is_empty());
    assert_eq!(warm::census(&sim).warm, 0);
}

