//! Reacting (03 §7.3): answering a window that was noticed.
//!
//! The engine does nothing by itself when a window is noticed: rules
//! listening for `window_noticed` decide, with the verbs here, and the
//! AI layer answers the same event for the minds it drives (09 §6.3).
//!
//! | Verb, command | Does |
//! |---------------|------|
//! | `react { who, window, action, with? }`, `React` | Starts a reaction with `$window` bound: an action whose `reaction_to` lists the window's kind |
//! | `ask_reaction { who, window }` | For a directly controlled `who`: a `Reaction` decision listing its options; time stops until `React` or `Pass` answers |
//! | `Pass` | Declines a `Reaction` decision |
//! | `close_window { window }` | Closes a window |
//! | `open_window { kind, source?, for?, time }` | Opens a window that closes after a length of time |
//!
//! # Starting a reaction
//!
//! In this order, refusing with the first reason that applies and
//! changing nothing:
//!
//! 1. The window is open (`engine:window_closed`).
//! 2. `who` is one of its candidates and has noticed it
//!    (`engine:window_noticed_late` for one whose delay was not over
//!    when it closed, `engine:window_not_noticed` otherwise).
//! 3. The action lists the window's kind in `reaction_to`
//!    (`engine:reaction_not_listed`).
//! 4. The window names `who`, or the action allows `on_behalf`
//!    (`engine:reaction_not_on_behalf`).
//! 5. Everything an action checks when it starts (03 §6.6), with
//!    `$window` bound. The reactor's own actions in phases that allow
//!    `cancel` are cancelled to make room; one that holds the slots and
//!    does not allow it makes the reaction unavailable
//!    (`engine:reaction_cannot_cancel`): someone caught in recovery
//!    cannot block.
//!
//! Then the reactor turns to face the window's source, and the action
//! starts as any other.
//!
//! # Options
//!
//! [`Sim::reaction_options`] lists, by the actions' `offer` (higher
//! first, decision D-943) and then in order of definition, every action
//! that may answer a window for an entity, each with whether its `ready`
//! phase would begin before the window is expected to close and, if it
//! cannot start now, why. The time to `ready` is the phases' durations
//! along their default path (the following phase), worked out as they
//! would be if it started now; a held phase with no `max` on the way
//! means it is never known to be ready, and so is not in time.

use std::sync::Arc;

use clash_core::{Dur, EntityId, Real};
use clash_expr::{Type, Value, VerbSpec};
use clash_world::{Cause, CauseKind};

use super::def::{ActionDef, Next, PhaseTime};
use super::event::ActEvent;
use super::ids::ActionDefId;
use super::reason as why;
use super::run::{ACTION_UNKNOWN, COMMAND_NOT_CONTROLLED, action_def, action_name};
use super::state::Params;
use super::windows::{Notice, OpenWindow, WindowId, kind_name};
use crate::Sim;
use crate::decision::{DecisionKind, ReactionOption};
use crate::entity::Control;
use crate::event::{EventBody, Where};
use crate::kernel_verbs::{arg, bad_argument, entity_arg};
use crate::later::ActionId;
use crate::reason::{Reason, codes};
use crate::rule::VerbCall;
use crate::verb::VerbDef;

impl Sim {
    /// The value of a window's kind, as a reason carries it.
    fn kind_value(&self, window: WindowId) -> Value {
        let Some(kind) = self.windows.get(window).map(|window| window.kind) else {
            return Value::Null;
        };
        self.defs
            .expr_registry()
            .def_type_id("window_kind")
            .map_or(Value::Null, |ty| {
                Value::Id(clash_expr::DefRef::new(ty, u32::from(kind.raw())))
            })
    }

