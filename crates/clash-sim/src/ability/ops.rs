//! Holding abilities (08 §3, §4, §6.1, §6.3): granting from sources,
//! losing them, revoking, dormancy, rank, parameters, what an instance
//! confers, and the seat.

use std::cell::Cell;
use std::sync::Arc;

use clash_core::{EntityId, Real};
use clash_expr::Value;
use clash_world::Cause;

use super::reactive::{self, Watcher};
use super::{
    ABILITY_ACTIVE, ABILITY_DORMANT, ABILITY_GRANTED, ABILITY_REVOKED, Abilities, AbilityDef,
    AbilityId, AbilityInst, AbilitySource, RANK_CHANGED, State, always_giver, always_source, codes,
    def_of, handle_value, name_of,
};
use crate::Sim;
use crate::bus::EventPayload;
use crate::event::{EventBody, SimEvent, Subject, Where};
use crate::host::Bound;
use crate::reason::Reason;
use crate::stats::{Modifier, Stats, TrackDef, TrackId};
use crate::traits::{TraitDef, TraitKey, Traits};

/// How deep the parameters of abilities may read each other while one
/// is worked out: a circle loading did not see gives null past it.
const MAX_PARAM_DEPTH: u8 = 16;

thread_local! {
    static PARAM_DEPTH: Cell<u8> = const { Cell::new(0) };
}

pub(super) fn not_registered() -> Reason {
    Reason::new(
        codes::NOT_REGISTERED,
        "the definitions were loaded without the `ability` type",
    )
}

pub(super) fn unknown(what: impl Into<String>) -> Reason {
    let what = what.into();
    Reason::new(codes::UNKNOWN, format!("there is no ability `{what}`")).with("ability", what)
}

pub(super) fn not_held(sim: &Sim, entity: EntityId, def: AbilityId) -> Reason {
    let name = name_of(&sim.defs, def);
    Reason::new(
        codes::NOT_HELD,
        format!("{entity} does not hold the ability `{name}`"),
    )
    .with("entity", entity.to_string())
    .with("ability", name)
}

fn abilities_mut(sim: &mut Sim, entity: EntityId) -> Option<&mut Abilities> {
    sim.components.get_mut::<Abilities>(entity)
}

pub(super) fn inst(sim: &Sim, entity: EntityId, number: u64) -> Option<&AbilityInst> {
    sim.components.get::<Abilities>(entity)?.by_number(number)
}

pub(super) fn inst_of(sim: &Sim, entity: EntityId, def: AbilityId) -> Option<&AbilityInst> {
    sim.components.get::<Abilities>(entity)?.of(def)
}

fn inst_mut(sim: &mut Sim, entity: EntityId, number: u64) -> Option<&mut AbilityInst> {
    abilities_mut(sim, entity)?.by_number_mut(number)
}

/// Raises one of the notifications of abilities about an owner.
pub(super) fn notify(sim: &mut Sim, entity: EntityId, payload: EventPayload, cause: Cause) {
    let payload = payload.subject("self", Subject::Entity(entity));
    sim.emit(Where::Entity(entity), cause, EventBody::Custom(payload));
}

// ---- Rank and parameters ---------------------------------------------------------------

/// The rank of an instance (08 §4.1): its track's level, or the rank
/// set directly.
pub fn rank(sim: &Sim, entity: EntityId, inst: &AbilityInst) -> u32 {
    let track = def_of(&sim.defs, inst.def()).and_then(|def| def.track);
    match track {
        Some(track) => sim
            .components
            .get::<Stats>(entity)
            .map_or(0, |stats| stats.track_level(track.id())),
        None => inst.own_rank(),
    }
}

/// Whether a parameter modifier names an ability: by id, or by a tag
/// the ability has.
fn names_ability(sim: &Sim, ability: Option<&str>, tag: Option<&str>, def: AbilityId) -> bool {
    if let Some(name) = ability {
        return super::lookup(&sim.defs, name) == Some(def);
    }
    if let Some(tag) = tag {
        return def_of(&sim.defs, def).is_some_and(|def| def.tags.iter().any(|have| have == tag));
    }
    false
}

