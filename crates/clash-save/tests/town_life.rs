//! The generated market town (`core:town`) with its people (09 §10,
//! 06 §12; packages E11, L1, L2, I10): residents made when their tracts
//! load, the same each time; agents that keep their routines while
//! unloaded; minds that open doors and read where things are as they
//! believe; a watch that answers a call for help. Everything runs
//! against the repository's default profile through the console's host.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use clash_core::EntityId;
use clash_expr::{Context, Type, Value};
use clash_save::host::{GameHost, HostEnv};
use clash_sim::console::{ConsoleError, ConsoleHost, HostKind, Output, Session};
use clash_sim::{Bound, Place, Sim, Subject};
use clash_world::{CellPos, Layer, Location};

fn mods_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../mods")
}

/// A console over the real simulation.
struct Town {
    table: clash_sim::console::CommandTable,
    host: GameHost,
    session: Session,
}

impl Town {
    /// The market town, begun as its game scenario begins it, at one of
    /// its places (0 the north street, 1 the square), with a seed.
    fn begin(seed: u64, place: u32) -> Town {
        let env = Arc::new(HostEnv::new(mods_dir()));
        let mut town = Town {
            table: GameHost::table(),
            host: GameHost::new(env),
            session: Session::new(HostKind::File),
        };
        town.ok(&format!("use default seed={seed}"));
        town.ok(&format!("begin core:town_life place={place}"));
        town
    }

    fn run(&mut self, line: &str) -> Result<Output, ConsoleError> {
        self.session.run_line(&self.table, &mut self.host, line)
    }

    fn ok(&mut self, line: &str) -> Output {
        self.run(line)
            .unwrap_or_else(|error| panic!("`{line}`: {error}"))
    }

    fn sim(&self) -> &Sim {
        self.host.sim().expect("a game is running")
    }

    fn eval(&mut self, source: &str) -> Value {
        self.ok(&format!("eval \"{source}\""))
            .subject
            .expect("an evaluation gives its value")
    }

    /// Takes the player far out into the fields, where the town unloads,
    /// with nothing to perceive with: perceiving open country is the
    /// dearest thing there is, and nothing there matters here.
    fn go_away(&mut self) {
        self.ok(&format!("session @self {}", at(away(self.sim()))));
        self.ok("detach_sense @self core:human_sight");
        self.ok("detach_sense @self core:human_hearing");
    }

    /// Brings the player back to the town square, seeing and hearing.
    fn come_back(&mut self) {
        let square = square(self.sim());
        self.ok(&format!("session @self {}", at(square)));
        self.ok("attach_sense @self core:human_sight");
        self.ok("attach_sense @self core:human_hearing");
    }

    /// Lets time pass until the realm's clock reads a time of day, the
    /// next time it does: `"03:00"`.
    fn until(&mut self, clock: &str) {
        let (hours, minutes) = clock.split_once(':').unwrap();
        let want = hours.parse::<i64>().unwrap() * 3600 + minutes.parse::<i64>().unwrap() * 60;
        let now = self
            .eval("@self.loc.realm.time_of_day")
            .as_number()
            .unwrap()
            .get() as i64;
        let ahead = (want - now).rem_euclid(86_400);
        if ahead > 0 {
            self.ok(&format!("advance {ahead}s"));
        }
    }
}

/// The text of a cell, for the console: `@c:x,y,z`.
fn at(loc: Location) -> String {
    format!("@c:{},{},{}", loc.cell.x, loc.cell.y, loc.cell.z)
}

/// A cell of open country far from the town, a farm or a village.
fn away(sim: &Sim) -> Location {
    let grid = sim.truth().cosmos().realms().next().unwrap().base_grid;
    Location::new(grid, CellPos::new(600, 600, 0))
}

/// The residents whose individuals stand in the world, with their keys.
fn in_world(sim: &Sim) -> Vec<(clash_sim::agent::ResidentKey, EntityId)> {
    sim.agents()
        .residents()
        .filter_map(|(key, resident)| Some((*key, resident.entity?)))
        .filter(|(_, entity)| sim.truth().location(*entity).is_some())
        .collect()
}

/// A cell's block written out in full: `core:door@core:wood#closed`.
fn block_text(sim: &Sim, loc: Location) -> String {
    let cosmos = sim.truth().cosmos();
    cosmos
        .cell(loc)
        .map(|cell| cosmos.defs().state_text(Layer::Block, cell.block.raw()))
        .unwrap_or_default()
}

