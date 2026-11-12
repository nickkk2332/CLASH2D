//! The definition types of minds (09 §3, §4): `mind`, `goal` and
//! `method`, and the steps a method is made of.
//!
//! Everything a goal or a method holds is compiled in the **mind
//! context** of the character (05 §7.3): `self` is its own and may be
//! read in full; anyone else may be read only through what `self`
//! believes (`self.believed_loc(x)`, `self.confidence(x)`). A read of
//! another entity's state is a load error naming the read (invariant
//! N1).

use std::collections::BTreeMap;

use clash_core::{Dur, Real, define_id};
use clash_data::expr::{Any, Number};
use clash_data::{CompileCx, Cond, Def, Expr, FinishCx, Ref, Scope, Text, Time, ValidateCx};
use serde::Deserialize;

use crate::action::def::ActionDef;

define_id! {
    /// A `mind` definition (09 §3.1).
    pub struct MindDefId(u16);
    /// A `goal` definition (09 §3.2).
    pub struct GoalId(u16);
    /// A `method` definition (09 §4).
    pub struct MethodId(u16);
}

/// The kind of subject a mind is.
pub const ENTITY_KIND: &str = "entity";

/// How long a mind waits between thoughts when nothing happens, if its
/// definition does not say.
pub const DEFAULT_THINK: Dur = Dur::SECOND;

/// How many times its `think` a quiet mind's interval may stretch to,
/// if its definition does not say.
pub const DEFAULT_STRETCH: i64 = 8;

/// How long a goal that failed is set aside, if it does not say.
pub const DEFAULT_RETRY_AFTER: Dur = Dur::from_secs(10);

/// How close a `go` step must come, in metres, if it does not say.
pub const DEFAULT_WITHIN_M: f64 = 0.0;

/// How far a `keep_from` step goes from what it keeps from, in metres,
/// if it does not say.
pub const DEFAULT_KEEP_FROM_M: f64 = 12.0;

/// How near a `keep_near` step stays, in metres, if it does not say.
pub const DEFAULT_KEEP_NEAR_M: f64 = 1.5;

fn default_think() -> Time {
    Time(DEFAULT_THINK)
}

fn default_stretch() -> i64 {
    DEFAULT_STRETCH
}

fn default_retry() -> Time {
    Time(DEFAULT_RETRY_AFTER)
}

fn one() -> Real {
    Real::ONE
}

fn keep_from_default() -> Real {
    Real::of(DEFAULT_KEEP_FROM_M)
}

fn keep_near_default() -> Real {
    Real::of(DEFAULT_KEEP_NEAR_M)
}

/// What a goal is about (09 §3.2): chosen from what the character
/// believes each time it thinks, and bound as `$about` for the rest of
/// the goal. Exactly one of the three.
///
/// | Written | `$about` is | `it` in `where` and `best` |
/// |---------|-------------|----------------------------|
/// | `{ "track": { "where": …, "best": … } }` | An entity the mind has a track of (`entity?`) | That entity: read it only through `self`'s belief properties |
/// | `{ "memory": { "events": […], "where": …, "best": … } }` | A memory, as a map (`map<any>?`) | The memory: `event`, `loc`, `age`, `salience`, `confidence`, `actor`, `how` |
/// | `{ "place": "…" }` | A cell (`loc?`) | — |
///
/// Of the candidates whose `where` holds, the one with the greatest
/// `best` is chosen; on a tie, the first in the order the mind keeps
/// them (tracks by persona, memories oldest first). A goal whose
/// `about` finds nothing is not wanted at all. A track or a memory the
/// mind has **settled** (the `settle` step) is passed over until it
/// changes: a track until it is perceived again, a memory for good.
#[derive(Debug, Default, Deserialize)]
pub struct About {
    /// One of the mind's tracks.
    #[serde(default)]
    pub track: Option<Pick>,
    /// One of the mind's memories.
    #[serde(default)]
    pub memory: Option<MemoryPick>,
    /// A place, worked out in the mind context.
    #[serde(default)]
    pub place: Option<Expr<Any>>,
}

/// How a track is picked (see [`About`]).
#[derive(Debug, Default, Deserialize)]
pub struct Pick {
    /// Which candidates may be chosen. Left out, all.
    #[serde(default, rename = "where")]
    pub filter: Option<Cond>,
    /// What is maximised. Left out, the first candidate is taken.
    #[serde(default)]
    pub best: Option<Expr<Number>>,
}

