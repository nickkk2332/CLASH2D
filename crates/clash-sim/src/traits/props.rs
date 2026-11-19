//! What expressions read of traits (06 §5, §13).
//!
//! None is safe for belief: what traits an entity has is its own to know
//! and anyone else's to perceive (15). A character reads its own
//! (05 §7.3). None is settable: traits change through `add_trait` and
//! `remove_trait`, so that hooks run and what follows is reported.

use clash_core::{Dur, EntityId, Real};
use clash_data::Def;
use clash_expr::{DefRef, Value};

use super::{Timer, TraitDef, TraitId, Traits};
use crate::Sim;
use crate::event::Subject;
use crate::registry::PropertyDef;

/// The events that change what traits an entity has.
const TRAIT_CHANGES: &[&str] = &[super::TRAIT_ADDED, super::TRAIT_REMOVED];

fn traits(sim: &Sim, subject: Subject) -> Option<&Traits> {
    let id: EntityId = subject.entity().filter(|id| sim.entities.contains(*id))?;
    sim.components.get::<Traits>(id)
}

/// The trait an argument names: an id of the type, or its qualified id
/// as text.
fn trait_arg(sim: &Sim, args: &[Value]) -> Option<TraitId> {
    match args.first()? {
        Value::Id(id) if Some(id.ty) == sim.exprs.registry().def_type_id(TraitDef::TYPE) => {
            TraitId::from_index(id.index as usize)
        }
        Value::Str(text) => super::lookup(&sim.defs, text),
        _ => None,
    }
}

/// The local time left before a trait ends by itself, the longest over
/// its givers, in seconds; `None` if it does not have it or some giver's
/// lasts until removed.
fn time_left(sim: &Sim, traits: &Traits, def: TraitId) -> Option<Real> {
    let mut longest: Option<Dur> = None;
    for inst in traits.of(def) {
        let left = match inst.timer() {
            Timer::Lasts => return None,
            Timer::Held(left) => left,
            Timer::Running(wake) => sim.timeline.remaining(wake).unwrap_or(Dur::ZERO),
        };
        longest = Some(longest.map_or(left, |have| have.max(left)));
    }
    longest.map(Dur::as_secs_real)
}

const ARG_DOC: &str = "The trait: its id, or its qualified id as text.";

/// The properties of traits, all of the kind `entity`.
pub fn properties() -> Vec<PropertyDef> {
    let p = PropertyDef::new;
    vec![
        p(
            "entity",
            "has_trait",
            "bool",
            "Whether the entity has a trait, from any giver, dormant or not (06 §5).",
            |sim, subject, args| {
                let found = traits(sim, subject)
                    .zip(trait_arg(sim, args))
                    .is_some_and(|(traits, def)| traits.has(def));
                Value::Bool(found)
            },
        )
        .arg("trait", "id<trait>", ARG_DOC)
        .changed_by(TRAIT_CHANGES),
        p(
            "entity",
            "traits",
            "list<id<trait>>",
            "The traits the entity has, each once, in the order it first gained them.",
            |sim, subject, _| {
                let ty = sim.exprs.registry().def_type_id(TraitDef::TYPE);
                let (Some(traits), Some(ty)) = (traits(sim, subject), ty) else {
                    return Value::list(core::iter::empty());
                };
                Value::list(
                    traits
                        .defs()
                        .into_iter()
                        .map(|def| Value::Id(DefRef::new(ty, def.raw()))),
                )
            },
        )
        .changed_by(TRAIT_CHANGES),
        p(
            "entity",
            "trait_stacks",
            "int",
            "How many stacks of a trait the entity has, over every giver: 0 for one it does not have, 1 for each giver of one that does not stack.",
            |sim, subject, args| {
                let stacks = traits(sim, subject)
                    .zip(trait_arg(sim, args))
                    .map_or(0, |(traits, def)| traits.stacks(def));
                Value::Int(i64::from(stacks))
            },
        )
        .arg("trait", "id<trait>", ARG_DOC)
        .changed_by(TRAIT_CHANGES),
        p(
            "entity",
            "trait_active",
            "bool",
            "Whether the entity has a trait that is not dormant from some giver: one whose `while` holds, so that it confers what it lists.",
            |sim, subject, args| {
                let active = traits(sim, subject)
                    .zip(trait_arg(sim, args))
                    .is_some_and(|(traits, def)| traits.of(def).any(|inst| !inst.is_dormant()));
                Value::Bool(active)
            },
        )
        .arg("trait", "id<trait>", ARG_DOC),
        p(
            "entity",
            "trait_left",
            "real?",
            "How long until a trait ends by itself, in seconds of the entity's own time: the longest over its givers. Null if the entity does not have it, or has it from a giver for as long as that giver lasts.",
            |sim, subject, args| {
                let left = traits(sim, subject)
                    .zip(trait_arg(sim, args))
                    .and_then(|(traits, def)| time_left(sim, traits, def));
                left.map_or(Value::Null, Value::Real)
            },
        )
        .arg("trait", "id<trait>", ARG_DOC),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_property_is_documented_and_registered() {
        let registry = crate::engine_registry();
        let entity = registry.kind_id("entity").unwrap();
        for property in properties() {
            assert!(!property.doc.is_empty());
            assert!(!property.belief_safe, "{} reads truth", property.name);
            assert!(
                registry.property_id(entity, property.name).is_some(),
                "`{}` is not in the engine's registry",
                property.name
            );
        }
    }
}
