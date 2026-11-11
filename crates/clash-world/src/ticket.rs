//! Tickets: requests to keep tracts loaded, and the reconciliation that
//! loads and unloads what they ask for (02 §10).
//!
//! Nothing is loaded because it is near some fixed point. A tract is
//! loaded because a [`Ticket`] covers it: the controlled character's, a
//! process's, a rule's. At every safe point the simulation calls
//! [`WorldCtx::reconcile_activity`], which compares what the tickets
//! want with what is loaded and loads or unloads the difference.
//!
//! - A ticket covers the tracts within its radius of its anchor
//!   (Chebyshev distance). A hot ticket also keeps a warm margin of
//!   [`WARM_MARGIN`] tracts around itself.
//! - A tract is loaded when a ticket covers it and unloaded only once
//!   it lies more than [`UNLOAD_MARGIN`] tract outside every ticket, so
//!   that an anchor pacing across a tract boundary loads and unloads
//!   nothing.
//! - At most [`TicketTable::max_loaded`] tracts are kept loaded. When
//!   the tickets want more, the ticket with the lowest priority is
//!   refused, and of equals the one made last; a pinned ticket never is.
//!   A refused ticket is gone: its holder is told by `TicketRefused` and
//!   in what reconciliation returns.
//! - Loads are made in order of priority, then distance from the
//!   ticket's anchor, then position, so the outcome is the same on
//!   every run.
//!
//! # Unloading takes two steps where someone is there
//!
//! The world does not own entities and cannot remove them (01 R4). A
//! tract with no occupant is unloaded at once, after `TractUnloading`.
//! For a tract that holds occupants, `TractUnloading` is emitted and the
//! tract stays for now: the simulation answers the event by taking its
//! entities out, and the next reconciliation unloads the tract. If that
//! tract is wanted again by then it is still unloaded, and loaded again
//! in the same pass with `TractLoaded`, so the simulation always sees
//! the two events in pairs. A tract that is still occupied at the next
//! reconciliation stays loaded and is reported as waiting.
//!
//! Tracts put into the world by `test_support` are outside all this:
//! reconciliation neither counts nor unloads them.

use std::collections::{BTreeMap, BTreeSet};

use clash_core::{EntityId, IVec2, StateHash, StateHasher, Tick, define_id};
use clash_time::ProcessId;
use serde::{Deserialize, Serialize};

use crate::cosmos::{Cosmos, RealmId};
use crate::edit::WorldCtx;
use crate::error::WorldError;
use crate::event::WorldEvent;
use crate::loading::{LoadError, LoadedTract, TractIo, first_occupant, tract_in};
use crate::pos::{Location, TractPos};

/// The priority of a ticket that is never refused.
pub const PINNED: u8 = 255;

/// The priority of a ticket that does not say.
pub const DEFAULT_PRIORITY: u8 = 100;

/// How many tracts of warm margin a hot ticket keeps around itself.
pub const WARM_MARGIN: u8 = 2;

/// How far outside every ticket a tract may lie and stay loaded, in
/// tracts. Tracts are loaded at a ticket's reach and unloaded only
/// beyond this margin, which prevents thrashing at the boundary.
pub const UNLOAD_MARGIN: u8 = 1;

/// The most tracts kept loaded unless the setting `max_loaded_tracts`
/// says otherwise.
pub const DEFAULT_MAX_LOADED_TRACTS: usize = 1600;

/// The radius of the controlled character's ticket, in tracts: the
/// default of the setting `world.hot_radius`.
pub const HOT_RADIUS: u8 = 6;

define_id! {
    /// A ticket in the [`TicketTable`]. Never reused.
    pub struct TicketId;
}

/// How much is going on in a loaded tract (02 §10.1). The world runs
/// physics in full on every loaded tract; the difference matters to the
/// simulation, which throttles minds and spawning in warm tracts.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Activity {
    /// Loaded, with the simulation throttled.
    Warm,
    /// Loaded, at full detail.
    Hot,
}

impl StateHash for Activity {
    fn state_hash(&self, hasher: &mut StateHasher) {
        hasher.write_u8(*self as u8);
    }
}

