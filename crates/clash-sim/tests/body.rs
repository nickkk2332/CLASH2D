//! Tests of bodies, hits and wounds (06 §6, §7; packages E4 and E5),
//! against the fixtures in `tests/fixtures/body` and a rig that stands
//! in for the kernel: it holds the stores and the timeline, pops
//! wake-ups, and calls `stats::refresh` when a function moves.

// The tests of hits and wounds, which use the rest of the rig, follow.
#![allow(dead_code, unused_imports)]

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, LazyLock};

use clash_core::{Dur, EntityId, Hash128, Real, Rng, Tick, codec};
use clash_data::{DefTypes, Defs, LoadOutcome, Loader, Runner};
use clash_expr::{FunctionId, Handle, Host, PropertyId, Registry, RuntimeWarning, Value};
use clash_sim::action::{ChannelId, ChannelSet};
use clash_sim::body::{
    self, Body, BodyCx, BodyDefs, BodyError, BodyStore, Change, Changes, Detach, FieldDose, Hit,
    HitAt, HitReport, HitSettings, Impact, PartKey, PartState, Relation, WornLayer, WoundState,
    props, reason,
};
use clash_sim::component::{ComponentKind, ComponentStore, Components, Containment, Physical};
use clash_sim::stats::{self, StatDefs, StatsCx, StatsStore};
use clash_time::{ClockId, Timeline, Wake};
use clash_world::defs::{DamageTypeDef, DamageTypeId, MaterialDef, MaterialId};
use clash_world::{Cause, DamagePacket, DamageResult, WorldDefs};
use proptest::prelude::*;

// ---- Loading ----------------------------------------------------------------

fn types() -> DefTypes {
    let mut types = DefTypes::new();
    clash_data::expr::register_defs(&mut types);
    clash_world::register_defs(&mut types);
    clash_sim::register_defs(&mut types);
    types
}

fn fixtures() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/body/bodyfix")
}

/// Loads a mod written in a test, as the mod `id`, with one data file.
fn load_text(id: &str, data: &str) -> LoadOutcome {
    let registry = Registry::with_builtins();
    let types = types();
    Loader::new(&types)
        .expr_registry(&registry)
        .memory_mod(
            id,
            &format!(r#"{{ "id": "{id}" }}"#),
            &[("data/body.json", data)],
        )
        .load_all()
}

fn clean(outcome: LoadOutcome) -> Arc<Defs> {
    assert_eq!(outcome.diagnostics.render(), "");
    outcome.defs.expect("a clean load gives definitions")
}

/// The tables the kernel would hold.
struct Tables {
    body: Arc<BodyDefs>,
    stats: Arc<StatDefs>,
}

fn tables(defs: &Arc<Defs>) -> Tables {
    let world =
        WorldDefs::from_defs(defs).unwrap_or_else(|problems| panic!("{}", problems.render()));
    Tables {
        body: Arc::new(BodyDefs::new(defs, &world).expect("bodies are registered")),
        stats: Arc::new(StatDefs::new(defs).expect("the stats are registered")),
    }
}

/// The fixtures, loaded once.
static FIXTURES: LazyLock<Tables> = LazyLock::new(|| {
    let registry = Registry::with_builtins();
    let types = types();
    let outcome = Loader::new(&types)
        .expr_registry(&registry)
        .load_mod_dir(&fixtures());
    tables(&clean(outcome))
});

// ---- The rig ----------------------------------------------------------------

/// Nothing is inside anything.
struct Apart;

impl Containment for Apart {
    fn container(&self, _id: EntityId) -> Option<EntityId> {
        None
    }

    fn contents(&self, _id: EntityId, _visit: &mut dyn FnMut(EntityId)) {}
}

static APART: Apart = Apart;

/// A host that answers nothing: the body functions never ask it.
#[derive(Clone, Default)]
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

/// The host the kernel would hand the stats: it forwards what bodies
/// answer.
struct BodyHost<'a> {
    bodies: &'a BodyStore,
    defs: &'a BodyDefs,
}

