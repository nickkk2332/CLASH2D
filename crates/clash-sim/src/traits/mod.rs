//! Traits (`docs/06-entities-and-bodies.md` section 5): anything an
//! entity has that changes what it is or can do, and that lasts until it
//! is removed or wears off.
//!
//! There is no separate status, mutation, perk, blessing, curse, drug or
//! stance. A trait with a `duration` is a status; one without lasts
//! until something removes it. The engine names no trait: a trait is
//! what its definition confers.
//!
//! | Part | Is | Module |
//! |------|----|--------|
//! | [`TraitDef`] | The `trait` definition type | `defs` |
//! | [`Traits`], [`TraitInst`] | The component: the trait instances an entity has, and what they put on it | here |
//! | [`Giver`] | Who gave an instance: a rule, an item, a posture, an ability, a template | here |
//! | [`add_trait`], [`remove_trait`], [`withdraw`], [`give_named`] | Adding and removing | `ops` |
//! | [`on_timer`], [`suspend`], [`resume`] | Durations: timers on the holder's clock (03 §9) | `ops` |
//! | [`reconsider`] | Working `while` out again | `ops` |
//! | [`conferred`], [`channel_change`] | What the instances confer, from scratch | `confer` |
//! | [`properties`], [`verbs()`], [`TRAIT_EVENTS`], [`hook_types`], [`TRAIT_REASONS`] | What rules read, call, listen for and are refused with | `props`, [`verbs`](mod@verbs), here |
//!
//! # The rules, in short
//!
//! - **Instances.** An entity has a trait once for each [`Giver`] that
//!   gave it: every rule is one giver, each item another, the posture
//!   another. Each instance has its own stacks and its own duration.
//! - **Stacking** (06 §5) applies to an instance given again by the same
//!   giver: `none` changes nothing, `refresh` starts the duration again,
//!   `stack` adds a stack up to `max` and starts the duration again.
//! - **Groups.** Adding a trait of a `group` removes the other traits of
//!   that group the entity has, whoever gave them.
//! - **Exactly what it added.** An instance's attribute modifiers and
//!   pool rates are kept by the stats under the instance as their
//!   source ([`source_of`]); its tags are recorded where the entity did
//!   not have them already; its rules are attached through
//!   [`RuleState::attach`](crate::rule::RuleState::attach) where they
//!   were not attached already. Removing it takes back exactly that. A
//!   tag or a rule two instances want stays until neither does.
//! - **Stacks.** Modifiers are applied once for each stack and pool
//!   rates multiplied by the stacks. Tags, rules and channels are not
//!   counted.
//! - **Dormancy.** While a trait's `while` does not hold it confers
//!   nothing, and its duration still runs.
//! - **Durations** are wake-ups on the holder's clock, scheduled for it
//!   through [`Sim::schedule_for`](crate::Sim::schedule_for): they follow it from clock to clock.
//!   When it leaves the simulation they are taken off the timeline with
//!   the local time they had left ([`suspend`]); when it returns they go
//!   on ([`resume`]): a `pause` duration as it was, an `elapse` one less
//!   the time that passed, ending at once if that was all of it.
//! - **Every refusal has a reason** of [`TRAIT_REASONS`].
//!
//! # What the kernel calls
//!
//! | When | The kernel |
//! |------|-----------|
//! | Building its definition types | Calls the crate's `register_defs`, which registers `trait` and the check of the trait names items and postures give |
//! | Building its registry | Calls [`register_types`], lists [`verbs()`] and [`properties`] and mirrors [`TRAIT_EVENTS`] |
//! | Building its event bus | Enters [`TRAIT_EVENTS`] and [`hook_types`] |
//! | A wake-up of the domain `Timer` pops | Calls [`on_timer`], which hands pools' and bodies' to `life::timer_wake` |
//! | An entity leaves the world and comes back (`Sim::relocate`) | Calls [`suspend`] before it goes and [`resume`] once it is back |
//! | An item comes into use by a holder and leaves it | `item` calls [`give_named`] and [`withdraw`] with [`Giver::item`] |
//! | A posture changes | `movement` calls [`posture_changed`] |
//!
//! # Stats and rules
//!
//! Modifiers and rates are changed over the simulation's own stores
//! (`Sim::with_stats`), and the `attr_changed` and `pool_crossed` events
//! of what a trait changed are raised as every change to stats is.
//! `on_apply` and `on_remove` run against the simulation with the holder
//! as `self` ([`run_effects`]).
//!
//! # What is not built here
//!
//! `grants`, `abilities`, `senses`, `bound`, `goals`, `weights` and
//! `seat` are read, checked and carried for the packages that give them
//! effect (actions, abilities, perception, minds). `channels` is read by
//! [`channel_change`] for the action runtime to apply. `while` is worked
//! out when a trait of the holder is added or removed and by
//! [`reconsider`], until reactive expressions keep it current.

