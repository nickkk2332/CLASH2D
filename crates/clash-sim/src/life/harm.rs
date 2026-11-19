//! Harm from the world (06 §7.4; 02 §13.2, §14.3, §15.2, §16): what the
//! world reports having done to an occupant becomes a hit on its body
//! or a dose into one of its pools.
//!
//! The world cannot see bodies (01 R4). It says what struck the cell an
//! occupant is in, and the simulation answers as it passes the world's
//! events on:
//!
//! | The world says | The simulation |
//! |----------------|----------------|
//! | `occupant_exposed`: a mass landed on the cell, or a field's `damage` rule struck it | A hit spread over the body ([`exposure_hit`]): the body takes what that intensity puts into its own volume, and never more than the cell was delivered. It goes through `hit_incoming` and the layers like any other hit ([`Sim::hit`]) |
//! | `occupant_dosed`: a field's `expose` rule | The hook `exposure_incoming` with the pool, the amount and the route, whose handlers may lessen the amount (a mask) or cancel; then the amount is added to the pool, stopping at its bounds. A body with no stats or without that pool takes nothing |
//! | `body_impact`: a body in flight that is an entity struck an element (02 §15.2) | [`Sim::body_impact`]: a hit of all the energy the body lost, spread over it with no direction, or anywhere with the element's contact surface if it has one |
//! | `body_hit_occupant`: a body in flight struck an occupant | The occupant takes the blow's packet spread over its body, travelling the way the body was |
//!
//! The answer comes after the kernel's own event about the entity
//! (`entity_exposed`), so what the hit did follows what caused it in
//! the stream. An occupant with no body is not hurt: the events are
//! all that happens.
//!
//! The pool a dose names is text, as the field's definition writes it:
//! a qualified id, or a bare one looked up in the namespace of the
//! field's own definition and then in every namespace in sorted order,
//! as a tissue's is (decision D-521).

use clash_core::{EntityId, Real};
use clash_world::defs::Contact;
use clash_world::defs::DamageTypeId;
use clash_world::{BodyOwner, Cause, ExposureRoute, FieldId, Location, WorldEvent};

use crate::Sim;
use crate::body::{self, HitOutcome, Impact, exposure_hit, impact_hit};
use crate::bus::FieldValue;
use crate::event::{Subject, Where};
use crate::life::body_reason;
use crate::life::events::{about, id_field};
use crate::reason::Reason;
use crate::stats::{PoolDef, PoolId};

/// Something the world did to an occupant that its body or its pools
/// answer for.
#[derive(Clone, PartialEq, Debug)]
pub(crate) enum Harm {
    /// Something struck the cell the occupant is in.
    Exposed {
        entity: EntityId,
        kind: DamageTypeId,
        energy: Real,
        intensity: Real,
        cause: Cause,
    },
    /// A body in flight that is this entity struck an element.
    Impact {
        entity: EntityId,
        energy: Real,
        intensity: Real,
        contact: Option<Contact>,
        cause: Cause,
    },
    /// A body in flight struck this occupant.
    Struck {
        entity: EntityId,
        kind: DamageTypeId,
        energy: Real,
        intensity: Real,
        dir: [Real; 3],
        cause: Cause,
    },
    /// A field's `expose` rule dosed the occupant.
    Dosed {
        entity: EntityId,
        loc: Location,
        field: FieldId,
        pool: String,
        amount: Real,
        route: ExposureRoute,
        cause: Cause,
    },
}

impl Harm {
    /// What one of the world's events did to an occupant, if anything.
    pub(crate) fn of(event: &WorldEvent) -> Option<Harm> {
        match event {
            WorldEvent::OccupantExposed {
                occupant,
                kind,
                energy,
                intensity,
                cause,
                ..
            } => Some(Harm::Exposed {
                entity: *occupant,
                kind: *kind,
                energy: *energy,
                intensity: *intensity,
                cause: *cause,
            }),
            WorldEvent::OccupantDosed {
                occupant,
                loc,
                field,
                pool,
                amount,
                route,
                cause,
                ..
            } => Some(Harm::Dosed {
                entity: *occupant,
                loc: *loc,
                field: *field,
                pool: pool.clone(),
                amount: *amount,
                route: *route,
                cause: *cause,
            }),
            WorldEvent::BodyImpact {
                owner: BodyOwner::Entity(entity),
                energy,
                intensity,
                contact,
                cause,
                ..
            } => Some(Harm::Impact {
                entity: *entity,
                energy: *energy,
                intensity: *intensity,
                contact: contact.as_deref().cloned(),
                cause: *cause,
            }),
            WorldEvent::BodyHitOccupant {
                occupant,
                packet,
                vel,
                ..
            } => Some(Harm::Struck {
                entity: *occupant,
                kind: packet.kind,
                energy: packet.energy,
                intensity: packet.intensity,
                dir: *vel,
                cause: packet.cause,
            }),
            _ => None,
        }
    }
}

