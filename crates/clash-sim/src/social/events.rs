//! The events and hooks of the social layer (15 §8 to §11), entered in
//! every simulation's event bus and in the engine's registry, and listed
//! for the generated reference.

use clash_core::EntityId;
use clash_expr::{EventSpec, Registry, Type};
use clash_world::Cause;

use super::defs::{GroupDef, LinesDef, MessageKindDef, RelationKindDef};
use super::order::{Order, id_value};
use super::relation::Toward;
use crate::Sim;
use crate::bus::{EventPayload, EventType, FieldSpec, FieldType, FieldValue};
use crate::event::{CAUSE_SUBJECTS, Subject, Where};
use crate::perception::PersonaId;
use crate::reason::Reason;

/// `identity_check`: whether a disguise holds for an observer (15 §8.2).
pub const IDENTITY_CHECK: &str = "identity_check";
/// `disguise_seen_through`: an observer saw through a disguise.
pub const DISGUISE_SEEN_THROUGH: &str = "disguise_seen_through";
/// `relation_changed`: a mind's relation changed (15 §9.1).
pub const RELATION_CHANGED: &str = "relation_changed";
/// `group_joined`: an entity joined a group (15 §9.2).
pub const GROUP_JOINED: &str = "group_joined";
/// `group_left`: an entity left a group.
pub const GROUP_LEFT: &str = "group_left";
/// `group_created`: a group was made in play.
pub const GROUP_CREATED: &str = "group_created";
/// `group_dissolved`: a group was dissolved.
pub const GROUP_DISSOLVED: &str = "group_dissolved";
/// `role_changed`: a member's role in a group changed.
pub const ROLE_CHANGED: &str = "role_changed";
/// `order_received`: the hook run when an order is heard (15 §10).
pub const ORDER_RECEIVED: &str = "order_received";
/// `order_given`: an order was kept by the one it was given to.
pub const ORDER_GIVEN: &str = "order_given";
/// `order_refused`: an order heard was not obeyed.
pub const ORDER_REFUSED: &str = "order_refused";
/// `order_cancelled`: an order kept was withdrawn or ran out.
pub const ORDER_CANCELLED: &str = "order_cancelled";
/// `understands`: the hook run when a message is heard (15 §11).
pub const UNDERSTANDS: &str = "understands";
/// `message_heard`: a message was heard and understood.
pub const MESSAGE_HEARD: &str = "message_heard";
/// `talk_options`: the hook that gathers what one can say to another.
pub const TALK_OPTIONS: &str = "talk_options";

/// One event or hook: its name, subjects, fields, the fields a handler
/// may change and what it is.
pub struct EventRow {
    /// Its name.
    pub name: &'static str,
    /// Its subjects: name (`?` when it may be left out) and kind.
    pub subjects: &'static [(&'static str, &'static str)],
    /// Its fields: name, type and what it holds.
    pub fields: &'static [(&'static str, &'static str, &'static str)],
    /// The fields a handler of a hook may change.
    pub mutable: &'static [&'static str],
    /// Whether it is a hook.
    pub hook: bool,
    /// What happened, or what a handler decides.
    pub doc: &'static str,
}

const SELF: (&str, &str) = ("self", "entity");
const GROUP_FIELD: (&str, &str, &str) = ("group", "string", "The group's key (15 §9.2).");
const ACTION_FIELD: (&str, &str, &str) = (
    "action",
    "any",
    "The action the order says to do: an id of an `action`.",
);

