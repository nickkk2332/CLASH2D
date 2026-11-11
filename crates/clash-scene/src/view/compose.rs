//! What a cell shows (12 §6.1).
//!
//! A [`Composer`] turns one cell of the view model into one cell of a
//! frame. It reads the view model and the glyph tables and nothing else,
//! allocates nothing and looks nothing up by name.
//!
//! # The order
//!
//! The character and its colour come from the first of these that the
//! view model holds for the cell:
//!
//! 1. a creature, or the theme's `suspected` mark for a presence that is
//!    suspected and not detected;
//! 2. the theme's `pile` mark where several items lie, or else an
//!    object;
//! 3. the fixture;
//! 4. the block;
//! 5. a field strong enough to have a glyph;
//! 6. the floor;
//! 7. for a cell with none of these, the cell below, and so on down for
//!    `look_down` storeys, each storey dimmer; then the backdrop.
//!
//! The background is that of the highest of them that gives one: a glyph
//! whose `bg` is left out shows what is beneath it.
//!
//! A cell that was never seen shows nothing, and so does a known open
//! cell over one that was never seen: what is down a hole nobody has
//! looked into is not known.
//!
//! # Then
//!
//! In order: a decal tints the terrain's character; light scales the
//! brightness; the ambience filters and dims; and a cell that is
//! remembered and not seen now is drained of most of its colour and
//! dimmed. The numbers are the constants of this module, recorded in
//! 12 §6.1.
//!
//! # Sixteen colours
//!
//! Brightness is carried by a cell's exact colour, which a terminal with
//! sixteen colours cannot show. So that it loses nothing essential, the
//! fallback changes too: a remembered cell's character falls back to
//! dark grey, and one at less than half its brightness to the darker of
//! its colour's pair.

use clash_world::{CellPos, ElementState, GridId, Layer, Location};

use super::ambience::Ambience;
use super::glyphs::WorldGlyphs;
use super::model::{EntitySprite, GridReader, KnownCell, LIGHT_FULL, SpriteKind, ViewModel};
use super::overlay::{Overlay, OverlayValues};
use crate::color::{Ansi16, Color, Rgb};
use crate::frame::{CellStyle, FrameCell};
use crate::glyph::{GlyphInputs, GlyphPart, resolve_part};

/// How many storeys a view looks down through open cells unless told
/// otherwise: the default of the setting `look_down` (12 §13).
pub const LOOK_DOWN_DEFAULT: u8 = 8;

/// A thousand: the whole, where shares are counted in thousandths.
const WHOLE: u32 = 1000;

/// The share of its brightness a cell keeps for each storey it lies
/// below the viewed one, in thousandths.
pub const DEPTH_KEEP: u32 = 700;

/// The share of its brightness a seen cell keeps in no light at all, in
/// thousandths. Between this and the whole, brightness follows the light
/// level in a straight line.
pub const LIGHT_FLOOR: u32 = 250;

/// The share of its colour a remembered cell keeps, in thousandths: the
/// rest goes to the grey of the same lightness.
pub const MEMORY_COLOR_KEEP: u32 = 250;

/// The share of its brightness a remembered cell keeps, in thousandths.
pub const MEMORY_KEEP: u32 = 550;

/// The most a decal tints the character it lies under, in thousandths
/// of the way to the decal's colour, at an amount of 255.
pub const DECAL_TINT_MAX: u32 = 700;

/// The most a field too weak for its glyph tints the background, in
/// thousandths of the way to the field's tint, just below the level its
/// glyph starts at.
pub const FIELD_TINT_MAX: u32 = 500;

/// Below this share of its brightness, in thousandths, a cell's
/// sixteen-colour fallback is the darker of its colour's pair.
const FALLBACK_DARKER_BELOW: u32 = 500;

/// How significant what a cell shows is: the order of 12 §6.1, with
/// what is not known at all below everything. A view zoomed out shows,
/// for each block of cells, the cell of the highest rank.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub enum Rank {
    /// Never seen, or beyond the edge of the grid.
    Unknown,
    /// Open all the way down: the backdrop.
    Backdrop,
    /// Something on a storey below, seen through open cells.
    Below,
    /// A floor.
    Floor,
    /// A field strong enough to have a glyph.
    Field,
    /// A block.
    Block,
    /// A fixture.
    Fixture,
    /// Items: a pile, or an object.
    Items,
    /// A creature, or a suspected presence.
    Creature,
}

