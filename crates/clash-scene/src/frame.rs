//! [`Frame`]: what a frontend is given to show (12 §4.2).
//!
//! A frame is a grid of [`FrameCell`]s, a list of [`Sprite`]s that are not
//! aligned to the grid, and whole-frame effects ([`FrameFx`]). The grid is
//! what a terminal can show; sprites and effects are how a window goes
//! further. Nothing essential may be conveyed only by what a terminal
//! cannot draw.
//!
//! # Drawing
//!
//! A frame is built once and reused: [`Frame::clear`] and
//! [`Frame::resize`] keep its storage, and a cell is plain data, so
//! building a frame allocates nothing per cell. Draw through a
//! [`Canvas`], which clips everything to a rectangle:
//!
//! ```
//! use clash_scene::{Canvas, CellStyle, Frame, Rect};
//!
//! let mut frame = Frame::new(20, 4);
//! let style = CellStyle::default();
//! let mut canvas = Canvas::new(&mut frame);
//! canvas.print(1, 1, "hello", style);
//! canvas.within(Rect::new(0, 0, 3, 4)).print(0, 2, "clipped", style);
//! assert_eq!(frame.row_text(1), " hello              ");
//! assert_eq!(frame.row_text(2), "cli                 ");
//! ```
//!
//! # Coordinates
//!
//! Column 0, row 0 is the top-left cell. A sprite's `x` and `y` are in
//! cells: a sprite at `(3.0, 2.0)` sits exactly on the cell in column 3,
//! row 2.

use bitflags::bitflags;

use crate::color::{Ansi16, Color, Rgb};

bitflags! {
    /// How a cell's character is drawn, beyond its colours.
    #[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Default)]
    pub struct Attrs: u8 {
        /// Heavier strokes.
        const BOLD = 1;
        /// Fainter than usual.
        const DIM = 2;
        /// Slanted.
        const ITALIC = 4;
        /// A line beneath.
        const UNDERLINE = 8;
        /// Foreground and background exchanged.
        const REVERSE = 16;
    }
}

/// Colours and attributes together: everything about a cell but its
/// character.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct CellStyle {
    /// The colour of the character.
    pub fg: Color,
    /// The colour behind it.
    pub bg: Color,
    /// Attributes.
    pub attrs: Attrs,
}

impl Default for CellStyle {
    /// Light grey on black with no attributes.
    fn default() -> CellStyle {
        CellStyle {
            fg: Color::ansi(Ansi16::White),
            bg: Color::ansi(Ansi16::Black),
            attrs: Attrs::empty(),
        }
    }
}

/// One cell of the grid: a character, its colours and its attributes.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct FrameCell {
    /// The character. One Unicode character of single width.
    pub ch: char,
    /// The colour of the character.
    pub fg: Color,
    /// The colour behind it.
    pub bg: Color,
    /// Attributes.
    pub attrs: Attrs,
}

impl FrameCell {
    /// A cell showing `ch` in `style`.
    pub const fn new(ch: char, style: CellStyle) -> FrameCell {
        FrameCell {
            ch,
            fg: style.fg,
            bg: style.bg,
            attrs: style.attrs,
        }
    }

    /// An empty cell in `style`.
    pub const fn blank(style: CellStyle) -> FrameCell {
        FrameCell::new(' ', style)
    }

    /// The cell's colours and attributes.
    pub const fn style(&self) -> CellStyle {
        CellStyle {
            fg: self.fg,
            bg: self.bg,
            attrs: self.attrs,
        }
    }
}

impl Default for FrameCell {
    fn default() -> FrameCell {
        FrameCell::blank(CellStyle::default())
    }
}

/// A glyph not aligned to the grid: something gliding between cells, a
/// flying fragment, floating text, an afterimage.
///
/// The terminal frontend draws each sprite in the nearest cell and ignores
/// `scale` and `alpha`.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Sprite {
    /// The character.
    pub ch: char,
    /// Its colour.
    pub fg: Color,
    /// Horizontal position in cells; whole numbers are cell columns.
    pub x: f32,
    /// Vertical position in cells; whole numbers are cell rows.
    pub y: f32,
    /// Size relative to a grid cell. 1 is the size of a cell.
    pub scale: f32,
    /// Opacity, 0 to 1.
    pub alpha: f32,
    /// Drawing order: higher layers are drawn over lower ones, and within
    /// a layer later sprites over earlier ones.
    pub layer: u8,
}

