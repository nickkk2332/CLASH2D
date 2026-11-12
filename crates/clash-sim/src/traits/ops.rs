//! Adding and removing traits, stacking, groups, dormancy and durations
//! (06 §5; 03 §9).

use std::sync::Arc;

use clash_core::{Dur, EntityId, Real};
use clash_data::Defs;
use clash_time::{Wake, WakeDomain, WakeId};
use clash_world::{Cause, CauseKind};

use super::confer::{def_of, settle};
use super::{
    Away, DURATION_FIELD, Giver, TRAIT_ADDED, TRAIT_ADDING, TRAIT_EXPIRED, TRAIT_REMOVED,
    TRAIT_REMOVING, Timer, TraitDef, TraitId, TraitInst, TraitKey, Traits, WhileUnloaded, codes,
};
use crate::Sim;
use crate::bus::{EventPayload, FieldValue};
use crate::event::{EventBody, Subject, Where};
use crate::host::Bound;
use crate::movement::{PostureDef, PostureId};
use crate::reason::{Reason, codes as kernel_codes};
use crate::rule::Effects;

/// The bit of a wake-up's `arg` that marks it as a trait's duration
/// running out. The low 32 bits of `arg` are the instance's
/// [`TraitKey`], and `id` is the holder's
/// [`EntityId::to_bits`](clash_core::EntityId::to_bits). The two bits
/// above it, which pools and bodies use, are clear.
pub const TRAIT_WAKE_TAG: u64 = 1 << 61;

/// The bits of `arg` that tell one timer of the domain from another.
const TAG_BITS: u64 = 0b111 << 61;

/// The bits of `arg` that hold the instance's key.
const KEY_BITS: u64 = u32::MAX as u64;

/// The wake-up that ends an instance's duration.
pub fn trait_wake(entity: EntityId, key: TraitKey) -> Wake {
    Wake::new(
        WakeDomain::Timer,
        entity.to_bits(),
        TRAIT_WAKE_TAG | u64::from(key),
    )
}

/// Whether a popped wake-up is a trait's duration running out.
pub fn is_trait_wake(wake: &Wake) -> bool {
    wake.domain == WakeDomain::Timer && wake.arg & TAG_BITS == TRAIT_WAKE_TAG
}

/// What giving a trait did.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Added {
    /// The instance it went to.
    pub key: TraitKey,
    /// How many stacks it has now.
    pub stacks: u16,
    /// Whether the entity did not have it from this giver before.
    pub first: bool,
    /// Whether anything changed: false for a trait that does not stack,
    /// given again by the same giver.
    pub changed: bool,
}

/// The cause a timer's ending is given.
const TIME: Cause = Cause::new(CauseKind::Process);

fn traits(sim: &Sim, entity: EntityId) -> Option<&Traits> {
    sim.components.get::<Traits>(entity)
}

fn traits_mut(sim: &mut Sim, entity: EntityId) -> Option<&mut Traits> {
    sim.components.get_mut::<Traits>(entity)
}

fn inst_mut(sim: &mut Sim, entity: EntityId, key: TraitKey) -> Option<&mut TraitInst> {
    traits_mut(sim, entity)?
        .insts
        .iter_mut()
        .find(|inst| inst.key == key)
}

/// The qualified id of a trait, or the empty text.
fn name_of(sim: &Sim, def: TraitId) -> String {
    sim.defs
        .try_registry::<TraitDef>()
        .filter(|registry| registry.try_get(def).is_some())
        .map_or_else(String::new, |registry| registry.name(def).to_owned())
}

fn look_of(sim: &Sim, def: TraitId) -> String {
    def_of(sim, def)
        .and_then(|def| def.look.clone())
        .unwrap_or_default()
}

fn not_registered() -> Reason {
    Reason::new(
        codes::NOT_REGISTERED,
        "the definitions were loaded without the `trait` type",
    )
}

/// The refusal for an id that names no trait.
pub(super) fn unknown(what: impl Into<String>) -> Reason {
    let what = what.into();
    Reason::new(codes::UNKNOWN, format!("there is no trait `{what}`")).with("trait", what)
}

