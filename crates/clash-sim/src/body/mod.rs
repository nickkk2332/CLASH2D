//! Bodies, hits and wounds (`docs/06-entities-and-bodies.md` sections 6
//! and 7).
//!
//! A body is a tree of **parts**. Each part is built of **layers** of
//! material, outermost first. A hit is a damage packet (02 §12) that
//! works inward through those layers by the rule that breaks walls. What
//! it breaches becomes a **wound**. There are no hit points: wounds lower
//! what parts can do, parts supply the body's **functions** and the
//! channel slots actions need, and breached tissue drains pools.
//!
//! | Part | Is | Module |
//! |------|----|--------|
//! | [`BodyPartDef`], [`BodyPlanDef`], [`BodyFunctionDef`] | The definition types `body_part`, `body_plan` and `body_function` | `defs` |
//! | [`BodyDefs`] | What is worked out from them and from the world's materials once mods have loaded | `defs` |
//! | [`Body`], [`PartInst`], [`LayerInst`] | The component: one entity's parts, wounds and cached derived values | here |
//! | [`BodyCx`] | What every function that changes a body is handed | here |
//! | [`Change`] | What a function reports having changed, for the caller to raise events from | here |
//! | [`BodyError`] | Why a change was refused, with a [`reason`] code | here |
//! | [`Hit`], [`apply_hit`], [`HitReport`] | Everything that harms a body, and what it did | [`hit`] |
//! | [`Wound`], [`close_wound`], [`on_wake`] | Wounds, their leaks, closing and healing | [`wound`] |
//! | [`props`] | The kinds, properties and events registered with the expression language, and [`property`], which answers them | [`props`] |
//!
//! Nothing here knows the simulation. The functions take the stores, the
//! definitions, the timeline and an expression host, and return what
//! changed as plain values.
//!
//! # The rules, in short
//!
//! - A part's **condition** is `1 − Σ (layer weight × layer damage / 255)`.
//!   Below its definition's `disable_below` the part is disabled: it
//!   contributes nothing and its channel slots are gone
//!   ([`Change::SlotsLost`]), which is what interrupts an action using
//!   them (03 §6.2).
//! - A **function**'s capacity combines what every part that provides it
//!   contributes: its `provides` times its condition while it works, and
//!   nothing while it is disabled or gone.
//! - **Scale.** Thickness and length go with scale, area with its
//!   square, mass with its cube.
//! - Every derived value (functions, slots, mass and size) is cached in
//!   the component, saved and hashed, and is bit for bit what
//!   [`recompute`] gives (06 invariants B2, B3).
//!
//! # What the kernel holds and calls
//!
//! The simulation holds a [`BodyDefs`] beside its definitions, made with
//! [`BodyDefs::new`] after loading. The [`Body`] components are in
//! `components.body`.
//!
//! | When | The kernel |
//! |------|-----------|
//! | Building its definition types | Calls the crate's `register_defs`, which registers the three types, the tissue check and the [registry step](props::registry_step). |
//! | A creature is made | Calls [`attach`] with a plan, a scale and the entity's clock, after its stats if it has any. If its stats read a body function, calls `stats::refresh` afterwards. |
//! | Anything harms a body | Builds a [`Hit`] (combat does, or [`exposure_hit`], [`impact_hit`], [`field_hit`] from what the world reported), raises the hook `hit_incoming` with the hit's fields, then calls [`apply_hit`] with the layers worn over the struck parts and its random stream. |
//! | A function returns [`Change`]s | Raises the events of [`props`] from them, in order; interrupts actions that held slots named by [`Change::SlotsLost`]; drops what a part that can no longer hold was holding; passes [`Change::MassChanged`] to the world's occupant record; raises the stats' events from [`Change::Stats`]; and calls `stats::refresh` if a [`Change::FunctionChanged`] was among them. |
//! | [`apply_hit`] returns [`Detached`] parts or [`Exit`]s | Hands the parts to whoever makes items, and carries each exit on as a packet along the hit's direction. |
//! | A rule disables, restores or removes a part, or treats a wound | Calls [`disable`], [`restore`], [`detach`], [`close_wound`], [`reopen_wound`], [`heal_part`]. |
//! | A wake-up pops for which [`is_body_wake`] is true | Calls [`on_wake`]. |
//! | The entity moves to another clock (03 §3.4) | Calls [`set_clock`]. |
//! | The entity is stored, unloaded or destroyed | Calls [`suspend`] before the component is taken out of the store, so that no wake-up is left behind. |
//! | A stored entity returns (03 §9) | Puts the component back, then calls [`resume`] with its clock and the local time that passed. |
//! | A posture changes how tall the body stands (06 §11.1) | Calls [`set_height_factor`]. |
//! | An expression reads a property for which [`BodyDefs::answers`] is true | Forwards to [`property`]. |
//!
//! The functions here change pools through `stats` and never evaluate an
//! expression, so the host in [`BodyCx`] is only passed on.
//!
//! # The wake-up
//!
//! A body with a wound that will close or a layer that is healing keeps
//! **one** wake-up on the timeline, on its owner's clock, in the domain
//! [`WakeDomain::Timer`], for the earliest of them. Its `id` is the
//! owner's [`EntityId::to_bits`] and its `arg` is [`BODY_WAKE_ARG`],
//! which is what tells it from a pool's crossing and from the kernel's
//! own timers: [`is_body_wake`].
//!
//! # What is not built here
//!
//! - **Raising events and running hooks.** Events are registered here
//!   and raised by the kernel from the [`Change`]s returned.
//! - **Editing bodies** (06 §6.5) beyond [`disable`], [`restore`],
//!   [`detach`] and [`heal_part`], and **bodies made of cells**
//!   (06 §6.6).
//! - **Items.** A severed or destroyed part is returned as a
//!   [`Detached`]; worn layers are handed in as [`WornLayer`]s and what
//!   they took is handed back as [`WornHit`]s.
//! - **Shared parts** (06 §6.4): every body holds its own parts.
//! - **Which parts face a hit's direction** (06 §7.4): nothing yet says
//!   which way a body faces, so every exposed part takes its share.
//! - **Saving.** [`Body`] serialises through `clash_core::codec` and
//!   hashes; writing its definition ids by qualified id through a save's
//!   id table is the simulation's save path.
//!
//! [`WakeDomain::Timer`]: clash_time::WakeDomain::Timer

pub mod bind;
mod defs;
pub mod edit;
pub mod hit;
pub mod props;
mod verbs;
pub mod wound;

use std::collections::BTreeMap;

use clash_core::{Dur, EntityId, Real, StateHash, StateHasher, define_id, impl_state_hash};
use clash_data::{DefTypes, Length, Mass, Volume};
use clash_expr::{Host, Value};
use clash_time::{ClockId, TimeError, WakeId};
use clash_world::defs::{DamageTypeId, MaterialId};
use clash_world::{Cause, Footprint};
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::action::{ChannelId, ChannelSet};
use crate::component::{
    ComponentStore, Containment, MassChange, Physical, PhysicalError, Shape, physical,
};
use crate::stats::{self, StatsCx, StatsError};

