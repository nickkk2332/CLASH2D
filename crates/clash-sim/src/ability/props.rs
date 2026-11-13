//! What expressions read of abilities (08 §12).
//!
//! On an entity: `has_ability(ability)`, `ability(ability)`, `abilities`,
//! `abilities_with(tag)` and `cannot_use(ability, action?)`. On the kind
//! `ability`: `rank`, `owner`, `state`, `def`, `id`, `tags`,
//! `mode_on(mode)`, `cooldown(action)` and each parameter by name.
//!
//! None is safe for belief: what abilities an entity has is its own to
//! know and anyone else's to perceive (15). A character reads its own
//! (05 §7.3), and `ability(…)` and `abilities_with(…)` give what is its
//! own, so that reading on from them in its mind is still reading its
//! own state. None is settable: abilities change through the verbs.

use clash_core::{EntityId, Real};
use clash_data::Def;
use clash_expr::{DefRef, Handle, PropertyId, Value};

use super::{
    ABILITY_GRANTED, ABILITY_REVOKED, Abilities, AbilityDef, AbilityId, Prop, def_of, handle_value,
    owner_of,
};
use crate::Sim;
use crate::action::{ActionDef, ActionDefId};
use crate::event::Subject;
use crate::registry::PropertyDef;

/// The events that change which abilities an entity holds.
const HELD_CHANGES: &[&str] = &[ABILITY_GRANTED, ABILITY_REVOKED];

fn abilities(sim: &Sim, subject: Subject) -> Option<&Abilities> {
    let id: EntityId = subject.entity().filter(|id| sim.entities.contains(*id))?;
    sim.components.get::<Abilities>(id)
}

/// The ability an argument names: an id of the type, or its qualified
/// id as text.
fn ability_arg(sim: &Sim, args: &[Value]) -> Option<AbilityId> {
    match args.first()? {
        Value::Id(id) if Some(id.ty) == sim.exprs.registry().def_type_id(AbilityDef::TYPE) => {
            AbilityId::from_index(id.index as usize)
        }
        Value::Str(text) => super::lookup(&sim.defs, text),
        _ => None,
    }
}

/// The action an argument names.
fn action_arg(sim: &Sim, arg: Option<&Value>) -> Option<ActionDefId> {
    match arg? {
        Value::Id(id) if Some(id.ty) == sim.exprs.registry().def_type_id(ActionDef::TYPE) => {
            ActionDefId::from_index(id.index as usize)
        }
        Value::Str(text) => sim.defs.lookup::<ActionDef>(text),
        _ => None,
    }
}

const ARG_DOC: &str = "The ability: its id, or its qualified id as text.";

/// The properties of abilities on entities, for the engine's table.
pub fn properties() -> Vec<PropertyDef> {
    let p = PropertyDef::new;
    vec![
        p(
            "entity",
            "has_ability",
            "bool",
            "Whether the entity holds an ability (08 §3.3), from any source, dormant or not.",
            |sim, subject, args| {
                let held = abilities(sim, subject)
                    .zip(ability_arg(sim, args))
                    .is_some_and(|(abilities, def)| abilities.holds(def));
                Value::Bool(held)
            },
        )
        .arg("ability", "id<ability>", ARG_DOC)
        .changed_by(HELD_CHANGES),
        p(
            "entity",
            "ability",
            "ability?",
            "The instance of an ability the entity holds (08 §3.2), to read its rank, state, parameters and cooldowns; null if it does not hold it.",
            |sim, subject, args| {
                let found = abilities(sim, subject)
                    .zip(ability_arg(sim, args))
                    .and_then(|(abilities, def)| abilities.of(def))
                    .filter(|inst| inst.is_held())
                    .map(super::AbilityInst::number);
                found.map_or(Value::Null, |number| handle_value(sim, number))
            },
        )
        .arg("ability", "id<ability>", ARG_DOC)
        .own()
        .changed_by(HELD_CHANGES),
        p(
            "entity",
            "abilities",
            "list<id<ability>>",
            "The abilities the entity holds, in the order it gained them.",
            |sim, subject, _| {
                let ty = sim.exprs.registry().def_type_id(AbilityDef::TYPE);
                let (Some(abilities), Some(ty)) = (abilities(sim, subject), ty) else {
                    return Value::list(core::iter::empty());
                };
                Value::list(
                    abilities
                        .instances()
                        .iter()
                        .filter(|inst| inst.is_held())
                        .map(|inst| Value::Id(DefRef::new(ty, inst.def().raw()))),
                )
            },
        )
        .changed_by(HELD_CHANGES),
        p(
            "entity",
            "abilities_with",
            "list<ability>",
            "The instances of the abilities the entity holds that have a tag, in the order it gained them.",
            |sim, subject, args| {
                let Some(Value::Str(tag)) = args.first() else {
                    return Value::list(core::iter::empty());
                };
                let Some(abilities) = abilities(sim, subject) else {
                    return Value::list(core::iter::empty());
                };
                Value::list(
                    abilities
                        .instances()
                        .iter()
                        .filter(|inst| inst.is_held())
                        .filter(|inst| {
                            def_of(&sim.defs, inst.def())
                                .is_some_and(|def| def.tags.iter().any(|have| **have == **tag))
                        })
                        .map(|inst| handle_value(sim, inst.number())),
                )
            },
        )
        .arg("tag", "string", "The tag.")
        .own()
        .changed_by(HELD_CHANGES),
        p(
            "entity",
            "cannot_use",
            "string?",
            "Why the entity cannot use an ability now (08 §5.1), as the reason's code (`ability.cooling_down`, `ability.cost_short`, …); null if it can. With an action, that grant; without, the ability's only one. Moves with time and pools, so it cannot be kept current.",
            |sim, subject, args| {
                let (Some(entity), Some(def)) = (subject.entity(), ability_arg(sim, args)) else {
                    return Value::Null;
                };
                match super::available(sim, entity, def, action_arg(sim, args.get(1))) {
                    Ok(()) => Value::Null,
                    Err(reason) => Value::from(reason.code.as_str()),
                }
            },
        )
        .arg("ability", "id<ability>", ARG_DOC)
        .optional_arg("action", "id<action>", "The action of the grant; left out, the ability's only one."),
        p(
            "entity",
            "bound_by",
            "entity?",
            "Whose ability the entity is bound to (08 §7.3), as a summoned servant is to its summoner: the owner of the ability whose `summon` made it, while the binding lasts; null otherwise. A character may read its own: whom it serves.",
            |sim, subject, _| {
                subject
                    .entity()
                    .and_then(|entity| super::bind::bound_by(sim, entity))
                    .map_or(Value::Null, |owner| sim.subject_value(Subject::Entity(owner)))
            },
        ),
    ]
}