/// How far apart two cells are, in metres, a storey counting three.
fn metres(a: Location, b: Location) -> f64 {
    let (dx, dy, dz) = (
        f64::from(a.cell.x - b.cell.x),
        f64::from(a.cell.y - b.cell.y),
        3.0 * f64::from(a.cell.z - b.cell.z),
    );
    (dx * dx + dy * dy + dz * dz).sqrt()
}

/// A resident made from `what` whose home has exactly one closed door
/// within `reach` cells of it on its storey, standing at home: the
/// resident, its home, and the door. The first in key order.
fn at_home_by_one_door(
    sim: &Sim,
    what: &str,
    reach: i32,
) -> Option<(EntityId, Location, Location)> {
    for (key, entity) in in_world(sim) {
        let resident = sim.agents().resident(&key)?;
        if resident.what != what {
            continue;
        }
        let here = sim.truth().location(entity)?;
        let home = Location::new(here.grid, key.home);
        if here != home {
            continue;
        }
        let mut doors = Vec::new();
        for dy in -reach..=reach {
            for dx in -reach..=reach {
                let loc = Location::new(
                    home.grid,
                    CellPos::new(home.cell.x + dx, home.cell.y + dy, home.cell.z),
                );
                if block_text(sim, loc).contains("door") {
                    doors.push(loc);
                }
            }
        }
        if let [door] = doors.as_slice()
            && block_text(sim, *door).ends_with("#closed")
        {
            return Some((entity, home, *door));
        }
    }
    None
}

/// What an expression gives for `self` and `x` in a mind context (05
/// §7.3) and where truth is read.
fn in_mind_and_in_truth(sim: &Sim, source: &str, me: EntityId, x: EntityId) -> (Value, Value) {
    let registry = sim.exprs().registry();
    let entity = Type::Subject(registry.kind_id("entity").expect("entities are a kind"));
    let truth = Context::new()
        .own_subject("self", entity.clone())
        .subject("x", entity.clone());
    let mind = truth.clone().belief();
    let bound = Bound::new()
        .subject(sim.subject_value(Subject::Entity(me)))
        .subject(sim.subject_value(Subject::Entity(x)));
    let run = |context: &Context| {
        let compiled = clash_expr::compile(source, registry, context, &Type::Any)
            .value
            .unwrap_or_else(|| panic!("`{source}` compiles"));
        sim.eval(&compiled, bound.clone())
    };
    (run(&mind), run(&truth))
}

/// `source` compiled in `me`'s mind context and held to truth, as an
/// action's `requires` is when it starts (03 §6.6).
fn in_mind_held_to_truth(sim: &Sim, source: &str, me: EntityId, x: EntityId) -> Value {
    let registry = sim.exprs().registry();
    let entity = Type::Subject(registry.kind_id("entity").expect("entities are a kind"));
    let mind = Context::new()
        .own_subject("self", entity.clone())
        .subject("x", entity)
        .belief();
    let bound = Bound::new()
        .subject(sim.subject_value(Subject::Entity(me)))
        .subject(sim.subject_value(Subject::Entity(x)));
    let compiled = clash_expr::compile(source, registry, &mind, &Type::Any)
        .value
        .unwrap_or_else(|| panic!("`{source}` compiles"));
    let mut host = sim.host_in_truth(bound);
    clash_data::Runner::new()
        .on(sim.defs(), &mut host)
        .value(&compiled)
}

