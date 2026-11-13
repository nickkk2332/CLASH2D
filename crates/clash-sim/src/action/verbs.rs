//! The verbs of actions and plans (03 §13): what a rule does to what a
//! character is doing.
//!
//! | Verb | Does | Gives |
//! |------|------|-------|
//! | `start_action { who, action, with? }` | Starts an action, as `Act` does | whether it started |
//! | `enqueue_action { who, action, with? }` | Puts an action at the end of a plan | null |
//! | `cancel_action { who, action?, channel?, tag? }` | Cancels the matching actions in phases that allow it at will | how many |
//! | `interrupt { who, action?, channel?, tag? }` | Interrupts the matching actions not in an `uninterruptible` phase | how many |
//! | `clear_plan { who }` | Empties a plan | null |
//! | `interrupt_plan { who }` | Empties a plan and asks what to do next | null |
//! | `disable_channel { who, channel, count? }` | Takes slots away, interrupting what used them | null |
//! | `enable_channel { who, channel, count? }` | Gives disabled slots back | null |
//! | `wait { who, time }` | Occupies an entity doing nothing | null |
//!
//! An action that cannot start is not a fault: `start_action` gives
//! false and `action_refused` says why. The verbs of windows, reactions
//! and roles are packages T5 and T6's.

use clash_core::{Dur, EntityId, Real};
use clash_expr::{Type, Value, VerbSpec};
use clash_world::Cause;

use super::event::ActEvent;
use super::ids::{ActionDefId, ChannelId};
use super::run::action_def;
use super::state::Params;
use crate::Sim;
use crate::decision::{DecisionKind, FreeReason};
use crate::entity::Control;
use crate::event::{EventBody, Where};
use crate::kernel_verbs::{arg, bad_argument, entity_arg};
use crate::later::ActionId;
use crate::reason::Reason;
use crate::rule::VerbCall;
use crate::verb::VerbDef;

/// An argument that names a definition of a type, by its number.
fn def_arg(sim: &Sim, call: &VerbCall<'_>, at: usize, ty: &str) -> Option<u32> {
    match arg(call, at) {
        Value::Id(id) if sim.defs().expr_registry().def_type_id(ty) == Some(id.ty) => {
            Some(id.index)
        }
        _ => None,
    }
}

/// The action an argument names, which must be one.
fn action_arg(
    sim: &Sim,
    call: &VerbCall<'_>,
    at: usize,
    verb: &str,
) -> Result<ActionDefId, Reason> {
    def_arg(sim, call, at, "action")
        .map(ActionDefId::new)
        .ok_or_else(|| bad_argument(verb, "action", "`action` must be the id of an action"))
}

/// The parameters a map gives, by name.
fn params_arg(call: &VerbCall<'_>, at: usize) -> Params {
    arg(call, at).as_map().map_or_else(Vec::new, |map| {
        map.iter()
            .map(|(key, value)| (key.to_string(), value.to_stored()))
            .collect()
    })
}

/// A count of slots: the argument, or one.
fn count_arg(call: &VerbCall<'_>, at: usize, verb: &str) -> Result<i32, Reason> {
    match arg(call, at) {
        Value::Null => Ok(1),
        Value::Int(count) if *count >= 0 => Ok(i32::try_from(*count).unwrap_or(i32::MAX)),
        _ => Err(bad_argument(
            verb,
            "count",
            "`count` must be a whole number of slots, zero or more",
        )),
    }
}

/// The channel an argument names, which must be one.
fn channel_arg(sim: &Sim, call: &VerbCall<'_>, at: usize, verb: &str) -> Result<ChannelId, Reason> {
    def_arg(sim, call, at, "channel")
        .and_then(|index| u8::try_from(index).ok())
        .map(ChannelId::new)
        .ok_or_else(|| bad_argument(verb, "channel", "`channel` must be the id of a channel"))
}