/// How a memory is picked (see [`About`]).
#[derive(Debug, Default, Deserialize)]
pub struct MemoryPick {
    /// The event types a memory must be of. Left out, any.
    #[serde(default)]
    pub events: Vec<String>,
    /// Which candidates may be chosen. Left out, all.
    #[serde(default, rename = "where")]
    pub filter: Option<Cond>,
    /// What is maximised. Left out, the most recent is taken.
    #[serde(default)]
    pub best: Option<Expr<Number>>,
}

/// What a goal's `about` binds, for its scope.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum AboutKind {
    /// Nothing: `$about` is null.
    None,
    /// An entity.
    Track,
    /// A memory, as a map.
    Memory,
    /// A cell.
    Place,
}

impl AboutKind {
    /// The type `$about` has. A goal with an `about` is scored only
    /// when something was found, so it is not null there; a goal held
    /// whose `about` is no longer believed is given up.
    pub const fn type_name(self) -> &'static str {
        match self {
            AboutKind::None => "any",
            AboutKind::Track => "entity",
            AboutKind::Memory => "map<any>",
            AboutKind::Place => "loc",
        }
    }
}

impl About {
    /// What it binds.
    pub fn kind(&self) -> AboutKind {
        if self.track.is_some() {
            AboutKind::Track
        } else if self.memory.is_some() {
            AboutKind::Memory
        } else if self.place.is_some() {
            AboutKind::Place
        } else {
            AboutKind::None
        }
    }
}

/// The mind context of a goal or a method: `self` its own, `$about` as
/// the goal binds it (05 §7.3).
pub fn mind_scope(about: AboutKind) -> Scope {
    Scope::new()
        .mind("self", ENTITY_KIND)
        .local("about", about.type_name())
}

/// The mind context of an `about`'s `where` and `best`, with `it` the
/// candidate.
pub fn pick_scope(about: AboutKind) -> Scope {
    let it = match about {
        AboutKind::Memory => "map<any>",
        _ => ENTITY_KIND,
    };
    Scope::new().mind("self", ENTITY_KIND).subject("it", it)
}

/// The mind context of a mind's `adjust` conditions.
pub fn self_scope() -> Scope {
    Scope::new().mind("self", ENTITY_KIND)
}

/// A change to a mind while a condition on the character holds (09 §3.1:
/// traits adjust weights and add goals).
///
/// ```json
/// { "if": "self.has_tag('frightened')", "weights": { "caution": 0.5 }, "goals": ["hide"] }
/// ```
#[derive(Debug, Deserialize)]
pub struct Adjust {
    /// When it applies: a condition on the character itself.
    #[serde(rename = "if")]
    pub when: Cond,
    /// Added to the mind's weights of the same names.
    #[serde(default)]
    pub weights: BTreeMap<String, Real>,
    /// Goals it may have besides the mind's own.
    #[serde(default)]
    pub goals: Vec<Ref<GoalDef>>,
}

