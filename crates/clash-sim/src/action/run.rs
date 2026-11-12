//! The action runtime (03 §6): starting an action, walking its phases on
//! the actor's clock, ending it, and the slots it holds.
//!
//! # Starting
//!
//! [`Sim::start_action`] does what the [module above](super) lists, in
//! this order, and refuses with the first reason that applies, changing
//! nothing:
//!
//! 1. The actor is known and in the world, and the definition is loaded.
//! 2. The locals are made: the roles from the parameters given under
//!    their names (the first role is the actor), each parameter from the
//!    value given, held to its type and, for a list, to its bounds, or
//!    else its `default`, worked out in the actor's mind in order; each
//!    variable from its constant.
//! 3. `footing`, then `requires` (with `fail_text` as the sentence).
//! 4. Slots: for each channel the action uses, enough enabled slots
//!    ([`CHANNEL_LACKING`](super::reason::CHANNEL_LACKING)) and enough of
//!    them free ([`CHANNEL_BUSY`](super::reason::CHANNEL_BUSY)). Slots
//!    held by an action in a phase whose `cancel` allows it for this
//!    action's tags count as free: that action is cancelled to make room.
//! 5. The hook `action_starting`, if anything listens: a handler may
//!    change the parameters or cancel. From here on the state has
//!    changed, so a refusal is raised as `action_refused` rather than
//!    returned.
//! 6. The slots are claimed, `action_started` is raised, and
//!    `requires_at_start` is tested against truth: if it fails the action
//!    is interrupted at once. Then `on_start` runs and the first phase is
//!    entered.
//!
//! # Phases
//!
//! Entering a phase works out its duration (its `time` over the tempo,
//! or a held phase's `max`), raises `phase_timing` if anything listens,
//! schedules one wake-up that far ahead on the actor's clock, raises
//! `phase_entered` (and `window_opened` for a phase with a window), runs
//! the phase's `do` and schedules its first tick. A phase of no length
//! ends at once, in the same tick; more than `rules.max_transitions`
//! phases entered in one tick interrupt the action. Leaving a phase runs
//! its `on_exit`, raises `phase_exited` and goes where the first entry
//! of `next` that holds says, else to the following phase, else out of
//! the action.
//!
//! # Ending
//!
//! | How | Runs | Raises |
//! |-----|------|--------|
//! | The last phase ends | `on_finish` | `action_finished` |
//! | `Cancel`, `cancel_action` in a phase that allows it | `on_cancel` | `action_cancelled` |
//! | `interrupt`, a lost slot, `requires_at_start`, the transition limit | `on_interrupt` | `action_interrupted` |
//! | The actor leaves the world or is removed | `on_abandon` | `action_abandoned` |
//!
//! In every case the slots are released and the wake-ups cancelled before
//! the event is raised. When a foreground action ends, other than by
//! being abandoned, the actor's plan is consulted and, if nothing is
//! started, the actor is free ([`plan`](super::plan)).
//!
//! # Wake-ups
//!
//! Every wake-up of the runtime is in the `Action` domain: the end of a
//! phase and a tick carry the action's number, a wait and an idle the
//! entity's.

use std::sync::Arc;

use clash_core::{Dur, EntityId, Real};
use clash_data::{Defs, Runner};
use clash_expr::{Context, Type, Value};
use clash_time::{Wake, WakeDomain, WakeId};
use clash_world::{Cause, CauseKind, query::Footing};

use super::def::{ActionDef, Effects, LocalSource, Next, PhaseTime, Slots};
use super::event::{ACTION_STARTING, ActEvent, PHASE_TIMING, duration_of};
use super::ids::{ActionDefId, ChannelId, PhaseIndex};
use super::reason as why;
use super::state::{Params, RunningAction};
use crate::Sim;
use crate::body::Body;
use crate::bus::{EventPayload, FieldValue};
use crate::decision::FreeReason;
use crate::event::{EventBody, Subject, Where};
use crate::host::Bound;
use crate::later::ActionId;
use crate::reason::{Reason, ReasonInfo, codes};

/// How many slots of each channel an entity with no body has (03 §6.2):
/// one of every channel, so that a device or a test creature can act.
pub const DEFAULT_SLOTS: u16 = 1;

/// The setting that limits phase changes without time passing
/// (03 §13).
pub const MAX_TRANSITIONS_SETTING: &str = "rules.max_transitions";

/// The default of [`MAX_TRANSITIONS_SETTING`].
pub const DEFAULT_MAX_TRANSITIONS: i64 = 1_000;

/// What the `arg` of an `Action` wake-up says it is.
pub(crate) const WAKE_PHASE: u64 = 0;
pub(crate) const WAKE_TICK: u64 = 1;
pub(crate) const WAKE_WAIT: u64 = 2;

/// No action definition has the id a command or an effect gave.
pub const ACTION_UNKNOWN: &str = "action.unknown";
/// An action named to be cancelled is not running, or is not the
/// entity's.
pub const ACTION_NOT_RUNNING: &str = "action.not_running";
/// The `interrupt` effect ended an action.
pub const ACTION_INTERRUPTED: &str = "action.interrupted";
/// A duration, a tempo or an interval came out as nothing a phase can
/// use.
pub const ACTION_BAD_TIME: &str = "action.bad_time";
/// A command addressed an entity that nobody controls.
pub const COMMAND_NOT_CONTROLLED: &str = "command.not_controlled";
/// A command carried a value it cannot use.
pub const COMMAND_BAD_ARGUMENT: &str = "command.bad_argument";

/// The reasons of the action runtime and of the commands that act,
/// beside the 23 of [`reason`](super::reason), for the generated
/// reference.
pub const RUNTIME_REASONS: &[ReasonInfo] = &[
    ReasonInfo {
        code: ACTION_UNKNOWN,
        doc: "No action definition has the id that was given. Values: `action`.",
    },
    ReasonInfo {
        code: ACTION_NOT_RUNNING,
        doc: "`Cancel` named an action that is not running, or is not the entity's. Values: `entity`, `action`.",
    },
    ReasonInfo {
        code: ACTION_INTERRUPTED,
        doc: "The `interrupt` effect ended the action. The reason `action_interrupted` carries. Values: `action`.",
    },
    ReasonInfo {
        code: ACTION_BAD_TIME,
        doc: "An action's tempo came out at zero or below, or null; a phase's `time` or `max` below zero or null; or a tick's `every` at zero or below, or null. A fault: the tempo counts as 1, the duration as none, and the tick does not repeat. Values: `action`, `phase`.",
    },
    ReasonInfo {
        code: COMMAND_NOT_CONTROLLED,
        doc: "A command that only a controller gives (`Act`, `Wait`, `Cancel`, `ClearPlan`) addressed an entity whose control is `none` (03 §5.1). Values: `entity`.",
    },
    ReasonInfo {
        code: COMMAND_BAD_ARGUMENT,
        doc: "A command carried a value it cannot use: a wait of less than no time. Values: `entity`, `argument`.",
    },
];

