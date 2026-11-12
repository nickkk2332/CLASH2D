//! [`ViewModel`]: the scene's picture of the world (12 §5).
//!
//! The view model is plain data the scene owns. It holds, for each cell
//! that has been put into it, what was seen there, whether it is seen now
//! or only remembered, and how well lit it was; for each perceived
//! character or object, where it is and how it looks; and markers for
//! what was only heard.
//!
//! It is not a copy of the world. It holds what it has been told through
//! the calls below and nothing else, and it has no way to read the world:
//! this module never names the world's read API. Whoever fills it decides
//! what the player may know: the perception layer and playback for an
//! ordinary game, and [`crate::view::omniscient`] for debugging.
//!
//! # Filling it
//!
//! | Call | Means |
//! |------|-------|
//! | [`ViewModel::observe_cell`] | This cell is seen now, and this is in it |
//! | [`ViewModel::observe_light`] | The light on a known cell changed |
//! | [`ViewModel::unsee_cell`], [`unsee_box`](ViewModel::unsee_box), [`unsee_all`](ViewModel::unsee_all) | No longer in view: what was there is kept as remembered |
//! | [`ViewModel::forget_cell`], [`forget_grid`](ViewModel::forget_grid), [`clear`](ViewModel::clear) | Map memory lost |
//! | [`ViewModel::upsert_sprite`], [`move_sprite`](ViewModel::move_sprite), [`remove_sprite`](ViewModel::remove_sprite) | A perceived character or object appeared, changed, moved or is gone |
//! | [`ViewModel::add_heard`], [`remove_heard`](ViewModel::remove_heard), [`clear_heard`](ViewModel::clear_heard) | Something was only heard |
//! | [`ViewModel::set_grid_shape`] | How a grid wraps and where it ends, so a view can draw across a wrapped edge |
//!
//! Every call touches one cell or one sprite: playing the destruction of
//! a wall is one `observe_cell`.
//!
//! # Storage
//!
//! Cells are kept in slabs: one storey of one tract, allocated when its
//! first cell becomes known and dropped when its last is forgotten. Two
//! view models that hold the same knowledge are equal, whatever order
//! they learnt it in, which is what the check of 12 §5 (caught-up
//! playback equals a fresh initialisation) compares.

use std::collections::{BTreeMap, BTreeSet};

use bitflags::bitflags;
use clash_core::define_id;
use clash_world::dims::CHUNK_AREA;
use clash_world::{
    BlockState, CellBox, CellPos, DecalId, FieldId, FixtureState, FloorState, GridId, Location,
    Topology, TractPos,
};

use crate::color::Color;
use crate::glyph::{Facing, GlyphPart};

/// The light level of a fully lit cell. Levels run from 0, no light at
/// all, to this.
pub const LIGHT_FULL: u8 = u8::MAX;

/// How many cells one slab holds: one storey of one tract.
const SLAB_CELLS: usize = CHUNK_AREA;

define_id! {
    /// What the view model knows a perceived character or object by. The
    /// number is chosen by whoever feeds the view model (a persona, an
    /// item) and means nothing here.
    pub struct SpriteId(u64);
    /// A marker for something only heard.
    pub struct HeardId(u64);
}

bitflags! {
    /// What the view model records about a cell beside its contents.
    #[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Default)]
    struct KnownFlags: u8 {
        /// The cell has been seen at some time.
        const KNOWN = 1;
        /// The cell is seen now.
        const SEEN = 2;
        /// Several items lie here: the pile marker is drawn.
        const PILE = 4;
        /// The cell is known to be open to the sky.
        const SKY = 8;
    }
}

/// The strongest field seen in a cell.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct FieldSeen {
    /// Which field.
    pub field: FieldId,
    /// Its level, 1 to 255.
    pub level: u8,
}

/// The most marked decal seen in a cell.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct DecalSeen {
    /// Which kind of mark.
    pub kind: DecalId,
    /// How much of it, 1 to 255.
    pub amount: u8,
}

/// What an observer reports about one cell: what [`ViewModel::observe_cell`]
/// is given.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct CellContents {
    /// The floor.
    pub floor: FloorState,
    /// The block.
    pub block: BlockState,
    /// The fixture.
    pub fixture: FixtureState,
    /// The strongest field, if any is there.
    pub field: Option<FieldSeen>,
    /// The most marked decal, if any is there.
    pub decal: Option<DecalSeen>,
    /// Whether several items lie here, so that the pile marker is drawn.
    /// A single item is a sprite of kind [`SpriteKind::Object`].
    pub pile: bool,
    /// Whether the cell is known to be open to the sky.
    pub open_sky: bool,
    /// How well lit the cell is, 0 to [`LIGHT_FULL`].
    pub light: u8,
}

impl CellContents {
    /// A fully lit cell that holds nothing.
    pub const EMPTY: CellContents = CellContents {
        floor: FloorState::NONE,
        block: BlockState::NONE,
        fixture: FixtureState::NONE,
        field: None,
        decal: None,
        pile: false,
        open_sky: false,
        light: LIGHT_FULL,
    };