/// A mind (09 §3.1): which goals a character may have, the numbers that
/// colour its choices, how often it reconsiders, and how it stands
/// toward others until relations are built.
///
/// ```json
/// { "type": "mind", "id": "watchman",
///   "goals": ["patrol", "investigate", "confront", "flee"],
///   "weights": { "duty": 0.9, "caution": 0.4 },
///   "think": "1 s",
///   "go_action": "go_to",
///   "hostile_to": ["outlaw"], "hostile_after": ["hit_landed"] }
/// ```
#[derive(Debug, Deserialize)]
pub struct MindDef {
    /// The goals it may have, in the order ties are broken.
    pub goals: Vec<Ref<GoalDef>>,
    /// Numbers that colour its choices, read by `self.weight(name)`.
    #[serde(default)]
    pub weights: BTreeMap<String, Real>,
    /// Changes while a condition on the character holds, in order.
    #[serde(default)]
    pub adjust: Vec<Adjust>,
    /// How long it waits between thoughts when nothing happens, in its
    /// own time. Each mind's first wait is staggered by a hash of its
    /// entity.
    #[serde(default = "default_think")]
    pub think: Time,
    /// How many times `think` its interval may stretch to while nothing
    /// changes. At least 1.
    #[serde(default = "default_stretch")]
    pub stretch: i64,
    /// How clear a percept must be (received over threshold, 15 §6) to
    /// make it think at once.
    #[serde(default = "one")]
    pub wake_clarity: Real,
    /// The action a `go` step starts, given the cell as its parameter
    /// `to`. A mind without one cannot pursue a `go`, `keep_near` or
    /// `keep_from` step.
    #[serde(default)]
    pub go_action: Option<Ref<ActionDef>>,
    /// Entity tags whose bearers it holds hostile, as it perceives them.
    #[serde(default)]
    pub hostile_to: Vec<String>,
    /// Entity tags whose bearers it fears, as it perceives them.
    #[serde(default)]
    pub fear_of: Vec<String>,
    /// Events whose actor, when it perceives who that was, it holds
    /// hostile from then on.
    #[serde(default)]
    pub hostile_after: Vec<String>,
    /// Events whose actor, when it perceives who that was, it fears from
    /// then on.
    #[serde(default)]
    pub fear_after: Vec<String>,
    /// What a step into a cell where it saw someone in the last few
    /// seconds costs it more, in metres of walking (09 §5.3): others are
    /// stepped round, not taken for walls.
    #[serde(default = "default_crowd_cost")]
    pub crowd_cost: Real,
    /// What it routes round, by what it remembers (09 §5.3).
    #[serde(default)]
    pub hazards: Vec<Hazard>,
    /// Senses its character is given when its mind first thinks, if it
    /// has none (15 §4), by id as written: what a character that thinks
    /// by this mind perceives with when its templates gave it nothing
    /// to perceive with. A directly controlled character never thinks,
    /// and is given its senses by whoever made it.
    #[serde(default)]
    pub senses: Vec<String>,
    /// The reactions (03 §7.3) it answers a window it noticed with, in
    /// order of preference: the first that may start and would be ready
    /// in time, else the first that may start. Left out, it answers
    /// nothing.
    #[serde(default)]
    pub reactions: Vec<Ref<ActionDef>>,
    /// What it is, for whoever reads the definitions.
    #[serde(default)]
    pub doc: Option<String>,
}

/// What a step into a cell where a mind saw someone costs, if its
/// definition does not say, in metres.
pub const DEFAULT_CROWD_COST_M: f64 = 3.0;

fn default_crowd_cost() -> Real {
    Real::of(DEFAULT_CROWD_COST_M)
}

fn default_hazard_radius() -> Real {
    Real::of(2.0)
}

fn default_hazard_cost() -> Real {
    Real::of(10.0)
}

fn default_hazard_floor() -> Real {
    Real::of(0.05)
}

/// Something a mind routes round (09 §5.3): wherever it remembers one
/// of these events, while the memory's salience is at least `below`,
/// every step within `radius` metres costs `cost` metres more.
///
/// ```json
/// { "events": ["release_started", "field_appeared"], "radius": 3, "cost": 20 }
/// ```
#[derive(Debug, Deserialize)]
pub struct Hazard {
    /// The event types of the memories that mark it.
    pub events: Vec<String>,
    /// How far round the remembered place it reaches, in metres.
    #[serde(default = "default_hazard_radius")]
    pub radius: Real,
    /// What a step within it costs more, in metres of walking.
    #[serde(default = "default_hazard_cost")]
    pub cost: Real,
    /// The salience below which the memory no longer marks it.
    #[serde(default = "default_hazard_floor")]
    pub below: Real,
}

impl Def for MindDef {
    type Id = MindDefId;
    const TYPE: &'static str = "mind";
    const DOC: &'static str = "A mind: the goals a character may have, the weights that colour its choices, how often it thinks, and how it stands toward others (09 §3.1).";

    fn compile(&mut self, cx: &mut CompileCx<'_>) {
        for adjust in &mut self.adjust {
            cx.compile(&mut adjust.when, &self_scope().pure());
        }
    }

