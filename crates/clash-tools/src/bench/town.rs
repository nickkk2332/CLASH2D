//! The benchmarks of the market town (09 §19; stream "performance 2"):
//! the bundled town (`core:town`, seed 7) begun as its game scenario
//! begins it, its residents loaded round the player, through the
//! console's host, as a player meets it.
//!
//! | Benchmark | Times |
//! |-----------|-------|
//! | `town.step_in_market` | One simulated second (a player's step) with the player standing in the square, over a simulated minute |
//! | `town.step_walking` | The same with the player walking across the town |
//! | `town.minute_wait` | A minute's wait in the square, in one call |
//! | `town.minute_wait_night` | The same at eleven at night, most of the town asleep |
//! | `town.hour_wait` | An hour's wait in the square by day, in one call, with the levels of a resting party (D-932) |
//! | `town.hour_wait_night` | The same from eleven at night |
//!
//! Each is timed once: a town takes seconds to begin, and a minute of it
//! is many steps. The figure is the mean of the minute's steps; the
//! slowest is printed beside it.

use std::sync::Arc;
use std::time::Instant;

use clash_save::host::{GameHost, HostEnv};
use clash_sim::Sim;
use clash_sim::console::{CommandTable, HostKind, Session};

use super::{Measured, show_ns};

/// The town's seed: the one its timing test and its test scripts use.
const TOWN_SEED: u64 = 7;

/// The scenario's place in the square (its places are the north street
/// and the square).
const SQUARE: u32 = 1;

/// Simulated seconds let pass before timing: the first thoughts of
/// everyone just made, and the first sweeps.
const SETTLE_S: u32 = 5;

/// Simulated seconds timed, one at a time.
const STEPS: u32 = 60;

/// How far east of the square the walker's errand takes it, in cells,
/// and how far south: across the market and on through the town.
const WALK_EAST: i32 = 90;
const WALK_SOUTH: i32 = 30;

/// A console over the game's host, as the town's tests drive it.
struct Town {
    table: CommandTable,
    host: GameHost,
    session: Session,
}

impl Town {
    /// The market town begun at one of its places, with its seed.
    fn begin(place: u32) -> Result<Town, String> {
        let env = Arc::new(HostEnv::new(super::cases::mods_dir()));
        let mut town = Town {
            table: GameHost::table(),
            host: GameHost::new(env),
            session: Session::new(HostKind::File),
        };
        town.run(&format!("use default seed={TOWN_SEED}"))?;
        town.run(&format!("begin core:town_life place={place}"))?;
        Ok(town)
    }

    fn run(&mut self, line: &str) -> Result<(), String> {
        self.session
            .run_line(&self.table, &mut self.host, line)
            .map(|_| ())
            .map_err(|error| format!("`{line}`: {error}"))
    }

    fn sim(&self) -> Result<&Sim, String> {
        self.host
            .sim()
            .ok_or_else(|| "no game is running".to_owned())
    }

    /// What the town holds: its residents, those in the world, and how
    /// many of those are hot.
    fn census(&self) -> Result<String, String> {
        let sim = self.sim()?;
        let residents = sim.agents().residents().count();
        let loaded = sim
            .agents()
            .residents()
            .filter(|(_, resident)| {
                resident
                    .entity
                    .is_some_and(|entity| sim.truth().location(entity).is_some())
            })
            .count();
        let levels = clash_sim::ai::warm::census(sim);
        let tracts = sim.truth().cosmos().tickets().kept_count();
        Ok(format!(
            "{residents} residents, {loaded} in the world, {} hot and {} warm; {tracts} tracts loaded{}",
            levels.hot,
            levels.warm,
            if clash_sim::rest::is_resting(sim) {
                ", the party resting"
            } else {
                ""
            }
        ))
    }

    /// Times `STEPS` simulated seconds one by one, doing `before` ahead
    /// of each: the mean and the slowest, in nanoseconds.
    fn time_steps(
        &mut self,
        mut before: impl FnMut(&mut Town) -> Result<(), String>,
    ) -> Result<(f64, f64), String> {
        let mut total = 0.0;
        let mut worst: f64 = 0.0;
        for _ in 0..STEPS {
            before(self)?;
            let started = Instant::now();
            self.run("advance 1s")?;
            let took = started.elapsed().as_nanos() as f64;
            total += took;
            worst = worst.max(took);
        }
        Ok((total / f64::from(STEPS), worst))
    }
}