/// The running actions of an entity that match what a verb names: a
/// definition, a channel held, a tag of the definition. All of them if
/// it names none.
fn matching(sim: &Sim, who: EntityId, call: &VerbCall<'_>) -> Vec<ActionId> {
    let action = def_arg(sim, call, 1, "action").map(ActionDefId::new);
    let channel = def_arg(sim, call, 2, "channel")
        .and_then(|index| u8::try_from(index).ok())
        .map(ChannelId::new);
    let tag = arg(call, 3)
        .as_str()
        .map(|tag| sim.defs().tags().lookup(tag));
    sim.actions_of(who)
        .iter()
        .copied()
        .filter(|id| {
            let Some(running) = sim.actions.get(*id) else {
                return false;
            };
            action.is_none_or(|action| running.def == action)
                && channel
                    .is_none_or(|channel| running.held.iter().any(|(have, _)| *have == channel))
                && tag.is_none_or(|tag| {
                    tag.is_some_and(|tag| {
                        action_def(sim.defs(), running.def)
                            .is_some_and(|def| def.tags.contains(tag))
                    })
                })
        })
        .collect()
}

fn start_action(sim: &mut Sim, call: &VerbCall<'_>, cause: Cause) -> Result<Value, Reason> {
    let who = entity_arg(sim, call, 0, "start_action", "who")?;
    let action = action_arg(sim, call, 1, "start_action")?;
    let params = params_arg(call, 2);
    match sim.start_action(who, action, &params, cause) {
        Ok(Some(_)) => Ok(Value::Bool(true)),
        Ok(None) => Ok(Value::Bool(false)),
        Err(reason) => {
            let name = super::run::action_name(sim.defs(), action);
            sim.emit(
                Where::Entity(who),
                cause,
                EventBody::Act(ActEvent::ActionRefused {
                    actor: who,
                    action: name,
                    reason,
                }),
            );
            Ok(Value::Bool(false))
        }
    }
}

fn enqueue_action(sim: &mut Sim, call: &VerbCall<'_>, cause: Cause) -> Result<Value, Reason> {
    let who = entity_arg(sim, call, 0, "enqueue_action", "who")?;
    let action = action_arg(sim, call, 1, "enqueue_action")?;
    let params = params_arg(call, 2);
    sim.enqueue_action(who, action, params, cause)?;
    Ok(Value::Null)
}

fn count(n: usize) -> Value {
    Value::Int(i64::try_from(n).unwrap_or(i64::MAX))
}

fn cancel_action(sim: &mut Sim, call: &VerbCall<'_>, cause: Cause) -> Result<Value, Reason> {
    let who = entity_arg(sim, call, 0, "cancel_action", "who")?;
    let mut done = 0;
    for id in matching(sim, who, call) {
        if sim.cancel_action(who, id, true, cause).is_ok() {
            done += 1;
        }
    }
    Ok(count(done))
}

fn interrupt(sim: &mut Sim, call: &VerbCall<'_>, cause: Cause) -> Result<Value, Reason> {
    let who = entity_arg(sim, call, 0, "interrupt", "who")?;
    let mut done = 0;
    for id in matching(sim, who, call) {
        let held_firm = sim
            .actions
            .get(id)
            .and_then(|running| {
                action_def(sim.defs(), running.def)?
                    .phase(running.phase)
                    .map(|phase| phase.uninterruptible)
            })
            .unwrap_or(true);
        if held_firm {
            continue;
        }
        let name = sim
            .actions
            .get(id)
            .map(|running| super::run::action_name(sim.defs(), running.def))
            .unwrap_or_default();
        let reason = Reason::new(super::run::ACTION_INTERRUPTED, "something interrupted it")
            .with("action", name.as_str());
        if sim.interrupt_action(id, reason, cause) {
            done += 1;
        }
    }
    Ok(count(done))
}

fn clear_plan(sim: &mut Sim, call: &VerbCall<'_>, cause: Cause) -> Result<Value, Reason> {
    let who = entity_arg(sim, call, 0, "clear_plan", "who")?;
    sim.clear_plan(who, None, cause);
    Ok(Value::Null)
}

