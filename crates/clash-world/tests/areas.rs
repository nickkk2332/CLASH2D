//! Tests for W7, second part: the area table with its per-tract lookup,
//! enter and exit events, clocks and ambient overrides, the `area_kind`
//! and `plane` definition types, enclosing a space, and the site table.

mod common;

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use clash_core::{EntityId, Hash128, IVec3, Real, Tick, Value, VarKey, VarMap, codec};
use clash_time::Timeline;
use clash_world::dims::{REGION_CELLS, TRACT_XY};
use clash_world::test_support::load_flat;
use clash_world::{
    AREA_MAX_CELLS, AmbientOverride, AreaAnchor, AreaDrift, AreaId, AreaKindId, AreaShape,
    AreaSpec, AreaTable, BlockState, Cause, CauseKind, CellBox, CellPos, Cosmos, Drift, EventSink,
    FloorState, Footprint, GravityOverride, GridId, Location, Occupant, OccupantFilter,
    OccupantKind, PlaneId, PlaneMask, RealmId, SiteDefId, SiteId, SiteSpec, SiteTable, TractPos,
    WorldCtx, WorldDefs, WorldError, WorldEvent, distance_sq_cells,
};
use proptest::prelude::*;

/// How many cells round the test realm is, east to west.
const ROUND: i32 = 2 * REGION_CELLS;

/// The base grid of the one realm these tests make.
const GRID: GridId = GridId::new(0);

fn at(x: i32, y: i32, z: i32) -> Location {
    Location::new(GRID, CellPos::new(x, y, z))
}

const EXTRA_DEFS: &str = r#"[
  { "type": "area_kind", "id": "stopped_time", "clock_rate": 0, "priority": 100 },
  { "type": "area_kind", "id": "vault",
    "tags": ["secure", "indoors", "secure"],
    "vars": { "alarm": false, "level": 3, "heat": 0.5, "owner": "bank" },
    "sealed": true, "leak": "0.01 K/s", "ambience": "vault_light",
    "rules": [ "lockdown",
               { "on": "area_entered", "do": [ { "set_var": { "alarm": true } } ] } ] },
  { "type": "plane", "id": "astral" },
  { "type": "plane", "id": "mirror", "tags": ["reflected"] },
  { "type": "floor", "id": "grating", "material": "steel", "thickness": 0.05,
    "flags": ["grate"] }
]"#;

fn test_defs() -> Arc<WorldDefs> {
    let mut raw = common::test_raw();
    common::add_json(&mut raw, "core", EXTRA_DEFS, "EXTRA_DEFS");
    Arc::new(WorldDefs::build(raw).unwrap())
}

struct World {
    defs: Arc<WorldDefs>,
    timeline: Timeline,
    cosmos: Cosmos,
    events: EventSink<WorldEvent>,
    realm: RealmId,
    grid: GridId,
    wall: BlockState,
    slab: FloorState,
}

/// A cosmos with one `testland` realm and flat ground loaded either side
/// of the seam where the world wraps: tracts −1, 0 and 1 east to west,
/// rows 0 and 1.
fn world() -> World {
    let defs = test_defs();
    let mut timeline = Timeline::new();
    let mut cosmos = Cosmos::new(Arc::clone(&defs));
    let def = defs.realms().lookup("core:testland").unwrap();
    let realm = cosmos.create_realm(&mut timeline, def, 7).unwrap();
    let grid = cosmos.realm(realm).unwrap().base_grid;
    assert_eq!(grid, GRID);
    let rock = defs.block_state("solid", None).unwrap();
    let grass = defs.floor_state("grass", None).unwrap();
    let last = ROUND / TRACT_XY - 1;
    let tracts = [0, 1]
        .into_iter()
        .flat_map(|y| [last, 0, 1].map(|x| TractPos::new(x, y)));
    load_flat(&mut cosmos, grid, tracts, rock, grass, 0).unwrap();
    World {
        wall: defs.block_state("wall", None).unwrap(),
        slab: defs.floor_state("slab", None).unwrap(),
        defs,
        timeline,
        cosmos,
        events: EventSink::new(),
        realm,
        grid,
    }
}

impl World {
    fn ctx(&mut self) -> WorldCtx<'_> {
        WorldCtx::new(&mut self.cosmos, &mut self.timeline, &mut self.events)
    }

    fn place(&mut self, slot: u32, at: Location, footprint: Footprint) -> EntityId {
        let id = EntityId::new(slot, 0);
        self.ctx()
            .place(
                Occupant::new(id, OccupantKind::Creature),
                at,
                footprint,
                Cause::DEBUG,
            )
            .unwrap();
        id
    }

    fn box_area(&mut self, min: (i32, i32, i32), max: (i32, i32, i32)) -> AreaId {
        let shape = AreaShape::Box(CellBox::new(
            CellPos::new(min.0, min.1, min.2),
            CellPos::new(max.0, max.1, max.2),
        ));
        let spec = AreaSpec::new(self.grid, shape);
        self.ctx().create_area(spec, Cause::DEBUG).unwrap()
    }

    fn kind(&self, id: &str) -> AreaKindId {
        self.defs.area_kinds().lookup(id).unwrap()
    }

    /// The area events waiting, as (entered, area, occupant slot), and
    /// the sink emptied.
    fn crossings(&mut self) -> Vec<(bool, AreaId, u32)> {
        self.events
            .drain()
            .into_iter()
            .filter_map(|event| match event {
                WorldEvent::AreaEntered { area, occupant, .. } => {
                    Some((true, area, occupant.slot()))
                }
                WorldEvent::AreaExited { area, occupant, .. } => {
                    Some((false, area, occupant.slot()))
                }
                _ => None,
            })
            .collect()
    }
}

fn mover() -> Cause {
    Cause::new(CauseKind::Action).by(EntityId::new(1, 0))
}

#[test]
fn area_kinds_and_planes_are_definitions() {
    let defs = test_defs();
    let vault = defs
        .area_kinds()
        .get(defs.area_kinds().lookup("core:vault").unwrap())
        .unwrap();
    assert_eq!(vault.tags, ["indoors", "secure"]);
    assert_eq!(vault.vars.get("alarm"), Some(&Value::Bool(false)));
    assert_eq!(vault.vars.get("level"), Some(&Value::Int(3)));
    assert_eq!(vault.vars.get("heat"), Some(&Value::Real(Real::of(0.5))));
    assert_eq!(vault.vars.get("owner"), Some(&Value::from("bank")));
    assert!(vault.sealed);
    assert_eq!(vault.leak, Some(serde_json::json!("0.01 K/s")));
    assert_eq!(vault.ambience.as_deref(), Some("vault_light"));
    assert_eq!((vault.clock_rate, vault.priority), (None, 0));
    // Rules are carried as written, for the rule runtime to read.
    assert_eq!(vault.rules.len(), 2);
    assert_eq!(vault.rules[0], serde_json::json!("lockdown"));
    assert_eq!(vault.rules[1]["on"], serde_json::json!("area_entered"));

    let stopped = defs
        .area_kinds()
        .get(defs.area_kinds().lookup("core:stopped_time").unwrap())
        .unwrap();
    assert_eq!(
        (stopped.clock_rate, stopped.priority),
        (Some(Real::ZERO), 100)
    );
    assert!(!stopped.sealed && stopped.rules.is_empty());

    // Plane 0 is built in; the rest are numbered in order of id, and a
    // plane's id is its bit in a mask.
    assert_eq!(
        defs.planes().lookup("engine:default"),
        Some(PlaneId::new(0))
    );
    let astral = defs.planes().lookup("core:astral").unwrap();
    let mirror = defs.planes().lookup("core:mirror").unwrap();
    assert_eq!((astral, mirror), (PlaneId::new(1), PlaneId::new(2)));
    assert_eq!(defs.planes().get(mirror).unwrap().tags, ["reflected"]);
    assert_eq!(PlaneMask::of(astral).bits(), 0b10);
    assert_eq!(PlaneMask::DEFAULT, PlaneMask::of(PlaneId::new(0)));
    assert!(PlaneMask::ALL.has(mirror));
    assert!(PlaneMask::of(PlaneId::new(40)).is_empty());
    assert_eq!(
        PlaneMask::DEFAULT.with(astral).without(PlaneId::new(0)),
        PlaneMask::of(astral)
    );

    // Mistakes are reported, each with where it is.
    let mut raw = common::test_raw();
    common::add_json(
        &mut raw,
        "bad",
        r#"[
          { "type": "area_kind", "id": "odd", "clock_rate": -1, "vars": { "list": [1, 2] } },
          { "type": "plane", "id": "Astral" }
        ]"#,
        "bad",
    );
    let errors = WorldDefs::build(raw).unwrap_err();
    let found: Vec<(&str, &str, &str)> = errors
        .iter()
        .map(|error| (error.kind, error.id.as_str(), error.path.as_str()))
        .collect();
    assert_eq!(
        found,
        [
            ("plane", "bad:Astral", "id"),
            ("area_kind", "bad:odd", "vars.list"),
            ("area_kind", "bad:odd", "clock_rate"),
        ]
    );
    assert!(
        common::try_add_json(
            &mut common::test_raw(),
            "bad",
            r#"[{ "type": "area_kind", "id": "odd", "colour": "red" }]"#
        )
        .is_err()
    );

    // There is a bit for thirty-two planes, counting the built-in one.
    let mut raw = common::test_raw();
    for plane in 0..32 {
        let text = format!(r#"[{{ "type": "plane", "id": "p{plane:02}" }}]"#);
        common::add_json(&mut raw, "many", &text, "planes");
    }
    let errors = WorldDefs::build(raw).unwrap_err();
    assert_eq!(errors.len(), 1);
    assert_eq!(
        (errors[0].kind, errors[0].id.as_str()),
        ("plane", "many:p31")
    );
}