impl Sprite {
    /// A sprite of ordinary size and full opacity on layer 0.
    pub const fn new(ch: char, fg: Color, x: f32, y: f32) -> Sprite {
        Sprite {
            ch,
            fg,
            x,
            y,
            scale: 1.0,
            alpha: 1.0,
            layer: 0,
        }
    }

    /// The cell nearest to the sprite, which may lie outside the grid.
    pub fn cell(&self) -> (i32, i32) {
        ((self.x + 0.5).floor() as i32, (self.y + 0.5).floor() as i32)
    }
}

/// A colour laid over the whole frame at some strength.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Wash {
    /// The colour.
    pub color: Rgb,
    /// How strongly it shows, 0 to 1.
    pub strength: f32,
}

/// Effects on the whole frame. A terminal ignores all of them.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct FrameFx {
    /// Screen shake, 0 (none) to 1 (violent).
    pub shake: f32,
    /// A flash of colour over everything.
    pub flash: Option<Wash>,
    /// Magnification about the centre. 1 is none.
    pub zoom: f32,
    /// Letterbox bars: the share of the height covered at top and bottom
    /// together, 0 to 1.
    pub bars: f32,
    /// A lasting tint over everything.
    pub tint: Option<Wash>,
}

impl Default for FrameFx {
    fn default() -> FrameFx {
        FrameFx {
            shake: 0.0,
            flash: None,
            zoom: 1.0,
            bars: 0.0,
            tint: None,
        }
    }
}

/// A rectangle of cells. `w` and `h` are never negative.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Default)]
pub struct Rect {
    /// Leftmost column.
    pub x: i32,
    /// Topmost row.
    pub y: i32,
    /// Width in cells.
    pub w: i32,
    /// Height in cells.
    pub h: i32,
}

impl Rect {
    /// A rectangle. A negative width or height becomes zero.
    pub const fn new(x: i32, y: i32, w: i32, h: i32) -> Rect {
        Rect {
            x,
            y,
            w: if w < 0 { 0 } else { w },
            h: if h < 0 { 0 } else { h },
        }
    }

    /// Whether it holds no cells.
    pub const fn is_empty(self) -> bool {
        self.w == 0 || self.h == 0
    }

    /// The column just past the right edge.
    pub const fn right(self) -> i32 {
        self.x + self.w
    }

    /// The row just past the bottom edge.
    pub const fn bottom(self) -> i32 {
        self.y + self.h
    }

    /// Whether the cell is inside.
    pub const fn contains(self, x: i32, y: i32) -> bool {
        x >= self.x && x < self.right() && y >= self.y && y < self.bottom()
    }

    /// The part common to both.
    pub fn intersect(self, other: Rect) -> Rect {
        let x = self.x.max(other.x);
        let y = self.y.max(other.y);
        let right = self.right().min(other.right());
        let bottom = self.bottom().min(other.bottom());
        Rect::new(x, y, right - x, bottom - y)
    }

    /// The rectangle with `by` cells taken off every side.
    pub const fn inset(self, by: i32) -> Rect {
        Rect::new(self.x + by, self.y + by, self.w - 2 * by, self.h - 2 * by)
    }
}

/// What a frontend is given to show.
#[derive(Clone, PartialEq, Debug)]
pub struct Frame {
    /// Width of the grid in cells.
    pub cols: u16,
    /// Height of the grid in cells.
    pub rows: u16,
    /// The grid, row by row from the top: `cols * rows` cells.
    pub cells: Vec<FrameCell>,
    /// Glyphs not aligned to the grid.
    pub sprites: Vec<Sprite>,
    /// Effects on the whole frame.
    pub fx: FrameFx,
}

impl Frame {
    /// A frame of empty cells.
    pub fn new(cols: u16, rows: u16) -> Frame {
        Frame {
            cols,
            rows,
            cells: vec![FrameCell::default(); usize::from(cols) * usize::from(rows)],
            sprites: Vec::new(),
            fx: FrameFx::default(),
        }
    }

