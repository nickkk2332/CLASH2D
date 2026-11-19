//! The definition types of game scenarios (13 §3, §5): `scenario`,
//! `mission` and `setting`.
//!
//! The engine gives none of them a meaning of its own: a scenario is a
//! place to put definitions other documents give meaning to, an
//! objective is a condition and some effects, a setting is a value
//! rules read.

use clash_core::{Dur, Real, define_id};
use clash_data::expr::Any;
use clash_data::{
    CompileCx, Cond, CrossCx, Def, DefLabel, Diagnostic, Expr, FromKind, Kind, KindValue, Located,
    Scope, Text, Time, ValidateCx, by_kind, code,
};
use clash_world::defs::{OrderedMap, RealmDef};
use serde::Deserialize;
use serde::de::{Deserializer, MapAccess};

use crate::perception::SenseDef;
use crate::rule::{Effects, RuleGroupDef};
use crate::template::TemplateDef;

define_id! {
    /// A `scenario` definition (13 §3).
    pub struct ScenarioId(u16);
}

define_id! {
    /// A `mission` definition (13 §5.2).
    pub struct MissionId(u16);
}

define_id! {
    /// A `setting` definition (04 §10).
    pub struct SettingId(u16);
}

/// The name an objective's taker is bound under.
pub const TAKER: &str = "taker";
/// The name the party is bound under: a list of the directly
/// controlled characters.
pub const PARTY: &str = "party";
/// The name the site the party began at is bound under.
pub const START_SITE: &str = "start_site";
/// The name the cell the party began at is bound under.
pub const START_CELL: &str = "start_cell";

/// The saving policies a scenario may name (13 §9.2).
pub const SAVING_POLICIES: [&str; 4] = ["free", "single", "checkpoints", "none"];

/// What may follow when the party is gone (13 §9.3).
pub const PARTY_GONE_OUTCOMES: [&str; 3] = ["end", "continue", "restore"];

/// The ways of carrying on after the party is gone that are built
/// (13 §9.3 `by`): another of the player's own characters, or a new one
/// made from the scenario's character template at the start.
pub const CARRY_ON_WAYS: [&str; 2] = ["own", "create"];

/// The kinds of setting (04 §10).
pub const SETTING_KINDS: [&str; 3] = ["flag", "choice", "number"];

/// The scope every scenario expression shares: who and what it is
/// about, in slot order. `self` and `taker` are the same character: the
/// objective's taker, or the first of the party.
fn subjects(truth: bool) -> Scope {
    let scope = if truth {
        Scope::new().own("self", "entity").own(TAKER, "entity")
    } else {
        Scope::new().mind("self", "entity").own(TAKER, "entity")
    };
    let scope = if truth {
        scope.subject(PARTY, "list<entity>")
    } else {
        scope
    };
    scope
        .subject(START_SITE, "site?")
        .subject(START_CELL, "loc")
        .subject(crate::rule::GLOBAL_SUBJECT, crate::rule::GLOBAL_SUBJECT)
}

/// The scope of conditions that read truth and of effects: `self`,
/// `taker`, `party`, `start_site`, `start_cell`, `global`, and a
/// mission's parameters as locals.
pub fn truth_scope(params: &[(String, String)]) -> Scope {
    let mut scope = subjects(true);
    for (name, ty) in params {
        scope = scope.local(name.clone(), ty.clone());
    }
    scope
}

/// The scope of what the taker is shown: a mind context of the taker
/// (05 §7.3), pure, with `self`, `taker`, `start_site`, `start_cell`,
/// `global` and a
/// mission's parameters as locals.
pub fn mind_scope(params: &[(String, String)]) -> Scope {
    let mut scope = subjects(false).pure();
    for (name, ty) in params {
        scope = scope.local(name.clone(), ty.clone());
    }
    scope
}

// ---- Objectives ------------------------------------------------------------------

/// Where an objective points, as the taker knows it (13 §5.1): one of
/// three, each an expression in the taker's mind context.
#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MarkerDef {
    /// An entity: the marker is where the taker last perceived it, or
    /// was told it was (15 §7.2), and none while it never has.
    #[serde(default)]
    pub last_known: Option<Expr<Any>>,
    /// A site: the marker is the cell that stands for it, its anchor.
    #[serde(default)]
    pub site: Option<Expr<Any>>,
    /// A cell.
    #[serde(default)]
    pub at: Option<Expr<Any>>,
}

impl MarkerDef {
    fn count(&self) -> usize {
        usize::from(self.last_known.is_some())
            + usize::from(self.site.is_some())
            + usize::from(self.at.is_some())
    }
}