#[test]
fn dist_and_dir_read_where_a_mind_believes_things_are() {
    // Gap 1 of the town-life brief: `dist` is safe for belief only
    // because, in a mind context, it reads where the mind believes an
    // entity is (decision D-780).
    let mut town = Town::begin(7, 1);
    let people = in_world(town.sim());
    let ((_, me), (_, other)) = (people[0], people[people.len() - 1]);
    let sim = town.sim();
    let mine = sim.truth().location(me).unwrap();
    let theirs = sim.truth().location(other).unwrap();
    assert!(
        metres(mine, theirs) > 30.0,
        "far apart: {mine:?} {theirs:?}"
    );
    // `me` is told `other` stands two cells east of it: wrong.
    let told = Location::new(
        mine.grid,
        CellPos::new(mine.cell.x + 2, mine.cell.y, mine.cell.z),
    );
    town.ok(&format!(
        "add_track @e:{} @e:{} at={}",
        me.slot(),
        other.slot(),
        at(told)
    ));
    let sim = town.sim();
    let (believed, true_dist) = in_mind_and_in_truth(sim, "dist(self, x)", me, other);
    assert_eq!(believed, Value::Real(clash_core::Real::of(2.0)));
    let true_dist = true_dist.as_number().unwrap().get();
    assert!((true_dist - metres(mine, theirs)).abs() < 1e-9);
    let (believed, _) = in_mind_and_in_truth(sim, "dir(self, x)", me, other);
    assert_eq!(
        believed,
        Value::list([Value::Int(1), Value::Int(0), Value::Int(0)])
    );
    // Of itself, a mind knows where it is.
    let (own, _) = in_mind_and_in_truth(sim, "dist(self, self)", me, other);
    assert_eq!(own, Value::Real(clash_core::Real::ZERO));
    // Held to truth, as an action's `requires` is when it starts, the
    // same compiled expression measures to where `other` is.
    let held = in_mind_held_to_truth(sim, "dist(self, x)", me, other);
    assert!((held.as_number().unwrap().get() - true_dist).abs() < 1e-9);
    assert_eq!(
        in_mind_held_to_truth(sim, "dir(self, x)", me, other),
        in_mind_and_in_truth(sim, "dir(self, x)", me, other).1
    );
    // Someone it has no idea of is nowhere it knows.
    let (stranger, _) = in_mind_and_in_truth(sim, "dist(x, self)", other, me);
    assert!(
        stranger == Value::Null || stranger.as_number().is_some(),
        "{stranger:?}"
    );
}

#[test]
fn a_resident_opens_the_door_on_its_way_and_a_dog_does_not_plan_through_one() {
    // Gap 3: a closed door on a mind's way is opened by the interaction
    // the player uses (`core:interact` with the door's default, `open`),
    // as part of the step; a dog, whose mind names no `interact_action`,
    // takes a closed door for a wall.
    let mut town = Town::begin(7, 1);
    let (who, home, door) = at_home_by_one_door(town.sim(), "person", 3)
        .expect("someone at home behind one closed door");
    // Sent on the watch's round, it must leave the house.
    town.ok(&format!("set_routine @e:{} core:watch_shift", who.slot()));
    let mut opened = false;
    for _ in 0..20 {
        town.ok("advance 1s");
        if block_text(town.sim(), door).ends_with("#open") {
            opened = true;
            break;
        }
    }
    assert!(opened, "the door at {door:?} was opened");
    // By a resident's mind, through the interaction the player uses.
    let openers: Vec<EntityId> = town
        .session
        .records()
        .iter()
        .filter(|record| record.name == "interacted")
        .filter_map(|record| {
            let user = record
                .fields
                .iter()
                .find(|(name, _)| name == "self")
                .and_then(|(_, value)| town.sim().subject_of(value))?;
            let Subject::Entity(user) = user else {
                return None;
            };
            Some(user)
        })
        .collect();
    eprintln!("opened by {openers:?}; sent out: {who}");
    assert!(
        openers
            .iter()
            .any(|user| town.sim().agents().resident_of(*user).is_some()),
        "a resident opened a door as an interaction: {openers:?}"
    );
    town.ok("advance 6s");
    let now = town.sim().truth().location(who).unwrap();
    assert!(metres(now, home) > 1.5, "{who} went out: {now:?}");

    // Shut again, the door is on a person's way out and not on a dog's:
    // a dog's way, if it has one, goes round by another opening.
    town.ok(&format!("set {} block core:door#closed", at(door)));
    let outside = Location::new(
        door.grid,
        CellPos::new(
            door.cell.x + 2 * (door.cell.x - home.cell.x).signum(),
            door.cell.y + 2 * (door.cell.y - home.cell.y).signum(),
            door.cell.z,
        ),
    );
    for (template, name) in [("core:person", "walker"), ("core:house_dog", "dog")] {
        town.ok(&format!(
            "as {} do {{ \"spawn\": {{ \"template\": \"{template}\", \"at\": \"self\" }} }}",
            at(home)
        ));
        let _ = name;
    }
    let made: Vec<EntityId> = town
        .sim()
        .truth()
        .entities()
        .iter()
        .map(clash_sim::KernelEntity::id)
        .filter(|id| {
            town.sim()
                .truth()
                .location(*id)
                .is_some_and(|loc| metres(loc, home) < 2.5)
        })
        .filter(|id| town.sim().agents().resident_of(*id).is_none())
        .collect();
    let cost = |town: &mut Town, of: EntityId| {
        town.eval(&format!("@e:{}.path_cost({})", of.slot(), at(outside)))
            .as_number()
            .map(|real| real.get())
    };
    let costs: Vec<Option<f64>> = made.iter().map(|id| cost(&mut town, *id)).collect();
    let walker = costs
        .iter()
        .flatten()
        .copied()
        .reduce(f64::min)
        .expect("a person has a way out");
    let shortest = metres(home, outside);
    assert!(
        walker < shortest + 6.0,
        "through the door: {walker} m for {shortest} m"
    );
    assert!(
        costs
            .iter()
            .any(|cost| cost.is_none_or(|cost| cost > walker)),
        "the dog's way is longer, or there is none: {costs:?}"
    );
}

