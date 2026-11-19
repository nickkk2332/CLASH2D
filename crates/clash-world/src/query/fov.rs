//! Field of view: [`Cosmos::field_of_view`] and the [`FovSet`] it fills
//! (02 §22).
//!
//! On one storey this is symmetric shadowcasting over whole numbers: a
//! cell that does not block sight is seen when the line from the middle
//! of the viewer's cell to the middle of that cell is clear, so between
//! two such cells sight goes both ways or neither. Between storeys it
//! goes straight down and straight up through cells that nothing closes.

use clash_core::{IVec3, Real};

use crate::cosmos::{Cosmos, GridView};
use crate::defs::{CellFlags, EngineFlag, StateInfo, WorldDefs};
use crate::dims::{CELL_M, STOREY_CELLS, STOREY_M, STOREY_MM};
use crate::error::WorldError;
use crate::grid::Column;
use crate::pos::{CellBox, CellPos, Location, TractPos};
use crate::topology::{Extent, Topology};

use super::stamp::{RevParts, RevStamp};

/// The furthest a field of view reaches, in cells. A larger radius is
/// cut to this.
pub const FOV_MAX_REACH: i32 = 255;

/// The bits of one word of a [`FovSet`].
const WORD_BITS: usize = u64::BITS as usize;

/// Whether an answer was worked out or was already there.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Freshness {
    /// The answer was worked out now.
    Computed,
    /// The buffer already held the answer to this question, and no cell
    /// it depends on has changed since.
    Reused,
}

/// The question a [`FovSet`] holds the answer to.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
struct Asked {
    origin: Location,
    eye_alt_mm: u16,
    radius_m: Real,
}

/// A slope as a fraction with a positive denominator.
#[derive(Clone, Copy, Debug)]
struct Slope {
    num: i64,
    den: i64,
}

impl Slope {
    /// The slope of the edge of a cell's diamond nearer the axis.
    fn of(col: i64, depth: i64) -> Slope {
        Slope {
            num: 2 * col - 1,
            den: 2 * depth,
        }
    }

    /// `depth × slope`, rounded to the nearest whole number, halves up.
    fn first_col(self, depth: i64) -> i64 {
        (2 * depth * self.num + self.den).div_euclid(2 * self.den)
    }

    /// `depth × slope`, rounded to the nearest whole number, halves down.
    fn last_col(self, depth: i64) -> i64 {
        -(self.den - 2 * depth * self.num).div_euclid(2 * self.den)
    }
}

/// One row of a quadrant still to be scanned, between two slopes.
#[derive(Clone, Copy, Debug)]
struct Row {
    depth: i64,
    start: Slope,
    end: Slope,
}

/// Reads cells of one grid, remembering the last tract it looked up:
/// neighbouring cells are nearly always in the same one.
struct CellReader<'a> {
    view: GridView<'a>,
    tract: Option<(TractPos, Option<&'a Column>)>,
}

