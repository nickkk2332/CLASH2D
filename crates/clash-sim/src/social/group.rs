//! Groups (15 §9.2): bodies of people with roles, relations of their
//! own, claims and shared knowledge.
//!
//! A group is named by its **key**: a `group` definition's group by the
//! definition's qualified id (`core:watch`), one made in play by
//! `create_group` by the definition's id and a number
//! (`core:gang#1`). A definition's group exists from the start; what it
//! keeps of its own (members, relations, knowledge) is made when first
//! needed.
//!
//! **Members** are entities. A member's true persona **claims** the
//! group unless it joined in secret: wearing the colours, everyone who
//! sees it takes it for one of them. A persona made with
//! `create_persona` claims what it is given.
//!
//! **Knowledge.** What a member reports to its group is added to the
//! group's knowledge and counts as told, at once, to every other member
//! (decision D-240), and to whoever joins later.

use std::collections::BTreeMap;

use clash_core::{EntityId, Real, impl_state_hash};
use clash_world::{Cause, Location};
use serde::{Deserialize, Serialize};

use super::defs::{GroupDef, GroupDefId, RelationKindId};
use super::reason as why;
use super::relation::{self, Relation, Toward, named};
use super::{events, local_now, memories_of, universal_now};
use crate::Sim;
use crate::bus::{Delivery, FieldValue};
use crate::perception::{Knowing, Memory, PersonaId, belief};
use crate::reason::Reason;

/// One member of a group.
#[derive(Clone, PartialEq, Eq, Debug, Default, Serialize, Deserialize)]
pub struct Member {
    /// Its role, if it has one.
    pub role: Option<String>,
    /// Whether it belongs in secret: its true persona does not claim the
    /// group.
    pub secret: bool,
}

impl_state_hash!(Member { role, secret });

/// What a group knows of a persona's whereabouts.
#[derive(Clone, PartialEq, Debug, Serialize, Deserialize)]
pub struct GroupTrack {
    /// Whom.
    pub persona: PersonaId,
    /// Where it was last known to be.
    pub loc: Option<Location>,
    /// How sure the reporter was.
    pub confidence: Real,
}

impl_state_hash!(GroupTrack {
    persona,
    loc,
    confidence
});

/// What a group keeps of its own (15 §9.2).
#[derive(Clone, PartialEq, Debug, Serialize, Deserialize)]
pub struct Group {
    /// Its definition.
    pub(crate) def: GroupDefId,
    /// The name it was made with in play, if any.
    pub(crate) name: Option<String>,
    /// Its members, by entity.
    pub(crate) members: BTreeMap<EntityId, Member>,
    /// What it holds toward personas and groups, beyond its definition,
    /// in order of whom and kind; set in universal time.
    pub(crate) relations: Vec<Relation>,
    /// What it knows happened, in the order reported.
    pub(crate) memories: Vec<Memory>,
    /// What it knows of where people are, in order of persona.
    pub(crate) tracks: Vec<GroupTrack>,
}

impl_state_hash!(Group {
    def,
    name,
    members,
    relations,
    memories,
    tracks
});

impl Group {
    /// Its definition.
    pub fn def(&self) -> GroupDefId {
        self.def
    }

    /// Its members, by entity.
    pub fn members(&self) -> &BTreeMap<EntityId, Member> {
        &self.members
    }

    /// What it knows happened, in the order reported.
    pub fn memories(&self) -> &[Memory] {
        &self.memories
    }

    /// What it knows of where people are.
    pub fn tracks(&self) -> &[GroupTrack] {
        &self.tracks
    }
}

fn unknown_group(name: &str) -> Reason {
    Reason::new(why::UNKNOWN_GROUP, format!("there is no group `{name}`")).with("group", name)
}

/// The key of the group a name stands for: a group made in play by its
/// key, or a definition's by its id, qualified or bare. `None` for one
/// that does not exist or was dissolved.
pub fn group_key(sim: &Sim, name: &str) -> Option<String> {
    if sim.social.groups.contains_key(name) && !sim.social.dissolved.iter().any(|key| key == name) {
        return Some(name.to_owned());
    }
    if name.contains('#') {
        return None;
    }
    let id = named::<GroupDef>(sim, name)?;
    let key = sim.defs.try_registry::<GroupDef>()?.name(id).to_owned();
    (!sim.social.dissolved.contains(&key)).then_some(key)
}

