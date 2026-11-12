//! Tests of body editing (06 §6.5; package E6): the body model's edits
//! against its own stores, with random sequences checked against
//! invariants B2 and B3, and the simulation's verbs around them, the
//! reshaping touch among them. Bodies are those of
//! `tests/fixtures/reshape`.

mod common;

use std::path::{Path, PathBuf};
use std::sync::{Arc, LazyLock};

use clash_core::{EntityId, Hash128, Real, Rng};
use clash_data::{Defs, Loader, Runner};
use clash_expr::{FunctionId, Handle, Host, PropertyId, RuntimeWarning, Value};
use clash_sim::body::edit::{self, Attach, EditError};
use clash_sim::body::{
    self, Body, BodyCx, BodyDefs, BodyPartId, Detach, Detached, PartKey, PartState, Relation,
    recompute,
};
use clash_sim::component::{Containment, Physical};
use clash_sim::item::{self, ItemDef};
use clash_sim::reshape;
use clash_sim::rule::{EocDef, RunView, VerbCall};
use clash_sim::stats::{self, StatDefs, StatsCx};
use clash_sim::{Delivery, Handler, Place, Reason, Sim, SimEvent, Subject};
use clash_time::{ClockId, Timeline};
use clash_world::Cause;
use clash_world::defs::{MaterialDef, MaterialId};
use common::{creature, on, sim_over, spawn};
use proptest::prelude::*;

// ---- Loading ----------------------------------------------------------------

fn fixtures(area: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join(format!("tests/fixtures/{area}"))
}

/// A coat for the tests' own: it is worn over both arms and the torso.
const TEST_DEFS: &str = r#"[
  { "type": "item", "id": "coat", "material": "steel", "mass": "1.8 kg", "length": "1 m",
    "volume": 0.004, "grip": { "reshapefix:arm": 1 },
    "wear": { "parts": ["torso", "arm"], "layer": 3, "thickness": "3 mm" } }
]"#;

