//! Minds and vital state (`docs/06-entities-and-bodies.md` section 8).
//!
//! A **mind** is what decides for a body. It is a component of its own,
//! [`Mind`], so that the two can part company: it can be given, taken
//! away, and moved to another entity with what is seated in it
//! (06 §8.2). A body with no mind may still be made to act by a
//! **driver**, another entity that acts for it ([`Driver`]).
//!
//! # Stopping and starting
//!
//! A mind drives its body, or it does not: [`VitalState`] is `awake`,
//! `unconscious` or `dead`. The engine has no notion of what kills.
//! Definitions say it:
//!
//! | Definition | Field | Says |
//! |------------|-------|------|
//! | `body_plan` | `needs: { function: state }` | While the body can do none of a function (its capacity is zero), its owner is in that state |
//! | `pool` | `stops: [ { at, when, state } ]` | While the pool's level is on that side of one of its thresholds, its owner is in that state |
//!
//! where a state is `unconscious` or `dead`. Whenever a pool crosses a
//! threshold, a function of the body moves or a body is changed, the
//! wiring asks what the definitions now say ([`Sim::settle_vital`]) and
//! the worst of them holds:
//!
//! - `dead`: [`Sim::die`]. The hook `entity_dying` may cancel. If not,
//!   the mind stops for good, whoever was waiting on a decision of the
//!   entity's is forgotten, control of it is released, and
//!   `entity_died` and then `mind_released` are raised. The entity is
//!   still there, with its body, its wounds and what it holds: it can
//!   be carried, searched, or given a new mind. Only [`Sim::revive`]
//!   starts it again.
//! - `unconscious`: the mind stops until nothing says so any more;
//!   `entity_unconscious` and `entity_woke` are raised.
//!
//! An entity with no [`Mind`] is never stopped: there is nothing to
//! stop. A creature whose template gives it no mind, or whose plan has
//! no `needs` and whose pools have no `stops`, simply goes on.
//!
//! # Who may act
//!
//! [`Sim::can_act`] is what a command is checked against (03 §4.4): an
//! entity acts if its mind is awake, or if it has a driver. The reasons
//! are in [`MIND_REASONS`].
//!
//! # Not built
//!
//! Beliefs, relations and senses (15) are not part of the mind yet, so
//! a transfer moves none. The `mind` definition type is the AI layer's
//! (09): a mind carries the id it was given as text until then. Where
//! abilities come from after a transfer (08 §3.4) waits for abilities.

mod verbs;

use std::sync::Arc;

use clash_core::{EntityId, impl_state_hash};
use clash_data::Defs;
use clash_world::Cause;
use serde::{Deserialize, Serialize};

use crate::Sim;
use crate::body::{BodyDefs, BodyFunctionId, BodyPlanDef};
use crate::entity::Control;
use crate::event::{Subject, Where};
use crate::life::events::about;
use crate::life::stats_reason;
use crate::reason::{Reason, ReasonInfo};
use crate::stats::{self, PoolDef, PoolId, Seat, StatDefs, StopSide};

pub use verbs::verbs;

/// What a definition says a lack does to a creature with a mind
/// (06 §8.3). The later is the worse.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Stop {
    /// Its mind stops driving it until the lack is gone.
    Unconscious,
    /// Its mind stops driving it for good.
    Dead,
}

/// Whether a mind drives its body (06 §8.3).
#[derive(
    Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug, Default, Serialize, Deserialize,
)]
#[serde(rename_all = "snake_case")]
#[repr(u8)]
pub enum VitalState {
    /// It does.
    #[default]
    Awake,
    /// It does not, for now.
    Unconscious,
    /// It does not, until something revives it.
    Dead,
}

impl VitalState {
    /// The word rules read: `awake`, `unconscious`, `dead`.
    pub const fn name(self) -> &'static str {
        match self {
            VitalState::Awake => "awake",
            VitalState::Unconscious => "unconscious",
            VitalState::Dead => "dead",
        }
    }
}

