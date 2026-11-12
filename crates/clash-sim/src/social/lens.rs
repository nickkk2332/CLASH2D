//! What a reader calls someone (05 §4.6; 15 §8): the reader lens every
//! simulation is bound with.
//!
//! | Who | Is called |
//! |-----|-----------|
//! | The reader | "you" (the text helpers say so before the lens is asked) |
//! | Someone the reader takes for a persona whose name it knows | That name: `Mara` |
//! | Someone else | A description of what the reader takes them to be: the name of the template the persona looks like, `the person`; `someone` if there is none |
//! | A thing that is no one (no mind, presenting nothing) | Its own name, if it has one, as before personas |
//!
//! A name is known by having been learnt (`learn_name`); a reader knows
//! its own. What the reader takes someone for is [`persona_of`]: a
//! disguise is named as what it shows.

use clash_core::EntityId;
use clash_data::Def;

use super::persona::{knows_name, persona_name, persona_of, persona_record, presenting};
use crate::Sim;
use crate::component::Identity;
use crate::host::Bound;
use crate::perception::PersonaId;
use crate::reader::{Known, plain_lens};
use crate::template::TemplateDef;

/// What a reader calls someone it does not know and has no description
/// of.
pub const SOMEONE: &str = "someone";

/// Whether an entity is someone, whom personas apply to: it has a mind,
/// or presents another's persona.
fn is_someone(sim: &Sim, entity: EntityId) -> bool {
    sim.components.mind.contains(entity) || presenting(sim, entity) != PersonaId::true_of(entity)
}

/// The reader lens of the social layer: a known name, else a
/// description (see the [module documentation](self)).
pub fn social_lens(sim: &Sim, reader: EntityId, of: EntityId) -> Option<Known> {
    if !is_someone(sim, of) {
        return plain_lens(sim, reader, of);
    }
    let persona = persona_of(sim, reader, of);
    if knows_name(sim, reader, persona)
        && let Some(name) = persona_name(sim, persona)
    {
        return Some(Known::Proper(name));
    }
    Some(description(sim, persona))
}

/// How a persona is described to one who does not know its name: the
/// name of the template it looks like, its record's or its owner's
/// first.
pub fn description(sim: &Sim, persona: PersonaId) -> Known {
    let templates = sim.defs.try_registry::<TemplateDef>();
    let apparent = persona_record(sim, persona)
        .and_then(|record| record.apparent.as_deref())
        .and_then(|name| crate::template::template_by_name(&sim.defs, name));
    let first = || {
        sim.components
            .identity
            .get(persona.entity())
            .and_then(|identity: &Identity| identity.templates().first().copied())
    };
    let Some(template) = apparent.or_else(first) else {
        return Known::Proper(SOMEONE.to_owned());
    };
    let Some(def) = templates.and_then(|registry| registry.try_get(template)) else {
        return Known::Proper(SOMEONE.to_owned());
    };
    let name = match &def.name {
        Some(text) => {
            let mut host = sim.host(Bound::new());
            text.render_on(&sim.defs, &mut host)
        }
        None => templates
            .map(|registry| registry.name(template))
            .and_then(|qualified| qualified.split_once(':').map(|(_, bare)| bare.to_owned()))
            .unwrap_or_else(|| TemplateDef::TYPE.to_owned()),
    };
    Known::Common(name.to_lowercase())
}