static DEFS: LazyLock<Arc<Defs>> = LazyLock::new(|| {
    let types = common::types();
    let dir = common::mods_dir().join("core/data/world");
    let mut paths: Vec<PathBuf> = std::fs::read_dir(&dir)
        .unwrap()
        .filter_map(|entry| entry.ok().map(|entry| entry.path()))
        .filter(|path| path.extension().is_some_and(|ext| ext == "json"))
        .collect();
    paths.sort();
    let mut files: Vec<(String, String)> = paths
        .iter()
        .map(|file| {
            let name = file.file_name().unwrap().to_string_lossy();
            (
                format!("data/world/{name}"),
                std::fs::read_to_string(file).unwrap(),
            )
        })
        .collect();
    files.push(("data/test/items.json".to_owned(), TEST_DEFS.to_owned()));
    let files: Vec<(&str, &str)> = files
        .iter()
        .map(|(path, text)| (path.as_str(), text.as_str()))
        .collect();
    let outcome = Loader::new(&types)
        .expr_registry(&clash_sim::engine_registry())
        .memory_mod("core", r#"{ "id": "core" }"#, &files)
        .search_path(fixtures("reshape"))
        .load_mods(&["core", "reshapefix"]);
    // `self.part(name)` may be null, and the fixture rule hands it to
    // verbs as it is: those are warnings, and nothing else may be said.
    let others: Vec<String> = outcome
        .diagnostics
        .iter()
        .filter(|diagnostic| diagnostic.code != "W0644")
        .map(clash_data::Diagnostic::render)
        .collect();
    assert!(others.is_empty(), "{}", others.join("\n"));
    outcome.defs.expect("the fixtures load")
});

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

fn part_def(name: &str) -> BodyPartId {
    TABLES
        .body
        .part_id(&format!("reshapefix:{name}"))
        .unwrap_or_else(|| panic!("no part `{name}`"))
}

fn material(name: &str) -> MaterialId {
    DEFS.lookup::<MaterialDef>(&format!("reshapefix:{name}"))
        .unwrap()
}

fn function(name: &str) -> body::BodyFunctionId {
    TABLES
        .body
        .function_id(&format!("reshapefix:{name}"))
        .unwrap()
}

// ---- Bodies without a simulation --------------------------------------------

struct Apart;

impl Containment for Apart {
    fn container(&self, _id: EntityId) -> Option<EntityId> {
        None
    }

    fn contents(&self, _id: EntityId, _visit: &mut dyn FnMut(EntityId)) {}
}

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

/// The stores a body lives in, apart from any simulation.
#[derive(Default)]
struct Stores {
    bodies: body::BodyStore,
    physical: clash_sim::component::ComponentStore<Physical>,
    stats: stats::StatsStore,
    timeline: Timeline,
}

impl Stores {
    fn run<R>(&mut self, run: impl FnOnce(&mut BodyCx<'_, Quiet>) -> R) -> R {
        let mut runner = Runner::new();
        let mut quiet = Quiet;
        let mut cx = BodyCx {
            bodies: &mut self.bodies,
            defs: &TABLES.body,
            physical: &mut self.physical,
            containment: &Apart,
            stats: StatsCx {
                stats: &mut self.stats,
                defs: &TABLES.stats,
                timeline: &mut self.timeline,
                host: &mut quiet,
                runner: &mut runner,
            },
        };
        run(&mut cx)
    }

    fn body(&self, who: EntityId) -> &Body {
        self.bodies.get(who).unwrap()
    }
}

const WHO: EntityId = EntityId::new(1, 0);

fn humanoid_apart() -> Stores {
    let mut stores = Stores::default();
    let plan = TABLES.body.plan_id("reshapefix:humanoid").unwrap();
    stores.run(|cx| {
        stats::attach(&mut cx.stats, WHO, ClockId::UNIVERSAL).unwrap();
        body::attach(cx, WHO, plan, Real::ONE, ClockId::UNIVERSAL).unwrap();
    });
    stores
}

fn key_of(body: &Body, name: &str) -> PartKey {
    body.part_named(name)
        .unwrap_or_else(|| panic!("no part `{name}`"))
        .key()
}

/// Invariants B2 and B3 and the shape of the tree: what is cached equals
/// a recomputation bit for bit; a part works exactly when it is there,
/// nothing holds it and its condition allows; every part's parent is
/// listed and a part that is there has a parent that is there; names of
/// parts that are there are not repeated; sockets are within their
/// counts and take what is in them; the body's mass and size are what
/// `Physical` holds.
fn check_body(stores: &Stores, who: EntityId) {
    let body = stores.body(who);
    let defs = &TABLES.body;
    assert_eq!(
        Hash128::of(body.derived()),
        Hash128::of(&recompute(body, defs)),
        "B3: the cache equals a recomputation"
    );
    let parts = body.parts();
    for pair in parts.windows(2) {
        assert!(pair[0].key() < pair[1].key(), "parts are in order of key");
    }
    let roots: Vec<_> = parts
        .iter()
        .filter(|part| part.parent().is_none())
        .collect();
    assert_eq!(roots.len(), 1, "one root");
    let mut names: Vec<&str> = Vec::new();
    for part in parts {
        let def = defs.part(part.def()).unwrap();
        let should_work =
            part.is_present() && !part.is_held() && part.condition() >= def.disable_below;
        assert_eq!(part.is_working(), should_work, "B2: `{}`", part.name());
        if let Some(parent) = part.parent() {
            let parent = body
                .part(parent)
                .unwrap_or_else(|| panic!("the parent of `{}` is listed", part.name()));
            if part.is_present() {
                assert!(parent.is_present(), "`{}` hangs off nothing", part.name());
            }
            if part.relation() == Relation::In {
                assert!(part.socket().is_none());
            }
        }
        if part.is_present() {
            assert!(!names.contains(&part.name()), "`{}` twice", part.name());
            names.push(part.name());
            assert!(!part.layers().is_empty());
        } else {
            assert!(part.layers().is_empty());
        }
        // Sockets within their counts.
        for (socket, socket_def) in &def.sockets {
            if !part.is_present() {
                continue;
            }
            let held: Vec<_> = body.in_socket(part.key(), socket).collect();
            assert!(
                held.len() <= usize::from(socket_def.count),
                "`{socket}` overfull"
            );
            for child in held {
                assert!(
                    socket_def.accepts.is_empty()
                        || socket_def.accepts.iter().any(|a| a.id() == child.def()),
                    "`{socket}` holds what it does not take"
                );
            }
        }
    }
    for wound in body.wounds() {
        assert!(
            body.part(wound.part())
                .is_some_and(|part| part.is_present())
        );
    }
    let physical = stores.physical.get(who).unwrap();
    assert_eq!(
        physical.own_mass().get().get().to_bits(),
        body.mass().get().to_bits()
    );
    assert_eq!(
        physical.volume().get().get().to_bits(),
        body.volume().get().to_bits()
    );
}

#[derive(Clone, Debug)]
enum Op {
    Add(usize, usize, bool),
    Remove(usize),
    Sever(usize),
    Replace(usize, usize),
    Graft(usize, bool),
    Layer(usize, Option<usize>, usize, Option<u8>),
    ScalePart(usize, u8),
    ScaleBody(u8),
    Plan(bool),
    Damage(usize, Option<usize>, u8),
    Heal(usize, u8),
    Hold(usize, bool),
}

const PARTS: &[&str] = &["wing", "claw", "hand", "tentacle", "arm", "leg", "sac"];
const MATERIALS: &[&str] = &["skin", "chitin", "bone"];

fn op() -> impl Strategy<Value = Op> {
    let at = 0_usize..64;
    prop_oneof![
        3 => (0..PARTS.len(), at.clone(), any::<bool>()).prop_map(|(def, parent, inside)| Op::Add(def, parent, inside)),
        2 => at.clone().prop_map(Op::Remove),
        2 => at.clone().prop_map(Op::Sever),
        2 => (at.clone(), 0..PARTS.len()).prop_map(|(part, def)| Op::Replace(part, def)),
        2 => (at.clone(), any::<bool>()).prop_map(|(parent, inside)| Op::Graft(parent, inside)),
        2 => (at.clone(), proptest::option::of(0_usize..4), 0..MATERIALS.len(), proptest::option::of(1_u8..40))
            .prop_map(|(part, layer, material, mm)| Op::Layer(part, layer, material, mm)),
        1 => (at.clone(), 1_u8..8).prop_map(|(part, quarters)| Op::ScalePart(part, quarters)),
        1 => (1_u8..8).prop_map(Op::ScaleBody),
        1 => any::<bool>().prop_map(Op::Plan),
        2 => (at.clone(), proptest::option::of(0_usize..4), 1_u8..255).prop_map(|(part, layer, share)| Op::Damage(part, layer, share)),
        1 => (at.clone(), 1_u8..255).prop_map(|(part, share)| Op::Heal(part, share)),
        1 => (at, any::<bool>()).prop_map(|(part, restore)| Op::Hold(part, restore)),
    ]
}

/// A part of the body by position among those that are there.
fn pick(stores: &Stores, at: usize) -> PartKey {
    let present: Vec<PartKey> = stores
        .body(WHO)
        .parts()
        .iter()
        .filter(|part| part.is_present())
        .map(|part| part.key())
        .collect();
    present[at % present.len()]
}

fn real(value: f64) -> Real {
    Real::of(value)
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(64))]

    /// Invariants B2 and B3, and the shape of the tree, hold after any
    /// sequence of edits; and an edit that is refused changes nothing.
    #[test]
    fn edits_keep_the_body_whole(ops in proptest::collection::vec(op(), 1..30)) {
        let mut stores = humanoid_apart();
        let mut pieces: Vec<Detached> = Vec::new();
        check_body(&stores, WHO);
        for op in ops {
            let before = Hash128::of(stores.body(WHO));
            let result: Result<(), EditError> = match op {
                Op::Add(def, parent, inside) => {
                    let at = Attach {
                        parent: pick(&stores, parent),
                        relation: if inside { Relation::In } else { Relation::On },
                        socket: None,
                    };
                    stores
                        .run(|cx| edit::add_part(cx, WHO, part_def(PARTS[def]), &at, None, None, Real::ONE))
                        .map(|_| ())
                }
                Op::Remove(part) => {
                    let key = pick(&stores, part);
                    stores
                        .run(|cx| edit::remove_part(cx, WHO, key, Cause::DEBUG))
                        .map(|(piece, _)| pieces.push(piece))
                }
                Op::Sever(part) => {
                    let key = pick(&stores, part);
                    stores
                        .run(|cx| body::detach(cx, WHO, key, Detach::Severed, Cause::DEBUG))
                        .map(|(piece, _)| pieces.push(piece))
                        .map_err(EditError::from)
                }
                Op::Replace(part, def) => {
                    let key = pick(&stores, part);
                    stores
                        .run(|cx| edit::replace_part(cx, WHO, key, part_def(PARTS[def]), Cause::DEBUG))
                        .map(|(off, _)| pieces.extend(off))
                }
                Op::Graft(parent, inside) => match pieces.pop() {
                    None => Ok(()),
                    Some(piece) => {
                        let at = Attach {
                            parent: pick(&stores, parent),
                            relation: if inside { Relation::In } else { Relation::On },
                            socket: None,
                        };
                        stores.run(|cx| edit::graft_part(cx, WHO, &piece, &at)).map(|_| ())
                    }
                },
                Op::Layer(part, layer, stuff, mm) => {
                    let key = pick(&stores, part);
                    let thickness = mm.map(|mm| real(f64::from(mm) / 1000.0));
                    stores
                        .run(|cx| edit::set_layer(cx, WHO, &[key], layer, Some(material(MATERIALS[stuff])), thickness))
                        .map(|_| ())
                }
                Op::ScalePart(part, quarters) => {
                    let key = pick(&stores, part);
                    stores
                        .run(|cx| edit::scale_part(cx, WHO, key, real(f64::from(quarters) / 4.0)))
                        .map(|_| ())
                }
                Op::ScaleBody(quarters) => stores
                    .run(|cx| edit::scale_body(cx, WHO, real(f64::from(quarters) / 4.0)))
                    .map(|_| ()),
                Op::Plan(crawler) => {
                    let name = if crawler { "reshapefix:crawler" } else { "reshapefix:humanoid" };
                    let plan = TABLES.body.plan_id(name).unwrap();
                    stores
                        .run(|cx| edit::set_body_plan(cx, WHO, plan, Cause::DEBUG))
                        .map(|(off, _)| pieces.extend(off))
                }
                Op::Damage(part, layer, share) => {
                    let key = pick(&stores, part);
                    stores
                        .run(|cx| edit::damage_part(cx, WHO, key, layer, real(f64::from(share) / 255.0)))
                        .map(|_| ())
                }
                Op::Heal(part, share) => {
                    let key = pick(&stores, part);
                    stores
                        .run(|cx| body::heal_part(cx, WHO, key, real(f64::from(share) / 255.0)))
                        .map(|_| ())
                        .map_err(EditError::from)
                }
                Op::Hold(part, restore) => {
                    let key = pick(&stores, part);
                    stores
                        .run(|cx| if restore { body::restore(cx, WHO, key) } else { body::disable(cx, WHO, key) })
                        .map(|_| ())
                        .map_err(EditError::from)
                }
            };
            if result.is_err() {
                prop_assert_eq!(Hash128::of(stores.body(WHO)), before, "a refused edit changed the body: {:?}", op);
            }
            check_body(&stores, WHO);
        }
    }
}

