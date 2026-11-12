//! Minds against the real simulation in the test city (09 §3 to §5):
//! goals, methods and steps, think scheduling, navigation for minds,
//! and the bundled minds of `mods/core` doing what their documentation
//! says. Characters are made from `mods/core`'s templates by debug
//! command, so that they think by the bundled minds.

mod common;

use std::path::PathBuf;
use std::sync::Arc;

use clash_core::{Dur, EntityId, Real, Value as Stored};
use clash_data::{Defs, Loader};
use clash_sim::ai::{self, nav};
use clash_sim::template::CharacterSpec;
use clash_sim::{AdvanceLimit, Command, DebugCommand, Place, Sim, SimEvent, Subject};
use clash_time::Timeline;
use clash_world::dims::TRACT_XY;
use clash_world::{
    Cause, CellPos, Cosmos, GridId, Layer, Location, MoveRules, PathLimits, StepKind, TractPos,
    WorldDefs,
};

/// The tract the tests stand in: well inside the city.
const START: TractPos = TractPos::new(20, 20);

/// The base grid of the city.
const GRID: GridId = GridId::new(0);

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

/// `mods/core`'s world, perception, living things, movement and minds,
/// with one more file.
fn try_load(extra: &str) -> Result<Arc<Defs>, String> {
    let types = common::types();
    let mut files = core_files("world");
    for dir in ["perception", "life", "ai"] {
        files.extend(core_files(dir));
    }
    files.push((
        "data/movement.json".to_owned(),
        std::fs::read_to_string(common::mods_dir().join("core/data/movement.json")).unwrap(),
    ));
    files.push((
        "data/test/channels.json".to_owned(),
        r#"[{ "type": "channel", "id": "arm", "name": "arm" }]"#.to_owned(),
    ));
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

/// A cell of the street that runs east along the north edge of the
/// start tract.
fn street(dx: i32) -> Location {
    Location::new(
        GRID,
        CellPos::new(START.x * TRACT_XY + dx, START.y * TRACT_XY + 1, 0),
    )
}

/// The test city with the tracts round the start loaded and hot.
fn city_with(extra: &str, seed: u64) -> Sim {
    let defs = load(extra);
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
    sim.drain_events();
    sim
}

fn city(seed: u64) -> Sim {
    city_with("[]", seed)
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

/// A plain creature with a tag, standing in a cell: nobody decides for it.
fn creature(sim: &mut Sim, tag: &str, at: Location) -> EntityId {
    let id = sim.truth().entities().next_id().unwrap();
    common::debug(
        sim,
        DebugCommand::CreateEntity {
            spec: common::creature().with_tag(tag),
            place: Place::at(at),
        },
    )
    .unwrap();
    id
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

fn goal_of(sim: &Sim, who: EntityId) -> Option<String> {
    sim.property_named(Subject::Entity(who), "goal", &[])
        .as_str()
        .map(str::to_owned)
}

/// The goals a mind took up, in order, from events drained.
fn adopted(events: &[SimEvent], who: EntityId) -> Vec<String> {
    events
        .iter()
        .filter(|event| event.name() == "goal_adopted")
        .filter(|event| common::entity_of(event, "self") == Some(who))
        .filter_map(|event| match event.body.field("goal") {
            Some(clash_sim::FieldValue::Plain(Stored::Str(goal))) => Some(goal),
            _ => None,
        })
        .collect()
}

fn named<'a>(events: &'a [SimEvent], name: &str, who: EntityId) -> Vec<&'a SimEvent> {
    events
        .iter()
        .filter(|event| event.name() == name && common::entity_of(event, "self") == Some(who))
        .collect()
}

fn loc_of(sim: &Sim, who: EntityId) -> Location {
    sim.truth().location(who).unwrap()
}

fn metres(sim: &Sim, a: Location, b: Location) -> f64 {
    nav::metres(sim.truth().cosmos().delta(a, b).unwrap()).get()
}

/// Strikes a body as a blow from someone would, through the `hit` verb.
fn strike(sim: &mut Sim, by: EntityId, target: EntityId) {
    let verb = clash_sim::verb::engine_verbs()
        .into_iter()
        .find(|verb| verb.name == "hit")
        .unwrap();
    let id = sim.exprs().registry().verb_id("hit").unwrap();
    let args = [
        sim.subject_value(Subject::Entity(target)),
        clash_expr::Value::from("core:cut"),
        clash_expr::Value::Real(Real::of(300.0)),
        clash_expr::Value::Real(Real::of(300.0 / (2.0e-5 * 0.002))),
        clash_expr::Value::Null,
        clash_expr::Value::Null,
        clash_expr::Value::Null,
        clash_expr::Value::Real(Real::of(2.0e-5)),
        clash_expr::Value::Real(Real::of(0.9)),
        clash_expr::Value::Null,
    ];
    let call = clash_sim::rule::VerbCall {
        verb: id,
        args: &args,
        by: clash_sim::rule::RunView::new(&[], &[]),
    };
    let outcome = (verb.run)(sim, &call, Cause::DEBUG.by(by)).unwrap();
    if std::env::var_os("AI_DEBUG").is_some() {
        eprintln!("hit: {outcome:?}");
    }
    sim.pump();
}

// ---- I3: minds and goals ---------------------------------------------------------

#[test]
fn a_goal_that_reads_another_entitys_state_fails_to_load_naming_the_read() {
    // Invariant N1: `$about` is someone the mind has a track of; its
    // place is truth, and a mind may read it only as believed.
    let error = try_load(
        r#"[{ "type": "goal", "id": "peek", "about": { "track": {} },
             "score": "dist(self, $about.loc) ?? 0",
             "methods": [ { "steps": [ { "wait": { "time": 1 } } ] } ] }]"#,
    )
    .unwrap_err();
    assert!(error.contains("entity.loc"), "{error}");
    assert!(error.contains("not safe for belief"), "{error}");
    // In a step, and in an `about`'s `where`, the same.
    let error = try_load(
        r#"[{ "type": "goal", "id": "peek", "about": { "track": { "where": "it.mass > 50" } },
             "score": "1",
             "methods": [ { "steps": [ { "go": { "to": "$about.loc" } } ] } ] }]"#,
    )
    .unwrap_err();
    assert!(error.contains("entity.mass"), "{error}");
    assert!(error.contains("entity.loc"), "{error}");
    // What it believes is fine.
    try_load(
        r#"[{ "type": "goal", "id": "look_at", "about": { "track": {} },
             "score": "dist(self, self.believed_loc($about)) ?? 0",
             "methods": [ { "steps": [ { "go": { "to": "self.believed_loc($about)" } } ] } ] }]"#,
    )
    .unwrap();
}