/// The modifiers that apply to one parameter of one ability of an
/// entity: from its traits that are not dormant, once for each stack,
/// and from the entries of `always` its abilities apply.
fn param_modifiers(sim: &Sim, entity: EntityId, def: AbilityId, param: &str) -> Vec<(Real, Real)> {
    let mut out = Vec::new();
    if let (Some(traits), Some(registry)) = (
        sim.components.get::<Traits>(entity),
        sim.defs.try_registry::<TraitDef>(),
    ) {
        for trait_inst in traits.instances() {
            if trait_inst.is_dormant() {
                continue;
            }
            let Some(trait_def) = registry.try_get(trait_inst.def()) else {
                continue;
            };
            for modifier in &trait_def.ability_params {
                if modifier.param == param
                    && names_ability(
                        sim,
                        modifier.ability.as_deref(),
                        modifier.tag.as_deref(),
                        def,
                    )
                {
                    for _ in 0..trait_inst.stacks() {
                        out.push((modifier.add, modifier.mul));
                    }
                }
            }
        }
    }
    if let Some(abilities) = sim.components.get::<Abilities>(entity) {
        for other in abilities.instances() {
            if !other.confers() {
                continue;
            }
            let Some(other_def) = def_of(&sim.defs, other.def()) else {
                continue;
            };
            for entry in &other.applied {
                let Some(always) = other_def.always.0.get(usize::from(*entry)) else {
                    continue;
                };
                for modifier in &always.params {
                    if modifier.param == param
                        && names_ability(
                            sim,
                            modifier.ability.as_deref(),
                            modifier.tag.as_deref(),
                            def,
                        )
                    {
                        out.push((modifier.add, modifier.mul));
                    }
                }
            }
        }
    }
    out
}

/// A parameter of an instance (08 §4.2): its expression, read with the
/// owner as `self` and the instance as `ability`, then `(value + Σ add)
/// × Π mul` over the modifiers that apply. `None` for an instance that
/// does not exist or an ability with no such parameter; a value that
/// comes to null counts as zero.
pub fn param(sim: &Sim, entity: EntityId, number: u64, name: &str) -> Option<Real> {
    let inst = inst(sim, entity, number)?;
    let def = def_of(&sim.defs, inst.def())?;
    let expr = def.params.get(name)?;
    let depth = PARAM_DEPTH.with(Cell::get);
    if depth >= MAX_PARAM_DEPTH {
        return None;
    }
    PARAM_DEPTH.with(|cell| cell.set(depth + 1));
    let bound = Bound::new()
        .subject(sim.subject_value(Subject::Entity(entity)))
        .subject(handle_value(sim, number));
    let mut host = sim.host(bound);
    let base = expr.eval_on(&sim.defs, &mut host);
    PARAM_DEPTH.with(|cell| cell.set(depth));
    let base = base.filter(|value| value.is_finite()).unwrap_or(Real::ZERO);
    let modifiers = param_modifiers(sim, entity, inst.def(), name);
    let mut add = Real::ZERO;
    let mut mul = Real::ONE;
    for (plus, times) in modifiers {
        add += plus;
        mul *= times;
    }
    Some((base + add) * mul)
}

// ---- What an instance confers ---------------------------------------------------------

/// The entries of `always` an instance should apply now.
fn wanted_entries(sim: &Sim, entity: EntityId, inst: &AbilityInst) -> Vec<u16> {
    if !inst.confers() {
        return Vec::new();
    }
    let Some(def) = def_of(&sim.defs, inst.def()) else {
        return Vec::new();
    };
    let rank = rank(sim, entity, inst);
    def.always
        .0
        .iter()
        .enumerate()
        .filter(|(_, always)| always.at_rank.is_none_or(|at| rank >= at))
        .filter_map(|(index, _)| u16::try_from(index).ok())
        .collect()
}

