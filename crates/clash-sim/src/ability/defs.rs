//! The `ability` definition type (08 §3.1): what an ability makes
//! available, what it confers while it is active, what using it costs
//! and how it grows.
//!
//! The engine names no ability. A fire bolt, a sword style, a powered
//! suit's repulsors and a gift for languages are the same kind of
//! definition: grants of actions, things conferred while it is held,
//! modes, a rank and parameters.

use std::collections::BTreeMap;

use clash_core::{Real, define_id};
use clash_data::expr::{Any, Number};
use clash_data::{
    CompileCx, Cond, Def, Diagnostic, Expr, FromKind, Kind, KindValue, Located, Ref, Scope, Text,
    ValidateCx, by_kind, code,
};
use serde::Deserialize;
use serde::de::{Deserializer, MapAccess};

use crate::action::{ActionDef, ChannelDef};
use crate::item::ModifierDef;
use crate::rule::{Effects, Rules};
use crate::stats::{PoolDef, Seat, TrackDef};

define_id! {
    /// An `ability` definition (08 §3.1).
    pub struct AbilityId;
}

/// The most modes one ability may have: a mode's number fits in the
/// low byte of the wake-up that ends its upkeep.
pub const MAX_MODES: usize = 255;

/// The first slot of an instance's givers and stats sources that stands
/// for an entry of its `always`, beside the slots of its modes.
pub const ALWAYS_SLOT: u16 = 0x100;

/// The most entries `always` may have.
pub const MAX_ALWAYS: usize = 0xFF00;

/// The rank an ability with no track is granted at, unless the
/// definition or the grant says otherwise.
pub const DEFAULT_RANK: u32 = 1;

/// The highest rank set directly: as high as a track's level can go.
pub const MAX_RANK: u32 = crate::stats::TRACK_LEVEL_CAP;

/// The names of the properties every ability has (08 §12.1): a
/// parameter may not take one.
pub const RESERVED_PARAMS: &[&str] = &[
    "rank", "owner", "state", "def", "id", "tags", "mode_on", "cooldown", "kind", "var",
];

/// What an ability's grant is for, as a hint to whoever chooses for a
/// character (08 §11; 09 §6.1). The engine reads nothing into it.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Intent {
    /// It hurts someone.
    Harm,
    /// It shields someone.
    Protect,
    /// It gets its user away.
    Escape,
    /// It makes someone do or not do something.
    Control,
    /// It helps someone.
    Aid,
    /// Anything else.
    Utility,
}

impl Intent {
    /// The word data writes.
    pub const fn name(self) -> &'static str {
        match self {
            Intent::Harm => "harm",
            Intent::Protect => "protect",
            Intent::Escape => "escape",
            Intent::Control => "control",
            Intent::Aid => "aid",
            Intent::Utility => "utility",
        }
    }
}

/// Where a cost the owner cannot meet may be made up (08 §5.2): what is
/// missing is taken from another pool at a ratio.
#[derive(Debug, Deserialize)]
pub struct OverdrawDef {
    /// The pool that makes up the shortfall.
    pub from: Ref<PoolDef>,
    /// How much of it each missing unit costs. Default 1.
    #[serde(default = "one")]
    pub ratio: Real,
}

fn one() -> Real {
    Real::ONE
}

/// One cost of a grant or a mode (08 §5.2), keyed in data by the pool it
/// is paid from, or by the item consumed when `item` is true.
///
/// In data: an amount (a number or an expression), or
/// `{ "amount": …, "overdraw": { "from": pool, "ratio": 2.0 }, "from": "…" }`
/// for a pool, or `{ "item": true, "count": 1 }` for items.
#[derive(Debug)]
pub struct CostDef {
    /// How much, for a pool: an expression with the owner as `self` and
    /// the instance as `ability`.
    pub amount: Expr<Number>,
    /// Where a shortfall is made up from, if it may be.
    pub overdraw: Option<OverdrawDef>,
    /// Who pays in the owner's place: an expression that gives an
    /// entity, such as something bound to the ability (08 §7.3). Left
    /// out, the owner pays.
    pub from: Option<Expr<Any>>,
    /// Whether the key names an item consumed rather than a pool.
    pub item: bool,
    /// For an item: how many are consumed. Default 1.
    pub count: u32,
}

