//! Defences (07 §5): the `defence` block of an action, which defences
//! are in force when a strike lands, and what each does to it.
//!
//! ```json
//! "defence": {
//!   "during": "guard",
//!   "form": "interpose",
//!   "with": { "held": "blade", "surface": "flat" },
//!   "covers": { "arc": 120 },
//!   "handles": 1,
//!   "contest": { "id": "control", "by": "fighting(self)", "against": "fighting(attacker)" },
//!   "deflect": 0.9,
//!   "holds": "self.attr.strength * 60",
//!   "brace": 0.3 }
//! ```
//!
//! # In force
//!
//! A defence is in force while its action is in the phase `during`
//! names, and only then (invariant C3): a reaction that is not yet ready
//! when the blow lands, or a guard let go, defends nothing. A held phase
//! is a guard, in force for as long as it is held.
//!
//! # At step 5 of a strike
//!
//! The defences in force for the target are tried in a fixed order:
//! first those of others that protect it, then its own; within each, in
//! the order their phases began, then by entity id, then by action. Each
//! is tried if it covers the strike (the direction it comes from, within
//! `covers.arc` of the defender's facing; the part it landed on, among
//! `covers.parts`) and has not dealt with `handles` strikes yet. Its
//! `contest`, if it has one, is drawn from the rule stream; at or above
//! zero it succeeds. `strike_defended` reports each defence tried.
//!
//! - **avoid** (slipping): the strike misses.
//! - **interpose**: `turned = deflect × min(energy, holds)` is taken off
//!   and the next defence meets what is left. The push share of what is
//!   left pushes the target, less the largest `brace` of the defences
//!   that succeeded. The rest lands on each interposed surface in turn,
//!   as a packet of the strike's kind at an intensity of the energy over
//!   that surface's area times its give; what passes the last continues
//!   to the part originally aimed at, with the striking surface. What an
//!   interposed surface was not affected by comes back into the striking
//!   surface as recoil.
//!
//! Leaving (a dodge that moves) needs nothing here: a defender out of
//! reach is missed at step 2, with no contest.

use std::sync::Arc;

use clash_core::{EntityId, Real, Tick};
use clash_data::expr::{Any, Number};
use clash_data::{
    CompileCx, Expr, FromKind, Kind, KindValue, Ref, Runner, Scope, ValidateCx, by_kind,
};
use clash_expr::Value;
use clash_world::Cause;
use serde::Deserialize;
use serde::de::{Deserializer, MapAccess};

use super::attack::WithDef;
use super::contest::ContestDef;
use super::surface::{self, Surface};
use crate::Sim;
use crate::action::{ActionDefId, PhaseRef};
use crate::body::PartKey;
use crate::bus::{EventPayload, FieldValue};
use crate::event::{Subject, Where};
use crate::host::{Bound, RULE_STREAM};
use crate::later::ActionId;
use crate::registry::PropertyDef;

/// The subject a defence's expressions read the one striking as.
pub const ATTACKER_SUBJECT: &str = "attacker";

/// Raised for each defence a strike was tried against.
pub const STRIKE_DEFENDED: &str = "strike_defended";
/// The hook raised before a defence is applied.
pub const DEFENCE_RESOLVING: &str = "defence_resolving";

/// Degrees in a whole turn.
const TURN_DEG: Real = Real::of(360.0);
/// Degrees in a half turn.
const HALF_TURN_DEG: Real = Real::of(180.0);

/// How a defence works (07 §5.1).
#[derive(Clone, Copy, PartialEq, Eq, Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DefenceForm {
    /// The defender is not where the strike lands: on success it misses.
    Avoid,
    /// Something is put in the way: a blade, a shield, an arm.
    Interpose,
}

impl DefenceForm {
    /// The form as data writes it.
    pub const fn name(self) -> &'static str {
        match self {
            DefenceForm::Avoid => "avoid",
            DefenceForm::Interpose => "interpose",
        }
    }
}

/// How many strikes a defence deals with while in force (07 §5.1).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Handles {
    /// At most this many.
    Count(u32),
    /// Every one: a shield wall.
    All,
}

