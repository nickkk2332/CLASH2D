//! Light: how bright each cell is (15 §3.3).
//!
//! | Need | Use |
//! |------|-----|
//! | The light at one cell | [`Cosmos::light`] |
//! | The light of every cell of a box: a screen, a field of view | [`Cosmos::fill_light`] |
//! | Carried lamps, flashes and other sources that move | a list of [`MovingLight`] handed to either, or [`Cosmos::add_moving_light`] over a level already read |
//! | The parts that do not change with the time of day | [`Cosmos::static_light`], a [`StaticLight`] |
//! | The moment light is read at | [`SkyTime`]; [`SkyTime::now`] reads it from the timeline |
//! | The sky over a cell: its levels, its backdrop | [`Cosmos::sky`], a [`Sky`] |
//! | What gives light | `light` on a floor, block or fixture or one of its states ([`LightSpec`]); a `sky` definition ([`SkyDef`]) named by a stratum |
//! | What the cache has done | [`Cosmos::light_stats`] |
//!
//! # The scale
//!
//! A light level is a [`Real`] from 0, dark, upward, with no upper end.
//! 1 is what a cell open to the sky gets from a sky at level 1: full
//! daylight, by convention. A source of intensity 1 gives its own cell
//! 1 and a cell `d` metres away `1 / (1 + d²)`.
//!
//! # What makes up the light of a cell
//!
//! - **The sky.** The stratum's `sky` gives a level by the time of day.
//!   A cell open to the sky gets all of it. From those cells it spreads
//!   through cells that hold nothing opaque standing taller than
//!   [`LIGHT_EYE_MM`]: windows, open doors and holes alike. It loses a
//!   fixed fraction for each cell and goes no further than the sky's
//!   reach. A step to a diagonal neighbour counts as a cell and a half
//!   and needs both cells beside the corner clear; a step between
//!   storeys counts as three cells and needs the upper cell to have no
//!   floor. A cell that stops light is lit by its neighbours and passes
//!   nothing on: one sees the wall of a lit room.
//! - **Fixed sources.** Each element state that gives light adds
//!   `intensity / (1 + d²)` to every cell it sees within its radius. It
//!   sees what an eye in its cell would at [`LIGHT_EYE_MM`]
//!   ([`Cosmos::field_of_view`]). An intensity may be negative.
//! - **Moving sources**, handed in with the question, add in the same
//!   way.
//!
//! The first two never go below zero together, and nor does the whole.
//!
//! # Time
//!
//! The world does not hold the timeline, so the moment is a parameter: a
//! [`SkyTime`] carries the local time of the realm's clock, which
//! `Timeline::local_now(realm.clock)` gives. The scene may ask for the
//! light of the moment it is playing back.
//!
//! # The cache
//!
//! How far each cell is from the open sky and what the fixed sources
//! give it are kept for each chunk asked about, in a table beside the
//! chunks. Neither depends on the time of day. A chunk's entry is
//! worked out again only when cells within light's reach of it change in
//! a way light cares about: an edit elsewhere, or one that moves nothing
//! opaque, no floor and no source, costs nothing. The cache is derived:
//! it is not saved and not hashed, and a clone of the cosmos starts
//! with none.
//!
//! # Truth
//!
//! Light is a fact about the world. Nothing here decides for a
//! character: perception reads it to work out what a character receives
//! (15 §3.1), and what the character then believes is its own.
//!
//! # Unloaded space
//!
//! [`Cosmos::light`] of a cell whose tract is not loaded is the error
//! every read gives (02 §7.1). [`Cosmos::fill_light`] writes zero for
//! such cells. Light neither comes out of nor passes through space that
//! is not loaded.

mod cache;
mod defs;
mod solve;

#[cfg(test)]
mod tests;

use std::sync::{Mutex, MutexGuard};

use clash_core::{Dur, IVec3, Real};
use clash_time::Timeline;

pub(crate) use cache::LightCache;
pub use cache::{LIGHT_CACHE_MAX_CHUNKS, LIGHT_SOURCES_MAX_PER_CHUNK};
pub use defs::{LightSource, LightSpec, Sky, SkyDef, SkyId, SkyKeySpec, SkySpreadSpec};

use crate::cosmos::{Cosmos, GridView};
use crate::defs::RealmKind;
use crate::dims::{CELL_M, CHUNK_AREA, CHUNK_XY_BITS, CHUNK_XY_MASK, CHUNK_Z_MASK};
use crate::error::WorldError;
use crate::pos::{CellBox, CellPos, GridId, Location, TractPos, distance_sq_cells};
use crate::query::FovSet;