/// Every event and hook of the social layer.
pub const SOCIAL_EVENTS: &[EventRow] = &[
    EventRow {
        name: IDENTITY_CHECK,
        subjects: &[SELF, ("about", "entity")],
        fields: &[
            (
                "sense",
                "string",
                "The qualified id of the sense it was detected by.",
            ),
            (
                "quality",
                "real",
                "How well what `about` presents holds as a disguise, 0 to 1: the persona's `quality`, or a copy's.",
            ),
            ("scrutiny", "real", "The sense's `scrutiny`."),
            (
                "familiarity",
                "real",
                "How well the observer knows `about`'s true persona: the confidence its track of it had when last brought up to date, 0 for none.",
            ),
            (
                "holds",
                "bool",
                "Whether the disguise holds: true unless a handler sets it false. Mutable.",
            ),
        ],
        mutable: &["holds"],
        hook: true,
        doc: "Runs when an observer (`self`) detects an entity (`about`) that presents a persona other than its true one and that the observer has not linked to it (15 §8.2). A handler sets `holds`; with none, the disguise holds. Seen through, the observer takes `about` for its true persona, and `disguise_seen_through` follows.",
    },
    EventRow {
        name: DISGUISE_SEEN_THROUGH,
        subjects: &[SELF, ("about", "entity")],
        fields: &[
            (
                "shown",
                "string",
                "The persona `about` presents, as text (`7v1/2`).",
            ),
            (
                "seen",
                "string",
                "The persona the observer now takes it for: its true one.",
            ),
            (
                "how",
                "string",
                "`checked`, by `identity_check`, or `revealed`, by `reveal_persona`.",
            ),
        ],
        mutable: &[],
        hook: false,
        doc: "An observer saw through what an entity presents (15 §8.2): it takes it for its true persona, and where the persona shown is one the entity made, it has linked the two in its own beliefs, and in nobody else's.",
    },
    EventRow {
        name: RELATION_CHANGED,
        subjects: &[SELF],
        fields: &[
            ("kind", "string", "The relation kind's qualified id."),
            (
                "toward",
                "string",
                "Whom it is held toward: a persona as text, or a group's key.",
            ),
            ("group", "bool", "Whether `toward` is a group."),
            ("value", "real", "The value now."),
            (
                "was",
                "real",
                "The value before: the kind's default if it was not held.",
            ),
        ],
        mutable: &[],
        hook: false,
        doc: "A relation a mind holds of its own changed (15 §9.1): set, added to or cleared. Fading is not a change: it is worked out when the relation is read.",
    },
    EventRow {
        name: GROUP_JOINED,
        subjects: &[SELF],
        fields: &[GROUP_FIELD, ("role", "any", "Its role, or null.")],
        mutable: &[],
        hook: false,
        doc: "An entity joined a group (15 §9.2), and was told what the group knows.",
    },
    EventRow {
        name: GROUP_LEFT,
        subjects: &[SELF],
        fields: &[GROUP_FIELD, ("role", "any", "The role it had, or null.")],
        mutable: &[],
        hook: false,
        doc: "An entity left a group, or the group was dissolved (15 §9.2).",
    },
    EventRow {
        name: GROUP_CREATED,
        subjects: &[],
        fields: &[
            GROUP_FIELD,
            ("def", "string", "The group definition's qualified id."),
        ],
        mutable: &[],
        hook: false,
        doc: "A group was made in play by `create_group` (15 §9.2).",
    },
    EventRow {
        name: GROUP_DISSOLVED,
        subjects: &[],
        fields: &[GROUP_FIELD],
        mutable: &[],
        hook: false,
        doc: "A group was dissolved by `dissolve_group` (15 §9.2): its members have left it.",
    },
    EventRow {
        name: ROLE_CHANGED,
        subjects: &[SELF],
        fields: &[
            GROUP_FIELD,
            ("role", "any", "Its role now, or null."),
            ("was", "any", "Its role before, or null."),
        ],
        mutable: &[],
        hook: false,
        doc: "A member's role in a group changed by `set_role` (15 §9.2).",
    },
    EventRow {
        name: ORDER_RECEIVED,
        subjects: &[SELF, ("giver", "entity")],
        fields: &[
            ACTION_FIELD,
            ("with", "any", "The action's parameters, by name."),
            (
                "obeys",
                "bool",
                "Whether it is obeyed: true if `self` holds toward whom it takes `giver` for a relation of a kind flagged `orders` above its default. Mutable.",
            ),
        ],
        mutable: &["obeys"],
        hook: true,
        doc: "Runs when an order is heard by the one it was given to, who made out the giver (15 §10). A handler may set `obeys`, or cancel with a reason; not obeyed, `order_refused` follows, else the order is kept and `order_given` follows.",
    },
    EventRow {
        name: ORDER_GIVEN,
        subjects: &[SELF, ("giver?", "entity")],
        fields: &[
            ("order", "int", "The order's number, for `cancel_order`."),
            ACTION_FIELD,
            ("with", "any", "The action's parameters, by name."),
            (
                "until",
                "any",
                "When it stops standing, in seconds of universal time from now, or null for good.",
            ),
        ],
        mutable: &[],
        hook: false,
        doc: "An order was heard and kept by the one it was given to (15 §10): it stands until cancelled, until its time is up, or until the relation that made it one to obey is lost. What is done about it is a goal's, or a rule's (the bundled `core:do_as_ordered`).",
    },
    EventRow {
        name: ORDER_REFUSED,
        subjects: &[SELF, ("giver?", "entity")],
        fields: &[
            ACTION_FIELD,
            (
                "reason",
                "string",
                "The code of why: `social.not_obeyed`, `social.giver_unknown`, or a handler's.",
            ),
            ("message", "string", "Why, in words."),
        ],
        mutable: &[],
        hook: false,
        doc: "An order heard by the one it was given to was not obeyed (15 §10): it holds no relation flagged `orders` toward the giver, did not make the giver out, or a handler of `order_received` refused.",
    },
    EventRow {
        name: ORDER_CANCELLED,
        subjects: &[SELF],
        fields: &[
            ("order", "int", "The order's number."),
            (
                "how",
                "string",
                "`cancelled` by `cancel_order`, `expired` past its `until`, or `relation_lost` when the relation that made it one to obey fell to its default.",
            ),
        ],
        mutable: &[],
        hook: false,
        doc: "An order an entity kept no longer stands (15 §10).",
    },
    EventRow {
        name: UNDERSTANDS,
        subjects: &[SELF, ("speaker?", "entity")],
        fields: &[
            ("kind", "string", "The message kind's qualified id."),
            (
                "language",
                "any",
                "The language it was said in, as written, or null.",
            ),
            (
                "understood",
                "bool",
                "Whether the hearer understands it: true unless a handler sets it false. Mutable.",
            ),
        ],
        mutable: &["understood"],
        hook: true,
        doc: "Runs when a message is heard (15 §11): a handler that sets `understood` false, or cancels, keeps it from the hearer, who then neither hears it nor gains what it carries. `speaker` is bound if the hearer made out who spoke.",
    },
    EventRow {
        name: MESSAGE_HEARD,
        subjects: &[
            SELF,
            ("speaker?", "entity"),
            ("about?", "entity"),
            ("to?", "entity"),
        ],
        fields: &[
            ("kind", "string", "The message kind's qualified id."),
            (
                "text",
                "any",
                "The line it was said with, written for the hearer as reader, or null.",
            ),
            (
                "addressed",
                "bool",
                "Whether it was said to the hearer: to it, to a group it belongs to, or to nobody in particular.",
            ),
            (
                "report",
                "bool",
                "Whether the hearer gained beliefs from it: a report said to it that carried something.",
            ),
            (
                "clarity",
                "real",
                "How far over its sense's threshold it was heard.",
            ),
            (
                "where",
                "loc",
                "Where it seemed to come from: the speaker's cell when it was said.",
            ),
        ],
        mutable: &[],
        hook: false,
        doc: "A message was heard and understood (15 §11). `speaker` is bound if the hearer made out who spoke; `about` and `to` if the message named them. A report's beliefs have been gained, and an order received, before this is delivered.",
    },
    EventRow {
        name: TALK_OPTIONS,
        subjects: &[SELF, ("to", "entity")],
        fields: &[(
            "options",
            "any",
            "What `self` can say to `to`: a list of maps, each with `id`, `text` and, if it says something, `kind` (a message kind) and `lines`. Starts empty; handlers add to it. Mutable.",
        )],
        mutable: &["options"],
        hook: true,
        doc: "Runs when the application asks what one can say to another (15 §11): rules add options by what each believes; the application offers them through `ask`.",
    },
];