    fn validate(&self, cx: &mut ValidateCx<'_>) {
        if self.think.dur() <= Dur::ZERO {
            cx.error("`think` is a time above zero");
        }
        if self.stretch < 1 {
            cx.error("`stretch` is a whole number of at least 1");
        }
        if !(self.wake_clarity.is_finite() && self.wake_clarity >= Real::ZERO) {
            cx.error("`wake_clarity` is a number of zero or more");
        }
        // A sense is checked where senses are loaded at all.
        let known: Vec<&str> = cx
            .defs()
            .iter::<crate::perception::SenseDef>()
            .filter_map(|(id, _)| cx.defs().name::<crate::perception::SenseDef>(id))
            .collect();
        for sense in &self.senses {
            let found = known.iter().any(|name| {
                *name == sense.as_str() || name.rsplit(':').next() == Some(sense.as_str())
            });
            if !known.is_empty() && !found {
                cx.error(format!("there is no sense `{sense}`"));
            }
        }
        if !(self.crowd_cost.is_finite() && self.crowd_cost >= Real::ZERO) {
            cx.error("`crowd_cost` is a number of metres, zero or more");
        }
        for hazard in &self.hazards {
            let fine = [hazard.radius, hazard.cost, hazard.below]
                .into_iter()
                .all(|value| value.is_finite() && value >= Real::ZERO);
            if !fine {
                cx.error("a hazard's `radius`, `cost` and `below` are numbers of zero or more");
            }
        }
        for (name, weight) in &self.weights {
            if !weight.is_finite() {
                cx.error(format!("the weight `{name}` is not a finite number"));
            }
        }
        let moves = self.goals.iter().any(|goal| {
            cx.defs()
                .get::<GoalDef>(goal.id())
                .is_some_and(|goal| goal.moves(cx.defs()))
        });
        if moves && self.go_action.is_none() {
            cx.error(
                "a goal of this mind has a `go`, `keep_near` or `keep_from` step, and the mind names no `go_action`",
            );
        }
    }
}

/// A goal (09 §3.2): how much the character wants it now, what it is
/// about, and the methods that pursue it.
///
/// ```json
/// { "type": "goal", "id": "investigate",
///   "about": { "memory": { "events": ["element_destroyed"], "where": "it.age < 60" } },
///   "score": "self.weight('duty') * $about.salience",
///   "methods": [ { "steps": [ { "go": { "to": "$about.loc", "within": 2 } },
///                             { "look": { "time": "3 s" } }, { "settle": true } ] } ],
///   "commit": 0.25, "interrupts": true }
/// ```
#[derive(Debug, Deserialize)]
pub struct GoalDef {
    /// How much the character wants it now. Zero or below, or null,
    /// means not at all.
    pub score: Expr<Number>,
    /// What it concerns. Left out, `$about` is null.
    #[serde(default)]
    pub about: About,
    /// The ways to pursue it, in order: the first whose `if` holds and
    /// that has not failed in this pursuit is used.
    #[serde(default)]
    pub methods: Vec<MethodEntry>,
    /// When it is over and succeeded.
    #[serde(default)]
    pub done: Option<Cond>,
    /// When it is over and failed.
    #[serde(default)]
    pub fail: Option<Cond>,
    /// How much another goal must outscore it, as a fraction of its
    /// score, to displace it once it is held.
    #[serde(default)]
    pub commit: Real,
    /// Whether adopting it stops what the character is doing, rather
    /// than waiting for its action to end.
    #[serde(default)]
    pub interrupts: bool,
    /// How long it is set aside after it failed.
    #[serde(default = "default_retry")]
    pub retry_after: Time,
    /// A random amount up to this much is added to its score each time
    /// it is scored, from the character's own stream: the chance in a
    /// character's choices (09 §1).
    #[serde(default)]
    pub noise: Real,
    /// Events that make a mind that may have this goal think at once,
    /// when they bind that mind as one of their subjects.
    #[serde(default)]
    pub wake_on: Vec<String>,
    /// What it is, for whoever reads the definitions.
    #[serde(default)]
    pub doc: Option<String>,
}

impl GoalDef {
    /// Whether any of its methods has a step that moves the character.
    fn moves(&self, defs: &clash_data::DefsView<'_>) -> bool {
        self.methods.iter().any(|entry| {
            entry.steps.iter().any(Step::moves)
                || entry
                    .method
                    .and_then(|method| defs.get::<MethodDef>(method.id()))
                    .is_some_and(|method| method.steps.iter().any(Step::moves))
        })
    }
}

impl Def for GoalDef {
    type Id = GoalId;
    const TYPE: &'static str = "goal";
    const DOC: &'static str = "A goal: how much a character wants it, what it is about, how it is pursued, and when it is over (09 §3.2).";

    fn compile(&mut self, cx: &mut CompileCx<'_>) {
        let kind = self.about.kind();
        let scope = mind_scope(kind);
        if let Some(pick) = &mut self.about.track {
            cx.compile(&mut pick.filter, &pick_scope(kind));
            cx.compile(&mut pick.best, &pick_scope(kind));
        }
        if let Some(pick) = &mut self.about.memory {
            cx.compile(&mut pick.filter, &pick_scope(kind));
            cx.compile(&mut pick.best, &pick_scope(kind));
        }
        cx.compile(&mut self.about.place, &self_scope());
        cx.compile(&mut self.score, &scope);
        cx.compile(&mut self.done, &scope);
        cx.compile(&mut self.fail, &scope);
        for entry in &mut self.methods {
            entry.compile(cx, &scope);
        }
    }