#[test]
fn a_part_added_takes_a_socket_that_takes_it_and_brings_its_organs() {
    let mut stores = humanoid_apart();
    let torso = key_of(stores.body(WHO), "torso");
    let (wing, _) = stores
        .run(|cx| {
            edit::add_part(
                cx,
                WHO,
                part_def("wing"),
                &Attach::on(torso),
                Some("l_wing"),
                None,
                Real::ONE,
            )
        })
        .unwrap();
    let body = stores.body(WHO);
    assert_eq!(body.part(wing).unwrap().socket(), Some("back"));
    // Unnamed, a second wing is numbered.
    let (second, _) = stores
        .run(|cx| {
            edit::add_part(
                cx,
                WHO,
                part_def("wing"),
                &Attach::on(torso),
                None,
                None,
                Real::ONE,
            )
        })
        .unwrap();
    assert_eq!(stores.body(WHO).part(second).unwrap().name(), "wing");
    // A tentacle needs a shoulder or the back, and all are taken.
    let refused = stores.run(|cx| {
        edit::add_part(
            cx,
            WHO,
            part_def("tentacle"),
            &Attach::on(torso),
            None,
            None,
            Real::ONE,
        )
    });
    assert_eq!(refused.unwrap_err().code(), "no_free_socket");
    let refused = stores.run(|cx| {
        edit::add_part(
            cx,
            WHO,
            part_def("leg"),
            &Attach {
                parent: torso,
                relation: Relation::On,
                socket: Some("shoulder".to_owned()),
            },
            None,
            None,
            Real::ONE,
        )
    });
    assert_eq!(refused.unwrap_err().code(), "socket_refuses");
    let refused = stores.run(|cx| {
        edit::add_part(
            cx,
            WHO,
            part_def("wing"),
            &Attach {
                parent: torso,
                relation: Relation::On,
                socket: Some("tail".to_owned()),
            },
            None,
            None,
            Real::ONE,
        )
    });
    assert_eq!(refused.unwrap_err().code(), "no_such_socket");
    let refused = stores.run(|cx| {
        edit::add_part(
            cx,
            WHO,
            part_def("sac"),
            &Attach::inside(torso),
            Some("head"),
            None,
            Real::ONE,
        )
    });
    assert_eq!(refused.unwrap_err().code(), "part_name_taken");
    // A tentacle inside the torso brings its sac.
    let (tentacle, _) = stores
        .run(|cx| {
            edit::add_part(
                cx,
                WHO,
                part_def("tentacle"),
                &Attach::inside(torso),
                Some("tail"),
                None,
                Real::ONE,
            )
        })
        .unwrap();
    let sac = stores.body(WHO).part_named("tail.sac").unwrap();
    assert_eq!(sac.parent(), Some(tentacle));
    assert_eq!(sac.relation(), Relation::In);
    check_body(&stores, WHO);
}