    /// A fully lit cell holding these three elements and nothing else.
    pub const fn elements(floor: FloorState, block: BlockState, fixture: FixtureState) -> Self {
        CellContents {
            floor,
            block,
            fixture,
            ..CellContents::EMPTY
        }
    }
}

impl Default for CellContents {
    fn default() -> CellContents {
        CellContents::EMPTY
    }
}

/// What the view model holds for one cell that has been seen.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Default)]
pub struct KnownCell {
    /// The floor that was seen.
    pub floor: FloorState,
    /// The block that was seen.
    pub block: BlockState,
    /// The fixture that was seen.
    pub fixture: FixtureState,
    /// The strongest field that was seen.
    pub field: Option<FieldSeen>,
    /// The most marked decal that was seen.
    pub decal: Option<DecalSeen>,
    /// How well lit it was, 0 to [`LIGHT_FULL`].
    pub light: u8,
    flags: KnownFlags,
}

impl KnownCell {
    /// Whether the cell is seen now. Otherwise it is remembered.
    #[inline]
    pub const fn seen(&self) -> bool {
        self.flags.contains(KnownFlags::SEEN)
    }

    /// Whether several items lie here.
    #[inline]
    pub const fn pile(&self) -> bool {
        self.flags.contains(KnownFlags::PILE)
    }

    /// Whether the cell is known to be open to the sky.
    #[inline]
    pub const fn open_sky(&self) -> bool {
        self.flags.contains(KnownFlags::SKY)
    }

    #[inline]
    const fn known(&self) -> bool {
        self.flags.contains(KnownFlags::KNOWN)
    }

    /// What was reported about the cell.
    pub const fn contents(&self) -> CellContents {
        CellContents {
            floor: self.floor,
            block: self.block,
            fixture: self.fixture,
            field: self.field,
            decal: self.decal,
            pile: self.pile(),
            open_sky: self.open_sky(),
            light: self.light,
        }
    }
}

/// What kind of thing a sprite is: where it stands in the order of
/// 12 §6.1.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub enum SpriteKind {
    /// An item lying in the cell, or any object that is not a creature.
    Object,
    /// A creature. Drawn over everything else in its cell.
    Creature,
}

/// A perceived character or object: where it is and how it looks.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct EntitySprite {
    /// Where it was perceived.
    pub loc: Location,
    /// A creature, or an object.
    pub kind: SpriteKind,
    /// Its glyph for the state it is in: the result of
    /// [`Glyph::part`](crate::Glyph::part).
    pub glyph: GlyphPart,
    /// The colour this particular thing carries, for `"own"`.
    pub own: Option<Color>,
    /// The colour of what it is seen to be wearing, for `"dress"`.
    pub dress: Option<Color>,
    /// Which way it faces, if that is known.
    pub facing: Option<Facing>,
    /// A presence that is suspected and not detected: drawn as the
    /// theme's `suspected` mark instead of its glyph.
    pub suspected: bool,
}

/// A marker for something that was only heard (12 §5).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct HeardMarker {
    /// Where it seemed to come from.
    pub loc: Location,
    /// The character to mark it with. `None` uses the theme's `heard`
    /// mark.
    pub ch: Option<char>,
}

/// One storey of one tract.
#[derive(Clone, PartialEq, Eq, Debug)]
struct Slab {
    cells: Box<[KnownCell; SLAB_CELLS]>,
    /// How many of the cells are known.
    known: u32,
}

impl Slab {
    fn new() -> Slab {
        Slab {
            cells: Box::new([KnownCell::default(); SLAB_CELLS]),
            known: 0,
        }
    }
}

/// Which slab a cell is in: its grid, its storey and its tract.
type SlabKey = (GridId, i32, TractPos);

#[inline]
const fn slab_key(grid: GridId, cell: CellPos) -> SlabKey {
    (grid, cell.z, cell.tract())
}

/// A cell's place within its slab.
#[inline]
const fn slab_index(cell: CellPos) -> usize {
    cell.column()
}

/// The scene's picture of the world. See the [module](self).
#[derive(Clone, Debug, Default)]
pub struct ViewModel {
    slabs: BTreeMap<SlabKey, Slab>,
    sprites: BTreeMap<SpriteId, EntitySprite>,
    /// The sprites by where they are, for drawing.
    sprites_at: BTreeSet<(GridId, CellPos, SpriteId)>,
    heard: BTreeMap<HeardId, HeardMarker>,
    next_heard: u64,
    shapes: BTreeMap<GridId, Topology>,
    /// Bumped by every change, so that whatever caches a drawing of the
    /// view model knows when it is stale. Not part of what is known.
    revision: u64,
}

