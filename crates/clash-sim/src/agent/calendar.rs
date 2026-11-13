//! Calendars (09 §14.2; package L1): the length of a realm's day, the
//! names of the parts of its day and of its seasons, and the time of day
//! its clock reads.
//!
//! A realm's day is the day of the sky over it (02 §4.1, decision D-411):
//! how long it is and what time of day the realm's clock starts at. Its
//! clock runs at the realm's rate (03 §3.3), so a realm whose clock runs
//! fast has days that pass fast. A `calendar` definition, named by the
//! realm's `calendar` field, gives the day's parts and the year their
//! names; with none, a realm has days and times of day and no names.
//!
//! ```json
//! { "type": "calendar", "id": "common",
//!   "parts": [ { "name": "night", "from": "00:00" }, { "name": "dawn", "from": "05:00" },
//!              { "name": "day", "from": "07:00" }, { "name": "dusk", "from": "18:00" },
//!              { "name": "night", "from": "21:00" } ],
//!   "year": 360,
//!   "seasons": [ { "name": "spring", "from": 0 }, { "name": "summer", "from": 90 } ] }
//! ```
//!
//! A time of day written `"HH:MM"`, here and in routines (09 §17), is
//! hours and minutes of a twenty-four hour day, scaled to the length of
//! the realm's own.

use clash_core::{Dur, Real, Tick, define_id};
use clash_data::{Def, ValidateCx};
use clash_world::{CellPos, Location, RealmId};
use serde::Deserialize;

use crate::Sim;

define_id! {
    /// A `calendar` definition (09 §14.2).
    pub struct CalendarId(u16);
}

/// The length of the day a time of day written `"HH:MM"` counts in.
pub const WRITTEN_DAY: Dur = Dur::DAY;

/// A part of the day: its name and when it begins.
#[derive(Debug, Deserialize)]
pub struct DayPartDef {
    /// Its name, as rules read it: `dawn`, `night`.
    pub name: String,
    /// When it begins: `"HH:MM"` of a twenty-four hour day.
    pub from: String,
}

/// A season: its name and the day of the year it begins on.
#[derive(Debug, Deserialize)]
pub struct SeasonDef {
    /// Its name, as rules read it.
    pub name: String,
    /// The day of the year it begins on, counted from 0.
    pub from: u32,
}

/// A calendar (09 §14.2): the names of the parts of a realm's day, and
/// the length of its year and the names of its seasons. The length of
/// the day itself is the sky's (decision D-782).
#[derive(Debug, Deserialize)]
pub struct CalendarDef {
    /// The parts of the day, in order of when each begins; the first
    /// part also runs from midnight to its `from` if that is later, and
    /// the last until midnight.
    #[serde(default)]
    pub parts: Vec<DayPartDef>,
    /// How many days a year has. 0 or left out: there is no year, and no
    /// season.
    #[serde(default)]
    pub year: u32,
    /// The seasons, in order of the day each begins on.
    #[serde(default)]
    pub seasons: Vec<SeasonDef>,
    /// What it is, for the reference.
    #[serde(default)]
    pub doc: Option<String>,
}

impl Def for CalendarDef {
    type Id = CalendarId;
    const TYPE: &'static str = "calendar";
    const DOC: &'static str = "A realm's calendar (09 §14.2): the names of the parts of its day, each from a time of day, and the length of its year with the names of its seasons. The day's length and the time its clock starts at are the sky's.";

    fn validate(&self, cx: &mut ValidateCx<'_>) {
        let mut last: Option<Dur> = None;
        for part in &self.parts {
            match parse_clock(&part.from) {
                None => cx.error(format!(
                    "the part `{}` begins at `{}`, which is not a time of day `HH:MM`",
                    part.name, part.from
                )),
                Some(at) => {
                    if last.is_some_and(|last| at <= last) {
                        cx.error(format!(
                            "the part `{}` begins no later than the one before it: parts are written in order",
                            part.name
                        ));
                    }
                    last = Some(at);
                }
            }
        }
        let mut last_day: Option<u32> = None;
        for season in &self.seasons {
            if self.year == 0 {
                cx.error("seasons need a `year` of at least one day");
                break;
            }
            if season.from >= self.year {
                cx.error(format!(
                    "the season `{}` begins on day {}, and the year has {} days",
                    season.name, season.from, self.year
                ));
            }
            if last_day.is_some_and(|last| season.from <= last) {
                cx.error(format!(
                    "the season `{}` begins no later than the one before it: seasons are written in order",
                    season.name
                ));
            }
            last_day = Some(season.from);
        }
    }
}

/// A time of day written `"HH:MM"`, as time since midnight of a
/// twenty-four hour day. `None` if it is not one.
pub fn parse_clock(text: &str) -> Option<Dur> {
    let (hours, minutes) = text.trim().split_once(':')?;
    let hours: i64 = hours.trim().parse().ok()?;
    let minutes: i64 = minutes.trim().parse().ok()?;
    ((0..=24).contains(&hours) && (0..60).contains(&minutes) && !(hours == 24 && minutes > 0))
        .then(|| Dur::from_secs(hours * 3_600 + minutes * 60))
}

/// A time of a twenty-four hour day scaled to a day of another length.
pub fn scaled(written: Dur, day: Dur) -> Dur {
    if day == WRITTEN_DAY {
        return written;
    }
    let share = Real::from_i64(written.as_micros()) / Real::from_i64(WRITTEN_DAY.as_micros());
    Dur::from_micros((share * Real::from_i64(day.as_micros())).floor().to_i64())
}

