//! Structural support and load (02 §14.1, §14.5): whether what stands
//! near a change is still held up, and whether what holds it up can
//! bear it.
//!
//! [`Cosmos::assess_stability`] reads the world and changes nothing. It
//! finds the structure joined to the cells it is given (the *island*),
//! works out support storey by storey from the bottom, sums loads down
//! the carriers from the top, and reports which blocks are crushed and
//! which elements nothing holds up. The collapse process acts on the
//! report (see [`crate::collapse`]).
//!
//! Everything here walks cells in a fixed order, so the same structure
//! gives the same report on every machine.

use std::collections::VecDeque;

use clash_core::Real;

use crate::chunk::DAMAGE_MAX;
use crate::cosmos::{Cosmos, GridView};
use crate::damage::DAMAGE_PARTS;
use crate::defs::{BlockState, ElementState, EngineFlag, Layer, WorldDefs};
use crate::dims::{CELL_M, CHUNK_Z_BITS};
use crate::error::WorldError;
use crate::grid::Column;
use crate::pos::{CellPos, GridId, Location, TractPos};
use crate::query::Footing;

/// How far sideways from a change, in cells, the structure joined to it
/// is examined. What lies further is assumed supported (02 §14.2). The
/// default of the setting `support.reach`.
pub const SUPPORT_REACH_CELLS: i32 = 24;

/// How far up and down from a change, in storeys, the structure joined
/// to it is examined. The default of the setting `support.reach_storeys`.
pub const SUPPORT_REACH_STOREYS: i32 = 40;

/// The most elements one check examines. Past this the rest are assumed
/// supported. The default of the setting `support.max_elements`.
pub const SUPPORT_MAX_ELEMENTS: usize = 1 << 17;

/// How many times one check will crush what is overloaded and look
/// again. What is still overloaded after that is found by the check
/// that the crushing itself asks for. The default of the setting
/// `support.max_passes`.
pub const SUPPORT_MAX_PASSES: u32 = 48;

/// The most cells the box round the changes of one check may hold.
const SUPPORT_MAX_BOX_CELLS: i64 = 1 << 22;

// What is known of a cell, as bits.
/// The cell has been read.
const READ: u16 = 1 << 0;
/// It has a floor.
const FLOOR: u16 = 1 << 1;
/// It has a block.
const BLOCK: u16 = 1 << 2;
/// Its block can hold up what is above it.
const BEARING: u16 = 1 << 3;
/// Its block is always supported: an anchor, implicit fill, or unknown.
const ANCHOR: u16 = 1 << 4;
/// Its floor lies on ground that is always supported, and so is too.
const FIXED: u16 = 1 << 5;
/// Its floor is part of the island.
const IN_FLOOR: u16 = 1 << 6;
/// Its block is part of the island.
const IN_BLOCK: u16 = 1 << 7;
/// Its floor is supported.
const OK_FLOOR: u16 = 1 << 8;
/// Its block is supported.
const OK_BLOCK: u16 = 1 << 9;

/// A node is an element of a cell of the box: the cell's index twice,
/// plus one for the block.
type Node = u32;

const NO_CARRIER: Node = u32::MAX;

const fn node(index: usize, layer: usize) -> Node {
    ((index as u32) << 1) | layer as u32
}

const fn split(node: Node) -> (usize, usize) {
    ((node >> 1) as usize, (node & 1) as usize)
}

/// The slot of a layer in a pair: floor, then block.
const fn slot(layer: Layer) -> Option<usize> {
    match layer {
        Layer::Floor => Some(0),
        Layer::Block => Some(1),
        Layer::Fixture => None,
    }
}

const IN: [u16; 2] = [IN_FLOOR, IN_BLOCK];
const OK: [u16; 2] = [OK_FLOOR, OK_BLOCK];
const LAYERS: [Layer; 2] = [Layer::Floor, Layer::Block];

/// A box of cells held as a flat array, storey by storey and row by row.
#[derive(Clone, Copy, Debug)]
struct Dense {
    min: CellPos,
    nx: i32,
    ny: i32,
    nz: i32,
}

