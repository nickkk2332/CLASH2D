//! The definition types of the action system: `action`, `channel` and
//! `window_kind` (03 §6.1 to §6.4, §7.1, §8).
//!
//! Each is read by the mod loader into the struct here, resolved and
//! compiled in place, and then never changes. What the loader hands the
//! runtime is the same struct: names have become indices, channels a
//! bitset, and every expression is compiled against one fixed layout of
//! slots. See the [module above](crate::action) for how it is walked.

mod channel;
mod param;
mod phase;
mod window;

use std::collections::BTreeMap;
use std::fmt;

use clash_core::Real;
use clash_data::diag::{closest, did_you_mean};
use clash_data::expr::Number;
use clash_data::{
    CompileCx, Cond, Def, Diagnostic, Expr, FinishCx, Located, Ref, Scope, Span, TagSet, Text,
    ValidateCx, code, is_id_part,
};
use serde::Deserialize;
use serde::de::{Deserializer, MapAccess, Visitor};

pub use crate::rule::Effects;
pub use channel::{ChannelDef, ChannelNeed, ChannelUse, Slots};
pub use param::{ListBounds, Local, LocalSource, Param, Var, WINDOW_LOCAL};
pub use phase::{
    CancelRule, HOLD, Next, Phase, PhaseRef, PhaseTime, PhaseWindow, Tick, Transition,
};
pub use window::{CANDIDATE_SUBJECT, SOURCE_SUBJECT, WINDOW_SUBJECT, WindowKindDef};

use crate::action::ids::{ActionDefId, ChannelSet, PhaseIndex, WindowKindId};
use param::{ENTITY_KIND, LOCAL_NAME_RULE, Params, Vars, is_local_name, with_locals};

/// The name the actor is bound under in every expression of an action.
/// It is the only bound subject, so its slot is 0.
pub const ACTOR_SUBJECT: &str = "self";

/// The fewest roles a joint action has (03 §8): with one participant it
/// is an ordinary action.
pub const MIN_ROLES: usize = 2;

/// One role of a joint action (03 §8).
///
/// ```json
/// "roles": {
///   "attacker": { "uses": { "arm": "all" } },
///   "victim":   { "uses": { "arm": "all", "legs": "all" }, "forced": true }
/// }
/// ```
#[derive(Debug)]
pub struct Role {
    /// The name. The participant is bound under it as `$name` in every
    /// expression and effect of the action.
    pub name: Located<String>,
    /// The channel slots claimed on this role's participant.
    pub uses: ChannelUse,
    /// Whether the participant is entered without consenting. A role
    /// that is not forced is offered and must be accepted. The first
    /// role is whoever starts the action and is never forced.
    pub forced: bool,
    forced_at: Option<Span>,
    slot: u16,
}

impl Role {
    /// The slot of the participant among the action's locals.
    pub fn slot(&self) -> u16 {
        self.slot
    }
}

/// A role as written.
#[derive(Deserialize)]
struct RoleObject {
    #[serde(default)]
    uses: ChannelUse,
    #[serde(default)]
    forced: Option<Located<bool>>,
}

/// The roles of a joint action, in the order written: the first is
/// whoever starts it (04 §5).
#[derive(Debug, Default)]
struct Roles(Vec<Role>);

impl<'de> Deserialize<'de> for Roles {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Roles, D::Error> {
        struct RolesVisitor;

        impl<'de> Visitor<'de> for RolesVisitor {
            type Value = Roles;

            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str("an object of roles, each a name and what it uses")
            }

            fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Roles, A::Error> {
                let mut roles = Vec::new();
                while let Some(name) = map.next_key::<Located<String>>()? {
                    let object = map.next_value::<RoleObject>()?;
                    roles.push(Role {
                        name,
                        uses: object.uses,
                        forced: object.forced.as_ref().is_some_and(|forced| forced.value),
                        forced_at: object.forced.map(|forced| forced.span),
                        slot: 0,
                    });
                }
                Ok(Roles(roles))
            }
        }

        deserializer.deserialize_map(RolesVisitor)
    }
}

fn yes() -> bool {
    true
}

fn unit_tempo() -> Expr<Number> {
    Expr::literal(1)
}

