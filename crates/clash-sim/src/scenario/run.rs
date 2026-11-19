//! A game being played: beginning it, keeping its objectives and
//! missions current, ending it, and the verbs rules call (13 §4, §5,
//! §9).

use std::collections::BTreeSet;
use std::sync::Arc;

use clash_core::{EntityId, Tick, Value as Stored};
use clash_data::{Cond, Defs};
use clash_expr::{Type, Value, VerbSpec};
use clash_time::{ClockId, Wake, WakeDomain, WakeId};
use clash_world::{Cause, CauseKind, CellPos, Location, MoveProfile, MoveRules, RealmId, SiteId};
use serde::{Deserialize, Serialize};

use super::def::{
    CharacterEntry, MissionDef, ObjectiveDef, PlaceEntry, ScenarioDef, SettingDef, namespace_of,
    qualified, stored_value,
};
use super::state::{Game, Mission, MissionState, Objective, ObjectiveState};
use super::world::scenario_named;
use super::{
    CHECKPOINT_MADE, DEFAULT_MAX_ROUNDS, MAX_ROUNDS_SETTING, MISSION_DONE, MISSION_EXPIRED,
    MISSION_FAILED, MISSION_STARTED, OBJECTIVE_CHANGED, PARTY_GONE, SCENARIO_ENDED,
    SCENARIO_ENDING, SCENARIO_STARTED, SETTING_CHANGED, why,
};
use crate::advance::AppRequest;
use crate::bus::{EventPayload, FieldValue, Handler};
use crate::entity::{Control, Place};
use crate::event::{Subject, Where};
use crate::host::Bound;
use crate::kernel_verbs::bad_argument;
use crate::mind::VitalState;
use crate::reason::Reason;
use crate::rule::{Effects, RuleGroupDef, VerbCall};
use crate::sim::Sim;
use crate::template::CharacterSpec;
use crate::verb::VerbDef;

/// The cause of what a scenario does by itself: beginning, objectives
/// settling, endings. What a rule does is caused as that rule is.
pub const SCENARIO_CAUSE: Cause = Cause::new(CauseKind::Eoc);

/// The bit of a wake-up's `arg` that marks it as a scenario's: an
/// objective's or a mission's time running out. Pools, bodies, traits
/// and abilities' modes use the four bits above it.
pub const SCENARIO_WAKE_TAG: u64 = 1 << 59;

/// The events that may mean the party is gone.
const PARTY_EVENTS: [&str; 3] = ["entity_died", "entity_removed", "control_changed"];

/// What `Command::BeginScenario` carries: the answers the player gave
/// (13 §12).
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub struct BeginScenario {
    /// The scenario, by qualified id.
    pub scenario: String,
    /// The party: the characters the player begins as, already made and
    /// out of the world. One in this build.
    pub party: Vec<EntityId>,
    /// Which of the scenario's places to begin at, from 0.
    pub place: u32,
    /// The answers to the scenario's `options`: settings by key, each
    /// with its value.
    pub options: Vec<(String, Stored)>,
}

/// Whether a wake-up is a scenario's.
pub fn is_scenario_wake(wake: &Wake) -> bool {
    wake.domain == WakeDomain::Timer && wake.arg == SCENARIO_WAKE_TAG
}

/// The handler of the `Timer` domain: a scenario's wake-up has the
/// objectives looked at again at the safe point that follows; every
/// other goes on to the traits' handler, as before.
pub fn on_timer(sim: &mut Sim, id: WakeId, wake: Wake) {
    if is_scenario_wake(&wake) {
        sim.scenario.dirty = true;
        return;
    }
    crate::traits::on_timer(sim, id, wake);
}

// ---- Setting up ------------------------------------------------------------------

/// Registers what keeps a game current: a handler for each event an
/// objective or an ending watches, and for those that may mean the
/// party is gone; and one for each ending written with `on`. Handlers
/// are not state: this runs on every simulation, new or loaded.
pub(crate) fn bind(sim: &mut Sim) {
    let tables = Arc::clone(&sim.scenario_rt);
    let mut names: BTreeSet<String> = tables.events.clone();
    names.extend(PARTY_EVENTS.iter().map(|name| (*name).to_owned()));
    if tables.every {
        names.extend(
            sim.event_types()
                .filter(|ty| !ty.hook)
                .map(|ty| ty.name.clone()),
        );
    }
    for name in names {
        let hook = sim
            .event_types()
            .find(|ty| ty.name == name)
            .map(|ty| ty.hook);
        if hook != Some(false) {
            continue;
        }
        let noted = name.clone();
        let _ = sim.on(
            &name,
            Handler::new(move |sim, _| {
                if sim.scenario.playing() {
                    sim.scenario.stale.insert(noted.clone());
                }
            }),
        );
    }
    let defs = Arc::clone(&sim.defs);
    let Some(scenarios) = defs.try_registry::<ScenarioDef>() else {
        return;
    };
    for (id, def) in scenarios.iter() {
        let scenario = scenarios.name(id).to_owned();
        for end in &def.ends {
            let Some(on) = &end.on else {
                continue;
            };
            let (owner, outcome) = (scenario.clone(), end.outcome.clone());
            let mut handler = Handler::new(move |sim, _| {
                let ours = sim
                    .scenario
                    .game
                    .as_ref()
                    .is_some_and(|game| game.scenario == owner && game.ended.is_none());
                if ours {
                    sim.scenario.pending_ends.push(outcome.clone());
                }
            });
            for (field, value) in &end.filter.0 {
                if let Some(value) = field_value(&value.value) {
                    handler = handler.filter(field.clone(), value);
                }
            }
            let _ = sim.on(&on.value, handler);
        }
    }
}

/// A constant of data as an event's field holds it.
fn field_value(value: &serde_json::Value) -> Option<FieldValue> {
    match stored_value(value)? {
        Stored::Bool(flag) => Some(FieldValue::from(flag)),
        Stored::Int(int) => Some(FieldValue::from(int)),
        Stored::Real(real) => Some(FieldValue::from(real)),
        Stored::Str(text) => Some(FieldValue::text(text)),
        _ => None,
    }
}

// ---- Places --------------------------------------------------------------------------

/// The highest cell of a column a walker can stand in, looking down from
/// the top of the grid.
fn standing_cell(sim: &Sim, grid: clash_world::GridId, x: i32, y: i32) -> Option<Location> {
    let topology = sim.cosmos.topology(grid).ok()?;
    let walker = MoveProfile::walker();
    (topology.z_min..=topology.z_max).rev().find_map(|z| {
        let loc = Location::new(grid, CellPos::new(x, y, z));
        let here = sim.cosmos.neighbours(loc).ok()?.here;
        walker.can_occupy(&here).then_some(loc)
    })
}

/// The most regions of a realm surveyed so that a site can be chosen:
/// a larger realm has only those near where sessions began surveyed.
pub const MAX_SURVEYED_REGIONS: u32 = 64;

/// Surveys every region of a realm, if it has no more than
/// [`MAX_SURVEYED_REGIONS`], so that its sites can be chosen from.
fn survey(sim: &mut Sim, realm: RealmId) {
    let Some(def) = sim.cosmos.realm(realm).map(|realm| realm.def) else {
        return;
    };
    let Some([width, height]) = sim
        .cosmos
        .defs()
        .realms()
        .get(def)
        .map(|kind| kind.size_regions)
    else {
        return;
    };
    if width.saturating_mul(height) > MAX_SURVEYED_REGIONS {
        return;
    }
    for y in 0..height {
        for x in 0..width {
            let region = clash_world::RegionPos {
                x: x as i32,
                y: y as i32,
            };
            let _ = sim.cosmos.survey_region(&sim.generators, realm, region);
        }
    }
}

