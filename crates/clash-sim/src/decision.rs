//! Decisions: what a directly controlled entity is asked, and the queue
//! of requests waiting for an answer (03 §5.2).
//!
//! The simulation never waits. When a directly controlled entity must
//! choose, a request joins the [`DecisionQueue`] and
//! [`Sim::advance`](crate::Sim::advance) returns it instead of moving
//! time. Requests are returned one at a time, lowest entity id first,
//! and time does not move until each has been answered (invariant T8).
//!
//! The kernel owns the queue and its order. What raises a request (an
//! action ending with an empty plan, a rule interrupting, a window, an
//! `ask` effect) and which commands answer one belong to packages T4, T5
//! and K9, which call [`Sim::request_decision`] and
//! [`Sim::answer_decision`].

use std::collections::{BTreeMap, VecDeque};

use clash_core::{Dur, EntityId, StateHash, StateHasher, define_id};
use clash_world::Cause;
use serde::{Deserialize, Serialize};

use crate::Sim;
use crate::entity::Control;
use crate::event::{EventBody, Where};
use crate::later::ActionId;
use crate::reason::{Reason, codes};

define_id! {
    /// A choice put by a rule's `ask` (05 §10.6): never reused within
    /// one simulation.
    pub struct ChoiceId(u64);
}

/// One option of a choice, as whoever chooses is shown it.
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub struct ChoiceOption {
    /// What it says, rendered for the one who chooses.
    pub label: String,
    /// Whether it may be taken: its `if` held when the choice was put.
    /// One that may not is shown and refused.
    pub available: bool,
}

impl StateHash for ChoiceOption {
    fn state_hash(&self, hasher: &mut StateHasher) {
        hasher.write_str(&self.label);
        self.available.state_hash(hasher);
    }
}

/// One way to answer a window (03 §7.3), as whoever is asked is shown
/// it: a reaction that lists the window's kind.
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub struct ReactionOption {
    /// The reaction.
    pub action: crate::action::ActionDefId,
    /// What it is called, rendered for the one asked: its `name`, or
    /// its id.
    pub label: String,
    /// Whether its `ready` phase would begin before the window is
    /// expected to close (03 §7.3); true where the deadline is not
    /// known. One that would not may still be chosen.
    pub in_time: bool,
    /// Why it cannot be started now, if it cannot: the reactor's own
    /// actions hold the slots it needs in phases that cannot be
    /// cancelled (`engine:reaction_cannot_cancel`), its `requires`
    /// fails, and the rest of 03 §6.6. One with a reason is shown and
    /// refused.
    pub refused: Option<Reason>,
}

impl StateHash for ReactionOption {
    fn state_hash(&self, hasher: &mut StateHasher) {
        self.action.state_hash(hasher);
        hasher.write_str(&self.label);
        self.in_time.state_hash(hasher);
        self.refused.state_hash(hasher);
    }
}

/// A request for a directly controlled entity to choose (03 §5.2).
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub struct DecisionRequest {
    /// Who must choose.
    pub entity: EntityId,
    /// What is being asked.
    pub kind: DecisionKind,
    /// How much of the entity's own time has passed since it last chose
    /// (03 §12): local time on whatever clocks it has been on.
    pub elapsed: Dur,
}

/// What is being asked. Packages T5 and K9 add the reaction to a window
/// and the choice put by an `ask` effect: match with a wildcard arm.
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum DecisionKind {
    /// What to do next.
    Free {
        /// Why the entity is free to choose.
        why: FreeReason,
    },
    /// One of several options, put by a rule's `ask` (05 §10.6). The
    /// `Choose` command answers with the position of an option.
    Choice {
        /// Which choice: what `Choose` names.
        choice: ChoiceId,
        /// The question, rendered for the one who chooses.
        prompt: String,
        /// The options, in the order written.
        options: Vec<ChoiceOption>,
    },
    /// How to answer a window the entity noticed before it closes
    /// (03 §5.2, §7.3), put by `ask_reaction`. `React` answers with one
    /// of the options' actions; `Pass` declines.
    Reaction {
        /// The window.
        window: crate::action::WindowId,
        /// The universal tick the window is expected to close at; `None`
        /// where that is not known.
        deadline: Option<clash_core::Tick>,
        /// The reactions that list the window's kind and may answer it
        /// for this entity, in order of definition.
        options: Vec<ReactionOption>,
    },
}