/// One objective as a scenario or a mission writes it (13 §5.1).
///
/// ```json
/// { "id": "deliver", "text": "Take the parcel to the town hall.",
///   "done": "dist(global.var.parcel, global.var.errand_to.loc) < 4",
///   "failed": "is_null(global.var.parcel.loc)",
///   "marker": { "site": "global.var.errand_to" }, "expires": "5 h" }
/// ```
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ObjectiveDef {
    /// The objective's name, unique in its scenario or mission.
    pub id: Located<String>,
    /// What the player is told they are trying to do: a template, read
    /// in the taker's mind context.
    pub text: Text,
    /// When it is done: a reactive condition (05 §7.5) that reads truth.
    #[serde(default)]
    pub done: Option<Cond>,
    /// When it has failed: as `done`. Tested first.
    #[serde(default)]
    pub failed: Option<Cond>,
    /// Where it points, as the taker knows it.
    #[serde(default)]
    pub marker: Option<MarkerDef>,
    /// The objective it is part of. One with children and no `done` of
    /// its own is done when they all are.
    #[serde(default)]
    pub parent: Option<String>,
    /// The objectives that must be done before this one is shown: until
    /// then it is hidden.
    #[serde(default)]
    pub after: Vec<String>,
    /// Hidden until an effect shows it (`complete_objective` shows
    /// nothing; a rule may switch it with `show_objective`).
    #[serde(default)]
    pub hidden: bool,
    /// How long it may be active before it fails, in universal time.
    #[serde(default)]
    pub expires: Option<Time>,
    /// What is done when it is done.
    #[serde(default)]
    pub on_done: Effects,
    /// What is done when it fails.
    #[serde(default)]
    pub on_failed: Effects,
}

impl ObjectiveDef {
    fn compile(&mut self, cx: &mut CompileCx<'_>, params: &[(String, String)]) {
        let mind = mind_scope(params);
        let truth = truth_scope(params);
        let reactive = truth.clone().pure();
        cx.compile(&mut self.text, &mind);
        if let Some(done) = &mut self.done {
            cx.compile(done, &reactive);
        }
        if let Some(failed) = &mut self.failed {
            cx.compile(failed, &reactive);
        }
        if let Some(marker) = &mut self.marker {
            for expr in [&mut marker.last_known, &mut marker.site, &mut marker.at]
                .into_iter()
                .flatten()
            {
                cx.compile(expr, &mind);
            }
        }
        cx.compile(&mut self.on_done, &truth);
        cx.compile(&mut self.on_failed, &truth);
    }
}

/// Checks a list of objectives: names unique, `parent` and `after`
/// naming objectives of the same list, a marker of exactly one kind.
fn validate_objectives(objectives: &[ObjectiveDef], cx: &mut ValidateCx<'_>) {
    let names: Vec<&str> = objectives.iter().map(|o| o.id.value.as_str()).collect();
    for (index, objective) in objectives.iter().enumerate() {
        let name = objective.id.value.as_str();
        if name.is_empty() {
            cx.error(format!("objectives {index}: `id` must not be empty"));
        }
        if names.iter().filter(|other| **other == name).count() > 1 {
            cx.error(format!(
                "objectives {index}: there is more than one objective `{name}`"
            ));
        }
        if let Some(parent) = &objective.parent
            && (!names.contains(&parent.as_str()) || parent == name)
        {
            cx.error(format!(
                "objectives {index}: `parent` names `{parent}`, which is no other objective here"
            ));
        }
        for after in &objective.after {
            if !names.contains(&after.as_str()) || after == name {
                cx.error(format!(
                    "objectives {index}: `after` names `{after}`, which is no other objective here"
                ));
            }
        }
        if let Some(marker) = &objective.marker
            && marker.count() != 1
        {
            cx.error(format!(
                "objectives {index}: a `marker` has exactly one of `last_known`, `site` and `at`"
            ));
        }
        if objective
            .expires
            .is_some_and(|time| time.dur() <= Dur::ZERO)
        {
            cx.error(format!(
                "objectives {index}: `expires` must be a time above zero"
            ));
        }
        objective.on_done.validate(cx);
        objective.on_failed.validate(cx);
    }
}

// ---- Scenarios ---------------------------------------------------------------------

/// How a world's seed is chosen (13 §4.1).
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum SeedSpec {
    /// Always this one.
    Fixed(u64),
    /// Drawn afresh by the application for each game.
    Random,
    /// The one the player gives on the new-game screen.
    #[default]
    Ask,
}

impl SeedSpec {
    /// The seed a game is made with, given the one the player gave and
    /// one the application drew.
    pub const fn choose(self, asked: u64, drawn: u64) -> u64 {
        match self {
            SeedSpec::Fixed(seed) => seed,
            SeedSpec::Random => drawn,
            SeedSpec::Ask => asked,
        }
    }

    /// The word data writes, or the number.
    pub fn written(self) -> String {
        match self {
            SeedSpec::Fixed(seed) => seed.to_string(),
            SeedSpec::Random => "random".to_owned(),
            SeedSpec::Ask => "ask".to_owned(),
        }
    }
}