    /// Whether an entity may answer a window at all: it is open, and the
    /// entity is a candidate that has noticed it. The reason if not.
    fn may_answer(&self, entity: EntityId, window: WindowId) -> Result<(), Reason> {
        let Some(open) = self.windows.get(window) else {
            return Err(self.refusal(&why::WINDOW_CLOSED, entity, &[("kind", Value::Null)], None));
        };
        let kind = self.kind_value(window);
        match open.candidate(entity) {
            Some(candidate) if candidate.state.noticed() => Ok(()),
            Some(candidate) if candidate.state == Notice::Late => Err(self.refusal(
                &why::WINDOW_NOTICED_LATE,
                entity,
                &[
                    ("kind", kind),
                    ("notice", Value::Real(candidate.delay.as_secs_real())),
                    (
                        "open",
                        Value::Real(self.now().since(open.opened).as_secs_real()),
                    ),
                ],
                None,
            )),
            _ => Err(self.refusal(&why::WINDOW_NOT_NOTICED, entity, &[("kind", kind)], None)),
        }
    }

    /// Whether an action may answer a window for an entity, and if so
    /// what it would start with (see the [module documentation](self)).
    /// Changes nothing but the rule stream, which the caller puts back.
    fn prepare_reaction(
        &mut self,
        entity: EntityId,
        window: WindowId,
        action: ActionDefId,
        params: &Params,
    ) -> Result<super::run::Prepared, Reason> {
        self.known(entity)?;
        self.may_answer(entity, window)?;
        let defs = Arc::clone(&self.defs);
        let def = action_def(&defs, action).ok_or_else(|| {
            Reason::new(
                ACTION_UNKNOWN,
                format!("there is no action definition {action}"),
            )
            .with("action", i64::from(action.raw()))
        })?;
        let open = self.windows.get(window).ok_or_else(|| {
            self.refusal(&why::WINDOW_CLOSED, entity, &[("kind", Value::Null)], None)
        })?;
        let (kind, named) = (open.kind, open.names(entity));
        let values = |sim: &Sim| {
            vec![
                (
                    "action",
                    sim.defs
                        .expr_registry()
                        .def_type_id(<ActionDef as clash_data::Def>::TYPE)
                        .map_or(Value::Null, |ty| {
                            Value::Id(clash_expr::DefRef::new(ty, action.raw()))
                        }),
                ),
                ("kind", sim.kind_value(window)),
            ]
        };
        if !def.reacts_to(kind) {
            return Err(self.refusal(&why::NOT_A_REACTION, entity, &values(self), None));
        }
        if !named && !def.on_behalf {
            return Err(self.refusal(&why::NOT_ON_BEHALF, entity, &values(self), None));
        }
        let handle = self.window_value(window);
        self.prepare_with(entity, action, params, true, &handle)
            .map_err(|reason| {
                if reason.code != why::CHANNEL_BUSY.code {
                    return reason;
                }
                // The slots are held by actions that cannot be broken off
                // for this one: someone caught in recovery cannot block.
                let holder = self
                    .actions_of(entity)
                    .iter()
                    .filter_map(|id| self.actions.get(*id).map(|running| (*id, running)))
                    .find(|(id, _)| !self.may_be_displaced(*id, &def.tags));
                let (holder_value, phase) =
                    holder.map_or((Value::Null, Value::Null), |(_, running)| {
                        let phase = action_def(&defs, running.def)
                            .and_then(|def| def.phase(running.phase))
                            .map_or(Value::Null, |phase| Value::from(phase.name()));
                        let id = defs
                            .expr_registry()
                            .def_type_id(<ActionDef as clash_data::Def>::TYPE)
                            .map_or(Value::Null, |ty| {
                                Value::Id(clash_expr::DefRef::new(ty, running.def.raw()))
                            });
                        (id, phase)
                    });
                let mut all = values(self);
                all.retain(|(name, _)| *name == "action");
                all.push((
                    "channel",
                    reason
                        .values
                        .iter()
                        .find(|(name, _)| name == "channel")
                        .map_or(Value::Null, |(_, value)| Value::from(value)),
                ));
                all.push(("holder", holder_value));
                all.push(("phase", phase));
                self.refusal(&why::REACTION_CANNOT_CANCEL, entity, &all, None)
            })
    }

    /// Whether a running action is in a phase that may be broken off to
    /// start an action with these tags.
    fn may_be_displaced(&self, id: ActionId, tags: &clash_data::TagSet) -> bool {
        self.actions
            .get(id)
            .and_then(|running| action_def(&self.defs, running.def)?.phase(running.phase))
            .is_some_and(|phase| phase.cancel.allows(tags))
    }