#[derive(Deserialize)]
struct CostObject {
    #[serde(default)]
    amount: Option<Expr<Number>>,
    #[serde(default)]
    overdraw: Option<OverdrawDef>,
    #[serde(default)]
    from: Option<Expr<Any>>,
    #[serde(default)]
    item: bool,
    #[serde(default)]
    count: Option<u32>,
}

impl<'de> FromKind<'de> for CostDef {
    fn from_kind<A: MapAccess<'de>>(
        kind: Kind,
        value: KindValue<'_, A>,
    ) -> Result<CostDef, A::Error> {
        match kind {
            Kind::Number | Kind::String => value.read::<Expr<Number>>().map(|amount| CostDef {
                amount,
                overdraw: None,
                from: None,
                item: false,
                count: 1,
            }),
            Kind::Object => value.read::<CostObject>().map(|cost| CostDef {
                amount: cost.amount.unwrap_or_else(|| Expr::literal(Real::ZERO)),
                overdraw: cost.overdraw,
                from: cost.from,
                item: cost.item,
                count: cost.count.unwrap_or(1),
            }),
            other => Err(other.unexpected("an amount, or a cost with `amount` or `item`")),
        }
    }
}

impl<'de> Deserialize<'de> for CostDef {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<CostDef, D::Error> {
        by_kind(deserializer)
    }
}

/// What starting a granted action or switching a mode on spends, by
/// the pool (or item) each cost names, as written: a qualified id, or a
/// bare one looked up in sorted namespace order.
pub type Costs = BTreeMap<String, CostDef>;

/// An action an ability makes available to its owner (08 §5.1).
///
/// In data: an action's id, or
/// `{ "action": id, "params": { name: expression }, "cost": { … },
///    "cooldown": time, "at_rank": int, "if": condition, "intent": word,
///    "value": expression }`.
#[derive(Debug)]
pub struct AbilityGrantDef {
    /// The action it starts.
    pub action: Ref<ActionDef>,
    /// Parameters of the action the ability supplies: expressions with
    /// the owner as `self` and the instance as `ability`. They stand in
    /// for whatever a command gives under the same names.
    pub params: BTreeMap<String, Expr<Any>>,
    /// What starting it spends.
    pub cost: Costs,
    /// Seconds of the owner's own time before it can be used again: an
    /// expression with the owner as `self` and the instance as
    /// `ability`.
    pub cooldown: Option<Expr<Number>>,
    /// The least rank it needs.
    pub at_rank: Option<u32>,
    /// A further condition, in the owner's mind: what it believes, its
    /// own state and the instance.
    pub condition: Option<Cond>,
    /// A hint to whoever chooses for a character.
    pub intent: Option<Intent>,
    /// An optional score of a use, for choosers that cannot preview it
    /// (08 §11). Carried for the AI layer.
    pub value: Option<Expr<Number>>,
}

#[derive(Deserialize)]
struct GrantObject {
    action: Ref<ActionDef>,
    #[serde(default)]
    params: BTreeMap<String, Expr<Any>>,
    #[serde(default)]
    cost: Costs,
    #[serde(default)]
    cooldown: Option<Expr<Number>>,
    #[serde(default)]
    at_rank: Option<u32>,
    #[serde(default, rename = "if")]
    condition: Option<Cond>,
    #[serde(default)]
    intent: Option<Intent>,
    #[serde(default)]
    value: Option<Expr<Number>>,
}

