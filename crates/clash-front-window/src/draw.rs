//! The renderer: a [`Frame`] drawn into a [`PixelBuffer`] (12 §11).
//!
//! # Order
//!
//! 1. Each cell's background, and its glyph over it in its foreground,
//!    in one pass: a glyph's mask is one cell in size, so each pixel of
//!    the grid is written once.
//! 2. The sprites, lowest layer first and within a layer in the order
//!    given, centred on their position in cells, at their scale and
//!    opacity, over whatever is beneath.
//! 3. The frame's effects: the tint and the flash wash every colour
//!    toward theirs; the shake moves the whole picture by a few pixels,
//!    in a pattern that turns with each frame drawn; the zoom magnifies
//!    the picture about the centre of the grid (12 §6.3, the camera's
//!    place); the letterbox bars cover the top and the bottom.
//!
//! # Placing the grid
//!
//! The grid is centred in the picture, with what is left over at the
//! edges drawn in the backdrop colour. [`Layout`] says where each cell
//! is, and which cell is under a point, which is what the mouse needs.
//!
//! # Cost
//!
//! Drawing is integer arithmetic on whole pixels, the same on every
//! run. A cell's pixels are blended from its two colours by its glyph's
//! coverage with no branch per pixel. Nothing is allocated once the
//! atlas holds the frame's glyphs and the renderer's buffers have grown
//! to the picture's size.

use clash_scene::{Attrs, Frame, Rgb, Sprite, Wash};

use crate::atlas::{GlyphAtlas, GlyphMask, GlyphStyle, MAX_CELL_HEIGHT};
use crate::pixels::{PixelBuffer, pixel};

/// How far a dim character's colour is moved toward its background, in
/// thousandths.
const DIM_PERMILLE: u16 = 450;

/// How far the screen moves at a shake of 1, as thousandths of a cell
/// in each direction.
const SHAKE_PERMILLE_OF_CELL: u32 = 500;

/// The directions the picture is moved in by a shake, one per frame
/// drawn, in turn.
const SHAKE_PATTERN: [(i32, i32); 8] = [
    (1, 1),
    (-1, 0),
    (0, -1),
    (-1, 1),
    (1, 0),
    (0, 1),
    (1, -1),
    (-1, -1),
];

/// The smallest and largest zoom drawn; a frame asking for more is
/// drawn at these.
const ZOOM_RANGE: (f32, f32) = (0.25, 8.0);

/// How near 1 a zoom must be to be drawn as none.
const ZOOM_NONE: f32 = 0.001;

/// The smallest and largest scale a sprite is drawn at.
const SPRITE_SCALE_RANGE: (f32, f32) = (0.0625, 16.0);

/// The colour of the letterbox bars.
const BAR_COLOR: Rgb = Rgb::BLACK;

/// Full opacity on the scale blending works in: coverage 255 is 256.
const OPAQUE: u32 = 256;

/// No pixel: a zoom that maps outside the picture.
const OUTSIDE: u32 = u32::MAX;

/// Where the grid of a frame lies in a picture: the size of a cell and
/// the top-left corner of the grid, in pixels, before any shake or zoom.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Default)]
pub struct Layout {
    /// Columns of the grid.
    pub cols: u16,
    /// Rows of the grid.
    pub rows: u16,
    /// The width of a cell in pixels.
    pub cell_width: u32,
    /// The height of a cell in pixels.
    pub cell_height: u32,
    /// The left edge of the grid.
    pub left: i32,
    /// The top edge of the grid.
    pub top: i32,
}

impl Layout {
    /// The grid of a frame `cols` by `rows` in a picture `width` by
    /// `height` pixels, with the atlas's cells: centred, or at the top
    /// left where it does not fit.
    pub fn new(cols: u16, rows: u16, atlas: &GlyphAtlas, width: u32, height: u32) -> Layout {
        let (cell_width, cell_height) = (atlas.cell_width(), atlas.cell_height());
        let grid_width = u64::from(cols) * u64::from(cell_width);
        let grid_height = u64::from(rows) * u64::from(cell_height);
        let margin = |room: u32, used: u64| (u64::from(room).saturating_sub(used) / 2) as i32;
        Layout {
            cols,
            rows,
            cell_width,
            cell_height,
            left: margin(width, grid_width),
            top: margin(height, grid_height),
        }
    }

