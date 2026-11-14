//! Interactions (16 §3; package G1) against the real simulation, with
//! the bundled content of `mods/core`: a character opens a closed door
//! through the `core:interact` action and walks through it; a locked
//! door refuses without its key and opens with it; hiding in a locker
//! takes a character out of the world; a blast that destroys the locker
//! spills them back; a rule on `interaction_starting` refuses with a
//! reason the player is told; each reach kind; a hook that extends
//! reach; a rule that interacts for a device; and a save in the middle.

mod common;

use std::sync::Arc;

use clash_core::{Dur, EntityId, Real};
use clash_data::Defs;
use clash_expr::{DefRef, Value};
use clash_sim::action::{ActEvent, ActionDef, How};
use clash_sim::device::{self, Device, DeviceDef};
use clash_sim::interaction::{self, Target};
use clash_sim::movement::{self, parse_dir};
use clash_sim::{
    AdvanceLimit, Command, CommandError, Control, DebugCommand, EntitySpec, EventBody, Handler,
    Place, Presence, Sim, SimEvent, Subject,
};
use clash_time::Timeline;
use clash_world::{
    Cause, CellPos, Cosmos, EnergyRelease, GridId, Layer, Location, OccupantKind, WorldDefs,
};

use common::{GRID, at, creature, debug, names, on, spawn};

/// What these tests add: a reason and a rule that seals a door, a panel
/// with one interaction of each reach, a console on its network, a
/// torch whose interaction needs it in hand, and a rule a device's
/// actuator runs.
const FIXTURES: &str = r#"[
  { "type": "channel", "id": "arm", "name": "arm" },
  { "type": "variable", "id": "touched", "scope": "cell", "value": "bool", "default": false },
  { "type": "variable", "id": "pressed", "scope": "cell", "value": "bool", "default": false },
  { "type": "variable", "id": "zapped", "scope": "cell", "value": "bool", "default": false },
  { "type": "variable", "id": "remote", "scope": "cell", "value": "bool", "default": false },
  { "type": "variable", "id": "waved", "scope": "entity", "value": "bool", "default": false },
  { "type": "reason", "id": "test_sealed", "text": "It is sealed shut.",
    "doc": "Given when a sealed door is to be opened." },
  { "type": "eoc", "id": "test_seal", "on": "interaction_starting",
    "if": "element.has_tag('sealed')",
    "doc": "Nobody opens what is sealed.",
    "do": [ { "cancel_event": { "reason": "test_sealed" } } ] },
  { "type": "block", "id": "test_vault_door", "copy_from": "door", "tags": ["sealed"] },

  { "type": "interaction", "id": "test_touch", "reach": "same_cell",
    "do": [ { "set": { "here.var.touched": true } } ] },
  { "type": "interaction", "id": "test_press", "reach": "adjacent",
    "do": [ { "set": { "here.var.pressed": true } } ] },
  { "type": "interaction", "id": "test_zap", "reach": { "range": 6 },
    "do": [ { "set": { "here.var.zapped": true } } ] },
  { "type": "interaction", "id": "test_remote", "reach": "network",
    "do": [ { "set": { "here.var.remote": true } } ] },
  { "type": "device", "id": "test_panel_device", "tags": ["network:lab"] },
  { "type": "fixture", "id": "test_panel", "material": "steel",
    "height": 0.3, "fill": 0.01,
    "interactions": ["test_touch", "test_press", "test_zap", "test_remote"],
    "device": "test_panel_device",
    "glyph": { "ch": "=", "fg": "material" } },
  { "type": "device", "id": "test_terminal", "tags": ["network:lab"] },
  { "type": "fixture", "id": "test_console", "material": "steel",
    "height": 1.0, "fill": 0.1, "device": "test_terminal",
    "glyph": { "ch": "&", "fg": "material" } },

  { "type": "interaction", "id": "test_wave", "target": "entity", "reach": "held",
    "do": [ { "set": { "target.var.waved": true } } ] },
  { "type": "item", "id": "test_torch", "name": "torch",
    "material": "steel", "mass": "0.5 kg", "length": "0.3 m",
    "grip": { "arm": 1 }, "interactions": ["test_wave"],
    "glyph": { "ch": "/", "fg": "yellow" } },

  { "type": "eoc", "id": "test_actuate",
    "params": { "who": "entity", "door": "element" },
    "doc": "What an actuator's rule does: opens the door beside it.",
    "do": [ { "interact": { "who": "$who", "target": "$door", "interaction": "open" } } ] }
]"#;