impl<'de> FromKind<'de> for SeedSpec {
    fn from_kind<A: MapAccess<'de>>(
        kind: Kind,
        value: KindValue<'_, A>,
    ) -> Result<SeedSpec, A::Error> {
        match kind {
            Kind::Number => value.read::<u64>().map(SeedSpec::Fixed),
            Kind::String => match value.read::<String>()?.as_str() {
                "random" => Ok(SeedSpec::Random),
                "ask" => Ok(SeedSpec::Ask),
                other => Err(serde::de::Error::custom(format!(
                    "a seed is a whole number, `random` or `ask`, and `{other}` is none of them"
                ))),
            },
            other => Err(other.unexpected("a whole number, `random` or `ask`")),
        }
    }
}

impl<'de> Deserialize<'de> for SeedSpec {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<SeedSpec, D::Error> {
        by_kind(deserializer)
    }
}

/// A world a scenario creates (13 §4.1).
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorldEntry {
    /// The realm definition: a qualified id, or a bare name in the
    /// scenario's namespace.
    pub realm: Located<String>,
    /// What rules call it. Carried; one world is built.
    #[serde(default, rename = "as")]
    pub name: Option<String>,
    /// How its seed is chosen. Default `ask`.
    #[serde(default)]
    pub seed: SeedSpec,
}

/// A character of the party at the start (13 §4.2).
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CharacterEntry {
    /// The template the character is made from with every choice at its
    /// default, when the player has made none of their own: a qualified
    /// id, or a bare name in the scenario's namespace.
    #[serde(default)]
    pub template: Option<Located<String>>,
    /// Whether the player may make the character on the creation screen
    /// (`create`): begin from a template offered and make every choice
    /// it has. Default true.
    #[serde(default = "yes")]
    pub create: bool,
    /// The templates offered to make or choose the character from
    /// (`pick`): `true` for every playable template, a list of templates
    /// for those alone. With `create` false the player chooses one,
    /// ready-made, and gives it no more than a name. Left out, every
    /// playable template where `create` holds, and none where it does
    /// not: the player is given the character of `template`.
    #[serde(default)]
    pub pick: PickEntry,
    /// The senses it perceives with when its templates give it none:
    /// `sense` definitions. Left out, every sense the loaded mods define.
    #[serde(default)]
    pub senses: Vec<Located<String>>,
}

fn yes() -> bool {
    true
}

/// The templates a scenario offers the player its character from
/// (13 §4.2 `pick`).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum PickEntry {
    /// Nothing said: every playable template where the character may
    /// be made, none where it may not.
    #[default]
    Unsaid,
    /// `false`: none.
    Nothing,
    /// `true`: every playable template.
    Playable,
    /// These templates, as written: qualified ids or bare names in the
    /// scenario's namespace.
    Templates(Vec<Located<String>>),
}

impl<'de> FromKind<'de> for PickEntry {
    fn from_kind<A: MapAccess<'de>>(kind: Kind, value: KindValue<'_, A>) -> Result<Self, A::Error> {
        match kind {
            Kind::Bool => value.read::<bool>().map(|all| {
                if all {
                    PickEntry::Playable
                } else {
                    PickEntry::Nothing
                }
            }),
            Kind::List => value
                .read::<Vec<Located<String>>>()
                .map(PickEntry::Templates),
            other => Err(other.unexpected("`true`, `false` or a list of templates")),
        }
    }
}

impl<'de> Deserialize<'de> for PickEntry {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<PickEntry, D::Error> {
        by_kind(deserializer)
    }
}

impl CharacterEntry {
    /// Whether the player makes the character's choices (`create`), or
    /// at most chooses a ready-made one and names it.
    pub const fn editable(&self) -> bool {
        self.create
    }

    /// The qualified ids of the templates offered to make or choose the
    /// character from, for a scenario defined in `namespace`: those
    /// `pick` lists, or every template marked `playable` in order of
    /// qualified id (13 §4.2). Empty when the player is given the
    /// character of `template`.
    pub fn offered(&self, defs: &clash_data::Defs, namespace: &str) -> Vec<String> {
        let all = || {
            let Some(templates) = defs.try_registry::<TemplateDef>() else {
                return Vec::new();
            };
            let mut ids: Vec<String> = templates
                .iter()
                .filter(|(_, def)| def.playable)
                .map(|(id, _)| templates.name(id).to_owned())
                .collect();
            ids.sort();
            ids
        };
        match &self.pick {
            PickEntry::Nothing => Vec::new(),
            PickEntry::Unsaid if !self.create => Vec::new(),
            PickEntry::Unsaid | PickEntry::Playable => all(),
            PickEntry::Templates(listed) => listed
                .iter()
                .map(|template| qualified(&template.value, namespace))
                .collect(),
        }
    }
}

/// How a site is chosen for a place to begin (13 §4.3).
#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SiteQuery {
    /// The site definition it was made from: its bare name or qualified
    /// id.
    #[serde(default)]
    pub def: Option<String>,
    /// A tag it carries.
    #[serde(default)]
    pub tag: Option<String>,
    /// Its name, as shown.
    #[serde(default)]
    pub name: Option<String>,
}

