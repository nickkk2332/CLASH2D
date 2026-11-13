//! Doors on a mind's way (09 §5.3; decision D-781): a closed door on a
//! path is opened through the interaction the player would use, as part
//! of the step that moves.
//!
//! **Who can open things.** A mind whose definition names an
//! `interact_action` (an action with the parameters `target` and
//! `interaction`, as `core:interact` has) opens what it meets; its paths
//! count a closed door as passable at the cost of opening it. A mind
//! that names none (a dog) cannot: a closed door is a wall to its paths.
//! A character with no mind definition, the player's, plans as before.
//!
//! **Meeting one.** A step of a `go`, `keep_near` or `keep_from` is
//! refused because the way is closed (`step_refused` with the code
//! `move.closed`): what the player meets as a bump. The mind notes the
//! direction and, at its next thought, stops walking and starts its
//! `interact_action` with the default interaction of what is there
//! (`interaction::bump_default`: the block's, else the fixture's, else
//! the floor's, that can be done now), exactly as walking into it does
//! for the player. When that action ends the step is begun again, and
//! its path planned afresh from where the character stands.
//!
//! **One it cannot open.** With nothing there it can do by default (a
//! locked door and no key), or after [`MAX_DOOR_TRIES`] openings on one
//! step, the step fails with `ai.door_shut`, naming why the default
//! cannot be done, and the cell is kept among the mind's shut doors
//! (at most [`MAX_SHUT`]), which its paths go round from then on.

use clash_core::EntityId;
use clash_data::Def;
use clash_world::{CellPos, Location, StepKind as Dir};

use super::state::{Door, Outcome};
use super::think::{mind_def, mind_def_of};
use super::{StepKind, reason as why};
use crate::Sim;
use crate::event::SimEvent;
use crate::interaction::{self, Target};
use crate::reason::Reason;

/// How many doors one step that moves opens before it gives up: a door
/// shut again behind it, over and over, is not followed for ever.
pub const MAX_DOOR_TRIES: u8 = 4;

/// How many closed ways a mind remembers it could not open.
pub const MAX_SHUT: usize = 16;

/// Whether a character's paths count a closed door as passable at the
/// cost of opening it: one whose mind can open things, or one with no
/// mind definition (the player's, a script's). A mind that names no
/// `interact_action` cannot.
pub fn opens_doors(sim: &Sim, entity: EntityId) -> bool {
    match mind_def_of(sim, entity).and_then(|id| mind_def(sim, id)) {
        Some(def) => def
            .interact_action
            .as_deref()
            .is_some_and(|name| action_named(sim, name).is_some()),
        None => true,
    }
}

/// The cells a mind found shut and could not open, on a grid.
pub fn shut_cells(sim: &Sim, entity: EntityId, grid: clash_world::GridId) -> Vec<CellPos> {
    sim.components
        .mind_state
        .get(entity)
        .map(|state| {
            state
                .shut
                .iter()
                .filter(|loc| loc.grid == grid)
                .map(|loc| loc.cell)
                .collect()
        })
        .unwrap_or_default()
}

/// The refusal of a step: a mind on a step that moves, whose way was
/// closed, notes the direction for its next thought.
pub(super) fn step_refused(sim: &mut Sim, event: &SimEvent) {
    let Some(mover) = event.subject("self").and_then(|subject| subject.entity()) else {
        return;
    };
    let code = super::think::text_field(event.body.field("code"));
    if code.as_deref() != Some(crate::movement::codes::CLOSED) {
        return;
    }
    let Some(dir) = super::think::text_field(event.body.field("dir"))
        .and_then(|dir| crate::movement::parse_dir(&dir))
        .and_then(|dir| Dir::ALL.iter().position(|kind| *kind == dir))
        .and_then(|at| u8::try_from(at).ok())
    else {
        return;
    };
    if !opens_doors(sim, mover) || mind_def_of(sim, mover).is_none() {
        return;
    }
    let Some(pending) = sim
        .components
        .mind_state
        .get_mut(mover)
        .and_then(|state| state.held.as_mut())
        .and_then(|held| held.pending.as_mut())
    else {
        return;
    };
    let moves = matches!(
        pending.kind(),
        StepKind::Go | StepKind::KeepNear | StepKind::KeepFrom
    );
    if !moves || pending.outcome != Outcome::Running {
        return;
    }
    if matches!(
        pending.door,
        Door::None | Door::Opened { .. } | Door::Walking { .. }
    ) {
        pending.door = Door::Met {
            dir,
            tries: pending.door.tries(),
        };
        sim.ai.due.insert(mover);
    }
}

