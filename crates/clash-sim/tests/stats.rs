//! Tests of attributes, pools and tracks (06 §4; package E2), against
//! the fixtures in `tests/fixtures/stats` and a small host that knows
//! one thing the stats do not: how heavy an entity is.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, LazyLock};

use clash_core::{Dur, EntityId, Hash128, Real, Rng, Tick, codec};
use clash_data::{DefTypes, Defs, LoadOutcome, Loader, Runner};
use clash_expr::{
    Context, FunctionId, Handle, Host, KindSpec, Machine, PropertyId, PropertySpec, Registry,
    RuntimeWarning, Type, Value,
};
use clash_sim::component::{ComponentKind, Components};
use clash_sim::stats::{
    self, AttributeId, Change, Changes, Modifier, PoolId, Source, SourceKind, StatDefs, StatsCx,
    StatsError, StatsStore, TrackId, props, reason,
};
use clash_time::{ClockId, Timeline, Wake, WakeDomain};
use proptest::prelude::*;

// ---- The host ---------------------------------------------------------------

/// The small host: it answers `entity.mass` and keeps the warnings it is
/// given. Everything about stats is answered by the module under test.
#[derive(Clone, Debug)]
struct World {
    mass_property: PropertyId,
    mass: BTreeMap<EntityId, Real>,
    warnings: Vec<String>,
}

impl Host for World {
    fn subject(&mut self, _slot: u16) -> Value {
        Value::Null
    }

    fn local(&mut self, _slot: u16) -> Value {
        Value::Null
    }

    fn event_field(&mut self, _slot: u16) -> Value {
        Value::Null
    }

    fn property(&mut self, property: PropertyId, subject: Handle, _args: &[Value]) -> Value {
        if property != self.mass_property {
            return Value::Null;
        }
        self.mass
            .get(&EntityId::from_bits(subject.payload))
            .map_or(Value::Null, |mass| Value::Real(*mass))
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

    fn warn(&mut self, warning: RuntimeWarning) {
        self.warnings.push(warning.fault.to_string());
    }
}

/// What the kernel would hand the loader: the built-in functions, the
/// kind `entity` and its `mass`.
fn base_registry() -> (Registry, PropertyId) {
    let mut registry = Registry::with_builtins();
    let entity = registry
        .add_kind(KindSpec::new("entity").doc("Any entity."))
        .unwrap();
    let mass = registry
        .add_property(PropertySpec::new(entity, "mass", Type::Real).doc("Mass in kilograms."))
        .unwrap();
    (registry, mass)
}

fn types() -> DefTypes {
    let mut types = DefTypes::new();
    clash_data::expr::register_defs(&mut types);
    clash_sim::register_defs(&mut types);
    types
}

fn fixtures() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/stats/statfix")
}

/// Loads a mod written in a test, as the mod `id`, with one data file.
fn load_text(id: &str, data: &str) -> LoadOutcome {
    let (registry, _) = base_registry();
    let types = types();
    Loader::new(&types)
        .expr_registry(&registry)
        .memory_mod(
            id,
            &format!(r#"{{ "id": "{id}" }}"#),
            &[("data/stats.json", data)],
        )
        .load_all()
}

fn clean(outcome: LoadOutcome) -> Arc<Defs> {
    assert_eq!(outcome.diagnostics.render(), "");
    outcome.defs.expect("a clean load gives definitions")
}

/// The fixtures, loaded once.
static FIXTURES: LazyLock<Arc<StatDefs>> = LazyLock::new(|| {
    let (registry, _) = base_registry();
    let types = types();
    let outcome = Loader::new(&types)
        .expr_registry(&registry)
        .load_mod_dir(&fixtures());
    Arc::new(StatDefs::new(&clean(outcome)).expect("the stats are registered"))
});

// ---- The rig ----------------------------------------------------------------

const HERO: EntityId = EntityId::new(3, 0);
const OTHER: EntityId = EntityId::new(4, 2);

/// Everything the functions under test are handed.
#[derive(Clone)]
struct Rig {
    defs: Arc<StatDefs>,
    stats: StatsStore,
    timeline: Timeline,
    world: World,
    runner: Runner,
}

impl Rig {
    fn over(defs: Arc<StatDefs>) -> Rig {
        let (_, mass_property) = base_registry();
        Rig {
            defs,
            stats: StatsStore::new(),
            timeline: Timeline::new(),
            world: World {
                mass_property,
                mass: BTreeMap::new(),
                warnings: Vec::new(),
            },
            runner: Runner::new(),
        }
    }

    /// The fixtures, and a hero of 80 kg with stats on the universal
    /// clock.
    fn new() -> Rig {
        let mut rig = Rig::over(Arc::clone(&FIXTURES));
        rig.world.mass.insert(HERO, Real::of(80.0));
        stats::attach(&mut rig.cx(), HERO, ClockId::UNIVERSAL).unwrap();
        rig
    }

    fn cx(&mut self) -> StatsCx<'_, World> {
        StatsCx {
            stats: &mut self.stats,
            defs: &self.defs,
            timeline: &mut self.timeline,
            host: &mut self.world,
            runner: &mut self.runner,
        }
    }

    fn attr_id(&self, name: &str) -> AttributeId {
        self.defs
            .attribute_id(&format!("statfix:{name}"))
            .expect("the fixture has the attribute")
    }

    fn pool_id(&self, name: &str) -> PoolId {
        self.defs
            .pool_id(&format!("statfix:{name}"))
            .expect("the fixture has the pool")
    }

    fn track_id(&self, name: &str) -> TrackId {
        self.defs
            .track_id(&format!("statfix:{name}"))
            .expect("the fixture has the track")
    }

    fn attr(&self, name: &str) -> f64 {
        let stats = self.stats.get(HERO).expect("the hero has stats");
        stats.attr(self.attr_id(name)).expect("a value").get()
    }

    fn level(&self, name: &str) -> f64 {
        let stats = self.stats.get(HERO).expect("the hero has stats");
        stats
            .pool_level(&self.timeline, self.pool_id(name))
            .expect("the hero has the pool")
            .get()
    }

    fn add(&mut self, name: &str, source: Source, amount: f64) -> Changes {
        let modifier = Modifier::add(self.attr_id(name), source, Real::of(amount));
        stats::add_modifier(&mut self.cx(), HERO, modifier).unwrap()
    }

    fn mul(&mut self, name: &str, source: Source, factor: f64) -> Changes {
        let modifier = Modifier::mul(self.attr_id(name), source, Real::of(factor));
        stats::add_modifier(&mut self.cx(), HERO, modifier).unwrap()
    }

    fn give_pool(&mut self, name: &str) {
        let pool = self.pool_id(name);
        stats::add_pool(&mut self.cx(), HERO, pool).unwrap();
    }

    fn pool_add(&mut self, name: &str, amount: f64) -> stats::PoolAdded {
        let pool = self.pool_id(name);
        stats::pool_add(&mut self.cx(), HERO, pool, Real::of(amount)).unwrap()
    }

    /// The one wake-up scheduled, and the tick it is due at.
    fn due(&self, name: &str) -> Option<Tick> {
        let wake = self.stats.get(HERO)?.pool_wake_id(self.pool_id(name))?;
        self.timeline.due(wake)
    }

    /// Pops the next wake-up, hands it to the module and says when it
    /// fired and what it reported.
    fn run_next(&mut self) -> (Tick, Changes) {
        let (_, wake) = self.timeline.pop(Tick::MAX).expect("a wake-up is queued");
        assert!(stats::is_pool_wake(&wake));
        let changes = stats::on_wake(&mut self.cx(), wake).unwrap();
        (self.timeline.now(), changes)
    }

    fn hash(&self) -> (Hash128, Hash128) {
        (Hash128::of(&self.stats), Hash128::of(&self.timeline))
    }
}

