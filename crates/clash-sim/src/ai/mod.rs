//! Minds (`docs/09-ai-and-simulation.md` sections 3 to 7): what makes a
//! character that nobody controls decide, from what it believes.
//!
//! **Consistent, not clever** (09 §1, decision D-196). A mind scores the
//! goals its definition lists from what the character believes and
//! feels, takes up the best, and pursues it by the steps of a method:
//! the actions, waits and words a player could have chosen for it. The
//! same character in the same situation does the same thing; whatever
//! chance there is comes from its own stream. Nothing here looks ahead,
//! reasons about the player or plans a deception.
//!
//! | Part | Is | Module |
//! |------|----|--------|
//! | [`MindDef`], [`GoalDef`], [`MethodDef`], [`Step`] | The definition types `mind`, `goal` and `method`, and the steps | [`defs`] |
//! | [`MindState`], [`AiState`] | The `mind_state` component; what the simulation keeps for every mind | [`state`] |
//! | [`think_due`], [`prompt`] | When minds think, choosing, pursuing | [`think`] |
//! | [`MindCosts`], [`recheck_routes`], [`away_from`] | Navigation for minds, the beta subset | [`nav`] |
//! | Properties, verbs, events | What rules and expressions see | [`properties`], [`verbs`], [`events`] |
//!
//! # Belief, not truth
//!
//! Everything a goal, a method or a step holds is compiled in the mind
//! context (05 §7.3): the character's own state in full, and anyone
//! else only through what it believes. A goal that reads another
//! entity's state directly fails to load, naming the read (invariant
//! N1). What the engine computes for a mind in Rust reads the same:
//! its own place, its tracks and memories, and the shape of the world.
//!
//! # What the kernel calls
//!
//! | When | The kernel |
//! |------|-----------|
//! | An event has been delivered to its handlers | Calls [`on_event`]: prompts the minds it concerns, keeps what came of a step's action, notes cells whose shape changed |
//! | The event queue is empty at the end of a pump | Calls [`think_due`]; if it did anything, delivers what that raised |
//! | Building its registry and its verb table | Lists [`props::properties`] and [`props::verbs`] |
//!
//! # Not built
//!
//! Options and tactics (09 §6.1: a `choose` step starts the first action
//! tagged with its purpose that can start), `threat`, positions and
//! `best_reaction` (I5: a mind answers a window by the first of its
//! definition's `reactions` that may start); `use`, `job`, `goal`, `follow` and `run` steps (they
//! wait for interactions, jobs and queries); the hook `goal_scoring`;
//! routines (L2); hierarchical regions and flow fields (I2); collectives
//! (I7); warm stepping (I8); the inspector's views (I9). Stances stand
//! in for relations (S7) and speech raises `said` until speech (S8) is
//! built.

pub mod defs;
pub mod doors;
pub mod events;
pub mod nav;
mod props;
pub mod state;
pub mod think;

use std::collections::{BTreeMap, BTreeSet};

use clash_core::EntityId;
use clash_data::{DefTypes, Defs};

pub use defs::{
    About, AboutKind, GoalDef, GoalId, Hazard, MethodDef, MethodEntry, MethodId, MindDef,
    MindDefId, Step, StepKind,
};
pub use events::ROUTE_BLOCKED;
pub use nav::{MindCosts, away_from, nearest_standable, recheck_routes, with_mind_costs};
pub use props::{properties, verbs};
pub use state::{AboutHeld, AiState, Held, MemoryKey, MindState, Outcome, Pending, Stance};
pub use think::{
    DEFAULT_MAX_BATCH, DEFAULT_MAX_STEPS, DEFAULT_MAX_THOUGHTS, MAX_BATCH_SETTING,
    MAX_STEPS_SETTING, MAX_THOUGHTS_SETTING, mind_def_of, prompt, set_stance, stance_toward,
    think_due,
};

use crate::Sim;
use crate::action::ActEvent;
use crate::bus::FieldValue;
use crate::entity::{Control, Place};
use crate::event::{EventBody, SimEvent, Subject};
use crate::reason::ReasonInfo;

