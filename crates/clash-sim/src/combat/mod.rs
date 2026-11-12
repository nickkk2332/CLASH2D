//! Combat (`docs/07-combat.md`): contests, attacks, the strike, aim,
//! delivery and push.
//!
//! Combat adds no timing system and no damage system (decision D-101).
//! An attack is a block on an action (03 §6); the `strike` effect, run
//! from one of the action's phases, resolves it against a target; what
//! reaches the target is a [`Hit`](crate::body::Hit) on its body
//! (06 §7) or a packet on an element of the world (02 §12); and the
//! share set aside to push it moves it, through the world's bodies in
//! flight (02 §15) when it is enough.
//!
//! | Part | Is | Module |
//! |------|----|--------|
//! | [`ContestDef`], [`contest_fn`], [`odds_fn`] | The `contest` definition type and the functions `contest` and `odds` (07 §4.1) | [`contest`] |
//! | [`Attacks`], [`AttackDef`], [`WithDef`], [`AimDef`], [`ArcDef`] | The `attacks` block of an action (07 §3.1) | [`attack`] |
//! | [`Surface`], [`Striker`] | What strikes: a part, an item held, an inline surface (07 §3.2) | [`surface`] |
//! | [`strike`](strike::strike), [`run_strike`], [`StrikeArgs`], [`StrikeTally`] | The strike, in order, with delivery, recoil and passing through (07 §3.4, §6) | [`strike`] |
//! | [`aim`](aim::aim) | Where a strike lands (07 §4.2) | [`aim`] |
//! | [`push`](push::push), [`Push`], [`Pushed`] | What the push share does (07 §7) | [`push`] |
//! | [`COMBAT_EVENTS`], [`COMBAT_REASONS`], [`COMBAT_SETTINGS`] | Events and hooks, refusals, settings | [`events`], [`reason`] |
//!
//! # The `strike` effect
//!
//! `strike` is an effect of the rule compiler, not a verb: it leaves
//! what came of it in the locals of the action whose list runs it
//! (`$landed`, `$absorbed`, `$passed`), which a verb cannot reach, and
//! it resolves an attack of that action, which the compiled effect finds
//! again through the definition its origin names. It is written only in
//! an action's lists, and never in an attack's own `on_land`,
//! `on_miss` and `on_stopped` ([`check_strikes`]).
//!
//! # What is not built here yet
//!
//! Defences (07 §5), clashes (§8), grips and throws (§9), takedowns
//! (§10), projectiles and ranged weapons (§11) and previews (§13) are
//! the later packages of `docs/plans/combat.md`. The strike calls a
//! step for each of the first two and finds none.

pub mod aim;
pub mod attack;
pub mod contest;
pub mod events;
pub mod push;
pub mod reason;
pub mod strike;
pub mod surface;

use clash_core::{Real, Rng};
use clash_data::{Def, DefTypes, ValidateCx};
use clash_expr::{Registry, Value};

pub use attack::{
    AimDef, ArcDef, ArcShape, AttackDef, Attacks, LIMB_SUBJECT, RESULT_LOCALS, TARGET_SUBJECT,
    WEAPON_SUBJECT, WithDef, WithSource,
};
pub use contest::{ContestDef, ContestForm, ContestId};
pub use events::{COMBAT_EVENTS, CombatEvent, event_types};
pub use push::{Push, Pushed};
pub use reason::{COMBAT_REASONS, COMBAT_SETTINGS};
pub use strike::{StrikeArgs, StrikeTally, run_strike};
pub use surface::{Striker, Surface};

use crate::Sim;
use crate::action::ActionDef;
use crate::registry::FunctionDef;
use crate::rule::{Block, EffectKind};

/// The heading combat's functions are listed under in the reference.
const GROUP_COMBAT: &str = "Combat";

/// Registers combat's definition types with the loader, `contest`, and
/// its registry step, which enters its events. Called from the crate's
/// `register_defs`, after the step of living things.
pub fn register_defs(types: &mut DefTypes) {
    types.register::<ContestDef>();
    types.add_registry_step(events::registry_step);
}

/// Enters in a registry the definition types combat's functions name,
/// unless they are there already: `contest`. Called while the engine's
/// registry is built, before its functions.
pub fn register_types(registry: &mut Registry) {
    if registry.def_type_id(ContestDef::TYPE).is_none() {
        let _ = registry.add_def_type(ContestDef::TYPE, ContestDef::DOC);
    }
}

/// The `strike` effects of a list and of the lists inside it, each with
/// the JSON path it is written at.
fn strikes_in<'a>(block: &'a Block, out: &mut Vec<(&'a crate::rule::Strike, &'a str)>) {
    for effect in block.effects() {
        let inner: Vec<&Block> = match &effect.kind {
            EffectKind::Strike(strike) => {
                out.push((strike, &effect.path));
                Vec::new()
            }
            EffectKind::If {
                then, otherwise, ..
            } => vec![then, otherwise],
            EffectKind::First { entries } => entries.iter().map(|(_, body)| body).collect(),
            EffectKind::Switch { cases, default, .. } => cases
                .iter()
                .map(|(_, body)| body)
                .chain([default])
                .collect(),
            EffectKind::ForEach { body, .. }
            | EffectKind::Repeat { body, .. }
            | EffectKind::While { body, .. }
            | EffectKind::Rebind { body, .. } => vec![body],
            EffectKind::Weighted { entries } => entries.iter().map(|(_, body)| body).collect(),
            EffectKind::Timer(timer) => vec![&timer.body],
            EffectKind::Ask(ask) => match &ask.options {
                crate::rule::AskOptions::Listed(options) => {
                    options.iter().map(|option| &option.body).collect()
                }
                crate::rule::AskOptions::Computed(_) => Vec::new(),
            },
            _ => Vec::new(),
        };
        for block in inner {
            strikes_in(block, out);
        }
    }
}

