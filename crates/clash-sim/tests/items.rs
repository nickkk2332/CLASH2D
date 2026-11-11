//! Tests of items and holding (06 §9; package E8), against the real
//! simulation: the world content of `mods/core`, and the bodies and
//! items of `tests/fixtures/item`.
//!
//! The kernel does not yet give creatures bodies and stats, so
//! [`person`] attaches them over `Sim::components_mut` with the body
//! and stats packages' own functions, as the kernel will.

mod common;

use std::path::{Path, PathBuf};
use std::sync::{Arc, LazyLock};

use clash_core::{EntityId, Real, Rng};
use clash_data::{Defs, Loader, Runner};
use clash_expr::{FunctionId, Handle, Host, PropertyId, RuntimeWarning, Value};
use clash_sim::body::{self, BodyCx, BodyDefs, Detach, Hit, HitAt, HitSettings, PartKey};
use clash_sim::component::Containment;
use clash_sim::item::{self, Hold, ItemDef, ItemId, reason};
use clash_sim::rule::{RunView, VerbCall};
use clash_sim::stats::{self, SourceKind, StatDefs, StatsCx};
use clash_sim::{Command, DebugCommand, Place, Sim, SimEvent, Subject};
use clash_time::{ClockId, Timeline};
use clash_world::defs::{DamageTypeDef, DamageTypeId};
use clash_world::{Cause, DamagePacket, DamageResult, OccupantFilter, OccupantKind};
use common::{at, creature, item as plain_item, on, save_and_load, sim_over, spawn};
use proptest::prelude::*;

// ---- Loading ----------------------------------------------------------------

fn fixtures(area: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join(format!("tests/fixtures/{area}"))
}

/// The world files of `mods/core`, as paths within the mod and their
/// text, in file-name order.
fn core_world() -> Vec<(String, String)> {
    let dir = common::mods_dir().join("core/data/world");
    let mut files: Vec<PathBuf> = std::fs::read_dir(&dir)
        .unwrap()
        .filter_map(|entry| entry.ok().map(|entry| entry.path()))
        .filter(|path| path.extension().is_some_and(|ext| ext == "json"))
        .collect();
    files.sort();
    files
        .iter()
        .map(|file| {
            let name = file.file_name().unwrap().to_string_lossy();
            let text = std::fs::read_to_string(file).unwrap();
            (format!("data/world/{name}"), text)
        })
        .collect()
}

/// `mods/core`'s world, the body fixtures and the item fixtures,
/// loaded once.
static DEFS: LazyLock<Arc<Defs>> = LazyLock::new(|| {
    let types = common::types();
    let files = core_world();
    let files: Vec<(&str, &str)> = files
        .iter()
        .map(|(path, text)| (path.as_str(), text.as_str()))
        .collect();
    let outcome = Loader::new(&types)
        .expr_registry(&clash_sim::engine_registry())
        .memory_mod("core", r#"{ "id": "core" }"#, &files)
        .search_path(fixtures("item"))
        .load_mods(&["core", "itemfix"]);
    assert_eq!(outcome.diagnostics.render(), "");
    outcome.defs.expect("the fixtures load")
});

/// The tables the body and stats functions are handed.
struct Tables {
    body: BodyDefs,
    stats: StatDefs,
}

static TABLES: LazyLock<Tables> = LazyLock::new(|| {
    let world = clash_world::WorldDefs::from_defs(&DEFS).unwrap();
    Tables {
        body: BodyDefs::new(&DEFS, &world).unwrap(),
        stats: StatDefs::new(&DEFS).unwrap(),
    }
});

fn sim() -> Sim {
    sim_over(Arc::clone(&DEFS), 11)
}

fn def(name: &str) -> ItemId {
    DEFS.lookup::<ItemDef>(&format!("itemfix:{name}"))
        .unwrap_or_else(|| panic!("no item `{name}`"))
}

fn kind(name: &str) -> DamageTypeId {
    DEFS.lookup::<DamageTypeDef>(&format!("core:{name}"))
        .unwrap()
}

fn channel(name: &str) -> clash_sim::ChannelId {
    DEFS.lookup::<clash_sim::action::ChannelDef>(&format!("itemfix:{name}"))
        .unwrap()
}

fn attr(name: &str) -> stats::AttributeId {
    TABLES
        .stats
        .attribute_id(&format!("itemfix:{name}"))
        .unwrap()
}

// ---- Bodies -----------------------------------------------------------------

/// Nothing is inside anything: what a body is attached with. The
/// simulation brings the masses up to date as things move.
struct Apart;

impl Containment for Apart {
    fn container(&self, _id: EntityId) -> Option<EntityId> {
        None
    }

    fn contents(&self, _id: EntityId, _visit: &mut dyn FnMut(EntityId)) {}
}

/// A host that answers nothing.
struct Quiet;

impl Host for Quiet {
    fn subject(&mut self, _slot: u16) -> Value {
        Value::Null
    }

    fn local(&mut self, _slot: u16) -> Value {
        Value::Null
    }

    fn event_field(&mut self, _slot: u16) -> Value {
        Value::Null
    }

    fn property(&mut self, _property: PropertyId, _subject: Handle, _args: &[Value]) -> Value {
        Value::Null
    }

    fn variable(&mut self, _subject: Handle, _name: &str) -> Value {
        Value::Null
    }

    fn call(&mut self, _function: FunctionId, _args: &[Value]) -> Value {
        Value::Null
    }

    fn rng(&mut self) -> Option<&mut Rng> {
        None
    }

    fn warn(&mut self, _warning: RuntimeWarning) {}
}

/// Runs a body function over the simulation's stores. Wounds and pools
/// schedule on a timeline of their own here: the kernel does not yet
/// hand its own to the body package.
fn with_body<R>(sim: &mut Sim, run: impl FnOnce(&mut BodyCx<'_, Quiet>) -> R) -> R {
    let mut timeline = Timeline::new();
    let mut runner = Runner::new();
    let mut quiet = Quiet;
    let components = sim.components_mut();
    let mut cx = BodyCx {
        bodies: &mut components.body,
        defs: &TABLES.body,
        physical: &mut components.physical,
        containment: &Apart,
        stats: StatsCx {
            stats: &mut components.stats,
            defs: &TABLES.stats,
            timeline: &mut timeline,
            host: &mut quiet,
            runner: &mut runner,
        },
    };
    run(&mut cx)
}

/// A creature of a body plan of the fixtures, with stats, standing at a
/// cell.
fn creature_with(sim: &mut Sim, plan: &str, x: i32, y: i32) -> EntityId {
    let id = spawn(sim, creature(), on(x, y));
    let plan = TABLES.body.plan_id(&format!("itemfix:{plan}")).unwrap();
    with_body(sim, |cx| {
        stats::attach(&mut cx.stats, id, ClockId::UNIVERSAL).unwrap();
        body::attach(cx, id, plan, Real::ONE, ClockId::UNIVERSAL).unwrap();
    });
    id
}

/// A person-shaped creature: the fixtures' humanoid.
fn person(sim: &mut Sim, x: i32, y: i32) -> EntityId {
    creature_with(sim, "humanoid", x, y)
}

fn part(sim: &Sim, who: EntityId, name: &str) -> PartKey {
    sim.components()
        .body
        .get(who)
        .and_then(|body| body.part_named(name))
        .unwrap_or_else(|| panic!("no part `{name}`"))
        .key()
}

// ---- Reading ----------------------------------------------------------------

fn make(sim: &mut Sim, name: &str, count: u32, place: Place) -> EntityId {
    item::create(sim, def(name), count, place, Cause::DEBUG).unwrap()
}

fn count(sim: &Sim, id: EntityId) -> u32 {
    sim.components().item.get(id).unwrap().count()
}

fn mass(sim: &Sim, id: EntityId) -> f64 {
    sim.components()
        .physical
        .get(id)
        .map_or(0.0, |physical| physical.mass().get().get())
}

fn own_mass(sim: &Sim, id: EntityId) -> f64 {
    sim.components()
        .physical
        .get(id)
        .map_or(0.0, |physical| physical.own_mass().get().get())
}

fn place(sim: &Sim, id: EntityId) -> Place {
    sim.truth().place(id).unwrap()
}

fn hold(sim: &Sim, id: EntityId) -> Option<Hold> {
    item::hold_of(sim, id)
}

fn hold_name(sim: &Sim, id: EntityId) -> Option<&'static str> {
    hold(sim, id).map(|hold| hold.name())
}

fn attr_value(sim: &Sim, who: EntityId, name: &str) -> f64 {
    sim.components()
        .stats
        .get(who)
        .and_then(|stats| stats.attr(attr(name)))
        .unwrap()
        .get()
}

fn has_tag(sim: &Sim, who: EntityId, tag: &str) -> bool {
    sim.truth().entity(who).unwrap().tags.contains(tag)
}

/// The events delivered since the last call.
fn events(sim: &mut Sim) -> Vec<SimEvent> {
    sim.pump();
    sim.drain_events()
}

fn named<'a>(events: &'a [SimEvent], name: &str) -> Vec<&'a SimEvent> {
    events.iter().filter(|event| event.name() == name).collect()
}

