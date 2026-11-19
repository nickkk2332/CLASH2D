//! Relations (15 §9.1): values one mind holds toward a persona or a
//! group, of kinds content defines.
//!
//! **Reading.** What a mind holds toward an entity is what it holds
//! toward the persona it takes the entity for ([`persona_of`](super::persona_of)). Where it
//! holds nothing of a kind of its own, it uses its groups' (15 §9.2), in
//! this order, the first that holds anything counting:
//!
//! 1. its own relation toward the persona;
//! 2. the strongest of its own relations toward the groups the persona
//!    claims;
//! 3. the strongest of what its groups hold: each group's relation
//!    toward the persona, else the strongest of the group's relations
//!    toward the groups the persona claims (its own toward each before
//!    what its definition gives);
//! 4. the kind's default.
//!
//! The strongest of several values is the one furthest from the kind's
//! default, the first in order of key of equals: a watchman who is also
//! one of the townsfolk takes the watch's full hostility toward the gang
//! over the townsfolk's milder fear.
//!
//! **Fade.** A relation moves toward its kind's default by `fade` each
//! minute of the holder's own time (a group's, of universal time), worked
//! out when it is read; one that reaches its default is no longer held.

use std::sync::Arc;

use clash_core::{Dur, EntityId, Real, StateHash, StateHasher, impl_state_hash};
use clash_world::Cause;
use serde::{Deserialize, Serialize};

use super::defs::{GroupDef, RelationKindDef, RelationKindId};
use super::group::{self, claims_of};
use super::reason as why;
use super::{events, local_now, mind, mind_mut, tidy, universal_now};
use crate::Sim;
use crate::perception::PersonaId;
use crate::reason::Reason;

/// Whom a relation is held toward: a persona, or a group by key.
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Debug, Serialize, Deserialize)]
pub enum Toward {
    /// A persona.
    Persona(PersonaId),
    /// A group, by key.
    Group(String),
}

impl StateHash for Toward {
    fn state_hash(&self, hasher: &mut StateHasher) {
        match self {
            Toward::Persona(persona) => {
                hasher.write_u8(0);
                persona.state_hash(hasher);
            }
            Toward::Group(key) => {
                hasher.write_u8(1);
                key.state_hash(hasher);
            }
        }
    }
}

impl std::fmt::Display for Toward {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Toward::Persona(persona) => persona.fmt(f),
            Toward::Group(key) => f.write_str(key),
        }
    }
}

/// One relation: of a kind, toward someone, its value when it was set
/// and when that was, in the holder's own time.
#[derive(Clone, PartialEq, Debug, Serialize, Deserialize)]
pub struct Relation {
    /// Whom.
    pub toward: Toward,
    /// Of which kind.
    pub kind: RelationKindId,
    /// The value when it was set.
    pub value: Real,
    /// When it was set: the holder's local time, or universal time for a
    /// group's.
    pub at: Dur,
}

impl_state_hash!(Relation {
    toward,
    kind,
    value,
    at
});

/// How a mind stands toward someone, by the flags of the kinds of
/// relation it holds (09 §3.4).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Stance {
    /// A kind flagged `opposes` is held above its default, by more than
    /// any flagged `sides_with`.
    Opposes,
    /// A kind flagged `sides_with` is held above its default, by more
    /// than any flagged `opposes`.
    SidesWith,
    /// Neither.
    Neutral,
}

impl Stance {
    /// The word rules read: `opposes`, `sides_with` or `neutral`.
    pub const fn name(self) -> &'static str {
        match self {
            Stance::Opposes => "opposes",
            Stance::SidesWith => "sides_with",
            Stance::Neutral => "neutral",
        }
    }
}

/// A relation kind's definition.
pub fn kind_def(sim: &Sim, kind: RelationKindId) -> Option<&RelationKindDef> {
    sim.defs.try_registry::<RelationKindDef>()?.try_get(kind)
}

