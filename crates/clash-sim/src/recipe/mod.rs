//! Recipes, work and know-how (16 §8; packages G6 and G17): turning
//! what is at hand into what is needed.
//!
//! A **recipe** says what is used up, what tools must be at hand, what
//! know-how it needs, how much work it takes and what it gives. Work is
//! done at a rate, by a character's work action or by a machine alone,
//! and its completion is one scheduled wake-up that follows the rate
//! exactly (invariant G5; see [`work`]). One definition type covers
//! making by hand, at a bench, cooking at an oven, distilling, mending
//! and growing (decision D-095).
//!
//! | Part | Is |
//! |------|----|
//! | [`RecipeDef`], [`KnowHowDef`] | The `recipe` and `know_how` definition types |
//! | [`Process`] | A recipe under way: on the station's device, or on the worker by hand |
//! | [`work::start`], [`work::check`] | Starting or resuming one; whether it could be |
//! | [`recipe_options`] | What a character could make here, for the make screen |
//! | [`know_how`] | What a mind knows how to do: `teach`, `unlearn`, `learn_from`, `knows` |
//! | [`supply`] | Where inputs and tools are looked for |
//! | [`properties`], [`verbs()`], [`event_types`], [`RECIPE_REASONS`] | What rules read, call, listen for and are refused with |
//!
//! # Working by hand and at a station
//!
//! A character works through an action whose parameters are named
//! `recipe` and (optionally) `station`: the bundled `core:make`. When it
//! starts, the simulation's handler of `action_starting` refuses it with
//! the reason [`work::check`] gives against truth. Its held phase calls
//! the verb `work`, which starts or resumes the recipe; when the work is
//! done, the recipe completes and lets the held phase go. If the action
//! ends before (cancelled, interrupted, the worker gone), the work done
//! stays where it is kept, and starting the same recipe there again
//! goes on with it.
//!
//! # Not built here
//!
//! Networks (G3): a station draws what its recipe's `while` wants from
//! its own stores. Jobs (G8): unfinished work by hand stays on the
//! worker, and starting another recipe by hand gives it up. Resource
//! and account inputs, salvage and construction (16 §10).

pub mod defs;
pub mod know_how;
pub mod options;
pub mod supply;
pub mod work;


use clash_core::{Dur, EntityId, Real, impl_state_hash};
use clash_data::{Def, DefTypes, Text};
use clash_expr::{EventSpec, Registry, Type, Value};
use clash_time::{Wake, WakeId};
use serde::{Deserialize, Serialize};

use crate::Sim;
use crate::bus::{EventPayload, EventType, FieldSpec, FieldType, Handler};
use crate::event::{CAUSE_SUBJECTS, EventBody, SimEvent, Subject};
use crate::host::Bound;
use crate::item::ItemDef;
use crate::later::ActionId;
use crate::reason::{Reason, ReasonInfo};
use crate::registry::PropertyDef;
use crate::verb::VerbDef;
use crate::world_verbs::{ArgRow, arg, spec};

pub use defs::{
    BY_HAND, EmitsDef, InputDef, KnowHowDef, KnowHowId, OutputDef, RecipeDef, RecipeId,
    RepairsDef, ToolDef, WhileDef,
};
pub use know_how::KnowHow;
pub use options::{RecipeOption, recipe_options, stations_near};

/// Registers the `recipe` and `know_how` definition types.
pub fn register_defs(types: &mut DefTypes) {
    types.register::<RecipeDef>();
    types.register::<KnowHowDef>();
}

/// Enters the `recipe` and `know_how` definition types into an
/// expression registry if it lacks them.
pub fn register_types(registry: &mut Registry) {
    for (name, doc) in [
        (RecipeDef::TYPE, RecipeDef::DOC),
        (KnowHowDef::TYPE, KnowHowDef::DOC),
    ] {
        if registry.def_type_id(name).is_none() {
            let _ = registry.add_def_type(name, doc);
        }
    }
}

// ---- The component ---------------------------------------------------------------

/// A recipe under way (16 §8.2): the `Process` component of the station's
/// device, or of the worker for work by hand.
#[derive(Clone, PartialEq, Debug, Serialize, Deserialize)]
pub struct Process {
    /// The recipe.
    recipe: RecipeId,
    /// Who works it now, if anyone does.
    worker: Option<EntityId>,
    /// The worker's action that its completion lets go.
    action: Option<ActionId>,
    /// The station's device, for work at one.
    station: Option<EntityId>,
    /// The work done, in seconds at a rate of 1, at `mark`.
    done: Real,
    /// The work it takes.
    needed: Real,
    /// The rate it works at now.
    rate: Real,
    /// The share of its `while` its stores give now.
    supply: Real,
    /// The own time of `clock` when `done` was recorded.
    mark: Dur,
    /// The entity whose time the work is done in: the worker while one
    /// works, else the station.
    clock: EntityId,
    /// Its one wake-up.
    wake: Option<WakeId>,
    /// Why it does not advance, while it does not.
    stalled: Option<String>,
    /// The item it mends, chosen when it began.
    mends: Option<EntityId>,
}