/// The height above the base of its cell that light travels at, in
/// millimetres: half a storey. Something opaque that stands no taller
/// than this casts no shadow and does not stop the sky's light.
pub const LIGHT_EYE_MM: u16 = 1500;

/// The furthest a sky's light may be given to spread from the cells open
/// to it, in cells.
pub const SKY_REACH_MAX_CELLS: i32 = 16;

/// How far a sky's light spreads when its definition does not say, in
/// cells.
pub const SKY_SPREAD_REACH_CELLS: i32 = 8;

/// The fraction of a sky's spreading light lost for each cell when its
/// definition does not say.
pub const SKY_SPREAD_LOSS: Real = Real::of(0.25);

/// The widest a source of light may shine, in metres.
pub const LIGHT_RADIUS_MAX_M: Real = Real::of(16.0);

/// A source that gives no radius shines as far as its light stays above
/// this.
pub const LIGHT_NEGLIGIBLE: Real = Real::of(0.01);

/// The most cells one [`Cosmos::fill_light`] fills.
pub const LIGHT_FILL_MAX_CELLS: u64 = 1 << 20;

/// What a step of spreading sky light to a neighbour on the same storey
/// costs, in spread steps: a spread step is half a cell.
pub const STEP_SIDE: u8 = 2;

/// What a step to a diagonal neighbour costs: a cell and a half.
pub const STEP_DIAGONAL: u8 = 3;

/// What a step to the storey above or below costs: three cells, as a
/// storey is three metres.
pub const STEP_STOREY: u8 = 6;

/// The number of spread steps that means the sky's light does not reach
/// a cell at all.
pub const SKY_DARK: u8 = u8::MAX;

/// The moment light is read at: what the sky is doing.
///
/// ```
/// use clash_core::{Dur, Real};
/// use clash_world::SkyTime;
///
/// let noon = SkyTime::at(Dur::from_secs(12 * 3600));
/// assert_eq!(noon.share, Real::ONE);
/// assert_eq!(noon.dimmed(Real::of(0.5)).share, Real::of(0.5));
/// ```
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct SkyTime {
    /// The local time of the realm's clock: what
    /// `Timeline::local_now(realm.clock)` gives. The sky's definition
    /// turns it into a time of day.
    pub local: Dur,
    /// The share of the sky's light that gets through: 1 for a clear
    /// sky. This is where the weather's `light` (09 §14.1) goes.
    pub share: Real,
}

impl SkyTime {
    /// A clear sky when the realm's clock reads `local`.
    pub const fn at(local: Dur) -> SkyTime {
        SkyTime {
            local,
            share: Real::ONE,
        }
    }

    /// The same moment under a sky that lets this share of its light
    /// through. A share below zero counts as none.
    pub fn dimmed(self, share: Real) -> SkyTime {
        SkyTime {
            share: share.max(Real::ZERO),
            ..self
        }
    }

    /// Now, under a clear sky, in the realm a grid belongs to: the
    /// local time of that realm's clock, read from the timeline.
    pub fn now(cosmos: &Cosmos, timeline: &Timeline, grid: GridId) -> Result<SkyTime, WorldError> {
        let clock = cosmos.grid(grid)?.clock();
        Ok(SkyTime::at(timeline.local_now(clock)?))
    }
}

/// A source of light that is not part of the cells: a carried lamp, a
/// muzzle flash, a glowing creature. Whoever asks for light supplies the
/// ones that matter; the world keeps none.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct MovingLight {
    /// The cell the source is in.
    pub at: Location,
    /// How bright it is and how far it shines.
    pub source: LightSource,
}

impl MovingLight {
    /// A source in a cell with an intensity, shining as far as its
    /// light matters ([`LightSource::of`]).
    pub fn new(at: Location, intensity: Real) -> MovingLight {
        MovingLight {
            at,
            source: LightSource::of(intensity),
        }
    }
}

/// The light of a cell that does not change with the time of day: what
/// the cache keeps.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct StaticLight {
    /// How far the cell is from the open sky, in spread steps of half a
    /// cell: 0 open to the sky, [`STEP_SIDE`] for each cell sideways,
    /// [`STEP_DIAGONAL`] diagonally, [`STEP_STOREY`] between storeys,
    /// and [`SKY_DARK`] where the sky's light does not reach.
    /// [`Sky::share`] turns it into the share of the sky's light.
    pub sky_steps: u8,
    /// What the fixed sources give the cell. Negative where sources of
    /// darkness outweigh the rest.
    pub fixed: Real,
}