/// The type of a field as written in a row.
fn field_type(written: &str) -> FieldType {
    FieldType::parse(written).unwrap_or(FieldType::Any)
}

/// The event types of the social layer, for the simulation's bus.
pub fn event_types() -> Vec<EventType> {
    SOCIAL_EVENTS
        .iter()
        .map(|row| {
            let mut ty = EventType::new(row.name).doc(row.doc);
            for (name, kind) in row.subjects {
                ty = ty.subject(*name, *kind);
            }
            ty.fields = row
                .fields
                .iter()
                .map(|(name, written, doc)| FieldSpec {
                    name: (*name).to_owned(),
                    ty: (*written).to_owned(),
                    check: Some(field_type(written)),
                    doc: (*doc).to_owned(),
                })
                .collect();
            if row.hook {
                ty = ty.hook(row.mutable.iter().copied());
            }
            ty
        })
        .collect()
}

/// The hooks of the social layer, for whoever lists hooks apart.
pub fn hook_types() -> Vec<EventType> {
    event_types().into_iter().filter(|ty| ty.hook).collect()
}

/// Enters in a registry what the social layer's properties, verbs and
/// events name: its definition types and its events, unless they are
/// there already.
pub fn register_types(registry: &mut Registry) {
    for (name, doc) in [
        (RelationKindDef::TYPE, RelationKindDef::DOC),
        (GroupDef::TYPE, GroupDef::DOC),
        (MessageKindDef::TYPE, MessageKindDef::DOC),
        (LinesDef::TYPE, LinesDef::DOC),
    ] {
        if registry.def_type_id(name).is_none() {
            let _ = registry.add_def_type(name, doc);
        }
    }
    for row in SOCIAL_EVENTS {
        if registry.event_id(row.name).is_some() {
            continue;
        }
        let mut spec = EventSpec::new(row.name).doc(row.doc);
        for (name, kind) in row.subjects.iter().chain(CAUSE_SUBJECTS) {
            if let Some(kind) = registry.kind_id(kind) {
                spec = spec.subject(name.trim_end_matches('?'), kind);
            }
        }
        for (name, written, doc) in row.fields {
            let ty = registry.parse_type(written).unwrap_or(Type::Any);
            spec = spec.field(*name, ty, *doc);
        }
        if row.hook {
            spec = spec.hook(row.mutable.iter().copied());
        }
        let _ = registry.add_event(spec);
    }
}