/// The relation kind a name stands for: a qualified id exactly; a bare
/// one in the first namespace, in sorted order, that has it.
pub fn kind_named(sim: &Sim, name: &str) -> Option<RelationKindId> {
    named::<RelationKindDef>(sim, name)
}

/// The definition of a type a name stands for, after loading: a
/// qualified id exactly; a bare one in the first namespace, in sorted
/// order, that has it.
pub(crate) fn named<T: clash_data::Def>(sim: &Sim, name: &str) -> Option<T::Id> {
    let registry = sim.defs.try_registry::<T>()?;
    if name.contains(':') {
        return registry.lookup(name);
    }
    let mut found: Vec<(&str, T::Id)> = registry
        .iter()
        .map(|(id, _)| (registry.name(id), id))
        .filter(|(qualified, _)| {
            qualified
                .split_once(':')
                .is_some_and(|(_, bare)| bare == name)
        })
        .collect();
    found.sort_unstable_by(|a, b| a.0.cmp(b.0));
    found.first().map(|(_, id)| *id)
}

/// The reason for a relation kind no definition answers to.
pub(crate) fn unknown_kind(name: &str) -> Reason {
    Reason::new(
        why::UNKNOWN_RELATION_KIND,
        format!("there is no relation kind `{name}`"),
    )
    .with("kind", name)
}

/// A relation's value at `now`: faded toward the kind's default by its
/// `fade` for every minute since it was set.
pub(crate) fn faded(relation: &Relation, def: &RelationKindDef, now: Dur) -> Real {
    let default = def.default_value();
    if def.fade <= Real::ZERO {
        return relation.value;
    }
    let minutes = ((now - relation.at).as_secs_real() / Real::of(60.0)).max(Real::ZERO);
    let moved = def.fade * minutes;
    if relation.value > default {
        (relation.value - moved).max(default)
    } else {
        (relation.value + moved).min(default)
    }
}

/// The value of a relation of a kind in a list, faded to `now`, if the
/// list holds one above or below its default.
fn found_in(
    relations: &[Relation],
    def: &RelationKindDef,
    kind: RelationKindId,
    toward: &Toward,
    now: Dur,
) -> Option<Real> {
    let at = relations
        .binary_search_by(|relation| (&relation.toward, relation.kind).cmp(&(toward, kind)))
        .ok()?;
    let value = faded(&relations[at], def, now);
    (value != def.default_value()).then_some(value)
}

/// [`found_in`] with the time worked out only if the list holds a
/// relation of the kind toward that one.
fn found_in_with(
    relations: &[Relation],
    def: &RelationKindDef,
    kind: RelationKindId,
    toward: &Toward,
    now: impl FnOnce() -> Dur,
) -> Option<Real> {
    let at = relations
        .binary_search_by(|relation| (&relation.toward, relation.kind).cmp(&(toward, kind)))
        .ok()?;
    let value = faded(&relations[at], def, now());
    (value != def.default_value()).then_some(value)
}

/// Of several values of a kind, the strongest: the one furthest from
/// the kind's default, the first of equals.
fn strongest(values: impl IntoIterator<Item = Real>, default: Real) -> Option<Real> {
    let mut best: Option<Real> = None;
    for value in values {
        if best.is_none_or(|best| (value - default).abs() > (best - default).abs()) {
            best = Some(value);
        }
    }
    best
}

/// What a group holds of a kind toward a persona: its own relation
/// toward it; else the strongest of what it holds toward the groups the
/// persona claims, the group's own relation toward each before what its
/// definition gives.
fn group_toward(
    sim: &Sim,
    key: &str,
    def: &RelationKindDef,
    kind: RelationKindId,
    persona: Option<PersonaId>,
    claims: &[String],
) -> Option<Real> {
    let now = universal_now(sim);
    let state = sim.social.groups.get(key);
    if let (Some(group), Some(persona)) = (state, persona)
        && let Some(value) = found_in(&group.relations, def, kind, &Toward::Persona(persona), now)
    {
        return Some(value);
    }
    let toward_claims = claims.iter().filter_map(|claim| {
        state
            .and_then(|group| {
                found_in(
                    &group.relations,
                    def,
                    kind,
                    &Toward::Group(claim.clone()),
                    now,
                )
            })
            .or_else(|| defined_toward(sim, key, kind, claim))
    });
    strongest(toward_claims, def.default_value())
}