/// The sites of a realm a place's query answers, in id order.
fn sites_for(sim: &Sim, realm: RealmId, place: &PlaceEntry) -> Vec<SiteId> {
    let Some(query) = &place.site else {
        return Vec::new();
    };
    let names = sim.defs.try_registry::<clash_gen::region::SiteDef>();
    sim.cosmos
        .sites()
        .filter(|site| site.realm == realm)
        .filter(|site| query.tag.as_ref().is_none_or(|tag| site.has_tag(tag)))
        .filter(|site| query.name.as_ref().is_none_or(|name| &site.name == name))
        .filter(|site| {
            query.def.as_ref().is_none_or(|wanted| {
                names.is_some_and(|names| {
                    let name = names.name(site.def);
                    name == wanted || name.rsplit(':').next() == Some(wanted.as_str())
                })
            })
        })
        .map(|site| site.id)
        .collect()
}

fn bad_place(index: u32, message: impl Into<String>) -> Reason {
    Reason::new(why::BAD_PLACE, message).with("place", i64::from(index))
}

/// Where a place is: the cell to start a session at, and whether a
/// walker's footing must still be found in its column (`[x, y]`, a
/// site's anchor).
fn place_cell(
    sim: &Sim,
    realm: RealmId,
    place: &PlaceEntry,
    site: Option<SiteId>,
    index: u32,
) -> Result<(Location, bool), Reason> {
    let grid = sim
        .cosmos
        .realm(realm)
        .map(|realm| realm.base_grid)
        .ok_or_else(|| bad_place(index, "the realm is gone"))?;
    let top = sim
        .cosmos
        .topology(grid)
        .map(|topology| topology.z_max)
        .map_err(|error| Reason::from_world(&error))?;
    match (&place.cell, site) {
        (Some(cell), _) => match cell.as_slice() {
            [x, y] => Ok((Location::new(grid, CellPos::new(*x, *y, top)), true)),
            [x, y, z] => Ok((Location::new(grid, CellPos::new(*x, *y, *z)), false)),
            _ => Err(bad_place(
                index,
                "a place's cell is `[x, y]` or `[x, y, z]`",
            )),
        },
        (None, Some(site)) => {
            let anchor = sim
                .cosmos
                .site(site)
                .map(|site| site.anchor)
                .ok_or_else(|| bad_place(index, "the site chosen is gone"))?;
            Ok((
                Location::new(grid, CellPos::new(anchor.x, anchor.y, top)),
                true,
            ))
        }
        (None, None) => Err(bad_place(
            index,
            "no site answers the place's query among the realm's surveyed sites",
        )),
    }
}

/// How far round a place's column, in cells, footing is looked for when
/// the column itself has none (a site's anchor under a statue).
pub const FOOTING_REACH: i32 = 8;

/// The nearest column to `(x, y)` with somewhere a walker can stand,
/// looking ring by ring out to [`FOOTING_REACH`], each ring row by row:
/// the highest such cell of it.
fn footing_near(sim: &Sim, grid: clash_world::GridId, x: i32, y: i32) -> Option<Location> {
    for ring in 0..=FOOTING_REACH {
        for dy in -ring..=ring {
            for dx in -ring..=ring {
                if dx.abs().max(dy.abs()) != ring {
                    continue;
                }
                if let Some(found) = standing_cell(sim, grid, x + dx, y + dy) {
                    return Some(found);
                }
            }
        }
    }
    None
}

/// Puts a character at a place: a session there, which loads the
/// tracts round it, and then, for a place that names no storey, where a
/// walker can stand in its column or the nearest one that has footing.
fn put_at(
    sim: &mut Sim,
    who: EntityId,
    at: Location,
    footing: bool,
    index: u32,
) -> Result<(), Reason> {
    sim.check_session(who, at)?;
    sim.start_session(who, at, SCENARIO_CAUSE)?;
    if footing {
        let stand = footing_near(sim, at.grid, at.cell.x, at.cell.y).ok_or_else(|| {
            bad_place(
                index,
                format!(
                    "there is nothing to stand on at {}, {} or within {FOOTING_REACH} cells of it",
                    at.cell.x, at.cell.y
                ),
            )
        })?;
        sim.relocate(who, Place::at(stand), SCENARIO_CAUSE)?;
    }
    Ok(())
}

/// Gives a character senses if its templates gave it none: those its
/// entry names, else every sense the loaded mods define, in order of
/// qualified id (15 §15).
fn give_senses(sim: &mut Sim, who: EntityId, entry: &CharacterEntry, namespace: &str) {
    if sim.components.senses.contains(who) {
        return;
    }
    let defs = Arc::clone(&sim.defs);
    let Some(senses) = defs.try_registry::<crate::perception::SenseDef>() else {
        return;
    };
    let names: Vec<String> = if entry.senses.is_empty() {
        let mut all = senses.names().to_vec();
        all.sort();
        all
    } else {
        entry
            .senses
            .iter()
            .map(|sense| qualified(&sense.value, namespace))
            .collect()
    };
    for name in names {
        if let Some(id) = senses.lookup(&name) {
            let _ = crate::perception::attach_sense(sim, who, id);
        }
    }
}

// ---- Beginning -----------------------------------------------------------------------

/// A setting's definition, if one is loaded under the key.
fn setting_def<'a>(defs: &'a Defs, key: &str) -> Option<&'a SettingDef> {
    let registry = defs.try_registry::<SettingDef>()?;
    registry.lookup(key).map(|id| registry.get(id))
}

/// A value as JSON, to be checked against a setting's definition.
fn as_json(value: &Stored) -> serde_json::Value {
    match value {
        Stored::Bool(flag) => serde_json::Value::Bool(*flag),
        Stored::Int(int) => serde_json::Value::from(*int),
        Stored::Real(real) => serde_json::Value::from(real.get()),
        Stored::Str(text) => serde_json::Value::String(text.clone()),
        _ => serde_json::Value::Null,
    }
}

/// Checks a value against the setting of its key, if one is defined.
fn check_setting(defs: &Defs, key: &str, value: &Stored) -> Result<(), Reason> {
    match setting_def(defs, key) {
        Some(def) => def.check(&as_json(value)).map_err(|why| {
            Reason::new(why::BAD_SETTING, format!("`{key}`: {why}")).with("key", key)
        }),
        None => Ok(()),
    }
}

/// Sets a setting and raises `setting_changed` if that changed it.
fn put_setting(sim: &mut Sim, key: &str, value: Stored, cause: Cause) {
    let new = Value::from(value.clone());
    let old = sim.set_setting(key, value);
    let old = old.map_or(Value::Null, Value::from);
    if old == new {
        return;
    }
    let payload = EventPayload::new(SETTING_CHANGED)
        .with("key", key)
        .with("old", plain(&old))
        .with("new", plain(&new));
    let _ = sim.raise(Where::Global, cause, payload);
}

/// A plain value as an event's field.
fn plain(value: &Value) -> FieldValue {
    match value {
        Value::Bool(flag) => FieldValue::from(*flag),
        Value::Int(int) => FieldValue::from(*int),
        Value::Real(real) => FieldValue::from(*real),
        Value::Str(text) => FieldValue::text(text.as_ref()),
        _ => FieldValue::Null,
    }
}

/// The `Setting` command (04 §10): refused for a setting the scenario
/// being played locks, and for a value its definition does not take.
pub(crate) fn setting_command(sim: &mut Sim, key: &str, value: Stored) -> Result<(), Reason> {
    if let Some(game) = &sim.scenario.game {
        let defs = Arc::clone(&sim.defs);
        if let Ok(def) = scenario_named(&defs, &game.scenario)
            && def.locked.iter().any(|locked| locked == key)
        {
            return Err(Reason::new(
                why::LOCKED,
                format!("`{key}` cannot be changed in this scenario"),
            )
            .with("key", key)
            .with("scenario", game.scenario.as_str()));
        }
    }
    check_setting(&Arc::clone(&sim.defs), key, &value)?;
    put_setting(sim, key, value, Cause::DEBUG);
    Ok(())
}