    /// Changes the size of the grid, keeping its storage where it can.
    /// What the cells hold afterwards is unspecified: clear the frame.
    pub fn resize(&mut self, cols: u16, rows: u16) {
        self.cols = cols;
        self.rows = rows;
        self.cells
            .resize(usize::from(cols) * usize::from(rows), FrameCell::default());
    }

    /// Makes every cell `cell`, removes the sprites and resets the effects.
    /// Allocates nothing.
    pub fn clear(&mut self, cell: FrameCell) {
        self.cells.fill(cell);
        self.sprites.clear();
        self.fx = FrameFx::default();
    }

    /// The whole grid as a rectangle.
    pub fn area(&self) -> Rect {
        Rect::new(0, 0, i32::from(self.cols), i32::from(self.rows))
    }

    fn index(&self, x: i32, y: i32) -> Option<usize> {
        if x < 0 || y < 0 || x >= i32::from(self.cols) || y >= i32::from(self.rows) {
            return None;
        }
        Some(y as usize * usize::from(self.cols) + x as usize)
    }

    /// The cell at a column and row, if it is on the grid.
    pub fn get(&self, x: i32, y: i32) -> Option<&FrameCell> {
        self.index(x, y).map(|i| &self.cells[i])
    }

    /// The cell at a column and row, to change it, if it is on the grid.
    pub fn get_mut(&mut self, x: i32, y: i32) -> Option<&mut FrameCell> {
        self.index(x, y).map(|i| &mut self.cells[i])
    }

    /// Replaces a cell. Off the grid, nothing happens.
    pub fn set(&mut self, x: i32, y: i32, cell: FrameCell) {
        if let Some(i) = self.index(x, y) {
            self.cells[i] = cell;
        }
    }

    /// The characters of one row, for tests and messages.
    pub fn row_text(&self, row: u16) -> String {
        let cols = usize::from(self.cols);
        let start = usize::from(row) * cols;
        self.cells
            .get(start..start + cols)
            .map(|cells| cells.iter().map(|c| c.ch).collect())
            .unwrap_or_default()
    }

    /// The characters of the whole grid, one line per row.
    pub fn text(&self) -> String {
        let mut out = String::with_capacity((usize::from(self.cols) + 1) * usize::from(self.rows));
        for row in 0..self.rows {
            out.push_str(&self.row_text(row));
            out.push('\n');
        }
        out
    }

    /// Draws every sprite into the grid at its nearest cell, lowest layer
    /// first, as a frontend without sub-cell motion shows them. The
    /// sprite's character and colour replace the cell's; the background
    /// stays. Sprites off the grid are dropped. The sprite list is left
    /// empty.
    pub fn snap_sprites(&mut self) {
        let mut sprites = core::mem::take(&mut self.sprites);
        // A stable sort: within a layer, later sprites stay on top.
        sprites.sort_by_key(|s| s.layer);
        for sprite in &sprites {
            let (x, y) = sprite.cell();
            if let Some(cell) = self.get_mut(x, y) {
                cell.ch = sprite.ch;
                cell.fg = sprite.fg;
            }
        }
        sprites.clear();
        // Hand the storage back so the next frame does not allocate.
        self.sprites = sprites;
    }
}

/// The six characters a border is drawn with (a theme's `borders`).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Borders {
    /// Horizontal edge.
    pub h: char,
    /// Vertical edge.
    pub v: char,
    /// Top-left corner.
    pub tl: char,
    /// Top-right corner.
    pub tr: char,
    /// Bottom-left corner.
    pub bl: char,
    /// Bottom-right corner.
    pub br: char,
}

impl Default for Borders {
    fn default() -> Borders {
        Borders {
            h: '─',
            v: '│',
            tl: '┌',
            tr: '┐',
            bl: '└',
            br: '┘',
        }
    }
}

/// A frame to draw on and the rectangle drawing is confined to.
///
/// Coordinates given to a canvas are always those of the frame. Whatever
/// falls outside the clip rectangle is dropped without error.
#[derive(Debug)]
pub struct Canvas<'a> {
    frame: &'a mut Frame,
    clip: Rect,
}