fn bad_duration(seconds: Real) -> Reason {
    Reason::new(
        codes::BAD_DURATION,
        format!("a trait cannot last {seconds} seconds: a duration is a finite time above zero"),
    )
    .with("duration", seconds)
}

/// The trait a name written in data stands for: a qualified id exactly;
/// a bare one in the first namespace, in sorted order, that has it.
pub fn lookup(defs: &Defs, name: &str) -> Option<TraitId> {
    let registry = defs.try_registry::<TraitDef>()?;
    if name.contains(':') {
        return registry.lookup(name);
    }
    let mut found: Vec<(&str, TraitId)> = registry
        .iter()
        .map(|(id, _)| (registry.name(id), id))
        .filter(|(qualified, _)| {
            qualified
                .split_once(':')
                .is_some_and(|(_, bare)| bare == name)
        })
        .collect();
    found.sort_unstable();
    found.first().map(|(_, id)| *id)
}

/// A duration as an event's field carries it: seconds, or null.
fn duration_field(duration: Option<Dur>) -> FieldValue {
    duration.map_or(FieldValue::Null, |dur| FieldValue::from(dur.as_secs_real()))
}

/// Whether a trait's `while` holds for its holder. One without holds.
fn holds(sim: &Sim, entity: EntityId, def: &TraitDef) -> bool {
    let Some(condition) = &def.condition else {
        return true;
    };
    let bound = Bound::new().subject(sim.subject_value(Subject::Entity(entity)));
    let mut host = sim.host(bound);
    condition.holds_on(&sim.defs, &mut host)
}

// ---- Durations ---------------------------------------------------------------

/// Starts an instance's duration afresh, or makes it last until removed.
/// A duration that cannot be put on the timeline (its holder is out of
/// the simulation) stands still with all its time left until the
/// holder returns.
fn start_timer(sim: &mut Sim, entity: EntityId, key: TraitKey, duration: Option<Dur>) {
    let Some(inst) = traits(sim, entity).and_then(|traits| traits.get(key)) else {
        return;
    };
    if let Timer::Running(wake) = inst.timer {
        sim.cancel_wake(wake);
    }
    let away = traits(sim, entity).is_some_and(Traits::is_away);
    let timer = match duration {
        None => Timer::Lasts,
        Some(left) if away => Timer::Held(left),
        Some(left) => match sim.schedule_for(entity, left, trait_wake(entity, key)) {
            Ok(wake) => Timer::Running(wake),
            Err(_) => Timer::Held(left),
        },
    };
    if let Some(inst) = inst_mut(sim, entity, key) {
        inst.timer = timer;
    }
}

/// Takes an entity's durations off the timeline, each with the local
/// time it has left, as it leaves the simulation (03 §9): its tract is
/// unloaded, it is stored, or it is put inside something that is not in
/// the world. Where it was is kept, to measure the time away by.
///
/// The kernel calls this before the entity's place changes. Calling it
/// for an entity that is away already, or has no traits, does nothing.
pub fn suspend(sim: &mut Sim, entity: EntityId) {
    let Some(traits) = traits(sim, entity) else {
        return;
    };
    if traits.is_away() {
        return;
    }
    let running: Vec<(TraitKey, WakeId)> = traits
        .insts
        .iter()
        .filter_map(|inst| match inst.timer {
            Timer::Running(wake) => Some((inst.key, wake)),
            Timer::Lasts | Timer::Held(_) => None,
        })
        .collect();
    let clock = sim.clock_of(entity);
    let local = clock
        .and_then(|clock| sim.timeline.local_now(clock).ok())
        .unwrap_or(Dur::ZERO);
    let away = Away {
        clock,
        local,
        tick: sim.now(),
    };
    for (key, wake) in running {
        let left = sim
            .timeline
            .remaining(wake)
            .unwrap_or(Dur::ZERO)
            .max(Dur::ZERO);
        sim.cancel_wake(wake);
        if let Some(inst) = inst_mut(sim, entity, key) {
            inst.timer = Timer::Held(left);
        }
    }
    if let Some(traits) = traits_mut(sim, entity) {
        traits.away = Some(away);
    }
}