/// The realm of the simulation made from the scenario's realm
/// definition: the first, by id.
fn scenario_realm(sim: &Sim, def: &ScenarioDef, scenario: &str) -> Result<RealmId, Reason> {
    let name = def.realm_id(namespace_of(scenario)).unwrap_or_default();
    let wanted = sim.cosmos.defs().realms().lookup(&name);
    sim.cosmos
        .realms()
        .find(|realm| Some(realm.def) == wanted)
        .map(|realm| realm.id)
        .ok_or_else(|| {
            Reason::new(
                why::BAD_WORLD,
                format!("the simulation has no realm `{name}`: make its world with `make_world`"),
            )
            .with("realm", name.as_str())
        })
}

/// The objectives of a list as a game begins them: shown unless hidden
/// or waiting for others.
fn opening(objectives: &[ObjectiveDef]) -> Vec<Objective> {
    objectives
        .iter()
        .map(|objective| Objective {
            id: objective.id.value.clone(),
            state: if objective.hidden || !objective.after.is_empty() {
                ObjectiveState::Hidden
            } else {
                ObjectiveState::Active
            },
            expires: None,
        })
        .collect()
}

/// Begins a game (13 §4, §12): `Command::BeginScenario`. See the
/// [module](super) documentation for what it does, in order.
///
/// Refused, changing nothing, while a game is being played, for a
/// scenario that is not loaded, a place it does not have, a party of
/// entities that do not exist, a realm the simulation was not made
/// with, an answer to a setting it does not ask about or a value a
/// setting does not take. A site that cannot be found, or a session
/// that cannot be started, is refused after the realm's regions have
/// been surveyed: that is the one trace it leaves.
pub fn begin(sim: &mut Sim, begin: BeginScenario) -> Result<(), Reason> {
    if let Some(game) = &sim.scenario.game {
        return Err(Reason::new(why::BEGUN, "a game is being played already")
            .with("scenario", game.scenario.as_str()));
    }
    let defs = Arc::clone(&sim.defs);
    let def = scenario_named(&defs, &begin.scenario)?;
    let scenario = defs
        .try_registry::<ScenarioDef>()
        .and_then(|registry| {
            registry
                .lookup(&begin.scenario)
                .map(|id| registry.name(id).to_owned())
        })
        .unwrap_or_else(|| begin.scenario.clone());
    let namespace = namespace_of(&scenario).to_owned();
    let index = begin.place;
    let place = def.start.at.get(index as usize).ok_or_else(|| {
        bad_place(
            index,
            format!("the scenario has no place {index} to begin at"),
        )
    })?;
    let Some(&lead) = begin.party.first() else {
        return Err(Reason::new(why::BAD_PARTY, "the party has no one in it"));
    };
    for who in &begin.party {
        if !sim.entities.contains(*who) {
            return Err(
                Reason::new(why::BAD_PARTY, format!("there is no entity {who}"))
                    .with("entity", who.to_string()),
            );
        }
    }
    let realm = scenario_realm(sim, def, &scenario)?;
    let asked: Vec<String> = def
        .options
        .iter()
        .map(|option| qualified(&option.value, &namespace))
        .collect();
    for (key, value) in &begin.options {
        if !asked.contains(key) {
            return Err(Reason::new(
                why::NOT_AN_OPTION,
                format!("the scenario does not ask about `{key}`"),
            )
            .with("key", key.as_str()));
        }
        check_setting(&defs, key, value)?;
    }

    // Where: a site is chosen among those of the realm, from the rule
    // stream, once the realm has been surveyed.
    let site = if place.site.is_some() {
        survey(sim, realm);
        let found = sites_for(sim, realm, place);
        if found.is_empty() {
            return Err(bad_place(
                index,
                "no site of the realm answers the place's query",
            ));
        }
        let drawn = sim.rng.stream(crate::RULE_STREAM).below(found.len() as u64) as usize;
        Some(found[drawn])
    } else {
        None
    };
    let (at, footing) = place_cell(sim, realm, place, site, index)?;
    sim.check_session(lead, at)?;

    // From here on the game is under way. The party is placed first:
    // a session that cannot be started leaves only the survey and the
    // senses behind.
    if let Some(entry) = def.character() {
        for who in &begin.party {
            give_senses(sim, *who, entry, &namespace);
        }
    }
    put_at(sim, lead, at, footing, index)?;
    let cause = SCENARIO_CAUSE;
    for (key, value) in &def.settings.0 {
        if let Some(value) = stored_value(&value.value) {
            put_setting(sim, key, value, cause);
        }
    }
    for (key, value) in &begin.options {
        put_setting(sim, key, value.clone(), cause);
    }
    switch_groups(sim, &defs, &scenario);
    for who in &begin.party {
        sim.set_control(*who, Control::Direct, cause)?;
    }
    let now = sim.now();
    sim.scenario.game = Some(Game {
        scenario: scenario.clone(),
        began: now,
        party: begin.party.clone(),
        fallen: Vec::new(),
        place: index,
        realm: Some(realm),
        site,
        at: sim.truth().location(lead),
        objectives: opening(&def.objectives),
        missions: Vec::new(),
        checkpoints: Vec::new(),
        gone: false,
        ended: None,
    });
    for (position, objective) in def.objectives.iter().enumerate() {
        if game_objective(sim, None, position).is_some_and(|o| o.state == ObjectiveState::Active) {
            shown(sim, None, position, objective);
        }
    }
    let (subjects, mut locals) = truth_bound(sim, lead, &[]);
    sim.run_effects(&def.start.effects, &subjects, &mut locals, None, cause);
    let payload = EventPayload::new(SCENARIO_STARTED)
        .subject("self", Subject::Entity(lead))
        .with("scenario", scenario.as_str())
        .with("place", i64::from(index));
    let _ = sim.raise(Where::Entity(lead), cause, payload);
    sim.scenario.dirty = true;
    Ok(())
}

/// Switches the rule groups scenarios name: on for those this one
/// names, off for every other a scenario names.
fn switch_groups(sim: &mut Sim, defs: &Defs, scenario: &str) {
    let (Some(scenarios), Some(groups)) = (
        defs.try_registry::<ScenarioDef>(),
        defs.try_registry::<RuleGroupDef>(),
    ) else {
        return;
    };
    let mut named: Vec<(crate::rule::RuleGroupId, bool)> = Vec::new();
    for (id, def) in scenarios.iter() {
        let name = scenarios.name(id);
        let ours = name == scenario;
        for group in &def.rules {
            let Some(group) = groups.lookup(&qualified(&group.value, namespace_of(name))) else {
                continue;
            };
            match named.iter_mut().find(|(have, _)| *have == group) {
                Some((_, on)) => *on |= ours,
                None => named.push((group, ours)),
            }
        }
    }
    for (group, on) in named {
        sim.rules.set_group(group, Some(on));
    }
}

// ---- What expressions are bound to ----------------------------------------------------

/// The party as a list of entity handles.
fn party_value(sim: &Sim) -> Value {
    let party = sim
        .scenario
        .game
        .as_ref()
        .map(|game| game.party.clone())
        .unwrap_or_default();
    Value::list(
        party
            .into_iter()
            .map(|who| sim.subject_value(Subject::Entity(who))),
    )
}

fn start_cell_value(sim: &Sim) -> Value {
    sim.scenario
        .game
        .as_ref()
        .and_then(|game| game.at)
        .map_or(Value::Null, |at| sim.subject_value(Subject::Loc(at)))
}

fn start_site_value(sim: &Sim) -> Value {
    sim.scenario
        .game
        .as_ref()
        .and_then(|game| game.site)
        .map_or(Value::Null, |site| sim.subject_value(Subject::Site(site)))
}

/// The subjects and locals of a truth scope (`self`, `taker`, `party`,
/// `start_site`, `start_cell`, `global`, then the parameters).
pub(crate) fn truth_bound(
    sim: &Sim,
    taker: EntityId,
    params: &[Stored],
) -> (Vec<Value>, Vec<Value>) {
    let who = sim.subject_value(Subject::Entity(taker));
    let subjects = vec![
        who.clone(),
        who,
        party_value(sim),
        start_site_value(sim),
        start_cell_value(sim),
        sim.subject_value(Subject::Global),
    ];
    (subjects, params.iter().cloned().map(Value::from).collect())
}

