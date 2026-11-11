//! Tests for W6: `WorldCtx`, `WorldEdit`, the bookkeeping on every
//! structural change, causes, world events and the event sink.

mod common;

use std::sync::Arc;

use clash_core::{EntityId, Hash128, Tick, Value, VarKey, codec};
use clash_time::Timeline;
use clash_world::dims::{CHUNK_XY, TRACT_XY};
use clash_world::test_support::load_flat;
use clash_world::{
    BlockState, Cause, CauseKind, CellFlags, CellPos, ChunkRev, ColumnBuilder, Cosmos, DecalId,
    EventSink, FixtureState, FloorState, Footprint, GridId, Layer, Location, Occupant,
    OccupantKind, StabilityQueue, TractPos, WorldCtx, WorldDefs, WorldError, WorldEvent,
};
use proptest::prelude::*;

struct World {
    defs: Arc<WorldDefs>,
    timeline: Timeline,
    cosmos: Cosmos,
    events: EventSink<WorldEvent>,
    grid: GridId,
    rock: BlockState,
    wall: BlockState,
    pillar: BlockState,
    door: BlockState,
    door_open: BlockState,
    slab: FloorState,
    grass: FloorState,
    desk: FixtureState,
}

/// A cosmos with one `testland` realm and flat rock under grass in the
/// tracts (0, 0) and (1, 0).
fn world() -> World {
    let defs = common::test_defs();
    let mut timeline = Timeline::new();
    let mut cosmos = Cosmos::new(Arc::clone(&defs));
    let def = defs.realms().lookup("core:testland").unwrap();
    let realm = cosmos.create_realm(&mut timeline, def, 7).unwrap();
    let grid = cosmos.realm(realm).unwrap().base_grid;
    let rock = defs.block_state("solid", None).unwrap();
    let grass = defs.floor_state("grass", None).unwrap();
    load_flat(
        &mut cosmos,
        grid,
        [TractPos::new(0, 0), TractPos::new(1, 0)],
        rock,
        grass,
        0,
    )
    .unwrap();
    World {
        rock,
        wall: defs.block_state("wall", None).unwrap(),
        pillar: defs.block_state("pillar", None).unwrap(),
        door: defs.block_state("door", None).unwrap(),
        door_open: defs.block_state("door#open", None).unwrap(),
        slab: defs.floor_state("slab", None).unwrap(),
        grass,
        desk: defs.fixture_state("desk", None).unwrap(),
        defs,
        timeline,
        cosmos,
        events: EventSink::new(),
        grid,
    }
}

impl World {
    fn ctx(&mut self) -> WorldCtx<'_> {
        WorldCtx::new(&mut self.cosmos, &mut self.timeline, &mut self.events)
    }

    fn at(&self, x: i32, y: i32, z: i32) -> Location {
        Location::new(self.grid, CellPos::new(x, y, z))
    }

    /// The revision counters of the chunk a cell is in, and whether its
    /// column is dirty.
    fn rev(&self, loc: Location) -> (Option<ChunkRev>, bool) {
        let column = self
            .cosmos
            .grid(loc.grid)
            .unwrap()
            .column(loc.cell.tract())
            .unwrap();
        (
            column.chunk(loc.cell.chunk().z).map(|chunk| chunk.rev()),
            column.is_dirty(),
        )
    }
}

fn someone() -> Cause {
    Cause::new(CauseKind::Action).by(EntityId::new(3, 0))
}

