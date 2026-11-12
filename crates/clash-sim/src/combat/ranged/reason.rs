//! Why ranged combat declines (14 §5), and the settings it reads
//! (07 §16).

use crate::reason::ReasonInfo;

/// A store holds no round to give up next.
pub const EMPTY: &str = "ranged.empty";
/// A shot names no projectile and has no store to take a round from.
pub const NO_PROJECTILE: &str = "ranged.no_projectile";
/// A shot names neither a speed nor an energy, and has no round.
pub const NO_ENERGY: &str = "ranged.no_energy";
/// A number a shot needs is one it cannot use.
pub const BAD_NUMBER: &str = "ranged.bad_number";
/// A shot is aimed nowhere it can go.
pub const NO_AIM: &str = "ranged.no_aim";
/// The shooter or thrower is not standing in the world.
pub const NOT_IN_WORLD: &str = "ranged.not_in_world";
/// A shot asks for no shots, or more than `shoot.max_count`.
pub const TOO_MANY: &str = "ranged.too_many";
/// What is to be fired from or loaded is not a store and has none
/// fitted.
pub const NOT_A_STORE: &str = "ranged.not_a_store";
/// There are no rounds to load.
pub const NOTHING_TO_LOAD: &str = "ranged.nothing_to_load";
/// An item cannot be fitted to another.
pub const CANNOT_FIT: &str = "ranged.cannot_fit";
/// A thing to be thrown is not held by the thrower.
pub const NOT_HELD: &str = "ranged.not_held";
/// A thing is to be thrown striking with a surface it does not have.
pub const NO_SURFACE: &str = "ranged.no_surface";
/// A verb of ranged combat was given an argument of the wrong kind.
pub const BAD_ARGUMENT: &str = "ranged.bad_argument";

/// Every reason ranged combat gives, for the generated reference.
pub const RANGED_REASONS: &[ReasonInfo] = &[
    ReasonInfo {
        code: EMPTY,
        doc: "The store a shot fires from holds no round to give up next: it is empty, or what it would give up next is not a round. Values: `store`.",
    },
    ReasonInfo {
        code: NO_PROJECTILE,
        doc: "A shot names no projectile that is loaded and has no store to take a round from, or its round's projectile is not loaded.",
    },
    ReasonInfo {
        code: NO_ENERGY,
        doc: "A shot or a throw names neither a speed nor an energy, and has no round to take an energy from.",
    },
    ReasonInfo {
        code: BAD_NUMBER,
        doc: "A number a shot or a throw needs came out as one it cannot use: an energy, a spread, a noise or a flash below zero or not finite, a speed not above zero, a thing thrown with no mass. Values: `what`, `value`.",
    },
    ReasonInfo {
        code: NO_AIM,
        doc: "A shot is aimed at nothing it can go toward: neither `at` nor `dir`, a target not in the world, a place in another grid, or the point it leaves from. Values: `at`.",
    },
    ReasonInfo {
        code: NOT_IN_WORLD,
        doc: "Whoever shoots or throws is not standing in the world. Values: `entity`.",
    },
    ReasonInfo {
        code: TOO_MANY,
        doc: "A shot asks for no shots, or for more than the setting `shoot.max_count`. Values: `count`, `limit`.",
    },
    ReasonInfo {
        code: NOT_A_STORE,
        doc: "What a shot fires from, or rounds are loaded into, is not a store (a container whose `holds` has an `order`) and has none fitted. Values: `store`.",
    },
    ReasonInfo {
        code: NOTHING_TO_LOAD,
        doc: "There are no rounds the store takes where `load` looked for them. Values: `store`.",
    },
    ReasonInfo {
        code: CANNOT_FIT,
        doc: "An item cannot be fitted to another: none of its tags is among what the other's `shot.fits` lists, or something carrying that tag is fitted already. Values: `item`, `into`.",
    },
    ReasonInfo {
        code: NOT_HELD,
        doc: "A thing to be thrown is not held by whoever is to throw it. Values: `item`, `who`.",
    },
    ReasonInfo {
        code: NO_SURFACE,
        doc: "A thing is to be thrown striking with a surface its definition does not have. Values: `item`, `surface`.",
    },
    ReasonInfo {
        code: BAD_ARGUMENT,
        doc: "A verb of ranged combat was given an argument that is missing or of the wrong kind. Values: `verb`, `arg`.",
    },
];

/// The setting of how close, in metres, a shot must pass to raise
/// `projectile_passed`.
pub const NEAR_MISS_SETTING: &str = "projectile.near_miss";
/// The default of [`NEAR_MISS_SETTING`].
pub const DEFAULT_NEAR_MISS: f64 = 2.0;
/// The setting of the most shots one `shoot` may fire.
pub const MAX_COUNT_SETTING: &str = "shoot.max_count";
/// The default of [`MAX_COUNT_SETTING`].
pub const DEFAULT_MAX_COUNT: i64 = 1000;
/// The setting of how far an instant shot goes when it says nothing, in
/// metres.
pub const INSTANT_RANGE_SETTING: &str = "shoot.instant_range";
/// The default of [`INSTANT_RANGE_SETTING`].
pub const DEFAULT_INSTANT_RANGE: f64 = 1000.0;

/// The settings ranged combat reads, for the generated reference.
pub const RANGED_SETTINGS: &[(&str, &str)] = &[
    (
        NEAR_MISS_SETTING,
        "Metres: a shot that passes this close to someone without striking them raises `projectile_passed`. Default 2.",
    ),
    (
        MAX_COUNT_SETTING,
        "The most shots one `shoot` may fire. Default 1000.",
    ),
    (
        INSTANT_RANGE_SETTING,
        "Metres: how far an instant shot goes when it gives no `range`. Default 1000.",
    ),
];