/// How much local time passed for an entity while it was away: on the
/// clock it was on when it left, if that clock still exists, and else in
/// universal time.
fn time_away(sim: &Sim, away: Away) -> Dur {
    let local = away
        .clock
        .and_then(|clock| sim.timeline.local_now(clock).ok())
        .map(|now| now - away.local);
    local
        .unwrap_or_else(|| sim.now().since(away.tick))
        .max(Dur::ZERO)
}

/// Puts an entity's durations back on the timeline as it returns to the
/// simulation (03 §9). A `pause` duration goes on with the time it had
/// left; an `elapse` one has `elapsed` taken off, or with `None` the
/// time that passed since [`suspend`] on the clock it was on then. Those
/// whose time has run out end at once, each with `trait_expired` and
/// `trait_removed`, the one that ran out first first.
///
/// The kernel calls this once the entity is back in the world. A
/// duration that still cannot be put on the timeline stands still.
pub fn resume(sim: &mut Sim, entity: EntityId, elapsed: Option<Dur>, cause: Cause) {
    let Some(away) = traits_mut(sim, entity).map(|traits| traits.away.take()) else {
        return;
    };
    let elapsed = elapsed
        .or_else(|| away.map(|away| time_away(sim, away)))
        .unwrap_or(Dur::ZERO)
        .max(Dur::ZERO);
    let held: Vec<(TraitKey, TraitId, Dur)> = traits(sim, entity)
        .map(|traits| {
            traits
                .insts
                .iter()
                .filter_map(|inst| match inst.timer {
                    Timer::Held(left) => Some((inst.key, inst.def, left)),
                    Timer::Lasts | Timer::Running(_) => None,
                })
                .collect()
        })
        .unwrap_or_default();
    let mut overdue: Vec<(Dur, TraitKey)> = Vec::new();
    for (key, def, left) in held {
        let elapses =
            def_of(sim, def).is_some_and(|def| def.while_unloaded == WhileUnloaded::Elapse);
        let left = if elapses { left - elapsed } else { left };
        if left <= Dur::ZERO {
            overdue.push((left, key));
        } else {
            start_timer(sim, entity, key, Some(left));
        }
    }
    overdue.sort_unstable();
    for (_, key) in overdue {
        expire(sim, entity, key, cause);
    }
}

/// Takes the durations of an entity and of everything inside it off the
/// timeline: what `Sim::relocate` calls before something leaves the
/// world.
pub(crate) fn leaving_world(sim: &mut Sim, entity: EntityId) {
    suspend(sim, entity);
    for inside in sim.entities.contents_deep(entity) {
        suspend(sim, inside);
    }
}

/// Puts them back: what `Sim::relocate` calls once something has
/// returned to the world.
pub(crate) fn returned_to_world(sim: &mut Sim, entity: EntityId, cause: Cause) {
    resume(sim, entity, None, cause);
    for inside in sim.entities.contents_deep(entity) {
        resume(sim, inside, None, cause);
    }
}

/// Handles a wake-up of the domain `Timer`: a trait's duration that has
/// run out ends with `trait_expired` and `trait_removed`. A wake-up of
/// the domain that is not a trait's is reported as a fault, as the
/// kernel reports any wake-up nothing handles.
pub fn on_timer(sim: &mut Sim, id: WakeId, wake: Wake) {
    if !is_trait_wake(&wake) {
        sim.fault(
            Reason::new(
                kernel_codes::ADVANCE_UNHANDLED_WAKE,
                format!(
                    "a wake-up of domain {:?} was due and nothing handles it",
                    wake.domain
                ),
            )
            .with("domain", format!("{:?}", wake.domain))
            .with("id", wake.id as i64)
            .with("arg", wake.arg as i64),
        );
        return;
    }
    let entity = EntityId::from_bits(wake.id);
    let Ok(key) = TraitKey::try_from(wake.arg & KEY_BITS) else {
        return;
    };
    // Only the instance's own wake-up ends it: one that was moved or
    // cancelled since is stale.
    let current = traits(sim, entity)
        .and_then(|traits| traits.get(key))
        .is_some_and(|inst| inst.timer == Timer::Running(id));
    if !current {
        return;
    }
    if let Some(inst) = inst_mut(sim, entity, key) {
        inst.timer = Timer::Lasts;
    }
    expire(sim, entity, key, TIME);
}