#[test]
fn entering_and_leaving_emit_events_for_diagonal_moves() {
    let mut w = world();
    let vault = w.box_area((10, 10, 0), (14, 14, 0));
    let id = w.place(1, at(9, 9, 0), Footprint::SINGLE);
    assert!(w.events.is_empty());
    w.timeline.advance_to(Tick(60));

    // Diagonally in across the corner: one event, with everything in it.
    let inside = at(10, 10, 0);
    w.ctx().relocate(id, inside, mover()).unwrap();
    assert_eq!(
        w.events.drain(),
        [WorldEvent::AreaEntered {
            tick: Tick(60),
            area: vault,
            occupant: id,
            loc: inside,
            clock: None,
            cause: mover(),
        }]
    );
    assert_eq!(w.cosmos.occupant(id).unwrap().areas, [vault]);
    // About inside it, diagonally or not, or to where it already is:
    // nothing.
    for (x, y) in [(11, 11), (14, 10), (10, 14), (10, 14), (13, 13)] {
        let to = at(x, y, 0);
        w.ctx().relocate(id, to, mover()).unwrap();
    }
    assert!(w.events.is_empty());
    // Diagonally out past the far corner.
    let outside = at(15, 15, 0);
    w.ctx().relocate(id, at(14, 14, 0), mover()).unwrap();
    w.ctx().relocate(id, outside, mover()).unwrap();
    assert_eq!(
        w.events.drain(),
        [WorldEvent::AreaExited {
            tick: Tick(60),
            area: vault,
            occupant: id,
            loc: outside,
            clock: None,
            cause: mover(),
        }]
    );
    assert!(w.cosmos.occupant(id).unwrap().areas.is_empty());
    // A diagonal step that only brushes the corner from outside does not
    // enter: (15, 9) to (9, 15) the long way never touches the box.
    w.ctx().relocate(id, at(15, 9, 0), mover()).unwrap();
    w.ctx().relocate(id, at(9, 15, 0), mover()).unwrap();
    assert!(w.events.is_empty());
    // Up a storey from inside is out: the area is one storey.
    w.ctx().relocate(id, at(12, 12, 0), mover()).unwrap();
    w.ctx().relocate(id, at(12, 12, 1), mover()).unwrap();
    assert_eq!(w.crossings(), [(true, vault, 1), (false, vault, 1)]);

    // Being placed inside is entering, and being removed is leaving.
    let other = w.place(2, at(12, 12, 0), Footprint::SINGLE);
    w.ctx().remove(other, mover()).unwrap();
    assert_eq!(w.crossings(), [(true, vault, 2), (false, vault, 2)]);
}

#[test]
fn a_footprint_is_inside_when_any_cell_of_it_is() {
    let mut w = world();
    let vault = w.box_area((10, 10, 0), (14, 14, 0));
    let yard = w.box_area((14, 14, 0), (20, 20, 0));
    // A cart of 2 × 2 cells, north-west of the vault and clear of it.
    let cart = w.place(1, at(8, 8, 0), Footprint::new(2, 2, 1));
    assert!(w.crossings().is_empty());
    // One step diagonally: its far corner is now on the vault's corner.
    w.ctx().relocate(cart, at(9, 9, 0), mover()).unwrap();
    assert_eq!(w.crossings(), [(true, vault, 1)]);
    // Across the vault: still inside all the way.
    for (x, y) in [(10, 10), (12, 12), (13, 12)] {
        let to = at(x, y, 0);
        w.ctx().relocate(cart, to, mover()).unwrap();
    }
    assert!(w.crossings().is_empty());
    // Onto the corner the two areas share: in both.
    w.ctx().relocate(cart, at(13, 13, 0), mover()).unwrap();
    assert_eq!(w.crossings(), [(true, yard, 1)]);
    assert_eq!(w.cosmos.occupant(cart).unwrap().areas, [vault, yard]);
    // One more diagonal step: its near corner leaves the vault. Leaving
    // is reported before entering.
    w.ctx().relocate(cart, at(15, 15, 0), mover()).unwrap();
    assert_eq!(w.crossings(), [(false, vault, 1)]);
    // Turning on the spot can cross an edge too.
    w.ctx()
        .relocate_as(cart, at(15, 13, 0), Footprint::new(1, 1, 1), mover())
        .unwrap();
    assert_eq!(w.crossings(), [(false, yard, 1)]);
    w.ctx()
        .relocate_as(cart, at(15, 13, 0), Footprint::new(1, 3, 1), mover())
        .unwrap();
    assert_eq!(w.crossings(), [(true, yard, 1)]);
    // A jump from one area straight into the other: out, then in.
    w.ctx()
        .relocate_as(cart, at(11, 11, 0), Footprint::SINGLE, mover())
        .unwrap();
    assert_eq!(w.crossings(), [(false, yard, 1), (true, vault, 1)]);
}