#[test]
fn a_limb_that_was_lost_grows_back_in_its_place() {
    let mut stores = humanoid_apart();
    let arm = key_of(stores.body(WHO), "l_arm");
    stores
        .run(|cx| body::detach(cx, WHO, arm, Detach::Severed, Cause::DEBUG))
        .unwrap();
    let grasping = stores.body(WHO).function(function("grasping"));
    assert_eq!(grasping, real(0.5));
    let torso = key_of(stores.body(WHO), "torso");
    let (regrown, _) = stores
        .run(|cx| {
            edit::add_part(
                cx,
                WHO,
                part_def("arm"),
                &Attach::on(torso),
                Some("l_arm"),
                None,
                Real::ONE,
            )
        })
        .unwrap();
    let body = stores.body(WHO);
    assert_ne!(regrown, arm, "a key is never given again");
    assert!(
        body.part_named("l_hand").is_none(),
        "the lost hand's listing went"
    );
    stores
        .run(|cx| {
            edit::add_part(
                cx,
                WHO,
                part_def("hand"),
                &Attach::on(regrown),
                Some("l_hand"),
                None,
                Real::ONE,
            )
        })
        .unwrap();
    assert_eq!(stores.body(WHO).function(function("grasping")), real(1.0));
    check_body(&stores, WHO);
}