impl Default for Handles {
    fn default() -> Handles {
        Handles::Count(1)
    }
}

impl Handles {
    /// Whether a defence that has dealt with `count` strikes may deal with
    /// one more.
    pub fn allows(self, count: u32) -> bool {
        match self {
            Handles::Count(most) => count < most,
            Handles::All => true,
        }
    }
}

impl<'de> FromKind<'de> for Handles {
    fn from_kind<A: MapAccess<'de>>(
        kind: Kind,
        value: KindValue<'_, A>,
    ) -> Result<Handles, A::Error> {
        match kind {
            Kind::Number => Ok(Handles::Count(value.read::<u32>()?)),
            Kind::String => {
                let word = value.read::<String>()?;
                if word == "all" {
                    Ok(Handles::All)
                } else {
                    Err(Kind::String.unexpected("a number of strikes, or `\"all\"`"))
                }
            }
            other => Err(other.unexpected("a number of strikes, or `\"all\"`")),
        }
    }
}

impl<'de> Deserialize<'de> for Handles {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Handles, D::Error> {
        by_kind(deserializer)
    }
}

/// What is put in the way: one source of a surface, or a list of them,
/// of which the first that answers is used (a shield if one is held,
/// else the arm).
#[derive(Debug, Default)]
pub struct WithList(pub Vec<WithDef>);

impl<'de> FromKind<'de> for WithList {
    fn from_kind<A: MapAccess<'de>>(
        kind: Kind,
        value: KindValue<'_, A>,
    ) -> Result<WithList, A::Error> {
        match kind {
            Kind::Object => Ok(WithList(vec![value.read::<WithDef>()?])),
            Kind::List => Ok(WithList(value.read::<Vec<WithDef>>()?)),
            other => Err(other.unexpected(
                "what is put in the way: `{ \"held\": tag, \"surface\": id }`, `{ \"part\": name, \"surface\": id }`, or a list of them",
            )),
        }
    }
}

impl<'de> Deserialize<'de> for WithList {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<WithList, D::Error> {
        by_kind(deserializer)
    }
}

/// Which strikes a defence applies to (07 §5.1).
#[derive(Debug, Default, Deserialize)]
pub struct Covers {
    /// The angle, in degrees, centred on the defender's facing, that the
    /// strike must come from. Left out, every direction.
    #[serde(default)]
    pub arc: Option<Real>,
    /// The parts the strike must land on: each a part's name in the
    /// body, the id of its definition or a tag its definition carries.
    /// Left out, every part.
    #[serde(default)]
    pub parts: Vec<String>,
}

/// Whether a defence succeeds (07 §5.1): a contest between the
/// defender's value and the attacker's.
#[derive(Debug, Deserialize)]
pub struct DefenceContest {
    /// The contest.
    pub id: Ref<ContestDef>,
    /// The defender's value.
    pub by: Expr<Number>,
    /// The attacker's value.
    pub against: Expr<Number>,
}

fn zero() -> Expr<Number> {
    Expr::literal(0)
}