impl<'de> FromKind<'de> for AbilityGrantDef {
    fn from_kind<A: MapAccess<'de>>(
        kind: Kind,
        value: KindValue<'_, A>,
    ) -> Result<AbilityGrantDef, A::Error> {
        match kind {
            Kind::String => value
                .read::<Ref<ActionDef>>()
                .map(|action| AbilityGrantDef {
                    action,
                    params: BTreeMap::new(),
                    cost: Costs::new(),
                    cooldown: None,
                    at_rank: None,
                    condition: None,
                    intent: None,
                    value: None,
                }),
            Kind::Object => value.read::<GrantObject>().map(|grant| AbilityGrantDef {
                action: grant.action,
                params: grant.params,
                cost: grant.cost,
                cooldown: grant.cooldown,
                at_rank: grant.at_rank,
                condition: grant.condition,
                intent: grant.intent,
                value: grant.value,
            }),
            other => Err(other.unexpected("an action's id, or a grant with an `action`")),
        }
    }
}

impl<'de> Deserialize<'de> for AbilityGrantDef {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<AbilityGrantDef, D::Error> {
        by_kind(deserializer)
    }
}

/// A change to a parameter of abilities (08 §4.2), made by whatever
/// lists it while that lasts: a trait (its `ability_params`) or an
/// active ability (its `always`). It names one ability or a tag, never
/// both, and one parameter.
#[derive(Clone, Debug, Deserialize)]
pub struct ParamModifierDef {
    /// The ability whose parameter it changes, as written.
    #[serde(default)]
    pub ability: Option<String>,
    /// Or: every ability with this tag.
    #[serde(default)]
    pub tag: Option<String>,
    /// The parameter.
    pub param: String,
    /// Added to the parameter's value. Default 0.
    #[serde(default)]
    pub add: Real,
    /// Multiplied in after every addition. Default 1.
    #[serde(default = "one")]
    pub mul: Real,
}

impl ParamModifierDef {
    /// Reports what is wrong with one, where it is listed.
    pub fn check(&self, at: &str, cx: &mut ValidateCx<'_>) {
        match (&self.ability, &self.tag) {
            (Some(_), Some(_)) => cx.error(format!(
                "{at}: a parameter modifier names an `ability` or a `tag`, not both"
            )),
            (None, None) => cx.error(format!(
                "{at}: a parameter modifier names the `ability` or the `tag` it changes"
            )),
            _ => {}
        }
        if !self.add.is_finite() || !self.mul.is_finite() {
            cx.error(format!("{at}: `add` and `mul` must be finite"));
        }
    }
}

/// What an ability confers on its owner while it is active (08 §3.1):
/// from a rank up, if `at_rank` says.
#[derive(Debug, Default, Deserialize)]
pub struct AlwaysDef {
    /// The least rank this applies from.
    #[serde(default)]
    pub at_rank: Option<u32>,
    /// Traits given, with the instance as their giver.
    #[serde(default)]
    pub traits: Vec<String>,
    /// Attribute modifiers, under the instance as their source.
    #[serde(default)]
    pub modifiers: Vec<ModifierDef>,
    /// Changes to the parameters of abilities: of this one or others.
    #[serde(default)]
    pub params: Vec<ParamModifierDef>,
    /// Senses added or altered (15), as written. Carried until
    /// perception reads them.
    #[serde(default)]
    pub senses: Vec<String>,
}

/// `always`: one set of what is conferred, or a list of them each from
/// its own rank.
#[derive(Debug, Default)]
pub struct Always(pub Vec<AlwaysDef>);

impl<'de> FromKind<'de> for Always {
    fn from_kind<A: MapAccess<'de>>(
        kind: Kind,
        value: KindValue<'_, A>,
    ) -> Result<Always, A::Error> {
        match kind {
            Kind::Object => value.read::<AlwaysDef>().map(|one| Always(vec![one])),
            Kind::List => value.read::<Vec<AlwaysDef>>().map(Always),
            other => Err(other.unexpected("what is conferred, or a list of such")),
        }
    }
}

