//! Bindings (08 §7.3): what an ability leaves in the world, kept with
//! the ability and ended with it.
//!
//! | What is bound | Made by | `remove` | `restore` | `release` |
//! |---------------|---------|----------|-----------|-----------|
//! | A trait on someone | `bind_trait` | The trait is taken away | As `remove` | The trait stays, given by no one who will take it back |
//! | An entity | `summon` | It is removed from the simulation | As `remove` | It stays, free |
//! | Cells | `conjure` | What was conjured is taken away, where it is still what was conjured | What was there is put back, where the cell is still what was conjured | The cells stay as they are |
//! | An area | `create_area` with `bound` | The area is removed | As `remove` | It stays |
//! | A rule on someone | `bind_rule` | The rule is detached | As `remove` | It stays attached |
//!
//! A binding ends when its time (`for`, on its owner's own time) runs
//! out, when it is dismissed (`dismiss`) or dispelled (`dispel`), and,
//! as its `ends_on` says, when the mode that made it ends (`mode`), when
//! its ability is suppressed (`suppressed`) or goes dormant (`dormant`),
//! or when its owner dies (`owner_stops`). Left out, `ends_on` is all of
//! them. It always ends when its ability is revoked or lost, and when
//! its owner is removed (invariant A3). Before it ends the hook
//! `binding_ending` runs: a handler may change `on_end`, or prolong it,
//! or cancel; when the ability itself is gone neither prolonging nor
//! cancelling keeps it.
//!
//! Bindings are kept by the simulation, by a number given once in its
//! life ([`AbilityIndex`](super::AbilityIndex)), so that they can be
//! ended when their owner is gone.

use std::sync::Arc;

use clash_core::{Dur, EntityId, Real, impl_state_hash};
use clash_data::Def;
use clash_expr::{Registry, Value, VerbSpec};
use clash_time::WakeId;
use clash_world::{AreaId, Cause, CauseKind, ElementState, Layer, Location};
use serde::{Deserialize, Serialize};

use super::ops::{inst, notify};
use super::{
    Abilities, AbilityDef, BINDING_CREATED, BINDING_ENDED, BINDING_ENDING, codes, def_of, name_of,
    number_of, owner_of, timer_wake,
};
use crate::Sim;
use crate::bus::{EventPayload, FieldValue};
use crate::entity::Place;
use crate::event::{Subject, Where};
use crate::reason::Reason;
use crate::rule::{EocDef, EocId, VerbCall};
use crate::template::{CharacterSpec, TemplateDef};
use crate::traits::{Giver, GiverKind, TraitDef};
use crate::verb::VerbDef;

/// The setting that limits how many bindings one ability may hold at
/// once (08 §12).
pub const MAX_BINDINGS_SETTING: &str = "rules.max_bindings";

/// The default of [`MAX_BINDINGS_SETTING`].
pub const DEFAULT_MAX_BINDINGS: i64 = 64;

/// The most cells one `conjure` places.
pub const CONJURE_MAX_CELLS: usize = 64;

/// The bit of a trait giver's number that marks a bound trait: the
/// rest is the binding's number. An ability's own givers
/// ([`slot_id`](super::slot_id)) never reach it.
const BOUND_TRAIT_GIVER: u64 = 1 << 63;

/// The giver of a trait a binding put on someone.
pub const fn binding_giver(binding: u64) -> Giver {
    Giver::new(GiverKind::Ability, BOUND_TRAIT_GIVER | binding)
}

/// What happens to what is bound when its binding ends (08 §7.3).
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[repr(u8)]
pub enum OnEnd {
    /// It goes: the trait is taken away, the entity removed, the area
    /// removed, the rule detached, what was conjured taken away.
    Remove,
    /// For cells, what was there is put back; for anything else, as
    /// `remove`.
    Restore,
    /// It stays where it is, no longer bound.
    Release,
}

impl OnEnd {
    /// The word data writes.
    pub const fn name(self) -> &'static str {
        match self {
            OnEnd::Remove => "remove",
            OnEnd::Restore => "restore",
            OnEnd::Release => "release",
        }
    }

    /// The word, read.
    pub fn parse(word: &str) -> Option<OnEnd> {
        [OnEnd::Remove, OnEnd::Restore, OnEnd::Release]
            .into_iter()
            .find(|on_end| on_end.name() == word)
    }
}

impl clash_core::StateHash for OnEnd {
    fn state_hash(&self, hasher: &mut clash_core::StateHasher) {
        hasher.write_u8(*self as u8);
    }
}

/// When a binding ends besides its time running out, being dismissed or
/// dispelled, and its ability being lost: bits of [`Binding::ends_on`].
pub mod ends_on {
    /// When the mode it was made by ends.
    pub const MODE: u8 = 1;
    /// When its ability is suppressed.
    pub const SUPPRESSED: u8 = 2;
    /// When its ability goes dormant.
    pub const DORMANT: u8 = 4;
    /// When its owner dies (06 §8.3).
    pub const OWNER_STOPS: u8 = 8;
    /// All of them: what a binding ends on unless it says.
    pub const ALL: u8 = MODE | SUPPRESSED | DORMANT | OWNER_STOPS;
    /// The words data writes, with their bits.
    pub const WORDS: [(&str, u8); 4] = [
        ("mode", MODE),
        ("suppressed", SUPPRESSED),
        ("dormant", DORMANT),
        ("owner_stops", OWNER_STOPS),
    ];
}

/// One cell `conjure` placed something in.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub struct Conjured {
    /// The cell.
    pub at: Location,
    /// What was there before.
    pub before: ElementState,
    /// What was put there.
    pub placed: ElementState,
}

impl_state_hash!(Conjured { at, before, placed });

/// What a binding holds (08 §7.3). Portals are not built (02 §19), so
/// nothing can be bound to one yet.
#[derive(Clone, PartialEq, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Bound {
    /// A trait given to someone with the binding as its giver
    /// ([`binding_giver`]).
    Trait {
        /// Who has it.
        on: EntityId,
        /// Which trait.
        def: crate::traits::TraitId,
    },
    /// An entity, made by `summon`.
    Entity {
        /// Which.
        id: EntityId,
    },
    /// Elements placed by `conjure`, on one layer.
    Edit {
        /// The layer.
        layer: Layer,
        /// The cells, in the order they were placed.
        cells: Vec<Conjured>,
    },
    /// An area.
    Area {
        /// Which.
        id: AreaId,
    },
    /// A rule attached to someone.
    Rule {
        /// Who it is attached to.
        on: EntityId,
        /// Which rule.
        eoc: EocId,
    },
}