fn interrupt_plan(sim: &mut Sim, call: &VerbCall<'_>, cause: Cause) -> Result<Value, Reason> {
    let who = entity_arg(sim, call, 0, "interrupt_plan", "who")?;
    sim.clear_plan(who, None, cause);
    // A wait is broken off too: what interrupts a plan interrupts
    // waiting for an hour (decision D-871).
    sim.end_wait(who);
    let control = sim
        .truth()
        .entity(who)
        .map_or(Control::None, |record| record.control());
    let why = FreeReason::Interrupted(cause);
    if control == Control::Direct {
        if !sim.decisions().has_free(who) {
            sim.request_decision(who, DecisionKind::Free { why }, cause)?;
        }
    } else {
        sim.emit(
            Where::Entity(who),
            cause,
            EventBody::Act(ActEvent::DecisionNeeded {
                entity: who,
                control,
                why,
            }),
        );
    }
    Ok(Value::Null)
}

fn change_slots(
    sim: &mut Sim,
    call: &VerbCall<'_>,
    cause: Cause,
    verb: &str,
    sign: i32,
) -> Result<Value, Reason> {
    let who = entity_arg(sim, call, 0, verb, "who")?;
    let channel = channel_arg(sim, call, 1, verb)?;
    let by = count_arg(call, 2, verb)?;
    if let Some(record) = sim.entity_mut(who) {
        record.channels.change(channel, by.saturating_mul(sign));
    }
    sim.check_slots(who, cause);
    Ok(Value::Null)
}

fn disable_channel(sim: &mut Sim, call: &VerbCall<'_>, cause: Cause) -> Result<Value, Reason> {
    change_slots(sim, call, cause, "disable_channel", 1)
}

fn enable_channel(sim: &mut Sim, call: &VerbCall<'_>, cause: Cause) -> Result<Value, Reason> {
    change_slots(sim, call, cause, "enable_channel", -1)
}

fn wait(sim: &mut Sim, call: &VerbCall<'_>, cause: Cause) -> Result<Value, Reason> {
    let who = entity_arg(sim, call, 0, "wait", "who")?;
    let secs = arg(call, 1).as_number().ok_or_else(|| {
        bad_argument("wait", "time", "`time` must be a length of time in seconds")
    })?;
    if secs < Real::ZERO {
        return Err(bad_argument(
            "wait",
            "time",
            "a wait cannot last less than no time",
        ));
    }
    sim.wait(who, Dur::from_secs_real(secs), cause)?;
    Ok(Value::Null)
}

/// The arguments that pick actions of an entity, after `who`.
fn picks(spec: VerbSpec, action: &Type, channel: &Type) -> VerbSpec {
    spec.optional_arg("action", action.clone(), "Only actions of this definition.")
        .optional_arg(
            "channel",
            channel.clone(),
            "Only actions holding slots of this channel.",
        )
        .optional_arg(
            "tag",
            Type::Str,
            "Only actions whose definition carries this tag.",
        )
}

