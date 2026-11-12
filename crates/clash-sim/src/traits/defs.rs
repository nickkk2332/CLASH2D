//! The `trait` definition type (06 §5): what an entity has that changes
//! what it is or can do, and everything the trait confers while it lasts.
//!
//! The engine names no trait and gives none a meaning. A trait is the
//! modifiers, rates, tags, rules and the rest that its data lists.

use std::collections::BTreeMap;

use clash_core::{Dur, Real, define_id};
use clash_data::{
    CompileCx, Cond, Def, FromKind, Kind, KindValue, Located, Ref, Scope, Text, Time, ValidateCx,
    by_kind,
};
use serde::Deserialize;
use serde::de::{Deserializer, MapAccess};

use crate::action::{ActionDef, ChannelDef};
use crate::item::ModifierDef;
use crate::rule::{Effects, Rules};
use crate::stats::{PoolDef, Seat};

define_id! {
    /// A `trait` definition (06 §5).
    pub struct TraitId;
}

/// The most stacks a trait that stacks may reach, whatever its `max`
/// says.
pub const MAX_STACKS: u16 = 1000;

/// What adding a trait an entity already has from the same giver does
/// (06 §5).
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Stacking {
    /// Nothing: the trait is had once.
    #[default]
    None,
    /// Its duration starts again.
    Refresh,
    /// It gains a stack, up to its `max`, and its duration starts again.
    Stack,
}

impl Stacking {
    /// The word data writes.
    pub const fn name(self) -> &'static str {
        match self {
            Stacking::None => "none",
            Stacking::Refresh => "refresh",
            Stacking::Stack => "stack",
        }
    }
}

/// What a trait's duration does while its owner is out of the
/// simulation (03 §9).
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WhileUnloaded {
    /// It stands still, and goes on where it stopped when the owner
    /// returns.
    #[default]
    Pause,
    /// The time that passed is taken off when the owner returns; a
    /// trait whose time has run out then ends at once.
    Elapse,
}

/// A pool rate a trait applies while it lasts (06 §4.2): what the pool
/// gains in a second of its owner's time, negative to drain. A trait
/// with stacks applies it once for each stack.
#[derive(Debug, Deserialize)]
pub struct RateDef {
    /// The pool.
    pub pool: Ref<PoolDef>,
    /// Per second of the owner's time.
    pub rate: Real,
}

/// An action a trait makes available (08 §5.1), with what using it
/// costs. Carried until the action runtime offers granted actions.
///
/// In data: an action's id, or
/// `{ "action": id, "cost": { pool: expression }, "cooldown": time,
///    "at_rank": int, "if": condition }`.
#[derive(Debug)]
pub struct GrantDef {
    /// The action.
    pub action: Ref<ActionDef>,
    /// What starting it spends, by pool, as written.
    pub cost: BTreeMap<String, Located<String>>,
    /// Local time before it can be used again.
    pub cooldown: Option<Time>,
    /// The least rank of the ability that gives the trait.
    pub at_rank: Option<u32>,
    /// A further condition, as written.
    pub condition: Option<Located<String>>,
}

/// The object form of a grant.
#[derive(Deserialize)]
struct GrantObject {
    action: Ref<ActionDef>,
    #[serde(default)]
    cost: BTreeMap<String, Located<String>>,
    #[serde(default)]
    cooldown: Option<Time>,
    #[serde(default)]
    at_rank: Option<u32>,
    #[serde(default, rename = "if")]
    condition: Option<Located<String>>,
}

impl<'de> FromKind<'de> for GrantDef {
    fn from_kind<A: MapAccess<'de>>(
        kind: Kind,
        value: KindValue<'_, A>,
    ) -> Result<GrantDef, A::Error> {
        match kind {
            Kind::String => value.read::<Ref<ActionDef>>().map(|action| GrantDef {
                action,
                cost: BTreeMap::new(),
                cooldown: None,
                at_rank: None,
                condition: None,
            }),
            Kind::Object => value.read::<GrantObject>().map(|grant| GrantDef {
                action: grant.action,
                cost: grant.cost,
                cooldown: grant.cooldown,
                at_rank: grant.at_rank,
                condition: grant.condition,
            }),
            other => Err(other.unexpected("an action's id, or a grant with an `action`")),
        }
    }
}