/// An action (03 §6): a small state machine of phases that a character
/// runs, occupying channels while it does.
///
/// ```json
/// {
///   "type": "action",
///   "id": "heavy_punch",
///   "name": "Heavy punch",
///   "uses": { "arm": 1 },
///   "params": {
///     "target": { "type": "entity" },
///     "part":   { "type": "part", "default": "auto_part($target)" }
///   },
///   "requires": "dist(self, $target) <= 1.5",
///   "tempo": "self.attr.quickness",
///   "phases": [
///     { "id": "windup", "time": "300 ms", "cancel": true,
///       "window": { "kind": "strike", "for": ["$target"], "closes": "impact" } },
///     { "id": "impact", "time": 0,
///       "do": [ { "strike": { "target": "$target", "part": "$part" } } ] },
///     { "id": "recovery", "time": "400 ms" }
///   ]
/// }
/// ```
///
/// # Scopes
///
/// Every expression of an action binds one subject, `self`, the actor
/// (slot 0), and the action's [locals](ActionDef::locals) in slot order.
/// What differs between fields is what they may read:
///
/// | Field | Reads | Pure |
/// |-------|-------|------|
/// | `requires`, `fail_text`, a parameter's `check`, `name` | Belief: the mind context of the actor | Yes |
/// | A parameter's `default` | Belief | No |
/// | `requires_at_start`, `tempo`, a phase's `time`, `max`, `until`, its tick's `every`, the conditions of `next`, a window's `for` | Truth | No |
/// | A variable's `default` | Nothing: a constant | Yes |
///
/// A parameter's `default` sees the locals before the parameter, and its
/// `check` those and the parameter itself; `name` sees no locals. Effect
/// lists ([`Effects`]) read truth and may draw at random; they are
/// compiled in [`effect_scope`](ActionDef::effect_scope), and whatever
/// they define with `let` takes the slots after the action's own
/// locals.
#[derive(Debug, Deserialize)]
pub struct ActionDef {
    /// What the action is called, shown to players. A text template in
    /// the mind context of the actor, with no locals.
    #[serde(default)]
    pub name: Option<Text>,
    #[serde(default)]
    uses: ChannelUse,
    #[serde(default)]
    params: Params,
    #[serde(default)]
    vars: Vars,
    /// The condition that must hold to start. Compiled in the mind
    /// context of the actor, and pure, so that it can be previewed; it is
    /// tested against truth only when the action is started, and a
    /// failure then is a refusal ([`reason::REQUIRES`](super::reason::REQUIRES)).
    #[serde(default)]
    pub requires: Cond,
    /// The sentence shown when `requires` fails. A text template in the
    /// same scope as `requires`.
    #[serde(default)]
    pub fail_text: Option<Text>,
    /// A condition tested against truth only when the action begins, and
    /// never shown beforehand. If it fails the action is interrupted at
    /// once ([`reason::START_CONDITION`](super::reason::START_CONDITION)).
    #[serde(default)]
    pub requires_at_start: Cond,
    /// What phase durations are divided by: evaluated against truth at
    /// the start of each phase that is not `fixed`. Left out, it is 1.
    #[serde(default = "unit_tempo")]
    pub tempo: Expr<Number>,
    /// Whether finishing this action can prompt a decision.
    #[serde(default = "yes")]
    pub foreground: bool,
    /// Whether the actor must be standing on something to start.
    #[serde(default = "yes")]
    pub footing: bool,
    #[serde(default)]
    reaction_to: Vec<Located<Ref<WindowKindDef>>>,
    /// Whether, as a reaction, the action may answer a window aimed at
    /// someone else.
    #[serde(default)]
    pub on_behalf: bool,
    #[serde(default)]
    ready: Option<PhaseRef>,
    phases: Vec<Phase>,
    /// Effects run when the action starts, before its first phase.
    #[serde(default)]
    pub on_start: Effects,
    /// Effects run when the last phase has ended.
    #[serde(default)]
    pub on_finish: Effects,
    /// Effects run when the actor abandons the action.
    #[serde(default)]
    pub on_cancel: Effects,
    /// Effects run when the action is ended against the actor's will.
    #[serde(default)]
    pub on_interrupt: Effects,
    /// Effects run when the action ends because its actor left the
    /// simulation (03 §9).
    #[serde(default)]
    pub on_abandon: Effects,
    #[serde(default)]
    roles: Option<Located<Roles>>,
    /// Free-form tags, queryable by rules and named by a phase's
    /// `cancel`.
    #[serde(default)]
    pub tags: TagSet,
    /// Whether the neighbourhood's detail may be lowered while every
    /// directly controlled entity is in such an action (03 §11).
    #[serde(default)]
    pub low_attention: bool,
    #[serde(skip)]
    locals: Vec<Local>,
    #[serde(skip)]
    reacts: Vec<WindowKindId>,
    /// What the action strikes with (07 §3.1): its attacks, which the
    /// `strike` effect of one of its lists resolves. An action with
    /// attacks is given the locals `strike` leaves (`$landed`,
    /// `$absorbed`, `$passed`) as variables, unless it declares them.
    #[serde(default)]
    pub attacks: crate::combat::Attacks,
    /// How the action defends (07 §5.1): in force during one of its
    /// phases, avoiding a strike or putting something in its way.
    #[serde(default)]
    pub defence: Option<crate::combat::DefenceDef>,
}

