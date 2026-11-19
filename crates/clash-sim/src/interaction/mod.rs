//! Interactions (16 §3; package G1): how a character acts on the world.
//! Open, pull, load, hack, drink from: each is an `interaction`
//! definition with a reach, a duration, conditions and effects, offered
//! by elements, devices, items and, with `"offered_by": "any"`, by
//! everything of a kind. The engine knows no "open" and no "use": every
//! interaction is content (decision D-100).
//!
//! | Part | Is |
//! |------|----|
//! | [`InteractionDef`], [`Reach`], [`TargetKind`] | The `interaction` definition type |
//! | [`Target`] | What an interaction is done to: an element or an entity |
//! | [`interactions_for`] | What a target offers a character now, each [`Offer`] available or with the reason it is not |
//! | [`default_for`], [`bump_default`] | The interaction used when a thing is bumped into or clicked without a choice |
//! | [`offers_near`] | Every offer within a step of a character, for a context menu |
//! | [`begin`], [`complete`], [`interact`] | Doing one: the checks and the hook before, the effects after |
//! | [`properties`], [`verbs()`], [`event_types`], [`INTERACTION_REASONS`] | What rules read, call, listen for and are refused with |
//!
//! # Doing an interaction
//!
//! An interaction is done by an action (03 §6). The bundled one,
//! `core:interact`, takes the parameters `target` and `interaction`.
//! Any action whose parameters are named so is an interaction's
//! action: when it starts, the simulation's handler of `action_starting`
//! calls [`begin`], which tests reach (raising the hook
//! `interaction_reach`), tests `if` against truth and raises the hook
//! `interaction_starting`; a refusal by any of them refuses the action,
//! and the player is told why as for any action (`action_refused`).
//! When the action's phase ends its effect `complete_interaction` calls
//! [`complete`], which tests reach and `if` again, runs the
//! interaction's `do` through the rule runtime and raises `interacted`.
//! A device or a rule that acts at once calls the verb `interact`, which
//! is [`begin`] and [`complete`] together.
//!
//! # Belief
//!
//! What a character is offered is worked out from what it believes
//! (05 §7.3): `shown_if` compiles in its mind context, and reach and
//! `if` are read through the [`belief`] seam. Until perception supplies
//! belief that seam reads truth, as movement's does (D-574); what is
//! attempted is tested against truth whatever the character believed.
//!
//! # Not built here
//!
//! The channels of an interaction (`uses`) are claimed by the action
//! that does it, and the bundled action claims none of its own; `tools`
//! wait for package G6; creatures offer interactions through
//! `offered_by: any` until templates carry `interactions`; networks are
//! stubbed (see [`Reach::Network`]).

pub mod belief;
mod defs;

use std::sync::Arc;

use clash_core::{EntityId, Real};
use clash_data::{Def, DefTypes, Defs};
use clash_expr::{EventSpec, Registry, Type, Value};
use clash_world::defs::EngineFlag;
use clash_world::{Cause, CellFlags, CellPos, Layer, Location, OccupantFilter, distinct_ids};

use crate::Sim;
use crate::bus::{EventPayload, EventType, FieldSpec, FieldType, FieldValue, Handler};
use crate::device::{self, Device, DeviceDef};
use crate::entity::Place;
use crate::event::{CAUSE_SUBJECTS, Subject, Where};
use crate::host::Bound;
use crate::item::Item;
use crate::reason::{Reason, ReasonInfo};
use crate::registry::PropertyDef;
use crate::verb::VerbDef;
use crate::world_verbs::{ArgRow, arg, entity_arg, spec};

pub use defs::{
    HERE_SUBJECT, InteractionDef, InteractionId, OfferedBy, Reach, TARGET_SUBJECT, TargetKind,
    USER_SUBJECT, check_element_interactions,
};

/// Registers the `interaction` definition type and the check that every
/// element's `interactions` names one. Called from the crate's
/// `register_defs`.
pub fn register_defs(types: &mut DefTypes) {
    types
        .register::<InteractionDef>()
        .add_cross_check(check_element_interactions);
}

/// Enters the `interaction` definition type into an expression registry
/// if it lacks it. Called when the engine's registry is built, before
/// its properties.
pub fn register_types(registry: &mut Registry) {
    if registry.def_type_id(InteractionDef::TYPE).is_none() {
        let _ = registry.add_def_type(InteractionDef::TYPE, InteractionDef::DOC);
    }
}

// ---- Targets -------------------------------------------------------------------

/// What an interaction is done to (16 §3.2).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Target {
    /// The floor, block or fixture of a cell.
    Element(Location, Layer),
    /// An entity: a device, an item, a creature.
    Entity(EntityId),
}

impl Target {
    /// The target as a subject.
    pub const fn subject(self) -> Subject {
        match self {
            Target::Element(loc, layer) => Subject::Element(loc, layer),
            Target::Entity(id) => Subject::Entity(id),
        }
    }

    /// A subject as a target, if it is an element or an entity.
    pub const fn of(subject: Subject) -> Option<Target> {
        match subject {
            Subject::Element(loc, layer) => Some(Target::Element(loc, layer)),
            Subject::Entity(id) => Some(Target::Entity(id)),
            _ => None,
        }
    }

    /// The kind of target it is.
    pub const fn kind(self) -> TargetKind {
        match self {
            Target::Element(..) => TargetKind::Element,
            Target::Entity(_) => TargetKind::Entity,
        }
    }
}

/// One interaction a target offers a character (16 §3.2).
#[derive(Clone, PartialEq, Debug)]
pub struct Offer {
    /// The interaction.
    pub interaction: InteractionId,
    /// Its qualified id.
    pub id: String,
    /// What a menu calls it: its `name`, or its id.
    pub name: String,
    /// Whether it is the target's default.
    pub default: bool,
    /// Why it cannot be done now, or `None` if it can.
    pub refusal: Option<Reason>,
}

impl Offer {
    /// Whether it can be done now, as far as the character believes.
    pub fn available(&self) -> bool {
        self.refusal.is_none()
    }
}

// ---- Reasons -------------------------------------------------------------------

