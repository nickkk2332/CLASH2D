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
        self.ok("session @self @c:2007,994,0");
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

    /// The player's character.
    fn player(&mut self) -> EntityId {
        let value = self.eval("@self");
        match self.sim().subject_of(&value) {
            Some(Subject::Entity(entity)) => entity,
            other => panic!("`@self` is {other:?}"),
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

#[test]
fn the_towns_residents_are_made_as_its_tracts_load() {
    let town = Town::begin(7, 0);
    let sim = town.sim();
    let residents: Vec<_> = sim.agents().residents().collect();
    let people = in_world(sim);
    eprintln!(
        "{} residents, {} in the world",
        residents.len(),
        people.len()
    );
    assert!(
        residents.len() > 300,
        "the town has its people: {}",
        residents.len()
    );
    for site in sim.truth().cosmos().sites() {
        eprintln!(
            "site {} {:?} anchor {:?} tags {:?}",
            site.name, site.id, site.anchor, site.tags
        );
    }
    let mut shown = 0;
    for (key, entity) in &people {
        let grid = sim.truth().location(*entity).unwrap().grid;
        let home = Location::new(grid, key.home);
        let mut doors = Vec::new();
        for dy in -6..=6 {
            for dx in -6..=6 {
                let loc = Location::new(
                    grid,
                    CellPos::new(key.home.x + dx, key.home.y + dy, key.home.z),
                );
                let text = block_text(sim, loc);
                if text.contains("door") {
                    doors.push((dx, dy, text));
                }
            }
        }
        let resident = sim.agents().resident(key).unwrap();
        if shown < 25 {
            eprintln!(
                "{entity} {} home {:?} at {:?} doors {:?}",
                resident.what,
                home.cell,
                sim.truth().location(*entity).map(|l| l.cell),
                doors
            );
            shown += 1;
        }
    }
    let _ = (
        at,
        block_text,
        Place::Stored,
        CellPos::new(0, 0, 0),
        Subject::Global,
        Bound::new(),
        Context::new(),
        Type::Any,
    );
}

#[test]
#[ignore]
fn debug_far() {
    let mut town = Town::begin(7, 1);
    town.ok("session @self @c:600,600,0");
    town.ok("detach_sense @self core:human_sight");
    town.ok("detach_sense @self core:human_hearing");
    let before = town.sim().steps();
    eprintln!(
        "scheduled {} wake {:?} entities {}",
        town.sim().agents().scheduled(),
        town.sim().agents().timeline_wake(),
        town.sim().truth().entities().iter().count()
    );
    let started = std::time::Instant::now();
    town.ok("advance 60s");
    eprintln!(
        "60 s: {} wake-ups in {:?}",
        town.sim().steps() - before,
        started.elapsed()
    );
    let mut kinds: std::collections::BTreeMap<String, usize> = std::collections::BTreeMap::new();
    for record in town.session.records() {
        *kinds.entry(record.name.clone()).or_default() += 1;
    }
    eprintln!("{kinds:?}");
}

#[test]
#[ignore]
fn debug_steps() {
    let mut town = Town::begin(7, 1);
    town.ok("set_routine @e:229 core:watch_shift");
    town.ok("advance 3s");
    for record in town.session.records() {
        let interesting = [
            "step_failed",
            "goal_failed",
            "path_failed",
            "goal_adopted",
            "action_refused",
            "step_refused",
        ];
        if interesting.contains(&record.name.as_str()) {
            let mine = record.fields.iter().any(|(name, value)| {
                name == "self"
                    && town.sim().subject_of(value) == Some(Subject::Entity(EntityId::new(229, 0)))
            });
            if mine {
                eprintln!("{} {:?}", record.name, record.fields);
            }
        }
    }
}
