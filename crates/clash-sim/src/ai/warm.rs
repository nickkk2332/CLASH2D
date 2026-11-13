//! Detail levels of minds (`docs/09-ai-and-simulation.md` §9,
//! `docs/03-time-and-actions.md` §10; packages I8 and T7; decisions
//! D-890 to D-899): who is stepped in full and who in bulk.
//!
//! # Levels
//!
//! A character with a mind standing in the world is **hot** (stepped as
//! sections 3 to 8 of 09 say: it sweeps, thinks when prompted and acts)
//! or **warm** (stepped in bulk). Of the minds in the world, at most
//! `sim.max_hot_minds` are hot; the rest are warm (09 §9.3). A mind's
//! tract does not make it warm: one in a warm tract is ranked as any
//! other, nearest first, and so is among the first stepped down; its
//! perception is a warm observer's there whatever its level (15 §14;
//! D-890). A directly controlled character has no mind that decides and
//! is never warm.
//!
//! # Attention
//!
//! Levels are worked out by one **stepper** (03 §10) every
//! [`WARM_STEP`] of universal time while any mind is warm, or could be:
//! it starts when a mind thinks while more minds have thought than the
//! cap allows, and stops when nobody is warm. At each step the minds are ranked, highest first:
//!
//! 1. those the attention cap never steps down: a character holding a
//!    hot ticket of its own (class 1), one sharing an action, a window or
//!    a grip with a directly controlled character or such a holder
//!    (class 2), and one standing where the party sees (15 §13);
//! 2. those that became hot less than [`DWELL`] ago;
//! 3. everyone else;
//! 4. those that became warm less than [`DWELL`] ago,
//!
//! and within each by class (1, 2, notable, anonymous), then those the
//! party sees first, then nearest to a directly controlled character
//! (else to the anchor of a hot ticket), then by id. The first group is
//! hot whatever the cap; then, in order, the others fill what is left
//! of it. So the cap is exceeded only by the first group, and
//! nobody changes level more often than [`DWELL`] allows unless the cap
//! or the first group makes it.
//!
//! Between steps a warm character is **promoted** at once when the
//! party comes to see it (a sweep of a member's sight), when an action
//! that starts names it (actor, parameter or participant: 03 §10's
//! promotion by involvement), and when its own mind would start an
//! action (a warm character starts none: 03 §10). The next step steps
//! the lowest-ranked down to keep the cap.
//!
//! # Warm
//!
//! A warm character does not sweep, and receives only signals at or
//! over `perception.warm_factor` times its threshold (15 §14). Its mind
//! thinks only at the stepper's steps, in order of entity, when it is
//! due: prompted, or its interval passed. It thinks as a hot one does,
//! from the same beliefs and goals, but a step that moves is walked in
//! bulk: its path is planned over its belief as `go_to` plans it, and
//! at each step it takes as many of the path's steps as the time it has
//! had since allows, each checked against the world as any step is
//! (09 §9.1). Waits and steps that take no time are as for a hot mind.
//!
//! # Moving between levels (09 §9.2)
//!
//! | From | To | What happens |
//! |------|----|--------------|
//! | Warm | Hot | Its sweeps begin again and it thinks at once; a step it was walking in bulk is begun again as its `go_action`, toward the same place |
//! | Hot | Warm | What it is doing runs on; an action that walks a step that moves ends at its next phase, its path kept as the route the mind walks in bulk, toward the same place |
//!
//! # Not built
//!
//! Bulk exchanges between opposed warm characters (09 §9.1): a warm
//! mind that would strike starts an action, and so is promoted first,
//! and the fight is resolved in full. `low_attention` actions
//! (03 §11). A stepper of each realm on its own clock: one stepper on
//! universal time advances each warm character by the time its own
//! clock has run.

use std::collections::{BTreeMap, BTreeSet};

