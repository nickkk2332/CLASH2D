//! Ranged combat (07 §11): projectiles, shots, ammunition, stores and
//! what is fitted to weapons.
//!
//! Nothing here is a gun, a bow or a bullet. A **projectile** is a
//! definition: a mass, a cross-section and a striking surface. A shot
//! hands one to the world as a body in flight (02 §15) with a
//! `Projectile` owner, or casts it as a ray that strikes at once. What
//! it meets is the world's to say; what it does to a creature is the
//! same delivery as a strike's (07 §6): worn layers, the body's layers,
//! and what exits flies on. A **round** is an item that names a
//! projectile and an energy; a **store** is a container that gives up
//! what is in it in the order it was put in; a weapon is an item with a
//! `shot` block, and what is fitted to it multiplies its numbers.
//!
//! | Part | Is | Module |
//! |------|----|--------|
//! | [`ProjectileDef`], [`RoundDef`], [`ShotDef`] | The `projectile` type, and the `round` and `shot` blocks of items | [`defs`] |
//! | [`shoot()`], [`ShotSpec`], [`throw()`], [`ThrowSpec`] | Firing and throwing | [`shoot`](mod@shoot) |
//! | [`RangedState`] | The shots in flight | [`state`] |
//! | [`FlightNews`](flight::FlightNews) | What the world says of a shot in flight, and what the simulation does | [`flight`] |
//! | Instant shots | A ray that strikes at once | [`ray`] |
//! | [`load`](store::load), [`fit`](store::fit), [`feed_of`](store::feed_of) | Stores, rounds and what is fitted | [`store`] |
//! | [`ballistic_dir`](aim::ballistic_dir), [`band_point`](aim::band_point) | Aiming helpers | [`aim`] |
//! | [`verbs`], [`properties`], [`functions`], [`RANGED_EVENTS`], [`RANGED_REASONS`], [`RANGED_SETTINGS`] | What rules call, read and listen for | here, [`events`], [`reason`] |
//!
//! # Seams
//!
//! - **The `shot` window** (07 §11.2). `shoot` takes `window`, and the
//!   window would open as the shot is fired for whoever it is aimed at
//!   and close as it arrives. Until reaction windows are built (03 §7,
//!   package T5) it is offered to nobody, as an action phase's window
//!   is: nobody steps out of a bullet's way yet.
//! - **Guidance and fuses** (07 §11.1) are read and carried. A timed
//!   charge is an item thrown with a rule's `after` (05 §13).
//! - **`lead_point`** (07 §11.4) waits for believed velocities.

pub mod aim;
pub mod defs;
pub mod events;
pub(crate) mod flight;
pub(crate) mod ray;
pub mod reason;
pub mod shoot;
pub mod state;
pub mod store;

use clash_core::{EntityId, Real};
use clash_data::Def;
use clash_expr::{Registry, Value};
use clash_world::{CellPos, Location};

pub use defs::{ProjectileDef, ProjectileId, RoundDef, ShotDef};
pub use events::RANGED_EVENTS;
pub use reason::{RANGED_REASONS, RANGED_SETTINGS};
pub use shoot::{Aim, Fired, SHOT_STREAM, ShotSpec, ThrowSpec, shoot, throw};
pub use state::{RangedState, Shot};

use crate::Sim;
use crate::body::props::part_of_handle;
use crate::event::Subject;
use crate::reason::Reason;
use crate::registry::{FunctionDef, PropertyDef};
use crate::rule::VerbCall;
use crate::verb::VerbDef;
use crate::world_verbs::{arg, spec};

/// The heading ranged combat's functions are listed under in the
/// reference.
const GROUP_RANGED: &str = "Ranged combat";

/// Registers the `projectile` definition type with the loader. Called
/// from combat's `register_defs`.
pub fn register_defs(types: &mut clash_data::DefTypes) {
    types.register::<ProjectileDef>();
}

/// Enters the `projectile` definition type in an expression registry,
/// unless it is there: `shoot` and `round` name it.
pub fn register_types(registry: &mut Registry) {
    if registry.def_type_id(ProjectileDef::TYPE).is_none() {
        let _ = registry.add_def_type(ProjectileDef::TYPE, ProjectileDef::DOC);
    }
}