impl ActionDef {
    /// The channel slots the actor needs for as long as the action runs.
    /// Empty for a joint action, whose [roles](ActionDef::roles) say what
    /// each participant needs.
    pub fn uses(&self) -> &ChannelUse {
        &self.uses
    }

    /// The roles of a joint action, in the order written: the first is
    /// whoever starts it. Empty for an action with one participant.
    pub fn roles(&self) -> &[Role] {
        self.roles.as_ref().map_or(&[], |roles| &roles.value.0)
    }

    /// Whether the action has several participants (03 §8).
    pub fn is_joint(&self) -> bool {
        !self.roles().is_empty()
    }

    /// Every channel the action uses on anyone taking part.
    pub fn channels(&self) -> ChannelSet {
        self.roles()
            .iter()
            .fold(self.uses.set(), |set, role| set | role.uses.set())
    }

    /// The parameters, in the order written, which is the order their
    /// defaults are worked out in.
    pub fn params(&self) -> &[Param] {
        &self.params.0
    }

    /// The parameter with this name.
    pub fn param(&self, name: &str) -> Option<&Param> {
        self.params().iter().find(|param| param.name.value == name)
    }

    /// The action's own variables, in the order written.
    pub fn vars(&self) -> &[Var] {
        &self.vars.0
    }

    /// The locals, in slot order: the window of a reaction, then the
    /// roles, the parameters and the variables. A running action keeps
    /// one value for each, in this order.
    pub fn locals(&self) -> &[Local] {
        &self.locals
    }

    /// The slot of the local with this name.
    pub fn local_slot(&self, name: &str) -> Option<u16> {
        self.locals
            .iter()
            .position(|local| local.name == name)
            .and_then(|slot| u16::try_from(slot).ok())
    }

    /// The phases, in the order written. There is always at least one,
    /// and the action starts in the first.
    pub fn phases(&self) -> &[Phase] {
        &self.phases
    }

    /// The phase at a position.
    pub fn phase(&self, index: PhaseIndex) -> Option<&Phase> {
        self.phases.get(index.index())
    }

    /// The position of the phase with this id. For tools and effects
    /// that name a phase; the runtime walks by index.
    pub fn phase_index(&self, id: &str) -> Option<PhaseIndex> {
        self.phases
            .iter()
            .position(|phase| phase.name() == id)
            .and_then(PhaseIndex::from_index)
    }

    /// For a reaction: the phase from which it counts as ready
    /// (03 §7.3).
    pub fn ready(&self) -> Option<PhaseIndex> {
        self.ready.as_ref().map(PhaseRef::index)
    }

    /// The kinds of window the action may answer, in order of id.
    pub fn reaction_to(&self) -> &[WindowKindId] {
        &self.reacts
    }

    /// Whether the action may answer a window of this kind.
    pub fn reacts_to(&self, kind: WindowKindId) -> bool {
        self.reacts.binary_search(&kind).is_ok()
    }

    /// The scope of the action's expressions that read truth: `self`,
    /// the actor, and the locals in slot order.
    pub fn scope(&self) -> Scope {
        with_locals(Scope::new().own(ACTOR_SUBJECT, ENTITY_KIND), &self.locals)
    }

    /// The same scope as a mind context of the actor: what `requires`,
    /// defaults and `check` are compiled in (05 §7.3).
    pub fn mind_scope(&self) -> Scope {
        with_locals(Scope::new().mind(ACTOR_SUBJECT, ENTITY_KIND), &self.locals)
    }

    /// The scope the action's effect lists are compiled in: they read
    /// truth and may draw at random.
    pub fn effect_scope(&self) -> Scope {
        self.scope()
    }

    /// Every effect list of the action, in the order written: its own
    /// five, then for each phase `do`, `on_exit` and the tick's, then
    /// for each attack `on_land`, `on_miss` and `on_stopped`.
    pub fn effect_lists(&self) -> impl Iterator<Item = &Effects> {
        let own = [
            &self.on_start,
            &self.on_finish,
            &self.on_cancel,
            &self.on_interrupt,
            &self.on_abandon,
        ];
        let phases = self.phases.iter().flat_map(|phase| {
            [Some(&phase.on_enter), Some(&phase.on_exit)]
                .into_iter()
                .chain([phase.tick.as_ref().map(|tick| &tick.effects)])
                .flatten()
        });
        own.into_iter()
            .chain(phases)
            .chain(self.attacks.effect_lists())
    }