/// The definition of the group a key names.
pub(crate) fn def_of_key(sim: &Sim, key: &str) -> Option<GroupDefId> {
    if let Some(group) = sim.social.groups.get(key) {
        return Some(group.def);
    }
    sim.defs.try_registry::<GroupDef>()?.lookup(key)
}

/// The group a key names, to change: what it keeps is made if it has
/// nothing yet.
fn group_mut<'a>(sim: &'a mut Sim, key: &str) -> Result<&'a mut Group, Reason> {
    if !sim.social.groups.contains_key(key) {
        let def = def_of_key(sim, key).ok_or_else(|| unknown_group(key))?;
        sim.social.groups.insert(
            key.to_owned(),
            Group {
                def,
                name: None,
                members: BTreeMap::new(),
                relations: Vec::new(),
                memories: Vec::new(),
                tracks: Vec::new(),
            },
        );
    }
    sim.social
        .groups
        .get_mut(key)
        .ok_or_else(|| unknown_group(key))
}

/// The key of a group a name stands for, or the reason there is none.
pub(crate) fn require(sim: &Sim, name: &str) -> Result<String, Reason> {
    group_key(sim, name).ok_or_else(|| unknown_group(name))
}

/// The groups an entity belongs to, by key, in order.
pub fn groups_of(sim: &Sim, entity: EntityId) -> Vec<String> {
    sim.social
        .groups
        .iter()
        .filter(|(_, group)| group.members.contains_key(&entity))
        .map(|(key, _)| key.clone())
        .collect()
}

/// The role an entity has in a group, if it is a member with one.
pub fn role_in(sim: &Sim, entity: EntityId, key: &str) -> Option<String> {
    sim.social
        .groups
        .get(key)?
        .members
        .get(&entity)?
        .role
        .clone()
}

/// The groups a persona presents as belonging to (15 §9.2), by key, in
/// order: what its record claims, and for a true persona every group its
/// owner belongs to openly.
pub fn claims_of(sim: &Sim, persona: PersonaId) -> Vec<String> {
    let mut claims: Vec<String> = super::persona::persona_record(sim, persona)
        .map(|record| record.claims.clone())
        .unwrap_or_default();
    if persona.is_true() {
        for (key, group) in &sim.social.groups {
            if group
                .members
                .get(&persona.entity())
                .is_some_and(|member| !member.secret)
            {
                claims.push(key.clone());
            }
        }
    }
    claims.sort();
    claims.dedup();
    claims
}

/// Checks a role against the group's definition.
fn check_role(sim: &Sim, key: &str, role: Option<&str>) -> Result<(), Reason> {
    let Some(role) = role else {
        return Ok(());
    };
    let def = def_of_key(sim, key)
        .and_then(|id| sim.defs.try_registry::<GroupDef>()?.try_get(id))
        .ok_or_else(|| unknown_group(key))?;
    if def.roles.contains_key(role) {
        Ok(())
    } else {
        Err(Reason::new(
            why::UNKNOWN_ROLE,
            format!("the group `{key}` has no role `{role}`"),
        )
        .with("group", key)
        .with("role", role))
    }
}

/// An entity joins a group (`join_group`), with a role or none, openly
/// or in secret, and is told what the group knows. Raises `group_joined`;
/// joining again changes the role and the secrecy.
pub fn join_group(
    sim: &mut Sim,
    who: EntityId,
    name: &str,
    role: Option<&str>,
    secret: bool,
    cause: Cause,
) -> Result<(), Reason> {
    sim.known(who)?;
    let key = require(sim, name)?;
    check_role(sim, &key, role)?;
    let member = Member {
        role: role.map(str::to_owned),
        secret,
    };
    let was = group_mut(sim, &key)?.members.insert(who, member);
    if was.is_none() {
        events::group_joined(sim, who, &key, role, cause);
        tell_member(sim, &key, who, None);
    }
    Ok(())
}

/// An entity leaves a group (`leave_group`). Raises `group_left`.
/// Returns whether it was a member.
pub fn leave_group(sim: &mut Sim, who: EntityId, name: &str, cause: Cause) -> Result<bool, Reason> {
    sim.known(who)?;
    let key = require(sim, name)?;
    let gone = sim
        .social
        .groups
        .get_mut(&key)
        .and_then(|group| group.members.remove(&who));
    if let Some(member) = &gone {
        events::group_left(sim, who, &key, member.role.as_deref(), cause);
    }
    Ok(gone.is_some())
}

