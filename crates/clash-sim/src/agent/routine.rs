//! Routines (09 §10.2; package L2): where a character is and what it is
//! doing at each part of the day, and the goal it pursues there when it
//! is loaded.
//!
//! ```json
//! { "type": "routine", "id": "townsperson_day",
//!   "day": [
//!     { "from": "06:30", "at": { "home": true }, "doing": "at_home", "goal": "spend_time" },
//!     { "from": "08:00", "at": [ { "site_tag": "work", "pick": "any" }, { "home": true } ],
//!       "doing": "working", "goal": "spend_time" },
//!     { "from": "12:00", "at": { "site_tag": "market" }, "doing": "shopping", "goal": "spend_time" },
//!     { "from": "15:00", "at": { "home": true }, "doing": "at_home", "goal": "spend_time" },
//!     { "from": "22:00", "at": { "home": true }, "doing": "asleep", "goal": "sleep" } ] }
//! ```
//!
//! A place is named by role, never by coordinates: the character's
//! `home` (where the settlement put it), or a site of its realm by
//! definition or tag. Which site of several: the nearest to its home, one
//! drawn from its own seed (`any`), or every one of them (`all`, for a
//! patrol). `at` may list several, the first that names a place being
//! used: a routine can name a tavern that a town may not have and fall
//! back on home. A site's place is its anchor, and each character keeps
//! to a spot of its own within `spread` cells of it. Roles inside a site
//! (`role`) are carried: the world does not record where a site's marks
//! went, so its anchor stands for every role (decision D-751).
//!
//! The engine names no place and no activity: `doing` is a word content
//! chooses, and so is every tag.

use clash_core::{Dur, HashDomain, Real, define_id, pos_hash};
use clash_data::{Def, FromKind, Kind, KindValue, Ref, Time, ValidateCx, by_kind};
use clash_world::{CellPos, GridId, Location, RealmId};
use serde::de::MapAccess;
use serde::{Deserialize, Deserializer};

use super::calendar::{self, Day};
use crate::Sim;
use crate::ai::GoalDef;

define_id! {
    /// A `routine` definition (09 §10.2).
    pub struct RoutineId(u16);
}

/// How fast a character is taken to walk between the places of its
/// routine while it is not loaded, if the routine does not say, in
/// metres a second.
pub const DEFAULT_TRAVEL_SPEED: f64 = 1.2;

/// How much longer than the straight line between two places the way
/// between them is taken to be while unloaded: streets are not straight.
pub const TRAVEL_DETOUR: f64 = 1.3;

/// How close to its place counts as there, if an entry does not say, in
/// metres.
pub const DEFAULT_WITHIN_M: f64 = 2.0;

/// How far from a site's anchor a character's own spot there may be, if
/// an entry does not say, in cells.
pub const DEFAULT_SPREAD: i32 = 3;

/// The most cells a spot may be from its site's anchor.
pub const MAX_SPREAD: i32 = 16;

/// The domain a character's own spot at a place is drawn from.
const SPOT_DOMAIN: HashDomain = HashDomain::new("agent.spot");

/// The domain a character's pick of one site of several is drawn from.
const PICK_DOMAIN: HashDomain = HashDomain::new("agent.pick");

/// The domain whether a character keeps a part of its routine on a day
/// is drawn from.
const CHANCE_DOMAIN: HashDomain = HashDomain::new("agent.chance");

/// The domain a character's own delay at the start of a part is drawn
/// from.
const JITTER_DOMAIN: HashDomain = HashDomain::new("agent.jitter");

fn default_speed() -> Real {
    Real::of(DEFAULT_TRAVEL_SPEED)
}

fn default_within() -> Real {
    Real::of(DEFAULT_WITHIN_M)
}

fn default_spread() -> i32 {
    DEFAULT_SPREAD
}

fn one() -> Real {
    Real::ONE
}

/// Which of the sites a place selector answers to a character takes.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Pick {
    /// The one nearest its home, the first in id order of equals.
    #[default]
    Nearest,
    /// One drawn from the character's own seed: the same one every day.
    Any,
    /// Every one, nearest its home first: for a beat between them.
    All,
}