/// The player standing in the square: one simulated second.
pub(super) fn step_in_market(_kit: &mut super::cases::Kit) -> Result<Measured, String> {
    let mut town = Town::begin(SQUARE)?;
    town.run(&format!("advance {SETTLE_S}s"))?;
    let steps = town.sim()?.steps();
    let (mean, worst) = town.time_steps(|_| Ok(()))?;
    let wakes = town.sim()?.steps() - steps;
    Ok(Measured::new(
        mean,
        format!(
            "slowest step {}; {} wake-ups a second; {}",
            show_ns(worst),
            wakes / u64::from(STEPS),
            town.census()?
        ),
    ))
}

/// The player walking across the town from the square: one simulated
/// second of it.
pub(super) fn step_walking(_kit: &mut super::cases::Kit) -> Result<Measured, String> {
    let mut town = Town::begin(SQUARE)?;
    town.run(&format!("advance {SETTLE_S}s"))?;
    let start = town
        .sim()?
        .truth()
        .location(player(town.sim()?)?)
        .ok_or("the player is not in the world")?;
    let goal = clash_world::Location::new(
        start.grid,
        clash_world::CellPos::new(
            start.cell.x + WALK_EAST,
            start.cell.y + WALK_SOUTH,
            start.cell.z,
        ),
    );
    let steps = town.sim()?.steps();
    let (mean, worst) = town.time_steps(|town| {
        let sim = town.sim()?;
        let me = player(sim)?;
        let here = sim
            .truth()
            .location(me)
            .ok_or("the player is not in the world")?;
        if sim.is_free(me) {
            // The errand, from where the player stands now: `go_to`
            // with the goal as a cell that far from here.
            let errand = format!(
                "as @self do {{ \"start_action\": {{ \"who\": \"self\", \"action\": \"core:go_to\", \"with\": \"{{'to': offset(self.loc, {}, {})}}\" }} }}",
                goal.cell.x - here.cell.x,
                goal.cell.y - here.cell.y
            );
            town.run(&errand)?;
        }
        Ok(())
    })?;
    let wakes = town.sim()?.steps() - steps;
    let end = town
        .sim()?
        .truth()
        .location(player(town.sim()?)?)
        .ok_or("the player is not in the world")?;
    let walked = (end.cell.x - start.cell.x).abs() + (end.cell.y - start.cell.y).abs();
    if walked < 20 {
        return Err(format!(
            "the player walked {walked} cells in a minute: the errand did not take"
        ));
    }
    Ok(Measured::new(
        mean,
        format!(
            "slowest step {}; {walked} cells walked; {} wake-ups a second; {}",
            show_ns(worst),
            wakes / u64::from(STEPS),
            town.census()?
        ),
    ))
}

/// A minute's wait in the square, in one call.
pub(super) fn minute_wait(_kit: &mut super::cases::Kit) -> Result<Measured, String> {
    let mut town = Town::begin(SQUARE)?;
    town.run(&format!("advance {SETTLE_S}s"))?;
    let started = Instant::now();
    wait_a_minute(&mut town)?;
    let took = started.elapsed().as_nanos() as f64;
    Ok(Measured::new(took, town.census()?))
}

/// A minute's wait in the square at night (performance 3): the town
/// begun as by day, the player taken out into the fields while the day
/// passes (the town unloaded, its residents agents), and brought back to
/// where it began at `NIGHT_S` past midnight, when most are asleep.
pub(super) fn minute_wait_night(_kit: &mut super::cases::Kit) -> Result<Measured, String> {
    let mut town = Town::begin(SQUARE)?;
    let sim = town.sim()?;
    let start = sim
        .truth()
        .location(player(sim)?)
        .ok_or("the player is not in the world")?;
    let away = clash_world::Location::new(start.grid, clash_world::CellPos::new(AWAY.0, AWAY.1, 0));
    town.run(&format!("session @self {}", cell(away)))?;
    town.run("detach_sense @self core:human_sight")?;
    town.run("detach_sense @self core:human_hearing")?;
    let now = town.time_of_day()?;
    town.run(&format!("advance {}s", (NIGHT_S - now).rem_euclid(DAY_S)))?;
    town.run(&format!("session @self {}", cell(start)))?;
    town.run("attach_sense @self core:human_sight")?;
    town.run("attach_sense @self core:human_hearing")?;
    town.run(&format!("advance {SETTLE_S}s"))?;
    let started = Instant::now();
    wait_a_minute(&mut town)?;
    let took = started.elapsed().as_nanos() as f64;
    Ok(Measured::new(took, town.census()?))
}