    fn finish(&mut self, cx: &mut FinishCx<'_>) {
        if self.methods.is_empty() {
            finish_error(cx, "a goal has at least one method".to_owned());
        }
        for (at, entry) in self.methods.iter().enumerate() {
            match (&entry.method, entry.steps.is_empty()) {
                (Some(_), false) => finish_error(
                    cx,
                    format!(
                        "method {at} names a `method` and has `steps` of its own: one or the other"
                    ),
                ),
                (None, true) => finish_error(
                    cx,
                    format!("method {at} has no `steps` and names no `method`"),
                ),
                (None, false) => check_steps(&entry.steps, cx),
                (Some(_), true) => {}
            }
        }
    }

    fn validate(&self, cx: &mut ValidateCx<'_>) {
        let about = &self.about;
        let count = usize::from(about.track.is_some())
            + usize::from(about.memory.is_some())
            + usize::from(about.place.is_some());
        if count > 1 {
            cx.error("`about` is one of `track`, `memory` and `place`");
        }
        if !(self.commit.is_finite() && self.commit >= Real::ZERO) {
            cx.error("`commit` is a fraction of zero or more");
        }
        if !(self.noise.is_finite() && self.noise >= Real::ZERO) {
            cx.error("`noise` is a number of zero or more");
        }
        if self.retry_after.dur() < Dur::ZERO {
            cx.error("`retry_after` is a time of zero or more");
        }
        let kind = about.kind();
        for (at, entry) in self.methods.iter().enumerate() {
            for (index, step) in entry.steps.iter().enumerate() {
                step.validate(index, cx);
            }
            if let Some(method) = entry.method
                && let Some(def) = cx.defs().get::<MethodDef>(method.id())
                && def.about.kind() != kind
            {
                cx.error(format!(
                    "this goal's `about` is {}, and method {at}, {}, is written for {}",
                    about_name(kind),
                    cx.defs().name::<MethodDef>(method.id()).unwrap_or("?"),
                    about_name(def.about.kind())
                ));
            }
        }
    }
}

fn about_name(kind: AboutKind) -> &'static str {
    match kind {
        AboutKind::None => "nothing",
        AboutKind::Track => "a track",
        AboutKind::Memory => "a memory",
        AboutKind::Place => "a place",
    }
}

/// What a method written on its own says `$about` is: `track`,
/// `memory`, `place` or nothing.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AboutWritten {
    /// `$about` is null.
    #[default]
    None,
    /// An entity.
    Track,
    /// A memory.
    Memory,
    /// A cell.
    Place,
}

impl AboutWritten {
    /// The kind it stands for.
    pub const fn kind(self) -> AboutKind {
        match self {
            AboutWritten::None => AboutKind::None,
            AboutWritten::Track => AboutKind::Track,
            AboutWritten::Memory => AboutKind::Memory,
            AboutWritten::Place => AboutKind::Place,
        }
    }
}

/// One of a goal's ways to pursue it (09 §4): written in place, with
/// `if` and `steps`, or naming a `method` definition with `method` (and
/// an `if` of its own beside it, if wanted).
///
/// ```json
/// "methods": [ { "if": "self.weight('caution') > 0.5", "method": "keep_watch" },
///              { "steps": [ { "wait": { "time": "2 s" } } ] } ]
/// ```
#[derive(Debug, Default, Deserialize)]
pub struct MethodEntry {
    /// When it may be used. Left out, always.
    #[serde(default, rename = "if")]
    pub when: Option<Cond>,
    /// The steps, in order, for a method written in place.
    #[serde(default)]
    pub steps: Vec<Step>,
    /// A method written on its own, in place of `steps`.
    #[serde(default)]
    pub method: Option<Ref<MethodDef>>,
}

impl MethodEntry {
    fn compile(&mut self, cx: &mut CompileCx<'_>, scope: &Scope) {
        cx.compile(&mut self.when, scope);
        for step in &mut self.steps {
            step.compile(cx, scope);
        }
    }
}