#[test]
fn an_effective_edit_emits_one_event_with_old_new_and_cause() {
    let mut w = world();
    let at = w.at(4, 5, 0);
    let (wall, grass, slab, desk) = (w.wall, w.grass, w.slab, w.desk);
    w.timeline.advance_to(Tick(900));
    let structure_before = w.rev(at).0.unwrap().structure;

    let mut ctx = w.ctx();
    let mut edit = ctx.edit(someone());
    assert_eq!(edit.cause(), someone());
    assert_eq!(edit.set_block(at, wall), Ok(BlockState::NONE));
    assert_eq!(edit.set_floor(at, slab), Ok(grass));
    assert_eq!(edit.set_fixture(at, desk), Ok(FixtureState::NONE));
    assert_eq!(
        w.events.drain(),
        [
            WorldEvent::ElementChanged {
                tick: Tick(900),
                loc: at,
                layer: Layer::Block,
                old: BlockState::NONE.raw(),
                new: wall.raw(),
                cause: someone(),
            },
            WorldEvent::ElementChanged {
                tick: Tick(900),
                loc: at,
                layer: Layer::Floor,
                old: grass.raw(),
                new: slab.raw(),
                cause: someone(),
            },
            WorldEvent::ElementChanged {
                tick: Tick(900),
                loc: at,
                layer: Layer::Fixture,
                old: FixtureState::NONE.raw(),
                new: desk.raw(),
                cause: someone(),
            },
        ]
    );
    let cell = w.cosmos.cell(at).unwrap();
    assert_eq!((cell.block, cell.floor, cell.fixture), (wall, slab, desk));
    // The bookkeeping: derived flags, the heightmap, the revision and the
    // dirty flag.
    assert!(
        w.cosmos
            .flags(at)
            .unwrap()
            .contains(CellFlags::WALK_BLOCKED | CellFlags::OPAQUE | CellFlags::HAS_FLOOR)
    );
    assert_eq!(w.cosmos.height(at), Ok(0));
    let (rev, dirty) = w.rev(at);
    assert_eq!(rev.unwrap().structure, structure_before + 3);
    assert!(dirty);

    // A wall two storeys up raises the column's height, and taking it
    // away brings the height back down.
    let high = w.at(4, 5, 2);
    w.ctx().edit(Cause::DEBUG).set_block(high, wall).unwrap();
    assert_eq!(w.cosmos.height(at), Ok(2));
    assert_eq!(w.cosmos.under_open_sky(w.at(4, 5, 1)), Ok(false));
    w.ctx()
        .edit(Cause::DEBUG)
        .set_block(high, BlockState::NONE)
        .unwrap();
    assert_eq!(w.cosmos.height(at), Ok(0));
    assert_eq!(w.events.len(), 2);
}

#[test]
fn an_edit_that_changes_nothing_emits_nothing_and_bumps_nothing() {
    let mut w = world();
    let at = w.at(9, 9, 0);
    let (wall, grass) = (w.wall, w.grass);
    w.ctx().edit(Cause::DEBUG).set_block(at, wall).unwrap();
    w.events.clear();
    let stored = |w: &World| {
        let view = w.cosmos.grid(w.grid).unwrap();
        view.column(TractPos::new(0, 0)).unwrap().stored_range()
    };
    let before = (w.rev(at), Hash128::of(&w.cosmos), stored(&w));

    let mut ctx = w.ctx();
    let mut edit = ctx.edit(someone());
    assert_eq!(edit.set_block(at, wall), Ok(wall));
    assert_eq!(edit.set_floor(at, grass), Ok(grass));
    assert_eq!(
        edit.set_fixture(at, FixtureState::NONE),
        Ok(FixtureState::NONE)
    );
    assert_eq!(
        edit.set_element(at, Layer::Block, wall.raw()),
        Ok(wall.raw())
    );
    assert_eq!(
        edit.set_material(at, Layer::Block, wall.material()),
        Ok(wall.raw())
    );
    assert_eq!(edit.set_damage(at, Layer::Block, 0), Ok(0));
    // There is no fixture to damage or to make of something else.
    assert_eq!(edit.set_damage(at, Layer::Fixture, 40), Ok(0));
    assert_eq!(
        edit.set_material(at, Layer::Fixture, wall.material()),
        Ok(FixtureState::NONE.raw())
    );
    assert_eq!(edit.remove_decals(at, DecalId::new(1)), Ok(0));
    assert_eq!(edit.clear_var(at, VarKey::new("core:mark")), Ok(None));
    // Writing nothing where nothing is stored does not create storage.
    let sky = Location::new(at.grid, CellPos::new(9, 9, 150));
    assert_eq!(edit.set_block(sky, BlockState::NONE), Ok(BlockState::NONE));
    assert_eq!(edit.clear_var(sky, VarKey::new("core:mark")), Ok(None));

    assert!(w.events.is_empty());
    assert_eq!((w.rev(at), Hash128::of(&w.cosmos), stored(&w)), before);
    assert!(w.cosmos.stability_requests().is_empty());
}

