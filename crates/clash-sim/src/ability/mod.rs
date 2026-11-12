//! Abilities (08 §2 to §6; packages A1 to A4): what a character has and
//! can use at a cost, that a source grants, that ranks up, that has a
//! cooldown, that is switched on and off.
//!
//! The engine names no ability and gives none a meaning. A skill, a
//! spell, a superpower, a cheat and a powered suit's repulsors are the
//! same kind of definition ([`AbilityDef`]): actions it makes available
//! (grants), with costs and cooldowns; what it confers while active;
//! modes with an upkeep; a rank and parameters. What any of it does is
//! the actions, traits and verbs it names (invariant A1).
//!
//! | Part | Is | Module |
//! |------|----|--------|
//! | [`AbilityDef`] and its parts | The `ability` definition type | `defs` |
//! | [`Abilities`], [`AbilityInst`], [`AbilitySource`], [`State`] | The component: the instances an entity holds, from which sources | here |
//! | [`AbilityIndex`] | Which entity holds each instance, by its number: state of the simulation | here |
//! | [`grant`], [`revoke`], [`lose_source`], [`set_rank`] | Holding an ability | `ops` |
//! | [`use_ability`], [`available`], [`options`] | Using one: costs, cooldowns, `ability_using` | `using` |
//! | [`start_mode`], [`end_mode`] | Modes and their upkeep | `modes` |
//! | [`reactive`] | Expressions kept current (05 §7.5): `while` of abilities and traits | `reactive` |
//! | [`properties`], [`verbs()`], [`ABILITY_EVENTS`], [`hook_types`], [`ABILITY_REASONS`] | What rules read, call, listen for and are refused with | `props`, `verbs`, here |
//!
//! # Holding
//!
//! An entity holds an ability while any **source** of it remains: a
//! template it was made from, a trait it has that lists the ability
//! (only while that trait is not dormant), or a rule (`grant_ability`).
//! One ability from several sources is one instance, with one rank,
//! one set of cooldowns and modes (invariant A8). Each instance has a
//! **number** that no other instance of the simulation ever has; an
//! expression's `ability` is a handle of the kind `ability` whose bits
//! are that number, so a handle never names another instance.
//! `revoke_ability` takes an instance away whatever its sources; it
//! stays revoked while they remain, until a rule grants it again.
//!
//! While **active**, an instance confers its `always` (traits with the
//! instance as giver, attribute modifiers under it as a stats source,
//! parameter modifiers) and attaches its `rules`. While its `while`
//! does not hold it is **dormant**: it confers nothing, its modes end
//! and its grants cannot be used (08 §6.1).
//!
//! # Rank and parameters
//!
//! Rank is the level of the ability's track, or with no track a number
//! set when it is granted and by `set_rank`. A parameter is its
//! expression, read with the owner as `self` and the instance as
//! `ability`, then modified: `(value + Σ add) × Π mul` over the
//! parameter modifiers of the owner's active traits (their
//! `ability_params`, once for each stack) and active abilities (their
//! `always.params`), each naming the ability or one of its tags.
//!
//! # Using
//!
//! A grant makes an action available. [`use_ability`] checks the
//! instance (held, active, rank, cooldown, condition, cost), raises the
//! hook `ability_using`, which may change the cost and the parameters
//! or cancel, prepares the action through the action runtime, takes the
//! cost in full (invariant A4), starts the cooldown on the owner's own
//! time, starts the action and raises `ability_used`. A grant's `params`
//! are the ability's numbers handed to the action as its parameters, so
//! one action serves every rank.
//!
//! # Modes
//!
//! Switching a mode on gives its trait, applies its upkeep as pool
//! rates under the instance and runs `on_start`; switching it off takes
//! both away together and runs `on_end` (invariant A5). The moment the
//! first pool of its upkeep runs dry is worked out from the pool's
//! line and scheduled once on the owner's clock; it is worked out again
//! whenever something tells the owner's pools moved (a crossing, a
//! trait, an attribute, a cost). Nothing runs per tick.
//!
//! # What the kernel calls
//!
//! | When | The kernel |
//! |------|-----------|
//! | Building its definition types | Calls [`register_defs`] |
//! | Building its registry | Calls [`register_types`], lists [`properties`] and [`verbs()`] and mirrors [`ABILITY_EVENTS`] |
//! | Building its event bus | Enters [`ABILITY_EVENTS`] and [`hook_types`] |
//! | An event has been delivered | Calls [`on_event`] |
//! | The event queue is empty at the end of a pump | Calls `reactive::settle` |
//! | A wake-up of the domain `Timer` pops | Hands it to [`handle_wake`] first |
//! | A variable is written | Calls `reactive::var_written` |
//! | A property of a handle is read | Asks [`property`] first |
//! | A template has made an entity | Calls [`grant_from_template`] |

mod defs;
mod modes;
mod ops;
mod props;
pub mod reactive;
mod using;
pub mod verbs;

use std::collections::BTreeMap;
use std::sync::Arc;

use clash_core::{Dur, EntityId, impl_state_hash};
use clash_data::{CrossCx, Def, DefLabel, DefTypes, Defs, DefsView, Diagnostic, RegistryCx, code};
use clash_expr::{
    EventSpec, Handle, KindId, KindSpec, PropertyId, PropertySpec, Registry, Type, Value,
};
use clash_time::{Wake, WakeDomain, WakeId};
use serde::{Deserialize, Serialize};

use crate::Sim;
use crate::action::ActionDefId;
use crate::bus::{EventType, FieldType};
use crate::event::{CAUSE_SUBJECTS, EventInfo, SimEvent};
use crate::item::ItemDef;
use crate::reason::ReasonInfo;
use crate::rule::EocId;
use crate::stats::PoolDef;
use crate::template::TemplateDef;
use crate::traits::{TraitDef, TraitId};

pub use defs::{
    ALWAYS_SLOT, AbilityDef, AbilityGrantDef, AbilityId, Always, AlwaysDef, CostDef, Costs,
    DEFAULT_RANK, Intent, MAX_MODES, MAX_RANK, ModeDef, OverdrawDef, ParamModifierDef,
    RESERVED_PARAMS,
};
pub use modes::{end_mode, start_mode};
pub use ops::{
    grant, grant_from_template, lose_source, param, rank, revoke, set_rank, sync_trait_sources,
};
pub use props::{properties, property};
pub use using::{CostLine, UseOption, available, options, use_ability};
pub use verbs::verbs;