/// Ends an instance whose duration has run out.
fn expire(sim: &mut Sim, entity: EntityId, key: TraitKey, cause: Cause) {
    let Some(inst) = traits(sim, entity)
        .and_then(|traits| traits.get(key))
        .cloned()
    else {
        return;
    };
    let name = name_of(sim, inst.def);
    sim.emit(
        Where::Entity(entity),
        cause,
        EventBody::Custom(
            EventPayload::new(TRAIT_EXPIRED)
                .subject("self", Subject::Entity(entity))
                .with("trait", name.as_str())
                .with("giver", inst.giver.kind.name()),
        ),
    );
    end(sim, entity, key, "expired", cause);
    reconsider(sim, entity);
}

// ---- Adding ------------------------------------------------------------------

/// Finds the instance of a trait one giver gave.
fn find(sim: &Sim, entity: EntityId, def: TraitId, giver: Giver) -> Option<TraitInst> {
    traits(sim, entity)?
        .insts
        .iter()
        .find(|inst| inst.def == def && inst.giver == giver)
        .cloned()
}

/// How many stacks an instance would have if given again, or `None` if
/// giving it again changes nothing.
fn next_stacks(def: &TraitDef, existing: Option<&TraitInst>) -> Option<u16> {
    match existing {
        None => Some(1),
        Some(inst) => match def.stacking {
            super::Stacking::None => None,
            super::Stacking::Refresh => Some(inst.stacks),
            super::Stacking::Stack => Some(inst.stacks.saturating_add(1).min(def.max_stacks())),
        },
    }
}