/// The verbs of actions and plans. See the [module documentation](self).
pub fn verbs() -> Vec<VerbDef> {
    vec![
        VerbDef {
            name: "start_action",
            build: |registry| {
                let entity = registry.kind_id("entity")?;
                let action = registry.def_type_id("action")?;
                Some(
                    VerbSpec::new("start_action")
                        .arg("who", Type::Subject(entity), "Who acts.")
                        .arg("action", Type::Id(action), "The action.")
                        .optional_arg("with", Type::map(Type::Any), "Its parameters, by name; those left out take their defaults.")
                        .doc("Starts an action now, as the `Act` command does (03 §6.6). Gives whether it started; if it did not, `action_refused` says why."),
                )
            },
            run: start_action,
        },
        VerbDef {
            name: "enqueue_action",
            build: |registry| {
                let entity = registry.kind_id("entity")?;
                let action = registry.def_type_id("action")?;
                Some(
                    VerbSpec::new("enqueue_action")
                        .arg("who", Type::Subject(entity), "Whose plan.")
                        .arg("action", Type::Id(action), "The action.")
                        .optional_arg("with", Type::map(Type::Any), "Its parameters, by name.")
                        .doc("Puts an action at the end of a plan (03 §5.3); it starts when what is before it has and its channels are free."),
                )
            },
            run: enqueue_action,
        },
        VerbDef {
            name: "cancel_action",
            build: |registry| {
                let entity = registry.kind_id("entity")?;
                let action = Type::Id(registry.def_type_id("action")?);
                let channel = Type::Id(registry.def_type_id("channel")?);
                Some(
                    picks(
                        VerbSpec::new("cancel_action").arg("who", Type::Subject(entity), "Whose actions."),
                        &action,
                        &channel,
                    )
                    .doc("Cancels an entity's running actions that match, each in a phase whose `cancel` is true: `on_cancel` runs and `action_cancelled` is raised. A held phase is given up. Gives how many."),
                )
            },
            run: cancel_action,
        },
        VerbDef {
            name: "interrupt",
            build: |registry| {
                let entity = registry.kind_id("entity")?;
                let action = Type::Id(registry.def_type_id("action")?);
                let channel = Type::Id(registry.def_type_id("channel")?);
                Some(
                    picks(
                        VerbSpec::new("interrupt").arg("who", Type::Subject(entity), "Whose actions."),
                        &action,
                        &channel,
                    )
                    .doc("Interrupts an entity's running actions that match, except those in an `uninterruptible` phase: `on_interrupt` runs and `action_interrupted` is raised with `action.interrupted`. Gives how many."),
                )
            },
            run: interrupt,
        },
        VerbDef {
            name: "clear_plan",
            build: |registry| {
                let entity = registry.kind_id("entity")?;
                Some(
                    VerbSpec::new("clear_plan")
                        .arg("who", Type::Subject(entity), "Whose plan.")
                        .doc(
                            "Empties a plan (03 §5.3). Raises `plan_cleared` if it held anything.",
                        ),
                )
            },
            run: clear_plan,
        },
        VerbDef {
            name: "interrupt_plan",
            build: |registry| {
                let entity = registry.kind_id("entity")?;
                Some(
                    VerbSpec::new("interrupt_plan")
                        .arg("who", Type::Subject(entity), "Whose plan.")
                        .doc("Empties a plan, ends a wait in progress, and asks what to do next: a `Free` decision for a directly controlled entity, `decision_needed` for anyone else, whatever is running. What rules use to break off a long wait when something happens (03 §5.3)."),
                )
            },
            run: interrupt_plan,
        },
        VerbDef {
            name: "disable_channel",
            build: |registry| {
                let entity = registry.kind_id("entity")?;
                let channel = registry.def_type_id("channel")?;
                Some(
                    VerbSpec::new("disable_channel")
                        .arg("who", Type::Subject(entity), "Whose slots.")
                        .arg("channel", Type::Id(channel), "Of which channel.")
                        .optional_arg("count", Type::Int, "How many slots; one if left out.")
                        .doc("Disables slots of a channel (03 §6.2). Whatever then holds more slots than are enabled is interrupted, newest first, with `engine:action_channel_disabled`."),
                )
            },
            run: disable_channel,
        },
        VerbDef {
            name: "enable_channel",
            build: |registry| {
                let entity = registry.kind_id("entity")?;
                let channel = registry.def_type_id("channel")?;
                Some(
                    VerbSpec::new("enable_channel")
                        .arg("who", Type::Subject(entity), "Whose slots.")
                        .arg("channel", Type::Id(channel), "Of which channel.")
                        .optional_arg("count", Type::Int, "How many slots; one if left out.")
                        .doc(
                            "Enables slots of a channel that were disabled; never more than were.",
                        ),
                )
            },
            run: enable_channel,
        },
        VerbDef {
            name: "wait",
            build: |registry| {
                let entity = registry.kind_id("entity")?;
                Some(
                    VerbSpec::new("wait")
                        .arg("who", Type::Subject(entity), "Who waits.")
                        .arg("time", Type::Real, "How long, in seconds of its own time: `\"2 s\"`.")
                        .doc("Occupies an entity doing nothing (03 §13). When the time is over it is free, and asked what to do next."),
                )
            },
            run: wait,
        },
    ]
}