impl Dense {
    fn new(min: CellPos, max: CellPos) -> Dense {
        Dense {
            min,
            nx: max.x.saturating_sub(min.x).saturating_add(1).max(1),
            ny: max.y.saturating_sub(min.y).saturating_add(1).max(1),
            nz: max.z.saturating_sub(min.z).saturating_add(1).max(1),
        }
    }

    fn len(&self) -> usize {
        (i64::from(self.nx) * i64::from(self.ny) * i64::from(self.nz)) as usize
    }

    fn row(&self) -> usize {
        self.nx as usize
    }

    fn plane(&self) -> usize {
        (self.nx * self.ny) as usize
    }

    /// The index of a cell, if it is in the box.
    fn index(&self, cell: CellPos) -> Option<usize> {
        let x = cell.x.wrapping_sub(self.min.x);
        let y = cell.y.wrapping_sub(self.min.y);
        let z = cell.z.wrapping_sub(self.min.z);
        let inside =
            (0..self.nx).contains(&x) && (0..self.ny).contains(&y) && (0..self.nz).contains(&z);
        inside.then(|| ((z * self.ny + y) * self.nx + x) as usize)
    }

    /// The cell at an index. Not normalised: a box may reach across a
    /// wrapped edge.
    fn cell(&self, index: usize) -> CellPos {
        let index = index as i32;
        let x = index % self.nx;
        let y = (index / self.nx) % self.ny;
        let z = index / (self.nx * self.ny);
        CellPos::new(
            self.min.x.wrapping_add(x),
            self.min.y.wrapping_add(y),
            self.min.z.wrapping_add(z),
        )
    }

    /// Whether an index is on a face of the box.
    fn on_face(&self, index: usize) -> bool {
        let index = index as i32;
        let x = index % self.nx;
        let y = (index / self.nx) % self.ny;
        let z = index / (self.nx * self.ny);
        x == 0 || y == 0 || z == 0 || x == self.nx - 1 || y == self.ny - 1 || z == self.nz - 1
    }

    /// The offsets, in indices, to the eight cells round a cell on its
    /// own storey: clockwise from north.
    fn hood(&self) -> [isize; 8] {
        let row = self.nx as isize;
        [-row, -row + 1, 1, row + 1, row, row - 1, -1, -row - 1]
    }
}

/// The first walk: finds the island, reading each cell once.
struct Probe<'a> {
    view: GridView<'a>,
    defs: &'a WorldDefs,
    dense: Dense,
    bits: Vec<u16>,
    column: Option<(TractPos, Option<&'a Column>)>,
    queue: VecDeque<Node>,
    count: usize,
    truncated: bool,
    low: CellPos,
    high: CellPos,
}