const A: Source = Source::new(SourceKind::Trait, 1);
const B: Source = Source::new(SourceKind::Item, 7);
const C: Source = Source::new(SourceKind::Wound, 0);

fn seconds(secs: i64) -> Tick {
    Tick(Dur::from_secs(secs).as_micros())
}

/// The attributes a list of changes names, in order, with their new
/// values.
fn attr_changes(rig: &Rig, changes: &[Change]) -> Vec<(String, f64)> {
    let names = rig.defs.defs().registry::<stats::AttributeDef>();
    changes
        .iter()
        .filter_map(|change| match change {
            Change::AttrChanged { attr, new, .. } => Some((
                names.name(*attr).trim_start_matches("statfix:").to_owned(),
                new.get(),
            )),
            _ => None,
        })
        .collect()
}

/// The thresholds a list of changes says were crossed, in order, with
/// the direction.
fn crossings(changes: &[Change]) -> Vec<(f64, bool)> {
    changes
        .iter()
        .filter_map(|change| match change {
            Change::PoolCrossed {
                threshold, rising, ..
            } => Some((threshold.get(), *rising)),
            _ => None,
        })
        .collect()
}

// ---- Definitions --------------------------------------------------------------

#[test]
fn the_fixtures_load_and_order_their_attributes() {
    let rig = Rig::new();
    let names = rig.defs.defs().registry::<stats::AttributeDef>();
    let order: Vec<&str> = rig
        .defs
        .attribute_order()
        .iter()
        .map(|attr| names.name(*attr))
        .collect();
    // Each after what it reads; of those ready, the lowest id first.
    assert_eq!(
        order,
        [
            "statfix:heft",
            "statfix:might",
            "statfix:carry",
            "statfix:wits",
            "statfix:burden"
        ]
    );
    let carry = rig.defs.attribute(rig.attr_id("carry")).unwrap();
    assert!(carry.is_derived());
    assert_eq!(carry.reads(), [rig.attr_id("might")]);
    let might = rig.defs.attribute(rig.attr_id("might")).unwrap();
    assert_eq!(might.literal_base(), Some(Real::of(10.0)));
    assert_eq!(might.seat, stats::Seat::Body);
    assert_eq!(
        rig.defs.attribute(rig.attr_id("wits")).unwrap().seat,
        stats::Seat::Mind
    );

    // Thresholds are kept lowest first however they were written.
    let stamina = rig.defs.pool(rig.pool_id("stamina")).unwrap();
    assert_eq!(stamina.thresholds, [Real::of(0.25), Real::of(1.0)]);
    assert_eq!(stamina.reads(), [rig.attr_id("might")]);

    // A track's level-up effects are carried as they were written.
    let blades = rig.defs.track(rig.track_id("blades")).unwrap();
    let effects = blades.on_advance.as_ref().expect("the fixture has effects");
    assert_eq!(effects.value.as_array().map(Vec::len), Some(1));
    assert_eq!(
        rig.defs.track(rig.track_id("rank")).unwrap().level_limit(),
        3
    );
}

#[test]
fn a_cycle_between_derived_attributes_is_a_located_load_error() {
    let data = r#"[
  { "type": "attribute", "id": "a", "base": "self.attr.b + 1" },
  { "type": "attribute", "id": "b", "base": "self.attr.c * 2" },
  { "type": "attribute", "id": "c", "base": "self.attr.a" },
  { "type": "attribute", "id": "downstream", "base": "self.attr.a" },
  { "type": "attribute", "id": "selfish", "base": "self.attr.selfish + 1" }
]"#;
    let outcome = load_text("loop", data);
    assert!(outcome.defs.is_none(), "a cycle must not load");
    let errors: Vec<_> = outcome
        .diagnostics
        .iter()
        .filter(|diagnostic| diagnostic.is_error())
        .collect();
    let found: Vec<(u32, &str)> = errors
        .iter()
        .map(|diagnostic| {
            let location = diagnostic.location.as_ref().expect("it has a place");
            assert_eq!(location.file, "mods/loop/data/stats.json");
            assert_eq!(diagnostic.path, "base");
            assert_eq!(diagnostic.code, "E0701");
            (location.line, diagnostic.message.as_str())
        })
        .collect();
    // One for each attribute on a cycle, at its own `base`; none for the
    // one that only reads them.
    assert_eq!(
        found,
        [
            (
                2,
                "attribute `loop:a` is derived from itself: loop:a → loop:b → loop:c → loop:a"
            ),
            (
                3,
                "attribute `loop:b` is derived from itself: loop:b → loop:c → loop:a → loop:b"
            ),
            (
                4,
                "attribute `loop:c` is derived from itself: loop:c → loop:a → loop:b → loop:c"
            ),
            (
                6,
                "attribute `loop:selfish` is derived from itself: loop:selfish → loop:selfish"
            ),
        ]
    );
    let column = errors[0].location.as_ref().unwrap().column;
    let line = data.lines().nth(1).unwrap();
    assert_eq!(column as usize, line.find("\"self.attr.b").unwrap() + 1);
}

#[test]
fn a_definition_out_of_range_is_refused() {
    let outcome = load_text(
        "bad",
        r#"[
  { "type": "attribute", "id": "upside", "base": 1, "min": 5, "max": 2 },
  { "type": "pool", "id": "over", "max": 10, "thresholds": [0.5, 1.5] },
  { "type": "pool", "id": "twice", "max": 10, "thresholds": [0.5, 0.5] },
  { "type": "track", "id": "odd", "levels": "$level", "on_advance": "soon" }
]"#,
    );
    assert!(outcome.defs.is_none());
    let rendered = outcome.diagnostics.render();
    assert!(
        rendered.contains("`min` (5) is above `max` (2)"),
        "{rendered}"
    );
    assert!(
        rendered.contains("from 0 to 1, and 1.5 is not"),
        "{rendered}"
    );
    assert!(
        rendered.contains("the threshold 0.5 is listed twice"),
        "{rendered}"
    );
    assert!(
        rendered.contains("`on_advance` is a list of effects"),
        "{rendered}"
    );
}

// ---- Attributes ---------------------------------------------------------------

#[test]
fn attaching_gives_every_attribute_its_base() {
    let rig = Rig::new();
    assert_eq!(rig.attr("might"), 10.0);
    assert_eq!(rig.attr("wits"), 8.0);
    assert_eq!(rig.attr("heft"), 40.0);
    assert_eq!(rig.attr("carry"), 10.0 * 4.0 + 80.0 * 0.25);
    assert_eq!(rig.attr("burden"), 60.0 * 1.5 + 8.0);

    // Twice is refused, with a reason, and changes nothing.
    let mut rig = rig;
    let before = rig.hash();
    let refused = stats::attach(&mut rig.cx(), HERO, ClockId::UNIVERSAL).unwrap_err();
    assert_eq!(refused, StatsError::HasStats { entity: HERO });
    assert_eq!(refused.code(), reason::HAS_STATS);
    assert_eq!(rig.hash(), before);
    // So is touching something with no stats.
    let might = rig.attr_id("might");
    let refused = stats::set_base(&mut rig.cx(), OTHER, might, None).unwrap_err();
    assert_eq!(refused.code(), reason::NO_STATS);
    assert_eq!(refused.to_string(), "4v2 has no stats");
}

