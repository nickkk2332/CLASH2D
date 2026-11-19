//! Tests for W9: tickets, activity levels, the warm margin, hysteresis,
//! reconciliation in the specified order, and the budget (02 §10).

mod common;

use std::collections::BTreeSet;
use std::sync::Arc;

use clash_core::{EntityId, Hash128, Tick};
use clash_time::{ProcessId, Timeline};
use clash_world::test_support::{flat_column, load_column};
use clash_world::{
    Activity, BlockState, Cause, CellPos, Cosmos, EventSink, FloorState, Footprint, Generators,
    GridId, HOT_RADIUS, LoadError, Location, MemoryStore, Occupant, OccupantKind, PINNED, RealmId,
    Reconciled, Ticket, TicketAnchor, TicketReason, TractIo, TractPos, UNLOAD_MARGIN, WARM_MARGIN,
    WorldCtx, WorldDefs, WorldError, WorldEvent,
};

/// A world of flat ground with its generators and a store, and nothing
/// loaded.
struct World {
    defs: Arc<WorldDefs>,
    timeline: Timeline,
    cosmos: Cosmos,
    events: EventSink<WorldEvent>,
    generators: Generators,
    store: MemoryStore,
    realm: RealmId,
    grid: GridId,
}

fn world() -> World {
    let defs = common::core_defs();
    let mut timeline = Timeline::new();
    let mut cosmos = Cosmos::new(Arc::clone(&defs));
    let def = common::realm_def(&defs, common::FLAT_REALM);
    let realm = cosmos.create_realm(&mut timeline, def, 9).unwrap();
    let grid = cosmos.realm(realm).unwrap().base_grid;
    World {
        generators: clash_gen::generators(&defs),
        defs,
        timeline,
        cosmos,
        events: EventSink::new(),
        store: MemoryStore::new(),
        realm,
        grid,
    }
}

impl World {
    /// One reconciliation, as the simulation makes at a safe point.
    fn reconcile(&mut self) -> Reconciled {
        let mut io = TractIo::new(&self.generators, &mut self.store);
        WorldCtx::new(&mut self.cosmos, &mut self.timeline, &mut self.events)
            .reconcile_activity(&mut io)
    }

    fn ctx(&mut self) -> WorldCtx<'_> {
        WorldCtx::new(&mut self.cosmos, &mut self.timeline, &mut self.events)
    }

    fn loaded(&self) -> BTreeSet<TractPos> {
        self.cosmos.loaded_tracts(self.realm).into_iter().collect()
    }

    fn warm(&self, at: TractPos, radius: u8) -> Ticket {
        Ticket::new(
            self.realm,
            TicketAnchor::Fixed(at),
            radius,
            Activity::Warm,
            TicketReason::Debug,
        )
    }

    /// The middle cell of a tract at the surface.
    fn middle(&self, tract: TractPos) -> Location {
        let origin = tract.origin(0);
        Location::new(self.grid, CellPos::new(origin.x + 16, origin.y + 16, 0))
    }
}

/// The tracts within `reach` of a tract, the longer way along either
/// axis. Far enough from the realm's edges that none is cut off.
fn square(centre: TractPos, reach: i32) -> BTreeSet<TractPos> {
    (-reach..=reach)
        .flat_map(|dy| (-reach..=reach).map(move |dx| TractPos::new(centre.x + dx, centre.y + dy)))
        .collect()
}

fn names(events: &[WorldEvent]) -> Vec<&'static str> {
    events.iter().map(WorldEvent::name).collect()
}