/// The most memories a mind keeps as settled.
pub const MAX_SETTLED: usize = 64;

/// The stable codes of the reasons minds give.
pub mod reason {
    /// A character thinks by no `mind` definition: it has no mind, or its
    /// profile names none. Values: `entity`.
    pub const NO_MIND_DEF: &str = "ai.no_mind_def";
    /// `set_goal` named no goal. Values: `argument`.
    pub const UNKNOWN_GOAL: &str = "ai.unknown_goal";
    /// An argument of the wrong sort. Values: `argument`.
    pub const BAD_VALUE: &str = "ai.bad_value";
    /// No method of a goal could be used. Values: `goal`.
    pub const NO_METHOD: &str = "ai.no_method";
    /// A goal's `fail` held. Values: `goal`.
    pub const GOAL_FAILED: &str = "ai.goal_failed";
    /// A step's target named no place the mind knows of. Values: `step`.
    pub const NO_TARGET: &str = "ai.no_target";
    /// A step that moves ended short of where it was going. Values:
    /// `entity`.
    pub const NOT_ARRIVED: &str = "ai.not_arrived";
    /// A step that moves, for a mind that names no `go_action`. Values:
    /// `entity`.
    pub const NO_GO_ACTION: &str = "ai.no_go_action";
    /// A `keep_from` step found nowhere it believes it can get to.
    /// Values: `entity`.
    pub const NO_WAY_OUT: &str = "ai.no_way_out";
    /// A `choose` step found no action of its purpose that could start.
    /// Values: `purpose`.
    pub const NO_OPTION: &str = "ai.no_option";
    /// A handler of `action_starting` refused a step's action. Values:
    /// `action`.
    pub const REFUSED: &str = "ai.refused";
    /// A step's action was cancelled by something other than its mind.
    /// Values: `action`.
    pub const CANCELLED: &str = "ai.cancelled";
    /// A step that moves met a closed way it cannot open: nothing there
    /// can be done by default, or it has opened too many on the way.
    /// Values: `entity`, `dir`, `why` (what the default there says).
    pub const DOOR_SHUT: &str = "ai.door_shut";
}

/// Every reason minds give, for the generated reference.
pub const REASONS: &[ReasonInfo] = &[
    ReasonInfo {
        code: reason::NO_MIND_DEF,
        doc: "The character thinks by no `mind` definition: it has no mind, or its mind's profile names none. Values: `entity`.",
    },
    ReasonInfo {
        code: reason::UNKNOWN_GOAL,
        doc: "`set_goal` named no goal that is loaded. Values: `argument`.",
    },
    ReasonInfo {
        code: reason::BAD_VALUE,
        doc: "A verb of minds was given an argument it cannot use. Values: `argument`.",
    },
    ReasonInfo {
        code: reason::NO_METHOD,
        doc: "No method of a goal could be used: none's `if` held, or each failed in this pursuit. The goal fails. Values: `goal`.",
    },
    ReasonInfo {
        code: reason::GOAL_FAILED,
        doc: "A goal's `fail` held. Values: `goal`.",
    },
    ReasonInfo {
        code: reason::NO_TARGET,
        doc: "A step's target named no place the mind knows of: null, or someone it has no track of. Values: `step`.",
    },
    ReasonInfo {
        code: reason::NOT_ARRIVED,
        doc: "A step that moves ended short of where it was going: its action ended there, or no way was found. Values: `entity`.",
    },
    ReasonInfo {
        code: reason::NO_GO_ACTION,
        doc: "A step that moves, for a mind that names no `go_action`. Values: `entity`.",
    },
    ReasonInfo {
        code: reason::NO_WAY_OUT,
        doc: "A `keep_from` step found no place it believes it can reach that is further from what it keeps from. Values: `entity`.",
    },
    ReasonInfo {
        code: reason::NO_OPTION,
        doc: "A `choose` step found no action tagged with its purpose that could start. Values: `purpose`.",
    },
    ReasonInfo {
        code: reason::REFUSED,
        doc: "A handler of `action_starting` refused a step's action. Values: `action`.",
    },
    ReasonInfo {
        code: reason::CANCELLED,
        doc: "A step's action was cancelled by something other than the mind that started it. Values: `action`.",
    },
    ReasonInfo {
        code: reason::DOOR_SHUT,
        doc: "A step that moves met a closed way (a step refused with `move.closed`) that it cannot open: nothing there can be done by default now (a locked door and no key), its mind names no `interact_action`, or it has opened four on the one step. Values: `entity`, `dir` (the step's direction), `why` (what the default interaction there says, if there is one).",
    },
];