/// A setting that is a number, if it is set to one.
pub(crate) fn setting_real(sim: &Sim, key: &str) -> Option<Real> {
    match sim.settings().get(key)? {
        clash_core::Value::Real(real) => Some(*real),
        clash_core::Value::Int(int) => Some(Real::from_i64(*int)),
        _ => None,
    }
}

// ---- Arguments ---------------------------------------------------------------

fn bad_argument(verb: &str, name: &str, wanted: &str) -> Reason {
    Reason::new(
        reason::BAD_ARGUMENT,
        format!("`{verb}`: the argument `{name}` must be {wanted}"),
    )
    .with("verb", verb)
    .with("arg", name)
}

/// An optional argument that is a number.
fn number(
    verb: &str,
    call: &VerbCall<'_>,
    index: usize,
    name: &str,
) -> Result<Option<Real>, Reason> {
    match arg(call, index) {
        Value::Null => Ok(None),
        value => value
            .as_number()
            .filter(|number| number.is_finite())
            .map(Some)
            .ok_or_else(|| bad_argument(verb, name, "a finite number")),
    }
}

/// An optional argument that is an entity.
fn entity(
    sim: &Sim,
    verb: &str,
    call: &VerbCall<'_>,
    index: usize,
    name: &str,
) -> Result<Option<EntityId>, Reason> {
    match arg(call, index) {
        Value::Null => Ok(None),
        value => match sim.subject_of(value) {
            Some(Subject::Entity(id)) => Ok(Some(id)),
            _ => Err(bad_argument(verb, name, "an entity")),
        },
    }
}

/// An argument that is an entity, or else the `self` of the run.
fn entity_or_self(
    sim: &Sim,
    verb: &str,
    call: &VerbCall<'_>,
    index: usize,
    name: &str,
) -> Result<EntityId, Reason> {
    if let Some(id) = entity(sim, verb, call, index, name)? {
        return Ok(id);
    }
    call.by
        .subject("self")
        .and_then(|value| sim.subject_of(value))
        .and_then(|subject| subject.entity())
        .ok_or_else(|| bad_argument(verb, name, "an entity, where there is no `self`"))
}

/// A point as data writes it: a list of three numbers.
fn point(value: &Value) -> Option<[Real; 3]> {
    let items = value.as_list()?;
    let parts: Vec<Real> = items.iter().filter_map(Value::as_number).collect();
    match parts.as_slice() {
        [x, y, z] if parts.iter().all(|part| part.is_finite()) => Some([*x, *y, *z]),
        _ => None,
    }
}

/// Where a shot or a throw from `from` is aimed, by its `at`, `dir`,
/// `part` and `height` arguments.
fn aim_of(
    sim: &Sim,
    verb: &str,
    from: EntityId,
    (at, dir, part, height): (&Value, &Value, &Value, Option<Real>),
) -> Result<(Aim, Option<EntityId>), Reason> {
    let part = part
        .as_subject()
        .and_then(|handle| part_of_handle(&sim.components().body, handle));
    if !dir.is_null() {
        let dir = point(dir).ok_or_else(|| bad_argument(verb, "dir", "a list of three numbers"))?;
        return Ok((Aim::Dir(dir), None));
    }
    let no_aim = || {
        Reason::new(
            reason::NO_AIM,
            format!("`{verb}` is aimed at nothing it can go toward"),
        )
        .with("at", format!("{at:?}"))
    };
    match sim.subject_of(at) {
        Some(Subject::Entity(target)) => {
            let part = part
                .filter(|(owner, _)| *owner == target)
                .map(|(_, key)| key);
            let aimed = shoot::aim_at_entity(sim, target, part, height).ok_or_else(no_aim)?;
            Ok((aimed, Some(target)))
        }
        Some(Subject::Loc(loc) | Subject::Element(loc, _)) => {
            let rise = match height {
                Some(height) => height,
                // Level: at the height it leaves from, above the floor of
                // the cell it is aimed at.
                None => {
                    let (from_loc, origin) =
                        aim::shot_origin(sim, from, None).ok_or_else(no_aim)?;
                    origin[2] - aim::cell_point(from_loc, Real::ZERO)[2]
                }
            };
            Ok((Aim::At(loc, aim::cell_point(loc, rise)), None))
        }
        _ => {
            let to = point(at).ok_or_else(no_aim)?;
            let (from_loc, _) = aim::footing(sim, from).ok_or_else(no_aim)?;
            let loc = Location::new(from_loc.grid, CellPos::containing(to));
            Ok((Aim::At(loc, to), None))
        }
    }
}