#[test]
fn making_moving_and_removing_an_area_reports_who_it_covers() {
    let mut w = world();
    let a = w.place(3, at(5, 5, 0), Footprint::SINGLE);
    let b = w.place(1, at(7, 5, 0), Footprint::new(2, 1, 1));
    let c = w.place(2, at(20, 5, 0), Footprint::SINGLE);

    // Made over two of them: both enter, in id order.
    let area = w.box_area((4, 4, 0), (7, 6, 0));
    assert_eq!(w.crossings(), [(true, area, 1), (true, area, 3)]);
    let inside = w
        .cosmos
        .occupants_in_area(area, &OccupantFilter::ANY)
        .unwrap();
    let cells: Vec<(i32, u32)> = inside
        .iter()
        .map(|occupied| (occupied.cell.x, occupied.id.slot()))
        .collect();
    assert_eq!(cells, [(5, 3), (7, 1)]);
    assert_eq!(w.cosmos.areas_at(at(7, 6, 0)), Ok(vec![area]));
    assert_eq!(w.cosmos.areas_at(at(8, 6, 0)), Ok(vec![]));

    // Moved east: it leaves one behind and comes over another. Those it
    // leaves are reported first.
    w.ctx()
        .move_area(area, IVec3::new(13, 0, 0), mover())
        .unwrap();
    assert_eq!(
        w.crossings(),
        [(false, area, 1), (false, area, 3), (true, area, 2)]
    );
    assert_eq!(w.cosmos.occupant(c).unwrap().areas, [area]);
    assert!(w.cosmos.occupant(a).unwrap().areas.is_empty());
    assert!(w.cosmos.occupant(b).unwrap().areas.is_empty());
    // Moved by nothing, or given the shape it has: nothing.
    w.ctx().move_area(area, IVec3::ZERO, mover()).unwrap();
    let same = w.cosmos.area(area).unwrap().shape.clone();
    w.ctx().set_area_shape(area, same, mover()).unwrap();
    assert!(w.events.is_empty());
    // Reshaped to a radius that reaches all three.
    let wide = AreaShape::Radius {
        centre: CellPos::new(12, 5, 0),
        radius: Real::of(8.0),
    };
    w.ctx().set_area_shape(area, wide, mover()).unwrap();
    assert_eq!(w.crossings(), [(true, area, 1), (true, area, 3)]);
    // Removed: everyone inside leaves.
    let removed = w.ctx().remove_area(area, mover()).unwrap();
    assert_eq!(removed.id, area);
    assert_eq!(
        w.crossings(),
        [(false, area, 1), (false, area, 2), (false, area, 3)]
    );
    assert!(w.cosmos.area(area).is_none());
    assert!(w.cosmos.occupant(c).unwrap().areas.is_empty());
    assert!(w.cosmos.area_table().is_empty());
    assert_eq!(
        w.ctx().remove_area(area, mover()).err(),
        Some(WorldError::UnknownArea(area))
    );
}

#[test]
fn an_anchored_area_moves_with_its_anchor() {
    let mut w = world();
    let hero = w.place(1, at(2, 20, 0), Footprint::SINGLE);
    let bystander = w.place(2, at(6, 20, 0), Footprint::SINGLE);
    let far = w.place(3, at(-4, 21, 0), Footprint::SINGLE);
    let mut spec = AreaSpec::new(
        w.grid,
        AreaShape::Radius {
            centre: CellPos::new(2, 20, 0),
            radius: Real::of(3.0),
        },
    );
    spec.anchor = Some(AreaAnchor::Entity(hero));
    let aura = w.ctx().create_area(spec, Cause::DEBUG).unwrap();
    assert_eq!(w.crossings(), [(true, aura, 1)]);
    assert_eq!(w.cosmos.area_table().anchored_to(hero), [aura]);

    // A step east brings the bystander inside. The hero, who carries the
    // aura, neither leaves nor enters.
    w.ctx().relocate(hero, at(3, 20, 0), mover()).unwrap();
    assert_eq!(w.crossings(), [(true, aura, 2)]);
    let centre = |w: &World| w.cosmos.area(aura).unwrap().shape.centre().unwrap();
    assert_eq!(centre(&w), CellPos::new(3, 20, 0));
    // Across the seam where the world wraps, the short way.
    w.ctx().relocate(hero, at(-2, 20, 0), mover()).unwrap();
    assert_eq!(w.crossings(), [(false, aura, 2), (true, aura, 3)]);
    assert_eq!(centre(&w), CellPos::new(ROUND - 2, 20, 0));
    assert_eq!(w.cosmos.areas_at(at(0, 20, 0)), Ok(vec![aura]));
    assert_eq!(
        w.cosmos
            .area_table()
            .in_tract(w.grid, TractPos::new(ROUND / TRACT_XY - 1, 0)),
        [aura]
    );
    // A leap far away: the aura goes too.
    w.ctx().relocate(hero, at(40, 40, 0), mover()).unwrap();
    assert_eq!(w.crossings(), [(false, aura, 3)]);
    assert_eq!(w.cosmos.occupant(hero).unwrap().areas, [aura]);
    // Let go of, it stays where it is.
    w.cosmos.set_area_anchor(aura, None).unwrap();
    w.ctx().relocate(hero, at(50, 40, 0), mover()).unwrap();
    assert_eq!(w.crossings(), [(false, aura, 1)]);
    assert_eq!(centre(&w), CellPos::new(40, 40, 0));
    let _ = (bystander, far);
}

#[test]
fn moving_to_another_grid_leaves_the_areas_of_the_first() {
    let mut w = world();
    let def = w.defs.realms().lookup("core:training_room").unwrap();
    let room = w.cosmos.create_realm(&mut w.timeline, def, 3).unwrap();
    let other = w.cosmos.realm(room).unwrap().base_grid;
    let rock = w.defs.block_state("solid", None).unwrap();
    load_flat(
        &mut w.cosmos,
        other,
        [TractPos::new(0, 0)],
        rock,
        FloorState::NONE,
        0,
    )
    .unwrap();

    let street = w.box_area((0, 0, 0), (20, 20, 0));
    let hero = w.place(1, at(5, 5, 0), Footprint::new(2, 1, 1));
    let friend = w.place(2, at(6, 6, 0), Footprint::SINGLE);
    let mut spec = AreaSpec::new(
        GRID,
        AreaShape::Radius {
            centre: CellPos::new(5, 5, 0),
            radius: Real::of(2.0),
        },
    );
    spec.anchor = Some(AreaAnchor::Entity(hero));
    let aura = w.ctx().create_area(spec, Cause::DEBUG).unwrap();
    // The same coordinates in the other grid are another place.
    let there = Location::new(other, CellPos::new(9, 9, 0));
    let mut spec = AreaSpec::new(other, AreaShape::Box(CellBox::single(there.cell)));
    spec.tags = vec!["arrival".to_owned()];
    let arrival = w.ctx().create_area(spec, Cause::DEBUG).unwrap();
    w.events.clear();

    assert_eq!(w.ctx().relocate(hero, there, mover()), Ok(at(5, 5, 0)));
    // The friend is no longer in the aura, which went with the hero; the
    // hero has left the street and arrived.
    assert_eq!(
        w.crossings(),
        [(false, aura, 2), (false, street, 1), (true, arrival, 1)]
    );
    let record = w.cosmos.occupant(hero).unwrap();
    assert_eq!(
        (record.loc, record.areas.as_slice()),
        (there, &[aura, arrival][..])
    );
    let moved = w.cosmos.area(aura).unwrap();
    assert_eq!(
        (moved.grid, moved.shape.centre()),
        (other, Some(there.cell))
    );
    assert_eq!(w.cosmos.areas_at(there), Ok(vec![aura, arrival]));
    assert_eq!(w.cosmos.areas_at(at(9, 9, 0)), Ok(vec![street]));
    assert_eq!(w.cosmos.areas_at(at(5, 5, 0)), Ok(vec![street]));
    let any = OccupantFilter::ANY;
    assert!(w.cosmos.occupants_at(at(5, 5, 0), &any).unwrap().is_empty());
    assert!(w.cosmos.occupants_at(at(6, 5, 0), &any).unwrap().is_empty());
    assert_eq!(w.cosmos.occupants_at(there, &any).unwrap().len(), 1);
    assert_eq!(
        w.cosmos
            .occupants_at(there.offset(IVec3::new(1, 0, 0)), &any)
            .unwrap()
            .len(),
        1
    );
    assert_eq!(w.cosmos.occupant(friend).unwrap().areas, [street]);
}