impl_state_hash!(Process {
    recipe,
    worker,
    action,
    station,
    done,
    needed,
    rate,
    supply,
    mark,
    clock,
    wake,
    stalled,
    mends
});

impl Process {
    /// The recipe.
    pub fn recipe(&self) -> RecipeId {
        self.recipe
    }

    /// Who works it now.
    pub fn worker(&self) -> Option<EntityId> {
        self.worker
    }

    /// The share of its work done when it was last recorded, from 0 to 1.
    pub fn recorded_progress(&self) -> Real {
        if self.needed > Real::ZERO {
            (self.done / self.needed).min(Real::ONE)
        } else {
            Real::ONE
        }
    }

    /// The rate it works at now.
    pub fn rate(&self) -> Real {
        self.rate
    }

    /// Why it does not advance, while it does not.
    pub fn stalled(&self) -> Option<&str> {
        self.stalled.as_deref()
    }
}

/// The share of a process's work done now, from 0 to 1: what was
/// recorded and what its rate has done since. Invariant G5.
pub fn progress(sim: &Sim, holder: EntityId) -> Option<Real> {
    let process = sim.components.get::<Process>(holder)?;
    let elapsed = (sim.own_time(process.clock) - process.mark)
        .as_secs_real()
        .max(Real::ZERO);
    let done = (process.done + process.rate * elapsed).min(process.needed);
    Some(if process.needed > Real::ZERO {
        done / process.needed
    } else {
        Real::ONE
    })
}

/// Whether an entity keeps a recipe under way.
pub fn has_process(sim: &Sim, entity: EntityId) -> bool {
    sim.components.get::<Process>(entity).is_some()
}

pub(crate) use work::{bring_up_to_date, drawn_since, station_changed};

// ---- Names -----------------------------------------------------------------------

/// A recipe's definition.
pub fn recipe_def(sim: &Sim, id: RecipeId) -> Option<&RecipeDef> {
    sim.defs.try_registry::<RecipeDef>()?.try_get(id)
}

/// A recipe's qualified id.
pub fn recipe_name(sim: &Sim, id: RecipeId) -> String {
    sim.defs
        .try_registry::<RecipeDef>()
        .filter(|registry| id.index() < registry.len())
        .map_or_else(|| id.raw().to_string(), |registry| registry.name(id).to_owned())
}

/// What a reader calls a recipe: its `name`, else its id.
pub fn recipe_label(sim: &Sim, id: RecipeId) -> String {
    let name = recipe_name(sim, id);
    recipe_def(sim, id).map_or(name.clone(), |def| label(sim, def.name.as_ref(), &name))
}

/// A text rendered for no one in particular, or the bare part of an id
/// where there is none.
pub(crate) fn label(sim: &Sim, text: Option<&Text>, qualified: &str) -> String {
    text.map(|text| {
        let mut host = sim.host(Bound::new());
        text.render_on(&sim.defs, &mut host)
    })
    .filter(|text| !text.is_empty())
    .unwrap_or_else(|| {
        qualified
            .split_once(':')
            .map_or(qualified, |(_, bare)| bare)
            .to_owned()
    })
}

/// What a reader calls an item definition.
pub(crate) fn item_def_name(sim: &Sim, id: crate::item::ItemId) -> String {
    let Some(registry) = sim.defs.try_registry::<ItemDef>() else {
        return String::new();
    };
    let Some(def) = registry.try_get(id) else {
        return String::new();
    };
    label(sim, def.name.as_ref(), registry.name(id))
}

/// The recipe a value names: an id of the type, or its qualified id.
pub fn recipe_of(sim: &Sim, value: &Value) -> Option<RecipeId> {
    match value {
        Value::Id(id) if sim.exprs.registry().def_type_id(RecipeDef::TYPE) == Some(id.ty) => {
            u16::try_from(id.index).ok().map(RecipeId::new)
        }
        Value::Str(name) => sim.defs.try_registry::<RecipeDef>()?.lookup(name),
        _ => None,
    }
}

// ---- Reasons -----------------------------------------------------------------------

