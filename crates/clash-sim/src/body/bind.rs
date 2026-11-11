//! Bodies in the running simulation: the functions of
//! [`body`](mod@crate::body) called through [`Sim`], with their events
//! raised and what follows from them carried out (06 §6, §7).
//!
//! | The simulation | Does |
//! |----------------|------|
//! | [`Sim::attach_body`] | Gives an entity a body of a plan at a scale. Its mass and size reach its `Physical`, its presence and the world's occupant record; its stats are worked out again |
//! | [`Sim::hit`] | Raises the hook `hit_incoming`, applies the hit with the stream [`HIT_STREAM`](crate::life::HIT_STREAM), raises what it did, reports what came off and carries on what exits |
//! | [`Sim::close_wound`], [`Sim::reopen_wound`], [`Sim::heal_part`] | Treat wounds |
//! | [`Sim::disable_part`], [`Sim::restore_part`], [`Sim::remove_part`] | Edit the body (06 §6.5) |
//! | [`Sim::set_height_factor`] | What a posture makes of the body's height (06 §11.1) |
//!
//! # What follows from a change
//!
//! Every function of `body` returns what changed. From each change the
//! wiring raises the event of 06 §7.6 that names it, in order, and:
//!
//! | Change | Also |
//! |--------|------|
//! | `MassChanged` | The new mass is written to the kernel's record of the entity and to the world's occupant record. The stats of whoever's mass changed are worked out again |
//! | `SlotsLost`, `SlotsGained` | `slots_lost` and `slots_gained` are raised with the part, the channels and how many slots of each: the action runtime interrupts whatever held one (03 §6.2) |
//! | `FunctionChanged` | Once the events are raised, the entity's stats are worked out again (06 §6.3) |
//! | `BodyDestroyed` | `body_destroyed` is raised |
//! | `Stats` | The stats' own event |
//!
//! and then the entity's definitions say whether it still goes on
//! ([`Sim::settle_vital`](crate::Sim::settle_vital)).
//!
//! # What a hit leaves behind
//!
//! A part that came off is reported as `part_detached`, an event that
//! carries the [`Detached`] itself: the parts as they were, their
//! wounds and their mass. Whoever makes items makes one of it, or
//! nothing where its definition `leaves` nothing (06 §7.5).
//!
//! Energy that left the far side is reported as `hit_exited`. Where it
//! has a direction and the body stands in the world, the block of the
//! next cell along that direction is struck with it by the world's own
//! rule (02 §12.2) first; what stands behind in that cell is combat's
//! to resolve (07 §6.3).
//!
//! # Seams
//!
//! Worn layers are handed to [`Sim::hit_with`] by whoever holds the
//! items, and what each took comes back in [`HitOutcome::report`];
//! [`Sim::hit`] is `hit_with` over [`Sim::worn_layers`], which answers
//! with none until the item package fills it in.

use clash_core::{EntityId, Real, Value};
use clash_data::Def;
use clash_world::{Cause, CellPos, Layer, Location};

use crate::Sim;
use crate::action::def::ChannelDef;
use crate::body::props::{self, part_handle, wound_handle};
use crate::body::{
    self, BodyCx, BodyError, BodyPlanId, Change, Changes, Detach, Detached, Exit, Hit, HitAt,
    HitReport, HitSettings, Leaves, OVERKILL_SETTING, PartKey, SlotCount, WornLayer, WoundId,
};
use crate::bus::{EventPayload, FieldValue};
use crate::event::{EventBody, Subject, Where};
use crate::life::events::{about, dir_field, field_dir, field_handle, handle_field, id_field};
use crate::life::{NullHost, body_reason};
use crate::reason::Reason;

/// What came of [`Sim::hit`].
#[derive(Clone, PartialEq, Debug, Default)]
pub struct HitOutcome {
    /// For a hit a handler of `hit_incoming` cancelled, why. Nothing
    /// was changed and the report is empty.
    pub cancelled: Option<Reason>,
    /// What the hit did: what each part and each worn layer took, the
    /// energy accounted for, what exits and what came off. Its
    /// `changes` are empty: their events have been raised.
    pub report: HitReport,
}

