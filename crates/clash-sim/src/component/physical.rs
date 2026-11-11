//! [`Physical`]: how much of a thing there is (06 §3.1), with the mass of
//! everything it holds derived and cached.
//!
//! # What is stored and what is derived
//!
//! A [`Physical`] holds a [`Shape`], which is what the thing itself
//! measures: its own mass, volume, height, footprint and cross-section.
//! Whoever knows the thing's make-up supplies it: the body for a creature
//! (mass from its parts and scale, height from its posture), the item
//! definition for an item. This module derives nothing inside a shape.
//!
//! What it derives is the **carried mass**: the mass of everything inside
//! the thing, to any depth. [`Physical::mass`], the figure the world uses
//! for a body in flight or a collision (02 §15), is the two together:
//!
//! ```text
//! mass(e)    = own mass(e) + carried(e)
//! carried(e) = Σ mass(c), for each c directly inside e, in order of id
//! ```
//!
//! An entity with no `Physical` weighs nothing itself and passes on what
//! is inside it: its mass is the sum over its contents.
//!
//! # Where containment comes from
//!
//! Containment is the kernel's (`Place`, 03 §4.2). This module is handed
//! it as a [`Containment`] and keeps no copy. The cache is brought up to
//! date by the functions here, which whoever changes a shape or a place
//! must call:
//!
//! | After | Call |
//! |-------|------|
//! | A thing's make-up changed, or it is given a shape for the first time | [`set_shape`], or [`set_own_mass`] |
//! | An entity entered or left a container | [`refresh`] on that container (both, when it moved from one to another) |
//! | An entity with a `Physical` was removed | [`refresh`] on the container it was in |
//! | A component bundle was put back (a stored entity returning) | [`refresh`] on the entity |
//!
//! Each recomputes the sums it touches from their parts and never adds or
//! subtracts a difference, so the cache is always bit for bit what
//! [`recompute`] gives (06 invariant B3). Each returns the
//! [`MassChange`]s it made, so that the caller can tell the world the new
//! mass of an occupant and raise whatever events it has.
//!
//! The cache is saved and hashed with the component, so a load needs no
//! rebuilding and a cache that had gone wrong would show in the hash.

use clash_core::{EntityId, Real, impl_state_hash};
use clash_data::{Length, Mass, Volume};
use clash_world::Footprint;
use serde::{Deserialize, Serialize};
use smallvec::SmallVec;
use thiserror::Error;

use super::store::ComponentStore;

/// How deep containment may go: a coin in a purse in a pack on a rider on
/// a mount is five. A chain longer than this, which is also what a thing
/// that holds itself looks like, is refused rather than followed.
pub const MAX_CONTAINMENT_DEPTH: usize = 64;

/// Who is inside whom: the containment relation of 01 §6, as this module
/// needs to read it. The kernel implements it over its entities' places.
///
/// The two functions must agree: `c` is listed by `contents(e)` exactly
/// when `container(c)` is `e`. The order in which contents are listed
/// does not matter.
pub trait Containment {
    /// The entity `id` is directly inside, or `None` when it is in the
    /// world, stored, or not an entity.
    fn container(&self, id: EntityId) -> Option<EntityId>;

    /// Calls `visit` once for each entity directly inside `id`.
    fn contents(&self, id: EntityId, visit: &mut dyn FnMut(EntityId));
}

/// What a thing itself measures, leaving out whatever it holds.
///
/// Every measure is finite and at least zero; [`Shape::check`] says
/// whether one is.
#[derive(Clone, Copy, PartialEq, Debug, Default, Serialize, Deserialize)]
pub struct Shape {
    /// The thing's own mass.
    pub mass: Mass,
    /// The space it takes up.
    pub volume: Volume,
    /// How tall it stands, in its present posture.
    pub height: Length,
    /// The cells it fills.
    pub footprint: Footprint,
    /// The area it presents to what it moves through, in square metres.
    pub cross_section: Real,
}

impl_state_hash!(Shape {
    mass,
    volume,
    height,
    footprint,
    cross_section
});

