//! The world a scenario's game is made in (13 §4.1, §4.3).

use std::sync::Arc;

use clash_core::{Dur, Real, Tick};
use clash_data::Defs;
use clash_time::Timeline;
use clash_world::{CellPos, Cosmos, Location, RealmId, WorldDefs};

use super::def::{ScenarioDef, namespace_of};
use super::why;
use crate::reason::Reason;

/// A world made for a scenario: the cosmos with its realm, the timeline
/// its clock is on, already at the time the game begins, and the realm.
#[derive(Debug)]
pub struct WorldMade {
    /// The world.
    pub cosmos: Cosmos,
    /// Its time, set so that the realm's clock reads the day and time
    /// the scenario begins at.
    pub timeline: Timeline,
    /// The realm the game is played in.
    pub realm: RealmId,
}

/// The scenario of a qualified id, or the reason there is none.
pub(crate) fn scenario_named<'a>(
    defs: &'a Defs,
    scenario: &str,
) -> Result<&'a ScenarioDef, Reason> {
    defs.try_registry::<ScenarioDef>()
        .and_then(|registry| registry.lookup(scenario).map(|id| registry.get(id)))
        .ok_or_else(|| {
            Reason::new(why::UNKNOWN, format!("there is no scenario `{scenario}`"))
                .with("scenario", scenario)
        })
}

/// The universal tick a scenario's game begins at, so that the realm's
/// clock reads the day and time its `start.time` names: the time since
/// the realm's clock began that it would read then, less the time of
/// day the realm's sky starts its clock at, at the realm's rate. A
/// scenario that names no time begins at tick zero.
pub fn start_tick(def: &ScenarioDef, day_length: Dur, starts_at: Dur, rate: Real) -> Tick {
    let Some(time) = &def.start.time else {
        return Tick::ZERO;
    };
    let Some(at) = time.since_midnight() else {
        return Tick::ZERO;
    };
    let day = day_length.as_micros().max(1);
    let days = i64::from(time.day.saturating_sub(1));
    let mut local = days.saturating_mul(day) + at.as_micros() - starts_at.as_micros();
    while local < 0 {
        local += day;
    }
    let universal = if rate > Real::ZERO && rate != Real::ONE {
        Dur::from_micros(local).div_real(rate)
    } else {
        Dur::from_micros(local)
    };
    Tick::ZERO + universal
}

/// Makes the world of a scenario (13 §4.1): its realm, created with
/// `seed`, and time moved on so that the game begins at the day and time
/// its `start.time` names (13 §4.3). Nothing is in the world yet, so
/// nothing happens in the time that passes.
///
/// `seed` is the seed the scenario's `seed` chooses
/// ([`SeedSpec::choose`](super::SeedSpec::choose)).
pub fn make_world(
    defs: &Defs,
    world: Arc<WorldDefs>,
    scenario: &str,
    seed: u64,
) -> Result<WorldMade, Reason> {
    let def = scenario_named(defs, scenario)?;
    let realm_name = def
        .realm_id(namespace_of(scenario))
        .ok_or_else(|| Reason::new(why::BAD_WORLD, "the scenario names no world to be made in"))?;
    let realm_def = world.realms().lookup(&realm_name).ok_or_else(|| {
        Reason::new(
            why::BAD_WORLD,
            format!("the scenario is made in a realm `{realm_name}` that no mod defines"),
        )
        .with("realm", realm_name.as_str())
    })?;
    let rate = world
        .realms()
        .get(realm_def)
        .map_or(Real::ONE, |kind| kind.time_rate);
    let mut timeline = Timeline::new();
    let mut cosmos = Cosmos::new(world);
    let realm = cosmos
        .create_realm(&mut timeline, realm_def, seed)
        .map_err(|error| Reason::from_world(&error))?;
    let grid = cosmos
        .realm(realm)
        .map(|realm| realm.base_grid)
        .ok_or_else(|| Reason::new(why::BAD_WORLD, "the realm was not made"))?;
    let (day, starts_at) = cosmos
        .topology(grid)
        .ok()
        .and_then(|topology| {
            let probe = Location::new(grid, CellPos::new(0, 0, topology.z_max));
            cosmos
                .sky(probe)
                .ok()
                .flatten()
                .map(|sky| (sky.day, sky.starts_at))
        })
        .unwrap_or((Dur::DAY, Dur::ZERO));
    timeline.advance_to(start_tick(def, day, starts_at, rate));
    Ok(WorldMade {
        cosmos,
        timeline,
        realm,
    })
}
