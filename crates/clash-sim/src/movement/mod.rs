//! Movement and posture (`docs/06-entities-and-bodies.md` section 11):
//! what a step is, how long it takes, where a mover can go, what
//! happens with nothing underfoot, and following a path.
//!
//! Moving is an action (03 §6.6) defined as data, whose effect is one
//! of the verbs here. `mods/core/data/movement.json` holds the two the
//! play screen names: `step` and `go_to`.
//!
//! | Need | Use |
//! |------|-----|
//! | What a mover can do now | [`mover_rules`]: a [`MoverRules`], the world's `MoveRules` for one entity |
//! | To take one step | [`step`], with a [`StepKind`](clash_world::StepKind); [`parse_dir`] reads a direction as data writes it |
//! | How long a step takes | [`step_time`], in the mover's own time; `entity.step_time(dir)` in an expression |
//! | To put a mover somewhere it can be | [`move_to`] |
//! | To follow a path | [`plan_path`], then [`follow_path`] once for each step; [`clear_path`] |
//! | What a mover believes | [`belief`]: [`believed_rules`], [`believed_step`], [`believed_path`], [`believed_step_time`] |
//! | Postures | [`set_posture`], [`posture_of`], [`posture_occupies`], [`signature_factor`] |
//! | Losing one's footing | [`lose_footing`], [`fall`]; [`launched`] and [`take_back`] as the world carries a body |
//! | The ways of moving an entity has | [`set_modes`] |
//! | The definition types | [`MovementDef`], [`PostureDef`] |
//! | The component | [`Mover`] |
//! | Verbs, properties, events, reasons | [`verbs`], [`properties`], [`MOVEMENT_EVENTS`], [`hook_types`], [`MOVEMENT_REASONS`] |
//!
//! # The rules, in short
//!
//! - **Belief, then truth.** A path is planned from what the mover
//!   believes ([`belief`]); each step is checked against the world as
//!   it is at the moment it is taken. A door closed since the plan was
//!   made refuses the step, with a reason.
//! - **By data.** Which steps a creature can take, and how fast, is
//!   what its `movement` definitions say: where each takes its mover
//!   ([`Surface`]), a `speed` expression over the creature, the channel
//!   slots it needs. A creature left with no way of moving that works
//!   cannot move; nothing here names a leg.
//! - **One mutation path.** A step relocates through
//!   [`Sim::relocate`](crate::Sim::relocate) and nothing else.
//! - **Time is the action's.** A verb takes no time. [`step_time`] says
//!   how long a step takes, in the mover's own time, and the action's
//!   phase waits that long: `"time": "self.step_time($dir) ?? 0"`.
//! - **Occupancy.** A mover that blocks movement cannot enter a cell
//!   another such occupant fills on a plane they share, unless that one
//!   yields: the hook `passage_requested` decides, and the two then
//!   change places in the one step.
//! - **Every refusal has a reason** of [`MOVEMENT_REASONS`], and a step
//!   that was refused raises `step_refused` with it.
//!
//! # What the kernel calls
//!
//! | When | The kernel |
//! |------|-----------|
//! | An event has been delivered to its handlers | Calls [`on_event`]: `entity_unsupported` makes a mover that cannot stay where it is fall; `body_launched` and `body_landed` of an entity's body measure the fall and raise `landed` |
//! | Building its registry | Calls [`register_types`], and lists [`verbs`] and [`properties`] |
//! | Building its event bus | Enters [`hook_types`] |
//!
//! # What is not built here
//!
//! - **Moving as a body** (02 §15). A mover that falls is a body of the
//!   world's until it lands; `sweep` and `leap`, which launch a mover on
//!   purpose, are not built, `teleport` waits for realm transfer
//!   (02 §19) and `set_altitude` for fliers.
//! - **Belief.** [`belief`] reads truth until perception is built.
//! - **A body's height.** A posture scales the height of the entity's
//!   `Physical`; a body's own height factor
//!   (`body::set_height_factor`) is set when bodies are held by the
//!   simulation.
//! - **Channels.** What a posture `occupies` is read by
//!   [`posture_occupies`] and by the check of a way of moving's `uses`;
//!   claiming and disabling slots is the action runtime's.
//! - **Wide movers.** A path is planned for the corner cell of a
//!   mover's footprint; a step checks every cell of it.