impl Host for BodyHost<'_> {
    fn subject(&mut self, _slot: u16) -> Value {
        Value::Null
    }

    fn local(&mut self, _slot: u16) -> Value {
        Value::Null
    }

    fn event_field(&mut self, _slot: u16) -> Value {
        Value::Null
    }

    fn property(&mut self, property: PropertyId, subject: Handle, args: &[Value]) -> Value {
        body::property_at(self.bodies, self.defs, &|_| None, property, subject, args)
            .unwrap_or(Value::Null)
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

const HERO: EntityId = EntityId::new(3, 0);
const BLOB: EntityId = EntityId::new(9, 1);

/// Everything the functions under test are handed.
#[derive(Clone)]
struct Rig {
    defs: Arc<BodyDefs>,
    stat_defs: Arc<StatDefs>,
    bodies: BodyStore,
    physical: ComponentStore<Physical>,
    stats: StatsStore,
    timeline: Timeline,
    runner: Runner,
    rng: Rng,
    quiet: Quiet,
}

impl Rig {
    fn empty() -> Rig {
        Rig {
            defs: Arc::clone(&FIXTURES.body),
            stat_defs: Arc::clone(&FIXTURES.stats),
            bodies: BodyStore::new(),
            physical: ComponentStore::new(),
            stats: StatsStore::new(),
            timeline: Timeline::new(),
            runner: Runner::new(),
            rng: Rng::from_seed(7),
            quiet: Quiet,
        }
    }

    /// A humanoid hero at scale 1 with blood and pain, on the universal
    /// clock.
    fn new() -> Rig {
        let mut rig = Rig::empty();
        rig.spawn(HERO, "humanoid", 1.0);
        rig
    }

    fn cx(&mut self) -> BodyCx<'_, Quiet> {
        BodyCx {
            bodies: &mut self.bodies,
            defs: &self.defs,
            physical: &mut self.physical,
            containment: &APART,
            stats: StatsCx {
                stats: &mut self.stats,
                defs: &self.stat_defs,
                timeline: &mut self.timeline,
                host: &mut self.quiet,
                runner: &mut self.runner,
            },
        }
    }

    /// What the kernel does when a creature is made: stats, a body,
    /// pools, and the stats worked out again now that there is a body
    /// for them to read.
    fn spawn(&mut self, entity: EntityId, plan: &str, scale: f64) {
        let plan = self.plan(plan);
        {
            let mut host = BodyHost {
                bodies: &self.bodies,
                defs: &self.defs,
            };
            let mut cx = StatsCx {
                stats: &mut self.stats,
                defs: &self.stat_defs,
                timeline: &mut self.timeline,
                host: &mut host,
                runner: &mut self.runner,
            };
            stats::attach(&mut cx, entity, ClockId::UNIVERSAL).unwrap();
            for pool in ["blood", "pain"] {
                let pool = self.stat_defs.pool_id(&format!("bodyfix:{pool}")).unwrap();
                stats::add_pool(&mut cx, entity, pool).unwrap();
            }
        }
        body::attach(
            &mut self.cx(),
            entity,
            plan,
            Real::of(scale),
            ClockId::UNIVERSAL,
        )
        .unwrap();
        self.refresh_stats(entity);
    }

    /// What the kernel does when a function moved.
    fn refresh_stats(&mut self, entity: EntityId) -> stats::Changes {
        let mut host = BodyHost {
            bodies: &self.bodies,
            defs: &self.defs,
        };
        let mut cx = StatsCx {
            stats: &mut self.stats,
            defs: &self.stat_defs,
            timeline: &mut self.timeline,
            host: &mut host,
            runner: &mut self.runner,
        };
        stats::refresh(&mut cx, entity).unwrap()
    }

    fn plan(&self, name: &str) -> body::BodyPlanId {
        self.defs.plan_id(&format!("bodyfix:{name}")).unwrap()
    }

    fn function(&self, name: &str) -> body::BodyFunctionId {
        self.defs.function_id(&format!("bodyfix:{name}")).unwrap()
    }

    fn kind(&self, name: &str) -> DamageTypeId {
        self.defs
            .defs()
            .lookup::<DamageTypeDef>(&format!("bodyfix:{name}"))
            .unwrap()
    }

    fn material(&self, name: &str) -> MaterialId {
        self.defs
            .defs()
            .lookup::<MaterialDef>(&format!("bodyfix:{name}"))
            .unwrap()
    }

    fn channel(&self, name: &str) -> ChannelId {
        self.defs
            .defs()
            .lookup::<clash_sim::action::ChannelDef>(&format!("bodyfix:{name}"))
            .unwrap()
    }

    fn pool(&self, name: &str) -> stats::PoolId {
        self.stat_defs.pool_id(&format!("bodyfix:{name}")).unwrap()
    }

    fn body(&self) -> &Body {
        self.bodies.get(HERO).expect("the hero has a body")
    }

    fn key(&self, name: &str) -> PartKey {
        self.body()
            .part_named(name)
            .unwrap_or_else(|| panic!("no part `{name}`"))
            .key()
    }

    fn fn_value(&self, name: &str) -> f64 {
        self.body().function(self.function(name)).get()
    }

    fn pool_level(&self, name: &str) -> f64 {
        self.stats
            .get(HERO)
            .unwrap()
            .pool_level(&self.timeline, self.pool(name))
            .unwrap()
            .get()
    }

    fn pool_rate(&self, name: &str) -> f64 {
        self.stats
            .get(HERO)
            .unwrap()
            .pool_rate(self.pool(name))
            .unwrap()
            .get()
    }

    /// A hit at a part of the hero.
    fn hit_at(&self, part: &str, kind: &str, energy: f64, area: f64, give: f64, span: f64) -> Hit {
        Hit {
            target: HERO,
            at: HitAt::Part(self.key(part)),
            packet: DamagePacket::new(
                self.kind(kind),
                Real::of(energy),
                Real::of(energy / (area * give)),
                Cause::DEBUG,
            ),
            area: Real::of(area),
            span: Real::of(span),
            dir: Some([Real::ONE, Real::ZERO, Real::ZERO]),
        }
    }

    /// A long edge drawn across a part (07 §3.2): the reference blade.
    fn cut(&self, part: &str, energy: f64) -> Hit {
        self.hit_at(part, "cut", energy, 2.0e-5, 0.002, 0.9)
    }

    fn apply(&mut self, hit: &Hit) -> HitReport {
        self.apply_worn(hit, &[])
    }

    fn apply_worn(&mut self, hit: &Hit, worn: &[WornLayer]) -> HitReport {
        let mut rng = self.rng.clone();
        let report =
            body::apply_hit(&mut self.cx(), hit, worn, &mut rng, &HitSettings::default()).unwrap();
        self.rng = rng;
        self.check_invariants();
        report
    }

    fn disable(&mut self, part: &str) -> Changes {
        let key = self.key(part);
        let changes = body::disable(&mut self.cx(), HERO, key).unwrap();
        self.check_invariants();
        changes
    }

    fn restore(&mut self, part: &str) -> Changes {
        let key = self.key(part);
        let changes = body::restore(&mut self.cx(), HERO, key).unwrap();
        self.check_invariants();
        changes
    }

    /// Pops the next wake-up and hands it to whichever module it is
    /// for. Says when it fired and what came back.
    fn run_next(&mut self) -> Option<(Tick, Wake, Changes)> {
        let (_, wake) = self.timeline.pop(Tick::MAX)?;
        let changes = if body::is_body_wake(&wake) {
            body::on_wake(&mut self.cx(), wake).unwrap()
        } else {
            assert!(stats::is_pool_wake(&wake));
            let mut cx = StatsCx {
                stats: &mut self.stats,
                defs: &self.stat_defs,
                timeline: &mut self.timeline,
                host: &mut self.quiet,
                runner: &mut self.runner,
            };
            stats::on_wake(&mut cx, wake)
                .unwrap()
                .into_iter()
                .map(Change::Stats)
                .collect()
        };
        self.check_invariants();
        Some((self.timeline.now(), wake, changes))
    }

    /// Runs wake-ups until one is due after `until`, and gathers what
    /// they reported.
    fn run_until(&mut self, until: Tick) -> Vec<(Tick, Change)> {
        let mut seen = Vec::new();
        while self.timeline.peek().is_some_and(|due| due <= until) {
            let Some((tick, _, changes)) = self.run_next() else {
                break;
            };
            seen.extend(changes.into_iter().map(|change| (tick, change)));
        }
        self.timeline.advance_to(until);
        seen
    }

    /// Invariants B2 and B3 for every body, and that `Physical` holds
    /// what the body says.
    fn check_invariants(&self) {
        for (entity, body) in self.bodies.iter() {
            let fresh = body::recompute(body, &self.defs);
            assert_eq!(
                Hash128::of(body.derived()),
                Hash128::of(&fresh),
                "cached derived values differ from recomputation"
            );
            // B2, worked out another way: a part's slots are there
            // exactly when the part is whole.
            let mut slots: BTreeMap<ChannelId, u32> = BTreeMap::new();
            let mut channels = ChannelSet::EMPTY;
            for part in body.parts() {
                if part.state() != PartState::Whole {
                    continue;
                }
                for (channel, count) in self.defs.channels_of(part.def()) {
                    *slots.entry(channel).or_insert(0) += u32::from(count);
                    channels.insert(channel);
                }
            }
            assert_eq!(body.channels(), channels);
            let cached: BTreeMap<ChannelId, u32> = body
                .slots()
                .iter()
                .map(|slot| (slot.channel, u32::from(slot.count)))
                .collect();
            assert_eq!(cached, slots);
            for part in body.parts() {
                let def = self.defs.part(part.def()).unwrap();
                if part.is_present() {
                    let disabled = part.is_held() || part.condition() < def.disable_below;
                    assert_eq!(
                        part.state() == PartState::Disabled,
                        disabled,
                        "{}",
                        part.name()
                    );
                } else {
                    assert!(part.layers().is_empty());
                }
            }
            let physical = self.physical.get(entity).expect("a body gives a shape");
            assert_eq!(physical.own_mass().get(), body.mass());
            assert_eq!(physical.height().get(), body.height());
            // Every wound is on a part that is there, and an open one
            // has applied what it leaks.
            for wound in body.wounds() {
                assert!(
                    body.part(wound.part())
                        .is_some_and(|part| part.is_present())
                );
                assert_ne!(wound.state(), WoundState::Healed);
            }
        }
    }

    fn hash(&self) -> [Hash128; 4] {
        [
            Hash128::of(&self.bodies),
            Hash128::of(&self.stats),
            Hash128::of(&self.physical),
            Hash128::of(&self.timeline),
        ]
    }
}