// ---- The verbs -----------------------------------------------------------------

fn shoot_verb(
    sim: &mut Sim,
    call: &VerbCall<'_>,
    cause: clash_world::Cause,
) -> Result<Value, Reason> {
    const VERB: &str = "shoot";
    let shooter = entity_or_self(sim, VERB, call, 0, "from")?;
    let height = number(VERB, call, 4, "height")?;
    let (aim, target) = aim_of(
        sim,
        VERB,
        shooter,
        (arg(call, 1), arg(call, 2), arg(call, 3), height),
    )?;
    let projectile = match arg(call, 5) {
        Value::Null => None,
        value => {
            let name = crate::world_verbs::def_name_of(sim, value)
                .ok_or_else(|| bad_argument(VERB, "projectile", "a projectile"))?;
            Some(sim.defs().lookup::<ProjectileDef>(&name).ok_or_else(|| {
                Reason::new(
                    reason::NO_PROJECTILE,
                    format!("there is no projectile `{name}`"),
                )
            })?)
        }
    };
    let (instant, speed) = match arg(call, 9) {
        Value::Null => (false, None),
        Value::Str(text) if &**text == "instant" => (true, None),
        value => (
            false,
            Some(
                value
                    .as_number()
                    .filter(|speed| speed.is_finite())
                    .ok_or_else(|| bad_argument(VERB, "speed", "a number of m/s, or `instant`"))?,
            ),
        ),
    };
    let count = match arg(call, 11) {
        Value::Null => 1,
        value => value
            .as_int()
            .and_then(|count| u32::try_from(count).ok())
            .ok_or_else(|| bad_argument(VERB, "count", "a whole number"))?,
    };
    let spec = ShotSpec {
        shooter,
        aim,
        target,
        projectile,
        store: entity(sim, VERB, call, 6, "store")?,
        weapon: entity(sim, VERB, call, 7, "with")?,
        energy: number(VERB, call, 8, "energy")?,
        speed,
        instant,
        spread: number(VERB, call, 10, "spread")?,
        count,
        noise: number(VERB, call, 12, "noise")?,
        flash: number(VERB, call, 13, "flash")?,
        lob: arg(call, 14).as_bool().unwrap_or(false),
        height,
        range: number(VERB, call, 15, "range")?,
    };
    let fired = shoot::shoot(sim, &spec, cause)?;
    Ok(Value::Int(i64::from(fired.projectiles)))
}

fn throw_verb(
    sim: &mut Sim,
    call: &VerbCall<'_>,
    cause: clash_world::Cause,
) -> Result<Value, Reason> {
    const VERB: &str = "throw";
    let thrower = entity_or_self(sim, VERB, call, 0, "who")?;
    let what = entity(sim, VERB, call, 1, "what")?
        .ok_or_else(|| bad_argument(VERB, "what", "an entity"))?;
    let height = number(VERB, call, 5, "height")?;
    let (aim, target) = aim_of(
        sim,
        VERB,
        thrower,
        (arg(call, 2), arg(call, 3), arg(call, 4), height),
    )?;
    let spec = ThrowSpec {
        thrower,
        what,
        aim,
        target,
        energy: number(VERB, call, 6, "energy")?,
        speed: number(VERB, call, 7, "speed")?,
        spread: number(VERB, call, 8, "spread")?.unwrap_or(Real::ZERO),
        surface: arg(call, 9)
            .as_str()
            .filter(|surface| !surface.is_empty())
            .map(str::to_owned),
        lob: arg(call, 10).as_bool().unwrap_or(false),
    };
    shoot::throw(sim, &spec, cause)?;
    Ok(Value::Null)
}