pub mod belief;
mod defs;
mod fall;
mod mover;
mod path;
mod posture;
mod props;
mod rules;
mod step;
mod verbs;

use clash_data::{Def, DefTypes};
use clash_expr::{EventSpec, Registry, Type};
use clash_world::{BodyOwner, WorldEvent};

use crate::bus::{EventType, FieldType};
use crate::event::{CAUSE_SUBJECTS, EventBody, EventInfo, SimEvent};
use crate::reason::ReasonInfo;
use crate::sim::Sim;

pub use belief::{believed_path, believed_rules, believed_step, believed_step_time};
pub use defs::{MovementDef, MovementId, PostureDef, PostureId, Surface};
pub use fall::{Fall, Landing, fall, launched, lose_footing, take_back};
pub use mover::{Flight, Journey, Mover};
pub use path::{Planned, clear_path, follow_path, mark_journey_stale, path_cost, plan_path};
pub use posture::{posture_occupies, posture_of, set_posture, signature_factor};
pub use props::properties;
pub use rules::{ModeRule, MoverRules, StepFault, StepPrice, mover_rules};
pub use step::{Stepped, dir_name, move_to, parse_dir, set_modes, step, step_time};
pub use verbs::verbs;

/// Raised when a step was refused.
pub const STEP_REFUSED: &str = "step_refused";
/// Raised when no path could be planned.
pub const PATH_FAILED: &str = "path_failed";
/// Raised when an entity's posture has changed.
pub const POSTURE_CHANGED: &str = "posture_changed";
/// Raised when an entity has lost its footing and begun to fall.
pub const FELL: &str = "fell";
/// Raised when a fall has ended.
pub const LANDED: &str = "landed";
/// The hook raised before a posture changes.
pub const POSTURE_CHANGING: &str = "posture_changing";
/// The hook raised when a mover asks whoever fills a cell to let it by.
pub const PASSAGE_REQUESTED: &str = "passage_requested";
/// The field of [`PASSAGE_REQUESTED`] a handler sets to let the mover
/// by.
pub const YIELDS_FIELD: &str = "yields";

/// The notifications movement raises, entered beside the kernel's own in
/// every simulation's event bus and in the engine's registry, and listed
/// for the generated reference. Each is raised as a payload under its
/// name.
pub const MOVEMENT_EVENTS: &[EventInfo] = &[
    EventInfo {
        name: STEP_REFUSED,
        subjects: &[("self", "entity")],
        fields: &[
            (
                "dir",
                "string",
                "The direction of the step: `north` to `north_west`, `up` or `down`; empty for a placement.",
            ),
            ("code", "string", "The code of the reason (06 §11)."),
            ("message", "string", "The reason, as a sentence."),
        ],
        doc: "A step, or a placement by `move_to`, was refused when it was tried: the mover is where it was.",
    },
    EventInfo {
        name: PATH_FAILED,
        subjects: &[("self", "entity"), ("here?", "loc")],
        fields: &[
            ("code", "string", "The code of the reason (06 §11)."),
            ("message", "string", "The reason, as a sentence."),
        ],
        doc: "No path could be planned for a mover to a cell, bound as `here`. Whatever path it was following is forgotten.",
    },
    EventInfo {
        name: POSTURE_CHANGED,
        subjects: &[("self", "entity")],
        fields: &[
            (
                "from",
                "string",
                "The qualified id of the posture it was in; empty for none.",
            ),
            (
                "to",
                "string",
                "The qualified id of the posture it is in; empty for none.",
            ),
            (
                "look",
                "string",
                "The `look` of the posture it is in, as its definition writes it; empty for none.",
            ),
        ],
        doc: "An entity's posture changed, and with it its height (06 §11.1).",
    },
    EventInfo {
        name: FELL,
        subjects: &[("self", "entity"), ("here", "loc")],
        fields: &[],
        doc: "An entity had nothing under it where it could not stay and began to fall from `here`: the world carries it as a body from now on. `landed` follows when it comes to rest.",
    },
    EventInfo {
        name: LANDED,
        subjects: &[("self", "entity"), ("here", "loc")],
        fields: &[
            (
                "height",
                "real",
                "How far below where it set off it came to rest, in metres; zero if it came to rest higher.",
            ),
            (
                "speed",
                "real",
                "How fast something falling that far lands, in metres a second: `sqrt(2 g h)`, with nothing taken off for the air.",
            ),
            (
                "energy",
                "real",
                "The energy of a fall that far, in joules: its mass, with what it carries, times gravity times the height.",
            ),
        ],
        doc: "The world has stopped carrying an entity as a body: it came to rest at `here`, after a fall or after being thrown. What the landing does to it is for whoever turns impacts into hits (06 §7); the blows themselves are the world's `body_impact`.",
    },
];

