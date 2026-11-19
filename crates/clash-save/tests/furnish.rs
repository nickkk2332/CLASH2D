//! The furnished town (10 §3.2, §4.1; 06 §9.3; 16 §4.3): the bundled
//! `core:town`, made from the repository's default profile at its
//! default seed, with the items, content tables and devices its maps
//! name made when its tracts first load.
//!
//! The bakery has its oven and its bread, the guardhouse its rack of
//! weapons, homes their food; what a container holds is latent until it
//! is opened and is the same whenever it is opened, after a save and a
//! load and after its tract was unloaded and loaded again; what is taken
//! stays taken; the same seed gives the same things and another seed a
//! town as valid; bread is baked at the town's oven; and saving,
//! loading and replay change nothing.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, LazyLock};

use clash_core::{EntityId, Tick};
use clash_data::Defs;
use clash_gen::map::MapDefs;
use clash_gen::place::PlacedMap;
use clash_gen::{GenDefs, StandardGenerator};
use clash_sim::action::{ActionDef, How, Params};
use clash_sim::item::{Item, ItemDef};
use clash_sim::template::CharacterSpec;
use clash_sim::{
    Advance, AdvanceLimit, Command, Control, DebugCommand, Place, Sim, SimEvent, Subject,
    debug_ticket,
};
use clash_time::Timeline;
use clash_world::{
    Cause, CellPos, Cosmos, GridId, Layer, Location, MemoryStore, RealmId, SerialRunner, WorldDefs,
};

/// The seed the new-game screen offers first: the town as a player
/// first sees it.
const DEFAULT_SEED: u64 = 1;
const SECOND: i64 = 1_000_000;

fn mods_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../mods")
}

/// The default profile, loaded once: its definitions, the world's
/// tables and what tracts are generated from.
static PROFILE: LazyLock<(Arc<Defs>, Arc<WorldDefs>, Arc<GenDefs>)> = LazyLock::new(|| {
    let types = clash_scene::engine_types();
    let defs = clash_scene::engine_loader(&types)
        .search_path(mods_dir())
        .load_profile("default")
        .into_result()
        .unwrap_or_else(|diagnostics| panic!("{}", diagnostics.render()));
    let world = WorldDefs::from_defs(&defs).unwrap_or_else(|d| panic!("{}", d.render()));
    let generation = GenDefs::from_defs(&defs, &world).unwrap_or_else(|d| panic!("{}", d.render()));
    (defs, world, generation)
});

/// A simulation over the bundled town at a seed, with nothing loaded
/// yet.
struct Town {
    sim: Sim,
    realm: RealmId,
    grid: GridId,
    /// The debug ticket that holds loaded what is round whoever visits.
    scout: Option<clash_world::TicketId>,
}

fn town(seed: u64) -> Town {
    let (defs, world, generation) = PROFILE.clone();
    let mut timeline = Timeline::new();
    let mut cosmos = Cosmos::new(Arc::clone(&world));
    let def = world.realms().lookup("core:town").unwrap();
    let realm = cosmos.create_realm(&mut timeline, def, seed).unwrap();
    let grid = cosmos.realm(realm).unwrap().base_grid;
    let mut registry = clash_gen::registry_with(&generation);
    registry.set_runner(Arc::new(SerialRunner));
    let generators = registry.bind(&world);
    let sim = Sim::from_parts(defs, cosmos, timeline, seed)
        .with_io(generators, Box::new(MemoryStore::new()));
    Town {
        sim,
        realm,
        grid,
        scout: None,
    }
}

fn maps() -> &'static Arc<MapDefs> {
    PROFILE.2.maps()
}

/// The marks of a site of the town at a seed, in world cells.
fn site_marks(seed: u64, site: &str) -> BTreeMap<String, Vec<CellPos>> {
    let generation = &PROFILE.2;
    let generator = StandardGenerator::of_realm(generation, "core:town").unwrap();
    let plan = generator.settlement_plan(seed, 0).unwrap();
    let id = generation.resolve_site(site, Some("core")).unwrap();
    let planned = plan
        .sites
        .iter()
        .find(|planned| planned.def == id)
        .unwrap_or_else(|| panic!("no {site} at seed {seed}"));
    PlacedMap::resolve(maps(), &planned.placement)
        .unwrap()
        .unwrap()
        .marks()
}

/// The first cell of a mark of a site.
fn mark(seed: u64, site: &str, name: &str) -> CellPos {
    site_marks(seed, site)
        .get(name)
        .and_then(|cells| cells.first().copied())
        .unwrap_or_else(|| panic!("{site} has no mark {name}"))
}