pub use bind::HitOutcome;
pub(crate) use defs::pool_by_name;
pub use defs::{
    BodyDefTypes, BodyDefs, BodyFunctionDef, BodyFunctionId, BodyKinds, BodyPartDef, BodyPartId,
    BodyPlanDef, BodyPlanId, Combine, DAMAGE_PARTS, InsideDef, LayerDef, Leak, Leaves, MAX_LAYERS,
    MAX_NESTING, MAX_PARTS, PlanLayout, PlanPartDef, PlannedPart, Relation, Signal, SocketDef,
    SurfaceDef, Tissue,
};
pub use hit::{
    DEFAULT_OVERKILL_FACTOR, Exit, FieldDose, Hit, HitAt, HitReport, HitSettings, Impact,
    OVERKILL_SETTING, PartHit, WornHit, WornLayer, apply_hit, exposure_hit, field_hit, impact_hit,
};
pub use props::{property, property_at};
pub use verbs::verbs;
pub use wound::{
    BODY_WAKE_ARG, Wound, WoundState, body_wake, close_wound, heal_part, is_body_wake, on_wake,
    reopen_wound, resume, set_clock, suspend,
};

define_id! {
    /// One part of one body: the number the part was given when it was
    /// made. It is never given to another part of the same body, so it
    /// still means the same part after others have come and gone.
    pub struct PartKey(u16);
    /// One wound of one body: the number it was given when it was
    /// inflicted, not shared with any other wound the body has.
    pub struct WoundId(u16);
}

/// The store of every entity's [`Body`].
pub type BodyStore = ComponentStore<Body>;

/// Registers this module's definition types, its whole-set check and its
/// registry step. Called from the crate's `register_defs`.
pub fn register_defs(types: &mut DefTypes) {
    types
        .register::<BodyFunctionDef>()
        .register::<BodyPartDef>()
        .register::<BodyPlanDef>()
        .add_cross_check(defs::check_tissues)
        .add_registry_step(props::registry_step);
}

/// What every function that changes a body is handed: the stores, the
/// definitions, how entities are contained, and what changes stats.
///
/// The timeline is the one in `stats`: wounds and pools run on it alike.
pub struct BodyCx<'a, H: Host + ?Sized> {
    /// Every entity's body.
    pub bodies: &'a mut BodyStore,
    /// The definitions.
    pub defs: &'a BodyDefs,
    /// Every entity's size and mass, which a body supplies for its own.
    pub physical: &'a mut ComponentStore<Physical>,
    /// Who is inside whom, for the masses of containers.
    pub containment: &'a dyn Containment,
    /// The stats, their definitions, the timeline and the host: wounds
    /// leak into pools through these.
    pub stats: StatsCx<'a, H>,
}

/// Whether a part is there and works (06 §6.4).
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[repr(u8)]
pub enum PartState {
    /// There, and working.
    Whole,
    /// There, and not working: its condition is below what it needs, or
    /// something disabled it.
    Disabled,
    /// Gone: cut off, or gone with the part it was on.
    Severed,
    /// Gone: destroyed where it was.
    Destroyed,
}

impl PartState {
    /// Whether the part is still on the body.
    pub const fn is_present(self) -> bool {
        matches!(self, PartState::Whole | PartState::Disabled)
    }

    /// The word rules read: `whole`, `disabled`, `severed`, `destroyed`.
    pub const fn name(self) -> &'static str {
        match self {
            PartState::Whole => "whole",
            PartState::Disabled => "disabled",
            PartState::Severed => "severed",
            PartState::Destroyed => "destroyed",
        }
    }
}

impl StateHash for PartState {
    fn state_hash(&self, hasher: &mut StateHasher) {
        hasher.write_u8(*self as u8);
    }
}

/// One layer of one part: what it is made of now, how thick it is at
/// the part's size, and how damaged.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub struct LayerInst {
    /// What it is made of.
    pub(super) material: MaterialId,
    /// How thick it is, in metres, scale applied.
    pub(super) thickness: Real,
    /// How much the part's working depends on it.
    pub(super) weight: Real,
    /// How damaged it is, in 255ths: 255 is gone across the whole width
    /// of the part.
    pub(super) damage: u8,
    /// The local time healed toward its next 255th.
    pub(super) progress: Dur,
}

impl_state_hash!(LayerInst {
    material,
    thickness,
    weight,
    damage,
    progress
});

impl LayerInst {
    /// What it is made of.
    pub fn material(&self) -> MaterialId {
        self.material
    }

    /// How thick it is, in metres.
    pub fn thickness(&self) -> Real {
        self.thickness
    }

    /// How much the part's working depends on it.
    pub fn weight(&self) -> Real {
        self.weight
    }

    /// How damaged it is, in 255ths.
    pub fn damage(&self) -> u8 {
        self.damage
    }

    /// How damaged it is, from 0 to 1.
    pub fn damage_fraction(&self) -> Real {
        Real::from_i64(i64::from(self.damage)) / Real::from_i64(i64::from(DAMAGE_PARTS))
    }
}

/// One part of one body (06 §6.4).
///
/// A part that is gone stays listed, with a state that says how and no
/// layers, so that what the body has lost can be read and still counts,
/// as nothing, toward its functions.
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub struct PartInst {
    key: PartKey,
    name: String,
    def: BodyPartId,
    parent: Option<PartKey>,
    relation: Relation,
    socket: Option<String>,
    side: Option<String>,
    scale: Real,
    pub(super) layers: Vec<LayerInst>,
    pub(super) state: PartState,
    /// Disabled from outside, whatever its condition.
    held: bool,
}

impl_state_hash!(PartInst {
    key,
    name,
    def,
    parent,
    relation,
    socket,
    side,
    scale,
    layers,
    state,
    held
});

impl PartInst {
    /// The part's number in its body.
    pub fn key(&self) -> PartKey {
        self.key
    }

    /// Its name in its body: the plan's `id` for it.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// What kind of part it is.
    pub fn def(&self) -> BodyPartId {
        self.def
    }

    /// The part it is joined to. `None` for the root.
    pub fn parent(&self) -> Option<PartKey> {
        self.parent
    }

    /// How it is joined to its parent.
    pub fn relation(&self) -> Relation {
        self.relation
    }

    /// The socket of its parent that it takes.
    pub fn socket(&self) -> Option<&str> {
        self.socket.as_deref()
    }

    /// Its side label.
    pub fn side(&self) -> Option<&str> {
        self.side.as_deref()
    }

    /// Its size relative to the rest of its body.
    pub fn scale(&self) -> Real {
        self.scale
    }

    /// Its layers, outermost first. None for a part that is gone.
    pub fn layers(&self) -> &[LayerInst] {
        &self.layers
    }

    /// Whether it is there and whether it works.
    pub fn state(&self) -> PartState {
        self.state
    }

    /// Whether it is still on the body.
    pub fn is_present(&self) -> bool {
        self.state.is_present()
    }

    /// Whether it works: there, and not disabled.
    pub fn is_working(&self) -> bool {
        self.state == PartState::Whole
    }

    /// Whether something disabled it from outside ([`disable`]).
    pub fn is_held(&self) -> bool {
        self.held
    }

    /// Its condition, from 0 to 1 (06 §6.4):
    /// `1 − Σ (layer weight × layer damage fraction)`. Zero for a part
    /// that is gone.
    pub fn condition(&self) -> Real {
        if !self.is_present() {
            return Real::ZERO;
        }
        let mut lost = Real::ZERO;
        for layer in &self.layers {
            lost += layer.weight * layer.damage_fraction();
        }
        (Real::ONE - lost).clamp(Real::ZERO, Real::ONE)
    }
}