/// The stable codes of the reasons recipes and know-how refuse for.
pub mod codes {
    /// No recipe has that id. Values: `recipe`.
    pub const UNKNOWN: &str = "recipe.unknown";
    /// It needs a station there is none of. Values: `recipe`, `at`.
    pub const NO_STATION: &str = "recipe.no_station";
    /// The station is not within a step. Values: `recipe`.
    pub const OUT_OF_REACH: &str = "recipe.out_of_reach";
    /// Something else is under way at the station, or someone else
    /// works it. Values: `recipe`, `other`.
    pub const BUSY: &str = "recipe.busy";
    /// Neither the worker nor the station has the know-how. Values:
    /// `recipe`, `know_how`.
    pub const UNKNOWN_HOW: &str = "recipe.unknown_how";
    /// The station does not work in its mode now. Values: `recipe`,
    /// `mode`.
    pub const STATION_IDLE: &str = "recipe.station_idle";
    /// Its `needs` does not hold. Values: `recipe`.
    pub const NEEDS: &str = "recipe.needs";
    /// An input is short. Values: `recipe`, `what`, `need`, `have`.
    pub const MISSING_INPUT: &str = "recipe.missing_input";
    /// A tool is missing. Values: `recipe`, `what`.
    pub const MISSING_TOOL: &str = "recipe.missing_tool";
    /// Nothing at hand is damaged and of the tag it mends. Values:
    /// `recipe`, `tag`.
    pub const NOTHING_TO_MEND: &str = "recipe.nothing_to_mend";
    /// Nothing at hand has room for a substance it gives. Values:
    /// `recipe`.
    pub const NO_ROOM: &str = "recipe.no_room";
    /// A verb's argument is missing or of the wrong kind. Values:
    /// `verb`, `arg`.
    pub const BAD_ARGUMENT: &str = "recipe.bad_argument";
    /// No know-how has that id. Values: `know_how`.
    pub const UNKNOWN_KNOW_HOW: &str = "know_how.unknown";
    /// What it needs is not known yet. Values: `know_how`, `needs`.
    pub const NEEDS_FIRST: &str = "know_how.needs";
    /// The entity has no mind to know it with. Values: `entity`.
    pub const NO_MIND: &str = "know_how.no_mind";
    /// The item teaches nothing. Values: `item`.
    pub const TEACHES_NOTHING: &str = "know_how.teaches_nothing";
}

/// The reasons of recipes and know-how, for the generated reference.
pub const RECIPE_REASONS: &[ReasonInfo] = &[
    ReasonInfo {
        code: codes::UNKNOWN,
        doc: "The id is not a `recipe` definition. Values: `recipe`.",
    },
    ReasonInfo {
        code: codes::NO_STATION,
        doc: "The recipe is made at a station whose tag it names in `at`, and none was given, or what was given carries none of them. Values: `recipe`, `at`.",
    },
    ReasonInfo {
        code: codes::OUT_OF_REACH,
        doc: "The station is not in the worker's cell or one of the eight round it on its storey. Values: `recipe`.",
    },
    ReasonInfo {
        code: codes::BUSY,
        doc: "Another recipe is under way at the station, or someone else is working this one there. Values: `recipe`, `other`.",
    },
    ReasonInfo {
        code: codes::UNKNOWN_HOW,
        doc: "The recipe needs know-how that the worker does not have and the station does not provide. Values: `recipe`, `know_how`.",
    },
    ReasonInfo {
        code: codes::STATION_IDLE,
        doc: "The station's `speed` does not apply in the mode it is in: a cold oven. Values: `recipe`, `mode`.",
    },
    ReasonInfo {
        code: codes::NEEDS,
        doc: "The recipe's `needs` does not hold for the worker; the message is its `fail_text`. Values: `recipe`.",
    },
    ReasonInfo {
        code: codes::MISSING_INPUT,
        doc: "Less of an input is at hand than the recipe uses. Values: `recipe`, `what`, `need`, `have` (a count, or millilitres).",
    },
    ReasonInfo {
        code: codes::MISSING_TOOL,
        doc: "A tool the recipe needs is not at hand. Values: `recipe`, `what`.",
    },
    ReasonInfo {
        code: codes::NOTHING_TO_MEND,
        doc: "The recipe `repairs` something and nothing at hand of its tag is damaged. Values: `recipe`, `tag`.",
    },
    ReasonInfo {
        code: codes::NO_ROOM,
        doc: "The recipe gives a substance and nothing at hand has room to take it. Values: `recipe`.",
    },
    ReasonInfo {
        code: codes::BAD_ARGUMENT,
        doc: "A verb of recipes or know-how was given an argument that is missing or of the wrong kind. Values: `verb`, `arg`.",
    },
    ReasonInfo {
        code: codes::UNKNOWN_KNOW_HOW,
        doc: "The id is not a `know_how` definition. Values: `know_how`.",
    },
    ReasonInfo {
        code: codes::NEEDS_FIRST,
        doc: "`teach` of know-how whose `needs` are not all known yet. Values: `know_how`, `needs` (the first not known).",
    },
    ReasonInfo {
        code: codes::NO_MIND,
        doc: "Know-how is held by a mind, and the entity has none. Values: `entity`.",
    },
    ReasonInfo {
        code: codes::TEACHES_NOTHING,
        doc: "`learn_from` an item whose definition `teaches` nothing. Values: `item`.",
    },
];

