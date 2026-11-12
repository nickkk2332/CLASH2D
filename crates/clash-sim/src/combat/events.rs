//! The events and hooks of combat (07 §16), as one table: what the bus
//! is told so that they can be raised, and what the expression registry
//! is told so that rules can listen for them.
//!
//! Every event also binds `actor` and `instigator` from its cause: for
//! a strike, the attacker. Ranged combat's events
//! ([`RANGED_EVENTS`](super::ranged::RANGED_EVENTS)) are raised and
//! registered with these.

use clash_data::RegistryCx;
use clash_expr::{EventSpec, Registry, Type};

use crate::bus::{EventType, FieldType};

/// Raised when a strike landed on a target or an element.
pub const STRIKE_LANDED: &str = "strike_landed";
/// Raised when a strike missed a target.
pub const STRIKE_MISSED: &str = "strike_missed";
/// Raised when a strike reached a target and nothing of it was
/// delivered.
pub const STRIKE_STOPPED: &str = "strike_stopped";
/// Raised when energy came back into the surface that struck.
pub const STRIKE_RECOILED: &str = "strike_recoiled";
/// Raised when a push staggered its target.
pub const STAGGERED: &str = "staggered";
/// Raised when a push launched its target as a body in flight.
pub const LAUNCHED: &str = "launched";
/// The hook raised before a strike is resolved.
pub const STRIKE_RESOLVING: &str = "strike_resolving";
/// The hook raised before a push moves its target.
pub const KNOCKBACK_INCOMING: &str = "knockback_incoming";

/// One event or hook of combat.
#[derive(Clone, Copy, Debug)]
pub struct CombatEvent {
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
    /// What happened, or for a hook what it precedes.
    pub doc: &'static str,
}

const ACTION: (&str, &str, &str) = (
    "action",
    "string",
    "The qualified id of the action whose attack it is.",
);
const ATTACK: (&str, &str, &str) = ("attack", "string", "The id of the attack within it.");