/// Two view models are equal when they hold the same knowledge: the same
/// cells, sprites, markers and grid shapes. How they came by it, and how
/// often they changed, does not count.
impl PartialEq for ViewModel {
    fn eq(&self, other: &ViewModel) -> bool {
        self.slabs == other.slabs
            && self.sprites == other.sprites
            && self.heard.values().eq(other.heard.values())
            && self.shapes == other.shapes
    }
}

impl ViewModel {
    /// A view model that knows nothing.
    pub fn new() -> ViewModel {
        ViewModel::default()
    }

    /// A number that changes whenever the view model does.
    pub fn revision(&self) -> u64 {
        self.revision
    }

    #[inline]
    fn touch(&mut self) {
        self.revision = self.revision.wrapping_add(1);
    }

    // -----------------------------------------------------------------
    // Cells
    // -----------------------------------------------------------------

    /// Records that a cell is seen now and what is in it, replacing
    /// whatever was known of it.
    pub fn observe_cell(&mut self, loc: Location, contents: CellContents) {
        let slab = self
            .slabs
            .entry(slab_key(loc.grid, loc.cell))
            .or_insert_with(Slab::new);
        let cell = &mut slab.cells[slab_index(loc.cell)];
        if !cell.known() {
            slab.known += 1;
        }
        let mut flags = KnownFlags::KNOWN | KnownFlags::SEEN;
        flags.set(KnownFlags::PILE, contents.pile);
        flags.set(KnownFlags::SKY, contents.open_sky);
        *cell = KnownCell {
            floor: contents.floor,
            block: contents.block,
            fixture: contents.fixture,
            field: contents.field.filter(|field| field.level > 0),
            decal: contents.decal.filter(|decal| decal.amount > 0),
            light: contents.light,
            flags,
        };
        self.touch();
    }

    /// Changes the light level of a cell that is already known. Returns
    /// whether the cell was known: light on a cell never seen is not
    /// recorded.
    pub fn observe_light(&mut self, loc: Location, light: u8) -> bool {
        let Some(cell) = self.cell_mut(loc) else {
            return false;
        };
        cell.light = light;
        self.touch();
        true
    }

    /// Records that a cell is no longer in view. What was seen there is
    /// kept as remembered. Returns whether the cell was seen until now.
    pub fn unsee_cell(&mut self, loc: Location) -> bool {
        let Some(cell) = self.cell_mut(loc) else {
            return false;
        };
        if !cell.seen() {
            return false;
        }
        cell.flags.remove(KnownFlags::SEEN);
        self.touch();
        true
    }

    /// Records that no cell of a box is in view any longer.
    pub fn unsee_box(&mut self, grid: GridId, cells: CellBox) {
        let lo = (grid, cells.min.z, TractPos::new(i32::MIN, i32::MIN));
        let hi = (grid, cells.max.z, TractPos::new(i32::MAX, i32::MAX));
        for ((_, z, tract), slab) in self.slabs.range_mut(lo..=hi) {
            for (index, cell) in slab.cells.iter_mut().enumerate() {
                if cell.seen() && cells.contains(tract.cell(index, *z)) {
                    cell.flags.remove(KnownFlags::SEEN);
                }
            }
        }
        self.touch();
    }

    /// Records that nothing is in view any longer: every known cell
    /// becomes remembered. The perception layer calls this before it
    /// reports what is seen from a new place.
    pub fn unsee_all(&mut self) {
        for slab in self.slabs.values_mut() {
            for cell in slab.cells.iter_mut() {
                cell.flags.remove(KnownFlags::SEEN);
            }
        }
        self.touch();
    }

    /// Forgets a cell altogether. Returns whether it was known.
    pub fn forget_cell(&mut self, loc: Location) -> bool {
        let key = slab_key(loc.grid, loc.cell);
        let Some(slab) = self.slabs.get_mut(&key) else {
            return false;
        };
        let cell = &mut slab.cells[slab_index(loc.cell)];
        if !cell.known() {
            return false;
        }
        *cell = KnownCell::default();
        slab.known -= 1;
        if slab.known == 0 {
            self.slabs.remove(&key);
        }
        self.touch();
        true
    }

    /// Forgets every cell, sprite and marker of one grid.
    pub fn forget_grid(&mut self, grid: GridId) {
        self.slabs.retain(|(of, _, _), _| *of != grid);
        self.sprites.retain(|_, sprite| sprite.loc.grid != grid);
        self.sprites_at.retain(|(of, _, _)| *of != grid);
        self.heard.retain(|_, marker| marker.loc.grid != grid);
        self.touch();
    }

    /// Forgets everything but the shapes of the grids.
    pub fn clear(&mut self) {
        self.slabs.clear();
        self.sprites.clear();
        self.sprites_at.clear();
        self.heard.clear();
        self.touch();
    }

    fn cell_mut(&mut self, loc: Location) -> Option<&mut KnownCell> {
        let slab = self.slabs.get_mut(&slab_key(loc.grid, loc.cell))?;
        let cell = &mut slab.cells[slab_index(loc.cell)];
        cell.known().then_some(cell)
    }

