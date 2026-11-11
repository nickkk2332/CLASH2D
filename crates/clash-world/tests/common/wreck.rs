//! A world to break, for the tests of damage and of collapse: one
//! `testland` realm with flat rock under grass in two tracts, whatever a
//! test builds on it, and a loop that drives the timeline the way the
//! simulation kernel will.

// Each test binary uses its own part of this module.
#![allow(dead_code)]

use std::sync::Arc;

use clash_core::{Dur, EntityId, Hash128, Tick};
use clash_time::{Timeline, WakeDomain};
use clash_world::test_support::load_column;
use clash_world::{
    BlockState, Cause, CauseKind, CellPos, ColumnBuilder, Cosmos, DamageTypeId, EventSink,
    FixtureState, FloorState, GridId, Layer, Location, TractPos, WorldCtx, WorldDefs, WorldEvent,
};

pub struct World {
    pub defs: Arc<WorldDefs>,
    pub timeline: Timeline,
    pub cosmos: Cosmos,
    pub events: EventSink<WorldEvent>,
    pub grid: GridId,
}

/// A world over `defs`: rock below storey 0 and grass on it, in tracts
/// (0, 0) and (1, 0). `build` adds to the column of tract (0, 0) before
/// it is loaded, so that what a test stands up asks for no checks.
pub fn world_from(defs: Arc<WorldDefs>, build: impl FnOnce(&mut ColumnBuilder<'_>)) -> World {
    let mut timeline = Timeline::new();
    let mut cosmos = Cosmos::new(Arc::clone(&defs));
    let def = defs.realms().lookup("core:testland").unwrap();
    let realm = cosmos.create_realm(&mut timeline, def, 7).unwrap();
    let grid = cosmos.realm(realm).unwrap().base_grid;
    let rock = defs.block_state("core:solid", None).unwrap();
    let grass = defs.floor_state("core:grass", None).unwrap();
    let mut build = Some(build);
    for tract in [TractPos::new(0, 0), TractPos::new(1, 0)] {
        let mut builder = ColumnBuilder::new(&defs, rock, BlockState::NONE, 0);
        builder.fill_floor(0, grass);
        if let Some(build) = build.take() {
            build(&mut builder);
        }
        load_column(&mut cosmos, grid, tract, builder.build()).unwrap();
    }
    World {
        defs,
        timeline,
        cosmos,
        events: EventSink::new(),
        grid,
    }
}

pub fn at(x: i32, y: i32, z: i32) -> CellPos {
    CellPos::new(x, y, z)
}

pub fn someone() -> Cause {
    Cause::new(CauseKind::Action).by(EntityId::new(3, 0))
}

impl World {
    pub fn ctx(&mut self) -> WorldCtx<'_> {
        WorldCtx::new(&mut self.cosmos, &mut self.timeline, &mut self.events)
    }

    pub fn loc(&self, x: i32, y: i32, z: i32) -> Location {
        Location::new(self.grid, at(x, y, z))
    }

    pub fn block(&self, name: &str) -> BlockState {
        self.defs.block_state(name, None).unwrap()
    }

    pub fn floor(&self, name: &str) -> FloorState {
        self.defs.floor_state(name, None).unwrap()
    }

    pub fn fixture(&self, name: &str) -> FixtureState {
        self.defs.fixture_state(name, None).unwrap()
    }

    pub fn damage_type(&self, name: &str) -> DamageTypeId {
        self.defs.damage_types().lookup(name).unwrap()
    }

    pub fn block_at(&self, x: i32, y: i32, z: i32) -> BlockState {
        self.cosmos.cell(self.loc(x, y, z)).unwrap().block
    }

    pub fn floor_at(&self, x: i32, y: i32, z: i32) -> FloorState {
        self.cosmos.cell(self.loc(x, y, z)).unwrap().floor
    }

    pub fn fixture_at(&self, x: i32, y: i32, z: i32) -> FixtureState {
        self.cosmos.cell(self.loc(x, y, z)).unwrap().fixture
    }

    pub fn damage_at(&self, x: i32, y: i32, z: i32, layer: Layer) -> u8 {
        self.cosmos.damage(self.loc(x, y, z)).unwrap()[layer.index()]
    }

    /// Runs every wake-up due up to a tick, the world's through its one
    /// entry point, and moves time on to the tick.
    pub fn run_until(&mut self, until: Tick) {
        while let Some((_, wake)) = self.timeline.pop(until) {
            if wake.domain == WakeDomain::World {
                self.ctx().handle_wake(&wake).unwrap();
            }
        }
        self.timeline.advance_to(until);
    }

    /// Runs for a span of universal time from now.
    pub fn run_for(&mut self, span: Dur) {
        let until = self.timeline.now() + span;
        self.run_until(until);
    }

    /// Runs until nothing more is queued. Wake-ups parked on a stopped
    /// clock stay parked.
    pub fn run_out(&mut self) {
        while let Some(due) = self.timeline.peek() {
            self.run_until(due);
        }
    }

    /// The hash of everything that is state: the cosmos and the
    /// timeline.
    pub fn state_hash(&self) -> (Hash128, Hash128) {
        (Hash128::of(&self.cosmos), Hash128::of(&self.timeline))
    }

    /// How many cells of a box hold a floor of a definition.
    pub fn count_floors(&self, low: CellPos, high: CellPos, floor: FloorState) -> usize {
        self.count(low, high, |w, x, y, z| {
            w.floor_at(x, y, z).def() == floor.def()
        })
    }

    /// How many cells of a box hold a block of a definition.
    pub fn count_blocks(&self, low: CellPos, high: CellPos, block: BlockState) -> usize {
        self.count(low, high, |w, x, y, z| {
            w.block_at(x, y, z).def() == block.def()
        })
    }

    fn count(
        &self,
        low: CellPos,
        high: CellPos,
        holds: impl Fn(&World, i32, i32, i32) -> bool,
    ) -> usize {
        let mut count = 0;
        for z in low.z..=high.z {
            for y in low.y..=high.y {
                for x in low.x..=high.x {
                    count += usize::from(holds(self, x, y, z));
                }
            }
        }
        count
    }
}