#[test]
fn an_edit_where_nothing_can_be_edited_is_an_error_value() {
    let mut w = world();
    let wall = w.wall;
    let unloaded = w.at(5 * TRACT_XY, 0, 0);
    let below = w.at(0, 0, -17);
    let north = w.at(0, -1, 0);
    let nowhere = Location::new(GridId::new(9), CellPos::new(0, 0, 0));
    let before = Hash128::of(&w.cosmos);

    let mut ctx = w.ctx();
    let mut edit = ctx.edit(Cause::DEBUG);
    assert_eq!(
        edit.set_block(unloaded, wall),
        Err(WorldError::NotLoaded(unloaded))
    );
    assert!(matches!(
        edit.set_floor(below, FloorState::NONE),
        Err(WorldError::OutsideRealm { .. })
    ));
    assert!(matches!(
        edit.set_fixture(north, FixtureState::NONE),
        Err(WorldError::OutsideRealm { .. })
    ));
    assert_eq!(
        edit.set_block(nowhere, wall),
        Err(WorldError::UnknownGrid(GridId::new(9)))
    );
    assert_eq!(
        edit.set_damage(unloaded, Layer::Block, 3),
        Err(WorldError::NotLoaded(unloaded))
    );
    assert_eq!(
        edit.add_decal(unloaded, DecalId::new(0), 1),
        Err(WorldError::NotLoaded(unloaded))
    );
    assert_eq!(
        edit.remove_decals(unloaded, DecalId::new(0)),
        Err(WorldError::NotLoaded(unloaded))
    );
    assert_eq!(
        edit.set_var(unloaded, VarKey::new("core:x"), Value::from(1)),
        Err(WorldError::NotLoaded(unloaded))
    );
    assert_eq!(
        edit.set_material(unloaded, Layer::Block, wall.material()),
        Err(WorldError::NotLoaded(unloaded))
    );
    assert!(w.events.is_empty());
    assert_eq!(Hash128::of(&w.cosmos), before);

    // The world wraps east to west: a position past the seam is the cell
    // on the other side, and the event says so.
    let west = w.at(-1, 3, 0);
    assert_eq!(
        w.ctx().edit(Cause::DEBUG).set_block(west, wall),
        Err(WorldError::NotLoaded(Location::new(
            w.grid,
            CellPos::new(2 * clash_world::dims::REGION_CELLS - 1, 3, 0)
        )))
    );
}

#[test]
fn damage_goes_with_the_element_it_is_on() {
    let mut w = world();
    let at = w.at(2, 2, 0);
    let (wall, pillar) = (w.wall, w.pillar);
    let steel = w.defs.materials().lookup("core:steel").unwrap();
    let mut ctx = w.ctx();
    let mut edit = ctx.edit(someone());
    edit.set_block(at, wall).unwrap();
    assert_eq!(edit.set_damage(at, Layer::Block, 90), Ok(0));
    assert_eq!(edit.set_damage(at, Layer::Floor, 255), Ok(0));
    assert_eq!(w.cosmos.damage(at), Ok([254, 90, 0]));
    assert!(matches!(
        w.events.events()[1],
        WorldEvent::ElementDamaged {
            layer: Layer::Block,
            old: 0,
            new: 90,
            ..
        }
    ));
    assert_eq!(w.events.len(), 3);

    // Transmuting keeps the element and its damage.
    let mut ctx = w.ctx();
    let mut edit = ctx.edit(someone());
    assert_eq!(edit.set_material(at, Layer::Block, steel), Ok(wall.raw()));
    assert_eq!(w.cosmos.damage(at), Ok([254, 90, 0]));
    assert_eq!(w.cosmos.cell(at).unwrap().block, wall.with_material(steel));
    // Replacing it does not: the new element starts undamaged.
    w.ctx().edit(someone()).set_block(at, pillar).unwrap();
    assert_eq!(w.cosmos.damage(at), Ok([254, 0, 0]));
    w.ctx()
        .edit(someone())
        .set_floor(at, FloorState::NONE)
        .unwrap();
    assert_eq!(w.cosmos.damage(at), Ok([0, 0, 0]));
}