/// The homes of the town's lots, each with the cells of its kitchen
/// table: every lot whose map marks a kitchen.
fn kitchens(seed: u64) -> Vec<CellPos> {
    let generator = StandardGenerator::of_realm(&PROFILE.2, "core:town").unwrap();
    let plan = generator.settlement_plan(seed, 0).unwrap();
    plan.lots
        .iter()
        .filter_map(|lot| PlacedMap::resolve(maps(), &lot.placement).ok().flatten())
        .filter_map(|placed| placed.marks().get("kitchen").cloned())
        .flatten()
        .collect()
}

impl Town {
    fn at(&self, cell: CellPos) -> Location {
        Location::new(self.grid, cell)
    }

    /// Loads the tracts round a cell, as a debug ticket would.
    fn load_round(&mut self, cell: CellPos) -> clash_world::TicketId {
        let ticket = self
            .sim
            .hold_ticket(debug_ticket(self.realm, cell.tract(), 1))
            .unwrap();
        self.sim.reconcile_activity();
        self.sim.pump();
        ticket
    }

    /// A cell next to `cell` that a person can stand in.
    fn open_beside(&self, cell: CellPos) -> Location {
        let cosmos = self.sim.truth().cosmos();
        for (dx, dy) in [
            (0, 1),
            (1, 0),
            (0, -1),
            (-1, 0),
            (1, 1),
            (-1, 1),
            (1, -1),
            (-1, -1),
        ] {
            let at = self.at(CellPos::new(cell.x + dx, cell.y + dy, cell.z));
            let Ok(view) = cosmos.cell(at) else { continue };
            if !view.floor.is_none() && view.block.is_none() && view.fixture.is_none() {
                return at;
            }
        }
        panic!("nowhere to stand beside {cell:?}");
    }

    /// A person put next to a cell, under the player's control, with
    /// the tracts round it held loaded by a debug ticket: kept in
    /// `scout` until the person is moved away.
    fn visit(&mut self, cell: CellPos) -> EntityId {
        let scout = self.load_round(cell);
        let at = self.open_beside(cell);
        let who = self.sim.truth().entities().next_id().unwrap();
        self.sim
            .submit(Command::Debug(DebugCommand::Spawn {
                spec: CharacterSpec::of(["core:person"]),
                place: Place::at(at),
            }))
            .unwrap();
        // Its mind thinks by no `mind` definition: nothing moves it but
        // what the test does.
        self.sim.remove_mind(who, Cause::DEBUG).unwrap();
        self.sim.create_mind(who, None, Cause::DEBUG).unwrap();
        self.sim
            .submit(Command::Debug(DebugCommand::SetControl {
                entity: who,
                control: Control::Delegated,
            }))
            .unwrap();
        self.scout = Some(scout);
        self.sim.pump();
        who
    }

    /// Moves someone next to another cell, loading what is round it
    /// and letting go of what was held round them before: what is out
    /// of reach of every ticket is unloaded.
    fn go(&mut self, who: EntityId, cell: CellPos) {
        let next = self.load_round(cell);
        let at = self.open_beside(cell);
        self.sim.relocate(who, Place::at(at), Cause::DEBUG).unwrap();
        if let Some(old) = self.scout.replace(next) {
            self.sim.release_ticket(old);
        }
        self.run_for(SECOND);
    }

    /// The items lying in a cell, by definition and count, in id order.
    fn lying(&self, cell: CellPos) -> Vec<(String, u32)> {
        let at = self.sim.truth().cosmos().normalize(self.at(cell)).unwrap();
        self.items_where(|place| matches!(place, Place::At { loc, .. } if loc == at))
    }

    /// The items directly inside an entity.
    fn inside(&self, holder: EntityId) -> Vec<(String, u32)> {
        self.items_where(|place| place == Place::In(holder))
    }

    fn items_where(&self, wanted: impl Fn(Place) -> bool) -> Vec<(String, u32)> {
        let registry = self.sim.defs().try_registry::<ItemDef>().unwrap();
        self.sim
            .components()
            .store::<Item>()
            .iter()
            .filter(|(id, _)| self.sim.truth().place(*id).is_some_and(&wanted))
            .map(|(_, item)| {
                let name = item.def().map_or("", |def| registry.name(def)).to_owned();
                (name, item.count())
            })
            .collect()
    }

