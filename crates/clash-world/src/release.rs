//! Energy releases (02 §13): explosions, shockwaves, blasts and beams.
//!
//! [`WorldCtx::release`] is the single entry point. A release is a
//! process of the world on the clock of its origin. Its front spreads
//! on the timeline at its speed, one shell of cells a metre thick at
//! each wake, so that something faster than the front can leave before
//! it arrives.
//!
//! - **Intensity** at a distance is the energy spread over the volume
//!   swept: `E / (4/3 · π · r³)`, for `r` of half a metre or more. A
//!   cone divides by its share of the sphere. The release reaches out to
//!   where that falls to [`WORLD_MIN_INTENSITY`].
//! - **Shadowing.** The release keeps how much of itself still gets
//!   through in each direction: a cube map of bins about the origin, so
//!   that no trigonometry is needed. A cell receives the intensity of
//!   its distance times what gets through toward it. What survives
//!   closes its share of the directions it covers; what is destroyed
//!   lets through the share of the packet that passed.
//! - **Packets.** Each element of a reached cell receives one packet at
//!   that intensity, with the energy the wave carries through the
//!   element's share of the cell. Each occupant gets `OccupantExposed`
//!   with the same intensity, once, and is thrown if the wave gives it
//!   speed enough.
//! - **Confinement.** What the wave could not mark turns it back. If
//!   the space round the origin is closed, that energy stays in it and
//!   strikes everything in the space and round it once more, evenly.
//!   If the space is open, it is gone.
//! - **A beam** marches along its axis a metre at a time, taking from
//!   its energy what each thing in its way costs, until something stops
//!   it, its energy is spent or its range is reached.
//!
//! Whatever happens, no more energy is absorbed than was released
//! (invariant W8), and a release whose front would visit more cells than
//! the budget is cut off there and says so.

use clash_core::{Dur, EntityId, IVec3, Real, StateHash, StateHasher, define_id};
use clash_time::Next;
use serde::{Deserialize, Serialize};

use crate::body::{BODY_ASSUMED_DENSITY, Body, BodyId, BodyOwner};
use crate::cause::{Cause, CauseKind};
use crate::cosmos::Cosmos;
use crate::damage::{DamagePacket, DamageResult};
use crate::defs::{DamageTypeId, ElementState, Layer};
use crate::dims::{CELL_M, STOREY_M};
use crate::edit::WorldCtx;
use crate::error::WorldError;
use crate::event::WorldEvent;
use crate::occupancy::{OccupantFilter, OccupantKind, distinct_ids};
use crate::pos::{CellPos, Location};
use crate::process::WorldProcess;

define_id! {
    /// An energy release. Never reused within one cosmos.
    pub struct ReleaseId;
}

/// The intensity, in pascals, at which a release ends: the default of
/// the setting `world.min_intensity` (02 §13.1).
pub const WORLD_MIN_INTENSITY: Real = Real::of(5.0e3);

/// The most cells the front of one release may visit: the default of
/// the setting `release.max_cells`. A release that would visit more is
/// cut off at the shell that would pass it.
pub const RELEASE_MAX_CELLS: u32 = 2_000_000;

/// The speed, in m/s, of the front of a release in vacuum that states
/// none: the default of the setting `release.vacuum_speed`.
pub const RELEASE_VACUUM_SPEED: Real = Real::of(1000.0);

/// The nearest to its origin, in metres, that a release's intensity is
/// reckoned: within this it is what it is at this distance.
pub const RELEASE_MIN_RADIUS_M: Real = Real::of(0.5);

/// How deep a slab of the wave, in metres, gives its energy to moving
/// what the wave passes: the thing takes the energy held in a slab of
/// its own cross-section and this depth. The default of the setting
/// `release.push_depth`.
pub const RELEASE_PUSH_DEPTH_M: Real = Real::of(0.01);

/// The speed, in m/s, at or above which what a wave moves is thrown and
/// becomes a body in flight: the default of the setting
/// `release.throw_speed`, and the `launch_at` of 07 §7.
pub const RELEASE_THROW_SPEED: Real = Real::of(3.0);

/// How high above the base of its cell an occupant thrown by a wave
/// starts its flight, in metres.
pub const RELEASE_THROW_ALT_M: Real = Real::of(1.0);

/// The most bins along a side of a face of the transmission map.
pub const RELEASE_MAX_BINS: u32 = 48;

/// The fewest bins along a side of a face of the transmission map.
const RELEASE_MIN_BINS: u32 = 16;

/// The most cells a closed space may hold for a release to be confined
/// in it (02 §13.2).
pub const RELEASE_FILL_MAX_CELLS: u32 = 4000;

/// How far above the base of its cell a release is unless it says: the
/// middle of a storey.
pub const RELEASE_DEFAULT_ALT_MM: u16 = 1500;

/// The least depth, in metres, at which a box is projected onto a face
/// of the transmission map.
const NEAR_M: Real = Real::of(1.0e-9);

type Vec3 = [Real; 3];

fn dot(a: Vec3, b: Vec3) -> Real {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

fn norm(a: Vec3) -> Real {
    dot(a, a).sqrt()
}

fn unit(a: Vec3) -> Option<Vec3> {
    let length = norm(a);
    (length > Real::ZERO && length.is_finite()).then(|| a.map(|part| part / length))
}

/// The volume of a cell in cubic metres.
fn cell_volume() -> Real {
    CELL_M * CELL_M * STOREY_M
}

/// The shape of a release (02 §13). A fixed set: fronts come with
/// package W24.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReleaseShape {
    /// In every direction.
    Sphere,
    /// Within an angle of a direction. Its intensity is that of a
    /// sphere divided by the cone's share of the sphere.
    Cone {
        /// The axis: east, south and up. Of any length.
        dir: [Real; 3],
        /// The angle from the axis to the edge, in radians.
        half_angle: Real,
    },
    /// A beam along a direction (02 §13.2).
    Beam {
        /// The axis: east, south and up. Of any length.
        dir: [Real; 3],
        /// How far from its axis it reaches, in metres.
        radius: Real,
        /// How far it goes, in metres.
        range: Real,
        /// Whether what is left of it where it ends is released there as
        /// a sphere.
        burst: bool,
    },
}

impl StateHash for ReleaseShape {
    fn state_hash(&self, hasher: &mut StateHasher) {
        match self {
            ReleaseShape::Sphere => hasher.write_u8(0),
            ReleaseShape::Cone { dir, half_angle } => {
                hasher.write_u8(1);
                dir.state_hash(hasher);
                half_angle.state_hash(hasher);
            }
            ReleaseShape::Beam {
                dir,
                radius,
                range,
                burst,
            } => {
                hasher.write_u8(2);
                dir.state_hash(hasher);
                radius.state_hash(hasher);
                range.state_hash(hasher);
                burst.state_hash(hasher);
            }
        }
    }
}

