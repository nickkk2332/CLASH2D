//! The CLASH2D window frontend: draws the scene's frames of glyph cells
//! in its own window, with what a terminal cannot show: sprites between
//! cells at their scale and opacity, shake, flash, zoom, letterbox bars,
//! 24-bit colour and the mouse. This crate is outside the simulation and
//! may read the wall clock. Specified in `docs/12-presentation-and-ui.md`
//! section 11.
//!
//! # Drawn on the processor
//!
//! Each frame is drawn into a buffer of pixels by copying glyphs from an
//! atlas that `fontdue` rasterises from the theme's font, and the buffer
//! is shown with `softbuffer` in a `winit` window (decision D-830). The
//! drawing needs no window, so it is tested headless: a frame is drawn
//! to an image and compared with a stored one.
//!
//! # Parts
//!
//! | Part | Does |
//! |------|------|
//! | [`GlyphAtlas`] | The theme's font, rasterised glyph by glyph as asked for and kept, per character, style and height. |
//! | [`Renderer`], [`draw()`] | A [`Frame`](clash_scene::Frame) drawn into a [`PixelBuffer`]: cells, sprites, effects. |
//! | [`Layout`], [`grid_size`] | Where each cell is in the picture, which cell is under a point, and how many cells a window holds. |
//! | [`PixelBuffer`] | Pixels, written to and read from PPM images. |
//! | [`golden`] | Pictures compared with stored images, rewritten with `BLESS=1`. |
//! | [`theme_font`] | The font file a theme names, found inside its mod. |
//! | `WindowFrontend` | The [`Frontend`](clash_scene::Frontend): a `winit` window shown through `softbuffer`. With the `window` feature, on by default. |
//! | `input` | The window's keys, text and mouse turned into the scene's input events, tested without a window. With the `window` feature. |
//!
//! Without the `window` feature the crate is the renderer alone, with no
//! window system linked: what the benchmarks build (decision D-841).

pub mod atlas;
pub mod draw;
mod font;
pub mod golden;
#[cfg(feature = "window")]
pub mod input;
mod lines;
mod pixels;
#[cfg(feature = "window")]
pub mod window;

pub use atlas::{
    FontError, GlyphAtlas, GlyphId, GlyphMask, GlyphStyle, MAX_CELL_HEIGHT, MIN_CELL_HEIGHT,
};
pub use draw::{Layout, Renderer, draw, grid_size};
pub use font::{FontSource, theme_font};
pub use pixels::{PixelBuffer, pixel, rgb_of};
#[cfg(feature = "window")]
pub use window::{DEFAULT_GRID, WindowError, WindowFrontend, WindowOptions, display_available};