/// The hooks movement raises (05 §8.2): entered in the event bus of
/// every simulation.
pub fn hook_types() -> Vec<EventType> {
    vec![
        EventType::new(POSTURE_CHANGING)
            .subject("self", "entity")
            .field(
                "from",
                FieldType::Str,
                "The qualified id of the posture it is in; empty for none.",
            )
            .field(
                "to",
                FieldType::Str,
                "The qualified id of the posture it would take; empty for none.",
            )
            .hook(Vec::<String>::new())
            .doc("Runs before an entity's posture changes (06 §11.1). A handler may cancel: the posture stays."),
        EventType::new(PASSAGE_REQUESTED)
            .subject("self", "entity")
            .subject("target", "entity")
            .subject("here", "loc")
            .field(
                YIELDS_FIELD,
                FieldType::Bool,
                "Whether `target` lets the mover by. False until a handler sets it.",
            )
            .hook([YIELDS_FIELD])
            .doc("Runs when a mover (`self`) would step into the cell `here` that `target` fills (06 §11). A handler sets `yields` to true to make the two change places; cancelling, or leaving it false, refuses the step."),
    ]
}

/// The type of a field of a hook, as an expression sees it.
fn hook_field_type(written: &str) -> Type {
    match FieldType::parse(written) {
        Some(FieldType::Bool) => Type::Bool,
        Some(FieldType::Int) => Type::Int,
        Some(FieldType::Real) => Type::Real,
        Some(FieldType::Str) => Type::Str,
        Some(FieldType::Any) | None => Type::Any,
    }
}

/// Enters in a registry what movement's verbs and properties name and
/// what its hooks are: the definition types `movement` and `posture`
/// and the two hooks, unless they are there already. Called while the
/// engine's registry is built, before its properties and verbs.
pub fn register_types(registry: &mut Registry) {
    for (name, doc) in [
        (MovementDef::TYPE, MovementDef::DOC),
        (PostureDef::TYPE, PostureDef::DOC),
    ] {
        if registry.def_type_id(name).is_none() {
            let _ = registry.add_def_type(name, doc);
        }
    }
    for hook in hook_types() {
        if registry.event_id(&hook.name).is_some() {
            continue;
        }
        let mut spec = EventSpec::new(hook.name.clone()).doc(hook.doc.clone());
        let subjects = hook
            .subjects
            .iter()
            .map(|(name, kind)| (name.as_str(), kind.as_str()));
        for (name, kind) in subjects.chain(CAUSE_SUBJECTS.iter().copied()) {
            if let Some(kind) = registry.kind_id(kind) {
                spec = spec.subject(name.trim_end_matches('?'), kind);
            }
        }
        for field in &hook.fields {
            spec = spec.field(
                field.name.clone(),
                hook_field_type(&field.ty),
                field.doc.clone(),
            );
        }
        let _ = registry.add_event(spec.hook(hook.mutable.iter().cloned()));
    }
}

/// Registers the definition types `movement` and `posture` with the
/// loader. Called from the crate's `register_defs`.
pub fn register_defs(types: &mut DefTypes) {
    types.register::<MovementDef>().register::<PostureDef>();
}