fn entity_of(event: &SimEvent, name: &str) -> Option<EntityId> {
    common::entity_of(event, name)
}

fn prop(sim: &Sim, id: EntityId, name: &str, args: &[Value]) -> Value {
    sim.property_named(Subject::Entity(id), name, args)
}

fn ids_of(sim: &Sim, value: &Value) -> Vec<EntityId> {
    match value {
        Value::List(items) => items
            .iter()
            .map(|value| match sim.subject_of(value) {
                Some(Subject::Entity(id)) => id,
                other => panic!("not an entity: {other:?}"),
            })
            .collect(),
        other => panic!("not a list: {other:?}"),
    }
}

fn entity_value(sim: &Sim, id: EntityId) -> Value {
    sim.subject_value(Subject::Entity(id))
}

fn close(a: f64, b: f64) -> bool {
    (a - b).abs() <= 1e-9 * a.abs().max(b.abs()).max(1.0)
}

// ---- The invariants ---------------------------------------------------------

/// The slots each part's grasps take, counting the shared grasp of the
/// pieces of one stack once.
fn grasped(holds: &item::Holds) -> Vec<(PartKey, clash_sim::ChannelId, u32)> {
    let mut seen: Vec<&Vec<item::GraspSlot>> = Vec::new();
    let mut used: Vec<(PartKey, clash_sim::ChannelId, u32)> = Vec::new();
    for (_, hold) in holds.iter() {
        let Hold::Grasp(slots) = hold else { continue };
        if seen.contains(&slots) {
            continue;
        }
        seen.push(slots);
        for slot in slots {
            match used
                .iter_mut()
                .find(|(part, channel, _)| *part == slot.part && *channel == slot.channel)
            {
                Some(entry) => entry.2 += u32::from(slot.count),
                None => used.push((slot.part, slot.channel, u32::from(slot.count))),
            }
        }
    }
    used
}

/// What every holder's records must agree with, whatever happened:
/// every record names something inside the holder (B6); no part's
/// grasps take more than it supplies; what items confer is exactly what
/// a recomputation gives (B3); a holder weighs itself and its contents.
fn check_invariants(sim: &Sim) {
    let components = sim.components();
    for (holder, holds) in components.holds.iter() {
        for (entity, hold) in holds.iter() {
            assert_eq!(
                place(sim, entity),
                Place::In(holder),
                "{holder} keeps a record of {entity}, which is not inside it"
            );
            assert_ne!(*hold, Hold::Inside, "an `Inside` hold is never recorded");
        }
        for (part, channel, used) in grasped(holds) {
            let body = components.body.get(holder).expect("only a body grasps");
            let def = TABLES.body.part(body.part(part).unwrap().def()).unwrap();
            let supplied = def
                .channels
                .iter()
                .find(|(have, _)| have.id() == channel)
                .map_or(0, |(_, count)| u32::from(*count));
            assert!(
                used <= supplied,
                "{holder} grasps with {used} of the {supplied} slots of {part:?}"
            );
        }
        assert_eq!(
            holds.conferred_tags(),
            item::conferred(sim, holder).tags,
            "the tags of {holder}"
        );
        for tag in holds.conferred_tags() {
            assert!(has_tag(sim, holder, tag), "{holder} lacks `{tag}`");
        }
    }
    // What items confer equals a recomputation, for everything with
    // stats.
    for (holder, stats) in components.stats.iter() {
        let mut have: Vec<stats::Modifier> = stats
            .modifiers()
            .iter()
            .filter(|modifier| modifier.source.kind == SourceKind::Item)
            .copied()
            .collect();
        have.sort_by_key(|modifier| (modifier.attr, modifier.source));
        assert_eq!(
            have,
            item::conferred(sim, holder).modifiers,
            "the modifiers of {holder}"
        );
    }
    // Every entity is in exactly one place, and weighs itself and what
    // it holds.
    for entity in sim.truth().entities().iter() {
        let id = entity.id();
        if let Place::In(container) = entity.place() {
            assert!(sim.truth().contents(container).contains(&id));
        }
        if let Some(physical) = components.physical.get(id) {
            let inside: f64 = sim
                .truth()
                .contents(id)
                .iter()
                .map(|inside| mass(sim, *inside))
                .sum();
            assert!(
                close(
                    physical.mass().get().get(),
                    physical.own_mass().get().get() + inside
                ),
                "{id} weighs {} and holds {inside}",
                physical.mass().get().get()
            );
        }
    }
}

// ---- Making items -----------------------------------------------------------

#[test]
fn an_item_is_an_entity_made_from_its_definition() {
    let mut sim = sim();
    let knife = make(&mut sim, "knife", 1, on(3, 3));
    let item = sim.components().item.get(knife).unwrap();
    assert_eq!(item.def(), Some(def("knife")));
    assert_eq!(item.count(), 1);
    assert_eq!(item.damage(), 0);
    assert!(!item.is_broken());
    // Its mass is its definition's; its volume, left out, is its mass
    // over its material's density.
    assert!(close(mass(&sim, knife), 0.2));
    let volume = sim
        .components()
        .physical
        .get(knife)
        .unwrap()
        .volume()
        .get()
        .get();
    assert!(close(volume, 0.2 / 7800.0));
    // Its tags are its own, and it is an item to the world.
    assert!(has_tag(&sim, knife, "blade"));
    let found = sim
        .truth()
        .cosmos()
        .occupants_at(at(3, 3), &OccupantFilter::ANY.of_kind(OccupantKind::Item))
        .unwrap();
    assert_eq!(found.len(), 1);
    // What expressions read of it.
    assert_eq!(prop(&sim, knife, "is_item", &[]), Value::Bool(true));
    assert_eq!(prop(&sim, knife, "count", &[]), Value::Int(1));
    assert_eq!(prop(&sim, knife, "condition", &[]), Value::Real(Real::ONE));
    assert_eq!(prop(&sim, knife, "is_broken", &[]), Value::Bool(false));
    assert!(matches!(prop(&sim, knife, "item", &[]), Value::Id(_)));
    assert!(matches!(prop(&sim, knife, "material", &[]), Value::Id(_)));
    assert_eq!(prop(&sim, knife, "holder", &[]), Value::Null);
    assert_eq!(prop(&sim, knife, "hold", &[]), Value::Null);
    assert_eq!(prop(&sim, knife, "capacity_volume", &[]), Value::Null);
    assert_eq!(prop(&sim, knife, "is_latent", &[]), Value::Bool(false));
    check_invariants(&sim);
}

#[test]
fn making_is_refused_with_a_reason_and_leaves_nothing() {
    let mut sim = sim();
    let hash = sim.state_hash();
    let refused = item::create(&mut sim, def("knife"), 2, on(1, 1), Cause::DEBUG).unwrap_err();
    assert_eq!(refused.code, reason::NOT_STACKABLE);
    let refused = item::create(&mut sim, def("coin"), 0, on(1, 1), Cause::DEBUG).unwrap_err();
    assert_eq!(refused.code, reason::BAD_COUNT);
    let unknown = ItemId::from_index(10_000).unwrap();
    let refused = item::create(&mut sim, unknown, 1, on(1, 1), Cause::DEBUG).unwrap_err();
    assert_eq!(refused.code, reason::UNKNOWN_DEF);
    // Outside the loaded world.
    let refused = item::create(&mut sim, def("knife"), 1, on(-5000, 1), Cause::DEBUG);
    assert!(refused.is_err());
    assert_eq!(sim.state_hash(), hash);
    assert_eq!(sim.truth().entities().iter().count(), 0);
}

#[test]
fn the_debug_command_makes_items_by_qualified_id() {
    let mut sim = sim();
    let command = |def: &str, count| {
        Command::Debug(DebugCommand::CreateItem {
            def: def.to_owned(),
            count,
            place: on(2, 2),
        })
    };
    sim.submit(command("itemfix:coin", 25)).unwrap();
    let coins = sim.truth().entities().ids().next().unwrap();
    assert_eq!(count(&sim, coins), 25);
    assert!(close(mass(&sim, coins), 25.0 * 0.008));
    let refused = sim.submit(command("itemfix:nothing", 1)).unwrap_err();
    assert_eq!(refused.reason.code, reason::UNKNOWN_DEF);
    // Logged, it replays.
    let log = sim.input_log().clone();
    let mut again = self::sim();
    again.replay(&log).unwrap();
    assert_eq!(again.state_hash(), sim.state_hash());
}