#[test]
fn an_area_with_a_clock_is_where_time_runs_differently() {
    let mut w = world();
    let realm_clock = w.cosmos.realm(w.realm).unwrap().clock;
    let mut spec = AreaSpec::new(
        w.grid,
        AreaShape::Radius {
            centre: CellPos::new(30, 30, 0),
            radius: Real::of(4.0),
        },
    );
    spec.kind = Some(w.kind("core:stopped_time"));
    let bubble = w.ctx().create_area(spec, Cause::DEBUG).unwrap();
    let area = w.cosmos.area(bubble).unwrap();
    let clock = area.clock.unwrap();
    assert_eq!(area.priority, 100);
    assert_eq!(w.timeline.rate(clock), Some(Real::ZERO));
    assert_eq!(w.timeline.parent(clock), Some(realm_clock));

    // Inside, things run on the area's clock; outside, on the realm's.
    let inside = at(31, 31, 0);
    let outside = at(36, 30, 0);
    assert_eq!(w.cosmos.ambient(inside).unwrap().clock, clock);
    assert_eq!(w.cosmos.ambient(outside).unwrap().clock, realm_clock);
    // Entering and leaving say which clock, so that whoever schedules
    // for the occupant can move it across.
    let id = w.place(1, outside, Footprint::SINGLE);
    w.ctx().relocate(id, inside, mover()).unwrap();
    assert!(matches!(
        w.events.drain()[..],
        [WorldEvent::AreaEntered { clock: Some(on), .. }] if on == clock
    ));

    // A slower area of lower priority over the same place does not win.
    let mut spec = AreaSpec::new(w.grid, AreaShape::Box(CellBox::single(inside.cell)));
    spec.clock_rate = Some(Real::of(0.5));
    let slow = w.ctx().create_area(spec, Cause::DEBUG).unwrap();
    let slow_clock = w.cosmos.area(slow).unwrap().clock.unwrap();
    assert_eq!(w.cosmos.ambient(inside).unwrap().clock, clock);
    w.cosmos.set_area_priority(slow, 200).unwrap();
    assert_eq!(w.cosmos.ambient(inside).unwrap().clock, slow_clock);

    // The rate can be changed, and the clock given up or taken on.
    w.ctx()
        .set_area_clock_rate(bubble, Some(Real::of(2.0)))
        .unwrap();
    assert_eq!(w.timeline.rate(clock), Some(Real::of(2.0)));
    w.ctx().set_area_clock_rate(slow, None).unwrap();
    assert!(!w.timeline.has_clock(slow_clock));
    assert_eq!(w.cosmos.area(slow).unwrap().clock, None);
    assert_eq!(w.cosmos.ambient(inside).unwrap().clock, clock);
    w.ctx()
        .set_area_clock_rate(slow, Some(Real::of(3.0)))
        .unwrap();
    assert!(w.cosmos.area(slow).unwrap().clock.is_some());
    assert!(matches!(
        w.ctx().set_area_clock_rate(slow, Some(Real::of(-1.0))),
        Err(WorldError::Time(_))
    ));

    // Removing the area destroys its clock.
    w.ctx().remove_area(bubble, Cause::DEBUG).unwrap();
    assert!(!w.timeline.has_clock(clock));
    w.ctx().remove_area(slow, Cause::DEBUG).unwrap();
    assert_eq!(w.cosmos.ambient(inside).unwrap().clock, realm_clock);
}

#[test]
fn an_ambient_override_sets_gravity_and_drift() {
    let mut w = world();
    let here = at(20, 40, 0);
    let plain = w.cosmos.ambient(here).unwrap();
    assert_eq!(plain.gravity, Real::of(9.81));
    assert_eq!(
        plain.gravity_vector,
        [Real::ZERO, Real::ZERO, Real::of(-9.81)]
    );
    assert_eq!(plain.drift, None);

    // Half gravity and a steady wind, in a box.
    let wind = Drift::Flow([Real::of(4.0), Real::of(-1.0), Real::ZERO]);
    let mut spec = AreaSpec::new(
        w.grid,
        AreaShape::Box(CellBox::new(
            CellPos::new(16, 36, 0),
            CellPos::new(24, 44, 3),
        )),
    );
    spec.ambient = Some(AmbientOverride {
        gravity: Some(GravityOverride::Scale(Real::of(0.5))),
        drift: Some(wind),
    });
    let gusty = w.ctx().create_area(spec, Cause::DEBUG).unwrap();
    let inside = w.cosmos.ambient(here).unwrap();
    assert_eq!(inside.gravity, Real::of(9.81) * Real::of(0.5));
    assert_eq!(inside.gravity_vector[2], -inside.gravity);
    assert_eq!(
        inside.drift,
        Some(AreaDrift {
            area: gusty,
            drift: wind,
            centre: CellPos::new(20, 40, 1),
        })
    );
    assert_eq!(
        (inside.atmosphere, inside.clock),
        (plain.atmosphere, plain.clock)
    );
    assert_eq!(w.cosmos.ambient(at(25, 40, 0)).unwrap(), plain);
    assert_eq!(w.cosmos.ambient(at(20, 40, 4)).unwrap(), plain);

    // A whirlwind inside it: a vortex, of higher priority, that sets no
    // gravity. Its drift applies and the box's gravity still does.
    let whirl = Drift::Vortex {
        speed: Real::of(20.0),
        inflow: Real::of(3.0),
        lift: Real::of(5.0),
    };
    let mut spec = AreaSpec::new(
        w.grid,
        AreaShape::Radius {
            centre: CellPos::new(21, 40, 0),
            radius: Real::of(2.0),
        },
    );
    spec.ambient = Some(AmbientOverride {
        gravity: None,
        drift: Some(whirl),
    });
    spec.priority = Some(5);
    let vortex = w.ctx().create_area(spec, Cause::DEBUG).unwrap();
    let both = w.cosmos.ambient(here).unwrap();
    assert_eq!(both.gravity, inside.gravity);
    assert_eq!(
        both.drift,
        Some(AreaDrift {
            area: vortex,
            drift: whirl,
            centre: CellPos::new(21, 40, 0),
        })
    );
    // Of two of the same priority, the one made first applies.
    w.cosmos.set_area_priority(vortex, 0).unwrap();
    assert_eq!(w.cosmos.ambient(here).unwrap().drift.unwrap().area, gusty);

    // Gravity as a vector: sideways, here. The scalar is what is left
    // along −z.
    let sideways = [Real::of(9.0), Real::ZERO, Real::of(-1.5)];
    w.cosmos
        .set_area_ambient(
            vortex,
            Some(AmbientOverride {
                gravity: Some(GravityOverride::Vector(sideways)),
                drift: None,
            }),
        )
        .unwrap();
    w.cosmos.set_area_priority(vortex, 9).unwrap();
    let pulled = w.cosmos.ambient(here).unwrap();
    assert_eq!(pulled.gravity_vector, sideways);
    assert_eq!(pulled.gravity, Real::of(1.5));
    assert_eq!(pulled.drift.unwrap().area, gusty);

    // Ambient conditions need nothing loaded, and neither do areas.
    let unloaded = AreaShape::Box(CellBox::single(CellPos::new(900, 900, 0)));
    let mut spec = AreaSpec::new(w.grid, unloaded);
    spec.ambient = Some(AmbientOverride {
        gravity: Some(GravityOverride::Scale(Real::ZERO)),
        drift: None,
    });
    w.ctx().create_area(spec, Cause::DEBUG).unwrap();
    assert_eq!(
        w.cosmos.ambient(at(900, 900, 0)).unwrap().gravity,
        Real::ZERO
    );
}