#[test]
fn removing_or_weakening_structure_asks_for_a_stability_check() {
    let mut w = world();
    let (wall, door, door_open, slab, desk, rock) =
        (w.wall, w.door, w.door_open, w.slab, w.desk, w.rock);
    let a = w.at(1, 1, 0);
    let b = w.at(2, 1, 0);
    let c = w.at(3, 1, 0);
    w.timeline.advance_to(Tick(40));

    let mut ctx = w.ctx();
    let mut edit = ctx.edit(someone());
    // Building asks for nothing: nothing was removed or weakened.
    edit.set_block(a, wall).unwrap();
    edit.set_block(b, door).unwrap();
    edit.set_fixture(c, desk).unwrap();
    edit.set_floor(w_at(a, 0, 0, 1), slab).unwrap();
    // Nor does a door opening and closing, a fixture going, or damage to
    // a fixture.
    edit.set_block(b, door_open).unwrap();
    edit.set_block(b, door).unwrap();
    edit.set_damage(c, Layer::Fixture, 100).unwrap();
    edit.set_fixture(c, FixtureState::NONE).unwrap();
    assert!(w.cosmos.stability_requests().is_empty());

    // Damage to a wall does, and healing it does not.
    let mut ctx = w.ctx();
    let mut edit = ctx.edit(someone());
    edit.set_damage(a, Layer::Block, 10).unwrap();
    assert!(w.cosmos.stability_requests().contains(a));
    assert_eq!(w.cosmos.take_stability_requests().len(), 1);
    w.ctx()
        .edit(someone())
        .set_damage(a, Layer::Block, 5)
        .unwrap();
    assert!(w.cosmos.stability_requests().is_empty());

    // Removing a wall, a floor or the ground does; so does making it of
    // something else. One cell asked about twice is one request: the
    // first.
    w.timeline.advance_to(Tick(50));
    let steel = w.defs.materials().lookup("core:steel").unwrap();
    let under = w_at(a, 0, 0, -1);
    let above = w_at(a, 0, 0, 1);
    let mut ctx = w.ctx();
    let mut edit = ctx.edit(someone());
    edit.set_material(a, Layer::Block, steel).unwrap();
    edit.set_block(a, BlockState::NONE).unwrap();
    edit.set_floor(above, FloorState::NONE).unwrap();
    assert_eq!(edit.set_block(under, BlockState::NONE), Ok(rock));
    let mut other = ctx.edit(Cause::DEBUG);
    other.set_block(a, wall).unwrap();
    other.set_block(a, BlockState::NONE).unwrap();

    let waiting: Vec<_> = w.cosmos.stability_requests().iter().collect();
    assert_eq!(
        waiting
            .iter()
            .map(|request| request.loc)
            .collect::<Vec<_>>(),
        [under, a, above]
    );
    assert!(
        waiting
            .iter()
            .all(|request| request.tick == Tick(50) && request.cause == someone())
    );
    // The queue is state: it hashes, and it survives the binary codec.
    let bytes = codec::to_bytes(w.cosmos.stability_requests()).unwrap();
    let back: StabilityQueue = codec::from_bytes(&bytes).unwrap();
    assert_eq!(&back, w.cosmos.stability_requests());
    let hashed = Hash128::of(&w.cosmos);
    assert_eq!(w.cosmos.take_stability_requests(), waiting);
    assert_ne!(Hash128::of(&w.cosmos), hashed);
}

fn w_at(loc: Location, dx: i32, dy: i32, dz: i32) -> Location {
    loc.offset(clash_core::IVec3::new(dx, dy, dz))
}