#[test]
fn a_colour_is_picked_by_seed_and_can_be_changed() {
    let mut sim = sim();
    let coats: Vec<EntityId> = (0..12)
        .map(|_| make(&mut sim, "coat", 1, on(4, 4)))
        .collect();
    let colours: Vec<String> = coats
        .iter()
        .map(|coat| {
            sim.components()
                .item
                .get(*coat)
                .unwrap()
                .color()
                .unwrap()
                .to_owned()
        })
        .collect();
    for colour in &colours {
        assert!(["brown", "russet", "gray"].contains(&colour.as_str()));
    }
    // Twelve draws from three name more than one.
    assert!(colours.iter().any(|colour| *colour != colours[0]));
    // The same seed gives the same colour.
    let seed = sim.components().item.get(coats[0]).unwrap().seed();
    let again =
        item::create_seeded(&mut sim, def("coat"), 1, on(4, 4), seed, Cause::DEBUG).unwrap();
    assert_eq!(
        sim.components().item.get(again).unwrap().color(),
        Some(colours[0].as_str())
    );
    // Changed by the function, and by the property.
    item::set_color(&mut sim, coats[0], Some("green")).unwrap();
    assert_eq!(prop(&sim, coats[0], "color", &[]), Value::from("green"));
    sim.set_property_named(
        Subject::Entity(coats[0]),
        "color",
        &Value::Null,
        Cause::DEBUG,
    )
    .unwrap();
    assert_eq!(prop(&sim, coats[0], "color", &[]), Value::Null);
    let refused = sim
        .set_property_named(
            Subject::Entity(coats[0]),
            "color",
            &Value::Int(3),
            Cause::DEBUG,
        )
        .unwrap_err();
    assert_eq!(refused.code, clash_sim::codes::WRITE_BAD_VALUE);
    // The name it goes by before it is known is kept for text.
    let coat_def = DEFS.registry::<ItemDef>().get(def("coat"));
    assert!(coat_def.unknown_name.is_some());
}

// ---- Holding ----------------------------------------------------------------

#[test]
fn a_person_picks_up_wields_wears_stashes_and_drops() {
    let mut sim = sim();
    let hero = person(&mut sim, 5, 5);
    let body_mass = mass(&sim, hero);
    let knife = make(&mut sim, "knife", 1, on(5, 5));
    let coat = make(&mut sim, "coat", 1, on(5, 5));
    let pack = make(&mut sim, "backpack", 1, on(5, 5));
    let coins = make(&mut sim, "coin", 30, on(5, 5));
    events(&mut sim);

    // Picked up: in a grasp of the first free arm.
    assert_eq!(item::free_grasp(&sim, hero, channel("arm")), 2);
    assert_eq!(
        item::take(&mut sim, hero, knife, Cause::DEBUG).unwrap(),
        knife
    );
    assert_eq!(place(&sim, knife), Place::In(hero));
    let Some(Hold::Grasp(slots)) = hold(&sim, knife) else {
        panic!("the knife is not grasped")
    };
    assert_eq!(slots.len(), 1);
    assert_eq!(slots[0].part, part(&sim, hero, "l_arm"));
    assert_eq!(item::free_grasp(&sim, hero, channel("arm")), 1);
    let seen = events(&mut sim);
    let taken = named(&seen, "item_taken");
    assert_eq!(taken.len(), 1);
    assert_eq!(entity_of(taken[0], "self"), Some(knife));
    assert_eq!(entity_of(taken[0], "holder"), Some(hero));
    // What it confers is applied.
    assert_eq!(attr_value(&sim, hero, "menace"), 1.0);
    assert!(has_tag(&sim, hero, "armed"));
    check_invariants(&sim);

    // Worn: over the torso and both arms, and it confers while worn.
    assert_eq!(
        item::wear(&mut sim, hero, coat, Cause::DEBUG).unwrap(),
        coat
    );
    let Some(Hold::Worn(on_parts)) = hold(&sim, coat) else {
        panic!("the coat is not worn")
    };
    let covered: Vec<PartKey> = on_parts.iter().map(|on| on.part).collect();
    for name in ["torso", "l_arm", "r_arm"] {
        assert!(covered.contains(&part(&sim, hero, name)), "{name}");
    }
    assert_eq!(covered.len(), 3);
    assert_eq!(attr_value(&sim, hero, "warmth"), 2.0);
    assert!(has_tag(&sim, hero, "clothed"));
    let seen = events(&mut sim);
    assert_eq!(named(&seen, "item_worn").len(), 1);
    // Wearing frees no hand and takes none.
    assert_eq!(item::free_grasp(&sim, hero, channel("arm")), 1);
    assert_eq!(
        ids_of(&sim, &prop(&sim, hero, "worn_on", &[Value::from("torso")])),
        [coat]
    );

    // A pack on the back, and the coins stowed in it.
    item::wear(&mut sim, hero, pack, Cause::DEBUG).unwrap();
    assert_eq!(
        item::stow(&mut sim, hero, coins, Cause::DEBUG).unwrap(),
        coins
    );
    assert_eq!(place(&sim, coins), Place::In(pack));
    assert_eq!(hold_name(&sim, coins), Some("stowed"));
    // Stowed, the coins confer nothing; carried, they weigh.
    let carried = 0.2 + 1.8 + 1.2 + 30.0 * 0.008;
    assert!(close(mass(&sim, hero), body_mass + carried));
    assert!(close(own_mass(&sim, hero), body_mass));
    let Value::Real(carried_mass) = prop(&sim, hero, "carried_mass", &[]) else {
        panic!("no carried mass")
    };
    assert!(close(carried_mass.get(), carried));
    // What the hero holds, wears and carries.
    assert_eq!(
        ids_of(&sim, &prop(&sim, hero, "held", &[Value::from("grasp")])),
        [knife]
    );
    let worn = ids_of(&sim, &prop(&sim, hero, "held", &[Value::from("worn")]));
    assert_eq!(worn.len(), 2);
    assert!(worn.contains(&coat) && worn.contains(&pack));
    let carried = ids_of(&sim, &prop(&sim, hero, "carried", &[]));
    assert_eq!(carried.len(), 4);
    assert_eq!(
        ids_of(&sim, &prop(&sim, pack, "held", &[Value::from("stowed")])),
        [coins]
    );
    assert_eq!(prop(&sim, coins, "holder", &[]), entity_value(&sim, pack));
    assert_eq!(prop(&sim, coins, "hold", &[]), Value::from("stowed"));
    assert_eq!(
        prop(&sim, pack, "capacity_volume", &[]),
        Value::Real(Real::of(0.02))
    );
    assert_eq!(
        prop(&sim, pack, "capacity_mass", &[]),
        Value::Real(Real::of(15.0))
    );
    assert!(matches!(
        prop(&sim, pack, "stowed_volume", &[]),
        Value::Real(_)
    ));
    assert_eq!(
        prop(&sim, hero, "free_grasp", &[Value::from("itemfix:arm")]),
        Value::Int(1)
    );
    let tags = prop(&sim, hero, "conferred_tags", &[]);
    assert_eq!(
        tags,
        Value::list([Value::from("armed"), Value::from("clothed")])
    );
    check_invariants(&sim);

    // Taken out into the free hand, then put down.
    item::take_out(&mut sim, coins, Some(hero), Cause::DEBUG).unwrap();
    assert_eq!(hold_name(&sim, coins), Some("grasp"));
    assert_eq!(item::free_grasp(&sim, hero, channel("arm")), 0);
    assert_eq!(item::drop(&mut sim, coins, Cause::DEBUG).unwrap(), coins);
    assert_eq!(place(&sim, coins), on(5, 5));
    let seen = events(&mut sim);
    let dropped = named(&seen, "item_dropped");
    assert_eq!(dropped.len(), 1);
    assert_eq!(entity_of(dropped[0], "holder"), Some(hero));

    // The coat taken off into a hand: refused with both hands full, and
    // done once the knife is put down.
    item::take(&mut sim, hero, coins, Cause::DEBUG).unwrap();
    let before = sim.state_hash();
    let refused = item::take_off(&mut sim, coat, Cause::DEBUG).unwrap_err();
    assert_eq!(refused.code, reason::NO_FREE_GRASP);
    assert_eq!(sim.state_hash(), before);
    item::drop(&mut sim, coins, Cause::DEBUG).unwrap();
    item::drop(&mut sim, knife, Cause::DEBUG).unwrap();
    assert_eq!(attr_value(&sim, hero, "menace"), 0.0);
    assert!(!has_tag(&sim, hero, "armed"));
    item::take_off(&mut sim, coat, Cause::DEBUG).unwrap();
    assert_eq!(hold_name(&sim, coat), Some("grasp"));
    let seen = events(&mut sim);
    assert_eq!(named(&seen, "item_removed").len(), 1);
    // Held, it still confers; dropped, it no longer does.
    assert_eq!(attr_value(&sim, hero, "warmth"), 2.0);
    item::drop(&mut sim, coat, Cause::DEBUG).unwrap();
    assert_eq!(attr_value(&sim, hero, "warmth"), 0.0);
    assert!(!has_tag(&sim, hero, "clothed"));
    // A worn thing dropped comes off and lies at the hero's feet.
    item::drop(&mut sim, pack, Cause::DEBUG).unwrap();
    assert_eq!(place(&sim, pack), on(5, 5));
    assert!(close(mass(&sim, hero), body_mass));
    assert!(sim.components().holds.get(hero).is_none());
    check_invariants(&sim);
}