    /// The pixel rectangle of a cell: left, top, width and height.
    pub fn cell_rect(&self, col: u16, row: u16) -> (i32, i32, u32, u32) {
        (
            self.left + i32::from(col) * self.cell_width as i32,
            self.top + i32::from(row) * self.cell_height as i32,
            self.cell_width,
            self.cell_height,
        )
    }

    /// The centre of the grid, in pixels: what a zoom magnifies about.
    pub fn centre(&self) -> (f64, f64) {
        (
            f64::from(self.left) + f64::from(self.cols) * f64::from(self.cell_width) / 2.0,
            f64::from(self.top) + f64::from(self.rows) * f64::from(self.cell_height) / 2.0,
        )
    }

    /// The cell under a point of the picture drawn with `zoom`, or
    /// `None` off the grid. The point is in pixels from the top left.
    pub fn cell_under(&self, x: f64, y: f64, zoom: f32) -> Option<(u16, u16)> {
        let zoom = f64::from(effective_zoom(zoom).unwrap_or(1.0));
        let (cx, cy) = self.centre();
        let (x, y) = (cx + (x - cx) / zoom, cy + (y - cy) / zoom);
        let col = ((x - f64::from(self.left)) / f64::from(self.cell_width.max(1))).floor();
        let row = ((y - f64::from(self.top)) / f64::from(self.cell_height.max(1))).floor();
        let inside = |at: f64, count: u16| at >= 0.0 && at < f64::from(count);
        (inside(col, self.cols) && inside(row, self.rows)).then_some((col as u16, row as u16))
    }
}

/// How many cells of an atlas fit in a picture `width` by `height`
/// pixels: the grid a window of that size shows. At least one each way.
pub fn grid_size(width: u32, height: u32, atlas: &GlyphAtlas) -> (u16, u16) {
    let fit = |room: u32, cell: u32| (room / cell.max(1)).clamp(1, u32::from(u16::MAX)) as u16;
    (
        fit(width, atlas.cell_width()),
        fit(height, atlas.cell_height()),
    )
}

/// The zoom a frame's effect asks for, if it is drawn at all.
fn effective_zoom(zoom: f32) -> Option<f32> {
    if !zoom.is_finite() || (zoom - 1.0).abs() < ZOOM_NONE {
        return None;
    }
    Some(zoom.clamp(ZOOM_RANGE.0, ZOOM_RANGE.1))
}

/// A share from 0 to 1 as thousandths; anything not a number is none.
fn permille(share: f32) -> u16 {
    if share.is_finite() {
        (share.clamp(0.0, 1.0) * 1000.0).round() as u16
    } else {
        0
    }
}

/// A coverage byte on the scale blending works in, 0 to 256.
#[inline(always)]
fn weight(coverage: u8) -> u32 {
    let coverage = u32::from(coverage);
    coverage + (coverage >> 7)
}

/// `over` laid on `under` at `weight` out of 256, two channels at a
/// time.
#[inline(always)]
fn blend(under: u32, over: u32, weight: u32) -> u32 {
    let keep = OPAQUE - weight;
    let red_blue =
        (((under & 0x00ff_00ff) * keep + (over & 0x00ff_00ff) * weight) >> 8) & 0x00ff_00ff;
    let green = (((under & 0x0000_ff00) * keep + (over & 0x0000_ff00) * weight) >> 8) & 0x0000_ff00;
    red_blue | green
}

/// The washes laid over every colour: the tint, then the flash.
#[derive(Clone, Copy, Debug, Default)]
struct Washes {
    layers: [(Rgb, u16); 2],
    count: usize,
}

impl Washes {
    fn of(tint: Option<Wash>, flash: Option<Wash>) -> Washes {
        let mut washes = Washes::default();
        for wash in [tint, flash].into_iter().flatten() {
            let strength = permille(wash.strength);
            if strength > 0 {
                washes.layers[washes.count] = (wash.color, strength);
                washes.count += 1;
            }
        }
        washes
    }