impl Bound {
    /// The word rules read: `trait`, `entity`, `edit`, `area` or `rule`.
    pub const fn kind(&self) -> &'static str {
        match self {
            Bound::Trait { .. } => "trait",
            Bound::Entity { .. } => "entity",
            Bound::Edit { .. } => "edit",
            Bound::Area { .. } => "area",
            Bound::Rule { .. } => "rule",
        }
    }
}

impl clash_core::StateHash for Bound {
    fn state_hash(&self, hasher: &mut clash_core::StateHasher) {
        match self {
            Bound::Trait { on, def } => {
                hasher.write_u8(0);
                on.state_hash(hasher);
                def.state_hash(hasher);
            }
            Bound::Entity { id } => {
                hasher.write_u8(1);
                id.state_hash(hasher);
            }
            Bound::Edit { layer, cells } => {
                hasher.write_u8(2);
                layer.state_hash(hasher);
                cells.state_hash(hasher);
            }
            Bound::Area { id } => {
                hasher.write_u8(3);
                id.state_hash(hasher);
            }
            Bound::Rule { on, eoc } => {
                hasher.write_u8(4);
                on.state_hash(hasher);
                eoc.state_hash(hasher);
            }
        }
    }
}

/// Something an ability left in the world, bound to it (08 §7.3).
#[derive(Clone, PartialEq, Debug, Serialize, Deserialize)]
pub struct Binding {
    /// Its number, given once in the simulation's life.
    pub id: u64,
    /// Who holds the ability.
    pub owner: EntityId,
    /// The ability instance, by its number.
    pub number: u64,
    /// What is bound.
    pub what: Bound,
    /// The name it was given (`as`), for `ability.bound(name)`.
    pub name: Option<String>,
    /// The mode whose `on_start` made it, if one did.
    pub mode: Option<u16>,
    /// When it ends besides its time and its ability's loss: bits of
    /// [`ends_on`].
    pub ends_on: u8,
    /// What happens to what is bound when it ends.
    pub on_end: OnEnd,
    /// Its owner's own time at which it ends, if it lasts a time.
    pub until: Option<Dur>,
    /// The wake-up for that moment, while one is scheduled.
    pub wake: Option<WakeId>,
}

impl_state_hash!(Binding {
    id,
    owner,
    number,
    what,
    name,
    mode,
    ends_on,
    on_end,
    until,
    wake
});

/// How a binding is to be made: what the verbs that bind read from
/// their arguments.
#[derive(Clone, Debug)]
pub struct Terms {
    /// The ability instance it is bound to.
    pub number: u64,
    /// Who holds it.
    pub owner: EntityId,
    /// Seconds of the owner's own time it lasts, if it ends by itself.
    pub seconds: Option<Real>,
    /// When it ends: bits of [`ends_on`].
    pub ends_on: u8,
    /// What happens to what is bound when it ends.
    pub on_end: OnEnd,
    /// The name it is given.
    pub name: Option<String>,
}

// ---- Reasons -----------------------------------------------------------------------

fn no_binder() -> Reason {
    Reason::new(
        codes::NO_BINDER,
        "there is no ability to bind it to: give `ability`, or make it where an ability's own effects run",
    )
}

fn bad_term(arg: &str, what: &str) -> Reason {
    Reason::new(
        codes::BAD_BINDING,
        format!("the binding's `{arg}` must be {what}"),
    )
    .with("arg", arg)
}

// ---- Making --------------------------------------------------------------------------

/// Reads the terms of a binding from five arguments of a verb starting
/// at `at`: `ability`, `for`, `ends_on`, `on_end`, `as`. The ability is
/// the argument, or the `ability` bound where the effect is written.
pub(crate) fn read_terms(
    sim: &Sim,
    call: &VerbCall<'_>,
    at: usize,
    default_on_end: OnEnd,
) -> Result<Terms, Reason> {
    let arg = |index: usize| call.args.get(index).unwrap_or(&Value::Null);
    let given = match arg(at) {
        Value::Null => call.by.subject("ability").cloned().unwrap_or(Value::Null),
        value => value.clone(),
    };
    let number = number_of(sim, &given).ok_or_else(no_binder)?;
    let owner = owner_of(sim, number).ok_or_else(no_binder)?;
    let held = inst(sim, owner, number).is_some_and(super::AbilityInst::is_held);
    if !held {
        return Err(no_binder());
    }
    let seconds = match arg(at + 1) {
        Value::Null => None,
        value => match value.as_number() {
            Some(seconds) if seconds.is_finite() && seconds > Real::ZERO => Some(seconds),
            _ => return Err(bad_term("for", "a time above zero, in seconds")),
        },
    };
    let ends_on = match arg(at + 2) {
        Value::Null => ends_on::ALL,
        Value::List(items) => {
            let mut bits = 0;
            for item in items.iter() {
                let word = item.as_str().unwrap_or("");
                let Some((_, bit)) = ends_on::WORDS.iter().find(|(have, _)| *have == word) else {
                    return Err(bad_term(
                        "ends_on",
                        "a list of `mode`, `suppressed`, `dormant` and `owner_stops`",
                    ));
                };
                bits |= bit;
            }
            bits
        }
        _ => {
            return Err(bad_term(
                "ends_on",
                "a list of `mode`, `suppressed`, `dormant` and `owner_stops`",
            ));
        }
    };
    let on_end = match arg(at + 3) {
        Value::Null => default_on_end,
        value => value
            .as_str()
            .and_then(OnEnd::parse)
            .ok_or_else(|| bad_term("on_end", "`remove`, `restore` or `release`"))?,
    };
    let name = match arg(at + 4) {
        Value::Null => None,
        Value::Str(name) if !name.is_empty() => Some(name.to_string()),
        _ => return Err(bad_term("as", "a name")),
    };
    Ok(Terms {
        number,
        owner,
        seconds,
        ends_on,
        on_end,
        name,
    })
}

/// Refuses a binding past `rules.max_bindings` for one ability.
pub(crate) fn check_room(sim: &Sim, number: u64) -> Result<(), Reason> {
    let limit = sim
        .setting_int(MAX_BINDINGS_SETTING, DEFAULT_MAX_BINDINGS)
        .max(0);
    let held = sim
        .abilities
        .bindings
        .values()
        .filter(|binding| binding.number == number)
        .count();
    if i64::try_from(held).unwrap_or(i64::MAX) >= limit {
        return Err(Reason::new(
            codes::TOO_MANY_BINDINGS,
            format!(
                "the ability holds {held} bindings, as many as `{MAX_BINDINGS_SETTING}` allows"
            ),
        )
        .with("limit", limit));
    }
    Ok(())
}