    fn element(&self, cell: CellPos, layer: Layer) -> String {
        let cosmos = self.sim.truth().cosmos();
        let state = cosmos
            .grid(self.grid)
            .unwrap()
            .element(cell, layer)
            .unwrap();
        cosmos.defs().state_text(layer, state)
    }

    fn latent(&self, cell: CellPos) -> bool {
        clash_sim::furnish::is_latent(&self.sim, self.at(cell), Layer::Fixture)
    }

    /// Opens a container fixture's latent contents, and returns what it
    /// holds then.
    fn open(&mut self, cell: CellPos) -> Vec<(String, u32)> {
        let at = self.at(cell);
        clash_sim::furnish::open_element(&mut self.sim, at, Layer::Fixture, Cause::DEBUG).unwrap();
        self.sim.pump();
        let device = clash_sim::device::device_at(&self.sim, at, Layer::Fixture).unwrap();
        self.inside(device)
    }

    fn run_for(&mut self, micros: i64) {
        let until = Tick(self.sim.now().as_micros() + micros);
        loop {
            match self.sim.advance(AdvanceLimit::Until(until)) {
                Advance::Reached(_) => return,
                Advance::Yielded => {}
                other => panic!("{other:?}"),
            }
        }
    }

    fn act(&mut self, who: EntityId, name: &str, params: Params) {
        let action = self
            .sim
            .defs()
            .lookup::<ActionDef>(&format!("core:{name}"))
            .unwrap();
        self.sim
            .submit(Command::Act {
                entity: who,
                action,
                params,
                how: How::Start,
            })
            .unwrap_or_else(|error| panic!("{name}: {error}"));
    }

    /// Does an interaction through the `interact` action, and waits for
    /// it to end.
    fn interact(&mut self, who: EntityId, cell: CellPos, interaction: &str) {
        let target = self
            .sim
            .subject_value(Subject::Element(self.at(cell), Layer::Fixture))
            .to_stored();
        let id = self
            .sim
            .defs()
            .lookup::<clash_sim::interaction::InteractionDef>(&format!("core:{interaction}"))
            .unwrap();
        let kind = self
            .sim
            .exprs()
            .registry()
            .def_type_id("interaction")
            .unwrap();
        let params = vec![
            ("target".to_owned(), target),
            (
                "interaction".to_owned(),
                clash_core::Value::Id {
                    ty: kind.raw(),
                    index: id.raw().into(),
                },
            ),
        ];
        self.act(who, "interact", params);
        self.run_for(5 * SECOND);
    }

    /// What someone holds in a grasp.
    fn in_hand(&self, who: EntityId) -> Vec<EntityId> {
        self.sim
            .truth()
            .entities()
            .contents(who)
            .iter()
            .copied()
            .filter(|id| self.sim.components().get::<Item>(*id).is_some())
            .collect()
    }
}

fn named<'a>(events: &'a [SimEvent], name: &str) -> Vec<&'a SimEvent> {
    events.iter().filter(|event| event.name() == name).collect()
}

/// Fails on any fault but those of the town's people finding their way
/// about (their minds do not open doors yet; that is the town's life,
/// not its furniture).
fn no_faults(events: &[SimEvent]) {
    let faults: Vec<String> = events
        .iter()
        .filter(|event| event.name() == "fault")
        .map(|event| format!("{event:?}"))
        .filter(|text| !text.contains("action core:go_to"))
        .collect();
    assert!(faults.is_empty(), "{faults:#?}");
}

/// The bakery of a town, loaded, with someone standing by its oven.
fn at_the_bakery(seed: u64) -> (Town, EntityId) {
    let mut town = town(seed);
    let oven = mark(seed, "bakery", "oven");
    let who = town.visit(oven);
    (town, who)
}

// ---- What the town holds ---------------------------------------------------------

