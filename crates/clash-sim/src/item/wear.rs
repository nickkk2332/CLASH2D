//! Item damage, worn items as layers for hits, and what a body's
//! changes do to what it holds (06 §7.2, §7.5, §9.1, §9.2).
//!
//! # Damage
//!
//! An item takes damage by the element rule (02 §12.2). Its capacity is
//! its material's toughness times the volume of one member, and its
//! damage is counted in 255ths of that. At all 255 it is **broken**: it
//! is still there, it adds no layer when worn, and what else being
//! broken means is a rule's to say.
//!
//! A worn layer is resolved by the body's hit, against the stuff under
//! the contact (06 §7.2). What it absorbed comes back as a
//! [`WornHit`] and is damage to the item: the 255ths of the item's whole
//! capacity that the absorbed energy amounts to, and never less than
//! one. A layer that was breached may reach 255; one that was only
//! damaged stops at 254.
//!
//! # Whoever applies a hit
//!
//! ```ignore
//! let worn = item::worn_layers_of(sim, target);
//! let report = body::apply_hit(&mut cx, &hit, &worn, rng, &settings)?;
//! item::apply_worn_hits(sim, &report.worn, hit.packet.kind, cause);
//! for detached in &report.detached {
//!     item::part_item(sim, detached, cause)?;
//! }
//! item::on_body_changes(sim, &report.changes, cause);
//! ```

use std::sync::Arc;

use clash_core::{EntityId, Real};
use clash_data::{Def, Length, Mass, Volume};
use clash_world::defs::{DamageTypeId, MaterialId, Resist};
use clash_world::{
    Cause, DamageOutcome, DamagePacket, DamageResult, Footprint, OccupantKind, resolve_packet,
};

use super::hold::{self, Hold, Holds, ground_of, notify};
use super::make::{ITEM_STREAM, member_volume};
use super::{
    ITEM_BROKEN, ITEM_DAMAGED, ITEM_REPAIRED, Item, def_of, item_of, latent, reason, refuse, stack,
};
use crate::Sim;
use crate::body::{self, Body, BodyPartDef, Detach, Detached, Leaves, PartKey, WornHit, WornLayer};
use crate::bus::EventPayload;
use crate::component::{DefKey, Identity, Shape};
use crate::entity::EntitySpec;
use crate::event::Subject;
use crate::reason::Reason;

/// The parts an item's damage is counted in, as an element's is
/// (02 §12.2).
pub const DAMAGE_PARTS: u8 = 255;

/// The most damage an item that is not broken carries.
const DAMAGE_MAX: u8 = DAMAGE_PARTS - 1;

/// What a packet did to an item.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct ItemHit {
    /// The item that took it: the one named, or the member of a stack
    /// that was split off to take it.
    pub item: EntityId,
    /// What the packet rule said: unaffected, damaged or destroyed, and
    /// the joules absorbed and passed on.
    pub outcome: DamageOutcome,
    /// The item's damage afterwards, in 255ths.
    pub damage: u8,
}

/// The joules one member of an item absorbs whole, and how its
/// material resists a kind of damage.
fn capacity(sim: &Sim, item: EntityId, material: MaterialId, kind: DamageTypeId) -> (Real, Resist) {
    let (toughness, resist) = sim.cosmos.defs().materials().get(material).map_or(
        (Real::ZERO, Resist::NONE),
        |material| {
            (
                material.toughness,
                material
                    .resist
                    .get(kind.index())
                    .copied()
                    .unwrap_or(Resist::NONE),
            )
        },
    );
    (toughness * member_volume(sim, item), resist)
}