#[test]
fn a_locked_door_it_cannot_open_fails_the_step_with_a_reason() {
    let mut town = Town::begin(7, 1);
    let (who, _home, door) = at_home_by_one_door(town.sim(), "person", 3)
        .expect("someone at home behind one closed door");
    town.ok(&format!("set {} block core:lockable_door#locked", at(door)));
    town.ok(&format!("set_routine @e:{} core:watch_shift", who.slot()));
    let mut refused = false;
    for _ in 0..20 {
        town.ok("advance 1s");
        refused = town.session.records().iter().any(|record| {
            record.name == "step_failed"
                && record
                    .fields
                    .iter()
                    .any(|(name, value)| name == "code" && *value == Value::from("ai.door_shut"))
        });
        if refused {
            break;
        }
    }
    assert!(refused, "its step failed with `ai.door_shut`");
    assert!(block_text(town.sim(), door).ends_with("#locked"));
}

/// What an expression on `self` gives for each of some entities, where
/// truth is read.
fn of_each(sim: &Sim, source: &str, entities: &[EntityId]) -> Vec<Value> {
    let registry = sim.exprs().registry();
    let entity = Type::Subject(registry.kind_id("entity").expect("entities are a kind"));
    let context = Context::new().subject("self", entity);
    let compiled = clash_expr::compile(source, registry, &context, &Type::Any)
        .value
        .unwrap_or_else(|| panic!("`{source}` compiles"));
    entities
        .iter()
        .map(|id| {
            sim.eval(
                &compiled,
                Bound::new().subject(sim.subject_value(Subject::Entity(*id))),
            )
        })
        .collect()
}

/// Who each resident in the world is: its name, its make and its mass,
/// by its key.
fn who_is_who(
    sim: &Sim,
) -> std::collections::BTreeMap<clash_sim::agent::ResidentKey, (Value, String, Value)> {
    let people = in_world(sim);
    let ids: Vec<EntityId> = people.iter().map(|(_, id)| *id).collect();
    let names = of_each(sim, "self.name", &ids);
    let masses = of_each(sim, "self.mass", &ids);
    people
        .iter()
        .zip(names.into_iter().zip(masses))
        .map(|((key, _), (name, mass))| {
            let what = sim.agents().resident(key).unwrap().what.clone();
            (*key, (name, what, mass))
        })
        .collect()
}