/// What a ticket is centred on.
///
/// Bodies and subgrids join this with the packages that build them
/// (W14, W20a). Until then whoever holds a ticket for one moves a fixed
/// anchor with [`Cosmos::move_ticket`].
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum TicketAnchor {
    /// A tract of the ticket's realm.
    Fixed(TractPos),
    /// Wherever this occupant is. The ticket follows it from tract to
    /// tract, and into another realm. While the entity is not in the
    /// world the ticket stays where it was last.
    Entity(EntityId),
}

impl StateHash for TicketAnchor {
    fn state_hash(&self, hasher: &mut StateHasher) {
        match self {
            TicketAnchor::Fixed(tract) => {
                hasher.write_u8(0);
                tract.state_hash(hasher);
            }
            TicketAnchor::Entity(entity) => {
                hasher.write_u8(1);
                entity.state_hash(hasher);
            }
        }
    }
}

/// Who holds a ticket, so that they can be told when it is refused.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum TicketReason {
    /// A controlled character.
    Player,
    /// A world process: a collapse, a blast. Refused, it is told to
    /// finish at once in resolve mode.
    Process(ProcessId),
    /// A body in flight. Refused, it is resolved coarsely.
    Body,
    /// A rule or a scenario, by the rule's number. The world gives the
    /// number no meaning.
    Eoc(u32),
    /// A tool or a test.
    Debug,
}

impl StateHash for TicketReason {
    fn state_hash(&self, hasher: &mut StateHasher) {
        match self {
            TicketReason::Player => hasher.write_u8(0),
            TicketReason::Process(process) => {
                hasher.write_u8(1);
                process.state_hash(hasher);
            }
            TicketReason::Body => hasher.write_u8(2),
            TicketReason::Eoc(rule) => {
                hasher.write_u8(3);
                hasher.write_u32(*rule);
            }
            TicketReason::Debug => hasher.write_u8(4),
        }
    }
}

/// A request to keep tracts loaded (02 §10.1).
///
/// ```
/// use clash_world::{Activity, RealmId, Ticket, TicketAnchor, TicketReason, TractPos};
///
/// let ticket = Ticket::new(
///     RealmId::new(0),
///     TicketAnchor::Fixed(TractPos::new(4, 4)),
///     1,
///     Activity::Hot,
///     TicketReason::Debug,
/// );
/// // One tract of hot around the anchor, and two more of warm margin.
/// assert_eq!(ticket.reach(), 3);
/// assert!(!ticket.is_pinned());
/// assert!(ticket.pinned().is_pinned());
/// ```
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Serialize, Deserialize)]
pub struct Ticket {
    /// The realm whose tracts it keeps. A ticket anchored to an entity
    /// follows the entity's realm.
    pub realm: RealmId,
    /// What it is centred on.
    pub anchor: TicketAnchor,
    /// How far from the anchor it reaches, in tracts, counted the
    /// longer way along either axis.
    pub radius: u8,
    /// How much it asks for within its radius.
    pub level: Activity,
    /// Which tickets are refused first when there are too many: the
    /// lowest. [`PINNED`] is never refused.
    pub priority: u8,
    /// Who holds it.
    pub reason: TicketReason,
    /// When it lapses of itself, in universal time.
    pub expires: Option<Tick>,
}

impl Ticket {
    /// A ticket of the default priority that does not lapse.
    pub const fn new(
        realm: RealmId,
        anchor: TicketAnchor,
        radius: u8,
        level: Activity,
        reason: TicketReason,
    ) -> Ticket {
        Ticket {
            realm,
            anchor,
            radius,
            level,
            priority: DEFAULT_PRIORITY,
            reason,
            expires: None,
        }
    }

    /// The ticket of a controlled character: hot, of the default hot
    /// radius, pinned (02 §10.1).
    pub const fn player(realm: RealmId, entity: EntityId) -> Ticket {
        Ticket::new(
            realm,
            TicketAnchor::Entity(entity),
            HOT_RADIUS,
            Activity::Hot,
            TicketReason::Player,
        )
        .pinned()
    }