/// Binds something to an ability on the terms given (08 §7.3): the
/// binding is kept, its time is scheduled on its owner's clock and
/// `binding_created` is raised. Returns its number. The caller has
/// made the thing and checked [`check_room`].
pub(crate) fn bind(sim: &mut Sim, terms: &Terms, what: Bound, cause: Cause) -> u64 {
    let id = reserve(sim);
    bind_as(sim, id, terms, what, cause);
    id
}

/// Takes the number the next binding will have, for one whose thing
/// must know it before it is bound (a trait's giver).
fn reserve(sim: &mut Sim) -> u64 {
    let id = sim.abilities.next_binding;
    sim.abilities.next_binding += 1;
    id
}

/// [`bind`] under a number already taken.
fn bind_as(sim: &mut Sim, id: u64, terms: &Terms, what: Bound, cause: Cause) {
    let mode = sim
        .ability_rt
        .making
        .filter(|(number, _)| *number == terms.number)
        .map(|(_, mode)| mode);
    let until = terms
        .seconds
        .map(|seconds| sim.own_time(terms.owner) + Dur::from_secs_real(seconds));
    let kind = what.kind();
    sim.abilities.bindings.insert(
        id,
        Binding {
            id,
            owner: terms.owner,
            number: terms.number,
            what,
            name: terms.name.clone(),
            mode,
            ends_on: terms.ends_on,
            on_end: terms.on_end,
            until,
            wake: None,
        },
    );
    schedule(sim, id);
    let ability = inst(sim, terms.owner, terms.number)
        .map(|inst| name_of(&sim.defs, inst.def()))
        .unwrap_or_default();
    notify(
        sim,
        terms.owner,
        EventPayload::new(BINDING_CREATED)
            .with("ability", ability.as_str())
            .with("name", terms.name.as_deref().unwrap_or(""))
            .with("what", kind)
            .with("on_end", terms.on_end.name()),
        cause,
    );
}

// ---- Ending ------------------------------------------------------------------------

/// Why a binding ends, as rules read it, and whether the ability is
/// gone so that it cannot be kept.
fn is_final(why: &str) -> bool {
    matches!(why, "revoked" | "lost" | "moved" | "owner_removed")
}

/// Ends one binding (08 §7.3): `binding_ending` runs first, and may
/// change how it ends, prolong it or keep it; then what is bound is
/// removed, restored or released, and `binding_ended` is raised.
/// Returns whether it ended.
pub(crate) fn end(sim: &mut Sim, id: u64, why: &str, cause: Cause) -> bool {
    let Some(binding) = sim.abilities.bindings.get(&id).cloned() else {
        return false;
    };
    let ability = inst(sim, binding.owner, binding.number)
        .map(|inst| name_of(&sim.defs, inst.def()))
        .unwrap_or_default();
    let mut on_end = binding.on_end;
    if sim.bus.has_listeners(BINDING_ENDING) && sim.entities.contains(binding.owner) {
        let payload = EventPayload::new(BINDING_ENDING)
            .subject("self", Subject::Entity(binding.owner))
            .with("ability", ability.as_str())
            .with("name", binding.name.as_deref().unwrap_or(""))
            .with("what", binding.what.kind())
            .with("why", why)
            .with("on_end", on_end.name())
            .with("prolong", Real::ZERO);
        match sim.raise(Where::Entity(binding.owner), cause, payload) {
            Ok(raised) => {
                if let Some(FieldValue::Plain(clash_core::Value::Str(word))) =
                    raised.payload.field("on_end")
                {
                    match OnEnd::parse(word) {
                        Some(changed) => on_end = changed,
                        None => sim.fault(bad_term(
                            "on_end",
                            &format!("`remove`, `restore` or `release`, not {word:?}"),
                        )),
                    }
                }
                let prolong = raised
                    .payload
                    .field("prolong")
                    .and_then(FieldValue::as_real)
                    .filter(|seconds| seconds.is_finite() && *seconds > Real::ZERO);
                if !is_final(why) {
                    if let Some(seconds) = prolong {
                        let until = sim.own_time(binding.owner) + Dur::from_secs_real(seconds);
                        if let Some(kept) = sim.abilities.bindings.get_mut(&id) {
                            kept.until = Some(until);
                        }
                        schedule(sim, id);
                        return false;
                    }
                    if raised.cancelled.is_some() {
                        if why == "duration"
                            && let Some(kept) = sim.abilities.bindings.get_mut(&id)
                        {
                            kept.until = None;
                            kept.wake = None;
                        }
                        return false;
                    }
                }
            }
            Err(reason) => sim.fault(reason),
        }
    }
    // A handler may have ended it already.
    let Some(binding) = sim.abilities.bindings.remove(&id) else {
        return false;
    };
    if let Some(wake) = binding.wake {
        sim.cancel_wake(wake);
    }
    let thing = carry_out(sim, &binding, on_end, cause);
    let payload = EventPayload::new(BINDING_ENDED)
        .subject("self", Subject::Entity(binding.owner))
        .with("ability", ability.as_str())
        .with("name", binding.name.as_deref().unwrap_or(""))
        .with("what", binding.what.kind())
        .with("why", why)
        .with("how", on_end.name())
        .with("thing", thing);
    let on = if sim.entities.contains(binding.owner) {
        Where::Entity(binding.owner)
    } else {
        Where::Global
    };
    sim.emit(on, cause, crate::event::EventBody::Custom(payload));
    true
}