#[test]
fn a_ticket_loads_its_tracts_nearest_first_and_its_release_unloads_them() {
    let mut w = world();
    let centre = TractPos::new(20, 20);
    let ticket = w.cosmos.hold_ticket(w.warm(centre, 2)).unwrap();
    // Holding a ticket loads nothing: reconciliation does.
    assert!(w.loaded().is_empty());
    let done = w.reconcile();
    assert_eq!(w.loaded(), square(centre, 2));
    assert_eq!(done.loaded.len(), 25);
    assert!(done.unloaded.is_empty() && done.refused.is_empty() && done.failed.is_empty());
    // Nearest the anchor first, then row by row.
    let order: Vec<TractPos> = done.loaded.iter().map(|loaded| loaded.tract).collect();
    assert_eq!(order[0], centre);
    let ring = |tract: &TractPos| (tract.x - centre.x).abs().max((tract.y - centre.y).abs());
    assert!(
        order
            .windows(2)
            .all(|pair| { (ring(&pair[0]), pair[0]) < (ring(&pair[1]), pair[1]) })
    );
    assert_eq!(names(&w.events.drain()), vec!["tract_loaded"; 25]);
    assert_eq!(w.cosmos.activity(w.realm, centre), Some(Activity::Warm));
    assert_eq!(w.cosmos.tickets().kept_count(), 25);

    // Nothing changed: the next reconciliation does nothing.
    assert!(w.reconcile().is_empty());
    assert!(w.events.is_empty());

    // Released, its tracts go, row by row.
    assert_eq!(w.cosmos.release_ticket(ticket), Some(w.warm(centre, 2)));
    let done = w.reconcile();
    assert!(w.loaded().is_empty());
    let gone: Vec<TractPos> = done.unloaded.iter().map(|(_, tract)| *tract).collect();
    assert_eq!(gone, square(centre, 2).into_iter().collect::<Vec<_>>());
    assert_eq!(names(&w.events.drain()), vec!["tract_unloading"; 25]);
    assert_eq!(w.cosmos.activity(w.realm, centre), None);
    // Flat ground nobody touched is not stored.
    assert!(w.store.is_empty());
}

#[test]
fn a_hot_ticket_keeps_a_warm_margin() {
    let mut w = world();
    let centre = TractPos::new(30, 30);
    let ticket = Ticket::new(
        w.realm,
        TicketAnchor::Fixed(centre),
        1,
        Activity::Hot,
        TicketReason::Debug,
    );
    w.cosmos.hold_ticket(ticket).unwrap();
    w.reconcile();
    let reach = 1 + i32::from(WARM_MARGIN);
    assert_eq!(w.loaded(), square(centre, reach));
    for tract in square(centre, reach) {
        let hot = square(centre, 1).contains(&tract);
        let level = if hot { Activity::Hot } else { Activity::Warm };
        assert_eq!(w.cosmos.activity(w.realm, tract), Some(level), "{tract}");
    }
    assert_eq!(w.cosmos.activity(w.realm, TractPos::new(34, 30)), None);
    // The controlled character's ticket loads seventeen tracts square.
    let player = Ticket::player(w.realm, EntityId::new(1, 0));
    assert_eq!(
        player.reach(),
        i32::from(HOT_RADIUS) + i32::from(WARM_MARGIN)
    );
    assert_eq!(player.priority, PINNED);
    assert_eq!((2 * player.reach() + 1).pow(2), 289);
}