/// How many slots of one channel a body supplies.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub struct SlotCount {
    /// The channel.
    pub channel: ChannelId,
    /// How many slots of it.
    pub count: u16,
}

impl_state_hash!(SlotCount { channel, count });

/// Everything about a body that follows from its parts: what it can do,
/// the slots it supplies, and its mass and size.
#[derive(Clone, PartialEq, Debug, Default, Serialize, Deserialize)]
pub struct Derived {
    /// The capacity of each function, by function id: every function
    /// that is defined.
    pub functions: Vec<Real>,
    /// The channels its working parts supply slots of.
    pub channels: ChannelSet,
    /// How many slots of each, in order of channel.
    pub slots: Vec<SlotCount>,
    /// The mass of the parts that are there, in kilograms.
    pub mass: Real,
    /// The space they take up, in cubic metres.
    pub volume: Real,
    /// How tall the body stands, in metres.
    pub height: Real,
    /// The area its outer parts present, in square metres.
    pub cross_section: Real,
}

impl_state_hash!(Derived {
    functions,
    channels,
    slots,
    mass,
    volume,
    height,
    cross_section
});

/// One entity's body (06 §6.4): the `Body` component.
///
/// Its fields change only through the functions of this module, which
/// report what changed. Everything in it is saved and hashed, the cached
/// derived values included.
#[derive(Clone, PartialEq, Debug, Serialize, Deserialize)]
pub struct Body {
    /// Where it started.
    plan: BodyPlanId,
    /// The overall size factor.
    scale: Real,
    /// What its posture makes of its height.
    height_factor: Real,
    /// The parts, in order of key.
    pub(super) parts: Vec<PartInst>,
    /// The key the next part gets.
    next_part: u16,
    /// The wounds, in order of id.
    pub(super) wounds: Vec<Wound>,
    /// Where the search for the next wound's id starts.
    pub(super) next_wound: u16,
    /// What follows from the parts. Derived, and cached.
    derived: Derived,
    /// The clock wounds close and heal on: the owner's.
    pub(super) clock: ClockId,
    /// Whether the owner is out of the simulation: nothing closes or
    /// heals until [`resume`].
    pub(super) suspended: bool,
    /// The local time on the clock up to which closing and healing have
    /// been worked out.
    pub(super) synced: Dur,
    /// The wake-up for the next wound to close or layer to heal.
    pub(super) wake: Option<WakeId>,
}

impl_state_hash!(Body {
    plan,
    scale,
    height_factor,
    parts,
    next_part,
    wounds,
    next_wound,
    derived,
    clock,
    suspended,
    synced,
    wake
});

impl Body {
    /// A body of a plan with no parts in it yet, on a clock: what
    /// [`attach`] fills from the plan. To give an entity a body, use
    /// [`attach`]. This is for a body made apart from a store.
    pub fn new(plan: BodyPlanId, scale: Real, clock: ClockId) -> Body {
        Body {
            plan,
            scale,
            height_factor: Real::ONE,
            parts: Vec::new(),
            next_part: 0,
            wounds: Vec::new(),
            next_wound: 0,
            derived: Derived::default(),
            clock,
            suspended: false,
            synced: Dur::ZERO,
            wake: None,
        }
    }

    /// The plan it was made from.
    pub fn plan(&self) -> BodyPlanId {
        self.plan
    }

    /// Its overall size factor.
    pub fn scale(&self) -> Real {
        self.scale
    }

    /// The clock its wounds close and heal on.
    pub fn clock(&self) -> ClockId {
        self.clock
    }

    /// Whether nothing closes or heals: the owner is out of the
    /// simulation ([`suspend`]).
    pub fn is_suspended(&self) -> bool {
        self.suspended
    }

    /// The wake-up for the next wound to close or layer to heal, if one
    /// is ahead.
    pub fn wake_id(&self) -> Option<WakeId> {
        self.wake
    }

    /// Every part, in order of key, those that are gone included.
    pub fn parts(&self) -> &[PartInst] {
        &self.parts
    }

    fn part_at(&self, key: PartKey) -> Option<usize> {
        self.parts.binary_search_by_key(&key, PartInst::key).ok()
    }

    /// A part by its key.
    pub fn part(&self, key: PartKey) -> Option<&PartInst> {
        self.part_at(key).map(|at| &self.parts[at])
    }

    pub(super) fn part_mut(&mut self, key: PartKey) -> Option<&mut PartInst> {
        self.part_at(key).map(|at| &mut self.parts[at])
    }

    /// A part by its name.
    pub fn part_named(&self, name: &str) -> Option<&PartInst> {
        self.parts.iter().find(|part| part.name == name)
    }

    /// The root: the part joined to nothing.
    pub fn root(&self) -> Option<&PartInst> {
        self.parts.iter().find(|part| part.parent.is_none())
    }

    /// The parts joined to a part, in order of key, whether on it or in
    /// it, those that are gone included.
    pub fn children(&self, key: PartKey) -> impl Iterator<Item = &PartInst> {
        self.parts
            .iter()
            .filter(move |part| part.parent == Some(key))
    }