use clash_core::{Dur, EntityId, StateHash, StateHasher, Tick, impl_state_hash};
use clash_time::{ClockId, Wake, WakeDomain, WakeId};
use clash_world::{Activity, Location, OccupantFilter, OccupantFlags, TicketAnchor};
use serde::{Deserialize, Serialize};

use super::defs::StepKind;
use super::state::{Door, Outcome};
use super::think;
use crate::Sim;
use crate::entity::Control;
use crate::action::ActEvent;
use crate::bus::EventPayload;
use crate::event::{EventBody, EventInfo, SimEvent, Subject, Where};

/// How often the warm stepper wakes, in universal time (03 §10).
pub const WARM_STEP: Dur = Dur::from_secs(1);

/// How long a character that changed level keeps it, unless the cap
/// or a promotion makes otherwise (09 §9.3).
pub const DWELL: Dur = Dur::from_secs(5);

/// The setting that caps how many minds are hot at once (09 §9.3).
pub const MAX_HOT_SETTING: &str = "sim.max_hot_minds";

/// The default of [`MAX_HOT_SETTING`].
pub const DEFAULT_MAX_HOT: i64 = 150;

/// What marks a wake-up of the `Stepper` domain as the warm stepper's.
pub const WAKE_WARM: u64 = 0x5741_524D;

/// `entity_promoted`: a character stepped warm became hot (03 §13).
pub const ENTITY_PROMOTED: &str = "entity_promoted";
/// `entity_demoted`: a hot character became warm (03 §13).
pub const ENTITY_DEMOTED: &str = "entity_demoted";

/// The events of levels, for the bus, the registry and the reference.
pub const LEVEL_EVENTS: &[EventInfo] = &[
    EventInfo {
        name: ENTITY_PROMOTED,
        subjects: &[("self", "entity")],
        fields: &[(
            "why",
            "string",
            "Why: `attention` (ranked under the cap at a warm step), `seen` (the party came to see it), `involved` (an action that started names it) or `acting` (its own mind was to start an action).",
        )],
        doc: "A character stepped warm became hot (09 §9.2): its senses sweep again and its mind thinks at once, going on with what it was doing.",
    },
    EventInfo {
        name: ENTITY_DEMOTED,
        subjects: &[("self", "entity")],
        fields: &[],
        doc: "A hot character became warm (09 §9.2): ranked beyond `sim.max_hot_minds`. It no longer sweeps, hears only loud things and is stepped in bulk; what it was doing runs on.",
    },
];

/// Why a warm character is promoted.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Why {
    /// Ranked under the cap at a warm step.
    Attention,
    /// The party came to see it.
    Seen,
    /// An action that started names it.
    Involved,
    /// Its own mind was to start an action.
    Acting,
}

impl Why {
    /// The word `entity_promoted` carries.
    pub const fn name(self) -> &'static str {
        match self {
            Why::Attention => "attention",
            Why::Seen => "seen",
            Why::Involved => "involved",
            Why::Acting => "acting",
        }
    }
}

/// How many of the minds in the world are hot and how many warm.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct Census {
    /// Minds stepped in full.
    pub hot: usize,
    /// Minds stepped in bulk.
    pub warm: usize,
}

/// A warm mind walking a step that moves in bulk: which step, and the
/// time it has in hand.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub struct Walk {
    /// Where the step goes: the pending step's target when the walk set
    /// out. A step begun anew, or another, ends the walk.
    target: Location,
    /// The step of its method it is.
    step: u16,
    /// Its own time not yet spent on steps.
    credit: Dur,
    /// Its own clock's time when `credit` was last brought up to date.
    from: Dur,
    /// How many steps in a row someone has stood in its way.
    held: u8,
}

impl_state_hash!(Walk {
    target,
    step,
    credit,
    from,
    held
});

/// How many warm steps in a row a walker waits for someone standing in
/// its way to move on before it tries the step, and its mind judges the
/// refusal.
pub const HELD_STEPS: u8 = 3;