#[test]
fn a_moving_anchor_loads_and_unloads_exactly_the_expected_tracts() {
    let mut w = world();
    let radius = 2;
    let keep = radius + i32::from(UNLOAD_MARGIN);
    let mut at = TractPos::new(40, 30);
    let ticket = w.cosmos.hold_ticket(w.warm(at, 2)).unwrap();
    w.reconcile();
    let mut expected = square(at, radius);
    assert_eq!(w.loaded(), expected);
    // East, south-east, a long jump, and back west.
    let steps = [
        (1, 0),
        (1, 0),
        (1, 1),
        (0, 1),
        (9, 0),
        (-1, 0),
        (-1, -1),
        (-3, 0),
    ];
    for (dx, dy) in steps {
        at = TractPos::new(at.x + dx, at.y + dy);
        w.cosmos.move_ticket(ticket, at).unwrap();
        let before = w.loaded();
        let done = w.reconcile();
        // What is kept: everything within the margin that was loaded.
        // What is added: everything within the radius.
        let stay: BTreeSet<TractPos> = expected.intersection(&square(at, keep)).copied().collect();
        expected = stay.union(&square(at, radius)).copied().collect();
        assert_eq!(w.loaded(), expected, "after a step of ({dx}, {dy})");
        let loaded: BTreeSet<TractPos> = done.loaded.iter().map(|l| l.tract).collect();
        let unloaded: BTreeSet<TractPos> = done.unloaded.iter().map(|(_, t)| *t).collect();
        assert_eq!(loaded, expected.difference(&before).copied().collect());
        assert_eq!(unloaded, before.difference(&expected).copied().collect());
        assert!(done.failed.is_empty() && done.waiting.is_empty());
    }
}

#[test]
fn pacing_across_a_tract_boundary_loads_and_unloads_nothing() {
    let mut w = world();
    let west = TractPos::new(50, 20);
    let east = TractPos::new(51, 20);
    w.cosmos.hold_ticket(w.warm(west, 2)).unwrap();
    w.reconcile();
    let someone = EntityId::new(3, 0);
    // An occupant in the eastmost column of the west tract, followed by
    // a ticket of its own; the fixed one is no longer needed.
    let edge = Location::new(
        w.grid,
        CellPos::new(west.origin(0).x + 31, west.origin(0).y, 0),
    );
    w.ctx()
        .place(
            Occupant::new(someone, OccupantKind::Creature),
            edge,
            Footprint::SINGLE,
            Cause::DEBUG,
        )
        .unwrap();
    let follow = Ticket::new(
        w.realm,
        TicketAnchor::Entity(someone),
        2,
        Activity::Warm,
        TicketReason::Player,
    );
    let ticket = w.cosmos.hold_ticket(follow).unwrap();
    let fixed = w.cosmos.tickets().iter().next().unwrap().0;
    w.cosmos.release_ticket(fixed);
    assert!(w.reconcile().is_empty());
    assert_eq!(w.cosmos.tickets().centre(ticket), Some(west));

    // The first step east brings the far column in and drops nothing.
    let across = Location::new(w.grid, CellPos::new(east.origin(0).x, east.origin(0).y, 0));
    w.ctx().relocate(someone, across, Cause::DEBUG).unwrap();
    let first = w.reconcile();
    assert_eq!(first.loaded.len(), 5);
    assert!(first.unloaded.is_empty());
    assert_eq!(w.cosmos.tickets().centre(ticket), Some(east));
    let settled = w.loaded();
    assert_eq!(
        settled,
        square(west, 2).union(&square(east, 2)).copied().collect()
    );
    w.events.clear();

    // From then on, back and forth as often as it likes.
    for _ in 0..6 {
        w.ctx().relocate(someone, edge, Cause::DEBUG).unwrap();
        assert!(w.reconcile().is_empty());
        w.ctx().relocate(someone, across, Cause::DEBUG).unwrap();
        assert!(w.reconcile().is_empty());
    }
    assert_eq!(w.loaded(), settled);
    assert!(
        w.events
            .events()
            .iter()
            .all(|event| { !matches!(event.name(), "tract_loaded" | "tract_unloading") })
    );
    // The tracts kept only by the margin are warm.
    assert_eq!(
        w.cosmos.activity(w.realm, TractPos::new(48, 20)),
        Some(Activity::Warm)
    );
}