#[test]
fn spawned_people_think_by_the_bundled_minds() {
    let mut sim = city(3);
    let ada = spawn(&mut sim, &["core:person"], street(4));
    let watch = spawn(&mut sim, &["core:person", "core:guard"], street(6));
    let rex = spawn(&mut sim, &["core:dog"], street(8));
    advance_by(&mut sim, 2.0);
    for (who, mind) in [
        (ada, "core:townsperson"),
        (watch, "core:guard"),
        (rex, "core:dog"),
    ] {
        assert_eq!(
            sim.property_named(Subject::Entity(who), "mind_def", &[]),
            clash_expr::Value::from(mind)
        );
        assert!(goal_of(&sim, who).is_some(), "{mind} holds no goal");
    }
    assert_eq!(goal_of(&sim, watch).as_deref(), Some("core:patrol"));
}

#[test]
fn every_property_of_minds_answers() {
    let mut sim = city(6);
    let watch = spawn(&mut sim, &["core:person", "core:guard"], street(4));
    let outlaw = creature(&mut sim, "outlaw", street(9));
    let stone = creature(
        &mut sim,
        "stone",
        Location::new(
            GRID,
            street(4).cell.offset(clash_core::IVec3::new(0, 30, 0)),
        ),
    );
    advance_by(&mut sim, 1.5);
    let me = Subject::Entity(watch);
    let them = sim.subject_value(Subject::Entity(outlaw));
    let read = |name: &str, args: &[clash_expr::Value]| sim.property_named(me, name, args);
    assert_eq!(
        read("weight", &[clash_expr::Value::from("duty")]),
        clash_expr::Value::Real(Real::of(0.9))
    );
    assert_eq!(
        read("weight", &[clash_expr::Value::from("nothing")]),
        clash_expr::Value::Real(Real::ZERO)
    );
    assert_eq!(read("goal", &[]), clash_expr::Value::from("core:confront"));
    assert_eq!(read("goal_about", &[]), them);
    assert!(read("step", &[]).as_str().is_some());
    assert_eq!(read("last_failure", &[]), clash_expr::Value::Null);
    assert_eq!(
        read("home", &[]),
        sim.subject_value(Subject::Loc(street(4)))
    );
    assert_eq!(read("mind_def", &[]), clash_expr::Value::from("core:guard"));
    assert_eq!(
        read("hostile", std::slice::from_ref(&them)),
        clash_expr::Value::Bool(true)
    );
    assert_eq!(
        read("fears", std::slice::from_ref(&them)),
        clash_expr::Value::Bool(false)
    );
    assert_eq!(read("stance", &[them]), clash_expr::Value::from("hostile"));
    // Someone it has never perceived is held nothing, whatever they
    // present.
    let unseen = sim.subject_value(Subject::Entity(stone));
    assert_eq!(
        read("hostile", std::slice::from_ref(&unseen)),
        clash_expr::Value::Bool(false)
    );
    assert_eq!(read("stance", &[unseen]), clash_expr::Value::Null);
    // A creature with no mind answers nothing.
    let none = Subject::Entity(outlaw);
    assert_eq!(
        sim.property_named(none, "goal", &[]),
        clash_expr::Value::Null
    );
    assert_eq!(
        sim.property_named(none, "mind_def", &[]),
        clash_expr::Value::Null
    );
}