impl<'a> CellReader<'a> {
    fn new(view: GridView<'a>) -> CellReader<'a> {
        CellReader { view, tract: None }
    }

    /// The column that holds a cell and the cell normalised, or `None`
    /// if it is not loaded or lies beyond an edge.
    fn locate(&mut self, cell: CellPos) -> Option<(&'a Column, CellPos)> {
        let cell = self.view.normalize(cell).ok()?;
        let tract = cell.tract();
        let column = match self.tract {
            Some((have, column)) if have == tract => column,
            _ => {
                let column = self.view.column(tract);
                self.tract = Some((tract, column));
                column
            }
        };
        Some((column?, cell))
    }

    fn flags(&mut self, cell: CellPos) -> Option<CellFlags> {
        let (column, cell) = self.locate(cell)?;
        Some(column.flags(cell))
    }

    /// Whether a cell blocks sight at eye level: something `opaque` in it
    /// stands taller than the eye. `None` if the cell cannot be read.
    fn blocks_sight(&mut self, defs: &WorldDefs, cell: CellPos, eye_m: Real) -> Option<bool> {
        let (column, cell) = self.locate(cell)?;
        if !column.flags(cell).contains(CellFlags::OPAQUE) {
            return Some(false);
        }
        let taller = |info: &StateInfo| info.flags.has(EngineFlag::Opaque) && info.height > eye_m;
        Some(
            taller(defs.block_info(column.block(cell)))
                || taller(defs.fixture_info(column.fixture(cell))),
        )
    }
}

/// The whole part of the square root of a number that is not negative.
fn isqrt(n: i64) -> i64 {
    if n <= 0 {
        return 0;
    }
    let mut root = Real::from_i64(n).sqrt().floor().to_i64();
    while root * root > n {
        root -= 1;
    }
    while (root + 1) * (root + 1) <= n {
        root += 1;
    }
    root
}

/// The cells a viewer sees: the answer of [`Cosmos::field_of_view`].
///
/// A buffer to keep and hand back each time a view is wanted. It
/// remembers the question it last answered and the revisions of the
/// chunks the answer was read from, so asking the same question of an
/// unchanged world costs a few comparisons, and once it has grown to the
/// size of a view it allocates nothing.
///
/// Cells are held as offsets from the viewer's cell, so a view across a
/// wrapped edge is one piece.
///
/// ```
/// use clash_world::FovSet;
///
/// let view = FovSet::new();
/// assert!(view.is_empty());
/// assert_eq!(view.origin(), None);
/// ```
#[derive(Clone, Debug)]
pub struct FovSet {
    asked: Option<Asked>,
    topology: Topology,
    /// Cells out from the origin on `x` and on `y`.
    reach: i32,
    /// Storeys out from the origin, up and down.
    storeys: i32,
    /// The square of the radius, in cell widths.
    radius_sq: i64,
    /// Words in one row of one storey.
    row_words: usize,
    /// One bit per cell: storey by storey, row by row.
    bits: Vec<u64>,
    /// Which storeys hold anything. The words of the others are stale.
    used: Vec<bool>,
    count: usize,
    stamp: RevStamp,
    /// Rows waiting to be scanned.
    stack: Vec<Row>,
}

impl Default for FovSet {
    fn default() -> FovSet {
        FovSet::new()
    }
}

impl FovSet {
    /// An empty view.
    pub fn new() -> FovSet {
        FovSet {
            asked: None,
            topology: Topology::unbounded(),
            reach: 0,
            storeys: 0,
            radius_sq: 0,
            row_words: 1,
            bits: Vec::new(),
            used: Vec::new(),
            count: 0,
            stamp: RevStamp::new(RevParts::STRUCTURE),
            stack: Vec::new(),
        }
    }

    /// Empties the view and forgets the question, keeping the storage.
    /// The next [`field_of_view`](Cosmos::field_of_view) works its
    /// answer out afresh.
    pub fn clear(&mut self) {
        self.asked = None;
        self.count = 0;
        self.used.iter_mut().for_each(|used| *used = false);
        self.stamp.clear();
    }

    /// The viewer's cell, or `None` if the view is empty.
    pub fn origin(&self) -> Option<Location> {
        self.asked.map(|asked| asked.origin)
    }

    /// The height of the eye above the base of the viewer's cell, in
    /// millimetres.
    pub fn eye_alt_mm(&self) -> u16 {
        self.asked.map_or(0, |asked| asked.eye_alt_mm)
    }

    /// How far the view reaches, in metres.
    pub fn radius_m(&self) -> Real {
        self.asked.map_or(Real::ZERO, |asked| asked.radius_m)
    }

    /// How many cells are seen.
    pub fn len(&self) -> usize {
        self.count
    }

    /// Whether nothing is seen.
    pub fn is_empty(&self) -> bool {
        self.count == 0
    }

    /// Whether the answer still stands: no cell it was read from has
    /// been edited since. An empty view is never current.
    pub fn is_current(&self, cosmos: &Cosmos) -> bool {
        self.asked.is_some() && self.stamp.is_current(cosmos)
    }