mod confer;
mod defs;
mod ops;
mod props;
pub mod verbs;

use clash_core::{Dur, EntityId, Tick, impl_state_hash};
use clash_data::{CrossCx, Def, DefLabel, DefTypes, DefsView, Diagnostic, code};
use clash_expr::{EventSpec, Registry, Type};
use clash_time::{ClockId, WakeId};
use serde::{Deserialize, Serialize};

use crate::bus::{EventType, FieldType};
use crate::event::{CAUSE_SUBJECTS, EventInfo};
use crate::item::ItemDef;
use crate::movement::{PostureDef, PostureId};
use crate::reason::ReasonInfo;
use crate::rule::EocId;
use crate::stats::{Source, SourceKind};

pub use confer::{Conferred, channel_change, conferred};
pub use defs::{GrantDef, MAX_STACKS, RateDef, Stacking, TraitDef, TraitId, WhileUnloaded};
pub use ops::{
    Added, TRAIT_WAKE_TAG, add_trait, give_named, is_trait_wake, lookup, on_timer, posture_changed,
    reconsider, remove_trait, resume, run_effects, suspend, trait_wake, withdraw,
};
pub(crate) use ops::{leaving_world, returned_to_world};
pub use props::properties;
pub use verbs::verbs;

/// Registers the `trait` definition type and the check that every trait
/// an item or a posture names is defined. Called from the crate's
/// `register_defs`.
pub fn register_defs(types: &mut DefTypes) {
    types
        .register::<TraitDef>()
        .add_cross_check(check_named_traits);
}

/// What kind of thing gave a trait.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[repr(u8)]
pub enum GiverKind {
    /// A rule, through `add_trait`, or code acting as one. All rules are
    /// one giver.
    Rule,
    /// An item held in a grasp or worn (06 §9.1).
    Item,
    /// The posture the entity is in (06 §11.1).
    Posture,
    /// An ability (08 §3.3).
    Ability,
    /// A template applied at creation (06 §10).
    Template,
    /// Anything else.
    Other,
}

impl GiverKind {
    /// The word rules read: `rule`, `item`, `posture`, `ability`,
    /// `template` or `other`.
    pub const fn name(self) -> &'static str {
        match self {
            GiverKind::Rule => "rule",
            GiverKind::Item => "item",
            GiverKind::Posture => "posture",
            GiverKind::Ability => "ability",
            GiverKind::Template => "template",
            GiverKind::Other => "other",
        }
    }
}

impl clash_core::StateHash for GiverKind {
    fn state_hash(&self, hasher: &mut clash_core::StateHasher) {
        hasher.write_u8(*self as u8);
    }
}

/// Who gave an entity a trait: a kind and a number that tells one of
/// that kind from another (an item's entity bits, a posture's id). An
/// entity has a trait once for each giver.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug, Serialize, Deserialize)]
pub struct Giver {
    /// What kind of thing.
    pub kind: GiverKind,
    /// Which one.
    pub id: u64,
}

impl_state_hash!(Giver { kind, id });

impl Giver {
    /// Every rule: what `add_trait` gives as.
    pub const RULE: Giver = Giver {
        kind: GiverKind::Rule,
        id: 0,
    };

    /// A giver of a kind.
    pub const fn new(kind: GiverKind, id: u64) -> Giver {
        Giver { kind, id }
    }

    /// An item, held or worn.
    pub const fn item(item: EntityId) -> Giver {
        Giver::new(GiverKind::Item, item.to_bits())
    }