/// The bundled content these tests use, with the fixtures.
fn defs() -> Arc<Defs> {
    let read = |name: &str| {
        let path = common::mods_dir().join("core/data").join(name);
        std::fs::read_to_string(&path).unwrap_or_else(|error| panic!("{}: {error}", path.display()))
    };
    let (movement, locks, interact, interactions) = (
        read("movement.json"),
        read("locks.json"),
        read("interact.json"),
        read("interactions.json"),
    );
    common::try_defs_with(&[
        ("data/movement.json", movement.as_str()),
        ("data/interactions.json", interactions.as_str()),
        ("data/locks.json", locks.as_str()),
        ("data/interact.json", interact.as_str()),
        ("data/test/interactions.json", FIXTURES),
    ])
    .unwrap_or_else(|error| panic!("the definitions do not load:\n{error}"))
}

fn cell(x: i32, y: i32, z: i32) -> Location {
    Location::new(GRID, CellPos::new(x, y, z))
}

/// A character under direct control, standing in a cell.
fn character(sim: &mut Sim, at: Location) -> EntityId {
    let id = sim.truth().entities().next_id().unwrap();
    debug(
        sim,
        DebugCommand::CreateEntity {
            spec: creature(),
            place: Place::at(at),
        },
    )
    .unwrap();
    debug(
        sim,
        DebugCommand::SetControl {
            entity: id,
            control: Control::Direct,
        },
    )
    .unwrap();
    id
}

fn put(sim: &mut Sim, at: Location, layer: Layer, written: &str) {
    let world = Arc::clone(sim.truth().cosmos().defs());
    let state = match layer {
        Layer::Floor => world.floor_state(written, None).map(|s| s.raw()),
        Layer::Block => world.block_state(written, None).map(|s| s.raw()),
        Layer::Fixture => world.fixture_state(written, None).map(|s| s.raw()),
    }
    .unwrap_or_else(|error| panic!("{written}: {error}"));
    sim.world(|ctx| ctx.edit(Cause::DEBUG).set_element(at, layer, state))
        .unwrap();
}

fn state_name(sim: &Sim, at: Location, layer: Layer) -> Value {
    sim.property_named(Subject::Element(at, layer), "state_name", &[])
}

fn id_of(sim: &Sim, name: &str) -> interaction::InteractionId {
    interaction::lookup(sim, name).unwrap_or_else(|| panic!("no interaction {name}"))
}

/// Does an interaction at once, as a rule's `interact` does.
fn interact(
    sim: &mut Sim,
    who: EntityId,
    target: Target,
    name: &str,
) -> Result<(), clash_sim::Reason> {
    let id = id_of(sim, name);
    interaction::interact(sim, who, target, id, Cause::DEBUG)
}

/// Starts `core:interact` by command, as the play screen does.
fn act(sim: &mut Sim, who: EntityId, target: Target, name: &str) -> Result<(), CommandError> {
    let action = sim.defs().lookup::<ActionDef>("core:interact").unwrap();
    let ty = sim.exprs().registry().def_type_id("interaction").unwrap();
    let id = Value::Id(DefRef::new(ty, u32::from(id_of(sim, name).raw())));
    let params = vec![
        (
            "target".to_owned(),
            sim.subject_value(target.subject()).to_stored(),
        ),
        ("interaction".to_owned(), id.to_stored()),
    ];
    sim.submit(Command::Act {
        entity: who,
        action,
        params,
        how: How::Start,
    })
}

/// Lets time pass, then gives what was delivered.
fn run(sim: &mut Sim, millis: i64) -> Vec<SimEvent> {
    let until = sim.now() + Dur::from_millis(millis);
    // A directly controlled character is asked what to do next when it
    // is free: answering with a wait keeps time going.
    for _ in 0..16 {
        if sim.now() >= until {
            break;
        }
        match sim.advance(AdvanceLimit::Until(until)) {
            clash_sim::Advance::NeedInput(request) => {
                let _ = sim.submit(Command::Wait {
                    entity: request.entity,
                    dur: Dur::from_millis(100),
                });
            }
            _ => break,
        }
    }
    sim.pump();
    sim.drain_events()
}