    /// The scope an attack's expressions are compiled in (07 §3.1): the
    /// action's, reading truth, with the subjects `weapon` (the item
    /// that strikes), `limb` (the part that strikes) and `target` (the
    /// one struck) after `self`. Each is null where there is none.
    pub fn attack_scope(&self) -> Scope {
        let subjects = Scope::new()
            .own(ACTOR_SUBJECT, ENTITY_KIND)
            .subject(crate::combat::WEAPON_SUBJECT, ENTITY_KIND)
            .subject(crate::combat::LIMB_SUBJECT, "part")
            .subject(crate::combat::TARGET_SUBJECT, ENTITY_KIND);
        with_locals(subjects, &self.locals)
    }

    /// The scope an attack's effect lists are compiled in: the action's
    /// effect scope with the subject `target` after `self`.
    pub fn attack_effect_scope(&self) -> Scope {
        let subjects = Scope::new()
            .own(ACTOR_SUBJECT, ENTITY_KIND)
            .subject(crate::combat::TARGET_SUBJECT, ENTITY_KIND);
        with_locals(subjects, &self.locals)
    }

    /// Gives an action with attacks the locals the `strike` effect
    /// leaves (07 §3.4), as variables, where it does not declare them.
    fn imply_strike_results(&mut self) {
        let Some(first) = self.attacks.iter().next() else {
            return;
        };
        let at = first.id.span;
        for (name, written) in crate::combat::RESULT_LOCALS {
            let declared = self.roles().iter().any(|role| role.name.value == name)
                || self.params().iter().any(|param| param.name.value == name)
                || self.vars().iter().any(|var| var.name.value == name);
            if declared {
                continue;
            }
            let start = if written == "bool" {
                Expr::literal(false)
            } else {
                Expr::literal(Real::ZERO)
            };
            self.vars.0.push(Var::implied(name, written, start, at));
        }
    }

    /// An action that does nothing: one phase of no length. What a
    /// lenient load puts in place of a broken action.
    fn inert() -> ActionDef {
        let nowhere = Span::file_start(clash_data::FileId(0));
        ActionDef {
            name: None,
            uses: ChannelUse::default(),
            params: Params::default(),
            vars: Vars::default(),
            requires: Cond::constant(false),
            fail_text: None,
            requires_at_start: Cond::default(),
            tempo: unit_tempo(),
            foreground: true,
            footing: true,
            reaction_to: Vec::new(),
            on_behalf: false,
            ready: None,
            phases: vec![Phase::inert(nowhere)],
            on_start: Effects::empty(),
            on_finish: Effects::empty(),
            on_cancel: Effects::empty(),
            on_interrupt: Effects::empty(),
            on_abandon: Effects::empty(),
            roles: None,
            tags: TagSet::new(),
            low_attention: false,
            locals: Vec::new(),
            reacts: Vec::new(),
            attacks: crate::combat::Attacks::default(),
            defence: None,
        }
    }

    /// The scope a defence's expressions are compiled in (07 §5.1): the
    /// action's, reading truth, with the subjects `attacker` (who
    /// strikes) and `target` (the one struck) after `self`, the
    /// defender.
    pub fn defence_scope(&self) -> Scope {
        let subjects = Scope::new()
            .own(ACTOR_SUBJECT, ENTITY_KIND)
            .subject(crate::combat::defence::ATTACKER_SUBJECT, ENTITY_KIND)
            .subject(crate::combat::TARGET_SUBJECT, ENTITY_KIND);
        with_locals(subjects, &self.locals)
    }
}

/// Reports an error of the action's own at a place in the file, found
/// before expressions are compiled. The definition is broken.
fn fail(cx: &mut FinishCx<'_>, span: Span, path: &str, message: impl Into<String>) {
    let diagnostic = Diagnostic::new(code::VALIDATION, message)
        .at(cx.sources(), span)
        .with_path(path);
    cx.report(diagnostic);
}

/// Reports something allowed and probably not meant.
fn warn(cx: &mut FinishCx<'_>, span: Option<Span>, path: &str, message: impl Into<String>) {
    let mut diagnostic = Diagnostic::new(code::VALIDATION_WARNING, message).with_path(path);
    if let Some(span) = span {
        diagnostic = diagnostic.at(cx.sources(), span);
    }
    cx.report(diagnostic);
}

/// The phases that can be entered from any of `starts`, by position.
fn reached(phases: &[Phase], starts: impl IntoIterator<Item = Next>) -> Vec<bool> {
    let position = |next: Next| match next {
        Next::Phase(index) => Some(index.index()),
        Next::Finish => None,
    };
    let mut seen = vec![false; phases.len()];
    let mut queue: Vec<usize> = starts.into_iter().filter_map(position).collect();
    while let Some(at) = queue.pop() {
        let (Some(phase), Some(mark)) = (phases.get(at), seen.get_mut(at)) else {
            continue;
        };
        if *mark {
            continue;
        }
        *mark = true;
        queue.extend(phase.exits().filter_map(position));
    }
    seen
}