#[test]
fn replacing_a_part_keeps_what_fits_and_drops_what_does_not() {
    let mut stores = humanoid_apart();
    let arm = key_of(stores.body(WHO), "r_arm");
    let hand = key_of(stores.body(WHO), "r_hand");
    stores
        .run(|cx| edit::damage_part(cx, WHO, arm, None, real(0.2)))
        .unwrap();
    let (off, _) = stores
        .run(|cx| edit::replace_part(cx, WHO, arm, part_def("tentacle"), Cause::DEBUG))
        .unwrap();
    assert_eq!(off.len(), 1, "a tentacle has no wrist");
    assert_eq!(off[0].root, hand);
    let body = stores.body(WHO);
    let replaced = body.part(arm).unwrap();
    assert_eq!(replaced.def(), part_def("tentacle"));
    assert_eq!(replaced.name(), "r_arm");
    assert!(
        replaced.layers().iter().all(|layer| layer.damage() == 0),
        "new tissue"
    );
    assert!(
        body.part_named("r_arm.sac").is_some(),
        "its organ came with it"
    );
    assert!(body.part_named("r_hand").is_none());
    // An arm cannot become a leg in a shoulder.
    let refused = stores.run(|cx| edit::replace_part(cx, WHO, arm, part_def("leg"), Cause::DEBUG));
    assert_eq!(refused.unwrap_err().code(), "socket_refuses");
    check_body(&stores, WHO);
}

#[test]
fn a_part_that_came_off_can_be_grafted_back_with_its_damage() {
    let mut stores = humanoid_apart();
    let arm = key_of(stores.body(WHO), "l_arm");
    stores
        .run(|cx| edit::damage_part(cx, WHO, arm, Some(1), real(0.3)))
        .unwrap();
    let damaged: Vec<u8> = stores
        .body(WHO)
        .part(arm)
        .unwrap()
        .layers()
        .iter()
        .map(|layer| layer.damage())
        .collect();
    let (piece, _) = stores
        .run(|cx| edit::remove_part(cx, WHO, arm, Cause::DEBUG))
        .unwrap();
    assert!(stores.body(WHO).part_named("l_arm").is_none());
    let torso = key_of(stores.body(WHO), "torso");
    let shoulder = Attach {
        parent: torso,
        relation: Relation::On,
        socket: Some("shoulder".to_owned()),
    };
    let (grafted, _) = stores
        .run(|cx| edit::graft_part(cx, WHO, &piece, &shoulder))
        .unwrap();
    let body = stores.body(WHO);
    let part = body.part(grafted).unwrap();
    assert_eq!(part.name(), "l_arm");
    assert_eq!(part.socket(), Some("shoulder"));
    let again: Vec<u8> = part.layers().iter().map(|layer| layer.damage()).collect();
    assert_eq!(again, damaged);
    assert_eq!(body.part_named("l_hand").unwrap().parent(), Some(grafted));
    // Grafting the same again would repeat its names.
    let refused = stores.run(|cx| edit::graft_part(cx, WHO, &piece, &Attach::on(torso)));
    assert_eq!(refused.unwrap_err().code(), "part_name_taken");
    // A part already gone beneath one removed goes with it.
    let leg = key_of(stores.body(WHO), "r_leg");
    let arm = key_of(stores.body(WHO), "r_arm");
    let hand = key_of(stores.body(WHO), "r_hand");
    stores
        .run(|cx| body::detach(cx, WHO, hand, Detach::Severed, Cause::DEBUG))
        .unwrap();
    stores
        .run(|cx| edit::remove_part(cx, WHO, arm, Cause::DEBUG))
        .unwrap();
    assert!(stores.body(WHO).part(hand).is_none());
    assert!(stores.body(WHO).part(leg).is_some());
    check_body(&stores, WHO);
}

#[test]
fn scale_goes_with_size_and_mass_with_its_cube() {
    let mut stores = humanoid_apart();
    let mass = stores.body(WHO).mass();
    let arm = key_of(stores.body(WHO), "l_arm");
    let skin = stores.body(WHO).part(arm).unwrap().layers()[0].thickness();
    stores
        .run(|cx| edit::scale_body(cx, WHO, real(2.0)))
        .unwrap();
    assert_eq!(stores.body(WHO).mass(), mass * real(8.0));
    let thicker = stores.body(WHO).part(arm).unwrap().layers()[0].thickness();
    assert_eq!(thicker, skin * real(2.0));
    let arm_mass = |stores: &Stores| {
        let body = stores.body(WHO);
        body.part_mass(&TABLES.body, body.part(arm).unwrap())
    };
    let before = arm_mass(&stores);
    stores
        .run(|cx| edit::scale_part(cx, WHO, arm, real(0.5)))
        .unwrap();
    assert_eq!(arm_mass(&stores), before * real(0.125));
    check_body(&stores, WHO);
}