/// The subjects and locals of a mind scope (`self`, `taker`,
/// `start_site`, `start_cell`, `global`, then the parameters).
pub(crate) fn mind_bound(sim: &Sim, taker: EntityId, params: &[Stored]) -> Bound {
    let who = sim.subject_value(Subject::Entity(taker));
    Bound {
        subjects: vec![
            who.clone(),
            who,
            start_site_value(sim),
            start_cell_value(sim),
            sim.subject_value(Subject::Global),
        ],
        locals: params.iter().cloned().map(Value::from).collect(),
        fields: Vec::new(),
    }
}

/// Whether a condition holds against truth, for a taker.
fn holds(sim: &Sim, cond: &Cond, taker: EntityId, params: &[Stored]) -> bool {
    let (subjects, locals) = truth_bound(sim, taker, params);
    let mut host = sim.host(Bound {
        subjects,
        locals,
        fields: Vec::new(),
    });
    cond.holds_on(&sim.defs, &mut host)
}

// ---- Objectives --------------------------------------------------------------------------

/// Who an objective of the scenario's own is for: the character the
/// player plays now, the last of the party.
fn lead(game: &Game) -> Option<EntityId> {
    game.party.last().copied()
}

/// An objective of the game: one of the scenario's (`mission` none) or
/// of a mission by number.
fn game_objective(sim: &Sim, mission: Option<u32>, index: usize) -> Option<&Objective> {
    let game = sim.scenario.game.as_ref()?;
    match mission {
        None => game.objectives.get(index),
        Some(serial) => game.mission(serial)?.objectives.get(index),
    }
}

fn game_objective_mut(sim: &mut Sim, mission: Option<u32>, index: usize) -> Option<&mut Objective> {
    let game = sim.scenario.game.as_mut()?;
    match mission {
        None => game.objectives.get_mut(index),
        Some(serial) => game
            .missions
            .iter_mut()
            .find(|m| m.serial == serial)?
            .objectives
            .get_mut(index),
    }
}

/// The definitions of a list of objectives, the taker and the
/// parameters: of the scenario's own, or of a mission.
struct ListOf {
    defs: Arc<Defs>,
    scenario: Option<String>,
    mission: Option<String>,
    taker: EntityId,
    params: Vec<Stored>,
}

impl ListOf {
    fn of(sim: &Sim, mission: Option<u32>) -> Option<ListOf> {
        let game = sim.scenario.game.as_ref()?;
        let defs = Arc::clone(&sim.defs);
        match mission {
            None => Some(ListOf {
                defs,
                scenario: Some(game.scenario.clone()),
                mission: None,
                taker: lead(game)?,
                params: Vec::new(),
            }),
            Some(serial) => {
                let found = game.mission(serial)?;
                Some(ListOf {
                    defs,
                    scenario: None,
                    mission: Some(found.def.clone()),
                    taker: found.taker,
                    params: found.params.clone(),
                })
            }
        }
    }

    fn objectives(&self) -> &[ObjectiveDef] {
        if let Some(scenario) = &self.scenario {
            return scenario_named(&self.defs, scenario).map_or(&[], |def| &def.objectives);
        }
        self.mission
            .as_ref()
            .and_then(|mission| mission_named(&self.defs, mission))
            .map_or(&[], |def| &def.objectives)
    }
}

/// A mission definition by qualified id.
fn mission_named<'a>(defs: &'a Defs, mission: &str) -> Option<&'a MissionDef> {
    let registry = defs.try_registry::<MissionDef>()?;
    registry.lookup(mission).map(|id| registry.get(id))
}

/// What an objective's conditions say now, ignoring time: `failed`
/// first, then `done`; one with children and no `done` of its own is
/// done when they all are. Active if neither holds.
fn evaluate(sim: &Sim, list: &ListOf, mission: Option<u32>, index: usize) -> ObjectiveState {
    let objectives = list.objectives();
    let Some(def) = objectives.get(index) else {
        return ObjectiveState::Active;
    };
    if def
        .failed
        .as_ref()
        .is_some_and(|cond| holds(sim, cond, list.taker, &list.params))
    {
        return ObjectiveState::Failed;
    }
    match &def.done {
        Some(cond) => {
            if holds(sim, cond, list.taker, &list.params) {
                return ObjectiveState::Done;
            }
        }
        None => {
            let children: Vec<usize> = objectives
                .iter()
                .enumerate()
                .filter(|(_, child)| child.parent.as_deref() == Some(def.id.value.as_str()))
                .map(|(position, _)| position)
                .collect();
            let all_done = !children.is_empty()
                && children.iter().all(|child| {
                    game_objective(sim, mission, *child)
                        .is_some_and(|o| o.state == ObjectiveState::Done)
                });
            if all_done {
                return ObjectiveState::Done;
            }
        }
    }
    ObjectiveState::Active
}

/// What a fresh evaluation of an objective's conditions gives now: for
/// checking invariant Z4. `mission` is the mission's number, or `None`
/// for one of the scenario's own. `None` if there is no such objective.
pub fn fresh_objective_state(sim: &Sim, mission: Option<u32>, id: &str) -> Option<ObjectiveState> {
    let list = ListOf::of(sim, mission)?;
    let index = list
        .objectives()
        .iter()
        .position(|objective| objective.id.value == id)?;
    Some(evaluate(sim, &list, mission, index))
}

/// Where an objective of the game stands: `mission` as for
/// [`fresh_objective_state`].
pub fn objective_state(sim: &Sim, mission: Option<u32>, id: &str) -> Option<ObjectiveState> {
    let game = sim.scenario.game.as_ref()?;
    let list = match mission {
        None => &game.objectives,
        Some(serial) => &game.mission(serial)?.objectives,
    };
    list.iter()
        .find(|objective| objective.id == id)
        .map(|objective| objective.state)
}

/// An objective has just been shown: its expiry is set and
/// `objective_changed` raised with `active`.
fn shown(sim: &mut Sim, mission: Option<u32>, index: usize, def: &ObjectiveDef) {
    if let Some(time) = def.expires {
        let at = sim.now() + time.dur();
        if let Some(objective) = game_objective_mut(sim, mission, index) {
            objective.expires = Some(at);
        }
        let _ = sim.schedule_on(
            ClockId::UNIVERSAL,
            time.dur(),
            Wake::new(WakeDomain::Timer, 0, SCENARIO_WAKE_TAG),
        );
    }
    let taker = ListOf::of(sim, mission).map(|list| list.taker);
    raise_objective(sim, taker, &def.id.value, ObjectiveState::Active, mission);
}

fn raise_objective(
    sim: &mut Sim,
    taker: Option<EntityId>,
    id: &str,
    state: ObjectiveState,
    mission: Option<u32>,
) {
    let Some(taker) = taker else {
        return;
    };
    let payload = EventPayload::new(OBJECTIVE_CHANGED)
        .subject("taker", Subject::Entity(taker))
        .with("id", id)
        .with("state", state.name())
        .with("mission", i64::from(mission.unwrap_or(0)));
    let _ = sim.raise(Where::Entity(taker), SCENARIO_CAUSE, payload);
}

/// Settles an objective as done or failed: its state, its effects and
/// `objective_changed`. Does nothing to one that is not active.
fn settle_objective(sim: &mut Sim, mission: Option<u32>, index: usize, state: ObjectiveState) {
    let Some(list) = ListOf::of(sim, mission) else {
        return;
    };
    match game_objective_mut(sim, mission, index) {
        Some(objective) if objective.state == ObjectiveState::Active => objective.state = state,
        _ => return,
    }
    let Some(def) = list.objectives().get(index) else {
        return;
    };
    let effects: &Effects = if state == ObjectiveState::Done {
        &def.on_done
    } else {
        &def.on_failed
    };
    let (subjects, mut locals) = truth_bound(sim, list.taker, &list.params);
    sim.run_effects(effects, &subjects, &mut locals, None, SCENARIO_CAUSE);
    raise_objective(sim, Some(list.taker), &def.id.value, state, mission);
    sim.scenario.dirty = true;
}