fn refusals(events: &[SimEvent]) -> Vec<String> {
    events
        .iter()
        .filter_map(|event| match &event.body {
            EventBody::Act(ActEvent::ActionRefused { reason, .. }) => Some(reason.message.clone()),
            _ => None,
        })
        .collect()
}

// ---- The door of the test city ---------------------------------------------------

/// The outer door in the east wall of the first block's building, and
/// the sidewalk outside it (`cargo run -p clash-gen --example city_ascii
/// -- 1 0`).
const EAST_DOOR: (i32, i32) = (14, 5);

fn city(at: Location) -> (Sim, EntityId) {
    let defs = defs();
    let world = WorldDefs::from_defs(&defs).unwrap_or_else(|d| panic!("{}", d.render()));
    let mut timeline = Timeline::new();
    let mut cosmos = Cosmos::new(Arc::clone(&world));
    let def = world.realms().lookup("core:test_city").unwrap();
    cosmos.create_realm(&mut timeline, def, 1).unwrap();
    let mut sim = Sim::from_parts(defs, cosmos, timeline, 3);
    debug(
        &mut sim,
        DebugCommand::CreateEntity {
            spec: creature(),
            place: Place::Stored,
        },
    )
    .unwrap();
    let walker = sim.truth().entities().ids().next().unwrap();
    debug(&mut sim, DebugCommand::StartSession { entity: walker, at }).unwrap();
    debug(
        &mut sim,
        DebugCommand::SetControl {
            entity: walker,
            control: Control::Direct,
        },
    )
    .unwrap();
    sim.pump();
    sim.drain_events();
    (sim, walker)
}

#[test]
fn a_character_opens_a_closed_door_by_interaction_and_walks_through() {
    let outside = Location::new(
        GridId::new(0),
        CellPos::new(EAST_DOOR.0 + 1, EAST_DOOR.1, 0),
    );
    let door = Location::new(GridId::new(0), CellPos::new(EAST_DOOR.0, EAST_DOOR.1, 0));
    let (mut sim, walker) = city(outside);
    assert_eq!(state_name(&sim, door, Layer::Block), Value::from("closed"));
    let west = parse_dir("west").unwrap();

    // Walking into the closed door is refused, and offers its default.
    let refused =
        movement::step(&mut sim, walker, west, false, Cause::DEBUG.by(walker)).unwrap_err();
    assert_eq!(refused.code, "move.closed");
    let (target, offer) = interaction::bump_default(&sim, walker, "west").unwrap();
    assert_eq!(target, Target::Element(door, Layer::Block));
    assert_eq!(offer.id, "core:open");
    assert!(offer.available());
    let menu = interaction::interactions_for(&sim, walker, target);
    let close = menu.iter().find(|offer| offer.id == "core:close").unwrap();
    assert_eq!(
        close.refusal.as_ref().map(|r| r.code.as_str()),
        Some("interaction.not_now")
    );

    // The player's choice starts the action; its time passes; the door
    // is open and the step is taken.
    act(&mut sim, walker, target, "core:open").unwrap();
    let events = run(&mut sim, 1_000);
    assert!(
        names(&events).contains(&"interacted"),
        "{:?}",
        names(&events)
    );
    assert_eq!(state_name(&sim, door, Layer::Block), Value::from("open"));
    movement::step(&mut sim, walker, west, false, Cause::DEBUG.by(walker)).unwrap();
    movement::step(&mut sim, walker, west, false, Cause::DEBUG.by(walker)).unwrap();
    let inside = Location::new(
        GridId::new(0),
        CellPos::new(EAST_DOOR.0 - 1, EAST_DOOR.1, 0),
    );
    assert_eq!(sim.truth().place(walker), Some(Place::at(inside)));
    // Opened, the door offers no default: bumping into it is a step.
    assert!(interaction::default_for(&sim, walker, target).is_none());
}

// ---- Locks, lockers and rules ------------------------------------------------------

fn flat() -> Sim {
    common::sim_over(defs(), 9)
}