/// An action definition by its dense id, if it is loaded.
pub(crate) fn action_def(defs: &Defs, id: ActionDefId) -> Option<&ActionDef> {
    defs.try_registry::<ActionDef>()?.try_get(id)
}

/// The qualified id of an action definition.
pub(crate) fn action_name(defs: &Defs, id: ActionDefId) -> String {
    defs.try_registry::<ActionDef>()
        .filter(|registry| id.index() < registry.len())
        .map_or_else(|| id.to_string(), |registry| registry.name(id).to_owned())
}

/// What a value is, said for a reason.
fn plural_value(values: &[(&str, Value)], name: &str) -> Value {
    values
        .iter()
        .find(|(have, _)| *have == name)
        .map_or(Value::Null, |(_, value)| value.clone())
}

/// What a start found it can do, before it changes anything.
pub(crate) struct Prepared {
    def: ActionDefId,
    actor: EntityId,
    locals: Vec<Value>,
    /// The slots to claim, by channel.
    claim: Vec<(ChannelId, u16)>,
    /// The actions to cancel to make room.
    displace: Vec<ActionId>,
}

/// How entering a phase went.
enum Entered {
    /// It lasts: a wake-up will end it, or a release.
    Lasts,
    /// It has no length: it ends now.
    Instant,
    /// The action is no longer running.
    Gone,
}

impl Sim {
    // ---- reading ----------------------------------------------------------

    /// How many slots of a channel an entity has, enabled or not: what
    /// its body supplies, or [`DEFAULT_SLOTS`] if it has no body (03 §6.2).
    pub fn slots_of(&self, entity: EntityId, channel: ChannelId) -> u16 {
        self.components
            .get::<Body>(entity)
            .map_or(DEFAULT_SLOTS, |body| body.slots_of(channel))
    }

    /// How many slots of a channel an entity has that are not disabled.
    pub fn enabled_slots(&self, entity: EntityId, channel: ChannelId) -> u16 {
        let disabled = self
            .entities
            .get(entity)
            .map_or(0, |record| record.channels.disabled(channel));
        self.slots_of(entity, channel).saturating_sub(disabled)
    }

    /// How many slots of a channel an entity's running actions hold.
    pub fn held_slots(&self, entity: EntityId, channel: ChannelId) -> u16 {
        self.holders(entity, channel)
            .iter()
            .map(|(_, count)| *count)
            .fold(0u16, u16::saturating_add)
    }

    /// The running actions of an entity that hold slots of a channel,
    /// oldest first, with how many each holds.
    fn holders(&self, entity: EntityId, channel: ChannelId) -> Vec<(ActionId, u16)> {
        let Some(record) = self.entities.get(entity) else {
            return Vec::new();
        };
        record
            .actions
            .iter()
            .filter_map(|id| {
                let action = self.actions.get(*id)?;
                let held = action
                    .held
                    .iter()
                    .find(|(have, _)| *have == channel)
                    .map(|(_, count)| *count)?;
                Some((*id, held))
            })
            .collect()
    }

    /// An entity's running actions, oldest first.
    pub fn actions_of(&self, entity: EntityId) -> &[ActionId] {
        self.entities
            .get(entity)
            .map_or(&[], |record| record.actions.as_slice())
    }

    /// Whether an entity has a foreground action running (03 §6.1).
    pub fn is_acting(&self, entity: EntityId) -> bool {
        let defs = &self.defs;
        self.actions_of(entity).iter().any(|id| {
            self.actions
                .get(*id)
                .and_then(|action| action_def(defs, action.def))
                .is_some_and(|def| def.foreground)
        })
    }

    /// The value of a definition's id, as expressions hold one.
    fn def_value(&self, type_name: &str, index: u32) -> Value {
        self.defs
            .expr_registry()
            .def_type_id(type_name)
            .map_or(Value::Null, |ty| {
                Value::Id(clash_expr::DefRef::new(ty, index))
            })
    }

    /// The value an entity is bound as in expressions.
    fn actor_value(&self, actor: EntityId) -> Value {
        self.subject_value(Subject::Entity(actor))
    }

    /// The locals of a running action as expressions read them.
    fn locals_of(&self, id: ActionId) -> Vec<Value> {
        self.actions
            .get(id)
            .map(|action| action.locals.iter().map(Value::from).collect())
            .unwrap_or_default()
    }

    /// What an action's doings are caused by: its actor, acting.
    fn action_cause(actor: EntityId) -> Cause {
        Cause {
            actor: Some(actor),
            instigator: Some(actor),
            kind: CauseKind::Action,
        }
    }