impl<'de> Deserialize<'de> for Always {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Always, D::Error> {
        by_kind(deserializer)
    }
}

/// A state an ability can be switched into (08 §5.4): an aura, a
/// stance of power, a transformation, flight.
#[derive(Debug, Deserialize)]
pub struct ModeDef {
    /// Its name within the ability: lowercase letters, digits and `_`.
    pub id: Located<String>,
    /// What a reader calls it.
    #[serde(default)]
    pub name: Option<Text>,
    /// The least rank it can be switched on at.
    #[serde(default)]
    pub at_rank: Option<u32>,
    /// The trait it gives while on, as written.
    #[serde(default, rename = "trait")]
    pub trait_name: Option<String>,
    /// What it drains while on, per second of the owner's time, by pool:
    /// an expression with the owner as `self` and the instance as
    /// `ability`, worked out when it is switched on.
    #[serde(default)]
    pub upkeep: BTreeMap<Ref<PoolDef>, Expr<Number>>,
    /// What switching it on spends.
    #[serde(default)]
    pub cost: Costs,
    /// Channel slots it holds while on (03 §6.2). Carried: a mode's
    /// trait disables slots with its `channels` meanwhile.
    #[serde(default)]
    pub uses: BTreeMap<Ref<ChannelDef>, u16>,
    /// Effects run as it comes on, with the owner as `self` and the
    /// instance as `ability`.
    #[serde(default)]
    pub on_start: Effects,
    /// Effects run as it ends, however it ends.
    #[serde(default)]
    pub on_end: Effects,
    /// How its owner appears while it is on (12 §4.4), as written.
    #[serde(default)]
    pub look: Option<String>,
}

/// An ability (08 §3.1): actions it lets its owner take, things it
/// confers while active, sustained modes, a rank and parameters.
///
/// ```json
/// { "type": "ability", "id": "pyrokinesis", "name": "Pyrokinesis",
///   "tags": ["elemental", "fire"], "seat": "mind", "track": "pyrokinesis",
///   "params": { "power": "2.0e4 * 2 ^ ability.rank" },
///   "always": { "traits": ["heat_tolerant"] },
///   "grants": [ { "action": "fire_bolt", "params": { "energy": "ability.power" },
///                 "cost": { "heat_reserve": "ability.power / 5000" }, "cooldown": "2 s" } ],
///   "modes": [ { "id": "burning_aura", "at_rank": 2, "trait": "burning_aura",
///                "upkeep": { "heat_reserve": 2.0 } } ],
///   "while": "self.loc.medium != 'water'" }
/// ```
#[derive(Debug, Deserialize)]
pub struct AbilityDef {
    /// What a reader calls it.
    #[serde(default)]
    pub name: Option<Text>,
    /// What a closer look says of it.
    #[serde(default)]
    pub description: Option<Text>,
    /// What it is, for suppression, counters, filters and rules that
    /// care.
    #[serde(default)]
    pub tags: Vec<String>,
    /// `body` or `mind`: which it stays with when they part (06 §8.2).
    #[serde(default)]
    pub seat: Seat,
    /// The track whose level is its rank. Left out, the rank is set when
    /// it is granted and changed by `set_rank`.
    #[serde(default)]
    pub track: Option<Ref<TrackDef>>,
    /// The rank it is granted at when it has no track and the grant
    /// gives none. Default 1.
    #[serde(default)]
    pub rank: Option<u32>,
    /// Named numbers its actions and traits read, as `ability.<name>`:
    /// expressions with the owner as `self` and the instance as
    /// `ability`, worked out when they are read, then modified.
    #[serde(default)]
    pub params: BTreeMap<String, Expr<Number>>,
    /// What it confers while active.
    #[serde(default)]
    pub always: Always,
    /// The actions it makes available.
    #[serde(default)]
    pub grants: Vec<AbilityGrantDef>,
    /// Sustained states it can be switched into.
    #[serde(default)]
    pub modes: Vec<ModeDef>,
    /// A reactive condition (05 §7.5) about the owner: while it is false
    /// the ability is dormant and confers nothing.
    #[serde(default, rename = "while")]
    pub condition: Option<Cond>,
    /// Rules attached to the owner while the ability is active
    /// (05 §9.3).
    #[serde(default)]
    pub rules: Rules,
}

