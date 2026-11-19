//! What the agents' layer offers rules and minds (09 §17; 06 §13).
//!
//! # Properties
//!
//! Of an `entity`, none safe for belief: read of oneself in a goal
//! (`self.routine_place`), and of anyone by a world rule or the
//! inspector. Of a `realm`, safe: the time of day is everyone's.
//!
//! | Property | Type | Is |
//! |----------|------|----|
//! | `entity.routine` | `string?` | The qualified id of the routine it keeps: the one `set_routine` gave it, else its mind's |
//! | `entity.routine_doing` | `string?` | What its routine has it doing now: the part's `doing` |
//! | `entity.routine_place` | `loc?` | Where its routine has it be now: its own spot at the first place the part names, its home if none answers. Null if it keeps no routine, or the place is in a tract that is not loaded |
//! | `entity.routine_places` | `list<loc>` | Every place the part names (for `pick: all`, every site), each its own spot, in order: for a beat between them. Only those loaded |
//! | `entity.routine_away` | `bool?` | Whether it is further from `routine_place` than the part's `within`. Null where `routine_place` is |
//! | `entity.resident` | `bool` | Whether it stands for a resident of a settlement (06 §12.2) |
//! | `entity.resident_home` | `loc?` | Its home as a resident: where generation asked for it |
//! | `entity.agent_doing` | `string?` | For an agent out of the world: what it is doing |
//! | `realm.time_of_day` | `real?` | Seconds since midnight by the realm's day (09 §14.2) |
//! | `realm.day` | `int?` | The day, counted from 1 when the realm's clock began |
//! | `realm.day_length` | `real?` | The length of the realm's day, in seconds of its clock |
//! | `realm.day_part` | `string?` | The name of the part of the day by its calendar; null with none |
//! | `realm.season` | `string?` | The name of the season by its calendar; null with none |
//!
//! # Verbs
//!
//! | Verb | Arguments | Does |
//! |------|-----------|------|
//! | `make_anonymous` | `who` | Makes a notable individual anonymous again (06 §12.2); an agent made so becomes nothing. Gives null. Refused with `agent.not_an_individual` |
//! | `set_routine` | `who`, `routine?` | Gives an entity a routine in place of its mind's; left out, takes it away. Gives null. Refused with `agent.unknown_routine` |
//!
//! # Events
//!
//! | Event | Subjects | Fields |
//! |-------|----------|--------|
//! | `agent_stored` | `self` | `doing` (`string`), `aim` (`string?`) |
//! | `agent_loaded` | `self` | `doing` (`string`) |
//! | `became_anonymous` | `self` | None |

use clash_core::Real;
use clash_expr::{Type, Value, VerbSpec};
use clash_world::{Cause, RealmId};

use super::calendar;
use super::reason as why;
use crate::Sim;
use crate::event::{EventInfo, Subject};
use crate::life::args;
use crate::reason::Reason;
use crate::registry::PropertyDef;
use crate::rule::VerbCall;
use crate::verb::VerbDef;

/// `agent_stored`: a notable resident's individual left the world with
/// its tract and is now an agent.
pub const AGENT_STORED: &str = "agent_stored";
/// `agent_loaded`: an agent came back into the world.
pub const AGENT_LOADED: &str = "agent_loaded";
/// `became_anonymous`: an individual was made anonymous again.
pub const BECAME_ANONYMOUS: &str = "became_anonymous";

/// The events of this module, for the bus, the registry and the
/// reference.
pub const AGENT_EVENTS: &[EventInfo] = &[
    EventInfo {
        name: AGENT_STORED,
        subjects: &[("self", "entity")],
        fields: &[
            (
                "doing",
                "string",
                "What it is doing as an agent: its routine's `doing` now, or `waiting`.",
            ),
            (
                "aim",
                "string?",
                "The qualified id of the goal it was pursuing, if not its routine's (09 §10.4).",
            ),
        ],
        doc: "A notable resident's individual left the world with its tract and is now an agent (09 §10.4): stored, placed by its routine, costing nothing between wake-ups.",
    },
    EventInfo {
        name: AGENT_LOADED,
        subjects: &[("self", "entity")],
        fields: &[(
            "doing",
            "string",
            "What its routine has it doing as it comes back.",
        )],
        doc: "An agent came back into the world (09 §10.4): where its routine puts it, or on its way between two places.",
    },
    EventInfo {
        name: BECAME_ANONYMOUS,
        subjects: &[("self", "entity")],
        fields: &[],
        doc: "A notable individual was made anonymous again by `make_anonymous` (06 §12.2): it will be forgotten when its tract unloads.",
    },
];

fn opt<T: Into<Value>>(value: Option<T>) -> Value {
    value.map_or(Value::Null, Into::into)
}

