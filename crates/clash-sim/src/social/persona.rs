//! Personas (15 §8): who someone presents as, and who an observer takes
//! them for.
//!
//! Every entity has a true persona ([`PersonaId::true_of`]), may make
//! others ([`create_persona`]), and presents one at a time
//! ([`presenting`]): its own, one of its own it made, or someone else's
//! ([`copy_persona`]). An observer takes it for what it presents unless
//! it has seen through it ([`persona_of`]).
//!
//! **Seeing through.** When an observer detects an entity that presents
//! a persona other than its true one, the hook `identity_check` decides
//! whether the disguise holds (`identify`); with no rule answering, it
//! does. Seen through, the observer takes the entity for its true
//! persona while it goes on perceiving it, and, where the persona is one
//! the entity made, links the two for good: a mask it has seen through
//! once fools it no more. A copy of someone else's persona is not
//! linked, because that persona is truly someone else's: being seen
//! through once, a stolen face is checked again at the next detection.
//! Whoever watches an entity change what it presents sees the change, and
//! goes on taking it for whom it took it for.

use clash_core::{EntityId, Real, StateHash, StateHasher, impl_state_hash};
use clash_world::{Cause, Location, OccupantKind};
use serde::{Deserialize, Serialize};

use super::reason as why;
use super::{SeenAs, events, mind, mind_mut, tidy};
use crate::Sim;
use crate::bus::EventPayload;
use crate::component::Identity;
use crate::entity::{EntitySpec, Place, Presence};
use crate::event::{Subject, Where};
use crate::perception::{PersonaId, SenseId, SignalKindId, belief};
use crate::reason::Reason;

/// How well a copy of someone's persona holds as a disguise when the
/// one who copies it gives no quality: a perfect likeness.
pub const COPY_QUALITY: Real = Real::ONE;

/// How well a persona made with `create_persona` holds as a disguise
/// when it is given no quality.
pub const DEFAULT_QUALITY: Real = Real::of(0.5);

/// How many links a mind follows from one persona to another before it
/// stops: a guard against links that go round.
const MAX_LINK_STEPS: usize = 8;

/// What a persona is beyond its number (15 §8.1): its name, pronouns,
/// tags, the groups it claims, how well it holds as a disguise and the
/// kind of thing it looks like. A true persona needs no record until
/// something is set on it; its name is then its owner's own.
#[derive(Clone, PartialEq, Debug, Default, Serialize, Deserialize)]
pub struct PersonaRecord {
    /// What it is called. A true persona with none goes by its owner's
    /// own name.
    pub name: Option<String>,
    /// The id of its set of pronouns, as written (05 §4.6). Kept for the
    /// `language` and `pronouns` types; text uses `they` until they are
    /// built.
    pub pronouns: Option<String>,
    /// Its tags: a uniform, an insignia, a role.
    pub tags: Vec<String>,
    /// The groups it presents as belonging to, by key (15 §9.2).
    pub claims: Vec<String>,
    /// How well it holds as a disguise, from 0 to 1: what
    /// `identity_check` is given as `quality`.
    pub quality: Real,
    /// The qualified id of the template whose ordinary body an observer
    /// takes it to have (15 §8.1). Left out, its owner's first template.
    pub apparent: Option<String>,
}

impl_state_hash!(PersonaRecord {
    name,
    pronouns,
    tags,
    claims,
    quality,
    apparent
});

/// What an entity presents: a persona, and how well it holds as a
/// disguise.
#[derive(Clone, Copy, PartialEq, Debug, Serialize, Deserialize)]
pub(crate) struct Presented {
    pub(crate) persona: PersonaId,
    pub(crate) quality: Real,
}

impl StateHash for Presented {
    fn state_hash(&self, hasher: &mut StateHasher) {
        self.persona.state_hash(hasher);
        self.quality.state_hash(hasher);
    }
}

/// What a new persona or a change to one is to be: each field left out
/// is left as it is, or as a new persona has it.
#[derive(Clone, PartialEq, Debug, Default)]
pub struct PersonaSpec {
    /// Its name.
    pub name: Option<String>,
    /// Its pronouns' id.
    pub pronouns: Option<String>,
    /// Its tags.
    pub tags: Option<Vec<String>>,
    /// The groups it claims, by key or definition id.
    pub claims: Option<Vec<String>>,
    /// How well it holds as a disguise, 0 to 1.
    pub quality: Option<Real>,
    /// The template it looks like, by qualified id.
    pub apparent: Option<String>,
}

fn bad(argument: &str, message: impl Into<String>) -> Reason {
    Reason::new(why::BAD_ARGUMENT, message).with("argument", argument)
}

