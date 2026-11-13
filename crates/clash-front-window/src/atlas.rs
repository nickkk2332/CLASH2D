//! [`GlyphAtlas`]: the glyphs of the theme's font, rasterised by
//! `fontdue` once and kept.
//!
//! # The cell
//!
//! The theme's font `size` is the height of a cell in pixels (12 §4.3).
//! The font is scaled so that its line, from the top of its ascent to
//! the bottom of its descent with its line gap, fills that height, with
//! the top of the ascent on the top edge of the cell and less than a
//! pixel cut off at the bottom; that is what lets the line-drawing
//! characters of borders meet from one cell to the next. A cell is as
//! wide as the font's advance at that scale, rounded down, so that
//! horizontal lines overlap by a fraction of a pixel rather than leave a
//! gap. The fonts meant are monospaced; the advance is that of `M`.
//!
//! # Glyphs
//!
//! A glyph is kept as a coverage mask exactly one cell in size, at its
//! height (sprites may be drawn larger or smaller), already placed on
//! the baseline, so that drawing it is copying rows. A mask is made the
//! first time a character is asked for in a style and at a height, and
//! kept: after the first frames nothing is rasterised and nothing is
//! allocated.
//!
//! The styles are made from the one font file: bold by thickening each
//! stroke sideways, italic by slanting the rows, underline as a line
//! under the baseline. The line-drawing characters of one weight (light,
//! heavy, double) and the half and full blocks are drawn in whole pixels
//! instead of taken from the font, so that borders are crisp and meet
//! from cell to cell at any size. A character the font does not have is drawn as
//! a hollow box, so that it shows and is not mistaken for a space; a
//! control character is drawn as nothing, as the terminal draws it.

use std::collections::HashMap;
use std::fmt;
use std::path::{Path, PathBuf};

use clash_scene::Attrs;
use fontdue::{Font, FontSettings};

/// How many characters, from U+0000, are looked up through a table at
/// the cell's own height instead of the map: the printable ASCII range
/// and the control characters before it.
const FAST_CHARS: usize = 128;

/// How many glyphs are kept before the cache is emptied and begun
/// again, so that sprites drawn at many sizes cannot fill memory.
const MAX_GLYPHS: usize = 16_384;

/// The character whose advance gives the width of a cell.
const WIDTH_CHAR: char = 'M';

/// A coverage value that covers a pixel entirely.
const FULL: u8 = u8::MAX;

/// How far bold thickens a stroke: one pixel for every this many
/// pixels of cell height, and at least one.
const BOLD_HEIGHT_PER_PIXEL: u32 = 16;

/// How far italic slants a row: this many thousandths of a pixel
/// sideways for each pixel above the baseline.
const ITALIC_SLANT_PERMILLE: i32 = 200;

/// Where an underline is drawn: this many pixels below the baseline for
/// every pixel of cell height, at least one.
const UNDERLINE_DROP_PER_HEIGHT: u32 = 10;

/// How thick an underline is: one pixel for every this many pixels of
/// cell height, and at least one.
const UNDERLINE_HEIGHT_PER_PIXEL: u32 = 16;

/// The margin of the box drawn for a missing character: one pixel for
/// every this many pixels of the cell's width or height, at least one.
const MISSING_BOX_MARGIN_DIVISOR: u32 = 6;

/// The smallest cell height a font is drawn at, in pixels.
pub const MIN_CELL_HEIGHT: u32 = 4;

/// The largest cell height a font is drawn at, in pixels.
pub const MAX_CELL_HEIGHT: u32 = 512;

/// How a glyph is drawn, beyond its character: the attributes of a cell
/// that change its shape. Colour and the rest are the renderer's.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Default)]
pub struct GlyphStyle(u8);

impl GlyphStyle {
    /// As the font draws it.
    pub const PLAIN: GlyphStyle = GlyphStyle(0);
    /// Heavier strokes.
    pub const BOLD: GlyphStyle = GlyphStyle(1);
    /// Slanted.
    pub const ITALIC: GlyphStyle = GlyphStyle(2);
    /// A line beneath.
    pub const UNDERLINE: GlyphStyle = GlyphStyle(4);
    /// How many styles there are.
    const COUNT: usize = 8;