/// What a group's definition gives it of a kind toward another group.
fn defined_toward(sim: &Sim, key: &str, kind: RelationKindId, toward: &str) -> Option<Real> {
    let def_id = group::def_of_key(sim, key)?;
    let registry = sim.defs.try_registry::<GroupDef>()?;
    let def = registry.try_get(def_id)?;
    let toward_def = group::def_of_key(sim, toward)?;
    let kind_name = sim.defs.try_registry::<RelationKindDef>()?.name(kind);
    let bare = kind_name
        .split_once(':')
        .map_or(kind_name, |(_, bare)| bare);
    def.relations.iter().find_map(|(group_name, values)| {
        if named::<GroupDef>(sim, group_name) != Some(toward_def) {
            return None;
        }
        values
            .iter()
            .find(|(name, _)| name.as_str() == kind_name || name.as_str() == bare)
            .map(|(_, value)| *value)
    })
}

/// What a mind holds of a kind toward a persona or a group, with its
/// groups' standing in where it holds nothing of its own (see the
/// [module documentation](self)). The kind's default where nothing is
/// held; `None` for a kind that does not exist.
pub fn rel_toward(
    sim: &Sim,
    mind_id: EntityId,
    kind: RelationKindId,
    toward: &Toward,
) -> Option<Real> {
    let asking = Asking::new(sim, mind_id, toward);
    rel_toward_with(sim, mind_id, kind, toward, &asking)
}

/// What looking up a mind's relations toward a persona or a group reads
/// whatever the kind: the groups it is in, whom it asks about, the
/// groups they claim, and the mind's own time, worked out when a
/// relation it holds is found. Worked out once for all the kinds a
/// question reads (D-926); the answers are those of each kind asked
/// alone.
struct Asking {
    persona: Option<PersonaId>,
    claims: Vec<String>,
    groups: Vec<String>,
    now: std::cell::OnceCell<Dur>,
}

impl Asking {
    fn new(sim: &Sim, mind_id: EntityId, toward: &Toward) -> Asking {
        let (persona, claims) = match toward {
            Toward::Persona(persona) => (Some(*persona), claims_of(sim, *persona)),
            Toward::Group(key) => (None, vec![key.clone()]),
        };
        Asking {
            persona,
            claims,
            groups: group::groups_of(sim, mind_id),
            now: std::cell::OnceCell::new(),
        }
    }

    /// The mind's own time now.
    fn now(&self, sim: &Sim, mind_id: EntityId) -> Dur {
        *self.now.get_or_init(|| local_now(sim, mind_id))
    }
}

/// [`rel_toward`] with what does not depend on the kind given.
fn rel_toward_with(
    sim: &Sim,
    mind_id: EntityId,
    kind: RelationKindId,
    toward: &Toward,
    asking: &Asking,
) -> Option<Real> {
    let defs = Arc::clone(&sim.defs);
    let def = defs.try_registry::<RelationKindDef>()?.try_get(kind)?;
    let now = || asking.now(sim, mind_id);
    let own: &[Relation] = mind(sim, mind_id).map_or(&[], |held| &held.relations);
    if let Some(value) = found_in_with(own, def, kind, toward, now) {
        return Some(value);
    }
    let (persona, claims) = (asking.persona, &asking.claims);
    if persona.is_some() {
        let toward_claims = claims
            .iter()
            .filter_map(|claim| found_in_with(own, def, kind, &Toward::Group(claim.clone()), now));
        if let Some(value) = strongest(toward_claims, def.default_value()) {
            return Some(value);
        }
    }
    let of_groups = asking
        .groups
        .iter()
        .filter_map(|key| group_toward(sim, key, def, kind, persona, claims));
    if let Some(value) = strongest(of_groups, def.default_value()) {
        return Some(value);
    }
    Some(def.default_value())
}