#[test]
fn a_new_plan_keeps_what_it_names_and_lets_the_rest_go() {
    let mut stores = humanoid_apart();
    let before = stores.body(WHO).clone();
    let l_arm = key_of(&before, "l_arm");
    stores
        .run(|cx| edit::damage_part(cx, WHO, l_arm, Some(0), real(0.1)))
        .unwrap();
    let plan = TABLES.body.plan_id("reshapefix:crawler").unwrap();
    let (off, _) = stores
        .run(|cx| edit::set_body_plan(cx, WHO, plan, Cause::DEBUG))
        .unwrap();
    let body = stores.body(WHO);
    assert_eq!(body.plan(), plan);
    assert_eq!(key_of(body, "l_arm"), l_arm, "kept parts keep their keys");
    assert!(
        body.part(l_arm).unwrap().layers()[0].damage() > 0,
        "and their damage"
    );
    assert_eq!(body.part(l_arm).unwrap().scale(), real(1.2));
    assert_eq!(
        body.part_named("r_arm").unwrap().def(),
        part_def("tentacle")
    );
    assert!(body.part_named("l_claw").is_some());
    assert!(body.part_named("l_hand").is_none());
    // Off came the left hand, and the old right arm with its hand.
    let mut roots: Vec<String> = off
        .iter()
        .map(|piece| {
            piece
                .parts
                .iter()
                .find(|part| part.key() == piece.root)
                .unwrap()
                .name()
                .to_owned()
        })
        .collect();
    roots.sort();
    assert_eq!(roots, ["l_hand", "r_arm"]);
    assert!(close(body.height().get(), 0.9));
    check_body(&stores, WHO);
}

fn close(a: f64, b: f64) -> bool {
    (a - b).abs() <= 1e-9 * a.abs().max(b.abs()).max(1.0)
}

#[test]
fn damage_part_never_takes_a_layer_away() {
    let mut stores = humanoid_apart();
    let head = key_of(stores.body(WHO), "head");
    stores
        .run(|cx| edit::damage_part(cx, WHO, head, None, real(5.0)))
        .unwrap();
    let body = stores.body(WHO);
    let part = body.part(head).unwrap();
    assert!(part.is_present());
    assert!(part.layers().iter().all(|layer| layer.damage() == 254));
    assert_eq!(part.state(), PartState::Disabled);
    let refused = stores.run(|cx| edit::damage_part(cx, WHO, head, Some(7), real(0.1)));
    assert_eq!(refused.unwrap_err().code(), "no_such_layer");
}

// ---- The simulation's verbs -------------------------------------------------

fn sim() -> Sim {
    sim_over(Arc::clone(&DEFS), 9)
}

/// A person standing at a cell, with stats and the fixtures' humanoid
/// body, given them by the simulation.
fn person(sim: &mut Sim, x: i32, y: i32) -> EntityId {
    let id = spawn(sim, creature(), on(x, y));
    let plan = TABLES.body.plan_id("reshapefix:humanoid").unwrap();
    sim.attach_stats(id, Cause::DEBUG).unwrap();
    sim.attach_body(id, plan, Real::ONE, Cause::DEBUG).unwrap();
    id
}

fn body_of(sim: &Sim, who: EntityId) -> &Body {
    sim.components().body.get(who).unwrap()
}

fn events(sim: &mut Sim) -> Vec<SimEvent> {
    sim.pump();
    sim.drain_events()
}

fn named<'a>(events: &'a [SimEvent], name: &str) -> Vec<&'a SimEvent> {
    events.iter().filter(|event| event.name() == name).collect()
}

fn text_field(event: &SimEvent, name: &str) -> String {
    event
        .body
        .field(name)
        .and_then(|value| {
            value
                .plain()
                .and_then(|plain| plain.as_str().map(str::to_owned))
        })
        .unwrap_or_default()
}

fn part_value(sim: &Sim, who: EntityId, name: &str) -> Value {
    let key = reshape::part_named(sim, who, name).unwrap();
    let kinds = sim.body_defs().unwrap().kinds();
    Value::Subject(clash_sim::body::props::part_handle(kinds, who, key))
}

