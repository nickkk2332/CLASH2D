//! Game scenarios (13): what a game is, what the player is trying to do
//! and how it ends.
//!
//! | Part | Is |
//! |------|----|
//! | [`ScenarioDef`], [`MissionDef`], [`SettingDef`] | The `scenario`, `mission` and `setting` definition types |
//! | [`make_world`] | The realm a scenario's game is made in, with its seed and its time of day (13 §4.1, §4.3) |
//! | `Command::BeginScenario` | Places the party, applies settings, rules and `start.do`, raises `scenario_started` (13 §4) |
//! | [`ScenarioState`], [`Game`] | The game in the simulation, saved and hashed |
//! | [`journal`] | What the journal screen reads: `entity.objectives` |
//! | [`verbs`], [`event_types`], [`SCENARIO_EVENTS`], [`SCENARIO_REASONS`] | What rules call, listen for and are refused with |
//!
//! # Starting a game
//!
//! Whoever starts a game (the application, the console) makes the
//! world with [`make_world`]: the scenario's realm, created from the
//! seed the scenario's `seed` chooses, and the realm's clock set so
//! that the game begins at the day and time `start.time` names. Time
//! moved so passes before anything is in the world. It makes the
//! simulation over that world, spawns the party's character (the
//! command `Spawn`) and submits `Command::BeginScenario`, which:
//!
//! 1. records the game: the scenario, when it began, the party;
//! 2. applies the scenario's `settings`, then the answers to its
//!    `options`, each checked against its `setting` definition;
//! 3. switches the rule groups the scenario names in `rules` on, and
//!    every group another scenario names off (13 §3: rules in force
//!    only in that scenario);
//! 4. gives each character of the party senses if its templates gave
//!    none: those its entry names, or every sense the mods define;
//! 5. places the party at the place chosen: a cell, or the anchor of a
//!    site chosen by query among the realm's surveyed sites, from the
//!    rule stream; and gives it direct control;
//! 6. shows the objectives that wait for nothing, sets their expiry,
//!    runs `start.do` and raises `scenario_started`.
//!
//! The same answers (scenario, seed, place, options) give the same
//! state hash.
//!
//! # Objectives are kept current without polling
//!
//! An objective's `done` and `failed` are reactive conditions (05
//! §7.5). When mods load, each is looked through for what it reads:
//! the events each property and function it reads is registered as
//! changed by, and `var_changed` for a variable. A handler is
//! registered for each such event; when one is delivered while a game
//! is being played, the objectives that watch it are looked at again
//! at the next safe point (03 §4.3), after the pump. A condition that
//! reads something no event is registered for (a property with no
//! `changed_by`, a query) is looked at again after every event. What
//! changes is settled in rounds, at most `rules.max_reactive_rounds`
//! (default 8), as its effects may change what others read.
//!
//! So an objective's state always equals a fresh evaluation of its
//! conditions until it is done or failed, and then it does not change
//! (invariant Z4).
//!
//! # Endings and requests
//!
//! Saving, restoring and ending act on the simulation as a whole: the
//! simulation records what was asked and raises an [`AppRequest`]
//! (13 §9.1), which [`Sim::advance`](crate::Sim::advance) returns at the next safe point
//! (invariant Z6). The application carries it out.
//!
//! [`AppRequest`]: crate::AppRequest

pub mod def;
pub mod journal;
mod run;
pub mod state;
mod world;

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use clash_data::{Cond, DefTypes, Defs};
use clash_expr::Registry;

use crate::bus::{EventType, FieldType};
use crate::event::EventInfo;
use crate::reason::ReasonInfo;

pub use def::{
    CharacterEntry, EndEntry, MarkerDef, MissionDef, MissionId, ObjectiveDef, PartyGone,
    PlaceEntry, SAVING_POLICIES, SavingDef, ScenarioDef, ScenarioId, SeedSpec, SettingDef,
    SettingId, SiteQuery, StartPart, TimeOfDay, WorldEntry, stored_value,
};
pub use run::{
    BeginScenario, MAX_SURVEYED_REGIONS, SCENARIO_CAUSE, SCENARIO_WAKE_TAG, begin,
    fresh_objective_state, is_scenario_wake, objective_state, on_timer, played, settle, verbs,
};
pub(crate) use run::{bind, setting_command};
pub use state::{Game, Mission, MissionState, Objective, ObjectiveState, ScenarioState};
pub use world::{WorldMade, make_world, start_tick};