/// What the simulation keeps about levels (09 §9). Saved with the
/// minds' state, and hashed only when it holds something.
#[derive(Clone, PartialEq, Eq, Debug, Default, Serialize, Deserialize)]
pub struct Levels {
    /// The characters stepped warm, with the tick each became warm.
    warm: BTreeMap<EntityId, Tick>,
    /// Characters that became hot again, with the tick, while it is
    /// less than [`DWELL`] ago.
    hot_since: BTreeMap<EntityId, Tick>,
    /// Warm minds walking a step that moves in bulk.
    walking: BTreeMap<EntityId, Walk>,
    /// Minds stepped down while an action walked a step that moves: it
    /// ends at its next phase.
    ending: BTreeSet<EntityId>,
    /// Warm minds due to think at the next step.
    due: BTreeSet<EntityId>,
    /// The stepper's next wake-up, while it runs.
    stepper: Option<(Tick, WakeId)>,
}

impl StateHash for Levels {
    fn state_hash(&self, hasher: &mut StateHasher) {
        self.warm.state_hash(hasher);
        self.hot_since.state_hash(hasher);
        self.walking.state_hash(hasher);
        self.ending.state_hash(hasher);
        self.due.state_hash(hasher);
        self.stepper.state_hash(hasher);
    }
}

impl Levels {
    /// Whether it holds nothing: the stepper is not running and nobody is
    /// warm.
    pub fn is_empty(&self) -> bool {
        self.warm.is_empty()
            && self.hot_since.is_empty()
            && self.walking.is_empty()
            && self.ending.is_empty()
            && self.due.is_empty()
            && self.stepper.is_none()
    }

    /// The characters stepped warm, in order of entity.
    pub fn warm(&self) -> impl Iterator<Item = EntityId> + '_ {
        self.warm.keys().copied()
    }

    /// Whether the stepper is running.
    pub fn stepping(&self) -> bool {
        self.stepper.is_some()
    }
}

// ---- reading ---------------------------------------------------------------

/// The cap in force.
pub fn max_hot(sim: &Sim) -> usize {
    usize::try_from(sim.setting_int(MAX_HOT_SETTING, DEFAULT_MAX_HOT).max(0)).unwrap_or(usize::MAX)
}

/// Whether a character is stepped warm.
pub fn is_warm(sim: &Sim, entity: EntityId) -> bool {
    !sim.ai.levels.warm.is_empty() && sim.ai.levels.warm.contains_key(&entity)
}

/// Whether a mind walks a step that moves in bulk.
pub fn is_walking(sim: &Sim, entity: EntityId) -> bool {
    !sim.ai.levels.walking.is_empty() && sim.ai.levels.walking.contains_key(&entity)
}

/// How many of the minds standing in the world are hot and how many
/// warm.
pub fn census(sim: &Sim) -> Census {
    let mut out = Census::default();
    for (entity, _) in sim.components.mind_state.iter() {
        if sim.entities.location(entity).is_none() {
            continue;
        }
        if is_warm(sim, entity) {
            out.warm += 1;
        } else {
            out.hot += 1;
        }
    }
    out
}

// ---- the stepper -----------------------------------------------------------

/// What a mind's thought calls first: starts the stepper if levels may
/// be needed, because more minds have thought than the cap allows.
pub(super) fn notice(sim: &mut Sim) {
    if sim.ai.levels.stepper.is_some() {
        return;
    }
    if sim.components.mind_state.len() > max_hot(sim) {
        schedule(sim);
    }
}

/// Puts the stepper's next wake-up on the timeline, if it has none.
fn schedule(sim: &mut Sim) {
    if sim.ai.levels.stepper.is_some() {
        return;
    }
    if let Ok(id) = sim.schedule_on(
        ClockId::UNIVERSAL,
        WARM_STEP,
        Wake::new(WakeDomain::Stepper, 0, WAKE_WARM),
    ) {
        sim.ai.levels.stepper = Some((sim.now() + WARM_STEP, id));
    }
}