impl clash_core::StateHash for VitalState {
    fn state_hash(&self, hasher: &mut clash_core::StateHasher) {
        hasher.write_u8(*self as u8);
    }
}

/// What decides for a body (06 §8.1): the `Mind` component.
///
/// What is seated in a mind (attributes, pools and tracks whose
/// definitions say `"seat": "mind"`) is kept with the rest of the
/// entity's stats and moved with the mind by
/// [`Sim::transfer_mind`].
#[derive(Clone, PartialEq, Eq, Debug, Default, Serialize, Deserialize)]
pub struct Mind {
    /// The behaviour profile for the AI layer (09): the id of a `mind`
    /// definition, as written.
    profile: Option<String>,
    /// Whether it drives its body.
    state: VitalState,
}

impl_state_hash!(Mind { profile, state });

impl Mind {
    /// A mind that is awake, with a behaviour profile or none.
    pub const fn new(profile: Option<String>) -> Mind {
        Mind {
            profile,
            state: VitalState::Awake,
        }
    }

    /// The behaviour profile it was given, as written.
    pub fn profile(&self) -> Option<&str> {
        self.profile.as_deref()
    }

    /// Whether it drives its body.
    pub const fn state(&self) -> VitalState {
        self.state
    }

    /// Whether it drives its body: `mind.active` of 06 §8.1.
    pub fn is_active(&self) -> bool {
        self.state == VitalState::Awake
    }
}

/// Who acts for a body without being its mind (06 §8.2): the `Driver`
/// component of the body's entity.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub struct Driver {
    /// The entity that acts for this one.
    pub by: EntityId,
}

impl_state_hash!(Driver { by });

/// One entry of a pool's `stops`, resolved.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
struct PoolStop {
    /// The position of the threshold among the pool's, lowest first.
    threshold: usize,
    /// The side of it that stops the owner.
    side: StopSide,
    /// What it does.
    stop: Stop,
}

/// What stops a creature, by its definitions (06 §8.3): the `needs` of
/// each body plan and the `stops` of each pool. Not state: made from
/// the definitions.
#[derive(Clone, Debug, Default)]
pub struct VitalDefs {
    /// By body plan: the functions it cannot do without.
    needs: Vec<Vec<(BodyFunctionId, Stop)>>,
    /// By pool: its stops.
    stops: Vec<Vec<PoolStop>>,
}

impl VitalDefs {
    /// The tables for a set of loaded definitions.
    pub(crate) fn new(
        defs: &Arc<Defs>,
        stats: Option<&StatDefs>,
        body: Option<&BodyDefs>,
    ) -> VitalDefs {
        let needs = match (body, defs.try_registry::<BodyPlanDef>()) {
            (Some(_), Some(plans)) => plans
                .iter()
                .map(|(_, plan)| {
                    plan.needs
                        .iter()
                        .map(|(function, stop)| (function.id(), *stop))
                        .collect()
                })
                .collect(),
            _ => Vec::new(),
        };
        let stops = match (stats, defs.try_registry::<PoolDef>()) {
            (Some(_), Some(pools)) => pools
                .iter()
                .map(|(_, pool)| {
                    pool.stops
                        .iter()
                        .filter_map(|stop| {
                            let threshold = pool.thresholds.iter().position(|at| *at == stop.at)?;
                            Some(PoolStop {
                                threshold,
                                side: stop.when,
                                stop: stop.state,
                            })
                        })
                        .collect()
                })
                .collect(),
            _ => Vec::new(),
        };
        VitalDefs { needs, stops }
    }
}