/// The `defence` block of an action (07 §5.1).
///
/// | Field | Read |
/// |-------|------|
/// | `during` | The id of the phase in which it is in force. Required. |
/// | `form` | `avoid` or `interpose`. Required. |
/// | `with` | For `interpose`: what is put in the way, as an attack's `with` (a held item, a part; not inline), or a list of them, the first that answers being used. Required for `interpose`; not allowed for `avoid`. |
/// | `protects` | An expression giving whom it defends. Left out: the defender. Another is defended only while the defender stands beside them. |
/// | `covers` | [`Covers`]. Left out: every strike. |
/// | `handles` | How many strikes it deals with while in force, or `"all"`. Default 1. |
/// | `contest` | [`DefenceContest`]. Left out: it always succeeds. |
/// | `deflect` | The share, 0 to 1, of the strike's energy turned aside on success. Default 0. |
/// | `holds` | The most joules it can turn aside. Left out: no limit. |
/// | `brace` | The share, 0 to 1, of the push the defender resists. Default 0. |
/// | `evasion` | What it adds, while in force, to the defender's value in the aim of a strike at it that comes from within `covers.arc` (07 §4.2). Default none. |
///
/// `contest`, `deflect`, `holds`, `brace` and `evasion` read truth when the strike
/// lands, with the subjects `self` (the defender), `attacker` and
/// `target` (the one struck) and the action's locals; `protects` reads
/// truth with `self` and the locals.
#[derive(Debug, Deserialize)]
pub struct DefenceDef {
    /// The phase in which it is in force.
    pub during: PhaseRef,
    /// How it works.
    pub form: DefenceForm,
    /// What is put in the way.
    #[serde(default)]
    pub with: WithList,
    /// Whom it defends; the defender if left out.
    #[serde(default)]
    pub protects: Option<Expr<Any>>,
    /// Which strikes it applies to.
    #[serde(default)]
    pub covers: Covers,
    /// How many strikes it deals with while in force.
    #[serde(default)]
    pub handles: Handles,
    /// Whether it succeeds.
    #[serde(default)]
    pub contest: Option<DefenceContest>,
    /// The share of the strike's energy turned aside on success.
    #[serde(default = "zero")]
    pub deflect: Expr<Number>,
    /// The most energy, in joules, it can turn aside.
    #[serde(default)]
    pub holds: Option<Expr<Number>>,
    /// The share of the push the defender resists.
    #[serde(default = "zero")]
    pub brace: Expr<Number>,
    /// What it adds to the defender's value in the aim of a strike at it
    /// while it is in force (07 §4.2): a sway or a weave makes someone
    /// harder to land on squarely. Left out, nothing.
    #[serde(default)]
    pub evasion: Option<Expr<Number>>,
}

impl DefenceDef {
    /// Compiles its expressions: `exprs` binds `self`, `attacker`,
    /// `target` and the locals; `protects` binds `self` and the locals.
    pub(crate) fn compile(&mut self, cx: &mut CompileCx<'_>, exprs: &Scope, protects: &Scope) {
        cx.compile(&mut self.protects, &protects.clone().expecting("entity?"));
        if let Some(contest) = &mut self.contest {
            cx.compile(&mut contest.by, exprs);
            cx.compile(&mut contest.against, exprs);
        }
        cx.compile(&mut self.deflect, exprs);
        cx.compile(&mut self.holds, exprs);
        cx.compile(&mut self.brace, exprs);
        cx.compile(&mut self.evasion, exprs);
    }

    /// The block's own checks.
    pub(crate) fn validate(&self, cx: &mut ValidateCx<'_>) {
        match self.form {
            DefenceForm::Interpose => {
                if self.with.0.is_empty() {
                    cx.error(
                        "defence.with: an interposing defence says what it puts in the way: `{ \"held\": tag, \"surface\": id }` or `{ \"part\": name, \"surface\": id }`".to_owned(),
                    );
                }
                for (at, with) in self.with.0.iter().enumerate() {
                    let path = format!("defence.with[{at}]");
                    if matches!(with.source(), Some(super::WithSource::Inline)) {
                        cx.error(format!(
                            "{path}: a defence puts a held item or a part in the way, not an inline surface"
                        ));
                    }
                }
            }
            DefenceForm::Avoid => {
                if !self.with.0.is_empty() {
                    cx.error(
                        "defence.with: a defence that avoids puts nothing in the way; leave `with` out, or make it `interpose`".to_owned(),
                    );
                }
            }
        }
        for (name, expr) in [("deflect", &self.deflect), ("brace", &self.brace)] {
            if let Some(value) = expr.constant().and_then(|value| value.as_number())
                && !(value >= Real::ZERO && value <= Real::ONE)
            {
                cx.error(format!("defence.{name}: is a share, from 0 to 1"));
            }
        }
        if let Some(value) = self
            .holds
            .as_ref()
            .and_then(Expr::constant)
            .and_then(|value| value.as_number())
            && !(value.is_finite() && value >= Real::ZERO)
        {
            cx.error("defence.holds: is joules, zero or more".to_owned());
        }
        if self.handles == Handles::Count(0) {
            cx.error(
                "defence.handles: a defence deals with at least one strike, or `\"all\"`"
                    .to_owned(),
            );
        }
        if let Some(arc) = self.covers.arc
            && !(arc.is_finite() && arc > Real::ZERO && arc <= TURN_DEG)
        {
            cx.error(
                "defence.covers.arc: is an angle in degrees, above 0 and at most 360".to_owned(),
            );
        }
    }
}