#[test]
fn the_same_inputs_give_the_same_goals_whatever_order_minds_think_in() {
    // Invariant N8: two minds prompted together decide the same in
    // either order.
    let mut sim = city(5);
    let a = spawn(&mut sim, &["core:person", "core:guard"], street(4));
    let b = spawn(&mut sim, &["core:person"], street(7));
    advance_by(&mut sim, 1.5);
    let outlaw = creature(&mut sim, "outlaw", street(12));
    advance_by(&mut sim, 1.0);
    let mut first = sim.clone();
    let mut second = sim.clone();
    ai::think::think_in_order(&mut first, &[a, b]);
    ai::think::think_in_order(&mut second, &[b, a]);
    for who in [a, b] {
        assert_eq!(
            first
                .mind_state(who)
                .and_then(|state| state.held().cloned()),
            second
                .mind_state(who)
                .and_then(|state| state.held().cloned()),
        );
    }
    assert_eq!(goal_of(&first, a).as_deref(), Some("core:confront"));
    let _ = outlaw;
}

const FLAPPING: &str = r#"[
  { "type": "variable", "id": "flip", "scope": "entity", "value": "int", "default": 0 },
  { "type": "goal", "id": "east", "score": "if self.var.flip == 1 then 0.50 else 0.52",
    "commit": 0.25, "methods": [ { "steps": [ { "wait": { "time": "500 ms" } } ] } ] },
  { "type": "goal", "id": "west", "score": "if self.var.flip == 1 then 0.52 else 0.50",
    "commit": 0.25, "methods": [ { "steps": [ { "wait": { "time": "500 ms" } } ] } ] },
  { "type": "goal", "id": "east_loose", "score": "if self.var.flip == 1 then 0.50 else 0.52",
    "methods": [ { "steps": [ { "wait": { "time": "500 ms" } } ] } ] },
  { "type": "goal", "id": "west_loose", "score": "if self.var.flip == 1 then 0.52 else 0.50",
    "methods": [ { "steps": [ { "wait": { "time": "500 ms" } } ] } ] },
  { "type": "mind", "id": "torn", "goals": ["east", "west"], "think": "500 ms" },
  { "type": "mind", "id": "fickle", "goals": ["east_loose", "west_loose"], "think": "500 ms" }
]"#;

fn flips(mind: &str) -> usize {
    let mut sim = city_with(FLAPPING, 9);
    let who = creature(&mut sim, "x", street(4));
    sim.create_mind(who, Some(mind.to_owned()), Cause::DEBUG)
        .unwrap();
    ai::prompt(&mut sim, who);
    sim.pump();
    sim.drain_events();
    for turn in 0..10 {
        sim.set_var(
            Subject::Entity(who),
            "core:flip",
            clash_expr::Value::Int(turn % 2),
            Cause::DEBUG,
        )
        .unwrap();
        advance_by(&mut sim, 1.0);
    }
    adopted(&sim.drain_events(), who).len()
}