    /// What is known of a cell, if it has ever been seen.
    #[inline]
    pub fn cell(&self, loc: Location) -> Option<&KnownCell> {
        self.cell_in(loc.grid, loc.cell)
    }

    /// What is known of a cell of a grid, if it has ever been seen.
    #[inline]
    pub fn cell_in(&self, grid: GridId, cell: CellPos) -> Option<&KnownCell> {
        let slab = self.slabs.get(&slab_key(grid, cell))?;
        let known = &slab.cells[slab_index(cell)];
        known.known().then_some(known)
    }

    /// How many cells are known.
    pub fn known_cells(&self) -> usize {
        self.slabs.values().map(|slab| slab.known as usize).sum()
    }

    // -----------------------------------------------------------------
    // Sprites
    // -----------------------------------------------------------------

    /// Adds a perceived character or object, or replaces what was known
    /// of it. Returns what was known before.
    pub fn upsert_sprite(&mut self, id: SpriteId, sprite: EntitySprite) -> Option<EntitySprite> {
        let before = self.sprites.insert(id, sprite);
        if let Some(before) = before {
            self.sprites_at
                .remove(&(before.loc.grid, before.loc.cell, id));
        }
        self.sprites_at
            .insert((sprite.loc.grid, sprite.loc.cell, id));
        self.touch();
        before
    }

    /// Moves a sprite. Returns whether there is such a sprite.
    pub fn move_sprite(&mut self, id: SpriteId, to: Location) -> bool {
        let Some(sprite) = self.sprites.get_mut(&id) else {
            return false;
        };
        let from = sprite.loc;
        sprite.loc = to;
        self.sprites_at.remove(&(from.grid, from.cell, id));
        self.sprites_at.insert((to.grid, to.cell, id));
        self.touch();
        true
    }

    /// Removes a sprite: the thing is gone, or no longer perceived and
    /// not to be remembered. Returns what was known of it.
    pub fn remove_sprite(&mut self, id: SpriteId) -> Option<EntitySprite> {
        let sprite = self.sprites.remove(&id)?;
        self.sprites_at
            .remove(&(sprite.loc.grid, sprite.loc.cell, id));
        self.touch();
        Some(sprite)
    }

    /// What is known of a perceived character or object.
    pub fn sprite(&self, id: SpriteId) -> Option<&EntitySprite> {
        self.sprites.get(&id)
    }

    /// Every sprite, in id order.
    pub fn sprites(&self) -> impl Iterator<Item = (SpriteId, &EntitySprite)> + '_ {
        self.sprites.iter().map(|(id, sprite)| (*id, sprite))
    }

    /// The sprite drawn in a cell: the creature with the lowest id, or
    /// else the object with the lowest id.
    pub fn top_sprite_at(&self, grid: GridId, cell: CellPos) -> Option<&EntitySprite> {
        if self.sprites_at.is_empty() {
            return None;
        }
        let lo = (grid, cell, SpriteId::new(u64::MIN));
        let hi = (grid, cell, SpriteId::new(u64::MAX));
        let mut object = None;
        for (_, _, id) in self.sprites_at.range(lo..=hi) {
            let Some(sprite) = self.sprites.get(id) else {
                continue;
            };
            match sprite.kind {
                SpriteKind::Creature => return Some(sprite),
                SpriteKind::Object => object = object.or(Some(sprite)),
            }
        }
        object
    }

    /// The sprite drawn in a cell when the sprites in `hidden` are not
    /// drawn where they are: what [`ViewModel::top_sprite_at`] gives of
    /// the rest. Playback hides a sprite while it glides between cells.
    pub fn top_sprite_at_except(
        &self,
        grid: GridId,
        cell: CellPos,
        hidden: &[SpriteId],
    ) -> Option<&EntitySprite> {
        if self.sprites_at.is_empty() {
            return None;
        }
        let lo = (grid, cell, SpriteId::new(u64::MIN));
        let hi = (grid, cell, SpriteId::new(u64::MAX));
        let mut object = None;
        for (_, _, id) in self.sprites_at.range(lo..=hi) {
            if hidden.contains(id) {
                continue;
            }
            let Some(sprite) = self.sprites.get(id) else {
                continue;
            };
            match sprite.kind {
                SpriteKind::Creature => return Some(sprite),
                SpriteKind::Object => object = object.or(Some(sprite)),
            }
        }
        object
    }

    /// The number of the sprite drawn in a cell: the one
    /// [`ViewModel::top_sprite_at`] gives.
    pub fn top_sprite_id_at(&self, grid: GridId, cell: CellPos) -> Option<SpriteId> {
        let lo = (grid, cell, SpriteId::new(u64::MIN));
        let hi = (grid, cell, SpriteId::new(u64::MAX));
        let mut object = None;
        for (_, _, id) in self.sprites_at.range(lo..=hi) {
            match self.sprites.get(id).map(|sprite| sprite.kind) {
                Some(SpriteKind::Creature) => return Some(*id),
                Some(SpriteKind::Object) => object = object.or(Some(*id)),
                None => {}
            }
        }
        object
    }

    // -----------------------------------------------------------------
    // Heard
    // -----------------------------------------------------------------

    /// Adds a marker for something only heard and returns its id, with
    /// which playback removes it when it has been shown long enough.
    pub fn add_heard(&mut self, marker: HeardMarker) -> HeardId {
        let id = HeardId::new(self.next_heard);
        self.next_heard = self.next_heard.wrapping_add(1);
        self.heard.insert(id, marker);
        self.touch();
        id
    }

    /// Removes a heard marker. Returns whether it was there.
    pub fn remove_heard(&mut self, id: HeardId) -> bool {
        let removed = self.heard.remove(&id).is_some();
        if removed {
            self.touch();
        }
        removed
    }

    /// Removes every heard marker.
    pub fn clear_heard(&mut self) {
        self.heard.clear();
        self.touch();
    }

    /// The heard markers, oldest first.
    pub fn heard(&self) -> impl Iterator<Item = &HeardMarker> + '_ {
        self.heard.values()
    }

    // -----------------------------------------------------------------
    // Grid shapes
    // -----------------------------------------------------------------

    /// Tells the view model the shape of a grid: where it ends and
    /// whether it wraps. A view uses it to draw across a wrapped edge. A
    /// grid whose shape was never given is taken to have no edges.
    pub fn set_grid_shape(&mut self, grid: GridId, shape: Topology) {
        self.shapes.insert(grid, shape);
        self.touch();
    }

    /// The shape of a grid, if it was given.
    pub fn grid_shape(&self, grid: GridId) -> Option<&Topology> {
        self.shapes.get(&grid)
    }

    /// A position brought inside its grid: wrapped on wrapped axes, or
    /// `None` beyond an edge. With no shape given, the position as it is.
    #[inline]
    pub fn normalize(&self, grid: GridId, cell: CellPos) -> Option<CellPos> {
        match self.shapes.get(&grid) {
            Some(shape) => shape.normalize(cell).ok(),
            None => Some(cell),
        }
    }
}