    /// The style a cell's attributes ask for.
    pub fn from_attrs(attrs: Attrs) -> GlyphStyle {
        let mut bits = 0;
        if attrs.contains(Attrs::BOLD) {
            bits |= GlyphStyle::BOLD.0;
        }
        if attrs.contains(Attrs::ITALIC) {
            bits |= GlyphStyle::ITALIC.0;
        }
        if attrs.contains(Attrs::UNDERLINE) {
            bits |= GlyphStyle::UNDERLINE.0;
        }
        GlyphStyle(bits)
    }

    /// Whether every part of `other` is in this style.
    pub const fn contains(self, other: GlyphStyle) -> bool {
        self.0 & other.0 == other.0
    }

    /// Both styles together.
    pub const fn with(self, other: GlyphStyle) -> GlyphStyle {
        GlyphStyle(self.0 | other.0)
    }
}

/// Why a font could not be used.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct FontError {
    /// The font file.
    pub path: PathBuf,
    /// What went wrong.
    pub reason: String,
}

impl fmt::Display for FontError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "the font `{}` cannot be used: {}",
            self.path.display(),
            self.reason
        )
    }
}

impl std::error::Error for FontError {}

/// Which glyph of an atlas: what [`GlyphAtlas::glyph`] gives, to be
/// handed straight to [`GlyphAtlas::mask`]. It is good until the next
/// glyph is asked for, which may empty the cache.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct GlyphId(u32);

/// A glyph's coverage: one byte per pixel, row by row from the top, 0
/// for none and 255 for all.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct GlyphMask<'a> {
    /// The width in pixels.
    pub width: u32,
    /// The height in pixels.
    pub height: u32,
    /// The coverage, `width * height` bytes.
    pub coverage: &'a [u8],
    /// Whether nothing is covered: the glyph draws nothing.
    pub blank: bool,
}

/// One kept glyph: where its mask is in the arena.
#[derive(Clone, Copy, Debug)]
struct Entry {
    offset: u32,
    width: u16,
    height: u16,
    blank: bool,
}

/// A character in a style at a height.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
struct Key {
    ch: char,
    style: GlyphStyle,
    height: u16,
}

/// The size of the cell at one height, and where its baseline is.
#[derive(Clone, Copy, Debug)]
struct Geometry {
    width: u32,
    height: u32,
    /// Pixels from the top of the cell to the baseline.
    baseline: i32,
    /// The font's size in pixels per em.
    px: f32,
}

/// The font's proportions in its own units.
#[derive(Clone, Copy, Debug)]
struct Proportions {
    units_per_em: f32,
    /// From the top of the line to the baseline: the ascent and half
    /// the line gap.
    above: f32,
    /// The whole line: ascent, descent and line gap.
    line: f32,
    /// The advance of a character.
    advance: f32,
}

/// The glyphs of one font, rasterised as they are asked for and kept.
/// See the [module](self) documentation.
pub struct GlyphAtlas {
    font: Font,
    proportions: Proportions,
    cell: Geometry,
    entries: Vec<Entry>,
    arena: Vec<u8>,
    map: HashMap<Key, u32>,
    /// The glyphs of the first [`FAST_CHARS`] characters at the cell's
    /// own height, by style and character; `u32::MAX` for not yet made.
    fast: Vec<u32>,
}

impl fmt::Debug for GlyphAtlas {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("GlyphAtlas")
            .field("cell_width", &self.cell.width)
            .field("cell_height", &self.cell.height)
            .field("glyphs", &self.entries.len())
            .finish_non_exhaustive()
    }
}

impl GlyphAtlas {
    /// An atlas of the font in a file, with cells `cell_height` pixels
    /// high. A file that cannot be read or is not a font is an error
    /// that names it.
    pub fn open(path: &Path, cell_height: u32) -> Result<GlyphAtlas, FontError> {
        let error = |reason: String| FontError {
            path: path.to_path_buf(),
            reason,
        };
        let bytes = std::fs::read(path).map_err(|e| error(e.to_string()))?;
        GlyphAtlas::from_bytes(&bytes, cell_height).map_err(error)
    }