/// So much energy of a kind let go at a point (02 §13).
///
/// ```
/// use clash_core::Real;
/// use clash_world::{Cause, CellPos, DamageTypeId, EnergyRelease, GridId, Location};
///
/// let at = Location::new(GridId::new(0), CellPos::new(8, 8, 0));
/// let blast = EnergyRelease::sphere(at, DamageTypeId::new(0), Real::of(8.0e5), Cause::DEBUG);
/// // A grenade's worth reaches 3.4 m before it falls to 5 kPa.
/// assert!((blast.reach().get() - 3.37).abs() < 0.01);
/// ```
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub struct EnergyRelease {
    /// The cell it is in.
    pub origin: Location,
    /// How high above the base of that cell, in millimetres.
    pub origin_alt_mm: u16,
    /// How much, in joules.
    pub energy: Real,
    /// The kind of damage it does.
    pub kind: DamageTypeId,
    /// Its shape.
    pub shape: ReleaseShape,
    /// The speed of its front in m/s. `None` takes the speed of sound
    /// where it is, or [`RELEASE_VACUUM_SPEED`] in vacuum.
    pub speed: Option<Real>,
    /// Who and what let it go.
    pub cause: Cause,
}

clash_core::impl_state_hash!(EnergyRelease {
    origin,
    origin_alt_mm,
    energy,
    kind,
    shape,
    speed,
    cause
});

impl EnergyRelease {
    /// A release in every direction from the middle of a cell, at the
    /// speed of sound there.
    pub const fn sphere(
        origin: Location,
        kind: DamageTypeId,
        energy: Real,
        cause: Cause,
    ) -> EnergyRelease {
        EnergyRelease {
            origin,
            origin_alt_mm: RELEASE_DEFAULT_ALT_MM,
            energy,
            kind,
            shape: ReleaseShape::Sphere,
            speed: None,
            cause,
        }
    }

    /// The share of the whole sphere the release spreads into.
    fn share(&self) -> Real {
        match self.shape {
            ReleaseShape::Cone { half_angle, .. } => {
                ((Real::ONE - half_angle.cos()) * Real::of(0.5)).clamp(Real::ZERO, Real::ONE)
            }
            ReleaseShape::Sphere | ReleaseShape::Beam { .. } => Real::ONE,
        }
    }

    /// The intensity, in pascals, at a distance in metres with nothing
    /// in the way (02 §13.1): the energy over the volume swept.
    pub fn intensity_at(&self, distance_m: Real) -> Real {
        let r = distance_m.max(RELEASE_MIN_RADIUS_M);
        let swept = Real::of(4.0 / 3.0) * Real::PI * r * r * r * self.share();
        self.energy / swept
    }

    /// How far it reaches, in metres: where its intensity falls to
    /// [`WORLD_MIN_INTENSITY`], or for a beam its range.
    pub fn reach(&self) -> Real {
        if let ReleaseShape::Beam { range, .. } = self.shape {
            return range;
        }
        let swept = self.energy / WORLD_MIN_INTENSITY;
        (swept / (Real::of(4.0 / 3.0) * Real::PI * self.share())).cbrt()
    }

    /// The cause of what it does: a process, on behalf of whoever let
    /// it go.
    fn doing(&self) -> Cause {
        Cause {
            actor: self.cause.actor,
            instigator: self.cause.instigator.or(self.cause.actor),
            kind: CauseKind::Process,
        }
    }
}

/// What a release came to, for the one responsible (02 §7.4).
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub struct ReleaseTally {
    /// Who is responsible, if anyone.
    pub instigator: Option<EntityId>,
    /// How many elements it destroyed.
    pub elements: u32,
    /// The mass of what it destroyed, in kilograms.
    pub mass: Real,
    /// How many times an occupant was exposed to it.
    pub exposed: u32,
    /// How many occupants it threw.
    pub thrown: u32,
    /// The joules elements took from it.
    pub absorbed: Real,
}

clash_core::impl_state_hash!(ReleaseTally {
    instigator,
    elements,
    mass,
    exposed,
    thrown,
    absorbed
});

/// A release whose front is still spreading: a process of the world.
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub struct Release {
    /// Its id.
    pub id: ReleaseId,
    /// What was let go.
    pub spec: EnergyRelease,
    /// The speed of its front, in m/s.
    pub speed: Real,
    /// How far it reaches, in metres.
    pub reach: Real,
    /// The shell, or for a beam the slab, it handles at its next wake:
    /// so many metres out.
    pub shell: u32,
    /// How many bins there are along a side of a face of the
    /// transmission map.
    pub bins: u32,
    /// How much still gets through in each direction, 0 to 1: six faces
    /// of `bins × bins`.
    pub transmission: Vec<Real>,
    /// The joules of it still unspent. For a beam, what it has left to
    /// go on with.
    pub left: Real,
    /// The joules turned back by what it could not mark.
    pub blocked: Real,
    /// How many cells its front has visited.
    pub cells: u32,
    /// Whether it was cut off for the budget.
    pub cut_off: bool,
    /// The occupants it has exposed, in id order: each is exposed by
    /// the front once.
    pub exposed: Vec<EntityId>,
    /// What it has come to.
    pub tally: ReleaseTally,
}

clash_core::impl_state_hash!(Release {
    id,
    spec,
    speed,
    reach,
    shell,
    bins,
    transmission,
    left,
    blocked,
    cells,
    cut_off,
    exposed,
    tally
});

/// What the world keeps about releases besides their processes.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub struct ReleaseState {
    next: u32,
    /// The setting `release.max_cells`.
    max_cells: u32,
}

impl Default for ReleaseState {
    fn default() -> ReleaseState {
        ReleaseState {
            next: 0,
            max_cells: RELEASE_MAX_CELLS,
        }
    }
}

impl StateHash for ReleaseState {
    fn state_hash(&self, hasher: &mut StateHasher) {
        hasher.write_u32(self.next);
        hasher.write_u32(self.max_cells);
    }
}

/// Visits the bins of a transmission map that a box covers, as seen
/// from the origin. The box is given by its corners in metres from the
/// origin.
///
/// Each face of the cube holds the directions that are most along one
/// axis, laid out by the other two coordinates divided by that one. A
/// box is projected onto a face as the rectangle that bounds it there,
/// and covers the bins whose middles lie in that rectangle: so boxes
/// side by side share no bin and leave none between them, and whatever
/// lies wholly behind a box covers no bin the box does not. A box too
/// small or too far to hold the middle of any bin covers the one bin
/// its own middle lies in.
fn cover(bins: u32, low: Vec3, high: Vec3, mut visit: impl FnMut(usize)) {
    let side = bins as usize;
    if side == 0 {
        return;
    }
    let count = Real::from(bins);
    // Where a coordinate of −1 to 1 falls along a side, in bins.
    let along =
        |value: Real| ((value + Real::ONE) * Real::of(0.5) * count).clamp(Real::ZERO, count);
    let mut visited = false;
    for face in 0..6_usize {
        let axis = face / 2;
        let (near, far) = if face % 2 == 0 {
            (low[axis], high[axis])
        } else {
            (-high[axis], -low[axis])
        };
        if far <= Real::ZERO {
            continue;
        }
        let near = near.max(NEAR_M);
        let far = far.max(near);
        // The reach of the box along one of the other two axes, over
        // the depths it spans.
        let span = |other: usize| {
            let from = (low[other] / near).min(low[other] / far);
            let to = (high[other] / near).max(high[other] / far);
            (along(from), along(to))
        };
        let (u_from, u_to) = span((axis + 1) % 3);
        let (v_from, v_to) = span((axis + 2) % 3);
        // The bins whose middles, at a half past each whole number,
        // lie from `from` to `to`.
        let half = Real::of(0.5);
        let within = |from: Real, to: Real| {
            let first = (from - half).ceil().to_i64().max(0);
            let last = (to - half).floor().to_i64().min(side as i64 - 1);
            (first, last)
        };
        let (u_first, u_last) = within(u_from, u_to);
        let (v_first, v_last) = within(v_from, v_to);
        for row in v_first..=v_last {
            for column in u_first..=u_last {
                visited = true;
                visit(face * side * side + row as usize * side + column as usize);
            }
        }
    }
    if visited {
        return;
    }
    // The one bin the middle of the box lies in.
    let middle = [
        (low[0] + high[0]) * Real::of(0.5),
        (low[1] + high[1]) * Real::of(0.5),
        (low[2] + high[2]) * Real::of(0.5),
    ];
    let axis = (0..3)
        .max_by_key(|axis| middle[*axis].abs())
        .unwrap_or_default();
    let depth = middle[axis].abs();
    if depth <= Real::ZERO {
        return;
    }
    let face = axis * 2 + usize::from(middle[axis] < Real::ZERO);
    let bin_of = |other: usize| {
        let at = along(middle[other] / depth).floor().to_i64();
        at.clamp(0, side as i64 - 1) as usize
    };
    let (column, row) = (bin_of((axis + 1) % 3), bin_of((axis + 2) % 3));
    visit(face * side * side + row * side + column);
}