#[test]
fn areas_take_defaults_from_their_kind_and_are_found_by_tract() {
    let mut w = world();
    let vault_kind = w.kind("core:vault");
    let mut spec = AreaSpec::new(
        w.grid,
        AreaShape::Box(CellBox::new(
            CellPos::new(TRACT_XY - 2, 30, 0),
            CellPos::new(TRACT_XY + 1, TRACT_XY + 1, 0),
        )),
    );
    spec.kind = Some(vault_kind);
    spec.tags = vec!["wet".to_owned(), "indoors".to_owned()];
    spec.vars = VarMap::from_iter([("level", 7), ("crew", 2)]);
    let vault = w.ctx().create_area(spec, Cause::DEBUG).unwrap();
    let area = w.cosmos.area(vault).unwrap();
    assert_eq!(area.kind, Some(vault_kind));
    assert_eq!(area.tags, ["indoors", "secure", "wet"]);
    assert!(area.has_tag("secure") && !area.has_tag("dry"));
    assert_eq!(area.vars.get("level"), Some(&Value::Int(7)));
    assert_eq!(area.vars.get("crew"), Some(&Value::Int(2)));
    assert_eq!(area.vars.get("owner"), Some(&Value::from("bank")));
    assert_eq!(
        (area.clock, area.priority, area.is_breached()),
        (None, 0, false)
    );

    // It is listed for each of the four tracts it touches, and for no
    // other.
    let table = w.cosmos.area_table();
    for (x, y) in [(0, 0), (1, 0), (0, 1), (1, 1)] {
        assert_eq!(table.in_tract(w.grid, TractPos::new(x, y)), [vault]);
    }
    assert!(table.in_tract(w.grid, TractPos::new(2, 0)).is_empty());
    assert!(
        table
            .in_tract(GridId::new(5), TractPos::new(0, 0))
            .is_empty()
    );
    assert_eq!(
        w.cosmos.areas_at(at(TRACT_XY + 1, TRACT_XY + 1, 0)),
        Ok(vec![vault])
    );
    assert_eq!(w.cosmos.areas_at(at(TRACT_XY + 2, 30, 0)), Ok(vec![]));
    assert_eq!(w.cosmos.areas().count(), 1);

    // Tags and variables change in place.
    assert_eq!(w.cosmos.set_area_tag(vault, "wet", false), Ok(true));
    assert_eq!(w.cosmos.set_area_tag(vault, "wet", false), Ok(false));
    assert_eq!(w.cosmos.set_area_tag(vault, "loud", true), Ok(true));
    assert_eq!(
        w.cosmos
            .set_area_var(vault, VarKey::new("alarm"), Some(Value::from(true))),
        Ok(Some(Value::Bool(false)))
    );
    assert_eq!(
        w.cosmos.set_area_var(vault, VarKey::new("crew"), None),
        Ok(Some(Value::Int(2)))
    );
    let area = w.cosmos.area(vault).unwrap();
    assert_eq!(area.tags, ["indoors", "loud", "secure"]);
    assert_eq!(area.vars.get("crew"), None);

    // What cannot be made is refused with a reason.
    let ghost = AreaId::new(99);
    let shape = || AreaShape::Box(CellBox::single(CellPos::new(1, 1, 0)));
    let mut ctx = w.ctx();
    let mut spec = AreaSpec::new(GridId::new(5), shape());
    assert_eq!(
        ctx.create_area(spec.clone(), Cause::DEBUG),
        Err(WorldError::UnknownGrid(GridId::new(5)))
    );
    spec.grid = ctx.cosmos.area(vault).unwrap().grid;
    spec.kind = Some(AreaKindId::new(77));
    assert_eq!(
        ctx.create_area(spec.clone(), Cause::DEBUG),
        Err(WorldError::UnknownAreaKind(AreaKindId::new(77)))
    );
    spec.kind = None;
    spec.shape = AreaShape::Radius {
        centre: CellPos::new(1, 1, 0),
        radius: Real::of(-2.0),
    };
    assert!(matches!(
        ctx.create_area(spec.clone(), Cause::DEBUG),
        Err(WorldError::BadValue { .. })
    ));
    spec.shape = AreaShape::Radius {
        centre: CellPos::new(1, 1, 0),
        radius: Real::of(5000.0),
    };
    assert!(matches!(
        ctx.create_area(spec.clone(), Cause::DEBUG),
        Err(WorldError::TooLarge { .. })
    ));
    spec.shape = AreaShape::Box(CellBox::single(CellPos::new(1, -1, 0)));
    assert!(matches!(
        ctx.create_area(spec.clone(), Cause::DEBUG),
        Err(WorldError::OutsideRealm { .. })
    ));
    spec.shape = shape();
    spec.clock_rate = Some(Real::of(-1.0));
    assert!(matches!(
        ctx.create_area(spec.clone(), Cause::DEBUG),
        Err(WorldError::Time(_))
    ));
    assert_eq!(
        ctx.move_area(ghost, IVec3::new(1, 0, 0), Cause::DEBUG),
        Err(WorldError::UnknownArea(ghost))
    );
    assert!(matches!(
        ctx.move_area(vault, IVec3::new(0, -40, 0), Cause::DEBUG),
        Err(WorldError::OutsideRealm { .. })
    ));
    assert_eq!(
        ctx.cosmos.set_area_tag(ghost, "x", true),
        Err(WorldError::UnknownArea(ghost))
    );
    assert_eq!(ctx.cosmos.areas().count(), 1);
    assert!(ctx.events.is_empty());

    // The table is state: it hashes by what the areas are, and survives
    // the binary codec with its lists.
    let table = w.cosmos.area_table();
    let bytes = codec::to_bytes(table).unwrap();
    let back: AreaTable = codec::from_bytes(&bytes).unwrap();
    assert_eq!(&back, table);
    assert_eq!(Hash128::of(&back), Hash128::of(table));
    let before = Hash128::of(&w.cosmos);
    w.cosmos.set_area_tag(vault, "loud", false).unwrap();
    assert_ne!(Hash128::of(&w.cosmos), before);
}

/// Builds a room: walls around the cells from `min` to `max` on storey
/// 0, and a slab over all of it at the base of storey 1. Returns the
/// cells inside.
fn build_room(w: &mut World, min: (i32, i32), max: (i32, i32)) -> BTreeSet<CellPos> {
    let (wall, slab, grid) = (w.wall, w.slab, w.grid);
    let mut inside = BTreeSet::new();
    let mut ctx = w.ctx();
    let mut edit = ctx.edit(Cause::GENERATOR);
    for y in min.1 - 1..=max.1 + 1 {
        for x in min.0 - 1..=max.0 + 1 {
            let on_wall = x < min.0 || x > max.0 || y < min.1 || y > max.1;
            if on_wall {
                edit.set_block(Location::new(grid, CellPos::new(x, y, 0)), wall)
                    .unwrap();
            } else {
                inside.insert(CellPos::new(x, y, 0));
            }
            edit.set_floor(Location::new(grid, CellPos::new(x, y, 1)), slab)
                .unwrap();
        }
    }
    inside
}

fn cells_of(w: &World, area: AreaId) -> BTreeSet<CellPos> {
    match &w.cosmos.area(area).unwrap().shape {
        AreaShape::Cells(cells) => cells.iter().copied().collect(),
        other => panic!("an enclosed area lists its cells, not {other:?}"),
    }
}

