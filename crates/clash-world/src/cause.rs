//! [`Cause`]: who and what is behind a change to the world (02 §7.3).

use clash_core::{EntityId, StateHash, StateHasher};
use serde::{Deserialize, Serialize};

/// The kind of thing that made a change.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CauseKind {
    /// An entity's action.
    Action,
    /// A world process: a blast, a collapse, a body in flight.
    Process,
    /// A field acting on its cell.
    Field,
    /// A rule's effect.
    Eoc,
    /// World generation.
    Generator,
    /// A coarse change applied when a tract loaded.
    Coarse,
    /// A tool or a test.
    Debug,
}

impl StateHash for CauseKind {
    fn state_hash(&self, hasher: &mut StateHasher) {
        hasher.write_u8(*self as u8);
    }
}

/// Who and what is behind a change to the world.
///
/// Processes pass the instigator along. When someone throws a body
/// through a wall, the wall's destruction has the body as actor and the
/// thrower as instigator. Causes are ground truth: what witnesses believe
/// happened is a separate matter (01 R11).
///
/// ```
/// use clash_core::EntityId;
/// use clash_world::{Cause, CauseKind};
///
/// let thrower = EntityId::new(1, 0);
/// let thrown = EntityId::new(2, 0);
/// let cause = Cause::new(CauseKind::Process).by(thrown).instigated_by(thrower);
/// assert_eq!(cause.actor, Some(thrown));
/// assert_eq!(cause.instigator, Some(thrower));
/// ```
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Serialize, Deserialize)]
pub struct Cause {
    /// What physically did it. May be a thrown body.
    pub actor: Option<EntityId>,
    /// Who is responsible.
    pub instigator: Option<EntityId>,
    /// The kind of thing that made the change.
    pub kind: CauseKind,
}

impl Cause {
    /// A cause of a kind, with no actor and no instigator.
    pub const fn new(kind: CauseKind) -> Cause {
        Cause {
            actor: None,
            instigator: None,
            kind,
        }
    }

    /// The cause of a tool or a test.
    pub const DEBUG: Cause = Cause::new(CauseKind::Debug);

    /// The cause of world generation.
    pub const GENERATOR: Cause = Cause::new(CauseKind::Generator);

    /// This cause with an actor. The actor is also the instigator unless
    /// one is named.
    pub const fn by(mut self, actor: EntityId) -> Cause {
        self.actor = Some(actor);
        if self.instigator.is_none() {
            self.instigator = Some(actor);
        }
        self
    }

    /// This cause with an instigator.
    pub const fn instigated_by(mut self, instigator: EntityId) -> Cause {
        self.instigator = Some(instigator);
        self
    }
}

impl StateHash for Cause {
    fn state_hash(&self, hasher: &mut StateHasher) {
        self.actor.state_hash(hasher);
        self.instigator.state_hash(hasher);
        self.kind.state_hash(hasher);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_actor_is_responsible_unless_someone_else_is_named() {
        let actor = EntityId::new(4, 0);
        let alone = Cause::new(CauseKind::Action).by(actor);
        assert_eq!(alone.instigator, Some(actor));
        let other = EntityId::new(9, 2);
        let pushed = Cause::new(CauseKind::Process)
            .instigated_by(other)
            .by(actor);
        assert_eq!(pushed.instigator, Some(other));
        assert_eq!(pushed.actor, Some(actor));
        assert_eq!(Cause::DEBUG.actor, None);
    }
}