    /// Starts a reaction to a window (03 §7.3): see the
    /// [module documentation](self). `Err` is a refusal before anything
    /// changed; `Ok(None)` an action a handler of `action_starting`
    /// refused, which raised `action_refused`.
    pub fn react(
        &mut self,
        entity: EntityId,
        window: WindowId,
        action: ActionDefId,
        params: &Params,
        cause: Cause,
    ) -> Result<Option<ActionId>, Reason> {
        let streams = self.rng.clone();
        let prepared = match self.prepare_reaction(entity, window, action, params) {
            Ok(prepared) => prepared,
            Err(reason) => {
                self.rng = streams;
                return Err(reason);
            }
        };
        // A reaction a player was asked for is answered.
        if self
            .windows
            .get(window)
            .and_then(|open| open.candidate(entity))
            .map(|c| c.state)
            == Some(Notice::Asked)
        {
            self.answer_which(entity, |kind| {
                matches!(kind, DecisionKind::Reaction { window: asked, .. } if *asked == window)
            });
        }
        // The reactor turns to meet what it answers.
        let source = self
            .windows
            .get(window)
            .and_then(|open| open.source)
            .filter(|source| *source != entity)
            .and_then(|source| self.entities.location(source));
        if let Some(at) = source
            && self.components.senses.contains(entity)
        {
            let _ = crate::perception::face_toward(self, entity, at);
        }
        let started = self.begin(prepared, cause).ok();
        if let Some(id) = started {
            self.set_notice(window, entity, Notice::Reacted(id));
        }
        Ok(started)
    }

    /// The reactions that may answer a window for an entity, by their
    /// `offer`, higher first, and equals in order of definition (03 §7.3,
    /// decision D-943): see the [module documentation](self).
    /// Changes nothing. Empty for a window that is not open or that the
    /// entity has not noticed.
    pub fn reaction_options(&mut self, entity: EntityId, window: WindowId) -> Vec<ReactionOption> {
        let Some(open) = self.windows.get(window) else {
            return Vec::new();
        };
        if self.may_answer(entity, window).is_err() {
            return Vec::new();
        }
        let (kind, named, deadline) = (open.kind, open.names(entity), open.deadline);
        let defs = Arc::clone(&self.defs);
        let Some(registry) = defs.try_registry::<ActionDef>() else {
            return Vec::new();
        };
        let mut out = Vec::new();
        for (id, def) in registry.iter() {
            if !def.reacts_to(kind) || !(named || def.on_behalf) {
                continue;
            }
            let streams = self.rng.clone();
            let prepared = self.prepare_reaction(entity, window, id, &Vec::new());
            // Where it is offered (D-943): by what it would start with,
            // and 0 for one that cannot start.
            let offer = prepared.as_ref().map_or(Real::ZERO, |prepared| {
                crate::offer::offer_of(self, entity, def, prepared.locals())
            });
            let (in_time, refused) = match prepared {
                Ok(prepared) => {
                    let ready = self.time_to_ready(entity, def, prepared.locals());
                    let in_time = match (ready, deadline) {
                        (_, None) => true,
                        (None, Some(_)) => false,
                        (Some(own), Some(deadline)) => {
                            self.own_to_universal(entity, own).is_some_and(|universal| {
                                self.now().as_micros().saturating_add(universal.as_micros())
                                    <= deadline.as_micros()
                            })
                        }
                    };
                    (in_time, None)
                }
                Err(reason) => (false, Some(reason)),
            };
            self.rng = streams;
            let label = def.name.as_ref().map_or_else(
                || {
                    let name = registry.name(id);
                    name.rsplit(':').next().unwrap_or(name).to_owned()
                },
                |text| {
                    let bound = crate::host::Bound::new()
                        .subject(self.subject_value(crate::event::Subject::Entity(entity)));
                    let mut host = self.host_for_reader(bound, entity);
                    text.render_on(&defs, &mut host)
                },
            );
            out.push((
                core::cmp::Reverse(offer),
                ReactionOption {
                    action: id,
                    label,
                    in_time,
                    refused,
                },
            ));
        }
        // Higher `offer` first; equals in order of definition.
        out.sort_by_key(|(offer, _)| *offer);
        out.into_iter().map(|(_, option)| option).collect()
    }