/// The settings of minds, with their defaults, for the reference.
pub const SETTINGS: &[(&str, i64, &str)] = &[
    (
        MAX_THOUGHTS_SETTING,
        DEFAULT_MAX_THOUGHTS,
        "How many times one mind thinks in one tick; more wait a millisecond.",
    ),
    (
        MAX_BATCH_SETTING,
        DEFAULT_MAX_BATCH,
        "How many minds think in one batch; the rest think in the next.",
    ),
    (
        MAX_STEPS_SETTING,
        DEFAULT_MAX_STEPS,
        "How many steps that take no time one thought takes.",
    ),
];

/// What the definitions say minds listen for: made with the simulation,
/// not state.
#[derive(Clone, Debug, Default)]
pub struct AiDefs {
    /// Every `mind` by qualified id, and by bare name where only one
    /// mod has a mind of that name.
    by_name: BTreeMap<String, MindDefId>,
    /// Events some goal names in `wake_on`.
    wake: BTreeSet<String>,
    /// Events some mind names in `hostile_after` or `fear_after`.
    stance_events: BTreeSet<String>,
}

impl AiDefs {
    /// The tables for a set of loaded definitions.
    pub fn new(defs: &Defs) -> AiDefs {
        let mut out = AiDefs::default();
        if let Some(minds) = defs.try_registry::<MindDef>() {
            let mut bare: BTreeMap<String, Vec<MindDefId>> = BTreeMap::new();
            for (id, def) in minds.iter() {
                let name = minds.name(id).to_owned();
                if let Some((_, short)) = name.split_once(':') {
                    bare.entry(short.to_owned()).or_default().push(id);
                }
                out.by_name.insert(name, id);
                out.stance_events.extend(def.hostile_after.iter().cloned());
                out.stance_events.extend(def.fear_after.iter().cloned());
            }
            for (short, ids) in bare {
                if let [only] = ids.as_slice() {
                    out.by_name.entry(short).or_insert(*only);
                }
            }
        }
        if let Some(goals) = defs.try_registry::<GoalDef>() {
            for (_, goal) in goals.iter() {
                out.wake.extend(goal.wake_on.iter().cloned());
            }
        }
        out
    }

    /// The `mind` a profile names, qualified or bare.
    pub fn mind_named(&self, name: &str) -> Option<MindDefId> {
        self.by_name.get(name).copied()
    }
}

/// Registers the definition types of minds. Called from the crate's
/// `register_defs`.
pub fn register_defs(types: &mut DefTypes) {
    types
        .register::<MindDef>()
        .register::<GoalDef>()
        .register::<MethodDef>();
}

/// Gives a character that thinks by a `mind` definition its state, if
/// it has none yet, and the senses its mind names if it has none
/// ([`think::begin_state`]).
pub(crate) fn ensure_state(sim: &mut Sim, who: EntityId) {
    if sim.components.mind_state.contains(who) {
        return;
    }
    let defs = std::sync::Arc::clone(&sim.defs);
    let Some(def) = mind_def_of(sim, who)
        .and_then(|id| defs.try_registry::<MindDef>().and_then(|r| r.try_get(id)))
    else {
        return;
    };
    think::begin_state(sim, who, def, None);
}

/// Gives a character a state that thinks by a named `mind`, if it has
/// none yet, and the senses that mind names if it has none
/// ([`think::begin_state`]).
pub(crate) fn ensure_state_with(sim: &mut Sim, who: EntityId, mind: &str) {
    if sim.components.mind_state.contains(who) {
        return;
    }
    let defs = std::sync::Arc::clone(&sim.defs);
    let Some(def) = sim
        .ai_defs
        .mind_named(mind)
        .and_then(|id| defs.try_registry::<MindDef>().and_then(|r| r.try_get(id)))
    else {
        return;
    };
    think::begin_state(sim, who, def, Some(mind));
}