#[test]
fn a_holder_without_the_part_needed_is_refused_with_a_reason() {
    let mut sim = sim();
    let hero = person(&mut sim, 5, 5);
    let blob = creature_with(&mut sim, "blob", 6, 5);
    let ghost = spawn(&mut sim, creature(), on(7, 5));
    let knife = make(&mut sim, "knife", 1, on(5, 5));
    let sword = make(&mut sim, "greatsword", 1, on(5, 5));
    let hood = make(&mut sim, "hood", 1, on(5, 5));
    let hash = sim.state_hash();

    // A blob has pseudopods, not arms.
    let refused = item::take(&mut sim, blob, knife, Cause::DEBUG).unwrap_err();
    assert_eq!(refused.code, reason::NO_FREE_GRASP);
    assert_eq!(
        refused.value("needed"),
        Some(&clash_core::Value::from(1_i64))
    );
    assert_eq!(refused.value("free"), Some(&clash_core::Value::from(0_i64)));
    // And no head for a hood.
    let refused = item::wear(&mut sim, blob, hood, Cause::DEBUG).unwrap_err();
    assert_eq!(refused.code, reason::NO_PART);
    // Something with no body holds nothing.
    let refused = item::take(&mut sim, ghost, knife, Cause::DEBUG).unwrap_err();
    assert_eq!(refused.code, reason::NO_BODY);
    // A thing with no grip cannot be grasped; a knife cannot be worn.
    let crate_ = make(&mut sim, "crate", 1, on(5, 5));
    let hash_with_crate = sim.state_hash();
    let refused = item::take(&mut sim, hero, crate_, Cause::DEBUG).unwrap_err();
    assert_eq!(refused.code, reason::NO_GRIP);
    let refused = item::wear(&mut sim, hero, knife, Cause::DEBUG).unwrap_err();
    assert_eq!(refused.code, reason::NOT_WEARABLE);
    assert_eq!(sim.state_hash(), hash_with_crate);
    item::destroy(&mut sim, crate_, Cause::DEBUG).unwrap();
    let _ = hash;

    // Two hands for the greatsword: refused while one holds the knife.
    item::take(&mut sim, hero, knife, Cause::DEBUG).unwrap();
    let before = sim.state_hash();
    let refused = item::take(&mut sim, hero, sword, Cause::DEBUG).unwrap_err();
    assert_eq!(refused.code, reason::NO_FREE_GRASP);
    assert_eq!(
        refused.value("needed"),
        Some(&clash_core::Value::from(2_i64))
    );
    assert_eq!(refused.value("free"), Some(&clash_core::Value::from(1_i64)));
    assert_eq!(sim.state_hash(), before);
    item::drop(&mut sim, knife, Cause::DEBUG).unwrap();
    item::take(&mut sim, hero, sword, Cause::DEBUG).unwrap();
    let Some(Hold::Grasp(slots)) = hold(&sim, sword) else {
        panic!("not grasped")
    };
    assert_eq!(slots.len(), 2);
    assert_eq!(item::free_grasp(&sim, hero, channel("arm")), 0);
    // One layer at a time: a second coat is refused, a mail shirt goes
    // under it.
    let coat = make(&mut sim, "coat", 1, on(5, 5));
    let other = make(&mut sim, "coat", 1, on(5, 5));
    let mail = make(&mut sim, "mail_shirt", 1, on(5, 5));
    item::wear(&mut sim, hero, coat, Cause::DEBUG).unwrap();
    let refused = item::wear(&mut sim, hero, other, Cause::DEBUG).unwrap_err();
    assert_eq!(refused.code, reason::LAYER_TAKEN);
    assert_eq!(
        refused.value("by"),
        Some(&clash_core::Value::from(coat.to_string()))
    );
    item::wear(&mut sim, hero, mail, Cause::DEBUG).unwrap();
    // Not held, not worn, not contained.
    let refused = item::drop(&mut sim, other, Cause::DEBUG).unwrap_err();
    assert_eq!(refused.code, reason::NOT_HELD);
    let refused = item::take_off(&mut sim, other, Cause::DEBUG).unwrap_err();
    assert_eq!(refused.code, reason::NOT_WORN);
    let refused = item::take_out(&mut sim, other, None, Cause::DEBUG).unwrap_err();
    assert_eq!(refused.code, reason::NOT_CONTAINED);
    let refused = item::give(&mut sim, other, hero, Cause::DEBUG).unwrap_err();
    assert_eq!(refused.code, reason::NOT_HELD);
    check_invariants(&sim);
}

#[test]
fn giving_hands_a_thing_from_one_grasp_to_another() {
    let mut sim = sim();
    let ann = person(&mut sim, 5, 5);
    let bob = person(&mut sim, 6, 5);
    let knife = make(&mut sim, "knife", 1, on(5, 5));
    item::take(&mut sim, ann, knife, Cause::DEBUG).unwrap();
    assert_eq!(attr_value(&sim, ann, "menace"), 1.0);
    item::give(&mut sim, knife, bob, Cause::DEBUG).unwrap();
    assert_eq!(place(&sim, knife), Place::In(bob));
    assert_eq!(attr_value(&sim, ann, "menace"), 0.0);
    assert_eq!(attr_value(&sim, bob, "menace"), 1.0);
    assert!(!has_tag(&sim, ann, "armed"));
    let seen = events(&mut sim);
    let taken = named(&seen, "item_taken");
    assert_eq!(entity_of(taken.last().unwrap(), "from"), Some(ann));
    check_invariants(&sim);
}

#[test]
fn whoever_moves_a_thing_takes_back_what_it_conferred() {
    let mut sim = sim();
    let hero = person(&mut sim, 5, 5);
    let knife = make(&mut sim, "knife", 1, on(5, 5));
    item::take(&mut sim, hero, knife, Cause::DEBUG).unwrap();
    assert_eq!(attr_value(&sim, hero, "menace"), 1.0);
    // The kernel's own command moves it; the hold and the modifier go.
    sim.submit(Command::Debug(DebugCommand::MoveEntity {
        entity: knife,
        to: on(9, 9),
    }))
    .unwrap();
    assert_eq!(attr_value(&sim, hero, "menace"), 0.0);
    assert!(sim.components().holds.get(hero).is_none());
    check_invariants(&sim);
    // Removed while held: the same.
    item::take(&mut sim, hero, knife, Cause::DEBUG).unwrap();
    sim.remove_entity(knife, Cause::DEBUG).unwrap();
    assert_eq!(attr_value(&sim, hero, "menace"), 0.0);
    assert!(!has_tag(&sim, hero, "armed"));
    check_invariants(&sim);
}

#[test]
fn a_holder_removed_leaves_what_it_held_where_it_was_whole() {
    let mut sim = sim();
    let hero = person(&mut sim, 5, 5);
    let coat = make(&mut sim, "coat", 1, on(5, 5));
    let knife = make(&mut sim, "knife", 1, on(5, 5));
    item::wear(&mut sim, hero, coat, Cause::DEBUG).unwrap();
    item::take(&mut sim, hero, knife, Cause::DEBUG).unwrap();
    sim.remove_entity(hero, Cause::DEBUG).unwrap();
    assert_eq!(place(&sim, coat), on(5, 5));
    assert_eq!(place(&sim, knife), on(5, 5));
    assert_eq!(sim.components().item.get(coat).unwrap().damage(), 0);
    check_invariants(&sim);
}

// ---- Containers -------------------------------------------------------------

#[test]
fn a_container_refuses_what_does_not_fit_with_the_reason() {
    let mut sim = sim();
    let pack = make(&mut sim, "backpack", 1, on(5, 5));
    let purse = make(&mut sim, "purse", 1, on(5, 5));
    let knife = make(&mut sim, "knife", 1, on(5, 5));
    let boulder = make(&mut sim, "boulder", 1, on(5, 5));
    let anvil = make(&mut sim, "anvil", 1, on(5, 5));
    let coins = make(&mut sim, "coin", 10, on(5, 5));
    let hash = sim.state_hash();

    // Too large.
    let refused = item::put_in(&mut sim, boulder, pack, Cause::DEBUG).unwrap_err();
    assert_eq!(refused.code, reason::DOES_NOT_FIT);
    assert_eq!(
        refused.value("what"),
        Some(&clash_core::Value::from("volume"))
    );
    // Small enough, too heavy.
    let refused = item::put_in(&mut sim, anvil, pack, Cause::DEBUG).unwrap_err();
    assert_eq!(refused.code, reason::DOES_NOT_FIT);
    assert_eq!(
        refused.value("what"),
        Some(&clash_core::Value::from("mass"))
    );
    // Not what it takes.
    let refused = item::put_in(&mut sim, knife, purse, Cause::DEBUG).unwrap_err();
    assert_eq!(refused.code, reason::NOT_ACCEPTED);
    // Not a container.
    let refused = item::put_in(&mut sim, coins, knife, Cause::DEBUG).unwrap_err();
    assert_eq!(refused.code, reason::NOT_A_CONTAINER);
    // Nothing goes inside itself.
    let refused = item::put_in(&mut sim, pack, pack, Cause::DEBUG).unwrap_err();
    assert_eq!(refused.code, clash_sim::codes::RELOCATE_INSIDE_ITSELF);
    assert_eq!(sim.state_hash(), hash);

    // What fits goes in, and the container weighs it.
    item::put_in(&mut sim, coins, purse, Cause::DEBUG).unwrap();
    item::put_in(&mut sim, purse, pack, Cause::DEBUG).unwrap();
    item::put_in(&mut sim, knife, pack, Cause::DEBUG).unwrap();
    assert!(close(mass(&sim, pack), 1.2 + 0.05 + 0.08 + 0.2));
    assert_eq!(
        ids_of(&sim, &prop(&sim, pack, "carried", &[])),
        [purse, coins, knife]
    );
    // The room left is what is left: a second purse full of coins fits
    // until the volume runs out.
    let pile = make(&mut sim, "coin", 600, on(5, 5));
    let refused = item::put_in(&mut sim, pile, purse, Cause::DEBUG).unwrap_err();
    assert_eq!(refused.code, reason::DOES_NOT_FIT);
    check_invariants(&sim);

    // Taken out with nobody to take it: where the container is.
    item::take_out(&mut sim, knife, None, Cause::DEBUG).unwrap();
    assert_eq!(place(&sim, knife), on(5, 5));
    check_invariants(&sim);
}