    /// A posture.
    pub fn posture(posture: PostureId) -> Giver {
        Giver::new(GiverKind::Posture, posture.index() as u64)
    }
}

/// The number of a trait instance on its entity: given when it is added
/// and never to another instance of the same entity.
pub type TraitKey = u32;

/// The stats source an instance's modifiers and rates are kept under.
pub const fn source_of(key: TraitKey) -> Source {
    Source::new(SourceKind::Trait, key as u64)
}

/// Where an instance's duration stands.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub enum Timer {
    /// It lasts until removed.
    Lasts,
    /// It runs: the wake-up that ends it is on the holder's clock.
    Running(WakeId),
    /// It stands still with this much local time left: its holder is out
    /// of the simulation.
    Held(Dur),
}

impl clash_core::StateHash for Timer {
    fn state_hash(&self, hasher: &mut clash_core::StateHasher) {
        match self {
            Timer::Lasts => hasher.write_u8(0),
            Timer::Running(wake) => {
                hasher.write_u8(1);
                wake.state_hash(hasher);
            }
            Timer::Held(left) => {
                hasher.write_u8(2);
                left.state_hash(hasher);
            }
        }
    }
}

/// One trait an entity has, from one giver.
#[derive(Clone, PartialEq, Debug, Serialize, Deserialize)]
pub struct TraitInst {
    key: TraitKey,
    def: TraitId,
    giver: Giver,
    stacks: u16,
    /// Its `while` does not hold: it confers nothing.
    dormant: bool,
    timer: Timer,
}

impl_state_hash!(TraitInst {
    key,
    def,
    giver,
    stacks,
    dormant,
    timer
});

impl TraitInst {
    /// Its number on its entity.
    pub fn key(&self) -> TraitKey {
        self.key
    }

    /// Which trait.
    pub fn def(&self) -> TraitId {
        self.def
    }

    /// Who gave it.
    pub fn giver(&self) -> Giver {
        self.giver
    }

    /// How many stacks it has: at least one.
    pub fn stacks(&self) -> u16 {
        self.stacks
    }

    /// Whether its `while` does not hold, so that it confers nothing.
    pub fn is_dormant(&self) -> bool {
        self.dormant
    }

    /// Where its duration stands.
    pub fn timer(&self) -> Timer {
        self.timer
    }
}

/// Where an entity was when it left the simulation: what its returning
/// measures the time away by.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub struct Away {
    /// The clock it was on, if it was on one.
    pub clock: Option<ClockId>,
    /// That clock's local time then.
    pub local: Dur,
    /// The universal time then.
    pub tick: Tick,
}

impl_state_hash!(Away { clock, local, tick });

/// The traits of one entity (06 §5): the `Traits` component.
///
/// It holds the instances in the order they were added, and what they
/// put on the entity besides the stats: the tags it did not have before
/// and the rules that were not attached before. The modifiers and rates
/// are the stats', under each instance's [`source_of`].
///
/// Its fields change only through the functions of this module. It is
/// saved and hashed.
#[derive(Clone, PartialEq, Debug, Default, Serialize, Deserialize)]
pub struct Traits {
    insts: Vec<TraitInst>,
    next_key: TraitKey,
    /// The tags the entity carries only because of its traits, in order.
    tags: Vec<String>,
    /// The rules attached to it only because of its traits, in order.
    rules: Vec<EocId>,
    /// Set while the entity is out of the simulation.
    away: Option<Away>,
}

impl_state_hash!(Traits {
    insts,
    next_key,
    tags,
    rules,
    away
});

impl Traits {
    /// Every instance, in the order they were added.
    pub fn instances(&self) -> &[TraitInst] {
        &self.insts
    }

    /// The instances of one trait, in the order they were added.
    pub fn of(&self, def: TraitId) -> impl Iterator<Item = &TraitInst> {
        self.insts.iter().filter(move |inst| inst.def == def)
    }

    /// An instance by its key.
    pub fn get(&self, key: TraitKey) -> Option<&TraitInst> {
        self.insts.iter().find(|inst| inst.key == key)
    }