/// What a cell shows, and how significant it is.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Shown {
    /// The frame cell.
    pub cell: FrameCell,
    /// What kind of thing it shows.
    pub rank: Rank,
}

/// Which overlay a view draws over its cells.
#[derive(Clone, Copy, Debug, Default)]
pub enum OverlayDraw<'a> {
    /// None.
    #[default]
    None,
    /// The built-in `light` overlay: each known cell shaded by its light
    /// level.
    Light,
    /// An `overlay` definition, with what evaluates its expression.
    Custom(&'a Overlay, &'a dyn OverlayValues),
}

/// Turns cells of a view model into cells of a frame. See the
/// [module](self).
#[derive(Clone, Copy, Debug)]
pub struct Composer<'a> {
    /// The picture to draw from.
    pub model: &'a ViewModel,
    /// How each element state is drawn.
    pub glyphs: &'a WorldGlyphs,
    /// The ambience of where the viewed character is.
    pub ambience: Option<&'a Ambience>,
    /// How many storeys to look down through open cells.
    pub look_down: u8,
    /// The overlay drawn over the cells.
    pub overlay: OverlayDraw<'a>,
}

/// `rgb` at a share of its brightness, in thousandths.
#[inline]
fn scale(rgb: Rgb, keep: u32) -> Rgb {
    rgb.toward(Rgb::BLACK, (WHOLE - keep.min(WHOLE)) as u16)
}

/// The grey of the same lightness as a colour.
#[inline]
fn grey(rgb: Rgb) -> Rgb {
    // Integer weights of the three channels, in thousandths.
    const RED: u32 = 299;
    const GREEN: u32 = 587;
    const BLUE: u32 = 114;
    let luma =
        (RED * u32::from(rgb.r) + GREEN * u32::from(rgb.g) + BLUE * u32::from(rgb.b) + WHOLE / 2)
            / WHOLE;
    let luma = luma.min(u32::from(u8::MAX)) as u8;
    Rgb::new(luma, luma, luma)
}

/// The darker of a terminal colour's pair: the plain colour of a bright
/// one, and dark grey for light grey.
const fn darker(color: Ansi16) -> Ansi16 {
    match color {
        Ansi16::BrightRed => Ansi16::Red,
        Ansi16::BrightGreen => Ansi16::Green,
        Ansi16::BrightYellow => Ansi16::Yellow,
        Ansi16::BrightBlue => Ansi16::Blue,
        Ansi16::BrightMagenta => Ansi16::Magenta,
        Ansi16::BrightCyan => Ansi16::Cyan,
        Ansi16::BrightWhite => Ansi16::White,
        Ansi16::White => Ansi16::BrightBlack,
        other => other,
    }
}

/// The share of its brightness a seen cell keeps at a light level, in
/// thousandths.
#[inline]
fn light_keep(light: u8) -> u32 {
    LIGHT_FLOOR + (WHOLE - LIGHT_FLOOR) * u32::from(light) / u32::from(LIGHT_FULL)
}

/// What the layers of one cell came to.
struct Layers {
    cell: FrameCell,
    rank: Rank,
}

/// What the last steps of composing a cell need to know about it.
struct Finish<'k> {
    /// What kind of thing is shown.
    rank: Rank,
    /// The cell of the viewed storey.
    top: Location,
    /// What is known of that cell.
    top_known: &'k KnownCell,
    /// What is known of the cell that holds what is shown: the same, or
    /// one below it.
    shown_known: &'k KnownCell,
    /// How many storeys below the viewed one what is shown lies.
    depth: u32,
    /// Whether any cell looked through is remembered and not seen now.
    remembered: bool,
}