/// `scenario_started`: the party is placed and `start.do` has run.
pub const SCENARIO_STARTED: &str = "scenario_started";
/// `objective_changed`: an objective was shown, done or failed.
pub const OBJECTIVE_CHANGED: &str = "objective_changed";
/// `mission_started`.
pub const MISSION_STARTED: &str = "mission_started";
/// `mission_done`.
pub const MISSION_DONE: &str = "mission_done";
/// `mission_failed`.
pub const MISSION_FAILED: &str = "mission_failed";
/// `mission_expired`.
pub const MISSION_EXPIRED: &str = "mission_expired";
/// `party_gone`: no directly controlled character remains able to act.
pub const PARTY_GONE: &str = "party_gone";
/// `checkpoint_made`: a rule asked for a checkpoint.
pub const CHECKPOINT_MADE: &str = "checkpoint_made";
/// `scenario_ending`: the hook before an ending takes effect.
pub const SCENARIO_ENDING: &str = "scenario_ending";
/// `scenario_ended`.
pub const SCENARIO_ENDED: &str = "scenario_ended";
/// `setting_changed`: the `Setting` command changed a setting (04 §10).
pub const SETTING_CHANGED: &str = "setting_changed";

/// The setting that bounds how many rounds of looking at objectives
/// again one safe point makes (05 §7.5).
pub const MAX_ROUNDS_SETTING: &str = "rules.max_reactive_rounds";
/// Its default.
pub const DEFAULT_MAX_ROUNDS: i64 = 8;

/// The codes of the reasons scenarios refuse with.
pub mod why {
    /// No game is being played.
    pub const NO_GAME: &str = "scenario.no_game";
    /// The game has ended.
    pub const ENDED: &str = "scenario.ended";
    /// A game is being played already.
    pub const BEGUN: &str = "scenario.begun";
    /// No scenario of that id is loaded.
    pub const UNKNOWN: &str = "scenario.unknown";
    /// The place asked for is not one of the scenario's, or cannot be
    /// found or stood in.
    pub const BAD_PLACE: &str = "scenario.bad_place";
    /// A character of the party is not one that exists.
    pub const BAD_PARTY: &str = "scenario.bad_party";
    /// The scenario's world was not made as it says.
    pub const BAD_WORLD: &str = "scenario.bad_world";
    /// No objective of that id.
    pub const NO_OBJECTIVE: &str = "scenario.no_objective";
    /// No mission of that id or number.
    pub const NO_MISSION: &str = "scenario.no_mission";
    /// A mission was started with parameters it does not have, or
    /// without one it needs.
    pub const BAD_PARAMS: &str = "scenario.bad_params";
    /// No checkpoint to go back to.
    pub const NO_CHECKPOINT: &str = "scenario.no_checkpoint";
    /// The setting is locked in the scenario being played.
    pub const LOCKED: &str = "setting.locked";
    /// The value is not one the setting takes.
    pub const BAD_SETTING: &str = "setting.bad_value";
    /// An answer was given to a setting the scenario does not ask about.
    pub const NOT_AN_OPTION: &str = "setting.not_an_option";
}