use clash_data::Def as _;

/// Raises a notification of this layer, reporting a refusal as a fault.
fn notify(sim: &mut Sim, on: Where, cause: Cause, payload: EventPayload) {
    if let Err(reason) = sim.raise(on, cause, payload) {
        sim.fault(reason);
    }
}

fn opt_text(text: Option<&str>) -> FieldValue {
    text.map_or(FieldValue::Null, FieldValue::text)
}

pub(crate) fn disguise_seen_through(
    sim: &mut Sim,
    observer: EntityId,
    about: EntityId,
    shown: PersonaId,
    seen: PersonaId,
    how: &str,
) {
    let payload = EventPayload::new(DISGUISE_SEEN_THROUGH)
        .subject("self", Subject::Entity(observer))
        .subject("about", Subject::Entity(about))
        .with("shown", shown.to_string().as_str())
        .with("seen", seen.to_string().as_str())
        .with("how", how);
    // What an observer works out names no actor (15 §6).
    let cause = Cause::new(clash_world::CauseKind::Process);
    notify(sim, Where::Entity(observer), cause, payload);
}

pub(crate) fn relation_changed(
    sim: &mut Sim,
    who: EntityId,
    toward: &Toward,
    kind: &str,
    value: clash_core::Real,
    was: clash_core::Real,
    cause: Cause,
) {
    let payload = EventPayload::new(RELATION_CHANGED)
        .subject("self", Subject::Entity(who))
        .with("kind", kind)
        .with("toward", toward.to_string().as_str())
        .with("group", matches!(toward, Toward::Group(_)))
        .with("value", value)
        .with("was", was);
    notify(sim, Where::Entity(who), cause, payload);
}