#[test]
fn a_locked_door_refuses_without_its_key_and_opens_with_it() {
    let mut sim = flat();
    let door = at(10, 10);
    put(&mut sim, door, Layer::Block, "lockable_door#locked");
    let hero = character(&mut sim, at(11, 10));
    let target = Target::Element(door, Layer::Block);

    let menu = interaction::interactions_for(&sim, hero, target);
    let unlock = menu.iter().find(|offer| offer.id == "core:unlock").unwrap();
    let refusal = unlock.refusal.as_ref().unwrap();
    assert_eq!(refusal.code, "interaction.not_now");
    assert_eq!(refusal.message, "It is locked, and you have no key.");
    let refused = act(&mut sim, hero, target, "core:open").unwrap_err();
    assert_eq!(refused.reason.message, "It does not open.");

    // What the action reads of the character, as properties.
    let door_value = sim.subject_value(Subject::Element(door, Layer::Block));
    let ty = sim.exprs().registry().def_type_id("interaction").unwrap();
    let unlock_id = Value::Id(DefRef::new(ty, u32::from(id_of(&sim, "core:unlock").raw())));
    let ask = |sim: &Sim, name: &str, args: &[Value]| {
        sim.property_named(Subject::Entity(hero), name, args)
    };
    let both = [door_value.clone(), unlock_id.clone()];
    assert_eq!(ask(&sim, "can_interact", &both), Value::Bool(false));
    assert_eq!(
        ask(&sim, "interaction_fault", &both),
        Value::from("It is locked, and you have no key.")
    );
    assert_eq!(
        ask(&sim, "interaction_time", std::slice::from_ref(&unlock_id)),
        Value::Real(Real::of(1.0))
    );
    assert_eq!(
        ask(&sim, "has_item", &[Value::from("core:key")]),
        Value::Bool(false)
    );
    assert_eq!(
        sim.property_named(Subject::Element(door, Layer::Block), "interactions", &[]),
        Value::list(["core:open", "core:close", "core:lock", "core:unlock"].map(Value::from))
    );
    let refused = act(&mut sim, hero, target, "core:unlock").unwrap_err();
    assert_eq!(refused.reason.message, "It is locked, and you have no key.");
    assert_eq!(state_name(&sim, door, Layer::Block), Value::from("locked"));

    // With the key in a pocket, the door unlocks, and then opens.
    debug(
        &mut sim,
        DebugCommand::CreateItem {
            def: "core:key".to_owned(),
            count: 1,
            place: Place::In(hero),
        },
    )
    .unwrap();
    assert_eq!(
        interaction::default_for(&sim, hero, target).map(|offer| offer.id),
        Some("core:unlock".to_owned())
    );
    assert_eq!(
        sim.property_named(
            Subject::Entity(hero),
            "has_item",
            &[Value::from("core:key")]
        ),
        Value::Bool(true)
    );
    act(&mut sim, hero, target, "core:unlock").unwrap();
    run(&mut sim, 1_500);
    assert_eq!(state_name(&sim, door, Layer::Block), Value::from("closed"));
    act(&mut sim, hero, target, "core:open").unwrap();
    run(&mut sim, 1_000);
    assert_eq!(state_name(&sim, door, Layer::Block), Value::from("open"));
}

#[test]
fn a_rule_on_interaction_starting_refuses_with_a_reason_the_player_is_told() {
    let mut sim = flat();
    let door = at(10, 10);
    put(&mut sim, door, Layer::Block, "test_vault_door");
    let hero = character(&mut sim, at(11, 10));
    let target = Target::Element(door, Layer::Block);
    // As far as the character knows it can be opened; the rule says no
    // when it is tried.
    assert!(interaction::default_for(&sim, hero, target).is_some());
    act(&mut sim, hero, target, "core:open").unwrap();
    let events = run(&mut sim, 1_000);
    assert_eq!(refusals(&events), ["It is sealed shut."]);
    assert_eq!(state_name(&sim, door, Layer::Block), Value::from("closed"));
    let refused = interact(&mut sim, hero, target, "core:open").unwrap_err();
    assert_eq!(refused.message, "It is sealed shut.");
}

fn device_count(sim: &Sim) -> usize {
    sim.components().store::<Device>().len()
}