impl<'a> Probe<'a> {
    fn column(&mut self, tract: TractPos) -> Option<&'a Column> {
        match self.column {
            Some((have, column)) if have == tract => column,
            _ => {
                let column = self.view.column(tract);
                self.column = Some((tract, column));
                column
            }
        }
    }

    /// What is known of a cell, read from the world the first time.
    fn read(&mut self, index: usize) -> u16 {
        let have = self.bits[index];
        if have & READ != 0 {
            return have;
        }
        let mut bits = READ;
        if let Ok(cell) = self.view.normalize(self.dense.cell(index)) {
            match self.column(cell.tract()) {
                // Not loaded: there is no telling, so it holds.
                None => bits |= FLOOR | BLOCK | BEARING | ANCHOR | FIXED,
                Some(column) => {
                    let held = column.cell(cell);
                    if !held.floor.is_none() {
                        bits |= FLOOR;
                    }
                    let flags = self.defs.block_info(held.block).flags;
                    // A still liquid is not structure: it neither falls
                    // nor holds anything up.
                    if !held.block.is_none() && !flags.has(EngineFlag::Liquid) {
                        bits |= BLOCK;
                        // Fill outside the stored cells is solid ground.
                        let implicit = column.chunk(cell.z >> CHUNK_Z_BITS).is_none();
                        if implicit || flags.has(EngineFlag::LoadBearing) {
                            bits |= BEARING;
                        }
                        if implicit || flags.has(EngineFlag::Anchor) {
                            bits |= ANCHOR;
                        }
                    }
                }
            }
        }
        self.bits[index] = bits;
        bits
    }

    fn enter(&mut self, index: usize, layer: usize) {
        if self.count >= SUPPORT_MAX_ELEMENTS {
            self.truncated = true;
            return;
        }
        self.bits[index] |= IN[layer];
        self.count += 1;
        self.queue.push_back(node(index, layer));
        let cell = self.dense.cell(index);
        if self.count == 1 {
            (self.low, self.high) = (cell, cell);
        } else {
            self.low = CellPos::new(
                self.low.x.min(cell.x),
                self.low.y.min(cell.y),
                self.low.z.min(cell.z),
            );
            self.high = CellPos::new(
                self.high.x.max(cell.x),
                self.high.y.max(cell.y),
                self.high.z.max(cell.z),
            );
        }
    }

    /// Takes a cell's floor into the island, unless it lies on ground
    /// that is always supported or on a face of the box.
    fn add_floor(&mut self, index: usize) {
        let bits = self.read(index);
        if bits & FLOOR == 0 || bits & (IN_FLOOR | FIXED) != 0 {
            return;
        }
        let ground = BLOCK | BEARING | ANCHOR;
        if self.dense.on_face(index) || self.read(index - self.dense.plane()) & ground == ground {
            self.bits[index] |= FIXED;
            return;
        }
        self.enter(index, 0);
    }

    /// Takes a cell's block into the island, unless it is always
    /// supported or on a face of the box.
    fn add_block(&mut self, index: usize) {
        let bits = self.read(index);
        if bits & BLOCK == 0 || bits & (IN_BLOCK | ANCHOR) != 0 {
            return;
        }
        if self.dense.on_face(index) {
            self.bits[index] |= ANCHOR;
            return;
        }
        self.enter(index, 1);
    }

    /// Takes in whatever a change in a cell could have been holding up:
    /// what is in the cell, round it and over it.
    fn seed(&mut self, cell: CellPos) {
        let Some(index) = self.dense.index(cell) else {
            return;
        };
        if self.dense.on_face(index) {
            return;
        }
        self.add_floor(index);
        self.add_block(index);
        for step in self.dense.hood() {
            let beside = index.wrapping_add_signed(step);
            self.add_floor(beside);
            self.add_block(beside);
        }
        let above = index + self.dense.plane();
        self.add_floor(above);
        self.add_block(above);
    }

    /// Follows the structure outward from what has been taken in: along
    /// every way support or load can pass.
    fn flood(&mut self) {
        let hood = self.dense.hood();
        let plane = self.dense.plane();
        while let Some(next) = self.queue.pop_front() {
            let (index, layer) = split(next);
            if layer == 0 {
                self.add_block(index);
                for step in hood {
                    self.add_floor(index.wrapping_add_signed(step));
                }
                self.add_block(index - plane);
            } else {
                self.add_floor(index);
                for step in hood {
                    self.add_block(index.wrapping_add_signed(step));
                }
                self.add_floor(index + plane);
                self.add_block(index + plane);
                self.add_block(index - plane);
            }
        }
    }
}

/// The second walk's ground: the island and a margin of one cell round
/// it, with what the passes work out.
#[derive(Clone, Debug)]
struct Island {
    dense: Dense,
    bits: Vec<u16>,
    floor: Vec<ElementState>,
    block: Vec<ElementState>,
    /// The damage of each block, in 255ths.
    damage: Vec<u8>,
    /// How far each element can reach from support, floor then block.
    span: Vec<[u16; 2]>,
    /// The mass of the fixture in each cell.
    fixture: Vec<Real>,
    /// How far each element is from its source of support.
    dist: Vec<[u16; 2]>,
    /// The element each element's support came from.
    carrier: Vec<[Node; 2]>,
    /// What each element carries, its own mass included, in kilograms.
    load: Vec<[Real; 2]>,
    /// The supported elements of each storey, in the order support
    /// reached them.
    order: Vec<Vec<Node>>,
}

