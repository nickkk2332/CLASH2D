//! Suppression (08 §6.2): an ability kept from conferring by something
//! outside it.
//!
//! Three things suppress, each by the tags of the abilities it names:
//!
//! | Suppressor | Lasts | Made by |
//! |------------|-------|---------|
//! | [`Suppressor::Effect`] | Until its time runs out or `restore_abilities` lifts it | `suppress_abilities` ([`suppress_abilities`]) |
//! | [`Suppressor::Trait`] | While the owner has a trait whose definition lists `suppresses`, and it is not dormant | Giving the trait |
//! | [`Suppressor::Area`] | While the owner is inside an area whose kind lists `suppresses` | The owner, or the area, moving |
//!
//! Before a suppression takes hold of an ability the hook
//! `ability_suppressing` runs; a handler that cancels makes the ability
//! resist it for as long as it lasts. A suppressed ability behaves as a
//! dormant one: it confers nothing (invariant A2), its modes end (`how`
//! `suppressed`), what was bound to end on suppression ends, and its
//! grants cannot be used. When the last suppression lifts it confers
//! again, unchanged; its modes stay off.
//!
//! What suppresses each instance is kept in it, so nothing is derived:
//! a save carries it.

use clash_core::{Dur, EntityId, Real};
use clash_time::WakeId;
use clash_world::{Cause, CauseKind};

use super::ops::{inst, notify};
use super::{
    ABILITY_RESTORED, ABILITY_SUPPRESSED, ABILITY_SUPPRESSING, Abilities, Imposed, Suppressor,
    codes, def_of, name_of, timer_wake,
};
use crate::Sim;
use crate::bus::{EventPayload, FieldValue};
use crate::event::{Subject, Where};
use crate::reason::Reason;
use crate::traits::{TraitDef, Traits};

/// A suppression that applies to an entity now: what imposes it and
/// the tags it names (`None`: every ability).
type Wanted = (Suppressor, Option<Vec<String>>);

/// Whether a suppression naming `tags` reaches an ability with
/// `ability_tags`.
fn reaches(tags: Option<&[String]>, ability_tags: &[String]) -> bool {
    match tags {
        None => true,
        Some(tags) => tags.iter().any(|tag| ability_tags.contains(tag)),
    }
}

/// Every suppression that applies to an entity now, in a fixed order:
/// those imposed, then its traits', then its areas'.
fn wanted(sim: &Sim, entity: EntityId) -> Vec<Wanted> {
    let mut out: Vec<Wanted> = Vec::new();
    if let Some(abilities) = sim.components.get::<Abilities>(entity) {
        for imposed in &abilities.imposed {
            out.push((Suppressor::Effect(imposed.id), imposed.tags.clone()));
        }
    }
    if let (Some(traits), Some(registry)) = (
        sim.components.get::<Traits>(entity),
        sim.defs.try_registry::<TraitDef>(),
    ) {
        for trait_inst in traits.instances() {
            if trait_inst.is_dormant() {
                continue;
            }
            if let Some(def) = registry.try_get(trait_inst.def())
                && !def.suppresses.is_empty()
            {
                out.push((
                    Suppressor::Trait(trait_inst.key()),
                    Some(def.suppresses.clone()),
                ));
            }
        }
    }
    if let Some(record) = sim.cosmos.occupant(entity) {
        let kinds = sim.cosmos.defs().area_kinds();
        for id in &record.areas {
            let suppresses = sim
                .cosmos
                .area(*id)
                .and_then(|area| area.kind)
                .and_then(|kind| kinds.get(kind))
                .map(|kind| kind.suppresses.as_slice())
                .unwrap_or_default();
            if !suppresses.is_empty() {
                out.push((Suppressor::Area(*id), Some(suppresses.to_vec())));
            }
        }
    }
    out
}

/// Whether an area's kind suppresses anything.
pub(super) fn area_suppresses(sim: &Sim, area: clash_world::AreaId) -> bool {
    let kinds = sim.cosmos.defs().area_kinds();
    sim.cosmos
        .area(area)
        .and_then(|area| area.kind)
        .and_then(|kind| kinds.get(kind))
        .is_some_and(|kind| !kind.suppresses.is_empty())
}

/// Whether any instance of an entity is suppressed by, or resisted, a
/// suppressor.
pub(super) fn touched_by(sim: &Sim, entity: EntityId, by: Suppressor) -> bool {
    sim.components
        .get::<Abilities>(entity)
        .is_some_and(|abilities| {
            abilities
                .instances()
                .iter()
                .any(|inst| inst.suppressed_by.contains(&by) || inst.resisted.contains(&by))
        })
}