/// A tower in a column being built: walls round the edge of a square
/// footprint on every storey from the ground up, a slab over the whole
/// footprint at the top of each storey, and optionally a pillar up the
/// middle.
pub fn build_tower(
    builder: &mut ColumnBuilder<'_>,
    defs: &WorldDefs,
    corner: (i32, i32),
    side: i32,
    storeys: i32,
    pillar: bool,
) {
    let wall = defs.block_state("core:wall", None).unwrap();
    let slab = defs.floor_state("core:slab", None).unwrap();
    let post = defs.block_state("core:pillar", None).unwrap();
    let (x0, y0) = corner;
    for z in 0..=storeys {
        for y in 0..side {
            for x in 0..side {
                let cell = at(x0 + x, y0 + y, z);
                if z >= 1 {
                    builder.set_floor(cell, slab);
                }
                if z == storeys {
                    continue;
                }
                let edge = x == 0 || y == 0 || x == side - 1 || y == side - 1;
                if edge {
                    builder.set_block(cell, wall);
                } else if pillar && x == side / 2 && y == side / 2 {
                    builder.set_block(cell, post);
                }
            }
        }
    }
}

/// The cells of the edge of a square footprint on one storey.
pub fn edge_cells(corner: (i32, i32), side: i32, z: i32) -> Vec<CellPos> {
    let (x0, y0) = corner;
    let mut cells = Vec::new();
    for y in 0..side {
        for x in 0..side {
            if x == 0 || y == 0 || x == side - 1 || y == side - 1 {
                cells.push(at(x0 + x, y0 + y, z));
            }
        }
    }
    cells
}