fn entity_of(sim: &Sim, subject: Subject) -> Option<clash_core::EntityId> {
    match subject {
        Subject::Entity(id) if sim.entities.contains(id) => Some(id),
        _ => None,
    }
}

fn realm_of(subject: Subject) -> Option<RealmId> {
    match subject {
        Subject::Realm(id) => Some(id),
        _ => None,
    }
}

fn seconds(dur: clash_core::Dur) -> Value {
    Value::Real(dur.as_secs_real())
}

/// The places a routine names now that are loaded.
fn loaded_places(sim: &Sim, entity: clash_core::EntityId) -> Vec<clash_world::Location> {
    super::routine_places(sim, entity)
        .into_iter()
        .filter(|loc| sim.cosmos.is_loaded(*loc))
        .collect()
}

/// The properties of this module.
pub fn properties() -> Vec<PropertyDef> {
    let p = PropertyDef::new;
    vec![
        p(
            "entity",
            "routine",
            "string?",
            "The qualified id of the routine it keeps (09 §10.2): the one `set_routine` gave it, else the one its mind names. Null if none.",
            |sim, subject, _| {
                opt(entity_of(sim, subject)
                    .and_then(|entity| super::routine_of(sim, entity))
                    .map(|(id, _)| super::routine::routine_name(sim, id)))
            },
        ),
        p(
            "entity",
            "routine_doing",
            "string?",
            "What its routine has it doing now: the `doing` of the part of the day it is at (09 §10.2). Null if it keeps no routine.",
            |sim, subject, _| {
                let Some(entity) = entity_of(sim, subject) else {
                    return Value::Null;
                };
                let Some((_, def)) = super::routine_of(sim, entity) else {
                    return Value::Null;
                };
                opt(super::whom_of(sim, entity)
                    .and_then(|whom| super::current_entry(sim, def, &whom).map(|(_, entry, _)| entry.doing.clone())))
            },
        ),
        p(
            "entity",
            "routine_place",
            "loc?",
            "Where its routine has it be now (09 §10.2): its own spot at the first place the part of the day names (on a `beat`, the one it has come to), or its home if none answers. Null if it keeps no routine, or if that place is in a tract that is not loaded, where it cannot walk.",
            |sim, subject, _| {
                opt(entity_of(sim, subject)
                    .and_then(|entity| super::routine_place(sim, entity))
                    .filter(|loc| sim.cosmos.is_loaded(*loc))
                    .map(|loc| sim.subject_value(Subject::Loc(loc))))
            },
        ),
        p(
            "entity",
            "routine_places",
            "list<loc>",
            "Every place the part of the day its routine is at names, each its own spot, in order (for `pick: all`, every site it answers, nearest its home first): a beat between them. Only those in loaded tracts.",
            |sim, subject, _| {
                let Some(entity) = entity_of(sim, subject) else {
                    return Value::list(Vec::new());
                };
                Value::list(
                    loaded_places(sim, entity)
                        .into_iter()
                        .map(|loc| sim.subject_value(Subject::Loc(loc))),
                )
            },
        ),
        p(
            "entity",
            "routine_away",
            "bool?",
            "Whether it is further from `routine_place` than the part of the day's `within` allows: not yet where its routine has it be. Null where `routine_place` is.",
            |sim, subject, _| {
                let Some(entity) = entity_of(sim, subject) else {
                    return Value::Null;
                };
                let Some((_, def)) = super::routine_of(sim, entity) else {
                    return Value::Null;
                };
                let Some(whom) = super::whom_of(sim, entity) else {
                    return Value::Null;
                };
                let Some((_, entry, _)) = super::current_entry(sim, def, &whom) else {
                    return Value::Null;
                };
                let Some(place) = super::routine_place(sim, entity)
                    .filter(|loc| sim.cosmos.is_loaded(*loc))
                else {
                    return Value::Null;
                };
                let far = crate::ai::nav::distance_to(sim, entity, place).unwrap_or(Real::INFINITY);
                Value::Bool(far > entry.within + Real::of(crate::ai::think::ARRIVED_SLACK_M))
            },
        ),
        p(
            "entity",
            "resident",
            "bool",
            "Whether it stands for a resident of a settlement (06 §12.2): made from a request generation handed over, the same person each time it is made.",
            |sim, subject, _| {
                Value::Bool(
                    entity_of(sim, subject)
                        .is_some_and(|entity| sim.agents.resident_of(entity).is_some()),
                )
            },
        ),
        p(
            "entity",
            "resident_home",
            "loc?",
            "Its home as a resident: the cell generation asked for it at. Null if it is no resident.",
            |sim, subject, _| {
                opt(entity_of(sim, subject)
                    .and_then(|entity| sim.agents.resident_of(entity))
                    .and_then(|(key, _)| {
                        let grid = sim.cosmos.realm(key.realm)?.base_grid;
                        Some(sim.subject_value(Subject::Loc(clash_world::Location::new(
                            grid, key.home,
                        ))))
                    }))
            },
        ),
        p(
            "entity",
            "agent_doing",
            "string?",
            "For an agent out of the world (09 §10.1): what it is doing. Null for anyone in the world, and for anyone who is no resident.",
            |sim, subject, _| {
                opt(entity_of(sim, subject)
                    .and_then(|entity| sim.agents.resident_of(entity))
                    .and_then(|(_, resident)| resident.agent.as_ref())
                    .map(|agent| agent.doing.what.clone()))
            },
        ),
        p(
            "realm",
            "time_of_day",
            "real?",
            "The time of day in the realm, in seconds since midnight by its day (09 §14.2): its sky's length of day and the time its clock starts at, its clock running at the realm's rate.",
            |sim, subject, _| {
                opt(realm_of(subject)
                    .and_then(|realm| calendar::time_of_day(sim, realm))
                    .map(|at| at.as_secs_real()))
            },
        )
        .safe(),
        p(
            "realm",
            "day",
            "int?",
            "The day it is in the realm, counted from 1 on the day its clock began (09 §14.2).",
            |sim, subject, _| opt(realm_of(subject).and_then(|realm| calendar::day_number(sim, realm))),
        )
        .safe(),
        p(
            "realm",
            "day_length",
            "real?",
            "How long a day of the realm is, in seconds of its clock: its sky's day (09 §14.2).",
            |sim, subject, _| match realm_of(subject) {
                Some(realm) if sim.cosmos.realm(realm).is_some() => {
                    seconds(calendar::realm_day(sim, realm).length)
                }
                _ => Value::Null,
            },
        )
        .safe(),
        p(
            "realm",
            "day_part",
            "string?",
            "The name of the part of the day it is in the realm, by the calendar the realm names (09 §14.2). Null for a realm with no calendar, or a calendar with no parts.",
            |sim, subject, _| opt(realm_of(subject).and_then(|realm| calendar::day_part(sim, realm))),
        )
        .safe(),
        p(
            "realm",
            "season",
            "string?",
            "The name of the season it is in the realm, by its calendar's year and seasons (09 §14.2). Null for a realm with no calendar, no year or no seasons.",
            |sim, subject, _| opt(realm_of(subject).and_then(|realm| calendar::season(sim, realm))),
        )
        .safe(),
    ]
}