fn bad_argument(verb: &str, name: &str, wanted: &str) -> Reason {
    Reason::new(
        codes::BAD_ARGUMENT,
        format!("`{verb}`: the argument `{name}` must be {wanted}"),
    )
    .with("verb", verb)
    .with("arg", name)
}

// ---- Events ------------------------------------------------------------------------

/// The hook raised before a recipe begins or resumes.
pub const RECIPE_STARTING: &str = "recipe_starting";
/// A recipe began or resumed.
pub const RECIPE_STARTED: &str = "recipe_started";
/// A recipe under way stopped advancing.
pub const RECIPE_STALLED: &str = "recipe_stalled";
/// The hook raised before a recipe's outputs are made.
pub const RECIPE_COMPLETING: &str = "recipe_completing";
/// A recipe completed.
pub const RECIPE_COMPLETED: &str = "recipe_completed";
/// A mind gained know-how.
pub const KNOW_HOW_GAINED: &str = "know_how_gained";
/// A mind lost know-how.
pub const KNOW_HOW_LOST: &str = "know_how_lost";

struct EventRow {
    name: &'static str,
    doc: &'static str,
    subjects: &'static [(&'static str, &'static str)],
    fields: &'static [(&'static str, &'static str, &'static str)],
    mutable: &'static [&'static str],
    hook: bool,
}

const WORK_SUBJECTS: &[(&str, &str)] = &[("self", "entity"), ("station?", "entity")];
const RECIPE_FIELD: (&str, &str, &str) = ("recipe", "string", "The recipe's qualified id.");

const EVENTS: &[EventRow] = &[
    EventRow {
        name: RECIPE_STARTING,
        doc: "Runs before a recipe begins or resumes (16 §8.2), by hand or at a station, by a worker or a machine alone, once it could be. `self` is the worker, or the machine; `station` the station's device. A handler may cancel with a reason, which refuses it.",
        subjects: WORK_SUBJECTS,
        fields: &[RECIPE_FIELD],
        mutable: &[],
        hook: true,
    },
    EventRow {
        name: RECIPE_STARTED,
        doc: "A recipe began, its inputs used up, or resumed where it was left (16 §8.2).",
        subjects: WORK_SUBJECTS,
        fields: &[
            RECIPE_FIELD,
            ("resumed", "bool", "Whether it goes on with work already done."),
            ("work", "real", "How much work it takes in all, in seconds at a rate of 1."),
        ],
        mutable: &[],
        hook: false,
    },
    EventRow {
        name: RECIPE_STALLED,
        doc: "A recipe under way stopped advancing (16 §8.2): its worker stopped, its station does not work in its mode, a store it draws on ran dry, its condition on the place does not hold, or its worker's rate came to nothing. Its work done is kept.",
        subjects: WORK_SUBJECTS,
        fields: &[
            RECIPE_FIELD,
            ("why", "string", "`stopped`, `station`, `supply`, `condition` or `worker`."),
        ],
        mutable: &[],
        hook: false,
    },
    EventRow {
        name: RECIPE_COMPLETING,
        doc: "Runs as a recipe's work is done, before its outputs are made (16 §8.2). A handler may set `spoiled`, or cancel, and nothing is made: the inputs are gone all the same.",
        subjects: WORK_SUBJECTS,
        fields: &[
            RECIPE_FIELD,
            ("spoiled", "bool", "Whether it comes to nothing. Mutable."),
        ],
        mutable: &["spoiled"],
        hook: true,
    },
    EventRow {
        name: RECIPE_COMPLETED,
        doc: "A recipe's work is done (16 §8.2): its outputs are made, what it mends is mended and its `result` has run, unless it was spoiled.",
        subjects: WORK_SUBJECTS,
        fields: &[
            RECIPE_FIELD,
            ("spoiled", "bool", "Whether it came to nothing."),
        ],
        mutable: &[],
        hook: false,
    },
    EventRow {
        name: KNOW_HOW_GAINED,
        doc: "A mind came to know something (16 §8.5).",
        subjects: &[("self", "entity")],
        fields: &[
            ("know_how", "string", "The know-how's qualified id."),
            ("how", "string", "`template`, `taught` or `read`, or what the rule that taught it said."),
        ],
        mutable: &[],
        hook: false,
    },
    EventRow {
        name: KNOW_HOW_LOST,
        doc: "A mind no longer knows something (16 §8.5 `unlearn`).",
        subjects: &[("self", "entity")],
        fields: &[
            ("know_how", "string", "The know-how's qualified id."),
            ("how", "string", "`unlearned`."),
        ],
        mutable: &[],
        hook: false,
    },
];