/// Applies one entry of `always`: its traits and its modifiers.
fn apply_entry(sim: &mut Sim, entity: EntityId, inst: &AbilityInst, entry: u16, cause: Cause) {
    let tables = Arc::clone(&sim.ability_rt.tables);
    let traits = tables
        .always_traits
        .get(inst.def().index())
        .and_then(|entries| entries.get(usize::from(entry)))
        .cloned()
        .unwrap_or_default();
    for trait_id in traits {
        let _ = crate::traits::add_trait(
            sim,
            entity,
            trait_id,
            always_giver(inst.number(), entry),
            None,
            cause,
        );
    }
    let defs = Arc::clone(&sim.defs);
    let Some(always) =
        def_of(&defs, inst.def()).and_then(|def| def.always.0.get(usize::from(entry)))
    else {
        return;
    };
    if sim.components.get::<Stats>(entity).is_none() {
        return;
    }
    let source = always_source(inst.number(), entry);
    for modifier in &always.modifiers {
        let modifier = Modifier {
            attr: modifier.attr.id(),
            source,
            add: modifier.add,
            mul: modifier.mul,
        };
        if let Err(reason) = sim.add_modifier(entity, modifier, cause) {
            sim.fault(reason);
        }
    }
}

/// Takes back one entry of `always`.
fn take_back_entry(sim: &mut Sim, entity: EntityId, number: u64, entry: u16, cause: Cause) {
    crate::traits::withdraw(sim, entity, always_giver(number, entry), cause);
    if sim.components.get::<Stats>(entity).is_some() {
        let _ = sim.remove_stat_source(entity, always_source(number, entry), cause);
    }
}

/// Brings what an instance confers into line with whether it confers
/// and its rank: entries of `always` newly wanted are applied, those no
/// longer wanted taken back; then the rules attached because of
/// abilities are settled.
pub(super) fn settle(sim: &mut Sim, entity: EntityId, number: u64, cause: Cause) {
    let Some(current) = inst(sim, entity, number).cloned() else {
        settle_rules(sim, entity);
        return;
    };
    let wanted = wanted_entries(sim, entity, &current);
    for entry in &current.applied {
        if !wanted.contains(entry) {
            take_back_entry(sim, entity, number, *entry, cause);
        }
    }
    for entry in &wanted {
        if !current.applied.contains(entry) {
            apply_entry(sim, entity, &current, *entry, cause);
        }
    }
    if let Some(inst) = inst_mut(sim, entity, number) {
        inst.applied = wanted;
    }
    settle_rules(sim, entity);
}

/// Takes back everything an instance applied.
fn take_back_all(sim: &mut Sim, entity: EntityId, number: u64, cause: Cause) {
    let applied = inst(sim, entity, number)
        .map(|inst| inst.applied.clone())
        .unwrap_or_default();
    for entry in applied {
        take_back_entry(sim, entity, number, entry, cause);
    }
    if let Some(inst) = inst_mut(sim, entity, number) {
        inst.applied.clear();
    }
}

/// Brings the rules attached because of abilities into line with the
/// instances that confer: a rule not attached already is attached and
/// recorded; a recorded rule nothing wants is detached.
fn settle_rules(sim: &mut Sim, entity: EntityId) {
    let defs = Arc::clone(&sim.defs);
    let eocs = defs.try_registry::<crate::rule::EocDef>();
    let mut wanted: Vec<crate::rule::EocId> = Vec::new();
    if let Some(abilities) = sim.components.get::<Abilities>(entity) {
        for inst in abilities.instances().iter().filter(|inst| inst.confers()) {
            let Some(def) = def_of(&defs, inst.def()) else {
                continue;
            };
            for rule in def.rules.iter() {
                let id = rule.id();
                let attachable = eocs
                    .and_then(|eocs| eocs.try_get(id))
                    .is_some_and(crate::rule::EocDef::is_attached);
                if attachable && !wanted.contains(&id) {
                    wanted.push(id);
                }
            }
        }
    }
    wanted.sort_unstable();
    let Some(handle) = sim.exprs.kinds().handle(Subject::Entity(entity)) else {
        return;
    };
    let Some(abilities) = sim.components.get_mut::<Abilities>(entity) else {
        return;
    };
    let rules = &mut sim.rules;
    abilities.rules.retain(|rule| {
        let keep = wanted.binary_search(rule).is_ok();
        if !keep {
            rules.detach(handle, *rule);
        }
        keep
    });
    for rule in wanted {
        if let Err(at) = abilities.rules.binary_search(&rule)
            && rules.attach(handle, rule)
        {
            abilities.rules.insert(at, rule);
        }
    }
}

