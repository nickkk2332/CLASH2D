//! Modes, settings and stores of a device (16 §4.1, §4.2; package G2):
//! what a device keeps that its element's sixteen bits cannot.
//!
//! A device starts in the first of its definition's `modes`, with every
//! setting at its default and every store at its `start`. Changing any
//! of them is a reason for its element to be promoted, and keeps the
//! device from being demoted until it is back as it was made. Each
//! change tells the recipes under way at the device (`crate::recipe`),
//! so that a station whose speed or stores changed works at its new
//! rate from that moment.
//!
//! Stores stand in for networks (package G3): a station that needs fuel
//! or power takes it from its own store, which content fills with
//! `add_resource`. A store with a `rate` gives no more than that a
//! second, and a recipe that wants more works at the share it gets.

use clash_core::{EntityId, Real};
use clash_expr::Value;
use clash_world::Cause;

use super::{Device, DeviceDef, codes, def_of, promote, raise, settle};
use crate::Sim;
use crate::bus::{EventPayload, FieldValue};
use crate::event::Subject;
use crate::interaction::Target;
use crate::reason::Reason;
use crate::recipe::KnowHowId;

/// A device's mode changed.
pub const DEVICE_MODE_CHANGED: &str = "device_mode_changed";
/// A setting of a device was set.
pub const DEVICE_CONFIGURED: &str = "device_configured";

fn not_a_device() -> Reason {
    Reason::new(codes::NOT_A_DEVICE, "that is not a device").with("entity", "")
}

/// The device a target is or has: a device entity, or the device of an
/// element, promoted if it has not been. Refused for anything else.
pub fn device_for(sim: &mut Sim, target: Target, cause: Cause) -> Result<EntityId, Reason> {
    match target {
        Target::Entity(id) if sim.components.get::<Device>(id).is_some() => Ok(id),
        Target::Entity(id) => Err(not_a_device().with("entity", id.to_string())),
        Target::Element(loc, layer) => promote(sim, loc, layer, cause),
    }
}

/// The device a target is or has, without promoting: a device entity,
/// or the device an element has been promoted to.
pub fn device_now(sim: &Sim, target: Target) -> Option<EntityId> {
    match target {
        Target::Entity(id) => sim.components.get::<Device>(id).map(|_| id),
        Target::Element(loc, layer) => super::device_at(sim, loc, layer),
    }
}

/// The definition behind a target: a device entity's, or the one its
/// element names, promoted or not.
pub fn def_for(sim: &Sim, target: Target) -> Option<&DeviceDef> {
    match target {
        Target::Entity(id) => def_of(sim, id),
        Target::Element(loc, layer) => {
            let state = sim
                .cosmos
                .grid(loc.grid)
                .ok()?
                .element(loc.cell, layer)
                .ok()?;
            if state.is_none() {
                return None;
            }
            let id = super::element_device(sim, layer, state).ok().flatten()?;
            sim.defs.try_registry::<DeviceDef>()?.try_get(id)
        }
    }
}

// ---- Modes ---------------------------------------------------------------------

/// The name of a device's mode, if its definition has modes.
pub fn mode_name(sim: &Sim, device: EntityId) -> Option<String> {
    let index = sim.components.get::<Device>(device)?.mode_index();
    def_of(sim, device)?.modes.get(index).cloned()
}

/// The mode a target is in: its device's, or, for an element that has
/// not been promoted, the first of its device's `modes`.
pub fn mode_of(sim: &Sim, target: Target) -> Option<String> {
    match device_now(sim, target) {
        Some(device) => mode_name(sim, device),
        None => def_for(sim, target)?.modes.first().cloned(),
    }
}

/// Puts a device in a mode (16 §4.2), raising `device_mode_changed`
/// with the old and the new. Returns whether it changed. Refused with
/// `device.unknown_mode` for a mode its definition does not name.
pub fn set_mode(sim: &mut Sim, device: EntityId, mode: &str, cause: Cause) -> Result<bool, Reason> {
    let def = def_of(sim, device).ok_or_else(not_a_device)?;
    let index = def.mode_index(mode).ok_or_else(|| {
        Reason::new(codes::UNKNOWN_MODE, format!("that has no mode `{mode}`")).with("mode", mode)
    })?;
    let old = mode_name(sim, device).unwrap_or_default();
    let Some(component) = sim.components.get_mut::<Device>(device) else {
        return Err(not_a_device());
    };
    let index = u16::try_from(index).unwrap_or(u16::MAX);
    if component.mode == index {
        return Ok(false);
    }
    component.mode = index;
    raise(
        sim,
        device,
        EventPayload::new(DEVICE_MODE_CHANGED)
            .subject("self", Subject::Entity(device))
            .with("old", FieldValue::text(&old))
            .with("new", FieldValue::text(mode)),
        cause,
    );
    crate::recipe::station_changed(sim, device, cause);
    settle(sim, device, cause);
    Ok(true)
}