/// The kind of subject an ability instance is in expressions (05 §5.1).
pub const ABILITY_KIND: &str = "ability";

// ---- Instances -----------------------------------------------------------------

/// What gave an entity an ability (08 §3.3).
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AbilitySource {
    /// A template the entity was made from: it lasts until revoked.
    Template,
    /// A trait instance of the entity that lists the ability, while the
    /// trait is not dormant.
    Trait(crate::traits::TraitKey),
    /// A rule, through `grant_ability`, or code acting as one.
    Rule,
}

impl AbilitySource {
    /// The word rules read: `template`, `trait` or `rule`.
    pub const fn name(self) -> &'static str {
        match self {
            AbilitySource::Template => "template",
            AbilitySource::Trait(_) => "trait",
            AbilitySource::Rule => "rule",
        }
    }
}

impl clash_core::StateHash for AbilitySource {
    fn state_hash(&self, hasher: &mut clash_core::StateHasher) {
        match self {
            AbilitySource::Template => hasher.write_u8(0),
            AbilitySource::Trait(key) => {
                hasher.write_u8(1);
                hasher.write_u32(*key);
            }
            AbilitySource::Rule => hasher.write_u8(2),
        }
    }
}

/// Whether an instance confers what it lists (08 §6).
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[repr(u8)]
pub enum State {
    /// Its `while` holds: it confers, and its grants and modes can be
    /// used.
    Active,
    /// Its `while` does not hold: it confers nothing.
    Dormant,
}

impl State {
    /// The word rules read: `active` or `dormant`.
    pub const fn name(self) -> &'static str {
        match self {
            State::Active => "active",
            State::Dormant => "dormant",
        }
    }
}

impl clash_core::StateHash for State {
    fn state_hash(&self, hasher: &mut clash_core::StateHasher) {
        hasher.write_u8(*self as u8);
    }
}

/// A mode that is on (08 §5.4).
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub struct ModeOn {
    /// Which mode of the definition.
    pub mode: u16,
    /// The wake-up for the moment its upkeep runs a pool dry, if one is
    /// ahead.
    pub wake: Option<WakeId>,
}

impl_state_hash!(ModeOn { mode, wake });

/// A grant waiting out its cooldown: the owner's own time at which it
/// can be used again (03 §3.4). Own time stands still while the owner
/// is out of the world, and so does the cooldown.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub struct Cooldown {
    /// The action of the grant.
    pub action: ActionDefId,
    /// When it is ready, in the owner's own time.
    pub ready_at: Dur,
}

impl_state_hash!(Cooldown { action, ready_at });

/// One ability an entity holds (08 §3.2).
#[derive(Clone, PartialEq, Debug, Serialize, Deserialize)]
pub struct AbilityInst {
    number: u64,
    def: AbilityId,
    /// What gives it, in the order they gave it.
    sources: Vec<AbilitySource>,
    /// Taken away by `revoke_ability`: it confers nothing and is not
    /// held, though sources remain.
    revoked: bool,
    /// The rank set directly, for an ability with no track.
    rank: u32,
    state: State,
    modes: Vec<ModeOn>,
    cooldowns: Vec<Cooldown>,
    /// The entries of the definition's `always` applied now, in order.
    applied: Vec<u16>,
}

impl_state_hash!(AbilityInst {
    number,
    def,
    sources,
    revoked,
    rank,
    state,
    modes,
    cooldowns,
    applied
});

impl AbilityInst {
    /// Its number, which no other instance of the simulation has.
    pub fn number(&self) -> u64 {
        self.number
    }

    /// Which ability.
    pub fn def(&self) -> AbilityId {
        self.def
    }

    /// What gives it.
    pub fn sources(&self) -> &[AbilitySource] {
        &self.sources
    }

    /// Whether `revoke_ability` took it away while sources remain.
    pub fn is_revoked(&self) -> bool {
        self.revoked
    }

    /// Whether it confers what it lists.
    pub fn state(&self) -> State {
        self.state
    }

    /// The rank set directly. An ability with a track ranks by the
    /// track: see [`rank`].
    pub fn own_rank(&self) -> u32 {
        self.rank
    }

    /// The modes that are on.
    pub fn modes_on(&self) -> &[ModeOn] {
        &self.modes
    }

    /// Whether a mode is on.
    pub fn mode_is_on(&self, mode: usize) -> bool {
        self.modes.iter().any(|on| usize::from(on.mode) == mode)
    }

    /// The grants waiting out a cooldown.
    pub fn cooldowns(&self) -> &[Cooldown] {
        &self.cooldowns
    }

    /// Whether it is held: not revoked.
    pub fn is_held(&self) -> bool {
        !self.revoked
    }

    /// Whether it confers: held and active.
    pub fn confers(&self) -> bool {
        !self.revoked && self.state == State::Active
    }
}

/// The abilities of one entity (08 §3.2): the `Abilities` component.
///
/// Its fields change only through the functions of this module. It is
/// saved and hashed.
#[derive(Clone, PartialEq, Debug, Default, Serialize, Deserialize)]
pub struct Abilities {
    insts: Vec<AbilityInst>,
    /// The rules attached to the entity only because of its abilities.
    rules: Vec<EocId>,
}

impl_state_hash!(Abilities { insts, rules });

impl Abilities {
    /// Every instance, held or revoked, in the order they were made.
    pub fn instances(&self) -> &[AbilityInst] {
        &self.insts
    }

    /// The instance of an ability, if there is one.
    pub fn of(&self, def: AbilityId) -> Option<&AbilityInst> {
        self.insts.iter().find(|inst| inst.def == def)
    }

    /// The instance with a number.
    pub fn by_number(&self, number: u64) -> Option<&AbilityInst> {
        self.insts.iter().find(|inst| inst.number == number)
    }

    /// Whether the entity holds an ability: it has an instance that is
    /// not revoked, dormant or not.
    pub fn holds(&self, def: AbilityId) -> bool {
        self.of(def).is_some_and(AbilityInst::is_held)
    }

    /// The rules attached because of abilities.
    pub fn attached_rules(&self) -> &[EocId] {
        &self.rules
    }