/// Shows the hidden objectives of a list whose `after` are all done.
fn reveal(sim: &mut Sim, mission: Option<u32>) {
    let Some(list) = ListOf::of(sim, mission) else {
        return;
    };
    let objectives = list.objectives();
    for (index, def) in objectives.iter().enumerate() {
        let waiting = game_objective(sim, mission, index)
            .is_some_and(|objective| objective.state == ObjectiveState::Hidden);
        if !waiting || def.after.is_empty() {
            continue;
        }
        let ready = def.after.iter().all(|before| {
            objectives
                .iter()
                .position(|other| &other.id.value == before)
                .and_then(|position| game_objective(sim, mission, position))
                .is_some_and(|objective| objective.state == ObjectiveState::Done)
        });
        if ready {
            if let Some(objective) = game_objective_mut(sim, mission, index) {
                objective.state = ObjectiveState::Active;
            }
            shown(sim, mission, index, def);
            sim.scenario.dirty = true;
        }
    }
}

/// Looks at the active objectives of a list again: those whose watch
/// was hit, those whose time ran out, and, when `all`, every one.
fn look_at(
    sim: &mut Sim,
    mission: Option<u32>,
    watches: &[super::Watch],
    stale: &BTreeSet<String>,
    all: bool,
) {
    let Some(list) = ListOf::of(sim, mission) else {
        return;
    };
    let now = sim.now();
    for index in 0..list.objectives().len() {
        let Some(objective) = game_objective(sim, mission, index) else {
            continue;
        };
        if objective.state != ObjectiveState::Active {
            continue;
        }
        let expired = objective.expires.is_some_and(|at| at <= now);
        let hit = all || watches.get(index).is_some_and(|watch| watch.hit(stale));
        let state = if expired {
            ObjectiveState::Failed
        } else if hit {
            evaluate(sim, &list, mission, index)
        } else {
            continue;
        };
        if state.settled() {
            settle_objective(sim, mission, index, state);
        }
    }
    reveal(sim, mission);
}

// ---- Missions ----------------------------------------------------------------------------

/// Settles a mission: its state, its effects and its event.
fn settle_mission(sim: &mut Sim, serial: u32, state: MissionState) {
    let Some(game) = sim.scenario.game.as_mut() else {
        return;
    };
    let Some(mission) = game.missions.iter_mut().find(|m| m.serial == serial) else {
        return;
    };
    if mission.state != MissionState::Active {
        return;
    }
    mission.state = state;
    let (name, taker, params) = (mission.def.clone(), mission.taker, mission.params.clone());
    let defs = Arc::clone(&sim.defs);
    let (event, effects) = match (mission_named(&defs, &name), state) {
        (Some(def), MissionState::Done) => (MISSION_DONE, Some(&def.on_done)),
        (Some(def), MissionState::Failed) => (MISSION_FAILED, Some(&def.on_failed)),
        (Some(def), MissionState::Expired) => (MISSION_EXPIRED, Some(&def.on_expired)),
        (None, MissionState::Done) => (MISSION_DONE, None),
        (None, MissionState::Failed) => (MISSION_FAILED, None),
        (None, _) => (MISSION_EXPIRED, None),
        (Some(_), MissionState::Active) => return,
    };
    if let Some(effects) = effects {
        let (subjects, mut locals) = truth_bound(sim, taker, &params);
        sim.run_effects(effects, &subjects, &mut locals, None, SCENARIO_CAUSE);
    }
    raise_mission(sim, event, taker, &name, serial);
    sim.scenario.dirty = true;
}

fn raise_mission(sim: &mut Sim, event: &str, taker: EntityId, def: &str, serial: u32) {
    let payload = EventPayload::new(event)
        .subject("taker", Subject::Entity(taker))
        .with("mission", def)
        .with("serial", i64::from(serial));
    let _ = sim.raise(Where::Entity(taker), SCENARIO_CAUSE, payload);
}

/// Looks at every active mission again.
fn look_at_missions(sim: &mut Sim, stale: &BTreeSet<String>, all: bool) {
    let tables = Arc::clone(&sim.scenario_rt);
    let missions: Vec<(u32, String)> = sim
        .scenario
        .game
        .as_ref()
        .map(|game| {
            game.missions
                .iter()
                .filter(|m| m.state == MissionState::Active)
                .map(|m| (m.serial, m.def.clone()))
                .collect()
        })
        .unwrap_or_default();
    let defs = Arc::clone(&sim.defs);
    for (serial, name) in missions {
        let Some((watches, failed_watch)) = tables.missions.get(&name) else {
            continue;
        };
        let expired = sim
            .scenario
            .game
            .as_ref()
            .and_then(|game| game.mission(serial))
            .and_then(|mission| mission.expires)
            .is_some_and(|at| at <= sim.now());
        if expired {
            settle_mission(sim, serial, MissionState::Expired);
            continue;
        }
        look_at(sim, Some(serial), watches, stale, all);
        let Some(mission) = sim
            .scenario
            .game
            .as_ref()
            .and_then(|game| game.mission(serial))
        else {
            continue;
        };
        let (taker, params) = (mission.taker, mission.params.clone());
        let any_failed = mission
            .objectives
            .iter()
            .any(|o| o.state == ObjectiveState::Failed);
        let all_done = mission
            .objectives
            .iter()
            .all(|o| o.state == ObjectiveState::Done);
        let failed = any_failed
            || ((all || failed_watch.hit(stale))
                && mission_named(&defs, &name)
                    .and_then(|def| def.failed.as_ref())
                    .is_some_and(|cond| holds(sim, cond, taker, &params)));
        if failed {
            settle_mission(sim, serial, MissionState::Failed);
        } else if all_done {
            settle_mission(sim, serial, MissionState::Done);
        }
    }
}

/// Starts a mission (13 §5.2) for a taker, with its parameters by name:
/// the mission is recorded, its objectives that wait for nothing are
/// shown and `mission_started` raised. Gives its number.
pub(crate) fn start_mission(
    sim: &mut Sim,
    name: &str,
    taker: EntityId,
    with: &[(String, Value)],
) -> Result<u32, Reason> {
    let defs = Arc::clone(&sim.defs);
    let registry = defs.try_registry::<MissionDef>();
    let Some((qualified_name, def)) = registry.and_then(|registry| {
        let id = registry.lookup(name).or_else(|| {
            registry
                .names()
                .iter()
                .filter(|q| q.rsplit(':').next() == Some(name))
                .min()
                .and_then(|q| registry.lookup(q))
        })?;
        Some((registry.name(id).to_owned(), registry.get(id)))
    }) else {
        return Err(
            Reason::new(why::NO_MISSION, format!("there is no mission `{name}`"))
                .with("mission", name),
        );
    };
    let bad = |param: &str, message: String| {
        Reason::new(why::BAD_PARAMS, message)
            .with("mission", qualified_name.as_str())
            .with("param", param)
    };
    for (given, _) in with {
        if !def.params.0.iter().any(|(param, _)| param == given) {
            return Err(bad(
                given,
                format!("the mission has no parameter `{given}`"),
            ));
        }
    }
    let mut params = Vec::new();
    for (param, _) in &def.params.0 {
        let value = with
            .iter()
            .find(|(given, _)| given == param)
            .map(|(_, value)| value.to_stored())
            .ok_or_else(|| bad(param, format!("the mission needs `{param}`")))?;
        params.push(value);
    }
    let Some(game) = sim.scenario.game.as_mut() else {
        return Err(no_game());
    };
    if let Some(outcome) = &game.ended {
        return Err(ended(outcome));
    }
    let serial = game.missions.last().map_or(1, |m| m.serial + 1);
    let now = sim.timeline.now();
    let expires = def.expires.map(|time| now + time.dur());
    game.missions.push(Mission {
        serial,
        def: qualified_name.clone(),
        taker,
        params,
        state: MissionState::Active,
        started: now,
        expires,
        objectives: opening(&def.objectives),
    });
    if let Some(time) = def.expires {
        let _ = sim.schedule_on(
            ClockId::UNIVERSAL,
            time.dur(),
            Wake::new(WakeDomain::Timer, u64::from(serial), SCENARIO_WAKE_TAG),
        );
    }
    raise_mission(sim, MISSION_STARTED, taker, &qualified_name, serial);
    for (index, objective) in def.objectives.iter().enumerate() {
        if game_objective(sim, Some(serial), index)
            .is_some_and(|o| o.state == ObjectiveState::Active)
        {
            shown(sim, Some(serial), index, objective);
        }
    }
    sim.scenario.dirty = true;
    Ok(serial)
}