/// One way in which two view models differ: what
/// [`ViewModel::differences`] lists.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ViewDifference {
    /// A cell is known differently, or known to one and not the other.
    Cell {
        /// The cell.
        loc: Location,
        /// What this view model holds for it.
        here: Option<KnownCell>,
        /// What the other holds.
        there: Option<KnownCell>,
    },
    /// A sprite differs, or one has it and the other does not.
    Sprite {
        /// The sprite's number.
        id: SpriteId,
        /// What this view model holds.
        here: Option<EntitySprite>,
        /// What the other holds.
        there: Option<EntitySprite>,
    },
    /// The shape of a grid differs, or one was told it and the other
    /// was not.
    Shape {
        /// The grid.
        grid: GridId,
    },
}

impl core::fmt::Display for ViewDifference {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        let cell = |cell: &Option<KnownCell>| match cell {
            None => "unknown".to_owned(),
            Some(cell) => format!(
                "{} floor {} block {} fixture {} light {}{}{}",
                if cell.seen() { "seen" } else { "remembered" },
                cell.floor.raw().def,
                cell.block.raw().def,
                cell.fixture.raw().def,
                cell.light,
                if cell.field.is_some() { " field" } else { "" },
                if cell.pile() { " pile" } else { "" },
            ),
        };
        let sprite = |sprite: &Option<EntitySprite>| match sprite {
            None => "absent".to_owned(),
            Some(sprite) => format!("at {}", sprite.loc.cell),
        };
        match self {
            ViewDifference::Cell { loc, here, there } => write!(
                f,
                "cell {} of grid {}: played {}; fresh {}",
                loc.cell,
                loc.grid.raw(),
                cell(here),
                cell(there)
            ),
            ViewDifference::Sprite { id, here, there } => write!(
                f,
                "sprite {}: played {}; fresh {}",
                id.raw(),
                sprite(here),
                sprite(there)
            ),
            ViewDifference::Shape { grid } => {
                write!(f, "the shape of grid {} differs", grid.raw())
            }
        }
    }
}

