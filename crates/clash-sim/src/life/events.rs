//! The events and hooks of living things, as one table: what the bus is
//! told so that they can be raised, and what the expression registry is
//! told so that rules can listen for them (05 §8.1; 06 §13).
//!
//! The stats' and the bodies' events are registered with the expression
//! language by their own registry steps, with their exact types
//! (`stats::props`, `body::props`). This table gives the bus every one
//! of them, and gives the expression registry those that are raised by
//! the wiring itself: death and waking, minds, what a hit left behind,
//! spawning.
//!
//! # How a field is carried
//!
//! | Written | Carried as |
//! |---------|-----------|
//! | `real`, `int`, `bool`, `string` | The plain value |
//! | `id<…>` | A stored id: the number of the definition type and the definition's number within it |
//! | `part`, `wound` | A stored handle of that kind (`body::props`) |
//! | `list<…>` | A stored list |
//!
//! All of them reach a rule as the expression language's own values
//! (`Bindings::field_value`).

use clash_core::{EntityId, Real, Value};
use clash_data::RegistryCx;
use clash_expr::{DefRef, EventSpec, Handle, Registry, Type};

use crate::bus::{EventPayload, EventType, FieldType, FieldValue};
use crate::event::Subject;

/// One event or hook of living things.
#[derive(Clone, Copy, Debug)]
pub struct LifeEvent {
    /// The name as rules write it.
    pub name: &'static str,
    /// The subjects it binds, in delivery order: name and kind. A name
    /// ending in `?` is bound only when there is one.
    pub subjects: &'static [(&'static str, &'static str)],
    /// Its fields: name, type as the language writes types, and what it
    /// holds.
    pub fields: &'static [(&'static str, &'static str, &'static str)],
    /// For a hook: the fields a handler may change. `None` for a
    /// notification.
    pub hook: Option<&'static [&'static str]>,
    /// Whether another module's registry step enters it in the
    /// expression registry (the stats' and the bodies' do).
    pub registered_elsewhere: bool,
    /// What happened, or for a hook what it precedes.
    pub doc: &'static str,
}

const SELF: &[(&str, &str)] = &[("self", "entity")];
const SELF_HERE: &[(&str, &str)] = &[("self", "entity"), ("here?", "loc")];
const PART: (&str, &str, &str) = ("part", "part", "The part.");
const KIND: (&str, &str, &str) = ("kind", "id<damage_type>", "The kind of damage.");

/// A notification about `self`.
const fn told(
    name: &'static str,
    fields: &'static [(&'static str, &'static str, &'static str)],
    registered_elsewhere: bool,
    doc: &'static str,
) -> LifeEvent {
    LifeEvent {
        name,
        subjects: SELF,
        fields,
        hook: None,
        registered_elsewhere,
        doc,
    }
}