/// One defence in force for a target.
#[derive(Clone, Copy, Debug)]
struct InForce {
    action: ActionId,
    defender: EntityId,
    def: ActionDefId,
    since: Tick,
    own: bool,
}

/// The strike a defence meets.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Incoming<'a> {
    pub attacker: EntityId,
    pub target: EntityId,
    /// The part the strike landed on, if aim found one.
    pub part: Option<PartKey>,
    /// Joules reaching the target.
    pub energy: Real,
    pub action: &'a str,
    pub attack: &'a str,
    pub cause: Cause,
}

/// A surface something was put in the way with.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Interposed {
    /// Whose it is.
    pub defender: EntityId,
    /// What it is.
    pub surface: Surface,
}

/// What the defences did to a strike.
#[derive(Clone, Debug)]
pub(crate) enum Defended {
    /// Nothing: none was in force, covered it, or succeeded.
    Nothing,
    /// One that avoids succeeded: the strike misses. The margin of its
    /// contest, if it had one.
    Avoided(Option<Real>),
    /// Something was put in the way.
    Interposed {
        /// Joules turned aside.
        turned: Real,
        /// The surfaces in the way, in the order the strike meets them.
        surfaces: Vec<Interposed>,
        /// The share of the push the target resists.
        brace: Real,
    },
}

/// The defence block of a running action, and its phase.
fn defence_of(sim: &Sim, action: ActionId) -> Option<(&DefenceDef, ActionDefId)> {
    let running = sim.actions.get(action)?;
    let def = crate::action::run::action_def(sim.defs(), running.def)?;
    let defence = def.defence.as_ref()?;
    (defence.during.index() == running.phase).then_some((defence, running.def))
}

/// Whether an entity has a defence of its own in force now.
pub fn defending(sim: &Sim, entity: EntityId) -> bool {
    sim.actions_of(entity)
        .iter()
        .any(|id| defence_of(sim, *id).is_some())
}

/// The defences in force for a target, in the order they are tried: see
/// the [module documentation](self).
fn in_force(sim: &mut Sim, target: EntityId) -> Vec<InForce> {
    let running: Vec<(ActionId, EntityId, ActionDefId, Tick)> = sim
        .actions
        .iter()
        .filter(|(id, _)| defence_of(sim, *id).is_some())
        .map(|(id, running)| (id, running.actor, running.def, running.entered))
        .collect();
    let defs = Arc::clone(sim.defs());
    let mut out = Vec::new();
    for (action, defender, def_id, since) in running {
        let Some(defence) =
            crate::action::run::action_def(&defs, def_id).and_then(|def| def.defence.as_ref())
        else {
            continue;
        };
        let protected = match &defence.protects {
            None => defender,
            Some(expr) => {
                let locals: Vec<Value> = sim
                    .actions
                    .get(action)
                    .map(|running| running.locals.iter().map(Value::from).collect())
                    .unwrap_or_default();
                let value = sim.action_eval(defender, &locals, |defs, host| {
                    expr.eval_on(defs, host).unwrap_or_default()
                });
                match sim.subject_of(&value) {
                    Some(Subject::Entity(id)) => id,
                    _ => continue,
                }
            }
        };
        if protected != target {
            continue;
        }
        let own = defender == target;
        // Another is defended only from beside them.
        if !own && super::strike::gap(sim, defender, target).is_none_or(|gap| gap > Real::ZERO) {
            continue;
        }
        out.push(InForce {
            action,
            defender,
            def: def_id,
            since,
            own,
        });
    }
    out.sort_by(|a, b| {
        a.own
            .cmp(&b.own)
            .then(a.since.cmp(&b.since))
            .then(a.defender.cmp(&b.defender))
            .then(a.action.cmp(&b.action))
    });
    out
}