fn load_verb(
    sim: &mut Sim,
    call: &VerbCall<'_>,
    cause: clash_world::Cause,
) -> Result<Value, Reason> {
    const VERB: &str = "load";
    let who = entity_or_self(sim, VERB, call, 0, "who")?;
    let into = entity(sim, VERB, call, 1, "into")?
        .ok_or_else(|| bad_argument(VERB, "into", "an entity"))?;
    let from = entity(sim, VERB, call, 2, "from")?;
    let count = match arg(call, 3) {
        Value::Null => None,
        value => Some(
            value
                .as_int()
                .and_then(|count| u32::try_from(count).ok())
                .ok_or_else(|| bad_argument(VERB, "count", "a whole number"))?,
        ),
    };
    let loaded = store::load(sim, who, into, from, count, cause)?;
    Ok(Value::Int(i64::from(loaded)))
}

fn fit_verb(
    sim: &mut Sim,
    call: &VerbCall<'_>,
    cause: clash_world::Cause,
) -> Result<Value, Reason> {
    const VERB: &str = "fit";
    let what = entity(sim, VERB, call, 0, "what")?
        .ok_or_else(|| bad_argument(VERB, "what", "an entity"))?;
    let into = entity(sim, VERB, call, 1, "into")?
        .ok_or_else(|| bad_argument(VERB, "into", "an entity"))?;
    let fitted = store::fit(sim, what, into, cause)?;
    Ok(sim.subject_value(Subject::Entity(fitted)))
}