/// The warm stepper's wake-up (03 §10): levels are worked out again,
/// warm minds walking in bulk take their steps, and warm minds due
/// think. It wakes again while anyone is warm.
pub(crate) fn on_wake(sim: &mut Sim, id: WakeId) {
    if sim.ai.levels.stepper.is_none_or(|(_, have)| have != id) {
        return;
    }
    sim.ai.levels.stepper = None;
    attend(sim);
    walk(sim);
    think_warm(sim);
    // Nobody warm: the stepper stops. Those promoted at this step go on
    // with what they walked when they think, at once.
    if sim.ai.levels.warm.is_empty() {
        let levels = &mut sim.ai.levels;
        levels.hot_since.clear();
        levels.due.clear();
    } else {
        schedule(sim);
    }
}

// ---- attention -------------------------------------------------------------

/// How a mind ranks: the lower the earlier.
type Rank = (u8, u8, u8, i64, EntityId);

/// Works levels out again (see the [module](self)).
fn attend(sim: &mut Sim) {
    let now = sim.now();
    let cap = max_hot(sim);
    let mut principals: BTreeSet<EntityId> = BTreeSet::new();
    let mut anchors: Vec<Location> = Vec::new();
    for record in sim.entities.iter() {
        if record.control() == Control::Direct {
            principals.insert(record.id());
            if let Some(loc) = sim.entities.location(record.id()) {
                anchors.push(loc);
            }
        }
    }
    let mut ticketed: BTreeSet<EntityId> = BTreeSet::new();
    let mut ticket_anchors: Vec<Location> = Vec::new();
    for (_, ticket) in sim.cosmos.tickets().iter() {
        if ticket.level != Activity::Hot {
            continue;
        }
        match ticket.anchor {
            TicketAnchor::Entity(entity) => {
                ticketed.insert(entity);
                principals.insert(entity);
                if let Some(loc) = sim.entities.location(entity) {
                    ticket_anchors.push(loc);
                }
            }
            TicketAnchor::Fixed(tract) => {
                if let Some(realm) = sim.cosmos.realm(ticket.realm) {
                    let half = clash_world::dims::TRACT_XY / 2;
                    ticket_anchors.push(Location::new(
                        realm.base_grid,
                        clash_world::CellPos::new(
                            tract.x * clash_world::dims::TRACT_XY + half,
                            tract.y * clash_world::dims::TRACT_XY + half,
                            0,
                        ),
                    ));
                }
            }
            _ => {}
        }
    }
    if anchors.is_empty() {
        anchors = ticket_anchors;
    }
    let involved = involved_with(sim, &principals);

    let mut ranked: Vec<Rank> = Vec::new();
    let mut known: BTreeSet<EntityId> = BTreeSet::new();
    for (entity, _) in sim.components.mind_state.iter() {
        let Some(record) = sim.entities.get(entity) else {
            continue;
        };
        if record.control() == Control::Direct {
            continue;
        }
        let Some(loc) = sim.entities.location(entity) else {
            continue;
        };
        if think::mind_def_of(sim, entity).is_none() {
            continue;
        }
        known.insert(entity);
        let class = if ticketed.contains(&entity) {
            1
        } else if involved.contains(&entity) {
            2
        } else if sim.is_notable(entity) {
            3
        } else {
            4
        };
        let seen = crate::perception::party::sees(sim, loc);
        let warm_since = sim.ai.levels.warm.get(&entity).copied();
        let group = if class <= 2 || seen {
            0
        } else if warm_since.is_some_and(|since| since + DWELL > now) {
            3
        } else if warm_since.is_none()
            && sim
                .ai
                .levels
                .hot_since
                .get(&entity)
                .is_some_and(|since| *since + DWELL > now)
        {
            1
        } else {
            2
        };
        let distance = anchors
            .iter()
            .filter_map(|anchor| sim.cosmos.delta(*anchor, loc))
            .map(clash_world::distance_sq_cells)
            .min()
            .unwrap_or(i64::MAX);
        ranked.push((group, class, u8::from(!seen), distance, entity));
    }
    ranked.sort_unstable();

    let mut slots = cap;
    let mut up = Vec::new();
    let mut down = Vec::new();
    for (group, _, _, _, entity) in ranked {
        let hot = if group == 0 {
            true
        } else if group == 3 || slots == 0 {
            false
        } else {
            true
        };
        if hot {
            slots = slots.saturating_sub(1);
        }
        let now_hot = !sim.ai.levels.warm.contains_key(&entity);
        if hot && !now_hot {
            up.push(entity);
        } else if !hot && now_hot {
            down.push(entity);
        }
    }
    for entity in down {
        demote(sim, entity);
    }
    for entity in up {
        promote(sim, entity, Why::Attention);
    }
    // Whoever left the world, or came under direct control, is no
    // longer anyone's level to keep.
    let levels = &mut sim.ai.levels;
    levels.warm.retain(|entity, _| known.contains(entity));
    levels.hot_since.retain(|_, since| *since + DWELL > now);
    levels
        .walking
        .retain(|entity, _| levels.warm.contains_key(entity) || known.contains(entity));
    levels.ending.retain(|entity| levels.warm.contains_key(entity));
    levels.due.retain(|entity| levels.warm.contains_key(entity));
}