/// The event types of recipes and know-how, for the simulation's bus.
pub fn event_types() -> Vec<EventType> {
    EVENTS
        .iter()
        .map(|row| {
            let mut ty = EventType::new(row.name).doc(row.doc);
            for (name, kind) in row.subjects {
                ty = ty.subject(*name, *kind);
            }
            ty.fields = row
                .fields
                .iter()
                .map(|(name, written, doc)| FieldSpec {
                    name: (*name).to_owned(),
                    ty: (*written).to_owned(),
                    check: Some(FieldType::parse(written).unwrap_or(FieldType::Any)),
                    doc: (*doc).to_owned(),
                })
                .collect();
            if row.hook {
                ty = ty.hook(row.mutable.iter().copied());
            }
            ty
        })
        .collect()
}

/// Enters the events of recipes and know-how into an expression
/// registry, leaving alone those it already holds.
pub fn register_events(registry: &mut Registry) {
    for row in EVENTS {
        if registry.event_id(row.name).is_some() {
            continue;
        }
        let mut spec = EventSpec::new(row.name).doc(row.doc);
        for (name, kind) in row.subjects.iter().chain(CAUSE_SUBJECTS) {
            if let Some(kind) = registry.kind_id(kind) {
                spec = spec.subject(name.trim_end_matches('?'), kind);
            }
        }
        for (name, written, doc) in row.fields {
            let ty = registry.parse_type(written).unwrap_or(Type::Any);
            spec = spec.field(*name, ty, *doc);
        }
        if row.hook {
            spec = spec.hook(row.mutable.iter().copied());
        }
        let _ = registry.add_event(spec);
    }
}

// ---- The work action ---------------------------------------------------------------

/// The name of the action parameter that makes an action a recipe's work
/// action: the recipe.
pub const RECIPE_PARAM: &str = "recipe";
/// The name of the parameter that holds the station: an element, an
/// entity, or null by hand.
pub const STATION_PARAM: &str = "station";

/// Registers what makes a work action begin only when its recipe could:
/// a handler of `action_starting` that, for an action whose parameter
/// `recipe` names a recipe, refuses the action with the reason
/// [`work::check`] gives. Called when the simulation is set up and after
/// a load.
pub(crate) fn bind(sim: &mut Sim) {
    let _ = sim.on(crate::action::ACTION_STARTING, Handler::new(action_starting));
}

fn action_starting(sim: &mut Sim, delivery: &mut crate::bus::Delivery<'_>) {
    let Some(params) = delivery.field("params") else {
        return;
    };
    let params = sim.exprs.field_value(&params);
    let Some(map) = params.as_map() else {
        return;
    };
    let Some(recipe) = map.get(RECIPE_PARAM).and_then(|value| recipe_of(sim, value)) else {
        return;
    };
    let station = map
        .get(STATION_PARAM)
        .and_then(|value| options::station_of(sim, value));
    let Some(Subject::Entity(worker)) = delivery.event().subject("self") else {
        return;
    };
    if let Err(reason) = work::check(sim, Some(worker), recipe, station) {
        let _ = delivery.cancel(reason);
    }
}

/// What recipes and know-how do about an event the kernel has delivered:
/// work stops when its worker's action ends; a machine looks at what was
/// put into it; a new entity learns what its templates list.
pub(crate) fn on_event(sim: &mut Sim, event: &SimEvent) {
    match &event.body {
        EventBody::Act(
            crate::action::ActEvent::ActionFinished { actor, id, .. }
            | crate::action::ActEvent::ActionCancelled { actor, id, .. }
            | crate::action::ActEvent::ActionInterrupted { actor, id, .. }
            | crate::action::ActEvent::ActionAbandoned { actor, id, .. },
        ) => work::action_ended(sim, *actor, *id, event.cause),
        EventBody::EntityRelocated {
            to: crate::entity::Place::In(container),
            ..
        } => work::put_into(sim, *container, event.cause),
        _ if event.name() == "entity_spawned" => {
            if let Some(entity) = event.subject("self").and_then(|subject| subject.entity()) {
                know_how::from_templates(sim, entity, event.cause);
            }
        }
        _ => {}
    }
}

/// The handler of the `Stepper` wake-up domain: a recipe's wake-up.
pub(crate) fn on_wake(sim: &mut Sim, id: WakeId, wake: Wake) {
    if wake.arg == work::WAKE_RECIPE {
        work::on_wake(sim, id, wake);
    } else {
        sim.fault(
            Reason::new(
                crate::reason::codes::ADVANCE_UNHANDLED_WAKE,
                "a wake-up of the stepper domain was due that no recipe owns",
            )
            .with("arg", wake.arg as i64),
        );
    }
}