#[test]
fn the_bakery_has_its_oven_and_its_bread() {
    let (mut town, _) = at_the_bakery(DEFAULT_SEED);
    let events = town.sim.drain_events();
    no_faults(&events);
    let oven = mark(DEFAULT_SEED, "bakery", "oven");
    assert!(town.element(oven, Layer::Fixture).starts_with("core:oven"));
    // It is a plain element until someone uses it (16 §4.3).
    assert!(clash_sim::device::device_at(&town.sim, town.at(oven), Layer::Fixture).is_none());
    // Loaves lie on the counter for anyone to take.
    let counters = site_marks(DEFAULT_SEED, "bakery")["counter"].clone();
    let loaves: u32 = counters
        .iter()
        .flat_map(|cell| town.lying(*cell))
        .filter(|(name, _)| name == "core:loaf")
        .map(|(_, count)| count)
        .sum();
    assert!(loaves >= 1, "no bread on the counter");
    // The rack of bread is latent until it is opened, and then holds
    // loaves; the cupboard holds flour and water.
    let rack = mark(DEFAULT_SEED, "bakery", "bread");
    assert!(town.latent(rack));
    let bread = town.open(rack);
    assert!(
        bread
            .iter()
            .any(|(name, count)| name == "core:loaf" && *count >= 3),
        "{bread:?}"
    );
    assert!(!town.latent(rack));
    let stores = town.open(mark(DEFAULT_SEED, "bakery", "stores"));
    assert!(
        stores.iter().any(|(name, _)| name == "core:flour"),
        "{stores:?}"
    );
    assert!(
        stores.iter().any(|(name, _)| name == "core:waterskin"),
        "{stores:?}"
    );
    no_faults(&town.sim.drain_events());
}

#[test]
fn the_guardhouse_has_its_rack_behind_a_locked_door_and_the_key_on_the_desk() {
    let mut town = town(DEFAULT_SEED);
    let rack = mark(DEFAULT_SEED, "guardhouse", "armoury");
    let desk = mark(DEFAULT_SEED, "guardhouse", "desk");
    town.visit(desk);
    assert!(town.element(rack, Layer::Fixture).starts_with("core:rack"));
    assert!(town.latent(rack));
    let held = town.open(rack);
    assert!(held.iter().any(|(name, _)| name == "core:club"), "{held:?}");
    assert_eq!(town.lying(desk), vec![("core:key".to_owned(), 1)]);
    // The armoury's door is locked.
    let marks = site_marks(DEFAULT_SEED, "guardhouse");
    let door_locked = marks["armoury"].iter().any(|cell| {
        (-1..=2).any(|dx| {
            (-1..=2).any(|dy| {
                town.element(CellPos::new(cell.x + dx, cell.y + dy, cell.z), Layer::Block)
                    .ends_with("#locked")
            })
        })
    });
    assert!(door_locked);
}

#[test]
fn homes_have_food_on_their_kitchen_tables() {
    let mut town = town(DEFAULT_SEED);
    let kitchens = kitchens(DEFAULT_SEED);
    assert!(kitchens.len() >= 10, "only {} kitchens", kitchens.len());
    let mut fed = 0;
    for &kitchen in kitchens.iter().take(12) {
        let scout = town.load_round(kitchen);
        let lying = town.lying(kitchen);
        if lying.iter().any(|(name, _)| {
            [
                "core:loaf",
                "core:waterskin",
                "core:mug_of_ale",
                "core:bottle",
            ]
            .contains(&name.as_str())
        }) {
            fed += 1;
        }
        town.sim.release_ticket(scout);
        town.sim.reconcile_activity();
    }
    assert!(fed >= 9, "only {fed} of 12 kitchens have food or drink");
}

// ---- Latent contents ---------------------------------------------------------------

#[test]
fn latent_contents_are_the_same_however_and_whenever_they_are_opened() {
    let stores = mark(DEFAULT_SEED, "bakery", "stores");
    // At once.
    let (mut fresh, _) = at_the_bakery(DEFAULT_SEED);
    let first = fresh.open(stores);
    assert!(!first.is_empty());
    // After a save and a load.
    let (saved, _) = at_the_bakery(DEFAULT_SEED);
    let blob = saved.sim.save().unwrap();
    let mut loaded = Town {
        sim: {
            let mut sim = Sim::load(
                Arc::clone(saved.sim.defs()),
                saved.sim.truth().cosmos().clone(),
                &blob,
            )
            .unwrap();
            assert!(sim.reindex().is_empty());
            sim
        },
        ..saved
    };
    assert!(loaded.latent(stores));
    assert_eq!(loaded.open(stores), first);
    // After its tract was unloaded and loaded again.
    let (mut away, who) = at_the_bakery(DEFAULT_SEED);
    away.go(who, far_from(stores));
    assert!(
        !away
            .sim
            .truth()
            .cosmos()
            .is_tract_loaded(away.realm, stores.tract())
    );
    away.go(who, stores);
    assert!(away.latent(stores));
    assert_eq!(away.open(stores), first);
}