    /// How long, in the reactor's own time, before a reaction that
    /// started now would reach its `ready` phase: the durations of the
    /// phases before it along their default path. Zero with no `ready`;
    /// `None` where a held phase with no `max` is on the way or the
    /// path never reaches it.
    fn time_to_ready(
        &mut self,
        entity: EntityId,
        def: &ActionDef,
        locals: &[Value],
    ) -> Option<Dur> {
        let ready = def.ready()?;
        let mut total = Real::ZERO;
        let mut at = Next::Phase(super::ids::PhaseIndex::new(0));
        for _ in 0..def.phases().len() {
            let Next::Phase(index) = at else {
                return None;
            };
            if index == ready {
                return Some(Dur::from_secs_real(total));
            }
            let phase = def.phase(index)?;
            let secs = match &phase.time {
                PhaseTime::For(time) => self.action_eval(entity, locals, |defs, host| {
                    let secs = time.eval_on(defs, host)?;
                    let tempo = if phase.fixed {
                        Real::ONE
                    } else {
                        def.tempo.eval_on(defs, host).filter(|t| *t > Real::ZERO)?
                    };
                    Some(secs / tempo)
                })?,
                PhaseTime::Hold => self.action_eval(entity, locals, |defs, host| {
                    phase.max.as_ref()?.eval_on(defs, host)
                })?,
            };
            total += secs.max(Real::ZERO);
            at = phase.after();
        }
        None
    }

    /// Asks a directly controlled entity how to answer a window it has
    /// noticed (03 §7.3): a `Reaction` decision with its options, and
    /// time does not move until `React` or `Pass` answers. Gives false,
    /// asking nothing, for anyone else, for an entity that has already
    /// been asked, and where no action may answer the window for it.
    pub fn ask_reaction(
        &mut self,
        entity: EntityId,
        window: WindowId,
        cause: Cause,
    ) -> Result<bool, Reason> {
        let control = self.known(entity)?.control();
        self.may_answer(entity, window)?;
        if control != Control::Direct {
            return Ok(false);
        }
        // A stopped mind is asked nothing, as it is not when it is free
        // (06 §8.4): the blow simply lands on someone unconscious, and
        // time goes on (decision D-907).
        if self.life_lets_act(entity).is_err() {
            return Ok(false);
        }
        let state = self
            .windows
            .get(window)
            .and_then(|open| open.candidate(entity))
            .map(|candidate| candidate.state);
        if state == Some(Notice::Asked) {
            return Ok(false);
        }
        let options = self.reaction_options(entity, window);
        if options.is_empty() {
            return Ok(false);
        }
        let deadline = self.windows.get(window).and_then(|open| open.deadline);
        let asked = self.request_decision(
            entity,
            DecisionKind::Reaction {
                window,
                deadline,
                options,
            },
            cause,
        )?;
        if asked {
            self.set_notice(window, entity, Notice::Asked);
        }
        Ok(asked)
    }

    /// Whether commands may address an entity: it is under `Direct` or
    /// `Delegated` control (03 §4.4).
    fn reactor_commanded(&self, entity: EntityId) -> Result<(), Reason> {
        if self.known(entity)?.control() == Control::None {
            return Err(Reason::new(
                COMMAND_NOT_CONTROLLED,
                format!("nobody controls entity {entity}, so it takes no commands"),
            )
            .with("entity", entity.to_string()));
        }
        Ok(())
    }

    /// The `React` command (03 §4.4): starts a reaction for an entity
    /// under direct or delegated control, answering the `Reaction`
    /// decision it was asked for the window, if it was. Refused, with the
    /// reason, if the reaction cannot start; a handler of
    /// `action_starting` that refuses it raises `action_refused`, the
    /// command having been taken.
    pub(crate) fn command_react(
        &mut self,
        entity: EntityId,
        window: WindowId,
        action: ActionDefId,
        params: Params,
    ) -> Result<(), Reason> {
        self.reactor_commanded(entity)?;
        self.life_lets_act(entity)?;
        let cause = Cause {
            actor: Some(entity),
            instigator: Some(entity),
            kind: CauseKind::Action,
        };
        self.react(entity, window, action, &params, cause)?;
        Ok(())
    }