    /// Evaluates against truth for an action, drawing from the rule
    /// stream; run-time warnings are reported.
    fn action_eval<T>(
        &mut self,
        actor: EntityId,
        locals: &[Value],
        work: impl FnOnce(&Defs, &mut crate::SimHost<'_>) -> T,
    ) -> T {
        let defs = Arc::clone(&self.defs);
        let bound = Bound {
            subjects: vec![self.actor_value(actor)],
            locals: locals.to_vec(),
            fields: Vec::new(),
        };
        let (result, warnings) = self.with_random(bound, |host| {
            let result = work(&defs, host);
            (result, host.take_warnings())
        });
        for warning in warnings {
            self.report_warning(warning);
        }
        result
    }

    /// A reason of the action system (03 §13), with its sentence
    /// rendered for the actor, who is told it, and its values.
    pub(crate) fn refusal(
        &self,
        reason: &why::Reason,
        actor: EntityId,
        values: &[(&str, Value)],
        sentence: Option<String>,
    ) -> Reason {
        let message = sentence.unwrap_or_else(|| self.sentence(reason, actor, values));
        let mut out = Reason::new(reason.code, message);
        for (name, value) in values {
            out = out.with(*name, value.to_stored());
        }
        out
    }

    /// Renders the sentence of a reason of the action system for the
    /// one it is about.
    fn sentence(&self, reason: &why::Reason, actor: EntityId, values: &[(&str, Value)]) -> String {
        let registry = self.defs.expr_registry();
        let Some(entity) = registry.kind_id("entity") else {
            return reason.doc.to_owned();
        };
        let mut context = Context::new().subject("self", Type::Subject(entity));
        let mut bound = Bound::new().subject(self.actor_value(actor));
        for value in reason.values {
            let ty = registry.parse_type(value.ty).unwrap_or(Type::Any);
            context = context.local(value.name, ty);
            bound = bound.local(plural_value(values, value.name));
        }
        let Some(text) = clash_expr::compile_template(reason.text, registry, &context).value else {
            return reason.doc.to_owned();
        };
        let mut host = self.host_for_reader(bound, actor);
        crate::reader::capitalised(&Runner::new().on(&self.defs, &mut host).text(&text))
    }

    // ---- starting -----------------------------------------------------------

    /// Starts an action (03 §6.6): see the [module documentation](self).
    ///
    /// `Err` is a refusal before anything changed. `Ok(None)` is an
    /// action a handler of `action_starting` refused: `action_refused`
    /// was raised. `Ok(Some(id))` is an action that started; it may
    /// already have ended, in a phase of no length or by its
    /// `requires_at_start`.
    pub fn start_action(
        &mut self,
        actor: EntityId,
        action: ActionDefId,
        params: &Params,
        cause: Cause,
    ) -> Result<Option<ActionId>, Reason> {
        // A default that draws at random draws from the rule stream; a
        // refusal puts the stream back, so that it leaves no trace.
        let streams = self.rng.clone();
        let prepared = match self.prepare(actor, action, params, true) {
            Ok(prepared) => prepared,
            Err(reason) => {
                self.rng = streams;
                return Err(reason);
            }
        };
        Ok(self.begin(prepared, cause).ok())
    }

    /// Works out whether an action can start and what it would claim,
    /// changing nothing but the rule stream. `displace` says whether
    /// actions in phases that allow it may be cancelled to make room.
    pub(crate) fn prepare(
        &mut self,
        actor: EntityId,
        id: ActionDefId,
        params: &Params,
        displace: bool,
    ) -> Result<Prepared, Reason> {
        let defs = Arc::clone(&self.defs);
        let def = action_def(&defs, id).ok_or_else(|| {
            Reason::new(
                ACTION_UNKNOWN,
                format!("there is no action definition {id}"),
            )
            .with("action", i64::from(id.raw()))
        })?;
        let action_id = self.def_value(<ActionDef as clash_data::Def>::TYPE, id.raw());
        let record = self.known(actor)?;
        let place = record.place();
        if self.entities.location(actor).is_none() {
            return Err(Reason::new(
                codes::ENTITY_STORED,
                format!("entity {actor} is not in the world, so it cannot act"),
            )
            .with("entity", actor.to_string()));
        }
        let refuse = |sim: &Sim, reason: &why::Reason, values: &[(&str, Value)]| {
            let mut all = vec![("action", action_id.clone())];
            all.extend(values.iter().cloned());
            sim.refusal(reason, actor, &all, None)
        };

        // The locals, in slot order.
        let mut locals: Vec<Value> = Vec::with_capacity(def.locals().len());
        let given = |name: &str| {
            params
                .iter()
                .find(|(have, _)| have == name)
                .map(|(_, value)| Value::from(value))
        };
        for (name, _) in params {
            let known = def.local_slot(name).is_some_and(|slot| {
                def.locals().get(usize::from(slot)).is_some_and(|local| {
                    matches!(local.source, LocalSource::Param(_) | LocalSource::Role(_))
                })
            });
            if !known {
                return Err(refuse(
                    self,
                    &why::PARAMETER_INVALID,
                    &[
                        ("param", Value::from(name.as_str())),
                        ("expected", Value::from("a parameter of the action")),
                    ],
                ));
            }
        }
        for local in def.locals() {
            let value = match local.source {
                LocalSource::Window => Value::Null,
                LocalSource::Role(0) => self.actor_value(actor),
                LocalSource::Role(_) => match given(&local.name) {
                    Some(value @ Value::Subject(_)) => value,
                    _ => {
                        return Err(refuse(
                            self,
                            &why::ROLE_UNFILLED,
                            &[("role", Value::from(local.name.as_str()))],
                        ));
                    }
                },
                LocalSource::Param(position) => {
                    let Some(param) = def.params().get(usize::from(position)) else {
                        continue;
                    };
                    let value = match given(&local.name) {
                        Some(value) => value,
                        None => match &param.default {
                            Some(default) => self.action_eval(actor, &locals, |defs, host| {
                                default.eval_on(defs, host).unwrap_or_default()
                            }),
                            None => {
                                return Err(refuse(
                                    self,
                                    &why::PARAMETER_MISSING,
                                    &[("param", Value::from(local.name.as_str()))],
                                ));
                            }
                        },
                    };
                    self.check_param(&param.written_type.value, param.ty(), param.list(), &value)
                        .map_err(|(reason, extra)| {
                            let mut values = vec![("param", Value::from(local.name.as_str()))];
                            values.extend(extra);
                            refuse(self, reason, &values)
                        })?;
                    value
                }
                LocalSource::Var(position) => def
                    .vars()
                    .get(usize::from(position))
                    .and_then(|var| var.default.as_ref())
                    .and_then(|default| default.constant().cloned())
                    .unwrap_or(Value::Null),
            };
            locals.push(value);
        }

        if def.footing && !self.has_footing(place) {
            return Err(refuse(self, &why::NO_FOOTING, &[]));
        }
        let holds = {
            let bound = Bound {
                subjects: vec![self.actor_value(actor)],
                locals: locals.clone(),
                fields: Vec::new(),
            };
            let mut host = self.host(bound);
            def.requires.holds_on(&defs, &mut host)
        };
        if !holds {
            let sentence = def.fail_text.as_ref().map(|text| {
                let bound = Bound {
                    subjects: vec![self.actor_value(actor)],
                    locals: locals.clone(),
                    fields: Vec::new(),
                };
                let mut host = self.host_for_reader(bound, actor);
                crate::reader::capitalised(&text.render_on(&defs, &mut host))
            });
            let mut reason = self.refusal(
                &why::REQUIRES,
                actor,
                &[("action", action_id.clone())],
                sentence,
            );
            reason.code = why::REQUIRES.code.to_owned();
            return Err(reason);
        }

        // Slots, for an ordinary action; for a joint one, the first
        // role's on the actor (the others are package T6's).
        let uses = def.roles().first().map_or(def.uses(), |role| &role.uses);
        let mut claim = Vec::new();
        let mut displaced: Vec<ActionId> = Vec::new();
        for need in uses.needs() {
            let channel = need.channel;
            let enabled = self.enabled_slots(actor, channel);
            let held = self.held_slots(actor, channel);
            let needed = match need.slots {
                Slots::Count(count) => count,
                Slots::All => enabled.max(1),
            };
            let channel_values = |sim: &Sim| {
                let name = sim.channel_name(channel);
                vec![
                    (
                        "channel",
                        sim.def_value(
                            <super::def::ChannelDef as clash_data::Def>::TYPE,
                            u32::from(channel.raw()),
                        ),
                    ),
                    ("channel_name", Value::from(name.as_str())),
                ]
            };
            if enabled < needed {
                let mut values = channel_values(self);
                values.push(("needed", Value::Int(i64::from(needed))));
                values.push(("enabled", Value::Int(i64::from(enabled))));
                return Err(refuse(self, &why::CHANNEL_LACKING, &values));
            }
            let mut free = enabled.saturating_sub(held);
            if free < needed && displace {
                // Holders in a phase that may be broken off for this
                // action make room, oldest first.
                for (holder, count) in self.holders(actor, channel) {
                    if free >= needed {
                        break;
                    }
                    if displaced.contains(&holder) {
                        continue;
                    }
                    if self.may_cancel_for(holder, &def.tags) {
                        displaced.push(holder);
                        free = free.saturating_add(count);
                    }
                }
            }
            if free < needed {
                let mut values = channel_values(self);
                values.push(("needed", Value::Int(i64::from(needed))));
                values.push(("free", Value::Int(i64::from(free))));
                let holders: Vec<Value> = self
                    .holders(actor, channel)
                    .iter()
                    .filter_map(|(holder, _)| self.actions.get(*holder))
                    .map(|action| Value::from(action_name(&defs, action.def).as_str()))
                    .collect();
                values.push(("holders", Value::list(holders)));
                return Err(refuse(self, &why::CHANNEL_BUSY, &values));
            }
            claim.push((channel, needed));
        }
        Ok(Prepared {
            def: id,
            actor,
            locals,
            claim,
            displace: displaced,
        })
    }

    /// Whether a value fits a parameter: its type and, for a list, its
    /// bounds. The reason and the values it carries if not.
    #[allow(clippy::type_complexity)]
    fn check_param(
        &self,
        written: &str,
        ty: &Type,
        bounds: Option<super::def::ListBounds>,
        value: &Value,
    ) -> Result<(), (&'static why::Reason, Vec<(&'static str, Value)>)> {
        let expected = || ("expected", Value::from(written));
        if value.is_null() && !ty.may_be_null() && *ty != Type::Any {
            return Err((&why::PARAMETER_INVALID, vec![expected()]));
        }
        if !crate::rule::conforms(value, ty) {
            return Err((&why::PARAMETER_INVALID, vec![expected()]));
        }
        if let (Type::List(inner), Some(items)) = (ty.non_null(), value.as_list())
            && !items.iter().all(|item| crate::rule::conforms(item, inner))
        {
            return Err((&why::PARAMETER_INVALID, vec![expected()]));
        }
        if let (Some(bounds), Some(items)) = (bounds, value.as_list())
            && !bounds.allows(items.len())
        {
            return Err((
                &why::LIST_LENGTH,
                vec![
                    (
                        "count",
                        Value::Int(i64::try_from(items.len()).unwrap_or(i64::MAX)),
                    ),
                    ("min", Value::Int(i64::from(bounds.min))),
                    (
                        "max",
                        bounds
                            .max
                            .map_or(Value::Null, |max| Value::Int(i64::from(max))),
                    ),
                ],
            ));
        }
        Ok(())
    }