    /// This ticket with a priority.
    pub const fn with_priority(mut self, priority: u8) -> Ticket {
        self.priority = priority;
        self
    }

    /// This ticket, never to be refused.
    pub const fn pinned(self) -> Ticket {
        self.with_priority(PINNED)
    }

    /// This ticket, lapsing at a tick.
    pub const fn until(mut self, tick: Tick) -> Ticket {
        self.expires = Some(tick);
        self
    }

    /// Whether the ticket is never refused.
    pub const fn is_pinned(&self) -> bool {
        self.priority == PINNED
    }

    /// How far from its anchor the ticket keeps tracts loaded: its
    /// radius, and for a hot ticket the warm margin beyond it.
    pub const fn reach(&self) -> i32 {
        let margin = match self.level {
            Activity::Hot => WARM_MARGIN as i32,
            Activity::Warm => 0,
        };
        self.radius as i32 + margin
    }
}

clash_core::impl_state_hash!(Ticket {
    realm,
    anchor,
    radius,
    level,
    priority,
    reason,
    expires
});

/// A ticket and where its anchor was last seen.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
struct Held {
    ticket: Ticket,
    /// The tract of the ticket's realm that its anchor is in, or was in
    /// when it was last in the world. `None` for an entity that has
    /// never been seen there: the ticket covers nothing yet.
    at: Option<TractPos>,
}

clash_core::impl_state_hash!(Held { ticket, at });

/// A tract of a realm.
type TractKey = (RealmId, TractPos);

fn yes() -> bool {
    true
}

/// Every ticket, and which tracts they keep loaded (02 §10).
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub struct TicketTable {
    tickets: BTreeMap<TicketId, Held>,
    next: u32,
    max_loaded: usize,
    /// The tracts that were loaded by the loading procedure and are
    /// still loaded: those reconciliation counts and may unload.
    kept: BTreeSet<TractKey>,
    /// The tracts whose unloading has been announced and that are
    /// waiting for their occupants to be taken out.
    leaving: BTreeSet<TractKey>,
    /// The activity of each kept tract, as of the last reconciliation.
    /// Derived from the tickets; not state.
    #[serde(skip)]
    levels: BTreeMap<TractKey, Activity>,
    /// Whether anything has changed that the next reconciliation must
    /// look at. Derived; not state.
    #[serde(skip, default = "yes")]
    stale: bool,
}

impl Default for TicketTable {
    fn default() -> TicketTable {
        TicketTable::new()
    }
}

impl TicketTable {
    /// A table of no tickets, with the default budget.
    pub fn new() -> TicketTable {
        TicketTable {
            tickets: BTreeMap::new(),
            next: 0,
            max_loaded: DEFAULT_MAX_LOADED_TRACTS,
            kept: BTreeSet::new(),
            leaving: BTreeSet::new(),
            levels: BTreeMap::new(),
            stale: true,
        }
    }

    /// How many tickets are held.
    pub fn len(&self) -> usize {
        self.tickets.len()
    }

    /// Whether no ticket is held.
    pub fn is_empty(&self) -> bool {
        self.tickets.is_empty()
    }

    /// A ticket by id.
    pub fn get(&self, id: TicketId) -> Option<&Ticket> {
        self.tickets.get(&id).map(|held| &held.ticket)
    }

    /// Every ticket, in id order.
    pub fn iter(&self) -> impl Iterator<Item = (TicketId, &Ticket)> + '_ {
        self.tickets.iter().map(|(id, held)| (*id, &held.ticket))
    }

    /// The tract a ticket is centred on now: its fixed anchor, or where
    /// its entity was last seen.
    pub fn centre(&self, id: TicketId) -> Option<TractPos> {
        self.tickets.get(&id)?.at
    }

    /// The most tracts reconciliation keeps loaded (`max_loaded_tracts`).
    pub fn max_loaded(&self) -> usize {
        self.max_loaded
    }

    /// How many tracts the tickets are keeping loaded.
    pub fn kept_count(&self) -> usize {
        self.kept.len()
    }

    /// Whether a tract was loaded by the loading procedure and is still
    /// loaded: whether reconciliation counts it and may unload it.
    pub fn is_kept(&self, realm: RealmId, tract: TractPos) -> bool {
        self.kept.contains(&(realm, tract))
    }

    /// Notes that the loading procedure has loaded a tract.
    pub(crate) fn note_loaded(&mut self, realm: RealmId, tract: TractPos) {
        self.kept.insert((realm, tract));
        self.stale = true;
    }

    /// Forgets a tract that is no longer loaded.
    pub(crate) fn forget(&mut self, realm: RealmId, tract: TractPos) {
        let key = (realm, tract);
        if self.kept.remove(&key) {
            self.stale = true;
        }
        self.leaving.remove(&key);
        self.levels.remove(&key);
    }
}