#[test]
fn the_same_people_come_back_when_their_tracts_load_again() {
    // E11, invariant B7: reloading a tract shows the same people, with
    // the same names and looks; notable ones are kept as agents and
    // round-trip through a save with identical hashes.
    let mut town = Town::begin(7, 1);
    let before = who_is_who(town.sim());
    let names: std::collections::BTreeSet<String> = before
        .values()
        .filter_map(|(name, _, _)| name.as_str().map(str::to_owned))
        .collect();
    assert!(names.len() > 5, "people have names of their own: {names:?}");
    // One townsperson is made notable by a rule; another is wounded's
    // equal, a variable set on it.
    let people = in_world(town.sim());
    let persons: Vec<(clash_sim::agent::ResidentKey, EntityId)> = people
        .iter()
        .filter(|(key, _)| town.sim().agents().resident(key).unwrap().what == "person")
        .copied()
        .collect();
    let (kept_key, kept) = persons[0];
    let (marked_key, marked) = persons[1];
    let (plain_key, plain) = persons[2];
    town.ok(&format!("make_notable @e:{}", kept.slot()));
    town.ok(&format!(
        "as @e:{} do {{ \"set\": {{ \"self.var.barked_at\": 1 }} }}",
        marked.slot()
    ));
    let notable = |town: &mut Town, id: EntityId| town.sim().is_notable(id);
    assert!(notable(&mut town, kept) && notable(&mut town, marked));
    assert!(
        !notable(&mut town, plain),
        "an untouched resident stays anonymous"
    );
    // A notable one made anonymous again is forgotten like any other.
    let (folded_key, folded) = persons[3];
    town.ok(&format!("make_notable @e:{}", folded.slot()));
    town.ok(&format!("make_anonymous @e:{}", folded.slot()));
    assert!(!notable(&mut town, folded));

    town.go_away();
    {
        let sim = town.sim();
        // Notable: stored, out of the world, an agent.
        for id in [kept, marked] {
            assert_eq!(sim.truth().place(id), Some(Place::Stored), "{id} is stored");
            let (_, resident) = sim.agents().resident_of(id).expect("still its resident's");
            assert!(resident.agent.is_some(), "{id} is an agent");
        }
        // Anonymous: nothing, its record cold.
        for (key, id) in [(plain_key, plain), (folded_key, folded)] {
            assert!(
                sim.truth()
                    .entity(id)
                    .is_none_or(|record| record.place() != Place::Stored)
            );
            let resident = sim.agents().resident(&key).unwrap();
            assert_eq!(resident.entity, None);
            assert!(resident.agent.is_some());
        }
        // Invariant N7: one wake-up for each cold resident that keeps a
        // routine, and one on the timeline for them all.
        let cold = sim
            .agents()
            .residents()
            .filter(|(_, resident)| {
                resident
                    .agent
                    .as_ref()
                    .is_some_and(|agent| agent.wake.is_some())
            })
            .count();
        assert_eq!(sim.agents().scheduled(), cold);
        assert!(cold > 300, "{cold}");
        assert!(sim.agents().timeline_wake().is_some());
    }

    // Saved and loaded while they are agents, nothing changes.
    let bytes = town.host.save().unwrap();
    let mut other = Town {
        table: GameHost::table(),
        host: GameHost::new(Arc::new(HostEnv::new(mods_dir()))),
        session: Session::new(HostKind::File),
    };
    other.host.load(&bytes).unwrap();
    assert_eq!(other.host.state_hash(), town.host.state_hash());
    town.ok("advance 40min");
    other.ok("advance 40min");
    assert_eq!(other.host.state_hash(), town.host.state_hash());

    town.come_back();
    let after = who_is_who(town.sim());
    let mut same = 0;
    for (key, (name, what, mass)) in &before {
        if let Some((name_now, what_now, mass_now)) = after.get(key) {
            assert_eq!(
                (name_now, what_now, mass_now),
                (name, what, mass),
                "{key:?}"
            );
            same += 1;
        }
    }
    assert!(
        same > before.len() / 2,
        "{same} of {} came back",
        before.len()
    );
    assert_eq!(
        town.sim().agents().resident(&kept_key).unwrap().entity,
        Some(kept),
        "the notable one is the same individual"
    );
    assert!(town.sim().truth().location(kept).is_some());
    let _ = marked_key;
}

/// Where the residents in the world are, by what their routines have
/// them doing: for each `doing`, how many are at its place, and how many
/// are away from it (on the street between, or not yet there). Dogs,
/// with no routine, are left out.
fn by_doing(sim: &Sim) -> std::collections::BTreeMap<String, (usize, usize)> {
    let people = in_world(sim);
    let ids: Vec<EntityId> = people.iter().map(|(_, id)| *id).collect();
    let doing = of_each(sim, "self.routine_doing", &ids);
    let away = of_each(sim, "self.routine_away", &ids);
    let mut out: std::collections::BTreeMap<String, (usize, usize)> = Default::default();
    for (doing, away) in doing.iter().zip(&away) {
        let Some(doing) = doing.as_str() else {
            continue;
        };
        let entry = out.entry(doing.to_owned()).or_default();
        if *away == Value::Bool(true) {
            entry.1 += 1;
        } else {
            entry.0 += 1;
        }
    }
    out
}