/// A property with one more argument that may be left out.
trait OptionalArg {
    fn optional_arg(self, name: &'static str, ty: &'static str, doc: &'static str) -> Self;
}

impl OptionalArg for PropertyDef {
    fn optional_arg(
        mut self,
        name: &'static str,
        ty: &'static str,
        doc: &'static str,
    ) -> PropertyDef {
        self.args.push(crate::registry::ArgDef {
            name,
            ty,
            doc,
            optional: true,
        });
        self
    }
}

/// Answers a property of the kind `ability`, if `property` is one and
/// the handle is of that kind: what the kernel asks before its own
/// table. `None` for anything else.
pub fn property(sim: &Sim, property: PropertyId, subject: Handle, args: &[Value]) -> Option<Value> {
    let tables = &sim.ability_rt.tables;
    let prop = tables.props.get(&property)?;
    if Some(subject.kind) != tables.kind {
        return None;
    }
    let number = subject.payload;
    let Some(owner) = owner_of(sim, number) else {
        return Some(Value::Null);
    };
    let Some(inst) = sim
        .components
        .get::<Abilities>(owner)
        .and_then(|abilities| abilities.by_number(number))
    else {
        return Some(Value::Null);
    };
    if !inst.is_held() {
        return Some(Value::Null);
    }
    let def = def_of(&sim.defs, inst.def());
    Some(match prop {
        Prop::Rank => Value::Int(i64::from(super::rank(sim, owner, inst))),
        Prop::Owner => sim.subject_value(Subject::Entity(owner)),
        Prop::State => Value::from(inst.state_name()),
        Prop::Def => sim
            .exprs
            .registry()
            .def_type_id(AbilityDef::TYPE)
            .map_or(Value::Null, |ty| {
                Value::Id(DefRef::new(ty, inst.def().raw()))
            }),
        Prop::Id => Value::from(super::name_of(&sim.defs, inst.def()).as_str()),
        Prop::Tags => Value::list(
            def.map(|def| {
                def.tags
                    .iter()
                    .map(|tag| Value::from(tag.as_str()))
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default(),
        ),
        Prop::ModeOn => {
            let Some(Value::Str(mode)) = args.first() else {
                return Some(Value::Bool(false));
            };
            let on = def
                .and_then(|def| def.mode_index(mode))
                .is_some_and(|mode| inst.mode_is_on(mode));
            Value::Bool(on)
        }
        Prop::Cooldown => {
            let Some(action) = action_arg(sim, args.first()) else {
                return Some(Value::Real(Real::ZERO));
            };
            let now = sim.own_time(owner);
            let left = inst
                .cooldowns()
                .iter()
                .find(|cooldown| cooldown.action == action)
                .map_or(Real::ZERO, |cooldown| {
                    (cooldown.ready_at - now)
                        .max(clash_core::Dur::ZERO)
                        .as_secs_real()
                });
            Value::Real(left)
        }
        Prop::Bound => match args.first() {
            Some(Value::Str(name)) => super::bind::bound_value(sim, number, name),
            _ => Value::Null,
        },
        Prop::Param(name) => super::ops::param_value(sim, owner, number, name),
    })
}