fn unknown(persona: PersonaId) -> Reason {
    Reason::new(
        why::UNKNOWN_PERSONA,
        format!("there is no persona {persona}"),
    )
    .with("persona", persona.to_string())
}

/// What an entity presents (15 §8.1): its true persona, unless it has
/// assumed or copied another.
pub fn presenting(sim: &Sim, entity: EntityId) -> PersonaId {
    sim.social
        .presenting
        .get(&entity)
        .map_or(PersonaId::true_of(entity), |presented| presented.persona)
}

/// How well what an entity presents holds as a disguise.
fn presented_quality(sim: &Sim, entity: EntityId) -> Real {
    sim.social
        .presenting
        .get(&entity)
        .map_or(Real::ONE, |presented| presented.quality)
}

/// A persona's record, if it has one.
pub fn persona_record(sim: &Sim, persona: PersonaId) -> Option<&PersonaRecord> {
    sim.social.personas.get(&persona)
}

/// Whether a persona exists: a true persona of an entity that does, or
/// one made and kept.
pub fn persona_exists(sim: &Sim, persona: PersonaId) -> bool {
    if persona.is_true() {
        sim.entities.contains(persona.entity())
    } else {
        sim.social.personas.contains_key(&persona)
    }
}

/// The tags a persona presents (15 §8.1), in order: its record's, and
/// for a true persona its owner's own tags as well. A uniform's tags are
/// those it was made with.
pub fn persona_tags(sim: &Sim, persona: PersonaId) -> Vec<String> {
    let mut tags: Vec<String> = persona_record(sim, persona)
        .map(|record| record.tags.clone())
        .unwrap_or_default();
    if persona.is_true()
        && let Some(owner) = sim.entities.get(persona.entity())
    {
        tags.extend(owner.tags.iter().map(str::to_owned));
    }
    tags.sort();
    tags.dedup();
    tags
}

/// What a persona is called: its record's name, or for a true persona its
/// owner's own name. `None` for one with no name.
pub fn persona_name(sim: &Sim, persona: PersonaId) -> Option<String> {
    if let Some(name) = persona_record(sim, persona).and_then(|record| record.name.clone()) {
        return Some(name);
    }
    if persona.is_true() {
        return sim
            .components
            .identity
            .get(persona.entity())
            .and_then(Identity::name)
            .map(str::to_owned);
    }
    None
}

/// An entity's own personas (15 §8.3): its true one, then those made for
/// it, in the order they were made (each is given the next number of
/// the simulation's life, so the order of their ids).
pub fn own_personas(sim: &Sim, entity: EntityId) -> Vec<PersonaId> {
    std::iter::once(PersonaId::true_of(entity))
        .chain(
            sim.social
                .personas
                .range(PersonaId::made(entity, 1)..)
                .map(|(persona, _)| *persona)
                .take_while(|persona| persona.entity() == entity),
        )
        .collect()
}

/// The persona a mind takes another persona for, by the links it holds:
/// one it has seen through, followed to what it knows it to be.
fn resolved(sim: &Sim, mind_id: EntityId, persona: PersonaId) -> PersonaId {
    let Some(held) = mind(sim, mind_id) else {
        return persona;
    };
    let mut at = persona;
    for _ in 0..MAX_LINK_STEPS {
        match held.links.binary_search_by_key(&at, |(from, _)| *from) {
            Ok(found) => at = held.links[found].1,
            Err(_) => break,
        }
    }
    at
}

/// The persona a mind takes an entity for (15 §8.2): what it took it for
/// when it last detected it, while it presents the same; else what it
/// presents, followed through the links the mind holds. A mind takes
/// itself for its true persona.
pub fn persona_of(sim: &Sim, mind_id: EntityId, entity: EntityId) -> PersonaId {
    if mind_id == entity {
        return PersonaId::true_of(entity);
    }
    let shown = presenting(sim, entity);
    if let Some(held) = mind(sim, mind_id)
        && let Ok(at) = held
            .seen_as
            .binary_search_by_key(&entity, |seen| seen.entity)
        && held.seen_as[at].shown == shown
    {
        return held.seen_as[at].taken;
    }
    resolved(sim, mind_id, shown)
}

/// Records whom a mind takes an entity for while it presents `shown`.
fn record_seen(
    sim: &mut Sim,
    mind_id: EntityId,
    entity: EntityId,
    shown: PersonaId,
    taken: PersonaId,
) {
    let held = mind_mut(sim, mind_id);
    let seen = SeenAs {
        entity,
        shown,
        taken,
    };
    match held
        .seen_as
        .binary_search_by_key(&entity, |seen| seen.entity)
    {
        Ok(at) => held.seen_as[at] = seen,
        Err(at) => held.seen_as.insert(at, seen),
    }
}