/// Gives an entity a trait (06 §5), from a giver.
///
/// - If it does not have the trait from this giver, it gains a new
///   instance with one stack. Traits of the same `group` it has, from
///   any giver, are removed first (`trait_removed`, `how` `replaced`).
/// - If it does, the trait's `stacking` says what happens: `none`
///   changes nothing (and raises nothing), `refresh` starts the
///   duration again, `stack` adds a stack up to `max` and starts the
///   duration again.
///
/// `duration` stands in for the definition's, for this giving; `None`
/// keeps the definition's. The hook `trait_adding` runs first and may
/// cancel or change the duration. What the trait confers is then
/// applied, `trait_added` is raised and, for a new instance, `on_apply`
/// runs. The `while` of every trait the entity has is worked out again.
///
/// Refused, changing nothing, for an unknown entity, an id that is not a
/// trait, a duration that is not above zero, and with the reason a
/// handler cancelled for.
pub fn add_trait(
    sim: &mut Sim,
    entity: EntityId,
    def: TraitId,
    giver: Giver,
    duration: Option<Dur>,
    cause: Cause,
) -> Result<Added, Reason> {
    sim.known(entity)?;
    let defs = Arc::clone(&sim.defs);
    let registry = defs.try_registry::<TraitDef>().ok_or_else(not_registered)?;
    let trait_def = registry
        .try_get(def)
        .ok_or_else(|| unknown(def.index().to_string()))?;
    let name = registry.name(def).to_owned();
    if let Some(given) = duration
        && given <= Dur::ZERO
    {
        return Err(bad_duration(given.as_secs_real()));
    }
    let duration = duration.or_else(|| trait_def.lasts());
    let existing = find(sim, entity, def, giver);
    let Some(stacks) = next_stacks(trait_def, existing.as_ref()) else {
        let inst = existing
            .as_ref()
            .map_or((0, 0), |inst| (inst.key, inst.stacks));
        return Ok(Added {
            key: inst.0,
            stacks: inst.1,
            first: false,
            changed: false,
        });
    };

    let asked = EventPayload::new(TRAIT_ADDING)
        .subject("self", Subject::Entity(entity))
        .with("trait", name.as_str())
        .with("giver", giver.kind.name())
        .with("stacks", i64::from(stacks))
        .with(DURATION_FIELD, duration_field(duration));
    let raised = sim.raise(Where::Entity(entity), cause, asked)?;
    if let Some(reason) = raised.cancelled {
        return Err(reason);
    }
    let duration = match raised.payload.field(DURATION_FIELD) {
        None | Some(FieldValue::Null) => None,
        Some(value) => {
            // Anything but a number is no duration a trait can have.
            let seconds = value.as_real().unwrap_or(Real::INFINITY);
            if !(seconds.is_finite() && seconds > Real::ZERO) {
                return Err(bad_duration(seconds));
            }
            Some(Dur::from_secs_real(seconds).max(Dur::from_micros(1)))
        }
    };

    // The handlers may have given or taken this trait meanwhile.
    let existing = find(sim, entity, def, giver);
    let Some(stacks) = next_stacks(trait_def, existing.as_ref()) else {
        let inst = existing
            .as_ref()
            .map_or((0, 0), |inst| (inst.key, inst.stacks));
        return Ok(Added {
            key: inst.0,
            stacks: inst.1,
            first: false,
            changed: false,
        });
    };
    let first = existing.is_none();
    if first && let Some(group) = &trait_def.group {
        let rivals: Vec<TraitKey> = traits(sim, entity)
            .map(|traits| {
                traits
                    .insts
                    .iter()
                    .filter(|inst| inst.def != def)
                    .filter(|inst| {
                        def_of(sim, inst.def).and_then(|other| other.group.as_ref()) == Some(group)
                    })
                    .map(|inst| inst.key)
                    .collect()
            })
            .unwrap_or_default();
        for key in rivals {
            end(sim, entity, key, "replaced", cause);
        }
    }

    let traits = sim
        .components
        .store_mut::<Traits>()
        .get_or_insert_with(entity, Traits::default);
    let key = match &existing {
        Some(inst) => inst.key,
        None => {
            let key = traits.next_key;
            traits.next_key = traits.next_key.wrapping_add(1);
            traits.insts.push(TraitInst {
                key,
                def,
                giver,
                stacks: 1,
                dormant: false,
                timer: Timer::Lasts,
            });
            key
        }
    };
    if let Some(inst) = inst_mut(sim, entity, key) {
        inst.stacks = stacks;
    }
    let dormant = !holds(sim, entity, trait_def);
    if let Some(inst) = inst_mut(sim, entity, key) {
        inst.dormant = dormant;
    }
    start_timer(sim, entity, key, duration);
    settle(sim, entity, &[key]);
    sim.emit(
        Where::Entity(entity),
        cause,
        EventBody::Custom(
            EventPayload::new(TRAIT_ADDED)
                .subject("self", Subject::Entity(entity))
                .with("trait", name.as_str())
                .with("giver", giver.kind.name())
                .with("stacks", i64::from(stacks))
                .with("first", first)
                .with("duration", duration_field(duration))
                .with("look", look_of(sim, def).as_str()),
        ),
    );
    if first {
        run_effects(sim, entity, &trait_def.on_apply, cause);
    }
    reconsider(sim, entity);
    Ok(Added {
        key,
        stacks,
        first,
        changed: true,
    })
}

/// Gives an entity each of a list of traits named as data writes them
/// (bare or qualified ids), from one giver: what an item, a posture or
/// a template gives. A name that is not a trait, and a giving that is
/// refused, give nothing; the rest are given all the same.
pub fn give_named(sim: &mut Sim, entity: EntityId, names: &[String], giver: Giver, cause: Cause) {
    let defs = Arc::clone(&sim.defs);
    for name in names {
        if let Some(def) = lookup(&defs, name) {
            let _ = add_trait(sim, entity, def, giver, None, cause);
        }
    }
}