/// What movement does about an event once its handlers have run (06
/// §11): an entity whose footing has gone falls, unless it can stay
/// where it is; an entity the world begins to carry as a body is noted
/// where it set off; and one whose body has come to rest is taken back,
/// raising `landed`. Called by the kernel for every delivered event.
pub fn on_event(sim: &mut Sim, event: &SimEvent) {
    match &event.body {
        EventBody::EntityUnsupported { entity, .. } => {
            let _ = lose_footing(sim, *entity, event.cause);
        }
        EventBody::World(WorldEvent::BodyLaunched {
            owner: BodyOwner::Entity(entity),
            loc,
            pos,
            ..
        }) => launched(sim, *entity, *loc, pos[2]),
        EventBody::World(WorldEvent::BodyLanded {
            owner: BodyOwner::Entity(entity),
            loc,
            pos,
            ..
        }) => {
            let _ = take_back(sim, *entity, *loc, pos[2], event.cause);
        }
        _ => {}
    }
}

/// The codes of movement's reasons. Each is listed in
/// [`MOVEMENT_REASONS`].
pub mod codes {
    /// The mover has no way of moving that works now.
    pub const CANNOT_MOVE: &str = "move.cannot_move";
    /// The mover is not standing in the world.
    pub const NOWHERE: &str = "move.nowhere";
    /// The direction is not one of the ten.
    pub const BAD_DIRECTION: &str = "move.bad_direction";
    /// A verb was given something that is not what it takes.
    pub const BAD_ARGUMENT: &str = "move.bad_argument";
    /// There is no cell there to step into.
    pub const EDGE: &str = "move.edge";
    /// Something stands in the way.
    pub const BLOCKED: &str = "move.blocked";
    /// What stands in the way could stand aside, and does not.
    pub const CLOSED: &str = "move.closed";
    /// A diagonal step past something that is in the way.
    pub const CORNER: &str = "move.corner";
    /// No way up or down.
    pub const TOO_STEEP: &str = "move.too_steep";
    /// Nothing to stand on.
    pub const NO_FOOTING: &str = "move.no_footing";
    /// Liquid, and the mover does not swim.
    pub const LIQUID: &str = "move.liquid";
    /// Someone fills the cell.
    pub const OCCUPIED: &str = "move.occupied";
    /// The mover does not fit.
    pub const NO_ROOM: &str = "move.no_room";
    /// There is no way to the goal.
    pub const NO_WAY: &str = "move.no_way";
    /// The goal is too far.
    pub const TOO_FAR: &str = "move.too_far";
    /// The mover cannot be at the goal.
    pub const GOAL_BLOCKED: &str = "move.goal_blocked";
    /// The goal is in another grid.
    pub const OTHER_GRID: &str = "move.other_grid";
    /// The mover is following no path.
    pub const NO_PATH: &str = "move.no_path";
    /// The world is carrying the mover as a body.
    pub const IN_FLIGHT: &str = "move.in_flight";
    /// The mover is not where its path left it.
    pub const OFF_PATH: &str = "move.off_path";
    /// No such posture.
    pub const POSTURE_UNKNOWN: &str = "posture.unknown";
    /// No such way of moving.
    pub const MOVEMENT_UNKNOWN: &str = "move.unknown_movement";
}

