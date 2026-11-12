//! Previews (07 §13): what a strike is likely to do, worked out without
//! drawing anything or changing anything (invariant C5).
//!
//! `strike_preview(who, action, target, part, effort, attack)` finds the
//! attack and what strikes as the strike would (07 §3.1), evaluates its
//! numbers with the attacker, the weapon or limb and the target bound
//! and the action's locals as a command would give them (`$target`,
//! `$part`, `$effort`; the second role of a joint action is the
//! target), through a host that may not draw, and gives:
//!
//! | Field | Type | Is |
//! |-------|------|----|
//! | `in_reach` | `bool` | Whether the target stands within the attack's reach now |
//! | `odds` | `real` | The chance it lands on the part aimed at: the contest's `odds` of its aim (07 §4.1); 1 for an attack that does not aim |
//! | `odds_near` | `real` | The chance it lands on that part or one beside it: a margin within the contest's `near` below zero |
//! | `energy` | `real` | Joules it would deliver as harm: its energy at the effort, less the push |
//! | `intensity` | `real` | How concentrated that is, in pascals (07 §6.1) |
//! | `push_speed` | `real` | Metres a second the push would give the target, unbraced (07 §7) |
//! | `staggers`, `launches` | `bool` | Whether that staggers it or launches it, by `combat.stagger_at` and `combat.launch_at` |
//!
//! It reads the target as it is: its place, its mass, and the value of
//! the aim's `against`, which for the bundled `evasion` reads whether
//! the target perceives the attacker. It is not safe for belief: a mind
//! cannot call it (decision D-815). Layers breached, parts disabled or
//! severed, what would pass and what would recoil need a dry run of the
//! body model, and are not given yet.

use std::sync::Arc;

use clash_core::{EntityId, Real};
use clash_data::Runner;
use clash_expr::Value;

use super::reason::{DEFAULT_LAUNCH_AT, DEFAULT_STAGGER_AT, LAUNCH_AT_SETTING, STAGGER_AT_SETTING};
use super::surface::{self, Striker};
use crate::Sim;
use crate::action::{ActionDef, ActionDefId, LocalSource};
use crate::body::props::{part_handle, part_of_handle};
use crate::event::Subject;
use crate::host::Bound;
use crate::registry::FunctionDef;

/// What a preview says.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Preview {
    /// Whether the target is within reach now.
    pub in_reach: bool,
    /// The chance it lands on the part aimed at.
    pub odds: Real,
    /// The chance it lands on that part or one beside it.
    pub odds_near: Real,
    /// Joules delivered as harm.
    pub energy: Real,
    /// Pascals.
    pub intensity: Real,
    /// Metres a second the push gives the target, unbraced.
    pub push_speed: Real,
    /// Whether that push staggers.
    pub staggers: bool,
    /// Whether that push launches.
    pub launches: bool,
}

/// A setting that is a number, or its default.
fn setting(sim: &Sim, key: &str, default: f64) -> Real {
    super::ranged::setting_real(sim, key).unwrap_or(Real::of(default))
}

/// The chance a margin of a contest is at or above `-near`.
fn odds_within(def: &super::ContestDef, a: Real, b: Real, near: Real) -> Real {
    let d = def.difference(a, b) + near;
    if def.spread > Real::ZERO && d.is_finite() {
        Real::ONE / (Real::ONE + (-d / def.spread).exp())
    } else if d >= Real::ZERO {
        Real::ONE
    } else {
        Real::ZERO
    }
}