impl HitOutcome {
    /// The joules that left the far side of the body, every exit
    /// summed.
    pub fn exited(&self) -> Real {
        let mut total = Real::ZERO;
        for exit in &self.report.exits {
            total += exit.packet.energy;
        }
        total
    }
}

/// The cell one step along a direction from a cell: each axis steps
/// whose share of the direction is at least half the greatest. `None`
/// for no direction at all.
fn next_along(loc: Location, dir: [Real; 3]) -> Option<Location> {
    let most = dir[0].abs().max(dir[1].abs()).max(dir[2].abs());
    if !most.is_finite() || most <= Real::ZERO {
        return None;
    }
    let step = |along: Real| {
        if along.abs() * Real::of(2.0) < most {
            0
        } else if along > Real::ZERO {
            1
        } else {
            -1
        }
    };
    Some(Location::new(
        loc.grid,
        CellPos::new(
            loc.cell.x + step(dir[0]),
            loc.cell.y + step(dir[1]),
            loc.cell.z + step(dir[2]),
        ),
    ))
}

/// The payload of `slots_lost` and `slots_gained`.
fn slots_payload(
    name: &str,
    entity: EntityId,
    part: FieldValue,
    slots: &[SlotCount],
    channel_type: Option<clash_expr::DefTypeId>,
) -> EventPayload {
    let channels = slots
        .iter()
        .map(|slot| match id_field(channel_type, slot.channel.index()) {
            FieldValue::Plain(value) => value,
            _ => Value::Null,
        })
        .collect();
    let counts = slots
        .iter()
        .map(|slot| Value::Int(i64::from(slot.count)))
        .collect();
    about(name, entity)
        .with("part", part)
        .with("channels", FieldValue::Plain(Value::List(channels)))
        .with("counts", FieldValue::Plain(Value::List(counts)))
}