impl ViewModel {
    /// How this view model's knowledge differs from another's: cells,
    /// then sprites, then grid shapes, at most `most` of them. Markers
    /// of what was only heard are not compared: they are shown for a
    /// while and are not knowledge of the world. An empty list means
    /// the two hold the same cells, sprites and shapes.
    pub fn differences(&self, other: &ViewModel, most: usize) -> Vec<ViewDifference> {
        let mut found = Vec::new();
        let keys: BTreeSet<&SlabKey> = self.slabs.keys().chain(other.slabs.keys()).collect();
        for key in keys {
            let (grid, z, tract) = *key;
            let (mine, theirs) = (self.slabs.get(key), other.slabs.get(key));
            if mine == theirs {
                continue;
            }
            for index in 0..SLAB_CELLS {
                let known = |slab: Option<&Slab>| {
                    slab.map(|slab| slab.cells[index]).filter(KnownCell::known)
                };
                let (here, there) = (known(mine), known(theirs));
                if here != there {
                    if found.len() >= most {
                        return found;
                    }
                    found.push(ViewDifference::Cell {
                        loc: Location::new(grid, tract.cell(index, z)),
                        here,
                        there,
                    });
                }
            }
        }
        let ids: BTreeSet<&SpriteId> = self.sprites.keys().chain(other.sprites.keys()).collect();
        for id in ids {
            let (here, there) = (
                self.sprites.get(id).copied(),
                other.sprites.get(id).copied(),
            );
            if here != there {
                if found.len() >= most {
                    return found;
                }
                found.push(ViewDifference::Sprite {
                    id: *id,
                    here,
                    there,
                });
            }
        }
        let grids: BTreeSet<&GridId> = self.shapes.keys().chain(other.shapes.keys()).collect();
        for grid in grids {
            if self.shapes.get(grid) != other.shapes.get(grid) {
                if found.len() >= most {
                    return found;
                }
                found.push(ViewDifference::Shape { grid: *grid });
            }
        }
        found
    }
}

/// How many slabs a [`GridReader`] remembers: one for each of this many
/// consecutive storeys, which is as far as a view looks down by default
/// and the storey it looks down from.
const READER_SLOTS: usize = 16;

/// A read-only handle on one grid of a view model, for code that reads
/// many of its cells: a painter drawing a view. It remembers where the
/// cells of the storeys it last read are kept, so that reading along a
/// row costs a lookup only where the row enters another tract.
#[derive(Clone, Debug)]
pub struct GridReader<'a> {
    model: &'a ViewModel,
    grid: GridId,
    shape: Option<&'a Topology>,
    /// The slab last read for each storey, by storey modulo the number
    /// of slots: its tract, and the slab if there is one.
    slots: [Option<(i32, TractPos, Option<&'a Slab>)>; READER_SLOTS],
}

impl ViewModel {
    /// A handle for reading many cells of one grid.
    pub fn reader(&self, grid: GridId) -> GridReader<'_> {
        GridReader {
            model: self,
            grid,
            shape: self.shapes.get(&grid),
            slots: [None; READER_SLOTS],
        }
    }
}

impl<'a> GridReader<'a> {
    /// The grid being read.
    pub fn grid(&self) -> GridId {
        self.grid
    }

    /// A position brought inside the grid: see [`ViewModel::normalize`].
    #[inline]
    pub fn normalize(&self, cell: CellPos) -> Option<CellPos> {
        match self.shape {
            Some(shape) => shape.normalize(cell).ok(),
            None => Some(cell),
        }
    }

    /// What is known of a cell, if it has ever been seen: see
    /// [`ViewModel::cell_in`].
    #[inline]
    pub fn cell(&mut self, cell: CellPos) -> Option<&'a KnownCell> {
        let tract = cell.tract();
        let slot = &mut self.slots[cell.z.rem_euclid(READER_SLOTS as i32) as usize];
        let slab = match slot {
            Some((z, of, slab)) if *z == cell.z && *of == tract => *slab,
            _ => {
                let slab = self.model.slabs.get(&(self.grid, cell.z, tract));
                *slot = Some((cell.z, tract, slab));
                slab
            }
        };
        let known = &slab?.cells[slab_index(cell)];
        known.known().then_some(known)
    }

    /// The sprite drawn in a cell: see [`ViewModel::top_sprite_at`].
    #[inline]
    pub fn top_sprite_at(&self, cell: CellPos) -> Option<&'a EntitySprite> {
        self.model.top_sprite_at(self.grid, cell)
    }

    /// The sprite drawn in a cell, leaving some out: see
    /// [`ViewModel::top_sprite_at_except`].
    #[inline]
    pub fn top_sprite_at_except(
        &self,
        cell: CellPos,
        hidden: &[SpriteId],
    ) -> Option<&'a EntitySprite> {
        self.model.top_sprite_at_except(self.grid, cell, hidden)
    }
}

#[cfg(test)]
mod tests {
    use clash_world::{BlockId, MaterialId};

    use super::*;

    fn at(x: i32, y: i32, z: i32) -> Location {
        Location::new(GridId::new(0), CellPos::new(x, y, z))
    }

    fn wall() -> CellContents {
        CellContents {
            block: BlockState::new(BlockId::new(2), MaterialId::new(1), 0),
            ..CellContents::EMPTY
        }
    }

    fn sprite(loc: Location, kind: SpriteKind) -> EntitySprite {
        EntitySprite {
            loc,
            kind,
            glyph: GlyphPart {
                ch: Some('@'),
                ..GlyphPart::default()
            },
            own: None,
            dress: None,
            facing: None,
            suspected: false,
        }
    }