    fn by_number_mut(&mut self, number: u64) -> Option<&mut AbilityInst> {
        self.insts.iter_mut().find(|inst| inst.number == number)
    }

    /// An instance with numbers of the test suite's choosing: for the
    /// tests of the component store.
    #[cfg(test)]
    pub(crate) fn sample(seed: u32) -> Abilities {
        Abilities {
            insts: vec![AbilityInst {
                number: u64::from(seed),
                def: AbilityId::new(seed),
                sources: vec![AbilitySource::Rule],
                revoked: false,
                rank: 1,
                state: State::Active,
                modes: Vec::new(),
                cooldowns: Vec::new(),
                applied: Vec::new(),
            }],
            rules: Vec::new(),
        }
    }
}

/// What abilities keep that belongs to no one entity: the number the
/// next instance takes, and which entity holds each instance. Saved and
/// hashed with the simulation.
#[derive(Clone, PartialEq, Debug, Default, Serialize, Deserialize)]
pub struct AbilityIndex {
    next: u64,
    owners: BTreeMap<u64, EntityId>,
}

impl_state_hash!(AbilityIndex { next, owners });

impl AbilityIndex {
    /// The entity that holds the instance with a number.
    pub fn owner(&self, number: u64) -> Option<EntityId> {
        self.owners.get(&number).copied()
    }

    /// How many instances exist.
    pub fn len(&self) -> usize {
        self.owners.len()
    }

    /// Whether no instance exists.
    pub fn is_empty(&self) -> bool {
        self.owners.is_empty()
    }
}

/// The number a giver of traits or a stats source carries for one slot
/// of an instance: a mode's number, or [`ALWAYS_SLOT`] plus the entry
/// of `always`.
pub const fn slot_id(number: u64, slot: u16) -> u64 {
    (number << 16) | slot as u64
}

/// The giver of the traits an entry of an instance's `always` gives.
pub const fn always_giver(number: u64, entry: u16) -> crate::traits::Giver {
    crate::traits::Giver::new(
        crate::traits::GiverKind::Ability,
        slot_id(number, ALWAYS_SLOT + entry),
    )
}

/// The giver of a mode's trait.
pub const fn mode_giver(number: u64, mode: u16) -> crate::traits::Giver {
    crate::traits::Giver::new(crate::traits::GiverKind::Ability, slot_id(number, mode))
}

/// The stats source of the modifiers an entry of `always` applies.
pub const fn always_source(number: u64, entry: u16) -> crate::stats::Source {
    crate::stats::Source::new(
        crate::stats::SourceKind::Ability,
        slot_id(number, ALWAYS_SLOT + entry),
    )
}

/// The stats source of a mode's upkeep.
pub const fn mode_source(number: u64, mode: u16) -> crate::stats::Source {
    crate::stats::Source::new(crate::stats::SourceKind::Ability, slot_id(number, mode))
}

/// The entity that holds the instance with a number, if it still does.
pub(crate) fn owner_of(sim: &Sim, number: u64) -> Option<EntityId> {
    let owner = sim.abilities.owner(number)?;
    sim.components
        .get::<Abilities>(owner)?
        .by_number(number)
        .map(|_| owner)
}

/// The handle of an instance, as expressions hold it; null if the
/// registry has no `ability` kind.
pub fn handle_value(sim: &Sim, number: u64) -> Value {
    match sim.ability_rt.tables.kind {
        Some(kind) => Value::Subject(Handle::new(kind, number)),
        None => Value::Null,
    }
}

/// The number of the instance a value names, if it is a handle of the
/// kind `ability`.
pub fn number_of(sim: &Sim, value: &Value) -> Option<u64> {
    let handle = value.as_subject()?;
    (Some(handle.kind) == sim.ability_rt.tables.kind).then_some(handle.payload)
}

/// An ability definition.
pub(crate) fn def_of(defs: &Defs, id: AbilityId) -> Option<&AbilityDef> {
    defs.try_registry::<AbilityDef>()?.try_get(id)
}

/// The qualified id of an ability, or the empty text.
pub(crate) fn name_of(defs: &Defs, id: AbilityId) -> String {
    defs.try_registry::<AbilityDef>()
        .filter(|registry| registry.try_get(id).is_some())
        .map_or_else(String::new, |registry| registry.name(id).to_owned())
}

/// The definition of a type a name written in data stands for: a
/// qualified id exactly; a bare one in the first namespace, in sorted
/// order, that has it.
pub(crate) fn lookup_named<T: Def>(defs: &Defs, name: &str) -> Option<T::Id> {
    let registry = defs.try_registry::<T>()?;
    if name.contains(':') {
        return registry.lookup(name);
    }
    let mut found: Vec<(&str, T::Id)> = registry
        .iter()
        .map(|(id, _)| (registry.name(id), id))
        .filter(|(qualified, _)| {
            qualified
                .split_once(':')
                .is_some_and(|(_, bare)| bare == name)
        })
        .collect();
    found.sort_unstable_by(|a, b| a.0.cmp(b.0));
    found.first().map(|(_, id)| *id)
}

/// The ability a name written in data stands for.
pub fn lookup(defs: &Defs, name: &str) -> Option<AbilityId> {
    lookup_named::<AbilityDef>(defs, name)
}

// ---- Tables ----------------------------------------------------------------------

/// What a cost is paid with.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum CostKind {
    /// A pool, by its dense id.
    Pool(crate::stats::PoolId),
    /// Items of a definition.
    Item(crate::item::ItemId),
    /// Nothing loaded: the name named nothing.
    Unknown,
}

/// The properties of the kind `ability` the simulation answers.
#[derive(Clone, PartialEq, Eq, Debug)]
pub(crate) enum Prop {
    Rank,
    Owner,
    State,
    Def,
    Id,
    Tags,
    ModeOn,
    Cooldown,
    Param(String),
}

/// What the definitions resolve to, worked out once for a simulation.
/// Not state.
#[derive(Clone, Debug, Default)]
pub struct Tables {
    /// The kind `ability` in the registry.
    pub(crate) kind: Option<KindId>,
    /// The properties of that kind.
    pub(crate) props: BTreeMap<PropertyId, Prop>,
    /// For each ability, for each grant, what each cost is paid with,
    /// in the order of the cost's keys.
    pub(crate) grant_costs: Vec<Vec<Vec<CostKind>>>,
    /// For each ability, for each mode, its trait and what switching it
    /// on is paid with.
    pub(crate) modes: Vec<Vec<(Option<TraitId>, Vec<CostKind>)>>,
    /// For each ability, for each entry of `always`, its traits.
    pub(crate) always_traits: Vec<Vec<Vec<TraitId>>>,
    /// For each trait, the abilities it lists.
    pub(crate) trait_abilities: Vec<Vec<AbilityId>>,
}