// ---- The party and endings -----------------------------------------------------------

/// Whether a character of the party is still the player's to play: it
/// exists, is directly controlled and is not dead.
fn able(sim: &Sim, who: EntityId) -> bool {
    sim.entities
        .get(who)
        .is_some_and(|entity| entity.control() == Control::Direct)
        && sim.vital(who) != Some(VitalState::Dead)
}

fn no_game() -> Reason {
    Reason::new(why::NO_GAME, "no game is being played")
}

fn ended(outcome: &str) -> Reason {
    Reason::new(why::ENDED, format!("the game has ended, {outcome}")).with("as", outcome)
}

/// Ends the game (13 §9.4): raises `scenario_ending`, whose handlers may
/// change the outcome or cancel; then records the outcome, raises
/// `scenario_ended` and asks the application to end it. Refused while
/// no game is being played or once it has ended; a cancelled ending is
/// refused with the handler's reason.
pub(crate) fn end_scenario(
    sim: &mut Sim,
    outcome: &str,
    reason: &str,
    cause: Cause,
) -> Result<(), Reason> {
    match &sim.scenario.game {
        None => return Err(no_game()),
        Some(game) => {
            if let Some(done) = &game.ended {
                return Err(ended(done));
            }
        }
    }
    let raised = sim.raise(
        Where::Global,
        cause,
        EventPayload::new(SCENARIO_ENDING).with("as", outcome),
    )?;
    if let Some(cancelled) = raised.cancelled {
        return Err(cancelled);
    }
    let outcome = raised
        .payload
        .field("as")
        .and_then(FieldValue::plain)
        .and_then(clash_core::Value::as_str)
        .map_or_else(|| outcome.to_owned(), str::to_owned);
    if let Some(game) = sim.scenario.game.as_mut() {
        game.ended = Some(outcome.clone());
    }
    let _ = sim.raise(
        Where::Global,
        cause,
        EventPayload::new(SCENARIO_ENDED)
            .with("as", outcome.as_str())
            .with("reason", reason),
    );
    sim.request_app(AppRequest::EndScenario { outcome });
    Ok(())
}

/// The party is gone: `party_gone`, the scenario's `do`, and what it
/// says follows (13 §9.3).
fn party_gone(sim: &mut Sim, def: &ScenarioDef, scenario: &str) {
    let Some(game) = sim.scenario.game.as_mut() else {
        return;
    };
    game.gone = true;
    let last = lead(game);
    let gone = &def.saving.on_party_gone;
    if let Some(last) = last {
        let payload = EventPayload::new(PARTY_GONE)
            .subject("self", Subject::Entity(last))
            .with("then", gone.then.as_str());
        let _ = sim.raise(Where::Entity(last), SCENARIO_CAUSE, payload);
        let (subjects, mut locals) = truth_bound(sim, last, &[]);
        sim.run_effects(&gone.effects, &subjects, &mut locals, None, SCENARIO_CAUSE);
    }
    let outcome = gone.outcome.clone();
    match gone.then.as_str() {
        "restore" => {
            let last_checkpoint = sim
                .scenario
                .game
                .as_ref()
                .and_then(|game| game.checkpoints.last().cloned());
            match last_checkpoint {
                Some(name) => sim.request_app(AppRequest::RestoreCheckpoint { name }),
                None => {
                    let _ = end_scenario(
                        sim,
                        &outcome,
                        "there is no checkpoint to go back to",
                        SCENARIO_CAUSE,
                    );
                }
            }
        }
        "continue" => {
            let ways: Vec<String> = if gone.by.is_empty() {
                super::def::CARRY_ON_WAYS
                    .iter()
                    .map(|w| (*w).to_owned())
                    .collect()
            } else {
                gone.by.clone()
            };
            let mut taken = None;
            for way in &ways {
                taken = match way.as_str() {
                    "own" => own_character(sim),
                    "create" => new_character(sim, def, scenario).ok(),
                    _ => None,
                };
                if taken.is_some() {
                    break;
                }
            }
            match taken {
                Some(who) => take_control(sim, who, SCENARIO_CAUSE),
                None => {
                    let _ = end_scenario(
                        sim,
                        &outcome,
                        "there is no one to carry on as",
                        SCENARIO_CAUSE,
                    );
                }
            }
        }
        _ => {
            let _ = end_scenario(sim, &outcome, "", SCENARIO_CAUSE);
        }
    }
}

/// Another of the player's own: a character of the party that is alive,
/// whoever decides for it now.
fn own_character(sim: &Sim) -> Option<EntityId> {
    let game = sim.scenario.game.as_ref()?;
    game.party.iter().copied().find(|who| {
        sim.entities.contains(*who)
            && sim.vital(*who) != Some(VitalState::Dead)
            && !game.fallen.contains(who)
    })
}

/// A new character from the scenario's character template, every choice
/// at its default, given senses and put at the place the party began at.
fn new_character(sim: &mut Sim, def: &ScenarioDef, scenario: &str) -> Result<EntityId, Reason> {
    let namespace = namespace_of(scenario).to_owned();
    let template = def.template_id(&namespace).ok_or_else(|| {
        Reason::new(
            why::BAD_PARTY,
            "the scenario names no template to make a character from",
        )
    })?;
    let game = sim.scenario.game.clone().ok_or_else(no_game)?;
    let realm = game.realm.ok_or_else(no_game)?;
    let place = def
        .start
        .at
        .get(game.place as usize)
        .ok_or_else(|| bad_place(game.place, "the place the party began at is gone"))?;
    let (at, footing) = place_cell(sim, realm, place, game.site, game.place)?;
    let spawned = sim.spawn_template(
        &CharacterSpec::of([template]),
        Place::Stored,
        SCENARIO_CAUSE,
    )?;
    let who = spawned.entity;
    if let Some(entry) = def.character() {
        give_senses(sim, who, entry, &namespace);
    }
    put_at(sim, who, at, footing, game.place)?;
    Ok(who)
}

/// Gives the player a character to play (`take_control`): direct
/// control, a place in the party, and the request to the application to
/// show it. The characters the player played before that can no longer
/// act are counted as fallen.
pub(crate) fn take_control(sim: &mut Sim, who: EntityId, cause: Cause) {
    if sim.set_control(who, Control::Direct, cause).is_err() {
        return;
    }
    let fallen: Vec<EntityId> = sim
        .scenario
        .game
        .as_ref()
        .map(|game| {
            game.party
                .iter()
                .copied()
                .filter(|member| *member != who && !able(sim, *member))
                .collect()
        })
        .unwrap_or_default();
    if let Some(game) = sim.scenario.game.as_mut() {
        game.party
            .retain(|member| !fallen.contains(member) && *member != who);
        game.fallen.extend(fallen);
        game.party.push(who);
        game.gone = false;
    }
    sim.request_app(AppRequest::TakeControl { entity: who });
    sim.scenario.dirty = true;
}

// ---- Settling --------------------------------------------------------------------------