impl Sim {
    /// Runs a function of the body model and raises what it changed.
    fn change_body(
        &mut self,
        entity: EntityId,
        cause: Cause,
        change: impl FnOnce(&mut BodyCx<'_, NullHost>) -> Result<Changes, BodyError>,
    ) -> Result<(), Reason> {
        self.known(entity)?;
        let changes = self
            .with_body(|cx, _| change(cx))?
            .map_err(|error| body_reason(&error))?;
        self.raise_body_changes(entity, changes, cause);
        Ok(())
    }

    /// Gives an entity a body built from a plan, at a scale (06 §6.2,
    /// §6.4). Its mass, volume, height and cross-section are handed to
    /// its `Physical`, with the plan's footprint if it had none; its
    /// presence and the world's occupant record follow; and its stats,
    /// if it has any, are worked out again, since they may read what the
    /// body can do and what it weighs.
    ///
    /// Its wounds will close and heal on the clock it is on. An entity
    /// that is not in the world is given its body standing still.
    ///
    /// Refused, changing nothing, for an unknown entity, one that has a
    /// body, a plan a body cannot be built from, a scale that is not
    /// above zero, and a plan whose footprint does not fit where the
    /// entity stands.
    pub fn attach_body(
        &mut self,
        entity: EntityId,
        plan: BodyPlanId,
        scale: Real,
        cause: Cause,
    ) -> Result<(), Reason> {
        let record = self.known(entity)?;
        let (place, mut presence) = (record.place(), *record.presence());
        let defs = self.life.body_defs()?;
        if self.components.body.contains(entity) {
            return Err(body_reason(&BodyError::HasBody { entity }));
        }
        let plan_def = defs
            .plan(plan)
            .ok_or_else(|| body_reason(&BodyError::UnknownPlan))?;
        // The footprint is the plan's unless the entity has a size
        // already.
        let footprint = self.components.physical.get(entity).map_or_else(
            || plan_def.footprint(),
            |physical| physical.shape().footprint,
        );
        if footprint != presence.footprint {
            self.check_place(Some(entity), footprint, place)?;
        }
        let clock = self.life_clock(entity);
        let changes = self
            .with_body(|cx, _| {
                let on = clock.unwrap_or(clash_time::ClockId::UNIVERSAL);
                let mut changes = body::attach(cx, entity, plan, scale, on)?;
                if clock.is_none() {
                    changes.extend(body::suspend(cx, entity)?);
                }
                Ok(changes)
            })?
            .map_err(|error: BodyError| body_reason(&error))?;
        presence.footprint = footprint;
        if let Err(reason) = self.set_presence(entity, presence, cause) {
            self.fault(reason);
        }
        self.raise_body_changes(entity, changes, cause);
        // Whether or not a mass was reported: the stats may read what
        // the body can do.
        if self.components.stats.contains(entity)
            && let Err(reason) = self.refresh_stats(entity, cause)
        {
            self.fault(reason);
        }
        self.settle_vital(entity, cause);
        Ok(())
    }

    /// The layers worn over an entity's parts, outermost first for each
    /// part, for a hit to go through (06 §7.2, §9.2).
    ///
    /// A seam for the item package: nothing is worn until it answers.
    pub fn worn_layers(&self, _entity: EntityId) -> Vec<WornLayer> {
        Vec::new()
    }

    /// Applies a hit to a body (06 §7): [`hit_with`](Sim::hit_with)
    /// over whatever the target wears.
    pub fn hit(&mut self, hit: Hit) -> Result<HitOutcome, Reason> {
        let worn = self.worn_layers(hit.target);
        self.hit_with(hit, &worn)
    }

    /// Applies a hit to a body through the layers worn over it (06 §7).
    ///
    /// The hook `hit_incoming` runs first, with the hit's fields: a
    /// handler may change its energy, its intensity, the part it lands
    /// on and its direction, or cancel it, and the outcome then says
    /// why. The hit is resolved with the stream
    /// [`HIT_STREAM`](crate::life::HIT_STREAM). What it did is raised as
    /// events in order; parts that came off are reported as
    /// `part_detached`; energy that left the far side is reported as
    /// `hit_exited` and, where it has a direction, carried on to the
    /// block of the next cell along it.
    ///
    /// Refused, changing nothing, if the target has no body, if the hit
    /// names a part the body does not have or has lost, or if one of
    /// its numbers is negative or not finite.
    pub fn hit_with(&mut self, mut hit: Hit, worn: &[WornLayer]) -> Result<HitOutcome, Reason> {
        let target = hit.target;
        let cause = hit.packet.cause;
        self.known(target)?;
        let defs = self.life.body_defs()?;
        let kinds = defs.kinds();
        if !self.components.body.contains(target) {
            return Err(body_reason(&BodyError::NoBody { entity: target }));
        }

        // The hook, with the hit's fields as its handlers may change
        // them.
        let payload = about(props::HIT_INCOMING, target)
            .with(
                "part",
                match hit.at {
                    HitAt::Part(part) => handle_field(part_handle(kinds, target, part)),
                    _ => FieldValue::Null,
                },
            )
            .with(
                "height",
                match hit.at {
                    HitAt::Height(height) => FieldValue::from(height),
                    _ => FieldValue::Null,
                },
            )
            .with("spread", hit.at == HitAt::Spread)
            .with(
                "kind",
                id_field(defs.def_types().damage_type, hit.packet.kind.index()),
            )
            .with("energy", hit.packet.energy)
            .with("intensity", hit.packet.intensity)
            .with("area", hit.area)
            .with("span", hit.span)
            .with("dir", dir_field(hit.dir));
        let raised = self.raise(Where::Entity(target), cause, payload)?;
        if let Some(reason) = raised.cancelled {
            return Ok(HitOutcome {
                cancelled: Some(reason),
                report: HitReport::default(),
            });
        }
        if let Some(energy) = raised.field("energy").and_then(FieldValue::as_real) {
            hit.packet.energy = energy;
        }
        if let Some(intensity) = raised.field("intensity").and_then(FieldValue::as_real) {
            hit.packet.intensity = intensity;
        }
        let aimed = field_handle(raised.field("part"))
            .filter(|handle| handle.kind == kinds.part)
            .and_then(|handle| props::part_of_handle(&self.components.body, handle))
            .filter(|(owner, _)| *owner == target)
            .map(|(_, part)| part);
        match (aimed, hit.at) {
            (Some(part), _) => hit.at = HitAt::Part(part),
            // A handler took the part away: the hit lands where it may.
            (None, HitAt::Part(_)) => hit.at = HitAt::Any,
            (None, _) => {}
        }
        hit.dir = field_dir(raised.field("dir"));

        let settings = HitSettings {
            overkill_factor: self
                .settings
                .get(OVERKILL_SETTING)
                .and_then(|value| match value {
                    Value::Real(real) => Some(*real),
                    Value::Int(int) => Some(Real::from_i64(*int)),
                    _ => None,
                })
                .unwrap_or(body::DEFAULT_OVERKILL_FACTOR),
        };
        let mut report = self
            .with_body(|cx, rng| body::apply_hit(cx, &hit, worn, rng, &settings))?
            .map_err(|error| body_reason(&error))?;
        self.raise_body_changes(target, core::mem::take(&mut report.changes), cause);
        for detached in &report.detached {
            self.report_detached(detached.clone());
        }
        for exit in &report.exits {
            self.carry_exit(target, exit);
        }
        Ok(HitOutcome {
            cancelled: None,
            report,
        })
    }

    /// Reports a part that came off a body as `part_detached`, with the
    /// part itself as data for whoever makes an item of it (06 §7.5).
    pub(crate) fn report_detached(&mut self, detached: Detached) {
        let entity = detached.from;
        let leaves_item = self
            .life
            .body_defs()
            .ok()
            .and_then(|defs| {
                let root = detached.parts.first()?;
                Some(defs.part(root.def())?.leaves == Leaves::Item)
            })
            .unwrap_or(false);
        let cause = detached.cause;
        let loc = self.entities.location(entity);
        self.emit(
            Where::Entity(entity),
            cause,
            EventBody::PartDetached {
                entity,
                loc,
                leaves_item,
                detached: Box::new(detached),
            },
        );
    }

    /// Reports energy that left the far side of a body, and carries it
    /// on to the block of the next cell along its direction, if it has
    /// one and the body stands in the world.
    fn carry_exit(&mut self, entity: EntityId, exit: &Exit) {
        if exit.packet.energy <= Real::ZERO {
            return;
        }
        let Ok(defs) = self.life.body_defs() else {
            return;
        };
        let loc = self.entities.location(entity);
        if let (Some(loc), Some(dir)) = (loc, exit.dir)
            && let Some(next) = next_along(loc, dir)
        {
            let packet = exit.packet;
            // A cell that is not loaded, or outside the realm, takes
            // nothing: the energy is spent on the air.
            let _ = self.world(|ctx| {
                ctx.edit(packet.cause)
                    .damage_along(next, Layer::Block, packet, Some(dir))
            });
        }
        let mut payload = about("hit_exited", entity)
            .with(
                "part",
                exit.part.map_or(FieldValue::Null, |part| {
                    handle_field(part_handle(defs.kinds(), entity, part))
                }),
            )
            .with(
                "kind",
                id_field(defs.def_types().damage_type, exit.packet.kind.index()),
            )
            .with("energy", exit.packet.energy)
            .with("intensity", exit.packet.intensity)
            .with("dir", dir_field(exit.dir));
        if let Some(loc) = loc {
            payload = payload.subject("here", Subject::Loc(loc));
        }
        if let Err(reason) = self.raise(Where::Entity(entity), exit.packet.cause, payload) {
            self.fault(reason);
        }
    }

    /// The verb `close_wound` (06 §7.3): closes an open wound.
    pub fn close_wound(
        &mut self,
        entity: EntityId,
        wound: WoundId,
        cause: Cause,
    ) -> Result<(), Reason> {
        self.change_body(entity, cause, |cx| body::close_wound(cx, entity, wound))
    }

    /// The verb `reopen_wound` (06 §7.3): opens a closed wound again.
    pub fn reopen_wound(
        &mut self,
        entity: EntityId,
        wound: WoundId,
        cause: Cause,
    ) -> Result<(), Reason> {
        self.change_body(entity, cause, |cx| body::reopen_wound(cx, entity, wound))
    }

    /// The verb `heal_part` (06 §6.5): takes a share of a whole layer
    /// off the damage of every layer of a part.
    pub fn heal_part(
        &mut self,
        entity: EntityId,
        part: PartKey,
        share: Real,
        cause: Cause,
    ) -> Result<(), Reason> {
        self.change_body(entity, cause, |cx| body::heal_part(cx, entity, part, share))
    }

    /// The verb `disable_part` (06 §6.5): disables a part from outside,
    /// whatever its condition.
    pub fn disable_part(
        &mut self,
        entity: EntityId,
        part: PartKey,
        cause: Cause,
    ) -> Result<(), Reason> {
        self.change_body(entity, cause, |cx| body::disable(cx, entity, part))
    }

    /// The verb `restore_part` (06 §6.5): takes back what `disable_part`
    /// did.
    pub fn restore_part(
        &mut self,
        entity: EntityId,
        part: PartKey,
        cause: Cause,
    ) -> Result<(), Reason> {
        self.change_body(entity, cause, |cx| body::restore(cx, entity, part))
    }

    /// The verb `remove_part` (06 §6.5): takes a part off a body with
    /// everything on it and in it. With [`Detach::Removed`] the body no
    /// longer has such parts at all; with [`Detach::Severed`] or
    /// [`Detach::Destroyed`] they stay listed as gone. What came off is
    /// reported as `part_detached` and returned.
    ///
    /// Refused for a part that is not there, and for the root.
    pub fn remove_part(
        &mut self,
        entity: EntityId,
        part: PartKey,
        how: Detach,
        cause: Cause,
    ) -> Result<Detached, Reason> {
        self.known(entity)?;
        let (detached, changes) = self
            .with_body(|cx, _| body::detach(cx, entity, part, how, cause))?
            .map_err(|error| body_reason(&error))?;
        self.raise_body_changes(entity, changes, cause);
        self.report_detached(detached.clone());
        Ok(detached)
    }

    /// Says what a posture makes of a body's height (06 §11.1).
    pub fn set_height_factor(
        &mut self,
        entity: EntityId,
        factor: Real,
        cause: Cause,
    ) -> Result<(), Reason> {
        self.change_body(entity, cause, |cx| {
            body::set_height_factor(cx, entity, factor)
        })
    }

    /// Raises the events of what a function of the body model changed,
    /// in order (06 §7.6), and carries out what follows: see the
    /// [module documentation](self).
    pub(crate) fn raise_body_changes(&mut self, entity: EntityId, changes: Changes, cause: Cause) {
        if changes.is_empty() {
            return;
        }
        let Ok(defs) = self.life.body_defs() else {
            return;
        };
        let (kinds, types) = (defs.kinds(), defs.def_types());
        let channel_type = self.exprs.registry().def_type_id(ChannelDef::TYPE);
        let part = |owner: EntityId, part: PartKey| handle_field(part_handle(kinds, owner, part));
        let wound =
            |owner: EntityId, wound: WoundId| handle_field(wound_handle(kinds, owner, wound));
        let kind =
            |kind: clash_world::defs::DamageTypeId| id_field(types.damage_type, kind.index());
        let mut refresh = false;
        let mut wounded = false;
        for change in changes {
            let payload = match change {
                Change::HitLanded {
                    entity,
                    part: struck,
                    kind: of,
                    absorbed,
                    passed,
                    depth,
                } => about(props::HIT_LANDED, entity)
                    .with("part", part(entity, struck))
                    .with("kind", kind(of))
                    .with("absorbed", absorbed)
                    .with("passed", passed)
                    .with("depth", i64::from(depth)),
                Change::Wounded {
                    entity,
                    wound: id,
                    part: on,
                    kind: of,
                    depth,
                    area,
                } => {
                    wounded = true;
                    about(props::WOUND_INFLICTED, entity)
                        .with("wound", wound(entity, id))
                        .with("part", part(entity, on))
                        .with("kind", kind(of))
                        .with("depth", i64::from(depth))
                        .with("area", area)
                }
                Change::WoundClosed {
                    entity,
                    wound: id,
                    part: on,
                } => about(props::WOUND_CLOSED, entity)
                    .with("wound", wound(entity, id))
                    .with("part", part(entity, on)),
                Change::WoundReopened {
                    entity,
                    wound: id,
                    part: on,
                } => about(props::WOUND_REOPENED, entity)
                    .with("wound", wound(entity, id))
                    .with("part", part(entity, on)),
                Change::WoundHealed {
                    entity,
                    part: on,
                    kind: of,
                    ..
                } => about(props::WOUND_HEALED, entity)
                    .with("part", part(entity, on))
                    .with("kind", kind(of)),
                Change::PartDisabled { entity, part: key } => {
                    about(props::PART_DISABLED, entity).with("part", part(entity, key))
                }
                Change::PartRestored { entity, part: key } => {
                    about(props::PART_RESTORED, entity).with("part", part(entity, key))
                }
                Change::PartSevered {
                    entity,
                    part: key,
                    parent,
                } => about(props::PART_SEVERED, entity)
                    .with("part", part(entity, key))
                    .with(
                        "parent",
                        parent.map_or(FieldValue::Null, |parent| part(entity, parent)),
                    ),
                Change::PartDestroyed { entity, part: key } => {
                    about(props::PART_DESTROYED, entity).with("part", part(entity, key))
                }
                Change::SlotsLost {
                    entity,
                    part: key,
                    slots,
                    ..
                } => slots_payload(
                    "slots_lost",
                    entity,
                    part(entity, key),
                    &slots,
                    channel_type,
                ),
                Change::SlotsGained {
                    entity,
                    part: key,
                    slots,
                    ..
                } => slots_payload(
                    "slots_gained",
                    entity,
                    part(entity, key),
                    &slots,
                    channel_type,
                ),
                Change::FunctionChanged {
                    entity,
                    function,
                    old,
                    new,
                } => {
                    refresh = true;
                    about(props::FUNCTION_CHANGED, entity)
                        .with(
                            "function",
                            id_field(Some(types.body_function), function.index()),
                        )
                        .with("old", old)
                        .with("new", new)
                }
                Change::MassChanged(change) => {
                    self.write_masses(core::slice::from_ref(&change), None, cause);
                    if change.entity == entity {
                        refresh = true;
                    } else {
                        self.life_mass_changed(change.entity, cause);
                    }
                    continue;
                }
                Change::BodyDestroyed { entity } => about("body_destroyed", entity),
                Change::Stats(change) => {
                    self.raise_stat_changes(entity, vec![change], cause);
                    continue;
                }
            };
            if let Err(reason) = self.raise(Where::Entity(entity), cause, payload) {
                self.fault(reason);
            }
        }
        if wounded {
            // A wounded anonymous individual is notable (06 §12.2).
            self.make_notable(entity, cause);
        }
        if refresh
            && self.components.stats.contains(entity)
            && let Err(reason) = self.refresh_stats(entity, cause)
        {
            self.fault(reason);
        }
        self.settle_vital(entity, cause);
    }
}