    /// The chunk revisions the answer was read from.
    pub fn stamp(&self) -> &RevStamp {
        &self.stamp
    }

    fn side(&self) -> usize {
        2 * self.reach as usize + 1
    }

    fn layer_words(&self) -> usize {
        self.side() * self.row_words
    }

    /// The word and bit of an offset, if it is inside the buffer.
    fn slot(&self, offset: IVec3) -> Option<(usize, usize, u64)> {
        let inside = (-self.reach..=self.reach).contains(&offset.x)
            && (-self.reach..=self.reach).contains(&offset.y)
            && (-self.storeys..=self.storeys).contains(&offset.z);
        if !inside {
            return None;
        }
        let layer = (offset.z + self.storeys) as usize;
        let row = (offset.y + self.reach) as usize;
        let col = (offset.x + self.reach) as usize;
        let word = layer * self.layer_words() + row * self.row_words + col / WORD_BITS;
        Some((layer, word, 1 << (col % WORD_BITS)))
    }

    /// Whether the cell at an offset from the viewer's cell is seen.
    pub fn contains_offset(&self, offset: IVec3) -> bool {
        match self.slot(offset) {
            // A view that has answered nothing has no storeys at all.
            Some((layer, word, bit)) => {
                self.used.get(layer).is_some_and(|used| *used)
                    && self.bits.get(word).is_some_and(|bits| bits & bit != 0)
            }
            None => false,
        }
    }

    /// Whether a cell is seen.
    pub fn contains(&self, loc: Location) -> bool {
        match self.asked {
            Some(asked) if asked.origin.grid == loc.grid => {
                self.contains_offset(self.topology.delta(asked.origin.cell, loc.cell))
            }
            _ => false,
        }
    }

    /// The lowest and highest storeys that hold a seen cell, as offsets
    /// from the viewer's storey. `None` if nothing is seen.
    pub fn storeys(&self) -> Option<(i32, i32)> {
        let low = self.used.iter().position(|used| *used)?;
        let high = self.used.iter().rposition(|used| *used)?;
        Some((low as i32 - self.storeys, high as i32 - self.storeys))
    }

    /// The seen cells as offsets from the viewer's cell, storey by
    /// storey, row by row.
    pub fn offsets(&self) -> FovOffsets<'_> {
        FovOffsets {
            set: self,
            word: 0,
            end: self.used.len() * self.layer_words(),
            pending: 0,
            started: false,
        }
    }