/// The stable codes of the reasons interactions refuse for. Each is
/// listed in [`INTERACTION_REASONS`].
pub mod codes {
    /// The id is not an interaction. Values: `interaction`.
    pub const UNKNOWN: &str = "interaction.unknown";
    /// The target does not offer it. Values: `interaction`.
    pub const NOT_OFFERED: &str = "interaction.not_offered";
    /// The user cannot reach the target. Values: `interaction`, `reach`.
    pub const OUT_OF_REACH: &str = "interaction.out_of_reach";
    /// Its `if` does not hold. Values: `interaction`.
    pub const NOT_NOW: &str = "interaction.not_now";
    /// The target is gone or not loaded. Values: `interaction`.
    pub const NO_TARGET: &str = "interaction.no_target";
    /// A verb's argument is missing or of the wrong kind. Values:
    /// `verb`, `arg`.
    pub const BAD_ARGUMENT: &str = "interaction.bad_argument";
    /// The definitions were loaded without the `interaction` type.
    pub const NOT_REGISTERED: &str = "interaction.not_registered";
}

/// The reasons interactions refuse for, for the generated reference.
pub const INTERACTION_REASONS: &[ReasonInfo] = &[
    ReasonInfo {
        code: codes::UNKNOWN,
        doc: "The id is not an `interaction` definition. Values: `interaction`.",
    },
    ReasonInfo {
        code: codes::NOT_OFFERED,
        doc: "The target does not offer the interaction: its definition does not list it, it is not `offered_by: any`, or it is done to the other kind of target. Values: `interaction`.",
    },
    ReasonInfo {
        code: codes::OUT_OF_REACH,
        doc: "The user cannot reach the target by the interaction's `reach`, and no handler of `interaction_reach` let it. Values: `interaction`, `reach`.",
    },
    ReasonInfo {
        code: codes::NOT_NOW,
        doc: "The interaction's `if` does not hold; the message is its `fail_text`. Values: `interaction`.",
    },
    ReasonInfo {
        code: codes::NO_TARGET,
        doc: "The target no longer exists, or its cell is not loaded. Values: `interaction`.",
    },
    ReasonInfo {
        code: codes::BAD_ARGUMENT,
        doc: "A verb of interactions was given an argument that is missing or of the wrong kind. Values: `verb`, `arg`.",
    },
    ReasonInfo {
        code: codes::NOT_REGISTERED,
        doc: "The definitions were loaded without the `interaction` definition type. Values: none.",
    },
];

// ---- Definitions -----------------------------------------------------------------

/// The interaction definitions, if they were loaded.
fn registry(defs: &Defs) -> Option<&clash_data::Registry<InteractionDef>> {
    defs.try_registry::<InteractionDef>()
}

/// An interaction's definition.
pub fn def(sim: &Sim, id: InteractionId) -> Option<&InteractionDef> {
    registry(&sim.defs)?.try_get(id)
}

/// The interaction a qualified id names.
pub fn lookup(sim: &Sim, name: &str) -> Option<InteractionId> {
    registry(&sim.defs)?.lookup(name)
}

/// The qualified id of an interaction.
pub fn name_of(sim: &Sim, id: InteractionId) -> String {
    registry(&sim.defs).map_or_else(String::new, |registry| registry.name(id).to_owned())
}

/// The namespace of a qualified id.
fn namespace_of(qualified: &str) -> Option<&str> {
    qualified.split_once(':').map(|(namespace, _)| namespace)
}

/// The interaction a reference written in a definition of `owner`
/// names: as written if qualified, else in the owner's namespace.
fn resolve(sim: &Sim, owner: &str, written: &str) -> Option<InteractionId> {
    let registry = registry(&sim.defs)?;
    if written.contains(':') {
        return registry.lookup(written);
    }
    namespace_of(owner)
        .and_then(|namespace| registry.lookup(&format!("{namespace}:{written}")))
        .or_else(|| registry.lookup(written))
}

/// What a target's definitions list, in the order written: an element's
/// `interactions`, a device's and an item's.
fn listed(sim: &Sim, target: Target) -> Vec<InteractionId> {
    let mut found = Vec::new();
    match target {
        Target::Element(loc, layer) => {
            let Some(state) = belief::element(sim, loc, layer) else {
                return found;
            };
            let world = sim.cosmos.defs();
            let Some(element) = world.element(layer, state) else {
                return found;
            };
            let own = world.state_text(layer, state);
            found.extend(
                element
                    .interactions
                    .iter()
                    .filter_map(|written| resolve(sim, &own, written)),
            );
        }
        Target::Entity(id) => {
            if let Some(device) = device::def_of(sim, id) {
                found.extend(device.interactions.iter().map(|reference| reference.id()));
            }
            if let Some(item) = sim
                .components
                .get::<Item>(id)
                .and_then(|item| crate::item::def_of(&sim.defs, item))
            {
                found.extend(item.interactions.iter().map(|reference| reference.id()));
            }
        }
    }
    found
}

/// Every interaction a target offers, in order: what its definitions
/// list, then those offered by `any` of its kind, by id; each once.
fn offered(sim: &Sim, target: Target) -> Vec<InteractionId> {
    let mut found = listed(sim, target);
    if let Some(registry) = registry(&sim.defs) {
        for (id, def) in registry.iter() {
            if def.offered_by == Some(OfferedBy::Any) && def.target == target.kind() {
                found.push(id);
            }
        }
    }
    let mut seen = Vec::with_capacity(found.len());
    found.retain(|id| {
        if seen.contains(id) {
            false
        } else {
            seen.push(*id);
            true
        }
    });
    found
}

// ---- Reach -----------------------------------------------------------------------

/// Where a target is: its cell and its height above the cell's base.
fn place_of(sim: &Sim, target: Target) -> Option<(Location, u16)> {
    crate::world_verbs::position_of(sim, target.subject())
}

/// The cell of a target, for `here`.
fn here_of(sim: &Sim, target: Target) -> Option<Location> {
    place_of(sim, target).map(|(loc, _)| loc)
}

/// Whether a cell has stairs, a ramp or a ladder in it: a way between
/// storeys.
fn climbs(sim: &Sim, loc: Location) -> bool {
    let world = sim.cosmos.defs();
    [Layer::Block, Layer::Fixture].into_iter().any(|layer| {
        belief::element(sim, loc, layer).is_some_and(|state| {
            let flags = world.flags(layer, state);
            flags.has(EngineFlag::Stairs)
                || flags.has(EngineFlag::Ramp)
                || flags.has(EngineFlag::Ladder)
        })
    })
}