/// Writes an item's damage and says what changed.
fn set_damage(sim: &mut Sim, item: EntityId, new: u8, kind: Option<DamageTypeId>, cause: Cause) {
    let Some(component) = sim.components.get_mut::<Item>(item) else {
        return;
    };
    let old = component.damage;
    if old == new {
        return;
    }
    component.damage = new;
    let mut payload = EventPayload::new(if new > old {
        ITEM_DAMAGED
    } else {
        ITEM_REPAIRED
    })
    .subject("self", Subject::Entity(item))
    .with("old", i64::from(old))
    .with("new", i64::from(new));
    if new > old {
        let name = kind.map_or("", |kind| sim.cosmos.defs().damage_types().name(kind));
        payload = payload.with("kind", name);
    }
    notify(sim, item, payload, cause);
    if new == DAMAGE_PARTS {
        let payload = EventPayload::new(ITEM_BROKEN).subject("self", Subject::Entity(item));
        notify(sim, item, payload, cause);
    }
}

fn bad_amount(amount: Real) -> Reason {
    refuse(
        reason::BAD_AMOUNT,
        format!("{amount} cannot be used here: it must be finite and not negative"),
    )
    .with("amount", amount)
}

/// Applies a damage packet to an item (06 §9.1; 02 §12.2). One member
/// of a stack is split off to take it, in the same place and hold. A
/// packet that cannot affect the item's material changes nothing. A
/// container whose contents have not been made yet is opened by the
/// first packet that affects it (06 §9.3).
///
/// Refused with `item.not_an_item` for anything else, and with
/// `item.bad_amount` for a packet whose energy or intensity is negative
/// or not finite.
pub fn damage(sim: &mut Sim, item: EntityId, packet: &DamagePacket) -> Result<ItemHit, Reason> {
    for number in [packet.energy, packet.intensity] {
        if !(number.is_finite() && number >= Real::ZERO) {
            return Err(bad_amount(number));
        }
    }
    let component = item_of(sim, item)?;
    let (old, material, count) = (component.damage, component.material, component.count);
    let (capacity, resist) = capacity(sim, item, material, packet.kind);
    let (outcome, after) = resolve_packet(resist, capacity, old, packet.energy, packet.intensity);
    if outcome.result == DamageResult::Unaffected {
        return Ok(ItemHit {
            item,
            outcome,
            damage: old,
        });
    }
    let target = if count > 1 {
        stack::split(sim, item, 1, packet.cause)?
    } else {
        item
    };
    if latent::is_latent(sim, target) {
        latent::materialise(sim, target, packet.cause)?;
    }
    let new = match outcome.result {
        DamageResult::Destroyed => DAMAGE_PARTS,
        _ => after.min(DAMAGE_MAX),
    };
    set_damage(sim, target, new, Some(packet.kind), packet.cause);
    Ok(ItemHit {
        item: target,
        outcome,
        damage: new,
    })
}

/// Takes a share of an item's whole capacity off its damage (06 §9.1):
/// `1` mends it entirely. Every member of a stack is mended alike. An
/// item that is equal again to a stack in the same hold rejoins it, and
/// the entity that now holds it is returned.
///
/// Refused with `item.bad_amount` for a share that is not finite and
/// above zero.
pub fn repair(
    sim: &mut Sim,
    item: EntityId,
    share: Real,
    cause: Cause,
) -> Result<EntityId, Reason> {
    if !(share.is_finite() && share > Real::ZERO) {
        return Err(bad_amount(share));
    }
    let old = item_of(sim, item)?.damage;
    let parts = (share * Real::from_i64(i64::from(DAMAGE_PARTS)))
        .ceil()
        .to_i64()
        .clamp(1, i64::from(DAMAGE_PARTS));
    let new = u8::try_from((i64::from(old) - parts).max(0)).unwrap_or(0);
    set_damage(sim, item, new, None, cause);
    Ok(stack::rejoin(sim, item, cause))
}