fn seconds(secs: i64) -> Tick {
    Tick(Dur::from_secs(secs).as_micros())
}

fn close(a: f64, b: f64) -> bool {
    (a - b).abs() <= 1.0e-9 * a.abs().max(b.abs()).max(1.0)
}

// ---- E4: definitions and plans ----------------------------------------------

#[test]
fn the_fixtures_load_and_a_plan_lays_out() {
    let rig = Rig::new();
    let body = rig.body();
    // Fifteen parts from the plan, and the brain the head brings.
    assert_eq!(body.parts().len(), 16);
    let names: Vec<&str> = body.parts().iter().map(|part| part.name()).collect();
    assert_eq!(names[0], "torso");
    assert_eq!(names[15], "head.brain");
    let root = body.root().unwrap();
    assert_eq!(root.name(), "torso");
    assert_eq!(root.parent(), None);

    let brain = body.part_named("head.brain").unwrap();
    assert_eq!(brain.parent(), Some(rig.key("head")));
    assert_eq!(brain.relation(), Relation::In);
    assert_eq!(body.covering(brain.key()), [rig.key("head")]);
    assert!(!body.is_exposed(brain.key()));
    assert!(body.is_exposed(rig.key("l_hand")));

    // By socket, by tag, by definition.
    let shoulders: Vec<&str> = body
        .in_socket(rig.key("torso"), "shoulder")
        .map(|part| part.name())
        .collect();
    assert_eq!(shoulders, ["l_arm", "r_arm"]);
    let limb = rig.defs.defs().tags().lookup("limb").unwrap();
    assert_eq!(body.with_tag(&rig.defs, limb).count(), 6);
    let organs: Vec<&str> = body
        .inside(rig.key("torso"))
        .map(|part| part.name())
        .collect();
    assert_eq!(organs, ["heart", "l_lung", "r_lung"]);
    assert_eq!(body.part_named("l_arm").unwrap().side(), Some("left"));
    assert_eq!(body.missing().count(), 0);

    // Reach: the hand is at the end of the arm; the foot's heel reaches
    // further.
    let hand_reach = body.part_reach(&rig.defs, rig.key("l_hand")).get();
    assert!(close(hand_reach, 0.7 + 0.18));
    assert!(close(body.reach(&rig.defs).get(), 0.9 + 0.25));

    // Height and bands.
    assert!(close(body.height().get(), 1.75));
    let at_head: Vec<&str> = body
        .parts_at_height(&rig.defs, Real::of(1.64))
        .map(|part| part.name())
        .collect();
    assert_eq!(at_head, ["head", "l_eye", "r_eye"]);
    let at_ankle: Vec<&str> = body
        .parts_at_height(&rig.defs, Real::of(0.03))
        .map(|part| part.name())
        .collect();
    assert_eq!(at_ankle, ["l_foot", "r_foot"]);
}