#[test]
fn enclosing_a_closed_room_covers_exactly_its_open_cells() {
    let mut w = world();
    let mut inside = build_room(&mut w, (11, 11), (15, 14));
    // Furniture does not stop the fill, nor does a pillar, which covers
    // half its cell. A cell of solid rock is not part of the space.
    let desk = w.defs.fixture_state("desk", None).unwrap();
    let pillar = w.defs.block_state("pillar", None).unwrap();
    let rock = w.defs.block_state("solid", None).unwrap();
    let mut ctx = w.ctx();
    let mut edit = ctx.edit(Cause::GENERATOR);
    edit.set_fixture(at(11, 11, 0), desk).unwrap();
    edit.set_block(at(14, 11, 0), pillar).unwrap();
    edit.set_block(at(15, 14, 0), rock).unwrap();
    inside.remove(&CellPos::new(15, 14, 0));
    let guard = w.place(1, at(13, 13, 0), Footprint::SINGLE);
    let passer = w.place(2, at(20, 13, 0), Footprint::SINGLE);
    w.events.clear();

    let from = at(12, 12, 0);
    assert_eq!(
        w.cosmos.enclosed_cells(from, 100).unwrap(),
        inside.iter().copied().collect::<Vec<_>>()
    );
    let vault_kind = w.kind("core:vault");
    let room = w
        .ctx()
        .enclose(from, Some(vault_kind), 100, mover())
        .unwrap();
    assert_eq!(cells_of(&w, room), inside);
    assert_eq!(inside.len(), 19);
    let area = w.cosmos.area(room).unwrap();
    assert_eq!(area.kind, Some(vault_kind));
    assert!(area.has_tag("secure"));
    let made = area.enclosure.unwrap();
    assert_eq!(
        (made.from, made.limit, made.breached),
        (from.cell, 100, false)
    );
    assert!(!area.is_breached());
    // Whoever is in the room has entered the area; no one else has.
    assert_eq!(w.crossings(), [(true, room, 1)]);
    assert_eq!(w.cosmos.areas_at(at(11, 11, 0)), Ok(vec![room]));
    assert_eq!(w.cosmos.areas_at(at(10, 11, 0)), Ok(vec![]));
    assert_eq!(w.cosmos.areas_at(at(15, 14, 0)), Ok(vec![]));
    assert_eq!(w.cosmos.areas_at(at(12, 12, 1)), Ok(vec![]));
    // A space of exactly the limit closes; one cell less does not.
    assert_eq!(w.cosmos.enclosed_cells(from, 19).unwrap().len(), 19);
    assert!(matches!(
        w.cosmos.enclosed_cells(from, 18),
        Err(WorldError::NotEnclosed { .. })
    ));
    let _ = (guard, passer);
}

#[test]
fn enclosing_an_open_room_is_refused_and_names_a_leak() {
    let mut w = world();
    build_room(&mut w, (31, 11), (33, 13));
    // A gap in the east wall.
    let gap = at(34, 12, 0);
    w.ctx()
        .edit(Cause::GENERATOR)
        .set_block(gap, BlockState::NONE)
        .unwrap();
    w.events.clear();
    let from = at(32, 12, 0);
    // The leak is the first cell under open sky that the fill reaches:
    // the one just outside the gap, which the roof still covers.
    let leak = at(35, 12, 0);
    assert_eq!(
        w.ctx().enclose(from, None, 100, mover()),
        Err(WorldError::NotEnclosed { leak })
    );
    assert_eq!(
        w.cosmos.enclosed_cells(from, 100),
        Err(WorldError::NotEnclosed { leak })
    );
    assert!(w.cosmos.area_table().is_empty());
    assert!(w.events.is_empty());

    // Out in the open the leak is where one stands. Inside a wall there
    // is no space at all. A limit beyond the engine's is refused.
    let field = at(50, 50, 0);
    assert_eq!(
        w.ctx().enclose(field, None, 30, mover()),
        Err(WorldError::NotEnclosed { leak: field })
    );
    let in_wall = at(30, 12, 0);
    assert_eq!(
        w.ctx().enclose(in_wall, None, 30, mover()),
        Err(WorldError::NothingToEnclose(in_wall))
    );
    assert!(matches!(
        w.ctx().enclose(from, None, AREA_MAX_CELLS + 1, mover()),
        Err(WorldError::TooLarge { .. })
    ));
    assert_eq!(
        w.ctx().enclose(from, None, 0, mover()),
        Err(WorldError::NotEnclosed { leak: from })
    );
    // A space that runs into what is not loaded cannot be judged.
    let edge = at(2 * TRACT_XY - 1, 5, 0);
    assert!(matches!(
        w.ctx().enclose(edge, None, 5000, mover()),
        Err(WorldError::NotLoaded(_))
    ));
    let unloaded = at(3 * TRACT_XY, 5, 0);
    assert_eq!(
        w.ctx().enclose(unloaded, None, 10, mover()),
        Err(WorldError::NotLoaded(unloaded))
    );
}

#[test]
fn knocking_through_a_wall_breaches_the_area_and_mending_it_closes_it() {
    let mut w = world();
    let inside = build_room(&mut w, (11, 11), (15, 14));
    let from = at(12, 12, 0);
    let room = w.ctx().enclose(from, None, 100, Cause::DEBUG).unwrap();
    let wall = w.wall;
    w.events.clear();
    w.timeline.advance_to(Tick(500));
    let breached = |w: &World| w.cosmos.area(room).unwrap().is_breached();

    // Out goes a cell of the east wall.
    let hole = at(16, 12, 0);
    let leak = at(17, 12, 0);
    w.ctx()
        .edit(mover())
        .set_block(hole, BlockState::NONE)
        .unwrap();
    assert!(breached(&w));
    assert_eq!(
        w.cosmos.area(room).unwrap().enclosure.unwrap().leak,
        Some(leak.cell)
    );
    let events = w.events.drain();
    assert_eq!(events.len(), 2);
    assert_eq!(events[0].name(), "element_changed");
    assert_eq!(
        events[1],
        WorldEvent::AreaBreached {
            tick: Tick(500),
            area: room,
            leak,
            cause: mover(),
        }
    );
    // A breached area keeps the shape it last had.
    assert_eq!(cells_of(&w, room), inside);
    // A second hole changes nothing more: it is already open.
    let second = at(16, 13, 0);
    w.ctx()
        .edit(mover())
        .set_block(second, BlockState::NONE)
        .unwrap();
    assert_eq!(w.events.drain().len(), 1);
    // Mending one of two holes does not close it.
    w.ctx().edit(mover()).set_block(second, wall).unwrap();
    assert!(breached(&w));
    assert_eq!(w.events.drain().len(), 1);

    // Mending the other does.
    w.ctx().edit(mover()).set_block(hole, wall).unwrap();
    assert!(!breached(&w));
    assert_eq!(w.cosmos.area(room).unwrap().enclosure.unwrap().leak, None);
    let events = w.events.drain();
    assert_eq!(events.len(), 2);
    assert_eq!(
        events[1],
        WorldEvent::AreaClosed {
            tick: Tick(500),
            area: room,
            loc: from,
            cause: mover(),
        }
    );
    assert_eq!(cells_of(&w, room), inside);

    // A door in the wall: closed it holds, open it does not.
    let door = w.defs.block_state("door", None).unwrap();
    let door_open = w.defs.block_state("door#open", None).unwrap();
    let doorway = at(10, 12, 0);
    w.ctx().edit(mover()).set_block(doorway, door).unwrap();
    assert!(!breached(&w));
    assert_eq!(w.events.drain().len(), 1);
    w.ctx().edit(mover()).set_block(doorway, door_open).unwrap();
    assert!(breached(&w));
    w.ctx().edit(mover()).set_block(doorway, door).unwrap();
    assert!(!breached(&w));
    let names: Vec<&str> = w.events.events().iter().map(WorldEvent::name).collect();
    assert_eq!(
        names,
        [
            "element_changed",
            "area_breached",
            "element_changed",
            "area_closed"
        ]
    );
    w.events.clear();

    // The roof: a slab taken off opens it to the sky, a grating laid in
    // its place does not close it, and a slab does.
    let grating = w.defs.floor_state("grating", None).unwrap();
    let slab = w.slab;
    let above = at(13, 12, 1);
    w.ctx()
        .edit(mover())
        .set_floor(above, FloorState::NONE)
        .unwrap();
    assert!(breached(&w));
    // The leak is the cell under the hole: it is under open sky now.
    assert_eq!(
        w.cosmos.area(room).unwrap().enclosure.unwrap().leak,
        Some(CellPos::new(13, 12, 0))
    );
    w.ctx().edit(mover()).set_floor(above, grating).unwrap();
    assert!(breached(&w));
    w.ctx().edit(mover()).set_floor(above, slab).unwrap();
    assert!(!breached(&w));
    // Things that do not matter to the fill ask nothing of the area.
    w.events.clear();
    let desk = w.defs.fixture_state("desk", None).unwrap();
    w.ctx().edit(mover()).set_fixture(from, desk).unwrap();
    w.ctx()
        .edit(mover())
        .set_damage(hole, clash_world::Layer::Block, 30)
        .unwrap();
    assert_eq!(w.events.drain().len(), 2);
}