#[test]
fn modifiers_add_then_multiply_and_the_result_is_bounded() {
    let mut rig = Rig::new();
    rig.add("might", A, 2.0);
    rig.mul("might", B, 1.5);
    assert_eq!(rig.attr("might"), (10.0 + 2.0) * 1.5);
    // A later addition still comes before every multiplication.
    rig.add("might", B, 3.0);
    assert_eq!(rig.attr("might"), (10.0 + 2.0 + 3.0) * 1.5);
    // `min` holds, and says nothing about the modifiers themselves.
    let changes = rig.add("might", C, -100.0);
    assert_eq!(rig.attr("might"), 0.0);
    assert_eq!(attr_changes(&rig, &changes)[0], ("might".to_owned(), 0.0));
    stats::remove_source(&mut rig.cx(), HERO, C).unwrap();
    assert_eq!(rig.attr("might"), 22.5);
    // `max` holds too.
    rig.mul("wits", A, 10.0);
    assert_eq!(rig.attr("wits"), 20.0);

    let stats = rig.stats.get(HERO).unwrap();
    assert_eq!(stats.modifiers_of(rig.attr_id("might")).len(), 3);
    assert_eq!(stats.modifiers().len(), 4);
}

#[test]
fn a_change_reaches_exactly_what_depends_on_it() {
    let mut rig = Rig::new();
    // The host's answer changes and nobody is told: whatever is worked
    // out from now on shows it, and whatever is not does not.
    rig.world.mass.insert(HERO, Real::of(100.0));

    let changes = rig.add("might", A, 2.0);
    assert_eq!(
        attr_changes(&rig, &changes),
        [
            ("might".to_owned(), 12.0),
            ("carry".to_owned(), 12.0 * 4.0 + 100.0 * 0.25),
            ("burden".to_owned(), 73.0 * 1.5 + 8.0),
        ]
    );
    // `heft` reads only the mass: it was not worked out again.
    assert_eq!(rig.attr("heft"), 40.0);

    rig.world.mass.insert(HERO, Real::of(200.0));
    let changes = rig.add("wits", A, 1.0);
    assert_eq!(
        attr_changes(&rig, &changes),
        [
            ("wits".to_owned(), 9.0),
            ("burden".to_owned(), 73.0 * 1.5 + 9.0)
        ]
    );
    // `carry` does not read `wits`: it still holds what it was.
    assert_eq!(rig.attr("carry"), 73.0);

    // Told that something else changed, everything is worked out.
    let changes = stats::refresh(&mut rig.cx(), HERO).unwrap();
    assert_eq!(
        attr_changes(&rig, &changes),
        [
            ("heft".to_owned(), 100.0),
            ("carry".to_owned(), 12.0 * 4.0 + 200.0 * 0.25),
            ("burden".to_owned(), 98.0 * 1.5 + 9.0),
        ]
    );
    // And a second time finds nothing to do.
    assert_eq!(stats::refresh(&mut rig.cx(), HERO).unwrap(), []);
}

#[test]
fn removing_a_source_removes_exactly_what_it_added() {
    let mut rig = Rig::new();
    rig.give_pool("dose");
    let dose = rig.pool_id("dose");

    let mut only_b = rig.clone();
    for rig in [&mut rig, &mut only_b] {
        rig.mul("might", B, 2.0);
        stats::set_rate(&mut rig.cx(), HERO, dose, B, Real::of(0.25)).unwrap();
    }
    rig.add("might", A, 2.0);
    rig.add("might", A, 1.0);
    rig.add("wits", A, 1.0);
    stats::set_rate(&mut rig.cx(), HERO, dose, A, Real::of(-1.0)).unwrap();
    assert_eq!(rig.attr("might"), (10.0 + 2.0 + 1.0) * 2.0);
    assert_eq!(
        rig.stats.get(HERO).unwrap().pool_rate(dose),
        Some(Real::of(-0.5 - 1.0 + 0.25))
    );

    let changes = stats::remove_source(&mut rig.cx(), HERO, A).unwrap();
    assert_eq!(
        attr_changes(&rig, &changes),
        [
            ("might".to_owned(), 20.0),
            ("carry".to_owned(), 100.0),
            ("wits".to_owned(), 8.0),
            ("burden".to_owned(), 158.0),
        ]
    );
    // What is left is what there would be had A never added anything.
    let (stats, twin) = (
        rig.stats.get(HERO).unwrap(),
        only_b.stats.get(HERO).unwrap(),
    );
    assert_eq!(stats.modifiers(), twin.modifiers());
    assert_eq!(stats.pool_rates(dose), twin.pool_rates(dose));
    for name in ["might", "carry", "burden", "wits", "heft"] {
        assert_eq!(rig.attr(name), only_b.attr(name), "{name}");
    }
    // Removing it again, or a source that added nothing, does nothing.
    assert_eq!(stats::remove_source(&mut rig.cx(), HERO, A).unwrap(), []);
    assert_eq!(stats::remove_source(&mut rig.cx(), HERO, C).unwrap(), []);

    // One attribute's modifiers can go alone.
    rig.add("wits", B, 4.0);
    let wits = rig.attr_id("wits");
    stats::remove_modifier(&mut rig.cx(), HERO, B, wits).unwrap();
    assert_eq!(rig.attr("wits"), 8.0);
    assert_eq!(rig.attr("might"), 20.0);
}

#[test]
fn a_base_of_its_own_stands_in_for_the_definitions() {
    let mut rig = Rig::new();
    let (might, carry) = (rig.attr_id("might"), rig.attr_id("carry"));
    stats::set_base(&mut rig.cx(), HERO, might, Some(Real::of(11.0))).unwrap();
    assert_eq!(rig.attr("might"), 11.0);
    assert_eq!(rig.attr("carry"), 64.0);

    // Given a base, a derived attribute no longer follows what it read.
    let changes = stats::set_base(&mut rig.cx(), HERO, carry, Some(Real::of(5.0))).unwrap();
    assert_eq!(
        attr_changes(&rig, &changes),
        [("carry".to_owned(), 5.0), ("burden".to_owned(), 15.5)]
    );
    let changes = rig.add("might", A, 9.0);
    assert_eq!(attr_changes(&rig, &changes), [("might".to_owned(), 20.0)]);
    assert_eq!(
        rig.stats.get(HERO).unwrap().own_base(carry),
        Some(Real::of(5.0))
    );

    // Given the definition's back, it does again.
    stats::set_base(&mut rig.cx(), HERO, carry, None).unwrap();
    assert_eq!(rig.attr("carry"), 100.0);
}

#[test]
fn an_attribute_read_through_a_function_is_refused_not_left_to_go_stale() {
    let defs = clean(load_text(
        "sly",
        r#"[
  { "type": "function", "id": "peek", "params": { "who": "entity" }, "returns": "real",
    "value": "$who.attr.vigor" },
  { "type": "attribute", "id": "vigor", "base": 6 },
  { "type": "attribute", "id": "open", "base": "self.attr.vigor * 2" },
  { "type": "attribute", "id": "hidden", "base": "peek(self) * 2" }
]"#,
    ));
    let mut rig = Rig::over(Arc::new(StatDefs::new(&defs).unwrap()));
    stats::attach(&mut rig.cx(), HERO, ClockId::UNIVERSAL).unwrap();
    let id = |name: &str| rig.defs.attribute_id(&format!("sly:{name}")).unwrap();
    let (vigor, open, hidden) = (id("vigor"), id("open"), id("hidden"));
    let value = |rig: &Rig, attr| rig.stats.get(HERO).unwrap().attr(attr).unwrap().get();
    assert_eq!(value(&rig, open), 12.0);
    // Loading cannot see that `hidden` reads `vigor`, so it may not.
    assert_eq!(value(&rig, hidden), 0.0);
    assert!(
        rig.world
            .warnings
            .iter()
            .any(|warning| warning.contains("attributes read inside a function")),
        "{:?}",
        rig.world.warnings
    );
    // So the cache cannot be wrong: a change to `vigor` leaves nothing
    // behind that a recomputation would find.
    let modifier = Modifier::add(vigor, A, Real::of(1.0));
    stats::add_modifier(&mut rig.cx(), HERO, modifier).unwrap();
    assert_eq!(value(&rig, open), 14.0);
    assert_eq!(stats::refresh(&mut rig.cx(), HERO).unwrap(), []);
}

// ---- Done when: cached attributes equal recomputation -------------------------