/// The codes of the reasons this module refuses for (14 §5).
pub mod reason {
    /// The entity has no mind. Values: `entity`.
    pub const NO_MIND: &str = "no_mind";
    /// The entity already has a mind. Values: `entity`.
    pub const HAS_MIND: &str = "has_mind";
    /// The entity is dead: its mind does not drive it, and only
    /// `revive` starts it again. Values: `entity`.
    pub const ENTITY_DEAD: &str = "entity_dead";
    /// The entity is unconscious: its mind does not drive it for now.
    /// Values: `entity`.
    pub const ENTITY_UNCONSCIOUS: &str = "entity_unconscious";
    /// The entity is not dead, so it cannot be revived. Values:
    /// `entity`.
    pub const NOT_DEAD: &str = "not_dead";
    /// An entity cannot drive itself, or move its mind into itself.
    /// Values: `entity`.
    pub const SAME_ENTITY: &str = "same_entity";
}

/// The reasons of minds and vital state, for the generated reference.
pub const MIND_REASONS: &[ReasonInfo] = &[
    ReasonInfo {
        code: reason::NO_MIND,
        doc: "The entity has no mind: there is none to move, remove, stop or start, and nothing to act for it unless it has a driver. Values: `entity`.",
    },
    ReasonInfo {
        code: reason::HAS_MIND,
        doc: "The entity already has a mind: `create_mind` in it, or `transfer_mind` into it without `swap`. Values: `entity`.",
    },
    ReasonInfo {
        code: reason::ENTITY_DEAD,
        doc: "The entity is dead: its mind does not drive it, so it cannot act and cannot die again. Values: `entity`.",
    },
    ReasonInfo {
        code: reason::ENTITY_UNCONSCIOUS,
        doc: "The entity is unconscious: its mind does not drive it for now, so it cannot act. Values: `entity`.",
    },
    ReasonInfo {
        code: reason::NOT_DEAD,
        doc: "`revive` of an entity that is not dead. Values: `entity`.",
    },
    ReasonInfo {
        code: reason::SAME_ENTITY,
        doc: "An entity was named as its own driver, or as where its own mind is to go. Values: `entity`.",
    },
];

fn refusal(code: &'static str, entity: EntityId, message: String) -> Reason {
    Reason::new(code, message).with("entity", entity.to_string())
}

fn no_mind(entity: EntityId) -> Reason {
    refusal(reason::NO_MIND, entity, format!("{entity} has no mind"))
}

impl Sim {
    /// An entity's mind, if it has one. Truth.
    pub fn mind(&self, entity: EntityId) -> Option<&Mind> {
        self.components.mind.get(entity)
    }

    /// Whether an entity's mind drives it: `awake`, `unconscious` or
    /// `dead`. `None` for an entity with no mind.
    pub fn vital(&self, entity: EntityId) -> Option<VitalState> {
        self.mind(entity).map(Mind::state)
    }

    /// Who acts for an entity's body without being its mind, if anyone
    /// does (06 §8.2).
    pub fn driver(&self, entity: EntityId) -> Option<EntityId> {
        self.components.driver.get(entity).map(|driver| driver.by)
    }

    /// Whether an entity can act (06 §8.2, §8.3): its mind is awake, or
    /// it has a driver. What a command for the entity is checked
    /// against.
    ///
    /// Refused with `entity_dead` or `entity_unconscious` for one whose
    /// mind is stopped and that nobody drives, and with `no_mind` for
    /// one that has no mind and no driver.
    pub fn can_act(&self, entity: EntityId) -> Result<(), Reason> {
        self.known(entity)?;
        if self.driver(entity).is_some() {
            return Ok(());
        }
        match self.vital(entity) {
            Some(VitalState::Awake) => Ok(()),
            Some(VitalState::Unconscious) => Err(refusal(
                reason::ENTITY_UNCONSCIOUS,
                entity,
                format!("{entity} is unconscious"),
            )),
            Some(VitalState::Dead) => Err(refusal(
                reason::ENTITY_DEAD,
                entity,
                format!("{entity} is dead"),
            )),
            None => Err(no_mind(entity)),
        }
    }

