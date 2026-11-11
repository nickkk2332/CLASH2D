//! Tests for W7, first part: the occupant index, `place`, `relocate` and
//! `remove`, box footprints, planes, and the queries at a cell, in a box,
//! within a radius and along a line.

mod common;

use std::collections::BTreeMap;
use std::collections::btree_map::Entry;
use std::sync::Arc;

use clash_core::{EntityId, Hash128, IVec3, Real};
use clash_time::Timeline;
use clash_world::dims::{REGION_CELLS, TRACT_XY};
use clash_world::iter::segment_cells;
use clash_world::test_support::{load_flat, unload_column};
use clash_world::{
    BlockState, Cause, CellBox, CellPos, Cosmos, EventSink, FOOTPRINT_MAX_SIDE, Footprint, GridId,
    Location, Occupant, OccupantFilter, OccupantFlags, OccupantKind, OccupantKinds, Occupied,
    PlaneId, PlaneMask, PointM, TractPos, WorldCtx, WorldError, WorldEvent, distance_sq_cells,
    distinct_ids,
};
use proptest::prelude::*;

/// How many cells round the test realm is, east to west.
const ROUND: i32 = 2 * REGION_CELLS;

struct World {
    timeline: Timeline,
    cosmos: Cosmos,
    events: EventSink<WorldEvent>,
    grid: GridId,
    wall: BlockState,
}