#[test]
fn marks_and_variables_change_through_edits_with_events() {
    let mut w = world();
    let at = w.at(6, 6, 0);
    let blood = DecalId::new(2);
    let key = VarKey::new("core:alarm");
    w.timeline.advance_to(Tick(77));
    let structure_before = w.rev(at).0.unwrap().structure;

    let mut ctx = w.ctx();
    let mut edit = ctx.edit(someone());
    edit.add_decal(at, blood, 9).unwrap();
    edit.add_decal(at, DecalId::new(5), 1).unwrap();
    edit.add_decal(at, blood, 4).unwrap();
    assert_eq!(edit.set_var(at, key.clone(), Value::from(true)), Ok(None));
    assert_eq!(
        edit.set_var(at, key.clone(), Value::from(true)),
        Ok(Some(Value::from(true)))
    );
    assert_eq!(
        edit.set_var(at, key.clone(), Value::from(3)),
        Ok(Some(Value::from(true)))
    );
    assert_eq!(edit.remove_decals(at, blood), Ok(2));
    assert_eq!(edit.remove_decals(at, blood), Ok(0));
    assert_eq!(edit.clear_var(at, key.clone()), Ok(Some(Value::from(3))));

    let decals = w.cosmos.decals(at).unwrap();
    assert_eq!(decals.len(), 1);
    assert_eq!(
        (decals[0].kind, decals[0].cause, decals[0].tick),
        (DecalId::new(5), someone(), Tick(77))
    );
    assert_eq!(w.cosmos.var(at, "core:alarm"), Ok(None));

    let names: Vec<&str> = w.events.events().iter().map(WorldEvent::name).collect();
    assert_eq!(
        names,
        [
            "decal_added",
            "decal_added",
            "decal_added",
            "cell_var_changed",
            "cell_var_changed",
            "decals_removed",
            "cell_var_changed",
        ]
    );
    assert!(w.events.events().iter().all(|event| {
        event.tick() == Tick(77) && event.cause() == someone() && event.location() == at
    }));
    assert_eq!(
        w.events.events()[5],
        WorldEvent::DecalsRemoved {
            tick: Tick(77),
            loc: at,
            kind: blood,
            count: 2,
            cause: someone(),
        }
    );
    assert_eq!(
        w.events.events()[6],
        WorldEvent::CellVarChanged {
            tick: Tick(77),
            loc: at,
            key,
            old: Some(Value::from(3)),
            new: None,
            cause: someone(),
        }
    );
    // None of this is structure.
    assert!(w.cosmos.stability_requests().is_empty());
    assert_eq!(w.rev(at).0.unwrap().structure, structure_before);
}