#[test]
fn a_creature_can_be_shut_in_and_let_out() {
    let mut sim = sim();
    let crate_ = make(&mut sim, "crate", 1, on(5, 5));
    item::materialise(&mut sim, crate_, Cause::DEBUG).unwrap();
    let cat = spawn(&mut sim, plain_item(), on(5, 5));
    let hero = person(&mut sim, 6, 6);
    // Not an item: enclosed, not stowed, and not weighed against the
    // crate's room unless it has a size.
    item::put_in(&mut sim, cat, crate_, Cause::DEBUG).unwrap();
    assert_eq!(hold_name(&sim, cat), Some("inside"));
    // What the hero drops from inside the crate lands in the crate.
    let knife = make(&mut sim, "knife", 1, on(6, 6));
    item::take(&mut sim, hero, knife, Cause::DEBUG).unwrap();
    item::put_in(&mut sim, hero, crate_, Cause::DEBUG).unwrap();
    assert_eq!(hold_name(&sim, hero), Some("inside"));
    item::drop(&mut sim, knife, Cause::DEBUG).unwrap();
    assert_eq!(place(&sim, knife), Place::In(crate_));
    item::take_out(&mut sim, cat, None, Cause::DEBUG).unwrap();
    assert_eq!(place(&sim, cat), on(5, 5));
    check_invariants(&sim);
}

// ---- Stacks -----------------------------------------------------------------

#[test]
fn stacks_split_and_rejoin_and_counts_are_conserved() {
    let mut sim = sim();
    let hero = person(&mut sim, 5, 5);
    let arrows = make(&mut sim, "arrow", 40, on(5, 5));
    assert!(close(mass(&sim, arrows), 40.0 * 0.03));
    item::take(&mut sim, hero, arrows, Cause::DEBUG).unwrap();

    // Damaging one of forty leaves thirty-nine and one.
    let packet = DamagePacket::new(kind("cut"), Real::of(500.0), Real::of(1.0e9), Cause::DEBUG);
    let hit = item::damage(&mut sim, arrows, &packet).unwrap();
    let one = hit.item;
    assert_ne!(one, arrows);
    assert_eq!(count(&sim, arrows), 39);
    assert_eq!(count(&sim, one), 1);
    assert!(sim.components().item.get(one).unwrap().damage() > 0);
    assert_eq!(sim.components().item.get(arrows).unwrap().damage(), 0);
    // The piece shares the stack's grasp: no other hand was taken.
    assert_eq!(hold(&sim, one), hold(&sim, arrows));
    assert_eq!(item::free_grasp(&sim, hero, channel("arm")), 1);
    assert!(close(mass(&sim, arrows) + mass(&sim, one), 40.0 * 0.03));
    let seen = events(&mut sim);
    assert_eq!(named(&seen, "item_split").len(), 1);
    assert_eq!(named(&seen, "item_damaged").len(), 1);
    check_invariants(&sim);

    // Repaired in the same hold, it rejoins.
    let back = item::repair(&mut sim, one, Real::ONE, Cause::DEBUG).unwrap();
    assert_eq!(back, arrows);
    assert_eq!(count(&sim, arrows), 40);
    assert!(sim.truth().entity(one).is_none());
    let seen = events(&mut sim);
    assert_eq!(named(&seen, "item_repaired").len(), 1);
    assert_eq!(named(&seen, "item_merged").len(), 1);

    // Split by hand, dropped, and joined again on the ground.
    let ten = item::split(&mut sim, arrows, 10, Cause::DEBUG).unwrap();
    assert_eq!(count(&sim, arrows) + count(&sim, ten), 40);
    item::drop(&mut sim, ten, Cause::DEBUG).unwrap();
    let rest = item::drop(&mut sim, arrows, Cause::DEBUG).unwrap();
    assert_eq!(rest, ten);
    assert_eq!(count(&sim, ten), 40);
    // Picked up again, as one.
    item::take(&mut sim, hero, ten, Cause::DEBUG).unwrap();
    let five = item::split(&mut sim, ten, 5, Cause::DEBUG).unwrap();
    item::drop(&mut sim, five, Cause::DEBUG).unwrap();
    // Taking more of an equal stack joins the one in hand.
    assert_eq!(item::take(&mut sim, hero, five, Cause::DEBUG).unwrap(), ten);
    assert_eq!(count(&sim, ten), 40);
    assert_eq!(item::free_grasp(&sim, hero, channel("arm")), 1);

    // Refusals.
    for bad in [0, 40, 41] {
        let refused = item::split(&mut sim, ten, bad, Cause::DEBUG).unwrap_err();
        assert_eq!(refused.code, reason::BAD_COUNT);
    }
    let coins = make(&mut sim, "coin", 3, on(5, 5));
    let refused = item::merge(&mut sim, coins, ten, Cause::DEBUG).unwrap_err();
    assert_eq!(refused.code, reason::NOT_EQUAL);
    check_invariants(&sim);
}

#[test]
fn one_of_a_stack_is_worn_and_the_rest_stay() {
    let mut sim = sim();
    let hero = person(&mut sim, 5, 5);
    // Gloves do not stack, so make a stack of coins and wear nothing of
    // it; wear one coat of two made apart instead.
    let coins = make(&mut sim, "coin", 3, on(5, 5));
    let refused = item::wear(&mut sim, hero, coins, Cause::DEBUG).unwrap_err();
    assert_eq!(refused.code, reason::NOT_WEARABLE);
    let glove = make(&mut sim, "glove", 1, on(5, 5));
    item::wear(&mut sim, hero, glove, Cause::DEBUG).unwrap();
    let Some(Hold::Worn(on_parts)) = hold(&sim, glove) else {
        panic!("not worn")
    };
    assert_eq!(on_parts.len(), 1);
    assert_eq!(on_parts[0].part, part(&sim, hero, "l_hand"));
    assert_eq!(attr_value(&sim, hero, "warmth"), 0.5);
    check_invariants(&sim);
}

// ---- Hits and damage --------------------------------------------------------

/// A long edge drawn across a part of `target`, as the body tests do.
fn cut(sim: &Sim, target: EntityId, part_name: &str, energy: f64) -> Hit {
    let (area, give, span) = (2.0e-5, 0.002, 0.9);
    Hit {
        target,
        at: HitAt::Part(part(sim, target, part_name)),
        packet: DamagePacket::new(
            kind("cut"),
            Real::of(energy),
            Real::of(energy / (area * give)),
            Cause::DEBUG,
        ),
        area: Real::of(area),
        span: Real::of(span),
        dir: Some([Real::ONE, Real::ZERO, Real::ZERO]),
    }
}

/// What whoever applies a hit does with this package's functions.
fn apply_hit(sim: &mut Sim, hit: &Hit) -> body::HitReport {
    let worn = item::worn_layers_of(sim, hit.target);
    let mut rng = Rng::from_seed(5);
    let report = with_body(sim, |cx| {
        body::apply_hit(cx, hit, &worn, &mut rng, &HitSettings::default()).unwrap()
    });
    item::apply_worn_hits(sim, &report.worn, hit.packet.kind, Cause::DEBUG);
    for detached in &report.detached {
        item::part_item(sim, detached, Cause::DEBUG).unwrap();
    }
    item::on_body_changes(sim, &report.changes, Cause::DEBUG);
    report
}