    /// Whether the entity has a trait, from any giver, dormant or not.
    pub fn has(&self, def: TraitId) -> bool {
        self.of(def).next().is_some()
    }

    /// The stacks of a trait, over every giver: 0 for one it lacks.
    pub fn stacks(&self, def: TraitId) -> u32 {
        self.of(def).map(|inst| u32::from(inst.stacks)).sum()
    }

    /// The traits the entity has, each once, in the order first added.
    pub fn defs(&self) -> Vec<TraitId> {
        let mut seen: Vec<TraitId> = Vec::new();
        for inst in &self.insts {
            if !seen.contains(&inst.def) {
                seen.push(inst.def);
            }
        }
        seen
    }

    /// The tags the entity carries only because of its traits.
    pub fn conferred_tags(&self) -> &[String] {
        &self.tags
    }

    /// The rules attached to it only because of its traits.
    pub fn attached_rules(&self) -> &[EocId] {
        &self.rules
    }

    /// Whether the entity is out of the simulation, its durations
    /// standing still.
    pub fn is_away(&self) -> bool {
        self.away.is_some()
    }

    /// An instance with a number of the test suite's choosing: for the
    /// tests of the component store.
    #[cfg(test)]
    pub(crate) fn sample(seed: u32) -> Traits {
        Traits {
            insts: vec![TraitInst {
                key: seed,
                def: TraitId::new(seed),
                giver: Giver::RULE,
                stacks: 1,
                dormant: false,
                timer: Timer::Lasts,
            }],
            next_key: seed + 1,
            tags: vec![format!("tag {seed}")],
            rules: Vec::new(),
            away: None,
        }
    }
}

/// Whether the traits of an entity confer a tag on it: what the items
/// package asks before it takes back a tag of its own.
pub fn confers_tag(sim: &crate::Sim, entity: EntityId, tag: &str) -> bool {
    sim.components()
        .get::<Traits>(entity)
        .is_some_and(|traits| {
            traits
                .tags
                .binary_search_by(|have| have.as_str().cmp(tag))
                .is_ok()
        })
}

// ---- Reasons -----------------------------------------------------------------

/// The codes of this module's reasons. Each is listed in
/// [`TRAIT_REASONS`].
pub mod codes {
    /// The id is not a `trait` definition.
    pub const UNKNOWN: &str = "trait.unknown";
    /// A duration is not a finite time above zero.
    pub const BAD_DURATION: &str = "trait.bad_duration";
    /// A count of stacks is not a whole number above zero.
    pub const BAD_STACKS: &str = "trait.bad_stacks";
    /// A verb's argument is missing or of the wrong kind.
    pub const BAD_ARGUMENT: &str = "trait.bad_argument";
    /// The definitions were loaded without the `trait` type.
    pub const NOT_REGISTERED: &str = "trait.not_registered";
}

/// The reasons this module refuses for, for the generated reference.
pub const TRAIT_REASONS: &[ReasonInfo] = &[
    ReasonInfo {
        code: codes::UNKNOWN,
        doc: "The id is not a `trait` definition. Values: `trait`, as given.",
    },
    ReasonInfo {
        code: codes::BAD_DURATION,
        doc: "A duration given to `add_trait`, or left by a handler of `trait_adding`, is not a finite time above zero. Values: `duration`, in seconds.",
    },
    ReasonInfo {
        code: codes::BAD_STACKS,
        doc: "A count of stacks to remove is not a whole number above zero. Values: `stacks`.",
    },
    ReasonInfo {
        code: codes::BAD_ARGUMENT,
        doc: "A verb of traits was given an argument that is missing or of the wrong kind. Values: `arg`, the argument's name.",
    },
    ReasonInfo {
        code: codes::NOT_REGISTERED,
        doc: "The definitions were loaded without the `trait` definition type. Values: none.",
    },
];

// ---- Events ------------------------------------------------------------------