    /// The `Pass` command (03 §4.4): declines the `Reaction` decision an
    /// entity was asked for a window. Refused if it was asked none.
    pub(crate) fn command_pass(
        &mut self,
        entity: EntityId,
        window: WindowId,
    ) -> Result<(), Reason> {
        self.reactor_commanded(entity)?;
        let answered = self.answer_which(
            entity,
            |kind| matches!(kind, DecisionKind::Reaction { window: asked, .. } if *asked == window),
        );
        if answered.is_none() {
            return Err(Reason::new(
                codes::DECISION_NONE_PENDING,
                format!("entity {entity} was not asked how to answer window {window}"),
            )
            .with("entity", entity.to_string())
            .with("window", i64::try_from(window.raw()).unwrap_or(i64::MAX)));
        }
        self.set_notice(window, entity, Notice::Passed);
        Ok(())
    }
}

/// An argument that names a window.
fn window_arg(sim: &Sim, call: &VerbCall<'_>, at: usize, verb: &str) -> Result<WindowId, Reason> {
    sim.window_of(arg(call, at))
        .ok_or_else(|| bad_argument(verb, "window", "`window` must be a window"))
}

/// An argument that names a definition of a type, by its number.
fn def_arg(sim: &Sim, call: &VerbCall<'_>, at: usize, ty: &str) -> Option<u32> {
    match arg(call, at) {
        Value::Id(id) if sim.defs().expr_registry().def_type_id(ty) == Some(id.ty) => {
            Some(id.index)
        }
        _ => None,
    }
}