    /// The seen cells as offsets from the viewer's cell on the storeys
    /// from `low` to `high` (as offsets from the viewer's storey) alone:
    /// [`offsets`](FovSet::offsets) without the others, and without
    /// passing over them.
    pub fn offsets_between(&self, low: i32, high: i32) -> FovOffsets<'_> {
        let layers = self.used.len() as i64;
        let first = (i64::from(low) + i64::from(self.storeys)).clamp(0, layers) as usize;
        let last = (i64::from(high) + i64::from(self.storeys) + 1).clamp(0, layers) as usize;
        let words = self.layer_words();
        FovOffsets {
            set: self,
            word: first * words,
            end: (last * words).max(first * words),
            pending: 0,
            started: false,
        }
    }

    /// The seen cells, in the order of [`offsets`](FovSet::offsets):
    /// storey by storey, row by row, outward from no particular side.
    /// Across a wrapped edge the positions wrap and the order does not.
    pub fn cells(&self) -> impl Iterator<Item = Location> + '_ {
        let origin = self.asked.map(|asked| asked.origin);
        self.offsets().filter_map(move |offset| {
            let origin = origin?;
            let cell = self.topology.normalize(origin.cell.offset(offset)).ok()?;
            Some(Location::new(origin.grid, cell))
        })
    }

    /// Sets the buffer up for a question, with nothing seen.
    fn prepare(&mut self, asked: Asked, topology: Topology) {
        // The radius as whole cells, and the same in storeys.
        let radius_cells = (asked.radius_m / CELL_M).clamp(Real::ZERO, Real::from(FOV_MAX_REACH));
        self.radius_sq = (radius_cells * radius_cells).floor().to_i64();
        let mut reach = isqrt(self.radius_sq) as i32;
        // Round a wrapped axis no cell may be seen twice.
        for extent in [topology.x, topology.y] {
            if let Extent::Wrapped(size) = extent {
                reach = reach.min((size.max(1) - 1) / 2);
            }
        }
        self.asked = Some(asked);
        self.topology = topology;
        self.reach = reach;
        self.storeys = isqrt(self.radius_sq / (STOREY_CELLS * STOREY_CELLS)) as i32;
        self.row_words = self.side().div_ceil(WORD_BITS);
        let layers = 2 * self.storeys as usize + 1;
        self.bits.resize(layers * self.layer_words(), 0);
        self.used.clear();
        self.used.resize(layers, false);
        self.count = 0;
    }

    /// Whether an offset is within the radius.
    fn in_radius(&self, dx: i64, dy: i64, dz: i64) -> bool {
        let up = dz * STOREY_CELLS;
        dx * dx + dy * dy + up * up <= self.radius_sq
    }

    /// Marks the cell at an offset as seen.
    fn mark(&mut self, offset: IVec3) {
        let Some((layer, word, bit)) = self.slot(offset) else {
            return;
        };
        if !self.used[layer] {
            let words = self.layer_words();
            self.bits[layer * words..(layer + 1) * words].fill(0);
            self.used[layer] = true;
        }
        if self.bits[word] & bit == 0 {
            self.bits[word] |= bit;
            self.count += 1;
        }
    }

    /// Marks a run of one row of one storey as seen: the offsets from
    /// `dx_low` to `dx_high` at `dy`, `dz`. The run must lie inside the
    /// buffer.
    fn mark_run(&mut self, dx_low: i32, dx_high: i32, dy: i32, dz: i32) {
        let (Some((layer, first, _)), Some((_, last, _))) = (
            self.slot(IVec3::new(dx_low, dy, dz)),
            self.slot(IVec3::new(dx_high, dy, dz)),
        ) else {
            return;
        };
        if !self.used[layer] {
            let words = self.layer_words();
            self.bits[layer * words..(layer + 1) * words].fill(0);
            self.used[layer] = true;
        }
        let low_bit = (dx_low + self.reach) as usize % WORD_BITS;
        let high_bit = (dx_high + self.reach) as usize % WORD_BITS;
        for word in first..=last {
            let mut run = u64::MAX;
            if word == first {
                run &= u64::MAX << low_bit;
            }
            if word == last {
                run &= u64::MAX >> (WORD_BITS - 1 - high_bit);
            }
            self.count += (run & !self.bits[word]).count_ones() as usize;
            self.bits[word] |= run;
        }
    }

    /// The highest storey that is not open in any cell column within
    /// reach of the viewer, or `None` if some cell within reach cannot
    /// be read. Above it there is nothing but air.
    fn highest_within(&self, cells: &mut CellReader<'_>, origin: CellPos) -> Option<i32> {
        let mut top = i32::MIN;
        for dy in -self.reach..=self.reach {
            for dx in -self.reach..=self.reach {
                let (column, cell) = cells.locate(origin.offset(IVec3::new(dx, dy, 0)))?;
                top = top.max(column.height_at(cell));
            }
        }
        Some(top)
    }

    /// Marks everything within the radius on a storey that holds nothing
    /// at all: what casting would see there, without reading a cell.
    fn mark_disc(&mut self, dz: i32) {
        let up = i64::from(dz) * STOREY_CELLS;
        let left = self.radius_sq - up * up;
        if left < 0 {
            return;
        }
        let limit = isqrt(left).min(i64::from(self.reach));
        for dy in -limit..=limit {
            let half = isqrt(left - dy * dy).min(i64::from(self.reach)) as i32;
            self.mark_run(-half, half, dy as i32, dz);
        }
    }

    /// Symmetric shadowcasting on one storey, from the cell of that
    /// storey that is over or under the viewer's.
    fn cast_storey(
        &mut self,
        cells: &mut CellReader<'_>,
        defs: &WorldDefs,
        centre: CellPos,
        dz: i32,
        eye_m: Real,
    ) {
        let up = i64::from(dz) * STOREY_CELLS;
        let depth_limit = isqrt(self.radius_sq - up * up).min(i64::from(self.reach));
        for quadrant in 0..4 {
            self.stack.clear();
            self.stack.push(Row {
                depth: 1,
                start: Slope { num: -1, den: 1 },
                end: Slope { num: 1, den: 1 },
            });
            while let Some(mut row) = self.stack.pop() {
                while row.depth <= depth_limit {
                    let depth = row.depth;
                    let mut prev_wall: Option<bool> = None;
                    for col in row.start.first_col(depth)..=row.end.last_col(depth) {
                        let (dx, dy) = match quadrant {
                            0 => (col, -depth),
                            1 => (depth, col),
                            2 => (col, depth),
                            _ => (-depth, col),
                        };
                        let offset = IVec3::new(dx as i32, dy as i32, dz);
                        let blocks =
                            cells.blocks_sight(defs, centre.offset(offset.xy().with_z(0)), eye_m);
                        // What cannot be read hides what is behind it
                        // and is not itself seen.
                        let wall = blocks != Some(false);
                        let centre_in_view = col * row.start.den >= depth * row.start.num
                            && col * row.end.den <= depth * row.end.num;
                        if blocks.is_some()
                            && (wall || centre_in_view)
                            && self.in_radius(dx, dy, i64::from(dz))
                        {
                            self.mark(offset);
                        }
                        if prev_wall == Some(true) && !wall {
                            row.start = Slope::of(col, depth);
                        }
                        if prev_wall == Some(false) && wall {
                            self.stack.push(Row {
                                depth: depth + 1,
                                start: row.start,
                                end: Slope::of(col, depth),
                            });
                        }
                        prev_wall = Some(wall);
                    }
                    if prev_wall != Some(false) {
                        break;
                    }
                    row.depth += 1;
                }
            }
        }
    }

    /// Works the answer out.
    fn compute(&mut self, view: &GridView<'_>, defs: &WorldDefs, asked: Asked) {
        self.prepare(asked, *view.topology());
        let origin = asked.origin.cell;
        let eye_m = Real::from(asked.eye_alt_mm) * STOREY_M / Real::from(STOREY_MM);
        let mut cells = CellReader::new(*view);

        // The viewer's own storey.
        self.mark(IVec3::ZERO);
        self.cast_storey(&mut cells, defs, origin, 0, eye_m);

        // Down: under every seen cell of that storey that nothing
        // closes, the cells below, as far as the first that has a floor
        // or holds something opaque. That one is seen and ends it. The
        // viewer looks past whatever shares its own cell: only a floor
        // closes that one.
        let closed = CellFlags::HAS_FLOOR | CellFlags::OPAQUE;
        let mut lowest = 0;
        let level = self.storeys as usize * self.layer_words();
        for row in 0..self.side() {
            for part in 0..self.row_words {
                let mut word = self.bits[level + row * self.row_words + part];
                while word != 0 {
                    let col = part * WORD_BITS + word.trailing_zeros() as usize;
                    word &= word - 1;
                    let dx = col as i32 - self.reach;
                    let dy = row as i32 - self.reach;
                    let Some(mut flags) = cells.flags(origin.offset(IVec3::new(dx, dy, 0))) else {
                        continue;
                    };
                    if (dx, dy) == (0, 0) {
                        flags = flags & CellFlags::HAS_FLOOR;
                    }
                    let mut down = 0;
                    while !flags.intersects(closed) {
                        down += 1;
                        if !self.in_radius(i64::from(dx), i64::from(dy), i64::from(down)) {
                            break;
                        }
                        lowest = lowest.max(down);
                        let offset = IVec3::new(dx, dy, -down);
                        let Some(below) = cells.flags(origin.offset(offset)) else {
                            break;
                        };
                        self.mark(offset);
                        flags = below;
                    }
                }
            }
        }

        // Up: while nothing closes the cells over the viewer's head,
        // each storey above as seen from the cell over the viewer. This
        // mirrors looking down, so that sight between storeys goes both
        // ways or neither.
        let mut highest = 0;
        // The storey above which there is only air within reach, looked
        // for when first needed. A storey of nothing but air is seen
        // whole, which is what casting would find, at no cost.
        let mut air_above: Option<Option<i32>> = None;
        for up in 1..=self.storeys {
            highest = up;
            let offset = IVec3::new(0, 0, up);
            let Some(flags) = cells.flags(origin.offset(offset)) else {
                break;
            };
            if flags.contains(CellFlags::HAS_FLOOR) {
                break;
            }
            self.mark(offset);
            if flags.contains(CellFlags::OPAQUE) {
                break;
            }
            let top = match air_above {
                Some(top) => top,
                None => *air_above.insert(self.highest_within(&mut cells, origin)),
            };
            let storey = origin.z.saturating_add(up);
            if top.is_some_and(|top| storey > top) {
                self.mark_disc(up);
            } else {
                self.cast_storey(&mut cells, defs, origin.offset(offset), up, eye_m);
            }
        }

        // Every cell read lies in this box.
        self.stamp.clear();
        self.stamp.record_box(
            view,
            CellBox::new(
                origin.offset(IVec3::new(-self.reach, -self.reach, -lowest)),
                origin.offset(IVec3::new(self.reach, self.reach, highest)),
            ),
        );
    }
}