#[test]
fn commit_keeps_a_mind_from_flapping_between_two_close_goals() {
    assert_eq!(flips("core:torn"), 0, "a held goal within its commit stays");
    assert!(flips("core:fickle") >= 8, "without commit it flaps");
}

// ---- I4: methods and steps -----------------------------------------------------------

const FAILING: &str = r#"[
  { "type": "goal", "id": "dig", "score": "0.9", "retry_after": "30 s",
    "methods": [ { "steps": [ { "go": { "to": "offset(self.loc, 0, 0, -1)" } } ] } ] },
  { "type": "goal", "id": "idle", "score": "0.1",
    "methods": [ { "steps": [ { "wait": { "time": "1 s" } } ] } ] },
  { "type": "mind", "id": "digger", "goals": ["dig", "idle"], "go_action": "go_to" }
]"#;

#[test]
fn a_failed_step_makes_the_mind_think_and_a_failing_goal_is_set_aside() {
    let mut sim = city_with(FAILING, 4);
    let who = spawn(&mut sim, &["core:person"], street(4));
    // The person thinks by `digger` from now on.
    let verb = clash_sim::verb::engine_verbs()
        .into_iter()
        .find(|verb| verb.name == "set_mind_def")
        .unwrap();
    let mind = sim
        .exprs()
        .registry()
        .def_type_id("mind")
        .and_then(|ty| sim.exprs().registry().def_ref(ty, "core:digger"))
        .unwrap();
    let args = [
        sim.subject_value(Subject::Entity(who)),
        clash_expr::Value::Id(mind),
    ];
    let call = clash_sim::rule::VerbCall {
        verb: sim.exprs().registry().verb_id("set_mind_def").unwrap(),
        args: &args,
        by: clash_sim::rule::RunView::new(&[], &[]),
    };
    (verb.run)(&mut sim, &call, Cause::DEBUG).unwrap();
    sim.drain_events();
    advance_by(&mut sim, 20.0);
    let events = sim.drain_events();
    assert!(!named(&events, "step_failed", who).is_empty());
    assert_eq!(named(&events, "goal_failed", who).len(), 1);
    let goals = adopted(&events, who);
    assert_eq!(goals.first().map(String::as_str), Some("core:dig"));
    // Set aside: twenty seconds later it has not been taken up again.
    assert_eq!(
        goals.iter().filter(|goal| *goal == "core:dig").count(),
        1,
        "{goals:?}"
    );
    assert_eq!(goal_of(&sim, who).as_deref(), Some("core:idle"));
    assert_eq!(
        sim.property_named(Subject::Entity(who), "last_failure", &[]),
        clash_expr::Value::from("ai.no_method")
    );
}

#[test]
fn a_townsperson_who_hears_a_wall_break_goes_to_look_then_goes_about_again() {
    let mut sim = city(7);
    let ada = spawn(&mut sim, &["core:person"], street(10));
    set_block(&mut sim, street(26), "wall");
    advance_by(&mut sim, 3.0);
    sim.drain_events();
    // The wall is broken by nobody it can see.
    let impact = sim
        .truth()
        .cosmos()
        .defs()
        .damage_types()
        .lookup("core:impact")
        .unwrap();
    let packet =
        clash_world::DamagePacket::new(impact, Real::of(2.0e6), Real::of(1.0e9), Cause::DEBUG);
    sim.world(|ctx| {
        ctx.edit(Cause::DEBUG)
            .damage(street(26), Layer::Block, packet)
    })
    .unwrap();
    sim.pump();
    advance_by(&mut sim, 1.0);
    assert_eq!(
        goal_of(&sim, ada).as_deref(),
        Some("core:investigate_noise")
    );
    advance_by(&mut sim, 20.0);
    let events = sim.drain_events();
    let goals = adopted(&events, ada);
    assert_eq!(
        goals.first().map(String::as_str),
        Some("core:investigate_noise")
    );
    // It went to look: it came within a few metres of the wall.
    let came_close = events
        .iter()
        .filter(|event| event.name() == "entity_relocated")
        .filter(|event| common::entity_of(event, "self") == Some(ada))
        .filter_map(|event| match event.subject("here") {
            Some(Subject::Loc(loc)) => Some(loc),
            _ => None,
        })
        .any(|loc| metres(&sim, loc, street(26)) <= 3.5);
    assert!(came_close, "it did not go to look");
    // And then went back to going about.
    assert_eq!(
        goals.last().map(String::as_str),
        Some("core:go_about"),
        "{goals:?}"
    );
}