/// Whether two cells are joined for reaching: the same cell, one of the
/// eight round it on its storey, or straight above or below with no
/// floor between or a way up.
fn adjacent(sim: &Sim, from: Location, to: Location) -> bool {
    let Some(delta) = sim.cosmos.delta(from, to) else {
        return false;
    };
    if delta.z == 0 {
        return delta.x.abs() <= 1 && delta.y.abs() <= 1;
    }
    if delta.x != 0 || delta.y != 0 || delta.z.abs() != 1 {
        return false;
    }
    let (lower, upper) = if delta.z > 0 { (from, to) } else { (to, from) };
    let open = sim
        .cosmos
        .flags(upper)
        .is_ok_and(|flags| !flags.contains(CellFlags::HAS_FLOOR));
    open || climbs(sim, lower) || climbs(sim, upper)
}

/// Whether `user` holds `what` at any depth.
fn holds(sim: &Sim, user: EntityId, what: EntityId) -> bool {
    let mut at = what;
    for _ in 0..=crate::entity::MAX_CONTAINMENT_DEPTH {
        match sim.entities.get(at).map(crate::entity::KernelEntity::place) {
            Some(Place::In(container)) if container == user => return true,
            Some(Place::In(container)) => at = container,
            _ => return false,
        }
    }
    false
}

/// The devices a user operates for `network` reach, as definitions:
/// those it holds; device entities within a step of it; and the devices
/// the elements within a step of it name, promoted or not, so that a
/// console does not have to become an entity to be used.
fn terminals(sim: &Sim, user: EntityId) -> Vec<&DeviceDef> {
    let mut found: Vec<&DeviceDef> = sim
        .entities
        .contents_deep(user)
        .into_iter()
        .filter_map(|id| device::def_of(sim, id))
        .collect();
    let Some((loc, _)) = place_of(sim, Target::Entity(user)) else {
        return found;
    };
    let filter = OccupantFilter::ANY.of_kinds(clash_world::OccupantKinds::of(&[
        clash_world::OccupantKind::Device,
    ]));
    if let Ok(near) = sim.cosmos.occupants_within(loc, Real::of(1.5), &filter) {
        found.extend(
            distinct_ids(&near)
                .into_iter()
                .filter(|id| {
                    place_of(sim, Target::Entity(*id))
                        .is_some_and(|(there, _)| adjacent(sim, loc, there))
                })
                .filter_map(|id| device::def_of(sim, id)),
        );
    }
    for dy in -1..=1 {
        for dx in -1..=1 {
            let cell = CellPos::new(loc.cell.x + dx, loc.cell.y + dy, loc.cell.z);
            let Ok(there) = sim.cosmos.normalize(Location::new(loc.grid, cell)) else {
                continue;
            };
            for layer in Layer::ALL {
                if let Some(def) = target_device(sim, Target::Element(there, layer)) {
                    found.push(def);
                }
            }
        }
    }
    found
}

/// The tags that name a network on a device's definition: those that
/// begin with `network:`.
fn networks(def: &DeviceDef) -> impl Iterator<Item = &str> {
    def.tags
        .iter()
        .map(String::as_str)
        .filter(|tag| tag.starts_with(NETWORK_TAG_PREFIX))
}

/// The prefix of a device tag that names a network, for the stub of
/// `network` reach.
pub const NETWORK_TAG_PREFIX: &str = "network:";

/// The device definition behind a target: a device entity's, or the
/// one its element names.
fn target_device(sim: &Sim, target: Target) -> Option<&DeviceDef> {
    match target {
        Target::Entity(id) => device::def_of(sim, id),
        Target::Element(loc, layer) => {
            let state = belief::element(sim, loc, layer)?;
            let id = device::element_device(sim, layer, state).ok().flatten()?;
            sim.defs.try_registry::<DeviceDef>()?.try_get(id)
        }
    }
}

/// Whether a user reaches a target by a reach, as it stands, without the
/// hook (16 §3.3).
///
/// `network` is a stub until networks are built (G3, G4): the user
/// operates a device (one it holds, a device entity within a step, or
/// one an element within a step names) whose definition carries a tag
/// beginning `network:` that the target's device carries too. The
/// target itself is reached through the network wherever it is.
pub fn reaches(sim: &Sim, user: EntityId, target: Target, reach: Reach) -> bool {
    if let (Reach::Held, Target::Entity(what)) = (reach, target) {
        return holds(sim, user, what);
    }
    let (Some((from, from_alt)), Some((to, to_alt))) =
        (place_of(sim, Target::Entity(user)), place_of(sim, target))
    else {
        return false;
    };
    match reach {
        Reach::Held => false,
        Reach::SameCell => sim
            .cosmos
            .delta(from, to)
            .is_some_and(|delta| delta == clash_core::IVec3::ZERO),
        Reach::Adjacent => adjacent(sim, from, to),
        Reach::Range(range) => {
            let near = sim
                .cosmos
                .delta(from, to)
                .is_some_and(|delta| clash_world::distance_m(delta) <= range);
            near && sim
                .cosmos
                .line_of_sight_between(from, from_alt, to, to_alt)
                .unwrap_or(false)
        }
        Reach::Network => {
            let Some(wanted) = target_device(sim, target) else {
                return false;
            };
            terminals(sim, user).into_iter().any(|have| {
                networks(have).any(|network| networks(wanted).any(|other| other == network))
            })
        }
    }
}

// ---- Conditions ------------------------------------------------------------------

/// What an interaction's expressions are given: the user, the target and
/// its cell, as values in slot order.
fn bound(sim: &Sim, user: EntityId, target: Target) -> Vec<Value> {
    let here = here_of(sim, target).map_or(Value::Null, |loc| sim.subject_value(Subject::Loc(loc)));
    vec![
        sim.subject_value(Subject::Entity(user)),
        sim.subject_value(target.subject()),
        here,
    ]
}

fn bound_of(values: &[Value]) -> Bound {
    values
        .iter()
        .fold(Bound::new(), |bound, value| bound.subject(value.clone()))
}