/// Everyone sharing a running action, an open window or a grip with one
/// of `principals` (09 §9.3's second class).
fn involved_with(sim: &Sim, principals: &BTreeSet<EntityId>) -> BTreeSet<EntityId> {
    let mut out = BTreeSet::new();
    if principals.is_empty() {
        return out;
    }
    let join = |cast: &[EntityId], out: &mut BTreeSet<EntityId>| {
        if cast.iter().any(|entity| principals.contains(entity)) {
            out.extend(cast.iter().copied());
        }
    };
    for (id, action) in sim.actions.iter() {
        let mut cast = cast_of(sim, action.actor, &action.locals);
        cast.extend(sim.actions.joined(id).iter().map(|joined| joined.entity));
        join(&cast, &mut out);
    }
    for (_, window) in sim.windows.iter() {
        let mut cast: Vec<EntityId> = window.candidates.iter().map(|c| c.entity).collect();
        cast.extend(window.source);
        cast.extend(window.aimed_at.iter().copied());
        join(&cast, &mut out);
    }
    for grip in sim.grips.iter() {
        join(&[grip.holder, grip.held], &mut out);
    }
    out
}

/// The actor of an action and every entity among its parameters.
fn cast_of(sim: &Sim, actor: EntityId, locals: &[clash_core::Value]) -> Vec<EntityId> {
    let mut cast = vec![actor];
    for value in locals {
        if let Some(Subject::Entity(entity)) = sim.subject_of(&clash_expr::Value::from(value))
            && !cast.contains(&entity)
        {
            cast.push(entity);
        }
    }
    cast
}

/// An action is starting (03 §10): every warm character it names (its
/// actor, an entity among its parameters, a participant) is promoted
/// before it starts.
pub(crate) fn involve(sim: &mut Sim, actor: EntityId, locals: &[clash_expr::Value], joined: &[EntityId]) {
    if sim.ai.levels.warm.is_empty() {
        return;
    }
    let mut cast = vec![actor];
    cast.extend(joined.iter().copied());
    for value in locals {
        if let Some(Subject::Entity(entity)) = sim.subject_of(value) {
            cast.push(entity);
        }
    }
    for entity in cast {
        promote(sim, entity, Why::Involved);
    }
}