/// One place the party may begin at (13 §4.3).
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PlaceEntry {
    /// What the player is offered it as.
    #[serde(default)]
    pub name: String,
    /// A cell of the realm's base grid: `[x, y]`, the highest cell of
    /// that column a walker can stand in, or `[x, y, z]`.
    #[serde(default)]
    pub cell: Option<Vec<i32>>,
    /// A site chosen by query: the party begins where a walker can stand
    /// in the column of its anchor.
    #[serde(default)]
    pub site: Option<SiteQuery>,
    /// A role at the site. Carried: the world does not record where a
    /// site's marks went yet, so the anchor stands for every role.
    #[serde(default)]
    pub role: Option<String>,
}

/// The time of day a game begins at (13 §4.3).
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TimeOfDay {
    /// The day, from 1.
    #[serde(default = "first_day")]
    pub day: u32,
    /// The time on the clock, `HH:MM`.
    pub at: Located<String>,
}

fn first_day() -> u32 {
    1
}

impl TimeOfDay {
    /// The time since midnight it names, if it is written well.
    pub fn since_midnight(&self) -> Option<Dur> {
        let (hours, minutes) = self.at.value.split_once(':')?;
        let hours: i64 = hours.trim().parse().ok()?;
        let minutes: i64 = minutes.trim().parse().ok()?;
        ((0..24).contains(&hours) && (0..60).contains(&minutes))
            .then(|| Dur::from_secs(hours * 3_600 + minutes * 60))
    }
}

/// Who, where, when and what is set up (13 §4).
#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StartPart {
    /// The party: one character in this build.
    #[serde(default)]
    pub characters: Vec<CharacterEntry>,
    /// The places the player may begin at. With more than one the player
    /// chooses.
    #[serde(default)]
    pub at: Vec<PlaceEntry>,
    /// The day and time the game begins at.
    #[serde(default)]
    pub time: Option<TimeOfDay>,
    /// Effects run once, after the party is placed (13 §4.4), with
    /// `self` and `taker` the first of the party.
    #[serde(default, rename = "do")]
    pub effects: Effects,
}

/// How a game ends (13 §9.4).
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EndEntry {
    /// A reactive condition (05 §7.5) that reads truth: the game ends
    /// when it comes to hold.
    #[serde(default, rename = "if")]
    pub condition: Option<Cond>,
    /// An event: the game ends when one is delivered whose fields equal
    /// `filter`.
    #[serde(default)]
    pub on: Option<Located<String>>,
    /// The fields the event of `on` must have, each with its value.
    #[serde(default)]
    pub filter: OrderedMap<Located<serde_json::Value>>,
    /// The outcome it names.
    #[serde(rename = "as")]
    pub outcome: String,
}

/// What follows when the party is gone (13 §9.3).
#[derive(Debug, Default)]
pub struct PartyGone {
    /// `end`, `continue` or `restore`.
    pub then: String,
    /// For `continue`: the ways of carrying on offered, in order.
    pub by: Vec<String>,
    /// The outcome an ending names. Default `lost`.
    pub outcome: String,
    /// Effects run first.
    pub effects: Effects,
}

/// `on_party_gone` written in full (13 §9.3): what follows, the ways of
/// carrying on, the outcome of an ending and effects run first. Written
/// as a string alone, it is `then` with the rest at their defaults.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PartyGoneObject {
    /// What follows: `end` (an ending), `continue` (the player takes
    /// another character in the same world) or `restore` (the last
    /// checkpoint is restored; with none, the game ends with the reason).
    then: String,
    /// For `continue`: the ways of carrying on tried, in order, the first
    /// that finds a character taken. `own` is another of the party still
    /// alive, `create` a new character from the scenario's character
    /// template at the place the party began. Default both, in that
    /// order.
    #[serde(default)]
    by: Vec<String>,
    /// The outcome an ending names, as `ends` names one. Default `lost`.
    #[serde(default, rename = "as")]
    outcome: Option<String>,
    /// Effects run once when the party is gone, before what follows, with
    /// the last of the party as `taker`.
    #[serde(default, rename = "do")]
    effects: Effects,
}

/// The outcome an ending names when the party is gone and nothing says
/// another.
pub const LOST: &str = "lost";

impl<'de> FromKind<'de> for PartyGone {
    fn from_kind<A: MapAccess<'de>>(
        kind: Kind,
        value: KindValue<'_, A>,
    ) -> Result<PartyGone, A::Error> {
        match kind {
            Kind::String => value.read::<String>().map(|then| PartyGone {
                then,
                by: Vec::new(),
                outcome: LOST.to_owned(),
                effects: Effects::default(),
            }),
            Kind::Object => value.read::<PartyGoneObject>().map(|object| PartyGone {
                then: object.then,
                by: object.by,
                outcome: object.outcome.unwrap_or_else(|| LOST.to_owned()),
                effects: object.effects,
            }),
            other => {
                Err(other.unexpected("`end`, `continue`, `restore`, or an object with `then`"))
            }
        }
    }
}

impl<'de> Deserialize<'de> for PartyGone {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<PartyGone, D::Error> {
        by_kind(deserializer)
    }
}