// ---- Removing ----------------------------------------------------------------

/// Ends one instance: its timer is cancelled, what it conferred is taken
/// back, `trait_removed` is raised with `how`, and `on_remove` runs.
fn end(sim: &mut Sim, entity: EntityId, key: TraitKey, how: &str, cause: Cause) {
    let Some(traits) = traits_mut(sim, entity) else {
        return;
    };
    let Some(at) = traits.insts.iter().position(|inst| inst.key == key) else {
        return;
    };
    let inst = traits.insts.remove(at);
    if let Timer::Running(wake) = inst.timer {
        sim.cancel_wake(wake);
    }
    settle(sim, entity, &[key]);
    let name = name_of(sim, inst.def);
    sim.emit(
        Where::Entity(entity),
        cause,
        EventBody::Custom(
            EventPayload::new(TRAIT_REMOVED)
                .subject("self", Subject::Entity(entity))
                .with("trait", name.as_str())
                .with("giver", inst.giver.kind.name())
                .with("stacks", i64::from(inst.stacks))
                .with("left", 0_i64)
                .with("how", how)
                .with("look", look_of(sim, inst.def).as_str()),
        ),
    );
    let defs = Arc::clone(&sim.defs);
    if let Some(def) = defs
        .try_registry::<TraitDef>()
        .and_then(|registry| registry.try_get(inst.def))
    {
        run_effects(sim, entity, &def.on_remove, cause);
    }
}

/// Removes a trait from an entity, whoever gave it (06 §5): every
/// instance, or with `stacks` that many stacks, taken from the
/// instances in the order they were added. An instance left with no
/// stacks ends. Returns how many stacks went: none for a trait the
/// entity does not have, which changes nothing.
///
/// The hook `trait_removing` runs first and may cancel. For each
/// instance touched, what it conferred is taken back or restated for
/// its remaining stacks and `trait_removed` is raised (`how`
/// `removed`); an instance that ends runs its `on_remove`.
///
/// Refused, changing nothing, for an unknown entity, an id that is not a
/// trait, a count of stacks of zero, and with the reason a handler
/// cancelled for.
pub fn remove_trait(
    sim: &mut Sim,
    entity: EntityId,
    def: TraitId,
    stacks: Option<u16>,
    cause: Cause,
) -> Result<u32, Reason> {
    sim.known(entity)?;
    let defs = Arc::clone(&sim.defs);
    let registry = defs.try_registry::<TraitDef>().ok_or_else(not_registered)?;
    if registry.try_get(def).is_none() {
        return Err(unknown(def.index().to_string()));
    }
    if stacks == Some(0) {
        return Err(Reason::new(
            codes::BAD_STACKS,
            "a count of stacks to remove must be at least one",
        )
        .with("stacks", 0_i64));
    }
    let have = traits(sim, entity).map_or(0, |traits| traits.stacks(def));
    if have == 0 {
        return Ok(0);
    }
    let going = stacks.map_or(have, |stacks| u32::from(stacks).min(have));
    let name = registry.name(def).to_owned();
    let asked = EventPayload::new(TRAIT_REMOVING)
        .subject("self", Subject::Entity(entity))
        .with("trait", name.as_str())
        .with("stacks", i64::from(going));
    if let Some(reason) = sim.raise(Where::Entity(entity), cause, asked)?.cancelled {
        return Err(reason);
    }
    // The handlers may have changed what there is.
    let insts: Vec<(TraitKey, u16, Giver)> = traits(sim, entity)
        .map(|traits| {
            traits
                .of(def)
                .map(|inst| (inst.key, inst.stacks, inst.giver))
                .collect()
        })
        .unwrap_or_default();
    let mut left_to_take = going;
    let mut taken = 0;
    for (key, have, giver) in insts {
        if left_to_take == 0 {
            break;
        }
        let take = u32::from(have).min(left_to_take);
        left_to_take -= take;
        taken += take;
        if take == u32::from(have) {
            end(sim, entity, key, "removed", cause);
            continue;
        }
        // Fewer than it has: some stacks go and the rest stay.
        let remaining = have - u16::try_from(take).unwrap_or(have);
        if let Some(inst) = inst_mut(sim, entity, key) {
            inst.stacks = remaining;
        }
        settle(sim, entity, &[key]);
        sim.emit(
            Where::Entity(entity),
            cause,
            EventBody::Custom(
                EventPayload::new(TRAIT_REMOVED)
                    .subject("self", Subject::Entity(entity))
                    .with("trait", name.as_str())
                    .with("giver", giver.kind.name())
                    .with("stacks", i64::from(take))
                    .with("left", i64::from(remaining))
                    .with("how", "removed")
                    .with("look", look_of(sim, def).as_str()),
            ),
        );
    }
    reconsider(sim, entity);
    Ok(taken)
}

