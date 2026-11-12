//! What the action runtime keeps (03 §5.3, §6): the running actions, the
//! waits, each entity's channel slots and plan. All of it is plain data
//! that saves and hashes; nothing here decides anything. The runtime
//! that changes it is [`run`](super::run).

use std::collections::BTreeMap;

use clash_core::{Dur, EntityId, StateHash, StateHasher, Tick, impl_state_hash};
use clash_time::WakeId;
use serde::{Deserialize, Serialize};

use super::ids::{ActionDefId, ChannelId, PhaseIndex};
use crate::later::ActionId;

/// The parameters a command or an effect gives an action, by name, in
/// any order. A parameter left out takes its default (03 §6.3).
pub type Params = Vec<(String, clash_core::Value)>;

/// Whether `Act` starts an action now or puts it at the end of the plan
/// (03 §5.3).
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum How {
    /// Start it now: refused if it cannot start.
    #[default]
    Start,
    /// Plan it: it starts when the actions before it in the plan have,
    /// and its channels are free.
    Enqueue,
}

impl clash_core::StateHash for How {
    fn state_hash(&self, hasher: &mut clash_core::StateHasher) {
        hasher.write_u8(*self as u8);
    }
}

/// One action that is running (03 §6.6): which definition, who, the
/// values of its locals, where it is in its phases and the wake-ups
/// that will move it on.
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub struct RunningAction {
    /// Who acts.
    pub actor: EntityId,
    /// What is being done.
    pub def: ActionDefId,
    /// The phase it is in.
    pub phase: PhaseIndex,
    /// One value for each of the definition's locals, in slot order:
    /// the window, the roles, the parameters and the variables.
    pub locals: Vec<clash_core::Value>,
    /// The slots it holds of each channel, in order of channel.
    pub held: Vec<(ChannelId, u16)>,
    /// The wake-up that ends the phase: its duration, or a held phase's
    /// `max`. `None` for a held phase with no `max`.
    pub wake: Option<WakeId>,
    /// The wake-up of the phase's next tick.
    pub tick: Option<WakeId>,
    /// The interval of the phase's tick, as worked out when it began.
    pub every: Option<Dur>,
    /// The universal tick the phase began at.
    pub entered: Tick,
    /// The universal tick the phase is due to end at as the clocks ran
    /// when it began, if it has an end.
    pub due: Option<Tick>,
    /// How many phases were entered without time passing, at `hop_tick`
    /// (`rules.max_transitions`).
    pub hops: u32,
    /// The tick `hops` counts at.
    pub hop_tick: Tick,
}

impl_state_hash!(RunningAction {
    actor,
    def,
    phase,
    locals,
    held,
    wake,
    tick,
    every,
    entered,
    due,
    hops,
    hop_tick
});

/// An entity that is occupied doing nothing (03 §13 `wait`, the `Wait`
/// command), or that nobody has told what to do and idles until it is
/// asked again (03 §5.2).
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub struct Waiting {
    /// The wake-up that ends it.
    pub wake: Option<WakeId>,
    /// Whether it is idling because nothing decided for it, rather than
    /// waiting because it was told to.
    pub idle: bool,
}

impl_state_hash!(Waiting { wake, idle });

/// Every running action and every wait (03 §6). Kept by the simulation
/// as [`Sim::actions`](crate::Sim); changed only by the runtime.
#[derive(Clone, PartialEq, Eq, Default, Debug, Serialize, Deserialize)]
pub struct ActionTable {
    /// The number the next action is given: no two of one simulation
    /// have the same.
    next: u32,
    running: BTreeMap<ActionId, RunningAction>,
    waits: BTreeMap<EntityId, Waiting>,
}

impl_state_hash!(ActionTable {
    next,
    running,
    waits
});

impl ActionTable {
    /// A running action.
    pub fn get(&self, id: ActionId) -> Option<&RunningAction> {
        self.running.get(&id)
    }

    /// Every running action, in the order they were started.
    pub fn iter(&self) -> impl Iterator<Item = (ActionId, &RunningAction)> {
        self.running.iter().map(|(id, action)| (*id, action))
    }

    /// How many actions are running.
    pub fn len(&self) -> usize {
        self.running.len()
    }

    /// Whether none is.
    pub fn is_empty(&self) -> bool {
        self.running.is_empty()
    }

    /// What an entity is waiting for, if it is waiting.
    pub fn wait_of(&self, entity: EntityId) -> Option<&Waiting> {
        self.waits.get(&entity)
    }

    pub(crate) fn get_mut(&mut self, id: ActionId) -> Option<&mut RunningAction> {
        self.running.get_mut(&id)
    }

    /// Enters an action and gives it its number.
    pub(crate) fn insert(&mut self, action: RunningAction) -> Option<ActionId> {
        let id = ActionId::new(self.next);
        self.next = self.next.checked_add(1)?;
        self.running.insert(id, action);
        Some(id)
    }

    pub(crate) fn remove(&mut self, id: ActionId) -> Option<RunningAction> {
        self.running.remove(&id)
    }

    pub(crate) fn set_wait(&mut self, entity: EntityId, wait: Waiting) -> Option<Waiting> {
        self.waits.insert(entity, wait)
    }