#[derive(Clone, Debug)]
enum Op {
    Modify {
        attr: usize,
        source: u64,
        add: i32,
        mul: usize,
    },
    RemoveSource(u64),
    RemoveModifier(u64, usize),
    SetBase(usize, Option<i32>),
    SetMass(u32),
}

const ATTRS: [&str; 5] = ["might", "carry", "burden", "wits", "heft"];
const FACTORS: [f64; 6] = [1.0, 0.5, 1.5, 2.0, -1.0, 0.1];

fn op() -> impl Strategy<Value = Op> {
    prop_oneof![
        4 => (0..ATTRS.len(), 0u64..4, -40i32..40, 0..FACTORS.len()).prop_map(
            |(attr, source, add, mul)| Op::Modify {
                attr,
                source,
                add,
                mul
            }
        ),
        2 => (0u64..4).prop_map(Op::RemoveSource),
        1 => (0u64..4, 0..ATTRS.len()).prop_map(|(source, attr)| Op::RemoveModifier(source, attr)),
        1 => (0..ATTRS.len(), proptest::option::of(-20i32..60))
            .prop_map(|(attr, base)| Op::SetBase(attr, base)),
        1 => (1u32..400).prop_map(Op::SetMass),
    ]
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(96))]

    /// Invariant B3: after any run of changes, every cached attribute is
    /// bit for bit what working everything out from scratch gives, and
    /// so are the pools that read them.
    #[test]
    fn cached_attributes_equal_a_recomputation(ops in proptest::collection::vec(op(), 1..40)) {
        let mut rig = Rig::new();
        rig.give_pool("stamina");
        rig.give_pool("blood");
        for op in ops {
            match op {
                Op::Modify { attr, source, add, mul } => {
                    let modifier = Modifier {
                        attr: rig.attr_id(ATTRS[attr]),
                        source: Source::new(SourceKind::Rule, source),
                        add: Real::of(f64::from(add) * 0.25),
                        mul: Real::of(FACTORS[mul]),
                    };
                    stats::add_modifier(&mut rig.cx(), HERO, modifier).unwrap();
                }
                Op::RemoveSource(source) => {
                    let source = Source::new(SourceKind::Rule, source);
                    stats::remove_source(&mut rig.cx(), HERO, source).unwrap();
                }
                Op::RemoveModifier(source, attr) => {
                    let (source, attr) = (Source::new(SourceKind::Rule, source), rig.attr_id(ATTRS[attr]));
                    stats::remove_modifier(&mut rig.cx(), HERO, source, attr).unwrap();
                }
                Op::SetBase(attr, base) => {
                    let attr = rig.attr_id(ATTRS[attr]);
                    let base = base.map(|base| Real::of(f64::from(base) * 0.5));
                    stats::set_base(&mut rig.cx(), HERO, attr, base).unwrap();
                }
                Op::SetMass(mass) => {
                    // Not an attribute: the host's to announce.
                    rig.world.mass.insert(HERO, Real::of(f64::from(mass)));
                    stats::refresh(&mut rig.cx(), HERO).unwrap();
                }
            }
            let mut scratch = rig.clone();
            let found = stats::refresh(&mut scratch.cx(), HERO).unwrap();
            prop_assert_eq!(&found, &Changes::new(), "a recomputation found something to change");
            let (cached, fresh) = (rig.stats.get(HERO).unwrap(), scratch.stats.get(HERO).unwrap());
            for name in ATTRS {
                let attr = rig.attr_id(name);
                let (cached, fresh) = (cached.attr(attr).unwrap(), fresh.attr(attr).unwrap());
                prop_assert_eq!(cached.get().to_bits(), fresh.get().to_bits(), "{}", name);
            }
            for name in ["stamina", "blood"] {
                let pool = rig.pool_id(name);
                prop_assert_eq!(cached.pool_max(pool), fresh.pool_max(pool), "{}", name);
                prop_assert_eq!(cached.pool_rate(pool), fresh.pool_rate(pool), "{}", name);
            }
        }
    }
}

// ---- Pools --------------------------------------------------------------------

#[test]
fn an_idle_pool_schedules_nothing() {
    let mut rig = Rig::new();
    // Empty and fading: it is already where its rate takes it.
    rig.give_pool("dose");
    // No rate at all.
    rig.give_pool("blood");
    assert_eq!(rig.timeline.wake_count(), 0);
    assert_eq!(rig.level("dose"), 0.0);
    assert_eq!(rig.level("blood"), 80.0 * 0.07);
    rig.timeline.advance_to(seconds(3600));
    assert_eq!(rig.level("dose"), 0.0);
    assert_eq!(rig.level("blood"), 80.0 * 0.07);
    assert_eq!(rig.timeline.wake_count(), 0);

    // Rising with no threshold above it: nothing either. The bound is
    // kept without a wake-up.
    let blood = rig.pool_id("blood");
    stats::pool_add(&mut rig.cx(), HERO, blood, Real::of(-0.5)).unwrap();
    stats::set_rate(&mut rig.cx(), HERO, blood, A, Real::of(0.125)).unwrap();
    assert_eq!(rig.timeline.wake_count(), 0);
    rig.timeline.advance_to(seconds(3602));
    assert_eq!(rig.level("blood"), 80.0 * 0.07 - 0.5 + 0.25);
    rig.timeline.advance_to(seconds(7200));
    assert_eq!(rig.level("blood"), 80.0 * 0.07);
}

#[test]
fn a_draining_pool_crosses_at_the_analytic_tick() {
    let mut rig = Rig::new();
    rig.give_pool("dose");
    // Up past the half mark, which is reported at once.
    let added = rig.pool_add("dose", 80.0);
    assert_eq!(added.applied, Real::of(80.0));
    assert_eq!(crossings(&added.changes), [(0.5, true)]);
    // It fades at half a unit a second: thirty units take a minute.
    assert_eq!(rig.timeline.wake_count(), 1);
    assert_eq!(rig.due("dose"), Some(seconds(60)));

    // Nothing runs in between, and the level can be read at any moment.
    rig.timeline.advance_to(seconds(30));
    assert_eq!(rig.level("dose"), 65.0);
    assert_eq!(rig.timeline.wake_count(), 1);

    let (fired, changes) = rig.run_next();
    assert_eq!(fired, seconds(60));
    assert_eq!(
        changes,
        [Change::PoolCrossed {
            entity: HERO,
            pool: rig.pool_id("dose"),
            threshold: Real::of(0.5),
            rising: false,
            level: Real::of(50.0),
        }]
    );
    // Below its only threshold, it goes on to empty with no wake-up.
    assert_eq!(rig.timeline.wake_count(), 0);
    rig.timeline.advance_to(seconds(100));
    assert_eq!(rig.level("dose"), 30.0);
    rig.timeline.advance_to(seconds(1000));
    assert_eq!(rig.level("dose"), 0.0);
}

#[test]
fn a_pool_crosses_each_threshold_once_in_order() {
    let mut rig = Rig::new();
    rig.give_pool("blood");
    let blood = rig.pool_id("blood");
    let max = rig.stats.get(HERO).unwrap().pool_max(blood).unwrap().get();
    assert_eq!(rig.stats.get(HERO).unwrap().pool_band(blood), Some(3));

    // A cut: a tenth of a unit a second.
    stats::set_rate(&mut rig.cx(), HERO, blood, C, Real::of(-0.1)).unwrap();
    let mut seen = Vec::new();
    while rig.timeline.wake_count() > 0 {
        let (fired, changes) = rig.run_next();
        for (threshold, rising) in crossings(&changes) {
            assert!(!rising);
            seen.push((threshold, fired));
        }
    }
    let thresholds: Vec<f64> = seen.iter().map(|(threshold, _)| *threshold).collect();
    assert_eq!(thresholds, [0.7, 0.4, 0.0]);
    for (threshold, fired) in seen {
        // The line meets the mark after (1 − threshold) × max ÷ rate.
        let exact = (1.0 - threshold) * max / 0.1 * 1e6;
        let off = fired.as_micros() as f64 - exact;
        assert!(
            (0.0..2.0).contains(&off),
            "{threshold}: {off} µs from {exact}"
        );
    }
    assert_eq!(rig.level("blood"), 0.0);
    assert_eq!(rig.stats.get(HERO).unwrap().pool_band(blood), Some(0));
}