#[test]
fn events_hash_and_survive_the_binary_codec() {
    let mut w = world();
    let at = w.at(1, 2, 0);
    let wall = w.wall;
    let mut ctx = w.ctx();
    let mut edit = ctx.edit(someone());
    edit.set_block(at, wall).unwrap();
    edit.set_damage(at, Layer::Block, 12).unwrap();
    edit.add_decal(at, DecalId::new(1), 2).unwrap();
    edit.set_var(at, VarKey::new("core:n"), Value::from("x"))
        .unwrap();
    edit.remove_decals(at, DecalId::new(1)).unwrap();

    let bytes = codec::to_bytes(&w.events).unwrap();
    let back: EventSink<WorldEvent> = codec::from_bytes(&bytes).unwrap();
    assert_eq!(back, w.events);
    assert_eq!(Hash128::of(&back), Hash128::of(&w.events));
    let json = serde_json::to_string(&w.events.events()[0]).unwrap();
    assert!(json.starts_with(r#"{"element_changed":{"#), "{json}");

    // Two events that differ in one field hash differently.
    let mut other = w.events.clone();
    let mut events = other.drain();
    if let WorldEvent::ElementDamaged { new, .. } = &mut events[1] {
        *new += 1;
    }
    events.into_iter().for_each(|event| other.emit(event));
    assert_ne!(Hash128::of(&other), Hash128::of(&w.events));
}

#[test]
fn an_edit_that_fills_a_cell_makes_room_for_whoever_stood_there() {
    let mut w = world();
    let (wall, pillar) = (w.wall, w.pillar);
    let rubble = w.defs.fixture_state("rubble", None).unwrap();
    let at = w.at(10, 10, 0);
    let walker = EntityId::new(1, 0);
    let coin = EntityId::new(2, 0);
    let lock = EntityId::new(3, 0);
    let mut ctx = w.ctx();
    ctx.place(
        Occupant::new(walker, OccupantKind::Creature),
        at,
        Footprint::SINGLE,
        Cause::DEBUG,
    )
    .unwrap();
    ctx.place(
        Occupant::new(coin, OccupantKind::Item),
        at,
        Footprint::SINGLE,
        Cause::DEBUG,
    )
    .unwrap();
    ctx.place(
        Occupant::new(lock, OccupantKind::Device),
        at,
        Footprint::SINGLE,
        Cause::DEBUG,
    )
    .unwrap();

    // Rubble is low enough to share a cell with: no one moves.
    ctx.edit(someone()).set_fixture(at, rubble).unwrap();
    assert_eq!(w.events.len(), 1);
    assert_eq!(w.cosmos.occupant(walker).unwrap().loc, at);

    // A pillar is not. The creature and the item go to the nearest cell
    // with room, which is the first of the ring around: north-west. The
    // device stays with its element. The edit itself is not refused.
    w.events.clear();
    w.ctx().edit(someone()).set_block(at, pillar).unwrap();
    let to = w.at(9, 9, 0);
    assert_eq!(
        w.events.drain(),
        [
            WorldEvent::ElementChanged {
                tick: Tick(0),
                loc: at,
                layer: Layer::Block,
                old: BlockState::NONE.raw(),
                new: pillar.raw(),
                cause: someone(),
            },
            WorldEvent::OccupantDisplaced {
                tick: Tick(0),
                occupant: walker,
                from: at,
                to,
                cause: someone(),
            },
            WorldEvent::OccupantDisplaced {
                tick: Tick(0),
                occupant: coin,
                from: at,
                to,
                cause: someone(),
            },
        ]
    );
    assert_eq!(w.cosmos.occupant(walker).unwrap().loc, to);
    assert_eq!(w.cosmos.occupant(lock).unwrap().loc, at);

    // Walled in on every side within reach, there is nowhere to go: the
    // occupant stays and is reported entombed.
    let mut ctx = w.ctx();
    let mut edit = ctx.edit(Cause::DEBUG);
    for y in 6..=12 {
        for x in 6..=12 {
            if (x, y) != (9, 9) {
                edit.set_block(Location::new(to.grid, CellPos::new(x, y, 0)), wall)
                    .unwrap();
            }
        }
    }
    w.events.clear();
    w.ctx().edit(someone()).set_block(to, wall).unwrap();
    let names: Vec<&str> = w.events.events().iter().map(WorldEvent::name).collect();
    assert_eq!(
        names,
        ["element_changed", "occupant_entombed", "occupant_entombed"]
    );
    assert_eq!(w.cosmos.occupant(walker).unwrap().loc, to);
}

#[derive(Clone, Debug)]
struct Step {
    x: i32,
    y: i32,
    z: i32,
    layer: u8,
    choice: u8,
}

fn step() -> impl Strategy<Value = Step> {
    (0..6_i32, 0..2_i32, -2..4_i32, 0..5_u8, 0..4_u8).prop_map(|(x, y, z, layer, choice)| Step {
        // Either side of the boundary between the two loaded tracts.
        x: CHUNK_XY - 3 + x,
        y,
        z,
        layer,
        choice,
    })
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(48))]

    /// Random edits: each that changes something emits exactly one event
    /// carrying what was there and what is there now, each that changes
    /// nothing emits none, and afterwards the world is what building the
    /// same cells from scratch gives, caches and all.
    #[test]
    fn edits_emit_one_event_each_and_keep_the_books(steps in prop::collection::vec(step(), 1..80)) {
        let mut w = world();
        let blocks = [BlockState::NONE, w.wall, w.rock, w.door_open];
        let floors = [FloorState::NONE, w.slab, w.grass, w.slab];
        let fixtures = [FixtureState::NONE, w.desk, FixtureState::NONE, w.desk];
        // The same cells, built outside the world.
        let defs = Arc::clone(&w.defs);
        let mut builders = [
            ColumnBuilder::new(&defs, w.rock, BlockState::NONE, 0),
            ColumnBuilder::new(&defs, w.rock, BlockState::NONE, 0),
        ];
        for builder in &mut builders {
            builder.fill_floor(0, w.grass);
        }

        for step in &steps {
            let at = w.at(step.x, step.y, step.z);
            let choice = usize::from(step.choice);
            let before = w.cosmos.cell(at).unwrap();
            let damage_before = w.cosmos.damage(at).unwrap();
            let rev_before = w.rev(at);
            let hash_before = Hash128::of(&w.cosmos);
            let builder = &mut builders[usize::try_from(step.x / CHUNK_XY).unwrap()];

            let mut ctx = w.ctx();
            let mut edit = ctx.edit(someone());
            let expected = match step.layer {
                0 => {
                    builder.set_floor(at.cell, floors[choice]);
                    let old = edit.set_floor(at, floors[choice]).unwrap();
                    prop_assert_eq!(old, before.floor);
                    (old != floors[choice]).then(|| (Layer::Floor, old.raw(), floors[choice].raw()))
                }
                1 | 2 => {
                    builder.set_block(at.cell, blocks[choice]);
                    let old = edit.set_block(at, blocks[choice]).unwrap();
                    prop_assert_eq!(old, before.block);
                    (old != blocks[choice]).then(|| (Layer::Block, old.raw(), blocks[choice].raw()))
                }
                3 => {
                    builder.set_fixture(at.cell, fixtures[choice]);
                    let old = edit.set_fixture(at, fixtures[choice]).unwrap();
                    prop_assert_eq!(old, before.fixture);
                    (old != fixtures[choice])
                        .then(|| (Layer::Fixture, old.raw(), fixtures[choice].raw()))
                }
                _ => {
                    // Damage to whatever block is there.
                    let amount = 40 * step.choice;
                    let old = edit.set_damage(at, Layer::Block, amount).unwrap();
                    prop_assert_eq!(old, damage_before[1]);
                    let events = w.events.drain();
                    if before.block.is_none() || old == amount {
                        prop_assert!(events.is_empty());
                        prop_assert_eq!(w.rev(at), rev_before);
                    } else {
                        prop_assert_eq!(events.len(), 1);
                        let reported = matches!(
                            events[0],
                            WorldEvent::ElementDamaged { old: was, new: is, .. }
                                if was == old && is == amount
                        );
                        prop_assert!(reported);
                    }
                    continue;
                }
            };
            let events = w.events.drain();
            match expected {
                Some((layer, old, new)) => {
                    prop_assert_eq!(
                        events,
                        vec![WorldEvent::ElementChanged {
                            tick: Tick(0),
                            loc: at,
                            layer,
                            old,
                            new,
                            cause: someone(),
                        }]
                    );
                    let (rev, dirty) = w.rev(at);
                    prop_assert!(dirty);
                    prop_assert_ne!(rev, rev_before.0);
                    // The replaced element's damage went with it.
                    prop_assert_eq!(w.cosmos.damage(at).unwrap()[layer.index()], 0);
                }
                None => {
                    prop_assert!(events.is_empty());
                    prop_assert_eq!(w.rev(at), rev_before);
                    prop_assert_eq!(Hash128::of(&w.cosmos), hash_before);
                }
            }
        }

        // What the world holds is what building the same cells gives.
        let view = w.cosmos.grid(w.grid).unwrap();
        for (tract, builder) in builders.iter().enumerate() {
            let tract = TractPos::new(i32::try_from(tract).unwrap(), 0);
            let edited = view.column(tract).unwrap();
            let built = builder.column();
            for y in 0..2 {
                for x in 0..CHUNK_XY {
                    for z in -4..6 {
                        let cell = CellPos::new(x, y, z);
                        prop_assert_eq!(edited.cell(cell), built.cell(cell));
                        prop_assert_eq!(edited.flags(cell), built.flags(cell));
                        prop_assert_eq!(edited.is_open(cell), built.is_open(cell));
                    }
                    let cell = CellPos::new(x, y, 0);
                    prop_assert_eq!(edited.height_at(cell), built.height_at(cell));
                }
            }
        }
    }
}