/// A place named by role (09 §10.2): exactly one of `home`, `site` and
/// `site_tag`.
#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PlaceSel {
    /// The character's home: where the settlement put it, or for a
    /// character that is not a resident, its mind's home.
    #[serde(default)]
    pub home: bool,
    /// A site of the realm by its definition, qualified or bare.
    #[serde(default)]
    pub site: Option<String>,
    /// A site of the realm carrying this tag.
    #[serde(default)]
    pub site_tag: Option<String>,
    /// A role within the site, as its definition names it. Carried: the
    /// site's anchor stands for every role.
    #[serde(default)]
    pub role: Option<String>,
    /// Which of several sites: `nearest` (the default), `any` or `all`.
    #[serde(default)]
    pub pick: Pick,
}

/// The place of a routine entry: one selector, written as an object, or
/// a list of them of which the first that names a place is used.
#[derive(Debug)]
pub enum PlaceSpec {
    /// One selector.
    One(PlaceSel),
    /// Selectors in order of preference.
    Any(Vec<PlaceSel>),
}

impl<'de> FromKind<'de> for PlaceSpec {
    fn from_kind<A: MapAccess<'de>>(kind: Kind, value: KindValue<'_, A>) -> Result<Self, A::Error> {
        match kind {
            Kind::List => value.read::<Vec<PlaceSel>>().map(PlaceSpec::Any),
            Kind::Object => value.read::<PlaceSel>().map(PlaceSpec::One),
            other => Err(other.unexpected("a place, or a list of places")),
        }
    }
}

impl<'de> Deserialize<'de> for PlaceSpec {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<PlaceSpec, D::Error> {
        by_kind(deserializer)
    }
}

impl PlaceSpec {
    /// The selectors in order of preference.
    pub fn selectors(&self) -> &[PlaceSel] {
        match self {
            PlaceSpec::One(one) => core::slice::from_ref(one),
            PlaceSpec::Any(many) => many,
        }
    }
}

/// One part of a routine's day.
#[derive(Debug, Deserialize)]
pub struct RoutineEntry {
    /// When it begins: `"HH:MM"` of a twenty-four hour day, scaled to the
    /// realm's (09 §17).
    pub from: String,
    /// Where the character is meant to be.
    pub at: PlaceSpec,
    /// What it is doing there: a word content chooses (`asleep`,
    /// `working`), read by rules and the inspector.
    pub doing: String,
    /// The goal it pursues while this part lasts and it is loaded: it
    /// joins the goals its mind may have, and is scored as they are.
    #[serde(default)]
    pub goal: Option<Ref<GoalDef>>,
    /// How close to its place counts as there, in metres. Default 2.
    #[serde(default = "default_within")]
    pub within: Real,
    /// How far from a site's anchor its own spot there may be, in cells.
    /// Default 3; a home is exact.
    #[serde(default = "default_spread")]
    pub spread: i32,
    /// The share of days on which a character keeps this part, from 0 to
    /// 1, drawn for each character and day from its own seed: on a day
    /// it does not, the part before runs on. Default 1: every day.
    #[serde(default = "one")]
    pub chance: Real,
    /// Up to how much later than `from` each character begins it, the
    /// same every day, drawn from its own seed: a street does not empty
    /// at one tick. Less than the time to the next part. Default 0.
    #[serde(default)]
    pub jitter: Time,
    /// For a part whose `at` names several places (`pick: all`): how
    /// long it keeps to each before the next, in turn from the first, a
    /// beat between them. Default 0: the first place only.
    #[serde(default)]
    pub beat: Time,
}

/// A routine (09 §10.2): the parts of a day, each with a place, an
/// activity and the goal pursued there.
#[derive(Debug, Deserialize)]
pub struct RoutineDef {
    /// The parts of the day, in order of when each begins. The last runs
    /// on past midnight into the first.
    pub day: Vec<RoutineEntry>,
    /// How fast the character is taken to walk between places while it
    /// is not loaded, in metres a second. Default 1.2.
    #[serde(default = "default_speed")]
    pub speed: Real,
    /// What it is, for the reference.
    #[serde(default)]
    pub doc: Option<String>,
}

impl Def for RoutineDef {
    type Id = RoutineId;
    const TYPE: &'static str = "routine";
    const DOC: &'static str = "A routine (09 §10.2): the parts of a character's day, each beginning at a time of day, with the place it is meant to be by role (home, a site by definition or tag), what it is doing, and the goal it pursues there while loaded. While unloaded it is placed by its routine, walking between places at `speed`.";