#[test]
fn nothing_expects_limbs() {
    let mut rig = Rig::empty();
    rig.spawn(BLOB, "blob", 1.0);
    let body = rig.bodies.get(BLOB).unwrap();
    assert_eq!(body.parts().len(), 1);
    assert_eq!(body.function(rig.function("walking")).get(), 1.0);
    assert_eq!(body.function(rig.function("thought")).get(), 0.0);
    assert_eq!(body.slots_of(rig.channel("pseudopod")), 2);
    assert_eq!(body.slots_of(rig.channel("arm")), 0);
    // One layer with no weight written carries the whole part.
    assert_eq!(body.parts()[0].layers()[0].weight().get(), 1.0);
    rig.check_invariants();
}

#[test]
fn what_a_function_is_follows_its_combination() {
    let mut rig = Rig::new();
    assert_eq!(rig.fn_value("grasp"), 1.0); // sum of two hands
    assert_eq!(rig.fn_value("walking"), 1.0); // sum of two legs
    assert_eq!(rig.fn_value("breathing"), 1.0); // mean of two lungs
    assert_eq!(rig.fn_value("circulation"), 1.0); // max
    assert_eq!(rig.fn_value("footing"), 1.0); // min of two feet
    assert_eq!(rig.fn_value("sight"), 1.0);

    // A part that does not work contributes nothing, and still counts.
    let changes = rig.disable("l_lung");
    assert_eq!(rig.fn_value("breathing"), 0.5);
    assert!(changes.contains(&Change::FunctionChanged {
        entity: HERO,
        function: rig.function("breathing"),
        old: Real::ONE,
        new: Real::of(0.5),
    }));
    rig.disable("l_foot");
    assert_eq!(rig.fn_value("footing"), 0.0);
    rig.restore("l_foot");
    assert_eq!(rig.fn_value("footing"), 1.0);
    rig.check_invariants();
}