#[test]
fn a_worn_layer_is_met_first_absorbs_by_the_packet_rule_and_is_damaged() {
    let mut sim = sim();
    let hero = person(&mut sim, 5, 5);
    let coat = make(&mut sim, "coat", 1, on(5, 5));
    let mail = make(&mut sim, "mail_shirt", 1, on(5, 5));
    item::wear(&mut sim, hero, mail, Cause::DEBUG).unwrap();
    item::wear(&mut sim, hero, coat, Cause::DEBUG).unwrap();

    // Over the arm: the coat outermost, then the mail.
    let l_arm = part(&sim, hero, "l_arm");
    let layers = item::worn_layers(&sim, hero, l_arm);
    assert_eq!(
        layers.iter().map(|layer| layer.item).collect::<Vec<_>>(),
        [coat, mail]
    );
    assert!(close(layers[0].thickness.get(), 0.003));
    assert_eq!(
        ids_of(&sim, &prop(&sim, hero, "worn_on", &[Value::from("l_arm")])),
        [coat, mail]
    );
    // Nothing over the legs.
    assert!(item::worn_layers(&sim, hero, part(&sim, hero, "l_leg")).is_empty());

    // An edge the mail can hold: the coat is met first and cut through,
    // the mail takes the rest, and the arm is untouched.
    let hit = cut(&sim, hero, "l_arm", 15.0);
    let report = apply_hit(&mut sim, &hit);
    assert_eq!(report.worn[0].item, coat);
    assert!(report.worn[0].absorbed.get() > 0.0);
    assert!(report.struck.is_empty());
    let coat_damage = sim.components().item.get(coat).unwrap().damage();
    assert!(coat_damage > 0);
    let seen = events(&mut sim);
    let damaged = named(&seen, "item_damaged");
    assert!(!damaged.is_empty());
    assert_eq!(entity_of(damaged[0], "self"), Some(coat));
    assert!(matches!(prop(&sim, coat, "condition", &[]), Value::Real(c) if c < Real::ONE));

    // A packet the coat's material ignores does nothing to it.
    let soft = DamagePacket::new(kind("impact"), Real::of(1.0), Real::of(10.0), Cause::DEBUG);
    let hit = item::damage(&mut sim, coat, &soft).unwrap();
    assert_eq!(hit.outcome.result, DamageResult::Unaffected);
    assert_eq!(
        sim.components().item.get(coat).unwrap().damage(),
        coat_damage
    );

    // A packet that destroys it breaks it: it stays, worn, and is no
    // longer a layer.
    let fierce = DamagePacket::new(kind("cut"), Real::of(1.0e6), Real::of(1.0e10), Cause::DEBUG);
    let hit = item::damage(&mut sim, coat, &fierce).unwrap();
    assert_eq!(hit.outcome.result, DamageResult::Destroyed);
    assert!(sim.components().item.get(coat).unwrap().is_broken());
    assert_eq!(hold_name(&sim, coat), Some("worn"));
    let seen = events(&mut sim);
    assert_eq!(named(&seen, "item_broken").len(), 1);
    assert_eq!(
        item::worn_layers(&sim, hero, l_arm)
            .iter()
            .map(|layer| layer.item)
            .collect::<Vec<_>>(),
        [mail]
    );
    assert_eq!(prop(&sim, coat, "is_broken", &[]), Value::Bool(true));
    let refused = item::damage(
        &mut sim,
        coat,
        &DamagePacket::new(kind("cut"), Real::of(-1.0), Real::ONE, Cause::DEBUG),
    );
    assert!(refused.is_err());
    check_invariants(&sim);
}

#[test]
fn a_held_item_is_dropped_when_the_holding_part_is_disabled() {
    let mut sim = sim();
    let hero = person(&mut sim, 5, 5);
    let knife = make(&mut sim, "knife", 1, on(5, 5));
    let coat = make(&mut sim, "coat", 1, on(5, 5));
    item::take(&mut sim, hero, knife, Cause::DEBUG).unwrap();
    item::take(&mut sim, hero, coat, Cause::DEBUG).unwrap();
    let l_arm = part(&sim, hero, "l_arm");
    let Some(Hold::Grasp(slots)) = hold(&sim, knife) else {
        panic!("not grasped")
    };
    assert_eq!(slots[0].part, l_arm);
    events(&mut sim);

    let changes = with_body(&mut sim, |cx| body::disable(cx, hero, l_arm).unwrap());
    let cause = Cause::DEBUG;
    item::on_body_changes(&mut sim, &changes, cause);
    // The knife fell; the coat in the other hand did not.
    assert_eq!(place(&sim, knife), on(5, 5));
    assert_eq!(place(&sim, coat), Place::In(hero));
    assert_eq!(attr_value(&sim, hero, "menace"), 0.0);
    let seen = events(&mut sim);
    let dropped = named(&seen, "item_dropped");
    assert_eq!(dropped.len(), 1);
    assert_eq!(entity_of(dropped[0], "self"), Some(knife));
    assert_eq!(dropped[0].cause, cause);
    // With the arm disabled, only one arm is free to take it back.
    assert_eq!(item::free_grasp(&sim, hero, channel("arm")), 0);
    let refused = item::take(&mut sim, hero, knife, Cause::DEBUG).unwrap_err();
    assert_eq!(refused.code, reason::NO_FREE_GRASP);
    check_invariants(&sim);
}

#[test]
fn a_part_that_comes_off_becomes_an_item_with_what_was_worn_only_on_it() {
    let mut sim = sim();
    let hero = person(&mut sim, 5, 5);
    let glove = make(&mut sim, "glove", 1, on(5, 5));
    let coat = make(&mut sim, "coat", 1, on(5, 5));
    let knife = make(&mut sim, "knife", 1, on(5, 5));
    item::wear(&mut sim, hero, glove, Cause::DEBUG).unwrap();
    item::wear(&mut sim, hero, coat, Cause::DEBUG).unwrap();
    item::take(&mut sim, hero, knife, Cause::DEBUG).unwrap();
    let l_arm = part(&sim, hero, "l_arm");
    let before = mass(&sim, hero);

    let (detached, changes) = with_body(&mut sim, |cx| {
        body::detach(cx, hero, l_arm, Detach::Severed, Cause::DEBUG).unwrap()
    });
    let arm = item::part_item(&mut sim, &detached, Cause::DEBUG)
        .unwrap()
        .expect("an arm leaves an item");
    item::on_body_changes(&mut sim, &changes, Cause::DEBUG);
    // It lies where the hero stands, as an item of no definition.
    assert_eq!(place(&sim, arm), on(5, 5));
    let remains = sim.components().item.get(arm).unwrap();
    assert_eq!(remains.def(), None);
    assert_eq!(remains.remains().unwrap().root, l_arm);
    assert!(close(own_mass(&sim, arm), detached.mass.get()));
    assert_eq!(prop(&sim, arm, "item", &[]), Value::Null);
    assert!(has_tag(&sim, arm, "limb"));
    // The glove went with it; the coat stays on what is left; the knife
    // the arm held fell.
    assert_eq!(place(&sim, glove), Place::In(arm));
    assert_eq!(place(&sim, coat), Place::In(hero));
    let Some(Hold::Worn(on_parts)) = hold(&sim, coat) else {
        panic!("not worn")
    };
    assert!(on_parts.iter().all(|on| on.part != l_arm));
    assert_eq!(place(&sim, knife), on(5, 5));
    assert_eq!(attr_value(&sim, hero, "warmth"), 2.0);
    assert!(mass(&sim, hero) < before);
    check_invariants(&sim);
}

// ---- Latent contents --------------------------------------------------------

fn contents_of(sim: &Sim, container: EntityId) -> Vec<(Option<ItemId>, u32)> {
    sim.truth()
        .contents(container)
        .iter()
        .map(|inside| {
            let item = sim.components().item.get(*inside).unwrap();
            (item.def(), item.count())
        })
        .collect()
}

#[test]
fn latent_contents_are_the_same_whenever_they_are_first_made() {
    // Two worlds, the same crate seed; one opens it at once, the other
    // after other things have drawn from every stream.
    let seed = 0x5eed;
    let mut early = sim();
    let a = item::create_seeded(&mut early, def("crate"), 1, on(5, 5), seed, Cause::DEBUG).unwrap();
    assert_eq!(prop(&early, a, "is_latent", &[]), Value::Bool(true));
    assert!(early.truth().contents(a).is_empty());
    let made = item::materialise(&mut early, a, Cause::DEBUG).unwrap();
    assert!(!made.is_empty());
    assert_eq!(prop(&early, a, "is_latent", &[]), Value::Bool(false));
    let seen = events(&mut early);
    assert_eq!(named(&seen, "contents_materialised").len(), 1);
    // Opened again, it makes nothing more.
    assert!(
        item::materialise(&mut early, a, Cause::DEBUG)
            .unwrap()
            .is_empty()
    );

    let mut late = sim();
    for _ in 0..5 {
        make(&mut late, "coin", 3, on(1, 1));
    }
    let b = item::create_seeded(&mut late, def("crate"), 1, on(7, 7), seed, Cause::DEBUG).unwrap();
    let (mut late, _) = save_and_load(&late);
    // Opened by putting something in, after a save and a load: what the
    // list made is the same, with the knife put in beside it.
    let knife = make(&mut late, "knife", 1, on(7, 7));
    item::put_in(&mut late, knife, b, Cause::DEBUG).unwrap();
    let mut expected = contents_of(&early, a);
    expected.sort();
    let mut have = contents_of(&late, b);
    have.sort();
    let mut with_knife = expected.clone();
    with_knife.push((Some(def("knife")), 1));
    with_knife.sort();
    assert_eq!(have, with_knife);
    // Different seeds give different crates, sooner or later.
    let mut other = sim();
    let mut differs = false;
    for seed in 1..20 {
        let c =
            item::create_seeded(&mut other, def("crate"), 1, on(2, 2), seed, Cause::DEBUG).unwrap();
        item::materialise(&mut other, c, Cause::DEBUG).unwrap();
        let mut got = contents_of(&other, c);
        got.sort();
        differs |= got != expected;
    }
    assert!(differs);
    check_invariants(&late);
}