/// What a hook says of a suppressor: its word and its source.
fn source_of(sim: &Sim, entity: EntityId, by: Suppressor) -> FieldValue {
    match by {
        Suppressor::Effect(_) => FieldValue::Null,
        Suppressor::Area(area) => FieldValue::Subject(Subject::Area(area)),
        Suppressor::Trait(key) => {
            let name = sim
                .components
                .get::<Traits>(entity)
                .and_then(|traits| traits.get(key))
                .and_then(|inst| {
                    sim.defs
                        .try_registry::<TraitDef>()
                        .filter(|registry| registry.try_get(inst.def()).is_some())
                        .map(|registry| registry.name(inst.def()).to_owned())
                })
                .unwrap_or_default();
            FieldValue::text(name)
        }
    }
}

/// Runs `ability_suppressing` for one instance and one suppressor, if
/// anything listens. Whether a handler cancelled: the ability resists.
fn resists(sim: &mut Sim, entity: EntityId, number: u64, by: Suppressor, cause: Cause) -> bool {
    if !sim.bus.has_listeners(ABILITY_SUPPRESSING) {
        return false;
    }
    let Some(def) = inst(sim, entity, number).map(super::AbilityInst::def) else {
        return false;
    };
    let name = name_of(&sim.defs, def);
    let payload = EventPayload::new(ABILITY_SUPPRESSING)
        .subject("self", Subject::Entity(entity))
        .with("ability", name.as_str())
        .with("by", by.name())
        .with("source", source_of(sim, entity, by));
    match sim.raise(Where::Entity(entity), cause, payload) {
        Ok(raised) => raised.cancelled.is_some(),
        Err(reason) => {
            sim.fault(reason);
            false
        }
    }
}

/// Brings what suppresses one instance into line with what applies to
/// its owner: suppressions that no longer apply are forgotten, and each
/// new one takes hold unless a handler of `ability_suppressing` resists
/// it. Returns whether it was suppressed before and is now, or `None`
/// if there is no such instance.
fn refresh(
    sim: &mut Sim,
    entity: EntityId,
    number: u64,
    wanted: &[Wanted],
    cause: Cause,
) -> Option<(bool, bool)> {
    let current = inst(sim, entity, number)?.clone();
    if current.is_revoked() {
        return None;
    }
    let ability_tags = def_of(&sim.defs, current.def())
        .map(|def| def.tags.clone())
        .unwrap_or_default();
    let applies: Vec<Suppressor> = wanted
        .iter()
        .filter(|(_, tags)| reaches(tags.as_deref(), &ability_tags))
        .map(|(by, _)| *by)
        .collect();
    let was = current.is_suppressed();
    if let Some(inst) = super::ops::inst_mut(sim, entity, number) {
        inst.suppressed_by.retain(|by| applies.contains(by));
        inst.resisted.retain(|by| applies.contains(by));
    }
    for by in applies {
        let known = inst(sim, entity, number)
            .is_some_and(|inst| inst.suppressed_by.contains(&by) || inst.resisted.contains(&by));
        if known {
            continue;
        }
        let resisted = resists(sim, entity, number, by, cause);
        // A handler may have taken the ability away.
        let inst = super::ops::inst_mut(sim, entity, number)?;
        if resisted {
            inst.resisted.push(by);
        } else {
            inst.suppressed_by.push(by);
        }
    }
    let now = inst(sim, entity, number).is_some_and(super::AbilityInst::is_suppressed);
    Some((was, now))
}

/// Carries out an instance becoming suppressed or no longer suppressed:
/// its modes end and what it conferred is taken back, or it confers
/// again; what is bound to end on suppression ends; the event is
/// raised.
fn changed(sim: &mut Sim, entity: EntityId, number: u64, suppressed: bool, cause: Cause) {
    let Some(current) = inst(sim, entity, number).cloned() else {
        return;
    };
    let name = name_of(&sim.defs, current.def());
    if suppressed {
        super::modes::end_all(sim, entity, number, "suppressed", cause);
        super::bind::ending_on(
            sim,
            number,
            super::bind::ends_on::SUPPRESSED,
            "suppressed",
            cause,
        );
        super::ops::settle(sim, entity, number, cause);
        let by = current
            .suppressed_by
            .first()
            .map_or("effect", |by| by.name());
        notify(
            sim,
            entity,
            EventPayload::new(ABILITY_SUPPRESSED)
                .with("ability", name.as_str())
                .with("by", by),
            cause,
        );
    } else {
        super::ops::settle(sim, entity, number, cause);
        notify(
            sim,
            entity,
            EventPayload::new(ABILITY_RESTORED).with("ability", name.as_str()),
            cause,
        );
    }
}