fn cost_kinds(defs: &Defs, costs: &Costs) -> Vec<CostKind> {
    costs
        .iter()
        .map(|(name, cost)| {
            if cost.item {
                lookup_named::<ItemDef>(defs, name).map_or(CostKind::Unknown, CostKind::Item)
            } else {
                lookup_named::<PoolDef>(defs, name).map_or(CostKind::Unknown, CostKind::Pool)
            }
        })
        .collect()
}

fn traits_named(defs: &Defs, names: &[String]) -> Vec<TraitId> {
    names
        .iter()
        .filter_map(|name| crate::traits::lookup(defs, name))
        .collect()
}

impl Tables {
    /// The tables for a set of definitions and the registry they were
    /// compiled against.
    pub fn new(defs: &Defs, registry: &Registry) -> Tables {
        let kind = registry.kind_id(ABILITY_KIND);
        let mut props = BTreeMap::new();
        if let Some(kind) = kind {
            for (name, prop) in [
                ("rank", Prop::Rank),
                ("owner", Prop::Owner),
                ("state", Prop::State),
                ("def", Prop::Def),
                ("id", Prop::Id),
                ("tags", Prop::Tags),
                ("mode_on", Prop::ModeOn),
                ("cooldown", Prop::Cooldown),
            ] {
                if let Some(id) = registry.property_id(kind, name) {
                    props.insert(id, prop);
                }
            }
            if let Some(abilities) = defs.try_registry::<AbilityDef>() {
                for (_, def) in abilities.iter() {
                    for name in def.params.keys() {
                        if let Some(id) = registry.property_id(kind, name) {
                            props.entry(id).or_insert_with(|| Prop::Param(name.clone()));
                        }
                    }
                }
            }
        }
        let mut tables = Tables {
            kind,
            props,
            ..Tables::default()
        };
        if let Some(abilities) = defs.try_registry::<AbilityDef>() {
            for (_, def) in abilities.iter() {
                tables.grant_costs.push(
                    def.grants
                        .iter()
                        .map(|grant| cost_kinds(defs, &grant.cost))
                        .collect(),
                );
                tables.modes.push(
                    def.modes
                        .iter()
                        .map(|mode| {
                            let trait_id = mode
                                .trait_name
                                .as_deref()
                                .and_then(|name| crate::traits::lookup(defs, name));
                            (trait_id, cost_kinds(defs, &mode.cost))
                        })
                        .collect(),
                );
                tables.always_traits.push(
                    def.always
                        .0
                        .iter()
                        .map(|always| traits_named(defs, &always.traits))
                        .collect(),
                );
            }
        }
        if let Some(traits) = defs.try_registry::<TraitDef>() {
            for (_, def) in traits.iter() {
                tables.trait_abilities.push(
                    def.abilities
                        .iter()
                        .filter_map(|name| lookup(defs, name))
                        .collect(),
                );
            }
        }
        tables
    }
}

/// What abilities keep that is not state: the tables of the definitions
/// and the expressions kept current. Made with the simulation and again
/// after a load.
#[derive(Clone, Debug, Default)]
pub struct Runtime {
    /// The tables.
    pub tables: Arc<Tables>,
    /// The expressions kept current (05 §7.5).
    pub reactive: reactive::Reactive,
}

impl Runtime {
    /// The runtime for a set of definitions and the registry they were
    /// compiled against.
    pub fn new(defs: &Defs, registry: &Registry) -> Runtime {
        Runtime {
            tables: Arc::new(Tables::new(defs, registry)),
            reactive: reactive::Reactive::default(),
        }
    }
}

impl Sim {
    /// Which entity holds each ability instance.
    pub fn ability_index(&self) -> &AbilityIndex {
        &self.abilities
    }

    /// The expressions kept current (05 §7.5), to read.
    pub fn reactive(&self) -> &reactive::Reactive {
        &self.ability_rt.reactive
    }
}

// ---- Reasons -----------------------------------------------------------------------

/// The codes of this module's reasons. Each is listed in
/// [`ABILITY_REASONS`].
pub mod codes {
    /// The id is not an `ability` definition.
    pub const UNKNOWN: &str = "ability.unknown";
    /// The entity does not hold the ability.
    pub const NOT_HELD: &str = "ability.not_held";
    /// The ability grants no such action, or several and none was named.
    pub const NO_GRANT: &str = "ability.no_grant";
    /// The ability is dormant.
    pub const DORMANT: &str = "ability.dormant";
    /// The ability's rank is below what a grant or mode needs.
    pub const RANK_TOO_LOW: &str = "ability.rank_too_low";
    /// The grant is waiting out its cooldown.
    pub const COOLING_DOWN: &str = "ability.cooling_down";
    /// The grant's `if` does not hold.
    pub const CONDITION: &str = "ability.condition";
    /// A pool cost is more than is there, overdraw included.
    pub const COST_SHORT: &str = "ability.cost_short";
    /// Whoever a cost is drawn `from` is gone or has no such pool.
    pub const COST_SOURCE_GONE: &str = "ability.cost_source_gone";
    /// Fewer items are held than a cost consumes.
    pub const ITEMS_SHORT: &str = "ability.items_short";
    /// A cost names a pool or an item that is not loaded.
    pub const COST_UNKNOWN: &str = "ability.cost_unknown";
    /// The ability has no mode of that id.
    pub const MODE_UNKNOWN: &str = "ability.mode_unknown";
    /// The mode is on already.
    pub const MODE_ON: &str = "ability.mode_on";
    /// The mode is not on.
    pub const MODE_OFF: &str = "ability.mode_off";
    /// A mode's upkeep cannot be met: its pool is empty or missing.
    pub const UPKEEP: &str = "ability.upkeep";
    /// `set_rank` on an ability whose rank is its track's level.
    pub const RANK_FROM_TRACK: &str = "ability.rank_from_track";
    /// A rank that is not a whole number from 0 to the most there is.
    pub const BAD_RANK: &str = "ability.bad_rank";
    /// A verb's argument is missing or of the wrong kind.
    pub const BAD_ARGUMENT: &str = "ability.bad_argument";
    /// The definitions were loaded without the `ability` type.
    pub const NOT_REGISTERED: &str = "ability.not_registered";
}