    #[test]
    fn a_cell_is_unknown_then_seen_then_remembered_then_forgotten() {
        let mut model = ViewModel::new();
        assert!(model.cell(at(3, 4, 0)).is_none());
        model.observe_cell(at(3, 4, 0), wall());
        let known = model.cell(at(3, 4, 0)).unwrap();
        assert!(known.seen());
        assert_eq!(known.contents(), wall());
        assert_eq!(model.known_cells(), 1);
        // Its neighbours and the same cell of other storeys and grids
        // stay unknown.
        assert!(model.cell(at(4, 4, 0)).is_none());
        assert!(model.cell(at(3, 4, 1)).is_none());
        assert!(
            model
                .cell(Location::new(GridId::new(1), CellPos::new(3, 4, 0)))
                .is_none()
        );

        assert!(model.unsee_cell(at(3, 4, 0)));
        assert!(!model.unsee_cell(at(3, 4, 0)));
        let known = model.cell(at(3, 4, 0)).unwrap();
        assert!(!known.seen());
        assert_eq!(known.block, wall().block);

        assert!(model.forget_cell(at(3, 4, 0)));
        assert!(!model.forget_cell(at(3, 4, 0)));
        assert!(model.cell(at(3, 4, 0)).is_none());
        assert_eq!(model.known_cells(), 0);
        assert_eq!(model, ViewModel::new(), "an emptied slab leaves no trace");
    }

    #[test]
    fn negative_coordinates_and_tract_edges_address_their_own_cells() {
        let mut model = ViewModel::new();
        for (x, y) in [(-1, -1), (0, 0), (31, 31), (32, 0), (-33, 5)] {
            model.observe_cell(at(x, y, -2), wall());
        }
        assert_eq!(model.known_cells(), 5);
        for (x, y) in [(-1, -1), (0, 0), (31, 31), (32, 0), (-33, 5)] {
            assert!(model.cell(at(x, y, -2)).is_some(), "({x}, {y})");
        }
        assert!(model.cell(at(-2, -1, -2)).is_none());
        assert!(model.cell(at(31, 0, -2)).is_none());
    }

    #[test]
    fn a_box_of_cells_goes_out_of_view_together() {
        let mut model = ViewModel::new();
        let cells = CellBox::new(CellPos::new(-2, -2, 0), CellPos::new(40, 3, 1));
        for cell in cells.cells() {
            model.observe_cell(Location::new(GridId::new(0), cell), wall());
        }
        let half = CellBox::new(CellPos::new(-2, -2, 0), CellPos::new(10, 3, 0));
        model.unsee_box(GridId::new(0), half);
        for cell in cells.cells() {
            let seen = model.cell_in(GridId::new(0), cell).unwrap().seen();
            assert_eq!(seen, !half.contains(cell), "{cell}");
        }
        model.unsee_all();
        assert!(
            cells
                .cells()
                .all(|cell| !model.cell_in(GridId::new(0), cell).unwrap().seen())
        );
    }

    #[test]
    fn light_is_recorded_only_on_known_cells() {
        let mut model = ViewModel::new();
        assert!(!model.observe_light(at(0, 0, 0), 10));
        model.observe_cell(at(0, 0, 0), wall());
        assert_eq!(model.cell(at(0, 0, 0)).unwrap().light, LIGHT_FULL);
        assert!(model.observe_light(at(0, 0, 0), 10));
        assert_eq!(model.cell(at(0, 0, 0)).unwrap().light, 10);
    }

    #[test]
    fn sprites_are_found_where_they_are() {
        let mut model = ViewModel::new();
        let (rat, key, cat) = (SpriteId::new(7), SpriteId::new(3), SpriteId::new(9));
        model.upsert_sprite(key, sprite(at(1, 1, 0), SpriteKind::Object));
        model.upsert_sprite(cat, sprite(at(1, 1, 0), SpriteKind::Creature));
        model.upsert_sprite(rat, sprite(at(1, 1, 0), SpriteKind::Creature));
        // A creature is drawn over an object, and of two creatures the
        // one with the lower id.
        let top = model.top_sprite_at(GridId::new(0), CellPos::new(1, 1, 0));
        assert_eq!(top, model.sprite(rat));
        assert!(
            model
                .top_sprite_at(GridId::new(0), CellPos::new(2, 1, 0))
                .is_none()
        );

        assert!(model.move_sprite(rat, at(2, 1, 0)));
        assert!(model.remove_sprite(cat).is_some());
        assert!(!model.move_sprite(cat, at(5, 5, 0)));
        let top = model.top_sprite_at(GridId::new(0), CellPos::new(1, 1, 0));
        assert_eq!(top.map(|sprite| sprite.kind), Some(SpriteKind::Object));
        let top = model.top_sprite_at(GridId::new(0), CellPos::new(2, 1, 0));
        assert_eq!(top, model.sprite(rat));
        assert_eq!(model.sprites().count(), 2);

        // Replacing a sprite moves it in the index too.
        let before = model.upsert_sprite(key, sprite(at(4, 4, 1), SpriteKind::Object));
        assert_eq!(before.map(|sprite| sprite.loc), Some(at(1, 1, 0)));
        assert!(
            model
                .top_sprite_at(GridId::new(0), CellPos::new(1, 1, 0))
                .is_none()
        );
    }