    /// Whether something at a place has something under it: a cell's
    /// footing at the base of the cell, or a container (03 §6.1).
    fn has_footing(&self, place: crate::Place) -> bool {
        match place {
            crate::Place::At { loc, alt_mm } => {
                alt_mm == 0 && !matches!(self.cosmos.footing(loc), Ok(Footing::None) | Err(_))
            }
            crate::Place::In(_) => true,
            crate::Place::Stored => false,
        }
    }

    /// What a channel is called where a player is told of it.
    fn channel_name(&self, channel: ChannelId) -> String {
        let Some(registry) = self.defs.try_registry::<super::def::ChannelDef>() else {
            return channel.to_string();
        };
        if channel.index() >= registry.len() {
            return channel.to_string();
        }
        let id = registry.name(channel).to_owned();
        let named = registry.try_get(channel).and_then(|def| def.name.as_ref());
        match named {
            Some(text) => {
                let mut host = self.host(Bound::new());
                text.render_on(&self.defs, &mut host)
            }
            None => id.rsplit(':').next().unwrap_or(&id).to_owned(),
        }
    }

    /// Whether a running action is in a phase that may be broken off to
    /// start an action with these tags.
    fn may_cancel_for(&self, id: ActionId, tags: &clash_data::TagSet) -> bool {
        let Some(action) = self.actions.get(id) else {
            return false;
        };
        action_def(&self.defs, action.def)
            .and_then(|def| def.phase(action.phase))
            .is_some_and(|phase| phase.cancel.allows(tags))
    }