#[test]
fn over_budget_the_lowest_priority_is_refused_first_and_pinned_never() {
    let mut w = world();
    // Room for two tickets of nine tracts and a little more.
    w.cosmos.set_max_loaded_tracts(20);
    let at = |x: i32| TractPos::new(x, 30);
    let pinned = w.cosmos.hold_ticket(w.warm(at(10), 1).pinned()).unwrap();
    let low = w
        .cosmos
        .hold_ticket(
            Ticket::new(
                w.realm,
                TicketAnchor::Fixed(at(20)),
                1,
                Activity::Warm,
                TicketReason::Process(ProcessId::new(4)),
            )
            .with_priority(10),
        )
        .unwrap();
    let older = w
        .cosmos
        .hold_ticket(w.warm(at(30), 1).with_priority(50))
        .unwrap();
    let newer = w
        .cosmos
        .hold_ticket(w.warm(at(40), 1).with_priority(50))
        .unwrap();
    w.timeline.advance_to(Tick(77));

    // Four tickets want 36 tracts and 20 may be loaded. The lowest
    // priority goes first; of the two equals, the one made last.
    let done = w.reconcile();
    let refused: Vec<_> = done.refused.iter().map(|refused| refused.id).collect();
    assert_eq!(refused, [low, newer]);
    assert_eq!(
        done.refused[0].ticket.reason,
        TicketReason::Process(ProcessId::new(4))
    );
    assert!(w.cosmos.ticket(low).is_none() && w.cosmos.ticket(newer).is_none());
    assert!(w.cosmos.ticket(pinned).is_some() && w.cosmos.ticket(older).is_some());
    let expected: BTreeSet<TractPos> = square(at(10), 1)
        .union(&square(at(30), 1))
        .copied()
        .collect();
    assert_eq!(w.loaded(), expected);
    // The pinned ticket's tracts were loaded first.
    assert!(
        done.loaded[..9]
            .iter()
            .all(|loaded| square(at(10), 1).contains(&loaded.tract))
    );
    // Each holder is told, before anything is loaded.
    let events = w.events.drain();
    assert_eq!(names(&events)[..2], ["ticket_refused", "ticket_refused"]);
    assert_eq!(
        events[0],
        WorldEvent::TicketRefused {
            tick: Tick(77),
            ticket: low,
            realm: w.realm,
            loc: Location::new(w.grid, at(20).origin(0)),
            reason: TicketReason::Process(ProcessId::new(4)),
        }
    );

    // A pinned ticket is never refused, even when pinned tickets alone
    // want more than may be loaded: what does not fit is not loaded,
    // and is reported with the reason.
    let second = w.cosmos.hold_ticket(w.warm(at(50), 1).pinned()).unwrap();
    let third = w.cosmos.hold_ticket(w.warm(at(60), 1).pinned()).unwrap();
    let done = w.reconcile();
    let refused: Vec<_> = done.refused.iter().map(|refused| refused.id).collect();
    assert_eq!(refused, [older]);
    for id in [pinned, second, third] {
        assert!(w.cosmos.ticket(id).is_some());
    }
    assert_eq!(w.loaded().len(), 20);
    assert_eq!(done.failed.len(), 7);
    assert!(
        done.failed
            .iter()
            .all(|failure| { matches!(failure.error, LoadError::OverBudget { max: 20, .. }) })
    );
    // With room again, what was put off is loaded without being asked.
    w.cosmos.set_max_loaded_tracts(100);
    let done = w.reconcile();
    assert_eq!(done.loaded.len(), 7);
    assert_eq!(w.loaded().len(), 27);
}