impl AbilityDef {
    /// The rank it is granted at when nothing else says.
    pub fn starting_rank(&self) -> u32 {
        self.rank.unwrap_or(DEFAULT_RANK)
    }

    /// The number of a mode by its id.
    pub fn mode_index(&self, id: &str) -> Option<usize> {
        self.modes.iter().position(|mode| mode.id.value == id)
    }
}

/// The scope of what an ability works out about itself: the owner as
/// `self` and the instance as `ability`, reading truth.
pub(crate) fn ability_scope() -> Scope {
    Scope::new()
        .subject("self", "entity")
        .subject("ability", super::ABILITY_KIND)
}

/// The scope of a grant's condition: the owner's mind, which may read
/// its own state and its own ability.
fn mind_scope() -> Scope {
    Scope::new()
        .mind("self", "entity")
        .own("ability", super::ABILITY_KIND)
        .pure()
}

fn compile_costs(costs: &mut Costs, cx: &mut CompileCx<'_>) {
    let pure = ability_scope().pure();
    for cost in costs.values_mut() {
        cx.compile(&mut cost.amount, &pure);
        cx.compile(&mut cost.from, &pure);
    }
}

fn validate_costs(costs: &Costs, at: &str, cx: &mut ValidateCx<'_>) {
    for (name, cost) in costs {
        if name.is_empty() {
            cx.error(format!("{at}: a cost names a pool or an item"));
        }
        if cost.item && (cost.from.is_some() || cost.overdraw.is_some()) {
            cx.error(format!(
                "{at}: the cost `{name}` consumes items, which has no `from` or `overdraw`"
            ));
        }
        if cost.item && cost.count == 0 {
            cx.error(format!(
                "{at}: the cost `{name}` consumes at least one item"
            ));
        }
        if let Some(constant) = cost
            .amount
            .constant()
            .and_then(clash_expr::Value::as_number)
            && (!constant.is_finite() || constant < Real::ZERO)
        {
            cx.error(format!(
                "{at}: the cost `{name}` must be a finite amount of zero or more"
            ));
        }
        if let Some(overdraw) = &cost.overdraw
            && !(overdraw.ratio.is_finite() && overdraw.ratio > Real::ZERO)
        {
            cx.error(format!(
                "{at}: the overdraw of `{name}` needs a finite `ratio` above zero"
            ));
        }
    }
}

/// Whether a name can be written after a dot: lowercase letters,
/// digits and `_`, starting with a letter.
fn is_name(name: &str) -> bool {
    let mut chars = name.chars();
    chars.next().is_some_and(|first| first.is_ascii_lowercase())
        && chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
}

impl Def for AbilityDef {
    type Id = AbilityId;
    const TYPE: &'static str = "ability";
    const DOC: &'static str = "Something a character has and can use: a skill, a spell, a power, a gift (08 §3). Actions it makes available with their costs and cooldowns, what it confers while active, modes that are switched on and off, a rank and parameters.";

