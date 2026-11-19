//! Tests of bodies, hits and wounds (06 §6, §7; packages E4 and E5),
//! against the fixtures in `tests/fixtures/body` and a rig that stands
//! in for the kernel: it holds the stores and the timeline, pops
//! wake-ups, and calls `stats::refresh` when a function moves.

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

// ---- E5: hits ---------------------------------------------------------------

/// What a report accounts for, against what the hit delivered.
fn accounted(report: &HitReport) -> f64 {
    let worn: f64 = report.worn.iter().map(|worn| worn.absorbed.get()).sum();
    let exits: f64 = report
        .exits
        .iter()
        .map(|exit| exit.packet.energy.get())
        .sum();
    report.absorbed.get() + worn + report.returned.get() + exits
}

#[test]
fn at_equal_energy_a_small_contact_goes_deeper_than_a_large_one() {
    let small = {
        let mut rig = Rig::new();
        let hit = rig.hit_at("l_arm", "impact", 60.0, 1.0e-4, 0.01, 0.01);
        let report = rig.apply(&hit);
        assert!(close(accounted(&report), 60.0));
        (
            report.struck[0],
            rig.body().part_named("l_arm").unwrap().layers().to_vec(),
        )
    };
    let large = {
        let mut rig = Rig::new();
        let hit = rig.hit_at("l_arm", "impact", 60.0, 1.0e-2, 0.01, 0.1);
        let report = rig.apply(&hit);
        assert!(close(accounted(&report), 60.0));
        (
            report.struck[0],
            rig.body().part_named("l_arm").unwrap().layers().to_vec(),
        )
    };
    // The small one is through the skin and the muscle and into the
    // bone; the large one stops in the muscle.
    assert_eq!(small.0.depth, 2);
    assert_eq!(large.0.depth, 1);
    assert!(small.1[2].damage() > 0);
    assert_eq!(large.1[2].damage(), 0);
    assert!(!small.0.through && !large.0.through);
}

#[test]
fn hits_below_a_layers_threshold_never_accumulate() {
    let mut rig = Rig::new();
    let before = rig.hash();
    let mut hit = rig.hit_at("l_arm", "impact", 50.0, 5.0e-3, 0.02, 0.08);
    // Skin shrugs off anything under 200 kPa.
    hit.packet.intensity = Real::of(150.0e3);
    for _ in 0..10_000 {
        let report = rig.apply(&hit);
        assert_eq!(report.changes, []);
        assert_eq!(report.absorbed, Real::ZERO);
        assert_eq!(report.returned.get(), 50.0);
        assert!(report.struck.is_empty() || report.struck[0].depth == 0);
    }
    assert_eq!(rig.hash(), before);
    assert!(rig.body().wounds().is_empty());
    assert_eq!(rig.pool_level("pain"), 0.0);

    // A hair over the threshold does accumulate: every hit that has
    // any effect leaves the layer worse.
    hit.packet.intensity = Real::of(201.0e3);
    let mut hits = 0;
    while rig.body().part_named("l_arm").unwrap().layers()[0].damage() < 255 {
        rig.apply(&hit);
        hits += 1;
        assert!(hits <= 255);
    }
    assert!(hits > 1);
}

#[test]
fn a_severing_cut_returns_the_part_and_leaves_a_wound_on_the_parent() {
    let mut rig = Rig::new();
    let (torso, l_arm, l_hand) = (rig.key("torso"), rig.key("l_arm"), rig.key("l_hand"));
    let cut = rig.cut("l_arm", 200.0);
    let report = rig.apply(&cut);

    // Through the skin (0.08 J), the muscle (0.6 J) and the bone (40 J).
    assert!(close(report.absorbed.get(), 40.68));
    assert_eq!(report.struck.len(), 1);
    assert!(report.struck[0].through);
    assert_eq!(report.struck[0].depth, 3);
    // What is left carries on, along the cut.
    assert_eq!(report.exits.len(), 1);
    assert_eq!(report.exits[0].part, Some(l_arm));
    assert!(close(report.exits[0].packet.energy.get(), 200.0 - 40.68));
    assert_eq!(report.exits[0].dir, cut.dir);
    assert_eq!(report.exits[0].packet.kind, cut.packet.kind);
    assert!(close(accounted(&report), 200.0));

    // The arm, and the hand on it, as they were.
    assert_eq!(report.detached.len(), 1);
    let detached = &report.detached[0];
    assert_eq!(detached.how, Detach::Severed);
    assert_eq!(detached.root, l_arm);
    assert_eq!(detached.parent, Some(torso));
    let names: Vec<&str> = detached.parts.iter().map(|part| part.name()).collect();
    assert_eq!(names, ["l_arm", "l_hand"]);
    assert!(
        detached
            .parts
            .iter()
            .all(|part| part.layers().iter().all(|layer| layer.damage() == 0))
    );
    assert!(close(detached.mass.get(), 4.5));

    // On the body: gone, with a stump on the torso.
    assert_eq!(rig.body().part(l_arm).unwrap().state(), PartState::Severed);
    assert_eq!(rig.body().part(l_hand).unwrap().state(), PartState::Severed);
    assert_eq!(rig.body().wounds().len(), 1);
    let stump = &rig.body().wounds()[0];
    assert_eq!(stump.part(), torso);
    assert_eq!(stump.lost(), Some(l_arm));
    assert_eq!(stump.depth(), 3);
    assert!(close(stump.area().get(), 0.09 * 0.09));
    assert_eq!(stump.state(), WoundState::Open);
    assert_eq!(stump.closes_in(), Some(Dur::from_secs(30 * 60)));
    // Skin and muscle bleed by their share of the cross-section.
    let rate = 0.0081 * (0.05 * 2.0 / 57.0 + 3.0 * 40.0 / 57.0);
    assert!(close(rig.pool_rate("blood"), -rate));

    let changes = &report.changes;
    assert!(changes.contains(&Change::PartSevered {
        entity: HERO,
        part: l_arm,
        parent: Some(torso),
    }));
    assert!(changes.iter().any(|change| matches!(
        change,
        Change::Wounded { part, depth: 3, .. } if *part == torso
    )));
    assert!(changes.contains(&Change::FunctionChanged {
        entity: HERO,
        function: rig.function("grasp"),
        old: Real::ONE,
        new: Real::of(0.5),
    }));
    assert_eq!(
        changes
            .iter()
            .filter(|change| matches!(change, Change::SlotsLost { .. }))
            .count(),
        2
    );
    assert!(
        changes
            .iter()
            .any(|change| matches!(change, Change::MassChanged(_)))
    );

    // A short edge at the same energy goes as deep and takes nothing off.
    let mut rig = Rig::new();
    let stab = rig.hit_at("l_arm", "cut", 200.0, 2.0e-5, 0.002, 0.03);
    let report = rig.apply(&stab);
    assert!(report.detached.is_empty());
    assert!(report.struck[0].through);
    assert_eq!(rig.body().part(l_arm).unwrap().state(), PartState::Whole);
    // An impact as wide as the arm does not sever either: it ruins it.
    let mut rig = Rig::new();
    let crush = rig.hit_at("l_arm", "impact", 5.0e4, 5.0e-3, 0.001, 0.2);
    let report = rig.apply(&crush);
    assert_eq!(report.detached[0].how, Detach::Destroyed);
    assert_eq!(
        rig.body().part(l_arm).unwrap().state(),
        PartState::Destroyed
    );
    assert_eq!(rig.body().part(l_hand).unwrap().state(), PartState::Severed);
    assert!(report.changes.contains(&Change::PartDestroyed {
        entity: HERO,
        part: l_arm
    }));
}