/// What the light cache has done since the cosmos was made.
#[derive(Clone, Copy, PartialEq, Eq, Default, Debug)]
pub struct LightStats {
    /// How many times the light of a chunk was worked out: once when it
    /// is first asked about, and again each time cells within reach of
    /// it change in a way light cares about.
    pub chunks_lit: u64,
    /// How many times the cells of a chunk were read to see what they
    /// look like to light.
    pub shapes_read: u64,
    /// How many chunks have their light kept now.
    pub cached_chunks: usize,
}

/// The cosmos's light cache, behind a lock so that reads of light, which
/// take the cosmos by shared reference, can fill it. It is derived data:
/// a clone starts empty, and it is neither saved nor hashed.
pub(crate) struct LightSlot(Mutex<LightCache>);

impl LightSlot {
    pub(crate) fn new() -> LightSlot {
        LightSlot(Mutex::new(LightCache::new()))
    }

    /// The cache, for a question. A lock left poisoned by a panic
    /// elsewhere is taken with the cache emptied.
    fn lock(&self) -> MutexGuard<'_, LightCache> {
        match self.0.lock() {
            Ok(cache) => cache,
            Err(poisoned) => {
                let mut cache = poisoned.into_inner();
                *cache = LightCache::new();
                cache
            }
        }
    }

    /// The cache, for whoever holds the cosmos to change it.
    pub(crate) fn get_mut(&mut self) -> &mut LightCache {
        match self.0.get_mut() {
            Ok(cache) => cache,
            Err(poisoned) => {
                let cache = poisoned.into_inner();
                *cache = LightCache::new();
                cache
            }
        }
    }
}

impl Clone for LightSlot {
    fn clone(&self) -> LightSlot {
        LightSlot::new()
    }
}

impl core::fmt::Debug for LightSlot {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str("LightSlot")
    }
}

/// The sky over a storey of a realm, if its stratum names one.
fn sky_of<'a>(cosmos: &'a Cosmos, kind: Option<&RealmKind>, z: i32) -> Option<&'a Sky> {
    let id = kind?.stratum_at(z)?.sky_id?;
    cosmos.defs().skies().get(id)
}

/// The light a sky gives a cell open to it at a moment.
fn sky_level(sky: Option<&Sky>, time: SkyTime) -> Real {
    sky.map_or(Real::ZERO, |sky| sky.level_at(time.local) * time.share)
}

impl Cosmos {
    /// The sky over a cell: the `sky` definition its stratum names, if
    /// it names one. The cell must be inside its grid; its tract need
    /// not be loaded.
    pub fn sky(&self, loc: Location) -> Result<Option<&Sky>, WorldError> {
        let view = self.grid(loc.grid)?;
        let cell = view.normalize(loc.cell)?;
        Ok(sky_of(self, self.realm_kind(view.realm()), cell.z))
    }

    /// The light at a cell (15 §3.3): what the sky gives it at this
    /// moment and what the fixed sources give it, never below zero. See
    /// the [module](crate::light) for the scale and the rules.
    ///
    /// An error if the cell lies beyond an edge of its grid or its tract
    /// is not loaded. For many cells use [`fill_light`](Cosmos::fill_light),
    /// which costs far less for each.
    pub fn light(&self, loc: Location, time: SkyTime) -> Result<Real, WorldError> {
        let view = self.grid(loc.grid)?;
        let cell = loaded_cell(&view, loc.cell)?;
        let fixed = {
            let mut cache = self.light.lock();
            cache.begin();
            cache.lit(self, &view, cell.chunk()).at(cell.idx())
        };
        let sky = sky_of(self, self.realm_kind(view.realm()), cell.z);
        Ok(combine(sky, sky_level(sky, time), fixed.0, fixed.1))
    }

    /// The light at a cell with moving sources counted:
    /// [`light`](Cosmos::light), then
    /// [`add_moving_light`](Cosmos::add_moving_light).
    pub fn light_with(
        &self,
        loc: Location,
        time: SkyTime,
        moving: &[MovingLight],
    ) -> Result<Real, WorldError> {
        let level = self.light(loc, time)?;
        self.add_moving_light(loc, level, moving)
    }

