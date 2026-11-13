//! Know-how (16 §8.5; package G17): what someone knows how to make, a
//! set of named things held by a mind.
//!
//! The set is the [`KnowHow`] component of the entity whose mind it is,
//! and it goes where the mind goes: `transfer_mind` moves it with the
//! mind, and the body left behind keeps none. It is gained from a
//! template's `know_how` when something is made from it, by `teach`,
//! which is refused while what it `needs` is not known, and by reading:
//! `learn_from` teaches what an item `teaches`. `unlearn` takes it
//! away.
//!
//! A recipe that needs know-how is met by the worker, or by its station:
//! what the station's definition `provides` in its mode now, and what
//! the items loaded into it provide. A machine working alone has only
//! the latter two.

use clash_core::{EntityId, impl_state_hash};
use clash_world::Cause;
use serde::{Deserialize, Serialize};

use super::defs::{KnowHowDef, KnowHowId};
use super::{KNOW_HOW_GAINED, KNOW_HOW_LOST, codes};
use crate::Sim;
use crate::bus::{EventPayload, FieldValue};
use crate::event::{Subject, Where};
use crate::item::Item;
use crate::reason::Reason;

/// What a mind knows how to do (16 §8.5): the `KnowHow` component.
#[derive(Clone, PartialEq, Eq, Debug, Default, Serialize, Deserialize)]
pub struct KnowHow {
    /// What it knows, in order of id, each once.
    known: Vec<KnowHowId>,
}

impl_state_hash!(KnowHow { known });

impl KnowHow {
    /// What it knows, in order of id.
    pub fn known(&self) -> &[KnowHowId] {
        &self.known
    }

    /// Whether it knows one.
    pub fn knows(&self, id: KnowHowId) -> bool {
        self.known.binary_search(&id).is_ok()
    }

    fn insert(&mut self, id: KnowHowId) -> bool {
        match self.known.binary_search(&id) {
            Ok(_) => false,
            Err(at) => {
                self.known.insert(at, id);
                true
            }
        }
    }

    fn remove(&mut self, id: KnowHowId) -> bool {
        match self.known.binary_search(&id) {
            Ok(at) => {
                self.known.remove(at);
                true
            }
            Err(_) => false,
        }
    }
}

/// The know-how definitions, if loaded.
fn registry(sim: &Sim) -> Option<&clash_data::Registry<KnowHowDef>> {
    sim.defs.try_registry::<KnowHowDef>()
}

/// The qualified id of a piece of know-how.
pub fn know_how_name(sim: &Sim, id: KnowHowId) -> String {
    registry(sim).map_or_else(String::new, |registry| registry.name(id).to_owned())
}

/// What a reader calls a piece of know-how: its `name`, else its id.
pub fn know_how_label(sim: &Sim, id: KnowHowId) -> String {
    let Some(def) = registry(sim).and_then(|registry| registry.try_get(id)) else {
        return String::new();
    };
    super::label(sim, def.name.as_ref(), &know_how_name(sim, id))
}

/// The know-how a qualified id names.
pub fn lookup(sim: &Sim, name: &str) -> Option<KnowHowId> {
    registry(sim)?.lookup(name)
}

/// Whether an entity knows something (16 §8.5 `knows_how`).
pub fn knows(sim: &Sim, who: EntityId, id: KnowHowId) -> bool {
    sim.components
        .get::<KnowHow>(who)
        .is_some_and(|known| known.knows(id))
}

/// What an entity knows, in order of id.
pub fn known_by(sim: &Sim, who: EntityId) -> Vec<KnowHowId> {
    sim.components
        .get::<KnowHow>(who)
        .map_or_else(Vec::new, |known| known.known.clone())
}

/// What a station provides: what its definition provides in its mode
/// now, and what the items in it provide, in that order, each once.
pub fn provided_at(sim: &Sim, station: Option<crate::interaction::Target>) -> Vec<KnowHowId> {
    let Some(target) = station else {
        return Vec::new();
    };
    let mut found = crate::device::provided_by_def(sim, target);
    if let Some(device) = crate::device::device_now(sim, target) {
        for inside in sim.entities.contents_deep(device) {
            let Some(def) = sim
                .components
                .get::<Item>(inside)
                .and_then(|item| crate::item::def_of(&sim.defs, item))
            else {
                continue;
            };
            for provided in &def.provides {
                if !found.contains(&provided.id()) {
                    found.push(provided.id());
                }
            }
        }
    }
    found
}

fn unknown(name: &str) -> Reason {
    Reason::new(
        codes::UNKNOWN_KNOW_HOW,
        format!("there is no know-how `{name}`"),
    )
    .with("know_how", name)
}

/// The know-how an expression value names: an id of the type, or its
/// qualified id as a text.
pub fn know_how_of(sim: &Sim, value: &clash_expr::Value) -> Option<KnowHowId> {
    match value {
        clash_expr::Value::Id(id)
            if sim.exprs.registry().def_type_id(<KnowHowDef as clash_data::Def>::TYPE)
                == Some(id.ty) =>
        {
            u16::try_from(id.index).ok().map(KnowHowId::new)
        }
        clash_expr::Value::Str(name) => lookup(sim, name),
        _ => None,
    }
}

fn raise(sim: &mut Sim, name: &'static str, who: EntityId, id: KnowHowId, how: &str, cause: Cause) {
    let payload = EventPayload::new(name)
        .subject("self", Subject::Entity(who))
        .with("know_how", FieldValue::text(know_how_name(sim, id)))
        .with("how", FieldValue::text(how));
    let _ = sim.raise(Where::Entity(who), cause, payload);
}