#[test]
fn a_function_feeds_a_derived_attribute() {
    let mut rig = Rig::new();
    let speed = rig.stat_defs.attribute_id("bodyfix:speed").unwrap();
    let value = |rig: &Rig| rig.stats.get(HERO).unwrap().attr(speed).unwrap().get();
    assert!(close(value(&rig), 1.4));
    let changes = rig.disable("r_leg");
    assert!(
        changes
            .iter()
            .any(|change| matches!(change, Change::FunctionChanged { .. }))
    );
    // The kernel works the stats out again when a function moved.
    let stat_changes = rig.refresh_stats(HERO);
    assert!(close(value(&rig), 0.7));
    assert_eq!(stat_changes.len(), 1);
}

#[test]
fn disabling_a_part_reports_the_slots_that_went() {
    let mut rig = Rig::new();
    let arm = rig.channel("arm");
    assert_eq!(rig.body().slots_of(arm), 2);
    let l_arm = rig.key("l_arm");
    assert_eq!(
        rig.body()
            .part_channels(&rig.defs, rig.body().part(l_arm).unwrap()),
        ChannelSet::of(arm)
    );

    let changes = body::disable(&mut rig.cx(), HERO, l_arm).unwrap();
    assert_eq!(
        changes,
        [
            Change::PartDisabled {
                entity: HERO,
                part: l_arm
            },
            Change::SlotsLost {
                entity: HERO,
                part: l_arm,
                channels: ChannelSet::of(arm),
                slots: vec![body::SlotCount {
                    channel: arm,
                    count: 1
                }],
            },
        ]
    );
    assert_eq!(rig.body().slots_of(arm), 1);
    assert_eq!(rig.body().part(l_arm).unwrap().state(), PartState::Disabled);
    // Again changes nothing.
    assert_eq!(body::disable(&mut rig.cx(), HERO, l_arm).unwrap(), []);

    let changes = body::restore(&mut rig.cx(), HERO, l_arm).unwrap();
    assert!(matches!(changes[0], Change::PartRestored { part, .. } if part == l_arm));
    assert!(
        matches!(&changes[1], Change::SlotsGained { channels, .. } if *channels == ChannelSet::of(arm))
    );
    assert_eq!(rig.body().slots_of(arm), 2);
    rig.check_invariants();
}