    fn validate(&self, cx: &mut ValidateCx<'_>) {
        if self.day.is_empty() {
            cx.error("a routine has at least one part of the day");
        }
        let mut last: Option<Dur> = None;
        for entry in &self.day {
            match calendar::parse_clock(&entry.from) {
                None => cx.error(format!("`{}` is not a time of day `HH:MM`", entry.from)),
                Some(at) => {
                    if last.is_some_and(|last| at <= last) {
                        cx.error(format!(
                            "the part beginning at `{}` begins no later than the one before it: parts are written in order",
                            entry.from
                        ));
                    }
                    last = Some(at);
                }
            }
            if entry.at.selectors().is_empty() {
                cx.error(format!(
                    "the part beginning at `{}` names no place",
                    entry.from
                ));
            }
            for sel in entry.at.selectors() {
                let named = usize::from(sel.home)
                    + usize::from(sel.site.is_some())
                    + usize::from(sel.site_tag.is_some());
                if named != 1 {
                    cx.error(format!(
                        "a place of the part beginning at `{}` is exactly one of `home`, `site` and `site_tag`",
                        entry.from
                    ));
                }
            }
            if entry.doing.is_empty() {
                cx.error(format!(
                    "the part beginning at `{}` says what is being done in `doing`",
                    entry.from
                ));
            }
            if !(entry.within.is_finite() && entry.within >= Real::ZERO) {
                cx.error("`within` is a distance of zero or more");
            }
            if !(0..=MAX_SPREAD).contains(&entry.spread) {
                cx.error(format!("`spread` is from 0 to {MAX_SPREAD} cells"));
            }
            if !(entry.chance.is_finite() && (Real::ZERO..=Real::ONE).contains(&entry.chance)) {
                cx.error("`chance` is a share from 0 to 1");
            }
            if entry.jitter.dur() < Dur::ZERO {
                cx.error("`jitter` is a time of zero or more");
            }
            if entry.beat.dur() < Dur::ZERO {
                cx.error("`beat` is a time of zero or more");
            }
        }
        // Each part's jitter ends before the next part begins.
        let starts: Vec<Option<Dur>> = self
            .day
            .iter()
            .map(|entry| calendar::parse_clock(&entry.from))
            .collect();
        for (at, entry) in self.day.iter().enumerate() {
            let (Some(Some(this)), Some(Some(next))) =
                (starts.get(at), starts.get((at + 1) % starts.len().max(1)))
            else {
                continue;
            };
            let mut gap = *next - *this;
            if gap <= Dur::ZERO {
                gap += calendar::WRITTEN_DAY;
            }
            if self.day.len() > 1 && entry.jitter.dur() >= gap {
                cx.error(format!(
                    "the part beginning at `{}` has a `jitter` as long as the time to the next part",
                    entry.from
                ));
            }
        }
        if !(self.speed.is_finite() && self.speed > Real::ZERO) {
            cx.error("`speed` is a speed above zero, in metres a second");
        }
    }
}

/// A routine definition by name, qualified or bare.
pub fn routine_named<'d>(sim: &'d Sim, name: &str) -> Option<(RoutineId, &'d RoutineDef)> {
    let registry = sim.defs.try_registry::<RoutineDef>()?;
    let id = registry.lookup(name).or_else(|| {
        if name.contains(':') {
            return None;
        }
        registry
            .names()
            .iter()
            .filter(|qualified| qualified.rsplit(':').next() == Some(name))
            .min()
            .and_then(|qualified| registry.lookup(qualified))
    })?;
    Some((id, registry.get(id)))
}

/// The qualified id of a routine.
pub fn routine_name(sim: &Sim, id: RoutineId) -> String {
    sim.defs
        .try_registry::<RoutineDef>()
        .filter(|registry| id.index() < registry.len())
        .map_or_else(String::new, |registry| registry.name(id).to_owned())
}

/// Whose routine is being read: the realm it lives in, its home and its
/// own seed.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Whom {
    /// The realm.
    pub realm: RealmId,
    /// Its home.
    pub home: Location,
    /// Its own seed: which site of several, and its spot at each.
    pub seed: u64,
}

/// How many days back and ahead the parts of a routine are looked for:
/// a part with a `chance` may be skipped several days running.
pub const LOOK_DAYS: i64 = 8;