/// Previews one attack of an action (07 §13): see the
/// [module documentation](self). `None` where the action has no such
/// attack, or nothing answers to what it strikes with.
pub fn strike_preview(
    sim: &Sim,
    who: EntityId,
    action: ActionDefId,
    attack: Option<&str>,
    target: EntityId,
    part: Option<crate::body::PartKey>,
    effort: Real,
) -> Option<Preview> {
    let defs = Arc::clone(sim.defs());
    let def: &ActionDef = crate::action::run::action_def(&defs, action)?;
    let (_, attack_def) = match attack {
        Some(name) => def.attacks.find(name)?,
        None if def.attacks.0.len() == 1 => (0, def.attacks.0.first()?),
        None => return None,
    };
    let action_name = crate::action::run::action_name(&defs, action);
    let found = surface::find(sim, who, &attack_def.with, &action_name, &attack_def.id.value).ok()?;
    let body_defs = sim.life.body_defs().ok();
    let weapon = match found.striker {
        Striker::Item { item, .. } | Striker::Gripped { held: item, .. } => {
            sim.subject_value(Subject::Entity(item))
        }
        _ => Value::Null,
    };
    let limb = match (found.striker, &body_defs) {
        (Striker::Part(key), Some(body_defs)) => {
            Value::Subject(part_handle(body_defs.kinds(), who, key))
        }
        _ => Value::Null,
    };
    let target_value = sim.subject_value(Subject::Entity(target));
    let part_value = match (part, &body_defs) {
        (Some(key), Some(body_defs)) => Value::Subject(part_handle(body_defs.kinds(), target, key)),
        _ => Value::Null,
    };
    let locals: Vec<Value> = def
        .locals()
        .iter()
        .map(|local| match local.source {
            LocalSource::Role(0) => sim.subject_value(Subject::Entity(who)),
            LocalSource::Role(_) => target_value.clone(),
            LocalSource::Param(_) => match local.name.as_str() {
                "target" => target_value.clone(),
                "part" => part_value.clone(),
                "effort" => Value::Real(effort),
                _ => Value::Null,
            },
            LocalSource::Var(position) => def
                .vars()
                .get(usize::from(position))
                .and_then(|var| var.default.as_ref())
                .and_then(|default| default.constant().cloned())
                .unwrap_or(Value::Null),
            LocalSource::Window => Value::Null,
        })
        .collect();
    let bound = Bound {
        subjects: vec![
            sim.subject_value(Subject::Entity(who)),
            weapon,
            limb,
            target_value,
        ],
        locals,
        fields: Vec::new(),
    };
    // A host with no stream: nothing is drawn (invariant C5).
    let mut host = sim.host(bound);
    let mut runner = Runner::new();
    let mut eval = |expr: &clash_data::Expr<clash_data::expr::Number>| {
        expr.eval(&mut runner.on(&defs, &mut host))
    };
    let energy = eval(&attack_def.energy).unwrap_or(Real::ZERO).max(Real::ZERO);
    let push = eval(&attack_def.push)
        .unwrap_or(Real::ZERO)
        .clamp(Real::ZERO, Real::ONE);
    let reach = attack_def
        .reach
        .as_ref()
        .and_then(&mut eval)
        .unwrap_or(found.reach);
    let aim = attack_def.aim.as_ref().map(|aim| {
        let by = eval(&aim.by).unwrap_or(Real::ZERO);
        let against = eval(&aim.against).unwrap_or(Real::ZERO);
        (aim.contest.id(), by, against)
    });
    drop(host);
    let effort = effort.clamp(Real::ZERO, Real::ONE);
    let put_in = energy * effort;
    let harm = put_in * (Real::ONE - push);
    let (odds, odds_near) = match aim {
        Some((contest, by, against)) => {
            let contest = defs
                .try_registry::<super::ContestDef>()
                .and_then(|registry| registry.try_get(contest));
            match contest {
                Some(contest) => (
                    contest.odds(by, against),
                    odds_within(contest, by, against, contest.near),
                ),
                None => (Real::ONE, Real::ONE),
            }
        }
        None => (Real::ONE, Real::ONE),
    };
    let push_speed = super::push::push_speed(
        put_in * push,
        Real::ZERO,
        super::push::mass_of(sim, target),
    );
    let stagger_at = setting(sim, STAGGER_AT_SETTING, DEFAULT_STAGGER_AT);
    let launch_at = setting(sim, LAUNCH_AT_SETTING, DEFAULT_LAUNCH_AT);
    Some(Preview {
        in_reach: super::strike::gap(sim, who, target).is_some_and(|gap| gap <= reach),
        odds,
        odds_near,
        energy: harm,
        intensity: super::strike::intensity_of(&found, harm),
        push_speed,
        staggers: push_speed >= stagger_at && push_speed < launch_at,
        launches: push_speed >= launch_at,
    })
}

/// The body of the function `strike_preview`.
fn preview_fn(sim: &Sim, args: &[Value]) -> Value {
    let entity = |value: Option<&Value>| match value.and_then(|value| sim.subject_of(value)) {
        Some(Subject::Entity(id)) => Some(id),
        _ => None,
    };
    let (Some(who), Some(target)) = (entity(args.first()), entity(args.get(2))) else {
        return Value::Null;
    };
    let Some(action) = args
        .get(1)
        .and_then(|value| crate::world_verbs::def_name_of(sim, value))
        .and_then(|name| sim.defs().lookup::<ActionDef>(&name))
    else {
        return Value::Null;
    };
    let part = args
        .get(3)
        .and_then(Value::as_subject)
        .and_then(|handle| part_of_handle(&sim.components().body, handle))
        .filter(|(owner, _)| *owner == target)
        .map(|(_, key)| key);
    let effort = args
        .get(4)
        .and_then(Value::as_number)
        .unwrap_or(Real::ONE);
    let attack = args.get(5).and_then(Value::as_str);
    strike_preview(sim, who, action, attack, target, part, effort).map_or(Value::Null, |preview| {
        Value::map([
            ("in_reach", Value::Bool(preview.in_reach)),
            ("odds", Value::Real(preview.odds)),
            ("odds_near", Value::Real(preview.odds_near)),
            ("energy", Value::Real(preview.energy)),
            ("intensity", Value::Real(preview.intensity)),
            ("push_speed", Value::Real(preview.push_speed)),
            ("staggers", Value::Bool(preview.staggers)),
            ("launches", Value::Bool(preview.launches)),
        ])
    })
}

/// The function `strike_preview` (07 §13).
pub fn functions() -> Vec<FunctionDef> {
    vec![
        FunctionDef::new(
            "strike_preview",
            "map<any>?",
            super::GROUP_COMBAT,
            "What a strike of an attack of an action would likely do, worked out without drawing anything or changing anything (07 §13, invariant C5): `in_reach` (bool), `odds` (real, the chance it lands on the part aimed at), `odds_near` (real, on it or a part beside it), `energy` (real, joules of harm at the effort, less the push), `intensity` (real, pascals), `push_speed` (real, metres a second), `staggers` and `launches` (bool). Reads the target as it is: not safe for belief. Null for an action with no such attack, or with nothing to strike with.",
            preview_fn,
        )
        .arg("who", "entity", "Who would strike.")
        .arg("action", "any", "The action: its id.")
        .arg("target", "entity", "Whom it would strike.")
        .optional("part", "any", "The part aimed at, of the kind `part`; null for none.")
        .optional("effort", "real", "The effort, 0 to 1. Default 1.")
        .optional("attack", "string", "The attack's id; left out for an action with one."),
    ]
}