#[test]
fn a_townsperson_who_sees_a_fight_calls_out_and_flees() {
    let mut sim = city(11);
    let ada = spawn(&mut sim, &["core:person"], street(10));
    let thug = creature(&mut sim, "brawler", street(15));
    let victim = spawn(&mut sim, &["core:person"], street(16));
    advance_by(&mut sim, 1.5);
    assert!(clash_sim::perception::perceives(&sim, ada, thug));
    sim.drain_events();
    let before = metres(&sim, loc_of(&sim, ada), loc_of(&sim, thug));
    strike(&mut sim, thug, victim);
    advance_by(&mut sim, 6.0);
    let events = sim.drain_events();
    if std::env::var_os("AI_DEBUG").is_some() {
        for event in &events {
            if common::entity_of(event, "self") == Some(ada)
                || event.name().starts_with("hit")
                || event.name().starts_with("wound")
            {
                eprintln!("{} {} {:?}", event.tick, event.name(), event.body);
            }
        }
    }
    let goals = adopted(&events, ada);
    assert!(
        goals
            .iter()
            .any(|goal| goal == "core:flee_danger" || goal == "core:flee_feared"),
        "{goals:?}"
    );
    assert!(
        !named(&events, "said", ada).is_empty(),
        "it did not call out"
    );
    assert_eq!(
        sim.property_named(
            Subject::Entity(ada),
            "stance",
            &[sim.subject_value(Subject::Entity(thug))]
        ),
        clash_expr::Value::from("feared")
    );
    let after = metres(&sim, loc_of(&sim, ada), loc_of(&sim, thug));
    assert!(after > before + 4.0, "{before} -> {after}");
}

#[test]
fn a_guard_goes_for_a_hostile_loses_it_behind_a_door_searches_and_gives_up() {
    let mut sim = city(13);
    let watch = spawn(&mut sim, &["core:person", "core:guard"], street(4));
    // A wall across the street with a door in it, east of the guard.
    let door_x = 20;
    let row = |dy: i32| {
        Location::new(
            GRID,
            CellPos::new(START.x * TRACT_XY + door_x, START.y * TRACT_XY + dy, 0),
        )
    };
    for dy in -3..=4 {
        if dy == 1 {
            continue;
        }
        let cell = sim.truth().cosmos().cell(row(dy)).unwrap();
        if cell.block.is_none() {
            set_block(&mut sim, row(dy), "wall");
        }
    }
    let outlaw = creature(&mut sim, "outlaw", street(14));
    advance_by(&mut sim, 6.0);
    let events = sim.drain_events();
    assert_eq!(
        adopted(&events, watch).last().map(String::as_str),
        Some("core:confront")
    );
    assert!(!named(&events, "said", watch).is_empty());
    // It reached the outlaw.
    let gap = metres(&sim, loc_of(&sim, watch), loc_of(&sim, outlaw));
    assert!(gap <= 2.5, "the guard is {gap} m from the outlaw");
    // The outlaw goes through the door, which shuts behind it, and on
    // out of earshot.
    common::debug(
        &mut sim,
        DebugCommand::MoveEntity {
            entity: outlaw,
            to: Place::at(street(23)),
        },
    )
    .unwrap();
    set_block(&mut sim, street(door_x), "door");
    common::debug(
        &mut sim,
        DebugCommand::MoveEntity {
            entity: outlaw,
            to: Place::at(Location::new(
                GRID,
                CellPos::new(START.x * TRACT_XY + 30, (START.y + 2) * TRACT_XY + 1, 0),
            )),
        },
    )
    .unwrap();
    advance_by(&mut sim, 30.0);
    let events = sim.drain_events();
    let goals = adopted(&events, watch);
    assert!(
        goals.iter().any(|goal| goal == "core:search_lost"),
        "{goals:?}"
    );
    assert_eq!(
        goals.last().map(String::as_str),
        Some("core:patrol"),
        "{goals:?}"
    );
    assert!(!clash_sim::perception::perceives(&sim, watch, outlaw));
}