/// An angle in degrees brought into `[0, 360)`.
fn wrap(degrees: Real) -> Real {
    let wrapped = degrees - (degrees / TURN_DEG).floor() * TURN_DEG;
    if wrapped >= TURN_DEG {
        Real::ZERO
    } else {
        wrapped
    }
}

/// The bearing from one entity to another, in degrees clockwise from
/// north; `None` for two in one cell or not in the world.
fn bearing(sim: &Sim, from: EntityId, to: EntityId) -> Option<Real> {
    let v = super::strike::offset(sim, from, to)?;
    if v[0] == Real::ZERO && v[1] == Real::ZERO {
        return None;
    }
    let degrees = v[0].atan2(-v[1]) * HALF_TURN_DEG / Real::PI;
    Some(wrap(degrees))
}

/// Whether a defence covers a strike: its direction and its part.
fn covers(sim: &Sim, defence: &DefenceDef, defender: EntityId, incoming: &Incoming<'_>) -> bool {
    if let Some(arc) = defence.covers.arc
        && arc < TURN_DEG
        && let Some(facing) = sim
            .components
            .senses
            .get(defender)
            .map(|senses| senses.facing())
        && let Some(bearing) = bearing(sim, defender, incoming.attacker)
    {
        let off = wrap(bearing - facing + HALF_TURN_DEG) - HALF_TURN_DEG;
        if off.abs() > arc / Real::of(2.0) {
            return false;
        }
    }
    if defence.covers.parts.is_empty() {
        return true;
    }
    let Some(part) = incoming.part.and_then(|key| {
        sim.components()
            .body
            .get(incoming.target)
            .and_then(|body| body.part(key))
    }) else {
        // Where the body picks the part, any guard is in the way.
        return true;
    };
    defence
        .covers
        .parts
        .iter()
        .any(|name| surface::answers(sim, part, name))
}

/// Evaluates an expression of a defence against truth, with `self`,
/// `attacker`, `target` and the defending action's locals bound.
fn eval(
    sim: &mut Sim,
    force: &InForce,
    incoming: &Incoming<'_>,
    expr: &Expr<Number>,
) -> Option<Real> {
    let defs = Arc::clone(sim.defs());
    let locals: Vec<Value> = sim
        .actions
        .get(force.action)
        .map(|running| running.locals.iter().map(Value::from).collect())
        .unwrap_or_default();
    let bound = Bound {
        subjects: vec![
            sim.subject_value(Subject::Entity(force.defender)),
            sim.subject_value(Subject::Entity(incoming.attacker)),
            sim.subject_value(Subject::Entity(incoming.target)),
        ],
        locals,
        fields: Vec::new(),
    };
    let (value, warnings) = sim.with_random(bound, |host| {
        let mut runner = Runner::new();
        let value = expr.eval(&mut runner.on(&defs, host));
        (value, host.take_warnings())
    });
    for warning in warnings {
        sim.report_warning(warning);
    }
    value
}

/// A share from 0 to 1: what a value of `deflect` or `brace` gives.
fn share(value: Option<Real>) -> Real {
    value
        .filter(|value| value.is_finite())
        .map_or(Real::ZERO, |value| value.max(Real::ZERO).min(Real::ONE))
}