#[test]
fn one_bad_cut() {
    let mut rig = Rig::new();
    let l_arm = rig.key("l_arm");
    let arm = rig.channel("arm");
    let cut = rig.cut("l_arm", 30.0);
    let report = rig.apply(&cut);

    // Through the skin and the muscle, and most of the way into bone.
    let layers = rig.body().part(l_arm).unwrap().layers().to_vec();
    assert_eq!(
        [layers[0].damage(), layers[1].damage(), layers[2].damage()],
        [255, 255, 187]
    );
    assert!(rig.body().part(l_arm).unwrap().condition().get() < 0.5);
    assert!(report.exits.is_empty() && report.detached.is_empty());
    assert!(close(accounted(&report), 30.0));

    // The arm is disabled, and its slot is gone.
    assert!(report.changes.contains(&Change::PartDisabled {
        entity: HERO,
        part: l_arm
    }));
    assert!(report.changes.iter().any(|change| matches!(
        change,
        Change::SlotsLost { part, channels, .. } if *part == l_arm && *channels == ChannelSet::of(arm)
    )));
    assert_eq!(rig.body().slots_of(arm), 1);

    // A wound two layers deep, as wide as the arm.
    assert_eq!(rig.body().wounds().len(), 1);
    let wound = rig.body().wounds()[0].clone();
    assert_eq!((wound.part(), wound.depth()), (l_arm, 2));
    assert!(close(wound.area().get(), 0.09 * 0.032));
    assert!(report.changes.iter().any(|change| matches!(
        change,
        Change::Wounded { wound: id, depth: 2, .. } if *id == wound.id()
    )));

    // It drains the blood: skin and muscle by their share of the cut.
    let rate = wound.leaks()[0].rate.get();
    assert!(close(
        rate,
        0.09 * 0.032 * (0.05 * 2.0 / 32.0 + 3.0 * 30.0 / 32.0)
    ));
    assert_eq!(rig.pool_rate("blood"), -rate);
    // And it hurts, by the joules each tissue took.
    assert!(close(
        rig.pool_level("pain"),
        0.08 * 0.5 + 0.6 * 0.2 + 29.32 * 0.1
    ));

    // The blood crosses its marks when the line says, each from the
    // stats' own wake-up.
    let crossing = |fraction: f64| {
        let micros = ((5.6 - fraction * 5.6_f64).abs() / rate * 1.0e6).ceil();
        Tick(micros as i64)
    };
    let seen = rig.run_until(seconds(1000));
    let crossings: Vec<(Tick, f64)> = seen
        .iter()
        .filter_map(|(tick, change)| match change {
            Change::Stats(stats::Change::PoolCrossed {
                threshold, rising, ..
            }) => {
                assert!(!rising);
                Some((*tick, threshold.get()))
            }
            _ => None,
        })
        .collect();
    assert_eq!(crossings.len(), 3);
    assert_eq!(crossings[0], (crossing(0.7), 0.7));
    assert_eq!(crossings[1].1, 0.4);
    assert_eq!(crossings[2].1, 0.0);
    let near = |tick: Tick, secs: f64| (tick.as_micros() as f64 / 1.0e6 - secs).abs() < 0.01;
    assert!(near(crossings[0].0, 207.18));
    assert!(near(crossings[1].0, 2.0 * 207.18));
    assert_eq!(rig.pool_level("blood"), 0.0);

    // Left alone, it closes when its slowest tissue does.
    let seen = rig.run_until(seconds(1801));
    assert_eq!(
        seen,
        [(
            seconds(1800),
            Change::WoundClosed {
                entity: HERO,
                wound: wound.id(),
                part: l_arm
            }
        )]
    );
    assert_eq!(rig.pool_rate("blood"), 0.0);
}