    /// Starts what was prepared: the hook, the claim, the events, the
    /// first phase. See [`start_action`](Sim::start_action). `Err` is a
    /// refusal by a handler of `action_starting`, which has been raised
    /// as `action_refused`.
    pub(crate) fn begin(&mut self, prepared: Prepared, cause: Cause) -> Result<ActionId, Reason> {
        let defs = Arc::clone(&self.defs);
        let Prepared {
            def: def_id,
            actor,
            mut locals,
            claim,
            displace,
        } = prepared;
        let name = action_name(&defs, def_id);
        let refused = |sim: &mut Sim, reason: Reason| {
            sim.emit(
                Where::Entity(actor),
                cause,
                EventBody::Act(ActEvent::ActionRefused {
                    actor,
                    action: name.clone(),
                    reason: reason.clone(),
                }),
            );
            Err(reason)
        };
        let Some(def) = action_def(&defs, def_id) else {
            let reason = Reason::new(
                ACTION_UNKNOWN,
                format!("there is no action definition {def_id}"),
            );
            return refused(self, reason);
        };

        // The hook: a handler may change the parameters, or cancel.
        if self.bus.has_listeners(ACTION_STARTING) {
            let params = Value::map(def.params().iter().map(|param| {
                let value = locals
                    .get(usize::from(param.slot()))
                    .cloned()
                    .unwrap_or_default();
                (param.name.value.clone(), value)
            }));
            let payload = EventPayload::new(ACTION_STARTING)
                .subject("self", Subject::Entity(actor))
                .with("action", FieldValue::text(&name))
                .with("params", self.field_of(&params));
            match self.raise(Where::Entity(actor), cause, payload) {
                Ok(raised) => {
                    if let Some(reason) = raised.cancelled {
                        let reason = if reason.code == codes::EVENTS_CANCELLED {
                            self.refusal(
                                &why::VETOED,
                                actor,
                                &[
                                    ("action", Value::from(name.as_str())),
                                    ("rule", Value::from("")),
                                ],
                                None,
                            )
                        } else {
                            reason
                        };
                        return refused(self, reason);
                    }
                    if let Some(changed) = raised.field("params") {
                        let changed = self.exprs.field_value(changed);
                        if let Some(map) = changed.as_map() {
                            for param in def.params() {
                                let Some(value) = map.get(param.name.value.as_str()) else {
                                    continue;
                                };
                                if let Err((reason, extra)) = self.check_param(
                                    &param.written_type.value,
                                    param.ty(),
                                    param.list(),
                                    value,
                                ) {
                                    let mut values =
                                        vec![("param", Value::from(param.name.value.as_str()))];
                                    values.extend(extra);
                                    let reason = self.refusal(reason, actor, &values, None);
                                    return refused(self, reason);
                                }
                                if let Some(slot) = locals.get_mut(usize::from(param.slot())) {
                                    *slot = value.clone();
                                }
                            }
                        }
                    }
                }
                Err(reason) => self.fault(reason),
            }
        }

        // Room is made, then the slots are claimed.
        for holder in displace {
            self.end_cancelled(holder, cause);
        }
        // A foreground action ends a wait; one in the background does
        // not occupy the actor.
        if def.foreground
            && self.actions.wait_of(actor).is_some()
            && let Some(wait) = self.actions.take_wait(actor)
            && let Some(wake) = wait.wake
        {
            self.cancel_wake(wake);
        }
        let now = self.now();
        let Some(id) = self.actions.insert(RunningAction {
            actor,
            def: def_id,
            phase: PhaseIndex::new(0),
            locals: locals.iter().map(Value::to_stored).collect(),
            held: claim,
            wake: None,
            tick: None,
            every: None,
            entered: now,
            due: None,
            hops: 0,
            hop_tick: now,
        }) else {
            let reason = Reason::new(
                ACTION_UNKNOWN,
                "every number an action can have has been given out",
            );
            return refused(self, reason);
        };
        if let Some(record) = self.entities.get_mut(actor) {
            record.actions.push(id);
        }
        self.emit(
            Where::Entity(actor),
            cause,
            EventBody::Act(ActEvent::ActionStarted {
                actor,
                id,
                action: name.clone(),
            }),
        );

        let at_start = self.action_eval(actor, &locals, |defs, host| {
            def.requires_at_start.holds_on(defs, host)
        });
        if !at_start {
            let reason = self.refusal(
                &why::START_CONDITION,
                actor,
                &[("action", Value::from(name.as_str()))],
                None,
            );
            self.interrupt_action(id, reason, cause);
            return Ok(id);
        }
        if !self.run_list(id, |def| Some(&def.on_start)) {
            return Ok(id);
        }
        self.run_phases(id, Next::Phase(PhaseIndex::new(0)));
        Ok(id)
    }

    // ---- effect lists -------------------------------------------------------

    /// Runs one of a running action's effect lists with its locals, and
    /// keeps what the list did to them. `false` if the action is no
    /// longer running afterwards, or was not.
    fn run_list(&mut self, id: ActionId, list: impl Fn(&ActionDef) -> Option<&Effects>) -> bool {
        let defs = Arc::clone(&self.defs);
        let Some((actor, def_id)) = self
            .actions
            .get(id)
            .map(|action| (action.actor, action.def))
        else {
            return false;
        };
        let Some(effects) = action_def(&defs, def_id).and_then(list) else {
            return self.actions.get(id).is_some();
        };
        self.run_list_of(id, actor, effects);
        self.actions.get(id).is_some()
    }

    /// Runs an effect list of an action that may have ended already, with
    /// the locals it had.
    fn run_list_of(&mut self, id: ActionId, actor: EntityId, effects: &Effects) {
        if effects.is_empty() {
            return;
        }
        let mut locals = self.locals_of(id);
        let subjects = [self.actor_value(actor)];
        self.run_effects(
            effects,
            &subjects,
            &mut locals,
            Some(Subject::Entity(actor)),
            Self::action_cause(actor),
        );
        if let Some(action) = self.actions.get_mut(id) {
            action.locals = locals.iter().map(Value::to_stored).collect();
        }
    }

    // ---- phases ---------------------------------------------------------------

    /// Walks an action's phases from `next` until one lasts, the action
    /// finishes, or it has gone.
    fn run_phases(&mut self, id: ActionId, mut next: Next) {
        let limit = u32::try_from(
            self.setting_int(MAX_TRANSITIONS_SETTING, DEFAULT_MAX_TRANSITIONS)
                .max(1),
        )
        .unwrap_or(u32::MAX);
        loop {
            let phase = match next {
                Next::Finish => {
                    self.finish_action(id);
                    return;
                }
                Next::Phase(phase) => phase,
            };
            let now = self.now();
            let Some(action) = self.actions.get_mut(id) else {
                return;
            };
            if action.hop_tick == now {
                action.hops = action.hops.saturating_add(1);
            } else {
                action.hop_tick = now;
                action.hops = 1;
            }
            if action.hops > limit {
                let actor = action.actor;
                let reason = self.refusal(
                    &why::TRANSITION_LIMIT,
                    actor,
                    &[
                        ("phase", Value::from(self.phase_name(id).as_str())),
                        ("limit", Value::Int(i64::from(limit))),
                    ],
                    None,
                );
                self.interrupt_action(id, reason, Self::action_cause(actor));
                return;
            }
            match self.enter_phase(id, phase) {
                Entered::Lasts | Entered::Gone => return,
                Entered::Instant => match self.leave_phase(id) {
                    Some(to) => next = to,
                    None => return,
                },
            }
        }
    }