/// Hashes the tickets with where each was last seen, the next id, the
/// budget, and the tracts kept and leaving. The activity levels follow
/// from these and are not hashed.
impl StateHash for TicketTable {
    fn state_hash(&self, hasher: &mut StateHasher) {
        self.tickets.state_hash(hasher);
        hasher.write_u32(self.next);
        self.max_loaded.state_hash(hasher);
        self.kept.state_hash(hasher);
        self.leaving.state_hash(hasher);
    }
}

/// A ticket that was refused because the tickets together wanted more
/// tracts than may be loaded. It is no longer in the table.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct RefusedTicket {
    /// The id it had.
    pub id: TicketId,
    /// The ticket: its `reason` says who held it.
    pub ticket: Ticket,
}

/// A tract that could not be loaded or unloaded, and why.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct TractFailure {
    /// The realm.
    pub realm: RealmId,
    /// The tract.
    pub tract: TractPos,
    /// Why.
    pub error: LoadError,
}

/// What one reconciliation did (02 §10.2).
#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct Reconciled {
    /// The tracts loaded, in the order they were: by priority, then
    /// distance from their ticket's anchor, then position. Each carries
    /// what the simulation is to put there.
    pub loaded: Vec<LoadedTract>,
    /// The tracts unloaded, by realm and then row by row.
    pub unloaded: Vec<(RealmId, TractPos)>,
    /// The tickets refused for the budget, lowest priority first. They
    /// are gone from the table; each holder is also told by
    /// `TicketRefused`.
    pub refused: Vec<RefusedTicket>,
    /// The tickets that lapsed, in id order.
    pub expired: Vec<RefusedTicket>,
    /// The tracts that are to be unloaded and still hold occupants.
    /// `TractUnloading` has been emitted for each; it is unloaded by the
    /// first reconciliation that finds it empty.
    pub waiting: Vec<(RealmId, TractPos)>,
    /// The tracts that could not be loaded or unloaded, each with the
    /// reason.
    pub failed: Vec<TractFailure>,
}

impl Reconciled {
    /// Whether the reconciliation found nothing to do.
    pub fn is_empty(&self) -> bool {
        self.loaded.is_empty()
            && self.unloaded.is_empty()
            && self.refused.is_empty()
            && self.expired.is_empty()
            && self.waiting.is_empty()
            && self.failed.is_empty()
    }
}

/// What the tickets ask of one tract.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
struct Want {
    level: Activity,
    /// The highest priority of the tickets covering it.
    priority: u8,
    /// Its distance from the anchor of the ticket of that priority
    /// nearest to it, in tracts.
    dist: i32,
}

impl Cosmos {
    /// The tickets.
    pub fn tickets(&self) -> &TicketTable {
        &self.tickets
    }

    /// Holds a ticket: asks for the tracts around its anchor to be kept
    /// loaded from the next reconciliation on (the verb `hold_ticket`).
    ///
    /// Refused for an unknown realm and for a fixed anchor outside it.
    pub fn hold_ticket(&mut self, mut ticket: Ticket) -> Result<TicketId, WorldError> {
        if self.realm(ticket.realm).is_none() {
            return Err(WorldError::UnknownRealm(ticket.realm));
        }
        let at = match ticket.anchor {
            TicketAnchor::Fixed(tract) => {
                let tract = self.normalize_tract(ticket.realm, tract)?;
                ticket.anchor = TicketAnchor::Fixed(tract);
                Some(tract)
            }
            TicketAnchor::Entity(entity) => match self.anchor_of(entity) {
                Some((realm, tract)) => {
                    ticket.realm = realm;
                    Some(tract)
                }
                None => None,
            },
        };
        let next = self
            .tickets
            .next
            .checked_add(1)
            .ok_or(WorldError::IdsExhausted("ticket"))?;
        let id = TicketId::new(self.tickets.next);
        self.tickets.next = next;
        self.tickets.tickets.insert(id, Held { ticket, at });
        self.tickets.stale = true;
        Ok(id)
    }