    fn apply(&self, rgb: Rgb) -> Rgb {
        self.layers[..self.count]
            .iter()
            .fold(rgb, |rgb, &(color, strength)| rgb.toward(color, strength))
    }
}

/// A rectangle of the picture a blit covers, clipped to it: the first
/// pixel inside, how far into the source it is, and the size.
#[derive(Clone, Copy, Debug)]
struct Clip {
    x: usize,
    y: usize,
    from_x: usize,
    from_y: usize,
    width: usize,
    height: usize,
}

/// Clips a source of `width` by `height` placed at `(x, y)` to a
/// picture, or `None` if nothing of it shows.
fn clip(x: i64, y: i64, width: u32, height: u32, picture: (u32, u32)) -> Option<Clip> {
    let (pw, ph) = (i64::from(picture.0), i64::from(picture.1));
    let (x0, y0) = (x.max(0), y.max(0));
    let (x1, y1) = (
        x.saturating_add(i64::from(width)).min(pw),
        y.saturating_add(i64::from(height)).min(ph),
    );
    (x1 > x0 && y1 > y0).then(|| Clip {
        x: x0 as usize,
        y: y0 as usize,
        from_x: (x0 - x) as usize,
        from_y: (y0 - y) as usize,
        width: (x1 - x0) as usize,
        height: (y1 - y0) as usize,
    })
}

/// Draws frames into pixel buffers. Keeps what it needs from one frame
/// to the next: the turn of the shake, and the buffers the sprites'
/// order and the zoom are worked out in. See the [module](self)
/// documentation.
#[derive(Clone, Debug)]
pub struct Renderer {
    backdrop: Rgb,
    phase: usize,
    order: Vec<u32>,
    scratch: PixelBuffer,
    map_x: Vec<u32>,
    map_y: Vec<u32>,
}

impl Default for Renderer {
    fn default() -> Renderer {
        Renderer::new(Rgb::BLACK)
    }
}

impl Renderer {
    /// A renderer that draws what lies outside the grid in `backdrop`.
    pub fn new(backdrop: Rgb) -> Renderer {
        Renderer {
            backdrop,
            phase: 0,
            order: Vec::new(),
            scratch: PixelBuffer::default(),
            map_x: Vec::new(),
            map_y: Vec::new(),
        }
    }

    /// The colour outside the grid.
    pub fn backdrop(&self) -> Rgb {
        self.backdrop
    }

    /// Changes the colour outside the grid.
    pub fn set_backdrop(&mut self, backdrop: Rgb) {
        self.backdrop = backdrop;
    }

    /// Which step of the shake's pattern the next frame drawn takes. A
    /// new renderer begins at 0; each frame drawn moves it on by one.
    pub fn set_shake_phase(&mut self, phase: usize) {
        self.phase = phase % SHAKE_PATTERN.len();
    }

    /// Draws a frame into `target`, filling all of it. The grid is laid
    /// out by [`Layout::new`] for the target's size.
    pub fn draw(&mut self, frame: &Frame, atlas: &mut GlyphAtlas, target: &mut PixelBuffer) {
        let (width, height) = (target.width(), target.height());
        let layout = Layout::new(frame.cols, frame.rows, atlas, width, height);
        let washes = Washes::of(frame.fx.tint, frame.fx.flash);
        let backdrop = pixel(washes.apply(self.backdrop));
        let (dx, dy) = self.shake_offset(frame.fx.shake, &layout);
        self.phase = (self.phase + 1) % SHAKE_PATTERN.len();
        let zoom = effective_zoom(frame.fx.zoom);

        // With a zoom the picture is drawn aside and magnified into the
        // target; without one, straight into it.
        let mut scratch = core::mem::take(&mut self.scratch);
        let canvas = if zoom.is_some() {
            scratch.resize(width, height);
            &mut scratch
        } else {
            &mut *target
        };
        let grid = (layout.left + dx, layout.top + dy);
        fill_outside_grid(canvas, &layout, grid, backdrop);
        draw_cells(canvas, frame, atlas, &layout, grid, &washes);
        self.draw_sprites(canvas, &frame.sprites, atlas, &layout, grid, &washes);
        if let Some(zoom) = zoom {
            self.magnify(&scratch, target, &layout, zoom, backdrop);
        }
        self.scratch = scratch;
        draw_bars(target, frame.fx.bars);
    }

