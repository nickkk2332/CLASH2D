//! The phases of an action (03 §6.4): what was written, and what the
//! runtime walks.

use clash_data::expr::{Any, Number, Written};
use clash_data::{Cond, Expr, FromKind, Kind, KindValue, Located, Ref, Span, Tag, TagSet, by_kind};
use serde::Deserialize;
use serde::de::{Deserializer, MapAccess};

use super::window::WindowKindDef;
use crate::action::ids::{PhaseIndex, WindowKindId};
use crate::rule::Effects;

/// What `time` is written as for a phase that lasts until it is let go.
pub const HOLD: &str = "hold";

/// A phase named in data, and its position once the action has loaded.
///
/// A `goto`, a window's `closes` and a reaction's `ready` are each one of
/// these. The loader resolves the name against the action's phases; a
/// name that is no phase's breaks the definition, so in a loaded action
/// the index is always that of a phase.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct PhaseRef {
    name: Located<String>,
    index: PhaseIndex,
}

impl PhaseRef {
    /// The position of the phase among the action's phases.
    pub fn index(&self) -> PhaseIndex {
        self.index
    }

    /// The phase's id, as written.
    pub fn name(&self) -> &str {
        &self.name.value
    }

    /// Where the id was written.
    pub fn span(&self) -> Span {
        self.name.span
    }

    pub(crate) fn resolve(&mut self, index: PhaseIndex) {
        self.index = index;
    }
}

impl<'de> Deserialize<'de> for PhaseRef {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<PhaseRef, D::Error> {
        Ok(PhaseRef {
            name: Located::<String>::deserialize(deserializer)?,
            index: PhaseIndex::new(0),
        })
    }
}

/// How long a phase lasts (03 §6.4).
#[derive(Clone, Debug)]
pub enum PhaseTime {
    /// For a duration: an expression giving seconds of the actor's local
    /// time, evaluated when the phase starts and divided by the action's
    /// tempo unless the phase is `fixed`. Zero runs the phase and moves
    /// on within the same tick.
    For(Expr<Number>),
    /// Until it is let go (`"hold"`): by its `until`, by its `max`, or by
    /// a `Cancel` command, which counts as release.
    Hold,
}

impl PhaseTime {
    /// Whether the phase lasts until it is let go.
    pub fn is_hold(&self) -> bool {
        matches!(self, PhaseTime::Hold)
    }

    /// The duration, for a phase that has one.
    pub fn duration(&self) -> Option<&Expr<Number>> {
        match self {
            PhaseTime::For(duration) => Some(duration),
            PhaseTime::Hold => None,
        }
    }
}

impl<'de> Deserialize<'de> for PhaseTime {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<PhaseTime, D::Error> {
        let duration = Expr::<Number>::deserialize(deserializer)?;
        let hold = matches!(duration.written(), Written::Text(text) if text.trim() == HOLD);
        Ok(if hold {
            PhaseTime::Hold
        } else {
            PhaseTime::For(duration)
        })
    }
}

/// Whether the actor may abandon the action during a phase (03 §6.4).
///
/// In data: `false` (the default), `true`, or a list of action tags.
#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub enum CancelRule {
    /// Not during this phase.
    #[default]
    Never,
    /// At will.
    Always,
    /// Only to start an action that carries one of these tags: a recovery
    /// that a follow-up may cut short and nothing else.
    For(TagSet),
}

impl CancelRule {
    /// Whether the phase may be abandoned at will.
    pub fn at_will(&self) -> bool {
        matches!(self, CancelRule::Always)
    }

    /// Whether the phase may be abandoned to start an action carrying
    /// these tags.
    pub fn allows(&self, tags: &TagSet) -> bool {
        match self {
            CancelRule::Never => false,
            CancelRule::Always => true,
            CancelRule::For(allowed) => allowed.iter().any(|tag| tags.contains(tag)),
        }
    }
}

impl<'de> FromKind<'de> for CancelRule {
    fn from_kind<A: MapAccess<'de>>(
        kind: Kind,
        value: KindValue<'_, A>,
    ) -> Result<CancelRule, A::Error> {
        match kind {
            Kind::Bool => Ok(if value.read::<bool>()? {
                CancelRule::Always
            } else {
                CancelRule::Never
            }),
            Kind::List => {
                let tags: TagSet = value.read::<Vec<Tag>>()?.into_iter().collect();
                Ok(CancelRule::For(tags))
            }
            other => Err(other.unexpected("`true`, `false` or a list of action tags")),
        }
    }
}

impl<'de> Deserialize<'de> for CancelRule {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<CancelRule, D::Error> {
        by_kind(deserializer)
    }
}

/// Where a phase leads once it has ended.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Default)]
pub enum Next {
    /// Into the phase at this position.
    Phase(PhaseIndex),
    /// Out of the action: it finishes.
    #[default]
    Finish,
}

/// One entry of a phase's `next` (03 §6.4): a condition and where to go
/// when it holds.
///
/// ```json
/// "next": [ { "if": "$power >= 4", "goto": "release" }, { "finish": true } ]
/// ```
#[derive(Debug, Deserialize)]
pub struct Transition {
    /// The condition, in the action's scope, reading truth. Left out, the
    /// entry always holds.
    #[serde(default, rename = "if")]
    pub when: Option<Cond>,
    /// The phase to enter.
    #[serde(default)]
    pub goto: Option<PhaseRef>,
    /// Whether the action finishes here. An entry has `goto` or
    /// `"finish": true`, not both.
    #[serde(default)]
    pub finish: Option<Located<bool>>,
    #[serde(skip)]
    to: Next,
}

impl Transition {
    /// Where the entry leads.
    pub fn to(&self) -> Next {
        self.to
    }