/// The seen cells of a [`FovSet`] as offsets from the viewer's cell,
/// storey by storey, row by row.
#[derive(Clone, Debug)]
pub struct FovOffsets<'a> {
    set: &'a FovSet,
    /// The word after the one `pending` came from.
    word: usize,
    /// The word the walk stops at.
    end: usize,
    /// Bits of the current word not yet given out.
    pending: u64,
    started: bool,
}

impl Iterator for FovOffsets<'_> {
    type Item = IVec3;

    fn next(&mut self) -> Option<IVec3> {
        let set = self.set;
        let layer_words = set.layer_words();
        while self.pending == 0 {
            if self.started {
                self.word += 1;
            }
            self.started = true;
            if self.word >= self.end {
                return None;
            }
            if !set.used[self.word / layer_words] {
                // Nothing on this storey: on to the last word of it.
                self.word = (self.word / layer_words + 1) * layer_words - 1;
                continue;
            }
            self.pending = set.bits[self.word];
        }
        let bit = self.pending.trailing_zeros() as usize;
        self.pending &= self.pending - 1;
        let layer = self.word / layer_words;
        let in_layer = self.word % layer_words;
        let row = in_layer / set.row_words;
        let col = (in_layer % set.row_words) * WORD_BITS + bit;
        Some(IVec3::new(
            col as i32 - set.reach,
            row as i32 - set.reach,
            layer as i32 - set.storeys,
        ))
    }
}