/// Links, in a mind, a persona to what it knows it to be.
fn link(sim: &mut Sim, mind_id: EntityId, from: PersonaId, to: PersonaId) {
    if from == to {
        return;
    }
    let held = mind_mut(sim, mind_id);
    match held.links.binary_search_by_key(&from, |(have, _)| *have) {
        Ok(at) => held.links[at].1 = to,
        Err(at) => held.links.insert(at, (from, to)),
    }
}

/// An observer has just detected an entity (15 §8.2): whom it takes it
/// for is settled. Nothing is asked when the entity presents its true
/// persona, or one the observer has linked to it already; otherwise the
/// hook `identity_check` runs, and if a handler leaves `holds` false the
/// disguise is seen through.
pub(crate) fn identify(sim: &mut Sim, observer: EntityId, target: EntityId, sense: SenseId) {
    if observer == target {
        return;
    }
    let shown = presenting(sim, target);
    let true_persona = PersonaId::true_of(target);
    let known = resolved(sim, observer, shown);
    if shown == true_persona || known == true_persona {
        record_seen(sim, observer, target, shown, known);
        return;
    }
    let holds = if sim.bus.has_listeners(events::IDENTITY_CHECK) {
        let quality = presented_quality(sim, target);
        let scrutiny = sim
            .perception_defs
            .sense(sense)
            .map_or(Real::ZERO, |def| def.scrutiny);
        let familiarity = belief::track_of_persona(sim, observer, true_persona)
            .map_or(Real::ZERO, |track| track.confidence_then());
        let payload = EventPayload::new(events::IDENTITY_CHECK)
            .subject("self", Subject::Entity(observer))
            .subject("about", Subject::Entity(target))
            .with("sense", sim.perception_defs.sense_name(sense))
            .with("quality", quality)
            .with("scrutiny", scrutiny)
            .with("familiarity", familiarity)
            .with("holds", true);
        match sim.raise(
            Where::Entity(observer),
            Cause::new(clash_world::CauseKind::Process),
            payload,
        ) {
            Ok(raised) => raised
                .field("holds")
                .and_then(|value| value.plain())
                .and_then(clash_core::Value::as_bool)
                .unwrap_or(true),
            Err(reason) => {
                sim.fault(reason);
                true
            }
        }
    } else {
        true
    };
    if holds {
        record_seen(sim, observer, target, shown, known);
        return;
    }
    see_through(sim, observer, target, shown, "checked");
}

/// An observer sees through what an entity presents: it takes it for
/// its true persona, links a persona the entity made to it, and
/// `disguise_seen_through` is raised.
fn see_through(sim: &mut Sim, observer: EntityId, target: EntityId, shown: PersonaId, how: &str) {
    let true_persona = PersonaId::true_of(target);
    record_seen(sim, observer, target, shown, true_persona);
    if shown.entity() == target {
        link(sim, observer, shown, true_persona);
    }
    events::disguise_seen_through(sim, observer, target, shown, true_persona, how);
}

/// An observer perceives an entity through no sense any more: it takes
/// it for whatever it presents when next it detects it.
pub(crate) fn lost_sight(sim: &mut Sim, observer: EntityId, target: EntityId) {
    let Some(held) = sim.social.minds.get_mut(&observer) else {
        return;
    };
    if let Ok(at) = held
        .seen_as
        .binary_search_by_key(&target, |seen| seen.entity)
    {
        held.seen_as.remove(at);
    }
    tidy(sim, observer);
}

/// Every observer that perceives an entity now, in order of id.
fn watchers(sim: &Sim, entity: EntityId) -> Vec<EntityId> {
    let mut found: Vec<EntityId> = sim
        .components
        .awareness
        .iter()
        .filter(|(owner, awareness)| *owner != entity && awareness.detects(entity))
        .map(|(owner, _)| owner)
        .collect();
    found.sort_unstable();
    found
}

/// An entity presents another persona: whoever perceives it now sees the
/// change, and goes on taking it for whom it took it for.
fn present(sim: &mut Sim, entity: EntityId, persona: PersonaId, quality: Real) {
    let before: Vec<(EntityId, PersonaId)> = watchers(sim, entity)
        .into_iter()
        .map(|watcher| (watcher, persona_of(sim, watcher, entity)))
        .collect();
    if persona == PersonaId::true_of(entity) {
        sim.social.presenting.remove(&entity);
    } else {
        sim.social
            .presenting
            .insert(entity, Presented { persona, quality });
    }
    for (watcher, taken) in before {
        record_seen(sim, watcher, entity, persona, taken);
        if persona.entity() == entity && taken == PersonaId::true_of(entity) {
            link(sim, watcher, persona, taken);
        }
    }
}