#[test]
fn hiding_in_a_locker_takes_a_character_out_of_the_world_until_a_blast_spills_them() {
    let mut sim = flat();
    let locker = at(10, 10);
    put(&mut sim, locker, Layer::Fixture, "steel_locker");
    let hero = character(&mut sim, at(11, 10));
    let target = Target::Element(locker, Layer::Fixture);
    assert_eq!(device_count(&sim), 0);

    act(&mut sim, hero, target, "core:hide_inside").unwrap();
    run(&mut sim, 2_000);
    let box_ = device::device_at(&sim, locker, Layer::Fixture).unwrap();
    assert_eq!(sim.truth().place(hero), Some(Place::In(box_)));
    assert!(sim.truth().cosmos().occupant(hero).is_none());
    // From inside, the locker offers a way out, and no way in.
    let menu = interaction::interactions_for(&sim, hero, target);
    let ids: Vec<&str> = menu.iter().map(|offer| offer.id.as_str()).collect();
    assert!(
        ids.contains(&"core:get_out") && !ids.contains(&"core:hide_inside"),
        "{ids:?}"
    );
    act(&mut sim, hero, target, "core:get_out").unwrap();
    run(&mut sim, 2_000);
    assert_eq!(sim.truth().place(hero), Some(Place::at(locker)));
    assert_eq!(
        device_count(&sim),
        0,
        "the empty, shut locker is plain again"
    );

    // Hidden again, and a blast beside the locker destroys it: the
    // character is out in the world.
    debug(
        &mut sim,
        DebugCommand::MoveEntity {
            entity: hero,
            to: on(11, 10),
        },
    )
    .unwrap();
    act(&mut sim, hero, target, "core:hide_inside").unwrap();
    run(&mut sim, 2_000);
    assert!(matches!(sim.truth().place(hero), Some(Place::In(_))));
    let blast = sim
        .truth()
        .cosmos()
        .defs()
        .damage_types()
        .lookup("core:blast")
        .unwrap();
    let release = EnergyRelease::sphere(locker, blast, Real::of(5.0e7), Cause::DEBUG);
    sim.world(|ctx| ctx.release(release)).unwrap();
    let events = run(&mut sim, 300);
    assert!(
        names(&events).contains(&"device_spilled"),
        "{:?}",
        names(&events)
    );
    assert!(matches!(sim.truth().place(hero), Some(Place::At { .. })));
    assert!(sim.truth().cosmos().occupant(hero).is_some());
}

#[test]
fn a_locker_takes_what_is_in_hand_and_gives_it_back() {
    let mut sim = flat();
    let locker = at(10, 10);
    put(&mut sim, locker, Layer::Fixture, "steel_locker#open");
    let hero = character(&mut sim, at(11, 10));
    let target = Target::Element(locker, Layer::Fixture);
    // A bare creature has no hands: give it the key as though held.
    debug(
        &mut sim,
        DebugCommand::CreateItem {
            def: "core:key".to_owned(),
            count: 1,
            place: Place::In(hero),
        },
    )
    .unwrap();
    let key = sim.truth().contents(hero)[0];
    let box_ = device::device_at(&sim, locker, Layer::Fixture);
    assert!(box_.is_some(), "an open locker is a device");
    clash_sim::item::put_in(&mut sim, key, box_.unwrap(), Cause::DEBUG).unwrap();
    let menu = interaction::interactions_for(&sim, hero, target);
    let retrieve = menu
        .iter()
        .find(|offer| offer.id == "core:retrieve")
        .unwrap();
    assert!(retrieve.available());
}

// ---- Reach ---------------------------------------------------------------------------

fn available(sim: &Sim, who: EntityId, target: Target, name: &str) -> bool {
    interaction::interactions_for(sim, who, target)
        .iter()
        .find(|offer| offer.id == name)
        .is_some_and(interaction::Offer::available)
}