/// A realm's day: how long it is, and what time of day its clock reads
/// when it reads zero.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Day {
    /// The length of a day, in the realm's local time.
    pub length: Dur,
    /// The time of day at local time zero.
    pub starts_at: Dur,
}

/// The day of a realm: the sky's over storey 0 of its base grid, where
/// people live, else the sky's at its top, as a scenario finds it
/// (13 §4.3); a day of twenty-four hours starting at midnight for a
/// realm with no sky.
pub fn realm_day(sim: &Sim, realm: RealmId) -> Day {
    let fallback = Day {
        length: WRITTEN_DAY,
        starts_at: Dur::ZERO,
    };
    let Some(grid) = sim.cosmos.realm(realm).map(|realm| realm.base_grid) else {
        return fallback;
    };
    let Ok(topology) = sim.cosmos.topology(grid) else {
        return fallback;
    };
    let ground = 0.clamp(topology.z_min, topology.z_max);
    [ground, topology.z_max]
        .into_iter()
        .find_map(|z| {
            let probe = Location::new(grid, CellPos::new(0, 0, z));
            sim.cosmos.sky(probe).ok().flatten().map(|sky| Day {
                length: if sky.day > Dur::ZERO {
                    sky.day
                } else {
                    WRITTEN_DAY
                },
                starts_at: sky.starts_at,
            })
        })
        .unwrap_or(fallback)
}

/// The local time of a realm's clock now.
pub fn local_now(sim: &Sim, realm: RealmId) -> Option<Dur> {
    let clock = sim.cosmos.realm(realm)?.clock;
    sim.timeline.local_now(clock).ok()
}

/// The time of the realm's clock, counted from the midnight before its
/// clock began: what days and times of day are reckoned from.
pub fn reckoned(day: Day, local: Dur) -> Dur {
    Dur::from_micros(local.as_micros().saturating_add(day.starts_at.as_micros()))
}

/// The time of day in a realm now, as time since midnight.
pub fn time_of_day(sim: &Sim, realm: RealmId) -> Option<Dur> {
    let day = realm_day(sim, realm);
    let local = local_now(sim, realm)?;
    Some(Dur::from_micros(
        reckoned(day, local)
            .as_micros()
            .rem_euclid(day.length.as_micros().max(1)),
    ))
}

/// The day it is in a realm now, counted from 1: the day the realm's
/// clock began on is day 1 (13 §4.3).
pub fn day_number(sim: &Sim, realm: RealmId) -> Option<i64> {
    let day = realm_day(sim, realm);
    let local = local_now(sim, realm)?;
    Some(
        reckoned(day, local)
            .as_micros()
            .div_euclid(day.length.as_micros().max(1))
            + 1,
    )
}

/// The calendar a realm names, if a loaded definition answers to it.
pub fn calendar_of(sim: &Sim, realm: RealmId) -> Option<&CalendarDef> {
    let def = sim.cosmos.realm(realm)?.def;
    let written = sim.cosmos.defs().realms().get(def)?.calendar.as_ref()?;
    let registry = sim.defs.try_registry::<CalendarDef>()?;
    let id = registry.lookup(written).or_else(|| {
        registry
            .names()
            .iter()
            .filter(|name| name.rsplit(':').next() == Some(written.as_str()))
            .min()
            .and_then(|name| registry.lookup(name))
    })?;
    Some(registry.get(id))
}

/// The name of the part of the day it is in a realm now, by its
/// calendar. `None` for a realm with no calendar or no parts.
pub fn day_part(sim: &Sim, realm: RealmId) -> Option<String> {
    let calendar = calendar_of(sim, realm)?;
    let now = time_of_day(sim, realm)?;
    let length = realm_day(sim, realm).length;
    let mut current = calendar.parts.last()?;
    for part in &calendar.parts {
        let from = parse_clock(&part.from).map(|at| scaled(at, length))?;
        if from <= now {
            current = part;
        }
    }
    Some(current.name.clone())
}

/// The name of the season it is in a realm now, by its calendar.
/// `None` for a realm with no calendar, no year or no seasons.
pub fn season(sim: &Sim, realm: RealmId) -> Option<String> {
    let calendar = calendar_of(sim, realm)?;
    if calendar.year == 0 {
        return None;
    }
    let of_year = (day_number(sim, realm)? - 1).rem_euclid(i64::from(calendar.year));
    let mut current = calendar.seasons.last()?;
    for season in &calendar.seasons {
        if i64::from(season.from) <= of_year {
            current = season;
        }
    }
    Some(current.name.clone())
}

/// The universal tick at which a realm's clock will read a local time,
/// at the rate it runs now. `None` if the clock is stopped or gone, or
/// the time is not after now.
pub fn tick_at_local(sim: &Sim, realm: RealmId, local: Dur) -> Option<Tick> {
    let clock = sim.cosmos.realm(realm)?.clock;
    let now_local = sim.timeline.local_now(clock).ok()?;
    let rate = sim.timeline.effective_rate(clock)?;
    if rate <= Real::ZERO || local <= now_local {
        return None;
    }
    let ahead = local - now_local;
    let universal = if rate == Real::ONE {
        ahead
    } else {
        Dur::from_secs_real(ahead.as_secs_real() / rate)
    };
    Some(sim.now() + universal.max(Dur::from_micros(1)))
}