// ---- Properties --------------------------------------------------------------------

/// The recipe and the station a property's arguments name.
fn recipe_and_station(sim: &Sim, args: &[Value]) -> Option<(RecipeId, Option<crate::interaction::Target>)> {
    let recipe = recipe_of(sim, args.first()?)?;
    let station = args.get(1).and_then(|value| options::station_of(sim, value));
    Some((recipe, station))
}

/// The properties of recipes and know-how (16 §8, §14). None is safe for
/// belief: a character reads its own.
pub fn properties() -> Vec<PropertyDef> {
    vec![
        PropertyDef::new(
            "entity",
            "recipe_options",
            "list<map<any>>",
            "What the entity, as it believes, could make here (16 §8; 12 §10.7 Make): every recipe done by hand, and every recipe of each station within a step, in order of recipe. Each is a map: `recipe` (its id), `id` (its qualified id), `name`, `station` (the element or device, null by hand), `where` (`hand`, `here` or a compass direction), `available`, `resumes` (it goes on with work under way there), `why` (the refusal's sentence, else empty), `work` (seconds at a rate of 1), `inputs` (a list of maps: `what`, `need`, `have`, `text`), `takes` and `gives` (texts).",
            |sim, subject, _| match subject {
                Subject::Entity(id) => options::options_value(sim, id),
                _ => Value::list([]),
            },
        ),
        PropertyDef::new(
            "entity",
            "can_make",
            "bool",
            "Whether the entity, as it believes, could start or resume a recipe now, by hand or at a station: what a work action `requires`.",
            |sim, subject, args| match (subject, recipe_and_station(sim, args)) {
                (Subject::Entity(id), Some((recipe, station))) => {
                    Value::Bool(work::check(sim, Some(id), recipe, station).is_ok())
                }
                _ => Value::Bool(false),
            },
        )
        .arg("recipe", "any", "The recipe: its id, or its qualified id as a text.")
        .arg("station", "any", "The station: an element or a device; null by hand."),
        PropertyDef::new(
            "entity",
            "make_fault",
            "string?",
            "Why the entity, as it believes, could not start a recipe now: null if it could. A refusal's sentence, for a work action's `fail_text`.",
            |sim, subject, args| match (subject, recipe_and_station(sim, args)) {
                (Subject::Entity(id), Some((recipe, station))) => match work::check(sim, Some(id), recipe, station) {
                    Ok(_) => Value::Null,
                    Err(reason) => Value::text(&reason.message),
                },
                _ => Value::text("there is no such recipe"),
            },
        )
        .arg("recipe", "any", "The recipe: its id, or its qualified id as a text.")
        .arg("station", "any", "The station: an element or a device; null by hand."),
        PropertyDef::new(
            "entity",
            "making",
            "map<any>?",
            "The recipe under way that the entity keeps (16 §8.2): a station's, or work by hand on a worker. A map: `recipe` (its qualified id), `name`, `progress` (the share done now, 0 to 1), `rate`, `worker` (or null) and `why` (why it does not advance, else empty). Null when there is none.",
            |sim, subject, _| {
                let Subject::Entity(id) = subject else {
                    return Value::Null;
                };
                let Some(process) = sim.components.get::<Process>(id) else {
                    return Value::Null;
                };
                Value::map([
                    ("recipe", Value::text(recipe_name(sim, process.recipe))),
                    ("name", Value::text(recipe_label(sim, process.recipe))),
                    ("progress", Value::Real(progress(sim, id).unwrap_or(Real::ZERO))),
                    ("rate", Value::Real(process.rate)),
                    ("worker", process.worker.map_or(Value::Null, |worker| sim.subject_value(Subject::Entity(worker)))),
                    ("why", Value::text(process.stalled.clone().unwrap_or_default())),
                ])
            },
        )
        .changed_by(&[RECIPE_STARTED, RECIPE_STALLED, RECIPE_COMPLETED]),
        PropertyDef::new(
            "entity",
            "knows_how",
            "bool",
            "Whether the entity's mind knows something (16 §8.5): what a recipe's `know_how` and any condition may ask.",
            |sim, subject, args| match (subject, args.first().and_then(|value| know_how::know_how_of(sim, value))) {
                (Subject::Entity(id), Some(what)) => Value::Bool(know_how::knows(sim, id, what)),
                _ => Value::Bool(false),
            },
        )
        .arg("what", "id<know_how>", "The know-how.")
        .changed_by(&[KNOW_HOW_GAINED, KNOW_HOW_LOST]),
        PropertyDef::new(
            "entity",
            "know_how",
            "list<string>",
            "The qualified ids of what the entity's mind knows, in order of id.",
            |sim, subject, _| match subject {
                Subject::Entity(id) => Value::list(
                    know_how::known_by(sim, id)
                        .into_iter()
                        .map(|known| Value::text(know_how::know_how_name(sim, known))),
                ),
                _ => Value::list([]),
            },
        )
        .changed_by(&[KNOW_HOW_GAINED, KNOW_HOW_LOST]),
        PropertyDef::new(
            "entity",
            "know_how_list",
            "list<map<any>>",
            "Every piece of know-how there is, as the entity stands to it, for the know-how screen (12 §10.7): one map each, in order of id: `id`, `name`, `group`, `known`, `ready` (known, or all it needs is known), `makes` (the names of the recipes that need it) and `needs` (the names of what it needs).",
            |sim, subject, _| match subject {
                Subject::Entity(id) => options::know_how_value(sim, id),
                _ => Value::list([]),
            },
        ),
    ]
}