/// How a game is saved (13 §9.2, §9.3).
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SavingDef {
    /// `free`, `single`, `checkpoints` or `none`. Default `free`.
    #[serde(default = "free")]
    pub policy: String,
    /// What follows when the party is gone. Default `end`.
    #[serde(default = "party_gone_end")]
    pub on_party_gone: PartyGone,
}

fn free() -> String {
    SAVING_POLICIES[0].to_owned()
}

fn party_gone_end() -> PartyGone {
    PartyGone {
        then: PARTY_GONE_OUTCOMES[0].to_owned(),
        by: Vec::new(),
        outcome: LOST.to_owned(),
        effects: Effects::default(),
    }
}

impl Default for SavingDef {
    fn default() -> SavingDef {
        SavingDef {
            policy: free(),
            on_party_gone: party_gone_end(),
        }
    }
}

/// A game scenario (13 §3): the worlds a game has, who the player is and
/// where they begin, the rules in force, what the player is trying to
/// do and how it ends.
///
/// ```json
/// { "type": "scenario", "id": "the_errand", "name": "The errand",
///   "doc": "Take a parcel across town before nightfall.",
///   "worlds": [ { "realm": "town", "seed": "ask" } ],
///   "start": { "characters": [ { "template": "person" } ],
///              "at": [ { "name": "The north street", "cell": [2040, 898] } ],
///              "time": { "day": 1, "at": "14:00" } },
///   "objectives": [ { "id": "deliver", "text": "Take the parcel to the town hall." } ],
///   "ends": [ { "on": "objective_changed", "filter": { "id": "deliver", "state": "done" }, "as": "delivered" } ],
///   "saving": { "policy": "free", "on_party_gone": "end" } }
/// ```
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ScenarioDef {
    /// What it is called where the player chooses. Left out, its id.
    #[serde(default)]
    pub name: String,
    /// A line or two about it.
    #[serde(default)]
    pub doc: String,
    /// What the introduction screen says when the game begins. Left out,
    /// no introduction is shown.
    #[serde(default)]
    pub intro: String,
    /// Tags, by which the new-game screen may group scenarios.
    #[serde(default)]
    pub tags: Vec<String>,
    /// The realms created when the game begins: one in this build.
    #[serde(default)]
    pub worlds: Vec<WorldEntry>,
    /// The settings the player is asked about before the game begins:
    /// `setting` definitions.
    #[serde(default)]
    pub options: Vec<Located<String>>,
    /// The values settings begin with in this scenario, by key.
    #[serde(default)]
    pub settings: OrderedMap<Located<serde_json::Value>>,
    /// The settings the player may not change in this scenario.
    #[serde(default)]
    pub locked: Vec<String>,
    /// Who, where, when and what is set up.
    #[serde(default)]
    pub start: StartPart,
    /// The rule groups in force only in this scenario: switched on when
    /// it begins, and every group another scenario names switched off.
    #[serde(default)]
    pub rules: Vec<Located<String>>,
    /// What the player is trying to do.
    #[serde(default)]
    pub objectives: Vec<ObjectiveDef>,
    /// How the game ends.
    #[serde(default)]
    pub ends: Vec<EndEntry>,
    /// How the game is saved, and what follows when the party is gone.
    #[serde(default)]
    pub saving: SavingDef,
    /// Screens to use in place of the bundled ones, by role:
    /// `intro`, `summary`. Carried for the application.
    #[serde(default)]
    pub screens: OrderedMap<String>,
}

/// A name as written in a definition, qualified: a bare name means the
/// one in the namespace it is written in.
pub fn qualified(name: &str, namespace: &str) -> String {
    if name.contains(':') {
        name.to_owned()
    } else {
        format!("{namespace}:{name}")
    }
}

/// The namespace of a qualified id: what is before its first `:`.
pub fn namespace_of(id: &str) -> &str {
    id.split_once(':').map_or("", |(namespace, _)| namespace)
}

impl ScenarioDef {
    /// The world the game is made in: the first of `worlds`.
    pub fn world(&self) -> Option<&WorldEntry> {
        self.worlds.first()
    }

    /// The qualified id of the realm definition of its world, for a
    /// scenario defined in `namespace`.
    pub fn realm_id(&self, namespace: &str) -> Option<String> {
        self.world()
            .map(|world| qualified(&world.realm.value, namespace))
    }

    /// The first character of the party.
    pub fn character(&self) -> Option<&CharacterEntry> {
        self.start.characters.first()
    }

    /// The qualified id of the template the first character is made
    /// from by default, if it names one.
    pub fn template_id(&self, namespace: &str) -> Option<String> {
        self.character()
            .and_then(|character| character.template.as_ref())
            .map(|template| qualified(&template.value, namespace))
    }

    /// The qualified ids of the senses the first character is given when
    /// its templates give none.
    pub fn sense_ids(&self, namespace: &str) -> Vec<String> {
        self.character()
            .map(|character| {
                character
                    .senses
                    .iter()
                    .map(|sense| qualified(&sense.value, namespace))
                    .collect()
            })
            .unwrap_or_default()
    }