/// Gives an entity a piece of know-how without asking what it needs:
/// what a template lists. Raises `know_how_gained` with `how`. Returns
/// whether it is new to it.
pub fn grant(sim: &mut Sim, who: EntityId, id: KnowHowId, how: &str, cause: Cause) -> bool {
    if sim.mind(who).is_none() {
        return false;
    }
    let added = match sim.components.get_mut::<KnowHow>(who) {
        Some(known) => known.insert(id),
        None => {
            let mut known = KnowHow::default();
            known.insert(id);
            sim.components.insert(who, known);
            true
        }
    };
    if added {
        raise(sim, KNOW_HOW_GAINED, who, id, how, cause);
    }
    added
}

/// Teaches an entity a piece of know-how (16 §8.5 `teach`): refused with
/// `know_how.no_mind` for one with no mind and `know_how.needs` while
/// something it `needs` is not known. Returns whether it is new.
pub fn teach(sim: &mut Sim, who: EntityId, id: KnowHowId, how: &str, cause: Cause) -> Result<bool, Reason> {
    sim.known(who)?;
    let name = know_how_name(sim, id);
    let def = registry(sim)
        .and_then(|registry| registry.try_get(id))
        .ok_or_else(|| unknown(&name))?;
    if sim.mind(who).is_none() {
        return Err(Reason::new(
            codes::NO_MIND,
            format!("{who} has no mind to know `{name}` with"),
        )
        .with("entity", who.to_string()));
    }
    let missing: Vec<KnowHowId> = def
        .needs
        .iter()
        .map(|needed| needed.id())
        .filter(|needed| !knows(sim, who, *needed))
        .collect();
    if let Some(first) = missing.first() {
        let needed = know_how_label(sim, *first);
        return Err(Reason::new(
            codes::NEEDS_FIRST,
            format!("{needed} must be known first"),
        )
        .with("know_how", name.as_str())
        .with("needs", know_how_name(sim, *first)));
    }
    Ok(grant(sim, who, id, how, cause))
}

/// Takes a piece of know-how away (16 §8.5 `unlearn`), raising
/// `know_how_lost`. Returns whether it was known.
pub fn unlearn(sim: &mut Sim, who: EntityId, id: KnowHowId, cause: Cause) -> Result<bool, Reason> {
    sim.known(who)?;
    let removed = sim
        .components
        .get_mut::<KnowHow>(who)
        .is_some_and(|known| known.remove(id));
    if sim
        .components
        .get::<KnowHow>(who)
        .is_some_and(|known| known.known.is_empty())
    {
        sim.components.remove::<KnowHow>(who);
    }
    if removed {
        raise(sim, KNOW_HOW_LOST, who, id, "unlearned", cause);
    }
    Ok(removed)
}

/// Teaches what an item `teaches` (16 §8.5: reading), each as `teach`
/// does, in order: what cannot be taught yet is passed by. Returns how
/// many were new. Refused with `know_how.teaches_nothing` for an item
/// that teaches nothing, and with the first refusal when none could be
/// taught.
pub fn learn_from(sim: &mut Sim, who: EntityId, item: EntityId, cause: Cause) -> Result<i64, Reason> {
    let taught: Vec<KnowHowId> = sim
        .components
        .get::<Item>(item)
        .and_then(|it| crate::item::def_of(&sim.defs, it))
        .map_or_else(Vec::new, |def| def.teaches.iter().map(|id| id.id()).collect());
    if taught.is_empty() {
        return Err(Reason::new(
            codes::TEACHES_NOTHING,
            "there is nothing to be learnt from that",
        )
        .with("item", item.to_string()));
    }
    let mut new = 0;
    let mut first_refusal = None;
    let mut any = false;
    for id in taught {
        match teach(sim, who, id, "read", cause) {
            Ok(added) => {
                any = true;
                if added {
                    new += 1;
                }
            }
            Err(reason) => {
                first_refusal.get_or_insert(reason);
            }
        }
    }
    match (any, first_refusal) {
        (false, Some(reason)) => Err(reason),
        _ => Ok(new),
    }
}

/// Moves the know-how of one entity's mind to another's: what
/// `transfer_mind` does, with `swap` exchanging the two.
pub(crate) fn follow_mind(sim: &mut Sim, from: EntityId, to: EntityId) {
    let leaving = sim.components.remove::<KnowHow>(from);
    let returning = sim.components.remove::<KnowHow>(to);
    if let Some(known) = leaving {
        sim.components.insert(to, known);
    }
    if let Some(known) = returning {
        sim.components.insert(from, known);
    }
}

/// Gives a new entity the know-how its templates list (06 §10), once
/// `entity_spawned` says it was made from them.
pub(crate) fn from_templates(sim: &mut Sim, who: EntityId, cause: Cause) {
    let Some(identity) = sim.components.get::<crate::component::Identity>(who) else {
        return;
    };
    let templates: Vec<crate::template::TemplateId> = identity.templates().to_vec();
    let Some(registry) = sim.defs.try_registry::<crate::template::TemplateDef>() else {
        return;
    };
    let mut names: Vec<(String, String)> = Vec::new();
    for template in templates {
        let Some(def) = registry.try_get(template) else {
            continue;
        };
        let owner = registry.name(template).to_owned();
        names.extend(def.know_how.iter().map(|name| (owner.clone(), name.clone())));
    }
    for (owner, written) in names {
        let id = if written.contains(':') {
            lookup(sim, &written)
        } else {
            owner
                .split_once(':')
                .and_then(|(namespace, _)| lookup(sim, &format!("{namespace}:{written}")))
                .or_else(|| lookup(sim, &written))
        };
        if let Some(id) = id {
            grant(sim, who, id, "template", cause);
        }
    }
}