/// Why an entity needs a free decision.
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum FreeReason {
    /// It has just come under direct control, or just entered the world.
    Started,
    /// A foreground action of its own ended and its plan is empty.
    Finished(ActionId),
    /// A rule interrupted what it was doing.
    Interrupted(Cause),
    /// Its plan ran out or was cleared; the reason says why if a step
    /// could not be started.
    PlanEmpty(Option<Reason>),
    /// A wait it asked for is over.
    Waited,
}

impl StateHash for DecisionKind {
    fn state_hash(&self, hasher: &mut StateHasher) {
        match self {
            DecisionKind::Free { why } => {
                hasher.write_u8(0);
                why.state_hash(hasher);
            }
            DecisionKind::Choice {
                choice,
                prompt,
                options,
            } => {
                hasher.write_u8(1);
                choice.state_hash(hasher);
                hasher.write_str(prompt);
                options.state_hash(hasher);
            }
            DecisionKind::Reaction {
                window,
                deadline,
                options,
            } => {
                hasher.write_u8(2);
                window.state_hash(hasher);
                deadline.state_hash(hasher);
                options.state_hash(hasher);
            }
        }
    }
}

impl StateHash for FreeReason {
    fn state_hash(&self, hasher: &mut StateHasher) {
        match self {
            FreeReason::Started => hasher.write_u8(0),
            FreeReason::Finished(action) => {
                hasher.write_u8(1);
                action.state_hash(hasher);
            }
            FreeReason::Interrupted(cause) => {
                hasher.write_u8(2);
                cause.state_hash(hasher);
            }
            FreeReason::PlanEmpty(reason) => {
                hasher.write_u8(3);
                reason.state_hash(hasher);
            }
            FreeReason::Waited => hasher.write_u8(4),
        }
    }
}

/// The requests waiting for an answer: for each entity, in the order
/// they were raised.
#[derive(Clone, PartialEq, Eq, Default, Debug, Serialize, Deserialize)]
pub struct DecisionQueue {
    pending: BTreeMap<EntityId, VecDeque<DecisionKind>>,
}

impl DecisionQueue {
    /// Whether nothing is waiting.
    pub fn is_empty(&self) -> bool {
        self.pending.is_empty()
    }

    /// How many requests are waiting.
    pub fn len(&self) -> usize {
        self.pending.values().map(VecDeque::len).sum()
    }

    /// The request to answer next: the oldest of the entity with the
    /// lowest id.
    pub fn front(&self) -> Option<(EntityId, &DecisionKind)> {
        let (entity, kinds) = self.pending.iter().next()?;
        kinds.front().map(|kind| (*entity, kind))
    }

    /// The oldest request waiting for an entity.
    pub fn front_of(&self, entity: EntityId) -> Option<&DecisionKind> {
        self.pending.get(&entity)?.front()
    }

    /// The entities with a request waiting, in id order.
    pub fn entities(&self) -> impl Iterator<Item = EntityId> + '_ {
        self.pending.keys().copied()
    }

    fn push(&mut self, entity: EntityId, kind: DecisionKind) {
        self.pending.entry(entity).or_default().push_back(kind);
    }

    fn pop(&mut self, entity: EntityId) -> Option<DecisionKind> {
        let kinds = self.pending.get_mut(&entity)?;
        let kind = kinds.pop_front();
        if kinds.is_empty() {
            self.pending.remove(&entity);
        }
        kind
    }

    /// Forgets every request of an entity.
    pub(crate) fn forget(&mut self, entity: EntityId) {
        self.pending.remove(&entity);
    }

    /// Whether an entity is being asked what to do next.
    pub fn has_free(&self, entity: EntityId) -> bool {
        self.pending.get(&entity).is_some_and(|kinds| {
            kinds
                .iter()
                .any(|kind| matches!(kind, DecisionKind::Free { .. }))
        })
    }

    /// The choices an entity is being asked to make, oldest first.
    pub fn choices_of(&self, entity: EntityId) -> impl Iterator<Item = ChoiceId> + '_ {
        self.pending
            .get(&entity)
            .into_iter()
            .flatten()
            .filter_map(|kind| match kind {
                DecisionKind::Choice { choice, .. } => Some(*choice),
                _ => None,
            })
    }

    /// Takes out the first request of an entity that `which` picks.
    pub(crate) fn take(
        &mut self,
        entity: EntityId,
        which: impl Fn(&DecisionKind) -> bool,
    ) -> Option<DecisionKind> {
        let kinds = self.pending.get_mut(&entity)?;
        let at = kinds.iter().position(which)?;
        let kind = kinds.remove(at);
        if kinds.is_empty() {
            self.pending.remove(&entity);
        }
        kind
    }
}