    /// The id of the phase a running action is in.
    fn phase_name(&self, id: ActionId) -> String {
        self.actions
            .get(id)
            .and_then(|action| {
                action_def(&self.defs, action.def)?
                    .phase(action.phase)
                    .map(|phase| phase.name().to_owned())
            })
            .unwrap_or_default()
    }

    /// Reports that an action's timing came out as nothing a phase can
    /// use.
    fn bad_time(&mut self, id: ActionId, what: &str) {
        let name = self
            .actions
            .get(id)
            .map(|action| action_name(&self.defs, action.def))
            .unwrap_or_default();
        let phase = self.phase_name(id);
        self.fault(
            Reason::new(
                ACTION_BAD_TIME,
                format!("{what} of action `{name}` in phase `{phase}` cannot be used"),
            )
            .with("action", name.as_str())
            .with("phase", phase.as_str()),
        );
    }

    /// Enters a phase: see the [module documentation](self).
    #[allow(clippy::too_many_lines)]
    fn enter_phase(&mut self, id: ActionId, index: PhaseIndex) -> Entered {
        let defs = Arc::clone(&self.defs);
        let now = self.now();
        let Some(action) = self.actions.get_mut(id) else {
            return Entered::Gone;
        };
        action.phase = index;
        action.entered = now;
        action.due = None;
        action.every = None;
        let (actor, def_id) = (action.actor, action.def);
        let Some(def) = action_def(&defs, def_id) else {
            return Entered::Gone;
        };
        let Some(phase) = def.phase(index) else {
            return Entered::Gone;
        };
        let name = action_name(&defs, def_id);
        let locals = self.locals_of(id);

        // How long: its time over the tempo, or a held phase's `max`.
        let mut duration = match &phase.time {
            PhaseTime::For(time) => {
                let (secs, tempo) = self.action_eval(actor, &locals, |defs, host| {
                    let secs = time.eval_on(defs, host);
                    let tempo = if phase.fixed {
                        Some(Real::ONE)
                    } else {
                        def.tempo.eval_on(defs, host)
                    };
                    (secs, tempo)
                });
                let secs = match secs {
                    Some(secs) if secs >= Real::ZERO => secs,
                    _ => {
                        self.bad_time(id, "the `time`");
                        Real::ZERO
                    }
                };
                let tempo = match tempo {
                    Some(tempo) if tempo > Real::ZERO => tempo,
                    _ => {
                        self.bad_time(id, "the tempo");
                        Real::ONE
                    }
                };
                Some(Dur::from_secs_real(secs / tempo))
            }
            PhaseTime::Hold => match &phase.max {
                Some(max) => {
                    let secs =
                        self.action_eval(actor, &locals, |defs, host| max.eval_on(defs, host));
                    match secs {
                        Some(secs) if secs >= Real::ZERO => Some(Dur::from_secs_real(secs)),
                        _ => {
                            self.bad_time(id, "the `max`");
                            None
                        }
                    }
                }
                None => None,
            },
        };

        // The hook may change it.
        if let Some(planned) = duration
            && self.bus.has_listeners(PHASE_TIMING)
        {
            let payload = EventPayload::new(PHASE_TIMING)
                .subject("self", Subject::Entity(actor))
                .with("action", FieldValue::text(&name))
                .with("id", FieldValue::from(i64::from(id.raw())))
                .with("phase", FieldValue::text(phase.name()))
                .with("duration", FieldValue::from(planned.as_secs_real()));
            match self.raise(Where::Entity(actor), Self::action_cause(actor), payload) {
                Ok(raised) => {
                    if raised.cancelled.is_none()
                        && let Some(changed) = duration_of(raised.field("duration"))
                    {
                        duration = Some(changed);
                    }
                }
                Err(reason) => self.fault(reason),
            }
            if self.actions.get(id).is_none() {
                return Entered::Gone;
            }
        }

        // One wake-up ends it.
        let lasts = duration.is_none_or(|duration| duration > Dur::ZERO);
        let mut wake = None;
        if let Some(duration) = duration.filter(|duration| *duration > Dur::ZERO) {
            match self.schedule_for(
                actor,
                duration,
                Wake::new(WakeDomain::Action, u64::from(id.raw()), WAKE_PHASE),
            ) {
                Ok(scheduled) => wake = Some(scheduled),
                Err(reason) => {
                    self.fault(reason);
                    self.abandon_action(id);
                    return Entered::Gone;
                }
            }
        }
        let due = wake.and_then(|wake| self.timeline.due(wake));
        if let Some(action) = self.actions.get_mut(id) {
            action.wake = wake;
            action.due = due;
        }
        self.emit(
            Where::Entity(actor),
            Self::action_cause(actor),
            EventBody::Act(ActEvent::PhaseEntered {
                actor,
                id,
                action: name.clone(),
                phase: phase.name().to_owned(),
                held: phase.is_held(),
                duration,
                due,
            }),
        );
        if let Some(window) = &phase.window {
            let aimed: Vec<Value> = self.action_eval(actor, &locals, |defs, host| {
                window
                    .aimed_at
                    .iter()
                    .map(|expr| expr.eval_on(defs, host).unwrap_or_default())
                    .collect()
            });
            let aimed_at: Vec<EntityId> = aimed
                .iter()
                .filter_map(|value| self.subject_of(value)?.entity())
                .collect();
            let kind = defs
                .try_registry::<super::def::WindowKindDef>()
                .map_or_else(String::new, |registry| {
                    registry.name(window.kind_id()).to_owned()
                });
            self.emit(
                Where::Entity(actor),
                Self::action_cause(actor),
                EventBody::Act(ActEvent::WindowOpened {
                    actor,
                    id,
                    action: name.clone(),
                    kind,
                    closes: window.closes.name().to_owned(),
                    aimed_at,
                }),
            );
        }
        if !self.run_list(id, |def| def.phase(index).map(|phase| &phase.on_enter)) {
            return Entered::Gone;
        }
        // An effect of the phase may have moved the action on.
        if self
            .actions
            .get(id)
            .is_none_or(|action| action.phase != index || action.entered != now)
        {
            return Entered::Gone;
        }
        if !lasts {
            return Entered::Instant;
        }
        if let Some(tick) = &phase.tick {
            self.schedule_tick(id, actor, &locals, tick);
        }
        Entered::Lasts
    }