/// A way to pursue a goal written on its own (09 §4), so that several
/// goals can name it: a condition and an ordered list of steps, which
/// become the character's actions one at a time. `about` says what the
/// goals that name it are about, so that `$about` has a type when it is
/// compiled; a goal about something else may not name it.
///
/// ```json
/// { "type": "method", "id": "walk_over", "about": "memory",
///   "steps": [ { "go": { "to": "$about.loc", "within": 2 } }, { "look": { "time": "2 s" } } ] }
/// ```
#[derive(Debug, Deserialize)]
pub struct MethodDef {
    /// What `$about` is in it.
    #[serde(default)]
    pub about: AboutWritten,
    /// When it may be used. Left out, always.
    #[serde(default, rename = "if")]
    pub when: Option<Cond>,
    /// The steps, in order.
    pub steps: Vec<Step>,
}

impl Def for MethodDef {
    type Id = MethodId;
    const TYPE: &'static str = "method";
    const DOC: &'static str = "A method written on its own: a condition and the steps that pursue a goal, which become the character's actions one at a time (09 §4).";

    fn finish(&mut self, cx: &mut FinishCx<'_>) {
        check_steps(&self.steps, cx);
    }

    fn compile(&mut self, cx: &mut CompileCx<'_>) {
        let scope = mind_scope(self.about.kind());
        cx.compile(&mut self.when, &scope);
        for step in &mut self.steps {
            step.compile(cx, &scope);
        }
    }

    fn validate(&self, cx: &mut ValidateCx<'_>) {
        for (at, step) in self.steps.iter().enumerate() {
            step.validate(at, cx);
        }
    }
}

/// Reports an error of a definition as a whole: it is broken.
fn finish_error(cx: &mut FinishCx<'_>, message: String) {
    cx.report(clash_data::Diagnostic::new(
        clash_data::code::INVALID_VALUE,
        message,
    ));
}

/// Reports a method with no steps, and a step that is not exactly one
/// kind.
fn check_steps(steps: &[Step], cx: &mut FinishCx<'_>) {
    if steps.is_empty() {
        finish_error(cx, "a method has at least one step".to_owned());
    }
    for (at, step) in steps.iter().enumerate() {
        let kinds = step.kinds();
        if kinds != 1 {
            finish_error(
                cx,
                format!(
                    "step {at} is exactly one of `go`, `do`, `choose`, `wait`, `look`, `say`, `set`, `settle`, `stance`, `keep_near` and `keep_from`; it has {kinds}"
                ),
            );
        }
    }
}

/// One step of a method (09 §4). Exactly one of its kinds is written;
/// `if` may be written beside it.
///
/// | Step | Does | Ends |
/// |------|------|------|
/// | `go: { to, within? }` | Starts the mind's `go_action` toward a cell, or toward where an entity is believed to be | When the character is within `within` metres of it; fails if the action ends short of it |
/// | `do: { action, with? }` | Starts an action, its parameters from expressions; missing ones take their defaults | When the action finishes; fails if it is refused or interrupted |
/// | `choose: { purpose, target? }` | Starts the first action tagged `purpose` that can start, with `target` as its parameter `target` | As `do`; fails if none can start |
/// | `wait: { time?, until? }` | Waits | After `time`, or when `until` holds at a thought |
/// | `look: { at?, time? }` | Turns to face a place or an entity's believed place, and waits | After `time` |
/// | `say: { text, volume? }` | Raises `said` | At once |
/// | `set: { var, value }` | Sets a variable of the character's own | At once |
/// | `settle: true` | Marks `$about` settled: the goal's `about` passes it over until it changes | At once |
/// | `stance: { of, toward }` | Holds someone `hostile`, `feared` or `none` | At once |
/// | `keep_near: { who, within? }` | Goes to within `within` of where `who` is believed to be, again whenever that changes | Never by itself: until the goal changes |
/// | `keep_from: { from, distance? }` | Goes to a place `distance` metres further from `from` that the character believes it can reach | When it is there |
#[derive(Debug, Default, Deserialize)]
pub struct Step {
    /// Only if this holds when the step is reached; else it is passed.
    #[serde(default, rename = "if")]
    pub when: Option<Cond>,
    /// Going somewhere.
    #[serde(default)]
    pub go: Option<GoStep>,
    /// Doing an action.
    #[serde(default, rename = "do")]
    pub act: Option<DoStep>,
    /// Doing an action chosen by its purpose.
    #[serde(default)]
    pub choose: Option<ChooseStep>,
    /// Waiting.
    #[serde(default)]
    pub wait: Option<WaitStep>,
    /// Looking at something.
    #[serde(default)]
    pub look: Option<LookStep>,
    /// Speaking.
    #[serde(default)]
    pub say: Option<SayStep>,
    /// Setting a variable of its own.
    #[serde(default)]
    pub set: Option<SetStep>,
    /// Marking `$about` settled.
    #[serde(default)]
    pub settle: Option<bool>,
    /// Holding someone hostile, feared, or neither.
    #[serde(default)]
    pub stance: Option<StanceStep>,
    /// Staying near someone.
    #[serde(default)]
    pub keep_near: Option<KeepNearStep>,
    /// Getting away from someone or something.
    #[serde(default)]
    pub keep_from: Option<KeepFromStep>,
}