// ---- Granting and losing ------------------------------------------------------------------

/// Watches an instance's `while`, if it has one, and gives whether it
/// holds; true for one with none.
fn watch_while(sim: &mut Sim, entity: EntityId, number: u64) -> bool {
    reactive::watch(sim, Watcher::Ability { entity, number }).unwrap_or(true)
}

/// Gives an entity an ability from a source (08 §3.3).
///
/// - New to it, the ability gains an instance with a number no other
///   has had, at `rank` if given (and the ability has no track), else
///   its definition's. Its `while` is watched from now on; if it holds,
///   what the ability confers is applied. `ability_granted` follows.
/// - Held already, the source is added to it, and `rank`, if given,
///   sets the rank. One ability is one instance (invariant A8).
/// - Revoked and given again by a rule (or a template), it is held
///   again, with `ability_granted`. A trait that lists it does not undo
///   a revocation.
///
/// Returns the instance's number. Refused, changing nothing, for an
/// unknown entity and an id that is not an ability.
pub fn grant(
    sim: &mut Sim,
    entity: EntityId,
    def: AbilityId,
    source: AbilitySource,
    rank: Option<u32>,
    cause: Cause,
) -> Result<u64, Reason> {
    sim.known(entity)?;
    let defs = Arc::clone(&sim.defs);
    let registry = defs
        .try_registry::<AbilityDef>()
        .ok_or_else(not_registered)?;
    let ability = registry
        .try_get(def)
        .ok_or_else(|| unknown(def.index().to_string()))?;
    let name = registry.name(def).to_owned();
    if let Some(rank) = rank
        && rank > super::MAX_RANK
    {
        return Err(bad_rank(i64::from(rank)));
    }
    if let Some(existing) = inst_of(sim, entity, def).cloned() {
        let number = existing.number();
        let lifts = existing.is_revoked() && !matches!(source, AbilitySource::Trait(_));
        if let Some(inst) = inst_mut(sim, entity, number) {
            if !inst.sources.contains(&source) {
                inst.sources.push(source);
            }
            if lifts {
                inst.revoked = false;
            }
        }
        if let (Some(rank), None) = (rank, ability.track) {
            set_own_rank(sim, entity, number, rank, cause);
        }
        if lifts {
            let holds = watch_while(sim, entity, number);
            if let Some(inst) = inst_mut(sim, entity, number) {
                inst.state = if holds { State::Active } else { State::Dormant };
            }
            settle(sim, entity, number, cause);
            let rank = inst(sim, entity, number).map_or(0, |inst| super::rank(sim, entity, inst));
            notify(
                sim,
                entity,
                EventPayload::new(ABILITY_GRANTED)
                    .with("ability", name.as_str())
                    .with("source", source.name())
                    .with("rank", i64::from(rank)),
                cause,
            );
        }
        return Ok(number);
    }
    let number = sim.abilities.next;
    sim.abilities.next += 1;
    sim.abilities.owners.insert(number, entity);
    let own_rank = match ability.track {
        Some(_) => 0,
        None => rank.unwrap_or_else(|| ability.starting_rank()),
    };
    sim.components
        .store_mut::<Abilities>()
        .get_or_insert_with(entity, Abilities::default)
        .insts
        .push(AbilityInst {
            number,
            def,
            sources: vec![source],
            revoked: false,
            rank: own_rank,
            state: State::Active,
            modes: Vec::new(),
            cooldowns: Vec::new(),
            applied: Vec::new(),
        });
    let holds = watch_while(sim, entity, number);
    if let Some(inst) = inst_mut(sim, entity, number) {
        inst.state = if holds { State::Active } else { State::Dormant };
    }
    settle(sim, entity, number, cause);
    let rank = inst(sim, entity, number).map_or(0, |inst| super::rank(sim, entity, inst));
    notify(
        sim,
        entity,
        EventPayload::new(ABILITY_GRANTED)
            .with("ability", name.as_str())
            .with("source", source.name())
            .with("rank", i64::from(rank)),
        cause,
    );
    Ok(number)
}