/// What the step under way does about a door it met, at a thought.
/// `Some(outcome)` settles the step; `None` leaves it as it is, running.
/// A step that has opened its door is begun again by the caller.
pub(super) fn settle(sim: &mut Sim, mind: EntityId) -> Option<Option<Outcome>> {
    let pending = sim
        .components
        .mind_state
        .get(mind)
        .and_then(|state| state.held.as_ref())
        .and_then(|held| held.pending.clone())?;
    match pending.door {
        Door::None | Door::Opened { .. } | Door::Walking { .. } => None,
        Door::Met { dir, tries } => Some(open(sim, mind, dir, tries, pending.action)),
        Door::Opening { action, tries } => {
            if sim.actions.get(action).is_some() {
                return Some(None);
            }
            set_door(sim, mind, Door::Opened { tries });
            Some(None)
        }
    }
}

/// Starts opening what closes the way in a direction, or fails the step
/// with the reason it cannot be opened.
fn open(
    sim: &mut Sim,
    mind: EntityId,
    dir: u8,
    tries: u8,
    walking: Option<crate::later::ActionId>,
) -> Option<Outcome> {
    let cause = super::think::cause_of(mind);
    // Walking stops: the path is planned again once it is open.
    if let Some(id) = walking
        && sim.actions.get(id).is_some()
    {
        let _ = sim.cancel_action(mind, id, true, cause);
    }
    let kind = Dir::ALL.get(usize::from(dir)).copied()?;
    let dir_name = crate::movement::dir_name(kind);
    let next = sim.entities.location(mind).and_then(|here| {
        let delta = kind.delta();
        sim.cosmos
            .normalize(Location::new(
                here.grid,
                CellPos::new(
                    here.cell.x + delta.x,
                    here.cell.y + delta.y,
                    here.cell.z + delta.z,
                ),
            ))
            .ok()
    });
    let action = mind_def_of(sim, mind)
        .and_then(|id| mind_def(sim, id))
        .and_then(|def| def.interact_action.as_deref())
        .and_then(|name| action_named(sim, name));
    let offer = interaction::bump_default(sim, mind, dir_name);
    let (Some(action), Some((target, offer)), true) = (action, offer, tries < MAX_DOOR_TRIES)
    else {
        let why_not = next.map(|next| refusal_there(sim, mind, next));
        if let Some(next) = next {
            remember_shut(sim, mind, next);
        }
        let mut reason = Reason::new(why::DOOR_SHUT, "the way is shut and it cannot open it")
            .with("entity", mind.to_string())
            .with("dir", dir_name);
        if let Some(Some(text)) = why_not {
            reason = reason.with("why", text.as_str());
        }
        set_door(sim, mind, Door::None);
        return Some(Outcome::Failed(reason));
    };
    let params = vec![
        (
            "target".to_owned(),
            sim.subject_value(target.subject()).to_stored(),
        ),
        (
            "interaction".to_owned(),
            interaction_value(sim, offer.interaction).to_stored(),
        ),
    ];
    if sim.actions.wait_of(mind).is_some() {
        let _ = sim.wait(mind, clash_core::Dur::ZERO, cause);
    }
    match sim.start_action(mind, action, &params, cause) {
        Ok(Some(id)) if sim.actions.get(id).is_some() => {
            set_door(
                sim,
                mind,
                Door::Opening {
                    action: id,
                    tries: tries + 1,
                },
            );
            if let Some(pending) = sim
                .components
                .mind_state
                .get_mut(mind)
                .and_then(|state| state.held.as_mut())
                .and_then(|held| held.pending.as_mut())
            {
                pending.action = Some(id);
            }
            None
        }
        // Done at once, or refused by a handler: try the way again.
        Ok(_) => {
            set_door(sim, mind, Door::Opened { tries: tries + 1 });
            None
        }
        Err(reason) => {
            set_door(sim, mind, Door::None);
            Some(Outcome::Failed(reason))
        }
    }
}