// ---- Settings ------------------------------------------------------------------

/// What a setting of a device holds: what its user set, else its
/// definition's default. `None` for a setting it does not have.
pub fn config_value(sim: &Sim, device: EntityId, key: &str) -> Option<clash_core::Value> {
    let def = def_of(sim, device)?;
    let entry = def.config_entry(key)?;
    let set = sim
        .components
        .get::<Device>(device)
        .and_then(|component| component.config.get(key).cloned());
    Some(set.unwrap_or_else(|| entry.default_value()))
}

/// Every setting of a device, as a map an expression reads:
/// `self.config.enabled`.
pub fn config_map(sim: &Sim, device: EntityId) -> Value {
    let Some(def) = def_of(sim, device) else {
        return Value::Null;
    };
    let entries: Vec<(String, Value)> = def
        .config
        .iter()
        .filter_map(|entry| {
            let value = config_value(sim, device, &entry.id)?;
            Some((entry.id.clone(), Value::from(&value)))
        })
        .collect();
    Value::map(entries)
}

/// Sets a setting of a device (16 §4.2), raising `device_configured`
/// with its id and the old and new values. Refused with
/// `device.unknown_config` for a setting it does not have and
/// `device.bad_config_value` for a value of the wrong kind, a number
/// outside its bounds or a choice it does not offer.
pub fn set_config(
    sim: &mut Sim,
    device: EntityId,
    key: &str,
    value: &Value,
    cause: Cause,
) -> Result<(), Reason> {
    let def = def_of(sim, device).ok_or_else(not_a_device)?;
    let entry = def.config_entry(key).ok_or_else(|| {
        Reason::new(
            codes::UNKNOWN_CONFIG,
            format!("that has no setting `{key}`"),
        )
        .with("key", key)
    })?;
    let bad = || {
        Reason::new(
            codes::BAD_CONFIG_VALUE,
            format!("`{value}` is not a value the setting `{key}` takes"),
        )
        .with("key", key)
    };
    let stored = match (entry.kind, value) {
        (super::defs::ConfigKind::Flag, Value::Bool(on)) => clash_core::Value::Bool(*on),
        (super::defs::ConfigKind::Number, value) => {
            let number = value
                .as_number()
                .filter(|number| number.is_finite())
                .ok_or_else(bad)?;
            if entry.min.is_some_and(|min| number < min)
                || entry.max.is_some_and(|max| number > max)
            {
                return Err(bad());
            }
            clash_core::Value::Real(number)
        }
        (super::defs::ConfigKind::Choice, Value::Str(text))
            if entry.options.iter().any(|option| **option == **text) =>
        {
            clash_core::Value::Str(text.to_string())
        }
        (super::defs::ConfigKind::Text, Value::Str(text)) => {
            clash_core::Value::Str(text.to_string())
        }
        _ => return Err(bad()),
    };
    let default = entry.default_value();
    let old = config_value(sim, device, key).unwrap_or(clash_core::Value::Null);
    let Some(component) = sim.components.get_mut::<Device>(device) else {
        return Err(not_a_device());
    };
    if stored == default {
        component.config.remove(key);
    } else {
        component.config.set(key, stored.clone());
    }
    raise(
        sim,
        device,
        EventPayload::new(DEVICE_CONFIGURED)
            .subject("self", Subject::Entity(device))
            .with("key", FieldValue::text(key))
            .with("old", FieldValue::Plain(old))
            .with("new", FieldValue::Plain(stored)),
        cause,
    );
    crate::recipe::station_changed(sim, device, cause);
    settle(sim, device, cause);
    Ok(())
}

// ---- Stores --------------------------------------------------------------------

/// The level a store was last brought to: what was recorded, else its
/// definition's `start`. A recipe under way draws on it from then on
/// ([`store_level`] counts that).
pub fn stored(sim: &Sim, device: EntityId, resource: &str) -> Option<Real> {
    let def = def_of(sim, device)?.stores.get(resource)?;
    let recorded = sim
        .components
        .get::<Device>(device)?
        .stores
        .iter()
        .find(|(name, _)| name == resource)
        .map(|(_, level)| *level);
    Some(recorded.unwrap_or(def.start))
}