    /// Releases a ticket and returns it (the verb `release_ticket`).
    /// The tracts only it kept are unloaded by the next reconciliation.
    pub fn release_ticket(&mut self, id: TicketId) -> Option<Ticket> {
        let held = self.tickets.tickets.remove(&id)?;
        self.tickets.stale = true;
        Some(held.ticket)
    }

    /// A ticket by id.
    pub fn ticket(&self, id: TicketId) -> Option<&Ticket> {
        self.tickets.get(id)
    }

    /// Moves a ticket with a fixed anchor to another tract of its realm.
    /// Refused for an unknown ticket, a ticket anchored to an entity,
    /// and a tract outside the realm.
    pub fn move_ticket(&mut self, id: TicketId, to: TractPos) -> Result<(), WorldError> {
        let held = self.tickets.tickets.get(&id).ok_or(WorldError::BadValue {
            what: "a ticket",
            why: "there is no ticket with that id",
        })?;
        if !matches!(held.ticket.anchor, TicketAnchor::Fixed(_)) {
            return Err(WorldError::BadValue {
                what: "a ticket's anchor",
                why: "only a fixed anchor is moved by hand; this one follows its entity",
            });
        }
        let to = self.normalize_tract(held.ticket.realm, to)?;
        if let Some(held) = self.tickets.tickets.get_mut(&id)
            && held.at != Some(to)
        {
            held.ticket.anchor = TicketAnchor::Fixed(to);
            held.at = Some(to);
            self.tickets.stale = true;
        }
        Ok(())
    }

    /// Sets the most tracts reconciliation keeps loaded: the setting
    /// `max_loaded_tracts`.
    pub fn set_max_loaded_tracts(&mut self, max: usize) {
        if self.tickets.max_loaded != max {
            self.tickets.max_loaded = max;
            self.tickets.stale = true;
        }
    }

    /// The activity of a tract as of the last reconciliation: the
    /// highest level of any ticket covering it, warm for a tract kept
    /// only by a margin, and `None` for a tract the tickets do not keep.
    pub fn activity(&self, realm: RealmId, tract: TractPos) -> Option<Activity> {
        let tract = self.normalize_tract(realm, tract).ok()?;
        self.tickets.levels.get(&(realm, tract)).copied()
    }

    /// The realm and tract an entity is in, if it is an occupant of a
    /// realm's base grid.
    fn anchor_of(&self, entity: EntityId) -> Option<TractKey> {
        let loc = self.occupant(entity)?.loc;
        let realm = self.realm_of(loc.grid)?;
        // A subgrid's cells are in its own coordinates: until subgrids
        // are built only base grids hold occupants.
        (self.realm(realm)?.base_grid == loc.grid).then(|| (realm, loc.cell.tract()))
    }

    /// Brings every entity-anchored ticket to where its entity is.
    fn follow_anchors(&mut self) {
        let moves: Vec<(TicketId, TractKey)> = self
            .tickets
            .tickets
            .iter()
            .filter_map(|(id, held)| match held.ticket.anchor {
                TicketAnchor::Entity(entity) => {
                    let (realm, tract) = self.anchor_of(entity)?;
                    (held.ticket.realm != realm || held.at != Some(tract))
                        .then_some((*id, (realm, tract)))
                }
                TicketAnchor::Fixed(_) => None,
            })
            .collect();
        for (id, (realm, tract)) in moves {
            if let Some(held) = self.tickets.tickets.get_mut(&id) {
                held.ticket.realm = realm;
                held.at = Some(tract);
                self.tickets.stale = true;
            }
        }
    }