    /// The parts that are there and sit in a socket of a part.
    pub fn in_socket<'a>(
        &'a self,
        parent: PartKey,
        socket: &'a str,
    ) -> impl Iterator<Item = &'a PartInst> {
        self.children(parent)
            .filter(move |part| part.is_present() && part.socket() == Some(socket))
    }

    /// The parts that are there and sit inside a part: its organs.
    pub fn inside(&self, key: PartKey) -> impl Iterator<Item = &PartInst> {
        self.children(key)
            .filter(|part| part.is_present() && part.relation == Relation::In)
    }

    /// The parts that are there and whose definition carries a tag.
    pub fn with_tag<'a>(
        &'a self,
        defs: &'a BodyDefs,
        tag: clash_data::Tag,
    ) -> impl Iterator<Item = &'a PartInst> {
        self.parts.iter().filter(move |part| {
            part.is_present()
                && defs
                    .part(part.def)
                    .is_some_and(|def| def.tags.contains(tag))
        })
    }

    /// The parts that are there and are of one definition.
    pub fn of_def(&self, def: BodyPartId) -> impl Iterator<Item = &PartInst> {
        self.parts
            .iter()
            .filter(move |part| part.is_present() && part.def == def)
    }

    /// The parts that are gone.
    pub fn missing(&self) -> impl Iterator<Item = &PartInst> {
        self.parts.iter().filter(|part| !part.is_present())
    }

    /// The parts a part sits inside, nearest first: what a hit must go
    /// through to reach it. Empty for a part on the outside.
    pub fn covering(&self, key: PartKey) -> Vec<PartKey> {
        let mut covering = Vec::new();
        let mut at = self.part(key);
        // A tree has no more levels than parts.
        for _ in 0..self.parts.len() {
            let Some(part) = at else { break };
            let Some(parent) = part.parent else { break };
            if part.relation == Relation::In {
                covering.push(parent);
            }
            at = self.part(parent);
        }
        covering
    }

    /// Whether a part is there and on the outside: it sits inside
    /// nothing, and neither does anything it is on.
    pub fn is_exposed(&self, key: PartKey) -> bool {
        self.part(key).is_some_and(PartInst::is_present) && self.covering(key).is_empty()
    }

    /// A part's size factor: the body's times its own.
    pub fn part_scale(&self, part: &PartInst) -> Real {
        self.scale * part.scale
    }

    /// How long a part is, in metres.
    pub fn part_length(&self, defs: &BodyDefs, part: &PartInst) -> Real {
        defs.part(part.def)
            .map_or(Real::ZERO, |def| def.length.get() * self.part_scale(part))
    }

    /// How far across a part is, in metres.
    pub fn part_girth(&self, defs: &BodyDefs, part: &PartInst) -> Real {
        defs.part(part.def)
            .map_or(Real::ZERO, |def| def.girth.get() * self.part_scale(part))
    }

    /// The area a part presents, in square metres: its silhouette,
    /// `length × girth`. Every layer of the part has this area.
    pub fn part_area(&self, defs: &BodyDefs, part: &PartInst) -> Real {
        self.part_length(defs, part) * self.part_girth(defs, part)
    }

    /// A part's mass, in kilograms. Zero for one that is gone.
    pub fn part_mass(&self, defs: &BodyDefs, part: &PartInst) -> Real {
        if !part.is_present() {
            return Real::ZERO;
        }
        let scale = self.part_scale(part);
        defs.part(part.def)
            .map_or(Real::ZERO, |def| def.mass.get() * scale * scale * scale)
    }

    /// The area a part offers to a hit that is not aimed, in square
    /// metres: its area times its definition's `exposure`. Zero for a
    /// part that is gone or sits inside another.
    pub fn exposed_area(&self, defs: &BodyDefs, part: &PartInst) -> Real {
        if !self.is_exposed(part.key) {
            return Real::ZERO;
        }
        let exposure = defs.part(part.def).map_or(Real::ZERO, |def| def.exposure);
        self.part_area(defs, part) * exposure.max(Real::ZERO)
    }

    /// How far a part can reach from the trunk, in metres: its own
    /// length and that of every part between it and the root.
    pub fn part_reach(&self, defs: &BodyDefs, key: PartKey) -> Real {
        let mut reach = Real::ZERO;
        let mut at = self.part(key);
        for _ in 0..self.parts.len() {
            let Some(part) = at else { break };
            let Some(parent) = part.parent else { break };
            reach += self.part_length(defs, part);
            at = self.part(parent);
        }
        reach
    }

    /// How far the body can reach to strike, in metres: the greatest
    /// reach among its working parts that have a striking surface.
    pub fn reach(&self, defs: &BodyDefs) -> Real {
        let mut reach = Real::ZERO;
        for part in &self.parts {
            let strikes = defs
                .part(part.def)
                .is_some_and(|def| !def.surfaces.is_empty());
            if part.is_working() && strikes {
                reach = reach.max(self.part_reach(defs, part.key));
            }
        }
        reach
    }

    /// The heights above its footing that a part spans, in metres: its
    /// definition's `band` of the body's height.
    pub fn part_band(&self, defs: &BodyDefs, part: &PartInst) -> Option<[Real; 2]> {
        let [low, high] = defs.part(part.def)?.band?;
        let height = self.derived.height;
        Some([low * height, high * height])
    }

    /// The outer parts that are there and whose band holds a height
    /// above the body's footing, in order of key.
    pub fn parts_at_height<'a>(
        &'a self,
        defs: &'a BodyDefs,
        height: Real,
    ) -> impl Iterator<Item = &'a PartInst> {
        self.parts.iter().filter(move |part| {
            self.is_exposed(part.key)
                && self
                    .part_band(defs, part)
                    .is_some_and(|[low, high]| low <= height && height <= high)
        })
    }

    /// The channels a part supplies slots of while it works.
    pub fn part_channels(&self, defs: &BodyDefs, part: &PartInst) -> ChannelSet {
        defs.part(part.def)
            .map_or(ChannelSet::EMPTY, BodyPartDef::channel_set)
    }

    /// What follows from the parts, as cached.
    pub fn derived(&self) -> &Derived {
        &self.derived
    }

    /// The capacity of a function (06 §6.3). Zero for one no part
    /// provides.
    pub fn function(&self, function: BodyFunctionId) -> Real {
        self.derived
            .functions
            .get(function.index())
            .copied()
            .unwrap_or(Real::ZERO)
    }

    /// The channels the working parts supply slots of.
    pub fn channels(&self) -> ChannelSet {
        self.derived.channels
    }

    /// How many slots of each channel the working parts supply, in
    /// order of channel.
    pub fn slots(&self) -> &[SlotCount] {
        &self.derived.slots
    }

    /// How many slots of one channel the working parts supply.
    pub fn slots_of(&self, channel: ChannelId) -> u16 {
        self.derived
            .slots
            .binary_search_by_key(&channel, |slot| slot.channel)
            .map_or(0, |at| self.derived.slots[at].count)
    }

    /// The mass of the parts that are there, in kilograms.
    pub fn mass(&self) -> Real {
        self.derived.mass
    }

    /// How tall the body stands, in metres.
    pub fn height(&self) -> Real {
        self.derived.height
    }

    /// The space the body takes up, in cubic metres.
    pub fn volume(&self) -> Real {
        self.derived.volume
    }

    /// The area its outer parts present, in square metres.
    pub fn cross_section(&self) -> Real {
        self.derived.cross_section
    }

    /// The wounds, in order of id.
    pub fn wounds(&self) -> &[Wound] {
        &self.wounds
    }

    /// A wound by its id.
    pub fn wound(&self, id: WoundId) -> Option<&Wound> {
        self.wounds
            .binary_search_by_key(&id, Wound::id)
            .ok()
            .map(|at| &self.wounds[at])
    }

    /// The wounds on a part.
    pub fn wounds_on(&self, part: PartKey) -> impl Iterator<Item = &Wound> {
        self.wounds.iter().filter(move |wound| wound.part() == part)
    }

    /// The keys of a part and of everything joined to it, directly or
    /// through others, in order of key.
    pub fn subtree(&self, root: PartKey) -> Vec<PartKey> {
        let mut keys = vec![root];
        let mut next = 0;
        while next < keys.len() {
            let parent = keys[next];
            keys.extend(self.children(parent).map(PartInst::key));
            next += 1;
        }
        keys.sort_unstable();
        keys
    }
}