/// The verbs of ranged combat, for [`crate::verb::engine_verbs`].
pub fn verbs() -> Vec<VerbDef> {
    vec![
        VerbDef {
            name: "shoot",
            build: |registry| {
                spec(
                    registry,
                    "shoot",
                    "Fires a shot (07 §11.1): from where `from` stands, at a height, toward `at` or along `dir`, with dispersion of `spread` drawn from the stream `shots`. With a `store`, each of `count` shots takes the store's next round and launches its projectile, as many as the round says, with the round's energy times the weapon's efficiency; with none, `count` projectiles of `projectile`. Each flies as a body in flight (02 §15) and strikes what it meets, or with `speed` `instant` is cast as a ray that strikes at once (07 §11.3). Raises `projectile_fired` for each; spends each round and leaves its casing. Returns how many projectiles were fired. Refused with `ranged.empty`, `ranged.no_projectile`, `ranged.no_energy`, `ranged.no_aim`, `ranged.not_in_world`, `ranged.not_a_store`, `ranged.too_many` and `ranged.bad_number`, having changed nothing.",
                    &[
                        ("from", "entity", true, "Who shoots. Default `self`."),
                        (
                            "at",
                            "any",
                            true,
                            "What it is aimed at: an entity, at the height of `part` or `height`, or the middle of its body; a cell, at `height` above its floor or level; or a point, a list of three numbers in metres. One of `at` and `dir`.",
                        ),
                        (
                            "dir",
                            "any",
                            true,
                            "The direction it is aimed along, east, south and up, as a list of three numbers.",
                        ),
                        (
                            "part",
                            "part",
                            true,
                            "The part of the entity aimed at: the shot is aimed at the middle of its band.",
                        ),
                        (
                            "height",
                            "real",
                            true,
                            "Metres above the footing of the entity, or the floor of the cell, it is aimed at; and above the shooter's footing it leaves from. Default: at a part's band, or the middle of a body; it leaves at 0.8 of the shooter's height.",
                        ),
                        (
                            "projectile",
                            "id<projectile>",
                            true,
                            "What it fires, for a shot with no store, or in place of what the store's rounds name.",
                        ),
                        (
                            "store",
                            "entity",
                            true,
                            "The store its rounds come from: a magazine, a quiver, or a weapon with one fitted.",
                        ),
                        (
                            "with",
                            "entity",
                            true,
                            "The weapon it is fired with: its `shot` numbers, and those of what is fitted to it, give the spread, efficiency, noise and flash the effect leaves out.",
                        ),
                        (
                            "energy",
                            "real",
                            true,
                            "Joules each projectile leaves with, in place of the round's times the efficiency.",
                        ),
                        (
                            "speed",
                            "any",
                            true,
                            "Metres a second each projectile leaves at, in place of what its energy gives; or `instant`, for a ray.",
                        ),
                        (
                            "spread",
                            "real",
                            true,
                            "Radians: the standard deviation of each of its two angles of dispersion. Default the weapon's, else none.",
                        ),
                        (
                            "count",
                            "int",
                            true,
                            "How many shots: rounds from the store, or projectiles. Default 1; at most `shoot.max_count`.",
                        ),
                        (
                            "noise",
                            "real",
                            true,
                            "How loud, for `projectile_fired`. Default the weapon's, else 0.",
                        ),
                        (
                            "flash",
                            "real",
                            true,
                            "How bright, for `projectile_fired`. Default the weapon's, else 0.",
                        ),
                        (
                            "lob",
                            "bool",
                            true,
                            "Whether it is sent along the lower arc that comes down on the aim point under gravity, not straight at it. Default false.",
                        ),
                        (
                            "range",
                            "real",
                            true,
                            "For an instant shot: how far the ray goes, in metres. Default `shoot.instant_range`.",
                        ),
                        (
                            "window",
                            "any",
                            true,
                            "The kind of reaction window it opens for whoever it is aimed at (07 §11.2). Offered to nobody until reaction windows are built.",
                        ),
                    ],
                )
            },
            run: shoot_verb,
        },
        VerbDef {
            name: "throw",
            build: |registry| {
                spec(
                    registry,
                    "throw",
                    "Throws an item `who` holds (07 §11.1): it leaves the hand and flies as a body that is the item, with its own mass and cross-section, from where `who` stands toward `at` or along `dir`, and strikes what it meets with its surface named `surface`, through the same delivery as a projectile's; with none, with a blunt blow. Raises `thrown`. Refused with `ranged.not_held`, `ranged.no_surface`, `ranged.no_energy`, `ranged.no_aim`, `ranged.not_in_world` and `ranged.bad_number`.",
                    &[
                        ("who", "entity", true, "Who throws. Default `self`."),
                        ("what", "entity", false, "The item thrown: one `who` holds."),
                        ("at", "any", true, "What it is aimed at, as for `shoot`."),
                        (
                            "dir",
                            "any",
                            true,
                            "The direction it is aimed along, as a list of three numbers.",
                        ),
                        ("part", "part", true, "The part of the entity aimed at."),
                        (
                            "height",
                            "real",
                            true,
                            "Metres above the footing of the entity, or the floor of the cell, it is aimed at.",
                        ),
                        ("energy", "real", true, "Joules it leaves the hand with."),
                        (
                            "speed",
                            "real",
                            true,
                            "Metres a second it leaves the hand at, in place of what its energy gives.",
                        ),
                        (
                            "spread",
                            "real",
                            true,
                            "Radians of dispersion, as for `shoot`. Default none.",
                        ),
                        (
                            "surface",
                            "string",
                            true,
                            "The id of the item's surface it strikes with. Default none: a blunt blow.",
                        ),
                        (
                            "lob",
                            "bool",
                            true,
                            "Whether it is sent along the arc that comes down on the aim point. Default false.",
                        ),
                    ],
                )
            },
            run: throw_verb,
        },
        VerbDef {
            name: "load",
            build: |registry| {
                spec(
                    registry,
                    "load",
                    "Loads rounds into a store (07 §11.5), one at a time and in that order: from `from`, a container or a stack of rounds; with none, from whatever `who` carries, at any depth. Only rounds the store takes, and as many as fit, at most `count`. `into` is the store, or a weapon with one fitted. Returns how many were loaded. Refused with `ranged.not_a_store`, and with `ranged.nothing_to_load` or the first refusal of `put_in` when none was.",
                    &[
                        (
                            "who",
                            "entity",
                            true,
                            "Who loads, whose carried rounds are taken. Default `self`.",
                        ),
                        (
                            "into",
                            "entity",
                            false,
                            "The store, or a weapon whose store it fills.",
                        ),
                        (
                            "from",
                            "entity",
                            true,
                            "A container to take rounds from, or a stack of rounds.",
                        ),
                        (
                            "count",
                            "int",
                            true,
                            "The most to load. Default as many as fit.",
                        ),
                    ],
                )
            },
            run: load_verb,
        },
        VerbDef {
            name: "fit",
            build: |registry| {
                spec(
                    registry,
                    "fit",
                    "Fits an item to another (07 §11.5): a magazine, a sight, a muzzle. It is held `fixed` in it, and gives a shot through it its `shot` numbers while it is; taking it into a grasp takes it off. Returns the item. Refused with `ranged.cannot_fit`: none of its tags is in the other's `shot.fits`, or one carrying it is fitted already.",
                    &[
                        ("what", "entity", false, "The item fitted."),
                        ("into", "entity", false, "The item it is fitted to."),
                    ],
                )
            },
            run: fit_verb,
        },
    ]
}