    /// An atlas of a font held in memory (a TrueType or OpenType file's
    /// bytes), with cells `cell_height` pixels high. What is wrong with
    /// the font is returned as a sentence.
    pub fn from_bytes(bytes: &[u8], cell_height: u32) -> Result<GlyphAtlas, String> {
        let font = Font::from_bytes(bytes, FontSettings::default())
            .map_err(|reason| format!("it is not a font that can be read ({reason})"))?;
        let units_per_em = font.units_per_em();
        if !(units_per_em.is_finite() && units_per_em > 0.0) {
            return Err("its size in font units is not given".to_owned());
        }
        let (ascent, descent, gap) = font
            .horizontal_line_metrics(units_per_em)
            .map_or((units_per_em, 0.0, 0.0), |line| {
                (line.ascent, line.descent, line.line_gap)
            });
        let line = ascent - descent + gap;
        if !(line.is_finite() && line > 0.0) {
            return Err("its line has no height".to_owned());
        }
        let advance = [WIDTH_CHAR, '0', '#']
            .into_iter()
            .map(|ch| font.metrics(ch, units_per_em).advance_width)
            .find(|advance| advance.is_finite() && *advance > 0.0)
            .unwrap_or(units_per_em / 2.0);
        let proportions = Proportions {
            units_per_em,
            above: ascent + gap / 2.0,
            line,
            advance,
        };
        let mut atlas = GlyphAtlas {
            font,
            proportions,
            cell: geometry(proportions, MIN_CELL_HEIGHT),
            entries: Vec::new(),
            arena: Vec::new(),
            map: HashMap::new(),
            fast: vec![u32::MAX; FAST_CHARS * GlyphStyle::COUNT],
        };
        atlas.set_cell_height(cell_height);
        Ok(atlas)
    }

    /// Changes the height of a cell, as when the window moves to a
    /// screen of another scale, and forgets every glyph. The height is
    /// kept between [`MIN_CELL_HEIGHT`] and [`MAX_CELL_HEIGHT`].
    pub fn set_cell_height(&mut self, cell_height: u32) {
        let cell_height = cell_height.clamp(MIN_CELL_HEIGHT, MAX_CELL_HEIGHT);
        self.cell = geometry(self.proportions, cell_height);
        self.forget();
    }

    /// Forgets every glyph made.
    fn forget(&mut self) {
        self.entries.clear();
        self.arena.clear();
        self.map.clear();
        self.fast.fill(u32::MAX);
    }

    /// The width of a cell in pixels.
    pub fn cell_width(&self) -> u32 {
        self.cell.width
    }

    /// The height of a cell in pixels.
    pub fn cell_height(&self) -> u32 {
        self.cell.height
    }

    /// The width of a cell that is `height` pixels high.
    pub fn width_at(&self, height: u32) -> u32 {
        geometry(self.proportions, height.max(1)).width
    }

    /// How many glyphs are kept.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Whether no glyph is kept.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Whether the font has a glyph for the character.
    pub fn has_char(&self, ch: char) -> bool {
        self.font.lookup_glyph_index(ch) != 0
    }

    /// The glyph of a character in a style at the cell's own height.
    pub fn glyph(&mut self, ch: char, style: GlyphStyle) -> GlyphId {
        let code = ch as usize;
        if code < FAST_CHARS {
            let slot = usize::from(style.0) * FAST_CHARS + code;
            let found = self.fast[slot];
            if found != u32::MAX {
                return GlyphId(found);
            }
            let height = self.cell.height;
            let made = self.glyph_at(ch, style, height);
            // Making it may have emptied the cache, table and all.
            self.fast[slot] = made.0;
            return made;
        }
        let height = self.cell.height;
        self.glyph_at(ch, style, height)
    }

    /// The glyph of a character in a style in a cell `height` pixels
    /// high, for a sprite drawn larger or smaller than a cell.
    pub fn glyph_at(&mut self, ch: char, style: GlyphStyle, height: u32) -> GlyphId {
        let height = height.clamp(1, MAX_CELL_HEIGHT);
        let key = Key {
            ch,
            style,
            height: height as u16,
        };
        if let Some(&found) = self.map.get(&key) {
            return GlyphId(found);
        }
        if self.entries.len() >= MAX_GLYPHS {
            self.forget();
        }
        let geometry = geometry(self.proportions, height);
        let start = self.arena.len();
        let size = geometry.width as usize * geometry.height as usize;
        self.arena.resize(start + size, 0);
        let mask = &mut self.arena[start..];
        draw_glyph(&self.font, ch, style, geometry, mask);
        let blank = mask.iter().all(|&c| c == 0);
        let index = self.entries.len() as u32;
        self.entries.push(Entry {
            offset: start as u32,
            width: geometry.width as u16,
            height: geometry.height as u16,
            blank,
        });
        self.map.insert(key, index);
        GlyphId(index)
    }