/// Does to what a binding holds what its ending says. Returns what the
/// thing was, for the event: the entity, the area, the holder of the
/// trait or rule, or the first cell.
fn carry_out(sim: &mut Sim, binding: &Binding, on_end: OnEnd, cause: Cause) -> FieldValue {
    match &binding.what {
        Bound::Trait { on, .. } => {
            if on_end != OnEnd::Release && sim.entities.contains(*on) {
                crate::traits::withdraw(sim, *on, binding_giver(binding.id), cause);
            }
            FieldValue::Subject(Subject::Entity(*on))
        }
        Bound::Entity { id } => {
            let thing = FieldValue::Subject(Subject::Entity(*id));
            if on_end != OnEnd::Release
                && sim.entities.contains(*id)
                && let Err(reason) = sim.remove_entity(*id, cause)
            {
                sim.fault(reason);
            }
            thing
        }
        Bound::Area { id } => {
            if on_end != OnEnd::Release && sim.cosmos.area(*id).is_some() {
                let removed = sim.world(|ctx| ctx.remove_area(*id, cause));
                if let Err(error) = removed {
                    sim.fault(Reason::from_world(&error));
                }
            }
            FieldValue::Subject(Subject::Area(*id))
        }
        Bound::Rule { on, eoc } => {
            if on_end != OnEnd::Release
                && let Some(handle) = sim.exprs.kinds().handle(Subject::Entity(*on))
            {
                sim.rules.detach(handle, *eoc);
            }
            FieldValue::Subject(Subject::Entity(*on))
        }
        Bound::Edit { layer, cells } => {
            if on_end != OnEnd::Release {
                for cell in cells {
                    let now = sim
                        .cosmos
                        .grid(cell.at.grid)
                        .and_then(|grid| grid.element(cell.at.cell, *layer));
                    // Only what is still what was conjured; a cell that
                    // is not loaded is left as it is.
                    if now != Ok(cell.placed) {
                        continue;
                    }
                    let to = match on_end {
                        OnEnd::Restore => cell.before,
                        _ => ElementState::NONE,
                    };
                    let written = sim.world(|ctx| ctx.edit(cause).set_element(cell.at, *layer, to));
                    if let Err(error) = written {
                        sim.fault(Reason::from_world(&error));
                    }
                }
            }
            cells.first().map_or(FieldValue::Null, |cell| {
                FieldValue::Subject(Subject::Loc(cell.at))
            })
        }
    }
}

/// The numbers of the bindings of an instance, in the order made, that
/// a filter keeps.
fn of_instance(sim: &Sim, number: u64, keep: impl Fn(&Binding) -> bool) -> Vec<u64> {
    sim.abilities
        .bindings
        .values()
        .filter(|binding| binding.number == number)
        .filter(|binding| keep(binding))
        .map(|binding| binding.id)
        .collect()
}

/// Ends the bindings of an instance that end on what has happened to
/// it: a bit of [`ends_on`], with the word rules read.
pub(crate) fn ending_on(sim: &mut Sim, number: u64, bit: u8, why: &str, cause: Cause) {
    for id in of_instance(sim, number, |binding| binding.ends_on & bit != 0) {
        end(sim, id, why, cause);
    }
}

/// Ends the bindings a mode made that end with it.
pub(crate) fn mode_ended(sim: &mut Sim, number: u64, mode: u16, cause: Cause) {
    let made = of_instance(sim, number, |binding| {
        binding.mode == Some(mode) && binding.ends_on & ends_on::MODE != 0
    });
    for id in made {
        end(sim, id, "mode", cause);
    }
}

/// Ends every binding of an instance: its ability is revoked, lost or
/// gone with its owner (invariant A3).
pub(crate) fn end_all(sim: &mut Sim, number: u64, why: &str, cause: Cause) {
    for id in of_instance(sim, number, |_| true) {
        end(sim, id, why, cause);
    }
}

/// Follows an instance that moved to another entity with a mind: its
/// bindings are its new owner's.
pub(crate) fn moved(sim: &mut Sim, number: u64, to: EntityId) {
    for binding in sim.abilities.bindings.values_mut() {
        if binding.number == number {
            binding.owner = to;
        }
    }
    let ids = of_instance(sim, number, |_| true);
    for id in ids {
        schedule(sim, id);
    }
}

/// What the kernel's events mean for bindings: an owner that died or
/// was removed, a bound entity that is gone.
pub(crate) fn on_event(sim: &mut Sim, name: &str, entity: EntityId, cause: Cause) {
    match name {
        "entity_died" => {
            let numbers: Vec<u64> = sim
                .abilities
                .bindings
                .values()
                .filter(|binding| binding.owner == entity)
                .map(|binding| binding.number)
                .collect();
            let mut seen = Vec::new();
            for number in numbers {
                if !seen.contains(&number) {
                    seen.push(number);
                    ending_on(sim, number, ends_on::OWNER_STOPS, "owner_stops", cause);
                }
            }
        }
        "entity_removed" => removed(sim, entity, cause),
        "entity_relocated" => reschedule(sim, entity),
        _ => {}
    }
}

/// An entity was removed from the simulation: every binding it owned
/// ends (`owner_removed`), and a binding that held it is forgotten.
pub(crate) fn removed(sim: &mut Sim, entity: EntityId, cause: Cause) {
    let owned: Vec<u64> = sim
        .abilities
        .bindings
        .values()
        .filter(|binding| binding.owner == entity)
        .map(|binding| binding.id)
        .collect();
    for id in owned {
        end(sim, id, "owner_removed", cause);
    }
    sim.abilities
        .bindings
        .retain(|_, binding| !matches!(binding.what, Bound::Entity { id } if id == entity));
}

// ---- Time -----------------------------------------------------------------------------

/// Schedules the wake-up for the moment a binding's time runs out,
/// replacing the one it had. None while its owner is out of the world
/// (its own time stands still) or it lasts until something ends it.
fn schedule(sim: &mut Sim, id: u64) {
    let Some(binding) = sim.abilities.bindings.get(&id).cloned() else {
        return;
    };
    if let Some(wake) = binding.wake {
        sim.cancel_wake(wake);
    }
    let wake = binding.until.and_then(|until| {
        let after = (until - sim.own_time(binding.owner)).max(Dur::ZERO);
        sim.schedule_for(binding.owner, after, timer_wake(binding.owner))
            .ok()
    });
    if let Some(binding) = sim.abilities.bindings.get_mut(&id) {
        binding.wake = wake;
    }
}

/// Schedules again the wake-ups of an owner's bindings whose wake-up is
/// gone: it came back into the world.
pub(crate) fn reschedule(sim: &mut Sim, owner: EntityId) {
    let lost: Vec<u64> = sim
        .abilities
        .bindings
        .values()
        .filter(|binding| binding.owner == owner && binding.until.is_some())
        .filter(|binding| {
            binding
                .wake
                .is_none_or(|wake| sim.wake_owner(wake) != Some(owner))
        })
        .map(|binding| binding.id)
        .collect();
    for id in lost {
        schedule(sim, id);
    }
}

/// A timer of abilities popped for an entity: if it is the wake-up of a
/// binding it owns, the binding ends once its time has run (`duration`).
/// Returns whether it was one.
pub(crate) fn on_wake(sim: &mut Sim, id: WakeId, entity: EntityId) -> bool {
    let Some(binding) = sim
        .abilities
        .bindings
        .values()
        .find(|binding| binding.owner == entity && binding.wake == Some(id))
        .cloned()
    else {
        return false;
    };
    if let Some(kept) = sim.abilities.bindings.get_mut(&binding.id) {
        kept.wake = None;
    }
    let due = binding
        .until
        .is_none_or(|until| sim.own_time(entity) >= until);
    if due {
        end(sim, binding.id, "duration", Cause::new(CauseKind::Process));
    } else {
        schedule(sim, binding.id);
    }
    true
}