// ---- Properties ---------------------------------------------------------------

fn entity_of(sim: &Sim, subject: Subject) -> Option<EntityId> {
    subject
        .entity()
        .filter(|id| sim.truth().entity(*id).is_some())
}

fn entity_value(sim: &Sim, id: Option<EntityId>) -> Value {
    id.map_or(Value::Null, |id| sim.subject_value(Subject::Entity(id)))
}

/// The properties of ranged combat, all of the kind `entity`.
pub fn properties() -> Vec<PropertyDef> {
    let p = PropertyDef::new;
    vec![
        p(
            "entity",
            "shot",
            "real?",
            "One of the numbers a shot through an item takes (07 §11.5): `spread`, `efficiency`, `noise`, `flash` or `energy`, the product of the item's own `shot` number and those of everything fitted to it that states it. Null where none states it.",
            |sim, subject, args| {
                let name = args.first().and_then(Value::as_str).unwrap_or_default();
                entity_of(sim, subject)
                    .and_then(|id| store::shot_number(sim, id, name))
                    .map_or(Value::Null, Value::Real)
            },
        )
        .arg("name", "string", "`spread`, `efficiency`, `noise`, `flash` or `energy`.")
        .changed_by(&["item_taken", "item_dropped", "entity_relocated"]),
        p(
            "entity",
            "feed",
            "entity?",
            "The store an item fires from (07 §11.5): itself if it is a store, else the first store fitted to it. Null for neither.",
            |sim, subject, _| {
                entity_value(sim, entity_of(sim, subject).and_then(|id| store::feed_of(sim, id)))
            },
        )
        .own()
        .changed_by(&["entity_relocated"]),
        p(
            "entity",
            "rounds",
            "int",
            "How many rounds an item can fire before it is empty (07 §11.5): the members of the rounds in its feed. Zero for one with none.",
            |sim, subject, _| {
                let count = entity_of(sim, subject)
                    .and_then(|id| store::feed_of(sim, id))
                    .map_or(0, |feed| store::rounds_in(sim, feed));
                Value::Int(i64::from(count))
            },
        )
        .changed_by(&["item_taken", "item_dropped", "item_merged", "item_split", "entity_relocated", "entity_removed"]),
        p(
            "entity",
            "next_round",
            "entity?",
            "The round an item's feed gives up next (07 §11.5). Null for an empty one.",
            |sim, subject, _| {
                entity_value(
                    sim,
                    entity_of(sim, subject)
                        .and_then(|id| store::feed_of(sim, id))
                        .and_then(|feed| store::next_round(sim, feed)),
                )
            },
        )
        .own()
        .changed_by(&["item_taken", "item_dropped", "entity_relocated", "entity_removed"]),
        p(
            "entity",
            "store_of",
            "entity?",
            "The first store the entity has that takes things carrying a tag (07 §11.5): itself, then what it holds in any way but enclosed, in order of id, and what is fitted to those. A quiver worn, a magazine fitted to a rifle held. Null for none.",
            |sim, subject, args| {
                let tag = args.first().and_then(Value::as_str).unwrap_or_default();
                entity_value(sim, entity_of(sim, subject).and_then(|id| store::store_of(sim, id, tag)))
            },
        )
        .arg("tag", "string", "A tag the store takes.")
        .own()
        .changed_by(&["item_taken", "item_dropped", "item_worn", "item_removed", "entity_relocated"]),
        p(
            "entity",
            "wielding",
            "entity?",
            "The first thing the entity holds in a grasp, in order of id, that carries a tag; any thing, for an empty tag. Null for none.",
            |sim, subject, args| {
                let tag = args.first().and_then(Value::as_str).unwrap_or_default();
                entity_value(sim, entity_of(sim, subject).and_then(|id| store::wielding(sim, id, tag)))
            },
        )
        .arg("tag", "string", "The tag; empty for any.")
        .own()
        .changed_by(&["item_taken", "item_dropped", "entity_relocated"]),
    ]
}