/// Every event and hook of living things (06 §4.4, §7.6, §8, §10, §13).
pub const LIFE_EVENTS: &[LifeEvent] = &[
    // ---- Stats (06 §4.4) -------------------------------------------------
    told(
        "attr_changed",
        &[
            (
                "attr",
                "id<attribute>",
                "The attribute whose value changed.",
            ),
            ("old", "real", "Its value before."),
            ("new", "real", "Its value now."),
        ],
        true,
        "An attribute's value changed: its base, a modifier, or something its base reads.",
    ),
    told(
        "pool_crossed",
        &[
            ("pool", "id<pool>", "The pool."),
            (
                "threshold",
                "real",
                "The threshold crossed, as the fraction of `max` the definition lists.",
            ),
            ("rising", "bool", "True if the level rose to it or past it."),
            ("level", "real", "The pool's level after the crossing."),
        ],
        true,
        "A pool's level crossed one of its thresholds.",
    ),
    LifeEvent {
        name: "pool_changing",
        subjects: SELF,
        fields: &[
            ("pool", "id<pool>", "The pool."),
            (
                "amount",
                "real",
                "What is about to be added; negative to take away.",
            ),
        ],
        hook: Some(&["amount"]),
        registered_elsewhere: true,
        doc: "Runs before a verb changes a pool's level. A handler may change the amount or cancel.",
    },
    told(
        "track_advanced",
        &[
            ("track", "id<track>", "The track."),
            ("level", "int", "The level reached."),
        ],
        true,
        "A track reached a level.",
    ),
    // ---- Bodies and wounds (06 §7.6) -------------------------------------
    LifeEvent {
        name: "hit_incoming",
        subjects: SELF,
        fields: &[
            ("part", "part?", "The part the hit is aimed at."),
            (
                "height",
                "real?",
                "The height it arrives at, for a hit aimed at a height.",
            ),
            (
                "spread",
                "bool",
                "Whether it is spread over the whole body.",
            ),
            KIND,
            ("energy", "real", "Joules delivered."),
            (
                "intensity",
                "real",
                "How concentrated the delivery is, in pascals.",
            ),
            ("area", "real", "The area of contact, in square metres."),
            (
                "span",
                "real",
                "The length of contact across the part, in metres.",
            ),
            (
                "dir",
                "list<real>?",
                "The direction it travels in: east, south and up.",
            ),
        ],
        hook: Some(&["energy", "intensity", "part", "dir"]),
        registered_elsewhere: true,
        doc: "Runs before a hit is applied to a body. A handler may change its energy, its intensity, the part it lands on and its direction, or cancel it.",
    },
    told(
        "hit_landed",
        &[
            PART,
            KIND,
            ("absorbed", "real", "Joules the part's own layers took."),
            ("passed", "real", "Joules that went on through them."),
            (
                "depth",
                "int",
                "How many of the part's layers the hit got through.",
            ),
        ],
        true,
        "A hit did something to a part.",
    ),
    told(
        "wound_inflicted",
        &[
            ("wound", "wound", "The wound."),
            PART,
            KIND,
            ("depth", "int", "How many layers it goes through."),
            ("area", "real", "Its area, in square metres."),
        ],
        true,
        "A hit breached a part, or made a wound it had worse.",
    ),
    told(
        "wound_closed",
        &[("wound", "wound", "The wound."), PART],
        true,
        "A wound stopped leaking.",
    ),
    told(
        "wound_reopened",
        &[("wound", "wound", "The wound."), PART],
        true,
        "A closed wound was opened again.",
    ),
    told(
        "wound_healed",
        &[PART, KIND],
        true,
        "A wound is gone: every layer it went through is whole again.",
    ),
    told("part_disabled", &[PART], true, "A part stopped working."),
    told("part_restored", &[PART], true, "A part works again."),
    told(
        "part_severed",
        &[PART, ("parent", "part?", "The part it was joined to.")],
        true,
        "A part came off, with everything on it and in it.",
    ),
    told("part_destroyed", &[PART], true, "A part was destroyed."),
    told(
        "function_changed",
        &[
            ("function", "id<body_function>", "The function."),
            ("old", "real", "Its capacity before."),
            ("new", "real", "Its capacity now."),
        ],
        true,
        "What the body can do of something changed.",
    ),
    // ---- What the wiring itself raises -----------------------------------
    told(
        "slots_lost",
        &[
            ("part", "part", "The part that supplied them."),
            (
                "channels",
                "list<id<channel>>",
                "The channels, in order of channel.",
            ),
            (
                "counts",
                "list<int>",
                "How many slots of each, in the same order.",
            ),
        ],
        false,
        "Channel slots are gone with a part that stopped working or came off (06 §6.4). An action that held one of them is to be interrupted (03 §6.2).",
    ),
    told(
        "slots_gained",
        &[
            ("part", "part", "The part that supplies them."),
            (
                "channels",
                "list<id<channel>>",
                "The channels, in order of channel.",
            ),
            (
                "counts",
                "list<int>",
                "How many slots of each, in the same order.",
            ),
        ],
        false,
        "Channel slots are back with a part that works again (06 §6.4).",
    ),
    told(
        "body_destroyed",
        &[],
        false,
        "A body is in pieces: its root part was destroyed (06 §7.5). Whether that ends the creature is decided by its definitions (06 §8.3).",
    ),
    LifeEvent {
        name: "hit_exited",
        subjects: SELF_HERE,
        fields: &[
            (
                "part",
                "part?",
                "The part it left through; null if the hit met no part.",
            ),
            KIND,
            ("energy", "real", "Joules that left the far side."),
            (
                "intensity",
                "real",
                "How concentrated they are, in pascals.",
            ),
            (
                "dir",
                "list<real>?",
                "The direction they travel in: east, south and up.",
            ),
        ],
        hook: None,
        registered_elsewhere: false,
        doc: "Energy of a hit left the far side of a body (06 §7.2). With a direction, the block of the next cell along it has already been struck with it.",
    },
    LifeEvent {
        name: "exposure_incoming",
        subjects: SELF_HERE,
        fields: &[
            ("pool", "id<pool>", "The pool the dose adds to."),
            ("amount", "real", "The dose."),
            (
                "route",
                "string",
                "The route it takes: `breath`, `skin` or `any` from a field; `ingest`, `inject`, `skin` or `breath` for a substance (16 §9.5).",
            ),
            (
                "field",
                "string",
                "What it comes from: the qualified id of a `field`, or of the `substance` dosed (16 §9.5).",
            ),
        ],
        hook: Some(&["amount"]),
        registered_elsewhere: false,
        doc: "Runs before a dose reaches a body by any route (02 §16). A handler may change the amount, which is how a mask keeps part of it out, or cancel it.",
    },
    LifeEvent {
        name: "entity_dying",
        subjects: SELF,
        fields: &[],
        hook: Some(&[]),
        registered_elsewhere: false,
        doc: "Runs before an entity stops for good (06 §8.3). A handler may cancel: the entity then goes on.",
    },
    told(
        "entity_died",
        &[],
        false,
        "An entity's mind stopped driving it for good (06 §8.3). It is still there, with its body, its wounds and what it holds. Control of it was released first.",
    ),
    told(
        "mind_released",
        &[],
        false,
        "Raised at death, after `entity_died`: the hook for whatever collects minds (06 §8.3).",
    ),
    told(
        "entity_revived",
        &[],
        false,
        "A dead entity's mind drives it again (06 §8.3).",
    ),
    told(
        "entity_unconscious",
        &[],
        false,
        "An entity's mind stopped driving it for now: something its definitions say it cannot do without is gone (06 §8.3).",
    ),
    told(
        "entity_woke",
        &[],
        false,
        "An unconscious entity's mind drives it again.",
    ),
    LifeEvent {
        name: "mind_transferring",
        subjects: &[("self", "entity"), ("target?", "entity")],
        fields: &[(
            "swap",
            "bool",
            "Whether the two minds are to change places.",
        )],
        hook: Some(&[]),
        registered_elsewhere: false,
        doc: "Runs before a mind is moved from `self` to `target`, and before `self` is given a driver (06 §8.2). A handler may cancel.",
    },
    LifeEvent {
        name: "mind_transferred",
        subjects: &[("self", "entity"), ("target", "entity")],
        fields: &[("swap", "bool", "Whether the two minds changed places.")],
        hook: None,
        registered_elsewhere: false,
        doc: "The mind that was in `self` is now in `target`, with what is seated in it (06 §8.2).",
    },
    LifeEvent {
        name: "driver_changed",
        subjects: &[("self", "entity"), ("driver?", "entity")],
        fields: &[],
        hook: None,
        registered_elsewhere: false,
        doc: "Who acts for `self`'s body changed: `driver` is who does now, and is not bound when nobody does (06 §8.2).",
    },
    LifeEvent {
        name: "entity_spawned",
        subjects: SELF_HERE,
        fields: &[
            (
                "templates",
                "list<string>",
                "The qualified ids of the templates it was made from, in the order applied.",
            ),
            (
                "items",
                "list<string>",
                "What it is to start with, as its templates and chosen options wrote it: for whoever makes items.",
            ),
            (
                "traits",
                "list<string>",
                "The traits it is to start with, as written: for whoever gives traits.",
            ),
        ],
        hook: None,
        registered_elsewhere: false,
        doc: "An entity was made from templates and is complete (06 §10.3).",
    },
    told(
        "became_notable",
        &[],
        false,
        "An anonymous individual became notable: something about it would be lost by forgetting it (06 §12.2).",
    ),
];