/// Checks the `strike` effects of an action (07 §3.4): each names one of
/// its attacks, or leaves the attack out of an action that has one; and
/// none is written in an attack's own lists, which run while a strike is
/// being resolved.
pub fn check_strikes(action: &ActionDef, cx: &mut ValidateCx<'_>) {
    let mut found = Vec::new();
    for effects in action.effect_lists() {
        if let Some(block) = effects.block() {
            strikes_in(block, &mut found);
        }
    }
    for (strike, path) in &found {
        match &strike.attack {
            Some(name) if action.attacks.find(name).is_none() => {
                let known: Vec<&str> = action
                    .attacks
                    .iter()
                    .map(|attack| attack.id.value.as_str())
                    .collect();
                cx.error(format!(
                    "{path}: `strike` names the attack `{name}`, and this action's attacks are {}",
                    if known.is_empty() {
                        "none: give it an `attacks` block".to_owned()
                    } else {
                        known
                            .iter()
                            .map(|name| format!("`{name}`"))
                            .collect::<Vec<_>>()
                            .join(", ")
                    }
                ));
            }
            None if action.attacks.0.len() != 1 => {
                cx.error(format!(
                    "{path}: `strike` leaves out `attack`, which only an action with one attack may; this one has {}",
                    action.attacks.0.len()
                ));
            }
            _ => {}
        }
    }
    let mut inner = Vec::new();
    for effects in action.attacks.effect_lists() {
        if let Some(block) = effects.block() {
            strikes_in(block, &mut inner);
        }
    }
    for (_, path) in inner {
        cx.error(format!(
            "{path}: an attack's `on_land`, `on_miss` and `on_stopped` run while it is being resolved, and cannot `strike`"
        ));
    }
}

/// The contest a value names, if it names a loaded one.
fn contest_of<'a>(sim: &'a Sim, value: Option<&Value>) -> Option<&'a ContestDef> {
    let id = value?.as_id()?;
    let index = usize::try_from(id.index).ok()?;
    sim.defs()
        .try_registry::<ContestDef>()?
        .try_get(ContestId::from_index(index)?)
}

/// The three arguments of `contest` and `odds`: the contest, `a` and
/// `b`.
fn contest_args<'a>(sim: &'a Sim, args: &[Value]) -> Option<(&'a ContestDef, Real, Real)> {
    let def = contest_of(sim, args.first())?;
    let a = args.get(1)?.as_number()?;
    let b = args.get(2)?.as_number()?;
    Some((def, a, b))
}

/// The body of `contest(contest, a, b)` where a random stream is at
/// hand: a margin, drawn from `rng` (07 §4.1). Null for a contest that
/// is not loaded or a value that is null.
pub fn contest_fn(sim: &Sim, args: &[Value], rng: &mut Rng) -> Value {
    contest_args(sim, args).map_or(Value::Null, |(def, a, b)| {
        Value::Real(def.margin(a, b, rng))
    })
}

/// The body of `odds(contest, a, b)`: the chance that a margin of the
/// contest is at or above zero (07 §4.1). It draws nothing. Null for a
/// contest that is not loaded or a value that is null.
pub fn odds_fn(sim: &Sim, args: &[Value]) -> Value {
    contest_args(sim, args).map_or(Value::Null, |(def, a, b)| Value::Real(def.odds(a, b)))
}

/// The functions combat offers to expressions (07 §16).
pub fn functions() -> Vec<FunctionDef> {
    vec![
        FunctionDef::new(
            "contest",
            "real?",
            GROUP_COMBAT,
            "A margin of a contest (07 §4.1): the scaled difference, or log-ratio, of `a` and `b`, plus logistic noise of the contest's spread drawn from the rule stream. At or above zero, `a` won. Refused in a pure context. Null for a value that is null.",
            // Where nothing may be drawn there is no margin.
            |_, _| Value::Null,
        )
        .arg("contest", "id<contest>", "The contest.")
        .arg("a", "real", "The value of the side the margin favours when it is positive.")
        .arg("b", "real", "The value of the other side.")
        .safe()
        .drawing(contest_fn),
        FunctionDef::new(
            "odds",
            "real?",
            GROUP_COMBAT,
            "The chance that a margin of a contest is at or above zero (07 §4.1), from 0 to 1. Pure: it draws nothing. Null for a value that is null.",
            odds_fn,
        )
        .arg("contest", "id<contest>", "The contest.")
        .arg("a", "real", "The value of the side the margin favours.")
        .arg("b", "real", "The value of the other side.")
        .safe(),
    ]
}