/// Works out everything that follows from a body's parts, trusting no
/// cache. This is the recomputation that [`Body::derived`] must always
/// equal bit for bit (06 invariants B2, B3).
pub fn recompute(body: &Body, defs: &BodyDefs) -> Derived {
    /// What a function's contributions come to so far.
    #[derive(Clone, Copy)]
    struct Tally {
        sum: Real,
        min: Real,
        max: Real,
        count: u32,
    }

    let count = defs.function_count();
    let mut tallies = vec![
        Tally {
            sum: Real::ZERO,
            min: Real::INFINITY,
            max: Real::ZERO,
            count: 0,
        };
        count
    ];
    let mut channels = ChannelSet::EMPTY;
    let mut slots: BTreeMap<ChannelId, u32> = BTreeMap::new();
    let mut mass = Real::ZERO;
    let mut volume = Real::ZERO;
    let mut cross_section = Real::ZERO;
    for part in &body.parts {
        let Some(def) = defs.part(part.def) else {
            continue;
        };
        let working = part.is_working();
        let condition = part.condition();
        for (function, provided) in &def.provides {
            let Some(tally) = tallies.get_mut(function.id().index()) else {
                continue;
            };
            let contribution = if working {
                *provided * condition
            } else {
                Real::ZERO
            };
            tally.sum += contribution;
            tally.min = tally.min.min(contribution);
            tally.max = tally.max.max(contribution);
            tally.count += 1;
        }
        if working {
            for (channel, supplied) in &def.channels {
                channels.insert(channel.id());
                *slots.entry(channel.id()).or_insert(0) += u32::from(*supplied);
            }
        }
        if part.is_present() {
            let part_mass = body.part_mass(defs, part);
            mass += part_mass;
            let mut thickness = Real::ZERO;
            let mut weighed = Real::ZERO;
            for layer in &part.layers {
                let density = defs
                    .tissue(layer.material)
                    .map_or(Real::ZERO, |tissue| tissue.density);
                thickness += layer.thickness;
                weighed += layer.thickness * density;
            }
            if weighed > Real::ZERO {
                // Mass over the mean density of its layers.
                volume += part_mass * thickness / weighed;
            }
            if body.covering(part.key).is_empty() {
                cross_section += body.part_area(defs, part);
            }
        }
    }
    let functions = tallies
        .iter()
        .enumerate()
        .map(|(index, tally)| {
            if tally.count == 0 {
                return Real::ZERO;
            }
            let combine = BodyFunctionId::from_index(index)
                .and_then(|id| defs.function(id))
                .map_or(Combine::Sum, |def| def.combine);
            match combine {
                Combine::Sum => tally.sum,
                Combine::Min => tally.min,
                Combine::Max => tally.max,
                Combine::Mean => tally.sum / Real::from_i64(i64::from(tally.count)),
            }
        })
        .collect();
    let height = defs
        .plan(body.plan)
        .map_or(Real::ZERO, |plan| plan.height.get())
        * body.scale
        * body.height_factor;
    Derived {
        functions,
        channels,
        slots: slots
            .into_iter()
            .map(|(channel, count)| SlotCount {
                channel,
                count: u16::try_from(count).unwrap_or(u16::MAX),
            })
            .collect(),
        mass,
        volume,
        height,
        cross_section,
    }
}

/// How a part came off a body.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[repr(u8)]
pub enum Detach {
    /// Cut off by a hit: it leaves a wound where it was, and stays
    /// listed as gone.
    Severed,
    /// Destroyed by a hit: as severed, and the part itself is ruined.
    Destroyed,
    /// Taken off by an edit (06 §6.5): the body no longer has such a
    /// part at all, and nothing is left listed.
    Removed,
}

impl StateHash for Detach {
    fn state_hash(&self, hasher: &mut StateHasher) {
        hasher.write_u8(*self as u8);
    }
}

/// A part that came off a body, with everything that was on it and in
/// it: what the item package makes an item of (06 §7.5), or nothing if
/// the root's definition says it [`Leaves`] nothing.
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub struct Detached {
    /// Whose it was.
    pub from: EntityId,
    /// How it came off.
    pub how: Detach,
    /// The plan of the body it came off.
    pub plan: BodyPlanId,
    /// That body's overall size factor.
    pub scale: Real,
    /// The part that came off: the first of `parts`.
    pub root: PartKey,
    /// The part it was joined to.
    pub parent: Option<PartKey>,
    /// The parts, as they were on the body, in order of key: keys,
    /// names, layers and damage kept.
    pub parts: Vec<PartInst>,
    /// The wounds that were on them.
    pub wounds: Vec<Wound>,
    /// Their mass, in kilograms.
    pub mass: Real,
    /// Who and what took it off.
    pub cause: Cause,
}

impl_state_hash!(Detached {
    from,
    how,
    plan,
    scale,
    root,
    parent,
    parts,
    wounds,
    mass,
    cause
});

/// Something a function changed that the rest of the simulation is told
/// of. The kernel raises an event from each that names one (06 §13).
#[derive(Clone, PartialEq, Debug)]
pub enum Change {
    /// A hit did something to a part: `hit_landed`.
    HitLanded {
        /// Whose body.
        entity: EntityId,
        /// The part struck.
        part: PartKey,
        /// The kind of damage.
        kind: DamageTypeId,
        /// Joules the part's own layers took.
        absorbed: Real,
        /// Joules that went on through it.
        passed: Real,
        /// How many of its layers the hit got through.
        depth: u8,
    },
    /// A hit breached a part, or made a wound it had worse:
    /// `wound_inflicted`.
    Wounded {
        /// Whose body.
        entity: EntityId,
        /// The wound.
        wound: WoundId,
        /// The part it is on.
        part: PartKey,
        /// The kind of damage.
        kind: DamageTypeId,
        /// How many layers it goes through.
        depth: u8,
        /// Its area, in square metres.
        area: Real,
    },
    /// A wound stopped leaking: `wound_closed`.
    WoundClosed {
        /// Whose body.
        entity: EntityId,
        /// The wound.
        wound: WoundId,
        /// The part it is on.
        part: PartKey,
    },
    /// A closed wound was opened again: `wound_reopened`.
    WoundReopened {
        /// Whose body.
        entity: EntityId,
        /// The wound.
        wound: WoundId,
        /// The part it is on.
        part: PartKey,
    },
    /// A wound is gone, its layers whole again: `wound_healed`.
    WoundHealed {
        /// Whose body.
        entity: EntityId,
        /// The id it had.
        wound: WoundId,
        /// The part it was on.
        part: PartKey,
        /// The kind of damage it was.
        kind: DamageTypeId,
    },
    /// A part stopped working: `part_disabled`.
    PartDisabled {
        /// Whose body.
        entity: EntityId,
        /// The part.
        part: PartKey,
    },
    /// A part works again: `part_restored`.
    PartRestored {
        /// Whose body.
        entity: EntityId,
        /// The part.
        part: PartKey,
    },
    /// A part came off: `part_severed`. Raised for the part cut, not for
    /// what went with it.
    PartSevered {
        /// Whose body.
        entity: EntityId,
        /// The part.
        part: PartKey,
        /// The part it was on.
        parent: Option<PartKey>,
    },
    /// A part was destroyed: `part_destroyed`.
    PartDestroyed {
        /// Whose body.
        entity: EntityId,
        /// The part.
        part: PartKey,
    },
    /// Channel slots are gone with a part that stopped working or came
    /// off. An action that held one of them is to be interrupted
    /// (03 §6.2). No event of its own.
    SlotsLost {
        /// Whose body.
        entity: EntityId,
        /// The part that supplied them.
        part: PartKey,
        /// The channels.
        channels: ChannelSet,
        /// How many slots of each, in order of channel.
        slots: Vec<SlotCount>,
    },
    /// Channel slots are back with a part that works again. No event of
    /// its own.
    SlotsGained {
        /// Whose body.
        entity: EntityId,
        /// The part that supplies them.
        part: PartKey,
        /// The channels.
        channels: ChannelSet,
        /// How many slots of each, in order of channel.
        slots: Vec<SlotCount>,
    },
    /// A function's capacity moved: `function_changed`.
    FunctionChanged {
        /// Whose body.
        entity: EntityId,
        /// Which function.
        function: BodyFunctionId,
        /// Its capacity before.
        old: Real,
        /// Its capacity now.
        new: Real,
    },
    /// The mass of the body's owner, or of something that holds it,
    /// changed. The world's occupant record is told.
    MassChanged(MassChange),
    /// The body is in pieces: its root part was destroyed (06 §7.5).
    /// Whether that ends the entity is not decided here.
    BodyDestroyed {
        /// Whose body.
        entity: EntityId,
    },
    /// A pool or an attribute changed because of this: a leak crossed a
    /// threshold, a signal was added.
    Stats(stats::Change),
}