impl<'a> Canvas<'a> {
    /// A canvas over the whole frame.
    pub fn new(frame: &'a mut Frame) -> Canvas<'a> {
        let clip = frame.area();
        Canvas { frame, clip }
    }

    /// The rectangle drawing is confined to.
    pub fn clip(&self) -> Rect {
        self.clip
    }

    /// The frame's sprite list, to add to it.
    pub fn sprites(&mut self) -> &mut Vec<Sprite> {
        &mut self.frame.sprites
    }

    /// A canvas on the same frame confined to `rect` as well.
    pub fn within(&mut self, rect: Rect) -> Canvas<'_> {
        Canvas {
            clip: self.clip.intersect(rect),
            frame: self.frame,
        }
    }

    /// The cell at a column and row, if it is inside the clip rectangle.
    pub fn cell(&self, x: i32, y: i32) -> Option<&FrameCell> {
        if self.clip.contains(x, y) {
            self.frame.get(x, y)
        } else {
            None
        }
    }

    /// Replaces one cell.
    pub fn put(&mut self, x: i32, y: i32, ch: char, style: CellStyle) {
        if self.clip.contains(x, y) {
            self.frame.set(x, y, FrameCell::new(ch, style));
        }
    }

    /// Changes one cell in place.
    pub fn edit(&mut self, x: i32, y: i32, change: impl FnOnce(&mut FrameCell)) {
        if self.clip.contains(x, y)
            && let Some(cell) = self.frame.get_mut(x, y)
        {
            change(cell);
        }
    }

    /// Writes `text` rightward from a cell, one character per cell, and
    /// returns how many cells it would cover unclipped.
    pub fn print(&mut self, x: i32, y: i32, text: &str, style: CellStyle) -> i32 {
        let mut column = x;
        for ch in text.chars() {
            self.put(column, y, ch, style);
            column += 1;
        }
        column - x
    }

    /// Fills a rectangle with one character.
    pub fn fill(&mut self, rect: Rect, ch: char, style: CellStyle) {
        let rect = rect.intersect(self.clip);
        let cell = FrameCell::new(ch, style);
        for y in rect.y..rect.bottom() {
            for x in rect.x..rect.right() {
                self.frame.set(x, y, cell);
            }
        }
    }

    /// Draws a border just inside the edge of `rect`. A rectangle less
    /// than two cells wide or high gets none.
    pub fn border(&mut self, rect: Rect, borders: &Borders, style: CellStyle) {
        if rect.w < 2 || rect.h < 2 {
            return;
        }
        let (left, top, right, bottom) = (rect.x, rect.y, rect.right() - 1, rect.bottom() - 1);
        for x in left + 1..right {
            self.put(x, top, borders.h, style);
            self.put(x, bottom, borders.h, style);
        }
        for y in top + 1..bottom {
            self.put(left, y, borders.v, style);
            self.put(right, y, borders.v, style);
        }
        self.put(left, top, borders.tl, style);
        self.put(right, top, borders.tr, style);
        self.put(left, bottom, borders.bl, style);
        self.put(right, bottom, borders.br, style);
    }
}