/// The reasons this module refuses for, for the generated reference.
pub const ABILITY_REASONS: &[ReasonInfo] = &[
    ReasonInfo {
        code: codes::UNKNOWN,
        doc: "The id is not an `ability` definition. Values: `ability`, as given.",
    },
    ReasonInfo {
        code: codes::NOT_HELD,
        doc: "The entity does not hold the ability, or it was revoked. Values: `entity`, `ability`.",
    },
    ReasonInfo {
        code: codes::NO_GRANT,
        doc: "The ability grants no such action, or grants several and none was named. Values: `ability`, `action` (empty when none was named).",
    },
    ReasonInfo {
        code: codes::DORMANT,
        doc: "The ability is dormant: its `while` does not hold (08 §6.1). Values: `ability`.",
    },
    ReasonInfo {
        code: codes::RANK_TOO_LOW,
        doc: "The ability's rank is below the `at_rank` of the grant or mode. Values: `ability`, `rank`, `needed`.",
    },
    ReasonInfo {
        code: codes::COOLING_DOWN,
        doc: "The grant is waiting out its cooldown. Values: `ability`, `action`, `left`: seconds of the owner's own time.",
    },
    ReasonInfo {
        code: codes::CONDITION,
        doc: "The grant's `if` does not hold. Values: `ability`, `action`.",
    },
    ReasonInfo {
        code: codes::COST_SHORT,
        doc: "A cost is more than its pool holds, with whatever overdraw allows (08 §5.2). Values: `entity` (who pays), `pool`, `wanted`, `have`, `short`: what is missing.",
    },
    ReasonInfo {
        code: codes::COST_SOURCE_GONE,
        doc: "A cost drawn `from` something gives no entity that exists and has the pool. Values: `ability`, `pool`.",
    },
    ReasonInfo {
        code: codes::ITEMS_SHORT,
        doc: "Fewer items of a kind are held than a cost consumes. Values: `item`, `wanted`, `have`.",
    },
    ReasonInfo {
        code: codes::COST_UNKNOWN,
        doc: "A cost names a pool or an item that is not loaded. Values: `cost`.",
    },
    ReasonInfo {
        code: codes::MODE_UNKNOWN,
        doc: "The ability has no mode of that id. Values: `ability`, `mode`.",
    },
    ReasonInfo {
        code: codes::MODE_ON,
        doc: "The mode is on already. Values: `ability`, `mode`.",
    },
    ReasonInfo {
        code: codes::MODE_OFF,
        doc: "The mode is not on. Values: `ability`, `mode`.",
    },
    ReasonInfo {
        code: codes::UPKEEP,
        doc: "A mode's upkeep cannot be met: the entity lacks the pool or it is empty. Values: `ability`, `mode`, `pool`.",
    },
    ReasonInfo {
        code: codes::RANK_FROM_TRACK,
        doc: "`set_rank` was asked of an ability whose rank is its track's level: advance the track. Values: `ability`.",
    },
    ReasonInfo {
        code: codes::BAD_RANK,
        doc: "A rank is not a whole number from 0 to 10,000. Values: `rank`.",
    },
    ReasonInfo {
        code: codes::BAD_ARGUMENT,
        doc: "A verb of abilities was given an argument that is missing or of the wrong kind. Values: `arg`, the argument's name.",
    },
    ReasonInfo {
        code: codes::NOT_REGISTERED,
        doc: "The definitions were loaded without the `ability` definition type. Values: none.",
    },
];

// ---- Events --------------------------------------------------------------------------

/// Raised when an entity comes to hold an ability.
pub const ABILITY_GRANTED: &str = "ability_granted";
/// Raised when an entity stops holding an ability.
pub const ABILITY_REVOKED: &str = "ability_revoked";
/// Raised when an ability goes dormant.
pub const ABILITY_DORMANT: &str = "ability_dormant";
/// Raised when a dormant ability is active again.
pub const ABILITY_ACTIVE: &str = "ability_active";
/// Raised when an ability's rank changes.
pub const RANK_CHANGED: &str = "rank_changed";
/// Raised when a mode is switched on.
pub const MODE_STARTED: &str = "mode_started";
/// Raised when a mode ends.
pub const MODE_ENDED: &str = "mode_ended";
/// Raised when a granted action has been started through its ability.
pub const ABILITY_USED: &str = "ability_used";
/// Raised when a use was refused after the hook `ability_using` ran, or
/// asked by a rule.
pub const ABILITY_REFUSED: &str = "ability_refused";
/// The hook raised before a granted action is started.
pub const ABILITY_USING: &str = "ability_using";
/// The hook raised before a rule grants an ability.
pub const ABILITY_GRANTING: &str = "ability_granting";
/// The hook raised before a rule revokes an ability.
pub const ABILITY_REVOKING: &str = "ability_revoking";

const ABILITY_FIELD: (&str, &str, &str) = ("ability", "string", "The qualified id of the ability.");
const MODE_FIELD: (&str, &str, &str) = ("mode", "string", "The mode's id within the ability.");
const ACTION_FIELD: (&str, &str, &str) = ("action", "string", "The qualified id of the action.");