impl Sim {
    /// Carries out what the world did to an occupant. A refusal is
    /// reported as a fault: the world has already done its part.
    pub(crate) fn apply_harm(&mut self, harm: Harm) {
        let done = match harm {
            Harm::Exposed {
                entity,
                kind,
                energy,
                intensity,
                cause,
            } => self
                .expose(entity, kind, energy, intensity, cause)
                .map(|_| ()),
            Harm::Impact {
                entity,
                energy,
                intensity,
                contact,
                cause,
            } => {
                let kind = match &contact {
                    Some(contact) => Some(contact.kind),
                    None => self.cosmos.defs().kinetic_damage_type(),
                };
                match kind {
                    Some(kind) => {
                        let impact = Impact {
                            kind,
                            energy,
                            intensity,
                            dir: None,
                            contact: contact.map(|contact| {
                                (
                                    contact.area,
                                    contact.span.unwrap_or_else(|| contact.area.sqrt()),
                                )
                            }),
                            cause,
                        };
                        self.body_impact(entity, &impact).map(|_| ())
                    }
                    // A world with no kinetic damage type has nothing to
                    // strike with.
                    None => Ok(()),
                }
            }
            Harm::Struck {
                entity,
                kind,
                energy,
                intensity,
                dir,
                cause,
            } => {
                let impact = Impact {
                    kind,
                    energy,
                    intensity,
                    dir: Some(dir),
                    contact: None,
                    cause,
                };
                self.body_impact(entity, &impact).map(|_| ())
            }
            Harm::Dosed {
                entity,
                loc,
                field,
                pool,
                amount,
                route,
                cause,
            } => self
                .dose(entity, loc, field, &pool, amount, route, cause)
                .map(|_| ()),
        };
        if let Err(reason) = done {
            self.fault(reason);
        }
    }

    /// Hurts an entity by what struck the cell it is in (06 §7.4): a
    /// hit spread over its body, of what that intensity puts into the
    /// body's own volume and never more than was delivered. `None` for
    /// an entity that has no body, which is not hurt.
    pub fn expose(
        &mut self,
        entity: EntityId,
        kind: DamageTypeId,
        energy: Real,
        intensity: Real,
        cause: Cause,
    ) -> Result<Option<HitOutcome>, Reason> {
        let Some(found) = self.components.body.get(entity) else {
            return Ok(None);
        };
        let hit = exposure_hit(found, entity, kind, energy, intensity, cause);
        self.hit(hit).map(Some)
    }

    /// Hurts an entity that struck something in flight (02 §15.2;
    /// 06 §7.4): a hit of all the energy it lost, spread over its body,
    /// or anywhere with the surface it landed on if that has one of its
    /// own. `None` for an entity that has no body.
    ///
    /// The simulation calls this for the world's `body_impact` whose
    /// owner is an entity, and for the occupant of a
    /// `body_hit_occupant`.
    pub fn body_impact(
        &mut self,
        entity: EntityId,
        impact: &Impact,
    ) -> Result<Option<HitOutcome>, Reason> {
        if !self.components.body.contains(entity) {
            return Ok(None);
        }
        self.hit(impact_hit(entity, impact)).map(Some)
    }

    /// The pool a field's dose names, as its definition writes it.
    fn dosed_pool(&self, field: FieldId, pool: &str) -> Option<PoolId> {
        let defs = self.life.stat_defs().ok()?;
        let pools = defs.defs().try_registry::<PoolDef>()?;
        let field_name = self.cosmos.defs().fields().name(field);
        let names = pools.names().iter().map(String::as_str);
        let found = body::pool_by_name(names, field_name, pool)?;
        pools.lookup(found)
    }

    /// Doses an entity as a field's `expose` rule says (02 §16): raises
    /// the hook `exposure_incoming`, then adds what its handlers left of
    /// the amount to the pool, stopping at the pool's bounds. Returns
    /// what was added: nothing for an entity with no stats or without
    /// the pool, and for a dose a handler cancelled.
    ///
    /// The world applies no route and no guard, and neither does this:
    /// a handler of the hook does, by the `route` it is told.
    #[allow(clippy::too_many_arguments)]
    pub fn dose(
        &mut self,
        entity: EntityId,
        loc: Location,
        field: FieldId,
        pool: &str,
        amount: Real,
        route: ExposureRoute,
        cause: Cause,
    ) -> Result<Real, Reason> {
        let Some(pool) = self.dosed_pool(field, pool) else {
            return Ok(Real::ZERO);
        };
        let has = self
            .components
            .stats
            .get(entity)
            .is_some_and(|stats| stats.has_pool(pool));
        if !has || !amount.is_finite() {
            return Ok(Real::ZERO);
        }
        let defs = self.life.stat_defs()?;
        let field_name = self.cosmos.defs().fields().name(field).to_owned();
        let payload = about("exposure_incoming", entity)
            .subject("here", Subject::Loc(loc))
            .with("pool", id_field(Some(defs.def_types().pool), pool.index()))
            .with("amount", amount)
            .with("route", route.name())
            .with("field", field_name.as_str());
        let raised = self.raise(Where::Entity(entity), cause, payload)?;
        if raised.cancelled.is_some() {
            return Ok(Real::ZERO);
        }
        let amount = raised
            .field("amount")
            .and_then(FieldValue::as_real)
            .filter(|amount| amount.is_finite())
            .unwrap_or(amount);
        self.dose_pool(entity, pool, amount, cause)
    }
}

// The refusals of a hit are the body model's own.
const _: fn(&body::BodyError) -> Reason = body_reason;