impl Release {
    /// How much of the release still gets through toward a box: the
    /// mean over the directions it covers.
    fn through(&self, low: Vec3, high: Vec3) -> Real {
        let (mut sum, mut count) = (Real::ZERO, 0_u32);
        cover(self.bins, low, high, |bin| {
            if let Some(passes) = self.transmission.get(bin) {
                sum += *passes;
                count += 1;
            }
        });
        if count > 0 {
            sum / Real::from(count)
        } else {
            Real::ONE
        }
    }

    /// The share of all directions that a box covers, as the map counts
    /// them: what part of a release in every direction is going its way.
    fn facing(&self, low: Vec3, high: Vec3) -> Real {
        let mut covered = 0_u32;
        cover(self.bins, low, high, |_| covered += 1);
        let all = 6 * self.bins * self.bins;
        if all > 0 {
            (Real::from(covered) / Real::from(all)).min(Real::ONE)
        } else {
            Real::ONE
        }
    }

    /// Notes that a box closes the directions it covers by a share:
    /// `stopped` of what was getting through them no longer will. The
    /// shade falls when the shell is done ([`Release::settle`]), so
    /// that the things of one shell, which stand side by side, do not
    /// shelter one another.
    fn shade(&self, cast: &mut Vec<(usize, Real)>, low: Vec3, high: Vec3, stopped: Real) {
        if stopped <= Real::ZERO {
            return;
        }
        let kept = Real::ONE - stopped.min(Real::ONE);
        cover(self.bins, low, high, |bin| cast.push((bin, kept)));
    }

    /// Lets the shade of a shell fall: every direction noted is closed
    /// by what was noted for it.
    fn settle(&mut self, cast: &mut Vec<(usize, Real)>) {
        for (bin, kept) in cast.drain(..) {
            if let Some(passes) = self.transmission.get_mut(bin) {
                *passes *= kept;
            }
        }
    }

    /// Closes the directions a box covers by a share, at once.
    #[cfg(test)]
    fn shadow(&mut self, low: Vec3, high: Vec3, stopped: Real) {
        let mut cast = Vec::new();
        self.shade(&mut cast, low, high, stopped);
        self.settle(&mut cast);
    }

    /// The height of the origin above the base of its cell, in metres.
    fn alt(&self) -> Real {
        Real::from(self.spec.origin_alt_mm) / Real::of(1000.0)
    }

    /// The box of a cell, so many cells from the origin's, in metres
    /// from the origin.
    fn cell_box(&self, offset: IVec3) -> (Vec3, Vec3) {
        let half = CELL_M * Real::of(0.5);
        let east = Real::from(offset.x) * CELL_M;
        let south = Real::from(offset.y) * CELL_M;
        let base = Real::from(offset.z) * STOREY_M - self.alt();
        (
            [east - half, south - half, base],
            [east + half, south + half, base + STOREY_M],
        )
    }

    /// Whether a point, in metres from the origin, lies within the
    /// release's shape. A sphere holds every point.
    fn holds(&self, point: Vec3) -> bool {
        match self.spec.shape {
            ReleaseShape::Cone { dir, half_angle } => match (unit(dir), unit(point)) {
                (Some(axis), Some(toward)) => dot(axis, toward) >= half_angle.cos(),
                // The origin itself.
                _ => true,
            },
            ReleaseShape::Sphere | ReleaseShape::Beam { .. } => true,
        }
    }
}

/// The offsets, in cells, of the cells whose centres lie from `inner`
/// up to but not including `inner + 1` metres from a point `alt` metres
/// above the base of the middle cell: storey by storey, row by row.
fn shell_cells(inner: u32, alt: Real) -> Vec<IVec3> {
    let near = Real::from(inner);
    let far = near + Real::ONE;
    let (near_sq, far_sq) = (near * near, far * far);
    let reach = i32::try_from(inner)
        .unwrap_or(i32::MAX - 1)
        .saturating_add(1);
    let middle = STOREY_M * Real::of(0.5) - alt;
    let storeys = ((far + alt.abs() + STOREY_M) / STOREY_M).ceil().to_i32();
    let mut cells = Vec::new();
    for z in -storeys..=storeys {
        let up = Real::from(z) * STOREY_M + middle;
        let up_sq = up * up;
        if up_sq >= far_sq {
            continue;
        }
        for y in -reach..=reach {
            let flat_sq = up_sq + Real::from(y) * Real::from(y);
            if flat_sq >= far_sq {
                continue;
            }
            // The row crosses the shell between these two distances
            // from its middle; the test below is the exact one.
            let outer = (far_sq - flat_sq).sqrt().ceil().to_i32().min(reach);
            let inner_edge = if flat_sq >= near_sq {
                0
            } else {
                ((near_sq - flat_sq).sqrt().floor().to_i32() - 1).max(0)
            };
            let mut visit = |x: i32| {
                let dist_sq = flat_sq + Real::from(x) * Real::from(x);
                if dist_sq >= near_sq && dist_sq < far_sq {
                    cells.push(IVec3::new(x, y, z));
                }
            };
            if inner_edge == 0 {
                for x in -outer..=outer {
                    visit(x);
                }
            } else {
                for x in -outer..=-inner_edge {
                    visit(x);
                }
                for x in inner_edge..=outer {
                    visit(x);
                }
            }
        }
    }
    cells
}

impl Cosmos {
    /// The releases whose fronts are still spreading, in the order they
    /// were let go.
    pub fn releases(&self) -> impl Iterator<Item = &Release> + '_ {
        self.activity
            .processes
            .iter()
            .filter_map(|(_, process)| match process {
                WorldProcess::Release(release) => Some(&**release),
                _ => None,
            })
    }

    /// The most cells the front of one release may visit: the setting
    /// `release.max_cells`.
    pub fn release_max_cells(&self) -> u32 {
        self.activity.releases.max_cells
    }

    /// Sets the most cells the front of one release may visit.
    pub fn set_release_max_cells(&mut self, cells: u32) {
        self.activity.releases.max_cells = cells;
    }
}