/// Gives a member of a group another role (`set_role`), or none. Raises
/// `role_changed`.
pub fn set_role(
    sim: &mut Sim,
    who: EntityId,
    name: &str,
    role: Option<&str>,
    cause: Cause,
) -> Result<(), Reason> {
    sim.known(who)?;
    let key = require(sim, name)?;
    check_role(sim, &key, role)?;
    let member = sim
        .social
        .groups
        .get_mut(&key)
        .and_then(|group| group.members.get_mut(&who))
        .ok_or_else(|| {
            Reason::new(why::NOT_MEMBER, format!("{who} is not a member of `{key}`"))
                .with("entity", who.to_string())
                .with("group", key.as_str())
        })?;
    let was = core::mem::replace(&mut member.role, role.map(str::to_owned));
    if was.as_deref() != role {
        events::role_changed(sim, who, &key, role, was.as_deref(), cause);
    }
    Ok(())
}

/// Makes a group in play (`create_group`): of a definition, with a name
/// if given. Raises `group_created`. Returns its key.
pub fn create_group(
    sim: &mut Sim,
    def_name: &str,
    name: Option<&str>,
    cause: Cause,
) -> Result<String, Reason> {
    let def = named::<GroupDef>(sim, def_name).ok_or_else(|| unknown_group(def_name))?;
    let qualified = sim
        .defs
        .try_registry::<GroupDef>()
        .map_or(String::new(), |registry| registry.name(def).to_owned());
    sim.social.next_group += 1;
    let key = format!("{qualified}#{}", sim.social.next_group);
    sim.social.groups.insert(
        key.clone(),
        Group {
            def,
            name: name.map(str::to_owned),
            members: BTreeMap::new(),
            relations: Vec::new(),
            memories: Vec::new(),
            tracks: Vec::new(),
        },
    );
    events::group_created(sim, &key, &qualified, cause);
    Ok(key)
}

/// Dissolves a group (`dissolve_group`): every member leaves it, with
/// `group_left`, and what it kept goes. Raises `group_dissolved`.
pub fn dissolve_group(sim: &mut Sim, name: &str, cause: Cause) -> Result<(), Reason> {
    let key = require(sim, name)?;
    let members: Vec<(EntityId, Member)> = sim
        .social
        .groups
        .get(&key)
        .map(|group| group.members.clone().into_iter().collect())
        .unwrap_or_default();
    for (member, record) in members {
        events::group_left(sim, member, &key, record.role.as_deref(), cause);
    }
    sim.social.groups.remove(&key);
    if !key.contains('#') {
        sim.social.dissolved.push(key.clone());
        sim.social.dissolved.sort();
    }
    events::group_dissolved(sim, &key, cause);
    Ok(())
}

/// Sets what a group holds of a kind toward a persona or another group
/// (`set_group_relation`), kept within the kind's range. Returns the
/// value now.
pub fn set_group_relation(
    sim: &mut Sim,
    name: &str,
    toward: Toward,
    kind: RelationKindId,
    value: Real,
) -> Result<Real, Reason> {
    let key = require(sim, name)?;
    let def = relation::kind_def(sim, kind)
        .ok_or_else(|| relation::unknown_kind(&kind.index().to_string()))?;
    if !value.is_finite() {
        return Err(
            Reason::new(why::BAD_ARGUMENT, "a relation's value is a finite number")
                .with("argument", "value"),
        );
    }
    let (low, high) = def.bounds();
    let value = value.clamp(low, high);
    let default = def.default_value();
    let now = universal_now(sim);
    let group = group_mut(sim, &key)?;
    let at = group
        .relations
        .binary_search_by(|relation| (&relation.toward, relation.kind).cmp(&(&toward, kind)));
    match (at, value == default) {
        (Ok(at), true) => {
            group.relations.remove(at);
        }
        (Err(_), true) => {}
        (Ok(at), false) => {
            group.relations[at] = Relation {
                toward,
                kind,
                value,
                at: now,
            };
        }
        (Err(at), false) => group.relations.insert(
            at,
            Relation {
                toward,
                kind,
                value,
                at: now,
            },
        ),
    }
    Ok(value)
}