/// An hour's wait in the square by day (smooth play, D-932): the
/// player's character waits an hour in one call, as `shift+p` waits it,
/// with the levels the bundled game sets while the party rests.
pub(super) fn hour_wait(_kit: &mut super::cases::Kit) -> Result<Measured, String> {
    let mut town = Town::begin(SQUARE)?;
    town.run(&format!("advance {SETTLE_S}s"))?;
    timed_hour(&mut town)
}

/// An hour's wait in the square, the player's own wait made first so
/// that the census can be taken half way through it: the time, and the
/// census half way and at the end.
fn timed_hour(town: &mut Town) -> Result<Measured, String> {
    town.run(&format!(
        "as @self do {{ \"wait\": {{ \"who\": \"self\", \"time\": {HOUR_S} }} }}"
    ))?;
    let started = Instant::now();
    town.run(&format!("advance {}s", HOUR_S / 2))?;
    let half = started.elapsed();
    let midway = town.census()?;
    let resumed = Instant::now();
    town.run(&format!("advance {}s", HOUR_S / 2))?;
    let took = (half + resumed.elapsed()).as_nanos() as f64;
    Ok(Measured::new(
        took,
        format!("half way: {midway}; at the end: {}", town.census()?),
    ))
}

/// An hour's wait in the square at night: as [`minute_wait_night`]
/// brings the player back to the square at eleven, then an hour.
pub(super) fn hour_wait_night(_kit: &mut super::cases::Kit) -> Result<Measured, String> {
    let mut town = Town::begin(SQUARE)?;
    let sim = town.sim()?;
    let start = sim
        .truth()
        .location(player(sim)?)
        .ok_or("the player is not in the world")?;
    let away = clash_world::Location::new(start.grid, clash_world::CellPos::new(AWAY.0, AWAY.1, 0));
    town.run(&format!("session @self {}", cell(away)))?;
    town.run("detach_sense @self core:human_sight")?;
    town.run("detach_sense @self core:human_hearing")?;
    let now = town.time_of_day()?;
    town.run(&format!("advance {}s", (NIGHT_S - now).rem_euclid(DAY_S)))?;
    town.run(&format!("session @self {}", cell(start)))?;
    town.run("attach_sense @self core:human_sight")?;
    town.run("attach_sense @self core:human_hearing")?;
    town.run(&format!("advance {SETTLE_S}s"))?;
    timed_hour(&mut town)
}

/// Simulated seconds in an hour's wait.
const HOUR_S: u32 = 3600;

/// The minute timed by the waits, kept out of line so that a profiler
/// can be told to count it alone (`valgrind --tool=callgrind
/// --toggle-collect='*town::wait_a_minute*'`).
#[inline(never)]
fn wait_a_minute(town: &mut Town) -> Result<(), String> {
    town.run(&format!("advance {STEPS}s"))
}

/// The time of day of the night wait: eleven in the evening, in seconds
/// past midnight.
const NIGHT_S: i64 = 23 * 3600;

/// Seconds in the town's day.
const DAY_S: i64 = 86_400;

/// A cell of open country far from the town, where it unloads.
const AWAY: (i32, i32) = (600, 600);

/// The text of a cell, for the console: `@c:x,y,z`.
fn cell(loc: clash_world::Location) -> String {
    format!("@c:{},{},{}", loc.cell.x, loc.cell.y, loc.cell.z)
}

impl Town {
    /// The time of day where the player is, in whole seconds.
    fn time_of_day(&mut self) -> Result<i64, String> {
        let out = self
            .session
            .run_line(
                &self.table,
                &mut self.host,
                "eval \"@self.loc.realm.time_of_day\"",
            )
            .map_err(|error| format!("the time of day: {error}"))?;
        out.subject
            .and_then(|value| value.as_number())
            .map(|seconds| seconds.get() as i64)
            .ok_or_else(|| "the time of day is not a number".to_owned())
    }
}

/// The character the player plays.
fn player(sim: &Sim) -> Result<clash_core::EntityId, String> {
    sim.truth()
        .entities()
        .iter()
        .find(|record| record.control() == clash_sim::Control::Direct)
        .map(clash_sim::KernelEntity::id)
        .ok_or_else(|| "nobody is played".to_owned())
}