/// Whether a character fills exactly one cell: not a control character,
/// not a combining mark, and not one of the double-width characters of
/// East Asian scripts or emoji.
///
/// Glyphs are single-width (12 §16, question 3). This is a table of the
/// main blocks, not the whole of Unicode's width data.
pub fn is_single_width(ch: char) -> bool {
    const ZERO_OR_CONTROL: &[(u32, u32)] = &[
        (0x0000, 0x001f),
        (0x007f, 0x009f),
        (0x0300, 0x036f),
        (0x200b, 0x200f),
        (0x2028, 0x202e),
        (0x2060, 0x2064),
        (0xfe00, 0xfe0f),
        (0xfeff, 0xfeff),
    ];
    const WIDE: &[(u32, u32)] = &[
        (0x1100, 0x115f),
        (0x2e80, 0x303e),
        (0x3041, 0x33ff),
        (0x3400, 0x4dbf),
        (0x4e00, 0x9fff),
        (0xa000, 0xa4cf),
        (0xac00, 0xd7a3),
        (0xf900, 0xfaff),
        (0xfe30, 0xfe4f),
        (0xff00, 0xff60),
        (0xffe0, 0xffe6),
        (0x1f300, 0x1f64f),
        (0x1f900, 0x1f9ff),
        (0x20000, 0x3fffd),
    ];
    let code = ch as u32;
    let within = |ranges: &[(u32, u32)]| ranges.iter().any(|(lo, hi)| (*lo..=*hi).contains(&code));
    !within(ZERO_OR_CONTROL) && !within(WIDE)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn drawing_off_the_grid_or_outside_the_clip_is_dropped() {
        let mut frame = Frame::new(4, 2);
        let style = CellStyle::default();
        let mut canvas = Canvas::new(&mut frame);
        assert_eq!(canvas.print(2, 0, "abcdef", style), 6);
        canvas.put(-1, 0, 'x', style);
        canvas.put(0, 9, 'x', style);
        canvas
            .within(Rect::new(0, 1, 2, 1))
            .fill(Rect::new(-5, -5, 50, 50), '#', style);
        assert_eq!(frame.text(), "  ab\n##  \n");
    }

    #[test]
    fn clearing_and_resizing_reuse_the_storage() {
        let mut frame = Frame::new(10, 5);
        let capacity = frame.cells.capacity();
        frame
            .sprites
            .push(Sprite::new('*', Color::default(), 1.0, 1.0));
        frame.fx.shake = 0.5;
        frame.resize(5, 4);
        frame.clear(FrameCell::default());
        assert_eq!(frame.cells.len(), 20);
        assert_eq!(frame.cells.capacity(), capacity);
        assert!(frame.sprites.is_empty());
        assert_eq!(frame.fx, FrameFx::default());
    }

    #[test]
    fn sprites_snap_to_the_nearest_cell_by_layer() {
        let mut frame = Frame::new(4, 2);
        let red = Color::ansi(Ansi16::Red);
        let blue = Color::ansi(Ansi16::Blue);
        frame.sprites.push(Sprite {
            layer: 2,
            ..Sprite::new('a', red, 1.4, 0.4)
        });
        frame.sprites.push(Sprite::new('b', blue, 0.6, -0.4));
        frame.sprites.push(Sprite::new('c', blue, 2.5, 0.5));
        frame.sprites.push(Sprite::new('d', blue, 9.0, 9.0));
        frame.sprites.push(Sprite::new('e', blue, -0.6, 0.0));
        frame.snap_sprites();
        assert_eq!(frame.text(), " a  \n   c\n");
        assert_eq!(frame.get(1, 0).unwrap().fg, red);
        assert!(frame.sprites.is_empty());
    }

    #[test]
    fn a_border_goes_just_inside_its_rectangle() {
        let mut frame = Frame::new(5, 3);
        Canvas::new(&mut frame).border(
            Rect::new(0, 0, 5, 3),
            &Borders::default(),
            CellStyle::default(),
        );
        assert_eq!(frame.text(), "┌───┐\n│   │\n└───┘\n");
    }

    #[test]
    fn rectangles_intersect_and_inset() {
        let a = Rect::new(0, 0, 10, 10);
        assert_eq!(a.intersect(Rect::new(5, 5, 10, 10)), Rect::new(5, 5, 5, 5));
        assert!(a.intersect(Rect::new(20, 20, 3, 3)).is_empty());
        assert_eq!(a.inset(1), Rect::new(1, 1, 8, 8));
        assert_eq!(Rect::new(0, 0, 1, 1).inset(1), Rect::new(1, 1, 0, 0));
        assert!(a.contains(9, 9) && !a.contains(10, 9));
    }

    #[test]
    fn width_check_rejects_wide_and_invisible_characters() {
        for ok in ['a', '#', '─', '█', '░', 'é', '☺', '▲'] {
            assert!(is_single_width(ok), "{ok}");
        }
        for bad in ['\n', '\u{0301}', '漢', '한', '\u{1f600}', '\u{200b}', 'Ａ'] {
            assert!(!is_single_width(bad), "{bad:?}");
        }
    }
}