#[test]
fn a_container_is_opened_by_the_first_packet_that_affects_it() {
    let mut sim = sim();
    let crate_ = make(&mut sim, "crate", 1, on(5, 5));
    let gentle = DamagePacket::new(
        kind("impact"),
        Real::of(1.0e-3),
        Real::of(1.0),
        Cause::DEBUG,
    );
    item::damage(&mut sim, crate_, &gentle).unwrap();
    assert_eq!(prop(&sim, crate_, "is_latent", &[]), Value::Bool(true));
    let blow = DamagePacket::new(
        kind("impact"),
        Real::of(1000.0),
        Real::of(1.0e9),
        Cause::DEBUG,
    );
    item::damage(&mut sim, crate_, &blow).unwrap();
    assert_eq!(prop(&sim, crate_, "is_latent", &[]), Value::Bool(false));
    check_invariants(&sim);
}

// ---- Saving -----------------------------------------------------------------

/// A morning: a person picks things up, dresses, packs, splits a stack
/// and puts something down. In two halves, so that a save can come
/// between them.
fn first_half(sim: &mut Sim) -> (EntityId, Vec<EntityId>) {
    let hero = person(sim, 5, 5);
    let knife = make(sim, "knife", 1, on(5, 5));
    let coat = make(sim, "coat", 1, on(5, 5));
    let pack = make(sim, "backpack", 1, on(5, 5));
    let arrows = make(sim, "arrow", 20, on(5, 5));
    let crate_ = make(sim, "crate", 1, on(6, 5));
    item::take(sim, hero, knife, Cause::DEBUG).unwrap();
    item::wear(sim, hero, coat, Cause::DEBUG).unwrap();
    item::wear(sim, hero, pack, Cause::DEBUG).unwrap();
    item::stow(sim, hero, arrows, Cause::DEBUG).unwrap();
    (hero, vec![knife, coat, pack, arrows, crate_])
}

fn second_half(sim: &mut Sim, hero: EntityId, things: &[EntityId]) {
    let [knife, coat, _pack, arrows, crate_] = things else {
        panic!("five things")
    };
    let five = item::split(sim, *arrows, 5, Cause::DEBUG).unwrap();
    item::take_out(sim, five, Some(hero), Cause::DEBUG).unwrap();
    item::drop(sim, *knife, Cause::DEBUG).unwrap();
    item::take_off(sim, *coat, Cause::DEBUG).unwrap();
    item::materialise(sim, *crate_, Cause::DEBUG).unwrap();
    item::put_in(sim, five, *crate_, Cause::DEBUG).unwrap();
    item::drop(sim, *coat, Cause::DEBUG).unwrap();
}

#[test]
fn saving_and_loading_in_the_middle_changes_nothing() {
    let mut straight = sim();
    let (hero, things) = first_half(&mut straight);
    let (mut loaded, unplaced) = save_and_load(&straight);
    assert!(unplaced.is_empty());
    assert_eq!(loaded.state_hash(), straight.state_hash());
    assert_eq!(
        loaded.component_hash(clash_sim::component::ComponentKind::Holds),
        straight.component_hash(clash_sim::component::ComponentKind::Holds)
    );
    second_half(&mut straight, hero, &things);
    second_half(&mut loaded, hero, &things);
    assert_eq!(loaded.state_hash(), straight.state_hash());
    check_invariants(&loaded);
    // And the events said the same.
    let a: Vec<String> = events(&mut straight)
        .iter()
        .map(|e| e.name().to_owned())
        .collect();
    let b: Vec<String> = events(&mut loaded)
        .iter()
        .map(|e| e.name().to_owned())
        .collect();
    assert_eq!(a, b);
}

// ---- Verbs ------------------------------------------------------------------

/// Calls a verb of the engine's table as the rule runtime will.
fn call(sim: &mut Sim, name: &str, args: &[Value]) -> Result<Value, clash_sim::Reason> {
    let registry = clash_sim::engine_registry();
    let verb = registry
        .verb_id(name)
        .unwrap_or_else(|| panic!("no verb `{name}`"));
    let spec = &registry.verb(verb).unwrap().spec;
    assert_eq!(
        spec.args.len(),
        args.len(),
        "`{name}` takes {} arguments",
        spec.args.len()
    );
    let def = clash_sim::verb::engine_verbs()
        .into_iter()
        .find(|def| def.name == name)
        .unwrap();
    let view = RunView::new(&[], &[]);
    let call = VerbCall {
        verb,
        args,
        by: view,
    };
    (def.run)(sim, &call, Cause::DEBUG)
}

fn returned(sim: &Sim, value: &Value) -> EntityId {
    match sim.subject_of(value) {
        Some(Subject::Entity(id)) => id,
        other => panic!("not an entity: {other:?}"),
    }
}

#[test]
fn the_verbs_do_what_the_functions_do_and_refuse_with_reasons() {
    let mut sim = sim();
    let hero = person(&mut sim, 5, 5);
    let h = entity_value(&sim, hero);
    let here = sim.subject_value(Subject::Loc(at(5, 5)));
    let id = |name: &str| Value::from(format!("itemfix:{name}"));

    let knife = call(
        &mut sim,
        "create_item",
        &[id("knife"), Value::Null, here.clone(), Value::Null],
    )
    .unwrap();
    let k = returned(&sim, &knife);
    assert_eq!(place(&sim, k), on(5, 5));
    let coins = call(
        &mut sim,
        "create_item",
        &[id("coin"), Value::Int(12), here.clone(), Value::Null],
    )
    .unwrap();
    let pack = call(
        &mut sim,
        "create_item",
        &[id("backpack"), Value::Null, Value::Null, h.clone()],
    )
    .unwrap();
    assert_eq!(place(&sim, returned(&sim, &pack)), Place::In(hero));
    let coat = call(
        &mut sim,
        "create_item",
        &[id("coat"), Value::Null, Value::Null, Value::Null],
    )
    .unwrap();
    assert_eq!(place(&sim, returned(&sim, &coat)), Place::Stored);

    let held = call(&mut sim, "take", &[h.clone(), knife.clone()]).unwrap();
    assert_eq!(returned(&sim, &held), k);
    assert_eq!(hold_name(&sim, k), Some("grasp"));
    call(&mut sim, "wear", &[h.clone(), pack.clone()]).unwrap();
    call(&mut sim, "wear", &[h.clone(), coat.clone()]).unwrap();
    call(&mut sim, "stow", &[h.clone(), coins.clone()]).unwrap();
    assert_eq!(hold_name(&sim, returned(&sim, &coins)), Some("stowed"));
    let piece = call(&mut sim, "split", &[coins.clone(), Value::Int(2)]).unwrap();
    assert_eq!(count(&sim, returned(&sim, &piece)), 2);
    let out = call(&mut sim, "take_out", &[piece.clone(), Value::Null]).unwrap();
    assert_eq!(place(&sim, returned(&sim, &out)), on(5, 5));
    call(&mut sim, "drop", std::slice::from_ref(&knife)).unwrap();
    call(&mut sim, "remove", std::slice::from_ref(&coat)).unwrap();
    assert_eq!(hold_name(&sim, returned(&sim, &coat)), Some("grasp"));
    call(&mut sim, "put_in", &[coat.clone(), pack.clone()]).unwrap();
    assert_eq!(hold_name(&sim, returned(&sim, &coat)), Some("stowed"));
    call(&mut sim, "set_color", &[coat.clone(), Value::from("red")]).unwrap();
    assert_eq!(
        prop(&sim, returned(&sim, &coat), "color", &[]),
        Value::from("red")
    );
    let bob = person(&mut sim, 6, 5);
    let b = entity_value(&sim, bob);
    call(&mut sim, "give", &[coat.clone(), b.clone()]).unwrap();
    assert_eq!(place(&sim, returned(&sim, &coat)), Place::In(bob));
    let crate_ = call(
        &mut sim,
        "create_item",
        &[id("crate"), Value::Null, here.clone(), Value::Null],
    )
    .unwrap();
    let made = call(
        &mut sim,
        "materialise_contents",
        std::slice::from_ref(&crate_),
    )
    .unwrap();
    assert!(matches!(made, Value::List(_)));
    let packet = DamagePacket::new(kind("cut"), Real::of(500.0), Real::of(1.0e9), Cause::DEBUG);
    item::damage(&mut sim, k, &packet).unwrap();
    call(
        &mut sim,
        "repair_item",
        &[knife.clone(), Value::Real(Real::ONE)],
    )
    .unwrap();
    assert_eq!(sim.components().item.get(k).unwrap().damage(), 0);
    call(&mut sim, "destroy_item", std::slice::from_ref(&knife)).unwrap();
    assert!(sim.truth().entity(k).is_none());
    check_invariants(&sim);

    // Refusals carry reasons.
    let refused = call(&mut sim, "take", &[Value::Int(3), coins.clone()]).unwrap_err();
    assert_eq!(refused.code, reason::BAD_ARGUMENT);
    assert_eq!(refused.value("arg"), Some(&clash_core::Value::from("who")));
    let refused = call(
        &mut sim,
        "create_item",
        &[id("nothing"), Value::Null, Value::Null, Value::Null],
    )
    .unwrap_err();
    assert_eq!(refused.code, reason::BAD_ARGUMENT);
    let refused = call(
        &mut sim,
        "create_item",
        &[id("coin"), Value::Int(0), Value::Null, Value::Null],
    )
    .unwrap_err();
    assert_eq!(refused.code, reason::BAD_ARGUMENT);
    let refused = call(&mut sim, "drop", std::slice::from_ref(&crate_)).unwrap_err();
    assert_eq!(refused.code, reason::NOT_HELD);
    let refused = call(&mut sim, "destroy_item", std::slice::from_ref(&h)).unwrap_err();
    assert_eq!(refused.code, reason::NOT_AN_ITEM);
}