/// Raised when an entity gains a trait, or one it has is refreshed or
/// gains a stack.
pub const TRAIT_ADDED: &str = "trait_added";
/// Raised when a trait, or some of its stacks, are gone.
pub const TRAIT_REMOVED: &str = "trait_removed";
/// Raised when a trait's duration has run out, before `trait_removed`.
pub const TRAIT_EXPIRED: &str = "trait_expired";
/// The hook raised before a trait is added or given again.
pub const TRAIT_ADDING: &str = "trait_adding";
/// The hook raised before `remove_trait` removes a trait.
pub const TRAIT_REMOVING: &str = "trait_removing";
/// The field of [`TRAIT_ADDING`] a handler may change.
pub const DURATION_FIELD: &str = "duration";

const TRAIT_FIELD: (&str, &str, &str) = ("trait", "string", "The qualified id of the trait.");
const GIVER_FIELD: (&str, &str, &str) = (
    "giver",
    "string",
    "What gave it: `rule`, `item`, `posture`, `ability`, `template` or `other`.",
);
const LOOK_FIELD: (&str, &str, &str) = (
    "look",
    "string",
    "The trait's `look` as its definition writes it (12 §4.4); empty for none.",
);

/// The notifications of traits, entered beside the kernel's own in every
/// simulation's event bus and in the engine's registry, and listed for
/// the generated reference. Each is raised as a payload under its name.
pub const TRAIT_EVENTS: &[EventInfo] = &[
    EventInfo {
        name: TRAIT_ADDED,
        subjects: &[("self", "entity")],
        fields: &[
            TRAIT_FIELD,
            GIVER_FIELD,
            (
                "stacks",
                "int",
                "How many stacks the instance has now: 1 unless it stacks.",
            ),
            (
                "first",
                "bool",
                "Whether the entity did not have the trait from this giver before: false for a refresh or another stack.",
            ),
            (
                "duration",
                "real",
                "How long it will last, in seconds of the holder's time; null for one that lasts until removed.",
            ),
            LOOK_FIELD,
        ],
        doc: "An entity was given a trait (06 §5): it gained it, or one it had from the same giver was refreshed or gained a stack. What it confers is applied before this is raised.",
    },
    EventInfo {
        name: TRAIT_REMOVED,
        subjects: &[("self", "entity")],
        fields: &[
            TRAIT_FIELD,
            GIVER_FIELD,
            ("stacks", "int", "How many stacks went."),
            (
                "left",
                "int",
                "How many stacks of the instance are left: 0 when it is gone.",
            ),
            (
                "how",
                "string",
                "`removed` by `remove_trait`, `expired`, `replaced` by a trait of the same group, or `withdrawn` by what gave it: an item let go, a posture left.",
            ),
            LOOK_FIELD,
        ],
        doc: "A trait, or some of its stacks, is gone (06 §5), and what it conferred with them has been taken back. The trait's own attached rules are detached before this is delivered: a trait answers its own ending with `on_remove`.",
    },
    EventInfo {
        name: TRAIT_EXPIRED,
        subjects: &[("self", "entity")],
        fields: &[TRAIT_FIELD, GIVER_FIELD],
        doc: "A trait's duration ran out (03 §9). `trait_removed` follows, with `how` `expired`.",
    },
];

/// The hooks of traits (05 §8.2): entered in the event bus of every
/// simulation and in the engine's registry.
pub fn hook_types() -> Vec<EventType> {
    vec![
        EventType::new(TRAIT_ADDING)
            .subject("self", "entity")
            .field(TRAIT_FIELD.0, FieldType::Str, TRAIT_FIELD.2)
            .field(GIVER_FIELD.0, FieldType::Str, GIVER_FIELD.2)
            .field(
                "stacks",
                FieldType::Int,
                "How many stacks the instance will have.",
            )
            .field(
                DURATION_FIELD,
                FieldType::Any,
                "How long it will last, in seconds of the holder's time, or null for one that lasts until removed. A handler may change it: a number above zero, or null.",
            )
            .hook([DURATION_FIELD])
            .doc("Runs before an entity is given a trait (06 §5), whether it is new to it, refreshed or stacked. A handler may cancel, with a reason, and may change `duration`. Not raised when giving it would change nothing."),
        EventType::new(TRAIT_REMOVING)
            .subject("self", "entity")
            .field(TRAIT_FIELD.0, FieldType::Str, TRAIT_FIELD.2)
            .field(
                "stacks",
                FieldType::Int,
                "How many stacks would go, over every giver.",
            )
            .hook(Vec::<String>::new())
            .doc("Runs before `remove_trait` removes a trait or some of its stacks (06 §5). A handler may cancel, with a reason. Not raised when a trait expires, is replaced by one of its group, or is withdrawn by what gave it."),
    ]
}