// ---- I1: navigation ---------------------------------------------------------------------

/// The cheapest cost from one cell to another under some rules, found by
/// trying every way: a plain Dijkstra over the ten steps.
fn brute_force(sim: &Sim, rules: &impl MoveRules, from: Location, to: Location) -> Option<f64> {
    use std::collections::BTreeMap;
    let cosmos = sim.truth().cosmos();
    let mut best: BTreeMap<(i32, i32, i32), f64> = BTreeMap::new();
    let mut open: Vec<(f64, Location)> = vec![(0.0, from)];
    best.insert((from.cell.x, from.cell.y, from.cell.z), 0.0);
    while let Some(at) = (0..open.len()).min_by(|a, b| open[*a].0.total_cmp(&open[*b].0)) {
        let (cost, here) = open.swap_remove(at);
        if here == to {
            return Some(cost);
        }
        let key = (here.cell.x, here.cell.y, here.cell.z);
        if best.get(&key).is_some_and(|have| *have < cost) {
            continue;
        }
        let Ok(around) = cosmos.neighbours(here) else {
            continue;
        };
        for kind in StepKind::ALL {
            let Some(step) = around.step(kind) else {
                continue;
            };
            if !rules.can_occupy(&step.to) {
                continue;
            }
            let Some(price) = rules.step_cost(&step) else {
                continue;
            };
            let price = price.max(step.length_m).get();
            let next = Location::new(here.grid, step.to.cell);
            // Within a box round the two ends: what the test's maps hold.
            if (next.cell.x - from.cell.x).abs() > 48 || (next.cell.y - from.cell.y).abs() > 24 {
                continue;
            }
            let total = cost + price;
            let key = (next.cell.x, next.cell.y, next.cell.z);
            if best.get(&key).is_none_or(|have| total < *have) {
                best.insert(key, total);
                open.push((total, next));
            }
        }
    }
    None
}

#[test]
fn paths_for_minds_are_as_cheap_as_trying_every_way() {
    // A property test over random walls: the path a mind plans costs
    // what the cheapest way found by brute force costs, to a millimetre.
    let mut sim = city(17);
    let ada = spawn(&mut sim, &["core:person"], street(2));
    advance_by(&mut sim, 0.1);
    let mut rng = clash_core::Rng::from_seed(99);
    let open_cell = |sim: &Sim, loc: Location| {
        sim.truth()
            .cosmos()
            .cell(loc)
            .is_ok_and(|cell| cell.block.is_none() && cell.fixture.is_none())
    };
    let mut compared = 0;
    for _ in 0..12 {
        // Scatter walls in the street ahead.
        for _ in 0..10 {
            let dx = rng.range(4, 30) as i32;
            let dy = rng.range(-1, 3) as i32;
            let at = Location::new(
                GRID,
                street(dx).cell.offset(clash_core::IVec3::new(0, dy, 0)),
            );
            if open_cell(&sim, at) {
                set_block(&mut sim, at, "wall");
            }
        }
        let to = street(rng.range(10, 34) as i32);
        if !open_cell(&sim, to) {
            continue;
        }
        let here = loc_of(&sim, ada);
        let rules = nav::mind_rules(&sim, ada, GRID);
        let planned = sim
            .truth()
            .cosmos()
            .find_path(here, to, &rules, PathLimits::DEFAULT)
            .ok()
            .map(|path| path.cost_m().get());
        let brute = brute_force(&sim, &rules, here, to);
        if let (Some(planned), Some(brute)) = (planned, brute) {
            assert!((planned - brute).abs() < 1.0e-3, "{planned} vs {brute}");
            compared += 1;
        } else {
            assert_eq!(planned.is_some(), brute.is_some());
        }
    }
    assert!(compared >= 6, "only {compared} paths were compared");
}