fn call(sim: &mut Sim, name: &str, args: &[Value]) -> Result<Value, Reason> {
    let registry = Arc::clone(sim.defs().expr_registry());
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

fn check_sim_body(sim: &Sim, who: EntityId) {
    let body = body_of(sim, who);
    assert_eq!(
        Hash128::of(body.derived()),
        Hash128::of(&recompute(body, &TABLES.body))
    );
    let physical = sim.components().physical.get(who).unwrap();
    assert_eq!(physical.own_mass().get(), body.mass());
    // The world's record of it weighs what it weighs.
    let record = sim.truth().entity(who).unwrap();
    assert_eq!(record.presence().mass, physical.mass().get());
}

fn items_at(sim: &Sim) -> Vec<EntityId> {
    sim.truth()
        .entities()
        .iter()
        .map(|entity| entity.id())
        .filter(|id| sim.components().item.contains(*id))
        .collect()
}

/// The reshaping touch of 08 §9.2, test script `reshaping_touch`: the
/// fixture rule `apply_form`, run against the simulation, takes a hand
/// away, grows a wing, turns an arm into a tentacle and the torso's skin
/// to chitin.
#[test]
fn reshaping_touch() {
    let mut sim = sim();
    let target = person(&mut sim, 4, 4);
    let grasping = |sim: &Sim| body_of(sim, target).function(function("grasping"));
    assert_eq!(grasping(&sim), real(1.0));
    let mass = body_of(&sim, target).mass();
    events(&mut sim);

    let rule = DEFS.lookup::<EocDef>("reshapefix:apply_form").unwrap();
    let me = sim.subject_value(Subject::Entity(target));
    sim.call_rule(rule, &[], &[("who", me)], Cause::DEBUG)
        .unwrap();

    let body = body_of(&sim, target);
    assert!(body.part_named("l_hand").is_none());
    assert!(body.part_named("l_wing").is_some());
    assert_eq!(
        body.part_named("r_arm").unwrap().def(),
        part_def("tentacle")
    );
    assert!(body.part_named("r_arm.sac").is_some());
    assert!(
        body.part_named("r_hand").is_none(),
        "a tentacle has no wrist"
    );
    let torso = body.part_named("torso").unwrap();
    assert_eq!(torso.layers()[0].material(), material("chitin"));
    assert_eq!(grasping(&sim), real(0.3), "only the tentacle grasps");
    assert_ne!(body.mass(), mass);
    check_sim_body(&sim, target);

    // Both hands lie where the target stands.
    let items = items_at(&sim);
    assert_eq!(items.len(), 2);
    for item in items {
        assert_eq!(sim.truth().place(item), Some(on(4, 4)));
    }
    let seen = events(&mut sim);
    let changes: Vec<String> = named(&seen, "body_changed")
        .iter()
        .map(|event| text_field(event, "verb"))
        .collect();
    assert_eq!(changes, ["add_part", "replace_part", "set_layer"]);
    assert_eq!(named(&seen, "part_detached").len(), 2);
    assert!(!named(&seen, "function_changed").is_empty());
}

#[test]
fn the_verbs_take_parts_and_give_parts() {
    let mut sim = sim();
    let target = person(&mut sim, 4, 4);
    let torso = part_value(&sim, target, "torso");
    let wing = call(
        &mut sim,
        "add_part",
        &[
            torso,
            Value::text("reshapefix:wing"),
            Value::Null,
            Value::Null,
            Value::text("l_wing"),
            Value::text("left"),
            Value::Real(real(0.5)),
        ],
    )
    .unwrap();
    assert_eq!(wing, part_value(&sim, target, "l_wing"));
    assert_eq!(
        body_of(&sim, target).part_named("l_wing").unwrap().socket(),
        Some("back")
    );
    call(
        &mut sim,
        "scale_part",
        &[wing.clone(), Value::Real(real(2.0))],
    )
    .unwrap();
    call(
        &mut sim,
        "damage_part",
        &[wing.clone(), Value::Int(0), Value::Real(real(0.1))],
    )
    .unwrap();
    call(
        &mut sim,
        "set_layer",
        &[
            wing,
            Value::Null,
            Value::text("reshapefix:chitin"),
            Value::Real(real(0.004)),
        ],
    )
    .unwrap();
    let me = sim.subject_value(Subject::Entity(target));
    call(
        &mut sim,
        "scale_body",
        &[me.clone(), Value::Real(real(1.5))],
    )
    .unwrap();
    call(
        &mut sim,
        "set_body_plan",
        &[me, Value::text("reshapefix:crawler")],
    )
    .unwrap();
    assert_eq!(
        body_of(&sim, target).plan(),
        TABLES.body.plan_id("reshapefix:crawler").unwrap()
    );
    check_sim_body(&sim, target);
}

#[test]
fn a_body_that_cannot_be_reshaped_refuses_and_changes_nothing() {
    let mut sim = sim();
    let target = person(&mut sim, 4, 4);
    sim.on(
        "body_changing",
        Handler::new(|_sim: &mut Sim, delivery: &mut Delivery<'_>| {
            let _ = delivery.cancel(Reason::new("test.unchanging", "it keeps its shape"));
        }),
    )
    .unwrap();
    let before = Hash128::of(body_of(&sim, target));
    let arm = reshape::part_named(&sim, target, "l_arm").unwrap();
    let refused = reshape::remove_part(&mut sim, target, arm, Cause::DEBUG);
    assert_eq!(refused.unwrap_err().code, "test.unchanging");
    assert_eq!(Hash128::of(body_of(&sim, target)), before);
}