// ---- Reading ------------------------------------------------------------------------

/// What `ability.bound(name)` gives: the thing the instance's binding of
/// that name holds, if it is still there: the entity, the area, the one
/// a trait or a rule is on, or the cells conjured as a list of `loc`.
pub fn bound_value(sim: &Sim, number: u64, name: &str) -> Value {
    let Some(binding) = sim
        .abilities
        .bindings
        .values()
        .find(|binding| binding.number == number && binding.name.as_deref() == Some(name))
    else {
        return Value::Null;
    };
    match &binding.what {
        Bound::Entity { id } | Bound::Trait { on: id, .. } | Bound::Rule { on: id, .. } => {
            if sim.entities.contains(*id) {
                sim.subject_value(Subject::Entity(*id))
            } else {
                Value::Null
            }
        }
        Bound::Area { id } => {
            if sim.cosmos.area(*id).is_some() {
                sim.subject_value(Subject::Area(*id))
            } else {
                Value::Null
            }
        }
        Bound::Edit { cells, .. } => Value::list(
            cells
                .iter()
                .map(|cell| sim.subject_value(Subject::Loc(cell.at))),
        ),
    }
}

/// The owner of the ability a summoned entity is bound to, if it is
/// bound by one.
pub fn bound_by(sim: &Sim, entity: EntityId) -> Option<EntityId> {
    sim.abilities
        .bindings
        .values()
        .find(|binding| matches!(binding.what, Bound::Entity { id } if id == entity))
        .map(|binding| binding.owner)
}

impl Sim {
    /// Every binding of the simulation, by its number (08 §7.3).
    pub fn bindings(&self) -> impl Iterator<Item = &Binding> + '_ {
        self.abilities.bindings.values()
    }
}

// ---- The verbs ------------------------------------------------------------------------