/// Every reason of scenarios, with what it means: for the generated
/// reference.
pub const SCENARIO_REASONS: &[ReasonInfo] = &[
    ReasonInfo {
        code: why::NO_GAME,
        doc: "No game is being played: the simulation was not begun from a scenario. Nothing was done.",
    },
    ReasonInfo {
        code: why::ENDED,
        doc: "The game has ended. Nothing was done. Values: `as`.",
    },
    ReasonInfo {
        code: why::BEGUN,
        doc: "`BeginScenario` was given to a simulation that is playing a game already. Values: `scenario`.",
    },
    ReasonInfo {
        code: why::UNKNOWN,
        doc: "No scenario of that id is loaded. Values: `scenario`.",
    },
    ReasonInfo {
        code: why::BAD_PLACE,
        doc: "The place to begin at is not one of the scenario's, no site answers its query, or nothing there can be stood on. Values: `place`.",
    },
    ReasonInfo {
        code: why::BAD_PARTY,
        doc: "A character of the party is not an entity that exists. Values: `entity`.",
    },
    ReasonInfo {
        code: why::BAD_WORLD,
        doc: "The simulation has no realm of the scenario's: its world was not made with `make_world`. Values: `realm`.",
    },
    ReasonInfo {
        code: why::NO_OBJECTIVE,
        doc: "No objective of that id in the scenario, or in the mission named. Values: `id`.",
    },
    ReasonInfo {
        code: why::NO_MISSION,
        doc: "No mission definition of that id, or no mission of that number in the game. Values: `mission`.",
    },
    ReasonInfo {
        code: why::BAD_PARAMS,
        doc: "A mission was started with a parameter it does not have, or without one it needs. Values: `mission`, `param`.",
    },
    ReasonInfo {
        code: why::NO_CHECKPOINT,
        doc: "There is no checkpoint of that name, or none at all, to go back to. Values: `name`.",
    },
    ReasonInfo {
        code: why::LOCKED,
        doc: "The setting may not be changed in the scenario being played: it is in the scenario's `locked`. Values: `key`, `scenario`.",
    },
    ReasonInfo {
        code: why::BAD_SETTING,
        doc: "The value is not one the setting takes, by its definition: a flag's truth value, one of a choice's options, a number in its range. Values: `key`.",
    },
    ReasonInfo {
        code: why::NOT_AN_OPTION,
        doc: "`BeginScenario` was given an answer for a setting the scenario does not ask about in `options`. Values: `key`.",
    },
];

const TAKER_SUBJECT: (&str, &str) = ("taker", "entity");

/// The notifications of scenarios, entered in every simulation's event
/// bus and in the engine's registry. Each is raised as a payload under
/// its name.
pub const SCENARIO_EVENTS: &[EventInfo] = &[
    EventInfo {
        name: SCENARIO_STARTED,
        subjects: &[("self", "entity")],
        fields: &[
            ("scenario", "string", "The scenario's qualified id."),
            (
                "place",
                "int",
                "Which of its places the party began at, from 0.",
            ),
        ],
        doc: "A game began (13 §12): the party is placed, its settings and rules are in force and `start.do` has run. `self` is the first of the party.",
    },
    EventInfo {
        name: OBJECTIVE_CHANGED,
        subjects: &[TAKER_SUBJECT],
        fields: &[
            ("id", "string", "The objective's id."),
            (
                "state",
                "string",
                "What it is now: `active` (shown), `done` or `failed`.",
            ),
            (
                "mission",
                "int",
                "The number of the mission it is part of; 0 for one of the scenario's own.",
            ),
        ],
        doc: "An objective was shown, done or failed (13 §5.1). Its `on_done` or `on_failed` has run. `taker` is who it is for.",
    },
    EventInfo {
        name: MISSION_STARTED,
        subjects: &[TAKER_SUBJECT],
        fields: MISSION_FIELDS,
        doc: "A mission was started (13 §5.2): its objectives that wait for nothing are shown.",
    },
    EventInfo {
        name: MISSION_DONE,
        subjects: &[TAKER_SUBJECT],
        fields: MISSION_FIELDS,
        doc: "Every objective of a mission is done. Its `on_done` has run.",
    },
    EventInfo {
        name: MISSION_FAILED,
        subjects: &[TAKER_SUBJECT],
        fields: MISSION_FIELDS,
        doc: "An objective of a mission failed, its `failed` came to hold, or `fail_mission` was called. Its `on_failed` has run.",
    },
    EventInfo {
        name: MISSION_EXPIRED,
        subjects: &[TAKER_SUBJECT],
        fields: MISSION_FIELDS,
        doc: "A mission's `expires` ran out while it was active. Its `on_expired` has run.",
    },
    EventInfo {
        name: PARTY_GONE,
        subjects: &[("self", "entity")],
        fields: &[(
            "then",
            "string",
            "What follows, by the scenario's `on_party_gone`: `end`, `continue` or `restore`.",
        )],
        doc: "No directly controlled character of the party remains alive (13 §9.3). `self` is the last of them. Raised once, before what follows is done.",
    },
    EventInfo {
        name: CHECKPOINT_MADE,
        subjects: &[],
        fields: &[("name", "string", "The checkpoint's name.")],
        doc: "A rule asked for a checkpoint (13 §9.1). The application saves it at the next safe point, unless the game's saving policy is `none`.",
    },
    EventInfo {
        name: SCENARIO_ENDED,
        subjects: &[],
        fields: &[
            ("as", "string", "The outcome the ending names."),
            (
                "reason",
                "string",
                "Why, where an ending stands in for something refused (a `restore` with no checkpoint); else empty.",
            ),
        ],
        doc: "The game ended (13 §9.4). The application shows the summary and applies the saving policy.",
    },
    EventInfo {
        name: SETTING_CHANGED,
        subjects: &[],
        fields: &[
            ("key", "string", "The setting's key."),
            ("old", "any", "What it was; null if it was not set."),
            ("new", "any", "What it is now."),
        ],
        doc: "The `Setting` command, or a scenario as it began, changed a setting (04 §10).",
    },
];