/// The refusal for an `if` that does not hold, with the interaction's
/// `fail_text`.
fn not_now(sim: &Sim, id: InteractionId, def: &InteractionDef, values: &[Value]) -> Reason {
    let name = name_of(sim, id);
    let text = def
        .fail_text
        .as_ref()
        .map(|text| {
            let mut host = sim.host(bound_of(values));
            text.render_on(&sim.defs, &mut host)
        })
        .filter(|text| !text.is_empty())
        .unwrap_or_else(|| format!("`{name}` cannot be done now"));
    Reason::new(codes::NOT_NOW, text).with("interaction", name)
}

fn out_of_reach(sim: &Sim, id: InteractionId, reach: Reach) -> Reason {
    let name = name_of(sim, id);
    // Said to a player as an action's fail text: no internal name in it
    // (D-864); the interaction is among the values.
    Reason::new(codes::OUT_OF_REACH, "that is out of reach")
        .with("interaction", name)
        .with("reach", reach.name())
}

fn not_offered(sim: &Sim, id: InteractionId) -> Reason {
    let name = name_of(sim, id);
    Reason::new(codes::NOT_OFFERED, format!("that does not offer `{name}`"))
        .with("interaction", name)
}

/// Whether the target still is: an entity that exists, an element in a
/// loaded cell.
fn present(sim: &Sim, target: Target) -> bool {
    match target {
        Target::Entity(id) => sim.entities.contains(id),
        Target::Element(loc, layer) => belief::element(sim, loc, layer).is_some(),
    }
}

// ---- Listing ---------------------------------------------------------------------

/// What a target offers a character now (16 §3.2): every interaction
/// its definitions list and every one `offered_by: any` of its kind
/// whose `shown_if` holds in the character's mind, each with the reason
/// it cannot be done now, if there is one (out of reach, or its `if`).
///
/// Worked out from what the character believes, through [`belief`].
/// Changes nothing and raises nothing: `interaction_reach` is raised
/// when an interaction is attempted, so a handler that extends reach is
/// honoured then, not here.
pub fn interactions_for(sim: &Sim, user: EntityId, target: Target) -> Vec<Offer> {
    if !present(sim, target) || !sim.entities.contains(user) {
        return Vec::new();
    }
    let values = bound(sim, user, target);
    let mut offers = Vec::new();
    for id in offered(sim, target) {
        let Some(def) = def(sim, id) else {
            continue;
        };
        if def.target != target.kind() {
            continue;
        }
        let shown = {
            let mut host = sim.host(bound_of(&values));
            def.shown_if.holds_on(&sim.defs, &mut host)
        };
        if !shown {
            continue;
        }
        let refusal = if !belief::reaches(sim, user, target, def.reach) {
            Some(out_of_reach(sim, id, def.reach))
        } else if !belief::holds(sim, &def.condition, &values) {
            Some(not_now(sim, id, def, &values))
        } else {
            None
        };
        let name = def
            .name
            .as_ref()
            .map(|name| {
                let mut host = sim.host(bound_of(&values));
                name.render_on(&sim.defs, &mut host)
            })
            .filter(|name| !name.is_empty())
            .unwrap_or_else(|| name_of(sim, id));
        offers.push(Offer {
            interaction: id,
            id: name_of(sim, id),
            name,
            default: def.default,
            refusal,
        });
    }
    offers
}

/// The interaction used when a target is bumped into or clicked without
/// a choice (16 §3.1 `default`): the first offered that is a default and
/// can be done now.
pub fn default_for(sim: &Sim, user: EntityId, target: Target) -> Option<Offer> {
    interactions_for(sim, user, target)
        .into_iter()
        .find(|offer| offer.default && offer.available())
}

/// What walking into the next cell in a direction offers instead of a
/// step (16 §3.1): the default interaction of the block there, else of
/// its fixture, else of its floor. `dir` is one of the ten directions of
/// a step (`north`, …, `up`, `down`). `None` if there is none, or the
/// user is not in the world.
pub fn bump_default(sim: &Sim, user: EntityId, dir: &str) -> Option<(Target, Offer)> {
    let step = crate::movement::parse_dir(dir)?;
    let (from, _) = place_of(sim, Target::Entity(user))?;
    let delta = step.delta();
    let to = Location::new(
        from.grid,
        CellPos::new(
            from.cell.x + delta.x,
            from.cell.y + delta.y,
            from.cell.z + delta.z,
        ),
    );
    let to = sim.cosmos.normalize(to).ok()?;
    [Layer::Block, Layer::Fixture, Layer::Floor]
        .into_iter()
        .map(|layer| Target::Element(to, layer))
        .find_map(|target| default_for(sim, user, target).map(|offer| (target, offer)))
}

/// Every offer within reach of a step of a character, for a context
/// menu (12 §9.4): the elements of its cell and of the cells round it on
/// its storey, the entities standing there, and what it holds; each
/// target with what it offers, leaving out targets that offer nothing.
pub fn offers_near(sim: &Sim, user: EntityId) -> Vec<(Target, Vec<Offer>)> {
    let Some((at, _)) = place_of(sim, Target::Entity(user)) else {
        return Vec::new();
    };
    let mut targets = Vec::new();
    for dy in -1..=1 {
        for dx in -1..=1 {
            let cell = CellPos::new(at.cell.x + dx, at.cell.y + dy, at.cell.z);
            let Ok(loc) = sim.cosmos.normalize(Location::new(at.grid, cell)) else {
                continue;
            };
            for layer in [Layer::Block, Layer::Fixture, Layer::Floor] {
                targets.push(Target::Element(loc, layer));
            }
            if let Ok(found) = sim
                .cosmos
                .occupants_at(loc, &OccupantFilter::ANY.except(user))
            {
                targets.extend(distinct_ids(&found).into_iter().map(Target::Entity));
            }
        }
    }
    targets.extend(
        sim.entities
            .contents_deep(user)
            .into_iter()
            .map(Target::Entity),
    );
    targets
        .into_iter()
        .filter_map(|target| {
            let offers = interactions_for(sim, user, target);
            (!offers.is_empty()).then_some((target, offers))
        })
        .collect()
}

/// The furthest, in metres, [`places_offering`] looks from where it is
/// asked: a house and the rooms about it, never a town.
pub const MAX_PLACES_RADIUS_M: f64 = 16.0;