impl Shape {
    /// Refuses a shape with a measure that is negative or not finite.
    pub fn check(&self) -> Result<(), PhysicalError> {
        measure("mass", self.mass.get())?;
        measure("volume", self.volume.get())?;
        measure("height", self.height.get())?;
        measure("cross-section", self.cross_section)
    }
}

/// Refuses a measure that is negative or not finite.
fn measure(name: &'static str, value: Real) -> Result<(), PhysicalError> {
    if value.is_finite() && value >= Real::ZERO {
        Ok(())
    } else {
        Err(PhysicalError::BadMeasure {
            measure: name,
            value,
        })
    }
}

/// Why a change to a [`Physical`] was refused. A refused change has
/// changed nothing.
#[derive(Debug, Clone, PartialEq, Error)]
#[non_exhaustive]
pub enum PhysicalError {
    /// The entity has no `Physical` to change.
    #[error("{entity} has no physical component")]
    NoComponent {
        /// The entity.
        entity: EntityId,
    },
    /// A mass, volume, height or cross-section was negative or not
    /// finite.
    #[error("a {measure} of {value} is not a finite amount of zero or more")]
    BadMeasure {
        /// Which measure.
        measure: &'static str,
        /// What was given.
        value: Real,
    },
    /// Containment goes deeper than [`MAX_CONTAINMENT_DEPTH`], or round
    /// in a circle.
    #[error("{entity} is held more than {MAX_CONTAINMENT_DEPTH} deep, or holds itself")]
    TooDeep {
        /// Where the walk gave up.
        entity: EntityId,
    },
}

/// How much of a thing there is: its own [`Shape`], and the cached mass
/// of everything inside it (06 §3.1).
///
/// A shape's size is changed through the setters here. Its mass, on which
/// the cache of every container around it depends, is changed only
/// through [`set_shape`] and [`set_own_mass`].
#[derive(Clone, Copy, PartialEq, Debug, Serialize, Deserialize)]
pub struct Physical {
    /// What the thing itself measures.
    own: Shape,
    /// The mass of everything inside it, to any depth. Derived.
    carried: Mass,
}

impl_state_hash!(Physical { own, carried });

impl Physical {
    /// A thing of this shape that holds nothing. To give an entity one,
    /// use [`set_shape`], which also brings its containers up to date.
    pub fn new(shape: Shape) -> Result<Physical, PhysicalError> {
        shape.check()?;
        Ok(Physical {
            own: shape,
            carried: Mass::new(Real::ZERO),
        })
    }

    /// The mass of the thing and everything inside it.
    pub fn mass(&self) -> Mass {
        Mass::new(self.own.mass.get() + self.carried.get())
    }

    /// The mass of the thing alone.
    pub const fn own_mass(&self) -> Mass {
        self.own.mass
    }

    /// The mass of everything inside the thing, to any depth.
    pub const fn carried_mass(&self) -> Mass {
        self.carried
    }

    /// The space the thing takes up.
    pub const fn volume(&self) -> Volume {
        self.own.volume
    }

    /// How tall the thing stands.
    pub const fn height(&self) -> Length {
        self.own.height
    }

    /// The cells the thing fills.
    pub const fn footprint(&self) -> Footprint {
        self.own.footprint
    }

    /// The area the thing presents to what it moves through, in square
    /// metres.
    pub const fn cross_section(&self) -> Real {
        self.own.cross_section
    }

    /// What the thing itself measures.
    pub const fn shape(&self) -> Shape {
        self.own
    }

    /// Changes the space the thing takes up.
    pub fn set_volume(&mut self, volume: Volume) -> Result<(), PhysicalError> {
        measure("volume", volume.get())?;
        self.own.volume = volume;
        Ok(())
    }

    /// Changes how tall the thing stands.
    pub fn set_height(&mut self, height: Length) -> Result<(), PhysicalError> {
        measure("height", height.get())?;
        self.own.height = height;
        Ok(())
    }

    /// Changes the cells the thing fills. The world learns of it through
    /// relocation, not from here.
    pub const fn set_footprint(&mut self, footprint: Footprint) {
        self.own.footprint = footprint;
    }