/// A `go` step.
#[derive(Debug, Deserialize)]
pub struct GoStep {
    /// A cell, or an entity, which stands for where the mind believes
    /// it is.
    pub to: Expr<Any>,
    /// How close is close enough, in metres. Zero: the cell itself, or
    /// the nearest the character can stand in.
    #[serde(default)]
    pub within: Option<Expr<Number>>,
}

/// A `do` step.
#[derive(Debug, Deserialize)]
pub struct DoStep {
    /// The action.
    pub action: Ref<ActionDef>,
    /// Its parameters by name; missing ones take their defaults.
    #[serde(default, rename = "with")]
    pub params: BTreeMap<String, Expr<Any>>,
}

/// A `choose` step: the seam options (09 §6.1) stand behind.
#[derive(Debug, Deserialize)]
pub struct ChooseStep {
    /// The purpose: an action tag. Every action carrying it is a
    /// candidate, in the order the definitions were loaded.
    pub purpose: String,
    /// Given as the parameter `target` to an action that has one.
    #[serde(default)]
    pub target: Option<Expr<Any>>,
}

/// A `wait` step.
#[derive(Debug, Default, Deserialize)]
pub struct WaitStep {
    /// How long, in seconds of the character's own time.
    #[serde(default)]
    pub time: Option<Expr<Number>>,
    /// Ends the wait when it holds at a thought.
    #[serde(default)]
    pub until: Option<Cond>,
}

/// A `look` step.
#[derive(Debug, Default, Deserialize)]
pub struct LookStep {
    /// A cell, or an entity, which stands for where it is believed to be.
    #[serde(default)]
    pub at: Option<Expr<Any>>,
    /// How long to look, in seconds.
    #[serde(default)]
    pub time: Option<Expr<Number>>,
}

/// A `say` step.
#[derive(Debug, Deserialize)]
pub struct SayStep {
    /// What is said, as a text template in the mind context.
    pub text: Text,
    /// How loud, as a factor that content gives the `said` event's
    /// signal. 1 if left out.
    #[serde(default)]
    pub volume: Option<Expr<Number>>,
}

/// A `set` step.
#[derive(Debug, Deserialize)]
pub struct SetStep {
    /// The name of a variable of the character's own, as the
    /// simulation keeps it: qualified for a declared variable
    /// (`core:called_out_at`).
    pub var: String,
    /// Its new value.
    pub value: Expr<Any>,
}

/// A `stance` step.
#[derive(Debug, Deserialize)]
pub struct StanceStep {
    /// Whom.
    pub of: Expr<Any>,
    /// `hostile`, `feared` or `none`.
    pub toward: StanceWritten,
}

/// A stance as data writes it.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StanceWritten {
    /// Held hostile.
    Hostile,
    /// Feared.
    Feared,
    /// Neither.
    None,
}

/// A `keep_near` step.
#[derive(Debug, Deserialize)]
pub struct KeepNearStep {
    /// Whom, or where.
    pub who: Expr<Any>,
    /// How near, in metres.
    #[serde(default = "keep_near_default")]
    pub within: Real,
}

/// A `keep_from` step.
#[derive(Debug, Deserialize)]
pub struct KeepFromStep {
    /// Whom, or where.
    pub from: Expr<Any>,
    /// How far to go, in metres.
    #[serde(default = "keep_from_default")]
    pub distance: Real,
}

/// What kind of step one is, for events and the inspector.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum StepKind {
    /// `go`.
    Go,
    /// `do`.
    Do,
    /// `choose`.
    Choose,
    /// `wait`.
    Wait,
    /// `look`.
    Look,
    /// `say`.
    Say,
    /// `set`.
    Set,
    /// `settle`.
    Settle,
    /// `stance`.
    Stance,
    /// `keep_near`.
    KeepNear,
    /// `keep_from`.
    KeepFrom,
}