/// Brings what suppresses every ability of an entity into line with the
/// suppressions that apply to it now, and carries out what changed.
/// What the kernel's events call: a trait given or taken, an area
/// entered or left, a suppression imposed or lifted.
pub(crate) fn sync(sim: &mut Sim, entity: EntityId, cause: Cause) {
    let numbers: Vec<u64> = match sim.components.get::<Abilities>(entity) {
        Some(abilities) => abilities
            .instances()
            .iter()
            .filter(|inst| inst.is_held())
            .map(super::AbilityInst::number)
            .collect(),
        None => return,
    };
    let wanted = wanted(sim, entity);
    for number in numbers {
        if let Some((was, now)) = refresh(sim, entity, number, &wanted, cause)
            && was != now
        {
            changed(sim, entity, number, now, cause);
        }
    }
}

/// Works out what suppresses an instance that has just come to be held,
/// before it confers anything: what `grant` calls. Returns whether it
/// is suppressed; the caller raises `ability_suppressed` after
/// `ability_granted`.
pub(super) fn on_gained(sim: &mut Sim, entity: EntityId, number: u64, cause: Cause) -> bool {
    let wanted = wanted(sim, entity);
    refresh(sim, entity, number, &wanted, cause).is_some_and(|(_, now)| now)
}

/// Raises `ability_suppressed` for an instance just gained suppressed.
pub(super) fn announce(sim: &mut Sim, entity: EntityId, number: u64, cause: Cause) {
    let Some(current) = inst(sim, entity, number).cloned() else {
        return;
    };
    if !current.is_suppressed() {
        return;
    }
    let name = name_of(&sim.defs, current.def());
    let by = current
        .suppressed_by
        .first()
        .map_or("effect", |by| by.name());
    notify(
        sim,
        entity,
        EventPayload::new(ABILITY_SUPPRESSED)
            .with("ability", name.as_str())
            .with("by", by),
        cause,
    );
}

/// The reason a suppressed ability cannot be used.
pub(super) fn suppressed(name: &str, by: Suppressor) -> Reason {
    Reason::new(
        codes::SUPPRESSED,
        format!("the ability `{name}` is suppressed"),
    )
    .with("ability", name)
    .with("by", by.name())
}

pub(super) fn bad_duration(seconds: Real) -> Reason {
    Reason::new(
        codes::BAD_DURATION,
        format!("a duration is a time above zero, not {seconds} s"),
    )
    .with("seconds", seconds)
}

/// Suppresses an entity's abilities with any of `tags`, or every one
/// with none (08 §6.2), for `seconds` of its own time or until lifted:
/// `suppress_abilities`. Abilities it gains while the suppression lasts
/// are suppressed too. Each ability is asked through
/// `ability_suppressing` first, and may resist. Returns how many of its
/// abilities are suppressed by it now.
///
/// Refused, changing nothing, for an unknown entity and a time that is
/// not above zero.
pub fn suppress_abilities(
    sim: &mut Sim,
    entity: EntityId,
    tags: Option<Vec<String>>,
    seconds: Option<Real>,
    cause: Cause,
) -> Result<u32, Reason> {
    sim.known(entity)?;
    let until = match seconds {
        None => None,
        Some(seconds) if seconds.is_finite() && seconds > Real::ZERO => {
            Some(sim.own_time(entity) + Dur::from_secs_real(seconds))
        }
        Some(seconds) => return Err(bad_duration(seconds)),
    };
    let abilities = sim
        .components
        .store_mut::<Abilities>()
        .get_or_insert_with(entity, Abilities::default);
    let id = abilities.next_imposed;
    abilities.next_imposed += 1;
    abilities.imposed.push(Imposed {
        id,
        tags,
        until,
        wake: None,
    });
    schedule(sim, entity, id);
    sync(sim, entity, cause);
    let by = Suppressor::Effect(id);
    let count = sim
        .components
        .get::<Abilities>(entity)
        .map_or(0, |abilities| {
            abilities
                .instances()
                .iter()
                .filter(|inst| inst.suppressed_by.contains(&by))
                .count()
        });
    Ok(u32::try_from(count).unwrap_or(u32::MAX))
}