/// Step 5 of a strike (07 §3.4, §5): tries the defences in force for
/// the target, raising `strike_defended` for each, and says what they
/// did. See the [module documentation](self).
pub(crate) fn defend(sim: &mut Sim, incoming: &Incoming<'_>) -> Defended {
    let forces = in_force(sim, incoming.target);
    if forces.is_empty() {
        return Defended::Nothing;
    }
    let defs = Arc::clone(sim.defs());
    let mut energy = incoming.energy;
    let mut turned = Real::ZERO;
    let mut surfaces = Vec::new();
    let mut brace = Real::ZERO;
    for force in forces {
        let Some(def) = crate::action::run::action_def(&defs, force.def) else {
            continue;
        };
        let Some(defence) = def.defence.as_ref() else {
            continue;
        };
        if !defence
            .handles
            .allows(sim.windows.handled(force.action, force.since))
        {
            continue;
        }
        if !covers(sim, defence, force.defender, incoming) {
            continue;
        }
        let defence_name = crate::action::run::action_name(&defs, force.def);
        // What it puts in the way, found now: a blade dropped since it
        // was raised is not there.
        let found = match defence.form {
            DefenceForm::Avoid => None,
            DefenceForm::Interpose => {
                let found = defence.with.0.iter().find_map(|with| {
                    surface::find(sim, force.defender, with, &defence_name, "defence").ok()
                });
                if found.is_none() {
                    continue;
                }
                found
            }
        };
        sim.windows.count_handled(force.action, force.since);
        let margin = defence.contest.as_ref().map(|contest| {
            let by = eval(sim, &force, incoming, &contest.by).unwrap_or(Real::ZERO);
            let against = eval(sim, &force, incoming, &contest.against).unwrap_or(Real::ZERO);
            let def = defs
                .try_registry::<ContestDef>()
                .and_then(|registry| registry.try_get(contest.id.id()));
            let rng = sim.rng.stream(RULE_STREAM);
            match def {
                Some(def) => def.margin(by, against, rng),
                None => by - against,
            }
        });
        let mut succeeded = margin.is_none_or(|margin| margin >= Real::ZERO);
        let mut deflect = share(eval(sim, &force, incoming, &defence.deflect));
        let mut holds = defence
            .holds
            .as_ref()
            .and_then(|holds| eval(sim, &force, incoming, holds))
            .map(|holds| holds.max(Real::ZERO));
        let mut braced = share(eval(sim, &force, incoming, &defence.brace));

        // The hook: a handler may change how much it turns, or make it
        // fail.
        if sim.bus.has_listeners(DEFENCE_RESOLVING) {
            let payload = EventPayload::new(DEFENCE_RESOLVING)
                .subject("self", Subject::Entity(force.defender))
                .subject("attacker", Subject::Entity(incoming.attacker))
                .subject("target", Subject::Entity(incoming.target))
                .with("action", defence_name.as_str())
                .with("form", defence.form.name())
                .with("succeeded", succeeded)
                .with("margin", margin.map_or(FieldValue::Null, FieldValue::from))
                .with("deflect", deflect)
                .with("holds", holds.map_or(FieldValue::Null, FieldValue::from))
                .with("brace", braced);
            match sim.raise(Where::Entity(force.defender), incoming.cause, payload) {
                Ok(raised) => {
                    if raised.cancelled.is_some() {
                        succeeded = false;
                    }
                    if let Some(FieldValue::Plain(clash_core::Value::Bool(changed))) =
                        raised.field("succeeded")
                    {
                        succeeded = *changed;
                    }
                    if let Some(changed) = raised.field("deflect").and_then(FieldValue::as_real) {
                        deflect = share(Some(changed));
                    }
                    match raised.field("holds") {
                        Some(FieldValue::Null) => holds = None,
                        Some(field) => {
                            if let Some(changed) = field.as_real() {
                                holds = Some(changed.max(Real::ZERO));
                            }
                        }
                        None => {}
                    }
                    if let Some(changed) = raised.field("brace").and_then(FieldValue::as_real) {
                        braced = share(Some(changed));
                    }
                }
                Err(reason) => sim.fault(reason),
            }
        }

        let mut this_turned = Real::ZERO;
        if succeeded && defence.form == DefenceForm::Interpose {
            let held = holds.map_or(energy, |holds| energy.min(holds));
            this_turned = deflect * held;
            energy -= this_turned;
            turned += this_turned;
            brace = brace.max(braced);
            if let Some(surface) = found {
                surfaces.push(Interposed {
                    defender: force.defender,
                    surface,
                });
            }
        }
        let payload = EventPayload::new(STRIKE_DEFENDED)
            .subject("self", Subject::Entity(incoming.attacker))
            .subject("target", Subject::Entity(incoming.target))
            .subject("defender", Subject::Entity(force.defender))
            .with("action", incoming.action)
            .with("attack", incoming.attack)
            .with("defence", defence_name.as_str())
            .with("form", defence.form.name())
            .with("succeeded", succeeded)
            .with("margin", margin.map_or(FieldValue::Null, FieldValue::from))
            .with("turned", this_turned);
        if let Err(reason) = sim.raise(Where::Entity(incoming.attacker), incoming.cause, payload) {
            sim.fault(reason);
        }
        if succeeded && defence.form == DefenceForm::Avoid {
            return Defended::Avoided(margin);
        }
    }
    if surfaces.is_empty() && turned <= Real::ZERO {
        return Defended::Nothing;
    }
    Defended::Interposed {
        turned,
        surfaces,
        brace,
    }
}