#[test]
fn a_route_in_use_is_replaced_when_a_wall_falls_across_it() {
    // `wall_falls_reroute`: what a planner consults agrees with the
    // world after an edit (invariant N3).
    let mut sim = city(19);
    // A walker with no mind: the recheck is for every route in use.
    let ada = creature(&mut sim, "walker", street(2));
    advance_by(&mut sim, 0.1);
    let go_to = sim
        .defs()
        .try_registry::<clash_sim::action::def::ActionDef>()
        .unwrap()
        .lookup("core:go_to")
        .unwrap();
    // Walk east along the street, as a mind's `go` would.
    let to = sim.subject_value(Subject::Loc(street(30))).to_stored();
    sim.start_action(
        ada,
        go_to,
        &vec![("to".to_owned(), to)],
        Cause::DEBUG.by(ada),
    )
    .unwrap();
    advance_by(&mut sim, 2.0);
    let journey = sim
        .components()
        .mover
        .get(ada)
        .and_then(clash_sim::movement::Mover::journey)
        .cloned()
        .expect("it is walking");
    // A wall falls across the way ahead, leaving a gap a row to the side.
    let blocked = street(18);
    assert!(journey.remaining().contains(&blocked.cell));
    set_block(&mut sim, blocked, "wall");
    sim.pump();
    let events = sim.drain_events();
    assert_eq!(named(&events, "route_blocked", ada).len(), 1);
    advance_by(&mut sim, 30.0);
    // It went round and arrived.
    assert!(metres(&sim, loc_of(&sim, ada), street(30)) <= 1.5);
}

// ---- determinism ------------------------------------------------------------------------

fn town(seed: u64) -> (Sim, Vec<EntityId>) {
    let mut sim = city(seed);
    let mut people = Vec::new();
    for at in [3, 9, 15] {
        people.push(spawn(&mut sim, &["core:person"], street(at)));
    }
    people.push(spawn(&mut sim, &["core:person", "core:guard"], street(20)));
    people.push(spawn(&mut sim, &["core:dog"], street(24)));
    (sim, people)
}

#[test]
fn the_same_town_runs_the_same_twice_and_through_a_save() {
    let (mut once, _) = town(21);
    let (mut twice, _) = town(21);
    advance_by(&mut once, 20.0);
    advance_by(&mut twice, 20.0);
    assert_eq!(once.state_hash(), twice.state_hash());
    // Saved and loaded half way, the same.
    let (mut split, _) = town(21);
    advance_by(&mut split, 10.0);
    let blob = split.save().unwrap();
    let cosmos = split.truth().cosmos().clone();
    let mut loaded = Sim::load(Arc::clone(split.defs()), cosmos, &blob).unwrap();
    assert!(loaded.reindex().is_empty());
    assert_eq!(loaded.hashes(), split.hashes());
    advance_by(&mut loaded, 10.0);
    assert_eq!(once.hashes(), loaded.hashes());
}

// ---- N2: a character's acts are a player's -------------------------------------------------

#[test]
fn a_minds_acts_replayed_as_commands_reach_the_same_place() {
    let (mut sim, people) = town(23);
    let who = people[0];
    let mut acts: Vec<(
        clash_core::Tick,
        clash_sim::ActionDefId,
        clash_sim::action::Params,
    )> = Vec::new();
    for _ in 0..300 {
        advance_by(&mut sim, 0.05);
        for event in sim.drain_events() {
            if event.name() != "action_started" || common::entity_of(&event, "self") != Some(who) {
                continue;
            }
            let clash_sim::EventBody::Act(clash_sim::action::ActEvent::ActionStarted {
                id, ..
            }) = &event.body
            else {
                continue;
            };
            let Some(running) = sim.actions.get(*id) else {
                continue;
            };
            let def = sim
                .defs()
                .try_registry::<clash_sim::action::def::ActionDef>()
                .unwrap()
                .get(running.def);
            let params = def
                .params()
                .iter()
                .filter_map(|param| {
                    let slot = def.local_slot(&param.name.value)?;
                    let value = running.locals.get(usize::from(slot))?.clone();
                    Some((param.name.value.clone(), value))
                })
                .collect();
            acts.push((event.tick, running.def, params));
        }
    }
    assert!(acts.len() >= 2, "the mind did too little to replay");
    // The same town, but the character's mind thinks by nothing and its
    // acts are given as commands.
    let (mut replay, people) = town(23);
    let them = people[0];
    // Its mind thinks by no `mind` definition: nothing decides for it
    // but the commands.
    replay.remove_mind(them, Cause::DEBUG).unwrap();
    replay.create_mind(them, None, Cause::DEBUG).unwrap();
    common::debug(
        &mut replay,
        DebugCommand::SetControl {
            entity: them,
            control: clash_sim::Control::Delegated,
        },
    )
    .unwrap();
    for (tick, action, params) in acts {
        replay.advance(AdvanceLimit::Until(tick));
        let _ = replay.submit(Command::Act {
            entity: them,
            action,
            params,
            how: clash_sim::action::How::Start,
        });
    }
    replay.advance(AdvanceLimit::Until(sim.now()));
    assert_eq!(loc_of(&sim, who), loc_of(&replay, them));
}