/// Brings the game up to date at a safe point (03 §4.3): endings asked
/// for by events, the party, objectives whose watched events were
/// delivered or whose time ran out, missions, and endings by condition.
/// Runs in rounds while what it did makes more to look at, at most
/// `rules.max_reactive_rounds`; what is left is looked at at the next
/// safe point. Called by the kernel after every pump of a safe point.
pub fn settle(sim: &mut Sim) {
    if sim.scenario.settling {
        return;
    }
    if !sim.scenario.playing() {
        sim.scenario.stale.clear();
        sim.scenario.dirty = false;
        sim.scenario.pending_ends.clear();
        return;
    }
    let rounds = sim
        .setting_int(MAX_ROUNDS_SETTING, DEFAULT_MAX_ROUNDS)
        .clamp(1, 64);
    sim.scenario.settling = true;
    for _ in 0..rounds {
        let stale = std::mem::take(&mut sim.scenario.stale);
        let dirty = std::mem::take(&mut sim.scenario.dirty);
        let ends = std::mem::take(&mut sim.scenario.pending_ends);
        if !dirty && stale.is_empty() && ends.is_empty() {
            break;
        }
        step(sim, &stale, dirty, &ends);
        sim.pump();
        if !sim.scenario.playing() {
            sim.scenario.stale.clear();
            sim.scenario.pending_ends.clear();
            break;
        }
    }
    sim.scenario.settling = false;
}

/// One round of [`settle`].
fn step(sim: &mut Sim, stale: &BTreeSet<String>, dirty: bool, ends: &[String]) {
    let Some(game) = sim.scenario.game.clone() else {
        return;
    };
    let defs = Arc::clone(&sim.defs);
    let Ok(def) = scenario_named(&defs, &game.scenario) else {
        return;
    };
    if let Some(outcome) = ends.first() {
        let _ = end_scenario(sim, outcome, "", SCENARIO_CAUSE);
        return;
    }
    let party_news = dirty || PARTY_EVENTS.iter().any(|name| stale.contains(*name));
    if !game.gone && party_news && !game.party.iter().any(|who| able(sim, *who)) {
        party_gone(sim, def, &game.scenario);
        if !sim.scenario.playing() {
            return;
        }
    }
    let tables = Arc::clone(&sim.scenario_rt);
    let empty = (Vec::new(), Vec::new());
    let (objective_watches, end_watches) = tables.scenarios.get(&game.scenario).unwrap_or(&empty);
    look_at(sim, None, objective_watches, stale, dirty);
    look_at_missions(sim, stale, dirty);
    for (index, end) in def.ends.iter().enumerate() {
        let Some(condition) = &end.condition else {
            continue;
        };
        let hit = dirty || end_watches.get(index).is_some_and(|watch| watch.hit(stale));
        let taker = sim.scenario.game.as_ref().and_then(lead);
        if hit
            && let Some(taker) = taker
            && holds(sim, condition, taker, &[])
        {
            let _ = end_scenario(sim, &end.outcome, "", SCENARIO_CAUSE);
            return;
        }
    }
}

// ---- Verbs ----------------------------------------------------------------------------

/// The objective an `id` and an optional mission number name, as an
/// index into its list.
fn objective_named(sim: &Sim, id: &str, mission: Option<u32>) -> Result<usize, Reason> {
    let missing =
        || Reason::new(why::NO_OBJECTIVE, format!("there is no objective `{id}`")).with("id", id);
    let game = sim.scenario.game.as_ref().ok_or_else(no_game)?;
    let list = match mission {
        None => &game.objectives,
        Some(serial) => &game.mission(serial).ok_or_else(missing)?.objectives,
    };
    list.iter().position(|o| o.id == id).ok_or_else(missing)
}

/// The mission number argument: null for none.
fn mission_arg(call: &VerbCall<'_>, at: usize) -> Option<u32> {
    call.args
        .get(at)
        .and_then(Value::as_int)
        .and_then(|n| u32::try_from(n).ok())
        .filter(|n| *n > 0)
}

fn text_of(call: &VerbCall<'_>, at: usize, verb: &str, name: &str) -> Result<String, Reason> {
    call.args
        .get(at)
        .and_then(Value::as_str)
        .map(str::to_owned)
        .ok_or_else(|| bad_argument(verb, name, format!("`{name}` must be a text")))
}

fn playing(sim: &Sim) -> Result<(), Reason> {
    match &sim.scenario.game {
        None => Err(no_game()),
        Some(game) => game
            .ended
            .as_ref()
            .map_or(Ok(()), |outcome| Err(ended(outcome))),
    }
}

fn complete_objective(sim: &mut Sim, call: &VerbCall<'_>, _cause: Cause) -> Result<Value, Reason> {
    playing(sim)?;
    let id = text_of(call, 0, "complete_objective", "id")?;
    let mission = mission_arg(call, 1);
    let index = objective_named(sim, &id, mission)?;
    settle_objective(sim, mission, index, ObjectiveState::Done);
    Ok(Value::Null)
}

fn fail_objective(sim: &mut Sim, call: &VerbCall<'_>, _cause: Cause) -> Result<Value, Reason> {
    playing(sim)?;
    let id = text_of(call, 0, "fail_objective", "id")?;
    let mission = mission_arg(call, 1);
    let index = objective_named(sim, &id, mission)?;
    settle_objective(sim, mission, index, ObjectiveState::Failed);
    Ok(Value::Null)
}

fn add_objective(sim: &mut Sim, call: &VerbCall<'_>, _cause: Cause) -> Result<Value, Reason> {
    playing(sim)?;
    let id = text_of(call, 0, "add_objective", "id")?;
    let mission = mission_arg(call, 1);
    let index = objective_named(sim, &id, mission)?;
    let hidden = game_objective(sim, mission, index)
        .is_some_and(|objective| objective.state == ObjectiveState::Hidden);
    if hidden {
        if let Some(objective) = game_objective_mut(sim, mission, index) {
            objective.state = ObjectiveState::Active;
        }
        let list = ListOf::of(sim, mission);
        if let Some(def) = list.as_ref().and_then(|list| list.objectives().get(index)) {
            shown(sim, mission, index, def);
        }
        sim.scenario.dirty = true;
    }
    Ok(Value::Null)
}

fn start_mission_verb(sim: &mut Sim, call: &VerbCall<'_>, _cause: Cause) -> Result<Value, Reason> {
    playing(sim)?;
    let name = text_of(call, 0, "start_mission", "mission")?;
    let taker = match call.args.get(1) {
        Some(value) if !value.is_null() => match sim.subject_of(value) {
            Some(Subject::Entity(who)) if sim.entities.contains(who) => who,
            _ => {
                return Err(bad_argument(
                    "start_mission",
                    "taker",
                    "`taker` must be an entity that exists",
                ));
            }
        },
        _ => sim
            .scenario
            .game
            .as_ref()
            .and_then(lead)
            .ok_or_else(no_game)?,
    };
    let with: Vec<(String, Value)> = match call.args.get(2) {
        Some(Value::Map(map)) => map
            .iter()
            .map(|(key, value)| (key.to_string(), value.clone()))
            .collect(),
        _ => Vec::new(),
    };
    let serial = start_mission(sim, &name, taker, &with)?;
    Ok(Value::Int(i64::from(serial)))
}

fn fail_mission(sim: &mut Sim, call: &VerbCall<'_>, _cause: Cause) -> Result<Value, Reason> {
    playing(sim)?;
    let serial = mission_arg(call, 0).ok_or_else(|| {
        bad_argument(
            "fail_mission",
            "mission",
            "`mission` must be a mission's number",
        )
    })?;
    let active = sim
        .scenario
        .game
        .as_ref()
        .and_then(|game| game.mission(serial))
        .ok_or_else(|| {
            Reason::new(why::NO_MISSION, format!("there is no mission {serial}"))
                .with("mission", i64::from(serial))
        })?
        .state
        == MissionState::Active;
    if active {
        settle_mission(sim, serial, MissionState::Failed);
    }
    Ok(Value::Null)
}