#[test]
fn a_wound_closes_heals_and_gives_the_part_back() {
    let mut rig = Rig::new();
    let l_arm = rig.key("l_arm");
    let cut = rig.cut("l_arm", 30.0);
    rig.apply(&cut);
    let wound = rig.body().wounds()[0].id();
    rig.run_until(seconds(10));

    let muscle = rig
        .defs
        .tissue(rig.material("muscle"))
        .unwrap()
        .heal_step
        .unwrap();
    let skin = rig
        .defs
        .tissue(rig.material("skin"))
        .unwrap()
        .heal_step
        .unwrap();
    assert!(close(muscle.as_micros() as f64, 86_400.0e6 / (255.0 * 0.1)));

    let changes = body::close_wound(&mut rig.cx(), HERO, wound).unwrap();
    assert_eq!(
        changes,
        [Change::WoundClosed {
            entity: HERO,
            wound,
            part: l_arm
        }]
    );
    assert_eq!(rig.pool_rate("blood"), 0.0);
    let again = body::close_wound(&mut rig.cx(), HERO, wound);
    assert_eq!(again.unwrap_err().code(), reason::WOUND_NOT_OPEN);

    // Opened again, it leaks as it did; closed again, it heals.
    let changes = body::reopen_wound(&mut rig.cx(), HERO, wound).unwrap();
    assert!(matches!(changes[0], Change::WoundReopened { .. }));
    assert!(rig.pool_rate("blood") < 0.0);
    let again = body::reopen_wound(&mut rig.cx(), HERO, wound);
    assert_eq!(again.unwrap_err().code(), reason::WOUND_NOT_CLOSED);
    body::close_wound(&mut rig.cx(), HERO, wound).unwrap();
    rig.check_invariants();

    // Nothing under the wound healed while it was open: the skin's
    // first 255th comes one step after it closed.
    let seen = rig.run_until(Tick(seconds(10).0 + skin.as_micros()));
    assert!(seen.is_empty());
    assert_eq!(rig.body().part(l_arm).unwrap().layers()[0].damage(), 254);

    // Ten days on the muscle is whole, the wound is gone, and the arm
    // came back on the way.
    let seen = rig.run_until(seconds(11 * 86_400));
    let healed_at = Tick(seconds(10).0 + 255 * muscle.as_micros());
    let healed: Vec<Tick> = seen
        .iter()
        .filter(|(_, change)| matches!(change, Change::WoundHealed { .. }))
        .map(|(tick, _)| *tick)
        .collect();
    assert_eq!(healed, [healed_at]);
    let restored: Vec<Tick> = seen
        .iter()
        .filter(|(_, change)| matches!(change, Change::PartRestored { part, .. } if *part == l_arm))
        .map(|(tick, _)| *tick)
        .collect();
    assert_eq!(restored.len(), 1);
    assert!(restored[0] < healed_at);
    assert!(seen.iter().any(|(tick, change)| *tick == restored[0]
        && matches!(change, Change::SlotsGained { part, .. } if *part == l_arm)));
    assert!(rig.body().wounds().is_empty());
    assert_eq!(rig.body().slots_of(rig.channel("arm")), 2);
    // The bone is slower: still mending.
    let layers = rig.body().part(l_arm).unwrap().layers();
    assert_eq!((layers[0].damage(), layers[1].damage()), (0, 0));
    assert!(layers[2].damage() > 0);

    // `heal_part` takes the rest away at once.
    let changes = body::heal_part(&mut rig.cx(), HERO, l_arm, Real::ONE).unwrap();
    assert!(changes.is_empty());
    assert_eq!(rig.body().part(l_arm).unwrap().condition(), Real::ONE);
    assert_eq!(rig.body().wake_id(), None);
    let refused = body::heal_part(&mut rig.cx(), HERO, l_arm, Real::ZERO);
    assert_eq!(refused.unwrap_err().code(), reason::BAD_AMOUNT);
}

#[test]
fn a_body_out_of_the_simulation_stands_still() {
    let prepared = {
        let mut rig = Rig::new();
        let cut = rig.cut("l_arm", 30.0);
        rig.apply(&cut);
        let wound = rig.body().wounds()[0].id();
        body::close_wound(&mut rig.cx(), HERO, wound).unwrap();
        rig
    };
    let day = seconds(86_400);
    let layers = |rig: &Rig| {
        let part = rig.body().part_named("l_arm").unwrap();
        [
            part.layers()[0].damage(),
            part.layers()[1].damage(),
            part.layers()[2].damage(),
        ]
    };

    // In the simulation for a day.
    let mut stayed = prepared.clone();
    stayed.run_until(day);

    // Away for a day that counts.
    let mut away = prepared.clone();
    body::suspend(&mut away.cx(), HERO).unwrap();
    assert!(away.body().is_suspended());
    assert_eq!(away.body().wake_id(), None);
    away.timeline.advance_to(day);
    assert_eq!(layers(&away), layers(&prepared));
    let changes = body::resume(
        &mut away.cx(),
        HERO,
        ClockId::UNIVERSAL,
        Dur::from_secs(86_400),
    )
    .unwrap();
    assert!(changes.is_empty());
    assert_eq!(layers(&away), layers(&stayed));
    assert_ne!(layers(&away), layers(&prepared));
    // Brought up to the same moment, the two are the same in every part
    // and every wound: only the wake-up's id, which says how many were
    // scheduled on the way, tells them apart.
    assert!(stayed.restore("torso").is_empty());
    assert!(away.body().parts() == stayed.body().parts());
    assert!(away.body().wounds() == stayed.body().wounds());
    away.check_invariants();

    // Away for a day that does not.
    let mut paused = prepared.clone();
    body::suspend(&mut paused.cx(), HERO).unwrap();
    paused.timeline.advance_to(day);
    body::resume(&mut paused.cx(), HERO, ClockId::UNIVERSAL, Dur::ZERO).unwrap();
    assert_eq!(layers(&paused), layers(&prepared));
    assert!(paused.body().wake_id().is_some());

    // On a clock at half speed, healing takes twice as long.
    let mut slowed = prepared.clone();
    let slow = slowed
        .timeline
        .create_clock(ClockId::UNIVERSAL, Real::of(0.5))
        .unwrap();
    body::set_clock(&mut slowed.cx(), HERO, slow).unwrap();
    slowed.run_until(Tick(2 * day.0));
    assert_eq!(layers(&slowed), layers(&stayed));
}

#[test]
fn a_spread_hit_is_shared_out_by_exposed_area() {
    let mut rig = Rig::new();
    let blast = rig.kind("blast");
    // What struck the cell: far more than the body can take in.
    let hit = body::exposure_hit(
        rig.body(),
        HERO,
        blast,
        Real::of(1.0e6),
        Real::of(60.0e3),
        Cause::DEBUG,
    );
    assert_eq!(hit.at, HitAt::Spread);
    // The body takes what that intensity puts into its own volume.
    let share = 60.0e3 * rig.body().volume().get();
    assert!(close(hit.packet.energy.get(), share));

    let exposed: Vec<(PartKey, f64)> = rig
        .body()
        .parts()
        .iter()
        .map(|part| (part.key(), rig.body().exposed_area(&rig.defs, part).get()))
        .filter(|(_, area)| *area > 0.0)
        .collect();
    // Twelve parts on the outside; the organs are not exposed.
    assert_eq!(exposed.len(), 12);
    let total: f64 = exposed.iter().map(|(_, area)| area).sum();

    let report = rig.apply(&hit);
    assert_eq!(report.struck.len(), 12);
    for (key, area) in &exposed {
        let struck = report
            .struck
            .iter()
            .find(|struck| struck.part == *key)
            .expect("every exposed part takes its share");
        assert!(close(struck.arrived.get(), share * area / total));
    }
    assert!(close(accounted(&report), share));
    assert!(report.detached.is_empty());

    // A small delivery is taken whole.
    let small = body::exposure_hit(
        rig.body(),
        HERO,
        blast,
        Real::of(10.0),
        Real::of(60.0e3),
        Cause::DEBUG,
    );
    assert_eq!(small.packet.energy.get(), 10.0);
}