impl ActionDef {
    /// Checks the names an action's expressions write as `$name`: each
    /// well formed, and no two alike.
    fn finish_names(&self, cx: &mut FinishCx<'_>) {
        let mut known: BTreeMap<&str, &str> = BTreeMap::new();
        if !self.reaction_to.is_empty() {
            known.insert(WINDOW_LOCAL, "the window a reaction answers");
        }
        let roles = self
            .roles()
            .iter()
            .map(|role| (&role.name, "roles", "a role"));
        let params = self
            .params()
            .iter()
            .map(|param| (&param.name, "params", "a parameter"));
        let vars = self
            .vars()
            .iter()
            .map(|var| (&var.name, "vars", "a variable"));
        for (name, group, what) in roles.chain(params).chain(vars) {
            let path = format!("{group}.{}", name.value);
            if !is_local_name(&name.value) {
                fail(
                    cx,
                    name.span,
                    &path,
                    format!(
                        "\"{}\" cannot be written `${}`: {LOCAL_NAME_RULE}",
                        name.value, name.value
                    ),
                );
                continue;
            }
            if let Some(taken) = known.get(name.value.as_str()) {
                fail(
                    cx,
                    name.span,
                    &path,
                    format!(
                        "`${}` already names {taken} in this action; {what} needs a name of its own",
                        name.value
                    ),
                );
                continue;
            }
            known.insert(name.value.as_str(), what);
        }
    }

    /// Checks the roles of a joint action (03 §8).
    fn finish_roles(&mut self, cx: &mut FinishCx<'_>) {
        let Some(roles) = &mut self.roles else {
            return;
        };
        if roles.value.0.len() < MIN_ROLES {
            fail(
                cx,
                roles.span,
                "roles",
                format!(
                    "a joint action has at least {MIN_ROLES} roles; an action with one participant says what it `uses` and has no `roles`"
                ),
            );
        }
        if let Some(need) = self.uses.needs().first() {
            fail(
                cx,
                need.span,
                "uses",
                "a joint action says what each role uses; move this into the role of whoever starts the action, which is the first",
            );
        }
        for (position, role) in roles.value.0.iter_mut().enumerate() {
            let path = format!("roles.{}", role.name.value);
            role.uses.finish(&format!("{path}.uses"), cx);
            if position == 0
                && role.forced
                && let Some(span) = role.forced_at
            {
                fail(
                    cx,
                    span,
                    &format!("{path}.forced"),
                    "the first role is whoever starts the action, who cannot be forced into it",
                );
            }
        }
    }