/// Removes an instance entirely: its modes end, what it confers is
/// taken back, it is no longer watched, and it leaves the component and
/// the index.
fn remove_inst(sim: &mut Sim, entity: EntityId, number: u64, how: &str, cause: Cause) {
    super::modes::end_all(sim, entity, number, how, cause);
    take_back_all(sim, entity, number, cause);
    reactive::unwatch(sim, Watcher::Ability { entity, number });
    if let Some(abilities) = abilities_mut(sim, entity) {
        abilities.insts.retain(|inst| inst.number != number);
    }
    sim.abilities.owners.remove(&number);
    settle_rules(sim, entity);
}

/// Takes one source of an ability away (08 §3.3): what a trait that
/// listed it calls as it ends or goes dormant. With no source left, the
/// instance ends: its modes end (`lost`), what it conferred is taken
/// back, and `ability_revoked` is raised with `how` `lost` (unless it
/// had been revoked already, which was reported then). Returns whether
/// it had that source.
pub fn lose_source(
    sim: &mut Sim,
    entity: EntityId,
    def: AbilityId,
    source: AbilitySource,
    cause: Cause,
) -> bool {
    let Some(existing) = inst_of(sim, entity, def).cloned() else {
        return false;
    };
    if !existing.sources.contains(&source) {
        return false;
    }
    let number = existing.number();
    if let Some(inst) = inst_mut(sim, entity, number) {
        inst.sources.retain(|have| *have != source);
    }
    let left = inst(sim, entity, number).map_or(0, |inst| inst.sources.len());
    if left > 0 {
        return true;
    }
    remove_inst(sim, entity, number, "lost", cause);
    if !existing.is_revoked() {
        let name = name_of(&sim.defs, def);
        notify(
            sim,
            entity,
            EventPayload::new(ABILITY_REVOKED)
                .with("ability", name.as_str())
                .with("how", "lost"),
            cause,
        );
    }
    true
}

/// Takes an ability away whatever its sources (08 §6.3): its modes end,
/// what it conferred is taken back and `ability_revoked` is raised with
/// `how` `revoked`. A trait that lists it does not bring it back while
/// that trait lasts; a rule's grant does. Returns whether it was held.
///
/// The hook `ability_revoking` is the verb's to raise, before this.
pub fn revoke(
    sim: &mut Sim,
    entity: EntityId,
    def: AbilityId,
    cause: Cause,
) -> Result<bool, Reason> {
    sim.known(entity)?;
    let defs = Arc::clone(&sim.defs);
    let registry = defs
        .try_registry::<AbilityDef>()
        .ok_or_else(not_registered)?;
    if registry.try_get(def).is_none() {
        return Err(unknown(def.index().to_string()));
    }
    let Some(existing) = inst_of(sim, entity, def).cloned() else {
        return Ok(false);
    };
    if existing.is_revoked() {
        return Ok(false);
    }
    let number = existing.number();
    let trait_sources: Vec<AbilitySource> = existing
        .sources
        .iter()
        .copied()
        .filter(|source| matches!(source, AbilitySource::Trait(_)))
        .collect();
    if trait_sources.is_empty() {
        remove_inst(sim, entity, number, "revoked", cause);
    } else {
        super::modes::end_all(sim, entity, number, "revoked", cause);
        if let Some(inst) = inst_mut(sim, entity, number) {
            inst.revoked = true;
            inst.sources = trait_sources;
        }
        reactive::unwatch(sim, Watcher::Ability { entity, number });
        settle(sim, entity, number, cause);
    }
    let name = registry.name(def).to_owned();
    notify(
        sim,
        entity,
        EventPayload::new(ABILITY_REVOKED)
            .with("ability", name.as_str())
            .with("how", "revoked"),
        cause,
    );
    Ok(true)
}