#[test]
fn many_times_what_a_body_could_absorb_destroys_it_outright() {
    let mut rig = Rig::new();
    let blast = rig.kind("blast");
    let mut hit = Hit {
        target: HERO,
        at: HitAt::Spread,
        packet: DamagePacket::new(blast, Real::of(1.0e9), Real::of(1.0e9), Cause::DEBUG),
        area: Real::ZERO,
        span: Real::ZERO,
        dir: None,
    };
    let report = rig.apply(&hit);
    assert!(report.destroyed);
    assert!(report.struck.is_empty());
    assert!(
        report
            .changes
            .contains(&Change::BodyDestroyed { entity: HERO })
    );
    assert_eq!(
        report
            .changes
            .iter()
            .filter(|change| matches!(change, Change::PartDestroyed { .. }))
            .count(),
        16
    );
    assert!(
        rig.body()
            .parts()
            .iter()
            .all(|part| part.state() == PartState::Destroyed)
    );
    assert_eq!(rig.body().mass(), Real::ZERO);
    assert_eq!(rig.fn_value("circulation"), 0.0);
    assert!(rig.body().channels().is_empty());
    assert!(close(accounted(&report), 1.0e9));
    let needed = report.absorbed.get();

    // Less than the factor times that, and the layers are resolved.
    let mut rig = Rig::new();
    hit.packet.energy = Real::of(needed * 5.0);
    let report = rig.apply(&hit);
    assert!(!report.struck.is_empty());
    assert!(close(accounted(&report), needed * 5.0));
    // With a lower factor in the settings, the same hit is enough.
    let mut rig = Rig::new();
    let settings = HitSettings {
        overkill_factor: Real::of(4.0),
    };
    let mut rng = Rng::from_seed(1);
    let report = body::apply_hit(&mut rig.cx(), &hit, &[], &mut rng, &settings).unwrap();
    assert!(report.destroyed && report.struck.is_empty());

    // Energy alone is not enough: a great deal of it, too thinly
    // spread to affect anything, does nothing.
    let mut rig = Rig::new();
    hit.packet = DamagePacket::new(blast, Real::of(1.0e12), Real::of(1.0e3), Cause::DEBUG);
    let report = rig.apply(&hit);
    assert!(!report.destroyed);
    assert_eq!(report.absorbed, Real::ZERO);
}

#[test]
fn what_goes_through_a_part_may_strike_what_is_inside_it() {
    let base = Rig::new();
    let torso = base.key("torso");
    let stab = base.hit_at("torso", "pierce", 500.0, 5.0e-5, 0.005, 0.02);
    let mut organ_hits = 0;
    let mut example = None;
    for seed in 0..200 {
        let mut rig = base.clone();
        rig.rng = Rng::from_seed(seed);
        let report = rig.apply(&stab);
        assert!(close(accounted(&report), 500.0));
        assert!(report.struck[0].through);
        assert_eq!(report.struck[0].part, torso);
        if report.struck.len() == 2 {
            organ_hits += 1;
            example.get_or_insert((rig, report));
        } else {
            assert_eq!(report.struck.len(), 1);
        }
    }
    // The heart and the two lungs are a third of the torso between them.
    assert!((30..110).contains(&organ_hits), "{organ_hits}");

    let (rig, report) = example.expect("an organ was struck");
    let organ = report.struck[1].part;
    assert_eq!(rig.body().covering(organ), [torso]);
    // The torso first, then what is inside it.
    let landed: Vec<PartKey> = report
        .changes
        .iter()
        .filter_map(|change| match change {
            Change::HitLanded { part, .. } => Some(*part),
            _ => None,
        })
        .collect();
    assert_eq!(landed, [torso, organ]);
    // Both are wounded, and the organ's does not close by itself.
    assert_eq!(rig.body().wounds().len(), 2);
    let inner = rig.body().wounds_on(organ).next().unwrap();
    assert_eq!(inner.closes_in(), None);
    assert_eq!(inner.state(), WoundState::Open);
    assert!(inner.leaks()[0].rate.get() > 0.0);
    // What went through it left the far side.
    assert_eq!(report.exits.len(), 1);
    assert!(report.exits[0].packet.energy.get() > 400.0);
}

#[test]
fn what_is_worn_is_met_first_and_takes_what_it_can() {
    let base = Rig::new();
    let l_arm = base.key("l_arm");
    let sleeve = EntityId::new(40, 0);
    let mail = |coverage: f64, part: PartKey| WornLayer {
        part,
        item: sleeve,
        material: base.material("mail"),
        thickness: Real::of(0.002),
        coverage: Real::of(coverage),
        damage: 0,
    };

    // An edge the mail can hold: the mail takes it all.
    let mut rig = base.clone();
    let before = Hash128::of(&rig.bodies);
    let report = rig.apply_worn(&base.cut("l_arm", 15.0), &[mail(1.0, l_arm)]);
    assert_eq!(report.worn.len(), 1);
    assert_eq!(report.worn[0].item, sleeve);
    assert_eq!(report.worn[0].result, DamageResult::Damaged);
    assert_eq!(report.worn[0].absorbed.get(), 15.0);
    assert!(report.struck.is_empty());
    assert_eq!(Hash128::of(&rig.bodies), before);

    // One it cannot: cut through, and the arm takes the rest.
    let mut rig = base.clone();
    let report = rig.apply_worn(&base.cut("l_arm", 60.0), &[mail(1.0, l_arm)]);
    assert_eq!(report.worn[0].result, DamageResult::Destroyed);
    assert!(close(report.worn[0].absorbed.get(), 20.0));
    assert!(close(report.struck[0].arrived.get(), 40.0));
    assert!(close(accounted(&report), 60.0));
    assert_eq!(rig.body().wounds().len(), 1);

    // One too weak to mark it: the energy stays with the blade.
    let mut rig = base.clone();
    let report = rig.apply_worn(&base.cut("l_arm", 5.0), &[mail(1.0, l_arm)]);
    assert_eq!(report.worn[0].result, DamageResult::Unaffected);
    assert_eq!(report.returned.get(), 5.0);
    assert_eq!(Hash128::of(&rig.bodies), before);

    // Mail on the other arm is not in the way.
    let mut rig = base.clone();
    let report = rig.apply_worn(&base.cut("l_arm", 15.0), &[mail(1.0, base.key("r_arm"))]);
    assert!(report.worn.is_empty());
    assert_eq!(report.struck.len(), 1);

    // Mail that covers half the arm is in the way half the time.
    let mut stopped = 0;
    for seed in 0..200 {
        let mut rig = base.clone();
        rig.rng = Rng::from_seed(seed);
        let report = rig.apply_worn(&base.cut("l_arm", 15.0), &[mail(0.5, l_arm)]);
        stopped += usize::from(!report.worn.is_empty());
    }
    assert!((70..130).contains(&stopped), "{stopped}");
}