/// What changed, in the order it happened.
pub type Changes = Vec<Change>;

/// The stable codes of the reasons this module refuses for (14 §5). Each
/// is the code of one [`BodyError`]; the sentence is the error's message
/// and the values are [`BodyError::values`].
pub mod reason {
    /// The entity has no body. Values: `entity`.
    pub const NO_BODY: &str = "no_body";
    /// The entity already has a body. Values: `entity`.
    pub const HAS_BODY: &str = "has_body";
    /// The id is not a plan of the loaded definitions, or the plan
    /// cannot be laid out. Values: none.
    pub const UNKNOWN_PLAN: &str = "unknown_plan";
    /// The body has no part with this key or name. Values: `entity`,
    /// `part`.
    pub const NO_SUCH_PART: &str = "no_such_part";
    /// The part is gone: severed or destroyed. Values: `entity`, `part`.
    pub const PART_MISSING: &str = "part_missing";
    /// The part is the body's root, which cannot be taken off. Values:
    /// `entity`, `part`.
    pub const ROOT_PART: &str = "root_part";
    /// The body has no wound with this id. Values: `entity`, `wound`.
    pub const NO_SUCH_WOUND: &str = "no_such_wound";
    /// The wound is not open. Values: `entity`, `wound`.
    pub const WOUND_NOT_OPEN: &str = "wound_not_open";
    /// The wound is not closed. Values: `entity`, `wound`.
    pub const WOUND_NOT_CLOSED: &str = "wound_not_closed";
    /// A scale, a share or a factor was not a finite number above zero.
    /// Values: `amount`.
    pub const BAD_AMOUNT: &str = "bad_amount";
    /// A hit's energy, intensity, area, span, height or direction was
    /// negative or not finite. Values: `what`.
    pub const BAD_HIT: &str = "bad_hit";
    /// The timeline refused: the body's clock is gone. Values: none.
    pub const NO_CLOCK: &str = "no_clock";
    /// The entity's size could not be set. Values: none.
    pub const BAD_SHAPE: &str = "bad_shape";
    /// The definitions were loaded without this module's registrations.
    /// Values: none.
    pub const NOT_REGISTERED: &str = "bodies_not_registered";
}

