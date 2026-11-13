//! What the agents' layer keeps (09 §10.1; 06 §12): the residents of the
//! world's settlements, the state of each that is not loaded, and the
//! one schedule of their wake-ups. All of it is saved with the
//! simulation and hashed with the kernel.

use std::collections::{BTreeMap, BTreeSet};

use clash_core::{EntityId, StateHash, StateHasher, Tick, impl_state_hash};
use clash_time::WakeId;
use clash_world::{CellPos, Location, RealmId};
use serde::{Deserialize, Deserializer, Serialize, Serializer};

/// Which resident: the realm it lives in, the cell its home is (where
/// generation asked for it), and which of those asked for at that cell
/// it is, from 0. The same resident has the same key every time its
/// tract loads (invariant B7).
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug, Serialize, Deserialize)]
pub struct ResidentKey {
    /// The realm.
    pub realm: RealmId,
    /// Its home: the cell it was asked for at.
    pub home: CellPos,
    /// Which of those asked for at that cell.
    pub index: u16,
}

impl_state_hash!(ResidentKey { realm, home, index });

/// Where an agent is (09 §10.1): at a place, or walking between two.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentPlace {
    /// At a place: the spot it keeps there.
    At {
        /// The cell.
        loc: Location,
    },
    /// On its way from one place to another, along the straight line
    /// between them, from one tick until another.
    Travelling {
        /// Where it set out from.
        from: Location,
        /// Where it is going.
        to: Location,
        /// When it set out.
        since: Tick,
        /// When it arrives.
        until: Tick,
    },
}

impl StateHash for AgentPlace {
    fn state_hash(&self, hasher: &mut StateHasher) {
        match self {
            AgentPlace::At { loc } => {
                hasher.write_u8(0);
                loc.state_hash(hasher);
            }
            AgentPlace::Travelling {
                from,
                to,
                since,
                until,
            } => {
                hasher.write_u8(1);
                from.state_hash(hasher);
                to.state_hash(hasher);
                since.state_hash(hasher);
                until.state_hash(hasher);
            }
        }
    }
}

/// What an agent is doing (09 §10.1): the word its routine gives, since
/// when, and until when.
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub struct Activity {
    /// What it is doing: its routine entry's `doing`, or `waiting` for an
    /// agent with no routine.
    pub what: String,
    /// When it began.
    pub since: Tick,
    /// When it ends, if anything ends it.
    pub until: Option<Tick>,
}

impl_state_hash!(Activity { what, since, until });

/// An agent: a resident that is not loaded (09 §10.1). It costs nothing
/// between wake-ups (invariant N7).
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub struct AgentState {
    /// Where it is.
    pub at: AgentPlace,
    /// What it is doing.
    pub doing: Activity,
    /// The goal it was pursuing when it was stored, if it was not its
    /// routine's: what it was about (09 §10.4). Carried: long goals and
    /// their cold methods are not built.
    pub aim: Option<String>,
    /// When it next needs attention: the next part of its routine, or
    /// its arrival. `None` for one with no routine: it waits where it
    /// was.
    pub wake: Option<Tick>,
}

impl_state_hash!(AgentState {
    at,
    doing,
    aim,
    wake
});

/// One resident of a settlement (06 §12.2): a member of its population
/// that has a home and an index, so that it is the same person every
/// time it is made, whose individual comes and goes with the tracts.
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub struct Resident {
    /// What it is made from: the template the request named, as written.
    pub what: String,
    /// The map that asked for it, by qualified id, if it said.
    pub map: Option<String>,
    /// Its own seed: everything drawn for it when it is made (its name,
    /// its looks), its pick of places and its spot at each.
    pub seed: u64,
    /// The individual that stands for it, if one does: in the world, or
    /// stored as an agent. An anonymous one is made anew each time.
    pub entity: Option<EntityId>,
    /// While it is not loaded: its agent state.
    pub agent: Option<AgentState>,
    /// It died or was removed: it is never made again.
    pub gone: bool,
}