const MISSION_FIELDS: &[(&str, &str, &str)] = &[
    (
        "mission",
        "string",
        "The mission definition's qualified id.",
    ),
    (
        "serial",
        "int",
        "The mission's number in this game, from 1.",
    ),
];

/// The hook `scenario_ending` (13 §13): runs before an ending takes
/// effect. A handler may change `as`, or cancel.
pub fn hook_types() -> Vec<EventType> {
    vec![
        EventType::new(SCENARIO_ENDING)
            .field(
                "as",
                FieldType::Str,
                "The outcome the ending names. A handler may change it.",
            )
            .hook(["as"])
            .doc("Runs before an ending takes effect (13 §9.4). A handler may change `as`, or cancel: the game goes on."),
    ]
}

/// The events and the hook of scenarios, for a simulation's bus: each
/// raised with a payload through `Sim::raise`.
pub fn event_types() -> Vec<EventType> {
    let mut types: Vec<EventType> = SCENARIO_EVENTS
        .iter()
        .map(|info| {
            let mut ty = EventType::new(info.name).doc(info.doc);
            for (name, kind) in info.subjects {
                ty = ty.subject(*name, *kind);
            }
            for (name, written, doc) in info.fields {
                ty = ty.field(
                    *name,
                    FieldType::parse(written).unwrap_or(FieldType::Any),
                    *doc,
                );
            }
            ty
        })
        .collect();
    types.extend(hook_types());
    types
}

/// Registers the definition types of scenarios with the loader and the
/// cross check of what they name.
pub fn register_defs(types: &mut DefTypes) {
    types
        .register::<SettingDef>()
        .register::<ScenarioDef>()
        .register::<MissionDef>()
        .add_cross_check(def::check_scenarios);
}

/// Enters the hook of scenarios in a registry, unless it is there.
/// Called while the engine's registry is built, before its properties.
pub fn register_types(registry: &mut Registry) {
    for hook in hook_types() {
        if registry.event_id(&hook.name).is_some() {
            continue;
        }
        let mut spec = clash_expr::EventSpec::new(hook.name.clone()).doc(hook.doc.clone());
        for (name, kind) in crate::event::CAUSE_SUBJECTS {
            if let Some(kind) = registry.kind_id(kind) {
                spec = spec.subject(name.trim_end_matches('?'), kind);
            }
        }
        for field in &hook.fields {
            spec = spec.field(field.name.clone(), clash_expr::Type::Str, field.doc.clone());
        }
        spec = spec.hook(hook.mutable.iter().cloned());
        let _ = registry.add_event(spec);
    }
}

impl crate::Sim {
    /// The game being played, if one is, as the simulation keeps it
    /// (13 §3).
    pub fn scenario(&self) -> &ScenarioState {
        &self.scenario
    }
}

// ---- What objectives watch -------------------------------------------------------

/// What a reactive condition is looked at again on.
#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub(crate) enum Watch {
    /// Nothing: it has no condition, or one that never changes.
    #[default]
    Nothing,
    /// These events.
    Events(BTreeSet<String>),
    /// Every event: it reads something no event is registered for.
    Every,
}