/// Makes an instance dormant or active again (08 §6.1): what the
/// watcher of its `while` calls when the value changes. Dormant, its
/// modes end and it confers nothing (`ability_dormant`); active, it
/// confers again (`ability_active`) and its modes stay off.
pub(crate) fn set_dormant(
    sim: &mut Sim,
    entity: EntityId,
    number: u64,
    dormant: bool,
    cause: Cause,
) {
    let Some(current) = inst(sim, entity, number).cloned() else {
        return;
    };
    if current.is_revoked() {
        return;
    }
    let state = if dormant {
        State::Dormant
    } else {
        State::Active
    };
    if current.state() == state {
        return;
    }
    if dormant {
        super::modes::end_all(sim, entity, number, "dormant", cause);
    }
    if let Some(inst) = inst_mut(sim, entity, number) {
        inst.state = state;
    }
    settle(sim, entity, number, cause);
    let name = name_of(&sim.defs, current.def());
    let event = if dormant {
        ABILITY_DORMANT
    } else {
        ABILITY_ACTIVE
    };
    notify(
        sim,
        entity,
        EventPayload::new(event).with("ability", name.as_str()),
        cause,
    );
}

/// Gives an entity the abilities a template lists, by name as written,
/// with the template as their source: what making an entity from a
/// template calls (06 §10.3). A name that is not an ability gives
/// nothing; loading reports it.
pub fn grant_from_template(sim: &mut Sim, entity: EntityId, names: &[String], cause: Cause) {
    let defs = Arc::clone(&sim.defs);
    for name in names {
        if let Some(def) = super::lookup(&defs, name)
            && let Err(reason) = grant(sim, entity, def, AbilitySource::Template, None, cause)
        {
            sim.fault(reason);
        }
    }
}

/// Brings the abilities an entity holds from its traits into line with
/// its traits (08 §3.3): each trait instance that is not dormant is a
/// source of every ability its definition lists. What the kernel calls
/// whenever the entity's traits changed.
pub fn sync_trait_sources(sim: &mut Sim, entity: EntityId, cause: Cause) {
    let tables = Arc::clone(&sim.ability_rt.tables);
    let mut wanted: Vec<(AbilityId, TraitKey)> = Vec::new();
    if let Some(traits) = sim.components.get::<Traits>(entity) {
        for trait_inst in traits.instances() {
            if trait_inst.is_dormant() {
                continue;
            }
            if let Some(listed) = tables.trait_abilities.get(trait_inst.def().index()) {
                for ability in listed {
                    wanted.push((*ability, trait_inst.key()));
                }
            }
        }
    }
    wanted.sort_unstable();
    wanted.dedup();
    let mut have: Vec<(AbilityId, TraitKey)> = sim
        .components
        .get::<Abilities>(entity)
        .map(|abilities| {
            abilities
                .instances()
                .iter()
                .flat_map(|inst| {
                    inst.sources.iter().filter_map(move |source| match source {
                        AbilitySource::Trait(key) => Some((inst.def(), *key)),
                        AbilitySource::Template | AbilitySource::Rule => None,
                    })
                })
                .collect()
        })
        .unwrap_or_default();
    have.sort_unstable();
    for (def, key) in &have {
        if wanted.binary_search(&(*def, *key)).is_err() {
            lose_source(sim, entity, *def, AbilitySource::Trait(*key), cause);
        }
    }
    for (def, key) in wanted {
        if have.binary_search(&(def, key)).is_err()
            && let Err(reason) = grant(sim, entity, def, AbilitySource::Trait(key), None, cause)
        {
            sim.fault(reason);
        }
    }
}

// ---- Rank ----------------------------------------------------------------------------------

fn bad_rank(rank: i64) -> Reason {
    Reason::new(
        codes::BAD_RANK,
        format!(
            "a rank is a whole number from 0 to {}, not {rank}",
            super::MAX_RANK
        ),
    )
    .with("rank", rank)
}

/// After a rank changed: what applies from a rank is brought into line,
/// and modes above it end.
fn rank_settled(sim: &mut Sim, entity: EntityId, number: u64, cause: Cause) {
    super::modes::end_above_rank(sim, entity, number, cause);
    settle(sim, entity, number, cause);
}