    /// The saving policy.
    pub fn saving_policy(&self) -> &str {
        &self.saving.policy
    }

    /// The name the player is shown: `name`, or else the id given.
    pub fn shown_name(&self, id: &str) -> String {
        if self.name.is_empty() {
            id.to_owned()
        } else {
            self.name.clone()
        }
    }

    /// The names of the places the player may begin at, in order; a
    /// place with no name is called by its number from one.
    pub fn place_names(&self) -> Vec<String> {
        self.start
            .at
            .iter()
            .enumerate()
            .map(|(index, place)| {
                if place.name.is_empty() {
                    format!("{}", index + 1)
                } else {
                    place.name.clone()
                }
            })
            .collect()
    }
}

impl Def for ScenarioDef {
    type Id = ScenarioId;
    const TYPE: &'static str = "scenario";
    const DOC: &'static str = "A game the player can start: its worlds, who the player is and where they begin, the rules in force, its objectives, how it ends and how it is saved (13 §3).";

    fn compile(&mut self, cx: &mut CompileCx<'_>) {
        let truth = truth_scope(&[]);
        cx.compile(&mut self.start.effects, &truth);
        for objective in &mut self.objectives {
            objective.compile(cx, &[]);
        }
        let reactive = truth.clone().pure();
        for end in &mut self.ends {
            if let Some(condition) = &mut end.condition {
                cx.compile(condition, &reactive);
            }
        }
        cx.compile(&mut self.saving.on_party_gone.effects, &truth);
    }

    fn validate(&self, cx: &mut ValidateCx<'_>) {
        if self.worlds.len() != 1 {
            cx.error(format!(
                "`worlds` names the one realm the game is made in, and this has {}: several worlds wait for charts (10 §9.4)",
                self.worlds.len()
            ));
        }
        if self.start.characters.len() != 1 {
            cx.error(format!(
                "`start.characters` has the one character the player begins as, and this has {}: parties wait for package SC2",
                self.start.characters.len()
            ));
        }
        if self.start.at.is_empty() {
            cx.error("`start.at` names at least one place to begin at");
        }
        for (index, place) in self.start.at.iter().enumerate() {
            match (&place.cell, &place.site) {
                (Some(cell), None) if (2..=3).contains(&cell.len()) => {}
                (Some(cell), None) => cx.error(format!(
                    "`start.at` {index}: `cell` is `[x, y]` or `[x, y, z]`, and this has {} numbers",
                    cell.len()
                )),
                (None, Some(_)) => {}
                _ => cx.error(format!(
                    "`start.at` {index}: a place is exactly one of `cell` and `site`"
                )),
            }
        }
        if let Some(time) = &self.start.time {
            if time.since_midnight().is_none() {
                cx.error(format!(
                    "`start.time.at` is a time on the clock, `HH:MM`, and `{}` is not one",
                    time.at.value
                ));
            }
            if time.day == 0 {
                cx.error("`start.time.day` counts from 1");
            }
        }
        if !SAVING_POLICIES.contains(&self.saving.policy.as_str()) {
            cx.error(format!(
                "`saving.policy` is one of {}, and `{}` is none of them",
                SAVING_POLICIES.join(", "),
                self.saving.policy
            ));
        }
        let gone = &self.saving.on_party_gone;
        if !PARTY_GONE_OUTCOMES.contains(&gone.then.as_str()) {
            cx.error(format!(
                "`saving.on_party_gone` is one of {}, and `{}` is none of them",
                PARTY_GONE_OUTCOMES.join(", "),
                gone.then
            ));
        }
        for way in &gone.by {
            if !CARRY_ON_WAYS.contains(&way.as_str()) {
                cx.error(format!(
                    "`saving.on_party_gone.by` takes {}, and `{way}` is none of them (`become` waits for package SC2)",
                    CARRY_ON_WAYS.join(", ")
                ));
            }
        }
        gone.effects.validate(cx);
        self.start.effects.validate(cx);
        validate_objectives(&self.objectives, cx);
        for (index, end) in self.ends.iter().enumerate() {
            if end.condition.is_some() == end.on.is_some() {
                cx.error(format!(
                    "`ends` {index}: an ending has exactly one of `if` and `on`"
                ));
            }
            if end.on.is_none() && !end.filter.0.is_empty() {
                cx.error(format!("`ends` {index}: `filter` goes with `on`"));
            }
            if end.outcome.is_empty() {
                cx.error(format!("`ends` {index}: `as` names the outcome"));
            }
        }
        for key in &self.locked {
            if key.is_empty() {
                cx.error("`locked` names settings by key, and one is empty");
            }
        }
    }
}

// ---- Missions ------------------------------------------------------------------------