#[test]
fn a_crossing_keeps_its_local_time_across_a_clock_rate_change() {
    for (rate, due_after) in [(2.0, 20), (0.5, 80), (1.0, 40)] {
        let mut rig = Rig::over(Arc::clone(&FIXTURES));
        // The hero has a clock of their own.
        let clock = rig
            .timeline
            .create_clock(ClockId::UNIVERSAL, Real::ONE)
            .unwrap();
        stats::attach(&mut rig.cx(), HERO, clock).unwrap();
        rig.give_pool("dose");
        rig.pool_add("dose", 80.0);
        assert_eq!(rig.due("dose"), Some(seconds(60)));

        // Twenty seconds in, forty of the hero's own seconds remain. The
        // clock changes speed and nothing here is told.
        rig.timeline.advance_to(seconds(20));
        assert_eq!(rig.level("dose"), 70.0);
        let before = rig.stats.clone();
        rig.timeline.set_rate(clock, Real::of(rate)).unwrap();
        assert_eq!(rig.stats, before);
        assert_eq!(rig.due("dose"), Some(seconds(20 + due_after)));

        // Half way there by the hero's time, wherever that is by
        // everyone else's.
        rig.timeline.advance_to(seconds(20 + due_after / 2));
        assert_eq!(rig.level("dose"), 60.0);

        let (fired, changes) = rig.run_next();
        assert_eq!(fired, seconds(20 + due_after), "at rate {rate}");
        assert_eq!(crossings(&changes), [(0.5, false)]);
        assert_eq!(rig.level("dose"), 50.0);
    }
}

#[test]
fn a_stopped_clock_parks_the_crossing_and_the_level() {
    let mut rig = Rig::over(Arc::clone(&FIXTURES));
    let clock = rig
        .timeline
        .create_clock(ClockId::UNIVERSAL, Real::ONE)
        .unwrap();
    stats::attach(&mut rig.cx(), HERO, clock).unwrap();
    rig.give_pool("dose");
    rig.pool_add("dose", 80.0);
    rig.timeline.advance_to(seconds(20));

    rig.timeline.set_rate(clock, Real::ZERO).unwrap();
    assert_eq!(rig.timeline.peek(), None);
    rig.timeline.advance_to(seconds(5000));
    assert_eq!(rig.level("dose"), 70.0);

    rig.timeline.set_rate(clock, Real::ONE).unwrap();
    let (fired, changes) = rig.run_next();
    assert_eq!(fired, seconds(5040));
    assert_eq!(crossings(&changes), [(0.5, false)]);
}

#[test]
fn a_rate_change_schedules_the_crossing_again() {
    let mut rig = Rig::new();
    rig.give_pool("dose");
    let dose = rig.pool_id("dose");
    rig.pool_add("dose", 80.0);
    let first = rig.stats.get(HERO).unwrap().pool_wake_id(dose).unwrap();

    // Ten seconds in, at 75, something drains it twice as fast: the
    // twenty-five left take twenty-five seconds.
    rig.timeline.advance_to(seconds(10));
    stats::set_rate(&mut rig.cx(), HERO, dose, C, Real::of(-0.5)).unwrap();
    assert_eq!(rig.due("dose"), Some(seconds(35)));
    assert_eq!(rig.timeline.wake_count(), 1);
    assert_eq!(rig.timeline.wake(first), None);

    // Ten more, at 65, and it stops: fifteen left at the old pace.
    rig.timeline.advance_to(seconds(20));
    stats::remove_rate(&mut rig.cx(), HERO, dose, C).unwrap();
    assert_eq!(rig.level("dose"), 65.0);
    assert_eq!(rig.due("dose"), Some(seconds(50)));

    // Fed as fast as it fades, it goes nowhere and waits for nothing.
    stats::set_rate(&mut rig.cx(), HERO, dose, B, Real::of(0.5)).unwrap();
    assert_eq!(rig.timeline.wake_count(), 0);
    rig.timeline.advance_to(seconds(900));
    assert_eq!(rig.level("dose"), 65.0);
}

#[test]
fn a_regenerating_pool_fills_and_then_rests() {
    let mut rig = Rig::new();
    rig.give_pool("stamina");
    let stamina = rig.pool_id("stamina");
    // Half of a hundred, coming back at two a second.
    assert_eq!(rig.level("stamina"), 50.0);
    assert_eq!(
        rig.stats.get(HERO).unwrap().pool_rate(stamina),
        Some(Real::of(2.0))
    );
    assert_eq!(rig.due("stamina"), Some(seconds(25)));
    let (fired, changes) = rig.run_next();
    assert_eq!(fired, seconds(25));
    assert_eq!(crossings(&changes), [(1.0, true)]);
    assert_eq!(rig.level("stamina"), 100.0);
    // Full: nothing more is scheduled, and it stays full.
    assert_eq!(rig.timeline.wake_count(), 0);
    rig.timeline.advance_to(seconds(500));
    assert_eq!(rig.level("stamina"), 100.0);

    // Spent, it starts again.
    let changes = stats::pool_spend(&mut rig.cx(), HERO, stamina, Real::of(90.0)).unwrap();
    assert_eq!(crossings(&changes), [(1.0, false), (0.25, false)]);
    assert_eq!(
        rig.due("stamina"),
        Some(seconds(500) + Dur::from_millis(7500))
    );
}

#[test]
fn rates_that_are_expressions_follow_the_attributes_they_read() {
    let mut rig = Rig::new();
    rig.give_pool("stamina");
    rig.give_pool("dose");
    rig.pool_add("dose", 80.0);
    let stamina = rig.pool_id("stamina");
    let dose_wake = rig
        .stats
        .get(HERO)
        .unwrap()
        .pool_wake_id(rig.pool_id("dose"));
    assert_eq!(rig.due("stamina"), Some(seconds(25)));

    // Mightier: more to hold, and it comes back faster.
    let changes = rig.add("might", A, 5.0);
    assert_eq!(crossings(&changes), []);
    let stats = rig.stats.get(HERO).unwrap();
    assert_eq!(stats.pool_max(stamina), Some(Real::of(150.0)));
    assert_eq!(stats.pool_rate(stamina), Some(Real::of(3.0)));
    // A hundred to go at three a second, rounded up to a whole tick.
    assert_eq!(rig.due("stamina"), Some(Tick(33_333_334)));
    // A pool that does not read the attribute was left alone.
    assert_eq!(stats.pool_wake_id(rig.pool_id("dose")), dose_wake);

    // Much mightier: the same fifty is now under a quarter.
    let changes = rig.add("might", B, 10.0);
    assert_eq!(crossings(&changes), [(0.25, false)]);
    assert_eq!(
        rig.stats.get(HERO).unwrap().pool_max(stamina),
        Some(Real::of(250.0))
    );

    // Weaker than the level allows: it comes down to the new most.
    stats::remove_source(&mut rig.cx(), HERO, A).unwrap();
    stats::remove_source(&mut rig.cx(), HERO, B).unwrap();
    let changes = rig.add("might", C, -8.0);
    assert_eq!(crossings(&changes), [(1.0, true)]);
    assert_eq!(rig.level("stamina"), 20.0);
    assert_eq!(
        rig.timeline.wake_count(),
        1,
        "only the dose is still moving"
    );
}