    fn compile(&mut self, cx: &mut CompileCx<'_>) {
        let pure = Scope::new().pure();
        cx.compile(&mut self.name, &pure);
        cx.compile(&mut self.description, &pure);
        let scope = ability_scope();
        let truth = scope.clone().pure();
        cx.compile(&mut self.params, &truth);
        super::reactive::check_param_cycles(self, cx);
        let errors = cx.error_count();
        cx.compile(&mut self.condition, &truth);
        if cx.error_count() == errors
            && let Some(condition) = &self.condition
        {
            super::reactive::check_reactive_cond(condition, cx);
        }
        for grant in &mut self.grants {
            cx.compile(&mut grant.params, &truth);
            compile_costs(&mut grant.cost, cx);
            cx.compile(&mut grant.cooldown, &truth);
            cx.compile(&mut grant.condition, &mind_scope());
            cx.compile(&mut grant.value, &mind_scope());
        }
        for mode in &mut self.modes {
            cx.compile(&mut mode.name, &pure);
            cx.compile(&mut mode.upkeep, &truth);
            compile_costs(&mut mode.cost, cx);
            cx.compile(&mut mode.on_start, &scope);
            cx.compile(&mut mode.on_end, &scope);
        }
    }

    fn validate(&self, cx: &mut ValidateCx<'_>) {
        if self.tags.iter().any(String::is_empty) {
            cx.error("a tag cannot be empty");
        }
        if self.track.is_some() && self.rank.is_some() {
            cx.error("`rank` is the rank of an ability with no track: this one's rank is its track's level");
        }
        if self.rank.is_some_and(|rank| rank > MAX_RANK) {
            cx.error(format!("`rank` is at most {MAX_RANK}"));
        }
        for name in self.params.keys() {
            if !is_name(name) {
                cx.error(format!(
                    "the parameter `{name}` must be a name: lowercase letters, digits and `_`, starting with a letter"
                ));
            }
            if RESERVED_PARAMS.contains(&name.as_str()) {
                cx.error(format!(
                    "`{name}` is a property every ability has, so a parameter cannot take the name"
                ));
            }
        }
        if self.always.0.len() > MAX_ALWAYS {
            cx.error(format!("`always` has at most {MAX_ALWAYS} entries"));
        }
        for (index, always) in self.always.0.iter().enumerate() {
            let at = format!("always {index}");
            for (at_modifier, modifier) in always.modifiers.iter().enumerate() {
                if !modifier.add.is_finite() || !modifier.mul.is_finite() {
                    cx.error(format!(
                        "{at}: modifiers {at_modifier}: `add` and `mul` must be finite"
                    ));
                }
            }
            for modifier in &always.params {
                modifier.check(&at, cx);
            }
        }
        let mut actions = Vec::new();
        for (index, grant) in self.grants.iter().enumerate() {
            let at = format!("grants {index}");
            if actions.contains(&grant.action.id()) {
                cx.error(format!(
                    "{at}: the action is granted twice: a use names the action it starts"
                ));
            }
            actions.push(grant.action.id());
            validate_costs(&grant.cost, &at, cx);
        }
        if self.modes.len() > MAX_MODES {
            cx.error(format!("an ability has at most {MAX_MODES} modes"));
        }
        let mut ids: Vec<&str> = Vec::new();
        for (index, mode) in self.modes.iter().enumerate() {
            let at = format!("modes {index}");
            if !is_name(&mode.id.value) {
                cx.error_at(
                    mode.id.span,
                    "a mode's `id` is lowercase letters, digits and `_`, starting with a letter",
                );
            }
            if ids.contains(&mode.id.value.as_str()) {
                cx.error_at(mode.id.span, "two modes of an ability have this `id`");
            }
            ids.push(&mode.id.value);
            validate_costs(&mode.cost, &at, cx);
            mode.on_start.validate(cx);
            mode.on_end.validate(cx);
            if mode.uses.values().any(|count| *count == 0) {
                cx.error(format!(
                    "{at}: `uses` gives each channel a count of at least one slot"
                ));
            }
        }
    }
}

/// Reports a diagnostic about a reactive field at the place it was
/// written.
pub(crate) fn report_at(
    cx: &mut CompileCx<'_>,
    site: &clash_data::Site,
    message: impl Into<String>,
) {
    let mut diagnostic = Diagnostic::new(code::VALIDATION, message).with_path(site.path.clone());
    if let Some(span) = site.span {
        diagnostic = diagnostic.at(cx.sources(), span);
    }
    cx.report(diagnostic);
}