/// Every event and hook of combat that is built (07 §16).
pub const COMBAT_EVENTS: &[CombatEvent] = &[
    CombatEvent {
        name: STRIKE_RESOLVING,
        subjects: &[("self", "entity")],
        fields: &[
            ACTION,
            ATTACK,
            (
                "target",
                "entity?",
                "Whom it strikes; null for a strike at a cell.",
            ),
            (
                "part",
                "part?",
                "The part of the target it is aimed at, if one is.",
            ),
            (
                "energy",
                "real",
                "Joules at full effort: what the attack's `energy` gave.",
            ),
            ("effort", "real", "The share of that put into it, 0 to 1."),
        ],
        hook: Some(&["target", "part", "energy", "effort"]),
        doc: "Runs before a strike by `self` is resolved (07 §3.4, step 1). A handler may change whom it strikes, the part it is aimed at, its energy and its effort, or cancel: nothing is struck and the action's `$landed` stays false.",
    },
    CombatEvent {
        name: STRIKE_LANDED,
        subjects: &[("self", "entity"), ("target?", "entity"), ("here?", "loc")],
        fields: &[
            ACTION,
            ATTACK,
            (
                "part",
                "part?",
                "The part it landed on; null where the body chose one, or for an element.",
            ),
            (
                "kind",
                "id<damage_type>",
                "The kind of damage it delivered.",
            ),
            (
                "energy",
                "real",
                "Joules it delivered as harm: its energy at the effort given, less the push.",
            ),
            (
                "intensity",
                "real",
                "How concentrated that was, in pascals: the energy over the surface's area times its give (07 §6.1).",
            ),
            (
                "absorbed",
                "real",
                "Joules the target's layers and what it wears took, or the element took.",
            ),
            ("passed", "real", "Joules that went on beyond it."),
            (
                "returned",
                "real",
                "Joules that came back into the surface that struck (07 §6.3).",
            ),
            (
                "push",
                "real",
                "Joules set aside to push the target (07 §7).",
            ),
            ("effort", "real", "The effort it was made with, 0 to 1."),
            (
                "margin",
                "real?",
                "The margin of its aim; null for a strike that did not aim.",
            ),
        ],
        hook: None,
        doc: "A strike by `self` landed on `target`, or on the element of the cell `here` (07 §3.4, step 8). What the hit did to the body is told by the body's own events (06 §7.6).",
    },
    CombatEvent {
        name: STRIKE_MISSED,
        subjects: &[("self", "entity"), ("target?", "entity")],
        fields: &[
            ACTION,
            ATTACK,
            (
                "why",
                "string",
                "`reach`: the target was not within reach; `aim`: the margin of aim was too low; `nothing`: the target has no body and is not an item.",
            ),
            ("margin", "real?", "The margin of its aim, where it aimed."),
        ],
        hook: None,
        doc: "A strike by `self` missed `target` (07 §3.4, steps 2 and 4).",
    },
    CombatEvent {
        name: STRIKE_STOPPED,
        subjects: &[("self", "entity"), ("target?", "entity")],
        fields: &[
            ACTION,
            ATTACK,
            (
                "code",
                "string",
                "The code of the reason nothing was delivered.",
            ),
            ("message", "string", "The reason, as a sentence."),
        ],
        hook: None,
        doc: "A strike by `self` reached `target` and nothing of it was delivered: a handler of `hit_incoming` cancelled the hit (06 §7.6).",
    },
    CombatEvent {
        name: STRIKE_RECOILED,
        subjects: &[("self", "entity"), ("weapon?", "entity")],
        fields: &[
            ACTION,
            ATTACK,
            (
                "part",
                "part?",
                "The part of `self` whose surface struck and took it back; null for an item or an inline surface.",
            ),
            (
                "energy",
                "real",
                "Joules that came back into the surface (07 §6.3).",
            ),
            (
                "absorbed",
                "real",
                "Joules of it the part's layers or the item took.",
            ),
        ],
        hook: None,
        doc: "Energy a strike by `self` delivered that the target neither absorbed nor passed came back into the surface that struck: the part of `self`, or the item `weapon` (07 §6.3). Whatever it did is told by the body's or the item's own events. Not raised for an inline surface, which takes nothing.",
    },
    CombatEvent {
        name: STAGGERED,
        subjects: &[("self", "entity")],
        fields: &[
            (
                "speed",
                "real",
                "The speed the push would give it, in metres a second.",
            ),
            (
                "severity",
                "real",
                "Where that speed lies between `combat.stagger_at` and `combat.launch_at`, from 0 to 1.",
            ),
            (
                "dir",
                "list<real>",
                "The direction of the push: east, south, up.",
            ),
        ],
        hook: None,
        doc: "A push staggered `self` (07 §7): too slow to move it, fast enough to unbalance it. What staggering does is content.",
    },
    CombatEvent {
        name: LAUNCHED,
        subjects: &[("self", "entity")],
        fields: &[
            ("speed", "real", "Its speed, in metres a second."),
            ("dir", "list<real>", "Its direction: east, south, up."),
            (
                "added",
                "bool",
                "True if it was already in flight and the push was added to its velocity (07 §7.1).",
            ),
        ],
        hook: None,
        doc: "A push launched `self` as a body in flight (02 §15), or added to the velocity of one already in flight. The world carries it until it lands (`landed`).",
    },
    CombatEvent {
        name: KNOCKBACK_INCOMING,
        subjects: &[("self", "entity")],
        fields: &[
            (
                "speed",
                "real",
                "The speed the push gives, in metres a second.",
            ),
            (
                "dir",
                "list<real>",
                "Its direction: east, south, up, of length one.",
            ),
        ],
        hook: Some(&["speed", "dir"]),
        doc: "Runs before a push moves `self` (07 §7). A handler may change the speed and the direction, or cancel: nothing moves it.",
    },
    CombatEvent {
        name: super::defence::DEFENCE_RESOLVING,
        subjects: &[
            ("self", "entity"),
            ("attacker", "entity"),
            ("target", "entity"),
        ],
        fields: &[
            (
                "action",
                "string",
                "The qualified id of the defending action.",
            ),
            ("form", "string", "`avoid` or `interpose`."),
            (
                "succeeded",
                "bool",
                "Whether it succeeds: its contest's margin was at or above zero, or it has none.",
            ),
            (
                "margin",
                "real?",
                "The margin of its contest; null for a defence with none.",
            ),
            (
                "deflect",
                "real",
                "The share of the strike's energy it turns aside, 0 to 1.",
            ),
            (
                "holds",
                "real?",
                "The most joules it can turn aside; null for no limit.",
            ),
            ("brace", "real", "The share of the push it resists, 0 to 1."),
        ],
        hook: Some(&["succeeded", "deflect", "holds", "brace"]),
        doc: "Runs before a defence of `self` in force is applied to a strike by `attacker` at `target` (07 §5, step 5 of 07 §3.4). A handler may change how much it turns (`deflect`, `holds`), what it braces, or make it fail (`succeeded` false, or cancel).",
    },
    CombatEvent {
        name: super::defence::STRIKE_DEFENDED,
        subjects: &[
            ("self", "entity"),
            ("target", "entity"),
            ("defender", "entity"),
        ],
        fields: &[
            ACTION,
            ATTACK,
            (
                "defence",
                "string",
                "The qualified id of the defending action.",
            ),
            ("form", "string", "`avoid` or `interpose`."),
            (
                "succeeded",
                "bool",
                "Whether it succeeded. An `avoid` that succeeded makes the strike miss (`strike_missed` with `why` `defence`).",
            ),
            (
                "margin",
                "real?",
                "The margin of the defence's contest; null for one with none.",
            ),
            (
                "turned",
                "real",
                "Joules it turned aside (07 §5.3); zero for one that avoids or failed.",
            ),
        ],
        hook: None,
        doc: "A strike by `self` at `target` met a defence of `defender` in force that covered it (07 §5, step 5 of 07 §3.4). Raised once for each defence tried, in the order they are tried.",
    },
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

/// The bus's type for each event of [`COMBAT_EVENTS`].
pub fn event_types() -> Vec<EventType> {
    COMBAT_EVENTS
        .iter()
        .chain(super::ranged::RANGED_EVENTS)
        .map(|info| {
            let mut ty = EventType::new(info.name).doc(info.doc);
            for (name, kind) in info.subjects {
                ty = ty.subject(*name, *kind);
            }
            for (name, written, doc) in info.fields {
                ty = ty.field(*name, field_type(written), *doc);
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

/// Enters in the expression registry the events of [`COMBAT_EVENTS`],
/// leaving alone whatever is there already.
pub fn register(registry: &mut Registry) {
    for info in COMBAT_EVENTS.iter().chain(super::ranged::RANGED_EVENTS) {
        if registry.event_id(info.name).is_some() {
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

/// Combat's registry step (04 §7.3): its events, which name the kind
/// `part` that the bodies' own step registers.
pub fn registry_step(cx: &mut RegistryCx<'_>) {
    register(cx.registry());
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_event_is_documented_and_a_hook_changes_only_its_own_fields() {
        let mut names: Vec<&str> = COMBAT_EVENTS.iter().map(|info| info.name).collect();
        names.sort_unstable();
        names.dedup();
        assert_eq!(names.len(), COMBAT_EVENTS.len(), "an event is listed twice");
        for info in COMBAT_EVENTS {
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
        assert_eq!(
            event_types().len(),
            COMBAT_EVENTS.len() + super::super::ranged::RANGED_EVENTS.len()
        );
    }
}
