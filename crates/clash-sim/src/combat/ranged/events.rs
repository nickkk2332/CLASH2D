//! The events of ranged combat (07 §11, §16.2), as one table, raised
//! and registered with combat's own.

use crate::combat::events::CombatEvent;

/// Raised for each projectile fired.
pub const PROJECTILE_FIRED: &str = "projectile_fired";
/// Raised when a projectile strikes a creature, an item or an element.
pub const PROJECTILE_HIT: &str = "projectile_hit";
/// Raised for whoever a projectile passed close to.
pub const PROJECTILE_PASSED: &str = "projectile_passed";
/// Raised when an item is thrown.
pub const THROWN: &str = "thrown";

const PROJECTILE: (&str, &str, &str) = (
    "projectile",
    "string",
    "The qualified id of the projectile; empty for an item thrown.",
);
const BODY: (&str, &str, &str) = (
    "body",
    "int?",
    "The number of its body in flight (02 §15); null for an instant shot.",
);

/// Every event of ranged combat (07 §16.2).
pub const RANGED_EVENTS: &[CombatEvent] = &[
    CombatEvent {
        name: PROJECTILE_FIRED,
        subjects: &[
            ("self", "entity"),
            ("target?", "entity"),
            ("weapon?", "entity"),
        ],
        fields: &[
            PROJECTILE,
            BODY,
            ("speed", "real", "Its speed as it left, in metres a second."),
            ("energy", "real", "The energy it left with, in joules."),
            (
                "noise",
                "real",
                "How loud the shot was: the weapon's `shot.noise`, or the effect's.",
            ),
            (
                "flash",
                "real",
                "How bright the shot was: the weapon's `shot.flash`, or the effect's.",
            ),
            (
                "instant",
                "bool",
                "Whether it was cast as a ray and struck at once (07 §11.3).",
            ),
        ],
        hook: None,
        doc: "`self` fired a projectile (07 §11.1), aimed at `target` if at anyone, with `weapon` if with anything. Raised once for each projectile, before any of it strikes. Content gives it off as a sound and a light (15 §3.2), so that shots are heard and seen.",
    },
    CombatEvent {
        name: PROJECTILE_HIT,
        subjects: &[("self?", "entity"), ("target?", "entity"), ("here", "loc")],
        fields: &[
            PROJECTILE,
            BODY,
            (
                "part",
                "part?",
                "The part of `target` it landed on; null for an element or an item.",
            ),
            ("energy", "real", "Joules it arrived with."),
            (
                "intensity",
                "real",
                "How concentrated, in pascals: the energy over its surface's area times its give.",
            ),
            (
                "absorbed",
                "real",
                "Joules what it struck took: the target's layers and what it wears, an item, an element.",
            ),
            (
                "passed",
                "real",
                "Joules it went on with beyond what it struck.",
            ),
            ("instant", "bool", "Whether it was an instant shot."),
        ],
        hook: None,
        doc: "A projectile, or an item thrown, fired by `self` struck `target`, or the element of the cell `here` (07 §11.1). What the hit did is told by the body's, the item's or the world's own events.",
    },
    CombatEvent {
        name: PROJECTILE_PASSED,
        subjects: &[("self", "entity"), ("shooter?", "entity")],
        fields: &[
            PROJECTILE,
            BODY,
            (
                "distance",
                "real",
                "How close it passed to the middle of `self`, in metres; zero for one that went by at a height where nothing of `self` was.",
            ),
        ],
        hook: None,
        doc: "A projectile, or an item thrown, fired by `shooter` passed within `projectile.near_miss` of `self` without striking it (07 §11.1). Told once for each projectile. What being shot at does to a person is content.",
    },
    CombatEvent {
        name: THROWN,
        subjects: &[
            ("self", "entity"),
            ("what", "entity"),
            ("target?", "entity"),
        ],
        fields: &[
            (
                "speed",
                "real",
                "Its speed as it left the hand, in metres a second.",
            ),
            ("energy", "real", "The energy it left with, in joules."),
        ],
        hook: None,
        doc: "`self` threw the item `what` (07 §11.1), aimed at `target` if at anyone. It flies as a body (02 §15) and strikes with the surface it was thrown with.",
    },
];