impl StepKind {
    /// The name data writes it with.
    pub const fn name(self) -> &'static str {
        match self {
            StepKind::Go => "go",
            StepKind::Do => "do",
            StepKind::Choose => "choose",
            StepKind::Wait => "wait",
            StepKind::Look => "look",
            StepKind::Say => "say",
            StepKind::Set => "set",
            StepKind::Settle => "settle",
            StepKind::Stance => "stance",
            StepKind::KeepNear => "keep_near",
            StepKind::KeepFrom => "keep_from",
        }
    }
}

impl Step {
    /// How many kinds are written: one for a good step.
    fn kinds(&self) -> usize {
        [
            self.go.is_some(),
            self.act.is_some(),
            self.choose.is_some(),
            self.wait.is_some(),
            self.look.is_some(),
            self.say.is_some(),
            self.set.is_some(),
            self.settle.is_some(),
            self.stance.is_some(),
            self.keep_near.is_some(),
            self.keep_from.is_some(),
        ]
        .into_iter()
        .filter(|written| *written)
        .count()
    }

    /// The kind written.
    pub fn kind(&self) -> StepKind {
        if self.go.is_some() {
            StepKind::Go
        } else if self.act.is_some() {
            StepKind::Do
        } else if self.choose.is_some() {
            StepKind::Choose
        } else if self.look.is_some() {
            StepKind::Look
        } else if self.say.is_some() {
            StepKind::Say
        } else if self.set.is_some() {
            StepKind::Set
        } else if self.settle.is_some() {
            StepKind::Settle
        } else if self.stance.is_some() {
            StepKind::Stance
        } else if self.keep_near.is_some() {
            StepKind::KeepNear
        } else if self.keep_from.is_some() {
            StepKind::KeepFrom
        } else {
            StepKind::Wait
        }
    }

    /// Whether it moves the character by the mind's `go_action`.
    fn moves(&self) -> bool {
        self.go.is_some() || self.keep_near.is_some() || self.keep_from.is_some()
    }

    fn compile(&mut self, cx: &mut CompileCx<'_>, scope: &Scope) {
        cx.compile(&mut self.when, scope);
        if let Some(go) = &mut self.go {
            cx.compile(&mut go.to, scope);
            cx.compile(&mut go.within, scope);
        }
        if let Some(act) = &mut self.act {
            cx.compile(&mut act.params, scope);
        }
        if let Some(choose) = &mut self.choose {
            cx.compile(&mut choose.target, scope);
        }
        if let Some(wait) = &mut self.wait {
            cx.compile(&mut wait.time, scope);
            cx.compile(&mut wait.until, scope);
        }
        if let Some(look) = &mut self.look {
            cx.compile(&mut look.at, scope);
            cx.compile(&mut look.time, scope);
        }
        if let Some(say) = &mut self.say {
            cx.compile(&mut say.text, scope);
            cx.compile(&mut say.volume, scope);
        }
        if let Some(set) = &mut self.set {
            cx.compile(&mut set.value, scope);
        }
        if let Some(stance) = &mut self.stance {
            cx.compile(&mut stance.of, scope);
        }
        if let Some(keep) = &mut self.keep_near {
            cx.compile(&mut keep.who, scope);
        }
        if let Some(keep) = &mut self.keep_from {
            cx.compile(&mut keep.from, scope);
        }
    }

    fn validate(&self, at: usize, cx: &mut ValidateCx<'_>) {
        if let Some(act) = &self.act
            && let Some(def) = cx.defs().get::<ActionDef>(act.action.id())
        {
            for name in act.params.keys() {
                let known = def.param(name).is_some()
                    || def.roles().iter().any(|role| role.name.value == *name);
                if !known {
                    cx.error(format!(
                        "step {at}: the action {} has no parameter `{name}`",
                        cx.defs().name::<ActionDef>(act.action.id()).unwrap_or("?")
                    ));
                }
            }
        }
        if let Some(keep) = &self.keep_near
            && !(keep.within.is_finite() && keep.within >= Real::ZERO)
        {
            cx.error(format!("step {at}: `within` is a distance of zero or more"));
        }
        if let Some(keep) = &self.keep_from
            && !(keep.distance.is_finite() && keep.distance > Real::ZERO)
        {
            cx.error(format!("step {at}: `distance` is a distance above zero"));
        }
        if let Some(choose) = &self.choose
            && choose.purpose.is_empty()
        {
            cx.error(format!("step {at}: `purpose` names an action tag"));
        }
        if let Some(wait) = &self.wait
            && wait.time.is_none()
            && wait.until.is_none()
        {
            cx.error(format!(
                "step {at}: a `wait` has a `time`, an `until` or both"
            ));
        }
    }
}