/// The entity an event binds under a name, if it is one.
fn entity_subject(event: &SimEvent, name: &str) -> Option<EntityId> {
    event.subject(name).and_then(|subject| subject.entity())
}

/// What the kernel calls for every event it delivers, once rules have
/// had their say: prompts the minds the event concerns, keeps what came
/// of a step's action, takes the stances `hostile_after` and
/// `fear_after` ask for, and notes cells whose shape changed for the
/// routes in use.
pub fn on_event(sim: &mut Sim, event: &SimEvent) {
    match &event.body {
        EventBody::Act(act) => on_act(sim, act),
        EventBody::World(world) => {
            if matches!(
                world,
                clash_world::WorldEvent::ElementChanged { .. }
                    | clash_world::WorldEvent::ElementDestroyed { .. }
            ) {
                nav::changed(sim, world.location());
            }
        }
        EventBody::EntityRelocated { entity, from, to } => {
            relocated(sim, *entity, *from, *to);
        }
        EventBody::ControlChanged { entity, new, .. } if *new != Control::Direct => {
            prompt(sim, *entity);
        }
        EventBody::Custom(payload) => on_custom(sim, event, &payload.name),
        _ => {}
    }
    if !sim.ai_defs.wake.is_empty() && sim.ai_defs.wake.contains(event.name()) {
        for (_, subject) in event.subjects() {
            if let Subject::Entity(entity) = subject
                && sim.components.mind_state.contains(entity)
            {
                prompt(sim, entity);
            }
        }
    }
}

fn on_act(sim: &mut Sim, act: &ActEvent) {
    match act {
        ActEvent::DecisionNeeded { entity, .. } => prompt(sim, *entity),
        ActEvent::ActionFinished { actor, id, .. } => {
            think::action_ended(sim, *actor, *id, Outcome::Done);
        }
        ActEvent::ActionInterrupted {
            actor, id, reason, ..
        } => think::action_ended(sim, *actor, *id, Outcome::Failed(reason.clone())),
        ActEvent::ActionCancelled {
            actor, id, action, ..
        } => {
            let reason =
                crate::reason::Reason::new(reason::CANCELLED, "the step's action was cancelled")
                    .with("action", action.as_str());
            think::action_ended(sim, *actor, *id, Outcome::Failed(reason));
        }
        _ => {}
    }
}

/// An entity's place changed: one that came into the world thinks; one
/// that moved toward where a step was taking it thinks when it has come
/// close enough.
fn relocated(sim: &mut Sim, entity: EntityId, from: Place, to: Place) {
    let Some(state) = sim.components.mind_state.get(entity) else {
        return;
    };
    if from == Place::Stored && to != Place::Stored {
        prompt(sim, entity);
        return;
    }
    let arrived = state
        .held()
        .and_then(|held| held.pending.as_ref())
        .filter(|pending| pending.outcome == Outcome::Running)
        .filter(|pending| {
            matches!(
                pending.kind(),
                StepKind::Go | StepKind::KeepNear | StepKind::KeepFrom
            )
        })
        .and_then(|pending| Some((pending.target?, pending.within)))
        .is_some_and(|(target, within)| {
            nav::distance_to(sim, entity, target)
                .is_some_and(|d| d <= within + clash_core::Real::of(think::ARRIVED_SLACK_M))
        });
    if arrived {
        sim.ai.due.insert(entity);
    }
}