    /// Adds what moving sources give a cell to a level already read for
    /// it, and returns the sum, never below zero. Each source in the
    /// cell's grid that sees the cell within its radius adds
    /// `intensity / (1 + d²)`, in the order of the list. A source sees
    /// what an eye in its cell would at [`LIGHT_EYE_MM`], as a fixed
    /// source does. The cache is not touched.
    ///
    /// An error if the cell lies beyond an edge of its grid or its tract
    /// is not loaded. A source in space that is not loaded gives
    /// nothing.
    pub fn add_moving_light(
        &self,
        loc: Location,
        level: Real,
        moving: &[MovingLight],
    ) -> Result<Real, WorldError> {
        let view = self.grid(loc.grid)?;
        let cell = loaded_cell(&view, loc.cell)?;
        let mut sum = level;
        let mut fov = FovSet::new();
        for light in moving {
            if light.at.grid != loc.grid || light.source.intensity == Real::ZERO {
                continue;
            }
            let Ok(from) = view.normalize(light.at.cell) else {
                continue;
            };
            let away = view.topology().delta(from, cell);
            if !within(&light.source, away)
                || self
                    .field_of_view(light.at, LIGHT_EYE_MM, light.source.radius, &mut fov)
                    .is_err()
                || !fov.contains_offset(away)
            {
                continue;
            }
            sum += light.source.at_distance_sq(distance_sq(away));
        }
        Ok(sum.max(Real::ZERO))
    }

    /// The parts of a cell's light that do not change with the time of
    /// day: how far it is from the open sky and what the fixed sources
    /// give it. This is what the cache keeps.
    pub fn static_light(&self, loc: Location) -> Result<StaticLight, WorldError> {
        let view = self.grid(loc.grid)?;
        let cell = loaded_cell(&view, loc.cell)?;
        let mut cache = self.light.lock();
        cache.begin();
        let (sky_steps, fixed) = cache.lit(self, &view, cell.chunk()).at(cell.idx());
        Ok(StaticLight { sky_steps, fixed })
    }

    /// Fills `out` with the light of every cell of a box of one grid,
    /// in the order of [`CellBox::cells`]: storey by storey, row by row.
    /// For a screen or a field of view the box is one storey. `out` is
    /// emptied first and keeps its storage, so a buffer kept between
    /// calls allocates nothing.
    ///
    /// Each cell gets exactly what [`light_with`](Cosmos::light_with)
    /// gives it, at a small part of the cost. A cell that lies beyond an
    /// edge of the grid or whose tract is not loaded gets zero. On a
    /// wrapped axis the box may run past the seam.
    ///
    /// An error if there is no such grid, or the box holds more than
    /// [`LIGHT_FILL_MAX_CELLS`] cells; `out` is then empty.
    pub fn fill_light(
        &self,
        grid: GridId,
        cells: CellBox,
        time: SkyTime,
        moving: &[MovingLight],
        out: &mut Vec<Real>,
    ) -> Result<(), WorldError> {
        out.clear();
        let view = self.grid(grid)?;
        if cells.volume() > LIGHT_FILL_MAX_CELLS {
            return Err(WorldError::TooLarge {
                what: "a box of cells to fill with light",
                limit: LIGHT_FILL_MAX_CELLS,
            });
        }
        out.resize(cells.volume() as usize, Real::ZERO);
        let kind = self.realm_kind(view.realm());
        let width = (cells.max.x - cells.min.x + 1) as usize;
        let rows = (cells.max.y - cells.min.y + 1) as usize;

        {
            let mut cache = self.light.lock();
            cache.begin();
            for (layer, z) in (cells.min.z..=cells.max.z).enumerate() {
                let sky = sky_of(self, kind, z);
                let level = sky_level(sky, time);
                let lz = (z & CHUNK_Z_MASK) as usize;
                // The box cut along the sides of chunks.
                for chunk_y in (cells.min.y >> CHUNK_XY_BITS)..=(cells.max.y >> CHUNK_XY_BITS) {
                    for chunk_x in (cells.min.x >> CHUNK_XY_BITS)..=(cells.max.x >> CHUNK_XY_BITS) {
                        let from = CellPos::new(
                            cells.min.x.max(chunk_x << CHUNK_XY_BITS),
                            cells.min.y.max(chunk_y << CHUNK_XY_BITS),
                            z,
                        );
                        let to_x = cells.max.x.min((chunk_x << CHUNK_XY_BITS) + CHUNK_XY_MASK);
                        let to_y = cells.max.y.min((chunk_y << CHUNK_XY_BITS) + CHUNK_XY_MASK);
                        let Ok(first) = loaded_cell(&view, from) else {
                            continue;
                        };
                        let lit = cache.lit(self, &view, first.chunk());
                        let steps = lit.sky_storey(lz);
                        let fixed = lit.fixed_storey(lz);
                        // One value for the whole piece: open ground, or
                        // deep rock.
                        let alike = match (steps, fixed) {
                            (Ok(all), None) => Some(combine(sky, level, all, Real::ZERO)),
                            _ => None,
                        };
                        for y in from.y..=to_y {
                            let row = (layer * rows + (y - cells.min.y) as usize) * width;
                            if let Some(alike) = alike {
                                let start = row + (from.x - cells.min.x) as usize;
                                let end = row + (to_x - cells.min.x) as usize;
                                out[start..=end].fill(alike);
                                continue;
                            }
                            let line = ((y & CHUNK_XY_MASK) << CHUNK_XY_BITS) as usize;
                            for x in from.x..=to_x {
                                let at = (line + (x & CHUNK_XY_MASK) as usize) & (CHUNK_AREA - 1);
                                let steps = match steps {
                                    Ok(all) => all,
                                    Err(each) => each[at],
                                };
                                let fixed = fixed.map_or(Real::ZERO, |fixed| fixed[at]);
                                out[row + (x - cells.min.x) as usize] =
                                    combine(sky, level, steps, fixed);
                            }
                        }
                    }
                }
            }
        }

        // Moving sources, each over the part of the box it reaches.
        let mut fov = FovSet::new();
        let mut touched = false;
        for light in moving {
            if light.at.grid != grid || light.source.intensity == Real::ZERO {
                continue;
            }
            let Ok(from) = view.normalize(light.at.cell) else {
                continue;
            };
            // The source's cell in the coordinates of the box, which on
            // a wrapped axis may be a turn away from where it is.
            let middle = CellPos::new(
                cells.min.x + (cells.max.x - cells.min.x) / 2,
                cells.min.y + (cells.max.y - cells.min.y) / 2,
                cells.min.z + (cells.max.z - cells.min.z) / 2,
            );
            let Ok(middle_in) = view.normalize(middle) else {
                continue;
            };
            let near = middle.offset(view.topology().delta(middle_in, from));
            let reach = light.source.reach_cells();
            let reach_box = CellBox::around(
                near,
                IVec3::new(reach, reach, reach / crate::dims::STOREY_CELLS as i32),
            );
            let Some(part) = cells.intersection(reach_box) else {
                continue;
            };
            if self
                .field_of_view(light.at, LIGHT_EYE_MM, light.source.radius, &mut fov)
                .is_err()
            {
                continue;
            }
            for z in part.min.z..=part.max.z {
                let layer = (z - cells.min.z) as usize;
                for y in part.min.y..=part.max.y {
                    let row = (layer * rows + (y - cells.min.y) as usize) * width;
                    for x in part.min.x..=part.max.x {
                        // A view holds no cell that is not loaded.
                        let away = CellPos::new(x, y, z) - near;
                        if !within(&light.source, away) || !fov.contains_offset(away) {
                            continue;
                        }
                        out[row + (x - cells.min.x) as usize] +=
                            light.source.at_distance_sq(distance_sq(away));
                        touched = true;
                    }
                }
            }
        }
        if touched {
            for level in out.iter_mut() {
                *level = (*level).max(Real::ZERO);
            }
        }
        Ok(())
    }