    /// The verb `create_mind` (06 §8.2): gives an entity a new mind,
    /// awake, with a behaviour profile or none. Refused for one that
    /// has a mind.
    pub fn create_mind(
        &mut self,
        entity: EntityId,
        profile: Option<String>,
        cause: Cause,
    ) -> Result<(), Reason> {
        self.known(entity)?;
        if self.components.mind.contains(entity) {
            return Err(refusal(
                reason::HAS_MIND,
                entity,
                format!("{entity} already has a mind"),
            ));
        }
        self.components.mind.insert(entity, Mind::new(profile));
        self.settle_vital(entity, cause);
        Ok(())
    }

    /// The verb `remove_mind` (06 §8.2): removes an entity's mind and
    /// discards it, with what was seated in it. Control of the entity is
    /// released.
    pub fn remove_mind(&mut self, entity: EntityId, cause: Cause) -> Result<Mind, Reason> {
        self.known(entity)?;
        let mind = self
            .components
            .mind
            .remove(entity)
            .ok_or_else(|| no_mind(entity))?;
        if self.components.stats.contains(entity) {
            let taken = self
                .with_stats(entity, |cx| stats::take_seated(cx, entity, Seat::Mind))?
                .map_err(|error| stats_reason(&error))?;
            self.raise_stat_changes(entity, taken.1, cause);
        }
        self.decisions.forget(entity);
        self.set_control(entity, Control::None, cause)?;
        Ok(mind)
    }

    /// The verb `transfer_mind` (06 §8.2): moves the mind of `from` into
    /// `to`, with the attributes, pools and tracks seated in it, and the
    /// control mode. With `swap`, a mind that `to` has goes the other
    /// way; without, `to` must have none.
    ///
    /// The hook `mind_transferring` runs first and may cancel. Refused,
    /// changing nothing, if `from` has no mind, if `to` has one and
    /// `swap` is not set, or if the two are the same entity.
    pub fn transfer_mind(
        &mut self,
        from: EntityId,
        to: EntityId,
        swap: bool,
        cause: Cause,
    ) -> Result<(), Reason> {
        self.known(from)?;
        self.known(to)?;
        if from == to {
            return Err(refusal(
                reason::SAME_ENTITY,
                from,
                format!("the mind of {from} is already in {from}"),
            ));
        }
        if !self.components.mind.contains(from) {
            return Err(no_mind(from));
        }
        let back = self.components.mind.contains(to);
        if back && !swap {
            return Err(refusal(
                reason::HAS_MIND,
                to,
                format!("{to} already has a mind"),
            ));
        }
        let payload = about("mind_transferring", from)
            .subject("target", Subject::Entity(to))
            .with("swap", swap);
        if let Some(reason) = self.raise(Where::Entity(from), cause, payload)?.cancelled {
            return Err(reason);
        }

        // What is seated in each mind leaves its body first, then
        // arrives in the other.
        let leaving = self.take_seated(from, cause)?;
        let returning = if back {
            self.take_seated(to, cause)?
        } else {
            stats::Seated::default()
        };
        let mind = self.components.mind.remove(from);
        let other = self.components.mind.remove(to);
        if let Some(mind) = mind {
            self.components.mind.insert(to, mind);
        }
        if let Some(other) = other {
            self.components.mind.insert(from, other);
        }
        self.give_seated(to, leaving, cause)?;
        self.give_seated(from, returning, cause)?;
        // What the mind knows how to do goes with it (16 §8.5).
        crate::recipe::know_how::follow_mind(self, from, to);

        // Control goes with the mind.
        let (was_from, was_to) = (self.known(from)?.control(), self.known(to)?.control());
        self.decisions.forget(from);
        self.decisions.forget(to);
        self.set_control(to, was_from, cause)?;
        self.set_control(from, if back { was_to } else { Control::None }, cause)?;

        let payload = about("mind_transferred", from)
            .subject("target", Subject::Entity(to))
            .with("swap", back);
        if let Err(reason) = self.raise(Where::Entity(to), cause, payload) {
            self.fault(reason);
        }
        self.settle_vital(from, cause);
        self.settle_vital(to, cause);
        Ok(())
    }