/// The elements within `radius_m` metres of `at` (as `dist` measures,
/// a storey counting three), blocks and fixtures, whose definition lists
/// the interaction, nearest `at` first and then in order of cell
/// (storey, row, column), a cell's block before its fixture, leaving out
/// those whose cell is in `except` (09 §4.1, the function `places`).
/// What elements stand where, and what their definitions offer, is
/// static geometry (09 §3.4), so any mind may know it; an element whose
/// definition is tagged `hidden` is not listed, nor are cells that are
/// not loaded. The radius is held to [`MAX_PLACES_RADIUS_M`].
pub fn places_offering(
    sim: &Sim,
    at: Location,
    radius_m: Real,
    interaction: InteractionId,
    except: &[Location],
) -> Vec<(Location, Layer)> {
    let radius = radius_m.min(Real::of(MAX_PLACES_RADIUS_M));
    if !radius.is_finite() || radius < Real::ZERO {
        return Vec::new();
    }
    let reach = radius.floor().to_i32();
    let storeys = (radius / clash_world::dims::STOREY_M).floor().to_i32();
    let world = sim.cosmos.defs();
    let mut found: Vec<(Real, Location, Layer)> = Vec::new();
    for dz in -storeys..=storeys {
        for dy in -reach..=reach {
            for dx in -reach..=reach {
                let cell = CellPos::new(at.cell.x + dx, at.cell.y + dy, at.cell.z + dz);
                let Ok(loc) = sim.cosmos.normalize(Location::new(at.grid, cell)) else {
                    continue;
                };
                let Some(distance) = sim.cosmos.delta(at, loc).map(clash_world::distance_m) else {
                    continue;
                };
                if distance > radius || except.contains(&loc) {
                    continue;
                }
                let offering = [Layer::Block, Layer::Fixture].into_iter().find(|layer| {
                    let Some(state) = belief::element(sim, loc, *layer) else {
                        return false;
                    };
                    let hidden = world.element(*layer, state).is_some_and(|element| {
                        element
                            .tags
                            .iter()
                            .any(|tag| tag == crate::perception::paths::HIDDEN_TAG)
                    });
                    !hidden && listed(sim, Target::Element(loc, *layer)).contains(&interaction)
                });
                if let Some(layer) = offering {
                    found.push((distance, loc, layer));
                }
            }
        }
    }
    found.sort_by(|(a, a_at, _), (b, b_at, _)| {
        a.cmp(b).then_with(|| {
            (a_at.cell.z, a_at.cell.y, a_at.cell.x).cmp(&(b_at.cell.z, b_at.cell.y, b_at.cell.x))
        })
    });
    found
        .into_iter()
        .map(|(_, loc, layer)| (loc, layer))
        .collect()
}

/// Where a target is from its user, in a word: `here`, `held`, one of
/// the eight compass directions on the user's storey, or `up` or
/// `down`; empty where it cannot be said.
fn where_from(sim: &Sim, user: EntityId, target: Target) -> &'static str {
    if let Target::Entity(id) = target
        && sim.entities.contents_deep(user).contains(&id)
    {
        return "held";
    }
    let (Some((from, _)), Some((to, _))) =
        (place_of(sim, Target::Entity(user)), place_of(sim, target))
    else {
        return "";
    };
    let Some(delta) = sim.cosmos.delta(from, to) else {
        return "";
    };
    match (delta.x.signum(), delta.y.signum(), delta.z.signum()) {
        (0, 0, 0) => "here",
        (0, 0, 1) => "up",
        (0, 0, -1) => "down",
        (0, -1, _) => "north",
        (1, -1, _) => "north-east",
        (1, 0, _) => "east",
        (1, 1, _) => "south-east",
        (0, 1, _) => "south",
        (-1, 1, _) => "south-west",
        (-1, 0, _) => "west",
        (-1, -1, _) => "north-west",
        _ => "",
    }
}

/// What [`offers_near`] gives, as the property `offers_near` gives it:
/// a list of maps, each one interaction one target offers.
fn offers_near_value(sim: &Sim, user: EntityId) -> Value {
    let ty = sim.exprs.registry().def_type_id(InteractionDef::TYPE);
    let mut rows = Vec::new();
    for (target, offers) in offers_near(sim, user) {
        let target_value = sim.subject_value(target.subject());
        let place = where_from(sim, user, target);
        for offer in offers {
            let interaction = ty.map_or_else(
                || Value::from(offer.id.as_str()),
                |ty| {
                    Value::Id(clash_expr::DefRef::new(
                        ty,
                        u32::from(offer.interaction.raw()),
                    ))
                },
            );
            let why = offer
                .refusal
                .as_ref()
                .map_or_else(String::new, |reason| reason.message.clone());
            rows.push(Value::map([
                ("target", target_value.clone()),
                ("interaction", interaction),
                ("id", Value::from(offer.id.as_str())),
                ("name", Value::from(offer.name.as_str())),
                ("default", Value::Bool(offer.default)),
                ("available", Value::Bool(offer.refusal.is_none())),
                ("why", Value::from(why.as_str())),
                ("where", Value::from(place)),
            ]));
        }
    }
    Value::list(rows)
}

// ---- Doing it --------------------------------------------------------------------

/// The interaction and its definition, or why there is none.
fn found(sim: &Sim, id: InteractionId) -> Result<&InteractionDef, Reason> {
    if registry(&sim.defs).is_none() {
        return Err(Reason::new(
            codes::NOT_REGISTERED,
            "the definitions were loaded without the `interaction` type",
        ));
    }
    def(sim, id).ok_or_else(|| {
        Reason::new(
            codes::UNKNOWN,
            format!("there is no interaction {}", id.raw()),
        )
        .with("interaction", i64::from(id.raw()))
    })
}

/// The subjects of this module's events for a user and a target:
/// `self`, and `target` for an entity or `element` for an element.
fn event_subjects(payload: EventPayload, user: EntityId, target: Target) -> EventPayload {
    let payload = payload.subject("self", Subject::Entity(user));
    match target {
        Target::Entity(id) => payload.subject("target", Subject::Entity(id)),
        Target::Element(loc, layer) => payload.subject("element", Subject::Element(loc, layer)),
    }
}