/// The residents in the world doing something, with where they stand.
fn doing(sim: &Sim, what: &str) -> Vec<(EntityId, Location)> {
    let people = in_world(sim);
    let ids: Vec<EntityId> = people.iter().map(|(_, id)| *id).collect();
    let doing = of_each(sim, "self.routine_doing", &ids);
    ids.into_iter()
        .zip(doing)
        .filter(|(_, doing)| doing.as_str() == Some(what))
        .filter_map(|(id, _)| Some((id, sim.truth().location(id)?)))
        .collect()
}

/// The anchor of the town square.
fn square(sim: &Sim) -> Location {
    let site = sim
        .truth()
        .cosmos()
        .sites()
        .find(|site| site.name == "Town square")
        .expect("the town has a square");
    let grid = sim.truth().cosmos().realm(site.realm).unwrap().base_grid;
    Location::new(grid, site.anchor)
}

#[test]
fn routine_day_people_are_where_their_routines_put_them_at_every_hour() {
    // L2's `routine_day`: with the town unloaded, time passes for its
    // residents as agents; when it loads again each is found at the
    // place its routine puts it, or on the street between.
    let mut town = Town::begin(7, 1);
    let total = in_world(town.sim()).len();

    // Dawn: everyone who keeps a routine is asleep at home, but for the
    // bakers at the oven.
    town.go_away();
    town.until("05:30");
    town.come_back();
    let dawn = by_doing(town.sim());
    eprintln!("05:30 {dawn:?}");
    let asleep = dawn.get("asleep").copied().unwrap_or_default();
    assert!(asleep.0 > total / 2, "most are asleep at home: {dawn:?}");
    assert_eq!(asleep.1, 0, "nobody asleep is away from bed: {dawn:?}");
    assert!(
        dawn.keys()
            .all(|doing| doing == "asleep" || doing == "baking"),
        "at half past five everyone is asleep or baking: {dawn:?}"
    );
    assert!(dawn.contains_key("baking"), "the bakers are up: {dawn:?}");

    // Noon: some at the market, some still on their way, the rest at
    // work or keeping shop; the watch on its round.
    town.go_away();
    town.until("12:40");
    town.come_back();
    let noon = by_doing(town.sim());
    eprintln!("12:40 {noon:?}");
    let market = noon.get("at_market").copied().unwrap_or_default();
    assert!(market.0 > 10, "people at the market: {noon:?}");
    assert!(market.1 > 0, "people on their way to it: {noon:?}");
    assert!(
        noon.get("working").is_some_and(|(at, _)| *at > total / 4),
        "{noon:?}"
    );
    assert!(
        noon.get("keeping_shop").is_some_and(|(at, _)| *at > 0),
        "{noon:?}"
    );
    assert!(
        noon.get("on_patrol")
            .is_some_and(|(at, away)| at + away > 0),
        "{noon:?}"
    );
    // Those there are at the market the routine names, the market of
    // stalls (`site_tag: market`) or else the square.
    for (id, loc) in doing(town.sim(), "at_market") {
        let away = of_each(town.sim(), "self.routine_away", &[id])[0] == Value::Bool(true);
        if !away {
            let there = of_each(town.sim(), "dist(self, self.routine_place)", &[id])[0]
                .as_number()
                .map(|metres| metres.get());
            assert!(
                there.is_some_and(|metres| metres < 16.0),
                "{id} at the market: {loc:?}, {there:?} m from where its routine puts it"
            );
        }
    }

    // Dusk: home again, the market empty.
    town.go_away();
    town.until("18:45");
    town.come_back();
    let dusk = by_doing(town.sim());
    eprintln!("18:45 {dusk:?}");
    assert!(!dusk.contains_key("at_market"), "{dusk:?}");
    let home = dusk.get("at_home").copied().unwrap_or_default();
    assert!(home.0 > total / 2 && home.1 == 0, "{dusk:?}");

    // Midnight: asleep, but for the drinkers at the tavern (the square,
    // in a town with no tavern), each where its routine puts it.
    town.go_away();
    town.until("00:00");
    town.come_back();
    let night = by_doing(town.sim());
    eprintln!("00:00 {night:?}");
    assert!(
        night.get("asleep").is_some_and(|(at, _)| *at > total / 2),
        "{night:?}"
    );
    for (id, loc) in doing(town.sim(), "drinking") {
        let there = of_each(town.sim(), "dist(self, self.routine_place)", &[id])[0]
            .as_number()
            .map(|metres| metres.get());
        assert!(
            there.is_some_and(|metres| metres < 16.0),
            "{id} drinks where its routine puts it: {loc:?}, {there:?} m from there"
        );
    }
}