    /// The tracts within reach of the tickets, with `extra` tracts of
    /// reach added to each, and what the tickets ask of each tract.
    fn cover(&self, extra: i32) -> BTreeMap<TractKey, Want> {
        let mut cover: BTreeMap<TractKey, Want> = BTreeMap::new();
        for held in self.tickets.tickets.values() {
            let (ticket, Some(centre)) = (&held.ticket, held.at) else {
                continue;
            };
            let Ok(info) = self.realm_info(ticket.realm) else {
                continue;
            };
            let reach = ticket.reach() + extra;
            for dy in -reach..=reach {
                for dx in -reach..=reach {
                    let Ok(tract) = tract_in(&info.topology, centre.offset(IVec2::new(dx, dy)))
                    else {
                        continue;
                    };
                    let dist = dx.abs().max(dy.abs());
                    let level = if dist <= i32::from(ticket.radius) {
                        ticket.level
                    } else {
                        Activity::Warm
                    };
                    let want = Want {
                        level,
                        priority: ticket.priority,
                        dist,
                    };
                    cover
                        .entry((ticket.realm, tract))
                        .and_modify(|have| {
                            have.level = have.level.max(level);
                            if (want.priority, -want.dist) > (have.priority, -have.dist) {
                                have.priority = want.priority;
                                have.dist = want.dist;
                            }
                        })
                        .or_insert(want);
                }
            }
        }
        cover
    }

    /// The ticket to refuse next: the lowest priority, and of equals
    /// the one made last. Never a pinned one.
    fn refusal(&self) -> Option<TicketId> {
        self.tickets
            .tickets
            .iter()
            .filter(|(_, held)| !held.ticket.is_pinned())
            .min_by_key(|(id, held)| (held.ticket.priority, core::cmp::Reverse(**id)))
            .map(|(id, _)| *id)
    }
}