/// A cosmos with one `testland` realm and flat ground loaded either side
/// of the seam where the world wraps: tracts −1, 0 and 1 east to west,
/// rows 0 and 1.
fn world() -> World {
    let defs = common::test_defs();
    let mut timeline = Timeline::new();
    let mut cosmos = Cosmos::new(Arc::clone(&defs));
    let def = defs.realms().lookup("core:testland").unwrap();
    let realm = cosmos.create_realm(&mut timeline, def, 7).unwrap();
    let grid = cosmos.realm(realm).unwrap().base_grid;
    let rock = defs.block_state("solid", None).unwrap();
    let grass = defs.floor_state("grass", None).unwrap();
    let last = ROUND / TRACT_XY - 1;
    let tracts = [0, 1]
        .into_iter()
        .flat_map(|y| [last, 0, 1].map(|x| TractPos::new(x, y)));
    load_flat(&mut cosmos, grid, tracts, rock, grass, 0).unwrap();
    World {
        wall: defs.block_state("wall", None).unwrap(),
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

    fn place(&mut self, slot: u32, kind: OccupantKind, at: Location) -> EntityId {
        let id = EntityId::new(slot, 0);
        self.ctx()
            .place(Occupant::new(id, kind), at, Footprint::SINGLE, Cause::DEBUG)
            .unwrap();
        id
    }
}

fn ids(found: &[Occupied]) -> Vec<u32> {
    found.iter().map(|occupied| occupied.id.slot()).collect()
}

#[test]
fn place_relocate_and_remove_keep_the_index() {
    let mut w = world();
    let a = w.at(3, 3, 0);
    let b = w.at(40, 9, 0);
    let before = Hash128::of(&w.cosmos);
    let id = EntityId::new(5, 2);
    let who = Occupant::new(id, OccupantKind::Creature)
        .with_mass(Real::of(80.0))
        .with_flags(OccupantFlags::BLOCKS_MOVE | OccupantFlags::COLLIDES);

    w.ctx()
        .place(who, a, Footprint::SINGLE, Cause::DEBUG)
        .unwrap();
    let record = w.cosmos.occupant(id).unwrap();
    assert_eq!((record.occupant, record.loc), (who, a));
    assert_eq!(record.footprint, Footprint::SINGLE);
    assert_eq!(w.cosmos.occupant_count(), 1);
    assert_eq!(
        ids(&w.cosmos.occupants_at(a, &OccupantFilter::ANY).unwrap()),
        [5]
    );
    assert_eq!(w.cosmos.is_occupied(a, &OccupantFilter::ANY), Ok(true));
    assert_eq!(w.cosmos.is_occupied(b, &OccupantFilter::ANY), Ok(false));

    // Moving is the only way a position changes; it says where it was.
    assert_eq!(w.ctx().relocate(id, b, Cause::DEBUG), Ok(a));
    assert_eq!(w.cosmos.occupant(id).unwrap().loc, b);
    assert!(
        w.cosmos
            .occupants_at(a, &OccupantFilter::ANY)
            .unwrap()
            .is_empty()
    );
    assert_eq!(
        ids(&w.cosmos.occupants_at(b, &OccupantFilter::ANY).unwrap()),
        [5]
    );
    let found = w.cosmos.occupants_at(b, &OccupantFilter::ANY).unwrap()[0];
    assert_eq!(
        (found.cell, found.kind, found.plane, found.flags),
        (b.cell, who.kind, who.plane, who.flags)
    );

    // What the world knows about it can be replaced where it stands.
    let ghost = who
        .on(PlaneMask::of(PlaneId::new(1)))
        .with_flags(OccupantFlags::NONE);
    w.ctx().update_occupant(ghost).unwrap();
    assert_eq!(w.cosmos.occupant(id).unwrap().occupant, ghost);
    let on_default = OccupantFilter::ANY.on(PlaneMask::DEFAULT);
    assert_eq!(w.cosmos.is_occupied(b, &on_default), Ok(false));

    let record = w.ctx().remove(id, Cause::DEBUG).unwrap();
    assert_eq!((record.occupant, record.loc), (ghost, b));
    assert_eq!(w.cosmos.occupant_count(), 0);
    assert!(
        w.cosmos
            .occupants_at(b, &OccupantFilter::ANY)
            .unwrap()
            .is_empty()
    );

    // Who is where is an index, not state: none of this moved the hash,
    // and none of it emitted anything.
    assert_eq!(Hash128::of(&w.cosmos), before);
    assert!(w.events.is_empty());
}

#[test]
fn what_cannot_be_done_is_refused_with_a_reason_and_changes_nothing() {
    let mut w = world();
    let a = w.at(3, 3, 0);
    let id = w.place(1, OccupantKind::Creature, a);
    let other = Occupant::new(EntityId::new(2, 0), OccupantKind::Item);
    let stranger = EntityId::new(9, 9);
    let unloaded = w.at(5 * TRACT_XY, 3, 0);
    let outside = w.at(3, -1, 0);
    let nowhere = Location::new(GridId::new(8), CellPos::new(0, 0, 0));
    let wide = Footprint::new(FOOTPRINT_MAX_SIDE + 1, 1, 1);
    // A box that starts in a loaded tract and ends in one that is not.
    let edge = w.at(2 * TRACT_XY - 1, 3, 0);

    let mut ctx = w.ctx();
    assert_eq!(
        ctx.place(
            Occupant::new(id, OccupantKind::Item),
            a,
            Footprint::SINGLE,
            Cause::DEBUG
        ),
        Err(WorldError::AlreadyPlaced(id))
    );
    assert_eq!(
        ctx.place(other, unloaded, Footprint::SINGLE, Cause::DEBUG),
        Err(WorldError::NotLoaded(unloaded))
    );
    assert!(matches!(
        ctx.place(other, outside, Footprint::SINGLE, Cause::DEBUG),
        Err(WorldError::OutsideRealm { .. })
    ));
    assert_eq!(
        ctx.place(other, nowhere, Footprint::SINGLE, Cause::DEBUG),
        Err(WorldError::UnknownGrid(GridId::new(8)))
    );
    assert!(matches!(
        ctx.place(other, a, wide, Cause::DEBUG),
        Err(WorldError::TooLarge { .. })
    ));
    assert!(matches!(
        ctx.place(other, edge, Footprint::new(2, 1, 1), Cause::DEBUG),
        Err(WorldError::NotLoaded(_))
    ));
    assert_eq!(
        ctx.relocate(stranger, a, Cause::DEBUG),
        Err(WorldError::UnknownOccupant(stranger))
    );
    assert_eq!(
        ctx.relocate(id, unloaded, Cause::DEBUG),
        Err(WorldError::NotLoaded(unloaded))
    );
    assert!(matches!(
        ctx.relocate_as(id, edge, Footprint::new(2, 1, 1), Cause::DEBUG),
        Err(WorldError::NotLoaded(_))
    ));
    assert!(matches!(
        ctx.relocate(id, outside, Cause::DEBUG),
        Err(WorldError::OutsideRealm { .. })
    ));
    assert_eq!(
        ctx.remove(stranger, Cause::DEBUG).err(),
        Some(WorldError::UnknownOccupant(stranger))
    );
    assert_eq!(
        ctx.update_occupant(other),
        Err(WorldError::UnknownOccupant(other.id))
    );

    assert_eq!(w.cosmos.occupant_count(), 1);
    assert_eq!(w.cosmos.occupant(id).unwrap().loc, a);
    assert_eq!(
        ids(&w.cosmos.occupants_at(a, &OccupantFilter::ANY).unwrap()),
        [1]
    );
    assert!(
        w.cosmos
            .occupants_in_box(
                w.grid,
                CellBox::new(CellPos::new(-40, 0, -4), CellPos::new(70, 63, 4)),
                &OccupantFilter::ANY
            )
            .unwrap()
            .len()
            == 1
    );

    // Queries need nothing loaded: unloaded space and space outside the
    // realm hold no one. Only an unknown grid is an error.
    assert_eq!(
        w.cosmos.occupants_at(unloaded, &OccupantFilter::ANY),
        Ok(vec![])
    );
    assert_eq!(
        w.cosmos.is_occupied(outside, &OccupantFilter::ANY),
        Ok(false)
    );
    assert_eq!(
        w.cosmos.occupants_at(nowhere, &OccupantFilter::ANY),
        Err(WorldError::UnknownGrid(GridId::new(8)))
    );
    // A query too large to answer is refused.
    let huge = CellBox::new(CellPos::new(0, 0, 0), CellPos::new(i32::MAX, i32::MAX, 0));
    assert!(matches!(
        w.cosmos
            .occupants_in_box(w.grid, huge, &OccupantFilter::ANY),
        Err(WorldError::TooLarge { .. })
    ));
    assert!(matches!(
        w.cosmos
            .occupants_within(a, Real::INFINITY, &OccupantFilter::ANY),
        Err(WorldError::TooLarge { .. })
    ));
    assert!(matches!(
        w.cosmos.occupants_along(
            a.centre(),
            [Real::of(1.0e9), Real::ZERO, Real::ZERO],
            &OccupantFilter::ANY
        ),
        Err(WorldError::TooLarge { .. })
    ));
    assert_eq!(
        w.cosmos
            .occupants_within(a, Real::of(-1.0), &OccupantFilter::ANY),
        Ok(vec![])
    );
}

#[test]
fn a_box_footprint_fills_every_cell_of_it() {
    let mut w = world();
    let id = EntityId::new(4, 0);
    let cart = Occupant::new(id, OccupantKind::Other);
    // Across the seam where the world wraps, and across a tract boundary.
    let at = w.at(-1, TRACT_XY - 1, 0);
    w.ctx()
        .place(cart, at, Footprint::new(2, 2, 2), Cause::DEBUG)
        .unwrap();
    assert_eq!(
        w.cosmos.occupant(id).unwrap().loc.cell,
        CellPos::new(ROUND - 1, TRACT_XY - 1, 0)
    );
    let everywhere = CellBox::new(CellPos::new(-40, 0, -2), CellPos::new(40, 60, 5));
    let found = w
        .cosmos
        .occupants_in_box(w.grid, everywhere, &OccupantFilter::ANY)
        .unwrap();
    let cells: Vec<CellPos> = found.iter().map(|occupied| occupied.cell).collect();
    let mut expected = Vec::new();
    for z in 0..2 {
        for y in [TRACT_XY - 1, TRACT_XY] {
            for x in [0, ROUND - 1] {
                expected.push(CellPos::new(x, y, z));
            }
        }
    }
    assert_eq!(cells, expected);
    assert_eq!(distinct_ids(&found), [id]);
    for cell in &expected {
        assert_eq!(
            w.cosmos
                .is_occupied(Location::new(w.grid, *cell), &OccupantFilter::ANY),
            Ok(true)
        );
    }

    // Turned on its side it fills other cells, and the old ones are free.
    w.ctx()
        .relocate_as(id, at, Footprint::new(1, 3, 1), Cause::DEBUG)
        .unwrap();
    let found = w
        .cosmos
        .occupants_in_box(w.grid, everywhere, &OccupantFilter::ANY)
        .unwrap();
    let cells: Vec<CellPos> = found.iter().map(|occupied| occupied.cell).collect();
    assert_eq!(
        cells,
        [0, 1, 2].map(|dy| CellPos::new(ROUND - 1, TRACT_XY - 1 + dy, 0))
    );
    w.ctx().remove(id, Cause::DEBUG).unwrap();
    assert!(
        w.cosmos
            .occupants_in_box(w.grid, everywhere, &OccupantFilter::ANY)
            .unwrap()
            .is_empty()
    );
}

#[test]
fn occupants_on_disjoint_planes_do_not_see_each_other() {
    let mut w = world();
    let at = w.at(8, 8, 0);
    let astral = PlaneMask::of(PlaneId::new(1));
    let mirror = PlaneMask::of(PlaneId::new(2));
    let body = Occupant::new(EntityId::new(1, 0), OccupantKind::Creature);
    let ghost = Occupant::new(EntityId::new(2, 0), OccupantKind::Creature).on(astral);
    let medium = Occupant::new(EntityId::new(3, 0), OccupantKind::Creature)
        .on(PlaneMask::DEFAULT.union(astral));
    let nothing = Occupant::new(EntityId::new(4, 0), OccupantKind::Item).on(PlaneMask::NONE);
    for who in [body, ghost, medium, nothing] {
        w.ctx()
            .place(who, at, Footprint::SINGLE, Cause::DEBUG)
            .unwrap();
    }
    let around = CellBox::around(at.cell, IVec3::new(2, 2, 0));
    let line_end = [Real::of(12.5), Real::of(8.5), Real::of(1.0)];
    let line_start = PointM::new(w.grid, [Real::of(4.5), Real::of(8.5), Real::of(1.0)]);
    // What each sees, by every kind of query.
    let sees = |w: &World, who: &Occupant| {
        let filter = OccupantFilter::ANY.on(who.plane).except(who.id);
        let at_cell = ids(&w.cosmos.occupants_at(at, &filter).unwrap());
        let in_box = ids(&w.cosmos.occupants_in_box(w.grid, around, &filter).unwrap());
        let near = ids(&w
            .cosmos
            .occupants_within(at, Real::of(3.0), &filter)
            .unwrap());
        let along = ids(&w
            .cosmos
            .occupants_along(line_start, line_end, &filter)
            .unwrap());
        assert_eq!(at_cell, in_box);
        assert_eq!(at_cell, near);
        assert_eq!(at_cell, along);
        assert_eq!(
            w.cosmos.is_occupied(at, &filter).unwrap(),
            !at_cell.is_empty()
        );
        at_cell
    };
    assert_eq!(sees(&w, &body), [3]);
    assert_eq!(sees(&w, &ghost), [3]);
    assert_eq!(sees(&w, &medium), [1, 2]);
    assert!(sees(&w, &nothing).is_empty());
    // A plane no one is on is empty; asking without a plane finds all.
    let on_mirror = OccupantFilter::ANY.on(mirror);
    assert!(w.cosmos.occupants_at(at, &on_mirror).unwrap().is_empty());
    assert_eq!(
        ids(&w.cosmos.occupants_at(at, &OccupantFilter::ANY).unwrap()),
        [1, 2, 3, 4]
    );
    // Kinds and flags filter the same way.
    let items = OccupantFilter::ANY.of_kind(OccupantKind::Item);
    assert_eq!(ids(&w.cosmos.occupants_at(at, &items).unwrap()), [4]);
    let none = OccupantFilter::ANY.of_kinds(OccupantKinds::NONE);
    assert!(w.cosmos.occupants_at(at, &none).unwrap().is_empty());
    let solid = OccupantFilter::ANY.with_flags(OccupantFlags::BLOCKS_MOVE);
    assert!(w.cosmos.occupants_at(at, &solid).unwrap().is_empty());
}

#[test]
fn the_index_does_not_depend_on_the_order_things_arrived_in() {
    let people: Vec<(u32, i32, i32)> = vec![
        (9, 4, 4),
        (2, 4, 4),
        (7, 5, 4),
        (1, 4, 4),
        (5, -3, 40),
        (3, 4, 5),
    ];
    let build = |order: &[usize]| {
        let mut w = world();
        for at in order {
            let (slot, x, y) = people[*at];
            let loc = w.at(x, y, 0);
            w.place(slot, OccupantKind::Creature, loc);
        }
        // Shuffle some about and back again.
        for at in order {
            let (slot, x, y) = people[*at];
            let id = EntityId::new(slot, 0);
            let away = w.at(x + 9, y + 1, 0);
            let home = w.at(x, y, 0);
            w.ctx().relocate(id, away, Cause::DEBUG).unwrap();
            w.ctx().relocate(id, home, Cause::DEBUG).unwrap();
        }
        w
    };
    let one = build(&[0, 1, 2, 3, 4, 5]);
    let two = build(&[5, 3, 4, 0, 2, 1]);
    let everywhere = CellBox::new(CellPos::new(-40, 0, -2), CellPos::new(60, 60, 5));
    let found_one = one
        .cosmos
        .occupants_in_box(one.grid, everywhere, &OccupantFilter::ANY)
        .unwrap();
    let found_two = two
        .cosmos
        .occupants_in_box(two.grid, everywhere, &OccupantFilter::ANY)
        .unwrap();
    assert_eq!(found_one, found_two);
    // By cell, storey by storey and row by row, then by id.
    assert_eq!(ids(&found_one), [1, 2, 9, 7, 3, 5]);
    let column = |w: &World| {
        let view = w.cosmos.grid(w.grid).unwrap();
        view.column(TractPos::new(0, 0))
            .unwrap()
            .chunk(0)
            .unwrap()
            .occupants()
            .clone()
    };
    assert_eq!(column(&one), column(&two));
    assert_eq!(column(&one).len(), 5);
    let records = |w: &World| -> Vec<EntityId> {
        w.cosmos
            .occupants()
            .iter()
            .map(|record| record.occupant.id)
            .collect()
    };
    assert_eq!(records(&one), records(&two));
}

#[test]
fn an_occupant_above_the_stored_cells_needs_no_storage() {
    let mut w = world();
    let wall = w.wall;
    let high = w.at(6, 6, 150);
    let range = |w: &World| {
        let view = w.cosmos.grid(w.grid).unwrap();
        view.column(TractPos::new(0, 0)).unwrap().stored_range()
    };
    let stored = range(&w);
    let hash = Hash128::of(&w.cosmos);
    let id = w.place(1, OccupantKind::Creature, high);
    assert_eq!(range(&w), stored);
    assert_eq!(
        ids(&w.cosmos.occupants_at(high, &OccupantFilter::ANY).unwrap()),
        [1]
    );
    assert_eq!(
        ids(&w
            .cosmos
            .occupants_within(high, Real::of(2.0), &OccupantFilter::ANY)
            .unwrap()),
        [1]
    );
    let view = w.cosmos.grid(w.grid).unwrap();
    let column = view.column(TractPos::new(0, 0)).unwrap();
    assert!(column.has_occupants());
    let rev = column.occupants_rev();
    assert_eq!(Hash128::of(&w.cosmos), hash);

    // Building up there gives the height storage; whoever was there is
    // still found.
    let beside = w.at(7, 6, 150);
    w.ctx().edit(Cause::DEBUG).set_block(beside, wall).unwrap();
    assert_ne!(range(&w), stored);
    assert_eq!(
        ids(&w.cosmos.occupants_at(high, &OccupantFilter::ANY).unwrap()),
        [1]
    );
    let lower = w.at(6, 6, 149);
    w.ctx().relocate(id, lower, Cause::DEBUG).unwrap();
    assert!(
        w.cosmos
            .occupants_at(high, &OccupantFilter::ANY)
            .unwrap()
            .is_empty()
    );
    assert_eq!(
        ids(&w.cosmos.occupants_at(lower, &OccupantFilter::ANY).unwrap()),
        [1]
    );
    let view = w.cosmos.grid(w.grid).unwrap();
    let column = view.column(TractPos::new(0, 0)).unwrap();
    assert_ne!(column.occupants_rev(), rev);
    assert_ne!(
        column.chunk(lower.cell.chunk().z).unwrap().rev().occupants,
        0
    );

    // Dropping a whole column takes whoever stood in it out of the index.
    assert!(unload_column(&mut w.cosmos, w.grid, TractPos::new(0, 0)).is_some());
    assert_eq!(w.cosmos.occupant_count(), 0);
    assert!(w.cosmos.occupant(id).is_none());
}

/// What the test knows about one occupant.
#[derive(Clone, Debug)]
struct Known {
    occupant: Occupant,
    cells: Vec<CellPos>,
}

#[derive(Clone, Debug)]
enum Op {
    Place {
        slot: u32,
        at: (i32, i32, i32),
        size: (u8, u8, u8),
        planes: u32,
        kind: usize,
        flags: bool,
    },
    Move {
        slot: u32,
        at: (i32, i32, i32),
    },
    Remove {
        slot: u32,
    },
}

/// A position anywhere over the loaded tracts, at ground level, a little
/// above or below it, or far above the stored cells.
fn position() -> impl Strategy<Value = (i32, i32, i32)> {
    (
        -TRACT_XY..2 * TRACT_XY - 4,
        0..2 * TRACT_XY - 4,
        prop_oneof![-3..6_i32, 148..152_i32],
    )
}

fn op() -> impl Strategy<Value = Op> {
    let slot = 0..10_u32;
    prop_oneof![
        3 => (slot.clone(), position(), (1..4_u8, 1..4_u8, 1..3_u8), 0..8_u32, 0..4_usize, any::<bool>())
            .prop_map(|(slot, at, size, planes, kind, flags)| Op::Place { slot, at, size, planes, kind, flags }),
        3 => (slot.clone(), position()).prop_map(|(slot, at)| Op::Move { slot, at }),
        1 => slot.prop_map(|slot| Op::Remove { slot }),
    ]
}

#[derive(Clone, Debug)]
struct Ask {
    at: (i32, i32, i32),
    size: (i32, i32, i32),
    radius: u32,
    to: (i32, i32, i32),
    planes: Option<u32>,
    kind: Option<usize>,
    flags: bool,
    except: Option<u32>,
}

fn ask() -> impl Strategy<Value = Ask> {
    (
        position(),
        (0..70_i32, 0..40_i32, 0..5_i32),
        0..25_u32,
        position(),
        prop::option::of(0..8_u32),
        prop::option::of(0..4_usize),
        any::<bool>(),
        prop::option::of(0..10_u32),
    )
        .prop_map(|(at, size, radius, to, planes, kind, flags, except)| Ask {
            at,
            size,
            radius,
            to,
            planes,
            kind,
            flags,
            except,
        })
}

fn wrapped(cell: CellPos) -> CellPos {
    CellPos::new(cell.x.rem_euclid(ROUND), cell.y, cell.z)
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(64))]

    /// A random population, placed, moved and removed at random: every
    /// query answers what a plain list of who is where says.
    #[test]
    fn queries_match_a_brute_force_list(
        ops in prop::collection::vec(op(), 1..60),
        asks in prop::collection::vec(ask(), 1..12),
    ) {
        let mut w = world();
        let mut known: BTreeMap<EntityId, Known> = BTreeMap::new();
        let cells_of = |at: (i32, i32, i32), size: Footprint| -> Vec<CellPos> {
            size.cells(CellPos::new(at.0, at.1, at.2)).cells().map(wrapped).collect()
        };
        for op in &ops {
            match op.clone() {
                Op::Place { slot, at, size, planes, kind, flags } => {
                    let id = EntityId::new(slot, 0);
                    let mut occupant = Occupant::new(id, OccupantKind::ALL[kind])
                        .on(PlaneMask::from_bits(planes));
                    if flags {
                        occupant = occupant.with_flags(OccupantFlags::BLOCKS_MOVE);
                    }
                    let size = Footprint::new(size.0, size.1, size.2);
                    let loc = w.at(at.0, at.1, at.2);
                    let placed = w.ctx().place(occupant, loc, size, Cause::DEBUG);
                    match known.entry(id) {
                        Entry::Occupied(_) => {
                            prop_assert_eq!(placed, Err(WorldError::AlreadyPlaced(id)));
                        }
                        Entry::Vacant(free) => {
                            prop_assert_eq!(placed, Ok(()));
                            free.insert(Known { occupant, cells: cells_of(at, size) });
                        }
                    }
                }
                Op::Move { slot, at } => {
                    let id = EntityId::new(slot, 0);
                    let loc = w.at(at.0, at.1, at.2);
                    let moved = w.ctx().relocate(id, loc, Cause::DEBUG);
                    match known.get_mut(&id) {
                        Some(entry) => {
                            prop_assert!(moved.is_ok());
                            let size = w.cosmos.occupant(id).unwrap().footprint;
                            entry.cells = cells_of(at, size);
                        }
                        None => prop_assert_eq!(moved, Err(WorldError::UnknownOccupant(id))),
                    }
                }
                Op::Remove { slot } => {
                    let id = EntityId::new(slot, 0);
                    let removed = w.ctx().remove(id, Cause::DEBUG);
                    prop_assert_eq!(removed.is_ok(), known.remove(&id).is_some());
                }
            }
        }
        prop_assert_eq!(w.cosmos.occupant_count(), known.len());
        for (id, entry) in &known {
            let record = w.cosmos.occupant(*id).unwrap();
            prop_assert_eq!(record.occupant, entry.occupant);
            prop_assert_eq!(record.loc.cell, entry.cells[0]);
        }

        let topology = *w.cosmos.topology(w.grid).unwrap();
        for ask in &asks {
            let mut filter = OccupantFilter::ANY;
            if let Some(planes) = ask.planes {
                filter = filter.on(PlaneMask::from_bits(planes));
            }
            if let Some(kind) = ask.kind {
                filter = filter.of_kind(OccupantKind::ALL[kind]);
            }
            if ask.flags {
                filter = filter.with_flags(OccupantFlags::BLOCKS_MOVE);
            }
            if let Some(slot) = ask.except {
                filter = filter.except(EntityId::new(slot, 0));
            }
            let wanted = |entry: &Known| {
                ask.planes.is_none_or(|planes| planes & entry.occupant.plane.bits() != 0)
                    && ask.kind.is_none_or(|kind| OccupantKind::ALL[kind] == entry.occupant.kind)
                    && (!ask.flags || entry.occupant.flags.contains(OccupantFlags::BLOCKS_MOVE))
                    && ask.except != Some(entry.occupant.id.slot())
            };
            // Every (cell, occupant) pair there is that the filter lets
            // through and `covers` holds for, by cell and then by id.
            let listed = |covers: &dyn Fn(CellPos) -> bool| -> Vec<(CellPos, EntityId)> {
                let mut pairs: Vec<(CellPos, EntityId)> = known
                    .iter()
                    .filter(|(_, entry)| wanted(entry))
                    .flat_map(|(id, entry)| entry.cells.iter().map(|cell| (*cell, *id)))
                    .filter(|(cell, _)| covers(*cell))
                    .collect();
                pairs.sort();
                pairs
            };
            let pairs = |found: &[Occupied]| -> Vec<(CellPos, EntityId)> {
                found.iter().map(|occupied| (occupied.cell, occupied.id)).collect()
            };
            let here = CellPos::new(ask.at.0, ask.at.1, ask.at.2);

            // At a cell.
            let loc = w.at(here.x, here.y, here.z);
            let found = w.cosmos.occupants_at(loc, &filter).unwrap();
            let expected = listed(&|cell| cell == wrapped(here));
            prop_assert_eq!(pairs(&found), expected.clone());
            prop_assert_eq!(w.cosmos.is_occupied(loc, &filter).unwrap(), !expected.is_empty());
            for occupied in &found {
                let entry = &known[&occupied.id];
                prop_assert_eq!(
                    (occupied.kind, occupied.plane, occupied.flags),
                    (entry.occupant.kind, entry.occupant.plane, entry.occupant.flags)
                );
            }

            // In a box, which may reach across the seam.
            let far = CellPos::new(here.x + ask.size.0, here.y + ask.size.1, here.z + ask.size.2);
            let cells = CellBox::new(here, far);
            let found = w.cosmos.occupants_in_box(w.grid, cells, &filter).unwrap();
            let in_box = |cell: CellPos| {
                [cell.x - ROUND, cell.x, cell.x + ROUND]
                    .into_iter()
                    .any(|x| cells.contains(CellPos::new(x, cell.y, cell.z)))
            };
            prop_assert_eq!(pairs(&found), listed(&in_box));

            // Within a radius, the shorter way round.
            let radius = Real::from_i64(i64::from(ask.radius)) / Real::of(2.0);
            let found = w.cosmos.occupants_within(loc, radius, &filter).unwrap();
            let near = |cell: CellPos| {
                let offset = topology.delta(wrapped(here), cell);
                Real::from_i64(distance_sq_cells(offset)) <= radius * radius
            };
            prop_assert_eq!(pairs(&found), listed(&near));

            // Along a line: the cells in the order the line reaches them,
            // and by id within a cell.
            let from = loc.point_at(1500);
            let to = w.at(ask.to.0, ask.to.1, ask.to.2).point_at(700).pos;
            let found = w.cosmos.occupants_along(from, to, &filter).unwrap();
            let mut expected = Vec::new();
            for cell in segment_cells(from.pos, to) {
                expected.extend(listed(&|at| at == wrapped(cell)));
            }
            prop_assert_eq!(pairs(&found), expected);
        }
    }
}
