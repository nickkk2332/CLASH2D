//! Postures (06 §11.1): the one an entity is in, and changing it.

use clash_core::{EntityId, Real};
use clash_data::Length;
use clash_world::Cause;

use crate::Sim;
use crate::action::ChannelUse;
use crate::bus::EventPayload;
use crate::component::Physical;
use crate::event::{EventBody, Subject, Where};
use crate::reason::Reason;

use super::defs::{PostureDef, PostureId};
use super::mover::Mover;
use super::rules::posture_def;
use super::{POSTURE_CHANGED, POSTURE_CHANGING, codes};

/// The posture an entity is in. `None` for one that is in none, which
/// changes nothing about it.
pub fn posture_of(sim: &Sim, entity: EntityId) -> Option<PostureId> {
    sim.components().get::<Mover>(entity)?.posture()
}

/// The channel slots an entity's posture makes unavailable (03 §6.2):
/// what its definition `occupies`. `None` for an entity in no posture.
/// Whoever claims slots for an action takes these off first.
pub fn posture_occupies(sim: &Sim, entity: EntityId) -> Option<&ChannelUse> {
    posture_def(sim, entity).map(PostureDef::occupies)
}

/// The factor an entity's posture puts on how easily it is perceived
/// (15 §5.3): its definition's `signature`, or 1 in no posture.
pub fn signature_factor(sim: &Sim, entity: EntityId) -> Real {
    posture_def(sim, entity).map_or(Real::ONE, |posture| posture.signature)
}

/// The qualified id of a posture, or the empty text for none.
fn posture_name(sim: &Sim, posture: Option<PostureId>) -> String {
    posture
        .zip(sim.defs().try_registry::<PostureDef>())
        .map_or_else(String::new, |(id, registry)| registry.name(id).to_owned())
}

/// Makes a posture's share of an entity's standing height the height
/// of its `Physical`, which sight and cover read (06 §3.1).
///
/// This is the seam for a body: once the simulation holds bodies, an
/// entity that has one is given the factor through
/// `body::set_height_factor`, which scales every part's band with it
/// and hands the height on itself.
fn apply_height(sim: &mut Sim, entity: EntityId, factor: Real, cause: Cause) -> Result<(), Reason> {
    let applied = sim
        .components()
        .get::<Mover>(entity)
        .map_or(Real::ONE, |mover| mover.height_factor);
    if applied == factor {
        return Ok(());
    }
    if let Some(physical) = sim.components().get::<Physical>(entity) {
        let mut shape = physical.shape();
        shape.height = Length::new(shape.height.get() / applied * factor);
        sim.set_shape(entity, shape, cause)?;
    }
    sim.components_mut()
        .store_mut::<Mover>()
        .get_or_insert_with(entity, Mover::default)
        .height_factor = factor;
    Ok(())
}

/// Puts an entity in a posture, or with `None` in none (06 §11.1).
/// Returns the posture it was in.
///
/// The hook `posture_changing` runs first and may cancel. Then the
/// entity's height becomes the posture's share of its standing height,
/// its ways of moving are those the posture allows at the posture's
/// speed, and `posture_changed` is raised. A posture it is already in
/// changes nothing and raises nothing.
///
/// Refused, changing nothing, for an unknown entity, a posture no
/// loaded definition has, and with the reason a handler of the hook
/// cancelled for.
pub fn set_posture(
    sim: &mut Sim,
    entity: EntityId,
    posture: Option<PostureId>,
    cause: Cause,
) -> Result<Option<PostureId>, Reason> {
    sim.known(entity)?;
    let factor = match posture {
        None => Real::ONE,
        Some(id) => sim
            .defs()
            .try_registry::<PostureDef>()
            .and_then(|registry| registry.try_get(id))
            .map(|def| def.height)
            .ok_or_else(|| {
                Reason::new(codes::POSTURE_UNKNOWN, format!("there is no posture {id}"))
            })?,
    };
    let old = posture_of(sim, entity);
    if old == posture {
        return Ok(old);
    }
    let (from, to) = (posture_name(sim, old), posture_name(sim, posture));
    let asked = EventPayload::new(POSTURE_CHANGING)
        .subject("self", Subject::Entity(entity))
        .with("from", from.as_str())
        .with("to", to.as_str());
    if let Some(reason) = sim.raise(Where::Entity(entity), cause, asked)?.cancelled {
        return Err(reason);
    }
    apply_height(sim, entity, factor, cause)?;
    sim.components_mut()
        .store_mut::<Mover>()
        .get_or_insert_with(entity, Mover::default)
        .posture = posture;
    // The traits a posture carries are its holder's while it lasts.
    crate::traits::posture_changed(sim, entity, old, posture, cause);
    let look = posture_def(sim, entity)
        .and_then(|def| def.look.clone())
        .unwrap_or_default();
    sim.emit(
        Where::Entity(entity),
        cause,
        EventBody::Custom(
            EventPayload::new(POSTURE_CHANGED)
                .subject("self", Subject::Entity(entity))
                .with("from", from.as_str())
                .with("to", to.as_str())
                .with("look", look.as_str()),
        ),
    );
    Ok(old)
}