/// The notifications of abilities, entered beside the kernel's own in
/// every simulation's event bus and in the engine's registry. Each is
/// raised as a payload under its name, with the owner as `self`.
pub const ABILITY_EVENTS: &[EventInfo] = &[
    EventInfo {
        name: ABILITY_GRANTED,
        subjects: &[("self", "entity")],
        fields: &[
            ABILITY_FIELD,
            (
                "source",
                "string",
                "What gave it: `template`, `trait` or `rule`.",
            ),
            ("rank", "int", "Its rank now."),
        ],
        doc: "An entity came to hold an ability (08 §3.3): its first source gave it, or a rule granted again one that was revoked. What it confers is applied before this is raised.",
    },
    EventInfo {
        name: ABILITY_REVOKED,
        subjects: &[("self", "entity")],
        fields: &[
            ABILITY_FIELD,
            (
                "how",
                "string",
                "`revoked` by `revoke_ability`, `lost` with its last source, or `moved` with a mind to another body (08 §3.4).",
            ),
        ],
        doc: "An entity no longer holds an ability (08 §6.3). Its modes have ended and what it conferred has been taken back.",
    },
    EventInfo {
        name: ABILITY_DORMANT,
        subjects: &[("self", "entity")],
        fields: &[ABILITY_FIELD],
        doc: "An ability's `while` stopped holding (08 §6.1): it confers nothing, and its modes have ended.",
    },
    EventInfo {
        name: ABILITY_ACTIVE,
        subjects: &[("self", "entity")],
        fields: &[ABILITY_FIELD],
        doc: "A dormant ability's `while` holds again: it confers what it lists. Its modes do not come back by themselves.",
    },
    EventInfo {
        name: RANK_CHANGED,
        subjects: &[("self", "entity")],
        fields: &[
            ABILITY_FIELD,
            ("old", "int", "The rank before."),
            ("new", "int", "The rank now."),
        ],
        doc: "An ability's rank changed (08 §4.1): its track advanced, or `set_rank` set it. What applies from a rank (`at_rank`) has been brought into line.",
    },
    EventInfo {
        name: MODE_STARTED,
        subjects: &[("self", "entity")],
        fields: &[ABILITY_FIELD, MODE_FIELD],
        doc: "A mode was switched on (08 §5.4): its trait is given and its upkeep runs.",
    },
    EventInfo {
        name: MODE_ENDED,
        subjects: &[("self", "entity")],
        fields: &[
            ABILITY_FIELD,
            MODE_FIELD,
            (
                "how",
                "string",
                "`ended` (switched off), `empty` (its upkeep ran a pool dry), `dormant`, `revoked`, `rank` (the rank fell below it) or `moved` (with a mind to another body).",
            ),
        ],
        doc: "A mode ended (08 §5.4): its trait and its upkeep are gone together, and its `on_end` has run.",
    },
    EventInfo {
        name: ABILITY_USED,
        subjects: &[("self", "entity")],
        fields: &[
            ABILITY_FIELD,
            ACTION_FIELD,
            (
                "cost",
                "any",
                "What was spent: a map of each pool's or item's qualified id to the amount, overdraw included.",
            ),
        ],
        doc: "A granted action was started through its ability (08 §5): the cost has been taken and the cooldown started.",
    },
    EventInfo {
        name: ABILITY_REFUSED,
        subjects: &[("self", "entity")],
        fields: &[
            ABILITY_FIELD,
            ACTION_FIELD,
            ("reason", "string", "The code of the reason."),
            ("message", "string", "The reason, in words."),
        ],
        doc: "A use of an ability was refused after `ability_using` ran, or when a rule asked for it (`use_ability`). Nothing was spent.",
    },
];

/// The hooks of abilities (05 §8.2; 08 §7.9, §12): entered in the event
/// bus of every simulation and in the engine's registry.
pub fn hook_types() -> Vec<EventType> {
    vec![
        EventType::new(ABILITY_USING)
            .subject("self", "entity")
            .field(ABILITY_FIELD.0, FieldType::Str, ABILITY_FIELD.2)
            .field(ACTION_FIELD.0, FieldType::Str, ACTION_FIELD.2)
            .field(
                "params",
                FieldType::Any,
                "The action's parameters, by name: what the command gave and the grant's `params`. A handler may change them; each is held to its type again.",
            )
            .field(
                "cost",
                FieldType::Any,
                "The cost by pool or item, a map of qualified id to amount. A handler may change an amount, to zero or more, for this use only.",
            )
            .hook(["params", "cost"])
            .doc("Runs before a granted action is started through its ability (08 §7.9), after the ability was found usable. A handler may change the parameters and the cost for this use, or cancel, with a reason."),
        EventType::new(ABILITY_GRANTING)
            .subject("self", "entity")
            .field(ABILITY_FIELD.0, FieldType::Str, ABILITY_FIELD.2)
            .field("source", FieldType::Str, "What gives it: `rule`.")
            .hook(Vec::<String>::new())
            .doc("Runs before a rule grants an ability (`grant_ability`; 08 §12). A handler may cancel, with a reason: a gift that cannot be given."),
        EventType::new(ABILITY_REVOKING)
            .subject("self", "entity")
            .field(ABILITY_FIELD.0, FieldType::Str, ABILITY_FIELD.2)
            .hook(Vec::<String>::new())
            .doc("Runs before a rule revokes an ability (`revoke_ability`; 08 §12). A handler may cancel, with a reason: a gift that cannot be stolen."),
    ]
}

fn field_type(written: &str) -> Type {
    match FieldType::parse(written) {
        Some(FieldType::Bool) => Type::Bool,
        Some(FieldType::Int) => Type::Int,
        Some(FieldType::Real) => Type::Real,
        Some(FieldType::Str) => Type::Str,
        Some(FieldType::Any) | None => Type::Any,
    }
}

/// A property of the kind `ability` every instance has: name, type,
/// argument (name and type), the events that change it, documentation.
pub(crate) type FixedProperty = (
    &'static str,
    &'static str,
    Option<(&'static str, &'static str)>,
    &'static [&'static str],
    &'static str,
);

/// The properties of the kind `ability` every instance has.
pub(crate) const FIXED_PROPERTIES: &[FixedProperty] = &[
    (
        "rank",
        "int",
        None,
        &[RANK_CHANGED],
        "Its rank: its track's level, or the rank set directly (08 §4.1).",
    ),
    (
        "owner",
        "entity?",
        None,
        &["mind_transferred", ABILITY_REVOKED],
        "The entity that holds it; null once it is not held.",
    ),
    (
        "state",
        "string?",
        None,
        &[ABILITY_DORMANT, ABILITY_ACTIVE],
        "`active` or `dormant` (08 §6.1); null once it is not held.",
    ),
    ("def", "id<ability>?", None, &[], "Its definition."),
    ("id", "string?", None, &[], "Its definition's qualified id."),
    ("tags", "list<string>?", None, &[], "Its definition's tags."),
    (
        "mode_on",
        "bool",
        Some(("mode", "string")),
        &[MODE_STARTED, MODE_ENDED],
        "Whether a mode of it, by id, is on (08 §5.4).",
    ),
    (
        "cooldown",
        "real",
        Some(("action", "id<action>")),
        &[],
        "Seconds of its owner's own time before the grant of an action can be used again: 0 when it is ready. Moves with time, so it cannot be kept current.",
    ),
];