    /// Resolves every phase named in the action to its position, and
    /// checks what a phase may and may not carry. `false` if a name
    /// could not be resolved.
    fn finish_phases(&mut self, cx: &mut FinishCx<'_>) -> bool {
        let mut sound = true;
        if self.phases.is_empty() {
            let diagnostic = Diagnostic::new(
                code::VALIDATION,
                "an action has at least one phase; one of no length is `{ \"id\": \"act\", \"time\": 0 }`",
            )
            .with_path("phases");
            cx.report(diagnostic);
            return false;
        }

        // Ids: well formed, no two alike, each with a position.
        let mut positions: BTreeMap<String, (PhaseIndex, Span)> = BTreeMap::new();
        for (position, phase) in self.phases.iter().enumerate() {
            let path = format!("phases.{}.id", phase.name());
            let Some(index) = PhaseIndex::from_index(position) else {
                fail(
                    cx,
                    phase.id.span,
                    &path,
                    "this action has more phases than can be counted",
                );
                return false;
            };
            if !is_id_part(phase.name()) {
                fail(
                    cx,
                    phase.id.span,
                    &path,
                    format!(
                        "\"{}\" is not a phase id: an id starts with a lowercase letter and holds lowercase letters, digits and `_`",
                        phase.name()
                    ),
                );
                sound = false;
            }
            if let Some((_, first)) = positions.get(phase.name()) {
                let diagnostic = Diagnostic::new(
                    code::VALIDATION,
                    format!("this action has two phases called `{}`", phase.name()),
                )
                .at(cx.sources(), phase.id.span)
                .with_path(path)
                .note_at(cx.sources(), *first, "the first is here");
                cx.report(diagnostic);
                sound = false;
                continue;
            }
            positions.insert(phase.name().to_owned(), (index, phase.id.span));
        }
        let known: Vec<&str> = positions.keys().map(String::as_str).collect();
        let resolve = |cx: &mut FinishCx<'_>, named: &mut PhaseRef, path: &str| match positions
            .get(named.name())
        {
            Some((index, _)) => {
                named.resolve(*index);
                true
            }
            None => {
                let diagnostic = Diagnostic::new(
                    code::VALIDATION,
                    format!("this action has no phase `{}`", named.name()),
                )
                .at(cx.sources(), named.span())
                .with_path(path)
                .suggest_opt(did_you_mean(closest(named.name(), known.iter().copied())));
                cx.report(diagnostic);
                false
            }
        };

        let count = self.phases.len();
        for (position, phase) in self.phases.iter_mut().enumerate() {
            let base = format!("phases.{}", phase.name());
            let following = PhaseIndex::from_index(position + 1).filter(|_| position + 1 < count);
            phase.set_after(following.map_or(Next::Finish, Next::Phase));

            for (at, entry) in phase.next.iter_mut().enumerate() {
                let path = format!("{base}.next[{at}]");
                let finishes = entry.finish.as_ref().is_some_and(|finish| finish.value);
                match (&mut entry.goto, finishes) {
                    (Some(goto), true) => {
                        fail(
                            cx,
                            goto.span(),
                            &format!("{path}.goto"),
                            "this entry both goes to a phase and finishes the action; it does one",
                        );
                        sound = false;
                    }
                    (Some(goto), false) => {
                        if resolve(cx, goto, &format!("{path}.goto")) {
                            let to = Next::Phase(goto.index());
                            entry.resolve(to);
                        } else {
                            sound = false;
                        }
                    }
                    (None, true) => entry.resolve(Next::Finish),
                    (None, false) => {
                        let span = entry
                            .finish
                            .as_ref()
                            .map(|finish| finish.span)
                            .or_else(|| entry.when.as_ref().and_then(Cond::span))
                            .unwrap_or(phase.id.span);
                        fail(
                            cx,
                            span,
                            &path,
                            "an entry of `next` says where to go: `\"goto\"` with a phase id, or `\"finish\": true`",
                        );
                        sound = false;
                    }
                }
            }

            if let Some(window) = &mut phase.window {
                let path = format!("{base}.window.closes");
                if !resolve(cx, &mut window.closes, &path) {
                    sound = false;
                } else if window.closes.index().index() == position {
                    fail(
                        cx,
                        window.closes.span(),
                        &path,
                        "a window closes when a later phase starts, and this is the phase that opens it",
                    );
                    sound = false;
                }
            }

            if !phase.time.is_hold() {
                if let Some(max) = &phase.max {
                    fail(
                        cx,
                        max.span().unwrap_or(phase.id.span),
                        &format!("{base}.max"),
                        "`max` is the longest a held phase lasts, and this phase has a `time` of its own; write `\"time\": \"hold\"` to hold it",
                    );
                }
                if let Some(until) = &phase.until {
                    fail(
                        cx,
                        until.span().unwrap_or(phase.id.span),
                        &format!("{base}.until"),
                        "`until` ends a held phase, and this phase has a `time` of its own; write `\"time\": \"hold\"` to hold it",
                    );
                }
            } else if let (Some(until), None) = (&phase.until, &phase.tick) {
                fail(
                    cx,
                    until.span().unwrap_or(phase.id.span),
                    &format!("{base}.until"),
                    "`until` is tested at each tick, and this phase has no `tick`; add one, such as `\"tick\": { \"every\": \"250 ms\" }`",
                );
            }
        }

        if let Some(ready) = &mut self.ready
            && !resolve(cx, ready, "ready")
        {
            sound = false;
        }
        if let Some(defence) = &mut self.defence
            && !resolve(cx, &mut defence.during, "defence.during")
        {
            sound = false;
        }
        sound
    }

    /// Checks that every phase can be entered and every window can
    /// close. Called once the phases' names are resolved.
    fn finish_paths(&self, cx: &mut FinishCx<'_>) {
        let from_start = reached(&self.phases, PhaseIndex::from_index(0).map(Next::Phase));
        for (phase, entered) in self.phases.iter().zip(&from_start) {
            let base = format!("phases.{}", phase.name());
            if !entered {
                fail(
                    cx,
                    phase.id.span,
                    &base,
                    format!(
                        "the phase `{}` is never entered: no phase leads to it",
                        phase.name()
                    ),
                );
            }
            // Nothing after an entry that always holds is ever tried.
            if let Some(last) = phase.next.iter().position(Transition::is_unconditional)
                && let Some(dead) = phase.next.get(last + 1)
            {
                let span = dead
                    .goto
                    .as_ref()
                    .map(PhaseRef::span)
                    .or_else(|| dead.finish.as_ref().map(|finish| finish.span));
                warn(
                    cx,
                    span,
                    &format!("{base}.next[{}]", last + 1),
                    "this entry is never tried: the one before it always holds",
                );
            }
            if let Some(window) = &phase.window {
                let later = reached(&self.phases, phase.exits());
                if later.get(window.closes.index().index()) != Some(&true) {
                    fail(
                        cx,
                        window.closes.span(),
                        &format!("{base}.window.closes"),
                        format!(
                            "this window can never close: no path leads from `{}` to `{}`",
                            phase.name(),
                            window.closes.name()
                        ),
                    );
                }
            }
        }
    }