impl Island {
    fn span_of(defs: &WorldDefs, state: ElementState) -> u16 {
        let span = defs
            .materials()
            .get(state.material)
            .and_then(|material| material.span)
            .unwrap_or(0);
        u16::try_from(span).unwrap_or(u16::MAX)
    }

    /// Reads the island's box out of the world.
    fn load_from(probe: &mut Probe<'_>) -> Island {
        let one = clash_core::IVec3::new(1, 1, 1);
        let dense = Dense::new(
            probe.low.offset(clash_core::IVec3::new(-1, -1, -1)),
            probe.high.offset(one),
        );
        let len = dense.len();
        let mut island = Island {
            dense,
            bits: vec![0; len],
            floor: vec![ElementState::NONE; len],
            block: vec![ElementState::NONE; len],
            damage: vec![0; len],
            span: vec![[0; 2]; len],
            fixture: vec![Real::ZERO; len],
            dist: vec![[0; 2]; len],
            carrier: vec![[NO_CARRIER; 2]; len],
            load: vec![[Real::ZERO; 2]; len],
            order: vec![Vec::new(); dense.nz as usize],
        };
        let defs = probe.defs;
        for index in 0..len {
            let cell = dense.cell(index);
            let Some(outer) = probe.dense.index(cell) else {
                continue;
            };
            let bits = probe.read(outer);
            island.bits[index] = bits;
            // An empty cell matters only for a fixture standing on the
            // block below it.
            let over_block =
                index >= dense.plane() && island.bits[index - dense.plane()] & BLOCK != 0;
            if bits & (FLOOR | BLOCK) == 0 && !over_block {
                continue;
            }
            let Ok(cell) = probe.view.normalize(cell) else {
                continue;
            };
            let Some(column) = probe.column(cell.tract()) else {
                continue;
            };
            let held = column.cell(cell);
            island.floor[index] = held.floor.raw();
            island.block[index] = held.block.raw();
            island.span[index] = [
                Island::span_of(defs, held.floor.raw()),
                Island::span_of(defs, held.block.raw()),
            ];
            if !held.block.is_none() {
                island.damage[index] = column.damage(cell)[Layer::Block.index()];
            }
            if !held.fixture.is_none() {
                island.fixture[index] = defs.bulk_mass(Layer::Fixture, held.fixture.raw());
            }
        }
        island
    }

    fn accept(&mut self, index: usize, layer: usize, dist: u16, from: Node) {
        self.bits[index] |= OK[layer];
        self.dist[index][layer] = dist;
        self.carrier[index][layer] = from;
    }

    /// Offers support to an element at a distance. It takes it if it is
    /// part of the island, has none yet, and can reach that far.
    fn offer(&mut self, index: usize, layer: usize, dist: u16, from: Node, next: &mut Vec<Node>) {
        let bits = self.bits[index];
        if bits & IN[layer] == 0 || bits & OK[layer] != 0 || self.span[index][layer] < dist {
            return;
        }
        self.accept(index, layer, dist, from);
        next.push(node(index, layer));
    }

    /// An element beside this one that is not part of the island, and so
    /// is taken to be supported: support can come from it.
    fn outside_support(&self, index: usize, layer: usize, hood: &[isize; 8]) -> Option<Node> {
        let has = [FLOOR, BLOCK];
        for step in hood {
            let beside = index.wrapping_add_signed(*step);
            let bits = self.bits[beside];
            if bits & has[layer] != 0 && bits & IN[layer] == 0 {
                return Some(node(beside, layer));
            }
        }
        // The other element of its own cell.
        let other = 1 - layer;
        let bits = self.bits[index];
        (bits & has[other] != 0 && bits & IN[other] == 0).then(|| node(index, other))
    }