#[test]
fn doubling_scale_doubles_thickness_quadruples_area_and_gives_eight_times_the_mass() {
    let mut rig = Rig::new();
    let giant = EntityId::new(20, 0);
    rig.spawn(giant, "humanoid", 2.0);
    let small = rig.body();
    let large = rig.bodies.get(giant).unwrap();
    for (a, b) in small.parts().iter().zip(large.parts()) {
        for (thin, thick) in a.layers().iter().zip(b.layers()) {
            assert_eq!(thick.thickness().get(), 2.0 * thin.thickness().get());
        }
        assert_eq!(
            large.part_area(&rig.defs, b).get(),
            4.0 * small.part_area(&rig.defs, a).get()
        );
        assert_eq!(
            large.part_mass(&rig.defs, b).get(),
            8.0 * small.part_mass(&rig.defs, a).get()
        );
    }
    assert!(close(large.mass().get(), 8.0 * small.mass().get()));
    assert!(close(large.height().get(), 2.0 * small.height().get()));
    assert!(close(
        large.cross_section().get(),
        4.0 * small.cross_section().get()
    ));
    assert!(close(large.volume().get(), 8.0 * small.volume().get()));
    // The masses reached `Physical`.
    assert_eq!(
        rig.physical.get(giant).unwrap().own_mass().get(),
        large.mass()
    );
    // The same edge at the same energy goes less deep into the giant.
    let cut = rig.cut("l_arm", 30.0);
    let on_small = rig.apply(&cut);
    let mut on_giant = cut;
    on_giant.target = giant;
    on_giant.at = HitAt::Part(rig.key("l_arm"));
    let mut rng = Rng::from_seed(1);
    let on_giant = body::apply_hit(
        &mut rig.cx(),
        &on_giant,
        &[],
        &mut rng,
        &HitSettings::default(),
    )
    .unwrap();
    let bone = |report: &HitReport, rig: &Rig, entity| {
        let body = rig.bodies.get(entity).unwrap();
        let _ = report;
        body.part_named("l_arm").unwrap().layers()[2].damage()
    };
    assert!(bone(&on_small, &rig, HERO) > bone(&on_giant, &rig, giant));
}

#[test]
fn removal_detaches_the_subtree_and_returns_it() {
    let mut rig = Rig::new();
    let (l_arm, l_hand, torso) = (rig.key("l_arm"), rig.key("l_hand"), rig.key("torso"));
    let before = rig.body().mass().get();

    // The root stays, and so does what is not there.
    let refused = body::detach(&mut rig.cx(), HERO, torso, Detach::Removed, Cause::DEBUG);
    assert_eq!(refused.unwrap_err().code(), reason::ROOT_PART);

    let (detached, changes) =
        body::detach(&mut rig.cx(), HERO, l_arm, Detach::Severed, Cause::DEBUG).unwrap();
    assert_eq!(detached.root, l_arm);
    assert_eq!(detached.parent, Some(torso));
    let names: Vec<&str> = detached.parts.iter().map(|part| part.name()).collect();
    assert_eq!(names, ["l_arm", "l_hand"]);
    assert!(close(detached.mass.get(), 4.5));
    assert!(detached.parts.iter().all(|part| part.layers().len() == 3));
    assert!(close(rig.body().mass().get(), before - 4.5));
    assert!(changes.iter().any(|change| matches!(
        change,
        Change::MassChanged(mass) if mass.entity == HERO && close(mass.after.get().get(), before - 4.5)
    )));
    assert!(changes.contains(&Change::PartSevered {
        entity: HERO,
        part: l_arm,
        parent: Some(torso),
    }));
    let lost: Vec<ChannelSet> = changes
        .iter()
        .filter_map(|change| match change {
            Change::SlotsLost { channels, .. } => Some(*channels),
            _ => None,
        })
        .collect();
    assert_eq!(
        lost,
        [
            ChannelSet::of(rig.channel("arm")),
            ChannelSet::of(rig.channel("grip"))
        ]
    );

    // Gone, and still listed as gone: the hand it took still counts,
    // as nothing.
    assert_eq!(rig.body().part(l_arm).unwrap().state(), PartState::Severed);
    assert_eq!(rig.body().part(l_hand).unwrap().state(), PartState::Severed);
    assert_eq!(rig.body().missing().count(), 2);
    assert_eq!(rig.fn_value("grasp"), 0.5);
    let again = body::detach(&mut rig.cx(), HERO, l_arm, Detach::Severed, Cause::DEBUG);
    assert_eq!(again.unwrap_err().code(), reason::PART_MISSING);
    let hit = rig.cut("l_arm", 10.0);
    let mut rng = Rng::from_seed(1);
    let refused = body::apply_hit(&mut rig.cx(), &hit, &[], &mut rng, &HitSettings::default());
    assert_eq!(refused.unwrap_err().code(), reason::PART_MISSING);

    // An edit takes the part away altogether.
    let r_leg = rig.key("r_leg");
    let (detached, _) =
        body::detach(&mut rig.cx(), HERO, r_leg, Detach::Removed, Cause::DEBUG).unwrap();
    assert_eq!(detached.parts.len(), 2);
    assert!(rig.body().part(r_leg).is_none());
    assert_eq!(rig.body().parts().len(), 14);
    // With one leg, and no record of another, walking is what one gives.
    assert_eq!(rig.fn_value("walking"), 0.5);
    assert_eq!(rig.fn_value("footing"), 1.0);
    rig.check_invariants();
}