    /// How far the shake moves the picture in this frame, in pixels.
    fn shake_offset(&self, shake: f32, layout: &Layout) -> (i32, i32) {
        let strength = u32::from(permille(shake));
        let reach = |cell: u32| (cell * strength * SHAKE_PERMILLE_OF_CELL / 1_000_000) as i32;
        let (sx, sy) = SHAKE_PATTERN[self.phase % SHAKE_PATTERN.len()];
        (
            sx * reach(layout.cell_width),
            sy * reach(layout.cell_height),
        )
    }

    /// Draws the sprites, lowest layer first and in the order given
    /// within a layer.
    fn draw_sprites(
        &mut self,
        canvas: &mut PixelBuffer,
        sprites: &[Sprite],
        atlas: &mut GlyphAtlas,
        layout: &Layout,
        grid: (i32, i32),
        washes: &Washes,
    ) {
        if sprites.is_empty() {
            return;
        }
        // A counting sort by layer: stable, and allocates nothing once
        // the order has room for the sprites.
        let mut starts = [0_u32; 257];
        for sprite in sprites {
            starts[usize::from(sprite.layer) + 1] += 1;
        }
        for layer in 1..starts.len() {
            starts[layer] += starts[layer - 1];
        }
        self.order.clear();
        self.order.resize(sprites.len(), 0);
        for (index, sprite) in sprites.iter().enumerate() {
            let slot = &mut starts[usize::from(sprite.layer)];
            self.order[*slot as usize] = index as u32;
            *slot += 1;
        }
        for &index in &self.order {
            draw_sprite(
                canvas,
                &sprites[index as usize],
                atlas,
                layout,
                grid,
                washes,
            );
        }
    }

    /// Magnifies `source` into `target` about the centre of the grid.
    fn magnify(
        &mut self,
        source: &PixelBuffer,
        target: &mut PixelBuffer,
        layout: &Layout,
        zoom: f32,
        backdrop: u32,
    ) {
        let (width, height) = (target.width() as usize, target.height() as usize);
        let (cx, cy) = layout.centre();
        let zoom = f64::from(zoom);
        let map = |into: &mut Vec<u32>, count: usize, centre: f64| {
            into.clear();
            into.extend((0..count).map(|at| {
                let from = (centre + (at as f64 + 0.5 - centre) / zoom).floor();
                if from >= 0.0 && from < count as f64 {
                    from as u32
                } else {
                    OUTSIDE
                }
            }));
        };
        map(&mut self.map_x, width, cx);
        map(&mut self.map_y, height, cy);
        // What maps inside is one run of columns: a zoom keeps order.
        let first = self.map_x.iter().position(|&x| x != OUTSIDE);
        let last = self.map_x.iter().rposition(|&x| x != OUTSIDE);
        let pixels = target.pixels_mut();
        let from = source.pixels();
        for (y, &from_y) in self.map_y.iter().enumerate() {
            let row = &mut pixels[y * width..(y + 1) * width];
            let (Some(first), Some(last)) = (first, last) else {
                row.fill(backdrop);
                continue;
            };
            if from_y == OUTSIDE {
                row.fill(backdrop);
                continue;
            }
            let source_row = &from[from_y as usize * width..(from_y as usize + 1) * width];
            row[..first].fill(backdrop);
            row[last + 1..].fill(backdrop);
            for (out, &from_x) in row[first..=last].iter_mut().zip(&self.map_x[first..=last]) {
                *out = source_row[from_x as usize];
            }
        }
    }
}

/// Draws a frame into a pixel buffer with a renderer of its own: for a
/// single picture. A frontend drawing frame after frame keeps a
/// [`Renderer`], which keeps its buffers and turns the shake.
pub fn draw(frame: &Frame, atlas: &mut GlyphAtlas, target: &mut PixelBuffer) {
    Renderer::default().draw(frame, atlas, target);
}