/// Makes a persona for an entity (`create_persona`, 15 §8.3): a cover
/// identity, a costume. It presents it only once it assumes it.
pub fn create_persona(
    sim: &mut Sim,
    owner: EntityId,
    spec: PersonaSpec,
) -> Result<PersonaId, Reason> {
    sim.known(owner)?;
    let quality = spec.quality.unwrap_or(DEFAULT_QUALITY);
    if !quality.is_finite() || quality < Real::ZERO || quality > Real::ONE {
        return Err(bad("quality", "a persona's quality is from 0 to 1").with("value", quality));
    }
    let claims = match spec.claims {
        Some(claims) => keys_of(sim, &claims)?,
        None => Vec::new(),
    };
    let index = sim.social.next_persona.max(1);
    sim.social.next_persona = index + 1;
    let persona = PersonaId::made(owner, index);
    sim.social.personas.insert(
        persona,
        PersonaRecord {
            name: spec.name,
            pronouns: spec.pronouns,
            tags: spec.tags.unwrap_or_default(),
            claims,
            quality,
            apparent: spec.apparent,
        },
    );
    Ok(persona)
}

/// The keys of the groups a list names, each a key or a definition's id.
fn keys_of(sim: &Sim, names: &[String]) -> Result<Vec<String>, Reason> {
    let mut keys = Vec::new();
    for name in names {
        let key = super::group::group_key(sim, name).ok_or_else(|| {
            Reason::new(why::UNKNOWN_GROUP, format!("there is no group `{name}`"))
                .with("group", name.as_str())
        })?;
        keys.push(key);
    }
    keys.sort();
    keys.dedup();
    Ok(keys)
}

/// An entity presents one of its own personas (`assume_persona`): one it
/// made, or its true one. Refused for a persona that is someone else's or
/// gone.
pub fn assume_persona(sim: &mut Sim, who: EntityId, persona: PersonaId) -> Result<(), Reason> {
    sim.known(who)?;
    if persona.entity() != who {
        return Err(Reason::new(
            why::NOT_OWN_PERSONA,
            format!("{persona} is not {who}'s own persona"),
        )
        .with("entity", who.to_string())
        .with("persona", persona.to_string()));
    }
    if !persona_exists(sim, persona) {
        return Err(unknown(persona));
    }
    let quality = persona_record(sim, persona).map_or(Real::ONE, |record| record.quality);
    present(sim, who, persona, quality);
    Ok(())
}

/// An entity presents its true persona again (`drop_persona`).
pub fn drop_persona(sim: &mut Sim, who: EntityId) -> Result<(), Reason> {
    sim.known(who)?;
    present(sim, who, PersonaId::true_of(who), Real::ONE);
    Ok(())
}

/// An entity presents what another presents (`copy_persona`, 15 §8.3): a
/// shapeshifter, a perfect mask. `quality` is how well the likeness
/// holds, [`COPY_QUALITY`] if left out. Returns the persona presented.
pub fn copy_persona(
    sim: &mut Sim,
    who: EntityId,
    of: EntityId,
    quality: Option<Real>,
) -> Result<PersonaId, Reason> {
    sim.known(who)?;
    sim.known(of)?;
    let quality = quality.unwrap_or(COPY_QUALITY);
    if !quality.is_finite() || quality < Real::ZERO || quality > Real::ONE {
        return Err(bad("quality", "a likeness's quality is from 0 to 1").with("value", quality));
    }
    let persona = presenting(sim, of);
    present(sim, who, persona, quality);
    Ok(persona)
}

/// A mind learns that what an entity presents is that entity
/// (`reveal_persona`, 15 §8.3): it takes it for its true persona while
/// it perceives it, and a persona the entity made is linked to it for
/// good. Raises `disguise_seen_through`. Returns whether anything was
/// hidden: false when the entity presents its true persona or the mind
/// knew already.
pub fn reveal_persona(sim: &mut Sim, mind_id: EntityId, of: EntityId) -> Result<bool, Reason> {
    sim.known(mind_id)?;
    sim.known(of)?;
    if mind_id == of {
        return Ok(false);
    }
    let shown = presenting(sim, of);
    if persona_of(sim, mind_id, of) == PersonaId::true_of(of) {
        return Ok(false);
    }
    see_through(sim, mind_id, of, shown, "revealed");
    Ok(true)
}

