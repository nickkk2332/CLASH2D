//! Paths over what a mind believes (15 §7.3; 09 §3.4, §5.3).
//!
//! Static geometry is common knowledge: a mind plans over the true
//! layout of walls, floors and doors. Two things are not free:
//!
//! - **Hidden elements.** An element tagged `hidden` that the mind has
//!   not noted ([`note_element`](super::note_element)) is not known for
//!   what it is: something hidden that would have to be opened to pass
//!   is taken for a wall.
//! - **Unknown places, for a mind that keeps map memory.** A directly
//!   controlled character plans through cells it has never seen as
//!   through any other, at [`UNKNOWN_CELL_FACTOR`] times the cost, so
//!   that it keeps to places it knows when it can.
//!
//! [`believed_move_rules`] wraps the plain walker that opens doors;
//! [`believed_move_rules_with`] wraps the mover's own rules.

use clash_core::{EntityId, Real};
use clash_world::defs::WorldDefs;
use clash_world::{
    CellGeometry, CellPos, Cosmos, ElementState, GridId, Layer, Location, MoveProfile, MoveRules,
    Step,
};

use super::belief;
use super::party::MapMemory;
use crate::Sim;
use crate::perception::belief::Beliefs;

/// The tag that keeps an element out of what a mind knows until it has
/// noted it (15 §7.3).
pub const HIDDEN_TAG: &str = "hidden";

/// How many times dearer a step into a cell a mind with map memory has
/// never seen is: unknown ground is planned through, but known ground is
/// preferred.
pub const UNKNOWN_CELL_FACTOR: Real = Real::of(1.5);

/// Movement rules that answer from what a mind believes. See the
/// [module](self).
pub struct BelievedMoveRules<'a, R> {
    inner: R,
    grid: Option<GridId>,
    cosmos: &'a Cosmos,
    beliefs: Option<&'a Beliefs>,
    memory: Option<&'a MapMemory>,
}

impl<R: core::fmt::Debug> core::fmt::Debug for BelievedMoveRules<'_, R> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("BelievedMoveRules")
            .field("inner", &self.inner)
            .field("grid", &self.grid)
            .finish_non_exhaustive()
    }
}

/// Believed rules for a mind over the plain walker of the world that
/// opens doors ([`MoveProfile::walker`] with `opens`).
pub fn believed_move_rules(sim: &Sim, mind: EntityId) -> BelievedMoveRules<'_, MoveProfile> {
    believed_move_rules_with(
        sim,
        mind,
        MoveProfile {
            opens: true,
            ..MoveProfile::walker()
        },
    )
}

/// Believed rules for a mind over the mover's own rules: what the mover
/// can do is `inner`'s to say, what the mind knows is this one's.
pub fn believed_move_rules_with<R: MoveRules>(
    sim: &Sim,
    mind: EntityId,
    inner: R,
) -> BelievedMoveRules<'_, R> {
    BelievedMoveRules {
        inner,
        grid: sim.entities.location(mind).map(|loc| loc.grid),
        cosmos: &sim.cosmos,
        beliefs: sim.components.beliefs.get(mind),
        memory: sim.components.map_memory.get(mind),
    }
}

impl<R> BelievedMoveRules<'_, R> {
    /// The rules for the cells of another grid than the mind's own.
    #[must_use]
    pub fn on_grid(mut self, grid: GridId) -> Self {
        self.grid = Some(grid);
        self
    }

    /// Whether a cell holds something hidden that the mind has not noted
    /// and that stands in the way of walking.
    fn hidden_in_way(&self, cell: CellPos) -> bool {
        let Some(grid) = self.grid else {
            return false;
        };
        let loc = Location::new(grid, cell);
        let Ok(states) = self.cosmos.cell(loc) else {
            return false;
        };
        let defs: &WorldDefs = self.cosmos.defs();
        [
            (Layer::Block, states.block.raw()),
            (Layer::Fixture, states.fixture.raw()),
        ]
        .into_iter()
        .any(|(layer, state)| self.unnoted_hidden(defs, loc, layer, state))
    }

    fn unnoted_hidden(
        &self,
        defs: &WorldDefs,
        loc: Location,
        layer: Layer,
        state: ElementState,
    ) -> bool {
        if state.is_none() {
            return false;
        }
        let hidden = defs
            .element(layer, state)
            .is_some_and(|element| element.tags.iter().any(|tag| tag == HIDDEN_TAG));
        hidden
            && !self.beliefs.is_some_and(|beliefs| {
                beliefs.noted().iter().any(|noted| {
                    noted.loc == loc && noted.layer == layer && noted.state.def == state.def
                })
            })
    }

    /// A cell as the mind takes it to be: one whose way is closed by
    /// something hidden and unnoted does not open.
    fn believed(&self, cell: &CellGeometry) -> CellGeometry {
        if cell.opens && self.hidden_in_way(cell.cell) {
            CellGeometry {
                opens: false,
                ..*cell
            }
        } else {
            *cell
        }
    }

    fn unknown(&self, cell: CellPos) -> bool {
        match (self.memory, self.grid) {
            (Some(memory), Some(grid)) => memory.cell(Location::new(grid, cell)).is_none(),
            _ => false,
        }
    }
}

impl<R: MoveRules> MoveRules for BelievedMoveRules<'_, R> {
    fn can_occupy(&self, cell: &CellGeometry) -> bool {
        self.inner.can_occupy(&self.believed(cell))
    }

    fn step_cost(&self, step: &Step) -> Option<Real> {
        let believed = Step {
            from: self.believed(&step.from),
            to: self.believed(&step.to),
            sides: step.sides.map(|side| side.map(|side| self.believed(&side))),
            ..*step
        };
        let cost = self.inner.step_cost(&believed)?;
        Some(if self.unknown(step.to.cell) {
            cost * UNKNOWN_CELL_FACTOR
        } else {
            cost
        })
    }
}

/// Whether a mind knows an element of a cell for what it is: it is not
/// tagged `hidden`, or the mind has noted it.
pub fn knows_element(sim: &Sim, mind: EntityId, loc: Location, layer: Layer) -> bool {
    let Ok(states) = sim.cosmos.cell(loc) else {
        return false;
    };
    let state = match layer {
        Layer::Floor => states.floor.raw(),
        Layer::Block => states.block.raw(),
        Layer::Fixture => states.fixture.raw(),
    };
    let hidden = sim
        .cosmos
        .defs()
        .element(layer, state)
        .is_some_and(|element| element.tags.iter().any(|tag| tag == HIDDEN_TAG));
    !hidden || belief::has_noted(sim, mind, loc, layer, state)
}
