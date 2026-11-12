//! Why combat declines (14 §5), and the settings it reads (07 §16).

use crate::reason::ReasonInfo;

/// A `strike` is written where there is no action, or in an action
/// that has no such attack.
pub const NO_ATTACK: &str = "combat.no_attack";
/// A `strike` names neither a target that is an entity nor a cell.
pub const NO_TARGET: &str = "combat.no_target";
/// The actor has no surface of the kind the attack's `with` names.
pub const NO_SURFACE: &str = "combat.no_surface";
/// An attack's `with` names something gripped, and nothing is.
pub const NOTHING_GRIPPED: &str = "combat.nothing_gripped";
/// An attack's energy, push, lift or reach, or a strike's effort, came
/// out as nothing a strike can use.
pub const BAD_NUMBER: &str = "combat.bad_number";
/// The part a strike is aimed at is not a part of its target.
pub const NOT_ITS_PART: &str = "combat.not_its_part";
/// The cell a strike is aimed at is not loaded, or holds no element.
pub const NOTHING_THERE: &str = "combat.nothing_there";

/// A strike was resolved inside too many others.
pub const TOO_DEEP: &str = "combat.too_deep";

/// Every reason combat gives, for the generated reference.
pub const COMBAT_REASONS: &[ReasonInfo] = &[
    ReasonInfo {
        code: TOO_DEEP,
        doc: "A strike was resolved inside eight others: one whose `on_land` started an action that struck at once, and so on. Values: `limit`.",
    },
    ReasonInfo {
        code: NO_ATTACK,
        doc: "A `strike` is written in a list that belongs to no action, or names an attack its action does not have, or names none in an action that has several. Values: `action`, `attack`.",
    },
    ReasonInfo {
        code: NO_TARGET,
        doc: "A `strike` was given neither an entity to strike nor a cell, or a target that is not in the world. Values: `target`.",
    },
    ReasonInfo {
        code: NO_SURFACE,
        doc: "The attacker has nothing that answers to the attack's `with`: no working part of that name with that surface, no item held with that tag and surface. Values: `action`, `attack`, `with`.",
    },
    ReasonInfo {
        code: NOTHING_GRIPPED,
        doc: "The attack strikes with something gripped (07 §9), and the attacker grips nothing: grips are not built yet. Values: `action`, `attack`.",
    },
    ReasonInfo {
        code: BAD_NUMBER,
        doc: "A number a strike needs came out as nothing it can use: an energy or a reach below zero or not finite, a push or an effort outside 0 to 1, a lift that is not finite. Values: `what`, `value`.",
    },
    ReasonInfo {
        code: NOT_ITS_PART,
        doc: "The part a strike is aimed at is not a part of the entity it strikes. Values: `target`.",
    },
    ReasonInfo {
        code: NOTHING_THERE,
        doc: "A strike at a cell found the cell not loaded, outside its grid, or holding no floor, block or fixture. Values: `cell`.",
    },
];

/// The setting below whose push speed a target does not move: metres a
/// second (07 §7).
pub const STAGGER_AT_SETTING: &str = "combat.stagger_at";
/// The default of [`STAGGER_AT_SETTING`].
pub const DEFAULT_STAGGER_AT: f64 = 0.5;
/// The setting at whose push speed a target is launched: metres a
/// second (07 §7).
pub const LAUNCH_AT_SETTING: &str = "combat.launch_at";
/// The default of [`LAUNCH_AT_SETTING`].
pub const DEFAULT_LAUNCH_AT: f64 = 3.0;

/// The settings combat reads, for the generated reference.
pub const COMBAT_SETTINGS: &[(&str, &str)] = &[
    (
        STAGGER_AT_SETTING,
        "Metres a second: a push slower than this moves nothing. Default 0.5.",
    ),
    (
        LAUNCH_AT_SETTING,
        "Metres a second: a push at least this fast launches its target as a body in flight; between the two it staggers it. Default 3.",
    ),
];