/// What a mind holds of a kind toward an entity: toward the persona it
/// takes it for (15 §9.1; `self.rel(kind, x)`).
pub fn rel(sim: &Sim, mind_id: EntityId, kind: RelationKindId, x: EntityId) -> Option<Real> {
    let persona = super::persona_of(sim, mind_id, x);
    rel_toward(sim, mind_id, kind, &Toward::Persona(persona))
}

/// How a mind stands toward someone (09 §3.4): by how far above their
/// defaults it holds the kinds flagged `opposes` and `sides_with`.
pub fn stance_toward(sim: &Sim, mind_id: EntityId, toward: &Toward) -> Stance {
    let Some(registry) = sim.defs.try_registry::<RelationKindDef>() else {
        return Stance::Neutral;
    };
    let (mut against, mut with) = (Real::ZERO, Real::ZERO);
    let mut asking: Option<Asking> = None;
    for (kind, def) in registry.iter() {
        if !def.opposes && !def.sides_with {
            continue;
        }
        let asking = asking.get_or_insert_with(|| Asking::new(sim, mind_id, toward));
        let excess = rel_toward_with(sim, mind_id, kind, toward, asking).unwrap_or(Real::ZERO)
            - def.default_value();
        if def.opposes {
            against = against.max(excess);
        } else {
            with = with.max(excess);
        }
    }
    if against > Real::ZERO && against >= with {
        Stance::Opposes
    } else if with > Real::ZERO {
        Stance::SidesWith
    } else {
        Stance::Neutral
    }
}

/// How a mind stands toward an entity, as the persona it takes it for.
pub fn stance_to(sim: &Sim, mind_id: EntityId, x: EntityId) -> Stance {
    let persona = super::persona_of(sim, mind_id, x);
    stance_toward(sim, mind_id, &Toward::Persona(persona))
}

/// Whether a mind holds toward an entity, as the persona it takes it
/// for, a relation of a kind flagged `intervenes` above its default
/// (15 §9.1): what makes it a candidate for windows opened against that
/// entity (03 §7.1; decision D-182). Whoever builds windows asks this.
pub fn intervenes(sim: &Sim, holder: EntityId, toward: EntityId) -> bool {
    holds_flagged(sim, holder, toward, |def| def.intervenes)
}

/// Whether a mind holds toward an entity, as the persona it takes it
/// for, a relation of a kind flagged `avoids` above its default: one to
/// keep away from, the AI's `feared` stance (09 §6.6).
pub fn avoids(sim: &Sim, holder: EntityId, toward: EntityId) -> bool {
    holds_flagged(sim, holder, toward, |def| def.avoids)
}

/// Whether a mind holds toward an entity a relation of a kind the test
/// picks above its default.
pub(crate) fn holds_flagged(
    sim: &Sim,
    holder: EntityId,
    toward: EntityId,
    flagged: impl Fn(&RelationKindDef) -> bool,
) -> bool {
    let persona = super::persona_of(sim, holder, toward);
    holds_flagged_toward(sim, holder, &Toward::Persona(persona), flagged)
}

/// Whether a mind holds toward a persona or a group a relation of a kind
/// the test picks above its default.
pub(crate) fn holds_flagged_toward(
    sim: &Sim,
    holder: EntityId,
    toward: &Toward,
    flagged: impl Fn(&RelationKindDef) -> bool,
) -> bool {
    let Some(registry) = sim.defs.try_registry::<RelationKindDef>() else {
        return false;
    };
    let mut asking: Option<Asking> = None;
    registry.iter().any(|(kind, def)| {
        flagged(def) && {
            let asking = asking.get_or_insert_with(|| Asking::new(sim, holder, toward));
            rel_toward_with(sim, holder, kind, toward, asking)
                .is_some_and(|value| value > def.default_value())
        }
    })
}

