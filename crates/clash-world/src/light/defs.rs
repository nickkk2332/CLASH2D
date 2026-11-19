//! The definitions light is made from (15 §3.3): the `light` an element
//! or one of its states gives off, and the `sky` definition a stratum
//! names.
//!
//! Each has a form as mods write it ([`LightSpec`], [`SkyDef`]) and a
//! resolved form ([`LightSource`], [`Sky`]) built with the rest of the
//! world's definitions by [`WorldDefs::build`](crate::WorldDefs::build).

use clash_core::{Dur, Real, define_id};
use clash_data::Time;
use serde::{Deserialize, Serialize};

use super::{
    LIGHT_NEGLIGIBLE, LIGHT_RADIUS_MAX_M, SKY_DARK, SKY_REACH_MAX_CELLS, SKY_SPREAD_LOSS,
    SKY_SPREAD_REACH_CELLS, STEP_SIDE,
};
use crate::defs::{GlyphSpec, Length};
use crate::dims::CELL_M;

define_id! {
    /// A `sky` definition: the light a stratum's sky gives by time of day.
    pub struct SkyId(u16);
}

/// The `light` of a floor, block or fixture, or of one of its states, as
/// mods write it: `"light": { "intensity": 1.0, "radius": "8 m" }`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LightSpec {
    /// How bright it is in its own cell, on the scale where full daylight
    /// is 1. Negative for a source of darkness. Zero is no source: a
    /// state writes it to put the definition's light out.
    pub intensity: Real,
    /// How far it shines. Left out, it shines as far as its light stays
    /// above [`LIGHT_NEGLIGIBLE`], and never further than
    /// [`LIGHT_RADIUS_MAX_M`].
    #[serde(default)]
    pub radius: Option<Length>,
}

/// A resolved source of light: an element state that gives light off,
/// an area that carries one (02 §8.3), or a moving source handed in with
/// a query.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub struct LightSource {
    /// The light it gives its own cell. A cell `d` metres away, in its
    /// line of sight and within its radius, gets `intensity / (1 + d²)`.
    pub intensity: Real,
    /// How far it shines, in metres. At most [`LIGHT_RADIUS_MAX_M`].
    pub radius: Real,
}

clash_core::impl_state_hash!(LightSource { intensity, radius });

impl LightSource {
    /// A source of this intensity that shines as far as its light stays
    /// above [`LIGHT_NEGLIGIBLE`], and never further than
    /// [`LIGHT_RADIUS_MAX_M`].
    pub fn of(intensity: Real) -> LightSource {
        LightSource {
            intensity,
            radius: default_radius(intensity),
        }
    }

    /// The light this source gives a cell whose centre is `distance_sq`
    /// square metres from its own, if it reaches the cell at all:
    /// `intensity / (1 + d²)`.
    #[inline]
    pub fn at_distance_sq(&self, distance_sq: Real) -> Real {
        self.intensity / (Real::ONE + distance_sq)
    }

    /// The source as an element wrote it, or why it cannot be. `None`
    /// for an intensity of zero: no source.
    pub(crate) fn from_spec(spec: &LightSpec) -> Result<Option<LightSource>, String> {
        LightSource::shining(spec.intensity, spec.radius.map(|radius| radius.get()))
    }

    /// A source of an intensity and, if given, a radius in metres, or
    /// why it cannot be: an intensity that is not finite, a radius
    /// outside 0 to [`LIGHT_RADIUS_MAX_M`]. `None` for an intensity of
    /// zero: no source. A negative intensity darkens (15 §3.3).
    pub fn shining(intensity: Real, radius: Option<Real>) -> Result<Option<LightSource>, String> {
        if !intensity.is_finite() {
            return Err("the intensity must be finite".to_owned());
        }
        let radius = match radius {
            Some(radius) => {
                if radius < Real::ZERO || radius > LIGHT_RADIUS_MAX_M || !radius.is_finite() {
                    return Err(format!(
                        "the radius must be from 0 to {LIGHT_RADIUS_MAX_M} m, not {radius}"
                    ));
                }
                radius
            }
            None => default_radius(intensity),
        };
        if intensity == Real::ZERO {
            return Ok(None);
        }
        Ok(Some(LightSource { intensity, radius }))
    }