impl WorldCtx<'_> {
    /// Lets energy go at a point (02 §13; the verb `release`): the one
    /// entry point for explosions, shockwaves, blasts and beams. Emits
    /// `ReleaseStarted` now and `EnergyReleased`, with what it came to,
    /// when its front has gone as far as it goes.
    ///
    /// Refused for energy that is not above zero and finite, a kind of
    /// damage that does not exist, an origin that is not loaded, a
    /// speed that is not above zero, and a shape that makes no sense.
    pub fn release(&mut self, spec: EnergyRelease) -> Result<ReleaseId, WorldError> {
        let bad = |why: &'static str| WorldError::BadValue {
            what: "an energy release",
            why,
        };
        if !(spec.energy > Real::ZERO && spec.energy.is_finite()) {
            return Err(bad("its energy must be above zero and finite"));
        }
        if self.cosmos.defs().damage_types().get(spec.kind).is_none() {
            return Err(bad("there is no such kind of damage"));
        }
        let sound = |shape_dir: Vec3| unit(shape_dir).is_some();
        match spec.shape {
            ReleaseShape::Sphere => {}
            ReleaseShape::Cone { dir, half_angle } => {
                if !sound(dir) || !(half_angle > Real::ZERO && half_angle <= Real::PI) {
                    return Err(bad(
                        "a cone needs a direction and a half angle above zero and at most π",
                    ));
                }
            }
            ReleaseShape::Beam {
                dir, radius, range, ..
            } => {
                let sized = radius > Real::ZERO
                    && range > Real::ZERO
                    && radius.is_finite()
                    && range.is_finite();
                if !sound(dir) || !sized {
                    return Err(bad(
                        "a beam needs a direction, a radius and a range above zero",
                    ));
                }
            }
        }
        let origin = self.cosmos.normalize(spec.origin)?;
        if !self.cosmos.is_loaded(origin) {
            return Err(WorldError::NotLoaded(origin));
        }
        let spec = EnergyRelease { origin, ..spec };
        let ambient = self.cosmos.ambient(origin)?;
        let speed = match spec.speed {
            Some(speed) if speed > Real::ZERO && speed.is_finite() => speed,
            Some(_) => return Err(bad("its speed must be above zero and finite")),
            None => ambient
                .atmosphere
                .and_then(|id| self.cosmos.defs().atmospheres().get(id))
                .map(|air| air.sound_speed)
                .filter(|speed| *speed > Real::ZERO)
                .unwrap_or(RELEASE_VACUUM_SPEED),
        };
        let reach = spec.reach();
        let beam = matches!(spec.shape, ReleaseShape::Beam { .. });
        // Fine enough that a bin at the edge is about a cell across.
        let bins = if beam {
            0
        } else {
            let wanted = (reach * Real::PI * Real::of(0.5)).ceil().to_i64();
            u32::try_from(wanted.clamp(i64::from(RELEASE_MIN_BINS), i64::from(RELEASE_MAX_BINS)))
                .unwrap_or(RELEASE_MAX_BINS)
        };
        let state = &mut self.cosmos.activity.releases;
        let next = state
            .next
            .checked_add(1)
            .ok_or(WorldError::IdsExhausted("release"))?;
        let id = ReleaseId::new(state.next);
        let release = Release {
            id,
            spec,
            speed,
            reach,
            shell: 0,
            bins,
            transmission: vec![Real::ONE; 6 * (bins as usize) * (bins as usize)],
            left: spec.energy,
            blocked: Real::ZERO,
            cells: 0,
            cut_off: false,
            exposed: Vec::new(),
            tally: ReleaseTally {
                instigator: spec.doing().instigator,
                elements: 0,
                mass: Real::ZERO,
                exposed: 0,
                thrown: 0,
                absorbed: Real::ZERO,
            },
        };
        self.cosmos.activity.processes.spawn(
            self.timeline,
            ambient.clock,
            Dur::ZERO,
            WorldProcess::Release(Box::new(release)),
        )?;
        self.cosmos.activity.releases.next = next;
        self.events.emit(WorldEvent::ReleaseStarted {
            tick: self.timeline.now(),
            release: id,
            loc: origin,
            kind: spec.kind,
            energy: spec.energy,
            reach,
            cause: spec.cause,
        });
        Ok(id)
    }

    /// A release has woken: its front has reached the next shell.
    pub(crate) fn run_release(&mut self, release: &mut Release) -> Next {
        let done = match release.spec.shape {
            ReleaseShape::Beam {
                dir,
                radius,
                range,
                burst,
            } => self.march(release, dir, radius, range, burst),
            ReleaseShape::Sphere | ReleaseShape::Cone { .. } => self.spread(release),
        };
        if done {
            self.finish_release(release);
            return Next::Done;
        }
        Next::Sleep(Dur::from_secs_real(CELL_M / release.speed).max(Dur::MICROSECOND))
    }

    /// Handles one shell of a release that spreads from a point, and
    /// says whether that was the last.
    fn spread(&mut self, release: &mut Release) -> bool {
        let shell = release.shell;
        let near = Real::from(shell);
        if near > release.reach || release.left <= Real::ZERO {
            return true;
        }
        // The cells of a shell fill its volume: this many, or one or
        // two more at its edges.
        let far = near + Real::ONE;
        let volume = Real::of(4.0 / 3.0) * Real::PI * (far * far * far - near * near * near);
        let expected = (volume / cell_volume()).ceil().to_i64();
        let budget = i64::from(self.cosmos.activity.releases.max_cells);
        if i64::from(release.cells) + expected > budget {
            release.cut_off = true;
            return true;
        }
        let cells = shell_cells(shell, release.alt());
        release.cells = release
            .cells
            .saturating_add(u32::try_from(cells.len()).unwrap_or(u32::MAX));
        self.push_bodies(release, near, far);
        let mut cast = Vec::new();
        for offset in cells {
            self.reach_cell(release, offset, &mut cast);
        }
        release.settle(&mut cast);
        release.shell = shell.saturating_add(1);
        Real::from(release.shell) > release.reach || release.left <= Real::ZERO
    }

    /// The front reaches one cell: its elements are struck and whoever
    /// is in it is exposed (02 §13.2).
    fn reach_cell(&mut self, release: &mut Release, offset: IVec3, cast: &mut Vec<(usize, Real)>) {
        let origin = release.spec.origin;
        let Ok(view) = self.cosmos.grid(origin.grid) else {
            return;
        };
        let Ok(cell) = view.normalize(origin.cell.offset(offset)) else {
            return;
        };
        // W17: a tract that is not loaded gets a stamp instead.
        let Some(column) = view.column(cell.tract()) else {
            return;
        };
        let held = column.cell(cell);
        let loc = Location::new(origin.grid, cell);
        let occupied = self
            .cosmos
            .is_occupied(loc, &OccupantFilter::ANY)
            .unwrap_or(false);
        if held.is_empty() && !occupied {
            return;
        }
        let (low, high) = release.cell_box(offset);
        let middle = [
            (low[0] + high[0]) * Real::of(0.5),
            (low[1] + high[1]) * Real::of(0.5),
            (low[2] + high[2]) * Real::of(0.5),
        ];
        if !release.holds(middle) {
            return;
        }
        let open = release.spec.intensity_at(norm(middle));
        if open < WORLD_MIN_INTENSITY {
            return;
        }
        let mut through = release.through(low, high);
        if through <= Real::ZERO {
            return;
        }
        let away = unit(middle).unwrap_or([Real::ZERO, Real::ZERO, Real::ONE]);
        let defs = std::sync::Arc::clone(self.cosmos.defs());

        // A floor is a slab at the base of its cell. How much of the
        // wave it takes depends on how squarely the wave meets it.
        let floor_info = defs.floor_info(held.floor);
        let floor_box = (low, [high[0], high[1], low[2] + floor_info.thickness]);
        let to_floor = [middle[0], middle[1], low[2]];
        let squarely = match unit(to_floor) {
            Some(toward) => toward[2].abs(),
            None => Real::ONE,
        };
        // A floor above the origin stands between it and the rest of
        // its cell: it is struck first, and shelters what is over it.
        let floor_first = low[2] > Real::ZERO;
        // No element is given more than the part of the release that
        // is going its way.
        let going = |release: &Release, low: Vec3, high: Vec3| {
            release.spec.energy * release.facing(low, high)
        };
        let floor_most = going(release, floor_box.0, floor_box.1);
        if floor_first && !held.floor.is_none() {
            let intensity = open * through;
            let passed = self.strike(
                release,
                loc,
                Layer::Floor,
                intensity,
                squarely,
                floor_most,
                away,
            );
            if let Some(passed) = passed {
                release.shade(cast, floor_box.0, floor_box.1, Real::ONE - passed);
                through *= passed;
            }
        }
        let intensity = open * through;
        if intensity < WORLD_MIN_INTENSITY {
            return;
        }
        if occupied {
            self.expose_cell(release, loc, intensity, middle);
        }
        for (layer, state) in [
            (Layer::Block, held.block.raw()),
            (Layer::Fixture, held.fixture.raw()),
        ] {
            if state.is_none() {
                continue;
            }
            let info = *defs.info(layer, state);
            if info.coverage <= Real::ZERO || info.height <= Real::ZERO {
                continue;
            }
            let top = [high[0], high[1], low[2] + info.height];
            let most = going(release, low, top);
            let Some(passed) =
                self.strike(release, loc, layer, intensity, info.coverage, most, away)
            else {
                continue;
            };
            // It closes its share of the width of the cell, up to its
            // own height.
            let across = (info.coverage * STOREY_M / info.height).min(Real::ONE);
            release.shade(cast, low, top, across * (Real::ONE - passed));
        }
        if !floor_first && !held.floor.is_none() {
            let passed = self.strike(
                release,
                loc,
                Layer::Floor,
                intensity,
                squarely,
                floor_most,
                away,
            );
            if let Some(passed) = passed {
                release.shade(cast, floor_box.0, floor_box.1, Real::ONE - passed);
            }
        }
    }

    /// Delivers a release's packet to one element: the intensity given,
    /// with the energy the wave carries through `share` of the cell,
    /// and no more than `most` nor than the release has left. Returns
    /// the share of the packet that passed, or `None` if nothing was
    /// delivered.
    #[allow(clippy::too_many_arguments, reason = "one packet, said in full")]
    fn strike(
        &mut self,
        release: &mut Release,
        loc: Location,
        layer: Layer,
        intensity: Real,
        share: Real,
        most: Real,
        away: Vec3,
    ) -> Option<Real> {
        let energy = (intensity * cell_volume() * share)
            .min(most)
            .min(release.left);
        if energy <= Real::ZERO {
            return None;
        }
        let cause = release.spec.doing();
        let state = self
            .cosmos
            .grid(loc.grid)
            .ok()?
            .element(loc.cell, layer)
            .ok()?;
        let mass = self.cosmos.defs().bulk_mass(layer, state);
        let packet = DamagePacket::new(release.spec.kind, energy, intensity, cause);
        let outcome = self
            .edit(cause)
            .damage_along(loc, layer, packet, Some(away))
            .ok()?;
        release.left = (release.left - outcome.absorbed).max(Real::ZERO);
        Some(match outcome.result {
            DamageResult::Unaffected => {
                release.blocked += energy;
                Real::ZERO
            }
            DamageResult::Damaged => Real::ZERO,
            DamageResult::Destroyed => {
                release.tally.elements = release.tally.elements.saturating_add(1);
                release.tally.mass += mass;
                (outcome.passed / energy).clamp(Real::ZERO, Real::ONE)
            }
        })
    }

    /// Tells every occupant of a cell the front has reached that it
    /// was exposed, once for each release, and throws those the wave
    /// moves fast enough (02 §13.2).
    fn expose_cell(&mut self, release: &mut Release, loc: Location, intensity: Real, toward: Vec3) {
        let Ok(found) = self.cosmos.occupants_at(loc, &OccupantFilter::ANY) else {
            return;
        };
        for occupant in distinct_ids(&found) {
            let Err(at) = release.exposed.binary_search(&occupant) else {
                continue;
            };
            release.exposed.insert(at, occupant);
            self.expose_one(release, occupant, loc, intensity, Some(toward));
        }
    }

    /// Exposes one occupant: `OccupantExposed` with the intensity and
    /// the energy the wave carries through a cell. With a direction,
    /// the wave also pushes it.
    fn expose_one(
        &mut self,
        release: &mut Release,
        occupant: EntityId,
        loc: Location,
        intensity: Real,
        toward: Option<Vec3>,
    ) {
        let cause = release.spec.doing();
        let energy = intensity * cell_volume();
        self.events.emit(WorldEvent::OccupantExposed {
            tick: self.timeline.now(),
            occupant,
            loc,
            kind: release.spec.kind,
            energy,
            intensity,
            cause,
        });
        release.tally.exposed = release.tally.exposed.saturating_add(1);
        let Some(toward) = toward else {
            return;
        };
        let Some(record) = self.cosmos.occupant(occupant) else {
            return;
        };
        let (mass, kind, plane) = (
            record.occupant.mass,
            record.occupant.kind,
            record.occupant.plane,
        );
        // A device stands in the cell of its own element, and what the
        // world was told no mass for it cannot move.
        if mass <= Real::ZERO || kind == OccupantKind::Device {
            return;
        }
        // The world is told no size for an occupant: it takes the
        // cross-section of a lump of its mass, as a body of unknown
        // size does.
        let side = (mass / BODY_ASSUMED_DENSITY).cbrt();
        let pushing = intensity * side * side * RELEASE_PUSH_DEPTH_M;
        let speed = (Real::of(2.0) * pushing / mass).sqrt();
        let away = unit(toward).unwrap_or([Real::ZERO, Real::ZERO, Real::ONE]);
        let push = away.map(|part| part * speed);
        if let Some(body) = self.cosmos.body_of(occupant) {
            // Already in the air: the push adds to its motion.
            let _ = self.impulse(body, push.map(|part| part * mass));
            return;
        }
        if speed < RELEASE_THROW_SPEED {
            return;
        }
        let cell = loc.cell;
        let start = [
            Real::from(cell.x) * CELL_M + CELL_M * Real::of(0.5),
            Real::from(cell.y) * CELL_M + CELL_M * Real::of(0.5),
            Real::from(cell.z) * STOREY_M + RELEASE_THROW_ALT_M,
        ];
        let mut body = Body::of_mass(
            BodyOwner::Entity(occupant),
            loc.grid,
            start,
            push,
            mass,
            release.spec.cause,
        );
        body.plane = plane;
        if self.launch(body).is_ok() {
            release.tally.thrown = release.tally.thrown.saturating_add(1);
        }
    }

    /// Pushes the bodies in flight that the front reaches in a shell:
    /// those from `near` up to `far` metres from the origin that are
    /// not occupants, which are pushed when they are exposed.
    fn push_bodies(&mut self, release: &Release, near: Real, far: Real) {
        if self.cosmos.activity.processes.is_empty() {
            return;
        }
        let origin = release.spec.origin;
        let Ok(topology) = self.cosmos.topology(origin.grid).copied() else {
            return;
        };
        let reached: Vec<(BodyId, Vec3, Real)> = self
            .cosmos
            .bodies()
            .filter(|flight| !flight.away && flight.body.grid == origin.grid)
            .filter(|flight| {
                flight
                    .body
                    .entity()
                    .is_none_or(|entity| self.cosmos.occupant(entity).is_none())
            })
            .filter_map(|flight| {
                let cell = topology.normalize(flight.body.cell()).ok()?;
                let offset = topology.delta(origin.cell, cell);
                let (low, high) = release.cell_box(offset);
                // Where it is within its cell, from the origin.
                let within = |axis: usize, size: Real| {
                    let at = flight.body.pos[axis] / size;
                    (at - at.floor()) * size
                };
                let point = [
                    low[0] + within(0, CELL_M),
                    low[1] + within(1, CELL_M),
                    low[2] + within(2, STOREY_M),
                ];
                let distance = norm(point);
                if distance < near || distance >= far || !release.holds(point) {
                    return None;
                }
                let intensity = release.spec.intensity_at(distance) * release.through(low, high);
                (intensity >= WORLD_MIN_INTENSITY).then_some((
                    flight.id,
                    point,
                    intensity * flight.body.area * RELEASE_PUSH_DEPTH_M,
                ))
            })
            .collect();
        for (id, point, energy) in reached {
            let Some(mass) = self.cosmos.body(id).map(|body| body.mass) else {
                continue;
            };
            let away = unit(point).unwrap_or([Real::ZERO, Real::ZERO, Real::ONE]);
            // The momentum of that mass at the speed the energy gives.
            let momentum = (Real::of(2.0) * energy * mass).sqrt();
            let _ = self.impulse(id, away.map(|part| part * momentum));
        }
    }

    /// Handles one slab of a beam, a metre along its axis, and says
    /// whether the beam ends there (02 §13.2).
    fn march(
        &mut self,
        release: &mut Release,
        dir: Vec3,
        radius: Real,
        range: Real,
        burst: bool,
    ) -> bool {
        let Some(axis) = unit(dir) else {
            return true;
        };
        let near = Real::from(release.shell);
        if near >= range || release.left <= Real::ZERO {
            return self.end_beam(release, axis, near.min(range), burst);
        }
        let far = (near + Real::ONE).min(range);
        // The cells whose centres lie in this stretch of the beam.
        let wide = radius.ceil().to_i32().saturating_add(1);
        let tall = (radius / STOREY_M).ceil().to_i32().saturating_add(1);
        let along = (near + far) * Real::of(0.5);
        let centre = axis.map(|part| part * along);
        let alt = release.alt();
        let middle_cell = IVec3::new(
            centre[0].round().to_i32(),
            centre[1].round().to_i32(),
            ((centre[2] + alt) / STOREY_M).floor().to_i32(),
        );
        let mut cells = Vec::new();
        for z in -tall..=tall {
            for y in -wide..=wide {
                for x in -wide..=wide {
                    let offset = IVec3::new(
                        middle_cell.x.saturating_add(x),
                        middle_cell.y.saturating_add(y),
                        middle_cell.z.saturating_add(z),
                    );
                    let (low, high) = release.cell_box(offset);
                    let point = [
                        (low[0] + high[0]) * Real::of(0.5),
                        (low[1] + high[1]) * Real::of(0.5),
                        (low[2] + high[2]) * Real::of(0.5),
                    ];
                    let forward = dot(point, axis);
                    let off_sq = dot(point, point) - forward * forward;
                    if forward >= near && forward < far && off_sq <= radius * radius {
                        cells.push(offset);
                    }
                }
            }
        }
        release.cells = release
            .cells
            .saturating_add(u32::try_from(cells.len()).unwrap_or(u32::MAX));
        // The energy it still has, over a metre of its cross-section.
        let intensity = release.left / (Real::PI * radius * radius * CELL_M);
        let origin = release.spec.origin;
        let mut stopped = false;
        for offset in cells {
            let Ok(view) = self.cosmos.grid(origin.grid) else {
                return true;
            };
            let Ok(cell) = view.normalize(origin.cell.offset(offset)) else {
                // The edge of the realm: it goes no further.
                stopped = true;
                continue;
            };
            let Some(column) = view.column(cell.tract()) else {
                // W17: beyond loaded space the rest of it is a stamp.
                stopped = true;
                continue;
            };
            let held = column.cell(cell);
            let loc = Location::new(origin.grid, cell);
            let (low, high) = release.cell_box(offset);
            let point = [
                (low[0] + high[0]) * Real::of(0.5),
                (low[1] + high[1]) * Real::of(0.5),
                (low[2] + high[2]) * Real::of(0.5),
            ];
            if let Ok(found) = self.cosmos.occupants_at(loc, &OccupantFilter::ANY) {
                for occupant in distinct_ids(&found) {
                    if let Err(at) = release.exposed.binary_search(&occupant) {
                        release.exposed.insert(at, occupant);
                        self.expose_one(release, occupant, loc, intensity, Some(point));
                    }
                }
            }
            let defs = std::sync::Arc::clone(self.cosmos.defs());
            let elements = [
                (Layer::Block, held.block.raw()),
                (Layer::Fixture, held.fixture.raw()),
                (Layer::Floor, held.floor.raw()),
            ];
            for (layer, state) in elements {
                if state.is_none() {
                    continue;
                }
                let info = *defs.info(layer, state);
                // A floor is in a beam's way as far as the beam climbs
                // or dives.
                let share = match layer {
                    Layer::Floor => axis[2].abs(),
                    Layer::Block | Layer::Fixture => info.coverage,
                };
                if share <= Real::ZERO {
                    continue;
                }
                let most = release.left;
                let passed = self.strike(release, loc, layer, intensity, share, most, axis);
                let whole = match layer {
                    Layer::Floor => share >= Real::ONE,
                    Layer::Block | Layer::Fixture => {
                        info.height > Real::ZERO
                            && info.coverage * STOREY_M / info.height >= Real::ONE
                    }
                };
                // What closes the whole way and stands stops the beam.
                if whole
                    && passed.is_some_and(|passed| passed <= Real::ZERO)
                    && still_there(self.cosmos, loc, layer, state)
                {
                    stopped = true;
                }
            }
        }
        release.shell = release.shell.saturating_add(1);
        if stopped || release.left <= Real::ZERO || far >= range {
            return self.end_beam(release, axis, far, burst);
        }
        false
    }

    /// A beam has ended so far along its axis. What is left of it is
    /// let go there as a sphere, if it was to be.
    fn end_beam(&mut self, release: &mut Release, axis: Vec3, along: Real, burst: bool) -> bool {
        release.reach = along;
        if !burst || release.left <= Real::ZERO {
            return true;
        }
        let alt = release.alt();
        let end = axis.map(|part| part * along);
        let up = end[2] + alt;
        let storey = (up / STOREY_M).floor();
        let offset = IVec3::new(
            end[0].round().to_i32(),
            end[1].round().to_i32(),
            storey.to_i32(),
        );
        let above = ((up - storey * STOREY_M) * Real::of(1000.0))
            .round()
            .to_i64();
        let origin = Location::new(
            release.spec.origin.grid,
            release.spec.origin.cell.offset(offset),
        );
        let rest = EnergyRelease {
            origin,
            origin_alt_mm: u16::try_from(above.clamp(0, i64::from(crate::dims::STOREY_MM) - 1))
                .unwrap_or(RELEASE_DEFAULT_ALT_MM),
            energy: release.left,
            kind: release.spec.kind,
            shape: ReleaseShape::Sphere,
            speed: Some(release.speed),
            cause: release.spec.cause,
        };
        // Where it ended may be no place a release can be: then what
        // was left is lost.
        let _ = self.release(rest);
        true
    }

    /// A release has gone as far as it goes. If what it could not mark
    /// turned energy back into a closed space, that energy strikes the
    /// space once more (02 §13.2). Then `EnergyReleased`.
    fn finish_release(&mut self, release: &mut Release) {
        if !matches!(release.spec.shape, ReleaseShape::Beam { .. }) {
            self.confine(release);
        }
        // What it has not got left, something took: never more than
        // was let go.
        release.tally.absorbed = (release.spec.energy - release.left).max(Real::ZERO);
        let covered = match release.spec.shape {
            ReleaseShape::Beam { .. } => release.reach,
            ReleaseShape::Sphere | ReleaseShape::Cone { .. } => {
                Real::from(release.shell).min(release.reach)
            }
        };
        self.events.emit(WorldEvent::EnergyReleased {
            tick: self.timeline.now(),
            release: release.id,
            loc: release.spec.origin,
            kind: release.spec.kind,
            energy: release.spec.energy,
            absorbed: release.tally.absorbed,
            reach: covered,
            cells: release.cells,
            cut_off: release.cut_off,
            tallies: vec![release.tally],
            cause: release.spec.doing(),
        });
    }

    /// The energy a release's front had turned back is held by a space
    /// that closes round its origin, and strikes everything in that
    /// space and round it, evenly: `held / volume`. A space that does
    /// not close holds nothing.
    fn confine(&mut self, release: &mut Release) {
        let held = release.blocked.min(release.left);
        if held <= Real::ZERO {
            return;
        }
        let origin = release.spec.origin;
        let Ok(space) = self.cosmos.enclosed_cells(origin, RELEASE_FILL_MAX_CELLS) else {
            return;
        };
        let volume = Real::from_i64(space.len() as i64) * cell_volume();
        let intensity = held / volume;
        if intensity < WORLD_MIN_INTENSITY {
            return;
        }
        let outward = [
            (IVec3::new(0, -1, 0), Layer::Block),
            (IVec3::new(1, 0, 0), Layer::Block),
            (IVec3::new(0, 1, 0), Layer::Block),
            (IVec3::new(-1, 0, 0), Layer::Block),
        ];
        for cell in &space {
            let loc = Location::new(origin.grid, *cell);
            if let Ok(found) = self.cosmos.occupants_at(loc, &OccupantFilter::ANY) {
                for occupant in distinct_ids(&found) {
                    self.expose_one(release, occupant, loc, intensity, None);
                }
            }
            // What stands in the space, and what bounds it on each
            // side, above and below.
            self.press(release, loc, Layer::Fixture, intensity, [Real::ZERO; 3]);
            for (step, layer) in outward {
                self.press_beyond(release, &space, *cell, step, layer, intensity);
            }
            let up = IVec3::new(0, 0, 1);
            if !self.press_beyond(release, &space, *cell, up, Layer::Floor, intensity) {
                self.press_beyond(release, &space, *cell, up, Layer::Block, intensity);
            }
            let below = cell.below();
            if space.binary_search(&below).is_err()
                && !self.press(
                    release,
                    loc,
                    Layer::Floor,
                    intensity,
                    [Real::ZERO, Real::ZERO, -Real::ONE],
                )
            {
                let down = IVec3::new(0, 0, -1);
                self.press_beyond(release, &space, *cell, down, Layer::Block, intensity);
            }
        }
    }

    /// Presses on the element of a layer in the cell a step beyond a
    /// cell of a confined space, if that cell is not part of the space.
    /// Says whether there was an element there.
    fn press_beyond(
        &mut self,
        release: &mut Release,
        space: &[CellPos],
        cell: CellPos,
        step: IVec3,
        layer: Layer,
        intensity: Real,
    ) -> bool {
        let grid = release.spec.origin.grid;
        let Ok(beyond) = self
            .cosmos
            .normalize(Location::new(grid, cell.offset(step)))
        else {
            return false;
        };
        if space.binary_search(&beyond.cell).is_ok() {
            return false;
        }
        let away = [Real::from(step.x), Real::from(step.y), Real::from(step.z)];
        self.press(release, beyond, layer, intensity, away)
    }

    /// Presses on one element with the intensity a confined space
    /// holds. Says whether there was an element there.
    fn press(
        &mut self,
        release: &mut Release,
        loc: Location,
        layer: Layer,
        intensity: Real,
        away: Vec3,
    ) -> bool {
        let Ok(state) = self
            .cosmos
            .grid(loc.grid)
            .and_then(|view| view.element(loc.cell, layer))
        else {
            return false;
        };
        if state.is_none() {
            return false;
        }
        let share = match layer {
            Layer::Floor => Real::ONE,
            Layer::Block | Layer::Fixture => self.cosmos.defs().info(layer, state).coverage,
        };
        // Pressing changes nothing about what gets through: the front
        // has passed.
        let blocked = release.blocked;
        let most = release.left;
        self.strike(release, loc, layer, intensity, share, most, away);
        release.blocked = blocked;
        true
    }
}