impl WorldCtx<'_> {
    /// Brings what is loaded into line with what the tickets want
    /// (02 §10.2). The simulation calls this at every safe point; when
    /// nothing has changed since the last call it does nothing and
    /// costs a look at each ticket.
    ///
    /// In order:
    ///
    /// 1. Tickets whose time has come lapse, and tickets anchored to an
    ///    entity follow it.
    /// 2. While the tickets want more tracts than may be loaded, the
    ///    ticket of the lowest priority is refused, and of equals the
    ///    one made last; never a pinned one. `TicketRefused` is emitted
    ///    for each.
    /// 3. Kept tracts more than [`UNLOAD_MARGIN`] outside every ticket
    ///    are unloaded, by realm and row by row. One that holds
    ///    occupants is announced with `TractUnloading` and waits for
    ///    the simulation to empty it (see the module documentation).
    /// 4. Wanted tracts that are not loaded are loaded, by priority,
    ///    then distance from their ticket's anchor, then position, as
    ///    far as the budget allows.
    ///
    /// Returns what was loaded, unloaded and refused, and what could
    /// not be done with the reason.
    pub fn reconcile_activity(&mut self, io: &mut TractIo<'_>) -> Reconciled {
        let mut out = Reconciled::default();
        let now = self.timeline.now();

        // 1. Lapsed tickets go; anchored tickets follow.
        let lapsed: Vec<TicketId> = self
            .cosmos
            .tickets
            .tickets
            .iter()
            .filter(|(_, held)| held.ticket.expires.is_some_and(|tick| tick <= now))
            .map(|(id, _)| *id)
            .collect();
        for id in lapsed {
            if let Some(ticket) = self.cosmos.release_ticket(id) {
                out.expired.push(RefusedTicket { id, ticket });
            }
        }
        self.cosmos.follow_anchors();
        if !self.cosmos.tickets.stale && self.cosmos.tickets.leaving.is_empty() {
            return out;
        }

        // 2. The budget: refuse until what is wanted fits.
        let max = self.cosmos.tickets.max_loaded;
        let mut wanted = self.cosmos.cover(0);
        while wanted.len() > max {
            let Some(id) = self.cosmos.refusal() else {
                break;
            };
            let centre = self.cosmos.tickets.centre(id);
            let Some(ticket) = self.cosmos.release_ticket(id) else {
                break;
            };
            let grid = self.cosmos.realm(ticket.realm).map(|realm| realm.base_grid);
            if let Some(grid) = grid {
                let at = centre.unwrap_or_default();
                self.events.emit(WorldEvent::TicketRefused {
                    tick: now,
                    ticket: id,
                    realm: ticket.realm,
                    loc: Location::new(grid, at.origin(0)),
                    reason: ticket.reason,
                });
            }
            out.refused.push(RefusedTicket { id, ticket });
            wanted = self.cosmos.cover(0);
        }

        // 3. Unload what lies beyond the margin, and what was waiting
        // to go. Tracts within the margin stay while there is room.
        let near = self.cosmos.cover(i32::from(UNLOAD_MARGIN));
        let mut room = max.saturating_sub(wanted.len());
        let kept: Vec<TractKey> = self.cosmos.tickets.kept.iter().copied().collect();
        for key in kept {
            let (realm, tract) = key;
            let announced = self.cosmos.tickets.leaving.contains(&key);
            let occupied = self.cosmos.loaded_tract_occupant(realm, tract).is_some();
            if !announced {
                if wanted.contains_key(&key) {
                    continue;
                }
                if near.contains_key(&key) && room > 0 {
                    room -= 1;
                    continue;
                }
                if occupied {
                    // Tell the simulation, and wait for it to take its
                    // entities out.
                    self.announce_unloading(realm, tract, now);
                    self.cosmos.tickets.leaving.insert(key);
                    out.waiting.push(key);
                    continue;
                }
            } else if occupied {
                if near.contains_key(&key) {
                    // Wanted again before anyone left: it stays.
                    self.cosmos.tickets.leaving.remove(&key);
                } else {
                    out.waiting.push(key);
                }
                continue;
            }
            match self.unload(io, realm, tract, !announced) {
                Ok(()) => out.unloaded.push(key),
                Err(error) => out.failed.push(TractFailure {
                    realm,
                    tract,
                    error,
                }),
            }
        }

        // 4. Load what is wanted and missing, in order, within budget.
        let mut missing: Vec<(TractKey, Want)> = wanted
            .iter()
            .filter(|((realm, tract), _)| !self.cosmos.is_tract_loaded(*realm, *tract))
            .map(|(key, want)| (*key, *want))
            .collect();
        missing.sort_by_key(|((realm, tract), want)| {
            (core::cmp::Reverse(want.priority), want.dist, *tract, *realm)
        });
        let space = max.saturating_sub(self.cosmos.tickets.kept.len());
        let mut deferred = false;
        if missing.len() > space {
            for ((realm, tract), _) in missing.split_off(space) {
                deferred = true;
                out.failed.push(TractFailure {
                    realm,
                    tract,
                    error: LoadError::OverBudget { realm, tract, max },
                });
            }
        }
        let order: Vec<TractKey> = missing.iter().map(|(key, _)| *key).collect();
        for (outcome, (realm, tract)) in self.load_tracts(io, &order).into_iter().zip(order) {
            match outcome {
                Ok(loaded) => out.loaded.push(loaded),
                Err(error) => out.failed.push(TractFailure {
                    realm,
                    tract,
                    error,
                }),
            }
        }

        // The levels of what is kept now.
        let levels: BTreeMap<TractKey, Activity> = self
            .cosmos
            .tickets
            .kept
            .iter()
            .map(|key| {
                let level = wanted.get(key).map_or(Activity::Warm, |want| want.level);
                (*key, level)
            })
            .collect();
        self.cosmos.tickets.levels = levels;
        // A tract put off for want of room is tried again next time.
        self.cosmos.tickets.stale = deferred;
        out
    }

    fn announce_unloading(&mut self, realm: RealmId, tract: TractPos, tick: Tick) {
        if let Some(grid) = self.cosmos.realm(realm).map(|realm| realm.base_grid) {
            self.events.emit(WorldEvent::TractUnloading {
                tick,
                realm,
                loc: Location::new(grid, tract.origin(0)),
                tract,
            });
        }
    }
}