    /// The support pass (02 §14.1): storey by storey from the bottom,
    /// support spreads from its sources as far as each element's span
    /// lets it, and each element records where its support came from.
    fn support(&mut self) {
        for bits in &mut self.bits {
            *bits &= !(OK_FLOOR | OK_BLOCK);
        }
        for carrier in &mut self.carrier {
            *carrier = [NO_CARRIER; 2];
        }
        let hood = self.dense.hood();
        let (row, plane) = (self.dense.row(), self.dense.plane());
        let (nx, ny, nz) = (
            self.dense.nx as usize,
            self.dense.ny as usize,
            self.dense.nz as usize,
        );
        let mut level: Vec<Node> = Vec::new();
        let mut next: Vec<Node> = Vec::new();
        for z in 1..nz.saturating_sub(1) {
            level.clear();
            next.clear();
            self.order[z].clear();
            // Sources: what stands on a supported load-bearing block.
            for y in 1..ny - 1 {
                for x in 1..nx - 1 {
                    let index = z * plane + y * row + x;
                    let bits = self.bits[index];
                    if bits & (IN_FLOOR | IN_BLOCK) == 0 {
                        continue;
                    }
                    let under = index - plane;
                    let below = self.bits[under];
                    let stands = below & (BLOCK | BEARING) == (BLOCK | BEARING)
                        && (below & IN_BLOCK == 0 || below & OK_BLOCK != 0);
                    if !stands {
                        continue;
                    }
                    for (layer, part) in IN.iter().enumerate() {
                        if bits & part != 0 {
                            self.accept(index, layer, 0, node(under, 1));
                            level.push(node(index, layer));
                        }
                    }
                }
            }
            // What hangs from something outside the island.
            for y in 1..ny - 1 {
                for x in 1..nx - 1 {
                    let index = z * plane + y * row + x;
                    for (layer, part) in IN.iter().enumerate() {
                        let bits = self.bits[index];
                        if bits & part == 0 || bits & OK[layer] != 0 || self.span[index][layer] < 1
                        {
                            continue;
                        }
                        if let Some(from) = self.outside_support(index, layer, &hood) {
                            self.accept(index, layer, 1, from);
                            next.push(node(index, layer));
                        }
                    }
                }
            }
            // Spread, a step at a time.
            let mut dist: u16 = 0;
            loop {
                for from in level.iter().copied() {
                    let (index, layer) = split(from);
                    let reach = dist.saturating_add(1);
                    // Between a floor and a block in one cell, and
                    // between like elements of cells side by side.
                    self.offer(index, 1 - layer, reach, from, &mut next);
                    for step in hood {
                        self.offer(
                            index.wrapping_add_signed(step),
                            layer,
                            reach,
                            from,
                            &mut next,
                        );
                    }
                }
                self.order[z].extend_from_slice(&level);
                if next.is_empty() {
                    break;
                }
                core::mem::swap(&mut level, &mut next);
                next.clear();
                dist = dist.saturating_add(1);
            }
        }
    }

    /// The load pass (02 §14.5): from the top storey down, each element
    /// carries its own mass and what its carrier role brings it. Returns
    /// the blocks that carry more than they can bear.
    fn weigh(&mut self, view: &GridView<'_>, defs: &WorldDefs) -> Vec<usize> {
        for load in &mut self.load {
            *load = [Real::ZERO; 2];
        }
        let plane = self.dense.plane();
        let nz = self.dense.nz as usize;
        for z in (1..nz.saturating_sub(1)).rev() {
            for at in (0..self.order[z].len()).rev() {
                let (index, layer) = split(self.order[z][at]);
                // Fixtures and rubble count with the cell they are in:
                // on its floor, or on the block below a cell without one.
                let own = if layer == 0 {
                    defs.mass(Layer::Floor, self.floor[index]) + self.fixture[index]
                } else {
                    let above = index + plane;
                    let stood_on = if self.bits[above] & FLOOR == 0 {
                        self.fixture[above]
                    } else {
                        Real::ZERO
                    };
                    defs.mass(Layer::Block, self.block[index]) + stood_on
                };
                let total = self.load[index][layer] + own;
                self.load[index][layer] = total;
                let carrier = self.carrier[index][layer];
                if carrier != NO_CARRIER {
                    let (onto, onto_layer) = split(carrier);
                    if self.bits[onto] & IN[onto_layer] != 0 {
                        self.load[onto][onto_layer] += total;
                    }
                }
            }
        }

        let parts = Real::from_i64(i64::from(DAMAGE_PARTS));
        let wanted = IN_BLOCK | OK_BLOCK | BEARING;
        let mut crushed = Vec::new();
        for index in 0..self.bits.len() {
            if self.bits[index] & wanted != wanted {
                continue;
            }
            let state = self.block[index];
            let Some(bearing) = defs
                .materials()
                .get(state.material)
                .and_then(|material| material.bearing)
            else {
                continue;
            };
            let area = defs.block_info(BlockState::from_raw(state)).fill * CELL_M * CELL_M;
            let gravity = gravity_at(view, self.dense.cell(index));
            if area <= Real::ZERO || gravity <= Real::ZERO {
                continue;
            }
            let damage = Real::from_i64(i64::from(self.damage[index].min(DAMAGE_MAX)));
            let stress = self.load[index][1] * gravity / area;
            if stress > bearing * (Real::ONE - damage / parts) {
                crushed.push(index);
            }
        }
        crushed
    }