fn react(sim: &mut Sim, call: &VerbCall<'_>, cause: Cause) -> Result<Value, Reason> {
    let who = entity_arg(sim, call, 0, "react", "who")?;
    let window = window_arg(sim, call, 1, "react")?;
    let action = def_arg(sim, call, 2, "action")
        .map(ActionDefId::new)
        .ok_or_else(|| bad_argument("react", "action", "`action` must be the id of an action"))?;
    let params: Params = arg(call, 3).as_map().map_or_else(Vec::new, |map| {
        map.iter()
            .map(|(key, value)| (key.to_string(), value.to_stored()))
            .collect()
    });
    match sim.react(who, window, action, &params, cause) {
        Ok(started) => Ok(Value::Bool(started.is_some())),
        Err(reason) => {
            let name = action_name(sim.defs(), action);
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

fn ask_reaction(sim: &mut Sim, call: &VerbCall<'_>, cause: Cause) -> Result<Value, Reason> {
    let who = entity_arg(sim, call, 0, "ask_reaction", "who")?;
    let window = window_arg(sim, call, 1, "ask_reaction")?;
    sim.ask_reaction(who, window, cause).map(Value::Bool)
}

fn close_window(sim: &mut Sim, call: &VerbCall<'_>, _cause: Cause) -> Result<Value, Reason> {
    let window = window_arg(sim, call, 0, "close_window")?;
    Ok(Value::Bool(sim.close_window(window)))
}

fn open_window(sim: &mut Sim, call: &VerbCall<'_>, _cause: Cause) -> Result<Value, Reason> {
    let kind = def_arg(sim, call, 0, "window_kind")
        .and_then(|index| u16::try_from(index).ok())
        .map(super::ids::WindowKindId::new)
        .ok_or_else(|| {
            bad_argument(
                "open_window",
                "kind",
                "`kind` must be the id of a window kind",
            )
        })?;
    let entity = |sim: &Sim, value: &Value| match sim.subject_of(value) {
        Some(crate::event::Subject::Entity(id)) => Some(id),
        _ => None,
    };
    let source = entity(sim, arg(call, 1));
    let aimed_at: Vec<EntityId> = arg(call, 2)
        .as_list()
        .map(|items| items.iter().filter_map(|item| entity(sim, item)).collect())
        .unwrap_or_default();
    let secs = arg(call, 3)
        .as_number()
        .filter(|secs| secs.is_finite() && *secs >= Real::ZERO)
        .ok_or_else(|| {
            bad_argument(
                "open_window",
                "time",
                "`time` must be a length of time in seconds, zero or more",
            )
        })?;
    let id = sim.open_window(OpenWindow {
        kind,
        source,
        aimed_at,
        open_for: Dur::from_secs_real(secs),
    })?;
    Ok(sim.window_value(id))
}

/// The verbs of windows and reactions (03 §13). See the
/// [module documentation](self).
pub fn verbs() -> Vec<VerbDef> {
    vec![
        VerbDef {
            name: "react",
            build: |registry| {
                let entity = registry.kind_id("entity")?;
                let window = registry.kind_id(super::windows::WINDOW_KIND)?;
                let action = registry.def_type_id("action")?;
                Some(
                    VerbSpec::new("react")
                        .arg("who", Type::Subject(entity), "Who reacts: a candidate that noticed the window.")
                        .arg("window", Type::Subject(window), "The window answered: `event.window` of `window_noticed`.")
                        .arg("action", Type::Id(action), "The reaction: an action whose `reaction_to` lists the window's kind.")
                        .optional_arg("with", Type::map(Type::Any), "Its parameters, by name; those left out take their defaults, worked out with `$window` bound.")
                        .doc("Starts a reaction to a window (03 §7.3), cancelling actions of `who` in phases that allow it to make room. Gives whether it started; if it did not, `action_refused` says why: `engine:window_closed`, `engine:window_not_noticed`, `engine:window_noticed_late`, `engine:reaction_not_listed`, `engine:reaction_not_on_behalf`, `engine:reaction_cannot_cancel` or any reason an action is refused."),
                )
            },
            run: react,
        },
        VerbDef {
            name: "ask_reaction",
            build: |registry| {
                let entity = registry.kind_id("entity")?;
                let window = registry.kind_id(super::windows::WINDOW_KIND)?;
                Some(
                    VerbSpec::new("ask_reaction")
                        .arg("who", Type::Subject(entity), "Who is asked: a directly controlled candidate that noticed the window.")
                        .arg("window", Type::Subject(window), "The window.")
                        .doc("Asks a directly controlled entity how to answer a window it noticed (03 §7.3): a `Reaction` decision listing the reactions that may answer it, each saying whether it would be ready in time and, if it cannot start, why. Time does not move until `React` or `Pass` answers. Gives whether it asked: false for anyone not directly controlled, for one already asked, and where nothing may answer. Refused with `engine:window_closed` or `engine:window_not_noticed`."),
                )
            },
            run: ask_reaction,
        },
        VerbDef {
            name: "close_window",
            build: |registry| {
                let window = registry.kind_id(super::windows::WINDOW_KIND)?;
                Some(
                    VerbSpec::new("close_window")
                        .arg("window", Type::Subject(window), "The window.")
                        .doc("Withdraws an opportunity (03 §13): the window closes now, as at its deadline. Gives whether it was open."),
                )
            },
            run: close_window,
        },
        VerbDef {
            name: "open_window",
            build: |registry| {
                let entity = registry.kind_id("entity")?;
                let window_kind = registry.def_type_id("window_kind")?;
                Some(
                    VerbSpec::new("open_window")
                        .arg("kind", Type::Id(window_kind), "Its kind.")
                        .optional_arg("source", Type::Subject(entity).nullable(), "Whose doing it is; none for a window no one opened.")
                        .optional_arg("for", Type::list(Type::Subject(entity)), "Those it is aimed at.")
                        .arg("time", Type::Real, "How long it stays open, in seconds of universal time: `\"2 s\"`.")
                        .doc("Publishes an opportunity (03 §7.1) that no phase closes: it is offered and noticed as a phase's window is, and closes when its time is over (`window_closed`). Gives the window."),
                )
            },
            run: open_window,
        },
    ]
}

/// The name of a window's kind, for tools.
pub fn window_kind_name(sim: &Sim, window: WindowId) -> Option<String> {
    let kind = sim.windows.get(window)?.kind;
    Some(kind_name(sim.defs(), kind))
}