/// What a store holds now: its recorded level less what a recipe under
/// way has drawn since. `None` for a store the device does not have.
pub fn store_level(sim: &Sim, device: EntityId, resource: &str) -> Option<Real> {
    let level = stored(sim, device, resource)?;
    let drawn = crate::recipe::drawn_since(sim, device, resource);
    Some((level - drawn).max(Real::ZERO))
}

/// What a store of a target holds now: its device's, or for an element
/// not promoted, its definition's `start`.
pub fn store_of(sim: &Sim, target: Target, resource: &str) -> Option<Real> {
    match device_now(sim, target) {
        Some(device) => store_level(sim, device, resource),
        None => def_for(sim, target)?
            .stores
            .get(resource)
            .map(|store| store.start),
    }
}

/// Records a store's level, held within its capacity. The one write of
/// a store's level, for this module and for recipes.
pub(crate) fn record_store(sim: &mut Sim, device: EntityId, resource: &str, level: Real) {
    let Some(def) = def_of(sim, device).and_then(|def| def.stores.get(resource)) else {
        return;
    };
    let (start, capacity) = (def.start, def.capacity);
    let level = level.max(Real::ZERO).min(capacity);
    let Some(component) = sim.components.get_mut::<Device>(device) else {
        return;
    };
    component.stores.retain(|(name, _)| name != resource);
    if level != start {
        component.stores.push((resource.to_owned(), level));
        component.stores.sort_by(|a, b| a.0.cmp(&b.0));
    }
}

/// Adds to a store of a device, negative to draw from it, within its
/// bounds (16 §5.2). Returns how much moved: what was added, or as a
/// negative number what was taken. Refused with `device.no_store` for a
/// store the device does not have and `device.bad_amount` for an amount
/// that is not finite.
pub fn add_resource(
    sim: &mut Sim,
    device: EntityId,
    resource: &str,
    amount: Real,
    cause: Cause,
) -> Result<Real, Reason> {
    if !amount.is_finite() {
        return Err(Reason::new(
            codes::BAD_AMOUNT,
            format!("{amount} is not an amount a store can take"),
        )
        .with("amount", amount));
    }
    // A recipe under way has drawn on it until now: that is recorded
    // first, and its rate worked out again after.
    crate::recipe::bring_up_to_date(sim, device, cause);
    let before = stored(sim, device, resource).ok_or_else(|| {
        Reason::new(codes::NO_STORE, format!("that stores no `{resource}`"))
            .with("resource", resource)
    })?;
    record_store(sim, device, resource, before + amount);
    let after = stored(sim, device, resource).unwrap_or(before);
    crate::recipe::station_changed(sim, device, cause);
    settle(sim, device, cause);
    Ok(after - before)
}

// ---- What it does with recipes ---------------------------------------------------

/// How fast a device works recipes now (16 §8.2): its `speed` in a mode
/// its `when` names, else nothing; 1 for one with no `speed`.
pub fn speed_now(sim: &Sim, device: EntityId) -> Real {
    let Some(def) = def_of(sim, device) else {
        return Real::ZERO;
    };
    let mode = mode_name(sim, device);
    match &def.speed {
        Some(speed) if speed.when.holds(mode.as_deref()) => speed.value,
        Some(_) => Real::ZERO,
        None => Real::ONE,
    }
}

/// How fast a target works recipes now: its device's speed, or for an
/// element not promoted, that of its device's first mode.
pub fn speed_of(sim: &Sim, target: Target) -> Real {
    match device_now(sim, target) {
        Some(device) => speed_now(sim, device),
        None => def_for(sim, target).map_or(Real::ZERO, |def| {
            let mode = def.modes.first().map(String::as_str);
            match &def.speed {
                Some(speed) if speed.when.holds(mode) => speed.value,
                Some(_) => Real::ZERO,
                None => Real::ONE,
            }
        }),
    }
}

/// Whether a device runs recipes by itself now (16 §8.2 `auto`).
pub fn auto_now(sim: &Sim, device: EntityId) -> bool {
    let Some(def) = def_of(sim, device) else {
        return false;
    };
    let mode = mode_name(sim, device);
    def.auto
        .as_ref()
        .is_some_and(|auto| auto.on && auto.when.holds(mode.as_deref()))
}

/// The know-how a target's definition provides in its mode now
/// (16 §8.5 `provides`): what is built in, not what is loaded into it.
pub fn provided_by_def(sim: &Sim, target: Target) -> Vec<KnowHowId> {
    let Some(def) = def_for(sim, target) else {
        return Vec::new();
    };
    let mode = mode_of(sim, target);
    def.provides
        .iter()
        .filter(|provides| provides.when.holds(mode.as_deref()))
        .map(|provides| provides.know_how.id())
        .collect()
}