/// A mission (13 §5.2): a template for objectives, started with
/// parameters.
///
/// ```json
/// { "type": "mission", "id": "fetch", "name": "Fetch {$what}",
///   "params": { "what": "string", "from": "site" },
///   "objectives": [ { "id": "go", "text": "Go to {$from.name}.", "marker": { "site": "$from" } } ],
///   "expires": "1 d" }
/// ```
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MissionDef {
    /// What the journal calls it: a template in the taker's mind
    /// context, with the parameters.
    #[serde(default)]
    pub name: Option<Text>,
    /// A line about it.
    #[serde(default)]
    pub doc: String,
    /// The parameters it is started with, each with its type as the
    /// expression language writes types. They are locals, `$name`, in
    /// every expression of the mission.
    #[serde(default)]
    pub params: OrderedMap<Located<String>>,
    /// Its objectives, in order. Ordering is by each one's `after`.
    #[serde(default)]
    pub objectives: Vec<ObjectiveDef>,
    /// How long it may run before it expires, in universal time.
    #[serde(default)]
    pub expires: Option<Time>,
    /// When it has failed beside a failed objective: a reactive
    /// condition that reads truth.
    #[serde(default)]
    pub failed: Option<Cond>,
    /// What is done when it is done: every objective done.
    #[serde(default)]
    pub on_done: Effects,
    /// What is done when it fails.
    #[serde(default)]
    pub on_failed: Effects,
    /// What is done when it expires.
    #[serde(default)]
    pub on_expired: Effects,
}

impl MissionDef {
    /// The parameters: names and types, in order.
    pub fn param_list(&self) -> Vec<(String, String)> {
        self.params
            .0
            .iter()
            .map(|(name, ty)| (name.clone(), ty.value.clone()))
            .collect()
    }
}

impl Def for MissionDef {
    type Id = MissionId;
    const TYPE: &'static str = "mission";
    const DOC: &'static str = "A template for objectives, started with parameters by `start_mission`: a bounty, a delivery, a job (13 §5.2).";

    fn compile(&mut self, cx: &mut CompileCx<'_>) {
        let params = self.param_list();
        if let Some(name) = &mut self.name {
            cx.compile(name, &mind_scope(&params));
        }
        for objective in &mut self.objectives {
            objective.compile(cx, &params);
        }
        let truth = truth_scope(&params);
        if let Some(failed) = &mut self.failed {
            cx.compile(failed, &truth.clone().pure());
        }
        cx.compile(&mut self.on_done, &truth);
        cx.compile(&mut self.on_failed, &truth);
        cx.compile(&mut self.on_expired, &truth);
    }

    fn validate(&self, cx: &mut ValidateCx<'_>) {
        if self.objectives.is_empty() {
            cx.error("a mission has at least one objective");
        }
        validate_objectives(&self.objectives, cx);
        if self.expires.is_some_and(|time| time.dur() <= Dur::ZERO) {
            cx.error("`expires` must be a time above zero");
        }
        self.on_done.validate(cx);
        self.on_failed.validate(cx);
        self.on_expired.validate(cx);
    }
}

// ---- Settings --------------------------------------------------------------------------

/// A setting (04 §10): a choice content offers players, which rules
/// read with `setting(id)`.
///
/// ```json
/// { "type": "setting", "id": "night_falls", "name": "Night falls",
///   "kind": "choice", "options": ["early", "late"], "default": "late" }
/// ```
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SettingDef {
    /// What the player is shown. Left out, its id.
    #[serde(default)]
    pub name: String,
    /// `flag`, `choice` or `number`.
    pub kind: String,
    /// For a `choice`: the values it takes.
    #[serde(default)]
    pub options: Vec<String>,
    /// For a `number`: the least and the greatest value.
    #[serde(default)]
    pub range: Option<[Real; 2]>,
    /// The value it has until something sets it.
    pub default: serde_json::Value,
    /// What it does.
    #[serde(default)]
    pub doc: String,
}

impl SettingDef {
    /// Whether a value is one the setting takes; if not, why.
    pub fn check(&self, value: &serde_json::Value) -> Result<(), String> {
        match self.kind.as_str() {
            "flag" if value.is_boolean() => Ok(()),
            "flag" => Err("a flag is true or false".to_owned()),
            "choice" => match value.as_str() {
                Some(text) if self.options.iter().any(|option| option == text) => Ok(()),
                _ => Err(format!(
                    "this choice takes one of {}",
                    self.options.join(", ")
                )),
            },
            "number" => {
                let Some(number) = value.as_f64() else {
                    return Err("a number is wanted".to_owned());
                };
                let number = Real::of(number);
                match self.range {
                    Some([low, high]) if number < low || number > high => {
                        Err(format!("this number is from {low} to {high}"))
                    }
                    _ => Ok(()),
                }
            }
            _ => Err("the setting is of no kind there is".to_owned()),
        }
    }
}

impl Def for SettingDef {
    type Id = SettingId;
    const TYPE: &'static str = "setting";
    const DOC: &'static str = "A choice content offers players, read by rules with `setting(id)` and changed only by the `Setting` command (04 §10).";