/// Sets the rank of an instance with no track, raising `rank_changed`
/// if it changed.
fn set_own_rank(sim: &mut Sim, entity: EntityId, number: u64, rank: u32, cause: Cause) -> u32 {
    let Some(current) = inst(sim, entity, number).cloned() else {
        return 0;
    };
    let old = current.own_rank();
    if old == rank {
        return old;
    }
    if let Some(inst) = inst_mut(sim, entity, number) {
        inst.rank = rank;
    }
    rank_settled(sim, entity, number, cause);
    let name = name_of(&sim.defs, current.def());
    notify(
        sim,
        entity,
        EventPayload::new(RANK_CHANGED)
            .with("ability", name.as_str())
            .with("old", i64::from(old))
            .with("new", i64::from(rank)),
        cause,
    );
    old
}

/// Sets the rank of an ability with no track (08 §4.1): `rank_changed`
/// follows if it changed, and what applies from a rank is brought into
/// line. Returns the rank it had. Refused for an ability not held, one
/// whose rank is its track's level, and a rank out of bounds.
pub fn set_rank(
    sim: &mut Sim,
    entity: EntityId,
    def: AbilityId,
    rank: i64,
    cause: Cause,
) -> Result<u32, Reason> {
    sim.known(entity)?;
    let defs = Arc::clone(&sim.defs);
    let ability = defs
        .try_registry::<AbilityDef>()
        .ok_or_else(not_registered)?
        .try_get(def)
        .ok_or_else(|| unknown(def.index().to_string()))?;
    let Ok(rank) = u32::try_from(rank) else {
        return Err(bad_rank(rank));
    };
    if rank > super::MAX_RANK {
        return Err(bad_rank(i64::from(rank)));
    }
    let Some(number) = inst_of(sim, entity, def)
        .filter(|inst| inst.is_held())
        .map(AbilityInst::number)
    else {
        return Err(not_held(sim, entity, def));
    };
    if ability.track.is_some() {
        let name = name_of(&defs, def);
        return Err(Reason::new(
            codes::RANK_FROM_TRACK,
            format!("the rank of `{name}` is its track's level: advance the track"),
        )
        .with("ability", name));
    }
    Ok(set_own_rank(sim, entity, number, rank, cause))
}

/// Answers `track_advanced`: each ability of the entity ranked by that
/// track has changed rank.
pub(super) fn track_advanced(sim: &mut Sim, entity: EntityId, event: &SimEvent) {
    let Some(track) = event.body.field("track").and_then(|field| match field {
        crate::bus::FieldValue::Plain(clash_core::Value::Id { index, .. }) => {
            TrackId::from_index(index as usize)
        }
        crate::bus::FieldValue::Plain(clash_core::Value::Str(name)) => {
            sim.defs.lookup::<TrackDef>(&name)
        }
        _ => None,
    }) else {
        return;
    };
    let level = event
        .body
        .field("level")
        .and_then(|field| field.as_real())
        .map_or(0, |level| level.to_i64());
    let defs = Arc::clone(&sim.defs);
    let ranked: Vec<(u64, AbilityId)> = sim
        .components
        .get::<Abilities>(entity)
        .map(|abilities| {
            abilities
                .instances()
                .iter()
                .filter(|inst| inst.is_held())
                .filter(|inst| {
                    def_of(&defs, inst.def())
                        .and_then(|def| def.track)
                        .is_some_and(|have| have.id() == track)
                })
                .map(|inst| (inst.number(), inst.def()))
                .collect()
        })
        .unwrap_or_default();
    for (number, def) in ranked {
        rank_settled(sim, entity, number, event.cause);
        let name = name_of(&defs, def);
        notify(
            sim,
            entity,
            EventPayload::new(RANK_CHANGED)
                .with("ability", name.as_str())
                .with("old", (level - 1).max(0))
                .with("new", level),
            event.cause,
        );
    }
}

// ---- Seat ----------------------------------------------------------------------------------

/// Answers `mind_transferred` (08 §3.4): the mind-seated abilities a
/// mind brings go with it to the other body, both ways for a swap.
/// Those that move are the ones held only from rules and templates,
/// which belong to no thing on the body; what a body confers through
/// its traits stays with it.
pub(super) fn mind_transferred(sim: &mut Sim, from: EntityId, event: &SimEvent) {
    let Some(to) = event.subject("target").and_then(|subject| subject.entity()) else {
        return;
    };
    let swap = matches!(
        event.body.field("swap"),
        Some(crate::bus::FieldValue::Plain(clash_core::Value::Bool(true)))
    );
    let going = held_mind_seated(sim, from);
    let coming = if swap {
        held_mind_seated(sim, to)
    } else {
        Vec::new()
    };
    move_numbers(sim, from, to, &going, event.cause);
    move_numbers(sim, to, from, &coming, event.cause);
}