fn verb_make_anonymous(sim: &mut Sim, call: &VerbCall<'_>, cause: Cause) -> Result<Value, Reason> {
    let who = args::entity(sim, call, 0, "who")?;
    super::make_anonymous(sim, who, cause)?;
    Ok(Value::Null)
}

fn verb_set_routine(sim: &mut Sim, call: &VerbCall<'_>, _cause: Cause) -> Result<Value, Reason> {
    let who = args::entity(sim, call, 0, "who")?;
    let written = args::optional_text(call, 1, "routine")?;
    match written {
        None => {
            sim.agents.routines.remove(&who);
        }
        Some(name) => {
            let (id, _) = super::routine::routine_named(sim, name).ok_or_else(|| {
                Reason::new(
                    why::UNKNOWN_ROUTINE,
                    format!("there is no routine `{name}`"),
                )
                .with("routine", name)
            })?;
            let qualified = super::routine::routine_name(sim, id);
            sim.agents.routines.insert(who, qualified);
        }
    }
    crate::ai::prompt(sim, who);
    Ok(Value::Null)
}

/// The verbs of this module.
pub fn verbs() -> Vec<VerbDef> {
    vec![
        VerbDef {
            name: "make_anonymous",
            build: |registry| {
                let entity = registry.kind_id("entity")?;
                Some(
                    VerbSpec::new("make_anonymous")
                        .arg("who", Type::Subject(entity), "The individual.")
                        .direct("who")
                        .doc("Makes a notable individual anonymous again (06 §12.2): it is forgotten when its tract unloads, and a resident is then made afresh from its seed when it next comes into view; an agent made anonymous is forgotten at once. Raises `became_anonymous`; gives null. Refused with `agent.not_an_individual` for something not made from templates."),
                )
            },
            run: verb_make_anonymous,
        },
        VerbDef {
            name: "set_routine",
            build: |registry| {
                let entity = registry.kind_id("entity")?;
                Some(
                    VerbSpec::new("set_routine")
                        .arg("who", Type::Subject(entity), "The character.")
                        .optional_arg("routine", Type::Str, "The routine's id, qualified or bare. Left out, the routine `set_routine` gave is taken away and its mind's applies again.")
                        .direct("who")
                        .doc("Gives a character a routine in place of the one its mind names (09 §17), and makes its mind think. Gives null. Refused with `agent.unknown_routine` for a routine no definition answers to."),
                )
            },
            run: verb_set_routine,
        },
    ]
}