/// When each part of a routine begins in a day, for a character: its
/// `from`, scaled to the realm's day, and its own delay within the
/// part's `jitter`, as microseconds into the day; `None` for a part
/// whose `from` does not read. The same every day: worked out once for
/// all the days [`entry_at`] and [`next_start`] look at.
fn day_starts(def: &RoutineDef, day: Day, seed: u64) -> Vec<Option<i64>> {
    def.day
        .iter()
        .enumerate()
        .map(|(entry, part)| {
            let from = calendar::scaled(calendar::parse_clock(&part.from)?, day.length).as_micros();
            let jitter = calendar::scaled(part.jitter.dur(), day.length).as_micros();
            let delay = if jitter > 0 {
                let hash = pos_hash(seed, JITTER_DOMAIN, entry as i32, 0, 0, 0);
                (hash % (jitter as u64 + 1)) as i64
            } else {
                0
            };
            Some(from + delay)
        })
        .collect()
}

/// When a part of a routine begins on a day, for a character, as a
/// reckoned time: its start in the day from [`day_starts`]. `None` if it
/// does not keep that part that day (its `chance`).
fn start_on(
    def: &RoutineDef,
    day: Day,
    seed: u64,
    starts: &[Option<i64>],
    entry: usize,
    on: i64,
) -> Option<i64> {
    let part = def.day.get(entry)?;
    let start = (*starts.get(entry)?)?;
    if part.chance < Real::ONE {
        let hash = pos_hash(
            seed,
            CHANCE_DOMAIN,
            entry as i32,
            on as i32,
            (on >> 32) as i32,
            0,
        );
        let draw = Real::from_i64((hash >> 11) as i64) / Real::from_i64(1_i64 << 53);
        if draw >= part.chance {
            return None;
        }
    }
    let length = day.length.as_micros().max(1);
    Some(on * length + start)
}

/// The part of a routine's day a character is at, at a reckoned time
/// (time since the midnight before the realm's clock began): the
/// entry's position, and the reckoned time its part began, which may be
/// a day before or more. A part it does not keep that day (its
/// `chance`) is passed over: the one before runs on.
pub fn entry_at(def: &RoutineDef, day: Day, reckoned: Dur, seed: u64) -> Option<(usize, Dur)> {
    let length = day.length.as_micros().max(1);
    let today = reckoned.as_micros().div_euclid(length);
    let starts = day_starts(def, day, seed);
    let mut found: Option<(i64, usize)> = None;
    for on in (today - LOOK_DAYS)..=today {
        for entry in 0..def.day.len() {
            let Some(start) = start_on(def, day, seed, &starts, entry, on) else {
                continue;
            };
            if start <= reckoned.as_micros() && found.is_none_or(|(have, _)| start >= have) {
                found = Some((start, entry));
            }
        }
    }
    found.map(|(start, entry)| (entry, Dur::from_micros(start)))
}

/// When the next part a character keeps begins after a reckoned time,
/// as a reckoned time.
pub fn next_start(def: &RoutineDef, day: Day, after: Dur, seed: u64) -> Option<Dur> {
    let length = day.length.as_micros().max(1);
    let today = after.as_micros().div_euclid(length);
    let starts = day_starts(def, day, seed);
    let mut found: Option<i64> = None;
    for on in today..=(today + LOOK_DAYS) {
        for entry in 0..def.day.len() {
            let Some(start) = start_on(def, day, seed, &starts, entry, on) else {
                continue;
            };
            if start > after.as_micros() && found.is_none_or(|have| start < have) {
                found = Some(start);
            }
        }
    }
    found.map(Dur::from_micros)
}

/// The base grid of a realm.
fn grid_of(sim: &Sim, realm: RealmId) -> Option<GridId> {
    sim.cosmos.realm(realm).map(|realm| realm.base_grid)
}

/// Whether a site's definition is the one a name means.
fn site_def_is(sim: &Sim, site: &clash_world::Site, wanted: &str) -> bool {
    sim.defs
        .try_registry::<clash_gen::region::SiteDef>()
        .is_some_and(|names| {
            let name = names.name(site.def);
            name == wanted || name.rsplit(':').next() == Some(wanted)
        })
}