impl_state_hash!(Resident {
    what,
    map,
    seed,
    entity,
    agent,
    gone
});

/// What the simulation keeps for its agents and residents (09 §10).
#[derive(Clone, PartialEq, Debug, Default)]
pub struct AgentLayer {
    /// Every resident, by key.
    pub(super) residents: BTreeMap<ResidentKey, Resident>,
    /// Routines given by `set_routine`, by entity, in place of those its
    /// mind names.
    pub(super) routines: BTreeMap<EntityId, String>,
    /// Every cold resident's next wake-up, by tick and then key. One
    /// entry for each that has one (invariant N7).
    pub(super) schedule: BTreeSet<(Tick, ResidentKey)>,
    /// The one wake-up on the timeline: the earliest of the schedule.
    pub(super) wake: Option<(Tick, WakeId)>,
    /// Derived: the resident each individual stands for.
    pub(super) by_entity: BTreeMap<EntityId, ResidentKey>,
}

impl AgentLayer {
    /// Whether nothing is kept.
    pub fn is_empty(&self) -> bool {
        self.residents.is_empty() && self.routines.is_empty() && self.schedule.is_empty()
    }

    /// Every resident, by key.
    pub fn residents(&self) -> impl Iterator<Item = (&ResidentKey, &Resident)> + '_ {
        self.residents.iter()
    }

    /// A resident by key.
    pub fn resident(&self, key: &ResidentKey) -> Option<&Resident> {
        self.residents.get(key)
    }

    /// The resident an individual stands for.
    pub fn resident_of(&self, entity: EntityId) -> Option<(&ResidentKey, &Resident)> {
        let key = self.by_entity.get(&entity)?;
        Some((key, self.residents.get(key)?))
    }

    /// How many wake-ups the schedule holds: one for each cold resident
    /// that has one.
    pub fn scheduled(&self) -> usize {
        self.schedule.len()
    }

    /// The tick of the wake-up on the timeline, if there is one.
    pub fn timeline_wake(&self) -> Option<Tick> {
        self.wake.map(|(tick, _)| tick)
    }

    /// The routine `set_routine` gave an entity, if one did.
    pub fn routine_given(&self, entity: EntityId) -> Option<&str> {
        self.routines.get(&entity).map(String::as_str)
    }

    /// Rebuilds what is derived.
    pub(super) fn reindex(&mut self) {
        self.by_entity = self
            .residents
            .iter()
            .filter(|(_, resident)| !resident.gone)
            .filter_map(|(key, resident)| Some((resident.entity?, *key)))
            .collect();
    }
}

/// The saved form: everything but what is derived.
#[derive(Deserialize)]
struct Saved {
    residents: BTreeMap<ResidentKey, Resident>,
    routines: BTreeMap<EntityId, String>,
    schedule: BTreeSet<(Tick, ResidentKey)>,
    wake: Option<(Tick, WakeId)>,
}

/// The saved form, borrowed.
#[derive(Serialize)]
struct SavedRef<'a> {
    residents: &'a BTreeMap<ResidentKey, Resident>,
    routines: &'a BTreeMap<EntityId, String>,
    schedule: &'a BTreeSet<(Tick, ResidentKey)>,
    wake: Option<(Tick, WakeId)>,
}

impl Serialize for AgentLayer {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        SavedRef {
            residents: &self.residents,
            routines: &self.routines,
            schedule: &self.schedule,
            wake: self.wake,
        }
        .serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for AgentLayer {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<AgentLayer, D::Error> {
        let saved = Saved::deserialize(deserializer)?;
        let mut layer = AgentLayer {
            residents: saved.residents,
            routines: saved.routines,
            schedule: saved.schedule,
            wake: saved.wake,
            by_entity: BTreeMap::new(),
        };
        layer.reindex();
        Ok(layer)
    }
}

impl StateHash for AgentLayer {
    fn state_hash(&self, hasher: &mut StateHasher) {
        self.residents.state_hash(hasher);
        self.routines.state_hash(hasher);
        self.schedule.state_hash(hasher);
        self.wake.state_hash(hasher);
    }
}