/// The type of a field of a hook, as an expression sees it.
fn field_type(written: &str) -> Type {
    match FieldType::parse(written) {
        Some(FieldType::Bool) => Type::Bool,
        Some(FieldType::Int) => Type::Int,
        Some(FieldType::Real) => Type::Real,
        Some(FieldType::Str) => Type::Str,
        Some(FieldType::Any) | None => Type::Any,
    }
}

/// Enters in a registry what the properties and verbs of traits name and
/// what their hooks are: the definition type `trait` and the two hooks,
/// unless they are there already. Called while the engine's registry is
/// built, before its properties and verbs.
pub fn register_types(registry: &mut Registry) {
    if registry.def_type_id(TraitDef::TYPE).is_none() {
        let _ = registry.add_def_type(TraitDef::TYPE, TraitDef::DOC);
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
            let ty = field_type(&field.ty);
            // `duration` may be null: a trait that lasts until removed.
            let ty = if field.name == DURATION_FIELD {
                Type::Real.nullable()
            } else {
                ty
            };
            spec = spec.field(field.name.clone(), ty, field.doc.clone());
        }
        let _ = registry.add_event(spec.hook(hook.mutable.iter().cloned()));
    }
}

// ---- Checks ------------------------------------------------------------------

/// Reports each trait an item or a posture names that no `trait`
/// definition answers to, at the definition that names it.
fn check_named_traits(cx: &mut CrossCx<'_>) {
    let view = cx.defs();
    let mut missing: Vec<(DefLabel, String)> = Vec::new();
    for (id, def) in view.iter::<ItemDef>() {
        for name in &def.traits {
            if lookup_in(view, name).is_none() {
                let label = DefLabel::new(ItemDef::TYPE, view.name::<ItemDef>(id).unwrap_or(""));
                missing.push((label, name.clone()));
            }
        }
    }
    for (id, def) in view.iter::<PostureDef>() {
        for name in &def.traits {
            if lookup_in(view, name).is_none() {
                let label =
                    DefLabel::new(PostureDef::TYPE, view.name::<PostureDef>(id).unwrap_or(""));
                missing.push((label, name.clone()));
            }
        }
    }
    for (label, name) in missing {
        cx.report(
            Diagnostic::new(code::VALIDATION, format!("there is no trait `{name}`"))
                .in_def(Some(&label))
                .with_path("traits"),
        );
    }
}

/// The trait a name written in data stands for, while definitions load:
/// a qualified id exactly; a bare one in the first namespace, in sorted
/// order, that has it.
fn lookup_in(view: &DefsView<'_>, name: &str) -> Option<TraitId> {
    if name.contains(':') {
        return view.lookup::<TraitDef>(name);
    }
    let mut found: Vec<(&str, TraitId)> = view
        .iter::<TraitDef>()
        .filter_map(|(id, _)| Some((view.name::<TraitDef>(id)?, id)))
        .filter(|(qualified, _)| {
            qualified
                .split_once(':')
                .is_some_and(|(_, bare)| bare == name)
        })
        .collect();
    found.sort_unstable();
    found.first().map(|(_, id)| *id)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_reason_is_listed_once() {
        let mut found: Vec<&str> = TRAIT_REASONS.iter().map(|info| info.code).collect();
        let count = found.len();
        found.sort_unstable();
        found.dedup();
        assert_eq!(found.len(), count, "a code is listed twice");
        for info in TRAIT_REASONS {
            assert!(info.code.starts_with("trait."));
            assert!(!info.doc.is_empty());
        }
    }

    #[test]
    fn the_events_and_hooks_are_in_the_engine_registry() {
        let registry = crate::engine_registry();
        for info in TRAIT_EVENTS {
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
        assert!(registry.def_type_id(TraitDef::TYPE).is_some());
    }
}