impl Cosmos {
    /// The cells seen from a cell, by an eye `eye_alt_mm` millimetres
    /// above its base, out to `radius_m` metres (02 §22). The answer
    /// goes into `out`, replacing what it held.
    ///
    /// **On the viewer's storey**, symmetric shadowcasting. A cell blocks
    /// sight when something `opaque` in it stands taller than the eye. A
    /// cell that blocks is seen if any of it is in view: one sees the
    /// wall. A cell that does not block is seen if the line from the
    /// middle of the viewer's cell to its own middle is clear. So for two
    /// cells that do not block, and eyes at one height, each sees the
    /// other or neither does.
    ///
    /// **Down.** Under every seen cell of the viewer's storey that has no
    /// floor and holds nothing opaque, the cell below is seen; and the
    /// one below that, if this one too has no floor and nothing opaque;
    /// and so on. The last is what the eye rests on: ground, a floor, a
    /// roof. Under its own cell the viewer looks past whatever shares
    /// that cell: only a floor closes it.
    ///
    /// **Up.** While the cell over the viewer's head has no floor, the
    /// storey above is seen as from that cell, and the storey above that
    /// in the same way, as long as the cells passed through hold nothing
    /// opaque. This is the mirror of looking down: whoever looks down
    /// through an opening on someone is seen by them looking up.
    ///
    /// Distance is between cell centres with a storey counting three
    /// metres, and a cell at exactly the radius is seen. Cells that are
    /// not loaded, or beyond an edge of the realm, are not seen and hide
    /// what is behind them. The radius is cut to [`FOV_MAX_REACH`] cells,
    /// and on a wrapped axis to half way round.
    ///
    /// If `out` already holds the answer to the same question and no
    /// chunk it was read from has been edited since, nothing is worked
    /// out and [`Freshness::Reused`] says so. An error if the viewer's
    /// cell is not loaded or does not exist; `out` is then empty.
    ///
    /// The sets of two viewers agree with
    /// [`line_of_sight`](Cosmos::line_of_sight) except close to corners,
    /// where a view counts a cell by its middle and a line by every cell
    /// it passes through.
    pub fn field_of_view(
        &self,
        origin: Location,
        eye_alt_mm: u16,
        radius_m: Real,
        out: &mut FovSet,
    ) -> Result<Freshness, WorldError> {
        let located = self
            .grid(origin.grid)
            .and_then(|view| Ok((view, super::locate(&view, origin.cell)?.1)));
        let (view, cell) = match located {
            Ok(found) => found,
            Err(error) => {
                out.clear();
                return Err(error);
            }
        };
        let asked = Asked {
            origin: Location::new(origin.grid, cell),
            eye_alt_mm: eye_alt_mm.min(STOREY_MM - 1),
            radius_m,
        };
        if out.asked == Some(asked) && out.stamp.is_current(self) {
            return Ok(Freshness::Reused);
        }
        out.compute(&view, self.defs(), asked);
        Ok(Freshness::Computed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn whole_square_roots_are_exact() {
        for n in 0..5000_i64 {
            let root = isqrt(n);
            assert!(root * root <= n && (root + 1) * (root + 1) > n, "{n}");
        }
        assert_eq!(isqrt(-4), 0);
        assert_eq!(isqrt(65_025), 255);
    }

    #[test]
    fn a_row_runs_between_its_slopes_rounded_to_cells() {
        // The whole quadrant: from −depth to depth.
        let (low, high) = (Slope { num: -1, den: 1 }, Slope { num: 1, den: 1 });
        for depth in 1..40 {
            assert_eq!(low.first_col(depth), -depth);
            assert_eq!(high.last_col(depth), depth);
        }
        // Against the same in fractions of a cell: halves round up at
        // the start of a row and down at its end.
        for den in 1..12_i64 {
            for num in -den..=den {
                let slope = Slope { num, den };
                for depth in 1..12_i64 {
                    // Twice the exact value, to stay in whole numbers.
                    let twice = 2 * depth * num;
                    let first = slope.first_col(depth);
                    assert!(2 * den * first - den <= twice && twice < 2 * den * first + den);
                    let last = slope.last_col(depth);
                    assert!(2 * den * last - den < twice && twice <= 2 * den * last + den);
                }
            }
        }
        // The edge of a cell's diamond nearer the axis.
        let edge = Slope::of(3, 4);
        assert_eq!((edge.num, edge.den), (5, 8));
    }

    #[test]
    fn an_empty_view_holds_nothing() {
        let view = FovSet::default();
        assert!(view.is_empty() && view.offsets().next().is_none());
        assert_eq!(view.storeys(), None);
        assert!(!view.contains_offset(IVec3::ZERO));
        assert_eq!((view.eye_alt_mm(), view.radius_m()), (0, Real::ZERO));
        assert!(view.stamp().is_empty());
    }
}