    #[test]
    fn equal_knowledge_is_equal_whatever_its_history() {
        let mut a = ViewModel::new();
        let mut b = ViewModel::new();
        a.observe_cell(at(0, 0, 0), wall());
        a.observe_cell(at(1, 0, 0), CellContents::EMPTY);
        a.upsert_sprite(SpriteId::new(1), sprite(at(0, 0, 0), SpriteKind::Creature));
        let heard = a.add_heard(HeardMarker {
            loc: at(5, 5, 0),
            ch: None,
        });
        a.remove_heard(heard);
        a.add_heard(HeardMarker {
            loc: at(6, 5, 0),
            ch: Some('!'),
        });

        b.upsert_sprite(SpriteId::new(1), sprite(at(9, 9, 0), SpriteKind::Creature));
        b.move_sprite(SpriteId::new(1), at(0, 0, 0));
        b.observe_cell(at(1, 0, 0), wall());
        b.observe_cell(at(1, 0, 0), CellContents::EMPTY);
        b.observe_cell(at(0, 0, 0), wall());
        b.observe_cell(at(7, 7, 7), wall());
        b.forget_cell(at(7, 7, 7));
        b.add_heard(HeardMarker {
            loc: at(6, 5, 0),
            ch: Some('!'),
        });
        assert_eq!(a, b);
        assert_ne!(a.revision(), b.revision());

        b.unsee_cell(at(0, 0, 0));
        assert_ne!(a, b, "seen and remembered are not the same knowledge");
    }

    #[test]
    fn a_grid_and_everything_can_be_forgotten() {
        let mut model = ViewModel::new();
        let other = Location::new(GridId::new(1), CellPos::new(0, 0, 0));
        model.observe_cell(at(0, 0, 0), wall());
        model.observe_cell(other, wall());
        model.upsert_sprite(SpriteId::new(1), sprite(other, SpriteKind::Creature));
        model.add_heard(HeardMarker {
            loc: other,
            ch: None,
        });
        model.forget_grid(GridId::new(1));
        assert!(model.cell(other).is_none());
        assert!(model.sprite(SpriteId::new(1)).is_none());
        assert_eq!(model.heard().count(), 0);
        assert!(model.cell(at(0, 0, 0)).is_some());
        model.clear();
        assert_eq!(model.known_cells(), 0);
    }

    #[test]
    fn a_reader_gives_what_the_view_model_gives() {
        let mut model = ViewModel::new();
        let cells = CellBox::new(CellPos::new(-40, -3, -20), CellPos::new(70, 2, 20));
        for cell in cells.cells() {
            // Every third cell is known, and each knows a light of its
            // own.
            if (cell.x + cell.y + cell.z).rem_euclid(3) == 0 {
                let contents = CellContents {
                    light: (cell.x * 7 + cell.z).rem_euclid(251) as u8,
                    ..wall()
                };
                model.observe_cell(Location::new(GridId::new(0), cell), contents);
            }
        }
        model.upsert_sprite(SpriteId::new(1), sprite(at(5, 0, 0), SpriteKind::Creature));
        let mut reader = model.reader(GridId::new(0));
        assert_eq!(reader.grid(), GridId::new(0));
        // Row by row, down through more storeys than the reader keeps,
        // and back again.
        let wider = CellBox::new(CellPos::new(-45, -4, -21), CellPos::new(75, 3, 21));
        let backwards: Vec<CellPos> = wider.cells().collect();
        for cell in wider.cells().chain(backwards.into_iter().rev()) {
            assert_eq!(
                reader.cell(cell),
                model.cell_in(GridId::new(0), cell),
                "{cell}"
            );
        }
        assert_eq!(
            reader.top_sprite_at(CellPos::new(5, 0, 0)),
            model.sprite(SpriteId::new(1))
        );
        assert_eq!(
            reader.normalize(CellPos::new(-1, 0, 0)),
            Some(CellPos::new(-1, 0, 0))
        );
        // Another grid's reader finds nothing of this one.
        assert!(
            model
                .reader(GridId::new(1))
                .cell(CellPos::new(0, 0, 0))
                .is_none()
        );
    }

    #[test]
    fn positions_wrap_by_the_shape_given() {
        use clash_world::Extent;
        let mut model = ViewModel::new();
        let grid = GridId::new(0);
        assert_eq!(
            model.normalize(grid, CellPos::new(-1, 0, 0)),
            Some(CellPos::new(-1, 0, 0))
        );
        model.set_grid_shape(
            grid,
            Topology::new(Extent::Wrapped(100), Extent::Bounded(50), 0, 9),
        );
        assert_eq!(
            model.normalize(grid, CellPos::new(-1, 0, 0)),
            Some(CellPos::new(99, 0, 0))
        );
        assert_eq!(model.normalize(grid, CellPos::new(0, -1, 0)), None);
    }
}