#[test]
fn each_reach_kind_is_honoured() {
    let mut sim = flat();
    let panel = cell(20, 20, 1);
    put(&mut sim, panel, Layer::Floor, "slab");
    put(&mut sim, panel, Layer::Fixture, "test_panel");
    let target = Target::Element(panel, Layer::Fixture);
    let hero = character(&mut sim, cell(21, 21, 1));
    // Adjacent, diagonally; not the same cell; within range in the open.
    assert!(available(&sim, hero, target, "core:test_press"));
    assert!(!available(&sim, hero, target, "core:test_touch"));
    assert!(available(&sim, hero, target, "core:test_zap"));
    debug(
        &mut sim,
        DebugCommand::MoveEntity {
            entity: hero,
            to: Place::at(panel),
        },
    )
    .unwrap();
    assert!(available(&sim, hero, target, "core:test_touch"));
    // Two cells off is not adjacent; five is within range; a wall
    // between takes away the line; eight is out of range.
    debug(
        &mut sim,
        DebugCommand::MoveEntity {
            entity: hero,
            to: Place::at(cell(25, 20, 1)),
        },
    )
    .unwrap();
    assert!(!available(&sim, hero, target, "core:test_press"));
    assert!(available(&sim, hero, target, "core:test_zap"));
    put(&mut sim, cell(22, 20, 1), Layer::Block, "wall");
    assert!(!available(&sim, hero, target, "core:test_zap"));
    debug(
        &mut sim,
        DebugCommand::MoveEntity {
            entity: hero,
            to: Place::at(cell(28, 21, 1)),
        },
    )
    .unwrap();
    assert!(!available(&sim, hero, target, "core:test_zap"));
    // Straight below, with the slab between, is not adjacent; with the
    // slab gone it is.
    debug(
        &mut sim,
        DebugCommand::MoveEntity {
            entity: hero,
            to: Place::at(cell(20, 20, 0)),
        },
    )
    .unwrap();
    assert!(!available(&sim, hero, target, "core:test_press"));
    put(&mut sim, panel, Layer::Floor, "engine:none");
    assert!(available(&sim, hero, target, "core:test_press"));

    // Network: nothing on the panel's network is operated, then a
    // console beside the character is, a plain element that names a
    // device on it.
    debug(
        &mut sim,
        DebugCommand::MoveEntity {
            entity: hero,
            to: Place::at(cell(5, 5, 0)),
        },
    )
    .unwrap();
    assert!(!available(&sim, hero, target, "core:test_remote"));
    put(&mut sim, cell(5, 6, 0), Layer::Fixture, "test_console");
    assert!(available(&sim, hero, target, "core:test_remote"));
    assert_eq!(device_count(&sim), 0);
    interact(&mut sim, hero, target, "core:test_remote").unwrap();
    assert_eq!(
        sim.var(Subject::Loc(panel), "core:remote"),
        Value::Bool(true)
    );

    // Held: the torch waves only in hand.
    let torch_def = clash_sim::item::def_named(&sim, "core:test_torch").unwrap();
    let torch = clash_sim::item::create(&mut sim, torch_def, 1, on(5, 4), Cause::DEBUG).unwrap();
    let held = Target::Entity(torch);
    assert!(!available(&sim, hero, held, "core:test_wave"));
    debug(
        &mut sim,
        DebugCommand::MoveEntity {
            entity: torch,
            to: Place::In(hero),
        },
    )
    .unwrap();
    assert!(available(&sim, hero, held, "core:test_wave"));
    interact(&mut sim, hero, held, "core:test_wave").unwrap();
    assert_eq!(
        sim.var(Subject::Entity(torch), "core:waved"),
        Value::Bool(true)
    );
}

#[test]
fn a_hook_on_interaction_reach_extends_reach() {
    let mut sim = flat();
    let panel = at(30, 30);
    put(&mut sim, panel, Layer::Fixture, "test_panel");
    let hero = character(&mut sim, at(10, 10));
    let target = Target::Element(panel, Layer::Fixture);
    let press = id_of(&sim, "core:test_press");
    let refused = interaction::interact(&mut sim, hero, target, press, Cause::DEBUG).unwrap_err();
    assert_eq!(refused.code, "interaction.out_of_reach");
    // Telekinesis, as a handler: whoever is tagged reaches anything.
    sim.on(
        "interaction_reach",
        Handler::new(|sim, delivery| {
            let reaching = delivery
                .event()
                .subject("self")
                .and_then(|subject| match subject {
                    Subject::Entity(id) => sim.truth().entity(id),
                    _ => None,
                })
                .is_some_and(|entity| entity.tags.contains("telekinetic"));
            if reaching {
                let _ = delivery.set_field("reached", true);
            }
        }),
    )
    .unwrap();
    sim.entity_mut(hero).unwrap().tags.insert("telekinetic");
    interaction::interact(&mut sim, hero, target, press, Cause::DEBUG).unwrap();
    assert_eq!(
        sim.var(Subject::Loc(panel), "core:pressed"),
        Value::Bool(true)
    );
}