impl<'a> Composer<'a> {
    /// A composer with no ambience and no overlay that looks down the
    /// default number of storeys.
    pub fn new(model: &'a ViewModel, glyphs: &'a WorldGlyphs) -> Composer<'a> {
        Composer {
            model,
            glyphs,
            ambience: None,
            look_down: LOOK_DOWN_DEFAULT,
            overlay: OverlayDraw::None,
        }
    }

    fn unknown(&self) -> Shown {
        Shown {
            cell: FrameCell::blank(self.glyphs.base()),
            rank: Rank::Unknown,
        }
    }

    /// One element's glyph laid over what is beneath it.
    #[inline]
    fn element(
        &self,
        layer: Layer,
        state: ElementState,
        section: bool,
        under: &mut Layers,
        rank: Rank,
    ) {
        let Some(mut part) = self.glyphs.element(layer, state) else {
            return;
        };
        if section && layer == Layer::Floor {
            // Seen edge on, a floor is a line at the foot of its cell.
            part.ch = Some(self.glyphs.marks().section_floor);
        }
        self.lay(
            part,
            self.glyphs.material(state.material),
            None,
            under,
            rank,
        );
    }

    /// One glyph laid over what is beneath it: its character and its
    /// colour replace what was there, and so does its background if it
    /// gives one.
    #[inline]
    fn lay(
        &self,
        part: GlyphPart,
        material: Option<Color>,
        sprite: Option<&EntitySprite>,
        under: &mut Layers,
        rank: Rank,
    ) {
        let base = self.glyphs.base();
        let inputs = GlyphInputs {
            material,
            own: sprite.and_then(|sprite| sprite.own),
            dress: sprite.and_then(|sprite| sprite.dress),
            ..GlyphInputs::default()
        };
        // Only the background shows through from beneath.
        let beneath = CellStyle {
            bg: under.cell.bg,
            ..base
        };
        under.cell = resolve_part(part, &inputs, beneath);
        under.rank = rank;
    }

    /// The layers of one known cell, bottom to top. `None` if the cell
    /// shows nothing of its own.
    fn layers(
        &self,
        reader: &GridReader<'a>,
        at: CellPos,
        known: &KnownCell,
        section: bool,
    ) -> Layers {
        let marks = self.glyphs.marks();
        let mut layers = Layers {
            cell: FrameCell::blank(self.glyphs.base()),
            rank: Rank::Backdrop,
        };
        self.element(
            Layer::Floor,
            known.floor.raw(),
            section,
            &mut layers,
            Rank::Floor,
        );
        if let Some(seen) = known.field
            && let Some(look) = self.glyphs.field(seen.field)
        {
            if seen.level >= look.glyph_from {
                let band = usize::from(look.band.max(1));
                let inputs = GlyphInputs {
                    level: Some(usize::from(seen.level - look.glyph_from) / band),
                    ..GlyphInputs::default()
                };
                let beneath = CellStyle {
                    bg: layers.cell.bg,
                    ..self.glyphs.base()
                };
                layers.cell = look.glyph.cell(&inputs, beneath);
                layers.rank = Rank::Field;
            } else if let Some(tint) = look.tint {
                // Too weak for its glyph: it tints the background.
                let strength =
                    FIELD_TINT_MAX * u32::from(seen.level) / u32::from(look.glyph_from.max(1));
                layers.cell.bg =
                    Color::from_rgb(layers.cell.bg.rgb.toward(tint.rgb, strength as u16));
            }
        }
        self.element(
            Layer::Block,
            known.block.raw(),
            section,
            &mut layers,
            Rank::Block,
        );
        self.element(
            Layer::Fixture,
            known.fixture.raw(),
            section,
            &mut layers,
            Rank::Fixture,
        );
        let sprite = reader.top_sprite_at(at);
        if known.pile() {
            self.lay(marks.pile, None, None, &mut layers, Rank::Items);
        } else if let Some(object) = sprite.filter(|sprite| sprite.kind == SpriteKind::Object) {
            self.lay(object.glyph, None, Some(object), &mut layers, Rank::Items);
        }
        if let Some(creature) = sprite.filter(|sprite| sprite.kind == SpriteKind::Creature) {
            let part = if creature.suspected {
                marks.suspected
            } else {
                creature.glyph
            };
            self.lay(part, None, Some(creature), &mut layers, Rank::Creature);
        }
        layers
    }

    /// The steps of 12 §6.1 after the layers: decal, light, ambience and
    /// memory, then the overlay.
    fn finish(&self, mut cell: FrameCell, of: Finish<'_>) -> FrameCell {
        let Finish {
            rank,
            top,
            top_known,
            shown_known,
            depth,
            remembered,
        } = of;
        // A decal marks the terrain, not who stands on it.
        let terrain = matches!(
            rank,
            Rank::Floor | Rank::Block | Rank::Fixture | Rank::Below
        );
        if terrain
            && let Some(decal) = shown_known.decal
            && let Some(tint) = self.glyphs.decal(decal.kind)
        {
            let strength = DECAL_TINT_MAX * u32::from(decal.amount) / u32::from(u8::MAX);
            // The mark's own terminal colour once it is most of what
            // shows.
            let fallback = if strength >= WHOLE / 2 {
                tint.fallback
            } else {
                cell.fg.fallback
            };
            cell.fg = Color::new(cell.fg.rgb.toward(tint.rgb, strength as u16), fallback);
        }

        let mut keep = light_keep(shown_known.light);
        for _ in 0..depth {
            keep = keep * DEPTH_KEEP / WHOLE;
        }
        if let Some(ambience) = self.ambience {
            keep = keep * u32::from(ambience.keep()) / WHOLE;
        }
        if remembered {
            keep = keep * MEMORY_KEEP / WHOLE;
        }
        let shade = |color: Color, character: bool| -> Color {
            let mut rgb = color.rgb;
            if let Some(ambience) = self.ambience {
                rgb = ambience.filter(rgb);
            }
            if remembered {
                rgb = rgb.toward(grey(rgb), (WHOLE - MEMORY_COLOR_KEEP) as u16);
            }
            let fallback = if !character {
                color.fallback
            } else if remembered && color.fallback != Ansi16::Black {
                Ansi16::BrightBlack
            } else if keep < FALLBACK_DARKER_BELOW {
                darker(color.fallback)
            } else {
                color.fallback
            };
            Color::new(scale(rgb, keep), fallback)
        };
        cell.fg = shade(cell.fg, true);
        cell.bg = shade(cell.bg, false);

        match self.overlay {
            OverlayDraw::None => {}
            OverlayDraw::Light => {
                let shade = self.glyphs.overlay_light().rgb;
                let level = WHOLE * u32::from(top_known.light) / u32::from(LIGHT_FULL);
                cell.bg = Color::from_rgb(scale(shade, level));
            }
            OverlayDraw::Custom(overlay, values) => {
                if let Some(value) = values.value(overlay, top, top_known)
                    && value > 0.0
                {
                    let strength = (value.min(1.0) * f64::from(WHOLE)) as u32;
                    cell.bg =
                        Color::new(scale(overlay.shade.rgb, strength), overlay.shade.fallback);
                }
            }
        }
        cell
    }

    /// What a cell shows seen from above (12 §6.1): a `map_view`.
    pub fn plan(&self, grid: GridId, cell: CellPos) -> Shown {
        self.plan_with(&mut self.model.reader(grid), cell)
    }

    /// [`Composer::plan`] for one cell of many: `reader` is a reader of
    /// this composer's view model, kept from cell to cell.
    pub fn plan_with(&self, reader: &mut GridReader<'a>, cell: CellPos) -> Shown {
        let grid = reader.grid();
        let Some(top) = reader.normalize(cell) else {
            return self.unknown();
        };
        let Some(top_known) = reader.cell(top) else {
            return self.unknown();
        };
        let mut at = top;
        let mut known = top_known;
        let mut remembered = !known.seen();
        let mut depth = 0u32;
        loop {
            let layers = self.layers(reader, at, known, false);
            if layers.rank > Rank::Backdrop {
                let rank = if depth > 0 { Rank::Below } else { layers.rank };
                let cell = self.finish(
                    layers.cell,
                    Finish {
                        rank,
                        top: Location::new(grid, top),
                        top_known,
                        shown_known: known,
                        depth,
                        remembered,
                    },
                );
                return Shown { cell, rank };
            }
            if depth >= u32::from(self.look_down) {
                // Open as far down as the view looks: the backdrop.
                let mut backdrop = Layers {
                    cell: layers.cell,
                    rank: Rank::Backdrop,
                };
                self.lay(
                    self.glyphs.marks().backdrop,
                    None,
                    None,
                    &mut backdrop,
                    Rank::Backdrop,
                );
                let cell = self.finish(
                    backdrop.cell,
                    Finish {
                        rank: Rank::Backdrop,
                        top: Location::new(grid, top),
                        top_known,
                        shown_known: top_known,
                        depth: 0,
                        remembered,
                    },
                );
                return Shown {
                    cell,
                    rank: Rank::Backdrop,
                };
            }
            depth += 1;
            at = at.below();
            match reader.cell(at) {
                Some(below) => {
                    known = below;
                    remembered |= !below.seen();
                }
                // What is down there was never seen.
                None => return self.unknown(),
            }
        }
    }

    /// What a cell shows seen from the side: a `section_view`. There is
    /// no looking down: a cell with nothing in it is air.
    pub fn section(&self, grid: GridId, cell: CellPos) -> Shown {
        self.section_with(&mut self.model.reader(grid), cell)
    }

    /// [`Composer::section`] for one cell of many: `reader` is a reader
    /// of this composer's view model, kept from cell to cell.
    pub fn section_with(&self, reader: &mut GridReader<'a>, cell: CellPos) -> Shown {
        let grid = reader.grid();
        let Some(at) = reader.normalize(cell) else {
            return self.unknown();
        };
        let Some(known) = reader.cell(at) else {
            return self.unknown();
        };
        let layers = self.layers(reader, at, known, true);
        let cell = self.finish(
            layers.cell,
            Finish {
                rank: layers.rank,
                top: Location::new(grid, at),
                top_known: known,
                shown_known: known,
                depth: 0,
                remembered: !known.seen(),
            },
        );
        Shown {
            cell,
            rank: layers.rank,
        }
    }

    /// What a block of `zoom` by `zoom` cells shows in a view zoomed out
    /// (12 §6.2): its most significant cell by [`Rank`], and of equals
    /// the first, row by row. `corner` is the block's cell of lowest
    /// coordinates.
    pub fn plan_block(&self, grid: GridId, corner: CellPos, zoom: u32) -> Shown {
        self.plan_block_with(&mut self.model.reader(grid), corner, zoom)
    }

    /// [`Composer::plan_block`] for one block of many: `reader` is a
    /// reader of this composer's view model, kept from block to block.
    pub fn plan_block_with(
        &self,
        reader: &mut GridReader<'a>,
        corner: CellPos,
        zoom: u32,
    ) -> Shown {
        let side = zoom.max(1) as i32;
        let mut best = self.plan_with(reader, corner);
        for dy in 0..side {
            for dx in 0..side {
                if dx == 0 && dy == 0 {
                    continue;
                }
                if best.rank == Rank::Creature {
                    return best;
                }
                let cell = CellPos::new(
                    corner.x.wrapping_add(dx),
                    corner.y.wrapping_add(dy),
                    corner.z,
                );
                let shown = self.plan_with(reader, cell);
                if shown.rank > best.rank {
                    best = shown;
                }
            }
        }
        best
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn brightness_follows_light_in_a_straight_line_above_a_floor() {
        assert_eq!(light_keep(0), LIGHT_FLOOR);
        assert_eq!(light_keep(LIGHT_FULL), WHOLE);
        assert!(light_keep(100) > light_keep(99));
        assert_eq!(scale(Rgb::new(200, 100, 0), 500), Rgb::new(100, 50, 0));
        assert_eq!(scale(Rgb::WHITE, 5000), Rgb::WHITE);
        assert_eq!(scale(Rgb::WHITE, 0), Rgb::BLACK);
    }

    #[test]
    fn grey_keeps_lightness_and_drops_colour() {
        assert_eq!(grey(Rgb::WHITE), Rgb::WHITE);
        assert_eq!(grey(Rgb::BLACK), Rgb::BLACK);
        assert_eq!(grey(Rgb::new(255, 0, 0)), Rgb::new(76, 76, 76));
        assert_eq!(grey(Rgb::new(90, 90, 90)), Rgb::new(90, 90, 90));
    }

    #[test]
    fn every_terminal_colour_has_a_darker_one_that_is_not_black() {
        for color in Ansi16::ALL {
            if color != Ansi16::Black {
                assert_ne!(darker(color), Ansi16::Black, "{color} would vanish");
            }
        }
        assert_eq!(darker(Ansi16::BrightRed), Ansi16::Red);
        assert_eq!(darker(Ansi16::White), Ansi16::BrightBlack);
        assert_eq!(darker(Ansi16::Green), Ansi16::Green);
    }

    #[test]
    fn ranks_are_in_the_order_of_the_specification() {
        let order = [
            Rank::Unknown,
            Rank::Backdrop,
            Rank::Below,
            Rank::Floor,
            Rank::Field,
            Rank::Block,
            Rank::Fixture,
            Rank::Items,
            Rank::Creature,
        ];
        assert!(order.windows(2).all(|pair| pair[0] < pair[1]));
    }
}