#[test]
fn a_hit_finds_a_part_by_height_or_by_chance() {
    let base = Rig::new();
    let at = |where_: HitAt, seed: u64| {
        let mut rig = base.clone();
        rig.rng = Rng::from_seed(seed);
        let mut hit = base.hit_at("torso", "impact", 80.0, 5.0e-3, 0.02, 0.08);
        hit.at = where_;
        let report = rig.apply(&hit);
        assert!(close(accounted(&report), 80.0));
        (
            report
                .struck
                .first()
                .map(|struck| rig.body().part(struck.part).unwrap().name().to_owned()),
            report,
        )
    };
    for seed in 0..40 {
        let (name, _) = at(HitAt::Height(Real::of(1.64)), seed);
        assert!(["head", "l_eye", "r_eye"].contains(&name.unwrap().as_str()));
        let (name, _) = at(HitAt::Height(Real::of(0.2)), seed);
        assert!(["l_leg", "r_leg"].contains(&name.unwrap().as_str()));
        let (name, _) = at(HitAt::Any, seed);
        let name = name.unwrap();
        let part = base.body().part_named(&name).unwrap();
        assert!(base.body().is_exposed(part.key()), "{name}");
    }
    // Over its head: nothing is there, and all of it passes.
    let (name, report) = at(HitAt::Height(Real::of(5.0)), 0);
    assert_eq!(name, None);
    assert_eq!(report.exits.len(), 1);
    assert_eq!(report.exits[0].part, None);
    assert_eq!(report.exits[0].packet.energy.get(), 80.0);
    assert!(report.changes.is_empty());
}

#[test]
fn what_the_world_reports_becomes_a_hit() {
    let mut rig = Rig::new();
    let impact = rig.kind("impact");
    // 02 §12.2: eighty kilograms into a wall at twenty metres a second.
    let thrown = Impact {
        kind: impact,
        energy: Real::of(16.0e3),
        intensity: Real::of(0.3e6),
        dir: Some([Real::ONE, Real::ZERO, Real::ZERO]),
        contact: None,
        cause: Cause::DEBUG,
    };
    let hit = body::impact_hit(HERO, &thrown);
    assert_eq!(hit.at, HitAt::Spread);
    assert_eq!(hit.packet.energy.get(), 16.0e3);
    assert_eq!(hit.packet.intensity.get(), 0.3e6);
    assert_eq!(hit.dir, thrown.dir);
    // The wall ignored it, and the one thrown did not.
    let report = rig.apply(&hit);
    assert!(close(accounted(&report), 16.0e3));
    assert!(report.absorbed.get() > 0.0);
    assert!(
        report
            .changes
            .iter()
            .any(|change| matches!(change, Change::HitLanded { .. }))
    );
    assert!(rig.pool_level("pain") > 0.0);
    assert!(
        rig.body()
            .parts()
            .iter()
            .any(|part| part.layers().iter().any(|layer| layer.damage() > 0))
    );

    // Onto spikes: with their surface, on one part.
    let spiked = Impact {
        contact: Some((Real::of(3.0e-5), Real::of(0.02))),
        ..thrown
    };
    let hit = body::impact_hit(HERO, &spiked);
    assert_eq!(hit.at, HitAt::Any);
    assert_eq!((hit.area.get(), hit.span.get()), (3.0e-5, 0.02));

    // A field's damage rule, one step of it.
    let dose = FieldDose {
        kind: rig.kind("blast"),
        level: 100,
        energy_per_level: Real::of(2000.0),
        intensity_per_level: Real::of(10.0e3),
        cause: Cause::DEBUG,
    };
    let hit = body::field_hit(rig.body(), HERO, &dose);
    assert_eq!(hit.at, HitAt::Spread);
    assert_eq!(hit.packet.intensity.get(), 1.0e6);
    assert!(close(
        hit.packet.energy.get(),
        (1.0e6 * rig.body().volume().get()).min(200.0e3)
    ));
}

#[test]
fn a_body_of_one_part_is_destroyed_with_it() {
    let mut rig = Rig::empty();
    rig.spawn(BLOB, "blob", 1.0);
    let mass = rig.bodies.get(BLOB).unwrap().parts()[0].key();
    let cut = rig.kind("cut");
    let hit = |energy: f64, span: f64| Hit {
        target: BLOB,
        at: HitAt::Part(mass),
        packet: DamagePacket::new(
            cut,
            Real::of(energy),
            Real::of(energy / 4.0e-8),
            Cause::DEBUG,
        ),
        area: Real::of(2.0e-5),
        span: Real::of(span),
        dir: None,
    };
    // A stab goes through and leaves it standing: no leak, no pain, and
    // it closes at once.
    let stab = hit(50.0, 0.05);
    let mut rng = Rng::from_seed(3);
    let report =
        body::apply_hit(&mut rig.cx(), &stab, &[], &mut rng, &HitSettings::default()).unwrap();
    assert!(!report.destroyed);
    assert_eq!(report.exits.len(), 1);
    let body = rig.bodies.get(BLOB).unwrap();
    assert_eq!(body.wounds().len(), 1);
    assert_eq!(body.wounds()[0].state(), WoundState::Closed);
    assert!(body.wounds()[0].leaks().is_empty());
    assert!(
        report
            .changes
            .iter()
            .any(|change| matches!(change, Change::WoundClosed { .. }))
    );
    // Jelly mends in minutes.
    let step = rig
        .defs
        .tissue(rig.material("jelly"))
        .unwrap()
        .heal_step
        .unwrap();
    let damage = body.parts()[0].layers()[0].damage();
    let seen = rig.run_until(Tick(i64::from(damage) * step.as_micros()));
    assert!(
        seen.iter()
            .any(|(_, change)| matches!(change, Change::WoundHealed { .. }))
    );
    assert_eq!(
        rig.bodies.get(BLOB).unwrap().parts()[0].layers()[0].damage(),
        0
    );

    // An edge all the way across cannot take the root off: it ruins it,
    // and the body is in pieces.
    let cleave = hit(500.0, 0.9);
    let report = body::apply_hit(
        &mut rig.cx(),
        &cleave,
        &[],
        &mut rng,
        &HitSettings::default(),
    )
    .unwrap();
    assert!(report.destroyed);
    assert!(report.detached.is_empty());
    assert!(
        report
            .changes
            .contains(&Change::BodyDestroyed { entity: BLOB })
    );
    let body = rig.bodies.get(BLOB).unwrap();
    assert_eq!(body.parts()[0].state(), PartState::Destroyed);
    assert_eq!(body.mass(), Real::ZERO);
    rig.check_invariants();
}