/// The type a raised field is checked against, from how it is written.
fn field_type(written: &str) -> FieldType {
    match written {
        "real" => FieldType::Real,
        "int" => FieldType::Int,
        "bool" => FieldType::Bool,
        "string" => FieldType::Str,
        _ => FieldType::Any,
    }
}

/// The bus's type for each event of [`LIFE_EVENTS`].
pub(crate) fn event_types() -> Vec<EventType> {
    LIFE_EVENTS
        .iter()
        .map(|info| {
            let mut ty = EventType::new(info.name).doc(info.doc);
            for (name, kind) in info.subjects {
                ty = ty.subject(*name, *kind);
            }
            for (name, written, doc) in info.fields {
                ty = ty.field(*name, field_type(written), *doc);
                // The reference prints the type as the language writes it.
                if let Some(field) = ty.fields.last_mut() {
                    field.ty = (*written).to_owned();
                }
            }
            if let Some(mutable) = info.hook {
                ty = ty.hook(mutable.iter().copied());
            }
            ty
        })
        .collect()
}

/// Enters in the expression registry the events of [`LIFE_EVENTS`] that
/// no other step enters, leaving alone whatever is there already.
pub fn register(registry: &mut Registry) {
    for info in LIFE_EVENTS {
        if info.registered_elsewhere || registry.event_id(info.name).is_some() {
            continue;
        }
        let mut spec = EventSpec::new(info.name).doc(info.doc);
        let subjects = info
            .subjects
            .iter()
            .chain(crate::event::CAUSE_SUBJECTS.iter());
        for (name, kind) in subjects {
            if let Some(kind) = registry.kind_id(kind) {
                spec = spec.subject(name.trim_end_matches('?'), kind);
            }
        }
        for (name, written, doc) in info.fields {
            let ty = registry.parse_type(written).unwrap_or(Type::Any);
            spec = spec.field(*name, ty, *doc);
        }
        if let Some(mutable) = info.hook {
            spec = spec.hook(mutable.iter().copied());
        }
        let _ = registry.add_event(spec);
    }
}