/// Ends every trait one giver gave an entity, without the hook
/// `trait_removing` (`trait_removed`, `how` `withdrawn`): what an item
/// that is let go of, a posture that is left or an ability that ends
/// calls. Returns how many instances ended.
pub fn withdraw(sim: &mut Sim, entity: EntityId, giver: Giver, cause: Cause) -> usize {
    let keys: Vec<TraitKey> = traits(sim, entity)
        .map(|traits| {
            traits
                .insts
                .iter()
                .filter(|inst| inst.giver == giver)
                .map(|inst| inst.key)
                .collect()
        })
        .unwrap_or_default();
    for key in &keys {
        end(sim, entity, *key, "withdrawn", cause);
    }
    if !keys.is_empty() {
        reconsider(sim, entity);
    }
    keys.len()
}

/// What `movement::set_posture` calls once an entity's posture has
/// changed (06 §11.1): the traits the old posture gave are withdrawn and
/// those the new one lists are given.
pub fn posture_changed(
    sim: &mut Sim,
    entity: EntityId,
    old: Option<PostureId>,
    new: Option<PostureId>,
    cause: Cause,
) {
    if let Some(old) = old {
        withdraw(sim, entity, Giver::posture(old), cause);
    }
    let Some(new) = new else {
        return;
    };
    let defs = Arc::clone(&sim.defs);
    let names = defs
        .try_registry::<PostureDef>()
        .and_then(|registry| registry.try_get(new))
        .map(|posture| posture.traits.clone())
        .unwrap_or_default();
    give_named(sim, entity, &names, Giver::posture(new), cause);
}

// ---- Dormancy and effects ----------------------------------------------------

/// Works out again whether each of an entity's traits is dormant (its
/// `while` does not hold) and brings what they confer into line. Done
/// whenever a trait of the entity is added or removed; the kernel calls
/// it when something else a `while` reads may have changed, until
/// reactive expressions (05 §7.5) keep it current.
pub fn reconsider(sim: &mut Sim, entity: EntityId) {
    let insts: Vec<(TraitKey, TraitId, bool)> = traits(sim, entity)
        .map(|traits| {
            traits
                .insts
                .iter()
                .map(|inst| (inst.key, inst.def, inst.dormant))
                .collect()
        })
        .unwrap_or_default();
    let mut changed = Vec::new();
    for (key, def, dormant) in insts {
        let Some(def) = def_of(sim, def) else {
            continue;
        };
        if def.condition.is_none() {
            continue;
        }
        let now_dormant = !holds(sim, entity, def);
        if now_dormant != dormant {
            changed.push(key);
            if let Some(inst) = inst_mut(sim, entity, key) {
                inst.dormant = now_dormant;
            }
        }
    }
    if !changed.is_empty() {
        settle(sim, entity, &changed);
    }
}

/// Runs a trait's `on_apply` or `on_remove` with its holder as `self`.
///
/// The rule runtime does not yet run against the simulation (its
/// binding, 05 §13.1, is another package's), so the effects are
/// compiled and kept, and this is the one place they will be run from.
pub fn run_effects(_sim: &mut Sim, _entity: EntityId, _effects: &Effects, _cause: Cause) {}