/// Tests reach against truth, then lets the hook `interaction_reach`
/// change the answer.
fn reach_with_hook(
    sim: &mut Sim,
    user: EntityId,
    target: Target,
    id: InteractionId,
    reach: Reach,
    cause: Cause,
) -> bool {
    let reached = reaches(sim, user, target, reach);
    if !sim.bus.has_listeners(INTERACTION_REACH) {
        return reached;
    }
    let payload = event_subjects(EventPayload::new(INTERACTION_REACH), user, target)
        .with("interaction", FieldValue::text(name_of(sim, id)))
        .with("reach", FieldValue::text(reach.name()))
        .with(
            "reached",
            FieldValue::Plain(clash_core::Value::Bool(reached)),
        );
    match sim.raise(Where::Entity(user), cause, payload) {
        Ok(raised) if raised.cancelled.is_none() => raised
            .field("reached")
            .and_then(FieldValue::plain)
            .and_then(clash_core::Value::as_bool)
            .unwrap_or(reached),
        Ok(_) => false,
        Err(_) => reached,
    }
}

/// The checks every attempt makes against truth: the target is there and
/// offers the interaction, the user reaches it (with the hook) and its
/// `if` holds. Gives the values its expressions are bound to.
fn check(
    sim: &mut Sim,
    user: EntityId,
    target: Target,
    id: InteractionId,
    cause: Cause,
) -> Result<Vec<Value>, Reason> {
    sim.known(user)?;
    let defs = Arc::clone(&sim.defs);
    let def = {
        let found = found(sim, id)?;
        (found.target, found.reach)
    };
    if !present(sim, target) {
        let name = name_of(sim, id);
        return Err(Reason::new(codes::NO_TARGET, "that is not there").with("interaction", name));
    }
    if def.0 != target.kind() || !offered(sim, target).contains(&id) {
        return Err(not_offered(sim, id));
    }
    if !reach_with_hook(sim, user, target, id, def.1, cause) {
        return Err(out_of_reach(sim, id, def.1));
    }
    let values = bound(sim, user, target);
    let Some(interaction) = defs
        .try_registry::<InteractionDef>()
        .and_then(|r| r.try_get(id))
    else {
        return Err(not_offered(sim, id));
    };
    let holds = sim.with_random(bound_of(&values), |host| {
        interaction.condition.holds_on(&defs, host)
    });
    if !holds {
        return Err(not_now(sim, id, interaction, &values));
    }
    Ok(values)
}

/// Begins an interaction (16 §3.4): checks that the target offers it,
/// that the user reaches it (raising `interaction_reach`, whose handlers
/// may extend or deny reach) and that its `if` holds, against truth;
/// then raises the hook `interaction_starting`, where locks, alarms,
/// permissions and traps are rules. A handler that cancels refuses the
/// interaction with its reason.
///
/// Changes nothing itself: what the hooks' handlers do is theirs.
pub fn begin(
    sim: &mut Sim,
    user: EntityId,
    target: Target,
    id: InteractionId,
    cause: Cause,
) -> Result<(), Reason> {
    check(sim, user, target, id, cause)?;
    if !sim.bus.has_listeners(INTERACTION_STARTING) {
        return Ok(());
    }
    let payload = event_subjects(EventPayload::new(INTERACTION_STARTING), user, target)
        .with("interaction", FieldValue::text(name_of(sim, id)));
    match sim.raise(Where::Entity(user), cause, payload) {
        Ok(raised) => raised.cancelled.map_or(Ok(()), Err),
        Err(_) => Ok(()),
    }
}

/// Completes an interaction (16 §3.4): checks reach and `if` against
/// truth again, since the world may have changed while it was being
/// done; runs its `do` through the rule runtime, with `self` the user,
/// `target` the thing and `here` its cell, and whatever it changes
/// caused by `cause`; raises `interacted`; and lets a device the target
/// has, or was promoted to on the way, settle (16 §4.3).
pub fn complete(
    sim: &mut Sim,
    user: EntityId,
    target: Target,
    id: InteractionId,
    cause: Cause,
) -> Result<(), Reason> {
    let values = check(sim, user, target, id, cause)?;
    let defs = Arc::clone(&sim.defs);
    if let Some(interaction) = defs
        .try_registry::<InteractionDef>()
        .and_then(|r| r.try_get(id))
    {
        let mut locals = Vec::new();
        sim.run_effects(
            &interaction.effects,
            &values,
            &mut locals,
            Some(Subject::Entity(user)),
            cause,
        );
    }
    let payload = event_subjects(EventPayload::new(INTERACTED), user, target)
        .with("interaction", FieldValue::text(name_of(sim, id)));
    let _ = sim.raise(Where::Entity(user), cause, payload);
    let device = match target {
        Target::Element(loc, layer) => device::device_at(sim, loc, layer),
        Target::Entity(id) => sim.components.get::<Device>(id).map(|_| id),
    };
    if let Some(device) = device {
        device::settle(sim, device, cause);
    }
    Ok(())
}

/// Does an interaction at once: [`begin`], then [`complete`]. What a
/// device's or a rule's `interact` does; a character does it through an
/// action that takes its time (see the module documentation).
pub fn interact(
    sim: &mut Sim,
    user: EntityId,
    target: Target,
    id: InteractionId,
    cause: Cause,
) -> Result<(), Reason> {
    begin(sim, user, target, id, cause)?;
    complete(sim, user, target, id, cause)
}

/// How long an interaction takes, in the user's own time.
pub fn time_of(sim: &Sim, id: InteractionId) -> Option<clash_core::Dur> {
    def(sim, id).map(|def| def.time.0)
}

// ---- The action's start -----------------------------------------------------------

/// The name of an action parameter that makes the action an
/// interaction's: the interaction.
pub const INTERACTION_PARAM: &str = "interaction";
/// The name of the parameter that holds what it is done to.
pub const TARGET_PARAM: &str = "target";

/// Registers what makes an interaction's action begin it: a handler of
/// `action_starting` that, for an action whose parameters `interaction`
/// and `target` name an interaction and an element or an entity, calls
/// [`begin`] and refuses the action with its reason. Called when the
/// simulation is set up and again after a load; handlers are not
/// state.
pub(crate) fn bind(sim: &mut Sim) {
    let _ = sim.on(
        crate::action::ACTION_STARTING,
        Handler::new(action_starting),
    );
}