    /// Schedules the first tick of a phase.
    fn schedule_tick(
        &mut self,
        id: ActionId,
        actor: EntityId,
        locals: &[Value],
        tick: &super::def::Tick,
    ) {
        let every = self.action_eval(actor, locals, |defs, host| tick.every.eval_on(defs, host));
        let Some(every) = every
            .filter(|every| *every > Real::ZERO)
            .map(Dur::from_secs_real)
            .filter(|every| *every > Dur::ZERO)
        else {
            self.bad_time(id, "the tick's `every`");
            return;
        };
        if let Some(action) = self.actions.get_mut(id) {
            action.every = Some(every);
        }
        self.schedule_next_tick(id, actor);
    }

    fn schedule_next_tick(&mut self, id: ActionId, actor: EntityId) {
        let Some(every) = self.actions.get(id).and_then(|action| action.every) else {
            return;
        };
        let wake = self.schedule_for(
            actor,
            every,
            Wake::new(WakeDomain::Action, u64::from(id.raw()), WAKE_TICK),
        );
        match wake {
            Ok(wake) => {
                if let Some(action) = self.actions.get_mut(id) {
                    action.tick = Some(wake);
                }
            }
            Err(reason) => self.fault(reason),
        }
    }

    /// Leaves the phase an action is in, normally: `on_exit`,
    /// `phase_exited`, and where to go. `None` if the action has gone.
    fn leave_phase(&mut self, id: ActionId) -> Option<Next> {
        let defs = Arc::clone(&self.defs);
        let action = self.actions.get_mut(id)?;
        let (actor, def_id, index) = (action.actor, action.def, action.phase);
        let wakes = [action.wake.take(), action.tick.take()];
        action.every = None;
        for wake in wakes.into_iter().flatten() {
            self.cancel_wake(wake);
        }
        let def = action_def(&defs, def_id)?;
        let phase = def.phase(index)?;
        if !self.run_list(id, |def| def.phase(index).map(|phase| &phase.on_exit)) {
            return None;
        }
        self.emit(
            Where::Entity(actor),
            Self::action_cause(actor),
            EventBody::Act(ActEvent::PhaseExited {
                actor,
                id,
                action: action_name(&defs, def_id),
                phase: phase.name().to_owned(),
            }),
        );
        let locals = self.locals_of(id);
        let chosen = self.action_eval(actor, &locals, |defs, host| {
            phase.next.iter().find_map(|entry| {
                let holds = entry
                    .when
                    .as_ref()
                    .is_none_or(|when| when.holds_on(defs, host));
                holds.then(|| entry.to())
            })
        });
        Some(chosen.unwrap_or_else(|| phase.after()))
    }

    /// A tick of a phase: its effects, then its `until`.
    fn on_tick(&mut self, id: ActionId) {
        let defs = Arc::clone(&self.defs);
        let Some(action) = self.actions.get_mut(id) else {
            return;
        };
        action.tick = None;
        let (actor, def_id, index) = (action.actor, action.def, action.phase);
        let Some(phase) = action_def(&defs, def_id).and_then(|def| def.phase(index)) else {
            return;
        };
        let entered = action.entered;
        if !self.run_list(id, |def| {
            def.phase(index)
                .and_then(|phase| phase.tick.as_ref())
                .map(|tick| &tick.effects)
        }) {
            return;
        }
        if self
            .actions
            .get(id)
            .is_none_or(|action| action.phase != index || action.entered != entered)
        {
            return;
        }
        let locals = self.locals_of(id);
        let done = phase.until.as_ref().is_some_and(|until| {
            self.action_eval(actor, &locals, |defs, host| until.holds_on(defs, host))
        });
        if done {
            if let Some(next) = self.leave_phase(id) {
                self.run_phases(id, next);
            }
        } else {
            self.schedule_next_tick(id, actor);
        }
    }

    // ---- ending -----------------------------------------------------------------

    /// Takes an action out of the table and its actor's list, and
    /// cancels its wake-ups: its slots are free.
    fn release(&mut self, id: ActionId) -> Option<RunningAction> {
        let action = self.actions.remove(id)?;
        for wake in [action.wake, action.tick].into_iter().flatten() {
            self.cancel_wake(wake);
        }
        if let Some(record) = self.entities.get_mut(action.actor) {
            record.actions.retain(|have| *have != id);
        }
        Some(action)
    }

    /// Whether a definition is a foreground one.
    fn foreground(&self, def: ActionDefId) -> bool {
        action_def(&self.defs, def).is_some_and(|def| def.foreground)
    }

    /// The last phase ended: `on_finish`, then the slots are released and
    /// `action_finished` raised.
    fn finish_action(&mut self, id: ActionId) {
        if !self.run_list(id, |def| Some(&def.on_finish)) {
            return;
        }
        let Some(action) = self.release(id) else {
            return;
        };
        let actor = action.actor;
        self.emit(
            Where::Entity(actor),
            Self::action_cause(actor),
            EventBody::Act(ActEvent::ActionFinished {
                actor,
                id,
                action: action_name(&self.defs, action.def),
            }),
        );
        if self.foreground(action.def) {
            self.after_action(actor, FreeReason::Finished(id), Self::action_cause(actor));
        } else {
            self.follow_plan(actor);
        }
    }

    /// The actor abandons an action: `on_cancel`, `action_cancelled`.
    /// Whether the phase allows it is the caller's to have checked.
    pub(crate) fn end_cancelled(&mut self, id: ActionId, cause: Cause) {
        let Some((actor, def_id)) = self
            .actions
            .get(id)
            .map(|action| (action.actor, action.def))
        else {
            return;
        };
        let phase = self.phase_name(id);
        let defs = Arc::clone(&self.defs);
        if let Some(def) = action_def(&defs, def_id) {
            self.run_list_of(id, actor, &def.on_cancel);
        }
        let Some(action) = self.release(id) else {
            return;
        };
        self.emit(
            Where::Entity(actor),
            cause,
            EventBody::Act(ActEvent::ActionCancelled {
                actor,
                id,
                action: action_name(&self.defs, action.def),
                phase,
            }),
        );
    }

    /// Ends an action against its actor's will: `on_interrupt`,
    /// `action_interrupted` with the reason, and the actor is free.
    /// Returns whether it was running.
    pub fn interrupt_action(&mut self, id: ActionId, reason: Reason, cause: Cause) -> bool {
        let Some((actor, def_id)) = self
            .actions
            .get(id)
            .map(|action| (action.actor, action.def))
        else {
            return false;
        };
        let phase = self.phase_name(id);
        let defs = Arc::clone(&self.defs);
        if let Some(def) = action_def(&defs, def_id) {
            self.run_list_of(id, actor, &def.on_interrupt);
        }
        let Some(action) = self.release(id) else {
            return true;
        };
        self.emit(
            Where::Entity(actor),
            cause,
            EventBody::Act(ActEvent::ActionInterrupted {
                actor,
                id,
                action: action_name(&self.defs, action.def),
                phase,
                reason,
            }),
        );
        if self.foreground(action.def) {
            self.after_action(actor, FreeReason::Interrupted(cause), cause);
        } else {
            self.follow_plan(actor);
        }
        true
    }