/// A cell of the town far enough from another for the tracts round the
/// one to be unloaded when someone stands at the other: four tracts
/// south, outside the town.
fn far_from(cell: CellPos) -> CellPos {
    CellPos::new(cell.x, cell.y + 4 * clash_world::dims::TRACT_XY, cell.z)
}

#[test]
fn an_item_taken_stays_taken_through_a_save_and_an_unloading() {
    let (mut town, who) = at_the_bakery(DEFAULT_SEED);
    let counters = site_marks(DEFAULT_SEED, "bakery")["counter"].clone();
    let counter = *counters
        .iter()
        .find(|cell| !town.lying(**cell).is_empty())
        .unwrap();
    let before: u32 = town.lying(counter).iter().map(|(_, count)| count).sum();
    let loaf = town
        .sim
        .components()
        .store::<Item>()
        .iter()
        .map(|(id, _)| id)
        .find(|id| {
            matches!(town.sim.truth().place(*id), Some(Place::At { loc, .. }) if loc.cell == counter)
        })
        .unwrap();
    let taken = clash_sim::item::split(&mut town.sim, loaf, 1, Cause::DEBUG)
        .and_then(|piece| clash_sim::item::take(&mut town.sim, who, piece, Cause::DEBUG))
        .or_else(|_| clash_sim::item::take(&mut town.sim, who, loaf, Cause::DEBUG))
        .unwrap();
    town.sim.pump();
    let left: u32 = town.lying(counter).iter().map(|(_, count)| count).sum();
    assert_eq!(left, before - 1);
    // Saved and loaded: still taken.
    let blob = town.sim.save().unwrap();
    let mut sim = Sim::load(
        Arc::clone(town.sim.defs()),
        town.sim.truth().cosmos().clone(),
        &blob,
    )
    .unwrap();
    assert!(sim.reindex().is_empty());
    assert_eq!(sim.truth().place(taken), town.sim.truth().place(taken));
    // Carried away, the bakery unloaded and loaded again: nothing is
    // made twice.
    town.go(who, far_from(counter));
    assert!(
        !town
            .sim
            .truth()
            .cosmos()
            .is_tract_loaded(town.realm, counter.tract())
    );
    town.go(who, counter);
    let again: u32 = town.lying(counter).iter().map(|(_, count)| count).sum();
    assert_eq!(again, left);
    assert_eq!(town.sim.truth().place(taken), Some(Place::In(who)));
}

// ---- Seeds -----------------------------------------------------------------------

#[test]
fn the_same_seed_gives_the_same_things_and_another_a_town_as_full() {
    let (mut one, _) = at_the_bakery(DEFAULT_SEED);
    let (mut two, _) = at_the_bakery(DEFAULT_SEED);
    assert_eq!(one.sim.state_hash(), two.sim.state_hash());
    let rack = mark(DEFAULT_SEED, "bakery", "bread");
    assert_eq!(one.open(rack), two.open(rack));
    // Another seed: another town, with its own bakery and its own
    // things, just as furnished.
    let other_seed = 2;
    let (mut other, _) = at_the_bakery(other_seed);
    no_faults(&other.sim.drain_events());
    let oven = mark(other_seed, "bakery", "oven");
    assert!(other.element(oven, Layer::Fixture).starts_with("core:oven"));
    assert_ne!(oven, mark(DEFAULT_SEED, "bakery", "oven"));
    let bread = other.open(mark(other_seed, "bakery", "bread"));
    assert!(bread.iter().any(|(name, _)| name == "core:loaf"));
    let rack = mark(other_seed, "guardhouse", "armoury");
    let who = other.visit(rack);
    other.go(who, rack);
    assert!(other.latent(rack));
}

// ---- Baking ------------------------------------------------------------------------