    fn validate(&self, cx: &mut ValidateCx<'_>) {
        if !SETTING_KINDS.contains(&self.kind.as_str()) {
            cx.error(format!(
                "`kind` is one of {}, and `{}` is none of them",
                SETTING_KINDS.join(", "),
                self.kind
            ));
            return;
        }
        if self.kind == "choice" && self.options.is_empty() {
            cx.error("a `choice` lists its `options`");
        }
        if let Some([low, high]) = self.range
            && low > high
        {
            cx.error("`range` is the least value and then the greatest");
        }
        if let Err(why) = self.check(&self.default) {
            cx.error(format!("`default` is not a value the setting takes: {why}"));
        }
    }
}

/// The value a JSON value of data stands for, as settings hold it.
pub fn stored_value(value: &serde_json::Value) -> Option<clash_core::Value> {
    match value {
        serde_json::Value::Bool(flag) => Some(clash_core::Value::Bool(*flag)),
        serde_json::Value::Number(number) => {
            number.as_i64().map(clash_core::Value::Int).or_else(|| {
                number
                    .as_f64()
                    .map(|real| clash_core::Value::Real(Real::of(real)))
            })
        }
        serde_json::Value::String(text) => Some(clash_core::Value::Str(text.clone())),
        _ => None,
    }
}

// ---- Cross checks ------------------------------------------------------------------------

/// Every reference of a scenario must name something: its realm, its
/// character's template and senses (where those are loaded), its rule
/// groups and the settings it asks about. A setting it gives a value is
/// checked against the setting's definition, if there is one.
pub(crate) fn check_scenarios(cx: &mut CrossCx<'_>) {
    let defs = cx.defs();
    let templates = defs.iter::<TemplateDef>().next().is_some();
    let senses = defs.iter::<SenseDef>().next().is_some();
    let mut problems: Vec<(String, String, String, clash_data::Span)> = Vec::new();
    for (id, scenario) in defs.iter::<ScenarioDef>() {
        let Some(name) = defs.name::<ScenarioDef>(id) else {
            continue;
        };
        let namespace = namespace_of(name);
        let mut missing = |message: String, path: &str, span: clash_data::Span| {
            problems.push((name.to_owned(), message, path.to_owned(), span));
        };
        for (index, world) in scenario.worlds.iter().enumerate() {
            let realm = qualified(&world.realm.value, namespace);
            if defs.lookup::<RealmDef>(&realm).is_none() {
                missing(
                    format!("there is no realm `{realm}` for this scenario to be made in"),
                    &format!("worlds.{index}.realm"),
                    world.realm.span,
                );
            }
        }
        for (index, character) in scenario.start.characters.iter().enumerate() {
            if let (true, Some(template)) = (templates, &character.template) {
                let id = qualified(&template.value, namespace);
                if defs.lookup::<TemplateDef>(&id).is_none() {
                    missing(
                        format!(
                            "there is no template `{id}` to make this scenario's character from"
                        ),
                        &format!("start.characters.{index}.template"),
                        template.span,
                    );
                }
            }
            if let (true, PickEntry::Templates(listed)) = (templates, &character.pick) {
                for template in listed {
                    let id = qualified(&template.value, namespace);
                    if defs.lookup::<TemplateDef>(&id).is_none() {
                        missing(
                            format!(
                                "there is no template `{id}` to offer as this scenario's character"
                            ),
                            &format!("start.characters.{index}.pick"),
                            template.span,
                        );
                    }
                }
            }
            for sense in character.senses.iter().filter(|_| senses) {
                let id = qualified(&sense.value, namespace);
                if defs.lookup::<SenseDef>(&id).is_none() {
                    missing(
                        format!("there is no sense `{id}` to give this scenario's character"),
                        &format!("start.characters.{index}.senses"),
                        sense.span,
                    );
                }
            }
        }
        for group in &scenario.rules {
            let id = qualified(&group.value, namespace);
            if defs.lookup::<RuleGroupDef>(&id).is_none() {
                missing(
                    format!("there is no rule group `{id}` for this scenario to put in force"),
                    "rules",
                    group.span,
                );
            }
        }
        for option in &scenario.options {
            let id = qualified(&option.value, namespace);
            if defs.lookup::<SettingDef>(&id).is_none() {
                missing(
                    format!("there is no setting `{id}` to ask the player about"),
                    "options",
                    option.span,
                );
            }
        }
        for (key, value) in &scenario.settings.0 {
            if let Some(setting) = defs
                .lookup::<SettingDef>(key)
                .and_then(|setting| defs.get::<SettingDef>(setting))
                && let Err(why) = setting.check(&value.value)
            {
                missing(
                    format!("`{key}` is given a value it does not take: {why}"),
                    "settings",
                    value.span,
                );
            }
        }
    }
    for (scenario, message, path, span) in problems {
        let diagnostic = Diagnostic::new(code::UNKNOWN_REFERENCE, message)
            .in_def(Some(&DefLabel::new(ScenarioDef::TYPE, scenario)))
            .with_path(path)
            .at(cx.sources(), span);
        cx.report(diagnostic);
    }
}