#[test]
fn an_enclosed_area_follows_what_is_built_inside_it() {
    let mut w = world();
    let inside = build_room(&mut w, (11, 11), (15, 14));
    let from = at(12, 12, 0);
    let room = w.ctx().enclose(from, None, 100, Cause::DEBUG).unwrap();
    let wall = w.wall;
    let east = w.place(1, at(15, 13, 0), Footprint::SINGLE);
    let west = w.place(2, at(11, 13, 0), Footprint::SINGLE);
    w.events.clear();

    // A partition goes up across the room, cell by cell. Each cell of it
    // stops being part of the space.
    for y in 11..=13 {
        let at = at(13, y, 0);
        w.ctx().edit(mover()).set_block(at, wall).unwrap();
    }
    let mut expected = inside.clone();
    for y in 11..=13 {
        expected.remove(&CellPos::new(13, y, 0));
    }
    assert_eq!(cells_of(&w, room), expected);
    assert!(w.crossings().is_empty());
    // The last cell cuts the room in two. The area is the part the space
    // was filled from, and whoever is on the far side has left it.
    let last = at(13, 14, 0);
    w.ctx().edit(mover()).set_block(last, wall).unwrap();
    expected.retain(|cell| cell.x < 13);
    assert_eq!(cells_of(&w, room), expected);
    assert_eq!(w.crossings(), [(false, room, 1)]);
    assert_eq!(w.cosmos.occupant(west).unwrap().areas, [room]);
    assert!(!w.cosmos.area(room).unwrap().is_breached());

    // Taking a cell of the partition out again joins the halves.
    w.ctx()
        .edit(mover())
        .set_block(last, BlockState::NONE)
        .unwrap();
    assert_eq!(w.crossings(), [(true, room, 1)]);
    let mut joined = inside.clone();
    for y in 11..=13 {
        joined.remove(&CellPos::new(13, y, 0));
    }
    assert_eq!(cells_of(&w, room), joined);
    // Walling up the cell it was filled from: it is worked out from
    // another of its cells.
    w.ctx().edit(mover()).set_block(from, wall).unwrap();
    joined.remove(&from.cell);
    assert_eq!(cells_of(&w, room), joined);
    assert!(!w.cosmos.area(room).unwrap().is_breached());
    let _ = east;
}

#[test]
fn sites_give_places_identity() {
    let mut w = world();
    let realm = w.realm;
    let spec = |name: &str, tracts: &[(i32, i32)]| SiteSpec {
        def: SiteDefId::new(4),
        name: name.to_owned(),
        realm,
        tracts: tracts.iter().map(|(x, y)| TractPos::new(*x, *y)).collect(),
        anchor: CellPos::new(40, 8, 0),
        vars: VarMap::from_iter([("vault_open", false)]),
        tags: vec!["money".to_owned(), "guarded".to_owned()],
    };
    let bank = w
        .cosmos
        .create_site(spec("First Bank", &[(1, 1), (1, 0), (2, 0)]))
        .unwrap();
    let site = w.cosmos.site(bank).unwrap();
    assert_eq!(site.name, "First Bank");
    assert_eq!(
        site.tracts,
        [
            TractPos::new(1, 0),
            TractPos::new(2, 0),
            TractPos::new(1, 1)
        ]
    );
    assert_eq!(site.tags, ["guarded", "money"]);
    assert!(site.has_tag("money"));
    // The world maps tracts to the site, loaded or not.
    assert_eq!(w.cosmos.site_at(realm, TractPos::new(2, 0)), Some(bank));
    assert_eq!(w.cosmos.site_at(realm, TractPos::new(0, 0)), None);
    assert_eq!(w.cosmos.site_of(at(TRACT_XY + 3, 5, 0)), Ok(Some(bank)));
    assert_eq!(w.cosmos.site_of(at(2 * TRACT_XY + 3, 5, 9)), Ok(Some(bank)));
    assert_eq!(w.cosmos.site_of(at(3, 5, 0)), Ok(None));
    assert!(w.cosmos.site_of(at(3, -5, 0)).is_err());

    // A tract belongs to one site.
    let lair = w.cosmos.create_site(spec("Lair", &[(5, 5)])).unwrap();
    assert_eq!(
        w.cosmos.create_site(spec("Annex", &[(9, 9), (2, 0)])),
        Err(WorldError::TractHasSite {
            realm,
            tract: TractPos::new(2, 0),
            site: bank,
        })
    );
    assert_eq!(w.cosmos.site_at(realm, TractPos::new(9, 9)), None);
    assert_eq!(
        w.cosmos.claim_site_tract(lair, TractPos::new(1, 1)),
        Err(WorldError::TractHasSite {
            realm,
            tract: TractPos::new(1, 1),
            site: bank,
        })
    );
    assert_eq!(
        w.cosmos.claim_site_tract(lair, TractPos::new(5, 6)),
        Ok(true)
    );
    assert_eq!(
        w.cosmos.claim_site_tract(lair, TractPos::new(5, 6)),
        Ok(false)
    );
    assert_eq!(
        w.cosmos.release_site_tract(bank, TractPos::new(2, 0)),
        Ok(true)
    );
    assert_eq!(
        w.cosmos.release_site_tract(bank, TractPos::new(2, 0)),
        Ok(false)
    );
    assert_eq!(
        w.cosmos.claim_site_tract(lair, TractPos::new(2, 0)),
        Ok(true)
    );
    assert_eq!(w.cosmos.site_at(realm, TractPos::new(2, 0)), Some(lair));
    let mut other_realm = spec("Nowhere", &[(0, 0)]);
    other_realm.realm = RealmId::new(9);
    assert_eq!(
        w.cosmos.create_site(other_realm),
        Err(WorldError::UnknownRealm(RealmId::new(9)))
    );

    // Names, tags and variables change in place.
    w.cosmos.set_site_name(bank, "Old Bank".to_owned()).unwrap();
    assert_eq!(w.cosmos.set_site_tag(bank, "money", false), Ok(true));
    assert_eq!(w.cosmos.set_site_tag(bank, "ruin", true), Ok(true));
    assert_eq!(
        w.cosmos
            .set_site_var(bank, VarKey::new("vault_open"), Some(Value::from(true))),
        Ok(Some(Value::Bool(false)))
    );
    let site = w.cosmos.site(bank).unwrap();
    assert_eq!(site.name, "Old Bank");
    assert_eq!(site.tags, ["guarded", "ruin"]);
    assert_eq!(site.vars.get("vault_open"), Some(&Value::Bool(true)));

    // An area can name its site, and an unknown site is refused.
    let mut area = AreaSpec::new(
        w.grid,
        AreaShape::Box(CellBox::single(CellPos::new(40, 8, 0))),
    );
    area.site = Some(bank);
    let lobby = w.ctx().create_area(area.clone(), Cause::DEBUG).unwrap();
    assert_eq!(w.cosmos.area(lobby).unwrap().site, Some(bank));
    area.site = Some(SiteId::new(77));
    assert_eq!(
        w.ctx().create_area(area, Cause::DEBUG),
        Err(WorldError::UnknownSite(SiteId::new(77)))
    );

    // The table is state: it hashes, and survives the binary codec.
    let table = w.cosmos.site_table();
    assert_eq!(table.len(), 2);
    let back: SiteTable = codec::from_bytes(&codec::to_bytes(table).unwrap()).unwrap();
    assert_eq!(&back, table);
    assert_eq!(Hash128::of(&back), Hash128::of(table));
    let before = Hash128::of(&w.cosmos);
    let removed = w.cosmos.remove_site(lair).unwrap();
    assert_eq!(removed.name, "Lair");
    assert_ne!(Hash128::of(&w.cosmos), before);
    assert_eq!(w.cosmos.site_at(realm, TractPos::new(5, 5)), None);
    assert_eq!(w.cosmos.sites().count(), 1);
    assert_eq!(
        w.cosmos.remove_site(lair).err(),
        Some(WorldError::UnknownSite(lair))
    );
}