/// An action by id as written, qualified or bare.
fn action_named(sim: &Sim, name: &str) -> Option<crate::action::ActionDefId> {
    let registry = sim.defs.try_registry::<crate::action::def::ActionDef>()?;
    registry.lookup(name).or_else(|| {
        if name.contains(':') {
            return None;
        }
        registry
            .names()
            .iter()
            .filter(|qualified| qualified.rsplit(':').next() == Some(name))
            .min()
            .and_then(|qualified| registry.lookup(qualified))
    })
}

/// An interaction as the value an action's parameter of type
/// `id<interaction>` takes.
fn interaction_value(sim: &Sim, id: interaction::InteractionId) -> clash_expr::Value {
    match sim
        .exprs()
        .registry()
        .def_type_id(interaction::InteractionDef::TYPE)
    {
        Some(ty) => clash_expr::Value::Id(clash_expr::DefRef::new(ty, u32::from(id.raw()))),
        None => clash_expr::Value::from(interaction::name_of(sim, id).as_str()),
    }
}

/// Why nothing at a cell can be done by default, if something there
/// would be: what walking into a locked door tells the player.
fn refusal_there(sim: &Sim, user: EntityId, at: Location) -> Option<String> {
    let mut why: Vec<String> = Vec::new();
    for layer in [
        clash_world::Layer::Block,
        clash_world::Layer::Fixture,
        clash_world::Layer::Floor,
    ] {
        for offer in interaction::interactions_for(sim, user, Target::Element(at, layer)) {
            if let (true, Some(reason)) = (offer.default, offer.refusal)
                && !why.contains(&reason.message)
            {
                why.push(reason.message);
            }
        }
    }
    (!why.is_empty()).then(|| why.join(" "))
}

/// Keeps a cell among those a mind could not open.
fn remember_shut(sim: &mut Sim, mind: EntityId, at: Location) {
    if let Some(state) = sim.components.mind_state.get_mut(mind) {
        state.shut.retain(|have| *have != at);
        state.shut.push(at);
        while state.shut.len() > MAX_SHUT {
            state.shut.remove(0);
        }
    }
}

/// Sets what the step under way is doing about a door.
fn set_door(sim: &mut Sim, mind: EntityId, door: Door) {
    if let Some(pending) = sim
        .components
        .mind_state
        .get_mut(mind)
        .and_then(|state| state.held.as_mut())
        .and_then(|held| held.pending.as_mut())
    {
        pending.door = door;
    }
}

/// Whether a step's door was opened, so that the step is begun again;
/// and how many doors it has opened.
pub(super) fn opened(sim: &Sim, mind: EntityId) -> Option<u8> {
    let pending = sim
        .components
        .mind_state
        .get(mind)?
        .held
        .as_ref()?
        .pending
        .as_ref()?;
    match pending.door {
        Door::Opened { tries } => Some(tries),
        _ => None,
    }
}

/// What an action ending means for a door being opened: the mind thinks
/// at once. Returns whether the action was the opening of a door.
pub(super) fn action_ended(sim: &mut Sim, actor: EntityId, id: crate::later::ActionId) -> bool {
    let opening = sim
        .components
        .mind_state
        .get(actor)
        .and_then(|state| state.held.as_ref())
        .and_then(|held| held.pending.as_ref())
        .is_some_and(
            |pending| matches!(pending.door, Door::Opening { action, .. } if action == id),
        );
    if opening {
        sim.ai.due.insert(actor);
    }
    opening
}