/// Sets what a mind holds of a kind toward someone (`set_relation`),
/// kept within the kind's range; at its default it is no longer held.
/// Raises `relation_changed` if the value changed. A kind flagged
/// `orders` that falls to its default withdraws the orders kept from
/// whom it was held toward (15 §10). Returns the value now.
pub fn set_relation(
    sim: &mut Sim,
    who: EntityId,
    toward: Toward,
    kind: RelationKindId,
    value: Real,
    cause: Cause,
) -> Result<Real, Reason> {
    sim.known(who)?;
    let defs = Arc::clone(&sim.defs);
    let def = defs
        .try_registry::<RelationKindDef>()
        .and_then(|registry| registry.try_get(kind))
        .ok_or_else(|| unknown_kind(&kind.index().to_string()))?;
    if !value.is_finite() {
        return Err(
            Reason::new(why::BAD_ARGUMENT, "a relation's value is a finite number")
                .with("argument", "value"),
        );
    }
    let (low, high) = def.bounds();
    let value = value.clamp(low, high);
    let now = local_now(sim, who);
    let was = own_value(sim, who, kind, &toward).unwrap_or(def.default_value());
    let held = mind_mut(sim, who);
    let at = held
        .relations
        .binary_search_by(|relation| (&relation.toward, relation.kind).cmp(&(&toward, kind)));
    if value == def.default_value() {
        if let Ok(at) = at {
            held.relations.remove(at);
        }
    } else {
        let relation = Relation {
            toward: toward.clone(),
            kind,
            value,
            at: now,
        };
        match at {
            Ok(at) => held.relations[at] = relation,
            Err(at) => held.relations.insert(at, relation),
        }
    }
    tidy(sim, who);
    if was != value {
        let name = defs
            .try_registry::<RelationKindDef>()
            .map_or(String::new(), |registry| registry.name(kind).to_owned());
        events::relation_changed(sim, who, &toward, &name, value, was, cause);
        if def.orders && value <= def.default_value() && was > def.default_value() {
            super::order::relation_lost(sim, who, &toward, cause);
        }
    }
    Ok(value)
}

/// What a mind holds of a kind toward someone of its own, faded to now:
/// `None` if it holds nothing.
pub fn own_value(sim: &Sim, who: EntityId, kind: RelationKindId, toward: &Toward) -> Option<Real> {
    let def = kind_def(sim, kind)?;
    let held = mind(sim, who)?;
    found_in(&held.relations, def, kind, toward, local_now(sim, who))
}

/// Adds to what a mind holds of a kind toward someone (`add_relation`):
/// to its own value, or the kind's default where it holds none.
pub fn add_relation(
    sim: &mut Sim,
    who: EntityId,
    toward: Toward,
    kind: RelationKindId,
    amount: Real,
    cause: Cause,
) -> Result<Real, Reason> {
    let current = own_value(sim, who, kind, &toward)
        .or_else(|| kind_def(sim, kind).map(RelationKindDef::default_value))
        .unwrap_or(Real::ZERO);
    set_relation(sim, who, toward, kind, current + amount, cause)
}

/// Takes away what a mind holds toward someone of its own
/// (`clear_relation`): of one kind, or of every kind. Returns how many
/// relations went.
pub fn clear_relation(
    sim: &mut Sim,
    who: EntityId,
    toward: Toward,
    kind: Option<RelationKindId>,
    cause: Cause,
) -> Result<usize, Reason> {
    sim.known(who)?;
    let kinds: Vec<RelationKindId> = mind(sim, who)
        .map(|held| {
            held.relations
                .iter()
                .filter(|relation| relation.toward == toward)
                .map(|relation| relation.kind)
                .filter(|have| kind.is_none_or(|kind| kind == *have))
                .collect()
        })
        .unwrap_or_default();
    for have in &kinds {
        let default = kind_def(sim, *have).map_or(Real::ZERO, RelationKindDef::default_value);
        set_relation(sim, who, toward.clone(), *have, default, cause)?;
    }
    Ok(kinds.len())
}