/// What the party saw at an observation: every warm character standing
/// there is promoted, so that the party perceives only hot characters
/// at full detail (09 §9.3).
pub(crate) fn seen_by_party(sim: &mut Sim, entities: &[EntityId]) {
    if sim.ai.levels.warm.is_empty() {
        return;
    }
    for entity in entities {
        promote(sim, *entity, Why::Seen);
    }
}

// ---- moving between levels -------------------------------------------------

/// A warm character becomes hot (09 §9.2): its sweeps begin again and
/// it thinks at once; a step it walked in bulk is begun again at that
/// thought. Nothing for one that is not warm.
pub(crate) fn promote(sim: &mut Sim, entity: EntityId, why: Why) {
    let now = sim.now();
    let levels = &mut sim.ai.levels;
    if levels.warm.remove(&entity).is_none() {
        return;
    }
    levels.hot_since.insert(entity, now);
    levels.due.remove(&entity);
    levels.ending.remove(&entity);
    sim.ai.due.insert(entity);
    crate::perception::resume_sweeps(sim, entity);
    let payload = EventPayload::new(ENTITY_PROMOTED)
        .subject("self", Subject::Entity(entity))
        .with("why", why.name());
    sim.emit(
        Where::Entity(entity),
        think::cause_of(entity),
        EventBody::Custom(payload),
    );
}

/// A hot character becomes warm (09 §9.2): what it does runs on, but an
/// action walking a step that moves ends at its next phase, and its path
/// becomes the route it walks in bulk.
fn demote(sim: &mut Sim, entity: EntityId) {
    let now = sim.now();
    let levels = &mut sim.ai.levels;
    levels.warm.insert(entity, now);
    levels.hot_since.remove(&entity);
    if sim.ai.due.remove(&entity) {
        sim.ai.levels.due.insert(entity);
    }
    let moving = sim
        .components
        .mind_state
        .get(entity)
        .and_then(|state| state.held())
        .and_then(|held| held.pending.as_ref())
        .filter(|pending| pending.outcome == Outcome::Running && pending.door == Door::None)
        .filter(|pending| {
            matches!(
                pending.kind(),
                StepKind::Go | StepKind::KeepNear | StepKind::KeepFrom
            )
        })
        .and_then(|pending| pending.action)
        .is_some_and(|id| sim.actions.get(id).is_some());
    if moving {
        sim.ai.levels.ending.insert(entity);
    }
    let payload = EventPayload::new(ENTITY_DEMOTED).subject("self", Subject::Entity(entity));
    sim.emit(
        Where::Entity(entity),
        think::cause_of(entity),
        EventBody::Custom(payload),
    );
}

/// What the kernel calls for every event, through the minds': a stepped
/// down mind's walking action ends at its next phase.
pub(super) fn on_event(sim: &mut Sim, event: &SimEvent) {
    if sim.ai.levels.is_empty() {
        return;
    }
    // One that leaves the world is nobody's level to keep.
    match &event.body {
        EventBody::EntityRemoved { entity }
        | EventBody::EntityRelocated {
            entity,
            to: crate::entity::Place::Stored,
            ..
        } => {
            forget(sim, *entity);
            return;
        }
        _ => {}
    }
    if sim.ai.levels.ending.is_empty() {
        return;
    }
    let EventBody::Act(act) = &event.body else {
        return;
    };
    match act {
        ActEvent::PhaseEntered { actor, id, .. } if sim.ai.levels.ending.contains(actor) => {
            end_walking_action(sim, *actor, *id);
        }
        ActEvent::ActionFinished { actor, .. }
        | ActEvent::ActionInterrupted { actor, .. }
        | ActEvent::ActionCancelled { actor, .. }
        | ActEvent::ActionAbandoned { actor, .. } => {
            sim.ai.levels.ending.remove(actor);
        }
        _ => {}
    }
}