    /// Whether the entry holds whatever the state: it has no condition,
    /// or its condition is the constant `true`.
    pub fn is_unconditional(&self) -> bool {
        self.when
            .as_ref()
            .is_none_or(|when| when.constant_value() == Some(true))
    }

    /// Whether the entry can never hold: its condition is the constant
    /// `false`.
    pub fn is_never(&self) -> bool {
        self.when
            .as_ref()
            .is_some_and(|when| when.constant_value() == Some(false))
    }

    pub(crate) fn resolve(&mut self, to: Next) {
        self.to = to;
    }
}

/// The reaction window a phase opens when it starts (03 §7.1).
///
/// ```json
/// "window": { "kind": "strike", "for": ["$target"], "closes": "impact" }
/// ```
#[derive(Debug, Deserialize)]
pub struct PhaseWindow {
    /// The kind of window.
    pub kind: Ref<WindowKindDef>,
    /// Those it is aimed at: expressions in the action's scope, each
    /// giving an entity. One that comes to null names nobody.
    #[serde(default, rename = "for")]
    pub aimed_at: Vec<Expr<Any>>,
    /// The later phase whose start is the window's deadline.
    pub closes: PhaseRef,
}

impl PhaseWindow {
    /// The kind of window, by its dense id.
    pub fn kind_id(&self) -> WindowKindId {
        self.kind.id()
    }
}

/// Effects repeated while a phase lasts (03 §6.4).
///
/// ```json
/// "tick": { "every": "250 ms", "do": [ { "add": { "$power": 1 } } ] }
/// ```
#[derive(Debug, Deserialize)]
pub struct Tick {
    /// The interval: an expression giving seconds of the actor's local
    /// time, evaluated when the phase starts. Tempo does not divide it.
    pub every: Expr<Number>,
    /// The effects run at each interval. May be left out, for a tick that
    /// only tests the phase's `until`.
    #[serde(default, rename = "do")]
    pub effects: Effects,
}

/// One phase of an action (03 §6.4).
///
/// ```json
/// { "id": "windup", "time": "300 ms", "cancel": true,
///   "window": { "kind": "strike", "for": ["$target"], "closes": "impact" } }
/// ```
#[derive(Debug, Deserialize)]
pub struct Phase {
    /// The phase's name within its action.
    pub id: Located<String>,
    /// How long it lasts.
    pub time: PhaseTime,
    /// For a held phase: the longest it lasts, an expression giving
    /// seconds of the actor's local time, evaluated when the phase
    /// starts. Tempo does not divide it.
    #[serde(default)]
    pub max: Option<Expr<Number>>,
    /// For a held phase: it ends when this holds. Tested at each tick, so
    /// a phase with `until` has a `tick`.
    #[serde(default)]
    pub until: Option<Cond>,
    /// Effects run when the phase starts.
    #[serde(default, rename = "do")]
    pub on_enter: Effects,
    /// Effects run when the phase ends normally.
    #[serde(default)]
    pub on_exit: Effects,
    /// Effects repeated at an interval while the phase lasts.
    #[serde(default)]
    pub tick: Option<Tick>,
    /// Whether the actor may abandon the action during this phase.
    #[serde(default)]
    pub cancel: CancelRule,
    /// Whether the `interrupt` effect is unable to end the action during
    /// this phase.
    #[serde(default)]
    pub uninterruptible: bool,
    /// Whether tempo leaves this phase's duration alone.
    #[serde(default)]
    pub fixed: bool,
    /// The reaction window opened when the phase starts.
    #[serde(default)]
    pub window: Option<PhaseWindow>,
    /// Free-form tags, queryable by rules.
    #[serde(default)]
    pub tags: TagSet,
    /// Where to go when the phase ends: the first entry that holds. If
    /// none does, [`after`](Phase::after).
    #[serde(default)]
    pub next: Vec<Transition>,
    #[serde(skip)]
    after: Next,
}

impl Phase {
    /// The phase's name.
    pub fn name(&self) -> &str {
        &self.id.value
    }

    /// Where the phase leads when no entry of `next` holds: into the
    /// phase written after it, and out of the action after the last.
    pub fn after(&self) -> Next {
        self.after
    }

    /// Whether the phase is held.
    pub fn is_held(&self) -> bool {
        self.time.is_hold()
    }

    /// Whether leaving the phase can fall through to
    /// [`after`](Phase::after): no entry of `next` always holds.
    pub fn falls_through(&self) -> bool {
        !self.next.iter().any(Transition::is_unconditional)
    }

    /// Every place the phase can lead, in the order they are tried. An
    /// entry that can never hold is left out, and nothing follows one
    /// that always does.
    pub fn exits(&self) -> impl Iterator<Item = Next> + '_ {
        let mut open = true;
        let entries = self.next.iter().filter_map(move |entry| {
            if !open || entry.is_never() {
                return None;
            }
            open = !entry.is_unconditional();
            Some(entry.to())
        });
        entries.chain(self.falls_through().then_some(self.after))
    }

    pub(crate) fn set_after(&mut self, after: Next) {
        self.after = after;
    }
}

impl Phase {
    /// A phase of no length that leads out of the action.
    pub(crate) fn inert(nowhere: Span) -> Phase {
        Phase {
            id: Located {
                value: "placeholder".to_owned(),
                span: nowhere,
            },
            time: PhaseTime::For(Expr::literal(0)),
            max: None,
            until: None,
            on_enter: Effects::empty(),
            on_exit: Effects::empty(),
            tick: None,
            cancel: CancelRule::Never,
            uninterruptible: false,
            fixed: false,
            window: None,
            tags: TagSet::new(),
            next: Vec::new(),
            after: Next::Finish,
        }
    }
}