#[test]
fn a_plan_that_cannot_be_built_is_a_load_error() {
    let part = r#"{ "type": "material", "id": "stuff", "density": 1000, "toughness": 1000 },
      { "type": "body_part", "id": "lump", "mass": 1, "length": 1, "girth": 1,
        "layers": [ { "material": "stuff", "thickness": 0.1 } ],
        "sockets": { "top": { "count": 1, "accepts": ["lump"] } } },
      { "type": "body_part", "id": "other", "mass": 1, "length": 1, "girth": 1,
        "layers": [ { "material": "stuff", "thickness": 0.1 } ] }"#;
    let plan = |parts: &str| {
        load_text(
            "m",
            &format!(r#"[ {part}, {{ "type": "body_plan", "id": "p", "parts": [ {parts} ] }} ]"#),
        )
        .diagnostics
        .render()
    };
    let cases = [
        (
            r#"{ "id": "a", "part": "lump" }, { "id": "b", "part": "lump" }"#,
            "one root",
        ),
        (
            r#"{ "id": "a", "part": "lump", "on": "b" }, { "id": "b", "part": "lump", "on": "a" }"#,
            "one root",
        ),
        (
            r#"{ "id": "a", "part": "lump" }, { "id": "a", "part": "lump", "on": "a" }"#,
            "listed twice",
        ),
        (
            r#"{ "id": "a", "part": "lump" }, { "id": "b", "part": "lump", "on": "c" }"#,
            "no part `c`",
        ),
        (
            r#"{ "id": "a", "part": "lump" }, { "id": "b", "part": "lump", "on": "a", "in": "a" }"#,
            "both",
        ),
        (
            r#"{ "id": "a", "part": "lump" }, { "id": "b", "part": "lump", "on": "a", "socket": "side" }"#,
            "no socket `side`",
        ),
        (
            r#"{ "id": "a", "part": "lump" }, { "id": "b", "part": "other", "on": "a", "socket": "top" }"#,
            "does not accept",
        ),
        (
            r#"{ "id": "a", "part": "lump" }, { "id": "b", "part": "lump", "on": "a", "socket": "top" }, { "id": "c", "part": "lump", "on": "a", "socket": "top" }"#,
            "is full",
        ),
        (
            r#"{ "id": "a", "part": "lump" }, { "id": "b", "part": "lump", "on": "c" }, { "id": "c", "part": "lump", "on": "b" }"#,
            "round in a circle",
        ),
    ];
    for (parts, expected) in cases {
        let rendered = plan(parts);
        assert!(rendered.contains(expected), "{parts}\n{rendered}");
        assert!(rendered.contains("data/body.json"), "{rendered}");
    }
    // A good one loads.
    clean(load_text(
        "m",
        &format!(
            r#"[ {part}, {{ "type": "body_plan", "id": "p", "parts": [
              {{ "id": "a", "part": "lump" }}, {{ "id": "b", "part": "lump", "on": "a", "socket": "top" }} ] }} ]"#
        ),
    ));
}