#[test]
fn a_tract_with_someone_in_it_waits_for_the_simulation_to_empty_it() {
    let mut w = world();
    let home = TractPos::new(10, 10);
    let ticket = w.cosmos.hold_ticket(w.warm(home, 0)).unwrap();
    w.reconcile();
    let someone = EntityId::new(5, 0);
    let spot = w.middle(home);
    w.ctx()
        .place(
            Occupant::new(someone, OccupantKind::Item),
            spot,
            Footprint::SINGLE,
            Cause::DEBUG,
        )
        .unwrap();
    w.cosmos.release_ticket(ticket);
    w.events.clear();

    // The tract is to go, and someone is there: the simulation is told
    // and the tract stays for now.
    let done = w.reconcile();
    assert_eq!(done.waiting, [(w.realm, home)]);
    assert!(done.unloaded.is_empty());
    assert_eq!(names(&w.events.drain()), ["tract_unloading"]);
    assert!(w.cosmos.is_tract_loaded(w.realm, home));
    // Told once: while it waits, nothing more is said.
    let done = w.reconcile();
    assert_eq!(done.waiting, [(w.realm, home)]);
    assert!(w.events.is_empty());

    // The simulation takes its entity out; the next reconciliation
    // unloads the tract, without a second announcement.
    w.ctx().remove(someone, Cause::DEBUG).unwrap();
    w.events.clear();
    let done = w.reconcile();
    assert_eq!(done.unloaded, [(w.realm, home)]);
    assert!(w.events.is_empty());
    assert!(!w.cosmos.is_tract_loaded(w.realm, home));

    // Announced, emptied and wanted again before it went: it is
    // unloaded and loaded again, so the simulation sees both events.
    let ticket = w.cosmos.hold_ticket(w.warm(home, 0)).unwrap();
    w.reconcile();
    w.ctx()
        .place(
            Occupant::new(someone, OccupantKind::Item),
            spot,
            Footprint::SINGLE,
            Cause::DEBUG,
        )
        .unwrap();
    w.cosmos.release_ticket(ticket);
    assert_eq!(w.reconcile().waiting, [(w.realm, home)]);
    w.ctx().remove(someone, Cause::DEBUG).unwrap();
    w.cosmos.hold_ticket(w.warm(home, 0)).unwrap();
    w.events.clear();
    let done = w.reconcile();
    assert_eq!(done.unloaded, [(w.realm, home)]);
    assert_eq!(done.loaded.len(), 1);
    assert_eq!(names(&w.events.drain()), ["tract_loaded"]);
    assert!(w.cosmos.is_tract_loaded(w.realm, home));
}

#[test]
fn a_ticket_follows_its_entity_and_lapses_when_its_time_comes() {
    let mut w = world();
    let start = TractPos::new(12, 40);
    let someone = EntityId::new(8, 0);
    // An entity that is not in the world yet anchors nothing.
    let follow = Ticket::new(
        w.realm,
        TicketAnchor::Entity(someone),
        1,
        Activity::Hot,
        TicketReason::Player,
    )
    .pinned();
    let ticket = w.cosmos.hold_ticket(follow).unwrap();
    assert!(w.reconcile().is_empty());
    assert_eq!(w.cosmos.tickets().centre(ticket), None);
    // Somewhere to stand, held for a moment by a ticket that lapses.
    let landing = w
        .cosmos
        .hold_ticket(w.warm(start, 0).until(Tick(100)))
        .unwrap();
    w.reconcile();
    let spot = w.middle(start);
    w.ctx()
        .place(
            Occupant::new(someone, OccupantKind::Creature),
            spot,
            Footprint::SINGLE,
            Cause::DEBUG,
        )
        .unwrap();
    let done = w.reconcile();
    assert_eq!(w.cosmos.tickets().centre(ticket), Some(start));
    assert_eq!(done.loaded.len(), 49 - 1);
    assert_eq!(w.cosmos.activity(w.realm, start), Some(Activity::Hot));

    // The landing ticket lapses at its tick and is reported.
    w.timeline.advance_to(Tick(99));
    assert!(w.reconcile().expired.is_empty());
    w.timeline.advance_to(Tick(100));
    let done = w.reconcile();
    assert_eq!(done.expired.len(), 1);
    assert_eq!(done.expired[0].id, landing);
    assert!(w.cosmos.ticket(landing).is_none());
    assert_eq!(w.loaded(), square(start, 3));

    // The entity walks three tracts east; its ticket goes with it.
    let far = w.middle(TractPos::new(start.x + 3, start.y));
    w.ctx().relocate(someone, far, Cause::DEBUG).unwrap();
    let done = w.reconcile();
    assert_eq!(
        w.cosmos.tickets().centre(ticket),
        Some(TractPos::new(15, 40))
    );
    assert_eq!(done.loaded.len(), 3 * 7);
    assert_eq!(done.unloaded.len(), 2 * 7);
    // A ticket anchored to an entity is not moved by hand.
    assert!(matches!(
        w.cosmos.move_ticket(ticket, start),
        Err(WorldError::BadValue { .. })
    ));
    // Reconciliation is part of the world's state: two worlds brought
    // here the same way hash the same.
    let again = {
        let mut w = world();
        let ticket = w.cosmos.hold_ticket(follow).unwrap();
        w.reconcile();
        w.cosmos
            .hold_ticket(w.warm(start, 0).until(Tick(100)))
            .unwrap();
        w.reconcile();
        let spot = w.middle(start);
        w.ctx()
            .place(
                Occupant::new(someone, OccupantKind::Creature),
                spot,
                Footprint::SINGLE,
                Cause::DEBUG,
            )
            .unwrap();
        w.reconcile();
        w.timeline.advance_to(Tick(99));
        w.reconcile();
        w.timeline.advance_to(Tick(100));
        w.reconcile();
        w.ctx().relocate(someone, far, Cause::DEBUG).unwrap();
        w.reconcile();
        assert!(w.cosmos.ticket(ticket).is_some());
        Hash128::of(&w.cosmos)
    };
    assert_eq!(Hash128::of(&w.cosmos), again);
}