#[test]
fn bread_is_baked_at_the_towns_oven() {
    let (mut town, who) = at_the_bakery(DEFAULT_SEED);
    let oven = mark(DEFAULT_SEED, "bakery", "oven");
    let stores = mark(DEFAULT_SEED, "bakery", "stores");
    let wood = mark(DEFAULT_SEED, "bakery", "wood");
    // The cupboard is opened: its flour and water are made.
    town.go(who, stores);
    town.interact(who, stores, "open");
    assert!(!town.latent(stores));
    // A log from the wood pile, into the oven, and the fire lit.
    town.go(who, wood);
    town.interact(who, wood, "retrieve");
    let logs: Vec<EntityId> = town
        .in_hand(who)
        .into_iter()
        .filter(|id| {
            town.sim
                .truth()
                .entities()
                .get(*id)
                .is_some_and(|e| e.tags.contains("fuel"))
        })
        .collect();
    assert!(!logs.is_empty(), "no log was taken from the pile");
    // Whatever else came out of the pile is put down.
    for id in town.in_hand(who) {
        if !logs.contains(&id) {
            clash_sim::item::drop(&mut town.sim, id, Cause::DEBUG).unwrap();
        }
    }
    town.go(who, oven);
    town.interact(who, oven, "stoke");
    town.interact(who, oven, "light_fire");
    let lit = clash_sim::device::device_at(&town.sim, town.at(oven), Layer::Fixture)
        .and_then(|device| clash_sim::device::mode_name(&town.sim, device));
    assert_eq!(lit.as_deref(), Some("lit"));
    let station = town
        .sim
        .subject_value(Subject::Element(town.at(oven), Layer::Fixture))
        .to_stored();
    town.sim.drain_events();
    town.act(
        who,
        "make",
        vec![
            (
                "recipe".to_owned(),
                clash_core::Value::Str("core:bread".to_owned()),
            ),
            ("station".to_owned(), station),
        ],
    );
    town.run_for(130 * SECOND);
    let events = town.sim.drain_events();
    no_faults(&events);
    assert_eq!(named(&events, "recipe_started").len(), 1);
    assert_eq!(named(&events, "recipe_completed").len(), 1);
    // A loaf came out of the oven, and the flour it was made from is
    // gone from the cupboard.
    let loaves: Vec<_> = town
        .in_hand(who)
        .into_iter()
        .chain(
            clash_sim::device::device_at(&town.sim, town.at(oven), Layer::Fixture)
                .map(|oven| town.sim.truth().entities().contents(oven).to_vec())
                .unwrap_or_default(),
        )
        .filter(|id| {
            town.sim
                .truth()
                .entities()
                .get(*id)
                .is_some_and(|e| e.tags.contains("food"))
        })
        .collect();
    assert!(!loaves.is_empty(), "no loaf was baked");
}

// ---- Saving and replay -------------------------------------------------------------

#[test]
fn a_furnished_town_runs_the_same_through_a_save() {
    let (mut once, _) = at_the_bakery(DEFAULT_SEED);
    let (mut split, _) = at_the_bakery(DEFAULT_SEED);
    let rack = mark(DEFAULT_SEED, "bakery", "bread");
    once.run_for(5 * SECOND);
    split.run_for(5 * SECOND);
    let blob = split.sim.save().unwrap();
    let mut sim = Sim::load(
        Arc::clone(split.sim.defs()),
        split.sim.truth().cosmos().clone(),
        &blob,
    )
    .unwrap();
    assert!(sim.reindex().is_empty());
    assert_eq!(sim.hashes(), split.sim.hashes());
    let mut registry = clash_gen::registry_with(&PROFILE.2);
    registry.set_runner(Arc::new(SerialRunner));
    let sim = sim.with_io(registry.bind(&PROFILE.1), Box::new(MemoryStore::new()));
    let mut loaded = Town { sim, ..split };
    once.open(rack);
    loaded.open(rack);
    once.run_for(5 * SECOND);
    loaded.run_for(5 * SECOND);
    assert_eq!(once.sim.hashes(), loaded.sim.hashes());
}

// ---- Bad data ----------------------------------------------------------------------

#[test]
fn a_map_that_names_items_nothing_defines_fails_to_load_at_the_cell_spec() {
    let types = clash_scene::engine_types();
    let maps = r#"[
      { "type": "map", "id": "pantry_shelf", "size": [2, 1],
        "palette": [ { "a": { "fixture": "table", "items": "no_such_loaf" },
                       "b": { "fixture": "table", "items": { "table": "no_such_table", "count": 2 } } } ],
        "levels": { "0": [ "ab" ] } } ]"#;
    let outcome = clash_scene::engine_loader(&types)
        .search_path(mods_dir())
        .memory_mod(
            "t",
            r#"{ "id": "t", "requires": ["core"] }"#,
            &[("data/maps.json", maps)],
        )
        .load_mods(&["core", "t"]);
    let text = outcome.diagnostics.render();
    assert!(outcome.defs.is_none(), "{text}");
    assert!(text.contains("there is no item `no_such_loaf`"), "{text}");
    assert!(
        text.contains("there is no content table `no_such_table`"),
        "{text}"
    );
    assert!(text.contains("data/maps.json:3"), "{text}");
    assert!(text.contains("palette[0].a.items"), "{text}");
}