/// What the target's own defences in force add to its value in the aim
/// of a strike at it (07 §4.2, §5.1 `evasion`): the sum of the
/// `evasion` of each that covers the direction the strike comes from.
/// Zero where none has one.
pub(crate) fn evasion(sim: &mut Sim, incoming: &Incoming<'_>) -> Real {
    let forces: Vec<InForce> = in_force(sim, incoming.target)
        .into_iter()
        .filter(|force| force.own)
        .collect();
    let defs = Arc::clone(sim.defs());
    let mut total = Real::ZERO;
    for force in forces {
        let Some(defence) =
            crate::action::run::action_def(&defs, force.def).and_then(|def| def.defence.as_ref())
        else {
            continue;
        };
        let Some(expr) = &defence.evasion else {
            continue;
        };
        // The part is not known before the aim: only the direction counts.
        let unaimed = Incoming {
            part: None,
            ..*incoming
        };
        if !covers(sim, defence, force.defender, &unaimed) {
            continue;
        }
        if let Some(value) = eval(sim, &force, incoming, expr).filter(|value| value.is_finite()) {
            total += value;
        }
    }
    total
}

/// The reason a strike that defences stopped delivered nothing.
pub const DEFENDED: &str = "combat.defended";

/// The first thing an entity holds in a grasp, in order of id, whose
/// definition has a striking surface of this id (07 §3.2): what a parry
/// that puts the flat of something in the way would use.
pub fn wielding_surface(sim: &Sim, who: EntityId, surface: &str) -> Option<EntityId> {
    let holds = sim.components().get::<crate::item::Holds>(who)?;
    holds.iter().find_map(|(item, hold)| {
        let crate::item::Hold::Grasp(_) = hold else {
            return None;
        };
        let def = sim
            .components()
            .get::<crate::item::Item>(item)
            .and_then(|component| crate::item::def_of(sim.defs(), component))?;
        def.surfaces
            .iter()
            .any(|have| have.id == surface)
            .then_some(item)
    })
}

/// The properties of defences (07 §16): `entity.defending` and
/// `entity.wielding_surface`.
pub fn properties() -> Vec<PropertyDef> {
    vec![
        PropertyDef::new(
            "entity",
            "wielding_surface",
            "entity?",
            "The first thing it holds in a grasp, in order of id, whose definition has a striking surface of this id (07 §3.2): the knife or the club whose `flat` a parry puts in a blow's way. Null for none. Readable of oneself in a mind context, so that a reaction's `requires` can say it cannot be done with nothing to do it with.",
            |sim, subject, args| {
                let surface = args.first().and_then(Value::as_str).unwrap_or_default();
                subject
                    .entity()
                    .and_then(|id| wielding_surface(sim, id, surface))
                    .map_or(Value::Null, |item| sim.subject_value(Subject::Entity(item)))
            },
        )
        .arg("surface", "string", "The id of the surface: `flat`, `edge`, `face`.")
        .own()
        .changed_by(&["item_taken", "item_dropped", "entity_relocated"]),
        PropertyDef::new(
            "entity",
            "defending",
            "bool",
            "Whether a defence of its own is in force now (07 §5.1): one of its actions with a `defence` is in the phase its `during` names.",
            |sim, subject, _| match subject {
                Subject::Entity(id) => Value::Bool(defending(sim, id)),
                _ => Value::Null,
            },
        )
        .changed_by(&["phase_entered", "action_finished", "action_cancelled", "action_interrupted", "action_abandoned"]),
    ]
}