#[test]
fn a_devices_rule_interacts_with_a_neighbouring_element() {
    let mut sim = flat();
    let door = at(10, 10);
    put(&mut sim, door, Layer::Block, "door");
    // An actuator standing beside the door: a device entity of its own.
    let actuator = spawn(
        &mut sim,
        EntitySpec::new(OccupantKind::Device).with_presence(Presence::new(OccupantKind::Device)),
        on(10, 11),
    );
    let def = sim
        .defs()
        .lookup::<DeviceDef>("core:test_terminal")
        .unwrap();
    sim.components_mut()
        .insert(actuator, Device::new(def, None));
    let rule = sim.rule_set().lookup("core:test_actuate").unwrap();
    let args = [
        ("who", sim.subject_value(Subject::Entity(actuator))),
        (
            "door",
            sim.subject_value(Subject::Element(door, Layer::Block)),
        ),
    ];
    sim.call_rule(rule, &[], &args, Cause::new(clash_world::CauseKind::Eoc))
        .unwrap();
    assert_eq!(state_name(&sim, door, Layer::Block), Value::from("open"));
}

#[test]
fn an_interaction_survives_a_save_in_the_middle() {
    let mut sim = flat();
    let door = at(10, 10);
    put(&mut sim, door, Layer::Block, "door");
    let hero = character(&mut sim, at(11, 10));
    act(
        &mut sim,
        hero,
        Target::Element(door, Layer::Block),
        "core:open",
    )
    .unwrap();
    run(&mut sim, 300);
    assert_eq!(state_name(&sim, door, Layer::Block), Value::from("closed"));
    let (mut loaded, unplaced) = common::save_and_load(&sim);
    assert!(unplaced.is_empty());
    assert_eq!(loaded.state_hash(), sim.state_hash());
    run(&mut sim, 1_000);
    run(&mut loaded, 1_000);
    assert_eq!(state_name(&loaded, door, Layer::Block), Value::from("open"));
    assert_eq!(loaded.state_hash(), sim.state_hash());
}

#[test]
fn the_bundled_doors_windows_lamps_and_lockers_offer_what_they_should() {
    let mut sim = flat();
    let hero = character(&mut sim, at(11, 11));
    let cases = [
        (Layer::Block, "door", vec!["core:open", "core:close"]),
        (
            Layer::Block,
            "window",
            vec!["core:open", "core:close", "core:break_through"],
        ),
        (Layer::Fixture, "lamp", vec!["core:switch"]),
        (
            Layer::Fixture,
            "steel_locker",
            // Searched by whoever looks for someone hidden in it (D-950).
            vec![
                "core:open",
                "core:close",
                "core:store",
                "core:retrieve",
                "core:hide_inside",
                "core:search",
            ],
        ),
    ];
    for (layer, written, expected) in cases {
        put(&mut sim, at(10, 10), layer, written);
        let target = Target::Element(at(10, 10), layer);
        let ids: Vec<String> = interaction::interactions_for(&sim, hero, target)
            .into_iter()
            .map(|offer| offer.id)
            .collect();
        assert_eq!(ids, expected, "{written}");
        put(&mut sim, at(10, 10), layer, "engine:none");
    }
    // The lamp switches off and on; the window breaks.
    put(&mut sim, at(10, 10), Layer::Fixture, "lamp");
    let lamp = Target::Element(at(10, 10), Layer::Fixture);
    interact(&mut sim, hero, lamp, "core:switch").unwrap();
    assert_eq!(
        state_name(&sim, at(10, 10), Layer::Fixture),
        Value::from("off")
    );
    put(&mut sim, at(12, 12), Layer::Block, "window");
    let window = Target::Element(at(12, 12), Layer::Block);
    interact(&mut sim, hero, window, "core:break_through").unwrap();
    assert_eq!(state_name(&sim, at(12, 12), Layer::Block), Value::Null);
    // Everything near the character, for a menu.
    let near = interaction::offers_near(&sim, hero);
    assert!(near.iter().any(|(target, _)| *target == lamp));
}