#[test]
fn saving_and_loading_in_the_middle_changes_nothing() {
    let mut straight = Rig::new();
    let cut = straight.cut("l_arm", 30.0);
    let stab = straight.hit_at("torso", "pierce", 500.0, 5.0e-5, 0.005, 0.02);
    straight.apply(&cut);
    straight.run_until(seconds(300));

    // Save everything, and load it into a second run.
    let mut loaded = straight.clone();
    loaded.bodies = codec::from_bytes(&codec::to_bytes(&straight.bodies).unwrap()).unwrap();
    loaded.stats = codec::from_bytes(&codec::to_bytes(&straight.stats).unwrap()).unwrap();
    loaded.physical = codec::from_bytes(&codec::to_bytes(&straight.physical).unwrap()).unwrap();
    loaded.timeline = codec::from_bytes(&codec::to_bytes(&straight.timeline).unwrap()).unwrap();
    assert_eq!(loaded.hash(), straight.hash());
    assert_eq!(loaded.bodies, straight.bodies);

    let finish = |rig: &mut Rig| {
        let mut seen = Vec::new();
        let report = rig.apply(&stab);
        seen.extend(rig.run_until(seconds(2000)));
        let wounds: Vec<_> = rig.body().wounds().iter().map(|wound| wound.id()).collect();
        for wound in wounds {
            if rig.body().wound(wound).unwrap().state() == WoundState::Open {
                body::close_wound(&mut rig.cx(), HERO, wound).unwrap();
            }
        }
        seen.extend(rig.run_until(seconds(400_000)));
        (report, seen, rig.hash())
    };
    let a = finish(&mut straight);
    let b = finish(&mut loaded);
    assert_eq!(a.0, b.0);
    assert_eq!(a.1, b.1);
    assert_eq!(a.2, b.2);
    assert!(!a.1.is_empty());
}

// ---- Refusals and properties ------------------------------------------------

#[test]
fn every_refusal_carries_a_reason() {
    let mut rig = Rig::new();
    let stranger = EntityId::new(77, 0);
    let plan = rig.plan("humanoid");
    let l_arm = rig.key("l_arm");
    let code = |result: Result<Changes, BodyError>| {
        let error = result.unwrap_err();
        assert!(!error.to_string().is_empty());
        error.code()
    };
    assert_eq!(
        code(body::attach(
            &mut rig.cx(),
            HERO,
            plan,
            Real::ONE,
            ClockId::UNIVERSAL
        )),
        reason::HAS_BODY
    );
    assert_eq!(
        code(body::attach(
            &mut rig.cx(),
            stranger,
            plan,
            Real::ZERO,
            ClockId::UNIVERSAL
        )),
        reason::BAD_AMOUNT
    );
    assert_eq!(
        code(body::attach(
            &mut rig.cx(),
            stranger,
            body::BodyPlanId::new(999),
            Real::ONE,
            ClockId::UNIVERSAL
        )),
        reason::UNKNOWN_PLAN
    );
    assert!(!rig.bodies.contains(stranger));
    assert_eq!(
        code(body::disable(&mut rig.cx(), stranger, l_arm)),
        reason::NO_BODY
    );
    assert_eq!(
        code(body::disable(&mut rig.cx(), HERO, PartKey::new(500))),
        reason::NO_SUCH_PART
    );
    assert_eq!(
        code(body::close_wound(
            &mut rig.cx(),
            HERO,
            body::WoundId::new(9)
        )),
        reason::NO_SUCH_WOUND
    );
    assert_eq!(
        code(body::set_height_factor(&mut rig.cx(), HERO, Real::of(-1.0))),
        reason::BAD_AMOUNT
    );
    let mut hit = rig.cut("l_arm", 10.0);
    hit.packet.energy = Real::of(-1.0);
    let mut rng = Rng::from_seed(1);
    let before = rig.hash();
    let refused = body::apply_hit(&mut rig.cx(), &hit, &[], &mut rng, &HitSettings::default());
    let error = refused.unwrap_err();
    assert_eq!(error.code(), reason::BAD_HIT);
    assert_eq!(error.values(), [("what", Value::text("energy"))]);
    hit.packet.energy = Real::of(10.0);
    hit.target = stranger;
    let refused = body::apply_hit(&mut rig.cx(), &hit, &[], &mut rng, &HitSettings::default());
    assert_eq!(refused.unwrap_err().code(), reason::NO_BODY);
    assert_eq!(rig.hash(), before);

    // A posture that halves the height halves every band with it.
    body::set_height_factor(&mut rig.cx(), HERO, Real::of(0.5)).unwrap();
    assert!(close(rig.body().height().get(), 0.875));
    assert_eq!(
        rig.body()
            .parts_at_height(&rig.defs, Real::of(0.82))
            .next()
            .unwrap()
            .name(),
        "head"
    );
    rig.check_invariants();
}