    /// Works out which windows the action answers.
    fn finish_reactions(&mut self, cx: &mut FinishCx<'_>) {
        let mut reacts: Vec<WindowKindId> = Vec::with_capacity(self.reaction_to.len());
        let mut repeats = Vec::new();
        for (at, kind) in self.reaction_to.iter().enumerate() {
            if reacts.contains(&kind.value.id()) {
                repeats.push((at, kind.span));
            } else {
                reacts.push(kind.value.id());
            }
        }
        reacts.sort_unstable();
        self.reacts = reacts;
        for (at, span) in repeats {
            warn(
                cx,
                Some(span),
                &format!("reaction_to[{at}]"),
                "this window kind is listed twice",
            );
        }
        if self.reaction_to.is_empty() {
            if let Some(ready) = &self.ready {
                warn(
                    cx,
                    Some(ready.span()),
                    "ready",
                    "`ready` says when a reaction counts as ready, and this action answers no window: it has no `reaction_to`",
                );
            }
            if self.on_behalf {
                warn(
                    cx,
                    None,
                    "on_behalf",
                    "`on_behalf` lets a reaction answer for someone else, and this action answers no window: it has no `reaction_to`",
                );
            }
        }
    }

    /// Lays out the locals and works out their types. `None`, with the
    /// reasons reported, if a declaration is wrong.
    fn compile_locals(&mut self, cx: &mut CompileCx<'_>) -> Option<Vec<Local>> {
        let entity = cx.parse_type(ENTITY_KIND, None, "")?;
        let mut locals = Vec::new();
        let mut sound = true;
        let next_slot = |locals: &[Local]| u16::try_from(locals.len()).unwrap_or(u16::MAX);
        if !self.reaction_to.is_empty() {
            // Null when the action was not started as a reaction.
            match cx.parse_type(&format!("{WINDOW_LOCAL}?"), None, "reaction_to") {
                Some(ty) => locals.push(Local {
                    name: WINDOW_LOCAL.to_owned(),
                    ty,
                    source: LocalSource::Window,
                }),
                None => sound = false,
            }
        }
        if let Some(roles) = &mut self.roles {
            for (position, role) in roles.value.0.iter_mut().enumerate() {
                role.slot = next_slot(&locals);
                locals.push(Local {
                    name: role.name.value.clone(),
                    ty: entity.clone(),
                    source: LocalSource::Role(u16::try_from(position).unwrap_or(u16::MAX)),
                });
            }
        }
        for (position, param) in self.params.0.iter_mut().enumerate() {
            sound &= param.resolve(next_slot(&locals), cx);
            locals.push(Local {
                name: param.name.value.clone(),
                ty: param.ty().clone(),
                source: LocalSource::Param(u16::try_from(position).unwrap_or(u16::MAX)),
            });
        }
        for (position, var) in self.vars.0.iter_mut().enumerate() {
            sound &= var.resolve(next_slot(&locals), cx);
            locals.push(Local {
                name: var.name.value.clone(),
                ty: var.ty().clone(),
                source: LocalSource::Var(u16::try_from(position).unwrap_or(u16::MAX)),
            });
        }
        if locals.len() > usize::from(u16::MAX) {
            cx.report(Diagnostic::new(
                code::VALIDATION,
                "this action has more roles, parameters and variables than can be counted",
            ));
            sound = false;
        }
        sound.then_some(locals)
    }
}

impl Def for ActionDef {
    type Id = ActionDefId;
    const TYPE: &'static str = "action";
    const DOC: &'static str = "Something a character does over time: phases with durations, the channels it occupies and the windows it opens (03 §6).";
    const DEFAULTS: &'static str = r#"{ "tempo": 1 }"#;

    fn finish(&mut self, cx: &mut FinishCx<'_>) {
        self.uses.finish("uses", cx);
        self.attacks.finish(cx);
        self.imply_strike_results();
        self.finish_roles(cx);
        self.finish_names(cx);
        self.finish_reactions(cx);
        if self.finish_phases(cx) {
            self.finish_paths(cx);
        }
    }