impl Cosmos {
    /// An occupant of a loaded tract, if it has any: the one with the
    /// lowest id.
    fn loaded_tract_occupant(&self, realm: RealmId, tract: TractPos) -> Option<EntityId> {
        let grid = self.realm(realm)?.base_grid;
        first_occupant(self.grid(grid).ok()?.column(tract)?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use clash_core::{Hash128, codec};

    fn table() -> TicketTable {
        let mut table = TicketTable::new();
        let realm = RealmId::new(2);
        let fixed = Ticket::new(
            realm,
            TicketAnchor::Fixed(TractPos::new(3, -4)),
            2,
            Activity::Hot,
            TicketReason::Process(ProcessId::new(9)),
        )
        .with_priority(40)
        .until(Tick(500));
        let follow = Ticket::player(realm, EntityId::new(6, 1));
        table.tickets.insert(
            TicketId::new(0),
            Held {
                ticket: fixed,
                at: Some(TractPos::new(3, -4)),
            },
        );
        table.tickets.insert(
            TicketId::new(1),
            Held {
                ticket: follow,
                at: None,
            },
        );
        table.next = 2;
        table.note_loaded(realm, TractPos::new(3, -4));
        table.note_loaded(realm, TractPos::new(4, -4));
        table.leaving.insert((realm, TractPos::new(4, -4)));
        table
    }

    #[test]
    fn the_table_round_trips_and_what_is_derived_is_left_out() {
        let mut table = table();
        table
            .levels
            .insert((RealmId::new(2), TractPos::new(3, -4)), Activity::Hot);
        table.stale = false;
        let bytes = codec::to_bytes(&table).unwrap();
        let back: TicketTable = codec::from_bytes(&bytes).unwrap();
        // The tickets, the budget and the tracts kept and leaving come
        // back; the levels are worked out again, and the table says it
        // must be looked at.
        assert_eq!(Hash128::of(&back), Hash128::of(&table));
        assert_eq!(back.len(), 2);
        assert_eq!(back.get(TicketId::new(1)), table.get(TicketId::new(1)));
        assert_eq!(back.centre(TicketId::new(0)), Some(TractPos::new(3, -4)));
        assert_eq!(back.kept_count(), 2);
        assert!(back.levels.is_empty());
        assert!(back.stale);
    }

    #[test]
    fn what_the_tickets_keep_is_state() {
        let table = table();
        let hash = Hash128::of(&table);
        let mut other = table.clone();
        other.forget(RealmId::new(2), TractPos::new(4, -4));
        assert_ne!(Hash128::of(&other), hash);
        assert!(!other.is_kept(RealmId::new(2), TractPos::new(4, -4)));
        assert!(other.leaving.is_empty());
        let mut other = table.clone();
        other.max_loaded += 1;
        assert_ne!(Hash128::of(&other), hash);
        let mut other = table.clone();
        other.tickets.remove(&TicketId::new(1));
        assert_ne!(Hash128::of(&other), hash);
        // What follows from the rest does not count.
        let mut other = table;
        other
            .levels
            .insert((RealmId::new(2), TractPos::new(0, 0)), Activity::Warm);
        other.stale = false;
        assert_eq!(Hash128::of(&other), hash);
    }

    #[test]
    fn a_hot_ticket_reaches_past_its_radius_and_a_warm_one_does_not() {
        let at = TicketAnchor::Fixed(TractPos::new(0, 0));
        let realm = RealmId::new(0);
        let hot = Ticket::new(realm, at, 6, Activity::Hot, TicketReason::Debug);
        let warm = Ticket::new(realm, at, 6, Activity::Warm, TicketReason::Debug);
        assert_eq!(hot.reach(), 6 + i32::from(WARM_MARGIN));
        assert_eq!(warm.reach(), 6);
        assert!(Activity::Hot > Activity::Warm);
        assert_eq!(hot.priority, DEFAULT_PRIORITY);
        assert!(!hot.is_pinned() && hot.pinned().is_pinned());
    }
}