    /// Takes what is seated in an entity's mind out of its stats.
    fn take_seated(&mut self, entity: EntityId, cause: Cause) -> Result<stats::Seated, Reason> {
        if !self.components.stats.contains(entity) {
            return Ok(stats::Seated::default());
        }
        let (seated, changes) = self
            .with_stats(entity, |cx| stats::take_seated(cx, entity, Seat::Mind))?
            .map_err(|error| stats_reason(&error))?;
        self.raise_stat_changes(entity, changes, cause);
        Ok(seated)
    }

    /// Puts what was seated in a mind into the stats of the entity the
    /// mind is now in, giving it stats if it has none.
    fn give_seated(
        &mut self,
        entity: EntityId,
        seated: stats::Seated,
        cause: Cause,
    ) -> Result<(), Reason> {
        if seated.is_empty() {
            return Ok(());
        }
        if !self.components.stats.contains(entity) {
            self.attach_stats(entity, cause)?;
        }
        let changes = self
            .with_stats(entity, |cx| stats::give_seated(cx, entity, seated))?
            .map_err(|error| stats_reason(&error))?;
        self.raise_stat_changes(entity, changes, cause);
        Ok(())
    }

    /// The verb `set_driver` (06 §8.2): makes one entity act for
    /// another's body without moving any mind, or with `None` lets the
    /// body go. The hook `mind_transferring` runs first, with the body
    /// as `self` and the driver as `target`, and may cancel.
    pub fn set_driver(
        &mut self,
        of: EntityId,
        driver: Option<EntityId>,
        cause: Cause,
    ) -> Result<(), Reason> {
        self.known(of)?;
        if let Some(driver) = driver {
            self.known(driver)?;
            if driver == of {
                return Err(refusal(
                    reason::SAME_ENTITY,
                    of,
                    format!("{of} cannot be its own driver"),
                ));
            }
        }
        if self.driver(of) == driver {
            return Ok(());
        }
        let mut payload = about("mind_transferring", of).with("swap", false);
        if let Some(driver) = driver {
            payload = payload.subject("target", Subject::Entity(driver));
        }
        if let Some(reason) = self.raise(Where::Entity(of), cause, payload)?.cancelled {
            return Err(reason);
        }
        match driver {
            Some(by) => {
                self.components.driver.insert(of, Driver { by });
            }
            None => {
                self.components.driver.remove(of);
            }
        }
        let mut payload = about("driver_changed", of);
        if let Some(driver) = driver {
            payload = payload.subject("driver", Subject::Entity(driver));
        }
        if let Err(reason) = self.raise(Where::Entity(of), cause, payload) {
            self.fault(reason);
        }
        Ok(())
    }

    /// What an entity's definitions say of it now: the worst of what
    /// the `needs` of its body's plan and the `stops` of its pools say
    /// (06 §8.3), or `None` if nothing stops it.
    pub fn stopped_by_definition(&self, entity: EntityId) -> Option<Stop> {
        let vital = self.life.vital();
        let mut worst: Option<Stop> = None;
        if let Some(body) = self.components.body.get(entity)
            && let Some(needs) = vital.needs.get(body.plan().index())
        {
            for (function, stop) in needs {
                if body.function(*function) <= clash_core::Real::ZERO {
                    worst = worst.max(Some(*stop));
                }
            }
        }
        if let Some(stats) = self.components.stats.get(entity) {
            for pool in stats.pools() {
                let Some(stops) = vital.stops.get(PoolId::index(pool)) else {
                    continue;
                };
                let Some(band) = stats.pool_band(pool) else {
                    continue;
                };
                for stop in stops {
                    // The band counts the thresholds the level is at or
                    // above, lowest first.
                    let holds = match stop.side {
                        StopSide::Below => band <= stop.threshold,
                        StopSide::Above => band > stop.threshold,
                    };
                    if holds {
                        worst = worst.max(Some(stop.stop));
                    }
                }
            }
        }
        worst
    }