// ---- budget ------------------------------------------------------------------------------

#[test]
#[ignore = "timing: run with --release --ignored"]
fn twenty_minds_in_a_block_for_a_minute_stay_within_budget() {
    let mut sim = city(29);
    for at in 0..20 {
        let template: &[&str] = if at % 5 == 0 {
            &["core:person", "core:guard"]
        } else {
            &["core:person"]
        };
        spawn(&mut sim, template, street(2 + at * 2));
    }
    advance_by(&mut sim, 1.0);
    let steps = sim.steps();
    #[allow(clippy::disallowed_types, clippy::disallowed_methods)]
    let started = std::time::Instant::now();
    advance_by(&mut sim, 60.0);
    let spent = started.elapsed();
    let wakes = sim.steps() - steps;
    let per_second = spent.as_secs_f64() / 60.0;
    println!(
        "twenty minds, a simulated minute: {:.1} ms in all, {:.2} ms a simulated second, {} wake-ups, {:.1} µs each",
        spent.as_secs_f64() * 1e3,
        per_second * 1e3,
        wakes,
        spent.as_secs_f64() * 1e6 / wakes.max(1) as f64
    );
    // The stated cost: twenty minds and their senses within 25 ms of
    // real time a simulated second.
    assert!(per_second < 0.025, "{per_second}");
}

#[test]
#[ignore = "timing: run with --release --ignored"]
fn a_path_across_a_tract_is_within_budget() {
    let mut sim = city(31);
    let ada = spawn(&mut sim, &["core:person"], street(0));
    advance_by(&mut sim, 0.1);
    let rules = nav::mind_rules(&sim, ada, GRID);
    let here = loc_of(&sim, ada);
    let to = street(TRACT_XY - 1);
    let plain = believed_alone(&sim, ada);
    // Once each first, so that both are timed with the world's caches
    // warm.
    for warm in [true, false] {
        let found = if warm {
            sim.truth()
                .cosmos()
                .find_path(here, to, &rules, PathLimits::DEFAULT)
        } else {
            sim.truth()
                .cosmos()
                .find_path(here, to, &plain, PathLimits::DEFAULT)
        };
        assert!(found.is_ok());
    }
    #[allow(clippy::disallowed_types, clippy::disallowed_methods)]
    let started = std::time::Instant::now();
    for _ in 0..100 {
        sim.truth()
            .cosmos()
            .find_path(here, to, &rules, PathLimits::DEFAULT)
            .unwrap();
    }
    let each = started.elapsed().as_secs_f64() / 100.0;
    // The same search with the mover's believed rules alone, to see
    // what a mind's costs add.
    #[allow(clippy::disallowed_types, clippy::disallowed_methods)]
    let started = std::time::Instant::now();
    for _ in 0..100 {
        sim.truth()
            .cosmos()
            .find_path(here, to, &plain, PathLimits::DEFAULT)
            .unwrap();
    }
    let alone = started.elapsed().as_secs_f64() / 100.0;
    println!(
        "a path across a tract: {:.1} µs with a mind's costs, {:.1} µs without",
        each * 1e6,
        alone * 1e6
    );
    // 09 §19: under 150 µs, typical. Measured about 43 µs, with or
    // without a mind's costs.
    assert!(each < 150.0e-6, "{each}");
}

/// A mover's believed rules without what its mind adds.
fn believed_alone(sim: &Sim, who: EntityId) -> impl MoveRules + '_ {
    let rules = clash_sim::movement::believed_rules(sim, who).through_openable();
    clash_sim::perception::believed_move_rules_with(sim, who, rules).on_grid(GRID)
}
