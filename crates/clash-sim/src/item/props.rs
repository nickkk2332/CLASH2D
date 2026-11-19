//! What expressions read of items and holds (06 §9, §13).
//!
//! All of it is truth, and none of it is safe for belief: what a thing
//! is, what it holds and what is inside it are its holder's to know and
//! anyone else's to perceive (15). A character reads its own
//! (05 §7.3). The lists (`held`, `worn_on`, `carried`) yield what
//! belongs to the same owner, so a character may go on to read what it
//! holds, what its own pack holds, and so on; what is inside someone
//! else's pack is not reachable from a mind.

use clash_core::{EntityId, Real};
use clash_data::Def;
use clash_expr::{DefRef, Value};
use clash_world::Cause;

use super::hold::{Holds, free_grasp, hold_of, holder_of};
use super::make::set_color;
use super::wear::{DAMAGE_PARTS, worn_layers};
use super::{Item, ItemDef, def_of};
use crate::Sim;
use crate::action::{ChannelDef, ChannelId};
use crate::body::Body;
use crate::event::Subject;
use crate::reason::{Reason, codes};
use crate::registry::{EngineDefTypes, PropertyDef};

/// The events that change how things are held.
const HOLD_EVENTS: &[&str] = &[
    "item_taken",
    "item_dropped",
    "item_worn",
    "item_removed",
    "entity_relocated",
];

fn entity_id(sim: &Sim, subject: Subject) -> Option<EntityId> {
    subject.entity().filter(|id| sim.entities.contains(*id))
}

fn item(sim: &Sim, subject: Subject) -> Option<&Item> {
    sim.components.get::<Item>(entity_id(sim, subject)?)
}

fn item_def(sim: &Sim, subject: Subject) -> Option<&ItemDef> {
    def_of(&sim.defs, item(sim, subject)?)
}

fn opt<T: Into<Value>>(value: Option<T>) -> Value {
    value.map_or(Value::Null, Into::into)
}

fn entities(sim: &Sim, ids: impl IntoIterator<Item = EntityId>) -> Value {
    Value::list(
        ids.into_iter()
            .map(|id| sim.subject_value(Subject::Entity(id))),
    )
}

/// What an entity holds in one kind of hold, in order of id.
fn held(sim: &Sim, holder: EntityId, kind: &str) -> Vec<EntityId> {
    sim.entities
        .contents(holder)
        .iter()
        .copied()
        .filter(|inside| hold_of(sim, *inside).is_some_and(|hold| hold.name() == kind))
        .collect()
}

/// The space what is directly inside an entity takes up.
fn stowed_volume(sim: &Sim, container: EntityId) -> Real {
    let mut total = Real::ZERO;
    for inside in sim.entities.contents(container) {
        if let Some(physical) = sim.components.get::<crate::component::Physical>(*inside) {
            total += physical.volume().get();
        }
    }
    total
}

/// The channel an argument names: an id, or its qualified id as text.
fn channel_arg(sim: &Sim, args: &[Value]) -> Option<ChannelId> {
    match args.first()? {
        Value::Id(id) => ChannelId::from_index(id.index as usize),
        Value::Str(text) => sim.defs.try_registry::<ChannelDef>()?.lookup(text),
        _ => None,
    }
}

/// `entity.color = 'red'`; null takes the colour away.
fn set_entity_color(
    sim: &mut Sim,
    subject: Subject,
    value: &Value,
    _cause: Cause,
) -> Result<(), Reason> {
    let id = entity_id(sim, subject).ok_or_else(|| {
        Reason::new(codes::SUBJECT_GONE, "the entity no longer exists").with("kind", "entity")
    })?;
    match value {
        Value::Null => set_color(sim, id, None),
        Value::Str(name) => set_color(sim, id, Some(&**name)),
        other => Err(Reason::new(
            codes::WRITE_BAD_VALUE,
            format!(
                "this takes a colour's name, or null for none, not a {}",
                other.type_name()
            ),
        )
        .with("found", other.type_name())),
    }
}