impl<'de> Deserialize<'de> for GrantDef {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<GrantDef, D::Error> {
        by_kind(deserializer)
    }
}

/// A trait (06 §5): anything an entity has that changes what it is or
/// can do. A species feature, a mutation, a perk, a blessing, a curse, a
/// drug, a stun, a stance: there is no other kind of status.
///
/// ```json
/// { "type": "trait", "id": "adrenaline", "name": "Adrenaline",
///   "duration": "20 s", "stacking": "refresh",
///   "modifiers": [ { "attr": "quickness", "mul": 1.3 } ],
///   "rates": [ { "pool": "stamina", "rate": -0.5 } ],
///   "tags": ["stimulated"],
///   "on_remove": [ { "add_trait": { "to": "self", "trait": "crash" } } ] }
/// ```
///
/// What it confers is applied when it is added, and taken back exactly
/// when it goes: modifiers and pool rates under the trait instance as
/// their source, tags it added, rules it attached.
#[derive(Debug, Deserialize)]
pub struct TraitDef {
    /// What a reader calls it.
    #[serde(default)]
    pub name: Option<Text>,
    /// What a closer look says of it.
    #[serde(default)]
    pub description: Option<Text>,
    /// `body` or `mind`: which it stays with when they part (06 §8.2).
    /// Carried until minds move.
    #[serde(default)]
    pub seat: Seat,
    /// Attribute modifiers while it lasts, once for each stack.
    #[serde(default)]
    pub modifiers: Vec<ModifierDef>,
    /// Pool rates while it lasts, times its stacks.
    #[serde(default)]
    pub rates: Vec<RateDef>,
    /// Tags its holder carries while it lasts.
    #[serde(default)]
    pub tags: Vec<String>,
    /// Actions its holder may use (03 §6; 08 §5.1). Carried until the
    /// action runtime offers them.
    #[serde(default)]
    pub grants: Vec<GrantDef>,
    /// Abilities conferred while it lasts and is not dormant (08 §3.3),
    /// by id as written: each instance is a source of each
    /// ([`crate::ability`]).
    #[serde(default)]
    pub abilities: Vec<String>,
    /// Changes to the parameters of its holder's abilities while it
    /// lasts and is not dormant (08 §4.2), once for each stack: each
    /// names an ability or a tag, a parameter, and `add` and `mul`.
    #[serde(default)]
    pub ability_params: Vec<crate::ability::ParamModifierDef>,
    /// Channel slots added (a positive count) or disabled (a negative
    /// one) while it lasts (03 §6.2). Read by
    /// [`channel_change`](super::channel_change) for the action runtime.
    #[serde(default)]
    pub channels: BTreeMap<Ref<ChannelDef>, i16>,
    /// Senses added or altered (15), by name as written. Carried until
    /// perception is built.
    #[serde(default)]
    pub senses: Vec<String>,
    /// Rules attached to its holder while it lasts (05 §9.3).
    #[serde(default)]
    pub rules: Rules,
    /// Effects run when it is added, with the holder as `self`.
    #[serde(default)]
    pub on_apply: Effects,
    /// Effects run when it ends, however it ends, with the holder as
    /// `self`.
    #[serde(default)]
    pub on_remove: Effects,
    /// If given, it ends by itself after this much of its holder's time
    /// (03 §9). Above zero.
    #[serde(default)]
    pub duration: Option<Time>,
    /// What its duration does while its holder is out of the simulation.
    /// Default `pause`.
    #[serde(default)]
    pub while_unloaded: WhileUnloaded,
    /// What adding it again does. Default `none`.
    #[serde(default)]
    pub stacking: Stacking,
    /// The most stacks it reaches, for `"stacking": "stack"`. Left out,
    /// [`MAX_STACKS`].
    #[serde(default)]
    pub max: Option<u16>,
    /// Traits of one group exclude each other: adding one removes the
    /// others. Stances, forms, moods.
    #[serde(default)]
    pub group: Option<String>,
    /// While this does not hold, the trait is dormant and confers
    /// nothing. About the holder, `self`. Worked out when a trait of
    /// the holder is added or removed until reactive expressions
    /// (05 §7.5) keep it current.
    #[serde(default, rename = "while")]
    pub condition: Option<Cond>,
    /// Whether, given by an ability, it ends when that ability does
    /// (08 §7.3). Carried until abilities are built.
    #[serde(default)]
    pub bound: bool,
    /// How its holder appears while it lasts (12 §4.4), as written.
    /// Reported with `trait_added` and `trait_removed`.
    #[serde(default)]
    pub look: Option<String>,
    /// Goals added to its holder's mind (09 §3), by id as written.
    /// Carried until minds decide.
    #[serde(default)]
    pub goals: Vec<String>,
    /// Changes to the weights its holder's mind scores goals by (09 §3).
    /// Carried until minds decide.
    #[serde(default)]
    pub weights: BTreeMap<String, Real>,
}