    /// The furthest whole cell the source reaches along an axis.
    pub(crate) fn reach_cells(&self) -> i32 {
        (self.radius / CELL_M)
            .floor()
            .clamp(Real::ZERO, LIGHT_RADIUS_MAX_M / CELL_M)
            .to_i32()
    }
}

/// How far a source shines when it does not say: to where
/// `|intensity| / (1 + d²)` falls to [`LIGHT_NEGLIGIBLE`].
fn default_radius(intensity: Real) -> Real {
    let ratio = intensity.abs() / LIGHT_NEGLIGIBLE;
    if ratio <= Real::ONE {
        return Real::ZERO;
    }
    (ratio - Real::ONE).sqrt().min(LIGHT_RADIUS_MAX_M)
}

/// One moment of a sky's day and the light it gives then.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SkyKeySpec {
    /// The time of day, from midnight.
    pub at: Time,
    /// The light the sky gives a cell open to it: 1 is full daylight.
    pub level: Real,
}

/// How a sky's light spreads from the cells open to it into the cells
/// that are not.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SkySpreadSpec {
    /// The fraction of the light lost for each cell it travels, 0 to 1.
    /// Default [`SKY_SPREAD_LOSS`].
    #[serde(default)]
    pub loss: Option<Real>,
    /// How many cells it travels at most, 0 to [`SKY_REACH_MAX_CELLS`].
    /// Default [`SKY_SPREAD_REACH_CELLS`].
    #[serde(default)]
    pub reach: Option<u32>,
}

/// A `sky` definition: the light a stratum's sky gives by time of day,
/// how that light spreads under cover, and a backdrop (15 §3.3, 12 §6.1).
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SkyDef {
    /// The length of a day. Default 24 hours. Until calendars are built
    /// (09 §14.2) this is where a realm's day comes from.
    #[serde(default)]
    pub day: Option<Time>,
    /// The time of day at which the realm's clock starts. Default
    /// midnight.
    #[serde(default)]
    pub starts_at: Option<Time>,
    /// The light by time of day, in order of time. Between two entries
    /// it runs in a straight line, and from the last round midnight to
    /// the first. One entry is a sky that never changes; none is a sky
    /// that gives no light.
    #[serde(default)]
    pub light: Vec<SkyKeySpec>,
    /// How the light spreads under cover.
    #[serde(default)]
    pub spread: Option<SkySpreadSpec>,
    /// What is drawn where a view looks down into nothing (12 §6.1). The
    /// world stores it and gives it no meaning.
    #[serde(default)]
    pub backdrop: Option<GlyphSpec>,
}

/// A resolved `sky` definition.
#[derive(Clone, Debug)]
pub struct Sky {
    /// The length of a day. More than nothing.
    pub day: Dur,
    /// The time of day at which the realm's clock starts.
    pub starts_at: Dur,
    /// The light by time of day, in order of time: each a time from
    /// midnight, less than a day, and the light then.
    pub keys: Vec<(Dur, Real)>,
    /// The fraction of spreading light lost for each cell.
    pub loss: Real,
    /// How many cells the light spreads at most.
    pub reach: i32,
    /// What is drawn where a view looks down into nothing, as written.
    pub backdrop: Option<GlyphSpec>,
    /// What is left of the sky's light after each number of spread
    /// steps, a step being half a cell.
    shares: Vec<Real>,
}