// ---- Functions -------------------------------------------------------------------

/// Where a value of `ballistic_dir` is, as a cell and a point in its
/// grid: an entity at the height a shot leaves it from, a cell at its
/// floor, or a list of three numbers.
fn place_of(sim: &Sim, value: &Value, anchor: Option<Location>) -> Option<(Location, [Real; 3])> {
    match sim.subject_of(value) {
        Some(Subject::Entity(id)) => aim::shot_origin(sim, id, None),
        Some(Subject::Loc(loc) | Subject::Element(loc, _)) => {
            Some((loc, aim::cell_point(loc, Real::ZERO)))
        }
        _ => {
            let to = point(value)?;
            let grid = anchor?.grid;
            Some((Location::new(grid, CellPos::containing(to)), to))
        }
    }
}

fn ballistic_dir_fn(sim: &Sim, args: &[Value]) -> Value {
    let (Some(from), Some(to), Some(speed)) = (
        args.first(),
        args.get(1),
        args.get(2).and_then(Value::as_number),
    ) else {
        return Value::Null;
    };
    let anchor = match (sim.subject_of(from), sim.subject_of(to)) {
        (Some(Subject::Entity(id)), _) | (_, Some(Subject::Entity(id))) => {
            aim::footing(sim, id).map(|(loc, _)| loc)
        }
        (Some(Subject::Loc(loc) | Subject::Element(loc, _)), _)
        | (_, Some(Subject::Loc(loc) | Subject::Element(loc, _))) => Some(loc),
        _ => None,
    };
    let (Some(from), Some(to)) = (place_of(sim, from, anchor), place_of(sim, to, anchor)) else {
        return Value::Null;
    };
    let Some(way) = aim::between(sim, from, to) else {
        return Value::Null;
    };
    let gravity = sim
        .truth()
        .cosmos()
        .ambient(from.0)
        .map_or(Real::ZERO, |ambient| ambient.gravity);
    aim::ballistic_dir(way, speed, gravity).map_or(Value::Null, |dir| {
        Value::list(dir.into_iter().map(Value::Real))
    })
}

fn band_point_fn(sim: &Sim, args: &[Value]) -> Value {
    let Some(Subject::Entity(target)) = args.first().and_then(|value| sim.subject_of(value)) else {
        return Value::Null;
    };
    let part = args
        .get(1)
        .and_then(Value::as_subject)
        .and_then(|handle| part_of_handle(&sim.components().body, handle))
        .filter(|(owner, _)| *owner == target)
        .map(|(_, key)| key);
    aim::band_point(sim, target, part).map_or(Value::Null, |(_, point)| {
        Value::list(point.into_iter().map(Value::Real))
    })
}

/// The functions of ranged combat (07 §11.4).
pub fn functions() -> Vec<FunctionDef> {
    vec![
        FunctionDef::new(
            "ballistic_dir",
            "list<real>?",
            GROUP_RANGED,
            "The direction, east, south and up, of length one, that sends a shot at `speed` from `from` down onto `to` under the gravity where `from` is, along the lower of the two arcs, with no drag (07 §11.4). An entity is taken at the height a shot leaves it from, a cell at its floor, and a list of three numbers as a point in metres. Null where no arc reaches it at that speed.",
            ballistic_dir_fn,
        )
        .arg("from", "any", "Where the shot leaves from.")
        .arg("to", "any", "Where it is to come down.")
        .arg("speed", "real", "Its speed, in metres a second."),
        FunctionDef::new(
            "band_point",
            "list<real>?",
            GROUP_RANGED,
            "The point in metres, east, south and up in its grid, at the middle of the band of a part of an entity where it stands (07 §11.4); for no part, the middle of its body. Reads truth. Null for one not in the world.",
            band_point_fn,
        )
        .arg("target", "entity", "Whose part.")
        .optional(
            "part",
            "any",
            "The part, of the kind `part`; null for the middle of the body.",
        ),
    ]
}