/// Fills what lies outside the grid's rectangle.
fn fill_outside_grid(canvas: &mut PixelBuffer, layout: &Layout, grid: (i32, i32), backdrop: u32) {
    let (width, height) = (canvas.width() as i64, canvas.height() as i64);
    let grid_width = i64::from(layout.cols) * i64::from(layout.cell_width);
    let grid_height = i64::from(layout.rows) * i64::from(layout.cell_height);
    let (left, top) = (
        i64::from(grid.0).clamp(0, width),
        i64::from(grid.1).clamp(0, height),
    );
    let right = (i64::from(grid.0) + grid_width).clamp(left, width);
    let bottom = (i64::from(grid.1) + grid_height).clamp(top, height);
    let (width, left, right) = (width as usize, left as usize, right as usize);
    let pixels = canvas.pixels_mut();
    pixels[..top as usize * width].fill(backdrop);
    pixels[bottom as usize * width..].fill(backdrop);
    for y in top as usize..bottom as usize {
        let row = &mut pixels[y * width..(y + 1) * width];
        row[..left].fill(backdrop);
        row[right..].fill(backdrop);
    }
}

/// Draws every cell: its background, and its glyph over it.
fn draw_cells(
    canvas: &mut PixelBuffer,
    frame: &Frame,
    atlas: &mut GlyphAtlas,
    layout: &Layout,
    grid: (i32, i32),
    washes: &Washes,
) {
    let picture = (canvas.width(), canvas.height());
    let stride = canvas.width() as usize;
    let (cell_width, cell_height) = (layout.cell_width, layout.cell_height);
    let pixels = canvas.pixels_mut();
    let cols = usize::from(frame.cols);
    for (index, cell) in frame
        .cells
        .iter()
        .enumerate()
        .take(cols * usize::from(frame.rows))
    {
        let (col, row) = ((index % cols) as i64, (index / cols) as i64);
        let x = i64::from(grid.0) + col * i64::from(cell_width);
        let y = i64::from(grid.1) + row * i64::from(cell_height);
        let Some(clip) = clip(x, y, cell_width, cell_height, picture) else {
            continue;
        };
        let (mut fg, mut bg) = (cell.fg.rgb, cell.bg.rgb);
        if cell.attrs.contains(Attrs::REVERSE) {
            core::mem::swap(&mut fg, &mut bg);
        }
        if cell.attrs.contains(Attrs::DIM) {
            fg = fg.toward(bg, DIM_PERMILLE);
        }
        let (fg, bg) = (pixel(washes.apply(fg)), pixel(washes.apply(bg)));
        let glyph = atlas.glyph(cell.ch, GlyphStyle::from_attrs(cell.attrs));
        let mask = atlas.mask(glyph);
        if mask.blank || mask.width != cell_width || mask.height != cell_height {
            for line in 0..clip.height {
                let start = (clip.y + line) * stride + clip.x;
                pixels[start..start + clip.width].fill(bg);
            }
            continue;
        }
        let mask_width = mask.width as usize;
        for line in 0..clip.height {
            let start = (clip.y + line) * stride + clip.x;
            let from = (clip.from_y + line) * mask_width + clip.from_x;
            let coverage = &mask.coverage[from..from + clip.width];
            for (out, &c) in pixels[start..start + clip.width].iter_mut().zip(coverage) {
                *out = blend(bg, fg, weight(c));
            }
        }
    }
}

/// Draws one sprite over what is beneath.
fn draw_sprite(
    canvas: &mut PixelBuffer,
    sprite: &Sprite,
    atlas: &mut GlyphAtlas,
    layout: &Layout,
    grid: (i32, i32),
    washes: &Washes,
) {
    let numbers = [sprite.x, sprite.y, sprite.scale, sprite.alpha];
    if numbers.iter().any(|n| !n.is_finite()) || sprite.alpha <= 0.0 || sprite.scale <= 0.0 {
        return;
    }
    let opacity = (sprite.alpha.min(1.0) * OPAQUE as f32).round() as u32;
    let scale = sprite
        .scale
        .clamp(SPRITE_SCALE_RANGE.0, SPRITE_SCALE_RANGE.1);
    let height = ((layout.cell_height as f32 * scale).round() as u32).clamp(1, MAX_CELL_HEIGHT);
    let glyph = if height == layout.cell_height {
        atlas.glyph(sprite.ch, GlyphStyle::PLAIN)
    } else {
        atlas.glyph_at(sprite.ch, GlyphStyle::PLAIN, height)
    };
    let mask = atlas.mask(glyph);
    if mask.blank || opacity == 0 {
        return;
    }
    // Centred where a cell at the sprite's position would be centred.
    let centre_x = f64::from(grid.0) + (f64::from(sprite.x) + 0.5) * f64::from(layout.cell_width);
    let centre_y = f64::from(grid.1) + (f64::from(sprite.y) + 0.5) * f64::from(layout.cell_height);
    let x = (centre_x - f64::from(mask.width) / 2.0).round() as i64;
    let y = (centre_y - f64::from(mask.height) / 2.0).round() as i64;
    let color = pixel(washes.apply(sprite.fg.rgb));
    blit_over(canvas, &mask, x, y, color, opacity);
}