impl Sky {
    /// Builds a sky from its definition. Each mistake is given to
    /// `error` with the path of the field and what is wrong, and the
    /// sky is built with the nearest value that makes sense.
    pub(crate) fn from_def(def: &SkyDef, mut error: impl FnMut(&str, String)) -> Sky {
        let day = match def.day {
            Some(day) if day.dur() > Dur::ZERO => day.dur(),
            Some(_) => {
                error("day", "must be more than no time".to_owned());
                Dur::DAY
            }
            None => Dur::DAY,
        };
        let within_day = |time: Dur| time >= Dur::ZERO && time < day;
        let starts_at = match def.starts_at {
            Some(time) if within_day(time.dur()) => time.dur(),
            Some(_) => {
                error(
                    "starts_at",
                    "must be a time of day: from nothing to less than a day".to_owned(),
                );
                Dur::ZERO
            }
            None => Dur::ZERO,
        };
        let mut keys: Vec<(Dur, Real)> = Vec::with_capacity(def.light.len());
        for (position, key) in def.light.iter().enumerate() {
            let at = key.at.dur();
            let mut good = true;
            if !within_day(at) {
                error(
                    &format!("light.{position}.at"),
                    "must be a time of day: from nothing to less than a day".to_owned(),
                );
                good = false;
            } else if keys.last().is_some_and(|(before, _)| *before >= at) {
                error(
                    &format!("light.{position}.at"),
                    "the entries come in order of time, and no two at one time".to_owned(),
                );
                good = false;
            }
            if key.level < Real::ZERO || !key.level.is_finite() {
                error(
                    &format!("light.{position}.level"),
                    format!("must be zero or more, not {}", key.level),
                );
                good = false;
            }
            if good {
                keys.push((at, key.level));
            }
        }
        let spread = def.spread.unwrap_or(SkySpreadSpec {
            loss: None,
            reach: None,
        });
        let loss = match spread.loss {
            Some(loss) if loss >= Real::ZERO && loss <= Real::ONE => loss,
            Some(loss) => {
                error("spread.loss", format!("must be from 0 to 1, not {loss}"));
                SKY_SPREAD_LOSS
            }
            None => SKY_SPREAD_LOSS,
        };
        let reach = match spread.reach {
            Some(reach) => match i32::try_from(reach) {
                Ok(reach) if reach <= SKY_REACH_MAX_CELLS => reach,
                _ => {
                    error(
                        "spread.reach",
                        format!("must be from 0 to {SKY_REACH_MAX_CELLS} cells, not {reach}"),
                    );
                    SKY_SPREAD_REACH_CELLS
                }
            },
            None => SKY_SPREAD_REACH_CELLS,
        };
        Sky {
            day,
            starts_at,
            keys,
            loss,
            reach,
            backdrop: def.backdrop.clone(),
            shares: spread_shares(loss, reach),
        }
    }

    /// The time of day when the realm's clock reads `local`: how long
    /// since the last midnight.
    pub fn time_of_day(&self, local: Dur) -> Dur {
        let day = self.day.as_micros().max(1);
        let since = local
            .as_micros()
            .rem_euclid(day)
            .saturating_add(self.starts_at.as_micros())
            .rem_euclid(day);
        Dur::from_micros(since)
    }

    /// The light the sky gives a cell open to it when the realm's clock
    /// reads `local`.
    pub fn level_at(&self, local: Dur) -> Real {
        self.level_at_time_of_day(self.time_of_day(local))
    }

    /// The light the sky gives a cell open to it at a time of day: on
    /// the straight line between the entries either side of that time,
    /// going round midnight from the last entry to the first.
    pub fn level_at_time_of_day(&self, time: Dur) -> Real {
        let (Some(&first), Some(&last)) = (self.keys.first(), self.keys.last()) else {
            return Real::ZERO;
        };
        let after = self.keys.partition_point(|(at, _)| *at <= time);
        // The entry at or before the time, and the one after it. Before
        // the first entry and after the last, the two are the last and
        // the first, a day apart.
        let (from, to) = match (after.checked_sub(1), self.keys.get(after)) {
            (Some(before), Some(&next)) => (self.keys[before], next),
            (Some(_), None) => (last, (first.0 + self.day, first.1)),
            (None, _) => ((last.0 - self.day, last.1), first),
        };
        let span = to.0 - from.0;
        if span <= Dur::ZERO {
            return from.1;
        }
        let along = Real::from_i64((time - from.0).as_micros()) / Real::from_i64(span.as_micros());
        from.1 + (to.1 - from.1) * along
    }