/// The properties this module registers, all of the kind `entity`.
pub fn properties() -> Vec<PropertyDef> {
    let mut all = listed();
    for property in &mut all {
        if property.name == "hold_fault" {
            property.args.push(crate::registry::ArgDef {
                name: "other",
                ty: "any",
                doc: "For `put_in` the container, for `give` whom it is given to.",
                optional: true,
            });
        }
    }
    all
}

fn listed() -> Vec<PropertyDef> {
    let p = PropertyDef::new;
    vec![
        p(
            "entity",
            "item",
            "id<item>?",
            "The `item` definition the entity was made from. Null for what is not an item, and for an item made from none: a part that came off a body.",
            |sim, subject, _| {
                let ty = sim.exprs.registry().def_type_id(ItemDef::TYPE);
                opt(item(sim, subject)
                    .and_then(Item::def)
                    .zip(ty)
                    .map(|(def, ty)| Value::Id(DefRef::new(ty, def.raw()))))
            },
        ),
        p(
            "entity",
            "is_item",
            "bool",
            "Whether the entity is an item.",
            |sim, subject, _| Value::Bool(item(sim, subject).is_some()),
        ),
        p(
            "entity",
            "count",
            "int?",
            "How many equal members share the entity: more than one for a stack. Null for what is not an item.",
            |sim, subject, _| opt(item(sim, subject).map(|item| i64::from(item.count()))),
        )
        .changed_by(&["item_split", "item_merged"]),
        p(
            "entity",
            "condition",
            "real?",
            "How whole an item is, from 1 for undamaged to 0 for broken: one less its damage in 255ths. Null for what is not an item.",
            |sim, subject, _| {
                opt(item(sim, subject).map(|item| {
                    Real::ONE
                        - Real::from_i64(i64::from(item.damage()))
                            / Real::from_i64(i64::from(DAMAGE_PARTS))
                }))
            },
        )
        .changed_by(&["item_damaged", "item_repaired"]),
        p(
            "entity",
            "is_broken",
            "bool",
            "Whether the entity is an item whose damage is all 255 parts.",
            |sim, subject, _| Value::Bool(item(sim, subject).is_some_and(Item::is_broken)),
        )
        .changed_by(&["item_damaged", "item_repaired"]),
        p(
            "entity",
            "material",
            "id<material>?",
            "What an item is made of. Null for what is not an item.",
            |sim, subject, _| {
                opt(item(sim, subject).map(|item| {
                    EngineDefTypes::id(sim.exprs.def_types().material, item.material().index())
                }))
            },
        ),
        p(
            "entity",
            "color",
            "string?",
            "The colour an item carries, by name: one its definition fixed or one picked when it was made. Null for none. Setting it changes it; setting null takes it away.",
            |sim, subject, _| opt(item(sim, subject).and_then(Item::color)),
        )
        .settable(set_entity_color),
        p(
            "entity",
            "holder",
            "entity?",
            "What holds the entity: what it is directly inside. Null for one that is inside nothing.",
            |sim, subject, _| {
                opt(entity_id(sim, subject)
                    .and_then(|id| holder_of(sim, id))
                    .map(|holder| sim.subject_value(Subject::Entity(holder))))
            },
        )
        .changed_by(HOLD_EVENTS),
        p(
            "entity",
            "hold",
            "string?",
            "How the entity is held by what it is inside: `grasp`, `worn`, `stowed`, `seat`, `inside` or `fixed`. Null for one that is inside nothing.",
            |sim, subject, _| {
                opt(entity_id(sim, subject)
                    .and_then(|id| hold_of(sim, id))
                    .map(|hold| hold.name()))
            },
        )
        .changed_by(HOLD_EVENTS),
        p(
            "entity",
            "held",
            "list<entity>",
            "What the entity holds in one kind of hold, in order of id: `self.held('grasp')`, `self.held('worn')`, a container's `held('stowed')`.",
            |sim, subject, args| match (entity_id(sim, subject), args.first().and_then(Value::as_str)) {
                (Some(id), Some(kind)) => entities(sim, held(sim, id, kind)),
                _ => Value::list([]),
            },
        )
        .arg(
            "hold",
            "string",
            "The kind of hold: `grasp`, `worn`, `stowed`, `seat`, `inside` or `fixed`.",
        )
        .own()
        .changed_by(HOLD_EVENTS),
        p(
            "entity",
            "carried",
            "list<entity>",
            "Everything inside the entity at any depth: what it holds, what those hold, and so on, each container before its contents.",
            |sim, subject, _| match entity_id(sim, subject) {
                Some(id) => entities(sim, sim.entities.contents_deep(id)),
                None => Value::list([]),
            },
        )
        .own()
        .changed_by(HOLD_EVENTS),
        p(
            "entity",
            "worn_on",
            "list<entity>",
            "What the entity wears over one part of its body, outermost first. A broken item is not listed: it is no longer a layer.",
            |sim, subject, args| {
                let found = entity_id(sim, subject).and_then(|id| {
                    let name = args.first()?.as_str()?;
                    let part = sim.components.get::<Body>(id)?.part_named(name)?.key();
                    Some(worn_layers(sim, id, part))
                });
                match found {
                    Some(layers) => {
                        let mut items: Vec<EntityId> = Vec::new();
                        for layer in layers {
                            if !items.contains(&layer.item) {
                                items.push(layer.item);
                            }
                        }
                        entities(sim, items)
                    }
                    None => Value::list([]),
                }
            },
        )
        .arg("part", "string", "The part's name in the body.")
        .own()
        .changed_by(HOLD_EVENTS),
        p(
            "entity",
            "free_grasp",
            "int",
            "How many slots of a channel the working parts of the entity's body supply that nothing it grasps occupies. Zero for something with no body.",
            |sim, subject, args| {
                let free = entity_id(sim, subject)
                    .zip(channel_arg(sim, args))
                    .map_or(0, |(id, channel)| free_grasp(sim, id, channel));
                Value::Int(i64::from(free))
            },
        )
        .arg("channel", "id<channel>", "The channel.")
        .changed_by(HOLD_EVENTS),
        p(
            "entity",
            "capacity_volume",
            "real?",
            "The most space what is stowed in a container may take up, in cubic metres. Null for what is not a container and for one with no limit.",
            |sim, subject, _| {
                opt(item_def(sim, subject)
                    .and_then(|def| def.holds.as_ref()?.volume)
                    .map(|volume| volume.get()))
            },
        ),
        p(
            "entity",
            "capacity_mass",
            "real?",
            "The most what is stowed in a container may weigh, in kilograms. Null for what is not a container and for one with no limit.",
            |sim, subject, _| {
                opt(item_def(sim, subject)
                    .and_then(|def| def.holds.as_ref()?.mass)
                    .map(|mass| mass.get()))
            },
        ),
        p(
            "entity",
            "stowed_volume",
            "real?",
            "The space what is directly inside the entity takes up, in cubic metres. Null for an entity that is gone.",
            |sim, subject, _| opt(entity_id(sim, subject).map(|id| stowed_volume(sim, id))),
        )
        .changed_by(HOLD_EVENTS),
        p(
            "entity",
            "is_latent",
            "bool",
            "Whether the entity is a container whose contents have not been made yet (06 §9.3).",
            |sim, subject, _| Value::Bool(item(sim, subject).is_some_and(Item::is_latent)),
        )
        .changed_by(&["contents_materialised"]),
        p(
            "entity",
            "hold_fault",
            "string?",
            "Why the entity, as it believes, could not do one of the holding verbs to a thing now, as a sentence for it to read: null if it could (06 §9.2, decision D-865). For an item action's `requires` and `fail_text`; read of the character itself in a mind context. `take`: pick up a thing lying loose within reach, or take one it carries into a hand (what no hand holds goes into something carried that takes it); `grasp`: hold it in a hand; `drop`; `wear`; `remove`, into a hand; `stow`, into something it wears or holds; `put_in`, into `other`, which it carries or perceives within reach; `take_out`, out of a container it carries or perceives within reach, into a hand; `give`, to `other`, someone it perceives within reach with a hand free for it. Doing it to the entity itself is refused; `take` of the entity itself says there is nothing within reach to pick up.",
            |sim, subject, args| {
                let who = entity_id(sim, subject);
                let verb = args.first().and_then(Value::as_str).unwrap_or("");
                let thing = |value: Option<&Value>| match value.and_then(|value| sim.subject_of(value)) {
                    Some(Subject::Entity(id)) => Some(id),
                    _ => None,
                };
                match who {
                    Some(who) => opt(super::fault::hold_fault(
                        sim,
                        who,
                        verb,
                        thing(args.get(1)),
                        thing(args.get(2)),
                    )),
                    None => Value::from("There is nobody to do it."),
                }
            },
        )
        .arg(
            "verb",
            "string",
            "`take`, `grasp`, `drop`, `wear`, `remove`, `stow`, `put_in`, `take_out` or `give`.",
        )
        .arg("what", "any", "The thing.")
        .changed_by(HOLD_EVENTS),
        p(
            "entity",
            "loose_near",
            "list<map<any>>",
            "The things the entity perceives lying loose within reach (in its cell, or one of the eight round it on its storey), its own cell first, then the nearest: what it could pick up. Each a map of `value` (the thing), `label` (what the entity calls it, with its article) and `where` (`here`, `north`, `north-east`, ...), as a list picker takes them (decision D-865). Read of the character itself in a mind context.",
            |sim, subject, _| match entity_id(sim, subject) {
                Some(id) => super::fault::loose_near_value(sim, id),
                None => Value::list([]),
            },
        )
        .changed_by(HOLD_EVENTS),
        p(
            "entity",
            "conferred_tags",
            "list<string>",
            "The tags the entity carries only because something it holds in a grasp or wears confers them, in order.",
            |sim, subject, _| {
                let tags = entity_id(sim, subject)
                    .and_then(|id| sim.components.get::<Holds>(id))
                    .map(|holds| holds.conferred_tags())
                    .unwrap_or_default();
                Value::list(tags.iter().map(|tag| Value::from(tag.as_str())))
            },
        )
        .changed_by(HOLD_EVENTS),
    ]
}