#[test]
fn bodies_are_read_through_registered_properties() {
    let mut rig = Rig::new();
    let cut = rig.cut("l_arm", 30.0);
    rig.apply(&cut);
    let defs = Arc::clone(&rig.defs);
    let registry = defs.defs().expr_registry();
    let kinds = defs.kinds();
    let entity = Handle::new(kinds.entity, HERO.to_bits());
    let read = |rig: &Rig, kind, name: &str, subject: Handle, args: &[Value]| {
        let property = registry
            .property_id(kind, name)
            .unwrap_or_else(|| panic!("no property `{name}`"));
        assert!(defs.answers(property));
        body::property(&rig.bodies, &defs, &rig.timeline, property, subject, args).unwrap()
    };

    // `self.fn.walking`, under both names and by id.
    let Value::Subject(functions) = read(&rig, kinds.entity, "fn", entity, &[]) else {
        panic!("`fn` gives a subject");
    };
    assert_eq!(functions.kind, kinds.functions);
    assert_eq!(
        read(&rig, kinds.functions, "walking", functions, &[]),
        Value::Real(Real::ONE)
    );
    assert_eq!(
        read(&rig, kinds.functions, "bodyfix:grasp", functions, &[]),
        Value::Real(Real::ONE)
    );
    assert_eq!(
        read(
            &rig,
            kinds.functions,
            "value",
            functions,
            &[Value::text("bodyfix:sight")]
        ),
        Value::Real(Real::ONE)
    );

    // A part, by name.
    let Value::Subject(arm) = read(&rig, kinds.entity, "part", entity, &[Value::text("l_arm")])
    else {
        panic!("`part` gives a subject");
    };
    assert_eq!(
        props::part_of_handle(&rig.bodies, arm),
        Some((HERO, rig.key("l_arm")))
    );
    assert_eq!(
        read(&rig, kinds.part, "name", arm, &[]),
        Value::text("l_arm")
    );
    assert_eq!(
        read(&rig, kinds.part, "state", arm, &[]),
        Value::text("disabled")
    );
    assert_eq!(
        read(&rig, kinds.part, "working", arm, &[]),
        Value::Bool(false)
    );
    assert_eq!(
        read(&rig, kinds.part, "exposed", arm, &[]),
        Value::Bool(true)
    );
    assert_eq!(read(&rig, kinds.part, "layers", arm, &[]), Value::Int(3));
    assert_eq!(
        read(&rig, kinds.part, "layer_damage", arm, &[Value::Int(0)]),
        Value::Real(Real::ONE)
    );
    assert_eq!(
        read(&rig, kinds.part, "layer_damage", arm, &[Value::Int(7)]),
        Value::Null
    );
    assert_eq!(
        read(&rig, kinds.part, "has_tag", arm, &[Value::text("limb")]),
        Value::Bool(true)
    );
    assert_eq!(
        read(&rig, kinds.part, "side", arm, &[]),
        Value::text("left")
    );
    let Value::Real(condition) = read(&rig, kinds.part, "condition", arm, &[]) else {
        panic!("a condition is a number");
    };
    assert!(condition.get() < 0.5);
    assert_eq!(
        read(&rig, kinds.entity, "part", entity, &[Value::text("tail")]),
        Value::Null
    );
    let Value::List(parts) = read(&rig, kinds.entity, "parts", entity, &[]) else {
        panic!("`parts` is a list");
    };
    assert_eq!(parts.len(), 16);

    // Its wound.
    let Value::List(wounds) = read(&rig, kinds.part, "wounds", arm, &[]) else {
        panic!("`wounds` is a list");
    };
    let Value::Subject(wound) = wounds[0] else {
        panic!("a wound is a subject");
    };
    assert_eq!(read(&rig, kinds.wound, "depth", wound, &[]), Value::Int(2));
    assert_eq!(
        read(&rig, kinds.wound, "state", wound, &[]),
        Value::text("open")
    );
    assert_eq!(
        read(&rig, kinds.wound, "part", wound, &[]),
        Value::Subject(arm)
    );
    assert_eq!(
        read(&rig, kinds.wound, "stump", wound, &[]),
        Value::Bool(false)
    );
    rig.run_until(seconds(100));
    assert_eq!(
        read(&rig, kinds.wound, "age", wound, &[]),
        Value::Real(Real::of(100.0))
    );

    // Nothing to read is null, not a mistake.
    let nobody = Handle::new(kinds.entity, EntityId::new(500, 0).to_bits());
    assert_eq!(read(&rig, kinds.entity, "scale", nobody, &[]), Value::Null);
    assert_eq!(
        read(&rig, kinds.entity, "scale", entity, &[]),
        Value::Real(Real::ONE)
    );
    // A handle from before its owner's slot was given to someone else
    // reads nothing of the newcomer.
    let body = rig.bodies.remove(HERO).unwrap();
    rig.bodies.insert(HERO.next_generation(), body);
    assert_eq!(read(&rig, kinds.part, "name", arm, &[]), Value::Null);
    assert_eq!(read(&rig, kinds.wound, "depth", wound, &[]), Value::Null);

    // None of it is safe for belief, and none of it can be set.
    for kind in [kinds.functions, kinds.part, kinds.wound] {
        for property in registry.properties_of(kind) {
            assert!(!property.spec.belief_safe, "{}", property.spec.name);
            assert!(!property.spec.settable, "{}", property.spec.name);
            assert!(!property.spec.doc.is_empty(), "{}", property.spec.name);
        }
    }
    for name in ["fn", "scale", "reach", "part", "parts", "wounds"] {
        let property = registry.property_id(kinds.entity, name).unwrap();
        let spec = &registry.property(property).unwrap().spec;
        assert!(!spec.belief_safe && !spec.settable);
    }

    // The events, and the hook with what a handler may change.
    let hook = &registry
        .event(registry.event_id(props::HIT_INCOMING).unwrap())
        .unwrap()
        .spec;
    assert!(hook.hook);
    assert_eq!(hook.mutable, ["energy", "intensity", "part", "dir"]);
    for name in [
        props::HIT_LANDED,
        props::WOUND_INFLICTED,
        props::WOUND_CLOSED,
        props::WOUND_REOPENED,
        props::WOUND_HEALED,
        props::PART_DISABLED,
        props::PART_RESTORED,
        props::PART_SEVERED,
        props::PART_DESTROYED,
        props::FUNCTION_CHANGED,
    ] {
        let event = &registry
            .event(registry.event_id(name).unwrap())
            .unwrap()
            .spec;
        assert!(!event.hook && !event.doc.is_empty(), "{name}");
        assert!(event.fields.iter().all(|field| !field.doc.is_empty()));
    }
}

// ---- Random histories -------------------------------------------------------

/// One thing done to the hero.
#[derive(Clone, Debug)]
enum Op {
    Hit {
        part: usize,
        kind: usize,
        energy: f64,
        area: f64,
        span: f64,
    },
    Unaimed {
        kind: usize,
        energy: f64,
        spread: bool,
    },
    Disable(usize),
    Restore(usize),
    Detach(usize, bool),
    Heal(usize, f64),
    Close(usize),
    Reopen(usize),
    Wait(i64),
    Away(i64),
}