    /// What is left of the sky's light in a cell `steps` spread steps
    /// from the open sky, as [`StaticLight::sky_steps`](super::StaticLight)
    /// gives them: 1 under the open sky, less with each cell, and nothing
    /// past the sky's reach.
    #[inline]
    pub fn share(&self, steps: u8) -> Real {
        match self.shares.get(usize::from(steps)) {
            Some(share) => *share,
            None => Real::ZERO,
        }
    }
}

/// What is left of spreading light after each number of steps, a step
/// being half a cell: `(1 − loss)` for each whole cell, and the square
/// root of that for the odd half. Nothing past `reach` cells.
fn spread_shares(loss: Real, reach: i32) -> Vec<Real> {
    let keep = Real::ONE - loss;
    let half = keep.sqrt();
    let steps = usize::try_from(reach).unwrap_or(0) * usize::from(STEP_SIDE);
    let steps = steps.min(usize::from(SKY_DARK) - 1);
    let mut shares = Vec::with_capacity(steps + 1);
    let mut whole = Real::ONE;
    for step in 0..=steps {
        if step % usize::from(STEP_SIDE) == 0 {
            if step > 0 {
                whole *= keep;
            }
            shares.push(whole);
        } else {
            shares.push(whole * half);
        }
    }
    shares
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hours(hours: i64) -> Time {
        Time(Dur::from_secs(hours * 3600))
    }

    fn day_night() -> Sky {
        let def = SkyDef {
            light: vec![
                SkyKeySpec {
                    at: hours(5),
                    level: Real::of(0.0),
                },
                SkyKeySpec {
                    at: hours(7),
                    level: Real::of(1.0),
                },
                SkyKeySpec {
                    at: hours(17),
                    level: Real::of(1.0),
                },
                SkyKeySpec {
                    at: hours(19),
                    level: Real::of(0.0),
                },
            ],
            starts_at: Some(hours(6)),
            ..SkyDef::default()
        };
        Sky::from_def(&def, |path, message| panic!("{path}: {message}"))
    }

    #[test]
    fn a_sky_runs_in_straight_lines_between_its_entries_and_round_midnight() {
        let sky = day_night();
        let at = |h: i64| sky.level_at_time_of_day(Dur::from_secs(h * 3600));
        assert_eq!(at(12), Real::ONE);
        assert_eq!(at(0), Real::ZERO);
        assert_eq!(at(23), Real::ZERO);
        assert_eq!(at(6), Real::of(0.5));
        assert_eq!(at(18), Real::of(0.5));
        assert_eq!(at(5), Real::ZERO);
        // The clock starts at six in the morning, and days repeat.
        assert_eq!(sky.time_of_day(Dur::ZERO), Dur::from_secs(6 * 3600));
        assert_eq!(sky.level_at(Dur::ZERO), Real::of(0.5));
        assert_eq!(sky.level_at(Dur::from_secs(6 * 3600)), Real::ONE);
        assert_eq!(sky.level_at(Dur::from_secs((6 + 48) * 3600)), Real::ONE);
        assert_eq!(sky.level_at(Dur::from_secs(-6 * 3600)), Real::ZERO);
    }

    #[test]
    fn a_sky_of_one_entry_never_changes_and_of_none_is_dark() {
        let constant = Sky::from_def(
            &SkyDef {
                light: vec![SkyKeySpec {
                    at: hours(3),
                    level: Real::of(0.4),
                }],
                ..SkyDef::default()
            },
            |path, message| panic!("{path}: {message}"),
        );
        for h in [0, 3, 12, 23] {
            assert_eq!(
                constant.level_at(Dur::from_secs(h * 3600)),
                Real::of(0.4),
                "{h}"
            );
        }
        let dark = Sky::from_def(&SkyDef::default(), |path, message| {
            panic!("{path}: {message}")
        });
        assert_eq!(dark.level_at(Dur::from_secs(12 * 3600)), Real::ZERO);
    }

    #[test]
    fn spreading_light_loses_a_fixed_fraction_for_each_cell() {
        let sky = day_night();
        assert_eq!(sky.loss, SKY_SPREAD_LOSS);
        assert_eq!(sky.reach, SKY_SPREAD_REACH_CELLS);
        assert_eq!(sky.share(0), Real::ONE);
        assert_eq!(sky.share(2), Real::of(0.75));
        assert_eq!(sky.share(4), Real::of(0.75 * 0.75));
        assert!(sky.share(1) < Real::ONE && sky.share(1) > sky.share(2));
        let last = u8::try_from(SKY_SPREAD_REACH_CELLS * 2).unwrap();
        assert!(sky.share(last) > Real::ZERO);
        assert_eq!(sky.share(last + 1), Real::ZERO);
        assert_eq!(sky.share(SKY_DARK), Real::ZERO);
    }

    #[test]
    fn mistakes_in_a_sky_are_reported_with_their_paths() {
        let def = SkyDef {
            day: Some(Time(Dur::ZERO)),
            starts_at: Some(hours(30)),
            light: vec![
                SkyKeySpec {
                    at: hours(7),
                    level: Real::of(1.0),
                },
                SkyKeySpec {
                    at: hours(7),
                    level: Real::of(-1.0),
                },
                SkyKeySpec {
                    at: hours(25),
                    level: Real::of(1.0),
                },
            ],
            spread: Some(SkySpreadSpec {
                loss: Some(Real::of(1.5)),
                reach: Some(99),
            }),
            backdrop: None,
        };
        let mut paths = Vec::new();
        let sky = Sky::from_def(&def, |path, _| paths.push(path.to_owned()));
        assert_eq!(
            paths,
            [
                "day",
                "starts_at",
                "light.1.at",
                "light.1.level",
                "light.2.at",
                "spread.loss",
                "spread.reach"
            ]
        );
        assert_eq!(sky.keys.len(), 1);
    }

    #[test]
    fn a_source_that_gives_no_radius_shines_while_its_light_matters() {
        let lamp = LightSource::of(Real::ONE);
        // 1 / (1 + d²) = 0.01 at d = √99.
        assert_eq!(lamp.radius, Real::of(99.0).sqrt());
        assert_eq!(lamp.reach_cells(), 9);
        assert_eq!(LightSource::of(Real::of(1000.0)).radius, LIGHT_RADIUS_MAX_M);
        assert_eq!(LightSource::of(Real::of(-1.0)).radius, lamp.radius);
        assert_eq!(LightSource::of(Real::of(0.005)).radius, Real::ZERO);
        assert_eq!(lamp.at_distance_sq(Real::of(4.0)), Real::of(0.2));
        let spec = |intensity: f64, radius: Option<f64>| LightSpec {
            intensity: Real::of(intensity),
            radius: radius.map(|radius| Length(Real::of(radius))),
        };
        assert_eq!(LightSource::from_spec(&spec(0.0, None)), Ok(None));
        assert_eq!(
            LightSource::from_spec(&spec(2.0, Some(5.0))),
            Ok(Some(LightSource {
                intensity: Real::of(2.0),
                radius: Real::of(5.0)
            }))
        );
        assert!(LightSource::from_spec(&spec(2.0, Some(17.0))).is_err());
        assert!(LightSource::from_spec(&spec(2.0, Some(-1.0))).is_err());
    }
}