    /// Takes a crushed block out, as far as the passes are concerned.
    fn remove_block(&mut self, index: usize) {
        self.bits[index] &= !(BLOCK | BEARING | IN_BLOCK | OK_BLOCK);
        self.block[index] = ElementState::NONE;
    }
}

/// Gravity along −z at a cell, in m/s². Zero where it cannot be read.
fn gravity_at(view: &GridView<'_>, cell: CellPos) -> Real {
    view.ambient(cell)
        .map_or(Real::ZERO, |ambient| ambient.gravity)
}

/// What a stability check found (02 §14): the structure joined to some
/// cells, which of its blocks carry more than they can bear, and which
/// of its elements nothing holds up.
#[derive(Clone, Debug)]
pub struct Assessment {
    grid: GridId,
    island: Option<Island>,
    crushed: Vec<CellPos>,
    falling: Vec<(CellPos, Layer)>,
    elements: usize,
    passes: u32,
    truncated: bool,
}

impl Assessment {
    /// The grid that was examined.
    pub fn grid(&self) -> GridId {
        self.grid
    }

    /// Whether everything examined stands: nothing is crushed and
    /// nothing falls.
    pub fn is_stable(&self) -> bool {
        self.crushed.is_empty() && self.falling.is_empty()
    }

    /// The load-bearing blocks that carry more than they can bear, in
    /// the order they give way: pass by pass, and within a pass storey
    /// by storey and row by row.
    pub fn crushed(&self) -> &[CellPos] {
        &self.crushed
    }

    /// The floors and blocks that nothing holds up once the crushed
    /// blocks are gone, and the fixtures in the changed cells and over
    /// them that have nothing left to stand on: column by column (row
    /// by row), each from the bottom up, a cell's floor before its block
    /// and its fixture last.
    pub fn falling(&self) -> &[(CellPos, Layer)] {
        &self.falling
    }

    /// How many floors and blocks were examined.
    pub fn elements(&self) -> usize {
        self.elements
    }

    /// How many times support and load were worked out: once, and once
    /// more for each round of crushing.
    pub fn passes(&self) -> u32 {
        self.passes
    }

    /// Whether the structure reached further than one check examines.
    /// What was not examined is assumed supported.
    pub fn truncated(&self) -> bool {
        self.truncated
    }

    fn at(&self, cell: CellPos, layer: Layer) -> Option<(&Island, usize, usize)> {
        let island = self.island.as_ref()?;
        let index = island.dense.index(cell)?;
        let layer = slot(layer)?;
        (island.bits[index] & IN[layer] != 0).then_some((island, index, layer))
    }

    /// Whether a floor or block was examined and found supported.
    /// `None` for one that was not examined.
    pub fn supported(&self, cell: CellPos, layer: Layer) -> Option<bool> {
        let (island, index, layer) = self.at(cell, layer)?;
        Some(island.bits[index] & OK[layer] != 0)
    }