/// The registry step of living things (04 §7.3): the events above, and
/// then the engine's verbs once more, since those whose arguments name
/// the kinds `part` and `wound` could not be entered before the bodies'
/// own step had registered those kinds.
pub fn registry_step(cx: &mut RegistryCx<'_>) {
    register(cx.registry());
    crate::verb::register_engine_verbs(cx.registry());
}

/// A definition as an event's field.
pub(crate) fn id_field(ty: Option<clash_expr::DefTypeId>, index: usize) -> FieldValue {
    match (ty, u32::try_from(index)) {
        (Some(ty), Ok(index)) => FieldValue::Plain(Value::Id {
            ty: ty.raw(),
            index,
        }),
        _ => FieldValue::Null,
    }
}

/// A handle of a kind the kernel's `Subject` does not name (a part, a
/// wound) as an event's field.
pub(crate) fn handle_field(handle: Handle) -> FieldValue {
    FieldValue::Plain(Value::Subject {
        kind: handle.kind.raw(),
        payload: handle.payload,
    })
}

/// A direction as an event's field.
pub(crate) fn dir_field(dir: Option<[Real; 3]>) -> FieldValue {
    match dir {
        Some(dir) => FieldValue::Plain(Value::List(dir.iter().map(|x| Value::Real(*x)).collect())),
        None => FieldValue::Null,
    }
}

/// A list of texts as an event's field.
pub(crate) fn texts_field(texts: &[String]) -> FieldValue {
    FieldValue::Plain(Value::List(
        texts.iter().map(|text| Value::Str(text.clone())).collect(),
    ))
}

/// The handle a field holds, if it holds one.
pub(crate) fn field_handle(field: Option<&FieldValue>) -> Option<Handle> {
    match field?.plain()? {
        Value::Subject { kind, payload } => {
            Some(Handle::new(clash_expr::KindId::new(*kind), *payload))
        }
        _ => None,
    }
}

/// The definition a field holds, if it holds one.
#[allow(dead_code)] // Used once templates are built (06 §10).
pub(crate) fn field_id(field: Option<&FieldValue>) -> Option<DefRef> {
    match field?.plain()? {
        Value::Id { ty, index } => Some(DefRef::new(clash_expr::DefTypeId::new(*ty), *index)),
        _ => None,
    }
}

/// The direction a field holds, if it holds three finite numbers.
pub(crate) fn field_dir(field: Option<&FieldValue>) -> Option<[Real; 3]> {
    let Value::List(items) = field?.plain()? else {
        return None;
    };
    let number = |value: &Value| match value {
        Value::Real(real) => Some(*real),
        Value::Int(int) => Some(Real::from_i64(*int)),
        _ => None,
    };
    match items.as_slice() {
        [x, y, z] => Some([number(x)?, number(y)?, number(z)?]),
        _ => None,
    }
}

/// A payload about an entity, bound as `self`.
pub(crate) fn about(name: &str, entity: EntityId) -> EventPayload {
    EventPayload::new(name).subject("self", Subject::Entity(entity))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_event_is_documented_and_a_hook_changes_only_its_own_fields() {
        let mut names: Vec<&str> = LIFE_EVENTS.iter().map(|info| info.name).collect();
        names.sort_unstable();
        names.dedup();
        assert_eq!(names.len(), LIFE_EVENTS.len(), "an event is listed twice");
        for info in LIFE_EVENTS {
            assert!(!info.doc.is_empty(), "`{}` has no documentation", info.name);
            for (name, _, doc) in info.fields {
                assert!(!doc.is_empty(), "`{}.{name}` is not documented", info.name);
            }
            for mutable in info.hook.unwrap_or(&[]) {
                assert!(
                    info.fields.iter().any(|(name, _, _)| name == mutable),
                    "`{}` lists `{mutable}` as mutable and has no such field",
                    info.name
                );
            }
        }
        assert_eq!(event_types().len(), LIFE_EVENTS.len());
    }
}