fn op() -> impl Strategy<Value = Op> {
    let energy = prop_oneof![1.0..100.0_f64, 100.0..5000.0_f64, 5.0e3..5.0e5_f64];
    let area = prop_oneof![Just(2.0e-5), Just(5.0e-5), Just(5.0e-3), Just(5.0e-2)];
    let span = prop_oneof![Just(0.9), Just(0.03), Just(0.08)];
    prop_oneof![
        6 => (0..16_usize, 0..4_usize, energy.clone(), area, span).prop_map(
            |(part, kind, energy, area, span)| Op::Hit { part, kind, energy, area, span }
        ),
        2 => (0..4_usize, energy, any::<bool>())
            .prop_map(|(kind, energy, spread)| Op::Unaimed { kind, energy, spread }),
        1 => (0..16_usize).prop_map(Op::Disable),
        1 => (0..16_usize).prop_map(Op::Restore),
        1 => (0..16_usize, any::<bool>()).prop_map(|(part, removed)| Op::Detach(part, removed)),
        1 => (0..16_usize, 0.01..1.0_f64).prop_map(|(part, share)| Op::Heal(part, share)),
        1 => (0..4_usize).prop_map(Op::Close),
        1 => (0..4_usize).prop_map(Op::Reopen),
        2 => (1..200_000_i64).prop_map(Op::Wait),
        1 => (0..100_000_i64).prop_map(Op::Away),
    ]
}

/// Does one thing to the hero. A refusal is fine; a broken invariant is
/// not.
fn perform(rig: &mut Rig, op: &Op) {
    let kinds = ["impact", "cut", "pierce", "blast"];
    let part = |_rig: &Rig, index: usize| PartKey::new(index as u16);
    let wound = |rig: &Rig, index: usize| rig.body().wounds().get(index).map(|wound| wound.id());
    match op {
        Op::Hit {
            part: index,
            kind,
            energy,
            area,
            span,
        } => {
            let key = part(rig, *index);
            let hit = Hit {
                target: HERO,
                at: HitAt::Part(key),
                packet: DamagePacket::new(
                    rig.kind(kinds[*kind]),
                    Real::of(*energy),
                    Real::of(energy / (area * 0.005)),
                    Cause::DEBUG,
                ),
                area: Real::of(*area),
                span: Real::of(*span),
                dir: None,
            };
            let mut rng = rig.rng.clone();
            if let Ok(report) =
                body::apply_hit(&mut rig.cx(), &hit, &[], &mut rng, &HitSettings::default())
            {
                assert!(close(accounted(&report), *energy), "{report:?}");
            }
            rig.rng = rng;
        }
        Op::Unaimed {
            kind,
            energy,
            spread,
        } => {
            let hit = Hit {
                target: HERO,
                at: if *spread { HitAt::Spread } else { HitAt::Any },
                packet: DamagePacket::new(
                    rig.kind(kinds[*kind]),
                    Real::of(*energy),
                    Real::of(energy / 1.0e-3),
                    Cause::DEBUG,
                ),
                area: Real::of(1.0e-2),
                span: Real::of(0.1),
                dir: None,
            };
            let report = rig.apply(&hit);
            assert!(close(accounted(&report), *energy), "{report:?}");
        }
        Op::Disable(index) => {
            let key = part(rig, *index);
            let _ = body::disable(&mut rig.cx(), HERO, key);
        }
        Op::Restore(index) => {
            let key = part(rig, *index);
            let _ = body::restore(&mut rig.cx(), HERO, key);
        }
        Op::Detach(index, removed) => {
            let key = part(rig, *index);
            let how = if *removed {
                Detach::Removed
            } else {
                Detach::Severed
            };
            let _ = body::detach(&mut rig.cx(), HERO, key, how, Cause::DEBUG);
        }
        Op::Heal(index, share) => {
            let key = part(rig, *index);
            let _ = body::heal_part(&mut rig.cx(), HERO, key, Real::of(*share));
        }
        Op::Close(index) => {
            if let Some(id) = wound(rig, *index) {
                let _ = body::close_wound(&mut rig.cx(), HERO, id);
            }
        }
        Op::Reopen(index) => {
            if let Some(id) = wound(rig, *index) {
                let _ = body::reopen_wound(&mut rig.cx(), HERO, id);
            }
        }
        Op::Wait(secs) => {
            let until = Tick(rig.timeline.now().0 + Dur::from_secs(*secs).as_micros());
            rig.run_until(until);
        }
        Op::Away(secs) => {
            body::suspend(&mut rig.cx(), HERO).unwrap();
            assert_eq!(rig.body().wake_id(), None);
            let until = Tick(rig.timeline.now().0 + Dur::from_secs(*secs).as_micros());
            rig.run_until(until);
            body::resume(
                &mut rig.cx(),
                HERO,
                ClockId::UNIVERSAL,
                Dur::from_secs(*secs),
            )
            .unwrap();
        }
    }
    rig.check_invariants();
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(48))]

    /// Whatever is done to a body, what is cached of it equals a
    /// recomputation, its slots are those of its whole parts, and its
    /// mass is what `Physical` holds (invariants B2 and B3). Saved and
    /// loaded half way, the rest of the history comes out the same.
    #[test]
    fn derived_values_equal_recomputation_under_random_changes(
        ops in proptest::collection::vec(op(), 1..40),
        seed in any::<u64>(),
    ) {
        let mut rig = Rig::new();
        rig.rng = Rng::from_seed(seed);
        let half = ops.len() / 2;
        for op in &ops[..half] {
            perform(&mut rig, op);
        }
        let mut loaded = rig.clone();
        loaded.bodies = codec::from_bytes(&codec::to_bytes(&rig.bodies).unwrap()).unwrap();
        loaded.stats = codec::from_bytes(&codec::to_bytes(&rig.stats).unwrap()).unwrap();
        loaded.physical = codec::from_bytes(&codec::to_bytes(&rig.physical).unwrap()).unwrap();
        loaded.timeline = codec::from_bytes(&codec::to_bytes(&rig.timeline).unwrap()).unwrap();
        for op in &ops[half..] {
            perform(&mut rig, op);
            perform(&mut loaded, op);
        }
        prop_assert_eq!(rig.hash(), loaded.hash());
        // What the stats hold of leaks is what the open wounds want.
        let leaking = rig
            .body()
            .wounds()
            .iter()
            .filter(|wound| !wound.leaks().is_empty())
            .count();
        let blood = rig.pool("blood");
        prop_assert_eq!(rig.stats.get(HERO).unwrap().pool_rates(blood).len(), leaking);
    }
}