#[test]
fn refusals_carry_reasons() {
    let mut sim = sim();
    let target = person(&mut sim, 4, 4);
    let torso = part_value(&sim, target, "torso");
    let head = part_value(&sim, target, "head");
    let refused = call(
        &mut sim,
        "add_part",
        &[
            torso,
            Value::text("reshapefix:head"),
            Value::text("neck"),
            Value::Null,
            Value::Null,
            Value::Null,
            Value::Null,
        ],
    );
    assert_eq!(refused.unwrap_err().code, "no_free_socket");
    let refused = call(
        &mut sim,
        "damage_part",
        &[head, Value::Int(9), Value::Real(real(0.5))],
    );
    assert_eq!(refused.unwrap_err().code, "no_such_layer");
    let me = sim.subject_value(Subject::Entity(target));
    let refused = call(
        &mut sim,
        "scale_body",
        &[me.clone(), Value::Real(real(0.0))],
    );
    assert_eq!(refused.unwrap_err().code, "bad_amount");
    let refused = call(
        &mut sim,
        "set_body_plan",
        &[me, Value::text("reshapefix:nothing")],
    );
    assert_eq!(refused.unwrap_err().code, "verb.bad_argument");
    let refused = call(
        &mut sim,
        "scale_part",
        &[Value::Null, Value::Real(real(1.0))],
    );
    assert_eq!(refused.unwrap_err().code, "verb.bad_argument");
    let torso = reshape::part_named(&sim, target, "torso").unwrap();
    let refused = reshape::remove_part(&mut sim, target, torso, Cause::DEBUG);
    assert_eq!(refused.unwrap_err().code, "root_part");
}

#[test]
fn what_was_worn_on_a_part_that_is_gone_comes_off_it() {
    let mut sim = sim();
    let target = person(&mut sim, 4, 4);
    let coat_def = DEFS.lookup::<ItemDef>("core:coat").unwrap();
    let coat = item::create(&mut sim, coat_def, 1, on(4, 4), Cause::DEBUG).unwrap();
    item::wear(&mut sim, target, coat, Cause::DEBUG).unwrap();
    let covers = |sim: &Sim| match item::hold_of(sim, coat) {
        Some(item::Hold::Worn(on)) => on.len(),
        other => panic!("{other:?}"),
    };
    assert_eq!(covers(&sim), 3);
    let arm = reshape::part_named(&sim, target, "r_arm").unwrap();
    reshape::remove_part(&mut sim, target, arm, Cause::DEBUG).unwrap();
    assert_eq!(covers(&sim), 2, "it covers what is left");
    assert_eq!(sim.truth().place(coat), Some(Place::In(target)));
}

#[test]
fn a_rebuilt_body_hands_its_new_mass_to_the_world() {
    let mut sim = sim();
    let target = person(&mut sim, 4, 4);
    let plan = TABLES.body.plan_id("reshapefix:crawler").unwrap();
    let items = reshape::set_body_plan(&mut sim, target, plan, Cause::DEBUG).unwrap();
    assert_eq!(items.len(), 2);
    check_sim_body(&sim, target);
    let arm = reshape::part_named(&sim, target, "l_arm").unwrap();
    reshape::scale_part(&mut sim, target, arm, real(2.0), Cause::DEBUG).unwrap();
    let head = reshape::part_named(&sim, target, "head").unwrap();
    reshape::damage_part(&mut sim, target, head, None, real(0.4), Cause::DEBUG).unwrap();
    sim.heal_part(target, head, real(1.0), Cause::DEBUG)
        .unwrap();
    assert!(
        body_of(&sim, target)
            .part(head)
            .unwrap()
            .layers()
            .iter()
            .all(|layer| layer.damage() == 0)
    );
    check_sim_body(&sim, target);
}

#[test]
fn a_limb_left_as_an_item_can_be_grafted_back_but_not_onto_another_with_its_names() {
    let mut sim = sim();
    let first = person(&mut sim, 4, 4);
    let second = person(&mut sim, 6, 6);
    let arm = reshape::part_named(&sim, first, "l_arm").unwrap();
    let limb = reshape::remove_part(&mut sim, first, arm, Cause::DEBUG)
        .unwrap()
        .unwrap();
    // The second has a left arm of its own: the names would repeat.
    let torso = reshape::part_named(&sim, second, "torso").unwrap();
    let refused = reshape::graft_part(&mut sim, second, limb, &Attach::on(torso), Cause::DEBUG);
    assert_eq!(refused.unwrap_err().code, "part_name_taken");
    // Back where it came from it fits, and the item is used up.
    let torso = reshape::part_named(&sim, first, "torso").unwrap();
    let shoulder = Attach {
        parent: torso,
        relation: Relation::On,
        socket: Some("shoulder".to_owned()),
    };
    reshape::graft_part(&mut sim, first, limb, &shoulder, Cause::DEBUG).unwrap();
    assert!(sim.truth().entity(limb).is_none());
    assert_eq!(
        body_of(&sim, first).function(function("grasping")),
        real(1.0)
    );
    check_sim_body(&sim, first);
    // Something that is not a part cannot be grafted.
    let coat_def = DEFS.lookup::<ItemDef>("core:coat").unwrap();
    let coat = item::create(&mut sim, coat_def, 1, on(4, 4), Cause::DEBUG).unwrap();
    let refused = reshape::graft_part(&mut sim, first, coat, &Attach::on(torso), Cause::DEBUG);
    assert_eq!(refused.unwrap_err().code, "not_a_part");
}