fn action_starting(sim: &mut Sim, delivery: &mut crate::bus::Delivery<'_>) {
    let Some(params) = delivery.field("params") else {
        return;
    };
    let params = sim.exprs.field_value(&params);
    let Some(map) = params.as_map() else {
        return;
    };
    let (Some(interaction), Some(target)) = (map.get(INTERACTION_PARAM), map.get(TARGET_PARAM))
    else {
        return;
    };
    let Some(id) = interaction_of(sim, interaction) else {
        return;
    };
    let (Some(Subject::Entity(user)), Some(target)) = (
        delivery.event().subject("self"),
        sim.subject_of(target).and_then(Target::of),
    ) else {
        return;
    };
    let cause = delivery.event().cause;
    if let Err(reason) = begin(sim, user, target, id, cause) {
        let _ = delivery.cancel(reason);
    }
}

/// The interaction a value names: an id of the type, or its qualified id
/// as a text.
pub fn interaction_of(sim: &Sim, value: &Value) -> Option<InteractionId> {
    match value {
        Value::Id(id) if sim.exprs.registry().def_type_id(InteractionDef::TYPE) == Some(id.ty) => {
            u16::try_from(id.index).ok().map(InteractionId::new)
        }
        Value::Str(name) => lookup(sim, name),
        _ => None,
    }
}

// ---- Events ----------------------------------------------------------------------

/// The hook raised when reach is tested for an attempt.
pub const INTERACTION_REACH: &str = "interaction_reach";
/// The hook raised before an interaction begins.
pub const INTERACTION_STARTING: &str = "interaction_starting";
/// An interaction was done.
pub const INTERACTED: &str = "interacted";

struct EventRow {
    name: &'static str,
    doc: &'static str,
    fields: &'static [(&'static str, &'static str, &'static str)],
    mutable: &'static [&'static str],
    hook: bool,
}

/// The subjects every event of this module has: the user, and the
/// target as an entity or an element.
const SUBJECTS: &[(&str, &str)] = &[
    ("self", "entity"),
    ("target?", "entity"),
    ("element?", "element"),
];

const INTERACTION_FIELD: (&str, &str, &str) = (
    "interaction",
    "string",
    "The qualified id of the interaction.",
);

const EVENTS: &[EventRow] = &[
    EventRow {
        name: INTERACTION_REACH,
        doc: "Runs when reach is tested for an attempt at an interaction (16 §3.3): `self` the user; `target` the entity, or `element` the element, it is done to. A handler may set `reached` to extend reach (telekinesis, a long tongue) or deny it. Not raised by a listing of what is offered.",
        fields: &[
            INTERACTION_FIELD,
            (
                "reach",
                "string",
                "The interaction's reach: `held`, `same_cell`, `adjacent`, `range` or `network`.",
            ),
            (
                "reached",
                "bool",
                "Whether the user reaches the target. Mutable.",
            ),
        ],
        mutable: &["reached"],
        hook: true,
    },
    EventRow {
        name: INTERACTION_STARTING,
        doc: "Runs before an interaction begins (16 §3.4), once reach and `if` have held. A handler may cancel with a reason, which is what the user is told: a lock, an alarm, a permission, a trap.",
        fields: &[INTERACTION_FIELD],
        mutable: &[],
        hook: true,
    },
    EventRow {
        name: INTERACTED,
        doc: "An interaction was done: its effects have run (16 §3.4).",
        fields: &[INTERACTION_FIELD],
        mutable: &[],
        hook: false,
    },
];