fn on_custom(sim: &mut Sim, event: &SimEvent, name: &str) {
    match name {
        crate::perception::events::PERCEIVED => {
            let Some(observer) = entity_subject(event, "self") else {
                return;
            };
            let Some(def) = mind_def_of(sim, observer).and_then(|id| think::mind_def(sim, id))
            else {
                return;
            };
            let clarity = match event.body.field("clarity") {
                Some(FieldValue::Plain(clash_core::Value::Real(value))) => Some(value),
                Some(FieldValue::Plain(clash_core::Value::Int(value))) => {
                    Some(clash_core::Real::from_i64(value))
                }
                _ => None,
            }
            .unwrap_or(clash_core::Real::ZERO);
            let carried = think::text_field(event.body.field("event"));
            let actor = match event.body.field("about_actor") {
                Some(FieldValue::Subject(Subject::Entity(actor))) => Some(actor),
                _ => None,
            };
            let stance = carried.as_deref().and_then(|carried| {
                if def.hostile_after.iter().any(|name| name == carried) {
                    Some(Stance::Hostile)
                } else if def.fear_after.iter().any(|name| name == carried) {
                    Some(Stance::Feared)
                } else {
                    None
                }
            });
            let wake = clarity >= def.wake_clarity;
            if let (Some(stance), Some(actor)) = (stance, actor)
                && actor != observer
            {
                think::set_stance(sim, observer, actor, Some(stance));
            }
            if wake || stance.is_some() {
                prompt(sim, observer);
            }
        }
        // Coming to perceive someone, losing them, and remembering
        // something are always worth a thought.
        crate::perception::events::DETECTED
        | crate::perception::events::LOST_TRACK
        | crate::perception::events::MEMORY_ADDED
        | "entity_spawned"
        | "entity_woke"
        | "entity_revived" => {
            if let Some(entity) = entity_subject(event, "self") {
                prompt(sim, entity);
            }
        }
        "mind_transferred" => {
            for subject in ["self", "target"] {
                if let Some(entity) = entity_subject(event, subject) {
                    prompt(sim, entity);
                }
            }
        }
        crate::action::WINDOW_NOTICED => react_to_window(sim, event),
        crate::movement::STEP_REFUSED => doors::step_refused(sim, event),
        _ => {}
    }
}

/// A mind answers a window its character noticed (03 §7.3, 09 §6.3)
/// with the first of its definition's `reactions` that may start and
/// would be ready in time, else the first that may start; with none,
/// nothing. A directly controlled character is the player's to answer.
fn react_to_window(sim: &mut Sim, event: &SimEvent) {
    let Some(who) = entity_subject(event, "self") else {
        return;
    };
    if sim
        .truth()
        .entity(who)
        .is_none_or(|record| record.control() == Control::Direct)
    {
        return;
    }
    let Some(wanted) = mind_def_of(sim, who)
        .and_then(|id| think::mind_def(sim, id))
        .map(|def| {
            def.reactions
                .iter()
                .map(|action| action.id())
                .collect::<Vec<_>>()
        })
        .filter(|wanted| !wanted.is_empty())
    else {
        return;
    };
    let Some(window) = event
        .body
        .field("window")
        .and_then(|field| sim.exprs().field_value(&field).as_subject())
        .and_then(|handle| sim.window_of(&clash_expr::Value::Subject(handle)))
    else {
        return;
    };
    let options = sim.reaction_options(who, window);
    let may = |action: &crate::action::ActionDefId, in_time: bool| {
        options.iter().any(|option| {
            option.action == *action && option.refused.is_none() && (option.in_time || !in_time)
        })
    };
    let Some(action) = wanted
        .iter()
        .find(|action| may(action, true))
        .or_else(|| wanted.iter().find(|action| may(action, false)))
        .copied()
    else {
        return;
    };
    let cause = clash_world::Cause {
        actor: Some(who),
        instigator: Some(who),
        kind: clash_world::CauseKind::Action,
    };
    if let Err(reason) = sim.react(who, window, action, &Vec::new(), cause) {
        sim.fault(reason);
    }
}

impl Sim {
    /// What the simulation keeps for every mind at once.
    pub fn ai(&self) -> &AiState {
        &self.ai
    }

    /// The tables of minds made from the definitions.
    pub fn ai_defs(&self) -> &AiDefs {
        &self.ai_defs
    }

    /// A character's mind state, if its mind has thought.
    pub fn mind_state(&self, entity: EntityId) -> Option<&MindState> {
        self.components.mind_state.get(entity)
    }
}