    pub(crate) fn take_wait(&mut self, entity: EntityId) -> Option<Waiting> {
        self.waits.remove(&entity)
    }
}

/// An entity's channel slots (03 §6.2): how many of each channel it has
/// is its body's to say ([`Sim::slots_of`](crate::Sim::slots_of)); what
/// is kept here is how many of them rules have disabled. Which actions
/// hold slots is kept by the actions ([`RunningAction::held`]).
///
/// Not to be confused with [`ChannelSet`](crate::action::ChannelSet),
/// which is a set of channel ids: what an action definition uses.
#[derive(Clone, PartialEq, Eq, Default, Debug, Serialize, Deserialize)]
pub struct ChannelSlots {
    /// Slots taken away by `disable_channel`, by channel, in order of
    /// channel. Boxed, and `None` when there are none, which is nearly
    /// always: every entity's kernel record holds one.
    disabled: Option<Box<[(ChannelId, u16)]>>,
}

impl StateHash for ChannelSlots {
    fn state_hash(&self, hasher: &mut StateHasher) {
        self.list().state_hash(hasher);
    }
}

impl ChannelSlots {
    /// The disabled slots, by channel.
    fn list(&self) -> &[(ChannelId, u16)] {
        self.disabled.as_deref().unwrap_or(&[])
    }

    /// How many slots of a channel are disabled.
    pub fn disabled(&self, channel: ChannelId) -> u16 {
        let list = self.list();
        list.binary_search_by_key(&channel, |(have, _)| *have)
            .map_or(0, |at| list[at].1)
    }

    /// Disables `count` more slots of a channel, or enables them again
    /// with a negative count. Never below none.
    pub(crate) fn change(&mut self, channel: ChannelId, by: i32) {
        let mut list: Vec<(ChannelId, u16)> = self.list().to_vec();
        let at = list.binary_search_by_key(&channel, |(have, _)| *have);
        let now = at.map_or(0, |at| i32::from(list[at].1));
        let next = u16::try_from((now + by).max(0)).unwrap_or(u16::MAX);
        match (at, next) {
            (Ok(at), 0) => {
                list.remove(at);
            }
            (Ok(at), next) => list[at].1 = next,
            (Err(_), 0) => {}
            (Err(at), next) => list.insert(at, (channel, next)),
        }
        self.disabled = (!list.is_empty()).then(|| list.into_boxed_slice());
    }
}

/// One planned action (03 §5.3).
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub struct PlanStep {
    /// What to do.
    pub action: ActionDefId,
    /// With what, by name; defaults are worked out when it starts.
    pub params: Params,
}

impl_state_hash!(PlanStep { action, params });

/// What an entity will do next (03 §5.3): actions queued with
/// `Act { how: Enqueue }` or `enqueue_action`, started in order as the
/// channels they need come free.
#[derive(Clone, PartialEq, Eq, Default, Debug, Serialize, Deserialize)]
pub struct Plan {
    /// Next first. A plan is short, and a list keeps the kernel record
    /// small.
    steps: Vec<PlanStep>,
}

impl_state_hash!(Plan { steps });

impl Plan {
    /// The steps, next first.
    pub fn steps(&self) -> impl Iterator<Item = &PlanStep> {
        self.steps.iter()
    }

    /// How many steps there are.
    pub fn len(&self) -> usize {
        self.steps.len()
    }

    /// Whether there are none.
    pub fn is_empty(&self) -> bool {
        self.steps.is_empty()
    }

    pub(crate) fn push(&mut self, step: PlanStep) {
        self.steps.push(step);
    }

    pub(crate) fn front(&self) -> Option<&PlanStep> {
        self.steps.first()
    }

    pub(crate) fn pop(&mut self) -> Option<PlanStep> {
        (!self.steps.is_empty()).then(|| self.steps.remove(0))
    }

    pub(crate) fn clear(&mut self) -> usize {
        let count = self.steps.len();
        self.steps.clear();
        count
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn disabled_slots_count_up_and_down_and_never_below_none() {
        let mut slots = ChannelSlots::default();
        let (arm, leg) = (ChannelId::new(1), ChannelId::new(0));
        slots.change(arm, 2);
        slots.change(leg, 1);
        assert_eq!((slots.disabled(arm), slots.disabled(leg)), (2, 1));
        slots.change(arm, -5);
        assert_eq!(slots.disabled(arm), 0);
        assert_eq!(slots, {
            let mut only_leg = ChannelSlots::default();
            only_leg.change(leg, 1);
            only_leg
        });
    }

    #[test]
    fn actions_are_numbered_once() {
        let mut table = ActionTable::default();
        let action = RunningAction {
            actor: EntityId::new(1, 0),
            def: ActionDefId::new(0),
            phase: PhaseIndex::new(0),
            locals: Vec::new(),
            held: Vec::new(),
            wake: None,
            tick: None,
            every: None,
            entered: Tick::ZERO,
            due: None,
            hops: 0,
            hop_tick: Tick::ZERO,
        };
        let first = table.insert(action.clone()).unwrap();
        table.remove(first);
        let second = table.insert(action).unwrap();
        assert_ne!(first, second);
        assert_eq!(table.len(), 1);
    }
}