#[test]
fn tracts_put_there_by_hand_are_left_alone() {
    let mut w = world();
    let by_hand = TractPos::new(70, 10);
    let column = flat_column(&w.defs, BlockState::NONE, FloorState::NONE, 0);
    load_column(&mut w.cosmos, w.grid, by_hand, column).unwrap();
    // No ticket wants it, and reconciliation neither counts it nor
    // unloads it.
    assert!(w.reconcile().is_empty());
    assert!(w.cosmos.is_tract_loaded(w.realm, by_hand));
    assert!(!w.cosmos.tickets().is_kept(w.realm, by_hand));
    assert_eq!(w.cosmos.tickets().kept_count(), 0);
    // A ticket over it loads the tracts around it and leaves it be.
    let ticket = w.cosmos.hold_ticket(w.warm(by_hand, 1)).unwrap();
    let done = w.reconcile();
    assert_eq!(done.loaded.len(), 8);
    w.cosmos.release_ticket(ticket);
    let done = w.reconcile();
    assert_eq!(done.unloaded.len(), 8);
    assert_eq!(w.loaded(), BTreeSet::from([by_hand]));
    // Tickets for places that cannot be are refused with the reason.
    assert_eq!(
        w.cosmos
            .hold_ticket(Ticket::new(
                RealmId::new(9),
                TicketAnchor::Fixed(by_hand),
                1,
                Activity::Warm,
                TicketReason::Debug
            ))
            .err(),
        Some(WorldError::UnknownRealm(RealmId::new(9)))
    );
    assert!(matches!(
        w.cosmos.hold_ticket(w.warm(TractPos::new(0, -5), 1)),
        Err(WorldError::OutsideRealm { .. })
    ));
    // A ticket at the realm's edge covers only what is there, and one
    // on the wrapped axis reaches round it.
    w.cosmos
        .hold_ticket(w.warm(TractPos::new(0, 0), 1))
        .unwrap();
    w.reconcile();
    let corner: BTreeSet<TractPos> = [(127, 0), (0, 0), (1, 0), (127, 1), (0, 1), (1, 1)]
        .into_iter()
        .map(|(x, y)| TractPos::new(x, y))
        .collect();
    let mut expected = corner;
    expected.insert(by_hand);
    assert_eq!(w.loaded(), expected);
}