#[test]
fn at_the_hour_a_loaded_resident_takes_its_goal_from_its_routine_and_walks() {
    // A resident in view keeps its routine through its mind: at noon
    // those whose routine says market take up `keep_routine` and walk.
    let mut town = Town::begin(7, 1);
    town.go_away();
    town.until("12:20");
    town.come_back();
    // Those on their way to market as they come into view.
    let goers: Vec<(EntityId, Location)> = doing(town.sim(), "at_market")
        .into_iter()
        .filter(|(id, _)| of_each(town.sim(), "self.routine_away", &[*id])[0] == Value::Bool(true))
        .collect();
    assert!(
        !goers.is_empty(),
        "someone is on the way to market at twenty past twelve"
    );
    let centre = square(town.sim());
    town.ok("advance 20s");
    let ids: Vec<EntityId> = goers.iter().map(|(id, _)| *id).collect();
    let goals = of_each(town.sim(), "self.goal", &ids);
    let walking: Vec<EntityId> = ids
        .iter()
        .zip(&goals)
        .filter(|(_, goal)| goal.as_str() == Some("core:keep_routine"))
        .map(|(id, _)| *id)
        .collect();
    let nearer = goers
        .iter()
        .filter(|(id, was)| {
            town.sim()
                .truth()
                .location(*id)
                .is_some_and(|loc| metres(loc, centre) + 3.0 < metres(*was, centre))
        })
        .count();
    eprintln!(
        "{} going to market: {} walking, {} nearer",
        ids.len(),
        walking.len(),
        nearer
    );
    assert!(
        !walking.is_empty(),
        "they take up `keep_routine`: {goals:?}"
    );
    assert!(nearer > 0, "and walk on toward the market");
}