pub(crate) fn group_joined(
    sim: &mut Sim,
    who: EntityId,
    group: &str,
    role: Option<&str>,
    cause: Cause,
) {
    let payload = EventPayload::new(GROUP_JOINED)
        .subject("self", Subject::Entity(who))
        .with("group", group)
        .with("role", opt_text(role));
    notify(sim, Where::Entity(who), cause, payload);
}

pub(crate) fn group_left(
    sim: &mut Sim,
    who: EntityId,
    group: &str,
    role: Option<&str>,
    cause: Cause,
) {
    let payload = EventPayload::new(GROUP_LEFT)
        .subject("self", Subject::Entity(who))
        .with("group", group)
        .with("role", opt_text(role));
    notify(sim, Where::Entity(who), cause, payload);
}

pub(crate) fn group_created(sim: &mut Sim, group: &str, def: &str, cause: Cause) {
    let payload = EventPayload::new(GROUP_CREATED)
        .with("group", group)
        .with("def", def);
    notify(sim, Where::Global, cause, payload);
}

pub(crate) fn group_dissolved(sim: &mut Sim, group: &str, cause: Cause) {
    let payload = EventPayload::new(GROUP_DISSOLVED).with("group", group);
    notify(sim, Where::Global, cause, payload);
}

pub(crate) fn role_changed(
    sim: &mut Sim,
    who: EntityId,
    group: &str,
    role: Option<&str>,
    was: Option<&str>,
    cause: Cause,
) {
    let payload = EventPayload::new(ROLE_CHANGED)
        .subject("self", Subject::Entity(who))
        .with("group", group)
        .with("role", opt_text(role))
        .with("was", opt_text(was));
    notify(sim, Where::Entity(who), cause, payload);
}

pub(crate) fn order_given(
    sim: &mut Sim,
    who: EntityId,
    order: &Order,
    action: clash_core::Value,
    cause: Cause,
) {
    let until = order.plan.until.map_or(FieldValue::Null, |until| {
        FieldValue::from(until.as_secs_real())
    });
    let payload = EventPayload::new(ORDER_GIVEN)
        .subject("self", Subject::Entity(who))
        .subject("giver", Subject::Entity(order.giver))
        .with("order", id_value(order.id))
        .with("action", FieldValue::Plain(action))
        .with(
            "with",
            FieldValue::Plain(clash_core::Value::Map(
                order.plan.params.iter().cloned().collect(),
            )),
        )
        .with("until", until);
    notify(sim, Where::Entity(who), cause, payload);
}

pub(crate) fn order_refused(
    sim: &mut Sim,
    who: EntityId,
    giver: Option<EntityId>,
    action: clash_core::Value,
    reason: &Reason,
    cause: Cause,
) {
    let mut payload = EventPayload::new(ORDER_REFUSED).subject("self", Subject::Entity(who));
    if let Some(giver) = giver {
        payload = payload.subject("giver", Subject::Entity(giver));
    }
    let payload = payload
        .with("action", FieldValue::Plain(action))
        .with("reason", reason.code.as_str())
        .with("message", reason.message.as_str());
    notify(sim, Where::Entity(who), cause, payload);
}

pub(crate) fn order_cancelled(sim: &mut Sim, who: EntityId, order: u64, how: &str, cause: Cause) {
    let payload = EventPayload::new(ORDER_CANCELLED)
        .subject("self", Subject::Entity(who))
        .with("order", id_value(order))
        .with("how", how);
    notify(sim, Where::Entity(who), cause, payload);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_events_and_hooks_are_in_the_engine_registry() {
        let registry = crate::engine_registry();
        for row in SOCIAL_EVENTS {
            let id = registry
                .event_id(row.name)
                .unwrap_or_else(|| panic!("`{}` is not registered", row.name));
            let spec = &registry.event(id).unwrap().spec;
            assert_eq!(spec.fields.len(), row.fields.len(), "{}", row.name);
            assert_eq!(spec.hook, row.hook, "{}", row.name);
            assert!(!row.doc.is_empty());
        }
        for ty in [
            RelationKindDef::TYPE,
            GroupDef::TYPE,
            MessageKindDef::TYPE,
            LinesDef::TYPE,
        ] {
            assert!(registry.def_type_id(ty).is_some(), "{ty}");
        }
    }
}