    /// What the light cache has done so far.
    pub fn light_stats(&self) -> LightStats {
        self.light.lock().stats()
    }

    /// Drops everything the light cache holds. Nothing needs this to
    /// get right answers: it frees the memory.
    pub fn clear_light(&self) {
        self.light.lock().clear();
    }

    /// Tells the light cache that a tract was loaded or unloaded, so
    /// that its cells are read again even if their revisions look
    /// unchanged (02 §22).
    pub(crate) fn light_tract_reloaded(&mut self, grid: GridId, tract: TractPos) {
        self.light.get_mut().tract_reloaded(grid, tract);
    }
}

/// A cell brought inside its grid, if its tract is loaded.
#[inline]
fn loaded_cell(view: &GridView<'_>, cell: CellPos) -> Result<CellPos, WorldError> {
    let cell = view.normalize(cell)?;
    match view.column(cell.tract()) {
        Some(_) => Ok(cell),
        None => Err(WorldError::NotLoaded(Location::new(view.id(), cell))),
    }
}

/// The static light of a cell at a moment: the sky's level by what
/// reaches the cell, and what the fixed sources give, never below zero.
#[inline]
fn combine(sky: Option<&Sky>, level: Real, steps: u8, fixed: Real) -> Real {
    let from_sky = match sky {
        Some(sky) if steps != SKY_DARK => level * sky.share(steps),
        _ => Real::ZERO,
    };
    (from_sky + fixed).max(Real::ZERO)
}

/// The square of the distance between the centres of two cells an
/// offset apart, in square metres.
#[inline]
fn distance_sq(away: IVec3) -> Real {
    Real::from_i64(distance_sq_cells(away)) * CELL_M * CELL_M
}

/// Whether a cell an offset away from a source lies within its radius.
#[inline]
fn within(source: &LightSource, away: IVec3) -> bool {
    distance_sq(away) <= source.radius * source.radius
}