    fn compile(&mut self, cx: &mut CompileCx<'_>) {
        let Some(locals) = self.compile_locals(cx) else {
            return;
        };
        self.locals = locals;
        let truth = self.scope();
        let mind = self.mind_scope();
        let previewed = mind.clone().pure();

        // A default sees what was settled before it; a check sees its
        // own parameter too.
        let locals = &self.locals;
        for param in &mut self.params.0 {
            let slot = usize::from(param.slot());
            let before = locals.get(..slot).unwrap_or(locals);
            let with_it = locals.get(..=slot).unwrap_or(locals);
            let actor = Scope::new().mind(ACTOR_SUBJECT, ENTITY_KIND);
            let wanted = param.ty().clone();
            if let Some(default) = &mut param.default {
                let scope = with_locals(actor.clone(), before).expecting(wanted);
                cx.compile(default, &scope);
            }
            cx.compile(&mut param.check, &with_locals(actor, with_it).pure());
        }

        let actor = Scope::new().mind(ACTOR_SUBJECT, ENTITY_KIND).pure();
        cx.compile(&mut self.name, &actor);
        cx.compile(&mut self.requires, &previewed);
        cx.compile(&mut self.fail_text, &previewed);
        cx.compile(&mut self.requires_at_start, &truth);
        cx.compile(&mut self.tempo, &truth);
        let aimed = truth.clone().expecting(format!("{ENTITY_KIND}?"));
        for phase in &mut self.phases {
            if let PhaseTime::For(duration) = &mut phase.time {
                cx.compile(duration, &truth);
            }
            cx.compile(&mut phase.max, &truth);
            cx.compile(&mut phase.until, &truth);
            if let Some(tick) = &mut phase.tick {
                cx.compile(&mut tick.every, &truth);
            }
            for entry in &mut phase.next {
                cx.compile(&mut entry.when, &truth);
            }
            if let Some(window) = &mut phase.window {
                cx.compile(&mut window.aimed_at, &aimed);
            }
        }

        // The attacks: truth, with the action's locals and what strikes.
        let (attack_exprs, attack_effects) = (self.attack_scope(), self.attack_effect_scope());
        self.attacks.compile(cx, &attack_exprs, &attack_effects);
        // The defence: truth, with the action's locals and who strikes.
        let defence_exprs = self.defence_scope();
        if let Some(defence) = &mut self.defence {
            defence.compile(cx, &defence_exprs, &truth);
        }

        // The effect lists: truth, with the action's locals.
        let effects = self.effect_scope();
        cx.compile(&mut self.on_start, &effects);
        cx.compile(&mut self.on_finish, &effects);
        cx.compile(&mut self.on_cancel, &effects);
        cx.compile(&mut self.on_interrupt, &effects);
        cx.compile(&mut self.on_abandon, &effects);
        for phase in &mut self.phases {
            cx.compile(&mut phase.on_enter, &effects);
            cx.compile(&mut phase.on_exit, &effects);
            if let Some(tick) = &mut phase.tick {
                cx.compile(&mut tick.effects, &effects);
            }
        }
    }

    fn validate(&self, cx: &mut ValidateCx<'_>) {
        for effects in self.effect_lists() {
            effects.validate(cx);
        }
        self.attacks.validate(cx);
        if let Some(defence) = &self.defence {
            defence.validate(cx);
        }
        crate::combat::check_strikes(self, cx);
        let constant = |expr: &Expr<Number>| expr.constant().and_then(|value| value.as_number());
        let mut refuse_at = |span: Option<Span>, path: String, message: &str| {
            let mut diagnostic = Diagnostic::new(code::VALIDATION, message).with_path(path);
            if let Some(span) = span {
                diagnostic = diagnostic.at(cx.sources(), span);
            }
            cx.report(diagnostic);
        };
        if constant(&self.tempo).is_some_and(|tempo| tempo <= Real::ZERO) {
            refuse_at(
                self.tempo.span(),
                "tempo".to_owned(),
                "tempo divides the duration of phases and must be above zero",
            );
        }
        for phase in &self.phases {
            let base = format!("phases.{}", phase.name());
            if let PhaseTime::For(duration) = &phase.time
                && constant(duration).is_some_and(|seconds| seconds < Real::ZERO)
            {
                refuse_at(
                    duration.span(),
                    format!("{base}.time"),
                    "a phase cannot last less than no time",
                );
            }
            if let Some(max) = &phase.max
                && constant(max).is_some_and(|seconds| seconds < Real::ZERO)
            {
                refuse_at(
                    max.span(),
                    format!("{base}.max"),
                    "a held phase cannot last less than no time",
                );
            }
            if let Some(tick) = &phase.tick
                && constant(&tick.every).is_some_and(|seconds| seconds <= Real::ZERO)
            {
                refuse_at(
                    tick.every.span(),
                    format!("{base}.tick.every"),
                    "a tick's interval must be above zero",
                );
            }
        }
    }

    fn placeholder() -> Option<ActionDef> {
        Some(ActionDef::inert())
    }
}