/// What a worn layer absorbed, as damage to its item (06 §7.2): the
/// `worn` of a `HitReport`, with the kind of damage the hit delivered.
pub fn apply_worn_hits(sim: &mut Sim, hits: &[WornHit], kind: DamageTypeId, cause: Cause) {
    for hit in hits {
        if hit.result == DamageResult::Unaffected {
            continue;
        }
        let Some(component) = sim.components.get::<Item>(hit.item) else {
            continue;
        };
        let (old, material) = (component.damage, component.material);
        let (capacity, _) = capacity(sim, hit.item, material, kind);
        let whole = i64::from(DAMAGE_PARTS);
        let gained = if capacity > Real::ZERO {
            (Real::from_i64(whole) * hit.absorbed / capacity)
                .ceil()
                .to_i64()
                .clamp(1, whole)
        } else {
            whole
        };
        let limit = match hit.result {
            DamageResult::Destroyed => whole,
            _ => i64::from(DAMAGE_MAX),
        };
        let new = (i64::from(old) + gained).min(limit).max(i64::from(old));
        set_damage(
            sim,
            hit.item,
            u8::try_from(new).unwrap_or(DAMAGE_PARTS),
            Some(kind),
            cause,
        );
    }
}

/// Every layer an entity wears, for `body::apply_hit` (06 §7.2): by
/// part, and over each part outermost first. A broken item adds none.
pub fn worn_layers_of(sim: &Sim, entity: EntityId) -> Vec<WornLayer> {
    let Some(holds) = sim.components.get::<Holds>(entity) else {
        return Vec::new();
    };
    let mut layers: Vec<(i16, WornLayer)> = Vec::new();
    for (item_id, hold) in holds.iter() {
        let Hold::Worn(on) = hold else { continue };
        let Some(item) = sim.components.get::<Item>(item_id) else {
            continue;
        };
        if item.is_broken() {
            continue;
        }
        let Some(def) = def_of(&sim.defs, item) else {
            continue;
        };
        for worn in on {
            let Some(entry) = def.wear.0.get(usize::from(worn.entry)) else {
                continue;
            };
            layers.push((
                worn.layer,
                WornLayer {
                    part: worn.part,
                    item: item_id,
                    material: entry
                        .material
                        .map_or(item.material(), |material| material.id()),
                    thickness: entry.thickness.get(),
                    coverage: entry.coverage,
                    damage: item.damage(),
                },
            ));
        }
    }
    layers.sort_by_key(|(layer, worn)| (worn.part, core::cmp::Reverse(*layer), worn.item));
    layers.into_iter().map(|(_, worn)| worn).collect()
}

/// The layers an entity wears over one part, outermost first.
pub fn worn_layers(sim: &Sim, entity: EntityId, part: PartKey) -> Vec<WornLayer> {
    worn_layers_of(sim, entity)
        .into_iter()
        .filter(|layer| layer.part == part)
        .collect()
}

/// What a body's changes do to what it holds (06 §9.2): what a part
/// that no longer supplies its slots was grasping is dropped where the
/// holder is, with `item_dropped` and the cause, and what was worn only
/// on parts that are gone is dropped too.
///
/// Call it with the changes any body function returned. For parts that
/// came off, call [`part_item`] first: what was worn on them goes with
/// them.
pub fn on_body_changes(sim: &mut Sim, changes: &[body::Change], cause: Cause) {
    for change in changes {
        match change {
            body::Change::SlotsLost { entity, part, .. } => {
                hold::let_go(sim, *entity, *part, cause);
            }
            body::Change::PartSevered { entity, .. }
            | body::Change::PartDestroyed { entity, .. }
            | body::Change::BodyDestroyed { entity } => {
                let present: Vec<PartKey> = sim
                    .components
                    .get::<Body>(*entity)
                    .map(|body| {
                        body.parts()
                            .iter()
                            .filter(|part| part.is_present())
                            .map(|part| part.key())
                            .collect()
                    })
                    .unwrap_or_default();
                hold::uncover(sim, *entity, &|part| !present.contains(&part), None, cause);
            }
            _ => {}
        }
    }
}