    /// The element a supported floor or block takes its support from:
    /// its carrier (02 §14.5). A floor's is the block it rests on or the
    /// neighbour its support came through; a block's is the block
    /// beneath it, the floor it stands on, or the neighbour it hangs
    /// from.
    pub fn carrier(&self, cell: CellPos, layer: Layer) -> Option<(CellPos, Layer)> {
        let (island, index, layer) = self.at(cell, layer)?;
        if island.bits[index] & OK[layer] == 0 {
            return None;
        }
        let (onto, onto_layer) = split(island.carrier[index][layer]);
        (onto < island.bits.len()).then(|| (island.dense.cell(onto), LAYERS[onto_layer]))
    }

    /// How many steps a supported floor or block is from its source of
    /// support. Zero for one that stands on a load-bearing block.
    pub fn distance(&self, cell: CellPos, layer: Layer) -> Option<u32> {
        let (island, index, layer) = self.at(cell, layer)?;
        (island.bits[index] & OK[layer] != 0).then(|| u32::from(island.dist[index][layer]))
    }

    /// What a supported floor or block carries, in kilograms: its own
    /// mass, the fixtures that stand on it, and the loads of everything
    /// it is the carrier of.
    pub fn load(&self, cell: CellPos, layer: Layer) -> Option<Real> {
        let (island, index, layer) = self.at(cell, layer)?;
        (island.bits[index] & OK[layer] != 0).then(|| island.load[index][layer])
    }
}