/// Why a change to an entity's body was refused. A refused change has
/// changed nothing.
///
/// Each is a reason in the form of 14 §5: [`code`](BodyError::code) is
/// its stable code, its message the sentence, and
/// [`values`](BodyError::values) the values involved.
#[derive(Debug, Clone, PartialEq, Error)]
#[non_exhaustive]
pub enum BodyError {
    /// The entity has no body.
    #[error("{entity} has no body")]
    NoBody {
        /// The entity.
        entity: EntityId,
    },
    /// The entity already has a body.
    #[error("{entity} already has a body")]
    HasBody {
        /// The entity.
        entity: EntityId,
    },
    /// The id is not a plan that can be built from.
    #[error("there is no body plan with this id that a body can be built from")]
    UnknownPlan,
    /// The body has no such part.
    #[error("{entity} has no part {part}")]
    NoSuchPart {
        /// The entity.
        entity: EntityId,
        /// The key asked for.
        part: PartKey,
    },
    /// The part is gone.
    #[error("the part `{name}` of {entity} is gone")]
    PartMissing {
        /// The entity.
        entity: EntityId,
        /// The part.
        part: PartKey,
        /// Its name.
        name: String,
    },
    /// The part is the root.
    #[error("the part `{name}` is the root of the body of {entity} and cannot be taken off")]
    RootPart {
        /// The entity.
        entity: EntityId,
        /// The part.
        part: PartKey,
        /// Its name.
        name: String,
    },
    /// The body has no such wound.
    #[error("{entity} has no wound {wound}")]
    NoSuchWound {
        /// The entity.
        entity: EntityId,
        /// The id asked for.
        wound: WoundId,
    },
    /// The wound is not open.
    #[error("the wound {wound} of {entity} is not open")]
    WoundNotOpen {
        /// The entity.
        entity: EntityId,
        /// The wound.
        wound: WoundId,
    },
    /// The wound is not closed.
    #[error("the wound {wound} of {entity} is not closed")]
    WoundNotClosed {
        /// The entity.
        entity: EntityId,
        /// The wound.
        wound: WoundId,
    },
    /// A scale, a share or a factor was not a finite number above zero.
    #[error("{amount} cannot be used here: it must be finite and above zero")]
    BadAmount {
        /// What was given.
        amount: Real,
    },
    /// A hit carried a number that is negative or not finite.
    #[error("a hit's {what} must be finite and not negative")]
    BadHit {
        /// Which number: "energy", "intensity", "area", "span",
        /// "height", "direction".
        what: &'static str,
    },
    /// The timeline refused: the body's clock is gone.
    #[error("the timeline refused: {0}")]
    Time(#[from] TimeError),
    /// The entity's size could not be set.
    #[error("the body's size could not be set: {0}")]
    Physical(#[from] PhysicalError),
    /// The stats refused something other than a missing pool.
    #[error("the stats refused: {0}")]
    Stats(#[from] StatsError),
    /// The definitions were loaded without this module's registrations.
    #[error("bodies are not registered: the definitions lack {what}")]
    NotRegistered {
        /// What is missing.
        what: &'static str,
    },
}

impl BodyError {
    /// The reason's stable code: one of [`reason`], or the stats' own
    /// for a refusal passed on from them.
    pub const fn code(&self) -> &'static str {
        match self {
            BodyError::NoBody { .. } => reason::NO_BODY,
            BodyError::HasBody { .. } => reason::HAS_BODY,
            BodyError::UnknownPlan => reason::UNKNOWN_PLAN,
            BodyError::NoSuchPart { .. } => reason::NO_SUCH_PART,
            BodyError::PartMissing { .. } => reason::PART_MISSING,
            BodyError::RootPart { .. } => reason::ROOT_PART,
            BodyError::NoSuchWound { .. } => reason::NO_SUCH_WOUND,
            BodyError::WoundNotOpen { .. } => reason::WOUND_NOT_OPEN,
            BodyError::WoundNotClosed { .. } => reason::WOUND_NOT_CLOSED,
            BodyError::BadAmount { .. } => reason::BAD_AMOUNT,
            BodyError::BadHit { .. } => reason::BAD_HIT,
            BodyError::Time(_) => reason::NO_CLOCK,
            BodyError::Physical(_) => reason::BAD_SHAPE,
            BodyError::Stats(error) => error.code(),
            BodyError::NotRegistered { .. } => reason::NOT_REGISTERED,
        }
    }

    /// The values involved, by name, as the expression language holds
    /// them. An entity is given as its id's bits, a part by its name
    /// where it has one and else its key, a wound by its id.
    pub fn values(&self) -> Vec<(&'static str, Value)> {
        let bits = |entity: &EntityId| Value::Int(entity.to_bits() as i64);
        match self {
            BodyError::NoBody { entity } | BodyError::HasBody { entity } => {
                vec![("entity", bits(entity))]
            }
            BodyError::NoSuchPart { entity, part } => vec![
                ("entity", bits(entity)),
                ("part", Value::Int(i64::from(part.raw()))),
            ],
            BodyError::PartMissing { entity, name, .. }
            | BodyError::RootPart { entity, name, .. } => {
                vec![("entity", bits(entity)), ("part", Value::text(name))]
            }
            BodyError::NoSuchWound { entity, wound }
            | BodyError::WoundNotOpen { entity, wound }
            | BodyError::WoundNotClosed { entity, wound } => vec![
                ("entity", bits(entity)),
                ("wound", Value::Int(i64::from(wound.raw()))),
            ],
            BodyError::BadAmount { amount } => vec![("amount", Value::Real(*amount))],
            BodyError::BadHit { what } => vec![("what", Value::text(what))],
            BodyError::Stats(error) => error.values(),
            BodyError::UnknownPlan
            | BodyError::Time(_)
            | BodyError::Physical(_)
            | BodyError::NotRegistered { .. } => Vec::new(),
        }
    }
}

/// The body of an entity, or the refusal that it has none.
pub(super) fn body_of(bodies: &BodyStore, entity: EntityId) -> Result<&Body, BodyError> {
    bodies.get(entity).ok_or(BodyError::NoBody { entity })
}

/// The body of an entity to change, or the refusal that it has none.
pub(super) fn body_mut(bodies: &mut BodyStore, entity: EntityId) -> Result<&mut Body, BodyError> {
    bodies.get_mut(entity).ok_or(BodyError::NoBody { entity })
}

/// A part that is there, or the refusal that says why not.
pub(super) fn present_part(
    body: &Body,
    entity: EntityId,
    part: PartKey,
) -> Result<&PartInst, BodyError> {
    let found = body
        .part(part)
        .ok_or(BodyError::NoSuchPart { entity, part })?;
    if !found.is_present() {
        return Err(BodyError::PartMissing {
            entity,
            part,
            name: found.name.clone(),
        });
    }
    Ok(found)
}

/// What one call has done so far, beyond the body itself.
#[derive(Default)]
pub(super) struct Work {
    /// What to report.
    pub(super) changes: Changes,
    /// Wounds that are no longer on the body, whose leaks the stats
    /// still hold.
    pub(super) gone: Vec<WoundId>,
}

/// The slots a part supplies while it works, as a change reports them.
fn slots_of_part(defs: &BodyDefs, part: &PartInst) -> (ChannelSet, Vec<SlotCount>) {
    let slots: Vec<SlotCount> = defs
        .channels_of(part.def)
        .map(|(channel, count)| SlotCount { channel, count })
        .collect();
    (slots.iter().map(|slot| slot.channel).collect(), slots)
}

/// Brings every part's state into line with its condition, and the
/// cached derived values into line with the parts, reporting what
/// changed.
pub(super) fn settle(body: &mut Body, defs: &BodyDefs, entity: EntityId, out: &mut Changes) {
    for at in 0..body.parts.len() {
        let part = &body.parts[at];
        if !part.is_present() {
            continue;
        }
        let Some(def) = defs.part(part.def) else {
            continue;
        };
        let wanted = if part.held || part.condition() < def.disable_below {
            PartState::Disabled
        } else {
            PartState::Whole
        };
        if wanted == part.state {
            continue;
        }
        let key = part.key;
        let (channels, slots) = slots_of_part(defs, part);
        body.parts[at].state = wanted;
        if wanted == PartState::Disabled {
            out.push(Change::PartDisabled { entity, part: key });
            if !slots.is_empty() {
                out.push(Change::SlotsLost {
                    entity,
                    part: key,
                    channels,
                    slots,
                });
            }
        } else {
            out.push(Change::PartRestored { entity, part: key });
            if !slots.is_empty() {
                out.push(Change::SlotsGained {
                    entity,
                    part: key,
                    channels,
                    slots,
                });
            }
        }
    }
    let derived = recompute(body, defs);
    for (index, new) in derived.functions.iter().enumerate() {
        let old = body
            .derived
            .functions
            .get(index)
            .copied()
            .unwrap_or(Real::ZERO);
        if old != *new
            && let Some(function) = BodyFunctionId::from_index(index)
        {
            out.push(Change::FunctionChanged {
                entity,
                function,
                old,
                new: *new,
            });
        }
    }
    body.derived = derived;
}

/// Hands the body's mass and size to the entity's `Physical`, if they
/// are not what it holds.
fn push_shape<H: Host + ?Sized>(
    cx: &mut BodyCx<'_, H>,
    entity: EntityId,
    out: &mut Changes,
) -> Result<(), BodyError> {
    let body = body_of(cx.bodies, entity)?;
    let existing = cx.physical.get(entity).map(Physical::shape);
    let footprint = existing.map_or_else(
        || {
            cx.defs
                .plan(body.plan)
                .map_or(Footprint::SINGLE, BodyPlanDef::footprint)
        },
        |shape| shape.footprint,
    );
    let shape = Shape {
        mass: Mass::new(body.derived.mass),
        volume: Volume::new(body.derived.volume),
        height: Length::new(body.derived.height),
        footprint,
        cross_section: body.derived.cross_section,
    };
    if existing == Some(shape) {
        return Ok(());
    }
    let changes = physical::set_shape(cx.physical, cx.containment, entity, shape)?;
    out.extend(changes.into_iter().map(Change::MassChanged));
    Ok(())
}

/// What every function that changed a body ends with: wounds whose time
/// has come close and heal, states and derived values are settled, the
/// leaks the stats hold are brought into line, the size is handed on,
/// and the next wake-up is scheduled.
pub(super) fn finish<H: Host + ?Sized>(
    cx: &mut BodyCx<'_, H>,
    entity: EntityId,
    work: &mut Work,
) -> Result<(), BodyError> {
    let defs = cx.defs;
    let body = body_mut(cx.bodies, entity)?;
    wound::advance(body, defs, Dur::ZERO, entity, work);
    settle(body, defs, entity, &mut work.changes);
    wound::reconcile_leaks(cx, entity, work)?;
    push_shape(cx, entity, &mut work.changes)?;
    wound::reschedule(cx, entity)
}

/// Gives an entity a body built from a plan (06 §6.2, §6.4), at a scale,
/// and hands its mass and size to its `Physical`. `clock` is the
/// entity's clock, which its wounds will close and heal on.
///
/// Refused if the entity already has a body, if the plan is not one a
/// body can be built from, or if the scale is not above zero.
pub fn attach<H: Host + ?Sized>(
    cx: &mut BodyCx<'_, H>,
    entity: EntityId,
    plan: BodyPlanId,
    scale: Real,
    clock: ClockId,
) -> Result<Changes, BodyError> {
    if cx.bodies.contains(entity) {
        return Err(BodyError::HasBody { entity });
    }
    if !(scale.is_finite() && scale > Real::ZERO) {
        return Err(BodyError::BadAmount { amount: scale });
    }
    let defs = cx.defs;
    let layout = defs.layout(plan).ok_or(BodyError::UnknownPlan)?;
    let synced = cx.stats.timeline.local_now(clock)?;
    let key = |index: usize| PartKey::from_index(index).ok_or(BodyError::UnknownPlan);
    let mut parts = Vec::with_capacity(layout.parts.len());
    for (index, planned) in layout.parts.iter().enumerate() {
        let def = defs.part(planned.def).ok_or(BodyError::UnknownPlan)?;
        let size = scale * planned.scale;
        let layers = def
            .layers
            .iter()
            .zip(def.weights())
            .map(|(layer, weight)| LayerInst {
                material: layer.material.id(),
                thickness: layer.thickness.get() * size,
                weight: *weight,
                damage: 0,
                progress: Dur::ZERO,
            })
            .collect();
        let (parent, relation) = match planned.parent {
            Some((parent, relation)) => (Some(key(parent)?), relation),
            None => (None, Relation::On),
        };
        parts.push(PartInst {
            key: key(index)?,
            name: planned.name.clone(),
            def: planned.def,
            parent,
            relation,
            socket: planned.socket.clone(),
            side: planned.side.clone(),
            scale: planned.scale,
            layers,
            state: PartState::Whole,
            held: false,
        });
    }
    let mut body = Body::new(plan, scale, clock);
    body.next_part = u16::try_from(parts.len()).map_err(|_| BodyError::UnknownPlan)?;
    body.parts = parts;
    body.synced = synced;
    // Nothing is reported of the parts: there was no body before.
    settle(&mut body, defs, entity, &mut Changes::new());
    cx.bodies.insert(entity, body);
    let mut out = Changes::new();
    if let Err(error) = push_shape(cx, entity, &mut out) {
        cx.bodies.remove(entity);
        return Err(error);
    }
    Ok(out)
}

/// Runs a change to one body between the bookkeeping every change
/// needs: closing and healing are brought up to date first, and
/// [`finish`] runs afterwards.
pub(super) fn change<H: Host + ?Sized>(
    cx: &mut BodyCx<'_, H>,
    entity: EntityId,
    make: impl FnOnce(&mut Body, &BodyDefs, &mut Work) -> Result<(), BodyError>,
) -> Result<Changes, BodyError> {
    let mut work = Work::default();
    wound::sync(cx, entity, &mut work)?;
    let defs = cx.defs;
    make(body_mut(cx.bodies, entity)?, defs, &mut work)?;
    finish(cx, entity, &mut work)?;
    Ok(work.changes)
}

/// Disables a part from outside, whatever its condition: a rule, a
/// trait, a grip that pins it. It contributes nothing and its slots are
/// gone until [`restore`].
///
/// Refused if the part is not there. Disabling one already disabled this
/// way changes nothing.
pub fn disable<H: Host + ?Sized>(
    cx: &mut BodyCx<'_, H>,
    entity: EntityId,
    part: PartKey,
) -> Result<Changes, BodyError> {
    present_part(body_of(cx.bodies, entity)?, entity, part)?;
    change(cx, entity, |body, _, _| {
        if let Some(found) = body.part_mut(part) {
            found.held = true;
        }
        Ok(())
    })
}

/// Takes back what [`disable`] did. The part works again if its
/// condition allows.
pub fn restore<H: Host + ?Sized>(
    cx: &mut BodyCx<'_, H>,
    entity: EntityId,
    part: PartKey,
) -> Result<Changes, BodyError> {
    present_part(body_of(cx.bodies, entity)?, entity, part)?;
    change(cx, entity, |body, _, _| {
        if let Some(found) = body.part_mut(part) {
            found.held = false;
        }
        Ok(())
    })
}

/// Says what a posture makes of the body's height (06 §11.1): a factor
/// on the plan's, and with it on every part's band.
pub fn set_height_factor<H: Host + ?Sized>(
    cx: &mut BodyCx<'_, H>,
    entity: EntityId,
    factor: Real,
) -> Result<Changes, BodyError> {
    if !(factor.is_finite() && factor > Real::ZERO) {
        return Err(BodyError::BadAmount { amount: factor });
    }
    body_of(cx.bodies, entity)?;
    change(cx, entity, |body, _, _| {
        body.height_factor = factor;
        Ok(())
    })
}

/// Takes a part off a body with everything on it and in it, and
/// returns it. Shared by [`detach`] and by hits.
///
/// With [`Detach::Removed`] the parts are no longer listed. Otherwise
/// each stays listed as gone: the part itself as `how` says, what went
/// with it as severed.
pub(super) fn take_off(
    body: &mut Body,
    defs: &BodyDefs,
    entity: EntityId,
    root: PartKey,
    how: Detach,
    cause: Cause,
    work: &mut Work,
) -> Option<Detached> {
    let parent = body.part(root)?.parent;
    let keys: Vec<PartKey> = body
        .subtree(root)
        .into_iter()
        .filter(|key| body.part(*key).is_some_and(PartInst::is_present))
        .collect();
    if keys.is_empty() {
        return None;
    }
    let mut parts = Vec::with_capacity(keys.len());
    let mut mass = Real::ZERO;
    for key in &keys {
        let Some(part) = body.part(*key) else {
            continue;
        };
        mass += body.part_mass(defs, part);
        if part.is_working() {
            let (channels, slots) = slots_of_part(defs, part);
            if !slots.is_empty() {
                work.changes.push(Change::SlotsLost {
                    entity,
                    part: *key,
                    channels,
                    slots,
                });
            }
        }
        parts.push(part.clone());
    }
    // The part that came off leads; the rest follow in order of key.
    parts.sort_by_key(|part| (part.key != root, part.key));
    let (taken, kept): (Vec<Wound>, Vec<Wound>) = core::mem::take(&mut body.wounds)
        .into_iter()
        .partition(|wound| keys.contains(&wound.part()));
    body.wounds = kept;
    work.gone.extend(taken.iter().map(Wound::id));
    if how == Detach::Removed {
        body.parts.retain(|part| !keys.contains(&part.key));
    } else {
        for part in &mut body.parts {
            if !keys.contains(&part.key) {
                continue;
            }
            part.layers.clear();
            part.held = false;
            part.state = if part.key == root && how == Detach::Destroyed {
                PartState::Destroyed
            } else {
                PartState::Severed
            };
        }
    }
    Some(Detached {
        from: entity,
        how,
        plan: body.plan,
        scale: body.scale,
        root,
        parent,
        parts,
        wounds: taken,
        mass,
        cause,
    })
}

/// Takes a part off a body with everything on it and in it, and returns
/// it as data for whoever makes an item of it (06 §6.5 `remove_part`;
/// 06 §7.5).
///
/// With [`Detach::Severed`] or [`Detach::Destroyed`] the parts stay
/// listed as gone and still count, as nothing, toward the body's
/// functions; no wound is made (a hit that severs makes one). With
/// [`Detach::Removed`] the body simply no longer has them.
///
/// Refused for a part that is not there, and for the root.
pub fn detach<H: Host + ?Sized>(
    cx: &mut BodyCx<'_, H>,
    entity: EntityId,
    part: PartKey,
    how: Detach,
    cause: Cause,
) -> Result<(Detached, Changes), BodyError> {
    let found = present_part(body_of(cx.bodies, entity)?, entity, part)?;
    if found.parent.is_none() {
        return Err(BodyError::RootPart {
            entity,
            part,
            name: found.name.clone(),
        });
    }
    let parent = found.parent;
    let mut detached = None;
    let changes = change(cx, entity, |body, defs, work| {
        detached = take_off(body, defs, entity, part, how, cause, work);
        match how {
            Detach::Severed => work.changes.push(Change::PartSevered {
                entity,
                part,
                parent,
            }),
            Detach::Destroyed => work.changes.push(Change::PartDestroyed { entity, part }),
            Detach::Removed => {}
        }
        Ok(())
    })?;
    // The part was there when asked, so there is something to return.
    let detached = detached.ok_or(BodyError::NoSuchPart { entity, part })?;
    Ok((detached, changes))
}