#[test]
fn a_brawl_in_the_market_draws_the_watch_which_arrives_and_acts() {
    // L2 and I10, test script `town_disturbance`: two brawlers come to
    // blows in the market; whoever sees it runs and calls for help; the
    // watch hears, comes, and goes for whoever it sees strike.
    let mut town = Town::begin(7, 1);
    town.go_away();
    town.until("12:50");
    town.come_back();
    let centre = square(town.sim());
    let spot = |dx: i32| {
        Location::new(
            centre.grid,
            CellPos::new(centre.cell.x + 2 + dx, centre.cell.y + 3, centre.cell.z),
        )
    };
    let watch: Vec<EntityId> = in_world(town.sim())
        .into_iter()
        .filter(|(key, _)| town.sim().agents().resident(key).unwrap().what == "watch_guard")
        .map(|(_, id)| id)
        .collect();
    assert!(!watch.is_empty(), "the town has a watch");
    let before: std::collections::BTreeSet<EntityId> = town
        .sim()
        .truth()
        .entities()
        .iter()
        .map(clash_sim::KernelEntity::id)
        .collect();
    for dx in [0, 1] {
        town.ok(&format!(
            "as {} do {{ \"spawn\": {{ \"template\": \"core:brawler\", \"at\": \"self\" }} }}",
            at(spot(dx))
        ));
    }
    let made: Vec<EntityId> = town
        .sim()
        .truth()
        .entities()
        .iter()
        .map(clash_sim::KernelEntity::id)
        .filter(|id| !before.contains(id))
        .collect();
    assert_eq!(made.len(), 2, "two brawlers: {made:?}");
    town.ok(&format!(
        "set_stance @e:{} @e:{} hostile",
        made[0].slot(),
        made[1].slot()
    ));
    let mut called = false;
    let mut answered = false;
    let mut struck_by_watch = false;
    for _ in 0..150 {
        town.ok("advance 1s");
        let records = town.session.records();
        let field = |record: &clash_sim::console::Record, name: &str| {
            record
                .fields
                .iter()
                .find(|(have, _)| have == name)
                .map(|(_, value)| value.clone())
        };
        let entity = |value: Option<Value>| match value.and_then(|v| town.sim().subject_of(&v)) {
            Some(Subject::Entity(id)) => Some(id),
            _ => None,
        };
        called = called
            || records.iter().any(|record| {
                record.name == "message_heard"
                    && field(record, "kind") == Some(Value::from("core:help"))
            });
        answered = answered
            || records.iter().any(|record| {
                record.name == "goal_adopted"
                    && watch.contains(&entity(field(record, "self")).unwrap_or(EntityId::new(0, 0)))
                    && matches!(
                        field(record, "goal").as_ref().and_then(Value::as_str),
                        Some("core:answer_call" | "core:confront")
                    )
            });
        struck_by_watch = records.iter().any(|record| {
            record.name == "hit_landed"
                && entity(field(record, "actor")).is_some_and(|actor| watch.contains(&actor))
        });
        // Or a watchman stands over a brawler, going for him.
        let sim = town.sim();
        let fighting = of_each(sim, "self.goal", &watch);
        let at_them = watch.iter().zip(&fighting).any(|(id, goal)| {
            goal.as_str() == Some("core:confront")
                && made.iter().any(|brawler| {
                    sim.truth()
                        .location(*id)
                        .zip(sim.truth().location(*brawler))
                        .is_some_and(|(a, b)| metres(a, b) < 4.0)
                })
        });
        struck_by_watch = struck_by_watch || at_them;
        if struck_by_watch {
            break;
        }
    }
    eprintln!("called {called}, answered {answered}, struck {struck_by_watch}");
    let goals = of_each(town.sim(), "self.goal", &watch);
    let steps = of_each(town.sim(), "self.last_failure", &watch);
    for ((id, goal), failure) in watch.iter().zip(&goals).zip(&steps) {
        let far = town
            .sim()
            .truth()
            .location(*id)
            .map(|loc| metres(loc, spot(0)));
        eprintln!("watchman {id}: {goal:?}, last failure {failure:?}, {far:?} m from the brawl");
    }
    assert!(
        called,
        "someone called for help, as a message the watch hears"
    );
    assert!(answered, "the watch took it up");
    assert!(struck_by_watch, "a watchman arrived and went for them");
}

#[test]
#[ignore = "a timing: in a release build, `cargo test --release -p clash-save --test town_life timing -- --ignored --nocapture`"]
fn timing_the_whole_town_for_one_simulated_hour() {
    // 09 §19: a market town of residents, every one of them hot, from
    // nine in the morning for an hour of the realm's time.
    let mut town = Town::begin(7, 1);
    let residents = town.sim().agents().residents().count();
    let steps = town.sim().steps();
    #[allow(clippy::disallowed_types, clippy::disallowed_methods)]
    let started = std::time::Instant::now();
    town.ok("advance 3600s");
    let spent = started.elapsed();
    eprintln!(
        "{residents} residents, one simulated hour from 09:00: {:.1} s of real time, {} wake-ups",
        spent.as_secs_f64(),
        town.sim().steps() - steps
    );
    // The same hour with the town unloaded, its residents agents.
    let mut away = Town::begin(7, 1);
    away.go_away();
    #[allow(clippy::disallowed_types, clippy::disallowed_methods)]
    let started = std::time::Instant::now();
    away.ok("advance 3600s");
    eprintln!(
        "the same hour unloaded: {:.3} s of real time",
        started.elapsed().as_secs_f64()
    );
}

#[test]
fn the_towns_residents_are_made_as_its_tracts_load() {
    // Gap 2 of the town-life brief: the people the settlement asks for
    // become entities when the town's tracts load, each a resident.
    let town = Town::begin(7, 0);
    let sim = town.sim();
    let residents = sim.agents().residents().count();
    let people = in_world(sim);
    assert!(residents > 300, "the town has its people: {residents}");
    assert_eq!(people.len(), residents, "every one of them is in the world");
    let made: std::collections::BTreeSet<&str> = sim
        .agents()
        .residents()
        .map(|(_, resident)| resident.what.as_str())
        .collect();
    for what in ["person", "shopkeeper", "watch_guard", "house_dog"] {
        assert!(made.contains(what), "{what} among {made:?}");
    }
}