fn checkpoint(sim: &mut Sim, call: &VerbCall<'_>, cause: Cause) -> Result<Value, Reason> {
    playing(sim)?;
    let name = text_of(call, 0, "checkpoint", "name")?;
    if let Some(game) = sim.scenario.game.as_mut() {
        game.checkpoints.retain(|have| *have != name);
        game.checkpoints.push(name.clone());
    }
    let _ = sim.raise(
        Where::Global,
        cause,
        EventPayload::new(CHECKPOINT_MADE).with("name", name.as_str()),
    );
    sim.request_app(AppRequest::Checkpoint { name });
    Ok(Value::Null)
}

fn restore_checkpoint(sim: &mut Sim, call: &VerbCall<'_>, _cause: Cause) -> Result<Value, Reason> {
    playing(sim)?;
    let asked = call.args.first().and_then(Value::as_str).map(str::to_owned);
    let made = sim
        .scenario
        .game
        .as_ref()
        .map(|game| game.checkpoints.clone())
        .unwrap_or_default();
    let name = match asked {
        Some(name) if made.contains(&name) => name,
        Some(name) => {
            return Err(Reason::new(
                why::NO_CHECKPOINT,
                format!("there is no checkpoint `{name}` to go back to"),
            )
            .with("name", name.as_str()));
        }
        None => made.last().cloned().ok_or_else(|| {
            Reason::new(why::NO_CHECKPOINT, "there is no checkpoint to go back to").with("name", "")
        })?,
    };
    sim.request_app(AppRequest::RestoreCheckpoint { name });
    Ok(Value::Null)
}

fn end_scenario_verb(sim: &mut Sim, call: &VerbCall<'_>, cause: Cause) -> Result<Value, Reason> {
    let outcome = text_of(call, 0, "end_scenario", "as")?;
    end_scenario(sim, &outcome, "", cause)?;
    Ok(Value::Null)
}

fn take_control_verb(sim: &mut Sim, call: &VerbCall<'_>, cause: Cause) -> Result<Value, Reason> {
    playing(sim)?;
    let who = crate::kernel_verbs::entity_arg(sim, call, 0, "take_control", "who")?;
    take_control(sim, who, cause);
    Ok(Value::Null)
}

/// The verbs of scenarios (13 §13).
///
/// | Verb | Does |
/// |------|------|
/// | `complete_objective { id, mission? }` | Settles an active objective as done: its `on_done`, `objective_changed` |
/// | `fail_objective { id, mission? }` | Settles an active objective as failed |
/// | `add_objective { id, mission? }` | Shows a hidden objective |
/// | `start_mission { mission, taker?, with? }` | Starts a mission; gives its number |
/// | `fail_mission { mission }` | Fails an active mission, by number |
/// | `checkpoint { name }` | Asks the application for a checkpoint save; raises `checkpoint_made` |
/// | `restore_checkpoint { name? }` | Asks the application to go back to a checkpoint: the one named, or the last |
/// | `end_scenario { as }` | Ends the game with an outcome, through the hook `scenario_ending` |
/// | `take_control { who }` | Gives the player direct control of a character, which joins the party |
pub fn verbs() -> Vec<VerbDef> {
    vec![
        VerbDef {
            name: "complete_objective",
            build: |_| {
                Some(
                    VerbSpec::new("complete_objective")
                        .arg("id", Type::Str, "The objective's id.")
                        .optional_arg("mission", Type::Int, "The number of the mission it is part of; left out, one of the scenario's own.")
                        .direct("id")
                        .doc("Settles an active objective as done (13 §5.1): its `on_done` runs and `objective_changed` is raised. An objective that is not active is left as it is. Refused with no game, or no such objective."),
                )
            },
            run: complete_objective,
        },
        VerbDef {
            name: "fail_objective",
            build: |_| {
                Some(
                    VerbSpec::new("fail_objective")
                        .arg("id", Type::Str, "The objective's id.")
                        .optional_arg("mission", Type::Int, "The number of the mission it is part of; left out, one of the scenario's own.")
                        .direct("id")
                        .doc("Settles an active objective as failed (13 §5.1): its `on_failed` runs and `objective_changed` is raised. Refused with no game, or no such objective."),
                )
            },
            run: fail_objective,
        },
        VerbDef {
            name: "add_objective",
            build: |_| {
                Some(
                    VerbSpec::new("add_objective")
                        .arg("id", Type::Str, "The objective's id.")
                        .optional_arg("mission", Type::Int, "The number of the mission it is part of; left out, one of the scenario's own.")
                        .direct("id")
                        .doc("Shows a hidden objective (13 §5.1): it becomes active, its expiry starts and `objective_changed` is raised with `active`. One that is not hidden is left as it is."),
                )
            },
            run: add_objective,
        },
        VerbDef {
            name: "start_mission",
            build: |registry| {
                let entity = registry.kind_id("entity")?;
                Some(
                    VerbSpec::new("start_mission")
                        .arg("mission", Type::Str, "The mission definition: a qualified id, or a bare name.")
                        .optional_arg("taker", Type::Subject(entity), "Who takes it on. Left out, the character the player plays.")
                        .optional_arg("with", Type::map(Type::Any), "Its parameters by name.")
                        .direct("mission")
                        .doc("Starts a mission (13 §5.2): it is recorded with its parameters, its objectives that wait for nothing are shown and `mission_started` is raised. Gives the mission's number. Refused for a mission that is not loaded, parameters it does not have or lacks, and with no game."),
                )
            },
            run: start_mission_verb,
        },
        VerbDef {
            name: "fail_mission",
            build: |_| {
                Some(
                    VerbSpec::new("fail_mission")
                        .arg("mission", Type::Int, "The mission's number.")
                        .direct("mission")
                        .doc("Fails an active mission (13 §5.2): its `on_failed` runs and `mission_failed` is raised."),
                )
            },
            run: fail_mission,
        },
        VerbDef {
            name: "checkpoint",
            build: |_| {
                Some(
                    VerbSpec::new("checkpoint")
                        .arg("name", Type::Str, "The checkpoint's name.")
                        .direct("name")
                        .doc("Asks the application for a save the player cannot make by hand (13 §9.1), at the next safe point, and raises `checkpoint_made`. It is saved under every saving policy but `none`."),
                )
            },
            run: checkpoint,
        },
        VerbDef {
            name: "restore_checkpoint",
            build: |_| {
                Some(
                    VerbSpec::new("restore_checkpoint")
                        .optional_arg("name", Type::Str, "The checkpoint. Left out, the last one made.")
                        .direct("name")
                        .doc("Asks the application to go back to a checkpoint (13 §9.1) at the next safe point. Refused, with `scenario.no_checkpoint`, when there is none of that name or none at all. Carrying memories across is not built."),
                )
            },
            run: restore_checkpoint,
        },
        VerbDef {
            name: "end_scenario",
            build: |_| {
                Some(
                    VerbSpec::new("end_scenario")
                        .arg("as", Type::Str, "The outcome it names: `won`, `lost`, `escaped`.")
                        .direct("as")
                        .doc("Ends the game (13 §9.4): raises the hook `scenario_ending`, whose handlers may change `as` or cancel, then `scenario_ended`, and asks the application to end it. Refused with no game, once it has ended, and when a handler cancels."),
                )
            },
            run: end_scenario_verb,
        },
        VerbDef {
            name: "take_control",
            build: |registry| {
                let entity = registry.kind_id("entity")?;
                Some(
                    VerbSpec::new("take_control")
                        .arg("who", Type::Subject(entity), "The character.")
                        .direct("who")
                        .doc("Gives the player direct control of a character (13 §13): it joins the party, those of the party that can no longer act are counted fallen, and the application is asked to show it."),
                )
            },
            run: take_control_verb,
        },
    ]
}

/// The time a game has run: since it began, in universal time.
pub fn played(sim: &Sim) -> clash_core::Dur {
    let began = sim
        .scenario
        .game
        .as_ref()
        .map_or(Tick::ZERO, |game| game.began);
    sim.now().since(began)
}