/// What the test knows about an area: a box or a radius.
#[derive(Clone, Debug)]
enum Model {
    Box(CellBox),
    Radius(CellPos, i32),
}

impl Model {
    fn shape(&self) -> AreaShape {
        match self {
            Model::Box(cells) => AreaShape::Box(*cells),
            Model::Radius(centre, radius) => AreaShape::Radius {
                centre: *centre,
                radius: Real::from_i64(i64::from(*radius)),
            },
        }
    }

    /// Whether a normalised cell is inside, worked out the long way.
    fn holds(&self, cell: CellPos) -> bool {
        match self {
            Model::Box(cells) => [cell.x - ROUND, cell.x, cell.x + ROUND]
                .into_iter()
                .any(|x| cells.contains(CellPos::new(x, cell.y, cell.z))),
            Model::Radius(centre, radius) => {
                let across = (cell.x - centre.x).rem_euclid(ROUND);
                let dx = across.min(ROUND - across);
                let offset = IVec3::new(dx, cell.y - centre.y, cell.z - centre.z);
                distance_sq_cells(offset) <= i64::from(*radius) * i64::from(*radius)
            }
        }
    }

    fn moved(&self, by: IVec3) -> Model {
        match self {
            Model::Box(cells) => Model::Box(cells.offset(by)),
            Model::Radius(centre, radius) => {
                let to = centre.offset(by);
                Model::Radius(CellPos::new(to.x.rem_euclid(ROUND), to.y, to.z), *radius)
            }
        }
    }
}

#[derive(Clone, Debug)]
enum Op {
    Walk { who: usize, by: (i32, i32, i32) },
    Jump { who: usize, to: (i32, i32) },
    Turn { who: usize, size: (u8, u8) },
    Drift { area: usize, by: (i32, i32) },
}

fn op() -> impl Strategy<Value = Op> {
    prop_oneof![
        5 => (0..4_usize, (-1..=1_i32, -1..=1_i32, -1..=1_i32))
            .prop_map(|(who, by)| Op::Walk { who, by }),
        1 => (0..4_usize, (-12..40_i32, 4..40_i32)).prop_map(|(who, to)| Op::Jump { who, to }),
        1 => (0..4_usize, (1..4_u8, 1..4_u8)).prop_map(|(who, size)| Op::Turn { who, size }),
        2 => (0..3_usize, (-2..=2_i32, -2..=2_i32)).prop_map(|(area, by)| Op::Drift { area, by }),
    ]
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(64))]

    /// Occupants with footprints of several cells walk, jump and turn
    /// among areas that themselves move, across the seam where the world
    /// wraps. After every step the events are exactly the changes in who
    /// is inside what, leavings before enterings, and each occupant's
    /// record lists the areas it is inside.
    #[test]
    fn enter_and_exit_events_match_who_is_inside(
        starts in prop::collection::vec(((-10..30_i32, 6..30_i32), (1..3_u8, 1..3_u8)), 4),
        ops in prop::collection::vec(op(), 1..80),
    ) {
        let mut w = world();
        let mut models = vec![
            Model::Box(CellBox::new(CellPos::new(-4, 10, 0), CellPos::new(5, 16, 0))),
            Model::Radius(CellPos::new(12, 14, 0), 4),
            Model::Box(CellBox::new(CellPos::new(3, 12, 0), CellPos::new(14, 20, 1))),
        ];
        let mut areas = Vec::new();
        for model in &mut models {
            let spec = AreaSpec::new(w.grid, model.shape());
            areas.push(w.ctx().create_area(spec, Cause::DEBUG).unwrap());
            // The world keeps a box with its lowest corner normalised.
            if let Model::Box(cells) = model {
                let shift = IVec3::new(cells.min.x.rem_euclid(ROUND) - cells.min.x, 0, 0);
                *model = Model::Box(cells.offset(shift));
            }
        }
        // Where each occupant is, and its footprint.
        let mut people: Vec<(CellPos, Footprint)> = Vec::new();
        for (slot, ((x, y), (dx, dy))) in starts.iter().enumerate() {
            let size = Footprint::new(*dx, *dy, 1);
            let start = CellPos::new(*x, *y, 0);
            w.place(u32::try_from(slot).unwrap(), Location::new(GRID, start), size);
            people.push((start, size));
        }
        let inside = |models: &[Model], people: &[(CellPos, Footprint)]| {
            let mut set: BTreeMap<(usize, usize), bool> = BTreeMap::new();
            for (who, (here, size)) in people.iter().enumerate() {
                for (area, model) in models.iter().enumerate() {
                    let any = size.cells(*here).cells().any(|cell| {
                        model.holds(CellPos::new(cell.x.rem_euclid(ROUND), cell.y, cell.z))
                    });
                    set.insert((who, area), any);
                }
            }
            set
        };
        let mut was = inside(&models, &people);
        w.events.clear();

        for op in &ops {
            // Whether the step is an occupant's or an area's decides the
            // order events come in.
            let by_area = matches!(op, Op::Drift { .. });
            match op.clone() {
                Op::Walk { who, by } => {
                    let (from, size) = people[who];
                    let at = from;
                    let to = CellPos::new(
                        (at.x + by.0).clamp(-30, 55),
                        (at.y + by.1).clamp(0, 50),
                        (at.z + by.2).clamp(0, 2),
                    );
                    w.ctx()
                        .relocate(EntityId::new(who as u32, 0), Location::new(GRID, to), mover())
                        .unwrap();
                    people[who] = (to, size);
                }
                Op::Jump { who, to } => {
                    let to = CellPos::new(to.0, to.1, 0);
                    w.ctx()
                        .relocate(EntityId::new(who as u32, 0), Location::new(GRID, to), mover())
                        .unwrap();
                    people[who].0 = to;
                }
                Op::Turn { who, size } => {
                    let size = Footprint::new(size.0, size.1, 1);
                    let here = people[who].0;
                    w.ctx()
                        .relocate_as(EntityId::new(who as u32, 0), Location::new(GRID, here), size, mover())
                        .unwrap();
                    people[who].1 = size;
                }
                Op::Drift { area, by } => {
                    let by = IVec3::new(by.0, by.1, 0);
                    let moved = models[area].moved(by);
                    let fits = match &moved {
                        Model::Box(cells) => cells.min.y >= 0,
                        Model::Radius(centre, _) => centre.y >= 0,
                    };
                    let done = w.ctx().move_area(areas[area], by, mover());
                    prop_assert_eq!(done.is_ok(), fits);
                    if fits {
                        models[area] = moved;
                    }
                }
            }
            let now = inside(&models, &people);
            let mut left = Vec::new();
            let mut came = Vec::new();
            for (key, is) in &now {
                match (was[key], *is) {
                    (true, false) => left.push((false, areas[key.1], key.0 as u32)),
                    (false, true) => came.push((true, areas[key.1], key.0 as u32)),
                    _ => {}
                }
            }
            if by_area {
                // One area, many occupants: by occupant.
                left.sort_by_key(|event| event.2);
                came.sort_by_key(|event| event.2);
            } else {
                // One occupant, many areas: by area.
                left.sort_by_key(|event| event.1);
                came.sort_by_key(|event| event.1);
            }
            left.extend(came);
            prop_assert_eq!(w.crossings(), left, "after {:?}", op);
            for (who, _) in people.iter().enumerate() {
                let listed = &w.cosmos.occupant(EntityId::new(who as u32, 0)).unwrap().areas;
                let expected: Vec<AreaId> = (0..areas.len())
                    .filter(|area| now[&(who, *area)])
                    .map(|area| areas[area])
                    .collect();
                prop_assert_eq!(listed, &expected);
            }
            was = now;
        }
    }
}