/// A member reports to its group (`report_to_group`, 15 §11): its
/// memories of an event type, or naming whom it takes `about` for, or
/// all of them if neither is given, and its track of `about`, are added
/// to the group's knowledge and told to every other member, as told by
/// the reporter. Returns how many things were added.
pub fn report_to_group(
    sim: &mut Sim,
    who: EntityId,
    name: &str,
    event: Option<&str>,
    about: Option<EntityId>,
) -> Result<usize, Reason> {
    sim.known(who)?;
    let key = require(sim, name)?;
    if !sim
        .social
        .groups
        .get(&key)
        .is_some_and(|group| group.members.contains_key(&who))
    {
        return Err(
            Reason::new(why::NOT_MEMBER, format!("{who} is not a member of `{key}`"))
                .with("entity", who.to_string())
                .with("group", key.as_str()),
        );
    }
    let about_persona = about.map(|about| super::persona_of(sim, who, about));
    let memories: Vec<Memory> = memories_of(sim, who)
        .iter()
        .filter(|memory| event.is_none_or(|event| memory.event == event))
        .filter(|memory| {
            about_persona.is_none_or(|persona| {
                memory
                    .roles
                    .iter()
                    .any(|(_, played)| *played == Some(persona))
            })
        })
        .cloned()
        .collect();
    let track = about_persona.and_then(|persona| {
        let track = belief::track_of_persona(sim, who, persona)?;
        Some(GroupTrack {
            persona,
            loc: track.loc(),
            confidence: track.confidence_then(),
        })
    });
    let group = group_mut(sim, &key)?;
    let mut added = Vec::new();
    for memory in memories {
        if !group.memories.contains(&memory) {
            group.memories.push(memory.clone());
            added.push(memory);
        }
    }
    let mut told_track = None;
    if let Some(track) = track {
        match group
            .tracks
            .binary_search_by_key(&track.persona, |have| have.persona)
        {
            Ok(at) => group.tracks[at] = track.clone(),
            Err(at) => group.tracks.insert(at, track.clone()),
        }
        told_track = Some(track);
    }
    let count = added.len() + usize::from(told_track.is_some());
    let members: Vec<EntityId> = sim
        .social
        .groups
        .get(&key)
        .map(|group| group.members.keys().copied().collect())
        .unwrap_or_default();
    for member in members {
        if member == who {
            continue;
        }
        let source = Some(super::persona_of(sim, member, who));
        for memory in &added {
            tell_memory(sim, member, memory, source);
        }
        if let Some(track) = &told_track {
            belief::put_persona_track(
                sim,
                member,
                track.persona,
                track.loc,
                Knowing::Told,
                track.confidence,
                source,
            );
        }
    }
    Ok(count)
}

/// A mind is told a memory: it remembers it as told, now, from a source.
fn tell_memory(sim: &mut Sim, mind: EntityId, memory: &Memory, source: Option<PersonaId>) {
    let told = Memory {
        at: local_now(sim, mind),
        how: Knowing::Told,
        source,
        ..memory.clone()
    };
    let _ = belief::remember(sim, mind, told);
}

/// A new member is told what its group knows, as told by the group.
fn tell_member(sim: &mut Sim, key: &str, member: EntityId, source: Option<PersonaId>) {
    let Some(group) = sim.social.groups.get(key) else {
        return;
    };
    let (memories, tracks) = (group.memories.clone(), group.tracks.clone());
    for memory in &memories {
        tell_memory(sim, member, memory, source);
    }
    for track in tracks {
        belief::put_persona_track(
            sim,
            member,
            track.persona,
            track.loc,
            Knowing::Told,
            track.confidence,
            source,
        );
    }
}

/// `entity_spawned`: the made entity joins the groups its templates name
/// (`groups` on a template: group id to role, `""` for none).
pub(crate) fn spawned(sim: &mut Sim, delivery: &mut Delivery<'_>) {
    let Some(crate::event::Subject::Entity(entity)) = delivery.event().subject("self") else {
        return;
    };
    let Some(FieldValue::Plain(clash_core::Value::List(names))) = delivery.field("templates")
    else {
        return;
    };
    let cause = delivery.event().cause;
    let defs = std::sync::Arc::clone(&sim.defs);
    let Some(templates) = defs.try_registry::<crate::template::TemplateDef>() else {
        return;
    };
    for name in names {
        let Some(name) = name.as_str() else {
            continue;
        };
        let Some(template) = templates.lookup(name).and_then(|id| templates.try_get(id)) else {
            continue;
        };
        for (group, role) in &template.groups {
            if group_key(sim, group).is_none() {
                // Reported when the definitions loaded (`check_template_groups`).
                continue;
            }
            let role = (!role.is_empty()).then_some(role.as_str());
            if let Err(reason) = join_group(sim, entity, group, role, false, cause) {
                sim.fault(reason.with("entity", entity.to_string()));
            }
        }
    }
}
