//! The kernel's side of components (06 §3): the simulation holds one
//! [`Components`], tells it who is inside whom, and passes on to the
//! world what it works out.
//!
//! | When | What happens |
//! |------|--------------|
//! | An entity is given a shape, or its own mass changes | [`Sim::set_shape`], [`Sim::set_own_mass`]: its mass and that of every container round it are worked out again, and each that is in the world has the new mass written to its occupant record (02 §8) |
//! | An entity is put into a container or taken out of one | [`Sim::relocate`] brings the old container and the new one up to date in the same way |
//! | An entity is removed | Its components go with it, after `relocate` has taken it out of whatever held it |
//! | An entity is shelved with its tract | Nothing: it keeps its id, so its components stay where they are |
//!
//! Containment is read from the entities' places: the kernel implements
//! [`Containment`] for its [`EntityTable`].
//!
//! # Changing a component
//!
//! [`Sim::components_mut`] hands out every store. Insert an `Identity`,
//! or a component of a later package, through it. A `Physical` is
//! different: the masses of the containers round it and the world's
//! occupant record depend on it, so it is given and changed only through
//! [`Sim::set_shape`] and [`Sim::set_own_mass`].

use clash_core::EntityId;
use clash_data::Mass;
use clash_world::Cause;

use crate::Sim;
use crate::component::{
    Components, Containment, MassChanges, Physical, PhysicalError, Shape, physical,
};
use crate::entity::{EntityTable, KernelEntity};
use crate::reason::{Reason, codes};

/// Who is inside whom, read from the entities' places.
impl Containment for EntityTable {
    fn container(&self, id: EntityId) -> Option<EntityId> {
        self.get(id)?.place().container()
    }

    fn contents(&self, id: EntityId, visit: &mut dyn FnMut(EntityId)) {
        for inside in EntityTable::contents(self, id) {
            visit(*inside);
        }
    }
}

/// The reason for a refusal by the `Physical` component.
fn physical_reason(error: &PhysicalError) -> Reason {
    let code = match error {
        PhysicalError::NoComponent { .. } => codes::COMPONENT_MISSING,
        PhysicalError::TooDeep { .. } => codes::RELOCATE_TOO_DEEP,
        _ => codes::PHYSICAL_BAD_MEASURE,
    };
    Reason::new(code, error.to_string())
}

impl Sim {
    /// Every component of every entity, to read (06 §3). Truth.
    pub fn components(&self) -> &Components {
        &self.components
    }

    /// Every component store, to change. See the
    /// [module documentation](self): a `Physical` is changed through
    /// [`set_shape`](Sim::set_shape) and
    /// [`set_own_mass`](Sim::set_own_mass), never through this.
    pub fn components_mut(&mut self) -> &mut Components {
        &mut self.components
    }

    /// Says what an entity itself measures, giving it a `Physical` if
    /// it has none (06 §3.1). Its mass and the mass of every container
    /// round it are worked out again; each of them that stands in the
    /// world has the new mass written to the world's occupant record,
    /// and the entity's own record takes the shape's footprint.
    ///
    /// Refused, changing nothing, for an unknown entity, a measure that
    /// is negative or not finite, and a footprint that does not fit
    /// where the entity stands.
    pub fn set_shape(&mut self, id: EntityId, shape: Shape, cause: Cause) -> Result<(), Reason> {
        let entity = self.known(id)?;
        let (place, mut presence) = (entity.place(), *entity.presence());
        shape.check().map_err(|error| physical_reason(&error))?;
        if shape.footprint != presence.footprint {
            self.check_place(Some(id), shape.footprint, place)?;
        }
        let changes = physical::set_shape(&mut self.components.physical, &self.entities, id, shape)
            .map_err(|error| physical_reason(&error))?;
        presence.footprint = shape.footprint;
        if let Some(physical) = self.components.get::<Physical>(id) {
            presence.mass = physical.mass().get();
        }
        self.set_presence(id, presence, cause)?;
        self.apply_mass_changes(&changes, Some(id), cause);
        Ok(())
    }

    /// Changes the mass of an entity itself, leaving the rest of its
    /// shape, and brings every container round it and the world's
    /// occupant records up to date as [`set_shape`](Sim::set_shape)
    /// does.
    ///
    /// Refused for an unknown entity, one with no `Physical`, and a mass
    /// that is negative or not finite.
    pub fn set_own_mass(&mut self, id: EntityId, mass: Mass, cause: Cause) -> Result<(), Reason> {
        self.known(id)?;
        let changes =
            physical::set_own_mass(&mut self.components.physical, &self.entities, id, mass)
                .map_err(|error| physical_reason(&error))?;
        self.apply_mass_changes(&changes, None, cause);
        Ok(())
    }

    /// Brings the mass of a container, and of every container round it,
    /// up to date after what is inside it changed.
    pub(crate) fn refresh_mass(&mut self, container: EntityId, cause: Cause) {
        // Containment is kept free of loops and within depth by
        // `relocate`, so the walk cannot fail; if it did, nothing was
        // changed.
        if let Ok(changes) =
            physical::refresh(&mut self.components.physical, &self.entities, container)
        {
            self.apply_mass_changes(&changes, None, cause);
        }
    }

    /// Writes changed masses to the kernel's records and, for whoever
    /// stands in the world, to the world's, and has the stats of each
    /// entity whose mass changed worked out again (06 §4.1). `skip` has
    /// been written already.
    fn apply_mass_changes(&mut self, changes: &MassChanges, skip: Option<EntityId>, cause: Cause) {
        self.write_masses(changes, skip, cause);
        for change in changes {
            self.life_mass_changed(change.entity, cause);
        }
    }

    /// Writes changed masses to the kernel's records and, for whoever
    /// stands in the world, to the world's. `skip` has been written
    /// already.
    pub(crate) fn write_masses(
        &mut self,
        changes: &[crate::component::MassChange],
        skip: Option<EntityId>,
        cause: Cause,
    ) {
        for change in changes {
            if Some(change.entity) == skip {
                continue;
            }
            let Some(mut presence) = self
                .entities
                .get(change.entity)
                .map(KernelEntity::presence)
                .copied()
            else {
                continue;
            };
            presence.mass = change.after.get();
            // The footprint is as it was, so the world has no reason to
            // refuse.
            let _ = self.set_presence(change.entity, presence, cause);
        }
    }
}