/// Forgets whatever levels keep of a character.
fn forget(sim: &mut Sim, entity: EntityId) {
    let levels = &mut sim.ai.levels;
    levels.warm.remove(&entity);
    levels.hot_since.remove(&entity);
    levels.walking.remove(&entity);
    levels.ending.remove(&entity);
    levels.due.remove(&entity);
}

/// Ends the action a warm mind walked a step with, at a phase boundary,
/// keeping its path as the route it walks in bulk.
fn end_walking_action(sim: &mut Sim, mind: EntityId, id: crate::later::ActionId) {
    let ours = sim
        .components
        .mind_state
        .get(mind)
        .and_then(|state| state.held())
        .and_then(|held| held.pending.as_ref())
        .is_some_and(|pending| pending.action == Some(id));
    if !ours || !is_warm(sim, mind) {
        return;
    }
    sim.ai.levels.ending.remove(&mind);
    let journey = crate::movement::journey_of(sim, mind);
    think::let_go(sim, mind);
    if sim
        .cancel_action(mind, id, true, think::cause_of(mind))
        .is_err()
    {
        // A phase that cannot be cancelled: the action runs on, and its
        // ending is the step's again.
        think::take_back(sim, mind, id);
        return;
    }
    crate::movement::keep_journey(sim, mind, journey);
    if let Some((target, step)) = walked_step(sim, mind) {
        start_walk(sim, mind, target, step);
    }
}

/// The step a mind is on that moves, if one is under way: where it goes
/// and its place in the method.
fn walked_step(sim: &Sim, mind: EntityId) -> Option<(Location, u16)> {
    let held = sim.components.mind_state.get(mind)?.held()?;
    let pending = held.pending.as_ref()?;
    let moves = matches!(
        pending.kind(),
        StepKind::Go | StepKind::KeepNear | StepKind::KeepFrom
    );
    (moves && pending.outcome == Outcome::Running).then_some((pending.target?, held.step))
}

/// A warm mind sets out on a step that moves in bulk, toward `target`,
/// the step `step` of its method. Time it had in hand on a walk it was
/// already on is kept.
pub(super) fn start_walk(sim: &mut Sim, mind: EntityId, target: Location, step: u16) {
    let from = think::local_now(sim, mind);
    let credit = sim
        .ai
        .levels
        .walking
        .get(&mind)
        .map_or(Dur::ZERO, |walk| walk.credit);
    sim.ai.levels.walking.insert(
        mind,
        Walk {
            target,
            step,
            credit,
            from,
            held: 0,
        },
    );
}

/// A mind that walked in bulk and is hot again: it stops walking so,
/// and its path is forgotten for its `go_action` to plan afresh.
/// Returns whether it was walking.
pub(super) fn stop_walk(sim: &mut Sim, mind: EntityId) -> bool {
    if sim.ai.levels.walking.remove(&mind).is_none() {
        return false;
    }
    crate::movement::clear_path(sim, mind);
    true
}

// ---- walking in bulk -------------------------------------------------------

/// Every warm mind walking a step that moves takes the steps of its path
/// that the time it has had allows (09 §9.1).
fn walk(sim: &mut Sim) {
    let walkers: Vec<EntityId> = sim.ai.levels.walking.keys().copied().collect();
    for mind in walkers {
        if is_warm(sim, mind) {
            walk_one(sim, mind);
        }
    }
}