// ---- Verbs -------------------------------------------------------------------------

fn entity_at(sim: &Sim, verb: &str, call: &crate::rule::VerbCall<'_>, index: usize, name: &str) -> Result<EntityId, Reason> {
    match sim.subject_of(arg(call, index)) {
        Some(Subject::Entity(id)) => Ok(id),
        _ => Err(bad_argument(verb, name, "an entity")),
    }
}

fn recipe_at(sim: &Sim, verb: &str, call: &crate::rule::VerbCall<'_>, index: usize) -> Result<RecipeId, Reason> {
    recipe_of(sim, arg(call, index)).ok_or_else(|| bad_argument(verb, "recipe", "a recipe"))
}

fn know_how_at(sim: &Sim, verb: &str, call: &crate::rule::VerbCall<'_>, index: usize) -> Result<KnowHowId, Reason> {
    know_how::know_how_of(sim, arg(call, index)).ok_or_else(|| bad_argument(verb, "what", "a piece of know-how"))
}

const WHO_WORKS: ArgRow = ("who", "entity", false, "Who works.");
const RECIPE_ARG: ArgRow = (
    "recipe",
    "any",
    false,
    "The recipe: its id, or its qualified id as a text.",
);

/// The verbs of recipes and know-how, for [`crate::verb::engine_verbs`].
pub fn verbs() -> Vec<VerbDef> {
    vec![
        VerbDef {
            name: "work",
            build: |registry| {
                spec(
                    registry,
                    "work",
                    "Starts a recipe, or goes on with it where it is under way, by a worker (16 §8.2): what a work action's held phase does. Checks as `can_make` does against truth, promotes the station, raises the hook `recipe_starting`, uses up the inputs of a new start and raises `recipe_started`. When the work is done the recipe completes and the worker's held action is let go; if the action ends first the work done is kept. Refused with the reasons of `recipe.*`; the worker's held action is then interrupted.",
                    &[
                        WHO_WORKS,
                        RECIPE_ARG,
                        ("at", "any", true, "The station: an element or a device. Left out, by hand."),
                    ],
                )
            },
            run: |sim, call, cause| {
                let who = entity_at(sim, "work", call, 0, "who")?;
                let recipe = recipe_at(sim, "work", call, 1)?;
                let station = options::station_of(sim, arg(call, 2));
                match work::start(sim, Some(who), recipe, station, cause) {
                    Ok(_) => Ok(Value::Null),
                    Err(reason) => {
                        if let Some(action) = work::held_action(sim, who) {
                            sim.interrupt_action(action, reason.clone(), cause);
                        }
                        Err(reason)
                    }
                }
            },
        },
        VerbDef {
            name: "start_recipe",
            build: |registry| {
                spec(
                    registry,
                    "start_recipe",
                    "Starts a recipe at a machine with nobody working it (16 §8.2), from what is in its own holds and with the know-how it provides: what a machine's rules call. It advances while the machine's `auto` applies. Refused with the reasons of `recipe.*`.",
                    &[
                        ("at", "any", false, "The machine: an element or a device."),
                        RECIPE_ARG,
                    ],
                )
            },
            run: |sim, call, cause| {
                let station = options::station_of(sim, arg(call, 0))
                    .ok_or_else(|| bad_argument("start_recipe", "at", "a device or an element"))?;
                let recipe = recipe_at(sim, "start_recipe", call, 1)?;
                work::start(sim, None, recipe, Some(station), cause)?;
                Ok(Value::Null)
            },
        },
        VerbDef {
            name: "cancel_recipe",
            build: |registry| {
                spec(
                    registry,
                    "cancel_recipe",
                    "Gives up the recipe under way at a station, or a worker's work by hand: what was used up is lost, and `recipe_stalled` is raised with `why` `cancelled`. Gives whether there was one.",
                    &[("at", "any", false, "The station, or the worker.")],
                )
            },
            run: |sim, call, cause| {
                let Some(target) = options::station_of(sim, arg(call, 0)) else {
                    return Err(bad_argument("cancel_recipe", "at", "a station or a worker"));
                };
                let holder = match target {
                    crate::interaction::Target::Entity(id) => Some(id),
                    other => crate::device::device_now(sim, other),
                };
                let Some(holder) = holder else {
                    return Ok(Value::Bool(false));
                };
                let Some(process) = sim.components.remove::<Process>(holder) else {
                    return Ok(Value::Bool(false));
                };
                if let Some(wake) = process.wake {
                    sim.cancel_wake(wake);
                }
                let actor = process.worker.unwrap_or(holder);
                let name = recipe_name(sim, process.recipe);
                let mut payload = EventPayload::new(RECIPE_STALLED)
                    .subject("self", Subject::Entity(actor))
                    .with("recipe", crate::bus::FieldValue::text(&name))
                    .with("why", crate::bus::FieldValue::text("cancelled"));
                if let Some(station) = process.station {
                    payload = payload.subject("station", Subject::Entity(station));
                }
                let _ = sim.raise(crate::event::Where::Entity(actor), cause, payload);
                if let (Some(worker), Some(action)) = (process.worker, process.action) {
                    let _ = sim.cancel_action(worker, action, false, cause);
                }
                crate::device::settle(sim, holder, cause);
                Ok(Value::Bool(true))
            },
        },
        VerbDef {
            name: "teach",
            build: |registry| {
                spec(
                    registry,
                    "teach",
                    "Teaches an entity's mind a piece of know-how (16 §8.5) and raises `know_how_gained`. Gives whether it was new. Refused with `know_how.needs` while something it needs is not known, and `know_how.no_mind`.",
                    &[
                        ("who", "entity", false, "Who learns."),
                        ("what", "id<know_how>", false, "The know-how."),
                        ("how", "string", true, "What `know_how_gained` says of how; `taught` if left out."),
                    ],
                )
            },
            run: |sim, call, cause| {
                let who = entity_at(sim, "teach", call, 0, "who")?;
                let what = know_how_at(sim, "teach", call, 1)?;
                let how = arg(call, 2).as_str().unwrap_or("taught").to_owned();
                know_how::teach(sim, who, what, &how, cause).map(Value::Bool)
            },
        },
        VerbDef {
            name: "unlearn",
            build: |registry| {
                spec(
                    registry,
                    "unlearn",
                    "Takes a piece of know-how from an entity's mind (16 §8.5) and raises `know_how_lost`. Gives whether it was known.",
                    &[
                        ("who", "entity", false, "Who forgets."),
                        ("what", "id<know_how>", false, "The know-how."),
                    ],
                )
            },
            run: |sim, call, cause| {
                let who = entity_at(sim, "unlearn", call, 0, "who")?;
                let what = know_how_at(sim, "unlearn", call, 1)?;
                know_how::unlearn(sim, who, what, cause).map(Value::Bool)
            },
        },
        VerbDef {
            name: "learn_from",
            build: |registry| {
                spec(
                    registry,
                    "learn_from",
                    "Teaches an entity what an item `teaches` (16 §8.5: reading a book), each as `teach` does, passing by what cannot be taught yet. Gives how many were new. Refused with `know_how.teaches_nothing`, and with the first refusal of `teach` when none could be taught.",
                    &[
                        ("who", "entity", false, "Who reads."),
                        ("from", "entity", false, "The item."),
                    ],
                )
            },
            run: |sim, call, cause| {
                let who = entity_at(sim, "learn_from", call, 0, "who")?;
                let from = entity_at(sim, "learn_from", call, 1, "from")?;
                know_how::learn_from(sim, who, from, cause).map(Value::Int)
            },
        },
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_reason_is_listed_once() {
        let mut codes: Vec<&str> = RECIPE_REASONS.iter().map(|info| info.code).collect();
        let count = codes.len();
        codes.sort_unstable();
        codes.dedup();
        assert_eq!(codes.len(), count);
    }

    #[test]
    fn every_event_and_verb_registers() {
        let registry = crate::engine_registry();
        for ty in event_types() {
            assert!(registry.event_id(&ty.name).is_some(), "{}", ty.name);
        }
        for verb in verbs() {
            assert!(registry.verb_id(verb.name).is_some(), "{}", verb.name);
        }
    }

    #[test]
    fn a_process_counts_its_progress() {
        let process = Process {
            recipe: RecipeId::new(0),
            worker: None,
            action: None,
            station: None,
            done: Real::of(3.0),
            needed: Real::of(12.0),
            rate: Real::ONE,
            supply: Real::ONE,
            mark: Dur::ZERO,
            clock: EntityId::from_bits(0),
            wake: None,
            stalled: None,
            mends: None,
        };
        assert_eq!(process.recorded_progress(), Real::of(0.25));
    }
}