    /// Changes the area the thing presents, in square metres.
    pub fn set_cross_section(&mut self, cross_section: Real) -> Result<(), PhysicalError> {
        measure("cross-section", cross_section)?;
        self.own.cross_section = cross_section;
        Ok(())
    }
}

/// One entity's mass, before and after a change.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct MassChange {
    /// Whose mass changed.
    pub entity: EntityId,
    /// Its mass before, with everything inside it.
    pub before: Mass,
    /// Its mass now.
    pub after: Mass,
}

/// The masses one call changed, innermost entity first. The last is the
/// outermost container, the one in the world, when the change reached it.
pub type MassChanges = SmallVec<[MassChange; 4]>;

/// Carried masses worked out and not yet stored, innermost first.
type Pending = SmallVec<[(EntityId, Mass); 4]>;

/// Sums masses over the containment relation.
struct Sums<'a, R: ?Sized> {
    store: &'a ComponentStore<Physical>,
    rel: &'a R,
    /// Carried masses that stand in for the cached ones.
    pending: &'a [(EntityId, Mass)],
    /// Whether to trust no cache and sum all the way down.
    scratch: bool,
}

impl<R: Containment + ?Sized> Sums<'_, R> {
    /// The mass of everything inside `id`: the masses of its contents,
    /// added in order of id.
    fn carried(&self, id: EntityId, depth: usize) -> Result<Mass, PhysicalError> {
        if depth > MAX_CONTAINMENT_DEPTH {
            return Err(PhysicalError::TooDeep { entity: id });
        }
        let mut inside: SmallVec<[EntityId; 8]> = SmallVec::new();
        self.rel.contents(id, &mut |held| inside.push(held));
        inside.sort_unstable();
        inside.dedup();
        let mut sum = Real::ZERO;
        for held in inside {
            sum += self.mass(held, depth + 1)?.get();
        }
        Ok(Mass::new(sum))
    }

    /// The mass of `id` with everything inside it.
    fn mass(&self, id: EntityId, depth: usize) -> Result<Mass, PhysicalError> {
        let Some(physical) = self.store.get(id) else {
            // Weightless itself; what it holds still counts.
            return self.carried(id, depth);
        };
        let carried = if self.scratch {
            self.carried(id, depth)?
        } else {
            self.pending
                .iter()
                .find(|(entity, _)| *entity == id)
                .map_or(physical.carried, |(_, carried)| *carried)
        };
        Ok(Mass::new(physical.own.mass.get() + carried.get()))
    }
}

/// Works out the carried mass of `id` and of every container around it,
/// innermost first, changing nothing.
fn climb<R: Containment + ?Sized>(
    store: &ComponentStore<Physical>,
    rel: &R,
    id: EntityId,
) -> Result<Pending, PhysicalError> {
    let mut pending = Pending::new();
    let mut at = id;
    for _ in 0..=MAX_CONTAINMENT_DEPTH {
        if store.contains(at) {
            let sums = Sums {
                store,
                rel,
                pending: &pending,
                scratch: false,
            };
            let carried = sums.carried(at, 0)?;
            pending.push((at, carried));
        }
        match rel.container(at) {
            Some(container) => at = container,
            None => return Ok(pending),
        }
    }
    Err(PhysicalError::TooDeep { entity: at })
}

/// Stores carried masses worked out by [`climb`] and lists the masses
/// that changed. `first`, when given, is what the named entity's mass was
/// before the caller touched it.
fn settle(
    store: &mut ComponentStore<Physical>,
    pending: &[(EntityId, Mass)],
    first: Option<(EntityId, Mass)>,
) -> MassChanges {
    let mut changes = MassChanges::new();
    for &(entity, carried) in pending {
        let Some(physical) = store.get_mut(entity) else {
            continue;
        };
        let before = match first {
            Some((changed, before)) if changed == entity => before,
            _ => physical.mass(),
        };
        physical.carried = carried;
        let after = physical.mass();
        if before != after {
            changes.push(MassChange {
                entity,
                before,
                after,
            });
        }
    }
    changes
}