impl Cosmos {
    /// Checks whether the structure joined to some cells of a grid still
    /// stands (02 §14). Reads the world and changes nothing.
    ///
    /// The cells are where something structural was removed or
    /// weakened. The check examines the floors and blocks joined to
    /// them, within [`SUPPORT_REACH_CELLS`] and [`SUPPORT_REACH_STOREYS`]
    /// of them and up to [`SUPPORT_MAX_ELEMENTS`] elements; whatever
    /// lies beyond, or is not loaded, is assumed supported.
    ///
    /// - **Support** (02 §14.1). Storey by storey from the bottom, the
    ///   sources of support are a block or floor standing on a supported
    ///   `load_bearing` block, or on solid fill. Blocks flagged `anchor`
    ///   and floors laid on them are always supported. A block flagged
    ///   `liquid` is not structure. Support spreads a
    ///   step at a time between a floor and a block in one cell, between
    ///   floors of the eight cells round a cell and between blocks of
    ///   them. An element takes it only at a distance within its
    ///   material's `span`.
    /// - **Load** (02 §14.5). From the top down, an element carries its
    ///   own mass, the fixtures standing on it, and the load of every
    ///   element whose support came from it. A `load_bearing` block
    ///   whose material has a `bearing` is crushed when
    ///   `load × gravity / (fill × 1 m²)` exceeds
    ///   `bearing × (1 − damage / 255)`, with the ambient gravity of its
    ///   own cell. The blocks crushed in one pass go together, and both
    ///   passes run again without them, up to [`SUPPORT_MAX_PASSES`]
    ///   times.
    /// - Where gravity is zero or less, nothing is crushed and nothing
    ///   falls.
    ///
    /// Refused if the grid is unknown, or the cells lie too far apart
    /// for one check.
    pub fn assess_stability(
        &self,
        grid: GridId,
        cells: &[CellPos],
    ) -> Result<Assessment, WorldError> {
        let view = self.grid(grid)?;
        let mut assessment = Assessment {
            grid,
            island: None,
            crushed: Vec::new(),
            falling: Vec::new(),
            elements: 0,
            passes: 0,
            truncated: false,
        };
        let Some(first) = cells.first() else {
            return Ok(assessment);
        };
        let (mut low, mut high) = (*first, *first);
        for cell in cells {
            low = CellPos::new(low.x.min(cell.x), low.y.min(cell.y), low.z.min(cell.z));
            high = CellPos::new(high.x.max(cell.x), high.y.max(cell.y), high.z.max(cell.z));
        }
        let topology = view.topology();
        let (across, up) = (SUPPORT_REACH_CELLS, SUPPORT_REACH_STOREYS);
        let low = CellPos::new(
            low.x.saturating_sub(across),
            low.y.saturating_sub(across),
            low.z
                .saturating_sub(up)
                .max(topology.z_min.saturating_sub(1)),
        );
        let high = CellPos::new(
            high.x.saturating_add(across),
            high.y.saturating_add(across),
            high.z
                .saturating_add(up)
                .min(topology.z_max.saturating_add(1)),
        );
        let dense = Dense::new(low, high);
        let volume = i64::from(dense.nx) * i64::from(dense.ny) * i64::from(dense.nz);
        if volume > SUPPORT_MAX_BOX_CELLS {
            return Err(WorldError::TooLarge {
                what: "the box of cells round the changes of one stability check",
                limit: SUPPORT_MAX_BOX_CELLS as u64,
            });
        }

        let mut probe = Probe {
            view,
            defs: self.defs(),
            dense,
            bits: vec![0; dense.len()],
            column: None,
            queue: VecDeque::new(),
            count: 0,
            truncated: false,
            low: *first,
            high: *first,
        };
        for cell in cells {
            probe.seed(*cell);
        }
        probe.flood();
        assessment.elements = probe.count;
        assessment.truncated = probe.truncated;
        if probe.count == 0 {
            assessment.falling = self.loose_fixtures(&view, cells);
            return Ok(assessment);
        }

        let mut island = Island::load_from(&mut probe);
        let defs = self.defs();
        loop {
            island.support();
            assessment.passes += 1;
            let crushed = island.weigh(&view, defs);
            if crushed.is_empty() {
                break;
            }
            for index in &crushed {
                assessment.crushed.push(island.dense.cell(*index));
                island.remove_block(*index);
            }
            if assessment.passes >= SUPPORT_MAX_PASSES {
                island.support();
                break;
            }
        }

        // What nothing holds up, column by column from the bottom.
        let (row, plane) = (island.dense.row(), island.dense.plane());
        let (nx, ny, nz) = (
            island.dense.nx as usize,
            island.dense.ny as usize,
            island.dense.nz as usize,
        );
        for y in 0..ny {
            for x in 0..nx {
                for z in 0..nz {
                    let index = z * plane + y * row + x;
                    let bits = island.bits[index];
                    for layer in 0..2 {
                        if bits & IN[layer] == 0 || bits & OK[layer] != 0 {
                            continue;
                        }
                        let cell = island.dense.cell(index);
                        // Without weight nothing falls.
                        if gravity_at(&view, cell) > Real::ZERO {
                            assessment.falling.push((cell, LAYERS[layer]));
                        }
                    }
                }
            }
        }
        assessment.island = Some(island);
        // With what stood on something that is gone.
        let loose = self.loose_fixtures(&view, cells);
        if !loose.is_empty() {
            assessment.falling.extend(loose);
            assessment
                .falling
                .sort_by_key(|(cell, layer)| (cell.y, cell.x, cell.z, *layer));
            assessment.falling.dedup();
        }
        Ok(assessment)
    }

    /// The fixtures in the changed cells, and in the cells over them,
    /// that nothing holds up any more: no floor, no solid top and no
    /// way up beneath them. They fall like anything else.
    fn loose_fixtures(&self, view: &GridView<'_>, cells: &[CellPos]) -> Vec<(CellPos, Layer)> {
        let mut loose = Vec::new();
        for cell in cells.iter().flat_map(|cell| [*cell, cell.above()]) {
            let Ok(cell) = view.normalize(cell) else {
                continue;
            };
            let has_fixture = view.cell(cell).is_ok_and(|held| !held.fixture.is_none());
            let at = Location::new(view.id(), cell);
            if has_fixture
                && matches!(self.footing(at), Ok(Footing::None))
                && gravity_at(view, cell) > Real::ZERO
            {
                loose.push((cell, Layer::Fixture));
            }
        }
        loose.sort_by_key(|(cell, layer)| (cell.y, cell.x, cell.z, *layer));
        loose.dedup();
        loose
    }
}