#[test]
fn a_part_or_a_tissue_that_makes_no_sense_is_a_load_error() {
    let render = |data: &str| load_text("m", data).diagnostics.render();
    let stuff = r#"{ "type": "material", "id": "stuff", "density": 1000, "toughness": 1000 }"#;
    let rendered = render(&format!(
        r#"[ {stuff}, {{ "type": "body_part", "id": "bad", "mass": -1, "length": 0, "girth": 1,
             "layers": [ {{ "material": "stuff", "thickness": 0.1, "weight": 0.7 }},
                         {{ "material": "stuff", "thickness": 0.1, "weight": 0.7 }} ],
             "band": [0.9, 0.2], "disable_below": 2 }} ]"#
    ));
    for expected in [
        "`mass`",
        "`length`",
        "more than 1",
        "`band`",
        "`disable_below`",
    ] {
        assert!(rendered.contains(expected), "{expected}\n{rendered}");
    }
    let rendered = render(
        r#"[ { "type": "body_part", "id": "bare", "mass": 1, "length": 1, "girth": 1, "layers": [] } ]"#,
    );
    assert!(rendered.contains("at least one layer"), "{rendered}");

    // A tissue that names a pool nobody defined: at the material, and
    // at the part built of it.
    let rendered = render(
        r#"[ { "type": "material", "id": "flesh", "density": 1000, "toughness": 1000,
               "leak": { "pool": "ichor", "rate": 1 } },
             { "type": "body_part", "id": "lump", "mass": 1, "length": 1, "girth": 1,
               "layers": [ { "material": "flesh", "thickness": 0.1 } ] } ]"#,
    );
    assert!(rendered.contains("there is no pool `ichor`"), "{rendered}");
    assert!(
        rendered.contains("names the pool `ichor` in its `leak`"),
        "{rendered}"
    );

    // A part that sits inside itself.
    let rendered = render(&format!(
        r#"[ {stuff}, {{ "type": "body_part", "id": "doll", "mass": 1, "length": 1, "girth": 1,
               "layers": [ {{ "material": "stuff", "thickness": 0.1 }} ],
               "inside": [ {{ "id": "inner", "part": "doll" }} ] }},
             {{ "type": "body_plan", "id": "p", "parts": [ {{ "id": "a", "part": "doll" }} ] }} ]"#
    ));
    assert!(rendered.contains("one contains itself"), "{rendered}");
}

#[test]
fn layers_that_leave_their_weight_out_share_what_is_left_by_thickness() {
    let defs = clean(load_text(
        "m",
        r#"[ { "type": "material", "id": "stuff", "density": 1000, "toughness": 1000 },
             { "type": "body_part", "id": "lump", "mass": 1, "length": 1, "girth": 1,
               "layers": [ { "material": "stuff", "thickness": 0.1, "weight": 0.4 },
                           { "material": "stuff", "thickness": 0.1 },
                           { "material": "stuff", "thickness": 0.2 } ] } ]"#,
    ));
    let part = defs.registry::<body::BodyPartDef>();
    let lump = part.get(part.lookup("m:lump").unwrap());
    let weights: Vec<f64> = lump.weights().iter().map(|weight| weight.get()).collect();
    assert!(close(weights[0], 0.4) && close(weights[1], 0.2) && close(weights[2], 0.4));
}

#[test]
fn a_body_is_a_component_and_survives_saving() {
    assert_eq!(ComponentKind::from_name("body"), Some(ComponentKind::Body));
    let mut rig = Rig::new();
    let cut = rig.cut("l_arm", 30.0);
    rig.apply(&cut);
    let components = Components {
        body: rig.bodies.clone(),
        stats: rig.stats.clone(),
        physical: rig.physical.clone(),
        ..Components::default()
    };
    assert!(components.has(ComponentKind::Body, HERO));
    let bytes = codec::to_bytes(&components).unwrap();
    let loaded: Components = codec::from_bytes(&bytes).unwrap();
    assert_eq!(loaded, components);
    assert_eq!(Hash128::of(&loaded), Hash128::of(&components));
    // Destroying the entity takes its body with everything else.
    let mut loaded = loaded;
    let bundle = loaded.remove_entity(HERO);
    assert!(bundle.body.is_some());
    assert!(!loaded.has(ComponentKind::Body, HERO));
}