#[test]
fn adding_is_bounded_and_spending_is_all_or_nothing() {
    let mut rig = Rig::new();
    rig.give_pool("blood");
    let blood = rig.pool_id("blood");
    let max = 80.0 * 0.07;

    // Taking more than there is takes what there is, and says so.
    let taken = rig.pool_add("blood", -2.0);
    assert_eq!(taken.applied, Real::of(-2.0));
    assert_eq!(crossings(&taken.changes), [(0.7, false)]);
    let taken = rig.pool_add("blood", -10.0);
    assert_eq!(taken.applied, Real::of(-(max - 2.0)));
    assert_eq!(crossings(&taken.changes), [(0.4, false), (0.0, false)]);
    assert_eq!(rig.level("blood"), 0.0);
    // And adding more than fits adds what fits.
    let added = rig.pool_add("blood", 10.0);
    assert_eq!(added.applied, Real::of(max));
    assert_eq!(
        crossings(&added.changes),
        [(0.0, true), (0.4, true), (0.7, true)]
    );

    // A cost is paid in full or refused, with the values involved.
    let before = rig.hash();
    let refused = stats::pool_spend(&mut rig.cx(), HERO, blood, Real::of(6.0)).unwrap_err();
    assert_eq!(refused.code(), reason::POOL_TOO_LOW);
    assert_eq!(
        refused.to_string(),
        format!("the pool `statfix:blood` holds {max}, which is less than the 6 wanted")
    );
    assert_eq!(
        refused.values(),
        [
            ("entity", Value::Int(HERO.to_bits() as i64)),
            ("pool", Value::text("statfix:blood")),
            ("wanted", Value::Real(Real::of(6.0))),
            ("have", Value::Real(Real::of(max))),
        ]
    );
    assert_eq!(rig.hash(), before, "a refusal changes nothing");
    stats::pool_spend(&mut rig.cx(), HERO, blood, Real::of(max)).unwrap();
    assert_eq!(rig.level("blood"), 0.0);

    // The other refusals.
    let dose = rig.pool_id("dose");
    let code = |result: Result<Changes, StatsError>| result.unwrap_err().code();
    assert_eq!(
        code(stats::pool_spend(&mut rig.cx(), HERO, dose, Real::ONE)),
        reason::NO_SUCH_POOL
    );
    assert_eq!(
        code(stats::pool_spend(
            &mut rig.cx(),
            HERO,
            blood,
            Real::of(-1.0)
        )),
        reason::BAD_AMOUNT
    );
    assert_eq!(
        code(stats::set_rate(
            &mut rig.cx(),
            HERO,
            blood,
            A,
            Real::INFINITY
        )),
        reason::BAD_AMOUNT
    );
    assert_eq!(
        stats::add_pool(&mut rig.cx(), HERO, blood)
            .unwrap_err()
            .code(),
        reason::HAS_POOL
    );
    assert_eq!(
        stats::add_pool(&mut rig.cx(), OTHER, blood)
            .unwrap_err()
            .code(),
        reason::NO_STATS
    );
    stats::remove_pool(&mut rig.cx(), HERO, blood).unwrap();
    assert!(!rig.stats.get(HERO).unwrap().has_pool(blood));
}

#[test]
fn saving_and_loading_mid_drain_changes_nothing() {
    let mut straight = Rig::new();
    straight.give_pool("dose");
    straight.give_pool("blood");
    let blood = straight.pool_id("blood");
    straight.pool_add("dose", 80.0);
    stats::set_rate(&mut straight.cx(), HERO, blood, C, Real::of(-0.1)).unwrap();
    straight.add("might", A, 2.0);
    straight.timeline.advance_to(seconds(7));

    // Saved and loaded in the middle of two drains.
    let mut loaded = straight.clone();
    loaded.stats = codec::from_bytes(&codec::to_bytes(&straight.stats).unwrap()).unwrap();
    loaded.timeline = codec::from_bytes(&codec::to_bytes(&straight.timeline).unwrap()).unwrap();
    assert_eq!(loaded.stats, straight.stats);
    assert_eq!(loaded.hash(), straight.hash());

    // Both go on to the same crossings at the same ticks, and end in
    // the same state.
    let mut fired = Vec::new();
    while straight.timeline.wake_count() > 0 {
        let ran = straight.run_next();
        assert_eq!(loaded.run_next(), ran);
        assert_eq!(loaded.hash(), straight.hash());
        fired.push(ran.0);
    }
    assert_eq!(fired.len(), 4);
    assert!(fired.contains(&seconds(60)));
    assert_eq!(loaded.timeline.wake_count(), 0);
}

#[test]
fn the_component_is_saved_and_hashed_with_the_others() {
    let mut rig = Rig::new();
    rig.give_pool("dose");
    rig.pool_add("dose", 80.0);
    let track = rig.track_id("blades");
    stats::advance_track(&mut rig.cx(), HERO, track, Real::of(120.0)).unwrap();

    let mut components = Components::new();
    components.stats = rig.stats.clone();
    assert!(components.has(ComponentKind::Stats, HERO));
    assert_eq!(
        ComponentKind::from_name("stats"),
        Some(ComponentKind::Stats)
    );
    let loaded: Components = codec::from_bytes(&codec::to_bytes(&components).unwrap()).unwrap();
    assert_eq!(loaded, components);
    assert_eq!(Hash128::of(&loaded), Hash128::of(&components));
    assert_ne!(
        components.hash_of(ComponentKind::Stats),
        Components::new().hash_of(ComponentKind::Stats)
    );
    // Removed with its entity, and put back as it was.
    let bundle = components.remove_entity(HERO);
    assert_eq!(bundle.stats.as_ref(), rig.stats.get(HERO));
    components.insert_bundle(HERO, bundle);
    assert_eq!(components.stats, rig.stats);
}

#[test]
fn leaving_stops_the_pools_and_returning_starts_them() {
    let mut rig = Rig::new();
    rig.give_pool("dose");
    rig.pool_add("dose", 80.0);
    rig.timeline.advance_to(seconds(10));

    // Gone: brought up to date, nothing left on the timeline.
    assert_eq!(stats::suspend(&mut rig.cx(), HERO).unwrap(), []);
    assert_eq!(rig.timeline.wake_count(), 0);
    assert!(rig.stats.get(HERO).unwrap().is_suspended());
    rig.timeline.advance_to(seconds(500));
    assert_eq!(rig.level("dose"), 75.0);
    // Changed while away, it still schedules nothing.
    rig.pool_add("dose", 5.0);
    assert_eq!(rig.timeline.wake_count(), 0);

    // Back as it was: the thirty left take a minute from now.
    let mut paused = rig.clone();
    let changes = stats::resume(&mut paused.cx(), HERO, ClockId::UNIVERSAL, Dur::ZERO).unwrap();
    assert_eq!(changes, []);
    assert_eq!(paused.level("dose"), 80.0);
    assert_eq!(paused.due("dose"), Some(seconds(560)));

    // Back with the time away counted: what was passed is reported once.
    let away = Dur::from_secs(100);
    let changes = stats::resume(&mut rig.cx(), HERO, ClockId::UNIVERSAL, away).unwrap();
    assert_eq!(crossings(&changes), [(0.5, false)]);
    assert_eq!(rig.level("dose"), 30.0);
    assert_eq!(rig.timeline.wake_count(), 0);
}