impl TraitDef {
    /// How long it lasts, if it ends by itself.
    pub fn lasts(&self) -> Option<Dur> {
        self.duration.map(Time::dur)
    }

    /// The most stacks it reaches: 1 unless it stacks.
    pub fn max_stacks(&self) -> u16 {
        match self.stacking {
            Stacking::Stack => self.max.unwrap_or(MAX_STACKS).clamp(1, MAX_STACKS),
            Stacking::None | Stacking::Refresh => 1,
        }
    }
}

/// The scope of what a trait says about its holder: the holder as
/// `self`.
fn holder_scope() -> Scope {
    Scope::new().subject("self", "entity")
}

impl Def for TraitDef {
    type Id = TraitId;
    const TYPE: &'static str = "trait";
    const DOC: &'static str = "Anything an entity has that changes what it is or can do: a feature, a mutation, a perk, a blessing, a curse, a drug, a stun, a stance (06 §5). What it confers lasts as long as it does.";

    fn compile(&mut self, cx: &mut CompileCx<'_>) {
        let pure = Scope::new().pure();
        cx.compile(&mut self.name, &pure);
        cx.compile(&mut self.description, &pure);
        cx.compile(&mut self.condition, &holder_scope().pure());
        cx.compile(&mut self.on_apply, &holder_scope());
        cx.compile(&mut self.on_remove, &holder_scope());
    }

    fn validate(&self, cx: &mut ValidateCx<'_>) {
        self.on_apply.validate(cx);
        self.on_remove.validate(cx);
        if self.duration.is_some_and(|time| time.dur() <= Dur::ZERO) {
            cx.error("`duration` must be above zero: leave it out for a trait that lasts until it is removed");
        }
        if self.duration.is_none() && self.while_unloaded != WhileUnloaded::Pause {
            cx.warn("`while_unloaded` says what a duration does, and this trait has none");
        }
        match (self.stacking, self.max) {
            (Stacking::Stack, Some(max)) if max == 0 || max > MAX_STACKS => cx.error(format!(
                "`max` is how many stacks it reaches: from 1 to {MAX_STACKS}"
            )),
            (Stacking::None | Stacking::Refresh, Some(_)) => {
                cx.error("`max` is for a trait whose `stacking` is `stack`");
            }
            _ => {}
        }
        for (index, modifier) in self.modifiers.iter().enumerate() {
            if !modifier.add.is_finite() || !modifier.mul.is_finite() {
                cx.error(format!("modifiers {index}: `add` and `mul` must be finite"));
            }
        }
        for (index, rate) in self.rates.iter().enumerate() {
            if !rate.rate.is_finite() {
                cx.error(format!("rates {index}: `rate` must be finite"));
            }
        }
        for modifier in &self.ability_params {
            modifier.check("ability_params", cx);
        }
        if self.channels.values().any(|count| *count == 0) {
            cx.error("`channels` gives each channel a count of slots: above zero to add, below to disable");
        }
        if self.tags.iter().any(String::is_empty) {
            cx.error("a tag cannot be empty");
        }
        if self.group.as_deref() == Some("") {
            cx.error("`group` names the traits it excludes: it cannot be empty");
        }
        for (name, weight) in &self.weights {
            if !weight.is_finite() {
                cx.error(format!("weights: `{name}` must be finite"));
            }
        }
    }
}