/// Lifts the suppressions `suppress_abilities` imposed on an entity
/// that name any of `tags`, or every one with none: `restore_abilities`
/// (08 §12). What its traits and the areas it is in suppress lasts as
/// long as they do. Returns how many suppressions were lifted.
pub fn restore_abilities(
    sim: &mut Sim,
    entity: EntityId,
    tags: Option<&[String]>,
    cause: Cause,
) -> Result<u32, Reason> {
    sim.known(entity)?;
    let Some(abilities) = sim.components.get_mut::<Abilities>(entity) else {
        return Ok(0);
    };
    let mut lifted: Vec<Option<WakeId>> = Vec::new();
    abilities.imposed.retain(|imposed| {
        let lifts = match (tags, &imposed.tags) {
            (None, _) | (Some(_), None) => true,
            (Some(wanted), Some(have)) => wanted.iter().any(|tag| have.contains(tag)),
        };
        if lifts {
            lifted.push(imposed.wake);
        }
        !lifts
    });
    for wake in lifted.iter().flatten() {
        sim.cancel_wake(*wake);
    }
    if !lifted.is_empty() {
        sync(sim, entity, cause);
    }
    Ok(u32::try_from(lifted.len()).unwrap_or(u32::MAX))
}

/// Schedules the wake-up for the moment an imposed suppression lifts,
/// replacing the one it had. None while the entity is out of the world
/// (its own time stands still) or the suppression lasts until lifted.
fn schedule(sim: &mut Sim, entity: EntityId, id: u64) {
    let Some(imposed) = sim
        .components
        .get::<Abilities>(entity)
        .and_then(|abilities| abilities.imposed.iter().find(|imposed| imposed.id == id))
        .cloned()
    else {
        return;
    };
    if let Some(wake) = imposed.wake {
        sim.cancel_wake(wake);
    }
    let wake = imposed.until.and_then(|until| {
        let after = (until - sim.own_time(entity)).max(Dur::ZERO);
        sim.schedule_for(entity, after, timer_wake(entity)).ok()
    });
    if let Some(imposed) = sim
        .components
        .get_mut::<Abilities>(entity)
        .and_then(|abilities| {
            abilities
                .imposed
                .iter_mut()
                .find(|imposed| imposed.id == id)
        })
    {
        imposed.wake = wake;
    }
}

/// Schedules again the wake-ups of imposed suppressions whose wake-up
/// is gone: what the kernel calls when an entity has moved, so that
/// one that came back into the world counts its time again.
pub(super) fn reschedule(sim: &mut Sim, entity: EntityId) {
    let lost: Vec<u64> = sim
        .components
        .get::<Abilities>(entity)
        .map(|abilities| {
            abilities
                .imposed
                .iter()
                .filter(|imposed| imposed.until.is_some())
                .filter(|imposed| {
                    imposed
                        .wake
                        .is_none_or(|wake| sim.wake_owner(wake) != Some(entity))
                })
                .map(|imposed| imposed.id)
                .collect()
        })
        .unwrap_or_default();
    for id in lost {
        schedule(sim, entity, id);
    }
}

/// A timer of abilities popped for an entity: if it is the wake-up of
/// one of its imposed suppressions, the suppression lifts once its time
/// has run. Returns whether it was one.
pub(super) fn on_wake(sim: &mut Sim, id: WakeId, entity: EntityId) -> bool {
    let Some(imposed) = sim
        .components
        .get::<Abilities>(entity)
        .and_then(|abilities| {
            abilities
                .imposed
                .iter()
                .find(|imposed| imposed.wake == Some(id))
        })
        .cloned()
    else {
        return false;
    };
    let due = imposed
        .until
        .is_none_or(|until| sim.own_time(entity) >= until);
    if !due {
        if let Some(imposed) = sim
            .components
            .get_mut::<Abilities>(entity)
            .and_then(|abilities| {
                abilities
                    .imposed
                    .iter_mut()
                    .find(|have| have.id == imposed.id)
            })
        {
            imposed.wake = None;
        }
        schedule(sim, entity, imposed.id);
        return true;
    }
    if let Some(abilities) = sim.components.get_mut::<Abilities>(entity) {
        abilities.imposed.retain(|have| have.id != imposed.id);
    }
    sync(sim, entity, Cause::new(CauseKind::Process));
    true
}

/// What abilities do about the suppressions of an entity when one of
/// its traits came or went, or it entered or left an area.
pub(super) fn traits_changed(sim: &mut Sim, entity: EntityId, cause: Cause) {
    let concerned = sim
        .components
        .get::<Abilities>(entity)
        .is_some_and(|abilities| !abilities.instances().is_empty());
    if concerned {
        sync(sim, entity, cause);
    }
}