    /// The mask of a glyph.
    pub fn mask(&self, id: GlyphId) -> GlyphMask<'_> {
        match self.entries.get(id.0 as usize) {
            Some(entry) => {
                let start = entry.offset as usize;
                let size = usize::from(entry.width) * usize::from(entry.height);
                GlyphMask {
                    width: u32::from(entry.width),
                    height: u32::from(entry.height),
                    coverage: &self.arena[start..start + size],
                    blank: entry.blank,
                }
            }
            // An id from before the cache was emptied: nothing.
            None => GlyphMask {
                width: 0,
                height: 0,
                coverage: &[],
                blank: true,
            },
        }
    }
}

/// The cell at a height.
///
/// The rasteriser puts the baseline on a pixel's edge, so the top of the
/// line meets the top of the cell only if the ascent is a whole number
/// of pixels. The baseline is therefore the line's ascent at this
/// height rounded up, and the font is scaled to make the ascent exactly
/// that: the line then fills the cell from its top edge, and runs past
/// its bottom by less than a pixel, which is cut off. Characters that
/// fill the line, such as the vertical line and the full block, cover
/// the whole height of the cell and meet the next cell without a seam.
fn geometry(proportions: Proportions, height: u32) -> Geometry {
    let per_unit = height as f32 / proportions.line;
    let (baseline, per_unit) = if proportions.above > 0.0 {
        let baseline = (proportions.above * per_unit).ceil().max(1.0);
        (baseline, baseline / proportions.above)
    } else {
        (0.0, per_unit)
    };
    Geometry {
        width: ((proportions.advance * per_unit) as u32).max(1),
        height,
        baseline: baseline as i32,
        px: proportions.units_per_em * per_unit,
    }
}

/// Draws a character's coverage into a mask one cell in size.
fn draw_glyph(font: &Font, ch: char, style: GlyphStyle, cell: Geometry, mask: &mut [u8]) {
    let (width, height) = (cell.width as usize, cell.height as usize);
    if crate::lines::draw_drawn(ch, cell.width, cell.height, mask) {
        // A line-drawing character or a block, drawn in whole pixels.
    } else if !ch.is_control() {
        let index = font.lookup_glyph_index(ch);
        if index != 0 {
            let (metrics, bitmap) = font.rasterize_indexed(index, cell.px);
            // The bitmap's bottom edge is `ymin` above the baseline.
            let top = cell.baseline - (metrics.ymin + metrics.height as i32);
            for (row, line) in bitmap.chunks_exact(metrics.width.max(1)).enumerate() {
                let y = top + row as i32;
                if y < 0 || y as usize >= height {
                    continue;
                }
                for (column, &coverage) in line.iter().enumerate() {
                    let x = metrics.xmin + column as i32;
                    if x >= 0 && (x as usize) < width {
                        let at = y as usize * width + x as usize;
                        mask[at] = mask[at].max(coverage);
                    }
                }
            }
        } else if !ch.is_whitespace() {
            missing_box(cell, mask);
        }
    }
    if style.contains(GlyphStyle::BOLD) {
        embolden(cell, mask);
    }
    if style.contains(GlyphStyle::ITALIC) {
        slant(cell, mask);
    }
    if style.contains(GlyphStyle::UNDERLINE) {
        underline(cell, mask);
    }
}

/// The hollow box drawn for a character the font does not have.
fn missing_box(cell: Geometry, mask: &mut [u8]) {
    let (width, height) = (cell.width, cell.height);
    let margin_x = (width / MISSING_BOX_MARGIN_DIVISOR).max(1);
    let margin_y = (height / MISSING_BOX_MARGIN_DIVISOR).max(1);
    if width <= 2 * margin_x + 1 || height <= 2 * margin_y + 1 {
        mask.fill(FULL);
        return;
    }
    let (left, right) = (margin_x, width - 1 - margin_x);
    let (top, bottom) = (margin_y, height - 1 - margin_y);
    for y in top..=bottom {
        for x in left..=right {
            if x == left || x == right || y == top || y == bottom {
                mask[(y * width + x) as usize] = FULL;
            }
        }
    }
}