/// Movement's reasons, for the generated reference (14 §5).
pub const MOVEMENT_REASONS: &[ReasonInfo] = &[
    ReasonInfo {
        code: codes::CANNOT_MOVE,
        doc: "The mover has no way of moving that works now: it lists none and no definition is its default, its posture allows none, its body lacks the channel slots, or every `speed` comes to nothing. Also given for a step that none of its ways of moving makes, where nothing more particular applies. Carries `entity`.",
    },
    ReasonInfo {
        code: codes::NOWHERE,
        doc: "The mover is not standing in the world: it is inside something or stored. Carries `entity`.",
    },
    ReasonInfo {
        code: codes::BAD_DIRECTION,
        doc: "A direction was not one of `north`, `north_east`, `east`, `south_east`, `south`, `south_west`, `west`, `north_west`, `up` and `down`. Carries `dir`.",
    },
    ReasonInfo {
        code: codes::BAD_ARGUMENT,
        doc: "A movement verb was given something it does not take: a `who` that is not an entity, a `to` that is not a cell. Carries `argument`.",
    },
    ReasonInfo {
        code: codes::EDGE,
        doc: "There is no cell to step into: it is not loaded, or lies beyond an edge of the realm. Carries `dir`.",
    },
    ReasonInfo {
        code: codes::BLOCKED,
        doc: "Something stands in the cell that the mover cannot step over and that has no state in which it stands aside: a wall. Carries `height`, in metres.",
    },
    ReasonInfo {
        code: codes::CLOSED,
        doc: "What stands in the cell could stand aside and does not: a closed door. Carries `height`.",
    },
    ReasonInfo {
        code: codes::CORNER,
        doc: "A diagonal step would pass a cell with something in the way: movers do not cut corners.",
    },
    ReasonInfo {
        code: codes::TOO_STEEP,
        doc: "There is no way up or down here for this mover: no ramp, stairs or ladder it can take, or a floor between.",
    },
    ReasonInfo {
        code: codes::NO_FOOTING,
        doc: "There is nothing to stand on in the cell and the mover cannot be there otherwise. A step given `drop` is made all the same, and the mover falls.",
    },
    ReasonInfo {
        code: codes::LIQUID,
        doc: "The cell is liquid and the mover has no way of moving through liquid.",
    },
    ReasonInfo {
        code: codes::OCCUPIED,
        doc: "Another occupant that blocks movement fills the cell on a plane the mover shares, and did not yield. Carries `by`, the occupant.",
    },
    ReasonInfo {
        code: codes::NO_ROOM,
        doc: "A mover wider or taller than a cell does not fit where the step would put it.",
    },
    ReasonInfo {
        code: codes::NO_WAY,
        doc: "No path leads to the goal: every cell the mover believes it can reach was tried, or the search settled as many cells as it may (`searched`) without reaching it.",
    },
    ReasonInfo {
        code: codes::TOO_FAR,
        doc: "No path was found within the cost allowed. Carries `max_cost`, in metres of travel.",
    },
    ReasonInfo {
        code: codes::GOAL_BLOCKED,
        doc: "The mover believes it cannot be in the goal cell at all.",
    },
    ReasonInfo {
        code: codes::OTHER_GRID,
        doc: "The goal is in another grid than the mover.",
    },
    ReasonInfo {
        code: codes::NO_PATH,
        doc: "`follow_path` was called for a mover that is following no path, or has reached the end of it.",
    },
    ReasonInfo {
        code: codes::OFF_PATH,
        doc: "The mover is not where its path left it, or an earlier step of the path was refused: the path must be planned again.",
    },
    ReasonInfo {
        code: codes::IN_FLIGHT,
        doc: "The world is carrying the mover as a body (02 §15): it is falling or thrown, and steps and placements wait until it lands. Carries `entity`.",
    },
    ReasonInfo {
        code: codes::POSTURE_UNKNOWN,
        doc: "A posture was named that no loaded definition has.",
    },
    ReasonInfo {
        code: codes::MOVEMENT_UNKNOWN,
        doc: "A way of moving was named that no loaded definition has.",
    },
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_event_and_hook_is_documented() {
        for info in MOVEMENT_EVENTS {
            assert!(!info.doc.is_empty(), "`{}` has no documentation", info.name);
            for (name, _, doc) in info.fields {
                assert!(
                    !doc.is_empty(),
                    "`{}.{name}` has no documentation",
                    info.name
                );
            }
        }
        for hook in hook_types() {
            assert!(hook.hook && !hook.doc.is_empty());
        }
    }

    #[test]
    fn reason_codes_are_listed_once() {
        let mut codes: Vec<&str> = MOVEMENT_REASONS.iter().map(|reason| reason.code).collect();
        let count = codes.len();
        codes.sort_unstable();
        codes.dedup();
        assert_eq!(codes.len(), count);
    }

    #[test]
    fn the_engine_registry_holds_what_movement_registers() {
        let registry = crate::engine_registry();
        assert!(registry.def_type_id("movement").is_some());
        assert!(registry.def_type_id("posture").is_some());
        for hook in hook_types() {
            assert!(registry.event_id(&hook.name).is_some(), "{}", hook.name);
        }
        for info in MOVEMENT_EVENTS {
            assert!(registry.event_id(info.name).is_some(), "{}", info.name);
        }
        for verb in verbs() {
            assert!(registry.verb_id(verb.name).is_some(), "{}", verb.name);
        }
        let entity = registry.kind_id("entity").unwrap();
        for property in properties() {
            assert!(
                registry.property_id(entity, property.name).is_some(),
                "{}",
                property.name
            );
            assert!(!property.doc.is_empty());
        }
    }
}