/// Sets what a persona of an entity is (`set_persona`): its name,
/// pronouns, tags, claims, quality and apparent kind; each left out is
/// left as it is. `persona` left out is the one the entity presents, if
/// it is its own, else its true one.
pub fn set_persona(
    sim: &mut Sim,
    who: EntityId,
    persona: Option<PersonaId>,
    spec: PersonaSpec,
) -> Result<PersonaId, Reason> {
    sim.known(who)?;
    let persona = persona.unwrap_or_else(|| {
        let shown = presenting(sim, who);
        if shown.entity() == who {
            shown
        } else {
            PersonaId::true_of(who)
        }
    });
    if persona.entity() != who {
        return Err(Reason::new(
            why::NOT_OWN_PERSONA,
            format!("{persona} is not {who}'s own persona"),
        )
        .with("entity", who.to_string())
        .with("persona", persona.to_string()));
    }
    if !persona_exists(sim, persona) {
        return Err(unknown(persona));
    }
    if let Some(quality) = spec.quality
        && (!quality.is_finite() || quality < Real::ZERO || quality > Real::ONE)
    {
        return Err(bad("quality", "a persona's quality is from 0 to 1").with("value", quality));
    }
    let claims = match &spec.claims {
        Some(claims) => Some(keys_of(sim, claims)?),
        None => None,
    };
    let record = sim
        .social
        .personas
        .entry(persona)
        .or_insert_with(|| PersonaRecord {
            quality: Real::ONE,
            ..PersonaRecord::default()
        });
    if let Some(name) = spec.name {
        record.name = Some(name);
    }
    if let Some(pronouns) = spec.pronouns {
        record.pronouns = Some(pronouns);
    }
    if let Some(tags) = spec.tags {
        record.tags = tags;
    }
    if let Some(claims) = claims {
        record.claims = claims;
    }
    if let Some(quality) = spec.quality {
        record.quality = quality;
    }
    if let Some(apparent) = spec.apparent {
        record.apparent = Some(apparent);
    }
    let quality = record.quality;
    if let Some(presented) = sim.social.presenting.get_mut(&who)
        && presented.persona == persona
    {
        presented.quality = quality;
    }
    Ok(persona)
}

/// A mind learns the name of whom it takes an entity for: it is named,
/// not described, in text it reads from now on (05 §4.6).
pub fn learn_name(sim: &mut Sim, mind_id: EntityId, of: EntityId) -> Result<(), Reason> {
    sim.known(mind_id)?;
    sim.known(of)?;
    let persona = persona_of(sim, mind_id, of);
    learn_persona_name(sim, mind_id, persona);
    Ok(())
}

/// A mind learns the name of a persona.
pub(crate) fn learn_persona_name(sim: &mut Sim, mind_id: EntityId, persona: PersonaId) {
    let held = mind_mut(sim, mind_id);
    if let Err(at) = held.names.binary_search(&persona) {
        held.names.insert(at, persona);
    }
}

/// Whether a mind knows a persona's name: its own true persona's, or one
/// it learnt.
pub fn knows_name(sim: &Sim, mind_id: EntityId, persona: PersonaId) -> bool {
    persona == PersonaId::true_of(mind_id)
        || mind(sim, mind_id).is_some_and(|held| held.names.binary_search(&persona).is_ok())
}

/// Makes an entity that is only signals (`create_illusion`, 15 §8.3): it
/// stands in a cell, gives off the signals given and nothing else, has
/// no body and blocks nothing. Presenting what `of` presents, it is
/// taken for them. Tagged `illusion`.
pub fn create_illusion(
    sim: &mut Sim,
    at: Location,
    of: Option<EntityId>,
    signals: &[(SignalKindId, Real)],
    cause: Cause,
) -> Result<EntityId, Reason> {
    if let Some(of) = of {
        sim.known(of)?;
    }
    for (_, strength) in signals {
        if !strength.is_finite() || *strength < Real::ZERO {
            return Err(
                bad("signals", "a signal's strength is a number of zero or more")
                    .with("value", *strength),
            );
        }
    }
    let spec = EntitySpec::new(OccupantKind::Other)
        .with_presence(Presence::new(OccupantKind::Other))
        .with_tag("illusion");
    let entity = sim.spawn(spec, Place::at(at), cause)?;
    for (kind, strength) in signals {
        crate::perception::set_signature(sim, entity, *kind, *strength)?;
    }
    if let Some(of) = of {
        let persona = presenting(sim, of);
        present(sim, entity, persona, COPY_QUALITY);
    }
    Ok(entity)
}