impl StateHash for DecisionQueue {
    fn state_hash(&self, hasher: &mut StateHasher) {
        self.pending.state_hash(hasher);
    }
}

impl Sim {
    /// The decisions waiting for an answer.
    pub fn decisions(&self) -> &DecisionQueue {
        &self.decisions
    }

    /// The request [`advance`](Sim::advance) will return next, if a
    /// decision is waiting, with the time that has passed for its entity.
    pub fn pending_decision(&self) -> Option<DecisionRequest> {
        let (entity, kind) = self.decisions.front()?;
        Some(DecisionRequest {
            entity,
            kind: kind.clone(),
            elapsed: self.since_decided(entity),
        })
    }

    /// Asks a directly controlled entity to choose. Time will not move
    /// until it has (invariant T8). Emits `decision_requested`.
    ///
    /// Returns `Ok(false)`, asking nothing, for an entity that is not
    /// under direct control: the AI layer decides for those, and whoever
    /// called turns to it. Refused for an unknown entity.
    pub fn request_decision(
        &mut self,
        entity: EntityId,
        kind: DecisionKind,
        cause: Cause,
    ) -> Result<bool, Reason> {
        let control = self.known(entity)?.control();
        if control != Control::Direct {
            return Ok(false);
        }
        self.decisions.push(entity, kind.clone());
        self.emit(
            Where::Entity(entity),
            cause,
            EventBody::DecisionRequested { entity, kind },
        );
        Ok(true)
    }

    /// Takes the oldest request waiting for an entity as answered, and
    /// starts counting the entity's time since it decided afresh. For
    /// the commands and effects that answer (`Act`, `Wait`, `React`,
    /// `Pass`, `Choose`): they call this once they have accepted an
    /// answer.
    ///
    /// Refused if the entity is unknown or is not waiting for a decision.
    pub fn answer_decision(&mut self, entity: EntityId) -> Result<DecisionKind, Reason> {
        self.known(entity)?;
        let Some(kind) = self.decisions.pop(entity) else {
            return Err(Reason::new(
                codes::DECISION_NONE_PENDING,
                format!("entity {entity} is not waiting for a decision"),
            )
            .with("entity", entity.to_string()));
        };
        let own = self.own_time(entity);
        if let Some(record) = self.entities.get_mut(entity) {
            record.time.decided = own;
        }
        Ok(kind)
    }

    /// Takes the first request of an entity that `which` picks as
    /// answered, and starts counting the entity's time since it decided
    /// afresh. `None` if it is waiting for no such request.
    pub(crate) fn answer_which(
        &mut self,
        entity: EntityId,
        which: impl Fn(&DecisionKind) -> bool,
    ) -> Option<DecisionKind> {
        let kind = self.decisions.take(entity, which)?;
        let own = self.own_time(entity);
        if let Some(record) = self.entities.get_mut(entity) {
            record.time.decided = own;
        }
        Some(kind)
    }

    /// How much of an entity's own time has passed since it last decided.
    fn since_decided(&self, entity: EntityId) -> Dur {
        let decided = self
            .entities
            .get(entity)
            .map_or(Dur::ZERO, |record| record.time.decided);
        self.own_time(entity) - decided
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn requests_come_lowest_entity_first_and_oldest_first_within_one() {
        let mut queue = DecisionQueue::default();
        let (low, high) = (EntityId::new(1, 0), EntityId::new(5, 0));
        let free = |why| DecisionKind::Free { why };
        queue.push(high, free(FreeReason::Started));
        queue.push(low, free(FreeReason::PlanEmpty(None)));
        queue.push(low, free(FreeReason::Started));
        assert_eq!(queue.len(), 3);
        assert_eq!(
            queue.front(),
            Some((low, &free(FreeReason::PlanEmpty(None))))
        );
        assert_eq!(queue.pop(low), Some(free(FreeReason::PlanEmpty(None))));
        assert_eq!(queue.front(), Some((low, &free(FreeReason::Started))));
        queue.pop(low);
        assert_eq!(queue.front(), Some((high, &free(FreeReason::Started))));
        assert_eq!(queue.entities().collect::<Vec<_>>(), [high]);
        queue.forget(high);
        assert!(queue.is_empty());
    }
}