/// Whether a hold is one of the names `held` takes: for the tests of
/// this module.
#[cfg(test)]
fn is_hold_name(name: &str) -> bool {
    use super::hold::Hold;
    [
        Hold::Grasp(Vec::new()),
        Hold::Worn(Vec::new()),
        Hold::Stowed,
        Hold::Seat(0),
        Hold::Inside,
        Hold::Fixed,
    ]
    .iter()
    .any(|hold| hold.name() == name)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_property_is_documented_and_none_is_safe_for_belief() {
        let table = properties();
        let mut names: Vec<&str> = table.iter().map(|property| property.name).collect();
        let count = names.len();
        names.sort_unstable();
        names.dedup();
        assert_eq!(names.len(), count, "a property is listed twice");
        for property in &table {
            assert_eq!(property.kind, "entity");
            assert!(!property.doc.is_empty(), "{}", property.name);
            assert!(!property.belief_safe, "{}", property.name);
            for arg in &property.args {
                assert!(!arg.doc.is_empty(), "{}.{}", property.name, arg.name);
            }
        }
        // What an entity holds belongs to the same owner; what holds it
        // does not.
        let own = |name: &str| table.iter().find(|p| p.name == name).unwrap().yields_own;
        assert!(own("held") && own("carried") && own("worn_on"));
        assert!(!own("holder"));
        assert!(is_hold_name("grasp") && is_hold_name("stowed") && !is_hold_name("pocket"));
    }

    #[test]
    fn every_property_enters_the_engine_registry() {
        let registry = crate::engine_registry();
        let entity = registry.kind_id("entity").unwrap();
        for property in properties() {
            assert!(
                registry.property_id(entity, property.name).is_some(),
                "`entity.{}` is not registered",
                property.name
            );
        }
    }
}