/// One warm mind's walk: the steps of its path the time it has in hand
/// allows. A step that is refused ends the walk, and its mind judges the
/// step at its next thought, as it judges a `go_action` that ended
/// short: a closed way it opens once promoted (it starts an action), and
/// for anything else its methods say what then (D-893).
fn walk_one(sim: &mut Sim, mind: EntityId) {
    let Some(mut walk) = sim.ai.levels.walking.get(&mind).copied() else {
        return;
    };
    let now = think::local_now(sim, mind);
    walk.credit += (now - walk.from).max(Dur::ZERO);
    walk.from = now;
    // The step it set out on, still under way: anything else (a goal
    // taken up in its place, the step begun anew) ends the walk.
    if walked_step(sim, mind) != Some((walk.target, walk.step)) {
        stop_walk(sim, mind);
        sim.ai.levels.due.insert(mind);
        return;
    }
    let within = sim
        .components
        .mind_state
        .get(mind)
        .and_then(|state| state.held())
        .and_then(|held| held.pending.as_ref())
        .map_or(clash_core::Real::ZERO, |pending| pending.within);
    let target = walk.target;
    loop {
        if super::nav::distance_to(sim, mind, target)
            .is_some_and(|d| d <= within + clash_core::Real::of(think::ARRIVED_SLACK_M))
        {
            arrive(sim, mind);
            return;
        }
        // The next cell of a path that can still be followed; with none,
        // the path ran out short of the place, or must be planned again:
        // the mind judges it.
        let next = sim
            .components()
            .get::<crate::movement::Mover>(mind)
            .and_then(crate::movement::Mover::journey)
            .filter(|journey| !journey.is_stale())
            .and_then(crate::movement::Journey::next);
        let (Some(next), Some(here)) = (next, sim.entities.location(mind)) else {
            arrive(sim, mind);
            return;
        };
        let kind = sim
            .cosmos
            .delta(here, Location::new(here.grid, next))
            .and_then(|delta| {
                clash_world::StepKind::ALL
                    .into_iter()
                    .find(|kind| kind.delta() == delta)
            });
        let time = kind
            .and_then(|kind| crate::movement::believed_step_time(sim, mind, kind))
            .unwrap_or(WARM_STEP);
        if time > walk.credit {
            break;
        }
        // Someone standing in the way is waited for a few steps, as
        // people step aside, before the step is tried and refused.
        let in_the_way = sim
            .cosmos
            .occupants_at(
                Location::new(here.grid, next),
                &OccupantFilter::ANY
                    .with_flags(OccupantFlags::BLOCKS_MOVE)
                    .except(mind),
            )
            .is_ok_and(|found| !found.is_empty());
        if in_the_way && walk.held < HELD_STEPS {
            walk.held += 1;
            walk.credit = Dur::ZERO;
            break;
        }
        walk.held = 0;
        if crate::movement::follow_path(sim, mind, think::cause_of(mind)).is_err() {
            stop_walk(sim, mind);
            sim.ai.levels.due.insert(mind);
            return;
        }
        walk.credit -= time;
        // Come into the party's view: hot from here on, as the party
        // perceives only hot characters (09 §9.3).
        if sim
            .entities
            .location(mind)
            .is_some_and(|loc| crate::perception::party::sees(sim, loc))
        {
            if let Some(have) = sim.ai.levels.walking.get_mut(&mind) {
                *have = walk;
            }
            promote(sim, mind, Why::Seen);
            return;
        }
    }
    // Time short of the next step is kept for it.
    if let Some(have) = sim.ai.levels.walking.get_mut(&mind) {
        *have = walk;
    }
}

/// A warm mind's walk is over, there or short of it: its mind judges it
/// at its next thought.
fn arrive(sim: &mut Sim, mind: EntityId) {
    sim.ai.levels.walking.remove(&mind);
    sim.ai.levels.due.insert(mind);
}

// ---- thinking --------------------------------------------------------------

/// A prompted mind that is warm: it thinks at the next step.
pub(super) fn defer(sim: &mut Sim, mind: EntityId) -> bool {
    if !is_warm(sim, mind) {
        return false;
    }
    sim.ai.levels.due.insert(mind);
    true
}

/// Warm minds due think, in order of entity.
fn think_warm(sim: &mut Sim) {
    let due = core::mem::take(&mut sim.ai.levels.due);
    for mind in due {
        if is_warm(sim, mind) {
            think::think(sim, mind);
        } else {
            sim.ai.due.insert(mind);
        }
    }
}