/// Says what `id` itself measures, giving it a `Physical` if it has none,
/// and brings its mass and that of every container around it up to date.
///
/// This is what a body calls when its parts or scale change and what an
/// item calls when it is made (06 §6.5, §9.1).
pub fn set_shape<R: Containment + ?Sized>(
    store: &mut ComponentStore<Physical>,
    rel: &R,
    id: EntityId,
    shape: Shape,
) -> Result<MassChanges, PhysicalError> {
    shape.check()?;
    let existing = store.get(id).copied();
    if let Some(physical) = existing
        && physical.own.mass.get().get().to_bits() == shape.mass.get().get().to_bits()
    {
        // Only its size changed: no mass anywhere depends on that.
        if let Some(physical) = store.get_mut(id) {
            physical.own = shape;
        }
        return Ok(MassChanges::new());
    }
    let before = Sums {
        store,
        rel,
        pending: &[],
        scratch: false,
    }
    .mass(id, 0)?;
    let carried = existing.map_or(Mass::new(Real::ZERO), |physical| physical.carried);
    store.insert(
        id,
        Physical {
            own: shape,
            carried,
        },
    );
    match climb(store, rel, id) {
        Ok(pending) => Ok(settle(store, &pending, Some((id, before)))),
        Err(error) => {
            // Put back exactly what was there.
            match existing {
                Some(physical) => {
                    store.insert(id, physical);
                }
                None => {
                    store.remove(id);
                }
            }
            Err(error)
        }
    }
}

/// Changes the mass of `id` itself, and brings its mass and that of every
/// container around it up to date.
pub fn set_own_mass<R: Containment + ?Sized>(
    store: &mut ComponentStore<Physical>,
    rel: &R,
    id: EntityId,
    mass: Mass,
) -> Result<MassChanges, PhysicalError> {
    let Some(physical) = store.get(id) else {
        return Err(PhysicalError::NoComponent { entity: id });
    };
    let shape = Shape {
        mass,
        ..physical.own
    };
    set_shape(store, rel, id, shape)
}

/// Brings the mass of `id` and of every container around it up to date,
/// after what is inside `id` changed. `id` need not have a `Physical`
/// itself.
pub fn refresh<R: Containment + ?Sized>(
    store: &mut ComponentStore<Physical>,
    rel: &R,
    id: EntityId,
) -> Result<MassChanges, PhysicalError> {
    let pending = climb(store, rel, id)?;
    Ok(settle(store, &pending, None))
}

/// The mass of `id` and everything inside it, summed from own masses
/// alone and trusting no cache. This is the recomputation that the cache
/// must always equal (06 invariant B3).
pub fn recompute<R: Containment + ?Sized>(
    store: &ComponentStore<Physical>,
    rel: &R,
    id: EntityId,
) -> Result<Mass, PhysicalError> {
    Sums {
        store,
        rel,
        pending: &[],
        scratch: true,
    }
    .mass(id, 0)
}

/// The entities whose cached mass is not bit for bit what [`recompute`]
/// gives, in dense order. Empty when invariant B3 holds.
pub fn mismatches<R: Containment + ?Sized>(
    store: &ComponentStore<Physical>,
    rel: &R,
) -> Vec<EntityId> {
    store
        .iter()
        .filter(|(id, physical)| {
            recompute(store, rel, *id).map(|mass| mass.get().get().to_bits())
                != Ok(physical.mass().get().get().to_bits())
        })
        .map(|(id, _)| id)
        .collect()
}

/// Works out every cached mass again from own masses alone. Nothing in
/// ordinary running needs it; it is for a store assembled some other way
/// than through the functions of this module.
pub fn rebuild<R: Containment + ?Sized>(
    store: &mut ComponentStore<Physical>,
    rel: &R,
) -> Result<(), PhysicalError> {
    let sums = Sums {
        store: &*store,
        rel,
        pending: &[],
        scratch: true,
    };
    let mut carried = Vec::with_capacity(store.len());
    for &id in store.owners() {
        carried.push(sums.carried(id, 0)?);
    }
    for ((_, physical), carried) in store.iter_mut().zip(carried) {
        physical.carried = carried;
    }
    Ok(())
}