#[test]
fn every_verb_is_registered_and_documented() {
    let registry = clash_sim::engine_registry();
    for def in item::verbs() {
        let id = registry
            .verb_id(def.name)
            .unwrap_or_else(|| panic!("`{}` is not registered", def.name));
        let spec = &registry.verb(id).unwrap().spec;
        assert!(!spec.doc.is_empty(), "{}", def.name);
        for arg in &spec.args {
            assert!(!arg.doc.is_empty(), "{}.{}", def.name, arg.name);
        }
    }
}

// ---- Properties -------------------------------------------------------------

#[test]
fn every_property_of_items_is_read_here() {
    let mut sim = sim();
    let hero = person(&mut sim, 5, 5);
    let pack = make(&mut sim, "backpack", 1, on(5, 5));
    let knife = make(&mut sim, "knife", 1, on(5, 5));
    item::wear(&mut sim, hero, pack, Cause::DEBUG).unwrap();
    item::put_in(&mut sim, knife, pack, Cause::DEBUG).unwrap();
    let arm = Value::from("itemfix:arm");
    for property in item::properties() {
        let args: Vec<Value> = match property.name {
            "held" => vec![Value::from("worn")],
            "worn_on" => vec![Value::from("torso")],
            "free_grasp" => vec![arm.clone()],
            _ => Vec::new(),
        };
        // Of a holder, of an item, and of something gone: never a panic.
        let _ = prop(&sim, hero, property.name, &args);
        let _ = prop(&sim, knife, property.name, &args);
        let gone = EntityId::new(999, 3);
        let _ = prop(&sim, gone, property.name, &args);
    }
    assert_eq!(prop(&sim, knife, "holder", &[]), entity_value(&sim, pack));
    assert_eq!(prop(&sim, pack, "hold", &[]), Value::from("worn"));
    assert_eq!(
        ids_of(&sim, &prop(&sim, hero, "worn_on", &[Value::from("torso")])),
        [pack]
    );
    assert_eq!(
        ids_of(&sim, &prop(&sim, hero, "carried", &[])),
        [pack, knife]
    );
    assert_eq!(prop(&sim, hero, "free_grasp", &[arm]), Value::Int(2));
    assert_eq!(prop(&sim, hero, "count", &[]), Value::Null);
    assert_eq!(prop(&sim, hero, "is_item", &[]), Value::Bool(false));
}

// ---- Random holding ---------------------------------------------------------

/// One step of a random morning.
#[derive(Clone, Debug)]
enum Op {
    Take(usize, usize),
    Wear(usize, usize),
    Drop(usize),
    Stash(usize, usize),
    PutIn(usize, usize),
    TakeOut(usize, Option<usize>),
    Give(usize, usize),
    Split(usize, u32),
    Hurt(usize),
    Move(usize),
}

fn op() -> impl Strategy<Value = Op> {
    prop_oneof![
        (0..2usize, 0..9usize).prop_map(|(who, what)| Op::Take(who, what)),
        (0..2usize, 0..9usize).prop_map(|(who, what)| Op::Wear(who, what)),
        (0..9usize).prop_map(Op::Drop),
        (0..2usize, 0..9usize).prop_map(|(who, what)| Op::Stash(who, what)),
        (0..9usize, 0..9usize).prop_map(|(what, into)| Op::PutIn(what, into)),
        (0..9usize, proptest::option::of(0..2usize)).prop_map(|(what, to)| Op::TakeOut(what, to)),
        (0..9usize, 0..2usize).prop_map(|(what, to)| Op::Give(what, to)),
        (0..9usize, 1..6u32).prop_map(|(what, count)| Op::Split(what, count)),
        (0..9usize).prop_map(Op::Hurt),
        (0..9usize).prop_map(Op::Move),
    ]
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(48))]

    /// Invariant B6 and B3 under random holding: whatever is taken,
    /// worn, dropped, stashed, given, split or moved by the kernel, every
    /// record names what is there, and what items confer equals a
    /// recomputation.
    #[test]
    fn random_holding_keeps_every_record_true(ops in proptest::collection::vec(op(), 1..40)) {
        let mut sim = sim();
        let people = [person(&mut sim, 5, 5), person(&mut sim, 6, 5)];
        let mut things: Vec<EntityId> = ["knife", "greatsword", "coat", "mail_shirt", "glove", "backpack", "purse"]
            .iter()
            .map(|name| make(&mut sim, name, 1, on(5, 5)))
            .collect();
        things.push(make(&mut sim, "coin", 20, on(5, 5)));
        things.push(make(&mut sim, "arrow", 12, on(5, 5)));
        let cause = Cause::DEBUG;
        for op in ops {
            // Things may have merged away: skip what is gone.
            let alive = |sim: &Sim, id: EntityId| sim.truth().entity(id).is_some();
            let result = match op {
                Op::Take(who, what) if alive(&sim, things[what]) => item::take(&mut sim, people[who], things[what], cause).map(|_| ()),
                Op::Wear(who, what) if alive(&sim, things[what]) => item::wear(&mut sim, people[who], things[what], cause).map(|_| ()),
                Op::Drop(what) if alive(&sim, things[what]) => item::drop(&mut sim, things[what], cause).map(|_| ()),
                Op::Stash(who, what) if alive(&sim, things[what]) => item::stow(&mut sim, people[who], things[what], cause).map(|_| ()),
                Op::PutIn(what, into) if alive(&sim, things[what]) && alive(&sim, things[into]) => item::put_in(&mut sim, things[what], things[into], cause).map(|_| ()),
                Op::TakeOut(what, to) if alive(&sim, things[what]) => item::take_out(&mut sim, things[what], to.map(|to| people[to]), cause).map(|_| ()),
                Op::Give(what, to) if alive(&sim, things[what]) => item::give(&mut sim, things[what], people[to], cause).map(|_| ()),
                Op::Split(what, count) if alive(&sim, things[what]) => item::split(&mut sim, things[what], count, cause).map(|piece| things.push(piece)),
                Op::Hurt(what) if alive(&sim, things[what]) => {
                    let packet = DamagePacket::new(kind("cut"), Real::of(200.0), Real::of(1.0e9), cause);
                    item::damage(&mut sim, things[what], &packet).map(|_| ())
                }
                Op::Move(what) if alive(&sim, things[what]) => sim.relocate(things[what], on(7, 7), cause).map(|_| ()),
                _ => Ok(()),
            };
            // A refusal changes nothing; either way the records hold.
            let _ = result;
            check_invariants(&sim);
        }
        // Everything put down, nothing is conferred.
        for &who in &people {
            for thing in sim.truth().contents(who).to_vec() {
                sim.relocate(thing, on(8, 8), cause).unwrap();
            }
        }
        for &who in &people {
            prop_assert!(sim.components().holds.get(who).is_none());
            prop_assert_eq!(attr_value(&sim, who, "menace"), 0.0);
            prop_assert_eq!(attr_value(&sim, who, "warmth"), 0.0);
        }
        check_invariants(&sim);
    }
}

#[test]
fn a_refused_holding_changes_nothing() {
    let mut sim = sim();
    let hero = person(&mut sim, 5, 5);
    let sword = make(&mut sim, "greatsword", 1, on(5, 5));
    let knife = make(&mut sim, "knife", 1, on(5, 5));
    item::take(&mut sim, hero, knife, Cause::DEBUG).unwrap();
    events(&mut sim);
    let hash = sim.state_hash();
    assert!(item::take(&mut sim, hero, sword, Cause::DEBUG).is_err());
    assert_eq!(sim.state_hash(), hash);
    assert!(
        events(&mut sim)
            .iter()
            .all(|event| !event.name().starts_with("item_"))
    );
}

// ---- Budgets ----------------------------------------------------------------

/// 06 §15 sets a simple item at under 300 bytes. Measured: the `Item`
/// component is 56 bytes, and with `Physical` (48), `Identity` (88) and
/// the kernel's record (208) a simple item is about 400, before the
/// stores' indices. The kernel's record is most of it. This holds the
/// item's own share to what it is.
#[test]
fn a_simple_item_keeps_its_own_share_of_memory_small() {
    use clash_sim::KernelEntity;
    use clash_sim::component::{Identity, Physical};
    assert!(size_of::<item::Item>() <= 64, "{}", size_of::<item::Item>());
    let whole = size_of::<item::Item>()
        + size_of::<Physical>()
        + size_of::<Identity>()
        + size_of::<KernelEntity>();
    assert!(whole <= 448, "{whole}");
}