/// Enters in a registry what abilities name: the kind `ability` and the
/// properties every instance has, the definition type `ability`, and
/// the hooks, unless they are there already. Called while the engine's
/// registry is built, before its properties and verbs.
pub fn register_types(registry: &mut Registry) {
    for (name, doc) in [
        (AbilityDef::TYPE, AbilityDef::DOC),
        (
            <crate::action::ActionDef as Def>::TYPE,
            <crate::action::ActionDef as Def>::DOC,
        ),
    ] {
        if registry.def_type_id(name).is_none() {
            let _ = registry.add_def_type(name, doc);
        }
    }
    let kind = match registry.kind_id(ABILITY_KIND) {
        Some(kind) => kind,
        None => match registry.add_kind(KindSpec::new(ABILITY_KIND).doc(
            "An ability an entity holds (08 §3.2), reached as `self.ability('id')`; `ability` in what an ability works out about itself.",
        )) {
            Ok(kind) => kind,
            Err(_) => return,
        },
    };
    for (name, ty, arg, changed_by, doc) in FIXED_PROPERTIES {
        if registry.property_id(kind, name).is_some() {
            continue;
        }
        let Some(ty) = registry.parse_type(ty) else {
            continue;
        };
        let mut spec = PropertySpec::new(kind, *name, ty)
            .doc(*doc)
            .yields_own(*name == "owner")
            .changed_by(changed_by.iter().copied());
        if let Some((arg, arg_ty)) = arg {
            let Some(arg_ty) = registry.parse_type(arg_ty) else {
                continue;
            };
            spec = spec.param(*arg, arg_ty, "Which.");
        }
        let _ = registry.add_property(spec);
    }
    for hook in hook_types() {
        if registry.event_id(&hook.name).is_some() {
            continue;
        }
        let mut spec = EventSpec::new(hook.name.clone()).doc(hook.doc.clone());
        let subjects = hook
            .subjects
            .iter()
            .map(|(name, kind)| (name.as_str(), kind.as_str()));
        for (name, kind) in subjects.chain(CAUSE_SUBJECTS.iter().copied()) {
            if let Some(kind) = registry.kind_id(kind) {
                spec = spec.subject(name.trim_end_matches('?'), kind);
            }
        }
        for field in &hook.fields {
            spec = spec.field(field.name.clone(), field_type(&field.ty), field.doc.clone());
        }
        let _ = registry.add_event(spec.hook(hook.mutable.iter().cloned()));
    }
}

/// The loader's registry step for abilities (04 §7.3): one property of
/// the kind `ability` for each parameter name an ability defines, so
/// that `ability.power` is checked against what exists. A parameter is
/// a number; an instance whose ability has no parameter of that name
/// reads it as null.
fn registry_step(cx: &mut RegistryCx<'_>) {
    let defs = cx.defs();
    let mut names: Vec<(String, String)> = Vec::new();
    for (id, def) in defs.iter::<AbilityDef>() {
        let owner = defs.name::<AbilityDef>(id).unwrap_or("").to_owned();
        for name in def.params.keys() {
            if !names.iter().any(|(have, _)| have == name) {
                names.push((name.clone(), owner.clone()));
            }
        }
    }
    let registry = cx.registry();
    register_types(registry);
    let Some(kind) = registry.kind_id(ABILITY_KIND) else {
        return;
    };
    for (name, first) in names {
        if registry.property_id(kind, &name).is_some() {
            continue;
        }
        let spec = PropertySpec::new(kind, name.clone(), Type::Real).doc(format!(
            "The parameter `{name}` of the ability (08 §4.2), as first defined by `{first}`: its expression read for the instance, then modified. Null for an instance whose ability has no parameter of that name, or that is no longer held. It may read anything, so it cannot be kept current."
        ));
        let _ = registry.add_property(spec);
    }
}

/// Registers the `ability` definition type, its registry step, and the
/// check that every ability a trait or a template names, and every
/// pool, item and trait an ability names, is defined. Called from the
/// crate's `register_defs`.
pub fn register_defs(types: &mut DefTypes) {
    types.register::<AbilityDef>().add_cross_check(check_names);
    types.add_registry_step(registry_step);
}

/// A name written in data found among the loading definitions of a type.
fn known<T: Def>(view: &DefsView<'_>, name: &str) -> bool {
    if name.contains(':') {
        return view.lookup::<T>(name).is_some();
    }
    view.iter::<T>().any(|(id, _)| {
        view.name::<T>(id)
            .and_then(|qualified| qualified.split_once(':'))
            .is_some_and(|(_, bare)| bare == name)
    })
}