#[test]
fn moving_to_another_clock_keeps_the_local_time_left() {
    let mut rig = Rig::new();
    let fast = rig
        .timeline
        .create_clock(ClockId::UNIVERSAL, Real::of(2.0))
        .unwrap();
    rig.give_pool("dose");
    rig.pool_add("dose", 80.0);
    rig.timeline.advance_to(seconds(10));

    // Fifty of its own seconds from the mark, on a clock twice as fast.
    assert_eq!(stats::set_clock(&mut rig.cx(), HERO, fast).unwrap(), []);
    assert_eq!(rig.stats.get(HERO).unwrap().clock(), fast);
    assert_eq!(rig.level("dose"), 75.0);
    assert_eq!(rig.due("dose"), Some(seconds(35)));
    let (fired, changes) = rig.run_next();
    assert_eq!(fired, seconds(35));
    assert_eq!(crossings(&changes), [(0.5, false)]);

    // A clock that is gone is a refusal, not a pool that stands still.
    let gone = rig
        .timeline
        .create_clock(ClockId::UNIVERSAL, Real::ONE)
        .unwrap();
    stats::set_clock(&mut rig.cx(), HERO, gone).unwrap();
    rig.timeline.destroy_clock(gone).unwrap();
    let dose = rig.pool_id("dose");
    let refused = stats::pool_add(&mut rig.cx(), HERO, dose, Real::ONE).unwrap_err();
    assert_eq!(refused.code(), reason::NO_CLOCK);
}

#[test]
fn a_wake_up_for_something_else_or_something_gone_changes_nothing() {
    let mut rig = Rig::new();
    rig.give_pool("dose");
    let dose = rig.pool_id("dose");
    rig.pool_add("dose", 80.0);
    let before = rig.hash();

    // The payload: a timer, the owner's bits, the pool with the tag.
    let wake = stats::pool_wake(HERO, dose);
    assert_eq!(wake.domain, WakeDomain::Timer);
    assert_eq!(wake.id, HERO.to_bits());
    assert_eq!(wake.arg, stats::POOL_WAKE_TAG | dose.index() as u64);
    assert!(stats::is_pool_wake(&wake));
    assert!(!stats::is_pool_wake(&Wake::new(WakeDomain::Timer, 9, 4)));
    assert!(!stats::is_pool_wake(&Wake::new(
        WakeDomain::Action,
        9,
        stats::POOL_WAKE_TAG
    )));

    // One that did not come off the timeline for this pool: ignored.
    for stray in [
        wake,
        stats::pool_wake(OTHER, dose),
        stats::pool_wake(HERO, rig.pool_id("blood")),
        Wake::new(WakeDomain::Timer, 9, 4),
    ] {
        assert_eq!(stats::on_wake(&mut rig.cx(), stray).unwrap(), []);
        assert_eq!(rig.hash(), before);
    }
}

// ---- Tracks -------------------------------------------------------------------

#[test]
fn a_track_reports_each_level_once() {
    let mut rig = Rig::new();
    let blades = rig.track_id("blades");
    let advance = |rig: &mut Rig, amount: f64| -> Vec<u32> {
        stats::advance_track(&mut rig.cx(), HERO, blades, Real::of(amount))
            .unwrap()
            .into_iter()
            .map(|change| match change {
                Change::TrackAdvanced {
                    entity,
                    track,
                    level,
                } => {
                    assert_eq!((entity, track), (HERO, blades));
                    level
                }
                other => panic!("a track reported {other:?}"),
            })
            .collect()
    };
    // The curve asks for 100, 303.1, 579.9, 918.9 …
    assert_eq!(advance(&mut rig, 50.0), [0u32; 0]);
    assert_eq!(advance(&mut rig, 60.0), [1]);
    // One advance past two levels reports both, lowest first.
    assert_eq!(advance(&mut rig, 500.0), [2, 3]);
    // Nothing is reported twice.
    assert_eq!(advance(&mut rig, 0.0), [0u32; 0]);
    assert_eq!(advance(&mut rig, 300.0), [0u32; 0]);
    assert_eq!(advance(&mut rig, 9.0), [4]);
    let stats = rig.stats.get(HERO).unwrap();
    assert_eq!(stats.track_level(blades), 4);
    assert_eq!(stats.track_amount(blades), Real::of(919.0));

    // A track with a last level stops there, however much is added.
    let rank = rig.track_id("rank");
    let changes = stats::advance_track(&mut rig.cx(), HERO, rank, Real::of(1e9)).unwrap();
    assert_eq!(changes.len(), 3);
    assert_eq!(rig.stats.get(HERO).unwrap().track_level(rank), 3);
    assert_eq!(
        stats::advance_track(&mut rig.cx(), HERO, rank, Real::of(1e9)).unwrap(),
        []
    );

    // A track only accumulates.
    let refused = stats::advance_track(&mut rig.cx(), HERO, rank, Real::of(-1.0)).unwrap_err();
    assert_eq!(refused.code(), reason::BAD_AMOUNT);
    // One never advanced is at nothing.
    let fresh = Rig::new();
    assert_eq!(fresh.stats.get(HERO).unwrap().track_level(blades), 0);
    assert_eq!(
        fresh.stats.get(HERO).unwrap().track_amount(blades),
        Real::ZERO
    );
}

// ---- Properties and events ----------------------------------------------------

/// What the kernel's host will do: forward this module's properties to
/// it and answer the rest itself.
struct Forward<'a> {
    rig: &'a mut Rig,
    subjects: Vec<Value>,
}

impl Host for Forward<'_> {
    fn subject(&mut self, slot: u16) -> Value {
        self.subjects
            .get(usize::from(slot))
            .cloned()
            .unwrap_or_default()
    }

    fn local(&mut self, _slot: u16) -> Value {
        Value::Null
    }

    fn event_field(&mut self, _slot: u16) -> Value {
        Value::Null
    }

    fn property(&mut self, property: PropertyId, subject: Handle, args: &[Value]) -> Value {
        let rig = &mut *self.rig;
        match stats::property(
            &rig.stats,
            &rig.defs,
            &rig.timeline,
            property,
            subject,
            args,
        ) {
            Some(value) => value,
            None => rig.world.property(property, subject, args),
        }
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

    fn warn(&mut self, warning: RuntimeWarning) {
        self.rig.world.warn(warning);
    }
}

fn context(rig: &Rig) -> Context {
    let entity = Type::Subject(rig.defs.kinds().entity);
    Context::new()
        .own_subject("self", entity.clone())
        .subject("target", entity)
        .namespace("statfix")
}

/// Evaluates an expression about the hero (`self`) and another entity
/// with no stats (`target`).
fn eval(rig: &mut Rig, source: &str) -> Value {
    let registry = Arc::clone(rig.defs.defs().expr_registry());
    let code = clash_expr::compile(source, &registry, &context(rig), &Type::Any)
        .into_result()
        .unwrap_or_else(|diagnostics| panic!("`{source}`: {diagnostics:?}"));
    let entity = rig.defs.kinds().entity;
    let subjects = vec![
        Value::Subject(Handle::new(entity, HERO.to_bits())),
        Value::Subject(Handle::new(entity, OTHER.to_bits())),
    ];
    Machine::new().eval(&code, &mut Forward { rig, subjects })
}