/// Lays a glyph's coverage in one colour over the picture at an
/// opacity out of 256.
fn blit_over(
    canvas: &mut PixelBuffer,
    mask: &GlyphMask<'_>,
    x: i64,
    y: i64,
    color: u32,
    opacity: u32,
) {
    let Some(clip) = clip(
        x,
        y,
        mask.width,
        mask.height,
        (canvas.width(), canvas.height()),
    ) else {
        return;
    };
    let stride = canvas.width() as usize;
    let mask_width = mask.width as usize;
    let pixels = canvas.pixels_mut();
    for line in 0..clip.height {
        let start = (clip.y + line) * stride + clip.x;
        let from = (clip.from_y + line) * mask_width + clip.from_x;
        let coverage = &mask.coverage[from..from + clip.width];
        for (out, &c) in pixels[start..start + clip.width].iter_mut().zip(coverage) {
            *out = blend(*out, color, (weight(c) * opacity) >> 8);
        }
    }
}

/// Covers the top and the bottom with the letterbox bars: `bars` of the
/// height in all, half at each edge.
fn draw_bars(target: &mut PixelBuffer, bars: f32) {
    let share = u64::from(permille(bars));
    if share == 0 {
        return;
    }
    let (width, height) = (target.width() as usize, u64::from(target.height()));
    let each = ((height * share + 1000) / 2000).min(height / 2 + height % 2) as usize;
    let height = height as usize;
    let pixels = target.pixels_mut();
    pixels[..each * width].fill(pixel(BAR_COLOR));
    pixels[(height - each) * width..].fill(pixel(BAR_COLOR));
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::atlas::tests::font_bytes;
    use crate::pixels::rgb_of;
    use clash_scene::{Ansi16, Canvas, CellStyle, Color, FrameCell};

    fn atlas() -> GlyphAtlas {
        GlyphAtlas::from_bytes(&font_bytes(), 16).unwrap()
    }

    fn style(fg: Rgb, bg: Rgb) -> CellStyle {
        CellStyle {
            fg: Color::from_rgb(fg),
            bg: Color::from_rgb(bg),
            attrs: Attrs::empty(),
        }
    }

    const RED: Rgb = Rgb::new(200, 0, 0);
    const BLUE: Rgb = Rgb::new(0, 0, 160);
    const GREY: Rgb = Rgb::new(40, 40, 40);

    #[test]
    fn blending_is_exact_at_the_ends() {
        let (a, b) = (pixel(RED), pixel(BLUE));
        assert_eq!(blend(a, b, 0), a);
        assert_eq!(blend(a, b, OPAQUE), b);
        assert_eq!(weight(255), OPAQUE);
        assert_eq!(weight(0), 0);
        let half = rgb_of(blend(pixel(Rgb::BLACK), pixel(Rgb::WHITE), weight(128)));
        assert_eq!(half, Rgb::new(128, 128, 128));
    }

    #[test]
    fn a_blank_frame_is_the_cells_background_with_margins_in_the_backdrop() {
        let mut atlas = atlas();
        let mut frame = Frame::new(2, 1);
        frame.clear(FrameCell::blank(style(RED, BLUE)));
        let mut target = PixelBuffer::new(20, 20);
        Renderer::new(GREY).draw(&frame, &mut atlas, &mut target);
        let layout = Layout::new(2, 1, &atlas, 20, 20);
        assert_eq!((layout.left, layout.top), (2, 2));
        assert_eq!(target.get(0, 0), Some(GREY));
        assert_eq!(target.get(2, 2), Some(BLUE));
        assert_eq!(target.get(17, 17), Some(BLUE));
        assert_eq!(target.get(18, 17), Some(GREY));
        assert_eq!(target.get(17, 18), Some(GREY));
    }

    #[test]
    fn a_glyph_is_drawn_in_its_cell_in_the_foreground() {
        let mut atlas = atlas();
        let mut frame = Frame::new(3, 1);
        frame.clear(FrameCell::blank(style(RED, BLUE)));
        Canvas::new(&mut frame).put(1, 0, '█', style(RED, BLUE));
        let mut target = PixelBuffer::new(24, 16);
        draw(&frame, &mut atlas, &mut target);
        // A full block fills its cell, and only its cell.
        for y in 0..16 {
            assert_eq!(target.get(7, y), Some(BLUE));
            assert_eq!(target.get(8, y), Some(RED), "row {y}");
            assert_eq!(target.get(15, y), Some(RED));
            assert_eq!(target.get(16, y), Some(BLUE));
        }
    }

    #[test]
    fn reverse_swaps_the_colours_and_dim_fades_the_character() {
        let mut atlas = atlas();
        let mut frame = Frame::new(2, 1);
        let mut reverse = style(RED, BLUE);
        reverse.attrs = Attrs::REVERSE;
        let mut dim = style(Rgb::WHITE, Rgb::BLACK);
        dim.attrs = Attrs::DIM;
        Canvas::new(&mut frame).put(0, 0, ' ', reverse);
        Canvas::new(&mut frame).put(1, 0, '█', dim);
        let mut target = PixelBuffer::new(16, 16);
        draw(&frame, &mut atlas, &mut target);
        assert_eq!(target.get(3, 3), Some(RED));
        let faded = target.get(12, 8).unwrap();
        assert_eq!(faded, Rgb::WHITE.toward(Rgb::BLACK, DIM_PERMILLE));
    }

    #[test]
    fn a_sprite_is_drawn_between_cells_at_its_scale_and_opacity() {
        let mut atlas = atlas();
        let mut frame = Frame::new(4, 2);
        frame.clear(FrameCell::blank(style(Rgb::WHITE, Rgb::BLACK)));
        let mut target = PixelBuffer::new(32, 32);
        let green = Color::new(Rgb::new(0, 200, 0), Ansi16::Green);
        frame.sprites.push(Sprite::new('█', green, 1.5, 0.0));
        draw(&frame, &mut atlas, &mut target);
        // Half a cell to the right of column 1: pixels 12 to 19.
        assert_eq!(target.get(11, 4), Some(Rgb::BLACK));
        assert_eq!(target.get(12, 4), Some(Rgb::new(0, 200, 0)));
        assert_eq!(target.get(19, 4), Some(Rgb::new(0, 200, 0)));
        assert_eq!(target.get(20, 4), Some(Rgb::BLACK));

        frame.sprites[0] = Sprite {
            alpha: 0.5,
            ..Sprite::new('█', green, 0.0, 0.0)
        };
        draw(&frame, &mut atlas, &mut target);
        assert_eq!(target.get(4, 8), Some(Rgb::new(0, 100, 0)));

        frame.sprites[0] = Sprite {
            scale: 2.0,
            ..Sprite::new('█', green, 1.0, 0.5)
        };
        draw(&frame, &mut atlas, &mut target);
        // Twice a cell, centred on column 1's middle: pixels 4 to 19 across.
        assert_eq!(target.get(3, 16), Some(Rgb::BLACK));
        assert_eq!(target.get(4, 16), Some(Rgb::new(0, 200, 0)));
        assert_eq!(target.get(19, 16), Some(Rgb::new(0, 200, 0)));
        assert_eq!(target.get(20, 16), Some(Rgb::BLACK));
    }

    #[test]
    fn sprites_are_drawn_by_layer_then_in_order() {
        let mut atlas = atlas();
        let mut frame = Frame::new(1, 1);
        let color = |rgb| Color::from_rgb(rgb);
        frame.sprites.push(Sprite {
            layer: 2,
            ..Sprite::new('█', color(RED), 0.0, 0.0)
        });
        frame.sprites.push(Sprite::new('█', color(BLUE), 0.0, 0.0));
        frame.sprites.push(Sprite {
            layer: 2,
            ..Sprite::new('█', color(GREY), 0.0, 0.0)
        });
        let mut target = PixelBuffer::new(8, 16);
        draw(&frame, &mut atlas, &mut target);
        assert_eq!(target.get(4, 8), Some(GREY));
    }

    #[test]
    fn nonsense_in_a_frame_draws_nothing_and_never_panics() {
        let mut atlas = atlas();
        let mut frame = Frame::new(3, 2);
        frame.cells.truncate(2);
        for (x, y, scale, alpha) in [
            (f32::NAN, 0.0, 1.0, 1.0),
            (0.0, f32::INFINITY, 1.0, 1.0),
            (0.0, 0.0, -1.0, 1.0),
            (0.0, 0.0, 1e30, 1.0),
            (1e30, -1e30, 1.0, 1.0),
            (0.0, 0.0, 1.0, 7.0),
        ] {
            frame.sprites.push(Sprite {
                scale,
                alpha,
                ..Sprite::new('\u{e000}', Color::default(), x, y)
            });
        }
        frame.fx.zoom = f32::NAN;
        frame.fx.shake = 1e9;
        frame.fx.bars = -3.0;
        for (width, height) in [(0, 0), (1, 1), (5, 300), (40, 40)] {
            let mut target = PixelBuffer::new(width, height);
            draw(&frame, &mut atlas, &mut target);
        }
        frame.fx.zoom = 1e9;
        frame.fx.bars = 7.0;
        draw(&frame, &mut atlas, &mut PixelBuffer::new(9, 9));
    }

    #[test]
    fn the_layout_finds_the_cell_under_the_pointer() {
        let atlas = atlas();
        let layout = Layout::new(10, 5, &atlas, 84, 84);
        assert_eq!((layout.left, layout.top), (2, 2));
        assert_eq!(layout.cell_rect(1, 1), (10, 18, 8, 16));
        assert_eq!(layout.cell_under(2.0, 2.0, 1.0), Some((0, 0)));
        assert_eq!(layout.cell_under(1.9, 2.0, 1.0), None);
        assert_eq!(layout.cell_under(81.9, 81.9, 1.0), Some((9, 4)));
        assert_eq!(layout.cell_under(82.0, 40.0, 1.0), None);
        // At twice the size a point halfway out from the centre is a
        // quarter of the way out.
        let (cx, cy) = layout.centre();
        assert_eq!((cx, cy), (42.0, 42.0));
        assert_eq!(layout.cell_under(82.0, 42.0, 2.0), Some((7, 2)));
        assert_eq!(grid_size(84, 84, &atlas), (10, 5));
        assert_eq!(grid_size(3, 3, &atlas), (1, 1));
    }

    #[test]
    fn drawing_again_allocates_nothing_new() {
        let mut atlas = atlas();
        let mut frame = Frame::new(10, 4);
        Canvas::new(&mut frame).print(0, 0, "Hello, ─│┌", CellStyle::default());
        frame
            .sprites
            .push(Sprite::new('*', Color::default(), 1.5, 1.5));
        frame.sprites.push(Sprite {
            scale: 2.0,
            ..Sprite::new('o', Color::default(), 3.0, 2.0)
        });
        frame.fx.zoom = 1.5;
        let mut renderer = Renderer::default();
        let mut target = PixelBuffer::new(100, 80);
        renderer.draw(&frame, &mut atlas, &mut target);
        let glyphs = atlas.len();
        let capacities = |r: &Renderer| {
            (
                r.order.capacity(),
                r.scratch.pixels().len(),
                r.map_x.capacity(),
                r.map_y.capacity(),
            )
        };
        let before = capacities(&renderer);
        let first = target.clone();
        for _ in 0..SHAKE_PATTERN.len() {
            renderer.draw(&frame, &mut atlas, &mut target);
        }
        assert_eq!(atlas.len(), glyphs);
        assert_eq!(capacities(&renderer), before);
        assert_eq!(target, first, "the same frame draws the same picture");
    }
}