/// Reports each name an ability, a trait or a template writes that no
/// definition answers to, at the definition that writes it.
fn check_names(cx: &mut CrossCx<'_>) {
    let view = cx.defs();
    let mut missing: Vec<(DefLabel, String, &'static str)> = Vec::new();
    for (id, def) in view.iter::<AbilityDef>() {
        let label = DefLabel::new(AbilityDef::TYPE, view.name::<AbilityDef>(id).unwrap_or(""));
        let costs = def
            .grants
            .iter()
            .map(|grant| (&grant.cost, "grants"))
            .chain(def.modes.iter().map(|mode| (&mode.cost, "modes")));
        for (costs, path) in costs {
            for (name, cost) in costs {
                let found = if cost.item {
                    known::<ItemDef>(view, name)
                } else {
                    known::<PoolDef>(view, name)
                };
                if !found {
                    let what = if cost.item { "item" } else { "pool" };
                    missing.push((
                        label.clone(),
                        format!("there is no {what} `{name}` to pay a cost from"),
                        path,
                    ));
                }
            }
        }
        for mode in &def.modes {
            if let Some(name) = &mode.trait_name
                && !known::<TraitDef>(view, name)
            {
                missing.push((
                    label.clone(),
                    format!("there is no trait `{name}`"),
                    "modes",
                ));
            }
        }
        for always in &def.always.0 {
            for name in &always.traits {
                if !known::<TraitDef>(view, name) {
                    missing.push((
                        label.clone(),
                        format!("there is no trait `{name}`"),
                        "always",
                    ));
                }
            }
            for modifier in &always.params {
                if let Some(name) = &modifier.ability
                    && !known::<AbilityDef>(view, name)
                {
                    missing.push((
                        label.clone(),
                        format!("there is no ability `{name}`"),
                        "always",
                    ));
                }
            }
        }
    }
    for (id, def) in view.iter::<TraitDef>() {
        let label = DefLabel::new(TraitDef::TYPE, view.name::<TraitDef>(id).unwrap_or(""));
        for name in &def.abilities {
            if !known::<AbilityDef>(view, name) {
                missing.push((
                    label.clone(),
                    format!("there is no ability `{name}`"),
                    "abilities",
                ));
            }
        }
        for modifier in &def.ability_params {
            if let Some(name) = &modifier.ability
                && !known::<AbilityDef>(view, name)
            {
                missing.push((
                    label.clone(),
                    format!("there is no ability `{name}`"),
                    "ability_params",
                ));
            }
        }
    }
    for (id, def) in view.iter::<TemplateDef>() {
        let label = DefLabel::new(
            TemplateDef::TYPE,
            view.name::<TemplateDef>(id).unwrap_or(""),
        );
        for name in def.all_abilities() {
            if !known::<AbilityDef>(view, name) {
                missing.push((
                    label.clone(),
                    format!("there is no ability `{name}`"),
                    "abilities",
                ));
            }
        }
    }
    for (label, message, path) in missing {
        cx.report(
            Diagnostic::new(code::VALIDATION, message)
                .in_def(Some(&label))
                .with_path(path),
        );
    }
}

// ---- What the kernel calls ------------------------------------------------------------

/// The bit of a `Timer` wake-up's `arg` that marks it as a mode's
/// upkeep running a pool dry. Bits 61 to 63, which traits, bodies and
/// pools use, are clear. The low byte is the mode's number and the bits
/// from 8 to 59 the instance's; `id` is the owner's entity bits.
pub const MODE_WAKE_TAG: u64 = 1 << 60;

/// The bits of `arg` that tell one timer of the domain from another.
const TAG_BITS: u64 = 0b1111 << 60;

/// The wake-up for the moment a mode's upkeep runs a pool dry.
pub fn mode_wake(entity: EntityId, number: u64, mode: u16) -> Wake {
    let number = number & ((1 << 52) - 1);
    Wake::new(
        WakeDomain::Timer,
        entity.to_bits(),
        MODE_WAKE_TAG | (number << 8) | u64::from(mode & 0xFF),
    )
}

/// Whether a popped wake-up is a mode's.
pub fn is_mode_wake(wake: &Wake) -> bool {
    wake.domain == WakeDomain::Timer && wake.arg & TAG_BITS == MODE_WAKE_TAG
}

/// Handles a wake-up of the domain `Timer` if it is a mode's: the kernel
/// hands every such wake-up here first. Returns whether it was one.
pub fn handle_wake(sim: &mut Sim, id: WakeId, wake: Wake) -> bool {
    if !is_mode_wake(&wake) {
        return false;
    }
    let entity = EntityId::from_bits(wake.id);
    let number = (wake.arg >> 8) & ((1 << 52) - 1);
    let mode = (wake.arg & 0xFF) as u16;
    modes::on_wake(sim, id, entity, number, mode);
    true
}

/// What abilities do about an event the kernel has delivered: marking
/// expressions kept current stale, following traits that list
/// abilities, tracks that rank them, minds that carry them, pools that
/// feed their modes, and entities that are gone.
pub fn on_event(sim: &mut Sim, event: &SimEvent) {
    reactive::event_delivered(sim, event);
    let name = event.name();
    let Some(entity) = event.subject("self").and_then(|subject| subject.entity()) else {
        if let crate::event::EventBody::EntityRemoved { entity } = &event.body {
            ops::forget_entity(sim, *entity);
        }
        return;
    };
    match name {
        crate::traits::TRAIT_ADDED | crate::traits::TRAIT_REMOVED => {
            reactive::rewatch_traits(sim, entity);
            ops::sync_trait_sources(sim, entity, event.cause);
            modes::reschedule_all(sim, entity);
        }
        "track_advanced" => ops::track_advanced(sim, entity, event),
        "mind_transferred" => ops::mind_transferred(sim, entity, event),
        "pool_crossed" | "attr_changed" => modes::reschedule_all(sim, entity),
        "entity_relocated" => modes::reschedule_all(sim, entity),
        "entity_removed" => ops::forget_entity(sim, entity),
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_reason_is_listed_once() {
        let mut found: Vec<&str> = ABILITY_REASONS.iter().map(|info| info.code).collect();
        let count = found.len();
        found.sort_unstable();
        found.dedup();
        assert_eq!(found.len(), count, "a code is listed twice");
        for info in ABILITY_REASONS {
            assert!(info.code.starts_with("ability."));
            assert!(!info.doc.is_empty());
        }
    }

    #[test]
    fn a_mode_wake_names_its_instance_and_mode() {
        let entity = EntityId::new(7, 3);
        let wake = mode_wake(entity, 123_456, 4);
        assert!(is_mode_wake(&wake));
        assert!(!crate::traits::is_trait_wake(&wake));
        assert!(!crate::stats::is_pool_wake(&wake));
        assert_eq!((wake.arg >> 8) & ((1 << 52) - 1), 123_456);
        assert_eq!(wake.arg & 0xFF, 4);
    }

    #[test]
    fn the_events_hooks_and_kind_are_in_the_engine_registry() {
        let registry = crate::engine_registry();
        for info in ABILITY_EVENTS {
            let id = registry
                .event_id(info.name)
                .unwrap_or_else(|| panic!("`{}` is not registered", info.name));
            let spec = &registry.event(id).unwrap().spec;
            assert_eq!(spec.fields.len(), info.fields.len(), "{}", info.name);
        }
        for hook in hook_types() {
            let id = registry.event_id(&hook.name).unwrap();
            let spec = &registry.event(id).unwrap().spec;
            assert!(spec.hook, "{}", hook.name);
            assert_eq!(spec.fields.len(), hook.fields.len());
        }
        let kind = registry.kind_id(ABILITY_KIND).unwrap();
        for (name, ..) in FIXED_PROPERTIES {
            assert!(registry.property_id(kind, name).is_some(), "{name}");
        }
        assert!(registry.def_type_id(AbilityDef::TYPE).is_some());
    }
}