#[test]
fn stats_are_read_as_properties() {
    let mut rig = Rig::new();
    rig.give_pool("dose");
    rig.pool_add("dose", 80.0);
    let blades = rig.track_id("blades");
    stats::advance_track(&mut rig.cx(), HERO, blades, Real::of(310.0)).unwrap();
    rig.timeline.advance_to(seconds(20));
    let real = |value: f64| Value::Real(Real::of(value));

    // Attributes: by bare id, by qualified id, by an id worked out.
    assert_eq!(eval(&mut rig, "self.attr.might"), real(10.0));
    assert_eq!(eval(&mut rig, "self.attr.statfix:carry"), real(60.0));
    assert_eq!(eval(&mut rig, "self.attr.value('burden') + 1"), real(99.0));
    assert_eq!(
        eval(&mut rig, "self.attr.carry > self.attr.might * 5"),
        Value::Bool(true)
    );
    // Pools: the level now, and what goes with it.
    assert_eq!(eval(&mut rig, "self.pool.dose"), real(70.0));
    assert_eq!(eval(&mut rig, "self.pool.level('dose')"), real(70.0));
    assert_eq!(eval(&mut rig, "self.pool.frac('dose')"), real(0.7));
    assert_eq!(eval(&mut rig, "self.pool.max('statfix:dose')"), real(100.0));
    assert_eq!(eval(&mut rig, "self.pool.rate('dose')"), real(-0.5));
    // Tracks.
    assert_eq!(eval(&mut rig, "self.track.blades"), Value::Int(2));
    assert_eq!(eval(&mut rig, "self.track.level('blades')"), Value::Int(2));
    assert_eq!(eval(&mut rig, "self.track.amount('blades')"), real(310.0));
    assert_eq!(eval(&mut rig, "self.track.rank"), Value::Int(0));

    // Null where there is nothing to read: no stats, no such pool.
    assert_eq!(eval(&mut rig, "target.attr.might"), Value::Null);
    assert_eq!(eval(&mut rig, "target.track.blades"), Value::Null);
    assert_eq!(eval(&mut rig, "self.pool.blood"), Value::Null);
    assert_eq!(eval(&mut rig, "self.pool.blood > 0"), Value::Bool(false));
    assert_eq!(eval(&mut rig, "self.pool.frac('blood')"), Value::Null);
    // What the host knows is still the host's.
    assert_eq!(eval(&mut rig, "self.mass"), real(80.0));
}

#[test]
fn a_misspelt_stat_or_a_read_of_another_mind_is_a_load_error() {
    let rig = Rig::new();
    let registry = rig.defs.defs().expr_registry();
    let check = |source: &str, context: &Context| {
        clash_expr::compile(source, registry, context, &Type::Any).diagnostics
    };
    let truth = context(&rig);
    let misspelt = check("self.attr.migth", &truth);
    assert!(misspelt[0].is_error());
    assert_eq!(misspelt[0].message, "`attrs` has no property `migth`");
    assert_eq!(
        misspelt[0].suggestion.as_deref(),
        Some("did you mean `might`?")
    );
    assert!(check("self.pool.frac('doze')", &truth)[0].is_error());
    assert!(check("self.track.blade", &truth)[0].is_error());

    // In a mind context a character reads its own stats and no one
    // else's (01 R11).
    let mind = truth.clone().belief();
    assert_eq!(check("self.attr.might + self.pool.frac('dose')", &mind), []);
    let other = check("target.attr.might", &mind);
    assert!(other[0].is_error());
    assert!(
        other[0].message.contains("not safe for belief"),
        "{other:?}"
    );
}

#[test]
fn everything_registered_is_documented() {
    let rig = Rig::new();
    let registry = rig.defs.defs().expr_registry();
    let kinds = rig.defs.kinds();

    // Kinds and properties: a type, documentation, never settable,
    // never safe for belief.
    for kind in [kinds.attrs, kinds.pools, kinds.tracks] {
        assert!(!registry.kind(kind).unwrap().spec.doc.is_empty());
        let properties: Vec<_> = registry.properties_of(kind).collect();
        assert!(properties.len() >= 4);
        for property in properties {
            assert!(rig.defs.answers(property.id), "{}", property.spec.name);
            assert!(!property.spec.doc.is_empty(), "{}", property.spec.name);
            assert!(!property.spec.settable && !property.spec.belief_safe);
            for param in &property.spec.params {
                assert!(!param.doc.is_empty());
            }
        }
    }
    for name in [
        props::ATTR_PROPERTY,
        props::POOL_PROPERTY,
        props::TRACK_PROPERTY,
    ] {
        let id = registry.property_id(kinds.entity, name).unwrap();
        let spec = &registry.property(id).unwrap().spec;
        assert!(rig.defs.answers(id));
        assert!(!spec.doc.is_empty() && spec.yields_own && !spec.belief_safe);
    }
    // The host's own property is not this module's to answer.
    let mass = registry.property_id(kinds.entity, "mass").unwrap();
    assert!(!rig.defs.answers(mass));
    assert_eq!(
        stats::property(
            &rig.stats,
            &rig.defs,
            &rig.timeline,
            mass,
            Handle::new(kinds.entity, HERO.to_bits()),
            &[]
        ),
        None
    );
    // What makes an attribute stale is named, for reactive expressions.
    let might = registry.property_id(kinds.attrs, "might").unwrap();
    assert_eq!(
        registry.property(might).unwrap().spec.changed_by,
        [props::ATTR_CHANGED]
    );

    // Events: subjects, fields with types and documentation.
    let fields = |name: &str| -> Vec<String> {
        let event = registry.event(registry.event_id(name).unwrap()).unwrap();
        assert!(!event.spec.doc.is_empty(), "{name}");
        assert_eq!(event.spec.subjects, [("self".to_owned(), kinds.entity)]);
        event
            .spec
            .fields
            .iter()
            .map(|field| {
                assert!(!field.doc.is_empty(), "{name}.{}", field.name);
                format!("{}: {}", field.name, registry.type_name(&field.ty))
            })
            .collect()
    };
    assert_eq!(
        fields(props::ATTR_CHANGED),
        ["attr: id<attribute>", "old: real", "new: real"]
    );
    assert_eq!(
        fields(props::POOL_CROSSED),
        [
            "pool: id<pool>",
            "threshold: real",
            "rising: bool",
            "level: real"
        ]
    );
    assert_eq!(
        fields(props::TRACK_ADVANCED),
        ["track: id<track>", "level: int"]
    );
    assert_eq!(
        fields(props::POOL_CHANGING),
        ["pool: id<pool>", "amount: real"]
    );
    let hook = registry
        .event(registry.event_id(props::POOL_CHANGING).unwrap())
        .unwrap();
    assert!(hook.spec.hook);
    assert_eq!(hook.spec.mutable, ["amount"]);
    assert!(
        !registry
            .event(registry.event_id(props::POOL_CROSSED).unwrap())
            .unwrap()
            .spec
            .hook
    );
}

#[test]
fn a_bare_id_is_a_property_only_where_it_names_one_thing() {
    let mut registry = Registry::new();
    let attribute = registry.add_def_type("attribute", "").unwrap();
    let pool = registry.add_def_type("pool", "").unwrap();
    registry.set_def_ids(
        attribute,
        [("one:grit", 0), ("two:grit", 1), ("one:vim", 2)],
    );
    registry.set_def_ids(pool, [("one:max", 0), ("one:if", 1)]);
    props::register(&mut registry).unwrap();

    let attrs = registry.kind_id(props::ATTRS_KIND).unwrap();
    let pools = registry.kind_id(props::POOLS_KIND).unwrap();
    // Two mods have a `grit`: each is read by its qualified id.
    assert_eq!(registry.property_id(attrs, "grit"), None);
    assert!(registry.property_id(attrs, "one:grit").is_some());
    assert!(registry.property_id(attrs, "two:grit").is_some());
    assert!(registry.property_id(attrs, "vim").is_some());
    assert!(registry.property_id(attrs, "one:vim").is_some());
    // A pool named like a property of every pool, or like a word of the
    // language, keeps its qualified id.
    let max = registry.property_id(pools, "max").unwrap();
    assert_eq!(registry.property(max).unwrap().spec.params.len(), 1);
    assert!(registry.property_id(pools, "one:max").is_some());
    assert_eq!(registry.property_id(pools, "if"), None);
    assert!(registry.property_id(pools, "one:if").is_some());
    // Nobody had registered `entity`: it is there now. Registering twice
    // is refused.
    assert!(registry.kind_id("entity").is_some());
    assert!(props::register(&mut registry).is_err());
}

#[test]
fn definitions_loaded_without_the_registrations_are_refused() {
    let types = DefTypes::new();
    let defs = clean(
        Loader::new(&types)
            .memory_mod("empty", r#"{ "id": "empty" }"#, &[])
            .load_all(),
    );
    let refused = StatDefs::new(&defs).unwrap_err();
    assert_eq!(refused.code(), reason::NOT_REGISTERED);
}