/// Whether the element a beam struck is still the one that was there.
fn still_there(cosmos: &Cosmos, loc: Location, layer: Layer, state: ElementState) -> bool {
    cosmos
        .grid(loc.grid)
        .and_then(|view| view.element(loc.cell, layer))
        .is_ok_and(|now| now == state)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pos::GridId;

    fn r(value: f64) -> Real {
        Real::of(value)
    }

    fn blast(energy: f64) -> EnergyRelease {
        let at = Location::new(GridId::new(0), CellPos::new(0, 0, 0));
        EnergyRelease::sphere(at, DamageTypeId::new(0), r(energy), Cause::DEBUG)
    }

    #[test]
    fn the_radius_at_the_walls_threshold_is_the_tables() {
        // 02 §13.1: where each release falls to 70 kPa.
        let at_70_kpa = |energy: f64| {
            let swept = energy / 70.0e3;
            (r(swept) / (r(4.0 / 3.0) * Real::PI)).cbrt().get()
        };
        for (energy, radius) in [
            (8.0e5, 1.4),
            (2.0e9, 19.0),
            (4.0e12, 240.0),
            (4.0e15, 2400.0),
        ] {
            let found = at_70_kpa(energy);
            assert!((found - radius).abs() < radius * 0.02, "{energy}: {found}");
            let there = blast(energy).intensity_at(r(found));
            assert!((there.get() - 70.0e3).abs() < 1.0e-6 * 70.0e3);
        }
        // Within half a metre it is what it is at half a metre.
        let near = blast(8.0e5);
        assert_eq!(near.intensity_at(Real::ZERO), near.intensity_at(r(0.5)));
        // It reaches to where it falls to 5 kPa.
        let reach = near.reach();
        assert!((near.intensity_at(reach).get() - 5.0e3).abs() < 1.0e-6);
    }

    #[test]
    fn a_cone_is_as_much_stronger_as_it_is_narrower() {
        let mut cone = blast(1.0e6);
        cone.shape = ReleaseShape::Cone {
            dir: [r(1.0), Real::ZERO, Real::ZERO],
            half_angle: Real::PI * r(0.5),
        };
        // Half the sphere: twice the intensity.
        let whole = blast(1.0e6).intensity_at(r(5.0));
        assert!((cone.intensity_at(r(5.0)).get() - 2.0 * whole.get()).abs() < 1.0e-6);
        assert!(cone.reach() > blast(1.0e6).reach());
    }

    #[test]
    fn the_shells_fill_space_once() {
        // Every cell within ten metres is in exactly one shell.
        let alt = r(1.5);
        let mut seen = std::collections::BTreeMap::new();
        for shell in 0..10 {
            for offset in shell_cells(shell, alt) {
                let dist_sq = offset.x * offset.x + offset.y * offset.y + 9 * offset.z * offset.z;
                let (near, far) = ((shell * shell) as i32, ((shell + 1) * (shell + 1)) as i32);
                assert!(dist_sq >= near && dist_sq < far, "{offset:?} in {shell}");
                assert!(seen.insert((offset.z, offset.y, offset.x), shell).is_none());
            }
        }
        let mut within = 0;
        for z in -4_i32..=4 {
            for y in -10_i32..=10 {
                for x in -10_i32..=10 {
                    within += usize::from(x * x + y * y + 9 * z * z < 100);
                }
            }
        }
        assert_eq!(seen.len(), within);
        // They come storey by storey, row by row.
        let first = shell_cells(3, alt);
        let mut sorted = first.clone();
        sorted.sort_by_key(|offset| (offset.z, offset.y, offset.x));
        assert_eq!(first, sorted);
        // Off the middle of a storey the shells still hold every cell
        // once.
        let low: usize = (0..12).map(|shell| shell_cells(shell, r(0.2)).len()).sum();
        let mut count = 0;
        for z in -5_i32..=5 {
            for y in -12_i32..=12 {
                for x in -12_i32..=12 {
                    let up = f64::from(z) * 3.0 + 1.3;
                    count += usize::from(f64::from(x * x + y * y) + up * up < 144.0);
                }
            }
        }
        assert_eq!(low, count);
    }

    fn release_with(bins: u32) -> Release {
        let spec = blast(1.0e6);
        Release {
            id: ReleaseId::new(0),
            spec,
            speed: r(343.0),
            reach: spec.reach(),
            shell: 0,
            bins,
            transmission: vec![Real::ONE; 6 * (bins * bins) as usize],
            left: spec.energy,
            blocked: Real::ZERO,
            cells: 0,
            cut_off: false,
            exposed: Vec::new(),
            tally: ReleaseTally {
                instigator: None,
                elements: 0,
                mass: Real::ZERO,
                exposed: 0,
                thrown: 0,
                absorbed: Real::ZERO,
            },
        }
    }

    #[test]
    fn what_stands_in_the_way_shelters_what_is_behind_it() {
        let mut release = release_with(RELEASE_MAX_BINS);
        // Where a cell is does not depend on what has been shadowed.
        let boxes = release_with(RELEASE_MAX_BINS);
        let cell = |x: i32, y: i32, z: i32| boxes.cell_box(IVec3::new(x, y, z));
        // A wall cell three metres east closes that way whole.
        let (low, high) = cell(3, 0, 0);
        assert_eq!(release.through(low, high), Real::ONE);
        release.shadow(low, high, Real::ONE);
        assert_eq!(release.through(low, high), Real::ZERO);
        // Straight behind it nothing gets through; off to the side, to
        // the west, above and below, everything does.
        let (low, high) = cell(9, 0, 0);
        assert_eq!(release.through(low, high), Real::ZERO);
        for (x, y, z) in [
            (-3, 0, 0),
            (0, 5, 0),
            (3, 5, 0),
            (0, 0, 2),
            (0, 0, -2),
            (3, -4, 0),
        ] {
            let (low, high) = cell(x, y, z);
            assert_eq!(release.through(low, high), Real::ONE, "{x} {y} {z}");
        }
        // A cell partly in its shadow gets part.
        let (low, high) = cell(9, 2, 0);
        let part = release.through(low, high).get();
        assert!(part > 0.2 && part < 0.9, "{part}");
        // Something further off than a bin is wide still closes the bin
        // it is in, and what is straight behind it is in that bin too.
        let (low, high) = cell(60, 25, 3);
        assert_eq!(release.through(low, high), Real::ONE);
        release.shadow(low, high, Real::ONE);
        let (low, high) = cell(120, 50, 6);
        assert_eq!(release.through(low, high), Real::ZERO);

        // What lets half through halves what is behind it, twice over.
        let mut release = release_with(16);
        let (low, high) = release.cell_box(IVec3::new(0, -4, 0));
        release.shadow(low, high, r(0.5));
        release.shadow(low, high, r(0.5));
        let (low, high) = release.cell_box(IVec3::new(0, -12, 0));
        assert_eq!(release.through(low, high), r(0.25));

        // The cell the origin is in covers every direction.
        let mut release = release_with(8);
        let (low, high) = release.cell_box(IVec3::new(0, 0, 0));
        release.shadow(low, high, Real::ONE);
        assert!(
            release
                .transmission
                .iter()
                .all(|passes| *passes == Real::ZERO)
        );
    }

    #[test]
    fn a_floor_shelters_what_is_over_it_and_not_what_is_beside_it() {
        let mut release = release_with(16);
        // The slab of the storey above, over the origin and round it.
        for y in -6..=6 {
            for x in -6..=6 {
                let (low, high) = release.cell_box(IVec3::new(x, y, 1));
                release.shadow(low, [high[0], high[1], low[2] + r(0.2)], Real::ONE);
            }
        }
        let (low, high) = release.cell_box(IVec3::new(2, 1, 1));
        assert_eq!(release.through(low, high), Real::ZERO);
        let (low, high) = release.cell_box(IVec3::new(0, 0, 2));
        assert_eq!(release.through(low, high), Real::ZERO);
        // Out along the ground, past the edge of the slab, nothing is
        // in the way.
        let (low, high) = release.cell_box(IVec3::new(12, 0, 0));
        assert_eq!(release.through(low, high), Real::ONE);
    }
}