/// The numbers of the mind-seated instances an entity holds only from
/// rules and templates.
fn held_mind_seated(sim: &Sim, entity: EntityId) -> Vec<u64> {
    sim.components
        .get::<Abilities>(entity)
        .map(|abilities| {
            abilities
                .instances()
                .iter()
                .filter(|inst| inst.is_held())
                .filter(|inst| {
                    def_of(&sim.defs, inst.def())
                        .is_some_and(|def| def.seat == crate::stats::Seat::Mind)
                })
                .filter(|inst| {
                    inst.sources
                        .iter()
                        .all(|source| !matches!(source, AbilitySource::Trait(_)))
                })
                .map(AbilityInst::number)
                .collect()
        })
        .unwrap_or_default()
}

/// Moves instances, by number, from one entity to another: their modes
/// end (`moved`), what they conferred is taken back and `ability_revoked`
/// is raised with `how` `moved`; at the other they keep their number,
/// rank and cooldowns, are watched afresh, confer and are announced with
/// `ability_granted`. An ability the other holds already gains their
/// sources instead.
fn move_numbers(sim: &mut Sim, from: EntityId, to: EntityId, numbers: &[u64], cause: Cause) {
    let defs = Arc::clone(&sim.defs);
    for number in numbers {
        let Some(moved) = inst(sim, from, *number).cloned() else {
            continue;
        };
        super::modes::end_all(sim, from, *number, "moved", cause);
        take_back_all(sim, from, *number, cause);
        reactive::unwatch(
            sim,
            Watcher::Ability {
                entity: from,
                number: *number,
            },
        );
        if let Some(abilities) = abilities_mut(sim, from) {
            abilities.insts.retain(|inst| inst.number != *number);
        }
        settle_rules(sim, from);
        let name = name_of(&defs, moved.def());
        notify(
            sim,
            from,
            EventPayload::new(ABILITY_REVOKED)
                .with("ability", name.as_str())
                .with("how", "moved"),
            cause,
        );
        if inst_of(sim, to, moved.def()).is_some() {
            sim.abilities.owners.remove(number);
            for source in moved.sources.clone() {
                let _ = grant(sim, to, moved.def(), source, None, cause);
            }
            continue;
        }
        sim.abilities.owners.insert(*number, to);
        let mut arriving = moved.clone();
        arriving.modes.clear();
        arriving.applied.clear();
        sim.components
            .store_mut::<Abilities>()
            .get_or_insert_with(to, Abilities::default)
            .insts
            .push(arriving);
        let holds = watch_while(sim, to, *number);
        if let Some(inst) = inst_mut(sim, to, *number) {
            inst.state = if holds { State::Active } else { State::Dormant };
        }
        settle(sim, to, *number, cause);
        let rank = inst(sim, to, *number).map_or(0, |inst| super::rank(sim, to, inst));
        let source = moved.sources.first().map_or("rule", |source| source.name());
        notify(
            sim,
            to,
            EventPayload::new(ABILITY_GRANTED)
                .with("ability", name.as_str())
                .with("source", source)
                .with("rank", i64::from(rank)),
            cause,
        );
    }
}

/// Forgets what was kept for an entity that has been removed: its
/// instances leave the index and its expressions are no longer watched.
/// Its component went with it.
pub(super) fn forget_entity(sim: &mut Sim, entity: EntityId) {
    sim.abilities.owners.retain(|_, owner| *owner != entity);
    reactive::forget_entity(sim, entity);
}

/// The value a parameter property gives: the parameter, or null.
pub(super) fn param_value(sim: &Sim, entity: EntityId, number: u64, name: &str) -> Value {
    param(sim, entity, number, name).map_or(Value::Null, Value::Real)
}