    /// Ends an action whose actor left the world or is being removed
    /// (03 §9): `on_abandon`, `action_abandoned`. Nobody is asked
    /// anything.
    pub(crate) fn abandon_action(&mut self, id: ActionId) {
        let Some((actor, def_id)) = self
            .actions
            .get(id)
            .map(|action| (action.actor, action.def))
        else {
            return;
        };
        let defs = Arc::clone(&self.defs);
        if let Some(def) = action_def(&defs, def_id) {
            self.run_list_of(id, actor, &def.on_abandon);
        }
        let Some(action) = self.release(id) else {
            return;
        };
        self.emit(
            Where::Entity(actor),
            Self::action_cause(actor),
            EventBody::Act(ActEvent::ActionAbandoned {
                actor,
                id,
                action: action_name(&self.defs, action.def),
            }),
        );
    }

    /// Ends every action of an entity that has left the world, and of
    /// everything inside it, and its wait.
    pub(crate) fn abandon_all(&mut self, entity: EntityId) {
        let mut whom = vec![entity];
        whom.extend(self.entities.contents_deep(entity));
        for who in whom {
            for id in self.actions_of(who).to_vec() {
                self.abandon_action(id);
            }
            if let Some(wait) = self.actions.take_wait(who)
                && let Some(wake) = wait.wake
            {
                self.cancel_wake(wake);
            }
        }
    }

    /// Cancels an action at the actor's own wish (the `Cancel` command):
    /// a held phase is released and the action goes on; otherwise it is
    /// abandoned where the phase allows it. With `abandon`, a held phase
    /// is abandoned too.
    pub fn cancel_action(
        &mut self,
        entity: EntityId,
        id: ActionId,
        abandon: bool,
        cause: Cause,
    ) -> Result<(), Reason> {
        let not_running = || {
            Reason::new(
                ACTION_NOT_RUNNING,
                format!("entity {entity} is doing no action {id}"),
            )
            .with("entity", entity.to_string())
            .with("action", i64::from(id.raw()))
        };
        let action = self
            .actions
            .get(id)
            .filter(|action| action.actor == entity)
            .ok_or_else(not_running)?;
        let defs = Arc::clone(&self.defs);
        let def = action_def(&defs, action.def).ok_or_else(not_running)?;
        let phase = def.phase(action.phase).ok_or_else(not_running)?;
        if phase.is_held() && !abandon {
            // Letting go of a held phase ends it as it would end.
            if let Some(next) = self.leave_phase(id) {
                self.run_phases(id, next);
            }
            return Ok(());
        }
        let values = [
            (
                "action",
                Value::from(action_name(&defs, action.def).as_str()),
            ),
            ("phase", Value::from(phase.name())),
        ];
        match &phase.cancel {
            super::def::CancelRule::Always => {}
            super::def::CancelRule::Never => {
                return Err(self.refusal(&why::CANNOT_CANCEL, entity, &values, None));
            }
            super::def::CancelRule::For(tags) => {
                let names: Vec<Value> = tags
                    .iter()
                    .map(|tag| Value::from(defs.tags().name(tag)))
                    .collect();
                let mut values = values.to_vec();
                values.push(("tags", Value::list(names)));
                return Err(self.refusal(&why::CANCEL_ONLY_FOR, entity, &values, None));
            }
        }
        let foreground = def.foreground;
        self.end_cancelled(id, cause);
        if foreground {
            self.after_action(entity, FreeReason::Finished(id), cause);
        } else {
            self.follow_plan(entity);
        }
        Ok(())
    }

    /// Interrupts whatever holds more slots of an entity's channels than
    /// it has enabled: the newest first, until the rest fit (03 §6.2).
    /// For whatever takes slots away: `disable_channel`, and a body that
    /// lost a part.
    pub fn check_slots(&mut self, entity: EntityId, cause: Cause) {
        let channels: Vec<ChannelId> = self
            .actions_of(entity)
            .iter()
            .filter_map(|id| self.actions.get(*id))
            .flat_map(|action| action.held.iter().map(|(channel, _)| *channel))
            .collect();
        for channel in channels {
            while self.held_slots(entity, channel) > self.enabled_slots(entity, channel) {
                let Some((newest, _)) = self.holders(entity, channel).last().copied() else {
                    break;
                };
                let def = self.actions.get(newest).map(|action| action.def);
                let name = def
                    .map(|def| action_name(&self.defs, def))
                    .unwrap_or_default();
                let channel_name = self.channel_name(channel);
                let reason = self.refusal(
                    &why::CHANNEL_DISABLED,
                    entity,
                    &[
                        ("action", Value::from(name.as_str())),
                        ("channel_name", Value::from(channel_name.as_str())),
                    ],
                    None,
                );
                self.interrupt_action(newest, reason, cause);
            }
        }
    }
}

/// The handler of the `Action` wake-up domain: the end of a phase, a
/// tick, the end of a wait or of an idle.
pub(crate) fn action_wake(sim: &mut Sim, wake_id: WakeId, wake: Wake) {
    match wake.arg {
        WAKE_PHASE | WAKE_TICK => {
            let Ok(raw) = u32::try_from(wake.id) else {
                return;
            };
            let id = ActionId::new(raw);
            let Some(action) = sim.actions.get_mut(id) else {
                return;
            };
            if wake.arg == WAKE_TICK {
                if action.tick == Some(wake_id) {
                    sim.on_tick(id);
                }
                return;
            }
            if action.wake != Some(wake_id) {
                return;
            }
            action.wake = None;
            if let Some(next) = sim.leave_phase(id) {
                sim.run_phases(id, next);
            }
        }
        WAKE_WAIT => {
            let entity = EntityId::from_bits(wake.id);
            let ours = sim
                .actions
                .wait_of(entity)
                .is_some_and(|wait| wait.wake == Some(wake_id));
            if ours {
                sim.actions.take_wait(entity);
                sim.free(entity, FreeReason::Waited, Cause::new(CauseKind::Process));
            }
        }
        _ => {}
    }
}