/// The event types of this module, for the simulation's bus (05 §8.1).
pub fn event_types() -> Vec<EventType> {
    EVENTS
        .iter()
        .map(|row| {
            let mut ty = EventType::new(row.name).doc(row.doc);
            for (name, kind) in SUBJECTS {
                ty = ty.subject(*name, *kind);
            }
            ty.fields = row
                .fields
                .iter()
                .map(|(name, written, doc)| FieldSpec {
                    name: (*name).to_owned(),
                    ty: (*written).to_owned(),
                    check: Some(FieldType::parse(written).unwrap_or(FieldType::Any)),
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

/// Enters this module's events into an expression registry, leaving
/// alone those it already holds.
pub fn register_events(registry: &mut Registry) {
    for row in EVENTS {
        if registry.event_id(row.name).is_some() {
            continue;
        }
        let mut spec = EventSpec::new(row.name).doc(row.doc);
        for (name, kind) in SUBJECTS.iter().chain(CAUSE_SUBJECTS) {
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

// ---- Properties and verbs ----------------------------------------------------------

/// The user and the target an entity property's arguments name.
fn user_and_target(sim: &Sim, subject: Subject, args: &[Value]) -> Option<(EntityId, Target)> {
    let Subject::Entity(user) = subject else {
        return None;
    };
    let target = sim.subject_of(args.first()?).and_then(Target::of)?;
    Some((user, target))
}

/// The offer of one interaction by a target, as the user believes it.
fn offer_of(sim: &Sim, subject: Subject, args: &[Value]) -> Option<Offer> {
    let (user, target) = user_and_target(sim, subject, args)?;
    let id = interaction_of(sim, args.get(1)?)?;
    interactions_for(sim, user, target)
        .into_iter()
        .find(|offer| offer.interaction == id)
}

/// The properties of interactions (16 §3.5, §14). None is safe for
/// belief; the entity's are read of the character itself in a mind
/// context, and read the world through [`belief`].
pub fn properties() -> Vec<PropertyDef> {
    vec![
        PropertyDef::new(
            "entity",
            "can_interact",
            "bool",
            "Whether the entity, as it believes, can do an interaction to a target now: the target offers it, it is in reach and its `if` holds (16 §3.2). What an interaction's action `requires`.",
            |sim, subject, args| Value::Bool(offer_of(sim, subject, args).is_some_and(|offer| offer.available())),
        )
        .arg("target", "any", "An element or an entity.")
        .arg("interaction", "id<interaction>", "The interaction."),
        PropertyDef::new(
            "entity",
            "interaction_fault",
            "string?",
            "Why the entity, as it believes, cannot do an interaction to a target now: null if it can. A refusal's sentence, for an action's `fail_text`.",
            |sim, subject, args| match offer_of(sim, subject, args) {
                Some(Offer { refusal: Some(reason), .. }) => Value::from(reason.message.as_str()),
                Some(_) => Value::Null,
                None => Value::from("that is not offered there"),
            },
        )
        .arg("target", "any", "An element or an entity.")
        .arg("interaction", "id<interaction>", "The interaction."),
        PropertyDef::new(
            "entity",
            "interaction_time",
            "real?",
            "How long an interaction takes, in seconds of the doer's own time (16 §3.1 `time`): what its action's phase lasts. Null for what is not an interaction.",
            |sim, _, args| {
                args.first()
                    .and_then(|value| interaction_of(sim, value))
                    .and_then(|id| time_of(sim, id))
                    .map_or(Value::Null, |time| Value::Real(time.as_secs_real()))
            },
        )
        .arg("interaction", "id<interaction>", "The interaction."),
        PropertyDef::new(
            "entity",
            "has_item",
            "bool",
            "Whether the entity holds, wears or carries an item of a definition, at any depth: a key in a pocket counts.",
            |sim, subject, args| {
                let Subject::Entity(id) = subject else {
                    return Value::Bool(false);
                };
                let wanted = args.first().and_then(|value| match value {
                    Value::Id(def) => sim.exprs.registry().def_name(*def).map(str::to_owned),
                    Value::Str(name) => Some(name.to_string()),
                    _ => None,
                });
                let Some(wanted) = wanted.and_then(|name| crate::item::def_named(sim, &name).ok()) else {
                    return Value::Bool(false);
                };
                Value::Bool(sim.entities.contents_deep(id).into_iter().any(|inside| {
                    sim.components
                        .get::<Item>(inside)
                        .is_some_and(|item| item.def() == Some(wanted))
                }))
            },
        )
        .arg("def", "id<item>", "The item definition."),
        PropertyDef::new(
            "entity",
            "offers_near",
            "list<map<any>>",
            "What the entity, as it believes, could do within a step: every interaction offered by the elements of its cell and the cells round it on its storey, by what stands there and by what it holds (16 §3.5.1), for a menu. Each is a map: `target` (the element or the entity), `interaction` (its id), `id` (its qualified id as a text), `name` (what a menu calls it), `default` (whether it is the target's default), `available` (whether it can be done now), `why` (the refusal's sentence where it cannot, else empty) and `where` (`here`, `held`, a compass direction such as `north-east`, `up` or `down`).",
            |sim, subject, _| match subject {
                Subject::Entity(id) => offers_near_value(sim, id),
                _ => Value::list([]),
            },
        ),
        PropertyDef::new(
            "element",
            "interactions",
            "list<string>",
            "The qualified ids of the interactions the element's definition lists (02 §5.5), in order. Those offered by `any` are not listed.",
            |sim, subject, _| match Target::of(subject) {
                Some(target @ Target::Element(..)) => {
                    Value::list(listed(sim, target).into_iter().map(|id| Value::from(name_of(sim, id))))
                }
                _ => Value::list([]),
            },
        )
        .changed_by(&["element_changed"]),
    ]
}

/// The arguments of the interaction verbs.
const VERB_ARGS: [ArgRow; 3] = [
    (
        "who",
        "entity",
        false,
        "Who does it: a character, or a device acting.",
    ),
    (
        "target",
        "any",
        false,
        "What it is done to: an element or an entity.",
    ),
    ("interaction", "id<interaction>", false, "The interaction."),
];

/// The three arguments of an interaction verb.
fn verb_args(
    sim: &Sim,
    verb: &str,
    call: &crate::rule::VerbCall<'_>,
) -> Result<(EntityId, Target, InteractionId), Reason> {
    let bad = |name: &str, wanted: &str| {
        Reason::new(
            codes::BAD_ARGUMENT,
            format!("`{verb}`: the argument `{name}` must be {wanted}"),
        )
        .with("verb", verb)
        .with("arg", name)
    };
    let who = entity_arg(sim, verb, call, 0, "who").map_err(|_| bad("who", "an entity"))?;
    let target = sim
        .subject_of(arg(call, 1))
        .and_then(Target::of)
        .ok_or_else(|| bad("target", "an element or an entity"))?;
    let id =
        interaction_of(sim, arg(call, 2)).ok_or_else(|| bad("interaction", "an interaction"))?;
    Ok((who, target, id))
}

/// The verbs of interactions, for [`crate::verb::engine_verbs`].
pub fn verbs() -> Vec<VerbDef> {
    vec![
        VerbDef {
            name: "interact",
            build: |registry| {
                spec(
                    registry,
                    "interact",
                    "Does an interaction at once (16 §3.4): checks that the target offers it, that `who` reaches it (raising `interaction_reach`) and that its `if` holds, raises `interaction_starting`, runs its `do` and raises `interacted`. What a device's rules call: an actuator that opens a door. A character does it through an interaction's action, which takes its time. Refused with `interaction.not_offered`, `interaction.out_of_reach`, `interaction.not_now`, `interaction.no_target`, and by a handler of `interaction_starting` with its reason.",
                    &VERB_ARGS,
                )
            },
            run: |sim, call, cause| {
                let (who, target, id) = verb_args(sim, "interact", call)?;
                interact(sim, who, target, id, cause)?;
                Ok(Value::Null)
            },
        },
        VerbDef {
            name: "complete_interaction",
            build: |registry| {
                spec(
                    registry,
                    "complete_interaction",
                    "Ends an interaction its action began (16 §3.4): tests reach and `if` against truth again, runs its `do` and raises `interacted`. The effect that ends the phase of an interaction's action. Refused as `interact` is, but for the hook `interaction_starting`, which ran when the action began.",
                    &VERB_ARGS,
                )
            },
            run: |sim, call, cause| {
                let (who, target, id) = verb_args(sim, "complete_interaction", call)?;
                complete(sim, who, target, id, cause)?;
                Ok(Value::Null)
            },
        },
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_reason_is_listed_once() {
        let mut codes: Vec<&str> = INTERACTION_REASONS.iter().map(|info| info.code).collect();
        let count = codes.len();
        codes.sort_unstable();
        codes.dedup();
        assert_eq!(codes.len(), count);
    }

    #[test]
    fn every_event_and_verb_registers() {
        let registry = crate::engine_registry();
        for ty in event_types() {
            assert!(registry.event_id(&ty.name).is_some(), "{}", ty.name);
        }
        for verb in verbs() {
            assert!(registry.verb_id(verb.name).is_some(), "{}", verb.name);
        }
    }
}