/// Makes the item a part that came off a body leaves (06 §7.5), where
/// the body is: in its cell, or inside whatever encloses it. `None` if
/// the part's definition says it leaves nothing.
///
/// The item is made from no `item` definition. It is of the material of
/// the part's thickest layer, has the mass the part had and the volume
/// its tissues take up, carries the tags of the part's definition, and
/// keeps the part as it was ([`Item::remains`]). Its `Identity` names
/// the `body_part` definition. A part that was destroyed leaves a
/// broken item.
///
/// What the body wore only on the parts that came off goes with them,
/// inside the item; what it wore on other parts as well stays on those;
/// what it grasped with them is dropped.
pub fn part_item(
    sim: &mut Sim,
    detached: &Detached,
    cause: Cause,
) -> Result<Option<EntityId>, Reason> {
    sim.known(detached.from)?;
    let defs = Arc::clone(&sim.defs);
    let part_defs = defs.try_registry::<BodyPartDef>();
    let (Some(root), Some(part_defs)) = (
        detached
            .parts
            .iter()
            .find(|part| part.key() == detached.root),
        part_defs,
    ) else {
        return Ok(None);
    };
    let Some(root_def) = part_defs.try_get(root.def()) else {
        return Ok(None);
    };
    let keys: Vec<PartKey> = detached.parts.iter().map(|part| part.key()).collect();
    let gone = |part: PartKey| keys.contains(&part);
    // What the parts grasped falls whether or not anything is left of
    // them.
    for key in &keys {
        hold::let_go(sim, detached.from, *key, cause);
    }
    // The thickest layer is what the item is made of.
    let mut material: Option<(MaterialId, Real)> = None;
    for layer in root.layers() {
        if material.is_none_or(|(_, thickness)| layer.thickness() > thickness) {
            material = Some((layer.material(), layer.thickness()));
        }
    }
    let (Leaves::Item, Some((material, _))) = (root_def.leaves, material) else {
        hold::uncover(sim, detached.from, &gone, None, cause);
        return Ok(None);
    };
    // Each part's mass over the mean density of its layers.
    let materials = sim.cosmos.defs().materials();
    let mut volume = Real::ZERO;
    for part in &detached.parts {
        let Some(def) = part_defs.try_get(part.def()) else {
            continue;
        };
        let scale = detached.scale * part.scale();
        let mass = def.mass.get() * scale * scale * scale;
        let mut thickness = Real::ZERO;
        let mut weighed = Real::ZERO;
        for layer in part.layers() {
            let density = materials
                .get(layer.material())
                .map_or(Real::ZERO, |material| material.density);
            thickness += layer.thickness();
            weighed += layer.thickness() * density;
        }
        if weighed > Real::ZERO {
            volume += mass * thickness / weighed;
        }
    }
    let length = root_def.length.get() * detached.scale * root.scale();
    let shape = Shape {
        mass: Mass::new(detached.mass.max(Real::ZERO)),
        volume: Volume::new(volume),
        height: Length::new(Real::ZERO),
        footprint: Footprint::SINGLE,
        cross_section: if length > Real::ZERO {
            volume / length
        } else {
            Real::ZERO
        },
    };
    let mut spec = EntitySpec::new(OccupantKind::Item);
    for tag in root_def.tags.iter() {
        spec.tags.insert(defs.tags().name(tag));
    }
    let place = ground_of(sim, detached.from);
    let id = sim.spawn(spec, place, cause)?;
    let seed = sim.rng.stream(ITEM_STREAM).next_u64();
    sim.components.insert(
        id,
        Item {
            def: None,
            material,
            count: 1,
            damage: if detached.how == Detach::Destroyed {
                DAMAGE_PARTS
            } else {
                0
            },
            color: None,
            seed,
            latent: false,
            remains: Some(Box::new(detached.clone())),
        },
    );
    let key = sim
        .exprs
        .registry()
        .def_type_id(BodyPartDef::TYPE)
        .map(|ty| DefKey::new(ty, u32::from(root.def().raw())));
    sim.components.insert(id, Identity::new(key));
    if let Err(reason) = sim.set_shape(id, shape, cause) {
        let _ = sim.remove_entity(id, cause);
        return Err(reason);
    }
    hold::uncover(sim, detached.from, &gone, Some(id), cause);
    Ok(Some(id))
}