impl Watch {
    /// What a condition reads, as the registry says it changes.
    fn of(cond: Option<&Cond>, registry: &Registry) -> Watch {
        let Some(cond) = cond else {
            return Watch::Nothing;
        };
        let (codes, queries) = cond.parts();
        if queries {
            return Watch::Every;
        }
        let mut events = BTreeSet::new();
        for code in codes {
            for id in code.properties_read() {
                match registry.property(id) {
                    Some(property) if !property.spec.changed_by.is_empty() => {
                        events.extend(property.spec.changed_by.iter().cloned());
                    }
                    _ => return Watch::Every,
                }
            }
            for id in code.functions_read() {
                match registry
                    .function(id)
                    .and_then(|f| f.spec.changed_by.as_ref())
                {
                    Some(changed) => events.extend(changed.iter().cloned()),
                    None => return Watch::Every,
                }
            }
            if !code.variables_read().is_empty() {
                events.insert("var_changed".to_owned());
            }
        }
        if events.is_empty() {
            Watch::Nothing
        } else {
            Watch::Events(events)
        }
    }

    /// The union of two.
    fn join(&self, other: &Watch) -> Watch {
        match (self, other) {
            (Watch::Every, _) | (_, Watch::Every) => Watch::Every,
            (Watch::Nothing, other) | (other, Watch::Nothing) => other.clone(),
            (Watch::Events(a), Watch::Events(b)) => Watch::Events(a.union(b).cloned().collect()),
        }
    }

    /// Whether anything of what was delivered is watched.
    pub(crate) fn hit(&self, delivered: &BTreeSet<String>) -> bool {
        match self {
            Watch::Nothing => false,
            Watch::Every => !delivered.is_empty(),
            Watch::Events(events) => !events.is_disjoint(delivered),
        }
    }
}

/// What the objectives of one list watch, in order.
fn watches(objectives: &[ObjectiveDef], registry: &Registry) -> Vec<Watch> {
    objectives
        .iter()
        .map(|objective| {
            Watch::of(objective.failed.as_ref(), registry)
                .join(&Watch::of(objective.done.as_ref(), registry))
        })
        .collect()
}

/// What the scenarios and missions of the loaded definitions watch:
/// worked out once, when the simulation is set up. Not state.
#[derive(Clone, Debug, Default)]
pub(crate) struct Tables {
    /// For each scenario by qualified id: its objectives' watches, and
    /// its `if` endings'.
    pub(crate) scenarios: BTreeMap<String, (Vec<Watch>, Vec<Watch>)>,
    /// For each mission by qualified id: its objectives' watches, and its
    /// `failed`'s.
    pub(crate) missions: BTreeMap<String, (Vec<Watch>, Watch)>,
    /// Every event something watches.
    pub(crate) events: BTreeSet<String>,
    /// Whether something watches every event.
    pub(crate) every: bool,
}

impl Tables {
    /// The tables of a set of definitions, against the registry their
    /// expressions were compiled with.
    pub(crate) fn for_defs(defs: &Defs, registry: &Registry) -> Arc<Tables> {
        let mut tables = Tables::default();
        let note = |watch: &Watch, tables: &mut Tables| match watch {
            Watch::Nothing => {}
            Watch::Every => tables.every = true,
            Watch::Events(events) => tables.events.extend(events.iter().cloned()),
        };
        if let Some(scenarios) = defs.try_registry::<ScenarioDef>() {
            for (id, scenario) in scenarios.iter() {
                let objectives = watches(&scenario.objectives, registry);
                let ends: Vec<Watch> = scenario
                    .ends
                    .iter()
                    .map(|end| Watch::of(end.condition.as_ref(), registry))
                    .collect();
                for watch in objectives.iter().chain(&ends) {
                    note(watch, &mut tables);
                }
                tables
                    .scenarios
                    .insert(scenarios.name(id).to_owned(), (objectives, ends));
            }
        }
        if let Some(missions) = defs.try_registry::<MissionDef>() {
            for (id, mission) in missions.iter() {
                let objectives = watches(&mission.objectives, registry);
                let failed = Watch::of(mission.failed.as_ref(), registry);
                for watch in objectives.iter().chain([&failed]) {
                    note(watch, &mut tables);
                }
                tables
                    .missions
                    .insert(missions.name(id).to_owned(), (objectives, failed));
            }
        }
        Arc::new(tables)
    }
}