    /// Brings an entity's vital state into line with what its
    /// definitions say (06 §8.3): it dies, falls unconscious or wakes.
    /// The wiring calls this whenever a pool crossed a threshold or a
    /// body changed. It does nothing for an entity with no mind, and
    /// nothing for one that is dead.
    pub fn settle_vital(&mut self, entity: EntityId, cause: Cause) {
        let Some(state) = self.vital(entity) else {
            return;
        };
        if state == VitalState::Dead {
            return;
        }
        match (state, self.stopped_by_definition(entity)) {
            (_, Some(Stop::Dead)) => {
                // A handler of `entity_dying` may have refused: the
                // entity then goes on as it was.
                let _ = self.die(entity, cause);
            }
            (VitalState::Awake, Some(Stop::Unconscious)) => {
                self.set_vital(entity, VitalState::Unconscious);
                self.decisions.forget(entity);
                self.tell(entity, "entity_unconscious", cause);
            }
            (VitalState::Unconscious, None) => {
                self.set_vital(entity, VitalState::Awake);
                self.tell(entity, "entity_woke", cause);
            }
            _ => {}
        }
    }

    fn set_vital(&mut self, entity: EntityId, state: VitalState) {
        if let Some(mind) = self.components.mind.get_mut(entity) {
            mind.state = state;
        }
    }

    /// Raises a notification about an entity that has no fields.
    fn tell(&mut self, entity: EntityId, event: &str, cause: Cause) {
        if let Err(reason) = self.raise(Where::Entity(entity), cause, about(event, entity)) {
            self.fault(reason);
        }
    }

    /// The verb `die` (06 §8.3): an entity's mind stops driving it for
    /// good. The hook `entity_dying` runs first and may cancel. Then
    /// whoever was waiting on a decision of the entity's is forgotten,
    /// control of it is released, and `entity_died` and `mind_released`
    /// are raised. The entity remains, with its body, its wounds and
    /// what it holds.
    ///
    /// Refused if the entity has no mind, is dead already, or a handler
    /// cancelled; the reason is then the handler's.
    pub fn die(&mut self, entity: EntityId, cause: Cause) -> Result<(), Reason> {
        self.known(entity)?;
        match self.vital(entity) {
            None => return Err(no_mind(entity)),
            Some(VitalState::Dead) => {
                return Err(refusal(
                    reason::ENTITY_DEAD,
                    entity,
                    format!("{entity} is dead already"),
                ));
            }
            Some(_) => {}
        }
        let raised = self.raise(Where::Entity(entity), cause, about("entity_dying", entity))?;
        if let Some(reason) = raised.cancelled {
            return Err(reason);
        }
        self.set_vital(entity, VitalState::Dead);
        self.decisions.forget(entity);
        self.set_control(entity, Control::None, cause)?;
        self.tell(entity, "entity_died", cause);
        self.tell(entity, "mind_released", cause);
        Ok(())
    }

    /// The verb `revive` (06 §8.3): a dead entity's mind drives it
    /// again, and `entity_revived` is raised. What its definitions say
    /// is then asked afresh: one that still lacks what it cannot do
    /// without stops again at once.
    ///
    /// Refused if the entity has no mind or is not dead.
    pub fn revive(&mut self, entity: EntityId, cause: Cause) -> Result<(), Reason> {
        self.known(entity)?;
        match self.vital(entity) {
            None => return Err(no_mind(entity)),
            Some(VitalState::Dead) => {}
            Some(_) => {
                return Err(refusal(
                    reason::NOT_DEAD,
                    entity,
                    format!("{entity} is not dead"),
                ));
            }
        }
        self.set_vital(entity, VitalState::Awake);
        self.tell(entity, "entity_revived", cause);
        self.settle_vital(entity, cause);
        Ok(())
    }
}