/// One argument as a verb's specification writes it: name, type, whether
/// it may be left out, what it means.
type ArgRow = (&'static str, &'static str, bool, &'static str);

/// The terms every verb that binds takes, in this order.
pub(crate) const TERM_ARGS: [ArgRow; 5] = [
    (
        "ability",
        "ability",
        true,
        "The ability it is bound to. Left out, the `ability` where the effect is written: a mode's `on_start`, an ability's own effects.",
    ),
    (
        "for",
        "real",
        true,
        "Seconds of the owner's own time before the binding ends. Left out, it lasts until something else ends it.",
    ),
    (
        "ends_on",
        "list<string>",
        true,
        "What else ends it: `mode` (the mode that made it ends), `suppressed`, `dormant` (its ability is), `owner_stops` (its owner dies). Left out, all of them. Revoking or losing the ability always ends it.",
    ),
    (
        "on_end",
        "string",
        true,
        "What happens to what is bound when it ends: `remove`, `restore` (cells: put back what was there) or `release` (it stays, unbound).",
    ),
    (
        "as",
        "string",
        true,
        "A name for the binding: `ability.bound(name)` then gives what it holds.",
    ),
];

fn spec(registry: &Registry, name: &str, doc: &str, args: &[ArgRow]) -> Option<VerbSpec> {
    let mut spec = VerbSpec::new(name).doc(doc);
    for (arg, written, optional, arg_doc) in args.iter().chain(TERM_ARGS.iter()) {
        let ty = registry.parse_type(written)?;
        spec = if *optional {
            spec.optional_arg(*arg, ty.nullable(), *arg_doc)
        } else {
            spec.arg(*arg, ty, *arg_doc)
        };
    }
    Some(spec)
}

fn bad_argument(name: &str, wanted: &str) -> Reason {
    Reason::new(
        codes::BAD_ARGUMENT,
        format!("the argument `{name}` must be {wanted}"),
    )
    .with("arg", name)
}

fn arg<'a>(call: &'a VerbCall<'_>, index: usize) -> &'a Value {
    call.args.get(index).unwrap_or(&Value::Null)
}

fn entity_arg(
    sim: &Sim,
    call: &VerbCall<'_>,
    index: usize,
    name: &str,
) -> Result<EntityId, Reason> {
    match sim.subject_of(arg(call, index)) {
        Some(Subject::Entity(id)) if sim.entities.contains(id) => Ok(id),
        _ => Err(bad_argument(name, "an entity that exists")),
    }
}

fn loc_arg(sim: &Sim, call: &VerbCall<'_>, index: usize, name: &str) -> Result<Location, Reason> {
    match sim.subject_of(arg(call, index)) {
        Some(Subject::Loc(loc) | Subject::Element(loc, _)) => Ok(loc),
        _ => Err(bad_argument(name, "a cell")),
    }
}

/// The definition an argument names, as an index of its type: an id,
/// or its qualified id as text.
fn def_arg<T: Def>(
    sim: &Sim,
    call: &VerbCall<'_>,
    index: usize,
    name: &str,
) -> Result<usize, Reason> {
    let ty = sim.exprs.registry().def_type_id(T::TYPE);
    match arg(call, index) {
        Value::Id(id) if Some(id.ty) == ty => Ok(id.index as usize),
        Value::Str(text) => sim
            .defs
            .try_registry::<T>()
            .and_then(|registry| registry.lookup(text))
            .map(clash_core::DenseId::index)
            .ok_or_else(|| bad_argument(name, &format!("a `{}` definition", T::TYPE))),
        _ => Err(bad_argument(name, &format!("a `{}` definition", T::TYPE))),
    }
}

fn number_out(count: usize) -> Value {
    Value::Int(i64::try_from(count).unwrap_or(i64::MAX))
}

/// `summon`: makes an entity from a template, bound (08 §7.3).
fn summon(sim: &mut Sim, call: &VerbCall<'_>, cause: Cause) -> Result<Value, Reason> {
    let template = def_arg::<TemplateDef>(sim, call, 0, "template")?;
    let at = loc_arg(sim, call, 1, "at")?;
    let mind = match arg(call, 2) {
        Value::Null => None,
        Value::Str(name) => Some(name.to_string()),
        _ => return Err(bad_argument("mind", "the name of a `mind`")),
    };
    let obey = match arg(call, 3) {
        Value::Null => true,
        Value::Bool(obey) => *obey,
        _ => return Err(bad_argument("obey", "true or false")),
    };
    let terms = read_terms(sim, call, 4, OnEnd::Remove)?;
    check_room(sim, terms.number)?;
    if let Some(mind) = &mind
        && sim.ai_defs.mind_named(mind).is_none()
    {
        return Err(bad_argument("mind", "the name of a `mind` that is loaded"));
    }
    let name = crate::template::TemplateId::from_index(template)
        .and_then(|id| sim.template_defs().name(id).map(str::to_owned))
        .ok_or_else(|| bad_argument("template", "a `template` definition"))?;
    let spawned = sim.spawn_template(&CharacterSpec::of([name]), Place::at(at), cause)?;
    let servant = spawned.entity;
    if let Some(mind) = &mind {
        crate::ai::ensure_state_with(sim, servant, mind);
    }
    if obey {
        obey_summoner(sim, servant, terms.owner);
    }
    give_mind_senses(sim, servant);
    bind(sim, &terms, Bound::Entity { id: servant }, cause);
    crate::ai::prompt(sim, servant);
    Ok(sim.subject_value(Subject::Entity(servant)))
}

/// A summoned entity that obeys takes the stances its summoner's mind
/// holds (09 §3.4), and holds none toward its summoner.
fn obey_summoner(sim: &mut Sim, servant: EntityId, summoner: EntityId) {
    let stances: Vec<(EntityId, crate::ai::Stance)> = sim
        .components
        .mind_state
        .get(summoner)
        .map(|state| {
            state
                .stances()
                .iter()
                .map(|(persona, stance)| (persona.entity(), *stance))
                .collect()
        })
        .unwrap_or_default();
    for (other, stance) in stances {
        if other != servant {
            crate::ai::set_stance(sim, servant, other, Some(stance));
        }
    }
    crate::ai::set_stance(sim, servant, summoner, None);
    // It knows whom it serves, and where they are, as if told (15 §7).
    let at = sim.entities.location(summoner);
    if let Err(reason) = crate::perception::add_track(
        sim,
        servant,
        summoner,
        at,
        crate::perception::Knowing::Told,
        Real::ONE,
    ) {
        sim.fault(reason);
    }
}

/// Gives a summoned entity the senses its mind names, if it has none:
/// what its first thought would have given it, had its mind's state not
/// been made before (09 §3.1).
fn give_mind_senses(sim: &mut Sim, servant: EntityId) {
    if sim.components.senses.contains(servant) {
        return;
    }
    let defs = Arc::clone(&sim.defs);
    let Some(def) = crate::ai::mind_def_of(sim, servant)
        .and_then(|id| defs.try_registry::<crate::ai::MindDef>()?.try_get(id))
    else {
        return;
    };
    for name in &def.senses {
        if let Some(sense) = sim.perception_defs.sense_named(name)
            && let Err(reason) = crate::perception::attach_sense(sim, servant, sense)
        {
            sim.fault(reason);
        }
    }
}

/// The cells of a straight line from one cell to another, both
/// included, each a step from the last.
fn line(from: clash_world::CellPos, to: clash_world::CellPos) -> Vec<clash_world::CellPos> {
    let (dx, dy, dz) = (to.x - from.x, to.y - from.y, to.z - from.z);
    let steps = dx.abs().max(dy.abs()).max(dz.abs());
    if steps == 0 {
        return vec![from];
    }
    let at = |start: i32, delta: i32, i: i32| start + (2 * delta * i + steps).div_euclid(2 * steps);
    (0..=steps)
        .map(|i| clash_world::CellPos::new(at(from.x, dx, i), at(from.y, dy, i), at(from.z, dz, i)))
        .collect()
}

/// `conjure`: places an element in a cell or a line of cells, bound and
/// remembering what was there (08 §7.3).
fn conjure(sim: &mut Sim, call: &VerbCall<'_>, cause: Cause) -> Result<Value, Reason> {
    let at = loc_arg(sim, call, 0, "at")?;
    let to = match arg(call, 1) {
        Value::Null => at,
        _ => loc_arg(sim, call, 1, "to")?,
    };
    if to.grid != at.grid {
        return Err(bad_argument("to", "a cell of the same grid as `at`"));
    }
    let layer = match arg(call, 2) {
        Value::Null => Layer::Block,
        Value::Str(name) => crate::world_verbs::layer_named(name)
            .ok_or_else(|| bad_argument("layer", "`floor`, `block` or `fixture`"))?,
        _ => return Err(bad_argument("layer", "`floor`, `block` or `fixture`")),
    };
    let written = arg(call, 3)
        .as_str()
        .ok_or_else(|| bad_argument("what", "a state as data writes it"))?
        .to_owned();
    let placed = {
        let defs = sim.cosmos.defs();
        let found = match layer {
            Layer::Floor => defs.floor_state(&written, None).map(|state| state.raw()),
            Layer::Block => defs.block_state(&written, None).map(|state| state.raw()),
            Layer::Fixture => defs.fixture_state(&written, None).map(|state| state.raw()),
        };
        found.map_err(|error| {
            Reason::new(crate::world_verbs::codes::UNKNOWN_STATE, error.to_string())
                .with("state", written.as_str())
        })?
    };
    let terms = read_terms(sim, call, 4, OnEnd::Restore)?;
    check_room(sim, terms.number)?;
    let cells = line(at.cell, to.cell);
    if cells.len() > CONJURE_MAX_CELLS {
        return Err(Reason::new(
            codes::BAD_ARGUMENT,
            format!(
                "`conjure` places at most {CONJURE_MAX_CELLS} cells at once, not {}",
                cells.len()
            ),
        )
        .with("arg", "to"));
    }
    // Every cell must be there to be read before anything changes.
    let mut conjured = Vec::with_capacity(cells.len());
    for cell in cells {
        let loc = Location::new(at.grid, cell);
        let before = crate::world_verbs::element_at(sim, loc, layer)?;
        conjured.push(Conjured {
            at: loc,
            before,
            placed,
        });
    }
    for cell in &conjured {
        sim.world(|ctx| ctx.edit(cause).set_element(cell.at, layer, placed))
            .map_err(|error| Reason::from_world(&error))?;
    }
    let count = conjured.len();
    bind(
        sim,
        &terms,
        Bound::Edit {
            layer,
            cells: conjured,
        },
        cause,
    );
    Ok(number_out(count))
}

/// `bind_trait`: gives someone a trait bound to an ability.
fn bind_trait(sim: &mut Sim, call: &VerbCall<'_>, cause: Cause) -> Result<Value, Reason> {
    let to = entity_arg(sim, call, 0, "to")?;
    let def = crate::traits::TraitId::from_index(def_arg::<TraitDef>(sim, call, 1, "trait")?)
        .ok_or_else(|| bad_argument("trait", "a `trait` definition"))?;
    let terms = read_terms(sim, call, 2, OnEnd::Remove)?;
    check_room(sim, terms.number)?;
    let id = reserve(sim);
    let added = crate::traits::add_trait(sim, to, def, binding_giver(id), None, cause)?;
    bind_as(sim, id, &terms, Bound::Trait { on: to, def }, cause);
    Ok(Value::Int(i64::from(added.stacks)))
}

/// `bind_rule`: attaches a rule to someone, bound to an ability.
fn bind_rule(sim: &mut Sim, call: &VerbCall<'_>, cause: Cause) -> Result<Value, Reason> {
    let to = entity_arg(sim, call, 0, "to")?;
    let eoc = EocId::from_index(def_arg::<EocDef>(sim, call, 1, "eoc")?)
        .ok_or_else(|| bad_argument("eoc", "an `eoc` definition"))?;
    let attachable = sim
        .defs
        .try_registry::<EocDef>()
        .and_then(|registry| registry.try_get(eoc))
        .is_some_and(EocDef::is_attached);
    if !attachable {
        return Err(bad_argument(
            "eoc",
            "a rule that can be attached (one with `on`)",
        ));
    }
    let terms = read_terms(sim, call, 2, OnEnd::Remove)?;
    check_room(sim, terms.number)?;
    let handle = sim
        .exprs
        .kinds()
        .handle(Subject::Entity(to))
        .ok_or_else(|| bad_argument("to", "an entity that exists"))?;
    if !sim.rules.attach(handle, eoc) {
        return Ok(Value::Bool(false));
    }
    bind(sim, &terms, Bound::Rule { on: to, eoc }, cause);
    Ok(Value::Bool(true))
}

/// `dismiss`: ends an ability's bindings of an owner, or one by name.
fn dismiss(sim: &mut Sim, call: &VerbCall<'_>, cause: Cause) -> Result<Value, Reason> {
    let who = entity_arg(sim, call, 0, "who")?;
    let def = super::AbilityId::from_index(def_arg::<AbilityDef>(sim, call, 1, "ability")?)
        .ok_or_else(|| bad_argument("ability", "an `ability` definition"))?;
    let name = match arg(call, 2) {
        Value::Null => None,
        Value::Str(name) => Some(name.to_string()),
        _ => return Err(bad_argument("as", "a name")),
    };
    let how = match arg(call, 3) {
        Value::Null => None,
        value => Some(
            value
                .as_str()
                .and_then(OnEnd::parse)
                .ok_or_else(|| bad_term("how", "`remove`, `restore` or `release`"))?,
        ),
    };
    let Some(number) = sim
        .components
        .get::<Abilities>(who)
        .and_then(|abilities| abilities.of(def))
        .map(super::AbilityInst::number)
    else {
        return Ok(Value::Int(0));
    };
    let ids = of_instance(sim, number, |binding| {
        name.is_none() || binding.name == name
    });
    let mut ended = 0;
    for id in ids {
        if let Some(how) = how
            && let Some(binding) = sim.abilities.bindings.get_mut(&id)
        {
            binding.on_end = how;
        }
        if end(sim, id, "dismissed", cause) {
            ended += 1;
        }
    }
    Ok(number_out(ended))
}

/// Where a binding's thing is, for `dispel` by place.
fn where_is(sim: &Sim, binding: &Binding) -> Vec<Location> {
    match &binding.what {
        Bound::Entity { id } | Bound::Trait { on: id, .. } | Bound::Rule { on: id, .. } => {
            sim.entities.location(*id).into_iter().collect()
        }
        Bound::Area { id } => sim
            .cosmos
            .area(*id)
            .and_then(|area| Some(Location::new(area.grid, area.shape.centre()?)))
            .into_iter()
            .collect(),
        Bound::Edit { cells, .. } => cells.iter().map(|cell| cell.at).collect(),
    }
}

/// Whether a place is within a distance of another, in metres, a storey
/// counting three (02 §3.3).
fn within(sim: &Sim, a: Location, b: Location, radius: Real) -> bool {
    if a.grid != b.grid {
        return false;
    }
    let Ok(grid) = sim.cosmos.grid(a.grid) else {
        return false;
    };
    let offset = grid.topology().delta(a.cell, b.cell);
    Real::from_i64(clash_world::distance_sq_cells(offset)) <= radius * radius
}

/// `dispel`: ends bindings by their abilities' tags, by owner, by place.
fn dispel(sim: &mut Sim, call: &VerbCall<'_>, cause: Cause) -> Result<Value, Reason> {
    let tags: Option<Vec<String>> = match arg(call, 0) {
        Value::Null => None,
        Value::List(items) => Some(
            items
                .iter()
                .map(|item| item.as_str().map(str::to_owned))
                .collect::<Option<Vec<_>>>()
                .ok_or_else(|| bad_argument("tags", "a list of texts"))?,
        ),
        _ => return Err(bad_argument("tags", "a list of texts")),
    };
    let owner = match arg(call, 1) {
        Value::Null => None,
        _ => Some(entity_arg(sim, call, 1, "owner")?),
    };
    let at = match arg(call, 2) {
        Value::Null => None,
        _ => Some(loc_arg(sim, call, 2, "at")?),
    };
    let radius = match arg(call, 3) {
        Value::Null => Real::ZERO,
        value => value
            .as_number()
            .filter(|radius| radius.is_finite() && *radius >= Real::ZERO)
            .ok_or_else(|| bad_argument("radius", "a distance of zero or more"))?,
    };
    let defs = Arc::clone(&sim.defs);
    let ids: Vec<u64> = sim
        .abilities
        .bindings
        .values()
        .filter(|binding| owner.is_none_or(|owner| binding.owner == owner))
        .filter(|binding| {
            let Some(tags) = &tags else {
                return true;
            };
            inst(sim, binding.owner, binding.number)
                .and_then(|inst| def_of(&defs, inst.def()))
                .is_some_and(|def| def.tags.iter().any(|tag| tags.contains(tag)))
        })
        .filter(|binding| {
            at.is_none_or(|at| {
                where_is(sim, binding)
                    .into_iter()
                    .any(|place| within(sim, place, at, radius))
            })
        })
        .map(|binding| binding.id)
        .collect();
    let mut ended = 0;
    for id in ids {
        if end(sim, id, "dispelled", cause) {
            ended += 1;
        }
    }
    Ok(number_out(ended))
}

/// The verbs of bindings, for the table of abilities' verbs.
pub fn verbs() -> Vec<VerbDef> {
    vec![
        VerbDef {
            name: "summon",
            build: |registry| {
                spec(
                    registry,
                    "summon",
                    "Makes an entity from a template at a cell, bound to an ability (08 §7.3): it lasts as the binding's terms say and is removed when the binding ends (or left free, with `on_end` `release`). With `obey`, it takes the stances its summoner's mind holds toward others and holds none toward its summoner (09 §3.4). Returns the entity. Refused with `ability.no_binder`, `ability.too_many_bindings`, `ability.bad_binding`, and as `spawn` is.",
                    &[
                        ("template", "id<template>", false, "What it is made from."),
                        ("at", "loc", false, "The cell to put it in."),
                        (
                            "mind",
                            "string",
                            true,
                            "The `mind` it thinks by in place of its template's, by name.",
                        ),
                        (
                            "obey",
                            "bool",
                            true,
                            "Whether it takes its summoner's stances. Default true.",
                        ),
                    ],
                )
            },
            run: summon,
        },
        VerbDef {
            name: "conjure",
            build: |registry| {
                spec(
                    registry,
                    "conjure",
                    "Places an element in a cell, or in each cell of a straight line from `at` to `to`, through `WorldEdit`, bound to an ability and remembering what was there (08 §7.3). When the binding ends, a cell that is still what was conjured is put back as it was (`restore`, the default), emptied (`remove`), or left (`release`); a cell altered since is left alone. Returns how many cells it placed. Refused, changing nothing, for a cell that is not loaded, a state the layer has not, more than 64 cells, and as the other verbs that bind are.",
                    &[
                        ("at", "loc", false, "The cell, or the first of the line."),
                        ("to", "loc", true, "The last cell of the line."),
                        (
                            "layer",
                            "string",
                            true,
                            "`floor`, `block` or `fixture`. Default `block`.",
                        ),
                        (
                            "what",
                            "string",
                            false,
                            "What to place, as data writes a state: `def`, `def@material`, `def#state`.",
                        ),
                    ],
                )
            },
            run: conjure,
        },
        VerbDef {
            name: "bind_trait",
            build: |registry| {
                spec(
                    registry,
                    "bind_trait",
                    "Gives someone a trait bound to an ability (08 §7.3): a curse, a blessing, a mark. The trait has the binding as its giver, and goes when the binding ends (or stays, with `release`). Returns the stacks it has. Refused as `add_trait` is, and as the other verbs that bind are.",
                    &[
                        ("to", "entity", false, "Who is given it."),
                        ("trait", "id<trait>", false, "The trait."),
                    ],
                )
            },
            run: bind_trait,
        },
        VerbDef {
            name: "bind_rule",
            build: |registry| {
                spec(
                    registry,
                    "bind_rule",
                    "Attaches a rule to someone, bound to an ability (08 §7.3): it is detached when the binding ends (or stays, with `release`). Returns whether it was attached: false if it was already, and then nothing is bound. Refused for a rule that cannot be attached, and as the other verbs that bind are.",
                    &[
                        ("to", "entity", false, "Who it is attached to."),
                        ("eoc", "id<eoc>", false, "The rule: one with `on`."),
                    ],
                )
            },
            run: bind_rule,
        },
        VerbDef {
            name: "dismiss",
            build: |registry| {
                let mut spec = VerbSpec::new("dismiss").doc("Ends the bindings an entity's ability holds, or the one of a name (08 §7.3), each as its `on_end` says unless `how` says otherwise; `binding_ending` runs for each first. Returns how many ended.");
                for (arg, written, optional, doc) in [
                    ("who", "entity", false, "Whose ability."),
                    ("ability", "id<ability>", false, "The ability."),
                    (
                        "as",
                        "string",
                        true,
                        "Only the binding of this name. Left out, all of them.",
                    ),
                    (
                        "how",
                        "string",
                        true,
                        "`remove`, `restore` or `release`, in place of each binding's own `on_end`.",
                    ),
                ] {
                    let ty = registry.parse_type(written)?;
                    spec = if optional {
                        spec.optional_arg(arg, ty.nullable(), doc)
                    } else {
                        spec.arg(arg, ty, doc)
                    };
                }
                Some(spec)
            },
            run: dismiss,
        },
        VerbDef {
            name: "dispel",
            build: |registry| {
                let mut spec = VerbSpec::new("dispel").doc("Ends bindings (08 §7.3): those whose abilities have any of `tags`, whose owner is `owner`, and whose thing is within `radius` metres of `at`, as many of the three as are given; `binding_ending` runs for each first. Returns how many ended.");
                for (arg, written, doc) in [
                    (
                        "tags",
                        "list<string>",
                        "Only bindings of abilities with any of these tags.",
                    ),
                    (
                        "owner",
                        "entity",
                        "Only bindings of this entity's abilities.",
                    ),
                    (
                        "at",
                        "loc",
                        "Only bindings whose thing is near here: an entity, the one a trait or rule is on, an area's middle, a conjured cell.",
                    ),
                    (
                        "radius",
                        "real",
                        "How near `at`, in metres. Default 0: the cell itself.",
                    ),
                ] {
                    let ty = registry.parse_type(written)?;
                    spec = spec.optional_arg(arg, ty.nullable(), doc);
                }
                Some(spec)
            },
            run: dispel,
        },
    ]
}

/// The terms of `create_area`'s binding: `bound` and the five terms
/// after it, from `at`. `None` if it is not bound.
pub(crate) fn area_terms(
    sim: &Sim,
    call: &VerbCall<'_>,
    at: usize,
) -> Result<Option<Terms>, Reason> {
    let flag = match call.args.get(at).unwrap_or(&Value::Null) {
        Value::Null => None,
        Value::Bool(flag) => Some(*flag),
        _ => return Err(bad_term("bound", "true or false")),
    };
    let terms_given = (at + 1..at + 6)
        .any(|index| index != at + 1 && !matches!(call.args.get(index), None | Some(Value::Null)));
    match flag {
        Some(false) => Ok(None),
        Some(true) => read_terms(sim, call, at + 1, OnEnd::Remove).map(Some),
        None if terms_given => read_terms(sim, call, at + 1, OnEnd::Remove).map(Some),
        None => Ok(None),
    }
}

/// Binds an area `create_area` made, on its terms.
pub(crate) fn bind_area(sim: &mut Sim, terms: &Terms, area: AreaId, cause: Cause) {
    bind(sim, terms, Bound::Area { id: area }, cause);
}