/// Thickens every stroke sideways, to the right.
fn embolden(cell: Geometry, mask: &mut [u8]) {
    let by = (cell.height / BOLD_HEIGHT_PER_PIXEL).max(1) as usize;
    for row in mask.chunks_exact_mut(cell.width as usize) {
        for _ in 0..by {
            for x in (1..row.len()).rev() {
                row[x] = row[x].max(row[x - 1]);
            }
        }
    }
}

/// Slants the rows: those above the baseline to the right, those below
/// to the left.
fn slant(cell: Geometry, mask: &mut [u8]) {
    let width = cell.width as usize;
    let mut moved = vec![0_u8; width];
    for (y, row) in mask.chunks_exact_mut(width).enumerate() {
        let rise = cell.baseline - y as i32;
        // Rounded to the nearest pixel, half away from the baseline.
        let shift = (rise * ITALIC_SLANT_PERMILLE + rise.signum() * 500) / 1000;
        if shift == 0 {
            continue;
        }
        moved.fill(0);
        for (x, &coverage) in row.iter().enumerate() {
            let to = x as i32 + shift;
            if to >= 0 && (to as usize) < width {
                moved[to as usize] = coverage;
            }
        }
        row.copy_from_slice(&moved);
    }
}

/// A line beneath the baseline, across the whole cell.
fn underline(cell: Geometry, mask: &mut [u8]) {
    let drop = (cell.height / UNDERLINE_DROP_PER_HEIGHT).max(1) as i32;
    let thickness = (cell.height / UNDERLINE_HEIGHT_PER_PIXEL).max(1) as i32;
    let last = cell.height as i32 - 1;
    let top = (cell.baseline + drop).min(last);
    let bottom = (top + thickness).min(last + 1);
    let width = cell.width as usize;
    for y in top.max(0)..bottom {
        let start = y as usize * width;
        mask[start..start + width].fill(FULL);
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// The font bundled with the interface mod.
    pub(crate) fn font_bytes() -> Vec<u8> {
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../mods/ui/assets/font.ttf");
        std::fs::read(path).expect("the bundled font is in mods/ui/assets")
    }

    fn atlas(height: u32) -> GlyphAtlas {
        GlyphAtlas::from_bytes(&font_bytes(), height).unwrap()
    }

    fn rows(mask: GlyphMask<'_>) -> Vec<String> {
        mask.coverage
            .chunks_exact(mask.width as usize)
            .map(|row| {
                row.iter()
                    .map(|&c| match c {
                        0 => '.',
                        1..=127 => '+',
                        _ => '#',
                    })
                    .collect()
            })
            .collect()
    }

    #[test]
    fn the_cell_is_the_themes_height_and_the_fonts_advance() {
        let atlas = atlas(16);
        assert_eq!((atlas.cell_width(), atlas.cell_height()), (8, 16));
        let atlas = GlyphAtlas::from_bytes(&font_bytes(), 32).unwrap();
        assert_eq!((atlas.cell_width(), atlas.cell_height()), (16, 32));
        assert_eq!(atlas.width_at(16), 8);
        let tiny = GlyphAtlas::from_bytes(&font_bytes(), 1).unwrap();
        assert_eq!(tiny.cell_height(), MIN_CELL_HEIGHT);
    }

    #[test]
    fn a_glyph_is_a_mask_one_cell_in_size_and_is_kept() {
        let mut atlas = atlas(16);
        let at = atlas.glyph('@', GlyphStyle::PLAIN);
        let mask = atlas.mask(at);
        assert_eq!((mask.width, mask.height), (8, 16));
        assert!(!mask.blank);
        assert_eq!(atlas.glyph('@', GlyphStyle::PLAIN), at);
        assert_eq!(atlas.len(), 1);
        let space = atlas.glyph(' ', GlyphStyle::PLAIN);
        assert!(atlas.mask(space).blank);
        let line = atlas.glyph('─', GlyphStyle::PLAIN);
        let mask = atlas.mask(line);
        let full_rows = rows(mask).iter().filter(|row| !row.contains('.')).count();
        assert!(full_rows >= 1, "a horizontal line crosses the whole cell");
        let bar = atlas.glyph('│', GlyphStyle::PLAIN);
        let covered_rows = rows(atlas.mask(bar))
            .iter()
            .filter(|row| row.contains('#'))
            .count();
        assert_eq!(covered_rows, 16, "a vertical line runs the whole height");
    }

    #[test]
    fn a_missing_character_is_a_box_and_a_control_character_nothing() {
        let mut atlas = atlas(16);
        assert!(!atlas.has_char('\u{e000}'));
        let missing = atlas.glyph('\u{e000}', GlyphStyle::PLAIN);
        let drawn = rows(atlas.mask(missing));
        assert_eq!(drawn[2], ".######.");
        assert_eq!(drawn[7], ".#....#.");
        assert_eq!(drawn[13], ".######.");
        let control = atlas.glyph('\u{7}', GlyphStyle::PLAIN);
        assert!(atlas.mask(control).blank);
        // Not in the font and not in the fast table either.
        let far = atlas.glyph('\u{10fffd}', GlyphStyle::BOLD);
        assert!(!atlas.mask(far).blank);
    }

    #[test]
    fn styles_are_made_from_the_one_font() {
        let mut atlas = atlas(16);
        let plain = atlas.glyph('l', GlyphStyle::PLAIN);
        let plain_ink: u32 = atlas
            .mask(plain)
            .coverage
            .iter()
            .map(|&c| u32::from(c))
            .sum();
        let bold = atlas.glyph('l', GlyphStyle::BOLD);
        let bold_ink: u32 = atlas
            .mask(bold)
            .coverage
            .iter()
            .map(|&c| u32::from(c))
            .sum();
        assert!(bold_ink > plain_ink, "bold is heavier");

        let space = atlas.glyph(' ', GlyphStyle::UNDERLINE);
        let underlined = rows(atlas.mask(space));
        assert!(!atlas.mask(space).blank, "an underlined space shows");
        assert_eq!(
            underlined.iter().filter(|row| *row == "########").count(),
            1
        );

        let upright_id = atlas.glyph('|', GlyphStyle::PLAIN);
        let upright = rows(atlas.mask(upright_id));
        let italic_id = atlas.glyph('|', GlyphStyle::ITALIC);
        let italic = rows(atlas.mask(italic_id));
        let first = |row: &String| row.find(|c| c != '.');
        assert_ne!(first(&upright[2]), first(&italic[2]), "the top leans");
        let both = atlas.glyph('|', GlyphStyle::ITALIC.with(GlyphStyle::BOLD));
        assert_ne!(both, italic_id);
    }

    #[test]
    fn sprites_get_glyphs_at_their_own_height() {
        let mut atlas = atlas(16);
        let big = atlas.glyph_at('#', GlyphStyle::PLAIN, 32);
        let mask = atlas.mask(big);
        assert_eq!((mask.width, mask.height), (16, 32));
        assert_eq!(atlas.glyph_at('#', GlyphStyle::PLAIN, 32), big);
        assert_ne!(atlas.glyph('#', GlyphStyle::PLAIN), big);
    }

    #[test]
    fn a_new_height_forgets_every_glyph() {
        let mut atlas = atlas(16);
        atlas.glyph('a', GlyphStyle::PLAIN);
        atlas.glyph('─', GlyphStyle::PLAIN);
        atlas.set_cell_height(24);
        assert!(atlas.is_empty());
        let a = atlas.glyph('a', GlyphStyle::PLAIN);
        assert_eq!(atlas.mask(a).height, 24);
    }

    #[test]
    fn what_is_not_a_font_is_refused() {
        let error = GlyphAtlas::from_bytes(b"not a font", 16).unwrap_err();
        assert!(error.contains("not a font"), "{error}");
        let error = GlyphAtlas::open(Path::new("no/such/font.ttf"), 16).unwrap_err();
        assert_eq!(error.path, Path::new("no/such/font.ttf"));
        assert!(
            error
                .to_string()
                .starts_with("the font `no/such/font.ttf` cannot be used: ")
        );
    }

    #[test]
    fn the_cache_is_bounded() {
        let mut atlas = atlas(4);
        for n in 0..=MAX_GLYPHS as u32 {
            let ch = char::from_u32(0x4e00 + n).unwrap();
            let id = atlas.glyph(ch, GlyphStyle::PLAIN);
            assert!(!atlas.mask(id).coverage.is_empty());
        }
        assert_eq!(atlas.len(), 1, "emptied once full, then begun again");
    }
}