/// The places one selector names for a character, before its own spot
/// is taken: its home, or the anchors of the sites it answers to.
fn selector_places(sim: &Sim, whom: &Whom, sel: &PlaceSel) -> Vec<Location> {
    if sel.home {
        return vec![whom.home];
    }
    let Some(grid) = grid_of(sim, whom.realm) else {
        return Vec::new();
    };
    let mut found: Vec<(Real, u32, Location)> = sim
        .cosmos
        .sites()
        .filter(|site| site.realm == whom.realm)
        .filter(|site| sel.site_tag.as_ref().is_none_or(|tag| site.has_tag(tag)))
        .filter(|site| {
            sel.site
                .as_ref()
                .is_none_or(|wanted| site_def_is(sim, site, wanted))
        })
        .map(|site| {
            let at = Location::new(grid, site.anchor);
            let far = sim
                .cosmos
                .delta(whom.home, at)
                .map_or(Real::INFINITY, crate::ai::nav::metres);
            (far, site.id.raw(), at)
        })
        .collect();
    found.sort_by(|a, b| a.0.cmp(&b.0).then(a.1.cmp(&b.1)));
    match sel.pick {
        Pick::Nearest => found.first().map(|(_, _, at)| *at).into_iter().collect(),
        Pick::All => found.into_iter().map(|(_, _, at)| at).collect(),
        Pick::Any => {
            if found.is_empty() {
                return Vec::new();
            }
            found.sort_by_key(|(_, id, _)| *id);
            let hash = pos_hash(whom.seed, PICK_DOMAIN, 0, 0, 0, found.len() as u64);
            let at = (hash % found.len() as u64) as usize;
            found.get(at).map(|(_, _, at)| *at).into_iter().collect()
        }
    }
}

/// A character's own spot near a place: the place moved by up to
/// `spread` cells each way, drawn from its seed and the place.
fn spot(whom: &Whom, place: Location, spread: i32, index: usize) -> Location {
    if spread <= 0 {
        return place;
    }
    let side = (2 * spread + 1) as u64;
    let hash = pos_hash(
        whom.seed,
        SPOT_DOMAIN,
        place.cell.x,
        place.cell.y,
        place.cell.z,
        index as u64,
    );
    let dx = (hash % side) as i32 - spread;
    let dy = ((hash / side) % side) as i32 - spread;
    Location::new(
        place.grid,
        CellPos::new(place.cell.x + dx, place.cell.y + dy, place.cell.z),
    )
}

/// Every place a routine entry names for a character, each its own spot:
/// those of the first selector that names any.
pub fn entry_places(sim: &Sim, whom: &Whom, entry: &RoutineEntry) -> Vec<Location> {
    for sel in entry.at.selectors() {
        let found = selector_places(sim, whom, sel);
        if found.is_empty() {
            continue;
        }
        let spread = if sel.home { 0 } else { entry.spread };
        return found
            .into_iter()
            .enumerate()
            .map(|(index, place)| spot(whom, place, spread, index))
            .collect();
    }
    Vec::new()
}

/// The place a routine entry names for a character at a time since its
/// part began: the first of [`entry_places`], or on a `beat` the one it
/// has come to; its home if none names one.
pub fn entry_place(
    sim: &Sim,
    whom: &Whom,
    entry: &RoutineEntry,
    day: Day,
    since_began: Dur,
) -> Location {
    let places = entry_places(sim, whom, entry);
    beat_place(&places, entry, day, since_began).unwrap_or(whom.home)
}

/// Of the places a part names, the one a character keeps to at a time
/// since the part began: on a `beat`, each in turn; else the first.
pub fn beat_place(
    places: &[Location],
    entry: &RoutineEntry,
    day: Day,
    since_began: Dur,
) -> Option<Location> {
    let beat = calendar::scaled(entry.beat.dur(), day.length).as_micros();
    if beat <= 0 || places.len() < 2 {
        return places.first().copied();
    }
    let turn = since_began.as_micros().max(0) / beat;
    places.get((turn as usize) % places.len()).copied()
}

/// How long it takes to walk between two places while unloaded, at a
/// routine's speed.
pub fn travel_time(sim: &Sim, def: &RoutineDef, from: Location, to: Location) -> Dur {
    let straight = sim
        .cosmos
        .delta(from, to)
        .map_or(Real::ZERO, crate::ai::nav::metres);
    if straight <= Real::ZERO || def.speed <= Real::ZERO {
        return Dur::ZERO;
    }
    Dur::from_secs_real(straight * Real::of(TRAVEL_DETOUR) / def.speed)
}
